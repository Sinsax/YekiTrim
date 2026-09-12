use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result};

/// 长静音判定阈值（秒）：超过此时长的连续静音视为可屏蔽区间。
pub const DEFAULT_MIN_GAP_SECS: f64 = 2.0;

/// 音频分析默认噪声门限（dB）。
pub const DEFAULT_NOISE_DB: f64 = -30.0;

pub fn detect_silence_from_srt(srt_path: &Path, threshold: f64) -> Result<Vec<(f64, f64)>> {
    let content = std::fs::read_to_string(srt_path)?;
    let transcript = crate::srt::parse_srt(&content)?;
    let segs = &transcript.segments;

    if segs.len() < 2 {
        return Ok(Vec::new());
    }

    let mut segments = Vec::new();
    for pair in segs.windows(2) {
        let gap = pair[1].start - pair[0].end;
        if gap >= threshold {
            segments.push((pair[0].end, pair[1].start));
        }
    }

    tracing::debug!(
        "SRT gap 检测: {} 段静音 (阈值={}s)",
        segments.len(),
        threshold,
    );
    Ok(segments)
}

pub fn detect_silence(input: &Path, duration: f64, noise_db: f64) -> Result<Vec<(f64, f64)>> {
    let result = Command::new("ffmpeg")
        .arg("-y")
        .arg("-i")
        .arg(input)
        .arg("-af")
        .arg(format!("silencedetect=n={}dB:d={}", noise_db, duration))
        .arg("-f")
        .arg("null")
        .arg("-")
        .output()
        .with_context(|| "ffmpeg silencedetect 失败")?;

    let stderr = String::from_utf8_lossy(&result.stderr);
    if !result.status.success() {
        anyhow::bail!("ffmpeg silencedetect 失败:\n{}", stderr.trim());
    }

    let mut segments: Vec<(f64, f64)> = Vec::new();
    let mut current_start: Option<f64> = None;

    for line in stderr.lines() {
        let line = line.trim();
        if let Some(pos) = line.find("silence_start: ") {
            let val = line[pos + 14..].trim();
            if let Ok(t) = val.parse::<f64>() {
                current_start = Some(t);
            }
        } else if let Some(pos) = line.find("silence_end: ") {
            let after = &line[pos + 12..];
            if let Some(pipe) = after.find('|') {
                let t_str = after[..pipe].trim();
                if let (Ok(end), Some(start)) = (t_str.parse::<f64>(), current_start) {
                    segments.push((start, end));
                    current_start = None;
                }
            }
        }
    }

    Ok(segments)
}

/// 检测长静音区间，供「消音版 = 映射到屏蔽轨道」使用。
///
/// 优先按字幕间隔检测（快、可复现，无需解码视频）；没有字幕文件时才退回音频分析。
/// 结果只用于写入 `mute_ranges`，不产生任何新的视频/字幕文件。
pub fn detect_silence_ranges(
    video: &Path,
    srt: Option<&Path>,
    min_gap: f64,
    noise_db: f64,
) -> Result<Vec<(f64, f64)>> {
    match srt {
        Some(srt) if srt.exists() => {
            tracing::debug!("按字幕间隔检测静音: {}", srt.display());
            detect_silence_from_srt(srt, min_gap)
        }
        _ => {
            if !video.exists() {
                anyhow::bail!("缺少字幕与视频文件，无法检测静音");
            }
            tracing::debug!("按音频分析检测静音 (noise={}dB)", noise_db);
            detect_silence(video, min_gap, noise_db)
        }
    }
}

