//! AppState：HTTP handler 共享的进程级状态与任务管理。
//!
//! ⚠️ **这里曾经存着一个"全局当前工程"（`workspace` / `workspace_root`），2026-10-03 删掉了。**
//! 它是**单工程时代**的遗留：那时一个 `yekitrim.toml` 就是一切。P2 起归属由 SQLite 的
//! `projects` 表决定，每个请求由 `scope.rs` 的 `Scope` 提取器**按 `?project=<id>` 现解析**
//! 出自己那份 `Workspace` —— 全局那份于是成了一条**没人查的旁路**：
//! 它只会跟着"上次打开的工程"漂，谁读谁就拿到别人的工程。
//!
//! 删它的判据（当时的普查）：全仓只剩两个消费者 —— 过渡期老契约 `/api/media/<绝对路径>`
//! 与 `/api/project/list` 回显的那个 `workspace_root`（它自己都标着 deprecated）。
//! **别因为"少个字段就少个能读工程的地方"而把它加回来** —— 要读工程就经 `Scope`。

use std::sync::Arc;

use super::auth::AuthService;
use super::jobs::JobManager;

/// 本地免登录模式下的固定身份（只有 loopback + 显式开关才会是 `Some`）。
///
/// 见 `ServerArgs::local_no_auth`：开的时候不校验会话，所有请求都当作本机管理员。
#[derive(Debug, Clone)]
pub struct LocalAdmin {
    pub id: i64,
    pub username: String,
    pub role: String,
}

#[derive(Debug, Clone)]
pub struct AppState {
    pub jobs: Arc<JobManager>,

    /// 会话鉴权（账号账本 + 限流）。身份数据不可缺省：打不开库就没有 AppState。
    pub auth: Arc<AuthService>,


    /// `Some` = 本地免登录（**只可能是 loopback + 显式开关**）。由 `auth::authenticate` 读。
    pub local_admin: Option<LocalAdmin>,
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
            local_admin: None,
        }
    }

    /// 打开本地免登录（`yekitrim-tray` 用）。⚠️ 调用方必须已经确认**监听的是 loopback** ——
    /// 这条不变式由 `run_with` 的开机守卫保证，别在别处随手打开。
    pub fn with_local_admin(mut self, la: LocalAdmin) -> Self {
        self.local_admin = Some(la);
        self
    }
}
