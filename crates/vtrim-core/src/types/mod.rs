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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TranscriptSegment {
    pub start: f64,
    pub end: f64,
    pub text: String,
    pub confidence: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RawSegment {
    pub index: usize,
    pub start: Timestamp,
    pub end: Timestamp,
    pub text: String,
    pub silence_before: f64,
    pub speech_density: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LLMEvent {
    pub start: String,
    pub end: String,
    pub label: String,
    pub summary: String,
    pub event_type: EventType,
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
    #[serde(default = "default_enabled")]
    pub enabled: bool,
}

fn default_enabled() -> bool { true }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventStore {
    pub titles: std::collections::BTreeMap<String, bool>,
    pub events: Vec<Event>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Segment {
    pub index: usize,
    pub event: Event,
    pub output_path: PathBuf,
    pub subtitle_path: PathBuf,
}

/// 弹幕条目（用于辅助字幕订正等场景）
#[derive(Debug, Clone)]
pub struct DanmakuRef {
    pub start: f64,
    pub end: f64,
    pub text: String,
}

#[derive(Clone)]
pub struct PipelineContext {
    pub config: Arc<Config>,
    pub input_path: PathBuf,
    pub input_stem: String,
    pub output_dir: PathBuf,
    pub danmaku_path: Option<PathBuf>,
    pub audio_path: Option<PathBuf>,
    pub vocal_path: Option<PathBuf>,
    pub transcript: Option<Transcript>,
    pub video_info: Option<VideoInfo>,
    pub events: Vec<Event>,
    pub segments: Vec<Segment>,
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
            vocal_path: None,
            transcript: None,
            video_info: None,
            events: Vec::new(),
            segments: Vec::new(),
            started_at: Utc::now(),
            progress: Arc::new(NoopProgress) as Arc<dyn ProgressReporter>,
        }
    }
}
