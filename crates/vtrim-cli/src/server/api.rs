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
use crate::service::Workspace;

use super::model::AppState;
use super::scope::Scope;
use super::{jobs::JobEvent, media, persist};

pub fn router(state: Arc<AppState>) -> Router {
    // 白名单（§10.1 step-2.1）：登录端点与 /health 之外，**所有**接口都要会话。
    let public = Router::new()
        .route("/health", get(health))
        .route("/auth/login", post(auth_login))
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
        .route("/admin/ingest", get(super::ingest::config_view))
        .route("/admin/ingest/assign", post(super::ingest::assign))
        .route("/admin/ingest/scan", post(super::ingest::scan_now))
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
        .route("/polish/render-frame", post(polish_render_frame))
        .route("/polish/font", get(polish_font))
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
        .route("/project-data/clips/:rank/thumb", get(project_data_clip_thumb))
        .route("/project-data/events/cut-all", post(project_data_event_cut_all))
        .route("/project-data/events/:rank/cut", post(project_data_event_cut))
        .route("/project-data/clips/:rank/silence", post(project_data_clip_silence))
        .route("/project-data/transcript", get(project_data_transcript).put(project_data_transcript_put))
        .route("/project-data/correct", post(project_data_correct))
        .route("/project-data/pipeline", post(project_data_pipeline))
        .route("/project-data/config", get(project_data_config).put(project_data_config_put))
        .route("/project-data/polish", get(project_data_polish))
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
async fn health(State(state): State<Arc<AppState>>, headers: axum::http::HeaderMap) -> ApiResult {
    // 能力位只看平台级配置（global-only）：health 是公开接口，不该泄露任何工程信息
    let cfg = vtrim_core::config::load_global_config();
    let llm = !cfg.llm.api_key.is_empty() && cfg.llm.api_key != "sk-your-api-key-here";
    let db = state.auth.db();
    let accounts = db.count_users().unwrap_or(0);
    let must_change_password = db
        .find_user("admin")
        .ok()
        .flatten()
        .map(|u| u.must_change_password)
        .unwrap_or(false);
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
        "accounts": accounts,
        "must_change_password": must_change_password,
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
    let models = ["sensevoice", "firered-zh", "qwen3-asr"];
    let model_status: Vec<_> = models.iter().map(|m| {
        serde_json::json!({ "name": m, "exists": models_dir.join(m).exists() })
    }).collect();
    ok(serde_json::json!({
        "ffmpeg": command_available("ffmpeg").await,
        "ffprobe": command_available("ffprobe").await,
        "models_dir": models_dir.to_string_lossy(),
        "models": model_status,
        "workspace": ws.config_file.to_string_lossy(),
    }))
}

async fn models_get() -> ApiResult {
    let home = vtrim_core::path::vtrim_home();
    let models_dir = home.join("models");
    let names = ["sensevoice", "firered-zh", "qwen3-asr"];
    let out: Vec<serde_json::Value> = names.iter().map(|name| {
        let dir = models_dir.join(name);
        serde_json::json!({
            "name": name,
            "installed": dir.exists(),
            "size_label": match *name { "sensevoice"=>"165 MB", "firered-zh"=>"520 MB", _=>"879 MB" },
            "note": match *name { "sensevoice"=>"快", "firered-zh"=>"较准", _=>"最准最慢" },
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
        "config_file": ws.config_file.to_string_lossy(),
        "root_dir": ws.root_dir.to_string_lossy(),
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

fn config_vtrim_put_impl(ws: &Workspace, body: &serde_json::Value) -> ApiResult {
    // patch 模式：`{ section: { key: value } }` 合并写（保留注释与其他 section），
    // 前端设置表单用它，避免"读整份 TOML → 正则替换 → 写回"这种易碎做法。
    if let Some(patch) = body.get("patch") {
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
    crate::service::validate_toml(raw)
        .map_err(|e| err(StatusCode::BAD_REQUEST, format!("vtrim.toml 不是合法 TOML，未保存: {e}")))?;
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
    // patch / 源文件两种写法都会读改写整份配置，加锁避免并发提交互相覆盖
    vtrim_core::filelock::with_path_lock(&path, || config_global_put_locked(&path, &body)).await
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
    crate::service::validate_toml(raw)
        .map_err(|e| err(StatusCode::BAD_REQUEST, format!("config.toml 不是合法 TOML，未保存: {e}")))?;
    if let Some(parent) = path.parent() { std::fs::create_dir_all(parent).map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("创建目录失败: {e}")))?; }
    std::fs::write(path, raw).map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("保存失败: {e}")))?;
    ok(serde_json::json!({ "ok": true }))
}

/// 保存 LLM 配置：`raw` 整份覆盖写，`patch` 合并写（保留注释与其他键）。
async fn config_llm_put(State(_state): State<Arc<AppState>>, Json(body): Json<serde_json::Value>) -> ApiResult {
    let path = vtrim_core::path::vtrim_home().join("llm.toml");
    vtrim_core::filelock::with_path_lock(&path, || config_llm_put_locked(&path, &body)).await
}

fn config_llm_put_locked(path: &Path, body: &serde_json::Value) -> ApiResult {
    if let Some(patch) = body.get("patch") {
        vtrim_core::config::patch_config_file(path, patch)
            .map_err(|e| err(StatusCode::BAD_REQUEST, format!("LLM 配置保存失败: {e:#}")))?;
        return ok(serde_json::json!({ "ok": true }));
    }
    let raw = body.get("raw").and_then(|v| v.as_str()).or_else(|| body.get("content").and_then(|v| v.as_str()));
    let Some(raw) = raw else { return Err(err(StatusCode::BAD_REQUEST, "缺少 raw 或 patch 字段")); };
    crate::service::validate_toml(raw)
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
    let id = match state.jobs.create(user.id, "download_models").await {
        Ok(id) => id,
        Err(busy) => return Err(err(StatusCode::CONFLICT, busy.to_string())),
    };
    state.jobs.mark_running(&id, "下载 ASR 模型").await;
    let jobs = state.jobs.clone();
    let job_id = id.clone();
    tokio::spawn(async move {
        let result = vtrim_core::download::download_all(None).await;
        if let Err(e) = result {
            jobs.fail(&job_id, format!("{e:#}")).await;
            return;
        }
        jobs.complete(&job_id).await;
    });
    ok(serde_json::json!({ "ok": true, "id": id }))
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
    crate::service::project_summary(ws)
}

// ---- 项目托管（P2，§5 / §8） ----

/// 取项目并校验归属；他人项目一律 404（不泄露"存在但无权限"）。
fn owned_project(
    db: &crate::service::db::Db,
    user: &super::auth::CurrentUser,
    id: i64,
) -> Result<crate::service::db::ProjectRow, AppError> {
    match crate::service::projects::find_owned(db, user.id, user.is_admin(), id) {
        Ok(Some(p)) => Ok(p),
        Ok(None) => Err(err(StatusCode::NOT_FOUND, "项目不存在或无权访问")),
        Err(e) => Err(err(StatusCode::INTERNAL_SERVER_ERROR, format!("读取项目失败: {e:#}"))),
    }
}

fn owner_name(db: &crate::service::db::Db, owner_id: i64) -> Option<String> {
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

    let out: Vec<serde_json::Value> = rows
        .iter()
        .map(|p| {
            let json = crate::service::projects::summary_json(p, owner_name(db, p.owner_id).as_deref());
            // 项目目录/vtrim.toml 被外部删掉时落库标记，列表里显示"缺失"而不是静默消失
            if json["missing"].as_bool() == Some(true) && !p.missing {
                let _ = db.set_project_missing(p.id, true);
            }
            json
        })
        .collect();
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
        source.get("dir").and_then(|v| v.as_str()).unwrap_or(""),
        source.get("input").and_then(|v| v.as_str()).unwrap_or(""),
        source.get("danmaku").and_then(|v| v.as_str()),
        source.get("output_dir").and_then(|v| v.as_str()),
    )
    .await
}

/// 用"服务器本机文件"建项目的公共实现（`POST /api/projects` 与过渡期的
/// `POST /api/project/init` 共用）。所有路径都囚笼在**租户根**之内。
async fn create_local_project(
    state: &Arc<AppState>,
    user: &super::auth::CurrentUser,
    rel_dir: &str,
    input_hint: &str,
    danmaku_hint: Option<&str>,
    output_dir: Option<&str>,
) -> ApiResult {
    let root = crate::service::projects::tenant_root(user.id);
    std::fs::create_dir_all(&root)
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("创建租户目录失败: {e}")))?;
    let rel_dir = rel_dir.trim();
    let dir = if rel_dir.is_empty() {
        root.clone()
    } else {
        vtrim_core::tenant::jail_existing(&root, rel_dir)
            .map_err(|e| err(StatusCode::BAD_REQUEST, format!("目录不可用: {e:#}")))?
    };
    if !dir.is_dir() {
        return Err(err(StatusCode::BAD_REQUEST, "只能选择目录"));
    }
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

    let config_path = dir.join("vtrim.toml");
    let output_rel = output_dir.map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
    if let Some(rel) = &output_rel {
        vtrim_core::tenant::jail_join(&dir, rel)
            .map_err(|e| err(StatusCode::BAD_REQUEST, format!("输出目录不可用: {e:#}")))?;
    }
    vtrim_core::config::write_project_config(&config_path, &input, danmaku.as_deref(), output_rel.as_deref())
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("初始化失败: {e:#}")))?;
    let ws = Workspace::load(Some(&config_path))
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("加载失败: {e:#}")))?;

    let db = state.auth.db();
    let name = dir
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "project".to_string());
    let label = dir
        .strip_prefix(crate::service::projects::tenant_root(user.id))
        .ok()
        .map(|rel| format!("本机文件 · {}", rel.display()))
        .unwrap_or_else(|| "本机文件".to_string());
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
        let _ = crate::service::projects::refresh_bytes(db, &p);
    }
    let _ = db.audit(Some(user.id), "project_create", &format!("id={id} kind=local"));
    activate_workspace(state, &ws).await;
    match db.project_by_id(id) {
        Ok(Some(p)) => ok(serde_json::json!({
            "ok": true,
            "id": id,
            "project": crate::service::projects::summary_json(&p, Some(&user.username)),
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

/// `DELETE /api/projects/:id`：**只注销项目**（删 `vtrim.toml`），产物与源文件都保留。
async fn projects_delete(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<super::auth::CurrentUser>,
    AxumPath(id): AxumPath<String>,
) -> ApiResult {
    let id = crate::service::projects::parse_project_id(&id)
        .ok_or_else(|| err(StatusCode::BAD_REQUEST, "项目 id 无效"))?;
    let db = state.auth.db();
    let project = owned_project(db, &user, id)?;
    if project.config_file.exists() {
        std::fs::remove_file(&project.config_file)
            .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("删除失败: {e}")))?;
    }
    db.delete_project(id)
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("注销失败: {e:#}")))?;
    // 旧列表里可能还有它：一并清掉，避免下次启动又被导入
    persist::forget_project(&project.config_file);
    let _ = db.audit(Some(user.id), "project_delete", &format!("id={id}"));
    ok(serde_json::json!({ "ok": true, "deleted": id }))
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
    let id = crate::service::projects::parse_project_id(&id)
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
    let (source_bytes, output_bytes) = crate::service::projects::refresh_bytes(db, &project)
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

/// **过渡期别名**：旧前端按 `config_path` 删项目；只允许删除**已登记且属于自己**的
/// `vtrim.toml`（P2 之后前端改用 `DELETE /api/projects/:id`）。
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
        return Err(err(StatusCode::BAD_REQUEST, "只允许删除 vtrim.toml"));
    }
    let db = state.auth.db();
    let project = db
        .project_by_config(&path)
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("读取项目失败: {e:#}")))?
        .filter(|p| user.is_admin() || p.owner_id == user.id)
        .ok_or_else(|| err(StatusCode::NOT_FOUND, "项目不存在或无权访问"))?;
    if project.config_file.exists() {
        std::fs::remove_file(&project.config_file)
            .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("删除失败: {e}")))?;
    }
    db.delete_project(project.id)
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("注销失败: {e:#}")))?;
    persist::forget_project(&project.config_file);
    let _ = db.audit(Some(user.id), "project_delete", &format!("id={}", project.id));
    ok(serde_json::json!({ "ok": true, "deleted": project.id }))
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
    let out: Vec<serde_json::Value> = rows
        .iter()
        .map(|p| crate::service::projects::summary_json(p, Some(&user.username)))
        .collect();
    ok(serde_json::json!({
        "workspace_root": root.to_string_lossy(),
        "projects": out,
        "deprecated": "请改用 GET /api/projects（本项目列表只回本人项目）",
    }))
}







fn project_snapshot(ws: &Workspace) -> serde_json::Value {
    crate::service::project_snapshot(ws)
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
    let tenant = crate::service::projects::tenant_root(user.id);
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
    let created = create_local_project(&state, &user, &rel_dir, "", danmaku, output).await?;
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
        (Some(id), _) => crate::service::projects::find_owned(state.auth.db(), user.id, user.is_admin(), id)
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

async fn project_data_status(scope: Scope) -> ApiResult {
    project_status_impl(&scope.ws)
}

async fn project_data_events(scope: Scope) -> ApiResult {
    data_events_impl(&scope.ws)
}

async fn project_data_clips(scope: Scope) -> ApiResult {
    data_clips_impl(&scope.ws)
}

async fn project_data_clip_thumb(scope: Scope, AxumPath(rank): AxumPath<String>) -> Response {
    let ws = scope.ws;
    // rank 直接来自 URL：必须是数字，否则会拼进路径（`clip-../../x`）
    let Ok(rank) = rank.parse::<u32>() else {
        return media::json_error(StatusCode::BAD_REQUEST, "rank 无效");
    };
    let clip_dir = ws.output_dir().join("clips").join(format!("clip-{:02}", rank));
    let video = clip_dir.join("clip.mp4");
    let thumb = clip_dir.join("_thumb.jpg");
    if !thumb.exists() && video.exists() {
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
    polish_projects_impl(&scope.ws)
}

async fn project_data_transcript(scope: Scope) -> ApiResult {
    data_transcript_impl(&scope.ws)
}

async fn project_data_transcript_put(scope: Scope, Json(body): Json<serde_json::Value>) -> ApiResult {
    let ws = scope.ws;
    let path = ws.output_dir().join("transcript.json");
    vtrim_core::filelock::with_path_lock(&path, || data_transcript_put_impl(&ws, &body)).await
}

async fn project_data_events_create(scope: Scope, Json(body): Json<serde_json::Value>) -> ApiResult {
    let ws = scope.ws;
    // events.json 是 read-modify-write：并发 PUT/POST 会丢事件
    let path = ws.output_dir().join("events.json");
    vtrim_core::filelock::with_path_lock(&path, || data_events_create_impl(&ws, &body)).await
}

async fn project_data_events_put(scope: Scope, Json(body): Json<serde_json::Value>) -> ApiResult {
    let ws = scope.ws;
    let path = ws.output_dir().join("events.json");
    vtrim_core::filelock::with_path_lock(&path, || data_events_put_impl(&ws, &body)).await
}

async fn project_data_event_cut(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<super::auth::CurrentUser>,
    scope: Scope,
    AxumPath(rank): AxumPath<String>,
) -> ApiResult {
    event_cut_impl(&scope.ws, &rank, &state.jobs, user.id, scope.project.id).await
}

async fn project_data_event_cut_all(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<super::auth::CurrentUser>,
    scope: Scope,
    Json(body): Json<serde_json::Value>,
) -> ApiResult {
    event_cut_all_impl(&scope.ws, &body, &state.jobs, user.id, scope.project.id).await
}

async fn project_data_clip_silence(scope: Scope, AxumPath(rank): AxumPath<String>) -> ApiResult {
    clip_silence_impl(&scope.ws, &rank).await
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
    config_get_impl(&scope.ws)
}

async fn project_data_config_put(scope: Scope, Json(body): Json<serde_json::Value>) -> ApiResult {
    let ws = scope.ws;
    // 读改写加锁：设置页可能同时提交源文件与其它字段
    let path = ws.config_file.clone();
    vtrim_core::filelock::with_path_lock(&path, || config_vtrim_put_impl(&ws, &body)).await
}

fn data_transcript_impl(ws: &Workspace) -> ApiResult {
    let cfg = ws.load_config().map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}")))?;
    let output = ws.output_dir();
    let path = cfg.project.transcript_file.as_ref().filter(|s| !s.is_empty()).map(PathBuf::from).filter(|p| p.exists())
        .or_else(|| {
            let j = output.join("transcript.json");
            if j.exists() { Some(j) } else { let s = output.join("video.srt"); if s.exists() { Some(s) } else { None } }
        });
    match path {
        Some(p) => {
            let raw = std::fs::read_to_string(&p).map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("读取失败: {e}")))?;
            let ext = p.extension().and_then(|e| e.to_str()).unwrap_or("");
            if ext == "json" {
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

fn data_transcript_put_impl(ws: &Workspace, body: &serde_json::Value) -> ApiResult {
    let output = ws.output_dir();
    let json_path = output.join("transcript.json");
    let srt_path = output.join("video.srt");
    let segments = body.get("segments").cloned().ok_or_else(|| err(StatusCode::BAD_REQUEST, "缺少 segments"))?;
    let segs: Vec<vtrim_core::types::TranscriptSegment> = serde_json::from_value(segments).map_err(|e| err(StatusCode::BAD_REQUEST, format!("segments 解析失败: {e}")))?;
    let transcript = vtrim_core::types::Transcript { segments: segs, language: body.get("language").and_then(|l| l.as_str()).unwrap_or("zh").to_string(), asr_metadata: None };
    if !json_path.exists() && !srt_path.exists() { return Err(err(StatusCode::NOT_FOUND, "暂无转写文件")); }
    std::fs::write(&json_path, serde_json::to_string_pretty(&transcript).map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("序列化失败: {e}")))?).map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("写入失败: {e}")))?;
    vtrim_core::srt::write_srt(&transcript, &srt_path).map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("写 SRT 失败: {e}")))?;
    ok(serde_json::json!({ "ok": true, "saved": transcript.segments.len() }))
}

/// 统计 SRT 文件中的字幕条数（以时间轴行 `-->` 为准）；文件缺失/编码异常时为 0。
fn count_srt_cues(path: &Path) -> u64 {
    match std::fs::read(path) {
        Ok(bytes) => String::from_utf8_lossy(&bytes).lines().filter(|l| l.contains("-->")).count() as u64,
        Err(_) => 0,
    }
}

/// 读取项目转写文件（transcript.json / video.srt）中所有字幕段的起始秒，升序。
fn transcript_starts(ws: &Workspace) -> Vec<f64> {
    let Ok(cfg) = ws.load_config() else { return Vec::new() };
    let output = ws.output_dir();
    let path = cfg.project.transcript_file.as_ref().filter(|s| !s.is_empty())
        .map(|s| ws.resolve(s)).filter(|p| p.exists())
        .or_else(|| {
            let j = output.join("transcript.json");
            if j.exists() { Some(j) } else { let s = output.join("video.srt"); if s.exists() { Some(s) } else { None } }
        });
    let Some(p) = path else { return Vec::new() };
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

/// 读取项目弹幕文件（ASS/SRT）中所有弹幕的起始秒，升序。
fn danmaku_starts(ws: &Workspace) -> Vec<f64> {
    let Ok(cfg) = ws.load_config() else { return Vec::new() };
    let path = cfg.project.danmaku_file.as_ref().filter(|s| !s.is_empty())
        .map(|s| ws.resolve(s)).filter(|p| p.exists());
    let Some(p) = path else { return Vec::new() };
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

fn data_clips_impl(ws: &Workspace) -> ApiResult {
    let clips = ws.output_dir().join("clips");
    if !clips.exists() { return Err(err(StatusCode::NOT_FOUND, "暂无 clips")); }
    let mut out = vec![];
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
        // 切片自己的弹幕/字幕条数（排序用）
        let subtitle_count = count_srt_cues(&dir_path.join("clip.srt"));
        let danmaku_count = count_srt_cues(&dir_path.join("clip.dm.srt"));
        out.push(serde_json::json!({
            "dir": dir_path.to_string_lossy(),
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
    out.sort_by(|a,b| a["name"].as_str().unwrap_or("").cmp(b["name"].as_str().unwrap_or("")));
    ok(serde_json::json!(out))
}

/// 「消音版」：把该切片的长静音段写入屏蔽轨道（polish.toml 的 mute_ranges）。
/// 不生成任何新文件，原始 clip.mp4 / clip.srt 保持不变。
async fn clip_silence_impl(ws: &Workspace, rank: &str) -> ApiResult {
    let clip_dir = ws.output_dir().join("clips").join(format!("clip-{:0>2}", rank));
    let video = clip_dir.join("clip.mp4");
    if !video.exists() { return Err(err(StatusCode::NOT_FOUND, "clip.mp4 不存在")); }
    ensure_polish_project(&clip_dir).await?;
    let applied = vtrim_polish::project::apply_silence(&clip_dir)
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("写入屏蔽轨道失败: {e:#}")))?;
    ok(serde_json::json!({
        "ok": true,
        "added": applied.added,
        "total": applied.total,
        "ranges": applied.ranges.iter().map(|(s, e)| serde_json::json!({ "start": s, "end": e })).collect::<Vec<_>>(),
    }))
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

    let id = match jobs.create(owner, "cut_all").await {
        Ok(id) => id,
        Err(busy) => return Err(err(StatusCode::CONFLICT, busy.to_string())),
    };
    jobs.attach_project(&id, project).await;
    let total = events.len() as u64;
    jobs.mark_running(&id, format!("一键全切 {} 个事件", total)).await;
    jobs.progress(&id, 0, total).await;
    let jobs_handle = jobs.clone();
    let cfg_file = ws.config_file.clone();
    let job_id = id.clone();
    let cancel = jobs.cancel_token(&id).await;
    tokio::spawn(async move {
        // 一次全切只产生一个 cut_all 任务；内部最多 3 个 ffmpeg 并发，进度统一写回该任务。
        let sem = Arc::new(tokio::sync::Semaphore::new(3));
        let counter = Arc::new(std::sync::atomic::AtomicU64::new(0));
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
            let jobs = jobs_handle.clone();
            let cfg_file = cfg_file.clone();
            let job_id = job_id.clone();
            let counter = counter.clone();
            handles.push(tokio::spawn(async move {
                let _permit = permit;
                let rank = ev.get("rank").and_then(|r| r.as_u64()).unwrap_or(0);
                let result = run_single_cut(&cfg_file, ev).await;
                let done = counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
                jobs.progress(&job_id, done, total).await;
                if let Err(e) = result {
                    let _ = jobs.log(&job_id, "error", format!("clip-{:02} 剪切失败: {e:#}", rank)).await;
                    Err(e)
                } else {
                    Ok(())
                }
            }));
        }
        let mut failures = 0usize;
        for handle in handles {
            match handle.await {
                Ok(Ok(())) => {}
                Ok(Err(_)) => failures += 1,
                Err(_) => failures += 1,
            }
        }
        if failures == 0 {
            jobs_handle.complete(&job_id).await;
        } else {
            jobs_handle.fail(&job_id, format!("{} 个事件剪切失败", failures)).await;
        }
    });
    ok(serde_json::json!({ "ok": true, "id": id }))
}

async fn run_single_cut(config_file: &Path, ev: serde_json::Value) -> anyhow::Result<()> {
    let cfg = vtrim_core::config::load_config_at(config_file)?;
    let w = crate::service::Workspace::load(Some(config_file))?;
    let input = cfg.project.input_file.as_ref().filter(|s| !s.is_empty()).map(|s| w.resolve(s))
        .ok_or_else(|| anyhow::anyhow!("未初始化 input_file"))?;
    let output = w.output_dir();
    let mut ctx = vtrim_core::types::PipelineContext::new(input, output);
    ctx.events = vec![serde_json::from_value(ev)?];
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
    let id = match jobs.create(owner, "cut_one").await {
        Ok(id) => id,
        Err(busy) => return Err(err(StatusCode::CONFLICT, busy.to_string())),
    };
    jobs.attach_project(&id, project).await;
    jobs.mark_running(&id, format!("剪切事件 #{}", rank)).await;
    let jobs_handle = jobs.clone(); let cfg_file = ws.config_file.clone(); let job_id=id.clone();
    tokio::spawn(async move {
        let result = (async || -> anyhow::Result<()> {
            let cfg = vtrim_core::config::load_config_at(&cfg_file)?;
            let w = crate::service::Workspace::load(Some(&cfg_file))?;
            let input = cfg.project.input_file.as_ref().filter(|s| !s.is_empty()).map(|s| w.resolve(s)).ok_or_else(|| anyhow::anyhow!("未初始化 input_file"))?;
            let output = w.output_dir();
            let mut ctx = vtrim_core::types::PipelineContext::new(input, output);
            ctx.events = vec![ev];
            let mut r = vtrim_core::pipeline::PipelineRunner::new(cfg);
            r.add_stage(Box::new(vtrim_core::cutting::Cutter));
            r.run(&mut ctx).await
        })().await;
        match result { Ok(_)=>jobs_handle.complete(&job_id).await, Err(e)=>jobs_handle.fail(&job_id, format!("{e:#}")).await }
    });
    ok(serde_json::json!({ "ok": true, "id": id }))
}

/// `POST /api/fs/list {path}`：**仅 admin**，且囚笼在该账号的租户根内（§0.2.7 / §5.2）。
///
/// 保留这个入口只为"本机部署用服务器上已有文件建项目"这条 admin-only 来源；
/// 它**不接受绝对路径**，只接受相对租户根（`<projects_root>/users/<uid>`）的路径，
/// 也**永不接受上传**。响应同时给出相对路径，前端据此拼 `POST /api/projects`。
async fn fs_list(
    Extension(user): Extension<super::auth::CurrentUser>,
    Json(body): Json<serde_json::Value>,
) -> ApiResult {
    if !user.is_admin() {
        return Err(err(StatusCode::FORBIDDEN, "仅管理员可浏览服务器文件"));
    }
    let rel = body
        .get("path")
        .or_else(|| body.get("dir"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    let root = crate::service::projects::tenant_root(user.id);
    std::fs::create_dir_all(&root)
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("创建租户目录失败: {e}")))?;
    let target = if rel.is_empty() {
        root.clone()
    } else {
        vtrim_core::tenant::jail_existing(&root, &rel)
            .map_err(|e| err(StatusCode::BAD_REQUEST, format!("路径不可用: {e:#}")))?
    };
    if !target.is_dir() {
        return Err(err(StatusCode::BAD_REQUEST, "只能浏览目录"));
    }
    let tenant = root.canonicalize().unwrap_or_else(|_| root.clone());
    let rel_prefix = target
        .canonicalize()
        .unwrap_or_else(|_| target.clone())
        .strip_prefix(&tenant)
        .map(|p| p.to_string_lossy().replace('\\', "/"))
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
    let parent = Path::new(&rel_prefix)
        .parent()
        .map(|p| p.to_string_lossy().replace('\\', "/"))
        .unwrap_or_default();
    ok(serde_json::json!({
        "path": rel_prefix,
        "parent": parent,
        "root": crate::service::projects::projects_root().to_string_lossy(),
        "dirs": dirs,
        "files": files,
    }))
}

/// 解析 `Range` 请求头（只支持单区间）。
///
/// - `None`：没有 / 无法解析（多区间、非 bytes 单位、数字非法）→ 调用方按 200 全量返回
/// - `Some(None)`：语法合法但不可满足（`start >= total`、`end < start`、`bytes=-0`）→ 416
/// - `Some(Some((start, end)))`：闭区间，已按 `total` 收敛
///
/// 旧实现在 handler 里直接 `end - start_raw + 1`：`bytes=500-100` 会整数下溢
/// （debug 下 panic；release 下算出天文数字的 Content-Length，客户端挂死）。
fn parse_byte_range(header: Option<&str>, total: u64) -> Option<Option<(u64, u64)>> {
    let spec = header?.trim().strip_prefix("bytes=")?.trim();
    if spec.contains(',') {
        return None;
    }
    let (start_s, end_s) = spec.split_once('-')?;
    let (start_s, end_s) = (start_s.trim(), end_s.trim());
    if total == 0 {
        return Some(None);
    }
    if start_s.is_empty() {
        // bytes=-N：最后 N 个字节
        let n: u64 = end_s.parse().ok()?;
        if n == 0 {
            return Some(None);
        }
        return Some(Some((total.saturating_sub(n), total - 1)));
    }
    let start: u64 = start_s.parse().ok()?;
    if start >= total {
        return Some(None);
    }
    let end = if end_s.is_empty() { total - 1 } else { end_s.parse::<u64>().ok()? };
    if end < start {
        return Some(None);
    }
    Some(Some((start, end.min(total - 1))))
}

async fn media_get(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<super::auth::CurrentUser>,
    AxumPath(path): AxumPath<String>,
    headers: axum::http::HeaderMap,
) -> Response {
    use tokio::io::{AsyncReadExt, AsyncSeekExt};

    // 只服务本人项目内的文件：
    // - 新契约 `/api/media/<project_id>/<项目内相对路径>`
    // - 过渡期 `/api/media/<绝对路径>`：必须落在自己的某个项目根内
    let db = state.auth.db();
    let (project, rel) = media::split_project_media(&path);
    let path = match project {
        Some(id) => {
            let proj = match super::scope::find_owned_project(db, &user, id) {
                Ok(Some(p)) => p,
                Ok(None) => return media::json_error(StatusCode::NOT_FOUND, "项目不存在或无权访问"),
                Err(e) => return media::json_error(StatusCode::INTERNAL_SERVER_ERROR, &format!("{e:#}")),
            };
            match vtrim_core::tenant::jail_existing(&proj.root_dir, &rel) {
                Ok(p) => p,
                Err(e) => return media::json_error(StatusCode::NOT_FOUND, &format!("文件不可用: {e:#}")),
            }
        }
        None => {
            let abs = if Path::new(&rel).is_absolute() {
                PathBuf::from(&rel)
            } else {
                // 相对路径必须配合项目 id（新契约）；这里用当前工作区解析后仍要过归属校验
                state.workspace().await.resolve(&rel)
            };
            match super::scope::find_project_for_dir(db, &user, &abs) {
                Ok(Some(proj)) => {
                    let root = proj.root_dir.canonicalize().unwrap_or_else(|_| proj.root_dir.clone());
                    let real = abs.canonicalize().unwrap_or_else(|_| abs.clone());
                    if !real.starts_with(&root) {
                        return media::json_error(StatusCode::FORBIDDEN, "文件不在你的项目内");
                    }
                    real
                }
                Ok(None) => {
                    return media::json_error(
                        StatusCode::FORBIDDEN,
                        "文件不在你的任何项目内（新契约：/api/media/<项目id>/<相对路径>）",
                    )
                }
                Err(e) => return e.into_response(),
            }
        }
    };
    if !path.exists() {
        return media::json_error(StatusCode::NOT_FOUND, "文件不存在");
    }
    let meta = match std::fs::metadata(&path) {
        Ok(m) => m,
        Err(_) => return StatusCode::NOT_FOUND.into_response(),
    };
    let total = meta.len();
    let mime = match path.extension().and_then(|e| e.to_str()).map(|e| e.to_ascii_lowercase()).as_deref() {
        Some("mp4" | "m4v") => "video/mp4".to_string(),
        Some("webm") => "video/webm".to_string(),
        Some("ogv") => "video/ogg".to_string(),
        Some("mov") => "video/quicktime".to_string(),
        _ => mime_guess::from_path(&path).first_or_octet_stream().to_string(),
    };
    let range_header = headers.get(axum::http::header::RANGE).and_then(|v| v.to_str().ok());
    let range = parse_byte_range(range_header, total);

    fn stream_file(
        file: tokio::fs::File,
        remaining: Option<u64>,
    ) -> impl futures_util::Stream<Item = Result<axum::body::Bytes, std::io::Error>> + Send {
        futures_util::stream::unfold((file, remaining), |(mut f, mut rem)| async move {
            let mut buf = vec![0u8; 64 * 1024];
            let want = match rem {
                Some(0) => return None,
                Some(r) => r.min(buf.len() as u64) as usize,
                None => buf.len(),
            };
            let n = match f.read(&mut buf[..want]).await {
                Ok(n) => n,
                Err(e) => return Some((Err(e), (f, rem))),
            };
            if n == 0 {
                return None;
            }
            buf.truncate(n);
            rem = rem.map(|r| r - n as u64);
            Some((Ok(axum::body::Bytes::from(buf)), (f, rem)))
        })
    }

    match range {
        // 语法合法但不可满足（start >= total / end < start / 后缀长度为 0）→ 416
        Some(None) => {
            return (StatusCode::RANGE_NOT_SATISFIABLE, [(axum::http::header::CONTENT_RANGE, format!("bytes */{}", total))]).into_response();
        }
        Some(Some((start_raw, end))) => {
            let len = end - start_raw + 1;
            let mut file = match tokio::fs::File::open(&path).await {
                Ok(f) => f,
                Err(_) => return StatusCode::NOT_FOUND.into_response(),
            };
            if file.seek(std::io::SeekFrom::Start(start_raw)).await.is_err() {
                return StatusCode::INTERNAL_SERVER_ERROR.into_response();
            }
            let body = axum::body::Body::from_stream(stream_file(file, Some(len)));
            return Response::builder()
                .status(StatusCode::PARTIAL_CONTENT)
                .header(axum::http::header::CONTENT_TYPE, mime)
                .header(axum::http::header::CONTENT_LENGTH, len)
                .header(axum::http::header::CONTENT_RANGE, format!("bytes {}-{}/{}", start_raw, end, total))
                .header(axum::http::header::ACCEPT_RANGES, "bytes")
                .body(body)
                .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response());
        }
        // 没有 Range 或无法解析（多区间、非 bytes 单位……）→ 200 全量
        None => {}
    }

    let file = match tokio::fs::File::open(&path).await {
        Ok(f) => f,
        Err(_) => return StatusCode::NOT_FOUND.into_response(),
    };
    let body = axum::body::Body::from_stream(stream_file(file, None));
    Response::builder()
        .header(axum::http::header::CONTENT_TYPE, mime)
        .header(axum::http::header::CONTENT_LENGTH, total)
        .header(axum::http::header::ACCEPT_RANGES, "bytes")
        .body(body)
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
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
    let id = match jobs.create(owner, kind).await {
        Ok(id) => id,
        Err(busy) => return Err(err(StatusCode::CONFLICT, busy.to_string())),
    };
    jobs.attach_project(&id, project).await;
    jobs.mark_running(&id, "准备中").await;
    let jobs = jobs.clone();
    let run_stages = stages.clone();
    let job_id = id.clone();
    let cancel = jobs.cancel_token(&id).await;
    tokio::spawn(async move {
        let result = crate::server::api::core::run_stages(config_file, run_stages, dry_run, resume, job_id.clone(), jobs.clone(), cancel).await;
        if let Err(e) = result {
            jobs.fail(&job_id, format!("{e:#}")).await;
            return;
        }
        jobs.complete(&job_id).await;
    });
    ok(serde_json::json!({ "ok": true, "id": id }))
}

async fn correct_run_impl(
    ws: &Workspace,
    jobs: &Arc<super::jobs::JobManager>,
    owner: i64,
    project: i64,
) -> ApiResult {
    let id = match jobs.create(owner, "correct").await {
        Ok(id) => id,
        Err(busy) => return Err(err(StatusCode::CONFLICT, busy.to_string())),
    };
    jobs.attach_project(&id, project).await;
    jobs.mark_running(&id, "LLM 订正运行中").await;
    let jobs_handle = jobs.clone();
    let cfg_file = ws.config_file.clone();
    let job_id = id.clone();
    tokio::spawn(async move {
        let result = crate::service::run_correct(&cfg_file, None, true).await;
        if let Err(e) = result {
            jobs_handle.fail(&job_id, format!("{e:#}")).await;
        } else {
            jobs_handle.complete(&job_id).await;
        }
    });
    ok(serde_json::json!({ "ok": true, "id": id }))
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
                "dir": dir.to_string_lossy(),
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
) -> ApiResult {
    // dir 必须先校验：`dir: ""` 会让 cmd_init 在 server 进程的 CWD 里写出
    // polish.toml / data / sfx，任意绝对路径也会被当作合法工程目录。
    // 归属由 `scope::enforce_scope` 中间件保证（只能是自己项目内的目录）。
    let path = validated_project_dir(&dir)?;
    let id = match state.jobs.create(owner, format!("polish_{}", kind)).await {
        Ok(id) => id,
        Err(busy) => return Err(err(StatusCode::CONFLICT, busy.to_string())),
    };
    if let Some(pid) = project {
        state.jobs.attach_project(&id, pid).await;
    }
    let dir_name = path
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| dir.clone());
    state.jobs.mark_running(&id, format!("polish {kind} · {dir_name}")).await;
    let jobs = state.jobs.clone();
    let job_id = id.clone();
    let kind = kind.to_string();
    tokio::spawn(async move {
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
                    // 使用外部 CLI 子进程渲染，避免在 WebUI 服务进程内启动 headless Chrome 时超时
                    let exe = std::env::current_exe().unwrap_or_else(|_| std::path::PathBuf::from("vtrim"));
                    let dir_str = path.to_string_lossy().into_owned();
                    let output = tokio::process::Command::new(&exe)
                        .args(["polish", "render", &dir_str])
                        .output()
                        .await
                        .map_err(|e| anyhow::anyhow!("渲染进程启动失败: {e}"))?;
                    if !output.status.success() {
                        let stderr = String::from_utf8_lossy(&output.stderr);
                        let stdout = String::from_utf8_lossy(&output.stdout);
                        let tail = if !stderr.trim().is_empty() {
                            stderr.lines().rev().take(20).collect::<Vec<_>>().join("\n")
                        } else {
                            stdout.lines().rev().take(20).collect::<Vec<_>>().join("\n")
                        };
                        anyhow::bail!("渲染进程退出码: {:?}\n{}", output.status.code(), tail);
                    }
                    Ok(())
                }
                "cover" => {
                    // 与 render 同理：封面生成也走外部 CLI 子进程，避免在 WebUI 服务进程内启动 headless Chrome 超时
                    let exe = std::env::current_exe().unwrap_or_else(|_| std::path::PathBuf::from("vtrim"));
                    let dir_str = path.to_string_lossy().into_owned();
                    let output = tokio::process::Command::new(&exe)
                        .args(["polish", "cover", &dir_str])
                        .output()
                        .await
                        .map_err(|e| anyhow::anyhow!("封面进程启动失败: {e}"))?;
                    if !output.status.success() {
                        let stderr = String::from_utf8_lossy(&output.stderr);
                        let stdout = String::from_utf8_lossy(&output.stdout);
                        let tail = if !stderr.trim().is_empty() {
                            stderr.lines().rev().take(20).collect::<Vec<_>>().join("
")
                        } else {
                            stdout.lines().rev().take(20).collect::<Vec<_>>().join("
")
                        };
                        anyhow::bail!("封面进程退出码: {:?}
{}", output.status.code(), tail);
                    }
                    Ok(())
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
    ok(serde_json::json!({ "ok": true, "id": id }))
}

/// polish 路由的作用域项目 id（由 `scope::enforce_scope` 注入；0 = 未解析）。
fn scope_pid(ext: Option<axum::extract::Extension<super::scope::ScopeProjectId>>) -> Option<i64> {
    ext.map(|e| e.0.0).filter(|v| *v > 0)
}

async fn polish_init(State(state): State<Arc<AppState>>, Extension(user): Extension<super::auth::CurrentUser>, pid: Option<Extension<super::scope::ScopeProjectId>>, Json(body): Json<serde_json::Value>) -> ApiResult {
    let dir = body.get("dir").and_then(|v| v.as_str()).unwrap_or("").to_string();
    polish_spawn(&state, user.id, scope_pid(pid), "init", dir).await
}

async fn polish_llm(State(state): State<Arc<AppState>>, Extension(user): Extension<super::auth::CurrentUser>, pid: Option<Extension<super::scope::ScopeProjectId>>, Json(body): Json<serde_json::Value>) -> ApiResult {
    let dir = body.get("dir").and_then(|v| v.as_str()).unwrap_or("").to_string();
    polish_spawn(&state, user.id, scope_pid(pid), "llm", dir).await
}

async fn polish_process(State(state): State<Arc<AppState>>, Extension(user): Extension<super::auth::CurrentUser>, pid: Option<Extension<super::scope::ScopeProjectId>>, Json(body): Json<serde_json::Value>) -> ApiResult {
    let dir = body.get("dir").and_then(|v| v.as_str()).unwrap_or("").to_string();
    polish_spawn(&state, user.id, scope_pid(pid), "process", dir).await
}

async fn polish_render(State(state): State<Arc<AppState>>, Extension(user): Extension<super::auth::CurrentUser>, pid: Option<Extension<super::scope::ScopeProjectId>>, Json(body): Json<serde_json::Value>) -> ApiResult {
    let dir = body.get("dir").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let path = validated_project_dir(&dir)?;
    if !path.join("polish.toml").exists() { return Err(err(StatusCode::BAD_REQUEST, "目录未初始化 polish")); }
    if let Some(output_file) = body.get("output_file").and_then(|v| v.as_str()).filter(|s| !s.is_empty()) {
        // polish.toml 读改写：与设置页 / 封面保存串行化
        let output_file = output_file.to_string();
        let dir = path.clone();
        vtrim_core::filelock::with_path_lock(&dir.join("polish.toml"), || -> Result<(), AppError> {
            let mut cfg = vtrim_polish::project::load_project(&dir).map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("加载失败: {e}")))?;
            cfg.project.output = output_file;
            let raw = cfg.to_toml_commented().map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("序列化失败: {e}")))?;
            std::fs::write(dir.join("polish.toml"), raw).map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("写入失败: {e}")))?;
            Ok(())
        })
        .await?;
    }
    polish_spawn(&state, user.id, scope_pid(pid), "render", dir).await
}

async fn polish_cover(State(state): State<Arc<AppState>>, Extension(user): Extension<super::auth::CurrentUser>, pid: Option<Extension<super::scope::ScopeProjectId>>, Json(body): Json<serde_json::Value>) -> ApiResult {
    let dir = body.get("dir").and_then(|v| v.as_str()).unwrap_or("").to_string();
    polish_spawn(&state, user.id, scope_pid(pid), "cover", dir).await
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
        .with_thinking(Some("disabled"));

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
        prompt_text.push_str("参考标题示例（来自 config.toml，只参考句式、信息密度和 emoji，不要照抄）：\n");
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
        if let Some(v) = cover.get("effects").and_then(|v| v.as_array()) {
            cfg.cover.effects = v.iter().filter_map(|e| {
                let kind = e.get("kind").and_then(|x| x.as_str())?.to_string();
                let amount = e.get("amount").and_then(|x| x.as_f64()).unwrap_or(0.3);
                let x = e.get("x").and_then(|x| x.as_f64()).unwrap_or(50.0);
                let y = e.get("y").and_then(|x| x.as_f64()).unwrap_or(50.0);
                let color = e.get("color").and_then(|x| x.as_str()).unwrap_or("#000000").to_string();
                Some(vtrim_polish::project::CoverEffect { kind, amount, x, y, color })
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
    let lock_path = path.join("polish.toml");
    vtrim_core::filelock::with_path_lock(&lock_path, || match action {
        "clear" => {
            let removed = vtrim_polish::project::clear_silence(&path)
                .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("移除静音屏蔽失败: {e:#}")))?;
            ok(serde_json::json!({ "ok": true, "applied": false, "removed": removed, "total": vtrim_polish::project::mute_range_count(&path) }))
        }
        "apply" => {
            let applied = vtrim_polish::project::apply_silence(&path)
                .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("写入屏蔽轨道失败: {e:#}")))?;
            ok(serde_json::json!({
                "ok": true,
                "applied": !applied.ranges.is_empty(),
                "added": applied.added,
                "total": applied.total,
                "ranges": applied.ranges.iter().map(|(s, e)| serde_json::json!({ "start": s, "end": e })).collect::<Vec<_>>(),
                "message": if applied.ranges.is_empty() { "未检测到长静音段" } else { "" },
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
        // 有任务在跑时不要并发跑隐式 process（会与任务同时写 polish.toml / 资产目录）
        if state.jobs.has_active().await {
            tracing::info!("有任务在运行，跳过预览前的隐式 process: {}", path.display());
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
        std::fs::write(path.join("_cover_frame.jpg"), bytes)
            .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("写入失败: {e}")))?;
        // 自定义背景后，把 cover.image 留空（使用 _cover_frame.jpg 截帧/上传帧）
        let mut cfg = vtrim_polish::project::load_project(&path)
            .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("加载失败: {e}")))?;
        cfg.cover.image = String::new();
        let raw = cfg.to_toml_commented().map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("序列化失败: {e}")))?;
        std::fs::write(path.join("polish.toml"), raw).map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("写入失败: {e}")))?;
        return ok(serde_json::json!({ "ok": true }));
    }
    if let Some(src) = body.get("source").and_then(|v| v.as_str()) {
        // 与其它 polish.toml 写入串行化
        let _guard = vtrim_core::filelock::lock_path(&path.join("polish.toml")).await;
        let sp = PathBuf::from(src);
        if !sp.exists() { return Err(err(StatusCode::BAD_REQUEST, "背景文件不存在")); }
        std::fs::copy(&sp, path.join("_cover_frame.jpg"))
            .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("复制失败: {e}")))?;
        let mut cfg = vtrim_polish::project::load_project(&path)
            .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("加载失败: {e}")))?;
        cfg.cover.image = String::new();
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
    // 与其它 polish.toml 写入串行化（本函数最后会写回 cover.image）
    let _guard = vtrim_core::filelock::lock_path(&path.join("polish.toml")).await;
    let config = vtrim_polish::project::load_project(&path)
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("加载失败: {e}")))?;
    let resolved = vtrim_polish::render::resolve_paths(&path, &config);
    let duration = config.segments.last().map(|s| s.end).unwrap_or(0.0);
    let time = body.get("time").and_then(|v| v.as_f64())
        .or_else(|| body.get("sec").and_then(|v| v.as_f64()))
        .unwrap_or_else(|| duration * config.cover.frame_pos);
    let time = time.max(0.1);
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
    ok(serde_json::json!({ "ok": true, "time": time }))
}

async fn polish_cover_preview(State(state): State<Arc<AppState>>, Json(body): Json<serde_json::Value>) -> ApiResult {
    let dir = body.get("dir").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let path = validated_project_dir(dir)?;
    if !path.join("polish.toml").exists() {
        return Err(err(StatusCode::BAD_REQUEST, "目录未初始化 polish"));
    }
    let config = vtrim_polish::project::load_project(&path).map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("加载失败: {e}")))?;
    let resolved = vtrim_polish::render::resolve_paths(&path, &config);
    let cover_image = path.join("_cover_frame.jpg");
    if !cover_image.exists() {
        let video_path = &resolved.video;
        let duration = config.segments.last().map(|s| s.end).unwrap_or(0.0);
        let frame_time = (duration * config.cover.frame_pos).max(0.1);
        let frame_out = std::process::Command::new("ffmpeg")
            .args(["-ss", &frame_time.to_string(), "-i", video_path,
                   "-vframes", "1", "-f", "image2pipe", "-vcodec", "mjpeg", "-q:v", "3", "-"])
            .stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::null())
            .output().map(|o| o.stdout);
        if let Ok(frame_data) = frame_out {
            if !frame_data.is_empty() {
                std::fs::write(&cover_image, frame_data).ok();
            }
        }
    }
    if !cover_image.exists() {
        return Err(err(StatusCode::BAD_REQUEST, "无法截取封面帧"));
    }
    // 封面服务按 dir|cover 复用；HTML 每次请求按当前 polish.toml 重建，改完设计刷新即可
    let port = preview_port_for(&state, &path, "cover")
        .await
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("封面预览启动失败: {e:#}")))?;
    let url = format!("http://127.0.0.1:{}/", port);
    ok(serde_json::json!({ "ok": true, "cover_url": url }))
}

async fn polish_subtitles_get(State(_state): State<Arc<AppState>>, Query(q): Query<std::collections::HashMap<String,String>>) -> ApiResult {
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
    let segs: Vec<vtrim_core::types::TranscriptSegment> = segments.as_array()
        .ok_or_else(|| err(StatusCode::BAD_REQUEST, "segments 必须是数组"))?
        .iter().map(|v| {
            let start = v.get("start").and_then(|x| x.as_f64()).unwrap_or(0.0);
            let end = v.get("end").and_then(|x| x.as_f64()).unwrap_or(start + 1.0);
            let text = v.get("text").and_then(|x| x.as_str()).unwrap_or("").to_string();
            let confidence = v.get("confidence").and_then(|x| x.as_f64()).unwrap_or(1.0);
            vtrim_core::types::TranscriptSegment { start, end, text, confidence }
        }).collect();
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

/// 响度曲线（带缓存）：同一源视频只测一次，结果存 data/loudness.json
fn loudness_curve_cached(project_dir: &Path, video_path: &Path) -> serde_json::Value {
    let cache_file = project_dir.join("data").join("loudness.json");
    let video_name = video_path.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    if let Ok(raw) = std::fs::read_to_string(&cache_file) {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&raw) {
            let same_video = v.get("video").and_then(|x| x.as_str()) == Some(video_name.as_str());
            if same_video {
                if let Some(points) = v.get("points") {
                    return points.clone();
                }
            }
        }
    }
    let curve = vtrim_polish::render::measure_loudness_curve(&video_path.to_string_lossy());
    if curve.is_empty() {
        return serde_json::json!([]);
    }
    let points: Vec<serde_json::Value> = curve
        .iter()
        .map(|(t, db)| serde_json::json!({ "t": t, "db": db }))
        .collect();
    let _ = std::fs::create_dir_all(project_dir.join("data"));
    let _ = std::fs::write(
        &cache_file,
        serde_json::json!({ "video": video_name, "points": points }).to_string(),
    );
    serde_json::json!(points)
}

async fn polish_project_get(State(_state): State<Arc<AppState>>, Query(q): Query<std::collections::HashMap<String,String>>) -> ApiResult {
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
            }
            candidates.push(Path::new("C:/Windows/Fonts").join(&base).with_extension("ttf"));
        }
    }
    candidates.push(std::path::Path::new("C:/Windows/Fonts/msyh.ttc").to_path_buf());
    // 发行包把内置兜底字体放在可执行文件同级 resources/ 下；开发时走仓库内的相对路径
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            candidates.push(dir.join("resources").join("lemi-bobo.ttf"));
        }
    }
    candidates.push(Path::new("crates/vtrim-core/resources/lemi-bobo.ttf").to_path_buf());
    for c in &candidates {
        if c.exists() {
            match std::fs::read(c) {
                Ok(bytes) => {
                    return ([(axum::http::header::CONTENT_TYPE, "font/ttf")], bytes).into_response();
                }
                Err(_) => continue,
            }
        }
    }
    StatusCode::NOT_FOUND.into_response()
}

async fn polish_project_put(State(_state): State<Arc<AppState>>, Json(body): Json<serde_json::Value>) -> ApiResult {
    let dir = body.get("dir").and_then(|v| v.as_str()).ok_or_else(|| err(StatusCode::BAD_REQUEST, "缺少 dir"))?;
    let path = validated_project_dir(dir)?;
    // polish.toml 的 read-modify-write：时间轴保存 / 屏蔽区间 / 贴纸配置可能同时提交，
    // 无锁时后写者会覆盖先写者（前端确实会并发发多个 PUT）。
    vtrim_core::filelock::with_path_lock(&path.join("polish.toml"), || polish_project_put_locked(&path, &body)).await
}

/// 同步实现（调用方保证持有 polish.toml 的进程内锁）。
fn polish_project_put_locked(dir: &Path, body: &serde_json::Value) -> ApiResult {
    let mut cfg = vtrim_polish::project::load_project(dir).map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("加载失败: {e}")))?;
    if let Some(segments) = body.get("segments") {
        let segs = serde_json::from_value(segments.clone()).map_err(|e| err(StatusCode::BAD_REQUEST, format!("segments 解析失败: {e}")))?;
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

// core runner submodule
pub(crate) mod core {
    use std::path::PathBuf;
    use std::sync::Arc;
    use tokio::runtime::Handle;

    use crate::server::jobs::JobManager;

    /// WebUI 任务执行入口：复用 service::core_pipeline，只做 Job/进度桥接。
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
        crate::service::run_stages(
            &config_file,
            &stages,
            dry_run,
            resume,
            Some(progress),
        ).await
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

    #[test]
    fn parse_byte_range_handles_edge_cases() {
        assert_eq!(parse_byte_range(Some("bytes=0-99"), 1000), Some(Some((0, 99))));
        assert_eq!(parse_byte_range(Some("bytes=500-"), 1000), Some(Some((500, 999))));
        assert_eq!(parse_byte_range(Some("bytes=-100"), 1000), Some(Some((900, 999))));
        // 尾部区间收敛到 total-1
        assert_eq!(parse_byte_range(Some("bytes=990-5000"), 1000), Some(Some((990, 999))));
        // 不可满足 → 416（旧实现的 `end - start` 会整数下溢）
        assert_eq!(parse_byte_range(Some("bytes=900-100"), 1000), Some(None));
        assert_eq!(parse_byte_range(Some("bytes=2000-"), 1000), Some(None));
        assert_eq!(parse_byte_range(Some("bytes=-0"), 1000), Some(None));
        // 无法解析 → 调用方按 200 全量返回
        assert_eq!(parse_byte_range(Some("bytes=abc"), 1000), None);
        assert_eq!(parse_byte_range(Some("items=0-1"), 1000), None);
        assert_eq!(parse_byte_range(Some("bytes=0-1,5-6"), 1000), None);
        assert_eq!(parse_byte_range(None, 1000), None);
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

    #[test]
    fn project_dir_validation_rejects_empty_and_missing() {
        assert!(validated_project_dir("").is_err());
        assert!(validated_project_dir("   ").is_err());
        let missing = std::env::temp_dir().join("vtrim-dir-does-not-exist-xyz");
        assert!(validated_project_dir(&missing).is_err());
        assert!(validated_project_dir(std::env::temp_dir()).is_ok());
        assert!(validated_project_dir(std::env::temp_dir().to_string_lossy().to_string()).is_ok());
    }

    /// 带一套独立账号账本 + 已登录会话的测试服务。
    ///
    /// 账号库放在临时目录里（不经 `VTRIM_DATA_DIR` 环境变量：测试是并行跑的，
    /// 改全局环境变量会互相踩）。登录本身也走真实 KDF 与真实会话表。
    struct TestApp {
        app: Router,
        auth: Arc<crate::server::auth::AuthService>,
        _dir: tempfile::TempDir,
        sid: String,
        csrf: String,
    }

    async fn test_app_with_must_change(must_change: bool) -> TestApp {
        use crate::server::auth::{AuthService, Listening};
        let dir = tempfile::tempdir().unwrap();
        let auth = AuthService::open(dir.path().to_path_buf(), Listening::Loopback).unwrap();
        let hash = crate::service::auth::hash_password("initial-pass-1234").unwrap();
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
            app: router(state),
            auth,
            _dir: dir,
            sid: bundle.session_id,
            csrf: bundle.csrf,
        }
    }

    async fn test_app() -> TestApp {
        test_app_with_must_change(false).await
    }

    impl TestApp {
        fn cookie(&self) -> String {
            format!(
                "{}={}; {}={}",
                crate::service::auth::COOKIE_SID,
                self.sid,
                crate::service::auth::COOKIE_CSRF,
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
                b.header(crate::service::auth::CSRF_HEADER, self.csrf.clone())
            }
        }

        async fn call(&self, req: axum::http::Request<Body>) -> Response {
            self.app.clone().oneshot(req).await.unwrap()
        }
    }

    async fn body_json(res: Response) -> serde_json::Value {
        let bytes = axum::body::to_bytes(res.into_body(), 256 * 1024).await.unwrap();
        serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null)
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
        assert_eq!(health["accounts"], 1);
        assert_eq!(health["listening"], "loopback");
        assert_eq!(health["must_change_password"], false);
        assert!(health.get("workspace").is_none(), "health 不得暴露任何路径: {health}");

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
                    .header(crate::service::auth::CSRF_HEADER, "not-the-token")
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

    /// 登录 → `/api/me` → 改密 → `must_change_password` 变 false（§3.5.10 第 1–5 条）。
    #[tokio::test]
    async fn login_me_change_password_flow() {
        use crate::server::auth::{AuthService, Listening};
        let dir = tempfile::tempdir().unwrap();
        let auth = AuthService::open(dir.path().to_path_buf(), Listening::Loopback).unwrap();
        let hash = crate::service::auth::hash_password("initial-pass-1234").unwrap();
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
                    .header(crate::service::auth::CSRF_HEADER, &csrf)
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
                    .header(crate::service::auth::CSRF_HEADER, &csrf)
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
                    .header(crate::service::auth::CSRF_HEADER, &csrf)
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
        let hash = crate::service::auth::hash_password("initial-pass-1234").unwrap();
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
}
