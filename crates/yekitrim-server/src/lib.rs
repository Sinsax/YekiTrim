//! Server 支持：axum HTTP + WebUI + JobManager。
//!
//! 独立 crate（原 `yekitrim-cli/src/server/`）。分层是**单向**的：
//!
//! ```text
//! yekitrim-core ─┐
//! yekitrim-polish ┼─→ yekitrim-service ──→ yekitrim-server ──→ yekitrim-cli（只剩入口）
//!             └──────────────────────────────────────┘
//! ```
//!
//! - `yekitrim server` 子命令启动
//! - REST API 覆盖项目/状态/配置/数据/管线/精修
//! - SSE 推送任务日志与进度
//! - **账号与会话**：启动即引导 admin 账号；除 `/api/health`、`/api/auth/login`
//!   外的全部接口都要求会话（含 loopback，无免登录分支，见 `auth.rs`）
//! - WebUI = `webui/dist`（Vue3 构建产物）；**不再内嵌单文件页面**，
//!   缺失时由 `web::webui_fallback` 返回 503 + 构建指引
//!
//! **不变式：本 crate 只依赖 `yekitrim-service`，绝不反向。** 服务端要用的领域逻辑
//! 一律通过 `yekitrim-service` 暴露（`yekitrim-service` 也因此不能 `use yekitrim_server::*`）。
//!
//! 设计文档: docs/superpowers/specs/2026-09-03-yekitrim-webui-design.md
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
pub mod publish;
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

/// `yekitrim server` CLI 参数（由 main.rs 转发）。
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

    /// 指定 WebUI 静态资源目录（**覆盖**默认来源）。
    ///
    /// 默认来源按构建形态定：**release = 内嵌**（`build.rs` 把 `webui/dist` 编进二进制，
    /// 于是"只换 exe 就能升级"）、**debug = 读盘**（改前端不用重链接服务端）。
    /// 这个开关给"生产上热修一份界面"与排障用；等价的环境变量是 `YEKITRIM_WEBUI_DIR`
    /// （`web::WEBUI_DIR_ENV`，命令行参数优先）。
    #[arg(long)]
    pub webui_dir: Option<PathBuf>,

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

    /// 本地免登录（**仅 loopback**）：不校验会话，所有请求都当作本机管理员。
    ///
    /// 这是给"双击就用的桌面形态"（`yekitrim-tray`）用的 —— 省掉登录这步，
    /// 代价是**同机上任何进程**都能调这套 API（它本来就是个本机工具，不是多租户服务）。
    /// 因此只允许绑 loopback，且另加两道闸：
    ///   · `Host` 必须是 loopback 主机名 —— 防 DNS rebinding（恶意网页把域名解析到 127.0.0.1）
    ///   · 写请求的 `Origin` / `Sec-Fetch-Site` 必须是同源 —— 防跨站表单把请求打进来
    /// ⚠️ 对外服务（`--allow-remote` / 非 loopback）一律拒绝启动，别拿它当"免密后台"。
    #[arg(long)]
    pub local_no_auth: bool,

    /// 强制接管 yekitrim.lock（仅在确认上一个实例已崩溃残留时使用）
    #[arg(long)]
    pub force: bool,

    /// 打开浏览器时使用的初始路径（例如 hash 路由；CLI 内部使用）
    #[arg(long, hide = true, default_value = "/")]
    pub open_path: String,
}

/// 旧版项目列表（`.yekitrim/server.toml` 的 `projects[]`）一次性导入 admin 名下。
///
/// P2 起**归属由 DB 决定**；老用户升级后原来的项目不能凭空消失，因此在这里只读导入：
/// 已登记过的路径会跳过（幂等），缺失的条目忽略。
/// 把**单人时期**的存量（素材 + 程序配置）归入**首个管理员**名下。
///
/// 判据是"`users` 里 id 最小的那个 admin"。为什么是**首个管理员**而不是"当前登录的人"：
/// 启动期没有会话；而单人时期的那些东西，事实上就是这台机器主人的 ——
/// 首个管理员正是当年 `create_user("admin", …)` 建的那个账号。
///
/// ⚠️ 没有管理员（全新安装 / 还没初始化）⇒ 直接跳过，**不建目录、不动任何文件**。
/// ⚠️ 幂等与"搬运而非复制"的语义全在 `yekitrim_core::owner::migrate_legacy_assets_to`，
/// 这里只负责选出那个 uid。**别在这里自己写 rm/mv** —— 那样会绕过它已经钉住的判据。
fn migrate_solo_assets_to_first_admin(db: &db::Db) -> anyhow::Result<Vec<std::path::PathBuf>> {
    let users = db.list_users()?;
    let Some(first_admin) = users
        .iter()
        .filter(|u| u.role == "admin")
        .min_by_key(|u| u.id)
    else {
        return Ok(Vec::new());
    };
    Ok(yekitrim_core::owner::migrate_legacy_assets_to(
        yekitrim_core::owner::Owner::User(first_admin.id),
        &yekitrim_core::path::data_dir(),
    )?)
}

/// 告诉 `owner` 层「`Local` 是谁」。
///
/// ## 为什么需要这一步
///
/// `Owner::Local` 代表**当前管理员**（托盘 / `--local-no-auth` / CLI 都是"这台机器的主人"），
/// 所以它必须解析到 `users/<admin>/` —— 与"admin 在 WebUI 里登录"**同一处**。
/// 但 core **不认识账本**（它连 `yekitrim_service` 都不许 `use`），所以"当前管理员是谁"
/// 只能由上层查出来注入一次。
///
/// ⚠️ **必须在任何按 `Local` 取路径的操作之前调**（本函数在启动流程里紧跟建表、早于
/// 所有 handler 与任务）。顺序反了不会报错，只会让那几次解析落到 `users/1/`
/// —— 恰好等于默认值，所以**只在"首个管理员不是 1"时才看得出来**。
/// 那种库是可能的（1 号账号被删过），所以别省这一步。
fn install_local_admin(db: &db::Db) -> anyhow::Result<Option<i64>> {
    let admin = db
        .list_users()?
        .iter()
        .filter(|u| u.role == "admin")
        .min_by_key(|u| u.id)
        .map(|u| u.id);
    if let Some(uid) = admin {
        yekitrim_core::owner::set_local_admin(uid);
    }
    Ok(admin)
}

fn import_legacy_projects(db: &db::Db) -> anyhow::Result<usize> {    let Some(admin) = db.find_user("admin")? else {
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
    let probe = dir.join(".yekitrim-write-probe");
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

/// 「初始密码怎么改」的可操作提示 —— 拒绝启动与启动警告**共用这一份文案**。
///
/// ⚠️ 第一行命令里的 `YEKITRIM_DATA_DIR=<账本目录>` **不能省**：`yekitrim user passwd` 是
/// **另一个进程**，它只认环境变量、不认 `--data-dir`（见 `db::data_dir` 的注释）。少了它，用户
/// 照着提示改的是 exe 旁边那份账本，服务器上这份仍是初始密码 —— 命令行一切正常、重启照样被拒。
/// 这正是"我明明改了密码还是起不来"的报障形态，所以它同时是 `password_change_hint_names_ledger`
/// 那条单测的判据。
fn password_change_hint(ledger_dir: &Path, initial_pw_file: &Path) -> String {
    let dir = ledger_dir.display();
    let mut s = String::from("改掉它再启动，任选一条：\n");
    s.push_str("  ① 账本里已有 admin —— 先改密码，再用同样的参数启动：\n");
    s.push_str(&format!(
        "       YEKITRIM_DATA_DIR={dir} yekitrim user passwd admin --password-file <密码文件>\n"
    ));
    s.push_str(&format!(
        "       Windows: set YEKITRIM_DATA_DIR={dir} && yekitrim user passwd admin --password-file <密码文件>\n"
    ));
    s.push_str("     ⚠️ 那个变量必须与启动时的 --data-dir 同值：改密码是另一个进程，它不认 --data-dir。\n");
    s.push_str("  ② 全新部署（账本里还没有账号）—— 启动时直接给，不走「先改密」：\n");
    s.push_str("       yekitrim server <其余参数> --admin-password-file <密码文件>\n");
    s.push_str("  改完用 `yekitrim user list` 确认 admin 那行后面不再标「(初始密码)」。\n");
    s.push_str(&format!(
        "  口令至少 {} 字符；忘了密码：YEKITRIM_DATA_DIR={dir} yekitrim user reset-admin-password --password-file <密码文件>\n",
        yekitrim_service::auth::MIN_PASSWORD_LEN
    ));
    s.push_str(&format!(
        "  初始密码文件（改完密码自动删除）：{}",
        initial_pw_file.display()
    ));
    s
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

/// 启动 HTTP server（阻塞直至监听关闭或收到关停信号）。
///
/// ⚠️ 走的是**优雅关停**：Ctrl+C 不再让进程被直接终止 —— 那样 `InstanceLock` 的 Drop 不跑，
/// 下次启动会多一条"残留锁"WARN（能自愈，但没必要留噪音）。
/// 托盘的"退出"就是靠给 `run_with` 传一个 future 实现的。
pub async fn run(args: ServerArgs) -> anyhow::Result<()> {
    run_with(args, async {
        // Ctrl+C 拿不到就算了（headless/服务场景下可能没有控制台）
        let _ = tokio::signal::ctrl_c().await;
    }, |_| {}).await
}

/// 进程级关停触发器 —— 给 **HTTP 控制端点**（`POST /api/shutdown`）用。
///
/// ⚠️ 存的是**当前这次运行**的 `Notify`（`run_with` 进来装、退出摘），不是 `OnceLock`：
/// 进程里连着起两次服务（测试/复用进程）时，上一次留下的 permit 会把下一次**立刻**关停。
static SHUTDOWN_TRIGGER: std::sync::Mutex<Option<std::sync::Arc<tokio::sync::Notify>>> =
    std::sync::Mutex::new(None);

/// 请求关停当前正在跑的服务；返回 `false` = 现在没有在跑的服务。
///
/// 效果**等价于 Ctrl+C**：一样走优雅关停（限时 `SHUTDOWN_GRACE`）、一样让 `InstanceLock`
/// 的 Drop 把锁写成 `pid=0`。跨进程的"托盘退出"就是靠它 —— 托盘那个 exe 不链接本 crate，
/// 只能通过 HTTP 敲这一下。
pub fn request_shutdown() -> bool {
    match SHUTDOWN_TRIGGER.lock().unwrap().as_ref() {
        Some(n) => {
            // notify_one 会**存一个 permit**：万一此刻还没有等待者（信号早于 select），也不丢
            n.notify_one();
            true
        }
        None => false,
    }
}

/// `run` 的可控版本：`shutdown` 一旦就绪就优雅关停；`on_listening` 在监听就绪后**回调一次**
/// （托盘靠它拿到**实际**监听地址 —— 端口被占时服务端会自己换一个，不能假定就是请求的那个）。
pub async fn run_with<S, F>(args: ServerArgs, shutdown: S, on_listening: F) -> anyhow::Result<()>
where
    S: std::future::Future<Output = ()> + Send + 'static,
    F: FnOnce(&str) + Send + 'static,
{
    // WebUI 来源要在**任何静态资源请求之前**定下来（也就早于下面那句启动日志）
    web::set_webui_dir(args.webui_dir.clone());
    let addr: SocketAddr = format!("{}:{}", args.host, args.port)
        .parse()
        .with_context(|| format!("无法解析监听地址: {}:{}", args.host, args.port))?;
    let is_loopback = match addr.ip() {
        std::net::IpAddr::V4(v4) => v4.is_loopback(),
        std::net::IpAddr::V6(v6) => v6.is_loopback(),
    };
    let listening = if is_loopback { Listening::Loopback } else { Listening::Remote };
    // 本地免登录的两条不变量 —— 缺一条就退化成"谁都能免密进来的后台"。
    if args.local_no_auth {
        if !is_loopback {
            anyhow::bail!(
                "--local-no-auth 只允许监听 loopback（当前 {addr}）。对外服务请去掉它、用账号登录。"
            );
        }
        if args.allow_remote {
            anyhow::bail!("--local-no-auth 与 --allow-remote 互斥（免登录不能对外）。");
        }
    }
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
        unsafe { std::env::set_var(yekitrim_core::path::DATA_DIR_ENV, &data_dir) };
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
    // 初始密码文件路径只算一次：警告与拒绝启动两条文案都要提它（指向"别再照它输"的那份）。
    let pw_file = yekitrim_service::initial_password_path();
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
            // ⚠️ 这里是"对外部署之前"唯一的提示位：本地起服务时看到它，就该知道要改密。
            // 文案与拒绝启动那条共用（含 `YEKITRIM_DATA_DIR` —— 改密是另一个进程）。
            tracing::warn!(
                "⚠ 管理员仍在使用初始密码 —— 一旦对外监听（--allow-remote / 非 loopback）服务会**拒绝启动**。\n{}",
                password_change_hint(&data_dir, &pw_file)
            );
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
            "拒绝在 {addr} 上对外监听：管理员仍在使用初始密码（初始密码会写进启动日志与 {}，等同把凭证摊在服务器上）。\n{}",
            pw_file.display(),
            password_change_hint(&data_dir, &pw_file)
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

    // ---- 单人存量归入首个管理员（2026-10-10）----
    //
    // 单人时期的素材与配置住在 `<数据>/resources/` 与 `<数据>/.yekitrim/`，
    // 而多用户形态下 `Owner::User` 解析到 `users/<uid>/…` —— 不做这一步，
    // 已用很久的机器一开多用户就是**空素材库 + 默认配置**，看着像"东西全丢了"。
    //
    // ⚠️ **只对 id 最小的管理员做一次**、且**只在目标还不存在时**搬（幂等，见
    // `owner::migrate_legacy_assets_to`）。搬运是 `rename`（不是复制 —— 素材可能几个 GB）；
    // 跨盘失败时**如实报错并保持原状**，绝不悄悄退化成"复制一半"。
    // ⚠️ 失败**不影响启动**：这是数据搬迁，搬不动就该让用户看到原因后自己处理。
    //
    // ⚠️ **顺序**：先 `install_local_admin` 再迁移。迁移本身用的是 `Owner::User(uid)`
    // （不依赖这一步），但**紧跟其后的** `ensure_global_config` 等初始化按 `Local` 取路径 ——
    // 顺序反了那些东西会落到 `users/1/`（默认值，通常恰好对，所以只在"首个管理员不是 1"时才露馅）。
    let admin_uid = match install_local_admin(auth.db()) {
        Ok(uid) => uid,
        Err(e) => {
            tracing::warn!("解析「当前管理员是谁」失败（按默认 uid=1 继续）: {e:#}");
            None
        }
    };
    match migrate_solo_assets_to_first_admin(auth.db()) {
        Ok(ref moved) if moved.is_empty() => {}
        Ok(moved) => info!(
            "已把单人时期的 {} 项存量归入首个管理员名下（素材/配置现在按账号分开）",
            moved.len()
        ),
        Err(e) => tracing::warn!("归入首个管理员的存量失败（不影响启动，数据保持原状）: {e:#}"),
    }

    // ---- 程序配置收敛（2026-11-13，方案 A）----
    //
    // 顶层 `<数据>/.yekitrim/{config,llm}.toml` **退役**：配置只有 `users/<uid>/.yekitrim/` 一份。
    // 上面那次素材迁移只在"目标还不存在"时 rename，所以**两份都存在**（本机就是）这种情况它不动 ——
    // 那正是"接入选顶层、设置页写账号树、各读一半"的来源。这里做一次**合并式**迁移：
    // 平台段（ingest/storage/publish/auth）以顶层为准、内容段以账号为准，搬完把顶层文件改名留档。
    // 幂等：顶层文件一旦搬走就不再触发；失败只 warn（迁移不动数据，启动必须照常）。
    if let Some(uid) = admin_uid {
        match yekitrim_core::config::migrate_top_level_config_to(
            yekitrim_core::owner::Owner::User(uid),
            &yekitrim_core::path::data_dir(),
        ) {
            Ok(ref moved) if moved.is_empty() => {}
            Ok(moved) => info!(
                "程序配置已收敛到 users/{uid}/：{} 份顶层旧文件已合并并留档（{}）",
                moved.len(),
                moved.iter().map(|p| p.file_name().unwrap_or_default().to_string_lossy().into_owned()).collect::<Vec<_>>().join(", ")
            ),
            Err(e) => tracing::warn!("顶层程序配置合并失败（不影响启动，原文件保持原状）: {e:#}"),
        }
    }

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

    // 本地免登录：开机就把"本机管理员"定下来，之后每个请求直接注入它（见 auth::authenticate）。
    let local_admin = if args.local_no_auth {
        let admin = auth
            .db()
            .list_users()?
            .into_iter()
            .find(|u| u.role == "admin" && !u.disabled)
            .ok_or_else(|| anyhow::anyhow!(
                "本地免登录需要一个启用中的管理员账号，但账号账本里没有"
            ))?;
        info!("本地免登录已开启：所有请求按管理员 {} 处理（仅 loopback）", admin.username);
        Some(model::LocalAdmin { id: admin.id, username: admin.username, role: admin.role })
    } else {
        None
    };
    let state = Arc::new(match local_admin {
        Some(la) => model::AppState::with_auth(auth).with_local_admin(la),
        None => model::AppState::with_auth(auth),
    });

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

    // 保留期与每日维护（T6）：过期会话 + audit / llm_usage / 按天日志，启动 10 分钟后首跳。
    // ⚠️ **别挂到上面那个 `spawn_housekeeping`**：它是 5s 粒度的调度自愈，
    // 拿它当"每日"用等于每 5 秒考虑一次删数据。
    let first_maintenance = maintenance_first_delay();
    info!(
        "每日维护：启动后 {first_maintenance:?} 首跳、之后每 {MAINTENANCE_PERIOD:?} 一次（保留期见 [storage]）"
    );
    spawn_daily_maintenance(state.clone(), first_maintenance, MAINTENANCE_PERIOD);

    let router = api::router(state.clone());
    let app = Router::new()
        .nest("/api", router)
        .fallback(get(web::webui_fallback));

    let listener = match TcpListener::bind(addr).await {
        Ok(l) => l,
        Err(e) => {
            // 绑不上别只丢一句"已被占用？"（用户既不知道谁占的、也不知道怎么办）。
            // 先下 blocking 池做两件事：探测对端是不是**另一个 YekiTrim**、探一个真能用的端口。
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
            // `--open` + 对端就是 YekiTrim：合理解释是"再开一次"，直接把已有的界面打开
            if args.open && peer.is_some() {
                info!("已有一个 YekiTrim 在 {addr}，直接打开它");
                open_browser(&url);
                return Ok(());
            }
            return Err(anyhow::Error::new(e).context(hint));
        }
    };
    let actual = listener.local_addr()?;
    info!("WebUI: http://{actual}/");
    info!("API:   http://{actual}/api/health");
    // 先告诉调用方**实际**地址（端口可能被换过），再决定要不要自动开浏览器
    on_listening(&format!("http://{actual}"));
    if bootstrap.must_change_password {
        info!("登录账号: admin（密码见上方初始密码 / 初始密码文件）");
    }
    // T24：SIGTERM（systemd / `systemctl --user stop`）也走同一条优雅关停 —— 别再只写「按 Ctrl+C」。
    info!("按 Ctrl+C（或 systemctl stop / SIGTERM）退出");

    if args.open {
        let url = format!("http://{actual}{}", args.open_path);
        open_browser(&url);
    }
    // WebUI 来源：release 内嵌、debug 读盘、显式覆盖优先。
    // ⚠️ 这行**别以 `WebUI: http://` 开头** —— 薄启动器就是靠那个前缀抓实际监听地址的
    // （跨进程契约，见 auth.rs 的 tray_and_server_agree_on_the_cross_process_contract）。
    if web::dist_ready() {
        tracing::info!("静态资源：{}", web::source_label());
    } else {
        tracing::warn!(
            "⚠ WebUI 静态资源不可用（{}），页面将返回 503。release 二进制本该内嵌一份 —— \
             看到这句通常说明 `--webui-dir` / `{}` 指到了一个空目录；\
             开发时请 `npm --prefix webui run build`，或直接 `npm run dev`（那条路不经服务端）",
            web::source_label(),
            web::WEBUI_DIR_ENV
        );
    }

    // ConnectInfo 让登录限流拿到真实对端 IP（不信任 X-Forwarded-For，§3.5.1）
    //
    // ⚠️ **优雅关停必须有时限**：`axum` 的 `with_graceful_shutdown` 会等**所有连接**收尾，
    // 而 WebUI 有一条**长期挂着的 SSE**（`/api/stream`，任务事件流）—— 只要页面开着就一定有一条，
    // 于是"退出"会**永远等不到返回**（实测：托盘点退出后图标还在，日志里连一条关停记录都没有）。
    // 所以收到关停信号后再给 `SHUTDOWN_GRACE`，到点就不等了：监听停掉、锁照样正常释放。
    // 这条对 CLI 的 Ctrl+C 同样成立（开着页面的 `yekitrim server` 按 Ctrl+C 也会卡住）。
    let signalled = std::sync::Arc::new(tokio::sync::Notify::new());
    let signalled2 = std::sync::Arc::clone(&signalled);
    // 两个关停源：调用方给的 future（托盘的 oneshot / Ctrl+C）+ 进程级触发器（HTTP 控制端点）
    let trigger = std::sync::Arc::new(tokio::sync::Notify::new());
    *SHUTDOWN_TRIGGER.lock().unwrap() = Some(std::sync::Arc::clone(&trigger));
    let shutdown = async move {
        tokio::select! {
            _ = shutdown => {}
            _ = trigger.notified() => {
                tracing::info!("收到控制端点（POST /api/shutdown）的关停请求");
            }
        }
        // 用 notify_one（会存一个 permit）：万一信号在 select 之前就到了，也不会丢
        signalled2.notify_one();
    };
    let serve = axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown);
    let served = tokio::select! {
        r = serve => r,
        _ = async {
            signalled.notified().await;
            tokio::time::sleep(SHUTDOWN_GRACE).await;
        } => {
            tracing::warn!(
                "关停时限 {:?} 到，仍有连接未收尾（多半是页面的 SSE 事件流），直接停止监听",
                SHUTDOWN_GRACE
            );
            Ok(())
        }
    };
    // 摘掉触发器：这个进程可能还会再跑一次服务（测试/复用），别让本次的 permit 漏过去
    *SHUTDOWN_TRIGGER.lock().unwrap() = None;
    served?;
    Ok(())
}

/// 收到关停信号后，最多再等这么久让在飞的请求收尾（见 `run_with` 里的说明）。
const SHUTDOWN_GRACE: std::time::Duration = std::time::Duration::from_secs(3);

// ============================================================================
// 保留期与每日维护（remediation P0 T6）
// ============================================================================
//
// 三处数据曾经**都没有保留期**：`purge_expired_sessions` 只在启动时跑一次（它自己的
// 注释却写着"启动 + 每日一次的后台任务用"，那个任务不存在），`audit` / `llm_usage`
// 两张表无清理，`<data_dir>/logs/yekitrim-YYYYMMDD.log` 一天一份、永不删除。这里补上那台"每日"。

/// 每日维护的**生产**首次延迟：启动后 10 分钟。
///
/// 为什么不立刻跑：启动期本来就有一串一次性工作（建表、旧配置迁移、项目占用刷新、孤儿清扫）。
/// 维护要**删数据**，晚 10 分钟既不耽误"今天生效"，也不会和启动抢磁盘。
const MAINTENANCE_FIRST_DELAY: std::time::Duration = std::time::Duration::from_secs(10 * 60);
/// 每日维护周期：24 小时。
const MAINTENANCE_PERIOD: std::time::Duration = std::time::Duration::from_secs(24 * 60 * 60);

/// 环境变量 `YEKITRIM_MAINTENANCE_FIRST_DELAY_SECS` = **只给人肉验收用**的缩短开关（秒）。
///
/// 判据是"启动后日志里出现 `daily maintenance: …`"，而生产值要等 10 分钟 —— 于是这条
/// 定时器最容易被"看着像实现了"糊过去。给一个显式开关：设成 15 起服务，15 秒后就能看到那行；
/// **不设时永远是生产值**，改不到用户的日常行为。
fn maintenance_first_delay() -> std::time::Duration {
    std::env::var("YEKITRIM_MAINTENANCE_FIRST_DELAY_SECS")
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .map(std::time::Duration::from_secs)
        .unwrap_or(MAINTENANCE_FIRST_DELAY)
}

/// 三处数据各自的保留天数（来自 `[storage]` 的
/// `audit_keep_days` / `usage_keep_days` / `log_keep_days`）。
///
/// 抽成显式参数而不是让清理函数自己读配置：这样"读配置"与"删数据"分开，单测能给定值跑，
/// 不必依赖本机真实 config.toml。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct RetentionDays {
    audit: u32,
    usage: u32,
    logs: u32,
}

impl RetentionDays {
    /// 读**当前管理员**的 `config.toml`（与设置页 / `/api/config` 的 `[storage]` 同一份）。
    ///
    /// 服务端进程里 `Owner::Local` 就是启动时 `install_local_admin` 定的那个管理员
    /// （`owner::set_local_admin`），所以这里读到的是"这台机器的配置"。
    fn from_config() -> Self {
        let s = yekitrim_core::config::load_global_config().storage;
        Self { audit: s.audit_keep_days, usage: s.usage_keep_days, logs: s.log_keep_days }
    }
}

/// 一次维护删了什么（正好是那条 info 的四个数）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct Purged {
    sessions: usize,
    audit: usize,
    usage: usize,
    logs: usize,
}

/// 四项清理本体（**同步**：SQLite 与目录删除都是阻塞 IO，必须由调用方放到 blocking 线程）。
///
/// ⚠️ 每个 purge 自己保证"只删超过保留期的"、`0` = 关闭（见 `yekitrim_service::db`）。
/// 单项失败只 warn 并算 0 —— 维护任务不该因为一项坏掉就整轮不跑。
fn purge_now(auth: &auth::AuthService, log_dir: &Path, keep: RetentionDays) -> Purged {
    let count = |what: &str, r: anyhow::Result<usize>| match r {
        Ok(n) => n,
        Err(e) => {
            tracing::warn!("每日维护：清理{what}失败（其余项照跑）: {e:#}");
            0
        }
    };
    Purged {
        sessions: auth.purge_expired(),
        audit: count("过期审计", auth.db().purge_old_audit(keep.audit)),
        usage: count("过期用量", auth.db().purge_old_llm_usage(keep.usage)),
        logs: logbus::prune_old_logs(log_dir, keep.logs),
    }
}

/// 那条 info 的正文（纯函数：格式是**验收判据**，单测直接钉它）。
///
/// 审计表是安全用途 —— "删了多少"必须写进日志，不许静默删。
fn maintenance_line(p: Purged, keep: RetentionDays) -> String {
    format!(
        "daily maintenance: sessions={} audit={} usage={} logs={}（保留 audit/usage/log = {}/{}/{} 天，0=该项关闭）",
        p.sessions, p.audit, p.usage, p.logs, keep.audit, keep.usage, keep.logs
    )
}

/// 跑一次每日维护：读保留期 → 四项清理（blocking 线程）→ 一条 info 说明删了多少。
async fn run_daily_maintenance(state: &Arc<model::AppState>) -> Purged {
    let auth = Arc::clone(&state.auth);
    // 日志目录与 `logbus::enable_file_log()` 是同一个：两者都从数据目录派生
    // （server 启动时 `--data-dir` 已经写进环境变量，`db::data_dir()` == 这里这个）。
    // 走 `auth.data_dir()` 而不是再调一次 `enable_file_log()`，是为了让单测能指到临时目录。
    let log_dir = state.auth.data_dir().join("logs");
    let done = tokio::task::spawn_blocking(move || {
        let keep = RetentionDays::from_config();
        (purge_now(&auth, &log_dir, keep), keep)
    })
    .await;
    let Ok((purged, keep)) = done else {
        tracing::warn!("每日维护未能执行（blocking 线程异常），本轮跳过");
        return Purged::default();
    };
    tracing::info!("{}", maintenance_line(purged, keep));
    purged
}

/// 定时骨架：**首跳在 `first_delay` 之后**，此后每 `period` 一跳，每跳跑一次 `task`。
///
/// 单独抽出来是为了能测：间隔是参数 ⇒ 单测传 `(10ms, 20ms)` + `tokio::time::pause()`
/// 就能证明**跳数与顺序**（首跳 ≠ 立刻、也 ≠ 一个 period）。别写"真等 10 分钟"的用例 ——
/// 那种要么被人删掉，要么被人偷偷调成 1ms 之后失去意义。
async fn run_periodic<F, Fut>(
    first_delay: std::time::Duration,
    period: std::time::Duration,
    mut task: F,
) where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = ()>,
{
    // `interval_at` = "第一次在 start 时刻触发，之后每 period"。比"先 sleep 再 interval"
    // 少一个状态（那两段之间的时间基准不同，很容易差出半个 period）。
    // `period` 为 0 时 `interval` 会 panic —— 给 1ms 兜底（生产是常量 24h）。
    let mut ticker = tokio::time::interval_at(
        tokio::time::Instant::now() + first_delay,
        period.max(std::time::Duration::from_millis(1)),
    );
    loop {
        ticker.tick().await;
        task().await;
    }
}

/// 起每日维护后台任务。生产传 `(MAINTENANCE_FIRST_DELAY, MAINTENANCE_PERIOD)` = (10min, 24h)。
///
/// ⚠️ **不要挂到 `jobs::spawn_housekeeping`**：那是 5s 粒度的调度自愈，两者差三个数量级，
/// 混在一起会让"删数据"以 5s 的频率被考虑一次。
pub fn spawn_daily_maintenance(
    state: Arc<model::AppState>,
    first_delay: std::time::Duration,
    period: std::time::Duration,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(run_periodic(first_delay, period, move || {
        let state = Arc::clone(&state);
        async move {
            run_daily_maintenance(&state).await;
        }
    }))
}

pub(crate) fn open_browser(url: &str) {
    let opened = if cfg!(target_os = "windows") {
        yekitrim_core::process::cmd("cmd")
            .args(["/c", "start", "", url])
            .spawn()
    } else if cfg!(target_os = "macos") {
        yekitrim_core::process::cmd("open").arg(url).spawn()
    } else {
        yekitrim_core::process::cmd("xdg-open").arg(url).spawn()
    };
    match opened {
        Ok(_) => info!("已打开浏览器: {url}"),
        Err(e) => info!("无法自动打开浏览器: {e}"),
    }
}

/// 本次实例的程序目录（绑定失败时要说清"你起的是哪一份" —— 两份实例各有一套 `.yekitrim`）。
fn exe_dir() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        .unwrap_or_else(|| PathBuf::from("."))
}

/// 从 `/api/health` 的响应体里认出"对端是另一个 YekiTrim 实例"。
#[derive(Debug, Clone, PartialEq, Eq)]
struct HealthPeer {
    version: String,
    listening: String,
}

/// 解析健康检查响应。**要求 `ok:true` + 有 `version`**：别的程序碰巧回 JSON 不该被误认成 YekiTrim。
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
/// 三种情形分开说：**对端是 YekiTrim**（别起了，打开它 / 换端口）、**系统拒绝**（10013 =
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
            "端口 {addr} 上已经有一个 YekiTrim 在跑（版本 {}，listening={}）—— 不用再起一个，\
             直接用浏览器打开 http://{addr}/ 即可。\n\
             确实要再跑一个实例就换端口：{port_cmd}（两份实例的 .yekitrim 数据目录各自独立，互不相干）。\n{me}",
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
            "绑不上（既没认出对端是 YekiTrim，也不像常见的端口冲突）",
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

    // ---- T6：保留期与每日维护 ----------------------------------------------

    /// 每日维护的**跳数与顺序**：间隔是参数，测试传 `(10ms, 20ms)` 配虚拟时钟证明
    /// —— 首跳在 `first_delay`（不是立刻、也不是一个 period），之后每 `period` 一跳。
    ///
    /// ⚠️ 断言的是**记录下来的虚拟时刻**，不是"真等 10 分钟"：
    /// `start_paused` 让时钟只在 `advance` 时走，所以这条用例是确定性的。
    #[tokio::test(start_paused = true)]
    async fn daily_maintenance_ticks_after_first_delay_then_every_period() {
        let ticks: Arc<std::sync::Mutex<Vec<u128>>> = Arc::new(std::sync::Mutex::new(Vec::new()));
        let start = tokio::time::Instant::now();
        let recorder = Arc::clone(&ticks);
        let handle = tokio::spawn(run_periodic(
            std::time::Duration::from_millis(10),
            std::time::Duration::from_millis(20),
            move || {
                let recorder = Arc::clone(&recorder);
                async move {
                    recorder
                        .lock()
                        .unwrap()
                        .push((tokio::time::Instant::now() - start).as_millis());
                }
            },
        ));

        // 先让它跑起来、把定时器**注册上**（否则下面第一次 advance 时它还没被 poll 过，
        // `interval_at` 的基准就成了"第一次被 poll 的时刻"，首跳会整体后移）
        settle().await;

        // 首跳之前：一跳都不许有（这条把"首跳 = first_delay"钉死，而不是"立刻"）
        tokio::time::advance(std::time::Duration::from_millis(9)).await;
        settle().await;
        assert!(ticks.lock().unwrap().is_empty(), "首跳必须等满 first_delay(10ms)");

        tokio::time::advance(std::time::Duration::from_millis(1)).await; // t=10ms
        settle().await;
        assert_eq!(*ticks.lock().unwrap(), vec![10], "首跳在 10ms");

        // 也不许"半个 period 就再来一次"（推进到差 1ms 那一刻，仍然只有首跳）
        tokio::time::advance(std::time::Duration::from_millis(19)).await; // t=29ms
        settle().await;
        assert_eq!(ticks.lock().unwrap().len(), 1, "period 没到，不许再来一跳");

        // 之后每 20ms 一跳（30 → 50 → 70），顺序单调。
        // ⚠️ 每次 advance 都要**正好落在截止时刻**上：虚拟时钟是"一次跳到目标时刻"，
        // 跳过头时任务看到的是跳过后的时刻（实测 10 → 35 那种），那就不是在验间隔了。
        for _ in 0..3 {
            tokio::time::advance(std::time::Duration::from_millis(1)).await;
            settle().await;
            tokio::time::advance(std::time::Duration::from_millis(19)).await;
            settle().await;
        }
        assert_eq!(*ticks.lock().unwrap(), vec![10, 30, 50, 70], "跳数 4、间隔 20ms、顺序单调");

        handle.abort();
    }

    /// 让刚被时钟叫醒的后台任务真的跑完一轮（虚拟时钟下 "advance 完就断言" 会抢跑）。
    async fn settle() {
        for _ in 0..3 {
            tokio::task::yield_now().await;
        }
    }

    /// 四项清理的**编排**（含"日志目录在哪"）：过期会话删、窗口外的用量删、
    /// 40 份按天日志删成 30 份，而窗口内的一条都不许动。
    #[tokio::test]
    async fn purge_now_deletes_only_expired_rows_and_old_logs() {
        let dir = tempfile::tempdir().unwrap();
        let auth = auth::AuthService::open(dir.path().to_path_buf(), Listening::Loopback).unwrap();
        let db = auth.db();
        let uid = db.create_user("admin", "h", "admin", false, 0).unwrap();
        let now = db::now_secs();
        db.create_session("sid-old", uid, "c", 1, None).unwrap();
        db.create_session("sid-new", uid, "c", now + 3600, None).unwrap();
        let mk = |ts: i64| yekitrim_llm::usage::UsageRecord {
            ts,
            label: "事件检测".into(),
            model: "m1".into(),
            host: "example.com".into(),
            attempts: 1,
            ok: true,
            ..Default::default()
        };
        db.insert_llm_usage(&mk(now - 400 * 86_400)).unwrap(); // 365 天外
        db.insert_llm_usage(&mk(now - 10 * 86_400)).unwrap(); // 窗口内

        // 40 份按天日志（今天往前一天一份）
        let log_dir = dir.path().join("logs");
        std::fs::create_dir_all(&log_dir).unwrap();
        let today = chrono::Local::now().date_naive();
        for i in 0..40 {
            let day = today.checked_sub_days(chrono::Days::new(i)).unwrap();
            std::fs::write(
                log_dir.join(format!("yekitrim-{}.log", day.format("%Y%m%d"))),
                "x",
            )
            .unwrap();
        }

        // 直接同步跑（生产由 `run_daily_maintenance` 放进 spawn_blocking）
        let purged = purge_now(&auth, &log_dir, RetentionDays { audit: 90, usage: 365, logs: 30 });
        assert_eq!(
            purged,
            Purged { sessions: 1, audit: 0, usage: 1, logs: 10 },
            "过期会话 1 / 过期用量 1 / 日志 40→30"
        );
        // 窗口内的都还在（这是删数据的改动，反向也要钉住）
        assert!(db.session("sid-new").unwrap().is_some(), "没过期的会话不许删");
        assert_eq!(
            std::fs::read_dir(&log_dir).unwrap().flatten().count(),
            30,
            "日志目录只剩 30 份"
        );
    }

    /// 那条 info 的**格式**是验收判据（"第一次上线要说明删了多少"）。
    #[test]
    fn maintenance_line_names_all_four_counts() {
        let line = maintenance_line(
            Purged { sessions: 1, audit: 2, usage: 3, logs: 4 },
            RetentionDays { audit: 90, usage: 365, logs: 30 },
        );
        assert!(line.starts_with("daily maintenance: sessions=1 audit=2 usage=3 logs=4"), "{line}");
        assert!(line.contains("90/365/30"), "保留天数也要露出来，排障才知道删的是哪一档: {line}");
    }

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

    /// 「怎么改密码」的提示必须**能照着做**：给出账本目录、改密命令、全新部署那条路。
    ///
    /// ⚠️ 少掉 `YEKITRIM_DATA_DIR` 那一项就是实测踩过的坑：`yekitrim user passwd` 是另一个进程，
    /// 它不认 `--data-dir`，于是用户改的是 exe 旁边那份账本、服务器上这份仍是初始密码 ——
    /// 命令行一切正常，重启还是被这条护栏拒掉（"我明明改了密码"）。
    #[test]
    fn password_change_hint_names_ledger_and_both_ways() {
        let hint = password_change_hint(
            Path::new("/var/lib/yekitrim"),
            Path::new("/var/lib/yekitrim/initial-admin-password.txt"),
        );
        assert!(
            hint.contains("YEKITRIM_DATA_DIR=/var/lib/yekitrim"),
            "必须指明账本目录（改密是另一个进程，不认 --data-dir）: {hint}"
        );
        assert!(hint.contains("yekitrim user passwd admin"), "必须给出改密命令: {hint}");
        assert!(hint.contains("--password-file"), "必须给出非交互形式: {hint}");
        assert!(
            hint.contains("--admin-password-file"),
            "必须给出「全新部署一次给」那条路: {hint}"
        );
        assert!(
            hint.contains("reset-admin-password"),
            "忘了密码要能自救: {hint}"
        );
        assert!(
            hint.contains("/var/lib/yekitrim/initial-admin-password.txt"),
            "要点名那份会撒谎的初始密码文件: {hint}"
        );
    }

    fn addr() -> SocketAddr {
        "127.0.0.1:7788".parse().unwrap()
    }

    #[test]
    fn health_peer_requires_yekitrim_shape() {
        // 真 YekiTrim 的响应
        let ok = parse_health_peer(r#"{"ok":true,"version":"0.1.0","listening":"loopback"}"#).unwrap();
        assert_eq!(ok.version, "0.1.0");
        assert_eq!(ok.listening, "loopback");
        // 别的程序碰巧回 JSON：不认（缺 version / ok 不为 true / 不是 JSON）
        assert!(parse_health_peer(r#"{"ok":true}"#).is_none());
        assert!(parse_health_peer(r#"{"ok":false,"version":"0.1.0"}"#).is_none());
        assert!(parse_health_peer("<html>hi</html>").is_none());
        assert!(parse_health_peer("").is_none());
        // 缺 listening 也算 YekiTrim（只有它是可选展示项）
        assert!(parse_health_peer(r#"{"ok":true,"version":"1"}"#).is_some());
    }

    /// 对端是 YekiTrim：别让人以为"起不来"，要给"打开它 / 换端口"。
    #[test]
    fn hint_for_existing_yekitrim_says_open_it() {
        let peer = HealthPeer { version: "0.1.0".into(), listening: "loopback".into() };
        let h = bind_conflict_hint(
            &addr(),
            Some(&peer),
            Path::new("F:/app"),
            Path::new("F:/app/.yekitrim/cloud"),
            Some(10048),
            Some(18088),
        );
        assert!(h.contains("已经有一个 YekiTrim 在跑"), "{h}");
        assert!(h.contains("0.1.0"), "{h}");
        assert!(h.contains("http://127.0.0.1:7788/"), "{h}");
        // 给的是**实测过**的端口，不是 port+1（同段里可能一起被拒）
        assert!(h.contains("--port 18088（刚实测可绑定）"), "{h}");
        // 身份两件套：不说清楚就分不出"我起的是哪一份"
        assert!(h.contains("F:/app") && h.contains("F:/app/.yekitrim/cloud"), "{h}");
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
            Path::new("F:/app/.yekitrim/cloud"),
            Some(10013),
            Some(18088),
        );
        assert!(h.contains("10013"), "{h}");
        assert!(h.contains("保留端口段"), "{h}");
        assert!(h.contains("excludedportrange"), "{h}");
        assert!(h.contains("net stop winnat"), "{h}");
        assert!(h.contains("--port 18088"), "{h}");
        assert!(!h.contains("已经有一个 YekiTrim"), "{h}");
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
