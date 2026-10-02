//! 单实例保护：`<data_dir>/yekitrim.lock`（设计稿 §3.5.8）。
//!
//! 两个 server 指向同一 `data_dir` 会抢同一批项目、同一个 SQLite（WAL 也挡不住
//! "两个进程各写一批"）与同一个监听端口，因此启动时必须互斥。
//!
//! 做法有两层：
//! 1. **文件独占句柄**（Windows 用 `share_mode(0)`，其它平台用普通句柄 + pid 存活判定）：
//!    进程崩溃时操作系统自动释放句柄，所以"崩溃残留"不会把服务永久锁死；
//! 2. 锁文件里写 `pid` + `started_at`：既能给 `vtrim doctor` 看，也能在句柄已释放、
//!    但记录的进程仍活着（锁文件被复制/共享目录）时提示用 `--force` 接管。
//!
//! 判定规则（`acquire`）：
//! - 拿到独占句柄 + 记录的 pid 仍存活 + 未 `--force` → 拒绝启动（明确报错）；
//! - 拿到独占句柄 + pid 已不存在 → 崩溃残留，自动接管并 warn；
//! - 拿不到独占句柄 → 已有实例正在运行 → 拒绝启动（`--force` 也不接管，因为对方真的在跑）。

use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};

/// 锁文件里记录的持有者信息（诊断用）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LockInfo {
    pub pid: u32,
    pub started_at: String,
}

impl LockInfo {
    /// `pid 1234（启动于 2026-09-12 20:11:00）`
    pub fn label(&self) -> String {
        if self.started_at.is_empty() {
            format!("pid {}", self.pid)
        } else {
            format!("pid {}（启动于 {}）", self.pid, self.started_at)
        }
    }
}

pub fn lock_path(data_dir: &Path) -> PathBuf {
    data_dir.join("yekitrim.lock")
}

fn parse_lock(raw: &str) -> Option<LockInfo> {
    let mut pid = None;
    let mut started_at = String::new();
    for line in raw.lines() {
        match line.split_once('=') {
            Some(("pid", v)) => pid = v.trim().parse::<u32>().ok(),
            Some(("started_at", v)) => started_at = v.trim().to_string(),
            _ => {}
        }
    }
    pid.map(|pid| LockInfo { pid, started_at })
}

/// 读取锁文件内容（拿不到内容时返回 None：如文件不存在或被独占句柄挡住）。
pub fn read_lock(path: &Path) -> Option<LockInfo> {
    let raw = std::fs::read_to_string(path).ok()?;
    parse_lock(&raw)
}

/// 进程是否存活。判定不出来时返回 `false`（宁可放行，也不要让服务永远起不来）。
pub fn process_alive(pid: u32) -> bool {
    if pid == 0 {
        return false;
    }
    #[cfg(windows)]
    {
        // 不引入 windows-sys：tasklist 是系统自带命令，且只在启动/诊断时调用一次。
        let out = std::process::Command::new("tasklist")
            .args(["/FI", &format!("PID eq {pid}"), "/NH", "/FO", "CSV"])
            .output();
        match out {
            Ok(o) => String::from_utf8_lossy(&o.stdout).contains(&format!("\"{pid}\"")),
            Err(_) => false,
        }
    }
    #[cfg(not(windows))]
    {
        if Path::new("/proc").join(pid.to_string()).exists() {
            return true;
        }
        std::process::Command::new("kill")
            .args(["-0", &pid.to_string()])
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }
}

fn open_exclusive(path: &Path) -> std::io::Result<File> {
    let mut opts = OpenOptions::new();
    opts.create(true).read(true).write(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        // 0 = 不共享：第二个进程连打开都会失败，进程死掉则由系统自动释放
        opts.share_mode(0);
    }
    opts.open(path)
}

/// 已持有的单实例锁；进程退出（含 `Drop`）时释放，并尽力删掉锁文件。
pub struct InstanceLock {
    path: PathBuf,
    file: Option<File>,
    info: LockInfo,
}

impl InstanceLock {
    pub fn acquire(data_dir: &Path, force: bool) -> Result<Self> {
        std::fs::create_dir_all(data_dir)
            .with_context(|| format!("无法创建数据目录 {}", data_dir.display()))?;
        let path = lock_path(data_dir);
        let me = std::process::id();
        let started_at = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();

        let file = match open_exclusive(&path) {
            Ok(f) => f,
            Err(e) => {
                // 拿不到独占句柄 = 真有进程在跑（崩溃会释放句柄）
                let holder = read_lock(&path);
                let who = holder
                    .as_ref()
                    .map(|h| h.label())
                    .unwrap_or_else(|| "无法读取持有者信息（锁被独占）".to_string());
                bail!(
                    "已有 yekitrim server 在使用同一个数据目录：{}（{}）。\n\
                     同一 data_dir 不允许两个实例同时运行（会抢项目、端口与 SQLite）。\n\
                     确认那个进程已停止后重试；若要用另一份账号账本，请用 --data-dir <其它目录>。\n\
                     底层错误: {e}",
                    path.display(),
                    who
                );
            }
        };

        // 句柄已独占，但文件里可能记着上一个持有者。
        // 同 pid = 本进程已持有一把（非 Windows 下句柄不互斥，靠这里兜住）。
        let prev = read_file(&file).and_then(|raw| parse_lock(&raw));
        let prev_alive = prev
            .as_ref()
            .map(|p| p.pid == me || process_alive(p.pid))
            .unwrap_or(false);
        if prev_alive && !force {
            let who = prev.as_ref().map(|p| p.label()).unwrap_or_default();
            bail!(
                "锁文件 {} 记录的进程仍在运行：{who}。\n\
                 若确认那是无关进程（例如锁文件是从别的部署复制过来的），用 --force 接管。",
                path.display()
            );
        }
        if prev_alive {
            tracing::warn!("⚠ 已用 --force 接管 yekitrim.lock（原持有者 {who} 仍存活）",
                who = prev.as_ref().map(|p| p.label()).unwrap_or_default());
        } else if let Some(p) = &prev {
            if p.pid != 0 {
                tracing::warn!("检测到上次运行残留的 yekitrim.lock（{}，进程已不存在），已接管",
                    p.label());
            }
        }

        let info = LockInfo { pid: me, started_at };
        let mut file = file;
        file.set_len(0).context("清空锁文件失败")?;
        file.seek(SeekFrom::Start(0))?;
        file.write_all(format!("pid={}\nstarted_at={}\n", info.pid, info.started_at).as_bytes())
            .context("写入锁文件失败")?;
        file.flush().ok();

        Ok(Self { path, file: Some(file), info })
    }

    pub fn info(&self) -> &LockInfo {
        &self.info
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// 读回锁文件当前内容（**通过自己持有的句柄**：独占模式下别人打不开这个文件，
    /// 自己 clone 出来的句柄可以）。
    pub fn read_info(&self) -> Option<LockInfo> {
        self.file.as_ref().and_then(read_file).and_then(|raw| parse_lock(&raw))
    }
}

fn read_file(file: &File) -> Option<String> {
    let mut f = file.try_clone().ok()?;
    f.seek(SeekFrom::Start(0)).ok()?;
    let mut buf = String::new();
    f.read_to_string(&mut buf).ok()?;
    Some(buf)
}

impl Drop for InstanceLock {
    fn drop(&mut self) {
        // Windows 上持独占句柄时无法 `remove_file`（共享模式 0 阻止删除），
        // 所以这里不改"删文件"而是写 `pid=0` 作为"已正常退出"的标记：
        // 下次启动看到 pid=0 会安静接管，不再报"崩溃残留"。
        // 若期间被 `--force` 接管过（内容已是别人的 pid），就别动它。
        if let Some(info) = self.read_info() {
            if info.pid == self.info.pid {
                if let Some(file) = self.file.as_mut() {
                    let _ = file.set_len(0);
                    let _ = file.seek(SeekFrom::Start(0));
                    let _ = file.write_all(b"pid=0\nreleased=1\n");
                    let _ = file.flush();
                }
            }
        }
        self.file.take();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_lock_content() {
        let info = parse_lock("pid=4242\nstarted_at=2026-09-12 20:11:00\n").unwrap();
        assert_eq!(info.pid, 4242);
        assert_eq!(info.started_at, "2026-09-12 20:11:00");
        assert!(info.label().contains("4242"));
        assert!(parse_lock("garbage").is_none());
        assert!(parse_lock("pid=notanumber").is_none());
    }

    #[test]
    fn acquire_writes_pid_and_blocks_second_instance() {
        let dir = tempfile::tempdir().unwrap();
        let lock = InstanceLock::acquire(dir.path(), false).unwrap();
        let info = lock.read_info().expect("锁文件应可读（自己持有句柄）");
        assert_eq!(info.pid, std::process::id());

        // 同一 data_dir 第二次 acquire：Windows 独占句柄 → 打开失败；
        // 其它平台 → 记录的 pid 仍是本进程，同样拒绝。
        let second = InstanceLock::acquire(dir.path(), false);
        assert!(second.is_err(), "同一 data_dir 的第二个实例必须被拒绝");
        assert!(
            format!("{:#}", second.err().unwrap()).contains("已有 yekitrim server"),
            "错误文案要能指导用户"
        );
    }

    #[test]
    fn stale_lock_from_dead_process_is_taken_over() {
        let dir = tempfile::tempdir().unwrap();
        // 造一个"崩溃残留的锁"：pid 一定不存在（u32::MAX 不会是有效进程）
        std::fs::write(
            lock_path(dir.path()),
            format!("pid={}\nstarted_at=2026-01-01 00:00:00\n", u32::MAX),
        )
        .unwrap();
        let lock = InstanceLock::acquire(dir.path(), false).unwrap();
        assert_eq!(lock.info().pid, std::process::id());
        assert!(!process_alive(u32::MAX), "u32::MAX 不应被判为存活");
    }

    #[test]
    fn drop_marks_lock_as_cleanly_released() {
        let dir = tempfile::tempdir().unwrap();
        let path = {
            let lock = InstanceLock::acquire(dir.path(), false).unwrap();
            lock.path().to_path_buf()
        };
        let info = read_lock(&path).expect("释放后应能读到锁文件");
        assert_eq!(info.pid, 0, "正常退出后 pid 归零（下次启动安静接管）");
        // 归零的锁不阻塞下一次启动
        let again = InstanceLock::acquire(dir.path(), false);
        assert!(again.is_ok(), "正常退出后必须能再次启动");
    }
}
