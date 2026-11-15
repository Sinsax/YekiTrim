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
use crate::types::{EventRange, PipelineContext};

pub struct Cutter;

/// 一条事件的剪切结果：剪成 / 按配置跳过 —— **都不算失败**。
enum ClipOutcome {
    Cut(ClipInfo),
    Skipped(String),
}

/// 「已有片段要不要覆盖」的最终判据。
///
/// - `[cutting] overwrite = true` ⇒ 覆盖（老行为，用户显式要求）。
/// - `false` / 没写 ⇒ **批量与 `process` 不覆盖**：目标 `clip.json` 与本条事件匹配（同一条事件）
///   且 `clip.mp4` 已存在 ⇒ 跳过并 warn（对应设置页「覆盖已有文件」没勾）。
/// - **单条剪切例外**：`ctx.events.len() <= 1` 就是用户在切片页点的那一条
///   （server 的 `run_single_cut_event` 只塞一个 event 进来），"点剪切"本身就是显式要求重剪，
///   没有别的通道能表达它，所以这时永远覆盖。
fn effective_overwrite(ctx: &PipelineContext) -> bool {
    if ctx.config.cutting.overwrite == Some(true) {
        return true;
    }
    ctx.events.len() <= 1
}

/// 把一条**非致命**的阶段结论落三处：tracing（运行日志/CLI 终端）+ `progress.note`
/// （任务消息通道，server 任务框可见）+ `notes`（= `ctx.pipeline_notes`，程序化消费方）。
///
/// ⚠️ 绝不能走 `progress.error`：server 的 error = 整条任务 fail，而「部分失败但其余已出片」
/// 按产品口径仍算成功（见 review D9 / P2-5）。
/// ⚠️ 参数拆开收 `notes` 与 `progress`，**不收 `&mut ctx`**：`Cutter::run` 里 `video`
/// 长期借用着 `ctx.input_path`，整借 ctx 会撞 E0502（字段级分离借用没事）。
fn push_pipeline_note(progress: &dyn crate::progress::ProgressReporter, notes: &mut Vec<String>, msg: String) {
    tracing::warn!("{msg}");
    progress.note(&msg);
    notes.push(msg);
}

/// `overwrite = false` 且目标目录里已经是**这条事件自己的**成片 ⇒ 返回一句跳过原因。
///
/// 判据与 `foreign_clip_conflict` 同源（`clip_meta_matches`：标题相同 或 原片区间 IoU ≥ 0.5）：
/// 别人的片段由 conflict 那条**报错**跳过，自己的成片在这里按配置**安静**跳过；
/// 没有 `clip.json` 的老目录照旧重剪（信息不足，不能当成"已经剪过"）。
fn skip_reason(
    clip_dir: &Path,
    label: &str,
    start: crate::types::Timestamp,
    end: crate::types::Timestamp,
    overwrite: bool,
) -> Option<String> {
    if overwrite || !clip_dir.join("clip.mp4").is_file() {
        return None;
    }
    let raw = std::fs::read_to_string(clip_dir.join("clip.json")).ok()?;
    let meta: serde_json::Value = serde_json::from_str(&raw).ok()?;
    if !clip_meta_matches(label, start.to_seconds(), end.to_seconds(), &meta) {
        return None;
    }
    Some(format!(
        "跳过已剪好的片段（[cutting] overwrite = false，要覆盖请设成 true 或单条重切）: {}",
        clip_dir.display()
    ))
}

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
        // 跳过 / 失败都是**非致命**结果：成片照常出，但要汇总给用户（→ `ctx.pipeline_notes`）。
        let mut skipped: Vec<String> = Vec::new();
        let mut failures: Vec<String> = Vec::new();
        let overwrite = effective_overwrite(ctx);
        // 同一个 `clip_dir` 被两条事件占用 ⇒ 两条 ffmpeg 抢写同一个 clip.mp4（成片互相截断）。
        // 派发前先按事件顺序认领目录，后来的跳过并记一条。检测侧 `assign_clip_dirs` 本该唯一，
        // 但手写/外部生成的 events.json 不受它管。
        let mut taken_dirs: std::collections::HashSet<String> = std::collections::HashSet::new();
        let mut duplicate_idx: Vec<usize> = Vec::new();
        for (i, event) in ctx.events.iter().enumerate() {
            if !taken_dirs.insert(clip_dir_name_for_event(event)) {
                duplicate_idx.push(i);
            }
        }
        if !duplicate_idx.is_empty() {
            let names: Vec<String> = duplicate_idx
                .iter()
                .map(|&i| clip_dir_name_for_event(&ctx.events[i]))
                .collect();
            let msg = format!(
                "事件清单里有 {} 条重复的片段目录（{}）：只剪第一个，其余跳过（否则两条 ffmpeg 会抢写同一个 clip.mp4）",
                duplicate_idx.len(),
                names.join(" / ")
            );
            push_pipeline_note(ctx.progress.as_ref(), &mut ctx.pipeline_notes, msg);
        }

        for batch_start in (0..total_events).step_by(max_concurrent) {
            // 取消在批次边界生效（进行中的 ffmpeg 不会被中断）
            if ctx.progress.cancelled() {
                anyhow::bail!("任务已取消");
            }
            // 磁盘预检也在批次边界：余量跌破 `[cutting] min_free_gb` ⇒ **整个阶段停止**，
            // 剩余片段不再派发（已剪的保留；重跑会跳过已有 clip.mp4 自动续作）。
            // 宁可停批也不能切到把盘写穿、被外部看门狗把源录播删掉（2026-11-15 事故）。
            crate::disk::ensure_free(&ctx.output_dir, ctx.config.cutting.min_free_bytes())?;
            let batch_end = (batch_start + max_concurrent).min(total_events);

            let batch_results: Vec<Result<ClipOutcome>> = std::thread::scope(|scope| {
                let mut handles = Vec::with_capacity(batch_end - batch_start);

                for (offset, event) in ctx.events[batch_start..batch_end].iter().enumerate() {
                    // 目录已被本批/前一批的另一条事件认领 —— 不剪（note 已记过一条）。
                    if duplicate_idx.binary_search(&(batch_start + offset)).is_ok() {
                        continue;
                    }
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
                    // **多段片段**（分散事件合并的产物）：`ranges` 为空时就是 `start..end` 那一段，
                    // 单段路径一个字节都不变（见 `Event::cut_ranges`）。
                    let ranges = event.cut_ranges();
                    let content_secs: f64 = ranges.iter().map(|r| r.duration()).sum();
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
                        // `[cutting] overwrite = false` 且目标已经是**这条事件自己的**成片 ⇒ 跳过
                        // （用户勾了「不覆盖已有文件」）。单条剪切不受它约束，见 `effective_overwrite`。
                        if let Some(reason) =
                            skip_reason(&clip_dir, &t, ts_start, ts_end, overwrite)
                        {
                            tracing::info!("{reason}");
                            return Ok(ClipOutcome::Skipped(reason));
                        }
                        std::fs::create_dir_all(&clip_dir)
                            .with_context(|| format!("创建目录失败: {}", clip_dir.display()))?;

                        let vid_path = clip_dir.join("clip.mp4");

                        if ranges.len() > 1 {
                            tracing::info!(
                                "剪切（多段）: {} {} 段，内容 {:.1}s / 跨度 {:.1}s-{:.1}s",
                                dir_name,
                                ranges.len(),
                                content_secs,
                                start,
                                end
                            );
                        } else {
                            tracing::debug!("剪切: {} ({:.1}s-{:.1}s)", dir_name, start, end);
                        }
                        cut_event(&v, &ranges, &vid_path)?;
                        let sub_path = clip_dir.join("clip.srt");
                        if let Some(ref t2) = transcript {
                            if let Err(e) = generate_clip_subtitle_ranges(t2, &ranges, &sub_path) {
                                tracing::warn!("字幕生成失败 ({}): {}", dir_name, e);
                            }
                        }

                        // ⚠️ `has_danmaku` 必须**逐条**判：这一段没有弹幕时 `generate_clip_danmaku_ranges`
                        // 什么都不写（不会留一个空文件），用全局 `!danmaku.is_empty()` 会让
                        // `publish_config.json` 指向一个不存在的 `clip.dm.srt`（实测 demo 的 clip-03）。
                        let mut has_danmaku = false;
                        if !danmaku.is_empty() {
                            let dm_path = clip_dir.join("clip.dm.srt");
                            match generate_clip_danmaku_ranges(&danmaku, &ranges, &dm_path) {
                                Ok(()) => has_danmaku = dm_path.is_file(),
                                Err(e) => tracing::warn!("弹幕生成失败 ({}): {}", dir_name, e),
                            }
                        }

                        let meta_path = clip_dir.join("clip.json");
                        if let Err(e) = write_clip_meta(
                            &meta_path, &t, &s, &et, pop, rank, ts_start, ts_end, &ranges,
                        ) {
                            tracing::warn!("写入 clip.json 失败 ({}): {}", dir_name, e);
                        }

                        Ok(ClipOutcome::Cut(ClipInfo {
                            dir_name,
                            title: t.clone(),
                            summary: s,
                            event_type: et,
                            popularity: pop,
                            rank,
                            start: ts_start,
                            end: ts_end,
                            has_danmaku,
                            content_secs,
                        }))
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
                    Ok(ClipOutcome::Cut(info)) => {
                        // 剪出明细（spec §1 P2）：逐片段 note 与批次百分比互补 ——
                        // 长片几十个片段时用户能看到「剪到哪一条」，而不是只有一根爬条。
                        ctx.progress.note(&format!(
                            "剪出 {}/{}：{}",
                            batch_end, total_events, info.title
                        ));
                        clip_infos.push(info);
                    }
                    Ok(ClipOutcome::Skipped(reason)) => {
                        tracing::info!("{reason}");
                        skipped.push(reason);
                    }
                    Err(e) => {
                        tracing::error!("剪切失败: {e:#}");
                        failures.push(format!("{e:#}"));
                    }
                }
            }

            ctx.progress.stage_progress(batch_end as u64, total_events as u64);
        }

        // 跳过 / 部分失败都要让用户看见，但不该把已经出片的任务判成失败（tracing + note + ctx 三落地）。
        if !skipped.is_empty() {
            let msg = format!(
                "按 [cutting] overwrite = false 跳过 {} 条已有片段（要重剪就把它设成 true，或在切片页单条重切）：{}",
                skipped.len(),
                skipped.join("; ")
            );
            push_pipeline_note(ctx.progress.as_ref(), &mut ctx.pipeline_notes, msg);
        }
        if !failures.is_empty() {
            let msg = format!(
                "{} 条片段剪切失败（其余已出片，失败条目不会进发布清单）：{}",
                failures.len(),
                failures.join(" | ")
            );
            push_pipeline_note(ctx.progress.as_ref(), &mut ctx.pipeline_notes, msg);
        }
        // 全部失败才算失败；「全部被跳过」是合法的空跑（发布清单按磁盘旧索引保留）。
        if clip_infos.is_empty() && !failures.is_empty() {
            anyhow::bail!("所有剪切任务均失败");
        }

        // 索引从磁盘重建：一次扫描把所有新片段收进 `clips/index.json`。
        // 失败只告警 —— 索引是缓存，不能因为写索引失败让已经剪好的成片算失败。
        if let Err(e) = refresh_index(&ctx.output_dir) {
            tracing::warn!("刷新片段索引失败: {e:#}");
        }

        // ⚠️ **不能只写这一批**：单条重切 / 一键全切是逐事件跑 Cutter 的（一个事件一次），
        //    直接 `PublishConfig::new(本次 clip_infos)` 会把另外两条片段从索引里**抹掉** ——
        //    实测一键全切之后 publish_config.json 只剩最后一条、clips_order.txt 也只剩一行。
        //    以磁盘上的旧索引为底：保留仍存在的老条目、用本次结果覆盖同名目录、丢掉文件已没了的。
        let publish_path = ctx.output_dir.join("publish_config.json");
        let publish = PublishConfig::merge_from_disk(&publish_path, video, &clip_infos);
        let json = serde_json::to_string_pretty(&publish)?;
        std::fs::write(&publish_path, &json).context("写入 publish_config.json 失败")?;
        tracing::info!("发布配置已保存: {}", publish_path.display());

        // 播放顺序同样从**合并后的全量**算（按弹幕热度降序），否则增量剪切会把它洗成一条。
        let order_path = clips_dir.join("clips_order.txt");
        let mut order_entries: Vec<&ClipEntry> = publish.clips.iter().collect();
        order_entries.sort_by(|a, b| {
            b.danmaku_popularity
                .partial_cmp(&a.danmaku_popularity)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        let order_lines: Vec<String> = order_entries.iter().map(|c| {
            format!("{} / {}", c.directory, c.title)
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

/// **多段剪切**：`ranges` 只有一段时走原来的单段路径（行为不变），多段时"逐段剪 → 拼接"。
///
/// 三段式兜底（与单段那条"硬件选中但跑到一半失效"同一精神）：
/// ① 同一编码器逐段剪成 `.ts` → `concat` **不重编码**；
/// ② 拼接失败（参数对不上/容器怪）→ 用同一编码器 `-filter_complex concat` **重编码一次**；
/// ③ 硬件编码器整体失败 → 换 `libx264` 把①②再来一遍。
///
/// ⚠️ 多段**必须一次选定编码器**：逐段各自探测的话，第一段 nvenc、第二段降级 libx264 就会
/// 拼出两个不同编码的流（`-c copy` 直接报错，或更糟：拼出坏文件）。
fn cut_event(video: &Path, ranges: &[EventRange], output: &Path) -> Result<()> {
    if ranges.len() <= 1 {
        let r = ranges
            .first()
            .copied()
            .unwrap_or(EventRange::new(0.0, 0.0));
        return cut_segment(video, r.start, r.end, output);
    }
    let enc = encoder_config();
    match cut_parts_and_concat(video, ranges, output, enc) {
        Ok(()) => Ok(()),
        Err(e) if !is_software_encoder(enc) => {
            tracing::warn!(
                "多段剪切（{} 段）用 {} 失败，降级 libx264 重试: {}",
                ranges.len(),
                enc.0,
                e
            );
            cut_parts_and_concat(video, ranges, output, SOFTWARE_ENCODER).map_err(|e2| {
                anyhow::anyhow!("{} 与 libx264 均失败；libx264 报: {}", enc.0, e2)
            })
        }
        Err(e) => Err(e),
    }
}

/// 逐段剪 `.ts` → concat；`.parts/` 目录无论如何都清掉（失败也不留垃圾）。
fn cut_parts_and_concat(
    video: &Path,
    ranges: &[EventRange],
    output: &Path,
    enc: EncoderSpec,
) -> Result<()> {
    let dir = output
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(".parts");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir)
        .with_context(|| format!("创建临时切片目录失败: {}", dir.display()))?;
    let result = (|| -> Result<()> {
        let mut parts: Vec<PathBuf> = Vec::with_capacity(ranges.len());
        for (i, r) in ranges.iter().enumerate() {
            let part = dir.join(format!("part-{i:03}.ts"));
            run_cut_part(video, r.start, r.end, &part, enc)?;
            parts.push(part);
        }
        match concat_parts_copy(&parts, output) {
            Ok(()) => Ok(()),
            Err(e) => {
                tracing::warn!("concat 直接拼接失败（改用重编码拼接）: {e}");
                concat_parts_reencode(&parts, output, enc)
            }
        }
    })();
    let _ = std::fs::remove_dir_all(&dir);
    result
}

/// 单段剪到 `.ts`（MPEG-TS 才能不重编码直接拼；`-bsf:v h264_mp4toannexb` 交给容器自己处理）。
fn run_cut_part(
    video: &Path,
    start: f64,
    end: f64,
    output: &Path,
    enc: EncoderSpec,
) -> Result<()> {
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
        .arg("-f")
        .arg("mpegts")
        .arg(output);
    run_ffmpeg(
        &mut cmd,
        Duration::from_secs_f64(cut_timeout_secs(duration)),
        "ffmpeg 分段剪切",
    )?;
    Ok(())
}

/// concat demuxer + `-c copy`（**不重编码**）。
/// concat demuxer + `-c copy`（**不重编码**）。
fn concat_parts_copy(parts: &[PathBuf], output: &Path) -> Result<()> {
    let list = output
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(".parts")
        .join("list.txt");
    let mut body = String::new();
    for p in parts {
        // concat demuxer 的单引号转义：单引号写成 '\''；路径反斜杠换正斜杠（Windows 上 ffmpeg 认）。
        // ⚠️ 刻意用 `\u{27}` / `\u{5c}` 而不是字符字面量：后者的反斜杠在补丁文本里太容易被吃掉。
        let q = '\u{27}';
        let bs = '\u{5c}';
        let s = p.to_string_lossy().replace(bs, "/");
        let s = s.replace(q, &format!("{q}{bs}{q}{q}"));
        body.push_str(&format!("file '{s}'\n"));
    }
    std::fs::write(&list, body).with_context(|| format!("写 concat 清单失败: {}", list.display()))?;

    let mut cmd = crate::process::cmd("ffmpeg");
    cmd.arg("-y")
        .arg("-loglevel")
        .arg("error")
        .arg("-f")
        .arg("concat")
        .arg("-safe")
        .arg("0")
        .arg("-i")
        .arg(&list)
        .arg("-c")
        .arg("copy")
        .arg("-movflags")
        .arg("+faststart")
        .arg(output);
    let total: f64 = parts.len() as f64 * 60.0;
    run_ffmpeg(
        &mut cmd,
        Duration::from_secs_f64(cut_timeout_secs(total)),
        "ffmpeg 分段拼接",
    )?;
    Ok(())
}

/// 重编码拼接（`concat` 滤镜）：copy 那条路失败时的兜底。
fn concat_parts_reencode(parts: &[PathBuf], output: &Path, enc: EncoderSpec) -> Result<()> {
    let mut cmd = crate::process::cmd("ffmpeg");
    cmd.arg("-y").arg("-loglevel").arg("error");
    for p in parts {
        cmd.arg("-i").arg(p);
    }
    let n = parts.len();
    let mut filter = String::new();
    for i in 0..n {
        filter.push_str(&format!("[{i}:v][{i}:a]"));
    }
    filter.push_str(&format!("concat=n={n}:v=1:a=1[v][a]"));
    cmd.arg("-filter_complex")
        .arg(filter)
        .arg("-map")
        .arg("[v]")
        .arg("-map")
        .arg("[a]")
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
    let total: f64 = n as f64 * 60.0;
    run_ffmpeg(
        &mut cmd,
        Duration::from_secs_f64(cut_timeout_secs(total)),
        "ffmpeg 分段重编码拼接",
    )?;
    Ok(())
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

/// 多段片段在成片时间轴上的**段起点**：第 i 段从 `offsets[i]` 开始（`offsets[0] == 0`）。
///
/// 单段时恒为 `[0.0]` —— 老路径一个字都不变。
pub(crate) fn range_offsets(ranges: &[EventRange]) -> Vec<f64> {
    let mut out = Vec::with_capacity(ranges.len().max(1));
    let mut acc = 0.0;
    for r in ranges {
        out.push(acc);
        acc += r.duration();
    }
    if out.is_empty() {
        out.push(0.0);
    }
    out
}

/// 时间窗列表（多段）→ 片段内时间轴：`(源区间, 成片内的段起点)`。
///
/// 多段片段是"分段剪、合并输出"：第 1 段之后的时间要减去前面所有段的时长。
/// **单段就是 `[(start, end)]`**，此时这段逻辑退化成老行为（减一个常数），所以老的
/// `generate_clip_subtitle` / `generate_clip_danmaku` 都改走这一条，不许再各写一份偏移。
fn clip_time_windows(ranges: &[EventRange]) -> Vec<(EventRange, f64)> {
    let offsets = range_offsets(ranges);
    ranges
        .iter()
        .copied()
        .zip(offsets)
        .collect::<Vec<(EventRange, f64)>>()
}

/// 多段字幕：逐段裁剪 + 按**段起点**平移（第 2 段起时间轴接在第 1 段之后）。
fn generate_clip_subtitle_ranges(
    transcript: &crate::types::Transcript,
    ranges: &[EventRange],
    output: &Path,
) -> Result<()> {
    let mut filtered: Vec<crate::types::TranscriptSegment> = Vec::new();
    for (r, off) in clip_time_windows(ranges) {
        for s in transcript.segments.iter().filter(|s| {
            s.start < r.end + SUBTITLE_TIME_EPSILON && s.end > r.start - SUBTITLE_TIME_EPSILON
        }) {
            let start = (s.start - r.start).max(0.0) + off;
            let end = (s.end - r.start).max(0.0) + off;
            if end <= start {
                continue;
            }
            filtered.push(crate::types::TranscriptSegment {
                start,
                end,
                text: s.text.clone(),
                confidence: s.confidence,
            });
        }
    }

    if filtered.is_empty() {
        return Ok(());
    }
    // 段落表按时间排一次：多段拼接后理论上有序，但源里跨段的段可能反序（防御）
    filtered.sort_by(|a, b| a.start.partial_cmp(&b.start).unwrap_or(std::cmp::Ordering::Equal));

    let clip_transcript = crate::types::Transcript {
        segments: filtered,
        language: transcript.language.clone(),
        asr_metadata: transcript.asr_metadata.clone(),
    };

    crate::srt::write_srt(&clip_transcript, output).context("写入 clip 字幕失败")?;
    Ok(())
}

/// 多段弹幕：与字幕同一条偏移口径。
fn generate_clip_danmaku_ranges(
    entries: &[DanmakuEntry],
    ranges: &[EventRange],
    output: &Path,
) -> Result<()> {
    struct Row {
        start: f64,
        end: f64,
        text: String,
    }
    let mut rows: Vec<Row> = Vec::new();
    for (r, off) in clip_time_windows(ranges) {
        for e in entries.iter().filter(|e| e.start < r.end && e.end > r.start) {
            let start = (e.start - r.start).max(0.0) + off;
            let end = (e.end - r.start).max(0.0) + off;
            if start >= end {
                continue;
            }
            rows.push(Row {
                start,
                end,
                text: e.text.clone(),
            });
        }
    }
    if rows.is_empty() {
        return Ok(());
    }
    rows.sort_by(|a, b| a.start.partial_cmp(&b.start).unwrap_or(std::cmp::Ordering::Equal));

    let mut file = std::fs::File::create(output)
        .with_context(|| format!("创建弹幕文件失败: {}", output.display()))?;
    use std::io::Write;
    file.write_all(b"\xEF\xBB\xBF")?;

    for (i, row) in rows.iter().enumerate() {
        let ts_start = crate::types::Timestamp::from_seconds(row.start);
        let ts_end = crate::types::Timestamp::from_seconds(row.end);
        writeln!(file, "{}", i + 1)?;
        writeln!(
            file,
            "{} --> {}",
            ts_start.to_srt_format(),
            ts_end.to_srt_format()
        )?;
        writeln!(file, "{}", row.text)?;
        writeln!(file)?;
    }

    Ok(())
}

#[cfg(test)]
fn generate_clip_subtitle(
    transcript: &crate::types::Transcript,
    start: f64,
    end: f64,
    output: &Path,
) -> Result<()> {
    generate_clip_subtitle_ranges(transcript, &[EventRange::new(start, end)], output)
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
    ranges: &[EventRange],
) -> Result<()> {
    let start_s = start.to_seconds();
    let end_s = end.to_seconds();
    // 成片时长 = **内容时长**（多段片段把段间空隙丢掉了，所以不是 `end - start`）。
    // 下游（预览/精修/切片卡）拿 `duration` 当"这条片段有多长"用，必须与文件一致。
    let duration = if ranges.is_empty() {
        (end_s - start_s).max(0.0)
    } else {
        ranges.iter().map(|r| r.duration()).sum::<f64>()
    };
    let mut meta = serde_json::json!({
        "title": title,
        // 切片自带摘要：重新检测会把 events.json 整份换掉，片段展示不该依赖它
        "summary": summary,
        "event_type": serde_json::to_value(event_type).ok(),
        "popularity": popularity,
        "rank": rank,
        "original_start": start_s,
        "original_end": end_s,
        "duration": duration,
    });
    if ranges.len() > 1 {
        // 多段片段：把每段源区间写进 `clip.json`（切片页/精修据此显示"由 N 段组成"）
        meta["ranges"] = serde_json::to_value(ranges).unwrap_or(serde_json::Value::Null);
    }
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

/// 工程配置里的相对路径 → 绝对路径：基准 = **工程根**（`output_dir` 的父目录）。
///
/// 与 `ProjectConfig::current_stage` / `pipeline::runner::resolve_danmaku` 同一条口径 ——
/// `transcript_file` / `danmaku_file` 都是**相对工程根**写进 yekitrim.toml 的
/// （`yekitrim init` 的 `path_to_rel`）。
fn resolve_project_path(ctx: &PipelineContext, path: PathBuf) -> PathBuf {
    if path.is_absolute() {
        return path;
    }
    match ctx.output_dir.parent() {
        Some(root) => root.join(path),
        None => path,
    }
}

/// 读一份转写文件（按扩展名分派 JSON / SRT）；读不了或解析不了 ⇒ `None`。
fn load_transcript_file(path: &Path) -> Option<crate::types::Transcript> {
    let content = std::fs::read_to_string(path).ok()?;
    let ext = path
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    if ext == "json" {
        serde_json::from_str(&content).ok()
    } else {
        crate::srt::parse_srt(&content).ok()
    }
}

fn load_transcript(ctx: &PipelineContext) -> Option<crate::types::Transcript> {
    // 1. 优先用 pipeline context 中的 transcript（已被 Transcriber 或 LLM 订正填充）
    if let Some(ref t) = ctx.transcript {
        return Some(t.clone());
    }

    // 2. 按 config 中 transcript_file 路径加载。
    //    ⚠️ 相对路径的基准是**工程根**，不是进程 CWD：裸 `PathBuf::from(p).exists()` 在
    //    `yekitrim server` 这种长驻进程里恒为假（CWD 与项目无关）⇒ 选中的外部/订正字幕
    //    静默退化成 `output/transcript.json` / `output/video.srt`，成片字幕"有、但是错的"。
    let cfg_path = ctx.config.project.transcript_file
        .as_ref()
        .filter(|p| !p.is_empty())
        .map(PathBuf::from)
        .map(|p| resolve_project_path(ctx, p))
        .filter(|p| p.exists());
    if let Some(path) = cfg_path {
        if let Some(t) = load_transcript_file(&path) {
            return Some(t);
        }
    }

    // 3. 兜底：**与 service 的 `Workspace::transcript_path` 同一顺序**
    //    （transcript_file → `output/transcript.json` → `output/video.srt`）。
    //    ⚠️ 只认 `video.srt` 会漏掉最常见的一种形态：ASR 产物在 `output/transcript.json`、
    //    工程没有 transcript_file、也没有 video.srt（单条剪切 / 一键全切实测：clip-01 只有
    //    `clip.dm.srt`、没有 `clip.srt` —— 片段无字幕）。
    if let Some(t) = load_transcript_file(&ctx.output_dir.join("transcript.json")) {
        return Some(t);
    }
    load_transcript_file(&ctx.output_dir.join("video.srt"))
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
    /// 这条片段实际剪出来的**内容时长**（多段 = 各段之和）。
    ///
    /// ⚠️ 与 `end - start`（跨度）不是一回事：`Event.ranges` 多段时成片长度是各段之和，
    /// 而跨度把段间空隙也算了进去 —— 发布清单里的 `duration` 必须用这个。
    content_secs: f64,
    has_danmaku: bool,
}

fn serialize_timestamp<S>(ts: &crate::types::Timestamp, s: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    s.serialize_f64(ts.to_seconds())
}

#[derive(serde::Serialize, serde::Deserialize)]
struct PublishConfig {
    #[serde(default)]
    video_source: String,
    #[serde(default)]
    clips: Vec<ClipEntry>,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct ClipEntry {
    #[serde(default)]
    file: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    subtitle: String,
    #[serde(default)]
    danmaku: Option<String>,
    #[serde(default)]
    title: String,
    #[serde(default)]
    summary: String,
    #[serde(default)]
    event_type: String,
    #[serde(default)]
    danmaku_popularity: f64,
    #[serde(default)]
    rank: u32,
    #[serde(default)]
    start: f64,
    #[serde(default)]
    end: f64,
    #[serde(default)]
    duration: f64,
    #[serde(default)]
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
                // 多段片段的成片时长 = 各段之和（跨度会把段间空隙算进去，上传/校验脚本对不上）。
                let duration = if c.content_secs.is_finite() && c.content_secs > 0.0 {
                    c.content_secs
                } else {
                    (end - start).max(0.0)
                };
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
                    duration,
                    directory: dir_name,
                }
            })
            .collect();

        Self {
            video_source: video_source.to_string_lossy().to_string(),
            clips,
        }
    }

    /// 以磁盘上的旧索引为底合并本次剪切的片段（见 `Cutter::run` 里的说明）。
    ///
    /// 保留规则：① 本次剪出的同名目录**覆盖**老条目；② 老条目的 `file` 仍存在才保留
    /// （片段被删/被移走后不许再指向不存在的文件）；③ `video_source` 优先保留老值
    /// （单条重切拿的是同一份源视频，老值更完整）。最后按 `rank` 排序，结果稳定。
    fn merge_from_disk(path: &Path, video_source: &Path, clips: &[ClipInfo]) -> Self {
        let fresh = Self::new(video_source, clips);
        let old = std::fs::read_to_string(path)
            .ok()
            .and_then(|raw| serde_json::from_str::<Self>(&raw).ok());
        let (video, mut merged) = match old {
            Some(o) => {
                let video = if o.video_source.trim().is_empty() {
                    fresh.video_source.clone()
                } else {
                    o.video_source
                };
                (video, o.clips)
            }
            None => (fresh.video_source.clone(), Vec::new()),
        };
        let out_dir = path.parent().unwrap_or_else(|| Path::new("."));
        merged.retain(|c| {
            if c.file.trim().is_empty() {
                return false;
            }
            if fresh.clips.iter().any(|n| n.directory == c.directory) {
                return false;
            }
            out_dir.join(&c.file).is_file()
        });
        // 老的 publish_config 可能是"全局 has_danmaku"那个 bug 写出来的（指向不存在的
        // `clip.dm.srt` / `clip.srt`）。顺手静默纠正：文件不在就把那个字段清成 None，
        // 别让发布链路拿着一个 404 的路径去取字幕。
        for c in merged.iter_mut() {
            if let Some(dm) = c.danmaku.clone() {
                if !out_dir.join(&dm).is_file() {
                    c.danmaku = None;
                }
            }
            if !c.subtitle.trim().is_empty() && !out_dir.join(&c.subtitle).is_file() {
                c.subtitle = String::new();
            }
        }
        merged.extend(fresh.clips);
        merged.sort_by_key(|c| c.rank);
        Self { video_source: video, clips: merged }
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

    /// **增量剪切不许洗掉发布索引**（2026-10-30 实测）：一键全切逐事件跑 Cutter，
    /// 每跑一次都会写 `publish_config.json`；旧实现只写本次那一条 ⇒ 全切完只剩最后一条。
    /// 这条钉住：老条目保留、同名覆盖、文件已没了的丢掉、`video_source` 不被重切冲掉。
    #[test]
    fn publish_index_merges_incremental_cuts_and_drops_missing_files() {
        let t = TmpDir::new("publish-merge");
        let out = t.0.join("output");
        std::fs::create_dir_all(out.join("clips/clip-01")).unwrap();
        std::fs::create_dir_all(out.join("clips/clip-02")).unwrap();
        std::fs::write(out.join("clips/clip-01/clip.mp4"), b"x").unwrap();
        std::fs::write(out.join("clips/clip-02/clip.mp4"), b"x").unwrap();
        let old = serde_json::json!({
            "video_source": "/old/video.mp4",
            "clips": [
                {"file":"clips/clip-01/clip.mp4","subtitle":"clips/clip-01/clip.srt","danmaku":null,
                 "title":"一","summary":"","event_type":"other","danmaku_popularity":1.0,
                 "rank":1,"start":0.0,"end":1.0,"duration":1.0,"directory":"clip-01"},
                {"file":"clips/gone/clip.mp4","subtitle":"","danmaku":null,
                 "title":"没了","summary":"","event_type":"other","danmaku_popularity":2.0,
                 "rank":2,"start":1.0,"end":2.0,"duration":1.0,"directory":"gone"}
            ]
        });
        std::fs::write(out.join("publish_config.json"), serde_json::to_string(&old).unwrap()).unwrap();

        let info = ClipInfo {
            dir_name: "clip-02".into(),
            title: "二".into(),
            summary: String::new(),
            event_type: crate::types::EventType::Other,
            popularity: 3.0,
            rank: 2,
            start: ts(2.0),
            end: ts(4.0),
            content_secs: 2.0,
            has_danmaku: false,
        };
        let p = PublishConfig::merge_from_disk(
            &out.join("publish_config.json"),
            Path::new("/new/video.mp4"),
            &[info],
        );
        assert_eq!(p.video_source, "/old/video.mp4", "单条重切不许改写 video_source");
        let dirs: Vec<&str> = p.clips.iter().map(|c| c.directory.as_str()).collect();
        assert_eq!(dirs, vec!["clip-01", "clip-02"], "老条目要保留、被删文件的条目要丢、新条目要进");
    }

    // ---------------------------------------------------------------- overwrite（D1）

    fn event_at(label: &str, start: f64, end: f64, rank: u32) -> crate::types::Event {
        crate::types::Event {
            start: crate::types::Timestamp::from_seconds(start),
            end: crate::types::Timestamp::from_seconds(end),
            label: label.into(),
            summary: String::new(),
            event_type: crate::types::EventType::Chat,
            source: crate::types::EventSource::LLM,
            confidence: 1.0,
            popularity: 0.5,
            rank,
            clip_dir: None,
            enabled: true,
            ranges: Vec::new(),
            topic: None,
        }
    }

    fn ctx_with_events_and_overwrite(
        root: &Path,
        events: Vec<crate::types::Event>,
        overwrite: Option<bool>,
    ) -> PipelineContext {
        let mut config = crate::config::Config::default();
        config.cutting.overwrite = overwrite;
        let mut ctx = PipelineContext::new(root.join("video.mp4"), root.join("output"));
        ctx.config = Arc::new(config);
        ctx.events = events;
        ctx
    }

    /// **磁盘预检（批次边界）**：余量低于 `[cutting] min_free_gb` ⇒ 整个阶段停止，
    /// 一条都不剪（预检挡在 ffmpeg 之前）。2026-11-15 事故：全切写穿数据盘，
    /// 外部看门狗把源录播删了 —— 从此宁停不切。
    #[tokio::test]
    async fn cutter_stops_entire_stage_when_disk_precheck_fails() {
        let t = TmpDir::new("disk-guard");
        std::fs::write(t.0.as_path().join("video.mp4"), b"x").unwrap();
        let events = vec![event_at("A", 0.0, 10.0, 1), event_at("B", 10.0, 20.0, 2)];
        let mut ctx = ctx_with_events_and_overwrite(t.0.as_path(), events, None);
        // 阈值拉满（saturating 乘法不回绕）⇒ 预检必失败
        let mut config = crate::config::Config::default();
        config.cutting.min_free_gb = u64::MAX;
        ctx.config = Arc::new(config);

        let err = Cutter
            .run(&mut ctx)
            .await
            .expect_err("阈值 u64::MAX ⇒ 预检必失败");
        // ⚠️ 查**整条错误链**：to_string() 只有最外层 context（"...所在卷预检未过"），
        // "磁盘空间不足"在 FreeSpaceError 那一层。
        assert!(format!("{err:#}").contains("磁盘空间不足"), "{err:#}");
        assert!(
            err.downcast_ref::<crate::disk::FreeSpaceError>().is_some(),
            "必须可 downcast，一键全切靠它停整批"
        );

        // 一条都没剪：clips 下没有任何 clip.mp4
        let clips = t.0.as_path().join("output/clips");
        let cut_any = std::fs::read_dir(&clips)
            .map(|rd| rd.flatten().any(|e| e.path().join("clip.mp4").exists()))
            .unwrap_or(false);
        assert!(!cut_any, "预检失败 ⇒ 不得剪出任何片段");
    }

    /// **批量 / `process`（多条事件）默认不覆盖**；显式 `overwrite = true` 才覆盖。
    #[test]
    fn effective_overwrite_defaults_to_false_for_batch() {
        let t = TmpDir::new("ow-batch");
        let events = vec![event_at("A", 0.0, 10.0, 1), event_at("B", 10.0, 20.0, 2)];
        for ow in [None, Some(false)] {
            let ctx = ctx_with_events_and_overwrite(t.0.as_path(), events.clone(), ow);
            assert!(
                !effective_overwrite(&ctx),
                "overwrite = {ow:?} 且批量（2 条）⇒ 不覆盖"
            );
        }
        let ctx = ctx_with_events_and_overwrite(t.0.as_path(), events.clone(), Some(true));
        assert!(effective_overwrite(&ctx), "显式 true ⇒ 覆盖（老行为）");
    }

    /// **单条剪切永远覆盖**：切片页点"剪切"本身就是显式要求重剪，没有别的通道能表达它
    /// （server 的 `run_single_cut_event` 只塞一个 event 进来）。
    #[test]
    fn effective_overwrite_single_recut_always_overrides() {
        let t = TmpDir::new("ow-single");
        let events = vec![event_at("A", 0.0, 10.0, 1)];
        for ow in [None, Some(false)] {
            let ctx = ctx_with_events_and_overwrite(t.0.as_path(), events.clone(), ow);
            assert!(
                effective_overwrite(&ctx),
                "单条（1 个事件）+ overwrite = {ow:?} ⇒ 仍覆盖"
            );
        }
    }

    /// 非致命结论**三落地**（D9 / P2-5）：tracing（运行日志/CLI 终端）+ `progress.note`
    /// （任务消息通道，server 任务框可见）+ `ctx.pipeline_notes`（程序化消费方）。
    /// server 的 `error()` = 整条任务 fail，"部分失败但其余已出片"绝不能走它。
    #[test]
    fn partial_failures_reach_note_channel_and_pipeline_notes() {
        use std::sync::Mutex;

        #[derive(Default)]
        struct Recording {
            notes: Mutex<Vec<String>>,
        }
        impl crate::progress::ProgressReporter for Recording {
            fn stage_started(&self, _: &str) {}
            fn stage_progress(&self, _: u64, _: u64) {}
            fn stage_completed(&self, _: &str) {}
            fn error(&self, _: &str) {}
            fn note(&self, msg: &str) {
                self.notes.lock().unwrap().push(msg.to_string());
            }
        }

        let rec = Arc::new(Recording::default());
        let mut ctx = PipelineContext::new(PathBuf::from("/tmp/v.mp4"), PathBuf::from("/tmp/out"));
        ctx.progress = rec.clone() as Arc<dyn crate::progress::ProgressReporter>;

        push_pipeline_note(ctx.progress.as_ref(), &mut ctx.pipeline_notes, "2 条片段剪切失败（其余已出片，失败条目不会进发布清单）：a / b".into());
        assert_eq!(ctx.pipeline_notes.len(), 1, "结论要进 ctx.pipeline_notes");
        let got = rec.notes.lock().unwrap().clone();
        assert_eq!(got.len(), 1, "结论要经 progress.note 进任务消息通道");
        assert!(got[0].contains("2 条片段剪切失败"), "{}", got[0]);
    }

    /// `overwrite = false` 且目标已是**这条事件自己的**成片 ⇒ 跳过；
    /// 别人的片段（对不上）不归它管（那是 foreign_clip_conflict 的报错路径）。
    #[test]
    fn skip_reason_skips_only_own_existing_clip() {
        let t = TmpDir::new("ow-skip");
        let own = t.0.join("own");
        std::fs::create_dir_all(&own).unwrap();
        std::fs::write(own.join("clip.mp4"), b"x").unwrap();
        write_meta(&own, serde_json::json!({
            "title": "同一条", "original_start": 0.0, "original_end": 10.0
        }));

        // 自己的成片 + 不覆盖 ⇒ 跳过
        let msg = skip_reason(&own, "同一条", ts(0.0), ts(10.0), false);
        assert!(msg.is_some(), "自己的成片且 overwrite=false ⇒ 跳过");
        assert!(msg.unwrap().contains("overwrite = false"));

        // 显式覆盖 ⇒ 不跳
        assert!(skip_reason(&own, "同一条", ts(0.0), ts(10.0), true).is_none());

        // 没有 clip.mp4（上次剪到一半）⇒ 不跳，照常重剪
        let half = t.0.join("half");
        std::fs::create_dir_all(&half).unwrap();
        write_meta(&half, serde_json::json!({
            "title": "同一条", "original_start": 0.0, "original_end": 10.0
        }));
        assert!(skip_reason(&half, "同一条", ts(0.0), ts(10.0), false).is_none(),
                "没有成片就不算已剪好，要照常重剪");

        // 别人的片段（标题/区间都对不上）⇒ 不在这里跳（foreign_clip_conflict 负责报错）
        let foreign = t.0.join("foreign");
        std::fs::create_dir_all(&foreign).unwrap();
        std::fs::write(foreign.join("clip.mp4"), b"x").unwrap();
        write_meta(&foreign, serde_json::json!({
            "title": "老片段标题", "original_start": 6000.0, "original_end": 6200.0
        }));
        assert!(skip_reason(&foreign, "新事件标题", ts(100.0), ts(300.0), false).is_none(),
                "别人的片段不走这条安静跳过的路径");

        // 老目录没有 clip.json ⇒ 信息不足，照旧重剪
        let legacy = t.0.join("legacy");
        std::fs::create_dir_all(&legacy).unwrap();
        std::fs::write(legacy.join("clip.mp4"), b"x").unwrap();
        assert!(skip_reason(&legacy, "随便", ts(0.0), ts(10.0), false).is_none());
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

    /// `transcript_file` 是**相对工程根**存的：外部/订正字幕必须按工程根找。
    ///
    /// 回归（2026-10-30）：`load_transcript` 曾用裸 `PathBuf::from(p).exists()` —— 那是
    /// "相对进程 CWD"。`yekitrim server` 的 CWD 与项目无关 ⇒ 选中的订正稿读不到，
    /// 静默退回 `output/video.srt`（旧 ASR 稿），成片字幕"有、但是错的"。
    #[test]
    fn load_transcript_resolves_relative_transcript_file_against_project_root() {
        let t = TmpDir::new("tr-base");
        let root = t.0.join("proj");
        let output = root.join("output");
        std::fs::create_dir_all(&output).unwrap();

        // 工程根下的订正稿（相对路径写作 "subs/fixed.srt"）
        std::fs::create_dir_all(root.join("subs")).unwrap();
        std::fs::write(
            root.join("subs/fixed.srt"),
            "1\n00:00:00,000 --> 00:00:01,000\n订正稿\n",
        )
        .unwrap();
        // 兜底的旧 ASR 稿：基准错了就会被静默选中
        std::fs::write(
            output.join("video.srt"),
            "1\n00:00:00,000 --> 00:00:01,000\n旧ASR稿\n",
        )
        .unwrap();

        let mut config = crate::config::Config::default();
        config.project.transcript_file = Some("subs/fixed.srt".into());
        let mut ctx = PipelineContext::new(root.join("video.mp4"), output);
        ctx.config = Arc::new(config);

        let tr = load_transcript(&ctx).expect("相对工程根的字幕必须能读到");
        assert_eq!(
            tr.segments[0].text, "订正稿",
            "必须按工程根解析，不能退回 output/video.srt"
        );
    }

    /// **只有 `output/transcript.json`**（无 transcript_file、无 video.srt）也必须剪出 `clip.srt`。
    ///
    /// 回归（2026-10-30 真实 server 实测）：Cutter 的兜底只认 `output/video.srt`，而 ASR 产物 /
    /// 用户只保留的那份是 `output/transcript.json` ⇒ `load_transcript` 返回 None，片段照剪但
    /// **没有 `clip.srt`**（demo 工程 cut-all：clip-01 只有 clip.dm.srt）。兜底顺序必须与
    /// service 的 `Workspace::transcript_path` 一致：transcript_file → transcript.json → video.srt。
    #[test]
    fn cut_falls_back_to_output_transcript_json_and_emits_clip_srt() {
        let t = TmpDir::new("tr-json-fallback");
        let root = t.0.join("proj");
        let output = root.join("output");
        std::fs::create_dir_all(output.join("clips")).unwrap();

        // 工程只有 output/transcript.json（transcript_file 为空、video.srt 不存在）
        let tr = transcript_of(vec![seg(1.0, 3.0, "只有JSON的工程")]);
        std::fs::write(
            output.join("transcript.json"),
            serde_json::to_string(&tr).unwrap(),
        )
        .unwrap();

        let ctx = PipelineContext::new(root.join("video.mp4"), output.clone());
        let loaded = load_transcript(&ctx).expect("只有 output/transcript.json 时必须能读到转写");
        assert_eq!(loaded.segments[0].text, "只有JSON的工程");

        // 片段里要真的有 clip.srt 且内容正确（`Cutter` 只在 transcript 为 Some 时才写它）
        let clip_dir = output.join("clips/clip-01");
        std::fs::create_dir_all(&clip_dir).unwrap();
        generate_clip_subtitle(&loaded, 1.0, 3.0, &clip_dir.join("clip.srt")).unwrap();
        let body = std::fs::read_to_string(clip_dir.join("clip.srt")).expect("必须写出 clip.srt");
        assert!(body.contains("只有JSON的工程"), "字幕内容不对: {body}");
    }

    /// 外部 `transcript_file` 指向 JSON 时**仍然优先**（不被新增的 `output/transcript.json`
    /// 兜底抢走），且相对基准仍是工程根 —— 上一轮修的路径口径不能被这次改动破坏。
    #[test]
    fn external_json_transcript_file_still_wins_over_output_fallback() {
        let t = TmpDir::new("tr-ext-json");
        let root = t.0.join("proj");
        let output = root.join("output");
        std::fs::create_dir_all(root.join("subs")).unwrap();
        std::fs::create_dir_all(&output).unwrap();

        let external = transcript_of(vec![seg(0.0, 1.0, "外部订正稿")]);
        std::fs::write(
            root.join("subs/fixed.json"),
            serde_json::to_string(&external).unwrap(),
        )
        .unwrap();
        let fallback = transcript_of(vec![seg(0.0, 1.0, "output兜底稿")]);
        std::fs::write(
            output.join("transcript.json"),
            serde_json::to_string(&fallback).unwrap(),
        )
        .unwrap();

        let mut config = crate::config::Config::default();
        config.project.transcript_file = Some("subs/fixed.json".into());
        let mut ctx = PipelineContext::new(root.join("video.mp4"), output);
        ctx.config = Arc::new(config);

        let tr = load_transcript(&ctx).expect("外部 JSON 必须能按工程根读到");
        assert_eq!(tr.segments[0].text, "外部订正稿", "外部 transcript_file 优先于 output 兜底");
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

#[cfg(test)]
mod multi_range_tests {
    use super::*;
    use crate::types::{Transcript, TranscriptSegment};

    fn seg(start: f64, end: f64, text: &str) -> TranscriptSegment {
        TranscriptSegment {
            start,
            end,
            text: text.into(),
            confidence: 1.0,
        }
    }

    fn read(path: &Path) -> String {
        std::fs::read_to_string(path).unwrap_or_default()
    }

    fn dm(start: f64, end: f64, text: &str) -> DanmakuEntry {
        DanmakuEntry {
            start,
            end,
            text: text.into(),
            color: "#ffffff".into(),
            y: 0.0,
            travel_duration: 8.0,
        }
    }

    fn tmp(tag: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let p = std::env::temp_dir().join(format!("yekitrim-mr-{tag}-{}-{nanos}", std::process::id()));
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    /// 段起点：`[0, d0, d0+d1, …]`；单段恒 `[0]`（老路径一个字不变）。
    #[test]
    fn range_offsets_accumulate() {
        assert_eq!(range_offsets(&[EventRange::new(10.0, 25.0)]), vec![0.0]);
        assert_eq!(
            range_offsets(&[
                EventRange::new(10.0, 25.0),
                EventRange::new(100.0, 110.0),
                EventRange::new(200.0, 201.0),
            ]),
            vec![0.0, 15.0, 25.0]
        );
        assert_eq!(range_offsets(&[]), vec![0.0]);
    }

    /// 多段字幕：第 2 段起时间轴**接在第 1 段之后**（不是源时间），空隙里的字幕**不进成片**。
    #[test]
    fn multi_range_subtitle_shifts_per_piece() {
        let dir = tmp("sub");
        let out = dir.join("clip.srt");
        let tr = Transcript {
            segments: vec![
                seg(10.0, 20.0, "第一段"),
                seg(30.0, 40.0, "空隙里的（不该出现）"),
                seg(100.0, 110.0, "第二段"),
            ],
            language: "zh".into(),
            asr_metadata: None,
        };
        generate_clip_subtitle_ranges(
            &tr,
            &[EventRange::new(10.0, 25.0), EventRange::new(100.0, 115.0)],
            &out,
        )
        .unwrap();
        let srt = read(&out);
        assert!(srt.contains("第一段"));
        assert!(srt.contains("第二段"));
        assert!(!srt.contains("空隙里"), "段间空隙的内容不许进成片：{srt}");
        // 第 1 段：源 10-20 → 成片 00:00:00,000-00:00:10,000
        assert!(srt.contains("00:00:00,000 --> 00:00:10,000"), "{srt}");
        // 第 2 段：源 100-110 → 成片 15-25（前一段内容 15s：10→25）
        assert!(srt.contains("00:00:15,000 --> 00:00:25,000"), "{srt}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 多段弹幕同一条偏移口径。
    #[test]
    fn multi_range_danmaku_shifts_per_piece() {
        let dir = tmp("dm");
        let out = dir.join("clip.dm.srt");
        let entries = vec![
            dm(11.0, 12.0, "A"),
            dm(33.0, 34.0, "空隙"),
            dm(101.0, 102.0, "B"),
        ];
        generate_clip_danmaku_ranges(
            &entries,
            &[EventRange::new(10.0, 25.0), EventRange::new(100.0, 115.0)],
            &out,
        )
        .unwrap();
        let srt = read(&out);
        assert!(srt.contains("A") && srt.contains("B"));
        assert!(!srt.contains("空隙"), "段间空隙的弹幕不许进成片：{srt}");
        assert!(srt.contains("00:00:01,000 --> 00:00:02,000"), "{srt}");
        assert!(srt.contains("00:00:16,000 --> 00:00:17,000"), "{srt}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// `clip.json`：多段时写 `ranges`，`duration` = **内容时长**（不是跨度）。
    #[test]
    fn clip_meta_writes_ranges_and_content_duration() {
        let dir = tmp("meta");
        let path = dir.join("clip.json");
        let ranges = vec![EventRange::new(100.0, 120.0), EventRange::new(200.0, 230.0)];
        write_clip_meta(
            &path,
            "标题",
            "摘要",
            &crate::types::EventType::Chat,
            0.5,
            3,
            crate::types::Timestamp::from_seconds(100.0),
            crate::types::Timestamp::from_seconds(230.0),
            &ranges,
        )
        .unwrap();
        let v: serde_json::Value = serde_json::from_str(&read(&path)).unwrap();
        assert_eq!(v["duration"].as_f64().unwrap(), 50.0, "内容时长 = 20 + 30");
        assert_eq!(v["original_start"].as_f64().unwrap(), 100.0);
        assert_eq!(v["original_end"].as_f64().unwrap(), 230.0);
        assert_eq!(v["ranges"].as_array().unwrap().len(), 2);
        assert_eq!(v["ranges"][1]["start"].as_f64().unwrap(), 200.0);

        // 单段（空 ranges）仍写 `end - start`，且**不写** `ranges` 键（老片段格式不变）
        let path2 = dir.join("clip2.json");
        write_clip_meta(
            &path2,
            "标题",
            "摘要",
            &crate::types::EventType::Chat,
            0.5,
            3,
            crate::types::Timestamp::from_seconds(10.0),
            crate::types::Timestamp::from_seconds(40.0),
            &[],
        )
        .unwrap();
        let v2: serde_json::Value = serde_json::from_str(&read(&path2)).unwrap();
        assert_eq!(v2["duration"].as_f64().unwrap(), 30.0);
        assert!(v2.get("ranges").is_none(), "单段不许多出 ranges 键");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// **真剪一次**（需要 ffmpeg）：两段 → 拼接，时长 ≈ 两段之和，且 `.parts/` 清干净。
    ///
    /// ```text
    /// VT_CUT_SRC=<任意 mp4> cargo test -p yekitrim-core --lib diagnose_multi_range_cut -- --ignored --nocapture
    /// ```
    #[test]
    #[ignore]
    fn diagnose_multi_range_cut() {
        let Ok(src) = std::env::var("VT_CUT_SRC") else {
            eprintln!("用法: VT_CUT_SRC=<mp4> cargo test ... diagnose_multi_range_cut -- --ignored --nocapture");
            return;
        };
        let dir = tmp("cut");
        let clip_dir = dir.join("clip-01");
        std::fs::create_dir_all(&clip_dir).unwrap();
        let out = clip_dir.join("clip.mp4");
        let ranges = vec![EventRange::new(0.5, 2.0), EventRange::new(3.0, 4.5)];
        let started = std::time::Instant::now();
        cut_event(Path::new(&src), &ranges, &out).expect("多段剪切");
        println!("剪切用时 {:.1}s", started.elapsed().as_secs_f64());
        assert!(out.exists(), "成片没生成");
        assert!(!clip_dir.join(".parts").exists(), ".parts 必须清干净");
        let probe = crate::process::cmd("ffprobe")
            .args(["-v", "error", "-show_entries", "format=duration", "-of", "default=nw=1:nk=1"])
            .arg(&out)
            .output()
            .expect("ffprobe");
        let dur: f64 = String::from_utf8_lossy(&probe.stdout).trim().parse().unwrap_or(0.0);
        println!("成片时长 {dur:.2}s（期望 ≈ 3.0s：1.5 + 1.5）");
        assert!((dur - 3.0).abs() < 0.5, "时长不对: {dur}");
        // 音视频轨都要在（拼接最常见的翻车就是"只剩画面"）
        let streams = crate::process::cmd("ffprobe")
            .args([
                "-v", "error", "-show_entries", "stream=codec_type,codec_name",
                "-of", "csv=p=0",
            ])
            .arg(&out)
            .output()
            .expect("ffprobe streams");
        let streams = String::from_utf8_lossy(&streams.stdout);
        println!("轨道: {}", streams.trim().lines().collect::<Vec<_>>().join(" | "));
        // ⚠️ `-show_entries stream=codec_type,codec_name` 的输出顺序是 **codec_type 在前**？
        // 实测是 `h264,video`（name 在前）—— 所以取**最后一个**字段判类型，别取第一个。
        let kinds: Vec<&str> = streams
            .lines()
            .filter_map(|l| l.split(',').next_back())
            .collect();
        assert!(kinds.contains(&"video"), "没有视频轨: {streams}");
        assert!(kinds.contains(&"audio"), "没有音频轨: {streams}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
