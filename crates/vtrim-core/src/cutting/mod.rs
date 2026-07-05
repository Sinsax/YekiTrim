use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

use async_trait::async_trait;
use anyhow::{Context, Result};

use crate::config::Config;
use crate::danmaku::DanmakuEntry;
use crate::pipeline::{Stage, STAGE_CUT};
use crate::types::{PipelineContext, Segment};

pub struct Cutter;

#[async_trait]
impl Stage for Cutter {
    fn name(&self) -> &'static str {
        "Cutter"
    }

    fn stage_number(&self) -> u32 {
        STAGE_CUT
    }

    fn is_enabled(&self, _config: &Config) -> bool {
        true
    }

    async fn run(&self, ctx: &mut PipelineContext) -> Result<()> {
        if ctx.events.is_empty() {
            anyhow::bail!("没有事件可切割，请先运行 detect-events 阶段");
        }

        let video = &ctx.input_path;
        if !video.exists() {
            anyhow::bail!("视频文件不存在: {}", video.display());
        }

        let clips_dir = ctx.output_dir.join("clips");

        std::fs::create_dir_all(&clips_dir)?;
        let (enc_name, enc_preset, _) = encoder_config();
        tracing::info!("视频编码: {} (preset={})", enc_name, enc_preset);

        let full_transcript = load_transcript(ctx);

        // Load danmaku entries if available
        let all_danmaku: Vec<DanmakuEntry> = ctx
            .danmaku_path
            .as_ref()
            .filter(|p| p.exists())
            .and_then(|p| crate::danmaku::parse_ass(p).ok())
            .unwrap_or_default();

        let max_concurrent = 3;
        let total_events = ctx.events.len();
        let mut clip_infos: Vec<ClipInfo> = Vec::new();

        for batch_start in (0..total_events).step_by(max_concurrent) {
            let batch_end = (batch_start + max_concurrent).min(total_events);

            let batch_results: Vec<Result<ClipInfo>> = std::thread::scope(|scope| {
                let mut handles = Vec::with_capacity(batch_end - batch_start);

                for event in &ctx.events[batch_start..batch_end] {
                    let start = event.start.to_seconds();
                    let end = event.end.to_seconds();
                    let dir_name = format!("clip-{:02}", event.rank);
                    let rank = event.rank;

                    let v = video.to_path_buf();
                    let cd = clips_dir.clone();
                    let t = event.label.clone();
                    let s = event.summary.clone();
                    let et = event.event_type.clone();
                    let pop = event.popularity;
                    let ts_start = event.start;
                    let ts_end = event.end;
                    let transcript = full_transcript.clone();
                    let danmaku = all_danmaku.clone();

                    handles.push(scope.spawn(move || {
                        let clip_dir = cd.join(&dir_name);
                        std::fs::create_dir_all(&clip_dir)
                            .with_context(|| format!("创建目录失败: {}", clip_dir.display()))?;

                        let vid_path = clip_dir.join("clip.mp4");

                        tracing::debug!("剪切: {} ({:.1}s-{:.1}s)", dir_name, start, end);
                        cut_segment(&v, start, end, &vid_path)?;

                        let sub_path = clip_dir.join("clip.srt");
                        if let Some(ref t2) = transcript {
                            if let Err(e) = generate_clip_subtitle(t2, start, end, &sub_path) {
                                tracing::warn!("字幕生成失败 ({}): {}", dir_name, e);
                            }
                        }

                        if !danmaku.is_empty() {
                            let dm_path = clip_dir.join("clip.dm.srt");
                            if let Err(e) = generate_clip_danmaku(&danmaku, start, end, &dm_path) {
                                tracing::warn!("弹幕生成失败 ({}): {}", dir_name, e);
                            }
                        }

                        let meta_path = clip_dir.join("clip.json");
                        if let Err(e) = write_clip_meta(&meta_path, &t, &et, pop, rank, ts_start, ts_end) {
                            tracing::warn!("写入 clip.json 失败 ({}): {}", dir_name, e);
                        }

                        Ok(ClipInfo {
                            title: t.clone(),
                            summary: s,
                            event_type: et,
                            popularity: pop,
                            rank,
                            start: ts_start,
                            end: ts_end,
                            has_danmaku: !danmaku.is_empty(),
                        })
                    }));
                }

                handles.into_iter().map(|h| h.join().unwrap()).collect()
            });

            for result in batch_results {
                match result {
                    Ok(info) => clip_infos.push(info),
                    Err(e) => tracing::error!("剪切失败: {}", e),
                }
            }

            ctx.progress.stage_progress(batch_end as u64, total_events as u64);
        }

        if clip_infos.is_empty() {
            anyhow::bail!("所有剪切任务均失败");
        }

        let publish = PublishConfig::new(video, &clip_infos);
        let json = serde_json::to_string_pretty(&publish)?;
        let publish_path = ctx.output_dir.join("publish_config.json");
        std::fs::write(&publish_path, &json).context("写入 publish_config.json 失败")?;
        tracing::info!("发布配置已保存: {}", publish_path.display());

        let order_path = clips_dir.join("clips_order.txt");
        let mut order_events: Vec<_> = ctx.events.iter().collect();
        order_events.sort_by(|a, b| b.popularity.partial_cmp(&a.popularity).unwrap());
        let order_lines: Vec<String> = order_events.iter().map(|ev| {
            format!("clip-{:02} / {}", ev.rank, ev.label)
        }).collect();
        std::fs::write(&order_path, order_lines.join("\n"))
            .context("写入 clips_order.txt 失败")?;
        tracing::info!("播放顺序已保存: {}", order_path.display());

        let mut segments = Vec::new();
        for ev in &ctx.events {
            let label = format!("clip-{:02}", ev.rank);
            let clip_dir = clips_dir.join(&label);
            segments.push(Segment {
                index: ev.rank as usize,
                event: ev.clone(),
                output_path: clip_dir.join("clip.mp4"),
                subtitle_path: clip_dir.join("clip.srt"),
            });
        }
        ctx.segments = segments;

        tracing::info!("剪切完成: {}/{} 个片段", clip_infos.len(), ctx.events.len());
        Ok(())
    }
}

fn encoder_config() -> (&'static str, &'static str, &'static str) {
    static CFG: OnceLock<(&'static str, &'static str, &'static str)> = OnceLock::new();
    *CFG.get_or_init(|| {
        let output = Command::new("ffmpeg")
            .args(["-hide_banner", "-encoders"])
            .output();
        let stdout = match output {
            Ok(o) => String::from_utf8_lossy(&o.stdout).to_string(),
            Err(_) => return ("libx264", "ultrafast", "-crf"),
        };

        if stdout.contains("h264_nvenc") {
            return ("h264_nvenc", "p1", "-cq");
        }
        if stdout.contains("h264_qsv") {
            return ("h264_qsv", "veryfast", "-global_quality");
        }
        if stdout.contains("h264_amf") {
            return ("h264_amf", "speed", "-quality");
        }
        ("libx264", "ultrafast", "-crf")
    })
}

fn cut_segment(video: &Path, start: f64, end: f64, output: &Path) -> Result<()> {
    let result = Command::new("ffmpeg")
        .arg("-y")
        .arg("-loglevel")
        .arg("error")
        .arg("-ss")
        .arg(format!("{:.3}", start))
        .arg("-i")
        .arg(video)
        .arg("-t")
        .arg(format!("{:.3}", (end - start).max(0.1)))
        .arg("-c:v")
        .arg(encoder_config().0)
        .arg("-preset")
        .arg(encoder_config().1)
        .arg(encoder_config().2)
        .arg("23")
        .arg("-pix_fmt")
        .arg("yuv420p")
        .arg("-c:a")
        .arg("aac")
        .arg("-movflags")
        .arg("+faststart")
        .arg(output)
        .output()
        .with_context(|| "ffmpeg 执行失败 (请确保 ffmpeg 在 PATH 中)")?;

    if !result.status.success() {
        let stderr = String::from_utf8_lossy(&result.stderr);
        anyhow::bail!("ffmpeg 失败: {}", stderr.trim());
    }
    Ok(())
}

/// 对已输出的视频文件执行静音移除（重编码）。
/// `duration` 为连续静音秒数阈值，超过此长度的静音段将被移除。
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

pub fn detect_silence(input: &Path, duration: f64) -> Result<Vec<(f64, f64)>> {
    let result = Command::new("ffmpeg")
        .arg("-y")
        .arg("-i")
        .arg(input)
        .arg("-af")
        .arg(format!("silencedetect=n=-30dB:d={}", duration))
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

pub fn remove_silence(input: &Path, output: &Path, duration: f64, srt_path: Option<&Path>) -> Result<Vec<(f64, f64)>> {
    let segments = match srt_path {
        Some(srt) if srt.exists() => {
            tracing::info!("使用 SRT 间隔检测静音: {}", srt.display());
            detect_silence_from_srt(srt, duration)?
        }
        _ => {
            tracing::info!("使用音频分析检测静音");
            detect_silence(input, duration)?
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
    keep_ranges.retain(|&(start, end)| end.map_or(true, |e| e > start));

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
    keep_ranges.retain(|&(s, e)| e.map_or(true, |e| e - s > 1.0));

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

fn generate_clip_subtitle(
    transcript: &crate::types::Transcript,
    start: f64,
    end: f64,
    output: &Path,
) -> Result<()> {
    let filtered: Vec<crate::types::TranscriptSegment> = transcript
        .segments
        .iter()
        .filter(|s| s.start >= start && s.end <= end)
        .map(|s| crate::types::TranscriptSegment {
            start: (s.start - start).max(0.0),
            end: (s.end - start).max(0.0),
            text: s.text.clone(),
            confidence: s.confidence,
        })
        .collect();

    if filtered.is_empty() {
        return Ok(());
    }

    let clip_transcript = crate::types::Transcript {
        segments: filtered,
        language: transcript.language.clone(),
    };

    crate::srt::write_srt(&clip_transcript, output)
        .context("写入 clip 字幕失败")?;
    Ok(())
}

fn generate_clip_danmaku(
    entries: &[DanmakuEntry],
    start: f64,
    end: f64,
    output: &Path,
) -> Result<()> {
    let filtered: Vec<&DanmakuEntry> = entries
        .iter()
        .filter(|e| e.start < end && e.end > start)
        .collect();

    if filtered.is_empty() {
        return Ok(());
    }

    let mut file = std::fs::File::create(output)
        .with_context(|| format!("创建弹幕文件失败: {}", output.display()))?;
    use std::io::Write;
    file.write_all(b"\xEF\xBB\xBF")?;

    for (i, entry) in filtered.iter().enumerate() {
        let local_start = (entry.start - start).max(0.0);
        let local_end = (entry.end - start).max(0.0);
        if local_start >= local_end {
            continue;
        }
        let ts_start = crate::types::Timestamp::from_seconds(local_start);
        let ts_end = crate::types::Timestamp::from_seconds(local_end);
        writeln!(file, "{}", i + 1)?;
        writeln!(
            file,
            "{} --> {}",
            ts_start.to_srt_format(),
            ts_end.to_srt_format()
        )?;
        writeln!(file, "{}", entry.text)?;
        writeln!(file)?;
    }

    Ok(())
}

/// 拼接多个 .ts 文件为单个 mp4（使用 ffmpeg concat demuxer，不重编码）
fn write_clip_meta(
    path: &Path,
    title: &str,
    event_type: &crate::types::EventType,
    popularity: f64,
    rank: u32,
    start: crate::types::Timestamp,
    end: crate::types::Timestamp,
) -> Result<()> {
    let start_s = start.to_seconds();
    let end_s = end.to_seconds();
    let meta = serde_json::json!({
        "title": title,
        "event_type": serde_json::to_value(event_type).ok(),
        "popularity": popularity,
        "rank": rank,
        "original_start": start_s,
        "original_end": end_s,
        "duration": (end_s - start_s).max(0.0),
    });
    std::fs::write(path, serde_json::to_string_pretty(&meta)?)
        .with_context(|| format!("写入元数据失败: {}", path.display()))?;
    Ok(())
}

fn load_transcript(ctx: &PipelineContext) -> Option<crate::types::Transcript> {
    // 1. 优先用 pipeline context 中的 transcript（已被 Transcriber 或 LLM 订正填充）
    if let Some(ref t) = ctx.transcript {
        return Some(t.clone());
    }

    // 2. 按 config 中 transcript_file 路径加载
    let cfg_path = ctx.config.project.transcript_file
        .as_ref()
        .filter(|p| !p.is_empty())
        .map(PathBuf::from)
        .filter(|p| p.exists());
    if let Some(path) = cfg_path {
        if let Ok(content) = std::fs::read_to_string(&path) {
            let ext = path.extension()
                .map(|e| e.to_string_lossy().to_lowercase())
                .unwrap_or_default();
            let transcript = if ext == "json" {
                serde_json::from_str(&content).ok()
            } else {
                crate::srt::parse_srt(&content).ok()
            };
            if transcript.is_some() {
                return transcript;
            }
        }
    }

    // 3. 兜底 output/video.srt
    let fallback = ctx.output_dir.join("video.srt");
    if fallback.exists() {
        if let Ok(content) = std::fs::read_to_string(&fallback) {
            if let Ok(t) = crate::srt::parse_srt(&content) {
                return Some(t);
            }
        }
    }

    None
}

#[derive(serde::Serialize)]
struct ClipInfo {
    title: String,
    summary: String,
    event_type: crate::types::EventType,
    popularity: f64,
    rank: u32,
    #[serde(serialize_with = "serialize_timestamp")]
    start: crate::types::Timestamp,
    #[serde(serialize_with = "serialize_timestamp")]
    end: crate::types::Timestamp,
    has_danmaku: bool,
}

fn serialize_timestamp<S>(ts: &crate::types::Timestamp, s: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    s.serialize_f64(ts.to_seconds())
}

#[derive(serde::Serialize)]
struct PublishConfig {
    video_source: String,
    clips: Vec<ClipEntry>,
}

#[derive(serde::Serialize)]
struct ClipEntry {
    file: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    subtitle: String,
    danmaku: Option<String>,
    title: String,
    summary: String,
    event_type: String,
    danmaku_popularity: f64,
    rank: u32,
    start: f64,
    end: f64,
    duration: f64,
    directory: String,
}

impl PublishConfig {
    fn new(video_source: &Path, clips: &[ClipInfo]) -> Self {
        let clips: Vec<ClipEntry> = clips
            .iter()
            .map(|c| {
                let start = c.start.to_seconds();
                let end = c.end.to_seconds();
                let dir_name = format!("clip-{:02}", c.rank);
                ClipEntry {
                    file: format!("clips/{}/clip.mp4", dir_name),
                    subtitle: format!("clips/{}/clip.srt", dir_name),
                    danmaku: if c.has_danmaku {
                        Some(format!("clips/{}/clip.dm.srt", dir_name))
                    } else {
                        None
                    },
                    title: c.title.clone(),
                    summary: c.summary.clone(),
                    event_type: serde_json::to_value(&c.event_type)
                        .ok()
                        .and_then(|v| v.as_str().map(|s| s.to_owned()))
                        .unwrap_or_default(),
                    danmaku_popularity: c.popularity,
                    rank: c.rank,
                    start,
                    end,
                    duration: (end - start).max(0.0),
                    directory: dir_name,
                }
            })
            .collect();

        Self {
            video_source: video_source.to_string_lossy().to_string(),
            clips,
        }
    }
}
