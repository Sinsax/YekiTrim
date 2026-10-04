//! `yekitrim-tray`：Windows 托盘外壳 —— 双击即后台起 server，并自动打开界面。
//!
//! 职责边界与 `main.rs` 同源：**本文件只做入口装配**，不作为地重实现任何领域逻辑。
//! 起服务一律经 `yekitrim_server::run_with()`（CLI 卡里那条"要用服务端能力只能经 run() 进入"的规矩）。
//!
//! 形状：
//!   · 主线程 = Win32 消息循环（`Shell_NotifyIcon` 的硬要求：托盘回调必须挂在线程消息泵上）
//!   · 后台线程 = tokio runtime，跑 `run_with`，用一个 oneshot 做**优雅关停**
//!   · 左键点图标 / 右键菜单「打开网页」= 开浏览器（地址取回调回来的**实际**地址）
//!   · 右键菜单「退出」= 触发关停 → server 收尾（`InstanceLock` 的 Drop 写 `pid=0`）→ 消息循环退出
//!
//! 它按**本地免登录**起服务（`ServerArgs::local_no_auth`）：桌面单机形态不该先让人登录一遍。
//! 服务端那边有两道闸兜着（Host 必须 loopback、写请求必须同源），见 `auth::guard_local_request`。
//!
//! ⚠️ `windows_subsystem = "windows"` **只在这个 bin 上**（Cargo 的每个 bin 各自成一个 crate），
//! 所以 `yekitrim.exe` 那个控制台程序不受影响。
//!
//! ⚠️ 非 Windows 平台照样要能构建（开发机/CI 不都是 Windows），所以这里只留一个占位 main。

#![cfg_attr(windows, windows_subsystem = "windows")]

// 与 main.rs 共用同一份装配。⚠️ 托盘**必须**也装：没有它，托盘起的服务
// 既没有日志文件、也没有 WebUI 的「运行日志」——一个没控制台的后台进程没日志就完全瞎了。
// 传 false：托盘是 windows_subsystem="windows"，根本没有 stdout。
mod tracing_setup;

fn main() {
    tracing_setup::setup(false);
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
    use std::sync::{Arc, Mutex};

    // ⚠️ windows-sys 0.52 的句柄一律是 `isize`（不是指针）⇒ 空句柄写 0、判空写 == 0。
    //    别照 0.6x 的写法用 null_mut()/is_null()，那是对不上的。
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

    /// 托盘回调消息（`WM_APP` 起，避开系统消息段）
    const WM_TRAY: u32 = WM_APP + 1;
    const ID_OPEN: usize = 1;
    const ID_QUIT: usize = 2;
    const TRAY_UID: u32 = 1;

    /// 挂在窗口 `GWLP_USERDATA` 上的共享状态
    struct Shared {
        /// `run_with` 回调回来的**实际**地址（端口被占时服务端会自己换一个，不能假定）
        url: Mutex<Option<String>>,
        /// 优雅关停的发送端
        shutdown: Mutex<Option<tokio::sync::oneshot::Sender<()>>>,
        /// 是否已经发过关停（重复点「退出」不该二次 send）
        stopping: Mutex<bool>,
        /// 托盘窗口句柄。⚠️ 服务线程**必须**用它来叫醒主线程：`PostQuitMessage` 只投给
        /// **调用它的那个线程**的消息队列 —— 在服务线程里调，主线程的 `GetMessageW` 永远收不到，
        /// 于是"服务已经退出、托盘进程却赖着不走"（实测踩到）。
        hwnd: Mutex<isize>,
    }

    impl Shared {
        fn open(&self) {
            match self.url.lock().unwrap().clone() {
                Some(u) => open_url(&u),
                None => message_box(
                    "YekiTrim",
                    "服务还在启动中（或启动失败）。\n详情见数据目录下 logs/ 里的日志。",
                ),
            }
        }
        fn quit(&self) {
            let mut stopping = self.stopping.lock().unwrap();
            if *stopping {
                return;
            }
            *stopping = true;
            // 先把图标摘掉，再等 server 收尾。
            // ⚠️ server 那边有 3 秒关停时限（页面的 SSE 要等），进程不会立刻消失 ——
            // 图标还杵在通知区里的话，用户看到的就是"点了退出没反应"。先摘图标 = 立刻有反馈。
            let hwnd = *self.hwnd.lock().unwrap();
            if hwnd != 0 {
                tray_icon(hwnd, 0, "", false);
            }
            if let Some(tx) = self.shutdown.lock().unwrap().take() {
                let _ = tx.send(());
            }
        }
    }

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    /// 开浏览器。
    ///
    /// ⚠️ 用 `ShellExecuteW`，**不要退回 `cmd /c start`**：托盘是无控制台子系统，
    /// 它一起去起 `cmd.exe` 会**闪一个黑色控制台窗口**（实测：托盘启动自动开页时闪一次、
    /// 右键「打开网页」时又闪一次）。`ShellExecuteW` 不建进程、不碰控制台。
    ///
    /// ⚠️ 这里**故意不复用** `yekitrim_server` 里那个 `open_browser`：那条机器门禁
    /// （`scripts/check_module_boundaries.py` 的「cli 只用 server 的入口」）只放行
    /// `yekitrim_server::{run*, ServerArgs}`，而它同时被根 AGENTS.md 的架构段引用（预算只剩个位数）。
    /// 为 3 行 Windows 专属代码去松门禁、再动根文件，不划算。
    fn open_url(url: &str) {
        use windows_sys::Win32::UI::Shell::ShellExecuteW;
        use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
        let op = wide("open");
        let u = wide(url);
        // 返回值 <= 32 才是失败（HINSTANCE 语义），> 32 表示成功
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
            tracing::warn!("ShellExecuteW 打开 {url} 失败（返回值 {r}）");
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

    /// 建/删托盘图标。`tip` 是悬停提示（Win32 上限 128 个 u16，故截断）。
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
            tracing::error!("Shell_NotifyIconW(NIM_ADD) 失败：通知区不会有图标");
        }
    }

    unsafe extern "system" fn wndproc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        if msg == WM_CREATE {
            // 窗口创建时把 lpCreateParams 存进 GWLP_USERDATA，之后每次回调取回来
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
                // 左键单击（抬起）直接开网页 —— 用户要的"单击也能打开网页"
                WM_LBUTTONUP => {
                    shared.open();
                    0
                }
                // 右键菜单：实际地址（灰显只读）/ 打开网页 / 退出
                WM_RBUTTONUP | WM_CONTEXTMENU => {
                    unsafe {
                        let menu = CreatePopupMenu();
                        if menu == 0 {
                            return 0;
                        }
                        // 第一行是**实际**地址，灰显不可点 —— 端口被换过时用户能看见真实地址
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

    pub fn main_impl() {
        // 端口：默认取**唯一权威常量** `doctor::DEFAULT_PORT`；`YEKITRIM_PORT` 只作覆盖
        // （开发时想跟已在跑的实例并存、或自动化测试要用）。⚠️ 别在这里写第二个端口字面量。
        let port = std::env::var("YEKITRIM_PORT")
            .ok()
            .and_then(|v| v.trim().parse::<u16>().ok())
            .unwrap_or(yekitrim_service::doctor::DEFAULT_PORT);
        let base_url = format!("http://127.0.0.1:{port}");

        // ---- ⓪ 先看这个端口上是不是**已经有一个 YekiTrim** ----
        // 双击第二次的合理反应是"打开它"，而不是报"端口被占"。这同时绕开了
        // 「服务端发现对端是另一个 YekiTrim 就拒绝启动」那条路径（那条是给 CLI 用的）。
        if yekitrim_service::doctor::probe_health(&base_url).is_some() {
            open_url(&base_url);
            return;
        }

        // ---- ① server 参数（与 `yekitrim server` 默认值一致；端口引用唯一常量，不写字面量）----
        let args = yekitrim_server::ServerArgs {
            host: "127.0.0.1".to_string(),
            port,
            // 托盘自己在监听就绪后开浏览器（那时才拿得到**实际**地址），所以不用 server 的 --open
            open: false,
            open_path: "/".to_string(),
            config: None,
            data_dir: None,
            admin_password_file: None,
            force: false,
            allow_remote: false,
            trust_proxy: false,
            // ⭐ 托盘就是"双击就用的桌面形态"：免登录、按本机管理员处理。
            //    服务端还有两道闸（Host 必须 loopback / 写请求必须同源），见 auth::guard_local_request。
            local_no_auth: true,
        };

        let shared = Arc::new(Shared {
            url: Mutex::new(None),
            shutdown: Mutex::new(None),
            stopping: Mutex::new(false),
            hwnd: Mutex::new(0),
        });

        // ---- ② 后台线程跑 server（主线程要留给消息循环）----
        {
            let shared = Arc::clone(&shared);
            let (tx, rx) = tokio::sync::oneshot::channel::<()>();
            *shared.shutdown.lock().unwrap() = Some(tx);
            std::thread::spawn(move || {
                let rt = match tokio::runtime::Builder::new_multi_thread()
                    .enable_all()
                    .build()
                {
                    Ok(rt) => rt,
                    Err(e) => {
                        message_box("YekiTrim 启动失败", &format!("无法创建运行时：{e}"));
                        unsafe { PostQuitMessage(1) };
                        return;
                    }
                };
                rt.block_on(async move {
                    let opened = Arc::new(Mutex::new(false));
                    let opened2 = Arc::clone(&opened);
                    // 回调闭包要吃掉一个 Arc，后面还要用 shared 取 hwnd ⇒ 先克隆一份给它
                    let shared_cb = Arc::clone(&shared);
                    let res = yekitrim_server::run_with(
                        args,
                        async move {
                            let _ = rx.await;
                        },
                        move |url| {
                            *shared_cb.url.lock().unwrap() = Some(url.to_string());
                            // 用户要求：挂后台的同时**自动打开**对应网页（只开一次）
                            if !*opened2.lock().unwrap() {
                                *opened2.lock().unwrap() = true;
                                open_url(url);
                            }
                        },
                    )
                    .await;
                    if let Err(e) = res {
                        // ⚠️ 托盘模式**没有控制台**：起不来必须弹出来，否则用户看到的是"双击了没反应"
                        let raw = format!("{e:#}");
                        let hint = if raw.contains("已在运行") || raw.contains("lock") {
                            "\n\n看起来已经有一个 YekiTrim 在跑（同一数据目录只允许一个实例）。\n\
                         可以直接用浏览器打开它，或先退出原来那个。"
                        } else {
                            "\n\n详情见数据目录下 logs/ 里的日志。"
                        };
                        message_box("YekiTrim 启动失败", &format!("{raw}{hint}"));
                    }
                    // ⚠️ 必须走 `PostMessageW(hwnd, WM_CLOSE)`，不能在这里 `PostQuitMessage`：
                    //    PostQuitMessage 只投给**当前线程**的队列（这里是服务线程），主线程的
                    //    GetMessageW 收不到 ⇒ 服务没了、托盘进程却一直挂着。
                    //    发给托盘窗口后由 DefWindowProc 走 DestroyWindow → 主线程收到 WM_DESTROY
                    //    → 那里的 PostQuitMessage 才是在主线程上、才真的打破消息循环。
                    let hwnd = *shared.hwnd.lock().unwrap();
                    if hwnd != 0 {
                        unsafe { PostMessageW(hwnd, WM_CLOSE, 0, 0) };
                    }
                });
            });
        }

        // ---- ③ 隐藏窗口承载托盘回调 ----
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
                0, // 无父窗口
                0, // 无菜单
                hinst,
                Arc::as_ptr(&shared) as *mut c_void,
            );
            if hwnd == 0 {
                message_box("YekiTrim 启动失败", "无法创建托盘窗口。");
                std::process::exit(1);
            }

            *shared.hwnd.lock().unwrap() = hwnd;

            // 图标：优先随包的 yekitrim.ico（多尺寸，DPI 缩放好看），缺失退回系统图标 ——
            // 托盘**不能因为少个图标就起不来**。
            let icon = load_icon(hinst);
            tray_icon(
                hwnd,
                icon,
                "YekiTrim · 切片工作台（左键开网页，右键更多）",
                true,
            );

            // ---- ④ 消息循环：等「退出」触发关停 → PostQuitMessage 打破它 ----
            let mut msg: MSG = std::mem::zeroed();
            while GetMessageW(&mut msg, 0, 0, 0) > 0 {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }

            tray_icon(hwnd, icon, "", false);
            DestroyWindow(hwnd);
        }
    }

    /// 图标数据**编进二进制**。
    ///
    /// ⚠️ 不要退回"运行时去 exe 同级读 `yekitrim.ico`"：开发时 exe 在 `target/release/`、
    /// 发行时在安装目录，两处都容易没有那个文件 —— 而**托盘没图标是静默的**（`Shell_NotifyIconW`
    /// 照样返回成功，只是通知区里空一块）。实测就是这样：我本地 `target/release/` 下没有 `.ico`，
    /// 托盘里一直没有图标。编进二进制后，图标不再依赖任何外部文件。
    const ICON_ICO: &[u8] = include_bytes!("../assets/yekitrim.ico");

    /// 从 `.ico` 的目录里挑一张最接近 `want` 的图，返回 (偏移, 长度)。
    ///
    /// ⚠️ `CreateIconFromResourceEx` 要的是**单张图**（RT_ICON 那种 BITMAPINFOHEADER+掩码 / PNG），
    /// **不是整个 .ico 文件**（.ico 外面还套着 6 字节 ICONDIR + 每张 16 字节的目录项）。
    /// 直接把整个文件喂进去会失败。
    fn pick_ico_image(want: u16) -> Option<(usize, usize)> {
        let b = ICON_ICO;
        if b.len() < 6 || b[0] != 0 || b[1] != 0 || b[2] != 1 || b[3] != 0 {
            return None; // reserved=0 type=1(icon) count>0
        }
        let count = u16::from_le_bytes([b[4], b[5]]) as usize;
        let rd16 = |o: usize| u16::from_le_bytes([b[o], b[o + 1]]) as usize;
        let rd32 = |o: usize| u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]]) as usize;
        let mut best: Option<(usize, usize, usize)> = None; // (差距, 偏移, 长度)
        for i in 0..count {
            let e = 6 + i * 16;
            if e + 16 > b.len() {
                break;
            }
            // 宽高字段 0 表示 256
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

    /// 托盘图标：**exe 自带资源** → 内嵌 .ico → exe 同级 `yekitrim.ico` → 系统默认图标。
    ///
    /// 为什么留前两条：① 是"与资源管理器同一份"，最正；② 不依赖链接期资源（万一 build.rs 没跑成，
    /// 或是别的构建方式），实测就是它把"托盘没图标"这个静默故障先兜住的。
    fn load_icon(hinst: isize) -> isize {
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            CreateIconFromResourceEx, GetSystemMetrics, LR_DEFAULTCOLOR, LR_LOADFROMFILE,
            SM_CXSMICON, SM_CYSMICON,
        };
        let (cx, cy) = unsafe { (GetSystemMetrics(SM_CXSMICON), GetSystemMetrics(SM_CYSMICON)) };
        // ① exe 资源（`build.rs` 把 assets/yekitrim.rc 编了进去，ID = 1）
        let h = unsafe { LoadIconW(hinst, 1 as *const u16) };
        if h != 0 {
            tracing::info!("托盘图标：用 exe 自带的图标资源");
            return h;
        }
        // ① 内嵌数据（首选，任何 cwd / 任何安装形状都成立）
        if let Some((off, size)) = pick_ico_image(cx.max(16) as u16) {
            let h = unsafe {
                CreateIconFromResourceEx(
                    ICON_ICO[off..].as_ptr(),
                    size as u32,
                    1, // fIcon = TRUE
                    0x0003_0000,
                    cx,
                    cy,
                    LR_DEFAULTCOLOR,
                )
            };
            if h != 0 {
                tracing::info!("托盘图标：用内嵌 .ico（{cx}x{cy}）");
                return h;
            }
            tracing::warn!("内嵌 .ico 创建图标失败，退回文件/系统图标");
        }
        // ② exe 同级 yekitrim.ico（发行包里 xtask 会放一份）
        if let Some(dir) = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        {
            let ico = dir.join("yekitrim.ico");
            if ico.is_file() {
                let path = wide(&ico.to_string_lossy());
                let h = unsafe {
                    LoadImageW(hinst, path.as_ptr(), IMAGE_ICON, cx, cy, LR_LOADFROMFILE)
                };
                if h != 0 {
                    tracing::info!("托盘图标：用 exe 同级的 yekitrim.ico");
                    return h;
                }
            }
        }
        tracing::warn!("托盘图标：退回系统默认图标（通知区里会是个通用图标）");
        unsafe { LoadIconW(0, IDI_APPLICATION) }
    }
}
