//! 转写中间产物落盘（`output/asr/`）。
//!
//! 目的只有一个：让"断句为什么长这样"变成**可查、可离线复算**的事实，而不是只能重跑一遍
//! 猜。一次 3 小时录播约 200 块，产物总共几 MB —— 相对 wav 可以忽略。
//!
//! | 文件 | 内容 | 什么时候看 |
//! |------|------|-----------|
//! | `raw/chunk-NNN.json` | 每块的原始 token + 时间戳 + 重叠区归属 | "模型到底听到了什么" |
//! | `timeline.json` | 合并后的字符级时间轴 + 气口位置 | "气口在哪、阈值合不合适" |
//! | `align.json` | 每个窗口的 qwen3 文本被切成几条、切点来自标点还是气口 | "为什么这条没切开/切多了" |
//! | `segments.raw.json` | 噪声清理**之前**的字幕段 | "是不是被后处理吃掉了" |
//!
//! ⚠️ 这里记的是**切分时看到的文本**，即 `asr::text::normalize` **之前**的原文 ——
//! 所以它和落盘的 `transcript.json` / `video.srt` 会有细微差别（行尾标点、转述引号）。
//! 这是有意的：断句是按原文标点做的，产物要能还原当时的输入；对着两边差异怀疑是 bug 之前先看这条。

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::Serialize;

use crate::asr::align::{BoundarySource, CharTimeline};
use crate::asr::local::RawChunk;
use crate::types::TranscriptSegment;

/// 一个语音窗口的断句明细（`align.json` 的一条）。
#[derive(Debug, Serialize)]
pub struct WindowRecord {
    pub window_index: usize,
    pub window_start: f64,
    pub window_end: f64,
    /// 该窗口喂给 ASR 的文本（hybrid 下是 qwen3 的填词结果）
    pub text: String,
    /// 对齐命中率（0~1）：qwen3 的字有多少在 SenseVoice 时间轴上找到了位置。
    /// 过低说明两份文本差太多，切点时间基本靠插值兜底。
    pub align_hit_rate: f64,
    pub resyncs: usize,
    /// 该窗口 SenseVoice 的时间轴字数与时长 → 用来判断"这段到底像不像人声"
    /// （低密度 = 音乐/噪声段，qwen3 容易在这里胡说或返回空）。
    /// 攒够数据再决定要不要按它过滤，**先只记录不行动**。
    pub timeline_chars: usize,
    pub duration_secs: f64,
    /// 是 qwen3 返回了空文本才会被跳过的窗口（这里恒为非空文本的窗口；
    /// 空文本窗口不落 `align.json`，由 `empty_windows.json` 单独记）
    pub text_chars: usize,
    pub pieces: Vec<PieceRecord>,
}

#[derive(Debug, Serialize)]
pub struct PieceRecord {
    pub start: f64,
    pub end: f64,
    pub text: String,
    pub boundary: Option<BoundarySource>,
}

/// 时间轴的紧凑表示：字合成一个字符串，时间单独成数组（比逐字 JSON 小一个量级）。
#[derive(Debug, Serialize)]
pub struct TimelineRecord {
    pub text: String,
    pub chars: usize,
    pub times: Vec<f64>,
    pub breath_gap: f64,
    pub breaths: Vec<BreathRecord>,
}

#[derive(Debug, Serialize)]
pub struct BreathRecord {
    pub index: usize,
    pub time: f64,
    pub gap: f64,
}

pub fn timeline_record(timeline: &CharTimeline, breath_gap: f64) -> TimelineRecord {
    TimelineRecord {
        text: timeline.text(),
        chars: timeline.len(),
        times: timeline.times.clone(),
        breath_gap,
        breaths: timeline
            .breath_points(breath_gap)
            .into_iter()
            .map(|b| BreathRecord { index: b.index, time: b.time, gap: b.gap })
            .collect(),
    }
}

/// 把全部中间产物写进 `<output_dir>/asr/`。
///
/// 失败只 warn：产物是给人看的，不该让整次转写跟着失败。
pub fn write_all(
    output_dir: &Path,
    chunks: &[RawChunk],
    timeline: &CharTimeline,
    breath_gap: f64,
    windows: &[WindowRecord],
    raw_segments: &[TranscriptSegment],
) {
    if let Err(e) = write_all_inner(output_dir, chunks, timeline, breath_gap, windows, raw_segments) {
        tracing::warn!("ASR 中间产物写入失败（不影响转写结果）: {e:#}");
    }
}

fn write_all_inner(
    output_dir: &Path,
    chunks: &[RawChunk],
    timeline: &CharTimeline,
    breath_gap: f64,
    windows: &[WindowRecord],
    raw_segments: &[TranscriptSegment],
) -> Result<()> {
    let dir = output_dir.join("asr");
    let raw_dir = dir.join("raw");
    std::fs::create_dir_all(&raw_dir).with_context(|| format!("创建 {}", raw_dir.display()))?;

    for chunk in chunks {
        write_json(&raw_dir.join(format!("chunk-{:03}.json", chunk.index)), chunk)?;
    }
    write_json(&dir.join("timeline.json"), &timeline_record(timeline, breath_gap))?;
    write_json(&dir.join("align.json"), &windows)?;
    write_json(&dir.join("segments.raw.json"), &raw_segments)?;

    tracing::info!(
        "ASR 中间产物: {} ({} 块 / {} 字 / {} 窗口)",
        dir.display(),
        chunks.len(),
        timeline.len(),
        windows.len()
    );
    Ok(())
}

/// 清掉上一次的产物。
///
/// 必须做：`raw/` 是**按块编号命名**的，上一次跑如果块更多（比如换了更小的
/// `chunk_duration`），残留的 `chunk-200.json` 会混在新产物里冒充本次结果。
pub fn reset(output_dir: &Path) -> Result<()> {
    let dir = output_dir.join("asr");
    if dir.exists() {
        std::fs::remove_dir_all(&dir).with_context(|| format!("清理 {}", dir.display()))?;
    }
    Ok(())
}

fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    let json = serde_json::to_string_pretty(value)?;
    std::fs::write(path, json).with_context(|| format!("写入 {}", path.display()))?;
    Ok(())
}

/// `output/asr/` 的绝对路径（供 CLI/日志显示）。
pub fn dir_of(output_dir: &Path) -> PathBuf {
    output_dir.join("asr")
}
