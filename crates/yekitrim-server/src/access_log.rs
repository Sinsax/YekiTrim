//! `/api/*` 的访问日志：**每次前端交互都留痕**。
//!
//! 为什么需要它：handler 的 `err(...)` 与几十处内联 `{"ok":false,...}` 从不打日志，
//! 前端又有大量 `.catch(() => [])` —— 两边一叠加，一次失败就"静默消失"了，
//! 只剩页面上一片空白。这里在响应侧统一记录：
//! 方法 / 路径 / 状态 / 耗时 / 操作者，并在状态 ≥400 时把**响应体里的
//! `error` 文案**一并记下（那才是排障要看的东西）。
//!
//! 由 `api::router` 挂在最外层（`.layer`），覆盖全部 `/api/*` 与 JSON 404 fallback。

use std::time::Instant;

use axum::body::Body;
use axum::extract::{OriginalUri, Request};
use axum::middleware::Next;
use axum::response::Response;

use super::auth::Actor;

/// 慢请求阈值：高频接口超过这个耗时**照样记 info**。
///
/// 高频接口成功路径本来是 debug（不然 3 秒一次的轮询会把日志刷满），但 3 秒一次的
/// `/api/jobs` 一旦变慢就正好是最难查的一类故障 —— 前端那次请求会把 `useJobs` 的去重
/// `inflight` 占住，任务列表与所有按作用域门控的按钮一起冻住，而日志里**一条都没有**
/// （实测排查"队列加载不出来 + 按钮一直灰"时，日志里完全看不到 `/api/jobs`）。
/// 慢的那几次必须留痕。
const SLOW_MS: u128 = 1000;

/// 高频接口（3 秒轮询 / 视频 Range / SSE）：成功路径降到 debug，免得把日志冲掉；
/// 出错时照常 warn/error，慢过 [`SLOW_MS`] 也照常 info —— 排障关心的正是那些。
/// 想看逐条访问日志：`RUST_LOG=yekitrim=debug`。
fn is_noisy(path: &str) -> bool {
    path.starts_with("/api/media/")
        || path == "/api/health"
        || path == "/api/stream"
        || path == "/api/jobs"
        || path == "/api/admin/logs/events"
}

/// 把路径里的一次性凭证抹掉再落日志。
///
/// 入库 webhook 的 token 是**路径段**（录播姬只能填 URL，加不了自定义头），原样记下来
/// 等于把凭证抄进日志文件与设置页的「运行日志」面板 —— 那两处都可能被备份、分享或被
/// 别的方式读到。这里只保留路由前缀，token 段一律打成 `***`。
fn redact_path(path: &str) -> String {
    const PREFIX: &str = "/api/ingest/bililive/";
    let Some(rest) = path.strip_prefix(PREFIX) else {
        return path.to_string();
    };
    let token = rest.split(['/', '?']).next().unwrap_or("");
    if token.is_empty() {
        return path.to_string();
    }
    path.replacen(token, "***", 1)
}

pub async fn access_log(req: Request, next: Next) -> Response {
    let method = req.method().clone();
    // 路径统一成用户实际请求的 `/api/...`：
    // - 生产：`nest("/api")` 之后 `uri()` 已去掉前缀，`OriginalUri` 是完整路径；
    // - 单测：直接打 `api::router()`（没有 nest），`OriginalUri`/`uri()` 只到 `/me`，
    //   补回前缀，避免同一件事在两种场景下记成两条不同形状的日志。
    let raw = req
        .extensions()
        .get::<OriginalUri>()
        .map(|u| u.0.to_string())
        .unwrap_or_else(|| req.uri().to_string());
    let path = if raw.starts_with("/api/") || raw == "/api" {
        raw
    } else {
        format!("/api{raw}")
    };
    let noisy = is_noisy(path.split('?').next().unwrap_or(path.as_str()));

    let started = Instant::now();
    let mut resp = next.run(req).await;
    let ms = started.elapsed().as_millis();
    let status = resp.status();

    // 只在错误分支读 body（服务端的错误文案写在那里）。非 JSON / 超 64KB 一律原样放行，
    // 绝不吞响应；正常响应（含 SSE、视频、缩略图）一个字节都不动。
    let mut detail = String::new();
    if status.as_u16() >= 400 {
        let is_json = resp
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .map(|v| v.contains("json"))
            .unwrap_or(false);
        if is_json {
            let (parts, body) = resp.into_parts();
            match axum::body::to_bytes(body, 64 * 1024).await {
                Ok(bytes) => {
                    if let Ok(v) = serde_json::from_slice::<serde_json::Value>(&bytes) {
                        if let Some(e) = v.get("error").and_then(|e| e.as_str()) {
                            detail = e.to_string();
                        }
                    }
                    resp = Response::from_parts(parts, Body::from(bytes));
                }
                Err(_) => resp = Response::from_parts(parts, Body::empty()),
            }
        }
    }

    let actor = resp
        .extensions()
        .get::<Actor>()
        .map(|a| a.username.clone())
        .unwrap_or_else(|| "-".to_string());

    let mut summary = format!("{method} {} → {status} ({ms}ms)", redact_path(&path));
    if !detail.is_empty() {
        summary.push_str(" · ");
        summary.push_str(&detail);
    }

    match status.as_u16() {
        500..=599 => tracing::error!(target: "yekitrim_cli::http", user = %actor, "{}", summary),
        400..=499 => tracing::warn!(target: "yekitrim_cli::http", user = %actor, "{}", summary),
        _ if noisy && ms < SLOW_MS => tracing::debug!(target: "yekitrim_cli::http", user = %actor, "{}", summary),
        _ => tracing::info!(target: "yekitrim_cli::http", user = %actor, "{}", summary),
    }
    resp
}

#[cfg(test)]
mod tests {
    use super::*;

    /// webhook token 不能出现在日志里（它是路径段）。
    #[test]
    fn webhook_token_is_redacted_in_logs() {
        assert_eq!(
            redact_path("/api/ingest/bililive/yekitrim_ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789ab"),
            "/api/ingest/bililive/***"
        );
        // 带 query / 多段路径同样只吃掉 token 段
        assert_eq!(
            redact_path("/api/ingest/bililive/yekitrim_secret123?x=1"),
            "/api/ingest/bililive/***?x=1"
        );
        // 其它路径一个字符都不动
        assert_eq!(redact_path("/api/projects?x=1"), "/api/projects?x=1");
        assert_eq!(redact_path("/api/ingest/bililive/"), "/api/ingest/bililive/");
        assert_eq!(redact_path("/api/ingest/records"), "/api/ingest/records");
        // 登录接口的凭证在 body 里，不在路径上 —— 这里只管路径
        assert_eq!(redact_path("/api/auth/login"), "/api/auth/login");
    }
}
