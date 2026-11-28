//! Server 状态持久化：记住上次打开的工程。
//!
//! 文件位置：`<data>/server.toml`（与账本/模型同级，扁平命名）。
//!
//! ⚠️ 这里只放**记忆**（"上次用了哪个工程"），不放**状态**。"当前工程"这个状态
//! 2026-10-03 已删 —— 每个请求由 `scope::Scope` 按 `?project=<id>` 现场解析。
//! 记忆允许过期（条目文件没了就当没有），状态不允许。

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerState {
    /// 配置结构版本（见 `yekitrim_core::config::version`）。
    ///
    /// ⚠️ **这个字段必须存在**：本文件是 `serde` 序列化写出去的，结构体里没有它就等于
    /// "每次保存都把版本行删掉" ⇒ 下次读又当成老文件再升一遍（来回横跳）。
    /// 默认值取**当前版本**：老文件缺键时直接算作当前版本（这份文件是程序自己写的，
    /// 没有"用户手工编辑"的语义，不需要为它保留一套迁移）。
    #[serde(default = "current_config_version")]
    pub config_version: u32,
    /// 上次 WebUI / server 使用的 yekitrim.toml 绝对路径
    #[serde(default)]
    pub last_config_file: Option<String>,
    /// 已知项目 yekitrim.toml 路径列表（多项目直接加载）
    ///
    /// ⚠️ 这是 **P1 时代**的项目清单，现在只用于"启动时只读导入"那条一次性迁移路径
    /// （`lib.rs::import_legacy_projects`）。P2 起归属以 `projects` 表为准，别在这里加新用途。
    #[serde(default)]
    pub projects: Vec<String>,
}

fn current_config_version() -> u32 {
    yekitrim_core::config::CONFIG_VERSION
}

impl Default for ServerState {
    fn default() -> Self {
        Self {
            config_version: yekitrim_core::config::CONFIG_VERSION,
            last_config_file: None,
            projects: Vec::new(),
        }
    }
}

pub fn state_path() -> PathBuf {
    yekitrim_core::path::server_toml_path()
}

pub fn load_state() -> Option<ServerState> {
    let path = state_path();
    // 读之前先跑版本迁移（老 server.toml 缺 `config_version` ⇒ 补上、写回并备份）。
    // ⚠️ 迁移失败不能让状态读不出来（这份文件丢了只是"忘了上次开的工程"）——
    // 退回原文本再试一次解析。
    let content = match yekitrim_core::config::version::prepare(
        &path,
        yekitrim_core::config::ConfigKind::Server,
    ) {
        Ok(p) => p.text,
        Err(e) => {
            tracing::debug!("server.toml 版本迁移跳过（{e:#}），按原文读");
            std::fs::read_to_string(&path).ok()?
        }
    };
    toml::from_str(&content).ok()
}

pub fn save_state(state: &ServerState) {
    let path = state_path();
    // 始终按**当前**版本写（这份文件由程序拥有；比它更新的版本不是这份程序写的）
    let mut state = state.clone();
    state.config_version = yekitrim_core::config::CONFIG_VERSION;
    let state = &state;
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
        tracing::debug!("Server 状态已保存: {}", path.display());
    }
}

/// 状态文件的进程内锁：`load → 改 → save` 必须原子，
/// 否则 `project_list` 循环里逐条 `remember_project` 会互相覆盖丢条目。
static STATE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// 原子的「读 → 改 → 写」。
pub fn update_state<F: FnOnce(&mut ServerState)>(f: F) {
    let _guard = STATE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut state = load_state().unwrap_or_default();
    f(&mut state);
    save_state(&state);
}

/// 工程切换后调用，更新"上次用的是哪个工程"这条记忆。
///
/// `yekitrim server` 不带 `--config` 启动时靠它恢复上次的工程（见 `lib.rs`）。
pub fn remember_last(config_file: &Path) {
    let cfg = config_file.to_string_lossy().into_owned();
    update_state(move |state| {
        state.last_config_file = Some(cfg);
    });
}

/// 将项目 yekitrim.toml 加入已知项目列表（不重复）。
///
/// P2 起项目列表以 `projects` 表为准，这个函数只在过渡期别名写 `server.toml` 时用，
/// 保留是为了让"启动时只读导入"这条迁移路径有对应的写入方。
#[allow(dead_code)]
pub fn remember_project(config_file: &Path) {
    let cfg = config_file.to_string_lossy().into_owned();
    update_state(|state| {
        if !state.projects.contains(&cfg) {
            state.projects.push(cfg);
        }
    });
}

/// 从已知项目列表移除一个 yekitrim.toml（注销项目绑定用）。
///
/// **同时清掉 `last_config_file`** —— 这一步不是顺手做的，少了它注销会被静默撤销：
/// ① 启动流程每次都会把"当前工程"重新写回 `projects[]`（`lib.rs` 的
///    `server_state.projects.push(cfg_str)`）；② `import_legacy_projects` 会把 `projects[]` 里
/// **"配置文件还在、DB 里没行"** 的路径重新登记到 admin 名下。
/// 而注销项目**不删任何文件**（用户要求：只注销绑定），所以 `yekitrim.toml` 一定还在 ——
/// 于是"移除了当前正在看的项目 → 重启一次路径回到列表 → 再重启一次项目自己回来"。
///
/// 清掉 `last_config_file` 之后，下次启动不再恢复那个工程，上面那条链就断了
/// （`Workspace::load(None)` 回退到 `CWD/yekitrim.toml` 且容忍文件缺失，不会因此启动失败）。
pub fn forget_project(config_file: &Path) {
    let cfg = config_file.to_string_lossy().into_owned();
    update_state(|state| forget_project_in(state, &cfg));
}

/// `forget_project` 的**纯逻辑**部分（不落盘，单测直接调它）。
fn forget_project_in(state: &mut ServerState, cfg: &str) {
    state.projects.retain(|p| p != cfg);
    if state.last_config_file.as_deref() == Some(cfg) {
        state.last_config_file = None;
    }
}

/// 旧版（P1 之前）的全局项目列表：`projects` 表接管后由启动流程**只读导入**一次。
///
/// 返回仍然存在的 `yekitrim.toml` 绝对路径；缺失的条目会被忽略（不导入、不报错）。
pub fn legacy_projects() -> Vec<PathBuf> {
    load_state()
        .map(|s| {
            s.projects
                .into_iter()
                .map(PathBuf::from)
                .filter(|p| p.exists())
                .collect()
        })
        .unwrap_or_default()
}

/// 上次记忆的工程（启动时"恢复上次工程"仍用它）。
pub fn last_config_file() -> Option<PathBuf> {
    load_state()
        .and_then(|s| s.last_config_file)
        .map(PathBuf::from)
        .filter(|p| p.exists())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(projects: &[&str], last: Option<&str>) -> ServerState {
        ServerState {
            config_version: yekitrim_core::config::CONFIG_VERSION,
            last_config_file: last.map(|s| s.to_string()),
            projects: projects.iter().map(|s| s.to_string()).collect(),
        }
    }

    /// 注销项目**必须同时清掉 `last_config_file`**，否则移除会被启动流程静默撤销：
    /// `lib.rs` 每次启动都把"当前工程"写回 `projects[]`，而
    /// `import_legacy_projects` 会把 `projects[]` 里"配置还在、DB 没行"的路径重新登记 ——
    /// 注销**不删任何文件**，所以那个配置一定还在，项目就会在第二次重启时自己回来。
    #[test]
    fn forget_project_clears_the_remembered_last_project() {
        let mut s = state(&["/a/yekitrim.toml", "/b/yekitrim.toml"], Some("/a/yekitrim.toml"));
        forget_project_in(&mut s, "/a/yekitrim.toml");
        assert_eq!(s.projects, vec!["/b/yekitrim.toml".to_string()]);
        assert_eq!(s.last_config_file, None, "指向被注销工程的记忆要一起清掉");
    }

    /// 注销**别的**项目时不能顺手清掉当前记忆（否则下次启动白白丢掉"恢复上次工程"）。
    #[test]
    fn forget_project_keeps_an_unrelated_remembered_project() {
        let mut s = state(&["/a/yekitrim.toml", "/b/yekitrim.toml"], Some("/a/yekitrim.toml"));
        forget_project_in(&mut s, "/b/yekitrim.toml");
        assert_eq!(s.projects, vec!["/a/yekitrim.toml".to_string()]);
        assert_eq!(s.last_config_file.as_deref(), Some("/a/yekitrim.toml"));
    }
}
