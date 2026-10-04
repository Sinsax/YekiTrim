//! `yekitrim-tray`：Windows 托盘 **薄启动器** —— 双击即后台起 `yekitrim server`，并打开界面。
//!
//! ## 它为什么必须"薄"
//!
//! 这里**刻意不 `use` 任何 `yekitrim_*` crate**（连 `yekitrim-core::path` 都不碰）。
//! 原因：`yekitrim-core` → `sherpa-onnx`（静态链接的 C++ ASR 引擎）**是整条依赖图的根**，
//! 只要引用 core 的任何一个符号，链接器就会把 ASR 一起拉进来 —— 最重的一坨（`libsherpa_onnx_sys`
//! 的 rlib 单个 **869 MB**，最终 exe 里几十 MB）。上一版就是这样：托盘 57 MB，和主程序一样重。
//!
//! 所以本文件只用 **std + windows-sys**，并把"真正干活"整个交给同目录的 `yekitrim.exe`：
//!
//! ```text
//! 双击 yekitrim-tray.exe
//!  ① 生成关停令牌（只在内存与子进程 env 里）→ spawn `yekitrim.exe server --local-no-auth --open`
//!       · CREATE_NO_WINDOW：它是 console 子系统，不这样会闪一个黑框
//!       · stdout/stderr 都接管道：stdout 用来抓"实际监听地址"，两个都必须**持续排空**
//!  ② 从子进程 stdout 抓 `WebUI: http://…` → 记下来（托盘菜单要用）
//!       子进程很快退出且码 0、又没抓到地址 ⇒ "端口上已经有一个 YekiTrim，已帮你打开"
//!       （这条是 `server` 自己处理的：`--open` + 对端是 YekiTrim ⇒ 开它的页面并 Ok 返回）
//!  ③ 托盘循环：左键开网页 / 右键 · 打开网页 · 退出
//!  ④ 退出 = 摘图标（立刻有反馈）→ POST `/api/shutdown` 带令牌 → 等子进程收尾 → 超时则 taskkill
//! ```
//!
//! ## 两条跨进程契约（本文件与 `yekitrim-server` 各写一份，**改一处必须改两处**）
//!
//! 1. 关停令牌走 **环境变量**（`SHUTDOWN_TOKEN_ENV`）：本仓有现成的不变式"秘密不进进程列表"
//!    （`--admin-password-file` 就是为它存在的），所以不用命令行参数。
//! 2. 实际监听地址从子进程 stdout 的 **`WebUI: http://…`** 那行抓。
//!
//! ⚠️ 这两条本可以靠链接 `yekitrim-server` 共享常量来保证，但那样就把 ASR 拉回来了 ——
//! 这个"抄一份"是**刻意换体积**的取舍；改了名字的表现是托盘退不掉（降级成 taskkill），不会崩。
//!
//! ## 已知取舍
//!
//! · 两个进程：托盘（几 MB）+ 真正干活的 `yekitrim.exe`。任务管理器里干活的是后者。
//! · 托盘被强杀后服务会留着，此时再双击 ⇒ 新托盘只会"打开已有实例"然后退出，**没有图标可停它**
//!   （要停就用任务管理器，或 `taskkill /IM yekitrim.exe`）。

#![cfg_attr(windows, windows_subsystem = "windows")]

fn main() {
    #[cfg(windows)]
    win::main_impl();
    #[cfg(not(windows))]
    {
        eprintln!("yekitrim-tray 只有 Windows 版；其它平台请直接用 `yekitrim server`。");
        std::process::exit(1);
    }
}

#[cfg(windows)]
mod win {
    use std::ffi::c_void;
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::TcpStream;
    use std::os::windows::process::CommandExt;
    use std::path::PathBuf;
    use std::process::{Child, Command, Stdio};
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
    use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows_sys::Win32::UI::Shell::{
        Shell_NotifyIconW, NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NOTIFYICONDATAW,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        AppendMenuW, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyMenu, DestroyWindow,
        DispatchMessageW, GetCursorPos, GetMessageW, GetWindowLongPtrW, LoadCursorW, LoadIconW,
        LoadImageW, PostMessageW, PostQuitMessage, RegisterClassW, SetForegroundWindow,
        SetWindowLongPtrW, TrackPopupMenu, TranslateMessage, CREATESTRUCTW, CW_USEDEFAULT,
        GWLP_USERDATA, IDC_ARROW, IDI_APPLICATION, IMAGE_ICON, MF_GRAYED, MF_SEPARATOR, MF_STRING,
        MSG, TPM_RETURNCMD, TPM_RIGHTBUTTON, WM_APP, WM_CLOSE, WM_COMMAND, WM_CONTEXTMENU,
        WM_CREATE, WM_DESTROY, WM_LBUTTONUP, WM_RBUTTONUP, WNDCLASSW, WS_EX_NOACTIVATE,
        WS_OVERLAPPED,
    };

    // ---------------------------------------------------------------- 跨进程契约
    /// ⚠️ 必须与 `yekitrim-server` 的 `auth::SHUTDOWN_TOKEN_ENV` **逐字一致**（见文件头"两条契约"）。
    const SHUTDOWN_TOKEN_ENV: &str = "YEKITRIM_SHUTDOWN_TOKEN";
    /// ⚠️ 必须与 `yekitrim-server` 的 `auth::SHUTDOWN_TOKEN_HEADER` 逐字一致。
    const SHUTDOWN_TOKEN_HEADER: &str = "x-yekitrim-shutdown-token";
    /// 子进程启动时打的那行（`WebUI: http://…`）—— 契约之二。
    const URL_MARKER: &str = "WebUI: http://";
    /// "已经有一个在跑"时 `server` 打的那句（用于把"已开好页面"与"启动失败"区分开）。
    const ALREADY_RUNNING_MARKER: &str = "已有一个 YekiTrim";

    const WM_TRAY: u32 = WM_APP + 1;
    const ID_OPEN: usize = 1;
    const ID_QUIT: usize = 2;
    const TRAY_UID: u32 = 1;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    /// 等子进程把监听地址打出来的上限（慢盘上首次建库+装字体可能要十几秒，给宽一点）
    const READY_TIMEOUT: Duration = Duration::from_secs(45);
    /// 发出关停请求后，等子进程自己收尾的上限（server 侧还有 3 秒的优雅关停时限）
    const EXIT_TIMEOUT: Duration = Duration::from_secs(10);

    /// 挂在窗口 `GWLP_USERDATA` 上的共享状态
    struct Shared {
        /// 子进程 stdout 里抓到的**实际**地址（端口被占时 server 会自己换一个）
        url: Mutex<Option<String>>,
        child: Mutex<Option<Child>>,
        token: String,
        stopping: Mutex<bool>,
        hwnd: Mutex<isize>,
    }

    impl Shared {
        fn open(&self) {
            match self.url.lock().unwrap().clone() {
                Some(u) => open_url(&u),
                None => message_box(
                    "YekiTrim",
                    "服务还在启动中（或启动失败）。\n详情见数据目录 logs/ 下的日志。",
                ),
            }
        }

        fn quit(&self) {
            let mut stopping = self.stopping.lock().unwrap();
            if *stopping {
                return;
            }
            *stopping = true;
            // ① 先摘图标：server 侧还要等 SSE 收尾（最多 3 秒 + 我们这里最多 10 秒），
            //    图标还杵着的话用户看到的就是"点了退出没反应"。
            let hwnd = *self.hwnd.lock().unwrap();
            if hwnd != 0 {
                tray_icon(hwnd, 0, "", false);
            }
            // ② 请 server 优雅关停（等价 Ctrl+C：锁会被写成 pid=0）
            let mut child = self.child.lock().unwrap().take();
            if let (Some(url), Some(ch)) = (self.url.lock().unwrap().clone(), child.as_mut()) {
                let sent = post_shutdown(&url, &self.token);
                log(format!("关停请求已发出={sent}（{url}）"));
                // ③ 等它自己收尾
                if wait_exit(ch, EXIT_TIMEOUT) {
                    log("子进程已正常退出");
                    self.close_window();
                    return;
                }
                log("等超时，强制结束子进程");
            }
            if let Some(mut ch) = child {
                kill_tree(ch.id());
                let _ = ch.wait();
            }
            self.close_window();
        }

        /// 打破消息循环（`WM_CLOSE` → `WM_DESTROY` → `PostQuitMessage`）。
        ///
        /// ⚠️ 这里**必须自己发**：旧（同进程）设计里是 server 线程在收尾后 `PostMessageW(WM_CLOSE)`，
        /// 现在托盘进程里根本没有 server 线程。而"看门狗"那条腿也指望不上 —— `quit()` 已经把
        /// child 从 `Shared` 里 `take()` 走了，看门狗再 `try_wait` 只会看到 `None`。
        /// 漏了这句的表现是：**子进程已经优雅退出、锁也干净了，托盘进程却一直挂着**（实测踩到）。
        fn close_window(&self) {
            let hwnd = *self.hwnd.lock().unwrap();
            if hwnd != 0 {
                unsafe { PostMessageW(hwnd, WM_CLOSE, 0, 0) };
            }
        }
    }

    /// 等子进程退出；`true` = 在时限内退出了。
    fn wait_exit(ch: &mut Child, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            match ch.try_wait() {
                Ok(Some(_)) => return true,
                Ok(None) => std::thread::sleep(Duration::from_millis(150)),
                Err(_) => return false,
            }
        }
        false
    }

    fn kill_tree(pid: u32) {
        let _ = Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/T", "/F"])
            .creation_flags(CREATE_NO_WINDOW)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }

    // ---------------------------------------------------------------- 小工具
    fn crate_dir() -> Option<PathBuf> {
        std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.to_path_buf()))
    }

    /// 数据目录。
    ///
    /// ⚠️ 这里是 `yekitrim_core::path::data_dir()` 规则的**第二份实现**（`YEKITRIM_DATA_DIR` ?? exe 同级）
    /// —— 链接 core 只为拿这个路径会把 ASR 拉回来，所以刻意抄。改动请一起改（规则正文在
    /// `docs/file-layout.md`）。本文件只拿它来放托盘自己的日志。
    fn data_dir() -> PathBuf {
        std::env::var_os("YEKITRIM_DATA_DIR")
            .map(PathBuf::from)
            .filter(|p| !p.as_os_str().is_empty())
            .or_else(crate_dir)
            .unwrap_or_else(|| PathBuf::from("."))
    }

    fn log(msg: impl AsRef<str>) {
        let dir = data_dir().join("logs");
        let _ = std::fs::create_dir_all(&dir);
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(dir.join("tray.log"))
        {
            let ts = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            let _ = writeln!(f, "[{ts}] {}", msg.as_ref());
        }
    }

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    /// 开浏览器。
    ///
    /// ⚠️ 用 `ShellExecuteW`，**不要退回 `cmd /c start`**：本进程是无控制台子系统，
    /// 起 `cmd.exe` 会**闪一个黑框**（实测：启动自动开页闪一次、右键「打开网页」又闪一次）。
    fn open_url(url: &str) {
        use windows_sys::Win32::UI::Shell::ShellExecuteW;
        use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
        let op = wide("open");
        let u = wide(url);
        let r = unsafe {
            ShellExecuteW(
                0,
                op.as_ptr(),
                u.as_ptr(),
                std::ptr::null(),
                std::ptr::null(),
                SW_SHOWNORMAL,
            )
        };
        if r <= 32 {
            log(format!("ShellExecuteW 打开 {url} 失败（返回值 {r}）"));
        }
    }

    fn message_box(title: &str, body: &str) {
        use windows_sys::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_ICONINFORMATION, MB_OK};
        unsafe {
            MessageBoxW(
                0,
                wide(body).as_ptr(),
                wide(title).as_ptr(),
                MB_OK | MB_ICONINFORMATION,
            );
        }
    }

    /// 建/删托盘图标（`tip` 上限 128 个 u16，故截断）。
    fn tray_icon(hwnd: HWND, icon: isize, tip: &str, add: bool) {
        let mut nid: NOTIFYICONDATAW = unsafe { std::mem::zeroed() };
        nid.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
        nid.hWnd = hwnd;
        nid.uID = TRAY_UID;
        nid.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
        nid.uCallbackMessage = WM_TRAY;
        nid.hIcon = icon;
        let t = wide(tip);
        let n = t.len().min(nid.szTip.len());
        nid.szTip[..n].copy_from_slice(&t[..n]);
        let ok = unsafe { Shell_NotifyIconW(if add { NIM_ADD } else { NIM_DELETE }, &nid) };
        if add && ok == 0 {
            // 静默失败最坑：进程活着、菜单也能弹，就是通知区里没有它
            log("Shell_NotifyIconW(NIM_ADD) 失败：通知区不会有图标");
        }
    }

    // ---------------------------------------------------------------- 起子进程
    fn spawn_server(token: &str) -> std::io::Result<Child> {
        let exe = crate_dir()
            .map(|d| d.join("yekitrim.exe"))
            .filter(|p| p.is_file())
            .ok_or_else(|| {
                std::io::Error::new(std::io::ErrorKind::NotFound, "同目录找不到 yekitrim.exe")
            })?;
        let mut cmd = Command::new(exe);
        cmd.arg("server")
            // 桌面单机形态：免登录（服务端另有两道闸，见 server 侧 api 卡）
            .arg("--local-no-auth");
        // 端口覆盖：`YEKITRIM_PORT` 是唯一的口子（开发时想跟已在跑的实例并存、自动化测试要用）。
        // ⚠️ 不设就**不传 `--port`**，让 server 用自己那份唯一权威常量 —— 托盘里不出现端口字面量。
        if let Ok(p) = std::env::var("YEKITRIM_PORT") {
            if !p.trim().is_empty() {
                cmd.arg("--port").arg(p.trim());
            }
        }
        cmd
            // ⭐ 让 **server 自己**按**实际**地址开浏览器 —— 这样端口被占导致它换了端口也开得对，
            //   还顺带处理了"已经有一个在跑"（它会去开那个实例的页面然后 Ok 返回）。
            .arg("--open")
            // 关停令牌：env 而不是命令行参数（不进进程列表）
            .env(SHUTDOWN_TOKEN_ENV, token)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        cmd.creation_flags(CREATE_NO_WINDOW);
        cmd.spawn()
    }

    /// 持续读一个子进程输出流，逐行交给 `on_line`。
    ///
    /// ⚠️ **必须一直读到 EOF**：管道容量有限，父进程不读，子进程写满就**卡死**。
    fn pump<R: Read + Send + 'static>(r: R, on_line: Arc<dyn Fn(&str) + Send + Sync>) {
        std::thread::spawn(move || {
            for line in BufReader::new(r).lines().map_while(Result::ok) {
                on_line(&line);
            }
        });
    }

    // ---------------------------------------------------------------- 关停请求
    /// 往控制端点发一次 `POST /api/shutdown`（手写 HTTP —— 本进程不链接任何 HTTP 栈）。
    fn post_shutdown(url: &str, token: &str) -> bool {
        let Some(rest) = url.strip_prefix("http://") else {
            return false;
        };
        let hostport = rest.split('/').next().unwrap_or(rest);
        let addr = if hostport.contains(':') {
            hostport.to_string()
        } else {
            format!("{hostport}:80")
        };
        let Ok(sa) = addr.parse() else { return false };
        let Ok(mut s) = TcpStream::connect_timeout(&sa, Duration::from_secs(3)) else {
            return false;
        };
        let _ = s.set_read_timeout(Some(Duration::from_secs(5)));
        // 用 HTTP/1.0：服务端会把连接关掉，于是 read_to_string 会自然结束
        let req = format!(
            "POST /api/shutdown HTTP/1.0\r\nHost: {hostport}\r\n{SHUTDOWN_TOKEN_HEADER}: {token}\r\nContent-Length: 0\r\n\r\n"
        );
        if s.write_all(req.as_bytes()).is_err() {
            return false;
        }
        let mut resp = String::new();
        let _ = s.read_to_string(&mut resp);
        // 看一眼状态行就够（不解析 body）
        resp.lines()
            .next()
            .map(|l| l.contains(" 200"))
            .unwrap_or(false)
    }

    // ---------------------------------------------------------------- 托盘窗口
    unsafe extern "system" fn wndproc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        if msg == WM_CREATE {
            let cs = lparam as *const CREATESTRUCTW;
            let shared = unsafe { (*cs).lpCreateParams } as *mut Shared;
            unsafe { SetWindowLongPtrW(hwnd, GWLP_USERDATA, shared as isize) };
            return 0;
        }
        let ptr = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *mut Shared;
        if ptr.is_null() {
            return unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) };
        }
        let shared = unsafe { &*ptr };
        match msg {
            WM_TRAY => match lparam as u32 {
                WM_LBUTTONUP => {
                    shared.open();
                    0
                }
                WM_RBUTTONUP | WM_CONTEXTMENU => {
                    unsafe {
                        let menu = CreatePopupMenu();
                        if menu == 0 {
                            return 0;
                        }
                        let addr = shared
                            .url
                            .lock()
                            .unwrap()
                            .clone()
                            .unwrap_or_else(|| "启动中…".to_string());
                        AppendMenuW(menu, MF_STRING | MF_GRAYED, 0, wide(&addr).as_ptr());
                        AppendMenuW(menu, MF_SEPARATOR, 0, std::ptr::null());
                        AppendMenuW(menu, MF_STRING, ID_OPEN, wide("打开网页").as_ptr());
                        AppendMenuW(menu, MF_SEPARATOR, 0, std::ptr::null());
                        AppendMenuW(menu, MF_STRING, ID_QUIT, wide("退出").as_ptr());
                        let mut pt = POINT { x: 0, y: 0 };
                        GetCursorPos(&mut pt);
                        // ⚠️ 必须先 SetForegroundWindow，否则点到菜单外面它不消失（托盘菜单的经典坑）
                        SetForegroundWindow(hwnd);
                        let cmd = TrackPopupMenu(
                            menu,
                            TPM_RETURNCMD | TPM_RIGHTBUTTON,
                            pt.x,
                            pt.y,
                            0,
                            hwnd,
                            std::ptr::null(),
                        ) as usize;
                        DestroyMenu(menu);
                        match cmd {
                            ID_OPEN => shared.open(),
                            ID_QUIT => shared.quit(),
                            _ => {}
                        }
                    }
                    0
                }
                _ => 0,
            },
            WM_COMMAND => {
                match wparam & 0xffff {
                    ID_OPEN => shared.open(),
                    ID_QUIT => shared.quit(),
                    _ => {}
                }
                0
            }
            WM_DESTROY => {
                unsafe { PostQuitMessage(0) };
                0
            }
            _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
        }
    }

    /// 托盘图标：**exe 自带资源** → 内嵌 .ico → exe 同级 `yekitrim.ico` → 系统默认。
    ///
    /// ⚠️ 别退回"运行时只读 exe 同级那个 .ico"：开发时 exe 在 `target/release/`、那里没有它，
    /// 而**没图标是静默的**（`NIM_ADD` 照样返回成功，只是通知区空一块）。实测就这么踩过。
    fn load_icon(hinst: isize) -> isize {
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            CreateIconFromResourceEx, GetSystemMetrics, LR_DEFAULTCOLOR, LR_LOADFROMFILE,
            SM_CXSMICON, SM_CYSMICON,
        };
        let (cx, cy) = unsafe { (GetSystemMetrics(SM_CXSMICON), GetSystemMetrics(SM_CYSMICON)) };
        // ① exe 资源（`build.rs` 把 assets/yekitrim.rc 编了进去，ID = 1）
        let h = unsafe { LoadIconW(hinst, 1 as *const u16) };
        if h != 0 {
            return h;
        }
        // ② 内嵌字节
        if let Some((off, size)) = pick_ico_image(cx.max(16) as u16) {
            let h = unsafe {
                CreateIconFromResourceEx(
                    ICON_ICO[off..].as_ptr(),
                    size as u32,
                    1,
                    0x0003_0000,
                    cx,
                    cy,
                    LR_DEFAULTCOLOR,
                )
            };
            if h != 0 {
                return h;
            }
        }
        // ③ exe 同级文件
        if let Some(dir) = crate_dir() {
            let ico = dir.join("yekitrim.ico");
            if ico.is_file() {
                let path = wide(&ico.to_string_lossy());
                let h = unsafe {
                    LoadImageW(hinst, path.as_ptr(), IMAGE_ICON, cx, cy, LR_LOADFROMFILE)
                };
                if h != 0 {
                    return h;
                }
            }
        }
        log("托盘图标：退回系统默认图标");
        unsafe { LoadIconW(0, IDI_APPLICATION) }
    }

    const ICON_ICO: &[u8] = include_bytes!("../assets/yekitrim.ico");

    /// 从 `.ico` 目录里挑一张最接近 `want` 的图，返回 (偏移, 长度)。
    ///
    /// ⚠️ `CreateIconFromResourceEx` 要的是**单张图**，**不是整个 .ico 文件**
    /// （外面还套着 6 字节 ICONDIR + 每张 16 字节的目录项）。
    fn pick_ico_image(want: u16) -> Option<(usize, usize)> {
        let b = ICON_ICO;
        if b.len() < 6 || b[0] != 0 || b[1] != 0 || b[2] != 1 || b[3] != 0 {
            return None;
        }
        let count = u16::from_le_bytes([b[4], b[5]]) as usize;
        let rd16 = |o: usize| u16::from_le_bytes([b[o], b[o + 1]]) as usize;
        let rd32 = |o: usize| u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]]) as usize;
        let mut best: Option<(usize, usize, usize)> = None;
        for i in 0..count {
            let e = 6 + i * 16;
            if e + 16 > b.len() {
                break;
            }
            let w = match rd16(e) {
                0 => 256usize,
                v => v,
            };
            let size = rd32(e + 8);
            let off = rd32(e + 12);
            if off + size > b.len() {
                continue;
            }
            let d = w.abs_diff(want as usize);
            if best.map(|(bd, _, _)| d < bd).unwrap_or(true) {
                best = Some((d, off, size));
            }
        }
        best.map(|(_, off, size)| (off, size))
    }

    // ---------------------------------------------------------------- 主流程
    pub fn main_impl() {
        // 令牌：只在内存、只通过 env 给子进程（32 字节十六进制，避免额外的随机源依赖）
        let token = new_token();

        let tail: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let shared = Arc::new(Shared {
            url: Mutex::new(None),
            child: Mutex::new(None),
            token: token.clone(),
            stopping: Mutex::new(false),
            hwnd: Mutex::new(0),
        });

        let mut child = match spawn_server(&token) {
            Ok(c) => c,
            Err(e) => {
                message_box(
                    "YekiTrim 启动失败",
                    &format!("无法启动同目录的 yekitrim.exe：{e}\n\n托盘需要和 yekitrim.exe 放在同一个目录里。"),
                );
                return;
            }
        };
        log(format!("已起子进程 pid={}", child.id()));

        // stdout：抓实际地址；两个流都**必须**持续排空（管道满了子进程会卡死）
        if let Some(out) = child.stdout.take() {
            let sh = Arc::clone(&shared);
            let tl = Arc::clone(&tail);
            pump(
                out,
                Arc::new(move |line: &str| {
                    if let Some(i) = line.find(URL_MARKER) {
                        let addr = line[i + URL_MARKER.len()..]
                            .split_whitespace()
                            .next()
                            .unwrap_or("")
                            .trim_end_matches('/');
                        if !addr.is_empty() {
                            let url = format!("http://{addr}");
                            if sh.url.lock().unwrap().is_none() {
                                log(format!("实际地址 = {url}"));
                            }
                            *sh.url.lock().unwrap() = Some(url);
                        }
                    }
                    keep_tail(&tl, line);
                }),
            );
        }
        if let Some(err) = child.stderr.take() {
            let tl = Arc::clone(&tail);
            pump(
                err,
                Arc::new(move |line: &str| {
                    keep_tail(&tl, line);
                }),
            );
        }
        *shared.child.lock().unwrap() = Some(child);

        // 等"就绪 / 已在运行 / 失败"三者之一
        let deadline = Instant::now() + READY_TIMEOUT;
        loop {
            if shared.url.lock().unwrap().is_some() {
                break; // 就绪（子进程已自己开好浏览器）
            }
            let exited = {
                let mut g = shared.child.lock().unwrap();
                match g.as_mut().map(|c| c.try_wait()) {
                    Some(Ok(Some(st))) => Some(st.code()),
                    Some(Err(_)) => Some(None),
                    _ => None,
                }
            };
            if let Some(code) = exited {
                let out = tail_text(&tail);
                if code == Some(0) && out.contains(ALREADY_RUNNING_MARKER) {
                    // 端口上已经有一个在跑：server 已经开好它的页面并 Ok 返回 —— 静默收工
                    log("已有实例在跑，已打开它的页面，托盘退出");
                    return;
                }
                message_box(
                    "YekiTrim 启动失败",
                    &format!(
                        "子进程退出（code={code:?}）。\n\n最近输出：\n{}",
                        short_tail(&out)
                    ),
                );
                return;
            }
            if Instant::now() >= deadline {
                log("等待就绪超时");
                if let Some(mut c) = shared.child.lock().unwrap().take() {
                    kill_tree(c.id());
                    let _ = c.wait();
                }
                message_box(
                    "YekiTrim 启动超时",
                    &format!(
                        "等了 {} 秒还没看到服务就绪。\n\n最近输出：\n{}",
                        READY_TIMEOUT.as_secs(),
                        short_tail(&tail_text(&tail))
                    ),
                );
                return;
            }
            std::thread::sleep(Duration::from_millis(200));
        }

        // ---- 建隐藏窗口承载托盘回调 ----
        unsafe {
            let hinst = GetModuleHandleW(std::ptr::null());
            let cls = wide("YekiTrimTrayWnd");
            let mut wc: WNDCLASSW = std::mem::zeroed();
            wc.lpfnWndProc = Some(wndproc);
            wc.hInstance = hinst;
            wc.lpszClassName = cls.as_ptr();
            wc.hCursor = LoadCursorW(0, IDC_ARROW);
            RegisterClassW(&wc);

            let hwnd = CreateWindowExW(
                WS_EX_NOACTIVATE,
                cls.as_ptr(),
                wide("YekiTrim").as_ptr(),
                WS_OVERLAPPED,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                0,
                0,
                0,
                0,
                hinst,
                Arc::as_ptr(&shared) as *mut c_void,
            );
            if hwnd == 0 {
                message_box("YekiTrim 启动失败", "无法创建托盘窗口。");
                shared.quit();
                return;
            }
            *shared.hwnd.lock().unwrap() = hwnd;

            let icon = load_icon(hinst);
            tray_icon(
                hwnd,
                icon,
                "YekiTrim · 切片工作台（左键开网页，右键更多）",
                true,
            );

            // 子进程若自己退了（崩溃/被关停），让消息循环也结束，别留一个孤儿托盘
            {
                let sh = Arc::clone(&shared);
                std::thread::spawn(move || loop {
                    std::thread::sleep(Duration::from_millis(700));
                    let exited = {
                        let mut g = sh.child.lock().unwrap();
                        match g.as_mut().map(|c| c.try_wait()) {
                            Some(Ok(Some(_))) => true,
                            _ => false,
                        }
                    };
                    if exited {
                        log("子进程已退出，托盘跟着收工");
                        let h = *sh.hwnd.lock().unwrap();
                        if h != 0 {
                            PostMessageW(h, WM_CLOSE, 0, 0);
                        }
                        return;
                    }
                });
            }

            let mut msg: MSG = std::mem::zeroed();
            while GetMessageW(&mut msg, 0, 0, 0) > 0 {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }

            tray_icon(hwnd, icon, "", false);
            DestroyWindow(hwnd);
        }
        // 消息循环结束（含"子进程已退出"那条腿）：确保子进程没被落下
        // ⚠️ 必须用块表达式把 MutexGuard 提前放掉：直接写 `shared.child.lock().unwrap().take()`
        // 会让那个临时 guard 活到语句结束，而 `shared` 是最后被 drop 的局部变量 ⇒ E0597。
        let leftover = { shared.child.lock().unwrap().take() };
        if let Some(mut c) = leftover {
            if !wait_exit(&mut c, Duration::from_secs(2)) {
                kill_tree(c.id());
                let _ = c.wait();
            }
        }
    }

    /// 32 字节十六进制令牌。用系统时间 + 进程 id + 栈地址混合 ——
    /// **够用即可**：它只在"本机 + 本次会话"内有效，且端点还卡着 loopback。
    fn new_token() -> String {
        let t = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let stack = &t as *const _ as usize;
        let mut x = t as u64
            ^ ((t >> 64) as u64).rotate_left(17)
            ^ (std::process::id() as u64).rotate_left(33)
            ^ (stack as u64);
        let mut s = String::with_capacity(32);
        for _ in 0..4 {
            // xorshift64*
            x ^= x >> 12;
            x ^= x << 25;
            x ^= x >> 27;
            s.push_str(&format!("{:016x}", x.wrapping_mul(0x2545_F491_4F6C_DD1D)));
        }
        s
    }

    fn keep_tail(tail: &Arc<Mutex<Vec<String>>>, line: &str) {
        let mut t = tail.lock().unwrap();
        if t.len() >= 40 {
            t.remove(0);
        }
        t.push(line.to_string());
    }

    fn tail_text(tail: &Arc<Mutex<Vec<String>>>) -> String {
        tail.lock().unwrap().join("\n")
    }

    /// 弹窗里只放最后几行 —— 整段输出塞进 MessageBox 会很难看
    fn short_tail(s: &str) -> String {
        let lines: Vec<&str> = s.lines().filter(|l| !l.trim().is_empty()).collect();
        let n = lines.len();
        lines[n.saturating_sub(8)..].join("\n")
    }
}
