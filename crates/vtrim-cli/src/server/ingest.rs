//! 录播姬采集接入（多用户服务 P3，设计稿 §4）。
//!
//! - `POST /api/ingest/bililive/<token>`：webhook 接收端，**只做"验 token → 去重 → 落库"**
//!   然后立即 2xx（官方对非 2xx 会最多重试 3 次且 payload 不变）；
//!   重活（完结判定、合并、落位、跑管线）交给后台 worker（P3b）。
//! - token 存哈希（复用 `service::auth` 的 scrypt），比对失败 401；`allow_ips` 白名单。
//! - 事件处理**不依赖顺序**：`FileOpening/FileClosed` 只累积文件列表，
//!   `SessionEnded` 只把状态推到 `closing`；真正的完结判定在 worker 里（含空闲窗口）。
//!
//! payload 字段名按官方文档是 PascalCase，但官方明确"以后可能新增事件类型"，
//! 这里统一用 [`field`] 做大小写不敏感的多别名取值，避免契约小改动就解析失败。

use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;

use crate::service::db::IngestSessionRow;

use super::auth::CurrentUser;
use super::model::AppState;
use crate::server::auth::ClientIp;

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

/// 大小写不敏感的多别名取值（`SessionId` / `session_id` / `sessionId` 都认）。
///
/// 官方的 `RoomId` / `ShortId` 是**数字**，所以这里同时接受字符串与数字。
fn field<'a>(v: &'a serde_json::Value, keys: &[&str]) -> Option<&'a str> {
    let obj = v.as_object()?;
    for (k, val) in obj {
        for want in keys {
            if k.eq_ignore_ascii_case(want) {
                if let Some(s) = val.as_str() {
                    return Some(s);
                }
            }
        }
    }
    None
}

/// 与 [`field`] 相同，但把数字也转成字符串（`RoomId: 23058`）。
fn field_text(v: &serde_json::Value, keys: &[&str]) -> Option<String> {
    if let Some(s) = field(v, keys) {
        return Some(s.to_string());
    }
    let obj = v.as_object()?;
    for (k, val) in obj {
        for want in keys {
            if k.eq_ignore_ascii_case(want) {
                if let Some(n) = val.as_i64() {
                    return Some(n.to_string());
                }
                if let Some(f) = val.as_f64() {
                    return Some(format!("{}", f as i64));
                }
            }
        }
    }
    None
}

fn field_u64(v: &serde_json::Value, keys: &[&str]) -> Option<u64> {
    let obj = v.as_object()?;
    for (k, val) in obj {
        for want in keys {
            if k.eq_ignore_ascii_case(want) {
                if let Some(n) = val.as_u64() {
                    return Some(n);
                }
                if let Some(s) = val.as_str() {
                    if let Ok(n) = s.parse::<u64>() {
                        return Some(n);
                    }
                }
            }
        }
    }
    None
}

fn field_f64(v: &serde_json::Value, keys: &[&str]) -> Option<f64> {
    let obj = v.as_object()?;
    for (k, val) in obj {
        for want in keys {
            if k.eq_ignore_ascii_case(want) {
                if let Some(n) = val.as_f64() {
                    return Some(n);
                }
                if let Some(s) = val.as_str() {
                    if let Ok(n) = s.parse::<f64>() {
                        return Some(n);
                    }
                }
            }
        }
    }
    None
}

/// 事件类型（`FileOpening` → `file_opening`）用于审计与去重表。
fn normalize_kind(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len() + 4);
    for (i, c) in raw.chars().enumerate() {
        if c.is_ascii_uppercase() {
            if i > 0 {
                out.push('_');
            }
            out.push(c.to_ascii_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}

/// webhook 处理结果（后台 worker 消费）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebhookOutcome {
    pub duplicate: bool,
    pub kind: String,
    pub session_id: Option<String>,
}

/// 把一次 webhook 投递落库（不做重活）。
///
/// 返回 `duplicate=true` 表示同 `EventId` 已处理过，调用方也应回 200。
pub fn handle_event(
    db: &crate::service::db::Db,
    body: &serde_json::Value,
) -> anyhow::Result<WebhookOutcome> {
    let kind_raw = field(body, &["EventType", "event_type"]).unwrap_or_default().to_string();
    let kind = normalize_kind(&kind_raw);
    let event_id = field(body, &["EventId", "event_id"]).map(|s| s.to_string());
    let data = body
        .get("EventData")
        .or_else(|| body.get("event_data"))
        .cloned()
        .unwrap_or(serde_json::Value::Null);

    let session_id = field(&data, &["SessionId", "session_id"]).map(|s| s.to_string());
    let room_id = field_text(&data, &["RoomId", "room_id"]);
    let short_id = field_text(&data, &["ShortId", "short_id"]);
    let rel_path = field(&data, &["RelativePath", "relative_path"]).map(|s| s.to_string());

    // EventId 是官方保证的稳定字段；缺失时退化成"按内容哈希"去重，仍然只处理一次
    let dedupe_key = event_id.unwrap_or_else(|| {
        format!(
            "{}|{}|{}|{}",
            kind,
            session_id.clone().unwrap_or_default(),
            rel_path.clone().unwrap_or_default(),
            field(&data, &["FileCloseTime", "FileOpenTime", "EventTimestamp"]).unwrap_or("")
        )
    });
    let first_time = db.ingest_event_record(&dedupe_key, &kind, session_id.as_deref())?;
    if !first_time {
        return Ok(WebhookOutcome {
            duplicate: true,
            kind,
            session_id,
        });
    }

    if let Some(sid) = session_id.as_deref() {
        db.ingest_session_touch(sid, room_id.as_deref(), short_id.as_deref())?;

        match kind.as_str() {
            "file_opening" => {
                if let Some(rel) = rel_path.as_deref() {
                    db.ingest_session_add_file(sid, rel, true)?;
                }
            }
            "file_closed" => {
                if let Some(rel) = rel_path.as_deref() {
                    db.ingest_session_add_file(sid, rel, false)?;
                    let size = field_u64(&data, &["FileSize", "file_size"]).unwrap_or(0);
                    let duration = field_f64(&data, &["Duration", "duration"]).unwrap_or(0.0);
                    db.upsert_ingest_file(
                        rel,
                        Some(sid),
                        room_id.as_deref(),
                        size,
                        duration as i64,
                        "closed",
                    )?;
                }
            }
            "session_ended" => {
                // 不立即建项目：worker 会在"SessionEnded + 空闲窗口"或"所有 FileOpening 都有
                // 对应 FileClosed"之后统一收尾（见 P3b）。
                let current = db.ingest_session(sid)?;
                if current.map(|s| s.state == "open").unwrap_or(false) {
                    db.set_ingest_state(sid, "closing", None)?;
                }
                // 归属：房间映射存在则直接指派，否则留在待分配池
                let assigned = db
                    .ingest_session(sid)?
                    .and_then(|s| s.owner_id);
                if assigned.is_none() {
                    if let Some(room) = room_id.as_deref() {
                        if let Some(rule) = db.channel_for_room(room)? {
                            if rule.enabled {
                                db.assign_ingest_session(sid, rule.user_id, Some(&rule.auto_pipeline))?;
                            }
                        }
                    }
                }
            }
            "session_started" => {
                if let Some(room) = room_id.as_deref() {
                    if let Some(rule) = db.channel_for_room(room)? {
                        if rule.enabled {
                            db.assign_ingest_session(sid, rule.user_id, Some(&rule.auto_pipeline))?;
                        }
                    }
                }
            }
            _ => {
                // 未知事件（官方声明以后会新增）与 StreamStarted/StreamEnded：只记事件表
            }
        }
    }

    Ok(WebhookOutcome {
        duplicate: false,
        kind,
        session_id,
    })
}

/// `POST /api/ingest/bililive/<token>`：**自带 token 鉴权**，不要求会话。
pub async fn webhook(
    State(state): State<Arc<AppState>>,
    axum::extract::Path(token): axum::extract::Path<String>,
    ClientIp(ip): ClientIp,
    Json(body): Json<serde_json::Value>,
) -> ApiResult {
    let cfg = vtrim_core::config::load_global_config();
    if !cfg.ingest.webhook_enabled || cfg.ingest.token_hash.trim().is_empty() {
        return Err(err(
            StatusCode::FORBIDDEN,
            "录播姬 webhook 未启用（`vtrim ingest token` 生成 token 并开启 [ingest] webhook_enabled）",
        ));
    }
    if !cfg.ingest.allow_ips.is_empty() {
        let allowed = ip
            .as_deref()
            .map(|addr| cfg.ingest.allow_ips.iter().any(|a| a == addr))
            .unwrap_or(false);
        if !allowed {
            return Err(err(
                StatusCode::FORBIDDEN,
                format!("来源地址不在白名单内: {}", ip.unwrap_or_else(|| "-".into())),
            ));
        }
    }

    // scrypt 校验是 CPU 密集：放 blocking 线程池，别占住 tokio worker
    let hash = cfg.ingest.token_hash.clone();
    let token_for_check = token.clone();
    let verified = tokio::task::spawn_blocking(move || {
        crate::service::auth::verify_password(&hash, &token_for_check).unwrap_or(false)
    })
    .await
    .unwrap_or(false);
    if !verified {
        let _ = state
            .auth
            .db()
            .audit_ip(None, "ingest_token_rejected", "", ip.as_deref());
        return Err(err(StatusCode::UNAUTHORIZED, "token 无效"));
    }

    let db = state.auth.db();
    let outcome = match handle_event(db, &body) {
        Ok(o) => o,
        Err(e) => {
            tracing::warn!("录播姬事件落库失败: {e:#}");
            // 仍然回 200：payload 重试也不会成功，而且官方会对非 2xx 重试 3 次
            return ok(serde_json::json!({ "ok": false, "error": format!("{e:#}") }));
        }
    };
    tracing::info!(
        "录播姬事件: kind={} session={:?} duplicate={}",
        outcome.kind,
        outcome.session_id,
        outcome.duplicate
    );

    // 立即 2xx：真正的入库/合并/跑管线由 worker 异步做（P3b 起会在这里唤醒 worker）
    if !outcome.duplicate {
        crate::server::ingest_worker::notify(state.clone(), outcome.session_id.clone());
    }
    ok(serde_json::json!({ "ok": true, "duplicate": outcome.duplicate, "kind": outcome.kind }))
}

/// 会话状态中文标签（前端徽标与记录列表共用）。
pub fn state_label(state: &str) -> &'static str {
    match state {
        "open" => "录制中",
        "closing" => "等待收尾",
        "finalized" => "已建项目",
        "failed" => "失败",
        "unassigned" => "未分配",
        _ => "未知",
    }
}

fn session_json(s: &IngestSessionRow, room_label: Option<&str>, bytes: u64) -> serde_json::Value {
    serde_json::json!({
        "session_id": s.session_id,
        "room_id": s.room_id,
        "short_id": s.short_id,
        "room_label": room_label.unwrap_or_else(|| s.room_id.as_deref().unwrap_or("")),
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

fn internal(e: anyhow::Error) -> AppError {
    err(StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}"))
}

/// `GET /api/channel-map`：本人的房间规则（admin 看全部）。
pub async fn channel_map_get(
    State(state): State<Arc<AppState>>,
    axum::extract::Extension(user): axum::extract::Extension<CurrentUser>,
) -> ApiResult {
    let db = state.auth.db();
    let rules = db.channel_map_list().map_err(internal)?;
    let name_of = |uid: i64| {
        db.list_users()
            .ok()
            .and_then(|us| us.into_iter().find(|u| u.id == uid))
            .map(|u| u.username)
    };
    let out: Vec<serde_json::Value> = rules
        .iter()
        .filter(|r| user.is_admin() || r.user_id == user.id)
        .map(|r| {
            serde_json::json!({
                "room_id": r.room_id,
                "user_id": r.user_id,
                "username": name_of(r.user_id),
                "auto_pipeline": r.auto_pipeline,
                "enabled": r.enabled,
            })
        })
        .collect();
    ok(serde_json::json!({ "ok": true, "rules": out }))
}

/// `PUT /api/channel-map`：整体替换**本人**的房间规则。
pub async fn channel_map_put(
    State(state): State<Arc<AppState>>,
    axum::extract::Extension(user): axum::extract::Extension<CurrentUser>,
    Json(body): Json<serde_json::Value>,
) -> ApiResult {
    let Some(list) = body.get("rules").and_then(|v| v.as_array()) else {
        return Err(err(StatusCode::BAD_REQUEST, "缺少 rules 数组"));
    };
    let mut rules: Vec<(String, String, bool)> = Vec::new();
    for item in list {
        let room = item.get("room_id").and_then(|v| v.as_str()).unwrap_or("").trim().to_string();
        if room.is_empty() {
            return Err(err(StatusCode::BAD_REQUEST, "room_id 不能为空"));
        }
        let pipeline = item
            .get("auto_pipeline")
            .and_then(|v| v.as_str())
            .unwrap_or("detect")
            .to_string();
        if !matches!(pipeline.as_str(), "none" | "transcribe" | "detect" | "cut") {
            return Err(err(
                StatusCode::BAD_REQUEST,
                format!("auto_pipeline 只能是 none/transcribe/detect/cut（收到 {pipeline}）"),
            ));
        }
        let enabled = item.get("enabled").and_then(|v| v.as_bool()).unwrap_or(true);
        rules.push((room, pipeline, enabled));
    }
    let db = state.auth.db();
    db.replace_channel_map(user.id, &rules).map_err(internal)?;
    let _ = db.audit(
        Some(user.id),
        "channel_map_update",
        &format!("rules={}", rules.len()),
    );
    channel_map_get(State(state), axum::extract::Extension(user)).await
}

/// `POST /api/admin/ingest/assign {session_id, user_id, auto_pipeline}`（仅 admin）。
pub async fn assign(
    State(state): State<Arc<AppState>>,
    axum::extract::Extension(user): axum::extract::Extension<CurrentUser>,
    Json(body): Json<serde_json::Value>,
) -> ApiResult {
    let session_id = body
        .get("session_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| err(StatusCode::BAD_REQUEST, "缺少 session_id"))?;
    let target = body
        .get("user_id")
        .and_then(|v| v.as_i64())
        .ok_or_else(|| err(StatusCode::BAD_REQUEST, "缺少 user_id"))?;
    let pipeline = body.get("auto_pipeline").and_then(|v| v.as_str());
    let db = state.auth.db();
    if db.find_user_by_id(target).map_err(internal)?.is_none() {
        return Err(err(StatusCode::NOT_FOUND, "目标账号不存在"));
    }
    if !db.assign_ingest_session(session_id, target, pipeline).map_err(internal)? {
        return Err(err(StatusCode::NOT_FOUND, "会话不存在"));
    }
    let _ = db.audit(
        Some(user.id),
        "ingest_assign",
        &format!("session={session_id} user={target}"),
    );
    // 指派后立刻唤醒 worker：现在有归属了，可以入库
    crate::server::ingest_worker::notify(state.clone(), Some(session_id.to_string()));
    ok(serde_json::json!({ "ok": true }))
}

/// `POST /api/ingest/retry {session_id}`：把失败会话重新排队（本人或 admin）。
pub async fn retry(
    State(state): State<Arc<AppState>>,
    axum::extract::Extension(user): axum::extract::Extension<CurrentUser>,
    Json(body): Json<serde_json::Value>,
) -> ApiResult {
    let session_id = body
        .get("session_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| err(StatusCode::BAD_REQUEST, "缺少 session_id"))?;
    let db = state.auth.db();
    let Some(session) = db.ingest_session(session_id).map_err(internal)? else {
        return Err(err(StatusCode::NOT_FOUND, "会话不存在"));
    };
    if !user.is_admin() && session.owner_id != Some(user.id) {
        return Err(err(StatusCode::NOT_FOUND, "会话不存在"));
    }
    db.set_ingest_state(session_id, "closing", None).map_err(internal)?;
    let _ = db.audit(Some(user.id), "ingest_retry", session_id);
    crate::server::ingest_worker::notify(state.clone(), Some(session_id.to_string()));
    ok(serde_json::json!({ "ok": true }))
}

/// `POST /api/admin/ingest/scan`：立刻跑一次兜底扫描（仅 admin）。
///
/// 返回本次新登记的完整录像数；用于"历史录播导入"与排障。
pub async fn scan_now(State(state): State<Arc<AppState>>) -> ApiResult {
    let cfg = vtrim_core::config::load_global_config();
    if cfg.ingest.recorder.root.trim().is_empty() {
        return Err(err(
            StatusCode::BAD_REQUEST,
            "未配置 [ingest.recorder] root，无法扫描",
        ));
    }
    let n = crate::server::ingest_worker::scan_recorder(&state, &cfg)
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}")))?;
    // 扫描可能产生新的可入库会话：立刻处理一轮
    crate::server::ingest_worker::notify(state, None);
    ok(serde_json::json!({ "ok": true, "registered": n }))
}

/// `GET /api/admin/ingest`：录播姬配置概览（不含 token 明文，只报是否已配置）。
pub async fn config_view() -> ApiResult {
    let cfg = vtrim_core::config::load_global_config();
    ok(serde_json::json!({
        "ok": true,
        "webhook_enabled": cfg.ingest.webhook_enabled,
        "token_configured": !cfg.ingest.token_hash.trim().is_empty(),
        "allow_ips": cfg.ingest.allow_ips,
        "recorder": {
            "root": cfg.ingest.recorder.root,
            "watch_interval_mins": cfg.ingest.recorder.watch_interval_mins,
            "finalize_idle_mins": cfg.ingest.recorder.finalize_idle_mins,
            "move_into_managed": cfg.ingest.recorder.move_into_managed,
            "concat_files": cfg.ingest.recorder.concat_files,
            "extensions": cfg.ingest.recorder.extensions,
            "auto_pipeline": cfg.ingest.recorder.auto_pipeline,
        },
        "webhook_path": "/api/ingest/bililive/<token>",
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::db::Db;

    fn db() -> (tempfile::TempDir, Db) {
        let dir = tempfile::tempdir().unwrap();
        let db = Db::open(&dir.path().join("vtrim.db")).unwrap();
        (dir, db)
    }

    fn ev(kind: &str, id: &str, session: &str, extra: serde_json::Value) -> serde_json::Value {
        let mut data = serde_json::json!({
            "SessionId": session,
            "RoomId": 23058,
            "RelativePath": "23058/rec.flv",
            "FileSize": 1234,
        });
        if let (Some(dst), Some(src)) = (data.as_object_mut(), extra.as_object()) {
            for (k, v) in src {
                dst.insert(k.clone(), v.clone());
            }
        }
        serde_json::json!({ "EventType": kind, "EventId": id, "EventData": data })
    }

    #[test]
    fn normalizes_event_kinds() {
        assert_eq!(normalize_kind("FileClosed"), "file_closed");
        assert_eq!(normalize_kind("SessionStarted"), "session_started");
        assert_eq!(normalize_kind("UnknownThing"), "unknown_thing");
    }

    #[test]
    fn field_lookup_is_case_insensitive_with_aliases() {
        let v = serde_json::json!({ "session_id": "abc", "FileSize": 42 });
        assert_eq!(field(&v, &["SessionId", "session_id"]), Some("abc"));
        assert_eq!(field_u64(&v, &["FileSize"]), Some(42));
        assert_eq!(field(&v, &["Missing"]), None);
    }

    #[test]
    fn webhook_flow_dedupes_and_accumulates_files() {
        let (_d, db) = db();
        // 乱序：先 SessionEnded 再 FileClosed（官方不保证顺序）
        let ended = ev("SessionEnded", "e1", "s1", serde_json::json!({}));
        assert!(!handle_event(&db, &ended).unwrap().duplicate);
        assert_eq!(db.ingest_session("s1").unwrap().unwrap().state, "closing");

        let closed = ev(
            "FileClosed",
            "e2",
            "s1",
            serde_json::json!({ "Duration": 3600.5 }),
        );
        assert!(!handle_event(&db, &closed).unwrap().duplicate);
        let s = db.ingest_session("s1").unwrap().unwrap();
        assert_eq!(s.closed_files, vec!["23058/rec.flv".to_string()]);
        assert_eq!(s.room_id.as_deref(), Some("23058"));
        // 乱序不应把 closing 退回 open
        assert_eq!(s.state, "closing");

        // 重复投递：同 EventId 只处理一次
        assert!(handle_event(&db, &closed).unwrap().duplicate);
        assert_eq!(db.ingest_session("s1").unwrap().unwrap().closed_files.len(), 1);

        // 未知事件类型：忽略但不报错
        let unknown = ev("SomethingNew", "e3", "s1", serde_json::json!({}));
        assert!(!handle_event(&db, &unknown).unwrap().duplicate);
        assert_eq!(unknown["EventType"], "SomethingNew");
    }

    #[test]
    fn channel_map_assigns_owner_on_session_started() {
        let (_d, db) = db();
        let uid = db.create_user("alice", "h", "user", false, 0).unwrap();
        db.replace_channel_map(uid, &[("23058".into(), "cut".into(), true)])
            .unwrap();

        handle_event(&db, &ev("SessionStarted", "e1", "s1", serde_json::json!({}))).unwrap();
        let s = db.ingest_session("s1").unwrap().unwrap();
        assert_eq!(s.owner_id, Some(uid));
        assert_eq!(s.auto_pipeline.as_deref(), Some("cut"));

        // 未映射房间 → 待分配池
        let mut other = ev("SessionStarted", "e2", "s2", serde_json::json!({}));
        other["EventData"]["RoomId"] = serde_json::json!(99999);
        handle_event(&db, &other).unwrap();
        assert!(db.ingest_session("s2").unwrap().unwrap().owner_id.is_none());
        assert_eq!(db.unassigned_ingest_sessions(10).unwrap().len(), 1);

        // 指派后从待分配池消失
        db.assign_ingest_session("s2", uid, Some("detect")).unwrap();
        assert_eq!(db.unassigned_ingest_sessions(10).unwrap().len(), 0);
    }
}
