use std::path::Path;
use std::process::Command;

use async_trait::async_trait;
use anyhow::{Context, Result};

use crate::pipeline::{Stage, STAGE_TRANSCRIBE};
use crate::srt::write_srt;
use crate::types::{PipelineContext, Transcript};

pub mod correct;
pub mod local;

pub struct Transcriber;

#[async_trait]
impl Stage for Transcriber {
    fn name(&self) -> &'static str {
        "Transcriber"
    }

    fn stage_number(&self) -> u32 {
        STAGE_TRANSCRIBE
    }

    async fn run(&self, ctx: &mut PipelineContext) -> Result<()> {
        let audio_path = match &ctx.audio_path {
            Some(p) => p.clone(),
            None => {
                let fallback = ctx.output_dir.join("video.wav");
                let from_cfg = ctx.config.project.audio_file.as_ref()
                    .filter(|p| !p.is_empty())
                    .map(std::path::PathBuf::from);
                let path = from_cfg
                    .filter(|p| p.exists())
                    .or_else(|| {
                        if fallback.exists() { Some(fallback.clone()) } else { None }
                    });
                match path {
                    Some(p) => p,
                    None => anyhow::bail!(
                        "AudioExtractor 阶段未执行，缺少音频文件 (查找路径: {} 或 config.audio_file)",
                        fallback.display()
                    ),
                }
            }
        };

        let total_duration = wav_duration_secs(&audio_path)?;
        tracing::info!("音频时长: {:.1}s", total_duration);

        // Local ASR: 单模型 sensevoice。
        local::ensure_ready(&ctx.config.asr)?;
        tracing::info!("使用单模型 SenseVoice 转写");

        let output = local::transcribe_sensevoice(
            &ctx.config.asr,
            &audio_path,
            &*ctx.progress,
        )?;
        let mut all_segments = output.segments;

        all_segments.retain(|s| s.text.trim() != "\u{FFFD}");

        if all_segments.is_empty() {
            anyhow::bail!("转写结果为空，音频可能无有效语音");
        }

        tracing::info!("转写完成: {} 个片段", all_segments.len());

        let mut metadata = output.metadata;
        metadata.model = ctx.config.asr.model.clone();
        let transcript = Transcript {
            segments: all_segments,
            language: "zh".into(),
            asr_metadata: Some(metadata),
        };

        let srt_path = ctx.output_dir.join("video.srt");
        write_srt(&transcript, &srt_path)?;
        let json_path = ctx.output_dir.join("transcript.json");
        save_transcript_json(&transcript, &json_path)?;
        tracing::info!("字幕已生成: {}", srt_path.display());
        tracing::info!("转录 JSON 已保存: {}", json_path.display());

        ctx.transcript = Some(transcript);

        tracing::info!("语音转文字完成");
        Ok(())
    }
}

pub(super) fn wav_duration_secs(path: &Path) -> Result<f64> {
    let output = Command::new("ffprobe")
        .arg("-v")
        .arg("error")
        .arg("-show_entries")
        .arg("format=duration")
        .arg("-of")
        .arg("default=noprint_wrappers=1:nokey=1")
        .arg(path)
        .output()
        .with_context(|| "ffprobe 获取音频时长失败")?;
    let s = String::from_utf8_lossy(&output.stdout);
    s.trim()
        .parse::<f64>()
        .map_err(|e| anyhow::anyhow!("无法解析音频时长 '{}': {}", s.trim(), e))
}

fn save_transcript_json(transcript: &Transcript, path: &Path) -> Result<()> {
    let json = serde_json::to_string_pretty(transcript)?;
    std::fs::write(path, json)?;
    Ok(())
}
