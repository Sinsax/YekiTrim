//! 下载器适配层（**端口**）：引擎无关的数据类型 + 适配器注册。
//!
//! ## 这个模块的边界（改之前先读）
//!
//! - **不认识 B站**：这里只出现 "URL"、"清晰度上限"、"cookie 文件" 这类通用概念。
//!   出现 `bvid` / `cid` / `comment.bilibili` 就是嵌合（§2.4 有 grep 判据）。
//! - **不开进程**：只产出 [`CommandSpec`]（纯数据）与"解析一行输出"的结果。
//!   真正的 `tokio::process::Command` 只在 `vtrim-server/download_worker.rs`。
//!   收益：命令构造与输出解析**完全离线可测**，不需要真跑一个下载器。
//!
//! ## 加第二个引擎怎么做
//!
//! 加一个 [`Downloader`] 实现 + 在 [`for_kind`] 里挂上即可，`server` 侧一行不改。
//! 逃生舱（`<绝对路径>`）走的就是这条：它是**同一个适配器**指向另一个二进制。

use std::path::PathBuf;

use anyhow::{bail, Result};

pub mod registry;
pub mod ytdlp;

/// 引擎种类（`[ingest] downloader` 的解析结果）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DownloaderKind {
    /// 内置 yt-dlp：注册表钉死版本与 sha256，随 release 分发（Unlicense）
    YtDlp,
    /// **逃生舱**：用户自备的可执行文件绝对路径，按 yt-dlp 的命令行契约调用。
    /// 不随包分发，因此不在 `THIRD-PARTY-NOTICES.md` 里。
    External(PathBuf),
}

impl DownloaderKind {
    /// 解析 `[ingest] downloader`。
    ///
    /// 判定很直白：等于 `"yt-dlp"`（大小写不敏感）走内置；其余非空值**一律当路径** ——
    /// 不认识的名字（`"bbdown"` 之类）会以"可执行文件不存在"的形式在启动期暴露，
    /// 而不是被静默当成 yt-dlp 用错参数。空值回退内置。
    pub fn parse(raw: &str) -> Self {
        let s = raw.trim();
        if s.is_empty() || s.eq_ignore_ascii_case("yt-dlp") || s.eq_ignore_ascii_case("ytdlp") {
            DownloaderKind::YtDlp
        } else {
            DownloaderKind::External(PathBuf::from(s))
        }
    }

    /// 诊断/文案用（`vtrim doctor`、`/api/health` 的就绪位）。
    pub fn label(&self) -> &'static str {
        match self {
            DownloaderKind::YtDlp => "yt-dlp",
            DownloaderKind::External(_) => "yt-dlp(自备)",
        }
    }
}

/// 一次下载任务的输入（**引擎无关**）。
#[derive(Debug, Clone)]
pub struct DownloadRequest {
    pub url: String,
    /// 托管项目目录（下载器直接写进来，不需要后续搬运）
    pub out_dir: PathBuf,
    /// 输出文件名模板（相对 `out_dir`），默认 `source.%(ext)s`
    pub out_template: String,
    /// 高度上限（`None` = 不限）
    pub max_height: Option<u32>,
    /// 限速 KB/s（`None`/0 = 不限）
    pub rate_limit_kbps: Option<u64>,
    /// cookie 文件（**Netscape 格式**）
    pub cookie_file: Option<PathBuf>,
    /// 只下第 N P（多P 视频）
    pub playlist_item: Option<u32>,
}

impl DownloadRequest {
    pub fn new(url: impl Into<String>, out_dir: impl Into<PathBuf>) -> Self {
        Self {
            url: url.into(),
            out_dir: out_dir.into(),
            out_template: "source.%(ext)s".to_string(),
            max_height: None,
            rate_limit_kbps: None,
            cookie_file: None,
            playlist_item: None,
        }
    }
}

/// 要执行的命令（**纯数据 + 要清掉的环境变量**）。
///
/// 进程边界的完整描述就在这里；`download_worker` 只负责把它 spawn 出来。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandSpec {
    pub program: PathBuf,
    pub args: Vec<String>,
    /// spawn 时必须 `.env_remove()` 的变量名。
    ///
    /// 默认含全部大小写代理变量：实测本机代理会把 yt-dlp 的出网打挂，
    /// 而"继承用户 shell"是隐式的 —— 换成清掉是显式的，不依赖部署环境的偶然状态。
    pub env_remove: Vec<String>,
}

/// 子进程里一律清掉的环境变量（大小写都要，Windows 环境变量大小写不敏感但这里显式列全）。
pub const PROXY_ENV_VARS: [&str; 8] = [
    "HTTP_PROXY",
    "HTTPS_PROXY",
    "ALL_PROXY",
    "NO_PROXY",
    "http_proxy",
    "https_proxy",
    "all_proxy",
    "no_proxy",
];

impl CommandSpec {
    pub fn new(program: impl Into<PathBuf>) -> Self {
        Self {
            program: program.into(),
            args: Vec::new(),
            env_remove: PROXY_ENV_VARS.iter().map(|s| s.to_string()).collect(),
        }
    }

    pub fn arg(mut self, a: impl Into<String>) -> Self {
        self.args.push(a.into());
        self
    }

    pub fn args<I, S>(mut self, items: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.args.extend(items.into_iter().map(Into::into));
        self
    }

    /// 诊断用的一行命令（**会把参数原样显示**，所以调用方不要往里塞 cookie 之外的秘密）。
    pub fn display_line(&self) -> String {
        let mut s = self.program.to_string_lossy().into_owned();
        for a in &self.args {
            s.push(' ');
            if a.contains(' ') {
                s.push('"');
                s.push_str(a);
                s.push('"');
            } else {
                s.push_str(a);
            }
        }
        s
    }
}

/// 从子进程 stdout 解析出的事件（按**哨兵前缀**分流）。
///
/// ⚠️ 进度与最终路径**都在 stdout**（实测）：非 TTY 下 yt-dlp 不给进度条，
/// 必须显式 `--progress`，而那时 `--progress-template` 与 `--print` 混在同一个流里 ——
/// 所以"最后一行就是路径"这类假设是不成立的，只能靠前缀。
#[derive(Debug, Clone, PartialEq)]
pub enum StreamEvent {
    /// 进度百分比（0–100）+ 速度/剩余时间的**原文**（不解析成数字：不同版本格式会变）
    Progress {
        percent: f32,
        speed: String,
        eta: String,
    },
    /// 下载完成后的**最终产物路径**（`after_move` 之后，已含混流与重命名）
    FinalPath(PathBuf),
    /// 产物的分辨率/编码（混流后）
    Meta {
        resolution: String,
        vcodec: String,
        acodec: String,
    },
}

/// 一次下载的产物（成功时）。
#[derive(Debug, Clone, Default)]
pub struct Downloaded {
    /// `--print` 回报的最终路径。**不要靠文件名模板去猜**：B站标题里的
    /// `/ \ : * ? " < >` 会被消毒，猜路径等于"下载成功但项目指向不存在的文件"。
    pub path: Option<PathBuf>,
    pub resolution: Option<String>,
    pub vcodec: Option<String>,
    pub acodec: Option<String>,
}

/// resolve（元数据预览）的结果，**引擎无关**。
///
/// 多P 视频在这里展开：`pages` 空 = 单P（用 `title`/`duration` 即可）。
#[derive(Debug, Clone, Default)]
pub struct ResolveInfo {
    pub title: String,
    pub uploader: String,
    pub duration_secs: Option<f64>,
    /// 封面 URL（引擎给的直链）
    pub thumbnail: Option<String>,
    pub pages: Vec<ResolvedPage>,
}

/// 多P 视频里的一个 P。
#[derive(Debug, Clone, Default)]
pub struct ResolvedPage {
    /// 1-based（与 B站的 `page` 同口径）
    pub index: u32,
    pub title: String,
    pub duration_secs: Option<f64>,
}

/// 错误分类（**按实测关键字**，见 `p4-prerequisite-verification.md` 的 T7）。
///
/// 每一项都带一句**给人看的**文案：这个字符串会直接落进
/// `ingest_sessions.error` 并显示在界面上，所以不能是原始 stderr。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DownloadErrorKind {
    /// 链接形式不认识
    UnsupportedUrl,
    /// 视频不存在/已删除
    NotFound,
    /// 需要登录（配 cookie 文件）
    NeedLogin,
    /// 需要大会员
    MemberOnly,
    /// 上游提取器对这个视频失效（**重试无用**）
    ExtractorBroken,
    /// 请求的清晰度组合不可用（内部错误：说明兜底格式串没写对）
    FormatUnavailable,
    /// 被风控 / 限流（稍后重试）
    RateLimited,
    /// 下载器没装好 / 起不来
    ToolMissing,
    /// 磁盘空间不足
    DiskFull,
    /// 其它（把原始行附在文案里，便于上报）
    Other,
}

impl DownloadErrorKind {
    pub fn as_str(self) -> &'static str {
        match self {
            DownloadErrorKind::UnsupportedUrl => "UnsupportedUrl",
            DownloadErrorKind::NotFound => "NotFound",
            DownloadErrorKind::NeedLogin => "NeedLogin",
            DownloadErrorKind::MemberOnly => "MemberOnly",
            DownloadErrorKind::ExtractorBroken => "ExtractorBroken",
            DownloadErrorKind::FormatUnavailable => "FormatUnavailable",
            DownloadErrorKind::RateLimited => "RateLimited",
            DownloadErrorKind::ToolMissing => "ToolMissing",
            DownloadErrorKind::DiskFull => "DiskFull",
            DownloadErrorKind::Other => "Other",
        }
    }

    /// 给用户看的文案（**人话**，不含原始 stderr）。
    pub fn user_message(self) -> String {
        match self {
            DownloadErrorKind::UnsupportedUrl => {
                "这个链接不支持（当前只支持单个视频链接与它的分P）".to_string()
            }
            DownloadErrorKind::NotFound => "视频不存在或已被删除".to_string(),
            DownloadErrorKind::NeedLogin => {
                "该视频需要登录才能下载；请在设置里配置 B站 cookie 文件后重试".to_string()
            }
            DownloadErrorKind::MemberOnly => "该视频需要大会员账号才能下载".to_string(),
            DownloadErrorKind::ExtractorBroken => {
                "下载器暂时不支持这个视频（上游提取器对该视频失效，重试无用）".to_string()
            }
            DownloadErrorKind::FormatUnavailable => {
                "所选清晰度不可用；请换一档清晰度重试".to_string()
            }
            DownloadErrorKind::RateLimited => {
                "被 B站风控拦截（请求过于频繁）；请稍后重试".to_string()
            }
            DownloadErrorKind::ToolMissing => {
                "下载器未就绪（未安装或不可执行）；请在设置页一键下载 yt-dlp".to_string()
            }
            DownloadErrorKind::DiskFull => {
                "磁盘空间不足；请清理部分项目的源录播后重试".to_string()
            }
            DownloadErrorKind::Other => "下载失败".to_string(),
        }
    }
}

/// 一次失败（分类 + 命中的原始行，用于日志与上报）。
#[derive(Debug, Clone)]
pub struct DownloadError {
    pub kind: DownloadErrorKind,
    /// 命中的原始行（**只进日志，不进界面**）
    pub raw: String,
}

impl DownloadError {
    pub fn new(kind: DownloadErrorKind, raw: impl Into<String>) -> Self {
        Self { kind, raw: raw.into() }
    }

    /// 落库文案：人话 + 分类标签，便于用户复述、也便于我们归因。
    pub fn user_facing(&self) -> String {
        format!("{}（{}）", self.kind.user_message(), self.kind.as_str())
    }
}

/// 引擎端口。
pub trait Downloader: Send + Sync {
    /// 引擎名（诊断用）。
    fn name(&self) -> &'static str;
    /// 元数据命令（resolve 预览卡）。
    fn resolve_spec(&self) -> CommandSpec;
    /// 下载命令。
    fn download_spec(&self, req: &DownloadRequest) -> Result<CommandSpec>;
    /// 按行分流 stdout（非哨兵行返回 `None`）。
    fn parse_stream_line(&self, line: &str) -> Option<StreamEvent>;
    /// 解析 resolve 的完整输出（`-J` 的 JSON）。
    fn parse_resolve(&self, stdout: &str) -> Result<ResolveInfo>;
    /// 从进程输出（stdout+stderr 合并）判定失败原因；返回 `None` = 没看出错。
    fn classify(&self, output: &str) -> Option<DownloadError>;
}

/// 按种类取适配器（**唯一的装配点**）。
///
/// `program` = 已解析好的可执行文件路径（`registry::ensure_ytdlp` 或 `ytdlp_path`）。
pub fn for_kind(kind: &DownloaderKind, program: PathBuf) -> Result<Box<dyn Downloader>> {
    match kind {
        DownloaderKind::YtDlp | DownloaderKind::External(_) => {
            Ok(Box::new(ytdlp::YtDlp::new(program)))
        }
    }
}

/// 多P 校验：`playlist_item` 必须 ≥ 1。
pub fn check_playlist_item(item: Option<u32>) -> Result<()> {
    if let Some(n) = item {
        if n == 0 {
            bail!("分P 序号从 1 开始");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `downloader` 的取值域：不认识的名字要变成**路径**（进而在启动期暴露为"文件不存在"），
    /// 而不是被静默当成内置引擎用错参数。
    #[test]
    fn kind_parses_builtin_vs_escape_hatch() {
        assert_eq!(DownloaderKind::parse(""), DownloaderKind::YtDlp);
        assert_eq!(DownloaderKind::parse("  yt-dlp "), DownloaderKind::YtDlp);
        assert_eq!(DownloaderKind::parse("YTDLP"), DownloaderKind::YtDlp);
        assert_eq!(
            DownloaderKind::parse("D:/tools/bbdown.exe"),
            DownloaderKind::External(PathBuf::from("D:/tools/bbdown.exe"))
        );
        // 不认识的名字也是路径（不是静默回退内置）
        assert_eq!(
            DownloaderKind::parse("bbdown"),
            DownloaderKind::External(PathBuf::from("bbdown"))
        );
        assert_eq!(DownloaderKind::parse("/usr/local/bin/yt-dlp").label(), "yt-dlp(自备)");
    }

    /// ⚠️ 回归钉子：**代理变量必须默认全部清掉**。
    ///
    /// 实测本机代理会把 yt-dlp 的出网打挂。这条挡住"顺手删掉 env_remove"或
    /// 只清大写形式的回退（Windows 大小写不敏感，但其它平台不是）。
    #[test]
    fn command_spec_clears_proxy_env_by_default() {
        let spec = CommandSpec::new("yt-dlp");
        for v in PROXY_ENV_VARS {
            assert!(spec.env_remove.contains(&v.to_string()), "缺少 {v}");
        }
        assert!(
            spec.env_remove.iter().any(|v| v.chars().any(|c| c.is_ascii_lowercase())),
            "小写形式也要清（非 Windows 平台大小写敏感）: {:?}",
            spec.env_remove
        );
    }

    /// 命令行的可读形态（日志/诊断用）——带空格的参数要加引号，否则没法复制粘贴复现。
    #[test]
    fn display_line_quotes_spaces() {
        let spec = CommandSpec::new("yt-dlp")
            .arg("-f")
            .arg("bv*[height<=1080]+ba/b");
        assert_eq!(spec.display_line(), "yt-dlp -f bv*[height<=1080]+ba/b");
        let spec = CommandSpec::new("/opt/my tools/dl").arg("--print").arg("a b");
        assert_eq!(spec.display_line(), "/opt/my tools/dl --print \"a b\"");
    }

    /// 分P 序号从 1 开始（0 会让 yt-dlp 报格式错误，错误分类也认不出来）。
    #[test]
    fn playlist_item_must_be_one_based() {
        assert!(check_playlist_item(None).is_ok());
        assert!(check_playlist_item(Some(1)).is_ok());
        assert!(check_playlist_item(Some(0)).is_err());
    }

    /// 错误文案必须是人话（**不含原始 stderr**），且带上分类标签便于归因。
    #[test]
    fn error_messages_are_human_readable() {
        let e = DownloadError::new(DownloadErrorKind::ExtractorBroken, "ERROR: Unable to extract initial state");
        let msg = e.user_facing();
        assert!(msg.contains("重试无用"), "{msg}");
        assert!(msg.contains("ExtractorBroken"), "{msg}");
        assert!(!msg.contains("ERROR:"), "界面文案不得带原始 stderr: {msg}");
        // 每一类都要有文案，且不能是空的
        for k in [
            DownloadErrorKind::UnsupportedUrl,
            DownloadErrorKind::NotFound,
            DownloadErrorKind::NeedLogin,
            DownloadErrorKind::MemberOnly,
            DownloadErrorKind::ExtractorBroken,
            DownloadErrorKind::FormatUnavailable,
            DownloadErrorKind::RateLimited,
            DownloadErrorKind::ToolMissing,
            DownloadErrorKind::DiskFull,
            DownloadErrorKind::Other,
        ] {
            assert!(!k.user_message().trim().is_empty(), "{:?}", k);
        }
    }
}
