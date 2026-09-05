//! AppState：HTTP handler 共享的当前工程状态与任务管理。

use std::path::PathBuf;
use std::sync::Arc;

use tokio::sync::RwLock;

use crate::service::Workspace;

use super::jobs::JobManager;

#[derive(Debug, Clone)]
pub struct AppState {
    pub workspace: Arc<RwLock<Workspace>>,
    pub workspace_root: Arc<RwLock<PathBuf>>,
    pub jobs: Arc<JobManager>,

    /// wgpu 单帧真渲染预览的常驻后端槽（feature 门控）。
    #[cfg(feature = "wgpu-render")]
    pub preview: PreviewSlot,
}

#[cfg(feature = "wgpu-render")]
#[derive(Clone, Default)]
pub struct PreviewSlot(pub Arc<std::sync::Mutex<Option<PreviewBackend>>>);

#[cfg(feature = "wgpu-render")]
impl std::fmt::Debug for PreviewSlot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PreviewSlot")
    }
}

#[cfg(feature = "wgpu-render")]
pub struct PreviewBackend {
    pub dir: PathBuf,
    pub backend: vtrim_polish::render::native::WgpuBackend,
    pub last_t: f64,
    pub last_jpeg: Option<Vec<u8>>,
    /// ffprobe 结果缓存（同 dir 的视频尺寸不变）
    pub video_size: Option<(u32, u32)>,
}

impl AppState {
    #[allow(dead_code)]
    pub fn new(workspace: Workspace) -> anyhow::Result<Self> {
        // 默认工作区 = 当前工程根目录的父目录，方便扫描多个子工程。
        let default_root = workspace
            .root_dir
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| workspace.root_dir.clone());
        Self::with_root(workspace, default_root)
    }

    pub fn with_root(workspace: Workspace, workspace_root: PathBuf) -> anyhow::Result<Self> {
        Ok(Self {
            workspace: Arc::new(RwLock::new(workspace)),
            workspace_root: Arc::new(RwLock::new(workspace_root)),
            jobs: Arc::new(JobManager::new()),
            #[cfg(feature = "wgpu-render")]
            preview: PreviewSlot::default(),
        })
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

    pub async fn set_workspace_root(&self, root: PathBuf) {
        *self.workspace_root.write().await = root;
    }
}
