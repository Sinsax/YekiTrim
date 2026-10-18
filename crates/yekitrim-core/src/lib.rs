//! yekitrim-core：切片领域内核（管线编排 + ASR / 事件检测 / 剪切 + 配置 / LLM 客户端 / 弹幕）。
//!
//! 入口：`types::PipelineContext`、`pipeline::{Stage, PipelineRunner}`、四个 Stage（`audio::AudioExtractor` / `asr::Transcriber` / `detection::EventDetector` / `cutting::Cutter`）、`config::{load_config, load_config_at}`。
//! 边界：core 不许认识上层 —— 源码里不得出现 `yekitrim_polish` / `yekitrim_service` / `yekitrim_server` / `yekitrim_cli`，需要上层能力时由调用方注入；本文件是全表，新增模块必须在这里登记 —— 见 `crates/yekitrim-core/AGENTS.md`

pub mod types;
pub mod config;
pub mod fonts;
pub mod path;
pub mod srt;
pub mod pipeline;
pub mod audio;
pub mod asr;
pub mod download;
pub mod detection;
pub mod danmaku;
pub mod cutting;
pub mod progress;
pub mod process;
pub mod filelock;
pub mod tenant;
pub mod owner;
