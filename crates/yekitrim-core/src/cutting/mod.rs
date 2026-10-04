//! cutting：Stage 4 切片（按事件剪视频，写片段元数据与片段弹幕字幕）。
//!
//! 入口：`Cutter`；子模块 `silence`（静音检测，整体重导出）、`clip_index`（片段目录命名与索引刷新）。
//! 边界：产物 `clips/clips_order.txt` 是 Stage 4 的推导依据，别改目录布局；取弹幕一律走 `danmaku::load_danmaku_auto` —— 见 `crates/yekitrim-core/src/cutting/AGENTS.md`

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use anyhow::{Context, Result};

mod silence;
pub use silence::*;

pub mod clip_index;

use clip_index::{clip_meta_matches, is_safe_component, refresh_index};

use crate::danmaku::DanmakuEntry;
use crate::pipeline::{Stage, STAGE_CUT};
use crate::types::PipelineContext;

pub struct Cutter;

#[async_trait]
impl Stage for Cutter {
    fn name(&self) -> &'static str {
        "Cutter"
    }

    fn stage_number(&self) -> u32 {
        STAGE_CUT
    }

    async fn run(&self, ctx: &mut PipelineContext) -> Result<()> {
        if ctx.events.is_empty() {
            anyhow::bail!("没有事件可切割，请先运行 detect 阶段");
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

        // Load danmaku entries if available (shared via Arc to avoid clone per event)
        // `load_danmaku_auto`：ASS / 录播姬 XML / SRT 都认 —— 与检测侧的热力图用同一条解析路径，
        // 否则 danmaku_file 指向 XML 时检测有热力图、这边却悄悄不生成 clip.dm.srt
        let all_danmaku: Arc<Vec<DanmakuEntry>> = Arc::new(ctx
            .danmaku_path
            .as_ref()
            .filter(|p| p.exists())
            .and_then(|p| crate::danmaku::load_danmaku_auto(p).ok())
            .unwrap_or_default());

        let max_concurrent = 3;
        let total_events = ctx.events.len();
        let mut clip_infos: Vec<ClipInfo> = Vec::new();

        for batch_start in (0..total_events).step_by(max_concurrent) {
            // 取消在批次边界生效（进行中的 ffmpeg 不会被中断）
            if ctx.progress.cancelled() {
                anyhow::bail!("任务已取消");
            }
            let batch_end = (batch_start + max_concurrent).min(total_events);

            let batch_results: Vec<Result<ClipInfo>> = std::thread::scope(|scope| {
                let mut handles = Vec::with_capacity(batch_end - batch_start);

                for event in &ctx.events[batch_start..batch_end] {
                    let start = event.start.to_seconds();
                    let end = event.end.to_seconds();
                    let dir_name = clip_dir_name_for_event(event);
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
                    let danmaku = Arc::clone(&all_danmaku);

                    handles.push(scope.spawn(move || {
                        let clip_dir = cd.join(&dir_name);
                        // **先确认这个目录不是别人的片段**：目录名 = `clip-{rank}`，重新检测会重排 rank，
                        // 新事件可能正好撞上老片段的序号 —— 直接写下去就把人家剪好（可能还精修过）的成片
                        // 覆盖了，不可恢复。判据与前端 `clipMatchesEvent` 一致：标题相同**或**区间基本重合
                        // （IoU ≥ 0.5）都算同一条事件的重剪，只有两者都对不上才拦。
                        if let Some(reason) = foreign_clip_conflict(&clip_dir, &t, ts_start, ts_end) {
                            return Err(anyhow::anyhow!(reason));
                        }
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
                        if let Err(e) = write_clip_meta(&meta_path, &t, &s, &et, pop, rank, ts_start, ts_end) {
                            tracing::warn!("写入 clip.json 失败 ({}): {}", dir_name, e);
                        }

                        Ok(ClipInfo {
                            dir_name,
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

                // 子线程 panic 不能 `unwrap` 出来：服务端的任务体是"末尾自己调 complete/fail"的，
                // panic 会让 Job 永远停在 running、资源键不释放（同片段的后续任务全排死）。
                // 这里把 panic 降级成这条剪切失败。
                handles
                    .into_iter()
                    .map(|h| match h.join() {
                        Ok(r) => r,
                        Err(_) => Err(anyhow::anyhow!("剪切线程 panic（已跳过这条）")),
                    })
                    .collect()
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

        // 索引从磁盘重建：一次扫描把所有新片段收进 `clips/index.json`。
        // 失败只告警 —— 索引是缓存，不能因为写索引失败让已经剪好的成片算失败。
        if let Err(e) = refresh_index(&ctx.output_dir) {
            tracing::warn!("刷新片段索引失败: {e:#}");
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
            format!("{} / {}", clip_dir_name_for_event(ev), ev.label)
        }).collect();
        std::fs::write(&order_path, order_lines.join("\n"))
            .context("写入 clips_order.txt 失败")?;
        tracing::info!("播放顺序已保存: {}", order_path.display());

        tracing::info!("剪切完成: {}/{} 个片段", clip_infos.len(), ctx.events.len());
        Ok(())
    }
}

/// 一个候选编码器：`(名称, preset, 质量参数名)`。
type EncoderSpec = (&'static str, &'static str, &'static str);

const SOFTWARE_ENCODER: EncoderSpec = ("libx264", "ultrafast", "-crf");

/// 硬件编码器的候选表（**顺序即优先级**）。
///
/// ⚠️ 探测分两步，缺一不可（2026-10-05 修）：
///   ① `ffmpeg -encoders` 里**有**这个名字 —— 只能说明这份 ffmpeg **带该编码器编译**；
///   ② **真编一小段**能成功 —— 才说明**这台机器**确实能编。
///
/// 只做 ① 会踩的坑（实测）：本机 ffmpeg 列表里有 `h264_nvenc`，但没有 NVIDIA 设备，
/// 于是选中 nvenc → 每条剪切都 `Conversion failed!` → `所有剪切任务均失败`。
/// 判据是"ffmpeg 的编译选项"而不是"有无显卡"，所以**装了全功能 ffmpeg 的无卡机器
/// （Windows 同样）都会中招**。
fn hw_encoder_candidates() -> &'static [EncoderSpec] {
    &[
        ("h264_nvenc", "p1", "-cq"),
        ("h264_qsv", "veryfast", "-global_quality"),
        ("h264_amf", "speed", "-quality"),
    ]
}

/// 真编一小段测试图，验证该编码器在本机可用。
///
/// 用 `testsrc` 合成源（不碰用户素材）+ `-f null`（不落盘），3 帧而已，代价可忽略；
/// 探测结果由 `encoder_config()` 的 `OnceLock` 缓存，每个进程只跑一次。
fn encoder_actually_works(spec: EncoderSpec) -> bool {
    let out = crate::process::cmd("ffmpeg")
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-f",
            "lavfi",
            "-i",
            "testsrc=size=128x128:rate=5",
            "-frames:v",
            "3",
            "-c:v",
            spec.0,
            "-preset",
            spec.1,
            spec.2,
            "23",
            "-pix_fmt",
            "yuv420p",
            "-f",
            "null",
            "-",
        ])
        .output();

    match out {
        Ok(o) => o.status.success(),
        Err(_) => false,
    }
}

/// 选择本机可用的视频编码器（进程内缓存一次）。
///
/// 优先硬件编码（逐个真编验证），全不可用则回落 `libx264`。
/// ⚠️ `libx264` 不参与探测：它是软件编码，`ffmpeg -encoders` 都没有它的情况极罕见，
/// 而"探测失败连兜底都没有"会让整个 Stage 4 直接不可用 —— 保持它无条件兜底。
fn encoder_config() -> EncoderSpec {
    static CFG: OnceLock<EncoderSpec> = OnceLock::new();
    *CFG.get_or_init(|| {
        let output = crate::process::cmd("ffmpeg")
            .args(["-hide_banner", "-encoders"])
            .output();
        let stdout = match output {
            Ok(o) => String::from_utf8_lossy(&o.stdout).to_string(),
            Err(_) => return SOFTWARE_ENCODER,
        };

        for spec in hw_encoder_candidates() {
            if !stdout.contains(spec.0) {
                continue;
            }
            // 列表里有 ≠ 本机能用：必须真编一次才算数。
            if encoder_actually_works(*spec) {
                return *spec;
            }
            tracing::warn!(
                "{} 已被编译进 ffmpeg，但本机实际编码失败（无对应硬件/驱动不可用），跳过",
                spec.0
            );
        }

        SOFTWARE_ENCODER
    })
}

/// 兜底：硬件编码器**选中时可用**、但**跑到一半失效**（驱动崩溃、显存耗尽、
/// 并发切片段时句柄耗尽……）的情况。此时用 `libx264` 重试一次，别让整批剪切白跑。
fn is_software_encoder(spec: EncoderSpec) -> bool {
    spec.0 == SOFTWARE_ENCODER.0
}

fn cut_segment(video: &Path, start: f64, end: f64, output: &Path) -> Result<()> {
    let enc = encoder_config();
    match run_cut(video, start, end, output, enc) {
        Ok(()) => Ok(()),
        Err(e) if !is_software_encoder(enc) => {
            // 硬件编码失败 → 用软件编码重试一次（AGENTS.md 承诺的"自动降级"）。
            tracing::warn!("{} 编码失败，降级 libx264 重试: {}", enc.0, e);
            run_cut(video, start, end, output, SOFTWARE_ENCODER).map_err(|e2| {
                anyhow::anyhow!("{} 与 libx264 均失败；libx264 报: {}", enc.0, e2)
            })
        }
        Err(e) => Err(e),
    }
}

fn run_cut(video: &Path, start: f64, end: f64, output: &Path, enc: EncoderSpec) -> Result<()> {
    let duration = (end - start).max(0.1);
    let mut cmd = crate::process::cmd("ffmpeg");
    cmd.arg("-y")
        .arg("-loglevel")
        .arg("error")
        .arg("-ss")
        .arg(format!("{:.3}", start))
        .arg("-i")
        .arg(video)
        .arg("-t")
        .arg(format!("{:.3}", duration))
        .arg("-c:v")
        .arg(enc.0)
        .arg("-preset")
        .arg(enc.1)
        .arg(enc.2)
        .arg("23")
        .arg("-pix_fmt")
        .arg("yuv420p")
        .arg("-c:a")
        .arg("aac")
        .arg("-movflags")
        .arg("+faststart")
        .arg(output);
    run_ffmpeg(&mut cmd, Duration::from_secs_f64(cut_timeout_secs(duration)), "ffmpeg 片段剪切")?;
    Ok(())
}

/// 单条 ffmpeg 剪切的硬超时（秒）。
///
/// 素材常在网络盘上（RaiDrive/WebDAV）：盘一抖，ffmpeg 会**永远**卡在 read 上。
/// 没有超时 = 这条剪切任务永远不结束、`p{pid}:out`/`idx` 永不释放，此后该项目**所有**
/// 剪切都会排队、切片页的按钮一直灰着（用户实测的"剪切单个视频被卡住"）。
/// 按片长给余量：正常编码远快于 3× 实时，真卡住则最多等这么久。
fn cut_timeout_secs(duration: f64) -> f64 {
    (duration * 3.0).max(180.0)
}

/// 跑一个 ffmpeg 子进程，**带超时**（超时就 kill，绝不无限等）。
///
/// stderr 交给独立线程抽干：管道写满会让子进程自己卡住，那正是这里要避免的情形。
/// 成功返回 stderr（调用方通常忽略），失败/超时返回带原因的 `Err`。
fn run_ffmpeg(cmd: &mut Command, timeout: Duration, what: &str) -> Result<String> {
    use std::io::Read;

    let mut child = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("{what} 启动失败（请确保 ffmpeg 在 PATH 中）"))?;
    let drain = {
        let stderr = child.stderr.take();
        std::thread::spawn(move || {
            let mut buf = String::new();
            if let Some(mut e) = stderr {
                let _ = e.read_to_string(&mut buf);
            }
            buf
        })
    };

    let deadline = Instant::now() + timeout;
    let status = loop {
        if let Some(s) = child.try_wait().with_context(|| format!("{what} 等待失败"))? {
            break Some(s);
        }
        if Instant::now() >= deadline {
            // 卡住的子进程要**连进程带管道**一起收掉，否则 drain 线程永远 join 不回来
            let _ = child.kill();
            let _ = child.wait();
            break None;
        }
        std::thread::sleep(Duration::from_millis(200));
    };
    let stderr = drain.join().unwrap_or_default();

    match status {
        None => anyhow::bail!(
            "{what} 超时（超过 {:.0}s）已终止：素材读不动，可能卡在网络盘上；稍后可重试",
            timeout.as_secs_f64()
        ),
        Some(s) if !s.success() => anyhow::bail!("{what} 失败: {}", stderr.trim()),
        Some(_) => Ok(stderr),
    }
}

/// 对已输出的视频文件执行静音移除（重编码）。
/// `duration` 为连续静音秒数阈值，超过此长度的静音段将被移除。
/// 字幕按**重叠**取，并对边界容差 —— 别改回"完全包含"。
///
/// ⚠️ 2026-10-05：原判据是 `s.start >= start && s.end <= end`（要求**完全包含**），
/// 结果是**边界贴合的字幕被静默丢掉**，整条片段的 `clip.srt` 直接不生成：
///
///   转写里 `segment.end = 3.8730000572204593`（f64 全精度）
///   事件里 `end        = 3.873`（SRT 只存到毫秒，事件是从它派生的）
///   ⇒ `3.8730000572204593 <= 3.873` 为 **false**，差 5.7e-8 就被判掉
///
/// 这是典型的"两个精度不一致的数直接比大小"。而且它**不报错**——
/// `generate_clip_subtitle` 见 `filtered.is_empty()` 就静默 `Ok(())`，
/// 用户只会发现"片段没字幕"，查不出为什么。
///
/// 修法：① 改成重叠语义（与下面的弹幕 `generate_clip_danmaku` 一致，
/// 跨边界的字幕本来就该被算进来，出片时再裁剪）；② 再给一点浮点容差。
const SUBTITLE_TIME_EPSILON: f64 = 1e-3; // 1ms：SRT 的最小时间单位

fn generate_clip_subtitle(
    transcript: &crate::types::Transcript,
    start: f64,
    end: f64,
    output: &Path,
) -> Result<()> {
    let filtered: Vec<crate::types::TranscriptSegment> = transcript
        .segments
        .iter()
        .filter(|s| {
            s.start < end + SUBTITLE_TIME_EPSILON && s.end > start - SUBTITLE_TIME_EPSILON
        })
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
        asr_metadata: transcript.asr_metadata.clone(),
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
    summary: &str,
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
        // 切片自带摘要：重新检测会把 events.json 整份换掉，片段展示不该依赖它
        "summary": summary,
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

/// 事件实际要写入的目录名：优先 `clip_dir`，非法/缺失时回退旧的 `clip-{rank:02}`。
///
/// pub 是给**剪切收尾**用的（`yekitrim_service::init_polish_for_events` 要按同一条规则
/// 算出刚剪出来的目录，再补精修工程）—— 别在服务层再抄一份命名规则。
pub fn clip_dir_name_for_event(event: &crate::types::Event) -> String {
    let dir = event.clip_dir_or_rank();
    if is_safe_component(&dir) {
        dir
    } else {
        format!("clip-{:02}", event.rank)
    }
}

/// 目标目录里已有的片段**不是**这次要剪的事件时返回一句人话（否则 None）。
///
/// 目录名现在由 `event.clip_dir` 决定（检测时已按标题/区间匹配过旧片段），
/// 这个函数是**最后一道防覆盖保险**：万一目录被手动改过，或旧 events.json
/// 还没有 `clip_dir`，仍不能让新事件覆盖别人的成片。
/// 判据与前端 `utils.js` 的 `clipMatchesEvent` / `clip_index::clip_meta_matches` 一致：
/// **标题相同 或 区间 IoU ≥ 0.5** 视为同一条事件。
/// 老目录没有 `clip.json`（或字段缺失）时放行 —— 不能挡住正常重剪。
fn foreign_clip_conflict(
    clip_dir: &Path,
    label: &str,
    start: crate::types::Timestamp,
    end: crate::types::Timestamp,
) -> Option<String> {
    let raw = std::fs::read_to_string(clip_dir.join("clip.json")).ok()?;
    let meta: serde_json::Value = serde_json::from_str(&raw).ok()?;
    let s = start.to_seconds();
    let e = end.to_seconds();
    if clip_meta_matches(label, s, e, &meta) {
        return None;
    }
    // 旧行为：缺 original_start/original_end 时放行（老数据不能挡住重剪）。
    let (os, oe) = (
        meta.get("original_start").and_then(|v| v.as_f64())?,
        meta.get("original_end").and_then(|v| v.as_f64())?,
    );
    let old_title = meta.get("title").and_then(|v| v.as_str()).unwrap_or("").trim();
    let dir = clip_dir
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    Some(format!(
        "clips/{dir} 已属于另一个片段（「{old_title}」 {os:.1}s-{oe:.1}s），与本次要剪的事件「{}」（{s:.1}s-{e:.1}s）对不上；\
         先删掉那个片段再剪，避免覆盖已剪好的成片",
        label.trim()
    ))
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
    dir_name: String,
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
                let dir_name = c.dir_name.clone();
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

#[cfg(test)]
mod tests {
    use super::*;

    /// 自建临时目录（yekitrim-core 不依赖 tempfile；pid + 纳秒避免并发测试撞名）。
    struct TmpDir(PathBuf);

    impl TmpDir {
        fn new(tag: &str) -> Self {
            let nanos = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0);
            let p = std::env::temp_dir().join(format!(
                "yekitrim-cut-{tag}-{}-{nanos}",
                std::process::id()
            ));
            std::fs::create_dir_all(&p).unwrap();
            TmpDir(p)
        }
    }

    impl Drop for TmpDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn write_meta(dir: &Path, json: serde_json::Value) {
        std::fs::write(dir.join("clip.json"), serde_json::to_string(&json).unwrap()).unwrap();
    }

    fn ts(sec: f64) -> crate::types::Timestamp {
        crate::types::Timestamp::from_seconds(sec)
    }

    /// 重新检测后新事件撞上老片段的序号：标题/区间都对不上就不能覆盖（成片要保住）。
    #[test]
    fn foreign_clip_conflict_blocks_unrelated_clip() {
        let t = TmpDir::new("foreign");
        write_meta(t.0.as_path(), serde_json::json!({
            "title": "老片段标题", "original_start": 6000.0, "original_end": 6200.0
        }));
        let msg = foreign_clip_conflict(t.0.as_path(), "新事件标题", ts(100.0), ts(300.0));
        assert!(msg.is_some(), "标题/区间都对不上时必须拦下来");
        let msg = msg.unwrap();
        assert!(msg.contains("老片段标题"), "{msg}");
        assert!(msg.contains("先删掉那个片段"), "{msg}");
    }

    /// 同一条事件重剪要放行：标题一样（哪怕区间被手动改过），或区间基本重合（哪怕改了标题）。
    #[test]
    fn foreign_clip_conflict_allows_recut_of_same_event() {
        let same_title = TmpDir::new("same-title");
        write_meta(same_title.0.as_path(), serde_json::json!({
            "title": "同一条", "original_start": 100.0, "original_end": 300.0
        }));
        assert!(foreign_clip_conflict(same_title.0.as_path(), "同一条", ts(500.0), ts(700.0)).is_none(),
                "标题相同 → 视为重剪");

        let overlap = TmpDir::new("overlap");
        write_meta(overlap.0.as_path(), serde_json::json!({
            "title": "改过标题", "original_start": 100.0, "original_end": 300.0
        }));
        // 区间 110-290：IoU ≈ 0.9 ≥ 0.5
        assert!(foreign_clip_conflict(overlap.0.as_path(), "另一标题", ts(110.0), ts(290.0)).is_none(),
                "区间基本重合 → 视为重剪");
    }

    /// 老目录没有 clip.json（或没有区间字段）时放行：不能挡住正常剪切。
    #[test]
    fn foreign_clip_conflict_allows_legacy_dirs() {
        let empty = TmpDir::new("empty");
        assert!(foreign_clip_conflict(empty.0.as_path(), "随便", ts(0.0), ts(10.0)).is_none(),
                "没有 clip.json → 放行");

        let no_range = TmpDir::new("no-range");
        write_meta(no_range.0.as_path(), serde_json::json!({ "title": "只有标题" }));
        assert!(foreign_clip_conflict(no_range.0.as_path(), "别的标题", ts(0.0), ts(10.0)).is_none(),
                "缺区间字段 → 放行（老数据）");
    }

    // ---------------------------------------------------------------- 字幕取用
    //
    // 回归钉子（2026-10-05）：边界贴合的字幕曾被**静默丢掉**，导致整条片段
    // 没有 clip.srt。根因是 `s.start >= start && s.end <= end` 的完全包含判据
    // 撞上"转写全精度 f64 vs 事件毫秒精度"的误差（差 5.7e-8 即判掉）。

    fn seg(start: f64, end: f64, text: &str) -> crate::types::TranscriptSegment {
        crate::types::TranscriptSegment {
            start,
            end,
            text: text.into(),
            confidence: 1.0,
        }
    }

    fn transcript_of(segs: Vec<crate::types::TranscriptSegment>) -> crate::types::Transcript {
        crate::types::Transcript { segments: segs, language: "zh".into(), asr_metadata: None }
    }

    /// 真实踩到的形态：转写段 end 略大于事件 end（毫秒舍入误差）。
    #[test]
    fn clip_subtitle_keeps_segment_that_overhangs_end_by_rounding() {
        let t = TmpDir::new("sub-round");
        let out = t.0.join("clip.srt");
        // 事件 0.853–3.873 来自 SRT；转写段保留 f64 全精度
        let tr = transcript_of(vec![seg(0.852999997317791, 3.8730000572204593, "边界")]);
        generate_clip_subtitle(&tr, 0.853, 3.873, &out).unwrap();

        assert!(out.exists(), "边界贴合的字幕必须落盘 —— 这正是曾静默丢失的形态");
        let body = std::fs::read_to_string(&out).unwrap();
        assert!(body.contains("边界"), "字幕内容不能空: {body}");
    }

    /// 完全落在片段内的字幕当然要留。
    #[test]
    fn clip_subtitle_keeps_segment_fully_inside() {
        let t = TmpDir::new("sub-inside");
        let out = t.0.join("clip.srt");
        let tr = transcript_of(vec![seg(2.0, 4.0, "里面")]);
        generate_clip_subtitle(&tr, 1.0, 10.0, &out).unwrap();
        assert!(std::fs::read_to_string(&out).unwrap().contains("里面"));
    }

    /// 跨片段边界的字幕要**算进来**（出片时再裁），与弹幕的重叠语义一致。
    #[test]
    fn clip_subtitle_keeps_segment_crossing_boundary() {
        let t = TmpDir::new("sub-cross");
        let out = t.0.join("clip.srt");
        let tr = transcript_of(vec![seg(0.5, 5.0, "跨边界")]);
        generate_clip_subtitle(&tr, 1.0, 3.0, &out).unwrap();
        assert!(std::fs::read_to_string(&out).unwrap().contains("跨边界"),
                "跨边界的字幕不能丢");
    }

    /// 但**真的**在片段之外的字幕不许被带进来（别把容差放大成"全都收"）。
    #[test]
    fn clip_subtitle_drops_segment_far_outside() {
        let t = TmpDir::new("sub-outside");
        let out = t.0.join("clip.srt");
        let tr = transcript_of(vec![seg(50.0, 60.0, "远处")]);
        generate_clip_subtitle(&tr, 1.0, 3.0, &out).unwrap();
        assert!(!out.exists(), "完全不重叠的字幕不应产出 clip.srt");
    }

    /// 时间全部相对片段起点平移，不能出现负数。
    #[test]
    fn clip_subtitle_shifts_times_to_clip_origin() {
        let t = TmpDir::new("sub-shift");
        let out = t.0.join("clip.srt");
        let tr = transcript_of(vec![seg(8.0, 12.0, "挪")]);
        // 片段 10–20s：该段与片段重叠 10–12s ⇒ 平移后应是 0.000–2.000
        generate_clip_subtitle(&tr, 10.0, 20.0, &out).unwrap();
        let body = std::fs::read_to_string(&out).unwrap();
        let time_line = body.lines().find(|l| l.contains("-->")).expect("应有时间行");
        // ⚠️ 别拿 '-' 当"负数"判据：SRT 的时间行本身就含 '-->'。
        assert_eq!(time_line.trim(), "00:00:00,000 --> 00:00:02,000",
                   "时间应相对片段起点平移且不为负: {time_line}");
    }

    /// 编码器选择必须有可用的兜底，且 `libx264` 被判为"软件编码"。
    #[test]
    fn software_encoder_is_the_fallback() {
        assert!(is_software_encoder(SOFTWARE_ENCODER));
        assert_eq!(SOFTWARE_ENCODER.0, "libx264");
        // 硬件候选不该把 libx264 重复列进去（否则兜底逻辑会绕圈）
        assert!(
            !hw_encoder_candidates().iter().any(|c| c.0 == "libx264"),
            "libx264 不该出现在硬件候选表里"
        );
    }

    /// 选出来的编码器必须是**真能编**的 —— 在无卡机器上应回落到 libx264。
    /// 这条是"无 N 卡也能出片"的守门测试（测本机 ffmpeg 的真实能力）。
    #[test]
    fn encoder_config_never_returns_an_unusable_hardware_encoder() {
        let picked = encoder_config();
        if !is_software_encoder(picked) {
            assert!(
                encoder_actually_works(picked),
                "选中了硬件编码器 {} 但实测编不了 —— 探测逻辑失效",
                picked.0
            );
        }
    }
}
