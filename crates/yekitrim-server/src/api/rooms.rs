//! `api::rooms`：直播间自助登记（录播姬）与注册开关。
//!
//! ⚠️ 不叫 `ingest`：**真正的入库 HTTP handler 在 `crate::ingest`**（webhook / 记录 / 待分配池）。
//! 这里只有「用户填房间号 → 去录播姬加房间」这一块，叫 `rooms` 才不会和它撞概念。

use std::sync::Arc;

use axum::{
    extract::{Extension, Path as AxumPath, State},
    http::StatusCode,
    Json,
};

use crate::model::AppState;
use super::*;

// ===================== 直播间（录播姬） =====================

/// 当前录播姬对接状态。**给普通用户只说能不能用**（不泄露地址/错误细节）。
pub(super) fn recorder_client() -> Option<yekitrim_service::recorder::RecorderClient> {
    let cfg = yekitrim_core::config::load_global_config();
    yekitrim_service::recorder::RecorderClient::from_config(&cfg.ingest.recorder)
}

/// `GET /api/rooms`：我的直播间 + 录播姬实时状态。
///
/// `available=false`（未对接 / 连不上）时前端把「配置直播间」置灰 —— 用户不该看到
/// 管理员才懂的那些配置指引。
pub(super) async fn rooms_list(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<crate::auth::CurrentUser>,
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
    let cfg = yekitrim_core::config::load_global_config();
    let summaries = if cfg.ingest.recorder.root.trim().is_empty() {
        None
    } else {
        match tokio::task::spawn_blocking(move || crate::ingest_worker::list_recordings(&cfg)).await {
            Ok(Ok(files)) => Some(crate::ingest_worker::room_summaries(&files)),
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
pub(super) async fn rooms_add(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<crate::auth::CurrentUser>,
    Json(body): Json<serde_json::Value>,
) -> ApiResult {
    let input = body
        .get("url")
        .or_else(|| body.get("room_id"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let room_id = yekitrim_service::recorder::parse_room_id(&input)
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
pub(super) async fn rooms_delete(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<crate::auth::CurrentUser>,
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
        let id = yekitrim_service::recorder::parse_room_id(&room_id)
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
pub(super) async fn admin_recorder_test() -> ApiResult {
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
pub(super) async fn admin_registration_put(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<crate::auth::CurrentUser>,
    Json(body): Json<serde_json::Value>,
) -> ApiResult {
    let enabled = body
        .get("enabled")
        .and_then(|v| v.as_bool())
        .ok_or_else(|| err(StatusCode::BAD_REQUEST, "缺少 enabled"))?;
    yekitrim_service::invite::set_registration_enabled(enabled)
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("写入配置失败: {e:#}")))?;
    let _ = state.auth.db().audit(Some(user.id), "registration_switch", if enabled { "on" } else { "off" });
    ok(serde_json::json!({ "ok": true, "registration_enabled": enabled }))
}







#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::test_support::*;


    use axum::body::Body;
    use axum::http::Request as HttpRequest;
    use tower::util::ServiceExt;
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
    let hash = yekitrim_service::auth::hash_password("pass-12345678").unwrap();
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
                    format!("{}={}", yekitrim_service::auth::COOKIE_SID, b.session_id),
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
}
