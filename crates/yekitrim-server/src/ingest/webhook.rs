//! 录播姬 webhook 的**会话流程**：验 token → 去重 → 累积文件 → 推 `closing`（P3）。
//!
//! - 接收端**只做「验 token → 去重 → 落库」** 然后立即 2xx（官方对非 2xx 最多重试 3 次、payload 不变）；
//!   重活（完结判定、合并、落位、跑管线）交给后台 worker（`crate::ingest_worker`）。
//! - token 存哈希（复用 `yekitrim_service::auth` 的 scrypt），比对失败 401；`allow_ips` 白名单。
//! - 事件处理**不依赖顺序**：`FileOpening/FileClosed` 只累积文件列表，`SessionEnded` 只把状态推到
//!   `closing`；真正的完结判定在 worker 里（含空闲窗口）。

use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;

use crate::auth::ClientIp;
use crate::model::AppState;

use super::*;
use super::webhook_event::{ParsedWebhookEvent, normalize_kind, parse_webhook_event};
use super::webhook_path::{prepare_webhook_project, resolve_webhook_completion};

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
pub(super) fn token_shape_ok(token: &str) -> bool {
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
        if let Err(wait) = crate::ratelimit::check_webhook(ip) {
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


#[cfg(test)]
mod tests {
    use super::*;
    use crate::ingest::testutil::{db, ev};
    use crate::ingest::webhook_event::WebhookCompletion;
    use crate::ingest::webhook_path::resolve_webhook_completion;

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
}
