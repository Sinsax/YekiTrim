//! `api::auth`：会话、注册、改密、doctor 与账号列表。

use std::sync::Arc;

use axum::{
    extract::{Extension, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};

use crate::model::AppState;
use super::*;

// ---- 认证（M1 / §3.5） ----

/// `GET /api/me`：已登录 → 200 `{user, must_change_password}`；未登录 → 中间件给 401。
pub(super) async fn me(Extension(user): Extension<crate::auth::CurrentUser>) -> ApiResult {
    ok(serde_json::json!({
        "ok": true,
        "user": user.json(),
        "must_change_password": user.must_change_password,
    }))
}

/// `POST /api/auth/login {username, password}` → 200 + `Set-Cookie`；401/429。
pub(super) async fn auth_login(
    State(state): State<Arc<AppState>>,
    crate::auth::ClientIp(ip): crate::auth::ClientIp,
    headers: axum::http::HeaderMap,
    Json(body): Json<serde_json::Value>,
) -> Response {
    let username = body.get("username").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let password = body.get("password").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let user_agent = headers
        .get(axum::http::header::USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());
    let secure = crate::auth::tls_terminated(&headers);

    let auth = state.auth.clone();
    // scrypt（N=2^15）是 CPU 密集：必须在 blocking 线程池里跑，否则会卡住 tokio worker
    let result = tokio::task::spawn_blocking(move || {
        auth.login(&username, &password, ip.as_deref(), user_agent.as_deref())
    })
    .await;

    match result {
        Ok(Ok(bundle)) => {
            let mut resp = Json(serde_json::json!({
                "ok": true,
                "user": bundle.user.json(),
                "must_change_password": bundle.user.must_change_password,
            }))
            .into_response();
            crate::auth::append_cookies(&mut resp, &crate::auth::session_cookies(&bundle, secure));
            resp
        }
        Ok(Err(crate::auth::LoginError::RateLimited(secs))) => err(
            StatusCode::TOO_MANY_REQUESTS,
            format!("尝试次数过多，请在 {} 分钟后再试", (secs + 59) / 60),
        )
        .into_response(),
        Ok(Err(crate::auth::LoginError::Denied(msg))) => {
            err(StatusCode::UNAUTHORIZED, msg).into_response()
        }
        Ok(Err(crate::auth::LoginError::Internal(msg))) => {
            tracing::error!("登录失败（内部错误）: {msg}");
            err(StatusCode::INTERNAL_SERVER_ERROR, "登录失败，请查看服务端日志").into_response()
        }
        Err(e) => {
            tracing::error!("登录任务失败: {e}");
            err(StatusCode::INTERNAL_SERVER_ERROR, "登录失败，请重试").into_response()
        }
    }
}

/// `POST /api/auth/logout`：删会话行 + 清 cookie。
pub(super) async fn auth_logout(
    State(state): State<Arc<AppState>>,
    crate::auth::ClientIp(ip): crate::auth::ClientIp,
    Extension(user): Extension<crate::auth::CurrentUser>,
) -> Response {
    state.auth.logout(&user.session_id, user.id, ip.as_deref());
    let mut resp = ok(serde_json::json!({ "ok": true })).into_response();
    crate::auth::append_cookies(&mut resp, &crate::auth::clear_cookies());
    resp
}

/// `POST /api/auth/register {username, password, invite}`：邀请码自助注册（公开接口）。
///
/// 三重条件：`[auth] registration_enabled` 打开 + 邀请码本身有效 + 用户名/密码合规。
/// 成功后与登录一样下发会话 cookie（注册即登录，角色固定 `user`）。
pub(super) async fn auth_register(
    State(state): State<Arc<AppState>>,
    crate::auth::ClientIp(ip): crate::auth::ClientIp,
    headers: axum::http::HeaderMap,
    Json(body): Json<serde_json::Value>,
) -> Response {
    // 总闸：关着就直接 403（不透露有没有可用的码）
    if !vtrim_core::config::load_global_config().auth.registration_enabled {
        return err(StatusCode::FORBIDDEN, "注册未开放（管理员可用 vtrim user invite 生成邀请码并开启）").into_response();
    }
    let username = body.get("username").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let password = body.get("password").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let invite = body.get("invite").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let user_agent = headers
        .get(axum::http::header::USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());
    let secure = crate::auth::tls_terminated(&headers);

    let auth = state.auth.clone();
    // 与登录同理：scrypt 是 CPU 密集，挪到 blocking 线程池
    let result = tokio::task::spawn_blocking(move || {
        auth.register(&username, &password, &invite, ip.as_deref(), user_agent.as_deref())
    })
    .await;

    match result {
        Ok(Ok(bundle)) => {
            let mut resp = Json(serde_json::json!({
                "ok": true,
                "user": bundle.user.json(),
                "must_change_password": false,
            }))
            .into_response();
            crate::auth::append_cookies(&mut resp, &crate::auth::session_cookies(&bundle, secure));
            resp
        }
        Ok(Err(crate::auth::LoginError::RateLimited(secs))) => err(
            StatusCode::TOO_MANY_REQUESTS,
            format!("尝试次数过多，请在 {} 分钟后再试", (secs + 59) / 60),
        )
        .into_response(),
        // 注册失败一律 403：不区分"码不对"和"码已用过"，避免被拿来枚举
        Ok(Err(crate::auth::LoginError::Denied(msg))) => {
            err(StatusCode::FORBIDDEN, msg).into_response()
        }
        Ok(Err(crate::auth::LoginError::Internal(msg))) => {
            tracing::error!("注册失败（内部错误）: {msg}");
            err(StatusCode::INTERNAL_SERVER_ERROR, "注册失败，请查看服务端日志").into_response()
        }
        Err(e) => {
            tracing::error!("注册任务失败: {e}");
            err(StatusCode::INTERNAL_SERVER_ERROR, "注册失败，请查看服务端日志").into_response()
        }
    }
}

/// `PUT /api/auth/password {old_password, new_password}`：改自己的密码。
///
/// 成功后 `must_change_password=false`（顶部横幅消失）、初始密码文件被删、
/// 本人的其它会话全部失效（当前会话保留）。
pub(super) async fn auth_password(
    State(state): State<Arc<AppState>>,
    crate::auth::ClientIp(ip): crate::auth::ClientIp,
    Extension(user): Extension<crate::auth::CurrentUser>,
    Json(body): Json<serde_json::Value>,
) -> Response {
    let old_password = body.get("old_password").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let new_password = body.get("new_password").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let auth = state.auth.clone();
    let user_id = user.id;
    let session_id = user.session_id.clone();
    let result = tokio::task::spawn_blocking(move || {
        auth.change_password(user_id, &session_id, &old_password, &new_password, ip.as_deref())
    })
    .await;

    match result {
        Ok(Ok(())) => ok(serde_json::json!({ "ok": true, "must_change_password": false })).into_response(),
        Ok(Err(crate::auth::LoginError::Denied(msg))) => {
            err(StatusCode::BAD_REQUEST, msg).into_response()
        }
        Ok(Err(crate::auth::LoginError::RateLimited(_))) => {
            err(StatusCode::TOO_MANY_REQUESTS, "操作过于频繁，请稍后再试").into_response()
        }
        Ok(Err(crate::auth::LoginError::Internal(msg))) => {
            tracing::error!("改密失败（内部错误）: {msg}");
            err(StatusCode::INTERNAL_SERVER_ERROR, "修改密码失败，请查看服务端日志").into_response()
        }
        Err(e) => {
            tracing::error!("改密任务失败: {e}");
            err(StatusCode::INTERNAL_SERVER_ERROR, "修改密码失败，请重试").into_response()
        }
    }
}

pub(super) async fn doctor(State(state): State<Arc<AppState>>) -> ApiResult {
    let ws = workspace_from_state(&state).await;
    let home = vtrim_core::path::vtrim_home();
    let models_dir = home.join("models");
    // 与 /api/models 同源：按运行时查找顺序（配置 → 用户目录 → 程序目录）判"能不能用"
    let asr = vtrim_core::config::load_global_config().asr;
    let model_status: Vec<_> = vtrim_core::download::model_names().iter().map(|m| {
        let st = vtrim_core::download::model_install(&asr, m);
        serde_json::json!({
            "name": m,
            "installed": st.installed,
            "dir": vtrim_core::path::display(&st.dir),
            "source": st.source.as_str(),
        })
    }).collect();
    ok(serde_json::json!({
        "ffmpeg": command_available("ffmpeg").await,
        "ffprobe": command_available("ffprobe").await,
        "models_dir": models_dir.to_string_lossy(),
        "models": model_status,
        "workspace": vtrim_core::path::display(&ws.config_file),
    }))
}

/// `GET /api/admin/users`：账号列表（**仅 admin**）。只回展示需要的字段，绝不含口令哈希。
///
/// 用途：入库待分配池的"指派给谁"下拉、后续管理页。
pub(super) async fn admin_users(State(state): State<Arc<AppState>>) -> ApiResult {
    let db = state.auth.db();
    let users = db
        .list_users()
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("读取账号失败: {e:#}")))?;
    let out: Vec<serde_json::Value> = users
        .iter()
        .map(|u| {
            serde_json::json!({
                "id": u.id,
                "username": u.username,
                "role": if u.role == "admin" { "admin" } else { "user" },
                "disabled": u.disabled,
                "must_change_password": u.must_change_password,
                "quota_bytes": u.quota_bytes,
                "created_at": u.created_at,
            })
        })
        .collect();
    ok(serde_json::json!({ "users": out }))
}







#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::test_support::*;


    use axum::body::Body;
    use axum::http::Request as HttpRequest;
    use tower::util::ServiceExt;
/// 仍在使用初始密码时，除改密/登出/自查外一律 403（服务端强制，不靠前端横幅）。
#[tokio::test]
async fn must_change_password_blocks_everything_but_the_password_flow() {
    let t = test_app_with_must_change(true).await;
    // 放行：自查（前端靠它显示横幅并弹改密框）
    assert_eq!(
        t.call(t.req("GET", "/me").body(Body::empty()).unwrap()).await.status(),
        StatusCode::OK
    );
    // 其余接口一律 403
    for (method, uri) in [
        ("GET", "/projects"),
        ("GET", "/jobs"),
        ("GET", "/project-data/status?project=1"),
    ] {
        let res = t.call(t.req(method, uri).body(Body::empty()).unwrap()).await;
        assert_eq!(res.status(), StatusCode::FORBIDDEN, "{method} {uri} 应被改密门拦住");
    }
    // 改密本身必须能用，改完立刻恢复
    let body = serde_json::json!({
        "old_password": "initial-pass-1234",
        "new_password": "pass-12345678"
    })
    .to_string();
    let res = t
        .call(
            t.req("PUT", "/auth/password")
                .header(axum::http::header::CONTENT_TYPE, "application/json")
                .body(Body::from(body))
                .unwrap(),
        )
        .await;
    assert_eq!(res.status(), StatusCode::OK, "改密请求本身不应被拦：{:?}", res.status());
    assert_eq!(
        t.call(t.req("GET", "/projects").body(Body::empty()).unwrap()).await.status(),
        StatusCode::OK,
        "改密后应恢复正常"
    );
}

/// 登录 → `/api/me` → 改密 → `must_change_password` 变 false（§3.5.10 第 1–5 条）。
#[tokio::test]
async fn login_me_change_password_flow() {
    use crate::auth::{AuthService, Listening};
    let dir = tempfile::tempdir().unwrap();
    let auth = AuthService::open(dir.path().to_path_buf(), Listening::Loopback).unwrap();
    let hash = vtrim_service::auth::hash_password("initial-pass-1234").unwrap();
    auth.db().create_user("admin", &hash, "admin", true, 0).unwrap();
    let ws = Workspace::load(Some(Path::new("vtrim.toml"))).expect("workspace");
    let app = router(Arc::new(AppState::with_auth(ws, std::env::temp_dir(), auth.clone())));

    // 初始密码：错的 401、对的 200 + Set-Cookie
    let bad = app
        .clone()
        .oneshot(
            HttpRequest::builder()
                .method("POST")
                .uri("/auth/login")
                .header(axum::http::header::CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"username":"admin","password":"wrong-pass"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(bad.status(), StatusCode::UNAUTHORIZED);

    let good = app
        .clone()
        .oneshot(
            HttpRequest::builder()
                .method("POST")
                .uri("/auth/login")
                .header(axum::http::header::CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"username":"admin","password":"initial-pass-1234"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(good.status(), StatusCode::OK);
    let set_cookie: Vec<String> = good
        .headers()
        .get_all(axum::http::header::SET_COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok().map(|s| s.to_string()))
        .collect();
    assert_eq!(set_cookie.len(), 2, "应同时下发 vtrim_sid 与 vtrim_csrf: {set_cookie:?}");
    assert!(set_cookie[0].contains("HttpOnly"));
    let sid = set_cookie[0]
        .split(';')
        .next()
        .unwrap()
        .trim_start_matches("vtrim_sid=")
        .to_string();
    let csrf = set_cookie[1]
        .split(';')
        .next()
        .unwrap()
        .trim_start_matches("vtrim_csrf=")
        .to_string();
    let cookie = format!("vtrim_sid={sid}; vtrim_csrf={csrf}");

    // /api/me：仍在使用初始密码
    let me = app
        .clone()
        .oneshot(
            HttpRequest::builder()
                .uri("/me")
                .header(axum::http::header::COOKIE, &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(me.status(), StatusCode::OK);
    let v = body_json(me).await;
    assert_eq!(v["user"]["username"], "admin");
    assert_eq!(v["user"]["role"], "admin");
    assert_eq!(v["must_change_password"], true);

    // 旧密码错 → 400；正确 → 200 且之后 must_change_password=false
    let wrong_old = app
        .clone()
        .oneshot(
            HttpRequest::builder()
                .method("PUT")
                .uri("/auth/password")
                .header(axum::http::header::COOKIE, &cookie)
                .header(vtrim_service::auth::CSRF_HEADER, &csrf)
                .header(axum::http::header::CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"old_password":"nope","new_password":"new-pass-5678"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(wrong_old.status(), StatusCode::BAD_REQUEST);

    let changed = app
        .clone()
        .oneshot(
            HttpRequest::builder()
                .method("PUT")
                .uri("/auth/password")
                .header(axum::http::header::COOKIE, &cookie)
                .header(vtrim_service::auth::CSRF_HEADER, &csrf)
                .header(axum::http::header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    r#"{"old_password":"initial-pass-1234","new_password":"new-pass-5678"}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(changed.status(), StatusCode::OK, "改密应成功");

    let me = app
        .clone()
        .oneshot(
            HttpRequest::builder()
                .uri("/me")
                .header(axum::http::header::COOKIE, &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(body_json(me).await["must_change_password"], false);

    // 新密码可登录、旧密码不可
    let relogin = |pw: &'static str| {
        let app = app.clone();
        async move {
            app.oneshot(
                HttpRequest::builder()
                    .method("POST")
                    .uri("/auth/login")
                    .header(axum::http::header::CONTENT_TYPE, "application/json")
                    .body(Body::from(format!(
                        r#"{{"username":"admin","password":"{pw}"}}"#
                    )))
                    .unwrap(),
            )
            .await
            .unwrap()
        }
    };
    assert_eq!(relogin("initial-pass-1234").await.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(relogin("new-pass-5678").await.status(), StatusCode::OK);

    // 登出后会话立即失效
    let out = app
        .clone()
        .oneshot(
            HttpRequest::builder()
                .method("POST")
                .uri("/auth/logout")
                .header(axum::http::header::COOKIE, &cookie)
                .header(vtrim_service::auth::CSRF_HEADER, &csrf)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(out.status(), StatusCode::OK);
    let me = app
        .oneshot(
            HttpRequest::builder()
                .uri("/me")
                .header(axum::http::header::COOKIE, &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(me.status(), StatusCode::UNAUTHORIZED);
}

/// 连续错密码触发限流 → 429，且**正确密码也被挡住**（15 分钟内）。
#[tokio::test]
async fn login_is_rate_limited_after_five_failures() {
    use crate::auth::{AuthService, Listening};
    let dir = tempfile::tempdir().unwrap();
    let auth = AuthService::open(dir.path().to_path_buf(), Listening::Loopback).unwrap();
    let hash = vtrim_service::auth::hash_password("initial-pass-1234").unwrap();
    auth.db().create_user("admin", &hash, "admin", false, 0).unwrap();
    let ws = Workspace::load(Some(Path::new("vtrim.toml"))).expect("workspace");
    let app = router(Arc::new(AppState::with_auth(ws, std::env::temp_dir(), auth)));

    let attempt = |pw: &'static str| {
        let app = app.clone();
        async move {
            app.oneshot(
                HttpRequest::builder()
                    .method("POST")
                    .uri("/auth/login")
                    .header(axum::http::header::CONTENT_TYPE, "application/json")
                    .body(Body::from(format!(
                        r#"{{"username":"admin","password":"{pw}"}}"#
                    )))
                    .unwrap(),
            )
            .await
            .unwrap()
            .status()
        }
    };
    for _ in 0..5 {
        assert_eq!(attempt("wrong-pass").await, StatusCode::UNAUTHORIZED);
    }
    assert_eq!(attempt("wrong-pass").await, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(
        attempt("initial-pass-1234").await,
        StatusCode::TOO_MANY_REQUESTS,
        "限流窗口内正确密码也应被拒绝"
    );
}

/// 被禁用的账号：不能登录（文案直出），已建会话立即 403。
#[tokio::test]
async fn disabled_account_cannot_login_or_use_session() {
    let t = test_app().await;
    // 会话正常时可用
    let res = t
        .call(t.req("GET", "/jobs").body(Body::empty()).unwrap())
        .await;
    assert_eq!(res.status(), StatusCode::OK);

    assert!(t.auth.db().set_disabled("admin", true).unwrap());

    let res = t
        .call(t.req("GET", "/jobs").body(Body::empty()).unwrap())
        .await;
    assert_eq!(res.status(), StatusCode::FORBIDDEN, "禁用后已有会话必须拒绝");
    let v = body_json(res).await;
    assert!(v["error"].as_str().unwrap_or("").contains("禁用"), "{v}");

    let res = t
        .call(
            HttpRequest::builder()
                .method("POST")
                .uri("/auth/login")
                .header(axum::http::header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    r#"{"username":"admin","password":"initial-pass-1234"}"#,
                ))
                .unwrap(),
        )
        .await;
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
    let v = body_json(res).await;
    assert!(v["error"].as_str().unwrap_or("").contains("禁用"), "{v}");
}
}
