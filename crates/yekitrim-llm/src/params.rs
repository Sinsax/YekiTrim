//! 一次 LLM 调用的**全部参数**（归一化面）。
//!
//! # 为什么不是 `LlmConfig`
//!
//! `LlmConfig` 是**配置文件**的形状（TOML、四条链路的 overrides、版本迁移），它属于
//! `yekitrim-core::config`。本 crate 只认识「这一趟要发什么」——于是同一个客户端可以被
//! 测试、CLI、服务端以完全不同的方式来配置，而不需要认识任何配置文件。

use serde_json::Value;

/// 一次调用的参数。字段与**今天的** `LlmConfig` 一一对应（P1 只做搬迁，不改行为）。
#[derive(Debug, Clone)]
pub struct LlmParams {
    /// API key（明文；`api_key_env` 那一档是 P2 的事）。
    pub api_key: String,
    /// 基础地址，如 `https://api.openai.com/v1`。空 = 未配置（构造客户端时报错）。
    pub base_url: String,
    /// 模型 id。空 = 用兜底（`gpt-4o-mini`）。
    pub model: String,
    /// 采样温度。
    pub temperature: f64,
    /// 输出上限。
    pub max_tokens: u32,
    /// 原样透传给请求体的额外键（`response_format` 等）。**合并优先级最低**。
    pub extra_params: Option<Value>,
    /// 思考开关（`"enabled"` / `"disabled"`）。`None` = 不表态。
    pub thinking: Option<String>,
    /// 并发窗口数（`[llm] max_concurrent`，四条链路共用）。
    pub max_concurrent: usize,
}

impl Default for LlmParams {
    /// 与 `LlmConfig::default()` 同值：temperature 0.3、并发 3（否则搬迁就会改行为）。
    fn default() -> Self {
        Self {
            api_key: String::new(),
            base_url: String::new(),
            model: String::new(),
            temperature: 0.3,
            max_tokens: 8192,
            extra_params: None,
            thinking: None,
            max_concurrent: 3,
        }
    }
}

impl From<&LlmParams> for LlmParams {
    fn from(p: &LlmParams) -> Self {
        p.clone()
    }
}
