use std::path::Path;

use anyhow::{bail, Result};

use crate::config::AsrConfig;

/// A detected speech interval in seconds (absolute on the original timeline).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SpeechSegment {
    pub start: f64,
    pub end: f64,
    /// 保留字段，当前恒为 false（不做音乐判断）
    pub is_music: bool,
}

/// SenseVoice-based alignment: use SenseVoice segments as speech intervals.
pub fn detect_sensevoice(cfg: &AsrConfig, wav_path: &Path) -> Result<Vec<SpeechSegment>> {
    let output = crate::asr::local::transcribe_sensevoice(cfg, wav_path, &crate::progress::NoopProgress)?;
    let mut segments: Vec<SpeechSegment> = output
        .segments
        .iter()
        .map(|s| SpeechSegment {
            start: s.start,
            end: s.end,
            is_music: false,
        })
        .collect();
    segments.sort_by(|a, b| a.start.partial_cmp(&b.start).unwrap());
    Ok(segments)
}

/// 打轴调度（当前仅 SenseVoice）。
pub fn detect(cfg: &AsrConfig, wav_path: &Path) -> Result<Vec<SpeechSegment>> {
    let align = cfg.align_model.trim();
    match align {
        "sensevoice" | "hybrid" | "" => detect_sensevoice(cfg, wav_path),
        other => bail!("未知打轴模型: {other} (可选: sensevoice / hybrid)"),
    }
}
