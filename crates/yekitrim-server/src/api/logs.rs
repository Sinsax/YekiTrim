//! `api::logs`：运行日志面板与 LLM 用量（仅 admin）。

use std::sync::Arc;
use std::time::Duration;

use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::{IntoResponse, Response, sse::{Event, KeepAlive, Sse}},
};
use futures_util::stream::Stream;
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::wrappers::errors::BroadcastStreamRecvError;
use yekitrim_service::logbus;
use futures_util::StreamExt;

use crate::model::AppState;
use super::*;

// ===================== 运行日志（仅 admin） =====================

#[derive(Debug, serde::Deserialize)]
pub(super) struct LogsQuery {
    /// 游标：只要 `seq > after` 的记录（SSE 断线重连补齐用）
    after: Option<u64>,
    limit: Option<usize>,
}

/// `GET /api/admin/logs`：拉最近的服务端日志（面板首次打开 / 重连补齐）。
///
/// 服务端日志含其他用户的工程路径与任务输出，因此**仅 admin**（与 `/api/admin/*` 一致）。
pub(super) async fn admin_logs(Query(q): Query<LogsQuery>) -> ApiResult {
    let limit = q.limit.unwrap_or(200).clamp(1, 500);
    let bus = logbus::bus();
    let (records, latest) = bus.snapshot(q.after.unwrap_or(0), limit);
    ok(serde_json::json!({ "records": records, "latest": latest }))
}

#[derive(serde::Deserialize)]
pub(super) struct UsageQuery {
    days: Option<i64>,
}

/// `GET /api/admin/usage?days=30`：LLM 用量汇总（总量 + 按天/链路/项目/模型）。
///
/// **仅 admin**：记录里带各用户的项目路径，与 `/api/admin/logs` 同一个理由。
/// 采集点在 `yekitrim_llm::LlmClient::send`，见 `yekitrim_service::usage`。
pub(super) async fn admin_usage(State(state): State<Arc<AppState>>, Query(q): Query<UsageQuery>) -> ApiResult {
    let days = q.days.unwrap_or(30);
    let db = state.auth.db();
    let summary = yekitrim_service::usage::summarize(db, days)
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("读取用量失败: {e:#}")))?;
    ok(serde_json::json!(summary))
}

/// `GET /api/admin/logs/events?after=<seq>`：实时日志流（SSE）。
///
/// 先回放环形缓冲里 `seq > after` 的（补上建立连接前的那几秒），再接实时。
pub(super) async fn admin_logs_stream(Query(q): Query<LogsQuery>) -> Response {
    let after = q.after.unwrap_or(0);
    let (history, rx) = logbus::bus().subscribe();
    let replayed = futures_util::stream::iter(
        history
            .into_iter()
            .filter(move |r| r.seq > after)
            .map(|r| log_sse(&r)),
    );
    let stream: std::pin::Pin<Box<dyn Stream<Item = Result<Event, axum::Error>> + Send>> =
        Box::pin(replayed.chain(log_live_stream(rx)).map(Ok));
    Sse::new(stream)
        .keep_alive(KeepAlive::new().interval(Duration::from_secs(15)))
        .into_response()
}

pub(super) fn log_sse(rec: &logbus::LogRecord) -> Event {
    Event::default()
        .event("message")
        .data(serde_json::to_string(rec).unwrap_or_else(|_| "{}".into()))
}

/// 实时日志流。`Lagged` = 订阅者太慢丢了若干条，**不能**报成错误
/// （与任务事件流同理），合成一条 warn 让面板自己显示"跳过 N 条"。
pub(super) fn log_live_stream(
    rx: tokio::sync::broadcast::Receiver<logbus::LogRecord>,
) -> impl Stream<Item = Event> + Send + 'static {
    BroadcastStream::new(rx).filter_map(|item| async move {
        match item {
            Ok(rec) => Some(log_sse(&rec)),
            Err(BroadcastStreamRecvError::Lagged(n)) => {
                let (_, latest) = logbus::bus().snapshot(0, 1);
                Some(log_sse(&logbus::LogRecord {
                    seq: latest,
                    time: chrono::Local::now().format("%H:%M:%S%.3f").to_string(),
                    level: "warn".into(),
                    target: "http".into(),
                    text: format!("日志流落后，已跳过 {n} 条（可刷新面板重新拉取）"),
                }))
            }
        }
    })
}







#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::test_support::*;

    use axum::Router;
    use axum::body::Body;
    use axum::http::Request as HttpRequest;
    use tower::util::ServiceExt;
/// 访问日志：**每次前端交互都留痕**（方法/路径/状态/耗时/操作者），
/// 错误响应还要把服务端的 `error` 文案带出来 —— 这正是"静默消失"的正面修复。
#[tokio::test]
async fn access_log_records_requests_with_actor_and_error_text() {
    use tracing_subscriber::layer::SubscriberExt;
    let bus = logbus::bus();
    let before = bus.snapshot(0, 1).1;
    // 线程局部订阅者：只有装上 LogBusLayer，tracing 事件才会进总线
    // （测试进程不会跑 main 的 setup_tracing；guard 出作用域即还原）
    let _sub = tracing::subscriber::set_default(
        tracing_subscriber::registry()
            .with(logbus::LogBusLayer::new(bus.clone())),
    );

    let t = test_app().await;
    // 1) 已登录的正常请求：要能看出是谁在操作
    let res = t.call(t.req("GET", "/me").body(Body::empty()).unwrap()).await;
    assert_eq!(res.status(), StatusCode::OK);
    // 2) 未注册接口：JSON 404 的 error 文案必须进日志
    let probe = "/__access_log_probe_7f3a";
    let res = t
        .call(HttpRequest::builder().uri(probe).body(Body::empty()).unwrap())
        .await;
    assert_eq!(res.status(), StatusCode::NOT_FOUND);
    // 3) 未登录请求：操作者记成 `-`
    let res = t
        .call(HttpRequest::builder().uri("/me").body(Body::empty()).unwrap())
        .await;
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);

    let all = bus
        .snapshot(before, usize::MAX)
        .0
        .into_iter()
        .map(|r| format!("[{}] {}", r.level, r.text))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        all.contains("GET /api/me → 200") && all.contains("user=admin"),
        "缺已登录请求的记录：\n{all}"
    );
    assert!(
        all.contains(probe) && all.contains("未知接口"),
        "404 记录里应带服务端 error 文案：\n{all}"
    );
    assert!(
        all.contains("GET /api/me → 401") && all.contains("user=-"),
        "未登录请求应记 user=-：\n{all}"
    );
}

/// 生产是 `nest("/api", api::router(…))`：访问日志里的路径必须带 `/api` 前缀
/// （单测直接打 router 时靠补前缀，两条路径都得对）。
#[tokio::test]
async fn access_log_keeps_api_prefix_when_nested() {
    use tracing_subscriber::layer::SubscriberExt;
    let bus = logbus::bus();
    let before = bus.snapshot(0, 1).1;
    let _sub = tracing::subscriber::set_default(
        tracing_subscriber::registry()
            .with(logbus::LogBusLayer::new(bus.clone())),
    );
    let t = test_app().await;
    let app = Router::new().nest("/api", t.app.clone());
    let res = app
        .oneshot(
            HttpRequest::builder()
                .uri("/api/me")
                .header(axum::http::header::COOKIE, t.cookie())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    let all = bus
        .snapshot(before, usize::MAX)
        .0
        .into_iter()
        .map(|r| r.text)
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        all.contains("GET /api/me → 200") && all.contains("user=admin"),
        "嵌套下路径要带 /api：\n{all}"
    );
}

/// `/api/admin/logs`：仅 admin，形状与 `webui/src/stores/log.js` 对齐。
#[tokio::test]
async fn admin_logs_is_admin_only_and_returns_records() {
    use tracing_subscriber::layer::SubscriberExt;
    let bus = logbus::bus();
    let _sub = tracing::subscriber::set_default(
        tracing_subscriber::registry()
            .with(logbus::LogBusLayer::new(bus.clone())),
    );
    let t = test_app().await;
    // 先制造一条日志（这次请求本身）
    let res = t.call(t.req("GET", "/me").body(Body::empty()).unwrap()).await;
    assert_eq!(res.status(), StatusCode::OK);

    let res = t
        .call(t.req("GET", "/admin/logs?limit=20").body(Body::empty()).unwrap())
        .await;
    assert_eq!(res.status(), StatusCode::OK);
    let v = body_json(res).await;
    let latest = v["latest"].as_u64().unwrap_or(0);
    assert!(latest > 0, "应已有日志：{v}");
    let recs = v["records"].as_array().cloned().unwrap_or_default();
    assert!(!recs.is_empty(), "应回放出刚才那次请求：{v}");
    let last = recs.last().unwrap();
    for k in ["seq", "time", "level", "target", "text"] {
        assert!(last.get(k).is_some(), "记录缺字段 {k}：{last}");
    }

    // 游标语义：`after=<最新>` 只给更新的记录（前端断线补齐靠它）
    let res = t
        .call(t.req("GET", &format!("/admin/logs?after={latest}")).body(Body::empty()).unwrap())
        .await;
    assert_eq!(res.status(), StatusCode::OK);
    let v2 = body_json(res).await;
    assert!(v2["records"]
        .as_array()
        .unwrap()
        .iter()
        .all(|r| r["seq"].as_u64().unwrap_or(0) > latest));

    // 普通用户一律 403（日志含其他用户的路径与任务输出）
    let hash = yekitrim_service::auth::hash_password("pass-12345678").unwrap();
    t.auth.db().create_user("plain_logs", &hash, "user", false, 0).unwrap();
    let b = t
        .auth
        .login("plain_logs", "pass-12345678", Some("10.9.9.9"), None)
        .unwrap();
    let res = t
        .app
        .clone()
        .oneshot(
            HttpRequest::builder()
                .uri("/admin/logs")
                .header(
                    axum::http::header::COOKIE,
                    format!("{}={}", yekitrim_service::auth::COOKIE_SID, b.session_id),
                )
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::FORBIDDEN, "普通用户不得读取服务端日志");
}

/// `/api/admin/logs/events`：SSE 先回放环形缓冲里 `seq > after` 的，再续实时。
///
/// 面板刚打开的那几秒就靠这段回放（否则只能看到连接之后的新日志）。
#[tokio::test]
async fn admin_logs_stream_replays_history() {
    use tracing_subscriber::layer::SubscriberExt;
    let bus = logbus::bus();
    let _sub = tracing::subscriber::set_default(
        tracing_subscriber::registry()
            .with(logbus::LogBusLayer::new(bus.clone())),
    );
    let t = test_app().await;
    let cursor = bus.snapshot(0, 1).1;
    bus.push("info", "test", "SSE 回放探针 abc123".into());

    let res = t
        .call(t.req("GET", &format!("/admin/logs/events?after={cursor}")).body(Body::empty()).unwrap())
        .await;
    assert_eq!(res.status(), StatusCode::OK);
    let ct = res
        .headers()
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    assert!(ct.contains("text/event-stream"), "content-type = {ct}");

    // SSE 是无限流：收集若干帧（并行测试也会往同一条总线上塞记录，所以是"找"不是"等第一帧"）
    let mut body = res.into_body().into_data_stream();
    let mut seen = String::new();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    while std::time::Instant::now() < deadline && !seen.contains("SSE 回放探针 abc123") {
        match tokio::time::timeout(std::time::Duration::from_millis(500), body.next()).await {
            Ok(Some(Ok(chunk))) => seen.push_str(&String::from_utf8_lossy(&chunk)),
            _ => break,
        }
    }
    assert!(seen.contains("SSE 回放探针 abc123"), "应回放历史记录：{seen}");
    assert!(
        seen.contains("\"seq\"") && seen.contains("\"level\""),
        "帧应是 LogRecord JSON：{seen}"
    );
}
}
