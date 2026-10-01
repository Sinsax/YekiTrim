//! AppState：HTTP handler 共享的当前工程状态与任务管理。

use std::path::PathBuf;
use std::sync::Arc;

use tokio::sync::RwLock;

use vtrim_service::Workspace;

use super::auth::AuthService;
use super::jobs::JobManager;

#[derive(Debug, Clone)]
pub struct AppState {
    pub workspace: Arc<RwLock<Workspace>>,
    pub workspace_root: Arc<RwLock<PathBuf>>,
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
    pub fn with_auth(
        workspace: Workspace,
        workspace_root: PathBuf,
        auth: Arc<AuthService>,
    ) -> Self {
        Self {
            workspace: Arc::new(RwLock::new(workspace)),
            workspace_root: Arc::new(RwLock::new(workspace_root)),
            jobs: Arc::new(JobManager::new()),
            auth,
            preview_servers: Arc::new(std::sync::Mutex::new(std::collections::HashMap::new())),
        }
    }

    pub async fn workspace(&self) -> Workspace {
        self.workspace.read().await.clone()
    }

    pub async fn set_workspace(&self, ws: Workspace) {
        *self.workspace.write().await = ws;
    }

    pub async fn workspace_root(&self) -> PathBuf {
        self.workspace_root.read().await.clone()
    }
}
