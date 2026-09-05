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

#[allow(unused_imports)]
pub use core_pipeline::*;
pub use correct::*;
pub use silence::*;
pub use status::*;
pub use init_config::*;
pub use workspace::*;
