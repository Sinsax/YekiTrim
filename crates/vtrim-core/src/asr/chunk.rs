use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result};

#[allow(dead_code)]
pub struct AudioChunk {
    pub path: PathBuf,
    pub offset: f64,
    pub overlap_end: Option<f64>,
}

#[allow(dead_code)]
pub fn split_at_points(
    source: &Path,
    split_points: &[f64],
    temp_dir: &Path,
) -> Result<Vec<AudioChunk>> {
    std::fs::create_dir_all(temp_dir)?;

    let mut chunks = Vec::new();

    for i in 0..split_points.len() - 1 {
        let start = split_points[i];
        let end = split_points[i + 1];
        let duration = end - start;

        if duration < 0.5 {
            continue;
        }

        let chunk_path = temp_dir.join(format!("chunk_{:04}.wav", i));

        let output = Command::new("ffmpeg")
            .arg("-y")
            .arg("-loglevel")
            .arg("error")
            .arg("-ss")
            .arg(format!("{:.3}", start))
            .arg("-to")
            .arg(format!("{:.3}", end))
            .arg("-i")
            .arg(source)
            .arg("-c")
            .arg("copy")
            .arg(&chunk_path)
            .output()
            .context("ffmpeg 音频分割失败")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("ffmpeg 分割失败 (chunk {}): {}", i, stderr.trim());
        }

        chunks.push(AudioChunk {
            path: chunk_path,
            offset: start,
            overlap_end: None,
        });
    }

    Ok(chunks)
}

pub fn split_fixed_with_overlap(
    source: &Path,
    total_duration: f64,
    chunk_dur: f64,
    overlap: f64,
    temp_dir: &Path,
) -> Result<Vec<AudioChunk>> {
    std::fs::create_dir_all(temp_dir)?;

    let mut chunks = Vec::new();
    let step = chunk_dur - overlap;
    let mut i = 0u32;

    let mut start = 0.0;
    while start < total_duration {
        let end = (start + chunk_dur).min(total_duration);
        let actual_dur = end - start;

        if actual_dur < 0.5 {
            break;
        }

        let chunk_path = temp_dir.join(format!("chunk_{:04}.wav", i));

        let output = Command::new("ffmpeg")
            .arg("-y")
            .arg("-loglevel")
            .arg("error")
            .arg("-ss")
            .arg(format!("{:.3}", start))
            .arg("-to")
            .arg(format!("{:.3}", end))
            .arg("-i")
            .arg(source)
            .arg("-c")
            .arg("copy")
            .arg(&chunk_path)
            .output()
            .context("ffmpeg 音频分割失败")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("ffmpeg 分割失败 (chunk {}): {}", i, stderr.trim());
        }

        let overlap_end = if end < total_duration {
            Some(end)
        } else {
            None
        };

        chunks.push(AudioChunk {
            path: chunk_path,
            offset: start,
            overlap_end,
        });

        start += step;
        i += 1;
    }

    Ok(chunks)
}

pub fn cleanup(temp_dir: &Path) {
    if let Err(e) = std::fs::remove_dir_all(temp_dir) {
        tracing::warn!("清理临时文件失败: {}", e);
    }
}
