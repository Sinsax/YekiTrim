use std::path::Path;
use std::sync::Arc;

use async_trait::async_trait;
use anyhow::Result;
use tokio::sync::Semaphore;

use crate::config::Config;
use crate::pipeline::{Stage, STAGE_TRANSCRIBE};
use crate::types::{PipelineContext, Timestamp, Transcript, TranscriptSegment};

mod chunk;
mod cloud_asr;
mod local;

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
                let fallback = ctx.output_dir.join("audio_16khz.wav");
                let from_cfg = ctx.config.audio_file.as_ref()
                    .filter(|p| !p.is_empty())
                    .map(std::path::PathBuf::from);
                let path = from_cfg
                    .filter(|p| p.exists())
                    .or_else(|| {
                        if fallback.exists() { Some(fallback.clone()) } else { None }
                    });
                match path {
                    Some(p) => {
                        tracing::info!("使用已有的音频文件: {}", p.display());
                        p
                    }
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
            let srt_path = ctx.output_dir.join(format!("{}.srt", ctx.input_stem));
            write_srt(&transcript, &srt_path)?;
            let json_path = ctx.output_dir.join(format!("{}_transcript.json", ctx.input_stem));
            save_transcript_json(&transcript, &json_path)?;
            ctx.transcript = Some(transcript);
            return Ok(());
        }

        local::ensure_ready(&ctx.config.asr)?;

        let chunk_duration = ctx.config.asr.chunk_duration;
        let chunk_overlap = ctx.config.asr.chunk_overlap;
        let temp_dir = std::env::temp_dir().join("vtrim_asr");

        let chunks = chunk::split_fixed_with_overlap(
            &audio_path,
            total_duration,
            chunk_duration + chunk_overlap,
            chunk_overlap,
            &temp_dir,
        )?;
        tracing::info!(
            "音频分割为 {} 个 {:.0}s 分块 (+{:.0}s 重叠)",
            chunks.len(),
            chunk_duration,
            chunk_overlap
        );

        let total = chunks.len();
        let cpu_count = std::thread::available_parallelism()
            .map(|n| n.get()).unwrap_or(4);
        let max_parallel = {
            let cfg = ctx.config.asr.num_threads;
            if cfg == 0 { ((cpu_count + 3) / 4).min(4) } else { (cfg as usize).min(cpu_count).max(1) }
        };
        tracing::info!("并行转写: {} 线程", max_parallel);

        ctx.progress.stage_progress(0, total as u64);
        let sem = Arc::new(Semaphore::new(max_parallel));
        let config = Arc::new(ctx.config.clone());

        let mut set: tokio::task::JoinSet<(usize, f64, Result<Vec<TranscriptSegment>>)> = tokio::task::JoinSet::new();
        let mut all_segments: Vec<TranscriptSegment> = Vec::new();
        let mut submitted = 0usize;
        let mut completed = 0usize;

        // 先提交 max_parallel 个
        while submitted < max_parallel.min(total) {
            let permit = sem.clone().acquire_owned().await?;
            let cfg = config.clone();
            let path = chunks[submitted].path.clone();
            let offset = chunks[submitted].offset;
            let idx = submitted;
            set.spawn_blocking(move || {
                let _permit = permit;
                (idx, offset, local::transcribe(&path, &cfg.asr, offset))
            });
            submitted += 1;
        }

        // 循环：完成一个 → 处理结果 → 提下一个
        while completed < total {
            if let Some(res) = set.join_next().await {
                match res {
                    Ok((i, offset, Ok(segs))) => {
                        for seg in segs {
                            if i < total - 1 {
                                let effective_end = offset + chunk_duration;
                                let mid = (seg.start + seg.end) / 2.0;
                                if mid >= effective_end { continue; }
                            }
                            all_segments.push(seg);
                        }
                    }
                    Ok((i, _, Err(e))) => {
                        tracing::warn!("分块 {}/{} 转写失败: {}", i + 1, total, e);
                    }
                    Err(e) => {
                        tracing::warn!("并行任务失败: {}", e);
                    }
                }
                completed += 1;
                ctx.progress.stage_progress(completed as u64, total as u64);
            }
            if submitted < total {
                let permit = sem.clone().acquire_owned().await?;
                let cfg = config.clone();
                let path = chunks[submitted].path.clone();
                let offset = chunks[submitted].offset;
                let idx = submitted;
                set.spawn_blocking(move || {
                    let _permit = permit;
                    (idx, offset, local::transcribe(&path, &cfg.asr, offset))
                });
                submitted += 1;
            }
        }

        all_segments.sort_by(|a, b| a.start.partial_cmp(&b.start).unwrap());

        dedup_overlaps(&mut all_segments);
        post_process_segments(&mut all_segments);

        if all_segments.is_empty() {
            anyhow::bail!("转写结果为空，音频可能无有效语音");
        }

        chunk::cleanup(&temp_dir);

        tracing::info!("转写完成: {} 个片段", all_segments.len());

        let transcript = Transcript {
            segments: all_segments,
            language: "zh".into(),
        };

        let srt_path = ctx.output_dir.join(format!("{}.srt", ctx.input_stem));
        write_srt(&transcript, &srt_path)?;
        let json_path = ctx.output_dir.join(format!("{}_transcript.json", ctx.input_stem));
        save_transcript_json(&transcript, &json_path)?;
        tracing::info!("字幕已生成: {}", srt_path.display());
        tracing::info!("转录 JSON 已保存: {}", json_path.display());

        ctx.transcript = Some(transcript);

        tracing::info!("语音转文字完成");
        Ok(())
    }
}

/// Post-process: remove garbage segments and merge short/noisy ones.
fn post_process_segments(segments: &mut Vec<TranscriptSegment>) {
    if segments.is_empty() {
        return;
    }

    // 1. Remove segments with < 2 meaningful characters
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

    // 2. Merge single-char segments into adjacent ones, or tiny segments close together
    let mut i = 0;
    while i + 1 < segments.len() {
        let cur = &segments[i];
        let nxt = &segments[i + 1];
        let gap = nxt.start - cur.end;
        let cur_len = cur.text.trim().chars().count();

        // Single-char or very short segment: merge into next
        if cur_len <= 2 && gap < 2.0 {
            let removed = segments.remove(i);
            segments[i].start = removed.start;
            segments[i].text = format!("{}{}", removed.text.trim(), segments[i].text.trim());
            continue;
        }

        // Next segment is very short and close: merge into current
        let nxt_len = nxt.text.trim().chars().count();
        if nxt_len <= 2 && gap < 1.0 {
            let removed = segments.remove(i + 1);
            segments[i].end = removed.end;
            segments[i].text = format!("{}{}", segments[i].text.trim(), removed.text.trim());
            continue;
        }

        // Both segments are short and very close: merge
        if cur_len <= 4 && nxt_len <= 4 && gap < 0.5 {
            let removed = segments.remove(i + 1);
            segments[i].end = removed.end;
            segments[i].text = format!("{}{}", segments[i].text.trim(), removed.text.trim());
            continue;
        }

        i += 1;
    }
}

/// Remove duplicate segments that overlap in time with similar text content.
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

fn wav_duration_secs(path: &Path) -> Result<f64> {
    let meta = std::fs::metadata(path)?;
    let data_size = meta.len().saturating_sub(44);
    Ok(data_size as f64 / 32000.0)
}

fn save_transcript_json(transcript: &Transcript, path: &Path) -> Result<()> {
    let json = serde_json::to_string_pretty(transcript)?;
    std::fs::write(path, json)?;
    Ok(())
}

/// 从 SRT 字符串解析 Transcript
pub fn parse_srt(content: &str) -> Result<Transcript> {
    let mut segments = Vec::new();
    let mut lines = content.lines().peekable();

    while lines.peek().is_some() {
        // skip index line
        let _ = lines.next();
        // timestamp line
        let time_line = match lines.next() {
            Some(l) => l,
            None => break,
        };
        let (start_str, end_str) = time_line
            .split_once(" --> ")
            .ok_or_else(|| anyhow::anyhow!("SRT 时间格式无效: {}", time_line))?;
        let start = parse_srt_time(start_str.trim())?;
        let end = parse_srt_time(end_str.trim())?;
        // text lines until blank
        let mut text = String::new();
        while let Some(l) = lines.next() {
            if l.trim().is_empty() {
                break;
            }
            if !text.is_empty() {
                text.push(' ');
            }
            text.push_str(l.trim());
        }
        if !text.is_empty() {
            segments.push(TranscriptSegment {
                start,
                end,
                text,
                confidence: 1.0,
            });
        }
    }

    Ok(Transcript {
        segments,
        language: "zh".into(),
    })
}

fn parse_srt_time(s: &str) -> Result<f64> {
    // 00:00:01,234 or 00:00:01.234
    let s = s.replace(',', ".");
    let parts: Vec<&str> = s.split(':').collect();
    if parts.len() == 3 {
        let h: f64 = parts[0].parse().map_err(|e| anyhow::anyhow!("SRT 时间解析失败: {}", e))?;
        let m: f64 = parts[1].parse()?;
        let sec: f64 = parts[2].parse()?;
        Ok(h * 3600.0 + m * 60.0 + sec)
    } else {
        anyhow::bail!("SRT 时间格式异常: {}", s)
    }
}

pub fn write_srt(transcript: &Transcript, path: &Path) -> Result<()> {
    use std::io::Write;

    let mut f = std::fs::File::create(path)?;
    for (i, seg) in transcript.segments.iter().enumerate() {
        let start = Timestamp::from_seconds(seg.start);
        let end = Timestamp::from_seconds(seg.end);
        writeln!(f, "{}", i + 1)?;
        writeln!(
            f,
            "{} --> {}",
            start.to_srt_format(),
            end.to_srt_format()
        )?;
        writeln!(f, "{}", seg.text)?;
        writeln!(f)?;
    }
    Ok(())
}
