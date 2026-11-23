//! 房间 → 归属映射（**待分配池**）：指派 / 重试 / 兜底扫描 / 「首个管理员」兜底。
//!
//! ⚠️ 会话状态机只有 `closing` 会被入库：指派与导入都必须把待分配会话推到 `closing`；
//! 扫描（`scan_now`）**不能把已有归属的会话打回** `unassigned`（改状态前先看 `owner_id`）。

use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;

use crate::auth::CurrentUser;
use crate::model::AppState;

use super::*;
use super::records::internal;

pub(super) fn first_admin(db: &yekitrim_service::db::Db) -> Result<i64, AppError> {
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


#[cfg(test)]
mod tests {
    
    use crate::ingest::testutil::{db, ev};
    use crate::ingest::webhook::handle_event;

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
}
