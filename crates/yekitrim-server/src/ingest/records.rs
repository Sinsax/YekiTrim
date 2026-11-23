//! 录制记录面：`GET /api/ingest/records`（本人 / admin 的 `?all=1` / `?unassigned=1`）、
//! `GET /api/rooms/:id/recordings`（**只读**）、`POST /api/rooms/:id/recordings/import {rel}`（**单个文件**）。
//!
//! ⚠️ 导入的两道校验别删：文件必须属于该房间（`room_id_from_rel_path`）+ 路径过 `resolve_recorder_path`。

use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use yekitrim_service::db::IngestSessionRow;
use yekitrim_service::ingest::landing;

use crate::auth::CurrentUser;
use crate::model::AppState;

use super::*;
use super::webhook_path::find_danmaku;

/// 会话状态中文标签（前端徽标与记录列表共用）。
pub fn state_label(state: &str) -> &'static str {
    match state {
        "open" => "录制中",
        "closing" => "等待收尾",
        "finalized" => "已建项目",
        "failed" => "失败",
        "unassigned" => "未分配",
        // B站下载通道（P4）
        "queued" => "等待下载",
        "downloading" => "下载中",
        _ => "未知",
    }
}

fn session_json(s: &IngestSessionRow, room_label: Option<&str>, bytes: u64) -> serde_json::Value {
    // 列表页那一列「来源标识」：录播姬是房间号，B站是 BV 号（多P 形如 `BV1xx-p3`）。
    // 两者共用一个字段，前端不用按 kind 分叉渲染。
    let fallback_label = if s.kind == "bilibili" {
        s.source_ref.as_deref().unwrap_or("")
    } else {
        s.room_id.as_deref().unwrap_or("")
    };
    serde_json::json!({
        "session_id": s.session_id,
        // 来源判别（v8 新增列）：前端据此换图标与文案，也是重试语义的分歧点
        "kind": s.kind,
        "source_ref": s.source_ref,
        "room_id": s.room_id,
        "short_id": s.short_id,
        "room_label": room_label.unwrap_or(fallback_label),
        "state": s.state,
        "state_label": state_label(&s.state),
        "files": s.closed_files,
        "opened_files": s.opened_files,
        "file_count": s.closed_files.len(),
        "bytes": bytes,
        "owner_id": s.owner_id,
        "project_id": s.project_id,
        "auto_pipeline": s.auto_pipeline,
        "error": s.error,
        "started_at": s.first_event_at,
        "updated_at": s.last_event_at.or(Some(s.created_at)),
    })
}

/// `GET /api/ingest/records`：本人会话（admin 额外可 `?all=1` 看全平台，或 `?unassigned=1` 看待分配池）。
pub async fn records(
    State(state): State<Arc<AppState>>,
    axum::extract::Extension(user): axum::extract::Extension<CurrentUser>,
    axum::extract::Query(q): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> ApiResult {
    let db = state.auth.db();
    let flag = |k: &str| q.get(k).map(|v| v == "1" || v.eq_ignore_ascii_case("true")).unwrap_or(false);
    let limit = q
        .get("limit")
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(50)
        .min(500);

    let (rows, scope) = if user.is_admin() && flag("unassigned") {
        (db.unassigned_ingest_sessions(limit).map_err(internal)?, "unassigned")
    } else if user.is_admin() && flag("all") {
        (db.list_ingest_sessions(None, &[], limit).map_err(internal)?, "all")
    } else if flag("failed") {
        // 失败记录：每个人都能看自己的（admin 加 ?all=1 看全平台）
        let owner = if user.is_admin() && flag("all") { None } else { Some(user.id) };
        (
            db.list_ingest_sessions(owner, &["failed"], limit).map_err(internal)?,
            "failed",
        )
    } else {
        (
            db.list_ingest_sessions(Some(user.id), &[], limit).map_err(internal)?,
            "mine",
        )
    };

    let out: Vec<serde_json::Value> = rows
        .iter()
        .map(|s| {
            let bytes = db.ingest_file_sizes(&s.closed_files).unwrap_or(0);
            session_json(s, None, bytes)
        })
        .collect();
    ok(serde_json::json!({ "ok": true, "scope": scope, "records": out }))
}

pub(super) fn internal(e: anyhow::Error) -> AppError {
    err(StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}"))
}

/// 单个房间最多列多少条录像（录像目录可能非常大；超出的在响应里报 `total`）。
const RECORDINGS_PAGE: usize = 200;

/// 录像文件在列表里的状态：只关心"入库了没有"，与会话状态机无关。
fn recording_state(
    session: Option<&IngestSessionRow>,
) -> (&'static str, &'static str, Option<i64>) {
    match session {
        Some(s) if s.project_id.is_some() => ("imported", "已入库", s.project_id),
        Some(s) if s.state == "failed" => ("failed", "失败", None),
        Some(_) => ("importing", "入库中", None),
        None => ("new", "未导入", None),
    }
}

/// 归属校验：房间必须是调用者自己的（admin 通吃）。不可见一律 404，不泄露存在性。
///
/// `pub(crate)`（2026-11-14 后端 review B2）：`/api/recorder/recordings` 用同一份判据 ——
/// 同一语义两个端点一套有一道没有，正是 review 指出的问题；**别抄第二份**。
pub(crate) fn owned_room(
    db: &yekitrim_service::db::Db,
    user: &CurrentUser,
    room_id: &str,
) -> Result<yekitrim_service::db::ChannelRule, AppError> {
    db.channel_for_room(room_id)
        .map_err(internal)?
        .filter(|r| user.is_admin() || r.user_id == user.id)
        .ok_or_else(|| err(StatusCode::NOT_FOUND, "直播间未登记"))
}

/// `GET /api/rooms/:room_id/recordings`：该直播间在录像目录里的文件（本人或 admin）。
///
/// **只读**：不登记会话、不改任何库表 —— 用户展开房间行时看到的就是"盘上现在有什么"。
pub async fn room_recordings(
    State(state): State<Arc<AppState>>,
    axum::extract::Extension(user): axum::extract::Extension<CurrentUser>,
    axum::extract::Path(room_id): axum::extract::Path<String>,
) -> ApiResult {
    let db = state.auth.db();
    let room_id = room_id.trim().to_string();
    owned_room(db, &user, &room_id)?;

    let cfg = yekitrim_core::config::load_global_config();
    if cfg.ingest.recorder.root.trim().is_empty() {
        return Err(err(
            StatusCode::BAD_REQUEST,
            "未配置 [ingest.recorder] root，列不出录像（设置页「录播姬（采集）」）",
        ));
    }
    // biliLive-tools 的路径里没有房间号 ⇒ 走它的录像历史；录播姬仍然扫目录。
    let backend = yekitrim_service::recorder::RecorderBackend::from_config(&cfg.ingest);
    let mut mine: Vec<crate::ingest_worker::RecordedFile> =
        if backend.as_ref().is_some_and(|c| c.is_bililive_tools()) {
            let client = backend.as_ref().unwrap();
            let rooms = client
                .list_rooms()
                .await
                .map_err(|e| err(StatusCode::BAD_GATEWAY, format!("{e:#}")))?;
            let Some(st) = rooms.iter().find(|r| r.room_id.to_string() == room_id) else {
                return Err(err(StatusCode::NOT_FOUND, "biliLive-tools 的录制列表里没有这个直播间"));
            };
            let items = client
                .list_history(&room_id, &st.platform)
                .await
                .map_err(|e| err(StatusCode::BAD_GATEWAY, format!("{e:#}")))?;
            let root = std::path::PathBuf::from(cfg.ingest.recorder.root.trim());
            let container_root = cfg.ingest.recorder.container_root.clone();
            let room_for_scan = room_id.clone();
            tokio::task::spawn_blocking(move || {
                let mut out = Vec::new();
                for it in items {
                    let Ok(abs) = crate::ingest::map_webhook_path(&root, &container_root, &it.video_file) else { continue };
                    let Ok(md) = std::fs::metadata(&abs) else { continue };
                    let rel = abs
                        .strip_prefix(&root)
                        .map(|p| p.to_string_lossy().replace('\\', "/"))
                        .unwrap_or_default();
                    if rel.is_empty() {
                        continue;
                    }
                    let name = abs.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    let mtime = md
                        .modified()
                        .ok()
                        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                        .map(|d| d.as_secs() as i64)
                        .unwrap_or(0);
                    out.push(crate::ingest_worker::RecordedFile {
                        rel,
                        name,
                        size: md.len(),
                        mtime,
                        room: Some(room_for_scan.clone()),
                    });
                }
                out
            })
            .await
            .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("列录像线程失败: {e}")))?
        } else {
            // 递归 read_dir + 逐文件 metadata（可能在网络盘上）→ 下 blocking 池
            let cfg_scan = cfg.clone();
            let files = tokio::task::spawn_blocking(move || crate::ingest_worker::list_recordings(&cfg_scan))
                .await
                .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("列目录线程失败: {e}")))?
                .map_err(internal)?;
            files
                .into_iter()
                .filter(|f| f.room.as_deref() == Some(room_id.as_str()))
                .collect()
        };
    // 新的在前（列表第一眼要看到刚录完的）
    mine.sort_by(|a, b| b.mtime.cmp(&a.mtime).then_with(|| a.rel.cmp(&b.rel)));
    let total = mine.len();
    let bytes: u64 = mine.iter().map(|f| f.size).sum();

    let list: Vec<serde_json::Value> = mine
        .into_iter()
        .take(RECORDINGS_PAGE)
        .map(|f| {
            let row = db.ingest_file(&f.rel).ok().flatten();
            let session = row
                .as_ref()
                .and_then(|r| r.session_id.as_deref())
                .and_then(|sid| db.ingest_session(sid).ok().flatten());
            let (state, label, project_id) = recording_state(session.as_ref());
            serde_json::json!({
                "rel": f.rel,
                "name": f.name,
                "size": f.size,
                "mtime": f.mtime,
                "state": state,
                "state_label": label,
                "project_id": project_id,
            })
        })
        .collect();
    ok(serde_json::json!({
        "ok": true,
        "room_id": room_id,
        "total": total,
        "shown": list.len(),
        "bytes": bytes,
        "files": list,
    }))
}

/// `POST /api/rooms/:room_id/recordings/import {rel}`：把**一个**录像文件导入成项目。
///
/// 与兜底扫描同一套会话命名（`scan-<目录名>-<文件名主干>`），所以之后扫描不会再登记一遍；
/// 会话创建后直接置 `closing` 并唤醒 worker（该文件已同时记 opened/closed →
/// `is_finalized()` 立刻为真 → 不用等空闲窗口）。
pub async fn import_recording(
    State(state): State<Arc<AppState>>,
    axum::extract::Extension(user): axum::extract::Extension<CurrentUser>,
    axum::extract::Path(room_id): axum::extract::Path<String>,
    Json(body): Json<serde_json::Value>,
) -> ApiResult {
    let db = state.auth.db();
    let room_id = room_id.trim().to_string();
    let rule = owned_room(db, &user, &room_id)?;

    let rel = body
        .get("rel")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .replace('\\', "/");
    if rel.is_empty() {
        return Err(err(StatusCode::BAD_REQUEST, "缺少 rel（录像相对路径）"));
    }
    // 这个文件必须真的属于这个房间：否则用户能拿任意 rel 把别人的录像导进自己项目。
    // 录播姬靠目录名解析房间号；biliLive-tools 的目录名里没有房间号 ⇒ 用它的录像历史核对。
    let cfg_early = yekitrim_core::config::load_global_config();
    let backend_early =
        yekitrim_service::recorder::RecorderBackend::from_config(&cfg_early.ingest);
    if backend_early.as_ref().is_some_and(|c| c.is_bililive_tools()) {
        let client = backend_early.as_ref().unwrap();
        let rooms = client
            .list_rooms()
            .await
            .map_err(|e| err(StatusCode::BAD_GATEWAY, format!("{e:#}")))?;
        let Some(st) = rooms.iter().find(|r| r.room_id.to_string() == room_id) else {
            return Err(err(StatusCode::NOT_FOUND, "biliLive-tools 的录制列表里没有这个直播间"));
        };
        let items = client
            .list_history(&room_id, &st.platform)
            .await
            .map_err(|e| err(StatusCode::BAD_GATEWAY, format!("{e:#}")))?;
        let root = std::path::Path::new(cfg_early.ingest.recorder.root.trim());
        let cr = cfg_early.ingest.recorder.container_root.as_str();
        let mut belongs = false;
        for it in &items {
            if let Ok(abs) = crate::ingest::map_webhook_path(root, cr, &it.video_file) {
                if let Ok(p) = abs.strip_prefix(root) {
                    if p.to_string_lossy().replace('\\', "/") == rel {
                        belongs = true;
                        break;
                    }
                }
            }
        }
        if !belongs {
            return Err(err(
                StatusCode::BAD_REQUEST,
                format!("这个文件不属于直播间 {room_id}：{rel}"),
            ));
        }

        // 原地建项目（**不搬文件**）：biliLive-tools 的录像历史记的是原路径，搬走会让它那边打不开。
        // 与通用 Webhook 同一语义 —— 工程就建在录像目录里。
        let root = std::path::PathBuf::from(cfg_early.ingest.recorder.root.trim());
        let rel_for_abs = rel.clone();
        let abs = tokio::task::spawn_blocking(move || {
            crate::ingest_worker::resolve_recorder_path(&root, &rel_for_abs)
        })
        .await
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("校验线程失败: {e}")))?
        .map_err(internal)?;
        let parent = abs
            .parent()
            .ok_or_else(|| err(StatusCode::BAD_REQUEST, "无法确定项目目录"))?
            .to_path_buf();
        let input_name = abs
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .ok_or_else(|| err(StatusCode::BAD_REQUEST, "文件名不合法"))?;
        let danmaku_name = find_danmaku(&abs).and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()));
        let title = parent
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| input_name.clone());
        let landed = landing::land_project(
            db,
            &landing::ProjectLanding {
                owner_id: user.id,
                name: title.clone(),
                source_kind: "recorder",
                source_label: Some("biliLive-tools"),
                room: Some(&room_id),
                project_root: &parent,
                input_file: &input_name,
                danmaku_file: danmaku_name.as_deref(),
                session_id: None,
            },
        )
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("创建项目失败: {e:#}")))?;
        return ok(serde_json::json!({
            "ok": true,
            "project_id": landed.project_id,
            "duplicate": !landed.created,
        }));
    } else if yekitrim_service::recorder::room_id_from_rel_path(&rel).as_deref()
        != Some(room_id.as_str())
    {
        return Err(err(
            StatusCode::BAD_REQUEST,
            format!("这个文件不属于直播间 {room_id}：{rel}"),
        ));
    }
    // 已经出过片的不要重复导入（否则会把同一段录像重复建项目）
    if let Some(row) = db.ingest_file(&rel).map_err(internal)? {
        if let Some(s) = row
            .session_id
            .as_deref()
            .and_then(|sid| db.ingest_session(sid).ok().flatten())
        {
            if s.project_id.is_some() {
                return Err(err(StatusCode::CONFLICT, "这个录像已经入库过了"));
            }
        }
    }

    let cfg = yekitrim_core::config::load_global_config();
    let root = std::path::PathBuf::from(cfg.ingest.recorder.root.trim());
    if root.as_os_str().is_empty() {
        return Err(err(StatusCode::BAD_REQUEST, "未配置 [ingest.recorder] root，无法导入"));
    }
    // 路径囚笼 + 必须存在且是文件（与 webhook 同一条校验）；stat 可能落在网络盘 → blocking
    let rel_for_stat = rel.clone();
    let (abs, meta) = tokio::task::spawn_blocking(move || {
        let abs = crate::ingest_worker::resolve_recorder_path(&root, &rel_for_stat)?;
        let meta = std::fs::metadata(&abs)?;
        Ok::<_, anyhow::Error>((abs, meta))
    })
    .await
    .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("校验线程失败: {e}")))?
    .map_err(internal)?;

    let dir = std::path::Path::new(&rel)
        .parent()
        .and_then(|p| p.file_name())
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let stem = abs
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "rec".to_string());
    let session_id = format!("scan-{}-{}", if dir.is_empty() { "root" } else { &dir }, stem);
    let mtime = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);

    db.ingest_session_touch(&session_id, Some(&room_id), None).map_err(internal)?;
    db.ingest_session_add_file(&session_id, &rel, true).map_err(internal)?;
    db.ingest_session_add_file(&session_id, &rel, false).map_err(internal)?;
    db.upsert_ingest_file(&rel, Some(&session_id), Some(&room_id), meta.len(), mtime, "ingested")
        .map_err(internal)?;
    db.assign_ingest_session(&session_id, user.id, Some(&rule.auto_pipeline))
        .map_err(internal)?;
    // 会话刚建出来是 `open`，而入库只认 `closing` → 显式推它一把（与扫描同款）
    db.set_ingest_state(&session_id, "closing", None).map_err(internal)?;
    let _ = db.audit(
        Some(user.id),
        "recording_import",
        &format!("room={room_id} rel={rel}"),
    );
    crate::ingest_worker::notify(state.clone(), Some(session_id.clone()));
    ok(serde_json::json!({ "ok": true, "session_id": session_id }))
}

