//! AppState：HTTP handler 共享的进程级状态与任务管理。
//!
//! ⚠️ **这里曾经存着一个"全局当前工程"（`workspace` / `workspace_root`），2026-10-03 删掉了。**
//! 它是**单工程时代**的遗留：那时一个 `yekitrim.toml` 就是一切。P2 起归属由 SQLite 的
//! `projects` 表决定，每个请求由 `scope.rs` 的 `Scope` 提取器**按 `?project=<id>` 现解析**
//! 出自己那份 `Workspace` —— 全局那份于是成了一条**没人查的旁路**：
//! 它只会跟着"上次打开的工程"漂，谁读谁就拿到别人的工程。
//!
//! 删它的判据（当时的普查）：全仓只剩两个消费者 —— 过渡期老契约 `/api/media/<绝对路径>`
//! 与 `/api/project/list` 回显的那个 `workspace_root`（它自己都标了 deprecated）。
//! **别因为"少个字段就少个能读工程的地方"而把它加回来** —— 要读工程就经 `Scope`。

use std::sync::Arc;

use super::auth::AuthService;
use super::jobs::JobManager;

#[derive(Debug, Clone)]
pub struct AppState {
    pub jobs: Arc<JobManager>,

    /// 会话鉴权（账号账本 + 限流）。身份数据不可缺省：打不开库就没有 AppState。
    pub auth: Arc<AuthService>,

    /// 已启动的预览服务端口，key = `"{dir}|{kind}"`（kind: video / cover）。
    ///
    /// `RenderServer` 起的监听**无法关闭**，旧实现每次点击预览都新起一个，
    /// 端口与任务会无限泄漏；这里按目录复用（HTML 每次请求重建，不会过期）。
    pub preview_servers: Arc<std::sync::Mutex<std::collections::HashMap<String, u16>>>,
}

impl AppState {
    /// 用已打开的服务（含账号账本）构造状态。
    ///
    /// 库/目录不可写一律在 [`AuthService::open`] 处升级为启动失败（§3.5.7：
    /// 配置可缺省，身份不可缺省），因此这里不再有"内建默认值继续跑"的分支。
    pub fn with_auth(auth: Arc<AuthService>) -> Self {
        Self {
            jobs: Arc::new(JobManager::new()),
            auth,
            preview_servers: Arc::new(std::sync::Mutex::new(std::collections::HashMap::new())),
        }
    }
}
