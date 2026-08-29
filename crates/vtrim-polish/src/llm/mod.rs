pub mod segment;
pub mod keywords;

use crate::parse::srt::Subtitle;
use crate::project::{Segment, Event};
use vtrim_core::danmaku::DanmakuEntry;
use vtrim_core::llm::LlmClient;

/// 运行 LLM 全流程: 话题区间 + 关键词提取
/// LLM 直接在 segment.events 中生成事件, 不再经 reaction_rules 中转
/// sticker_source 控制提示词中是否提及 sticker 及其语言风格
/// sticker_budget/sfx_budget: 数量上限, 提示词中明确约束
/// sfx_blacklist: 被屏蔽的音效/标签, 注入 prompt
pub async fn run_llm_pipeline(
    api_key: &str,
    base_url: &str,
    model: &str,
    subtitles: &[Subtitle],
    danmaku: &[DanmakuEntry],
    event_type: &str,
    sticker_source: &[String],
    sticker_budget: usize,
    sfx_budget: usize,
    sfx_blacklist: &[String],
) -> anyhow::Result<(Vec<Segment>, Vec<String>, Vec<String>)> {
    let client = std::sync::Arc::new(LlmClient::new(api_key, base_url, model).with_thinking(Some("disabled")));

    // Step 1: 分段 + 每段效果方向 (大分类/音效tag/贴纸/特效/预算)
    println!("  ── LLM: Segmentation + direction (type: {}) ──", event_type);
    let mut segments = match segment::generate_segments(
        &client, subtitles, event_type, sticker_source, sticker_budget, sfx_budget, sfx_blacklist,
    ).await {
        Ok(s) => { println!("  ✓ {} segments", s.len()); s }
        Err(e) => { println!("  ⚠ Segmentation failed: {}. Using default.", e); vec![default_segment()] }
    };

    // Step 1.5: 每段预算 clamp 校验 (min<=max, 全片不超预算)
    for seg in &mut segments {
        let Some(b) = seg.budget.as_mut() else { continue };
        b.sfx_min = b.sfx_min.min(b.sfx_max);
        b.sticker_min = b.sticker_min.min(b.sticker_max);
        b.sfx_max = b.sfx_max.min(sfx_budget.max(1));
        b.sticker_max = b.sticker_max.min(sticker_budget.max(1));
    }

    // Step 2: 逐段事件生成 (并发 ≤3, 注入该段音效候选, 跨段未使用优先)
    println!("  ── LLM: Per-segment events ──");
    let used_sfx: std::sync::Arc<std::sync::Mutex<std::collections::HashSet<String>>> =
        std::sync::Arc::new(std::sync::Mutex::new(std::collections::HashSet::new()));
    let sem = std::sync::Arc::new(tokio::sync::Semaphore::new(3));
    let mut set: tokio::task::JoinSet<(usize, anyhow::Result<Vec<Event>>)> = tokio::task::JoinSet::new();
    let subtitle_clone = subtitles.to_vec();

    for (idx, seg) in segments.iter().enumerate() {
        let tags: Vec<String> = seg.direction.as_ref().map(|d| d.sfx_tags.clone()).unwrap_or_default();
        let used_snapshot = used_sfx.lock().unwrap().clone();
        let candidate_text = crate::assets::sfx_catalog::candidates_text(&tags, &used_snapshot, 32, &sfx_blacklist);
        let seg_own = seg.clone();
        let sub_clone = subtitle_clone.clone();
        let src = sticker_source.to_vec();
        let bl = sfx_blacklist.to_vec();
        let sem2 = sem.clone();
        let client_ref = client.clone();
        set.spawn(async move {
            let _permit = sem2.acquire().await.expect("semaphore");
            let r = segment::generate_segment_events(
                &client_ref, &seg_own, &sub_clone, &candidate_text, &src, &bl,
            ).await;
            (idx, r)
        });
    }
    while let Some(res) = set.join_next().await {
        match res {
            Ok((idx, Ok(events))) => {
                if let Some(seg) = segments.get_mut(idx) {
                    let n = events.len();
                    // 记录已选中的音效 (跨段未使用优先)
                    let mut used = used_sfx.lock().unwrap();
                    for ev in &events {
                        if let Event::Sfx { name, .. } = ev {
                            if !name.is_empty() { used.insert(name.clone()); }
                        }
                        if let Some(sfx) = crate::process::event_sfx_ref(ev) {
                            used.insert(sfx.to_string());
                        }
                    }
                    drop(used);
                    seg.events = events;
                    println!("  ✓ segment {}: {} events", idx + 1, n);
                }
            }
            Ok((idx, Err(e))) => {
                println!("  ⚠ segment {} events failed: {} (空事件, process 兜底)", idx + 1, e);
            }
            Err(e) => println!("  ⚠ event task error: {}", e),
        }
    }

    // Step 2: 关键词提取
    println!("  ── LLM: Keyword extraction ──");
    let sub_texts: Vec<String> = subtitles.iter().map(|s| s.text.clone()).collect();
    let dm_texts: Vec<String> = danmaku.iter().map(|d| d.text.clone()).collect();
    let (dm_filter, hl_words) = match keywords::extract_keywords(&client, &sub_texts, &dm_texts).await {
        Ok(k) => { println!("  ✓ Keywords extracted"); k }
        Err(e) => { println!("  ⚠ Keywords failed: {}. Using defaults.", e); (vec!["OBS".into(), "游戏".into()], vec!["OBS".into()]) }
    };

    // Step 3: 补默认 Camera event (如果 LLM 没有生成)
    for seg in &mut segments {
        if seg.end - seg.start < 1.0 { continue; }
        if !seg.events.iter().any(|e| matches!(e, Event::Camera { .. })) {
            seg.events.push(Event::Camera {
                time: seg.start, duration: seg.end - seg.start,
                zoom: "中".into(), origin: "50% 50%".into(),
            });
        }
    }

    // Step 4: emoji/irasu 位置计算 ("auto" → 计算值, 未来实现)
    // LLM 输出的 x/y 如果是 "auto", 由 cmd_process 统一计算

    Ok((segments, dm_filter, hl_words))
}

fn default_segment() -> Segment {
    Segment {
        start: 0.0, end: 10.0, fx: "low".into(),
        note: "Auto".into(), events: vec![],
        direction: None, budget: None, extra: Default::default(),
    }
}
