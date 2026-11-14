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

use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;

use yekitrim_service::db::IngestSessionRow;
use yekitrim_service::ingest::landing;

use super::auth::CurrentUser;
use super::model::AppState;
use crate::auth::{owner_of, ClientIp};

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

/// 验 token + IP 白名单 + 限流。**两条采集 webhook 共用一份**（录播姬 / 通用），
/// 别各自再抄一遍 —— 认证是未认证入口唯一的门，两份实现迟早分叉。
async fn authorize_webhook(
    state: &Arc<AppState>,
    token: &str,
    ip: &Option<String>,
) -> Result<yekitrim_core::config::Config, AppError> {
    // ⚠️ 未认证入口：连"读全局配置"都不该留在 async worker 上（读的是磁盘上的 config.toml）
    let cfg = off(|| Ok(yekitrim_core::config::load_global_config())).await?;
    if !cfg.ingest.webhook_enabled || cfg.ingest.token_hash.trim().is_empty() {
        return Err(err(
            StatusCode::FORBIDDEN,
            "采集 webhook 未启用（`yekitrim ingest token` 生成 token 并开启 [ingest] webhook_enabled）",
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
                format!("来源地址不在白名单内: {}", ip.clone().unwrap_or_else(|| "-".into())),
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
    if !token_shape_ok(token) {
        return Err(err(StatusCode::UNAUTHORIZED, "token 无效"));
    }

    // scrypt 校验是 CPU 密集：放 blocking 线程池，别占住 tokio worker
    let hash = cfg.ingest.token_hash.clone();
    let token_for_check = token.to_string();
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
    Ok(cfg)
}

/// `POST /api/ingest/bililive/<token>`：**自带 token 鉴权**，不要求会话。
pub async fn webhook(
    State(state): State<Arc<AppState>>,
    axum::extract::Path(token): axum::extract::Path<String>,
    ClientIp(ip): ClientIp,
    Json(body): Json<serde_json::Value>,
) -> ApiResult {
    let _cfg = authorize_webhook(&state, &token, &ip).await?;

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

/// 一次"录制完成"事件（两条入口共用的中间结构）。
#[derive(Debug, Clone)]
struct WebhookCompletion {
    file: String,
    danmaku: Option<String>,
    room: Option<String>,
    platform: Option<String>,
    title: Option<String>,
}

enum ParsedWebhookEvent {
    Completion(WebhookCompletion),
    Ignored(String),
}

/// 事件类型归一化：`FileClosed` / `file_closed` / `file-closed` / `FILE_CLOSED`
/// 全部变成 `file_closed`。
///
/// 三种来源的写法不一样（biliLive-tools 的 `type` 是 snake_case、录播姬的 `EventType`
/// 是 PascalCase、手写脚本什么都可能），归一化只做一次、只此一份。
fn normalize_event_type(raw: &str) -> String {
    let raw = raw.trim();
    if raw.is_empty() {
        return String::new();
    }
    let mut out = String::with_capacity(raw.len() + 4);
    let mut prev_lower = false;
    for ch in raw.chars() {
        if ch == '-' || ch == ' ' || ch == '.' {
            push_event_sep(&mut out);
            prev_lower = false;
        } else if ch.is_ascii_uppercase() {
            // `FileClosed` → `file_closed`；`file_closed` / `FILE_CLOSED` 原样保留。
            if prev_lower {
                push_event_sep(&mut out);
            }
            out.push(ch.to_ascii_lowercase());
            prev_lower = false;
        } else {
            out.push(ch);
            prev_lower = true;
        }
    }
    out
}

fn push_event_sep(out: &mut String) {
    if !out.ends_with('_') {
        out.push('_');
    }
}

/// 解析通用 webhook 的 body。
///
/// 兼容三种形状（按优先级）：
/// 1. biliLive-tools 的 `externalWebhook`：`{type:"file_completed", data:{filePath,roomId,platform}}`；
/// 2. YekiTrim 自己的规范事件：`{event:"file_completed", file, danmaku?, room_id?, platform?}`；
/// 3. 录播姬的 `FileClosed`：`{EventType:"FileClosed", EventData:{RelativePath,RoomId}}`
///    （这条走 `/bililive/<token>` 也可以；这里接住是为了让同一个 URL 兼容更多软件）。
///
/// `record_start` / `record_end` / `file_created` / `task` 等**不建项目**的事件
/// 一律返回 `Ignored`，入口回 200 —— 录制软件那边不需要为"这条不归你管"重试。
fn parse_webhook_event(body: &serde_json::Value) -> Result<ParsedWebhookEvent, String> {
    // ⚠️ 只 `to_ascii_lowercase()` 是不够的：录播姬的 `EventType` 是 **PascalCase**
    // （`FileClosed` → `fileclosed`），那样永远匹配不上 `file_closed` —— 于是上面注释里
    // "同一个 URL 兼容更多软件"只是愿望，录播姬原样打过来会回 400「不认识的事件类型」。
    let ty = normalize_event_type(
        field(
            body,
            &["type", "event_type", "event", "EventType", "eventType"],
        )
        .unwrap_or_default(),
    );
    let data = body
        .get("data")
        .or_else(|| body.get("EventData"))
        .or_else(|| body.get("event_data"))
        .unwrap_or(body);

    // biliLive-tools 的 type / 录播姬的 EventType 都会被这里归一化成 snake_case。
    if ty == "file_completed" || ty == "file_closed" {
        let file = field(
            data,
            &[
                "filePath",
                "file_path",
                "file",
                "path",
                "RelativePath",
                "relative_path",
            ],
        )
        .or_else(|| field(body, &["filePath", "file_path", "file"]))
        .ok_or_else(|| "file_completed 缺少 filePath/file".to_string())?;
        return Ok(ParsedWebhookEvent::Completion(WebhookCompletion {
            file: file.to_string(),
            danmaku: field(data, &["danmaku", "danmaku_path", "danmakuPath"]).map(str::to_string),
            room: field_text(data, &["roomId", "room_id", "RoomId", "room"]),
            platform: field(data, &["platform", "Platform"]).map(str::to_string),
            title: field(data, &["title", "Title"]).map(str::to_string),
        }));
    }

    // 没有 type 但直接给了文件：按"录制完成"处理（手写脚本最省事）。
    if ty.is_empty() {
        if let Some(file) = field(body, &["file", "filePath", "file_path"]) {
            return Ok(ParsedWebhookEvent::Completion(WebhookCompletion {
                file: file.to_string(),
                danmaku: field(body, &["danmaku", "danmaku_path"]).map(str::to_string),
                room: field_text(body, &["room_id", "roomId"]),
                platform: field(body, &["platform"]).map(str::to_string),
                title: field(body, &["title"]).map(str::to_string),
            }));
        }
    }

    if matches!(
        ty.as_str(),
        "record_start"
            | "record_end"
            | "file_created"
            | "task"
            | "session_started"
            | "session_ended"
            | "stream_started"
            | "stream_ended"
    ) {
        return Ok(ParsedWebhookEvent::Ignored(ty));
    }
    Err(format!(
        "不认识的事件类型: {ty}（支持 file_completed / file_closed，或 biliLive-tools 的 type=file_completed）"
    ))
}

/// 一条已经落地的"录像 + 弹幕"。
#[derive(Debug, Clone)]
struct ResolvedWebhook {
    project_root: PathBuf,
    input_name: String,
    danmaku_name: Option<String>,
    title: String,
    room: Option<String>,
    platform: Option<String>,
    video_display: String,
}

impl ResolvedWebhook {
    fn source_label(&self) -> String {
        let platform = self.platform.as_deref().unwrap_or("custom");
        match self.room.as_deref().filter(|s| !s.is_empty()) {
            Some(room) => format!("Webhook · {platform} · 房间 {room}"),
            None => format!("Webhook · {platform}"),
        }
    }
}

/// 通用 webhook 接受的录像容器（小写、不含点）。
///
/// 录播姬/B站下载落盘的都是这几个；`-后处理.mp4` 优先之后仍然是 mp4。
/// 其余扩展名一律 400（见 `resolve_webhook_completion`）。
const WEBHOOK_VIDEO_EXTS: &[&str] = &[
    "mp4", "flv", "mkv", "ts", "mov", "webm", "avi", "m4v", "wmv", "mpg", "mpeg",
];

/// 把事件里的路径解析成项目内的真实文件。
///
/// - `container_root` 非空且命中时先剥前缀（biliLive-tools 的 `/app/video/...`）；
/// - 其余相对/绝对路径都走 `tenant::ensure_within` —— **唯一**的越界判据；
/// - 同目录的 `-后处理.mp4` 优先（ffmpeg 转封装后的版本更通用）。
fn resolve_webhook_completion(
    cfg: &yekitrim_core::config::Config,
    c: &WebhookCompletion,
) -> Result<ResolvedWebhook, String> {
    let root = cfg.ingest.webhook.root.trim();
    if root.is_empty() {
        return Err("通用 webhook 未配置 [ingest.webhook].root".into());
    }
    let root_path = PathBuf::from(root);
    if !root_path.is_dir() {
        return Err(format!("[ingest.webhook].root 不是目录: {}", root_path.display()));
    }
    let container = cfg.ingest.webhook.container_root.trim();
    let original = map_webhook_path(&root_path, container, &c.file)?;
    let video = prefer_postprocessed(&original);
    // ⚠️ 只接受**录像容器**：事件里的路径是一个已存在的普通文件就建工程的话，
    // 把 `.xml` / `.jpg` / `.txt` 推过来会建出一个坏工程（后续转写/剪切报错，还可能白烧 ASR 额度）。
    let ext = video
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if !WEBHOOK_VIDEO_EXTS.contains(&ext.as_str()) {
        return Err(format!(
            "不像录像文件（扩展名 {:?}）：只接受 {}",
            ext,
            WEBHOOK_VIDEO_EXTS.join(" / ")
        ));
    }

    let danmaku = match c
        .danmaku
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        Some(d) => Some(map_webhook_path(&root_path, container, d)?),
        // 弹幕按**原始文件名**找：`-后处理.mp4` 的 stem 多了后缀，拿它找 `rec.xml` 会落空。
        None => find_danmaku(&original).or_else(|| find_danmaku(&video)),
    };
    if let Some(dm) = &danmaku {
        if dm.parent() != video.parent() {
            return Err(format!(
                "弹幕必须与视频同目录（或省略 danmaku 让服务端自动找同名）：{}",
                dm.display()
            ));
        }
    }

    let parent = video
        .parent()
        .ok_or_else(|| "无法确定项目目录".to_string())?;
    let input_name = file_name(&video)?;
    let danmaku_name = danmaku.as_ref().map(|d| file_name(d)).transpose()?;
    let title = c
        .title
        .clone()
        .filter(|s| !s.trim().is_empty())
        .or_else(|| {
            parent
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
        })
        .or_else(|| video.file_stem().map(|n| n.to_string_lossy().into_owned()))
        .unwrap_or_else(|| "Webhook 录像".to_string());
    Ok(ResolvedWebhook {
        project_root: parent.to_path_buf(),
        input_name,
        danmaku_name,
        title: sanitize_title(&title),
        room: c.room.clone().filter(|s| !s.trim().is_empty()),
        platform: c.platform.clone().filter(|s| !s.trim().is_empty()),
        video_display: video.display().to_string(),
    })
}

pub(crate) fn map_webhook_path(root: &Path, container_root: &str, raw: &str) -> Result<PathBuf, String> {
    let raw = raw.trim();
    if raw.is_empty() || raw.contains('\0') {
        return Err("路径为空或含非法字符".into());
    }
    let mut candidate = raw.to_string();
    if !container_root.is_empty() {
        if let Some(rest) = raw.strip_prefix(container_root) {
            if rest.is_empty() {
                return Err("容器路径只有前缀，没有文件名".into());
            }
            if !(rest.starts_with('/') || rest.starts_with('\\')) {
                return Err(format!("容器路径前缀边界不匹配: {raw}"));
            }
            candidate = rest.trim_start_matches(['/', '\\']).to_string();
        }
    }
    let abs = yekitrim_core::tenant::ensure_within(root, Path::new(&candidate))
        .map_err(|e| format!("路径不在 webhook.root 内: {e:#}"))?;
    if !abs.is_file() {
        return Err(format!("文件不存在或不是普通文件: {}", abs.display()));
    }
    Ok(abs)
}

fn prefer_postprocessed(video: &Path) -> PathBuf {
    let Some(stem) = video.file_stem().map(|s| s.to_string_lossy().into_owned()) else {
        return video.to_path_buf();
    };
    let Some(parent) = video.parent() else {
        return video.to_path_buf();
    };
    let candidate = parent.join(format!("{stem}-后处理.mp4"));
    if candidate.is_file() {
        candidate
    } else {
        video.to_path_buf()
    }
}

fn find_danmaku(video: &Path) -> Option<PathBuf> {
    let stem = video.file_stem()?.to_string_lossy().into_owned();
    let dir = video.parent()?;
    for ext in ["xml", "ass", "srt"] {
        let cand = dir.join(format!("{stem}.{ext}"));
        if cand.is_file() {
            return Some(cand);
        }
    }
    None
}

fn file_name(p: &Path) -> Result<String, String> {
    p.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .ok_or_else(|| format!("文件没有文件名: {}", p.display()))
}

fn sanitize_title(raw: &str) -> String {
    let cleaned: String = raw
        .chars()
        .filter(|c| !c.is_control())
        .collect::<String>()
        .trim()
        .to_string();
    let out: String = cleaned.chars().take(80).collect();
    if out.is_empty() {
        "Webhook 录像".to_string()
    } else {
        out
    }
}

struct PreparedWebhook {
    project_id: i64,
    config_path: PathBuf,
    duplicate: bool,
    owner: i64,
    pipeline: String,
}

/// 建项目 + 登记（同步部分；调用方已经在 blocking 池里）。
fn prepare_webhook_project(
    db: &yekitrim_service::db::Db,
    cfg: &yekitrim_core::config::Config,
    r: &ResolvedWebhook,
) -> Result<PreparedWebhook, AppError> {
    let config_path = r.project_root.join(yekitrim_core::path::PROJECT_CONFIG_FILE);
    if let Some(p) = db
        .project_by_config(&config_path)
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("查询项目失败: {e:#}")))?
    {
        return Ok(PreparedWebhook {
            project_id: p.id,
            config_path,
            duplicate: true,
            owner: p.owner_id,
            pipeline: "none".into(),
        });
    }
    if config_path.is_file() {
        return Err(err(
            StatusCode::CONFLICT,
            format!(
                "目录已有 {} 但不是已登记项目；请用「添加项目」入口，别让 webhook 覆盖它",
                yekitrim_core::path::PROJECT_CONFIG_FILE
            ),
        ));
    }
    let owner = first_admin(db)?;
    let label = r.source_label();
    let landed = landing::land_project(
        db,
        &landing::ProjectLanding {
            owner_id: owner,
            name: r.title.clone(),
            source_kind: "webhook",
            source_label: Some(label.as_str()),
            room: r.room.as_deref(),
            project_root: &r.project_root,
            input_file: &r.input_name,
            danmaku_file: r.danmaku_name.as_deref(),
            session_id: None,
        },
    )
    .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("创建项目失败: {e:#}")))?;
    Ok(PreparedWebhook {
        project_id: landed.project_id,
        config_path: landed.config_path,
        duplicate: !landed.created,
        owner,
        pipeline: cfg.ingest.webhook.auto_pipeline.clone(),
    })
}

fn first_admin(db: &yekitrim_service::db::Db) -> Result<i64, AppError> {
    let users = db
        .list_users()
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("读取账号失败: {e:#}")))?;
    users
        .into_iter()
        .filter(|u| u.role == "admin" && !u.disabled)
        .min_by_key(|u| u.id)
        .map(|u| u.id)
        .ok_or_else(|| err(StatusCode::INTERNAL_SERVER_ERROR, "账号账本里没有管理员"))
}

/// `POST /api/ingest/webhook/<token>`：**通用**录制完成入口（biliLive-tools / 任意脚本）。
///
/// 只做"验 token → 解析路径 → 建项目 → 起管线"，不搬源文件（项目就建在录像目录）。
/// **列在 `Ignored` 里的**事件（`record_start` / `file_created` / `session_ended` …）回
/// 200 + `ignored: true`，避免录制软件把"不归你管"的事件重试三天；**真认不出的** `type`
/// 仍然回 400 —— 那通常意味着 URL 填错了软件或脚本写错了字段，静默 200 只会让人以为接通了。
pub async fn custom_webhook(
    State(state): State<Arc<AppState>>,
    axum::extract::Path(token): axum::extract::Path<String>,
    ClientIp(ip): ClientIp,
    Json(body): Json<serde_json::Value>,
) -> ApiResult {
    let cfg = authorize_webhook(&state, &token, &ip).await?;
    if cfg.ingest.webhook.root.trim().is_empty() {
        return Err(err(
            StatusCode::FORBIDDEN,
            "通用 webhook 未配置 [ingest.webhook].root",
        ));
    }
    let parsed = parse_webhook_event(&body).map_err(|e| err(StatusCode::BAD_REQUEST, e))?;
    let completion = match parsed {
        ParsedWebhookEvent::Ignored(ty) => {
            return ok(serde_json::json!({ "ok": true, "ignored": true, "type": ty }))
        }
        ParsedWebhookEvent::Completion(c) => c,
    };
    // ⚠️ `resolve_webhook_completion` 里全是**同步**文件访问（`is_dir` / `is_file` /
    // 找同名弹幕），而工程目录很可能在网络盘上 —— 留在 async worker 上就是
    // 「剪切时整个服务器连不上」那类事故（见 api 卡「同步 IO 纪律」）。
    let cfg_for_resolve = cfg.clone();
    let completion_for_resolve = completion.clone();
    let resolved = match off(move || {
        Ok(resolve_webhook_completion(&cfg_for_resolve, &completion_for_resolve))
    })
    .await?
    {
        Ok(r) => r,
        Err(e) => return Err(err(StatusCode::BAD_REQUEST, e)),
    };

    let db = state.auth.db_arc();
    let cfg_for_block = cfg.clone();
    let resolved_for_block = resolved.clone();
    let prepared = off(move || prepare_webhook_project(&db, &cfg_for_block, &resolved_for_block)).await?;

    if !prepared.duplicate {
        let pipeline = crate::pipeline_boot::Pipeline::parse(&prepared.pipeline);
        if pipeline != crate::pipeline_boot::Pipeline::None {
            crate::pipeline_boot::spawn_pipeline(
                &state,
                prepared.owner,
                prepared.project_id,
                prepared.config_path.clone(),
                pipeline,
                "Webhook 入库后自动处理",
            )
            .await;
        }
    }
    tracing::info!(
        "通用 webhook: project={} duplicate={} file={} pipeline={}",
        prepared.project_id,
        prepared.duplicate,
        resolved.video_display,
        prepared.pipeline
    );
    ok(serde_json::json!({
        "ok": true,
        "project_id": prepared.project_id,
        "duplicate": prepared.duplicate,
        "input": resolved.input_name,
        "danmaku": resolved.danmaku_name,
        "pipeline": prepared.pipeline,
    }))
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
///
/// ⚠️ **别人的房间号不能被顶掉**：`room_id` 是 `channel_map` 的全局主键，无脑 upsert
/// 等于"任何账号都能把别人的直播间改到自己名下"（2026-11-14 后端 review 的 P1）。
/// 现在先由 `replace_channel_map` 在同一事务里查冲突：**命中别人的行 ⇒ 整批不生效 + 409**，
/// 与 `POST /api/rooms`（`api/rooms.rs` 里那条 409）同一口径。**未登记**的房间仍可自助登记，
/// admin 仍可接管（`allow_takeover`）。
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
    let conflicts = db
        .replace_channel_map(user.id, user.is_admin(), &rules)
        .map_err(internal)?;
    if !conflicts.is_empty() {
        // 409 而不是 403：语义是"这个房间号被别人先登记了"，不是"你没权限调这个接口"。
        // 冲突时**一条规则都没保存**（事务未提交），所以文案要写清"本次未保存"。
        return Err(err(
            StatusCode::CONFLICT,
            format!(
                "该直播间已被其他账号登记: {}（本次未保存任何改动）",
                conflicts.join(", ")
            ),
        ));
    }
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
        // 设置页「通用 Webhook」地址用：Docker/另一台机器不能用 127.0.0.1
        "lan_ip": local_lan_ip(),
        "proxy": cfg.ingest.proxy,
        "webhook_enabled": cfg.ingest.webhook_enabled,
        "token_configured": !cfg.ingest.token_hash.trim().is_empty(),
        "allow_ips": cfg.ingest.allow_ips,
        "bililive_tools": {
            "api_base": cfg.ingest.recorder.api_base,
            "api_key_set": !cfg.ingest.recorder.api_key.trim().is_empty(),
            "root": cfg.ingest.recorder.root,
            "container_root": cfg.ingest.recorder.container_root,
        },
        "recorder": {
            "kind": cfg.ingest.recorder.kind,
            "root": cfg.ingest.recorder.root,
            "api_base": cfg.ingest.recorder.api_base,
            "api_user": cfg.ingest.recorder.api_user,
            "api_pass_set": !cfg.ingest.recorder.api_pass.is_empty(),
            "api_key_set": !cfg.ingest.recorder.api_key.trim().is_empty(),
            "container_root": cfg.ingest.recorder.container_root,
            "watch_interval_mins": cfg.ingest.recorder.watch_interval_mins,
            "finalize_idle_mins": cfg.ingest.recorder.finalize_idle_mins,
            "move_into_managed": cfg.ingest.recorder.move_into_managed,
            "concat_files": cfg.ingest.recorder.concat_files,
            "extensions": cfg.ingest.recorder.extensions,
            "auto_pipeline": cfg.ingest.recorder.auto_pipeline,
        },
        // 通用 webhook（任意录制软件）：与 [ingest] 共用 token / allow_ips。
        "webhook": {
            "root": cfg.ingest.webhook.root,
            "container_root": cfg.ingest.webhook.container_root,
            "auto_pipeline": cfg.ingest.webhook.auto_pipeline,
            "path": "/api/ingest/webhook/<token>",
            "enabled": cfg.ingest.webhook_enabled
                && !cfg.ingest.token_hash.trim().is_empty()
                && !cfg.ingest.webhook.root.trim().is_empty(),
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

/// 本机默认出口 IP（给设置页拼「局域网 Webhook 地址」用）。
///
/// UDP `connect` 只选路由、不发包；不依赖外部服务。无默认路由/只有回环时返回 None，
/// 前端会退回 `location.origin` 并提示用户手填。
fn local_lan_ip() -> Option<String> {
    let sock = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    sock.connect("8.8.8.8:80").ok()?;
    let ip = sock.local_addr().ok()?.ip();
    if ip.is_loopback() || ip.is_unspecified() {
        None
    } else {
        Some(ip.to_string())
    }
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

    if let Some(v) = body.get("proxy") {
        let raw = v
            .as_str()
            .ok_or_else(|| err(StatusCode::BAD_REQUEST, "proxy 必须是字符串"))?
            .trim();
        // 允许清空（空串 = 直连）；非空则必须是带 scheme 的地址
        if !raw.is_empty()
            && !["http://", "https://", "socks5://", "socks5h://"]
                .iter()
                .any(|p| raw.starts_with(p))
        {
            return Err(err(
                StatusCode::BAD_REQUEST,
                format!("proxy 要写成 http:// / socks5:// / socks5h:// 开头：{raw}"),
            ));
        }
        ingest.insert("proxy".into(), serde_json::json!(raw));
    }

    if let Some(blt) = body.get("bililive_tools") {
        let blt = blt
            .as_object()
            .ok_or_else(|| err(StatusCode::BAD_REQUEST, "bililive_tools 必须是对象"))?;
        let mut out = serde_json::Map::new();
        for key in ["api_base", "root", "container_root"] {
            if let Some(v) = blt.get(key) {
                let raw = v
                    .as_str()
                    .ok_or_else(|| err(StatusCode::BAD_REQUEST, format!("bililive_tools.{key} 必须是字符串")))?
                    .trim();
                if key == "api_base"
                    && !raw.is_empty()
                    && !(raw.starts_with("http://") || raw.starts_with("https://"))
                {
                    return Err(err(
                        StatusCode::BAD_REQUEST,
                        format!("bililive_tools.api_base 需要以 http:// 或 https:// 开头：{raw}"),
                    ));
                }
                out.insert(key.into(), serde_json::json!(raw));
            }
        }
        if let Some(v) = blt.get("api_key") {
            let raw = v
                .as_str()
                .ok_or_else(|| err(StatusCode::BAD_REQUEST, "bililive_tools.api_key 必须是字符串"))?;
            if !raw.trim().is_empty() {
                out.insert("api_key".into(), serde_json::json!(raw));
            }
        }
        if !out.is_empty() {
            ingest.insert("bililive_tools".into(), serde_json::Value::Object(out));
        }
    }

    if let Some(rec) = body.get("recorder") {
        let rec = rec
            .as_object()
            .ok_or_else(|| err(StatusCode::BAD_REQUEST, "recorder 必须是对象"))?;
        let mut out = serde_json::Map::new();

        if let Some(v) = rec.get("kind") {
            let s = v
                .as_str()
                .ok_or_else(|| err(StatusCode::BAD_REQUEST, "recorder.kind 必须是字符串"))?
                .trim();
            if !["bililive", "bililive-tools", "none"].contains(&s) {
                return Err(err(
                    StatusCode::BAD_REQUEST,
                    format!("recorder.kind 只能是 bililive / bililive-tools / none：{s}"),
                ));
            }
            out.insert("kind".into(), serde_json::json!(s));
        }
        for key in ["root", "api_base", "api_user", "container_root"] {
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
        // biliLive-tools 的访问密钥同理：空串 = 不修改（前端只回 api_key_set 布尔）
        if let Some(v) = rec.get("api_key") {
            let s = v
                .as_str()
                .ok_or_else(|| err(StatusCode::BAD_REQUEST, "recorder.api_key 必须是字符串"))?;
            if !s.trim().is_empty() {
                out.insert("api_key".into(), serde_json::json!(s));
            }
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

    // 通用 webhook（`[ingest.webhook]` 子表）：root / container_root / auto_pipeline。
    // ⚠️ 它**必须**是子表（`IngestConfig::webhook`），与 downloader 那个字符串键相反。
    if let Some(wh) = body.get("webhook") {
        let wh = wh
            .as_object()
            .ok_or_else(|| err(StatusCode::BAD_REQUEST, "webhook 必须是对象"))?;
        let mut out = serde_json::Map::new();
        for key in ["root", "container_root"] {
            if let Some(v) = wh.get(key) {
                let s = v
                    .as_str()
                    .ok_or_else(|| err(StatusCode::BAD_REQUEST, format!("webhook.{key} 必须是字符串")))?
                    .trim();
                out.insert(key.into(), serde_json::json!(s));
            }
        }
        if let Some(v) = wh.get("auto_pipeline") {
            let s = v
                .as_str()
                .ok_or_else(|| err(StatusCode::BAD_REQUEST, "webhook.auto_pipeline 必须是字符串"))?;
            if !AUTO_PIPELINES.contains(&s) {
                return Err(err(
                    StatusCode::BAD_REQUEST,
                    format!("webhook.auto_pipeline 只能是 {} 之一：{s}", AUTO_PIPELINES.join(" / ")),
                ));
            }
            out.insert("auto_pipeline".into(), serde_json::json!(s));
        }
        let root = out.get("root").and_then(|v| v.as_str()).unwrap_or("");
        let container = out
            .get("container_root")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        if !root.is_empty() && !std::path::Path::new(root).is_dir() {
            warnings.push(format!(
                "通用 webhook 的录像目录当前不可访问（{root}）：事件里的路径会解析失败。若在网络盘/另一台机器上，请确认 YekiTrim 进程也读得到。"
            ));
        }
        // 相对路径会按**服务进程的 CWD**解析（`resolve_webhook_completion` 里就是
        // `Path::is_dir()` + 相对 root join）—— 换个启动方式/换个工作目录就静默失效，
        // 而配置里那句「宿主机绝对路径」只有人读得懂。
        if !root.is_empty() && !std::path::Path::new(root).is_absolute() {
            warnings.push(
                "通用 webhook 的录像目录要填**绝对路径**：相对路径按服务进程的当前工作目录解析，换个启动方式就会失效。"
                    .into(),
            );
        }
        if root.is_empty() && !container.is_empty() {
            warnings.push(
                "通用 webhook 填了 container_root 但 root 为空 —— 入口不会启用（root 是宿主机上的落点）。"
                    .into(),
            );
        }
        if !out.is_empty() {
            ingest.insert("webhook".into(), serde_json::Value::Object(out));
        }
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

/// 把 `[ingest]` / `[storage]` 的字段合并进**程序配置**（`config::global_config_path()`）。
///
/// ⚠️ **与运行时读的那份必须是同一个文件**（2026-11-13）：以前这里写顶层
/// `<数据>/.yekitrim/config.toml`，而设置页写 `users/<uid>/…` —— 两份长得一模一样，
/// 一半设置保存成功却不生效。现在两侧都走 `config::global_config_path()`。
async fn write_global_config(
    ingest: serde_json::Map<String, serde_json::Value>,
    storage: serde_json::Map<String, serde_json::Value>,
) -> Result<(), AppError> {
    write_global_config_at(
        &yekitrim_core::config::global_config_path(),
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
    let probe_dir = yekitrim_core::owner::owner_root(owner_of(&user));
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
/// ⚠️ **下载必须跑在 `tokio::spawn` 里，不能直接 `await` 在 handler 上**（2026-11-12 事故）：
/// axum 的 handler future 会在**客户端断开时被丢弃**，连带把下载一起取消 —— 40 MB 的包
/// 在慢网络上要几十分钟，用户一刷新 / 一切标签页 / 代理一超时，下载就死在中途，
/// 界面上表现为"一直转圈然后失败"，而 `.part` 停在那里不动。
/// `tokio::spawn` 出来的任务**不随 handler 一起 drop**，断了也照下完。
pub async fn downloader_install(
    State(_state): State<Arc<AppState>>,
    axum::extract::Extension(user): axum::extract::Extension<CurrentUser>,
) -> ApiResult {
    if !user.is_admin() {
        return Err(err(StatusCode::FORBIDDEN, "仅管理员可安装下载器"));
    }
    let (tx, rx) = tokio::sync::oneshot::channel();
    tokio::spawn(async move {
        let cfg = yekitrim_core::config::load_global_config();
        let r = yekitrim_service::downloader::registry::ensure_ytdlp(&cfg).await;
        let _ = tx.send(r);
    });
    let path = match rx.await {
        Ok(Ok(p)) => p,
        Ok(Err(e)) => {
            return Err(err(
                StatusCode::BAD_GATEWAY,
                format!(
                    "安装下载器失败: {e:#}\n\n{}",
                    yekitrim_service::downloader::registry::install_hint()
                ),
            ))
        }
        // 发信号的 task 挂了：下载本身不受影响（它在自己的 task 里），让用户稍后刷新
        Err(_) => {
            return Err(err(
                StatusCode::INTERNAL_SERVER_ERROR,
                "安装任务异常结束；下载可能仍在后台进行，稍后点「刷新」查看",
            ))
        }
    };
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
    // 安装进度（前端轮询画进度条）：托管目录里 `.part*` 的当前字节 vs 钉死的总大小
    let mut part_bytes: u64 = 0;
    let bin_dir = yekitrim_service::downloader::registry::managed_dir();
    if let Ok(rd) = std::fs::read_dir(&bin_dir) {
        for e in rd.flatten() {
            if e.file_name().to_string_lossy().starts_with("yt-dlp.part") {
                part_bytes += e.metadata().map(|m| m.len()).unwrap_or(0);
            }
        }
    }
    // 同样**不含路径**：这个响应给非管理员看
    ok(serde_json::json!({
        "ok": true,
        "kind": kind.label(),
        "ready": found.is_some(),
        "verified": found.map(|i| i.verified).unwrap_or(false),
        "pinned_version": yekitrim_service::downloader::registry::VERSION,
        "license": yekitrim_service::downloader::registry::LICENSE,
        "part_bytes": part_bytes,
        "total_bytes": yekitrim_service::downloader::registry::pin().size,
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
    fn webhook_rejects_non_video_files() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("rec.xml");
        std::fs::write(&file, b"<i/>").unwrap();
        let mut cfg = yekitrim_core::config::Config::default();
        cfg.ingest.webhook.root = dir.path().to_string_lossy().into_owned();
        let err = resolve_webhook_completion(
            &cfg,
            &WebhookCompletion {
                file: file.to_string_lossy().into_owned(),
                danmaku: None,
                room: None,
                platform: None,
                title: None,
            },
        )
        .unwrap_err();
        assert!(err.contains("不像录像文件"), "{err}");
        // 同一目录下的真录像仍然能过（路径/弹幕解析不受影响）
        std::fs::write(dir.path().join("rec.mp4"), b"fake").unwrap();
        assert!(resolve_webhook_completion(
            &cfg,
            &WebhookCompletion {
                file: dir.path().join("rec.mp4").to_string_lossy().into_owned(),
                danmaku: None,
                room: None,
                platform: None,
                title: None,
            },
        )
        .is_ok());
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
        db.replace_channel_map(uid, false, &[("23058".into(), "cut".into(), true)])
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

    /// **接口层**：`PUT /api/channel-map` 抢别人的直播间 → **409**，未登记的房间照旧可自助登记。
    ///
    /// DB 层的机制判据在 `yekitrim_service::db::tests::replace_channel_map_never_steals_another_users_room`；
    /// 这条钉的是**接口语义**（409 而不是 200/403），以及 `allow_takeover` 的接线
    /// （普通用户 false、admin true）—— 2026-11-14 后端 review 的 P1 的回归钉子。
    #[tokio::test]
    async fn channel_map_put_rejects_someone_elses_room_with_409() {
        use crate::api::test_support::*;
        let t = test_app().await;
        let hash = yekitrim_service::auth::hash_password("pass-12345678").unwrap();
        t.auth.db().create_user("cm_alice", &hash, "user", false, 0).unwrap();
        t.auth.db().create_user("cm_bob", &hash, "user", false, 0).unwrap();
        let alice = t.auth.login("cm_alice", "pass-12345678", Some("10.9.8.1"), None).unwrap();
        let bob = t.auth.login("cm_bob", "pass-12345678", Some("10.9.8.2"), None).unwrap();

        // ① alice 登记 26320007（未登记过 ⇒ 允许）
        let res = t
            .call(
                t.req_as(&alice.session_id, &alice.csrf, "PUT", "/channel-map")
                    .header(axum::http::header::CONTENT_TYPE, "application/json")
                    .body(axum::body::Body::from(
                        r#"{"rules":[{"room_id":"26320007","auto_pipeline":"detect"}]}"#,
                    ))
                    .unwrap(),
            )
            .await;
        assert_eq!(res.status(), 200, "未登记的房间该允许自助登记");

        // ② bob 想把 26320007 改成自己的 ⇒ 409，且归属仍是 alice
        let res = t
            .call(
                t.req_as(&bob.session_id, &bob.csrf, "PUT", "/channel-map")
                    .header(axum::http::header::CONTENT_TYPE, "application/json")
                    .body(axum::body::Body::from(
                        r#"{"rules":[{"room_id":"26320007","auto_pipeline":"cut","enabled":false}]}"#,
                    ))
                    .unwrap(),
            )
            .await;
        assert_eq!(res.status(), 409, "抢别人的直播间必须 409");
        let body = body_json(res).await;
        assert!(
            body["error"].as_str().unwrap_or("").contains("该直播间已被其他账号登记"),
            "文案要说清是「已被别人登记」：{body}"
        );
        let alice_id = t.auth.db().find_user("cm_alice").unwrap().unwrap().id;
        let rule = t.auth.db().channel_for_room("26320007").unwrap().unwrap();
        assert_eq!(rule.user_id, alice_id, "归属绝不能被改到 bob");
        assert!(rule.enabled, "也不能被 enabled:false 掐掉");
        assert_eq!(rule.auto_pipeline, "detect", "pipeline 不能被别人改");

        // ③ bob 登记一个**没被登记过**的房间 ⇒ 200（自助登记这条路不能被误伤）
        let res = t
            .call(
                t.req_as(&bob.session_id, &bob.csrf, "PUT", "/channel-map")
                    .header(axum::http::header::CONTENT_TYPE, "application/json")
                    .body(axum::body::Body::from(
                        r#"{"rules":[{"room_id":"18001","auto_pipeline":"none"}]}"#,
                    ))
                    .unwrap(),
            )
            .await;
        assert_eq!(res.status(), 200, "未登记的房间仍要允许占位");
        let bob_id = t.auth.db().find_user("cm_bob").unwrap().unwrap().id;
        assert_eq!(
            t.auth.db().channel_for_room("18001").unwrap().unwrap().user_id,
            bob_id
        );
        // ② 那次失败没有把 alice 的行删掉，① 那次也没有把 bob 的规则写进去
        assert_eq!(t.auth.db().channel_map_list().unwrap().len(), 2, "只该留下两条规则");
    }

    /// webhook token 的形状检查：只有"`<TOKEN_PREFIX>` + 40 位 base64url"才值得跑 scrypt。
    ///
    /// 这是未认证入口上的成本闸门 —— 形状检查失败时直接 401，不碰 KDF。
    ///
    /// ⚠️ **本测试一律从 `TOKEN_PREFIX` 拼，不写字面量前缀**：
    /// 曾经它把前缀写死，而生成端另有一份字面量——两端各写一个字面量时，
    /// 只改一端就**测不出来**（断言保护的是被写死的那份）。现在前缀只有一个来源。
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
        // 前缀不对就不是 token：换一个别的词打头，形状必须不过
        assert!(!token_shape_ok(&format!("notatoken_{}", "a".repeat(40))));
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

    /// 通用 webhook 的补丁必须落成 **`[ingest.webhook]` 子表**，且不碰 `[ingest]` 兄弟键。
    ///
    /// ⚠️ 与 downloader 那条**相反**：`[ingest] downloader` 本身是字符串键，所以必须平铺；
    /// 而 `webhook` 是子表（`IngestConfig::webhook`），提交成 inline table / 顶层键都不会报错，
    /// 只会在**下次读配置时静默变成默认值**（表单显示空白，用户以为没保存）。
    #[tokio::test]
    async fn webhook_patch_writes_a_subtable_and_keeps_siblings() {
        let dir = std::env::temp_dir().join(format!("yekitrim-ingest-wh-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.toml");
        std::fs::write(&path, "[ingest]\nwebhook_enabled = true\ntoken_hash = \"scrypt$keepme\"\n").unwrap();

        let (ingest, _, warnings) = build_ingest_patch(&serde_json::json!({
            "webhook": {
                "root": "/srv/rec",
                "container_root": "/app/video",
                "auto_pipeline": "detect"
            }
        }))
        .unwrap();
        write_global_config_at(&path, config_patch(ingest, serde_json::Map::new()))
            .await
            .unwrap();

        let out = std::fs::read_to_string(&path).unwrap();
        let section = out
            .split("[ingest.webhook]")
            .nth(1)
            .unwrap_or_else(|| panic!("必须有 [ingest.webhook] 表头：\n{out}"));
        for key in [
            "root = \"/srv/rec\"",
            "container_root = \"/app/video\"",
            "auto_pipeline = \"detect\"",
        ] {
            assert!(section.contains(key), "{key} 不在 [ingest.webhook] 里：\n{out}");
        }
        assert!(!out.contains("webhook = {"), "不能退化成 inline table：\n{out}");
        assert!(
            out.contains("token_hash = \"scrypt$keepme\"") && out.contains("webhook_enabled = true"),
            "[ingest] 的兄弟键被冲掉了：\n{out}"
        );
        // root 不可达**只告警不拒绝**（网络盘可能只是暂时没挂上）
        assert!(
            warnings.iter().any(|w| w.contains("/srv/rec")),
            "不可达的 root 要给出警告：{warnings:?}"
        );
        // 相对 root 要单独点名（它按进程 CWD 解析，换启动方式就静默失效）
        let (_, _, warnings) =
            build_ingest_patch(&serde_json::json!({"webhook": {"root": "relative/rec"}})).unwrap();
        assert!(
            warnings.iter().any(|w| w.contains("绝对路径")),
            "相对 root 必须给出警告：{warnings:?}"
        );

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

    #[test]
    fn webhook_parses_bililive_tools_completion() {
        let body = serde_json::json!({
            "id": "evt-1",
            "version": 1,
            "type": "file_completed",
            "data": {
                "filePath": "/app/video/Bilibili/a/[2026-10-29 16-00-00-000][t]/rec.flv",
                "roomId": 123456,
                "platform": "Bilibili"
            }
        });
        match parse_webhook_event(&body).unwrap() {
            ParsedWebhookEvent::Completion(c) => {
                assert!(c.file.ends_with("rec.flv"));
                assert_eq!(c.room.as_deref(), Some("123456"));
                assert_eq!(c.platform.as_deref(), Some("Bilibili"));
            }
            _ => panic!("file_completed 必须解析成 Completion"),
        }
    }

    #[test]
    fn webhook_parses_canonical_and_ignores_non_completion() {
        let body = serde_json::json!({
            "event": "file_completed",
            "file": "Bilibili/a/rec.mp4",
            "room_id": "42",
            "title": "测试"
        });
        match parse_webhook_event(&body).unwrap() {
            ParsedWebhookEvent::Completion(c) => {
                assert_eq!(c.file, "Bilibili/a/rec.mp4");
                assert_eq!(c.room.as_deref(), Some("42"));
                assert_eq!(c.title.as_deref(), Some("测试"));
            }
            _ => panic!("规范事件必须解析成 Completion"),
        }
        assert!(matches!(
            parse_webhook_event(&serde_json::json!({"type": "record_start"})).unwrap(),
            ParsedWebhookEvent::Ignored(_)
        ));
        assert!(parse_webhook_event(&serde_json::json!({"type": "unknown"})).is_err());
    }

    /// 录播姬原样打过来的形状（`EventType` + `EventData`，**PascalCase**）必须认。
    ///
    /// 这条曾经回 400：`parse_webhook_event` 只 `to_ascii_lowercase()`，`FileClosed`
    /// 变成 `fileclosed`，永远匹配不上 `file_closed` —— 于是「同一个 URL 兼容更多软件」
    /// 只是注释里的愿望，而录制软件会把 4xx 当成"没送到"反复重试。
    #[test]
    fn webhook_accepts_recorder_pascal_case_event_type() {
        let body = serde_json::json!({
            "EventType": "FileClosed",
            "EventData": {
                "RelativePath": "26320007-主播/2026-10-29 16-00-00/rec.flv",
                "RoomId": 26320007
            }
        });
        match parse_webhook_event(&body).unwrap() {
            ParsedWebhookEvent::Completion(c) => {
                assert_eq!(c.file, "26320007-主播/2026-10-29 16-00-00/rec.flv");
                assert_eq!(c.room.as_deref(), Some("26320007"));
            }
            _ => panic!("录播姬 FileClosed 必须解析成 Completion"),
        }
        // 不建项目的事件（PascalCase 写法）同样要认，否则 400 会让录制软件重试。
        assert!(matches!(
            parse_webhook_event(&serde_json::json!({"EventType": "SessionEnded"})).unwrap(),
            ParsedWebhookEvent::Ignored(_)
        ));
        // 三种来源三种写法都归一（归一化只此一份，别在解析处再写一遍匹配）。
        assert_eq!(normalize_event_type("FileClosed"), "file_closed");
        assert_eq!(normalize_event_type("file_closed"), "file_closed");
        assert_eq!(normalize_event_type("file-closed"), "file_closed");
        assert_eq!(normalize_event_type("FILE_CLOSED"), "file_closed");
        assert_eq!(normalize_event_type("  FileClosed  "), "file_closed");
        assert_eq!(normalize_event_type(""), "");
    }

    #[test]
    fn webhook_maps_container_path_and_prefers_postprocessed() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let session = root.join("Bilibili").join("a").join("[2026-10-29 16-00-00-000][t]");
        std::fs::create_dir_all(&session).unwrap();
        std::fs::write(session.join("rec.flv"), b"flv").unwrap();
        std::fs::write(session.join("rec-后处理.mp4"), b"mp4").unwrap();
        std::fs::write(session.join("rec.xml"), b"<i/>").unwrap();

        let mut cfg = yekitrim_core::config::Config::default();
        cfg.ingest.webhook.root = root.to_string_lossy().into_owned();
        cfg.ingest.webhook.container_root = "/app/video".into();
        let completion = WebhookCompletion {
            file: "/app/video/Bilibili/a/[2026-10-29 16-00-00-000][t]/rec.flv".into(),
            danmaku: None,
            room: Some("123456".into()),
            platform: Some("Bilibili".into()),
            title: None,
        };
        let r = resolve_webhook_completion(&cfg, &completion).unwrap();
        assert_eq!(r.project_root, session);
        assert_eq!(r.input_name, "rec-后处理.mp4");
        assert_eq!(r.danmaku_name.as_deref(), Some("rec.xml"));
        assert!(r.source_label().contains("123456"));
    }

    #[test]
    fn webhook_rejects_path_escape() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join("Bilibili")).unwrap();
        let mut cfg = yekitrim_core::config::Config::default();
        cfg.ingest.webhook.root = root.to_string_lossy().into_owned();
        cfg.ingest.webhook.container_root = "/app/video".into();
        let completion = WebhookCompletion {
            file: "/app/video/../outside.mp4".into(),
            danmaku: None,
            room: None,
            platform: None,
            title: None,
        };
        assert!(resolve_webhook_completion(&cfg, &completion).is_err());
    }
}
