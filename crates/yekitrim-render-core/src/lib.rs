//! V-Trim 渲染核心：事件/时间线**类型**与时间线数学。
//!
//! 被 `vtrim-polish` 用（`Event` / `Segment` / `CameraStyle` 是 `polish.toml` 的字段类型）。
//! 不依赖 vtrim-core / vtrim-polish / tokio。
//!
//! ⚠️ **这里曾经有一套 wgpu 合成管线（`draw` / `effects` / `composite` / `sprite` / `gpu` /
//! `render` / `shaders` / `gif`，由 `wgpu` feature 门控），与一个浏览器真渲染预览（`vtrim-wasm`）——
//! 整条已删除**（用户拍板：GPU 出片与本仓无关，预览/出片走 `templates/index.html` 的 canvas 路径，
//! wgpu 那侧交给独立的 Dhampir 底座）。**别再往这里加回任何 GPU 管线**：
//! 需要 GPU 的东西进底座，本 crate 只留类型与数学。

pub mod easing;
pub mod legacy;
pub mod types;
pub mod timeline;
