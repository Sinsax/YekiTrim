//! HTTP 侧会话鉴权（多用户服务 P1 step-2，设计稿 §3 / §3.5）。
//!
//! 不变式（§3.5）：**所有请求（含 `127.0.0.1`）都必须带会话**，没有免登录分支、
//! 没有来源地址判定。单人部署 = 只有一个 admin 账号的部署，认证路径一行都不分叉。
//!
//! 组成：
//! - [`AuthService`]：账号账本句柄 + 登录限流 + 会话读写（滑动续期 / 登出 / 改密）；
//! - [`require_user`]：axum 中间件，校验 cookie 会话、写请求校验 CSRF、把
//!   [`CurrentUser`] 塞进请求扩展供 handler 取用；
//! - cookie 构造与解析：`vtrim_sid`（HttpOnly）+ `vtrim_csrf`（JS 可读，供 `X-CSRF-Token`）。

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use axum::extract::{Request, State};
use axum::http::{header, HeaderMap, HeaderValue, Method, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::Json;

use crate::service::auth as kdf;
use crate::service::db::{self, Db, SessionRow};
use crate::service::user;

use super::model::AppState;

/// 登录失败上限与统计窗口（§3.5.9 暴力登录防护）。
const MAX_LOGIN_ATTEMPTS: u32 = 5;
const LOGIN_WINDOW: Duration = Duration::from_secs(15 * 60);

/// 监听范围（`/api/health` 的 `listening` 字段）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Listening {
    Loopback,
    Remote,
}

impl Listening {
    pub fn as_str(self) -> &'static str {
        match self {
            Listening::Loopback => "loopback",
            Listening::Remote => "remote",
        }
    }
}

/// 当前请求已认证的用户（由 `require_user` 写入请求扩展）。
#[derive(Debug, Clone)]
pub struct CurrentUser {
    pub id: i64,
    pub username: String,
    pub role: String,
    pub must_change_password: bool,
    pub session_id: String,
}

impl CurrentUser {
    pub fn is_admin(&self) -> bool {
        self.role == "admin"
    }

    pub fn json(&self) -> serde_json::Value {
        serde_json::json!({ "id": self.id, "username": self.username, "role": self.role })
    }
}

/// 登录成功后的凭据（交给 handler 写 cookie）。
#[derive(Debug, Clone)]
pub struct SessionBundle {
    pub session_id: String,
    pub csrf: String,
    pub expires_at: i64,
    pub user: CurrentUser,
}

/// 登录失败原因（handler 映射成 401 / 429 文案）。
#[derive(Debug, Clone)]
pub enum LoginError {
    /// 限流：附带剩余秒数
    RateLimited(u64),
    /// 账号或密码不对 / 账号被禁用
    Denied(String),
    /// 账本/口令 KDF 故障（500）
    Internal(String),
}

#[derive(Debug, Clone)]
struct Attempt {
    count: u32,
    first: Instant,
}

/// 会话与登录限流的服务端单例。
pub struct AuthService {
    db: Arc<Db>,
    data_dir: PathBuf,
    listening: Listening,
    attempts: Mutex<HashMap<String, Attempt>>,
}

impl std::fmt::Debug for AuthService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AuthService")
            .field("data_dir", &self.data_dir)
            .field("listening", &self.listening)
            .finish_non_exhaustive()
    }
}

impl AuthService {
    /// 打开（必要时创建）账号账本。身份数据不可缺省：打不开就是启动失败（§3.5.7）。
    pub fn open(data_dir: PathBuf, listening: Listening) -> Result<Arc<Self>> {
        std::fs::create_dir_all(&data_dir)
            .with_context(|| format!("无法创建数据目录 {}", data_dir.display()))?;
        let db_path = data_dir.join("vtrim.db");
        let db = Db::open(&db_path)
            .with_context(|| format!("无法打开账号账本 {}", db_path.display()))?;
        let svc = Arc::new(Self {
            db: Arc::new(db),
            data_dir,
            listening,
            attempts: Mutex::new(HashMap::new()),
        });
        let expired = svc.purge_expired();
        if expired > 0 {
            tracing::info!("已清理 {expired} 条过期会话");
        }
        Ok(svc)
    }

    pub fn db(&self) -> &Db {
        &self.db
    }

    /// 账号数据目录（启动日志与 `vtrim doctor` 用）。
    pub fn data_dir(&self) -> &PathBuf {
        &self.data_dir
    }

    pub fn listening(&self) -> Listening {
        self.listening
    }

    // ---- 登录限流（username + 客户端 IP，内存态，重启即清） ----

    fn rate_key(username: &str, ip: Option<&str>) -> String {
        format!("{}|{}", username.trim().to_lowercase(), ip.unwrap_or("-"))
    }

    /// 返回还需等待的秒数（0 = 未限流）。
    fn rate_wait(&self, key: &str) -> u64 {
        let mut map = self.attempts.lock().unwrap_or_else(|e| e.into_inner());
        let Some(a) = map.get(key) else { return 0 };
        if a.first.elapsed() >= LOGIN_WINDOW {
            map.remove(key);
            return 0;
        }
        if a.count < MAX_LOGIN_ATTEMPTS {
            return 0;
        }
        (LOGIN_WINDOW - a.first.elapsed()).as_secs() + 1
    }

    fn rate_fail(&self, key: &str) {
        let mut map = self.attempts.lock().unwrap_or_else(|e| e.into_inner());
        let entry = map.entry(key.to_string()).or_insert(Attempt { count: 0, first: Instant::now() });
        if entry.first.elapsed() >= LOGIN_WINDOW {
            *entry = Attempt { count: 1, first: Instant::now() };
            return;
        }
        entry.count += 1;
    }

    fn rate_clear(&self, key: &str) {
        let mut map = self.attempts.lock().unwrap_or_else(|e| e.into_inner());
        map.remove(key);
    }

    /// 校验口令并建会话（同步：scrypt 是 CPU 密集，handler 负责 `spawn_blocking`）。
    pub fn login(
        &self,
        username: &str,
        password: &str,
        ip: Option<&str>,
        user_agent: Option<&str>,
    ) -> std::result::Result<SessionBundle, LoginError> {
        let username = username.trim();
        if username.is_empty() || password.is_empty() {
            return Err(LoginError::Denied("用户名或密码不能为空".into()));
        }
        let key = Self::rate_key(username, ip);
        let wait = self.rate_wait(&key);
        if wait > 0 {
            return Err(LoginError::RateLimited(wait));
        }

        let internal = |e: anyhow::Error| LoginError::Internal(format!("{e:#}"));

        let Some(user) = self.db.find_user(username).map_err(internal)? else {
            // 用户不存在也要走一次 KDF 之外的固定代价，避免"用户名是否存在"被时间侧信道区分
            self.rate_fail(&key);
            let _ = self.db.audit_ip(None, "login_failed", &format!("no such user: {username}"), ip);
            return Err(LoginError::Denied("用户名或密码错误".into()));
        };

        let hash = self.db.user_password_hash(username).map_err(internal)?.unwrap_or_default();
        let matched = kdf::verify_password(&hash, password).unwrap_or(false);
        if !matched {
            self.rate_fail(&key);
            let _ = self.db.audit_ip(Some(user.id), "login_failed", "bad password", ip);
            return Err(LoginError::Denied("用户名或密码错误".into()));
        }
        if user.disabled {
            self.rate_fail(&key);
            let _ = self.db.audit_ip(Some(user.id), "login_disabled", "", ip);
            return Err(LoginError::Denied("账号已被禁用，请联系管理员".into()));
        }

        self.rate_clear(&key);
        let session_id = kdf::new_token();
        let csrf = kdf::new_token();
        let expires_at = db::now_secs() + kdf::SESSION_TTL_SECS;
        self.db
            .create_session(&session_id, user.id, &csrf, expires_at, user_agent)
            .map_err(internal)?;
        let _ = self.db.touch_last_login(user.id);
        let _ = self.db.audit_ip(Some(user.id), "login", "", ip);
        // 首次成功登录即删除初始密码文件（删不掉只 warn，§3.5.2）
        user::remove_initial_password_file();

        Ok(SessionBundle {
            session_id: session_id.clone(),
            csrf,
            expires_at,
            user: CurrentUser {
                id: user.id,
                username: user.username,
                role: user.role,
                must_change_password: user.must_change_password,
                session_id,
            },
        })
    }

    pub fn session(&self, session_id: &str) -> Result<Option<SessionRow>> {
        self.db.session(session_id)
    }

    pub fn logout(&self, session_id: &str, user_id: i64, ip: Option<&str>) {
        let _ = self.db.delete_session(session_id);
        let _ = self.db.audit_ip(Some(user_id), "logout", "", ip);
    }

    /// 滑动续期：剩余不足 `SESSION_TTL - 1h` 时才写库（见 §3.5.6）。
    pub fn renew_if_needed(&self, session: &SessionRow) {
        let now = db::now_secs();
        if session.expires_at - now > kdf::SESSION_TTL_SECS - kdf::SESSION_RENEW_AFTER_SECS {
            return;
        }
        let _ = self.db.touch_session(&session.id, now + kdf::SESSION_TTL_SECS);
    }

    /// 改自己的密码：校验旧密码 → 写新哈希（`must_change_password=false`）→
    /// 删初始密码文件 → 清掉本人的其它会话（保留当前会话）。
    pub fn change_password(
        &self,
        user_id: i64,
        current_session: &str,
        old_password: &str,
        new_password: &str,
        ip: Option<&str>,
    ) -> std::result::Result<(), LoginError> {
        let internal = |e: anyhow::Error| LoginError::Internal(format!("{e:#}"));
        let user = self
            .db
            .list_users()
            .map_err(internal)?
            .into_iter()
            .find(|u| u.id == user_id)
            .ok_or_else(|| LoginError::Denied("账号不存在".into()))?;
        let hash = self.db.user_password_hash(&user.username).map_err(internal)?.unwrap_or_default();
        if !kdf::verify_password(&hash, old_password).unwrap_or(false) {
            let _ = self.db.audit_ip(Some(user_id), "password_change_failed", "bad old password", ip);
            return Err(LoginError::Denied("原密码不正确".into()));
        }
        kdf::validate_password(new_password)
            .map_err(|e| LoginError::Denied(format!("{e:#}")))?;
        if new_password == old_password {
            return Err(LoginError::Denied("新密码不能与原密码相同".into()));
        }
        let new_hash = kdf::hash_password(new_password).map_err(internal)?;
        self.db
            .set_password_by_id(user_id, &new_hash, false)
            .map_err(internal)?;
        let removed = self.db.delete_user_sessions(user_id, Some(current_session)).unwrap_or(0);
        user::remove_initial_password_file();
        let _ = self.db.audit_ip(
            Some(user_id),
            "password_change",
            &format!("other_sessions_revoked={removed}"),
            ip,
        );
        tracing::info!("账号 {} 已修改密码（清理其它会话 {removed} 个）", user.username);
        Ok(())
    }

    /// 清理过期会话（启动 + 每日一次的后台任务用）。
    pub fn purge_expired(&self) -> usize {
        self.db.purge_expired_sessions(db::now_secs()).unwrap_or(0)
    }
}

/// 客户端 IP，取自 `ConnectInfo`（**不信任 `X-Forwarded-For`**：反代后它不是可信输入，
/// 见 §3.5.1）。`oneshot` 单测里没有 ConnectInfo 时为 `None`，不影响功能。
#[derive(Debug, Clone)]
pub struct ClientIp(pub Option<String>);

#[axum::async_trait]
impl<S> axum::extract::FromRequestParts<S> for ClientIp
where
    S: Send + Sync,
{
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(
        parts: &mut axum::http::request::Parts,
        _state: &S,
    ) -> Result<Self, Self::Rejection> {
        let ip = parts
            .extensions
            .get::<axum::extract::ConnectInfo<std::net::SocketAddr>>()
            .map(|c| c.0.ip().to_string());
        Ok(ClientIp(ip))
    }
}

// ---- cookie ----

pub fn cookie_value(headers: &HeaderMap, name: &str) -> Option<String> {
    for value in headers.get_all(header::COOKIE) {
        let Ok(raw) = value.to_str() else { continue };
        for part in raw.split(';') {
            if let Some((k, v)) = part.trim().split_once('=') {
                if k == name {
                    return Some(v.to_string());
                }
            }
        }
    }
    None
}

/// 登录成功时下发的两个 cookie：会话（HttpOnly）+ CSRF 令牌（JS 可读）。
pub fn session_cookies(bundle: &SessionBundle, secure: bool) -> Vec<String> {
    let max_age = bundle.expires_at - db::now_secs();
    let secure_flag = if secure { "; Secure" } else { "" };
    vec![
        format!(
            "{}={}; Path={}; Max-Age={}; HttpOnly; SameSite=Lax{}",
            kdf::COOKIE_SID, bundle.session_id, kdf::COOKIE_PATH, max_age, secure_flag
        ),
        format!(
            "{}={}; Path={}; Max-Age={}; SameSite=Lax{}",
            kdf::COOKIE_CSRF, bundle.csrf, kdf::COOKIE_PATH, max_age, secure_flag
        ),
    ]
}

pub fn clear_cookies() -> Vec<String> {
    vec![
        format!("{}=; Path={}; Max-Age=0; HttpOnly; SameSite=Lax", kdf::COOKIE_SID, kdf::COOKIE_PATH),
        format!("{}=; Path={}; Max-Age=0; SameSite=Lax", kdf::COOKIE_CSRF, kdf::COOKIE_PATH),
    ]
}

pub fn append_cookies(resp: &mut Response, cookies: &[String]) {
    for c in cookies {
        match HeaderValue::from_str(c) {
            Ok(v) => {
                resp.headers_mut().append(header::SET_COOKIE, v);
            }
            Err(_) => tracing::warn!("cookie 头非法，已跳过: {c}"),
        }
    }
}

/// 请求是否走了 TLS 终止的代理（只看可信代理头，仅作提示，§3.5.9）。
pub fn tls_terminated(headers: &HeaderMap) -> bool {
    headers
        .get("x-forwarded-proto")
        .and_then(|v| v.to_str().ok())
        .map(|v| v.split(',').next().unwrap_or("").trim().eq_ignore_ascii_case("https"))
        .unwrap_or(false)
}

// ---- 中间件 ----

fn json_error(status: StatusCode, msg: impl Into<String>) -> Response {
    (status, Json(serde_json::json!({ "ok": false, "error": msg.into() }))).into_response()
}

fn is_safe_method(m: &Method) -> bool {
    matches!(*m, Method::GET | Method::HEAD | Method::OPTIONS)
}

/// `require_user`：会话 cookie → 账号快照 → CSRF（写请求）→ 注入 [`CurrentUser`]。
pub async fn require_user(State(state): State<Arc<AppState>>, mut req: Request, next: Next) -> Response {
    let auth = state.auth.clone();
    let Some(sid) = cookie_value(req.headers(), kdf::COOKIE_SID) else {
        return json_error(StatusCode::UNAUTHORIZED, "未登录，请先登录");
    };
    let session = match auth.session(&sid) {
        Ok(Some(s)) => s,
        Ok(None) => return json_error(StatusCode::UNAUTHORIZED, "登录已过期，请重新登录"),
        Err(e) => {
            tracing::error!("读取会话失败: {e:#}");
            return json_error(StatusCode::INTERNAL_SERVER_ERROR, "读取会话失败");
        }
    };
    if session.expires_at <= db::now_secs() {
        let _ = auth.db().delete_session(&sid);
        return json_error(StatusCode::UNAUTHORIZED, "登录已过期，请重新登录");
    }
    if session.disabled {
        return json_error(StatusCode::FORBIDDEN, "账号已被禁用，请联系管理员");
    }
    if !is_safe_method(req.method()) {
        let token = req
            .headers()
            .get(kdf::CSRF_HEADER)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        if token.is_empty() {
            return json_error(StatusCode::FORBIDDEN, "缺少 CSRF 令牌（请通过页面操作）");
        }
        if !constant_time_eq(token.as_bytes(), session.csrf.as_bytes()) {
            return json_error(StatusCode::FORBIDDEN, "CSRF 令牌不匹配，请刷新页面后重试");
        }
    }

    auth.renew_if_needed(&session);
    req.extensions_mut().insert(CurrentUser {
        id: session.user_id,
        username: session.username,
        role: session.role,
        must_change_password: session.must_change_password,
        session_id: session.id,
    });
    next.run(req).await
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    use subtle::ConstantTimeEq;
    if a.len() != b.len() || a.is_empty() {
        return false;
    }
    a.ct_eq(b).into()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headers_with(pairs: &[(&str, &str)]) -> HeaderMap {
        let mut h = HeaderMap::new();
        for (k, v) in pairs {
            h.append(
                header::HeaderName::from_bytes(k.as_bytes()).unwrap(),
                HeaderValue::from_str(v).unwrap(),
            );
        }
        h
    }

    #[test]
    fn parses_cookie_header() {
        let h = headers_with(&[("cookie", "a=1; vtrim_sid=abc; vtrim_csrf=xyz")]);
        assert_eq!(cookie_value(&h, kdf::COOKIE_SID).as_deref(), Some("abc"));
        assert_eq!(cookie_value(&h, kdf::COOKIE_CSRF).as_deref(), Some("xyz"));
        assert_eq!(cookie_value(&h, "missing"), None);
    }

    #[test]
    fn cookie_flags_are_locked_down() {
        let b = SessionBundle {
            session_id: "sid".into(),
            csrf: "csrf".into(),
            expires_at: db::now_secs() + 100,
            user: CurrentUser {
                id: 1,
                username: "admin".into(),
                role: "admin".into(),
                must_change_password: false,
                session_id: String::new(),
            },
        };
        let cookies = session_cookies(&b, false);
        assert!(cookies[0].starts_with("vtrim_sid=sid;"), "{}", cookies[0]);
        assert!(cookies[0].contains("HttpOnly"), "会话 cookie 必须 HttpOnly");
        assert!(!cookies[1].contains("HttpOnly"), "CSRF cookie 需 JS 可读");
        assert!(session_cookies(&b, true).iter().all(|c| c.contains("Secure")));
        assert!(clear_cookies()[0].contains("Max-Age=0"));
    }

    #[test]
    fn constant_time_eq_rejects_length_mismatch_and_empty() {
        assert!(constant_time_eq(b"abc", b"abc"));
        assert!(!constant_time_eq(b"abc", b"abd"));
        assert!(!constant_time_eq(b"abc", b"ab"));
        assert!(!constant_time_eq(b"", b""));
    }

    #[test]
    fn rate_limiter_locks_after_five_failures() {
        let dir = tempfile::tempdir().unwrap();
        let auth = AuthService::open(dir.path().to_path_buf(), Listening::Loopback).unwrap();
        let key = AuthService::rate_key("Admin", Some("127.0.0.1"));
        assert_eq!(auth.rate_wait(&key), 0);
        for _ in 0..MAX_LOGIN_ATTEMPTS {
            auth.rate_fail(&key);
        }
        assert!(auth.rate_wait(&key) > 0, "5 次失败后必须限流");
        auth.rate_clear(&key);
        assert_eq!(auth.rate_wait(&key), 0);
        // 大小写与 IP 归一化：Admin@1.2.3.4 与 admin@1.2.3.4 同一个桶
        assert_eq!(key, AuthService::rate_key("admin", Some("127.0.0.1")));
    }

    #[test]
    fn safe_methods_skip_csrf() {
        assert!(is_safe_method(&Method::GET));
        assert!(is_safe_method(&Method::HEAD));
        assert!(is_safe_method(&Method::OPTIONS));
        assert!(!is_safe_method(&Method::POST));
        assert!(!is_safe_method(&Method::PUT));
        assert!(!is_safe_method(&Method::DELETE));
    }

    #[test]
    fn tls_terminated_reads_proxy_header_only_as_hint() {
        assert!(tls_terminated(&headers_with(&[("x-forwarded-proto", "https")])));
        assert!(tls_terminated(&headers_with(&[("x-forwarded-proto", "https, http")])));
        assert!(!tls_terminated(&headers_with(&[("x-forwarded-proto", "http")])));
        assert!(!tls_terminated(&HeaderMap::new()));
    }
}
