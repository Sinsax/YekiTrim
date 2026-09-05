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
    extract::{Path as AxumPath, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response, sse::{Event, KeepAlive, Sse}},
    routing::{get, post, put},
    Json,
};
use futures_util::{StreamExt, stream::Stream};
use tokio_stream::wrappers::BroadcastStream;
use crate::service::Workspace;

use super::model::AppState;
use super::{jobs::JobEvent, media, persist};

pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/doctor", get(doctor))
        .route("/config", get(config_get))
        .route("/config/global", put(config_global_put))
        .route("/config/llm", put(config_llm_put))
        .route("/llm/test", post(llm_test))
        .route("/models", get(models_get))
        .route("/models/download", post(models_download))
        .route("/project/list", get(project_list))
        .route("/project/delete", post(project_delete))
        .route("/project/init", post(project_init))
        .route("/project/open", post(project_open))
        .route("/project-data/status", get(project_data_status))
        .route("/project-data/events", get(project_data_events).post(project_data_events_create).put(project_data_events_put))
        .route("/project-data/clips", get(project_data_clips))
        .route("/project-data/clips/:rank/thumb", get(project_data_clip_thumb))
        .route("/project-data/events/:rank/cut", post(project_data_event_cut))
        .route("/project-data/clips/:rank/silence", post(project_data_clip_silence))
        .route("/project-data/transcript", get(project_data_transcript).put(project_data_transcript_put))
        .route("/project-data/correct", post(project_data_correct))
        .route("/project-data/pipeline", post(project_data_pipeline))
        .route("/project-data/config", get(project_data_config).put(project_data_config_put))
        .route("/project-data/polish", get(project_data_polish))
        .route("/fs/list", post(fs_list))
        .route("/stream", get(global_stream))
        .route("/jobs", get(jobs_list))
        .route("/jobs/:id", get(job_get))
        .route("/jobs/:id/events", get(job_events))
        .route("/jobs/:id/decision", post(job_decision))
        .route("/jobs/:id/cancel", post(job_cancel))
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
        .route("/polish/select-version", post(polish_select_version))
        .route("/polish/cover-config", post(polish_cover_config))
        .route("/media/*path", get(media_get))
        .with_state(state)
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

fn command_available(cmd: &str) -> Option<String> {
    let output = std::process::Command::new(cmd).arg("-version").output().ok()?;
    if !output.status.success() { return None; }
    let first = String::from_utf8_lossy(&output.stdout).lines().next().map(|s| s.to_string()).unwrap_or_default();
    Some(first)
}

fn chrome_available(cfg: Option<&vtrim_core::config::Config>) -> bool {
    // 全局配置 [chrome] path 优先
    let custom = cfg
        .map(|c| c.chrome.path.trim())
        .unwrap_or("")
        .to_string();
    if !custom.is_empty() {
        return Path::new(&custom).exists()
            || std::process::Command::new(&custom)
                .arg("--version")
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false);
    }
    let candidates = [
        "chrome",
        "google-chrome",
        "chromium",
        r"C:\Program Files\Google\Chrome\Application\chrome.exe",
        r"C:\Program Files (x86)\Google\Chrome\Application\chrome.exe",
    ];
    candidates.iter().any(|c| {
        if Path::new(c).exists() {
            return true;
        }
        std::process::Command::new(c)
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    })
}

async fn health(State(state): State<Arc<AppState>>) -> ApiResult {
    let ws = workspace_from_state(&state).await;
    let cfg = ws.load_config().ok();
    let llm = cfg.as_ref().map(|c| !c.llm.api_key.is_empty() && c.llm.api_key != "sk-your-api-key-here").unwrap_or(false);
    ok(serde_json::json!({
        "ok": true,
        "version": env!("CARGO_PKG_VERSION"),
        "ffmpeg": command_available("ffmpeg"),
        "ffprobe": command_available("ffprobe"),
        "chrome": chrome_available(cfg.as_ref()),
        "llm": llm,
        "workspace": ws.config_file.to_string_lossy(),
    }))
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
        "ffmpeg": command_available("ffmpeg"),
        "ffprobe": command_available("ffprobe"),
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
    let cfg = ws.load_config().ok();
    let llm_cfg = vtrim_core::config::load_llm_config();
    ok(serde_json::json!({
        "config_file": ws.config_file.to_string_lossy(),
        "root_dir": ws.root_dir.to_string_lossy(),
        "vtrim_raw": vtrim_raw,
        "global_raw": global_raw,
        "llm_raw": llm_raw,
        "config": cfg.map(|c| serde_json::to_value(c).unwrap_or_default()).unwrap_or(serde_json::Value::Null),
        "llm_config": serde_json::to_value(llm_cfg).unwrap_or_default(),
    }))
}

async fn config_get(State(state): State<Arc<AppState>>) -> ApiResult {
    let ws = workspace_from_state(&state).await;
    config_get_impl(&ws)
}

async fn config_vtrim_put(State(state): State<Arc<AppState>>, Json(body): Json<serde_json::Value>) -> ApiResult {
    let ws = workspace_from_state(&state).await;
    config_vtrim_put_impl(&ws, &body)
}

fn config_vtrim_put_impl(ws: &Workspace, body: &serde_json::Value) -> ApiResult {
    let raw = body.get("raw").and_then(|v| v.as_str()).or_else(|| body.get("content").and_then(|v| v.as_str()));
    let Some(raw) = raw else { return Err(err(StatusCode::BAD_REQUEST, "缺少 raw 字段")); };
    if let Some(parent) = ws.config_file.parent() { std::fs::create_dir_all(parent).map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("创建目录失败: {e}")))?; }
    std::fs::write(&ws.config_file, raw).map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("保存失败: {e}")))?;
    ok(serde_json::json!({ "ok": true }))
}

async fn config_global_put(State(_state): State<Arc<AppState>>, Json(body): Json<serde_json::Value>) -> ApiResult {
    let raw = body.get("raw").and_then(|v| v.as_str()).or_else(|| body.get("content").and_then(|v| v.as_str()));
    let Some(raw) = raw else { return Err(err(StatusCode::BAD_REQUEST, "缺少 raw 字段")); };
    let path = vtrim_core::path::vtrim_home().join("config.toml");
    if let Some(parent) = path.parent() { std::fs::create_dir_all(parent).map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("创建目录失败: {e}")))?; }
    std::fs::write(&path, raw).map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("保存失败: {e}")))?;
    ok(serde_json::json!({ "ok": true }))
}

async fn config_llm_put(State(_state): State<Arc<AppState>>, Json(body): Json<serde_json::Value>) -> ApiResult {
    let raw = body.get("raw").and_then(|v| v.as_str()).or_else(|| body.get("content").and_then(|v| v.as_str()));
    let Some(raw) = raw else { return Err(err(StatusCode::BAD_REQUEST, "缺少 raw 字段")); };
    let path = vtrim_core::path::vtrim_home().join("llm.toml");
    if let Some(parent) = path.parent() { std::fs::create_dir_all(parent).map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("创建目录失败: {e}")))?; }
    std::fs::write(&path, raw).map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("保存失败: {e}")))?;
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

async fn models_download(State(state): State<Arc<AppState>>, Json(_body): Json<serde_json::Value>) -> ApiResult {
    let id = state.jobs.create("download_models").await;
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

async fn project_switch(State(state): State<Arc<AppState>>, Json(body): Json<serde_json::Value>) -> ApiResult {
    let Some(p) = body.get("config_path").and_then(|v| v.as_str()) else {
        return Err(err(StatusCode::BAD_REQUEST, "缺少 config_path"));
    };
    let ws = Workspace::load(Some(Path::new(p))).map_err(|e| err(StatusCode::BAD_REQUEST, format!("切换失败: {e:#}")))?;
    if ws.config_file.exists() {
        let _ = vtrim_core::config::normalize_project_paths_in(&ws.config_file);
    }
    state.set_workspace(ws.clone()).await;
    persist::remember_last(&ws.config_file, Some(&state.workspace_root().await));
    ok(serde_json::json!({ "ok": true, "config_file": ws.config_file.to_string_lossy() }))
}

fn project_summary(ws: &Workspace) -> serde_json::Value {
    crate::service::project_summary(ws)
}

async fn find_workspace_by_id(state: &AppState, id: &str) -> Option<Workspace> {
    let root = state.workspace_root().await;
    let mut stack = vec![root];
    for depth in 0..4 {
        if stack.is_empty() { break; }
        let mut next = vec![];
        for dir in stack.drain(..) {
            let cfg_file = dir.join("vtrim.toml");
            if cfg_file.exists() {
                if let Ok(ws) = Workspace::load(Some(&cfg_file)) {
                    let pid = project_summary(&ws)["id"].as_str().unwrap_or("").to_string();
                    if pid == id || ws.root_dir.file_name().and_then(|s| s.to_str()).map(|s| s == id).unwrap_or(false) {
                        return Some(ws);
                    }
                }
            }
            if depth < 3 {
                if let Ok(entries) = std::fs::read_dir(&dir) {
                    for entry in entries.flatten() {
                        let p = entry.path();
                        if p.is_dir() {
                            let name = entry.file_name().to_string_lossy().to_lowercase();
                            if name == "target" || name == ".git" || name == "node_modules" { continue; }
                            next.push(p);
                        }
                    }
                }
            }
        }
        stack = next;
    }
    None
}

async fn project_workspace(State(state): State<Arc<AppState>>, Json(body): Json<serde_json::Value>) -> ApiResult {
    let root = body.get("root").and_then(|v| v.as_str()).unwrap_or_default();
    if root.is_empty() {
        return Err(err(StatusCode::BAD_REQUEST, "缺少 root"));
    }
    let path = PathBuf::from(root);
    if !path.is_dir() {
        return Err(err(StatusCode::BAD_REQUEST, "工作区目录不存在"));
    }
    state.set_workspace_root(path.clone()).await;
    persist::remember_workspace_root(&path);
    ok(serde_json::json!({ "ok": true, "root": path.to_string_lossy() }))
}

async fn project_delete(State(_state): State<Arc<AppState>>, Json(body): Json<serde_json::Value>) -> ApiResult {
    let p = body.get("config_path").and_then(|v| v.as_str()).ok_or_else(|| err(StatusCode::BAD_REQUEST, "缺少 config_path"))?;
    let path = PathBuf::from(p);
    if path.exists() { std::fs::remove_file(&path).map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("删除失败: {e}")))?; }
    else { return Err(err(StatusCode::NOT_FOUND, "配置文件不存在")); }
    persist::forget_project(&path);
    ok(serde_json::json!({ "ok": true, "deleted": path.to_string_lossy() }))
}

async fn project_list(State(state): State<Arc<AppState>>) -> ApiResult {
    let root = state.workspace_root().await;
    let saved = persist::load_state();
    let paths = saved.as_ref().map(|s| s.projects.clone()).unwrap_or_default();
    let mut out = vec![];

    if paths.is_empty() {
        // 首次启动兼容：扫描一次工作区根目录并写入 server.toml，后续不再依赖扫描
        let mut stack = vec![root.clone()];
        for depth in 0..4 {
            if stack.is_empty() { break; }
            let mut next = vec![];
            for dir in stack.drain(..) {
                let cfg_file = dir.join("vtrim.toml");
                if cfg_file.exists() {
                    if let Ok(ws) = Workspace::load(Some(&cfg_file)) {
                        out.push(project_summary(&ws));
                        persist::remember_project(&ws.config_file);
                    }
                }
                if depth < 3 {
                    if let Ok(entries) = std::fs::read_dir(&dir) {
                        for entry in entries.flatten() {
                            let p = entry.path();
                            if p.is_dir() {
                                let name = entry.file_name().to_string_lossy().to_lowercase();
                                if name == "target" || name == ".git" || name == "node_modules" { continue; }
                                next.push(p);
                            }
                        }
                    }
                }
            }
            stack = next;
        }
    } else {
        // 直接从 server.toml 读取项目 vtrim.toml 路径，支持任意目录
        for cfg in paths {
            let p = PathBuf::from(&cfg);
            if p.exists() {
                if let Ok(ws) = Workspace::load(Some(&p)) {
                    out.push(project_summary(&ws));
                }
            }
        }
    }

    out.sort_by(|a,b| a["name"].as_str().unwrap_or("").cmp(b["name"].as_str().unwrap_or("")));
    ok(serde_json::json!({ "workspace_root": root.to_string_lossy(), "projects": out }))
}





fn project_snapshot(ws: &Workspace) -> serde_json::Value {
    crate::service::project_snapshot(ws)
}

fn project_status_impl(ws: &Workspace) -> ApiResult {
    ok(project_snapshot(ws))
}

async fn project_status(State(state): State<Arc<AppState>>) -> ApiResult {
    let ws = workspace_from_state(&state).await;
    project_status_impl(&ws)
}

async fn project_detail(State(state): State<Arc<AppState>>) -> ApiResult {
    let ws = workspace_from_state(&state).await;
    let snapshot = project_snapshot(&ws);
    let mut detail = snapshot;
    let clips_dir = ws.output_dir().join("clips");
    let clip_count = if clips_dir.exists() {
        std::fs::read_dir(&clips_dir).map(|e| e.flatten().filter(|x| x.path().is_dir()).count()).unwrap_or(0)
    } else { 0 };
    detail["clip_count"] = serde_json::json!(clip_count);
    ok(detail)
}

async fn project_init(State(state): State<Arc<AppState>>, Json(body): Json<serde_json::Value>) -> ApiResult {
    let input = body.get("input").and_then(|v| v.as_str()).ok_or_else(|| err(StatusCode::BAD_REQUEST, "缺少 input"))?;
    let config_path = body.get("config_path").and_then(|v| v.as_str()).map(PathBuf::from).unwrap_or_else(|| {
        let p = Path::new(input);
        if p.is_absolute() { p.parent().map(|d| d.join("vtrim.toml")).unwrap_or_else(|| PathBuf::from("vtrim.toml")) }
        else { PathBuf::from("vtrim.toml") }
    });
    let input_path = PathBuf::from(input);
    if !input_path.exists() { return Err(err(StatusCode::BAD_REQUEST, "输入视频不存在")); }
    let danmaku = body.get("danmaku").and_then(|v| v.as_str()).map(PathBuf::from);
    let output = body.get("output_dir").and_then(|v| v.as_str());
    let _ = std::fs::create_dir_all(config_path.parent().unwrap_or(Path::new(".")));
    vtrim_core::config::write_project_config(
        &config_path, &input_path, danmaku.as_deref(), output,
    ).map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("初始化失败: {e:#}")))?;
    let ws = Workspace::load(Some(&config_path)).map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("加载失败: {e:#}")))?;
    persist::remember_project(&ws.config_file);
    ok(project_snapshot(&ws))
}

async fn project_open(State(state): State<Arc<AppState>>, Json(body): Json<serde_json::Value>) -> ApiResult {
    let input = body.get("input").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let config_path = body.get("config_path").and_then(|v| v.as_str()).map(PathBuf::from).or_else(|| {
        let p = Path::new(&input);
        if p.is_absolute() {
            p.parent().map(|d| d.join("vtrim.toml"))
        } else {
            None
        }
    });
    let Some(cfg) = config_path.filter(|p| p.exists()) else {
        return Err(err(StatusCode::NOT_FOUND, "未找到项目文件"));
    };
    let ws = Workspace::load(Some(&cfg)).map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("加载失败: {e:#}")))?;
    persist::remember_project(&ws.config_file);
    ok(project_snapshot(&ws))
}

async fn project_by_id_status(State(state): State<Arc<AppState>>, AxumPath(id): AxumPath<String>) -> ApiResult {
    let ws = find_workspace_by_id(&state, &id).await
        .ok_or_else(|| err(StatusCode::NOT_FOUND, "项目不存在"))?;
    project_status_impl(&ws)
}

async fn project_by_id_events(State(state): State<Arc<AppState>>, AxumPath(id): AxumPath<String>) -> ApiResult {
    let ws = find_workspace_by_id(&state, &id).await
        .ok_or_else(|| err(StatusCode::NOT_FOUND, "项目不存在"))?;
    data_events_impl(&ws)
}

async fn project_by_id_clips(State(state): State<Arc<AppState>>, AxumPath(id): AxumPath<String>) -> ApiResult {
    let ws = find_workspace_by_id(&state, &id).await
        .ok_or_else(|| err(StatusCode::NOT_FOUND, "项目不存在"))?;
    data_clips_impl(&ws)
}

async fn project_by_id_polish(State(state): State<Arc<AppState>>, AxumPath(id): AxumPath<String>) -> ApiResult {
    let ws = find_workspace_by_id(&state, &id).await
        .ok_or_else(|| err(StatusCode::NOT_FOUND, "项目不存在"))?;
    polish_projects_impl(&ws)
}

fn workspace_from_config(config: &str) -> Result<Workspace, AppError> {
    if config.is_empty() {
        return Err(err(StatusCode::BAD_REQUEST, "缺少 config 参数"));
    }
    Workspace::load(Some(Path::new(config))).map_err(|e| err(StatusCode::BAD_REQUEST, format!("加载项目失败: {e:#}")))
}

async fn project_data_status(State(_state): State<Arc<AppState>>, Query(q): Query<std::collections::HashMap<String, String>>) -> ApiResult {
    let config = q.get("config").cloned().unwrap_or_default();
    let ws = workspace_from_config(&config)?;
    project_status_impl(&ws)
}

async fn project_data_events(State(_state): State<Arc<AppState>>, Query(q): Query<std::collections::HashMap<String, String>>) -> ApiResult {
    let config = q.get("config").cloned().unwrap_or_default();
    let ws = workspace_from_config(&config)?;
    data_events_impl(&ws)
}

async fn project_data_clips(State(_state): State<Arc<AppState>>, Query(q): Query<std::collections::HashMap<String, String>>) -> ApiResult {
    let config = q.get("config").cloned().unwrap_or_default();
    let ws = workspace_from_config(&config)?;
    data_clips_impl(&ws)
}

async fn project_data_clip_thumb(State(_state): State<Arc<AppState>>, Query(q): Query<std::collections::HashMap<String, String>>, AxumPath(rank): AxumPath<String>) -> Response {
    let config = q.get("config").cloned().unwrap_or_default();
    let Ok(ws) = workspace_from_config(&config) else {
        return StatusCode::BAD_REQUEST.into_response();
    };
    let clip_dir = ws.output_dir().join("clips").join(format!("clip-{:0>2}", rank));
    let video = clip_dir.join("clip.mp4");
    let thumb = clip_dir.join("_thumb.jpg");
    if !thumb.exists() && video.exists() {
        let _ = std::process::Command::new("ffmpeg")
            .args(["-ss","1","-i", &video.to_string_lossy(), "-vframes","1","-q:v","4","-y", &thumb.to_string_lossy()])
            .stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).status();
    }
    match tokio::fs::read(&thumb).await {
        Ok(bytes) => ([(axum::http::header::CONTENT_TYPE, "image/jpeg")], bytes).into_response(),
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn project_data_polish(State(_state): State<Arc<AppState>>, Query(q): Query<std::collections::HashMap<String, String>>) -> ApiResult {
    let config = q.get("config").cloned().unwrap_or_default();
    let ws = workspace_from_config(&config)?;
    polish_projects_impl(&ws)
}

async fn project_data_transcript(State(_state): State<Arc<AppState>>, Query(q): Query<std::collections::HashMap<String, String>>) -> ApiResult {
    let config = q.get("config").cloned().unwrap_or_default();
    let ws = workspace_from_config(&config)?;
    data_transcript_impl(&ws)
}

async fn project_data_transcript_put(State(_state): State<Arc<AppState>>, Query(q): Query<std::collections::HashMap<String, String>>, Json(body): Json<serde_json::Value>) -> ApiResult {
    let config = q.get("config").cloned().unwrap_or_default();
    let ws = workspace_from_config(&config)?;
    data_transcript_put_impl(&ws, &body)
}

async fn project_data_events_create(State(_state): State<Arc<AppState>>, Query(q): Query<std::collections::HashMap<String, String>>, Json(body): Json<serde_json::Value>) -> ApiResult {
    let config = q.get("config").cloned().unwrap_or_default();
    let ws = workspace_from_config(&config)?;
    data_events_create_impl(&ws, &body)
}

async fn project_data_events_put(State(_state): State<Arc<AppState>>, Query(q): Query<std::collections::HashMap<String, String>>, Json(body): Json<serde_json::Value>) -> ApiResult {
    let config = q.get("config").cloned().unwrap_or_default();
    let ws = workspace_from_config(&config)?;
    data_events_put_impl(&ws, &body)
}

async fn project_data_event_cut(State(state): State<Arc<AppState>>, Query(q): Query<std::collections::HashMap<String, String>>, AxumPath(rank): AxumPath<String>) -> ApiResult {
    let config = q.get("config").cloned().unwrap_or_default();
    let ws = workspace_from_config(&config)?;
    event_cut_impl(&ws, &rank, &state.jobs).await
}

async fn project_data_clip_silence(State(state): State<Arc<AppState>>, Query(q): Query<std::collections::HashMap<String, String>>, AxumPath(rank): AxumPath<String>) -> ApiResult {
    let config = q.get("config").cloned().unwrap_or_default();
    let ws = workspace_from_config(&config)?;
    clip_silence_impl(&ws, &rank, &state.jobs).await
}

async fn project_data_correct(State(state): State<Arc<AppState>>, Query(q): Query<std::collections::HashMap<String, String>>) -> ApiResult {
    let config = q.get("config").cloned().unwrap_or_default();
    let ws = workspace_from_config(&config)?;
    correct_run_impl(&ws, &state.jobs).await
}

async fn project_data_pipeline(State(state): State<Arc<AppState>>, Json(body): Json<serde_json::Value>) -> ApiResult {
    let config = body.get("config").and_then(|v| v.as_str()).unwrap_or_default().to_string();
    let ws = workspace_from_config(&config)?;
    if let Some(stage) = body.get("stage").and_then(|v| v.as_str()) {
        let resume = body.get("resume").and_then(|v| v.as_str()).map(|s| s != "restart").unwrap_or(true);
        let stages = match stage {
            "extract" => vec!["extract"],
            "transcribe" => vec!["transcribe"],
            "detect" => vec!["detect"],
            "cut" => vec!["cut"],
            "process" => vec!["extract","transcribe","detect","cut"],
            _ => return Err(err(StatusCode::BAD_REQUEST, "未知 stage")),
        }.into_iter().map(|s| s.to_string()).collect();
        run_pipeline_job(&state.jobs, ws.config_file.clone(), &format!("stage_{}", stage), stages, false, resume).await
    } else {
        let stages = body.get("stages").and_then(|v| v.as_array()).map(|arr| {
            arr.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect::<Vec<_>>()
        }).unwrap_or_else(|| vec!["extract".into(),"transcribe".into(),"detect".into(),"cut".into()]);
        let dry = body.get("dry_run").and_then(|v| v.as_bool()).unwrap_or(false);
        let resume = body.get("resume").and_then(|v| v.as_str()).map(|s| s != "restart").unwrap_or(true);
        run_pipeline_job(&state.jobs, ws.config_file.clone(), "pipeline", stages, dry, resume).await
    }
}

async fn project_data_config(State(_state): State<Arc<AppState>>, Query(q): Query<std::collections::HashMap<String, String>>) -> ApiResult {
    let config = q.get("config").cloned().unwrap_or_default();
    let ws = workspace_from_config(&config)?;
    config_get_impl(&ws)
}

async fn project_data_config_put(State(_state): State<Arc<AppState>>, Query(q): Query<std::collections::HashMap<String, String>>, Json(body): Json<serde_json::Value>) -> ApiResult {
    let config = q.get("config").cloned().unwrap_or_default();
    let ws = workspace_from_config(&config)?;
    config_vtrim_put_impl(&ws, &body)
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

async fn data_transcript(State(state): State<Arc<AppState>>) -> ApiResult {
    let ws = workspace_from_state(&state).await;
    data_transcript_impl(&ws)
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

async fn data_transcript_put(State(state): State<Arc<AppState>>, Json(body): Json<serde_json::Value>) -> ApiResult {
    let ws = workspace_from_state(&state).await;
    data_transcript_put_impl(&ws, &body)
}

async fn data_heatmap(State(state): State<Arc<AppState>>) -> ApiResult {
    let ws = workspace_from_state(&state).await;
    let cfg = ws.load_config().map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}")))?;
    let danmaku = cfg.project.danmaku_file.as_ref().filter(|s| !s.is_empty()).map(|s| ws.resolve(s));
    let output = ws.output_dir();
    let duration = vtrim_core::detection::load_events(&output.join("events.json")).ok()
        .and_then(|evs| evs.last().map(|e| e.end.to_seconds())).unwrap_or(0.0);
    if let Some(dm) = danmaku.filter(|p| p.exists()) {
        let windows = vtrim_core::danmaku::analyze(&dm, cfg.detection.danmaku_window_secs, duration)
            .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("弹幕分析失败: {e}")))?;
        let windows: Vec<serde_json::Value> = windows.into_iter().map(|w| serde_json::json!({
            "start": w.start, "density": w.density, "count": w.count,
        })).collect();
        return ok(serde_json::json!({ "windows": windows }));
    }
    Err(err(StatusCode::NOT_FOUND, "无弹幕文件或数据"))
}

fn data_events_impl(ws: &Workspace) -> ApiResult {
    let path = ws.output_dir().join("events.json");
    if !path.exists() { return Err(err(StatusCode::NOT_FOUND, "暂无 events.json")); }
    let raw = std::fs::read_to_string(&path).map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("读取失败: {e}")))?;
    let mut v: serde_json::Value = serde_json::from_str(&raw).unwrap_or(serde_json::Value::Null);
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
        }
    }
    ok(v)
}

async fn data_events(State(state): State<Arc<AppState>>) -> ApiResult {
    let ws = workspace_from_state(&state).await;
    data_events_impl(&ws)
}

fn ev_time_secs(v: &serde_json::Value, key: &str) -> f64 {
    let o = match v.get(key).and_then(|x| x.as_object()) { Some(o)=>o, None=>return 0.0 };
    let h=o.get("h").and_then(|x|x.as_u64()).unwrap_or(0) as f64;
    let m=o.get("m").and_then(|x|x.as_u64()).unwrap_or(0) as f64;
    let s=o.get("s").and_then(|x|x.as_u64()).unwrap_or(0) as f64;
    let ms=o.get("ms").and_then(|x|x.as_u64()).unwrap_or(0) as f64;
    h*3600.0+m*60.0+s+ms/1000.0
}


fn sec_to_ts_value(sec: f64) -> serde_json::Value {
    let sec = sec.max(0.0);
    let h = (sec / 3600.0).floor() as u64;
    let m = ((sec % 3600.0) / 60.0).floor() as u64;
    let s = (sec % 60.0).floor() as u64;
    let ms = ((sec - sec.floor()) * 1000.0).round() as u64;
    serde_json::json!({ "h": h, "m": m, "s": s, "ms": ms })
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

async fn data_events_create(State(state): State<Arc<AppState>>, Json(body): Json<serde_json::Value>) -> ApiResult {
    let ws = workspace_from_state(&state).await;
    data_events_create_impl(&ws, &body)
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

async fn data_events_put(State(state): State<Arc<AppState>>, Json(body): Json<serde_json::Value>) -> ApiResult {
    let ws = workspace_from_state(&state).await;
    data_events_put_impl(&ws, &body)
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
        let silent_exists = !silent_files.is_empty();
        out.push(serde_json::json!({
            "dir": dir_path.to_string_lossy(),
            "name": entry.file_name().to_string_lossy(),
            "video_exists": video_path.exists(),
            "size_bytes": size_bytes,
            "duration_secs": duration_secs,
            "silenced": silent_exists,
            "silent_files": silent_files,
            "meta": meta,
        }));
    }
    out.sort_by(|a,b| a["name"].as_str().unwrap_or("").cmp(b["name"].as_str().unwrap_or("")));
    ok(serde_json::json!(out))
}

async fn data_clips(State(state): State<Arc<AppState>>) -> ApiResult {
    let ws = workspace_from_state(&state).await;
    data_clips_impl(&ws)
}

async fn clip_thumb(State(state): State<Arc<AppState>>, AxumPath(rank): AxumPath<String>) -> Response {
    let ws = state.workspace().await;
    let clip_dir = ws.output_dir().join("clips").join(format!("clip-{:0>2}", rank));
    let video = clip_dir.join("clip.mp4");
    let thumb = clip_dir.join("_thumb.jpg");
    if !thumb.exists() && video.exists() {
        let _ = std::process::Command::new("ffmpeg")
            .args(["-ss","1","-i", &video.to_string_lossy(), "-vframes","1","-q:v","4","-y", &thumb.to_string_lossy()])
            .stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).status();
    }
    match tokio::fs::read(&thumb).await {
        Ok(bytes) => ([(axum::http::header::CONTENT_TYPE, "image/jpeg")], bytes).into_response(),
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn clip_silence_impl(ws: &Workspace, rank: &str, jobs: &Arc<super::jobs::JobManager>) -> ApiResult {
    let clip_dir = ws.output_dir().join("clips").join(format!("clip-{:0>2}", rank));
    let video = clip_dir.join("clip.mp4");
    if !video.exists() { return Err(err(StatusCode::NOT_FOUND, "clip.mp4 不存在")); }
    let id = jobs.create("silence").await;
    jobs.mark_running(&id, format!("静音压缩 clip-{}", rank)).await;
    let jobs_handle = jobs.clone();
    let job_id = id.clone();
    let out = clip_dir.join(format!("clip_{}_cleaned.mp4", rank));
    tokio::spawn(async move {
        let result = vtrim_core::cutting::remove_silence(&video, &out, 2.0, None, -30.0);
        match result {
            Ok(_) => jobs_handle.complete(&job_id).await,
            Err(e) => jobs_handle.fail(&job_id, format!("{e:#}")).await,
        }
    });
    ok(serde_json::json!({ "ok": true, "id": id }))
}

async fn clip_silence(State(state): State<Arc<AppState>>, AxumPath(rank): AxumPath<String>) -> ApiResult {
    let ws = workspace_from_state(&state).await;
    clip_silence_impl(&ws, &rank, &state.jobs).await
}

async fn event_cut_impl(ws: &Workspace, rank: &str, jobs: &Arc<super::jobs::JobManager>) -> ApiResult {
    let ev_path = ws.output_dir().join("events.json");
    if !ev_path.exists() { return Err(err(StatusCode::NOT_FOUND, "暂无 events.json")); }
    let raw = std::fs::read_to_string(&ev_path).map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("读取失败: {e}")))?;
    let store: serde_json::Value = serde_json::from_str(&raw).map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("解析失败: {e}")))?;
    let rank_u: u32 = rank.parse().map_err(|_| err(StatusCode::BAD_REQUEST, "rank 无效"))?;
    let event: serde_json::Value = store.get("events").and_then(|a| a.as_array()).and_then(|arr| arr.iter().find(|e| e.get("rank").and_then(|r| r.as_u64()).unwrap_or(0) as u32 == rank_u).cloned())
        .ok_or_else(|| err(StatusCode::NOT_FOUND, "事件不存在"))?;
    let ev: vtrim_core::types::Event = serde_json::from_value(event).map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("事件解析失败: {e}")))?;
    let id = jobs.create("cut_one").await;
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

async fn event_cut(State(state): State<Arc<AppState>>, AxumPath(rank): AxumPath<String>) -> ApiResult {
    let ws = state.workspace().await;
    event_cut_impl(&ws, &rank, &state.jobs).await
}

async fn fs_list(State(state): State<Arc<AppState>>, Json(body): Json<serde_json::Value>) -> ApiResult {
    let ws = workspace_from_state(&state).await;
    let dir = body.get("dir").and_then(|v| v.as_str()).unwrap_or_default();
    let target = if dir.is_empty() { ws.root_dir.clone() } else if Path::new(dir).is_absolute() { PathBuf::from(dir) } else { ws.resolve(dir) };
    if !target.exists() || !target.is_dir() { return Err(err(StatusCode::BAD_REQUEST, "目录不存在")); }
    let mut files = vec![]; let mut dirs = vec![];
    for e in std::fs::read_dir(&target).map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("读取失败: {e}")))? {
        let e = e.map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("读取失败: {e}")))?;
        if e.file_type().map(|t| t.is_dir()).unwrap_or(false) { dirs.push(e.file_name().to_string_lossy().to_string()); }
        else { files.push(e.file_name().to_string_lossy().to_string()); }
    }
    dirs.sort(); files.sort();
    ok(serde_json::json!({ "dir": target.to_string_lossy(), "dirs": dirs, "files": files }))
}

async fn media_get(
    State(state): State<Arc<AppState>>,
    AxumPath(path): AxumPath<String>,
    headers: axum::http::HeaderMap,
) -> Response {
    use tokio::io::{AsyncReadExt, AsyncSeekExt};

    let ws = state.workspace().await;
    let root = state.workspace_root().await;
    let Some(path) = media::safe_join(&ws, &root, &path) else {
        return media::error_response("文件不存在或不在允许范围");
    };
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
    let range = headers.get(axum::http::header::RANGE).and_then(|v| v.to_str().ok()).and_then(|v| {
        let v = v.trim().strip_prefix("bytes=")?;
        let mut parts = v.split('-');
        let start: u64 = parts.next()?.parse().ok()?;
        let end: Option<u64> = parts.next().and_then(|e| if e.is_empty() { None } else { e.parse().ok() });
        Some((start, end))
    });

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

    if let Some((start_raw, end_opt)) = range {
        if start_raw >= total {
            return (StatusCode::RANGE_NOT_SATISFIABLE, [(axum::http::header::CONTENT_RANGE, format!("bytes */{}", total))]).into_response();
        }
        let end = end_opt.unwrap_or(total.saturating_sub(1)).min(total.saturating_sub(1));
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
    config_file: std::path::PathBuf,
    kind: &str,
    stages: Vec<String>,
    dry_run: bool,
    resume: bool,
) -> ApiResult {
    let id = jobs.create(kind).await;
    jobs.mark_running(&id, "准备中").await;
    let jobs = jobs.clone();
    let run_stages = stages.clone();
    let job_id = id.clone();
    tokio::spawn(async move {
        let result = crate::server::api::core::run_stages(config_file, run_stages, dry_run, resume, job_id.clone(), jobs.clone()).await;
        if let Err(e) = result {
            jobs.fail(&job_id, format!("{e:#}")).await;
            return;
        }
        jobs.complete(&job_id).await;
    });
    ok(serde_json::json!({ "ok": true, "id": id }))
}

async fn pipeline_run(State(state): State<Arc<AppState>>, Json(body): Json<serde_json::Value>) -> ApiResult {
    let stages = body.get("stages").and_then(|v| v.as_array()).map(|arr| {
        arr.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect::<Vec<_>>()
    }).unwrap_or_else(|| vec!["extract".into(),"transcribe".into(),"detect".into(),"cut".into()]);
    let dry = body.get("dry_run").and_then(|v| v.as_bool()).unwrap_or(false);
    let resume = body.get("resume").and_then(|v| v.as_str()).map(|s| s != "restart").unwrap_or(true);
    let ws = state.workspace().await;
    run_pipeline_job(&state.jobs, ws.config_file.clone(), "pipeline", stages, dry, resume).await
}

async fn pipeline_stage(State(state): State<Arc<AppState>>, Json(body): Json<serde_json::Value>) -> ApiResult {
    let stage = body.get("stage").and_then(|v| v.as_str()).unwrap_or("");
    let resume = body.get("resume").and_then(|v| v.as_str()).map(|s| s != "restart").unwrap_or(true);
    let stages = match stage {
        "extract" => vec!["extract"],
        "transcribe" => vec!["transcribe"],
        "detect" => vec!["detect"],
        "cut" => vec!["cut"],
        "process" => vec!["extract","transcribe","detect","cut"],
        _ => return Err(err(StatusCode::BAD_REQUEST, "未知 stage")),
    }.into_iter().map(|s| s.to_string()).collect();
    let ws = state.workspace().await;
    run_pipeline_job(&state.jobs, ws.config_file.clone(), &format!("stage_{}", stage), stages, false, resume).await
}

async fn correct_run_impl(ws: &Workspace, jobs: &Arc<super::jobs::JobManager>) -> ApiResult {
    let id = jobs.create("correct").await;
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

async fn correct_run(State(state): State<Arc<AppState>>) -> ApiResult {
    let ws = state.workspace().await;
    correct_run_impl(&ws, &state.jobs).await
}

async fn jobs_list(State(state): State<Arc<AppState>>) -> ApiResult {
    let list = state.jobs.list().await;
    ok(serde_json::to_value(list).unwrap_or_default())
}

async fn job_get(State(state): State<Arc<AppState>>, AxumPath(id): AxumPath<String>) -> ApiResult {
    match state.jobs.get(&id).await {
        Some(j) => ok(serde_json::to_value(j).unwrap_or_default()),
        None => Err(err(StatusCode::NOT_FOUND, "任务不存在")),
    }
}

async fn global_stream(State(state): State<Arc<AppState>>) -> Response {
    let rx = state.jobs.subscribe_all().await;
    let stream: std::pin::Pin<Box<dyn Stream<Item = Result<Event, axum::Error>> + Send>> =
        Box::pin(BroadcastStream::new(rx).map(|item| {
            let item = item.unwrap_or(JobEvent::Failed { error: "事件流中断".into() });
            Ok(Event::default().event("message").data(serde_json::to_string(&item).unwrap_or_else(|_| "{}".into())))
        }));
    Sse::new(stream).keep_alive(KeepAlive::new().interval(Duration::from_secs(15))).into_response()
}

async fn job_events(
    State(state): State<Arc<AppState>>,
    AxumPath(id): AxumPath<String>,
) -> Response {
    let Some(rx) = state.jobs.subscribe(&id).await else {
        return (StatusCode::NOT_FOUND, "任务不存在").into_response();
    };
    let stream: std::pin::Pin<Box<dyn Stream<Item = Result<Event, axum::Error>> + Send>> =
        Box::pin(BroadcastStream::new(rx).map(|item| {
            let item = item.unwrap_or(JobEvent::Failed { error: "事件流中断".into() });
            Ok(Event::default().event("message").data(serde_json::to_string(&item).unwrap_or_else(|_| "{}".into())))
        }));
    Sse::new(stream).keep_alive(KeepAlive::new().interval(Duration::from_secs(15))).into_response()
}

async fn job_decision(State(state): State<Arc<AppState>>, AxumPath(id): AxumPath<String>, Json(body): Json<serde_json::Value>) -> ApiResult {
    let decision = body.get("decision").and_then(|v| v.as_bool()).unwrap_or(false);
    if state.jobs.resume(&id, decision).await {
        ok(serde_json::json!({ "ok": true }))
    } else {
        Err(err(StatusCode::BAD_REQUEST, "任务不在等待状态或不存在"))
    }
}

async fn job_cancel(State(state): State<Arc<AppState>>, AxumPath(id): AxumPath<String>) -> ApiResult {
    if state.jobs.cancel(&id).await {
        ok(serde_json::json!({ "ok": true }))
    } else {
        Err(err(StatusCode::BAD_REQUEST, "无法取消任务"))
    }
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
            let rendered = dir.join("export.mp4").exists() || dir.join("vtuber-clip.mp4").exists();
            let cover = dir.join("_cover_frame.jpg").exists() || dir.join("cover.jpg").exists();
            let mut version = "original";
            if polish_ready {
                if let Ok(cfg) = vtrim_polish::project::load_project(&dir) {
                    if cfg.source.video.to_lowercase().contains("cleaned") { version = "silent"; }
                }
            }
            out.push(serde_json::json!({
                "dir": dir.to_string_lossy(),
                "title": title,
                "polish_ready": polish_ready,
                "processed": processed,
                "rendered": rendered,
                "cover": cover,
                "version": version,
            }));
        }
    }
    out.sort_by(|a,b| a["dir"].as_str().unwrap_or("").cmp(b["dir"].as_str().unwrap_or("")));
    ok(serde_json::json!(out))
}

async fn polish_projects(State(state): State<Arc<AppState>>) -> ApiResult {
    let ws = workspace_from_state(&state).await;
    polish_projects_impl(&ws)
}

async fn polish_spawn(state: &AppState, kind: &str, dir: String) -> ApiResult {
    let id = state.jobs.create(format!("polish_{}", kind)).await;
    let dir_name = Path::new(&dir)
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| dir.clone());
    state.jobs.mark_running(&id, format!("polish {kind} · {dir_name}")).await;
    let jobs = state.jobs.clone();
    let path = PathBuf::from(dir);
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
                "cover" => vtrim_polish::cover::cmd_cover(&path, None).await,
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

async fn polish_init(State(state): State<Arc<AppState>>, Json(body): Json<serde_json::Value>) -> ApiResult {
    let dir = body.get("dir").and_then(|v| v.as_str()).unwrap_or("").to_string();
    polish_spawn(&state, "init", dir).await
}

async fn polish_llm(State(state): State<Arc<AppState>>, Json(body): Json<serde_json::Value>) -> ApiResult {
    let dir = body.get("dir").and_then(|v| v.as_str()).unwrap_or("").to_string();
    polish_spawn(&state, "llm", dir).await
}

async fn polish_process(State(state): State<Arc<AppState>>, Json(body): Json<serde_json::Value>) -> ApiResult {
    let dir = body.get("dir").and_then(|v| v.as_str()).unwrap_or("").to_string();
    polish_spawn(&state, "process", dir).await
}

async fn polish_render(State(state): State<Arc<AppState>>, Json(body): Json<serde_json::Value>) -> ApiResult {
    let dir = body.get("dir").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let path = PathBuf::from(&dir);
    if !path.join("polish.toml").exists() { return Err(err(StatusCode::BAD_REQUEST, "目录未初始化 polish")); }
    if let Some(output_file) = body.get("output_file").and_then(|v| v.as_str()).filter(|s| !s.is_empty()) {
        let mut cfg = vtrim_polish::project::load_project(&path).map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("加载失败: {e}")))?;
        cfg.project.output = output_file.to_string();
        let raw = cfg.to_toml_commented().map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("序列化失败: {e}")))?;
        std::fs::write(path.join("polish.toml"), raw).map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("写入失败: {e}")))?;
    }
    polish_spawn(&state, "render", dir).await
}

async fn polish_cover(State(state): State<Arc<AppState>>, Json(body): Json<serde_json::Value>) -> ApiResult {
    let dir = body.get("dir").and_then(|v| v.as_str()).unwrap_or("").to_string();
    polish_spawn(&state, "cover", dir).await
}

async fn polish_cover_config(State(_state): State<Arc<AppState>>, Json(body): Json<serde_json::Value>) -> ApiResult {
    let dir = body.get("dir").and_then(|v| v.as_str()).ok_or_else(|| err(StatusCode::BAD_REQUEST, "缺少 dir"))?;
    let path = Path::new(dir);
    if !path.join("polish.toml").exists() { return Err(err(StatusCode::BAD_REQUEST, "未初始化 polish")); }
    let mut cfg = vtrim_polish::project::load_project(path).map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("加载失败: {e}")))?;
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
        if let Some(lines) = cover.get("title_lines").and_then(|v| v.as_array()) {
            cfg.cover.title_lines = lines.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect();
        }
        if let Some(v) = cover.get("image").and_then(|v| v.as_str()) { cfg.cover.image = v.to_string(); }
    }
    let raw = cfg.to_toml_commented().map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("序列化失败: {e}")))?;
    std::fs::write(path.join("polish.toml"), raw).map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("写入失败: {e}")))?;
    ok(serde_json::json!({ "ok": true }))
}

async fn polish_select_version(State(_state): State<Arc<AppState>>, Json(body): Json<serde_json::Value>) -> ApiResult {
    let dir = body.get("dir").and_then(|v| v.as_str()).ok_or_else(|| err(StatusCode::BAD_REQUEST, "缺少 dir"))?;
    let version = body.get("version").and_then(|v| v.as_str()).unwrap_or("original");
    let path = Path::new(dir);
    if !path.join("polish.toml").exists() { return Err(err(StatusCode::BAD_REQUEST, "未初始化 polish")); }
    let mut cfg = vtrim_polish::project::load_project(path).map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("加载失败: {e}")))?;
    if version == "silent" {
        let cleaned = std::fs::read_dir(path).map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("读取失败: {e}")))?;
        let cleaned_video = cleaned.flatten().find(|f| f.path().is_file() && f.file_name().to_string_lossy().to_lowercase().contains("cleaned") && f.file_name().to_string_lossy().to_lowercase().ends_with(".mp4"))
            .map(|f| f.file_name().to_string_lossy().into_owned());
        let cleaned_srt = std::fs::read_dir(path).map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("读取失败: {e}")))?;
        let cleaned_sub = cleaned_srt.flatten().find(|f| f.path().is_file() && f.file_name().to_string_lossy().to_lowercase().contains("cleaned") && f.file_name().to_string_lossy().to_lowercase().ends_with(".srt"))
            .map(|f| f.file_name().to_string_lossy().into_owned());
        if let Some(v) = cleaned_video { cfg.source.video = v; }
        if let Some(s) = cleaned_sub { cfg.source.subtitles = s; }
    } else {
        cfg.source.video = "clip.mp4".into();
        cfg.source.subtitles = "clip.srt".into();
        cfg.source.danmaku = if path.join("clip.dm.srt").exists() { "clip.dm.srt".into() } else { String::new() };
    }
    let raw = cfg.to_toml_commented().map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("序列化失败: {e}")))?;
    std::fs::write(path.join("polish.toml"), raw).map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("写入失败: {e}")))?;
    ok(serde_json::json!({ "ok": true, "version": version }))
}

async fn polish_preview(State(_state): State<Arc<AppState>>, Json(body): Json<serde_json::Value>) -> ApiResult {
    let dir = body.get("dir").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let path = PathBuf::from(dir);
    if !path.join("polish.toml").exists() {
        return Err(err(StatusCode::BAD_REQUEST, "目录未初始化 polish"));
    }
    // 启动预览服务（不阻塞，返回 URL 供前端 iframe）
    let config = vtrim_polish::project::load_project(&path).map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("加载失败: {e}")))?;
    if !config.processed {
        let _ = vtrim_polish::process::cmd_process(&path).await;
    }
    vtrim_polish::process::ensure_assets(&path).await;
    let mut server = vtrim_polish::render::server::RenderServer::new(path.clone());
    let port = server.start().await.map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("预览启动失败: {e}")))?;
    let video_url = format!("http://127.0.0.1:{}/", port);
    // 前端需要通过 API 代理访问本机 127.0.0.1 随机端口；这里直接返回绝对 URL（浏览器同源可访问）
    ok(serde_json::json!({ "ok": true, "video_url": video_url, "cover_url": video_url }))
}

async fn polish_cover_background(State(_state): State<Arc<AppState>>, Json(body): Json<serde_json::Value>) -> ApiResult {
    let dir = body.get("dir").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let path = PathBuf::from(dir);
    if !path.join("polish.toml").exists() {
        return Err(err(StatusCode::BAD_REQUEST, "目录未初始化 polish"));
    }
    if let Some(data) = body.get("image_data").and_then(|v| v.as_str()) {
        use base64::Engine as _;
        let bytes = base64::engine::general_purpose::STANDARD.decode(data)
            .map_err(|e| err(StatusCode::BAD_REQUEST, format!("图片解码失败: {e}")))?;
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
    let path = PathBuf::from(dir);
    if !path.join("polish.toml").exists() {
        return Err(err(StatusCode::BAD_REQUEST, "目录未初始化 polish"));
    }
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

async fn polish_cover_preview(State(_state): State<Arc<AppState>>, Json(body): Json<serde_json::Value>) -> ApiResult {
    let dir = body.get("dir").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let path = PathBuf::from(dir);
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
    let html = vtrim_polish::cover::build_cover_html(&path, &config, &resolved);
    let port = vtrim_polish::render::server::start_file_server(path.clone(), Some(html)).await
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("封面预览启动失败: {e}")))?;
    let url = format!("http://127.0.0.1:{}/", port);
    ok(serde_json::json!({ "ok": true, "cover_url": url }))
}

async fn polish_subtitles_get(State(_state): State<Arc<AppState>>, Query(q): Query<std::collections::HashMap<String,String>>) -> ApiResult {
    let dir = q.get("dir").ok_or_else(|| err(StatusCode::BAD_REQUEST, "缺少 dir"))?;
    let path = PathBuf::from(dir);
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
    let path = PathBuf::from(dir);
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
    let path = PathBuf::from(dir);
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

async fn polish_project_put(State(_state): State<Arc<AppState>>, Json(body): Json<serde_json::Value>) -> ApiResult {
    let dir = body.get("dir").and_then(|v| v.as_str()).ok_or_else(|| err(StatusCode::BAD_REQUEST, "缺少 dir"))?;
    let mut cfg = vtrim_polish::project::load_project(Path::new(dir)).map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("加载失败: {e}")))?;
    if let Some(segments) = body.get("segments") {
        let segs = serde_json::from_value(segments.clone()).map_err(|e| err(StatusCode::BAD_REQUEST, format!("segments 解析失败: {e}")))?;
        cfg.segments = segs;
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
    std::fs::write(Path::new(dir).join("polish.toml"), raw).map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("保存失败: {e}")))?;
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
    ) -> anyhow::Result<()> {
        let progress: Arc<dyn vtrim_core::progress::ProgressReporter> =
            Arc::new(WebProgress::new(job_id, jobs));
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
    }

    impl WebProgress {
        fn new(id: String, jobs: Arc<JobManager>) -> Self { Self { id, jobs } }
    }

    #[allow(clippy::let_underscore_future)]
    impl vtrim_core::progress::ProgressReporter for WebProgress {
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
