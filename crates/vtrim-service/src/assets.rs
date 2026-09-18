//! 项目素材（弹幕 / 字幕）的选择。
//!
//! 预处理页允许用户**随时切换**这两份素材，值就存在 `vtrim.toml` 的
//! `[project] danmaku_file` / `transcript_file`（相对工程根；工程外的绝对路径仅 admin）。
//! 本模块负责：列候选、按类别校验、写值。
//!
//! ⚠️ 解析口径统一在 [`Workspace::transcript_path`] / [`Workspace::danmaku_path`] 与
//! [`super::Workspace::resolve`] —— 两处都相对**工程根**。别在这里另起一套。

use std::path::Path;

use anyhow::{bail, Context, Result};
use serde::Serialize;

use super::Workspace;

/// 扫描候选时跳过的目录名（产物 / 内部状态 / 素材库，都不是可选输入）。
const SKIP_DIRS: [&str; 5] = ["output", "clips", ".vtrim", "stickers", "assets"];

/// 可选择的素材类别。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssetKind {
    /// 弹幕（热度评分 + `clip.dm.srt`）
    Danmaku,
    /// 转写字幕（事件检测的输入）
    Transcript,
}

impl AssetKind {
    pub fn parse(raw: &str) -> Result<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "danmaku" | "dm" | "弹幕" => Ok(Self::Danmaku),
            "transcript" | "subtitle" | "sub" | "字幕" => Ok(Self::Transcript),
            other => bail!("未知素材类别 {other:?}（支持 danmaku / transcript）"),
        }
    }

    /// `vtrim.toml` 里对应的键名（`[project]` 段）。
    pub fn config_key(self) -> &'static str {
        match self {
            Self::Danmaku => "danmaku_file",
            Self::Transcript => "transcript_file",
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Danmaku => "danmaku",
            Self::Transcript => "transcript",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Danmaku => "弹幕",
            Self::Transcript => "字幕",
        }
    }
}

/// 一个文件名能当哪几类素材用（只看扩展名）。
///
/// - `xml`（录播姬 / B站标准弹幕）、`ass` → 弹幕
///   （本项目的 ASS 解析器是**弹幕**解析器，字幕 ASS 不支持，所以不列进字幕候选）
/// - `srt` → **两类都列**：从文件名分不出是弹幕还是字幕，交给用户决定
/// - `json` → 字幕（`transcript.json` 结构）
pub fn classify(name: &str) -> Vec<AssetKind> {
    let ext = Path::new(name)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    match ext.as_str() {
        "xml" | "ass" => vec![AssetKind::Danmaku],
        "srt" => vec![AssetKind::Danmaku, AssetKind::Transcript],
        "json" => vec![AssetKind::Transcript],
        _ => Vec::new(),
    }
}

/// 一类素材的当前状态（**只 stat，不解析**：3 小时录播的弹幕 XML 有几十 MB，
/// 每次打开预处理页都解析一遍是不可接受的）。
#[derive(Debug, Clone, Serialize)]
pub struct AssetStatus {
    /// `vtrim.toml` 里配置的值（空 = 未配置）。弹幕只有这一条来源。
    pub value: String,
    /// 实际生效的绝对路径（未生效时为空）。
    pub abs: String,
    pub exists: bool,
    pub size: u64,
    /// 生效值是不是**用户提供的外部文件**（在 `output/` 之外）。
    /// 只有字幕有此语义：外部字幕意味着这个工程跳过过 ASR。
    pub external: bool,
    /// 生效值来自配置（false = 来自 `output/` 兜底）
    pub from_config: bool,
}

impl AssetStatus {
    fn empty() -> Self {
        Self {
            value: String::new(),
            abs: String::new(),
            exists: false,
            size: 0,
            external: false,
            from_config: false,
        }
    }
}

/// 一类素材的候选（项目内的文件）。
#[derive(Debug, Clone, Serialize)]
pub struct Candidate {
    pub name: String,
    /// 项目内相对路径（统一 `/` 分隔；非 admin 必须用这个值提交）
    pub rel: String,
    pub abs: String,
    pub size: u64,
    /// 修改时间（unix 秒；取不到为 0）
    pub mtime: u64,
    /// 与输入视频同名（只差扩展名）—— 排序与前端徽标都用它
    pub same_stem: bool,
}

/// 查询一类素材的当前状态。
pub fn asset_status(ws: &Workspace, cfg: &vtrim_core::config::Config, kind: AssetKind) -> AssetStatus {
    match kind {
        // 弹幕只认配置：管线里真正读弹幕的地方（`runner::inject_config`）就只看 danmaku_file，
        // 这里多报一个"视频同目录的 danmu.ass 也算数"会让界面撒谎。那种兜底只服务于
        // LLM 订正（`load_danmaku_for_correct`），而它现在也会出现在候选列表里供一键采纳。
        AssetKind::Danmaku => {
            let raw = cfg
                .project
                .danmaku_file
                .as_ref()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .unwrap_or_default();
            if raw.is_empty() {
                return AssetStatus::empty();
            }
            let abs = ws.resolve(&raw);
            let meta = std::fs::metadata(&abs).ok();
            AssetStatus {
                value: raw,
                exists: meta.as_ref().map(|m| m.is_file()).unwrap_or(false),
                size: meta.as_ref().map(|m| m.len()).unwrap_or(0),
                abs: vtrim_core::path::display(&abs),
                external: true,
                from_config: true,
            }
        }
        // 字幕报**实际生效**的那一份（配置 → output/transcript.json → output/video.srt），
        // 因为检测阶段读的就是这个顺序 —— 界面上写的必须就是它用的。
        AssetKind::Transcript => {
            let raw = cfg
                .project
                .transcript_file
                .as_ref()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .unwrap_or_default();
            if !raw.is_empty() {
                let abs = ws.resolve(&raw);
                let meta = std::fs::metadata(&abs).ok();
                let exists = meta.as_ref().map(|m| m.is_file()).unwrap_or(false);
                // ⚠️ 配置指向的文件不在时**照实报"文件不存在"**，不要静默飘到 output/ 兜底上：
                // 管线确实会退到 `output/`，但用户选的这份没了是他必须知道的事。
                return AssetStatus {
                    value: raw,
                    abs: vtrim_core::path::display(&abs),
                    exists,
                    size: meta.as_ref().map(|m| m.len()).unwrap_or(0),
                    external: true,
                    from_config: true,
                };
            }
            // 未配置 → 报 `output/` 里的产物（不是"外部字幕"）
            let Some(path) = ws.transcript_path(cfg) else {
                return AssetStatus::empty();
            };
            let output = ws.output_dir();
            let meta = std::fs::metadata(&path).ok();
            AssetStatus {
                value: String::new(),
                exists: true,
                size: meta.as_ref().map(|m| m.len()).unwrap_or(0),
                external: !path.starts_with(&output),
                abs: vtrim_core::path::display(&path),
                from_config: false,
            }
        }
    }
}

/// 扫描项目内的素材候选：项目根 + **一层**子目录。
///
/// 跳过产物与内部目录（[`SKIP_DIRS`]）与点开头目录。排序：与视频同名优先 → 修改时间倒序 → 名字。
pub fn scan_candidates(
    ws: &Workspace,
    cfg: &vtrim_core::config::Config,
    kind: AssetKind,
) -> Vec<Candidate> {
    let stem = cfg
        .project
        .input_file
        .as_ref()
        .filter(|s| !s.is_empty())
        .map(|s| ws.resolve(s))
        .and_then(|p| p.file_stem().map(|s| s.to_string_lossy().to_ascii_lowercase()))
        .unwrap_or_default();

    let mut dirs = vec![ws.root_dir.clone()];
    if let Ok(entries) = std::fs::read_dir(&ws.root_dir) {
        for entry in entries.flatten() {
            if !entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                continue;
            }
            let name = entry.file_name().to_string_lossy().to_string();
            let lower = name.to_ascii_lowercase();
            if name.starts_with('.') || SKIP_DIRS.contains(&lower.as_str()) {
                continue;
            }
            dirs.push(entry.path());
        }
    }

    let mut out: Vec<Candidate> = Vec::new();
    for dir in dirs {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            if !entry.file_type().map(|t| t.is_file()).unwrap_or(false) {
                continue;
            }
            let name = entry.file_name().to_string_lossy().to_string();
            if !classify(&name).contains(&kind) {
                continue;
            }
            let path = entry.path();
            let meta = entry.metadata().ok();
            let rel = ws.rel(&path);
            out.push(Candidate {
                same_stem: !stem.is_empty()
                    && path
                        .file_stem()
                        .map(|s| s.to_string_lossy().to_ascii_lowercase() == stem)
                        .unwrap_or(false),
                rel: rel.to_string_lossy().replace('\\', "/"),
                abs: vtrim_core::path::display(&path),
                size: meta.as_ref().map(|m| m.len()).unwrap_or(0),
                mtime: meta
                    .as_ref()
                    .and_then(|m| m.modified().ok())
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_secs())
                    .unwrap_or(0),
                name,
            });
        }
    }
    out.sort_by(|a, b| {
        b.same_stem
            .cmp(&a.same_stem)
            .then(b.mtime.cmp(&a.mtime))
            .then(a.name.cmp(&b.name))
    });
    out
}

/// 选定一类素材：空路径 = 清除（回落到自动/兜底）。
///
/// 返回 `(写入 vtrim.toml 的值, 解析出的条数)`。条数让前端立刻给出
/// 「弹幕 12,345 条」这种确认，同时也是**写前校验**：解析失败一律不落盘 ——
/// 存进去一个坏文件只会让后面的检测静默退化成"没有弹幕/没有字幕"。
///
/// 落在工程根内的一律存**相对路径**（可移植）；工程外只可能是 admin（非 admin 被
/// [`vtrim_core::tenant::jail_join`] 挡掉）。
pub fn set_asset(
    ws: &Workspace,
    kind: AssetKind,
    raw_path: &str,
    is_admin: bool,
) -> Result<(String, usize)> {
    let trimmed = raw_path.trim();
    if trimmed.is_empty() {
        write_value(ws, kind, "")?;
        return Ok((String::new(), 0));
    }
    let abs = ws
        .resolve(trimmed)
        .canonicalize()
        .with_context(|| format!("{}文件不存在: {trimmed}", kind.label()))?;
    if !abs.is_file() {
        bail!("{}不是文件: {}", kind.label(), abs.display());
    }
    if !is_admin {
        // 与 `config_vtrim_put_impl` 的 `check_project_paths` 同一口径：
        // 非 admin 只允许工程内相对路径（绝对路径 / `..` / 盘符在这一步就被拒）。
        vtrim_core::tenant::jail_join(&ws.root_dir, trimmed)
            .with_context(|| format!("{}必须是项目内相对路径", kind.label()))?;
    }
    let count = count_entries(kind, &abs)?;
    // ⚠️ 用**规范化后**的根做前缀比较：`canonicalize` 在 Windows 上会加 `\\?\` 前缀，
    // 拿它跟 `root_dir`（未规范化、可能还经过 junction）比一定对不上，
    // 于是"工程内的文件"会被当成外部路径、原样存绝对路径进 vtrim.toml。
    let root = ws
        .root_dir
        .canonicalize()
        .unwrap_or_else(|_| ws.root_dir.clone());
    let stored = match abs.strip_prefix(&root) {
        Ok(rel) => rel.to_string_lossy().replace('\\', "/"),
        Err(_) => vtrim_core::path::display(&abs),
    };
    write_value(ws, kind, &stored)?;
    Ok((stored, count))
}

fn write_value(ws: &Workspace, kind: AssetKind, value: &str) -> Result<()> {
    let patch = serde_json::json!({ "project": { kind.config_key(): value } });
    vtrim_core::config::patch_config_file(&ws.config_file, &patch)
        .with_context(|| format!("写入 {} 配置失败", kind.label()))
}

/// 预解析一份素材，返回条数（失败即视为不可用）。
fn count_entries(kind: AssetKind, path: &Path) -> Result<usize> {
    match kind {
        AssetKind::Danmaku => {
            let entries = vtrim_core::danmaku::load_danmaku_auto(path)
                .with_context(|| format!("弹幕解析失败: {}", path.display()))?;
            if entries.is_empty() {
                bail!("弹幕文件里没有解析出任何弹幕: {}", path.display());
            }
            Ok(entries.len())
        }
        AssetKind::Transcript => {
            let raw = std::fs::read_to_string(path)
                .with_context(|| format!("字幕读取失败: {}", path.display()))?;
            let ext = path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_ascii_lowercase();
            let segments = if ext == "json" {
                let t: vtrim_core::types::Transcript = serde_json::from_str(&raw)
                    .with_context(|| format!("字幕 JSON 结构不对（需要 transcript.json 的结构）: {}", path.display()))?;
                t.segments.len()
            } else {
                vtrim_core::srt::parse_srt(&raw)
                    .with_context(|| format!("字幕解析失败: {}", path.display()))?
                    .segments
                    .len()
            };
            if segments == 0 {
                bail!("字幕文件里没有解析出任何字幕: {}", path.display());
            }
            Ok(segments)
        }
    }
}

/// 新建项目时按来源类型**预载**哪类素材。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Preload {
    /// 不预载（本机文件：由用户在预处理页自由选择）
    None,
    /// 预载弹幕（录播姬）
    Danmaku,
    /// 预载字幕（B站下载：「有字幕就自动载入」）
    Subtitle,
}

/// 来源类型 → 预载策略。
///
/// - `recorder`：录播姬的弹幕在入库时就被搬到项目根并写进 `vtrim.toml`
///   （`server::ingest_worker::find_danmaku`），这里把这条既定行为登记进策略表；
/// - `local`（服务器本机文件）：**不预载** —— 用户要求"自由选择"，建项目时不做同名探测，
///   由预处理页列候选供一键点选（`create_local_project` 按本策略决定要不要 `.ass` 自动发现）；
/// - `bilibili`：下载来源应当"有字幕就自动载入"。**该来源仍是 P4 未实现**，
///   所以这个分支目前没有调用方 —— B站下载落地时照 `ingest_worker::find_danmaku` 的同名探测
///   填 `transcript_file`（策略表这里已经写好了，别再另写一套判断）。
pub fn preload_policy(source_kind: &str) -> Preload {
    match source_kind {
        "recorder" => Preload::Danmaku,
        "bilibili" => Preload::Subtitle,
        _ => Preload::None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn ws_at(root: &Path) -> Workspace {
        Workspace {
            config_file: root.join("vtrim.toml"),
            root_dir: root.to_path_buf(),
        }
    }

    #[test]
    fn classify_by_extension() {
        assert_eq!(classify("a.xml"), vec![AssetKind::Danmaku]);
        assert_eq!(classify("a.ASS"), vec![AssetKind::Danmaku]);
        assert_eq!(
            classify("a.srt"),
            vec![AssetKind::Danmaku, AssetKind::Transcript]
        );
        assert_eq!(classify("a.json"), vec![AssetKind::Transcript]);
        assert!(classify("a.mp4").is_empty());
        assert!(classify("noext").is_empty());
    }

    #[test]
    fn asset_kind_parse_accepts_aliases() {
        assert_eq!(AssetKind::parse("danmaku").unwrap(), AssetKind::Danmaku);
        assert_eq!(AssetKind::parse(" TRANSCRIPT ").unwrap(), AssetKind::Transcript);
        assert_eq!(AssetKind::parse("弹幕").unwrap(), AssetKind::Danmaku);
        assert!(AssetKind::parse("unknown").is_err());
    }

    #[test]
    fn scan_lists_same_stem_first_and_skips_output() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::create_dir_all(root.join("output")).unwrap();
        fs::create_dir_all(root.join(".vtrim")).unwrap();
        fs::create_dir_all(root.join("videos")).unwrap();
        fs::write(root.join("a.mp4"), b"x").unwrap();
        fs::write(root.join("a.xml"), b"<i/>").unwrap();
        fs::write(root.join("other.ass"), b"[Events]\n").unwrap();
        fs::write(root.join("output/transcript.json"), b"{}").unwrap();
        fs::write(root.join("videos/a.srt"), b"1\n").unwrap();
        fs::write(root.join("a.mp4"), b"x").unwrap();

        let ws = ws_at(root);
        let mut cfg = vtrim_core::config::Config::default();
        cfg.project.input_file = Some("a.mp4".to_string());

        let dm = scan_candidates(&ws, &cfg, AssetKind::Danmaku);
        // 同名的两条排在最前（两者之间按 mtime 倒序，测试里不断言具体次序），output/ 里的产物不算候选
        assert_eq!(dm.len(), 3);
        let names: Vec<&str> = dm.iter().map(|c| c.name.as_str()).collect();
        assert!(dm[0].same_stem && dm[1].same_stem, "同名候选必须排最前: {names:?}");
        assert!(!dm[2].same_stem);
        assert_eq!(dm[2].name, "other.ass");
        let rels: Vec<&str> = dm.iter().map(|c| c.rel.as_str()).collect();
        assert!(rels.contains(&"a.xml") && rels.contains(&"videos/a.srt"), "{rels:?}");

        // 字幕候选只收 srt/json，且 output/ 里的产物不算候选
        let sub = scan_candidates(&ws, &cfg, AssetKind::Transcript);
        let names: Vec<&str> = sub.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, vec!["a.srt"]);
        assert_eq!(sub[0].rel, "videos/a.srt");
    }

    #[test]
    fn set_asset_stores_relative_path_and_counts() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::create_dir_all(root.join("output")).unwrap();
        fs::write(
            root.join("dm.xml"),
            "<?xml version=\"1.0\"?><i><d p=\"1.5,1,25,16777215,0\">hi</d><d p=\"2.5,1,25,16777215,0\">yo</d></i>",
        )
        .unwrap();
        let ws = ws_at(root);
        let (value, count) = set_asset(&ws, AssetKind::Danmaku, "dm.xml", false).unwrap();
        assert_eq!(value, "dm.xml");
        assert_eq!(count, 2);
        let cfg = ws.load_config().unwrap();
        assert_eq!(cfg.project.danmaku_file.as_deref(), Some("dm.xml"));

        // 清除
        let (value, count) = set_asset(&ws, AssetKind::Danmaku, "", false).unwrap();
        assert_eq!(value, "");
        assert_eq!(count, 0);
        let cfg = ws.load_config().unwrap();
        assert_eq!(cfg.project.danmaku_file.as_deref(), Some(""));
    }

    #[test]
    fn set_asset_rejects_missing_and_unparsable() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::write(root.join("bad.srt"), "这不是字幕\n").unwrap();
        let ws = ws_at(root);
        assert!(set_asset(&ws, AssetKind::Transcript, "missing.srt", false).is_err());
        assert!(set_asset(&ws, AssetKind::Transcript, "bad.srt", false).is_err());
        // 拼不出字幕 → 不落盘
        let cfg = ws.load_config().unwrap();
        assert!(cfg
            .project
            .transcript_file
            .as_deref()
            .map(|s| s.is_empty())
            .unwrap_or(true));
    }

    #[test]
    fn set_asset_rejects_outside_project_for_non_admin() {
        let dir = tempfile::tempdir().unwrap();
        let other = tempfile::tempdir().unwrap();
        let outside = other.path().join("dm.xml");
        fs::write(&outside, "<?xml version=\"1.0\"?><i><d p=\"1.5,1,25,16777215,0\">hi</d></i>").unwrap();
        let ws = ws_at(dir.path());
        // 非 admin：绝对路径一律拒
        assert!(set_asset(&ws, AssetKind::Danmaku, &outside.to_string_lossy(), false).is_err());
        // admin：允许，且因为不在工程根内而原样存绝对路径
        let (value, count) = set_asset(&ws, AssetKind::Danmaku, &outside.to_string_lossy(), true).unwrap();
        assert_eq!(count, 1);
        assert!(Path::new(&value).is_absolute());
    }

    #[test]
    fn set_asset_accepts_transcript_json() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::write(
            root.join("sub.json"),
            r#"{"segments":[{"start":0.0,"end":1.0,"text":"a"},{"start":1.0,"end":2.0,"text":"b"}],"language":"zh"}"#,
        )
        .unwrap();
        let ws = ws_at(root);
        let (value, count) = set_asset(&ws, AssetKind::Transcript, "sub.json", false).unwrap();
        assert_eq!(value, "sub.json");
        assert_eq!(count, 2);
        let cfg = ws.load_config().unwrap();
        assert_eq!(cfg.project.transcript_file.as_deref(), Some("sub.json"));
    }

    #[test]
    fn transcript_status_prefers_config_and_marks_external() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::create_dir_all(root.join("output")).unwrap();
        fs::write(root.join("sub.srt"), "1\n00:00:01,000 --> 00:00:02,000\nhi\n").unwrap();
        fs::write(root.join("output/video.srt"), "1\n00:00:01,000 --> 00:00:02,000\nasr\n").unwrap();
        let ws = ws_at(root);
        let mut cfg = vtrim_core::config::Config::default();

        // 无配置 → 报 output/ 兜底，且不是"外部字幕"
        let st = asset_status(&ws, &cfg, AssetKind::Transcript);
        assert!(st.exists && !st.external && !st.from_config);
        assert_eq!(st.value, "");

        // 指向外部字幕 → 生效值就是它
        cfg.project.transcript_file = Some("sub.srt".to_string());
        let st = asset_status(&ws, &cfg, AssetKind::Transcript);
        assert!(st.exists && st.external && st.from_config);
        assert_eq!(st.value, "sub.srt");

        // 配置指向不存在的文件 → exists=false（不伪装成未设置）
        cfg.project.transcript_file = Some("nope.srt".to_string());
        let st = asset_status(&ws, &cfg, AssetKind::Transcript);
        assert!(!st.exists && st.from_config);
        assert_eq!(st.value, "nope.srt");
    }

    #[test]
    fn preload_policy_matches_sources() {
        assert_eq!(preload_policy("recorder"), Preload::Danmaku);
        assert_eq!(preload_policy("bilibili"), Preload::Subtitle);
        assert_eq!(preload_policy("local"), Preload::None);
        assert_eq!(preload_policy("whatever"), Preload::None);
    }
}
