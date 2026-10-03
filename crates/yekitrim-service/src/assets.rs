//! 项目素材（弹幕 / 字幕）的选择与**清单**。
//!
//! 预处理页允许用户**随时切换**这两份素材，值就存在 `yekitrim.toml` 的
//! `[project] danmaku_file` / `transcript_file`（相对工程根；工程外的绝对路径仅 admin）。
//! 本模块负责：列候选、按类别校验、写值、维护清单。
//!
//! 清单（`[project.materials]`，2026-09 起）解决的是"能不能切回来"：
//!
//! - **ASR 链路产物**按模式占槽（[`ASR_MODES`] 只有 hybrid / single）→ 最多两条、
//!   带 `source = "asr"` 标记、**不可删除**。用户说的"输出一次绑定一次、只能两次"就是这个。
//!   绑定由[`bind_asr_artifact`]完成：管线每次 ASR 跑完（`core_pipeline` / `correct`）
//!   调一次 —— **别在别处再写一份**，绑定必须和产物落盘同源。
//! - **用户自己加的**（手动选的项目内文件）不占槽、可删除。
//! - **弹幕没有 ASR 制约**，条目形态与字幕完全一致，只是不占槽。
//!
//! ⚠️ 解析口径统一在 [`Workspace::transcript_path`] / [`Workspace::danmaku_path`] 与
//! [`Workspace::resolve`]（两处都相对**工程根**）。别在这里另起一套。
//! 清单只回答"列表里有哪些候选"，**下游一律只认当前生效值**（那两个 `*_file` 键）。
//!
//! ⚠️ 写 `yekitrim.toml` 一律走 [`yekitrim_core::config::patch_config_file`]，而且**一次写完**
//! （生效值 + 整份清单在同一个 patch 里）：它内部是 `with_path_lock_sync`，而那个锁
//! **非重入**（std Mutex，外层已持锁时再调就自锁）—— 分两次写既会自锁、又会留下
//! "值写了清单没写"的中间态。

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde::Serialize;

use yekitrim_core::config::{Config, MaterialEntry, ASR_MODES, MATERIAL_SOURCE_ASR};

use super::Workspace;

/// 扫描候选时跳过的目录名（产物 / 内部状态 / 素材库，都不是可选输入）。
const SKIP_DIRS: [&str; 5] = ["output", "clips", yekitrim_core::path::PROJECT_STATE_DIR_NAME, "stickers", "assets"];

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

    /// `yekitrim.toml` 里对应的键名（`[project]` 段）。
    pub fn config_key(self) -> &'static str {
        match self {
            Self::Danmaku => "danmaku_file",
            Self::Transcript => "transcript_file",
        }
    }

    /// 清单里的数组名（`[project.materials]` 段）。
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

/// 清单里的一条素材（`[project.materials]`）+ 从盘上 stat 出来的状态。
///
/// 前端拿它渲染列表：`source == "asr"` 打标记、`removable == false` 不给删、
/// `active` 表示"当前生效的就是它"。
#[derive(Debug, Clone, Serialize)]
pub struct AssetItem {
    /// 写进/读自 `yekitrim.toml` 的路径（相对工程根；工程外只可能是 admin，那时是绝对路径）。
    pub path: String,
    /// 绝对路径（展示用；已去掉 Windows 的 `\\?\` 逐字前缀）。
    pub abs: String,
    /// 文件名（列表标题）。
    pub name: String,
    /// 来源：`"asr"` = ASR 链路产物；空串 / 其它值 = 用户素材。
    pub source: String,
    /// 是不是 ASR 链路产物（打标记 + 不可删 + 占模式槽）。
    pub is_asr: bool,
    /// ASR 模式槽（`hybrid` / `single`；非 ASR 条目为空串）。
    pub mode: String,
    /// 能不能从清单里删掉。
    ///
    /// ASR 产物默认不行；**但它的文件已经不在了就放行** —— 那才是真的"槽位空出来"，
    /// 否则删过一次文件的工程会被一条永远删不掉的死条目卡住。
    pub removable: bool,
    pub exists: bool,
    pub size: u64,
    /// 在 `output/` 之外（用户自己放的外部素材）。
    pub external: bool,
    /// 是不是当前**生效**的那一份。
    pub active: bool,
}

/// 一类素材的当前状态：**生效值** + **清单**。
///
/// **只 stat，不解析**：3 小时录播的弹幕 XML 有几十 MB，每次打开预处理页都解析一遍
/// 是不可接受的（解析只发生在真正选定一个文件时，见 [`set_asset`]）。
#[derive(Debug, Clone, Serialize)]
pub struct AssetStatus {
    /// `yekitrim.toml` 里配置的值（空 = 未配置）。弹幕只有这一条来源。
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
    /// 清单：列表里能一键切换的候选（ASR 产物排在前面，各自保持登记顺序）。
    pub items: Vec<AssetItem>,
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
            items: Vec::new(),
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

/// 查询一类素材的当前状态（生效值 + 清单）。
pub fn asset_status(ws: &Workspace, cfg: &Config, kind: AssetKind) -> AssetStatus {
    let mut status = own_status(ws, cfg, kind);
    status.items = list_items(ws, cfg, kind);
    status
}

/// 当前**生效**的那份文件的绝对路径（清单里的 `active` 标记按它算）。
///
/// - 弹幕只认 [`configured_value`]：管线里真正读弹幕的地方（`runner::inject_config`）
///   就只看 `danmaku_file`，这里多报一个"视频同目录的 danmu.ass 也算数"会让界面撒谎；
/// - 字幕走 [`Workspace::transcript_path`]（配置 → `output/transcript.json` → `output/video.srt`），
///   因为检测阶段读的就是这个顺序 —— 界面上写的必须就是它用的。
fn effective_path(ws: &Workspace, cfg: &Config, kind: AssetKind) -> Option<PathBuf> {
    match kind {
        AssetKind::Danmaku => configured_value(cfg, kind).map(|v| ws.resolve(&v)),
        AssetKind::Transcript => ws.transcript_path(cfg),
    }
}

/// 配置里写的值（去掉首尾空白；空 = 未配置）。
fn configured_value(cfg: &Config, kind: AssetKind) -> Option<String> {
    let raw = match kind {
        AssetKind::Danmaku => cfg.project.danmaku_file.as_ref(),
        AssetKind::Transcript => cfg.project.transcript_file.as_ref(),
    }?;
    let raw = raw.trim();
    (!raw.is_empty()).then(|| raw.to_string())
}

/// 生效值本身的状态（不含清单）。
fn own_status(ws: &Workspace, cfg: &Config, kind: AssetKind) -> AssetStatus {
    let Some(raw) = configured_value(cfg, kind) else {
        // 未配置：只有字幕会退到 `output/` 里的产物（弹幕没有兜底，见 `effective_path`）。
        if kind != AssetKind::Transcript {
            return AssetStatus::empty();
        }
        let Some(path) = ws.transcript_path(cfg) else {
            return AssetStatus::empty();
        };
        let output = ws.output_dir();
        let meta = std::fs::metadata(&path).ok();
        return AssetStatus {
            exists: true,
            size: meta.as_ref().map(|m| m.len()).unwrap_or(0),
            external: !path.starts_with(&output),
            abs: yekitrim_core::path::display(&path),
            ..AssetStatus::empty()
        };
    };
    let abs = ws.resolve(&raw);
    let meta = std::fs::metadata(&abs).ok();
    // ⚠️ 配置指向的文件不在时**照实报"文件不存在"**，不要静默飘到 output/ 兜底上：
    // 管线确实会退到 `output/`，但用户选的这份没了是他必须知道的事。
    AssetStatus {
        value: raw,
        exists: meta.as_ref().map(|m| m.is_file()).unwrap_or(false),
        size: meta.as_ref().map(|m| m.len()).unwrap_or(0),
        // 配了值就算"外部"（沿用老口径，前端那条 chip 用它）
        external: true,
        from_config: true,
        abs: yekitrim_core::path::display(&abs),
        items: Vec::new(),
    }
}

/// 一类素材的清单：`[project.materials]` 里登记的条目 + **盘上已存在但没登记的 ASR 产物**。
///
/// 后者是给两类情况兜底的：本功能上线前跑过 ASR 的老工程、以及绑定写入失败（配置被手改回滚）。
/// 它们只是"能被列出来并选中"（选中即补登记，见 [`set_asset`]），不影响下游读取口径。
pub fn list_items(ws: &Workspace, cfg: &Config, kind: AssetKind) -> Vec<AssetItem> {
    let output = ws.output_dir();
    let active = effective_path(ws, cfg, kind);
    let mut seen: Vec<PathBuf> = Vec::new();
    let mut out: Vec<AssetItem> = Vec::new();

    for entry in cfg.project.materials.of(kind.as_str()) {
        let raw = entry.path.trim();
        if raw.is_empty() {
            continue;
        }
        let abs = ws.resolve(raw);
        // 手改 toml 把同一个文件写了两遍 → 只列一次
        if seen.iter().any(|p| same_path(p, &abs)) {
            continue;
        }
        seen.push(abs.clone());
        out.push(build_item(
            raw,
            &abs,
            &output,
            active.as_deref(),
            entry.source.clone(),
            entry.slot(),
        ));
    }

    for mode in ASR_MODES {
        let Some(abs) = asr_artifact(kind, mode, &output) else {
            continue;
        };
        if seen.iter().any(|p| same_path(p, &abs)) {
            continue;
        }
        seen.push(abs.clone());
        let stored = config_path(ws, &abs);
        out.push(build_item(
            &stored,
            &abs,
            &output,
            active.as_deref(),
            MATERIAL_SOURCE_ASR.to_string(),
            mode.to_string(),
        ));
    }

    // 配了值却**没登记进清单**的那些（本功能上线前的老工程、绑定写入失败被手改回滚）：
    // 也要列出来，否则界面上"正在用的那份"在列表里找不到，用户连切走都费劲。
    // 它只是"能被看见并切走"，选中即补登记（[`set_asset`]）。
    //
    // ⚠️ 放在 ASR 产物兜底**之后**：`output/transcript.<mode>.json` 还在盘上时已经被上一段
    // 认成 ASR 条目（带模式标记、不可删），先加就会把它标成"用户素材"（可删且不占槽）。
    if let Some(raw) = configured_value(cfg, kind) {
        let abs = ws.resolve(&raw);
        if !seen.iter().any(|p| same_path(p, &abs)) {
            seen.push(abs.clone());
            out.push(build_item(
                &raw,
                &abs,
                &output,
                active.as_deref(),
                String::new(),
                String::new(),
            ));
        }
    }

    // ASR 产物在前（它们是"清单的主干"），用户素材在后；各自保持登记顺序。
    out.sort_by_key(|item| !item.is_asr);
    out
}

fn build_item(
    stored: &str,
    abs: &Path,
    output: &Path,
    active: Option<&Path>,
    source: String,
    mode: String,
) -> AssetItem {
    let is_asr = source.eq_ignore_ascii_case(MATERIAL_SOURCE_ASR);
    let meta = std::fs::metadata(abs).ok();
    let exists = meta.as_ref().map(|m| m.is_file()).unwrap_or(false);
    AssetItem {
        name: abs
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| stored.to_string()),
        path: stored.to_string(),
        abs: yekitrim_core::path::display(abs),
        source,
        is_asr,
        mode,
        // ASR 产物不可删；文件已经不在了那是例外（见 `AssetItem::removable`）
        removable: !is_asr || !exists,
        exists,
        size: meta.as_ref().map(|m| m.len()).unwrap_or(0),
        external: !abs.starts_with(output),
        active: active.map(|a| same_path(a, abs)).unwrap_or(false),
    }
}

/// 某个模式的分模式产物**文件名**：`transcript.<mode>.json`。
///
/// 与 `yekitrim-core/src/asr/mod.rs::mode_slug` 的规则**必须一致**（那边写、这边读）：
/// 小写、长度 ≤ 32、只允许 ASCII 字母数字与 `-` / `_`。模式串来自配置，手改成
/// 乱七八糟的值那边根本不落盘，所以这里也只认这一套（拼不出名字就当没有）。
fn asr_artifact_name(mode: &str) -> Option<String> {
    let mode = mode.trim().to_ascii_lowercase();
    if mode.is_empty() || mode.len() > 32 {
        return None;
    }
    if !mode
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return None;
    }
    Some(format!("transcript.{mode}.json"))
}

/// 某个模式的 ASR 产物落点（**已落盘**才是 `Some`）：`output/transcript.<mode>.json`。
///
/// 只认这一个位置：`video.srt` / `transcript.json` 是"最新一次"的覆盖式产物，
/// 挂到某个模式上会撒谎 —— 两条清单条目指着同一份文件就没法切换了，那正是要修的毛病。
fn asr_artifact(kind: AssetKind, mode: &str, output: &Path) -> Option<PathBuf> {
    if kind != AssetKind::Transcript {
        return None;
    }
    let path = output.join(asr_artifact_name(mode)?);
    path.is_file().then_some(path)
}

/// ASR 刚产出字幕时的**绑定**：把该模式的分模式产物登记进清单，并置为生效值。
///
/// 返回写进 `transcript_file` 的路径；**绑不上时返回 `None`**（调用方退回
/// `output/video.srt`）。用户的口径是"输出一次绑定一次、只能两次"：
/// [`upsert_entry`] 按模式槽唯一，同模式重跑覆盖自己那一条，所以清单里最多两条
/// `source = "asr"` 的条目（hybrid / single 各一），[`slot_bound`] 据此把用掉的模式置灰。
///
/// 产物没落盘（`[asr] mode` 被手改成奇怪的值 → core 的 `mode_slug` 返回 `None`）时
/// **不绑**：`transcript_file` 指着一个不存在的文件会让检测直接报"未找到转写文件"，
/// 比少一条清单条目坏得多。
pub fn bind_asr_artifact(ws: &Workspace, output: &Path, mode: &str) -> Option<String> {
    let artifact = output.join(asr_artifact_name(mode)?);
    if !artifact.is_file() {
        return None;
    }
    let stored = config_path(ws, &artifact);
    bind_or_warn(ws, AssetKind::Transcript, &stored, Some(MATERIAL_SOURCE_ASR), Some(mode))
}

/// 把一份**派生**素材登记进清单并置为生效值（LLM 订正后的 `output/video_llm.srt`）。
///
/// 与 [`bind_asr_artifact`] 的区别只在于**不占模式槽**：它不来自某一次 ASR，而是
/// 订正"当前这份字幕"的产物，所以按普通（可删除）条目登记。
pub fn bind_derived_material(ws: &Workspace, kind: AssetKind, abs: &Path) -> Option<String> {
    let stored = config_path(ws, abs);
    bind_or_warn(ws, kind, &stored, None, None)
}

/// 登记并置为生效值；失败**只警告**。
///
/// 走到这里时产物已经写好在盘上了，清单少一条不该把整条管线带崩
/// （[`list_items`] 会把盘上未登记的 ASR 产物兜底列出来，用户点一下就能补登记）。
fn bind_or_warn(
    ws: &Workspace,
    kind: AssetKind,
    stored: &str,
    source: Option<&str>,
    mode: Option<&str>,
) -> Option<String> {
    match bind_material(ws, kind, stored, source, mode) {
        Ok(()) => Some(stored.to_string()),
        Err(e) => {
            tracing::warn!("登记{}素材失败（不影响产物本身）: {e:#}", kind.label());
            None
        }
    }
}

/// 这个绝对路径是不是某个模式的 ASR 产物（`output/transcript.<mode>.json`）。
///
/// 认出来是为了**按槽登记**而不是当成用户素材：老工程 / 绑定写入失败时清单里没有它，
/// 用户在列表里点一下也应该占住那个模式槽（否则同模式还能再转写一次）。
fn asr_slot_of_path(ws: &Workspace, kind: AssetKind, abs: &Path) -> Option<String> {
    if kind != AssetKind::Transcript {
        return None;
    }
    let output = ws.output_dir();
    if !abs.parent().map(|p| same_path(p, &output)).unwrap_or(false) {
        return None;
    }
    let name = abs.file_name()?.to_str()?.to_ascii_lowercase();
    name.strip_prefix("transcript.")?
        .strip_suffix(".json")
        .filter(|m| !m.is_empty())
        .map(|m| m.to_string())
}

/// 绝对路径 → 写进 `yekitrim.toml` 的形态（工程根内用相对路径，可移植；根外原样绝对路径）。
///
/// ⚠️ 用**规范化后**的根做前缀比较：`canonicalize` 在 Windows 上会加 `\\?\` 前缀，
/// 拿它跟 `root_dir`（未规范化、可能还经过 junction）比一定对不上，
/// 于是"工程内的文件"会被当成外部路径、原样存绝对路径进 yekitrim.toml。
///
/// 管线侧（`core_pipeline` / `correct`）往清单里登记产物时也走这里，
/// 别各自再拼一份 `strip_prefix`（分隔符与 `\\?\` 两种口径都要一致）。
pub fn config_path(ws: &Workspace, abs: &Path) -> String {
    let root = ws
        .root_dir
        .canonicalize()
        .unwrap_or_else(|_| ws.root_dir.clone());
    let canon = abs.canonicalize().unwrap_or_else(|_| abs.to_path_buf());
    match canon.strip_prefix(&root) {
        Ok(rel) => rel.to_string_lossy().replace('\\', "/"),
        Err(_) => yekitrim_core::path::display(abs),
    }
}

/// 两个路径是不是同一个文件（先按字面比，再退化到 canonicalize）。
fn same_path(a: &Path, b: &Path) -> bool {
    if a == b {
        return true;
    }
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(x), Ok(y)) => x == y,
        _ => false,
    }
}

/// 扫描项目内的素材候选：项目根 + **一层**子目录。
///
/// 跳过产物与内部目录（[`SKIP_DIRS`]）与点开头目录。排序：与视频同名优先 → 修改时间倒序 → 名字。
pub fn scan_candidates(ws: &Workspace, cfg: &Config, kind: AssetKind) -> Vec<Candidate> {
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
                abs: yekitrim_core::path::display(&path),
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

/// 选定一类素材：置为**生效值**，并登记进清单，方便以后一键切回来。
///
/// 返回 `(写入 yekitrim.toml 的路径, 解析出的条数)`。条数让前端立刻给出
/// 「弹幕 12,345 条」这种确认，同时也是**写前校验**：解析失败一律不落盘 ——
/// 存进去一个坏文件只会让后面的检测静默退化成"没有弹幕/没有字幕"。
///
/// 空路径 = 清掉生效值（**不动清单**：列表里的候选还在，随时能切回来）。
///
/// 落在工程根内的一律存**相对路径**（可移植）；工程外只可能是 admin（非 admin 被
/// [`yekitrim_core::tenant::jail_join`] 挡掉）。
///
/// 选中的正好是某个模式的 ASR 产物（`output/transcript.<mode>.json`）时按**槽**登记
/// （带 `asr` 标记、不可删），而不是当用户素材 —— 否则"换个文件再点回来"就能把
/// 已经用掉的模式槽洗白，同模式又能转写一次。
pub fn set_asset(
    ws: &Workspace,
    kind: AssetKind,
    raw_path: &str,
    is_admin: bool,
) -> Result<(String, usize)> {
    let trimmed = raw_path.trim();
    if trimmed.is_empty() {
        write_state(ws, kind, Some(""), None)?;
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
        // 与 `config_yekitrim_put_impl` 的 `check_project_paths` 同一口径：
        // 非 admin 只允许工程内相对路径（绝对路径 / `..` / 盘符在这一步就被拒）。
        yekitrim_core::tenant::jail_join(&ws.root_dir, trimmed)
            .with_context(|| format!("{}必须是项目内相对路径", kind.label()))?;
    }
    let count = count_entries(kind, &abs)?;
    let stored = config_path(ws, &abs);
    match asr_slot_of_path(ws, kind, &abs) {
        Some(mode) => bind_material(ws, kind, &stored, Some(MATERIAL_SOURCE_ASR), Some(&mode))?,
        None => bind_material(ws, kind, &stored, None, None)?,
    }
    Ok((stored, count))
}

/// 把一条素材登记进清单并置为**生效值**（一次 patch 写完两个键，见模块头）。
///
/// `source = Some("asr")` + `mode` → 按**模式槽**登记：同模式重跑覆盖自己那一条，
/// 不会越攒越多（"输出一次绑定一次、只能两次"）。
pub fn bind_material(
    ws: &Workspace,
    kind: AssetKind,
    stored: &str,
    source: Option<&str>,
    mode: Option<&str>,
) -> Result<()> {
    let stored = stored.trim();
    if stored.is_empty() {
        bail!("{}路径为空", kind.label());
    }
    let cfg = ws.load_config()?;
    let mut entries = cfg.project.materials.of(kind.as_str()).to_vec();
    upsert_entry(&mut entries, ws, stored, source, mode);
    write_state(ws, kind, Some(stored), Some(&entries))
        .with_context(|| format!("登记{}素材失败", kind.label()))
}

fn upsert_entry(
    entries: &mut Vec<MaterialEntry>,
    ws: &Workspace,
    stored: &str,
    source: Option<&str>,
    mode: Option<&str>,
) {
    let slot = mode
        .map(|m| m.trim().to_ascii_lowercase())
        .filter(|m| !m.is_empty());
    if source.is_some_and(|s| s.eq_ignore_ascii_case(MATERIAL_SOURCE_ASR)) {
        // ASR 产物按模式槽唯一：同模式重跑 = 换掉自己那一条，不新增
        if let Some(slot) = slot.as_deref() {
            entries.retain(|e| !(e.is_asr() && e.slot() == slot));
        }
        let mut entry = MaterialEntry::asr(stored.to_string(), slot.as_deref().unwrap_or(""));
        if slot.is_none() {
            entry.mode = None;
        }
        entries.push(entry);
        return;
    }
    let abs = ws.resolve(stored);
    if let Some(existing) = entries
        .iter_mut()
        .find(|e| same_path(&ws.resolve(&e.path), &abs))
    {
        // 同一个文件只登记一次；它已经是 ASR 产物就保持"占槽、不可删"
        existing.path = stored.to_string();
        if !existing.is_asr() {
            existing.source = String::new();
            existing.mode = None;
        }
        return;
    }
    entries.push(MaterialEntry::user(stored.to_string()));
}

/// 从清单里删掉一条素材。
///
/// - **ASR 产物**（文件还在）拒绝：那一条代表一个已用掉的模式槽，删它要有明确后果；
///   文件已经不在了才放行（[`AssetItem::removable`] 同口径）；
/// - 删掉的正好是**生效值**时一并清空 `*_file`：清单里没了、管线还在用它是最难查的状态。
pub fn remove_material(ws: &Workspace, kind: AssetKind, raw: &str) -> Result<()> {
    let raw = raw.trim();
    if raw.is_empty() {
        bail!("未指定要删除的{}", kind.label());
    }
    let cfg = ws.load_config()?;
    let mut entries = cfg.project.materials.of(kind.as_str()).to_vec();
    let abs = ws.resolve(raw);
    let clearing = configured_value(&cfg, kind)
        .map(|v| same_path(&ws.resolve(&v), &abs))
        .unwrap_or(false);
    let mut found = false;
    if let Some(pos) = entries
        .iter()
        .position(|e| same_path(&ws.resolve(&e.path), &abs))
    {
        if entries[pos].is_asr() && abs.is_file() {
            bail!(
                "{}是 ASR 链路产物（模式 {}），不能删除",
                kind.label(),
                entries[pos].slot()
            );
        }
        entries.remove(pos);
        found = true;
    }
    // 没登记过、但正是**当前生效值**（老工程 / [`list_items`] 兜底列出来的那条）也放行 ——
    // 那条的语义是"停止使用"（清空 `*_file`），它本来就不在清单里，删不掉才是死角。
    if !found && !clearing {
        bail!("清单里没有这条{}: {raw}", kind.label());
    }
    write_state(ws, kind, clearing.then_some(""), Some(&entries))
        .with_context(|| format!("更新{}清单失败", kind.label()))
}

/// 转写门控：这个模式的 ASR 槽**是否已被占用**（清单登记过，或盘上已有分模式产物）。
///
/// 两种模式各只能出一次产物（用户要求），所以 `[asr] mode` 对应的槽占了就不许再转写 ——
/// 与从前 `转写文件存在 → 409` 是同一种语义，只是按模式分开数。
pub fn slot_bound(ws: &Workspace, cfg: &Config, kind: AssetKind, mode: &str) -> bool {
    let mode = mode.trim().to_ascii_lowercase();
    if mode.is_empty() {
        return false;
    }
    if cfg
        .project
        .materials
        .of(kind.as_str())
        .iter()
        .any(|e| e.slot() == mode)
    {
        return true;
    }
    let output = ws.output_dir();
    asr_artifact(kind, &mode, &output).is_some()
}

/// 写 `[project]`：`value`（`*_file` 生效值，`Some("")` = 清空）+ `entries`（整份清单）。
///
/// 两者在**同一个 patch** 里写下去：分两次调用会连撞两次 [`yekitrim_core::config::patch_config_file`]
/// 的路径锁（非重入），中间那一小段还会出现"值指向新文件、清单还是旧的"的错位。
fn write_state(
    ws: &Workspace,
    kind: AssetKind,
    value: Option<&str>,
    entries: Option<&[MaterialEntry]>,
) -> Result<()> {
    let mut project = serde_json::Map::new();
    if let Some(value) = value {
        project.insert(kind.config_key().to_string(), serde_json::json!(value));
    }
    if let Some(entries) = entries {
        let list: Vec<serde_json::Value> = entries
            .iter()
            .map(|e| {
                let mut obj = serde_json::Map::new();
                obj.insert("path".into(), serde_json::json!(e.path));
                obj.insert("source".into(), serde_json::json!(e.source));
                if let Some(mode) = e.mode.as_ref() {
                    obj.insert("mode".into(), serde_json::json!(mode));
                }
                serde_json::Value::Object(obj)
            })
            .collect();
        project.insert(
            "materials".to_string(),
            serde_json::json!({ kind.as_str(): list }),
        );
    }
    if project.is_empty() {
        return Ok(());
    }
    yekitrim_core::config::patch_config_file(&ws.config_file, &serde_json::json!({ "project": project }))
        .with_context(|| format!("写入 {} 配置失败", kind.label()))
}

/// 预解析一份素材，返回条数（失败即视为不可用）。
fn count_entries(kind: AssetKind, path: &Path) -> Result<usize> {
    match kind {
        AssetKind::Danmaku => {
            let entries = yekitrim_core::danmaku::load_danmaku_auto(path)
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
                let t: yekitrim_core::types::Transcript = serde_json::from_str(&raw)
                    .with_context(|| format!("字幕 JSON 结构不对（需要 transcript.json 的结构）: {}", path.display()))?;
                t.segments.len()
            } else {
                yekitrim_core::srt::parse_srt(&raw)
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
    /// 预载弹幕（录播姬、B站链接下载）
    Danmaku,
    /// 预载字幕。
    ///
    /// ⚠️ **当前没有任何来源映射到它**（`bilibili` 已改为 `Danmaku`）。
    /// 保留是因为"以字幕为主"的来源仍可能出现。**别把 `bilibili` 改回来**：
    /// B站的字幕是 CC 字幕（P4 明确不下载），而弹幕才是热度排序（`popularity`）的输入 ——
    /// B站项目预载字幕 = 稳定丢掉排序质量，而且**这个错不会报错**。
    Subtitle,
}

/// 来源类型 → 预载策略。
///
/// - `recorder`：录播姬的弹幕在入库时就被搬到项目根并写进 `yekitrim.toml`
///   （`server::ingest_worker::find_danmaku`），这里把这条既定行为登记进策略表；
/// - `local`（服务器本机文件）：**不预载** —— 用户要求"自由选择"，建项目时不做同名探测，
///   由预处理页列候选供一键点选（`create_local_project` 按本策略决定要不要 `.ass` 自动发现）；
/// - `bilibili`（链接下载，P4）：**预载弹幕**。与录播姬同构 —— 下载完落到
///   `<项目根>/danmaku.xml` 并写进 `yekitrim.toml`。B站的字幕是 CC 字幕且不是输入，
///   预载成字幕只会让热度排序静默变差（见 `Preload::Subtitle` 的说明）。
pub fn preload_policy(source_kind: &str) -> Preload {
    match source_kind {
        "recorder" | "bilibili" => Preload::Danmaku,
        _ => Preload::None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn ws_at(root: &Path) -> Workspace {
        Workspace {
            config_file: root.join("yekitrim.toml"),
            root_dir: root.to_path_buf(),
        }
    }

    /// 一份最小可用的 `transcript.json`（`set_asset` 会真的解析它来数条数）。
    fn write_transcript_json(path: &Path) {
        let t = serde_json::json!({
            "segments": [{"start": 0.0, "end": 1.0, "text": "你好"}],
            "language": "zh",
        });
        fs::write(path, serde_json::to_string(&t).unwrap()).unwrap();
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
        fs::create_dir_all(root.join(yekitrim_core::path::PROJECT_STATE_DIR_NAME)).unwrap();
        fs::create_dir_all(root.join("videos")).unwrap();
        fs::write(root.join("a.mp4"), b"x").unwrap();
        fs::write(root.join("a.xml"), b"<i/>").unwrap();
        fs::write(root.join("other.ass"), b"[Events]\n").unwrap();
        fs::write(root.join("output/transcript.json"), b"{}").unwrap();
        fs::write(root.join("videos/a.srt"), b"1\n").unwrap();
        fs::write(root.join("a.mp4"), b"x").unwrap();

        let ws = ws_at(root);
        let mut cfg = Config::default();
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
        // 选过就进清单（可删除的用户素材，不占 ASR 槽）
        assert_eq!(cfg.project.materials.danmaku.len(), 1);
        assert!(!cfg.project.materials.danmaku[0].is_asr());
        assert_eq!(cfg.project.materials.danmaku[0].path, "dm.xml");

        // 同一个文件再选一次：不重复登记
        set_asset(&ws, AssetKind::Danmaku, "dm.xml", false).unwrap();
        assert_eq!(ws.load_config().unwrap().project.materials.danmaku.len(), 1);

        // 清除：只清生效值，**清单保留**（随时能切回来）
        let (value, count) = set_asset(&ws, AssetKind::Danmaku, "", false).unwrap();
        assert_eq!(value, "");
        assert_eq!(count, 0);
        let cfg = ws.load_config().unwrap();
        assert_eq!(cfg.project.danmaku_file.as_deref(), Some(""));
        assert_eq!(cfg.project.materials.danmaku.len(), 1);
    }

    #[test]
    fn set_asset_rejects_missing_and_unparsable() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::write(root.join("bad.srt"), "这不是字幕\n").unwrap();
        let ws = ws_at(root);
        assert!(set_asset(&ws, AssetKind::Transcript, "missing.srt", false).is_err());
        assert!(set_asset(&ws, AssetKind::Transcript, "bad.srt", false).is_err());
        // 拼不出字幕 → 不落盘，也不进清单
        let cfg = ws.load_config().unwrap();
        assert!(cfg
            .project
            .transcript_file
            .as_deref()
            .map(|s| s.is_empty())
            .unwrap_or(true));
        assert!(cfg.project.materials.transcript.is_empty());
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
        let (value, count) =
            set_asset(&ws, AssetKind::Danmaku, &outside.to_string_lossy(), true).unwrap();
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
        let mut cfg = Config::default();

        // 无配置 → 报 output/ 兜底，且不是"外部字幕"
        let st = asset_status(&ws, &cfg, AssetKind::Transcript);
        assert!(st.exists && !st.external && !st.from_config);
        assert_eq!(st.value, "");
        assert!(st.items.is_empty());

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

    /// ASR 产物按**模式槽**登记：同模式重跑覆盖自己那一条，两种模式最多两条。
    #[test]
    fn bind_asr_artifact_keeps_one_entry_per_mode() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::create_dir_all(root.join("output")).unwrap();
        fs::write(root.join("output/transcript.hybrid.json"), "{}").unwrap();
        let ws = ws_at(root);

        bind_material(
            &ws,
            AssetKind::Transcript,
            "output/transcript.hybrid.json",
            Some(MATERIAL_SOURCE_ASR),
            Some("hybrid"),
        )
        .unwrap();
        // 同模式重跑（落点变了）→ 覆盖，不新增
        bind_material(
            &ws,
            AssetKind::Transcript,
            "output/video.srt",
            Some("asr"),
            Some("hybrid"),
        )
        .unwrap();
        bind_material(
            &ws,
            AssetKind::Transcript,
            "output/transcript.single.json",
            Some("asr"),
            Some("single"),
        )
        .unwrap();

        let cfg = ws.load_config().unwrap();
        let items = &cfg.project.materials.transcript;
        assert_eq!(items.len(), 2, "{items:?}");
        assert!(items.iter().all(|e| e.is_asr()));
        assert_eq!(items.iter().filter(|e| e.slot() == "hybrid").count(), 1);
        // 生效值 = 最后一次绑定的那份
        assert_eq!(
            cfg.project.transcript_file.as_deref(),
            Some("output/transcript.single.json")
        );
        // 两种模式都占了 → 转写门控拒（只能两次）
        assert!(slot_bound(&ws, &cfg, AssetKind::Transcript, "hybrid"));
        assert!(slot_bound(&ws, &cfg, AssetKind::Transcript, "single"));
        assert!(!slot_bound(&ws, &cfg, AssetKind::Transcript, "turbo"));
        // 弹幕没有 ASR 槽，哪怕手滑传了 asr 也判"没占"（它没有产物这一说）
        assert!(!slot_bound(&ws, &cfg, AssetKind::Danmaku, "hybrid"));
    }

    /// 管线侧的绑定入口：产物在盘上就绑（并写生效值），不在就**拒绝绑**。
    #[test]
    fn bind_asr_artifact_requires_artifact_on_disk() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let output = root.join("output");
        fs::create_dir_all(&output).unwrap();
        let ws = ws_at(root);
        fs::write(root.join("yekitrim.toml"), "input_file = \"a.mp4\"\n").unwrap();

        // 产物还没落盘 → 不绑（`transcript_file` 指到不存在的文件会让检测直接失败）
        assert_eq!(bind_asr_artifact(&ws, &output, "hybrid"), None);
        assert_eq!(bind_asr_artifact(&ws, &output, "  "), None);
        let cfg = ws.load_config().unwrap();
        assert!(cfg
            .project
            .transcript_file
            .as_deref()
            .map(|s| s.is_empty())
            .unwrap_or(true));
        assert!(cfg.project.materials.transcript.is_empty());

        // 产物落盘 → 绑上：生效值 = 分模式产物，且占住该模式槽
        write_transcript_json(&output.join("transcript.hybrid.json"));
        let stored = bind_asr_artifact(&ws, &output, "Hybrid").unwrap();
        assert_eq!(stored, "output/transcript.hybrid.json");
        let cfg = ws.load_config().unwrap();
        assert_eq!(
            cfg.project.transcript_file.as_deref(),
            Some("output/transcript.hybrid.json")
        );
        assert_eq!(cfg.project.materials.transcript.len(), 1);
        assert!(slot_bound(&ws, &cfg, AssetKind::Transcript, "hybrid"));
        assert!(!slot_bound(&ws, &cfg, AssetKind::Transcript, "single"));

        // 模式串拼不出文件名（core 那边也不会落盘）→ 仍不绑
        assert_eq!(bind_asr_artifact(&ws, &output, "hy b"), None);
        assert_eq!(bind_asr_artifact(&ws, &output, &"x".repeat(33)), None);
    }

    /// 盘上有、清单里没登记的 ASR 产物也要能列出来（老工程 / 绑定写入失败），选中即补登记。
    #[test]
    fn list_items_surfaces_asr_artifact_on_disk() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::create_dir_all(root.join("output")).unwrap();
        write_transcript_json(&root.join("output/transcript.hybrid.json"));
        let ws = ws_at(root);
        let cfg = ws.load_config().unwrap();

        let st = asset_status(&ws, &cfg, AssetKind::Transcript);
        assert_eq!(st.items.len(), 1, "{:?}", st.items);
        let item = &st.items[0];
        assert!(item.is_asr && !item.removable && item.exists);
        assert_eq!(item.mode, "hybrid");
        assert_eq!(item.path, "output/transcript.hybrid.json");
        assert_eq!(item.name, "transcript.hybrid.json");
        // 没配置 → 生效值是 output/ 兜底，不是这一条
        assert!(!item.active);

        // 选中它 → 登记进槽 + 生效 + 标记 active
        set_asset(
            &ws,
            AssetKind::Transcript,
            "output/transcript.hybrid.json",
            false,
        )
        .unwrap();
        let cfg = ws.load_config().unwrap();
        assert_eq!(
            cfg.project.transcript_file.as_deref(),
            Some("output/transcript.hybrid.json")
        );
        assert!(cfg.project.materials.transcript[0].is_asr());
        let st = asset_status(&ws, &cfg, AssetKind::Transcript);
        assert_eq!(st.items.len(), 1, "补登记后不能变成两条");
        assert!(st.items[0].active && st.items[0].is_asr);

        // 用户素材排在 ASR 产物后面
        fs::write(root.join("sub.srt"), "1\n00:00:01,000 --> 00:00:02,000\nhi\n").unwrap();
        set_asset(&ws, AssetKind::Transcript, "sub.srt", false).unwrap();
        let cfg = ws.load_config().unwrap();
        let items = asset_status(&ws, &cfg, AssetKind::Transcript).items;
        assert_eq!(items.len(), 2);
        assert!(items[0].is_asr && !items[1].is_asr);
        assert!(items[1].active && !items[0].active);
    }

    /// 配了值却没登记进清单的（本功能上线前的老工程）：列表兜底列出来，且能"删除"（= 停止使用）。
    #[test]
    fn list_items_falls_back_to_configured_value() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        // 项目字段在 `[project]` 段下（`save_*_path_in` / `patch_config_file` 都写这里）
        fs::write(
            root.join("yekitrim.toml"),
            "[project]\ninput_file = \"a.mp4\"\ndanmaku_file = \"dm.xml\"\n",
        )
        .unwrap();
        fs::write(root.join("dm.xml"), "<?xml version=\"1.0\"?><i></i>").unwrap();
        let ws = ws_at(root);
        let cfg = ws.load_config().unwrap();
        assert!(cfg.project.materials.danmaku.is_empty());

        let st = asset_status(&ws, &cfg, AssetKind::Danmaku);
        assert_eq!(st.value, "dm.xml");
        assert_eq!(st.items.len(), 1, "{:?}", st.items);
        let item = &st.items[0];
        assert!(item.active && item.exists && !item.is_asr && item.removable);
        assert_eq!(item.path, "dm.xml");

        // 删除 = 清掉生效值（本来就不在清单里，不该被判"清单里没有这条"）
        remove_material(&ws, AssetKind::Danmaku, "dm.xml").unwrap();
        let cfg = ws.load_config().unwrap();
        assert_eq!(cfg.project.danmaku_file.as_deref(), Some(""));
        assert!(asset_status(&ws, &cfg, AssetKind::Danmaku).items.is_empty());
        // 文件本身还在（删的是"这条选择"，不是磁盘文件）
        assert!(root.join("dm.xml").is_file());
    }

    /// 删条目：用户素材随时可删、删生效值要连带清掉 `*_file`；ASR 产物只有文件没了才放行。
    #[test]
    fn remove_material_clears_active_value_and_protects_asr() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::create_dir_all(root.join("output")).unwrap();
        fs::write(root.join("sub.srt"), "1\n00:00:01,000 --> 00:00:02,000\nhi\n").unwrap();
        write_transcript_json(&root.join("output/transcript.hybrid.json"));
        let ws = ws_at(root);

        set_asset(&ws, AssetKind::Transcript, "sub.srt", false).unwrap();
        bind_material(
            &ws,
            AssetKind::Transcript,
            "output/transcript.hybrid.json",
            Some("asr"),
            Some("hybrid"),
        )
        .unwrap();
        // 把生效值切回用户字幕，再删它 → 清单少一条、生效值清空
        set_asset(&ws, AssetKind::Transcript, "sub.srt", false).unwrap();
        remove_material(&ws, AssetKind::Transcript, "sub.srt").unwrap();
        let cfg = ws.load_config().unwrap();
        assert_eq!(cfg.project.transcript_file.as_deref(), Some(""));
        assert_eq!(cfg.project.materials.transcript.len(), 1);
        assert!(cfg.project.materials.transcript[0].is_asr());

        // ASR 产物（文件还在）不许删
        let err = remove_material(&ws, AssetKind::Transcript, "output/transcript.hybrid.json")
            .unwrap_err()
            .to_string();
        assert!(err.contains("不能删除"), "{err}");
        // 文件没了 → 放行（那才是"槽位空出来"）
        fs::remove_file(root.join("output/transcript.hybrid.json")).unwrap();
        remove_material(&ws, AssetKind::Transcript, "output/transcript.hybrid.json").unwrap();
        let cfg = ws.load_config().unwrap();
        assert!(cfg.project.materials.transcript.is_empty());
        assert!(!slot_bound(&ws, &cfg, AssetKind::Transcript, "hybrid"));

        // 删一个清单里没有的 → 报错（不要静默成功）
        assert!(remove_material(&ws, AssetKind::Transcript, "nope.srt").is_err());
    }

    /// 写清单走 `patch_config_file`：注释、其它 section、没提到的键都必须在。
    #[test]
    fn manifest_write_keeps_comments_and_other_sections() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::write(
            root.join("yekitrim.toml"),
            "# 顶注\n[project]\ninput_file = \"a.mp4\"   # 行内注释\ndanmaku_file = \"dm.xml\"\n\n[detection]\nvtuber_name = \"主播\"\n",
        )
        .unwrap();
        let ws = ws_at(root);
        bind_material(&ws, AssetKind::Transcript, "sub.srt", None, None).unwrap();

        let raw = fs::read_to_string(root.join("yekitrim.toml")).unwrap();
        assert!(raw.contains("# 顶注"), "{raw}");
        assert!(raw.contains("# 行内注释"), "{raw}");
        assert!(raw.contains("danmaku_file = \"dm.xml\""), "{raw}");
        assert!(raw.contains("vtuber_name = \"主播\""), "{raw}");
        assert!(raw.contains("transcript_file = \"sub.srt\""), "{raw}");
        assert!(raw.contains("[project.materials]"), "{raw}");
        // 读回来也得对得上
        let cfg = ws.load_config().unwrap();
        assert_eq!(cfg.project.transcript_file.as_deref(), Some("sub.srt"));
        assert_eq!(cfg.project.materials.transcript.len(), 1);
        assert_eq!(cfg.project.materials.danmaku.len(), 0);
    }

    #[test]
    fn preload_policy_matches_sources() {
        assert_eq!(preload_policy("recorder"), Preload::Danmaku);
        // ⚠️ 防回退：`bilibili` 必须是弹幕。改成 Subtitle 不会报错，
        // 只会让 B站项目的热度排序静默变差（见 `Preload::Subtitle` 的说明）。
        assert_eq!(preload_policy("bilibili"), Preload::Danmaku);
        assert_eq!(preload_policy("local"), Preload::None);
        assert_eq!(preload_policy("whatever"), Preload::None);
    }
}
