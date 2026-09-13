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
// 多用户服务 P1：账号账本 / 口令 KDF / `vtrim user` CLI
mod auth;
mod db;
mod user;

#[allow(unused_imports)]
pub use core_pipeline::*;
pub use correct::*;
pub use silence::*;
pub use status::*;
pub use init_config::*;
pub use workspace::*;
// auth / db 目前只被 user.rs 使用（HTTP 侧接入时再决定是否对外 re-export）
pub use user::*;
