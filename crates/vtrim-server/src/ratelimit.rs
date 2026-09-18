//! 请求限流：内存态固定窗口计数器 + 全站中间件。
//!
//! **为什么需要**：除登录/注册外，服务端此前对**任何**接口都没有限流。而它能读写项目目录、
//! 起 ffmpeg / headless Chrome、跑 ASR，每个后台任务还占一个 OS 线程（`api::spawn_job`）。
//! `docs/job-queue.md` §4 里"前台任务都是人点出来的，频率有天然上限"这条前提，在**公网多租户**
//! 部署下不成立 —— 脚本化的客户端可以每秒打几千次。
//!
//! 这里只做**粗粒度**的那一层（按客户端 IP 的通用额度）；真正针对"起任务"的节流在
//! 源头做：`JobManager::create` 的建任务频率与活动任务数上限（无论从哪个接口进来都挡得住，
//! 也不怕以后新加接口忘了挂限流）。
//!
//! 键取**客户端 IP**（`ConnectInfo` 的对端地址；只有 `--trust-proxy` 打开时才改认
//! `X-Forwarded-For` 的最后一段，见 `auth::resolve_client_ip`）。

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use axum::extract::{ConnectInfo, Request, State};
use axum::http::StatusCode;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::Json;

use super::model::AppState;

/// 计数表最多留多少个 key。
///
/// 超过这个数先清掉窗口已过期的；清完**还**超就拒绝新 key（fail closed）。
/// 不这样的话，"用不停变化的 key（伪造源地址）打请求"本身就能把这张表撑爆内存。
const MAX_KEYS: usize = 16_384;

/// 全站每 IP 的默认额度。
///
/// ⚠️ **必须装得下一次页面加载的突发**：前端进入工作台会连打几十个 `/api/*`
/// （实测有 1 秒 52 次的记录），所以这个值不是按"人点按钮的频率"设的。
/// 静态资源不走这里（不在 `/api` 下），否则一次首屏就能把额度用掉。
const GLOBAL_LIMIT: u32 = 600;
const GLOBAL_WINDOW_SECS: u64 = 60;

/// 一个固定窗口的计数。
#[derive(Debug)]
struct Window {
    count: u32,
    start: Instant,
}

/// 固定窗口限流器（key → 计数）。
#[derive(Debug)]
pub struct RateLimiter {
    limit: u32,
    window: Duration,
    hits: Mutex<HashMap<String, Window>>,
}

impl RateLimiter {
    pub fn new(limit: u32, window_secs: u64) -> Self {
        Self {
            limit,
            window: Duration::from_secs(window_secs),
            hits: Mutex::new(HashMap::new()),
        }
    }

    /// 记一次访问。`Err(等待秒数)` = 超限。
    pub fn check(&self, key: &str) -> Result<(), u64> {
        let mut map = self.hits.lock().unwrap_or_else(|e| e.into_inner());
        let now = Instant::now();
        if let Some(w) = map.get_mut(key) {
            if now.duration_since(w.start) >= self.window {
                *w = Window { count: 1, start: now };
                return Ok(());
            }
            w.count += 1;
            if w.count > self.limit {
                return Err((self.window - now.duration_since(w.start)).as_secs() + 1);
            }
            return Ok(());
        }
        // 新 key：先给表瘦身。清不掉又已经满员就 fail closed 拒绝（宁可误拒也不吃光内存）
        if map.len() >= MAX_KEYS {
            map.retain(|_, w| now.duration_since(w.start) < self.window);
            if map.len() >= MAX_KEYS {
                tracing::warn!(
                    "限流计数表已满（{} 个 key）且窗口内没有可清理项，拒绝新来源",
                    map.len()
                );
                return Err(self.window.as_secs());
            }
        }
        map.insert(key.to_string(), Window { count: 1, start: now });
        Ok(())
    }

    /// 当前计数表大小（只被单测用来断言"表不会无界增长"）。
    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.hits.lock().unwrap_or_else(|e| e.into_inner()).len()
    }
}

/// 全站请求限流器（进程级单例 —— 与 `api::probe_cache` 同一个模式）。
fn global_limiter() -> &'static RateLimiter {
    static L: OnceLock<RateLimiter> = OnceLock::new();
    L.get_or_init(|| RateLimiter::new(GLOBAL_LIMIT, GLOBAL_WINDOW_SECS))
}

/// 入库 webhook 专用额度：**未认证入口**，而且每次校验失败都要跑一次 scrypt
/// （N=2^15，约 32 MiB 内存 + 几十毫秒 CPU，见 `ingest::webhook`）。
/// 通用额度（600/分钟）对这个单次成本来说太宽了，所以单开一条更紧的。
const WEBHOOK_LIMIT: u32 = 30;
const WEBHOOK_WINDOW_SECS: u64 = 60;

/// webhook 的限流检查（在 handler 里、跑 scrypt **之前**调）。`Err(等待秒数)` = 超限。
pub fn check_webhook(ip: &str) -> Result<(), u64> {
    static L: OnceLock<RateLimiter> = OnceLock::new();
    let l = L.get_or_init(|| RateLimiter::new(WEBHOOK_LIMIT, WEBHOOK_WINDOW_SECS));
    l.check(&format!("webhook|{ip}"))
}

/// 上次记"正在限流"警告的时刻。
///
/// 被刷的时候每个请求都打一行日志，等于把日志文件也一起打满（这正是限流要防的后果之一）。
/// 所以警告按 `WARN_EVERY` 节流 —— 攻击期间仍然能从日志里看出"我在被限流"。
static LAST_WARN: OnceLock<Mutex<Instant>> = OnceLock::new();
const WARN_EVERY: Duration = Duration::from_secs(30);

fn warn_throttled(key: &str, wait: u64) {
    let m = LAST_WARN.get_or_init(|| Mutex::new(Instant::now()));
    let mut last = m.lock().unwrap_or_else(|e| e.into_inner());
    if last.elapsed() < WARN_EVERY {
        return;
    }
    *last = Instant::now();
    tracing::warn!("请求过于频繁，已限流（key={key}，需等待约 {wait}s）");
}

/// 从 `ConnectInfo`（+ 可选的 `X-Forwarded-For`）取出限流用的 key。
fn limit_key(req: &Request) -> Option<String> {
    let connect = req
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|c| c.0);
    super::auth::resolve_client_ip(connect, req.headers(), super::auth::trust_proxy())
}

/// 全站请求限流中间件（挂在 `/api` 路由的**最外层**）。
///
/// 位置很关键，两个理由：
/// 1. 要能覆盖**未认证**的入口（登录/注册/入库 webhook）—— 挂在 `require_user` 里面就漏了这些；
/// 2. 必须在 `access_log` **外面**：被限流的请求不该再各写一行访问日志（那正是攻击想造的后果）。
pub async fn limit_requests(
    State(_state): State<std::sync::Arc<AppState>>,
    req: Request,
    next: Next,
) -> Response {
    let Some(key) = limit_key(&req) else {
        // 拿不到对端地址（单测的 oneshot 调用没有 ConnectInfo）→ 不限流，别把测试与本地绑死
        return next.run(req).await;
    };
    if let Err(wait) = global_limiter().check(&key) {
        warn_throttled(&key, wait);
        return (
            StatusCode::TOO_MANY_REQUESTS,
            [(axum::http::header::RETRY_AFTER, wait.to_string())],
            Json(serde_json::json!({
                "ok": false,
                "error": format!("请求过于频繁，请 {wait} 秒后再试"),
            })),
        )
            .into_response();
    }
    next.run(req).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rate_limiter_allows_then_blocks_then_recovers() {
        let l = RateLimiter::new(3, 1);
        assert!(l.check("a").is_ok());
        assert!(l.check("a").is_ok());
        assert!(l.check("a").is_ok());
        let wait = l.check("a").unwrap_err();
        assert!(wait >= 1, "应给出等待秒数：{wait}");
        // 另一个 key 不受影响
        assert!(l.check("b").is_ok());
        // 窗口过去后恢复
        std::thread::sleep(Duration::from_millis(1100));
        assert!(l.check("a").is_ok());
    }

    /// key 数量不会无界增长：塞满之后清窗口、仍满就拒绝**新** key，且老 key 照常计。
    #[test]
    fn rate_limiter_bounds_key_table() {
        let l = RateLimiter::new(1000, 60);
        for i in 0..MAX_KEYS {
            assert!(l.check(&format!("k{i}")).is_ok(), "第 {i} 个 key 不该被拒");
        }
        assert_eq!(l.len(), MAX_KEYS);
        // 表已满且窗口都没过期 → 新 key 被拒（fail closed），内存不再增长
        assert!(l.check("brand-new").is_err());
        assert_eq!(l.len(), MAX_KEYS);
        // 已存在的 key 仍然能用（只有"新来源"被挡）
        assert!(l.check("k0").is_ok());
    }
}
