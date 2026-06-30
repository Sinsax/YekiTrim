use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use sherpa_onnx::{
    OfflineRecognizer, OfflineRecognizerConfig, OfflineSenseVoiceModelConfig, Wave,
};

use crate::config::AsrConfig;
use crate::progress::ProgressReporter;
use crate::types::TranscriptSegment;

fn model_dir(cfg: &AsrConfig) -> PathBuf {
    if !cfg.local_model_dir.is_empty() {
        PathBuf::from(&cfg.local_model_dir)
    } else {
        crate::path::vtrim_home().join("models").join(&cfg.local_model)
    }
}

fn find_model(model_dir: &Path) -> Result<PathBuf> {
    let candidates = ["model.int8.onnx", "model.onnx", "sense-voice.onnx"];
    for name in &candidates {
        let p = model_dir.join(name);
        if p.exists() {
            return Ok(p);
        }
    }
    bail!(
        "模型目录 {} 中未找到模型文件 (model.int8.onnx / model.onnx)",
        model_dir.display()
    );
}

fn resolve_num_threads(cfg: &AsrConfig) -> i32 {
    if cfg.num_threads == 0 {
        std::thread::available_parallelism()
            .map(|n| n.get() as i32)
            .unwrap_or(4)
    } else {
        cfg.num_threads as i32
    }
}

fn resolve_provider(cfg: &AsrConfig) -> Option<String> {
    let p = cfg.provider.trim();
    if p.is_empty() || p == "cpu" {
        None
    } else {
        Some(p.to_string())
    }
}

pub fn create_recognizer(cfg: &AsrConfig) -> Result<OfflineRecognizer> {
    let mdir = model_dir(cfg);
    let model_path = find_model(&mdir)?;
    let tokens_path = mdir.join("tokens.txt");
    if !tokens_path.exists() {
        bail!("模型目录缺少 tokens.txt: {}", mdir.display());
    }

    let mut config = OfflineRecognizerConfig::default();
    config.model_config.num_threads = resolve_num_threads(cfg);
    config.model_config.provider = resolve_provider(cfg);
    let language = if cfg.language.is_empty() { "auto".to_string() } else { cfg.language.clone() };
    config.model_config.sense_voice = OfflineSenseVoiceModelConfig {
        model: Some(model_path.to_str().unwrap().into()),
        language: Some(language),
        use_itn: true,
    };
    config.model_config.tokens = Some(tokens_path.to_str().unwrap().into());

    OfflineRecognizer::create(&config)
        .context("创建 OfflineRecognizer 失败，请检查模型文件是否正确")
}

pub struct ChunkConfig {
    pub chunk_duration: f64,
    pub chunk_overlap: f64,
}

impl Default for ChunkConfig {
    fn default() -> Self {
        Self {
            chunk_duration: 60.0,
            chunk_overlap: 10.0,
        }
    }
}

impl From<&AsrConfig> for ChunkConfig {
    fn from(cfg: &AsrConfig) -> Self {
        Self {
            chunk_duration: cfg.chunk_duration,
            chunk_overlap: cfg.chunk_overlap,
        }
    }
}

pub fn transcribe_chunked(
    recognizer: &OfflineRecognizer,
    audio_path: &Path,
    chunk_cfg: &ChunkConfig,
    progress: &dyn ProgressReporter,
) -> Result<Vec<TranscriptSegment>> {
    let audio_str = audio_path.to_str().context("音频路径包含非 UTF-8 字符")?;
    let wave = Wave::read(audio_str).context("读取音频文件失败")?;

    let sample_rate = wave.sample_rate() as usize;
    let total_samples = wave.num_samples() as usize;
    let all_samples = wave.samples();

    let chunk_samples = (chunk_cfg.chunk_duration * sample_rate as f64) as usize;
    let overlap_samples = (chunk_cfg.chunk_overlap * sample_rate as f64) as usize;
    let step_samples = chunk_samples.saturating_sub(overlap_samples);

    if step_samples == 0 {
        bail!("chunk_duration ({}) 必须大于 chunk_overlap ({})", chunk_cfg.chunk_duration, chunk_cfg.chunk_overlap);
    }

    let total_chunks = total_samples.div_ceil(step_samples);
    tracing::info!(
        "音频分 {total_chunks} 块 (每块 {:.0}s, 重叠 {:.0}s, 步进 {:.1}s)",
        chunk_cfg.chunk_duration, chunk_cfg.chunk_overlap,
        step_samples as f64 / sample_rate as f64
    );

    let mut all_segments: Vec<TranscriptSegment> = Vec::new();

    for chunk_idx in 0..total_chunks {
        let sample_start = chunk_idx * step_samples;
        let sample_end = (sample_start + chunk_samples).min(total_samples);
        let chunk_offset = sample_start as f64 / sample_rate as f64;
        let effective_end = chunk_offset + chunk_cfg.chunk_duration;

        let chunk = &all_samples[sample_start..sample_end];

        let stream = recognizer.create_stream();
        stream.accept_waveform(sample_rate as i32, chunk);
        recognizer.decode(&stream);

        if let Some(result) = stream.get_result() {
            let text = result.text.trim().to_string();
            if text.is_empty() {
                continue;
            }

            if let Some(timestamps) = &result.timestamps {
                if timestamps.len() >= 2 && !result.tokens.is_empty() {
                    let ts: Vec<f64> = timestamps.iter().map(|&t| t as f64).collect();
                    let segs = build_segments_from_timestamps(&result.tokens, &ts, chunk_offset);

                    for seg in &segs {
                        let mid = (seg.start + seg.end) / 2.0;
                        if mid < effective_end || chunk_idx == total_chunks - 1 {
                            all_segments.push(seg.clone());
                        }
                    }

                    merge_short_segments(&mut all_segments);
                } else {
                    all_segments.push(TranscriptSegment {
                        start: chunk_offset,
                        end: chunk_offset + (sample_end - sample_start) as f64 / sample_rate as f64,
                        text,
                        confidence: 1.0,
                    });
                }
            } else {
                all_segments.push(TranscriptSegment {
                    start: chunk_offset,
                    end: chunk_offset + (sample_end - sample_start) as f64 / sample_rate as f64,
                    text,
                    confidence: 1.0,
                });
            }
        }

        progress.stage_progress(chunk_idx as u64 + 1, total_chunks as u64);
    }

    all_segments.sort_by(|a, b| a.start.partial_cmp(&b.start).unwrap());
    dedup_overlaps(&mut all_segments);

    Ok(all_segments)
}

fn dedup_overlaps(segments: &mut Vec<TranscriptSegment>) {
    let mut i = 0;
    while i + 1 < segments.len() {
        let a = &segments[i];
        let b = &segments[i + 1];

        if b.start < a.end {
            let a_text = a.text.trim();
            let b_text = b.text.trim();
            let overlap = a.end.min(b.end) - b.start.max(a.start);
            let dur_a = a.end - a.start;
            let dur_b = b.end - b.start;

            let same_prefix = a_text.chars().take(3).collect::<String>()
                == b_text.chars().take(3).collect::<String>();

            if (overlap > dur_a * 0.6 || overlap > dur_b * 0.6) && same_prefix {
                if a.start > b.start {
                    segments.remove(i);
                } else {
                    segments.remove(i + 1);
                }
                continue;
            }
        }
        i += 1;
    }
}

fn build_segments_from_timestamps(
    tokens: &[String],
    timestamps: &[f64],
    offset: f64,
) -> Vec<TranscriptSegment> {
    let sentence_enders = ['。', '！', '？', '.', '!', '?'];
    let mut segments: Vec<TranscriptSegment> = Vec::new();
    let mut buf_text = String::new();
    let mut seg_start = timestamps[0];

    for i in 0..tokens.len() {
        let tok = &tokens[i];
        let ts = timestamps[i];

        if i > 0 {
            let prev_ts = timestamps[i - 1];
            if ts - prev_ts > 0.4 && !buf_text.is_empty() {
                let end = (ts + prev_ts) / 2.0;
                segments.push(TranscriptSegment {
                    start: offset + seg_start,
                    end: offset + end,
                    text: buf_text.trim().to_string(),
                    confidence: 1.0,
                });
                buf_text.clear();
                seg_start = ts;
            }
        }

        buf_text.push_str(tok);

        if let Some(ch) = tok.chars().next() {
            if sentence_enders.contains(&ch) {
                let end = if i + 1 < timestamps.len() {
                    (timestamps[i + 1] + ts) / 2.0
                } else {
                    ts + 0.3
                };
                segments.push(TranscriptSegment {
                    start: offset + seg_start,
                    end: offset + end,
                    text: buf_text.trim().to_string(),
                    confidence: 1.0,
                });
                buf_text.clear();
                if i + 1 < timestamps.len() {
                    seg_start = timestamps[i + 1];
                }
                continue;
            }
        }

        if !buf_text.is_empty() && ts - seg_start > 15.0 {
            let end = if i + 1 < timestamps.len() {
                (timestamps[i + 1] + ts) / 2.0
            } else {
                ts + 0.3
            };
            segments.push(TranscriptSegment {
                start: offset + seg_start,
                end: offset + end,
                text: buf_text.trim().to_string(),
                confidence: 1.0,
            });
            buf_text.clear();
            if i + 1 < timestamps.len() {
                seg_start = timestamps[i + 1];
            }
        }
    }

    if !buf_text.is_empty() {
        let last_ts = timestamps[timestamps.len() - 1];
        let dur = audio_duration_estimate(timestamps);
        let end = last_ts + dur.max(0.3);
        segments.push(TranscriptSegment {
            start: offset + seg_start,
            end: offset + end,
            text: buf_text.trim().to_string(),
            confidence: 1.0,
        });
    }

    segments
}

fn merge_short_segments(segments: &mut Vec<TranscriptSegment>) {
    segments.retain(|s| {
        let text = s.text.trim();
        if text.is_empty() || text == "\u{FFFD}" {
            return false;
        }
        text.chars().all(|ch| {
            ch.is_ascii()
                || ('\u{4E00}'..='\u{9FFF}').contains(&ch)
                || ('\u{3000}'..='\u{303F}').contains(&ch)
                || ('\u{FF00}'..='\u{FFEF}').contains(&ch)
        })
    });

    let mut i = 0;
    while i + 1 < segments.len() {
        let text = segments[i].text.trim();
        let dur = segments[i].end - segments[i].start;
        let char_count = text.chars().count();

        let too_short = (char_count == 1 && dur < 2.0)
            || (char_count < 3 && dur < 0.5);

        if too_short {
            let next = segments.remove(i + 1);
            let cur = &mut segments[i];
            cur.text = format!("{}{}", cur.text.trim(), next.text.trim());
            cur.end = next.end;
            continue;
        }
        i += 1;
    }
}

fn audio_duration_estimate(timestamps: &[f64]) -> f64 {
    if timestamps.len() < 2 {
        return 0.5;
    }
    let gaps: Vec<f64> = timestamps.windows(2).map(|w| w[1] - w[0]).collect();
    let median = {
        let mut sorted = gaps.clone();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
        sorted[sorted.len() / 2]
    };
    median
}

pub fn ensure_ready(cfg: &AsrConfig) -> Result<()> {
    let mdir = model_dir(cfg);
    let _model = find_model(&mdir)?;
    if !mdir.join("tokens.txt").exists() {
        bail!("模型目录缺少 tokens.txt: {}", mdir.display());
    }

    tracing::info!(
        "本地 ASR 就绪: 模型目录: {}",
        mdir.display()
    );
    Ok(())
}
