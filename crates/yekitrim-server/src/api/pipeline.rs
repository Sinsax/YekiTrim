//! `api::pipeline`：管线任务、任务队列查询与 SSE 事件流。

use std::sync::Arc;
use std::time::Duration;

use axum::{
    extract::{Extension, Path as AxumPath, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response, sse::{Event, KeepAlive, Sse}},
    Json,
};
use futures_util::stream::Stream;
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::wrappers::errors::BroadcastStreamRecvError;
use yekitrim_service::Workspace;
use futures_util::StreamExt;

use crate::model::AppState;
use crate::jobs::{JobClass, JobEvent, JobSpec};
use super::*;

// ===================== Pipeline =====================

pub(super) async fn run_pipeline_job(
    jobs: &Arc<crate::jobs::JobManager>,
    owner: i64,
    project: i64,
    config_file: std::path::PathBuf,
    kind: &str,
    stages: Vec<String>,
    dry_run: bool,
    resume: bool,
) -> ApiResult {
    // 资源域：项目输出目录；转写连带全局模型目录、含剪切断时连带发布索引
    // + 该切的所有 `clip-NN`（含剪切断的管线会重切同名片段，不能与正在精修/渲染的并行）。
    let mut keys = crate::scope_key::pipeline_keys(project, &stages);
    if stages.iter().any(|s| s == "cut" || s == "process") {
        if let Ok(ws) = yekitrim_service::Workspace::load(Some(&config_file)) {
            let out = ws.output_dir();
            keys.extend(crate::scope_key::cut_stage_clip_keys(project, &out, &out.join("events.json")));
        }
    }
    let run_stages = stages.clone();
    // 档位按阶段判定：含转写/剪切/全流程 = 重活，纯提取/检测 = 略重（见 `JobClass::for_pipeline`）
    let class = JobClass::for_pipeline(&stages);
    let handle = jobs.create(owner, JobSpec::new(kind, "准备中", keys, class)).await;
    let id = handle.id.clone();
    let queued = handle.queued;
    jobs.attach_project(&id, project).await;
    let jobs = jobs.clone();
    let job_id = id.clone();
    let cancel = jobs.cancel_token(&id).await;
    // 管线（转写/检测/剪切）是整段同步代码：跑在专用任务线程上，不占 HTTP worker
    spawn_job(jobs.clone(), job_id.clone(), move || async move {
        if handle.wait_start().await.is_err() {
            jobs.complete(&job_id).await;
            return;
        }
        let result = crate::api::core::run_stages(config_file, run_stages, dry_run, resume, job_id.clone(), jobs.clone(), cancel).await;
        if let Err(e) = result {
            jobs.fail(&job_id, format!("{e:#}")).await;
            return;
        }
        jobs.complete(&job_id).await;
    });
    ok(serde_json::json!({ "ok": true, "id": id, "queued": queued }))
}

pub(super) async fn correct_run_impl(
    ws: &Workspace,
    jobs: &Arc<crate::jobs::JobManager>,
    owner: i64,
    project: i64,
) -> ApiResult {
    // 订正写 output/*（correct.checkpoint / video_llm.srt），与管线同域。
    // 略重档：全程在等 LLM 网络，不该跟渲染抢重活名额。
    let handle = jobs
        .create(
            owner,
            JobSpec::new(
                "correct",
                "LLM 订正运行中",
                vec![crate::scope_key::project_out(project)],
                JobClass::Medium,
            ),
        )
        .await;
    let id = handle.id.clone();
    let queued = handle.queued;
    jobs.attach_project(&id, project).await;
    let jobs_handle = jobs.clone();
    let cfg_file = ws.config_file.clone();
    let job_id = id.clone();
    spawn_job(jobs_handle.clone(), job_id.clone(), move || async move {
        if handle.wait_start().await.is_err() {
            jobs_handle.complete(&job_id).await;
            return;
        }
        // 取消令牌 → `ProgressReporter` → Agent 的窗口边界探针（订正是分钟级链路，
        // 该跟检测/精修一个口径；用户在任务卡上点停止要真的能停）。
        let token = jobs_handle.cancel_token(&job_id).await;
        let progress = super::core::progress_bridge(job_id.clone(), jobs_handle.clone(), token);
        let result = yekitrim_service::run_correct(&cfg_file, None, true, Some(progress)).await;
        if let Err(e) = result {
            jobs_handle.fail(&job_id, format!("{e:#}")).await;
        } else {
            jobs_handle.complete(&job_id).await;
        }
    });
    ok(serde_json::json!({ "ok": true, "id": id, "queued": queued }))
}

/// `GET /api/jobs`：只列**本人**任务；admin 带 `?all=1` 可看全平台（P5 管理页用）。
pub(super) async fn jobs_list(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<crate::auth::CurrentUser>,
    Query(q): Query<std::collections::HashMap<String, String>>,
) -> ApiResult {
    let all = user.is_admin()
        && q.get("all")
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(false);
    let list = state.jobs.list_scoped(user.id, user.is_admin(), all).await;
    ok(serde_json::to_value(list).unwrap_or_default())
}

pub(super) async fn job_get(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<crate::auth::CurrentUser>,
    AxumPath(id): AxumPath<String>,
) -> ApiResult {
    // 他人任务一律 404（不泄露存在性）
    match state.jobs.get_owned(&id, user.id, user.is_admin()).await {
        Some(j) => ok(serde_json::to_value(j).unwrap_or_default()),
        None => Err(err(StatusCode::NOT_FOUND, "任务不存在")),
    }
}

pub(super) fn sse_event(ev: &JobEvent) -> Event {
    Event::default()
        .event("message")
        .data(serde_json::to_string(ev).unwrap_or_else(|_| "{}".into()))
}

/// 终态事件：之后不会再有有意义的事件，单任务流可以主动关闭。
pub(super) fn is_terminal_event(ev: &JobEvent) -> bool {
    matches!(
        ev,
        JobEvent::Completed | JobEvent::Failed { .. } | JobEvent::Cancelled
    )
}

/// 把 broadcast 接收端映射成 SSE 事件流。
///
/// - `Lagged`：只是订阅者太慢丢了若干条，**绝不能**报成 `Failed`
///   （旧实现把错误映射为 `failed: 事件流中断`，前端会把运行中的任务判为失败）；
/// - 发送端全部退出时 broadcast 流自然结束（tokio-stream 0.1 只有 `Lagged` 一种错误），
///   流随之正常收尾。
pub(super) fn job_event_stream(
    rx: tokio::sync::broadcast::Receiver<JobEvent>,
) -> impl Stream<Item = (bool, Event)> + Send + 'static {
    BroadcastStream::new(rx).filter_map(|item| async move {
        match item {
            Ok(ev) => {
                let terminal = is_terminal_event(&ev);
                Some((terminal, sse_event(&ev)))
            }
            Err(BroadcastStreamRecvError::Lagged(n)) => Some((
                false,
                sse_event(&JobEvent::Log {
                    level: "warn".into(),
                    message: format!("事件流落后，已跳过 {n} 条事件"),
                }),
            )),
        }
    })
}

/// 全局流的版本：额外带 `owner_id`，供 `/api/stream` 按归属过滤。
pub(super) fn global_event_stream(
    rx: tokio::sync::broadcast::Receiver<crate::jobs::OwnedEvent>,
) -> impl Stream<Item = (i64, bool, Event)> + Send + 'static {
    BroadcastStream::new(rx).filter_map(|item| async move {
        match item {
            Ok(ev) => {
                let terminal = is_terminal_event(&ev.event);
                Some((ev.owner_id, terminal, sse_event(&ev.event)))
            }
            Err(BroadcastStreamRecvError::Lagged(n)) => Some((
                0, // 0 = 面向所有订阅者的提示（不属于任何用户）
                false,
                sse_event(&JobEvent::Log {
                    level: "warn".into(),
                    message: format!("事件流落后，已跳过 {n} 条事件"),
                }),
            )),
        }
    })
}

/// `/api/stream`：**只推本人事件**（服务端过滤，前端无感知）。
pub(super) async fn global_stream(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<crate::auth::CurrentUser>,
) -> Response {
    let rx = state.jobs.subscribe_all().await;
    let uid = user.id;
    let is_admin = user.is_admin();
    let stream: std::pin::Pin<Box<dyn Stream<Item = Result<Event, axum::Error>> + Send>> =
        Box::pin(global_event_stream(rx).filter_map(move |(owner, _, ev)| {
            let mine = is_admin || owner == uid;
            std::future::ready(mine.then_some(Ok(ev)))
        }));
    Sse::new(stream).keep_alive(KeepAlive::new().interval(Duration::from_secs(15))).into_response()
}

pub(super) async fn job_events(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<crate::auth::CurrentUser>,
    AxumPath(id): AxumPath<String>,
) -> Response {
    // 归属校验：别人的任务连 SSE 都订不到（404）
    if !state.jobs.owned_by(&id, user.id, user.is_admin()).await {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "ok": false, "error": "任务不存在" })),
        )
            .into_response();
    }
    let Some((history, rx)) = state.jobs.subscribe(&id).await else {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "ok": false, "error": "任务不存在" })),
        )
            .into_response();
    };
    // 先回放历史：快任务可能在 EventSource 建立之前就结束，
    // 只给实时流会让前端永远停在「运行中」。终态事件之后流即结束。
    let replayed =
        futures_util::stream::iter(history.into_iter().map(|ev| (is_terminal_event(&ev), sse_event(&ev))));
    let stream: std::pin::Pin<Box<dyn Stream<Item = Result<Event, axum::Error>> + Send>> =
        Box::pin(
            replayed
                .chain(job_event_stream(rx))
                .take_while(|(terminal, _)| std::future::ready(!*terminal))
                .map(|(_, ev)| Ok(ev)),
        );
    Sse::new(stream).keep_alive(KeepAlive::new().interval(Duration::from_secs(15))).into_response()
}

pub(super) async fn job_cancel(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<crate::auth::CurrentUser>,
    AxumPath(id): AxumPath<String>,
) -> ApiResult {
    // 只能取消自己的任务（他人任务 404）
    if !state.jobs.owned_by(&id, user.id, user.is_admin()).await {
        return Err(err(StatusCode::NOT_FOUND, "任务不存在"));
    }
    if state.jobs.cancel(&id).await {
        ok(serde_json::json!({ "ok": true }))
    } else {
        Err(err(StatusCode::BAD_REQUEST, "无法取消任务"))
    }
}


/// 去掉 ANSI 颜色转义（子进程 stdout 里带着 tracing 的着色）。
pub(super) fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            for c2 in chars.by_ref() {
                if c2.is_ascii_alphabetic() {
                    break;
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// 从子进程输出里取「关键词后面那一段」（取最后一条命中的行）。
///
/// 渲染/封面都是外部 CLI 进程，成功路径的 stdout 原本被丢掉 —— 而产物**同名不覆盖**，
/// 第二次产出会叫 `cover_1.jpg` / `vtuber-clip_1.mp4`，日志里不写清楚，用户会以为被覆盖了。
pub(super) fn subprocess_note(raw: &str, needle: &str) -> Option<String> {
    let line = raw.lines().rev().find(|l| l.contains(needle))?;
    let plain = strip_ansi(line);
    let idx = plain.find(needle)? + needle.len();
    let mut tail = plain[idx..].trim();
    // `FINISHED: <路径> (12.3MB, 45s)` → 只留路径
    if tail.ends_with(')') {
        if let Some((head, _)) = tail.rsplit_once(" (") {
            tail = head.trim();
        }
    }
    if tail.is_empty() { None } else { Some(tail.to_string()) }
}

pub(super) fn latest_numbered_file(dir: &std::path::Path, stem: &str, ext: &str) -> Option<String> {
    let mut best: Option<(u32, String)> = None;
    let mut consider = |n: u32, name: String| {
        if best.as_ref().map(|(bn, _)| n > *bn).unwrap_or(true) {
            best = Some((n, name));
        }
    };
    let base = format!("{stem}.{ext}");
    if dir.join(&base).exists() {
        consider(0, base);
    }
    if let Ok(rd) = std::fs::read_dir(dir) {
        let prefix = format!("{stem}_");
        let suffix = format!(".{ext}");
        for entry in rd.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if !name.starts_with(&prefix) || !name.ends_with(&suffix) {
                continue;
            }
            // ⚠️ 用 strip_prefix/strip_suffix，别用算术切片：`cover_.jpg`（前后缀之间为空）
            // 会让 `name.len() - suffix.len() < prefix.len()` → `&name[6..5]` 直接 panic。
            // （任务体的 panic 现在会被 `spawn_job` 接住，但那次任务就失败了 —— 这里本来就该不 panic。）
            let Some(mid) = name
                .strip_prefix(&prefix)
                .and_then(|rest| rest.strip_suffix(&suffix))
            else {
                continue;
            };
            if let Ok(n) = mid.parse::<u32>() {
                consider(n, name);
            }
        }
    }
    best.map(|(_, name)| name)
}







#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::test_support::*;
    use crate::jobs::{JobClass, JobSpec};

    use axum::body::Body;


/// 资源域被占用时，新任务**排队**而不是 409（旧版是"已有任务在运行，请等待"）。
/// 这里验的是 handler 那一侧：资源键推导、`queued` 回传、以及取消排队任务不发车。
#[tokio::test]
async fn pipeline_job_queues_behind_conflicting_task() {
    let t = test_app().await;
    // 占住项目输出域（管线 / 剪切的资源键）
    let busy = t
        .state
        .jobs
        .create(
            1,
            JobSpec::new(
                "stage_detect",
                "检测",
                vec![crate::scope_key::project_out(1)],
                JobClass::Medium,
            ),
        )
        .await;
    assert!(!busy.queued);

    let res = t
        .call(
            t.req("POST", "/project-data/pipeline?project=1")
                .header(axum::http::header::CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"stage":"detect"}"#))
                .unwrap(),
        )
        .await;
    assert_eq!(res.status(), StatusCode::OK, "冲突时应排队而不是 409");
    let j = body_json(res).await;
    assert_eq!(j["queued"], true, "{j}");
    let id = j["id"].as_str().unwrap().to_string();
    let detail = t.state.jobs.get(&id).await.unwrap();
    assert_eq!(detail.status, "queued");
    assert!(detail.resources.contains(&crate::scope_key::project_out(1)), "{detail:?}");

    // 排队中被取消：收到 Err 直接收工（不跑管线、不发车）
    assert!(t.state.jobs.cancel(&id).await);
    t.state.jobs.complete(&busy.id).await;
    assert_eq!(t.state.jobs.get(&id).await.unwrap().status, "cancelled");
}

/// 渲染/封面子进程的日志提取：成功分支原本把 stdout 丢掉，用户看不到产出叫 `_1` / `_2`
/// （同名不覆盖），只好从 `FINISHED:` / `Cover saved:` 那行里把实际路径捞出来记进任务日志。
#[test]
fn subprocess_note_extracts_written_path() {
    let render = "\u{1b}[32m2026-09-14T17:44:03\u{1b}[0m \u{1b}[32mINFO\u{1b}[0m yekitrim: \
         FINISHED: F:\\video\\clip-23\\vtuber-clip_1.mp4 (96.2MB, 143s)";
    assert_eq!(
        subprocess_note(render, "FINISHED:").as_deref(),
        Some("F:\\video\\clip-23\\vtuber-clip_1.mp4")
    );
    let cover = "INFO yekitrim_polish::cover:   \u{2713} Cover saved: F:\\video\\clip-23\\cover_2.jpg";
    assert_eq!(
        subprocess_note(cover, "Cover saved:").as_deref(),
        Some("F:\\video\\clip-23\\cover_2.jpg")
    );
    // 没有那行（老版本 CLI / 失败路径）时返回 None，调用方就不写日志
    assert_eq!(subprocess_note("boom", "FINISHED:"), None);
    assert_eq!(strip_ansi("\u{1b}[1mhi\u{1b}[0m"), "hi");
}

/// `cover_.jpg` 这类"前缀与后缀之间为空"的文件名曾经直接 panic：
/// 旧写法是 `&name[prefix.len()..name.len()-suffix.len()]`，`cover_.jpg` 时算出 `&name[6..5]`，
/// `start > end` 越界 panic。HTTP 可达（`/project-data/clips` 一类的扫描路径），
/// HTTP 可达（`/project-data/clips` 一类的扫描路径），当时 release 下还会连带终止整个 server 进程。
#[test]
fn latest_numbered_file_ignores_empty_middle() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("cover_.jpg"), b"x").unwrap();
    std::fs::write(dir.path().join("cover_3.jpg"), b"x").unwrap();
    std::fs::write(dir.path().join("cover_12.jpg"), b"x").unwrap();
    // 空中间名跳过、按编号取最大
    assert_eq!(
        latest_numbered_file(dir.path(), "cover", "jpg").as_deref(),
        Some("cover_12.jpg")
    );
    // 只有空中间名时返回 None（不 panic、不误判成 0 号）
    let only_empty = tempfile::tempdir().unwrap();
    std::fs::write(only_empty.path().join("cover_.jpg"), b"x").unwrap();
    assert_eq!(latest_numbered_file(only_empty.path(), "cover", "jpg"), None);
    // 无编号后缀的基础名仍然认（老行为不能回退）
    let plain = tempfile::tempdir().unwrap();
    std::fs::write(plain.path().join("cover.jpg"), b"x").unwrap();
    assert_eq!(
        latest_numbered_file(plain.path(), "cover", "jpg").as_deref(),
        Some("cover.jpg")
    );
    // 非数字中间名（cover_abc.jpg）同样跳过
    let nonnum = tempfile::tempdir().unwrap();
    std::fs::write(nonnum.path().join("cover_abc.jpg"), b"x").unwrap();
    assert_eq!(latest_numbered_file(nonnum.path(), "cover", "jpg"), None);
}
}
