//! HTTP 侧会话鉴权（多用户服务 P1 step-2，设计稿 §3 / §3.5）。
//!
//! 不变式（§3.5）：**所有请求（含 `127.0.0.1`）都必须带会话**，没有免登录分支、
//! 没有来源地址判定。单人部署 = 只有一个 admin 账号的部署，认证路径一行都不分叉。
//!
//! 组成：
//! - [`AuthService`]：账号账本句柄 + 登录限流 + 会话读写（滑动续期 / 登出 / 改密）；
//! - [`require_user`]：axum 中间件，校验 cookie 会话、写请求校验 CSRF、把
//!   [`CurrentUser`] 塞进请求扩展供 handler 取用；
//! - cookie 构造与解析：`yekitrim_sid`（HttpOnly）+ `yekitrim_csrf`（JS 可读，供 `X-CSRF-Token`）。

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

use yekitrim_service::auth as kdf;
use yekitrim_service::db::{self, Db, SessionRow};
use yekitrim_service::user;

use super::model::AppState;

/// 登录失败上限与统计窗口（§3.5.9 暴力登录防护）。
const MAX_LOGIN_ATTEMPTS: u32 = 5;
const LOGIN_WINDOW: Duration = Duration::from_secs(15 * 60);

/// 登录失败计数表最多留多少个 key（超出后按窗口清理，仍满则不再记新 key）。
///
/// 表是按 `用户名|IP` 建的，而用户名**来自请求且不要求存在** —— 不设上限的话，
/// 逐个换用户名发请求就能把内存吃光（见 [`AuthService::rate_fail`]）。
const MAX_LOGIN_KEYS: usize = 4096;

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

/// **身份 → 数据归属**的唯一转换点（见 `yekitrim_core::owner`）。
///
/// 这是"单人 / 多用户两种形态共用一套业务"的那个**唯一分叉点**：
///
/// - 本地免登录（`--local-no-auth`，托盘形态）注入的是合成身份，其会话 id 是
///   [`LOCAL_SESSION_ID`] ⇒ [`Owner::Local`]，数据落在**程序目录**下（现状不变）；
/// - 正常登录 ⇒ [`Owner::User`]，数据落在该账号自己的树下。
///
/// ⚠️ **判据是"这次是不是本地形态"，不是"账号是不是 admin"** —— 别改成按 role 判。
/// 本地模式注入的合成身份**恰好**是 admin，但管理员在**多用户模式下登录**时，
/// 他的数据仍应落在自己的 `users/<uid>/` 里，而不是落到全局去。
///
/// ⚠️ 以后新增"按用户分数据"的地方，一律经这里拿 `Owner`，
/// **别自己看 `session_id` / `role` 拼路径**（那就是硬规矩 3 禁掉的业务分支）。
pub fn owner_of(user: &CurrentUser) -> yekitrim_core::owner::Owner {
    if user.session_id == LOCAL_SESSION_ID {
        yekitrim_core::owner::Owner::Local
    } else {
        yekitrim_core::owner::Owner::User(user.id)
    }
}

/// 已认证调用者（写进**响应**扩展，供最外层的 `access_log` 中间件记「谁在操作」）。
///
/// 请求扩展里的 [`CurrentUser`] 只在被认证层包住的 handler 里可见，挂在最外层的
/// 访问日志读不到请求扩展 —— 所以认证成功后往响应里再放一份。
#[derive(Debug, Clone)]
pub struct Actor {
    pub username: String,
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
        let db_path = data_dir.join("yekitrim.db");
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

    /// 账本的 `Arc`（给需要长期持有它的组件，如 LLM 用量 sink）。
    pub fn db_arc(&self) -> Arc<Db> {
        Arc::clone(&self.db)
    }

    /// 账号数据目录（启动日志与 `yekitrim doctor` 用）。
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
        // ⚠️ 这张表历史上**只增不减**：`rate_wait` 只在"被查询的同一个 key 已过期"时才删它，
        // 而攻击者可以每次换一个用户名 → 表被无限撑大（公网部署下就是一条内存 DoS）。
        // 表过大时先按窗口清一遍；清完仍满就不再记新 key（宁可少一层该 key 的限流，
        // 也不把内存交给对方 —— 全站按 IP 的限流仍挡着，见 `ratelimit`）。
        if map.len() >= MAX_LOGIN_KEYS {
            map.retain(|_, a| a.first.elapsed() < LOGIN_WINDOW);
            if map.len() >= MAX_LOGIN_KEYS {
                return;
            }
        }
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
        let bundle = self.issue_session(user.id, &user.username, &user.role, user.must_change_password, user_agent)
            .map_err(internal)?;
        let _ = self.db.touch_last_login(user.id);
        let _ = self.db.audit_ip(Some(user.id), "login", "", ip);
        // 首次成功登录即删除初始密码文件（删不掉只 warn，§3.5.2）
        user::remove_initial_password_file();
        Ok(bundle)
    }

    /// 建会话（登录 / 注册共用）。
    fn issue_session(
        &self,
        user_id: i64,
        username: &str,
        role: &str,
        must_change_password: bool,
        user_agent: Option<&str>,
    ) -> Result<SessionBundle> {
        let session_id = kdf::new_token();
        let csrf = kdf::new_token();
        let expires_at = db::now_secs() + kdf::SESSION_TTL_SECS;
        self.db.create_session(&session_id, user_id, &csrf, expires_at, user_agent)?;
        Ok(SessionBundle {
            session_id: session_id.clone(),
            csrf,
            expires_at,
            user: CurrentUser {
                id: user_id,
                username: username.to_string(),
                role: role.to_string(),
                must_change_password,
                session_id,
            },
        })
    }

    /// 邀请码注册（`POST /api/auth/register`，§3.5 之外的自助注册）。
    ///
    /// 三重条件缺一不可：注册开关打开（调用方查配置）、邀请码格式/哈希对得上、码未被用过。
    /// 用户名查重 + 消费码 + 建账号在 DB 层是一次原子操作（`register_with_invite`），
    /// 所以两个人同时抢同一个码时只有一个能成。注册成功直接建会话（免得再登一次）。
    pub fn register(
        &self,
        username: &str,
        password: &str,
        invite: &str,
        ip: Option<&str>,
        user_agent: Option<&str>,
    ) -> std::result::Result<SessionBundle, LoginError> {
        let username = username.trim();
        let internal = |e: anyhow::Error| LoginError::Internal(format!("{e:#}"));

        // 注册也限流：刷邀请码同样是 IP 维度的爆破（key 与登录分开，互不影响）
        let key = format!("register|{}", ip.unwrap_or("-"));
        let wait = self.rate_wait(&key);
        if wait > 0 {
            return Err(LoginError::RateLimited(wait));
        }
        let deny = |msg: &str, key: &str, ip: Option<&str>, this: &Self| {
            this.rate_fail(key);
            let _ = this.db.audit_ip(None, "register_failed", msg, ip);
        };

        if let Err(e) = validate_username(username) {
            return Err(LoginError::Denied(format!("{e}")));
        }
        if let Err(e) = kdf::validate_password(password) {
            return Err(LoginError::Denied(format!("{e}")));
        }
        if let Some(existing) = self.db.find_user_ci(username).map_err(internal)? {
            // 大小写不敏感：`Admin` 不能顶着 `admin` 注册
            deny(&format!("username taken: {username}"), &key, ip, self);
            return Err(LoginError::Denied(format!("用户名已被占用（现有账号 {}）", existing.username)));
        }

        // 邀请码：明文存在库里（管理员可重复查看），所以这里就是一次明文查表
        let invite = invite.trim();
        if !invite.starts_with(kdf::INVITE_PREFIX) {
            deny("bad invite format", &key, ip, self);
            return Err(LoginError::Denied("邀请码无效或已被使用".into()));
        }
        let now = db::now_secs();
        let found = self.db.find_invite_by_token(invite).map_err(internal)?;
        let Some(invite_row) = found.filter(|row| row.is_open(now)) else {
            // 不存在 / 已用过 / 已撤销 / 已过期 —— 对外都是同一句话
            deny("invite rejected", &key, ip, self);
            return Err(LoginError::Denied("邀请码无效或已被使用".into()));
        };

        let hash = kdf::hash_password(password).map_err(internal)?;
        let invite_id = invite_row.id;
        let user_id = self
            .db
            .register_with_invite(invite_id, username, &hash)
            .map_err(|e| {
                // 建号失败（用户名撞车 / 码被并发抢走）都算注册失败，按失败计数
                deny(&format!("register rejected: {e:#}"), &key, ip, self);
                LoginError::Denied(format!("{e}"))
            })?;

        self.rate_clear(&key);
        let mut bundle = self
            .issue_session(user_id, username, "user", false, user_agent)
            .map_err(internal)?;
        let _ = self.db.touch_last_login(user_id);
        let _ = self
            .db
            .audit_ip(Some(user_id), "register", &format!("invite_id={invite_id}"), ip);
        user::remove_initial_password_file();
        // 注册即登录：角色固定 user（永不自助成为 admin）
        bundle.user.role = "user".into();
        Ok(bundle)
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
        // 用户自设的密码 ⇒ 处置（删除）初始密码文件（2026-10-02 统一到 retire）。
        // ⚠️ 别再在这里手写 remove_initial_password_file —— 「改密入口必须处置该文件」
        // 这条规矩只该有一个落点（user::retire_initial_password_file）。
        user::retire_initial_password_file(None);
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

/// 是否信任反向代理转发的 `X-Forwarded-*` 头（`yekitrim server --trust-proxy` 打开）。
///
/// **默认关，因为这两个头是客户端可以随意伪造的**：
/// - 伪造 `X-Forwarded-Proto: https` 会让会话 cookie 被标成 `Secure`（于是反代后反而存不下 cookie）；
/// - 伪造 `X-Forwarded-For` 可以绕过按 IP 的限流（每个请求换一个假 IP）。
///
/// 只有部署者明确知道"我前面就是一台可信反代"时才该打开。见 [`resolve_client_ip`]。
static TRUST_PROXY: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// 设置是否信任代理头（`server::run` 启动时按 `--trust-proxy` 调一次）。
pub fn set_trust_proxy(on: bool) {
    TRUST_PROXY.store(on, std::sync::atomic::Ordering::Relaxed);
}

/// 当前是否信任代理头。
pub fn trust_proxy() -> bool {
    TRUST_PROXY.load(std::sync::atomic::Ordering::Relaxed)
}

/// 取客户端 IP。
///
/// - 默认（`trust_proxy = false`）：只认 socket 对端（`ConnectInfo`）；
/// - `trust_proxy = true`：改用 `X-Forwarded-For` 的**最后一段**。
///
/// ⚠️ 为什么是**最后**一段：自己那台反代是往列表**末尾追加**真实对端的，前面的值全是
/// 客户端自己填的。取第一段等于把"我是谁"交给请求方声明，限流立刻形同虚设。
pub fn resolve_client_ip(
    connect: Option<std::net::SocketAddr>,
    headers: &HeaderMap,
    trust_proxy: bool,
) -> Option<String> {
    if trust_proxy {
        if let Some(last) = headers
            .get("x-forwarded-for")
            .and_then(|v| v.to_str().ok())
            .and_then(|raw| {
                raw.split(',')
                    .map(|s| s.trim())
                    .filter(|s| !s.is_empty())
                    .next_back()
            })
        {
            return Some(last.to_string());
        }
    }
    connect.map(|a| a.ip().to_string())
}

/// 客户端 IP，取自 `ConnectInfo`（**默认不信 `X-Forwarded-For`**：反代后它不是可信输入，
/// 见 §3.5.1；只有 `--trust-proxy` 打开时才改认可信反代追加的那一段，
/// 见 [`resolve_client_ip`]）。`oneshot` 单测里没有 ConnectInfo 时为 `None`，不影响功能。
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
        let connect = parts
            .extensions
            .get::<axum::extract::ConnectInfo<std::net::SocketAddr>>()
            .map(|c| c.0);
        Ok(ClientIp(resolve_client_ip(
            connect,
            &parts.headers,
            trust_proxy(),
        )))
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

/// 自助注册的用户名校验（`yekitrim user add` 由管理员建号，不受此限）。
/// 只挡明显有问题的：空白/控制字符、过长。
pub fn validate_username(username: &str) -> Result<()> {
    let n = username.chars().count();
    if n < 2 || n > 32 {
        anyhow::bail!("用户名长度需在 2~32 个字符之间");
    }
    if username.chars().any(|c| c.is_whitespace() || c.is_control()) {
        anyhow::bail!("用户名不能包含空格或控制字符");
    }
    Ok(())
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

/// 请求是否走了 TLS 终止的代理。
///
/// ⚠️ 只在 `--trust-proxy` 打开时才采信 `X-Forwarded-Proto` —— 这个头任何人都能自己加上，
/// 不设门控会让"是不是 HTTPS"和"cookie 要不要标 `Secure`"由请求方说了算
/// （表现为反代后面反而登录不上：cookie 被标了 `Secure` 却是 http 连接）。
pub fn tls_terminated(headers: &HeaderMap) -> bool {
    tls_terminated_with(headers, trust_proxy())
}

/// [`tls_terminated`] 的纯函数版本（判定逻辑与全局开关分开，便于确定性地测）。
fn tls_terminated_with(headers: &HeaderMap, trust_proxy: bool) -> bool {
    if !trust_proxy {
        return false;
    }
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

/// `require_password_changed`：仍在使用初始密码时，除改密/登出/自查外一律 403。
///
/// **以前这里只有前端一条提示横幅**：服务端不拦，任何直接打 API 的客户端（脚本、curl、
/// 别人写的前端）都能拿这个会话做所有事。而初始密码会同时写进日志文件与
/// `initial-admin-password.txt`，等于把凭证摊在磁盘上 —— 所以它必须是一条**服务端**规则，
/// 与启动时"仍用初始密码则拒绝监听非 loopback"（`lib.rs`）配套。
///
/// ⚠️ 必须挂在 `require_user` **内层**（`require_user` 先跑并写入 [`CurrentUser`]）；
/// 反过来挂就看不到用户，等于不生效。挂载顺序见 `api::router` 的注释。
pub async fn require_password_changed(req: Request, next: Next) -> Response {
    let must = req
        .extensions()
        .get::<CurrentUser>()
        .map(|u| u.must_change_password)
        .unwrap_or(false);
    if !must {
        return next.run(req).await;
    }
    // 放行列表：改密本身、登出、以及"我是谁"（前端靠它显示横幅并弹改密框）。
    // 注意这里拿到的是 `nest("/api")` **去前缀后**的路径。
    const ALLOWED: [&str; 3] = ["/me", "/auth/password", "/auth/logout"];
    if ALLOWED.contains(&req.uri().path()) {
        return next.run(req).await;
    }
    json_error(
        StatusCode::FORBIDDEN,
        "仍在使用初始密码，请先修改密码（页面右上角「修改密码」，或 PUT /api/auth/password）",
    )
}

/// `require_user`：会话 cookie → 账号快照 → CSRF（写请求）→ 注入 [`CurrentUser`]。
pub async fn require_user(State(state): State<Arc<AppState>>, mut req: Request, next: Next) -> Response {
    match authenticate(&state, &mut req) {
        Ok(()) => {
            let actor = actor_of(&req);
            let mut resp = next.run(req).await;
            if let Some(a) = actor {
                resp.extensions_mut().insert(a);
            }
            resp
        }
        Err(resp) => resp,
    }
}

/// `require_admin`：先做完整的会话/CSRF 校验，再要求 `role == "admin"`。
///
/// 用于 `/api/admin/*`（平台配置、模型、录播姬 token、全平台任务等，§5.3）。
pub async fn require_admin(State(state): State<Arc<AppState>>, mut req: Request, next: Next) -> Response {
    match authenticate(&state, &mut req) {
        Ok(()) => {
            let actor = actor_of(&req);
            let is_admin = req
                .extensions()
                .get::<CurrentUser>()
                .map(|u| u.is_admin())
                .unwrap_or(false);
            let mut resp = if is_admin {
                next.run(req).await
            } else {
                json_error(StatusCode::FORBIDDEN, "仅管理员可访问该接口")
            };
            // 403 也带上操作者：访问日志里能看出"是谁被拒了"
            if let Some(a) = actor {
                resp.extensions_mut().insert(a);
            }
            resp
        }
        Err(resp) => resp,
    }
}

/// 已认证调用者 → 响应扩展（见 [`Actor`]）。
fn actor_of(req: &Request) -> Option<Actor> {
    req.extensions()
        .get::<CurrentUser>()
        .map(|u| Actor { username: u.username.clone() })
}

/// 共用认证：校验会话与 CSRF，并把 [`CurrentUser`] 注入请求扩展。
/// **关停控制端点的令牌**（`YEKITRIM_SHUTDOWN_TOKEN`）。
///
/// ⚠️ 为什么是**环境变量**而不是命令行参数：本仓有一条现成的不变式 —— 秘密不进进程列表
/// （`--admin-password-file` 就是为它存在的）。跨进程的"托盘退出"要一个只有托盘和子进程
/// 知道的凭证，env 是这里最合适的载体（托盘 spawn 时 `Command::env(..)` 逐个设，不落盘、不进列表）。
pub const SHUTDOWN_TOKEN_ENV: &str = "YEKITRIM_SHUTDOWN_TOKEN";

/// 控制端点要的请求头（自定义头 ⇒ 跨站请求会先被 CORS 预检挡掉，多一层）。
pub const SHUTDOWN_TOKEN_HEADER: &str = "x-yekitrim-shutdown-token";

/// 这个请求能不能关停服务。**没设 env 就一律 false**（端点当不存在，不给探测口）。
pub(crate) fn shutdown_token_ok(headers: &axum::http::HeaderMap) -> bool {
    match std::env::var(SHUTDOWN_TOKEN_ENV) {
        Ok(v) if !v.trim().is_empty() => token_ok_for(&v, headers),
        _ => false,
    }
}

/// 令牌比较的**纯函数**部分（把 env 摘出去，好在单测里直接打）。
///
/// ⚠️ 用 `constant_time_eq`，别退回 `==`：`==` 会在第一个不同的字节就返回，
/// 时间差能被逐字节试出来。
fn token_ok_for(want: &str, headers: &axum::http::HeaderMap) -> bool {
    let got = headers
        .get(SHUTDOWN_TOKEN_HEADER)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    !got.is_empty() && !want.is_empty() && constant_time_eq(got.as_bytes(), want.as_bytes())
}

/// 主机名是不是本机。接受 `127.0.0.1` / `localhost` / `[::1]`（可带端口）。
fn is_loopback_host(hostport: &str) -> bool {
    let host = hostport.rsplit_once(':').map(|(h, _)| h).unwrap_or(hostport);
    let host = host.trim_start_matches('[').trim_end_matches(']');
    matches!(host, "127.0.0.1" | "localhost" | "::1") || host == "0:0:0:0:0:0:0:1"
}

/// 本地免登录的两道闸（见 `authenticate` 里的说明）。任何一条不过就 403。
fn guard_local_request(req: &Request) -> Result<(), Response> {
    let h = req
        .headers()
        .get(axum::http::header::HOST)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if !is_loopback_host(h) {
        return Err(json_error(
            StatusCode::FORBIDDEN,
            "本地免登录只接受通过本机地址访问（Host 非 loopback）",
        ));
    }
    if !is_safe_method(req.method()) {
        if let Some(site) = req
            .headers()
            .get("sec-fetch-site")
            .and_then(|v| v.to_str().ok())
        {
            if site.eq_ignore_ascii_case("cross-site") {
                return Err(json_error(
                    StatusCode::FORBIDDEN,
                    "本地免登录拒绝跨站请求（Sec-Fetch-Site: cross-site）",
                ));
            }
        }
        if let Some(origin) = req
            .headers()
            .get(axum::http::header::ORIGIN)
            .and_then(|v| v.to_str().ok())
        {
            // `Origin: null`（file://、沙箱 iframe）一律拒 —— 那是拿不到 host 的
            let hostpart = origin
                .strip_prefix("http://")
                .or_else(|| origin.strip_prefix("https://"))
                .unwrap_or("");
            if hostpart.is_empty() || !is_loopback_host(hostpart) {
                return Err(json_error(
                    StatusCode::FORBIDDEN,
                    "本地免登录拒绝非同源来源（Origin 非 loopback）",
                ));
            }
        }
    }
    Ok(())
}

/// 本地免登录的**哨兵**会话 id：它不在 `sessions` 表里。
///
/// ⚠️ 因此依赖"真实会话"的功能在本地模式下不可用（最典型是改密码 —— `change_password`
/// 会拿它去查会话）。本地模式本来就没有"登录"这件事，这个取舍是有意的。
pub const LOCAL_SESSION_ID: &str = "local-no-auth";

fn authenticate(state: &Arc<AppState>, req: &mut Request) -> Result<(), Response> {
    // ---- 本地免登录（`--local-no-auth`，只有 loopback 才可能开启）----
    // 注入本机管理员，不查会话、不查 CSRF；但**必须先过两道闸**：
    //   ① Host 是 loopback 主机名 ⇒ 防 DNS rebinding
    //   ② 写请求的 Origin / Sec-Fetch-Site 同源 ⇒ 防别的网页跨站打进来
    // 少了任何一道，"免登录"就等于"任何网页都能指挥这台机器上的 YekiTrim"。
    if let Some(la) = &state.local_admin {
        guard_local_request(req)?;
        req.extensions_mut().insert(CurrentUser {
            id: la.id,
            username: la.username.clone(),
            role: la.role.clone(),
            must_change_password: false,
            session_id: LOCAL_SESSION_ID.to_string(),
        });
        return Ok(());
    }
    let auth = state.auth.clone();
    let Some(sid) = cookie_value(req.headers(), kdf::COOKIE_SID) else {
        return Err(json_error(StatusCode::UNAUTHORIZED, "未登录，请先登录"));
    };
    let session = match auth.session(&sid) {
        Ok(Some(s)) => s,
        Ok(None) => return Err(json_error(StatusCode::UNAUTHORIZED, "登录已过期，请重新登录")),
        Err(e) => {
            tracing::error!("读取会话失败: {e:#}");
            return Err(json_error(StatusCode::INTERNAL_SERVER_ERROR, "读取会话失败"));
        }
    };
    if session.expires_at <= db::now_secs() {
        let _ = auth.db().delete_session(&sid);
        return Err(json_error(StatusCode::UNAUTHORIZED, "登录已过期，请重新登录"));
    }
    if session.disabled {
        return Err(json_error(StatusCode::FORBIDDEN, "账号已被禁用，请联系管理员"));
    }
    if !is_safe_method(req.method()) {
        let token = req
            .headers()
            .get(kdf::CSRF_HEADER)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        if token.is_empty() {
            return Err(json_error(StatusCode::FORBIDDEN, "缺少 CSRF 令牌（请通过页面操作）"));
        }
        if !constant_time_eq(token.as_bytes(), session.csrf.as_bytes()) {
            return Err(json_error(StatusCode::FORBIDDEN, "CSRF 令牌不匹配，请刷新页面后重试"));
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
    Ok(())
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

    fn mk_user(id: i64, role: &str, session_id: &str) -> CurrentUser {
        CurrentUser {
            id,
            username: format!("u{id}"),
            role: role.to_string(),
            must_change_password: false,
            session_id: session_id.to_string(),
        }
    }

    /// 「两种形态共用一套业务」的**唯一分叉点**：本地形态 → `Owner::Local`，
    /// 登录形态 → `Owner::User(uid)`。
    #[test]
    fn owner_fork_is_local_session_versus_real_login() {
        assert_eq!(
            owner_of(&mk_user(1, "admin", LOCAL_SESSION_ID)),
            yekitrim_core::owner::Owner::Local,
            "本地免登录（托盘）必须是 Local，数据才落在程序目录下"
        );
        assert_eq!(
            owner_of(&mk_user(42, "user", "some-real-session")),
            yekitrim_core::owner::Owner::User(42)
        );
    }

    /// ⚠️ 判据是**会话形态**、不是 **role**。
    ///
    /// 本地模式注入的合成身份碰巧是 admin；但一个**在多用户模式下真登录的管理员**，
    /// 数据仍必须落在自己的 `users/<uid>/` 里 —— 否则他就把自己的东西写进全局，
    /// 与普通用户看到的混在一起。
    #[test]
    fn an_admin_logged_in_normally_still_gets_his_own_tree() {
        let real_admin = mk_user(1, "admin", "real-session-cookie");
        assert_eq!(
            owner_of(&real_admin),
            yekitrim_core::owner::Owner::User(1),
            "多用户模式下登录的管理员，数据要落在自己的树下、不是全局"
        );
    }

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
        let h = headers_with(&[("cookie", "a=1; yekitrim_sid=abc; yekitrim_csrf=xyz")]);
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
        assert!(cookies[0].starts_with("yekitrim_sid=sid;"), "{}", cookies[0]);
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

    /// 代理头只有在 `--trust-proxy` 打开时才被采信；默认一律不认。
    #[test]
    fn tls_terminated_requires_trust_proxy() {
        let https = headers_with(&[("x-forwarded-proto", "https")]);
        // 判定逻辑本身（纯函数）：https / 列表取首段 / http / 无头
        assert!(tls_terminated_with(&https, true));
        assert!(tls_terminated_with(&headers_with(&[("x-forwarded-proto", "https, http")]), true));
        assert!(!tls_terminated_with(&headers_with(&[("x-forwarded-proto", "http")]), true));
        assert!(!tls_terminated_with(&HeaderMap::new(), true));
        // 门控：关了就是关了，哪怕头里写着 https
        assert!(!tls_terminated_with(&https, false));
        // 全局开关默认必须是关的（公开接口上的伪造头不能影响 cookie 的 Secure 判定）
        assert!(!trust_proxy());
        assert!(!tls_terminated(&https));
    }

    /// 客户端 IP：默认只认 socket 对端，`trust_proxy` 打开才认 `X-Forwarded-For` 的**最后**一段。
    #[test]
    fn client_ip_ignores_forwarded_header_by_default() {
        let connect: Option<std::net::SocketAddr> = Some("10.0.0.9:1234".parse().unwrap());
        let xff = headers_with(&[("x-forwarded-for", "1.2.3.4, 5.6.7.8")]);
        // 默认：只认对端，伪造头无效（否则限流会被逐请求换 IP 绕过）
        assert_eq!(resolve_client_ip(connect, &xff, false).as_deref(), Some("10.0.0.9"));
        // 显式信任代理：取最后一段（自己那台反代追加的真实对端），不是客户端自填的第一段
        assert_eq!(resolve_client_ip(connect, &xff, true).as_deref(), Some("5.6.7.8"));
        // 信任代理但没有该头 → 退回对端
        assert_eq!(resolve_client_ip(connect, &HeaderMap::new(), true).as_deref(), Some("10.0.0.9"));
        // 没有 ConnectInfo（单测的 oneshot）→ None
        assert_eq!(resolve_client_ip(None, &HeaderMap::new(), false), None);
    }

    /// 本地免登录的两道闸：`Host` 必须 loopback（防 DNS rebinding），
    /// 写请求的 `Origin` / `Sec-Fetch-Site` 必须同源（防跨站）。
    ///
    /// ⚠️ 这条判据是"免登录"这个便利的**唯一**代价赔偿 —— 它一旦松掉，
    /// 本机上任何一个网页都能指挥这个服务（`is_loopback_host` 的 `localhost.` 之类变体尤其别放）。
    #[test]
    fn local_no_auth_guards_reject_foreign_host_and_cross_site() {
        use axum::body::Body;
        use axum::http::{Method, Request};

        let mk = |method: Method, hdrs: &[(&str, &str)]| {
            let mut b = Request::builder().method(method).uri("/api/projects");
            for (k, v) in hdrs {
                b = b.header(*k, *v);
            }
            b.body(Body::empty()).unwrap()
        };

        // ---- ① Host 防线 ----
        assert!(guard_local_request(&mk(Method::GET, &[("host", "127.0.0.1:26320")])).is_ok());
        assert!(guard_local_request(&mk(Method::GET, &[("host", "localhost:26320")])).is_ok());
        assert!(guard_local_request(&mk(Method::GET, &[("host", "[::1]:26320")])).is_ok());
        // 恶意域名（含"看起来像 localhost"的变体）一律拒
        for bad in ["evil.com", "evil.com:26320", "localhost.evil.com", "127.0.0.1.evil.com", ""] {
            assert!(
                guard_local_request(&mk(Method::GET, &[("host", bad)])).is_err(),
                "Host={bad:?} 不该放行"
            );
        }

        // ---- ② 跨站写请求 ----
        assert!(guard_local_request(&mk(
            Method::POST,
            &[("host", "127.0.0.1:26320"), ("origin", "http://127.0.0.1:26320")]
        ))
        .is_ok());
        assert!(guard_local_request(&mk(
            Method::POST,
            &[("host", "127.0.0.1:26320"), ("origin", "http://evil.com")]
        ))
        .is_err());
        // `Origin: null`（file:// / 沙箱 iframe）拿不到 host，一律拒
        assert!(guard_local_request(&mk(
            Method::POST,
            &[("host", "127.0.0.1:26320"), ("origin", "null")]
        ))
        .is_err());
        assert!(guard_local_request(&mk(
            Method::POST,
            &[("host", "127.0.0.1:26320"), ("sec-fetch-site", "cross-site")]
        ))
        .is_err());
        assert!(guard_local_request(&mk(
            Method::POST,
            &[("host", "127.0.0.1:26320"), ("sec-fetch-site", "same-origin")]
        ))
        .is_ok());
        // 读请求不看 Origin（同源 GET 本来就不带）—— 只靠 Host 兜
        assert!(guard_local_request(&mk(
            Method::GET,
            &[("host", "127.0.0.1:26320"), ("origin", "http://evil.com")]
        ))
        .is_ok());
    }

    /// ⚠️ **跨进程契约**：薄启动器（`yekitrim-tray`）**刻意不链接本 crate**（链接就会把
    /// 静态的 ASR 引擎一起拉回来，托盘从 0.49 MB 涨回 57 MB），所以下面这几个名字在两边
    /// **各写一份字面量**，过去只靠注释约定。改错的表现全是**静默降级**：
    /// 令牌名不一致 → 托盘退不掉服务（只能 taskkill）；`WebUI: http://` 那行变了 → 抓不到
    /// 实际地址，"打开网页"点不动。所以这里把它们读进来钉死。
    ///
    /// ⚠️ `include_str!` 在**编译期**只嵌一个字符串，**不引入依赖边**（`cli` 仍不许
    /// `use yekitrim_server::*`，见 `scripts/check_module_boundaries.py` 的 `cli 只用 server 的入口`）。
    #[test]
    fn tray_and_server_agree_on_the_cross_process_contract() {
        const TRAY: &str = include_str!("../../yekitrim-cli/src/tray.rs");
        const SERVER_LIB: &str = include_str!("lib.rs");
        const ROUTES: &str = include_str!("api/routes.rs");

        /// 从源码文本里取出 `const NAME: &str = "…";` 的那个字面量。
        fn lit(src: &str, name: &str) -> String {
            let needle = format!("const {name}: &str = \"");
            let i = src
                .find(&needle)
                .unwrap_or_else(|| panic!("在托盘源码里找不到 `{name}` —— 两边契约断了"));
            let rest = &src[i + needle.len()..];
            let j = rest.find('"').expect("字面量没有收尾引号");
            rest[..j].to_string()
        }

        // ① 关停令牌：env 名与头名（`run_with` 与 `api::shutdown` 都靠这两个键对齐）
        assert_eq!(lit(TRAY, "SHUTDOWN_TOKEN_ENV"), SHUTDOWN_TOKEN_ENV);
        assert_eq!(lit(TRAY, "SHUTDOWN_TOKEN_HEADER"), SHUTDOWN_TOKEN_HEADER);

        // ② 托盘抓"实际监听地址"的那行标记，必须是服务端**启动日志的前缀**
        let marker = lit(TRAY, "URL_MARKER");
        assert!(
            SERVER_LIB.contains(&format!("info!(\"{marker}")),
            "服务端启动日志不再以 `{marker}` 开头 —— 托盘会抓不到地址（'打开网页'点不动）"
        );

        // ③ 托盘 POST 的路径，必须就是路由里声明的那条
        //    （路由挂在 `nest("/api", …)` 里，所以全路径 = `/api` + `/shutdown`）
        assert!(
            TRAY.contains("POST /api/shutdown "),
            "托盘不再 POST 到 `/api/shutdown`"
        );
        assert!(
            ROUTES.contains(".route(\"/shutdown\", post(shutdown))"),
            "路由里没有 `/shutdown` —— 托盘敲的那个端点不存在了"
        );
    }

    /// 关停控制端点的令牌比较。⚠️ 这些是"跨进程优雅关停"唯一的门，闸门松一格就是
    /// 本机上任何进程都能把服务关掉。
    #[test]
    fn shutdown_token_gate_is_exact_and_empty_safe() {
        let mk = |v: Option<&str>| {
            let mut h = axum::http::HeaderMap::new();
            if let Some(v) = v {
                h.insert(SHUTDOWN_TOKEN_HEADER, v.parse().unwrap());
            }
            h
        };
        // 对上了
        assert!(token_ok_for("s3cret", &mk(Some("s3cret"))));
        // 差别一个字符 / 长度不同 / 大小写不同 → 都不行
        assert!(!token_ok_for("s3cret", &mk(Some("s3cres"))));
        assert!(!token_ok_for("s3cret", &mk(Some("s3cre"))));
        assert!(!token_ok_for("s3cret", &mk(Some("S3CRET"))));
        // 头缺失 / 空头
        assert!(!token_ok_for("s3cret", &mk(None)));
        assert!(!token_ok_for("s3cret", &mk(Some(""))));
        // 令牌本身为空时**不能**被空头蒙过去
        assert!(!token_ok_for("", &mk(Some(""))));
        assert!(!token_ok_for("", &mk(Some("x"))));
    }
}
