//! detection::llm：检测侧 prompt 与 LLM 调用（单窗口请求 / 重试 / 传播潜力评分）。
//!
//! 入口：`score_events_with_llm`、`call_with_progress`、`test_llm`、`create_default_event`、`parse_json`、`DEFAULT_PROMPT` / `DEFAULT_SYSTEM_PROMPT` / `TITLE_RULES`。
//! 边界：`DEFAULT_PROMPT` 里 `{segments}` 必须留在最后（provider 前缀缓存）；改内建 prompt 对 `.yekitrim/config.toml` 里的副本无效 —— 见 `crates/yekitrim-core/src/detection/AGENTS.md`

use std::path::Path;
use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;

use crate::config::{DetectionConfig, LlmConfig};
use yekitrim_llm::agent::{Agent, AgentItem, AgentProgress, AgentTask, AgentWindow};
use crate::types::{Event, EventSource, EventType, Timestamp, TranscriptSegment};

pub const DEFAULT_PROMPT: &str = r#"将直播转录切分为短视频切片。硬性要求：
- 时长服从话题完整：一般 0.5~3 分钟，话题没讲完就让它更长（4~6 分钟也行），不要为了压进 3 分钟而硬切
- **禁止把同一次连续事件等分成多段**：不要产出「xxx（1/2）」「xxx（2/2）」这种同题分段；要么完整成一段，要么各自的标题独立成立（各自是不同话题）
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

{hot_windows}

只输出JSON数组：
[{"start": 0.0, "end": 120.0, "label": "这操作给我看傻了??🤣", "summary": "核心看点...", "event_type": "interaction"}]

转录片段：

{segments}"#;
/// 事件检测阶段使用的标题(label)规则，封面标题生成复用同一份规则。
pub const TITLE_RULES: &str = r#"标题(label)要求（最重要）：
- 用主播原话/名场面/具体情景 + 反应/吐槽，制造反差和梗
- 高信息密度，8~40 字，禁止"主播聊/主播说"式第三人称开头
- 相邻标题句式不得重复；同一输出内混合：直接引语、事件→反应、爆点感叹、悬念反问
- emoji 可选，放句中/句尾，不放开头
"#;

/// **分散片段第二趟**的 system prompt（低标准：只要小段）。
///
/// 与第一趟（`DEFAULT_SYSTEM_PROMPT`）**口径相反**：第一趟"完整话题优先、低密度长段并走"，
/// 这一趟"宁可多报、不要漏报"，并由宿主按 `topic` 归组合并、按代码里的硬门槛丢弃。
pub const SCATTER_SYSTEM_PROMPT: &str = r#"你是直播切片分析助手。这一趟**只要小段**：把转录切成几十秒的零碎片段，宁可多报、不要漏报（后面会自动合并与丢弃）；每一段必须给一个 topic 短键，同一话题必须用完全一样的字符串。"#;

/// **分散片段第二趟**的 user prompt（`{segments}` 必须留在最后 —— 前缀缓存）。
pub const SCATTER_PROMPT: &str = r#"把直播转录切成**尽可能小**的候选片段（宿主随后按话题把它们合并成完整片段）。硬性要求：
- 单段时长 {scatter_min_secs}~{scatter_max_secs} 秒；一小段来回（一个梗、一句吐槽、一次互动、一段副歌）就值得单独成段
- **宁可多报，不要漏报**：拿不准要不要切就切出来 —— 后面会按 topic 合并、按标准丢弃
- 同一个话题被别的话题**打断**时，被打断的每一块都要单独报出来，并给它们**同一把 topic 键**（这正是这一趟的意义）
- 明显话题转换处才断开；同一句话不要拆开；ASR 乱码/纯哼唱不要单独成段

topic（归组键，这一趟最重要）：
- 6~10 个汉字以内，说清这一段在讲什么（如「唱歌」「上班吐槽」「情感问题」「游戏操作」）
- **同一个话题必须用一模一样、一字不差的字符串**（宿主靠它把分散的几块并成一条）；换了话题就换键

标题(label)：8~40 字，用主播原话/名场面/具体情景 + 反应吐槽，禁止"主播聊/主播说"式第三人称开头
摘要(summary)：20~60 字

参考标题：
{reference_titles}

event_type 取整段主体：gameplay、interaction、qna、reaction、storytelling、music、food、rant、chat、discussion、ranking、irl、other。

输出：只输出 JSON 数组，每个元素含 start/end/label/summary/event_type/topic，不要 markdown。

{hot_windows}

只输出JSON数组：
[{"start": 0.0, "end": 42.0, "label": "这操作给我看傻了??🤣", "summary": "核心看点...", "event_type": "interaction", "topic": "游戏操作"}]

转录片段：

{segments}"#;

/// 检测的两趟：第一趟（普通事件，老口径）/ 第二趟（分散片段，低标准）。
///
/// ⚠️ 它同时是 **checkpoint 指纹的一部分**（见 `DetectionTask::prompt_fingerprint`），
/// 两趟各写各的断点文件，别混。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DetectPass {
    Main,
    Scattered,
}

impl DetectPass {
    pub fn is_scattered(self) -> bool {
        matches!(self, DetectPass::Scattered)
    }
}

/// 内建参考标题示例：config `detection.reference_titles` 与旧版
/// prompt_template「参考标题：」段落都为空时的兜底（检测与封面标题共用）。
///
/// # 这几条是**格式示范**，不是某个主播的数据
///
/// ⚠️ 2026-10 换过一次：原来那 6 条是**某位具体主播的真实标题**（含其昵称、某场直播的
/// 具体梗，如「谁家小土豆地雷？【灰泽满Hazel】」）。三个问题：
/// ① 对**别的**主播，注入的是别人的标题 —— LLM 会照着学那个人的口吻与题材；
/// ② 里面挂着具体人物名，进了仓库就等于把某个人的数据当成全产品的默认值；
/// ③ 用户看不出"我没配，所以用的是内建那几条"（设置页是个空框，见 `SettingsView.vue`）。
///
/// 换成**中性示范**：保留要教给 LLM 的**结构特征**，把"是谁"抽掉 ——
/// ① 高信息密度、以主播原话/具体情景为主体（不是"主播聊…"的第三人称概括）；
/// ② 带反应/吐槽，常用 emoji 收尾；
/// ③ 标题长短错落，句式互不重复（避免 LLM 学成同一套模板）。
///
/// ⚠️ 改这几条**会改变所有用内建默认的存量用户的出片标题风格** —— 不是纯展示改动。
/// 要改先确认这是有意为之，别当成"改个文案"。
pub const DEFAULT_REFERENCE_TITLES: &[&str] = &[
    "弹幕问我今天穿什么，我脱口而出：这身啊😊结果被截图做成表情包了🥵",
    "这什么雷霆坐姿雷霆视角🥵羞死人了🥵",
    "深夜挑战恐怖游戏，怎么一言不合就🥵又是被吓到叫出声的一集😱",
    "刚开播那会儿连暖空调都不敢开，顶着0℃跟冰窖似的🥶居然也撑下来了😲",
    "第 3 次了，这个操作我真的会谢😅",
    "谁家小土豆地雷啊，这也太可爱了吧🥺",
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
    pass: DetectPass,
) -> String {
    let sc = &config.scattered;
    let template = match pass {
        DetectPass::Main if !config.prompt_template.is_empty() => &config.prompt_template,
        DetectPass::Main => DEFAULT_PROMPT,
        DetectPass::Scattered if !sc.prompt_template.is_empty() => &sc.prompt_template,
        DetectPass::Scattered => SCATTER_PROMPT,
    };
    let seg_text = format_segments(segments);
    let hot = hot_windows_block(hot_windows);
    let refs = crate::config::resolve_reference_titles(config).join("\n");

    // 前缀缓存：provider（DeepSeek 系等）按**最长公共前缀**命中上下文缓存，
    // 所以每窗口才变化的内容要尽量靠后——模板里 {segments} 收尾，
    // 每场固定不变的弹幕爆发块经 {hot_windows} 排在它之前。
    // 老模板（用户 config.toml 里的副本）没有 {hot_windows}：拼进 {segments} 的替换值里，
    // 位置等价、不丢内容，只是公共前缀短一点。
    let has_hot_slot = template.contains("{hot_windows}");
    let seg_value = format!("{hot}{}", seg_text.trim());
    template
        .replace("{hot_windows}", hot.trim_end())
        .replace(
            "{segments}",
            if has_hot_slot {
                seg_text.trim()
            } else {
                &seg_value
            },
        )
        .replace("{reference_titles}", &refs)
        // 第二趟的两个昼夜额度写进 prompt（用户改配置即可，不用改 prompt 正文）
        .replace("{scatter_min_secs}", &format!("{:.0}", sc.min_secs))
        .replace("{scatter_max_secs}", &format!("{:.0}", sc.max_secs))
}

/// 弹幕爆发时刻块：让 LLM 看到观众反应最激烈的名场面位置。
///
/// **每场固定、与窗口无关**，所以放在变化部分之前吃前缀缓存。无数据时返回空串。
fn hot_windows_block(hot_windows: &[(f64, f64)]) -> String {
    if hot_windows.is_empty() {
        return String::new();
    }
    let mut s = String::from(
        "\n\n=== 观众弹幕爆发时刻（弹幕热力图统计，以下时间区间内弹幕密度最高，是全场观众反应最激烈的名场面） ===\n",
    );
    for (start, end) in hot_windows {
        s.push_str(&format!("[{start:.1}-{end:.1}] 弹幕爆发\n"));
    }
    s.push_str(
        "要求：\n\
         - 这些时刻必须被包含在某个事件内，绝不能被遗漏\n\
         - 事件边界不得切在这些爆发区间内部（爆发时刻不能落在两个事件的交界处或事件之外）\n\
         - 若爆发区间横跨两段话题，将其归入与之更相关的事件\n",
    );
    s
}

/// 响应被输出上限截断（`finish_reason == "length"` / Anthropic 风格 `stop_reason == "max_tokens"`）。
///
/// 单独成一个错误类型是因为**同样的参数重试必然再截断**：调用方要换策略（拆窗 / 提示调大
/// `[llm].max_tokens`），而不是照原样再发一次。实测一个 4096 的窗口重试两次要白烧 ~45s 与
/// 两份输出 token，最后还是整窗丢掉。
#[derive(Debug)]
pub(crate) struct TruncatedOutput {
    pub reason: String,
    pub completion_tokens: u64,
}

impl std::fmt::Display for TruncatedOutput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "LLM 输出被 max_tokens 截断（finish_reason={}，已用满 {} 输出 token）：\
             调大 [llm].max_tokens，或让检测窗口更小",
            self.reason, self.completion_tokens
        )
    }
}

impl std::error::Error for TruncatedOutput {}

/// 响应是否被输出上限截断；是则带上原因与已用输出 token 数。
fn truncation_of(body: &serde_json::Value) -> Option<TruncatedOutput> {
    let choice = body.get("choices").and_then(|c| c.get(0));
    let reason = choice
        .and_then(|c| c.get("finish_reason"))
        .or_else(|| choice.and_then(|c| c.get("stop_reason")))
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    if !matches!(reason, "length" | "max_tokens" | "max_output_tokens") {
        return None;
    }
    Some(TruncatedOutput {
        reason: reason.to_string(),
        completion_tokens: body["usage"]["completion_tokens"].as_u64().unwrap_or(0),
    })
}

/// 从**被截断**的文本里抢救已经完整的 JSON 数组元素。
///
/// max_tokens 用尽时数组会停在中途（`[{…},{…},{"start":12`），整段判废等于把一个窗口的结果
/// 全丢掉。这里用状态机扫出每个顶层元素的收尾位置（字符串里的括号不计数），从后往前补一个
/// `]` 试解析，命中就返回「截断前已经完整的那些元素」。
///
/// 只处理**未闭合**的数组：已闭合的数组交给正常解析，它解析失败就是真的坏了（比如键没引号），
/// 那种情况不该被"抢救"成半截结果。
fn salvage_truncated_array(text: &str) -> Option<serde_json::Value> {
    let start = text.find('[')?;
    let body = &text[start..];
    let mut depth = 0i32;
    let mut in_str = false;
    let mut escaped = false;
    let mut cuts: Vec<usize> = Vec::new();

    for (i, b) in body.bytes().enumerate() {
        if in_str {
            if escaped {
                escaped = false;
            } else if b == b'\\' {
                escaped = true;
            } else if b == b'"' {
                in_str = false;
            }
            continue;
        }
        match b {
            b'"' => in_str = true,
            b'[' | b'{' => depth += 1,
            // 数组自己闭合了 → 不是截断，别抢正常解析的判定权
            b']' => {
                depth -= 1;
                if depth <= 0 {
                    return None;
                }
            }
            // 顶层元素（depth 2 → 1）的收尾处是一个可截断点
            b'}' => {
                depth -= 1;
                if depth == 1 {
                    cuts.push(i + 1);
                }
            }
            _ => {}
        }
    }

    for cut in cuts.iter().rev() {
        let candidate = format!("{}]", &body[..*cut]);
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(&candidate) {
            if value.is_array() {
                return Some(value);
            }
        }
    }
    None
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

    // 整段解析不出来时最后试一次「截断抢救」：把截断前已经完整的元素捡回来 ——
    // 丢一个窗口（实测事件覆盖率因此掉到 46%）比接受半截结果糟得多。
    if let Some(value) = salvage_truncated_array(json_str) {
        tracing::warn!(
            "LLM 输出疑似被 max_tokens 截断：已从截断的 JSON 里抢救出 {} 条完整结果",
            value.as_array().map(|a| a.len()).unwrap_or(0)
        );
        return Ok(value);
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
    /// **分散片段第二趟**的归组键（第一趟的普通事件不输出它 ⇒ 恒为 None）。
    #[serde(default)]
    topic: Option<String>,
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
                clip_dir: None,
                enabled: true,
                ranges: Vec::new(),
                // 第二趟的候选块会带 `topic`（归组键）；第一趟的普通事件没有
                topic: r.topic,
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
        "gameplay" | "游戏" | "游玩" | "游戏/操作" | "操作" => {
            return EventType::Gameplay
        }
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

pub const DEFAULT_SYSTEM_PROMPT: &str = r#"你是直播切片分析助手。把转录切成以话题为单位的短视频切片（一般 0.5~3 分钟，话题没讲完就让它更长）：优先保证每个事件是完整话题/名场面，不要为了多切而把内容拦腰截断，更不要把一个连续事件等分成「（1/2）」「（2/2）」这种同题分段。明显话题转换才切分；字幕很少的低密度长段不要单独成段，应并入相邻完整话题。start 包含必要上下文，end 到话题自然结束，禁止事件还没讲完就切。标题必须高信息密度，以主播原话/名场面/具体情景+反应吐槽为主体，禁止"主播聊/主播说"式第三人称开头，相邻标题句式不得重复。摘要 40~80 字。event_type 以整段主体为准，ASR 乱码不单独成段。"#;
fn system_prompt(config: &DetectionConfig) -> &str {
    if !config.system_prompt.is_empty() {
        return &config.system_prompt;
    }
    DEFAULT_SYSTEM_PROMPT
}

/// 第二趟（分散片段）的 system prompt：自己的覆盖项 → 内建。
fn scatter_system_prompt(config: &DetectionConfig) -> &str {
    if !config.scattered.system_prompt.is_empty() {
        return &config.scattered.system_prompt;
    }
    SCATTER_SYSTEM_PROMPT
}

/// 按趟取 system prompt（窗口调用只认这一处，别在别处再分支一遍）。
fn system_prompt_for(config: &DetectionConfig, pass: DetectPass) -> &str {
    match pass {
        DetectPass::Main => system_prompt(config),
        DetectPass::Scattered => scatter_system_prompt(config),
    }
}

/// 标题+摘要传播潜力评分的 system prompt。
///
/// 评分阶段**只**喂 `label` + `summary`，不带转录、时间、event_type，
/// 避免模型用"正文里有没有梗"代替"标题/摘要本身有没有吸引力"。
const SCORE_SYSTEM_PROMPT: &str = r#"你是短视频爆款潜力评估器。你只根据用户给出的「标题」和「摘要」评分，禁止引用或脑补未提供的正文、时间、事件类型等信息。"#;

/// 单次评分请求最多包含多少条事件（只含标题+摘要，40 条通常远小于上下文上限）。
const SCORE_BATCH_SIZE: usize = 40;

/// 用 LLM 给事件做标题+摘要传播潜力评分。
///
/// 返回与 `events` 等长的 `Vec<Option<f64>>`：成功条目是 0~1 的归一化分，
/// 失败/漏评条目是 `None`，调用方按 `llm_score_weight` 加权，不因评分失败丢掉检测结果。
/// `weight <= 0` 或非有限值时直接返回全 `None`，不发请求。
pub async fn score_events_with_llm(
    events: &[Event],
    llm: &LlmConfig,
    detect: &DetectionConfig,
    project: Option<&Path>,
    cancel: Option<yekitrim_llm::agent::CancelProbe>,
) -> Result<Vec<Option<f64>>> {
    if events.is_empty() {
        return Ok(Vec::new());
    }
    // 取消探针在**建客户端之前**先查一次：用户在检测阶段点了停止时，不该再为评分发任何请求
    //（顺带让"探针一开始就为真"成为一条不需要网络的单测）。
    if cancel.as_ref().is_some_and(|c| c()) {
        anyhow::bail!("任务已取消");
    }
    let weight = if detect.llm_score_weight.is_finite() {
        detect.llm_score_weight.clamp(0.0, 1.0)
    } else {
        0.0
    };
    if weight <= 0.0 {
        return Ok(vec![None; events.len()]);
    }

    let usage_project = project.map(crate::path::display);
    // 链路解析：热度评分与检测窗口同属一次检测，共用 `[llm.overrides.detect]`
    let llm = llm.for_link(crate::config::LlmLink::Detect);
    let client = Arc::new(
        yekitrim_llm::LlmClient::new(&llm)?
            .with_timeout(120)
            .with_usage_context(
                yekitrim_llm::usage::UsageContext::new("事件热度LLM评分").project(usage_project),
            ),
    );
    let max_concurrent = llm.concurrency(detect.max_concurrent).max(1);

    let all_items: Vec<(usize, String, String)> = events
        .iter()
        .enumerate()
        .map(|(i, e)| (i + 1, e.label.clone(), e.summary.clone()))
        .collect();
    let batches: Vec<Vec<(usize, String, String)>> = all_items
        .chunks(SCORE_BATCH_SIZE)
        .map(|chunk| chunk.to_vec())
        .collect();

    // 与检测窗口共用同一个并发旋钮；一个批次失败不影响其它批次。
    let semaphore = Arc::new(tokio::sync::Semaphore::new(max_concurrent));
    let mut join_set = tokio::task::JoinSet::new();
    for batch in batches {
        // 派发前查一次：命中就掐掉在飞的批次并报取消（尺度与 `Agent::run_windows` 一致：
        // 派发每个窗口前 + 每收一个窗口后各查一次，只在阶段边界查等于取消按钮形同虚设）。
        if cancel.as_ref().is_some_and(|c| c()) {
            join_set.abort_all();
            anyhow::bail!("任务已取消");
        }
        let permit = semaphore
            .clone()
            .acquire_owned()
            .await
            .map_err(|e| anyhow::anyhow!("LLM 评分并发控制失败: {e}"))?;
        let client = Arc::clone(&client);
        join_set.spawn(async move {
            let _permit = permit;
            score_batch(&client, &batch).await
        });
    }

    let mut scores: Vec<Option<f64>> = vec![None; events.len()];
    let mut ok_batches = 0usize;
    let mut failed_batches = 0usize;
    while let Some(joined) = join_set.join_next().await {
        // 每收一批再查一次：剩下的批次不该在用户点了停止之后继续跑完
        if cancel.as_ref().is_some_and(|c| c()) {
            join_set.abort_all();
            anyhow::bail!("任务已取消");
        }
        match joined {
            Ok(Ok(batch_scores)) => {
                ok_batches += 1;
                for (index, score) in batch_scores {
                    if let Some(slot) = scores.get_mut(index.saturating_sub(1)) {
                        *slot = Some(score);
                    }
                }
            }
            Ok(Err(e)) => {
                failed_batches += 1;
                tracing::warn!("LLM 热度评分批次失败: {e:#}");
            }
            Err(e) => {
                failed_batches += 1;
                tracing::warn!("LLM 热度评分任务失败: {e}");
            }
        }
    }

    if ok_batches == 0 {
        anyhow::bail!("LLM 热度评分全部失败");
    }
    if failed_batches > 0 {
        tracing::warn!(
            "LLM 热度评分部分失败: {} 批成功 / {} 批失败，失败条目保留原热度",
            ok_batches,
            failed_batches
        );
    }
    Ok(scores)
}

/// 构建单批评分 user prompt：**只包含 label + summary**。
fn build_score_prompt(batch: &[(usize, String, String)]) -> String {
    let mut out = String::from(
        "请对下列直播切片做传播潜力评分。\n\n\
         评分标准（在心里加权后给一个综合分）：\n\
         - 预测点击率/标题吸引力 40%\n\
         - 有趣程度/情绪价值 30%\n\
         - 爆点强度/冲突反转/名场面 30%\n\n\
         要求：\n\
         - 只依据每条给出的标题+摘要，不要假设或补充其它信息\n\
         - 0~100 整数；50=普通，70=值得一看，85+=强爆款潜力，95+=顶级\n\
         - 必须拉开差距，禁止所有条目同分或机械分布\n\
         - 只输出 JSON：{\"scores\":[{\"index\":1,\"score\":86}, ...]}\n\
         - index 与下方编号一致，数量一致，不要输出解释\n\n",
    );
    for (index, label, summary) in batch {
        out.push_str(&format!("{index}. 标题：{label}\n   摘要：{summary}\n"));
    }
    out
}

/// 调用一批评分，兼容 content / reasoning 两种返回位置。
async fn score_batch(
    client: &yekitrim_llm::LlmClient,
    batch: &[(usize, String, String)],
) -> Result<Vec<(usize, f64)>> {
    let prompt = build_score_prompt(batch);
    let messages = serde_json::json!([
        {"role": "system", "content": SCORE_SYSTEM_PROMPT},
        {"role": "user", "content": prompt}
    ]);
    let body = client.send(messages).await?;
    let content = body["choices"][0]["message"]["content"]
        .as_str()
        .unwrap_or_default();
    let reasoning = body["choices"][0]["message"]["reasoning"]
        .as_str()
        .unwrap_or_default();

    let mut last_err: Option<anyhow::Error> = None;
    for source in [content, reasoning] {
        if source.trim().is_empty() {
            continue;
        }
        match parse_score_response(source, batch) {
            Ok(scores) if !scores.is_empty() => return Ok(scores),
            Ok(_) => last_err = Some(anyhow::anyhow!("LLM 热度评分结果为空")),
            Err(e) => last_err = Some(e),
        }
    }
    Err(last_err.unwrap_or_else(|| anyhow::anyhow!("LLM 热度评分响应为空")))
}

/// 解析评分 JSON；兼容 `{"scores":[...]}` 和裸数组。
fn parse_score_response(
    text: &str,
    batch: &[(usize, String, String)],
) -> Result<Vec<(usize, f64)>> {
    let value = parse_json(text)?;
    let arr = value
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("LLM 热度评分 JSON 不是数组"))?;
    let valid: std::collections::HashSet<usize> =
        batch.iter().map(|(index, _, _)| *index).collect();

    let mut out = Vec::new();
    for (pos, item) in arr.iter().enumerate() {
        let index = extract_score_index(item)
            .or_else(|| batch.get(pos).map(|(index, _, _)| *index))
            .ok_or_else(|| anyhow::anyhow!("LLM 热度评分缺少 index"))?;
        if !valid.contains(&index) {
            anyhow::bail!("LLM 热度评分 index {index} 不在本批范围内");
        }
        let raw_score = extract_score_value(item)
            .ok_or_else(|| anyhow::anyhow!("LLM 热度评分 index {index} 缺少 score"))?;
        if !raw_score.is_finite() {
            anyhow::bail!("LLM 热度评分 index {index} 不是有效数字");
        }
        out.push((index, (raw_score / 100.0).clamp(0.0, 1.0)));
    }
    if out.is_empty() {
        anyhow::bail!("LLM 热度评分结果为空");
    }
    Ok(out)
}

fn extract_score_index(item: &serde_json::Value) -> Option<usize> {
    if let Some(obj) = item.as_object() {
        for key in ["index", "id", "idx", "编号", "序号"] {
            if let Some(n) = obj.get(key).and_then(value_as_usize) {
                return Some(n);
            }
        }
    }
    item.as_array()
        .and_then(|arr| arr.first())
        .and_then(value_as_usize)
}

fn extract_score_value(item: &serde_json::Value) -> Option<f64> {
    if let Some(obj) = item.as_object() {
        for key in ["score", "heat", "heat_score", "rating", "热度", "得分", "分数"] {
            if let Some(n) = obj.get(key).and_then(value_as_f64) {
                return Some(n);
            }
        }
    }
    if let Some(arr) = item.as_array() {
        return arr.get(1).and_then(value_as_f64);
    }
    value_as_f64(item)
}

fn value_as_usize(value: &serde_json::Value) -> Option<usize> {
    match value {
        serde_json::Value::Number(n) => n
            .as_f64()
            .filter(|f| f.is_finite() && f.fract() == 0.0)
            .map(|f| f as usize),
        serde_json::Value::String(s) => s.trim().parse::<usize>().ok(),
        _ => None,
    }
}

fn value_as_f64(value: &serde_json::Value) -> Option<f64> {
    match value {
        serde_json::Value::Number(n) => n.as_f64(),
        serde_json::Value::String(s) => {
            let s = s.trim().trim_end_matches('%').trim_end_matches('分').trim();
            s.parse::<f64>().ok()
        }
        _ => None,
    }
}

/// 窗口大小：每窗口最多处理多少条字幕段（避免一次塞太多导致 token 超限）
const DETECT_WINDOW_SIZE: usize = 30;
/// 窗口重叠条数：保证跨窗口的事件边界不会被切断（不宜过大，否则同一内容被多次分析）
const DETECT_WINDOW_OVERLAP: usize = 5;

/// 带 checkpoint / 进度回调的事件检测。
///
/// - `checkpoint_path`：非空时启用断点续传，每完成一个窗口立即原子写回；
///   已存在的 checkpoint 会跳过其中已完成窗口。
/// - `progress_tx`：非空时，每个窗口完成后立即发送该窗口事件（可用于边分析边保存，
///   中断后至少保留已完成窗口的结果）。
/// - `cancel`：取消探针。给了就在**每个窗口派发前**与每收一个窗口后检查，命中即中止
///   （含 abort 在飞的窗口），返回「任务已取消」而不是继续把剩下的窗口跑完。
pub async fn call_with_progress(
    segments: &[TranscriptSegment],
    llm: &LlmConfig,
    detect: &DetectionConfig,
    hot_windows: &[(f64, f64)],
    project: Option<&Path>,
    checkpoint_path: Option<&Path>,
    cancel: Option<yekitrim_llm::agent::CancelProbe>,
    progress_tx: Option<tokio::sync::mpsc::Sender<Vec<Event>>>,
    pass: DetectPass,
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

    let max_concurrent = llm.concurrency(detect.max_concurrent);
    let total_windows = agent_windows.len();
    tracing::info!(
        "事件检测: {} 个窗口，并发 {}",
        total_windows,
        max_concurrent
    );

    let task = DetectionTask {
        detect: detect.clone(),
        hot_windows: hot_windows.to_vec(),
        project: project.map(crate::path::display),
        pass,
    };
    // 链路解析：窗口调用走 `[llm.overrides.detect]`（Agent 的 llm 会传给 `process_window` 建客户端）
    let agent = Agent::new(llm.for_link(crate::config::LlmLink::Detect))
        .with_max_concurrent(max_concurrent);
    let agent = match cancel {
        Some(probe) => agent.with_cancel(probe),
        None => agent,
    };

    // 桥接通用 Agent 进度 → 调用方的事件流（spawn 转发，避免 send 阻塞 Agent）
    let (agent_progress_tx, mut agent_progress_rx) = if let Some(tx) = progress_tx {
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
        // 第二趟"一条都没报"是**合法结果**（门槛高/这段确实没有零碎话题）：不能当成检测失败，
        // 否则打开开关的副作用是"整个检测失败"。
        if pass.is_scattered() {
            tracing::info!("分散片段第二趟：窗口全部返回空 → 本次没有候选块");
            return Ok(Vec::new());
        }
        anyhow::bail!("LLM 事件检测失败");
    }

    // 合并相邻/重叠事件。
    // ⚠️ 门槛按趟取：第二趟**不许**用第一趟那条 30s 丢弃线（小块也是块，否则等于白开）。
    let merged = match pass {
        DetectPass::Main => merge_window_events(all_events),
        DetectPass::Scattered => merge_window_events_min(all_events, 0.0),
    };
    Ok(merged)
}

/// 事件检测任务：实现通用 AgentTask<Vec<Event>>。
struct DetectionTask {
    detect: DetectionConfig,
    hot_windows: Vec<(f64, f64)>,
    /// 项目根目录（`output_dir` 的父级），只用于用量记账归因。
    project: Option<String>,
    /// 这一趟是普通事件还是分散片段（决定 prompt / system prompt / 用量归因文案）。
    pass: DetectPass,
}

#[async_trait]
impl AgentTask<Vec<Event>> for DetectionTask {
    fn name(&self) -> &str {
        match self.pass {
            DetectPass::Main => "事件检测",
            DetectPass::Scattered => "事件检测（分散片段）",
        }
    }

    fn usage_context(&self) -> yekitrim_llm::usage::UsageContext {
        yekitrim_llm::usage::UsageContext::new(self.name()).project(self.project.clone())
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

    fn build_window_context(
        &self,
        items: &[AgentItem],
        window: &[AgentItem],
        global: &str,
    ) -> String {
        // 事件检测的 prompt 直接基于窗口 segments 构建，不额外拼接上下文
        let _ = (items, window, global);
        String::new()
    }

    fn system_prompt(&self) -> String {
        system_prompt_for(&self.detect, self.pass).to_string()
    }

    /// system/user prompt 的指纹 → 折进窗口 id。改了 prompt 之后重跑检测，
    /// checkpoint 里的旧窗口会全部失配、重新调用，而不是拿回上一版的结果。
    fn prompt_fingerprint(&self) -> String {
        let mut parts = vec![
            self.system_prompt(),
            match self.pass {
                DetectPass::Main => self.detect.prompt_template.clone(),
                DetectPass::Scattered => self.detect.scattered.prompt_template.clone(),
            },
            // 趟名：两趟的 prompt 撞上时（比如用户把第二趟模板填成第一趟那份）
            // 也不许共用断点文件里的同一批窗口
            if self.pass.is_scattered() {
                "pass=scattered".to_string()
            } else {
                "pass=main".to_string()
            },
            crate::config::resolve_reference_titles(&self.detect).join(
                "
",
            ),
        ];
        for (s, e) in &self.hot_windows {
            parts.push(format!("{s:.1}-{e:.1}"));
        }
        let refs: Vec<&str> = parts.iter().map(|p| p.as_str()).collect();
        yekitrim_llm::agent::fingerprint(&refs)
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
        build_prompt(&segments, &self.detect, &self.hot_windows, self.pass)
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
        llm_cfg: &yekitrim_llm::LlmParams,
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
        let client = yekitrim_llm::LlmClient::new(llm_cfg)?
            .with_timeout(180)
            .with_usage_context(self.usage_context());
        process_window_inner(&client, &segments, &self.detect, &self.hot_windows, 0, self.pass)
            .await
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
    // 第一趟的老口径：重叠 <50% 的两条里，截断后不足 30s 的**丢掉**（大概率是重复噪声）。
    merge_window_events_min(events, 30.0)
}

/// 跨窗口合并（可配"截断后多短就丢"）。
///
/// ⚠️ **第二趟必须传 0**（`merge_window_events_min(events, 0.0)`）：分散片段的每一块本来就
/// 可能只有几十秒，用第一趟那条 30s 门槛会把它们**全丢掉** —— 那样"降低标准"等于白做
/// （症状：开关打开了、事件数一个没变）。去重仍然照做（重叠 ≥50% 留更长的）。
fn merge_window_events_min(events: Vec<Event>, min_keep_secs: f64) -> Vec<Event> {
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
                // 截断后太短则丢弃（大概率是重复噪声；门槛由调用方给）
                if ev_end - last_end < min_keep_secs {
                    continue;
                }
                ev.start = Timestamp::from_seconds(last_end);
            }
        }
        merged.push(ev);
    }

    merged
}

/// 拆窗下限：比它更小的窗口不值得再拆 —— 每半只剩几条字幕，切出来的事件没有意义。
const MIN_SPLIT_SEGMENTS: usize = 8;
/// 拆窗最大递归深度（30 → 17+17 → 11+11）：防止"拆了还是失败"时无限递归。
const MAX_SPLIT_DEPTH: u8 = 2;

/// 处理一个窗口：先完整尝试，失败且窗口还够大时拆半递归。`depth` 是拆窗层数（顶层 0）。
fn process_window_inner<'a>(
    client: &'a yekitrim_llm::LlmClient,
    segments: &'a [TranscriptSegment],
    detect: &'a DetectionConfig,
    hot_windows: &'a [(f64, f64)],
    depth: u8,
    pass: DetectPass,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Vec<Event>>> + Send + 'a>> {
    Box::pin(async move {
        let win_start = segments.first().map(|s| s.start).unwrap_or(0.0);
        let win_end = segments.last().map(|s| s.end).unwrap_or(0.0);

        let window_hot: Vec<(f64, f64)> = hot_windows
            .iter()
            .copied()
            .filter(|(s, e)| s < &win_end && e > &win_start)
            .collect();

        let prompt = build_prompt(segments, detect, &window_hot, pass);
        let messages = serde_json::json!([
            {"role": "system", "content": system_prompt_for(detect, pass)},
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
                    tracing::warn!(
                        "窗口 [{:.1}-{:.1}] 返回事件为空 (第 {} 次尝试)",
                        win_start,
                        win_end,
                        attempt
                    );
                    last_err = Some(anyhow::anyhow!("LLM 返回的事件列表为空"));
                }
                Err(e) => {
                    tracing::warn!(
                        "窗口 [{:.1}-{:.1}] 失败 (第 {} 次尝试): {}",
                        win_start,
                        win_end,
                        attempt,
                        e
                    );
                    let truncated = e.downcast_ref::<TruncatedOutput>().is_some();
                    last_err = Some(e);
                    // 参数一个没变，第二次必然还是同样的截断：别白烧一次调用，直接拆窗
                    if truncated {
                        tracing::warn!(
                            "窗口 [{:.1}-{:.1}] 输出被 max_tokens 截断，同样参数重试必然再截断 → 拆窗重跑",
                            win_start,
                            win_end
                        );
                        break;
                    }
                }
            }
        }

        // 拆半递归，避免整窗丢失。
        //
        // ⚠️ 这里原来的条件是 `segments.len() > 30`，而窗口大小就是 30（`DETECT_WINDOW_SIZE`）——
        // 这条兜底从来没触发过一次。实测被截断的窗口因此整段丢失，事件覆盖率掉到 46%。
        if depth < MAX_SPLIT_DEPTH && segments.len() >= MIN_SPLIT_SEGMENTS {
            let mid = segments.len() / 2;
            // 拆分点两侧各留一点重叠（与相邻窗口的重叠同理）：跨越拆分点的话题在两半里都出现，
            // 交给 merge_window_events 去重，而不是被硬切在中间。
            let pad = (DETECT_WINDOW_OVERLAP / 2).max(1).min(mid);
            let left = &segments[..(mid + pad).min(segments.len())];
            let right = &segments[mid - pad..];
            tracing::warn!(
                "窗口 [{:.1}-{:.1}] 失败，拆半为 [{:.1}-{:.1}] 和 [{:.1}-{:.1}] 重跑",
                win_start,
                win_end,
                left.first().map(|s| s.start).unwrap_or(0.0),
                left.last().map(|s| s.end).unwrap_or(0.0),
                right.first().map(|s| s.start).unwrap_or(0.0),
                right.last().map(|s| s.end).unwrap_or(0.0)
            );

            // 两半各自记成败：**一半成功也把结果留下**（整段丢弃正是覆盖率掉一半的直接原因）。
            // 顺序执行（不并发），避免绕过 Agent 的并发闸门把在飞请求数翻倍。
            let mut combined: Vec<Event> = Vec::new();
            let mut errs: Vec<String> = Vec::new();
            match process_window_inner(client, left, detect, hot_windows, depth + 1, pass).await {
                Ok(mut events) => combined.append(&mut events),
                Err(e) => errs.push(format!("{e:#}")),
            }
            match process_window_inner(client, right, detect, hot_windows, depth + 1, pass).await {
                Ok(mut events) => combined.append(&mut events),
                Err(e) => errs.push(format!("{e:#}")),
            }
            if !combined.is_empty() {
                return Ok(combined);
            }
            return Err(anyhow::anyhow!("拆窗后仍全部失败: {}", errs.join(" | ")));
        }

        Err(last_err.unwrap_or_else(|| anyhow::anyhow!("窗口处理失败")))
    })
}

async fn attempt_call(
    client: &yekitrim_llm::LlmClient,
    messages: serde_json::Value,
) -> Result<Vec<Event>> {
    let body = client.send(messages).await?;
    // 先看截断标记：解析失败时它决定「这条错误该不该让调用方原样重试」
    let truncated = truncation_of(&body);

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
        // 截断且两处都空 = 模型把整个输出预算烧在思考上（实测 reasoning 占满 4096，
        // content 一个字都没留下）：报「截断」，别报成"模型异常"
        if let Some(t) = truncated {
            return Err(anyhow::Error::new(t));
        }
        anyhow::bail!("LLM 返回 content 和 reasoning 均为空（可能是 max_tokens 过小或模型异常）");
    }

    let mut last_err: Option<anyhow::Error> = None;
    for source in &parse_sources {
        match parse_json(source) {
            Ok(v) => match parse_events(v) {
                Ok(events) => return Ok(events),
                Err(e) => {
                    // 截断的响应里字段不完整是常态：报「截断」比报"字段解析失败"更可操作
                    let err = anyhow::anyhow!("解析事件失败: {e:#}");
                    return Err(match truncated {
                        Some(t) => anyhow::Error::new(t),
                        None => err,
                    });
                }
            },
            Err(e) => {
                last_err = Some(e);
            }
        }
    }

    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    // 转储落到程序目录的 `.yekitrim/llm_debug/`：以前写的是**相对进程 CWD** 的路径，
    // server 的 CWD 是仓库根 —— 一轮重跑就在仓库里堆 70+ 个 yekitrim_llm_debug_*.log。
    //
    // ⚠️ 这个目录**不在任何租户囚笼里**，而内容是**用户的直播转录原文** + 模型推理。
    // 所以三条一起做：① 不再退回进程 CWD；② 有保留期（只留最新若干份）；③ 单份有体积上限。
    fn prune_debug_dir(dir: &std::path::Path, keep: usize) {
        let Ok(rd) = std::fs::read_dir(dir) else { return };
        let mut files: Vec<std::path::PathBuf> = rd
            .flatten()
            .map(|e| e.path())
            .filter(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.starts_with("yekitrim_llm_debug_") && n.ends_with(".log"))
            })
            .collect();
        if files.len() <= keep {
            return;
        }
        // 文件名里的 unix 秒是等宽十进制 → 字典序 == 时间序
        files.sort();
        let drop_n = files.len() - keep;
        for p in files.into_iter().take(drop_n) {
            let _ = std::fs::remove_file(&p);
        }
    }
    /// 单份转储的体积上限（超出截断并注明 —— 别让"看不全"被误读成"模型只输出了这些"）。
    fn cap_debug(full: &str) -> std::borrow::Cow<'_, str> {
        const MAX: usize = 256 * 1024;
        if full.len() <= MAX {
            return std::borrow::Cow::Borrowed(full);
        }
        let mut cut = MAX;
        while cut > 0 && !full.is_char_boundary(cut) {
            cut -= 1;
        }
        let mut out = String::with_capacity(cut + 96);
        out.push_str(&full[..cut]);
        out.push_str(&format!(
            "\n\n--- 已截断：完整转储 {} 字节，此处只保留前 {} 字节 ---\n",
            full.len(),
            cut
        ));
        std::borrow::Cow::Owned(out)
    }

    let dir = crate::path::yekitrim_home().join("llm_debug");
    let _ = std::fs::create_dir_all(&dir);
    prune_debug_dir(&dir, 50);
    let log_path = dir.join(format!("yekitrim_llm_debug_{ts}.log"));
    let full = format!(
        "--- 截断: {} ---\n--- content ---\n{}\n\n--- reasoning ---\n{}\n\n--- full response ---\n{}",
        truncated
            .as_ref()
            .map(|t| t.to_string())
            .unwrap_or_else(|| "否".to_string()),
        content,
        reasoning,
        serde_json::to_string_pretty(&body).unwrap_or_default()
    );
    let full = cap_debug(&full);
    if let Err(e) = std::fs::write(&log_path, full.as_bytes()) {
        // ⚠️ 不再退回进程 CWD：那等于往仓库/启动目录里丢文件，用户既找不到也清不掉。
        // 写不了就只留一行 error —— 现场丢了，也好过污染工作目录。
        tracing::error!("LLM 调试转储写入失败（{}）: {e}", log_path.display());
    }
    match truncated {
        Some(t) => {
            tracing::warn!("LLM 响应解析失败：{t}，已保存到 {}", log_path.display());
            Err(anyhow::Error::new(t))
        }
        None => {
            tracing::warn!(
                "LLM 响应解析失败: {}，已保存到 {}",
                last_err.as_ref().map(|e| e.to_string()).unwrap_or_default(),
                log_path.display()
            );
            Err(last_err.unwrap_or_else(|| anyhow::anyhow!("LLM 响应 JSON 解析失败")))
        }
    }
}

pub async fn test_llm(llm: &LlmConfig, prompt: &str) -> Result<String> {
    if llm.api_key.is_empty() {
        anyhow::bail!("LLM API key 未配置");
    }
    let messages = serde_json::json!([
        {"role": "system", "content": "你是一个 AI 助手。"},
        {"role": "user", "content": prompt}
    ]);
    let client = yekitrim_llm::LlmClient::new(&llm.for_link(crate::config::LlmLink::Utility))?
        .with_timeout(180)
        .with_usage_context(yekitrim_llm::usage::UsageContext::new("LLM 测试"));
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
        clip_dir: None,
        enabled: true,
        ranges: Vec::new(),
        topic: None,
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
        let no_hot = build_prompt(&segs, &cfg, &[], DetectPass::Main);
        assert!(!no_hot.contains("观众弹幕爆发时刻"));

        // 有热点：追加区间 + 要求
        let hot = build_prompt(&segs, &cfg, &[(12.0, 22.0), (45.5, 55.5)], DetectPass::Main);
        assert!(hot.contains("[12.0-22.0] 弹幕爆发"));
        assert!(hot.contains("[45.5-55.5] 弹幕爆发"));
        assert!(hot.contains("不得切在这些爆发区间内部"));
    }

    #[test]
    fn test_build_score_prompt_only_contains_label_summary() {
        let batch = vec![
            (1, "标题一".to_string(), "摘要一".to_string()),
            (2, "标题二".to_string(), "摘要二".to_string()),
        ];
        let prompt = build_score_prompt(&batch);
        assert!(prompt.contains("1. 标题：标题一"));
        assert!(prompt.contains("摘要：摘要一"));
        assert!(prompt.contains("2. 标题：标题二"));
        assert!(prompt.contains("摘要：摘要二"));
        assert!(prompt.contains("只依据每条给出的标题+摘要"));
        assert!(prompt.contains("预测点击率"));
        assert!(prompt.contains("有趣程度"));
        assert!(prompt.contains("爆点强度"));
    }

    #[test]
    fn test_parse_score_response_object_wrapper_and_string_score() {
        let batch = vec![
            (1, "标题一".to_string(), "摘要一".to_string()),
            (2, "标题二".to_string(), "摘要二".to_string()),
        ];
        let text = r#"{"scores":[{"index":1,"score":86},{"index":2,"score":"42%"}]}"#;
        let parsed = parse_score_response(text, &batch).unwrap();
        assert_eq!(parsed, vec![(1, 0.86), (2, 0.42)]);
    }

    #[test]
    fn test_parse_score_response_bare_array_and_position_fallback() {
        let batch = vec![
            (3, "标题三".to_string(), "摘要三".to_string()),
            (4, "标题四".to_string(), "摘要四".to_string()),
        ];
        let text = r#"[{"score":70},{"score":"100"}]"#;
        let parsed = parse_score_response(text, &batch).unwrap();
        assert_eq!(parsed, vec![(3, 0.70), (4, 1.0)]);
    }

    #[tokio::test]
    async fn test_score_events_skips_llm_when_weight_zero() {
        let events = vec![Event {
            start: Timestamp::from_seconds(0.0),
            end: Timestamp::from_seconds(10.0),
            label: "标题".into(),
            summary: "摘要".into(),
            event_type: EventType::Chat,
            source: EventSource::LLM,
            confidence: 1.0,
            popularity: 0.0,
            rank: 0,
            clip_dir: None,
            enabled: true,
            ranges: Vec::new(),
            topic: None,
        }];
        let llm = LlmConfig::default();
        let mut detect = DetectionConfig::default();
        detect.llm_score_weight = 0.0;
        let scores = score_events_with_llm(&events, &llm, &detect, None, None)
            .await
            .unwrap();
        assert_eq!(scores, vec![None]);
    }

    /// 探针一开始就为真 → 立刻报「任务已取消」，**且一次请求都不发**。
    ///
    /// 这条链路历史上只查 `llm_score_weight`：`weight <= 0` 直接返回全 `None`，
    /// 而探针检查刻意放在**建客户端之前**，所以这个用例不需要网络（也不该需要）。
    #[tokio::test]
    async fn score_events_bails_immediately_when_cancelled() {
        let events = vec![Event {
            start: Timestamp::from_seconds(0.0),
            end: Timestamp::from_seconds(10.0),
            label: "标题".into(),
            summary: "摘要".into(),
            event_type: EventType::Chat,
            source: EventSource::LLM,
            confidence: 1.0,
            popularity: 0.0,
            rank: 0,
            clip_dir: None,
            enabled: true,
            ranges: Vec::new(),
            topic: None,
        }];
        let mut llm = LlmConfig::default();
        // 给个非空 key + 非 0 权重，确保不会被"未配置/权重为 0"的早退挡住，
        // 真正走到探针那一步（它是比建客户端更早的那道门）。
        llm.api_key = "sk-test".into();
        let mut detect = DetectionConfig::default();
        detect.llm_score_weight = 0.3;
        let cancel: Option<yekitrim_llm::agent::CancelProbe> =
            Some(std::sync::Arc::new(|| true));
        let err = score_events_with_llm(&events, &llm, &detect, None, cancel)
            .await
            .expect_err("已取消时不应返回评分");
        assert!(format!("{err:#}").contains("任务已取消"), "{err:#}");
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
                clip_dir: None,
                enabled: true,
                ranges: Vec::new(),
                topic: None,
            }
        }

        // 两个重叠窗口各自输出同一大话题的长事件，边界差 60s。
        // 只是相邻不重叠，不应合并。
        let events = vec![ev(100.0, 700.0, "A"), ev(760.0, 1300.0, "B")];
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
                clip_dir: None,
                enabled: true,
                ranges: Vec::new(),
                topic: None,
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
                clip_dir: None,
                enabled: true,
                ranges: Vec::new(),
                topic: None,
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

    fn seg(t: f64, text: &str) -> TranscriptSegment {
        TranscriptSegment {
            start: t,
            end: t + 3.0,
            text: text.into(),
            confidence: 1.0,
        }
    }

    /// 前缀缓存的硬性前提：每窗口才变化的内容（字幕）必须是 prompt 的**最后一段**。
    /// provider 按最长公共前缀命中上下文缓存，变化部分一旦跑到中间，后面的内容就白扔了。
    #[test]
    fn prompt_puts_varying_segments_last() {
        let cfg = DetectionConfig::default();
        let segs = vec![seg(0.0, "第一句"), seg(10.0, "第二句")];
        let p = build_prompt(&segs, &cfg, &[(120.0, 135.0)], DetectPass::Main);

        let seg_pos = p.find("第一句").expect("字幕应在 prompt 里");
        let hot_pos = p.find("观众弹幕爆发时刻").expect("爆发块应在 prompt 里");
        assert!(hot_pos < seg_pos, "爆发块必须排在字幕之前");
        // 字幕之后只剩空行（没有别的固定内容被甩到后面）
        assert!(
            p[seg_pos..].trim_end().ends_with("第二句"),
            "字幕之后不该再有内容: {:?}",
            &p[seg_pos..]
        );
    }

    /// 老模板（用户 config.toml 里的副本）没有 {hot_windows} 占位符：
    /// 爆发块仍要插在字幕之前，不能丢、也不能跑到字幕后面。
    #[test]
    fn prompt_injects_hot_block_into_legacy_template() {
        let mut cfg = DetectionConfig::default();
        cfg.prompt_template = "规则若干。
转录片段：
{segments}"
            .to_string();
        let p = build_prompt(&[seg(0.0, "第一句")], &cfg, &[(120.0, 135.0)], DetectPass::Main);
        let hot_pos = p.find("观众弹幕爆发时刻").expect("爆发块不能丢");
        let seg_pos = p.find("第一句").expect("字幕应在 prompt 里");
        assert!(hot_pos < seg_pos, "老模板也要把爆发块放在字幕之前: {p}");
    }

    /// 没有弹幕数据时不注入空块
    #[test]
    fn prompt_omits_hot_block_when_empty() {
        let cfg = DetectionConfig::default();
        let p = build_prompt(&[seg(0.0, "第一句")], &cfg, &[], DetectPass::Main);
        assert!(!p.contains("观众弹幕爆发时刻"));
        assert!(!p.contains("{hot_windows}"), "占位符必须被替换掉");
    }

    // ---- max_tokens 截断：抢救完整元素 + 拆窗兜底 ----

    /// 被截断的数组：截断前已经完整的元素要能捡回来（整窗作废是覆盖率掉到 46% 的直接原因）。
    #[test]
    fn parse_json_salvages_complete_items_from_truncated_array() {
        let text = r#"[{"start":1.0,"end":2.0,"label":"一"},{"start":3.0,"end":4.0,"label":"二"},{"start":5.0,"end":6.0,"la"#;
        let arr = parse_json(text).unwrap();
        let arr = arr.as_array().unwrap();
        assert_eq!(arr.len(), 2, "只保留截断前完整的元素");
        assert_eq!(arr[1]["label"], "二");
    }

    /// 字符串里的括号不参与深度计数（时间轴文本/emoji 里出现 `}` 很常见）。
    #[test]
    fn salvage_ignores_braces_inside_strings() {
        let text = r#"[{"label":"a}b}c","start":1.0,"end":2.0},{"label":"tail"#;
        let arr = parse_json(text).unwrap();
        assert_eq!(arr.as_array().unwrap().len(), 1);
    }

    /// 已闭合的数组解析失败 = 真的坏了（比如键没引号），不该被"抢救"成半截结果。
    #[test]
    fn parse_json_does_not_salvage_closed_array() {
        assert!(parse_json(r#"[{start: 1, end: 2}]"#).is_err());
        // 一个完整元素都没有时也没有可抢救的东西
        assert!(parse_json(r#"[{"start":1.0,"la"#).is_err());
        // 空数组是合法结果，走正常解析
        assert_eq!(parse_json("[]").unwrap().as_array().unwrap().len(), 0);
    }

    #[test]
    fn truncation_of_reads_finish_reason() {
        let truncated = serde_json::json!({
            "choices": [{"finish_reason": "length"}],
            "usage": {"completion_tokens": 4096}
        });
        let t = truncation_of(&truncated).expect("finish_reason=length 就是截断");
        assert_eq!(t.completion_tokens, 4096);
        assert!(t.to_string().contains("max_tokens"), "提示里要给出调参方向");

        // Anthropic 风格的 stop_reason 也认
        let anthropic = serde_json::json!({"choices": [{"stop_reason": "max_tokens"}]});
        assert!(truncation_of(&anthropic).is_some());

        let normal = serde_json::json!({"choices": [{"finish_reason": "stop"}]});
        assert!(truncation_of(&normal).is_none());
    }

    /// 极简脚本化假 LLM：第 n 个请求回 `script[n]`，用完后一直回最后一条。
    /// 返回 (base_url, 请求计数, **每个请求的 body**) —— 后者用来断言链路覆盖真的落进了请求体。
    async fn fake_llm(
        script: Vec<String>,
    ) -> (
        String,
        Arc<std::sync::atomic::AtomicUsize>,
        Arc<std::sync::Mutex<Vec<String>>>,
    ) {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let hits = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&hits);
        let bodies = Arc::new(std::sync::Mutex::new(Vec::new()));
        let captured = Arc::clone(&bodies);
        tokio::spawn(async move {
            let mut n = 0usize;
            loop {
                let Ok((mut sock, _)) = listener.accept().await else {
                    break;
                };
                // 把请求读干净再回（请求体 >8KB，只读一次会留下半个请求）
                let mut raw: Vec<u8> = Vec::new();
                let mut buf = [0u8; 65536];
                loop {
                    match tokio::time::timeout(std::time::Duration::from_millis(50), sock.read(&mut buf)).await {
                        Ok(Ok(0)) | Ok(Err(_)) | Err(_) => break,
                        Ok(Ok(read)) => raw.extend_from_slice(&buf[..read]),
                    }
                }
                let text = String::from_utf8_lossy(&raw).to_string();
                if let Some((_, body)) = text.split_once("\r\n\r\n") {
                    captured.lock().unwrap().push(body.to_string());
                }
                counter.fetch_add(1, Ordering::SeqCst);
                let body = script
                    .get(n)
                    .or_else(|| script.last())
                    .cloned()
                    .unwrap_or_default();
                if n + 1 < script.len() {
                    n += 1;
                }
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.as_bytes().len()
                );
                let _ = sock.write_all(resp.as_bytes()).await;
                let _ = sock.shutdown().await;
            }
        });
        (format!("http://{addr}/v1"), hits, bodies)
    }

    fn truncated_body() -> String {
        serde_json::json!({
            "choices": [{
                "finish_reason": "length",
                "message": {"content": "", "reasoning": "让我先想想……[6221.0-6266.1] 这段是在聊"}
            }],
            "usage": {"completion_tokens": 4096}
        })
        .to_string()
    }

    /// 截断的窗口不原样重试，而是拆成两半各跑一次（共 3 次请求）；两半结果都留下。
    #[tokio::test]
    async fn truncated_window_is_split_instead_of_retried_verbatim() {
        use std::sync::atomic::Ordering;

        let ok = serde_json::json!({
            "choices": [{
                "finish_reason": "stop",
                "message": {"content": r#"[{"start":0.0,"end":30.0,"label":"左","summary":"摘要","event_type":"chat"}]"#}
            }]
        })
        .to_string();
        let (base, hits, _bodies) = fake_llm(vec![truncated_body(), ok.clone(), ok]).await;
        let client = yekitrim_llm::LlmClient::raw("k", &base, "m").with_max_tokens(4096);
        let segs: Vec<TranscriptSegment> = (0..12)
            .map(|i| seg(i as f64 * 10.0, &format!("第 {i} 句")))
            .collect();

        let events = process_window_inner(&client, &segs, &DetectionConfig::default(), &[], 0, DetectPass::Main)
            .await
            .unwrap();

        assert_eq!(
            hits.load(Ordering::SeqCst),
            3,
            "截断后应直接拆窗（1 次截断 + 2 次半窗），而不是原样重试"
        );
        assert_eq!(events.len(), 2, "两半的结果都要留下");
    }

    /// 窗口小到不值得拆时，截断错误必须带上可操作提示，而且不再原样重试。
    #[tokio::test]
    async fn truncated_small_window_fails_fast_with_max_tokens_hint() {
        use std::sync::atomic::Ordering;

        let (base, hits, _bodies) = fake_llm(vec![truncated_body()]).await;
        let client = yekitrim_llm::LlmClient::raw("k", &base, "m").with_max_tokens(4096);
        let segs: Vec<TranscriptSegment> = (0..4).map(|i| seg(i as f64 * 10.0, "短句")).collect();

        let err = process_window_inner(&client, &segs, &DetectionConfig::default(), &[], 0, DetectPass::Main)
            .await
            .unwrap_err();

        assert_eq!(hits.load(Ordering::SeqCst), 1, "截断后不该用同样参数重试");
        assert!(
            err.to_string().contains("max_tokens"),
            "错误信息要指出调大 max_tokens: {err}"
        );
    }

    /// 链路覆盖必须真的落到请求体里：检测窗口走 `[llm.overrides.detect].max_tokens`，
    /// 而不是 `[llm].max_tokens`（改了 `for_link` 但某个调用点忘了走它 → 这条会红）。
    #[tokio::test]
    async fn detect_window_request_carries_link_max_tokens() {
        let ok = serde_json::json!({
            "choices": [{
                "finish_reason": "stop",
                "message": {"content": r#"[{"start":0.0,"end":60.0,"label":"标题","summary":"摘要","event_type":"chat"}]"#}
            }]
        })
        .to_string();
        let (base, _hits, bodies) = fake_llm(vec![ok]).await;

        let mut llm = LlmConfig::default();
        llm.base_url = base;
        llm.api_key = "k".into();
        llm.model = "m".into();
        llm.max_tokens = 16384;
        llm.overrides.detect.max_tokens = Some(8192);

        let segs = vec![seg(0.0, "第一句"), seg(30.0, "第二句")];
        let events = call_with_progress(
            &segs,
            &llm,
            &DetectionConfig::default(),
            &[],
            None,
            None,
            None,
            None,
            DetectPass::Main,
        )
        .await
        .unwrap();
        assert!(!events.is_empty(), "假响应应能解析成事件");

        let sent = bodies.lock().unwrap().join("\n");
        assert!(
            sent.contains("\"max_tokens\":8192"),
            "请求体里应是链路覆盖的上限：{sent}"
        );
        assert!(
            !sent.contains("16384"),
            "不该再用 [llm].max_tokens：{sent}"
        );
    }
}
