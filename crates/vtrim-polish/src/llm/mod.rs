pub mod client;
pub mod segment;
pub mod keywords;

use crate::parse::srt::Subtitle;
use crate::parse::danmaku::DanmakuEntry;
use crate::project::{Segment, Event};

/// 运行 LLM 全流程: 话题区间 + 关键词提取
/// LLM 直接在 segment.events 中生成事件, 不再经 reaction_rules 中转
/// sticker_source 控制提示词中是否提及 sticker 及其语言风格
pub async fn run_llm_pipeline(
    api_key: &str,
    base_url: &str,
    model: &str,
    subtitles: &[Subtitle],
    danmaku: &[DanmakuEntry],
    event_type: &str,
    sticker_source: &[String],
) -> anyhow::Result<(Vec<Segment>, Vec<String>, Vec<String>)> {
    let client = client::LlmClient::new(api_key.to_string(), Some(base_url.to_string()), Some(model.to_string()));

    // Step 1: 话题区间 (LLM 直接在 events 中生成 camera/sticker 等)
    println!("  ── LLM: Topic segmentation (type: {}) ──", event_type);
    let mut segments = match segment::generate_segments(&client, subtitles, event_type, sticker_source).await {
        Ok(s) => { println!("  ✓ {} segments", s.len()); s }
        Err(e) => { println!("  ⚠ Segmentation failed: {}. Using default.", e); vec![default_segment()] }
    };

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
        note: "Auto".into(), events: vec![], extra: Default::default(),
    }
}
