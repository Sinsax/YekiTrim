use std::path::Path;
use std::process::Command;
use std::sync::OnceLock;

use async_trait::async_trait;
use anyhow::{bail, Context, Result};
use tokio::sync::Mutex;

use crate::pipeline::{Stage, STAGE_TRANSCRIBE};
use crate::srt::write_srt;
use crate::types::{AsrMetadata, PipelineContext, Transcript, TranscriptSegment};

pub mod correct;
pub mod local;
pub mod vad;

pub struct Transcriber;

/// 全局 qwen3 recognizer，用 Mutex 保证同一时间只有一个识别任务在跑。
/// 超时/异常后丢弃，下次调用会重建，避免状态污染。
static QWEN3_RECOGNIZER: OnceLock<Mutex<Option<sherpa_onnx::OfflineRecognizer>>> = OnceLock::new();

fn qwen3_recognizer_lock() -> &'static Mutex<Option<sherpa_onnx::OfflineRecognizer>> {
    QWEN3_RECOGNIZER.get_or_init(|| Mutex::new(None))
}

/// 在全局锁内执行一次 qwen3 识别。
///
/// - 从锁中 take 出 recognizer，避免并发访问
/// - 放到 `spawn_blocking` 中执行，避免阻塞 async runtime
/// - 超时/异常时丢弃 recognizer（可能状态污染），下次重建
/// - 正常完成后放回 recognizer 复用
async fn transcribe_segment_with_qwen3(
    cfg: &crate::config::AsrConfig,
    samples: Vec<f32>,
    sample_rate: usize,
    timeout: std::time::Duration,
) -> Result<String> {
    let mut guard = qwen3_recognizer_lock().lock().await;
    if guard.is_none() {
        *guard = Some(local::create_qwen3_recognizer(cfg)?);
    }

    let rec = guard.take().expect("recognizer 已初始化");
    let result = tokio::time::timeout(
        timeout,
        tokio::task::spawn_blocking(move || {
            let r = local::transcribe_qwen3_samples(&rec, &samples, sample_rate);
            (rec, r)
        }),
    )
    .await;

    match result {
        Ok(Ok((rec, Ok(text)))) => {
            *guard = Some(rec);
            Ok(text)
        }
        Ok(Ok((_rec, Err(e)))) => {
            // recognizer 可能状态异常，不回收，下次重建
            tracing::warn!("qwen3 识别失败，丢弃 recognizer 待重建: {e:#}");
            Err(e)
        }
        Ok(Err(e)) => {
            tracing::warn!("qwen3 任务异常，丢弃 recognizer 待重建: {e:#}");
            Err(anyhow::anyhow!("qwen3 任务异常: {e}"))
        }
        Err(_) => {
            // 超时：rec 仍在后台阻塞任务中，无法回收，丢弃
            tracing::warn!("qwen3 推理超时({timeout:?})，丢弃 recognizer 待重建");
            Err(anyhow::anyhow!("qwen3 推理超时"))
        }
    }
}

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

        // ========== 判断模式 ==========
        let mode = ctx.config.asr.mode.trim().to_string();

        let all_segments: Vec<TranscriptSegment>;
        let metadata;

        if mode == "single" {
            // 单模型模式：SenseVoice 直接转写（自带精确时间戳）
            tracing::info!("单模型模式: SenseVoice 直接转写");
            local::ensure_ready(&ctx.config.asr)?;
            let output = local::transcribe_sensevoice(
                &ctx.config.asr,
                &audio_path,
                &*ctx.progress,
            )?;
            let mut segs = output.segments;
            segs.retain(|s| s.text.trim() != "\u{FFFD}");
            if segs.is_empty() {
                bail!("SenseVoice 转写结果为空");
            }
            all_segments = segs;
            metadata = output.metadata;
        } else {
            // 混合模式：SenseVoice 打轴 + qwen3 填词
            tracing::info!("混合模式: SenseVoice 打轴 + qwen3 填词");

            // ========== 打轴/分段 ==========
            let speech_segments = vad::detect_sensevoice(&ctx.config.asr, &audio_path)?;
            if speech_segments.is_empty() {
                bail!("打轴未检测到语音段");
            }
            tracing::info!("打轴检测到 {} 个语音段", speech_segments.len());

            // ========== 识别/填词 ==========
            const MAX_QWEN_SEGMENT_SECS: f64 = 60.0;
            let mut segs: Vec<TranscriptSegment> = Vec::new();
            let mut total_items = 0usize;

            for (i, seg) in speech_segments.iter().enumerate() {
                let mut sub_start = seg.start;
                while sub_start < seg.end {
                    let sub_end = (sub_start + MAX_QWEN_SEGMENT_SECS).min(seg.end);
                    tracing::info!(
                        "识别语音段 {}/{} [{:.1}s - {:.1}s]",
                        i + 1,
                        speech_segments.len(),
                        sub_start,
                        sub_end
                    );
                    let (samples, sample_rate) =
                        local::read_wav_range(&audio_path, sub_start, sub_end)?;

                    let seg_secs = (sub_end - sub_start).max(0.5);
                    let timeout_secs = (seg_secs * 3.0 + 5.0).clamp(10.0, 180.0);
                    let timeout = std::time::Duration::from_secs_f64(timeout_secs);

                    let text = match transcribe_segment_with_qwen3(
                        &ctx.config.asr,
                        samples,
                        sample_rate,
                        timeout,
                    )
                    .await
                    {
                        Ok(t) => t,
                        Err(e) => {
                            tracing::warn!(
                                "语音段 [{:.1}s - {:.1}s] 跳过: {e:#}",
                                sub_start,
                                sub_end
                            );
                            String::new()
                        }
                    };
                    let text = text.trim().trim_end_matches(['。', '！', '？', '.', '!', '?']).trim().to_string();
                    if !text.is_empty() {
                        segs.push(TranscriptSegment {
                            start: sub_start,
                            end: sub_end,
                            text,
                            confidence: 1.0,
                        });
                        total_items += 1;
                    }
                    sub_start = sub_end;
                }
                ctx.progress.stage_progress((i + 1) as u64, speech_segments.len() as u64);
            }

            if segs.is_empty() {
                bail!("qwen3 填词结果为空");
            }
            all_segments = segs;
            metadata = AsrMetadata {
                anchors: total_items,
                anchor_coverage: 1.0,
                max_interp_span: 0,
                fallback_reason: None,
                model: "qwen3-asr".into(),
            };
        }

        tracing::info!("转写完成: {} 个片段", all_segments.len());
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
