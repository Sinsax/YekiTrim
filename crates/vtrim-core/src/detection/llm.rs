use std::path::Path;

use anyhow::Result;
use async_trait::async_trait;

use crate::config::{DetectionConfig, LlmConfig};
use crate::llm::agent::{Agent, AgentItem, AgentProgress, AgentTask, AgentWindow};
use crate::types::{Event, EventSource, EventType, Timestamp, TranscriptSegment};

pub const DEFAULT_PROMPT: &str = r#"将直播转录切分为短视频切片。硬性要求：
- 每段 0.5~3 分钟，宁完整勿碎：优先保证事件内容完整，不要为了“碎”而把一个完整话题/名场面拦腰切断
- start 不要过早，但也不要只切爆点瞬间：应包含让观众看懂该事件所必需的短上下文；end 应到话题自然结束，不能事件还没讲完就切
- 明显话题转换才切分；同一个话题中途不要硬切
- 根据字幕数量/信息密度而不是时间长度来判断：字幕很少、间隔很大的低密度长段不要单独成段，应并入相邻完整话题；字幕密集的高能段可以单独成段
- 弹幕爆发时刻必须完整包含在某一段内，不得切在爆发区间中间
- ASR 乱码/哼唱不要单独成段

标题(label)要求（最重要）：
- 用主播原话/名场面/具体情景 + 反应/吐槽，制造反差和梗
- 高信息密度，8~40 字，禁止"主播聊/主播说"式第三人称开头
- 相邻标题句式不得重复；同一输出内混合：直接引语、事件→反应、爆点感叹、悬念反问
- emoji 可选，放句中/句尾，不放开头

摘要(summary)：40~80 字，写清核心内容和看点，尽量覆盖该事件完整内容。

参考标题：
{reference_titles}

event_type 取整段主体：gameplay、interaction、qna、reaction、storytelling、music、food、rant、chat、discussion、ranking、irl、other。唱歌/哼唱短暂穿插不算 music，整段以歌为主才算。

输出：只输出 JSON 数组，每个事件含 start/end/label/summary/event_type，不要 markdown。

转录片段：
{segments}

只输出JSON数组：
[{"start": 0.0, "end": 120.0, "label": "这操作给我看傻了??🤣", "summary": "核心看点...", "event_type": "interaction"}]"#;
/// 事件检测阶段使用的标题(label)规则，封面标题生成复用同一份规则。
pub const TITLE_RULES: &str = r#"标题(label)要求（最重要）：
- 用主播原话/名场面/具体情景 + 反应/吐槽，制造反差和梗
- 高信息密度，8~40 字，禁止"主播聊/主播说"式第三人称开头
- 相邻标题句式不得重复；同一输出内混合：直接引语、事件→反应、爆点感叹、悬念反问
- emoji 可选，放句中/句尾，不放开头
"#;

/// 内建参考标题示例：config `detection.reference_titles` 与旧版
/// prompt_template「参考标题：」段落都为空时的兜底（检测与封面标题共用）。
pub const DEFAULT_REFERENCE_TITLES: &[&str] = &[
    "弹幕问豆今色，豆脱口而出：梅川😊我去原来豆神喜欢挂空档啊🥵",
    "牢豆这什么雷霆坐姿雷霆视角🥵羞死人了🥵",
    "难绷花礼连自己拉的屎都想吃😅真是老吃家了🤤",
    "深夜鉴赏泫雅《没有明天》MV，怎么一言不合就🥵又是小时候被吓哭的一集😱",
    "刚出道时3D回不敢开暖空调，顶着0℃直播跟冰窖似的🥶当时居然能撑下来😲",
    "谁家小土豆地雷？【灰泽满Hazel】",
];

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
    let refs = crate::config::resolve_reference_titles(config);
    prompt = prompt.replace("{reference_titles}", &refs.join("\n"));

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

    // 支持 {"events": [...]} 对象包装：先尝试解析整个文本为对象
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(json_str) {
        if value.is_object() {
            if let Some(arr) = value.get("events").filter(|v| v.is_array()) {
                return Ok(arr.clone());
            }
        }
        if value.is_array() {
            return Ok(value);
        }
    }

    // 从文本中扫描所有可能的 JSON 数组/对象区间，尝试解析第一个合法事件数据
    let mut candidates: Vec<&str> = Vec::new();

    // 收集所有 '[' 到匹配 ']' 的区间（粗略：用括号深度匹配）
    let bytes = json_str.as_bytes();
    let mut stack: Vec<usize> = Vec::new();
    for (i, &b) in bytes.iter().enumerate() {
        match b {
            b'[' => stack.push(i),
            b']' => {
                if let Some(open) = stack.pop() {
                    candidates.push(&json_str[open..=i]);
                }
            }
            _ => {}
        }
    }

    // 如果没有数组，尝试收集对象区间
    if candidates.is_empty() {
        let mut stack: Vec<usize> = Vec::new();
        for (i, &b) in bytes.iter().enumerate() {
            match b {
                b'{' => stack.push(i),
                b'}' => {
                    if let Some(open) = stack.pop() {
                        candidates.push(&json_str[open..=i]);
                    }
                }
                _ => {}
            }
        }
    }

    // 按从长到短尝试（事件数组通常较长），解析第一个合法值
    candidates.sort_by_key(|s| std::cmp::Reverse(s.len()));
    for candidate in candidates {
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(candidate) {
            if value.is_array() {
                return Ok(value);
            }
            if value.is_object() {
                if let Some(arr) = value.get("events").filter(|v| v.is_array()) {
                    return Ok(arr.clone());
                }
            }
        }
    }

    anyhow::bail!("LLM 返回的 JSON 格式无效")
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

pub const DEFAULT_SYSTEM_PROMPT: &str = r#"你是直播切片分析助手。把转录切成 0.5~3 分钟的短视频切片，宁完整勿碎：优先保证每个事件是完整话题/名场面，不要为了多切而把内容拦腰截断。明显话题转换才切分；字幕很少的低密度长段不要单独成段，应并入相邻完整话题。start 包含必要上下文，end 到话题自然结束，禁止事件还没讲完就切。标题必须高信息密度，以主播原话/名场面/具体情景+反应吐槽为主体，禁止"主播聊/主播说"式第三人称开头，相邻标题句式不得重复。摘要 40~80 字。event_type 以整段主体为准，ASR 乱码不单独成段。"#;
fn system_prompt(config: &DetectionConfig) -> &str {
    if !config.system_prompt.is_empty() {
        return &config.system_prompt;
    }
    DEFAULT_SYSTEM_PROMPT
}

/// 窗口大小：每窗口最多处理多少条字幕段（避免一次塞太多导致 token 超限）
const DETECT_WINDOW_SIZE: usize = 20;
/// 窗口重叠条数：保证跨窗口的事件边界不会被切断（不宜过大，否则同一内容被多次分析）
const DETECT_WINDOW_OVERLAP: usize = 5;

/// 带 checkpoint / 进度回调的事件检测。
///
/// - `checkpoint_path`：非空时启用断点续传，每完成一个窗口立即原子写回；
///   已存在的 checkpoint 会跳过其中已完成窗口。
/// - `progress_tx`：非空时，每个窗口完成后立即发送该窗口事件（可用于边分析边保存，
///   中断后至少保留已完成窗口的结果）。
pub async fn call_with_progress(
    segments: &[TranscriptSegment],
    llm: &LlmConfig,
    detect: &DetectionConfig,
    hot_windows: &[(f64, f64)],
    checkpoint_path: Option<&Path>,
    progress_tx: Option<tokio::sync::mpsc::Sender<Vec<Event>>>,
) -> Result<Vec<Event>> {
    if segments.is_empty() {
        return Ok(Vec::new());
    }

    // 把 TranscriptSegment 转成 AgentItem（事件检测只需要时间+文本）
    let items: Vec<AgentItem> = segments
        .iter()
        .enumerate()
        .map(|(i, seg)| AgentItem {
            index: i + 1,
            start: seg.start,
            end: seg.end,
            text: seg.text.clone(),
        })
        .collect();

    // 按窗口切分，避免单次请求 token 过大（思考模式更耗 token）
    let total = items.len();
    let window_size = DETECT_WINDOW_SIZE.max(1);
    let overlap = DETECT_WINDOW_OVERLAP.min(window_size / 2);

    // 收集所有窗口（owned 副本，便于 move 到并行任务）
    let mut agent_windows: Vec<AgentWindow<Vec<Event>>> = Vec::new();
    let mut start = 0usize;
    while start < total {
        let end = (start + window_size).min(total);
        agent_windows.push(AgentWindow {
            id: format!("win_{start}"),
            offset: start,
            items: items[start..end].to_vec(),
            meta: None,
            output: None,
        });
        if end >= total {
            break;
        }
        start = end.saturating_sub(overlap);
    }

    let max_concurrent = detect.max_concurrent.max(1);
    let total_windows = agent_windows.len();
    tracing::info!("事件检测: {} 个窗口，并发 {}", total_windows, max_concurrent);

    let task = DetectionTask {
        detect: detect.clone(),
        hot_windows: hot_windows.to_vec(),
    };
    let agent = Agent::new(llm.clone()).with_max_concurrent(max_concurrent);

    // 桥接通用 Agent 进度 → 调用方的事件流（spawn 转发，避免 send 阻塞 Agent）
    let (agent_progress_tx, mut agent_progress_rx) =
        if let Some(tx) = progress_tx {
            let (agent_tx, agent_rx) = tokio::sync::mpsc::channel::<AgentProgress<Vec<Event>>>(64);
            let forwarder = tokio::spawn(async move {
                let mut rx = agent_rx;
                while let Some(progress) = rx.recv().await {
                    if let Some(events) = progress.output {
                        if tx.send(events).await.is_err() {
                            break;
                        }
                    }
                }
            });
            (Some(agent_tx), Some(forwarder))
        } else {
            (None, None)
        };

    let windows = agent
        .run_windows(task, agent_windows, checkpoint_path, agent_progress_tx)
        .await?;

    // 等待转发任务结束（此时 Agent 已关闭 channel）
    if let Some(forwarder) = agent_progress_rx.take() {
        let _ = forwarder.await;
    }

    let mut all_events: Vec<Event> = Vec::new();
    for w in windows {
        if let Some(mut events) = w.output {
            tracing::info!("窗口 {} 完成: {} 个事件", w.id, events.len());
            all_events.append(&mut events);
        }
    }

    if all_events.is_empty() {
        anyhow::bail!("LLM 事件检测失败");
    }

    // 合并相邻/重叠事件
    let merged = merge_window_events(all_events);
    Ok(merged)
}

/// 事件检测任务：实现通用 AgentTask<Vec<Event>>。
struct DetectionTask {
    detect: DetectionConfig,
    hot_windows: Vec<(f64, f64)>,
}

#[async_trait]
impl AgentTask<Vec<Event>> for DetectionTask {
    fn name(&self) -> &str {
        "事件检测"
    }

    fn thinking(&self) -> bool {
        // 思考模式由 [llm].extra_params.thinking 控制，这里不再强制开启
        false
    }

    fn build_global_context(&self, items: &[AgentItem]) -> String {
        // 事件检测目前不需要全局上下文（窗口内已含足够上下文）
        let _ = items;
        String::new()
    }

    fn build_window_context(&self, items: &[AgentItem], window: &[AgentItem], global: &str) -> String {
        // 事件检测的 prompt 直接基于窗口 segments 构建，不额外拼接上下文
        let _ = (items, window, global);
        String::new()
    }

    fn system_prompt(&self) -> String {
        system_prompt(&self.detect).to_string()
    }

    fn user_prompt(&self, global: &str, window_context: &str, window: &[AgentItem]) -> String {
        let _ = (global, window_context);
        let segments: Vec<TranscriptSegment> = window
            .iter()
            .map(|it| TranscriptSegment {
                start: it.start,
                end: it.end,
                text: it.text.clone(),
                confidence: 1.0,
            })
            .collect();
        build_prompt(&segments, &self.detect, &self.hot_windows)
    }

    fn parse_output(&self, _content: &str, _window: &[AgentItem]) -> Result<Vec<Event>> {
        // 事件检测不走默认的 content-only 解析，process_window 覆写为 attempt_call
        anyhow::bail!("DetectionTask 应使用覆写的 process_window")
    }

    async fn process_window(
        &self,
        _all_items: &[AgentItem],
        window: &AgentWindow<Vec<Event>>,
        _global: &str,
        llm_cfg: &LlmConfig,
    ) -> Result<Vec<Event>> {
        let segments: Vec<TranscriptSegment> = window
            .items
            .iter()
            .map(|it| TranscriptSegment {
                start: it.start,
                end: it.end,
                text: it.text.clone(),
                confidence: 1.0,
            })
            .collect();

        // 思考模式透传 [llm].extra_params 配置（enabled/disabled），不在此强制
        let client = crate::llm::LlmClient::from_config(llm_cfg)?.with_timeout(180);
        process_window_inner(&client, &segments, &self.detect, &self.hot_windows).await
    }

    fn save_windows(&self, windows: &[AgentWindow<Vec<Event>>]) -> Result<serde_json::Value> {
        Ok(serde_json::to_value(windows)?)
    }

    fn load_windows(&self, value: serde_json::Value) -> Result<Vec<AgentWindow<Vec<Event>>>> {
        let windows: Vec<AgentWindow<Vec<Event>>> = serde_json::from_value(value)?;
        Ok(windows)
    }
}

/// 合并来自不同窗口的事件：按时间排序，重叠或边界过近的合并（保留更长/更有信息量的标题）。
fn merge_window_events(events: Vec<Event>) -> Vec<Event> {
    if events.is_empty() {
        return events;
    }

    let mut events = events;
    events.sort_by(|a, b| {
        a.start
            .to_seconds()
            .partial_cmp(&b.start.to_seconds())
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let mut merged: Vec<Event> = Vec::new();
    for mut ev in events {
        if let Some(last) = merged.last_mut() {
            let last_start = last.start.to_seconds();
            let last_end = last.end.to_seconds();
            let ev_start = ev.start.to_seconds();
            let ev_end = ev.end.to_seconds();

            if ev_start < last_end {
                // 重叠时长 / 较短事件时长 = 重叠比例
                let overlap = last_end - ev_start;
                let last_dur = last_end - last_start;
                let ev_dur = ev_end - ev_start;
                let shorter = last_dur.min(ev_dur);
                let overlap_ratio = if shorter > 0.0 {
                    overlap / shorter
                } else {
                    1.0
                };

                if overlap_ratio >= 0.5 {
                    // 重叠 ≥50%：视为同一事件的重复，保留更长、更完整的一个
                    if ev_dur > last_dur {
                        *last = ev;
                    }
                    continue;
                }

                // 重叠 <50%：保留两个细分，后一个 start 截断到前一个 end
                // 截断后太短（<30s）则丢弃（大概率是重复噪声）
                if ev_end - last_end < 30.0 {
                    continue;
                }
                ev.start = Timestamp::from_seconds(last_end);
            }
        }
        merged.push(ev);
    }

    merged
}

/// 处理一个窗口：先完整尝试，失败且窗口较大时拆半递归。
fn process_window_inner<'a>(
    client: &'a crate::llm::LlmClient,
    segments: &'a [TranscriptSegment],
    detect: &'a DetectionConfig,
    hot_windows: &'a [(f64, f64)],
) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Vec<Event>>> + Send + 'a>> {
    Box::pin(async move {
    let win_start = segments.first().map(|s| s.start).unwrap_or(0.0);
    let win_end = segments.last().map(|s| s.end).unwrap_or(0.0);

    let window_hot: Vec<(f64, f64)> = hot_windows
        .iter()
        .copied()
        .filter(|(s, e)| s < &win_end && e > &win_start)
        .collect();

    let prompt = build_prompt(segments, detect, &window_hot);
    let messages = serde_json::json!([
        {"role": "system", "content": system_prompt(detect)},
        {"role": "user", "content": prompt}
    ]);

    // 完整窗口尝试 2 次
    let mut last_err: Option<anyhow::Error> = None;
    for attempt in 1..=2 {
        match attempt_call(client, messages.clone()).await {
            Ok(events) if !events.is_empty() => {
                tracing::info!(
                    "窗口 [{:.1}-{:.1}] 成功: {} 个事件",
                    win_start,
                    win_end,
                    events.len()
                );
                return Ok(events);
            }
            Ok(_) => {
                tracing::warn!("窗口 [{:.1}-{:.1}] 返回事件为空 (第 {} 次尝试)", win_start, win_end, attempt);
                last_err = Some(anyhow::anyhow!("LLM 返回的事件列表为空"));
            }
            Err(e) => {
                tracing::warn!("窗口 [{:.1}-{:.1}] 失败 (第 {} 次尝试): {}", win_start, win_end, attempt, e);
                last_err = Some(e);
            }
        }
    }

    // 如果窗口还大，拆半递归，避免超时后整窗丢失
    if segments.len() > 30 {
        let mid = segments.len() / 2;
        tracing::warn!(
            "窗口 [{:.1}-{:.1}] 失败，拆半为 [{:.1}-{:.1}] 和 [{:.1}-{:.1}] 重试",
            win_start,
            win_end,
            segments[0].start,
            segments[mid].end,
            segments[mid].start,
            win_end
        );

        let left = process_window_inner(client, &segments[..mid], detect, hot_windows).await?;
        let right = process_window_inner(client, &segments[mid..], detect, hot_windows).await?;
        let mut combined = left;
        combined.extend(right);
        return Ok(combined);
    }

    Err(last_err.unwrap_or_else(|| anyhow::anyhow!("窗口处理失败")))
    })
}

async fn attempt_call(
    client: &crate::llm::LlmClient,
    messages: serde_json::Value,
) -> Result<Vec<Event>> {
    let body = client.send(messages).await?;

    let content = body["choices"][0]["message"]["content"]
        .as_str()
        .unwrap_or_default();

    // 思考模式下 content 可能为空，最终 JSON 可能写在 reasoning 末尾
    let reasoning = body["choices"][0]["message"]["reasoning"]
        .as_str()
        .unwrap_or_default();

    let parse_sources = [content.trim(), reasoning.trim()]
        .iter()
        .filter(|s| !s.is_empty())
        .copied()
        .collect::<Vec<_>>();

    if parse_sources.is_empty() {
        anyhow::bail!(
            "LLM 返回 content 和 reasoning 均为空（可能是 max_tokens 过小或模型异常）"
        );
    }

    let mut last_err: Option<anyhow::Error> = None;
    for source in &parse_sources {
        match parse_json(source) {
            Ok(v) => {
                return parse_events(v).map_err(|e| {
                    anyhow::anyhow!("解析事件失败: {e:#}")
                });
            }
            Err(e) => {
                last_err = Some(e);
            }
        }
    }

    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let log_path = std::path::PathBuf::from(format!("vtrim_llm_debug_{}.log", ts));
    let full = format!(
        "--- content ---\n{}\n\n--- reasoning ---\n{}\n\n--- full response ---\n{}",
        content,
        reasoning,
        serde_json::to_string_pretty(&body).unwrap_or_default()
    );
    let _ = std::fs::write(&log_path, &full);
    tracing::warn!(
        "LLM 响应解析失败: {}，已保存到 {}",
        last_err.as_ref().map(|e| e.to_string()).unwrap_or_default(),
        log_path.display()
    );
    Err(last_err.unwrap_or_else(|| anyhow::anyhow!("LLM 响应 JSON 解析失败")))
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
        // 注：默认 user prompt 模板本身就含"弹幕爆发"字样（见文件顶部 prompt），
        // 所以这里断言的是"注入的爆发区间段落"没有出现，而不是这个词本身。
        let no_hot = build_prompt(&segs, &cfg, &[]);
        assert!(!no_hot.contains("观众弹幕爆发时刻"));

        // 有热点：追加区间 + 要求
        let hot = build_prompt(&segs, &cfg, &[(12.0, 22.0), (45.5, 55.5)]);
        assert!(hot.contains("[12.0-22.0] 弹幕爆发"));
        assert!(hot.contains("[45.5-55.5] 弹幕爆发"));
        assert!(hot.contains("不得切在这些爆发区间内部"));
    }

    #[test]
    fn test_merge_window_events_does_not_merge_far_apart_duplicates() {
        fn ev(s: f64, e: f64, label: &str) -> Event {
            Event {
                start: Timestamp::from_seconds(s),
                end: Timestamp::from_seconds(e),
                label: label.into(),
                summary: String::new(),
                event_type: EventType::Chat,
                source: EventSource::LLM,
                confidence: 1.0,
                popularity: 0.0,
                rank: 0,
                enabled: true,
            }
        }

        // 两个重叠窗口各自输出同一大话题的长事件，边界差 60s。
        // 只是相邻不重叠，不应合并。
        let events = vec![
            ev(100.0, 700.0, "A"),
            ev(760.0, 1300.0, "B"),
        ];
        let merged = merge_window_events(events);
        assert_eq!(merged.len(), 2);
    }

    #[test]
    fn test_merge_window_events_merges_high_overlap_keeps_longer() {
        fn ev(s: f64, e: f64, label: &str, etype: EventType) -> Event {
            Event {
                start: Timestamp::from_seconds(s),
                end: Timestamp::from_seconds(e),
                label: label.into(),
                summary: String::new(),
                event_type: etype,
                source: EventSource::LLM,
                confidence: 1.0,
                popularity: 0.0,
                rank: 0,
                enabled: true,
            }
        }

        // 重叠 500/600 ≈ 83% ≥50%：视为重复，保留更长者
        let events = vec![
            ev(100.0, 700.0, "短", EventType::Chat),
            ev(200.0, 900.0, "长", EventType::Gameplay),
        ];
        let merged = merge_window_events(events);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].label, "长");
        assert_eq!(merged[0].start.to_seconds(), 200.0);
        assert_eq!(merged[0].end.to_seconds(), 900.0);
    }

    #[test]
    fn test_merge_window_events_keeps_low_overlap_fine_grained() {
        fn ev(s: f64, e: f64, label: &str, etype: EventType) -> Event {
            Event {
                start: Timestamp::from_seconds(s),
                end: Timestamp::from_seconds(e),
                label: label.into(),
                summary: String::new(),
                event_type: etype,
                source: EventSource::LLM,
                confidence: 1.0,
                popularity: 0.0,
                rank: 0,
                enabled: true,
            }
        }

        // 重叠 100/400 = 25% <50%：保留两个细分，后一个 start 截断
        let events = vec![
            ev(0.0, 400.0, "前", EventType::Chat),
            ev(300.0, 700.0, "后", EventType::Gameplay),
        ];
        let merged = merge_window_events(events);
        assert_eq!(merged.len(), 2);
        assert_eq!(merged[0].label, "前");
        assert_eq!(merged[0].start.to_seconds(), 0.0);
        assert_eq!(merged[0].end.to_seconds(), 400.0);
        assert_eq!(merged[1].label, "后");
        assert_eq!(merged[1].start.to_seconds(), 400.0);
        assert_eq!(merged[1].end.to_seconds(), 700.0);
    }
}

