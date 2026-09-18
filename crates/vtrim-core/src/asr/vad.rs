//! ⚠️ **本模块已不被转写流程使用**（2026-09 断句重做后）：
//! `Transcriber` 现在直接调 `local::transcribe_sensevoice` 拿 **`CharTimeline` + 分段**，
//! 一次 SenseVoice 同时供两条链路用（single 直接出字幕 / hybrid 当 qwen3 的时间轴）。
//! 这里再跑一遍 SenseVoice 只为拿 `SpeechSegment` 是**多算一遍**，而且拿不到时间轴 ——
//! 保留仅为兼容既有调用点，新代码别再用它。
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
