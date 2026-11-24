//! `api::project_data`：项目数据面（事件 / 片段 / 字幕 / 素材 / 文件列表 / 媒体流）。

use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::{
    extract::{Extension, Path as AxumPath, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use yekitrim_service::Workspace;

use crate::model::AppState;
use crate::scope::Scope;
use crate::{
    jobs::{JobClass, JobSpec},
    media,
};
use super::*;
// ⚠️ **显式 use，不许 `…::*`**（整改 T16/T18 第一阶段，缘由见 `polish.rs` 顶部那段）。
// 现在 api 内部**没有模块环**：本文件是唯一的方向性依赖方（`project_data → polish` 只为
// `polish_projects_impl` 这一个快照函数），`polish` 侧已经不认识 `project_data`。
use super::config::{config_yekitrim_get_impl, config_yekitrim_put_impl};
use super::pipeline::{correct_run_impl, run_pipeline_job};
use super::polish::polish_projects_impl;
use super::projects::project_status_impl;

// ---- 项目数据（P2：作用域 = 本人项目 id，`?config=` 仅作过渡期反查） ----
//
// `Scope` 提取器从 `?project=<id>`（或过渡期的 `?config=<已登记本人项目>`）解析出项目与
// `Workspace`；**未登记的路径一律 404**，所以"任意路径"这条入口已经关闭。

// 以下都是"读一个项目的产物目录"的接口：目录可能在网络盘上，解析与统计一律下 blocking 池
pub(super) async fn project_data_status(scope: Scope) -> ApiResult {
    let ws = scope.ws;
    blocking(move || project_status_impl(&ws)).await
}

pub(super) async fn project_data_events(scope: Scope) -> ApiResult {
    let ws = scope.ws;
    blocking(move || data_events_impl(&ws)).await
}

pub(super) async fn project_data_clips(scope: Scope) -> ApiResult {
    let ws = scope.ws;
    blocking(move || data_clips_impl(&ws)).await
}

pub(super) async fn project_data_clip_thumb(scope: Scope, AxumPath(rank): AxumPath<String>) -> Response {
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
        let _guard = yekitrim_core::filelock::lock_path(&thumb).await;
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
pub(super) fn generate_clip_thumb(video: &Path, thumb: &Path) -> std::io::Result<()> {
    let status = yekitrim_core::process::cmd("ffmpeg")
        .args(["-ss","1","-i", &video.to_string_lossy(), "-vframes","1","-q:v","4","-y", &thumb.to_string_lossy()])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()?;
    if !status.success() {
        return Err(std::io::Error::new(std::io::ErrorKind::Other, "ffmpeg 退出码非 0"));
    }
    Ok(())
}

pub(super) async fn project_data_polish(scope: Scope) -> ApiResult {
    let ws = scope.ws;
    blocking(move || polish_projects_impl(&ws)).await
}

pub(super) async fn project_data_transcript(scope: Scope) -> ApiResult {
    let ws = scope.ws;
    blocking(move || data_transcript_impl(&ws)).await
}

pub(super) async fn project_data_transcript_put(scope: Scope, Json(body): Json<serde_json::Value>) -> ApiResult {
    let ws = scope.ws;
    let path = ws.output_dir().join("transcript.json");
    // 锁在 await 里持有，读改写本身下 blocking 池（`with_path_lock` 的闭包是同步的，会把
    // 整段 IO 留在 worker 上）
    let _guard = yekitrim_core::filelock::lock_path(&path).await;
    blocking(move || data_transcript_put_impl(&ws, &body)).await
}

/// `POST /api/project-data/events` 的请求体（手工建一条事件）。
///
/// ⚠️ 时间有**两条通道**且都还在用：新前端传 `start_sec`/`end_sec`，老调用方与 CLI 传
/// `start`/`end` —— 用 `alias` 保留，别删任何一条。`ranges`（多段剪）优先于单段起终点。
#[derive(serde::Deserialize)]
pub(super) struct CreateEventBody {
    /// 标题。**必填**（空串由 impl 回 400「标题不能为空」，与改造前同口径）。
    pub label: String,
    #[serde(default, alias = "start")]
    pub start_sec: Option<f64>,
    #[serde(default, alias = "end")]
    pub end_sec: Option<f64>,
    #[serde(default)]
    pub ranges: Vec<EventRangeBody>,
    #[serde(default)]
    pub event_type: Option<String>,
    #[serde(default)]
    pub summary: Option<String>,
}

/// `CreateEventBody.ranges` 的一条（多段剪的一段）。同样兼容 `start_sec`/`start` 两种写法。
#[derive(serde::Deserialize)]
pub(super) struct EventRangeBody {
    #[serde(default, alias = "start")]
    pub start_sec: Option<f64>,
    #[serde(default, alias = "end")]
    pub end_sec: Option<f64>,
}

pub(super) async fn project_data_events_create(scope: Scope, Json(body): Json<serde_json::Value>) -> ApiResult {
    let ws = scope.ws;
    let body: CreateEventBody = parse_body(body)?;
    // events.json 是 read-modify-write：并发 PUT/POST 会丢事件
    let path = ws.output_dir().join("events.json");
    let _guard = yekitrim_core::filelock::lock_path(&path).await;
    blocking(move || data_events_create_impl(&ws, &body)).await
}

pub(super) async fn project_data_events_put(scope: Scope, Json(body): Json<serde_json::Value>) -> ApiResult {
    let ws = scope.ws;
    let path = ws.output_dir().join("events.json");
    let _guard = yekitrim_core::filelock::lock_path(&path).await;
    blocking(move || data_events_put_impl(&ws, &body)).await
}

// 这两个入口在**建任务前**就要读 events.json / 逐个检查 clip.mp4 是否存在（全在网络盘上），
// 所以整段（含排队判定）下 blocking 池；任务体本身由 `spawn_job` 带走。
pub(super) async fn project_data_event_cut(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<crate::auth::CurrentUser>,
    scope: Scope,
    AxumPath(rank): AxumPath<String>,
) -> ApiResult {
    let (ws, st, uid, pid) = (scope.ws, state.clone(), user.id, scope.project.id);
    blocking(move || block_on_here(event_cut_impl(&ws, &rank, &st.jobs, uid, pid))).await
}

pub(super) async fn project_data_event_cut_all(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<crate::auth::CurrentUser>,
    scope: Scope,
    Json(body): Json<serde_json::Value>,
) -> ApiResult {
    let (ws, st, uid, pid) = (scope.ws, state.clone(), user.id, scope.project.id);
    blocking(move || block_on_here(event_cut_all_impl(&ws, &body, &st.jobs, uid, pid))).await
}

/// `POST /api/project-data/events/merge-splits` —— 把历史上被等分的
/// `xxx（1/2）`/`（2/2）` 合回一个事件（前端二次确认后调用）。
pub(super) async fn project_data_events_merge_splits(scope: Scope, State(state): State<Arc<AppState>>) -> ApiResult {
    let ws = scope.ws;
    let path = ws.output_dir().join("events.json");
    // 同步写 events.json（没法排队）：有任务正在写同批产物时当场拒绝，别对撞
    if let Some(busy) = state
        .jobs
        .busy_with(&[crate::scope_key::project_out(scope.project.id)])
        .await
    {
        tracing::info!("拒绝在 {} 上合并拆分事件：{busy}", busy.resource);
        return Err(err(StatusCode::CONFLICT, format!("{busy}（正在读写切片产物），请稍后再试")));
    }
    let _guard = yekitrim_core::filelock::lock_path(&path).await;
    blocking(move || data_events_merge_splits_impl(&ws)).await
}

pub(super) async fn project_data_clip_silence(scope: Scope, State(state): State<Arc<AppState>>, AxumPath(rank): AxumPath<String>) -> ApiResult {
    // 隐式初始化精修工程 + 扫描静音 + 写屏蔽轨道：整段同步 IO，下 blocking 池
    let (ws, st, pid) = (scope.ws, state.clone(), scope.project.id);
    blocking(move || block_on_here(clip_silence_impl(&ws, &rank, &st, pid))).await
}

/// `DELETE /api/project-data/clips/:rank` —— 删除该切片目录（前端二次确认后调用）。
/// 路径参数接受数字 rank（`2` → `clip-02`）或目录名（`clip-out`），见 `resolve_clip_dir`。
pub(super) async fn project_data_clip_delete(scope: Scope, State(state): State<Arc<AppState>>, AxumPath(rank): AxumPath<String>) -> ApiResult {
    // 删目录 + 重写发布索引都是对项目目录的同步 IO（可能在网络盘上）：整段下 blocking 池
    let (ws, pid) = (scope.ws, scope.project.id);
    blocking(move || block_on_here(clip_delete_impl(&ws, &rank, &state, pid))).await
}

pub(super) async fn project_data_correct(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<crate::auth::CurrentUser>,
    scope: Scope,
) -> ApiResult {
    correct_run_impl(&scope.ws, &state.jobs, user.id, scope.project.id).await
}

pub(super) async fn project_data_pipeline(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<crate::auth::CurrentUser>,
    scope: Scope,
    Json(body): Json<serde_json::Value>,
) -> ApiResult {
    // 作用域由 `Scope` 解析（`?project=` / 过渡期 `?config=`）；
    // 旧前端还会在 body 里带 `config`，那只是冗余字段，不再作为作用域来源。
    let ws = scope.ws;
    // 配额预检（review B7/D14）：转写/检测/剪切的产物也占账号配额 ——
    // 已超配额的普通账号不再放进新任务（admin 不受限）。
    quota_gate(&state, &user, Some(scope.project.id)).await?;
    if let Some(stage) = body.get("stage").and_then(|v| v.as_str()) {
        // 转写**每个模式只跑一次**：ASR 产物按模式占槽（`[asr] mode` 只有 hybrid / single
        // 两种，见 `yekitrim_service::assets`），该模式已经出过产物就不再提供重跑入口。
        // 想换字幕走素材列表（`POST /api/project-data/assets`）里已有的那份。
        // ⚠️ 只拦 HTTP 这一侧：CLI `yekitrim transcribe` 直接调 `core_pipeline::run_stage`，
        // 仍然可以强制重跑（转写中途崩掉、只留了半份产物时它是逃生口）。
        if stage == "transcribe" {
            let cfg = ws.load_config().map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}")))?;
            let mode = cfg.asr.mode.trim().to_string();
            // 模式串空（配置被清掉）= 没选模式：退回老口径"有转写就不给重跑"
            let bound = if mode.is_empty() {
                ws.transcript_path(&cfg).is_some()
            } else {
                yekitrim_service::assets::slot_bound(&ws, &cfg, yekitrim_service::assets::AssetKind::Transcript, &mode)
            };
            if bound {
                return Err(err(
                    StatusCode::CONFLICT,
                    if mode.is_empty() {
                        "该工程的转写已完成，不能再重跑；要换字幕请用素材列表里的那份".to_string()
                    } else {
                        format!("「{mode}」模式的转写已完成，不能再重跑；要换字幕请用素材列表里的那份")
                    },
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

/// `GET /api/project-data/config?project=<id>`：**某个工程**的配置。
///
/// 与 `GET /api/config`（纯程序配置）分开是本目录的既有一刀：那道接口编辑的是
/// `.yekitrim/config.toml` / `llm.toml`，这道回的是**这个工程的** `yekitrim.toml` 原文 + 合并后的生效配置。
/// 项目设置页的源码编辑器读 `yekitrim_raw`（`ProjectSettingsView.vue`），预处理页读 `config.asr.mode`。
pub(super) async fn project_data_config(scope: Scope) -> ApiResult {
    let ws = scope.ws;
    blocking(move || config_yekitrim_get_impl(&ws)).await
}

/// `GET /api/project-data/assets`：当前弹幕/字幕 + **素材清单** + 项目内的候选 + ASR 槽位。
///
/// 候选一次扫完（项目根 + 一层子目录），预处理页打开弹窗即可直接用。
/// 只 stat、不解析文件内容（3 小时录播的弹幕 XML 有几十 MB），所以这里不需要担心耗时。
///
/// `slots` 是**每个 ASR 模式是否已经出过产物**：它决定转写入口置灰。
/// 前端不去自己从 `items` 推 —— 判槽还要看盘上的分模式产物（老工程没登记过），
/// 那是服务端的口径（`assets::slot_bound`），抄一份到前端就会漂。
pub(super) async fn project_data_assets(scope: Scope) -> ApiResult {
    let ws = scope.ws;
    let source_kind = scope.project.source_kind.clone();
    let source_label = scope.project.source_label.clone().unwrap_or_default();
    let room = scope.project.room.clone();
    blocking(move || assets_impl(&ws, &source_kind, &source_label, room.as_deref())).await
}

/// `POST /api/project-data/assets`：改一类素材的清单 / 生效值。
///
/// - `{kind, path|dir+name}` → 选它（进清单 + 置为生效值）
/// - `{kind, path: ""}` → 清除生效值（清单不动，随时能切回来）
/// - `{kind, action: "remove", path}` → 从清单里删掉这条
///
/// 写前**解析一遍**（解析失败不落盘），成功返回条数供前端提示。
pub(super) async fn project_data_assets_put(
    scope: Scope,
    Extension(user): Extension<crate::auth::CurrentUser>,
    Json(body): Json<serde_json::Value>,
) -> ApiResult {
    let ws = scope.ws;
    let is_admin = user.is_admin();
    // 与设置页、去静音等写同一份 yekitrim.toml：读改写必须上路径锁
    let path = ws.config_file.clone();
    let _guard = yekitrim_core::filelock::lock_path(&path).await;
    blocking(move || assets_put_impl(&ws, &body, is_admin)).await
}

pub(super) fn assets_impl(
    ws: &Workspace,
    source_kind: &str,
    source_label: &str,
    room: Option<&str>,
) -> ApiResult {
    use yekitrim_service::assets::{asset_status, scan_candidates, slot_bound, AssetKind};
    let cfg = ws.load_config().map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}")))?;
    let mut candidates = serde_json::Map::new();
    for kind in [AssetKind::Danmaku, AssetKind::Transcript] {
        let list = scan_candidates(ws, &cfg, kind);
        candidates.insert(
            kind.as_str().to_string(),
            serde_json::to_value(list).unwrap_or(serde_json::Value::Array(vec![])),
        );
    }
    let mut slots = serde_json::Map::new();
    for mode in yekitrim_core::config::ASR_MODES {
        slots.insert(
            mode.to_string(),
            serde_json::Value::Bool(slot_bound(ws, &cfg, AssetKind::Transcript, mode)),
        );
    }
    ok(serde_json::json!({
        "danmaku": asset_status(ws, &cfg, AssetKind::Danmaku),
        "transcript": asset_status(ws, &cfg, AssetKind::Transcript),
        "source": { "kind": source_kind, "label": source_label, "room": room },
        "candidates": candidates,
        "slots": slots,
        "root_dir": yekitrim_core::path::display(&ws.root_dir),
    }))
}

pub(super) fn assets_put_impl(ws: &Workspace, body: &serde_json::Value, is_admin: bool) -> ApiResult {
    use yekitrim_service::assets::{asset_status, remove_material, set_asset, AssetKind};
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
    // 删清单条目：`remove_material` 自带"ASR 产物（文件还在）不给删"的守卫
    let removing = body
        .get("action")
        .and_then(|v| v.as_str())
        .map(|a| a.eq_ignore_ascii_case("remove"))
        .unwrap_or(false);
    if removing {
        remove_material(ws, kind, &path)
            .map_err(|e| err(StatusCode::BAD_REQUEST, format!("{e:#}")))?;
        let cfg = ws.load_config().map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}")))?;
        return ok(serde_json::json!({
            "ok": true,
            "kind": kind.as_str(),
            "removed": true,
            "value": "",
            "count": 0,
            "status": asset_status(ws, &cfg, kind),
        }));
    }
    let (value, count) = set_asset(ws, kind, &path, is_admin)
        .map_err(|e| err(StatusCode::BAD_REQUEST, format!("{e:#}")))?;
    let cfg = ws.load_config().map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}")))?;
    ok(serde_json::json!({
        "ok": true,
        "kind": kind.as_str(),
        "removed": false,
        "value": value,
        "count": count,
        "status": asset_status(ws, &cfg, kind),
    }))
}

pub(super) async fn project_data_config_put(
    scope: Scope,
    Extension(user): Extension<crate::auth::CurrentUser>,
    Json(body): Json<serde_json::Value>,
) -> ApiResult {
    let ws = scope.ws;
    let is_admin = user.is_admin();
    // 读改写加锁：设置页可能同时提交源文件与其它字段
    let path = ws.config_file.clone();
    let _guard = yekitrim_core::filelock::lock_path(&path).await;
    blocking(move || config_yekitrim_put_impl(&ws, &body, is_admin)).await
}

pub(super) fn data_transcript_impl(ws: &Workspace) -> ApiResult {
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
                let t = yekitrim_core::srt::parse_srt(&raw).map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("解析失败: {e}")))?;
                ok(serde_json::to_value(t).unwrap_or_default())
            }
        }
        None => Err(err(StatusCode::NOT_FOUND, "暂无转写文件")),
    }
}

/// 两个路径是否指同一个文件（canonicalize 兜住 `./` 与大小写；文件还不存在时退化成字面比较）。
pub(super) fn same_path(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
}

pub(super) fn data_transcript_put_impl(ws: &Workspace, body: &serde_json::Value) -> ApiResult {
    let cfg = ws.load_config().map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}")))?;
    let output = ws.output_dir();
    let configured = cfg
        .project
        .transcript_file
        .as_ref()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let segments = body.get("segments").cloned().ok_or_else(|| err(StatusCode::BAD_REQUEST, "缺少 segments"))?;
    let segs: Vec<yekitrim_core::types::TranscriptSegment> = serde_json::from_value(segments).map_err(|e| err(StatusCode::BAD_REQUEST, format!("segments 解析失败: {e}")))?;
    let transcript = yekitrim_core::types::Transcript { segments: segs, language: body.get("language").and_then(|l| l.as_str()).unwrap_or("zh").to_string(), asr_metadata: None };

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
            yekitrim_core::srt::write_srt(&transcript, target).map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("写字幕失败 {}: {e}", target.display())))?;
        }
    }
    let paths: Vec<String> = targets.iter().map(|p| yekitrim_core::path::display(p)).collect();
    ok(serde_json::json!({ "ok": true, "saved": transcript.segments.len(), "paths": paths }))
}

/// 统计 SRT 文件中的字幕条数（以时间轴行 `-->` 为准）；文件缺失/编码异常时为 0。
pub(super) fn count_srt_cues(path: &Path) -> u64 {
    match std::fs::read(path) {
        Ok(bytes) => String::from_utf8_lossy(&bytes).lines().filter(|l| l.contains("-->")).count() as u64,
        Err(_) => 0,
    }
}

/// 读取项目转写文件（`transcript_file` / `output/transcript.json` / `output/video.srt`）
/// 中所有字幕段的起始秒，升序。
pub(super) fn transcript_starts(ws: &Workspace) -> Vec<f64> {
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
        yekitrim_core::srt::parse_srt(&raw).map(|t| t.segments.iter().map(|s| s.start).collect()).unwrap_or_default()
    };
    starts.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    starts
}

/// 读取项目弹幕文件（XML / ASS / SRT）中所有弹幕的起始秒，升序。
pub(super) fn danmaku_starts(ws: &Workspace) -> Vec<f64> {
    let Ok(cfg) = ws.load_config() else { return Vec::new() };
    let Some(p) = ws.danmaku_path(&cfg) else { return Vec::new() };
    let Ok(entries) = yekitrim_core::danmaku::load_danmaku_auto(&p) else { return Vec::new() };
    let mut starts: Vec<f64> = entries.iter().map(|e| e.start).collect();
    starts.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    starts
}

/// 读取项目弹幕文件里的**弹幕正文**（升序，去掉空行）—— 首页切片总览的"搜弹幕"用。
///
/// ⚠️ 复用 `danmaku_starts` 那条**同一条解析路径**（`Workspace::danmaku_path` +
/// `load_danmaku_auto`，ASS / 录播姬 XML / SRT 都认）：换一条自己拼路径的读法就会
/// 重现"弹幕文件解析出来了、但正文读不到"那类静默失败。
///
/// ⚠️ 只回**正文**、不回时间：搜索是按关键词找片段，时间轴信息用不上；带上时间只会
/// 让载荷凭空翻几倍（实测单项目弹幕文件 4~72 KB）。
pub(super) fn danmaku_texts(ws: &Workspace) -> Vec<String> {
    let Ok(cfg) = ws.load_config() else { return Vec::new() };
    let Some(p) = ws.danmaku_path(&cfg) else { return Vec::new() };
    let Ok(entries) = yekitrim_core::danmaku::load_danmaku_auto(&p) else { return Vec::new() };
    let mut rows: Vec<(f64, String)> = entries
        .into_iter()
        .filter(|e| !e.text.trim().is_empty())
        .map(|e| (e.start, e.text))
        .collect();
    rows.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
    rows.into_iter().map(|(_, t)| t).collect()
}

/// 弹幕正文（首页切片总览「搜弹幕」按需拉；`{lines: [...]}`）。
///
/// 404 当空态处理由前端负责 —— 这里弹幕文件没配就回空数组，不报错（跟"没配弹幕"是一回事）。
pub(super) async fn project_data_danmaku(scope: Scope) -> ApiResult {
    let ws = scope.ws;
    blocking(move || ok(serde_json::json!({ "lines": danmaku_texts(&ws) }))).await
}

/// **搜索用文本**（首页切片总览的搜索框按需拉）：一次回**字幕 + 弹幕**两桶。
///
/// 一次回两桶而不是两个接口：勾上"字幕+弹幕"就是 12 个项目各一发，两桶分开就是 24 发 ——
/// 而且两边的失效时机完全一样（同一个任务结束刷新）。
///
/// ⚠️ **字幕桶要含 LLM 订正稿**（`output/video_llm.srt`，存在才读）：
/// 订正稿能救回原稿听错的词（实测"男友粉"那句只有订正稿完整，见 `.dsh/skills/yekitrim-topic-scan`）。
/// 两个都塞进同一个桶（前端不区分来源，只按文本匹配）；不做去重 —— 同一条话原稿/订正稿
/// 各存一份是正常的，去重反而要按时间对齐、徒增复杂度。
///
/// ⚠️ 只回**文本**、不回时间：搜索是按关键词找片段，时间轴信息用不上（带上会让载荷翻几倍）。
pub(super) fn search_texts(ws: &Workspace) -> serde_json::Value {
    let subs = subtitles_for_search(ws);
    ok_json_texts(subs, danmaku_texts(ws))
}

/// 两桶拼成 `{subs: [...], dm: [...]}`。
fn ok_json_texts(subs: Vec<String>, dm: Vec<String>) -> serde_json::Value {
    serde_json::json!({ "subs": subs, "dm": dm })
}

/// 字幕桶 = **生效转写稿** + **LLM 订正稿**（两者可能是同一个文件，那时只算一份）。
///
/// ⚠️ **生效转写稿未必是 JSON**：`transcript_file` 实测有三种取值 ——
/// `output/transcript.json`（最常见）、`output/video.srt`（订正/导出后的 SRT）、
/// `output/video_llm.srt`（**订正稿直接被登记成生效稿**）。只按 JSON 解会把 p2/p3 判成
/// "这个项目没有字幕"（实测 1018 / 2002 段全丢）—— 那是比"搜不到"更糟的静默错误。
/// 所以按**扩展名**分派解析。
///
/// ⚠️ 订正稿与生效稿**是同一个文件时不重复加**：同一条话存两份会让"命中 N 条"翻倍。
fn subtitles_for_search(ws: &Workspace) -> Vec<String> {
    let mut out = Vec::new();
    let mut seen: Option<PathBuf> = None;
    if let Ok(cfg) = ws.load_config() {
        if let Some(p) = ws.transcript_path(&cfg) {
            out.extend(read_subtitle_file(&p));
            seen = Some(p);
        }
    }
    // 订正稿：只有它**不是**刚才那份生效稿时才补（否则同内容算两遍）
    let llm = ws.output_dir().join("video_llm.srt");
    if seen.as_deref() != Some(llm.as_path()) {
        out.extend(read_subtitle_file(&llm));
    }
    out
}

/// 读一份字幕文件（`.json` 走 `segments[].text`，其余按 SRT 正文剥）。
///
/// 认不出的扩展名**按 SRT 试一遍**：字幕文件本质上只有这两种形态，而"读不到就当没有"
/// 正是这条路上最容易出的静默错误。
fn read_subtitle_file(path: &std::path::Path) -> Vec<String> {
    let Ok(raw) = std::fs::read_to_string(path) else { return Vec::new() };
    let is_json = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.eq_ignore_ascii_case("json"))
        .unwrap_or(false);
    if is_json {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&raw) {
            return v
                .get("segments")
                .and_then(|s| s.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|s| s.get("text").and_then(|t| t.as_str()))
                        .map(|t| t.trim().to_string())
                        .filter(|t| !t.is_empty())
                        .collect()
                })
                .unwrap_or_default();
        }
        return Vec::new();
    }
    srt_body_lines(&raw)
}

/// 从 SRT 正文里剥出可读文本：去掉序号行与时间行，丢掉只含标记的行。
///
/// ⚠️ 与前端 `utils.js::dmPlain` 那条是**两回事**（那条摘 ASS 覆写标签）：SRT 只需要
/// 跳过 `1` / `00:00:01,000 --> 00:00:03,000` 这两类行，别把正文里的数字一起误杀。
/// ⚠️ **先剥 BOM**：实测这几个 `video_llm.srt` 开头都是 `\u{feff}1`，
/// 不剥的话第一行的序号解析不成数字 → 那个 `1` 会被当成正文进搜索池。
fn srt_body_lines(raw: &str) -> Vec<String> {
    let raw = raw.trim_start_matches('\u{feff}');
    let mut out = Vec::new();
    for line in raw.lines() {
        let t = line.trim().trim_start_matches('\u{feff}').trim();
        if t.is_empty() || t.contains("-->") || t.parse::<u32>().is_ok() {
            continue;
        }
        out.push(t.to_string());
    }
    out
}

/// 搜索文本（`{subs: [...], dm: [...]}`）。
pub(super) async fn project_data_search_text(scope: Scope) -> ApiResult {
    let ws = scope.ws;
    blocking(move || ok(search_texts(&ws))).await
}

/// 单个路径的 mtime（unix 秒）；取不到（不存在 / 权限 / 时钟怪值）时是 0。
fn mtime_secs(path: &Path) -> u64 {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// 目录「最后一次改动」的 unix 秒 = 目录本身与**最多两层**子项里最新的 mtime。
///
/// 首页「切片总览」按这个值倒序（最新改动的片段浮到最前），所以它要覆盖「用户刚动过这个片段」
/// 的几条路：剪切（`clip.mp4` / `clip.json`）、精修保存（`polish.toml` / `clip.srt`）、
/// 出片与封面（`vtuber-clip_N.mp4` / `cover_N.jpg` —— 都是**直接子项**），精修侧的数据文件
/// 在 `data/` 里（只多一层，且新增/删除会带起 `data/` 自己的 mtime）。
/// ⚠️ **不递归到底**：项目目录经常挂在网络盘上（RaiDrive / WebDAV），深扫一个项目几百个片段
/// 的代价不可控，而这个接口每次页面加载都会打。
fn last_touched_secs(dir: &Path) -> u64 {
    fn walk(path: &Path, depth: u8, best: &mut u64) {
        *best = (*best).max(mtime_secs(path));
        if depth == 0 {
            return;
        }
        let Ok(entries) = std::fs::read_dir(path) else { return };
        for entry in entries.flatten() {
            // `DirEntry::metadata()` 在 Windows 上直接用目录项里的数据，不额外 `stat` ——
            // 网络盘上这一条很要紧（这里每个片段要走一遍）。
            let Ok(meta) = entry.metadata() else { continue };
            if let Some(secs) = meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs())
            {
                *best = (*best).max(secs);
            }
            if depth > 1 && meta.is_dir() {
                walk(&entry.path(), depth - 1, best);
            }
        }
    }
    let mut best = 0;
    walk(dir, 2, &mut best);
    best
}

/// 一条切片的**剪辑质量分**（总方案 §3）；算不出来回 `None`（前端不显示那一栏）。
///
/// ⚠️ **三条"不算"的口径**（都是"宁可不显示，也不给一个错的数"）：
/// - 没有 `polish.toml` ⇒ 还没精修：没有分段/事件，算出来是"零干扰满分" —— **假的高分**；
/// - 没有 `data/subtitles.json` ⇒ 句子判据无源：只剩"爆点/间隔"两条在工作，
///   分数**偏高且不可比**（用户无从知道少算了一条）；
/// - 解析失败 ⇒ 同上。
///
/// 时长取 `clip.json` 的 `duration`（即 `duration_secs`）；它 ≤ 0 时退回"末段 end"——
/// 老片段可能没有 duration，退回后至少还能算出一个分数。
///
/// ⚠️ **下面有两道冗余闸**（`polish.toml` 存在性 + `load_project` 成功），**两道都要留**：
/// 它们现在**互相遮蔽** —— 单独去掉任一道，没精修的片段仍会因为另一道回 `None`，
/// 所以**变异自证必须两道一起拆**才翻红（实测：只拆一道时
/// `clips_回质量分_但没精修过的回_null` 照样绿，差点被误判成"测试没锁住"）。
/// 留两道是因为第一道是**语义**（"没精修过"）、第二道是**容错**（文件在但读坏了）——
/// 判据不同，不是重复。
fn clip_quality(dir: &Path, duration_secs: f64) -> Option<serde_json::Value> {
    if !dir.join("polish.toml").exists() {
        return None;
    }
    let subs_raw = std::fs::read_to_string(dir.join("data").join("subtitles.json")).ok()?;
    let subtitles: Vec<yekitrim_polish::parse::srt::Subtitle> = serde_json::from_str(&subs_raw).ok()?;
    let cfg = yekitrim_polish::project::load_project(dir).ok()?;
    let duration = if duration_secs > 0.0 {
        duration_secs
    } else {
        cfg.segments.last().map(|s| s.end).unwrap_or(0.0)
    };
    let q = yekitrim_polish::process::quality::evaluate(&cfg.segments, &subtitles, duration);

    // 逐条干扰也回给前端：切片页 tooltip 要写"哪儿扣的分" ——
    // 只回一个数的话用户没法判断"这个 62 分到底怎么了"（与 §1 同一套理由）。
    let items: Vec<serde_json::Value> = q
        .interruptions
        .iter()
        .map(|i| serde_json::json!({ "kind": i.kind.key(), "at": i.at, "label": i.kind.label() }))
        .collect();
    Some(serde_json::json!({
        // 展示分保留 1 位小数 —— 界面上就显示到这个精度，别传一串浮点尾数
        "score": (q.score() * 10.0).round() / 10.0,
        "smoothness": (q.smoothness() * 100.0).round() / 100.0,
        "per_minute": (q.interruptions_per_minute() * 100.0).round() / 100.0,
        "count": q.interruption_count(),
        "segments": q.segments,
        "visual_events": q.visual_events,
        "interruptions": items,
    }))
}

/// 统计落在 [start, end] 内的起始点数量（starts 必须升序）。
pub(super) fn count_in_range(starts: &[f64], start: f64, end: f64) -> u64 {
    if starts.is_empty() || end < start { return 0; }
    let lo = starts.partition_point(|&x| x < start);
    let hi = starts.partition_point(|&x| x <= end);
    hi.saturating_sub(lo) as u64
}

pub(super) fn data_events_impl(ws: &Workspace) -> ApiResult {
    let path = ws.output_dir().join("events.json");
    // 同 data_clips_impl：**还没检测过**是正常空态，不是错误 —— 返回空集合，
    // 让调用方渲染"还没有事件"的空态，而不是把整页判成加载失败。
    if !path.exists() {
        return ok(serde_json::json!({ "titles": {}, "events": [] }));
    }
    let raw = std::fs::read_to_string(&path).map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("读取失败: {e}")))?;
    let mut v: serde_json::Value = serde_json::from_str(&raw).unwrap_or(serde_json::Value::Null);
    // 事件区间内的弹幕量/字幕量（切片页排序用），文件缺失时为 0。
    // 只有存在未剪切事件时才解析源弹幕/字幕，避免纯浏览列表时反复解析大文件。
    let clips_dir = path.parent().map(|p| p.join("clips"));
    let has_uncut = v.get("events").and_then(|e| e.as_array()).map(|evs| evs.iter().any(|ev| {
        let dir = event_clip_dir_or_rank(ev);
        !clips_dir.as_ref().map(|c| c.join(dir).join("clip.mp4").exists()).unwrap_or(false)
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
            let dir = event_clip_dir_or_rank(ev);
            let clip_dir = path.parent().map(|p| p.join("clips").join(dir)).unwrap_or_default();
            ev["cut_exists"] = serde_json::json!(clip_dir.join("clip.mp4").exists());
            ev["danmaku_count"] = serde_json::json!(count_in_range(&dm_starts, start, end));
            ev["subtitle_count"] = serde_json::json!(count_in_range(&sub_starts, start, end));
        }
    }
    ok(v)
}

/// 从 events.json 的一个事件里取实际片段目录名：优先 `clip_dir`，旧数据回退 `clip-{rank:02}`。
pub(super) fn event_clip_dir(v: &serde_json::Value) -> Option<String> {
    let dir = v
        .get("clip_dir")
        .and_then(|d| d.as_str())
        .map(str::trim)
        .filter(|d| !d.is_empty())?;
    if yekitrim_core::cutting::clip_index::is_safe_component(dir) {
        Some(dir.to_string())
    } else {
        None
    }
}

pub(super) fn event_clip_dir_or_rank(v: &serde_json::Value) -> String {
    if let Some(dir) = event_clip_dir(v) {
        return dir;
    }
    let rank = v.get("rank").and_then(|r| r.as_u64()).unwrap_or(0);
    format!("clip-{rank:02}")
}

pub(super) fn ev_time_secs(v: &serde_json::Value, key: &str) -> f64 {
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
pub(super) fn sec_to_ts_value(sec: f64) -> serde_json::Value {
    let ts = yekitrim_core::types::Timestamp::from_seconds(sec.max(0.0));
    serde_json::json!({ "h": ts.h, "m": ts.m, "s": ts.s, "ms": ts.ms })
}

pub(super) fn data_events_create_impl(ws: &Workspace, body: &CreateEventBody) -> ApiResult {
    let path = ws.output_dir().join("events.json");
    if !path.exists() {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("创建目录失败: {e}")))?;
        }
        std::fs::write(&path, "{\"events\":[]}").map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("创建 events.json 失败: {e}")))?;
    }
    let label = body.label.trim().to_string();
    if label.is_empty() {
        return Err(err(StatusCode::BAD_REQUEST, "标题不能为空"));
    }
    // **多段入点/出点列表**（粗剪拓展；2026-10-06）：预处理页可以传
    // `ranges = [{start_sec, end_sec}, …]` —— 剪出来的是"分段剪、合并输出"的一条片段。
    // 传了就**以 ranges 为准**：`start/end` 拉成首段起、末段末（与 `Event::normalize_ranges` 同一条）。
    // ⚠️ 单段（1 条 ranges）与不传 ranges 完全等价 ⇒ 老的「标记入点/出点 → 切片」一个字节不变。
    let mut ranges: Vec<yekitrim_core::types::EventRange> = Vec::new();
    if !body.ranges.is_empty() {
        for r in &body.ranges {
            if let (Some(rs), Some(re)) = (r.start_sec, r.end_sec) {
                if re > rs {
                    ranges.push(yekitrim_core::types::EventRange::new(rs, re));
                }
            }
        }
        ranges.sort_by(|a, b| a.start.partial_cmp(&b.start).unwrap_or(std::cmp::Ordering::Equal));
        // ⚠️ **重叠/相接的段必须并掉**再落盘：手工标段时两行可以互相压住（项目 12 那条 4 段片段
        // 就是 `1625.7–1653.8` 里套着 `1632.0–1643.0`）—— 不去重 Cutter 会把同一段**剪两遍**
        // （`clip.mp4` 158.3s = 四段之和，其中 11s 重复），而 `duration` 也按"各段之和"多报一遍，
        // 界面上完全看不出来。与 `Event::normalize_ranges` 走**同一个函数**：只并
        // `next.start <= last.end`，**有内容的空隙照旧留着**（那是"分段剪、中间别的话题丢掉"的表达）。
        yekitrim_core::types::merge_overlapping_ranges(&mut ranges);
    }
    let (start_sec, end_sec) = if ranges.is_empty() {
        let start = body.start_sec.unwrap_or(0.0);
        let end = body.end_sec.unwrap_or(start + 10.0);
        (start, end)
    } else {
        (
            ranges.iter().map(|r| r.start).fold(f64::MAX, f64::min),
            ranges.iter().map(|r| r.end).fold(f64::MIN, f64::max),
        )
    };
    if end_sec <= start_sec {
        return Err(err(StatusCode::BAD_REQUEST, "结尾必须大于开头"));
    }
    let event_type = body.event_type.clone().unwrap_or_else(|| "other".to_string());
    let summary = body.summary.clone().unwrap_or_default();
    let raw = std::fs::read_to_string(&path).map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("读取失败: {e}")))?;
    let mut store: serde_json::Value = serde_json::from_str(&raw).unwrap_or_else(|_| serde_json::json!({ "events": [] }));
    if !store.get("events").and_then(|a| a.as_array()).is_some() {
        store["events"] = serde_json::json!([]);
    }
    let next_rank = store.get("events").and_then(|a| a.as_array()).map(|arr| {
        arr.iter().filter_map(|e| e.get("rank").and_then(|r| r.as_u64())).max().unwrap_or(0) + 1
    }).unwrap_or(1);
    // 手动事件也分配稳定 clip_dir：rank 只做热度名次，目录名用标题 slug，
    // 同时避开磁盘上的旧片段目录和其它事件已占用的 clip_dir。
    let mut taken: std::collections::BTreeSet<String> = yekitrim_core::cutting::clip_index::reconcile_index(&ws.output_dir())
        .clips
        .keys()
        .map(|d| d.to_ascii_lowercase())
        .collect();
    if let Some(arr) = store.get("events").and_then(|a| a.as_array()) {
        for e in arr {
            if let Some(dir) = event_clip_dir(e) {
                taken.insert(dir.to_ascii_lowercase());
            }
        }
    }
    let clip_dir = yekitrim_core::cutting::clip_index::make_clip_dir_name(&label, &taken, next_rank as u32);
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
        "clip_dir": clip_dir,
        "enabled": true,
    });
    // 多段才写 `ranges`（单段 = 老格式，多一个键会让老工具以为"有多段"）
    let mut event = event;
    if ranges.len() > 1 {
        event["ranges"] = serde_json::to_value(&ranges).unwrap_or(serde_json::Value::Null);
    }
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

pub(super) fn data_events_put_impl(ws: &Workspace, body: &serde_json::Value) -> ApiResult {
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
/// `yekitrim_core::detection::merge_split_events`）。只在真的合并了东西时写盘。
/// 注意：事件目录名是 `clips/clip-{rank:02}`，合并后组内其余 rank 消失，
/// 它们的旧目录不会被删（要用户自己删掉重切），合并后的事件 rank 沿用组内最热的那条。
pub(super) fn data_events_merge_splits_impl(ws: &Workspace) -> ApiResult {
    let path = ws.output_dir().join("events.json");
    if !path.exists() { return Err(err(StatusCode::NOT_FOUND, "暂无 events.json")); }
    let raw = std::fs::read_to_string(&path)
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("读取失败: {e}")))?;
    let mut store: serde_json::Value = serde_json::from_str(&raw)
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("解析 events.json 失败: {e}")))?;
    let arr = store.get("events").and_then(|v| v.as_array()).cloned().unwrap_or_default();
    let events: Vec<yekitrim_core::types::Event> = serde_json::from_value(serde_json::Value::Array(arr))
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("事件字段解析失败: {e}")))?;
    let before = events.len();
    let (merged, groups) = yekitrim_core::detection::merge_split_events(events);
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

pub(super) fn data_clips_impl(ws: &Workspace) -> ApiResult {
    let clips = ws.output_dir().join("clips");
    // ⚠️ 目录不存在 = **还没有片段**，是正常空态，不是"资源不存在"（2026-11-15 修）：
    // 以前回 404「暂无 clips」，而切片页把四个接口放在 Promise.all 里 ——
    // 一个 404 让整页 catch，连**已经读到的 events 也一起丢掉**，界面显示
    // 「切片数据加载失败：暂无 clips」+ 事件列表空。实测用户报的就是这个
    // （刚跑完「查找事件」、67 条事件明明在 events.json 里，只是还没剪片段）。
    if !clips.exists() { return ok(serde_json::json!([])); }
    // 发布标题（`publish_config.json` 的 `clips[].title`）：页面上那个「发布标题」框要显示
    // **投稿真正会用的那句**，而不是 `clip.json` 的原始标题 —— 改过标题的片段刷新后
    // 必须还是改过的那句（否则用户会以为没保存成功，再改一遍）。整份文件读一次即可。
    // ⚠️ 这里读的 `title` 是**纯标题**（切片产物不带主播名，2026-11-22 起）；
    // 名字在下面用 `naming` 实时合成。第二项是片段的覆盖值（`None` = 跟随项目级）。
    let publish_entries: std::collections::HashMap<String, (String, Option<String>)> =
        std::fs::read_to_string(ws.output_dir().join("publish_config.json"))
            .ok()
            .and_then(|raw| serde_json::from_str::<serde_json::Value>(&raw).ok())
            .and_then(|v| {
                v.get("clips").and_then(|c| c.as_array()).map(|arr| {
                    arr.iter()
                        .filter_map(|c| {
                            let dir = c.get("directory").and_then(|d| d.as_str())?;
                            let title = c.get("title").and_then(|t| t.as_str()).unwrap_or("");
                            let streamer = c
                                .get("streamer_name")
                                .and_then(|s| s.as_str())
                                .map(str::to_string);
                            Some((dir.to_string(), (title.to_string(), streamer)))
                        })
                        .collect()
                })
            })
            .unwrap_or_default();
    // 项目级主播名：产物标题的唯一来源（`[detection].vtuber_name`）。读不到就当空 ——
    // **绝不因为配置读失败而拦掉整个切片列表**（那是用户唯一的操作面）。
    let project_streamer = ws
        .load_config()
        .map(|c| c.detection.vtuber_name)
        .unwrap_or_default();
    let mut out = vec![];
    // 源弹幕的起始时间表：只有遇到「没有 clip.dm.srt 的老切片」时才解析一次（见下）
    let mut dm_starts: Option<Vec<f64>> = None;
    let entries = std::fs::read_dir(&clips).map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("读取失败: {e}")))?;
    for entry in entries.flatten() {
        if !entry.path().is_dir() { continue; }
        let dir_path = entry.path();
        let dir_name = entry.file_name().to_string_lossy().into_owned();
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
        let silenced = yekitrim_polish::project::is_silent(&dir_path);
        let mute_range_count = if dir_path.join("polish.toml").exists() { yekitrim_polish::project::mute_range_count(&dir_path) } else { 0 };
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
        // 剪辑质量分（总方案 §3）：**只在已初始化精修工程时**才算 —— 没有 `polish.toml`
        // 就没有分段/事件，算出来是"零干扰满分"，那是**假的高分**、比没有更糟。
        // 算不出来（工程缺失/字幕缺失/解析失败）一律 `null`，前端不显示那一栏。
        let quality = clip_quality(&dir_path, duration_secs);
        // 先取出来再进 `json!`：`dir_name` 在下面会被 `json!` 吃掉（`to_value` 按值），
        // 之后再 `get(&dir_name)` 就借不到了。
        // 纯标题（编辑框那一份）：发布索引优先，没有再回落到 `clip.json` 的标题。
        let (stored_title, streamer_override) = publish_entries
            .get(&dir_name)
            .cloned()
            .unwrap_or_default();
        let base_title = if stored_title.trim().is_empty() {
            meta.get("title")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim()
                .to_string()
        } else {
            stored_title
        };
        // 三态归一化（含"老数据里 title 已经带着 `【旧名】`"那段）：
        // `(正文, 生效的名字)` —— 规则只有 `yekitrim_core::naming` 一份。
        let (title_core, streamer_effective) = yekitrim_core::naming::resolve_streamer(
            streamer_override.as_deref(),
            &base_title,
            &project_streamer,
        );
        let publish_title = yekitrim_core::naming::streamer_title(&title_core, &streamer_effective);
        out.push(serde_json::json!({
            "dir": yekitrim_core::path::display(&dir_path),
            "name": dir_name,
            // 发布标题 = **合成后**的那句（显示/复制用；投稿送出去的也是它）
            "publish_title": publish_title,
            // 纯标题（编辑框的值；不含主播名）
            "publish_title_core": title_core,
            // 片段级覆盖：null = 跟随项目级；"" = 这段不用；其它 = 锁定
            "streamer_name": streamer_override,
            // 实际生效的名字（界面拿它显示「跟随中 · 某某」）
            "streamer_name_effective": streamer_effective,
            "video_exists": video_path.exists(),
            "size_bytes": size_bytes,
            "duration_secs": duration_secs,
            // 「最后一次改动」= 这个片段目录里最新的 mtime（unix 秒，见 `last_touched_secs`）：
            // 首页「切片总览」按它倒序，刚剪完 / 刚精修的片段排在最前面。
            "mtime": last_touched_secs(&dir_path),
            "silenced": silenced,
            "mute_range_count": mute_range_count,
            "silent_files": silent_files,
            "meta": meta,
            "subtitle_count": subtitle_count,
            "danmaku_count": danmaku_count,
            "quality": quality,
        }));
    }
    // 目录名不再保证是 `clip-NN`（新片段用标题 slug）：优先按 `clip.json` 的 rank 排，
    // 没有 rank 的老数据再尝试解析 `clip-NN`，最后按目录名兜底。
    let rank_of = |v: &serde_json::Value| -> u64 {
        if let Some(r) = v
            .get("meta")
            .and_then(|m| m.get("rank"))
            .and_then(|r| r.as_u64())
            .filter(|r| *r > 0)
        {
            return r;
        }
        v.get("name")
            .and_then(|n| n.as_str())
            .and_then(|n| n.strip_prefix("clip-"))
            .and_then(|r| r.parse::<u64>().ok())
            .unwrap_or(u64::MAX)
    };
    out.sort_by(|a, b| {
        rank_of(a)
            .cmp(&rank_of(b))
            .then_with(|| a["name"].as_str().unwrap_or("").cmp(b["name"].as_str().unwrap_or("")))
    });
    ok(serde_json::json!(out))
}

/// 「消音版」：把该切片的长静音段写入屏蔽轨道（polish.toml 的 mute_ranges）。
/// 不生成任何新文件，原始 clip.mp4 / clip.srt 保持不变。
pub(super) async fn clip_silence_impl(ws: &Workspace, rank: &str, state: &AppState, pid: i64) -> ApiResult {
    // rank 直接来自 URL：先解析成数字再拼路径（`clip-../../x` 会走出项目目录）
    let Ok(rank) = rank.parse::<u32>() else {
        return Err(err(StatusCode::BAD_REQUEST, "rank 无效"));
    };
    let clip_dir = ws.output_dir().join("clips").join(format!("clip-{:0>2}", rank));
    let video = clip_dir.join("clip.mp4");
    if !video.exists() { return Err(err(StatusCode::NOT_FOUND, "clip.mp4 不存在")); }
    // 会隐式初始化精修工程 + 写屏蔽轨道：该片段有任务在跑时当场拒绝
    if let Some(busy) = state.jobs.busy_with(&crate::scope_key::polish_keys(Some(pid), &clip_dir)).await {
        tracing::info!("拒绝在 {} 上改屏蔽轨道：{busy}", busy.resource);
        return Err(err(StatusCode::CONFLICT, format!("{busy}，请等它结束再改屏蔽轨道")));
    }
    ensure_polish_project(&clip_dir).await?;
    // polish.toml 的 read-modify-write 要和整修页的保存走**同一把**路径锁，否则
    // "编辑器保存设计" 与 "去静音" 并发时后写者会把先写者的改动整段盖掉。
    // 锁在 await 里持有；ffmpeg 静音扫描 + 写盘下 blocking 池。
    let _guard = yekitrim_core::filelock::lock_path(&clip_dir.join("polish.toml")).await;
    let applied = blocking(move || {
        yekitrim_polish::project::apply_silence(&clip_dir)
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
pub(super) fn resolve_clip_dir(output_dir: &Path, arg: &str) -> Result<PathBuf, AppError> {
    if !yekitrim_polish::assets::sticker_index::is_safe_component(arg) {
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
pub(super) async fn clip_delete_impl(ws: &Workspace, arg: &str, state: &AppState, pid: i64) -> ApiResult {
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
            crate::scope_key::project_clip(pid, &dir_name),
            crate::scope_key::dir_key(&clip_dir),
            crate::scope_key::project_index(pid),
        ])
        .await
    {
        tracing::info!("拒绝删除 {}：{busy}", busy.resource);
        return Err(err(StatusCode::CONFLICT, format!("{busy}，请等它结束再删这个片段")));
    }
    // 下面两步都是同步文件访问，一起下 blocking 池：`remove_dir_all` 删的是**含成片的整个片段目录**
    // （项目在网络盘上时最重的就是这一处），`prune_clip_from_index` 内部要读
    // `publish_config.json` / `clips_order.txt` 再重建 `index.json` —— 同样同步 IO，
    // 所以必须跟删除落在**同一个闭包**里，不能只搬一半。
    // ⚠️ `busy_with` 是 async，留在闭包外（见上）。
    let del_dir = clip_dir.clone();
    let del_name = dir_name.clone();
    let output_dir = ws.output_dir();
    blocking(move || {
        std::fs::remove_dir_all(&del_dir).map_err(|e| {
            err(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("删除 {del_name} 失败: {e}"),
            )
        })?;
        prune_clip_from_index(&output_dir, &del_name);
        Ok(())
    })
    .await?;
    tracing::info!("已删除片段目录: {}", clip_dir.display());
    ok(serde_json::json!({ "ok": true, "removed": dir_name }))
}

/// 把某个切片从 `publish_config.json` / `clips_order.txt` 里摘掉（删目录后的索引同步）。
/// 两个文件都是 Cutter 的产物，缺失或格式异常就静默跳过 —— 目录本身已经删掉了。
pub(super) fn prune_clip_from_index(output_dir: &Path, dir_name: &str) {
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
    // 目录已经删掉：从磁盘重建 index.json，让被删条目同步消失。
    if let Err(e) = yekitrim_core::cutting::clip_index::refresh_index(output_dir) {
        tracing::warn!("刷新片段索引失败（删除 {} 后）: {e:#}", dir_name);
    }
}

/// 发布标题的字数上限（B站标题 80 字，这里留足余量给「标题 + 主播名」）。
const MAX_TITLE_CHARS: usize = 200;

/// 改一个片段的**发布标题**（output/publish_config.json 的 clips[].title）。
///
/// 为什么写这一份、不写 clip.json：投稿链路（crate::publish → biliLive-tools）读的就是
/// publish_config.json，改标题的目的正是「投稿时用我改过的这句」。
/// ⚠️ **刻意不碰 clip.json 的 title**：它是片段的**身份** —— clip_index::clip_meta_matches
/// 用「标题相同 或 原片区间 IoU ≥ 0.5」判断重新检测时能不能复用这个目录；改了它，老片段在重新
/// 检测后会匹配不上、另开一个目录（精修产物留在旧目录里，用户看到的是「精修白做了」）。
///
/// 生命周期：重切**这个**片段会按 clip.json 重新生成标题（Cutter 的「同名覆盖」），重切**别的**
/// 片段不会动它 —— 后者由 publish_index_merges_incremental_cuts_* 钉着。
/// 片段的**主播名覆盖**改动意图（三态，别拍平成 `Option<String>` —— 那就分不出
/// "不动" 与 "清除覆盖"）。
pub(super) enum StreamerPatch {
    /// 这次只改标题，主播名保持原样。
    Keep,
    /// 清除覆盖 ⇒ 恢复跟随项目级（键直接删掉）。
    Follow,
    /// 设成这个值；空串 = 这个片段明确不用主播名。
    Set(String),
}

pub(super) fn clip_title_put_impl(
    ws: &Workspace,
    arg: &str,
    title: &str,
    streamer: &StreamerPatch,
) -> ApiResult {
    let output = ws.output_dir();
    let clip_dir = resolve_clip_dir(&output, arg)?;
    let Some(dir_name) = clip_dir.file_name().map(|n| n.to_string_lossy().into_owned()) else {
        return Err(err(StatusCode::BAD_REQUEST, "rank 无效"));
    };
    if !clip_dir.is_dir() {
        return Err(err(StatusCode::NOT_FOUND, format!("{dir_name} 不存在")));
    }
    let publish = output.join("publish_config.json");
    let raw = std::fs::read_to_string(&publish).map_err(|e| {
        err(
            StatusCode::NOT_FOUND,
            format!("还没有发布索引 publish_config.json（重新剪一次会重建）: {e}"),
        )
    })?;
    let mut v: serde_json::Value = serde_json::from_str(&raw).map_err(|e| {
        err(StatusCode::INTERNAL_SERVER_ERROR, format!("publish_config.json 解析失败: {e}"))
    })?;
    let mut hit = false;
    if let Some(arr) = v.get_mut("clips").and_then(|c| c.as_array_mut()) {
        for c in arr.iter_mut() {
            if c.get("directory").and_then(|d| d.as_str()) == Some(dir_name.as_str()) {
                c["title"] = serde_json::json!(title);
                // 主播名三态：Keep 一个字节都不动；Follow 删键；Set 写值。
                match streamer {
                    StreamerPatch::Keep => {}
                    StreamerPatch::Follow => {
                        if let Some(obj) = c.as_object_mut() {
                            obj.remove("streamer_name");
                        }
                    }
                    StreamerPatch::Set(v) => c["streamer_name"] = serde_json::json!(v),
                }
                hit = true;
            }
        }
    }
    if !hit {
        return Err(err(
            StatusCode::NOT_FOUND,
            format!("{dir_name} 还没进发布索引（重新剪一次会重建）"),
        ));
    }
    let pretty = serde_json::to_string_pretty(&v)
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("序列化失败: {e}")))?;
    std::fs::write(&publish, pretty).map_err(|e| {
        err(StatusCode::INTERNAL_SERVER_ERROR, format!("写入 publish_config.json 失败: {e}"))
    })?;
    sync_order_title(&output, &dir_name, title);
    ok(serde_json::json!({
        "ok": true,
        "directory": dir_name,
        "title": title,
        "streamer_name": match streamer {
            StreamerPatch::Keep => serde_json::Value::Null,
            StreamerPatch::Follow => serde_json::Value::Null,
            StreamerPatch::Set(v) => serde_json::json!(v),
        },
    }))
}

/// clips/clips_order.txt 每行「<目录名> <标题>」：标题跟着改，否则那份索引还念着旧标题。
/// 文件缺失/格式异常一律静默跳过（与 prune_clip_from_index 同一条口径）。
fn sync_order_title(output_dir: &Path, dir_name: &str, title: &str) {
    let order = output_dir.join("clips").join("clips_order.txt");
    let Ok(raw) = std::fs::read_to_string(&order) else { return };
    // ⚠️ 分隔符必须与 Cutter 写的一致（目录名 + ' / ' + 标题，见 cutting/mod.rs 的 order_lines）：
    // 写成空格虽然仍能被 starts_with(目录名 + ' ') 命中，但同一份文件出现两种格式，人看像两份东西。
    let prefix = format!("{dir_name} ");
    let want = if title.trim().is_empty() {
        dir_name.to_string()
    } else {
        format!("{dir_name} / {title}")
    };
    let mut changed = false;
    let lines: Vec<String> = raw
        .lines()
        .map(|line| {
            let t = line.trim();
            if t == dir_name || t.starts_with(&prefix) {
                changed |= t != want;
                want.clone()
            } else {
                line.to_string()
            }
        })
        .collect();
    if changed {
        let _ = std::fs::write(&order, lines.join("\n"));
    }
}

/// `PUT /api/project-data/clips/:rank/title` 的请求体。
#[derive(serde::Deserialize)]
pub(super) struct ClipTitleBody {
    /// 发布标题。**必填**（空串 = 清掉标题，与前端 `setClipTitle` 同口径）。
    pub title: String,
    /// 主播名覆盖，**三态**直接映射成 `StreamerPatch`（键缺席 = Keep / `null` = Follow /
    /// 字符串 = Set）。见下面那个 `Deserialize` impl。
    #[serde(default)]
    pub streamer_name: StreamerPatch,
}

impl Default for StreamerPatch {
    fn default() -> Self {
        StreamerPatch::Keep
    }
}

/// **三态反序列化**（`#[serde(default)]` 落到 `Keep`）：键缺席 = 不动；`null` = 跟随项目级；
/// 字符串 = 锁定（空串 = 这段不用）；其它类型 ⇒ 400 且文案点名 `streamer_name`。
///
/// ⚠️ **不能**写成 `Option<serde_json::Value>`：serde 的 `Option<T>` 会把 `null` 解析成 `None`，
/// 于是"键缺席"与"显式 null"混成一档 —— 而这两个语义**相反**（一个不动、一个清掉用户的覆盖）。
/// 实测（2026-11-24）：写成 Option 后 ⑤「清回 null ⇒ 恢复跟随」那条老用例立刻红。
impl<'de> serde::Deserialize<'de> for StreamerPatch {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        match serde_json::Value::deserialize(d)? {
            serde_json::Value::Null => Ok(StreamerPatch::Follow),
            serde_json::Value::String(s) => Ok(StreamerPatch::Set(s.trim().to_string())),
            _ => Err(serde::de::Error::custom(
                "streamer_name 必须是字符串或 null",
            )),
        }
    }
}

/// PUT /api/project-data/clips/:rank/title —— 改发布标题（正文见 clip_title_put_impl）。
pub(super) async fn project_data_clip_title_put(
    scope: Scope,
    AxumPath(rank): AxumPath<String>,
    Json(body): Json<serde_json::Value>,
) -> ApiResult {
    let ws = scope.ws;
    let ClipTitleBody { title, streamer_name } = parse_body(body)?;
    let title = title.trim().to_string();
    let len = title.chars().count();
    if len > MAX_TITLE_CHARS {
        return Err(err(
            StatusCode::BAD_REQUEST,
            format!("标题最多 {MAX_TITLE_CHARS} 字（当前 {len}）"),
        ));
    }
    // 主播名的改动意图（三态，见 `yekitrim_core::naming`）：**结构体层的 `Deserialize` impl
    // 已经把它映射好了**（缺席 = Keep / null = Follow / 字符串 = Set / 其它 = 400 点名），
    // 这里不用再 match 一遍 —— 两处判据各写一份必然漂。
    let patch = streamer_name;
    // publish_config.json 是 read-modify-write（Cutter 也在写它）：与它走同一条路径锁
    let path = ws.output_dir().join("publish_config.json");
    let _guard = yekitrim_core::filelock::lock_path(&path).await;
    blocking(move || clip_title_put_impl(&ws, &rank, &title, &patch)).await
}

/// 精修工程不存在时自动初始化（与前端进入精修时的行为一致）。
///
/// 实现在 `yekitrim_service::init_polish_for_clip`（**唯一一份**）：剪切收尾也用同一条路，
/// 这里只把错误映射成 HTTP。`Ok(false)` = 已经有工程（或目录里没有 clip.mp4）= 无事可做。
pub(crate) async fn ensure_polish_project(dir: &Path) -> Result<(), AppError> {
    match yekitrim_service::init_polish_for_clip(dir).await {
        Ok(_) => Ok(()),
        Err(e) => Err(err(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("初始化精修工程失败: {e:#}"),
        )),
    }
}

pub(super) async fn event_cut_all_impl(
    ws: &Workspace,
    body: &serde_json::Value,
    jobs: &Arc<crate::jobs::JobManager>,
    owner: i64,
    project: i64,
) -> ApiResult {
    let ev_path = ws.output_dir().join("events.json");
    if !ev_path.exists() { return Err(err(StatusCode::NOT_FOUND, "暂无 events.json")); }
    let raw = tokio::task::spawn_blocking({ let __p = ev_path.clone(); move || std::fs::read_to_string(__p) }).await.unwrap_or_else(|_| Err(std::io::Error::other("读取任务崩了"))).map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("读取失败: {e}")))?;
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
        let clip_dir = output.join("clips").join(event_clip_dir_or_rank(ev));
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
    let keys = crate::scope_key::cut_keys(project, &output, None);
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
        // 被磁盘预检挡下的条数（含触发那条）：这不是"剪切失败"，是**没让剪** ——
        // 阶段预检（runner）对每条 run_single_cut 都会先查 `[cutting] min_free_gb`，
        // 一旦不达标整批就此打住，剩余条目留给"腾出空间后重跑"自动补剪。
        let mut blocked = 0usize;
        let mut done = 0u64;
        for handle in handles {
            match handle.await {
                Ok((_, Ok(()))) => {}
                Ok((rank, Err(e))) => {
                    if e.downcast_ref::<yekitrim_core::disk::FreeSpaceError>().is_some() {
                        blocked += 1;
                    } else {
                        failures += 1;
                        let _ = jobs_handle.log(&job_id, "error", format!("clip-{:02} 剪切失败: {e:#}", rank)).await;
                    }
                }
                Err(_) => failures += 1,
            }
            done += 1;
            jobs_handle.progress(&job_id, done, total).await;
        }
        if blocked > 0 {
            let ok = total.saturating_sub(failures as u64 + blocked as u64);
            jobs_handle
                .fail(
                    &job_id,
                    format!(
                        "磁盘空间不足，已停止剪切：成功 {ok} 条、失败 {failures} 条、{blocked} 条未处理；腾出空间后重新「一键全切」会自动补剪缺失的片段（只剪还没有 clip.mp4 的目录）"
                    ),
                )
                .await;
        } else if failures == 0 {
            jobs_handle.complete(&job_id).await;
        } else {
            jobs_handle.fail(&job_id, format!("{} 个事件剪切失败", failures)).await;
        }
    });
    ok(serde_json::json!({ "ok": true, "id": id, "queued": queued }))
}

pub(super) async fn run_single_cut(config_file: &Path, ev: serde_json::Value) -> anyhow::Result<()> {
    run_single_cut_event(config_file, serde_json::from_value(ev)?).await
}

/// 同上，但调用方已经拿着解析好的事件（单条重切走这条，省一次 JSON 往返）。
pub(super) async fn run_single_cut_event(config_file: &Path, ev: yekitrim_core::types::Event) -> anyhow::Result<()> {
    let cfg = yekitrim_core::config::load_config_at(config_file)?;
    let w = yekitrim_service::Workspace::load(Some(config_file))?;
    let input = cfg.project.input_file.as_ref().filter(|s| !s.is_empty()).map(|s| w.resolve(s))
        .ok_or_else(|| anyhow::anyhow!("未初始化 input_file"))?;
    let output = w.output_dir();
    let mut ctx = yekitrim_core::types::PipelineContext::new(input, output);
    ctx.events = vec![ev];
    let mut r = yekitrim_core::pipeline::PipelineRunner::new(cfg);
    r.add_stage(Box::new(yekitrim_core::cutting::Cutter));
    r.run(&mut ctx).await?;
    // **剪切收尾 = 补精修工程**（用户口径：剪切这个行为本身包含精修 init）。
    // 单条剪切与一键全切都走这里（`event_cut_all_impl` → `run_single_cut`），
    // 于是切片页的「精修」按钮、首页切片总览、`polish_ready` 判据进来时工程已经在了，
    // 不再有"发完 init 就跳转、页面先读到 polish.toml not found"的竞态。
    // 失败只 warn：剪切已经成功，进入精修时还有 `ensure_polish_project` 兜底。
    // 收尾进度传 None：单条收尾秒级（cut_all 的逐条进度由调用方循环统一报，
    // 不会在这里出现 >120s 的无事件窗口 —— spec §1 P2 口径核对过）。
    yekitrim_service::init_polish_for_events(&ctx.output_dir, &ctx.events, None).await;
    Ok(())
}

pub(super) async fn event_cut_impl(
    ws: &Workspace,
    rank: &str,
    jobs: &Arc<crate::jobs::JobManager>,
    owner: i64,
    project: i64,
) -> ApiResult {
    let ev_path = ws.output_dir().join("events.json");
    if !ev_path.exists() { return Err(err(StatusCode::NOT_FOUND, "暂无 events.json")); }
    let raw = tokio::task::spawn_blocking({ let __p = ev_path.clone(); move || std::fs::read_to_string(__p) }).await.unwrap_or_else(|_| Err(std::io::Error::other("读取任务崩了"))).map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("读取失败: {e}")))?;
    let store: serde_json::Value = serde_json::from_str(&raw).map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("解析失败: {e}")))?;
    let rank_u: u32 = rank.parse().map_err(|_| err(StatusCode::BAD_REQUEST, "rank 无效"))?;
    let event: serde_json::Value = store.get("events").and_then(|a| a.as_array()).and_then(|arr| arr.iter().find(|e| e.get("rank").and_then(|r| r.as_u64()).unwrap_or(0) as u32 == rank_u).cloned())
        .ok_or_else(|| err(StatusCode::NOT_FOUND, "事件不存在"))?;
    let clip_dir = event_clip_dir_or_rank(&event);
    let ev: yekitrim_core::types::Event = serde_json::from_value(event).map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("事件解析失败: {e}")))?;
    // 资源域含这个片段目录：单条重切会写 clip.mp4/clip.json，与同片段的精修互斥（排队）。
    let keys = crate::scope_key::cut_keys(project, &ws.output_dir(), Some(clip_dir.as_str()));
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
/// 可供浏览的**卷/盘符**（给"服务器文件"浏览器顶部的盘符入口）。
///
/// ⚠️ 这只是**展示用的候选**，**不构成访问限制** —— `/api/fs/list` 仍然允许 admin 浏览任意绝对路径，
/// `[storage] local_roots` 也仍然只是快捷入口。没有它的话，浏览器的目录树只能从"当前路径所在那个盘"
/// 往上展开（`parent` 到盘根就断了），用户**换不了盘**。
///
/// Windows 枚举 `A:`–`Z:` 里真实存在的；其它平台只给文件系统根 `/`。
fn available_volumes() -> Vec<serde_json::Value> {
    #[cfg(windows)]
    {
        let mut out = Vec::new();
        for c in b'A'..=b'Z' {
            let letter = c as char;
            let root = format!("{letter}:\\");
            if std::path::Path::new(&root).is_dir() {
                out.push(serde_json::json!({ "path": root, "label": format!("{letter}:") }));
            }
        }
        out
    }
    #[cfg(not(windows))]
    {
        vec![serde_json::json!({ "path": "/", "label": "/" })]
    }
}

pub(super) async fn fs_list(
    Extension(user): Extension<crate::auth::CurrentUser>,
    Json(body): Json<serde_json::Value>,
) -> ApiResult {
    if !user.is_admin() {
        return Err(err(StatusCode::FORBIDDEN, "仅管理员可浏览服务器文件"));
    }
    // 允许的根 = 租户根 + [storage] local_roots（这些只是"快捷入口"；管理员可用任意绝对路径）
    //
    // ⚠️ 这里的"租户根"是**项目托管根** `<projects_root>/users/<uid>`（= `local_roots[0]`），
    // 不是按账号分的素材/配置根 `<data>/users/<uid>`（`owner::owner_root`）—— 两者是不同目录。
    // 用后者去建目录会把**默认浏览根**留在不存在的位置：`dir=""` 时 `pick_local_root` 取前者，
    // 随后 `!target.is_dir()` 直接回 400「只能浏览目录」。全新部署（还没有任何项目）必现，
    // 2026-10 的"服务器文件 / 浏览录像打不开"就是这么来的。
    let tenant = yekitrim_service::projects::tenant_root(user.id);
    std::fs::create_dir_all(&tenant)
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("创建租户目录失败: {e}")))?;
    let roots = yekitrim_service::projects::local_roots(user.id);
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
            yekitrim_service::projects::resolve_target_dir(user.id, root_hint, &d, is_admin)
                .map_err(bad)?
        }
        // 兼容契约：{root, path}
        None if Path::new(path_hint.trim()).is_absolute() => {
            yekitrim_service::projects::resolve_target_dir(user.id, root_hint, path_hint, is_admin)
                .map_err(bad)?
        }
        None => {
            let root = yekitrim_service::projects::pick_local_root(user.id, root_hint, is_admin)
                .map_err(bad)?;
            if path_hint.trim().is_empty() {
                root
            } else {
                yekitrim_service::projects::resolve_target_dir(user.id, "", path_hint, is_admin)
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
                    format!("目录不存在: {name}（在 {} 下）", yekitrim_core::path::display(&target)),
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
    let cur_str = yekitrim_core::path::display(&cur);
    // 绝对父目录：让前端"上一级"能一路走到盘符根（管理员随意浏览）
    let parent = cur
        .parent()
        .map(|p| yekitrim_core::path::display(p))
        .unwrap_or_default();
    // 目录列举是阻塞 IO（网络盘/大目录会明显卡顿），放 spawn_blocking
    let scan_dir = target.clone();
    let (dirs, files, volumes) = tokio::task::spawn_blocking(move || -> std::io::Result<(Vec<String>, Vec<String>, Vec<serde_json::Value>)> {
        let mut files = vec![]; let mut dirs = vec![];
        for e in std::fs::read_dir(&scan_dir)? {
            let e = e?;
            if e.file_type().map(|t| t.is_dir()).unwrap_or(false) { dirs.push(e.file_name().to_string_lossy().to_string()); }
            else { files.push(e.file_name().to_string_lossy().to_string()); }
        }
        dirs.sort(); files.sort();
        // 盘符探测也是阻塞 IO（每个候选要 stat 一次），跟目录列举一起放这里
        Ok((dirs, files, available_volumes()))
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
                "path": yekitrim_core::path::display(&canon),
                "label": yekitrim_service::projects::root_label(user.id, &canon),
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
        "root_label": yekitrim_service::projects::root_label(user.id, &cur),
        "roots": root_list,
        // 盘符候选：前端把它铺在目录树顶部，否则用户换不了盘（见 available_volumes 注释）
        "volumes": volumes,
        "tenant_root": yekitrim_core::path::display(&tenant),
        "path": rel,
        "parent": parent,
        "dirs": dirs,
        "files": files,
    }))
}

pub(super) async fn media_get(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<crate::auth::CurrentUser>,
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
    let st = state.clone();
    let resolved = blocking(move || resolve_media_path(st.auth.db(), &user, project, &rel)).await;
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
    // **ETag/304 + 文本类 gzip**：口径与硬约束全在 `media::serve_file_cached` 的文档里，
    // 这里只负责把头交过去 —— **别再在这一层自己抠 `Range`/`If-None-Match`**：
    // 那正是「四个文件出口只有这一条补了 ETag、另外四条漏了」的成因（2026-11-21 修）。
    //
    // 历史（2026-10-02，别丢）：精修页进 Dhampir 时会把该工程引用的**全部动图**整份取一遍
    // —— 实测一组 5 张贴纸合计 **15.72 MB**（单张最大 4.95 MB）。没有校验符时浏览器每次进
    // 页面都得原样重收一遍，用户看到的是「动图加载要 6.5 秒」，而大头其实是这 15.7 MB 的搬运。
    media::serve_file_from_headers(&path, &mime, &headers).await
}

/// `/api/media/*` 的归属校验 + 路径解析（同步：DB 查询 + 项目目录上的 canonicalize/jail）。
/// 抽出来是为了整体丢进 blocking 池 —— 见 `media_get` 的调用点。
///
/// ⚠️ **过渡期那条 `/api/media/<绝对路径>` 已于 2026-10-03 删除**。它当年靠一份"全局当前工程"
/// （`AppState.workspace`）把相对路径解析成绝对路径、再用 `find_project_for_dir` 反查归属 ——
/// 那份全局状态已经删掉（见 `model.rs` 头部）。而且**前端早就明确不认这条路**：
/// `webui/src/utils.js::mediaUrl` 的注释写着"不接受绝对路径，也不做旧 `/api/media/<绝对路径>` 的兼容"，
/// 所有调用点都走 `mediaUrl` / `clipMediaUrl`。
///
/// 判据是**路径形状**而不是"全局状态在不在"：要服务媒体就必须带项目 id —— 这样归属校验
/// 永远只看 `find_owned_project` 一处，不再有第二条反查旁路。
pub(super) fn resolve_media_path(
    db: &yekitrim_service::db::Db,
    user: &crate::auth::CurrentUser,
    project: Option<i64>,
    rel: &str,
) -> Result<PathBuf, AppError> {
    // `None` = 路径第一段不是正整数 id（旧契约，或手敲的 URL）。
    // 回 404 而不是 403：与"项目不存在"同口径，不给"这个路径存在但你没权限"的区分度。
    let Some(id) = project else {
        return Err(err(
            StatusCode::NOT_FOUND,
            "媒体路径必须带项目 id（/api/media/<项目id>/<项目内相对路径>）",
        ));
    };
    let proj = match crate::scope::find_owned_project(db, user, id) {
        Ok(Some(p)) => p,
        Ok(None) => return Err(err(StatusCode::NOT_FOUND, "项目不存在或无权访问")),
        Err(e) => return Err(err(StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}"))),
    };
    yekitrim_core::tenant::jail_existing(&proj.root_dir, rel)
        .map_err(|e| err(StatusCode::NOT_FOUND, format!("文件不可用: {e:#}")))
}







#[cfg(test)]
mod tests {
    /// **空态不是错误**（2026-11-15 用户实测）：只跑过「查找事件」、还没剪片段的项目，
    /// `/project-data/clips` 曾经回 404「暂无 clips」—— 而切片页把四个接口放在
    /// `Promise.all` 里，一个 404 让整页 catch，**连已经读到的 events 一起丢掉**，
    /// 界面显示「切片数据加载失败：暂无 clips」，用户以为检测白跑了（67 条事件其实在）。
    ///
    /// 契约：没有产物 = 空集合（200），不是资源不存在（404）。
    /// ⚠️ 写接口（PUT events / merge-splits / cut-all）保持 404 —— 那些是"操作不存在的产物"。
    #[tokio::test]
    async fn empty_project_returns_empty_collections_not_404() {
        let t = test_app().await;
        let root = t._dir.path().join("empty-proj");
        std::fs::create_dir_all(root.join("output")).unwrap();
        std::fs::write(root.join("big.mp4"), vec![0u8; 16]).unwrap();
        let cfg_path = root.join("yekitrim.toml");
        std::fs::write(&cfg_path, "input_file = \"big.mp4\"\n").unwrap();
        let pid = t
            .auth
            .db()
            .create_project(1, "empty-proj", None, "local", Some("空工程"), &root, &cfg_path, None)
            .unwrap();

        let res = t
            .call(t.req("GET", &format!("/project-data/clips?project={pid}")).body(Body::empty()).unwrap())
            .await;
        assert_eq!(res.status(), StatusCode::OK, "没有片段目录 ⇒ 200 + 空数组，不是 404");
        let body = body_json(res).await;
        assert_eq!(body.as_array().map(|a| a.len()), Some(0), "必须是空数组：{body}");

        let res = t
            .call(t.req("GET", &format!("/project-data/events?project={pid}")).body(Body::empty()).unwrap())
            .await;
        assert_eq!(res.status(), StatusCode::OK, "没有 events.json ⇒ 200 + 空集合，不是 404");
        let body = body_json(res).await;
        assert_eq!(
            body.get("events").and_then(|e| e.as_array()).map(|a| a.len()),
            Some(0),
            "events 必须是空数组：{body}"
        );
    }

    /// **B7（2026-11-14 后端 review）：已超配额的普通账号不能再起转写/检测/剪切任务**。
    ///
    /// 占用是 `quota_gate_impl` 自己刷新的（源文件落盘即计入）—— 内核判据见 api/mod.rs
    /// 的单测；这里钉的是 HTTP 面：409 + 文案点名配额，admin 不受限。
    #[tokio::test]
    async fn pipeline_is_rejected_when_account_is_over_quota() {
        let t = test_app().await;
        let db = t.auth.db();
        let hash = yekitrim_service::auth::hash_password("pass-12345678").unwrap();
        let uid = db.create_user("quota_http", &hash, "user", false, 100).unwrap();
        let root = t._dir.path().join(format!("quota-proj-{uid}"));
        std::fs::create_dir_all(root.join("output")).unwrap();
        std::fs::write(root.join("big.mp4"), vec![0u8; 200]).unwrap();
        let cfg_path = root.join("yekitrim.toml");
        std::fs::write(&cfg_path, "input_file = \"big.mp4\"\n").unwrap();
        let pid = db
            .create_project(uid, "quota-proj", None, "local", Some("配额"), &root, &cfg_path, None)
            .unwrap();
        let b = t.auth.login("quota_http", "pass-12345678", None, None).unwrap();

        let res = t
            .call(
                t.req_as(&b.session_id, &b.csrf, "POST", &format!("/project-data/pipeline?project={pid}"))
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"stage":"detect"}"#.to_string()))
                    .unwrap(),
            )
            .await;
        assert_eq!(res.status(), StatusCode::CONFLICT, "已超配额必须 409");
        let j = body_json(res).await;
        assert!(j["error"].as_str().unwrap_or("").contains("配额"), "{j}");

        // admin 不受限：同一形状的请求（admin 自己的项目 1）不是 409
        let res = t
            .call(
                t.req("POST", "/project-data/pipeline?project=1")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"stage":"detect"}"#.to_string()))
                    .unwrap(),
            )
            .await;
        assert_ne!(res.status(), StatusCode::CONFLICT, "admin 不受配额约束");
    }

    /// **§3 剪辑质量分**：`/project-data/clips` 必须回它，且**没精修过的片段回 `null`**。
    ///
    /// 两条判据缺一不可：
    /// ① 有 `polish.toml` + 字幕 ⇒ 回一个对象（含 `score` 与逐条 `interruptions`）；
    /// ② **没有 `polish.toml`** ⇒ 回 `null`。这一条是**防"假的高分"**：没有分段/事件时
    ///    四条判据全部无可判 ⇒ `evaluate` 得 0 干扰 ⇒ **满分 100**。把它显示出去比不显示更糟 ——
    ///    用户会以为"这条剪得完美"，而其实系统**根本还没看过它**。
    #[tokio::test]
    async fn clips_回质量分_但没精修过的回_null() {
        let t = test_app().await;
        let pid = project_ids(&t).await[0];
        let clips_dir = t.root.join("output/clips");
        std::fs::create_dir_all(&clips_dir).unwrap();

        // ① 没精修过的片段：**有字幕、但没有 polish.toml**。
        //    ⚠️ 必须有字幕 —— 否则它会因为"读不到 subtitles.json"而回 null，
        //    那样即使删掉 polish.toml 那道检查，测试**照样绿**（变异自证当场抓出过这个漏洞：
        //    判据没有隔离到它要锁的那一行）。带上字幕后，唯一能让它回 null 的理由
        //    就只剩"没有 polish.toml"这一条。
        let bare = clips_dir.join("bare");
        std::fs::create_dir_all(bare.join("data")).unwrap();
        std::fs::write(bare.join("clip.json"), r#"{"rank":1,"duration":20.0}"#).unwrap();
        std::fs::write(
            bare.join("data/subtitles.json"),
            r#"[{"index":1,"start":0.5,"end":3.0,"text":"这话还没说完","html":""}]"#,
        )
        .unwrap();

        // ② 已精修：polish.toml + data/subtitles.json。边界 1.0 切在一句话中间 ⇒ 该扣分。
        let done = clips_dir.join("done");
        std::fs::create_dir_all(done.join("data")).unwrap();
        std::fs::write(done.join("clip.json"), r#"{"rank":2,"duration":10.0}"#).unwrap();
        std::fs::write(
            done.join("data/subtitles.json"),
            r#"[{"index":1,"start":0.5,"end":3.0,"text":"这话还没说完","html":""}]"#,
        )
        .unwrap();
        let cfg = yekitrim_polish::project::ProjectFile::example();
        std::fs::write(done.join("polish.toml"), cfg.to_toml_commented().unwrap()).unwrap();
        // `example()` 的分段是 0~10（单段）⇒ 没有内部切点。手工换成两段、边界落句中。
        let mut two = cfg;
        two.segments = vec![
            yekitrim_polish::project::Segment {
                role: String::new(),
                start: 0.0, end: 1.0, fx: "low".into(), note: String::new(),
                events: vec![], extra: Default::default(),
            },
            yekitrim_polish::project::Segment {
                role: String::new(),
                start: 1.0, end: 10.0, fx: "med".into(), note: String::new(),
                events: vec![], extra: Default::default(),
            },
        ];
        std::fs::write(done.join("polish.toml"), two.to_toml_commented().unwrap()).unwrap();

        let res = t
            .call(t.req("GET", &format!("/project-data/clips?project={pid}")).body(Body::empty()).unwrap())
            .await;
        assert_eq!(res.status(), StatusCode::OK);
        let json = body_json(res).await;
        let arr = json.as_array().expect("clips 要回数组");

        let by_name = |n: &str| {
            arr.iter()
                .find(|c| c.get("name").and_then(|v| v.as_str()) == Some(n))
                .unwrap_or_else(|| panic!("找不到片段 {n}: {json}"))
        };

        // ① 没精修过 ⇒ null（**不是** 100 分）
        assert!(
            by_name("bare").get("quality").map(|q| q.is_null()).unwrap_or(false),
            "没精修过的片段 quality 必须是 null —— 回满分就是「假的高分」: {}",
            by_name("bare")
        );

        // ② 已精修 ⇒ 有对象、分数被扣、逐条理由带着位置
        let q = by_name("done").get("quality").expect("要有 quality");
        assert!(!q.is_null(), "已精修过的片段必须回质量分: {q}");
        let score = q.get("score").and_then(|v| v.as_f64()).expect("score 要是数字");
        assert!(score < 100.0, "边界切在句子中间，分数必须低于满分，实得 {score}");
        assert_eq!(q.get("count").and_then(|v| v.as_u64()), Some(1), "应当正好 1 条干扰: {q}");
        let items = q.get("interruptions").and_then(|v| v.as_array()).expect("要带逐条明细");
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].get("kind").and_then(|v| v.as_str()), Some("mid_sentence"));
        assert_eq!(items[0].get("at").and_then(|v| v.as_f64()), Some(1.0));
        assert!(
            items[0].get("label").and_then(|v| v.as_str()).map(|s| !s.is_empty()).unwrap_or(false),
            "每条要有给人看的理由（界面 tooltip 直接显示）: {items:?}"
        );
    }

    /// **`/api/media/*` 必须带 ETag，并且命中 `If-None-Match` 时回 304。**
    ///
    /// # 为什么这条判据要有（2026-10-02）
    ///
    /// 精修页进 Dhampir 时会把该工程引用的**全部动图**原样取一遍 —— 实测一组 5 张贴纸
    /// 合计 **15.72 MB**（单张最大 4.95 MB）。这条路此前**没有任何校验符、也没有
    /// `Cache-Control`** ⇒ 每次进页面都得重新搬一遍这 15.7 MB。
    ///
    /// 用户当时看到的症状是「动图加载要 6.5 秒」，而那条日志里的"解码"其实把
    /// `fetch` + `arrayBuffer` 也算进去了（宿主侧计时区间打错，
    /// 见 `PolishPreview.vue` 里那段注释）—— 于是"解码很慢"这个结论是被错计时喂出来的，
    /// 真正的大头是这 15.7 MB 的搬运。
    ///
    /// ⚠️ 与 wasm 那条的差别：这里**不能开压缩**（`/api/media/*` 要服务视频的 Range，
    /// 压缩后字节不再一一对应，会破坏播放）。所以本用例只验 ETag/304 与 Range 的共存。
    #[tokio::test]
    async fn media_带_etag_且命中时回_304() {
        let t = test_app().await;
        let pid = project_ids(&t).await[0];
        let rel = "output/clips/media-etag/sticker.gif";
        let dir = t.root.join("output/clips/media-etag");
        std::fs::create_dir_all(&dir).unwrap();
        // 用纯 ASCII 文件名：URI 里带 CJK 要走百分号编码，那是另一条判据，别混进来。
        // 造一份"看起来像视频"的字节：内容无所谓，判据只看长度/mtime 算出的校验符。
        let bytes: Vec<u8> = (0..4096u32).map(|i| (i % 251) as u8).collect();
        std::fs::write(t.root.join(rel), &bytes).unwrap();

        let uri = format!("/media/{pid}/{rel}");
        let req = |extra: Vec<(&str, &str)>| {
            let mut b = t.req("GET", &uri);
            for (k, v) in extra {
                b = b.header(k, v);
            }
            b.body(Body::empty()).unwrap()
        };

        // ① 首次：200 + **必须有 ETag**（没有它浏览器就无从校验，只能整份重收）
        let res = t.call(req(vec![])).await;
        assert_eq!(res.status(), StatusCode::OK, "首次取媒体应当成功");
        let etag = res
            .headers()
            .get(axum::http::header::ETAG)
            .expect("`/api/media/*` 必须带 ETag —— 否则 15.7 MB 的贴纸每次进页面都重传")
            .to_str()
            .unwrap()
            .to_string();
        assert!(etag.starts_with('"') && etag.ends_with('"'), "ETag 要带引号：{etag}");
        assert!(
            res.headers().get(axum::http::header::CONTENT_ENCODING).is_none(),
            "媒体路由**不许压缩**（要服务 Range，压缩会破坏播放）"
        );
        let len = axum::body::to_bytes(res.into_body(), usize::MAX).await.unwrap().len();
        assert_eq!(len, bytes.len());

        // ② 回带同一个 ETag ⇒ 304 且**空体**（省下的正是那 15.7 MB）
        let res = t.call(req(vec![("if-none-match", etag.as_str())])).await;
        assert_eq!(res.status(), StatusCode::NOT_MODIFIED, "ETag 没变就该 304");
        let body = axum::body::to_bytes(res.into_body(), usize::MAX).await.unwrap();
        assert!(body.is_empty(), "304 不许带 body，实际 {} 字节", body.len());

        // ③ 文件变了（长度变）⇒ ETag 必须变，刷新不能拿到旧贴纸
        let mut bigger = bytes.clone();
        bigger.extend_from_slice(b"tail");
        std::fs::write(t.root.join(rel), &bigger).unwrap();
        let res = t.call(req(vec![("if-none-match", etag.as_str())])).await;
        assert_eq!(
            res.status(),
            StatusCode::OK,
            "文件变了却仍回 304 —— 用户刷新会拿到旧贴纸（比慢更糟）"
        );
        let res = t.call(req(vec![])).await;
        let new_etag = res
            .headers()
            .get(axum::http::header::ETAG)
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();
        assert_ne!(new_etag, etag, "长度变了 ETag 必须变");

        // ④ Range 与 ETag 共存：**206 仍然按原字节发**（ETag 不许把播放搞坏）
        let res = t.call(req(vec![("range", "bytes=0-99")])).await;
        assert_eq!(res.status(), StatusCode::PARTIAL_CONTENT, "带 Range 必须还是 206");
        let part = axum::body::to_bytes(res.into_body(), usize::MAX).await.unwrap();
        assert_eq!(part.len(), 100, "Range 的字节数不对：{}", part.len());
        assert_eq!(&part[..], &bigger[..100], "206 的字节必须与原始文件一一对应");
    }

    /// **复现真实场景的省流量**：5 张动图贴纸（合计约 15.7 MB）第二次进页面时**一字节都不传**。
    ///
    /// 上一条验的是"304 的机制对不对"，这条验的是"**省下的量级**对不对" ——
    /// 用户报的是"动图加载 6.5 秒"，而那 6.5 秒里的大头是这 15.7 MB 的搬运。
    /// 判据用**实际读出的字节数**，不用 header：只有真少传了才算数。
    #[tokio::test]
    async fn media_第二次进页面时贴纸一字节都不重传() {
        let t = test_app().await;
        let pid = project_ids(&t).await[0];
        let dir = t.root.join("output/clips/sticker-vol");
        std::fs::create_dir_all(&dir).unwrap();

        // 五个文件，尺寸取真实贴纸的量级（这里按 1/100 缩放造：够验机制，又不拖慢用例）。
        let sizes = [495, 426, 247, 217, 187];
        let mut names = Vec::new();
        for (n, size) in sizes.iter().enumerate() {
            let name = format!("sticker-{n}.gif");
            std::fs::write(dir.join(&name), vec![0xA5u8; size * 1024]).unwrap();
            names.push(name);
        }

        // 第一次：全量取，同时记下每个文件的 ETag 与字节数
        let mut total_first = 0usize;
        let mut etags = Vec::new();
        for name in &names {
            let uri = format!("/media/{pid}/output/clips/sticker-vol/{name}");
            let res = t.call(t.req("GET", &uri).body(Body::empty()).unwrap()).await;
            assert_eq!(res.status(), StatusCode::OK);
            etags.push(
                res.headers()
                    .get(axum::http::header::ETAG)
                    .expect("每个贴纸都要有 ETag")
                    .to_str()
                    .unwrap()
                    .to_string(),
            );
            total_first += axum::body::to_bytes(res.into_body(), usize::MAX).await.unwrap().len();
        }
        assert!(total_first > 1_000_000, "这组素材应当有 MB 级体量，实际 {total_first}");

        // 第二次（模拟用户再进一次精修页）：带 ETag ⇒ **全部 304、零字节**
        let mut total_second = 0usize;
        for (name, tag) in names.iter().zip(&etags) {
            let uri = format!("/media/{pid}/output/clips/sticker-vol/{name}");
            let res = t
                .call(t.req("GET", &uri).header("if-none-match", tag.as_str()).body(Body::empty()).unwrap())
                .await;
            assert_eq!(res.status(), StatusCode::NOT_MODIFIED, "{name} 没有命中 304");
            total_second += axum::body::to_bytes(res.into_body(), usize::MAX).await.unwrap().len();
        }
        assert_eq!(
            total_second, 0,
            "第二次进页面仍然搬了 {total_second} 字节 —— 贴纸应当一字节都不重传"
        );
    }

    use super::*;
    use crate::api::test_support::*;
    use crate::jobs::{JobClass, JobSpec};

    use axum::body::Body;


/// 选了**外部字幕**（`transcript_file` 指向工程内的 sub.srt）后，在预处理页保存编辑
/// 必须写回那份文件 —— 只写 output/ 的话下次读到的还是原文件，编辑静默丢失。
#[test]
fn transcript_save_writes_back_to_selected_subtitle() {
    let (ws, root) = tmp_workspace("sub-writeback");
    let sub = root.join("sub.srt");
    std::fs::write(&sub, "1\n00:00:01,000 --> 00:00:02,000\n原始\n\n").unwrap();
    yekitrim_core::config::patch_config_file(
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
    yekitrim_core::config::patch_config_file(
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
    yekitrim_core::config::patch_config_file(
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
    // 两种 ASR 模式都没产物 → 转写入口不置灰
    assert_eq!(v["slots"]["hybrid"], false);
    assert_eq!(v["slots"]["single"], false);
    let _ = std::fs::remove_dir_all(&root);
}

/// 某模式的 ASR 产物一落盘，`slots` 就报该槽已占（前端据此把转写入口置灰）。
#[test]
fn assets_impl_reports_asr_slots() {
    let (ws, root) = tmp_workspace("assets-slots");
    std::fs::create_dir_all(root.join("output")).unwrap();
    std::fs::write(
        root.join("output/transcript.hybrid.json"),
        r#"{"segments":[{"start":0.0,"end":1.0,"text":"a"}],"language":"zh"}"#,
    )
    .unwrap();
    let v = assets_impl(&ws, "local", "", None).unwrap();
    assert_eq!(v["slots"]["hybrid"], true);
    assert_eq!(v["slots"]["single"], false);
    // 清单里也要能看到它（盘上有、没登记过 → 兜底列出来），且不可删
    let items = v["transcript"]["items"].as_array().expect("items");
    assert_eq!(items.len(), 1, "{items:?}");
    assert_eq!(items[0]["mode"], "hybrid");
    assert_eq!(items[0]["is_asr"], true);
    assert_eq!(items[0]["removable"], false);
    let _ = std::fs::remove_dir_all(&root);
}

/// 清单删除：用户素材可删，ASR 产物（文件还在）必须被拒。
#[test]
fn assets_put_impl_remove_protects_asr_items() {
    let (ws, root) = tmp_workspace("assets-remove");
    std::fs::create_dir_all(root.join("output")).unwrap();
    std::fs::write(root.join("dm.xml"), "<?xml version=\"1.0\"?><i><d p=\"1.5,1,25,16777215,0\">hi</d></i>").unwrap();
    std::fs::write(
        root.join("output/transcript.hybrid.json"),
        r#"{"segments":[{"start":0.0,"end":1.0,"text":"a"}],"language":"zh"}"#,
    )
    .unwrap();
    // 选中 ASR 产物 → 按槽登记（不可删）
    let v = assets_put_impl(
        &ws,
        &serde_json::json!({ "kind": "transcript", "path": "output/transcript.hybrid.json" }),
        false,
    )
    .unwrap();
    assert_eq!(v["value"], "output/transcript.hybrid.json");
    assert_eq!(v["removed"], false);

    let e = assets_put_impl(
        &ws,
        &serde_json::json!({ "kind": "transcript", "action": "remove", "path": "output/transcript.hybrid.json" }),
        false,
    )
    .unwrap_err();
    assert_eq!(e.0, StatusCode::BAD_REQUEST);

    // 用户素材：选了能删，删掉的正好是生效值 → 连带清空 `danmaku_file`
    let _ = assets_put_impl(&ws, &serde_json::json!({ "kind": "danmaku", "path": "dm.xml" }), false).unwrap();
    let v = assets_put_impl(
        &ws,
        &serde_json::json!({ "kind": "danmaku", "action": "remove", "path": "dm.xml" }),
        false,
    )
    .unwrap();
    assert_eq!(v["removed"], true);
    assert_eq!(v["value"], "");
    let cfg = ws.load_config().unwrap();
    assert_eq!(cfg.project.danmaku_file.as_deref(), Some(""));
    assert!(cfg.project.materials.danmaku.is_empty());
    // 字幕的 ASR 条目还在（删弹幕不该动字幕）
    assert_eq!(cfg.project.materials.transcript.len(), 1);
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

/// 路径面归零：未登记的路径（含任意绝对路径）一律 404，只有本人项目 id 可用。
#[tokio::test]
async fn path_face_is_closed_for_project_data() {
    let t = test_app().await;
    // 未登记的 config 路径
    let res = t
        .call(t.req("GET", "/project-data/status?config=C%3A%5CWindows%5Cyekitrim.toml").body(Body::empty()).unwrap())
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
    std::fs::write(root.join("yekitrim.toml"), "[project]\ninput_file = \"a.mp4\"\ndanmaku_file = \"dm.ass\"\n").unwrap();

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

/// **T4（同步 IO 三处补 blocking）**：`clip_delete_impl` 里的"删整个片段目录 + 重写发布索引"
/// 必须真的跑到 blocking 池上 —— 单 worker runtime 下证明它不占住 worker。
///
/// 判据与 `api::tests::blocking_helper_keeps_workers_free` 同一条思路：只留 **1 个 worker**，
/// 另起一个每 1ms 跳一次的心跳任务；**删片段期间心跳必须继续跳**。把这两段同步 IO 留在
/// worker 上（本次整改前的写法），心跳一次都跑不动（`during == 0`）—— 这正是"剪切的时候
/// 整个服务器连不上"的机制：项目常在网络盘上，一次 `remove_dir_all` 就够把 worker 占住。
///
/// fixture 是最小目录（临时目录 + 几千个几字节的小文件）：删掉它要几十毫秒，够心跳跳几十次；
/// **不依赖网络盘**，也不构造真实成片。
#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn clip_delete_runs_sync_io_off_the_worker() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::time::{sleep, Duration};

    // ---- 最小 fixture：一个待删的片段目录（内容量刚好让 remove_dir_all 有真实耗时）----
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("proj");
    let clips = root.join("output").join("clips");
    let clip = clips.join("clip-01");
    std::fs::create_dir_all(&clip).unwrap();
    for i in 0..6000 {
        std::fs::write(clip.join(format!("f{i:05}.bin")), b"x").unwrap();
    }
    std::fs::write(clip.join("clip.mp4"), b"fake").unwrap();
    // 发布索引里登记它：删完这两份都得把它摘掉（`prune_clip_from_index` 也在同一个闭包里）
    std::fs::write(
        root.join("output").join("publish_config.json"),
        r#"{"video_source":"a.mp4","clips":[{"directory":"clip-01"},{"directory":"clip-02"}]}"#,
    )
    .unwrap();
    std::fs::write(clips.join("clips_order.txt"), "clip-01 / 甲\nclip-02 / 乙").unwrap();

    // `clip_delete_impl` 只需要 `AppState` 里的 JobManager（这里没有任何任务在跑）
    let auth = crate::auth::AuthService::open(tmp.path().join("auth"), crate::auth::Listening::Loopback)
        .unwrap();
    let state = Arc::new(AppState::with_auth(auth));
    let ws = Workspace {
        config_file: root.join("yekitrim.toml"),
        root_dir: root.clone(),
    };

    // ---- 心跳：只有 1 个 worker，同步 IO 若留在 worker 上它就再也跳不动 ----
    let ticks = Arc::new(AtomicUsize::new(0));
    let beats = ticks.clone();
    let beat = tokio::spawn(async move {
        loop {
            sleep(Duration::from_millis(1)).await;
            beats.fetch_add(1, Ordering::SeqCst);
        }
    });
    // 先让它真的跳起来 —— 否则"心跳 0 次"可能只是还没开始跳，什么也测不出来。
    // ⚠️ **轮询**而不是死等 5ms：并行跑整套测试时机器可能正被别的用例占满，
    //    5ms 内一次都没轮到它就会假红（2026-11-24 实测过：整套跑了 135s 的那轮）。
    let mut waited = 0u32;
    while ticks.load(Ordering::SeqCst) == 0 && waited < 2_000 {
        sleep(Duration::from_millis(5)).await;
        waited += 5;
    }
    assert!(
        ticks.load(Ordering::SeqCst) > 0,
        "心跳应当已经跳起来（等了 {waited}ms）"
    );

    let before = ticks.load(Ordering::SeqCst);
    let t0 = std::time::Instant::now();
    // ⚠️ 必须 `spawn` 上去：`#[tokio::test]` 的**测试体本身跑在 block_on 的 main 线程上**，
    // 直接在测试体里 `await` 它，被同步 IO 卡住的就是 main 线程而不是 worker ——
    // 那样无论实现对不对，心跳都在另一个线程上照跳，测不出任何东西（变异自证当场抓出来的坑）。
    let del = tokio::spawn(async move { clip_delete_impl(&ws, "clip-01", &state, 1).await });
    let res = del.await.expect("删除任务不该 panic");
    let elapsed = t0.elapsed();
    let during = ticks.load(Ordering::SeqCst) - before;
    beat.abort();

    // ---- 功能面：目录删干净了，两份索引里的引用也摘掉了 ----
    assert!(res.is_ok(), "删除片段应当成功: {:?}", res.as_ref().err());
    assert_eq!(res.unwrap().0["removed"], "clip-01");
    assert!(!clip.exists(), "整个片段目录必须被删掉");
    let publish = std::fs::read_to_string(root.join("output").join("publish_config.json")).unwrap();
    assert!(
        !publish.contains("clip-01") && publish.contains("clip-02"),
        "发布索引里不能再指向已删片段: {publish}"
    );
    assert_eq!(
        std::fs::read_to_string(clips.join("clips_order.txt")).unwrap(),
        "clip-02 / 乙",
        "clips_order.txt 里那行也要摘掉"
    );

    // ---- 判据：删除期间 worker 一直被让出来 ----
    assert!(
        elapsed.as_millis() >= 5,
        "fixture 太小（删除只用了 {elapsed:?}）：测不出 worker 是否被占，请加大文件数"
    );
    assert!(
        during >= 3,
        "删片段期间 worker 必须还能跑别的任务（心跳只跳了 {during} 次 / 删除用时 {elapsed:?}）\
         —— 同步 IO 又回到 worker 上了？"
    );
}

/// **磁盘预检把一键全切拦停**（2026-11-15 事故回归钉）：`[cutting] min_free_gb` 拉满时，
/// 每条 `run_single_cut` 都在 runner 的阶段预检处失败（`FreeSpaceError`）——
/// 任务必须以「磁盘空间不足」收场（**不是**"N 个事件剪切失败"），且一条片段都没剪出来；
/// 腾出空间后重跑只剪缺失的目录（全切的既有语义）＝天然续作。
#[tokio::test]
async fn cut_all_stops_and_reports_when_disk_precheck_fails() {
    let t = test_app().await;
    let root = t.root.clone();

    // 两个 enabled 事件，都没有 clip.mp4 ⇒ 全切两条都要剪
    let events = serde_json::json!({
        "titles": {},
        "events": [
            {"start": {"h":0,"m":0,"s":0,"ms":0}, "end": {"h":0,"m":0,"s":10,"ms":0},
             "label": "A", "summary": "", "event_type": "chat", "source": "LLM",
             "confidence": 1.0, "popularity": 0.5, "rank": 1, "clip_dir": "clip-a", "enabled": true},
            {"start": {"h":0,"m":0,"s":10,"ms":0}, "end": {"h":0,"m":0,"s":20,"ms":0},
             "label": "B", "summary": "", "event_type": "chat", "source": "LLM",
             "confidence": 1.0, "popularity": 0.4, "rank": 2, "clip_dir": "clip-b", "enabled": true}
        ]
    });
    std::fs::create_dir_all(root.join("output")).unwrap();
    std::fs::write(root.join("output").join("events.json"), serde_json::to_string(&events).unwrap()).unwrap();
    // 阈值拉满（9223372036854775807 = i64::MAX，TOML 整数上限；换算字节后饱和成
    // u64::MAX）⇒ 任何真实磁盘都必然不足
    std::fs::write(
        root.join("yekitrim.toml"),
        "[project]\ninput_file = \"a.mp4\"\n\n[cutting]\nmin_free_gb = 9223372036854775807\n",
    ).unwrap();

    let res = t.call(
        t.req("POST", "/project-data/events/cut-all?project=1")
            .header(axum::http::header::CONTENT_TYPE, "application/json")
            .body(Body::from("{}"))
            .unwrap(),
    ).await;
    assert_eq!(res.status(), StatusCode::OK, "cut-all 照常受理成任务");
    let v = body_json(res).await;
    let job_id = v["id"].as_str().unwrap_or("").to_string();
    assert!(!job_id.is_empty(), "{v}");

    // 轮询到终态（任务体跑在专用线程上，这里给它足够的时间）
    let mut error_msg = String::new();
    for _ in 0..200 {
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        let res = t.call(t.req("GET", &format!("/jobs/{job_id}")).body(Body::empty()).unwrap()).await;
        let v = body_json(res).await;
        let status = v["status"].as_str().unwrap_or("").to_string();
        if ["failed", "completed", "cancelled"].contains(&status.as_str()) {
            assert_eq!(status, "failed", "预检拦停 ⇒ 任务必须失败: {v}");
            error_msg = v["error"].as_str().unwrap_or("").to_string();
            break;
        }
    }
    assert!(error_msg.contains("磁盘空间不足"), "失败原因必须点名空间不足: {error_msg}");
    assert!(error_msg.contains("未处理"), "要把没剪的条数说清: {error_msg}");
    assert!(!error_msg.contains("个事件剪切失败"), "空间拦停不能伪装成剪切失败: {error_msg}");

    // 一条都没剪出来（预检挡在 ffmpeg 之前）
    assert!(!root.join("output/clips/clip-a/clip.mp4").exists());
    assert!(!root.join("output/clips/clip-b/clip.mp4").exists());
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
                vec![crate::scope_key::project_clip(1, "clip-03")],
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
                vec![crate::scope_key::project_out(1)],
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

/// `GET /project-data/clips` 要带「最后一次改动」时间（`mtime`，unix 秒）—— 首页「切片总览」
/// 按它倒序，刚剪完 / 刚精修过的片段要浮到最前面。
///
/// 取值 = 片段目录里最新的 mtime，**直接子项**（精修保存会改写 `polish.toml` / `clip.srt`）与
/// 精修数据目录 `data/` 下的文件都要算进去：只 stat 目录本身的话，改写**既有**文件不会带起
/// 目录的 mtime（只有新建/删除才会），刚精修过的片段就浮不上来。
#[tokio::test]
async fn clips_expose_last_touched_mtime() {
    let t = test_app().await;
    let clips = t.root.join("output").join("clips");
    std::fs::create_dir_all(&clips).unwrap();
    write_clip_dir(&clips, 1, 0.0, 10.0, 1, None);
    write_clip_dir(&clips, 2, 0.0, 10.0, 1, None);

    let now = std::time::SystemTime::now();
    let at = |secs: u64| now + std::time::Duration::from_secs(secs);
    let secs = |t: std::time::SystemTime| t.duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
    let touch = |p: &std::path::Path, when: std::time::SystemTime| {
        std::fs::File::options().write(true).open(p).unwrap().set_modified(when).unwrap();
    };

    // clip-01：动的是**直接子项** `polish.toml`（精修保存就改它）
    let one = clips.join("clip-01");
    std::fs::write(one.join("polish.toml"), "[source]\n").unwrap();
    touch(&one.join("polish.toml"), at(3_600));
    // clip-02：只动过 `data/` 里的文件（两层：片段目录 → data/ → 文件）
    let two_data = clips.join("clip-02").join("data");
    std::fs::create_dir_all(&two_data).unwrap();
    std::fs::write(two_data.join("subtitles.json"), "{}").unwrap();
    touch(&two_data.join("subtitles.json"), at(7_200));

    let res = t.call(t.req("GET", "/project-data/clips?project=1").body(Body::empty()).unwrap()).await;
    assert_eq!(res.status(), StatusCode::OK);
    let v = body_json(res).await;
    let mtime_of = |name: &str| -> u64 {
        v.as_array().unwrap().iter()
            .find(|c| c["name"].as_str() == Some(name))
            .and_then(|c| c["mtime"].as_u64())
            .unwrap_or_else(|| panic!("{name} 没有 mtime: {v}"))
    };
    assert_eq!(mtime_of("clip-01"), secs(at(3_600)), "直接子项的 mtime 要算进来: {v}");
    assert_eq!(mtime_of("clip-02"), secs(at(7_200)), "data/ 里的改动也要算进来: {v}");
    assert!(mtime_of("clip-02") > mtime_of("clip-01"), "最后改动的片段 mtime 更大: {v}");
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

/// `/api/media/<绝对路径>` 那条**过渡期契约**必须已经关掉（2026-10-03 随全局工程状态一起删）。
///
/// 它当年是拿"全局当前工程"把相对路径解析成绝对路径、再用 `find_project_for_dir` 反查归属的 ——
/// 那条反查是**第二套归属判据**，与 `find_owned_project` 并存。现在只剩后者一条路。
/// 钉住两件事：① 不带项目 id 一律 404；② 带项目 id 时仍然只能拿自己项目内的文件。
#[tokio::test]
async fn media_requires_project_id_and_stays_inside_the_project() {
    let t = test_app().await;
    let clip = t.root.join("output").join("clips").join("clip-01");
    std::fs::create_dir_all(&clip).unwrap();
    std::fs::write(clip.join("clip.mp4"), b"fake-mp4").unwrap();

    // ① 项目内相对路径 + 合法项目 id → 能取到
    let ok = t.call(t.req("GET", "/media/1/output/clips/clip-01/clip.mp4").body(Body::empty()).unwrap()).await;
    assert_eq!(ok.status(), StatusCode::OK, "带项目 id 的项目内文件应当可取");

    // ② 不带项目 id（旧契约形状）→ 404，且**不许**被解析成"当前工程"下的文件
    for legacy in ["/media/output/clips/clip-01/clip.mp4", "/media/C:/Windows/win.ini"] {
        let res = t.call(t.req("GET", legacy).body(Body::empty()).unwrap()).await;
        assert_eq!(res.status(), StatusCode::NOT_FOUND, "旧契约必须已关闭: {legacy}");
    }

    // ③ 带项目 id 但跳出项目根 → 不放行
    let esc = t.call(t.req("GET", "/media/1/../yekitrim.toml").body(Body::empty()).unwrap()).await;
    assert_ne!(esc.status(), StatusCode::OK, "越出项目根不许放行");
}

/// 全局工程状态删掉之后，`/api/config` 必须是**纯程序配置**：不再回项目相关字段。
///
/// 这几个字段（`config_file` / `root_dir` / `yekitrim_raw` / `config`）曾是"当前工程"的投影，
/// 前端**一个都没消费**（项目设置页读的是 `/api/project-data/config` 的 `yekitrim_raw`）。
/// 留着它们等于在项目无关的接口上继续暴露"当前工程"这个概念。
#[tokio::test]
async fn global_config_endpoint_no_longer_exposes_a_project() {
    let t = test_app().await;
    let res = t.call(t.req("GET", "/config").body(Body::empty()).unwrap()).await;
    assert_eq!(res.status(), StatusCode::OK);
    let v = body_json(res).await;
    for gone in ["config_file", "root_dir", "yekitrim_raw", "config"] {
        assert!(v.get(gone).is_none(), "/api/config 不该再有 {gone}: {v}");
    }
    // 程序配置那几样必须还在（设置页靠它们）
    for kept in ["global_raw", "llm_raw", "global_config", "llm_config"] {
        assert!(v.get(kept).is_some(), "/api/config 少了 {kept}: {v}");
    }
}

/// 项目级的配置读取必须仍然给全"这个工程"的四样（项目设置页 / 预处理页靠它）。
#[tokio::test]
async fn project_scoped_config_still_returns_the_project() {
    let t = test_app().await;
    let res = t.call(t.req("GET", "/project-data/config?project=1").body(Body::empty()).unwrap()).await;
    assert_eq!(res.status(), StatusCode::OK);
    let v = body_json(res).await;
    for kept in ["config_file", "root_dir", "yekitrim_raw", "config"] {
        assert!(v.get(kept).is_some(), "项目配置少了 {kept}: {v}");
    }
}

/// **手工标的多段切片：重叠的段落盘时必须并成一段**（否则同一段内容会被剪两遍）。
///
/// 现场：项目 12 那条手工 4 段片段 `1625.7–1653.8` 里套着 `1632.0–1643.0` —— `clip.mp4`
/// 158.3s = 四段之和，其中 11s 是重复的，而 `duration` 也按各段之和多报一遍。
/// 判据走**真接口**（`POST /project-data/events`），不直接调 `merge_overlapping_ranges`：
/// 要钉住的正是"这条路真的会调它"。
#[tokio::test]
async fn create_event_merges_overlapping_ranges() {
    let t = test_app().await;
    let pid = project_ids(&t).await[0];
    let body = serde_json::json!({
        "label": "重叠段用例",
        "ranges": [
            {"start_sec": 441.4, "end_sec": 486.5},
            {"start_sec": 929.0, "end_sec": 1003.0},
            {"start_sec": 1625.7, "end_sec": 1653.8},
            // ⚠️ 完全落在上一段里面：不许原样落盘
            {"start_sec": 1632.0, "end_sec": 1643.0}
        ]
    });
    let res = t
        .call(
            t.req("POST", &format!("/project-data/events?project={pid}"))
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&body).unwrap()))
                .unwrap(),
        )
        .await;
    assert_eq!(res.status(), StatusCode::OK, "建事件应当成功");

    let raw = std::fs::read_to_string(t.root.join("output/events.json")).unwrap();
    let v: serde_json::Value = serde_json::from_str(&raw).unwrap();
    let ev = v["events"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["label"] == "重叠段用例")
        .expect("新事件必须落盘");
    let rs = ev["ranges"].as_array().expect("多段事件必须带 ranges");
    assert_eq!(rs.len(), 3, "重叠的那段要被并掉：{rs:?}");
    assert_eq!(
        (
            rs[2]["start"].as_f64().unwrap(),
            rs[2]["end"].as_f64().unwrap()
        ),
        (1625.7, 1653.8),
        "并出来的应当是外层那段（内含段不许把外层截短）"
    );
    // 外沿不变；也就是说"跨度"这条口径没被并段改掉
    assert_eq!(ev["start"]["m"].as_u64().unwrap(), 7);
    assert_eq!(ev["end"]["m"].as_u64().unwrap(), 27);
}

/// 反例：**中间隔了内容的两段不许并**（那是"分段剪、把中间别的话题丢掉"的表达）。
#[tokio::test]
async fn create_event_keeps_ranges_with_content_gap() {
    let t = test_app().await;
    let pid = project_ids(&t).await[0];
    let body = serde_json::json!({
        "label": "分段用例",
        "ranges": [
            {"start_sec": 100.0, "end_sec": 120.0},
            {"start_sec": 140.0, "end_sec": 160.0}
        ]
    });
    let res = t
        .call(
            t.req("POST", &format!("/project-data/events?project={pid}"))
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&body).unwrap()))
                .unwrap(),
        )
        .await;
    assert_eq!(res.status(), StatusCode::OK);
    let raw = std::fs::read_to_string(t.root.join("output/events.json")).unwrap();
    let v: serde_json::Value = serde_json::from_str(&raw).unwrap();
    let ev = v["events"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["label"] == "分段用例")
        .expect("新事件必须落盘");
    assert_eq!(
        ev["ranges"].as_array().unwrap().len(),
        2,
        "中间隔了 20s 内容，两段必须留着"
    );
}

/// **SRT 正文抽取**：序号行与时间行不许进搜索池，正文里的数字不许被误杀。
///
/// 现场：仓里那几个 `video_llm.srt` 开头都是 **BOM + `1`**（实测 `video_llm.srt` 的首字节是
/// `EF BB BF`）—— 不剥 BOM 的话 `"\u{feff}1".parse::<u32>()` 失败，那个序号 `1` 会被当成
/// 正文塞进搜索池（搜 "1" 会命中每一个项目，纯噪声）。
#[test]
fn srt_body_skips_index_and_time_lines() {
    let raw = "\u{feff}1\n00:00:22,500 --> 00:00:24,990\nI took a ride, and when nobody\n\n2\n00:00:25,020 --> 00:00:27,780\n第 3 次见面说了 42 句\n";
    let got = srt_body_lines(raw);
    assert_eq!(
        got,
        vec![
            "I took a ride, and when nobody".to_string(),
            "第 3 次见面说了 42 句".to_string()
        ],
        "序号行/时间行要丢掉、BOM 要剥掉、正文里的数字要留住"
    );
    assert!(!got.iter().any(|l| l == "1" || l == "2"), "序号不许漏进正文");
}

/// 只有时间行/序号的 SRT ⇒ 空（别回一堆空串）。
#[test]
fn srt_body_of_markup_only_is_empty() {
    assert!(srt_body_lines("1\n00:00:01,000 --> 00:00:02,000\n\n").is_empty());
    assert!(srt_body_lines("").is_empty());
}

/// **生效转写稿可能是 SRT**（真 bug 的回归）：`transcript_file` 实测取值有
/// `output/transcript.json` / `output/video.srt` / `output/video_llm.srt` 三种。
/// 只按 JSON 解会把"生效稿是 SRT"的项目整份判成没有字幕 ——
/// 实测项目 2（1018 段）与项目 3（2002 段）就这么静默丢了，而项目 1 的生效稿正是 `video_llm.srt`。
#[test]
fn subtitle_reading_handles_srt_as_effective_transcript() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    std::fs::create_dir_all(root.join("output")).unwrap();
    let srt = root.join("output/video.srt");
    std::fs::write(
        &srt,
        "\u{feff}1\n00:00:01,000 --> 00:00:02,000\n第一句\n\n2\n00:00:03,000 --> 00:00:04,000\n第二句\n",
    )
    .unwrap();
    let got = read_subtitle_file(&srt);
    assert_eq!(got, vec!["第一句".to_string(), "第二句".to_string()], "SRT 生效稿要读得出来");

    // JSON 那份照旧
    let js = root.join("output/transcript.json");
    std::fs::write(&js, r#"{"segments":[{"text":"甲"},{"text":"乙"}]}"#).unwrap();
    assert_eq!(read_subtitle_file(&js), vec!["甲".to_string(), "乙".to_string()]);

    // 读不到 = 空，不 panic
    assert!(read_subtitle_file(&root.join("output/nope.json")).is_empty());
}

/// 订正稿与生效稿**是同一个文件时不重复收**（否则"命中 N 条"会翻倍）。
#[test]
fn subtitles_do_not_double_count_corrected_draft() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    std::fs::create_dir_all(root.join("output")).unwrap();
    let ws = yekitrim_service::Workspace {
        config_file: root.join(yekitrim_core::path::PROJECT_CONFIG_FILE),
        root_dir: root.to_path_buf(),
    };
    // 生效稿 = 订正稿（真实情形：项目 1 的 `transcript_file` 就是 `output/video_llm.srt`）
    std::fs::write(
        root.join("yekitrim.toml"),
        "[project]\ninput_file = \"a.mp4\"\ntranscript_file = \"output/video_llm.srt\"\n",
    )
    .unwrap();
    std::fs::write(
        root.join("output/video_llm.srt"),
        "1\n00:00:01,000 --> 00:00:02,000\n只此一句\n",
    )
    .unwrap();
    let got = subtitles_for_search(&ws);
    assert_eq!(got, vec!["只此一句".to_string()], "同一个文件只算一次");
}

/// 弹幕正文桶：**走同一条解析路径**，且 ASS 覆写标签原样保留
/// （摘标签是前端 `utils.js::dmPlain` 的活 —— 解析层保留标签是给渲染用的，别在这一层摘）。
#[tokio::test]
async fn danmaku_texts_keeps_raw_ass_text() {
    let t = test_app().await;
    let pid = project_ids(&t).await[0];
    // 弹幕放**视频同目录**、用 `danmu.ass` 这个名字：`Workspace::danmaku_path` 配置没写时
    // 会回落找同目录的 `danmu.ass` / `danmaku.ass`，比手改配置更贴近真实工程。
    let dm = t.root.join("danmu.ass");
    std::fs::write(
        &dm,
        "[Script Info]\n[Events]\nFormat: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\n\
         Dialogue: 0,0:00:01.00,0:00:05.00,Default,,0,0,0,,{\\move(1970,0,-50,0)}呜呜呜\n\
         Dialogue: 0,0:00:06.00,0:00:09.00,Default,,0,0,0,,怎么没人说话\n",
    )
    .unwrap();
    // ⚠️ `ProjectConfig` 是从 **`[project]` 表**反序列化的：fixture 里那行**顶层**
    //    `input_file = "a.mp4"` 读不出来（`load_config` 走 `[project]`）—— 而"配置没写就找同目录
    //    `danmu.ass`"这条回落**要 input_file** 才走得到。这里补一份带 `[project]` 的配置。
    std::fs::write(t.root.join("yekitrim.toml"), "[project]\ninput_file = \"a.mp4\"\n").unwrap();

    let res = t
        .call(t.req("GET", &format!("/project-data/search-text?project={pid}")).body(Body::empty()).unwrap())
        .await;
    assert_eq!(res.status(), StatusCode::OK, "search-text 应当可用");
    let v = body_json(res).await;
    let dm_lines: Vec<String> = v["dm"]
        .as_array()
        .expect("要回 dm 桶")
        .iter()
        .map(|x| x.as_str().unwrap_or("").to_string())
        .collect();
    assert_eq!(dm_lines.len(), 2, "两条弹幕都要在：{dm_lines:?}");
    assert!(
        dm_lines[0].contains("{\\move(1970,0,-50,0)}"),
        "解析层要**原样保留** ASS 覆写标签（摘标签是前端的事）：{dm_lines:?}"
    );
    assert!(dm_lines.iter().any(|l| l.contains("怎么没人说话")));
    // 没配弹幕文件的项目：**回空桶**、不报错（"没配弹幕"与"弹幕是空的"对搜索是一回事）
    assert!(v["subs"].as_array().is_some(), "subs 桶也必须存在（可能是空的）");
}

/// **主播名只进产物标题**（2026-11-22 用户口径：「主播名不参与标题以外的任何处理」）：
/// 项目级默认 ⇒ 跟随的片段自动带上；改成覆盖 ⇒ 锁定；关掉 ⇒ 不拼；清回 null ⇒ 又跟随。
/// ⚠️ 全程 `publish_config.json` 的 `title` 都是**纯标题**（切片产物里不许出现 `【名】`）——
/// 名字只在读取/投稿时合成，这正是「填错名字不用重切」的依据。
#[tokio::test]
async fn streamer_name_composes_into_the_publish_title_only() {
    let t = test_app().await;
    let root = t.root.clone();
    let clips = root.join("output").join("clips");
    std::fs::create_dir_all(&clips).unwrap();
    write_clip_dir(&clips, 1, 0.0, 10.0, 1, None);
    std::fs::write(
        root.join("output").join("publish_config.json"),
        r#"{"video_source":"a.mp4","clips":[{"directory":"clip-01","title":"标题"}]}"#,
    )
    .unwrap();
    // 项目级主播名（跟着本工程走，不写程序级配置）
    let cfg = root.join("yekitrim.toml");
    if !cfg.exists() {
        std::fs::write(&cfg, "[project]\ninput_file = \"a.mp4\"\n").unwrap();
    }
    yekitrim_core::config::patch_config_file(
        &cfg,
        &serde_json::json!({ "detection": { "vtuber_name": "主播" } }),
    )
    .unwrap();

    let list = || {
        let t = &t;
        async move {
            let res = t
                .call(t.req("GET", "/project-data/clips?project=1").body(Body::empty()).unwrap())
                .await;
            body_json(res).await
        }
    };
    let put = |body: serde_json::Value| {
        let t = &t;
        async move {
            let res = t
                .call(
                    t.req("PUT", "/project-data/clips/clip-01/title?project=1")
                        .header("content-type", "application/json")
                        .body(Body::from(body.to_string()))
                        .unwrap(),
                )
                .await;
            (res.status(), body_json(res).await)
        }
    };
    let title_on_disk = || {
        let raw = std::fs::read_to_string(root.join("output").join("publish_config.json")).unwrap();
        let v: serde_json::Value = serde_json::from_str(&raw).unwrap();
        v["clips"][0]["title"].as_str().unwrap_or("").to_string()
    };

    // ① 没动过 ⇒ 跟随项目级
    let v = list().await;
    assert_eq!(v[0]["publish_title"], "标题【主播】", "{v}");
    assert_eq!(v[0]["publish_title_core"], "标题", "只读标题框里不该出现名字：{v}");
    assert_eq!(v[0]["streamer_name"], serde_json::Value::Null, "从没改过 = 跟随（null）: {v}");

    // ② 改成覆盖 ⇒ 锁定成这个名字，磁盘上仍然是纯标题
    let (st, j) = put(serde_json::json!({ "title": "标题", "streamer_name": "片段名" })).await;
    assert_eq!(st, StatusCode::OK, "{j}");
    assert_eq!(title_on_disk(), "标题", "切片产物里不许写名字");
    let v = list().await;
    assert_eq!(v[0]["publish_title"], "标题【片段名】", "{v}");
    assert_eq!(v[0]["streamer_name"], "片段名", "改过 = 锁定: {v}");

    // ③ 改项目级名字 ⇒ 锁定的那条**不跟随**
    yekitrim_core::config::patch_config_file(
        &cfg,
        &serde_json::json!({ "detection": { "vtuber_name": "新主播" } }),
    )
    .unwrap();
    let v = list().await;
    assert_eq!(v[0]["publish_title"], "标题【片段名】", "锁定过的不许被全局改名带走: {v}");

    // ④ 关掉（空串）⇒ 不拼，且**也不跟随**
    let (st, _) = put(serde_json::json!({ "title": "标题", "streamer_name": "" })).await;
    assert_eq!(st, StatusCode::OK);
    let v = list().await;
    assert_eq!(v[0]["publish_title"], "标题", "{v}");

    // ⑤ 清回 null ⇒ 恢复跟随（吃到新的项目级名字）
    let (st, _) = put(serde_json::json!({ "title": "标题", "streamer_name": serde_json::Value::Null })).await;
    assert_eq!(st, StatusCode::OK);
    let v = list().await;
    assert_eq!(v[0]["publish_title"], "标题【新主播】", "{v}");
    assert_eq!(title_on_disk(), "标题");
}

/// 发布标题（2026-11-14 用户要求「产物那里加个标题，方便复制也方便改」）：
/// 写的是 `publish_config.json` 的 `clips[].title`（投稿链路读那份），**绝不动 `clip.json` 的
/// title** —— 后者是片段身份（`clip_meta_matches` 拿它判断重新检测能不能复用目录），改了会让
/// 老片段另开一个目录、精修产物留在旧目录里（用户看到的是「精修白做了」）。
/// 另外两件事一起钉住：`clips_order.txt` 那行标题跟着改；`/clips` 回包要回**改后**的
/// `publish_title`（页面刷新后靠它回填，否则用户会以为没保存）。
#[tokio::test]
async fn clip_title_writes_publish_config_and_leaves_clip_json_alone() {
    let t = test_app().await;
    let root = t.root.clone();
    let clips = root.join("output").join("clips");
    std::fs::create_dir_all(&clips).unwrap();
    write_clip_dir(&clips, 1, 0.0, 10.0, 1, None);
    std::fs::write(
        root.join("output").join("publish_config.json"),
        r#"{"video_source":"a.mp4","clips":[{"directory":"clip-01","title":"旧标题"}]}"#,
    )
    .unwrap();
    std::fs::write(clips.join("clips_order.txt"), "clip-01 旧标题").unwrap();

    let put = |uri: &str, body: serde_json::Value| {
        t.req("PUT", uri)
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap()
    };

    // 首尾空白要去掉（文本框里手滑敲个空格不该原样落盘）
    let res = t
        .call(put(
            "/project-data/clips/clip-01/title?project=1",
            serde_json::json!({ "title": "  新标题（改过）  " }),
        ))
        .await;
    let status = res.status();
    let j = body_json(res).await;
    assert_eq!(status, StatusCode::OK, "{j}");
    assert_eq!(j["title"], "新标题（改过）", "{j}");

    let pv: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(root.join("output").join("publish_config.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(pv["clips"][0]["title"], "新标题（改过）", "{pv}");
    let meta: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(clips.join("clip-01").join("clip.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(meta["title"], "片段1", "clip.json 的标题是片段身份，不许被改: {meta}");
    assert_eq!(
        std::fs::read_to_string(clips.join("clips_order.txt")).unwrap(),
        "clip-01 / 新标题（改过）",
        "clips_order.txt 那行的标题要跟着改"
    );

    // 列表接口回改后的那句（页面刷新靠它回填）
    let res = t
        .call(t.req("GET", "/project-data/clips?project=1").body(Body::empty()).unwrap())
        .await;
    let v = body_json(res).await;
    assert_eq!(v[0]["publish_title"], "新标题（改过）", "{v}");

    // 不存在的片段 → 404；没有发布索引 → 404（不许假装成功）；超长 → 400
    let res = t
        .call(put("/project-data/clips/nope/title?project=1", serde_json::json!({"title":"x"})))
        .await;
    assert_eq!(res.status(), StatusCode::NOT_FOUND);
    std::fs::remove_file(root.join("output").join("publish_config.json")).unwrap();
    let res = t
        .call(put("/project-data/clips/clip-01/title?project=1", serde_json::json!({"title":"x"})))
        .await;
    assert_eq!(res.status(), StatusCode::NOT_FOUND, "没有 publish_config 时不许假装成功");
    let long = "长".repeat(MAX_TITLE_CHARS + 1);
    let res = t
        .call(put("/project-data/clips/clip-01/title?project=1", serde_json::json!({"title": long})))
        .await;
    assert_eq!(res.status(), StatusCode::BAD_REQUEST, "超长标题必须 400");
}

/// **T9**：`CreateEventBody`（`POST /project-data/events`）—— 缺字段 / 类型错一律 400 且**点名字段**。
///
/// ⚠️ 这里特别钉住 `ranges` / `start_sec`：它们是"建一条片段"的核心参数，写错名字就会静默落盘一条
/// **起止 0-10s 的假事件**（改造前就是这样：`get()` 取不到 → `unwrap_or(默认)`），比 400 难查得多。
#[tokio::test]
async fn create_event_body_is_typed_and_names_the_field() {
    let t = test_app().await;
    let uri = "/project-data/events?project=1";

    // 缺 label
    let (status, text) = call_json(&t, "POST", uri, serde_json::json!({})).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "缺 label 必须 400：{text}");
    assert!(text.contains("label"), "报错要点名 label：{text}");

    // label 类型错
    let (status, text) = call_json(&t, "POST", uri, serde_json::json!({ "label": 5 })).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "label 类型错必须 400：{text}");
    assert!(text.contains("label"), "报错要点名 label：{text}");

    // ranges 类型错（不是数组）
    let (status, text) = call_json(
        &t,
        "POST",
        uri,
        serde_json::json!({ "label": "x", "ranges": "nope" }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "ranges 类型错必须 400：{text}");
    assert!(text.contains("ranges"), "报错要点名 ranges：{text}");

    // start_sec 类型错（字符串）
    let (status, text) = call_json(
        &t,
        "POST",
        uri,
        serde_json::json!({ "label": "x", "start_sec": "abc" }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "start_sec 类型错必须 400：{text}");
    assert!(text.contains("start_sec"), "报错要点名 start_sec：{text}");

    // 宽松解析：多传的陌生键照旧忽略（走到下游「标题不能为空」而不是解析错）
    let (status, text) = call_json(
        &t,
        "POST",
        uri,
        serde_json::json!({ "label": "", "nobody_knows": 1 }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(text.contains("标题不能为空"), "多余字段被忽略，于是走原有校验：{text}");
}

/// **T9**：`ClipTitleBody`（`PUT /project-data/clips/:rank/title`）。
///
/// `title` 是**必填**（缺了它这个接口没有任何意义）；`streamer_name` 的三态由 handler 判，
/// 结构体只负责"它得是个 JSON 值"—— 传数字仍然 400，且文案点名它。
#[tokio::test]
async fn clip_title_body_is_typed_and_names_the_field() {
    let t = test_app().await;
    let uri = "/project-data/clips/clip-01/title?project=1";

    let (status, text) = call_json(&t, "PUT", uri, serde_json::json!({})).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "缺 title 必须 400：{text}");
    assert!(text.contains("title"), "报错要点名 title：{text}");

    let (status, text) = call_json(&t, "PUT", uri, serde_json::json!({ "title": 5 })).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "title 类型错必须 400：{text}");
    assert!(text.contains("title"), "报错要点名 title：{text}");

    // 三态之外的写法（数字/null 之外的类型）仍由 handler 明确拒绝，文案点名 streamer_name
    let (status, text) = call_json(
        &t,
        "PUT",
        uri,
        serde_json::json!({ "title": "x", "streamer_name": 5 }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "streamer_name 不是字符串/null 必须 400：{text}");
    assert!(text.contains("streamer_name"), "报错要点名 streamer_name：{text}");
}

}
