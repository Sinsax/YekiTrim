//! `yt-dlp` 适配器：命令构造 / stdout 哨兵分流 / 错误分类。
//!
//! ## 这里是"纯函数区"
//!
//! 本文件**不 spawn 任何进程**（§2.1 的路径级硬规矩）。它只做两件事：
//! 1. 把 [`DownloadRequest`] 变成 [`CommandSpec`]（纯数据：程序 + 参数 + 要清的 env）；
//! 2. 把子进程吐出来的**一行文本**翻译成 [`StreamEvent`]，或把整个输出翻译成错误分类。
//!
//! 因此本文件的测试**全部离线**：不需要 yt-dlp、不联网。`download_worker` 才是执行方。
//! **也不认识 B站**：这里只有 "URL"、"清晰度上限"、"cookie 文件"。
//!
//! ## 三条实测出来的硬约束（改参数前先读）
//!
//! 1. **哨兵前缀必须在 `WHEN` 段之后**：`--print "after_move:YEKITRIM_PATH:%(filepath)s"`。
//!    写成 `"YEKITRIM_PATH:after_move:filepath"` 会输出 **`NA`**（`YEKITRIM_PATH` 被当成模板名）。
//! 2. **非 TTY 下一行进度都没有**，必须显式 `--progress`；而且 `--progress-template`
//!    与 `--print` 的输出**都落在 stdout**，只能靠前缀分流（不能假设"最后一行是路径"）。
//! 3. **格式串必须带兜底**：`-f "b[height<=360]/b"` 在"该视频没有这个档位"时
//!    yt-dlp 直接 exit 1（`Requested format is not available`），**不会自动回退**。

use std::path::{Path, PathBuf};

use anyhow::{bail, Result};
use serde_json::Value;

use super::{
    CommandSpec, DownloadError, DownloadErrorKind, DownloadRequest, Downloader, ResolvedPage,
    ResolveInfo, StreamEvent,
};

/// 进度行的哨兵前缀。
pub const SENTINEL_PROGRESS: &str = "YEKITRIM_PROG:";
/// 最终产物路径行的哨兵前缀。
pub const SENTINEL_PATH: &str = "YEKITRIM_PATH:";
/// 产物编码信息行的哨兵前缀。
pub const SENTINEL_META: &str = "YEKITRIM_META:";

/// 进度模板的字段顺序（解析时按 `|` 切，与构造处必须一致）。
const PROGRESS_TEMPLATE: &str = "%(progress._percent_str)s|%(progress._speed_str)s|%(progress._eta_str)s";
const META_TEMPLATE: &str = "%(resolution)s|%(vcodec)s|%(acodec)s";

/// yt-dlp 适配器。`program` 是**已解析好的**可执行文件路径（注册表下载的或用户指定的）。
#[derive(Debug, Clone)]
pub struct YtDlp {
    program: PathBuf,
}

impl YtDlp {
    pub fn new(program: impl Into<PathBuf>) -> Self {
        Self { program: program.into() }
    }

    /// 格式选择串。
    ///
    /// **必须带兜底**：`bv*[height<=N]+ba` 在"该视频没有 ≤N 的独立视频流"时直接失败，
    /// 后面的 `/b[height<=N]`（合流格式）与 `/bv*+ba/b`（不限高）才是回退。
    /// 实测教训：`b[height<=360]/b` 这种写法让一个 1080p-only 的视频**直接 exit 1**。
    pub fn format_selector(max_height: Option<u32>) -> String {
        match max_height {
            Some(h) => format!("bv*[height<={h}]+ba/b[height<={h}]/bv*+ba/b"),
            None => "bv*+ba/b".to_string(),
        }
    }

    /// 把 `%(ext)s` 模板拼成绝对输出路径。
    ///
    /// 用绝对路径而不是 `cd`：worker 不切换进程工作目录（多任务并发时那是共享状态）。
    fn output_template(req: &DownloadRequest) -> String {
        let t = if req.out_template.trim().is_empty() {
            "source.%(ext)s"
        } else {
            req.out_template.trim()
        };
        req.out_dir.join(t).to_string_lossy().into_owned()
    }

    /// 把一行拆成"前缀 + 载荷"。
    fn strip<'a>(line: &'a str, prefix: &str) -> Option<&'a str> {
        line.trim().strip_prefix(prefix).map(str::trim)
    }

    /// 「三选一」取第一个非空（yt-dlp 有些字段会回 `NA`）。
    fn pick(v: &Value, key: &str) -> Option<String> {
        let s = v.get(key)?.as_str()?.trim();
        (!s.is_empty() && s != "NA").then(|| s.to_string())
    }

    /// 一个 entry 的标题（多P 时 `title` 常是合集名，`part`/`chapter` 才是分P 名）。
    fn page_title(v: &Value) -> String {
        for key in ["part", "chapter", "title"] {
            if let Some(s) = Self::pick(v, key) {
                return s;
            }
        }
        String::new()
    }

    /// `duration` 可能是数字也可能是 `"NA"`。
    fn duration(v: &Value) -> Option<f64> {
        v.get("duration")
            .and_then(|d| d.as_f64())
            .filter(|d| *d > 0.0)
    }
}

impl Downloader for YtDlp {
    fn name(&self) -> &'static str {
        "yt-dlp"
    }

    /// `-J --skip-download`：拿标题 / UP主 / 时长 / 封面 + 分P 列表。
    fn resolve_spec(&self) -> CommandSpec {
        CommandSpec::new(self.program.clone())
            .arg("-J")
            .arg("--skip-download")
            .arg("--no-warnings")
    }

    fn download_spec(&self, req: &DownloadRequest) -> Result<CommandSpec> {
        super::check_playlist_item(req.playlist_item)?;

        let mut spec = CommandSpec::new(self.program.clone());

        // cookie 必须是 Netscape 格式（JSON 会被静默忽略 —— 表现为"配了 cookie 还是低清"）
        if let Some(c) = &req.cookie_file {
            if !c.is_file() {
                bail!("cookie 文件不存在: {}", c.display());
            }
            spec = spec.arg("--cookies").arg(c.to_string_lossy().into_owned());
        }

        spec = spec
            .arg("-f")
            .arg(Self::format_selector(req.max_height))
            // 避开默认选到的 HEVC：WebUI 预览与部分剪辑链播不了 H.265
            .arg("-S")
            .arg("vcodec:h264")
            .arg("--merge-output-format")
            .arg("mp4");

        if let Some(k) = req.rate_limit_kbps.filter(|k| *k > 0) {
            spec = spec.arg("--limit-rate").arg(format!("{k}K"));
        }

        spec = spec
            .arg("--newline")
            // 不加 --progress，非 TTY 下**一行进度都不会出现**（实测）
            .arg("--progress")
            .arg("--progress-template")
            .arg(format!("{SENTINEL_PROGRESS}{PROGRESS_TEMPLATE}"))
            // ⚠️ 前缀必须在 WHEN 段**之后**；写反了得到的是字面量 NA
            .arg("--print")
            .arg(format!("after_move:{SENTINEL_PATH}%(filepath)s"))
            .arg("--print")
            .arg(format!("{SENTINEL_META}{META_TEMPLATE}"))
            .arg("--no-warnings")
            .arg("-o")
            .arg(Self::output_template(req));

        // ⚠️ `--no-playlist` 等价于"只下第 1 个"；与 `--playlist-items N` 同时给会打架。
        // 所以二选一：指定了第 N P 就用 `--playlist-items`，否则用 `--no-playlist` 防止
        // 一个多P 链接把整季都下下来（那会成倍放大磁盘占用）。
        match req.playlist_item {
            Some(n) => spec = spec.arg("--playlist-items").arg(n.to_string()),
            None => spec = spec.arg("--no-playlist"),
        }

        Ok(spec.arg(&req.url))
    }

    fn parse_stream_line(&self, line: &str) -> Option<StreamEvent> {
        if let Some(payload) = Self::strip(line, SENTINEL_PATH) {
            if payload.is_empty() {
                return None;
            }
            return Some(StreamEvent::FinalPath(PathBuf::from(payload)));
        }
        if let Some(payload) = Self::strip(line, SENTINEL_META) {
            let mut it = payload.split('|');
            let g = |o: Option<&str>| o.unwrap_or("").trim().to_string();
            return Some(StreamEvent::Meta {
                resolution: g(it.next()),
                vcodec: g(it.next()),
                acodec: g(it.next()),
            });
        }
        if let Some(payload) = Self::strip(line, SENTINEL_PROGRESS) {
            let mut it = payload.split('|');
            let pct_raw = it.next().unwrap_or("").trim();
            let speed = it.next().unwrap_or("").trim().to_string();
            let eta = it.next().unwrap_or("").trim().to_string();
            // 形如 " 12.3%"（带前导空格）；解析不出来就跳过这一行（不 panic、不报假进度）
            let percent = pct_raw.trim_end_matches('%').trim().parse::<f32>().ok()?;
            return Some(StreamEvent::Progress {
                percent: percent.clamp(0.0, 100.0),
                speed,
                eta,
            });
        }
        None
    }

    fn parse_resolve(&self, stdout: &str) -> Result<ResolveInfo> {
        let v: Value = serde_json::from_str(stdout.trim())
            .map_err(|e| anyhow::anyhow!("解析下载器输出失败（不是合法 JSON）: {e}"))?;

        // 多P：`-J` 返回的是 playlist（`_type = "playlist"`，条目在 `entries`）。
        // ⚠️ `entries` 里可能有 `null`（yt-dlp 对失败的条目就是塞 null）。
        let entries = v.get("entries").and_then(|e| e.as_array());
        let is_playlist = v.get("_type").and_then(|t| t.as_str()) == Some("playlist")
            || entries.is_some();

        if is_playlist {
            let entries = entries.map(|e| e.as_slice()).unwrap_or(&[]);
            let pages: Vec<ResolvedPage> = entries
                .iter()
                .filter(|e| !e.is_null())
                .enumerate()
                .map(|(i, e)| {
                    let index = e
                        .get("playlist_index")
                        .and_then(|x| x.as_u64())
                        .map(|x| x as u32)
                        .unwrap_or_else(|| i as u32 + 1);
                    ResolvedPage {
                        index,
                        title: Self::page_title(e),
                        duration_secs: Self::duration(e),
                    }
                })
                .collect();
            if pages.is_empty() {
                bail!("下载器没有返回任何可下载的分P");
            }
            // 合集层的 title/uploader 取条目里第一个非空的（playlist 顶层常是空的）
            let first = entries.iter().find(|e| !e.is_null());
            let pick_first = |key: &str| -> Option<String> {
                first.and_then(|e| Self::pick(e, key))
            };
            return Ok(ResolveInfo {
                title: Self::pick(&v, "title").or_else(|| pick_first("title")).unwrap_or_default(),
                uploader: Self::pick(&v, "uploader")
                    .or_else(|| Self::pick(&v, "channel"))
                    .or_else(|| pick_first("uploader"))
                    .unwrap_or_default(),
                // 多P 的"总时长"没有意义（前端显示各 P 的），留 None 让调用方自己合计
                duration_secs: None,
                thumbnail: Self::pick(&v, "thumbnail").or_else(|| pick_first("thumbnail")),
                pages,
            });
        }

        let title = Self::pick(&v, "title").ok_or_else(|| {
            anyhow::anyhow!("下载器输出里没有 title（视频可能不可用）")
        })?;
        Ok(ResolveInfo {
            title,
            uploader: Self::pick(&v, "uploader")
                .or_else(|| Self::pick(&v, "channel"))
                .unwrap_or_default(),
            duration_secs: Self::duration(&v),
            thumbnail: Self::pick(&v, "thumbnail"),
            pages: Vec::new(),
        })
    }

    fn classify(&self, output: &str) -> Option<DownloadError> {
        // 顺序有讲究：先把"更具体"的判掉，避免被泛化的关键字抢先。
        // 每条的关键字都来自实测（见 p4-prerequisite-verification.md T7）。
        const RULES: &[(DownloadErrorKind, &[&str])] = &[
            // 上游提取器对这个视频失效（legacy 视频缺 __INITIAL_STATE__，issue #14793）。
            // 放在 NotFound 之前：它的原文里常带别的一串东西，认错了会误导用户去重试。
            (
                DownloadErrorKind::ExtractorBroken,
                &["Unable to extract initial state", "Unable to extract webpage video data"],
            ),
            (DownloadErrorKind::FormatUnavailable, &["Requested format is not available"]),
            (
                DownloadErrorKind::NeedLogin,
                &[
                    "only available for registered users",
                    "Sign in to confirm",
                    "This video is only available for registered users",
                    "Login required",
                ],
            ),
            (
                DownloadErrorKind::MemberOnly,
                &["Premium", "大会员", "VIP", "This video is only available to VIP"],
            ),
            (
                DownloadErrorKind::RateLimited,
                &["HTTP Error 412", "HTTP Error 429", "412 Precondition Failed"],
            ),
            (DownloadErrorKind::NotFound, &["HTTP Error 404", "HTTP Error 410"]),
            (
                DownloadErrorKind::UnsupportedUrl,
                &["Unsupported URL", "is not a valid URL"],
            ),
            (
                DownloadErrorKind::DiskFull,
                &["No space left on device", "os error 112"],
            ),
            (
                DownloadErrorKind::ToolMissing,
                &["Access is denied", "cannot execute", "No such file or directory"],
            ),
        ];

        // 只看带 ERROR 的行：yt-dlp 的 WARNING 里也会出现 `HTTP Error 404` 这类字样
        // （例如封面 404 但它照样把视频下完了），扫全文会把"成功的下载"判成失败。
        let mut hit: Option<(&DownloadErrorKind, String)> = None;
        for line in output.lines() {
            let trimmed = line.trim();
            if !trimmed.starts_with("ERROR:") {
                continue;
            }
            for (kind, keys) in RULES {
                if keys.iter().any(|k| trimmed.contains(k)) {
                    hit = Some((kind, trimmed.to_string()));
                    break;
                }
            }
            // 第一行 ERROR 命中哪条就认哪条：后面的 ERROR 往往是被前面的失败带出来的次生错误
            if hit.is_some() {
                break;
            }
            // 有 ERROR 但一条规则都没命中 → Other（保留原文便于上报）
            hit = Some((&DownloadErrorKind::Other, trimmed.to_string()));
            break;
        }

        hit.map(|(kind, raw)| DownloadError::new(*kind, raw))
    }
}

/// 用一个真实产物路径取它的"项目相对名"（写进 `yekitrim.toml` 的 `input_file`）。
///
/// 单独拆出来是为了**可离线测试**：`--print` 回报的是绝对路径，而 `write_project_config`
/// 需要相对项目根的路径。
pub fn relative_input_path(project_dir: &Path, final_path: &Path) -> Result<String> {
    let rel = final_path.strip_prefix(project_dir).map_err(|_| {
        anyhow::anyhow!(
            "下载产物落在了项目目录之外（{} 不在 {} 下）",
            final_path.display(),
            project_dir.display()
        )
    })?;
    let name = rel.to_string_lossy().replace('\\', "/");
    if name.is_empty() {
        bail!("下载产物路径为空");
    }
    Ok(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dl() -> YtDlp {
        YtDlp::new("/opt/tools/yt-dlp")
    }

    fn req() -> DownloadRequest {
        DownloadRequest {
            url: "https://example.com/v/1".into(),
            out_dir: PathBuf::from("/data/users/7/room/20260101-000000-1"),
            out_template: "source.%(ext)s".into(),
            max_height: Some(1080),
            rate_limit_kbps: None,
            cookie_file: None,
            playlist_item: None,
        }
    }

    /// ⚠️ 命令构造快照：挡住"参数回退"。
    ///
    /// 每一条都有实测依据（T6）：
    /// - `-S vcodec:h264`：不加会默认选到 **HEVC**，WebUI 预览播不了；
    /// - `--progress`：不加，非 TTY 下**一行进度都没有**；
    /// - `after_move:YEKITRIM_PATH:%(filepath)s`：前缀写反会输出字面量 `NA`（实测）；
    /// - `--no-playlist`：不加，一个多P 链接会把整季下下来。
    #[test]
    fn download_args_snapshot() {
        let req = req();
        // `-o` 的值必须用 `PathBuf` 拼而不是字面量：Windows 上分隔符是 `\`，
        // 字面量的 `/data/.../source.%(ext)s` 会在本机假红（这条真红过一次）。
        let out_arg = req.out_dir.join("source.%(ext)s").to_string_lossy().into_owned();
        let spec = dl().download_spec(&req).unwrap();
        assert_eq!(spec.program, PathBuf::from("/opt/tools/yt-dlp"));
        assert_eq!(
            spec.args,
            vec![
                "-f",
                "bv*[height<=1080]+ba/b[height<=1080]/bv*+ba/b",
                "-S",
                "vcodec:h264",
                "--merge-output-format",
                "mp4",
                "--newline",
                "--progress",
                "--progress-template",
                "YEKITRIM_PROG:%(progress._percent_str)s|%(progress._speed_str)s|%(progress._eta_str)s",
                "--print",
                "after_move:YEKITRIM_PATH:%(filepath)s",
                "--print",
                "YEKITRIM_META:%(resolution)s|%(vcodec)s|%(acodec)s",
                "--no-warnings",
                "-o",
                out_arg.as_str(),
                "--no-playlist",
                "https://example.com/v/1",
            ]
        );
        // 代理必须被清掉（实测本机代理会把 yt-dlp 打挂）
        assert!(spec.env_remove.iter().any(|v| v == "HTTPS_PROXY"));
        // 哨兵前缀**必须**在 WHEN 段之后（写反 → 输出 NA）
        let print_idx = spec
            .args
            .iter()
            .position(|a| a == "after_move:YEKITRIM_PATH:%(filepath)s")
            .expect("--print 的形态必须带 after_move 前缀");
        assert!(spec.args[print_idx].starts_with("after_move:"));
    }

    /// 限速 / cookie / 分P 是可选项：给了才出现，且分P 与 `--no-playlist` **互斥**。
    #[test]
    fn optional_args_are_mutually_exclusive_where_it_matters() {
        let dir = std::env::temp_dir().join(format!("yekitrim-cookie-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let cookie = dir.join("cookies.txt");
        std::fs::write(&cookie, b"# Netscape HTTP Cookie File\n").unwrap();

        let mut r = req();
        r.rate_limit_kbps = Some(512);
        r.max_height = None;
        r.cookie_file = Some(cookie.clone());
        r.playlist_item = Some(3);
        let spec = dl().download_spec(&r).unwrap();
        let a = &spec.args;
        assert!(a.windows(2).any(|w| w == ["--cookies", cookie.to_str().unwrap()]));
        assert!(a.windows(2).any(|w| w == ["--limit-rate", "512K"]));
        // 不限高 → 不留 height 过滤
        assert!(a.windows(2).any(|w| w == ["-f", "bv*+ba/b"]));
        // 分P：用 --playlist-items，且**不能**再塞 --no-playlist（两者会打架）
        assert!(a.windows(2).any(|w| w == ["--playlist-items", "3"]));
        assert!(!a.iter().any(|x| x == "--no-playlist"), "{a:?}");

        // cookie 文件不存在 → 早失败（错误分类认不出来的那种失败要在这里挡住）
        r.cookie_file = Some(dir.join("nope.txt"));
        assert!(dl().download_spec(&r).is_err());

        // 分P 序号 0 → 早失败
        r.cookie_file = None;
        r.playlist_item = Some(0);
        assert!(dl().download_spec(&r).is_err());

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// ⚠️ 回归钉子：**进度模板的字段顺序**与解析必须一一对应。
    ///
    /// 实测：`--progress-template` 与 `--print` 的输出**都在 stdout**，
    /// 所以进度的载荷里出现 `|` 是正常现象（速度里就有），不能靠切分次数判断类型。
    #[test]
    fn stream_lines_split_by_sentinel_not_by_position() {
        let d = dl();
        // 真实形态：`--percent_str` 带前导空格
        assert_eq!(
            d.parse_stream_line("YEKITRIM_PROG:  0.0%|  13.37KiB/s|00:12"),
            Some(StreamEvent::Progress { percent: 0.0, speed: "13.37KiB/s".into(), eta: "00:12".into() })
        );
        assert_eq!(
            d.parse_stream_line("YEKITRIM_PROG:100.0%|   2.00MiB/s|00:00"),
            Some(StreamEvent::Progress { percent: 100.0, speed: "2.00MiB/s".into(), eta: "00:00".into() })
        );
        // 真实形态：after_move 回报的是**绝对路径**
        assert_eq!(
            d.parse_stream_line(r"YEKITRIM_PATH:F:\para\Code\YekiTrim\proj\source.mp4"),
            Some(StreamEvent::FinalPath(PathBuf::from(r"F:\para\Code\YekiTrim\proj\source.mp4")))
        );
        assert_eq!(
            d.parse_stream_line("YEKITRIM_META:640x360|avc1.64001E|mp4a.40.2"),
            Some(StreamEvent::Meta {
                resolution: "640x360".into(),
                vcodec: "avc1.64001E".into(),
                acodec: "mp4a.40.2".into()
            })
        );
        // 非哨兵行（yt-dlp 的其它输出）一律忽略，不能误判成路径
        assert_eq!(d.parse_stream_line("[download] Destination: x.mp4"), None);
        assert_eq!(d.parse_stream_line(""), None);
        // 新的哨兵行也自动忽略（前向兼容）
        assert_eq!(d.parse_stream_line("YEKITRIM_FUTURE:whatever"), None);
        // 进度载荷解析不出来时**跳过**这一行，而不是当成 0%
        assert_eq!(d.parse_stream_line("YEKITRIM_PROG:NA|NA|NA"), None);
        // 空路径不算产物
        assert_eq!(d.parse_stream_line("YEKITRIM_PATH:"), None);
    }

    /// 错误分类：关键字全部来自实测输出。
    ///
    /// ⚠️ 两个"必须活着"的钉子：
    /// - `Unable to extract initial state` 是**独立错误类**，文案要写明"重试无用"
    ///   （不然用户会一直点重试）；
    /// - `Requested format is not available` 说明**兜底格式串没写对**，不是用户的问题。
    #[test]
    fn error_classification_matches_observed_output() {
        let d = dl();
        let cases = [
            (
                "ERROR: [BiliBili] 1wP4y1P72h: Unable to extract initial state; please report this issue",
                DownloadErrorKind::ExtractorBroken,
            ),
            (
                "ERROR: [BiliBili] 1: Requested format is not available. Use --list-formats",
                DownloadErrorKind::FormatUnavailable,
            ),
            (
                "ERROR: [BiliBili] 1: This video is only available for registered users",
                DownloadErrorKind::NeedLogin,
            ),
            ("ERROR: unable to download video data: HTTP Error 412", DownloadErrorKind::RateLimited),
            ("ERROR: [BiliBili] 1: HTTP Error 404: Not Found", DownloadErrorKind::NotFound),
            ("ERROR: Unsupported URL: https://example.com/x", DownloadErrorKind::UnsupportedUrl),
            (
                "ERROR: unable to write data: [Errno 28] No space left on device",
                DownloadErrorKind::DiskFull,
            ),
            ("ERROR: [BiliBili] 大会员专享视频", DownloadErrorKind::MemberOnly),
            // 没认出来的 ERROR → Other（带原文），不能是 None（否则会被当成"没出错"）
            ("ERROR: something nobody has seen before", DownloadErrorKind::Other),
        ];
        for (raw, want) in cases {
            let got = d.classify(raw).unwrap_or_else(|| panic!("应当能分类: {raw}"));
            assert_eq!(got.kind, want, "原文: {raw}");
            assert!(got.user_facing().contains(want.as_str()));
        }

        // ⚠️ 回归钉子：**WARNING 里的 404 不能判成失败**。
        // yt-dlp 在封面 404 时照样会把视频下完；扫全文会把成功的下载判成失败。
        let ok_run = "WARNING: Unable to download thumbnail: HTTP Error 404: Not Found\n\
                      [download] 100% of 19.40MiB in 00:00:06 at 3.06MiB/s";
        assert!(d.classify(ok_run).is_none(), "成功的下载不得被判成失败");

        // 干净结束 → 不出错
        assert!(d.classify("[download] 100% of 19.40MiB").is_none());
        // 第一行 ERROR 说了算（后面的往往是被它带出来的次生错误）
        let multi = "ERROR: [BiliBili] 1: HTTP Error 404: Not Found\nERROR: unable to download video data: HTTP Error 412";
        assert_eq!(d.classify(multi).unwrap().kind, DownloadErrorKind::NotFound);
    }

    /// resolve：单P 输出（字段名取自实测的 `-J` 输出）。
    #[test]
    fn resolve_parses_single_video() {
        let json = r#"{
            "_type": "video",
            "title": "游戏帧数增加40%？…",
            "uploader": "小夫太渴",
            "duration": 313.557,
            "thumbnail": "http://i0.hdslb.com/bfs/archive/7799.jpg",
            "formats": [{"height": 360}, {"height": 1080}]
        }"#;
        let r = dl().parse_resolve(json).unwrap();
        assert!(r.title.starts_with("游戏帧数"));
        assert_eq!(r.uploader, "小夫太渴");
        assert!((r.duration_secs.unwrap() - 313.557).abs() < 1e-3);
        assert_eq!(r.thumbnail.as_deref(), Some("http://i0.hdslb.com/bfs/archive/7799.jpg"));
        assert!(r.pages.is_empty(), "单P 不展开分P");
    }

    /// resolve：多P 输出 —— 展开 `entries`，跳过 `null`，缺 `playlist_index` 时按位置补。
    #[test]
    fn resolve_expands_playlist_and_skips_nulls() {
        let json = r#"{
            "_type": "playlist",
            "id": "BV1bK411W797",
            "title": "合集",
            "entries": [
                {"playlist_index": 1, "part": "P1 标题", "duration": 100.0},
                null,
                {"playlist_index": 3, "title": "P3 标题", "duration": null}
            ]
        }"#;
        let r = dl().parse_resolve(json).unwrap();
        assert_eq!(r.pages.len(), 2, "null 条目要跳过");
        assert_eq!(r.pages[0].index, 1);
        assert_eq!(r.pages[0].title, "P1 标题");
        assert!((r.pages[0].duration_secs.unwrap() - 100.0).abs() < 1e-6);
        assert_eq!(r.pages[1].index, 3);
        assert_eq!(r.pages[1].title, "P3 标题");
        assert!(r.pages[1].duration_secs.is_none(), "duration 为 null 不能变成 0");
        // 多P 的"总时长"没有意义 → 不编一个出来
        assert!(r.duration_secs.is_none());
    }

    /// 垃圾 / 空输出要报错，不能静默给一个空标题的预览卡。
    #[test]
    fn resolve_rejects_garbage() {
        assert!(dl().parse_resolve("").is_err());
        assert!(dl().parse_resolve("not json at all").is_err());
        assert!(dl().parse_resolve(r#"{"_type":"video"}"#).is_err(), "没有 title 要报错");
        assert!(dl().parse_resolve(r#"{"_type":"playlist","entries":[null]}"#).is_err(),
            "一条可下载的分P 都没有要报错");
    }

    /// 产物相对路径：`--print` 给的是绝对路径，写进 `yekitrim.toml` 要相对项目根。
    #[test]
    fn final_path_must_live_inside_project_dir() {
        let dir = PathBuf::from(r"F:\proj\bili-BV1-p1");
        let got = relative_input_path(&dir, &dir.join("source.mp4")).unwrap();
        assert_eq!(got, "source.mp4");
        // 落在外面的路径必须报错（否则项目会指向一个不存在于自己的文件）
        assert!(relative_input_path(&dir, Path::new(r"F:\elsewhere\source.mp4")).is_err());
    }
}
