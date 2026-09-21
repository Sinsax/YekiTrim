//! vtrim-polish：GSAP 精修引擎（从 `events.json` 出发生成字幕/贴纸/音效/特效 → 出片与封面）。
//!
//! 入口：子模块 `commands` / `cover` / `process` / `project` / `parse` / `llm` / `render` / `assets`。
//! 边界：只依赖 `vtrim-core` + `vtrim-render-core`，不许认识 `vtrim_service` / `vtrim_server` / `vtrim_cli`；工程状态只在 `polish.toml` + `data/` 里，不碰数据库 —— 见 `crates/vtrim-polish/AGENTS.md`

pub mod commands;
pub mod cover;
pub mod process;
pub mod project;
pub mod parse;
pub mod llm;
pub mod render;
pub mod assets;
