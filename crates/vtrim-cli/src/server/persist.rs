//! Server 状态持久化：记住上次打开的工程与工作区根。
//!
//! 文件位置：与全局配置同目录 `.vtrim/server.toml`（跟随可执行文件旁的 `.vtrim/`）。

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ServerState {
    /// 上次 WebUI / server 使用的 vtrim.toml 绝对路径
    pub last_config_file: Option<String>,
    /// 工作区扫描根目录
    pub workspace_root: Option<String>,
}

pub fn state_path() -> PathBuf {
    vtrim_core::path::vtrim_home().join("server.toml")
}

pub fn load_state() -> Option<ServerState> {
    let path = state_path();
    let content = std::fs::read_to_string(&path).ok()?;
    toml::from_str(&content).ok()
}

pub fn save_state(state: &ServerState) {
    let path = state_path();
    if let Some(parent) = path.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            tracing::warn!("无法创建状态目录 {}: {e}", parent.display());
            return;
        }
    }
    let body = toml::to_string_pretty(state).unwrap_or_default();
    if let Err(e) = std::fs::write(&path, body) {
        tracing::warn!("无法保存 server 状态 {}: {e}", path.display());
    } else {
        tracing::info!("Server 状态已保存: {}", path.display());
    }
}

/// 工程切换 / 工作区修改后调用，更新记忆。
pub fn remember_last(config_file: &Path, workspace_root: Option<&Path>) {
    let mut state = load_state().unwrap_or_default();
    state.last_config_file = Some(config_file.to_string_lossy().into_owned());
    if let Some(root) = workspace_root {
        state.workspace_root = Some(root.to_string_lossy().into_owned());
    }
    save_state(&state);
}

pub fn remember_workspace_root(root: &Path) {
    let mut state = load_state().unwrap_or_default();
    state.workspace_root = Some(root.to_string_lossy().into_owned());
    save_state(&state);
}
