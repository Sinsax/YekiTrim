use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result};

pub fn detect_silence(audio: &Path, min_duration: f64) -> Result<Vec<(f64, f64)>> {
    let output = Command::new("ffmpeg")
        .arg("-loglevel")
        .arg("error")
        .arg("-i")
        .arg(audio)
        .arg("-af")
        .arg(format!("silencedetect=noise=-30dB:d={}", min_duration))
        .arg("-f")
        .arg("null")
        .arg("-")
        .output()
        .context("ffmpeg 未找到，请确保 ffmpeg 已在 PATH 中")?;

    let stderr = String::from_utf8_lossy(&output.stderr);

    let mut silences: Vec<(f64, f64)> = Vec::new();
    let mut current_start: Option<f64> = None;

    for line in stderr.lines() {
        let line = line.trim();
        let Some(after_bracket) = line.strip_prefix("[silencedetect") else {
            continue;
        };
        // after_bracket = " @ 0xADDR] silence_start: 12.345"
        let Some(payload) = after_bracket.split("] ").nth(1) else {
            continue;
        };
        if let Some(start_str) = payload.strip_prefix("silence_start: ") {
            if let Ok(start) = start_str.trim().parse::<f64>() {
                current_start = Some(start);
            }
        } else if let Some(end_str) = payload
            .split(" | ")
            .next()
            .and_then(|s| s.strip_prefix("silence_end: "))
        {
            if let Some(start) = current_start {
                if let Ok(end) = end_str.trim().parse::<f64>() {
                    silences.push((start, end));
                }
            }
            current_start = None;
        }
    }

    Ok(silences)
}

pub fn build_split_points(silences: &[(f64, f64)], total_duration: f64) -> Vec<f64> {
    let mut points: Vec<f64> = silences
        .iter()
        .map(|(s, e)| (s + e) / 2.0)
        .collect();
    points.sort_by(|a, b| a.partial_cmp(b).unwrap());
    points.dedup();

    if points.is_empty() || points[0] > 0.0 {
        points.insert(0, 0.0);
    } else {
        points[0] = 0.0;
    }
    if points.last().map_or(true, |&p| p < total_duration) {
        points.push(total_duration);
    } else {
        *points.last_mut().unwrap() = total_duration;
    }

    points
}
