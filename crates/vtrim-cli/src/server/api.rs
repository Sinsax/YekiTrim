//! HTTP REST API handlers。
//!
//! 路由分组：
//! - /health /doctor /config /models
//! - /project/*
//! - /data/*
//! - /pipeline/* /jobs/*
//! - /polish/*

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use axum::{
    Router,
    extract::{Extension, Path as AxumPath, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response, sse::{Event, KeepAlive, Sse}},
    routing::{get, post, put},
    Json,
};
use futures_util::{StreamExt, stream::Stream};
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::wrappers::errors::BroadcastStreamRecvError;
use vtrim_service::Workspace;
use vtrim_service::logbus;

use super::model::AppState;
use super::scope::Scope;
use super::{
    jobs::{JobClass, JobEvent, JobSpec},
    media, persist,
};

pub fn router(state: Arc<AppState>) -> Router {
    // 白名单（§10.1 step-2.1）：登录端点与 /health 之外，**所有**接口都要会话。
    let public = Router::new()
        .route("/health", get(health))
        .route("/auth/login", post(auth_login))
        // 自助注册：邀请码是凭证，注册开关是总闸；两者都由服务端校验（§3.5 之外）
        .route("/auth/register", post(auth_register))
        // 录播姬 webhook：自带 token 鉴权（录播姬只能填 URL，无法加自定义头），
        // 因此它**不能**挂在 require_user 下（§4.1）。
        .route("/ingest/bililive/:token", post(super::ingest::webhook));

    // 平台级资源（P2 起归入 /api/admin/*，仅 admin）：配置 / 模型 / doctor / LLM 测试
    // 旧路径作为过渡别名挂在**同一个 require_admin 层**下（否则普通账号能读到 LLM key）
    let admin = Router::new()
        .route("/admin/doctor", get(doctor))
        .route("/admin/config", get(config_get))
        .route("/admin/config/global", put(config_global_put))
        .route("/admin/config/llm", put(config_llm_put))
        .route("/admin/llm/test", post(llm_test))
        .route("/admin/models", get(models_get))
        .route("/admin/models/download", post(models_download))
        // 采集/录播姬：`GET` 读概览、`PUT` 保存配置（表单）、`POST .../token` 生成/关闭 webhook token
        .route(
            "/admin/ingest",
            get(super::ingest::config_view).put(super::ingest::put_config),
        )
        .route("/admin/ingest/token", post(super::ingest::rotate_token))
        .route("/admin/ingest/assign", post(super::ingest::assign))
        .route("/admin/ingest/scan", post(super::ingest::scan_now))
        .route("/admin/users", get(admin_users))
        .route("/admin/recorder/test", post(admin_recorder_test))
        .route("/admin/invites", get(admin_invites).post(admin_invite_create))
        .route("/admin/invites/registration", put(admin_registration_put))
        .route("/admin/invites/:id", axum::routing::delete(admin_invite_revoke))
        // 运行日志（服务端 tracing → 环形缓冲 / SSE；「运行日志」面板用，仅 admin）
        .route("/admin/logs", get(admin_logs))
        .route("/admin/logs/events", get(admin_logs_stream))
        // LLM 用量（每次调用的 token / 缓存命中 / 耗时；设置页「LLM 用量」卡，仅 admin）
        .route("/admin/usage", get(admin_usage))
        // 表情包（本地贴纸库）：全局资产，只有管理员能导入 / 构建 / 删除
        .route("/admin/stickers", get(super::stickers::packs_list))
        .route("/admin/stickers/import", post(super::stickers::pack_import))
        // 上传 zip 建新包（WebUI 的「添加表情包」）。请求体是压缩包原始字节 ——
        // 单独给这一条放开体积上限（axum 默认 2 MB），其它接口维持不变。
        .route(
            "/admin/stickers/upload",
            post(super::stickers::pack_upload)
                .layer(axum::extract::DefaultBodyLimit::max(
                    super::stickers::UPLOAD_MAX_BYTES,
                )),
        )
        .route(
            "/admin/stickers/:pack",
            get(super::stickers::pack_detail).delete(super::stickers::pack_delete),
        )
        .route("/admin/stickers/:pack/build", post(super::stickers::pack_build))
        .route("/admin/stickers/:pack/files/:name", get(super::stickers::pack_file))
        .route("/doctor", get(doctor))
        .route("/config", get(config_get))
        .route("/config/global", put(config_global_put))
        .route("/config/llm", put(config_llm_put))
        .route("/llm/test", post(llm_test))
        .route("/models", get(models_get))
        .route("/models/download", post(models_download))
        .route_layer(axum::middleware::from_fn_with_state(
            state.clone(),
            super::auth::require_admin,
        ));

    // polish 系列：额外加"目录必须落在本人项目内"的守卫（路径面归零，§5.2）
    let polish = Router::new()
        .route("/polish/init", post(polish_init))
        .route("/polish/llm", post(polish_llm))
        .route("/polish/process", post(polish_process))
        .route("/polish/render", post(polish_render))
        .route("/polish/cover", post(polish_cover))
        .route("/polish/preview", post(polish_preview))
        .route("/polish/cover-preview", post(polish_cover_preview))
        .route("/polish/cover-background", post(polish_cover_background))
        .route("/polish/cover-frame", post(polish_cover_frame))
        .route("/polish/project", get(polish_project_get).put(polish_project_put))
        .route("/polish/subtitles", get(polish_subtitles_get).put(polish_subtitles_put))
        .route("/polish/sticker-sources", get(polish_sticker_sources))
        // 贴纸库只读浏览（编辑器里 keyword→贴纸 的两级选择）；写操作仍只在 admin
        .route("/polish/sticker-library", get(polish_sticker_library))
        .route("/polish/sticker-file", get(polish_sticker_file))
        // 音效目录（编辑器里音效事件的列表选择）
        .route("/polish/sfx-catalog", get(polish_sfx_catalog))
        .route("/polish/render-frame", post(polish_render_frame))
        .route("/polish/font", get(polish_font))
        // 字体库只读浏览（exe 同级 resources/fonts/）：字幕/封面字体的下拉数据源。
        // 与贴纸库同规矩 —— 只管读，**不放上传**（字体由管理员自己丢进目录）
        .route("/polish/fonts", get(polish_fonts))
        .route("/polish/select-version", post(polish_select_version))
        .route("/polish/silence", post(polish_silence))
        .route("/polish/cover-title", post(polish_cover_title))
        .route("/polish/cover-config", post(polish_cover_config))
        .route_layer(axum::middleware::from_fn_with_state(
            state.clone(),
            super::scope::enforce_scope,
        ));

    let protected = Router::new()
        .route("/me", get(me))
        .route("/auth/logout", post(auth_logout))
        .route("/auth/password", put(auth_password))
        .route("/project/list", get(project_list))
        .route("/projects", get(projects_list).post(projects_create))
        .route("/projects/:id", axum::routing::delete(projects_delete))
        .route("/projects/:id/source", axum::routing::delete(projects_source_delete))
        .route("/project/delete", post(project_delete))
        .route("/project/init", post(project_init))
        .route("/project/open", post(project_open))
        .route("/project-data/status", get(project_data_status))
        .route("/project-data/events", get(project_data_events).post(project_data_events_create).put(project_data_events_put))
        .route("/project-data/clips", get(project_data_clips))
        .route("/project-data/clips/:rank", axum::routing::delete(project_data_clip_delete))
        .route("/project-data/clips/:rank/thumb", get(project_data_clip_thumb))
        .route("/project-data/events/cut-all", post(project_data_event_cut_all))
        .route("/project-data/events/merge-splits", post(project_data_events_merge_splits))
        .route("/project-data/events/:rank/cut", post(project_data_event_cut))
        .route("/project-data/clips/:rank/silence", post(project_data_clip_silence))
        .route("/project-data/transcript", get(project_data_transcript).put(project_data_transcript_put))
        .route("/project-data/correct", post(project_data_correct))
        .route("/project-data/pipeline", post(project_data_pipeline))
        .route("/project-data/config", get(project_data_config).put(project_data_config_put))
        // 素材（弹幕 / 字幕）：预处理页随时查看与切换
        .route("/project-data/assets", get(project_data_assets).post(project_data_assets_put))
        .route("/project-data/polish", get(project_data_polish))
        // 直播间自助登记（用户只填房间号/链接，服务端去录播姬加房间）
        .route("/rooms", get(rooms_list).post(rooms_add))
        .route("/rooms/:room_id", axum::routing::delete(rooms_delete))
        // 该房间在录像目录里的文件（展开房间行时用）+ 单个文件导入
        .route("/rooms/:room_id/recordings", get(super::ingest::room_recordings))
        .route(
            "/rooms/:room_id/recordings/import",
            post(super::ingest::import_recording),
        )
        .route("/fs/list", post(fs_list))
        .route("/ingest/records", get(super::ingest::records))
        .route("/ingest/retry", post(super::ingest::retry))
        .route("/channel-map", get(super::ingest::channel_map_get).put(super::ingest::channel_map_put))
        .route("/stream", get(global_stream))
        .route("/jobs", get(jobs_list))
        .route("/jobs/:id", get(job_get))
        .route("/jobs/:id/events", get(job_events))
        .route("/jobs/:id/cancel", post(job_cancel))
        .route("/media/*path", get(media_get))
        .merge(polish)
        // ⚠️ 两层的顺序**不能反**：`require_user` 必须在外层（先跑并写入 `CurrentUser`），
        // 改密强制门在内层才读得到用户。axum 里"后加的 layer 在外层"，所以这两行看起来是倒着写的。
        .route_layer(axum::middleware::from_fn(super::auth::require_password_changed))
        .route_layer(axum::middleware::from_fn_with_state(
            state.clone(),
            super::auth::require_user,
        ));

    public
        .merge(admin)
        .merge(protected)
        // 未匹配的 /api/* 必须是 JSON 404：外层 fallback 是 SPA HTML，
        // 若让它兜住 API 请求，前端会把 200 HTML 解析成 null 并静默失败。
        .fallback(api_not_found)
        // 访问日志挂最外层：**每次前端交互都留痕**（含上面这个 JSON 404）。
        // 挂在 api::router 里而不是 `nest("/api", …)` 外面，是为了让路由单测也走它。
        .layer(axum::middleware::from_fn(super::access_log::access_log))
        // 限流挂**最外层的更外层**（在访问日志之外）：
        // 1. 要覆盖未认证入口（登录/注册/入库 webhook），挂进 require_user 就漏了；
        // 2. 被限流的请求不该再各写一行访问日志 —— 否则刷接口的人顺手把日志也打满。
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            super::ratelimit::limit_requests,
        ))
        .with_state(state)
}

async fn api_not_found(req: axum::extract::Request) -> Response {
    let method = req.method().clone();
    // 这个 fallback 挂在 `nest("/api", …)` 里，`uri().path()` 已经去掉了 `/api` 前缀；
    // 用 OriginalUri 还原成用户实际请求的路径，否则报错文案会指向不存在的 URL。
    let path = req
        .extensions()
        .get::<axum::extract::OriginalUri>()
        .map(|u| u.0.path().to_string())
        .unwrap_or_else(|| req.uri().path().to_string());
    (
        StatusCode::NOT_FOUND,
        Json(serde_json::json!({
            "ok": false,
            "error": format!("未知接口: {method} {path}"),
        })),
    )
        .into_response()
}

type AppError = (StatusCode, Json<serde_json::Value>);
type ApiResult<T = serde_json::Value> = Result<Json<T>, AppError>;

fn err(status: StatusCode, msg: impl Into<String>) -> AppError {
    (
        status,
        Json(serde_json::json!({ "ok": false, "error": msg.into() })),
    )
}

fn ok<T: serde::Serialize>(v: T) -> ApiResult<T> {
    Ok(Json(v))
}

/// 把**同步文件访问**挪出 async worker。
///
/// 项目目录经常在网络盘上（RaiDrive/WebDAV 挂载的录播目录）：盘被剪切/渲染的整段读取占满时，
/// 一次 `exists()`、一次读 `vtrim.toml` 就要几百毫秒到几秒。这类调用留在 worker 上做，
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
    jobs: Arc<super::jobs::JobManager>,
    job_id: String,
    body: JobBody,
) where
    JobBody: FnOnce() -> Fut + Send + 'static,
    Fut: std::future::Future<Output = ()>,
{
    // 线程起不来时也要给任务收尾（见下），所以失败分支得留着这两个句柄
    let fail_jobs = jobs.clone();
    let fail_id = job_id.clone();
    let spawned = std::thread::Builder::new().name("vtrim-job".into()).spawn(move || {
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

async fn workspace_from_state(state: &AppState) -> Workspace {
    state.workspace().await
}

/// 外部命令探测结果缓存（30s TTL）。
///
/// `/api/health` 会被前端在页面挂载与「环境检查」时调用，同步 spawn
/// ffmpeg/ffprobe/chrome（最多 7 个进程）会占住 tokio worker，故探测放
/// `spawn_blocking` 并缓存。
type ProbeCache = std::sync::Mutex<std::collections::HashMap<String, (std::time::Instant, Option<String>)>>;

fn probe_cache() -> &'static ProbeCache {
    static CACHE: std::sync::OnceLock<ProbeCache> = std::sync::OnceLock::new();
    CACHE.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()))
}

fn cache_get(key: &str, ttl: Duration) -> Option<Option<String>> {
    let cache = probe_cache().lock().unwrap_or_else(|e| e.into_inner());
    cache
        .get(key)
        .filter(|(t, _)| t.elapsed() < ttl)
        .map(|(_, v)| v.clone())
}

fn cache_put(key: String, value: Option<String>) {
    let mut cache = probe_cache().lock().unwrap_or_else(|e| e.into_inner());
    cache.insert(key, (std::time::Instant::now(), value));
}

fn command_available_blocking(cmd: &str) -> Option<String> {
    let output = std::process::Command::new(cmd).arg("-version").output().ok()?;
    if !output.status.success() { return None; }
    let first = String::from_utf8_lossy(&output.stdout).lines().next().map(|s| s.to_string()).unwrap_or_default();
    Some(first)
}

async fn command_available(cmd: &str) -> Option<String> {
    let key = format!("cmd:{cmd}");
    if let Some(cached) = cache_get(&key, Duration::from_secs(30)) {
        return cached;
    }
    let owned = cmd.to_string();
    let value = tokio::task::spawn_blocking(move || command_available_blocking(&owned))
        .await
        .ok()
        .flatten();
    cache_put(key, value.clone());
    value
}

fn chrome_available_blocking(custom: &str) -> Option<String> {
    let probe = |c: &str| -> Option<String> {
        if Path::new(c).exists() {
            return Some(c.to_string());
        }
        std::process::Command::new(c)
            .arg("--version")
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|_| c.to_string())
    };
    // 全局配置 [chrome] path 优先
    if !custom.is_empty() {
        return probe(custom);
    }
    let candidates = [
        "chrome",
        "google-chrome",
        "chromium",
        r"C:\Program Files\Google\Chrome\Application\chrome.exe",
        r"C:\Program Files (x86)\Google\Chrome\Application\chrome.exe",
    ];
    candidates.iter().find_map(|c| probe(c))
}

async fn chrome_available(cfg: Option<&vtrim_core::config::Config>) -> bool {
    let custom = cfg.map(|c| c.chrome.path.trim()).unwrap_or("").to_string();
    let key = format!("chrome:{custom}");
    if let Some(cached) = cache_get(&key, Duration::from_secs(30)) {
        return cached.is_some();
    }
    let value = tokio::task::spawn_blocking(move || chrome_available_blocking(&custom))
        .await
        .ok()
        .flatten();
    let ok = value.is_some();
    cache_put(key, value);
    ok
}

/// `/api/health`：白名单接口（未登录也要能判断"服务是否活着/要不要登录"）。
///
/// §3.5.9：**不暴露任何路径**，也没有 `mode/auto_login/setup_required` 这类分支字段。
///
/// ⚠️ **账号数量与"是否仍在使用初始密码"不在这里**（曾经有过）——这是**未认证**接口，
/// 对远端说出来等于直接告诉对方"这台机器还在用初始密码，来打它"。
/// 两者都有各自的出口：账号列表 `/api/admin/users`（admin）、自己的改密状态 `/api/me`（已登录）、
/// 本地排查 `vtrim doctor`（直接读本机账本，信息更全）。
/// 顺带一个好处：`/api/health` 会被前端反复轮询，去掉这里两次同步 SQLite 查询后它不再碰磁盘。
async fn health(State(state): State<Arc<AppState>>, headers: axum::http::HeaderMap) -> ApiResult {
    // 能力位只看平台级配置（global-only）：health 是公开接口，不该泄露任何工程信息
    let cfg = vtrim_core::config::load_global_config();
    let llm = !cfg.llm.api_key.is_empty() && cfg.llm.api_key != "sk-your-api-key-here";
    ok(serde_json::json!({
        "ok": true,
        "version": env!("CARGO_PKG_VERSION"),
        "ffmpeg": command_available("ffmpeg").await,
        "ffprobe": command_available("ffprobe").await,
        "chrome": chrome_available(Some(&cfg)).await,
        "llm": llm,
        // 单帧真渲染（/api/polish/render-frame）只在启用该 feature 的构建里可用
        "wgpu_render": cfg!(feature = "wgpu-render"),
        // 采集就绪位（P3）：webhook 已启用 + 录播姬目录已配置，前端据此决定是否显示引导卡
        "ingest_ready": cfg.ingest.webhook_enabled && !cfg.ingest.recorder.root.trim().is_empty(),
        // 注册开关（登录页据此决定要不要显示「用邀请码注册」入口）
        "registration_enabled": cfg.auth.registration_enabled,
        // 本机/远端与是否走 HTTPS 反代：只影响本地排查提示，不含任何账号信息
        "listening": state.auth.listening().as_str(),
        "tls_terminated": super::auth::tls_terminated(&headers),
    }))
}

// ---- 认证（M1 / §3.5） ----

/// `GET /api/me`：已登录 → 200 `{user, must_change_password}`；未登录 → 中间件给 401。
async fn me(Extension(user): Extension<super::auth::CurrentUser>) -> ApiResult {
    ok(serde_json::json!({
        "ok": true,
        "user": user.json(),
        "must_change_password": user.must_change_password,
    }))
}

/// `POST /api/auth/login {username, password}` → 200 + `Set-Cookie`；401/429。
async fn auth_login(
    State(state): State<Arc<AppState>>,
    super::auth::ClientIp(ip): super::auth::ClientIp,
    headers: axum::http::HeaderMap,
    Json(body): Json<serde_json::Value>,
) -> Response {
    let username = body.get("username").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let password = body.get("password").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let user_agent = headers
        .get(axum::http::header::USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());
    let secure = super::auth::tls_terminated(&headers);

    let auth = state.auth.clone();
    // scrypt（N=2^15）是 CPU 密集：必须在 blocking 线程池里跑，否则会卡住 tokio worker
    let result = tokio::task::spawn_blocking(move || {
        auth.login(&username, &password, ip.as_deref(), user_agent.as_deref())
    })
    .await;

    match result {
        Ok(Ok(bundle)) => {
            let mut resp = Json(serde_json::json!({
                "ok": true,
                "user": bundle.user.json(),
                "must_change_password": bundle.user.must_change_password,
            }))
            .into_response();
            super::auth::append_cookies(&mut resp, &super::auth::session_cookies(&bundle, secure));
            resp
        }
        Ok(Err(super::auth::LoginError::RateLimited(secs))) => err(
            StatusCode::TOO_MANY_REQUESTS,
            format!("尝试次数过多，请在 {} 分钟后再试", (secs + 59) / 60),
        )
        .into_response(),
        Ok(Err(super::auth::LoginError::Denied(msg))) => {
            err(StatusCode::UNAUTHORIZED, msg).into_response()
        }
        Ok(Err(super::auth::LoginError::Internal(msg))) => {
            tracing::error!("登录失败（内部错误）: {msg}");
            err(StatusCode::INTERNAL_SERVER_ERROR, "登录失败，请查看服务端日志").into_response()
        }
        Err(e) => {
            tracing::error!("登录任务失败: {e}");
            err(StatusCode::INTERNAL_SERVER_ERROR, "登录失败，请重试").into_response()
        }
    }
}

/// `POST /api/auth/logout`：删会话行 + 清 cookie。
async fn auth_logout(
    State(state): State<Arc<AppState>>,
    super::auth::ClientIp(ip): super::auth::ClientIp,
    Extension(user): Extension<super::auth::CurrentUser>,
) -> Response {
    state.auth.logout(&user.session_id, user.id, ip.as_deref());
    let mut resp = ok(serde_json::json!({ "ok": true })).into_response();
    super::auth::append_cookies(&mut resp, &super::auth::clear_cookies());
    resp
}

/// `POST /api/auth/register {username, password, invite}`：邀请码自助注册（公开接口）。
///
/// 三重条件：`[auth] registration_enabled` 打开 + 邀请码本身有效 + 用户名/密码合规。
/// 成功后与登录一样下发会话 cookie（注册即登录，角色固定 `user`）。
async fn auth_register(
    State(state): State<Arc<AppState>>,
    super::auth::ClientIp(ip): super::auth::ClientIp,
    headers: axum::http::HeaderMap,
    Json(body): Json<serde_json::Value>,
) -> Response {
    // 总闸：关着就直接 403（不透露有没有可用的码）
    if !vtrim_core::config::load_global_config().auth.registration_enabled {
        return err(StatusCode::FORBIDDEN, "注册未开放（管理员可用 vtrim user invite 生成邀请码并开启）").into_response();
    }
    let username = body.get("username").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let password = body.get("password").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let invite = body.get("invite").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let user_agent = headers
        .get(axum::http::header::USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());
    let secure = super::auth::tls_terminated(&headers);

    let auth = state.auth.clone();
    // 与登录同理：scrypt 是 CPU 密集，挪到 blocking 线程池
    let result = tokio::task::spawn_blocking(move || {
        auth.register(&username, &password, &invite, ip.as_deref(), user_agent.as_deref())
    })
    .await;

    match result {
        Ok(Ok(bundle)) => {
            let mut resp = Json(serde_json::json!({
                "ok": true,
                "user": bundle.user.json(),
                "must_change_password": false,
            }))
            .into_response();
            super::auth::append_cookies(&mut resp, &super::auth::session_cookies(&bundle, secure));
            resp
        }
        Ok(Err(super::auth::LoginError::RateLimited(secs))) => err(
            StatusCode::TOO_MANY_REQUESTS,
            format!("尝试次数过多，请在 {} 分钟后再试", (secs + 59) / 60),
        )
        .into_response(),
        // 注册失败一律 403：不区分"码不对"和"码已用过"，避免被拿来枚举
        Ok(Err(super::auth::LoginError::Denied(msg))) => {
            err(StatusCode::FORBIDDEN, msg).into_response()
        }
        Ok(Err(super::auth::LoginError::Internal(msg))) => {
            tracing::error!("注册失败（内部错误）: {msg}");
            err(StatusCode::INTERNAL_SERVER_ERROR, "注册失败，请查看服务端日志").into_response()
        }
        Err(e) => {
            tracing::error!("注册任务失败: {e}");
            err(StatusCode::INTERNAL_SERVER_ERROR, "注册失败，请查看服务端日志").into_response()
        }
    }
}

/// `PUT /api/auth/password {old_password, new_password}`：改自己的密码。
///
/// 成功后 `must_change_password=false`（顶部横幅消失）、初始密码文件被删、
/// 本人的其它会话全部失效（当前会话保留）。
async fn auth_password(
    State(state): State<Arc<AppState>>,
    super::auth::ClientIp(ip): super::auth::ClientIp,
    Extension(user): Extension<super::auth::CurrentUser>,
    Json(body): Json<serde_json::Value>,
) -> Response {
    let old_password = body.get("old_password").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let new_password = body.get("new_password").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let auth = state.auth.clone();
    let user_id = user.id;
    let session_id = user.session_id.clone();
    let result = tokio::task::spawn_blocking(move || {
        auth.change_password(user_id, &session_id, &old_password, &new_password, ip.as_deref())
    })
    .await;

    match result {
        Ok(Ok(())) => ok(serde_json::json!({ "ok": true, "must_change_password": false })).into_response(),
        Ok(Err(super::auth::LoginError::Denied(msg))) => {
            err(StatusCode::BAD_REQUEST, msg).into_response()
        }
        Ok(Err(super::auth::LoginError::RateLimited(_))) => {
            err(StatusCode::TOO_MANY_REQUESTS, "操作过于频繁，请稍后再试").into_response()
        }
        Ok(Err(super::auth::LoginError::Internal(msg))) => {
            tracing::error!("改密失败（内部错误）: {msg}");
            err(StatusCode::INTERNAL_SERVER_ERROR, "修改密码失败，请查看服务端日志").into_response()
        }
        Err(e) => {
            tracing::error!("改密任务失败: {e}");
            err(StatusCode::INTERNAL_SERVER_ERROR, "修改密码失败，请重试").into_response()
        }
    }
}

async fn doctor(State(state): State<Arc<AppState>>) -> ApiResult {
    let ws = workspace_from_state(&state).await;
    let home = vtrim_core::path::vtrim_home();
    let models_dir = home.join("models");
    // 与 /api/models 同源：按运行时查找顺序（配置 → 用户目录 → 程序目录）判"能不能用"
    let asr = vtrim_core::config::load_global_config().asr;
    let model_status: Vec<_> = vtrim_core::download::model_names().iter().map(|m| {
        let st = vtrim_core::download::model_install(&asr, m);
        serde_json::json!({
            "name": m,
            "installed": st.installed,
            "dir": vtrim_core::path::display(&st.dir),
            "source": st.source.as_str(),
        })
    }).collect();
    ok(serde_json::json!({
        "ffmpeg": command_available("ffmpeg").await,
        "ffprobe": command_available("ffprobe").await,
        "models_dir": models_dir.to_string_lossy(),
        "models": model_status,
        "workspace": vtrim_core::path::display(&ws.config_file),
    }))
}

/// `GET /api/admin/users`：账号列表（**仅 admin**）。只回展示需要的字段，绝不含口令哈希。
///
/// 用途：入库待分配池的"指派给谁"下拉、后续管理页。
async fn admin_users(State(state): State<Arc<AppState>>) -> ApiResult {
    let db = state.auth.db();
    let users = db
        .list_users()
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("读取账号失败: {e:#}")))?;
    let out: Vec<serde_json::Value> = users
        .iter()
        .map(|u| {
            serde_json::json!({
                "id": u.id,
                "username": u.username,
                "role": if u.role == "admin" { "admin" } else { "user" },
                "disabled": u.disabled,
                "must_change_password": u.must_change_password,
                "quota_bytes": u.quota_bytes,
                "created_at": u.created_at,
            })
        })
        .collect();
    ok(serde_json::json!({ "users": out }))
}

// ===================== 邀请码（自助注册，admin） =====================

/// `GET /api/admin/invites`：注册开关 + 邀请码列表。**明文回给管理员**（可重复查看、复制再发），
/// 每个码带 `used` 标志与绑定到的账号。
async fn admin_invites(State(state): State<Arc<AppState>>) -> ApiResult {
    let db = state.auth.db();
    let rows = db
        .list_invites()
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("读取邀请码失败: {e:#}")))?;
    let usernames = db
        .list_users()
        .map(|us| us.into_iter().map(|u| (u.id, u.username)).collect::<std::collections::HashMap<_, _>>())
        .unwrap_or_default();
    let now = vtrim_service::db::now_secs();
    let invites: Vec<serde_json::Value> = rows
        .iter()
        .map(|r| {
            serde_json::json!({
                "id": r.id,
                "token": r.token,
                "used": r.used_at.is_some(),
                "note": r.note,
                "created_at": r.created_at,
                "expires_at": r.expires_at,
                "used_at": r.used_at,
                "used_by": r.used_by,
                "used_by_name": r.used_by.and_then(|id| usernames.get(&id).cloned()),
                "created_by_name": r.created_by.and_then(|id| usernames.get(&id).cloned()),
                "revoked": r.revoked,
                "open": r.is_open(now),
            })
        })
        .collect();
    ok(serde_json::json!({
        "registration_enabled": vtrim_core::config::load_global_config().auth.registration_enabled,
        "invites": invites,
    }))
}

/// `POST /api/admin/invites {note?, days?, count?}`：生成邀请码。
/// 明文同时落库 → 之后 `GET` 列表里也能看到（管理员权限，用户要求）。
async fn admin_invite_create(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<super::auth::CurrentUser>,
    Json(body): Json<serde_json::Value>,
) -> ApiResult {
    let count = body.get("count").and_then(|v| v.as_u64()).unwrap_or(1);
    if count == 0 || count > 50 {
        return Err(err(StatusCode::BAD_REQUEST, "count 取值范围 1..=50"));
    }
    let days = body.get("days").and_then(|v| v.as_u64()).map(|d| d as u32);
    let days = vtrim_service::invite::validate_days(days)
        .map_err(|e| err(StatusCode::BAD_REQUEST, format!("{e}")))?;
    let note = body
        .get("note")
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    let db = state.auth.db();
    let expires_at = days.map(|d| vtrim_service::db::now_secs() + (d as i64) * 86400);
    let mut codes = Vec::new();
    for _ in 0..count {
        let code = vtrim_service::auth::new_invite_code();
        db.create_invite(&code, note.as_deref(), Some(user.id), expires_at)
            .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("写入邀请码失败: {e:#}")))?;
        codes.push(code);
    }
    // 有可用码就顺手把总闸打开（与 CLI `vtrim user invite` 一致）
    if let Err(e) = vtrim_service::invite::set_registration_enabled(true) {
        tracing::warn!("开启注册开关失败: {e:#}");
    }
    let _ = db.audit(Some(user.id), "invite_create", &format!("count={count}"));

    ok(serde_json::json!({
        "ok": true,
        "codes": codes,
        "registration_enabled": true,
        "expires_at": expires_at,
    }))
}

/// `DELETE /api/admin/invites/:id`：撤销一个邀请码（已注册的账号不受影响）。
async fn admin_invite_revoke(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<super::auth::CurrentUser>,
    AxumPath(id): AxumPath<i64>,
) -> ApiResult {
    let db = state.auth.db();
    let n = db
        .revoke_invite(id)
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("撤销失败: {e:#}")))?;
    if n == 0 {
        return Err(err(StatusCode::NOT_FOUND, "邀请码不存在或已是撤销状态"));
    }
    let _ = db.audit(Some(user.id), "invite_revoke", &format!("id={id}"));
    ok(serde_json::json!({ "ok": true }))
}

// ===================== 运行日志（仅 admin） =====================

#[derive(Debug, serde::Deserialize)]
struct LogsQuery {
    /// 游标：只要 `seq > after` 的记录（SSE 断线重连补齐用）
    after: Option<u64>,
    limit: Option<usize>,
}

/// `GET /api/admin/logs`：拉最近的服务端日志（面板首次打开 / 重连补齐）。
///
/// 服务端日志含其他用户的工程路径与任务输出，因此**仅 admin**（与 `/api/admin/*` 一致）。
async fn admin_logs(Query(q): Query<LogsQuery>) -> ApiResult {
    let limit = q.limit.unwrap_or(200).clamp(1, 500);
    let bus = logbus::bus();
    let (records, latest) = bus.snapshot(q.after.unwrap_or(0), limit);
    ok(serde_json::json!({ "records": records, "latest": latest }))
}

#[derive(serde::Deserialize)]
struct UsageQuery {
    days: Option<i64>,
}

/// `GET /api/admin/usage?days=30`：LLM 用量汇总（总量 + 按天/链路/项目/模型）。
///
/// **仅 admin**：记录里带各用户的项目路径，与 `/api/admin/logs` 同一个理由。
/// 采集点在 `vtrim_core::llm::LlmClient::send`，见 `vtrim_service::usage`。
async fn admin_usage(State(state): State<Arc<AppState>>, Query(q): Query<UsageQuery>) -> ApiResult {
    let days = q.days.unwrap_or(30);
    let db = state.auth.db();
    let summary = vtrim_service::usage::summarize(db, days)
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("读取用量失败: {e:#}")))?;
    ok(serde_json::json!(summary))
}

/// `GET /api/admin/logs/events?after=<seq>`：实时日志流（SSE）。
///
/// 先回放环形缓冲里 `seq > after` 的（补上建立连接前的那几秒），再接实时。
async fn admin_logs_stream(Query(q): Query<LogsQuery>) -> Response {
    let after = q.after.unwrap_or(0);
    let (history, rx) = logbus::bus().subscribe();
    let replayed = futures_util::stream::iter(
        history
            .into_iter()
            .filter(move |r| r.seq > after)
            .map(|r| log_sse(&r)),
    );
    let stream: std::pin::Pin<Box<dyn Stream<Item = Result<Event, axum::Error>> + Send>> =
        Box::pin(replayed.chain(log_live_stream(rx)).map(Ok));
    Sse::new(stream)
        .keep_alive(KeepAlive::new().interval(Duration::from_secs(15)))
        .into_response()
}

fn log_sse(rec: &logbus::LogRecord) -> Event {
    Event::default()
        .event("message")
        .data(serde_json::to_string(rec).unwrap_or_else(|_| "{}".into()))
}

/// 实时日志流。`Lagged` = 订阅者太慢丢了若干条，**不能**报成错误
/// （与任务事件流同理），合成一条 warn 让面板自己显示"跳过 N 条"。
fn log_live_stream(
    rx: tokio::sync::broadcast::Receiver<logbus::LogRecord>,
) -> impl Stream<Item = Event> + Send + 'static {
    BroadcastStream::new(rx).filter_map(|item| async move {
        match item {
            Ok(rec) => Some(log_sse(&rec)),
            Err(BroadcastStreamRecvError::Lagged(n)) => {
                let (_, latest) = logbus::bus().snapshot(0, 1);
                Some(log_sse(&logbus::LogRecord {
                    seq: latest,
                    time: chrono::Local::now().format("%H:%M:%S%.3f").to_string(),
                    level: "warn".into(),
                    target: "http".into(),
                    text: format!("日志流落后，已跳过 {n} 条（可刷新面板重新拉取）"),
                }))
            }
        }
    })
}

// ===================== 直播间（录播姬） =====================

/// 当前录播姬对接状态。**给普通用户只说能不能用**（不泄露地址/错误细节）。
fn recorder_client() -> Option<vtrim_service::recorder::RecorderClient> {
    let cfg = vtrim_core::config::load_global_config();
    vtrim_service::recorder::RecorderClient::from_config(&cfg.ingest.recorder)
}

/// `GET /api/rooms`：我的直播间 + 录播姬实时状态。
///
/// `available=false`（未对接 / 连不上）时前端把「配置直播间」置灰 —— 用户不该看到
/// 管理员才懂的那些配置指引。
async fn rooms_list(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<super::auth::CurrentUser>,
) -> ApiResult {
    let db = state.auth.db();
    let rows = db
        .rooms_of_user(user.id)
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("读取直播间失败: {e:#}")))?;

    let all_admin = user.is_admin();
    let (available, reason) = match recorder_client() {
        None => (
            false,
            if all_admin { "未配置录播姬（[ingest.recorder] api_base 为空）" } else { "录播姬未接入" },
        ),
        Some(client) => match client.list_rooms().await {
            Ok(_) => (true, ""),
            Err(e) => {
                tracing::warn!("读录播姬房间列表失败: {e:#}");
                (false, if all_admin { "连不上录播姬（检查 api_base / Basic 账号）" } else { "录播姬暂时不可用" })
            }
        },
    };
    // 在线状态（只有能连上时才查得到；查一次，按 room_id 建索引）
    let live = if available {
        match recorder_client() {
            Some(c) => c.list_rooms().await.unwrap_or_default(),
            None => Vec::new(),
        }
    } else {
        Vec::new()
    };

    // 录像目录汇总：每行显示"已录 N 个 · 大小"（展开时再列明细，见 /api/rooms/:id/recordings）。
    // 只读 walk，网络盘上可能要几秒 → blocking 池；失败只 warn，不让用户连房间列表都看不到。
    let cfg = vtrim_core::config::load_global_config();
    let summaries = if cfg.ingest.recorder.root.trim().is_empty() {
        None
    } else {
        match tokio::task::spawn_blocking(move || crate::server::ingest_worker::list_recordings(&cfg)).await {
            Ok(Ok(files)) => Some(crate::server::ingest_worker::room_summaries(&files)),
            Ok(Err(e)) => {
                tracing::warn!("读录像目录失败: {e:#}");
                None
            }
            Err(e) => {
                tracing::warn!("读录像目录线程失败: {e}");
                None
            }
        }
    };

    let rooms: Vec<serde_json::Value> = rows
        .iter()
        .map(|r| {
            let st = live.iter().find(|x| x.room_id.to_string() == r.room_id);
            let rec = summaries.as_ref().and_then(|m| m.get(&r.room_id)).copied();
            serde_json::json!({
                "room_id": r.room_id,
                "title": if r.title.is_empty() { st.map(|x| x.title.clone()).unwrap_or_default() } else { r.title.clone() },
                "enabled": r.enabled,
                "auto_pipeline": r.auto_pipeline,
                "source": r.source,
                "streaming": st.map(|x| x.streaming).unwrap_or(false),
                "recording": st.map(|x| x.recording).unwrap_or(false),
                "in_recorder": st.is_some(),
                // 录像目录里属于这个房间的文件（解析得出房间号才算得进来）
                "recordings": {
                    "files": rec.map(|s| s.files).unwrap_or(0),
                    "bytes": rec.map(|s| s.bytes).unwrap_or(0),
                    "latest": rec.map(|s| s.latest).unwrap_or(0),
                },
            })
        })
        .collect();
    if all_admin {
        ok(serde_json::json!({ "available": available, "reason": reason, "rooms": rooms }))
    } else {
        ok(serde_json::json!({ "available": available, "rooms": rooms }))
    }
}

/// `POST /api/rooms {url|room_id}`：把直播间加到录播姬并登记归属本人。
///
/// 这是用户端唯一的操作：填房间号/链接 → 录播姬开播即录 → 录完 webhook 按 `channel_map`
/// 归属自动入库成他的项目。
async fn rooms_add(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<super::auth::CurrentUser>,
    Json(body): Json<serde_json::Value>,
) -> ApiResult {
    let input = body
        .get("url")
        .or_else(|| body.get("room_id"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let room_id = vtrim_service::recorder::parse_room_id(&input)
        .map_err(|e| err(StatusCode::BAD_REQUEST, format!("{e}")))?;

    let Some(client) = recorder_client() else {
        return Err(err(
            StatusCode::SERVICE_UNAVAILABLE,
            if user.is_admin() {
                "未配置录播姬：把 [ingest.recorder] api_base 指向录播姬（例 http://127.0.0.1:2356）"
            } else {
                "录播姬未接入，请联系管理员"
            },
        ));
    };
    // 别人的房间不能被顶掉：先查归属
    let db = state.auth.db();
    if let Some(existing) = db
        .channel_for_room(&room_id.to_string())
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("查询归属失败: {e:#}")))?
    {
        if existing.user_id != user.id && !user.is_admin() {
            return Err(err(StatusCode::CONFLICT, "这个直播间已被其他用户登记"));
        }
    }

    let room = client
        .add_room(room_id)
        .await
        .map_err(|e| err(StatusCode::BAD_GATEWAY, format!("{e:#}")))?;
    let title = if room.title.is_empty() { String::new() } else { room.title.clone() };
    db.room_upsert(&room_id.to_string(), user.id, &title)
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("登记直播间失败: {e:#}")))?;
    let _ = db.audit(Some(user.id), "room_add", &format!("room={room_id}"));

    ok(serde_json::json!({
        "ok": true,
        "room": {
            "room_id": room_id.to_string(),
            "title": title,
            "streaming": room.streaming,
            "recording": room.recording,
            "in_recorder": true,
        }
    }))
}

/// `DELETE /api/rooms/:room_id`：从录播姬移除 + 注销登记（录像文件保留）。
///
/// 录播姬那边删失败就**不动登记** —— 否则会留下"还在录但没有归属"的房间。
async fn rooms_delete(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<super::auth::CurrentUser>,
    AxumPath(room_id): AxumPath<String>,
) -> ApiResult {
    let db = state.auth.db();
    let room_id = room_id.trim().to_string();
    let owned = db
        .channel_for_room(&room_id)
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("查询归属失败: {e:#}")))?
        .ok_or_else(|| err(StatusCode::NOT_FOUND, "直播间未登记"))?;
    if owned.user_id != user.id && !user.is_admin() {
        return Err(err(StatusCode::FORBIDDEN, "这是别人登记的直播间"));
    }

    if let Some(client) = recorder_client() {
        let id = vtrim_service::recorder::parse_room_id(&room_id)
            .map_err(|e| err(StatusCode::BAD_REQUEST, format!("{e}")))?;
        client
            .remove_room(id)
            .await
            .map_err(|e| err(StatusCode::BAD_GATEWAY, format!("从录播姬移除失败（登记未改动）：{e:#}")))?;
    }
    let owner_filter = if user.is_admin() { None } else { Some(user.id) };
    db.room_delete(&room_id, owner_filter)
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("注销直播间失败: {e:#}")))?;
    let _ = db.audit(Some(user.id), "room_delete", &format!("room={room_id}"));
    ok(serde_json::json!({ "ok": true }))
}

/// `POST /api/admin/recorder/test`：对接自检（版本 + 房间数），admin 用来看配置对不对。
async fn admin_recorder_test() -> ApiResult {
    let Some(client) = recorder_client() else {
        return ok(serde_json::json!({
            "ok": false,
            "error": "未配置 [ingest.recorder] api_base",
        }));
    };
    let version = client.version().await.ok();
    match client.list_rooms().await {
        Ok(rooms) => ok(serde_json::json!({
            "ok": true,
            "base": client.base(),
            "version": version,
            "rooms": rooms.len(),
        })),
        Err(e) => ok(serde_json::json!({
            "ok": false,
            "base": client.base(),
            "version": version,
            "error": format!("{e:#}"),
        })),
    }
}

/// `PUT /api/admin/invites/registration {enabled}`：注册总闸（关掉后不再受理注册请求）。
async fn admin_registration_put(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<super::auth::CurrentUser>,
    Json(body): Json<serde_json::Value>,
) -> ApiResult {
    let enabled = body
        .get("enabled")
        .and_then(|v| v.as_bool())
        .ok_or_else(|| err(StatusCode::BAD_REQUEST, "缺少 enabled"))?;
    vtrim_service::invite::set_registration_enabled(enabled)
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("写入配置失败: {e:#}")))?;
    let _ = state.auth.db().audit(Some(user.id), "registration_switch", if enabled { "on" } else { "off" });
    ok(serde_json::json!({ "ok": true, "registration_enabled": enabled }))
}

async fn models_get() -> ApiResult {
    // 判定与运行时同源（`download::model_install`：配置目录 → 用户目录 → 程序目录 + marker），
    // 列表也来自注册表 `model_names()` —— 别再手写模型数组，也别只看程序目录：
    // 曾经界面按"<exe>/.vtrim/models 有没有这个目录"判，而 qwen3-asr 放在用户目录
    // `~/.vtrim/models/qwen3-asr`，于是显示"未下载"、实际转写一直在用它。
    let asr = vtrim_core::config::load_global_config().asr;
    let out: Vec<serde_json::Value> = vtrim_core::download::model_names().iter().map(|name| {
        let st = vtrim_core::download::model_install(&asr, name);
        serde_json::json!({
            "name": name,
            "installed": st.installed,
            "dir": vtrim_core::path::display(&st.dir),
            "source": st.source.as_str(),
            "source_label": st.source.label(),
            "size_label": match *name { "sensevoice"=>"165 MB", _=>"879 MB" },
            "note": match *name { "sensevoice"=>"快", _=>"最准最慢" },
        })
    }).collect();
    ok(serde_json::json!(out))
}

fn config_get_impl(ws: &Workspace) -> ApiResult {
    let vtrim_raw = std::fs::read_to_string(&ws.config_file).unwrap_or_default();
    let home = vtrim_core::path::vtrim_home();
    let global_path = home.join("config.toml");
    let global_raw = std::fs::read_to_string(&global_path).unwrap_or_default();
    let llm_path = home.join("llm.toml");
    let llm_raw = std::fs::read_to_string(&llm_path).unwrap_or_default();
    let cfg = ws.load_config();
    // 设置页表单编辑的是**程序配置**，必须用 global-only 的值：
    // merged config 会被当前工程的 vtrim.toml 覆盖，直接用会出现「表单值和源文件对不上」
    let global_cfg = vtrim_core::config::load_global_config();
    let llm_cfg = vtrim_core::config::load_llm_config();
    let (cfg_value, config_error) = match cfg {
        Ok(c) => (serde_json::to_value(c).unwrap_or_default(), String::new()),
        // 配置解析失败时配置会被整体忽略，必须让前端能显示原因，否则表单只会静默回退到默认值
        Err(e) => (serde_json::Value::Null, format!("{e:#}")),
    };
    ok(serde_json::json!({
        "config_file": vtrim_core::path::display(&ws.config_file),
        "root_dir": vtrim_core::path::display(&ws.root_dir),
        "vtrim_raw": vtrim_raw,
        "global_raw": global_raw,
        "llm_raw": llm_raw,
        "config": cfg_value,
        "config_error": config_error,
        "global_config": serde_json::to_value(global_cfg).unwrap_or_default(),
        "llm_config": serde_json::to_value(llm_cfg).unwrap_or_default(),
    }))
}

async fn config_get(State(state): State<Arc<AppState>>) -> ApiResult {
    let ws = workspace_from_state(&state).await;
    config_get_impl(&ws)
}

/// 项目配置里会被当作**路径**的键：`(section, key)`。
///
/// ⚠️ 这些值会一路进 `Workspace::resolve`（`service/workspace.rs`），而它对**绝对路径原样放行**
/// （第 49-56 行），所以这里不校验就等于「普通用户把 `input_file` 指到别人的录像（转写完把
/// 字幕读出来）、把 `output_dir` 指到任意目录（产物写到项目外）」。**新增路径键时同步加进来。**
const PROJECT_PATH_KEYS: [(&str, &str); 6] = [
    ("project", "input_file"),
    ("project", "danmaku_file"),
    ("project", "transcript_file"),
    ("project", "audio_file"),
    ("project", "events_file"),
    ("cutting", "output_dir"),
];

/// 校验配置里的路径键必须落在本项目内（**只对非 admin 生效**）。
///
/// admin 不受此限：用「服务器文件」建项目时 `input_file` 本来就是项目外的绝对路径
/// （`service/projects.rs` 对 admin 的绝对路径不做租户根限制），卡住它会打断既有工作流；
/// 而 admin 已经能浏览服务器目录、在任意路径建项目，这里收紧并不增加实际安全性。
fn check_project_paths(ws: &Workspace, value: &serde_json::Value, is_admin: bool) -> Result<(), AppError> {
    if is_admin {
        return Ok(());
    }
    for (section, key) in PROJECT_PATH_KEYS {
        let Some(raw) = value
            .get(section)
            .and_then(|s| s.get(key))
            .and_then(|v| v.as_str())
        else {
            continue;
        };
        let raw = raw.trim();
        if raw.is_empty() {
            continue;
        }
        // `jail_join` 同时挡住词法越界（绝对路径 / 盘符 / UNC / `..` 段）与符号链接越界
        vtrim_core::tenant::jail_join(&ws.root_dir, raw).map_err(|e| {
            err(
                StatusCode::BAD_REQUEST,
                format!("[{section}] {key} 必须是项目内相对路径（{raw}）: {e:#}"),
            )
        })?;
    }
    Ok(())
}

fn config_vtrim_put_impl(ws: &Workspace, body: &serde_json::Value, is_admin: bool) -> ApiResult {
    // patch 模式：`{ section: { key: value } }` 合并写（保留注释与其他 section），
    // 前端设置表单用它，避免"读整份 TOML → 正则替换 → 写回"这种易碎做法。
    if let Some(patch) = body.get("patch") {
        check_project_paths(ws, patch, is_admin)?;
        vtrim_core::config::patch_config_file(&ws.config_file, patch)
            .map_err(|e| err(StatusCode::BAD_REQUEST, format!("项目配置保存失败: {e:#}")))?;
        return ok(serde_json::json!({ "ok": true }));
    }
    let raw = body.get("raw").and_then(|v| v.as_str()).or_else(|| body.get("content").and_then(|v| v.as_str()));
    let Some(raw) = raw else { return Err(err(StatusCode::BAD_REQUEST, "缺少 raw 字段")); };
    // 与 config_global_put / config_llm_put 对齐：空内容或语法错误会把整个项目配置
    // 变成不可解析（= 配置被整体忽略），必须在落盘前拦下。
    if raw.trim().is_empty() {
        return Err(err(StatusCode::BAD_REQUEST, "内容为空，未保存"));
    }
    vtrim_service::validate_toml(raw)
        .map_err(|e| err(StatusCode::BAD_REQUEST, format!("vtrim.toml 不是合法 TOML，未保存: {e}")))?;
    // 源文件模式同样要过路径校验：TOML 解析出来的结构与 JSON 一致，直接复用上面那张键表
    if !is_admin {
        let parsed: toml::Value = toml::from_str(raw)
            .map_err(|e| err(StatusCode::BAD_REQUEST, format!("vtrim.toml 解析失败，未保存: {e}")))?;
        let as_json = serde_json::to_value(&parsed).unwrap_or(serde_json::Value::Null);
        check_project_paths(ws, &as_json, is_admin)?;
    }
    if let Some(parent) = ws.config_file.parent() { std::fs::create_dir_all(parent).map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("创建目录失败: {e}")))?; }
    std::fs::write(&ws.config_file, raw).map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("保存失败: {e}")))?;
    ok(serde_json::json!({ "ok": true }))
}

/// 保存程序配置。
///
/// - `raw`：源文件模式，整份覆盖写；
/// - `patch`：表单模式，`{ section: { key: value } }` 合并写，未涉及的 section/注释原样保留。
async fn config_global_put(State(_state): State<Arc<AppState>>, Json(body): Json<serde_json::Value>) -> ApiResult {
    let path = vtrim_core::path::vtrim_home().join("config.toml");
    // patch / 源文件两种写法都会读改写整份配置，加锁避免并发提交互相覆盖；
    // 锁在 await 里持有，读改写下 blocking 池（`with_path_lock` 的闭包跑在当前线程）
    let _guard = vtrim_core::filelock::lock_path(&path).await;
    blocking(move || config_global_put_locked(&path, &body)).await
}

fn config_global_put_locked(path: &Path, body: &serde_json::Value) -> ApiResult {
    if let Some(patch) = body.get("patch") {
        vtrim_core::config::patch_config_file(path, patch)
            .map_err(|e| err(StatusCode::BAD_REQUEST, format!("配置保存失败: {e:#}")))?;
        return ok(serde_json::json!({ "ok": true }));
    }
    let raw = body.get("raw").and_then(|v| v.as_str()).or_else(|| body.get("content").and_then(|v| v.as_str()));
    let Some(raw) = raw else { return Err(err(StatusCode::BAD_REQUEST, "缺少 raw 或 patch 字段")); };
    // 语法错误会令整份配置被忽略，这里直接拦下，别让用户存进一个坏文件
    vtrim_service::validate_toml(raw)
        .map_err(|e| err(StatusCode::BAD_REQUEST, format!("config.toml 不是合法 TOML，未保存: {e}")))?;
    if let Some(parent) = path.parent() { std::fs::create_dir_all(parent).map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("创建目录失败: {e}")))?; }
    std::fs::write(path, raw).map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("保存失败: {e}")))?;
    ok(serde_json::json!({ "ok": true }))
}

/// 保存 LLM 配置：`raw` 整份覆盖写，`patch` 合并写（保留注释与其他键）。
async fn config_llm_put(State(_state): State<Arc<AppState>>, Json(body): Json<serde_json::Value>) -> ApiResult {
    let path = vtrim_core::path::vtrim_home().join("llm.toml");
    let _guard = vtrim_core::filelock::lock_path(&path).await;
    blocking(move || config_llm_put_locked(&path, &body)).await
}

fn config_llm_put_locked(path: &Path, body: &serde_json::Value) -> ApiResult {
    if let Some(patch) = body.get("patch") {
        vtrim_core::config::patch_config_file(path, patch)
            .map_err(|e| err(StatusCode::BAD_REQUEST, format!("LLM 配置保存失败: {e:#}")))?;
        return ok(serde_json::json!({ "ok": true }));
    }
    let raw = body.get("raw").and_then(|v| v.as_str()).or_else(|| body.get("content").and_then(|v| v.as_str()));
    let Some(raw) = raw else { return Err(err(StatusCode::BAD_REQUEST, "缺少 raw 或 patch 字段")); };
    vtrim_service::validate_toml(raw)
        .map_err(|e| err(StatusCode::BAD_REQUEST, format!("llm.toml 不是合法 TOML，未保存: {e}")))?;
    if let Some(parent) = path.parent() { std::fs::create_dir_all(parent).map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("创建目录失败: {e}")))?; }
    std::fs::write(path, raw).map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("保存失败: {e}")))?;
    ok(serde_json::json!({ "ok": true }))
}

async fn llm_test() -> ApiResult {
    let llm = vtrim_core::config::load_llm_config();
    if llm.api_key.is_empty() {
        return Err(err(StatusCode::BAD_REQUEST, "LLM API key 未配置"));
    }
    let start = std::time::Instant::now();
    let test_ok = vtrim_core::detection::test_llm(&llm, "ping").await.is_ok();
    let latency_ms = start.elapsed().as_millis() as u64;
    if test_ok {
        ok(serde_json::json!({ "ok": true, "latency_ms": latency_ms, "model": llm.model }))
    } else {
        Err(err(StatusCode::INTERNAL_SERVER_ERROR, format!("LLM 连接失败 ({latency_ms}ms)")))
    }
}

/// 下载全部 ASR 模型（**仅 admin**：平台级资源，见 §5.3）。
///
/// 前端 `POST /api/admin/models/download` 不带 body（旧实现要求 `Json` 提取器，空 body 会被
/// axum 直接 400，两个模型界面都因此不可用）。这里只做任务编排，不读 body。
async fn models_download(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<super::auth::CurrentUser>,
) -> ApiResult {
    if !user.is_admin() {
        return Err(err(StatusCode::FORBIDDEN, "仅管理员可下载平台模型"));
    }
    // 全局模型目录：与任何会 ensure_models 的转写任务互斥（排队）。
    // 略重档 + 后台优先级：纯网络下载，不该插在用户点按的任务前面。
    let handle = state
        .jobs
        .create(
            user.id,
            JobSpec::new(
                "download_models",
                "下载 ASR 模型",
                vec![super::scope_key::MODELS.to_string()],
                JobClass::Medium,
            )
            .background(),
        )
        .await;
    let id = handle.id.clone();
    let queued = handle.queued;
    let jobs = state.jobs.clone();
    let job_id = id.clone();
    // 下载完要解包（tar.bz2，纯同步 IO）→ 任务线程
    spawn_job(jobs.clone(), job_id.clone(), move || async move {
        if handle.wait_start().await.is_err() {
            jobs.complete(&job_id).await;
            return;
        }
        let result = vtrim_core::download::download_all(None).await;
        if let Err(e) = result {
            jobs.fail(&job_id, format!("{e:#}")).await;
            return;
        }
        jobs.complete(&job_id).await;
    });
    ok(serde_json::json!({ "ok": true, "id": id, "queued": queued }))
}

/// 打开/新建项目后把"当前工程"切到它：`/api/health`、`/api/fs/list` 的默认目录与
/// `/api/media` 的相对路径解析都依赖这份状态。
async fn activate_workspace(state: &AppState, ws: &Workspace) {
    state.set_workspace(ws.clone()).await;
    persist::remember_last(&ws.config_file, Some(&state.workspace_root().await));
}

/// 旧版项目摘要（`server.toml` 时代的列表形态）。
///
/// P2 起项目列表走 DB（`/api/projects`），这个函数只在过渡期别名的 stub 里出现，
/// 保留是为了让 `vtrim status` 与旧前端 JSON 结构继续可用。
#[allow(dead_code)]
fn project_summary(ws: &Workspace) -> serde_json::Value {
    vtrim_service::project_summary(ws)
}

// ---- 项目托管（P2，§5 / §8） ----

/// 取项目并校验归属；他人项目一律 404（不泄露"存在但无权限"）。
fn owned_project(
    db: &vtrim_service::db::Db,
    user: &super::auth::CurrentUser,
    id: i64,
) -> Result<vtrim_service::db::ProjectRow, AppError> {
    match vtrim_service::projects::find_owned(db, user.id, user.is_admin(), id) {
        Ok(Some(p)) => Ok(p),
        Ok(None) => Err(err(StatusCode::NOT_FOUND, "项目不存在或无权访问")),
        Err(e) => Err(err(StatusCode::INTERNAL_SERVER_ERROR, format!("读取项目失败: {e:#}"))),
    }
}

fn owner_name(db: &vtrim_service::db::Db, owner_id: i64) -> Option<String> {
    db.list_users()
        .ok()?
        .into_iter()
        .find(|u| u.id == owner_id)
        .map(|u| u.username)
}

/// `GET /api/projects`：只列本人项目；admin 额外看到所有人的（含 owner 列）。
async fn projects_list(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<super::auth::CurrentUser>,
) -> ApiResult {
    let db = state.auth.db();
    // 始终只回本人项目：admin 的"全平台项目"视图属于 P5 管理页（`/api/admin/*`）
    let rows = db
        .list_projects(Some(user.id))
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("读取项目列表失败: {e:#}")))?;

    // 每个项目的摘要都要读它的 `vtrim.toml` + 对产物目录做十几次 `exists()`
    // （`summary_json` → `project_snapshot`）。项目在网络盘上时这就是十几秒级的往返，
    // 而前端一次页面加载会成串地打这个接口 —— 整段算在 blocking 池里，别占 worker。
    let st = state.clone();
    let out = blocking(move || {
        let db = st.auth.db();
        let out: Vec<serde_json::Value> = rows
            .iter()
            .map(|p| {
                let json = vtrim_service::projects::summary_json(p, owner_name(db, p.owner_id).as_deref());
                // 项目目录/vtrim.toml 被外部删掉时落库标记，列表里显示"缺失"而不是静默消失
                if json["missing"].as_bool() == Some(true) && !p.missing {
                    let _ = db.set_project_missing(p.id, true);
                }
                json
            })
            .collect();
        Ok(out)
    })
    .await?;
    ok(serde_json::json!({ "ok": true, "projects": out }))
}

/// `POST /api/projects`：新建项目。
///
/// 目前只支持 `source.kind = "local"`（**仅 admin**，§0.2.7：单人/本机部署保留
/// "服务器端已有文件"来源，路径囚笼在租户根内，**永不接受上传**）。
/// `recorder`（录播姬入库）在 P3、`bilibili` 在 P4 接入。
async fn projects_create(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<super::auth::CurrentUser>,
    Json(body): Json<serde_json::Value>,
) -> ApiResult {
    let source = body.get("source").cloned().unwrap_or(serde_json::Value::Null);
    let kind = source.get("kind").and_then(|v| v.as_str()).unwrap_or("");
    match kind {
        "local" => {}
        "recorder" | "bilibili" => {
            return Err(err(
                StatusCode::BAD_REQUEST,
                "该来源尚未开放（录播姬在 P3、B站链接在 P4）；当前请用「服务器本机文件」（admin）",
            ))
        }
        other => {
            return Err(err(
                StatusCode::BAD_REQUEST,
                format!("未知来源 kind={other:?}（支持 local）"),
            ))
        }
    }
    if !user.is_admin() {
        return Err(err(StatusCode::FORBIDDEN, "仅管理员可指定服务器本机文件"));
    }
    create_local_project(
        &state,
        &user,
        source.get("root").and_then(|v| v.as_str()).unwrap_or(""),
        source.get("dir").and_then(|v| v.as_str()).unwrap_or(""),
        source.get("input").and_then(|v| v.as_str()).unwrap_or(""),
        source.get("danmaku").and_then(|v| v.as_str()),
        source.get("output_dir").and_then(|v| v.as_str()),
    )
    .await
}

/// 用"服务器本机文件"建项目的公共实现（`POST /api/projects` 与过渡期的
/// `POST /api/project/init` 共用）。
///
/// 管理员可以给**任意绝对目录**（与单用户时代一致），非管理员只能是租户根内的相对路径；
/// 目录里已经存在 `vtrim.toml` 时**只登记/打开，不覆盖**（避免把已有工程的设置冲掉）。
async fn create_local_project(
    state: &Arc<AppState>,
    user: &super::auth::CurrentUser,
    root_hint: &str,
    rel_dir: &str,
    input_hint: &str,
    danmaku_hint: Option<&str>,
    output_dir: Option<&str>,
) -> ApiResult {
    let tenant = vtrim_service::projects::tenant_root(user.id);
    std::fs::create_dir_all(&tenant)
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("创建租户目录失败: {e}")))?;
    let dir = vtrim_service::projects::resolve_target_dir(
        user.id,
        root_hint,
        rel_dir,
        user.is_admin(),
    )
    .map_err(|e| err(StatusCode::BAD_REQUEST, format!("目录不可用: {e:#}")))?;
    if !dir.is_dir() {
        return Err(err(StatusCode::BAD_REQUEST, "只能选择目录"));
    }
    let config_path = dir.join("vtrim.toml");
    let is_existing_project = config_path.is_file();

    // 已有 vtrim.toml：只登记/打开，不重写（否则会覆盖用户手改过的工程设置）
    if !is_existing_project {
        let hint = input_hint.trim();
        let input = if hint.is_empty() {
            // 未指定文件时自动挑目录里第一个视频
            find_first_video(&dir).ok_or_else(|| {
                err(StatusCode::BAD_REQUEST, "目录里没有可用的视频文件（mp4/flv/mkv/ts）")
            })?
        } else {
            vtrim_core::tenant::jail_existing(&dir, hint)
                .map_err(|e| err(StatusCode::BAD_REQUEST, format!("视频不可用: {e:#}")))?
        };
        let danmaku = danmaku_hint
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .map(|rel| {
                vtrim_core::tenant::jail_existing(&dir, &rel)
                    .map_err(|e| err(StatusCode::BAD_REQUEST, format!("弹幕不可用: {e:#}")))
            })
            .transpose()?;
        let output_rel = output_dir.map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
        if let Some(rel) = &output_rel {
            vtrim_core::tenant::jail_join(&dir, rel)
                .map_err(|e| err(StatusCode::BAD_REQUEST, format!("输出目录不可用: {e:#}")))?;
        }
        // `local` 来源按策略**不预载弹幕**（用户要求"自由选择"）：没显式给弹幕就留空，
        // 由预处理页列候选让用户自己挑（`vtrim_service::assets::preload_policy`）。
        // 所以这里用 `_exact`，不做"同目录同名 .ass"自动发现 —— 那条是 `vtrim init` 的语义。
        let auto_danmaku = matches!(
            vtrim_service::assets::preload_policy("local"),
            vtrim_service::assets::Preload::Danmaku
        );
        let write_cfg: fn(&Path, &Path, Option<&Path>, Option<&str>) -> anyhow::Result<()> = if auto_danmaku {
            vtrim_core::config::write_project_config
        } else {
            vtrim_core::config::write_project_config_exact
        };
        write_cfg(&config_path, &input, danmaku.as_deref(), output_rel.as_deref())
            .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("初始化失败: {e:#}")))?;
    }
    let ws = Workspace::load(Some(&config_path))
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("加载失败: {e:#}")))?;

    let db = state.auth.db();
    let name = dir
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "project".to_string());
    let in_tenant = dir.starts_with(&tenant);
    let label = if in_tenant {
        dir.strip_prefix(&tenant)
            .ok()
            .map(|rel| format!("本机文件 · {}", rel.display()))
            .unwrap_or_else(|| "本机文件".to_string())
    } else {
        // 任意位置（管理员）：用父目录名 + 目录名拼一个能认出来的标签
        let parent_name = dir
            .parent()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        format!("本机 · {parent_name}/{}", name)
    };
    let id = match db.project_by_config(&config_path) {
        Ok(Some(existing)) => existing.id,
        _ => db
            .create_project(
                user.id,
                &name,
                None,
                "local",
                Some(&label),
                &dir,
                &config_path,
                None,
            )
            .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("登记项目失败: {e:#}")))?,
    };
    if let Ok(Some(p)) = db.project_by_id(id) {
        let _ = vtrim_service::projects::refresh_bytes(db, &p);
    }
    let _ = db.audit(Some(user.id), "project_create", &format!("id={id} kind=local"));
    activate_workspace(state, &ws).await;
    match db.project_by_id(id) {
        Ok(Some(p)) => ok(serde_json::json!({
            "ok": true,
            "id": id,
            "project": vtrim_service::projects::summary_json(&p, Some(&user.username)),
        })),
        _ => Err(err(StatusCode::INTERNAL_SERVER_ERROR, "项目登记后读取失败")),
    }
}

/// 目录里第一个可用视频（mp4/flv/mkv/ts，按文件名排序，确定性）。
fn find_first_video(dir: &Path) -> Option<PathBuf> {
    let mut candidates: Vec<PathBuf> = std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.is_file()
                && p.extension()
                    .and_then(|e| e.to_str())
                    .map(|e| matches!(e.to_ascii_lowercase().as_str(), "mp4" | "flv" | "mkv" | "ts"))
                    .unwrap_or(false)
        })
        .collect();
    candidates.sort();
    candidates.into_iter().next()
}

/// 注销项目绑定：**只动服务器的账本与状态文件，项目目录里一个文件都不碰**。
///
/// 两个入口（`DELETE /api/projects/:id` 与过渡别名 `POST /project/delete`）都走这里 ——
/// 这段逻辑以前是两份拷贝，两边各删一次 `vtrim.toml`；要改就一起改，否则会各处半对。
///
/// ⚠️ `forget_project` 会**同时清掉 `last_config_file`**，这一步不是顺手做的：
/// 启动流程每次都会把"当前工程"重新写回 `projects[]`（`server/mod.rs`），
/// 而 `import_legacy_projects` 会把 `projects[]` 里"文件还在、DB 没行"的路径重新登记 ——
/// 光删 DB 行的话，移除过的项目会在**第二次重启**时自己回来（详见 `persist::forget_project`）。
///
/// 媒体文件与 `output/` 产物一律保留（那是「删除源录播」那条独立路径的事）。
fn unregister_project(
    db: &vtrim_service::db::Db,
    user_id: i64,
    project: &vtrim_service::db::ProjectRow,
) -> ApiResult {
    db.delete_project(project.id)
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("注销失败: {e:#}")))?;
    persist::forget_project(&project.config_file);
    let _ = db.audit(Some(user_id), "project_unregister", &format!("id={}", project.id));
    ok(serde_json::json!({ "ok": true, "deleted": project.id }))
}

/// `DELETE /api/projects/:id`：**只注销登记**，项目目录里零文件改动（媒体/产物/`vtrim.toml` 全保留）。
///
/// 想再挂回来就重新导入同一个目录（`POST /api/projects`）—— 目录里已有 `vtrim.toml` 时
/// 只登记、不覆盖（见 `create_local_project`），所以手改过的工程设置不会丢。
async fn projects_delete(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<super::auth::CurrentUser>,
    AxumPath(id): AxumPath<String>,
) -> ApiResult {
    let id = vtrim_service::projects::parse_project_id(&id)
        .ok_or_else(|| err(StatusCode::BAD_REQUEST, "项目 id 无效"))?;
    let db = state.auth.db();
    let project = owned_project(db, &user, id)?;
    unregister_project(db, user.id, &project)
}

/// `DELETE /api/projects/:id/source`：删源录播（**唯一的手动释放空间手段**，§7）。
///
/// 产物（切片/字幕/事件/封面）与项目本身都保留；只删 `input_file` 与根目录下的
/// 分片/弹幕文件，并刷新占用。
async fn projects_source_delete(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<super::auth::CurrentUser>,
    AxumPath(id): AxumPath<String>,
) -> ApiResult {
    let id = vtrim_service::projects::parse_project_id(&id)
        .ok_or_else(|| err(StatusCode::BAD_REQUEST, "项目 id 无效"))?;
    let db = state.auth.db();
    let project = owned_project(db, &user, id)?;

    let mut removed: Vec<String> = Vec::new();
    let mut freed = 0u64;
    let ws = Workspace::load(Some(&project.config_file)).ok();
    let mut targets: Vec<PathBuf> = Vec::new();
    if let Some(ws) = &ws {
        if let Ok(cfg) = ws.load_config() {
            for raw in [cfg.project.input_file.clone().unwrap_or_default(), cfg.project.danmaku_file.clone().unwrap_or_default()] {
                if !raw.trim().is_empty() {
                    targets.push(ws.resolve(&raw));
                }
            }
        }
    }
    // 根目录下的分片（录播姬多分片会话）也算源
    if let Ok(entries) = std::fs::read_dir(&project.root_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
            if matches!(ext.as_str(), "mp4" | "flv" | "mkv" | "ts") {
                targets.push(path);
            }
        }
    }
    // 去重后逐个删除（只删项目根目录内的文件）
    let mut seen: std::collections::HashSet<PathBuf> = std::collections::HashSet::new();
    let root = project.root_dir.canonicalize().unwrap_or_else(|_| project.root_dir.clone());
    for target in targets {
        let real = target.canonicalize().unwrap_or_else(|_| target.clone());
        if !seen.insert(real.clone()) {
            continue;
        }
        if !real.starts_with(&root) {
            tracing::warn!("跳过项目目录外的源文件: {}", real.display());
            continue;
        }
        if !real.is_file() {
            continue;
        }
        let size = std::fs::metadata(&real).map(|m| m.len()).unwrap_or(0);
        match std::fs::remove_file(&real) {
            Ok(()) => {
                freed += size;
                removed.push(real.to_string_lossy().into_owned());
            }
            Err(e) => tracing::warn!("删除源文件失败 {}: {e}", real.display()),
        }
    }
    let (source_bytes, output_bytes) = vtrim_service::projects::refresh_bytes(db, &project)
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("刷新占用失败: {e:#}")))?;
    let _ = db.audit(
        Some(user.id),
        "project_source_delete",
        &format!("id={id} files={} bytes={freed}", removed.len()),
    );
    ok(serde_json::json!({
        "ok": true,
        "removed": removed,
        "freed_bytes": freed,
        "source_bytes": source_bytes,
        "output_bytes": output_bytes,
    }))
}

/// **过渡期别名**：旧前端按 `config_path` 注销项目；只允许操作**已登记且属于自己**的
/// `vtrim.toml`（P2 之后前端改用 `DELETE /api/projects/:id`）。
///
/// 与 `DELETE /api/projects/:id` **是同一件事**，共用 `unregister_project`
/// —— 别再在这里单独写一份"删除"（以前两份拷贝各删一次 `vtrim.toml`）。
async fn project_delete(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<super::auth::CurrentUser>,
    Json(body): Json<serde_json::Value>,
) -> ApiResult {
    let raw = body
        .get("config_path")
        .and_then(|v| v.as_str())
        .ok_or_else(|| err(StatusCode::BAD_REQUEST, "缺少 config_path"))?;
    let path = PathBuf::from(raw);
    let is_project_file = path
        .file_name()
        .and_then(|n| n.to_str())
        .map(|n| n.eq_ignore_ascii_case("vtrim.toml"))
        .unwrap_or(false);
    if !is_project_file {
        return Err(err(StatusCode::BAD_REQUEST, "只允许操作 vtrim.toml"));
    }
    let db = state.auth.db();
    let project = db
        .project_by_config(&path)
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("读取项目失败: {e:#}")))?
        .filter(|p| user.is_admin() || p.owner_id == user.id)
        .ok_or_else(|| err(StatusCode::NOT_FOUND, "项目不存在或无权访问"))?;
    unregister_project(db, user.id, &project)
}

/// **过渡期别名**：旧前端的项目列表（`/api/projects` 的早期形态）。
/// 现在直接读 DB 里**自己的**项目，不再扫描磁盘或读取 `server.toml`。
async fn project_list(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<super::auth::CurrentUser>,
) -> ApiResult {
    let root = state.workspace_root().await;
    let db = state.auth.db();
    let rows = db
        .list_projects(Some(user.id))
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("读取项目列表失败: {e:#}")))?;
    // 同 `/api/projects`：摘要是对项目目录的同步 IO，下 blocking 池
    let user_name = user.username.clone();
    let out = blocking(move || {
        Ok(rows
            .iter()
            .map(|p| vtrim_service::projects::summary_json(p, Some(&user_name)))
            .collect::<Vec<serde_json::Value>>())
    })
    .await?;
    ok(serde_json::json!({
        "workspace_root": root.to_string_lossy(),
        "projects": out,
        "deprecated": "请改用 GET /api/projects（本项目列表只回本人项目）",
    }))
}







fn project_snapshot(ws: &Workspace) -> serde_json::Value {
    vtrim_service::project_snapshot(ws)
}

fn project_status_impl(ws: &Workspace) -> ApiResult {
    ok(project_snapshot(ws))
}

/// **过渡期别名**（P2 之后前端改用 `POST /api/projects {source:{kind:"local"}}`）：
/// 旧前端传的是绝对 `input`/`config_path`，这里把它当成"相对租户根"的提示，
/// 走同一条囚笼 + 登记流程，**不再接受任意绝对路径**。
async fn project_init(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<super::auth::CurrentUser>,
    Json(body): Json<serde_json::Value>,
) -> ApiResult {
    if !user.is_admin() {
        return Err(err(StatusCode::FORBIDDEN, "仅管理员可指定服务器本机文件"));
    }
    let raw = body
        .get("input")
        .or_else(|| body.get("config_path"))
        .or_else(|| body.get("dir"))
        .and_then(|v| v.as_str())
        .ok_or_else(|| err(StatusCode::BAD_REQUEST, "缺少 input"))?;
    let tenant = vtrim_service::projects::tenant_root(user.id);
    let abs = PathBuf::from(raw);
    // 旧值可能是绝对路径：只接受"落在自己租户根内"的那种，其余按相对路径处理
    let rel_dir = if abs.is_absolute() {
        let parent = if abs.is_dir() { abs.clone() } else { abs.parent().map(|p| p.to_path_buf()).unwrap_or_else(|| tenant.clone()) };
        match vtrim_core::tenant::ensure_within(&tenant, &parent) {
            Ok(inside) => inside
                .strip_prefix(&tenant)
                .map(|p| p.to_string_lossy().replace('\\', "/"))
                .unwrap_or_default(),
            Err(_) => raw.to_string(),
        }
    } else {
        raw.to_string()
    };
    let danmaku = body.get("danmaku").and_then(|v| v.as_str());
    let output = body.get("output_dir").and_then(|v| v.as_str());
    let created = create_local_project(&state, &user, "", &rel_dir, "", danmaku, output).await?;
    // 旧前端期待项目快照（含 current_stage / files）
    let ws = match created.0.get("id").and_then(|v| v.as_i64()).and_then(|id| state.auth.db().project_by_id(id).ok().flatten()) {
        Some(p) => Workspace::load(Some(&p.config_file))
            .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("加载失败: {e:#}")))?,
        None => return Err(err(StatusCode::INTERNAL_SERVER_ERROR, "项目登记后读取失败")),
    };
    ok(project_snapshot(&ws))
}

/// **过渡期别名**：按 `config_path`（已登记的本人项目）激活工作区并回项目快照。
async fn project_open(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<super::auth::CurrentUser>,
    Json(body): Json<serde_json::Value>,
) -> ApiResult {
    let raw = body.get("config_path").and_then(|v| v.as_str()).unwrap_or("").trim().to_string();
    let by_id = body.get("project").and_then(|v| v.as_i64());
    let project = match (by_id, raw.is_empty()) {
        (Some(id), _) => vtrim_service::projects::find_owned(state.auth.db(), user.id, user.is_admin(), id)
            .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("读取项目失败: {e:#}")))?,
        (None, false) => state
            .auth
            .db()
            .project_by_config(Path::new(&raw))
            .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("读取项目失败: {e:#}")))?
            .filter(|p| user.is_admin() || p.owner_id == user.id),
        _ => None,
    };
    let Some(project) = project else {
        return Err(err(
            StatusCode::NOT_FOUND,
            "项目不存在或无权访问（请用 /api/projects 的项目 id）",
        ));
    };
    let ws = Workspace::load(Some(&project.config_file))
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("加载失败: {e:#}")))?;
    activate_workspace(&state, &ws).await;
    ok(project_snapshot(&ws))
}

// ---- 项目数据（P2：作用域 = 本人项目 id，`?config=` 仅作过渡期反查） ----
//
// `Scope` 提取器从 `?project=<id>`（或过渡期的 `?config=<已登记本人项目>`）解析出项目与
// `Workspace`；**未登记的路径一律 404**，所以"任意路径"这条入口已经关闭。

// 以下都是"读一个项目的产物目录"的接口：目录可能在网络盘上，解析与统计一律下 blocking 池
async fn project_data_status(scope: Scope) -> ApiResult {
    let ws = scope.ws;
    blocking(move || project_status_impl(&ws)).await
}

async fn project_data_events(scope: Scope) -> ApiResult {
    let ws = scope.ws;
    blocking(move || data_events_impl(&ws)).await
}

async fn project_data_clips(scope: Scope) -> ApiResult {
    let ws = scope.ws;
    blocking(move || data_clips_impl(&ws)).await
}

async fn project_data_clip_thumb(scope: Scope, AxumPath(rank): AxumPath<String>) -> Response {
    let ws = scope.ws;
    // 路径参数可以是数字 rank，也可以是目录名（`clip-out`）—— 卡片墙按目录名请求缩略图。
    // 解析 + 两次 exists 都在项目目录上（网络盘上就是两次往返），下 blocking 池；
    // 卡片墙一次会打出几十个缩略图请求，留在 worker 上就是几十次白占。
    let rank_dir = rank.clone();
    let resolved = blocking(move || resolve_clip_dir(&ws.output_dir(), &rank_dir)).await;
    let clip_dir = match resolved {
        Ok(p) => p,
        Err(e) => return e.into_response(),
    };
    let video = clip_dir.join("clip.mp4");
    let thumb = clip_dir.join("_thumb.jpg");
    let (v2, t2) = (video.clone(), thumb.clone());
    let (has_thumb, has_video) = match blocking(move || Ok((t2.exists(), v2.exists()))).await {
        Ok(v) => v,
        Err(e) => return e.into_response(),
    };
    if !has_thumb && has_video {
        // ffmpeg 是阻塞调用；同一张图用路径锁串行化，后到者拿到锁后重查即可跳过重复生成
        let _guard = vtrim_core::filelock::lock_path(&thumb).await;
        if !thumb.exists() {
            let v = video.clone();
            let t = thumb.clone();
            match tokio::task::spawn_blocking(move || generate_clip_thumb(&v, &t)).await {
                Ok(Ok(())) => {}
                Ok(Err(e)) => tracing::warn!("生成缩略图失败 ({}): {e}", thumb.display()),
                Err(e) => tracing::warn!("生成缩略图任务失败 ({}): {e}", thumb.display()),
            }
        }
    }
    match tokio::fs::read(&thumb).await {
        Ok(bytes) => ([(axum::http::header::CONTENT_TYPE, "image/jpeg")], bytes).into_response(),
        Err(_) => media::json_error(StatusCode::NOT_FOUND, "缩略图不存在"),
    }
}

/// 截取切片首帧为 `_thumb.jpg`（同步阻塞，调用方负责 spawn_blocking）。
fn generate_clip_thumb(video: &Path, thumb: &Path) -> std::io::Result<()> {
    let status = std::process::Command::new("ffmpeg")
        .args(["-ss","1","-i", &video.to_string_lossy(), "-vframes","1","-q:v","4","-y", &thumb.to_string_lossy()])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()?;
    if !status.success() {
        return Err(std::io::Error::new(std::io::ErrorKind::Other, "ffmpeg 退出码非 0"));
    }
    Ok(())
}

async fn project_data_polish(scope: Scope) -> ApiResult {
    let ws = scope.ws;
    blocking(move || polish_projects_impl(&ws)).await
}

async fn project_data_transcript(scope: Scope) -> ApiResult {
    let ws = scope.ws;
    blocking(move || data_transcript_impl(&ws)).await
}

async fn project_data_transcript_put(scope: Scope, Json(body): Json<serde_json::Value>) -> ApiResult {
    let ws = scope.ws;
    let path = ws.output_dir().join("transcript.json");
    // 锁在 await 里持有，读改写本身下 blocking 池（`with_path_lock` 的闭包是同步的，会把
    // 整段 IO 留在 worker 上）
    let _guard = vtrim_core::filelock::lock_path(&path).await;
    blocking(move || data_transcript_put_impl(&ws, &body)).await
}

async fn project_data_events_create(scope: Scope, Json(body): Json<serde_json::Value>) -> ApiResult {
    let ws = scope.ws;
    // events.json 是 read-modify-write：并发 PUT/POST 会丢事件
    let path = ws.output_dir().join("events.json");
    let _guard = vtrim_core::filelock::lock_path(&path).await;
    blocking(move || data_events_create_impl(&ws, &body)).await
}

async fn project_data_events_put(scope: Scope, Json(body): Json<serde_json::Value>) -> ApiResult {
    let ws = scope.ws;
    let path = ws.output_dir().join("events.json");
    let _guard = vtrim_core::filelock::lock_path(&path).await;
    blocking(move || data_events_put_impl(&ws, &body)).await
}

// 这两个入口在**建任务前**就要读 events.json / 逐个检查 clip.mp4 是否存在（全在网络盘上），
// 所以整段（含排队判定）下 blocking 池；任务体本身由 `spawn_job` 带走。
async fn project_data_event_cut(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<super::auth::CurrentUser>,
    scope: Scope,
    AxumPath(rank): AxumPath<String>,
) -> ApiResult {
    let (ws, st, uid, pid) = (scope.ws, state.clone(), user.id, scope.project.id);
    blocking(move || block_on_here(event_cut_impl(&ws, &rank, &st.jobs, uid, pid))).await
}

async fn project_data_event_cut_all(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<super::auth::CurrentUser>,
    scope: Scope,
    Json(body): Json<serde_json::Value>,
) -> ApiResult {
    let (ws, st, uid, pid) = (scope.ws, state.clone(), user.id, scope.project.id);
    blocking(move || block_on_here(event_cut_all_impl(&ws, &body, &st.jobs, uid, pid))).await
}

/// `POST /api/project-data/events/merge-splits` —— 把历史上被等分的
/// `xxx（1/2）`/`（2/2）` 合回一个事件（前端二次确认后调用）。
async fn project_data_events_merge_splits(scope: Scope, State(state): State<Arc<AppState>>) -> ApiResult {
    let ws = scope.ws;
    let path = ws.output_dir().join("events.json");
    // 同步写 events.json（没法排队）：有任务正在写同批产物时当场拒绝，别对撞
    if let Some(busy) = state
        .jobs
        .busy_with(&[super::scope_key::project_out(scope.project.id)])
        .await
    {
        tracing::info!("拒绝在 {} 上合并拆分事件：{busy}", busy.resource);
        return Err(err(StatusCode::CONFLICT, format!("{busy}（正在读写切片产物），请稍后再试")));
    }
    let _guard = vtrim_core::filelock::lock_path(&path).await;
    blocking(move || data_events_merge_splits_impl(&ws)).await
}

async fn project_data_clip_silence(scope: Scope, State(state): State<Arc<AppState>>, AxumPath(rank): AxumPath<String>) -> ApiResult {
    // 隐式初始化精修工程 + 扫描静音 + 写屏蔽轨道：整段同步 IO，下 blocking 池
    let (ws, st, pid) = (scope.ws, state.clone(), scope.project.id);
    blocking(move || block_on_here(clip_silence_impl(&ws, &rank, &st, pid))).await
}

/// `DELETE /api/project-data/clips/:rank` —— 删除该切片目录（前端二次确认后调用）。
/// 路径参数接受数字 rank（`2` → `clip-02`）或目录名（`clip-out`），见 `resolve_clip_dir`。
async fn project_data_clip_delete(scope: Scope, State(state): State<Arc<AppState>>, AxumPath(rank): AxumPath<String>) -> ApiResult {
    // 删目录 + 重写发布索引都是对项目目录的同步 IO（可能在网络盘上）：整段下 blocking 池
    let (ws, pid) = (scope.ws, scope.project.id);
    blocking(move || block_on_here(clip_delete_impl(&ws, &rank, &state, pid))).await
}

async fn project_data_correct(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<super::auth::CurrentUser>,
    scope: Scope,
) -> ApiResult {
    correct_run_impl(&scope.ws, &state.jobs, user.id, scope.project.id).await
}

async fn project_data_pipeline(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<super::auth::CurrentUser>,
    scope: Scope,
    Json(body): Json<serde_json::Value>,
) -> ApiResult {
    // 作用域由 `Scope` 解析（`?project=` / 过渡期 `?config=`）；
    // 旧前端还会在 body 里带 `config`，那只是冗余字段，不再作为作用域来源。
    let ws = scope.ws;
    if let Some(stage) = body.get("stage").and_then(|v| v.as_str()) {
        // 转写一生只跑一次：工程里已经有转写（ASR 产物或用户选的外部字幕）就不再提供重跑入口。
        // 想换字幕走「选择字幕文件」（`POST /api/project-data/assets`）。
        // ⚠️ 只拦 HTTP 这一侧：CLI `vtrim transcribe` 直接调 `core_pipeline::run_stage`，
        // 仍然可以强制重跑（转写中途崩掉、只留了半份产物时它是逃生口）。
        if stage == "transcribe" {
            let cfg = ws.load_config().map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}")))?;
            if ws.transcript_path(&cfg).is_some() {
                return Err(err(
                    StatusCode::CONFLICT,
                    "该工程的转写已完成，不能再重跑；要换字幕请用「选择字幕文件」",
                ));
            }
        }
        let resume = body.get("resume").and_then(|v| v.as_str()).map(|s| s != "restart").unwrap_or(true);
        let stages: Vec<String> = match stage {
            "extract" => vec!["extract"],
            "transcribe" => vec!["transcribe"],
            "detect" => vec!["detect"],
            "cut" => vec!["cut"],
            "process" => vec!["extract","transcribe","detect","cut"],
            _ => return Err(err(StatusCode::BAD_REQUEST, "未知 stage")),
        }.into_iter().map(|s| s.to_string()).collect();
        run_pipeline_job(&state.jobs, user.id, scope.project.id, ws.config_file.clone(), &format!("stage_{}", stage), stages, false, resume).await
    } else {
        let stages = body.get("stages").and_then(|v| v.as_array()).map(|arr| {
            arr.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect::<Vec<_>>()
        }).unwrap_or_else(|| vec!["extract".into(),"transcribe".into(),"detect".into(),"cut".into()]);
        let dry = body.get("dry_run").and_then(|v| v.as_bool()).unwrap_or(false);
        let resume = body.get("resume").and_then(|v| v.as_str()).map(|s| s != "restart").unwrap_or(true);
        run_pipeline_job(&state.jobs, user.id, scope.project.id, ws.config_file.clone(), "pipeline", stages, dry, resume).await
    }
}

async fn project_data_config(scope: Scope) -> ApiResult {
    let ws = scope.ws;
    blocking(move || config_get_impl(&ws)).await
}

/// `GET /api/project-data/assets`：当前弹幕/字幕 + 项目内的候选。
///
/// 候选一次扫完（项目根 + 一层子目录），预处理页打开弹窗即可直接用。
/// 只 stat、不解析文件内容（3 小时录播的弹幕 XML 有几十 MB），所以这里不需要担心耗时。
async fn project_data_assets(scope: Scope) -> ApiResult {
    let ws = scope.ws;
    let source_kind = scope.project.source_kind.clone();
    let source_label = scope.project.source_label.clone().unwrap_or_default();
    let room = scope.project.room.clone();
    blocking(move || assets_impl(&ws, &source_kind, &source_label, room.as_deref())).await
}

/// `POST /api/project-data/assets` `{kind, path}`：选择/清除一类素材。
///
/// 写前**解析一遍**（解析失败不落盘），成功返回条数供前端提示。
async fn project_data_assets_put(
    scope: Scope,
    Extension(user): Extension<super::auth::CurrentUser>,
    Json(body): Json<serde_json::Value>,
) -> ApiResult {
    let ws = scope.ws;
    let is_admin = user.is_admin();
    // 与设置页、去静音等写同一份 vtrim.toml：读改写必须上路径锁
    let path = ws.config_file.clone();
    let _guard = vtrim_core::filelock::lock_path(&path).await;
    blocking(move || assets_put_impl(&ws, &body, is_admin)).await
}

fn assets_impl(
    ws: &Workspace,
    source_kind: &str,
    source_label: &str,
    room: Option<&str>,
) -> ApiResult {
    use vtrim_service::assets::{asset_status, scan_candidates, AssetKind};
    let cfg = ws.load_config().map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}")))?;
    let mut candidates = serde_json::Map::new();
    for kind in [AssetKind::Danmaku, AssetKind::Transcript] {
        let list = scan_candidates(ws, &cfg, kind);
        candidates.insert(
            kind.as_str().to_string(),
            serde_json::to_value(list).unwrap_or(serde_json::Value::Array(vec![])),
        );
    }
    ok(serde_json::json!({
        "danmaku": asset_status(ws, &cfg, AssetKind::Danmaku),
        "transcript": asset_status(ws, &cfg, AssetKind::Transcript),
        "source": { "kind": source_kind, "label": source_label, "room": room },
        "candidates": candidates,
        "root_dir": vtrim_core::path::display(&ws.root_dir),
    }))
}

fn assets_put_impl(ws: &Workspace, body: &serde_json::Value, is_admin: bool) -> ApiResult {
    use vtrim_service::assets::{asset_status, set_asset, AssetKind};
    let kind_raw = body.get("kind").and_then(|v| v.as_str()).unwrap_or("");
    let kind = AssetKind::parse(kind_raw)
        .map_err(|e| err(StatusCode::BAD_REQUEST, format!("{e:#}")))?;
    // 两种提交形态：`{path}`（项目内相对 / admin 的绝对路径），
    // 或 `{dir, name}`（服务器目录浏览选中的文件）—— 后者**由服务端拼路径**，
    // 因为回传的目录可能带 `\\?\` verbatim 前缀，前端拼 `\\?\a/b` 在 Windows 上是非法名。
    let path = match (
        body.get("dir").and_then(|v| v.as_str()),
        body.get("name").and_then(|v| v.as_str()),
    ) {
        (Some(dir), Some(name)) if !dir.trim().is_empty() && !name.trim().is_empty() => Path::new(dir.trim())
            .join(name.trim())
            .to_string_lossy()
            .into_owned(),
        _ => body.get("path").and_then(|v| v.as_str()).unwrap_or("").to_string(),
    };
    let (value, count) = set_asset(ws, kind, &path, is_admin)
        .map_err(|e| err(StatusCode::BAD_REQUEST, format!("{e:#}")))?;
    let cfg = ws.load_config().map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}")))?;
    ok(serde_json::json!({
        "ok": true,
        "kind": kind.as_str(),
        "value": value,
        "count": count,
        "status": asset_status(ws, &cfg, kind),
    }))
}

async fn project_data_config_put(
    scope: Scope,
    Extension(user): Extension<super::auth::CurrentUser>,
    Json(body): Json<serde_json::Value>,
) -> ApiResult {
    let ws = scope.ws;
    let is_admin = user.is_admin();
    // 读改写加锁：设置页可能同时提交源文件与其它字段
    let path = ws.config_file.clone();
    let _guard = vtrim_core::filelock::lock_path(&path).await;
    blocking(move || config_vtrim_put_impl(&ws, &body, is_admin)).await
}

fn data_transcript_impl(ws: &Workspace) -> ApiResult {
    let cfg = ws.load_config().map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}")))?;
    // 与检测阶段同一口径（`Workspace::transcript_path`）：`transcript_file` → output/transcript.json
    // → output/video.srt。⚠️ 别改回裸 `PathBuf::from(...)`（那是相对进程 CWD，不是工程根）。
    let path = ws.transcript_path(&cfg);
    match path {
        Some(p) => {
            let raw = std::fs::read_to_string(&p).map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("读取失败: {e}")))?;
            let ext = p.extension().and_then(|e| e.to_str()).unwrap_or("");
            if ext.eq_ignore_ascii_case("json") {
                let v: serde_json::Value = serde_json::from_str(&raw).unwrap_or(serde_json::Value::Null);
                ok(v)
            } else {
                let t = vtrim_core::srt::parse_srt(&raw).map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("解析失败: {e}")))?;
                ok(serde_json::to_value(t).unwrap_or_default())
            }
        }
        None => Err(err(StatusCode::NOT_FOUND, "暂无转写文件")),
    }
}

/// 两个路径是否指同一个文件（canonicalize 兜住 `./` 与大小写；文件还不存在时退化成字面比较）。
fn same_path(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
}

fn data_transcript_put_impl(ws: &Workspace, body: &serde_json::Value) -> ApiResult {
    let cfg = ws.load_config().map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}")))?;
    let output = ws.output_dir();
    let configured = cfg
        .project
        .transcript_file
        .as_ref()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let segments = body.get("segments").cloned().ok_or_else(|| err(StatusCode::BAD_REQUEST, "缺少 segments"))?;
    let segs: Vec<vtrim_core::types::TranscriptSegment> = serde_json::from_value(segments).map_err(|e| err(StatusCode::BAD_REQUEST, format!("segments 解析失败: {e}")))?;
    let transcript = vtrim_core::types::Transcript { segments: segs, language: body.get("language").and_then(|l| l.as_str()).unwrap_or("zh").to_string(), asr_metadata: None };

    // 落盘目标 = 当前**实际读到的那一份**（`transcript_file` 优先，可能是用户选的外部字幕），
    // 再同步已存在的 output/ 两份。只写 output/ 的话，「选了外部字幕后在预处理页编辑」
    // 下次读到的还是原文件 —— 编辑静默丢失。
    let mut targets: Vec<PathBuf> = Vec::new();
    if let Some(effective) = ws.transcript_path(&cfg) {
        targets.push(effective);
    }
    if targets.is_empty() {
        // 配置写了但文件不在（例如刚选的外部字幕被删了）→ 写回配置指的位置，把它建回来。
        match &configured {
            Some(raw) => targets.push(ws.resolve(raw)),
            None => return Err(err(StatusCode::NOT_FOUND, "暂无转写文件")),
        }
    }
    let json_path = output.join("transcript.json");
    let srt_path = output.join("video.srt");
    for existing in [json_path, srt_path] {
        if existing.exists() && !targets.iter().any(|t| same_path(t, &existing)) {
            targets.push(existing);
        }
    }

    let json_text = serde_json::to_string_pretty(&transcript).map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("序列化失败: {e}")))?;
    for target in &targets {
        if target.extension().map(|e| e.eq_ignore_ascii_case("json")).unwrap_or(false) {
            std::fs::write(target, &json_text).map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("写入失败 {}: {e}", target.display())))?;
        } else {
            vtrim_core::srt::write_srt(&transcript, target).map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("写字幕失败 {}: {e}", target.display())))?;
        }
    }
    let paths: Vec<String> = targets.iter().map(|p| vtrim_core::path::display(p)).collect();
    ok(serde_json::json!({ "ok": true, "saved": transcript.segments.len(), "paths": paths }))
}

/// 统计 SRT 文件中的字幕条数（以时间轴行 `-->` 为准）；文件缺失/编码异常时为 0。
fn count_srt_cues(path: &Path) -> u64 {
    match std::fs::read(path) {
        Ok(bytes) => String::from_utf8_lossy(&bytes).lines().filter(|l| l.contains("-->")).count() as u64,
        Err(_) => 0,
    }
}

/// 读取项目转写文件（`transcript_file` / `output/transcript.json` / `output/video.srt`）
/// 中所有字幕段的起始秒，升序。
fn transcript_starts(ws: &Workspace) -> Vec<f64> {
    let Ok(cfg) = ws.load_config() else { return Vec::new() };
    let Some(p) = ws.transcript_path(&cfg) else { return Vec::new() };
    let Ok(raw) = std::fs::read_to_string(&p) else { return Vec::new() };
    let mut starts: Vec<f64> = if p.extension().and_then(|e| e.to_str()) == Some("json") {
        serde_json::from_str::<serde_json::Value>(&raw).ok()
            .and_then(|v| v.get("segments").and_then(|s| s.as_array()).map(|arr| {
                arr.iter().filter_map(|s| s.get("start").and_then(|x| x.as_f64())).collect::<Vec<f64>>()
            }))
            .unwrap_or_default()
    } else {
        vtrim_core::srt::parse_srt(&raw).map(|t| t.segments.iter().map(|s| s.start).collect()).unwrap_or_default()
    };
    starts.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    starts
}

/// 读取项目弹幕文件（XML / ASS / SRT）中所有弹幕的起始秒，升序。
fn danmaku_starts(ws: &Workspace) -> Vec<f64> {
    let Ok(cfg) = ws.load_config() else { return Vec::new() };
    let Some(p) = ws.danmaku_path(&cfg) else { return Vec::new() };
    let Ok(entries) = vtrim_core::danmaku::load_danmaku_auto(&p) else { return Vec::new() };
    let mut starts: Vec<f64> = entries.iter().map(|e| e.start).collect();
    starts.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    starts
}

/// 统计落在 [start, end] 内的起始点数量（starts 必须升序）。
fn count_in_range(starts: &[f64], start: f64, end: f64) -> u64 {
    if starts.is_empty() || end < start { return 0; }
    let lo = starts.partition_point(|&x| x < start);
    let hi = starts.partition_point(|&x| x <= end);
    hi.saturating_sub(lo) as u64
}

fn data_events_impl(ws: &Workspace) -> ApiResult {
    let path = ws.output_dir().join("events.json");
    if !path.exists() { return Err(err(StatusCode::NOT_FOUND, "暂无 events.json")); }
    let raw = std::fs::read_to_string(&path).map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("读取失败: {e}")))?;
    let mut v: serde_json::Value = serde_json::from_str(&raw).unwrap_or(serde_json::Value::Null);
    // 事件区间内的弹幕量/字幕量（切片页排序用），文件缺失时为 0。
    // 只有存在未剪切事件时才解析源弹幕/字幕，避免纯浏览列表时反复解析大文件。
    let clips_dir = path.parent().map(|p| p.join("clips"));
    let has_uncut = v.get("events").and_then(|e| e.as_array()).map(|evs| evs.iter().any(|ev| {
        let rank = ev.get("rank").and_then(|r| r.as_u64()).unwrap_or(0);
        !clips_dir.as_ref().map(|c| c.join(format!("clip-{:0>2}", rank)).join("clip.mp4").exists()).unwrap_or(false)
    })).unwrap_or(false);
    let (dm_starts, sub_starts) = if has_uncut {
        (danmaku_starts(ws), transcript_starts(ws))
    } else {
        (Vec::new(), Vec::new())
    };
    if let Some(evs) = v.get_mut("events").and_then(|x| x.as_array_mut()) {
        for ev in evs {
            let start = ev.get("start").and_then(|s| s.as_object())
                .and_then(|o| o.get("h").and_then(|h| h.as_u64()))
                .map(|_| ev_time_secs(ev, "start")).unwrap_or(0.0);
            let end = ev_time_secs(ev, "end");
            ev["start_sec"] = serde_json::json!(start);
            ev["end_sec"] = serde_json::json!(end);
            let rank = ev.get("rank").and_then(|r| r.as_u64()).unwrap_or(0);
            let clip_dir = path.parent().map(|p| p.join("clips").join(format!("clip-{:0>2}", rank))).unwrap_or_default();
            ev["cut_exists"] = serde_json::json!(clip_dir.join("clip.mp4").exists());
            ev["danmaku_count"] = serde_json::json!(count_in_range(&dm_starts, start, end));
            ev["subtitle_count"] = serde_json::json!(count_in_range(&sub_starts, start, end));
        }
    }
    ok(v)
}

fn ev_time_secs(v: &serde_json::Value, key: &str) -> f64 {
    let o = match v.get(key).and_then(|x| x.as_object()) { Some(o)=>o, None=>return 0.0 };
    let h=o.get("h").and_then(|x|x.as_u64()).unwrap_or(0) as f64;
    let m=o.get("m").and_then(|x|x.as_u64()).unwrap_or(0) as f64;
    let s=o.get("s").and_then(|x|x.as_u64()).unwrap_or(0) as f64;
    let ms=o.get("ms").and_then(|x|x.as_u64()).unwrap_or(0) as f64;
    h*3600.0+m*60.0+s+ms/1000.0
}


/// 秒 → `events.json` 里的 `{h,m,s,ms}` 时间戳。
///
/// 用 `Timestamp::from_seconds`（按总毫秒计算）而不是手写 floor/round：
/// 后者在 `sec = 59.9999` 时会产出非法的 `ms = 1000`（SRT 会写成 `00:00:59,1000`）。
fn sec_to_ts_value(sec: f64) -> serde_json::Value {
    let ts = vtrim_core::types::Timestamp::from_seconds(sec.max(0.0));
    serde_json::json!({ "h": ts.h, "m": ts.m, "s": ts.s, "ms": ts.ms })
}

fn data_events_create_impl(ws: &Workspace, body: &serde_json::Value) -> ApiResult {
    let path = ws.output_dir().join("events.json");
    if !path.exists() {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("创建目录失败: {e}")))?;
        }
        std::fs::write(&path, "{\"events\":[]}").map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("创建 events.json 失败: {e}")))?;
    }
    let label = body.get("label").and_then(|v| v.as_str()).unwrap_or("").trim().to_string();
    if label.is_empty() {
        return Err(err(StatusCode::BAD_REQUEST, "标题不能为空"));
    }
    let start_sec = body.get("start_sec").and_then(|v| v.as_f64())
        .or_else(|| body.get("start").and_then(|v| v.as_f64()))
        .unwrap_or(0.0);
    let end_sec = body.get("end_sec").and_then(|v| v.as_f64())
        .or_else(|| body.get("end").and_then(|v| v.as_f64()))
        .unwrap_or(start_sec + 10.0);
    if end_sec <= start_sec {
        return Err(err(StatusCode::BAD_REQUEST, "结尾必须大于开头"));
    }
    let event_type = body.get("event_type").and_then(|v| v.as_str()).unwrap_or("other").to_string();
    let summary = body.get("summary").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let raw = std::fs::read_to_string(&path).map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("读取失败: {e}")))?;
    let mut store: serde_json::Value = serde_json::from_str(&raw).unwrap_or_else(|_| serde_json::json!({ "events": [] }));
    if !store.get("events").and_then(|a| a.as_array()).is_some() {
        store["events"] = serde_json::json!([]);
    }
    let next_rank = store.get("events").and_then(|a| a.as_array()).map(|arr| {
        arr.iter().filter_map(|e| e.get("rank").and_then(|r| r.as_u64())).max().unwrap_or(0) + 1
    }).unwrap_or(1);
    let event = serde_json::json!({
        "start": sec_to_ts_value(start_sec),
        "end": sec_to_ts_value(end_sec),
        "label": label,
        "summary": summary,
        "event_type": event_type,
        "source": "manual",
        "confidence": 1.0,
        "popularity": 0.0,
        "rank": next_rank,
        "enabled": true,
    });
    if let Some(arr) = store.get_mut("events").and_then(|a| a.as_array_mut()) {
        arr.push(event.clone());
        let mut titles = serde_json::Map::new();
        for target in arr.iter() {
            if let Some(l) = target.get("label").and_then(|x| x.as_str()) {
                titles.insert(l.to_string(), serde_json::json!(target.get("enabled").and_then(|e| e.as_bool()).unwrap_or(true)));
            }
        }
        store["titles"] = serde_json::Value::Object(titles);
    }
    std::fs::write(&path, serde_json::to_string_pretty(&store).unwrap_or_default()).map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("保存失败: {e}")))?;
    ok(serde_json::json!({ "ok": true, "event": event }))
}

fn data_events_put_impl(ws: &Workspace, body: &serde_json::Value) -> ApiResult {
    let path = ws.output_dir().join("events.json");
    if !path.exists() { return Err(err(StatusCode::NOT_FOUND, "暂无 events.json")); }
    let events = body.get("events").cloned().unwrap_or_else(|| body.clone());
    let raw = std::fs::read_to_string(&path).map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("读取失败: {e}")))?;
    let mut store: serde_json::Value = serde_json::from_str(&raw).unwrap_or_else(|_| serde_json::json!({ "events": [] }));
    let mut titles = serde_json::Map::new();
    if let Some(arr) = store.get_mut("events").and_then(|a| a.as_array_mut()) {
        for target in arr.iter_mut() {
            if let Some(update) = events.as_array().and_then(|updates| {
                updates.iter().find(|u| u.get("rank").and_then(|r| r.as_u64()) == target.get("rank").and_then(|r| r.as_u64()))
            }) {
                if let Some(en) = update.get("enabled") { target["enabled"] = en.clone(); }
                if let Some(label) = update.get("label").and_then(|l| l.as_str()) { target["label"] = serde_json::json!(label); }
                if let Some(et) = update.get("event_type").and_then(|v| v.as_str()) { target["event_type"] = serde_json::json!(et); }
                if let Some(summary) = update.get("summary").and_then(|v| v.as_str()) { target["summary"] = serde_json::json!(summary); }
                if let Some(start) = update.get("start").and_then(|v| v.as_object()) { target["start"] = serde_json::json!(start); }
                if let Some(end) = update.get("end").and_then(|v| v.as_object()) { target["end"] = serde_json::json!(end); }
            }
            if let Some(label) = target.get("label").and_then(|l| l.as_str()) {
                titles.insert(label.to_string(), serde_json::json!(target.get("enabled").and_then(|e| e.as_bool()).unwrap_or(true)));
            }
        }
    }
    store["titles"] = serde_json::Value::Object(titles);
    std::fs::write(&path, serde_json::to_string_pretty(&store).unwrap_or_default()).map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("保存失败: {e}")))?;
    ok(serde_json::json!({ "ok": true }))
}

/// 把 events.json 里被**等分**出来的 `基名（i/n）` 合回一个事件（规则见
/// `vtrim_core::detection::merge_split_events`）。只在真的合并了东西时写盘。
/// 注意：事件目录名是 `clips/clip-{rank:02}`，合并后组内其余 rank 消失，
/// 它们的旧目录不会被删（要用户自己删掉重切），合并后的事件 rank 沿用组内最热的那条。
fn data_events_merge_splits_impl(ws: &Workspace) -> ApiResult {
    let path = ws.output_dir().join("events.json");
    if !path.exists() { return Err(err(StatusCode::NOT_FOUND, "暂无 events.json")); }
    let raw = std::fs::read_to_string(&path)
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("读取失败: {e}")))?;
    let mut store: serde_json::Value = serde_json::from_str(&raw)
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("解析 events.json 失败: {e}")))?;
    let arr = store.get("events").and_then(|v| v.as_array()).cloned().unwrap_or_default();
    let events: Vec<vtrim_core::types::Event> = serde_json::from_value(serde_json::Value::Array(arr))
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("事件字段解析失败: {e}")))?;
    let before = events.len();
    let (merged, groups) = vtrim_core::detection::merge_split_events(events);
    if groups == 0 {
        return ok(serde_json::json!({ "ok": true, "groups": 0, "before": before, "after": before }));
    }
    let mut titles = serde_json::Map::new();
    for e in &merged {
        titles.insert(e.label.clone(), serde_json::json!(e.enabled));
    }
    store["events"] = serde_json::to_value(&merged)
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("序列化失败: {e}")))?;
    store["titles"] = serde_json::Value::Object(titles);
    std::fs::write(&path, serde_json::to_string_pretty(&store).unwrap_or_default())
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("保存失败: {e}")))?;
    ok(serde_json::json!({ "ok": true, "groups": groups, "before": before, "after": merged.len() }))
}

fn data_clips_impl(ws: &Workspace) -> ApiResult {
    let clips = ws.output_dir().join("clips");
    if !clips.exists() { return Err(err(StatusCode::NOT_FOUND, "暂无 clips")); }
    let mut out = vec![];
    // 源弹幕的起始时间表：只有遇到「没有 clip.dm.srt 的老切片」时才解析一次（见下）
    let mut dm_starts: Option<Vec<f64>> = None;
    let entries = std::fs::read_dir(&clips).map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("读取失败: {e}")))?;
    for entry in entries.flatten() {
        if !entry.path().is_dir() { continue; }
        let dir_path = entry.path();
        let meta_path = dir_path.join("clip.json");
        let meta = std::fs::read_to_string(&meta_path).ok()
            .and_then(|raw| serde_json::from_str::<serde_json::Value>(&raw).ok())
            .unwrap_or(serde_json::json!({}));
        let video_path = dir_path.join("clip.mp4");
        let (size_bytes, duration_secs) = if video_path.exists() {
            let size = std::fs::metadata(&video_path).map(|m| m.len()).unwrap_or(0);
            let dur = meta.get("duration").and_then(|v| v.as_f64()).unwrap_or(0.0);
            (size, dur)
        } else { (0,0.0) };
        let silent_files: Vec<String> = if let Ok(rd)=std::fs::read_dir(&dir_path) { rd.flatten().filter_map(|f| { let n=f.file_name().to_string_lossy().to_lowercase(); if n.contains("cleaned") && n.ends_with(".mp4") { Some(f.file_name().to_string_lossy().into_owned()) } else { None } }).collect() } else { Vec::new() };
        // 「消音版」= 静音区间已写入屏蔽轨道（不再产生 *_cleaned.mp4 文件）
        let silenced = vtrim_polish::project::is_silent(&dir_path);
        let mute_range_count = if dir_path.join("polish.toml").exists() { vtrim_polish::project::mute_range_count(&dir_path) } else { 0 };
        // 切片自己的弹幕/字幕条数（切片页的「弹幕量 / 字幕量」排序用）
        let subtitle_count = count_srt_cues(&dir_path.join("clip.srt"));
        let dm_file = dir_path.join("clip.dm.srt");
        let danmaku_count = if dm_file.exists() {
            count_srt_cues(&dm_file)
        } else {
            // 老切片没有 clip.dm.srt（这个文件是后加的）：改成按 clip.json 的原片区间从源弹幕里数。
            // 不算这一步整列都是 0，「弹幕量」排序会退化成按 rank 排（用户反馈过"排序没生效"）。
            let starts = dm_starts.get_or_insert_with(|| danmaku_starts(ws));
            let start = meta.get("original_start").and_then(|v| v.as_f64()).unwrap_or(0.0);
            let end = meta.get("original_end").and_then(|v| v.as_f64()).unwrap_or(0.0);
            count_in_range(&starts[..], start, end)
        };
        out.push(serde_json::json!({
            "dir": vtrim_core::path::display(&dir_path),
            "name": entry.file_name().to_string_lossy(),
            "video_exists": video_path.exists(),
            "size_bytes": size_bytes,
            "duration_secs": duration_secs,
            "silenced": silenced,
            "mute_range_count": mute_range_count,
            "silent_files": silent_files,
            "meta": meta,
            "subtitle_count": subtitle_count,
            "danmaku_count": danmaku_count,
        }));
    }
    // 按**数字**序号排：`clip-100` 必须排在 `clip-02` 后面。按字符串排会变成
    // clip-01, clip-02, clip-100, clip-101, …, clip-11, clip-110…（精修页的片段条照这个顺序显示，
    // 用户报过"片段列表排序有问题，只有两个的时候分得太开"）。解析不出序号的退回按名字排。
    let rank_of = |v: &serde_json::Value| -> Option<u64> {
        v.get("name")
            .and_then(|n| n.as_str())
            .and_then(|n| n.strip_prefix("clip-"))
            .and_then(|r| r.parse::<u64>().ok())
    };
    out.sort_by(|a, b| match (rank_of(a), rank_of(b)) {
        (Some(x), Some(y)) => x.cmp(&y),
        _ => a["name"].as_str().unwrap_or("").cmp(b["name"].as_str().unwrap_or("")),
    });
    ok(serde_json::json!(out))
}

/// 「消音版」：把该切片的长静音段写入屏蔽轨道（polish.toml 的 mute_ranges）。
/// 不生成任何新文件，原始 clip.mp4 / clip.srt 保持不变。
async fn clip_silence_impl(ws: &Workspace, rank: &str, state: &AppState, pid: i64) -> ApiResult {
    // rank 直接来自 URL：先解析成数字再拼路径（`clip-../../x` 会走出项目目录）
    let Ok(rank) = rank.parse::<u32>() else {
        return Err(err(StatusCode::BAD_REQUEST, "rank 无效"));
    };
    let clip_dir = ws.output_dir().join("clips").join(format!("clip-{:0>2}", rank));
    let video = clip_dir.join("clip.mp4");
    if !video.exists() { return Err(err(StatusCode::NOT_FOUND, "clip.mp4 不存在")); }
    // 会隐式初始化精修工程 + 写屏蔽轨道：该片段有任务在跑时当场拒绝
    if let Some(busy) = state.jobs.busy_with(&super::scope_key::polish_keys(Some(pid), &clip_dir)).await {
        tracing::info!("拒绝在 {} 上改屏蔽轨道：{busy}", busy.resource);
        return Err(err(StatusCode::CONFLICT, format!("{busy}，请等它结束再改屏蔽轨道")));
    }
    ensure_polish_project(&clip_dir).await?;
    // polish.toml 的 read-modify-write 要和整修页的保存走**同一把**路径锁，否则
    // "编辑器保存设计" 与 "去静音" 并发时后写者会把先写者的改动整段盖掉。
    // 锁在 await 里持有；ffmpeg 静音扫描 + 写盘下 blocking 池。
    let _guard = vtrim_core::filelock::lock_path(&clip_dir.join("polish.toml")).await;
    let applied = blocking(move || {
        vtrim_polish::project::apply_silence(&clip_dir)
            .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("写入屏蔽轨道失败: {e:#}")))
    })
    .await?;
    ok(serde_json::json!({
        "ok": true,
        "detected": applied.ranges.len(),
        "added": applied.added,
        "total": applied.total,
        "ranges": applied.ranges.iter().map(|(s, e)| serde_json::json!({ "start": s, "end": e })).collect::<Vec<_>>(),
    }))
}

/// 把切片接口的路径参数解析成 `clips/` 下的目录：**数字 rank**（`2` → `clip-02`，老契约）
/// 或**目录名本身**（`clip-out` 这类手工命名的目录）。
///
/// 目录名优先 —— 卡片墙列的是 `clips/` 下的**真实目录名**，名字不一定规范；以前只认数字，
/// 前端把 `clip-out` 解析成 rank 0 → 请求 `clips/0` → 稳定报「clip-00 不存在」。
/// 参数直接来自 URL，所以先过 `is_safe_component`（`..` / 分隔符 / 盘符 / 保留名一律拒绝），
/// 拼出来的路径就只可能落在 `clips/` 里面。
fn resolve_clip_dir(output_dir: &Path, arg: &str) -> Result<PathBuf, AppError> {
    if !vtrim_polish::assets::sticker_index::is_safe_component(arg) {
        return Err(err(StatusCode::BAD_REQUEST, "rank 无效"));
    }
    let clips = output_dir.join("clips");
    let by_name = clips.join(arg);
    if by_name.is_dir() {
        return Ok(by_name);
    }
    match arg.parse::<u32>() {
        // 没有同名目录时按老契约理解成 rank（`clips/2` → `clips/clip-02`）
        Ok(rank) => Ok(clips.join(format!("clip-{:0>2}", rank))),
        // 名字不存在：原样返回，调用方按「{arg} 不存在」报 404
        Err(_) => Ok(by_name),
    }
}

/// 删除一个已剪切片段：整个 `clips/<目录名>/` 目录移除（精修产物、成片、封面一并删掉），
/// 并把引用从 `publish_config.json` / `clips_order.txt` 里摘掉 —— 否则发布配置指向不存在的文件。
///
/// 路径参数可以是数字 rank，也可以是目录名（见 `resolve_clip_dir`）。
/// `events.json` 不动：目录没了，该事件在切片页自动回到「未剪切」，可以重新切。
async fn clip_delete_impl(ws: &Workspace, arg: &str, state: &AppState, pid: i64) -> ApiResult {
    let clip_dir = resolve_clip_dir(&ws.output_dir(), arg)?;
    let Some(dir_name) = clip_dir.file_name().map(|n| n.to_string_lossy().into_owned()) else {
        return Err(err(StatusCode::BAD_REQUEST, "rank 无效"));
    };
    if !clip_dir.is_dir() {
        return Err(err(StatusCode::NOT_FOUND, format!("{dir_name} 不存在")));
    }
    // 删目录是同步操作：该片段上有任务在跑（渲染/精修/重切）时当场拒绝 —— 删到一半
    // 被任务写回来会留下半个工程
    if let Some(busy) = state
        .jobs
        .busy_with(&[
            super::scope_key::project_clip(pid, &dir_name),
            super::scope_key::dir_key(&clip_dir),
            super::scope_key::project_index(pid),
        ])
        .await
    {
        tracing::info!("拒绝删除 {}：{busy}", busy.resource);
        return Err(err(StatusCode::CONFLICT, format!("{busy}，请等它结束再删这个片段")));
    }
    std::fs::remove_dir_all(&clip_dir)
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("删除 {dir_name} 失败: {e}")))?;
    prune_clip_from_index(&ws.output_dir(), &dir_name);
    tracing::info!("已删除片段目录: {}", clip_dir.display());
    ok(serde_json::json!({ "ok": true, "removed": dir_name }))
}

/// 把某个切片从 `publish_config.json` / `clips_order.txt` 里摘掉（删目录后的索引同步）。
/// 两个文件都是 Cutter 的产物，缺失或格式异常就静默跳过 —— 目录本身已经删掉了。
fn prune_clip_from_index(output_dir: &Path, dir_name: &str) {
    let publish = output_dir.join("publish_config.json");
    if let Ok(raw) = std::fs::read_to_string(&publish) {
        if let Ok(mut v) = serde_json::from_str::<serde_json::Value>(&raw) {
            if let Some(arr) = v.get_mut("clips").and_then(|c| c.as_array_mut()) {
                let before = arr.len();
                arr.retain(|c| c.get("directory").and_then(|d| d.as_str()) != Some(dir_name));
                if arr.len() != before {
                    if let Ok(out) = serde_json::to_string_pretty(&v) {
                        let _ = std::fs::write(&publish, out);
                    }
                }
            }
        }
    }
    // clips_order.txt：每行 `clip-NN / 标题`（没有标题时只有 `clip-NN`）
    let order = output_dir.join("clips").join("clips_order.txt");
    if let Ok(raw) = std::fs::read_to_string(&order) {
        let kept: Vec<&str> = raw
            .lines()
            .filter(|l| {
                let t = l.trim();
                t != dir_name && !t.starts_with(&format!("{dir_name} "))
            })
            .collect();
        if kept.len() != raw.lines().count() {
            let _ = std::fs::write(&order, kept.join("\n"));
        }
    }
}

/// polish 系列接口的 `dir` 参数校验：非空 + 必须已存在且是目录。
///
/// 前端永远传绝对路径；这里挡的是空串（会在 server 进程 CWD 下写入 polish.toml /
/// data / sfx）和不存在的路径（错误信息也更明确）。接受 `&str`/`String`/`&String`。
fn validated_project_dir(dir: impl AsRef<Path>) -> Result<PathBuf, AppError> {
    let path = dir.as_ref();
    if path.as_os_str().is_empty() {
        return Err(err(StatusCode::BAD_REQUEST, "缺少 dir"));
    }
    if !path.is_dir() {
        return Err(err(StatusCode::BAD_REQUEST, format!("目录不存在: {}", path.display())));
    }
    Ok(path.to_path_buf())
}

/// 精修工程不存在时自动初始化（与前端进入精修时的行为一致）。
pub(crate) async fn ensure_polish_project(dir: &Path) -> Result<(), AppError> {
    if dir.join("polish.toml").exists() {
        return Ok(());
    }
    vtrim_polish::commands::cmd_init(dir)
        .await
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("初始化精修工程失败: {e:#}")))
}

async fn event_cut_all_impl(
    ws: &Workspace,
    body: &serde_json::Value,
    jobs: &Arc<super::jobs::JobManager>,
    owner: i64,
    project: i64,
) -> ApiResult {
    let ev_path = ws.output_dir().join("events.json");
    if !ev_path.exists() { return Err(err(StatusCode::NOT_FOUND, "暂无 events.json")); }
    let raw = std::fs::read_to_string(&ev_path).map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("读取失败: {e}")))?;
    let store: serde_json::Value = serde_json::from_str(&raw).map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("解析失败: {e}")))?;
    let Some(all_events) = store.get("events").and_then(|a| a.as_array()) else {
        return Err(err(StatusCode::NOT_FOUND, "暂无事件"));
    };
    let only: Vec<u32> = body.get("ranks").and_then(|v| v.as_array())
        .map(|arr| arr.iter().filter_map(|x| x.as_u64().and_then(|n| u32::try_from(n).ok())).collect())
        .unwrap_or_default();
    let has_filter = !only.is_empty();
    let output = ws.output_dir();
    let mut events = Vec::new();
    for ev in all_events {
        let Some(rank) = ev.get("rank").and_then(|r| r.as_u64()).and_then(|r| u32::try_from(r).ok()) else {
            continue;
        };
        if has_filter && !only.contains(&rank) {
            continue;
        }
        // 未显式指定 ranks 时遵循事件的 enabled 开关（detection::load_events 也这么做），
        // 显式点了某个事件则按用户要求执行。
        if !has_filter && !ev.get("enabled").and_then(|e| e.as_bool()).unwrap_or(true) {
            continue;
        }
        let clip_dir = output.join("clips").join(format!("clip-{:02}", rank));
        if clip_dir.join("clip.mp4").exists() {
            continue;
        }
        events.push(ev.clone());
    }
    if events.is_empty() {
        return Err(err(StatusCode::BAD_REQUEST, "没有需要剪切的未剪切事件"));
    }

    // 资源域：out（读 events.json）+ idx（两个发布索引）。**不带 clip 键** ——
    // 全切只处理没有 clip.mp4 的目录，不会碰精修工程，所以刻意允许与精修并行。
    let keys = super::scope_key::cut_keys(project, &output, None);
    let total = events.len() as u64;
    let handle = jobs
        .create(
            owner,
            JobSpec::new("cut_all", format!("一键全切 {} 个事件", total), keys, JobClass::Heavy),
        )
        .await;
    let id = handle.id.clone();
    let queued = handle.queued;
    jobs.attach_project(&id, project).await;
    let jobs_handle = jobs.clone();
    let cfg_file = ws.config_file.clone();
    let job_id = id.clone();
    let cancel = jobs.cancel_token(&id).await;
    // 任务体跑在专用线程上；每条片段的剪切再下 blocking 池（3 路真并行，谁也占不到 HTTP worker）
    spawn_job(jobs_handle.clone(), job_id.clone(), move || async move {
        if handle.wait_start().await.is_err() {
            jobs_handle.complete(&job_id).await;
            return;
        }
        jobs_handle.progress(&job_id, 0, total).await;
        // 一次全切只产生一个 cut_all 任务；内部最多 3 个 ffmpeg 并发，进度统一写回该任务。
        let sem = Arc::new(tokio::sync::Semaphore::new(3));
        let mut handles = Vec::new();
        let is_cancelled = || {
            cancel
                .as_ref()
                .map(|c| c.load(std::sync::atomic::Ordering::SeqCst))
                .unwrap_or(false)
        };
        for ev in events {
            // 取消在事件边界生效（进行中的 ffmpeg 不会被中断）
            if is_cancelled() {
                break;
            }
            let permit = match sem.clone().acquire_owned().await {
                Ok(p) => p,
                Err(_) => break,
            };
            let cfg_file = cfg_file.clone();
            handles.push(tokio::task::spawn_blocking(move || {
                let _permit = permit;
                let rank = ev.get("rank").and_then(|r| r.as_u64()).unwrap_or(0);
                // 阻塞线程里没有 runtime：给这段管线单独开一个 current-thread runtime
                (rank, block_on_here(run_single_cut(&cfg_file, ev)))
            }));
        }
        let mut failures = 0usize;
        let mut done = 0u64;
        for handle in handles {
            match handle.await {
                Ok((_, Ok(()))) => {}
                Ok((rank, Err(e))) => {
                    failures += 1;
                    let _ = jobs_handle.log(&job_id, "error", format!("clip-{:02} 剪切失败: {e:#}", rank)).await;
                }
                Err(_) => failures += 1,
            }
            done += 1;
            jobs_handle.progress(&job_id, done, total).await;
        }
        if failures == 0 {
            jobs_handle.complete(&job_id).await;
        } else {
            jobs_handle.fail(&job_id, format!("{} 个事件剪切失败", failures)).await;
        }
    });
    ok(serde_json::json!({ "ok": true, "id": id, "queued": queued }))
}

async fn run_single_cut(config_file: &Path, ev: serde_json::Value) -> anyhow::Result<()> {
    run_single_cut_event(config_file, serde_json::from_value(ev)?).await
}

/// 同上，但调用方已经拿着解析好的事件（单条重切走这条，省一次 JSON 往返）。
async fn run_single_cut_event(config_file: &Path, ev: vtrim_core::types::Event) -> anyhow::Result<()> {
    let cfg = vtrim_core::config::load_config_at(config_file)?;
    let w = vtrim_service::Workspace::load(Some(config_file))?;
    let input = cfg.project.input_file.as_ref().filter(|s| !s.is_empty()).map(|s| w.resolve(s))
        .ok_or_else(|| anyhow::anyhow!("未初始化 input_file"))?;
    let output = w.output_dir();
    let mut ctx = vtrim_core::types::PipelineContext::new(input, output);
    ctx.events = vec![ev];
    let mut r = vtrim_core::pipeline::PipelineRunner::new(cfg);
    r.add_stage(Box::new(vtrim_core::cutting::Cutter));
    r.run(&mut ctx).await
}

async fn event_cut_impl(
    ws: &Workspace,
    rank: &str,
    jobs: &Arc<super::jobs::JobManager>,
    owner: i64,
    project: i64,
) -> ApiResult {
    let ev_path = ws.output_dir().join("events.json");
    if !ev_path.exists() { return Err(err(StatusCode::NOT_FOUND, "暂无 events.json")); }
    let raw = std::fs::read_to_string(&ev_path).map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("读取失败: {e}")))?;
    let store: serde_json::Value = serde_json::from_str(&raw).map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("解析失败: {e}")))?;
    let rank_u: u32 = rank.parse().map_err(|_| err(StatusCode::BAD_REQUEST, "rank 无效"))?;
    let event: serde_json::Value = store.get("events").and_then(|a| a.as_array()).and_then(|arr| arr.iter().find(|e| e.get("rank").and_then(|r| r.as_u64()).unwrap_or(0) as u32 == rank_u).cloned())
        .ok_or_else(|| err(StatusCode::NOT_FOUND, "事件不存在"))?;
    let ev: vtrim_core::types::Event = serde_json::from_value(event).map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("事件解析失败: {e}")))?;
    // 资源域含这个片段目录：单条重切会写 clip.mp4/clip.json，与同片段的精修互斥（排队）。
    let keys = super::scope_key::cut_keys(project, &ws.output_dir(), Some(rank_u));
    let handle = jobs
        .create(
            owner,
            JobSpec::new("cut_one", format!("剪切事件 #{rank}"), keys, JobClass::Medium),
        )
        .await;
    let id = handle.id.clone();
    let queued = handle.queued;
    jobs.attach_project(&id, project).await;
    let jobs_handle = jobs.clone(); let cfg_file = ws.config_file.clone(); let job_id=id.clone();
    // 剪切是一整段同步代码（ffmpeg 等源文件读完整段），跑在专用任务线程上 —— 不占 HTTP worker
    spawn_job(jobs_handle.clone(), job_id.clone(), move || async move {
        if handle.wait_start().await.is_err() {
            jobs_handle.complete(&job_id).await;
            return;
        }
        let result = run_single_cut_event(&cfg_file, ev).await;
        match result { Ok(_)=>jobs_handle.complete(&job_id).await, Err(e)=>jobs_handle.fail(&job_id, format!("{e:#}")).await }
    });
    ok(serde_json::json!({ "ok": true, "id": id, "queued": queued }))
}

/// `POST /api/fs/list {dir}`：**仅 admin**，浏览服务器任意目录（与单用户时代一致）。
///
/// - `dir` 可以是**任意绝对路径**（管理员），缺省 → 租户根；也兼容 `{root, path}`
///   （path 相对所选根，囚笼在内）；
/// - 只做目录列举，**永不接受上传**；
/// - 响应回传 `roots`（租户根 + `[storage] local_roots`，作为快捷入口）、`dir` 与 `parent`
///   （绝对父目录，供"上一级"一路走到盘符根）。
async fn fs_list(
    Extension(user): Extension<super::auth::CurrentUser>,
    Json(body): Json<serde_json::Value>,
) -> ApiResult {
    if !user.is_admin() {
        return Err(err(StatusCode::FORBIDDEN, "仅管理员可浏览服务器文件"));
    }
    // 允许的根 = 租户根 + [storage] local_roots（这些只是"快捷入口"；管理员可用任意绝对路径）
    let tenant = vtrim_service::projects::tenant_root(user.id);
    std::fs::create_dir_all(&tenant)
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("创建租户目录失败: {e}")))?;
    let roots = vtrim_service::projects::local_roots(user.id);
    let is_admin = user.is_admin();
    let dir_hint = body
        .get("dir")
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let root_hint = body.get("root").and_then(|v| v.as_str()).unwrap_or("");
    let path_hint = body.get("path").and_then(|v| v.as_str()).unwrap_or("");
    let bad = |e: anyhow::Error| err(StatusCode::BAD_REQUEST, format!("{e:#}"));
    let target = match dir_hint {
        // 新契约：直接给目录（绝对路径 = 任意位置；相对路径 = 相对快捷根）
        Some(d) => {
            vtrim_service::projects::resolve_target_dir(user.id, root_hint, &d, is_admin)
                .map_err(bad)?
        }
        // 兼容契约：{root, path}
        None if Path::new(path_hint.trim()).is_absolute() => {
            vtrim_service::projects::resolve_target_dir(user.id, root_hint, path_hint, is_admin)
                .map_err(bad)?
        }
        None => {
            let root = vtrim_service::projects::pick_local_root(user.id, root_hint, is_admin)
                .map_err(bad)?;
            if path_hint.trim().is_empty() {
                root
            } else {
                vtrim_service::projects::resolve_target_dir(user.id, "", path_hint, is_admin)
                    .map_err(bad)?
            }
        }
    };
    // `enter`：进入下一级目录（**名字由服务端拼接**，前端不再做字符串加法，
    // 否则 `\\?\F:\a` + "/b" 会拼成非法路径 → ERROR_INVALID_NAME 123）
    let target = match body
        .get("enter")
        .and_then(|v| v.as_str())
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
    {
        None => target,
        Some(name) => {
            if name.contains('/')
                || name.contains('\\')
                || name.contains(':')
                || name == "."
                || name == ".."
            {
                return Err(err(StatusCode::BAD_REQUEST, format!("非法目录名: {name}")));
            }
            let child = target.join(name);
            if !child.is_dir() {
                return Err(err(
                    StatusCode::BAD_REQUEST,
                    format!("目录不存在: {name}（在 {} 下）", vtrim_core::path::display(&target)),
                ));
            }
            child
        }
    };
    if !target.is_dir() {
        return Err(err(StatusCode::BAD_REQUEST, "只能浏览目录"));
    }
    let cur = target.canonicalize().unwrap_or_else(|_| target.clone());
    // **回到"人能读、能回传"的形式**：去掉 canonicalize 产生的 `\\?\` 前缀
    let cur_str = vtrim_core::path::display(&cur);
    // 绝对父目录：让前端"上一级"能一路走到盘符根（管理员随意浏览）
    let parent = cur
        .parent()
        .map(|p| vtrim_core::path::display(p))
        .unwrap_or_default();
    // 目录列举是阻塞 IO（网络盘/大目录会明显卡顿），放 spawn_blocking
    let scan_dir = target.clone();
    let (dirs, files) = tokio::task::spawn_blocking(move || -> std::io::Result<(Vec<String>, Vec<String>)> {
        let mut files = vec![]; let mut dirs = vec![];
        for e in std::fs::read_dir(&scan_dir)? {
            let e = e?;
            if e.file_type().map(|t| t.is_dir()).unwrap_or(false) { dirs.push(e.file_name().to_string_lossy().to_string()); }
            else { files.push(e.file_name().to_string_lossy().to_string()); }
        }
        dirs.sort(); files.sort();
        Ok((dirs, files))
    })
    .await
    .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("读取失败: {e}")))?
    .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("读取失败: {e}")))?;
    let root_list: Vec<serde_json::Value> = roots
        .iter()
        .map(|r| {
            // 显示形式（去掉 \\?\ 前缀）；前端原样回传即可命中快捷入口
            let canon = r.canonicalize().unwrap_or_else(|_| r.clone());
            serde_json::json!({
                "path": vtrim_core::path::display(&canon),
                "label": vtrim_service::projects::root_label(user.id, &canon),
                "tenant": canon == tenant.canonicalize().unwrap_or_else(|_| tenant.clone()),
            })
        })
        .collect();
    // 当前目录若不是快捷入口之一，也追加进下拉（下拉始终能显示"当前位置"）
    let mut root_list = root_list;
    if !root_list.iter().any(|x| x["path"].as_str() == Some(cur_str.as_str())) {
        root_list.push(serde_json::json!({
            "path": cur_str,
            "label": cur
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| cur_str.clone()),
            "tenant": false,
        }));
    }
    // 兼容旧前端：相对某个快捷根的路径
    let rel = roots
        .iter()
        .filter_map(|r| r.canonicalize().ok())
        .find_map(|r| {
            cur.strip_prefix(&r)
                .ok()
                .map(|p| p.to_string_lossy().replace('\\', "/"))
        })
        .unwrap_or_default();
    ok(serde_json::json!({
        "dir": cur_str,
        "root": cur_str,
        "root_label": vtrim_service::projects::root_label(user.id, &cur),
        "roots": root_list,
        "tenant_root": vtrim_core::path::display(&tenant),
        "path": rel,
        "parent": parent,
        "dirs": dirs,
        "files": files,
    }))
}

async fn media_get(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<super::auth::CurrentUser>,
    AxumPath(path): AxumPath<String>,
    headers: axum::http::HeaderMap,
) -> Response {
    // 只服务本人项目内的文件：
    // - 新契约 `/api/media/<project_id>/<项目内相对路径>`
    // - 过渡期 `/api/media/<绝对路径>`：必须落在自己的某个项目根内
    //
    // 归属校验 + `jail_existing`/`canonicalize` 都是对项目目录的同步 IO（可能网络盘），
    // 而播放视频会打出一连串 Range 请求 —— 整段下 blocking 池，别让播放把 worker 占满。
    let (project, rel) = media::split_project_media(&path);
    let ws = state.workspace().await;
    let st = state.clone();
    let resolved = blocking(move || resolve_media_path(st.auth.db(), &user, project, &rel, &ws)).await;
    let path = match resolved {
        Ok(p) => p,
        Err(e) => return e.into_response(),
    };
    let mime = match path.extension().and_then(|e| e.to_str()).map(|e| e.to_ascii_lowercase()).as_deref() {
        Some("mp4" | "m4v") => "video/mp4".to_string(),
        Some("webm") => "video/webm".to_string(),
        Some("ogv") => "video/ogg".to_string(),
        Some("mov") => "video/quicktime".to_string(),
        _ => mime_guess::from_path(&path).first_or_octet_stream().to_string(),
    };
    let range_header = headers.get(axum::http::header::RANGE).and_then(|v| v.to_str().ok());
    media::serve_file(&path, &mime, range_header).await
}

/// `/api/media/*` 的归属校验 + 路径解析（同步：DB 查询 + 项目目录上的 canonicalize/jail）。
/// 抽出来是为了整体丢进 blocking 池 —— 见 `media_get` 的调用点。
fn resolve_media_path(
    db: &vtrim_service::db::Db,
    user: &super::auth::CurrentUser,
    project: Option<i64>,
    rel: &str,
    ws: &Workspace,
) -> Result<PathBuf, AppError> {
    match project {
        Some(id) => {
            let proj = match super::scope::find_owned_project(db, user, id) {
                Ok(Some(p)) => p,
                Ok(None) => return Err(err(StatusCode::NOT_FOUND, "项目不存在或无权访问")),
                Err(e) => return Err(err(StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}"))),
            };
            vtrim_core::tenant::jail_existing(&proj.root_dir, rel)
                .map_err(|e| err(StatusCode::NOT_FOUND, format!("文件不可用: {e:#}")))
        }
        None => {
            let abs = if Path::new(rel).is_absolute() {
                PathBuf::from(rel)
            } else {
                // 相对路径必须配合项目 id（新契约）；这里用当前工作区解析后仍要过归属校验
                ws.resolve(rel)
            };
            match super::scope::find_project_for_dir(db, user, &abs) {
                Ok(Some(proj)) => {
                    let root = proj.root_dir.canonicalize().unwrap_or_else(|_| proj.root_dir.clone());
                    let real = abs.canonicalize().unwrap_or_else(|_| abs.clone());
                    if !real.starts_with(&root) {
                        return Err(err(StatusCode::FORBIDDEN, "文件不在你的项目内"));
                    }
                    Ok(real)
                }
                Ok(None) => Err(err(
                    StatusCode::FORBIDDEN,
                    "文件不在你的任何项目内（新契约：/api/media/<项目id>/<相对路径>）",
                )),
                Err(e) => Err(e),
            }
        }
    }
}

// ===================== Pipeline =====================

async fn run_pipeline_job(
    jobs: &Arc<super::jobs::JobManager>,
    owner: i64,
    project: i64,
    config_file: std::path::PathBuf,
    kind: &str,
    stages: Vec<String>,
    dry_run: bool,
    resume: bool,
) -> ApiResult {
    // 资源域：项目输出目录；转写连带全局模型目录、含剪切断时连带发布索引
    // + 该切的所有 `clip-NN`（含剪切断的管线会重切同名片段，不能与正在精修/渲染的并行）。
    let mut keys = super::scope_key::pipeline_keys(project, &stages);
    if stages.iter().any(|s| s == "cut" || s == "process") {
        if let Ok(ws) = vtrim_service::Workspace::load(Some(&config_file)) {
            let out = ws.output_dir();
            keys.extend(super::scope_key::cut_stage_clip_keys(project, &out, &out.join("events.json")));
        }
    }
    let run_stages = stages.clone();
    // 档位按阶段判定：含转写/剪切/全流程 = 重活，纯提取/检测 = 略重（见 `JobClass::for_pipeline`）
    let class = JobClass::for_pipeline(&stages);
    let handle = jobs.create(owner, JobSpec::new(kind, "准备中", keys, class)).await;
    let id = handle.id.clone();
    let queued = handle.queued;
    jobs.attach_project(&id, project).await;
    let jobs = jobs.clone();
    let job_id = id.clone();
    let cancel = jobs.cancel_token(&id).await;
    // 管线（转写/检测/剪切）是整段同步代码：跑在专用任务线程上，不占 HTTP worker
    spawn_job(jobs.clone(), job_id.clone(), move || async move {
        if handle.wait_start().await.is_err() {
            jobs.complete(&job_id).await;
            return;
        }
        let result = crate::server::api::core::run_stages(config_file, run_stages, dry_run, resume, job_id.clone(), jobs.clone(), cancel).await;
        if let Err(e) = result {
            jobs.fail(&job_id, format!("{e:#}")).await;
            return;
        }
        jobs.complete(&job_id).await;
    });
    ok(serde_json::json!({ "ok": true, "id": id, "queued": queued }))
}

async fn correct_run_impl(
    ws: &Workspace,
    jobs: &Arc<super::jobs::JobManager>,
    owner: i64,
    project: i64,
) -> ApiResult {
    // 订正写 output/*（correct.checkpoint / video_llm.srt），与管线同域。
    // 略重档：全程在等 LLM 网络，不该跟渲染抢重活名额。
    let handle = jobs
        .create(
            owner,
            JobSpec::new(
                "correct",
                "LLM 订正运行中",
                vec![super::scope_key::project_out(project)],
                JobClass::Medium,
            ),
        )
        .await;
    let id = handle.id.clone();
    let queued = handle.queued;
    jobs.attach_project(&id, project).await;
    let jobs_handle = jobs.clone();
    let cfg_file = ws.config_file.clone();
    let job_id = id.clone();
    spawn_job(jobs_handle.clone(), job_id.clone(), move || async move {
        if handle.wait_start().await.is_err() {
            jobs_handle.complete(&job_id).await;
            return;
        }
        let result = vtrim_service::run_correct(&cfg_file, None, true).await;
        if let Err(e) = result {
            jobs_handle.fail(&job_id, format!("{e:#}")).await;
        } else {
            jobs_handle.complete(&job_id).await;
        }
    });
    ok(serde_json::json!({ "ok": true, "id": id, "queued": queued }))
}

/// `GET /api/jobs`：只列**本人**任务；admin 带 `?all=1` 可看全平台（P5 管理页用）。
async fn jobs_list(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<super::auth::CurrentUser>,
    Query(q): Query<std::collections::HashMap<String, String>>,
) -> ApiResult {
    let all = user.is_admin()
        && q.get("all")
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(false);
    let list = state.jobs.list_scoped(user.id, user.is_admin(), all).await;
    ok(serde_json::to_value(list).unwrap_or_default())
}

async fn job_get(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<super::auth::CurrentUser>,
    AxumPath(id): AxumPath<String>,
) -> ApiResult {
    // 他人任务一律 404（不泄露存在性）
    match state.jobs.get_owned(&id, user.id, user.is_admin()).await {
        Some(j) => ok(serde_json::to_value(j).unwrap_or_default()),
        None => Err(err(StatusCode::NOT_FOUND, "任务不存在")),
    }
}

fn sse_event(ev: &JobEvent) -> Event {
    Event::default()
        .event("message")
        .data(serde_json::to_string(ev).unwrap_or_else(|_| "{}".into()))
}

/// 终态事件：之后不会再有有意义的事件，单任务流可以主动关闭。
fn is_terminal_event(ev: &JobEvent) -> bool {
    matches!(
        ev,
        JobEvent::Completed | JobEvent::Failed { .. } | JobEvent::Cancelled
    )
}

/// 把 broadcast 接收端映射成 SSE 事件流。
///
/// - `Lagged`：只是订阅者太慢丢了若干条，**绝不能**报成 `Failed`
///   （旧实现把错误映射为 `failed: 事件流中断`，前端会把运行中的任务判为失败）；
/// - 发送端全部退出时 broadcast 流自然结束（tokio-stream 0.1 只有 `Lagged` 一种错误），
///   流随之正常收尾。
fn job_event_stream(
    rx: tokio::sync::broadcast::Receiver<JobEvent>,
) -> impl Stream<Item = (bool, Event)> + Send + 'static {
    BroadcastStream::new(rx).filter_map(|item| async move {
        match item {
            Ok(ev) => {
                let terminal = is_terminal_event(&ev);
                Some((terminal, sse_event(&ev)))
            }
            Err(BroadcastStreamRecvError::Lagged(n)) => Some((
                false,
                sse_event(&JobEvent::Log {
                    level: "warn".into(),
                    message: format!("事件流落后，已跳过 {n} 条事件"),
                }),
            )),
        }
    })
}

/// 全局流的版本：额外带 `owner_id`，供 `/api/stream` 按归属过滤。
fn global_event_stream(
    rx: tokio::sync::broadcast::Receiver<super::jobs::OwnedEvent>,
) -> impl Stream<Item = (i64, bool, Event)> + Send + 'static {
    BroadcastStream::new(rx).filter_map(|item| async move {
        match item {
            Ok(ev) => {
                let terminal = is_terminal_event(&ev.event);
                Some((ev.owner_id, terminal, sse_event(&ev.event)))
            }
            Err(BroadcastStreamRecvError::Lagged(n)) => Some((
                0, // 0 = 面向所有订阅者的提示（不属于任何用户）
                false,
                sse_event(&JobEvent::Log {
                    level: "warn".into(),
                    message: format!("事件流落后，已跳过 {n} 条事件"),
                }),
            )),
        }
    })
}

/// `/api/stream`：**只推本人事件**（服务端过滤，前端无感知）。
async fn global_stream(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<super::auth::CurrentUser>,
) -> Response {
    let rx = state.jobs.subscribe_all().await;
    let uid = user.id;
    let is_admin = user.is_admin();
    let stream: std::pin::Pin<Box<dyn Stream<Item = Result<Event, axum::Error>> + Send>> =
        Box::pin(global_event_stream(rx).filter_map(move |(owner, _, ev)| {
            let mine = is_admin || owner == uid;
            std::future::ready(mine.then_some(Ok(ev)))
        }));
    Sse::new(stream).keep_alive(KeepAlive::new().interval(Duration::from_secs(15))).into_response()
}

async fn job_events(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<super::auth::CurrentUser>,
    AxumPath(id): AxumPath<String>,
) -> Response {
    // 归属校验：别人的任务连 SSE 都订不到（404）
    if !state.jobs.owned_by(&id, user.id, user.is_admin()).await {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "ok": false, "error": "任务不存在" })),
        )
            .into_response();
    }
    let Some((history, rx)) = state.jobs.subscribe(&id).await else {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "ok": false, "error": "任务不存在" })),
        )
            .into_response();
    };
    // 先回放历史：快任务可能在 EventSource 建立之前就结束，
    // 只给实时流会让前端永远停在「运行中」。终态事件之后流即结束。
    let replayed =
        futures_util::stream::iter(history.into_iter().map(|ev| (is_terminal_event(&ev), sse_event(&ev))));
    let stream: std::pin::Pin<Box<dyn Stream<Item = Result<Event, axum::Error>> + Send>> =
        Box::pin(
            replayed
                .chain(job_event_stream(rx))
                .take_while(|(terminal, _)| std::future::ready(!*terminal))
                .map(|(_, ev)| Ok(ev)),
        );
    Sse::new(stream).keep_alive(KeepAlive::new().interval(Duration::from_secs(15))).into_response()
}

async fn job_cancel(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<super::auth::CurrentUser>,
    AxumPath(id): AxumPath<String>,
) -> ApiResult {
    // 只能取消自己的任务（他人任务 404）
    if !state.jobs.owned_by(&id, user.id, user.is_admin()).await {
        return Err(err(StatusCode::NOT_FOUND, "任务不存在"));
    }
    if state.jobs.cancel(&id).await {
        ok(serde_json::json!({ "ok": true }))
    } else {
        Err(err(StatusCode::BAD_REQUEST, "无法取消任务"))
    }
}


/// 去掉 ANSI 颜色转义（子进程 stdout 里带着 tracing 的着色）。
fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            for c2 in chars.by_ref() {
                if c2.is_ascii_alphabetic() {
                    break;
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// 从子进程输出里取「关键词后面那一段」（取最后一条命中的行）。
///
/// 渲染/封面都是外部 CLI 进程，成功路径的 stdout 原本被丢掉 —— 而产物**同名不覆盖**，
/// 第二次产出会叫 `cover_1.jpg` / `vtuber-clip_1.mp4`，日志里不写清楚，用户会以为被覆盖了。
fn subprocess_note(raw: &str, needle: &str) -> Option<String> {
    let line = raw.lines().rev().find(|l| l.contains(needle))?;
    let plain = strip_ansi(line);
    let idx = plain.find(needle)? + needle.len();
    let mut tail = plain[idx..].trim();
    // `FINISHED: <路径> (12.3MB, 45s)` → 只留路径
    if tail.ends_with(')') {
        if let Some((head, _)) = tail.rsplit_once(" (") {
            tail = head.trim();
        }
    }
    if tail.is_empty() { None } else { Some(tail.to_string()) }
}

fn latest_numbered_file(dir: &std::path::Path, stem: &str, ext: &str) -> Option<String> {
    let mut best: Option<(u32, String)> = None;
    let mut consider = |n: u32, name: String| {
        if best.as_ref().map(|(bn, _)| n > *bn).unwrap_or(true) {
            best = Some((n, name));
        }
    };
    let base = format!("{stem}.{ext}");
    if dir.join(&base).exists() {
        consider(0, base);
    }
    if let Ok(rd) = std::fs::read_dir(dir) {
        let prefix = format!("{stem}_");
        let suffix = format!(".{ext}");
        for entry in rd.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if !name.starts_with(&prefix) || !name.ends_with(&suffix) {
                continue;
            }
            let mid = &name[prefix.len()..name.len()-suffix.len()];
            if let Ok(n) = mid.parse::<u32>() {
                consider(n, name);
            }
        }
    }
    best.map(|(_, name)| name)
}

// ===================== 渲染/封面子进程 =====================

/// 子进程输出里值得进「运行日志」的里程碑片段；其余行只走 debug（留在任务卡上，不刷日志）。
const RENDER_MILESTONES: &[&str] = &[
    "-- Server --",
    "-- Sound data --",
    "-- Sticker GIF --",
    "-- Render chunks",
    "-- Concat --",
    "-- Audio --",
    "-- SFX mix",
    "-- Mux --",
    "-- Cover --",
    "FINISHED:",
    "Cover saved:",
    "✂ 已导出去除",
    "⏱",
];

/// 跟随一个渲染/封面子进程的日志文件（边跑边读）。
#[derive(Default)]
struct RenderOutput {
    path: PathBuf,
    offset: u64,
    carry: Vec<u8>,
    /// 最近 30 行：失败时进错误文案
    tail: std::collections::VecDeque<String>,
    /// 每个分块的最新百分比（进度 = 最慢的那块）
    chunk_pct: std::collections::BTreeMap<String, u32>,
    last_active: Option<std::time::Instant>,
    warned_silent: Option<std::time::Instant>,
}

impl RenderOutput {
    fn new(path: PathBuf) -> Self {
        Self {
            path,
            last_active: Some(std::time::Instant::now()),
            ..Default::default()
        }
    }

    /// 读出自上次以来的完整行（不完整的尾巴留在 `carry` 里等下轮）。
    fn drain(&mut self) -> Vec<String> {
        use std::io::{Read, Seek, SeekFrom};
        let Ok(mut f) = std::fs::File::open(&self.path) else {
            return Vec::new();
        };
        if f.seek(SeekFrom::Start(self.offset)).is_err() {
            return Vec::new();
        }
        let mut buf = Vec::new();
        if f.read_to_end(&mut buf).is_err() || buf.is_empty() {
            return Vec::new();
        }
        self.offset += buf.len() as u64;
        self.carry.extend_from_slice(&buf);
        let mut lines = Vec::new();
        let mut start = 0usize;
        for i in 0..self.carry.len() {
            if self.carry[i] == b'\n' {
                lines.push(
                    String::from_utf8_lossy(&self.carry[start..i])
                        .trim_end_matches('\r')
                        .to_string(),
                );
                start = i + 1;
            }
        }
        self.carry.drain(..start);
        // 极端情况：一直没有换行（比如 ffmpeg 进度条），别让缓冲区无限涨
        if self.carry.len() > 512 * 1024 {
            lines.push(String::from_utf8_lossy(&self.carry).to_string());
            self.carry.clear();
        }
        lines
    }

    fn push_tail(&mut self, line: &str) {
        self.tail.push_back(line.to_string());
        while self.tail.len() > 30 {
            self.tail.pop_front();
        }
    }

    fn tail_text(&self) -> String {
        self.tail.iter().cloned().collect::<Vec<_>>().join("\n")
    }
}

/// 解析 `frames 0..1991: 45%  120s` / `chunk#2: done  1991 frames` → (标识, 百分比)。
///
/// 标识 = 冒号前那一段（帧区间或 `chunk#N`）：4 个分块各报各的，父进程按**最小值**
/// 聚合成整体进度 —— 最慢的那块决定渲染什么时候结束。
fn parse_chunk_progress(line: &str) -> Option<(String, u32)> {
    let after = line
        .split_once("frames ")
        .map(|(_, rest)| rest)
        .or_else(|| line.split_once("chunk").map(|(_, rest)| rest))?;
    let (tag, rest) = after.split_once(':')?;
    let tag = tag.trim();
    if tag.is_empty()
        || tag.len() > 24
        || !tag.chars().all(|c| c.is_ascii_digit() || c == '.' || c == '#')
    {
        return None;
    }
    if rest.contains("done") {
        return Some((tag.to_string(), 100));
    }
    if !rest.contains('%') {
        return None;
    }
    let pct_part = rest.split('%').next()?;
    let digits: String = pct_part.chars().filter(|c| c.is_ascii_digit()).collect();
    Some((tag.to_string(), digits.parse::<u32>().ok()?.min(100)))
}

/// 去掉 tracing 的前缀（时间/级别/target），只留消息本身。
fn pretty_render_line(raw: &str) -> String {
    let plain = strip_ansi(raw);
    let is_tracing = |head: &str| {
        head.contains("::")
            || ["INFO", "WARN", "ERROR", "DEBUG", "TRACE"]
                .iter()
                .any(|l| head.contains(l))
    };
    let msg = match plain.split_once(": ") {
        Some((head, tail)) if is_tracing(head) => tail,
        _ => plain.as_str(),
    };
    let trimmed = msg.trim();
    if trimmed.chars().count() > 300 {
        format!("{}…", trimmed.chars().take(300).collect::<String>())
    } else {
        trimmed.to_string()
    }
}

async fn emit_render_line(
    jobs: &Arc<super::jobs::JobManager>,
    job_id: &str,
    out: &mut RenderOutput,
    line: &str,
) {
    let msg = pretty_render_line(line);
    if msg.is_empty() {
        return;
    }
    out.last_active = Some(std::time::Instant::now());
    out.push_tail(&msg);
    if let Some((tag, pct)) = parse_chunk_progress(&msg) {
        out.chunk_pct.insert(tag, pct);
        // 最小的那个 = 最慢的分块 = 整体进度（等最慢的块跑完才算完）
        let slowest = out.chunk_pct.values().copied().min().unwrap_or(0);
        jobs.progress(job_id, slowest as u64, 100).await;
        return; // 每 1.5s 一条的进度行不进日志，靠任务卡的进度条表达
    }
    if msg.contains('⚠') {
        jobs.log(job_id, "warn", &msg).await;
    } else if RENDER_MILESTONES.iter().any(|m| msg.contains(m)) {
        jobs.log(job_id, "info", &msg).await;
    } else {
        jobs.log(job_id, "debug", &msg).await;
    }
}

/// 边跑边读子进程日志：里程碑进日志、`chunk#N: P%` 进进度条、长时间静默告警。
async fn follow_render_output(
    jobs: Arc<super::jobs::JobManager>,
    job_id: String,
    mut out: RenderOutput,
    stop: Arc<std::sync::atomic::AtomicBool>,
) -> RenderOutput {
    use std::sync::atomic::Ordering;
    loop {
        for line in out.drain() {
            emit_render_line(&jobs, &job_id, &mut out, &line).await;
        }
        if stop.load(Ordering::SeqCst) {
            for line in out.drain() {
                emit_render_line(&jobs, &job_id, &mut out, &line).await;
            }
            return out;
        }
        // 静默告警：子进程会周期性打分块进度，60s 一行都没有 = 很可能卡住了。
        // 这正是过去"渲染长时间无反应、日志里却什么都没有"的那段时间。
        let quiet = out.last_active.map(|t| t.elapsed().as_secs()).unwrap_or(0);
        if quiet >= 60
            && out
                .warned_silent
                .map(|w| w.elapsed().as_secs() >= 60)
                .unwrap_or(true)
        {
            out.warned_silent = Some(std::time::Instant::now());
            jobs.log(
                &job_id,
                "warn",
                format!("渲染子进程已 {quiet}s 无任何输出（可能卡在抽帧或某一分块上）"),
            )
            .await;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

/// 渲染中途失败/取消后清掉半成品：`_chunk_*.mp4` / `_temp_*`（成片与封面不动）。
fn cleanup_render_leftovers(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with("_chunk_") || name.starts_with("_temp_") {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

/// 跑一个 `vtrim polish <sub>` 子进程：输出**落文件**并流式回灌任务日志。
///
/// 为什么不是 `.output()`：Chrome/ffmpeg 会继承 stdout 句柄，只要还有一个活着，
/// 管道就永远不 EOF —— 子进程早就失败退出了，任务却永远停在"运行中"，连错误都看不到
/// （实测挂过两次，每次都留下 3 个 headless Chrome 空转两小时）。落文件 + `wait()`
/// 判定退出就没有这个问题。
///
/// 取消：子进程是**另一个进程**，看不到我们的进程内标志 —— 由 `cancel_watch`
/// 按 pid 杀掉整棵树（含 Chrome / ffmpeg）。
async fn run_polish_child(
    data_dir: &Path,
    jobs: &Arc<super::jobs::JobManager>,
    job_id: &str,
    sub: &str,
    dir: &Path,
) -> anyhow::Result<()> {
    use std::sync::atomic::{AtomicBool, Ordering};

    let log_path = super::render_proc::log_file(data_dir, job_id);
    if let Some(parent) = log_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let file = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .append(true)
        .open(&log_path)
        .map_err(|e| anyhow::anyhow!("创建渲染日志失败 {}: {e}", log_path.display()))?;
    let err_file = file
        .try_clone()
        .map_err(|e| anyhow::anyhow!("复制日志句柄失败: {e}"))?;

    let exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("vtrim"));
    let dir_str = dir.to_string_lossy().into_owned();
    let mut child = tokio::process::Command::new(&exe)
        .args(["polish", sub, &dir_str])
        .env("NO_COLOR", "1")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::from(file))
        .stderr(std::process::Stdio::from(err_file))
        .spawn()
        .map_err(|e| anyhow::anyhow!("{sub} 进程启动失败: {e}"))?;
    let pid = child.id().unwrap_or(0);
    if pid != 0 {
        super::render_proc::register(data_dir, job_id, pid);
    }
    jobs.log(
        job_id,
        "info",
        format!("{sub} 子进程已启动（pid {pid}）；完整输出: {}", log_path.display()),
    )
    .await;

    let stop = Arc::new(AtomicBool::new(false));
    let follow = tokio::spawn(follow_render_output(
        jobs.clone(),
        job_id.to_string(),
        RenderOutput::new(log_path.clone()),
        stop.clone(),
    ));
    let cancel_flag = jobs.cancel_token(job_id).await;
    let watch_stop = stop.clone();
    let cancel_watch = tokio::spawn(async move {
        let Some(flag) = cancel_flag else { return };
        if pid == 0 {
            return;
        }
        loop {
            if watch_stop.load(Ordering::SeqCst) {
                return;
            }
            if flag.load(Ordering::SeqCst) {
                tracing::warn!("渲染任务被取消，结束子进程树 pid {pid}");
                super::render_proc::kill_tree(pid);
                return;
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    });

    let status = child
        .wait()
        .await
        .map_err(|e| anyhow::anyhow!("等待 {sub} 进程失败: {e}"))?;
    stop.store(true, Ordering::SeqCst);
    super::render_proc::unregister(data_dir, job_id);
    let out = follow.await.unwrap_or_default();
    cancel_watch.abort();

    if !status.success() {
        cleanup_render_leftovers(dir);
        let tail = out.tail_text();
        anyhow::bail!("{sub} 进程退出码: {:?}\n{}", status.code(), tail);
    }
    // 成功也要把「写到哪个文件」带进日志：同名不覆盖，产出可能是 `xxx_1.mp4` /
    // `cover_2.jpg`（见 render::unique_output_path），不说清楚用户会以为"又覆盖了同一个文件"。
    let tail = out.tail_text();
    let (needle, label) = if sub == "cover" {
        ("Cover saved:", "封面已写出")
    } else {
        ("FINISHED:", "成片已写出")
    };
    if let Some(p) = subprocess_note(&tail, needle) {
        jobs.log(job_id, "info", &format!("{label}：{p}")).await;
    }
    Ok(())
}

// ===================== Polish =====================

fn polish_projects_impl(ws: &Workspace) -> ApiResult {
    let clips = ws.output_dir().join("clips");
    let mut out = vec![];
    if clips.exists() {
        for entry in std::fs::read_dir(&clips).map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("读取失败: {e}")))? {
            let entry = entry.map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("读取失败: {e}")))?;
            let dir = entry.path();
            if !dir.is_dir() { continue; }
            let title = std::fs::read_to_string(dir.join("clip.json")).ok()
                .and_then(|raw| serde_json::from_str::<serde_json::Value>(&raw).ok())
                .and_then(|v| v.get("title").and_then(|t| t.as_str()).map(|s| s.to_string()))
                .unwrap_or_default();
            let polish_ready = dir.join("polish.toml").exists();
            let processed = polish_ready && std::fs::read_to_string(dir.join("polish.toml")).ok()
                .map(|raw| raw.contains("processed = true")).unwrap_or(false);
            let rendered_file = latest_numbered_file(&dir, "vtuber-clip", "mp4");
            let rendered = rendered_file.is_some();
            let cover_file = latest_numbered_file(&dir, "cover", "jpg");
            let cover = cover_file.is_some();
            let mut version = "original";
            let mut mute_range_count = 0usize;
            if polish_ready {
                if let Ok(cfg) = vtrim_polish::project::load_project(&dir) {
                    // 消音版 = 静音区间已写入屏蔽轨道（video 始终是未改动的 clip.mp4）
                    if !cfg.silence_ranges.is_empty() { version = "silent"; }
                    mute_range_count = cfg.mute_ranges.len();
                }
            }
            out.push(serde_json::json!({
                "dir": vtrim_core::path::display(&dir),
                "title": title,
                "polish_ready": polish_ready,
                "processed": processed,
                "rendered": rendered,
                "rendered_file": rendered_file,
                "cover": cover,
                "cover_file": cover_file,
                "version": version,
                "mute_range_count": mute_range_count,
            }));
        }
    }
    out.sort_by(|a,b| a["dir"].as_str().unwrap_or("").cmp(b["dir"].as_str().unwrap_or("")));
    ok(serde_json::json!(out))
}

async fn polish_spawn(
    state: &AppState,
    owner: i64,
    project: Option<i64>,
    kind: &str,
    dir: String,
    output_file: Option<String>,
) -> ApiResult {
    // dir 必须先校验：`dir: ""` 会让 cmd_init 在 server 进程的 CWD 里写出
    // polish.toml / data / sfx，任意绝对路径也会被当作合法工程目录。
    // 归属由 `scope::enforce_scope` 中间件保证（只能是自己项目内的目录）。
    let path = validated_project_dir(&dir)?;
    let dir_name = path
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| dir.clone());
    // 资源域 = 这个片段目录：与同片段的其它精修任务、以及"单条重切"互斥（冲突则排队）。
    // 档位按 kind：渲染重活、LLM 略重、init/process/cover 轻活（`JobClass::for_polish`）。
    let keys = super::scope_key::polish_keys(project, &path);
    let class = JobClass::for_polish(kind);
    let handle = state
        .jobs
        .create(
            owner,
            JobSpec::new(format!("polish_{}", kind), format!("polish {kind} · {dir_name}"), keys, class),
        )
        .await;
    let id = handle.id.clone();
    let queued = handle.queued;
    if let Some(pid) = project {
        state.jobs.attach_project(&id, pid).await;
    }
    let jobs = state.jobs.clone();
    let job_id = id.clone();
    let kind = kind.to_string();
    // 子进程日志落在服务数据目录下；data_dir 先克隆出来（spawn 的任务要 'static）
    let data_dir = state.auth.data_dir().clone();
    // init/llm/process 都是整段同步代码（读改 polish.toml、逐段后处理），跑专用线程；
    // render/cover 是子进程 + 流式跟随，本来就不占 worker，跟着一起进去也无妨
    spawn_job(jobs.clone(), job_id.clone(), move || async move {
        if handle.wait_start().await.is_err() {
            // 排队期间被取消：没开工。complete 只为释放占位并唤醒后面的任务
            jobs.complete(&job_id).await;
            return;
        }
        let result = async {
            match kind.as_str() {
                "init" => vtrim_polish::commands::cmd_init(&path).await,
                // LLM 是显式步骤，process 隐式：完成后同一 job 内自动执行后处理
                "llm" => {
                    jobs.log(&job_id, "info", "LLM 生成中…").await;
                    vtrim_polish::commands::cmd_llm(&path).await?;
                    jobs.log(&job_id, "info", "LLM 完成，自动执行 process 后处理…").await;
                    vtrim_polish::process::cmd_process(&path).await
                }
                "process" => vtrim_polish::process::cmd_process(&path).await,
                "render" => {
                    // 输出文件名在**真开跑之后**才落到 polish.toml（排队期间写会覆盖别人
                    // 的改动，而且任务可能还要等很久）。update_polish_toml 自己带路径锁。
                    if let Some(name) = output_file.clone() {
                        vtrim_polish::project::update_polish_toml(&path, |cfg| {
                            cfg.project.output = name.clone();
                        })
                        .await?;
                    }
                    // 用外部 CLI 子进程渲染：headless Chrome 在 WebUI 服务进程内启动会超时。
                    // 输出流式回灌任务日志 + 取消时杀进程树，见 run_polish_child。
                    run_polish_child(&data_dir, &jobs, &job_id, "render", &path).await
                }
                "cover" => {
                    // 与 render 同理：封面也走外部 CLI（同样要能看进度、能取消）
                    run_polish_child(&data_dir, &jobs, &job_id, "cover", &path).await
                }
                _ => anyhow::bail!("未知 kind"),
            }
        }.await;
        if let Err(e) = result {
            jobs.fail(&job_id, format!("{e:#}")).await;
        } else {
            jobs.complete(&job_id).await;
        }
    });
    ok(serde_json::json!({ "ok": true, "id": id, "queued": queued }))
}

/// polish 路由的作用域项目 id（由 `scope::enforce_scope` 注入；0 = 未解析）。
fn scope_pid(ext: Option<axum::extract::Extension<super::scope::ScopeProjectId>>) -> Option<i64> {
    ext.map(|e| e.0.0).filter(|v| *v > 0)
}

async fn polish_init(State(state): State<Arc<AppState>>, Extension(user): Extension<super::auth::CurrentUser>, pid: Option<Extension<super::scope::ScopeProjectId>>, Json(body): Json<serde_json::Value>) -> ApiResult {
    let dir = body.get("dir").and_then(|v| v.as_str()).unwrap_or("").to_string();
    polish_spawn(&state, user.id, scope_pid(pid), "init", dir, None).await
}

async fn polish_llm(State(state): State<Arc<AppState>>, Extension(user): Extension<super::auth::CurrentUser>, pid: Option<Extension<super::scope::ScopeProjectId>>, Json(body): Json<serde_json::Value>) -> ApiResult {
    let dir = body.get("dir").and_then(|v| v.as_str()).unwrap_or("").to_string();
    polish_spawn(&state, user.id, scope_pid(pid), "llm", dir, None).await
}

async fn polish_process(State(state): State<Arc<AppState>>, Extension(user): Extension<super::auth::CurrentUser>, pid: Option<Extension<super::scope::ScopeProjectId>>, Json(body): Json<serde_json::Value>) -> ApiResult {
    let dir = body.get("dir").and_then(|v| v.as_str()).unwrap_or("").to_string();
    polish_spawn(&state, user.id, scope_pid(pid), "process", dir, None).await
}

async fn polish_render(State(state): State<Arc<AppState>>, Extension(user): Extension<super::auth::CurrentUser>, pid: Option<Extension<super::scope::ScopeProjectId>>, Json(body): Json<serde_json::Value>) -> ApiResult {
    let dir = body.get("dir").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let path = validated_project_dir(&dir)?;
    if !path.join("polish.toml").exists() { return Err(err(StatusCode::BAD_REQUEST, "目录未初始化 polish")); }
    // 输出文件名**不在这里写**：任务可能被排到后面，等它真开跑时再落到 polish.toml
    // （见 polish_spawn 的 render 分支）—— 否则排队期间别的任务/用户改动会被这次预写覆盖。
    let output_file = body
        .get("output_file")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());
    polish_spawn(&state, user.id, scope_pid(pid), "render", dir, output_file).await
}

async fn polish_cover(State(state): State<Arc<AppState>>, Extension(user): Extension<super::auth::CurrentUser>, pid: Option<Extension<super::scope::ScopeProjectId>>, Json(body): Json<serde_json::Value>) -> ApiResult {
    let dir = body.get("dir").and_then(|v| v.as_str()).unwrap_or("").to_string();
    polish_spawn(&state, user.id, scope_pid(pid), "cover", dir, None).await
}


fn cover_title_event_context(
    cfg: &vtrim_polish::project::ProjectFile,
    subtitles: &[vtrim_core::types::TranscriptSegment],
) -> String {
    use std::fmt::Write as _;

    let mut out = String::new();
    let muted = |start: f64, end: f64| cfg.mute_ranges.iter().any(|r| r.overlaps(start, end));

    if cfg.segments.is_empty() {
        let _ = writeln!(out, "polish.toml 暂无分段，按整片 clip.srt 字幕：");
        for s in subtitles.iter().filter(|s| !muted(s.start, s.end)) {
            let _ = writeln!(out, "[{:.2}-{:.2}] {}", s.start, s.end, s.text);
        }
        return out;
    }

    for (i, seg) in cfg.segments.iter().enumerate() {
        if muted(seg.start, seg.end) {
            continue;
        }
        let _ = write!(out, "[事件/分段 {}] {:.2}s-{:.2}s", i + 1, seg.start, seg.end);
        if !seg.note.trim().is_empty() {
            let _ = write!(out, " 备注: {}", seg.note.trim());
        }
        if !seg.fx.trim().is_empty() {
            let _ = write!(out, " 情绪: {}", seg.fx.trim());
        }
        let _ = writeln!(out, "");
        if !seg.events.is_empty() {
            let evs: Vec<String> = seg
                .events
                .iter()
                .map(|e| serde_json::to_string(e).unwrap_or_else(|_| "{}".into()))
                .collect();
            let _ = writeln!(out, "  特效事件: {}", evs.join(" | "));
        }
        let _ = writeln!(out, "  有效字幕:");
        let mut has = false;
        for sub in subtitles
            .iter()
            .filter(|s| s.end >= seg.start - 0.05 && s.start <= seg.end + 0.05)
            .filter(|s| !muted(s.start, s.end))
        {
            has = true;
            let _ = writeln!(out, "    [{:.2}-{:.2}] {}", sub.start, sub.end, sub.text);
        }
        if !has {
            let _ = writeln!(out, "    （无）");
        }
        if out.chars().count() > 8000 {
            let _ = writeln!(out, "（上下文过长，已截断）");
            break;
        }
    }
    out
}

fn sanitize_cover_title_line(line: &str) -> String {
    let mut s = line.trim().trim_matches(|c: char| {
        matches!(c, '"' | '“' | '”' | '`' | '【' | '】' | '[' | ']')
    });
    s = s.trim();
    s = s.trim_start_matches(|c: char| matches!(c, '▶' | '▸' | '•' | '·')).trim();
    for prefix in ["- ", "* ", "• ", "· "] {
        if let Some(rest) = s.strip_prefix(prefix) {
            s = rest.trim();
            break;
        }
    }
    if let Some(pos) = s.find(|c: char| c == '.' || c == '、' || c == ')' || c == '）') {
        let head = s[..pos].trim();
        if !head.is_empty() && head.chars().all(|c| c.is_ascii_digit()) {
            let step = s[pos..].chars().next().map(|c| c.len_utf8()).unwrap_or(0);
            s = s[pos + step..].trim();
        }
    }
    s.trim().to_string()
}

fn looks_like_cover_title_context(line: &str) -> bool {
    line.starts_with("[事件/分段")
        || line.starts_with("事件/分段")
        || line.starts_with("备注:")
        || line.starts_with("情绪:")
        || line.starts_with("特效事件:")
        || line.starts_with("有效字幕")
        || line.contains("title_lines")
        || (line.starts_with('{') && line.ends_with('}'))
        || (line.starts_with('[') && line.ends_with(']'))
}

fn push_cover_title_text(out: &mut Vec<String>, text: &str) {
    for part in text.lines() {
        let line = sanitize_cover_title_line(part);
        if !line.is_empty() && !looks_like_cover_title_context(&line) {
            out.push(line);
        }
    }
}

fn title_lines_from_json_value(value: &serde_json::Value) -> Option<Vec<String>> {
    if let Some(arr) = value.get("title_lines").and_then(|x| x.as_array()) {
        let mut out = Vec::new();
        for item in arr {
            if let Some(text) = item.as_str() {
                push_cover_title_text(&mut out, text);
            }
        }
        if !out.is_empty() {
            return Some(out);
        }
    }
    if let Some(text) = value.get("title_lines").and_then(|x| x.as_str()) {
        let mut out = Vec::new();
        push_cover_title_text(&mut out, text);
        if !out.is_empty() {
            return Some(out);
        }
    }
    if let Some(arr) = value.as_array() {
        let mut out = Vec::new();
        for item in arr {
            if let Some(text) = item.as_str() {
                push_cover_title_text(&mut out, text);
            } else if let Some(inner) = title_lines_from_json_value(item) {
                return Some(inner);
            }
        }
        if !out.is_empty() {
            return Some(out);
        }
    }
    if let Some(obj) = value.as_object() {
        for child in obj.values() {
            if let Some(inner) = title_lines_from_json_value(child) {
                return Some(inner);
            }
        }
    }
    None
}

fn find_matching_json_end(text: &str, start: usize) -> Option<usize> {
    let open = *text.as_bytes().get(start)?;
    let close = if open == b'{' { b'}' } else { b']' };
    let mut depth = 0i32;
    let mut in_string = false;
    let mut escaped = false;
    for (i, b) in text.bytes().enumerate().skip(start) {
        if in_string {
            if escaped {
                escaped = false;
            } else if b == 92 {
                escaped = true;
            } else if b == b'"' {
                in_string = false;
            }
            continue;
        }
        if b == b'"' {
            in_string = true;
        } else if b == open {
            depth += 1;
        } else if b == close {
            depth -= 1;
            if depth == 0 {
                return Some(i);
            }
        }
    }
    None
}

fn parse_cover_title_lines(content: &str) -> Vec<String> {
    let text = content.trim();
    let mut candidates: Vec<String> = vec![text.to_string()];
    if let Some(pos) = text.find("```") {
        let body = &text[pos + 3..];
        let end = body.find("```").unwrap_or(body.len());
        let fenced = body[..end].trim().trim_start_matches("json").trim();
        if !fenced.is_empty() {
            candidates.push(fenced.to_string());
        }
    }
    for open in ['{', '['] {
        if let Some(start) = text.find(open) {
            if let Some(end) = find_matching_json_end(text, start) {
                candidates.push(text[start..=end].to_string());
            }
        }
    }
    for cand in &candidates {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(cand) {
            if let Some(lines) = title_lines_from_json_value(&v) {
                if !lines.is_empty() {
                    return lines;
                }
            }
        }
    }
    let mut out = Vec::new();
    for line in text.lines() {
        let line = sanitize_cover_title_line(line);
        if !line.is_empty() && !looks_like_cover_title_context(&line) {
            out.push(line);
        }
    }
    out
}

fn is_emoji_char(c: char) -> bool {
    let u = c as u32;
    (0x1F300..=0x1FAFF).contains(&u)
        || (0x1F000..=0x1F2FF).contains(&u)
        || (0x2600..=0x27BF).contains(&u)
        || (0x2190..=0x21FF).contains(&u)
        || (0x2B00..=0x2BFF).contains(&u)
}

fn first_emoji(s: &str) -> Option<char> {
    s.chars().find(|c| is_emoji_char(*c))
}

fn count_emoji_in_lines(lines: &[String]) -> usize {
    lines
        .iter()
        .map(|l| l.chars().filter(|c| is_emoji_char(*c)).count())
        .sum()
}

fn title_char_count(lines: &[String]) -> usize {
    lines
        .iter()
        .flat_map(|l| l.chars())
        .filter(|c| !c.is_whitespace())
        .count()
}

fn trim_title_lines(lines: &mut Vec<String>, max_chars: usize) {
    if title_char_count(lines) <= max_chars {
        return;
    }
    let mut budget = max_chars;
    let mut out: Vec<String> = Vec::new();
    for line in lines.iter() {
        if budget == 0 {
            break;
        }
        let mut s = String::new();
        for ch in line.chars() {
            if ch.is_whitespace() {
                s.push(ch);
                continue;
            }
            if budget == 0 {
                break;
            }
            s.push(ch);
            budget -= 1;
        }
        let s = s.trim_end().to_string();
        if !s.is_empty() {
            out.push(s);
        }
    }
    *lines = out;
}

fn keep_single_emoji(lines: &mut Vec<String>) {
    let mut seen = false;
    for line in lines.iter_mut() {
        let mut out = String::new();
        for ch in line.chars() {
            if is_emoji_char(ch) {
                if seen {
                    continue;
                }
                seen = true;
            }
            out.push(ch);
        }
        *line = out;
    }
}

async fn polish_cover_title(State(_state): State<Arc<AppState>>, Json(body): Json<serde_json::Value>) -> ApiResult {
    let dir = body.get("dir").and_then(|v| v.as_str()).ok_or_else(|| err(StatusCode::BAD_REQUEST, "缺少 dir"))?;
    let path = validated_project_dir(dir)?;
    if !path.join("polish.toml").exists() {
        return Err(err(StatusCode::BAD_REQUEST, "目录未初始化 polish"));
    }

    // 只使用 clip.srt + polish.toml 作为输入；
    // project.name 仅作为“参考标题”参与生成，不读取 data/subtitles.json / clip-meta.json / clip.json。
    let cfg = vtrim_polish::project::load_project(&path)
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("加载 polish.toml 失败: {e}")))?;
    let srt_path = path.join("clip.srt");
    if !srt_path.exists() {
        return Err(err(StatusCode::BAD_REQUEST, "clip.srt 不存在"));
    }
    let raw = std::fs::read_to_string(&srt_path)
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("读取 clip.srt 失败: {e}")))?;
    let transcript = vtrim_core::srt::parse_srt(&raw)
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("解析 clip.srt 失败: {e}")))?;
    if transcript.segments.is_empty() {
        return Err(err(StatusCode::BAD_REQUEST, "clip.srt 暂无字幕，无法生成标题"));
    }
    let context = cover_title_event_context(&cfg, &transcript.segments);
    let global_cfg = vtrim_core::config::load_global_config();
    let reference_titles = vtrim_core::config::resolve_reference_titles(&global_cfg.detection);
    let reference_text = if !reference_titles.is_empty() {
        reference_titles.join("\n")
    } else {
        let raw = cfg.project.name.trim();
        let cleaned = raw.split('【').next().unwrap_or(raw).trim();
        if !cleaned.is_empty() {
            cleaned.to_string()
        } else {
            cfg.cover.title_lines.join(" / ")
        }
    };

    let llm = vtrim_core::config::load_llm_config();
    if llm.base_url.is_empty() || llm.api_key.is_empty() {
        return Err(err(StatusCode::BAD_REQUEST, "LLM 未配置"));
    }
    let client = vtrim_core::llm::LlmClient::from_config(&llm)
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("LLM 配置错误: {e}")))?
        .with_timeout(60)
        .with_thinking(Some("disabled"))
        .with_usage_context(
            vtrim_core::llm::usage::UsageContext::new("封面文案")
                .project(Some(vtrim_core::path::display(std::path::Path::new(&dir)))),
        );

    let system = format!(r#"你是直播切片封面标题助手。为当前切片生成封面大字标题，必须沿用事件检测阶段的标题规则：
{}
封面补充要求：
- 输出 1~3 行，适合封面大字排版；整组标题总字数严格 8~40 字，不要超长
- 只输出封面标题本身，不要输出 ▶ / 事件 / 分段 / 备注 / 上下文原文
- 必须且只能包含 1 个 emoji，放在句尾，不要放开头；参考标题有多个 emoji 也不要照搬
- 不要引号、不要解释、不要 markdown
- 参考标题只用于提取信息点，必须重新组织，不要照抄
- 只能依据输入的 polish.toml 逐个事件/分段与其对应的 clip.srt 字幕，不要引用输入之外的信息
- 输出 JSON：{{"title_lines": ["第一行", "第二行"]}}
"#, vtrim_core::detection::TITLE_RULES);
    let mut prompt_text = String::new();
    if !reference_text.is_empty() {
        prompt_text.push_str("参考标题示例（来自内建/全局配置，只参考句式、信息密度和 emoji，不要照抄）：\n");
        prompt_text.push_str(&reference_text);
        prompt_text.push_str("\n\n");
    }
    prompt_text.push_str("以下是 polish.toml 中的逐个事件/分段，以及 clip.srt 中对应字幕：\n");
    prompt_text.push_str(&context);
    prompt_text.push_str("\n请按上述事件标题规则生成封面标题。");
    let content = client.chat(&system, &prompt_text).await
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("LLM 生成失败: {e}")))?;

    let mut lines = parse_cover_title_lines(&content);
    // 封面最多 3 行 + 总字数 8~40，避免模型不守格式导致排版溢出
    lines.truncate(3);
    if lines.is_empty() {
        return Err(err(StatusCode::INTERNAL_SERVER_ERROR, "LLM 返回空标题"));
    }
    trim_title_lines(&mut lines, 40);
    keep_single_emoji(&mut lines);
    // 模型偶尔会漏 emoji：优先沿用参考标题里的 emoji，没有就补一个
    if count_emoji_in_lines(&lines) == 0 {
        trim_title_lines(&mut lines, 39);
        if let Some(last) = lines.last_mut() {
            let emoji = first_emoji(&reference_text).unwrap_or('✨');
            last.push(' ');
            last.push(emoji);
        }
    }
    ok(serde_json::json!({ "ok": true, "title_lines": lines }))
}

async fn polish_cover_config(State(_state): State<Arc<AppState>>, Json(body): Json<serde_json::Value>) -> ApiResult {
    let dir = body.get("dir").and_then(|v| v.as_str()).ok_or_else(|| err(StatusCode::BAD_REQUEST, "缺少 dir"))?;
    let path = validated_project_dir(dir)?;
    if !path.join("polish.toml").exists() { return Err(err(StatusCode::BAD_REQUEST, "未初始化 polish")); }
    // 整个 read-modify-write 期间持锁：封面设计保存与时间轴保存会并发写同一份 polish.toml
    let _guard = vtrim_core::filelock::lock_path(&path.join("polish.toml")).await;
    let mut cfg = vtrim_polish::project::load_project(&path).map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("加载失败: {e}")))?;
    if let Some(fp) = body.get("frame_pos").and_then(|v| v.as_f64()) { cfg.cover.frame_pos = fp.clamp(0.0,1.0); }
    if let Some(lines) = body.get("title_lines").and_then(|v| v.as_array()) {
        let ls = lines.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect::<Vec<_>>();
        cfg.cover.title_lines = ls;
    }
    if let Some(text) = body.get("title_text").and_then(|v| v.as_str()) {
        cfg.project.name = text.to_string();
        cfg.cover.title_lines = vec![text.to_string()];
    }
    if let Some(cover) = body.get("cover").and_then(|v| v.as_object()) {
        if let Some(v) = cover.get("frame_pos").and_then(|v| v.as_f64()) { cfg.cover.frame_pos = v.clamp(0.0,1.0); }
        // 封面字体：值 = 字体库里的 id，空串 = 回到系统字体栈（与旧工程一致）
        if let Some(v) = cover.get("font").and_then(|v| v.as_str()) { cfg.cover.font = v.trim().to_string(); }
        if let Some(v) = cover.get("font_size").and_then(|v| v.as_u64()) { cfg.cover.font_size = v as u32; }
        if let Some(v) = cover.get("stroke").and_then(|v| v.as_u64()) { cfg.cover.stroke = v as u32; }
        if let Some(v) = cover.get("hl_color").and_then(|v| v.as_str()) { cfg.cover.hl_color = v.to_string(); }
        if let Some(v) = cover.get("text_width").and_then(|v| v.as_f64()) { cfg.cover.text_width = v; }
        if let Some(v) = cover.get("text_x").and_then(|v| v.as_str()) { cfg.cover.text_x = v.to_string(); }
        if let Some(v) = cover.get("text_y").and_then(|v| v.as_str()) { cfg.cover.text_y = v.to_string(); }
        if let Some(v) = cover.get("gradient_dir").and_then(|v| v.as_str()) { cfg.cover.gradient_dir = v.to_string(); }
        if let Some(v) = cover.get("vignette").and_then(|v| v.as_f64()) { cfg.cover.vignette = v; }
        if let Some(v) = cover.get("letter_spacing").and_then(|v| v.as_str()) { cfg.cover.letter_spacing = v.to_string(); }
        if let Some(v) = cover.get("line_height").and_then(|v| v.as_f64()) { cfg.cover.line_height = v; }
        if let Some(v) = cover.get("text_align").and_then(|v| v.as_str()) { cfg.cover.text_align = v.to_string(); }
        if let Some(v) = cover.get("shadow_dir").and_then(|v| v.as_str()) { cfg.cover.shadow_dir = v.to_string(); }
        if let Some(v) = cover.get("shadow_intensity").and_then(|v| v.as_f64()) { cfg.cover.shadow_intensity = v.clamp(0.0, 1.0); }
        if let Some(v) = cover.get("text_shadow").and_then(|v| v.as_bool()) { cfg.cover.text_shadow = v; }
        if let Some(arr) = cover.get("highlight_keywords").and_then(|v| v.as_array()) {
            cfg.cover.highlight_keywords = arr.iter()
                .filter_map(|v| v.as_str().map(|s| s.trim().to_string()))
                .filter(|s| !s.is_empty())
                .collect();
        }
        if let Some(v) = cover.get("bg_x").and_then(|v| v.as_f64()) { cfg.cover.bg_x = v; }
        if let Some(v) = cover.get("bg_y").and_then(|v| v.as_f64()) { cfg.cover.bg_y = v; }
        if let Some(v) = cover.get("bg_zoom").and_then(|v| v.as_f64()) { cfg.cover.bg_zoom = v.clamp(0.5, 3.0); }
        // 背景模式 / 纯色 / 模糊倍率：颜色必须净化（它直接进 HTML 模板），
        // 倍率按 cover.rs 的同一组边界夹取，避免滑杆与服务端各夹各的
        if let Some(v) = cover.get("bg_mode").and_then(|v| v.as_str()) {
            cfg.cover.bg_mode = v.trim().to_ascii_lowercase();
        }
        if let Some(v) = cover.get("bg_color").and_then(|v| v.as_str()) {
            cfg.cover.bg_color = vtrim_polish::cover::sanitize_hex_color(v, "#000000");
        }
        if let Some(v) = cover.get("blur_scale").and_then(|v| v.as_f64()) {
            cfg.cover.blur_scale = v.clamp(
                vtrim_polish::cover::COVER_BLUR_SCALE_MIN,
                vtrim_polish::cover::COVER_BLUR_SCALE_MAX,
            );
        }
        if let Some(v) = cover.get("effects").and_then(|v| v.as_array()) {
            cfg.cover.effects = v.iter().filter_map(|e| {
                let kind = e.get("kind").and_then(|x| x.as_str())?.to_string();
                let amount = e.get("amount").and_then(|x| x.as_f64()).unwrap_or(0.3);
                let x = e.get("x").and_then(|x| x.as_f64()).unwrap_or(50.0);
                let y = e.get("y").and_then(|x| x.as_f64()).unwrap_or(50.0);
                let color = e.get("color").and_then(|x| x.as_str()).unwrap_or("#000000").to_string();
                let text = e.get("text").and_then(|x| x.as_str()).unwrap_or("").to_string();
                Some(vtrim_polish::project::CoverEffect { kind, amount, x, y, color, text })
            }).collect();
        }
        if let Some(lines) = cover.get("title_lines").and_then(|v| v.as_array()) {
            cfg.cover.title_lines = lines.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect();
        }
        if let Some(v) = cover.get("image").and_then(|v| v.as_str()) { cfg.cover.image = v.to_string(); }
    }
    let raw = cfg.to_toml_commented().map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("序列化失败: {e}")))?;
    std::fs::write(path.join("polish.toml"), raw).map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("写入失败: {e}")))?;
    ok(serde_json::json!({ "ok": true }))
}

/// 「去静音」快捷操作：把检测到的长静音段写进 / 移出 `mute_ranges`（时间轴「屏蔽」轨道）。
///
/// 不重编码、不改动视频文件；`action` 为 `apply`（默认）或 `clear`。
async fn polish_silence(State(_state): State<Arc<AppState>>, Json(body): Json<serde_json::Value>) -> ApiResult {
    let dir = body.get("dir").and_then(|v| v.as_str()).ok_or_else(|| err(StatusCode::BAD_REQUEST, "缺少 dir"))?;
    let action = body.get("action").and_then(|v| v.as_str()).unwrap_or("apply");
    let path = validated_project_dir(dir)?;
    if !path.join("polish.toml").exists() { return Err(err(StatusCode::BAD_REQUEST, "未初始化 polish")); }
    // 去静音要跑 ffmpeg silencedetect（整个视频）+ 读改写 polish.toml，全是同步重活：
    // 锁在 await 里持有，活儿下 blocking 池（`with_path_lock` 的闭包是在当前线程跑的）
    let lock_file = path.join("polish.toml");
    let _guard = vtrim_core::filelock::lock_path(&lock_file).await;
    let action = action.to_string();
    blocking(move || match action.as_str() {
        "clear" => {
            let removed = vtrim_polish::project::clear_silence(&path)
                .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("移除静音屏蔽失败: {e:#}")))?;
            ok(serde_json::json!({ "ok": true, "applied": false, "removed": removed, "total": vtrim_polish::project::mute_range_count(&path) }))
        }
        "apply" => {
            let applied = vtrim_polish::project::apply_silence(&path)
                .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("写入屏蔽轨道失败: {e:#}")))?;
            // 与当前屏蔽状态**相加**：added = 真正新加进去的区间，total = 相加后的轨道总段数
            ok(serde_json::json!({
                "ok": true,
                "applied": !applied.ranges.is_empty(),
                "detected": applied.ranges.len(),
                "added": applied.added,
                "total": applied.total,
                "ranges": applied.ranges.iter().map(|(s, e)| serde_json::json!({ "start": s, "end": e })).collect::<Vec<_>>(),
                "message": if applied.ranges.is_empty() {
                    "未检测到长静音段"
                } else if applied.added == 0 {
                    "检测到的静音已在屏蔽轨道里"
                } else {
                    ""
                },
            }))
        }
        _ => Err(err(StatusCode::BAD_REQUEST, "action 必须是 apply 或 clear")),
    })
    .await
}

/// 兼容旧接口：version=silent/original 等价于「去静音」的 apply/clear。
async fn polish_select_version(State(state): State<Arc<AppState>>, Json(body): Json<serde_json::Value>) -> ApiResult {
    let version = body.get("version").and_then(|v| v.as_str()).unwrap_or("original");
    let action = if version == "silent" { "apply" } else { "clear" };
    let mut forwarded = body.clone();
    forwarded["action"] = serde_json::json!(action);
    polish_silence(State(state), Json(forwarded)).await
}

/// 取（或启动）该工程目录的预览服务端口，按 `dir|kind` 复用。
///
/// `RenderServer` 起的监听无法关闭：旧实现每次 `/polish/preview`、
/// `/polish/cover-preview` 都新起一个，端口与后台任务会无限泄漏。
/// 服务端在每次请求时按磁盘配置重建 HTML，所以复用不会端出过期内容。
async fn preview_port_for(state: &AppState, dir: &Path, kind: &str) -> anyhow::Result<u16> {
    let key = format!("{}|{kind}", dir.display());
    let cached = |key: &str| -> Option<u16> {
        state
            .preview_servers
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(key)
            .copied()
    };
    if let Some(port) = cached(&key) {
        return Ok(port);
    }
    // 同目录串行：并发首访不要同时起两个监听
    let _guard = vtrim_core::filelock::lock_path(&dir.join(format!(".vtrim-preview-{kind}"))).await;
    if let Some(port) = cached(&key) {
        return Ok(port);
    }
    let mut server = vtrim_polish::render::server::RenderServer::new(dir.to_path_buf());
    let port = if kind == "cover" {
        server.start_cover().await?
    } else {
        server.start().await?
    };
    state
        .preview_servers
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(key, port);
    Ok(port)
}

async fn polish_preview(State(state): State<Arc<AppState>>, Json(body): Json<serde_json::Value>) -> ApiResult {
    let dir = body.get("dir").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let path = validated_project_dir(dir)?;
    if !path.join("polish.toml").exists() {
        return Err(err(StatusCode::BAD_REQUEST, "目录未初始化 polish"));
    }
    // 启动预览服务（不阻塞，返回 URL 供前端 iframe）
    let config = vtrim_polish::project::load_project(&path).map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("加载失败: {e}")))?;
    if !config.processed {
        // 隐式 process 只在这个片段目录空闲时才跑：**别的**片段/项目的任务不该挡住它
        // （以前只要有任何任务在跑就跳过），同目录有任务才真的会撞。
        if state
            .jobs
            .busy_with(&super::scope_key::polish_keys(None, &path))
            .await
            .is_some()
        {
            tracing::info!("该片段有任务在运行，跳过预览前的隐式 process: {}", path.display());
        } else {
            let _ = vtrim_polish::process::cmd_process(&path).await;
        }
    }
    vtrim_polish::process::ensure_assets(&path).await;
    let port = preview_port_for(&state, &path, "video")
        .await
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("预览启动失败: {e:#}")))?;
    let video_url = format!("http://127.0.0.1:{}/", port);
    // 前端需要通过 API 代理访问本机 127.0.0.1 随机端口；这里直接返回绝对 URL（浏览器同源可访问）
    ok(serde_json::json!({ "ok": true, "video_url": video_url, "cover_url": video_url }))
}

async fn polish_cover_background(State(_state): State<Arc<AppState>>, Json(body): Json<serde_json::Value>) -> ApiResult {
    let dir = body.get("dir").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let path = validated_project_dir(dir)?;
    if !path.join("polish.toml").exists() {
        return Err(err(StatusCode::BAD_REQUEST, "目录未初始化 polish"));
    }
    if let Some(data) = body.get("image_data").and_then(|v| v.as_str()) {
        // 与其它 polish.toml 写入串行化
        let _guard = vtrim_core::filelock::lock_path(&path.join("polish.toml")).await;
        use base64::Engine as _;
        let bytes = base64::engine::general_purpose::STANDARD.decode(data)
            .map_err(|e| err(StatusCode::BAD_REQUEST, format!("图片解码失败: {e}")))?;
        if bytes.len() > 32 * 1024 * 1024 {
            return Err(err(StatusCode::BAD_REQUEST, "背景图过大（上限 32MB）"));
        }
        // ⚠️ 写**背景层自己的文件** `_cover_bg.jpg`：封面三层 = 文字 / 关键帧(_cover_frame.jpg) / 背景，
        // 导入背景不能把关键帧那层盖掉（用户报过"又把关键帧那一层给我删了"）。
        std::fs::write(path.join(vtrim_polish::cover::COVER_BG_IMG), bytes)
            .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("写入失败: {e}")))?;
        let mut cfg = vtrim_polish::project::load_project(&path)
            .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("加载失败: {e}")))?;
        // 导入即切到「自定义图片」背景；关键帧与 cover.image 都不动
        cfg.cover.bg_mode = "image".into();
        let raw = cfg.to_toml_commented().map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("序列化失败: {e}")))?;
        std::fs::write(path.join("polish.toml"), raw).map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("写入失败: {e}")))?;
        return ok(serde_json::json!({ "ok": true, "bg_mode": "image" }));
    }
    if let Some(src) = body.get("source").and_then(|v| v.as_str()) {
        // 与其它 polish.toml 写入串行化
        let _guard = vtrim_core::filelock::lock_path(&path.join("polish.toml")).await;
        let sp = PathBuf::from(src);
        if !sp.exists() { return Err(err(StatusCode::BAD_REQUEST, "背景文件不存在")); }
        // 同上：走背景层的文件，别碰关键帧
        std::fs::copy(&sp, path.join(vtrim_polish::cover::COVER_BG_IMG))
            .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("复制失败: {e}")))?;
        let mut cfg = vtrim_polish::project::load_project(&path)
            .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("加载失败: {e}")))?;
        cfg.cover.bg_mode = "image".into();
        let raw = cfg.to_toml_commented().map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("序列化失败: {e}")))?;
        std::fs::write(path.join("polish.toml"), raw).map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("写入失败: {e}")))?;
        return ok(serde_json::json!({ "ok": true }));
    }
    Err(err(StatusCode::BAD_REQUEST, "需要 image_data 或 source"))
}

async fn polish_cover_frame(State(_state): State<Arc<AppState>>, Json(body): Json<serde_json::Value>) -> ApiResult {
    let dir = body.get("dir").and_then(|v| v.as_str()).ok_or_else(|| err(StatusCode::BAD_REQUEST, "缺少 dir"))?;
    let path = validated_project_dir(dir)?;
    if !path.join("polish.toml").exists() {
        return Err(err(StatusCode::BAD_REQUEST, "目录未初始化 polish"));
    }
    // 与其它 polish.toml 写入串行化（本函数最后会写回 cover.image）；
    // 截帧要跑 ffmpeg（网络盘上数百 ms 到数秒）→ 整段下 blocking 池，别占 worker
    let _guard = vtrim_core::filelock::lock_path(&path.join("polish.toml")).await;
    let want_time = body
        .get("time")
        .and_then(|v| v.as_f64())
        .or_else(|| body.get("sec").and_then(|v| v.as_f64()));
    let time = blocking(move || {
        let config = vtrim_polish::project::load_project(&path)
            .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("加载失败: {e}")))?;
        let resolved = vtrim_polish::render::resolve_paths(&path, &config);
        let duration = config.segments.last().map(|s| s.end).unwrap_or(0.0);
        let time = want_time.unwrap_or(duration * config.cover.frame_pos).max(0.1);
        let frame_out = std::process::Command::new("ffmpeg")
            .args(["-ss", &time.to_string(), "-i", &resolved.video,
                   "-vframes", "1", "-f", "image2pipe", "-vcodec", "mjpeg", "-q:v", "3", "-"])
            .stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::null())
            .output().map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("ffmpeg 启动失败: {e}")))?;
        if frame_out.stdout.is_empty() {
            return Err(err(StatusCode::INTERNAL_SERVER_ERROR, "截帧失败，视频可能不可读或时间超出范围"));
        }
        std::fs::write(path.join("_cover_frame.jpg"), &frame_out.stdout)
            .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("写入封面帧失败: {e}")))?;
        // 使用截取的帧作为封面背景，并清空自定义 image，保证后续 cover 使用该帧
        let mut cfg = vtrim_polish::project::load_project(&path)
            .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("加载失败: {e}")))?;
        cfg.cover.image = String::new();
        let raw = cfg.to_toml_commented().map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("序列化失败: {e}")))?;
        std::fs::write(path.join("polish.toml"), raw).map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("写入失败: {e}")))?;
        Ok(time)
    })
    .await?;
    ok(serde_json::json!({ "ok": true, "time": time }))
}

async fn polish_cover_preview(State(state): State<Arc<AppState>>, Json(body): Json<serde_json::Value>) -> ApiResult {
    let dir = body.get("dir").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let path = validated_project_dir(dir)?;
    if !path.join("polish.toml").exists() {
        return Err(err(StatusCode::BAD_REQUEST, "目录未初始化 polish"));
    }
    // 缺关键帧时现场截一帧（ffmpeg，可能数百 ms~数秒）→ 整段下 blocking 池
    let cover_path = path.clone();
    let got_frame = blocking(move || {
        let config = vtrim_polish::project::load_project(&cover_path)
            .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("加载失败: {e}")))?;
        let resolved = vtrim_polish::render::resolve_paths(&cover_path, &config);
        let cover_image = cover_path.join("_cover_frame.jpg");
        if !cover_image.exists() {
            let duration = config.segments.last().map(|s| s.end).unwrap_or(0.0);
            let frame_time = (duration * config.cover.frame_pos).max(0.1);
            let frame_out = std::process::Command::new("ffmpeg")
                .args(["-ss", &frame_time.to_string(), "-i", &resolved.video,
                       "-vframes", "1", "-f", "image2pipe", "-vcodec", "mjpeg", "-q:v", "3", "-"])
                .stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::null())
                .output().map(|o| o.stdout);
            if let Ok(frame_data) = frame_out {
                if !frame_data.is_empty() {
                    std::fs::write(&cover_image, frame_data).ok();
                }
            }
        }
        Ok(cover_image.exists())
    })
    .await?;
    if !got_frame {
        return Err(err(StatusCode::BAD_REQUEST, "无法截取封面帧"));
    }
    // 封面服务按 dir|cover 复用；HTML 每次请求按当前 polish.toml 重建，改完设计刷新即可
    let port = preview_port_for(&state, &path, "cover")
        .await
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("封面预览启动失败: {e:#}")))?;
    let url = format!("http://127.0.0.1:{}/", port);
    ok(serde_json::json!({ "ok": true, "cover_url": url }))
}

/// 读精修字幕（`data/subtitles.json` 或 `clip.srt`）：项目目录上的同步 IO，下 blocking 池。
async fn polish_subtitles_get(State(state): State<Arc<AppState>>, Query(q): Query<std::collections::HashMap<String,String>>) -> ApiResult {
    blocking(move || block_on_here(polish_subtitles_get_impl(State(state), Query(q)))).await
}

async fn polish_subtitles_get_impl(
    State(_state): State<Arc<AppState>>,
    Query(q): Query<std::collections::HashMap<String,String>>,
) -> ApiResult {
    let dir = q.get("dir").ok_or_else(|| err(StatusCode::BAD_REQUEST, "缺少 dir"))?;
    let path = validated_project_dir(dir)?;
    if !path.join("polish.toml").exists() {
        return Err(err(StatusCode::BAD_REQUEST, "目录未初始化 polish"));
    }
    let json_path = path.join("data").join("subtitles.json");
    if json_path.exists() {
        let raw = std::fs::read_to_string(&json_path).map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("读取失败: {e}")))?;
        let v: serde_json::Value = serde_json::from_str(&raw).unwrap_or(serde_json::Value::Null);
        let segments = if let Some(arr) = v.as_array() {
            arr.clone()
        } else if let Some(arr) = v.get("segments").and_then(|x| x.as_array()) {
            arr.clone()
        } else {
            Vec::new()
        };
        return ok(serde_json::json!({ "segments": segments }));
    }
    let srt_path = path.join("clip.srt");
    if srt_path.exists() {
        let raw = std::fs::read_to_string(&srt_path).map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("读取失败: {e}")))?;
        let t = vtrim_core::srt::parse_srt(&raw).map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("解析失败: {e}")))?;
        return ok(serde_json::json!({ "segments": t.segments }));
    }
    Err(err(StatusCode::NOT_FOUND, "暂无字幕文件"))
}

async fn polish_subtitles_put(State(_state): State<Arc<AppState>>, Json(body): Json<serde_json::Value>) -> ApiResult {
    let dir = body.get("dir").and_then(|v| v.as_str()).ok_or_else(|| err(StatusCode::BAD_REQUEST, "缺少 dir"))?;
    let path = validated_project_dir(dir)?;
    if !path.join("polish.toml").exists() {
        return Err(err(StatusCode::BAD_REQUEST, "目录未初始化 polish"));
    }
    let segments = body.get("segments").cloned().ok_or_else(|| err(StatusCode::BAD_REQUEST, "缺少 segments"))?;
    let mut segs: Vec<vtrim_core::types::TranscriptSegment> = segments.as_array()
        .ok_or_else(|| err(StatusCode::BAD_REQUEST, "segments 必须是数组"))?
        .iter().map(|v| {
            let start = v.get("start").and_then(|x| x.as_f64()).unwrap_or(0.0);
            let end = v.get("end").and_then(|x| x.as_f64()).unwrap_or(start + 1.0);
            let text = v.get("text").and_then(|x| x.as_str()).unwrap_or("").to_string();
            let confidence = v.get("confidence").and_then(|x| x.as_f64()).unwrap_or(1.0);
            vtrim_core::types::TranscriptSegment { start, end, text, confidence }
        }).collect();
    // **按开始时间排序后再落盘**：字幕轨道必须有序 —— 去静音按相邻间隔算静音、
    // 预览按时间二分取当前字幕都依赖它。编辑器「拆分」曾经把后半段追加到数组末尾
    // （`TimelineEditor.splitSelected`，已修），存过的那种文件是乱序的；
    // 这里顺手归一化，用户再存一次就自愈（老数据不用手工修）。
    segs.sort_by(|a, b| a.start.partial_cmp(&b.start).unwrap_or(std::cmp::Ordering::Equal));
    let data_dir = path.join("data");
    std::fs::create_dir_all(&data_dir).map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("创建目录失败: {e}")))?;
    // 写回 data/subtitles.json，保持 polish 使用的列表结构
    let out: Vec<serde_json::Value> = segs.iter().enumerate().map(|(i, seg)| {
        serde_json::json!({
            "index": i + 1,
            "start": seg.start,
            "end": seg.end,
            "text": seg.text,
            "html": seg.text,
        })
    }).collect();
    std::fs::write(data_dir.join("subtitles.json"), serde_json::to_string_pretty(&out).unwrap_or_default())
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("写入失败: {e}")))?;
    // 同步写回 clip.srt，方便外部工具读取
    let transcript = vtrim_core::types::Transcript {
        segments: segs,
        language: body.get("language").and_then(|l| l.as_str()).unwrap_or("zh").to_string(),
        asr_metadata: None,
    };
    vtrim_core::srt::write_srt(&transcript, &path.join("clip.srt"))
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("写 SRT 失败: {e}")))?;
    ok(serde_json::json!({ "ok": true, "saved": out.len() }))
}

/// 响度曲线（带缓存）：同一源视频只测一次，结果存 `data/loudness.json`。
///
/// 实现已收敛到 `vtrim_polish::render::measure_loudness_curve_cached`（出片页与混音共用同一份）。
fn loudness_curve_cached(project_dir: &Path, video_path: &Path) -> serde_json::Value {
    let curve = vtrim_polish::render::measure_loudness_curve_cached(project_dir, video_path);
    let points: Vec<serde_json::Value> = curve
        .iter()
        .map(|(t, db)| serde_json::json!({ "t": t, "db": db }))
        .collect();
    serde_json::json!(points)
}

/// 读一个精修工程（`polish.toml` + 贴纸映射 + 探测视频尺寸 + 响度曲线）——
/// 全是同步 IO / ffprobe，工程又在项目目录里（可能网络盘）。整段下 blocking 池：
/// 精修页每次加载与每次保存后都会打这个接口。
async fn polish_project_get(State(state): State<Arc<AppState>>, Query(q): Query<std::collections::HashMap<String,String>>) -> ApiResult {
    blocking(move || block_on_here(polish_project_get_impl(State(state), Query(q)))).await
}

async fn polish_project_get_impl(
    State(_state): State<Arc<AppState>>,
    Query(q): Query<std::collections::HashMap<String,String>>,
) -> ApiResult {
    let dir = q.get("dir").ok_or_else(|| err(StatusCode::BAD_REQUEST, "缺少 dir"))?;
    let path = validated_project_dir(dir)?;
    let cfg = vtrim_polish::project::load_project(&path).map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("加载失败: {e}")))?;
    let mut value = serde_json::to_value(&cfg).unwrap_or_default();
    // preview=1: 附加预览渲染所需的计算字段（sticker_map / 竖屏检测 / 响度曲线）
    if q.get("preview").map(|v| v == "1").unwrap_or(false) {
        if let Some(obj) = value.as_object_mut() {
            obj.insert("_assets".into(), serde_json::json!({
                "sticker_map": vtrim_polish::render::server::build_sticker_map(&path),
            }));
            let video_path = path.join(&cfg.source.video);
            if video_path.exists() {
                if let Some((w, h)) = vtrim_polish::render::get_video_size(&video_path.to_string_lossy()) {
                    if h > w {
                        let pos = obj.get("layout")
                            .and_then(|l| l.get("vertical_position"))
                            .and_then(|p| p.as_str())
                            .unwrap_or("left")
                            .to_string();
                        obj.insert("vertical".into(), serde_json::json!(true));
                        obj.insert("vertical_position".into(), serde_json::json!(pos));
                    }
                }
                obj.insert("loudness_curve".into(), loudness_curve_cached(&path, &video_path));
            }
        }
    }
    ok(value)
}

/// 单帧真渲染预览：用常驻 wgpu 后端渲染 dir 项目在 t 时刻的合成画面，返回 JPEG。
#[cfg(feature = "wgpu-render")]
async fn polish_render_frame(
    State(state): State<Arc<AppState>>,
    Json(body): Json<serde_json::Value>,
) -> Response {
    use vtrim_polish::render::backend::FrameRenderer as _;
    use vtrim_polish::render::frame::TimedFrame;

    let Some(dir) = body.get("dir").and_then(|v| v.as_str()).map(PathBuf::from) else {
        return (StatusCode::BAD_REQUEST, Json(serde_json::json!({"ok": false, "error": "缺少 dir"}))).into_response();
    };
    let t = body.get("t").and_then(|v| v.as_f64()).unwrap_or(0.0).max(0.0);
    let fps = 30usize;

    let cfg = {
        let t0 = std::time::Instant::now();
        let c = vtrim_polish::project::load_project(&dir);
        tracing::info!("render-frame: config load {}ms", t0.elapsed().as_millis());
        match c {
            Ok(c) => c,
            Err(e) => {
                return (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({"ok": false, "error": format!("加载失败: {e:#}")}))).into_response();
            }
        }
    };
    let video_path = dir.join(&cfg.source.video);

    let result = tokio::task::spawn_blocking(move || -> anyhow::Result<Vec<u8>> {
        let mut slot = state.preview.0.lock().expect("preview slot poisoned");
        // 目录切换 / 首次请求：重建后端（wgpu 设备常驻复用）
        let rebuild = match slot.as_ref() {
            Some(b) => b.dir != dir,
            None => true,
        };
        if rebuild {
            let backend = vtrim_polish::render::native::WgpuBackend::new(&dir, &cfg)?;
            *slot = Some(super::model::PreviewBackend {
                dir: dir.clone(),
                backend,
                last_t: -1.0,
                last_jpeg: None,
                video_size: None,
            });
        }
        let pb = slot.as_mut().unwrap();
        // 时间取整到帧网格，同帧命中缓存
        let round_t = (t * fps as f64).round() / fps as f64;
        if (pb.last_t - round_t).abs() < 1e-9 && pb.last_jpeg.is_some() {
            return Ok(pb.last_jpeg.clone().unwrap());
        }
        let mut ctx = vtrim_polish::render::RenderContext::new(dir.clone(), cfg.clone());
        ctx.fps = fps;
        // ffprobe 结果缓存（省 ~50ms/请求）
        if pb.video_size.is_none() {
            pb.video_size = vtrim_polish::render::get_video_size(&video_path.to_string_lossy());
        }
        let t_vs = std::time::Instant::now();
        if let Some((vw, vh)) = pb.video_size {
            if vh > vw {
                ctx.vertical = true;
            }
        }
        tracing::info!("render-frame: video size {}ms", t_vs.elapsed().as_millis());
        pb.backend.prepare(&ctx)?;
        let t_ex = std::time::Instant::now();
        let mut frames = vtrim_polish::render::frame::extract_chunk_frames(
            &video_path.to_string_lossy(),
            round_t,
            round_t + 1.0 / fps as f64,
            fps,
            true,
        )?;
        frames.truncate(1);
        if frames.is_empty() {
            anyhow::bail!("抽帧为空");
        }
        tracing::info!("render-frame: extract {}ms ({} frames)", t_ex.elapsed().as_millis(), frames.len());
        let t_rd = std::time::Instant::now();
        let jpeg = pb.backend.render_chunk(&frames)?.remove(0);
        tracing::info!("render-frame: render {}ms", t_rd.elapsed().as_millis());
        pb.last_t = round_t;
        pb.last_jpeg = Some(jpeg.clone());
        Ok(jpeg)
    })
    .await;

    match result {
        Ok(Ok(jpeg)) => ([(axum::http::header::CONTENT_TYPE, "image/jpeg")], jpeg).into_response(),
        Ok(Err(e)) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"ok": false, "error": format!("{e:#}")})),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"ok": false, "error": format!("任务失败: {e}")})),
        )
            .into_response(),
    }
}

/// feature 未启用时的占位响应。
#[cfg(not(feature = "wgpu-render"))]
async fn polish_render_frame(
    State(state): State<Arc<AppState>>,
    Json(body): Json<serde_json::Value>,
) -> Response {
    let _ = (state, body);
    (
        StatusCode::NOT_IMPLEMENTED,
        Json(serde_json::json!({"ok": false, "error": "wgpu-render feature 未启用（cargo build --features wgpu-render）"})),
    )
        .into_response()
}


/// 字体库全量（**只读**，非 admin 也能看）：字幕字体 / 封面字体的下拉数据源。
///
/// 库位置 = `<程序目录>/resources/fonts/`（`vtrim_core::path::font_dir`）。**以磁盘为准**：
/// 往里丢一个 `.ttf` / `.otf` / `.ttc` 就会出现在列表里，没有索引文件要维护。
/// `id` = 文件名去扩展名（配置里存的就是它）；`family` 是从字体 `name` 表读出来的展示名。
/// 扫描要读字体文件解析字族名，所以整段走 `blocking`（同步 IO 不能留在 worker 上）。
///
/// `root`（服务器上的绝对路径）**只给 admin**：非 admin 拿它没用（也丢不进文件），
/// 而它泄露的是服务器的安装路径 —— 与 `/api/admin/usage` 的理由同一条。
async fn polish_fonts(Extension(user): Extension<super::auth::CurrentUser>) -> ApiResult {
    let is_admin = user.is_admin();
    let payload = blocking(move || {
        let root = vtrim_core::path::font_dir();
        let fonts: Vec<serde_json::Value> = vtrim_core::fonts::scan()
            .into_iter()
            .map(|f| {
                serde_json::json!({
                    "id": f.id,
                    "file": f.file,
                    "family": f.family,
                    "bytes": f.bytes,
                })
            })
            .collect();
        Ok(serde_json::json!({
            "ok": true,
            // 出 JSON 的路径必须过 display（去掉 `\\?\` 逐字前缀），否则前端拼路径会炸
            "root": if is_admin { vtrim_core::path::display(&root) } else { String::new() },
            "fonts": fonts,
        }))
    })
    .await?;
    ok(payload)
}

/// 字体文件端点（wasm 预览 fetch 字体字节用）：项目字体候选 -> 内置资源。
async fn polish_font(Query(q): Query<std::collections::HashMap<String, String>>) -> Response {
    let name = q.get("name").cloned().unwrap_or_default();
    let dir = q.get("dir").map(std::path::PathBuf::from).unwrap_or_default();
    let mut candidates: Vec<PathBuf> = Vec::new();
    if !name.trim().is_empty() {
        // 只取文件名部分：name 来自查询参数，`..\..\x` 这类路径不能让字体接口越界读文件
        let base = Path::new(name.trim())
            .file_name()
            .map(PathBuf::from)
            .unwrap_or_default();
        if !base.as_os_str().is_empty() {
            for d in [
                dir.join("assets").join("fonts"),
                dir.join("fonts"),
                dir.clone(),
            ] {
                candidates.push(d.join(&base).with_extension("ttf"));
                candidates.push(d.join(&base).with_extension("otf"));
                candidates.push(d.join(&base).with_extension("ttc"));
            }
            // 程序级字体库（resources/fonts/）：`name` 就是字体 id（文件名去扩展名）。
            // 放在项目候选之后 —— 项目里自带的字体优先，库只是"别处都没有"时的来源。
            if let Some(p) = vtrim_core::fonts::resolve_file(name.trim()) {
                candidates.push(p);
            }
            candidates.push(Path::new("C:/Windows/Fonts").join(&base).with_extension("ttf"));
        }
    }
    candidates.push(std::path::Path::new("C:/Windows/Fonts/msyh.ttc").to_path_buf());
    // ⚠️ 这里**不再有内置字体文件**（发行包也刻意不带，见 README「字体」与 THIRD-PARTY-NOTICES.md）：
    // 候选链到系统字体（微软雅黑）就是最后一档兜底了。
    // 想用圆润可爱的标题风格，请**用户自行安装**乐米波波体（LemiBoBoTi，杭州乐米网络科技，
    // 官方授权地址 https://font.leminet.cn/）——它的授权是「免费商用，但**不包括嵌入式用途**」，
    // 随软件分发/内嵌字体文件必须另行授权，所以别再把 ttf 塞回仓库或发布包。
    for c in &candidates {
        if c.exists() {
            match std::fs::read(c) {
                Ok(bytes) => {
                    return ([(axum::http::header::CONTENT_TYPE, font_content_type(c))], bytes)
                        .into_response();
                }
                Err(_) => continue,
            }
        }
    }
    StatusCode::NOT_FOUND.into_response()
}

/// 字体文件的 Content-Type 按扩展名给：以前恒 `font/ttf`，`.otf` / `.ttc` 会被标错
/// （浏览器按 MIME 决定要不要当字体用，别赌它容错）。
fn font_content_type(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .as_deref()
    {
        Some("otf") => "font/otf",
        Some("ttc") => "font/collection",
        Some("woff") => "font/woff",
        Some("woff2") => "font/woff2",
        _ => "font/ttf",
    }
}

async fn polish_project_put(State(_state): State<Arc<AppState>>, Json(body): Json<serde_json::Value>) -> ApiResult {
    let dir = body.get("dir").and_then(|v| v.as_str()).ok_or_else(|| err(StatusCode::BAD_REQUEST, "缺少 dir"))?;
    let path = validated_project_dir(dir)?;
    // polish.toml 的 read-modify-write：时间轴保存 / 屏蔽区间 / 贴纸配置可能同时提交，
    // 无锁时后写者会覆盖先写者（前端确实会并发发多个 PUT）。
    // 锁在 await 里持有，读改写下 blocking 池
    let _guard = vtrim_core::filelock::lock_path(&path.join("polish.toml")).await;
    blocking(move || polish_project_put_locked(&path, &body)).await
}

/// 同步实现（调用方保证持有 polish.toml 的进程内锁）。
fn polish_project_put_locked(dir: &Path, body: &serde_json::Value) -> ApiResult {
    let mut cfg = vtrim_polish::project::load_project(dir).map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("加载失败: {e}")))?;
    if let Some(segments) = body.get("segments") {
        // 老的第三方调用方 / 手写 JSON 里的 `duration` 在这里升级成 `end`（与 polish.toml 同一条路）
        let mut up = segments.clone();
        vtrim_polish::project::upgrade_event_times(&mut up);
        let segs = serde_json::from_value(up).map_err(|e| err(StatusCode::BAD_REQUEST, format!("segments 解析失败: {e}")))?;
        cfg.segments = segs;
    }
    if let Some(ranges) = body.get("mute_ranges") {
        let rs = serde_json::from_value(ranges.clone()).map_err(|e| err(StatusCode::BAD_REQUEST, format!("mute_ranges 解析失败: {e}")))?;
        cfg.mute_ranges = rs;
    }
    if let Some(layout) = body.get("layout") {
        let lay: vtrim_polish::project::Layout = serde_json::from_value(layout.clone()).map_err(|e| err(StatusCode::BAD_REQUEST, format!("layout 解析失败: {e}")))?;
        cfg.layout = lay;
    }
    if let Some(ss) = body.get("sticker_source") {
        let list: Vec<String> = serde_json::from_value(ss.clone()).map_err(|e| err(StatusCode::BAD_REQUEST, format!("sticker_source 解析失败: {e}")))?;
        cfg.sticker_source = list;
    }
    // 字幕字体：值 = 字体库里的 id（文件名去扩展名），空串 = 回到系统字体栈。
    // 只影响渲染与预览（process 的贴纸/运镜重算不读它），所以**保存即生效、不用重跑任务**。
    if let Some(f) = body.get("subtitle_font").and_then(|v| v.as_str()) {
        cfg.style.subtitle.font = f.trim().to_string();
    }
    let raw = cfg.to_toml_commented().map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("序列化失败: {e}")))?;
    std::fs::write(dir.join("polish.toml"), raw).map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("保存失败: {e}")))?;
    ok(serde_json::json!({ "ok": true }))
}

/// 可用的贴图来源：内置 emoji 码点池 + 全局贴纸库（exe 同级 stickers/）的包名与 tags。
/// LLM 生成时按 sticker_source 过滤可用的贴纸池。
async fn polish_sticker_sources() -> ApiResult {
    use vtrim_polish::assets::sticker_index::{load_package_meta, load_sticker_index};
    let root = vtrim_core::path::sticker_dir();
    let index = load_sticker_index(&root);
    let mut out = vec![serde_json::json!({
        "name": "emoji",
        "builtin": true,
        "description": "内置 emoji 码点池（按情绪匹配）",
        "tags": [],
    })];
    for name in &index.packages {
        let meta = load_package_meta(&root, name);
        let (desc, tags) = match meta {
            Some(m) => (m.description, m.tags.iter().map(|t| t.tag.clone()).collect::<Vec<_>>()),
            None => (String::new(), Vec::new()),
        };
        out.push(serde_json::json!({
            "name": name,
            "builtin": false,
            "description": desc,
            "tags": tags,
        }));
    }
    ok(serde_json::json!({ "sources": out }))
}

/// 贴纸库全量（**只读**，非 admin 也能看）：编辑器里「keyword 一级 → 贴纸二级」选择用。
///
/// 以磁盘为准（`scan_packages`），每个包给齐 `tags[].files`（`<包名>.toml` 的反向索引）
/// 与全部文件名 —— 未构建的包也给文件，只是没有 tag（前端把它们平铺成一级条目）。
async fn polish_sticker_library() -> ApiResult {
    use vtrim_polish::assets::sticker_index::{
        list_package_files, load_package_meta, load_sticker_index, scan_packages,
    };
    let root = vtrim_core::path::sticker_dir();
    let index = load_sticker_index(&root);
    let packs: Vec<serde_json::Value> = scan_packages(&root)
        .into_iter()
        .map(|pkg| {
            let meta = load_package_meta(&root, &pkg.name);
            let files = list_package_files(&root.join(&pkg.name));
            let tags: Vec<serde_json::Value> = meta
                .as_ref()
                .map(|m| {
                    m.tags
                        .iter()
                        .map(|t| serde_json::json!({ "tag": t.tag, "files": t.files }))
                        .collect()
                })
                .unwrap_or_default();
            serde_json::json!({
                "name": pkg.name,
                "built": meta.is_some(),
                "registered": index.packages.iter().any(|n| n == &pkg.name),
                "description": meta.as_ref().map(|m| m.description.clone()).unwrap_or_default(),
                "tags": tags,
                "files": files,
            })
        })
        .collect();
    ok(serde_json::json!({ "ok": true, "packs": packs }))
}

/// 音效目录（只读）：编辑器里音效事件/特效配声的列表选择。
///
/// 数据来自编译进二进制的 `assets/sfx_catalog.json`；全局 `[sfx] blacklist`
/// 屏蔽掉的整族不列出来（选了也不会被解析）。
async fn polish_sfx_catalog() -> ApiResult {
    use vtrim_polish::assets::sfx_catalog;
    let blacklist = vtrim_core::config::load_config_at(std::path::Path::new("."))
        .map(|c| c.sfx.blacklist)
        .unwrap_or_default();
    let entries: Vec<serde_json::Value> = sfx_catalog::all()
        .iter()
        .filter(|e| !sfx_catalog::is_blacklisted(&e.id, &blacklist))
        .map(|e| {
            serde_json::json!({
                "id": e.id,
                "label": e.label,
                "cat": e.cat,
                "tags": e.tags,
            })
        })
        .collect();
    ok(serde_json::json!({ "ok": true, "entries": entries }))
}

/// 贴纸库里的单张图（编辑器缩略图直连 `<img>`）；只读 + 路径安全校验，与 admin 版同款。
async fn polish_sticker_file(
    axum::extract::Query(q): axum::extract::Query<std::collections::HashMap<String, String>>,
    headers: axum::http::HeaderMap,
) -> axum::response::Response {
    use vtrim_polish::assets::sticker_index::{is_safe_component, sanitize_pack_name};
    let root = vtrim_core::path::sticker_dir();
    let pack = q.get("pack").map(String::as_str).unwrap_or("");
    let name = q.get("name").map(String::as_str).unwrap_or("");
    let Some(pack) = sanitize_pack_name(pack) else {
        return super::media::json_error(StatusCode::BAD_REQUEST, "包名不合法");
    };
    if !is_safe_component(name) {
        return super::media::json_error(StatusCode::BAD_REQUEST, "文件名不合法");
    }
    let path = root.join(&pack).join(name);
    let real = path.canonicalize().unwrap_or_else(|_| path.clone());
    let root_real = root.canonicalize().unwrap_or_else(|_| root.clone());
    if !real.starts_with(&root_real) {
        return super::media::json_error(StatusCode::NOT_FOUND, "文件不在贴纸库内");
    }
    let mime = mime_guess::from_path(&path).first_or_octet_stream().to_string();
    let range = headers
        .get(axum::http::header::RANGE)
        .and_then(|v| v.to_str().ok());
    super::media::serve_file(&path, &mime, range).await
}

// core runner submodule
pub(crate) mod core {
    use std::path::PathBuf;
    use std::sync::Arc;
    use tokio::runtime::Handle;

    use crate::server::jobs::JobManager;

    /// WebUI 任务执行入口：复用 vtrim_service::core_pipeline，只做 Job/进度桥接。
    pub async fn run_stages(
        config_file: PathBuf,
        stages: Vec<String>,
        dry_run: bool,
        resume: bool,
        job_id: String,
        jobs: Arc<JobManager>,
        cancel: Option<Arc<std::sync::atomic::AtomicBool>>,
    ) -> anyhow::Result<()> {
        let progress: Arc<dyn vtrim_core::progress::ProgressReporter> =
            Arc::new(WebProgress::new(job_id, jobs, cancel));
        vtrim_service::run_stages(
            &config_file,
            &stages,
            dry_run,
            resume,
            Some(progress),
        ).await
    }

    /// 任务进度桥：把 `ProgressReporter` 的 `stage_*` 调用映射成 Job 事件。
    ///
    /// 管线之外的异步任务（如表情包的 LLM 标签构建）也走这一份 —— 进度只该有一套桥。
    pub(crate) fn progress_bridge(
        job_id: String,
        jobs: Arc<JobManager>,
        cancel: Option<Arc<std::sync::atomic::AtomicBool>>,
    ) -> Arc<dyn vtrim_core::progress::ProgressReporter> {
        Arc::new(WebProgress::new(job_id, jobs, cancel))
    }

    struct WebProgress {
        id: String,
        jobs: Arc<JobManager>,
        /// 任务的取消令牌：管线在阶段边界轮询它。
        cancel: Option<Arc<std::sync::atomic::AtomicBool>>,
    }

    impl WebProgress {
        fn new(
            id: String,
            jobs: Arc<JobManager>,
            cancel: Option<Arc<std::sync::atomic::AtomicBool>>,
        ) -> Self {
            Self { id, jobs, cancel }
        }
    }

    #[allow(clippy::let_underscore_future)]
    impl vtrim_core::progress::ProgressReporter for WebProgress {
        fn cancelled(&self) -> bool {
            self.cancel
                .as_ref()
                .map(|c| c.load(std::sync::atomic::Ordering::SeqCst))
                .unwrap_or(false)
        }
        fn stage_started(&self, name: &str) {
            let jobs = self.jobs.clone();
            let id = self.id.clone();
            let name = name.to_string();
            if let Ok(handle) = Handle::try_current() {
                let _ = handle.spawn(async move {
                    jobs.stage_started(&id, &name, stage_num(&name)).await;
                });
            }
        }
        fn stage_progress(&self, current: u64, total: u64) {
            let jobs = self.jobs.clone();
            let id = self.id.clone();
            if let Ok(handle) = Handle::try_current() {
                let _ = handle.spawn(async move {
                    jobs.progress(&id, current, total).await;
                });
            }
        }
        fn stage_completed(&self, name: &str) {
            let jobs = self.jobs.clone();
            let id = self.id.clone();
            let name = name.to_string();
            if let Ok(handle) = Handle::try_current() {
                let _ = handle.spawn(async move {
                    jobs.stage_completed(&id, &name).await;
                });
            }
        }
        fn error(&self, msg: &str) {
            let jobs = self.jobs.clone();
            let id = self.id.clone();
            let msg = msg.to_string();
            if let Ok(handle) = Handle::try_current() {
                let _ = handle.spawn(async move {
                    jobs.fail(&id, msg).await;
                });
            }
        }
    }

    fn stage_num(name: &str) -> u32 {
        match name {
            "AudioExtractor" => 1,
            "Transcriber" => 2,
            "EventDetector" => 3,
            "Cutter" => 4,
            _ => 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request as HttpRequest;
    use tower::util::ServiceExt;

    fn tmp_workspace(tag: &str) -> (Workspace, PathBuf) {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let root = std::env::temp_dir().join(format!("vtrim-api-{tag}-{}-{nanos}", std::process::id()));
        std::fs::create_dir_all(root.join("output")).unwrap();
        let ws = Workspace {
            config_file: root.join("vtrim.toml"),
            root_dir: root.clone(),
        };
        (ws, root)
    }

    fn put_transcript(ws: &Workspace, texts: &[&str]) -> ApiResult {
        let segments: Vec<serde_json::Value> = texts
            .iter()
            .enumerate()
            .map(|(i, t)| {
                serde_json::json!({ "start": i as f64, "end": i as f64 + 1.0, "text": t })
            })
            .collect();
        data_transcript_put_impl(ws, &serde_json::json!({ "segments": segments }))
    }

    /// 选了**外部字幕**（`transcript_file` 指向工程内的 sub.srt）后，在预处理页保存编辑
    /// 必须写回那份文件 —— 只写 output/ 的话下次读到的还是原文件，编辑静默丢失。
    #[test]
    fn transcript_save_writes_back_to_selected_subtitle() {
        let (ws, root) = tmp_workspace("sub-writeback");
        let sub = root.join("sub.srt");
        std::fs::write(&sub, "1\n00:00:01,000 --> 00:00:02,000\n原始\n\n").unwrap();
        vtrim_core::config::patch_config_file(
            &ws.config_file,
            &serde_json::json!({ "project": { "transcript_file": "sub.srt" } }),
        )
        .unwrap();

        let _ = put_transcript(&ws, &["改过的"]).unwrap();
        let saved = std::fs::read_to_string(&sub).unwrap();
        assert!(saved.contains("改过的"), "外部字幕必须就地更新: {saved}");
        // 没有 output/ 产物时不应凭空多造一份
        assert!(!root.join("output/transcript.json").exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    /// ASR 产物的工程：两份 output/ 文件都要同步写（老行为不能丢）。
    #[test]
    fn transcript_save_keeps_output_pair_in_sync() {
        let (ws, root) = tmp_workspace("output-pair");
        std::fs::write(root.join("output/transcript.json"), "{\"segments\":[]}").unwrap();
        std::fs::write(root.join("output/video.srt"), "1\n00:00:01,000 --> 00:00:02,000\n旧\n\n").unwrap();
        vtrim_core::config::patch_config_file(
            &ws.config_file,
            &serde_json::json!({ "project": { "transcript_file": "output/video.srt" } }),
        )
        .unwrap();

        let _ = put_transcript(&ws, &["新的"]).unwrap();
        assert!(std::fs::read_to_string(root.join("output/video.srt")).unwrap().contains("新的"));
        assert!(std::fs::read_to_string(root.join("output/transcript.json")).unwrap().contains("新的"));
        let _ = std::fs::remove_dir_all(&root);
    }

    /// 一份转写都没有、也没选字幕 → 仍然 404（不给空工程凭空造字幕）。
    #[test]
    fn transcript_save_without_any_transcript_is_404() {
        let (ws, root) = tmp_workspace("no-transcript");
        let e = put_transcript(&ws, &["x"]).unwrap_err();
        assert_eq!(e.0, StatusCode::NOT_FOUND);
        let _ = std::fs::remove_dir_all(&root);
    }

    /// `GET /api/project-data/assets` 的返回必须带来源与两侧候选（前端据此决定文案与预载语义）。
    #[test]
    fn assets_impl_reports_source_and_candidates() {
        let (ws, root) = tmp_workspace("assets-impl");
        std::fs::write(root.join("video.mp4"), b"x").unwrap();
        std::fs::write(root.join("video.xml"), b"<i/>").unwrap();
        vtrim_core::config::patch_config_file(
            &ws.config_file,
            &serde_json::json!({ "project": { "input_file": "video.mp4" } }),
        )
        .unwrap();
        let v = assets_impl(&ws, "recorder", "录播姬 · 房间 1", Some("1")).unwrap();
        assert_eq!(v["source"]["kind"], "recorder");
        assert_eq!(v["danmaku"]["value"], "");
        assert_eq!(v["danmaku"]["exists"], false);
        assert_eq!(v["candidates"]["danmaku"][0]["name"], "video.xml");
        assert_eq!(v["candidates"]["danmaku"][0]["same_stem"], true);
        let _ = std::fs::remove_dir_all(&root);
    }

    /// 选素材：写前解析（坏文件不落盘），并在返回里带上条数与状态。
    #[test]
    fn assets_put_impl_validates_and_reports_count() {
        let (ws, root) = tmp_workspace("assets-put");
        std::fs::write(
            root.join("dm.xml"),
            "<?xml version=\"1.0\"?><i><d p=\"1.5,1,25,16777215,0\">hi</d></i>",
        )
        .unwrap();
        let v = assets_put_impl(&ws, &serde_json::json!({ "kind": "danmaku", "path": "dm.xml" }), false)
            .unwrap();
        assert_eq!(v["count"], 1);
        assert_eq!(v["value"], "dm.xml");
        assert_eq!(v["status"]["exists"], true);

        // 坏文件（不是弹幕）→ 400 且不落盘
        std::fs::write(root.join("bad.xml"), "not danmaku at all").unwrap();
        let e = assets_put_impl(&ws, &serde_json::json!({ "kind": "danmaku", "path": "bad.xml" }), false)
            .unwrap_err();
        assert_eq!(e.0, StatusCode::BAD_REQUEST);
        let cfg = ws.load_config().unwrap();
        assert_eq!(cfg.project.danmaku_file.as_deref(), Some("dm.xml"));

        // 未知类别
        let e = assets_put_impl(&ws, &serde_json::json!({ "kind": "nope", "path": "dm.xml" }), false)
            .unwrap_err();
        assert_eq!(e.0, StatusCode::BAD_REQUEST);
        let _ = std::fs::remove_dir_all(&root);
    }

    /// 服务器目录浏览的提交形态：目录与文件名**分开发**，由服务端拼路径。
    /// 前端拼不出来是对的 —— 服务端回传的目录可能带 `\\?\` verbatim 前缀。
    #[test]
    fn assets_put_impl_accepts_dir_plus_name() {
        let (ws, root) = tmp_workspace("assets-put-dirname");
        let sub = root.join("sub");
        std::fs::create_dir_all(&sub).unwrap();
        std::fs::write(sub.join("dm.xml"), "<?xml version=\"1.0\"?><i><d p=\"1.5,1,25,16777215,0\">hi</d></i>").unwrap();
        let v = assets_put_impl(
            &ws,
            &serde_json::json!({ "kind": "danmaku", "dir": root.to_string_lossy(), "name": "sub/dm.xml" }),
            true,
        )
        .unwrap();
        assert_eq!(v["count"], 1);
        // 落在工程根内 → 存相对路径（可移植）
        assert_eq!(v["value"], "sub/dm.xml");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// 换字幕字体只改 `[style.subtitle] font` 一个字段：别的段（分段/屏蔽/封面）原样保留
    /// —— 它是"保存即生效"的同步接口，写坏了会把整个片段的精修配置一起带走。
    #[test]
    fn polish_project_put_writes_subtitle_font_only() {
        let dir = std::env::temp_dir().join(format!("vtrim-font-put-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut cfg = vtrim_polish::project::ProjectFile::example();
        cfg.mute_ranges.push(vtrim_polish::project::MuteRange { start: 1.0, end: 2.0 });
        std::fs::write(dir.join("polish.toml"), cfg.to_toml_commented().unwrap()).unwrap();

        let body = serde_json::json!({ "subtitle_font": "simhei" });
        let _ = polish_project_put_locked(&dir, &body).expect("保存字幕字体");

        let after = vtrim_polish::project::load_project(&dir).unwrap();
        assert_eq!(after.style.subtitle.font, "simhei");
        assert_eq!(after.mute_ranges.len(), 1, "没提到的字段必须原样保留");

        // 空串 = 回到系统字体栈
        let _ = polish_project_put_locked(&dir, &serde_json::json!({ "subtitle_font": "" })).unwrap();
        assert_eq!(vtrim_polish::project::load_project(&dir).unwrap().style.subtitle.font, "");
        let _ = std::fs::remove_dir_all(&dir);
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
        let jobs = Arc::new(crate::server::jobs::JobManager::new());
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

    #[test]
    fn sec_to_ts_never_produces_invalid_ms() {
        // 手写 floor/round 版本会给出 ms = 1000
        let v = sec_to_ts_value(59.9999);
        assert_eq!(v["m"].as_u64(), Some(1));
        assert_eq!(v["s"].as_u64(), Some(0));
        assert_eq!(v["ms"].as_u64(), Some(0));

        let v = sec_to_ts_value(12.5);
        assert_eq!(v["s"].as_u64(), Some(12));
        assert_eq!(v["ms"].as_u64(), Some(500));
        assert!(sec_to_ts_value(-3.0)["h"].as_u64() == Some(0));
    }

    /// 渲染进度行解析：4 个分块各报各的，父进程按最小值聚合。
    ///
    /// 行格式直接取自真实渲染日志（`frames A..B: NN%  Ns`）。
    #[test]
    fn render_progress_lines_are_parsed() {
        let (tag, pct) = parse_chunk_progress("    frames 0..1969: 45%  120s").unwrap();
        assert_eq!(tag, "0..1969");
        assert_eq!(pct, 45);
        let (tag2, _) = parse_chunk_progress("    frames 1969..3938: 3%  4s").unwrap();
        assert_ne!(tag, tag2, "不同分块必须是不同的键，否则进度会互相覆盖");

        // 分块收尾行 = 100%
        let (tag3, pct3) = parse_chunk_progress("    chunk#0: done  1969 frames（抽帧 23.2s / 渲染 145.0s / 编码 8.5s）").unwrap();
        assert!(tag3.starts_with('#'));
        assert_eq!(pct3, 100);

        // 普通行不能被当成进度
        assert!(parse_chunk_progress("  -- Render chunks (4) --").is_none());
        assert!(parse_chunk_progress("  v 9 sound events  ⏱ 3.4s").is_none());
        assert!(parse_chunk_progress("    chunk#0: extracting 1969 frames via ffmpeg...").is_none());
    }

    /// 子进程日志的增量读取：半行要留在缓冲区，不能读一半就当一条日志发出去。
    #[test]
    fn render_output_drains_complete_lines_only() {
        let path = std::env::temp_dir().join(format!("vtrim-render-out-{}.log", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let mut out = RenderOutput::new(path.clone());

        std::fs::write(&path, b"first\nsecond-part").unwrap();
        assert_eq!(out.drain(), vec!["first".to_string()]);

        // 补上后半行 + 新的一行
        std::fs::write(&path, b"first\nsecond-part\nthird\n").unwrap();
        assert_eq!(
            out.drain(),
            vec!["second-part".to_string(), "third".to_string()]
        );
        assert!(out.drain().is_empty(), "没有新内容时不该重复吐行");

        // 失败文案取尾部若干行
        out.push_tail("a");
        out.push_tail("b");
        assert_eq!(out.tail_text(), "a\nb");
        let _ = std::fs::remove_file(&path);
    }

    /// 流式跟随：里程碑行进任务日志（info，会进运行日志面板），进度行进 progress 事件。
    ///
    /// 这是"渲染期间日志一行没有"的直接修复点 —— 断言它在**子进程还在跑**的时候就有输出。
    #[tokio::test]
    async fn follow_render_output_emits_logs_and_progress_before_exit() {
        let jobs = Arc::new(super::super::jobs::JobManager::new());
        let handle = jobs
            .create(1, JobSpec::new("polish_render", "polish render", vec![], JobClass::Heavy))
            .await;
        let id = handle.id.clone();
        handle.wait_start().await.expect("资源不相干，应该直接开跑");

        let path = std::env::temp_dir().join(format!("vtrim-follow-{}.log", std::process::id()));
        let _ = std::fs::remove_file(&path);
        std::fs::write(
            &path,
            format!(
                "{}  INFO vtrim_polish::render::pipeline:   -- Render chunks (4) --\n\
                 {}  INFO vtrim_polish::render::chrome:     frames 0..1969: 45%  120s\n\
                 {}  INFO headless_chrome::browser: Starting Chrome\n",
                now_stamp(),
                now_stamp(),
                now_stamp()
            ),
        )
        .unwrap();

        let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let follow = tokio::spawn(follow_render_output(
            jobs.clone(),
            id.clone(),
            RenderOutput::new(path.clone()),
            stop.clone(),
        ));
        // 让它至少跑一轮
        tokio::time::sleep(Duration::from_millis(700)).await;
        stop.store(true, std::sync::atomic::Ordering::SeqCst);
        let out = follow.await.unwrap();

        let summary = jobs.get(&id).await.expect("任务存在");
        assert_eq!(summary.current, 45, "分块进度要进 progress");
        assert_eq!(summary.total, 100);
        let logs: Vec<String> = summary
            .logs
            .iter()
            .map(|l| l.message.clone())
            .collect();
        assert!(
            logs.iter().any(|l| l.contains("-- Render chunks")),
            "里程碑行要进日志: {logs:?}"
        );
        assert!(
            logs.iter().any(|l| l.contains("Starting Chrome")),
            "其余行进 debug 日志: {logs:?}"
        );
        assert!(
            logs.iter().all(|l| !l.contains("45%")),
            "进度行不该重复进日志: {logs:?}"
        );
        assert!(out.tail_text().contains("Starting Chrome"), "尾部要有最后一行");
        let _ = std::fs::remove_file(&path);
    }

    /// 日志行前缀（tracing 默认格式）里的时间戳，测试用。
    fn now_stamp() -> String {
        "2026-09-15T07:24:55.118063Z".to_string()
    }

    #[test]
    fn project_dir_validation_rejects_empty_and_missing() {
        assert!(validated_project_dir("").is_err());
        assert!(validated_project_dir("   ").is_err());
        let missing = std::env::temp_dir().join("vtrim-dir-does-not-exist-xyz");
        assert!(validated_project_dir(&missing).is_err());
        assert!(validated_project_dir(std::env::temp_dir()).is_ok());
        assert!(validated_project_dir(std::env::temp_dir().to_string_lossy().to_string()).is_ok());
    }

    /// 移除项目 = **只注销绑定**：目录里一个文件都不动，而且能重新导入同一个目录。
    ///
    /// 回归：这条路以前会 `remove_file(vtrim.toml)`（`projects_delete` 与过渡别名各删一次）。
    /// 留着 `vtrim.toml` 也正是重新绑定的前提 —— `create_local_project` 见到已有
    /// `vtrim.toml` 只登记、不覆盖，所以手改过的工程设置不会丢。
    ///
    /// ⚠️ 本用例会经 `forget_project` 写一份 `server.toml`，路径是
    /// `<测试可执行文件旁>/.vtrim/`（即 `target/debug/deps/.vtrim/`）——
    /// 与开发实例的 `target/debug/.vtrim/` 不在一处，不会干扰本机跑着的服务。
    #[tokio::test]
    async fn unregister_keeps_every_file_and_can_rebind() {
        let t = test_app().await;
        let cfg = t.root.join("vtrim.toml");
        let cfg_before = std::fs::read_to_string(&cfg).unwrap();
        // 造一个"产物"：移除后它必须还在
        let clip = t.root.join("output").join("clips").join("clip-01").join("clip.mp4");
        std::fs::create_dir_all(clip.parent().unwrap()).unwrap();
        std::fs::write(&clip, b"clip-bytes").unwrap();

        // 先取一次基线：列表里确实有它（否则"注销后不在列表里"这句会因为接口返回空而白通过）
        let before = project_ids(&t).await;
        assert!(before.contains(&1), "注销前应该能看到项目 1: {before:?}");

        let res = t
            .call(t.req("DELETE", "/projects/1").body(Body::empty()).unwrap())
            .await;
        assert_eq!(res.status(), StatusCode::OK);

        // ① 零文件改动：配置 / 源媒体 / 产物全部原样
        assert!(cfg.exists(), "vtrim.toml 必须留着（移除 ≠ 删目录）");
        assert_eq!(std::fs::read_to_string(&cfg).unwrap(), cfg_before, "内容也不该变");
        assert!(t.root.join("a.mp4").exists(), "源媒体必须留着");
        assert!(clip.exists(), "产物必须留着");

        // ② 服务器列表里已经没有它了
        let after = project_ids(&t).await;
        assert!(!after.contains(&1), "注销后不该再出现在列表里: {after:?}");

        // ③ 重新导入同一个目录 → 重新绑定成功，且**不覆盖**已有配置
        let body = serde_json::json!({
            "source": { "kind": "local", "dir": vtrim_core::path::display(&t.root) }
        });
        let res = t
            .call(
                t.req("POST", "/projects")
                    .header(axum::http::header::CONTENT_TYPE, "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await;
        assert_eq!(res.status(), StatusCode::OK, "同一个目录要能重新导入");
        let again = body_json(res).await;
        assert!(again.get("id").and_then(|v| v.as_u64()).is_some(), "{again}");
        assert_eq!(
            std::fs::read_to_string(&cfg).unwrap(),
            cfg_before,
            "重新导入不能改已有配置（只登记、不覆盖）"
        );
    }

    /// 带一套独立账号账本 + 已登录会话的测试服务。    ///
    /// 账号库放在临时目录里（不经 `VTRIM_DATA_DIR` 环境变量：测试是并行跑的，
    /// 改全局环境变量会互相踩）。登录本身也走真实 KDF 与真实会话表。
    struct TestApp {
        app: Router,
        auth: Arc<crate::server::auth::AuthService>,
        /// 同一个 AppState（任务管理器在它里面）：用例可以直接造"占着资源"的任务来验互斥
        state: Arc<AppState>,
        _dir: tempfile::TempDir,
        /// 测试项目根目录（`output/clips` 就在它下面），给需要造/验文件的用例用
        root: PathBuf,
        sid: String,
        csrf: String,
    }

    async fn test_app_with_must_change(must_change: bool) -> TestApp {
        use crate::server::auth::{AuthService, Listening};
        let dir = tempfile::tempdir().unwrap();
        let auth = AuthService::open(dir.path().to_path_buf(), Listening::Loopback).unwrap();
        let hash = vtrim_service::auth::hash_password("initial-pass-1234").unwrap();
        let uid = auth
            .db()
            .create_user("admin", &hash, "admin", must_change, 0)
            .unwrap();
        // 真项目目录 + 真 vtrim.toml：P2 之后所有接口都要经过项目作用域
        let root = dir.path().join("proj");
        std::fs::create_dir_all(root.join("output/clips")).unwrap();
        std::fs::write(root.join("a.mp4"), b"fake").unwrap();
        let cfg_path = root.join("vtrim.toml");
        std::fs::write(&cfg_path, "input_file = \"a.mp4\"\n").unwrap();
        let project_id = auth
            .db()
            .create_project(uid, "proj", None, "local", Some("测试"), &root, &cfg_path, None)
            .unwrap();
        assert_eq!(project_id, 1, "测试项目固定为 id=1");
        let bundle = auth
            .login("admin", "initial-pass-1234", None, None)
            .expect("测试账号应能登录");
        let ws = Workspace::load(Some(&cfg_path)).expect("workspace");
        let state = Arc::new(AppState::with_auth(ws, root.clone(), auth.clone()));
        TestApp {
            app: router(state.clone()),
            auth,
            state,
            _dir: dir,
            root,
            sid: bundle.session_id,
            csrf: bundle.csrf,
        }
    }

    /// 字幕保存要**按开始时间排序**后再落盘（去静音/预览都依赖字幕有序）。
    ///
    /// 回归：编辑器「拆分」曾把后半段 push 到数组末尾，存出来的字幕文件是乱序的 ——
    /// 去静音按文件顺序取相邻间隔，于是判出 13 段/121s 的假静音（实测 clip-19，
    /// 用户"去静音完全对不上，像用了别的素材"）。乱序的旧数据**再存一次就自愈**。
    #[tokio::test]
    async fn subtitle_save_normalizes_time_order() {
        let t = test_app().await;
        let dir = t.root.join("output").join("clips").join("clip-19");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("polish.toml"),
            "[source]\nvideo = \"clip.mp4\"\nsubtitles = \"clip.srt\"\n",
        )
        .unwrap();

        // 乱序：末尾那条其实是开头那段的「拆分后半段」（就是出问题的形状）
        let body = serde_json::json!({
            "dir": vtrim_core::path::display(&dir),
            "segments": [
                { "start": 0.0,  "end": 2.07,  "text": "前半" },
                { "start": 99.7, "end": 102.4, "text": "后面一句" },
                { "start": 2.07, "end": 5.79,  "text": "拆分出来的后半" },
            ],
        });
        let res = t
            .call(
                t.req("PUT", "/polish/subtitles")
                    .header(axum::http::header::CONTENT_TYPE, "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await;
        assert_eq!(res.status(), StatusCode::OK);

        let saved: Vec<f64> = {
            let raw = std::fs::read_to_string(dir.join("data").join("subtitles.json")).unwrap();
            serde_json::from_str::<serde_json::Value>(&raw)
                .unwrap()
                .as_array()
                .unwrap()
                .iter()
                .map(|s| s["start"].as_f64().unwrap())
                .collect()
        };
        assert_eq!(saved, vec![0.0, 2.07, 99.7], "落盘顺序要按开始时间");

        // clip.srt 同步归一化（外部工具读它）
        let srt = std::fs::read_to_string(dir.join("clip.srt")).unwrap();
        let order: Vec<&str> = srt
            .lines()
            .filter(|l| l.contains("-->"))
            .map(|l| l.split_whitespace().next().unwrap())
            .collect();
        assert_eq!(order, vec!["00:00:00,000", "00:00:02,070", "00:01:39,700"], "{srt}");
    }

    async fn test_app() -> TestApp {
        test_app_with_must_change(false).await
    }

    /// 贴纸包上传这条路的**路由层**：只有 admin 能调、body 上限确实被放开、垃圾字节是 400。
    ///
    /// 三件事缺任何一条，前端看到都是"接口不存在或返回格式错误"：
    /// ① 非 admin 必须 403（贴纸库是全局资产）；
    /// ② `DefaultBodyLimit` 必须挂在**这一条**路由上 —— axum 默认只收 2 MB，
    ///    用一个 3 MiB 的体试探：被 413 掉就说明那层丢了（改回默认值/漏挂 layer 都会挂在这条）；
    /// ③ 内容不是 zip 时给 400 与原因（不是 500，也不能当成成功）。
    /// 正常解压的路径在 `vtrim-polish::assets::sticker_zip` 的 13 条单测里（它们用临时目录，
    /// 不碰真实的贴纸库）；这里刻意只喂垃圾字节，免得往 `<exe_dir>/stickers/` 写真文件。
    #[tokio::test]
    async fn sticker_upload_route_is_admin_only_and_raises_body_limit() {
        let t = test_app().await;

        // ① 普通用户：403
        let hash = vtrim_service::auth::hash_password("pass-12345678").unwrap();
        t.auth.db().create_user("plain_upload", &hash, "user", false, 0).unwrap();
        let b = t.auth.login("plain_upload", "pass-12345678", Some("10.9.9.9"), None).unwrap();
        let res = t
            .call(
                t.req_as(&b.session_id, &b.csrf, "POST", "/admin/stickers/upload?name=zz_probe")
                    .body(Body::from(b"not a zip".to_vec()))
                    .unwrap(),
            )
            .await;
        assert_eq!(res.status(), StatusCode::FORBIDDEN, "非 admin 不能让上传落进全局贴纸库");

        // ③ 垃圾字节 → 400 且带原因
        let res = t
            .call(
                t.req("POST", "/admin/stickers/upload?name=zz_probe")
                    .body(Body::from(b"definitely not a zip".to_vec()))
                    .unwrap(),
            )
            .await;
        assert_eq!(res.status(), StatusCode::BAD_REQUEST);
        let v = body_json(res).await;
        assert!(
            v["error"].as_str().unwrap_or("").contains("不是有效的 zip"),
            "{v}"
        );

        // ② 3 MiB（> axum 默认 2 MB）：必须走到 handler（400），不能是 413
        let big = vec![0u8; 3 * 1024 * 1024];
        let res = t
            .call(
                t.req("POST", "/admin/stickers/upload?name=zz_probe")
                    .body(Body::from(big))
                    .unwrap(),
            )
            .await;
        assert_eq!(
            res.status(),
            StatusCode::BAD_REQUEST,
            "上传路由必须放开 axum 的 2 MB 默认上限（413 = DefaultBodyLimit 那层丢了）"
        );

        // 包名不合法 / 缺名字：在解压之前就挡住
        for uri in ["/admin/stickers/upload?name=..%2Fx", "/admin/stickers/upload"] {
            let res = t
                .call(t.req("POST", uri).body(Body::from(vec![1u8, 2, 3])).unwrap())
                .await;
            assert_eq!(res.status(), StatusCode::BAD_REQUEST, "{uri}");
        }
        // 真实贴纸库里不该留下任何东西（三条失败路径都只能在暂存目录里动手，且必须清掉）
        let root = vtrim_core::path::sticker_dir();
        if root.is_dir() {
            let leftovers: Vec<String> = std::fs::read_dir(&root)
                .unwrap()
                .flatten()
                .map(|e| e.file_name().to_string_lossy().to_string())
                .filter(|n| n.starts_with("zz_probe") || n.starts_with(".upload-"))
                .collect();
            assert!(leftovers.is_empty(), "上传失败不该留残留: {leftovers:?}");
        }
    }

    impl TestApp {
        fn cookie(&self) -> String {
            format!(
                "{}={}; {}={}",
                vtrim_service::auth::COOKIE_SID,
                self.sid,
                vtrim_service::auth::COOKIE_CSRF,
                self.csrf
            )
        }

        /// 构造已带会话 cookie 的请求；写请求自动带 CSRF 头。
        fn req(&self, method: &str, uri: &str) -> axum::http::request::Builder {
            let b = HttpRequest::builder()
                .method(method)
                .uri(uri)
                .header(axum::http::header::COOKIE, self.cookie());
            if matches!(method, "GET" | "HEAD" | "OPTIONS") {
                b
            } else {
                b.header(vtrim_service::auth::CSRF_HEADER, self.csrf.clone())
            }
        }

        async fn call(&self, req: axum::http::Request<Body>) -> Response {
            self.app.clone().oneshot(req).await.unwrap()
        }

        /// 用**另一个账号**的会话构造请求（跨租户回归用 —— `req()` 固定用测试自己的 admin 会话）。
        fn req_as(
            &self,
            sid: &str,
            csrf: &str,
            method: &str,
            uri: &str,
        ) -> axum::http::request::Builder {
            let b = HttpRequest::builder()
                .method(method)
                .uri(uri)
                .header(
                    axum::http::header::COOKIE,
                    format!(
                        "{}={}; {}={}",
                        vtrim_service::auth::COOKIE_SID,
                        sid,
                        vtrim_service::auth::COOKIE_CSRF,
                        csrf
                    ),
                );
            if matches!(method, "GET" | "HEAD" | "OPTIONS") {
                b
            } else {
                b.header(vtrim_service::auth::CSRF_HEADER, csrf.to_string())
            }
        }
    }

    async fn body_json(res: Response) -> serde_json::Value {
        let bytes = axum::body::to_bytes(res.into_body(), 256 * 1024).await.unwrap();
        serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null)
    }

    /// `GET /api/projects` 的项目 id 列表。
    ///
    /// 断言"某项目在/不在列表里"时**一定要先确认响应是好的**：接口报错会返回 `{error:…}`，
    /// 拿不到 `projects` 数组，直接断言"不在里面"就会白通过。
    async fn project_ids(t: &TestApp) -> Vec<u64> {
        let res = t.call(t.req("GET", "/projects").body(Body::empty()).unwrap()).await;
        assert_eq!(res.status(), StatusCode::OK, "读取项目列表失败");
        let json = body_json(res).await;
        json.get("projects")
            .and_then(|v| v.as_array())
            .unwrap_or_else(|| panic!("响应里没有 projects 数组: {json}"))
            .iter()
            .filter_map(|p| p.get("id").and_then(|i| i.as_u64()))
            .collect()
    }

    /// 未注册的 /api/* 必须返回 JSON 404：外层 fallback 是 SPA HTML，
    /// 若让它兜住 API 请求，前端会把 200 HTML 解析成 null 并静默失败。
    #[tokio::test]
    async fn unknown_api_path_returns_json_404() {
        let t = test_app().await;
        let res = t
            .call(HttpRequest::builder().uri("/__nope").body(Body::empty()).unwrap())
            .await;
        assert_eq!(res.status(), StatusCode::NOT_FOUND);
        let ct = res
            .headers()
            .get(axum::http::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        assert!(ct.contains("application/json"), "content-type = {ct}");
        let bytes = axum::body::to_bytes(res.into_body(), 64 * 1024).await.unwrap();
        let text = String::from_utf8_lossy(&bytes);
        assert!(text.contains("未知接口"), "body = {text}");
    }

    /// 前端实际调用的核心路由必须可达（避免再次出现"handler 写了但没注册"）。
    #[tokio::test]
    async fn core_routes_are_reachable() {
        let t = test_app().await;
        for (method, uri) in [("GET", "/health"), ("GET", "/jobs"), ("GET", "/config"), ("GET", "/models")] {
            let res = t.call(t.req(method, uri).body(Body::empty()).unwrap()).await;
            assert_eq!(res.status(), StatusCode::OK, "{method} {uri} 应为 200");
        }
    }

    /// 除白名单外，**未登录一律 401**（含 loopback：没有免登录分支，§3.5.1）。
    #[tokio::test]
    async fn unauthenticated_requests_are_401_except_whitelist() {
        let t = test_app().await;
        // 白名单：/health 必须公开（前端靠它判断服务是否活着）
        let res = t
            .call(HttpRequest::builder().uri("/health").body(Body::empty()).unwrap())
            .await;
        assert_eq!(res.status(), StatusCode::OK);
        let health = body_json(res).await;
        assert_eq!(health["listening"], "loopback");
        assert!(health.get("workspace").is_none(), "health 不得暴露任何路径: {health}");
        // 未认证接口不得泄露账号信息：账号数、是否仍用初始密码都只能用登录态或本地 doctor 看
        assert!(health.get("accounts").is_none(), "health 不得暴露账号数量: {health}");
        assert!(
            health.get("must_change_password").is_none(),
            "health 不得暴露「是否仍用初始密码」（等于给攻击者指路）: {health}"
        );

        for (method, uri) in [
            ("GET", "/me"),
            ("GET", "/jobs"),
            ("GET", "/config"),
            ("GET", "/models"),
            ("GET", "/project/list"),
            ("GET", "/project-data/status?config=vtrim.toml"),
            ("POST", "/polish/init"),
        ] {
            let res = t
                .call(
                    HttpRequest::builder()
                        .method(method)
                        .uri(uri)
                        .header(axum::http::header::CONTENT_TYPE, "application/json")
                        .body(Body::from("{}"))
                        .unwrap(),
                )
                .await;
            assert_eq!(res.status(), StatusCode::UNAUTHORIZED, "{method} {uri} 未登录必须 401");
        }

        // 未知路径仍走 JSON 404（不因为没登录就变成 401/HTML）
        let res = t
            .call(HttpRequest::builder().uri("/does-not-exist").body(Body::empty()).unwrap())
            .await;
        assert_eq!(res.status(), StatusCode::NOT_FOUND);
    }

    /// 写请求缺 CSRF / CSRF 不匹配一律 403（cookie 会话 + 自定义头双提交）；
    /// 带上正确 CSRF 后**还要**通过项目作用域（未登记的路径一律 404）。
    #[tokio::test]
    async fn write_requests_require_matching_csrf() {
        let t = test_app().await;
        let body = Body::from(r#"{"raw":""}"#);
        let res = t
            .call(
                HttpRequest::builder()
                    .method("PUT")
                    .uri("/project-data/config?project=1")
                    .header(axum::http::header::COOKIE, t.cookie())
                    .header(axum::http::header::CONTENT_TYPE, "application/json")
                    .body(body)
                    .unwrap(),
            )
            .await;
        assert_eq!(res.status(), StatusCode::FORBIDDEN, "缺 CSRF 头必须 403");

        let res = t
            .call(
                HttpRequest::builder()
                    .method("PUT")
                    .uri("/project-data/config?project=1")
                    .header(axum::http::header::COOKIE, t.cookie())
                    .header(vtrim_service::auth::CSRF_HEADER, "not-the-token")
                    .header(axum::http::header::CONTENT_TYPE, "application/json")
                    .body(Body::from(r#"{"raw":""}"#))
                    .unwrap(),
            )
            .await;
        assert_eq!(res.status(), StatusCode::FORBIDDEN, "CSRF 不匹配必须 403");

        // 带上正确 CSRF 后才会走到业务校验（空 raw → 400）
        let res = t
            .call(
                t.req("PUT", "/project-data/config?project=1")
                    .header(axum::http::header::CONTENT_TYPE, "application/json")
                    .body(Body::from(r#"{"raw":""}"#))
                    .unwrap(),
            )
            .await;
        assert_eq!(res.status(), StatusCode::BAD_REQUEST, "空内容必须被拒绝");
    }

    /// 路径面归零：未登记的路径（含任意绝对路径）一律 404，只有本人项目 id 可用。
    #[tokio::test]
    async fn path_face_is_closed_for_project_data() {
        let t = test_app().await;
        // 未登记的 config 路径
        let res = t
            .call(t.req("GET", "/project-data/status?config=C%3A%5CWindows%5Cvtrim.toml").body(Body::empty()).unwrap())
            .await;
        assert_eq!(res.status(), StatusCode::NOT_FOUND, "未登记路径必须 404");
        // 他人的项目 id
        let res = t
            .call(t.req("GET", "/project-data/status?project=999").body(Body::empty()).unwrap())
            .await;
        assert_eq!(res.status(), StatusCode::NOT_FOUND, "不存在/他人的项目 id 必须 404");
        // 本项目 id → 200
        let res = t
            .call(t.req("GET", "/project-data/status?project=1").body(Body::empty()).unwrap())
            .await;
        assert_eq!(res.status(), StatusCode::OK, "本人项目应可用: {:?}", res.status());
    }

    /// 三个"路径类 body 字段"都不能把项目外的文件拉进来（普通用户）。
    ///
    /// 回归钉子：`cover-background` 的 `source` 与 `cover-config` 的 `cover.image` 此前完全
    /// 没做归属校验（`scope.rs` 只归一化 `dir` / `output_file`，而模块注释却声称覆盖了
    /// `source`）。后果不是"多读一个文件"：`source` 指向 `<数据目录>/.vtrim/llm.toml`
    /// 就能把**全局 LLM API key** 复制进自己的项目，再用 `/api/media/<自己的项目>/…` 读回来；
    /// 同样能拿到账号账本 `cloud/vtrim.db`（明文邀请码 + 全部项目真实路径）。
    #[tokio::test]
    async fn polish_path_fields_cannot_read_outside_files() {
        let t = test_app().await;

        // 受害者（admin 的项目）里的一个"机密"文件
        let secret = t.root.join("output").join("secret.txt");
        std::fs::write(&secret, b"api_key = 'sk-real-secret'").unwrap();

        // 普通用户 + 他自己的项目目录（含 clip-01 与 polish.toml，够走完前置校验）
        let hash = vtrim_service::auth::hash_password("pass-12345678").unwrap();
        let uid = t.auth.db().create_user("plain_paths", &hash, "user", false, 0).unwrap();
        let my_root = t.root.join("plain-proj");
        let my_clip = my_root.join("output/clips/clip-01");
        std::fs::create_dir_all(&my_clip).unwrap();
        std::fs::write(
            my_clip.join("polish.toml"),
            "[source]\nvideo = \"clip.mp4\"\nsubtitles = \"clip.srt\"\n",
        )
        .unwrap();
        let my_own_bg = my_root.join("bg.jpg");
        std::fs::write(&my_own_bg, b"not a secret").unwrap();
        let my_cfg = my_root.join("vtrim.toml");
        std::fs::write(&my_cfg, "input_file = \"a.mp4\"\n").unwrap();
        let my_pid = t
            .auth
            .db()
            .create_project(uid, "plain", None, "local", Some("测试"), &my_root, &my_cfg, None)
            .unwrap();
        let b = t.auth.login("plain_paths", "pass-12345678", Some("10.1.1.1"), None).unwrap();
        let (sid, csrf) = (b.session_id.clone(), b.csrf.clone());

        let clip = my_clip.to_string_lossy().to_string();

        // ① cover-background：项目外的 source → 404，且**不许**复制出文件
        let body = serde_json::json!({ "dir": clip, "source": secret.to_string_lossy() }).to_string();
        let res = t
            .call(
                t.req_as(&sid, &csrf, "POST", "/polish/cover-background")
                    .header(axum::http::header::CONTENT_TYPE, "application/json")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await;
        assert_eq!(res.status(), StatusCode::NOT_FOUND, "项目外 source 必须 404");
        assert!(
            !my_clip.join("_cover_bg.jpg").exists(),
            "被拒的请求不得留下复制出来的文件"
        );
        // 正对照：自己项目内的文件照常可用（证明不是"一律拒绝"）
        let body = serde_json::json!({ "dir": clip, "source": my_own_bg.to_string_lossy() }).to_string();
        let res = t
            .call(
                t.req_as(&sid, &csrf, "POST", "/polish/cover-background")
                    .header(axum::http::header::CONTENT_TYPE, "application/json")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await;
        assert_eq!(res.status(), StatusCode::OK, "自己项目内的背景图应可用");

        // ② cover-config：cover.image 指向项目外 → 404
        let body = serde_json::json!({ "dir": clip, "cover": { "image": secret.to_string_lossy() } })
            .to_string();
        let res = t
            .call(
                t.req_as(&sid, &csrf, "POST", "/polish/cover-config")
                    .header(axum::http::header::CONTENT_TYPE, "application/json")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await;
        assert_eq!(res.status(), StatusCode::NOT_FOUND, "项目外 cover.image 必须 404");
        let cfg = vtrim_polish::project::load_project(&my_clip).unwrap();
        assert!(cfg.cover.image.is_empty(), "被拒的请求不得写进 polish.toml");
        // 正对照：自己项目内的图能存进去（证明 404 来自归属校验，不是别的校验在挡）
        let body = serde_json::json!({ "dir": clip, "cover": { "image": my_own_bg.to_string_lossy() } })
            .to_string();
        let res = t
            .call(
                t.req_as(&sid, &csrf, "POST", "/polish/cover-config")
                    .header(axum::http::header::CONTENT_TYPE, "application/json")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await;
        assert_eq!(res.status(), StatusCode::OK, "自己项目内的图应能存进配置");
        assert!(
            !vtrim_polish::project::load_project(&my_clip).unwrap().cover.image.is_empty(),
            "正对照应当真的写进 polish.toml"
        );

        // ③ project-data/config：绝对的 input_file / output_dir → 400
        for patch in [
            serde_json::json!({ "project": { "input_file": secret.to_string_lossy() } }),
            serde_json::json!({ "cutting": { "output_dir": t.root.to_string_lossy() } }),
        ] {
            let body = serde_json::json!({ "patch": patch }).to_string();
            let res = t
                .call(
                    t.req_as(&sid, &csrf, "PUT", &format!("/project-data/config?project={my_pid}"))
                        .header(axum::http::header::CONTENT_TYPE, "application/json")
                        .body(Body::from(body))
                        .unwrap(),
                )
                .await;
            assert_eq!(res.status(), StatusCode::BAD_REQUEST, "绝对路径键必须 400：{patch}");
        }
        let saved = std::fs::read_to_string(&my_cfg).unwrap();
        assert!(!saved.contains("secret.txt"), "被拒的配置不得落盘：{saved}");
        // 正对照：项目内相对路径照常能存
        let body = serde_json::json!({ "patch": { "project": { "input_file": "a.mp4" } } }).to_string();
        let res = t
            .call(
                t.req_as(&sid, &csrf, "PUT", &format!("/project-data/config?project={my_pid}"))
                    .header(axum::http::header::CONTENT_TYPE, "application/json")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await;
        assert_eq!(res.status(), StatusCode::OK, "项目内相对路径应能保存");
    }

    /// 仍在使用初始密码时，除改密/登出/自查外一律 403（服务端强制，不靠前端横幅）。
    #[tokio::test]
    async fn must_change_password_blocks_everything_but_the_password_flow() {
        let t = test_app_with_must_change(true).await;
        // 放行：自查（前端靠它显示横幅并弹改密框）
        assert_eq!(
            t.call(t.req("GET", "/me").body(Body::empty()).unwrap()).await.status(),
            StatusCode::OK
        );
        // 其余接口一律 403
        for (method, uri) in [
            ("GET", "/projects"),
            ("GET", "/jobs"),
            ("GET", "/project-data/status?project=1"),
        ] {
            let res = t.call(t.req(method, uri).body(Body::empty()).unwrap()).await;
            assert_eq!(res.status(), StatusCode::FORBIDDEN, "{method} {uri} 应被改密门拦住");
        }
        // 改密本身必须能用，改完立刻恢复
        let body = serde_json::json!({
            "old_password": "initial-pass-1234",
            "new_password": "pass-12345678"
        })
        .to_string();
        let res = t
            .call(
                t.req("PUT", "/auth/password")
                    .header(axum::http::header::CONTENT_TYPE, "application/json")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await;
        assert_eq!(res.status(), StatusCode::OK, "改密请求本身不应被拦：{:?}", res.status());
        assert_eq!(
            t.call(t.req("GET", "/projects").body(Body::empty()).unwrap()).await.status(),
            StatusCode::OK,
            "改密后应恢复正常"
        );
    }

    /// polish 的 `dir` 只能落在本人项目内：项目外绝对路径 → 404。
    #[tokio::test]
    async fn polish_dir_must_be_inside_own_project() {
        let t = test_app().await;
        let outside = std::env::temp_dir().join("vtrim-not-my-project");
        std::fs::create_dir_all(&outside).unwrap();
        let body = serde_json::json!({ "dir": outside.to_string_lossy() }).to_string();
        let res = t
            .call(
                t.req("POST", "/polish/init")
                    .header(axum::http::header::CONTENT_TYPE, "application/json")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await;
        assert_eq!(res.status(), StatusCode::NOT_FOUND, "项目外目录必须 404");
        let _ = std::fs::remove_dir_all(&outside);
    }

    /// 封面背景设置（bg_mode / bg_color / blur_scale）必须真的落进 polish.toml：
    /// 设计器保存走的是**白名单 patch**，漏一个键就是"点了没反应"。
    /// 颜色还要在服务端净化（它直接进 HTML 模板）。
    #[tokio::test]
    async fn cover_config_persists_background_mode_and_sanitizes_color() {
        let t = test_app().await;
        let clip = t.root.join("output/clips/clip-01");
        std::fs::create_dir_all(&clip).unwrap();
        std::fs::write(clip.join("polish.toml"), "[source]\nvideo = \"clip.mp4\"\nsubtitles = \"clip.srt\"\n").unwrap();
        let dir = clip.to_string_lossy().to_string();

        let body = serde_json::json!({
            "dir": dir,
            "cover": { "bg_mode": "blur", "bg_color": "#AABBCC", "blur_scale": 2.5 }
        })
        .to_string();
        let res = t
            .call(
                t.req("POST", "/polish/cover-config")
                    .header(axum::http::header::CONTENT_TYPE, "application/json")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await;
        assert_eq!(res.status(), StatusCode::OK);

        let cfg = vtrim_polish::project::load_project(&clip).unwrap();
        assert_eq!(cfg.cover.bg_mode, "blur");
        assert_eq!(cfg.cover.bg_color, "#aabbcc", "颜色应被规范化成小写 #rrggbb");
        assert!((cfg.cover.blur_scale - 2.5).abs() < 1e-9);

        // 非法颜色退回默认；倍率夹到上限
        let body = serde_json::json!({
            "dir": dir,
            "cover": { "bg_mode": "color", "bg_color": "red;background:url(x)", "blur_scale": 99.0 }
        })
        .to_string();
        let res = t
            .call(
                t.req("POST", "/polish/cover-config")
                    .header(axum::http::header::CONTENT_TYPE, "application/json")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await;
        assert_eq!(res.status(), StatusCode::OK);
        let cfg = vtrim_polish::project::load_project(&clip).unwrap();
        assert_eq!(cfg.cover.bg_color, "#000000", "非法颜色必须被净化成默认黑");
        assert!((cfg.cover.blur_scale - 4.0).abs() < 1e-9, "倍率要夹到上限");
        // 保存后仍能在配置文件里看到这些键（注释模板会带上说明）
        let raw = std::fs::read_to_string(clip.join("polish.toml")).unwrap();
        assert!(raw.contains("bg_mode") && raw.contains("blur_scale"), "polish.toml 缺键：{raw}");
    }

    /// 登录 → `/api/me` → 改密 → `must_change_password` 变 false（§3.5.10 第 1–5 条）。
    #[tokio::test]
    async fn login_me_change_password_flow() {
        use crate::server::auth::{AuthService, Listening};
        let dir = tempfile::tempdir().unwrap();
        let auth = AuthService::open(dir.path().to_path_buf(), Listening::Loopback).unwrap();
        let hash = vtrim_service::auth::hash_password("initial-pass-1234").unwrap();
        auth.db().create_user("admin", &hash, "admin", true, 0).unwrap();
        let ws = Workspace::load(Some(Path::new("vtrim.toml"))).expect("workspace");
        let app = router(Arc::new(AppState::with_auth(ws, std::env::temp_dir(), auth.clone())));

        // 初始密码：错的 401、对的 200 + Set-Cookie
        let bad = app
            .clone()
            .oneshot(
                HttpRequest::builder()
                    .method("POST")
                    .uri("/auth/login")
                    .header(axum::http::header::CONTENT_TYPE, "application/json")
                    .body(Body::from(r#"{"username":"admin","password":"wrong-pass"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(bad.status(), StatusCode::UNAUTHORIZED);

        let good = app
            .clone()
            .oneshot(
                HttpRequest::builder()
                    .method("POST")
                    .uri("/auth/login")
                    .header(axum::http::header::CONTENT_TYPE, "application/json")
                    .body(Body::from(r#"{"username":"admin","password":"initial-pass-1234"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(good.status(), StatusCode::OK);
        let set_cookie: Vec<String> = good
            .headers()
            .get_all(axum::http::header::SET_COOKIE)
            .iter()
            .filter_map(|v| v.to_str().ok().map(|s| s.to_string()))
            .collect();
        assert_eq!(set_cookie.len(), 2, "应同时下发 vtrim_sid 与 vtrim_csrf: {set_cookie:?}");
        assert!(set_cookie[0].contains("HttpOnly"));
        let sid = set_cookie[0]
            .split(';')
            .next()
            .unwrap()
            .trim_start_matches("vtrim_sid=")
            .to_string();
        let csrf = set_cookie[1]
            .split(';')
            .next()
            .unwrap()
            .trim_start_matches("vtrim_csrf=")
            .to_string();
        let cookie = format!("vtrim_sid={sid}; vtrim_csrf={csrf}");

        // /api/me：仍在使用初始密码
        let me = app
            .clone()
            .oneshot(
                HttpRequest::builder()
                    .uri("/me")
                    .header(axum::http::header::COOKIE, &cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(me.status(), StatusCode::OK);
        let v = body_json(me).await;
        assert_eq!(v["user"]["username"], "admin");
        assert_eq!(v["user"]["role"], "admin");
        assert_eq!(v["must_change_password"], true);

        // 旧密码错 → 400；正确 → 200 且之后 must_change_password=false
        let wrong_old = app
            .clone()
            .oneshot(
                HttpRequest::builder()
                    .method("PUT")
                    .uri("/auth/password")
                    .header(axum::http::header::COOKIE, &cookie)
                    .header(vtrim_service::auth::CSRF_HEADER, &csrf)
                    .header(axum::http::header::CONTENT_TYPE, "application/json")
                    .body(Body::from(r#"{"old_password":"nope","new_password":"new-pass-5678"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(wrong_old.status(), StatusCode::BAD_REQUEST);

        let changed = app
            .clone()
            .oneshot(
                HttpRequest::builder()
                    .method("PUT")
                    .uri("/auth/password")
                    .header(axum::http::header::COOKIE, &cookie)
                    .header(vtrim_service::auth::CSRF_HEADER, &csrf)
                    .header(axum::http::header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        r#"{"old_password":"initial-pass-1234","new_password":"new-pass-5678"}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(changed.status(), StatusCode::OK, "改密应成功");

        let me = app
            .clone()
            .oneshot(
                HttpRequest::builder()
                    .uri("/me")
                    .header(axum::http::header::COOKIE, &cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(body_json(me).await["must_change_password"], false);

        // 新密码可登录、旧密码不可
        let relogin = |pw: &'static str| {
            let app = app.clone();
            async move {
                app.oneshot(
                    HttpRequest::builder()
                        .method("POST")
                        .uri("/auth/login")
                        .header(axum::http::header::CONTENT_TYPE, "application/json")
                        .body(Body::from(format!(
                            r#"{{"username":"admin","password":"{pw}"}}"#
                        )))
                        .unwrap(),
                )
                .await
                .unwrap()
            }
        };
        assert_eq!(relogin("initial-pass-1234").await.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(relogin("new-pass-5678").await.status(), StatusCode::OK);

        // 登出后会话立即失效
        let out = app
            .clone()
            .oneshot(
                HttpRequest::builder()
                    .method("POST")
                    .uri("/auth/logout")
                    .header(axum::http::header::COOKIE, &cookie)
                    .header(vtrim_service::auth::CSRF_HEADER, &csrf)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(out.status(), StatusCode::OK);
        let me = app
            .oneshot(
                HttpRequest::builder()
                    .uri("/me")
                    .header(axum::http::header::COOKIE, &cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(me.status(), StatusCode::UNAUTHORIZED);
    }

    /// 连续错密码触发限流 → 429，且**正确密码也被挡住**（15 分钟内）。
    #[tokio::test]
    async fn login_is_rate_limited_after_five_failures() {
        use crate::server::auth::{AuthService, Listening};
        let dir = tempfile::tempdir().unwrap();
        let auth = AuthService::open(dir.path().to_path_buf(), Listening::Loopback).unwrap();
        let hash = vtrim_service::auth::hash_password("initial-pass-1234").unwrap();
        auth.db().create_user("admin", &hash, "admin", false, 0).unwrap();
        let ws = Workspace::load(Some(Path::new("vtrim.toml"))).expect("workspace");
        let app = router(Arc::new(AppState::with_auth(ws, std::env::temp_dir(), auth)));

        let attempt = |pw: &'static str| {
            let app = app.clone();
            async move {
                app.oneshot(
                    HttpRequest::builder()
                        .method("POST")
                        .uri("/auth/login")
                        .header(axum::http::header::CONTENT_TYPE, "application/json")
                        .body(Body::from(format!(
                            r#"{{"username":"admin","password":"{pw}"}}"#
                        )))
                        .unwrap(),
                )
                .await
                .unwrap()
                .status()
            }
        };
        for _ in 0..5 {
            assert_eq!(attempt("wrong-pass").await, StatusCode::UNAUTHORIZED);
        }
        assert_eq!(attempt("wrong-pass").await, StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(
            attempt("initial-pass-1234").await,
            StatusCode::TOO_MANY_REQUESTS,
            "限流窗口内正确密码也应被拒绝"
        );
    }

    /// 被禁用的账号：不能登录（文案直出），已建会话立即 403。
    #[tokio::test]
    async fn disabled_account_cannot_login_or_use_session() {
        let t = test_app().await;
        // 会话正常时可用
        let res = t
            .call(t.req("GET", "/jobs").body(Body::empty()).unwrap())
            .await;
        assert_eq!(res.status(), StatusCode::OK);

        assert!(t.auth.db().set_disabled("admin", true).unwrap());

        let res = t
            .call(t.req("GET", "/jobs").body(Body::empty()).unwrap())
            .await;
        assert_eq!(res.status(), StatusCode::FORBIDDEN, "禁用后已有会话必须拒绝");
        let v = body_json(res).await;
        assert!(v["error"].as_str().unwrap_or("").contains("禁用"), "{v}");

        let res = t
            .call(
                HttpRequest::builder()
                    .method("POST")
                    .uri("/auth/login")
                    .header(axum::http::header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        r#"{"username":"admin","password":"initial-pass-1234"}"#,
                    ))
                    .unwrap(),
            )
            .await;
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
        let v = body_json(res).await;
        assert!(v["error"].as_str().unwrap_or("").contains("禁用"), "{v}");
    }

    /// `awaiting_input` 链路（含 `/jobs/:id/decision`）已整体移除：
    /// 该端点现在必须走 JSON 404 兜底，而不是把任务状态改成 running 再返回 200。
    ///
    /// 注：`/models/download` 已去掉 `Json` 提取器（空 body 不再 400），
    /// 但这里不直接调它——它会真的启动 ~1.5GB 模型下载。
    #[tokio::test]
    async fn removed_decision_endpoint_falls_back_to_json_404() {
        let t = test_app().await;
        let res = t
            .call(
                HttpRequest::builder()
                    .method("POST")
                    .uri("/jobs/does-not-exist/decision")
                    .header(axum::http::header::CONTENT_TYPE, "application/json")
                    .body(Body::from("{}"))
                    .unwrap(),
            )
            .await;
        assert_eq!(res.status(), StatusCode::NOT_FOUND);
        let bytes = axum::body::to_bytes(res.into_body(), 64 * 1024).await.unwrap();
        assert!(String::from_utf8_lossy(&bytes).contains("未知接口"));
    }

    /// 删切片后的索引同步（破坏性路径）：publish_config.json 的条目要摘掉、
    /// clips_order.txt 的对应行要删掉，且**不能误伤别的片段**（clip-13 不能带走 clip-133）。
    #[test]
    fn prune_clip_from_index_removes_only_target_clip() {
        let td = tempfile::tempdir().unwrap();
        let out = td.path();
        std::fs::create_dir_all(out.join("clips")).unwrap();
        let publish = serde_json::json!({
            "video_source": "x.mp4",
            "clips": [
                { "directory": "clip-01", "rank": 1, "file": "clips/clip-01/clip.mp4" },
                { "directory": "clip-02", "rank": 2, "file": "clips/clip-02/clip.mp4" },
                { "directory": "clip-13", "rank": 13, "file": "clips/clip-13/clip.mp4" },
            ]
        });
        std::fs::write(out.join("publish_config.json"), serde_json::to_string_pretty(&publish).unwrap()).unwrap();
        std::fs::write(out.join("clips").join("clips_order.txt"), "clip-01 / 甲\nclip-02 / 乙\nclip-13 / 丙\nclip-133 / 丁").unwrap();

        prune_clip_from_index(out, "clip-02");
        prune_clip_from_index(out, "clip-13");

        let raw = std::fs::read_to_string(out.join("publish_config.json")).unwrap();
        let v: serde_json::Value = serde_json::from_str(&raw).unwrap();
        let dirs: Vec<&str> = v["clips"].as_array().unwrap().iter().filter_map(|c| c["directory"].as_str()).collect();
        assert_eq!(dirs, vec!["clip-01"]);

        let order = std::fs::read_to_string(out.join("clips").join("clips_order.txt")).unwrap();
        assert_eq!(order, "clip-01 / 甲\nclip-133 / 丁");
    }

    /// 索引文件缺失、或本来就没有该片段时必须是静默无操作（删目录已经成功了）。
    #[test]
    fn prune_clip_from_index_is_no_op_when_absent() {
        let td = tempfile::tempdir().unwrap();
        let out = td.path();
        prune_clip_from_index(out, "clip-09"); // 连 clips/ 都还没有
        std::fs::create_dir_all(out.join("clips")).unwrap();
        std::fs::write(out.join("clips").join("clips_order.txt"), "clip-01 / 甲").unwrap();
        prune_clip_from_index(out, "clip-02");
        assert_eq!(std::fs::read_to_string(out.join("clips").join("clips_order.txt")).unwrap(), "clip-01 / 甲");
    }

    /// 造一个切片目录：`clip-NN/clip.json`（原片区间）+ 假 clip.mp4（video_exists 才为真）。
    fn write_clip_dir(clips: &Path, rank: u32, start: f64, end: f64, srt_cues: usize, dm_cues: Option<usize>) {
        let dir = clips.join(format!("clip-{:0>2}", rank));
        std::fs::create_dir_all(&dir).unwrap();
        let meta = serde_json::json!({
            "duration": end - start,
            "original_start": start,
            "original_end": end,
            "rank": rank,
            "title": format!("片段{rank}"),
        });
        std::fs::write(dir.join("clip.json"), meta.to_string()).unwrap();
        std::fs::write(dir.join("clip.mp4"), b"fake").unwrap();
        let srt = |n: usize| (0..n).map(|i| format!("{i}\n00:00:0{i}.000 --> 00:00:0{}.500\n字幕{i}\n", i + 1)).collect::<String>();
        std::fs::write(dir.join("clip.srt"), srt(srt_cues)).unwrap();
        if let Some(n) = dm_cues {
            std::fs::write(dir.join("clip.dm.srt"), srt(n)).unwrap();
        }
    }

    /// 切片页两条回归（走真实路由 + 真实文件）：
    /// 1) 老切片没有 `clip.dm.srt` 时，弹幕量按 `clip.json` 的原片区间从**源弹幕**数出来 ——
    ///    否则整列都是 0，「弹幕量」排序会静默退化成按 rank 排（用户报过"排序没生效"）；
    ///    有 `clip.dm.srt` 时仍以文件为准。
    /// 2) `DELETE /project-data/clips/:rank` 删掉整个目录，并把引用从 `publish_config.json` /
    ///    `clips_order.txt` 里同步摘掉（发布配置不能指向已删文件）。
    #[tokio::test]
    async fn clip_danmaku_count_falls_back_to_source_and_delete_prunes_index() {
        let t = test_app().await;
        let root = t.root.clone();
        // 源弹幕：0-10s 一条、10-20s 两条、20-30s 三条（ASS，走 load_danmaku_auto 的 ASS 分支）
        let mut ass = String::from("[Script Info]\nScriptType: v4.00+\n\n[Events]\nFormat: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\n");
        for (i, sec) in [0.5f64, 10.5, 11.5, 20.5, 21.5, 22.5].iter().enumerate() {
            ass.push_str(&format!("Dialogue: 0,0:00:{:05.2},0:00:{:05.2},R2L,,0,0,0,,弹幕{i}\n", sec, sec + 1.0));
        }
        std::fs::write(root.join("dm.ass"), ass).unwrap();
        // `danmaku_file` 是 `[project]` 表里的字段（顶层写不进去，会被忽略）
        std::fs::write(root.join("vtrim.toml"), "[project]\ninput_file = \"a.mp4\"\ndanmaku_file = \"dm.ass\"\n").unwrap();

        let clips = root.join("output").join("clips");
        std::fs::create_dir_all(&clips).unwrap();
        write_clip_dir(&clips, 1, 0.0, 10.0, 2, None);        // 无 clip.dm.srt → 数源弹幕
        write_clip_dir(&clips, 2, 10.0, 20.0, 3, None);
        write_clip_dir(&clips, 3, 20.0, 30.0, 1, None);
        write_clip_dir(&clips, 4, 30.0, 40.0, 1, Some(5));     // 有 clip.dm.srt → 以文件为准（源弹幕这里为 0）

        let res = t.call(t.req("GET", "/project-data/clips?project=1").body(Body::empty()).unwrap()).await;
        assert_eq!(res.status(), StatusCode::OK);
        let v = body_json(res).await;
        let counts: std::collections::HashMap<String, u64> = v.as_array().unwrap().iter()
            .map(|c| (c["name"].as_str().unwrap_or("").to_string(), c["danmaku_count"].as_u64().unwrap_or(0)))
            .collect();
        assert_eq!(counts.get("clip-01"), Some(&1), "缺 clip.dm.srt 时应回落到源弹幕计数: {v}");
        assert_eq!(counts.get("clip-02"), Some(&2), "{v}");
        assert_eq!(counts.get("clip-03"), Some(&3), "{v}");
        assert_eq!(counts.get("clip-04"), Some(&5), "有 clip.dm.srt 时以文件为准: {v}");
        let subs: std::collections::HashMap<String, u64> = v.as_array().unwrap().iter()
            .map(|c| (c["name"].as_str().unwrap_or("").to_string(), c["subtitle_count"].as_u64().unwrap_or(0)))
            .collect();
        assert_eq!(subs.get("clip-02"), Some(&3), "{v}");

        // 发布索引里放三条，删掉中间一条后必须只剩两条
        let publish = serde_json::json!({
            "video_source": "a.mp4",
            "clips": [ { "directory": "clip-01" }, { "directory": "clip-02" }, { "directory": "clip-03" } ]
        });
        std::fs::write(root.join("output").join("publish_config.json"), serde_json::to_string_pretty(&publish).unwrap()).unwrap();
        std::fs::write(clips.join("clips_order.txt"), "clip-01 / 一\nclip-02 / 二\nclip-03 / 三").unwrap();

        let res = t.call(t.req("DELETE", "/project-data/clips/2?project=1").body(Body::empty()).unwrap()).await;
        let status = res.status();
        let j = body_json(res).await;
        assert_eq!(status, StatusCode::OK, "{j}");
        assert!(!clips.join("clip-02").exists(), "目录必须被删掉");
        assert!(clips.join("clip-01").is_dir() && clips.join("clip-03").is_dir(), "别的片段不能受影响");

        let raw = std::fs::read_to_string(root.join("output").join("publish_config.json")).unwrap();
        let pv: serde_json::Value = serde_json::from_str(&raw).unwrap();
        let dirs: Vec<&str> = pv["clips"].as_array().unwrap().iter().filter_map(|c| c["directory"].as_str()).collect();
        assert_eq!(dirs, vec!["clip-01", "clip-03"], "{pv}");
        assert_eq!(std::fs::read_to_string(clips.join("clips_order.txt")).unwrap(), "clip-01 / 一\nclip-03 / 三");

        // 删完列表里当然也没有它了（事件会自动回到「未剪切」，这里只确认片段数据）
        let res = t.call(t.req("GET", "/project-data/clips?project=1").body(Body::empty()).unwrap()).await;
        let v = body_json(res).await;
        assert!(!v.as_array().unwrap().iter().any(|c| c["name"] == "clip-02"), "{v}");

        // 重复删 / 名字不存在：都是 404（`abc` 现在按**目录名**理解，不是非法输入）
        let res = t.call(t.req("DELETE", "/project-data/clips/2?project=1").body(Body::empty()).unwrap()).await;
        assert_eq!(res.status(), StatusCode::NOT_FOUND);
        let res = t.call(t.req("DELETE", "/project-data/clips/abc?project=1").body(Body::empty()).unwrap()).await;
        assert_eq!(res.status(), StatusCode::NOT_FOUND, "不存在的目录名 → 404");
        assert!(clips.join("clip-01").is_dir(), "不存在的名字不能误删");
    }

    /// `resolve_clip_dir`：数字 rank（老契约 `2` → `clip-02`）与**目录名**（`clip-out` 这类
    /// 手工命名的目录，卡片墙列的就是真实目录名）都要认；`..`/分隔符/盘符一律 400 ——
    /// 这个参数直接来自 URL，拼出来的路径只能落在 `clips/` 里。
    #[test]
    fn resolve_clip_dir_accepts_rank_and_directory_name() {
        let td = tempfile::tempdir().unwrap();
        let out = td.path();
        let clips = out.join("clips");
        std::fs::create_dir_all(clips.join("clip-out")).unwrap();

        assert_eq!(resolve_clip_dir(out, "3").unwrap(), clips.join("clip-03"), "数字按老契约解释成 rank");
        assert_eq!(resolve_clip_dir(out, "clip-out").unwrap(), clips.join("clip-out"), "存在的目录名原样用");
        assert_eq!(resolve_clip_dir(out, "nope").unwrap(), clips.join("nope"), "不存在的名字原样返回（调用方报 404）");
        for bad in ["..", "../x", "a/b", "a\\b", "C:/x", "nul", ""] {
            let (status, _) = resolve_clip_dir(out, bad).unwrap_err();
            assert_eq!(status, StatusCode::BAD_REQUEST, "{bad:?} 必须 400");
        }
    }

    /// 手工命名的切片目录（`clip-out`）也要能删、能出缩略图：
    /// 卡片墙列的是 `clips/` 下的**真实目录名**，名字不一定规范。
    /// 以前接口只认数字 rank，前端把 `clip-out` 解析成 rank 0 → 请求 `clips/0` →
    /// 稳定报「clip-00 不存在」（删除永远失败），缩略图则每次加载刷一条 400。
    #[tokio::test]
    async fn clip_delete_and_thumb_accept_directory_name() {
        let t = test_app().await;
        let root = t.root.clone();
        let clips = root.join("output").join("clips");
        std::fs::create_dir_all(clips.join("clip-out")).unwrap();
        std::fs::write(clips.join("clip-out").join("clip.mp4"), b"fake").unwrap();
        std::fs::write(root.join("output").join("publish_config.json"), r#"{"clips":[{"directory":"clip-out"}]}"#).unwrap();
        std::fs::write(clips.join("clips_order.txt"), "clip-out / 手工目录").unwrap();

        // 缩略图：按目录名请求，不能被当成非法 rank 拒掉（假 mp4 生成不出图 → 404 也算正常）
        let res = t.call(t.req("GET", "/project-data/clips/clip-out/thumb?project=1").body(Body::empty()).unwrap()).await;
        assert_ne!(res.status(), StatusCode::BAD_REQUEST, "目录名不该被当成非法 rank");

        let res = t.call(t.req("DELETE", "/project-data/clips/clip-out?project=1").body(Body::empty()).unwrap()).await;
        let status = res.status();
        let j = body_json(res).await;
        assert_eq!(status, StatusCode::OK, "{j}");
        assert_eq!(j["removed"], "clip-out", "{j}");
        assert!(!clips.join("clip-out").exists(), "目录必须被删掉");
        let raw = std::fs::read_to_string(root.join("output").join("publish_config.json")).unwrap();
        let pv: serde_json::Value = serde_json::from_str(&raw).unwrap();
        assert!(pv["clips"].as_array().unwrap().is_empty(), "索引里的引用要摘掉: {pv}");
        assert_eq!(std::fs::read_to_string(clips.join("clips_order.txt")).unwrap(), "");
    }

    /// 资源域被占用时，新任务**排队**而不是 409（旧版是"已有任务在运行，请等待"）。
    /// 这里验的是 handler 那一侧：资源键推导、`queued` 回传、以及取消排队任务不发车。
    #[tokio::test]
    async fn pipeline_job_queues_behind_conflicting_task() {
        let t = test_app().await;
        // 占住项目输出域（管线 / 剪切的资源键）
        let busy = t
            .state
            .jobs
            .create(
                1,
                JobSpec::new(
                    "stage_detect",
                    "检测",
                    vec![crate::server::scope_key::project_out(1)],
                    JobClass::Medium,
                ),
            )
            .await;
        assert!(!busy.queued);

        let res = t
            .call(
                t.req("POST", "/project-data/pipeline?project=1")
                    .header(axum::http::header::CONTENT_TYPE, "application/json")
                    .body(Body::from(r#"{"stage":"detect"}"#))
                    .unwrap(),
            )
            .await;
        assert_eq!(res.status(), StatusCode::OK, "冲突时应排队而不是 409");
        let j = body_json(res).await;
        assert_eq!(j["queued"], true, "{j}");
        let id = j["id"].as_str().unwrap().to_string();
        let detail = t.state.jobs.get(&id).await.unwrap();
        assert_eq!(detail.status, "queued");
        assert!(detail.resources.contains(&crate::server::scope_key::project_out(1)), "{detail:?}");

        // 排队中被取消：收到 Err 直接收工（不跑管线、不发车）
        assert!(t.state.jobs.cancel(&id).await);
        t.state.jobs.complete(&busy.id).await;
        assert_eq!(t.state.jobs.get(&id).await.unwrap().status, "cancelled");
    }

    /// 片段目录被任务占用（渲染/精修/重切）时，**同步**写操作必须当场 409 ——
    /// 删除会把正在渲染的目录删掉、留下半个工程；合并会重写 events.json。
    /// 任务结束后同一请求要恢复正常。
    #[tokio::test]
    async fn sync_writes_are_rejected_while_scope_is_busy() {
        let t = test_app().await;
        let root = t.root.clone();
        let clips = root.join("output").join("clips");
        std::fs::create_dir_all(clips.join("clip-03")).unwrap();
        std::fs::write(clips.join("clip-03").join("clip.mp4"), b"fake").unwrap();
        std::fs::write(root.join("output").join("events.json"), r#"{"events":[]}"#).unwrap();

        // 造一个占着 clip-03 的任务（真任务会立刻开跑；这里不建工作协程，
        // 它就一直是 Running，等价于"渲染进行中"）
        let busy = t
            .state
            .jobs
            .create(
                1,
                JobSpec::new(
                    "polish_render",
                    "渲染 clip-03",
                    vec![crate::server::scope_key::project_clip(1, "clip-03")],
                    JobClass::Heavy,
                ),
            )
            .await;
        assert!(!busy.queued, "空载时应立刻开跑");

        let res = t.call(t.req("DELETE", "/project-data/clips/clip-03?project=1").body(Body::empty()).unwrap()).await;
        assert_eq!(res.status(), StatusCode::CONFLICT, "被占用时删除必须被拒");
        let j = body_json(res).await;
        assert!(j["error"].as_str().unwrap().contains("polish_render"), "{j}");
        assert!(clips.join("clip-03").exists(), "被拒时不能动目录");

        // 项目级资源（events.json / output/*）是另一个域：精修片段不该挡住合并事件
        let res = t.call(t.req("POST", "/project-data/events/merge-splits?project=1").body(Body::empty()).unwrap()).await;
        assert_ne!(res.status(), StatusCode::CONFLICT, "精修某片段与合并事件无关，不该互相挡");

        // 管线/剪切任务占的是 `p1:out`：那时 events.json 的同步写就必须被拒
        let pipeline = t
            .state
            .jobs
            .create(
                1,
                JobSpec::new(
                    "stage_detect",
                    "检测",
                    vec![crate::server::scope_key::project_out(1)],
                    JobClass::Medium,
                ),
            )
            .await;
        let res = t.call(t.req("POST", "/project-data/events/merge-splits?project=1").body(Body::empty()).unwrap()).await;
        assert_eq!(res.status(), StatusCode::CONFLICT, "管线在跑时合并事件必须被拒");
        t.state.jobs.complete(&pipeline.id).await;

        // 任务结束后恢复正常
        t.state.jobs.complete(&busy.id).await;
        let res = t.call(t.req("DELETE", "/project-data/clips/clip-03?project=1").body(Body::empty()).unwrap()).await;
        assert_eq!(res.status(), StatusCode::OK);
        assert!(!clips.join("clip-03").exists());
    }

    /// `POST /project-data/events/merge-splits`：把老的等分产物 `xxx（1/2）`/`（2/2）`
    /// 合回一个事件（时间取并集、标题去后缀、rank 沿用组内最热那条）；
    /// 同名但**时间不相邻**的一对是给两个不同话题加的区分后缀，绝不能被合掉。
    #[tokio::test]
    async fn merge_splits_endpoint_merges_equal_splits_only() {
        let t = test_app().await;
        let root = t.root.clone();
        let ts = |s: f64| {
            let total = (s * 1000.0).round() as u64;
            serde_json::json!({ "h": total / 3_600_000, "m": (total / 60_000) % 60, "s": (total / 1000) % 60, "ms": total % 1000 })
        };
        let ev = |rank: u64, label: &str, s: f64, e: f64, pop: f64| {
            serde_json::json!({
                "start": ts(s), "end": ts(e), "label": label, "summary": format!("摘要 {label}"),
                "event_type": "chat", "source": "LLM", "confidence": 1.0,
                "popularity": pop, "rank": rank, "enabled": true
            })
        };
        let events = serde_json::json!({
            "titles": {},
            "events": [
                ev(4, "神婆见我一脸惊恐：你把两次机会都用完了？😱（1/2）", 0.0, 216.0, 0.30),
                ev(29, "神婆见我一脸惊恐：你把两次机会都用完了？😱（2/2）", 216.0, 432.0, 0.80),
                ev(7, "同一个话题（1/2）", 600.0, 700.0, 0.10),
                ev(8, "同一个话题（2/2）", 2000.0, 2100.0, 0.10),
                ev(9, "正常事件", 800.0, 900.0, 0.05)
            ]
        });
        std::fs::write(root.join("output").join("events.json"), serde_json::to_string_pretty(&events).unwrap()).unwrap();

        let res = t.call(t.req("POST", "/project-data/events/merge-splits?project=1").body(Body::empty()).unwrap()).await;
        let status = res.status();
        let j = body_json(res).await;
        assert_eq!(status, StatusCode::OK, "{j}");
        assert_eq!(j["groups"], 1, "{j}");
        assert_eq!(j["before"], 5, "{j}");
        assert_eq!(j["after"], 4, "{j}");

        let raw = std::fs::read_to_string(root.join("output").join("events.json")).unwrap();
        let v: serde_json::Value = serde_json::from_str(&raw).unwrap();
        let evs = v["events"].as_array().unwrap();
        let merged = evs.iter().find(|e| e["label"].as_str().unwrap_or("").starts_with("神婆")).expect("合并后的事件还在");
        assert_eq!(merged["label"], "神婆见我一脸惊恐：你把两次机会都用完了？😱", "后缀必须去掉");
        assert_eq!(merged["start"], ts(0.0), "时间取并集（起点）");
        assert_eq!(merged["end"], ts(432.0), "时间取并集（终点）");
        assert_eq!(merged["rank"], 29, "rank 沿用组内最热的那条（已剪片段里至少一个目录还对得上）");
        assert!(!evs.iter().any(|e| e["label"].as_str().unwrap_or("").starts_with("神婆") && e["label"].as_str().unwrap_or("").contains("（")),
                "被合的成员不能留在文件里: {v}");
        assert_eq!(evs.iter().filter(|e| e["label"].as_str().unwrap_or("").starts_with("同一个话题")).count(), 2,
                   "同名但时间不相邻的一对不能被合并: {v}");
        assert!(evs.iter().any(|e| e["label"] == "正常事件"), "{v}");

        // 幂等：再点一次没有可合并的组，文件不再被改写
        let res = t.call(t.req("POST", "/project-data/events/merge-splits?project=1").body(Body::empty()).unwrap()).await;
        let j = body_json(res).await;
        assert_eq!(j["groups"], 0, "{j}");
        assert_eq!(j["after"], 4, "{j}");
    }

    /// 渲染/封面子进程的日志提取：成功分支原本把 stdout 丢掉，用户看不到产出叫 `_1` / `_2`
    /// （同名不覆盖），只好从 `FINISHED:` / `Cover saved:` 那行里把实际路径捞出来记进任务日志。
    #[test]
    fn subprocess_note_extracts_written_path() {
        let render = "\u{1b}[32m2026-09-14T17:44:03\u{1b}[0m \u{1b}[32mINFO\u{1b}[0m vtrim: \
             FINISHED: F:\\video\\clip-23\\vtuber-clip_1.mp4 (96.2MB, 143s)";
        assert_eq!(
            subprocess_note(render, "FINISHED:").as_deref(),
            Some("F:\\video\\clip-23\\vtuber-clip_1.mp4")
        );
        let cover = "INFO vtrim_polish::cover:   \u{2713} Cover saved: F:\\video\\clip-23\\cover_2.jpg";
        assert_eq!(
            subprocess_note(cover, "Cover saved:").as_deref(),
            Some("F:\\video\\clip-23\\cover_2.jpg")
        );
        // 没有那行（老版本 CLI / 失败路径）时返回 None，调用方就不写日志
        assert_eq!(subprocess_note("boom", "FINISHED:"), None);
        assert_eq!(strip_ansi("\u{1b}[1mhi\u{1b}[0m"), "hi");
    }

    /// `GET /project-data/clips` 必须按**数字**序号返回：`clip-100` 排在 `clip-02` 之后。
    /// 按字符串排会得到 clip-01, clip-02, clip-100, clip-101, …, clip-11 —— 精修页的片段条
    /// 直接照这个顺序显示，用户报过"片段列表排序有问题"（只有两张卡时还隔着老远）。
    #[tokio::test]
    async fn clips_list_is_sorted_by_numeric_rank() {
        let t = test_app().await;
        let clips = t.root.join("output").join("clips");
        std::fs::create_dir_all(&clips).unwrap();
        for rank in [100u32, 2, 10] {
            write_clip_dir(&clips, rank, 0.0, 10.0, 1, None);
        }
        let res = t.call(t.req("GET", "/project-data/clips?project=1").body(Body::empty()).unwrap()).await;
        assert_eq!(res.status(), StatusCode::OK);
        let v = body_json(res).await;
        let names: Vec<String> = v.as_array().unwrap().iter()
            .filter_map(|c| c["name"].as_str().map(|s| s.to_string()))
            .collect();
        assert_eq!(names, vec!["clip-02", "clip-10", "clip-100"], "{v}");
    }

    /// 邀请码注册链路：
    /// - HTTP 层：注册开关关着（测试环境没有全局配置 → 默认关）→ 403，且不透露细节；
    /// - 服务层：有效码 → 建号（角色固定 user）+ 建会话 + 码被消费；码复用/假码/重名/弱密码 → 拒；
    /// - 限流：连续失败会被锁（防邀请码爆破）。
    #[tokio::test]
    async fn register_flow_consumes_invite_and_respects_switch() {
        use vtrim_service::auth as kdf;
        let t = test_app().await;

        // 1) 开关关着（默认）→ 公开接口 403
        let body = serde_json::json!({
            "username": "newbie", "password": "pass-12345678", "invite": kdf::new_invite_code()
        })
        .to_string();
        let res = t
            .app
            .clone()
            .oneshot(
                HttpRequest::builder()
                    .method("POST")
                    .uri("/auth/register")
                    .header(axum::http::header::CONTENT_TYPE, "application/json")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::FORBIDDEN, "注册未开放必须 403");

        // 2) 造一个有效码 → 注册成功，用码的人记在邀请码行上
        let code = kdf::new_invite_code();
        let invite_id = t
            .auth
            .db()
            .create_invite(&code, Some("测试"), None, None)
            .unwrap();
        let bundle = t
            .auth
            .register("newbie", "pass-12345678", &code, Some("10.0.0.1"), None)
            .expect("有效码应能注册");
        assert_eq!(bundle.user.role, "user", "自助注册只能是 user");
        assert_eq!(bundle.user.username, "newbie");
        assert!(!bundle.user.must_change_password, "自设密码不提示改密");
        let row = t.auth.db().find_invite_by_id(invite_id).unwrap().unwrap();
        assert_eq!(row.used_by, Some(bundle.user.id));
        assert!(!row.is_open(vtrim_service::db::now_secs()), "用完即失效");

        // 注册即登录：这个 sid 直接能过鉴权
        let res = t
            .app
            .clone()
            .oneshot(
                HttpRequest::builder()
                    .method("GET")
                    .uri("/me")
                    .header(
                        axum::http::header::COOKIE,
                        format!("{}={}", vtrim_service::auth::COOKIE_SID, bundle.session_id),
                    )
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK, "注册后会话应立即可用");

        // 3) 同一个码再用 / 假码 / 重名（大小写不同也算）/ 弱密码 → 全拒
        assert!(
            t.auth.register("someone", "pass-12345678", &code, Some("10.0.0.2"), None).is_err(),
            "码不能复用"
        );
        assert!(
            t.auth.register("other", "pass-12345678", "vtrim_inv_aaaaaaaaaaaaaaaa", Some("10.0.0.3"), None).is_err(),
            "假码必须拒"
        );
        assert!(
            t.auth.register("NEWBIE", "pass-12345678", &kdf::new_invite_code(), Some("10.0.0.4"), None).is_err(),
            "大小写不同的重名也要拒"
        );
        assert!(
            t.auth.register("weakling", "short", &kdf::new_invite_code(), Some("10.0.0.5"), None).is_err(),
            "弱密码必须拒"
        );
        assert!(t.auth.db().find_user("someone").unwrap().is_none(), "失败不留半个账号");

        // 4) 限流：同一个 IP 连续失败会被锁（邀请码爆破防护）
        let mut limited = false;
        for _ in 0..6 {
            if let Err(crate::server::auth::LoginError::RateLimited(_)) =
                t.auth.register("bruteforce", "pass-12345678", "vtrim_inv_bbbbbbbbbbbb", Some("10.9.9.9"), None)
            {
                limited = true;
                break;
            }
        }
        assert!(limited, "连续失败必须触发限流");
    }

    /// 直播间接口：没对接录播姬时 `available=false`（**普通用户不返回管理员指引**），
    /// 输入校验先于一切（假房间号 400），未对接时加房间 503 且不留登记。
    #[tokio::test]
    async fn rooms_endpoints_validate_and_report_availability() {
        let t = test_app().await;

        // admin：能看到"为什么不可用"（配置指引）
        let res = t.call(t.req("GET", "/rooms").body(Body::empty()).unwrap()).await;
        assert_eq!(res.status(), StatusCode::OK);
        let j = body_json(res).await;
        assert_eq!(j["available"], false, "{j}");
        assert!(j["reason"].as_str().unwrap_or("").contains("api_base"), "admin 应看到配置指引: {j}");
        assert_eq!(j["rooms"].as_array().unwrap().len(), 0);

        // 输入校验：链接里没有房间号 → 400（不碰录播姬、不留登记）
        let body = serde_json::json!({ "url": "https://example.com/abc" }).to_string();
        let res = t
            .call(
                t.req("POST", "/rooms")
                    .header(axum::http::header::CONTENT_TYPE, "application/json")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await;
        assert_eq!(res.status(), StatusCode::BAD_REQUEST);

        // 合法房间号但没对接录播姬 → 503，且不能留下半个登记
        let body = serde_json::json!({ "url": "https://live.bilibili.com/12345" }).to_string();
        let res = t
            .call(
                t.req("POST", "/rooms")
                    .header(axum::http::header::CONTENT_TYPE, "application/json")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await;
        assert_eq!(res.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert!(t.auth.db().rooms_of_user(1).unwrap().is_empty(), "失败不该留下登记");

        // 普通用户：同样 200，但**不给**管理员向的 reason，也不泄露地址
        let hash = vtrim_service::auth::hash_password("pass-12345678").unwrap();
        let uid = t.auth.db().create_user("plain2", &hash, "user", false, 0).unwrap();
        t.auth.db().room_upsert("777", uid, "他的房间").unwrap();
        let b = t.auth.login("plain2", "pass-12345678", Some("10.3.3.3"), None).unwrap();
        let res = t
            .app
            .clone()
            .oneshot(
                HttpRequest::builder()
                    .method("GET")
                    .uri("/rooms")
                    .header(
                        axum::http::header::COOKIE,
                        format!("{}={}", vtrim_service::auth::COOKIE_SID, b.session_id),
                    )
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let j = body_json(res).await;
        assert_eq!(j["available"], false);
        assert!(j.get("reason").is_none(), "普通用户不显示管理员指引: {j}");
        assert_eq!(j["rooms"].as_array().unwrap().len(), 1, "只看得到自己的房间: {j}");
        assert_eq!(j["rooms"][0]["title"], "他的房间");
    }

    /// 邀请码管理是 admin-only：普通账号（哪怕已登录）读写都拿 403。
    #[tokio::test]
    async fn invite_admin_endpoints_reject_non_admin() {
        let t = test_app().await;
        let hash = vtrim_service::auth::hash_password("pass-12345678").unwrap();
        t.auth.db().create_user("plain", &hash, "user", false, 0).unwrap();
        let b = t.auth.login("plain", "pass-12345678", Some("10.1.1.1"), None).unwrap();
        let cookie = format!("{}={}", vtrim_service::auth::COOKIE_SID, b.session_id);
        for (method, uri) in [
            ("GET", "/admin/invites"),
            ("POST", "/admin/invites"),
            ("DELETE", "/admin/invites/1"),
            ("PUT", "/admin/invites/registration"),
            // LLM 用量同样带各用户的项目路径 → 与日志一样只给 admin
            ("GET", "/admin/usage"),
        ] {
            let res = t
                .app
                .clone()
                .oneshot(
                    HttpRequest::builder()
                        .method(method)
                        .uri(uri)
                        .header(axum::http::header::COOKIE, cookie.clone())
                        .header(vtrim_service::auth::CSRF_HEADER, b.csrf.clone())
                        .header(axum::http::header::CONTENT_TYPE, "application/json")
                        .body(Body::from("{}"))
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(res.status(), StatusCode::FORBIDDEN, "{method} {uri} 必须拒绝普通用户");
        }
    }

    /// 访问日志：**每次前端交互都留痕**（方法/路径/状态/耗时/操作者），
    /// 错误响应还要把服务端的 `error` 文案带出来 —— 这正是"静默消失"的正面修复。
    #[tokio::test]
    async fn access_log_records_requests_with_actor_and_error_text() {
        use tracing_subscriber::layer::SubscriberExt;
        let bus = logbus::bus();
        let before = bus.snapshot(0, 1).1;
        // 线程局部订阅者：只有装上 LogBusLayer，tracing 事件才会进总线
        // （测试进程不会跑 main 的 setup_tracing；guard 出作用域即还原）
        let _sub = tracing::subscriber::set_default(
            tracing_subscriber::registry()
                .with(logbus::LogBusLayer::new(bus.clone())),
        );

        let t = test_app().await;
        // 1) 已登录的正常请求：要能看出是谁在操作
        let res = t.call(t.req("GET", "/me").body(Body::empty()).unwrap()).await;
        assert_eq!(res.status(), StatusCode::OK);
        // 2) 未注册接口：JSON 404 的 error 文案必须进日志
        let probe = "/__access_log_probe_7f3a";
        let res = t
            .call(HttpRequest::builder().uri(probe).body(Body::empty()).unwrap())
            .await;
        assert_eq!(res.status(), StatusCode::NOT_FOUND);
        // 3) 未登录请求：操作者记成 `-`
        let res = t
            .call(HttpRequest::builder().uri("/me").body(Body::empty()).unwrap())
            .await;
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED);

        let all = bus
            .snapshot(before, usize::MAX)
            .0
            .into_iter()
            .map(|r| format!("[{}] {}", r.level, r.text))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            all.contains("GET /api/me → 200") && all.contains("user=admin"),
            "缺已登录请求的记录：\n{all}"
        );
        assert!(
            all.contains(probe) && all.contains("未知接口"),
            "404 记录里应带服务端 error 文案：\n{all}"
        );
        assert!(
            all.contains("GET /api/me → 401") && all.contains("user=-"),
            "未登录请求应记 user=-：\n{all}"
        );
    }

    /// 生产是 `nest("/api", api::router(…))`：访问日志里的路径必须带 `/api` 前缀
    /// （单测直接打 router 时靠补前缀，两条路径都得对）。
    #[tokio::test]
    async fn access_log_keeps_api_prefix_when_nested() {
        use tracing_subscriber::layer::SubscriberExt;
        let bus = logbus::bus();
        let before = bus.snapshot(0, 1).1;
        let _sub = tracing::subscriber::set_default(
            tracing_subscriber::registry()
                .with(logbus::LogBusLayer::new(bus.clone())),
        );
        let t = test_app().await;
        let app = Router::new().nest("/api", t.app.clone());
        let res = app
            .oneshot(
                HttpRequest::builder()
                    .uri("/api/me")
                    .header(axum::http::header::COOKIE, t.cookie())
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);

        let all = bus
            .snapshot(before, usize::MAX)
            .0
            .into_iter()
            .map(|r| r.text)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            all.contains("GET /api/me → 200") && all.contains("user=admin"),
            "嵌套下路径要带 /api：\n{all}"
        );
    }

    /// `/api/admin/logs`：仅 admin，形状与 `webui/src/stores/log.js` 对齐。
    #[tokio::test]
    async fn admin_logs_is_admin_only_and_returns_records() {
        use tracing_subscriber::layer::SubscriberExt;
        let bus = logbus::bus();
        let _sub = tracing::subscriber::set_default(
            tracing_subscriber::registry()
                .with(logbus::LogBusLayer::new(bus.clone())),
        );
        let t = test_app().await;
        // 先制造一条日志（这次请求本身）
        let res = t.call(t.req("GET", "/me").body(Body::empty()).unwrap()).await;
        assert_eq!(res.status(), StatusCode::OK);

        let res = t
            .call(t.req("GET", "/admin/logs?limit=20").body(Body::empty()).unwrap())
            .await;
        assert_eq!(res.status(), StatusCode::OK);
        let v = body_json(res).await;
        let latest = v["latest"].as_u64().unwrap_or(0);
        assert!(latest > 0, "应已有日志：{v}");
        let recs = v["records"].as_array().cloned().unwrap_or_default();
        assert!(!recs.is_empty(), "应回放出刚才那次请求：{v}");
        let last = recs.last().unwrap();
        for k in ["seq", "time", "level", "target", "text"] {
            assert!(last.get(k).is_some(), "记录缺字段 {k}：{last}");
        }

        // 游标语义：`after=<最新>` 只给更新的记录（前端断线补齐靠它）
        let res = t
            .call(t.req("GET", &format!("/admin/logs?after={latest}")).body(Body::empty()).unwrap())
            .await;
        assert_eq!(res.status(), StatusCode::OK);
        let v2 = body_json(res).await;
        assert!(v2["records"]
            .as_array()
            .unwrap()
            .iter()
            .all(|r| r["seq"].as_u64().unwrap_or(0) > latest));

        // 普通用户一律 403（日志含其他用户的路径与任务输出）
        let hash = vtrim_service::auth::hash_password("pass-12345678").unwrap();
        t.auth.db().create_user("plain_logs", &hash, "user", false, 0).unwrap();
        let b = t
            .auth
            .login("plain_logs", "pass-12345678", Some("10.9.9.9"), None)
            .unwrap();
        let res = t
            .app
            .clone()
            .oneshot(
                HttpRequest::builder()
                    .uri("/admin/logs")
                    .header(
                        axum::http::header::COOKIE,
                        format!("{}={}", vtrim_service::auth::COOKIE_SID, b.session_id),
                    )
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::FORBIDDEN, "普通用户不得读取服务端日志");
    }

    /// `/api/admin/logs/events`：SSE 先回放环形缓冲里 `seq > after` 的，再续实时。
    ///
    /// 面板刚打开的那几秒就靠这段回放（否则只能看到连接之后的新日志）。
    #[tokio::test]
    async fn admin_logs_stream_replays_history() {
        use tracing_subscriber::layer::SubscriberExt;
        let bus = logbus::bus();
        let _sub = tracing::subscriber::set_default(
            tracing_subscriber::registry()
                .with(logbus::LogBusLayer::new(bus.clone())),
        );
        let t = test_app().await;
        let cursor = bus.snapshot(0, 1).1;
        bus.push("info", "test", "SSE 回放探针 abc123".into());

        let res = t
            .call(t.req("GET", &format!("/admin/logs/events?after={cursor}")).body(Body::empty()).unwrap())
            .await;
        assert_eq!(res.status(), StatusCode::OK);
        let ct = res
            .headers()
            .get(axum::http::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        assert!(ct.contains("text/event-stream"), "content-type = {ct}");

        // SSE 是无限流：收集若干帧（并行测试也会往同一条总线上塞记录，所以是"找"不是"等第一帧"）
        let mut body = res.into_body().into_data_stream();
        let mut seen = String::new();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        while std::time::Instant::now() < deadline && !seen.contains("SSE 回放探针 abc123") {
            match tokio::time::timeout(std::time::Duration::from_millis(500), body.next()).await {
                Ok(Some(Ok(chunk))) => seen.push_str(&String::from_utf8_lossy(&chunk)),
                _ => break,
            }
        }
        assert!(seen.contains("SSE 回放探针 abc123"), "应回放历史记录：{seen}");
        assert!(
            seen.contains("\"seq\"") && seen.contains("\"level\""),
            "帧应是 LogRecord JSON：{seen}"
        );
    }

    /// `/api/models` 的每条必须带上「实际路径 + 来源」：判定跟运行时同一套查找顺序
    /// （配置 → 用户目录 → 程序目录），界面照它显示。曾经只回 `dir.exists()`（程序目录），
    /// qwen3-asr 放在用户目录 → 界面说"未下载"却一直在用它（用户报过）。
    #[tokio::test]
    async fn models_endpoint_reports_runtime_resolved_dir_and_source() {
        let t = test_app().await;
        let res = t.call(t.req("GET", "/models").body(Body::empty()).unwrap()).await;
        assert_eq!(res.status(), StatusCode::OK);
        let v = body_json(res).await;
        let arr = v.as_array().expect("array");
        let names: Vec<&str> = arr.iter().filter_map(|m| m["name"].as_str()).collect();
        // 注册表驱动：firered-zh 已下线，不该再出现在清单里
        assert_eq!(names, vec!["sensevoice", "qwen3-asr"], "{v}");
        for m in arr {
            let name = m["name"].as_str().unwrap();
            assert!(m["installed"].is_boolean(), "{m}");
            let dir = m["dir"].as_str().unwrap_or("");
            assert!(Path::new(dir).ends_with(std::path::Path::new("models").join(name)), "{m}");
            assert!(["config", "user", "exe"].contains(&m["source"].as_str().unwrap_or("")), "{m}");
            assert!(!m["source_label"].as_str().unwrap_or("").is_empty(), "{m}");
        }
    }

    /// 管理员能**重复查看**邀请码明文，并一眼看出「已使用 / 未使用 + 绑给了谁」（用户要求）。
    #[tokio::test]
    async fn admin_invites_expose_plaintext_and_usage() {
        let t = test_app().await;
        // 生成 2 个（这一步会顺手打开注册开关；测试末尾恢复，别影响别的用例）
        let body = serde_json::json!({ "note": "给小明", "count": 2 }).to_string();
        let res = t
            .call(
                t.req("POST", "/admin/invites")
                    .header(axum::http::header::CONTENT_TYPE, "application/json")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await;
        let j = body_json(res).await;
        let codes: Vec<String> = j["codes"]
            .as_array()
            .expect("codes")
            .iter()
            .map(|c| c.as_str().unwrap_or("").to_string())
            .collect();
        assert_eq!(codes.len(), 2, "{j}");
        assert!(codes.iter().all(|c| c.starts_with("vtrim_inv_")), "{j}");
        assert_eq!(j["registration_enabled"], true);

        // 列表里能再看到同样的明文，且都是「未使用」
        let res = t.call(t.req("GET", "/admin/invites").body(Body::empty()).unwrap()).await;
        let j = body_json(res).await;
        let rows = j["invites"].as_array().unwrap().clone();
        for c in &codes {
            let row = rows.iter().find(|r| r["token"] == c.as_str()).unwrap_or_else(|| panic!("列表应回明文: {j}"));
            assert_eq!(row["used"], false, "{row}");
            assert_eq!(row["note"], "给小明");
        }

        // 用一个码注册 → 该行变「已使用」并显示绑给了谁；另一个不受影响
        let bundle = t
            .auth
            .register("xiaoming", "pass-12345678", &codes[0], Some("10.2.2.2"), None)
            .expect("有效码应能注册");
        let res = t.call(t.req("GET", "/admin/invites").body(Body::empty()).unwrap()).await;
        let j = body_json(res).await;
        let rows = j["invites"].as_array().unwrap();
        let used = rows.iter().find(|r| r["token"] == codes[0].as_str()).unwrap();
        assert_eq!(used["used"], true, "{j}");
        assert_eq!(used["used_by"], bundle.user.id);
        assert_eq!(used["used_by_name"], "xiaoming");
        assert_eq!(used["open"], false, "用过的码不再是「可用」");
        let other = rows.iter().find(|r| r["token"] == codes[1].as_str()).unwrap();
        assert_eq!(other["used"], false, "另一个码不受影响");

        // 撤销后列表里也看得出来
        let id = used["id"].as_i64().unwrap();
        let res = t.call(t.req("DELETE", &format!("/admin/invites/{id}")).body(Body::empty()).unwrap()).await;
        assert_eq!(res.status(), StatusCode::OK);
        let res = t.call(t.req("GET", "/admin/invites").body(Body::empty()).unwrap()).await;
        let j = body_json(res).await;
        let row = j["invites"].as_array().unwrap().iter().find(|r| r["id"] == id).cloned().unwrap();
        assert_eq!(row["revoked"], true, "{j}");

        // 恢复注册开关：不给后续用例留下「已开启」状态
        vtrim_service::invite::set_registration_enabled(false).unwrap();
    }

    /// 项目配置里的路径键必须落在本项目内（非 admin）。
    ///
    /// 回归钉子：`config_vtrim_put_impl` 以前只校验"是合法 TOML"，于是普通用户能把
    /// `input_file` 改成别人的录像（转写完把字幕读出来）或把 `output_dir` 指到任意目录。
    #[test]
    fn project_config_paths_are_jailed_for_non_admin() {
        let root = std::env::temp_dir().join(format!("vtrim-cfg-paths-{}", std::process::id()));
        std::fs::create_dir_all(root.join("output")).unwrap();
        let ws = Workspace { config_file: root.join("vtrim.toml"), root_dir: root.clone() };

        // 正常形态放行：相对路径、空串、缺键
        let ok = serde_json::json!({
            "project": { "input_file": "records/a.mp4", "danmaku_file": "" },
            "cutting": { "output_dir": "./output" }
        });
        assert!(check_project_paths(&ws, &ok, false).is_ok(), "{ok}");

        // 绝对路径 → 400（读别人录像 / 写到项目外）
        let abs_in = serde_json::json!({ "project": { "input_file": "C:/other/clip.mp4" } });
        assert_eq!(
            check_project_paths(&ws, &abs_in, false).unwrap_err().0,
            StatusCode::BAD_REQUEST
        );
        let abs_out = serde_json::json!({ "cutting": { "output_dir": "D:/tmp/out" } });
        assert_eq!(
            check_project_paths(&ws, &abs_out, false).unwrap_err().0,
            StatusCode::BAD_REQUEST
        );
        // `..` 越界 → 400
        let up = serde_json::json!({ "project": { "audio_file": "../../secret.wav" } });
        assert_eq!(
            check_project_paths(&ws, &up, false).unwrap_err().0,
            StatusCode::BAD_REQUEST
        );

        // admin 不受限：「服务器文件」来源建的项目里 input_file 本来就是项目外的绝对路径
        assert!(check_project_paths(&ws, &abs_in, true).is_ok());
        assert!(check_project_paths(&ws, &abs_out, true).is_ok());

        let _ = std::fs::remove_dir_all(&root);
    }
}
