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

/// 预览端口表的容量上限：一次会话里同时"值得保留"的预览监听数。
///
/// 预览页一次巡回会翻过很多片段，但**正在看的**永远只有一两个：LRU 的 `get` 会把
/// 刚点开的那条顶到最新，所以 16 这个量级给的是"用户来回对比的片段数"，不是总片段数。
/// 超出即淘汰最久未用的那条 —— **淘汰会真的关掉那个监听**（见 `PreviewPorts::insert`）。
pub const MAX_PREVIEW_PORTS: usize = 16;

/// 已启动的预览服务端口表：LRU 上限 + **淘汰时关掉对应监听**（review polish.md E6）。
///
/// key = `"{dir}|{kind}"`（kind: video / cover），消费方 `api/polish.rs::preview_port_for`
/// 只用 `get` / `insert` 两个方法（字段类型换成它不需要那边改一行）。
///
/// `RenderServer` 起的 axum 监听**没有关闭入口**，所以 polish 侧把每个监听的
/// `AbortHandle` 登记在 `yekitrim_polish::render::server::preview_tasks`，这里淘汰时调
/// 它的 `shutdown_port(port)` —— 端口与 tokio 任务一起释放。旧实现这张表**无上限也无淘汰**：
/// 点开 N 个片段的预览 = N 个常驻监听直到进程退出。
///
/// ⚠️ 淘汰只发生在 `insert` 触顶时；`get` **也**算使用（真 LRU）—— 正在看的工程
/// 每次请求都会把它的条目顶到最新，不会被后来者挤掉。
#[derive(Debug)]
pub struct PreviewPorts {
    map: std::collections::HashMap<String, u16>,
    /// 插入/使用顺序，**front = 最旧**。与 `map` 的键集合保持一致。
    order: std::collections::VecDeque<String>,
    /// 本表的容量上限（生产 = [`MAX_PREVIEW_PORTS`]；单测用小表验证淘汰序）。
    cap: usize,
}

impl Default for PreviewPorts {
    fn default() -> Self {
        Self::with_cap(MAX_PREVIEW_PORTS)
    }
}

impl PreviewPorts {
    /// 生产用 `default()`（容量 [`MAX_PREVIEW_PORTS`]）；`with_cap` 让单测用小表验证淘汰序。
    pub(crate) fn with_cap(cap: usize) -> Self {
        Self {
            map: std::collections::HashMap::new(),
            order: std::collections::VecDeque::with_capacity(cap),
            cap,
        }
    }

    /// 取端口并**刷新使用时间**（LRU 的 touch）。返回 `&u16` 是刻意与
    /// `HashMap::get` 同形 —— 调用方的 `.get(key).copied()` 一字不改。
    pub fn get(&mut self, key: &str) -> Option<&u16> {
        if self.map.contains_key(key) {
            self.touch(key);
        }
        self.map.get(key)
    }

    /// 登记端口。表满时淘汰**最久未用**的条目，并**关掉**它对应的预览监听
    /// （不在 registry 里的端口是 no-op —— 例如本进程从未起过那个监听）。
    ///
    /// 同 key 重登记 = 该目录重新起了一次服务（旧监听先淘汰掉，别留悬空端口），
    /// 返回值与 `HashMap::insert` 同形（旧端口）。
    pub fn insert(&mut self, key: String, port: u16) -> Option<u16> {
        let old = self.map.remove(&key);
        self.order.retain(|k| k != &key);
        while self.order.len() >= self.cap {
            let Some(oldest) = self.order.pop_front() else {
                break;
            };
            if let Some(evicted) = self.map.remove(&oldest) {
                yekitrim_polish::render::server::shutdown_port(evicted);
            }
        }
        self.order.push_back(key.clone());
        self.map.insert(key, port);
        old
    }

    fn touch(&mut self, key: &str) {
        self.order.retain(|k| k != key);
        self.order.push_back(key.to_string());
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.map.len()
    }
}

#[derive(Debug, Clone)]
pub struct AppState {
    pub jobs: Arc<JobManager>,

    /// 会话鉴权（账号账本 + 限流）。身份数据不可缺省：打不开库就没有 AppState。
    pub auth: Arc<AuthService>,

    /// 已启动的预览服务端口（LRU，淘汰时关监听）：key = `"{dir}|{kind}"`（kind: video / cover）。
    ///
    /// `RenderServer` 起的监听本身没有关闭入口，复用 + 上限 + 淘汰见 [`PreviewPorts`]。
    /// 服务端在每次请求时按磁盘配置重建 HTML，所以复用不会端出过期内容。
    pub preview_servers: Arc<std::sync::Mutex<PreviewPorts>>,

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
            preview_servers: Arc::new(std::sync::Mutex::new(PreviewPorts::default())),
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

#[cfg(test)]
mod tests {
    use super::*;

    /// 超容淘汰最旧的那条；刚用过的（LRU touch）不淘汰。
    #[test]
    fn lru_evicts_least_recently_used_beyond_cap() {
        let mut p = PreviewPorts::with_cap(2);
        p.insert("a".into(), 1);
        p.insert("b".into(), 2);
        assert_eq!(p.get("a"), Some(&1), "get 也算使用：a 顶到最新");
        p.insert("c".into(), 3);
        assert_eq!(p.len(), 2, "表满再插 = 淘汰一条，不是膨胀");
        assert_eq!(p.get("b"), None, "b 最久未用，被淘汰");
        assert_eq!(p.get("a"), Some(&1), "刚用过的 a 保留");
        assert_eq!(p.get("c"), Some(&3), "刚插入的 c 保留");
    }

    /// 同 key 重登记：换端口 + 刷新使用时间（该目录重新起了一次服务）。
    #[test]
    fn reinsert_refreshes_recency_and_updates_port() {
        let mut p = PreviewPorts::with_cap(2);
        p.insert("a".into(), 1);
        p.insert("b".into(), 2);
        assert_eq!(p.insert("a".into(), 9), Some(1), "返回旧端口（与 HashMap::insert 同形）");
        p.insert("c".into(), 3);
        assert_eq!(p.get("b"), None, "a 重登记后 b 变成最久未用");
        assert_eq!(p.get("a"), Some(&9), "端口已更新");
    }

    /// 淘汰会**真的关掉**监听：起一个真预览服务，把它挤掉，端口随后不再可连
    /// （只从表里删一行、监听还在跑 = 等于没淘汰 —— 这正是 E6 的原始病灶）。
    #[tokio::test]
    async fn eviction_aborts_the_registered_listener() {
        let dir = tempfile::tempdir().unwrap();
        let port = yekitrim_polish::render::server::start_file_server(dir.path().to_path_buf(), None)
            .await
            .expect("起一个真预览服务");
        let addr = format!("127.0.0.1:{port}");

        let mut p = PreviewPorts::with_cap(1);
        p.insert("real".into(), port);
        // 先确认活着（起服务是异步的，给几轮重试）
        let mut alive = false;
        for _ in 0..50 {
            if tokio::net::TcpStream::connect(&addr).await.is_ok() {
                alive = true;
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        assert!(alive, "监听应当已经起来（:{addr}）");

        // 触顶淘汰 → shutdown_port → abort 监听任务
        p.insert("other".into(), 0);
        let mut gone = false;
        for _ in 0..50 {
            if tokio::net::TcpStream::connect(&addr).await.is_err() {
                gone = true;
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        assert!(gone, "被淘汰的监听必须真的关闭（:{addr}）");
        assert_eq!(p.get("real"), None, "表里的条目也一并移除");
    }
}
