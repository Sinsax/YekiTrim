//! B站专属知识（**不认识下载器**）。
//!
//! §2.4 的边界判据：本目录里**不许出现** `yt-dlp` / `--print` / `spawn`。
//! 引擎与 B站只在一处装配（`yekitrim-server/download_worker.rs`）。
//!
//! ## 为什么元数据走 view API 而不是 `yt-dlp -J`
//!
//! 实测（`p4-prerequisite-verification.md` T3）：普通 `yt-dlp -J` 里 **`cid` 出现 0 次**、
//! `subtitles` 是 `{}` —— 因为 `extract_subtitles` 有 `writesubtitles || listsubtitles` 门控。
//! 而 B站的 view API：

//! ```text
//! GET https://api.bilibili.com/x/web-interface/view?bvid=BV1xxxx
//! ```
//!
//! **裸 GET 就能用**：不需要 WBI 签名、不需要 cookie、不需要 Referer。一次调用给出
//! 全部分P 的 `cid` + P 名 + 时长 + 弹幕总数 + 封面。分P 视频也不需要逐 P 再请求 ——
//! 这比 `yt-dlp -J`（多P 要展开每个 entry）快一个数量级，而 resolve 卡是用户每次
//! 换链接都会点的。

pub mod api;
pub mod danmaku;

pub use api::{fetch_view, parse_view, BvInfo, BvPage};
pub use danmaku::{fetch_danmaku, DanmakuFetch};

/// B站侧的错误分类。
///
/// ⚠️ 与下载器的错误类**刻意分开**：这两条通道是独立的（view API / 弹幕 vs 视频流），
/// 失败原因也不同（一个可能是"稿件审核中"，另一个可能是"格式不可用"）。
/// 混成一个枚举会逼着两边都处理对方的分支。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BiliError {
    /// 解析不出 bvid（用户粘了一个不认识的链接）
    BadInput(String),
    /// 视频不存在 / 已删除
    NotFound,
    /// 无权限（`-403`）
    Forbidden,
    /// 稿件不可见（`62002`）/ 审核中（`62004`）
    NotAvailable(String),
    /// 被风控限流（412 / 429 / `-412`）
    RateLimited,
    /// 接口返回了看不懂的东西（`code != 0` 且没有已知映射）
    Api { code: i64, message: String },
    /// 传输层失败（超时/连接）
    Transport(String),
    /// 响应不是合法 XML/JSON（**必须报错，不能静默当空**）
    Malformed(String),
}

impl BiliError {
    /// 给用户看的文案（人话，不含原始响应）。
    pub fn user_message(&self) -> String {
        match self {
            BiliError::BadInput(s) => format!("链接里没有识别出视频号（{s}）"),
            BiliError::NotFound => "视频不存在或已被删除".to_string(),
            BiliError::Forbidden => "这个视频没有访问权限".to_string(),
            BiliError::NotAvailable(s) => format!("视频当前不可用（{s}）"),
            BiliError::RateLimited => "被 B站风控拦截（请求过于频繁）；请稍后重试".to_string(),
            BiliError::Api { code, message } => {
                if message.trim().is_empty() {
                    format!("B站接口返回错误（code={code}）")
                } else {
                    format!("B站接口返回错误（code={code}: {message}）")
                }
            }
            BiliError::Transport(s) => format!("访问 B站失败（{s}）"),
            BiliError::Malformed(s) => format!("B站返回了无法识别的数据（{s}）"),
        }
    }

    pub fn kind_str(&self) -> &'static str {
        match self {
            BiliError::BadInput(_) => "BadInput",
            BiliError::NotFound => "NotFound",
            BiliError::Forbidden => "Forbidden",
            BiliError::NotAvailable(_) => "NotAvailable",
            BiliError::RateLimited => "RateLimited",
            BiliError::Api { .. } => "Api",
            BiliError::Transport(_) => "Transport",
            BiliError::Malformed(_) => "Malformed",
        }
    }

    /// 落库/回给前端的形态。
    pub fn user_facing(&self) -> String {
        format!("{}（{}）", self.user_message(), self.kind_str())
    }
}

impl std::fmt::Display for BiliError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.user_facing())
    }
}

impl std::error::Error for BiliError {}

/// 从用户粘的任意形态里抠出 **bvid**。
///
/// 支持：纯 `BV1xxxxxxxxx`、完整 URL（`https://www.bilibili.com/video/BV1xx/?p=1`、
/// 带 `?bvid=` 的）、以及 `av123`（转成 **aid** 由调用方处理 —— 这里只认 bvid，
/// 因为 view API 也可以用 `aid=` 查，交给 [`api::fetch_view_by`]）。
///
/// ⚠️ **不支持也不猜**番剧（`/bangumi/play/ep123`）与课程（`/cheese/`）：
/// 它们没有 bvid，而且本项目没有对应的信息架构（见落地计划 §1 的"不做"清单）。
/// 认不出来就明确报错，**不要**当成"某个视频"去试。
pub fn extract_bvid(input: &str) -> Result<String, BiliError> {
    let s = input.trim();
    if s.is_empty() {
        return Err(BiliError::BadInput("输入为空".into()));
    }
    // 纯 bvid
    if let Some(bv) = as_bvid(s) {
        return Ok(bv);
    }
    // URL 形态：取路径/查询里的 BV 号
    if let Some(idx) = s.find("BV").or_else(|| s.find("bv")) {
        if let Some(bv) = as_bvid(&s[idx..]) {
            return Ok(bv);
        }
    }
    // 明确点名不支持的形式（免得用户以为是我们坏了）
    let lower = s.to_ascii_lowercase();
    if lower.contains("/bangumi/") || lower.contains("/cheese/") || lower.contains("ep") && lower.contains("bangumi") {
        return Err(BiliError::BadInput(
            "番剧/课程链接暂不支持（请用单个视频链接）".into(),
        ));
    }
    Err(BiliError::BadInput("没找到 BV 号".into()))
}

/// `BV` + 10 位字母数字（共 12 字符）。
///
/// ⚠️ **只把 `BV` 前缀规整成大写，后 10 位原样保留** —— bvid 的后半段是**大小写敏感**的
/// （B站用了一套自造的 base58 变体，`BV1GJ411x7h7` 与 `BV1GJ411X7H7` 是两个不同的视频）。
/// 早期版本的 `extract_bvid` 整串 `to_ascii_uppercase()`，会让所有小写字母的视频
/// **稳定查不到**（表现为"视频不存在"，很像是 B站的问题，极难归因）。
fn as_bvid(s: &str) -> Option<String> {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() < 12 {
        return None;
    }
    if !(chars[0] == 'B' || chars[0] == 'b') || !(chars[1] == 'V' || chars[1] == 'v') {
        return None;
    }
    let body: String = chars[2..12].iter().collect();
    if !body.chars().all(|c| c.is_ascii_alphanumeric()) {
        return None;
    }
    Some(format!("BV{body}"))
}

/// 下载会话在 `ingest_sessions.source_ref` 里的编码：`BV1GJ411x7h7` 或 `BV1GJ411x7h7/p3`。
///
/// ## 为什么编码进一个字段而不是加两列
///
/// 分P 视频**每 P 一个会话**（一 P 一个项目），所以会话必须能区分"哪个视频的哪一 P"。
/// 加 `page` 列要多一次迁移，而 `source_ref` 本来就只是"来源侧标识"、不对外暴露
/// （`projects.room` 才是给前端看的那个，bvid 绝不能塞那儿）。
///
/// ⚠️ bvid 固定 12 字符且**大小写敏感**，所以分隔符只能用不在 bvid 字符集里的字符：
/// `/` 是安全的（`as_bvid` 只接受字母数字）。**别再换回 `-` 或 `_`** —— 那些字符
/// 看起来更"路径安全"，但一旦将来 bvid 字符集变化就会产生歧义。
pub fn session_ref(bvid: &str, page: Option<u32>) -> String {
    match page.filter(|p| *p > 1) {
        // 第 1 P 不写后缀：单P 与"多P 的第一 P"用同一种形态，resolve 预览里也看不出差别
        None => bvid.to_string(),
        Some(p) => format!("{bvid}/p{p}"),
    }
}

/// [`session_ref`] 的反解。返回 `(bvid, page)`，`page` 缺省即 `None`（= 第 1 P）。
///
/// 认不出来返回 `None`（而不是给一个"大概对"的默认值）：调用方据此把会话标 failed，
/// 否则会拿一个错视频去下载。
pub fn parse_session_ref(raw: &str) -> Option<(String, Option<u32>)> {
    let s = raw.trim();
    match s.split_once('/') {
        None => Some((extract_bvid(s).ok()?, None)),
        Some((bv, rest)) => {
            let bvid = extract_bvid(bv).ok()?;
            let n = rest.strip_prefix('p')?.parse::<u32>().ok()?;
            if n == 0 {
                return None;
            }
            Some((bvid, Some(n)))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 会话引用的编解码要**往返一致**，且认不出来的要返回 `None`（不是给个默认视频）。
    #[test]
    fn session_ref_round_trips() {
        assert_eq!(session_ref("BV1GJ411x7h7", None), "BV1GJ411x7h7");
        // 第 1 P 归一：`Some(1)` 与 `None` 是同一种形态
        assert_eq!(session_ref("BV1GJ411x7h7", Some(1)), "BV1GJ411x7h7");
        assert_eq!(session_ref("BV1GJ411x7h7", Some(23)), "BV1GJ411x7h7/p23");
        assert_eq!(session_ref("BV1GJ411x7h7", Some(0)), "BV1GJ411x7h7");

        for (bvid, page) in [
            ("BV1GJ411x7h7", None),
            ("BV1GJ411x7h7", Some(3)),
            ("BV1bK411W797", Some(23)),
        ] {
            let raw = session_ref(bvid, page);
            let (b, p) = parse_session_ref(&raw).unwrap_or_else(|| panic!("解析失败: {raw}"));
            assert_eq!(b, bvid, "bvid 大小写必须原样保留: {raw}");
            assert_eq!(p, page);
        }

        for bad in ["", "/p3", "BV1GJ411x7h7/p", "BV1GJ411x7h7/p0", "BV1GJ411x7h7/x3", "bvid"] {
            assert!(parse_session_ref(bad).is_none(), "不该认出来: {bad}");
        }
    }

    /// 用户粘什么形态都要能认出来 —— 这是"粘贴框"的第一道关。
    ///
    /// ⚠️ 注意断言里的**大小写**：`BV1GJ411x7h7` 必须保持原样（bvid 后半段大小写敏感），
    /// 只有 `BV` 前缀被规整。全大写化是一个会让所有小写视频"查不到"的静默 bug。
    #[test]
    fn extract_bvid_accepts_real_world_forms() {
        for raw in [
            "BV1GJ411x7h7",
            "  BV1GJ411x7h7  ",
            "https://www.bilibili.com/video/BV1GJ411x7h7",
            "https://www.bilibili.com/video/BV1GJ411x7h7/",
            "https://www.bilibili.com/video/BV1GJ411x7h7?p=3&t=12",
            "https://www.bilibili.com/video/BV1GJ411x7h7/?spm_id_from=333.999",
            "https://b23.tv/BV1GJ411x7h7",
            "https://www.bilibili.com/video/bv1GJ411x7h7",
        ] {
            assert_eq!(extract_bvid(raw).unwrap(), "BV1GJ411x7h7", "输入: {raw}");
        }
        // 全大写的 bvid（真实存在的另一批视频）不能被改小写，也不能被改大小写
        assert_eq!(extract_bvid("BV1bK411W797").unwrap(), "BV1bK411W797");
    }

    /// 认不出来就**明确报错**，而且要把"番剧/课程不支持"单独说出来
    /// （不然用户会以为是我们的 bug，反复重试）。
    #[test]
    fn extract_bvid_rejects_unknown_forms_with_reason() {
        assert!(extract_bvid("").is_err());
        assert!(extract_bvid("https://example.com/hello").is_err());
        assert!(extract_bvid("BV").is_err());
        assert!(extract_bvid("BV123").is_err());

        let e = extract_bvid("https://www.bilibili.com/bangumi/play/ep123456").unwrap_err();
        assert_eq!(e.kind_str(), "BadInput");
        assert!(e.user_message().contains("番剧"), "{e}");
        let e = extract_bvid("https://www.bilibili.com/cheese/play/ss123").unwrap_err();
        assert!(e.user_message().contains("番剧") || e.user_message().contains("课程"), "{e}");
    }

    /// 错误文案必须是人话，且带分类标签。
    #[test]
    fn error_messages_are_human_readable() {
        let e = BiliError::Api { code: -400, message: "请求错误".into() };
        assert!(e.user_facing().contains("B站接口返回错误"));
        assert!(e.user_facing().contains("Api"), "要带分类标签便于归因");
        assert!(e.user_facing().contains("-400"));
        for e in [
            BiliError::NotFound,
            BiliError::Forbidden,
            BiliError::RateLimited,
            BiliError::NotAvailable("审核中".into()),
            BiliError::Transport("timed out".into()),
            BiliError::Malformed("not xml".into()),
            BiliError::BadInput("x".into()),
        ] {
            assert!(!e.user_message().trim().is_empty(), "{e:?}");
            assert!(e.user_facing().contains(e.kind_str()));
        }
    }
}
