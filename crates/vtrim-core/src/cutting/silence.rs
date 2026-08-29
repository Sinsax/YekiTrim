use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result};

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

pub fn remove_silence(input: &Path, output: &Path, duration: f64, srt_path: Option<&Path>, noise_db: f64) -> Result<Vec<(f64, f64)>> {
    let segments = match srt_path {
        Some(srt) if srt.exists() => {
            tracing::info!("使用 SRT 间隔检测静音: {}", srt.display());
            detect_silence_from_srt(srt, duration)?
        }
        _ => {
            tracing::info!("使用音频分析检测静音 (noise={}dB)", noise_db);
            detect_silence(input, duration, noise_db)?
        }
    };

    if segments.is_empty() {
        std::fs::copy(input, output)?;
        return Ok(segments);
    }

    // Build keep ranges (inverse of silence segments)
    let mut keep_ranges: Vec<(f64, Option<f64>)> = Vec::new();
    let mut prev_end = 0.0;
    for (s, e) in &segments {
        if *s > prev_end {
            keep_ranges.push((prev_end, Some(*s)));
        }
        prev_end = *e;
    }
    keep_ranges.push((prev_end, None));
    keep_ranges.retain(|&(start, end)| end.is_none_or(|e| e > start));

    // 前后留 0.3s 缓冲，让切除更自然
    const PAD: f64 = 0.3;
    for range in &mut keep_ranges {
        range.0 = (range.0 - PAD).max(0.0);
        if let Some(ref mut end) = range.1 {
            *end += PAD;
        }
    }

    // 合并因 padding 而重叠的相邻段
    let mut merged: Vec<(f64, Option<f64>)> = Vec::new();
    for range in keep_ranges {
        if let Some(last) = merged.last_mut() {
            let gap = range.0 - last.1.unwrap_or(f64::MAX);
            if gap < 0.0 {
                last.1 = range.1;
                continue;
            }
        }
        merged.push(range);
    }
    keep_ranges = merged;

    // 丢弃过短的保留段 (< 1s)
    keep_ranges.retain(|&(s, e)| e.is_none_or(|e| e - s > 1.0));

    let mut filter_parts: Vec<String> = Vec::new();
    let mut labels: Vec<String> = Vec::new();

    for (i, &(start, end)) in keep_ranges.iter().enumerate() {
        let v_label = format!("v{}", i);
        let a_label = format!("a{}", i);
        let vf = match end {
            Some(e) => format!("[0:v]trim=start={:.3}:end={:.3},setpts=PTS-STARTPTS[{}]", start, e, v_label),
            None => format!("[0:v]trim=start={:.3},setpts=PTS-STARTPTS[{}]", start, v_label),
        };
        let af = match end {
            Some(e) => format!("[0:a]atrim=start={:.3}:end={:.3},asetpts=PTS-STARTPTS[{}]", start, e, a_label),
            None => format!("[0:a]atrim=start={:.3},asetpts=PTS-STARTPTS[{}]", start, a_label),
        };
        filter_parts.push(vf);
        filter_parts.push(af);
        labels.push(format!("[{}][{}]", v_label, a_label));
    }

    let n = keep_ranges.len();

    if n > 1 {
        filter_parts.push(format!("{}concat=n={}:v=1:a=1[outv][outa]", labels.join(""), n));
    }

    let filter_complex = filter_parts.join(";");

    let mut cmd = Command::new("ffmpeg");
    cmd.arg("-y")
        .arg("-loglevel")
        .arg("error")
        .arg("-i")
        .arg(input)
        .arg("-filter_complex")
        .arg(&filter_complex);

    if n > 1 {
        cmd.arg("-map").arg("[outv]").arg("-map").arg("[outa]");
    } else {
        cmd.arg("-map").arg("[v0]").arg("-map").arg("[a0]");
    }

    let result = cmd
        .arg("-c:v")
        .arg("libx264")
        .arg("-preset")
        .arg("fast")
        .arg("-crf")
        .arg("23")
        .arg("-c:a")
        .arg("aac")
        .arg(output)
        .output()
        .with_context(|| "ffmpeg 静音移除失败")?;

    if !result.status.success() {
        let stderr = String::from_utf8_lossy(&result.stderr);
        anyhow::bail!("ffmpeg 静音移除失败: {}", stderr.trim());
    }

    // 返回实际切除的静音段（扣除 PAD 缓冲后），用于字幕偏移计算
    let actual_silence: Vec<(f64, f64)> = segments
        .iter()
        .map(|(s, e)| (s + PAD, e - PAD))
        .filter(|(s, e)| e > s)
        .collect();
    Ok(actual_silence)
}

pub fn adjust_srt(input: &Path, output: &Path, silence_segments: &[(f64, f64)]) -> Result<()> {
    let content = std::fs::read_to_string(input)?;
    let mut transcript = crate::srt::parse_srt(&content)?;

    for seg in &mut transcript.segments {
        for (s, e) in silence_segments {
            if seg.start > *s && seg.start < *e {
                seg.start = *e;
            }
            if seg.end > *s && seg.end < *e {
                seg.end = *s;
            }
        }
    }

    transcript.segments.retain_mut(|seg| {
        if seg.start >= seg.end {
            return false;
        }
        let shift_before_start: f64 = silence_segments
            .iter()
            .filter(|(_, e)| *e <= seg.start)
            .map(|(s, e)| e - s)
            .sum();
        let shift_before_end: f64 = silence_segments
            .iter()
            .filter(|(_, e)| *e <= seg.end)
            .map(|(s, e)| e - s)
            .sum();

        seg.start = (seg.start - shift_before_start).max(0.0);
        seg.end = (seg.end - shift_before_end).max(0.0);
        seg.start < seg.end
    });

    crate::srt::write_srt(&transcript, output).context("写入调整后字幕失败")
}

