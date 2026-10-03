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
        }
    }
}
