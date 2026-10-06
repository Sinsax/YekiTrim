//! `output/clips/index.json`：片段目录的索引与目录名分配。
//!
//! 背景：以前目录名 = `clip-{rank:02}`，而 `rank` 每次检测按热度重排，旧片段
//! 会挡住新事件（「序号被占用」）。这里把「片段目录名」从 rank 里解耦：
//!
//! - 磁盘仍是事实来源：每个 `clips/<dir>/clip.json` 是片段自己的元数据；
//! - `index.json` 是目录 → 元数据的映射缓存，可从磁盘重建；
//! - 新片段用标题净化后的可读目录名（重复/超长时加短哈希后缀）；
//! - 旧 `clip-NN` 原样保留，匹配上的事件继续复用旧目录。
//!
//! 对外只暴露纯函数 + 文件读写；Cutter / 检测 / server 都走这里，避免再抄一份命名规则。

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::types::{Event, Timestamp};

/// `clips/` 下的索引文件名。`data_clips_impl` 只枚举目录，不会把它当片段。
pub const CLIP_INDEX_FILE: &str = "index.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClipIndex {
    #[serde(default = "default_version")]
    pub version: u32,
    #[serde(default)]
    pub clips: BTreeMap<String, ClipEntry>,
    /// 保留未知顶层字段，避免以后加字段时旧版本程序把新数据写没。
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

impl Default for ClipIndex {
    fn default() -> Self {
        Self {
            version: 1,
            clips: BTreeMap::new(),
            extra: serde_json::Map::new(),
        }
    }
}

fn default_version() -> u32 {
    1
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ClipEntry {
    #[serde(default)]
    pub dir: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub event_type: String,
    #[serde(default)]
    pub rank: u32,
    #[serde(default)]
    pub popularity: f64,
    #[serde(default)]
    pub original_start: f64,
    #[serde(default)]
    pub original_end: f64,
    /// **多段片段的各段源区间**（`clip.json` 的 `ranges`；空 = 单段）。
    ///
    /// ⚠️ 空数组与"没有这个键"同义 —— 老索引/老片段一个字节都不用改。
    /// 前端据此给卡片打「N 段」标记（`ProjectClipsSection` / 精修片段条 / 事件表）。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ranges: Vec<crate::types::EventRange>,
    #[serde(default)]
    pub created_at: String,
    #[serde(default)]
    pub updated_at: String,
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

impl ClipEntry {
    pub fn new(dir: impl Into<String>, title: impl Into<String>) -> Self {
        Self {
            dir: dir.into(),
            title: title.into(),
            ..Default::default()
        }
    }

    /// 用事件信息补全索引条目（不覆盖已有 created_at）。
    pub fn from_event(event: &Event, dir: impl Into<String>) -> Self {
        let now = now_rfc3339();
        Self {
            dir: dir.into(),
            title: event.label.clone(),
            event_type: serde_json::to_value(&event.event_type)
                .ok()
                .and_then(|v| v.as_str().map(ToOwned::to_owned))
                .unwrap_or_default(),
            rank: event.rank,
            popularity: event.popularity,
            original_start: event.start.to_seconds(),
            original_end: event.end.to_seconds(),
            // 多段事件（分散事件合并的产物）：把各段记进索引，界面据此打标记
            ranges: if event.ranges.len() > 1 {
                event.ranges.clone()
            } else {
                Vec::new()
            },
            created_at: now.clone(),
            updated_at: now,
            extra: serde_json::Map::new(),
        }
    }
}

fn now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339()
}

pub fn index_path(output_dir: &Path) -> PathBuf {
    output_dir.join("clips").join(CLIP_INDEX_FILE)
}

/// 读取索引。文件缺失/损坏时回退空索引 —— 磁盘 `clip.json` 才是事实来源，
/// 这里绝不让坏索引把剪切流程卡死。
pub fn load_index(output_dir: &Path) -> ClipIndex {
    let path = index_path(output_dir);
    let Ok(raw) = std::fs::read_to_string(&path) else {
        return ClipIndex::default();
    };
    serde_json::from_str(&raw).unwrap_or_default()
}

/// 保存索引：先写临时文件再替换，避免写到一半留下半个 JSON。
pub fn save_index(output_dir: &Path, index: &ClipIndex) -> Result<()> {
    let path = index_path(output_dir);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("创建片段索引目录失败: {}", parent.display()))?;
    }
    let tmp = path.with_extension("json.tmp");
    let json = serde_json::to_string_pretty(index)?;
    std::fs::write(&tmp, json).with_context(|| format!("写入片段索引失败: {}", tmp.display()))?;
    // Windows 上 rename 覆盖已存在文件会失败；先删旧文件，失败也只是短暂窗口。
    let _ = std::fs::remove_file(&path);
    std::fs::rename(&tmp, &path)
        .with_context(|| format!("替换片段索引失败: {}", path.display()))?;
    Ok(())
}

/// 从磁盘扫描所有片段目录，和现有索引合并后返回一份“磁盘为准”的索引。
///
/// - 磁盘上存在的目录一定进结果（缺 `clip.json` 时 title 为空，仍占目录名）；
/// - 索引里记录但磁盘已不存在的条目被丢弃；
/// - 保留索引里已有的 `created_at` 等字段。
pub fn reconcile_index(output_dir: &Path) -> ClipIndex {
    let mut old = load_index(output_dir);
    let mut next = ClipIndex {
        version: old.version.max(1),
        clips: BTreeMap::new(),
        extra: std::mem::take(&mut old.extra),
    };
    let clips_dir = output_dir.join("clips");
    let Ok(entries) = std::fs::read_dir(&clips_dir) else {
        return next;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let Some(dir_name) = path.file_name().map(|n| n.to_string_lossy().into_owned()) else {
            continue;
        };
        if !is_safe_component(&dir_name) {
            continue;
        }
        let meta = read_clip_meta(&path);
        let mut item = old.clips.remove(&dir_name).unwrap_or_default();
        item.dir = dir_name.clone();
        if let Some(meta) = meta.as_ref() {
            merge_meta_into_entry(&mut item, meta);
        }
        next.clips.insert(dir_name, item);
    }
    next
}

/// 扫描磁盘并合并后直接写回索引（剪切/删除后调用）。
pub fn refresh_index(output_dir: &Path) -> Result<ClipIndex> {
    let index = reconcile_index(output_dir);
    save_index(output_dir, &index)?;
    Ok(index)
}

/// 新增/更新一个片段目录的索引条目。会先 reconcile，确保磁盘上的其它目录也在。
pub fn upsert_clip(output_dir: &Path, mut entry: ClipEntry) -> Result<()> {
    if !is_safe_component(&entry.dir) {
        anyhow::bail!("非法片段目录名: {}", entry.dir);
    }
    let mut index = reconcile_index(output_dir);
    if let Some(old) = index.clips.get(&entry.dir) {
        if entry.created_at.is_empty() {
            entry.created_at = old.created_at.clone();
        }
    }
    if entry.created_at.is_empty() {
        entry.created_at = now_rfc3339();
    }
    entry.updated_at = now_rfc3339();
    index.clips.insert(entry.dir.clone(), entry);
    save_index(output_dir, &index)
}

/// 删除一个片段目录的索引条目（目录本身由调用方负责删）。
pub fn remove_clip(output_dir: &Path, dir: &str) -> Result<()> {
    let mut index = reconcile_index(output_dir);
    if index.clips.remove(dir).is_some() {
        save_index(output_dir, &index)?;
    }
    Ok(())
}

/// 与 `foreign_clip_conflict` / 前端 `clipMatchesEvent` 共用的匹配规则：
/// **标题相同** 或 **原片区间 IoU ≥ 0.5**。
///
/// 缺标题/缺区间时返回 false —— 对“已有片段”来说，信息不足不能算匹配；
/// 对“要覆盖的目标目录”来说，`foreign_clip_conflict` 会走自己的放行逻辑。
pub fn clip_meta_matches(label: &str, start: f64, end: f64, meta: &serde_json::Value) -> bool {
    let old_title = meta
        .get("title")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim();
    if !old_title.is_empty() && old_title == label.trim() {
        return true;
    }
    let (Some(os), Some(oe)) = (
        meta.get("original_start").and_then(|v| v.as_f64()),
        meta.get("original_end").and_then(|v| v.as_f64()),
    ) else {
        return false;
    };
    let inter = (oe.min(end) - os.max(start)).max(0.0);
    let union = (oe.max(end) - os.min(start)).max(1e-6);
    inter / union >= 0.5
}

/// 生成一个安全、可读、唯一的片段目录名。
///
/// - 净化 Windows/macOS 非法字符与控制字符；
/// - 规避 Windows 保留名；
/// - 超长时截断 + 6 位短哈希；
/// - 与 `taken`（大小写不敏感）冲突时追加 `-2` / `-3`，仍冲突则用短哈希。
pub fn make_clip_dir_name(title: &str, taken: &BTreeSet<String>, fallback_rank: u32) -> String {
    let base = sanitize_title(title);
    let base = if base.is_empty() {
        format!("clip-{fallback_rank:02}")
    } else {
        base
    };
    let base = limit_len(&base, 50);
    let mut candidate = base.clone();
    let mut n = 2u32;
    while taken.contains(&candidate.to_ascii_lowercase()) {
        candidate = format!("{base}-{n}");
        n += 1;
        if n > 50 {
            candidate = format!("{base}-{}", short_hash(title));
            break;
        }
    }
    // 兜底：极端情况下短哈希也撞了，直接加纳秒后缀。
    if taken.contains(&candidate.to_ascii_lowercase()) {
        candidate = format!(
            "{base}-{}",
            short_hash(&format!("{title}{}", now_rfc3339()))
        );
    }
    candidate
}

fn sanitize_title(title: &str) -> String {
    let mut out = String::new();
    for ch in title.trim().chars() {
        match ch {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => out.push('_'),
            c if c.is_control() => {}
            c => out.push(c),
        }
    }
    // 折叠空白，去掉首尾空白和结尾的 `.`（Windows 目录名不能以点/空格结尾）。
    let mut collapsed = String::new();
    let mut last_space = false;
    for ch in out.chars() {
        if ch.is_whitespace() {
            if !last_space {
                collapsed.push(' ');
                last_space = true;
            }
        } else {
            collapsed.push(ch);
            last_space = false;
        }
    }
    let trimmed = collapsed.trim().trim_end_matches(['.', ' ']);
    let mut name = trimmed.to_string();
    if is_reserved_name(&name) {
        name = format!("_{name}");
    }
    name
}

fn limit_len(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max_chars.saturating_sub(7)).collect();
    out.push('-');
    out.push_str(&short_hash(s));
    out
}

fn is_reserved_name(s: &str) -> bool {
    let stem = s
        .split('.')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_uppercase();
    matches!(
        stem.as_str(),
        "CON"
            | "PRN"
            | "AUX"
            | "NUL"
            | "COM1"
            | "COM2"
            | "COM3"
            | "COM4"
            | "COM5"
            | "COM6"
            | "COM7"
            | "COM8"
            | "COM9"
            | "LPT1"
            | "LPT2"
            | "LPT3"
            | "LPT4"
            | "LPT5"
            | "LPT6"
            | "LPT7"
            | "LPT8"
            | "LPT9"
    )
}

/// 目录名必须是安全的单层组件（不能有路径分隔符、`..`、盘符、控制字符）。
pub fn is_safe_component(name: &str) -> bool {
    if name.is_empty() || name == "." || name == ".." {
        return false;
    }
    if name.contains(['/', '\\', ':', '*', '?', '"', '<', '>', '|']) {
        return false;
    }
    if name.chars().any(|c| c.is_control()) {
        return false;
    }
    // Windows 下不允许以点/空格结尾。
    if name.ends_with('.') || name.ends_with(' ') {
        return false;
    }
    true
}

/// 稳定短哈希（FNV-1a 低 24 位），只用于目录名后缀。
fn short_hash(s: &str) -> String {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in s.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    format!("{:06x}", h & 0x00ff_ffff)
}

fn read_clip_meta(dir: &Path) -> Option<serde_json::Value> {
    let raw = std::fs::read_to_string(dir.join("clip.json")).ok()?;
    serde_json::from_str(&raw).ok()
}

fn merge_meta_into_entry(entry: &mut ClipEntry, meta: &serde_json::Value) {
    if entry.title.is_empty() {
        entry.title = meta
            .get("title")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
    }
    if entry.event_type.is_empty() {
        entry.event_type = meta
            .get("event_type")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
    }
    if entry.rank == 0 {
        entry.rank = meta.get("rank").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
    }
    if entry.popularity == 0.0 {
        entry.popularity = meta
            .get("popularity")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);
    }
    if entry.original_start == 0.0 {
        entry.original_start = meta
            .get("original_start")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);
    }
    if entry.original_end == 0.0 {
        entry.original_end = meta
            .get("original_end")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);
    }
    // 多段标记：索引里没有就从 `clip.json` 补（判据与写侧一致：>1 段才算多段）
    if entry.ranges.is_empty() {
        if let Some(arr) = meta.get("ranges").and_then(|v| v.as_array()) {
            let parsed: Vec<crate::types::EventRange> = arr
                .iter()
                .filter_map(|r| {
                    let s = r.get("start").and_then(|v| v.as_f64())?;
                    let e = r.get("end").and_then(|v| v.as_f64())?;
                    (e > s).then(|| crate::types::EventRange::new(s, e))
                })
                .collect();
            if parsed.len() > 1 {
                entry.ranges = parsed;
            }
        }
    }
}

/// 为一个事件找可复用的旧片段目录：标题相同或区间 IoU ≥ 0.5。
fn find_matching_dir(
    index: &ClipIndex,
    event: &Event,
    claimed: &BTreeSet<String>,
) -> Option<String> {
    let start = event.start.to_seconds();
    let end = event.end.to_seconds();
    index
        .clips
        .values()
        .filter(|c| is_safe_component(&c.dir))
        .filter(|c| !claimed.contains(&c.dir.to_ascii_lowercase()))
        .find(|c| {
            let meta = serde_json::json!({
                "title": c.title,
                "original_start": c.original_start,
                "original_end": c.original_end,
            });
            clip_meta_matches(&event.label, start, end, &meta)
        })
        .map(|c| c.dir.clone())
}

/// 检测阶段调用：给每个事件分配/复用稳定的 `clip_dir`。
///
/// 规则：
/// 1. 已有 `clip_dir` 且安全 → 保留；
/// 2. 能在磁盘/index 里找到匹配旧片段 → 复用旧目录（保留精修产物）；
/// 3. 否则用标题生成新的唯一目录名。
///
/// `events` 保持原顺序；匹配认领按 `popularity` 从高到低，避免多个事件抢同一个旧片段。
pub fn assign_clip_dirs(output_dir: &Path, events: &mut [Event]) -> Result<()> {
    let index = reconcile_index(output_dir);
    let mut taken: BTreeSet<String> = index.clips.keys().map(|d| d.to_ascii_lowercase()).collect();
    let mut claimed: BTreeSet<String> = BTreeSet::new();

    let mut order: Vec<usize> = (0..events.len()).collect();
    order.sort_by(|&a, &b| {
        events[b]
            .popularity
            .partial_cmp(&events[a].popularity)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    for i in order {
        let event = &mut events[i];
        // 已经有安全 clip_dir 的（例如手动事件）直接保留；同一目录只能归一个事件。
        if let Some(dir) = event
            .clip_dir
            .as_deref()
            .map(str::trim)
            .filter(|d| !d.is_empty())
        {
            if is_safe_component(dir) {
                let key = dir.to_ascii_lowercase();
                if !claimed.contains(&key) {
                    claimed.insert(key.clone());
                    taken.insert(key);
                    continue;
                }
            }
        }
        if let Some(dir) = find_matching_dir(&index, event, &claimed) {
            claimed.insert(dir.to_ascii_lowercase());
            event.clip_dir = Some(dir);
            continue;
        }
        let dir = make_clip_dir_name(&event.label, &taken, event.rank);
        taken.insert(dir.to_ascii_lowercase());
        event.clip_dir = Some(dir);
    }
    Ok(())
}

/// 手动新增单个事件时分配一个不冲突的目录名（不修改 events.json）。
pub fn allocate_one_clip_dir(output_dir: &Path, event: &Event) -> String {
    let index = reconcile_index(output_dir);
    let mut taken: BTreeSet<String> = index.clips.keys().map(|d| d.to_ascii_lowercase()).collect();
    if let Some(dir) = event
        .clip_dir
        .as_deref()
        .map(str::trim)
        .filter(|d| !d.is_empty())
    {
        if is_safe_component(dir) {
            return dir.to_string();
        }
    }
    let dir = make_clip_dir_name(&event.label, &taken, event.rank);
    taken.insert(dir.to_ascii_lowercase());
    dir
}

/// `Timestamp` 转秒，方便调用方少写一层。
pub fn ts_secs(ts: &Timestamp) -> f64 {
    ts.to_seconds()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{EventSource, EventType};

    struct TmpDir(PathBuf);
    impl TmpDir {
        fn new(tag: &str) -> Self {
            let nanos = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0);
            let p = std::env::temp_dir().join(format!(
                "yekitrim-clip-index-{tag}-{}-{nanos}",
                std::process::id()
            ));
            std::fs::create_dir_all(&p).unwrap();
            Self(p)
        }
    }
    impl Drop for TmpDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn ev(label: &str, start: f64, end: f64, rank: u32) -> Event {
        Event {
            start: Timestamp::from_seconds(start),
            end: Timestamp::from_seconds(end),
            label: label.to_string(),
            summary: String::new(),
            event_type: EventType::Other,
            source: EventSource::LLM,
            confidence: 1.0,
            popularity: 0.5,
            rank,
            clip_dir: None,
            enabled: true,
            ranges: Vec::new(),
            topic: None,
        }
    }

    #[test]
    fn make_name_sanitizes_and_dedupes() {
        let mut taken = BTreeSet::new();
        let a = make_clip_dir_name("A/B:C*?<>|", &taken, 1);
        assert!(!a.contains(['/', ':', '*', '?', '<', '>', '|']), "{a}");
        taken.insert(a.to_ascii_lowercase());
        let b = make_clip_dir_name("A_B_C___", &taken, 1);
        assert_ne!(a, b, "同名/近似名必须去重");
    }

    #[test]
    fn reserved_and_empty_fallback() {
        assert!(make_clip_dir_name("CON", &BTreeSet::new(), 1).starts_with('_'));
        assert_eq!(make_clip_dir_name("", &BTreeSet::new(), 7), "clip-07");
    }

    #[test]
    fn assign_reuses_matching_old_clip_and_allocates_free_for_foreign() {
        let t = TmpDir::new("assign");
        let clips = t.0.join("clips");
        std::fs::create_dir_all(clips.join("clip-01")).unwrap();
        std::fs::write(
            clips.join("clip-01").join("clip.json"),
            r#"{"title":"旧事件","original_start":100.0,"original_end":200.0,"rank":1}"#,
        )
        .unwrap();
        let mut events = vec![
            ev("旧事件", 100.0, 200.0, 1),
            ev("全新事件", 300.0, 400.0, 1),
        ];
        assign_clip_dirs(&t.0, &mut events).unwrap();
        assert_eq!(
            events[0].clip_dir.as_deref(),
            Some("clip-01"),
            "匹配旧片段应复用目录"
        );
        assert_ne!(
            events[1].clip_dir.as_deref(),
            Some("clip-01"),
            "新事件不能撞旧目录"
        );
        assert!(events[1]
            .clip_dir
            .as_deref()
            .unwrap()
            .starts_with("全新事件"));
    }

    #[test]
    fn legacy_dir_without_meta_reserves_slot() {
        let t = TmpDir::new("legacy");
        let clips = t.0.join("clips");
        std::fs::create_dir_all(clips.join("clip-01")).unwrap();
        let mut events = vec![ev("新事件", 0.0, 10.0, 1)];
        assign_clip_dirs(&t.0, &mut events).unwrap();
        assert_ne!(
            events[0].clip_dir.as_deref(),
            Some("clip-01"),
            "无元数据旧目录不能被覆盖"
        );
    }

    #[test]
    fn index_reconciles_from_disk() {
        let t = TmpDir::new("reconcile");
        let clips = t.0.join("clips");
        std::fs::create_dir_all(clips.join("标题片段")).unwrap();
        std::fs::write(
            clips.join("标题片段").join("clip.json"),
            r#"{"title":"标题片段","original_start":1.0,"original_end":2.0,"rank":3}"#,
        )
        .unwrap();
        let idx = reconcile_index(&t.0);
        assert!(idx.clips.contains_key("标题片段"));
        assert_eq!(idx.clips["标题片段"].rank, 3);
    }
}

#[cfg(test)]
mod multi_range_index_tests {
    use super::*;
    use crate::types::{Event, EventRange, EventSource, EventType, Timestamp};

    fn ev_with_ranges(ranges: Vec<EventRange>) -> Event {
        let mut e = Event {
            start: Timestamp::from_seconds(0.0),
            end: Timestamp::from_seconds(0.0),
            label: "多段".into(),
            summary: String::new(),
            event_type: EventType::Other,
            source: EventSource::Merged,
            confidence: 1.0,
            popularity: 0.5,
            rank: 3,
            clip_dir: None,
            enabled: true,
            ranges,
            topic: Some("唱歌".into()),
        };
        e.normalize_ranges();
        e
    }

    /// 多段事件 → 索引条目带 `ranges`（界面据此打「N 段」标记）；单段**不带**（老格式不变）。
    #[test]
    fn entry_from_event_keeps_multi_ranges_only() {
        let multi = ev_with_ranges(vec![
            EventRange::new(100.0, 120.0),
            EventRange::new(200.0, 230.0),
        ]);
        let entry = ClipEntry::from_event(&multi, "clip-multi");
        assert_eq!(entry.ranges.len(), 2);
        assert_eq!((entry.original_start, entry.original_end), (100.0, 230.0));

        let single = ev_with_ranges(vec![EventRange::new(10.0, 40.0)]);
        let entry = ClipEntry::from_event(&single, "clip-single");
        assert!(entry.ranges.is_empty(), "1 段不算多段（写侧口径 > 1）");

        // 序列化：单段不许出现 `ranges` 键（老索引/老前端不受影响）
        let single_json = serde_json::to_value(ClipEntry::from_event(&single, "x")).unwrap();
        assert!(single_json.get("ranges").is_none(), "{single_json}");
        let multi_json = serde_json::to_value(ClipEntry::from_event(&multi, "x")).unwrap();
        assert_eq!(multi_json["ranges"].as_array().unwrap().len(), 2);
    }

    /// 索引里没有时从 `clip.json`（`meta`）补 `ranges` —— 老索引 + 新片段那条路。
    #[test]
    fn merge_meta_reads_ranges_from_clip_json() {
        let meta = serde_json::json!({
            "title": "多段片段",
            "original_start": 100.0,
            "original_end": 230.0,
            "duration": 50.0,
            "ranges": [{ "start": 100.0, "end": 120.0 }, { "start": 200.0, "end": 230.0 }],
        });
        let mut entry = ClipEntry::default();
        merge_meta_into_entry(&mut entry, &meta);
        assert_eq!(entry.ranges.len(), 2);
        assert_eq!(entry.ranges[1].start, 200.0);

        // 单段（1 条 ranges）与退化段（end <= start）都不算多段
        let mut e2 = ClipEntry::default();
        merge_meta_into_entry(
            &mut e2,
            &serde_json::json!({ "ranges": [{ "start": 10.0, "end": 40.0 }] }),
        );
        assert!(e2.ranges.is_empty());
        let mut e3 = ClipEntry::default();
        merge_meta_into_entry(
            &mut e3,
            &serde_json::json!({ "ranges": [{ "start": 5.0, "end": 5.0 }, { "start": 9.0, "end": 9.0 }] }),
        );
        assert!(e3.ranges.is_empty(), "退化段要滤掉");
    }
}
