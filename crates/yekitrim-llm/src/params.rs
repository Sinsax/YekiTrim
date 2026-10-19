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
//! 它的 `compat.thinkingFormat` / `maxTokensField` 就是这里的**方言**（[`crate::dialect::Dialect`]）：
//! 写成"注册表里的一行"，所以加一家新标准不用改本文件。

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
    /// 最强档（**本网关在用**：`["low","high","max"]`）。
    ///
    /// ⚠️ 它只是**词**：发不发仍由能力面（`levels`）决定 —— 表里没有的档一个都不发。
    Max,
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
            Self::Max => "max",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s.trim().to_ascii_lowercase().as_str() {
            "off" | "none" | "disabled" => Self::Off,
            "minimal" => Self::Minimal,
            "low" => Self::Low,
            "medium" | "mid" => Self::Medium,
            "high" => Self::High,
            "xhigh" => Self::XHigh,
            // ⚠️ max 曾经被折成 xhigh（pi-ai 数据里没有 max）；本网关在用，**分开**
            "max" => Self::Max,
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

/// **输入模态**（能力面）：这个模型吃不吃图片。
///
/// ⚠️ **保守默认 = 只有文字**：不认识的老网关不许被我们"猜"成支持图片
/// （猜错的症状是把图片塞进不支持它的接口 → 400 / 静默丢掉那一张）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InputModalities {
    pub text: bool,
    pub image: bool,
}

impl Default for InputModalities {
    fn default() -> Self {
        Self { text: true, image: false }
    }
}

impl InputModalities {
    /// 从一串模态名（`["text","image"]`）解析；**未知串忽略**、`text` 缺省为真。
    ///
    /// 为什么 `text` 缺省为真：每一家 chat/completions 都能吃文字，"没写"不等于"不支持文字"。
    pub fn parse<S: AsRef<str>>(items: impl IntoIterator<Item = S>) -> Self {
        let mut out = Self { text: false, image: false };
        for it in items {
            match it.as_ref().trim().to_ascii_lowercase().as_str() {
                "text" => out.text = true,
                "image" | "vision" | "img" => out.image = true,
                _ => {}
            }
        }
        if !out.text && !out.image {
            // 一个都认不出（空数组 / 全是未知串）⇒ 回到"只有文字"（保守，同上理由）
            return Self::default();
        }
        if !out.text {
            // 只说了 image 没提 text：文字仍可用，别把"没提"当"没有"
            out.text = true;
        }
        out
    }

    /// 给界面看的形状（与 `caps_json` 的 `input` 同一个键名）。
    pub fn to_json(self) -> serde_json::Value {
        serde_json::json!({ "text": self.text, "image": self.image })
    }

    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }
}

/// 一个模型的能力（由内置表 + 用户覆盖解析出来）。
#[derive(Debug, Clone)]
pub struct ModelCaps {
    /// 档位 → wire 拼写；`None` = 该档支持但**不发送**；**缺档 = 不支持**。
    pub levels: BTreeMap<ReasoningLevel, Option<String>>,
    /// **方言**（推理参数怎么写 + 输出上限发哪个字段名）。
    /// 未知一律指向 [`crate::dialect::none()`]（什么都不发）。
    pub dialect: &'static crate::dialect::Dialect,
    pub temperature: Support,
    pub context_window: Option<u32>,
    /// 该模型输出上限的**能力**（不是每请求默认值）。
    pub max_output_ceiling: Option<u32>,
    /// **输入模态**（文字/图片）。默认只有文字。
    pub input: InputModalities,
    /// **能不能关掉推理**（服务端自述 `can_disable_thinking`）。
    ///
    /// `None` = 未声明 —— **别默认 false**：那等于把"没说"说成"关不掉"，
    /// 界面就会把"关闭"标成不可用（用户明明能关）。
    pub can_disable: Option<bool>,
    /// **只会推理**（`only_reasoning`）：没有"不思考"这一档。默认 false。
    pub only_reasoning: bool,
    /// 服务端给的**默认档**（`reasoning_default_effort`）；`None` = 未声明。
    pub default_level: Option<ReasoningLevel>,
    /// 支不支持工具调用（服务端自述）。**只用于展示**：我们这一版不发工具。
    pub tool_call: Option<bool>,
}

impl Default for ModelCaps {
    /// **保守默认**：表没命中时用它 —— 不发推理参数、温度照发、`max_tokens`、只吃文字。
    fn default() -> Self {
        Self {
            levels: BTreeMap::new(),
            dialect: crate::dialect::none(),
            temperature: Support::Supported,
            context_window: None,
            max_output_ceiling: None,
            input: InputModalities::default(),
            can_disable: None,
            only_reasoning: false,
            default_level: None,
            tool_call: None,
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
    /// 输出上限（字段名由 `caps.dialect.max_output_field` 决定）。`None` = **不发送**。
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
