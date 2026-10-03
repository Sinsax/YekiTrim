//! Service Layer：CLI 与 WebUI 共用的领域逻辑。
//!
//! 独立 crate（原 `yekitrim-cli/src/service/`）。分层是**单向**的：
//!
//! ```text
//! yekitrim-core ─┐
//! yekitrim-polish ┼─→ yekitrim-service ──→ yekitrim-server ──→ yekitrim-cli（只剩入口）
//!             └──────────────────────────────────────┘
//! ```
//!
//! **不变式：本 crate 绝不能 `use yekitrim_server::*`。** 这是它能被抽出来的全部理由 ——
//! 一旦出现反向边，两层就又粘回去，`yekitrim-server` 也没法再独立。
//! 搬进来之前实测过这条边是 0（`server → service` 是重依赖，反方向没有），
//! 唯一的渗漏点 `logbus` 已经下沉到本 crate（见 `logbus` 模块头的说明）。
//!
//! 本 crate **不依赖 axum**（`recorder.rs` 的测试除外，那是 dev-dependency）：
//! 它是纯应用层，HTTP 与否由 `yekitrim-server` 决定。

mod core_pipeline;
mod correct;
mod silence;
mod status;
mod init_config;
mod workspace;
// 多用户服务 P1：账号账本 / 口令 KDF / 单实例锁 / `yekitrim user` CLI
// 这几个模块同时被 `yekitrim-server`（HTTP 会话、启动预检）与 `user` CLI 使用，
// 因此显式 pub，不再层层转发。
pub mod assets;
pub mod auth;
// P4：B站链接通道。`bilibili` 只认 bvid/cid/弹幕，`downloader` 只认 URL + 通用下载选项，
// 两者**只在 server 侧装配**（`download_worker`）——
// 加第二个下载引擎 = 加一个 `downloader` 适配器，server 一行不改。
pub mod bilibili;
pub mod db;
pub mod disk;
pub mod doctor;
pub mod downloader;
// `ingest` 是采集/入库领域（录播姬 CLI + 共享的容量预检/项目落地）
pub mod ingest;
pub mod instancelock;
pub mod invite;
pub mod logbus;
// `net`：HTTP 层小工具（解压 / 浏览器 UA），不含任何平台知识
pub mod net;
pub mod projects;
pub mod recorder;
pub mod usage;
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