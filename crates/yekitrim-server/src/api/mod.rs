//! `api` 模块根：拆分后留下的**共享底座**。
//!
//! - `AppError` / `ApiResult` / `err` / `ok`：全部 handler 的返回约定
//! - `blocking` / `block_on_here` / `spawn_job`：同步 IO 与任务的落点（见 AGENTS「同步 IO 绝不能留在 async 线程上」）
//!
//! ⚠️ 这里曾经还有 `workspace_from_state(state)`（从 `AppState` 取那份"全局当前工程"）。
//! **2026-10-03 随全局工程一起删掉了**：项目的 `Workspace` 现在**只能**从 `scope::Scope`
//! 提取器拿（它按 `?project=<id>` 校验归属后现场解析）。要工程就接一个 `Scope`；
//! 需要"跟具体项目无关"的那几个值，直接读 `path::yekitrim_home()` / `load_global_config()`。
//!
//! 外部命令探测（原 `probe_cache` / `command_available`）已搬去 `crate::tool_probe` —— 见该模块说明。
//!
//! 这里只放**被两个以上子模块用到**的东西；只服务一个领域的 helper 跟着那个模块走。

use std::sync::Arc;

use axum::{
    extract::State,
    http::StatusCode,
    Json,
};
use crate::model::AppState;
use crate::tool_probe::command_available;

/// ffmpeg / ffprobe 这一族的版本参数（`-version`）。
///
/// ⚠️ **yt-dlp 要 `--version`**，不是这个 —— 所以 `command_available` 强制调用方显式传参。
const FFMPEG_VERSION_ARG: &str = "-version";

pub(crate) type AppError = (StatusCode, Json<serde_json::Value>);
pub(crate) type ApiResult<T = serde_json::Value> = Result<Json<T>, AppError>;

pub(crate) fn err(status: StatusCode, msg: impl Into<String>) -> AppError {
    (
        status,
        Json(serde_json::json!({ "ok": false, "error": msg.into() })),
    )
}

pub(crate) fn ok<T: serde::Serialize>(v: T) -> ApiResult<T> {
    Ok(Json(v))
}

/// 把**同步文件访问**挪出 async worker。
///
/// 项目目录经常在网络盘上（RaiDrive/WebDAV 挂载的录播目录）：盘被剪切/渲染的整段读取占满时，
/// 一次 `exists()`、一次读 `yekitrim.toml` 就要几百毫秒到几秒。这类调用留在 worker 上做，
/// 十几个并发（前端一次页面加载就会打出一串）就能把 runtime 的 worker 全占住 ——
/// 表现是"任务在跑的时候整个服务器连新连接都建立不了"（用户实测）。
/// 同步 IO 一律走 blocking 线程池，worker 留给 accept 与轻量接口。
///
/// `pub(crate)`：兄弟模块（如 `stickers.rs` 解压上传的压缩包）也要用同一条通道。
pub(crate) async fn blocking<T, F>(f: F) -> Result<T, AppError>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, AppError> + Send + 'static,
{
    match tokio::task::spawn_blocking(f).await {
        Ok(r) => r,
        Err(e) => Err(err(StatusCode::INTERNAL_SERVER_ERROR, format!("阻塞任务失败: {e}"))),
    }
}

/// 在**当前线程**上把一个 future 跑完（自带一个 current-thread runtime）。
///
/// 只给任务专用线程 / blocking 线程用 —— 在 runtime worker 上调用它正是我们要避免的事。
pub(crate) fn block_on_here<F: std::future::Future>(fut: F) -> F::Output {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("创建任务运行时失败");
    rt.block_on(fut)
}

/// 把任务体放到**专用线程**上跑（自带一个 current-thread runtime）。
///
/// 管线（转写 / 事件检测 / 剪切 / 精修 process）是一整段同步代码：跑在 async worker 上就是
/// 白占一个 worker 到它结束（实测一条 59s 的剪切 = 37s；转写是分钟级），任务一多 worker 就见底。
/// 任务线程与 HTTP 的 worker 彻底分开后，跑多少任务都不影响服务器收发请求。
///
/// **panic 兜底**：任务体的 `complete/fail` 在它自己的末尾，一旦中间 panic，Job 会永远停在
/// `running`、资源键永不释放 —— 同资源（项目/片段）的后续任务就全部排队饿死。
/// 这里接住 panic 并把它记成任务失败（管线里 `join().unwrap()`、下标越界这类都可能触发）。
pub(crate) fn spawn_job<JobBody, Fut>(
    jobs: Arc<crate::jobs::JobManager>,
    job_id: String,
    body: JobBody,
) where
    JobBody: FnOnce() -> Fut + Send + 'static,
    Fut: std::future::Future<Output = ()>,
{
    // 线程起不来时也要给任务收尾（见下），所以失败分支得留着这两个句柄
    let fail_jobs = jobs.clone();
    let fail_id = job_id.clone();
    let spawned = std::thread::Builder::new().name("yekitrim-job".into()).spawn(move || {
        let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| block_on_here(body())));
        let Err(payload) = caught else { return };
        let msg = payload
            .downcast_ref::<&str>()
            .map(|s| (*s).to_string())
            .or_else(|| payload.downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "载荷不是字符串".into());
        tracing::error!("任务线程 panic（job={job_id}）：{msg}");
        // 任务体的 complete/fail 在它自己末尾，panic 就永远调不到 —— 这里替它收尾，
        // 否则 Job 停在 running、资源键不释放，同项目的后续任务会一直排队
        block_on_here(async move {
            jobs.fail(&job_id, format!("任务内部错误（panic）: {msg}")).await;
        });
    });
    if let Err(e) = spawned {
        // ⚠️ 不能只打日志：`create` 已经把这个 Job 标成 Running（或 Queued），
        // 没有工作协程就没有任何人会调 complete/fail —— 它会**永久**占着资源键与档位名额，
        // 同项目/同片段的后续任务全部排队饿死，而队列里只有一个"运行中"的任务什么都查不出来。
        // 兜底跟 panic 那条一样：替它把这个任务记成失败。**两条兜底缺一不可。**
        tracing::error!("启动任务线程失败（job={fail_id}）：{e}");
        block_on_here(async move {
            fail_jobs.fail(&fail_id, format!("启动任务线程失败: {e}")).await;
        });
    }
}

// ---------------------------------------------------------------------------
// B7（2026-11-14 后端 review）：出片 / 管线任务**建任务前**的配额预检。
//
// 配额此前只拦"入库/下载"入口（`ingest::capacity::check_capacity` 的三个调用点），
// 转写/检测/剪切/出片往 `output/` 里写的东西完全不受限 —— 一个已顶满配额的账号
// 仍能反复起任务把磁盘写穿（review 原话："出片/转写产物增长不设上限"）。
//
// 这里做的是 review 里的**低风险版**：建任务时把当前工程的占用刷新落库，然后按
// "已超配额 ⇒ 409"拦。判据复用 `projects::quota_exceeded(used, 0, quota)` ——
// 与入库闸门同一条纯函数、同一个边界（恰好用满放行，超 1 字节就拦）。
//
// 口径：
// - **admin 不受限**（与入库闸门一致：他要能收拾一个已经超额的账号）；
// - `project_id` 传得到就先把**这个工程**的占用刷新落库（`refresh_bytes` 是同步 IO，
//   整段在 blocking 池）；传不到（polish 用 `dir` 且没解析出项目）就按已存的汇总判；
// - **不做产物体积预估**：转写/出片能写多少取决于素材与参数，没有可靠又便宜的估算，
//   拍脑袋的预估只会误拒 —— review 里"能估的用 estimate_bytes"留给有真估算时再接。
// ---------------------------------------------------------------------------

/// 建任务前的配额预检：已超配额的普通账号 ⇒ 409，不再放进新的重活。
pub(super) async fn quota_gate(
    state: &Arc<AppState>,
    user: &crate::auth::CurrentUser,
    project_id: Option<i64>,
) -> Result<(), AppError> {
    if user.is_admin() {
        return Ok(());
    }
    let db = state.auth.db_arc();
    let uid = user.id;
    // ⚠️ `blocking` 的 T 是**成功值**类型（闭包回 `Ok(...)`，模式同 config_llm_put）——
    // 直接把 Result 塞进去会把 T 推断成 ()，`?` 之后就不是 Result 了。
    blocking(move || Ok(quota_gate_impl(&db, uid, project_id))).await?
}

/// [`quota_gate`] 的内核（同步部分：刷新占用 + 判配额，调用方已在 blocking 池）。
fn quota_gate_impl(
    db: &yekitrim_service::db::Db,
    uid: i64,
    project_id: Option<i64>,
) -> Result<(), AppError> {
    // 只刷新**本人**的工程：别人的占用归别人的账
    if let Some(pid) = project_id {
        if let Ok(Some(row)) = db.project_by_id(pid) {
            if row.owner_id == uid {
                let _ = yekitrim_service::projects::refresh_bytes(db, &row);
            }
        }
    }
    // 生效配额：账号自己的非零值 > 全局默认（与 capacity::check_capacity 同一套优先级）
    let cfg = yekitrim_core::config::load_global_config();
    let quota = db
        .find_user_by_id(uid)
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("读取配额失败: {e:#}")))?
        .map(|u| u.quota_bytes.max(0) as u64)
        .filter(|q| *q > 0)
        .or_else(|| (cfg.storage.default_quota_bytes > 0).then_some(cfg.storage.default_quota_bytes));
    let Some(quota) = quota else {
        return Ok(()); // 0 = 不限（全局默认值语义，见 projects::effective_quota_bytes）
    };
    let used = db
        .storage_summary(Some(uid))
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("统计占用失败: {e:#}")))?
        .first()
        .map(|(_, b)| *b)
        .unwrap_or(0);
    if yekitrim_service::projects::quota_exceeded(used, 0, quota) {
        return Err(err(
            StatusCode::CONFLICT,
            format!(
                "本账号已超出配额（已用 {} / 上限 {}），不再启动新的转写/出片任务；\
                 请删除部分项目的源录播或产物后重试",
                yekitrim_service::disk::human_bytes(used),
                yekitrim_service::disk::human_bytes(quota)
            ),
        ));
    }
    Ok(())
}

// 外部命令探测（`command_available` / `command_available_blocking` / 30s 缓存）
// 已搬去 `crate::tool_probe` —— 任务 worker 也要探外部工具，
// 让 worker 去 `use crate::api` 是反向依赖。见该模块头部说明。
//
// ⚠️ 这里曾经还有 `chrome_available()` / `chrome_available_blocking()`（`/api/health` 的
// `chrome` 能力位）。**别把它们加回来**：出片那条 headless Chrome 腿与封面那条 headless Chrome
// 截图腿都已整条删除（2026-10-01），本仓不再需要外部浏览器，探测它的结果没有任何消费者 ——
// 留在 health 上只会让前端以为"还能选 Chrome 出片"。

/// `/api/health`：白名单接口（未登录也要能判断"服务是否活着/要不要登录"）。
///
/// §3.5.9：**不暴露任何路径**，也没有 `mode/auto_login/setup_required` 这类分支字段。
///
/// ⚠️ **账号数量与"是否仍在使用初始密码"不在这里**（曾经有过）——这是**未认证**接口，
/// 对远端说出来等于直接告诉对方"这台机器还在用初始密码，来打它"。
/// 两者都有各自的出口：账号列表 `/api/admin/users`（admin）、自己的改密状态 `/api/me`（已登录）、
/// `POST /api/shutdown` —— **跨进程的优雅关停**（托盘的"退出"走这里）。
///
/// 三道闸，缺一不可：
/// ① env `YEKITRIM_SHUTDOWN_TOKEN` **没设 ⇒ 404**（端点当不存在，不给探测口）；
/// ② 对端必须是 **loopback**（纵深防御：即便将来绑了非本机地址，也不接受远程关停）；
/// ③ 自定义头里的令牌要**常量时间**匹配（自定义头还会先触发 CORS 预检，挡住网页侧乱敲）。
///
/// ⚠️ 效果**等价于 Ctrl+C**：优雅关停、限时 `SHUTDOWN_GRACE`、锁正常释放成 `pid=0`。
/// 别把它改成直接 `process::exit` —— 那会让 `InstanceLock` 的 Drop 跑不到、下次启动多一条残留 WARN。
pub(crate) async fn shutdown(headers: axum::http::HeaderMap, peer: Option<axum::extract::ConnectInfo<std::net::SocketAddr>>) -> ApiResult {
    if std::env::var(crate::auth::SHUTDOWN_TOKEN_ENV).map(|v| v.trim().is_empty()).unwrap_or(true) {
        // 没设令牌 = 这个部署没有控制通道，端点视为不存在
        return Err(err(StatusCode::NOT_FOUND, "Not Found"));
    }
    let is_loopback = peer.map(|axum::extract::ConnectInfo(a)| a.ip().is_loopback()).unwrap_or(false);
    if !is_loopback {
        return Err(err(StatusCode::FORBIDDEN, "关停控制端点只接受 loopback 请求"));
    }
    if !crate::auth::shutdown_token_ok(&headers) {
        return Err(err(StatusCode::FORBIDDEN, "关停令牌不匹配"));
    }
    // 先把响应发出去再关：handler 返回后 axum 才会写响应，而关停是异步的，顺序天然安全
    let requested = crate::request_shutdown();
    ok(serde_json::json!({ "ok": true, "shutdown": requested }))
}

/// `POST /api/open-path`：在**本机**用系统文件管理器弹出这个目录（精修页「打开输出目录」）。
///
/// ⚠️ **只在本地免登录模式下存在**（托盘双击起的那份：绑 loopback + `--local-no-auth`）。
/// 服务端模式一律 **404、当它不存在** —— 否则远端的人点一下，**服务器上**就弹一个窗口：
/// 那是荒唐且有人身风险的能力，不该存在于"多人共用一个后台"的部署里。
/// 判据 = `AppState::local_admin`，与 `auth::authenticate` 那条免登录分支**同一个开关**，
/// 所以不可能出现"能免登录进后台、但这个端点说不通"的错位。
///
/// 只开**已存在的目录**：`canonicalize` 之后再 `is_dir()`，不把带 `..` 的路径丢给 OS 去解析。
/// **不做 shell**（不像 `open_browser` 那样走 `cmd /c start`）：
/// 路径当**单个参数**传给 `explorer` / `open` / `xdg-open`，含 `&`、`%`、空格的路径都不会被拆开。
///
/// ⚠️ 放在 `api/mod.rs` 而不是 `api/polish.rs`：它不是"精修"的语义（任何页面都能用），
/// 而且**不能**挂进 `polish` 那组 —— 那组带 `enforce_scope` 的目录守卫，这条端点收的是
/// 任意本机目录（它自己的闸是"只有本地模式"）。
pub(crate) async fn open_path(
    State(state): State<Arc<AppState>>,
    Json(body): Json<serde_json::Value>,
) -> ApiResult {
    if state.local_admin.is_none() {
        return Err(err(StatusCode::NOT_FOUND, "未知接口: /api/open-path"));
    }
    let raw = body
        .get("path")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    if raw.is_empty() {
        return Err(err(StatusCode::BAD_REQUEST, "缺少 path"));
    }
    let dir = blocking(move || {
        let p = std::path::PathBuf::from(&raw);
        Ok(std::fs::canonicalize(&p).ok().filter(|c| c.is_dir()))
    })
    .await?;
    let Some(dir) = dir else {
        return Err(err(StatusCode::BAD_REQUEST, "目录不存在"));
    };
    // `spawn` 是阻塞调用（要 fork/exec），搬去 blocking 池；结果只关心"进程起没起来"
    let opened = blocking(move || Ok(open_in_file_manager(&dir))).await?;
    match opened {
        Ok(()) => ok(serde_json::json!({ "ok": true })),
        Err(e) => Err(err(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("无法打开目录: {e}"),
        )),
    }
}

/// 用系统文件管理器打开一个**已经校验过**的目录。
fn open_in_file_manager(path: &std::path::Path) -> std::io::Result<()> {
    let program = if cfg!(target_os = "windows") {
        "explorer"
    } else if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    yekitrim_core::process::cmd(program).arg(path).spawn().map(|_| ())
}


/// **`/api/health.llm` 在多配置下也必须是 true**（2026-10-20 那一族 bug：读顶层 api_key ⇒ 恒 false）。
#[test]
fn llm_ready_is_provider_aware() {
    fn cfg_with(p: Option<&str>, legacy: &str) -> yekitrim_core::config::Config {
        let mut c = yekitrim_core::config::Config::default();
        c.llm.api_key = legacy.into();
        if let Some(key) = p {
            c.llm.active = Some("a".into());
            c.llm.providers = vec![yekitrim_core::config::LlmProvider {
                id: "a".into(),
                api: "openai-chat".into(),
                api_key: key.into(),
                ..Default::default()
            }];
        }
        c
    }
    // provider 有 key、顶层没有 ⇒ true（这就是用户那份配置）
    assert!(llm_ready(&cfg_with(Some("k"), "").llm), "多配置下不许再判成未配置");
    // 两份都没有 ⇒ false
    assert!(!llm_ready(&cfg_with(None, "").llm));
    // legacy 顶层有 ⇒ true（老配置行为不变）
    assert!(llm_ready(&cfg_with(None, "k").llm));
    // 模板占位值仍算没配
    assert!(!llm_ready(&cfg_with(None, "sk-your-api-key-here").llm));
}

/// `/api/health` 的能力位：**当前配置配没配 API 密钥**（纯谓词，便于钉住）。
///
/// ⚠️ 入参是 **`LlmConfig` 而不是整份 `Config`**：health 里这一位**按账号**读
/// （`load_llm_config_for(owner)`，见 [`health`]），拿不到 `Config`；判据仍然只有
/// `has_api_key()` 这一个入口。
fn llm_ready(llm: &yekitrim_core::config::LlmConfig) -> bool {
    llm.has_api_key()
}

/// 本地排查 `yekitrim doctor`（直接读本机账本，信息更全）。
/// 顺带一个好处：`/api/health` 会被前端反复轮询，去掉这里两次同步 SQLite 查询后它不再碰磁盘。
pub(crate) async fn health(State(state): State<Arc<AppState>>, headers: axum::http::HeaderMap) -> ApiResult {
    // ⚠️ **`llm` 这一位按调用者的账号算**（2026-11-13 用户报「LLM 明明配好了，切片页仍显示
    // 「未配置 AI」、智能剪辑置灰」）。密钥存在 `users/<uid>/.yekitrim/llm.toml`（设置页
    // `PUT /config/llm` 写的就是那里），而 `load_global_config()` 读的是**顶层旧位置**、
    // 而且**根本不加载 `llm.toml`** ⇒ 恒 `false`。判定必须与真实消费侧
    // （`/polish/llm*`、事件检测）**同一个 loader**：`load_llm_config_for(owner)`。
    // 有会话 ⇒ 该账号；登录页/探活没有会话 ⇒ `Owner::Local`（= 当前管理员，与本地免登录同一档）。
    //
    // 其余能力位仍只看**平台级**配置（`load_global_config()` = 顶层 `config.toml`）：
    // `[ingest]` / `[auth]` 那几项是平台级的（`PUT /api/admin/ingest` 就写顶层那份），
    // 与按账号分的 `config.toml` / `llm.toml` 是**两件事**，别一把全改。
    //
    // 两件同步文件 IO（读配置 + `llm.toml`）都搬进 blocking 池：health 被前端反复轮询，
    // 而配置读取带 `version::prepare` 的迁移回写（见 api/AGENTS.md 的同步 IO 纪律）。
    let (llm_state, llm_headers) = (state.clone(), headers.clone());
    let (cfg, llm) = blocking(move || {
        let cfg = yekitrim_core::config::load_global_config();
        let owner = crate::auth::optional_owner(&llm_state, &llm_headers)
            .unwrap_or(yekitrim_core::owner::Owner::Local);
        let llm = llm_ready(&yekitrim_core::config::load_llm_config_for(owner));
        Ok((cfg, llm))
    })
    .await?;
    // ⚠️ 只查一次，别在 json! 里连着调两遍：`find_ytdlp` 会做 sha256 校验（17.8MB），
    // 而 health 是前端反复轮询的接口（registry 侧另有 (path,len,mtime) 缓存兜底，但两次调用仍多余）。
    let dl = yekitrim_service::downloader::registry::find_ytdlp(&cfg);
    // 同一条纪律：能力位只算一次（`cfg!(feature)` + 一次字体库目录扫描）
    let cover_static = yekitrim_polish::cover::cover_static_capability();
    ok(serde_json::json!({
        "ok": true,
        "version": env!("CARGO_PKG_VERSION"),
        "ffmpeg": command_available("ffmpeg", FFMPEG_VERSION_ARG).await,
        "ffprobe": command_available("ffprobe", FFMPEG_VERSION_ARG).await,
        // ⚠️ 这里曾经有 `"chrome"` 能力位（探测外部浏览器，给出片/封面那两条 headless Chrome
        // 腿用）。两条腿都已整条删除（2026-10-01），本仓不再需要外部浏览器 —— **别加回来**：
        // 它会让人以为还有"选 Chrome 出片/出封面"这条路（前端会据此露出入口）。
        "llm": llm,
        // ⚠️ 这里曾经有 `"wgpu_render"` 能力位（单帧真渲染 /api/polish/render-frame 的开关）。
        // 那条 GPU 链路已删除（用户拍板：GPU 那侧交给独立的 Dhampir 底座），能力位一并去掉 ——
        // 前端也不再按它隐藏任何入口。**别把这一位加回来当"预留能力位"**：它会让人以为还有 GPU 出片。
        // 采集就绪位（P3）：webhook 已启用 + 录播姬目录已配置，前端据此决定是否显示引导卡
        "ingest_ready": cfg.ingest.webhook_enabled && !cfg.ingest.recorder.root.trim().is_empty(),
        // B站下载就绪位（P4）：只回"能不能用"，**不回路径**（与 ffmpeg 那些字段一个口径）。
        // 探测只查文件存在性 + 哈希标记，不跑进程 —— health 会被前端反复轮询。
        "downloader": {
            "kind": yekitrim_service::downloader::DownloaderKind::parse(&cfg.ingest.downloader).label(),
            "ready": dl.is_some(),
            "verified": dl.as_ref().map(|i| i.verified).unwrap_or(false),
        },
        // 注册开关（登录页据此决定要不要显示「用邀请码注册」入口）
        "registration_enabled": cfg.auth.registration_enabled,
        // **多人形态开关**（2026-11-13 用户拍板：默认单人）—— 前端据此隐藏邀请码 / 账号配额 /
        // 注册入口 / 任务指派对象。只回布尔，不回任何账号信息（health 是公开接口）。
        "multi_user": cfg.auth.multi_user,
        // 本地免登录位（`--local-no-auth`，托盘那种"双击就用"的桌面形态）。
        // 前端据此隐藏「退出登录」这种在没有会话时毫无意义的入口。
        // ⚠️ 只回布尔，不回任何身份信息 —— health 是**公开**接口。
        "local_no_auth": state.local_admin.is_some(),
        // 底座出片通道的就绪位（M3）：**只回布尔**，不回任何路径 —— health 是公开接口
        // （路径、Node 版本与"有没有粗体那一支"走已登录的 `POST /api/polish/dhampir-status`）。
        // `deep = false`：这条被前端反复轮询，不起进程（也不跑 `node --version`）。
        "dhampir_ready": yekitrim_polish::render::dhampir::readiness("", false).ready,
        // 封面静态渲染通道（Takumi）的能力位（2026-10-01）：前端据此决定弹窗里那个选项
        // 是能点还是置灰 + 为什么。
        //
        // ⚠️ **只回布尔与计数，不回任何路径**（2026-11-14 后端 review B6）：这里原来还带了
        // `fonts_dir`（绝对路径），而 health 是**未认证**接口 —— 与同一张卡里"不含任何路径"的
        // 口径直接打架（实测匿名请求能拿到 `/home/…/resources/fonts`）。要看库路径请走已登录的
        // `GET /api/polish/fonts`（它回 `root`，且只回 admin 的）。
        // TODO(api/AGENTS.md): 那张卡里"只回布尔 + 字体库路径"与"不含任何路径"两句自相矛盾，
        // 需要 Lead 统一措辞（文档归 Lead，这里只留代码口径）。
        "cover_static": {
            "compiled": cover_static.0,
            "fonts": cover_static.1,
            "ready": cover_static.0 && cover_static.1 > 0,
        },
        // 本机/远端与是否走 HTTPS 反代：只影响本地排查提示，不含任何账号信息
        "listening": state.auth.listening().as_str(),
        "tls_terminated": crate::auth::tls_terminated(&headers),
    }))
}

pub(crate) mod compress;
pub(crate) mod routes;
pub(crate) mod auth;
pub(crate) mod invite;
pub(crate) mod logs;
pub(crate) mod rooms;
pub(crate) mod config;
pub(crate) mod projects;
pub(crate) mod project_data;
pub(crate) mod pipeline;
pub(crate) mod render;
pub(crate) mod polish;
pub(crate) mod core;

/// 单测共用的 HTTP 夹具（只在这些测试里编译）。
#[cfg(test)]
pub(crate) mod test_support;

/// 路由表的唯一入口（`lib.rs` 与路由单测都走 `crate::api::router`）。
pub use routes::router;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jobs::{JobClass, JobSpec};

/// 整改 T12：`/api/health.cover_static.compiled` 在**带 feature 的构建**里必须是 true。
///
/// 锚的正是 `health()` 那一列（`yekitrim_polish::cover::cover_static_capability().0`）。
/// 不带 feature 时 `render/takumi.rs`（约 1.1k 行）整份不参与编译 ⇒ 恒 false，
/// 于是「WebUI 生成封面那一半还能不能编过」在服务端语境里**没有任何判据**。
/// 带 feature 跑：`cargo test -p yekitrim-server --features cover-takumi`。
#[cfg(feature = "cover-takumi")]
#[test]
fn cover_static_compiled_is_true_with_the_feature() {
    let (compiled, _fonts, _dir) = yekitrim_polish::cover::cover_static_capability();
    assert!(
        compiled,
        "带 cover-takumi 时 cover_static.compiled 必须是 true（/api/health 靠它给前端置灰）"
    );
}

/// 反过来：不带 feature 时必须为 false —— 否则「到底编没编进来」就没有判据了。
#[cfg(not(feature = "cover-takumi"))]
#[test]
fn cover_static_compiled_is_false_without_the_feature() {
    let (compiled, _fonts, _dir) = yekitrim_polish::cover::cover_static_capability();
    assert!(
        !compiled,
        "不带 cover-takumi 时 cover_static.compiled 必须是 false"
    );
}




/// 阻塞工作必须真的跑到 blocking 池上：worker 全程空转（这里用一个多线程 runtime
/// 只留 1 个 worker 来模拟"worker 被占满"，同步阻塞跑在 worker 上就会互相排队）。
#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn blocking_helper_keeps_workers_free() {
    // 两个"重活"各睡 300ms：走 spawn_blocking 时它们并行（~300ms），
    // 若留在 worker 上就只能串行（~600ms）。
    let t0 = std::time::Instant::now();
    let (a, b) = tokio::join!(
        blocking(|| {
            std::thread::sleep(std::time::Duration::from_millis(300));
            Ok::<_, AppError>(1)
        }),
        blocking(|| {
            std::thread::sleep(std::time::Duration::from_millis(300));
            Ok::<_, AppError>(2)
        }),
    );
    let elapsed = t0.elapsed();
    assert_eq!((a.unwrap(), b.unwrap()), (1, 2));
    assert!(elapsed.as_millis() < 550, "两段阻塞工作应当是并行的，实测 {elapsed:?}");

    // worker 只有一个，但上面两段阻塞期间调度器仍然能跑别的任务
    let ticked = tokio::time::timeout(std::time::Duration::from_millis(200), async {
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        true
    })
    .await
    .unwrap_or(false);
    assert!(ticked, "blocking 池占用期间 worker 仍应能推进其它任务");
}

/// 任务体 panic 也要把 Job 收尾（否则永远 running、资源键不释放、同项目后续任务全部排队）
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn spawn_job_marks_job_failed_on_panic() {
    let jobs = Arc::new(crate::jobs::JobManager::new());
    let handle = jobs
        .create(1, JobSpec::new("test_panic", "panic 测试", vec!["dir:x".into()], JobClass::Medium))
        .await;
    let id = handle.id.clone();
    spawn_job(jobs.clone(), id.clone(), move || async move {
        let _ = handle.wait_start().await;
        panic!("模拟管线里的 join().unwrap() / 下标越界");
    });
    let mut status = String::new();
    for _ in 0..100 {
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        if let Some(j) = jobs.get(&id).await {
            status = j.status.clone();
            if status == "failed" {
                break;
            }
        }
    }
    assert_eq!(status, "failed", "任务 panic 后应当被标记为 failed");
    // 资源键必须释放：同资源的新任务能立刻开工
    assert!(jobs.busy_with(&["dir:x".to_string()]).await.is_none(), "panic 的任务不应继续占着资源键");
}

/// **B7（2026-11-14 后端 review）：配额预检的内核判据**。
///
/// `refresh_bytes` 把当前工程的源/产物落库后，`used > quota` ⇒ 409；恰好用满**放行**
/// （与入库闸门 `quota_exceeded` 的边界同一条，别在这里另立 `>=`）。
#[test]
fn quota_gate_impl_rejects_only_after_refreshing_usage() {
    let dir = tempfile::tempdir().unwrap();
    let db = yekitrim_service::db::Db::open(&dir.path().join("t.db")).unwrap();
    let uid = db.create_user("quota_u", "h", "user", false, 100).unwrap();
    let root = dir.path().join("proj");
    std::fs::create_dir_all(root.join("output")).unwrap();
    std::fs::write(root.join("a.mp4"), vec![0u8; 200]).unwrap(); // 200 字节 > 配额 100
    let cfg = root.join("yekitrim.toml");
    std::fs::write(&cfg, "input_file = \"a.mp4\"\n").unwrap();
    let pid = db.create_project(uid, "proj", None, "local", None, &root, &cfg, None).unwrap();

    // ① 第一次调用自己刷新占用：200 > 100 ⇒ 409（判据内部先 refresh，不依赖外部刷过账）
    let e = quota_gate_impl(&db, uid, Some(pid)).unwrap_err();
    assert_eq!(e.0, StatusCode::CONFLICT, "{e:?}");
    assert!(e.1 .0["error"].as_str().unwrap_or("").contains("配额"), "{e:?}");
    // 刷新真的落了库（storage_summary 反映盘上实况，而不是"没刷过账就放行"）
    assert_eq!(
        db.storage_summary(Some(uid)).unwrap().first().map(|(_, b)| *b).unwrap_or(0),
        200,
        "源录播要被计量落库"
    );

    // ② 删掉源再刷新：占用回落 ⇒ 放行（不是"一旦超就永远超"）
    std::fs::remove_file(root.join("a.mp4")).unwrap();
    assert!(quota_gate_impl(&db, uid, Some(pid)).is_ok(), "占用回落后要放行");

    // ③ 源回来了：再次拦（占用双向校准）
    std::fs::write(root.join("a.mp4"), vec![0u8; 200]).unwrap();
    assert!(quota_gate_impl(&db, uid, Some(pid)).is_err(), "占用回升要再拦");

    // ④ 别人的工程：不刷、不拦（配额按账号分账）
    let uid2 = db.create_user("quota_v", "h", "user", false, 100).unwrap();
    assert!(quota_gate_impl(&db, uid2, Some(pid)).is_ok(), "别人的工程不许动他的账");
}

/// B7：admin 不受限（`quota_gate` 的短路分支 —— 不读库、不刷盘，直接放行）。
#[tokio::test]
async fn quota_gate_lets_admin_through_without_touching_the_ledger() {
    let t = crate::api::test_support::test_app().await;
    let user = crate::auth::CurrentUser {
        id: 1,
        username: "admin".into(),
        role: "admin".into(),
        must_change_password: false,
        session_id: "test-session".into(),
    };
    let res = quota_gate(&t.state, &user, Some(1)).await;
    assert!(res.is_ok(), "admin 必须无条件放行：{res:?}");
}

/// 错误原样透传（handler 靠它把 4xx/5xx 文案给前端）
#[tokio::test]
async fn blocking_helper_passes_error_through() {
    let r = blocking(|| Err::<i32, _>(err(StatusCode::NOT_FOUND, "没有这条"))).await;
    match r {
        Err((status, Json(body))) => {
            assert_eq!(status, StatusCode::NOT_FOUND);
            assert_eq!(body["error"], "没有这条");
        }
        Ok(_) => panic!("应当把错误透传出去"),
    }
}
}
