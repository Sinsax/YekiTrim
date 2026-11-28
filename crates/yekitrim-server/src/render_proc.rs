//! 渲染/封面子进程的登记、取消与孤儿清扫。
//!
//! `yekitrim polish render` 自己会再拉起 ffmpeg / 底座二进制一堆**孙进程**：
//!
//! - 服务器重启不会带走它们（父进程死了，它们照样活着）；
//! - 取消任务原本只置一个进程内标志，子进程根本不看 —— 停不下来；
//! - 子进程失败退出后，活着的它们还会继续占着 CPU。
//!
//! 实测：两次渲染各留下 3 个渲染进程空转两小时，把后续渲染拖到根本跑不完。
//! 这里把每次渲染的 pid 落盘（`<data>/ledger/render-procs/<job_id>.pid`），
//! 取消时按 pid 杀**整棵进程树**，启动时清扫上一次运行留下的孤儿。

use std::path::{Path, PathBuf};

/// 子进程日志文件：`<data>/logs/render-<job_id>.log`。
///
/// ⚠ 渲染/封面的输出**必须落文件**，不能用管道：
/// ffmpeg / 底座孙进程会继承 stdout 句柄，只要还有一个活着管道就永远不 EOF，
/// `Command::output()` 会一直等下去（实测任务在日志里"一行没有、永远运行中"）。
pub fn log_file(job_id: &str) -> PathBuf {
    log_file_in(&yekitrim_core::path::logs_dir(), job_id)
}

fn log_file_in(logs: &Path, job_id: &str) -> PathBuf {
    logs.join(format!("render-{}.log", sanitize(job_id)))
}

/// 登记一个正在跑的渲染子进程（落 pid 文件，供取消与启动清扫使用）。
pub fn register(job_id: &str, pid: u32) {
    register_in(
        &yekitrim_core::path::ledger_dir(),
        &yekitrim_core::path::logs_dir(),
        job_id,
        pid,
    );
}

fn register_in(ledger: &Path, logs: &Path, job_id: &str, pid: u32) {
    let dir = procs_dir_in(ledger);
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    let _ = std::fs::write(dir.join(format!("{}.pid", sanitize(job_id))), pid.to_string());
    prune_logs_in(logs);
}

/// 任务收工：摘掉登记（不清日志文件，出问题时要能翻）。
pub fn unregister(job_id: &str) {
    let _ = std::fs::remove_file(
        procs_dir_in(&yekitrim_core::path::ledger_dir()).join(format!("{}.pid", sanitize(job_id))),
    );
}

/// 服务启动时清扫上一次运行留下的渲染子进程（还活着的）。
///
/// 只杀**能确认是我们自己的** `polish render/cover` 进程：pid 会被系统回收，
/// 拿一个陈旧的 pid 直接杀可能打到别人的进程上。查不到命令行就跳过（只告警）。
pub fn sweep_orphans() -> Vec<u32> {
    sweep_orphans_in(&yekitrim_core::path::ledger_dir())
}

fn sweep_orphans_in(ledger: &Path) -> Vec<u32> {
    let mut killed = Vec::new();
    let dir = procs_dir_in(ledger);
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return killed;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("pid") {
            continue;
        }
        let pid = std::fs::read_to_string(&path)
            .ok()
            .and_then(|raw| raw.trim().parse::<u32>().ok());
        if let Some(pid) = pid {
            match command_line_of(pid) {
                Some(cmd) if is_our_render(&cmd) => {
                    tracing::warn!("清扫上次运行遗留的渲染子进程 pid {pid}: {}", cmd.trim());
                    kill_tree(pid);
                    killed.push(pid);
                }
                Some(_) => tracing::warn!(
                    "忽略陈旧的渲染 pid 登记 {pid}（现在这个 pid 是别的进程，不杀）"
                ),
                None => tracing::info!("渲染 pid 登记 {pid} 已不存在，清理登记文件"),
            }
        }
        let _ = std::fs::remove_file(&path);
    }
    killed
}

/// 杀掉整棵进程树（渲染子进程 + 它的 ffmpeg / 底座孙进程）。
///
/// Windows 用 `taskkill /T`：只 `child.kill()` 的话孙进程会留下来继续烧 CPU。
pub fn kill_tree(pid: u32) {
    #[cfg(windows)]
    {
        let _ = yekitrim_core::process::cmd("taskkill")
            .args(["/F", "/T", "/PID", &pid.to_string()])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
    }
    #[cfg(not(windows))]
    {
        if pid == 0 {
            return;
        }
        // 我们的渲染子进程在 `run_polish_child` 里 `process_group(0)` 自成一组 ⇒
        // **负 pid = 整个进程组**（子进程 + 它拉起的 ffmpeg / 底座孙进程）。
        // 老版本起的子进程没有自成组（负 pid 找不到组）时退回杀单个，不至于一手不伸。
        let kill = |target: &str| {
            yekitrim_core::process::cmd("kill")
                .args(["-9", target])
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .map(|s| s.success())
                .unwrap_or(false)
        };
        if !kill(&format!("-{pid}"))
            // 组已经没了（正常竞争）就别再对单个 pid 补刀 —— `kill` 会往 stderr 打一行
            // "No such process"，污染渲染日志与测试输出。只有 pid 真的还活着才兜底。
            && yekitrim_service::instancelock::process_alive(pid)
        {
            let _ = kill(&pid.to_string());
        }
    }
}

/// 这个 pid 现在是不是我们的渲染/封面进程？
///
/// 判据：命令行里同时出现「我们的程序名」+ 「polish」+ 「render/cover」。
///
/// ⚠️ 程序名这一项必须写成**完整的品牌名**，不能写成它中间的一截：品牌名是 `yekitrim`，
/// 它并不包含早先那版名字，所以按"名字的一截"去匹配会**一个也认不出来**。曾经踩过一次：
/// 判据静默返回 false ⇒ 启动清扫**认不出**遗留渲染进程、那些 ffmpeg / 底座孙进程
/// **永远不会被回收**（症状是文件被占用、且没有任何报错）。下面的回归用例把它钉住。
fn is_our_render(cmd: &str) -> bool {
    let lower = cmd.to_ascii_lowercase();
    let is_ours = lower.contains("yekitrim");
    let is_polish = lower.contains("polish");
    let is_sub = lower.contains("render") || lower.contains("cover");
    is_ours && is_polish && is_sub
}

/// 查 pid 的命令行。查不到（进程不存在 / 权限不足 / 工具不可用）返回 `None`。
fn command_line_of(pid: u32) -> Option<String> {
    #[cfg(windows)]
    {
        let out = yekitrim_core::process::cmd("wmic")
            .args([
                "process",
                "where",
                &format!("ProcessId={pid}"),
                "get",
                "CommandLine",
                "/value",
            ])
            .output()
            .ok()?;
        let text = String::from_utf8_lossy(&out.stdout);
        text.lines()
            .find_map(|l| l.trim().strip_prefix("CommandLine=").map(|v| v.to_string()))
            .filter(|v| !v.trim().is_empty())
    }
    #[cfg(not(windows))]
    {
        let out = yekitrim_core::process::cmd("ps")
            .args(["-p", &pid.to_string(), "-o", "args="])
            .output()
            .ok()?;
        let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if s.is_empty() { None } else { Some(s) }
    }
}

fn procs_dir_in(ledger: &Path) -> PathBuf {
    ledger.join("render-procs")
}

/// 只留最近 20 份渲染日志（每次渲染都会新增一份，别让它们无限长）。
fn prune_logs_in(logs: &Path) {
    let dir = logs;
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return;
    };
    let mut logs: Vec<(std::time::SystemTime, PathBuf)> = entries
        .flatten()
        .filter(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            name.starts_with("render-") && name.ends_with(".log")
        })
        .filter_map(|e| {
            let mtime = e.metadata().ok()?.modified().ok()?;
            Some((mtime, e.path()))
        })
        .collect();
    if logs.len() <= 20 {
        return;
    }
    logs.sort_by_key(|(t, _)| *t);
    for (_, path) in logs.iter().take(logs.len() - 20) {
        let _ = std::fs::remove_file(path);
    }
}

/// 文件名净化：job_id 目前是 `0018-125829` 这种，仍然挡一手。
fn sanitize(id: &str) -> String {
    id.chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn log_file_lives_under_data_logs() {
        let p = log_file_in(Path::new("/tmp/yekitrim/logs"), "0018-125829");
        assert!(p.to_string_lossy().contains("logs"));
        assert!(p.to_string_lossy().ends_with("render-0018-125829.log"));
    }

    #[test]
    fn sanitize_strips_path_separators() {
        assert_eq!(sanitize("../evil/../x"), "___evil____x");
        assert_eq!(sanitize("0018-125829"), "0018-125829");
    }

    #[test]
    fn only_our_render_processes_are_recognized() {
        // 三个条件都认才放行：是我们的程序 + 子命令是 polish + 动作是 render/cover。
        assert!(is_our_render(r"F:\para\Code\YekiTrim\target\debug\yekitrim.exe polish render F:\a\b"));
        assert!(is_our_render(r"/usr/bin/yekitrim polish cover /srv/a"));
        // 别的程序 / 别的子命令都不认
        assert!(!is_our_render(r"F:\para\Code\YekiTrim\target\debug\yekitrim.exe server"));
        assert!(!is_our_render(r"C:\Program Files\SomeBrowser\browser.exe --headless"));
        // 缺任一个条件都不认
        assert!(!is_our_render(r"/usr/bin/yekitrim render /srv/a"));
        assert!(!is_our_render(r"/usr/bin/yekitrim polish /srv/a"));
    }

    #[test]
    fn register_and_unregister_roundtrip() {
        let dir = std::env::temp_dir().join(format!("yekitrim-renderproc-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        register_in(&dir.join("ledger"), &dir.join("logs"), "0001-000001", 4242);
        let pid_file = procs_dir_in(&dir.join("ledger")).join("0001-000001.pid");
        assert!(pid_file.exists());
        assert_eq!(std::fs::read_to_string(&pid_file).unwrap().trim(), "4242");

        // unregister 用真实路径，这里直接删同一个文件验证语义
        std::fs::remove_file(&pid_file).unwrap();
        assert!(!pid_file.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 陈旧的 pid 指向别的进程时不能乱杀：登记一个一定存在、且不是我们的 pid。
    #[test]
    fn sweep_skips_foreign_pid() {
        let dir = std::env::temp_dir().join(format!("yekitrim-renderproc-sweep-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        // 用当前进程自己的 pid：命令行是测试二进制，不是 yekitrim … polish render
        register_in(&dir.join("ledger"), &dir.join("logs"), "0002-000002", std::process::id());
        let killed = sweep_orphans_in(&dir.join("ledger"));
        assert!(killed.is_empty(), "不该杀掉不是渲染进程的 pid");
        assert!(
            !procs_dir_in(&dir.join("ledger")).join("0002-000002.pid").exists(),
            "登记文件要清掉"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// **Unix：取消渲染必须连孙进程一起收**（2026-10-05 移植报告 §3.2）。
    ///
    /// 根因：子进程没有自成进程组，而 `kill_tree` 又传的是**正** pid ⇒ 只杀
    /// `yekitrim polish render` 自己，它拉起的 ffmpeg / 底座孙进程留在原组里继续跑。
    /// 现在两侧一起改：spawn 时 `process_group(0)`、这里 `kill -9 -<pid>`。
    /// 变异自证：把 `kill_tree` 改回正 pid ⇒ 本测试红（孙进程仍存活）。
    #[cfg(unix)]
    #[test]
    fn kill_tree_reaps_grandchildren_in_its_process_group() {
        use std::os::unix::process::CommandExt;
        use std::time::Duration;
        let dir = std::env::temp_dir().join(format!("yekitrim-killtree-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let pidfile = dir.join("grandchild.pid");
        let script = format!("sleep 30 & echo $! > {}; wait", pidfile.display());
        // 边界门禁：子进程一律走 core 的启动助手（Unix 上是透传，但规则全仓统一）。
        let mut cmd = yekitrim_core::process::cmd("sh");
        cmd.arg("-c")
            .arg(script)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .process_group(0);
        let mut child = cmd.spawn().expect("起测试父进程");
        let pid = child.id();
        let mut grandchild = None;
        for _ in 0..100 {
            if let Ok(raw) = std::fs::read_to_string(&pidfile) {
                if let Ok(n) = raw.trim().parse::<u32>() {
                    grandchild = Some(n);
                    break;
                }
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        let grandchild = grandchild.expect("孙进程应当起来并写出 pid");
        assert!(
            yekitrim_service::instancelock::process_alive(grandchild),
            "孙进程 {grandchild} 没起来"
        );
        kill_tree(pid);
        let _ = child.wait();
        let mut dead = false;
        for _ in 0..150 {
            if !yekitrim_service::instancelock::process_alive(grandchild) {
                dead = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(dead, "孙进程 {grandchild} 仍存活：kill_tree 没有杀整个进程组");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
