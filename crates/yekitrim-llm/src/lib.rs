//! YekiTrim 的 **LLM 传输层**（隔离 crate）。
//!
//! # 它为什么独立成一个 crate
//!
//! 这里放的是「**别家的协议知识**」：请求体怎么拼、推理参数用哪种拼写、哪些模型不支持温度、
//! token 用量怎么读、重试与超时怎么定。这些知识一旦散到业务 crate 里，就会出现
//! 「同一个旋钮在两处各写一半」和「换个模型要改五个文件」。
//!
//! # 边界（机器判据：`scripts/check_module_boundaries.py` 的 `llm_is_a_leaf` / `only_llm_knows_wire`）
//!
//! * 本 crate **不认识**任何 `yekitrim-*` crate、**不读** TOML、**不认识** `llm.toml` / `polish.toml`；
//!   它只吃 [`params::LlmParams`]。配置 → 参数的换算在 `yekitrim-core::config`（`impl From<&LlmConfig>`）。
//! * **只有本 crate 可以写 LLM 的 wire 字面量**（`reasoning_effort` / `thinking` / `enable_thinking`
//!   / `max_completion_tokens` / `response_format` …）。业务侧只表达「要什么」，不表达「怎么发」。
//!
//! # 模块
//!
//! * [`protocol`] —— **协议注册表**：请求路径 + 鉴权头（"请求怎么发"，一家标准一条）。
//! * [`dialect`] —— **方言注册表**：推理参数怎么写 + 输出上限发哪个字段名（一家标准一行）。
//! * [`params`] —— 一次调用的全部参数（归一化面）。
//! * [`client`] —— HTTP + 重试 + 请求体 + **用量采集的唯一一处**。
//! * [`discovery`] —— `GET {base}/models`：问中转"有哪些模型"（失败分类，四条文案互不串味）。
//! * [`agent`] —— 长输入切窗口并发跑 + checkpoint 续传 + 取消探针。
//! * [`usage`] —— 用量记录与 sink（落库由上层实现 trait）。

pub mod agent;
pub mod catalog;
/// **能力读取器注册表**：`/v1/models` 里各家的自描述字段 → 能力片段（加标准只加表项）。
pub mod caps;
pub mod client;
/// **方言注册表**：一家 API 标准 = 一条表项（加标准只加表项，不碰 wire/params/catalog）。
pub mod dialect;
pub mod discovery;
pub mod params;
/// **协议注册表**：一份 provider 的「请求怎么发」（路径 + 鉴权头）。与 `dialect`（参数怎么拼）分工不同。
pub mod protocol;
pub mod wire;
pub mod usage;

pub use agent::{Agent, AgentItem, AgentProgress, AgentTask, AgentWindow, CancelProbe};
pub use client::LlmClient;
pub use params::{InputModalities, LlmParams, ModelCaps, ReasoningLevel};
