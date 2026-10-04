//! 子进程启动助手：**父进程没有控制台时，别让子进程自己新开一个控制台窗口**。
//!
//! 问题场景（2026-10-04 用户报"启动托盘时闪一下命令行"）：`yekitrim-tray` 是
//! `windows_subsystem = "windows"`，**进程没有控制台**；它拉起的每个控制台程序
//! （ffmpeg / ffprobe / node / dhampir / taskkill…）都会被 Windows **分配一个新的控制台窗口** ——
//! 用户看到的就是"闪一下黑框"。
//!
//! 实测：托盘启动（WebUI 自动打开 → `/api/health` 探 `ffmpeg -version` / `ffprobe -version`）
//! 会冒出**两个** `ConsoleWindowClass` 窗口。
//!
//! ⚠️ 判据是"**当前进程有没有控制台**"，不是无条件加 `CREATE_NO_WINDOW`：
//! CLI（有控制台）下行为必须一个字节都不变 —— 子进程继续继承我们的控制台，
//! 终端里该看到的输出/进度照旧。只有托盘那种没有控制台的宿主才需要这条。

use std::ffi::OsStr;
use std::process::Command;

/// `CREATE_NO_WINDOW`（winbase.h）。
///
/// ⚠️ 它与 `CREATE_NEW_CONSOLE` 互斥；也和"子进程要读写控制台"的场景互斥 ——
/// 所以只在**父进程本来就没有控制台**时才加。
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// 当前进程是否**有**控制台。
///
/// 用 `CONOUT$` 试开：拿得到就是有控制台。纯 std —— 不必为了这一句给
/// `yekitrim-core` / `polish` / `service` / `server` 都挂一份 `windows-sys`。
#[cfg(windows)]
fn has_console() -> bool {
    std::fs::OpenOptions::new().write(true).open("CONOUT$").is_ok()
}

/// 起子进程：`Command::new` 的同义替换。
///
/// **新代码一律走这里**，别直接写 `Command::new` —— 直接写就丢掉了"不闪控制台"这一条，
/// 而且丢了不报错（只会在托盘里闪一下，很难往这里想）。
pub fn cmd(program: impl AsRef<OsStr>) -> Command {
    let mut c = Command::new(program);
    hide_console_window(&mut c);
    c
}

/// 给**已经建好的** `Command` 补上这条（少数地方是先 `Command::new` 再按分支拼参数）。
///
/// ⚠️ 用 `creation_flags` 会**覆盖**整个 dwCreationFlags。全仓没有别处设过它，
/// 将来真要再设，得改成"读回来 OR 上去"而不是再调一次。
pub fn hide_console_window(c: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        if !has_console() {
            c.creation_flags(CREATE_NO_WINDOW);
        }
    }
    #[cfg(not(windows))]
    {
        let _ = c;
    }
}
