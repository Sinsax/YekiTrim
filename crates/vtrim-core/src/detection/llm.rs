use crate::config::{DetectionConfig, LlmConfig};
use crate::types::{Event, EventSource, EventType, Timestamp, TranscriptSegment};
use anyhow::{Context, Result};

pub const DEFAULT_PROMPT: &str = "请分析以下直播转录文本（虚拟主播/VTuber），将其划分为多个事件片段。要求：

1. 每个事件片段时长控制在2分钟-30分钟，连续的同个话题不应被拆分。短于2分钟的需与相邻片段合并，不要过度细分
   - **同一个大话题下的子话题切换不是事件边界**：例如连着一整段聊卫生习惯，中间从擦屁股换到洗屁股只是子话题切换，应合并为同一个事件
   - 只有明显的话题转换（如从聊天切换到打游戏、从讲故事切换到唱歌）才应切分事件
   - 事件边界必须精确：start 要设在话题实际开始的位置，不要包含前一事件的残余内容
   - 如果转录中前一个话题结束后还有几句过渡/闲聊，它们属于前一个事件，不要把过渡段落算给新事件
   - 转录中唱歌/外语/背景音常被 ASR 识别成无意义短句或乱码（如单个字、不成词片段）：不要因为出现几段乱码就新建事件，也不要把乱码逐字写进标题/摘要；整段以乱码/哼唱为主时概括为“哼唱/唱歌/歌声”即可
2. 标题(label)创作要求（**最重要**：标题是整个输出的灵魂，必须精心设计，严禁流水线模板）：
   - 标题必须信息密度极高，每个字都要有价值，宁可短不可水
   - 禁止用\"主播聊\"\"主播提到\"\"主播分享\"\"主播说\"等第三人称描述开头
   - **标题结构多样化要求**：确保相邻标题采用不同的开头句式，连续 3 个标题不能都用同样的结构
   - 禁止连续使用相同开头句式（如连续多个\"唱《XX》\"或连续多个\"爆了！\"）
   - 优先采用以下结构，且在同一次输出中**混合使用**：
     a) **直接引语嵌入**：把主播原话/名场面直接嵌入标题中，不要只转述大意。如\"弹幕问豆今色，豆脱口而出：梅川\"
     b) **事件→反应**：先交代一段具体情景（包含具体细节、数字、人名），再跟反应/吐槽。如\"深夜鉴赏泫雅MV，怎么一言不合就🥵又是小时候被吓哭的一集😱\"
     c) **爆点感叹开头**：\"爆了！\"\"难绷\"\"不是...\"\"笑死\"等引爆情绪，但不要连续使用
     d) **悬念/反问**：用 ? ?? ... 制造悬念
   - 长度 8~50 字，严禁超过 50 字
   - emoji **可选，可不加**。如果使用，放在句中爆点/吐槽处或句尾点缀即可，不要放在标题开头
   - 善用网络用语和 V 圈黑话制造信息密度：爆了、难绷、破防、神人、意难平、血压高了、笑死、寄、魔法期、老吃家了、小土豆、红温、串、纯良
   - **差异检查**：假设你自己就是观众，扫一眼 10 个标题，如果感觉它们\"长得很像\"\"都是一个套路\"，就要主动重写
3. 摘要(summary)详尽描述片段核心内容和看点，50-100字
4. 事件类型(event_type)从以下选择：
   - gameplay(游戏/操作)
   - interaction(互动/连麦/双播)
   - qna(问答/读 SC)
   - reaction(看视频/听歌/观影/鉴赏)
   - storytelling(讲故事/分享经历/回忆)
   - music(唱歌/演奏/歌切)
   - food(吃播/美食/试吃)
   - chat(聊天/杂谈/日常)
   - discussion(讨论/争论/锐评)
   - ranking(盘点/排行/排行榜)
   - **event_type 以整段主体内容为准**：如果唱歌/哼唱只是短暂穿插，而主体是聊天/锐评/讲故事/吃播，应归入主体类型，不要标成 music；只有整段主要内容是唱歌/演奏/歌切时才用 music
   - irl(线下/旅行/Vlog/外出)
   - rant(吐槽/破防/红温)
   - other(其他)

高质量标题示例（注意多样性和叙事层次，对比例子中不同的开头方式）：
  ▶ 直接引语+反应：弹幕问豆今色，豆特意低头看了下脱口而出：梅川😊我去原来豆神喜欢挂空档啊🥵
  ▶ 感叹+叙事：牢豆这什么雷霆坐姿雷霆视角🥵羞死人了🥵
  ▶ 难绷+细节：难绷花礼连自己拉的屎都想吃😅真是老吃家了🤤
  ▶ 叙事→反应：这戴佳伟也是纯串子！暗示谁点5个手枪腿呢🤭给肥梨气的怒吼：滚！下播了🤬
  ▶ 具体数据+感叹：属于牛哥的偶蹄目神了！沐霂豪取四千多分全场最佳！太强了！
  ▶ 事件铺垫+反问：深夜鉴赏泫雅经典《没有明天》MV，怎么一言不合就🥵又是小时候被吓哭的一集😱
  ▶ 叙事+吐槽：肥梨讲述如何在恬豆家拉屎把马桶堵住的故事💩我拉屎没关门是因为还没出来！出来了我就关门了🤢
  ▶ 主播原话+反差：保持虚拟感是我做虚拟偶像的准则😤我今天多有虚拟感啊😤娜娜娜~娜娜娜~😅
  ▶ 食材清单+吐槽：又神也搞起吃播观影回了😋今日食材\"一碗焖饭+一锅肥肠+一碗大果村泡菜+一瓶橙汁\"食量不赖👍
  ▶ 爆点+具体经过：爆了！肥梨顺嘴一句\"过来条条\"把禧运楼最有虚拟感的狗宝也开盒了😨原来私底下叫条条啊😄
  ▶ 回忆+反差：刚出道时3D回不敢开暖空调，怕机器热坏了，顶着0℃直播跟冰窖似的🥶当时居然能撑下来😲
  ▶ 动作+吐槽：这下主播猿形毕露了🐵又神太会暂停了笑死🤣
  ▶ 直接引语反问：谁家小土豆地雷？【灰泽满Hazel】

输出格式：只输出一个 JSON 数组，不要 markdown 或解释。
每个事件包含 start/end/label/summary/event_type。

转录片段：
{segments}

只输出JSON数组：
[{\"start\": 0.0, \"end\": 120.0, \"label\": \"这操作给我看傻了??🤣\", \"summary\": \"详尽描述这段内容的核心看点...\", \"event_type\": \"interaction\"}]";

fn format_segments(segments: &[TranscriptSegment]) -> String {
    let mut out = String::new();
    for seg in segments {
        out.push_str(&format!("[{:.1}-{:.1}] {}\n", seg.start, seg.end, seg.text));
    }
    out
}

fn build_prompt(
    segments: &[TranscriptSegment],
    config: &DetectionConfig,
    hot_windows: &[(f64, f64)],
) -> String {
    let template = if config.prompt_template.is_empty() {
        DEFAULT_PROMPT
    } else {
        &config.prompt_template
    };
    let mut prompt = template.replace("{segments}", format_segments(segments).trim());

    // 追加弹幕爆发时刻：让 LLM 看到观众反应最激烈的名场面位置
    if !hot_windows.is_empty() {
        prompt.push_str("\n\n=== 观众弹幕爆发时刻（弹幕热力图统计，以下时间区间内弹幕密度最高，是全场观众反应最激烈的名场面） ===\n");
        for (s, e) in hot_windows {
            prompt.push_str(&format!("[{:.1}-{:.1}] 弹幕爆发\n", s, e));
        }
        prompt.push_str(
            "要求：\n\
             - 这些时刻必须被包含在某个事件内，绝不能被遗漏\n\
             - 事件边界不得切在这些爆发区间内部（爆发时刻不能落在两个事件的交界处或事件之外）\n\
             - 若爆发区间横跨两段话题，将其归入与之更相关的事件\n",
        );
    }

    prompt
}

pub fn parse_json(text: &str) -> Result<serde_json::Value> {
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

    let value: serde_json::Value =
        serde_json::from_str(&json_str[start..end]).context("LLM 返回的 JSON 格式无效")?;

    // 支持 {"events": [...]} 对象包装（部分模型会包裹输出）
    if value.is_object() {
        if let Some(arr) = value.get("events").filter(|v| v.is_array()) {
            return Ok(arr.clone());
        }
        anyhow::bail!("LLM 返回的 JSON 是对象但缺少 events 数组");
    }
    Ok(value)
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

fn parse_events(value: serde_json::Value) -> Result<Vec<Event>> {
    let raw: Vec<RawEvent> = serde_json::from_value(value)
        .map_err(|e| anyhow::anyhow!("事件 JSON 字段解析失败: {}", e))?;

    Ok(raw
        .into_iter()
        .map(|r| {
            let event_type = parse_event_type(&r.event_type);
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
        .collect())
}

/// 将 LLM 输出的类型名映射为 EventType。
/// 优先精确匹配英文 key，失败后做中文/常见变体包含匹配
/// （部分模型会输出中文类型名，如"游戏""聊天""唱歌"）。
fn parse_event_type(raw: &str) -> EventType {
    let t = raw.trim().to_lowercase();
    match t.as_str() {
        "intro" | "开场" | "开头" | "opening" => return EventType::Intro,
        "gameplay" | "游戏" | "游玩" | "游戏/操作" | "操作" => return EventType::Gameplay,
        "interaction" | "互动" | "连麦" | "双播" | "联动" => return EventType::Interaction,
        "qna" | "问答" | "读sc" | "读sc环节" | "q&a" | "qa" => return EventType::QnA,
        "reaction" | "反应" | "看视频" | "观影" | "鉴赏" => return EventType::Reaction,
        "storytelling" | "讲故事" | "故事" | "回忆" | "分享经历" | "叙述" => {
            return EventType::Storytelling
        }
        "music" | "唱歌" | "演唱" | "演奏" | "音乐" | "歌切" | "歌曲" => {
            return EventType::Music
        }
        "food" | "吃播" | "美食" | "试吃" | "吃饭" => return EventType::Food,
        "rant" | "吐槽" | "破防" | "红温" | "抱怨" | "发牢骚" => return EventType::Rant,
        "break" | "休息" | "中场" | "暂停" => return EventType::Break,
        "outro" | "结尾" | "结束" | "下播" | "告别" => return EventType::Outro,
        "chat" | "chant" | "聊天" | "杂谈" | "日常" | "闲聊" => return EventType::Chat,
        "eating" | "进食" => return EventType::Eating,
        "irl" | "线下" | "旅行" | "vlog" | "外出" | "户外" => return EventType::IRL,
        "ranking" | "盘点" | "排行" | "排行榜" | "排名" => return EventType::Ranking,
        "discussion" | "讨论" | "争论" | "锐评" | "辩论" => return EventType::Discussion,
        _ => {}
    }

    // 包含匹配兜底（按特征词优先级，避免"音乐游戏"等复合词误判）
    if t.contains("游戏") {
        EventType::Gameplay
    } else if t.contains("唱") || t.contains("歌") {
        EventType::Music
    } else if t.contains("吃") {
        EventType::Food
    } else if t.contains("问答") || t.contains("读sc") {
        EventType::QnA
    } else if t.contains("互动") || t.contains("连麦") {
        EventType::Interaction
    } else if t.contains("吐槽") || t.contains("红温") || t.contains("破防") {
        EventType::Rant
    } else if t.contains("聊天") || t.contains("杂谈") || t.contains("日常") {
        EventType::Chat
    } else if t.contains("盘点") || t.contains("排行") {
        EventType::Ranking
    } else if t.contains("讨论") || t.contains("争论") || t.contains("锐评") {
        EventType::Discussion
    } else if t.contains("故事") || t.contains("回忆") || t.contains("讲述") {
        EventType::Storytelling
    } else if t.contains("观影") || t.contains("鉴赏") || t.contains("看视频") {
        EventType::Reaction
    } else if t.contains("旅行") || t.contains("线下") || t.contains("vlog") {
        EventType::IRL
    } else if t.contains("休息") || t.contains("暂停") {
        EventType::Break
    } else if t.contains("结束") || t.contains("下播") {
        EventType::Outro
    } else if t.contains("开场") || t.contains("开头") {
        EventType::Intro
    } else {
        EventType::Other
    }
}

pub const DEFAULT_SYSTEM_PROMPT: &str = "\
你是一个虚拟主播/VTuber 直播切片分析助手。分析直播转录文本，\
将内容划分为多个有看点的视频片段。\
同一个大话题下的子话题切换不是事件边界，只有明显的话题转换才应切分。\
事件边界必须精确：每个事件的 start 要设在话题实际开始的位置，不要包含前一话题的残余内容。\
过渡闲聊如果属于上一个话题，就不要分给新事件。\
标题必须高信息密度，以主播原话/独白/名场面直接输出为主体，\
禁止\"主播聊\"\"主播提到\"等第三人称描述开头。\
标题结构必须多样化，连续标题不得使用相同的开头句式。\
善用具体细节（人名、数字、场景）、主播原话嵌入、反应吐槽来制造层次感。\
emoji 可选，如果使用则放在句中或句尾，不要放在标题开头。\
摘要要详尽描述内容看点。质量优先于数量，每个事件必须有明显的看点、转折、爆点或清晰的话题边界，无话题切换的过渡内容应合并。ASR 对唱歌/外语常产生乱码短句，不要照抄乱码，也不要为乱码单独建事件；主体内容优先，event_type 以整段主体为准。";

fn system_prompt(config: &DetectionConfig) -> &str {
    if !config.system_prompt.is_empty() {
        return &config.system_prompt;
    }
    DEFAULT_SYSTEM_PROMPT
}

pub async fn call(
    segments: &[TranscriptSegment],
    llm: &LlmConfig,
    detect: &DetectionConfig,
    hot_windows: &[(f64, f64)],
) -> Result<Vec<Event>> {
    let prompt = build_prompt(segments, detect, hot_windows);
    let messages = serde_json::json!([
        {"role": "system", "content": system_prompt(detect)},
        {"role": "user", "content": prompt}
    ]);
    let client = crate::llm::LlmClient::from_config(llm)?.with_timeout(300);

    // 网络层已有重试（LlmClient::send：429/5xx/超时，最多 3 次）；
    // 这里对"发送成功但内容解析失败/事件为空"再整次重试 1 次（LLM 输出截断是常见失败）。
    let mut last_err: Option<anyhow::Error> = None;
    for attempt in 1..=2 {
        match attempt_call(&client, messages.clone()).await {
            Ok(events) if !events.is_empty() => return Ok(events),
            Ok(_) => {
                tracing::warn!("LLM 返回的事件列表为空 (第 {} 次尝试)", attempt);
                last_err = Some(anyhow::anyhow!("LLM 返回的事件列表为空"));
            }
            Err(e) => {
                tracing::warn!("LLM 事件检测失败 (第 {} 次尝试): {}", attempt, e);
                last_err = Some(e);
            }
        }
    }

    Err(last_err.unwrap_or_else(|| anyhow::anyhow!("LLM 事件检测失败")))
}

async fn attempt_call(
    client: &crate::llm::LlmClient,
    messages: serde_json::Value,
) -> Result<Vec<Event>> {
    let body = client.send(messages).await?;

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

    parse_events(json_value)
}

pub async fn test_llm(llm: &LlmConfig, prompt: &str) -> Result<String> {
    if llm.api_key.is_empty() {
        anyhow::bail!("LLM API key 未配置");
    }
    let messages = serde_json::json!([
        {"role": "system", "content": "你是一个 AI 助手。"},
        {"role": "user", "content": prompt}
    ]);
    let client = crate::llm::LlmClient::from_config(llm)?.with_timeout(180);
    client.send(messages).await.map(|v| v.to_string())
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_event_type_english_keys() {
        assert_eq!(parse_event_type("gameplay"), EventType::Gameplay);
        assert_eq!(parse_event_type("CHAT"), EventType::Chat);
        assert_eq!(parse_event_type("Q&A"), EventType::QnA);
        assert_eq!(parse_event_type("music"), EventType::Music);
        assert_eq!(parse_event_type("StoryTelling"), EventType::Storytelling);
        assert_eq!(parse_event_type("unknown_thing"), EventType::Other);
    }

    #[test]
    fn test_parse_event_type_chinese_exact() {
        assert_eq!(parse_event_type("游戏"), EventType::Gameplay);
        assert_eq!(parse_event_type("聊天"), EventType::Chat);
        assert_eq!(parse_event_type("唱歌"), EventType::Music);
        assert_eq!(parse_event_type("读SC"), EventType::QnA);
        assert_eq!(parse_event_type("吃播"), EventType::Food);
        assert_eq!(parse_event_type("红温"), EventType::Rant);
        assert_eq!(parse_event_type("盘点"), EventType::Ranking);
    }

    #[test]
    fn test_parse_event_type_chinese_contains() {
        // 包含匹配兑底：模型输出带修饰词的类型名
        assert_eq!(parse_event_type("游戏/操作/互动"), EventType::Gameplay);
        assert_eq!(parse_event_type("音乐游戏"), EventType::Gameplay); // 游戏优先于音乐
        assert_eq!(parse_event_type("唱歌环节"), EventType::Music);
        assert_eq!(parse_event_type("和观众聊天"), EventType::Chat);
        assert_eq!(parse_event_type("吃零食环节"), EventType::Food);
        assert_eq!(parse_event_type("直播结束"), EventType::Outro);
    }

    #[test]
    fn test_build_prompt_injects_hot_windows() {
        let segs = vec![TranscriptSegment {
            start: 0.0,
            end: 10.0,
            text: "测试".into(),
            confidence: 1.0,
        }];
        let cfg = crate::config::DetectionConfig::default();

        // 无热点：不追加爆发段落
        let no_hot = build_prompt(&segs, &cfg, &[]);
        assert!(!no_hot.contains("弹幕爆发"));

        // 有热点：追加区间 + 要求
        let hot = build_prompt(&segs, &cfg, &[(12.0, 22.0), (45.5, 55.5)]);
        assert!(hot.contains("[12.0-22.0] 弹幕爆发"));
        assert!(hot.contains("[45.5-55.5] 弹幕爆发"));
        assert!(hot.contains("不得切在这些爆发区间内部"));
    }
}

