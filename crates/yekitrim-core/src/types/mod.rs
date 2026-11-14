//! types：跨 stage 共享的数据类型（时间戳 / 转写 / 事件 / 管线上下文）。
//!
//! 入口：`PipelineContext`、`Timestamp`、`VideoInfo`、`Transcript`、`TranscriptSegment`、`Event`、`EventType`、`EventSource`、`EventStore`。
//! 边界：全管线的公共数据面，改字段会连带四个 stage 与落盘的 `transcript.json` / `events.json`；`types/` 无专卡，规矩在其上层卡 —— 见 `crates/yekitrim-core/AGENTS.md`

use std::path::PathBuf;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::config::Config;
use crate::progress::{NoopProgress, ProgressReporter};

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Timestamp {
    pub h: u32,
    pub m: u32,
    pub s: u32,
    pub ms: u32,
}

impl Timestamp {
    pub fn to_seconds(&self) -> f64 {
        self.h as f64 * 3600.0
            + self.m as f64 * 60.0
            + self.s as f64
            + self.ms as f64 / 1000.0
    }

    pub fn from_seconds(secs: f64) -> Self {
        let total_ms = (secs * 1000.0).round() as u32;
        let h = total_ms / 3_600_000;
        let rem = total_ms % 3_600_000;
        let m = rem / 60_000;
        let rem = rem % 60_000;
        let s = rem / 1000;
        let ms = rem % 1000;
        Timestamp { h, m, s, ms }
    }

    pub fn to_srt_format(&self) -> String {
        format!("{:02}:{:02}:{:02},{:03}", self.h, self.m, self.s, self.ms)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VideoInfo {
    pub duration_secs: f64,
    pub width: u32,
    pub height: u32,
    pub video_codec: String,
    pub audio_codec: String,
    pub format: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Transcript {
    pub segments: Vec<TranscriptSegment>,
    pub language: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub asr_metadata: Option<AsrMetadata>,
}

/// ASR 转写质量元数据，写入 `transcript.json` 供 `yekitrim status` / 回归分析使用。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AsrMetadata {
    #[serde(default)]
    pub anchors: usize,
    #[serde(default)]
    pub anchor_coverage: f64,
    #[serde(default)]
    pub max_interp_span: usize,
    #[serde(default)]
    pub fallback_reason: Option<String>,
    #[serde(default)]
    pub model: String,
    /// 这次转写用的 `[asr] mode`（`hybrid` / `single`）。
    ///
    /// 预处理页据此给字幕条目标「ASR · hybrid」这类徽标，并在老工程（`materials` 还是空的）
    /// 里推断这份产物占的是哪个模式槽。LLM 订正后的产物 `asr_metadata` 会被置空，
    /// 所以**别把槽位判定只建在这个字段上** —— 清单里的 `source`/`mode` 才是权威。
    #[serde(default)]
    pub mode: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TranscriptSegment {
    pub start: f64,
    pub end: f64,
    pub text: String,
    /// 置信度。前端手写字幕（新增行）可能不带该字段，缺省视为 1.0，
    /// 否则整个 segments 数组会因缺字段反序列化失败 → 保存 400。
    #[serde(default = "default_confidence")]
    pub confidence: f64,
}

fn default_confidence() -> f64 {
    1.0
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum EventType {
    #[serde(rename = "intro")]
    Intro,
    #[serde(rename = "gameplay")]
    Gameplay,
    #[serde(rename = "interaction")]
    Interaction,
    #[serde(rename = "qna")]
    QnA,
    #[serde(rename = "reaction")]
    Reaction,
    #[serde(rename = "storytelling")]
    Storytelling,
    #[serde(rename = "music")]
    Music,
    #[serde(rename = "food")]
    Food,
    #[serde(rename = "rant")]
    Rant,
    #[serde(rename = "break")]
    Break,
    #[serde(rename = "outro")]
    Outro,
    #[serde(rename = "chat")]
    Chat,
    #[serde(rename = "eating")]
    Eating,
    #[serde(rename = "irl")]
    IRL,
    #[serde(rename = "ranking")]
    Ranking,
    #[serde(rename = "discussion")]
    Discussion,
    #[serde(rename = "other")]
    Other,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum EventSource {
    #[serde(alias = "manual")]
    Rules,
    LLM,
    Merged,
}

/// 源区间（秒）。**多段片段**（分散事件合并的产物）用它记"这条片段由哪几段源拼成"。
///
/// 判据/不变式（`Event::ranges` 非空时）：`ranges[0].start == event.start`、
/// `ranges.last().end == event.end`（两条真源会打架，单测 `event_ranges_cover_span` 钉住）。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub struct EventRange {
    pub start: f64,
    pub end: f64,
}

impl EventRange {
    pub fn new(start: f64, end: f64) -> Self {
        Self { start, end }
    }

    pub fn duration(&self) -> f64 {
        (self.end - self.start).max(0.0)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Event {
    pub start: Timestamp,
    pub end: Timestamp,
    pub label: String,
    pub summary: String,
    pub event_type: EventType,
    pub source: EventSource,
    pub confidence: f64,
    #[serde(default)]
    pub popularity: f64,
    #[serde(default)]
    pub rank: u32,
    /// 实际片段目录名（`output/clips/` 下的安全组件名）。
    ///
    /// `None` = 旧 events.json：Cutter / 前端回退到旧的 `clip-{rank:02}` 契约。
    /// 新检测/手动事件会写入一个稳定目录名（可读标题 slug + 去重后缀）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clip_dir: Option<String>,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    /// **多段源区间**（分散事件合并的产物）：非空时这条片段由 `ranges` 里的若干段拼成，
    /// `start..end` 是它们的**并集跨度**（首段起、末段末）。
    ///
    /// ⚠️ 空 = 单段 = `start..end` —— 老 `events.json` / `clip.json` 一个字节都不用改，
    /// 所有读侧（Cutter / 精修 / 前端）都按"空则单段"解释。见 `docs/plan/scattered-events-merge.md`。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ranges: Vec<EventRange>,
    /// 分散事件第二趟的**归组键**（只有第二趟产出的候选块才有；合并后写在这一条上）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub topic: Option<String>,
}

impl Event {
    /// 这条片段实际要剪的源区间：`ranges` 为空时就是 `start..end` 那一段。
    pub fn cut_ranges(&self) -> Vec<EventRange> {
        if self.ranges.is_empty() {
            vec![EventRange::new(
                self.start.to_seconds(),
                self.end.to_seconds(),
            )]
        } else {
            self.ranges.clone()
        }
    }

    /// 实际内容时长（多段 = 各段之和，不含段间空隙）。
    pub fn content_secs(&self) -> f64 {
        self.cut_ranges().iter().map(|r| r.duration()).sum()
    }

    /// 把 `ranges` 与 `start/end` 重新对齐（首段起、末段末；段按时间排序；**重叠/相接的段并成一段**）。
    pub fn normalize_ranges(&mut self) {
        if self.ranges.is_empty() {
            return;
        }
        self.ranges
            .retain(|r| r.end > r.start + 1e-6);
        if self.ranges.is_empty() {
            return;
        }
        self.ranges.sort_by(|a, b| {
            a.start
                .partial_cmp(&b.start)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        merge_overlapping_ranges(&mut self.ranges);
        self.start = Timestamp::from_seconds(self.ranges[0].start);
        self.end = Timestamp::from_seconds(self.ranges[self.ranges.len() - 1].end);
    }
}

/// 把**已按 start 排序**的区间里"重叠或首尾相接"的那些并成一段（内容取并集）。
///
/// 为什么要单独一步：手工在预处理页标段时，两行入点/出点**可以互相压住**——项目 12 那条
/// 4 段片段就有 `1625.7–1653.8` 里套着 `1632.0–1643.0`。不去重的话 Cutter 会把这段
/// **剪两遍**（`clip.mp4` 158.3s = 四段之和，其中 11s 是重复的），而 `duration` / 卡片上的
/// 「内容时长」也按"各段之和"算 ⇒ 两处一起多报 11s，界面上完全看不出来。
/// 「相接」（`next.start == last.end`）并掉是等价的（并集 = 同样两段内容），顺手也并了。
///
/// ⚠️ 只并 `next.start <= last.end`：**有内容的空隙**（`next.start > last.end`）必须留着 ——
/// 那正是"分段剪、把中间别的话题丢掉"的表达（见 `detection::scattered::merge_group`）。
pub fn merge_overlapping_ranges(ranges: &mut Vec<EventRange>) {
    if ranges.len() < 2 {
        return;
    }
    let mut out: Vec<EventRange> = Vec::with_capacity(ranges.len());
    for r in ranges.iter() {
        match out.last_mut() {
            Some(last) if r.start <= last.end + 1e-6 => {
                if r.end > last.end {
                    last.end = r.end;
                }
            }
            _ => out.push(*r),
        }
    }
    *ranges = out;
}

impl Event {
    /// 片段目录名：优先用持久化的 `clip_dir`，旧事件回退 `clip-{rank:02}`。
    pub fn clip_dir_or_rank(&self) -> String {
        self.clip_dir
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(ToOwned::to_owned)
            .unwrap_or_else(|| format!("clip-{:02}", self.rank))
    }
}

fn default_enabled() -> bool { true }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventStore {
    /// 「出现过的标题」这份**附属索引**（`save_events` 用它做重名判断）。
    ///
    /// ⚠️ **必须宽容缺失**：它只是一个去重辅助，不是事件的载荷。以前没有
    /// `#[serde(default)]`，于是「手写 / 外部工具 / demo 工程」那种只有 `events` 的
    /// `events.json` 两种形状都解析失败 ⇒ `cut` / `process` 直接报「解析事件 JSON 失败」，
    /// 而错误里一个字都没说缺了 `titles`。
    #[serde(default)]
    pub titles: std::collections::BTreeMap<String, bool>,
    pub events: Vec<Event>,
}

#[derive(Clone)]
pub struct PipelineContext {
    pub config: Arc<Config>,
    pub input_path: PathBuf,
    pub input_stem: String,
    pub output_dir: PathBuf,
    pub danmaku_path: Option<PathBuf>,
    pub audio_path: Option<PathBuf>,
    pub transcript: Option<Transcript>,
    pub video_info: Option<VideoInfo>,
    /// 事件检测断点续传决策：Some(true)=继续，Some(false)=重来，None=自动（有 checkpoint 则继续）
    pub detect_resume: Option<bool>,
    pub events: Vec<Event>,
    pub started_at: DateTime<Utc>,
    pub progress: Arc<dyn ProgressReporter>,
    /// 阶段内**非致命**的结果记录（跳过 / 部分失败 / 冲突），由 stage 经
    /// `cutting::push_pipeline_note` 统一登记（tracing + `progress.note` 任务消息通道 + 本清单三落地）。
    ///
    /// 用途：剪切「按 `overwrite = false` 跳过 N 条」「M 条失败」这类信息必须让用户看到，
    /// 但它们又**不该**把整条任务判成失败（成片已经出来了）—— 绝不能走 `progress.error`
    ///（server 的 error = 整条任务 fail）。server 侧任务框的落点是 `ProgressReporter::note`。
    pub pipeline_notes: Vec<String>,
}

impl PipelineContext {
    pub fn new(input_path: PathBuf, output_dir: PathBuf) -> Self {
        let invalid_chars = ['\\', '/', ':', '*', '?', '"', '<', '>', '|', '\u{FFFD}'];
        let input_stem = input_path
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .map(|s| {
                s.chars()
                    .map(|c| if invalid_chars.contains(&c) || c.is_control() { '_' } else { c })
                    .collect()
            })
            .unwrap_or_else(|| "unknown".into());
        Self {
            config: Arc::new(Config::default()),
            input_path,
            input_stem,
            output_dir,
            danmaku_path: None,
            audio_path: None,
            transcript: None,
            video_info: None,
            detect_resume: None,
            events: Vec::new(),
            started_at: Utc::now(),
            progress: Arc::new(NoopProgress) as Arc<dyn ProgressReporter>,
            pipeline_notes: Vec::new(),
        }
    }
}

#[cfg(test)]
mod event_range_tests {
    use super::*;

    fn ev(start: f64, end: f64) -> Event {
        Event {
            start: Timestamp::from_seconds(start),
            end: Timestamp::from_seconds(end),
            label: "t".into(),
            summary: String::new(),
            event_type: EventType::Other,
            source: EventSource::LLM,
            confidence: 1.0,
            popularity: 0.0,
            rank: 0,
            clip_dir: None,
            enabled: true,
            ranges: Vec::new(),
            topic: None,
        }
    }

    /// 空 `ranges` ⇒ 单段 = `start..end`（老 events.json 的全部情形）。
    #[test]
    fn empty_ranges_mean_single_span() {
        let e = ev(10.0, 40.0);
        let r = e.cut_ranges();
        assert_eq!(r.len(), 1);
        assert_eq!((r[0].start, r[0].end), (10.0, 40.0));
        assert!((e.content_secs() - 30.0).abs() < 1e-9);
    }

    /// 多段 ⇒ `start/end` 必须被 `normalize_ranges` 拉回"首段起、末段末"（两条真源不许打架）。
    #[test]
    fn ranges_cover_span() {
        let mut e = ev(0.0, 0.0);
        e.ranges = vec![
            EventRange::new(50.0, 70.0),
            EventRange::new(10.0, 20.0),
            EventRange::new(30.0, 40.0),
        ];
        e.normalize_ranges();
        assert_eq!(e.start.to_seconds(), 10.0);
        assert_eq!(e.end.to_seconds(), 70.0);
        // 排序 + 内容时长 = 各段之和（不含空隙）
        assert_eq!(
            e.ranges.iter().map(|r| r.start).collect::<Vec<_>>(),
            vec![10.0, 30.0, 50.0]
        );
        assert!((e.content_secs() - 40.0).abs() < 1e-9);
        // 跨度 60s ≠ 内容 40s（空隙 20s）—— 这正是"合并后按内容时长判达标"的依据
        assert!((e.end.to_seconds() - e.start.to_seconds() - 60.0).abs() < 1e-9);
    }

    /// 退化段（end <= start）在归一化时被丢掉；全空则保持单段语义。
    #[test]
    fn normalize_ranges_drops_degenerate() {
        let mut e = ev(5.0, 25.0);
        e.ranges = vec![EventRange::new(5.0, 5.0), EventRange::new(5.0, 25.0)];
        e.normalize_ranges();
        assert_eq!(e.ranges.len(), 1);
        assert_eq!((e.start.to_seconds(), e.end.to_seconds()), (5.0, 25.0));

        let mut e2 = ev(5.0, 25.0);
        e2.ranges = vec![EventRange::new(9.0, 9.0)];
        e2.normalize_ranges();
        assert!(e2.ranges.is_empty());
        assert_eq!(e2.cut_ranges().len(), 1); // 回退单段
    }

    /// **重叠段必须并成一段**：重叠 = 同一段内容被剪两遍（项目 12 的真实数据：
    /// `1625.7–1653.8` 里套着 `1632.0–1643.0` ⇒ `clip.mp4` 158.3s 里 11s 是重复的）。
    #[test]
    fn normalize_ranges_merges_overlapping() {
        let mut e = ev(0.0, 0.0);
        e.ranges = vec![
            EventRange::new(441.4, 486.5),
            EventRange::new(929.0, 1003.0),
            EventRange::new(1625.7, 1653.8),
            EventRange::new(1632.0, 1643.0), // ← 完全落在上一段里面
        ];
        e.normalize_ranges();
        assert_eq!(e.ranges.len(), 3, "重叠那段要并掉：{:?}", e.ranges);
        assert_eq!((e.ranges[2].start, e.ranges[2].end), (1625.7, 1653.8));
        // 内容时长不再重复计那 11s（四段之和 158.2 → 并后 147.2）
        assert!((e.content_secs() - 147.2).abs() < 1e-6);
        assert_eq!((e.start.to_seconds(), e.end.to_seconds()), (441.4, 1653.8));
    }

    /// 部分重叠取并集（`10–30` + `20–40` ⇒ `10–40`）；**首尾相接**也并（内容等价，少一段噪声）。
    #[test]
    fn merge_overlapping_ranges_union_and_touch() {
        let mut v = vec![
            EventRange::new(10.0, 30.0),
            EventRange::new(20.0, 40.0),
            EventRange::new(40.0, 50.0),
        ];
        merge_overlapping_ranges(&mut v);
        assert_eq!(v.len(), 1);
        assert_eq!((v[0].start, v[0].end), (10.0, 50.0));

        // **有内容的空隙不许并**（那是"分段剪、中间别的话题丢掉"的表达）
        let mut w = vec![EventRange::new(100.0, 120.0), EventRange::new(140.0, 160.0)];
        merge_overlapping_ranges(&mut w);
        assert_eq!(w.len(), 2, "中间隔了 20s，必须留着两段");

        // 内含段取外层（不许把外层截短）
        let mut x = vec![EventRange::new(100.0, 200.0), EventRange::new(120.0, 140.0)];
        merge_overlapping_ranges(&mut x);
        assert_eq!(x.len(), 1);
        assert_eq!((x[0].start, x[0].end), (100.0, 200.0));
        assert_eq!(x[0].end, 200.0, "短的那段在后，不许把外层 end 改小");

        // 单段/空 = 原样
        let mut y = vec![EventRange::new(1.0, 2.0)];
        merge_overlapping_ranges(&mut y);
        assert_eq!(y.len(), 1);
        let mut z: Vec<EventRange> = Vec::new();
        merge_overlapping_ranges(&mut z);
        assert!(z.is_empty());
    }
}
