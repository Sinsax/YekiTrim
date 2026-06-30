use std::path::Path;
use std::process::Command;

use async_trait::async_trait;
use anyhow::{Context, Result};

use crate::config::Config;
use crate::pipeline::{Stage, STAGE_TRANSCRIBE};
use crate::srt::write_srt;
use crate::types::{PipelineContext, Transcript, TranscriptSegment};

mod cloud_asr;
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

    fn is_enabled(&self, _config: &Config) -> bool {
        true
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

        // Cloud ASR backends
        let backend = &ctx.config.asr.backend;
        if backend == "bcut" || backend == "kuaishou" {
            tracing::info!("使用云端 ASR 后端: {}", backend);
            let segs = cloud_asr::transcribe(&audio_path, backend).await
                .map_err(|e| anyhow::anyhow!("云端 ASR 失败: {}", e))?;
            if segs.is_empty() {
                anyhow::bail!("云端 ASR 结果为空");
            }
            tracing::info!("转写完成: {} 个片段", segs.len());
            let transcript = Transcript { segments: segs, language: "zh".into() };
            let srt_path = ctx.output_dir.join("video.srt");
            write_srt(&transcript, &srt_path)?;
            let json_path = ctx.output_dir.join("transcript.json");
            save_transcript_json(&transcript, &json_path)?;
            ctx.transcript = Some(transcript);
            return Ok(());
        }

        // Local ASR: use sherpa-onnx as a library (model loaded once)
        local::ensure_ready(&ctx.config.asr)?;
        tracing::info!(
            "使用本地 ASR (sherpa-onnx), 线程数: {}",
            resolve_num_threads_display(&ctx.config.asr)
        );

        let recognizer = local::create_recognizer(&ctx.config.asr)?;
        let mut all_segments = local::transcribe_chunked(
            &recognizer,
            &audio_path,
            &local::ChunkConfig::from(&ctx.config.asr),
            &*ctx.progress,
        )?;

        post_process_segments(&mut all_segments);

        if all_segments.is_empty() {
            anyhow::bail!("转写结果为空，音频可能无有效语音");
        }

        tracing::info!("转写完成: {} 个片段", all_segments.len());

        let transcript = Transcript {
            segments: all_segments,
            language: "zh".into(),
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

fn resolve_num_threads_display(cfg: &crate::config::AsrConfig) -> String {
    if cfg.num_threads == 0 {
        let cpus = std::thread::available_parallelism()
            .map(|n| n.get()).unwrap_or(4);
        format!("auto ({})", cpus)
    } else {
        cfg.num_threads.to_string()
    }
}

fn post_process_segments(segments: &mut Vec<TranscriptSegment>) {
    if segments.is_empty() {
        return;
    }

    segments.retain(|s| {
        let text = s.text.trim();
        if text.is_empty() || text == "\u{FFFD}" {
            return false;
        }
        let meaningful = text.chars().filter(|c| {
            c.is_ascii_alphanumeric()
                || ('\u{4E00}'..='\u{9FFF}').contains(c)
                || ('\u{3000}'..='\u{303F}').contains(c)
                || ('\u{FF00}'..='\u{FFEF}').contains(c)
        }).count();
        meaningful >= 2
    });

    let mut i = 0;
    while i + 1 < segments.len() {
        let cur = &segments[i];
        let nxt = &segments[i + 1];
        let gap = nxt.start - cur.end;
        let cur_len = cur.text.trim().chars().count();

        if cur_len <= 2 && gap < 2.0 {
            let removed = segments.remove(i);
            segments[i].start = removed.start;
            segments[i].text = format!("{}{}", removed.text.trim(), segments[i].text.trim());
            continue;
        }

        let nxt_len = nxt.text.trim().chars().count();
        if nxt_len <= 2 && gap < 1.0 {
            let removed = segments.remove(i + 1);
            segments[i].end = removed.end;
            segments[i].text = format!("{}{}", segments[i].text.trim(), removed.text.trim());
            continue;
        }

        if cur_len <= 4 && nxt_len <= 4 && gap < 0.5 {
            let removed = segments.remove(i + 1);
            segments[i].end = removed.end;
            segments[i].text = format!("{}{}", segments[i].text.trim(), removed.text.trim());
            continue;
        }

        i += 1;
    }
}

pub(super) fn wav_duration_secs(path: &Path) -> Result<f64> {
    let output = Command::new("ffprobe")
        .args([
            "-v", "error",
            "-show_entries", "format=duration",
            "-of", "default=noprint_wrappers=1:nokey=1",
            path.to_str().unwrap(),
        ])
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
