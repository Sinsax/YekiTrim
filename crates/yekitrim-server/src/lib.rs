//! Server 支持：axum HTTP + WebUI + JobManager。
//!
//! 独立 crate（原 `vtrim-cli/src/server/`）。分层是**单向**的：
//!
//! ```text
//! vtrim-core ─┐
//! vtrim-polish ┼─→ vtrim-service ──→ vtrim-server ──→ vtrim-cli（只剩入口）
//!             └──────────────────────────────────────┘
//! ```
//!
//! - `vtrim server` 子命令启动
//! - REST API 覆盖项目/状态/配置/数据/管线/精修
//! - SSE 推送任务日志与进度
//! - **账号与会话**：启动即引导 admin 账号；除 `/api/health`、`/api/auth/login`
//!   外的全部接口都要求会话（含 loopback，无免登录分支，见 `auth.rs`）
//! - WebUI = `webui/dist`（Vue3 构建产物）；**不再内嵌单文件页面**，
//!   缺失时由 `web::webui_fallback` 返回 503 + 构建指引
//!
//! **不变式：本 crate 只依赖 `vtrim-service`，绝不反向。** 服务端要用的领域逻辑
//! 一律通过 `vtrim-service` 暴露（`vtrim-service` 也因此不能 `use yekitrim_server::*`）。
//!
//! 设计文档: docs/superpowers/specs/2026-09-03-vtrim-webui-design.md
//!           docs/superpowers/specs/2026-09-12-multi-user-server-design.md（§3.5 / §10.1）

pub mod access_log;
pub mod api;
pub mod auth;
pub mod download_worker;
pub mod ingest;
pub mod ingest_worker;
pub mod jobs;
pub mod media;
pub mod model;
pub mod persist;
pub mod pipeline_boot;
pub mod ratelimit;
pub mod render_proc;
pub mod scope;
pub mod scope_key;
pub mod stickers;
pub mod tool_probe;
pub mod web;

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Context;
use axum::Router;
use axum::routing::get;
use clap::Parser;
use tokio::net::TcpListener;
use tracing::info;

use yekitrim_service::instancelock::InstanceLock;
use yekitrim_service::logbus;
use yekitrim_service::{self, db};

use self::auth::Listening;

/// `vtrim server` CLI 参数（由 main.rs 转发）。
#[derive(Debug, Clone, Parser)]
pub struct ServerArgs {
    /// 监听地址
    #[arg(long, default_value = "127.0.0.1")]
    pub host: String,
    /// 监听端口
    ///
    /// 默认 26320 是**挑出来躲开系统保留段**的（老默认 7788 在部分 Windows 上属于
    /// Hyper-V/WSL 的保留段，绑不上且 `netstat` 里看不到任何监听者）。理由与安全区间见
    /// `yekitrim_service::doctor::DEFAULT_PORT`。
    #[arg(long, default_value_t = yekitrim_service::doctor::DEFAULT_PORT)]
    pub port: u16,
    /// 启动后自动打开浏览器
    #[arg(long)]
    pub open: bool,
    /// 项目配置文件路径
    #[arg(long)]
    pub config: Option<PathBuf>,

    /// 允许监听非 loopback 地址。
    ///
    /// 服务可读写本机工程目录并调用 ffmpeg，默认只允许 127.0.0.1 / ::1；
    /// 确需对外监听必须显式打开，且**不允许仍在使用初始密码**（§3.5.4）。
    #[arg(long)]
    pub allow_remote: bool,

    /// 信任反向代理转发的 `X-Forwarded-*` 头（**只在你前面确实是自己那台反代时打开**）。
    ///
    /// 默认**关**：这两个头请求方可以随意伪造。打开后按 `X-Forwarded-For` 的**最后一段**
    /// 做限流与审计（`auth::resolve_client_ip`），并在 `X-Forwarded-Proto: https` 时把会话
    /// cookie 标 `Secure`（`auth::tls_terminated`）。反代部署不打开的话，所有请求会被当成
    /// 来自反代本机（限流粒度退化成全局），cookie 也不会带 `Secure`。
    #[arg(long)]
    pub trust_proxy: bool,

    /// 账号账本 / 会话 / 初始密码文件所在目录（默认 `<exe_dir>/.yekitrim/cloud`，也可用 `YEKITRIM_DATA_DIR`）
    #[arg(long)]
    pub data_dir: Option<PathBuf>,

    /// 无人值守部署：从这个文件读取初始管理员密码（不进进程列表）
    #[arg(long)]
    pub admin_password_file: Option<PathBuf>,

    /// 强制接管 yekitrim.lock（仅在确认上一个实例已崩溃残留时使用）
    #[arg(long)]
    pub force: bool,

    /// 打开浏览器时使用的初始路径（例如 hash 路由；CLI 内部使用）
    #[arg(long, hide = true, default_value = "/")]
    pub open_path: String,
}

/// 旧版项目列表（`.vtrim/server.toml` 的 `projects[]`）一次性导入 admin 名下。
///
/// P2 起**归属由 DB 决定**；老用户升级后原来的项目不能凭空消失，因此在这里只读导入：
/// 已登记过的路径会跳过（幂等），缺失的条目忽略。
fn import_legacy_projects(db: &db::Db) -> anyhow::Result<usize> {
    let Some(admin) = db.find_user("admin")? else {
        return Ok(0);
    };
    let mut imported = 0usize;
    for config_file in persist::legacy_projects() {
        if db.project_by_config(&config_file)?.is_some() {
            continue;
        }
        let Some(root_dir) = config_file.parent() else { continue };
        let name = root_dir
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "project".to_string());
        db.create_project(
            admin.id,
            &name,
            None,
            "local",
            Some("本机文件"),
            root_dir,
            &config_file,
            None,
        )?;
        imported += 1;
    }
    Ok(imported)
}

/// 数据目录可写性预检：身份数据不可缺省，写不了就**启动失败**（§3.5.7）。
fn prepare_data_dir(dir: &Path) -> anyhow::Result<()> {
    std::fs::create_dir_all(dir).with_context(|| {
        format!(
            "无法创建账号数据目录 {}。\n\
             身份数据不可缺省（配置可缺省，账号不能）：请用 --data-dir <可写路径> 指定别处，\
             或用环境变量 YEKITRIM_DATA_DIR",
            dir.display()
        )
    })?;
    let probe = dir.join(".vtrim-write-probe");
    std::fs::write(&probe, b"ok").with_context(|| {
        format!(
            "账号数据目录不可写: {}。\n\
             请用 --data-dir <可写路径> 指定别处（不会退化成「内存模式」：那会导致改过的密码/建过的账号重启后消失）",
            dir.display()
        )
    })?;
    let _ = std::fs::remove_file(&probe);
    Ok(())
}

/// 解析显式初始密码（`--admin-password-file` > `YEKITRIM_ADMIN_PASSWORD`）。
fn explicit_admin_password(args: &ServerArgs) -> anyhow::Result<Option<String>> {
    if let Some(path) = &args.admin_password_file {
        let raw = std::fs::read_to_string(path)
            .with_context(|| format!("读取 --admin-password-file 失败: {}", path.display()))?;
        let pw = raw.trim_end_matches(['\r', '\n']).to_string();
        if pw.is_empty() {
            anyhow::bail!("--admin-password-file 内容为空: {}", path.display());
        }
        return Ok(Some(pw));
    }
    if let Ok(pw) = std::env::var("YEKITRIM_ADMIN_PASSWORD") {
        if !pw.trim().is_empty() {
            tracing::warn!(
                "⚠ 正在使用环境变量 YEKITRIM_ADMIN_PASSWORD 作为初始密码：它会出现在 /proc/<pid>/environ、\
                 `docker inspect` 等位置。生产部署请改用 --admin-password-file"
            );
            return Ok(Some(pw));
        }
    }
    Ok(None)
}

/// 对外暴露（非 loopback 监听，或显式 `--allow-remote`）且仍在用初始密码 → 拒绝启动。
///
/// ⚠️ 判据必须是"是否对外暴露"，不是"是否非 loopback"：`--host 127.0.0.1 --allow-remote`
/// 是 `docs/deploy.md` 推荐的公网部署姿势（反代就在本机），旧判据只认非 loopback，
/// 于是文档承诺的护栏在它自己推荐的配置下从不触发。
fn must_refuse_initial_password(
    is_loopback: bool,
    allow_remote: bool,
    must_change_password: bool,
) -> bool {
    (!is_loopback || allow_remote) && must_change_password
}

/// 启动 HTTP server（阻塞直至监听关闭）。
pub async fn run(args: ServerArgs) -> anyhow::Result<()> {
    let addr: SocketAddr = format!("{}:{}", args.host, args.port)
        .parse()
        .with_context(|| format!("无法解析监听地址: {}:{}", args.host, args.port))?;
    let is_loopback = match addr.ip() {
        std::net::IpAddr::V4(v4) => v4.is_loopback(),
        std::net::IpAddr::V6(v6) => v6.is_loopback(),
    };
    let listening = if is_loopback { Listening::Loopback } else { Listening::Remote };
    // 代理信任开关要在建路由**之前**落定：限流键、cookie 的 Secure 判定都读它（见 auth）
    auth::set_trust_proxy(args.trust_proxy);

    // ---- 账号数据（身份不可缺省）----
    let data_dir = args.data_dir.clone().unwrap_or_else(db::data_dir);
    if args.data_dir.is_some() {
        // ⚠️ **写端**：把 `--data-dir` 经环境变量传给 service 侧（`db::data_dir` / `logbus` /
        // 初始密码文件路径都读它）。**必须与 `yekitrim_core::path::DATA_DIR_ENV` 同源** ——
        // 写成字面量时，改名只改读端漏改写端 ⇒ `--data-dir` **静默失效**：
        // server 用一个目录、CLI 侧命令读另一个，症状是"命令行说没这个用户、网页里明明有"，
        // 而且**只有传了 `--data-dir` 才暴露**。
        std::env::set_var(yekitrim_core::path::DATA_DIR_ENV, &data_dir);
    }
    prepare_data_dir(&data_dir)?;
    // 日志落盘（<data_dir>/logs/yekitrim-YYYYMMDD.log）。要在启动横幅**之前**开，
    // 否则"监听地址 / 初始密码"这些只有一次的信息进不了文件。
    let log_dir = logbus::enable_file_log();
    info!("日志文件: {}", logbus::file_log_path(&log_dir).display());
    let explicit_pw = explicit_admin_password(&args)?;
    let auth = auth::AuthService::open(data_dir.clone(), listening).with_context(|| {
        format!(
            "账号账本不可用（{}）。请检查磁盘空间/权限，或用 --data-dir <可写路径> 指定别处",
            data_dir.display()
        )
    })?;
    // LLM 用量记账：core 的 LlmClient 只负责产出记录，这里把它接到账号账本上
    // （网页端、CLI 端后续所有调用都会落 `llm_usage` 表；设置页「LLM 用量」读它）
    yekitrim_service::usage::install(auth.db_arc());
    let bootstrap = yekitrim_service::ensure_admin_bootstrap(explicit_pw.clone())?;
    if bootstrap.created {
        match (&bootstrap.initial_password, &bootstrap.password_file) {
            (Some(pw), Some(path)) => info!(
                "首次启动管理员账号: admin ｜ 初始密码: {pw} ｜ 已同时写入 {}，首次登录成功后该文件自动删除",
                path.display()
            ),
            (Some(_), None) => info!("首次启动管理员账号: admin（使用你提供的初始密码）"),
            _ => {}
        }
    } else {
        info!("账号账本: {} 个账号（{}）", bootstrap.accounts, auth.data_dir().display());
        if bootstrap.must_change_password {
            tracing::warn!("⚠ 管理员仍在使用初始密码，请尽快登录并修改（或 `yekitrim user passwd admin`）");
        }
    }
    // 对外监听 + 仍用初始密码 = 把密码写在服务器上 → 拒绝启动（§3.5.4）
    if !is_loopback {
        if !args.allow_remote {
            anyhow::bail!(
                "拒绝监听非 loopback 地址 {addr}：本服务可读写本机工程目录并调用外部命令。\
                 如确需对外监听，请显式加上 --allow-remote"
            );
        }
        tracing::warn!("⚠ 正在监听非 loopback 地址 {addr}：请务必使用 HTTPS 反向代理，并确保账号密码足够强");
        if args.trust_proxy {
            tracing::info!("已信任反向代理头（--trust-proxy）：限流按 X-Forwarded-For 最后一段，cookie 按 X-Forwarded-Proto 标 Secure");
        } else {
            tracing::info!("未信任反向代理头：所有请求都算作来自反代本机（限流粒度退化），且不会下发 Secure cookie；确认前置反代后加 --trust-proxy");
        }
    }
    // ⚠️ 判据是"是否**对外暴露**"，不是"是否非 loopback"：`--host 127.0.0.1 --allow-remote`
    // 是 `docs/deploy.md` 推荐的公网部署姿势（反代就在本机），旧判据只认非 loopback，
    // 于是文档承诺的这条护栏在它自己推荐的配置下**从不触发**。
    if must_refuse_initial_password(is_loopback, args.allow_remote, bootstrap.must_change_password) {
        anyhow::bail!(
            "拒绝在 {addr} 上对外监听：管理员仍在使用初始密码（初始密码会出现在日志与 {} 里，等同把密码写在服务器上）。\n\
             请先修改密码：yekitrim user passwd admin（非交互：--stdin / --password-file），\
             或在部署时用 --admin-password-file 显式设定初始密码。",
            yekitrim_service::initial_password_path().display()
        );
    }

    // 单实例保护：两个实例指向同一 data_dir 会抢项目/端口/SQLite 写（§3.5.8）
    let _instance_lock = InstanceLock::acquire(&data_dir, args.force)?;
    info!(
        "单实例锁: {}（{}）",
        _instance_lock.path().display(),
        _instance_lock.info().label()
    );

    // ---- 项目托管（P2）：旧 server.toml 列表只读导入 admin，并刷新占用 ----
    match import_legacy_projects(auth.db()) {
        Ok(0) => {}
        Ok(n) => info!("已把 {n} 个旧版项目导入 admin 名下（projects 表接管项目列表）"),
        Err(e) => tracing::warn!("导入旧版项目列表失败（不影响启动）: {e:#}"),
    }
    let refreshed = yekitrim_service::projects::refresh_all_bytes(auth.db());
    info!("项目占用已刷新（{refreshed} 个项目）");

    // 首次运行（没有程序配置）时自动生成默认配置；已有配置不会被覆盖
    match yekitrim_service::ensure_global_config() {
        Ok(report) => {
            for path in &report.created {
                info!("未找到程序配置，已生成默认配置: {}", path.display());
            }
            for (path, backup, parse_err) in &report.repaired {
                info!("⚠ 程序配置解析失败，已备份并重建: {} → {}（原因: {parse_err}）",
                    path.display(), backup.display());
            }
        }
        // 配置目录不可写（如只读安装位置）不应阻止 server 启动，用默认值继续
        Err(e) => info!("⚠ 自动生成程序配置失败（将使用内建默认值）: {e:#}"),
    }

    // 启动工程（2026-10-03 起**只是"上次用的是哪个"这条记忆**，不再是全局状态）：
    // 显式 `--config` 优先，否则恢复上次记忆的那份；都没有就什么都不做。
    // 它**不进 `AppState`** —— 每个请求由 `scope::Scope` 按 `?project=<id>` 现解析出自己那份。
    if let Some(cfg) = args.config.as_deref().map(Path::to_path_buf).or_else(persist::last_config_file) {
        if cfg.exists() {
            // 老工程里可能还是绝对路径，启动时顺手相对化（幂等）
            if let Err(e) = yekitrim_core::config::normalize_project_paths_in(&cfg) {
                tracing::debug!("工程路径相对化跳过（{}）: {e:#}", cfg.display());
            }
            info!("启动工程: {}", cfg.display());
            persist::remember_last(&cfg);
        }
    }

    let state = Arc::new(model::AppState::with_auth(auth));

    // 清扫上次运行遗留的渲染子进程：服务器重启不会带走 `polish render` 及它的
    // ffmpeg / 底座孙进程（实测空转两小时，把后续渲染拖到跑不完）。
    let swept = render_proc::sweep_orphans(state.auth.data_dir());
    if !swept.is_empty() {
        info!("已结束 {} 个遗留的渲染子进程: {:?}", swept.len(), swept);
    }

    // 录播姬入库 worker（P3）：webhook 落库后唤醒，另有 60s 兜底扫描
    ingest_worker::start(state.clone());

    // B站下载 worker（P4）：提交后唤醒，另有 10s 队列扫描 + 僵尸回收。
    // 与上一个**并列**、互不调用（共享部分已下沉到 `yekitrim_service::ingest`）。
    download_worker::start(state.clone());

    // 任务调度器自愈：调度是事件驱动的，有个工作协程永远不返回时就没有任何事件了 ——
    // 这个 tick 保证其它任务不会因此永远卡在队列里（并把"谁挡住谁"写进日志）。
    jobs::spawn_housekeeping(state.jobs.clone());

    let router = api::router(state.clone());
    let app = Router::new()
        .nest("/api", router)
        .fallback(get(web::webui_fallback));

    let listener = match TcpListener::bind(addr).await {
        Ok(l) => l,
        Err(e) => {
            // 绑不上别只丢一句"已被占用？"（用户既不知道谁占的、也不知道怎么办）。
            // 先下 blocking 池做两件事：探测对端是不是**另一个 V-Trim**、探一个真能用的端口。
            // 关键是别瞎猜端口：`port+1` 很可能落在同一个 Windows 保留段里（实测 7788 绑不上时
            // 7789/7834 也一样 EACCES）—— 所以这里实际试绑一次再报。
            let url = format!("http://{addr}/");
            let (probe, free_port) = {
                let ip = addr.ip();
                let probe_url = url.clone();
                tokio::task::spawn_blocking(move || {
                    (
                        yekitrim_service::doctor::probe_health(probe_url.trim_end_matches('/')),
                        first_bindable_port(ip, addr.port()),
                    )
                })
                .await
                .unwrap_or((None, None))
            };
            let peer = probe.as_deref().and_then(parse_health_peer);
            let hint = bind_conflict_hint(
                &addr,
                peer.as_ref(),
                &exe_dir(),
                &data_dir,
                e.raw_os_error(),
                free_port,
            );
            // 也落日志：窗口化/双击启动时 stderr 会丢，日志里只剩半截启动 banner
            tracing::error!("{hint}");
            // `--open` + 对端就是 V-Trim：合理解释是"再开一次"，直接把已有的界面打开
            if args.open && peer.is_some() {
                info!("已有一个 V-Trim 在 {addr}，直接打开它");
                open_browser(&url);
                return Ok(());
            }
            return Err(anyhow::Error::new(e).context(hint));
        }
    };
    let actual = listener.local_addr()?;
    info!("WebUI: http://{actual}/");
    info!("API:   http://{actual}/api/health");
    if bootstrap.must_change_password {
        info!("登录账号: admin（密码见上方初始密码 / 初始密码文件）");
    }
    info!("按 Ctrl+C 退出");

    if args.open {
        let url = format!("http://{actual}{}", args.open_path);
        open_browser(&url);
    }
    if !web::dist_ready() {
        tracing::warn!(
            "⚠ 未找到 WebUI 构建产物（webui/dist/index.html），页面将返回 503。\
             请在仓库根目录执行 `cd webui && npm install && npm run build`，\
             或把 webui/dist 放到可执行文件同级目录"
        );
    }

    // ConnectInfo 让登录限流拿到真实对端 IP（不信任 X-Forwarded-For，§3.5.1）
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await?;
    Ok(())
}

pub(crate) fn open_browser(url: &str) {
    let opened = if cfg!(target_os = "windows") {
        std::process::Command::new("cmd")
            .args(["/c", "start", "", url])
            .spawn()
    } else if cfg!(target_os = "macos") {
        std::process::Command::new("open").arg(url).spawn()
    } else {
        std::process::Command::new("xdg-open").arg(url).spawn()
    };
    match opened {
        Ok(_) => info!("已打开浏览器: {url}"),
        Err(e) => info!("无法自动打开浏览器: {e}"),
    }
}

/// 本次实例的程序目录（绑定失败时要说清"你起的是哪一份" —— 两份实例各有一套 `.vtrim`）。
fn exe_dir() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        .unwrap_or_else(|| PathBuf::from("."))
}

/// 从 `/api/health` 的响应体里认出"对端是另一个 V-Trim 实例"。
#[derive(Debug, Clone, PartialEq, Eq)]
struct HealthPeer {
    version: String,
    listening: String,
}

/// 解析健康检查响应。**要求 `ok:true` + 有 `version`**：别的程序碰巧回 JSON 不该被误认成 V-Trim。
fn parse_health_peer(body: &str) -> Option<HealthPeer> {
    let v: serde_json::Value = serde_json::from_str(body.trim()).ok()?;
    if v.get("ok").and_then(|x| x.as_bool()) != Some(true) {
        return None;
    }
    Some(HealthPeer {
        version: v.get("version").and_then(|x| x.as_str())?.to_string(),
        listening: v
            .get("listening")
            .and_then(|x| x.as_str())
            .unwrap_or("?")
            .to_string(),
    })
}

/// 试出一个**真能绑**的端口（`Err` 时给用户的可执行建议，不是猜的）。
///
/// ⚠️ 为什么不直接用 `port + 1`：Windows 的保留段是**一整段**，实测 7788 绑不上时
/// 7789、7834 也一样 `EACCES`（保留段 7735–7834），所以必须实测。
///
/// 候选按「先跨平台安全带内、再其余」排序（安全带 = 15001–32767，见
/// `yekitrim_service::doctor::DEFAULT_PORT`），免得给用户推荐一个刚从坑里爬出来的段。
fn first_bindable_port(ip: std::net::IpAddr, port: u16) -> Option<u16> {
    const SAFE: std::ops::RangeInclusive<u16> = 15001..=32767;
    let mut candidates: Vec<u16> = vec![18088, 28088, 38088];
    if let Some(p) = port.checked_add(10000) {
        candidates.push(p);
    }
    candidates.sort_by_key(|p| (!SAFE.contains(p), *p));
    for p in candidates {
        if p == port {
            continue;
        }
        if std::net::TcpListener::bind((ip, p)).is_ok() {
            return Some(p); // 立刻 drop → 端口释放；只是给用户一个已验证的建议
        }
    }
    None
}

/// 绑端口失败时给用户看的话（纯函数，便于单测）。
///
/// 三种情形分开说：**对端是 V-Trim**（别起了，打开它 / 换端口）、**系统拒绝**（10013 =
/// WSAEACCES，多半是落在 Windows 保留端口段里，netstat 看不到监听者）、**真被占用**（10048）。
fn bind_conflict_hint(
    addr: &SocketAddr,
    peer: Option<&HealthPeer>,
    exe_dir: &Path,
    data_dir: &Path,
    os_code: Option<i32>,
    free_port: Option<u16>,
) -> String {
    let me = format!(
        "本次实例：{} ｜ 数据目录：{}",
        exe_dir.display(),
        data_dir.display()
    );
    let port_cmd = match free_port {
        Some(p) => format!("--port {p}（刚实测可绑定）"),
        None => "--port <挑一个不在保留段里的>".to_string(),
    };
    if let Some(p) = peer {
        return format!(
            "端口 {addr} 上已经有一个 V-Trim 在跑（版本 {}，listening={}）—— 不用再起一个，\
             直接用浏览器打开 http://{addr}/ 即可。\n\
             确实要再跑一个实例就换端口：{port_cmd}（两份实例的 .vtrim 数据目录各自独立，互不相干）。\n{me}",
            p.version, p.listening
        );
    }
    // 10013/WSAEACCES 与 10048/WSAEADDRINUSE 是两码事，处置完全不同
    let p = addr.port();
    let (why, first_step) = match os_code {
        Some(10013) => (
            "被系统**拒绝**了（10013 = WSAEACCES）—— 不是被某个程序占用。\
             最常见的原因是它落在了 **Windows 保留端口段**里（Docker Desktop / WSL2 / Hyper-V 启动时会保留一段），\
             这种情况下 netstat 看不到任何监听者",
            "先确认是否命中保留段（命中就直接换端口，别去 netstat 找进程）：\n\
             \x20  netsh interface ipv4 show excludedportrange protocol=tcp\n\
             \x20  临时解法（会重置保留段，重启后 Docker/WSL 可能又保留新段）：net stop winnat && net start winnat"
                .to_string(),
        ),
        Some(10048) => (
            "确实被别的程序占用了（10048 = WSAEADDRINUSE）",
            format!(
                "找占用者：\n\
                 \x20  netstat -ano | findstr :{p}   ← 拿到 PID\n\
                 \x20  tasklist /FI \"PID eq <PID>\"      ← 看是谁"
            ),
        ),
        _ => (
            "绑不上（既没认出对端是 V-Trim，也不像常见的端口冲突）",
            format!(
                "两种常见成因，依次排除：\n\
                 \x20  1) 被别的程序占用：netstat -ano | findstr :{p} → tasklist /FI \"PID eq <PID>\"\n\
                 \x20  2) 落在 Windows 保留端口段（netstat 看不到监听者）：\n\
                 \x20     netsh interface ipv4 show excludedportrange protocol=tcp"
            ),
        ),
    };
    format!("端口 {addr} {why}。\n{first_step}\n最省事的解法：换端口起：{port_cmd}\n{me}")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 初始密码护栏的判据：**是否对外暴露**，不是"是否非 loopback"。
    #[test]
    fn initial_password_is_refused_whenever_exposed() {
        // 公网直连（非 loopback、没给 --allow-remote，上面那条已经先报错了）
        assert!(must_refuse_initial_password(false, false, true));
        // 非 loopback + --allow-remote
        assert!(must_refuse_initial_password(false, true, true));
        // ⚠️ 反代姿势：host 是 loopback 但显式 --allow-remote —— 旧判据放行，文档却承诺拒绝
        assert!(must_refuse_initial_password(true, true, true));
        // 纯本机自用：不拦（否则开发机第一次启动就没法用）
        assert!(!must_refuse_initial_password(true, false, true));
        // 改过密码：一律放行
        assert!(!must_refuse_initial_password(false, true, false));
        assert!(!must_refuse_initial_password(true, true, false));
    }

    fn addr() -> SocketAddr {
        "127.0.0.1:7788".parse().unwrap()
    }

    #[test]
    fn health_peer_requires_vtrim_shape() {
        // 真 V-Trim 的响应
        let ok = parse_health_peer(r#"{"ok":true,"version":"0.1.0","listening":"loopback"}"#).unwrap();
        assert_eq!(ok.version, "0.1.0");
        assert_eq!(ok.listening, "loopback");
        // 别的程序碰巧回 JSON：不认（缺 version / ok 不为 true / 不是 JSON）
        assert!(parse_health_peer(r#"{"ok":true}"#).is_none());
        assert!(parse_health_peer(r#"{"ok":false,"version":"0.1.0"}"#).is_none());
        assert!(parse_health_peer("<html>hi</html>").is_none());
        assert!(parse_health_peer("").is_none());
        // 缺 listening 也算 V-Trim（只有它是可选展示项）
        assert!(parse_health_peer(r#"{"ok":true,"version":"1"}"#).is_some());
    }

    /// 对端是 V-Trim：别让人以为"起不来"，要给"打开它 / 换端口"。
    #[test]
    fn hint_for_existing_vtrim_says_open_it() {
        let peer = HealthPeer { version: "0.1.0".into(), listening: "loopback".into() };
        let h = bind_conflict_hint(
            &addr(),
            Some(&peer),
            Path::new("F:/app"),
            Path::new("F:/app/.vtrim/cloud"),
            Some(10048),
            Some(18088),
        );
        assert!(h.contains("已经有一个 V-Trim 在跑"), "{h}");
        assert!(h.contains("0.1.0"), "{h}");
        assert!(h.contains("http://127.0.0.1:7788/"), "{h}");
        // 给的是**实测过**的端口，不是 port+1（同段里可能一起被拒）
        assert!(h.contains("--port 18088（刚实测可绑定）"), "{h}");
        // 身份两件套：不说清楚就分不出"我起的是哪一份"
        assert!(h.contains("F:/app") && h.contains("F:/app/.vtrim/cloud"), "{h}");
        // 这条分支不该出现排查命令（会误导）
        assert!(!h.contains("netstat"), "{h}");
    }

    /// 10013（系统拒绝）= 多半落在 Windows 保留端口段：**别让用户去 netstat 找进程**。
    ///
    /// 实测坐实过：7788 绑不上时报的就是 10013（EACCES），而 netstat 里**没有任何监听者**
    /// —— 因为它落在保留段 7735–7834 里（连 7789/7834 也一样）。
    #[test]
    fn hint_for_eacces_points_at_reserved_port_range() {
        let h = bind_conflict_hint(
            &addr(),
            None,
            Path::new("F:/app"),
            Path::new("F:/app/.vtrim/cloud"),
            Some(10013),
            Some(18088),
        );
        assert!(h.contains("10013"), "{h}");
        assert!(h.contains("保留端口段"), "{h}");
        assert!(h.contains("excludedportrange"), "{h}");
        assert!(h.contains("net stop winnat"), "{h}");
        assert!(h.contains("--port 18088"), "{h}");
        assert!(!h.contains("已经有一个 V-Trim"), "{h}");
    }

    /// 10048（真被占用）：给 netstat + tasklist。
    #[test]
    fn hint_for_addrinuse_points_at_holder() {
        let h = bind_conflict_hint(&addr(), None, Path::new("F:/app"), Path::new("F:/d"), Some(10048), None);
        assert!(h.contains("10048"), "{h}");
        assert!(h.contains("netstat -ano | findstr :7788"), "{h}");
        assert!(h.contains("tasklist /FI"), "{h}");
        // 没探到可用端口时不要瞎给一个具体数字
        assert!(h.contains("--port <挑一个不在保留段里的>"), "{h}");
        assert!(h.contains("F:/d"), "{h}");
    }

    /// 错误码认不出来时：两种成因一起给（别只赌一种）。
    #[test]
    fn hint_without_known_code_lists_both_causes() {
        let h = bind_conflict_hint(&addr(), None, Path::new("F:/app"), Path::new("F:/d"), None, None);
        assert!(h.contains("netstat -ano | findstr :7788"), "{h}");
        assert!(h.contains("excludedportrange"), "{h}");
    }
}
