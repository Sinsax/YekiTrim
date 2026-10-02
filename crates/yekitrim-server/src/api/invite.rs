//! `api::invite`：注册邀请码（自助注册，admin）。

use std::sync::Arc;

use axum::{
    extract::{Extension, Path as AxumPath, State},
    http::StatusCode,
    Json,
};

use crate::model::AppState;
use super::*;

// ===================== 邀请码（自助注册，admin） =====================

/// `GET /api/admin/invites`：注册开关 + 邀请码列表。**明文回给管理员**（可重复查看、复制再发），
/// 每个码带 `used` 标志与绑定到的账号。
pub(super) async fn admin_invites(State(state): State<Arc<AppState>>) -> ApiResult {
    let db = state.auth.db();
    let rows = db
        .list_invites()
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("读取邀请码失败: {e:#}")))?;
    let usernames = db
        .list_users()
        .map(|us| us.into_iter().map(|u| (u.id, u.username)).collect::<std::collections::HashMap<_, _>>())
        .unwrap_or_default();
    let now = yekitrim_service::db::now_secs();
    let invites: Vec<serde_json::Value> = rows
        .iter()
        .map(|r| {
            serde_json::json!({
                "id": r.id,
                "token": r.token,
                "used": r.used_at.is_some(),
                "note": r.note,
                "created_at": r.created_at,
                "expires_at": r.expires_at,
                "used_at": r.used_at,
                "used_by": r.used_by,
                "used_by_name": r.used_by.and_then(|id| usernames.get(&id).cloned()),
                "created_by_name": r.created_by.and_then(|id| usernames.get(&id).cloned()),
                "revoked": r.revoked,
                "open": r.is_open(now),
            })
        })
        .collect();
    ok(serde_json::json!({
        "registration_enabled": yekitrim_core::config::load_global_config().auth.registration_enabled,
        "invites": invites,
    }))
}

/// `POST /api/admin/invites {note?, days?, count?}`：生成邀请码。
/// 明文同时落库 → 之后 `GET` 列表里也能看到（管理员权限，用户要求）。
pub(super) async fn admin_invite_create(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<crate::auth::CurrentUser>,
    Json(body): Json<serde_json::Value>,
) -> ApiResult {
    let count = body.get("count").and_then(|v| v.as_u64()).unwrap_or(1);
    if count == 0 || count > 50 {
        return Err(err(StatusCode::BAD_REQUEST, "count 取值范围 1..=50"));
    }
    let days = body.get("days").and_then(|v| v.as_u64()).map(|d| d as u32);
    let days = yekitrim_service::invite::validate_days(days)
        .map_err(|e| err(StatusCode::BAD_REQUEST, format!("{e}")))?;
    let note = body
        .get("note")
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    let db = state.auth.db();
    let expires_at = days.map(|d| yekitrim_service::db::now_secs() + (d as i64) * 86400);
    let mut codes = Vec::new();
    for _ in 0..count {
        let code = yekitrim_service::auth::new_invite_code();
        db.create_invite(&code, note.as_deref(), Some(user.id), expires_at)
            .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("写入邀请码失败: {e:#}")))?;
        codes.push(code);
    }
    // 有可用码就顺手把总闸打开（与 CLI `vtrim user invite` 一致）
    if let Err(e) = yekitrim_service::invite::set_registration_enabled(true) {
        tracing::warn!("开启注册开关失败: {e:#}");
    }
    let _ = db.audit(Some(user.id), "invite_create", &format!("count={count}"));

    ok(serde_json::json!({
        "ok": true,
        "codes": codes,
        "registration_enabled": true,
        "expires_at": expires_at,
    }))
}

/// `DELETE /api/admin/invites/:id`：撤销一个邀请码（已注册的账号不受影响）。
pub(super) async fn admin_invite_revoke(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<crate::auth::CurrentUser>,
    AxumPath(id): AxumPath<i64>,
) -> ApiResult {
    let db = state.auth.db();
    let n = db
        .revoke_invite(id)
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("撤销失败: {e:#}")))?;
    if n == 0 {
        return Err(err(StatusCode::NOT_FOUND, "邀请码不存在或已是撤销状态"));
    }
    let _ = db.audit(Some(user.id), "invite_revoke", &format!("id={id}"));
    ok(serde_json::json!({ "ok": true }))
}







#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::test_support::*;


    use axum::body::Body;
    use axum::http::Request as HttpRequest;
    use tower::util::ServiceExt;
/// 邀请码注册链路：
/// - HTTP 层：注册开关关着（测试环境没有全局配置 → 默认关）→ 403，且不透露细节；
/// - 服务层：有效码 → 建号（角色固定 user）+ 建会话 + 码被消费；码复用/假码/重名/弱密码 → 拒；
/// - 限流：连续失败会被锁（防邀请码爆破）。
#[tokio::test]
async fn register_flow_consumes_invite_and_respects_switch() {
    use yekitrim_service::auth as kdf;
    let t = test_app().await;

    // 1) 开关关着（默认）→ 公开接口 403
    let body = serde_json::json!({
        "username": "newbie", "password": "pass-12345678", "invite": kdf::new_invite_code()
    })
    .to_string();
    let res = t
        .app
        .clone()
        .oneshot(
            HttpRequest::builder()
                .method("POST")
                .uri("/auth/register")
                .header(axum::http::header::CONTENT_TYPE, "application/json")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::FORBIDDEN, "注册未开放必须 403");

    // 2) 造一个有效码 → 注册成功，用码的人记在邀请码行上
    let code = kdf::new_invite_code();
    let invite_id = t
        .auth
        .db()
        .create_invite(&code, Some("测试"), None, None)
        .unwrap();
    let bundle = t
        .auth
        .register("newbie", "pass-12345678", &code, Some("10.0.0.1"), None)
        .expect("有效码应能注册");
    assert_eq!(bundle.user.role, "user", "自助注册只能是 user");
    assert_eq!(bundle.user.username, "newbie");
    assert!(!bundle.user.must_change_password, "自设密码不提示改密");
    let row = t.auth.db().find_invite_by_id(invite_id).unwrap().unwrap();
    assert_eq!(row.used_by, Some(bundle.user.id));
    assert!(!row.is_open(yekitrim_service::db::now_secs()), "用完即失效");

    // 注册即登录：这个 sid 直接能过鉴权
    let res = t
        .app
        .clone()
        .oneshot(
            HttpRequest::builder()
                .method("GET")
                .uri("/me")
                .header(
                    axum::http::header::COOKIE,
                    format!("{}={}", yekitrim_service::auth::COOKIE_SID, bundle.session_id),
                )
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK, "注册后会话应立即可用");

    // 3) 同一个码再用 / 假码 / 重名（大小写不同也算）/ 弱密码 → 全拒
    assert!(
        t.auth.register("someone", "pass-12345678", &code, Some("10.0.0.2"), None).is_err(),
        "码不能复用"
    );
    assert!(
        t.auth.register("other", "pass-12345678", "vtrim_inv_aaaaaaaaaaaaaaaa", Some("10.0.0.3"), None).is_err(),
        "假码必须拒"
    );
    assert!(
        t.auth.register("NEWBIE", "pass-12345678", &kdf::new_invite_code(), Some("10.0.0.4"), None).is_err(),
        "大小写不同的重名也要拒"
    );
    assert!(
        t.auth.register("weakling", "short", &kdf::new_invite_code(), Some("10.0.0.5"), None).is_err(),
        "弱密码必须拒"
    );
    assert!(t.auth.db().find_user("someone").unwrap().is_none(), "失败不留半个账号");

    // 4) 限流：同一个 IP 连续失败会被锁（邀请码爆破防护）
    let mut limited = false;
    for _ in 0..6 {
        if let Err(crate::auth::LoginError::RateLimited(_)) =
            t.auth.register("bruteforce", "pass-12345678", "vtrim_inv_bbbbbbbbbbbb", Some("10.9.9.9"), None)
        {
            limited = true;
            break;
        }
    }
    assert!(limited, "连续失败必须触发限流");
}

/// 邀请码管理是 admin-only：普通账号（哪怕已登录）读写都拿 403。
#[tokio::test]
async fn invite_admin_endpoints_reject_non_admin() {
    let t = test_app().await;
    let hash = yekitrim_service::auth::hash_password("pass-12345678").unwrap();
    t.auth.db().create_user("plain", &hash, "user", false, 0).unwrap();
    let b = t.auth.login("plain", "pass-12345678", Some("10.1.1.1"), None).unwrap();
    let cookie = format!("{}={}", yekitrim_service::auth::COOKIE_SID, b.session_id);
    for (method, uri) in [
        ("GET", "/admin/invites"),
        ("POST", "/admin/invites"),
        ("DELETE", "/admin/invites/1"),
        ("PUT", "/admin/invites/registration"),
        // LLM 用量同样带各用户的项目路径 → 与日志一样只给 admin
        ("GET", "/admin/usage"),
    ] {
        let res = t
            .app
            .clone()
            .oneshot(
                HttpRequest::builder()
                    .method(method)
                    .uri(uri)
                    .header(axum::http::header::COOKIE, cookie.clone())
                    .header(yekitrim_service::auth::CSRF_HEADER, b.csrf.clone())
                    .header(axum::http::header::CONTENT_TYPE, "application/json")
                    .body(Body::from("{}"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::FORBIDDEN, "{method} {uri} 必须拒绝普通用户");
    }
}

/// 管理员能**重复查看**邀请码明文，并一眼看出「已使用 / 未使用 + 绑给了谁」（用户要求）。
#[tokio::test]
async fn admin_invites_expose_plaintext_and_usage() {
    let t = test_app().await;
    // 生成 2 个（这一步会顺手打开注册开关；测试末尾恢复，别影响别的用例）
    let body = serde_json::json!({ "note": "给小明", "count": 2 }).to_string();
    let res = t
        .call(
            t.req("POST", "/admin/invites")
                .header(axum::http::header::CONTENT_TYPE, "application/json")
                .body(Body::from(body))
                .unwrap(),
        )
        .await;
    let j = body_json(res).await;
    let codes: Vec<String> = j["codes"]
        .as_array()
        .expect("codes")
        .iter()
        .map(|c| c.as_str().unwrap_or("").to_string())
        .collect();
    assert_eq!(codes.len(), 2, "{j}");
    assert!(codes.iter().all(|c| c.starts_with("vtrim_inv_")), "{j}");
    assert_eq!(j["registration_enabled"], true);

    // 列表里能再看到同样的明文，且都是「未使用」
    let res = t.call(t.req("GET", "/admin/invites").body(Body::empty()).unwrap()).await;
    let j = body_json(res).await;
    let rows = j["invites"].as_array().unwrap().clone();
    for c in &codes {
        let row = rows.iter().find(|r| r["token"] == c.as_str()).unwrap_or_else(|| panic!("列表应回明文: {j}"));
        assert_eq!(row["used"], false, "{row}");
        assert_eq!(row["note"], "给小明");
    }

    // 用一个码注册 → 该行变「已使用」并显示绑给了谁；另一个不受影响
    let bundle = t
        .auth
        .register("xiaoming", "pass-12345678", &codes[0], Some("10.2.2.2"), None)
        .expect("有效码应能注册");
    let res = t.call(t.req("GET", "/admin/invites").body(Body::empty()).unwrap()).await;
    let j = body_json(res).await;
    let rows = j["invites"].as_array().unwrap();
    let used = rows.iter().find(|r| r["token"] == codes[0].as_str()).unwrap();
    assert_eq!(used["used"], true, "{j}");
    assert_eq!(used["used_by"], bundle.user.id);
    assert_eq!(used["used_by_name"], "xiaoming");
    assert_eq!(used["open"], false, "用过的码不再是「可用」");
    let other = rows.iter().find(|r| r["token"] == codes[1].as_str()).unwrap();
    assert_eq!(other["used"], false, "另一个码不受影响");

    // 撤销后列表里也看得出来
    let id = used["id"].as_i64().unwrap();
    let res = t.call(t.req("DELETE", &format!("/admin/invites/{id}")).body(Body::empty()).unwrap()).await;
    assert_eq!(res.status(), StatusCode::OK);
    let res = t.call(t.req("GET", "/admin/invites").body(Body::empty()).unwrap()).await;
    let j = body_json(res).await;
    let row = j["invites"].as_array().unwrap().iter().find(|r| r["id"] == id).cloned().unwrap();
    assert_eq!(row["revoked"], true, "{j}");

    // 恢复注册开关：不给后续用例留下「已开启」状态
    yekitrim_service::invite::set_registration_enabled(false).unwrap();
}
}
