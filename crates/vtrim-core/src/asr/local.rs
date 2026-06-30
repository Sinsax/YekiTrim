use std::fs::File;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use memmap2::Mmap;
use sherpa_onnx::{
    OfflineRecognizer, OfflineRecognizerConfig, OfflineSenseVoiceModelConfig,
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
    config.model_config.provider = resolve_provider(cfg);
    let language = if cfg.language.is_empty() { "auto".to_string() } else { cfg.language.clone() };
    config.model_config.sense_voice = OfflineSenseVoiceModelConfig {
        model: Some(model_path.to_str().unwrap().into()),
        language: Some(language),
        use_itn: true,
    };
    config.model_config.num_threads = 1;
    config.model_config.tokens = Some(tokens_path.to_str().unwrap().into());

    OfflineRecognizer::create(&config)
        .context("创建 OfflineRecognizer 失败，请检查模型文件是否正确")
}

pub struct ChunkConfig {
    pub chunk_duration: f64,
    pub chunk_overlap: f64,
    pub segment_gap: f64,
    pub num_workers: usize,
}

impl Default for ChunkConfig {
    fn default() -> Self {
        Self {
            chunk_duration: 60.0,
            chunk_overlap: 10.0,
            segment_gap: 0.6,
            num_workers: 1,
        }
    }
}

impl From<&AsrConfig> for ChunkConfig {
    fn from(cfg: &AsrConfig) -> Self {
        let num_workers = if cfg.num_workers == 0 {
            std::thread::available_parallelism()
                .map(|n| (n.get() / 4).clamp(1, 4))
                .unwrap_or(1)
        } else {
            let cpus = std::thread::available_parallelism()
                .map(|n| n.get()).unwrap_or(1);
            (cfg.num_workers as usize).min(cpus)
        };
        Self {
            chunk_duration: cfg.chunk_duration,
            chunk_overlap: cfg.chunk_overlap,
            segment_gap: cfg.segment_gap,
            num_workers,
        }
    }
}

/// 内存映射的 WAV 文件读取器 — 避免将整个 PCM 数据转换为 f32 加载到内存。
/// 解析标准 PCM WAV 头，从内存映射区域按需读取并转换样本。
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

        // 遍历 RIFF sub-chunks，查找 "data" 块
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
            // RIFF 要求 sub-chunk 按 WORD 对齐
            if chunk_size % 2 != 0 {
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

    /// 将指定范围的 PCM s16 样本转换为 f32
    fn samples_f32(&self, start_sample: usize, count: usize) -> Vec<f32> {
        let byte_start = self.data_start + start_sample * 2;
        let byte_end = (byte_start + count * 2).min(self.mmap.len());
        self.mmap[byte_start..byte_end]
            .chunks_exact(2)
            .map(|b| i16::from_le_bytes([b[0], b[1]]) as f32 / 32768.0)
            .collect()
    }
}

pub fn transcribe_chunked(
    recognizer: &OfflineRecognizer,
    audio_path: &Path,
    chunk_cfg: &ChunkConfig,
    progress: &dyn ProgressReporter,
) -> Result<Vec<TranscriptSegment>> {
    let mapped = MappedWave::open(audio_path)?;
    let sample_rate = mapped.sample_rate;
    let total_samples = mapped.total_samples;

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

    let num_workers = chunk_cfg.num_workers.min(total_chunks);

    let mut all_chunk_results: Vec<Vec<TranscriptSegment>> = Vec::new();

    for batch_start in (0..total_chunks).step_by(num_workers) {
        let batch_end = (batch_start + num_workers).min(total_chunks);

        let batch_segments: Vec<Vec<TranscriptSegment>> = std::thread::scope(|scope| {
            let mut handles = Vec::with_capacity(batch_end - batch_start);

            for chunk_idx in batch_start..batch_end {
                let sample_start = chunk_idx * step_samples;
                let sample_end = (sample_start + chunk_samples).min(total_samples);
                let chunk_offset = sample_start as f64 / sample_rate as f64;
                let overlap_mid = chunk_offset + chunk_cfg.chunk_duration - chunk_cfg.chunk_overlap / 2.0;
                let is_last = chunk_idx == total_chunks - 1;

                // 仅从 mmap 转换当前块所需的 PCM 样本（s16→f32）
                let chunk_f32 = mapped.samples_f32(sample_start, sample_end - sample_start);

                handles.push(scope.spawn(move || {
                    let stream = recognizer.create_stream();
                    stream.accept_waveform(sample_rate as i32, &chunk_f32);
                    recognizer.decode(&stream);

                    let mut segments: Vec<TranscriptSegment> = Vec::new();

                    if let Some(result) = stream.get_result() {
                        let text = result.text.trim().to_string();
                        if !text.is_empty() {
                            if let Some(timestamps) = &result.timestamps {
                                if timestamps.len() >= 2 && !result.tokens.is_empty() {
                                    let ts: Vec<f64> = timestamps.iter().map(|&t| t as f64).collect();
                                    let segs = build_segments_from_timestamps(&result.tokens, &ts, chunk_offset, chunk_cfg.segment_gap);

                                    for seg in &segs {
                                        let mid = (seg.start + seg.end) / 2.0;
                                        if mid < overlap_mid || is_last {
                                            segments.push(seg.clone());
                                        }
                                    }
                                } else {
                                    segments.push(TranscriptSegment {
                                        start: chunk_offset,
                                        end: chunk_offset + (sample_end - sample_start) as f64 / sample_rate as f64,
                                        text,
                                        confidence: 1.0,
                                    });
                                }
                            } else {
                                segments.push(TranscriptSegment {
                                    start: chunk_offset,
                                    end: chunk_offset + (sample_end - sample_start) as f64 / sample_rate as f64,
                                    text,
                                    confidence: 1.0,
                                });
                            }
                        }
                    }

                    segments
                }));
            }

            handles.into_iter().map(|h| h.join().unwrap()).collect()
        });

        progress.stage_progress(batch_end as u64, total_chunks as u64);
        all_chunk_results.extend(batch_segments);
    }

    let mut all_segments: Vec<TranscriptSegment> = all_chunk_results.into_iter().flatten().collect();
    all_segments.sort_by(|a, b| a.start.partial_cmp(&b.start).unwrap());
    dedup_overlaps(&mut all_segments);
    all_segments.retain(|s| s.text.trim() != "\u{FFFD}");

    // 合并过短且紧贴相邻段的 segment
    let mut short_merged: Vec<TranscriptSegment> = Vec::with_capacity(all_segments.len());
    let mut i = 0;
    while i < all_segments.len() {
        let dur = all_segments[i].end - all_segments[i].start;
        if dur < 1.0 && i + 1 < all_segments.len() {
            let gap_to_next = all_segments[i + 1].start - all_segments[i].end;
            let gap_to_prev = short_merged.last()
                .map(|last| all_segments[i].start - last.end)
                .unwrap_or(f64::MAX);

            if gap_to_next < 0.5 || gap_to_prev < 0.5 {
                if gap_to_prev <= gap_to_next {
                    let last = short_merged.last_mut().unwrap();
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
        short_merged.push(all_segments[i].clone());
        i += 1;
    }
    all_segments = short_merged;

    Ok(all_segments)
}

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
