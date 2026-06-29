use std::path::Path;
use std::process::Command;
use std::sync::OnceLock;

use async_trait::async_trait;
use anyhow::{Context, Result};

use crate::config::Config;
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

        let srt_path = ctx.output_dir.join("video.srt");
        let full_transcript = if srt_path.exists() {
            let content = std::fs::read_to_string(&srt_path)
                .context("读取字幕文件失败")?;
            Some(crate::asr::parse_srt(&content)?)
        } else {
            None
        };

        let mut clip_infos: Vec<ClipInfo> = Vec::new();

        for event in &ctx.events {
            let start = event.start.to_seconds();
            let end = event.end.to_seconds();
            let label = sanitize_label(&event.label);
            let rank = event.rank;

            let filename = format!("{}.mp4", label);
            let output_path = clips_dir.join(&filename);

            tracing::info!("剪切: {} ({:.1}s-{:.1}s)", filename, start, end);
            cut_segment(video, start, end, &output_path)?;

            let sub_filename = format!("{}.srt", label);
            let sub_path = clips_dir.join(&sub_filename);
            if let Some(ref t) = full_transcript {
                if let Err(e) = generate_clip_subtitle(t, start, end, &sub_path) {
                    tracing::warn!("字幕生成失败 ({}): {}", filename, e);
                }
            }

            clip_infos.push(ClipInfo {
                filename,
                sub_filename,
                title: event.label.clone(),
                summary: event.summary.clone(),
                event_type: event.event_type.clone(),
                popularity: event.popularity,
                rank,
                start: event.start,
                end: event.end,
            });
        }

        if clip_infos.is_empty() {
            anyhow::bail!("所有剪切任务均失败");
        }

        let publish = PublishConfig::new(video, &clip_infos);
        let json = serde_json::to_string_pretty(&publish)?;
        let publish_path = ctx.output_dir.join("publish_config.json");
        std::fs::write(&publish_path, &json).context("写入 publish_config.json 失败")?;
        tracing::info!("发布配置已保存: {}", publish_path.display());

        // 按弹幕热度排序写入
        let order_path = clips_dir.join("clips_order.txt");
        let mut order_events: Vec<_> = ctx.events.iter().collect();
        order_events.sort_by(|a, b| b.popularity.partial_cmp(&a.popularity).unwrap());
        let order_lines: Vec<String> = order_events.iter().map(|ev| {
            sanitize_label(&ev.label)
        }).collect();
        std::fs::write(&order_path, order_lines.join("\n"))
            .context("写入 clips_order.txt 失败")?;
        tracing::info!("播放顺序已保存: {}", order_path.display());

        let mut segments = Vec::new();
        for ev in &ctx.events {
            let label = sanitize_label(&ev.label);
            let rank = ev.rank;
            segments.push(Segment {
                index: rank as usize,
                event: ev.clone(),
                output_path: clips_dir.join(format!("{}.mp4", label)),
                subtitle_path: clips_dir.join(format!("{}.srt", label)),
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

pub fn remove_silence(input: &Path, output: &Path, duration: f64) -> Result<Vec<(f64, f64)>> {
    let segments = detect_silence(input, duration)?;

    if segments.is_empty() {
        std::fs::copy(input, output)?;
        return Ok(segments);
    }

    let result = Command::new("ffmpeg")
        .arg("-y")
        .arg("-loglevel")
        .arg("error")
        .arg("-i")
        .arg(input)
        .arg("-af")
        .arg(format!(
            "silenceremove=stop_periods=-1:stop_duration={}:stop_threshold=-30dB",
            duration
        ))
        .arg("-vf")
        .arg("setpts=N/FRAME_RATE/TB")
        .arg("-async")
        .arg("1")
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
    Ok(segments)
}

pub fn adjust_srt(input: &Path, output: &Path, silence_segments: &[(f64, f64)]) -> Result<()> {
    let content = std::fs::read_to_string(input)?;
    let mut transcript = crate::asr::parse_srt(&content)?;

    for seg in &mut transcript.segments {
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
    }

    crate::asr::write_srt(&transcript, output).context("写入调整后字幕失败")
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

    crate::asr::write_srt(&clip_transcript, output)
        .context("写入 clip 字幕失败")?;
    Ok(())
}

fn sanitize_label(s: &str) -> String {
    let invalid = ['\\', '/', ':', '*', '?', '"', '<', '>', '|'];
    s.chars()
        .map(|c| {
            if invalid.contains(&c) || c.is_control() {
                '_'
            } else {
                c
            }
        })
        .collect::<String>()
        .trim()
        .chars()
        .take(20)
        .collect()
}

#[derive(serde::Serialize)]
struct ClipInfo {
    filename: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    sub_filename: String,
    title: String,
    summary: String,
    event_type: crate::types::EventType,
    popularity: f64,
    rank: u32,
    #[serde(serialize_with = "serialize_timestamp")]
    start: crate::types::Timestamp,
    #[serde(serialize_with = "serialize_timestamp")]
    end: crate::types::Timestamp,
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
    title: String,
    summary: String,
    event_type: String,
    danmaku_popularity: f64,
    rank: u32,
    start: f64,
    end: f64,
    duration: f64,
}

impl PublishConfig {
    fn new(video_source: &Path, clips: &[ClipInfo]) -> Self {
        let clips: Vec<ClipEntry> = clips
            .iter()
            .map(|c| {
                let start = c.start.to_seconds();
                let end = c.end.to_seconds();
                ClipEntry {
                    file: format!("clips/{}", c.filename),
                    subtitle: format!("clips/{}", c.sub_filename),
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
                }
            })
            .collect();

        Self {
            video_source: video_source.to_string_lossy().to_string(),
            clips,
        }
    }
}
