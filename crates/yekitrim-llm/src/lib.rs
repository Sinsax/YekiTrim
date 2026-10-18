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
//! * [`params`] —— 一次调用的全部参数（归一化面）。
//! * [`client`] —— HTTP + 重试 + 请求体 + **用量采集的唯一一处**。
//! * [`agent`] —— 长输入切窗口并发跑 + checkpoint 续传 + 取消探针。
//! * [`usage`] —— 用量记录与 sink（落库由上层实现 trait）。

pub mod agent;
pub mod catalog;
pub mod client;
pub mod params;
pub mod wire;
pub mod usage;

pub use agent::{Agent, AgentItem, AgentProgress, AgentTask, AgentWindow, CancelProbe};
pub use client::LlmClient;
pub use params::{LlmParams, ModelCaps, ReasoningLevel, ThinkingWire};
