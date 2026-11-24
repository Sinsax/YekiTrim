//! `api::project_data`：项目数据面的**共享底座**（作用域 + 跨域 helper + 给路由表的显式再导出）。
//!
//! 这个目录原来是一个 3,570 行的 `project_data.rs`（31 个 handler）。按语义分文件：
//!
//! | 文件 | 语义 |
//! |---|---|
//! | `status` | 状态 / 事件·片段列表入口 / 弹幕与搜索文本 / 程序与工程配置 |
//! | `events` | events.json 的增删改、逐条与一键剪切、管线入口 |
//! | `clips` | clips/ 目录：列表、缩略图、发布标题、删除、屏蔽轨道 |
//! | `assets` | 素材清单 `[project.materials]` 与转写稿读写 |
//! | `fs` | `POST /api/fs/list`（服务器文件浏览，仅 admin）|
//! | `media` | `/api/media/*path` 文件出口 |
//! | `mod` | 共享底座（跨文件 helper）+ 对外再导出 |
//!
//! ⚠️ **模块之间只许显式 `use`，不许 glob**（整改 P4 T18）：拆分前本文件与
//! `polish.rs` 互相 glob 引用形成模块环。阶段 A 已断环（`validated_project_dir` 下沉到
//! `api/mod.rs`），本阶段按语义切文件。**唯一允许的跨域边**是
//! `use super::polish::polish_projects_impl;`（本文件 → polish，单向）。

pub(super) mod status;
pub(super) mod events;
pub(super) mod clips;
pub(super) mod assets;
pub(super) mod fs;
pub(super) mod media;

// ---- 对外出口（`api/routes.rs` 的路由表）：**逐条点名**，不许 `pub use x::*` ----
pub(crate) use assets::{project_data_assets, project_data_assets_put, project_data_transcript, project_data_transcript_put};
pub(crate) use clips::{
    project_data_clip_delete, project_data_clip_silence, project_data_clip_thumb,
    project_data_clip_title_put, project_data_clips,
};
pub(crate) use events::{
    project_data_correct, project_data_event_cut, project_data_event_cut_all, project_data_events_create,
    project_data_events_merge_splits, project_data_events_put, project_data_pipeline,
};
pub(crate) use fs::fs_list;
pub(crate) use media::media_get;
pub(crate) use status::{
    project_data_config, project_data_config_put, project_data_danmaku, project_data_events,
    project_data_polish, project_data_search_text, project_data_status,
};

use super::*;
