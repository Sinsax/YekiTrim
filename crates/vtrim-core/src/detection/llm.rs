use crate::config::LlmConfig;
use crate::types::{Event, EventSource, EventType, Timestamp, TranscriptSegment};
use anyhow::{Context, Result};

fn build_prompt(segments: &[TranscriptSegment]) -> String {
    let mut lines = vec![
        "请分析以下直播转录文本，将其划分为多个事件片段。要求：".into(),
        "".into(),
        "1. 每个事件片段时长控制在1-3分钟，短于60秒的片段需与相邻片段合并".into(),
        "2. 标题(label)口语化、有槽点/反转/意想不到的瞬间，包含1-2个emoji🤣😱🥵😡😅🤭🤑👻，适当使用反问句或感叹号，总长度不超过40字".into(),
        "3. 摘要(summary)要详尽，完整概括该片段的核心内容和看点，50-100字".into(),
        "4. 事件类型(event_type)从以下选择: intro(开场), gameplay(游戏/操作), interaction(互动), qna(问答), break(休息), outro(结尾), other(其他)".into(),
        "".into(),
        "参考标题风格：".into(),
        "  这个反应也太真实了吧？？直接笑死🤣".into(),
        "  粉丝打SC问谁攻谁受，结果全员沉默？？".into(),
        "  你管这叫唱歌？这分明是作法现场????".into(),
        "  被自己翻唱尬到脚趾抠地?? 这也太真实了吧😅".into(),
        "  这什么雷霆坐姿雷霆视角🥵羞死人了🥵".into(),
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
            }
        })
        .collect()
}

fn system_prompt() -> &'static str {
    "你是一个直播录像分析助手。分析转录文本，将内容划分为多个有看点的视频片段。\
     标题要吸引B站观众，摘要要详尽描述内容。"
}

pub async fn call(segments: &[TranscriptSegment], config: &LlmConfig) -> Result<Vec<Event>> {
    let prompt = build_prompt(segments);

    let api_url = if config.api_base.is_empty() {
        "https://api.openai.com/v1/chat/completions".to_string()
    } else {
        let base = config.api_base.trim_end_matches('/');
        if base.ends_with("/chat/completions") {
            base.to_string()
        } else {
            format!("{}/chat/completions", base)
        }
    };

    let client = reqwest::Client::new();
    let resp = client
        .post(&api_url)
        .header("Authorization", format!("Bearer {}", config.api_key))
        .json(&serde_json::json!({
            "model": config.model,
            "temperature": config.temperature,
            "max_tokens": config.max_tokens,
            "messages": [
                {"role": "system", "content": system_prompt()},
                {"role": "user", "content": prompt}
            ]
        }))
        .send()
        .await
        .context("LLM API 请求失败")?;

    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        anyhow::bail!("LLM API 返回 {}: {}", status, body);
    }

    let body: serde_json::Value = resp.json().await.context("解析 LLM API 响应失败")?;

    let content = body["choices"][0]["message"]["content"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("LLM 响应缺少 choices[0].message.content"))?;

    let json_value = match parse_json(content) {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!("LLM 响应解析失败: {}", e);
            let log_path = std::path::PathBuf::from("vtrim_llm_debug.log");
            if let Err(we) = std::fs::write(&log_path, content) {
                tracing::warn!("无法写入调试日志: {}", we);
            } else {
                tracing::warn!("完整响应已保存到: {}", log_path.display());
            }
            return Err(e);
        }
    };
    let events = parse_events(json_value);

    if events.is_empty() {
        anyhow::bail!("LLM 返回的事件列表为空");
    }

    Ok(events)
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
    }
}
