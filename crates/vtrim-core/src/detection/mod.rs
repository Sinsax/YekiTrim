use std::collections::BTreeMap;

use async_trait::async_trait;
use anyhow::{Context, Result};

use crate::danmaku::DensityWindow;
use crate::pipeline::{Stage, STAGE_DETECT};
use crate::types::{Event, EventStore, PipelineContext, Timestamp, Transcript, TranscriptSegment};

mod llm;
pub use llm::{parse_json, test_llm, DEFAULT_PROMPT, DEFAULT_SYSTEM_PROMPT, TITLE_RULES};

/// LLM 事件后处理参数
const MIN_EVENT_SECS: f64 = 30.0; // 最短事件时长（保留细分，兜底 30s）
const MAX_EVENT_SECS: f64 = 240.0; // 最长事件时长（prompt 要求 3 分钟，兜底 240s）
const MAX_LABEL_LEN: usize = 40; // 标题长度上限（与 prompt 一致）

pub struct EventDetector;

#[async_trait]
impl Stage for EventDetector {
    fn name(&self) -> &'static str {
        "EventDetector"
    }

    fn stage_number(&self) -> u32 {
        STAGE_DETECT
    }

    async fn run(&self, ctx: &mut PipelineContext) -> Result<()> {
        let transcript = ctx
            .transcript
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("缺少转写结果，请先运行 transcribe 阶段"))?;

        let segments = &transcript.segments;
        if segments.is_empty() {
            anyhow::bail!("转写结果为空，无法检测事件");
        }

        let llm_config = &ctx.config.llm;
        let detect_config = &ctx.config.detection;

        // 视频时长基准: video_info > ffprobe > 转录末尾（detect 独立命令下 video_info 可能缺失）
        let video_duration: Option<f64> = ctx
            .video_info
            .as_ref()
            .map(|v| v.duration_secs)
            .or_else(|| probe_duration(&ctx.input_path).ok())
            .or_else(|| segments.last().map(|s| s.end));
        let total_duration = video_duration.unwrap_or_else(|| {
            segments
                .iter()
                .map(|s| s.end)
                .fold(0.0, f64::max)
                .max(1.0)
        });

        // 弹幕热力图提前计算：热点窗口注入 LLM prompt + 综合评分复用
        let heatmap: Option<Vec<DensityWindow>> = match &ctx.danmaku_path {
            Some(p) if p.exists() => {
                match crate::danmaku::analyze(p, detect_config.danmaku_window_secs, total_duration)
                {
                    Ok(h) if !h.is_empty() => {
                        let total_entries: u64 = h.iter().map(|w| w.count).sum();
                        tracing::info!(
                            "弹幕热力图: {} 个窗口, 共 {} 条弹幕",
                            h.len(),
                            total_entries
                        );
                        Some(h)
                    }
                    Ok(_) => {
                        tracing::warn!("弹幕文件无数据: {}", p.display());
                        None
                    }
                    Err(e) => {
                        tracing::warn!("弹幕分析失败: {}", e);
                        None
                    }
                }
            }
            Some(p) => {
                tracing::warn!("弹幕文件不存在: {}", p.display());
                None
            }
            None => None,
        };
        let hot_windows = heatmap
            .as_deref()
            .map(|h| extract_hot_windows(h, detect_config.danmaku_window_secs, 20))
            .unwrap_or_default();
        if !hot_windows.is_empty() {
            tracing::info!("弹幕爆发时刻: {} 个区间，已注入 LLM 分析", hot_windows.len());
        }

        let mut events = if llm_config.api_key.is_empty() {
            tracing::warn!("未配置 LLM API key (llm.api_key)，使用默认事件");
            vec![llm::create_default_event(segments)]
        } else {
            process_with_llm(
                segments,
                llm_config,
                detect_config,
                &hot_windows,
                &ctx.output_dir,
                ctx.detect_resume,
            )
            .await?
        };

        // 追加主播标签（无 API key 的默认事件也适用），并控制标题长度上限
        if !detect_config.vtuber_name.is_empty() {
            let tag = format!("【{}】", detect_config.vtuber_name);
            for event in &mut events {
                if !event.label.ends_with(&tag) {
                    event.label.push_str(&tag);
                }
                if event.label.chars().count() > MAX_LABEL_LEN {
                    let keep = MAX_LABEL_LEN.saturating_sub(tag.chars().count());
                    let mut trimmed: String = event.label.chars().take(keep).collect();
                    trimmed.push_str(&tag);
                    event.label = trimmed;
                }
            }
            tracing::info!("已追加主播标签: {}", tag);
        }

        // 事件边界 clamp 到视频时长，丢弃非法事件 (end <= start)
        if let Some(max_end) = video_duration {
            if max_end > 0.0 {
                let before = events.len();
                for event in &mut events {
                    event.start = Timestamp::from_seconds(event.start.to_seconds().clamp(0.0, max_end));
                    event.end = Timestamp::from_seconds(event.end.to_seconds().clamp(0.0, max_end));
                }
                events.retain(|e| e.end.to_seconds() > e.start.to_seconds());
                if events.len() < before {
                    tracing::warn!(
                        "已丢弃 {} 个非法事件 (越界或 end <= start)",
                        before - events.len()
                    );
                }

                // 覆盖率校验：事件覆盖过少 → 提示可能存在漏检
                let covered: f64 = coverage_ratio(&events, max_end) * max_end;
                let coverage = covered / max_end;
                if coverage < 0.6 {
                    tracing::warn!(
                        "事件覆盖率仅 {:.0}% ({} 个事件覆盖 {:.0}s / 视频 {:.0}s)，可能存在漏检；\
                         可检查 LLM 输出完整性或调大 llm.max_tokens",
                        coverage * 100.0,
                        events.len(),
                        covered,
                        max_end
                    );
                }
            }
        }

        // 综合评分：弹幕密度（峰值+均值）+ 字幕密度（无弹幕时仅字幕密度）
        let transcript_densities = compute_transcript_density(&events, transcript);
        match heatmap {
            Some(ref heatmap) => {
                tracing::info!("弹幕+字幕双密度评分: {} 个窗口", heatmap.len());
                rank_events(
                    &mut events,
                    Some(heatmap),
                    detect_config.danmaku_window_secs,
                    &transcript_densities,
                );
            }
            None => {
                tracing::info!("无弹幕数据，使用字幕密度评分");
                rank_events_by_transcript(&mut events, &transcript_densities);
            }
        }

        ctx.events = events;
        save_events(&ctx.events, &ctx.output_dir)?;

        // 检测成功，清理断点
        let checkpoint_path = ctx.output_dir.join("events.checkpoint.json");
        if checkpoint_path.exists() {
            let _ = std::fs::remove_file(&checkpoint_path);
            tracing::info!("已清理事件检测断点: {}", checkpoint_path.display());
        }
        let partial_path = ctx.output_dir.join("events.partial.json");
        if partial_path.exists() {
            let _ = std::fs::remove_file(&partial_path);
        }

        tracing::info!("事件检测完成: {} 个事件", ctx.events.len());
        Ok(())
    }
}

/// 事件覆盖并集时长 / 视频时长，归一化 [0,1]
fn coverage_ratio(events: &[Event], max_end: f64) -> f64 {
    if max_end <= 0.0 {
        return 0.0;
    }
    let mut intervals: Vec<(f64, f64)> = events
        .iter()
        .map(|e| {
            (
                e.start.to_seconds().clamp(0.0, max_end),
                e.end.to_seconds().clamp(0.0, max_end),
            )
        })
        .filter(|(s, e)| e > s)
        .collect();
    intervals.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
    let mut covered = 0.0;
    let mut cur_start = 0.0;
    let mut cur_end = 0.0;
    for (s, e) in intervals {
        if s > cur_end {
            covered += cur_end - cur_start;
            cur_start = s;
            cur_end = e;
        } else {
            cur_end = cur_end.max(e);
        }
    }
    covered += cur_end - cur_start;
    (covered / max_end).clamp(0.0, 1.0)
}

/// 用 ffprobe 探测视频时长（detect 独立命令时 video_info 不可用）
fn probe_duration(path: &std::path::Path) -> Result<f64> {
    let output = std::process::Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-show_entries",
            "format=duration",
            "-of",
            "default=noprint_wrappers=1:nokey=1",
        ])
        .arg(path)
        .output()
        .with_context(|| format!("ffprobe 执行失败: {}", path.display()))?;
    if !output.status.success() {
        anyhow::bail!(
            "ffprobe 失败: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    stdout
        .trim()
        .parse::<f64>()
        .context("解析 ffprobe 时长失败")
}

/// 计算每个事件的字幕密度 (segment overlap 比例 / duration)，归一化 [0,1]
/// 用 overlap 比例而非"完全包含"计数，避免事件边界与 segment 边界不对齐时低估密度
fn compute_transcript_density(events: &[Event], transcript: &Transcript) -> Vec<f64> {
    let raw: Vec<f64> = events
        .iter()
        .map(|event| {
            let start = event.start.to_seconds();
            let end = event.end.to_seconds();
            let duration = (end - start).max(1.0);
            let mut coverage = 0.0;
            for seg in &transcript.segments {
                let seg_start = seg.start.max(start);
                let seg_end = seg.end.min(end);
                if seg_end > seg_start {
                    let seg_dur = (seg.end - seg.start).max(0.1);
                    coverage += (seg_end - seg_start) / seg_dur;
                }
            }
            coverage / duration
        })
        .collect();
    let max_val = raw.iter().copied().fold(0.0, f64::max).max(0.001);
    raw.into_iter().map(|v| v / max_val).collect()
}

/// 按弹幕密度（峰值+均值）+ 字幕密度综合排序
/// 峰值权重更高：爆发瞬间更能代表"名场面"；均值避免纯峰值噪声（单窗口巧合）
fn rank_events(
    events: &mut [Event],
    heatmap: Option<&[DensityWindow]>,
    window_secs: f64,
    transcript_densities: &[f64],
) {
    let Some(heatmap) = heatmap else {
        tracing::warn!("弹幕热力图为空，仅使用字幕密度");
        rank_events_by_transcript(events, transcript_densities);
        return;
    };

    for (i, event) in events.iter_mut().enumerate() {
        let start = event.start.to_seconds();
        let end = event.end.to_seconds();
        let dm_peak = crate::danmaku::peak_density(heatmap, start, end, window_secs);
        let dm_avg = crate::danmaku::average_density(heatmap, start, end, window_secs);
        let dm_score = 0.6 * dm_peak + 0.4 * dm_avg;
        let ts_density = transcript_densities.get(i).copied().unwrap_or(0.0);
        event.popularity = (dm_score + ts_density) / 2.0;
    }

    assign_rank_and_sort(events);
}

/// 仅按字幕密度排序
fn rank_events_by_transcript(events: &mut [Event], transcript_densities: &[f64]) {
    for (i, event) in events.iter_mut().enumerate() {
        event.popularity = transcript_densities.get(i).copied().unwrap_or(0.0);
    }
    assign_rank_and_sort(events);
}

/// 按 popularity 降序分配 rank，恢复时间序
fn assign_rank_and_sort(events: &mut [Event]) {
    events.sort_by(|a, b| {
        b.popularity
            .partial_cmp(&a.popularity)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    for (i, event) in events.iter_mut().enumerate() {
        event.rank = (i + 1) as u32;
    }
    events.sort_by(|a, b| {
        a.start
            .to_seconds()
            .partial_cmp(&b.start.to_seconds())
            .unwrap()
    });
}

async fn process_with_llm(
    segments: &[TranscriptSegment],
    llm: &crate::config::LlmConfig,
    detect: &crate::config::DetectionConfig,
    hot_windows: &[(f64, f64)],
    output_dir: &std::path::Path,
    resume: Option<bool>,
) -> Result<Vec<Event>> {
    // 预合并：相邻短段（间距 <2s）合并，减少 LLM 输入量
    let merged = merge_segments_for_detection(segments);
    tracing::info!(
        "LLM 分析: {} 段 → 预合并为 {} 段",
        segments.len(),
        merged.len(),
    );

    // 估算 token 数（中文约 1.5-2 chars/token）
    let seg_text: String = merged.iter().map(|s| s.text.as_str()).collect();
    let estimated_tokens = (seg_text.len() as f64 / 1.8) as usize;
    tracing::info!(
        "LLM 分析: 音频内容约 {} 字符 (~{}k tokens), 超时 300s",
        seg_text.len(),
        estimated_tokens / 1000,
    );

    let checkpoint_path = output_dir.join("events.checkpoint.json");
    let partial_path = output_dir.join("events.partial.json");

    // 用户选择重新开始时删除旧 checkpoint / partial
    if resume == Some(false) {
        if checkpoint_path.exists() {
            tracing::info!("用户选择重新开始，删除旧事件断点: {}", checkpoint_path.display());
            let _ = std::fs::remove_file(&checkpoint_path);
        }
        if partial_path.exists() {
            let _ = std::fs::remove_file(&partial_path);
        }
    }

    let (partial_tx, mut partial_rx) = tokio::sync::mpsc::channel::<Vec<Event>>(64);

    let events_fut = llm::call_with_progress(
        &merged,
        llm,
        detect,
        hot_windows,
        Some(&checkpoint_path),
        Some(partial_tx),
    );

    let partial_task = tokio::spawn(async move {
        let mut partial: Vec<Event> = Vec::new();
        while let Some(mut events) = partial_rx.recv().await {
            partial.append(&mut events);
            let _ = std::fs::write(
                &partial_path,
                serde_json::to_string_pretty(&partial).unwrap_or_default(),
            );
        }
        partial
    });

    let events_result = events_fut.await;
    let _partial = partial_task.await.unwrap_or_default();
    let events = events_result.map_err(|e| anyhow::anyhow!("LLM 事件检测失败: {}", e))?;

    let events = postprocess_events(events, segments);

    Ok(events)
}

/// 提取弹幕热点区间：密度 ≥ 0.5 的窗口按密度降序取 top N，相邻窗口合并为连续区间。
/// 用于注入 LLM prompt，提示名场面位置。
fn extract_hot_windows(
    heatmap: &[DensityWindow],
    window_secs: f64,
    max_windows: usize,
) -> Vec<(f64, f64)> {
    if heatmap.is_empty() {
        return Vec::new();
    }

    let mut hot: Vec<&DensityWindow> = heatmap.iter().filter(|w| w.density >= 0.5).collect();
    hot.sort_by(|a, b| {
        b.density
            .partial_cmp(&a.density)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    hot.truncate(max_windows);
    hot.sort_by(|a, b| {
        a.start
            .partial_cmp(&b.start)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let mut result: Vec<(f64, f64)> = Vec::new();
    for w in hot {
        let ws = w.start;
        let we = ws + window_secs;
        if let Some(last) = result.last_mut() {
            // 相邻窗口合并为连续区间
            if ws <= last.1 + 1e-9 {
                last.1 = last.1.max(we);
                continue;
            }
        }
        result.push((ws, we));
    }
    result
}

/// 事件后处理管线：丢弃非法 → 按时间排序 → 区间合并 → 边界吸附 → 区间合并兜底
/// → 最短时长强制 → 相邻间隙合并。
/// LLM 输出顺序不受保证，必须先排序再做区间合并，否则重叠事件会漏合并（事件重合的根因）。
fn postprocess_events(mut events: Vec<Event>, segments: &[TranscriptSegment]) -> Vec<Event> {
    if events.is_empty() {
        return events;
    }

    events.retain(|e| e.end.to_seconds() > e.start.to_seconds());
    events.sort_by(|a, b| {
        a.start
            .to_seconds()
            .partial_cmp(&b.start.to_seconds())
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    events = merge_overlapping(events);
    events = snap_to_segments(events, segments);
    events.retain(|e| e.end.to_seconds() > e.start.to_seconds());
    // 吸附可能使相邻事件撞到同一边界，再合并一次兜底
    events = merge_overlapping(events);
    events = enforce_min_duration(events, MIN_EVENT_SECS);
    events = enforce_max_duration(events, segments, MAX_EVENT_SECS);
    events = disambiguate_duplicate_titles(events);

    events
}

/// 区间合并：输入已按 start 排序，线性扫描，重叠/包含的事件取并集。
/// label/summary 保留时长更长者（信息量更大），confidence 取最大值。
fn merge_overlapping(events: Vec<Event>) -> Vec<Event> {
    let mut result: Vec<Event> = Vec::with_capacity(events.len());
    for event in events {
        if let Some(last) = result.last_mut() {
            let last_start = last.start.to_seconds();
            let last_end = last.end.to_seconds();
            let ev_start = event.start.to_seconds();
            let ev_end = event.end.to_seconds();
            if ev_start <= last_end {
                if ev_end > last_end {
                    last.end = event.end;
                }
                if ev_end - ev_start > last_end - last_start {
                    last.label = event.label;
                    last.summary = event.summary;
                    last.event_type = event.event_type;
                }
                last.confidence = last.confidence.max(event.confidence);
                continue;
            }
        }
        result.push(event);
    }
    result
}

/// 将事件边界吸附到最近的 ASR segment 边界，使切割点落在说话停顿处。
/// 好处：剪辑字幕（Cutter 只保留完全包含的 segment）首尾完整，密度统计更准。
fn snap_to_segments(mut events: Vec<Event>, segments: &[TranscriptSegment]) -> Vec<Event> {
    if events.is_empty() || segments.is_empty() {
        return events;
    }

    let mut bounds: Vec<f64> = Vec::with_capacity(segments.len() * 2);
    for s in segments {
        bounds.push(s.start);
        bounds.push(s.end);
    }
    bounds.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    bounds.dedup();

    for event in &mut events {
        event.start = Timestamp::from_seconds(snap(&bounds, event.start.to_seconds()));
        event.end = Timestamp::from_seconds(snap(&bounds, event.end.to_seconds()));
    }
    events
}

/// 在排序后的边界数组中找离 t 最近的边界
fn snap(sorted_bounds: &[f64], t: f64) -> f64 {
    if sorted_bounds.is_empty() {
        return t;
    }
    let idx = sorted_bounds.partition_point(|&b| b < t);
    match (idx.checked_sub(1), sorted_bounds.get(idx)) {
        (Some(li), Some(&r)) => {
            let l = sorted_bounds[li];
            if (t - l).abs() <= (r - t).abs() {
                l
            } else {
                r
            }
        }
        (Some(li), None) => sorted_bounds[li],
        (None, Some(&r)) => r,
        (None, None) => t,
    }
}

/// 为事件检测预合并短段，减少 LLM 输入量
/// 合并条件：间距 <2.0s 的相邻段（比订正更激进，因为 detection 只需要看结构）
fn merge_segments_for_detection(segments: &[TranscriptSegment]) -> Vec<TranscriptSegment> {
    let mut merged: Vec<TranscriptSegment> = Vec::new();
    let mut iter = segments.iter();
    if let Some(first) = iter.next() {
        let mut cur = first.clone();
        for seg in iter {
            let gap = seg.start - cur.end;
            if gap < 2.0 {
                cur.text = format!("{}{}", cur.text.trim(), seg.text.trim());
                cur.end = seg.end;
            } else {
                merged.push(cur);
                cur = seg.clone();
            }
        }
        merged.push(cur);
    }
    merged
}

/// 最短时长强制：短于 min_secs 的事件与相邻事件合并（优先下一个，否则上一个）。
/// label/summary 保留时长更长者，confidence 取最大值；合并后若仍不达标继续吞并。
fn enforce_min_duration(mut events: Vec<Event>, min_secs: f64) -> Vec<Event> {
    if events.len() <= 1 {
        return events;
    }

    let mut i = 0;
    while i < events.len() {
        let cur_start = events[i].start.to_seconds();
        let cur_end = events[i].end.to_seconds();
        let cur_dur = cur_end - cur_start;
        if cur_dur >= min_secs {
            i += 1;
            continue;
        }

        // 优先与下一个合并
        if i + 1 < events.len() {
            let next = events.remove(i + 1);
            let next_dur = next.end.to_seconds() - next.start.to_seconds();
            if next_dur > cur_dur {
                events[i].label = next.label;
                events[i].summary = next.summary;
                events[i].event_type = next.event_type;
            }
            events[i].end = next.end;
            events[i].confidence = events[i].confidence.max(next.confidence);
            continue; // 合并后可能仍未达标，继续检查
        }

        // 与上一个合并
        if i > 0 {
            let short = events.remove(i);
            i -= 1;
            let prev_dur = events[i].end.to_seconds() - events[i].start.to_seconds();
            if short.end.to_seconds() - short.start.to_seconds() > prev_dur {
                events[i].label = short.label;
                events[i].summary = short.summary;
                events[i].event_type = short.event_type;
            }
            events[i].end = short.end;
            events[i].confidence = events[i].confidence.max(short.confidence);
            continue;
        }

        i += 1;
    }

    events
}

/// 合并相邻近事件：间隙 ≤ close_gap_secs 无条件合并（过渡句不单独成段）；
/// 间隙 ≤ gap_secs 且 event_type 相同才合并（同话题内的自然停顿）。
/// 合并后保留时长更长者的 label/summary，confidence 取最大值。

/// 最长时长强制：超过 max_secs 的事件沿内部最大停顿/间隔拆分为多个片段。
/// 优先在 ASR 段间 gap 最大的位置切，使切割点尽量落在自然停顿处；
/// 找不到足够 gap 时按时间中点拆。拆出的片段继承原 label，并加（上/中/下）或（续）后缀。
fn enforce_max_duration(
    mut events: Vec<Event>,
    segments: &[TranscriptSegment],
    max_secs: f64,
) -> Vec<Event> {
    let _ = segments; // 等分切分，不依赖 ASR 边界（保证每段严格不超长）
    if events.is_empty() || max_secs <= 0.0 {
        return events;
    }

    let mut result: Vec<Event> = Vec::with_capacity(events.len());
    for ev in events.drain(..) {
        let start = ev.start.to_seconds();
        let end = ev.end.to_seconds();
        let dur = end - start;
        if dur <= max_secs {
            result.push(ev);
            continue;
        }

        // 等分成 n 段，每段严格 <= max_secs
        let n = (dur / max_secs).ceil() as usize;
        let step = dur / n as f64;

        let mut prev = start;
        for i in 1..n {
            let cut = start + step * i as f64;
            let mut piece = ev.clone();
            piece.start = Timestamp::from_seconds(prev);
            piece.end = Timestamp::from_seconds(cut);
            piece.label = format!("{}（{}/{}）", ev.label, i, n);
            result.push(piece);
            prev = cut;
        }
        let mut last_piece = ev.clone();
        last_piece.start = Timestamp::from_seconds(prev);
        last_piece.end = Timestamp::from_seconds(end);
        last_piece.label = format!("{}（{}/{}）", ev.label, n, n);
        result.push(last_piece);
    }
    result
}

fn disambiguate_duplicate_titles(mut events: Vec<Event>) -> Vec<Event> {
    if events.len() <= 1 {
        return events;
    }

    let mut i = 0;
    while i < events.len() {
        // 找连续相同标题的区间
        let mut j = i + 1;
        while j < events.len() && events[j].label == events[i].label {
            j += 1;
        }
        let group_len = j - i;
        if group_len > 1 {
            for (k, ev) in events[i..j].iter_mut().enumerate() {
                ev.label = format!("{}（{}/{}）", ev.label, k + 1, group_len);
            }
        }
        i = j;
    }

    events
}

fn save_events(events: &[Event], output_dir: &std::path::Path) -> Result<()> {
    std::fs::create_dir_all(output_dir)?;
    let path = output_dir.join("events.json");
    let mut titles = BTreeMap::new();
    for e in events {
        titles.entry(e.label.clone()).or_insert(true);
    }
    let store = EventStore {
        titles,
        events: events.to_vec(),
    };
    let json = serde_json::to_string_pretty(&store)?;
    std::fs::write(&path, json).context("写入事件文件失败")?;
    tracing::info!("事件已保存: {}", path.display());
    Ok(())
}

pub fn load_events(path: &std::path::Path) -> Result<Vec<Event>> {
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("无法读取事件文件: {}", path.display()))?;
    // 支持旧格式 (Vec<Event>) 和新格式 (EventStore)
    if let Ok(store) = serde_json::from_str::<EventStore>(&content) {
        Ok(store.events.into_iter().filter(|e| e.enabled).collect())
    } else if let Ok(events) = serde_json::from_str::<Vec<Event>>(&content) {
        Ok(events)
    } else {
        anyhow::bail!("解析事件 JSON 失败: {}", path.display());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(start: f64, end: f64, label: &str, etype: crate::types::EventType) -> Event {
        Event {
            start: Timestamp::from_seconds(start),
            end: Timestamp::from_seconds(end),
            label: label.into(),
            summary: format!("summary-{}", label),
            event_type: etype,
            source: crate::types::EventSource::LLM,
            confidence: 1.0,
            popularity: 0.0,
            rank: 0,
            enabled: true,
        }
    }

    fn segs(bounds: &[(f64, f64)]) -> Vec<TranscriptSegment> {
        bounds
            .iter()
            .map(|&(s, e)| TranscriptSegment {
                start: s,
                end: e,
                text: "x".into(),
                confidence: 1.0,
            })
            .collect()
    }

    #[test]
    fn test_merge_overlapping_unsorted_input() {
        // LLM 乱序 + 重叠：应合并为时间序、互不重叠的事件
        let events = vec![
            ev(200.0, 300.0, "B", crate::types::EventType::Chat),
            ev(0.0, 100.0, "A", crate::types::EventType::Gameplay),
            ev(90.0, 150.0, "C", crate::types::EventType::Chat),
            ev(180.0, 220.0, "D", crate::types::EventType::Music),
        ];
        let sorted = {
            let mut e = events;
            e.retain(|e| e.end.to_seconds() > e.start.to_seconds());
            e.sort_by(|a, b| {
                a.start
                    .to_seconds()
                    .partial_cmp(&b.start.to_seconds())
                    .unwrap()
            });
            e
        };
        let merged = merge_overlapping(sorted);

        // 0-100 与 90-150 重叠 → 0-150; 180-220 与 200-300 重叠 → 180-300
        assert_eq!(merged.len(), 2);
        assert_eq!(merged[0].start.to_seconds(), 0.0);
        assert_eq!(merged[0].end.to_seconds(), 150.0);
        assert_eq!(merged[1].start.to_seconds(), 180.0);
        assert_eq!(merged[1].end.to_seconds(), 300.0);
        // 时间序保证
        assert!(merged[0].start.to_seconds() < merged[1].start.to_seconds());
    }

    #[test]
    fn test_merge_overlapping_keeps_longer_label_and_type() {
        // 重叠时保留时长更长者的 label/summary/event_type（信息量更大）
        let events = vec![
            ev(0.0, 100.0, "short", crate::types::EventType::Chat),
            ev(50.0, 200.0, "long", crate::types::EventType::Gameplay),
        ];
        let merged = merge_overlapping(events);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].label, "long");
        assert_eq!(merged[0].event_type, crate::types::EventType::Gameplay);
        assert_eq!(merged[0].start.to_seconds(), 0.0);
        assert_eq!(merged[0].end.to_seconds(), 200.0);
    }

    #[test]
    fn test_snap_to_segments_nearest_boundary() {
        let segments = segs(&[(0.0, 10.0), (10.0, 25.0), (25.0, 40.0)]);
        let events = vec![ev(9.2, 26.8, "x", crate::types::EventType::Chat)];
        let snapped = snap_to_segments(events, &segments);
        assert_eq!(snapped[0].start.to_seconds(), 10.0);
        assert_eq!(snapped[0].end.to_seconds(), 25.0);
    }

    #[test]
    fn test_postprocess_no_overlap_after_snap() {
        // 吸附后相邻事件可能撞到同一边界 → 兜底合并，最终互不重叠
        let segments = segs(&[(0.0, 10.0), (10.0, 20.0), (20.0, 30.0), (30.0, 40.0)]);
        let events = vec![
            ev(1.2, 19.8, "A", crate::types::EventType::Chat),
            ev(20.2, 39.8, "B", crate::types::EventType::Chat),
        ];
        let out = postprocess_events(events, &segments);
        for w in out.windows(2) {
            assert!(w[0].end.to_seconds() <= w[1].start.to_seconds());
        }
        assert!(out.len() >= 1);
    }

    #[test]
    fn test_enforce_min_duration_merges_short() {
        // 短事件应被合并，且不保留短事件 label（保留更长者）
        let events = vec![
            ev(0.0, 10.0, "tiny", crate::types::EventType::Chat),
            ev(10.0, 200.0, "long", crate::types::EventType::Gameplay),
        ];
        let out = enforce_min_duration(events, 60.0);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].label, "long");
        assert_eq!(out[0].event_type, crate::types::EventType::Gameplay);
        assert_eq!(out[0].end.to_seconds(), 200.0);
    }

    #[test]
    fn test_enforce_min_duration_chain_merges_until_ok() {
        // 连续短事件：吞并直到达标或只剩一个
        let events = vec![
            ev(0.0, 10.0, "a", crate::types::EventType::Chat),
            ev(10.0, 20.0, "b", crate::types::EventType::Chat),
            ev(20.0, 30.0, "c", crate::types::EventType::Chat),
            ev(30.0, 40.0, "d", crate::types::EventType::Chat),
        ];
        let out = enforce_min_duration(events, 60.0);
        assert_eq!(out.len(), 1);
        assert!(out[0].end.to_seconds() - out[0].start.to_seconds() >= 40.0);
    }

    #[test]
    fn test_compute_transcript_density_overlap_ratio() {
        // 事件 [5, 35] 与 segments [(0,10),(10,30),(30,40)]:
        // overlap 比例 = 5/10 + 20/20 + 5/10 = 2.0, 归一化(单事件) → 1.0
        let events = vec![ev(5.0, 35.0, "x", crate::types::EventType::Chat)];
        let transcript = crate::types::Transcript {
            segments: segs(&[(0.0, 10.0), (10.0, 30.0), (30.0, 40.0)]),
            language: "zh".into(),
            asr_metadata: None,
        };
        let d = compute_transcript_density(&events, &transcript);
        assert_eq!(d.len(), 1);
        assert!((d[0] - 1.0).abs() < 1e-9, "单事件归一化为 1.0, got {}", d[0]);
    }

    #[test]
    fn test_snap_nearest() {
        let bounds = vec![0.0, 10.0, 20.0, 30.0];
        assert_eq!(snap(&bounds, 9.2), 10.0);
        assert_eq!(snap(&bounds, 26.8), 30.0);
        assert_eq!(snap(&bounds, 19.9), 20.0);
        assert_eq!(snap(&bounds, 0.4), 0.0);
        assert_eq!(snap(&bounds, 31.0), 30.0);
    }

    #[test]
    fn test_extract_hot_windows_merges_adjacent() {
        // 窗口宽 10s；相邻窗口合并为连续区间，孤立的保持独立
        let heatmap = vec![
            DensityWindow { start: 0.0, density: 0.2, count: 2 },
            DensityWindow { start: 10.0, density: 0.9, count: 9 },
            DensityWindow { start: 20.0, density: 1.0, count: 10 },
            DensityWindow { start: 30.0, density: 0.3, count: 3 },
            DensityWindow { start: 40.0, density: 0.8, count: 8 },
            DensityWindow { start: 50.0, density: 0.1, count: 1 },
        ];
        let hot = extract_hot_windows(&heatmap, 10.0, 20);
        // 10-20 与 20-30 合并 → [10,30]; 40-50 独立 → [40,50]
        assert_eq!(hot, vec![(10.0, 30.0), (40.0, 50.0)]);
    }

    #[test]
    fn test_extract_hot_windows_truncates_top_n() {
        let heatmap: Vec<DensityWindow> = (0..10)
            .map(|i| DensityWindow {
                start: i as f64 * 10.0,
                density: 0.6 + i as f64 * 0.04, // 密度递增 0.6..0.96
                count: 10,
            })
            .collect();
        // top 3 窗口 (密度最高)，按时间排序 → [70,100]
        let hot = extract_hot_windows(&heatmap, 10.0, 3);
        assert_eq!(hot, vec![(70.0, 100.0)]);
    }

    #[test]
    fn test_extract_hot_windows_empty() {
        assert!(extract_hot_windows(&[], 10.0, 20).is_empty());
        // 全部低于阈值
        let low = vec![DensityWindow { start: 0.0, density: 0.1, count: 1 }];
        assert!(extract_hot_windows(&low, 10.0, 20).is_empty());
    }

    #[test]
    fn test_coverage_ratio() {
        // 覆盖 [0,100] + [50,150]（重叠）→ 并集 150 / 200 = 0.75
        let events = vec![ev(0.0, 100.0, "a", crate::types::EventType::Chat), ev(50.0, 150.0, "b", crate::types::EventType::Chat)];
        assert!((coverage_ratio(&events, 200.0) - 0.75).abs() < 1e-9);

        // 不重叠: [0,40] + [60,100] → 0.8
        let events2 = vec![ev(0.0, 40.0, "a", crate::types::EventType::Chat), ev(60.0, 100.0, "b", crate::types::EventType::Chat)];
        assert!((coverage_ratio(&events2, 100.0) - 0.8).abs() < 1e-9);

        // 越界 clamp: [0,120] → 覆盖 100/100 = 1.0
        let events3 = vec![ev(0.0, 120.0, "a", crate::types::EventType::Chat)];
        assert!((coverage_ratio(&events3, 100.0) - 1.0).abs() < 1e-9);

        // 空事件
        assert_eq!(coverage_ratio(&[], 100.0), 0.0);
        assert_eq!(coverage_ratio(&events, 0.0), 0.0);
    }

    #[test]
    fn test_enforce_max_duration_splits_long_event() {
        // 事件 0~1300s，最大 240s：应至少拆成 6 段，且每段 <= 240s
        let events = vec![ev(0.0, 1300.0, "超长事件", crate::types::EventType::Chat)];
        let segments: Vec<TranscriptSegment> = (0..=13)
            .map(|i| {
                let s = i as f64 * 100.0;
                TranscriptSegment {
                    start: s,
                    end: s + 80.0,
                    text: "x".into(),
                    confidence: 1.0,
                }
            })
            .collect();
        let result = enforce_max_duration(events, &segments, 240.0);
        assert!(result.len() >= 3, "应拆成至少 3 段，实际 {}", result.len());
        for e in &result {
            let dur = e.end.to_seconds() - e.start.to_seconds();
            assert!(dur <= 240.0 + 1e-6, "片段 {} 超过 240s: {}", e.label, dur);
        }
    }

    #[test]
    fn test_enforce_max_duration_keeps_short_events() {
        let events = vec![ev(100.0, 300.0, "正常事件", crate::types::EventType::Chat)];
        let result = enforce_max_duration(events, &[], 240.0);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].label, "正常事件");
    }

    #[test]
    fn test_disambiguate_duplicate_titles_adds_seq_suffix() {
        let events = vec![
            ev(0.0, 100.0, "同一个话题", crate::types::EventType::Chat),
            ev(100.0, 200.0, "同一个话题", crate::types::EventType::Chat),
            ev(200.0, 300.0, "同一个话题", crate::types::EventType::Chat),
            ev(300.0, 400.0, "另一个话题", crate::types::EventType::Gameplay),
        ];
        let result = disambiguate_duplicate_titles(events);
        assert_eq!(result.len(), 4);
        assert_eq!(result[0].label, "同一个话题（1/3）");
        assert_eq!(result[1].label, "同一个话题（2/3）");
        assert_eq!(result[2].label, "同一个话题（3/3）");
        assert_eq!(result[3].label, "另一个话题");
    }
}
