//! 录播姬采集接入（多用户服务 P3，设计稿 §4）。
//!
//! - `POST /api/ingest/bililive/<token>`：webhook 接收端，**只做"验 token → 去重 → 落库"**
//!   然后立即 2xx（官方对非 2xx 会最多重试 3 次且 payload 不变）；
//!   重活（完结判定、合并、落位、跑管线）交给后台 worker（P3b）。
//! - token 存哈希（复用 `yekitrim_service::auth` 的 scrypt），比对失败 401；`allow_ips` 白名单。
//! - 事件处理**不依赖顺序**：`FileOpening/FileClosed` 只累积文件列表，
//!   `SessionEnded` 只把状态推到 `closing`；真正的完结判定在 worker 里（含空闲窗口）。
//!
//! payload 字段名按官方文档是 PascalCase，但官方明确"以后可能新增事件类型"，
//! 这里统一用 [`field`] 做大小写不敏感的多别名取值，避免契约小改动就解析失败。

use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;

use yekitrim_service::db::IngestSessionRow;

use super::auth::CurrentUser;
use super::model::AppState;
use crate::auth::ClientIp;

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

/// 把同步 IO 搬出 async worker（与 `api::blocking` / `scope::off` 同款，只是用本文件自己的 `AppError`）。
///
/// ⚠️ 本文件的接口里有**未认证**入口（录播姬 webhook），读配置、写审计、事件落库全是同步 IO
/// —— 项目/数据目录在网络盘上时一次 `read_to_string` 就是几百毫秒，十几个 webhook 就能把
/// tokio worker 占满。细则见 `src/api/AGENTS.md`「同步 IO 纪律」。
async fn off<T, F>(f: F) -> Result<T, AppError>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, AppError> + Send + 'static,
{
    match tokio::task::spawn_blocking(f).await {
        Ok(r) => r,
        Err(e) => Err(err(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("阻塞任务失败: {e}"),
        )),
    }
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
    db: &yekitrim_service::db::Db,
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

/// token 的形状检查：`yekitrim_` + 40 位 base64url。
///
/// ⚠️ **前缀必须与生成端同源**：`yekitrim_service::ingest::TOKEN_PREFIX`。
/// 这里曾经是**另一个字面量**，改名时只改了生成端 ⇒ 产出的每个 token 都通不过本闸 ⇒
/// **用户在设置页拿到的 token 100% 401**（采集整条链不可用，且只回 401、无其他线索）。
/// 现在两端共用同一个常量，不可能再分叉。
///
/// 放在 scrypt **之前**当快速否掉用：形状不对的输入根本没可能是 token，为它付一次
/// N=2^15（约 32MiB 内存 + 几十毫秒 CPU）的代价是白送对方的放大器。
fn token_shape_ok(token: &str) -> bool {
    let Some(body) = token.strip_prefix(yekitrim_service::ingest::TOKEN_PREFIX) else {
        return false;
    };
    body.len() == 40
        && body
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// `POST /api/ingest/bililive/<token>`：**自带 token 鉴权**，不要求会话。
pub async fn webhook(
    State(state): State<Arc<AppState>>,
    axum::extract::Path(token): axum::extract::Path<String>,
    ClientIp(ip): ClientIp,
    Json(body): Json<serde_json::Value>,
) -> ApiResult {
    // ⚠️ 未认证入口：连"读全局配置"都不该留在 async worker 上（读的是磁盘上的 config.toml）
    let cfg = off(|| Ok(yekitrim_core::config::load_global_config())).await?;
    if !cfg.ingest.webhook_enabled || cfg.ingest.token_hash.trim().is_empty() {
        return Err(err(
            StatusCode::FORBIDDEN,
            "录播姬 webhook 未启用（`yekitrim ingest token` 生成 token 并开启 [ingest] webhook_enabled）",
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

    // 限流：这是**未认证**入口，而每次校验失败都要跑一次 scrypt（N=2^15，约 32MiB/次），
    // 不限流的话任何人都能拿它当 CPU/内存放大器（见 `ratelimit::check_webhook`）。
    if let Some(ip) = ip.as_deref() {
        if let Err(wait) = super::ratelimit::check_webhook(ip) {
            return Err(err(
                StatusCode::TOO_MANY_REQUESTS,
                format!("请求过于频繁，请 {wait} 秒后再试"),
            ));
        }
    }
    // 形状先否掉：一眼不可能成立的字符串没必要付 scrypt 的代价。
    // ⚠️ 这里**不写 audit**：那是 DB 写，会变成另一条"用垃圾请求放大开销"的路子
    // （真正的 token 校验失败仍然照旧审计）。
    if !token_shape_ok(&token) {
        return Err(err(StatusCode::UNAUTHORIZED, "token 无效"));
    }

    // scrypt 校验是 CPU 密集：放 blocking 线程池，别占住 tokio worker
    let hash = cfg.ingest.token_hash.clone();
    let token_for_check = token.clone();
    let verified = tokio::task::spawn_blocking(move || {
        yekitrim_service::auth::verify_password(&hash, &token_for_check).unwrap_or(false)
    })
    .await
    .unwrap_or(false);
    if !verified {
        // 审计是同步 DB 写，且这是失败路径（可能被刷）→ 同样搬出 worker
        let db = state.auth.db_arc();
        let ip_for_audit = ip.clone();
        let _ = off(move || {
            let _ = db.audit_ip(None, "ingest_token_rejected", "", ip_for_audit.as_deref());
            Ok(())
        })
        .await;
        return Err(err(StatusCode::UNAUTHORIZED, "token 无效"));
    }

    // 事件落库全是同步 SQLite 写 → blocking。错误语义保持原样：
    // "落库失败"仍然回 200（payload 重试也不会成功，而官方会对非 2xx 重试 3 次）。
    let db = state.auth.db_arc();
    let body_for_event = body.clone();
    let handled = off(move || {
        Ok(handle_event(&db, &body_for_event).map_err(|e| format!("{e:#}")))
    })
    .await?;
    let outcome = match handled {
        Ok(o) => o,
        Err(e) => {
            tracing::warn!("录播姬事件落库失败: {e}");
            return ok(serde_json::json!({ "ok": false, "error": e }));
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
        crate::ingest_worker::notify(state.clone(), outcome.session_id.clone());
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

fn internal(e: anyhow::Error) -> AppError {
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
fn owned_room(
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
    // 递归 read_dir + 逐文件 metadata（可能在网络盘上）→ 下 blocking 池
    let cfg_scan = cfg.clone();
    let files = tokio::task::spawn_blocking(move || crate::ingest_worker::list_recordings(&cfg_scan))
        .await
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("列目录线程失败: {e}")))?
        .map_err(internal)?;

    let mut mine: Vec<_> = files
        .into_iter()
        .filter(|f| f.room.as_deref() == Some(room_id.as_str()))
        .collect();
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
    // 这个文件必须真的属于这个房间：否则用户能拿任意 rel 把别人的录像导进自己项目
    if yekitrim_service::recorder::room_id_from_rel_path(&rel).as_deref() != Some(room_id.as_str()) {
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
    crate::ingest_worker::notify(state.clone(), Some(session_id.to_string()));
    ok(serde_json::json!({ "ok": true }))
}

/// `POST /api/ingest/retry {session_id}`：把失败会话重新排队（本人或 admin）。
///
/// ⚠️ 目标状态**按来源分派**：录播姬靠 `state == "closing"` 触发入库判定
/// （`ingest_worker::is_finalized`），B站下载则靠 `"queued"` 等下载 worker 认领。
/// 早先这里写死 `"closing"`，B站会话点"重试"会**永远不动**（下载 worker 只认 queued）。
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
    let requeued = if session.kind == "bilibili" {
        db.set_ingest_state(session_id, "queued", None).map_err(internal)?;
        crate::download_worker::notify(state.clone(), Some(session_id.to_string()));
        "queued"
    } else {
        db.set_ingest_state(session_id, "closing", None).map_err(internal)?;
        crate::ingest_worker::notify(state.clone(), Some(session_id.to_string()));
        "closing"
    };
    let _ = db.audit(Some(user.id), "ingest_retry", session_id);
    ok(serde_json::json!({ "ok": true, "state": requeued }))
}

/// `POST /api/admin/ingest/scan`：立刻跑一次兜底扫描（仅 admin）。
///
/// 返回本次新登记的完整录像数；用于"历史录播导入"与排障。
pub async fn scan_now(State(state): State<Arc<AppState>>) -> ApiResult {
    let cfg = yekitrim_core::config::load_global_config();
    if cfg.ingest.recorder.root.trim().is_empty() {
        return Err(err(
            StatusCode::BAD_REQUEST,
            "未配置 [ingest.recorder] root，无法扫描",
        ));
    }
    // 扫描是递归 read_dir + metadata（可能在网络盘上）→ 下 blocking 池，别占 worker
    let st = state.clone();
    let n = tokio::task::spawn_blocking(move || crate::ingest_worker::scan_recorder(&st, &cfg))
        .await
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("扫描线程失败: {e}")))?
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}")))?;
    // 扫描可能产生新的可入库会话：立刻处理一轮
    crate::ingest_worker::notify(state, None);
    ok(serde_json::json!({ "ok": true, "registered": n }))
}

/// 配置概览的 JSON（`GET /api/admin/ingest` 与两个写接口的响应共用）。
///
/// ⚠️ **`api_pass` 只回"是否已设置"，`token_hash` 一个字都不回** —— 表单回填不需要明文，
/// 而把口令塞进表单字段的后果是"用户没改也会被原样写回"。要看原文走 `GET /api/config`
/// 的 `global_raw`（admin 专用，也是源码编辑器 round-trip 的前提）。
fn config_view_json() -> serde_json::Value {
    let cfg = yekitrim_core::config::load_global_config();
    serde_json::json!({
        "ok": true,
        "webhook_enabled": cfg.ingest.webhook_enabled,
        "token_configured": !cfg.ingest.token_hash.trim().is_empty(),
        "allow_ips": cfg.ingest.allow_ips,
        "recorder": {
            "root": cfg.ingest.recorder.root,
            "api_base": cfg.ingest.recorder.api_base,
            "api_user": cfg.ingest.recorder.api_user,
            "api_pass_set": !cfg.ingest.recorder.api_pass.is_empty(),
            "watch_interval_mins": cfg.ingest.recorder.watch_interval_mins,
            "finalize_idle_mins": cfg.ingest.recorder.finalize_idle_mins,
            "move_into_managed": cfg.ingest.recorder.move_into_managed,
            "concat_files": cfg.ingest.recorder.concat_files,
            "extensions": cfg.ingest.recorder.extensions,
            "auto_pipeline": cfg.ingest.recorder.auto_pipeline,
        },
        // B站下载通道（P4）。前端表单只要这几项；`cookie_file` 是**路径**，
        // 但它是本机配置项（不是会话 cookie 内容），且这个接口本身仅 admin 可读。
        //
        // ⚠️ 分组名叫 `download` 而**不是** `downloader`：`[ingest] downloader` 是一个
        // **字符串键**（引擎名），把它当成子表分组名会让读代码的人以为配置里有个
        // `[ingest.downloader]` 子表 —— 那正是 `build_ingest_patch` 要小心的坑。
        "download": {
            "engine": cfg.ingest.downloader,
            "ytdlp_path": cfg.ingest.ytdlp_path,
            "max_quality": cfg.ingest.max_quality,
            "rate_limit_kbps": cfg.ingest.rate_limit_kbps,
            "max_concurrent_downloads": cfg.ingest.max_concurrent_downloads,
            "cookie_file": cfg.ingest.cookie_file,
            "idle_timeout_secs": cfg.ingest.download_idle_timeout_secs,
        },
        // 前端拿 `location.origin` 拼完整地址；录播姬不在本机时把主机名换成 YekiTrim 的局域网 IP
        "webhook_path": "/api/ingest/bililive/<token>",
        // 项目存放位置（`[storage] projects_root`）。**这是"下载/入库落到哪"的唯一开关**，
        // 与 B站通道无关（录播姬入库、本机导入同一个根），但设置页把它放在下载器卡里 ——
        // 用户问"下载到哪"时先在那一块找。
        //
        // `projects_root` 是配置原值（空 = 用默认），`projects_root_effective` 是**解析后**
        // 的真实落点：只回值不回结构，前端照原样展示即可（别让它自己拼默认路径）。
        "storage": {
            "projects_root": cfg.storage.projects_root,
            "projects_root_effective": yekitrim_core::path::display(
                &yekitrim_service::projects::projects_root(),
            ),
        },
    })
}

/// `GET /api/admin/ingest`：录播姬配置概览（不含 token 明文，只报是否已配置）。
pub async fn config_view() -> ApiResult {
    ok(config_view_json())
}

/// 允许的自动入库终点（与 `RecorderConfig::auto_pipeline` 的取值一致）。
const AUTO_PIPELINES: [&str; 4] = ["none", "transcribe", "detect", "cut"];

/// 把 `PUT /api/admin/ingest` 的 body 变成"要合并进配置的字段" + 警告列表。
///
/// 抽成纯函数有两个理由：校验规则能直接单测（不需要起服务、也不碰真实全局配置），
/// 而且"哪些键会被写"这件事在一处看得完。
///
/// 返回值是 `(要并进 [ingest] 的字段, 要并进 [storage] 的字段, 警告)` ——
/// `[storage]` 那一份只有 `projects_root` 一个键（项目存放位置），
/// 它跟采集无关，但设置页把它摆在下载器卡里，所以走同一个保存端点。
///
/// 语义（表单友好，关键是让"清空"与"不改"分得开）：
/// - 键**缺席** = 保持原值；
/// - 字符串给 `""` = 显式清空（`api_pass` 同此：给空串才清掉，缺省则保留原密码；
///   `projects_root` 给空串 = 回默认的"程序目录下"）；
/// - `allow_ips` 给 `[]` = 清空白名单。
fn build_ingest_patch(
    body: &serde_json::Value,
) -> Result<
    (
        serde_json::Map<String, serde_json::Value>,
        serde_json::Map<String, serde_json::Value>,
        Vec<String>,
    ),
    AppError,
> {
    let mut ingest = serde_json::Map::new();
    let mut storage = serde_json::Map::new();
    let mut warnings: Vec<String> = Vec::new();

    if let Some(v) = body.get("webhook_enabled") {
        let b = v
            .as_bool()
            .ok_or_else(|| err(StatusCode::BAD_REQUEST, "webhook_enabled 必须是布尔值"))?;
        ingest.insert("webhook_enabled".into(), serde_json::json!(b));
    }

    if let Some(v) = body.get("allow_ips") {
        let arr = v
            .as_array()
            .ok_or_else(|| err(StatusCode::BAD_REQUEST, "allow_ips 必须是字符串数组"))?;
        let mut ips: Vec<String> = Vec::new();
        for item in arr {
            let s = item.as_str().unwrap_or("").trim();
            if s.is_empty() {
                continue;
            }
            // 运行时是**精确字符串相等**（webhook 校验那段），所以只收 IP 字面量：
            // 放行 `192.168.1.0/24` 这种写法只会让来源被静默拦掉、还很难查。
            if s.parse::<std::net::IpAddr>().is_err() {
                return Err(err(
                    StatusCode::BAD_REQUEST,
                    format!("allow_ips 只接受单个 IP（不支持网段/CIDR，也不接受主机名）：{s}"),
                ));
            }
            if !ips.iter().any(|x| x == s) {
                ips.push(s.to_string());
            }
        }
        ingest.insert("allow_ips".into(), serde_json::json!(ips));
    }

    if let Some(rec) = body.get("recorder") {
        let rec = rec
            .as_object()
            .ok_or_else(|| err(StatusCode::BAD_REQUEST, "recorder 必须是对象"))?;
        let mut out = serde_json::Map::new();

        for key in ["root", "api_base", "api_user"] {
            if let Some(v) = rec.get(key) {
                let s = v
                    .as_str()
                    .ok_or_else(|| err(StatusCode::BAD_REQUEST, format!("recorder.{key} 必须是字符串")))?
                    .trim();
                out.insert(key.into(), serde_json::json!(s));
            }
        }
        // 口令**不 trim**：首尾空格是合法字符，替用户"修"掉比留着更容易出怪问题
        if let Some(v) = rec.get("api_pass") {
            let s = v
                .as_str()
                .ok_or_else(|| err(StatusCode::BAD_REQUEST, "recorder.api_pass 必须是字符串"))?;
            out.insert("api_pass".into(), serde_json::json!(s));
        }
        if let Some(v) = rec.get("auto_pipeline") {
            let s = v
                .as_str()
                .ok_or_else(|| err(StatusCode::BAD_REQUEST, "recorder.auto_pipeline 必须是字符串"))?;
            if !AUTO_PIPELINES.contains(&s) {
                return Err(err(
                    StatusCode::BAD_REQUEST,
                    format!("recorder.auto_pipeline 只能是 {} 之一：{s}", AUTO_PIPELINES.join(" / ")),
                ));
            }
            out.insert("auto_pipeline".into(), serde_json::json!(s));
        }
        if let Some(v) = rec.get("watch_interval_mins") {
            let n = v
                .as_u64()
                .ok_or_else(|| err(StatusCode::BAD_REQUEST, "recorder.watch_interval_mins 必须是非负整数"))?;
            out.insert("watch_interval_mins".into(), serde_json::json!(n));
        }

        // 目录不可达**不拒绝**：网络盘/跨机部署时它可能只是暂时没挂上，直接 400 会让人
        // 连"先存下来"都做不到。改成回一条 warning，前端显示出来。
        if let Some(root) = out.get("root").and_then(|v| v.as_str()) {
            if !root.is_empty() && !std::path::Path::new(root).is_dir() {
                warnings.push(format!(
                    "录像目录当前不可访问（{root}）：兜底扫描会跳过它。若在网络盘/另一台机器上，请确认 YekiTrim 进程也读得到这个路径。"
                ));
            }
        }
        if let Some(base) = out.get("api_base").and_then(|v| v.as_str()) {
            if !base.is_empty() && !(base.starts_with("http://") || base.starts_with("https://")) {
                return Err(err(
                    StatusCode::BAD_REQUEST,
                    format!("recorder.api_base 需要以 http:// 或 https:// 开头（例 http://127.0.0.1:2356）：{base}"),
                ));
            }
        }

        ingest.insert("recorder".into(), serde_json::Value::Object(out));
    }

    // B站下载通道（P4）。⚠️ 这里必须**平铺**成 `[ingest]` 的扁平键，不能嵌套成子表：
    // `[ingest] downloader` 本身是一个**字符串键**（引擎名）。提交 `{"downloader": {…}}`
    // 会把这一项写成 inline table —— 文件仍是合法 TOML、写入也不报错，
    // 但下次读配置时 `downloader` 反序列化成 `String` 直接失败，**整份配置被静默忽略**。
    // 回归钉子见 `patch_download_keys_are_flat_not_nested`。
    if let Some(v) = body.get("download") {
        let d = v
            .as_object()
            .ok_or_else(|| err(StatusCode::BAD_REQUEST, "download 必须是对象"))?;

        for (key, field) in [
            ("engine", "downloader"),
            ("ytdlp_path", "ytdlp_path"),
            ("cookie_file", "cookie_file"),
        ] {
            if let Some(x) = d.get(key) {
                let s = x
                    .as_str()
                    .ok_or_else(|| err(StatusCode::BAD_REQUEST, format!("download.{key} 必须是字符串")))?
                    .trim();
                ingest.insert(field.into(), serde_json::json!(s));
            }
        }

        if let Some(x) = d.get("max_quality") {
            let s = x
                .as_str()
                .ok_or_else(|| err(StatusCode::BAD_REQUEST, "download.max_quality 必须是字符串"))?;
            check_quality(s)
                .map_err(|r| err(StatusCode::BAD_REQUEST, format!("download.max_quality：{r}")))?;
            ingest.insert("max_quality".into(), serde_json::json!(s.trim()));
        }

        for (key, field) in [
            ("rate_limit_kbps", "rate_limit_kbps"),
            ("max_concurrent_downloads", "max_concurrent_downloads"),
            ("idle_timeout_secs", "download_idle_timeout_secs"),
        ] {
            if let Some(x) = d.get(key) {
                let n = x
                    .as_u64()
                    .ok_or_else(|| err(StatusCode::BAD_REQUEST, format!("download.{key} 必须是非负整数")))?;
                // 0 是合法的（不限速 / 不限并发由 core 侧 `.max(1)` 兜住）；
                // 但并发调大就是给 B站风控送人头，这里给一条 warning 说清楚。
                if key == "max_concurrent_downloads" && n > 2 {
                    warnings.push(format!(
                        "同时下载数设为 {n}：B站的 412 风控与 IP 频次相关，调大更容易被拦（建议 1）"
                    ));
                }
                ingest.insert(field.into(), serde_json::json!(n));
            }
        }

        // 自备路径/引擎名写错了不拒绝（可能只是文件还没放好），但要提醒
        if let Some(p) = ingest.get("ytdlp_path").and_then(|v| v.as_str()) {
            if !p.is_empty() && !std::path::Path::new(p).is_file() {
                warnings.push(format!(
                    "yt-dlp 路径当前不可执行（{p}）：会自动回退到托管副本或 PATH 里的 yt-dlp。"
                ));
            }
        }
    }

    // 项目存放位置（`[storage] projects_root`）：下载与入库的**共同**落点。
    // 放在 `[storage]` 而不是 `[ingest]` —— 它是平台级存储配置，录播姬入库/本机导入也用同一个根。
    if let Some(v) = body.get("storage") {
        let s = v
            .as_object()
            .ok_or_else(|| err(StatusCode::BAD_REQUEST, "storage 必须是对象"))?;
        if let Some(x) = s.get("projects_root") {
            let raw = x
                .as_str()
                .ok_or_else(|| err(StatusCode::BAD_REQUEST, "storage.projects_root 必须是字符串"))?;
            storage.insert("projects_root".into(), serde_json::json!(check_projects_root(raw)?));
        }
    }

    Ok((ingest, storage, warnings))
}

/// 校验 `[storage] projects_root`，返回**要写进配置的值**（`""` = 回默认）。
///
/// 与 `recorder.root` 那条"不可达只 warning"不同 —— 那个目录是**别人写的**（录播姬），
/// 我们只能读、不能替它建；而这是**我们自己要往里写**的根，所以在这里就把它建出来：
/// 等到下载跑完才发现目录建不了，用户已经白等了。
///
/// 判定顺序（先便宜后昂贵）：
/// 1. 空 → 回默认（程序目录下的 `.yekitrim/projects`）；
/// 2. 必须是**绝对路径** —— 相对路径的基准是进程 cwd，服务重启后可能换目录，落点会漂；
/// 3. 已存在但不是目录 → 拒（填成了某个文件）；
/// 4. `create_dir_all` + 真实写一个探针文件再删（`disk::writable`，与 `yekitrim doctor` 同一套判据）
///    → 建不出来或写不进去就拒。
fn check_projects_root(raw: &str) -> Result<String, AppError> {
    let s = raw.trim();
    if s.is_empty() {
        return Ok(String::new());
    }
    let p = std::path::Path::new(s);
    if !p.is_absolute() {
        return Err(err(
            StatusCode::BAD_REQUEST,
            format!("项目存放位置必须是绝对路径（例如 F:/yekitrim-projects，不要用相对路径）：{s}"),
        ));
    }
    if p.exists() && !p.is_dir() {
        return Err(err(
            StatusCode::BAD_REQUEST,
            format!("项目存放位置指向的是一个文件，不是目录：{}", yekitrim_core::path::display(p)),
        ));
    }
    if !yekitrim_service::disk::writable(p) {
        return Err(err(
            StatusCode::BAD_REQUEST,
            format!(
                "项目存放位置不可用（建不出目录或写不进去）：{} —— 换个存在的盘符或有写权限的目录",
                yekitrim_core::path::display(p)
            ),
        ));
    }
    Ok(yekitrim_core::path::display(p))
}

/// `PUT /api/admin/ingest`：保存采集/录播姬配置（仅 admin）。
///
/// 校验与语义见 [`build_ingest_patch`]；写入走 `patch_config_file`（toml_edit 合并）→
/// 只动这几个键，其余 section、键顺序、**注释**全部原样保留
/// （`[ingest.recorder]` 是子表，靠的就是它现在支持递归合并）。
pub async fn put_config(
    State(state): State<Arc<AppState>>,
    axum::extract::Extension(user): axum::extract::Extension<CurrentUser>,
    ClientIp(ip): ClientIp,
    Json(body): Json<serde_json::Value>,
) -> ApiResult {
    let (ingest, storage, warnings) = build_ingest_patch(&body)?;

    if ingest.is_empty() && storage.is_empty() {
        return ok(serde_json::json!({ "ok": true, "warnings": warnings, "changed": [] }));
    }

    // 审计只记**键名**：把 api_pass 的值写进 audit 表等于把口令抄一份到数据库里
    let changed = {
        let mut keys = audit_keys("ingest", &ingest);
        keys.extend(audit_keys("storage", &storage));
        keys.join(", ")
    };
    write_global_config(ingest, storage).await?;
    let _ = state
        .auth
        .db()
        .audit_ip(Some(user.id), "ingest_config_save", &changed, ip.as_deref());

    let mut out = config_view_json();
    out["warnings"] = serde_json::json!(warnings);
    out["changed"] = serde_json::json!(changed);
    ok(out)
}

/// 组装要合并进全局配置的补丁：`{ section: { key: value } }`。
///
/// **空的 section 不写进去** —— 否则一次"只改下载参数"的保存会凭空造出一个空的 `[storage]`。
fn config_patch(
    ingest: serde_json::Map<String, serde_json::Value>,
    storage: serde_json::Map<String, serde_json::Value>,
) -> serde_json::Value {
    let mut patch = serde_json::Map::new();
    if !ingest.is_empty() {
        patch.insert("ingest".into(), serde_json::Value::Object(ingest));
    }
    if !storage.is_empty() {
        patch.insert("storage".into(), serde_json::Value::Object(storage));
    }
    serde_json::Value::Object(patch)
}

/// 把 `[ingest]` / `[storage]` 的字段合并进**全局配置**（默认 `<程序目录>/.yekitrim/config.toml`）。
async fn write_global_config(
    ingest: serde_json::Map<String, serde_json::Value>,
    storage: serde_json::Map<String, serde_json::Value>,
) -> Result<(), AppError> {
    write_global_config_at(
        &yekitrim_core::path::yekitrim_home().join("config.toml"),
        config_patch(ingest, storage),
    )
    .await
}

/// [`write_global_config`] 的带路径版本（单测直接打临时文件，不碰真实全局配置）。
///
/// 与 `api.rs::config_global_put` 同一套：读改写会与设置页保存、CLI 并发，
/// 锁在 await 里持有，真正写盘那段搬到 blocking 线程。
async fn write_global_config_at(
    path: &std::path::Path,
    patch: serde_json::Value,
) -> Result<(), AppError> {
    let _guard = yekitrim_core::filelock::lock_path(path).await;
    let p = path.to_path_buf();
    let res = tokio::task::spawn_blocking(move || yekitrim_core::config::patch_config_file(&p, &patch))
        .await
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("保存线程失败: {e}")))?;
    res.map_err(|e| err(StatusCode::BAD_REQUEST, format!("保存配置失败: {e:#}")))
}

/// 这次改了哪些键（用于审计与响应里的 `changed`）。只列键名（带 section 前缀），不带值。
fn audit_keys(section: &str, map: &serde_json::Map<String, serde_json::Value>) -> Vec<String> {
    let mut keys: Vec<String> = Vec::new();
    for (k, v) in map {
        if let Some(sub) = v.as_object() {
            for sk in sub.keys() {
                keys.push(format!("{section}.{k}.{sk}"));
            }
        } else {
            keys.push(format!("{section}.{k}"));
        }
    }
    keys
}

/// `POST /api/admin/ingest/token {action:"generate"|"disable"}`：生成/轮换/关闭 webhook token。
///
/// 明文**只在这一个响应里回一次**（与 CLI `yekitrim ingest token` 同一语义），
/// 配置里只留 scrypt 哈希；`token_hash` 永远不回给前端。
pub async fn rotate_token(
    State(state): State<Arc<AppState>>,
    axum::extract::Extension(user): axum::extract::Extension<CurrentUser>,
    ClientIp(ip): ClientIp,
    Json(body): Json<serde_json::Value>,
) -> ApiResult {
    let action = body.get("action").and_then(|v| v.as_str()).unwrap_or("generate");
    let mut ingest = serde_json::Map::new();
    let mut plaintext: Option<String> = None;
    match action {
        // 关闭：只关开关，**保留** token（下次开回来还是同一个，除非重新生成）
        "disable" => {
            ingest.insert("webhook_enabled".into(), serde_json::json!(false));
        }
        "generate" => {
            let token = yekitrim_service::ingest::generate_token();
            // scrypt（N=2^15）是 CPU 密集：别占住 tokio worker
            let t = token.clone();
            let hash = tokio::task::spawn_blocking(move || yekitrim_service::auth::hash_password(&t))
                .await
                .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("生成线程失败: {e}")))?
                .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("生成 token 失败: {e:#}")))?;
            ingest.insert("token_hash".into(), serde_json::json!(hash));
            ingest.insert("webhook_enabled".into(), serde_json::json!(true));
            plaintext = Some(token);
        }
        other => {
            return Err(err(
                StatusCode::BAD_REQUEST,
                format!("未知 action: {other}（可用 generate / disable）"),
            ))
        }
    }

    write_global_config(ingest, serde_json::Map::new()).await?;
    // 审计**不记 token 明文**
    let detail = if plaintext.is_some() { "generated" } else { "disabled" };
    let _ = state
        .auth
        .db()
        .audit_ip(Some(user.id), "ingest_token", detail, ip.as_deref());

    let mut out = config_view_json();
    if let Some(t) = plaintext {
        // 只有这一条响应里有明文；前端要提示用户"只显示这一次"
        out["token"] = serde_json::json!(t);
    }
    ok(out)
}

// ---------------------------------------------------------------------------
// B站链接通道（P4）
// ---------------------------------------------------------------------------
//
// 这一节与上面的录播姬 webhook 是**两个来源**，但共用同一张 `ingest_sessions` 表
// （靠 `kind` 列区分）。放在同一个模块是因为它们是同一件事的两半：把外部素材接进来。
//
// 分工：
// - 本模块 = **HTTP 入口**（校验、预检、建会话、唤醒 worker）；
// - `crate::download_worker` = **执行**（spawn 下载器、落弹幕、建项目）。
//   两者之间只有 `ingest_sessions` 一行数据，不共享任何函数 —— 这是刻意的（计划 §2）。

/// B站接口错误 → HTTP 状态码。
///
/// 分开映射而不是一律 500：前端据此决定文案与"要不要让用户重试"
/// （`RateLimited` 是"稍后再来"，`Forbidden` 是"换链接"，完全不同）。
fn bili_status(e: &yekitrim_service::bilibili::BiliError) -> StatusCode {
    use yekitrim_service::bilibili::BiliError as B;
    match e {
        B::BadInput(_) => StatusCode::BAD_REQUEST,
        B::NotFound => StatusCode::NOT_FOUND,
        B::Forbidden | B::NotAvailable(_) => StatusCode::FORBIDDEN,
        B::RateLimited => StatusCode::TOO_MANY_REQUESTS,
        B::Api { .. } | B::Transport(_) | B::Malformed(_) => StatusCode::BAD_GATEWAY,
    }
}

fn bili_err(e: yekitrim_service::bilibili::BiliError) -> AppError {
    err(bili_status(&e), e.user_facing())
}

/// 清晰度档位合法性（前端下拉的取值域）。
///
/// 非法值**明确拒绝**而不是回退默认：`quality_height` 对认不出的串会返回 `None`
/// （= 不限高），静默接受会让"写错的清晰度"变成"下最高画质"，把磁盘与带宽吃光。
///
/// 返回的是**原因**而不是 `AppError`：两个调用点字段名不同
/// （提交端点叫 `quality`，设置页表单叫 `download.max_quality`），
/// 字段名前缀由调用方补 —— 共用校验、各自定位。
fn check_quality(raw: &str) -> Result<(), String> {
    let s = raw.trim().to_ascii_lowercase();
    let ok = s.is_empty()
        || s == "best"
        || matches!(s.as_str(), "1080p" | "720p" | "480p" | "360p")
        || s.trim_end_matches('p').parse::<u32>().is_ok();
    if ok {
        Ok(())
    } else {
        Err(format!(
            "未知清晰度 {raw:?}（可用 1080p / 720p / 480p / 360p / best）"
        ))
    }
}

/// 存放位置门禁：`None` = 放行；`Some(msg)` = 拒绝理由（**按角色**给文案）。
///
/// B站下载是唯一"替用户往磁盘写大量数据"的入口，落点必须有人显式选过 ——
/// 没配置就静默落到程序目录旁的默认位置，用户根本不知道东西下到哪了。
/// 文案分角色：管理员给去处（设置页）；普通用户只说联系管理员
/// （不暴露管理端入口，与前文 `dl.ready === false` 那张卡的措辞同一套）。
fn projects_root_gate(role: &str, projects_root: &str) -> Option<&'static str> {
    if !projects_root.trim().is_empty() {
        return None;
    }
    Some(if role == "admin" {
        "尚未配置项目存放位置：请在 设置 → 下载 里配置后重试"
    } else {
        "管理员尚未配置下载存放位置，暂时无法下载；请联系管理员"
    })
}

/// `POST /api/ingest/resolve {url}`：解析 B站链接，返回预览卡数据。
///
/// **薄壳、不落库**：用户每改一次输入框都会点一下解析，落库会攒出一堆没人要的会话。
///
/// 走的是 view API 而不是 `yt-dlp -J`（实测 T3：后者拿不到 cid 且慢得多，见
/// `yekitrim_service::bilibili` 的模块说明）。
///
/// 要求登录：这是个"替用户去请求 B站"的代理口，匿名开放等于给我们自己招风控。
pub async fn bilibili_resolve(
    axum::extract::Extension(_user): axum::extract::Extension<CurrentUser>,
    Json(body): Json<serde_json::Value>,
) -> ApiResult {
    let url = body.get("url").and_then(|v| v.as_str()).unwrap_or("").trim();
    if url.is_empty() {
        return Err(err(StatusCode::BAD_REQUEST, "请输入 B站视频链接"));
    }
    let bvid = yekitrim_service::bilibili::extract_bvid(url).map_err(bili_err)?;
    let info = yekitrim_service::bilibili::fetch_view(&bvid).await.map_err(bili_err)?;

    // 预览卡要显示"总占用" —— 前端不做码率估算，服务端给一个统一口径
    let cfg = yekitrim_core::config::load_global_config();
    let estimate_per_page: Vec<Option<u64>> = info
        .pages
        .iter()
        .map(|p| crate::download_worker::estimate_bytes(p.duration_secs, &cfg.ingest.max_quality))
        .collect();
    let total_estimate: u64 = estimate_per_page.iter().flatten().sum();

    ok(serde_json::json!({
        "ok": true,
        "bvid": info.bvid,
        "title": info.title,
        "uploader": info.uploader,
        "cover": info.cover,
        "duration_secs": info.duration_secs,
        "danmaku_total": info.danmaku_total,
        "view_total": info.view_total,
        // 多P 展开（单P 也会有一个元素，前端统一按列表渲染即可）
        "pages": info.pages.iter().zip(&estimate_per_page).map(|(p, est)| serde_json::json!({
            "index": p.index,
            "cid": p.cid,
            "part": p.part,
            "duration_secs": p.duration_secs,
            "estimate_bytes": est,
        })).collect::<Vec<_>>(),
        "estimate_bytes": total_estimate,
        "quality": cfg.ingest.max_quality,
        // 工具就绪位：没装时不拦解析（用户可以先看预览），提交时再引导
        "downloader_ready": yekitrim_service::downloader::registry::find_ytdlp(&cfg).is_some(),
    }))
}

/// `POST /api/ingest/bilibili {url, pages:[1,3], quality?, auto_pipeline?}`：提交下载。
///
/// 一次提交可以选多个 P —— **每个 P 一个会话**，一 P 一个项目。
/// 返回 `accepted`（本次真正入队的）与 `skipped`（已完成的），前端逐个跟进度。
pub async fn bilibili_submit(
    State(state): State<Arc<AppState>>,
    axum::extract::Extension(user): axum::extract::Extension<CurrentUser>,
    Json(body): Json<serde_json::Value>,
) -> ApiResult {
    let url = body.get("url").and_then(|v| v.as_str()).unwrap_or("").trim();
    if url.is_empty() {
        return Err(err(StatusCode::BAD_REQUEST, "请输入 B站视频链接"));
    }
    let bvid = yekitrim_service::bilibili::extract_bvid(url).map_err(bili_err)?;

    // 工具没就绪就**不收单**：收了也只能排在队列里干等，用户会以为卡住了。
    // 明确告诉他去装（前端有引导卡）。
    let cfg = yekitrim_core::config::load_global_config();
    if yekitrim_service::downloader::registry::find_ytdlp(&cfg).is_none() {
        return Err(err(
            StatusCode::SERVICE_UNAVAILABLE,
            "下载器（yt-dlp）未就绪；请在设置页一键下载后再提交",
        ));
    }
    if let Some(msg) = projects_root_gate(&user.role, &cfg.storage.projects_root) {
        return Err(err(StatusCode::PRECONDITION_REQUIRED, msg.to_string()));
    }
    if let Some(q) = body.get("quality").and_then(|v| v.as_str()) {
        check_quality(q)
            .map_err(|r| err(StatusCode::BAD_REQUEST, format!("quality：{r}")))?;
    }
    let auto_pipeline = body
        .get("auto_pipeline")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let info = yekitrim_service::bilibili::fetch_view(&bvid).await.map_err(bili_err)?;
    if info.pages.is_empty() {
        return Err(err(StatusCode::BAD_REQUEST, "这个视频没有可下载的分P"));
    }

    // 选中的 P：不传 = 第 1 P（**不默认全选** —— 23P 全下会直接撑爆配额）
    let wanted: Vec<u32> = match body.get("pages").and_then(|v| v.as_array()) {
        Some(arr) => arr
            .iter()
            .filter_map(|v| v.as_u64())
            .map(|n| n as u32)
            .collect(),
        None => vec![1],
    };
    if wanted.is_empty() {
        return Err(err(StatusCode::BAD_REQUEST, "请至少选择一个分P"));
    }
    let mut picked: Vec<&yekitrim_service::bilibili::BvPage> = Vec::new();
    for n in &wanted {
        let p = info
            .pages
            .iter()
            .find(|p| p.index == *n)
            .ok_or_else(|| {
                err(
                    StatusCode::BAD_REQUEST,
                    format!("这个视频没有第 {n} P（共 {} P）", info.pages.len()),
                )
            })?;
        if !picked.iter().any(|q| q.index == p.index) {
            picked.push(p);
        }
    }

    // 提交时先做一次**粗估**预检：不足就当场拒绝（别让用户等 worker 几分钟才失败）。
    // worker 里还会再查一次（那时下载目录已建、估算更准）。
    let incoming: u64 = picked
        .iter()
        .filter_map(|p| crate::download_worker::estimate_bytes(p.duration_secs, &cfg.ingest.max_quality))
        .sum();
    let probe_dir = yekitrim_service::projects::tenant_root(user.id);
    yekitrim_service::ingest::capacity::check_capacity(
        state.auth.db(),
        user.id,
        incoming,
        &probe_dir,
        &cfg,
    )
    .map_err(|e| err(StatusCode::INSUFFICIENT_STORAGE, format!("{e:#}")))?;

    let db = state.auth.db();
    let total = info.pages.len();
    let mut accepted: Vec<serde_json::Value> = Vec::new();
    let mut skipped: Vec<serde_json::Value> = Vec::new();

    for p in picked {
        let sid = format!("bili-{bvid}-p{}", p.index);
        let sref = yekitrim_service::bilibili::session_ref(&bvid, Some(p.index));

        // 已建过项目的：不重下（用户想重来应当去项目页删掉再提交）
        if let Some(existing) = db.ingest_session(&sid).map_err(internal)? {
            if existing.project_id.is_some() {
                skipped.push(serde_json::json!({
                    "session_id": sid,
                    "page": p.index,
                    "reason": "已存在项目",
                    "project_id": existing.project_id,
                }));
                continue;
            }
        }

        let created = db
            .create_ingest_session(
                &sid,
                "bilibili",
                user.id,
                Some(&sref),
                "queued",
                auto_pipeline.as_deref(),
            )
            .map_err(internal)?;
        // 重投同一份（上次失败/被取消）：显式打回队列 —— `create_ingest_session`
        // 幂等时不改 state，不给这一步的话"再点一次提交"会毫无反应。
        db.set_ingest_state(&sid, "queued", None).map_err(internal)?;

        let title = {
            let part = p.part.trim();
            if total <= 1 {
                info.title.clone()
            } else if part.is_empty() {
                format!("{} P{}", info.title, p.index)
            } else {
                format!("{} P{} {part}", info.title, p.index)
            }
        };
        accepted.push(serde_json::json!({
            "session_id": sid,
            "page": p.index,
            "title": title,
            "cid": p.cid,
            "duration_secs": p.duration_secs,
            "created": created,
        }));
    }

    let _ = db.audit(
        Some(user.id),
        "ingest_bilibili_submit",
        &format!("bvid={bvid} pages={} accepted={}", wanted.len(), accepted.len()),
    );
    // 唤醒 worker（周期 tick 也会兜底）
    crate::download_worker::notify(state, None);
    ok(serde_json::json!({
        "ok": true,
        "bvid": bvid,
        "accepted": accepted,
        "skipped": skipped,
        "estimate_bytes": incoming,
    }))
}

/// `POST /api/admin/downloader/install`：下载内置 yt-dlp 并校验 sha256（仅 admin）。
///
/// 同步等它下完（17.8MB，实测 1.8s，但网络差时会久）→ 下到 blocking 池外面，
/// 用 async；前端显示"正在下载"。
pub async fn downloader_install(
    State(_state): State<Arc<AppState>>,
    axum::extract::Extension(user): axum::extract::Extension<CurrentUser>,
) -> ApiResult {
    if !user.is_admin() {
        return Err(err(StatusCode::FORBIDDEN, "仅管理员可安装下载器"));
    }
    let path = yekitrim_service::downloader::registry::ensure_ytdlp(&yekitrim_core::config::load_global_config())
        .await
        .map_err(|e| err(StatusCode::BAD_GATEWAY, format!("安装下载器失败: {e:#}")))?;
    let cfg = yekitrim_core::config::load_global_config();
    // ⚠️ 只回版本与是否通过哈希校验，**不回路径**（与 §3.5.9 一致）
    let (verified, source) = match yekitrim_service::downloader::registry::find_ytdlp(&cfg) {
        Some(i) => {
            let s = i.source.label();
            (i.verified, s)
        }
        None => (false, "未知"),
    };
    ok(serde_json::json!({
        "ok": true,
        "path_present": path.is_file(),
        "version": yekitrim_service::downloader::registry::VERSION,
        "verified": verified,
        "source": source,
    }))
}

/// `GET /api/ingest/downloader`：下载器就绪位（任何登录用户可看，供前端引导卡）。
pub async fn downloader_status(
    State(_state): State<Arc<AppState>>,
) -> ApiResult {
    let cfg = yekitrim_core::config::load_global_config();
    let found = yekitrim_service::downloader::registry::find_ytdlp(&cfg);
    let kind = yekitrim_service::downloader::DownloaderKind::parse(&cfg.ingest.downloader);
    // 同样**不含路径**：这个响应给非管理员看
    ok(serde_json::json!({
        "ok": true,
        "kind": kind.label(),
        "ready": found.is_some(),
        "verified": found.map(|i| i.verified).unwrap_or(false),
        "pinned_version": yekitrim_service::downloader::registry::VERSION,
        "license": yekitrim_service::downloader::registry::LICENSE,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use yekitrim_service::db::Db;

    fn db() -> (tempfile::TempDir, Db) {
        let dir = tempfile::tempdir().unwrap();
        let db = Db::open(&dir.path().join("yekitrim.db")).unwrap();
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

    /// 存放位置门禁：未配置时**按角色**给不同文案；已配置（含看似空白）放行。
    #[test]
    fn projects_root_gate_by_role() {
        // 未配置：管理员给去处，普通用户不暴露管理端
        let a = projects_root_gate("admin", "").unwrap();
        assert!(a.contains("设置"), "{a}");
        let u = projects_root_gate("user", "").unwrap();
        assert!(u.contains("联系管理员") && !u.contains("设置"), "{u}");
        // 空白串（用户存了 " "）也算未配置
        assert!(projects_root_gate("admin", "   ").is_some());
        // 已配置：放行
        assert!(projects_root_gate("admin", "F:/media/yekitrim").is_none());
        assert!(projects_root_gate("user", "D:\\x").is_none());
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

    /// webhook token 的形状检查：只有"`<TOKEN_PREFIX>` + 40 位 base64url"才值得跑 scrypt。
    ///
    /// 这是未认证入口上的成本闸门 —— 形状检查失败时直接 401，不碰 KDF。
    ///
    /// ⚠️ **本测试故意不写字面量前缀**，一律从 `TOKEN_PREFIX` 拼：
    /// 以前它把 `yekitrim_` 写死，于是改名时"生成端改了、校验端没改"这件事
    /// **测不出来**（断言保护的就是旧名字）。现在前缀只有一个来源。
    #[test]
    fn token_shape_gate_rejects_junk_before_kdf() {
        use yekitrim_service::ingest::TOKEN_PREFIX as P;
        // 真实生成器的产物必须过
        assert!(token_shape_ok(&yekitrim_service::ingest::generate_token()));
        // ⚠️ **回归钉子**：生成端与校验端必须同前缀（改名时只改一端 ⇒ 每个 token 都 401）
        let generated = yekitrim_service::ingest::generate_token();
        assert!(
            generated.starts_with(P),
            "生成器产物 {generated} 必须带前缀 {P}"
        );
        // 前缀 / 长度 / 字符集 任一不对就否掉
        assert!(!token_shape_ok(""));
        assert!(!token_shape_ok(P));
        assert!(!token_shape_ok(&format!("{P}short")));
        assert!(!token_shape_ok(&format!("{P}{}", "a".repeat(39))));
        assert!(!token_shape_ok(&format!("{P}{}", "a".repeat(41))));
        assert!(!token_shape_ok(&format!("{P}{}", "a".repeat(39) + "!")));
        assert!(!token_shape_ok(&"a".repeat(40)));
        assert!(!token_shape_ok(&format!("{}", P.to_uppercase() + &"a".repeat(40))));
        // ⚠️ 旧前缀（`vtrim_`）**不再被认** —— 不兼容口径：存量 token 一律失效。
        //    这里**故意写旧名字面量**：写成 `P` 就成了同义反复，什么也证明不了。
        assert!(
            !token_shape_ok(&format!("vtrim_{}", "a".repeat(40))),
            "旧前缀必须被拒（不兼容口径）"
        );
        // `-` / `_` 是 base64url 的合法字符，不能误伤
        assert!(token_shape_ok(&format!("{P}{}", "-_A0".repeat(10))));
    }

    // ---- 采集配置表单（PUT /api/admin/ingest）----

    /// 校验：坏 IP / 坏 scheme / 坏 auto_pipeline / 类型不对，都要给出**可读的 400**。
    ///
    /// 特别是 `192.168.1.0/24` 这种写法：运行时白名单是精确字符串相等，
    /// 放行它只会让来源被静默拦掉、极难排查，所以这里必须挡住。
    #[test]
    fn ingest_patch_rejects_invalid_input() {
        let cases = [
            (serde_json::json!({"allow_ips": ["192.168.1.0/24"]}), "allow_ips"),
            (serde_json::json!({"allow_ips": ["recorder.local"]}), "allow_ips"),
            (serde_json::json!({"allow_ips": "127.0.0.1"}), "allow_ips"),
            (serde_json::json!({"recorder": {"api_base": "127.0.0.1:2356"}}), "api_base"),
            (serde_json::json!({"recorder": {"auto_pipeline": "everything"}}), "auto_pipeline"),
            (serde_json::json!({"recorder": {"root": 123}}), "root"),
            (serde_json::json!({"webhook_enabled": "yes"}), "webhook_enabled"),
            (serde_json::json!({"recorder": {"watch_interval_mins": -1}}), "watch_interval_mins"),
        ];
        for (body, needle) in cases {
            let e = build_ingest_patch(&body).unwrap_err();
            assert_eq!(e.0, StatusCode::BAD_REQUEST, "{body}");
            let msg = serde_json::to_string(&e.1 .0).unwrap();
            assert!(msg.contains(needle), "错误文案该点出是哪个键（{needle}）：{msg}");
        }
        // 合法的照常通过：单个 IP（v4/v6）、带 scheme 的地址、四个终点
        for ok_body in [
            serde_json::json!({"allow_ips": ["127.0.0.1", "::1"]}),
            serde_json::json!({"recorder": {"api_base": "http://127.0.0.1:2356"}}),
            serde_json::json!({"recorder": {"api_base": ""}}),
            serde_json::json!({"recorder": {"auto_pipeline": "cut"}}),
            serde_json::json!({"recorder": {"watch_interval_mins": 0}}),
        ] {
            assert!(build_ingest_patch(&ok_body).is_ok(), "{ok_body}");
        }
    }

    /// 项目存放位置（`[storage] projects_root`）—— 下载与入库的**共同**落点。
    ///
    /// 三条边界各对应一个真实事故：相对路径的基准是进程 cwd，服务换个启动目录落点就漂；
    /// 指向文件会让 `create_dir_all` 等到**下载阶段**才炸（用户白等几分钟）；
    /// 盘符写错是最常见的手滑，现场建一下目录就能立刻发现。
    #[test]
    fn projects_root_is_validated_and_created() {
        // ① 键缺席 = 不动它（不许凭空造出一个空 [storage] 段）
        let (_, st, _) = build_ingest_patch(&serde_json::json!({"webhook_enabled": true})).unwrap();
        assert!(st.is_empty(), "没提 storage 就不该动它：{st:?}");

        // ② 空串 = 显式回默认（程序目录下的 .yekitrim/projects）
        let (_, st, _) =
            build_ingest_patch(&serde_json::json!({"storage": {"projects_root": ""}})).unwrap();
        assert_eq!(st["projects_root"], serde_json::json!(""));

        // ③ 相对路径：拒
        for bad in ["yekitrim-projects", "./out", "../out"] {
            let e = build_ingest_patch(&serde_json::json!({"storage": {"projects_root": bad}}))
                .unwrap_err();
            assert_eq!(e.0, StatusCode::BAD_REQUEST, "{bad}");
            let msg = serde_json::to_string(&e.1 .0).unwrap();
            assert!(msg.contains("绝对路径"), "{bad} → {msg}");
        }

        // ④ 指向一个已存在的文件：拒
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("not-a-dir.txt");
        std::fs::write(&file, b"x").unwrap();
        let e = build_ingest_patch(
            &serde_json::json!({"storage": {"projects_root": file.to_string_lossy()}}),
        )
        .unwrap_err();
        let msg = serde_json::to_string(&e.1 .0).unwrap();
        assert!(msg.contains("不是目录"), "{msg}");

        // ⑤ 全新目录：放行，且**保存时就真的建出来**（等下载跑完才发现建不了就太晚了）
        let fresh = dir.path().join("projects").join("yekitrim");
        let (_, st, _) = build_ingest_patch(
            &serde_json::json!({"storage": {"projects_root": fresh.to_string_lossy()}}),
        )
        .unwrap();
        assert_eq!(
            st["projects_root"].as_str().unwrap(),
            yekitrim_core::path::display(&fresh)
        );
        assert!(fresh.is_dir(), "保存时就该把目录建出来：{}", fresh.display());
    }

    /// 落盘：全局配置里**原本没有 `[storage]` 段**时要能把它建出来，且不碰其它 section。
    ///
    /// 这条路径是本次新开的（`patch_config_file` 只在 section 不是表时插入新表，
    /// 之前所有调用方写的都是已存在的 `[ingest]`），所以要有钉子。
    #[tokio::test]
    async fn save_projects_root_creates_missing_storage_section() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(
            &path,
            "\
# 顶部注释不能丢
[ingest]
webhook_enabled = true

[detection]
vtuber_name = \"主播\"
",
        )
        .unwrap();

        let target = dir.path().join("my-projects");
        let (_, storage, _) = build_ingest_patch(
            &serde_json::json!({"storage": {"projects_root": target.to_string_lossy()}}),
        )
        .unwrap();
        write_global_config_at(&path, config_patch(serde_json::Map::new(), storage))
            .await
            .unwrap();

        let out = std::fs::read_to_string(&path).unwrap();
        assert!(out.contains("[storage]"), "该段必须被建出来：\n{out}");
        // TOML 会把路径里的反斜杠转义成 `\\`，逐字符比对太脆 —— 归一化后核对即可
        let expected = yekitrim_core::path::display(&target);
        assert!(
            out.contains("projects_root =") && out.replace("\\\\", "\\").contains(&expected),
            "写入的路径不对（期望 {expected}）：\n{out}"
        );
        // 原有内容一字不动
        for keep in ["# 顶部注释不能丢", "webhook_enabled = true", "vtuber_name = \"主播\""] {
            assert!(out.contains(keep), "丢了 {keep}：\n{out}");
        }

        // 再存一次"空 = 回默认"：只改值，不把段清空、也不动别的 section
        let (_, storage, _) =
            build_ingest_patch(&serde_json::json!({"storage": {"projects_root": ""}})).unwrap();
        write_global_config_at(&path, config_patch(serde_json::Map::new(), storage))
            .await
            .unwrap();
        let out = std::fs::read_to_string(&path).unwrap();
        assert!(out.contains("projects_root = \"\""), "回默认应写成空串：\n{out}");
        assert!(out.contains("webhook_enabled = true"), "{out}");
    }

    /// ⚠️ 回归钉子：**B站下载那几项必须平铺在 `[ingest]` 下，不能嵌套成子表**。
    ///
    /// `[ingest] downloader` 是一个**字符串键**（引擎名）。如果这里手滑写成
    /// `ingest.insert("downloader", json!({...}))`，文件写出来仍是合法 TOML、写入也不报错，
    /// 但下次读配置时 `downloader` 反序列化成 `String` 会失败 → **整份配置被静默忽略**
    /// （表现为"改完设置重启后所有设置都回到默认"，极难归因）。
    #[test]
    fn patch_download_keys_are_flat_not_nested() {
        let (m, _, _) = build_ingest_patch(&serde_json::json!({
            "download": { "engine": "D:/tools/yt-dlp.exe", "max_quality": "720p", "max_concurrent_downloads": 2 }
        }))
        .unwrap();

        assert!(
            m.get("downloader").map(|v| v.is_string()).unwrap_or(false),
            "downloader 必须是字符串（平铺键），不是对象：{m:?}"
        );
        assert_eq!(m.get("downloader").unwrap(), &serde_json::json!("D:/tools/yt-dlp.exe"));
        assert_eq!(m.get("max_quality").unwrap(), &serde_json::json!("720p"));
        assert_eq!(m.get("max_concurrent_downloads").unwrap(), &serde_json::json!(2));
        // 分组名本身不能出现在 patch 里（它只是 HTTP 层的容器）
        assert!(m.get("download").is_none(), "分组名不该落进配置：{m:?}");
        // 并发调大要给出风控提醒
        let (_, _, w) = build_ingest_patch(&serde_json::json!({
            "download": { "max_concurrent_downloads": 5 }
        }))
        .unwrap();
        assert!(w.iter().any(|s| s.contains("412")), "并发调大应提醒风控：{w:?}");

        // 非法值同样要挡（清晰度写错会静默变成"不限高"，把磁盘吃光）
        for (body, needle) in [
            (serde_json::json!({"download": {"max_quality": "超清"}}), "max_quality"),
            (serde_json::json!({"download": {"engine": 123}}), "engine"),
            (serde_json::json!({"download": "yt-dlp"}), "download"),
            (serde_json::json!({"download": {"rate_limit_kbps": -1}}), "rate_limit_kbps"),
        ] {
            let e = build_ingest_patch(&body).unwrap_err();
            assert_eq!(e.0, StatusCode::BAD_REQUEST, "{body}");
            let msg = serde_json::to_string(&e.1 .0).unwrap();
            assert!(msg.contains(needle), "{needle}: {msg}");
        }
    }

    /// 表单语义：**键缺席 = 不改**、给空串 = 清空、空数组 = 清空白名单。
    ///
    /// 这条是"留空不改"能不能成立的基础：如果缺席也被写成空串，
    /// 用户没碰密码框就会把已保存的密码清掉。
    #[test]
    fn ingest_patch_distinguishes_absent_from_empty() {
        // 缺席：patch 里连键都没有（于是文件里那行原样不动）
        let (m, _, w) = build_ingest_patch(&serde_json::json!({"webhook_enabled": true})).unwrap();
        assert_eq!(m.len(), 1);
        assert!(m.get("recorder").is_none(), "没提到 recorder 就不该出现：{m:?}");
        assert!(w.is_empty());

        // 显式空串 / 空数组 = 清空
        let (m, _, _) = build_ingest_patch(&serde_json::json!({
            "allow_ips": [],
            "recorder": {"api_base": "", "api_pass": ""}
        }))
        .unwrap();
        assert_eq!(m["allow_ips"], serde_json::json!([]));
        assert_eq!(m["recorder"]["api_base"], serde_json::json!(""));
        assert_eq!(m["recorder"]["api_pass"], serde_json::json!(""));

        // 口令不 trim（首尾空格是合法字符），其余路径/地址 trim；空项被丢掉、重复项去重
        let (m, _, _) = build_ingest_patch(&serde_json::json!({
            "allow_ips": [" 127.0.0.1 ", "", "127.0.0.1"],
            "recorder": {"root": "  D:/rec  ", "api_pass": " p w "}
        }))
        .unwrap();
        assert_eq!(m["allow_ips"], serde_json::json!(["127.0.0.1"]), "去重 + 去空");
        assert_eq!(m["recorder"]["root"], serde_json::json!("D:/rec"));
        assert_eq!(m["recorder"]["api_pass"], serde_json::json!(" p w "), "口令原样");

        // 目录不可达只给 warning，不拒绝（网络盘可能只是暂时没挂上）
        let (_, _, w) = build_ingest_patch(&serde_json::json!({
            "recorder": {"root": "Z:/definitely-not-here-yekitrim"}
        }))
        .unwrap();
        assert_eq!(w.len(), 1, "该有一条 warning：{w:?}");
        assert!(w[0].contains("Z:/definitely-not-here-yekitrim"), "{w:?}");
    }

    /// 写盘：只动 `[ingest]` / `[ingest.recorder]` 命中的键，**同表兄弟键与注释一个不能少**。
    ///
    /// 回归钉子：`patch_config_file` 曾经只认一层，`recorder` 会被整项替换成 inline table，
    /// `root`/`api_user`/`api_pass`/`watch_interval_mins` 与段内注释静默消失
    /// （文件仍是合法 TOML，不报错，下次读配置才发现）。
    #[tokio::test]
    async fn save_ingest_config_keeps_siblings_and_comments() {
        let dir = std::env::temp_dir().join(format!("yekitrim-ingest-cfg-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.toml");
        std::fs::write(
            &path,
            "\
# 顶部注释不能丢
[ingest]
# webhook 开关
webhook_enabled = true
token_hash = \"scrypt$keepme\"

[ingest.recorder]
# 录像目录
root = \"D:/rec\"
api_base = \"http://127.0.0.1:2356\"
api_user = \"admin\"
api_pass = \"oldpass\"
watch_interval_mins = 7

[detection]
vtuber_name = \"主播\"
",
        )
        .unwrap();

        let (ingest, _, _) = build_ingest_patch(&serde_json::json!({
            "allow_ips": ["127.0.0.1"],
            "recorder": {"api_base": "http://10.0.0.5:2356", "auto_pipeline": "cut"}
        }))
        .unwrap();
        write_global_config_at(&path, config_patch(ingest, serde_json::Map::new())).await.unwrap();

        let out = std::fs::read_to_string(&path).unwrap();
        assert!(out.contains("api_base = \"http://10.0.0.5:2356\""), "{out}");
        assert!(out.contains("allow_ips = [\"127.0.0.1\"]"), "{out}");
        assert!(out.contains("auto_pipeline = \"cut\""), "{out}");
        // 没提到的兄弟键原样（尤其密码：表单留空不该把它清掉）
        for keep in [
            "root = \"D:/rec\"",
            "api_user = \"admin\"",
            "api_pass = \"oldpass\"",
            "watch_interval_mins = 7",
            "token_hash = \"scrypt$keepme\"",
        ] {
            assert!(out.contains(keep), "丢了 {keep}：\n{out}");
        }
        // 注释与其它 section 原样
        for keep in ["# 顶部注释不能丢", "# webhook 开关", "# 录像目录", "vtuber_name = \"主播\""] {
            assert!(out.contains(keep), "丢了 {keep}：\n{out}");
        }
        assert!(out.contains("[ingest.recorder]"), "子表应保持表头形式：\n{out}");
        assert!(!out.contains("recorder = {"), "不该退化成 inline table：\n{out}");

        // 显式给空串才清掉密码
        let (ingest, _, _) = build_ingest_patch(&serde_json::json!({"recorder": {"api_pass": ""}})).unwrap();
        write_global_config_at(&path, config_patch(ingest, serde_json::Map::new())).await.unwrap();
        let out = std::fs::read_to_string(&path).unwrap();
        assert!(out.contains("api_pass = \"\""), "显式清空应生效：\n{out}");
        assert!(out.contains("api_user = \"admin\""), "清密码不该连用户名一起动：\n{out}");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 读接口**绝不**回 `token_hash` 或 `api_pass` 明文（表单要的是 `api_pass_set` 布尔）。
    #[test]
    fn config_view_never_leaks_secrets() {
        let text = serde_json::to_string(&config_view_json()).unwrap();
        assert!(!text.contains("token_hash"), "不该回 token_hash：{text}");
        let v = config_view_json();
        assert!(v["recorder"].get("api_pass").is_none(), "不该回 api_pass：{text}");
        // 该有的都在
        for k in ["webhook_enabled", "token_configured", "allow_ips", "webhook_path"] {
            assert!(v.get(k).is_some(), "缺字段 {k}");
        }
        for k in ["root", "api_base", "api_user", "api_pass_set", "auto_pipeline", "watch_interval_mins"] {
            assert!(v["recorder"].get(k).is_some(), "缺字段 recorder.{k}");
        }
        assert!(v["recorder"]["api_pass_set"].is_boolean());
        // B站下载分组（P4）：表单要的字段一个都不能少，否则设置页会显示成空
        for k in [
            "engine",
            "ytdlp_path",
            "max_quality",
            "rate_limit_kbps",
            "max_concurrent_downloads",
            "cookie_file",
            "idle_timeout_secs",
        ] {
            assert!(v["download"].get(k).is_some(), "缺字段 download.{k}");
        }
    }

    /// 生成的 token 能通过它自己的哈希校验（webhook 那条链路的唯一凭据）。
    #[test]
    fn generated_token_matches_its_hash() {
        let token = yekitrim_service::ingest::generate_token();
        assert!(token_shape_ok(&token), "生成器产物必须过形状检查：{token}");
        let hash = yekitrim_service::auth::hash_password(&token).unwrap();
        assert!(yekitrim_service::auth::verify_password(&hash, &token).unwrap_or(false));
        assert!(!yekitrim_service::auth::verify_password(&hash, "yekitrim_wrong").unwrap_or(false));
        assert!(hash.starts_with("scrypt$"), "配置里只该存哈希：{hash}");
    }

    /// 审计只记键名（绝不把口令值写进 audit 表）。
    #[test]
    fn audit_keys_are_names_only() {
        let (ingest, _, _) = build_ingest_patch(&serde_json::json!({
            "webhook_enabled": true,
            "recorder": {"api_pass": "supersecret", "api_base": "http://x:1"}
        }))
        .unwrap();
        let keys = audit_keys("ingest", &ingest);
        let s = keys.join(", ");
        assert!(s.contains("ingest.recorder.api_pass") && s.contains("ingest.recorder.api_base"), "{s}");
        assert!(s.contains("ingest.webhook_enabled"), "{s}");
        assert!(!s.contains("supersecret"), "审计不得带值：{s}");

        // `[storage]` 走同一个函数，但必须带自己的 section 前缀（否则审计里看不出改的是哪一段）
        let st = audit_keys("storage", &serde_json::json!({"projects_root": "F:/x"}).as_object().unwrap().clone());
        assert_eq!(st, vec!["storage.projects_root".to_string()]);
    }
}
