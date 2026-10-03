//! 弹幕抓取：`cid` → `comment.bilibili.com/{cid}.xml` → 项目根下的 `danmaku.xml`。
//!
//! ## 为什么不由下载器产出弹幕
//!
//! 实测（T5）：下载器把弹幕注册成一个**字幕轨**（`danmaku` → `comment.bilibili.com/{cid}.xml`），
//! `--write-subs --sub-langs danmaku` **确实会落盘**（"静默不落盘"的社区说法未能复现）。
//! 但我们仍然自己拉，理由是**职责分离**：
//!
//! - 弹幕是 popularity 排序的唯一输入，**失败要能单独重试**（不用重下整个视频）；
//! - 弹幕只需要 `cid`，而 `cid` 在 resolve 阶段就拿到了 → 预览卡能显示"实际能收到多少条"；
//! - 下载器的 `--write-subs` 行为是它的实现细节（哪天改了、或换个引擎，我们不跟着碎）。
//!
//! ## 两个实测出来的坑
//!
//! 1. **UA 是硬门槛**：非浏览器 UA（`python-urllib`、`reqwest/x.y`）→ **HTTP 412**，
//!    响应头带 `x-sec-request-id`。所以走 [`crate::net::browser_client`]。
//! 2. **响应恒为裸 deflate，且忽略 `Accept-Encoding: identity`** → 必须手工解压，
//!    见 [`crate::net::deflate::inflate`]（不解压会拿到乱码，正则一条都匹配不上，
//!    **静默产出 0 条弹幕**）。
//!
//! 解析本身**复用** `yekitrim_core::danmaku::parse_bililive_xml`（录播姬写的那套 XML 与
//! 主站同源；差异只在第 8 段的语义，见该函数的文档注释）。

use std::path::{Path, PathBuf};

use super::BiliError;
use crate::net::{browser_client, deflate};

/// 弹幕接口前缀。
pub const DANMAKU_API: &str = "https://comment.bilibili.com";
/// 落盘文件名（写进 `yekitrim.toml` 的 `danmaku_file`）。
pub const DANMAKU_FILE: &str = "danmaku.xml";

/// 一次弹幕抓取的结果。
#[derive(Debug, Clone)]
pub struct DanmakuFetch {
    /// 解压后的 XML 原文
    pub xml: String,
    /// 能解析出的弹幕条数（**主站/录播姬两种方言都能数**）
    pub entries: usize,
    /// 服务端声明的 `Content-Encoding`（只用于日志）
    pub declared_encoding: Option<String>,
}

impl DanmakuFetch {
    /// 一条都没有（合法但空）。
    ///
    /// 这不一定是错误：有些视频的弹幕池就是空的。调用方应当**明确记一笔**再继续
    /// （项目照样能跑，只是没有 popularity 排序），而不是静默当成"弹幕已就绪"。
    pub fn is_empty(&self) -> bool {
        self.entries == 0
    }
}

/// 解析一次弹幕响应（**离线可测**：输入是原始字节）。
///
/// 校验分两层，**都不能省**：
/// - 解压后必须**像 XML**（有 `<i>` 根），否则报错 —— 不解压就传给解析器会静默得到 0 条；
/// - 条数由真解析器数出来（不是 `grep` 计数），这样"格式变了"会体现在数字上。
pub fn parse_danmaku_response(
    cid: i64,
    body: &[u8],
    declared_encoding: Option<String>,
) -> Result<DanmakuFetch, BiliError> {
    let (raw, flavor) = deflate::inflate(body).map_err(|e| {
        BiliError::Malformed(format!("cid={cid} 的弹幕响应解压失败: {e}"))
    })?;
    let xml = String::from_utf8(raw).map_err(|e| {
        BiliError::Malformed(format!("cid={cid} 的弹幕不是合法 UTF-8: {e}"))
    })?;

    // 结构校验：真实的弹幕 XML 一定有 `<i>` 根。缺了就说明拿到的不是弹幕
    // （可能是风控页/404 页），此时**必须报错**，不能当"空弹幕"糊过去。
    let trimmed = xml.trim_start_matches('\u{feff}').trim_start();
    if !(trimmed.starts_with("<?xml") || trimmed.starts_with("<i>") || trimmed.starts_with("<i ")) {
        let head: String = trimmed.chars().take(120).collect();
        return Err(BiliError::Malformed(format!(
            "cid={cid} 的响应不是弹幕 XML（{flavor:?}，开头: {head}）"
        )));
    }

    let entries = yekitrim_core::danmaku::parse_bililive_xml(&xml).len();
    Ok(DanmakuFetch {
        xml,
        entries,
        declared_encoding,
    })
}

/// 拉取一个 cid 的弹幕。
pub async fn fetch_danmaku(cid: i64) -> Result<DanmakuFetch, BiliError> {
    if cid <= 0 {
        return Err(BiliError::BadInput(format!("非法 cid: {cid}")));
    }
    let url = format!("{DANMAKU_API}/{cid}.xml");
    let resp = browser_client()
        .get(&url)
        // Referer 不是必须的，但带上更接近真实浏览器（实测带上更稳）
        .header("Referer", "https://www.bilibili.com/")
        .send()
        .await
        .map_err(|e| {
            if e.is_timeout() {
                BiliError::Transport("请求弹幕超时".into())
            } else {
                BiliError::Transport(e.to_string())
            }
        })?;

    let status = resp.status();
    if status == reqwest::StatusCode::PRECONDITION_FAILED
        || status == reqwest::StatusCode::TOO_MANY_REQUESTS
    {
        return Err(BiliError::RateLimited);
    }
    if status == reqwest::StatusCode::NOT_FOUND {
        // 弹幕池可能被关闭/清空：给一个"没有弹幕"的合法空 XML，
        // 让上层走"项目照样建、但没有 popularity"那条路，而不是整体失败。
        tracing::warn!("cid={cid} 的弹幕接口返回 404（弹幕池可能已关闭），按空弹幕处理");
        return Ok(DanmakuFetch {
            xml: EMPTY_XML.to_string(),
            entries: 0,
            declared_encoding: None,
        });
    }
    if !status.is_success() {
        return Err(BiliError::Api {
            code: status.as_u16() as i64,
            message: format!("弹幕接口 HTTP {status}"),
        });
    }

    let declared = resp
        .headers()
        .get(reqwest::header::CONTENT_ENCODING)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.trim().to_ascii_lowercase());
    let body = resp
        .bytes()
        .await
        .map_err(|e| BiliError::Transport(format!("读取弹幕响应失败: {e}")))?;

    parse_danmaku_response(cid, &body, declared)
}

/// 合法的空弹幕文档（结构完整，只是没有 `<d>` 条目）。
pub const EMPTY_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?><i><chatserver>chat.bilibili.com</chatserver><maxlimit>0</maxlimit><state>0</state><source>k-v</source></i>"#;

/// 把弹幕写进项目根，返回**相对文件名**（直接可以塞进 `write_project_config`）。
///
/// 项目根不存在时先建（下载通路里目录通常已经建好，但重试弹幕时可能还没有）。
pub fn save_danmaku(project_dir: &Path, fetch: &DanmakuFetch) -> std::io::Result<String> {
    std::fs::create_dir_all(project_dir)?;
    std::fs::write(project_dir.join(DANMAKU_FILE), fetch.xml.as_bytes())?;
    Ok(DANMAKU_FILE.to_string())
}

/// 项目里 `danmaku.xml` 的完整路径。
pub fn danmaku_path(project_dir: &Path) -> PathBuf {
    project_dir.join(DANMAKU_FILE)
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::write::DeflateEncoder;
    use flate2::Compression;
    use std::io::Write;

    /// 真实主站弹幕的形状：9 段 `p`、第 8 段是 17–19 位 dmid。
    /// （逐字段取自实测 `cid=122300483`；这里裁到两条，格式一致。）
    const MAINSITE_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?><i><chatserver>chat.bilibili.com</chatserver><chatid>122300483</chatid><maxlimit>1000</maxlimit><state>0</state><source>k-v</source><d p="0.60100,4,25,15138834,1604742895,0,9e5adeaa,40685126698926083,10">月背</d><d p="1.5,1,25,16777215,1604742896,0,abc123,40685126698926084,0">前方高能</d></i>"#;

    fn raw_deflate(data: &[u8]) -> Vec<u8> {
        let mut e = DeflateEncoder::new(Vec::new(), Compression::default());
        e.write_all(data).unwrap();
        e.finish().unwrap()
    }

    /// ⚠️ 全链路钉子：**裸 deflate 的响应要能解出正确条数**。
    ///
    /// 这一条同时挡住两个真实故障：
    /// ① 不解压 → 乱码 → 正则一条都匹配不上 → **静默 0 条弹幕**；
    /// ② 解析器回归（例如又去拿 `p[7]` 当行号）→ 条数还是对，但 y 会塌，由 core 的测试兜。
    #[test]
    fn bare_deflate_response_decodes_and_counts() {
        let body = raw_deflate(MAINSITE_XML.as_bytes());
        let got = parse_danmaku_response(122_300_483, &body, Some("deflate".into())).unwrap();
        assert_eq!(got.entries, 2, "解压后要能数出真实条数");
        assert!(!got.is_empty());
        assert_eq!(got.xml, MAINSITE_XML);
        assert_eq!(got.declared_encoding.as_deref(), Some("deflate"));
    }

    /// 明文响应（服务端哪天不压缩了）也要能过，不能因此整体失败。
    #[test]
    fn plain_xml_response_also_works() {
        let got = parse_danmaku_response(1, MAINSITE_XML.as_bytes(), None).unwrap();
        assert_eq!(got.entries, 2);
    }

    /// ⚠️ 回归钉子：**结构不合法必须报错，不能当成"空弹幕"糊过去**。
    ///
    /// 风控页/错误页是 HTML，如果被当成"合法但 0 条"，用户会看到项目建好了、
    /// 但排序全错，而且没有任何错误可查 —— 这是最难归因的一类故障。
    #[test]
    fn non_xml_payload_is_an_error_not_empty_danmaku() {
        for junk in [
            &b"<html><body>412 Precondition Failed</body></html>"[..],
            &b"{\"code\":-412,\"message\":\"blocked\"}"[..],
            &[0x00, 0x01, 0x02, 0x03][..],
        ] {
            let e = parse_danmaku_response(9, junk, None).unwrap_err();
            assert_eq!(e.kind_str(), "Malformed", "输入: {junk:?}");
            assert!(e.user_message().contains("无法识别"), "{e}");
        }
    }

    /// 结构完整但没有 `<d>` → **合法但空**（不是错误）。
    /// 有些视频弹幕池就是空的，项目照样该能建起来。
    #[test]
    fn structurally_valid_but_empty_is_ok_and_flagged() {
        let got = parse_danmaku_response(1, EMPTY_XML.as_bytes(), None).unwrap();
        assert_eq!(got.entries, 0);
        assert!(got.is_empty(), "要能明确看出是空的（调用方据此记一笔）");
        assert!(got.xml.contains("<i>"));
    }

    /// 录播姬写出的 8 段 XML 也要能走同一条路（第 8 段恒为 0）。
    #[test]
    fn recorder_flavor_counts_too() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?><i><chatserver>chat.bilibili.com</chatserver><chatid>0</chatid><maxlimit>0</maxlimit><state>0</state><source>k-v</source><d p="1.5,1,25,16777215,1700000000,0,abc,0">滚动</d><d p="20,5,25,255,1700000020,0,abc,0">顶端</d></i>"#;
        let got = parse_danmaku_response(1, xml.as_bytes(), None).unwrap();
        assert_eq!(got.entries, 2);
    }

    /// 落盘：文件名固定、内容逐字节一致（`yekitrim.toml` 里写的就是这个名字）。
    #[test]
    fn save_writes_danmaku_xml_into_project_root() {
        let dir = std::env::temp_dir().join(format!("yekitrim-dm-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let fetch = parse_danmaku_response(1, MAINSITE_XML.as_bytes(), None).unwrap();
        let name = save_danmaku(&dir, &fetch).unwrap();
        assert_eq!(name, "danmaku.xml");
        assert_eq!(danmaku_path(&dir), dir.join("danmaku.xml"));
        // 写出来的东西要能被**现有解析器**直接读（这是"零解析器改动"的验收点）
        let entries = yekitrim_core::danmaku::load_danmaku_auto(&danmaku_path(&dir)).unwrap();
        assert_eq!(entries.len(), 2);
        assert!(entries.iter().all(|e| e.start >= 0.0));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
