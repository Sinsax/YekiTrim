use std::path::Path;

use anyhow::{Context, Result};

use crate::types::VideoInfo;

/// 用 ffmpeg 从视频提取 16kHz mono WAV
pub fn extract_audio(input: &Path, output_path: &Path) -> Result<()> {
    let result = crate::process::cmd("ffmpeg")
        .arg("-y")
        .arg("-loglevel")
        .arg("error")
        .arg("-i")
        .arg(input)
        .arg("-vn")
        .arg("-acodec")
        .arg("pcm_s16le")
        .arg("-ar")
        .arg("16000")
        .arg("-ac")
        .arg("1")
        .arg(output_path)
        .output()
        .context("ffmpeg 未找到，请确保 ffmpeg 已在 PATH 中")?;

    if !result.status.success() {
        let stderr = String::from_utf8_lossy(&result.stderr);
        anyhow::bail!("ffmpeg 音频提取失败: {}", stderr.trim());
    }
    Ok(())
}

/// 用 ffprobe 获取视频信息
pub fn get_video_info(input: &Path) -> Result<VideoInfo> {
    let output = crate::process::cmd("ffprobe")
        .arg("-v")
        .arg("quiet")
        .arg("-print_format")
        .arg("json")
        .arg("-show_format")
        .arg("-show_streams")
        .arg(input)
        .output()
        .context("ffprobe 未找到，请确保 ffprobe 已在 PATH 中")?;

    if !output.status.success() {
        anyhow::bail!("ffprobe 失败 (exit: {})", output.status);
    }

    let probe: ProbeOutput = serde_json::from_slice(&output.stdout)
        .context("无法解析 ffprobe 输出")?;

    let fmt_name = probe.format.as_ref().map(|f| f.format_name.clone()).unwrap_or_default();
    let video = probe.streams.iter().find(|s| s.codec_type == "video");
    let audio = probe.streams.iter().find(|s| s.codec_type == "audio");

    Ok(VideoInfo {
        duration_secs: probe
            .format
            .as_ref()
            .and_then(|f| f.duration.parse::<f64>().ok())
            .unwrap_or(0.0),
        width: video.map(|s| s.width).unwrap_or(0),
        height: video.map(|s| s.height).unwrap_or(0),
        video_codec: video.map(|s| s.codec_name.clone()).unwrap_or_default(),
        audio_codec: audio.map(|s| s.codec_name.clone()).unwrap_or_default(),
        format: fmt_name,
    })
}

#[derive(serde::Deserialize)]
struct ProbeOutput {
    streams: Vec<StreamInfo>,
    format: Option<FormatInfo>,
}

#[derive(serde::Deserialize)]
struct StreamInfo {
    codec_type: String,
    codec_name: String,
    #[serde(default)]
    width: u32,
    #[serde(default)]
    height: u32,
}

#[derive(serde::Deserialize)]
struct FormatInfo {
    format_name: String,
    duration: String,
}
