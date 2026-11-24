//! `api::project_data::events`：事件（events.json）的增删改、逐条/一键剪切与管线入口。

use std::path::Path;

use axum::extract::{Path as AxumPath, State};
use axum::http::StatusCode;

use axum::Json;
use yekitrim_service::Workspace;

use crate::model::AppState;
use crate::scope::Scope;

// 跨子模块 / 外部域：显式点名（不许 glob）
use axum::extract::Extension;
use super::clips::count_in_range;
use super::status::danmaku_starts;
use super::status::transcript_starts;

use crate::api::pipeline::{correct_run_impl, run_pipeline_job};
use crate::jobs::{JobClass, JobSpec};

use super::*;

/// `POST /api/project-data/events` 的请求体（手工建一条事件）。
///
/// ⚠️ 时间有**两条通道**且都还在用：新前端传 `start_sec`/`end_sec`，老调用方与 CLI 传
/// `start`/`end` —— 用 `alias` 保留，别删任何一条。`ranges`（多段剪）优先于单段起终点。
#[derive(serde::Deserialize)]
pub(super) struct CreateEventBody {
    /// 标题。**必填**（空串由 impl 回 400「标题不能为空」，与改造前同口径）。
    pub label: String,
    #[serde(default, alias = "start")]
    pub start_sec: Option<f64>,
    #[serde(default, alias = "end")]
    pub end_sec: Option<f64>,
    #[serde(default)]
    pub ranges: Vec<EventRangeBody>,
    #[serde(default)]
    pub event_type: Option<String>,
    #[serde(default)]
    pub summary: Option<String>,
}

/// `CreateEventBody.ranges` 的一条（多段剪的一段）。同样兼容 `start_sec`/`start` 两种写法。
#[derive(serde::Deserialize)]
pub(super) struct EventRangeBody {
    #[serde(default, alias = "start")]
    pub start_sec: Option<f64>,
    #[serde(default, alias = "end")]
    pub end_sec: Option<f64>,
}

pub(crate) async fn project_data_events_create(scope: Scope, Json(body): Json<serde_json::Value>) -> ApiResult {
    let ws = scope.ws;
    let body: CreateEventBody = parse_body(body)?;
    // events.json 是 read-modify-write：并发 PUT/POST 会丢事件
    let path = ws.output_dir().join("events.json");
    let _guard = yekitrim_core::filelock::lock_path(&path).await;
    blocking(move || data_events_create_impl(&ws, &body)).await
}

pub(crate) async fn project_data_events_put(scope: Scope, Json(body): Json<serde_json::Value>) -> ApiResult {
    let ws = scope.ws;
    let path = ws.output_dir().join("events.json");
    let _guard = yekitrim_core::filelock::lock_path(&path).await;
    blocking(move || data_events_put_impl(&ws, &body)).await
}

// 这两个入口在**建任务前**就要读 events.json / 逐个检查 clip.mp4 是否存在（全在网络盘上），
// 所以整段（含排队判定）下 blocking 池；任务体本身由 `spawn_job` 带走。
pub(crate) async fn project_data_event_cut(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<crate::auth::CurrentUser>,
    scope: Scope,
    AxumPath(rank): AxumPath<String>,
) -> ApiResult {
    let (ws, st, uid, pid) = (scope.ws, state.clone(), user.id, scope.project.id);
    blocking(move || block_on_here(event_cut_impl(&ws, &rank, &st.jobs, uid, pid))).await
}

pub(crate) async fn project_data_event_cut_all(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<crate::auth::CurrentUser>,
    scope: Scope,
    Json(body): Json<serde_json::Value>,
) -> ApiResult {
    let (ws, st, uid, pid) = (scope.ws, state.clone(), user.id, scope.project.id);
    blocking(move || block_on_here(event_cut_all_impl(&ws, &body, &st.jobs, uid, pid))).await
}

/// `POST /api/project-data/events/merge-splits` —— 把历史上被等分的
/// `xxx（1/2）`/`（2/2）` 合回一个事件（前端二次确认后调用）。
pub(crate) async fn project_data_events_merge_splits(scope: Scope, State(state): State<Arc<AppState>>) -> ApiResult {
    let ws = scope.ws;
    let path = ws.output_dir().join("events.json");
    // 同步写 events.json（没法排队）：有任务正在写同批产物时当场拒绝，别对撞
    if let Some(busy) = state
        .jobs
        .busy_with(&[crate::scope_key::project_out(scope.project.id)])
        .await
    {
        tracing::info!("拒绝在 {} 上合并拆分事件：{busy}", busy.resource);
        return Err(err(StatusCode::CONFLICT, format!("{busy}（正在读写切片产物），请稍后再试")));
    }
    let _guard = yekitrim_core::filelock::lock_path(&path).await;
    blocking(move || data_events_merge_splits_impl(&ws)).await
}

pub(crate) async fn project_data_correct(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<crate::auth::CurrentUser>,
    scope: Scope,
) -> ApiResult {
    correct_run_impl(&scope.ws, &state.jobs, user.id, scope.project.id).await
}

pub(crate) async fn project_data_pipeline(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<crate::auth::CurrentUser>,
    scope: Scope,
    Json(body): Json<serde_json::Value>,
) -> ApiResult {
    // 作用域由 `Scope` 解析（`?project=` / 过渡期 `?config=`）；
    // 旧前端还会在 body 里带 `config`，那只是冗余字段，不再作为作用域来源。
    let ws = scope.ws;
    // 配额预检（review B7/D14）：转写/检测/剪切的产物也占账号配额 ——
    // 已超配额的普通账号不再放进新任务（admin 不受限）。
    quota_gate(&state, &user, Some(scope.project.id)).await?;
    if let Some(stage) = body.get("stage").and_then(|v| v.as_str()) {
        // 转写**每个模式只跑一次**：ASR 产物按模式占槽（`[asr] mode` 只有 hybrid / single
        // 两种，见 `yekitrim_service::assets`），该模式已经出过产物就不再提供重跑入口。
        // 想换字幕走素材列表（`POST /api/project-data/assets`）里已有的那份。
        // ⚠️ 只拦 HTTP 这一侧：CLI `yekitrim transcribe` 直接调 `core_pipeline::run_stage`，
        // 仍然可以强制重跑（转写中途崩掉、只留了半份产物时它是逃生口）。
        if stage == "transcribe" {
            let cfg = ws.load_config().map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}")))?;
            let mode = cfg.asr.mode.trim().to_string();
            // 模式串空（配置被清掉）= 没选模式：退回老口径"有转写就不给重跑"
            let bound = if mode.is_empty() {
                ws.transcript_path(&cfg).is_some()
            } else {
                yekitrim_service::assets::slot_bound(&ws, &cfg, yekitrim_service::assets::AssetKind::Transcript, &mode)
            };
            if bound {
                return Err(err(
                    StatusCode::CONFLICT,
                    if mode.is_empty() {
                        "该工程的转写已完成，不能再重跑；要换字幕请用素材列表里的那份".to_string()
                    } else {
                        format!("「{mode}」模式的转写已完成，不能再重跑；要换字幕请用素材列表里的那份")
                    },
                ));
            }
        }
        let resume = body.get("resume").and_then(|v| v.as_str()).map(|s| s != "restart").unwrap_or(true);
        let stages: Vec<String> = match stage {
            "extract" => vec!["extract"],
            "transcribe" => vec!["transcribe"],
            "detect" => vec!["detect"],
            "cut" => vec!["cut"],
            "process" => vec!["extract","transcribe","detect","cut"],
            _ => return Err(err(StatusCode::BAD_REQUEST, "未知 stage")),
        }.into_iter().map(|s| s.to_string()).collect();
        run_pipeline_job(&state.jobs, user.id, scope.project.id, ws.config_file.clone(), &format!("stage_{}", stage), stages, false, resume).await
    } else {
        let stages = body.get("stages").and_then(|v| v.as_array()).map(|arr| {
            arr.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect::<Vec<_>>()
        }).unwrap_or_else(|| vec!["extract".into(),"transcribe".into(),"detect".into(),"cut".into()]);
        let dry = body.get("dry_run").and_then(|v| v.as_bool()).unwrap_or(false);
        let resume = body.get("resume").and_then(|v| v.as_str()).map(|s| s != "restart").unwrap_or(true);
        run_pipeline_job(&state.jobs, user.id, scope.project.id, ws.config_file.clone(), "pipeline", stages, dry, resume).await
    }
}

pub(super) fn data_events_impl(ws: &Workspace) -> ApiResult {
    let path = ws.output_dir().join("events.json");
    // 同 data_clips_impl：**还没检测过**是正常空态，不是错误 —— 返回空集合，
    // 让调用方渲染"还没有事件"的空态，而不是把整页判成加载失败。
    if !path.exists() {
        return ok(serde_json::json!({ "titles": {}, "events": [] }));
    }
    let raw = std::fs::read_to_string(&path).map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("读取失败: {e}")))?;
    let mut v: serde_json::Value = serde_json::from_str(&raw).unwrap_or(serde_json::Value::Null);
    // 事件区间内的弹幕量/字幕量（切片页排序用），文件缺失时为 0。
    // 只有存在未剪切事件时才解析源弹幕/字幕，避免纯浏览列表时反复解析大文件。
    let clips_dir = path.parent().map(|p| p.join("clips"));
    let has_uncut = v.get("events").and_then(|e| e.as_array()).map(|evs| evs.iter().any(|ev| {
        let dir = event_clip_dir_or_rank(ev);
        !clips_dir.as_ref().map(|c| c.join(dir).join("clip.mp4").exists()).unwrap_or(false)
    })).unwrap_or(false);
    let (dm_starts, sub_starts) = if has_uncut {
        (danmaku_starts(ws), transcript_starts(ws))
    } else {
        (Vec::new(), Vec::new())
    };
    if let Some(evs) = v.get_mut("events").and_then(|x| x.as_array_mut()) {
        for ev in evs {
            let start = ev.get("start").and_then(|s| s.as_object())
                .and_then(|o| o.get("h").and_then(|h| h.as_u64()))
                .map(|_| ev_time_secs(ev, "start")).unwrap_or(0.0);
            let end = ev_time_secs(ev, "end");
            ev["start_sec"] = serde_json::json!(start);
            ev["end_sec"] = serde_json::json!(end);
            let dir = event_clip_dir_or_rank(ev);
            let clip_dir = path.parent().map(|p| p.join("clips").join(dir)).unwrap_or_default();
            ev["cut_exists"] = serde_json::json!(clip_dir.join("clip.mp4").exists());
            ev["danmaku_count"] = serde_json::json!(count_in_range(&dm_starts, start, end));
            ev["subtitle_count"] = serde_json::json!(count_in_range(&sub_starts, start, end));
        }
    }
    ok(v)
}

/// 从 events.json 的一个事件里取实际片段目录名：优先 `clip_dir`，旧数据回退 `clip-{rank:02}`。
pub(super) fn event_clip_dir(v: &serde_json::Value) -> Option<String> {
    let dir = v
        .get("clip_dir")
        .and_then(|d| d.as_str())
        .map(str::trim)
        .filter(|d| !d.is_empty())?;
    if yekitrim_core::cutting::clip_index::is_safe_component(dir) {
        Some(dir.to_string())
    } else {
        None
    }
}

pub(super) fn event_clip_dir_or_rank(v: &serde_json::Value) -> String {
    if let Some(dir) = event_clip_dir(v) {
        return dir;
    }
    let rank = v.get("rank").and_then(|r| r.as_u64()).unwrap_or(0);
    format!("clip-{rank:02}")
}

pub(super) fn ev_time_secs(v: &serde_json::Value, key: &str) -> f64 {
    let o = match v.get(key).and_then(|x| x.as_object()) { Some(o)=>o, None=>return 0.0 };
    let h=o.get("h").and_then(|x|x.as_u64()).unwrap_or(0) as f64;
    let m=o.get("m").and_then(|x|x.as_u64()).unwrap_or(0) as f64;
    let s=o.get("s").and_then(|x|x.as_u64()).unwrap_or(0) as f64;
    let ms=o.get("ms").and_then(|x|x.as_u64()).unwrap_or(0) as f64;
    h*3600.0+m*60.0+s+ms/1000.0
}

/// 秒 → `events.json` 里的 `{h,m,s,ms}` 时间戳。
///
/// 用 `Timestamp::from_seconds`（按总毫秒计算）而不是手写 floor/round：
/// 后者在 `sec = 59.9999` 时会产出非法的 `ms = 1000`（SRT 会写成 `00:00:59,1000`）。
pub(super) fn sec_to_ts_value(sec: f64) -> serde_json::Value {
    let ts = yekitrim_core::types::Timestamp::from_seconds(sec.max(0.0));
    serde_json::json!({ "h": ts.h, "m": ts.m, "s": ts.s, "ms": ts.ms })
}

pub(super) fn data_events_create_impl(ws: &Workspace, body: &CreateEventBody) -> ApiResult {
    let path = ws.output_dir().join("events.json");
    if !path.exists() {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("创建目录失败: {e}")))?;
        }
        std::fs::write(&path, "{\"events\":[]}").map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("创建 events.json 失败: {e}")))?;
    }
    let label = body.label.trim().to_string();
    if label.is_empty() {
        return Err(err(StatusCode::BAD_REQUEST, "标题不能为空"));
    }
    // **多段入点/出点列表**（粗剪拓展；2026-10-06）：预处理页可以传
    // `ranges = [{start_sec, end_sec}, …]` —— 剪出来的是"分段剪、合并输出"的一条片段。
    // 传了就**以 ranges 为准**：`start/end` 拉成首段起、末段末（与 `Event::normalize_ranges` 同一条）。
    // ⚠️ 单段（1 条 ranges）与不传 ranges 完全等价 ⇒ 老的「标记入点/出点 → 切片」一个字节不变。
    let mut ranges: Vec<yekitrim_core::types::EventRange> = Vec::new();
    if !body.ranges.is_empty() {
        for r in &body.ranges {
            if let (Some(rs), Some(re)) = (r.start_sec, r.end_sec) {
                if re > rs {
                    ranges.push(yekitrim_core::types::EventRange::new(rs, re));
                }
            }
        }
        ranges.sort_by(|a, b| a.start.partial_cmp(&b.start).unwrap_or(std::cmp::Ordering::Equal));
        // ⚠️ **重叠/相接的段必须并掉**再落盘：手工标段时两行可以互相压住（项目 12 那条 4 段片段
        // 就是 `1625.7–1653.8` 里套着 `1632.0–1643.0`）—— 不去重 Cutter 会把同一段**剪两遍**
        // （`clip.mp4` 158.3s = 四段之和，其中 11s 重复），而 `duration` 也按"各段之和"多报一遍，
        // 界面上完全看不出来。与 `Event::normalize_ranges` 走**同一个函数**：只并
        // `next.start <= last.end`，**有内容的空隙照旧留着**（那是"分段剪、中间别的话题丢掉"的表达）。
        yekitrim_core::types::merge_overlapping_ranges(&mut ranges);
    }
    let (start_sec, end_sec) = if ranges.is_empty() {
        let start = body.start_sec.unwrap_or(0.0);
        let end = body.end_sec.unwrap_or(start + 10.0);
        (start, end)
    } else {
        (
            ranges.iter().map(|r| r.start).fold(f64::MAX, f64::min),
            ranges.iter().map(|r| r.end).fold(f64::MIN, f64::max),
        )
    };
    if end_sec <= start_sec {
        return Err(err(StatusCode::BAD_REQUEST, "结尾必须大于开头"));
    }
    let event_type = body.event_type.clone().unwrap_or_else(|| "other".to_string());
    let summary = body.summary.clone().unwrap_or_default();
    let raw = std::fs::read_to_string(&path).map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("读取失败: {e}")))?;
    let mut store: serde_json::Value = serde_json::from_str(&raw).unwrap_or_else(|_| serde_json::json!({ "events": [] }));
    if !store.get("events").and_then(|a| a.as_array()).is_some() {
        store["events"] = serde_json::json!([]);
    }
    let next_rank = store.get("events").and_then(|a| a.as_array()).map(|arr| {
        arr.iter().filter_map(|e| e.get("rank").and_then(|r| r.as_u64())).max().unwrap_or(0) + 1
    }).unwrap_or(1);
    // 手动事件也分配稳定 clip_dir：rank 只做热度名次，目录名用标题 slug，
    // 同时避开磁盘上的旧片段目录和其它事件已占用的 clip_dir。
    let mut taken: std::collections::BTreeSet<String> = yekitrim_core::cutting::clip_index::reconcile_index(&ws.output_dir())
        .clips
        .keys()
        .map(|d| d.to_ascii_lowercase())
        .collect();
    if let Some(arr) = store.get("events").and_then(|a| a.as_array()) {
        for e in arr {
            if let Some(dir) = event_clip_dir(e) {
                taken.insert(dir.to_ascii_lowercase());
            }
        }
    }
    let clip_dir = yekitrim_core::cutting::clip_index::make_clip_dir_name(&label, &taken, next_rank as u32);
    let event = serde_json::json!({
        "start": sec_to_ts_value(start_sec),
        "end": sec_to_ts_value(end_sec),
        "label": label,
        "summary": summary,
        "event_type": event_type,
        "source": "manual",
        "confidence": 1.0,
        "popularity": 0.0,
        "rank": next_rank,
        "clip_dir": clip_dir,
        "enabled": true,
    });
    // 多段才写 `ranges`（单段 = 老格式，多一个键会让老工具以为"有多段"）
    let mut event = event;
    if ranges.len() > 1 {
        event["ranges"] = serde_json::to_value(&ranges).unwrap_or(serde_json::Value::Null);
    }
    if let Some(arr) = store.get_mut("events").and_then(|a| a.as_array_mut()) {
        arr.push(event.clone());
        let mut titles = serde_json::Map::new();
        for target in arr.iter() {
            if let Some(l) = target.get("label").and_then(|x| x.as_str()) {
                titles.insert(l.to_string(), serde_json::json!(target.get("enabled").and_then(|e| e.as_bool()).unwrap_or(true)));
            }
        }
        store["titles"] = serde_json::Value::Object(titles);
    }
    std::fs::write(&path, serde_json::to_string_pretty(&store).unwrap_or_default()).map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("保存失败: {e}")))?;
    ok(serde_json::json!({ "ok": true, "event": event }))
}

pub(super) fn data_events_put_impl(ws: &Workspace, body: &serde_json::Value) -> ApiResult {
    let path = ws.output_dir().join("events.json");
    if !path.exists() { return Err(err(StatusCode::NOT_FOUND, "暂无 events.json")); }
    let events = body.get("events").cloned().unwrap_or_else(|| body.clone());
    let raw = std::fs::read_to_string(&path).map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("读取失败: {e}")))?;
    let mut store: serde_json::Value = serde_json::from_str(&raw).unwrap_or_else(|_| serde_json::json!({ "events": [] }));
    let mut titles = serde_json::Map::new();
    if let Some(arr) = store.get_mut("events").and_then(|a| a.as_array_mut()) {
        for target in arr.iter_mut() {
            if let Some(update) = events.as_array().and_then(|updates| {
                updates.iter().find(|u| u.get("rank").and_then(|r| r.as_u64()) == target.get("rank").and_then(|r| r.as_u64()))
            }) {
                if let Some(en) = update.get("enabled") { target["enabled"] = en.clone(); }
                if let Some(label) = update.get("label").and_then(|l| l.as_str()) { target["label"] = serde_json::json!(label); }
                if let Some(et) = update.get("event_type").and_then(|v| v.as_str()) { target["event_type"] = serde_json::json!(et); }
                if let Some(summary) = update.get("summary").and_then(|v| v.as_str()) { target["summary"] = serde_json::json!(summary); }
                if let Some(start) = update.get("start").and_then(|v| v.as_object()) { target["start"] = serde_json::json!(start); }
                if let Some(end) = update.get("end").and_then(|v| v.as_object()) { target["end"] = serde_json::json!(end); }
            }
            if let Some(label) = target.get("label").and_then(|l| l.as_str()) {
                titles.insert(label.to_string(), serde_json::json!(target.get("enabled").and_then(|e| e.as_bool()).unwrap_or(true)));
            }
        }
    }
    store["titles"] = serde_json::Value::Object(titles);
    std::fs::write(&path, serde_json::to_string_pretty(&store).unwrap_or_default()).map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("保存失败: {e}")))?;
    ok(serde_json::json!({ "ok": true }))
}

/// 把 events.json 里被**等分**出来的 `基名（i/n）` 合回一个事件（规则见
/// `yekitrim_core::detection::merge_split_events`）。只在真的合并了东西时写盘。
/// 注意：事件目录名是 `clips/clip-{rank:02}`，合并后组内其余 rank 消失，
/// 它们的旧目录不会被删（要用户自己删掉重切），合并后的事件 rank 沿用组内最热的那条。
pub(super) fn data_events_merge_splits_impl(ws: &Workspace) -> ApiResult {
    let path = ws.output_dir().join("events.json");
    if !path.exists() { return Err(err(StatusCode::NOT_FOUND, "暂无 events.json")); }
    let raw = std::fs::read_to_string(&path)
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("读取失败: {e}")))?;
    let mut store: serde_json::Value = serde_json::from_str(&raw)
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("解析 events.json 失败: {e}")))?;
    let arr = store.get("events").and_then(|v| v.as_array()).cloned().unwrap_or_default();
    let events: Vec<yekitrim_core::types::Event> = serde_json::from_value(serde_json::Value::Array(arr))
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("事件字段解析失败: {e}")))?;
    let before = events.len();
    let (merged, groups) = yekitrim_core::detection::merge_split_events(events);
    if groups == 0 {
        return ok(serde_json::json!({ "ok": true, "groups": 0, "before": before, "after": before }));
    }
    let mut titles = serde_json::Map::new();
    for e in &merged {
        titles.insert(e.label.clone(), serde_json::json!(e.enabled));
    }
    store["events"] = serde_json::to_value(&merged)
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("序列化失败: {e}")))?;
    store["titles"] = serde_json::Value::Object(titles);
    std::fs::write(&path, serde_json::to_string_pretty(&store).unwrap_or_default())
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("保存失败: {e}")))?;
    ok(serde_json::json!({ "ok": true, "groups": groups, "before": before, "after": merged.len() }))
}

pub(super) async fn event_cut_all_impl(
    ws: &Workspace,
    body: &serde_json::Value,
    jobs: &Arc<crate::jobs::JobManager>,
    owner: i64,
    project: i64,
) -> ApiResult {
    let ev_path = ws.output_dir().join("events.json");
    if !ev_path.exists() { return Err(err(StatusCode::NOT_FOUND, "暂无 events.json")); }
    let raw = tokio::task::spawn_blocking({ let __p = ev_path.clone(); move || std::fs::read_to_string(__p) }).await.unwrap_or_else(|_| Err(std::io::Error::other("读取任务崩了"))).map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("读取失败: {e}")))?;
    let store: serde_json::Value = serde_json::from_str(&raw).map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("解析失败: {e}")))?;
    let Some(all_events) = store.get("events").and_then(|a| a.as_array()) else {
        return Err(err(StatusCode::NOT_FOUND, "暂无事件"));
    };
    let only: Vec<u32> = body.get("ranks").and_then(|v| v.as_array())
        .map(|arr| arr.iter().filter_map(|x| x.as_u64().and_then(|n| u32::try_from(n).ok())).collect())
        .unwrap_or_default();
    let has_filter = !only.is_empty();
    let output = ws.output_dir();
    let mut events = Vec::new();
    for ev in all_events {
        let Some(rank) = ev.get("rank").and_then(|r| r.as_u64()).and_then(|r| u32::try_from(r).ok()) else {
            continue;
        };
        if has_filter && !only.contains(&rank) {
            continue;
        }
        // 未显式指定 ranks 时遵循事件的 enabled 开关（detection::load_events 也这么做），
        // 显式点了某个事件则按用户要求执行。
        if !has_filter && !ev.get("enabled").and_then(|e| e.as_bool()).unwrap_or(true) {
            continue;
        }
        let clip_dir = output.join("clips").join(event_clip_dir_or_rank(ev));
        if clip_dir.join("clip.mp4").exists() {
            continue;
        }
        events.push(ev.clone());
    }
    if events.is_empty() {
        return Err(err(StatusCode::BAD_REQUEST, "没有需要剪切的未剪切事件"));
    }

    // 资源域：out（读 events.json）+ idx（两个发布索引）。**不带 clip 键** ——
    // 全切只处理没有 clip.mp4 的目录，不会碰精修工程，所以刻意允许与精修并行。
    let keys = crate::scope_key::cut_keys(project, &output, None);
    let total = events.len() as u64;
    let handle = jobs
        .create(
            owner,
            JobSpec::new("cut_all", format!("一键全切 {} 个事件", total), keys, JobClass::Heavy),
        )
        .await;
    let id = handle.id.clone();
    let queued = handle.queued;
    jobs.attach_project(&id, project).await;
    let jobs_handle = jobs.clone();
    let cfg_file = ws.config_file.clone();
    let job_id = id.clone();
    let cancel = jobs.cancel_token(&id).await;
    // 任务体跑在专用线程上；每条片段的剪切再下 blocking 池（3 路真并行，谁也占不到 HTTP worker）
    spawn_job(jobs_handle.clone(), job_id.clone(), move || async move {
        if handle.wait_start().await.is_err() {
            jobs_handle.complete(&job_id).await;
            return;
        }
        jobs_handle.progress(&job_id, 0, total).await;
        // 一次全切只产生一个 cut_all 任务；内部最多 3 个 ffmpeg 并发，进度统一写回该任务。
        let sem = Arc::new(tokio::sync::Semaphore::new(3));
        let mut handles = Vec::new();
        let is_cancelled = || {
            cancel
                .as_ref()
                .map(|c| c.load(std::sync::atomic::Ordering::SeqCst))
                .unwrap_or(false)
        };
        for ev in events {
            // 取消在事件边界生效（进行中的 ffmpeg 不会被中断）
            if is_cancelled() {
                break;
            }
            let permit = match sem.clone().acquire_owned().await {
                Ok(p) => p,
                Err(_) => break,
            };
            let cfg_file = cfg_file.clone();
            handles.push(tokio::task::spawn_blocking(move || {
                let _permit = permit;
                let rank = ev.get("rank").and_then(|r| r.as_u64()).unwrap_or(0);
                // 阻塞线程里没有 runtime：给这段管线单独开一个 current-thread runtime
                (rank, block_on_here(run_single_cut(&cfg_file, ev)))
            }));
        }
        let mut failures = 0usize;
        // 被磁盘预检挡下的条数（含触发那条）：这不是"剪切失败"，是**没让剪** ——
        // 阶段预检（runner）对每条 run_single_cut 都会先查 `[cutting] min_free_gb`，
        // 一旦不达标整批就此打住，剩余条目留给"腾出空间后重跑"自动补剪。
        let mut blocked = 0usize;
        let mut done = 0u64;
        for handle in handles {
            match handle.await {
                Ok((_, Ok(()))) => {}
                Ok((rank, Err(e))) => {
                    if e.downcast_ref::<yekitrim_core::disk::FreeSpaceError>().is_some() {
                        blocked += 1;
                    } else {
                        failures += 1;
                        let _ = jobs_handle.log(&job_id, "error", format!("clip-{:02} 剪切失败: {e:#}", rank)).await;
                    }
                }
                Err(_) => failures += 1,
            }
            done += 1;
            jobs_handle.progress(&job_id, done, total).await;
        }
        if blocked > 0 {
            let ok = total.saturating_sub(failures as u64 + blocked as u64);
            jobs_handle
                .fail(
                    &job_id,
                    format!(
                        "磁盘空间不足，已停止剪切：成功 {ok} 条、失败 {failures} 条、{blocked} 条未处理；腾出空间后重新「一键全切」会自动补剪缺失的片段（只剪还没有 clip.mp4 的目录）"
                    ),
                )
                .await;
        } else if failures == 0 {
            jobs_handle.complete(&job_id).await;
        } else {
            jobs_handle.fail(&job_id, format!("{} 个事件剪切失败", failures)).await;
        }
    });
    ok(serde_json::json!({ "ok": true, "id": id, "queued": queued }))
}

pub(super) async fn run_single_cut(config_file: &Path, ev: serde_json::Value) -> anyhow::Result<()> {
    run_single_cut_event(config_file, serde_json::from_value(ev)?).await
}

/// 同上，但调用方已经拿着解析好的事件（单条重切走这条，省一次 JSON 往返）。
pub(super) async fn run_single_cut_event(config_file: &Path, ev: yekitrim_core::types::Event) -> anyhow::Result<()> {
    let cfg = yekitrim_core::config::load_config_at(config_file)?;
    let w = yekitrim_service::Workspace::load(Some(config_file))?;
    let input = cfg.project.input_file.as_ref().filter(|s| !s.is_empty()).map(|s| w.resolve(s))
        .ok_or_else(|| anyhow::anyhow!("未初始化 input_file"))?;
    let output = w.output_dir();
    let mut ctx = yekitrim_core::types::PipelineContext::new(input, output);
    ctx.events = vec![ev];
    let mut r = yekitrim_core::pipeline::PipelineRunner::new(cfg);
    r.add_stage(Box::new(yekitrim_core::cutting::Cutter));
    r.run(&mut ctx).await?;
    // **剪切收尾 = 补精修工程**（用户口径：剪切这个行为本身包含精修 init）。
    // 单条剪切与一键全切都走这里（`event_cut_all_impl` → `run_single_cut`），
    // 于是切片页的「精修」按钮、首页切片总览、`polish_ready` 判据进来时工程已经在了，
    // 不再有"发完 init 就跳转、页面先读到 polish.toml not found"的竞态。
    // 失败只 warn：剪切已经成功，进入精修时还有 `ensure_polish_project` 兜底。
    // 收尾进度传 None：单条收尾秒级（cut_all 的逐条进度由调用方循环统一报，
    // 不会在这里出现 >120s 的无事件窗口 —— spec §1 P2 口径核对过）。
    yekitrim_service::init_polish_for_events(&ctx.output_dir, &ctx.events, None).await;
    Ok(())
}

pub(super) async fn event_cut_impl(
    ws: &Workspace,
    rank: &str,
    jobs: &Arc<crate::jobs::JobManager>,
    owner: i64,
    project: i64,
) -> ApiResult {
    let ev_path = ws.output_dir().join("events.json");
    if !ev_path.exists() { return Err(err(StatusCode::NOT_FOUND, "暂无 events.json")); }
    let raw = tokio::task::spawn_blocking({ let __p = ev_path.clone(); move || std::fs::read_to_string(__p) }).await.unwrap_or_else(|_| Err(std::io::Error::other("读取任务崩了"))).map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("读取失败: {e}")))?;
    let store: serde_json::Value = serde_json::from_str(&raw).map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("解析失败: {e}")))?;
    let rank_u: u32 = rank.parse().map_err(|_| err(StatusCode::BAD_REQUEST, "rank 无效"))?;
    let event: serde_json::Value = store.get("events").and_then(|a| a.as_array()).and_then(|arr| arr.iter().find(|e| e.get("rank").and_then(|r| r.as_u64()).unwrap_or(0) as u32 == rank_u).cloned())
        .ok_or_else(|| err(StatusCode::NOT_FOUND, "事件不存在"))?;
    let clip_dir = event_clip_dir_or_rank(&event);
    let ev: yekitrim_core::types::Event = serde_json::from_value(event).map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("事件解析失败: {e}")))?;
    // 资源域含这个片段目录：单条重切会写 clip.mp4/clip.json，与同片段的精修互斥（排队）。
    let keys = crate::scope_key::cut_keys(project, &ws.output_dir(), Some(clip_dir.as_str()));
    let handle = jobs
        .create(
            owner,
            JobSpec::new("cut_one", format!("剪切事件 #{rank}"), keys, JobClass::Medium),
        )
        .await;
    let id = handle.id.clone();
    let queued = handle.queued;
    jobs.attach_project(&id, project).await;
    let jobs_handle = jobs.clone(); let cfg_file = ws.config_file.clone(); let job_id=id.clone();
    // 剪切是一整段同步代码（ffmpeg 等源文件读完整段），跑在专用任务线程上 —— 不占 HTTP worker
    spawn_job(jobs_handle.clone(), job_id.clone(), move || async move {
        if handle.wait_start().await.is_err() {
            jobs_handle.complete(&job_id).await;
            return;
        }
        let result = run_single_cut_event(&cfg_file, ev).await;
        match result { Ok(_)=>jobs_handle.complete(&job_id).await, Err(e)=>jobs_handle.fail(&job_id, format!("{e:#}")).await }
    });
    ok(serde_json::json!({ "ok": true, "id": id, "queued": queued }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::test_support::*;

    use axum::body::Body;

    /// **B7（2026-11-14 后端 review）：已超配额的普通账号不能再起转写/检测/剪切任务**。
    ///
    /// 占用是 `quota_gate_impl` 自己刷新的（源文件落盘即计入）—— 内核判据见 api/mod.rs
    /// 的单测；这里钉的是 HTTP 面：409 + 文案点名配额，admin 不受限。
    #[tokio::test]
    async fn pipeline_is_rejected_when_account_is_over_quota() {
        let t = test_app().await;
        let db = t.auth.db();
        let hash = yekitrim_service::auth::hash_password("pass-12345678").unwrap();
        let uid = db.create_user("quota_http", &hash, "user", false, 100).unwrap();
        let root = t._dir.path().join(format!("quota-proj-{uid}"));
        std::fs::create_dir_all(root.join("output")).unwrap();
        std::fs::write(root.join("big.mp4"), vec![0u8; 200]).unwrap();
        let cfg_path = root.join("yekitrim.toml");
        std::fs::write(&cfg_path, "input_file = \"big.mp4\"\n").unwrap();
        let pid = db
            .create_project(uid, "quota-proj", None, "local", Some("配额"), &root, &cfg_path, None)
            .unwrap();
        let b = t.auth.login("quota_http", "pass-12345678", None, None).unwrap();

        let res = t
            .call(
                t.req_as(&b.session_id, &b.csrf, "POST", &format!("/project-data/pipeline?project={pid}"))
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"stage":"detect"}"#.to_string()))
                    .unwrap(),
            )
            .await;
        assert_eq!(res.status(), StatusCode::CONFLICT, "已超配额必须 409");
        let j = body_json(res).await;
        assert!(j["error"].as_str().unwrap_or("").contains("配额"), "{j}");

        // admin 不受限：同一形状的请求（admin 自己的项目 1）不是 409
        let res = t
            .call(
                t.req("POST", "/project-data/pipeline?project=1")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"stage":"detect"}"#.to_string()))
                    .unwrap(),
            )
            .await;
        assert_ne!(res.status(), StatusCode::CONFLICT, "admin 不受配额约束");
    }

#[test]
fn sec_to_ts_never_produces_invalid_ms() {
    // 手写 floor/round 版本会给出 ms = 1000
    let v = sec_to_ts_value(59.9999);
    assert_eq!(v["m"].as_u64(), Some(1));
    assert_eq!(v["s"].as_u64(), Some(0));
    assert_eq!(v["ms"].as_u64(), Some(0));

    let v = sec_to_ts_value(12.5);
    assert_eq!(v["s"].as_u64(), Some(12));
    assert_eq!(v["ms"].as_u64(), Some(500));
    assert!(sec_to_ts_value(-3.0)["h"].as_u64() == Some(0));
}

/// **磁盘预检把一键全切拦停**（2026-11-15 事故回归钉）：`[cutting] min_free_gb` 拉满时，
/// 每条 `run_single_cut` 都在 runner 的阶段预检处失败（`FreeSpaceError`）——
/// 任务必须以「磁盘空间不足」收场（**不是**"N 个事件剪切失败"），且一条片段都没剪出来；
/// 腾出空间后重跑只剪缺失的目录（全切的既有语义）＝天然续作。
#[tokio::test]
async fn cut_all_stops_and_reports_when_disk_precheck_fails() {
    let t = test_app().await;
    let root = t.root.clone();

    // 两个 enabled 事件，都没有 clip.mp4 ⇒ 全切两条都要剪
    let events = serde_json::json!({
        "titles": {},
        "events": [
            {"start": {"h":0,"m":0,"s":0,"ms":0}, "end": {"h":0,"m":0,"s":10,"ms":0},
             "label": "A", "summary": "", "event_type": "chat", "source": "LLM",
             "confidence": 1.0, "popularity": 0.5, "rank": 1, "clip_dir": "clip-a", "enabled": true},
            {"start": {"h":0,"m":0,"s":10,"ms":0}, "end": {"h":0,"m":0,"s":20,"ms":0},
             "label": "B", "summary": "", "event_type": "chat", "source": "LLM",
             "confidence": 1.0, "popularity": 0.4, "rank": 2, "clip_dir": "clip-b", "enabled": true}
        ]
    });
    std::fs::create_dir_all(root.join("output")).unwrap();
    std::fs::write(root.join("output").join("events.json"), serde_json::to_string(&events).unwrap()).unwrap();
    // 阈值拉满（9223372036854775807 = i64::MAX，TOML 整数上限；换算字节后饱和成
    // u64::MAX）⇒ 任何真实磁盘都必然不足
    std::fs::write(
        root.join("yekitrim.toml"),
        "[project]\ninput_file = \"a.mp4\"\n\n[cutting]\nmin_free_gb = 9223372036854775807\n",
    ).unwrap();

    let res = t.call(
        t.req("POST", "/project-data/events/cut-all?project=1")
            .header(axum::http::header::CONTENT_TYPE, "application/json")
            .body(Body::from("{}"))
            .unwrap(),
    ).await;
    assert_eq!(res.status(), StatusCode::OK, "cut-all 照常受理成任务");
    let v = body_json(res).await;
    let job_id = v["id"].as_str().unwrap_or("").to_string();
    assert!(!job_id.is_empty(), "{v}");

    // 轮询到终态（任务体跑在专用线程上，这里给它足够的时间）
    let mut error_msg = String::new();
    for _ in 0..200 {
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        let res = t.call(t.req("GET", &format!("/jobs/{job_id}")).body(Body::empty()).unwrap()).await;
        let v = body_json(res).await;
        let status = v["status"].as_str().unwrap_or("").to_string();
        if ["failed", "completed", "cancelled"].contains(&status.as_str()) {
            assert_eq!(status, "failed", "预检拦停 ⇒ 任务必须失败: {v}");
            error_msg = v["error"].as_str().unwrap_or("").to_string();
            break;
        }
    }
    assert!(error_msg.contains("磁盘空间不足"), "失败原因必须点名空间不足: {error_msg}");
    assert!(error_msg.contains("未处理"), "要把没剪的条数说清: {error_msg}");
    assert!(!error_msg.contains("个事件剪切失败"), "空间拦停不能伪装成剪切失败: {error_msg}");

    // 一条都没剪出来（预检挡在 ffmpeg 之前）
    assert!(!root.join("output/clips/clip-a/clip.mp4").exists());
    assert!(!root.join("output/clips/clip-b/clip.mp4").exists());
}

/// `POST /project-data/events/merge-splits`：把老的等分产物 `xxx（1/2）`/`（2/2）`
/// 合回一个事件（时间取并集、标题去后缀、rank 沿用组内最热那条）；
/// 同名但**时间不相邻**的一对是给两个不同话题加的区分后缀，绝不能被合掉。
#[tokio::test]
async fn merge_splits_endpoint_merges_equal_splits_only() {
    let t = test_app().await;
    let root = t.root.clone();
    let ts = |s: f64| {
        let total = (s * 1000.0).round() as u64;
        serde_json::json!({ "h": total / 3_600_000, "m": (total / 60_000) % 60, "s": (total / 1000) % 60, "ms": total % 1000 })
    };
    let ev = |rank: u64, label: &str, s: f64, e: f64, pop: f64| {
        serde_json::json!({
            "start": ts(s), "end": ts(e), "label": label, "summary": format!("摘要 {label}"),
            "event_type": "chat", "source": "LLM", "confidence": 1.0,
            "popularity": pop, "rank": rank, "enabled": true
        })
    };
    let events = serde_json::json!({
        "titles": {},
        "events": [
            ev(4, "神婆见我一脸惊恐：你把两次机会都用完了？😱（1/2）", 0.0, 216.0, 0.30),
            ev(29, "神婆见我一脸惊恐：你把两次机会都用完了？😱（2/2）", 216.0, 432.0, 0.80),
            ev(7, "同一个话题（1/2）", 600.0, 700.0, 0.10),
            ev(8, "同一个话题（2/2）", 2000.0, 2100.0, 0.10),
            ev(9, "正常事件", 800.0, 900.0, 0.05)
        ]
    });
    std::fs::write(root.join("output").join("events.json"), serde_json::to_string_pretty(&events).unwrap()).unwrap();

    let res = t.call(t.req("POST", "/project-data/events/merge-splits?project=1").body(Body::empty()).unwrap()).await;
    let status = res.status();
    let j = body_json(res).await;
    assert_eq!(status, StatusCode::OK, "{j}");
    assert_eq!(j["groups"], 1, "{j}");
    assert_eq!(j["before"], 5, "{j}");
    assert_eq!(j["after"], 4, "{j}");

    let raw = std::fs::read_to_string(root.join("output").join("events.json")).unwrap();
    let v: serde_json::Value = serde_json::from_str(&raw).unwrap();
    let evs = v["events"].as_array().unwrap();
    let merged = evs.iter().find(|e| e["label"].as_str().unwrap_or("").starts_with("神婆")).expect("合并后的事件还在");
    assert_eq!(merged["label"], "神婆见我一脸惊恐：你把两次机会都用完了？😱", "后缀必须去掉");
    assert_eq!(merged["start"], ts(0.0), "时间取并集（起点）");
    assert_eq!(merged["end"], ts(432.0), "时间取并集（终点）");
    assert_eq!(merged["rank"], 29, "rank 沿用组内最热的那条（已剪片段里至少一个目录还对得上）");
    assert!(!evs.iter().any(|e| e["label"].as_str().unwrap_or("").starts_with("神婆") && e["label"].as_str().unwrap_or("").contains("（")),
            "被合的成员不能留在文件里: {v}");
    assert_eq!(evs.iter().filter(|e| e["label"].as_str().unwrap_or("").starts_with("同一个话题")).count(), 2,
               "同名但时间不相邻的一对不能被合并: {v}");
    assert!(evs.iter().any(|e| e["label"] == "正常事件"), "{v}");

    // 幂等：再点一次没有可合并的组，文件不再被改写
    let res = t.call(t.req("POST", "/project-data/events/merge-splits?project=1").body(Body::empty()).unwrap()).await;
    let j = body_json(res).await;
    assert_eq!(j["groups"], 0, "{j}");
    assert_eq!(j["after"], 4, "{j}");
}

/// **手工标的多段切片：重叠的段落盘时必须并成一段**（否则同一段内容会被剪两遍）。
///
/// 现场：项目 12 那条手工 4 段片段 `1625.7–1653.8` 里套着 `1632.0–1643.0` —— `clip.mp4`
/// 158.3s = 四段之和，其中 11s 是重复的，而 `duration` 也按各段之和多报一遍。
/// 判据走**真接口**（`POST /project-data/events`），不直接调 `merge_overlapping_ranges`：
/// 要钉住的正是"这条路真的会调它"。
#[tokio::test]
async fn create_event_merges_overlapping_ranges() {
    let t = test_app().await;
    let pid = project_ids(&t).await[0];
    let body = serde_json::json!({
        "label": "重叠段用例",
        "ranges": [
            {"start_sec": 441.4, "end_sec": 486.5},
            {"start_sec": 929.0, "end_sec": 1003.0},
            {"start_sec": 1625.7, "end_sec": 1653.8},
            // ⚠️ 完全落在上一段里面：不许原样落盘
            {"start_sec": 1632.0, "end_sec": 1643.0}
        ]
    });
    let res = t
        .call(
            t.req("POST", &format!("/project-data/events?project={pid}"))
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&body).unwrap()))
                .unwrap(),
        )
        .await;
    assert_eq!(res.status(), StatusCode::OK, "建事件应当成功");

    let raw = std::fs::read_to_string(t.root.join("output/events.json")).unwrap();
    let v: serde_json::Value = serde_json::from_str(&raw).unwrap();
    let ev = v["events"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["label"] == "重叠段用例")
        .expect("新事件必须落盘");
    let rs = ev["ranges"].as_array().expect("多段事件必须带 ranges");
    assert_eq!(rs.len(), 3, "重叠的那段要被并掉：{rs:?}");
    assert_eq!(
        (
            rs[2]["start"].as_f64().unwrap(),
            rs[2]["end"].as_f64().unwrap()
        ),
        (1625.7, 1653.8),
        "并出来的应当是外层那段（内含段不许把外层截短）"
    );
    // 外沿不变；也就是说"跨度"这条口径没被并段改掉
    assert_eq!(ev["start"]["m"].as_u64().unwrap(), 7);
    assert_eq!(ev["end"]["m"].as_u64().unwrap(), 27);
}

/// 反例：**中间隔了内容的两段不许并**（那是"分段剪、把中间别的话题丢掉"的表达）。
#[tokio::test]
async fn create_event_keeps_ranges_with_content_gap() {
    let t = test_app().await;
    let pid = project_ids(&t).await[0];
    let body = serde_json::json!({
        "label": "分段用例",
        "ranges": [
            {"start_sec": 100.0, "end_sec": 120.0},
            {"start_sec": 140.0, "end_sec": 160.0}
        ]
    });
    let res = t
        .call(
            t.req("POST", &format!("/project-data/events?project={pid}"))
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&body).unwrap()))
                .unwrap(),
        )
        .await;
    assert_eq!(res.status(), StatusCode::OK);
    let raw = std::fs::read_to_string(t.root.join("output/events.json")).unwrap();
    let v: serde_json::Value = serde_json::from_str(&raw).unwrap();
    let ev = v["events"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["label"] == "分段用例")
        .expect("新事件必须落盘");
    assert_eq!(
        ev["ranges"].as_array().unwrap().len(),
        2,
        "中间隔了 20s 内容，两段必须留着"
    );
}

/// **T9**：`CreateEventBody`（`POST /project-data/events`）—— 缺字段 / 类型错一律 400 且**点名字段**。
///
/// ⚠️ 这里特别钉住 `ranges` / `start_sec`：它们是"建一条片段"的核心参数，写错名字就会静默落盘一条
/// **起止 0-10s 的假事件**（改造前就是这样：`get()` 取不到 → `unwrap_or(默认)`），比 400 难查得多。
#[tokio::test]
async fn create_event_body_is_typed_and_names_the_field() {
    let t = test_app().await;
    let uri = "/project-data/events?project=1";

    // 缺 label
    let (status, text) = call_json(&t, "POST", uri, serde_json::json!({})).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "缺 label 必须 400：{text}");
    assert!(text.contains("label"), "报错要点名 label：{text}");

    // label 类型错
    let (status, text) = call_json(&t, "POST", uri, serde_json::json!({ "label": 5 })).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "label 类型错必须 400：{text}");
    assert!(text.contains("label"), "报错要点名 label：{text}");

    // ranges 类型错（不是数组）
    let (status, text) = call_json(
        &t,
        "POST",
        uri,
        serde_json::json!({ "label": "x", "ranges": "nope" }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "ranges 类型错必须 400：{text}");
    assert!(text.contains("ranges"), "报错要点名 ranges：{text}");

    // start_sec 类型错（字符串）
    let (status, text) = call_json(
        &t,
        "POST",
        uri,
        serde_json::json!({ "label": "x", "start_sec": "abc" }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "start_sec 类型错必须 400：{text}");
    assert!(text.contains("start_sec"), "报错要点名 start_sec：{text}");

    // 宽松解析：多传的陌生键照旧忽略（走到下游「标题不能为空」而不是解析错）
    let (status, text) = call_json(
        &t,
        "POST",
        uri,
        serde_json::json!({ "label": "", "nobody_knows": 1 }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(text.contains("标题不能为空"), "多余字段被忽略，于是走原有校验：{text}");
}

}