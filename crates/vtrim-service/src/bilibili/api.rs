//! B站 view 端点：`/x/web-interface/view`。
//!
//! ```text
//! GET https://api.bilibili.com/x/web-interface/view?bvid=BV1xxxx
//! Header: User-Agent: <浏览器 UA>      ← 必须有；不需要 Referer / Cookie / WBI 签名
//! ```
//!
//! 一次调用给出：`title`、`owner.name`、`duration`、`pic`（封面）、`stat.danmaku/view`，
//! 以及 `pages[]`（每个分P 的 `cid` / `part` / `duration`）。**分P 的 cid 在这里，
//! 不在 `yt-dlp -J` 里**（实测 T3：普通 `-J` 里 `cid` 出现 0 次）。

use serde::Deserialize;

use super::BiliError;
use crate::net::browser_client;

/// view 端点。
pub const VIEW_API: &str = "https://api.bilibili.com/x/web-interface/view";

/// 一个分P。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct BvPage {
    /// 1-based（与 B站的 `page` 同口径）
    pub index: u32,
    /// **这是弹幕接口要的那个 cid**
    pub cid: i64,
    /// 分P 标题（单P 时通常是视频标题）
    pub part: String,
    pub duration_secs: Option<f64>,
}

/// 一个视频（或一个多P 合集）的元数据。
#[derive(Debug, Clone, Default)]
pub struct BvInfo {
    pub bvid: String,
    pub aid: i64,
    pub title: String,
    pub uploader: String,
    pub duration_secs: Option<f64>,
    pub cover: Option<String>,
    /// 分P 列表。**单P 也有一条**（B站的 `pages` 对单P 也返回一项），
    /// 所以调用方不必为单P/多P 写两套分支 —— `pages.len() == 1` 就是单P。
    pub pages: Vec<BvPage>,
    /// B站自报的弹幕总数（用于预览卡上的"收录量级"提示，见风险表）
    pub danmaku_total: u64,
    pub view_total: u64,
}

impl BvInfo {
    /// 是否多P（前端据此决定要不要显示分P 勾选列表）。
    pub fn is_multi_page(&self) -> bool {
        self.pages.len() > 1
    }

    /// 选中若干 P 的合计时长（`None` = 有时长缺失，别硬凑一个数出来）。
    pub fn total_duration_of(&self, indices: &[u32]) -> Option<f64> {
        let mut sum = 0.0;
        let mut any = false;
        for p in &self.pages {
            if indices.contains(&p.index) {
                sum += p.duration_secs?;
                any = true;
            }
        }
        any.then_some(sum)
    }
}

// ---- 反序列化用的原始结构（字段名就是 B站的返回形态）----

#[derive(Deserialize)]
struct RawResp {
    code: i64,
    #[serde(default)]
    message: String,
    #[serde(default)]
    data: Option<RawData>,
}

#[derive(Deserialize)]
struct RawData {
    #[serde(default)]
    bvid: String,
    #[serde(default)]
    aid: i64,
    #[serde(default)]
    cid: i64,
    #[serde(default)]
    title: String,
    #[serde(default)]
    pic: String,
    #[serde(default)]
    duration: f64,
    #[serde(default)]
    owner: Option<RawOwner>,
    #[serde(default)]
    stat: Option<RawStat>,
    #[serde(default)]
    pages: Option<Vec<RawPage>>,
}

#[derive(Deserialize)]
struct RawOwner {
    #[serde(default)]
    name: String,
}

#[derive(Deserialize)]
struct RawStat {
    #[serde(default)]
    danmaku: u64,
    #[serde(default)]
    view: u64,
}

#[derive(Deserialize)]
struct RawPage {
    #[serde(default)]
    page: u32,
    #[serde(default)]
    cid: i64,
    #[serde(default)]
    part: String,
    #[serde(default)]
    duration: f64,
}

/// `code != 0` → 具体错误（**不要当成解析失败**：那会把"稿件审核中"报成"网络问题"）。
fn map_code(code: i64, message: &str) -> BiliError {
    match code {
        -400 => BiliError::BadInput(format!("B站拒绝了请求（{message}）")),
        -403 | -40300 => BiliError::Forbidden,
        -404 => BiliError::NotFound,
        -412 => BiliError::RateLimited,
        -509 => BiliError::RateLimited,
        62002 => BiliError::NotAvailable("稿件不可见".into()),
        62004 => BiliError::NotAvailable("稿件审核中".into()),
        62012 => BiliError::NotAvailable("仅 UP 主自己可见".into()),
        _ => BiliError::Api { code, message: message.to_string() },
    }
}

/// 解析 view 的 JSON 响应（**离线可测**：不发请求）。
pub fn parse_view(raw: &str) -> Result<BvInfo, BiliError> {
    let resp: RawResp = serde_json::from_str(raw).map_err(|e| {
        // 把前 120 字节放进错误里：B站风控时返回的是 HTML 而不是 JSON，
        // 只报"JSON 解析失败"会让人以为是我们的 bug。
        let head: String = raw.chars().take(120).collect();
        BiliError::Malformed(format!("响应不是合法 JSON（{}）；开头: {}", e, head))
    })?;

    if resp.code != 0 {
        return Err(map_code(resp.code, &resp.message));
    }
    let data = resp.data.ok_or_else(|| {
        BiliError::Malformed("code=0 但没有 data 字段".into())
    })?;

    let mut pages: Vec<BvPage> = data
        .pages
        .unwrap_or_default()
        .into_iter()
        .filter(|p| p.cid > 0)
        .map(|p| BvPage {
            index: p.page,
            cid: p.cid,
            part: p.part,
            duration_secs: (p.duration > 0.0).then_some(p.duration),
        })
        .collect();

    // 单P：`pages` 偶尔为空（老接口形态），用顶层 `cid` 补一条 —— 否则"取不到 cid"会让
    // 整个弹幕通道静默失效（弹幕是本项目的关键，不能因为一个可选字段缺失就降级）。
    if pages.is_empty() && data.cid > 0 {
        pages.push(BvPage {
            index: 1,
            cid: data.cid,
            part: data.title.clone(),
            duration_secs: (data.duration > 0.0).then_some(data.duration),
        });
    }
    if pages.is_empty() {
        return Err(BiliError::Malformed(
            "没有任何可用的 cid（无法取弹幕，也无法确定要下载哪一分P）".into(),
        ));
    }
    // 分P 序号缺失时按位置补（有些老响应不带 `page`）
    for (i, p) in pages.iter_mut().enumerate() {
        if p.index == 0 {
            p.index = i as u32 + 1;
        }
    }
    pages.sort_by_key(|p| p.index);

    let title = if data.title.trim().is_empty() {
        return Err(BiliError::Malformed("标题为空".into()));
    } else {
        data.title
    };

    Ok(BvInfo {
        bvid: data.bvid,
        aid: data.aid,
        title,
        uploader: data.owner.map(|o| o.name).unwrap_or_default(),
        duration_secs: (data.duration > 0.0).then_some(data.duration),
        cover: (!data.pic.trim().is_empty()).then_some(data.pic),
        pages,
        danmaku_total: data.stat.as_ref().map(|s| s.danmaku).unwrap_or(0),
        view_total: data.stat.as_ref().map(|s| s.view).unwrap_or(0),
    })
}

/// 拉取元数据（**一次 GET，免签名免 cookie**）。
pub async fn fetch_view(bvid: &str) -> Result<BvInfo, BiliError> {
    let url = format!("{VIEW_API}?bvid={bvid}");
    let resp = browser_client()
        .get(&url)
        .send()
        .await
        .map_err(|e| {
            if e.is_timeout() {
                BiliError::Transport("请求超时".into())
            } else {
                BiliError::Transport(e.to_string())
            }
        })?;

    let status = resp.status();
    if status == reqwest::StatusCode::PRECONDITION_FAILED {
        // 412：B站的"风控"应答（实测非浏览器 UA 必现）
        return Err(BiliError::RateLimited);
    }
    if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
        return Err(BiliError::RateLimited);
    }
    if !status.is_success() {
        return Err(BiliError::Api {
            code: status.as_u16() as i64,
            message: format!("HTTP {status}"),
        });
    }
    let body = resp.text().await.map_err(|e| BiliError::Transport(e.to_string()))?;
    parse_view(&body)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 单P：字段名与结构取自**实测响应**（2026-09-19，`BV13x41117TL` 的裁剪版）。
    #[test]
    fn parses_single_video() {
        let raw = r#"{
            "code": 0,
            "message": "0",
            "ttl": 1,
            "data": {
                "bvid": "BV13x41117TL",
                "aid": 14694589,
                "cid": 23780012,
                "title": "【示例】单P 视频",
                "pic": "http://i2.hdslb.com/bfs/archive/abc.jpg",
                "duration": 313,
                "owner": {"mid": 1, "name": "某UP主"},
                "stat": {"view": 12345, "danmaku": 678, "reply": 9},
                "pages": [{"cid": 23780012, "page": 1, "part": "P1", "duration": 313}]
            }
        }"#;
        let v = parse_view(raw).unwrap();
        assert_eq!(v.bvid, "BV13x41117TL");
        assert_eq!(v.aid, 14694589);
        assert_eq!(v.uploader, "某UP主");
        assert_eq!(v.danmaku_total, 678);
        assert_eq!(v.view_total, 12345);
        assert_eq!(v.duration_secs, Some(313.0));
        assert!(!v.is_multi_page());
        assert_eq!(v.pages.len(), 1);
        assert_eq!(v.pages[0].cid, 23780012, "cid 是弹幕通道的唯一输入");
    }

    /// 多P：`pages` 全展开，且**顺序按 `page` 排**（B站的返回顺序不保证）。
    #[test]
    fn parses_multi_page_and_sorts() {
        let raw = r#"{"code":0,"data":{
            "bvid":"BV1bK411W797","aid":1,"title":"合集","duration":0,
            "pages":[
                {"page":2,"cid":200,"part":"P2","duration":60},
                {"page":1,"cid":100,"part":"P1","duration":30},
                {"page":3,"cid":300,"part":"P3","duration":90}
            ]}}"#;
        let v = parse_view(raw).unwrap();
        assert!(v.is_multi_page());
        assert_eq!(
            v.pages.iter().map(|p| (p.index, p.cid)).collect::<Vec<_>>(),
            vec![(1, 100), (2, 200), (3, 300)]
        );
        assert_eq!(v.total_duration_of(&[1, 3]), Some(120.0));
        // 只勾一个P 时合计就是那一个
        assert_eq!(v.total_duration_of(&[2]), Some(60.0));
        // 勾了不存在的P → None（不硬凑 0）
        assert_eq!(v.total_duration_of(&[9]), None);
    }

    /// `pages` 缺失但顶层有 `cid` → 补一条单P（否则弹幕通道会静默失效）。
    #[test]
    fn falls_back_to_top_level_cid() {
        let raw = r#"{"code":0,"data":{"bvid":"BV1","cid":555,"title":"T","duration":10}}"#;
        let v = parse_view(raw).unwrap();
        assert_eq!(v.pages.len(), 1);
        assert_eq!(v.pages[0].cid, 555);
        assert_eq!(v.pages[0].index, 1);
    }

    /// ⚠️ 回归钉子：**`code != 0` 必须映射成具体错误**，不能当成"解析失败"，
    /// 也不能当成"没有 data"。
    #[test]
    fn maps_api_codes_to_specific_errors() {
        let cases = [
            (r#"{"code":-404,"message":"啥都木有"}"#, "NotFound"),
            (r#"{"code":-403,"message":"访问权限不足"}"#, "Forbidden"),
            (r#"{"code":62002,"message":"稿件不可见"}"#, "NotAvailable"),
            (r#"{"code":62004,"message":"稿件审核中"}"#, "NotAvailable"),
            (r#"{"code":-412,"message":"请求被拦截"}"#, "RateLimited"),
        ];
        for (raw, want) in cases {
            let e = parse_view(raw).unwrap_err();
            assert_eq!(e.kind_str(), want, "{raw}");
            assert!(!e.user_message().trim().is_empty());
        }
        // 没映射过的 code → Api，且**带上原始 code 与 message**（便于上报）
        let e = parse_view(r#"{"code":-509,"message":"请求过于频繁"}"#).unwrap_err();
        assert_eq!(e.kind_str(), "RateLimited", "-509 也是限流");
        let e = parse_view(r#"{"code":99999,"message":"未知"}"#).unwrap_err();
        assert_eq!(e.kind_str(), "Api");
        assert!(e.user_message().contains("99999"), "{e}");
    }

    /// 风控时 B站返回 HTML → 错误里要带**响应开头**，否则只看到"JSON 解析失败"没法归因。
    #[test]
    fn html_error_page_is_reported_as_malformed_with_preview() {
        let e = parse_view("<html><head><title>412 Precondition Failed</title></head>").unwrap_err();
        assert_eq!(e.kind_str(), "Malformed");
        assert!(e.user_message().contains("无法识别"), "{e}");
        assert!(format!("{e:?}").contains("412") || e.user_message().contains("无法识别"));
    }

    /// 缺 cid 的视频要**明确失败**（不是静默给个没有弹幕的项目）。
    #[test]
    fn missing_cid_is_an_error_not_a_silent_degrade() {
        let e = parse_view(r#"{"code":0,"data":{"title":"T","pages":[]}}"#).unwrap_err();
        assert_eq!(e.kind_str(), "Malformed");
        assert!(e.user_message().contains("cid") || e.user_message().contains("无法识别"), "{e}");
        // 标题为空也要报错（前端预览卡没法显示）
        let e = parse_view(r#"{"code":0,"data":{"cid":1,"title":"  "}}"#).unwrap_err();
        assert_eq!(e.kind_str(), "Malformed");
    }

    /// `duration` 为 0 / 缺失时**不能**变成 `Some(0.0)`：那会让"预计占用"显示 0。
    #[test]
    fn zero_duration_is_none_not_zero() {
        let v = parse_view(r#"{"code":0,"data":{"cid":1,"title":"T","duration":0}}"#).unwrap();
        assert!(v.duration_secs.is_none());
        assert!(v.pages[0].duration_secs.is_none());
        assert_eq!(v.total_duration_of(&[1]), None);
    }
}
