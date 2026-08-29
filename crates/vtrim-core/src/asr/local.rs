use std::fs::File;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use memmap2::Mmap;
use sherpa_onnx::{HomophoneReplacerConfig, OfflineRecognizer, OfflineRecognizerConfig, OfflineSenseVoiceModelConfig};

use crate::config::AsrConfig;
use crate::progress::ProgressReporter;
use crate::types::{AsrMetadata, TranscriptSegment};

fn model_dir(cfg: &AsrConfig) -> PathBuf {
    if !cfg.local_model_dir.is_empty() {
        return PathBuf::from(&cfg.local_model_dir);
    }
    crate::path::vtrim_home().join("models").join(&cfg.local_model)
}

fn find_model(model_dir: &Path, model: &str) -> Result<PathBuf> {
    let candidates: &[&str] = match model {
        "firered-zh" => &["model.int8.onnx", "model.onnx"],
        "qwen3-asr" => &["encoder.int8.onnx", "encoder.onnx"],
        _ => &["model.int8.onnx", "model.onnx", "sense-voice.onnx"],
    };
    for name in candidates {
        let p = model_dir.join(name);
        if p.exists() {
            return Ok(p);
        }
    }
    bail!(
        "模型目录 {} 中未找到模型文件 (需要: {})",
        model_dir.display(),
        candidates.join(" / ")
    );
}

fn require_file(dir: &Path, name: &str) -> Result<PathBuf> {
    let p = dir.join(name);
    if p.exists() {
        Ok(p)
    } else {
        bail!("模型目录 {} 缺少必需文件: {}", dir.display(), name)
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

    let mut config = OfflineRecognizerConfig::default();
    config.model_config.provider = resolve_provider(cfg);
    config.model_config.num_threads = 1;

    let model_path = find_model(&mdir, "sensevoice")?;
    let tokens_path = require_file(&mdir, "tokens.txt")?;
    let language = if cfg.language.is_empty() { "auto".to_string() } else { cfg.language.clone() };
    config.model_config.sense_voice = OfflineSenseVoiceModelConfig {
        model: Some(model_path.to_string_lossy().into()),
        language: Some(language),
        use_itn: true,
    };
    config.model_config.tokens = Some(tokens_path.to_string_lossy().into());
    tracing::info!("ASR 模型: SenseVoice (int8)");

    let lexicon = mdir.join("lexicon.txt");
    let rule_fst = mdir.join("replace.fst");
    if lexicon.exists() && rule_fst.exists() {
        config.hr = HomophoneReplacerConfig {
            lexicon: Some(lexicon.to_string_lossy().into()),
            rule_fsts: Some(rule_fst.to_string_lossy().into()),
        };
        tracing::info!("同音纠错已启用 (lexicon.txt + replace.fst)");
    } else {
        tracing::info!("同音纠错未启用（未找到 lexicon.txt / replace.fst）");
    }

    OfflineRecognizer::create(&config)
        .context("创建 OfflineRecognizer 失败，请检查模型文件是否正确")
}

/// 转写结果 + 聚合后的 ASR 质量元数据。
pub struct TranscribeOutput {
    pub segments: Vec<TranscriptSegment>,
    pub metadata: AsrMetadata,
}

/// 内存映射的 WAV 文件读取器 — 避免将整个 PCM 数据转换为 f32 加载到内存。
struct MappedWave {
    mmap: Mmap,
    sample_rate: usize,
    data_start: usize,
    total_samples: usize,
}

impl MappedWave {
    fn open(path: &Path) -> Result<Self> {
        let file = File::open(path).context("打开音频文件失败")?;
        let mmap = unsafe { Mmap::map(&file) }.context("内存映射音频文件失败")?;

        if mmap.len() < 44 {
            bail!("WAV 文件损坏或过小");
        }
        if &mmap[0..4] != b"RIFF" || &mmap[8..12] != b"WAVE" {
            bail!("文件不是有效的 WAV 格式");
        }

        let channels = u16::from_le_bytes([mmap[22], mmap[23]]) as usize;
        let sample_rate = u32::from_le_bytes([mmap[24], mmap[25], mmap[26], mmap[27]]) as usize;
        let bits_per_sample = u16::from_le_bytes([mmap[34], mmap[35]]) as usize;

        if bits_per_sample != 16 {
            bail!("仅支持 16-bit PCM WAV (当前: {} bit)", bits_per_sample);
        }
        if channels != 1 {
            bail!("仅支持单声道 WAV (当前: {} 声道)", channels);
        }

        let mut data_start = 12;
        let mut data_size = 0;
        while data_start + 8 <= mmap.len() {
            let chunk_id = &mmap[data_start..data_start + 4];
            let chunk_size = u32::from_le_bytes([
                mmap[data_start + 4],
                mmap[data_start + 5],
                mmap[data_start + 6],
                mmap[data_start + 7],
            ]) as usize;
            if chunk_id == b"data" {
                data_start += 8;
                data_size = chunk_size;
                break;
            }
            data_start += 8 + chunk_size;
            if !chunk_size.is_multiple_of(2) {
                data_start += 1;
            }
        }

        if data_size == 0 || data_start >= mmap.len() {
            bail!("WAV 文件中未找到音频数据");
        }

        let total_samples = data_size / 2;

        Ok(Self {
            mmap,
            sample_rate,
            data_start,
            total_samples,
        })
    }

    fn samples_f32(&self, start_sample: usize, count: usize) -> Vec<f32> {
        let byte_start = self.data_start + start_sample * 2;
        let byte_end = (byte_start + count * 2).min(self.mmap.len());
        self.mmap[byte_start..byte_end]
            .chunks_exact(2)
            .map(|b| i16::from_le_bytes([b[0], b[1]]) as f32 / 32768.0)
            .collect()
    }
}

/// 主流程：sensevoice 单模型并行转写。
pub fn transcribe_sensevoice(
    cfg: &AsrConfig,
    audio_path: &Path,
    progress: &dyn ProgressReporter,
) -> Result<TranscribeOutput> {
    let mapped = MappedWave::open(audio_path)?;
    let recognizer = create_recognizer(cfg)?;

    let mut segments = scan_sensevoice_segments(
        &mapped,
        &recognizer,
        cfg.chunk_duration,
        cfg.chunk_overlap,
        cfg.segment_gap,
        cfg.num_workers,
        progress,
    )?;

    segments.sort_by(|a, b| a.start.partial_cmp(&b.start).unwrap());
    dedup_overlaps(&mut segments);
    segments = cleanup_segments(segments);

    let metadata = AsrMetadata {
        anchors: 0,
        anchor_coverage: 0.0,
        max_interp_span: 0,
        fallback_reason: Some("sensevoice_single".into()),
        model: "sensevoice".into(),
    };

    Ok(TranscribeOutput {
        segments,
        metadata,
    })
}

/// 用 sensevoice 并行扫描音频，返回带文本和时间戳的字幕段。
fn scan_sensevoice_segments(
    mapped: &MappedWave,
    recognizer: &OfflineRecognizer,
    chunk_duration: f64,
    chunk_overlap: f64,
    segment_gap: f64,
    num_workers: u32,
    progress: &dyn ProgressReporter,
) -> Result<Vec<TranscriptSegment>> {
    let sample_rate = mapped.sample_rate;
    let total_samples = mapped.total_samples;

    let chunk_samples = (chunk_duration * sample_rate as f64) as usize;
    let overlap_samples = (chunk_overlap * sample_rate as f64) as usize;
    let step_samples = chunk_samples.saturating_sub(overlap_samples);

    if step_samples == 0 {
        bail!("chunk_duration ({}) 必须大于 chunk_overlap ({})", chunk_duration, chunk_overlap);
    }

    let total_chunks = total_samples.div_ceil(step_samples);
    tracing::info!(
        "sensevoice 扫描: 音频分 {total_chunks} 块 (每块 {:.0}s, 重叠 {:.0}s, 步进 {:.1}s)",
        chunk_duration, chunk_overlap, step_samples as f64 / sample_rate as f64
    );

    let workers = if num_workers == 0 {
        std::thread::available_parallelism()
            .map(|n| (n.get() / 4).clamp(1, 4))
            .unwrap_or(1)
    } else {
        (num_workers as usize).min(std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1))
    }
    .min(total_chunks)
    .max(1);

    let mut all_chunk_results: Vec<Vec<TranscriptSegment>> = Vec::with_capacity(total_chunks);

    for batch_start in (0..total_chunks).step_by(workers) {
        let batch_end = (batch_start + workers).min(total_chunks);
        let spawned: Vec<Vec<TranscriptSegment>> = std::thread::scope(|scope| {
            let mut handles = Vec::with_capacity(batch_end - batch_start);

            for chunk_idx in batch_start..batch_end {
                let sample_start = chunk_idx * step_samples;
                let sample_end = (sample_start + chunk_samples).min(total_samples);
                let chunk_offset = sample_start as f64 / sample_rate as f64;
                let overlap_mid = chunk_offset + chunk_duration - chunk_overlap / 2.0;
                let is_last = chunk_idx == total_chunks - 1;

                let chunk_f32 = mapped.samples_f32(sample_start, sample_end - sample_start);

                handles.push(scope.spawn(move || {
                    let mut out = Vec::new();
                    let stream = recognizer.create_stream();
                    stream.accept_waveform(sample_rate as i32, &chunk_f32);
                    recognizer.decode(&stream);

                    if let Some(result) = stream.get_result() {
                        if let Some(timestamps) = &result.timestamps {
                            if timestamps.len() >= 2 && !result.tokens.is_empty() {
                                let ts: Vec<f64> = timestamps.iter().map(|&t| t as f64).collect();
                                let segs = build_segments_from_timestamps(
                                    &result.tokens,
                                    &ts,
                                    chunk_offset,
                                    segment_gap,
                                );
                                for seg in segs {
                                    let mid = (seg.start + seg.end) / 2.0;
                                    if mid < overlap_mid || is_last {
                                        out.push(seg);
                                    }
                                }
                            }
                        }
                    }
                    out
                }));
            }

            handles.into_iter().map(|h| h.join().unwrap()).collect()
        });

        for segs in spawned {
            all_chunk_results.push(segs);
        }

        progress.stage_progress(batch_end as u64, total_chunks as u64);
    }

    Ok(all_chunk_results.into_iter().flatten().collect())
}

/// 对最终字幕段做短段合并和噪声清理。
fn cleanup_segments(all_segments: Vec<TranscriptSegment>) -> Vec<TranscriptSegment> {
    let mut all_segments = all_segments;
    all_segments.retain(|s| s.text.trim() != "\u{FFFD}");
    if all_segments.is_empty() {
        return all_segments;
    }

    let mut merged: Vec<TranscriptSegment> = Vec::with_capacity(all_segments.len());
    let mut i = 0;
    while i < all_segments.len() {
        let dur = all_segments[i].end - all_segments[i].start;
        let text_len = all_segments[i].text.trim().chars().count();
        let should_merge = dur < 1.0 || (dur < 3.0 && text_len <= 5);

        if should_merge && i + 1 < all_segments.len() {
            let gap_to_next = all_segments[i + 1].start - all_segments[i].end;
            let gap_to_prev = merged.last()
                .map(|last| all_segments[i].start - last.end)
                .unwrap_or(f64::MAX);

            if gap_to_next < 1.0 || gap_to_prev < 1.0 {
                if gap_to_prev <= gap_to_next {
                    let last = merged.last_mut().unwrap();
                    last.text = format!("{}{}", last.text.trim(), all_segments[i].text.trim());
                    last.end = all_segments[i].end;
                } else {
                    all_segments[i + 1].start = all_segments[i].start;
                    all_segments[i + 1].text =
                        format!("{}{}", all_segments[i].text.trim(), all_segments[i + 1].text.trim());
                }
                i += 1;
                continue;
            }
        }
        merged.push(all_segments[i].clone());
        i += 1;
    }

    let start_noise = merged
        .iter()
        .take_while(|s| s.start < 300.0 && s.text.trim().chars().count() <= 2)
        .count();

    let mut cleaned: Vec<TranscriptSegment> = Vec::with_capacity(merged.len());
    for (idx, seg) in merged.iter().enumerate().skip(start_noise) {
        let text = seg.text.trim();
        if text.chars().count() <= 1 && (seg.end - seg.start) > 5.0 {
            continue;
        }
        if text.chars().count() <= 2 && (seg.end - seg.start) < 5.0 {
            let gap_to_prev = if idx > 0 {
                seg.start - merged[idx - 1].end
            } else {
                f64::MAX
            };
            let gap_to_next = merged.get(idx + 1)
                .map(|n| n.start - seg.end)
                .unwrap_or(f64::MAX);
            if gap_to_prev > 3.0 && gap_to_next > 3.0 {
                continue;
            }
        }
        cleaned.push(seg.clone());
    }

    tracing::info!(
        "片段后处理: {} → {}{}",
        all_segments.len(),
        merged.len(),
        if cleaned.len() < merged.len() {
            format!(" (丢弃 {} 个噪声段)", merged.len() - cleaned.len())
        } else {
            String::new()
        },
    );

    cleaned
}

/// 重叠段去重：保留更长文本的段。
fn dedup_overlaps(segments: &mut Vec<TranscriptSegment>) {
    let mut kept: Vec<TranscriptSegment> = Vec::with_capacity(segments.len());
    for seg in segments.drain(..) {
        if let Some(last) = kept.last_mut() {
            if seg.start < last.end {
                let overlap = last.end.min(seg.end) - seg.start.max(last.start);
                let dur_a = last.end - last.start;
                let dur_b = seg.end - seg.start;

                if overlap > dur_a * 0.6 || overlap > dur_b * 0.6 {
                    if seg.text.len() > last.text.len() {
                        *last = seg;
                    }
                    continue;
                }
            }
        }
        kept.push(seg);
    }
    *segments = kept;
}

/// 从 sensevoice token 时间戳构建字幕段。
fn build_segments_from_timestamps(
    tokens: &[String],
    timestamps: &[f64],
    offset: f64,
    segment_gap: f64,
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
            if ts - prev_ts > segment_gap && !buf_text.is_empty() {
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

fn audio_duration_estimate(timestamps: &[f64]) -> f64 {
    if timestamps.len() < 2 {
        return 0.5;
    }
    let gaps: Vec<f64> = timestamps.windows(2).map(|w| w[1] - w[0]).collect();
    let mut sorted = gaps.clone();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
    sorted[sorted.len() / 2]
}

pub fn ensure_ready(cfg: &AsrConfig) -> Result<()> {
    let mdir = model_dir(cfg);
    find_model(&mdir, "sensevoice")?;
    require_file(&mdir, "tokens.txt")?;

    tracing::info!(
        "本地 ASR 就绪: model=sensevoice, 模型目录: {}",
        mdir.display()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_segments_splits_on_sentence_end() {
        let tokens = vec!["你".into(), "好".into(), "。".into(), "世".into(), "界".into(), "！".into()];
        let times = vec![0.0, 0.2, 0.4, 0.6, 0.8, 1.0];
        let segs = build_segments_from_timestamps(&tokens, &times, 0.0, 1.0);
        assert_eq!(segs.len(), 2);
        assert_eq!(segs[0].text, "你好。");
        assert_eq!(segs[1].text, "世界！");
    }

    #[test]
    fn cleanup_removes_isolated_short_noise() {
        let segs = vec![
            TranscriptSegment { start: 0.0, end: 10.0, text: "嗯".into(), confidence: 1.0 },
            TranscriptSegment { start: 400.0, end: 402.0, text: "你好".into(), confidence: 1.0 },
        ];
        let cleaned = cleanup_segments(segs);
        assert!(cleaned.is_empty(), "孤立短噪声应被清理");
    }
}
