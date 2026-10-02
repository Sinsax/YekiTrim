//! 进程内「同一文件路径串行化」。
//!
//! Server 的多个 handler 会对 `polish.toml` / `events.json` / `vtrim.toml` /
//! 全局 `config.toml` 做 read-modify-write；前端确实会并发发 PUT（例如时间轴保存
//! 与贴纸配置同时提交），无锁时后写的会覆盖先写的。这里按 canonical path 串行化
//! 同一文件的整段读改写，跨 crate（vtrim-cli / vtrim-polish）共用。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use tokio::sync::Mutex as AsyncMutex;

type LockMap = Mutex<HashMap<PathBuf, Arc<AsyncMutex<()>>>>;

fn locks() -> &'static LockMap {
    static LOCKS: OnceLock<LockMap> = OnceLock::new();
    LOCKS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// 以 canonical path 为键，保证不同写法（绝对/相对/含 `..`）指向同一把锁；
/// 文件还不存在时 canonicalize 失败，退回原路径。
fn key(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

fn lock_for(path: &Path) -> Arc<AsyncMutex<()>> {
    // 锁中毒（持有者 panic）不该让服务不可用：直接恢复内部数据
    let mut map = locks().lock().unwrap_or_else(|e| e.into_inner());
    map.entry(key(path)).or_default().clone()
}

/// 持有 `path` 的进程内锁执行同步闭包（读改写通常都是 std::fs）。
///
/// 注意：闭包内不要 `await`（同步闭包），锁会在函数返回时释放。
pub async fn with_path_lock<F, T>(path: &Path, f: F) -> T
where
    F: FnOnce() -> T,
{
    let lock = lock_for(path);
    let _guard = lock.lock().await;
    f()
}

/// 取 `path` 的进程内异步锁（返回 guard）。
///
/// 适合「先查后写 + 中间需要 await（spawn_blocking / ffmpeg）」的场景：
/// 同一路径的并发请求会排队，后到者在拿到锁后重新检查即可避免重复工作。
pub async fn lock_path(path: &Path) -> tokio::sync::OwnedMutexGuard<()> {
    lock_for(path).lock_owned().await
}

type SyncLockMap = Mutex<HashMap<PathBuf, Arc<Mutex<()>>>>;

fn sync_locks() -> &'static SyncLockMap {
    static LOCKS: OnceLock<SyncLockMap> = OnceLock::new();
    LOCKS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn sync_lock_for(path: &Path) -> Arc<Mutex<()>> {
    let mut map = sync_locks().lock().unwrap_or_else(|e| e.into_inner());
    map.entry(key(path)).or_default().clone()
}

/// **同步**版路径锁：给跑在同步上下文里的 read-modify-write 用。
///
/// 为什么不让异步版统一：core 管线的 `edit_toml_at`（写 `vtrim.toml`）是同步函数，
/// 它在阻塞线程上执行，拿不到 tokio 的异步锁；而设置页保存项目配置走的是
/// `with_path_lock` + 同一个 `edit_toml_at`。两边写的是同一份文件，无锁时后写的会把
/// 先写的键覆盖掉（管线刚写 `transcript_file`，设置页同时保存别的字段 → 丢一个）。
///
/// ⚠️ 只在**微秒级**的文件读改写里用：锁在同步上下文里是阻塞等待，别拿它包 ffmpeg /
/// 网络请求这类慢操作。另外它是**非重入**的（std Mutex），不能在已持锁的同一线程里再取。
pub fn with_path_lock_sync<F, T>(path: &Path, f: F) -> T
where
    F: FnOnce() -> T,
{
    let lock = sync_lock_for(path);
    let _guard = lock.lock().unwrap_or_else(|e| e.into_inner());
    f()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 20 个并发 read-modify-write 必须全部生效（无锁时会丢更新）。
    #[tokio::test]
    async fn serializes_read_modify_write() {
        let path = std::env::temp_dir().join("vtrim-filelock-test.txt");
        let _ = std::fs::remove_file(&path);
        std::fs::write(&path, "0").unwrap();

        let mut handles = Vec::new();
        for _ in 0..20 {
            let p = path.clone();
            handles.push(tokio::spawn(async move {
                with_path_lock(&p, || {
                    let n: u32 = std::fs::read_to_string(&p).unwrap().trim().parse().unwrap();
                    std::fs::write(&p, (n + 1).to_string()).unwrap();
                })
                .await;
            }));
        }
        for h in handles {
            h.await.unwrap();
        }

        let n: u32 = std::fs::read_to_string(&path).unwrap().trim().parse().unwrap();
        assert_eq!(n, 20);
        let _ = std::fs::remove_file(&path);
    }

    /// 不同路径不互相阻塞（按 canonical path 建锁）。
    #[tokio::test]
    async fn distinct_paths_use_distinct_locks() {
        let a = std::env::temp_dir().join("vtrim-filelock-a.txt");
        let b = std::env::temp_dir().join("vtrim-filelock-b.txt");
        std::fs::write(&a, "a").unwrap();
        std::fs::write(&b, "b").unwrap();
        let (ra, rb) = tokio::join!(
            with_path_lock(&a, || std::fs::read_to_string(&a).unwrap()),
            with_path_lock(&b, || std::fs::read_to_string(&b).unwrap()),
        );
        assert_eq!(ra.trim(), "a");
        assert_eq!(rb.trim(), "b");
        let _ = std::fs::remove_file(&a);
        let _ = std::fs::remove_file(&b);
    }
}
