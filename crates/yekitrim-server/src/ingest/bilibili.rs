//! B站链接通道：`POST /api/ingest/resolve`（只看不下载）与 `POST /api/ingest/bilibili`（提交即下载）。
//!
//! ⚠️ 录播姬与它**共用 `ingest_sessions`**（靠 `kind` 列区分）：别拿 `room_id` 塞 bvid（会被
//! `channel_map` 按房间号查的规则命中）；`projects.room` 同理。命令构造 / 哨兵在 `yekitrim_service`。

use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;

use crate::auth::CurrentUser;
use crate::auth::owner_of;
use crate::model::AppState;

use super::*;
use super::records::internal;
use super::config::projects_root_gate;

/// B站接口错误 → HTTP 状态码。
///
/// 分开映射而不是一律 500：前端据此决定文案与"要不要让用户重试"
/// （`RateLimited` 是"稍后再来"，`Forbidden` 是"换链接"，完全不同）。
fn bili_status(e: &yekitrim_service::bilibili::BiliError) -> StatusCode {
    use yekitrim_service::bilibili::BiliError as B;
    match e {
        B::BadInput(_) => StatusCode::BAD_REQUEST,
        B::NotFound => StatusCode::NOT_FOUND,
        B::Forbidden | B::NotAvailable(_) => StatusCode::FORBIDDEN,
        B::RateLimited => StatusCode::TOO_MANY_REQUESTS,
        B::Api { .. } | B::Transport(_) | B::Malformed(_) => StatusCode::BAD_GATEWAY,
    }
}

fn bili_err(e: yekitrim_service::bilibili::BiliError) -> AppError {
    err(bili_status(&e), e.user_facing())
}

/// 清晰度档位合法性（前端下拉的取值域）。
///
/// 非法值**明确拒绝**而不是回退默认：`quality_height` 对认不出的串会返回 `None`
/// （= 不限高），静默接受会让"写错的清晰度"变成"下最高画质"，把磁盘与带宽吃光。
///
/// 返回的是**原因**而不是 `AppError`：两个调用点字段名不同
/// （提交端点叫 `quality`，设置页表单叫 `download.max_quality`），
/// 字段名前缀由调用方补 —— 共用校验、各自定位。
pub(super) fn check_quality(raw: &str) -> Result<(), String> {
    let s = raw.trim().to_ascii_lowercase();
    let ok = s.is_empty()
        || s == "best"
        || matches!(s.as_str(), "1080p" | "720p" | "480p" | "360p")
        || s.trim_end_matches('p').parse::<u32>().is_ok();
    if ok {
        Ok(())
    } else {
        Err(format!(
            "未知清晰度 {raw:?}（可用 1080p / 720p / 480p / 360p / best）"
        ))
    }
}

/// `POST /api/ingest/resolve {url}`：解析 B站链接，返回预览卡数据。
///
/// **薄壳、不落库**：用户每改一次输入框都会点一下解析，落库会攒出一堆没人要的会话。
///
/// 走的是 view API 而不是 `yt-dlp -J`（实测 T3：后者拿不到 cid 且慢得多，见
/// `yekitrim_service::bilibili` 的模块说明）。
///
/// 要求登录：这是个"替用户去请求 B站"的代理口，匿名开放等于给我们自己招风控。
pub async fn bilibili_resolve(
    axum::extract::Extension(_user): axum::extract::Extension<CurrentUser>,
    Json(body): Json<serde_json::Value>,
) -> ApiResult {
    let url = body.get("url").and_then(|v| v.as_str()).unwrap_or("").trim();
    if url.is_empty() {
        return Err(err(StatusCode::BAD_REQUEST, "请输入 B站视频链接"));
    }
    let bvid = yekitrim_service::bilibili::extract_bvid(url).map_err(bili_err)?;
    let info = yekitrim_service::bilibili::fetch_view(&bvid).await.map_err(bili_err)?;

    // 预览卡要显示"总占用" —— 前端不做码率估算，服务端给一个统一口径
    let cfg = yekitrim_core::config::load_global_config();
    let estimate_per_page: Vec<Option<u64>> = info
        .pages
        .iter()
        .map(|p| crate::download_worker::estimate_bytes(p.duration_secs, &cfg.ingest.max_quality))
        .collect();
    let total_estimate: u64 = estimate_per_page.iter().flatten().sum();

    ok(serde_json::json!({
        "ok": true,
        "bvid": info.bvid,
        "title": info.title,
        "uploader": info.uploader,
        "cover": info.cover,
        "duration_secs": info.duration_secs,
        "danmaku_total": info.danmaku_total,
        "view_total": info.view_total,
        // 多P 展开（单P 也会有一个元素，前端统一按列表渲染即可）
        "pages": info.pages.iter().zip(&estimate_per_page).map(|(p, est)| serde_json::json!({
            "index": p.index,
            "cid": p.cid,
            "part": p.part,
            "duration_secs": p.duration_secs,
            "estimate_bytes": est,
        })).collect::<Vec<_>>(),
        "estimate_bytes": total_estimate,
        "quality": cfg.ingest.max_quality,
        // 工具就绪位：没装时不拦解析（用户可以先看预览），提交时再引导
        "downloader_ready": yekitrim_service::downloader::registry::find_ytdlp(&cfg).is_some(),
    }))
}

/// `POST /api/ingest/bilibili {url, pages:[1,3], quality?, auto_pipeline?}`：提交下载。
///
/// 一次提交可以选多个 P —— **每个 P 一个会话**，一 P 一个项目。
/// 返回 `accepted`（本次真正入队的）与 `skipped`（已完成的），前端逐个跟进度。
pub async fn bilibili_submit(
    State(state): State<Arc<AppState>>,
    axum::extract::Extension(user): axum::extract::Extension<CurrentUser>,
    Json(body): Json<serde_json::Value>,
) -> ApiResult {
    let url = body.get("url").and_then(|v| v.as_str()).unwrap_or("").trim();
    if url.is_empty() {
        return Err(err(StatusCode::BAD_REQUEST, "请输入 B站视频链接"));
    }
    let bvid = yekitrim_service::bilibili::extract_bvid(url).map_err(bili_err)?;

    // 工具没就绪就**不收单**：收了也只能排在队列里干等，用户会以为卡住了。
    // 明确告诉他去装（前端有引导卡）。
    let cfg = yekitrim_core::config::load_global_config();
    if yekitrim_service::downloader::registry::find_ytdlp(&cfg).is_none() {
        return Err(err(
            StatusCode::SERVICE_UNAVAILABLE,
            "下载器（yt-dlp）未就绪；请在设置页一键下载后再提交",
        ));
    }
    if let Some(msg) = projects_root_gate(&user.role, &cfg.storage.projects_root) {
        return Err(err(StatusCode::PRECONDITION_REQUIRED, msg.to_string()));
    }
    if let Some(q) = body.get("quality").and_then(|v| v.as_str()) {
        check_quality(q)
            .map_err(|r| err(StatusCode::BAD_REQUEST, format!("quality：{r}")))?;
    }
    let auto_pipeline = body
        .get("auto_pipeline")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let info = yekitrim_service::bilibili::fetch_view(&bvid).await.map_err(bili_err)?;
    if info.pages.is_empty() {
        return Err(err(StatusCode::BAD_REQUEST, "这个视频没有可下载的分P"));
    }

    // 选中的 P：不传 = 第 1 P（**不默认全选** —— 23P 全下会直接撑爆配额）
    let wanted: Vec<u32> = match body.get("pages").and_then(|v| v.as_array()) {
        Some(arr) => arr
            .iter()
            .filter_map(|v| v.as_u64())
            .map(|n| n as u32)
            .collect(),
        None => vec![1],
    };
    if wanted.is_empty() {
        return Err(err(StatusCode::BAD_REQUEST, "请至少选择一个分P"));
    }
    let mut picked: Vec<&yekitrim_service::bilibili::BvPage> = Vec::new();
    for n in &wanted {
        let p = info
            .pages
            .iter()
            .find(|p| p.index == *n)
            .ok_or_else(|| {
                err(
                    StatusCode::BAD_REQUEST,
                    format!("这个视频没有第 {n} P（共 {} P）", info.pages.len()),
                )
            })?;
        if !picked.iter().any(|q| q.index == p.index) {
            picked.push(p);
        }
    }

    // 提交时先做一次**粗估**预检：不足就当场拒绝（别让用户等 worker 几分钟才失败）。
    // worker 里还会再查一次（那时下载目录已建、估算更准）。
    let incoming: u64 = picked
        .iter()
        .filter_map(|p| crate::download_worker::estimate_bytes(p.duration_secs, &cfg.ingest.max_quality))
        .sum();
    let probe_dir = yekitrim_core::owner::owner_root(owner_of(&user));
    yekitrim_service::ingest::capacity::check_capacity(
        state.auth.db(),
        user.id,
        incoming,
        &probe_dir,
        &cfg,
    )
    .map_err(|e| err(StatusCode::INSUFFICIENT_STORAGE, format!("{e:#}")))?;

    let db = state.auth.db();
    let total = info.pages.len();
    let mut accepted: Vec<serde_json::Value> = Vec::new();
    let mut skipped: Vec<serde_json::Value> = Vec::new();

    for p in picked {
        let sid = format!("bili-{bvid}-p{}", p.index);
        let sref = yekitrim_service::bilibili::session_ref(&bvid, Some(p.index));

        // 已建过项目的：不重下（用户想重来应当去项目页删掉再提交）
        if let Some(existing) = db.ingest_session(&sid).map_err(internal)? {
            if existing.project_id.is_some() {
                skipped.push(serde_json::json!({
                    "session_id": sid,
                    "page": p.index,
                    "reason": "已存在项目",
                    "project_id": existing.project_id,
                }));
                continue;
            }
        }

        let created = db
            .create_ingest_session(
                &sid,
                "bilibili",
                user.id,
                Some(&sref),
                "queued",
                auto_pipeline.as_deref(),
            )
            .map_err(internal)?;
        // 重投同一份（上次失败/被取消）：显式打回队列 —— `create_ingest_session`
        // 幂等时不改 state，不给这一步的话"再点一次提交"会毫无反应。
        db.set_ingest_state(&sid, "queued", None).map_err(internal)?;

        let title = {
            let part = p.part.trim();
            if total <= 1 {
                info.title.clone()
            } else if part.is_empty() {
                format!("{} P{}", info.title, p.index)
            } else {
                format!("{} P{} {part}", info.title, p.index)
            }
        };
        accepted.push(serde_json::json!({
            "session_id": sid,
            "page": p.index,
            "title": title,
            "cid": p.cid,
            "duration_secs": p.duration_secs,
            "created": created,
        }));
    }

    let _ = db.audit(
        Some(user.id),
        "ingest_bilibili_submit",
        &format!("bvid={bvid} pages={} accepted={}", wanted.len(), accepted.len()),
    );
    // 唤醒 worker（周期 tick 也会兜底）
    crate::download_worker::notify(state, None);
    ok(serde_json::json!({
        "ok": true,
        "bvid": bvid,
        "accepted": accepted,
        "skipped": skipped,
        "estimate_bytes": incoming,
    }))
}

