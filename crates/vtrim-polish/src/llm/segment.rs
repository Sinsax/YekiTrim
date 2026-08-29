use crate::parse::srt::Subtitle;
use crate::project::Segment;
use vtrim_core::llm::LlmClient;

/// 调用 LLM 生成话题区间 + 段内事件 + 音效判断
/// sticker_source 影响提示词中 sticker 事件的语言描述, 空数组则不提及 sticker
/// sticker_budget/sfx_budget 为数量上限 (来自 content_budget), 提示词中明确约束
/// sfx_blacklist: 被屏蔽的音效/标签, 注入 prompt 让 LLM 主动避开
pub async fn generate_segments(
    client: &LlmClient,
    subtitles: &[Subtitle],
    event_type: &str,
    sticker_source: &[String],
    sticker_budget: usize,
    sfx_budget: usize,
    sfx_blacklist: &[String],
) -> anyhow::Result<Vec<Segment>> {
    let system_prompt = build_segmentation_system_prompt(sticker_source);
    let user_prompt = build_segmentation_user_prompt(subtitles, event_type, sticker_budget, sfx_budget, sfx_blacklist);
    let response = client.chat(&system_prompt, &user_prompt).await?;
    let json = extract_json_array(&response)?;
    // LLM 可能漏掉事件的 type 字段 (tagged enum 必需), 按字段特征补全后重试
    let json = normalize_segments_json(&json);
    let segments: Result<Vec<Segment>, _> = serde_json::from_str(&json);
    match segments {
        Ok(s) => Ok(s),
        Err(e) => {
            // 保存完整 JSON 到文件供调试
            if let Ok(mut f) = std::fs::File::create("_llm_debug.json") {
                use std::io::Write;
                let _ = f.write_all(json.as_bytes());
            }
            eprintln!("  ⚠ Full LLM response saved to _llm_debug.json");
            eprintln!("  ⚠ Error: {}", e);
            anyhow::bail!("Segments parse failed")
        }
    }
}

/// Step2: 为该段生成事件（注入音效候选清单 + 方向 + 预算约束）
pub async fn generate_segment_events(
    client: &LlmClient,
    segment: &Segment,
    subtitles: &[Subtitle],
    candidate_text: &str,
    sticker_source: &[String],
    sfx_blacklist: &[String],
) -> anyhow::Result<Vec<crate::project::Event>> {
    let system_prompt = build_events_system_prompt(sticker_source, sfx_blacklist);
    let user_prompt = build_events_user_prompt(segment, subtitles, candidate_text);
    let response = client.chat(&system_prompt, &user_prompt).await?;
    let json = extract_json_array(&response)?;
    let json = normalize_segments_json(&json);
    let events: Result<Vec<crate::project::Event>, _> = serde_json::from_str(&json);
    match events {
        Ok(evs) => Ok(evs),
        Err(e) => {
            if let Ok(mut f) = std::fs::File::create("_llm_debug.json") {
                use std::io::Write;
                let _ = f.write_all(json.as_bytes());
            }
            anyhow::bail!("Events parse failed: {}", e)
        }
    }
}

/// Step2 system prompt: 事件生成（音效只能从候选选择, 数量受预算约束）
fn build_events_system_prompt(sticker_source: &[String], sfx_blacklist: &[String]) -> String {
    let sticker_desc = sticker_lang_desc(sticker_source);
    let blacklist_line = if sfx_blacklist.is_empty() {
        String::new()
    } else {
        format!("- 以下音效被屏蔽, 禁止使用: {}
", sfx_blacklist.join(", "))
    };
    format!(r##"你是一个直播切片事件助手。为指定时间段生成视觉特效/贴纸/音效事件。

事件类型 (每个事件对象以 "type" 字段开头):
- camera: 运镜. {{"type":"camera","time":0.5,"duration":3.0,"zoom":"中","origin":"50% 50%"}}
  zoom = "中"/"近", origin 用 "50% 50%" 格式
- sticker: 贴图表情. {{"type":"sticker","time":10.0,"duration":2.0,"keyword":"laugh","x":"auto","y":"auto"}}
  keyword 语言: {}
- 视觉特效: shake/flash/blur/zoom_bounce/vignette/hue_shift/stutter/split/noise/pulse/color_shift
  {{"type":"shake","time":8.0,"intensity":0.5,"duration":1.2,"sfx":"tag:爆発"}}
- overlay: 覆盖层. {{"type":"overlay","time":0.0,"duration":5.0,"overlay_type":"gradient","color":"#ff0000","opacity":0.3}}
- sfx: 纯音效. {{"type":"sfx","time":3.0,"name":"explosion","volume":0.5,"duration":1.0}}

选择规则:
- 音效 name/sfx 只能从"可用音效候选"中选择具体 id, 或写 tag:xxx 由系统匹配; 候选外的 id 无效
- 音效只放在情绪爆发/转折/名场面时刻, 普通叙述不要加
- 贴纸在情绪点使用, keyword 与情绪匹配
- 特效 duration 覆盖情绪表达时长 (0.8~2.5s), 不要 0.3s 一闪而过
- x/y 始终写 "auto"
- **优先为每个特效/贴纸从候选清单配一个音效** (候选已按本段方向筛选, 配错风险低);
  仅当候选清单确实没有合适音效时才留空
{}
"##, sticker_desc, blacklist_line)
}

/// Step2 user prompt: 段信息 + 候选清单 + 段内字幕
fn build_events_user_prompt(segment: &Segment, subtitles: &[Subtitle], candidate_text: &str) -> String {
    let dir = segment.direction.as_ref();
    let budget = segment.budget.as_ref();
    let mut prompt = format!("段时间: [{:.1}s - {:.1}s]
", segment.start, segment.end);
    if let Some(d) = dir {
        prompt.push_str(&format!("方向: 大分类={}, 音效tag=[{}], 贴纸=[{}], 特效=[{}], mood={}
",
            d.cat, d.sfx_tags.join("+"), d.sticker.join("+"), d.effects.join("+"), d.mood));
    }
    if let Some(b) = budget {
        prompt.push_str(&format!("数量约束: sfx 事件 {}-{} 个, sticker 事件 {}-{} 个
",
            b.sfx_min, b.sfx_max, b.sticker_min, b.sticker_max));
    }
    if !candidate_text.is_empty() {
        prompt.push_str(&format!("
{}
", candidate_text));
    }
    prompt.push_str("
段内字幕:
");
    for sub in subtitles.iter().filter(|s| s.end >= segment.start - 3.0 && s.start <= segment.end + 3.0) {
        prompt.push_str(&format!("[{:.1}s-{:.1}s] {}
", sub.start, sub.end, sub.text));
    }
    prompt.push_str("
输出 JSON 数组 (不要 markdown 包裹):");
    prompt
}

/// 贴纸 keyword 语言说明（Step1/Step2 共用）
fn sticker_lang_desc(sticker_source: &[String]) -> String {
    if sticker_source.is_empty() {
        return "（不生成贴纸）".to_string();
    }
    if sticker_source.iter().any(|s| s == "emoji") {
        "(英文，如 \"laugh\" \"cry\" \"shock\" \"game\")".to_string()
    } else if sticker_source.iter().any(|s| s == "irasu") {
        "(日文优先，如「ゲーム」「驚き」「爆笑」)".to_string()
    } else {
        "(英文，如 \"laugh\" \"cry\" \"shock\" \"game\")".to_string()
    }
}

/// LLM 可能漏掉事件对象的 type 字段 (serde tagged enum 必需)。
/// 解析前按字段特征推断补全, 提高不同模型的输出容错性。
/// 推断规则: zoom/origin→camera, overlay_type→overlay, keyword→sticker,
/// name→sfx, intensity/color→shake。无法推断的保持原样（由 serde 报错）。
fn normalize_segments_json(json: &str) -> String {
    let infer_type = |obj: &serde_json::Map<String, serde_json::Value>| -> Option<&'static str> {
        if obj.contains_key("zoom") || obj.contains_key("origin") {
            return Some("camera");
        }
        if obj.contains_key("overlay_type") {
            return Some("overlay");
        }
        if obj.contains_key("keyword") {
            return Some("sticker");
        }
        if obj.contains_key("name") {
            return Some("sfx");
        }
        if obj.contains_key("intensity") || obj.contains_key("color") {
            return Some("shake");
        }
        None
    };

    match serde_json::from_str::<serde_json::Value>(json) {
        Ok(serde_json::Value::Array(mut segs)) => {
            for seg in segs.iter_mut() {
                if let Some(evs) = seg.get_mut("events").and_then(|e| e.as_array_mut()) {
                    for ev in evs.iter_mut() {
                        if let Some(obj) = ev.as_object_mut() {
                            if !obj.contains_key("type")
                                && let Some(t) = infer_type(obj) {
                                    obj.insert("type".into(), serde_json::Value::String(t.into()));
                                }
                        }
                    }
                }
            }
            serde_json::to_string(&segs).unwrap_or_else(|_| json.to_string())
        }
        _ => json.to_string(),
    }
}

/// Step1 system prompt: 分段 + 每段方向（大分类/细分tag/贴纸/特效/mood/预算）
pub fn build_segmentation_system_prompt(sticker_source: &[String]) -> String {
    let sticker_desc = sticker_lang_desc(sticker_source);
    format!(r#"你是一个直播切片助手。将字幕按话题切分成段，并为每段确定"效果方向"。

每段输出 (JSON 对象):
{{
  "start": 0.0, "end": 85.0,
  "fx": "high",
  "note": "话题备注",
  "direction": {{
    "cat": "① 主播情绪反应",
    "sfx_tags": ["爆炸", "打击格斗"],
    "sticker": ["laugh"],
    "effects": ["shake"],
    "mood": 3
  }},
  "budget": {{"sfx_min": 1, "sfx_max": 3, "sticker_min": 0, "sticker_max": 2}}
}}

方向选择规则:
- cat: 从 user prompt 的 7 个大分类中选择最匹配的一个
- sfx_tags: 从该大分类对应的细分 tag 中选择 1~3 个 (该段预期出现的音效场景);
  纯叙述段可以选空数组 []
- mood: 高光等级 0-3 (0=平淡叙述, 1=日常, 2=情绪起伏, 3=名场面/爆点)
- sticker: 该段的贴纸情绪词, 给出 2~3 个不同词 (如 ["laugh","shock","proud"]) 供各时刻选择 {}; 不需要贴纸时为空数组
- effects: 该段预期视觉特效类型 (shake/flash/blur/zoom_bounce/vignette/hue_shift/stutter/split/noise/pulse/color_shift), 不需要时为空数组
- budget: 按段长与情绪强度给数量上下限 (名场面段可多, 叙述段全 0; sfx_min<=sfx_max, sticker_min<=sticker_max)
- fx: 该段情绪密度 low/med/high
"#, sticker_desc)
}

/// Step1 user prompt: 视频类型 + 大分类导航 + 全量字幕
pub fn build_segmentation_user_prompt(
    subtitles: &[Subtitle],
    event_type: &str,
    sticker_budget: usize,
    sfx_budget: usize,
    sfx_blacklist: &[String],
) -> String {
    let mut prompt = format!("视频类型: {}

", event_type);
    prompt.push_str(&crate::assets::sfx_catalog::categories_text());
    if !sfx_blacklist.is_empty() {
        prompt.push_str(&format!("
音效黑名单(禁止使用): {}
", sfx_blacklist.join(", ")));
    }
    prompt.push_str(&format!("
全片预算: sticker 总数不超过 {} 个, sfx 事件总数不超过 {} 个
", sticker_budget, sfx_budget));
    prompt.push_str("
字幕内容 (每行: [开始秒-结束秒] 文本):
");
    for sub in subtitles.iter().take(200) {
        prompt.push_str(&format!("[{:.1}s-{:.1}s] {}
", sub.start, sub.end, sub.text));
    }
    prompt.push_str("
输出 JSON 数组 (不要 markdown 包裹):");
    prompt
}

fn extract_json_array(response: &str) -> anyhow::Result<String> {
    let trimmed = response.trim();
    if let Some(start) = trimmed.find('[')
        && let Some(end) = trimmed.rfind(']') {
            return Ok(trimmed[start..=end].to_string());
        }
    anyhow::bail!("No JSON array found:\n{}", &trimmed[..200.min(trimmed.len())])
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 回归测试: LLM 输出缺 type 字段时按字段特征补全 (实际失败样例)
    #[test]
    fn normalize_infers_missing_type() {
        let raw = r#"[
          {"start":0.0,"end":8.6,"fx":"low","note":"t","events":[
            {"time":0.5,"duration":3.0,"zoom":"中","origin":"center"},
            {"time":8.0,"duration":0.5,"intensity":0.3,"sfx":"tag:爆発"},
            {"type":"camera","time":2.0,"duration":1.0,"zoom":"近","origin":"50% 50%"},
            {"time":10.0,"duration":2.0,"keyword":"laugh","x":"auto","y":"auto"},
            {"time":3.0,"name":"pop","volume":0.5}
          ]}
        ]"#;
        let norm = normalize_segments_json(raw);
        let segments: Vec<crate::project::Segment> = serde_json::from_str(&norm).unwrap();
        let evs = &segments[0].events;
        assert_eq!(evs.len(), 5);
        assert!(matches!(evs[0], crate::project::Event::Camera { .. }));
        assert!(matches!(evs[1], crate::project::Event::Shake { .. }));
        assert!(matches!(evs[2], crate::project::Event::Camera { .. }));
        assert!(matches!(evs[3], crate::project::Event::Sticker { .. }));
        assert!(matches!(evs[4], crate::project::Event::Sfx { .. }));
    }

    /// 已有 type 的事件不被改动
    #[test]
    fn normalize_keeps_existing_type() {
        let raw = r#"[{"start":0.0,"end":1.0,"fx":"low","note":"","events":[
            {"type":"shake","time":0.5,"intensity":0.9}
        ]}]"#;
        let norm = normalize_segments_json(raw);
        let segments: Vec<crate::project::Segment> = serde_json::from_str(&norm).unwrap();
        assert!(matches!(segments[0].events[0], crate::project::Event::Shake { .. }));
    }
}
