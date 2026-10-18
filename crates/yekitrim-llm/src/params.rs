//! 一次 LLM 调用的**全部参数**（归一化面）+ 模型能力。
//!
//! # 为什么不是 `LlmConfig`
//!
//! `LlmConfig` 是**配置文件**的形状（TOML、四条链路的 overrides、版本迁移），它属于
//! `yekitrim-core::config`。本 crate 只认识「这一趟要发什么」。
//!
//! # 抄的是 `@earendil-works/pi-ai` 的契约（MIT）
//!
//! 它的 `thinkingLevelMap: { level: string | null }` 就是这里的 [`ModelCaps::levels`]：
//! **档位 → wire 拼写**；`null` = 支持但**不发送**；**缺档 = 该模型不提供这一档**。
//! 它的 `compat.thinkingFormat` / `maxTokensField` 就是 [`ThinkingWire`] / [`MaxOutputWire`]。

use serde_json::Value;
use std::collections::BTreeMap;

/// 推理档位（内部统一命名；**wire 上怎么拼由 [`ModelCaps::levels`] 决定**）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub enum ReasoningLevel {
    /// 不推理（多数 provider 的"不思考"就是**不发这个参数**）。
    #[default]
    Off,
    Minimal,
    Low,
    Medium,
    High,
    XHigh,
}

impl ReasoningLevel {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Minimal => "minimal",
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::XHigh => "xhigh",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s.trim().to_ascii_lowercase().as_str() {
            "off" | "none" | "disabled" => Self::Off,
            "minimal" => Self::Minimal,
            "low" => Self::Low,
            "medium" | "mid" => Self::Medium,
            "high" => Self::High,
            "xhigh" | "max" => Self::XHigh,
            _ => return None,
        })
    }

    /// 旧口径（`[llm] thinking = "enabled"/"disabled"`）→ 档位。这是**迁移期唯一的映射点**。
    pub fn from_thinking_flag(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "enabled" | "on" | "true" | "yes" => Some(Self::High),
            "disabled" | "off" | "false" | "no" => Some(Self::Off),
            other => Self::parse(other),
        }
    }
}

/// 温度这个旋钮能不能发。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Support {
    #[default]
    Supported,
    Unsupported,
}

/// 推理参数**怎么发**（方言）。
///
/// ⚠️ 未知方言一律降级成 [`ThinkingWire::None`] —— **宁可不发，也不瞎发**：
/// 发错的参数要么 400，要么被静默忽略而用户以为生效了。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ThinkingWire {
    /// `reasoning_effort: "high"`（OpenAI o 系 / gpt-5）
    OpenAIEffort,
    /// `thinking: { "type": "enabled" | "disabled" }`（DeepSeek / Anthropic 兼容层）
    ThinkingType,
    /// `enable_thinking: bool`（Qwen / DashScope）
    QwenEnableThinking,
    /// `reasoning: { "effort": "high" }`（OpenRouter）
    OpenRouterEffort,
    /// 不发推理参数。
    #[default]
    None,
}

impl ThinkingWire {
    pub fn parse(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "openai_effort" | "reasoning_effort" => Self::OpenAIEffort,
            "thinking_type" | "thinking" => Self::ThinkingType,
            "qwen_enable_thinking" | "enable_thinking" => Self::QwenEnableThinking,
            "openrouter_effort" => Self::OpenRouterEffort,
            _ => Self::None,
        }
    }
}

/// `max_output` 发到哪个字段。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MaxOutputWire {
    #[default]
    MaxTokens,
    MaxCompletionTokens,
}

impl MaxOutputWire {
    pub fn parse(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "max_completion_tokens" => Self::MaxCompletionTokens,
            _ => Self::MaxTokens,
        }
    }
}

/// 一个模型的能力（由内置表 + 用户覆盖解析出来）。
#[derive(Debug, Clone)]
pub struct ModelCaps {
    /// 档位 → wire 拼写；`None` = 该档支持但**不发送**；**缺档 = 不支持**。
    pub levels: BTreeMap<ReasoningLevel, Option<String>>,
    pub thinking_wire: ThinkingWire,
    pub max_output_wire: MaxOutputWire,
    pub temperature: Support,
    pub context_window: Option<u32>,
    /// 该模型输出上限的**能力**（不是每请求默认值）。
    pub max_output_ceiling: Option<u32>,
}

impl Default for ModelCaps {
    /// **保守默认**：表没命中时用它 —— 不发推理参数、温度照发、`max_tokens`。
    fn default() -> Self {
        Self {
            levels: BTreeMap::new(),
            thinking_wire: ThinkingWire::None,
            max_output_wire: MaxOutputWire::MaxTokens,
            temperature: Support::Supported,
            context_window: None,
            max_output_ceiling: None,
        }
    }
}

impl ModelCaps {
    /// 这个模型是否提供任何推理档位。
    pub fn reasoning_supported(&self) -> bool {
        !self.levels.is_empty()
    }

    /// 该档的 wire 拼写；`None` = 不支持这一档。
    pub fn wire_for(&self, level: ReasoningLevel) -> Option<&Option<String>> {
        self.levels.get(&level)
    }
}

/// 一次调用的参数。
#[derive(Debug, Clone)]
pub struct LlmParams {
    pub api_key: String,
    /// 基础地址，如 `https://api.openai.com/v1`。空 = 未配置（构造客户端时报错）。
    pub base_url: String,
    /// 模型 id。空 = 用兜底（`gpt-4o-mini`）。
    pub model: String,
    /// 采样温度（`caps.temperature == Unsupported` 时**不发送**）。
    pub temperature: f64,
    /// 输出上限（字段名由 `caps.max_output_wire` 决定）。`None` = **不发送**。
    pub max_tokens: Option<u32>,
    /// 推理档位。
    pub reasoning: ReasoningLevel,
    /// **LLM 层是否表过态**（旧 `[llm] thinking` 的语义）。
    ///
    /// 链路自己的老开关（如 `[correct] thinking`）只在它为 `false` 时才生效 ——
    /// 否则会把按链路解析出来的档位顶掉（2026-09 修过的一个真 bug）。
    pub reasoning_explicit: bool,
    pub top_p: Option<f64>,
    /// 要求 JSON 输出（写 `response_format: {"type":"json_object"}`）。
    pub json_mode: bool,
    /// 原样透传给请求体的额外键。**覆盖上面这些**（老逃逸口，2026-10 之前就是最后盖）。
    pub extra_params: Option<Value>,
    /// 并发窗口数（`[llm] max_concurrent`，四条链路共用）。
    pub max_concurrent: usize,
    /// 这个模型的能力（决定"发不发/怎么发"）。
    pub caps: ModelCaps,
}

impl Default for LlmParams {
    /// 与 `LlmConfig::default()` 同值：temperature 0.3、并发 3（否则搬迁就会改行为）。
    fn default() -> Self {
        Self {
            api_key: String::new(),
            base_url: String::new(),
            model: String::new(),
            temperature: 0.3,
            max_tokens: None,
            reasoning: ReasoningLevel::Off,
            reasoning_explicit: false,
            top_p: None,
            json_mode: false,
            extra_params: None,
            max_concurrent: 3,
            caps: ModelCaps::default(),
        }
    }
}

impl From<&LlmParams> for LlmParams {
    fn from(p: &LlmParams) -> Self {
        p.clone()
    }
}
