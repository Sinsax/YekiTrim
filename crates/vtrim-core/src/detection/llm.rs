use std::time::Duration;

use crate::config::{DetectionConfig, LlmConfig};
use crate::types::{Event, EventSource, EventType, Timestamp, TranscriptSegment};
use anyhow::{Context, Result};

fn format_segments(segments: &[TranscriptSegment]) -> String {
    let mut out = String::new();
    for seg in segments {
        out.push_str(&format!("[{:.1}-{:.1}] {}\n", seg.start, seg.end, seg.text));
    }
    out
}

fn build_prompt(segments: &[TranscriptSegment], config: &DetectionConfig) -> String {
    if !config.prompt_template.is_empty() {
        return config
            .prompt_template
            .replace("{segments}", &format_segments(segments).trim());
    }

    let mut lines = vec![
        "请分析以下直播转录文本（虚拟主播/VTuber），将其划分为多个事件片段。要求：".into(),
        "".into(),
        "1. 每个事件片段时长控制在2分钟-30分钟，短于2分钟的需与相邻片段合并。整段30分钟直播合理的片段数量为15-25个，不要过度细分".into(),
        "2. 标题(label)创作要求：".into(),
        "   - 标题必须信息密度极高，每个字都要有价值，宁可短不可水".into(),
        "   - 标题主体优先使用主播原话/独白/名场面直接输出，如".into(),
        "     \"我站着上厕所！\" \"谁家小土豆地雷？\" \"这也能信？？\"".into(),
        "     避免用\"主播聊\"\"主播提到\"\"主播分享\"等第三人称描述开头".into(),
        "   - 长度 8~50 字，严禁超过 50 字".into(),
        "   - 包含 1-3 个 emoji，常用：🤣😱🥵😡😅🤭🤑👻😊🐶🐮🎵🔥💥😭😫😋💩😈🥺😎".into(),
        "   - 优先使用以下句式（以下皆为要求）：".into(),
        "     a) 反问式/感叹式开头：句首用\"爆了！\"\"难绷\"\"不是...\"等引爆情绪".into(),
        "     b) 直接引语式：主播原话冒号引出，如\"弹幕问豆今色，豆脱口而出：梅川😊\"".into(),
        "     c) 问号叠加/悬念式：?? ???  ...  制造悬念".into(),
        "     d) 列点式（仅限信息密集的素材）".into(),
        "   - 善用网络用语和 V 圈黑话制造信息密度：爆了、难绷、破防、神人、意难平、血压高了、笑死、寄、魔法期、老吃家了、小土豆".into(),
        "   - 质量门槛：每个事件必须提取出核心爆点/名场面/主播原话中的高点。平淡叙事、过渡填充、无爆点水话应合并到相邻事件".into(),
        "3. 摘要(summary)详尽描述片段核心内容和看点，50-100字".into(),
        "4. 事件类型(event_type)从以下选择：".into(),
        "   - gameplay(游戏/操作)".into(),
        "   - interaction(互动/连麦/双播)".into(),
        "   - qna(问答/读 SC)".into(),
        "   - reaction(看视频/听歌/观影/鉴赏)".into(),
        "   - storytelling(讲故事/分享经历/回忆)".into(),
        "   - music(唱歌/演奏/歌切)".into(),
        "   - food(吃播/美食/试吃)".into(),
        "   - chat(聊天/杂谈/日常)".into(),
        "   - discussion(讨论/争论/锐评)".into(),
        "   - ranking(盘点/排行/排行榜)".into(),
        "   - irl(线下/旅行/Vlog/外出)".into(),
        "   - rant(吐槽/破防/红温)".into(),
        "   - other(其他)".into(),
        "".into(),
        "参考标题风格（高信息密度 + 爆点导向）：".into(),
        "  爆了！牢豆直播看塔菲中之人视频😱".into(),
        "  被初王葛孚雷打破防，妈的一天天使不完个牛劲我超！【七海Nana7mi】".into(),
        "  海子姐难绷90岁治疗ED【七海Nana7mi】".into(),
        "  这什么雷霆坐姿雷霆视角🥵羞死人了🥵".into(),
        "  灰泽满妈妈：他们连你成绩很差都知道了吗？？【灰泽满Hazel】".into(),
        "  难绷花礼连自己拉的屎都想吃😅真是老吃家了🤤".into(),
        "  岁己亲推脚臭味薯片：不是我脚臭，这是世俗意义上的脚臭【岁己SUI】".into(),
        "  谁家小土豆地雷？【灰泽满Hazel】".into(),
        "  我站着上厕所！【七海Nana7mi】".into(),
        "  弹幕问豆今色，豆特意低头看了下脱口而出：梅川😊".into(),
        "".into(),
        "输出格式：只输出一个 JSON 数组，不要 markdown 或解释：".into(),
        "".into(),
        "转录片段：".into(),
    ];

    for seg in segments {
        lines.push(format!("[{:.1}-{:.1}] {}", seg.start, seg.end, seg.text));
    }

    lines.push("".into());
    lines.push("只输出JSON数组：".into());
    lines.push("[{\"start\": 0.0, \"end\": 120.0, \"label\": \"这操作给我看傻了??🤣\", \"summary\": \"详尽描述这段内容的核心看点...\", \"event_type\": \"interaction\"}]".into());

    lines.join("\n")
}

fn parse_json(text: &str) -> Result<serde_json::Value> {
    let text = text.trim();

    let json_str = if let Some(pos) = text.find("```json") {
        let after = &text[pos + 7..];
        after.split("```").next().unwrap_or(text)
    } else if let Some(pos) = text.find("```") {
        let after = &text[pos + 3..];
        after.split("```").next().unwrap_or(text)
    } else {
        text
    };

    let json_str = json_str.trim();
    let start = json_str.find('[').unwrap_or(0);
    let end = json_str.rfind(']').map(|i| i + 1).unwrap_or(json_str.len());

    serde_json::from_str(&json_str[start..end]).context("LLM 返回的 JSON 格式无效")
}

#[derive(serde::Deserialize)]
struct RawEvent {
    start: f64,
    end: f64,
    label: String,
    #[serde(default)]
    summary: String,
    #[serde(default = "default_event_type")]
    event_type: String,
}

fn default_event_type() -> String {
    "other".into()
}

fn parse_events(value: serde_json::Value) -> Vec<Event> {
    let raw: Vec<RawEvent> = match serde_json::from_value(value) {
        Ok(r) => r,
        Err(_) => return Vec::new(),
    };

    raw.into_iter()
        .map(|r| {
            let event_type = match r.event_type.to_lowercase().as_str() {
                "intro" => EventType::Intro,
                "gameplay" => EventType::Gameplay,
                "interaction" => EventType::Interaction,
                "qna" => EventType::QnA,
                "reaction" => EventType::Reaction,
                "storytelling" => EventType::Storytelling,
                "music" => EventType::Music,
                "food" => EventType::Food,
                "chant" | "chat" => EventType::Chat,
                "eating" => EventType::Eating,
                "irl" => EventType::IRL,
                "ranking" => EventType::Ranking,
                "discussion" => EventType::Discussion,
                "rant" => EventType::Rant,
                "break" => EventType::Break,
                "outro" => EventType::Outro,
                _ => EventType::Other,
            };

            Event {
                start: Timestamp::from_seconds(r.start.max(0.0)),
                end: Timestamp::from_seconds(r.end.max(0.0)),
                label: r.label,
                summary: r.summary,
                event_type,
                source: EventSource::LLM,
                confidence: 1.0,
                popularity: 0.0,
                rank: 0,
                enabled: true,
            }
        })
        .collect()
}

fn system_prompt(config: &DetectionConfig) -> &str {
    if !config.system_prompt.is_empty() {
        return &config.system_prompt;
    }
    "你是一个虚拟主播/VTuber 直播切片分析助手。分析直播转录文本，\
     将内容划分为多个有看点的视频片段。\
     标题必须高信息密度，以主播原话/独白/名场面直接输出为主体，\
     禁止\"主播聊\"\"主播提到\"等第三人称描述开头。\
     善用反问、感叹、问号叠加制造悬念，嵌入主播原话+网络黑话\
     （爆了、难绷、破防、神人、意难平、魔法期、老吃家了、小土豆）。\
     摘要要详尽描述内容看点。质量优先于数量，每个事件必须有明显的看点、转折、爆点或清晰的话题边界，无话题切换的过渡内容应合并。"
}

pub fn build_request_body(llm: &LlmConfig, messages: serde_json::Value) -> serde_json::Value {
    let mut body = serde_json::json!({
        "temperature": llm.temperature,
        "max_tokens": llm.max_tokens,
        "messages": messages,
    });
    if !llm.model.is_empty() {
        body["model"] = serde_json::json!(llm.model);
    }
    if let Some(ref extra) = llm.extra_params {
        if let Ok(extra_val) = serde_json::to_value(extra) {
            if let serde_json::Value::Object(extra_map) = extra_val {
                for (k, v) in extra_map {
                    body[k] = v;
                }
            }
        }
    }
    body
}

pub fn build_api_url(llm: &LlmConfig) -> Result<String> {
    if llm.api_base.is_empty() {
        anyhow::bail!("未配置 LLM API 基础地址 (llm.api_base)，请设置后重试");
    }
    let base = llm.api_base.trim_end_matches('/');
    if base.ends_with("/chat/completions") {
        Ok(base.to_string())
    } else {
        Ok(format!("{}/chat/completions", base))
    }
}

pub async fn send_request(llm: &LlmConfig, body: &serde_json::Value, timeout_secs: u64) -> Result<serde_json::Value> {
    let api_url = build_api_url(llm)?;
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(timeout_secs))
        .build()
        .context("创建 HTTP 客户端失败")?;

    let mut last_err = None;
    for attempt in 0..3 {
        let resp = client
            .post(&api_url)
            .header("Authorization", format!("Bearer {}", llm.api_key))
            .json(body)
            .send()
            .await;

        match resp {
            Ok(r) if r.status().is_success() => {
                return r.json().await.context("解析 LLM API 响应失败");
            }
            Ok(r) => {
                let status = r.status();
                let text = r.text().await.unwrap_or_default();
                let retryable = status.as_u16() == 429 || status.as_u16() >= 500;
                if retryable && attempt < 2 {
                    let delay = Duration::from_secs(2u64.pow(attempt as u32 + 1));
                    tracing::warn!("LLM API 返回 {}，{}s 后重试 ({}/3)", status, delay.as_secs(), attempt + 2);
                    tokio::time::sleep(delay).await;
                    last_err = Some(anyhow::anyhow!("LLM API 返回 {}: {}", status, text));
                    continue;
                }
                anyhow::bail!("LLM API 返回 {}: {}", status, text);
            }
            Err(e) => {
                let retryable = e.is_timeout() || e.is_connect();
                if retryable && attempt < 2 {
                    let delay = Duration::from_secs(2u64.pow(attempt as u32 + 1));
                    tracing::warn!("LLM API 请求失败: {}，{}s 后重试 ({}/3)", e, delay.as_secs(), attempt + 2);
                    tokio::time::sleep(delay).await;
                    last_err = Some(anyhow::anyhow!("LLM API 请求失败: {}", e));
                    continue;
                }
                anyhow::bail!("LLM API 请求失败: {}", e);
            }
        }
    }

    Err(last_err.unwrap_or_else(|| anyhow::anyhow!("LLM API 请求失败")))
}

pub async fn call(segments: &[TranscriptSegment], llm: &LlmConfig, detect: &DetectionConfig) -> Result<Vec<Event>> {
    let prompt = build_prompt(segments, detect);
    let messages = serde_json::json!([
        {"role": "system", "content": system_prompt(detect)},
        {"role": "user", "content": prompt}
    ]);
    let body = build_request_body(llm, messages);
    let body = send_request(llm, &body, 180).await?;

    let content = body["choices"][0]["message"]["content"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("LLM 响应缺少 choices[0].message.content"))?;

    let json_value = match parse_json(content) {
        Ok(v) => v,
        Err(e) => {
            let ts = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            let log_path = std::path::PathBuf::from(format!("vtrim_llm_debug_{}.log", ts));
            let full = format!(
                "--- content ---\n{}\n\n--- full response ---\n{}",
                content,
                serde_json::to_string_pretty(&body).unwrap_or_default()
            );
            let _ = std::fs::write(&log_path, &full);
            tracing::warn!("LLM 响应解析失败: {}，已保存到 {}", e, log_path.display());
            return Err(e);
        }
    };
    let events = parse_events(json_value);

    if events.is_empty() {
        anyhow::bail!("LLM 返回的事件列表为空");
    }

    Ok(events)
}

pub async fn test_llm(llm: &LlmConfig, prompt: &str) -> Result<String> {
    if llm.api_key.is_empty() {
        anyhow::bail!("LLM API key 未配置");
    }
    let messages = serde_json::json!([
        {"role": "system", "content": "你是一个 AI 助手。"},
        {"role": "user", "content": prompt}
    ]);
    let body = build_request_body(llm, messages);
    send_request(llm, &body, 180).await.map(|v| v.to_string())
}

pub fn create_default_event(segments: &[TranscriptSegment]) -> Event {
    let start = segments.first().map(|s| s.start).unwrap_or(0.0);
    let end = segments.last().map(|s| s.end).unwrap_or(0.0);

    Event {
        start: Timestamp::from_seconds(start),
        end: Timestamp::from_seconds(end),
        label: "完整录像".into(),
        summary: "未能识别具体事件".into(),
        event_type: EventType::Other,
        source: EventSource::LLM,
        confidence: 0.3,
        popularity: 0.0,
        rank: 0,
        enabled: true,
    }
}
