//! Server 支持：axum HTTP + WebUI + JobManager。
//!
//! - `vtrim server` 子命令启动
//! - REST API 覆盖项目/状态/配置/数据/管线/精修
//! - SSE 推送任务日志与进度
//! - **账号与会话**：启动即引导 admin 账号；除 `/api/health`、`/api/auth/login`
//!   外的全部接口都要求会话（含 loopback，无免登录分支，见 `server/auth.rs`）
//! - WebUI = `webui/dist`（Vue3 构建产物）；**不再内嵌单文件页面**，
//!   缺失时由 `web::webui_fallback` 返回 503 + 构建指引
//!
//! 设计文档: docs/superpowers/specs/2026-09-03-vtrim-webui-design.md
//!           docs/superpowers/specs/2026-09-12-multi-user-server-design.md（§3.5 / §10.1）

pub mod api;
pub mod auth;
pub mod jobs;
pub mod media;
pub mod model;
pub mod persist;
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

use crate::service::instancelock::InstanceLock;
use crate::service::{self, db};
use crate::service::Workspace;

use self::auth::Listening;

/// `vtrim server` CLI 参数（由 main.rs 转发）。
#[derive(Debug, Clone, Parser)]
pub struct ServerArgs {
    /// 监听地址
    #[arg(long, default_value = "127.0.0.1")]
    pub host: String,
    /// 监听端口
    #[arg(long, default_value_t = 7788)]
    pub port: u16,
    /// 启动后自动打开浏览器
    #[arg(long)]
    pub open: bool,
    /// 项目配置文件路径
    #[arg(long)]
    pub config: Option<PathBuf>,

    /// 允许监听非 loopback 地址。
    ///
    /// 服务可读写本机工程目录并调用 ffmpeg/Chrome，默认只允许 127.0.0.1 / ::1；
    /// 确需对外监听必须显式打开，且**不允许仍在使用初始密码**（§3.5.4）。
    #[arg(long)]
    pub allow_remote: bool,

    /// 账号账本 / 会话 / 初始密码文件所在目录（默认 `<exe_dir>/.vtrim/cloud`，也可用 `VTRIM_DATA_DIR`）
    #[arg(long)]
    pub data_dir: Option<PathBuf>,

    /// 无人值守部署：从这个文件读取初始管理员密码（不进进程列表）
    #[arg(long)]
    pub admin_password_file: Option<PathBuf>,

    /// 强制接管 vtrim.lock（仅在确认上一个实例已崩溃残留时使用）
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
             或用环境变量 VTRIM_DATA_DIR",
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

/// 解析显式初始密码（`--admin-password-file` > `VTRIM_ADMIN_PASSWORD`）。
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
    if let Ok(pw) = std::env::var("VTRIM_ADMIN_PASSWORD") {
        if !pw.trim().is_empty() {
            tracing::warn!(
                "⚠ 正在使用环境变量 VTRIM_ADMIN_PASSWORD 作为初始密码：它会出现在 /proc/<pid>/environ、\
                 `docker inspect` 等位置。生产部署请改用 --admin-password-file"
            );
            return Ok(Some(pw));
        }
    }
    Ok(None)
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

    // ---- 账号数据（身份不可缺省）----
    let data_dir = args.data_dir.clone().unwrap_or_else(db::data_dir);
    if args.data_dir.is_some() {
        // 统一来源：CLI 其它入口 / 初始密码文件路径都读同一个变量
        std::env::set_var("VTRIM_DATA_DIR", &data_dir);
    }
    prepare_data_dir(&data_dir)?;
    let explicit_pw = explicit_admin_password(&args)?;
    let auth = auth::AuthService::open(data_dir.clone(), listening).with_context(|| {
        format!(
            "账号账本不可用（{}）。请检查磁盘空间/权限，或用 --data-dir <可写路径> 指定别处",
            data_dir.display()
        )
    })?;
    let bootstrap = service::ensure_admin_bootstrap(explicit_pw.clone())?;
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
            tracing::warn!("⚠ 管理员仍在使用初始密码，请尽快登录并修改（或 `vtrim user passwd admin`）");
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
        if bootstrap.must_change_password {
            anyhow::bail!(
                "拒绝在 {addr} 上监听：管理员仍在使用初始密码（初始密码会出现在日志与 {} 里，等同把密码写在服务器上）。\n\
                 请先修改密码：vtrim user passwd admin（非交互：--stdin / --password-file），\
                 或在部署时用 --admin-password-file 显式设定初始密码。",
                service::initial_password_path().display()
            );
        }
        tracing::warn!("⚠ 正在监听非 loopback 地址 {addr}：请务必使用 HTTPS 反向代理，并确保账号密码足够强");
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
    let refreshed = service::projects::refresh_all_bytes(auth.db());
    info!("项目占用已刷新（{refreshed} 个项目）");

    // 首次运行（没有程序配置）时自动生成默认配置；已有配置不会被覆盖
    match crate::service::ensure_global_config() {
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

    let saved = persist::load_state();
    let workspace = if args.config.is_none() {
        // 未显式指定工程时，优先恢复上次记忆的工程
        match persist::last_config_file() {
            Some(cfg) => {
                info!("恢复上次工程: {}", cfg.display());
                Workspace::load(Some(&cfg))?
            }
            None => Workspace::load(args.config.as_deref())?,
        }
    } else {
        Workspace::load(args.config.as_deref())?
    };
    info!("Workspace: {}", workspace.config_file.display());
    if workspace.config_file.exists() {
        let _ = vtrim_core::config::normalize_project_paths_in(&workspace.config_file);
    }

    // 若工作区根未持久化或目录失效，退回工程父目录
    let workspace_root: PathBuf = match saved.as_ref().and_then(|s| s.workspace_root.clone()) {
        Some(root) if Path::new(&root).is_dir() => PathBuf::from(root),
        _ => workspace
            .root_dir
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| workspace.root_dir.clone()),
    };
    let mut server_state = persist::load_state().unwrap_or_default();
    server_state.last_config_file = Some(workspace.config_file.to_string_lossy().into_owned());
    server_state.workspace_root = Some(workspace_root.to_string_lossy().into_owned());
    let cfg_str = workspace.config_file.to_string_lossy().into_owned();
    if !server_state.projects.contains(&cfg_str) {
        server_state.projects.push(cfg_str);
    }
    persist::save_state(&server_state);
    info!("Workspace root: {}", workspace_root.display());

    let state = Arc::new(model::AppState::with_auth(workspace, workspace_root, auth));

    let router = api::router(state.clone());
    let app = Router::new()
        .nest("/api", router)
        .fallback(get(web::webui_fallback));

    let listener = TcpListener::bind(addr)
        .await
        .with_context(|| format!("端口 {} 已被占用？", addr))?;
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
