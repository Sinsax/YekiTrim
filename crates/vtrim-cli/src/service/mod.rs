//! Service Layer：CLI 与 WebUI 共用的领域逻辑。
//!
//! M1 先提供 Workspace / 路径解析，后续 core_pipeline / polish_pipeline / jobs
//! 等模块在此目录扩展。

mod core_pipeline;
mod correct;
mod silence;
mod status;
mod init_config;
mod workspace;
// 多用户服务 P1：账号账本 / 口令 KDF / 单实例锁 / `vtrim user` CLI
// 这三个模块同时被 `server`（HTTP 会话、启动预检）与 `user` CLI 使用，
// 因此按 `service → server` 单向依赖显式 pub，不再层层转发。
pub mod auth;
pub mod db;
pub mod disk;
pub mod doctor;
pub mod ingest;
pub mod instancelock;
pub mod projects;
pub mod user;

#[allow(unused_imports)]
pub use core_pipeline::*;
pub use correct::*;
pub use silence::*;
pub use status::*;
pub use init_config::*;
pub use workspace::*;
pub use doctor::*;
pub use user::*;