use std::collections::BTreeMap;

use async_trait::async_trait;
use anyhow::{Context, Result};

use crate::config::Config;
use crate::pipeline::{Stage, STAGE_DETECT};
use crate::types::{Event, EventStore, PipelineContext};

mod llm;
pub use llm::{build_api_url, build_request_body, send_request, test_llm};

pub struct EventDetector;

#[async_trait]
impl Stage for EventDetector {
    fn name(&self) -> &'static str {
        "EventDetector"
    }

    fn stage_number(&self) -> u32 {
        STAGE_DETECT
    }

    fn is_enabled(&self, _config: &Config) -> bool {
        true
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

        let mut events = if llm_config.api_key.is_empty() {
            tracing::warn!("未配置 LLM API key (llm.api_key)，使用默认事件");
            vec![llm::create_default_event(segments)]
        } else {
            process_with_llm(segments, llm_config, detect_config).await?
        };

        // 追加主播标签（无 API key 的默认事件也适用）
        if !detect_config.vtuber_name.is_empty() {
            let tag = format!("【{}】", detect_config.vtuber_name);
            for event in &mut events {
                if !event.label.ends_with(&tag) {
                    event.label.push_str(&tag);
                }
            }
            tracing::info!("已追加主播标签: {}", tag);
        }

        // 弹幕热度评分
        if let Some(ref danmaku_path) = ctx.danmaku_path {
            if danmaku_path.exists() {
                tracing::info!("弹幕热度评分: {}", danmaku_path.display());
                enrich_with_danmaku(&mut events, danmaku_path, &ctx.config.detection)?;
            } else {
                tracing::warn!("弹幕文件不存在: {}", danmaku_path.display());
            }
        } else {
            tracing::info!("无弹幕文件，跳过弹幕热度评分");
        }

        ctx.events = events;
        save_events(&ctx.events, &ctx.output_dir)?;

        tracing::info!("事件检测完成: {} 个事件", ctx.events.len());
        Ok(())
    }
}

fn enrich_with_danmaku(
    events: &mut Vec<Event>,
    danmaku_path: &std::path::Path,
    config: &crate::config::DetectionConfig,
) -> Result<()> {
    let window_secs = config.danmaku_window_secs;

    // Determine total duration from events
    let total_duration = events
        .iter()
        .map(|e| e.end.to_seconds())
        .fold(0.0, f64::max)
        .max(1.0);

    let heatmap = crate::danmaku::analyze(danmaku_path, window_secs, total_duration)?;

    if heatmap.is_empty() {
        tracing::warn!("弹幕文件无数据，跳过热度评分");
        return Ok(());
    }

    let total_entries: u64 = heatmap.iter().map(|w| w.count).sum();
    tracing::info!(
        "弹幕热力图: {} 个窗口, 共 {} 条弹幕",
        heatmap.len(),
        total_entries
    );

    // Compute average density for each event
    for event in events.iter_mut() {
        let start = event.start.to_seconds();
        let end = event.end.to_seconds();
        let density = crate::danmaku::average_density(&heatmap, start, end, window_secs);
        event.popularity = density;
    }

    // Sort by popularity descending, assign rank
    events.sort_by(|a, b| {
        b.popularity
            .partial_cmp(&a.popularity)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    for (i, event) in events.iter_mut().enumerate() {
        event.rank = (i + 1) as u32;
    }

    // Restore chronological order for output
    events.sort_by(|a, b| {
        a.start
            .to_seconds()
            .partial_cmp(&b.start.to_seconds())
            .unwrap()
    });

    Ok(())
}

async fn process_with_llm(
    segments: &[crate::types::TranscriptSegment],
    llm: &crate::config::LlmConfig,
    detect: &crate::config::DetectionConfig,
) -> Result<Vec<Event>> {
    tracing::info!("LLM 一次性分析 {} 个 segments", segments.len());

    let mut events = llm::call(segments, llm, detect).await
        .map_err(|e| anyhow::anyhow!("LLM 事件检测失败: {}", e))?;

    events = dedup_and_merge(events);
    events = enforce_min_duration(events, 120.0);
    events = merge_close_events(events, 60.0);

    Ok(events)
}

fn dedup_and_merge(events: Vec<Event>) -> Vec<Event> {
    if events.is_empty() {
        return events;
    }

    let mut result: Vec<Event> = Vec::with_capacity(events.len());

    for event in events {
        if let Some(last) = result.last_mut() {
            let last_start = last.start.to_seconds();
            let last_end = last.end.to_seconds();
            let ev_start = event.start.to_seconds();
            let ev_end = event.end.to_seconds();

            if ev_start >= last_start && ev_end <= last_end {
                continue;
            }

            if ev_start < last_end {
                if last.end.to_seconds() < ev_end {
                    last.end = event.end;
                }
                if ev_start < last_start {
                    last.start = event.start;
                }
                last.confidence = last.confidence.max(event.confidence);
                continue;
            }
        }
        result.push(event);
    }

    result
}

fn enforce_min_duration(mut events: Vec<Event>, min_secs: f64) -> Vec<Event> {
    if events.len() <= 1 {
        return events;
    }

    let mut i = 0;
    while i < events.len() {
        let dur = events[i].end.to_seconds() - events[i].start.to_seconds();
        if dur >= min_secs {
            i += 1;
            continue;
        }

        if i + 1 < events.len() {
            let next = events.remove(i + 1);
            let prev_label = events[i].label.clone();
            events[i].end = next.end;
            events[i].label = prev_label;
            continue;
        }

        if i > 0 {
            let short = events.remove(i);
            i -= 1;
            events[i].end = short.end;
            continue;
        }

        i += 1;
    }

    events
}

/// Merge adjacent events that are close in time (< gap_secs) and share the same event_type.
fn merge_close_events(mut events: Vec<Event>, gap_secs: f64) -> Vec<Event> {
    if events.len() <= 1 {
        return events;
    }

    let mut i = 0;
    while i + 1 < events.len() {
        let cur_end = events[i].end.to_seconds();
        let next_start = events[i + 1].start.to_seconds();
        let gap = next_start - cur_end;
        let same_type = std::mem::discriminant(&events[i].event_type)
            == std::mem::discriminant(&events[i + 1].event_type);

        if gap >= 0.0 && gap <= gap_secs && same_type {
            let next = events.remove(i + 1);
            events[i].end = next.end;
            events[i].confidence = events[i].confidence.max(next.confidence);
        } else {
            i += 1;
        }
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
