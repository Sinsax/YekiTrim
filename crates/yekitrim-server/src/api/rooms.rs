//! `api::rooms`：直播间自助登记（录播姬）与注册开关。
//!
//! ⚠️ 不叫 `ingest`：**真正的入库 HTTP handler 在 `crate::ingest`**（webhook / 记录 / 待分配池）。
//! 这里只有「用户填房间号 → 去录播姬加房间」这一块，叫 `rooms` 才不会和它撞概念。

use std::sync::Arc;

use axum::{
    extract::{Extension, Path as AxumPath, State},
    http::StatusCode,
    Json,
};

use crate::model::AppState;
use super::*;

// ===================== 直播间（录播姬） =====================

/// 当前录制软件对接状态。**给普通用户只说能不能用**（不泄露地址/错误细节）。
pub(super) fn recorder_client() -> Option<yekitrim_service::recorder::RecorderBackend> {
    let cfg = yekitrim_core::config::load_global_config();
    yekitrim_service::recorder::RecorderBackend::from_config(&cfg.ingest)
}

fn missing_hint(kind: &str) -> String {
    match kind {
        "none" => "未接录制软件（设置 → 接入 → 录制软件）".into(),
        "bililive-tools" => "未配置 biliLive-tools（[ingest.recorder] api_base / api_key 为空）".into(),
        _ => "未配置录播姬（[ingest.recorder] api_base 为空）".into(),
    }
}

/// biliLive-tools 的 `video_file` → 本地 `(绝对路径, 大小, mtime)`；`None` = 磁盘上找不到
/// （历史里常有早已删掉的场次，**负结果也缓存** —— 否则每次点房间都要把上百次失败重跑一遍）。
type BltFileInfo = Option<(std::path::PathBuf, u64, i64)>;

/// 一批历史条目的**本地解析**（`map_webhook_path` 会 `canonicalize`，打在网络盘上就是几百毫秒）。
///
/// ⚠️ **同步**：调用方必须下 `spawn_blocking`，而且**一次调用解析整批**。
/// 这里曾经是「async handler 里逐条 `map_webhook_path` + `metadata`」—— 129 条历史 ≈ 18 秒，
/// 期间还占住 tokio worker（打开「添加录像」整块卡死就是这么来的）。
fn resolve_blt_paths(
    root: &std::path::Path,
    container_root: &str,
    paths: &[String],
) -> std::collections::HashMap<String, BltFileInfo> {
    // 命中：5 分钟（录像落盘后基本不再变；新文件不在缓存里，会当场解析）。
    // 未命中：只 30 秒 —— 否则「共享还没挂上时先点了一次」会把 129 条"不存在"记 5 分钟，
    // 用户挂好盘回来还是空的。
    const TTL_OK: std::time::Duration = std::time::Duration::from_secs(300);
    const TTL_MISS: std::time::Duration = std::time::Duration::from_secs(30);
    type Slot = std::sync::Mutex<
        std::collections::HashMap<String, (std::time::Instant, BltFileInfo)>,
    >;
    static CACHE: std::sync::OnceLock<Slot> = std::sync::OnceLock::new();
    let cache = CACHE.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()));

    // 缓存键带上 root / container_root：配置一改，旧结论立刻失效（不用清表）。
    let key_of = |p: &str| format!("{}|{}|{p}", root.display(), container_root);
    let mut out = std::collections::HashMap::with_capacity(paths.len());
    let mut todo: Vec<&String> = Vec::new();
    {
        let g = cache.lock().unwrap_or_else(|e| e.into_inner());
        for p in paths {
            match g.get(&key_of(p)) {
                Some((at, info))
                    if at.elapsed() < if info.is_some() { TTL_OK } else { TTL_MISS } =>
                {
                    out.insert(p.clone(), info.clone());
                }
                _ => todo.push(p),
            }
        }
    }
    if todo.is_empty() {
        return out;
    }

    // 单条解析：`map_webhook_path` 会 canonicalize（网络盘上每次几十~上百毫秒）。
    let resolve_one = |p: &String| -> BltFileInfo {
        match crate::ingest::map_webhook_path(root, container_root, p) {
            Ok(abs) => std::fs::metadata(&abs).ok().map(|md| {
                let mtime = md
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_secs() as i64)
                    .unwrap_or(0);
                (abs, md.len(), mtime)
            }),
            Err(_) => None,
        }
    };

    // 冷路径常有上百条：**串行**打 SMB 就是十几秒（实测 129 条 ≈ 18s）。按 6 路并行，
    // 单次点开降到秒级；不足 5 条就不值得起线程，直接串行。
    let fresh: Vec<(String, BltFileInfo)> = if todo.len() <= 4 {
        todo.iter().map(|p| ((*p).clone(), resolve_one(p))).collect()
    } else {
        const LANES: usize = 6;
        let chunk = todo.len().div_ceil(LANES);
        let results: std::sync::Mutex<Vec<(String, BltFileInfo)>> =
            std::sync::Mutex::new(Vec::with_capacity(todo.len()));
        std::thread::scope(|scope| {
            for part in todo.chunks(chunk) {
                let results = &results;
                let resolve_one = &resolve_one;
                scope.spawn(move || {
                    let mut local = Vec::with_capacity(part.len());
                    for p in part {
                        local.push(((*p).clone(), resolve_one(*p)));
                    }
                    results.lock().unwrap_or_else(|e| e.into_inner()).extend(local);
                });
            }
        });
        results.into_inner().unwrap_or_else(|e| e.into_inner())
    };
    for (p, info) in &fresh {
        out.insert(p.clone(), info.clone());
    }
    {
        let mut g = cache.lock().unwrap_or_else(|e| e.into_inner());
        // 进程级缓存，体积封顶：超了就整表清空（代价只是下一轮重新 stat 一次）。
        if g.len() + fresh.len() > 4096 {
            g.clear();
        }
        let now = std::time::Instant::now();
        for (p, info) in fresh {
            g.insert(key_of(&p), (now, info));
        }
    }
    out
}

/// biliLive-tools：每个房间的"已录 N 个 · 大小"来自它的录像历史（路径里没有房间号可解析）。
async fn blt_summaries(
    cfg: &yekitrim_core::config::Config,
    client: &yekitrim_service::recorder::RecorderBackend,
    room_ids: &[String],
    live: &[yekitrim_service::recorder::RecorderRoom],
) -> std::collections::HashMap<String, crate::ingest_worker::RoomRecordings> {
    use std::collections::HashMap;
    let mut out: HashMap<String, crate::ingest_worker::RoomRecordings> = HashMap::new();
    let root = std::path::PathBuf::from(cfg.ingest.recorder.root.trim());
    let container_root = cfg.ingest.recorder.container_root.clone();

    // 先把各房间的历史取齐（HTTP，实测几十毫秒），再**一次**把全部路径解析到本地。
    // ⚠️ 别再退回「逐房间 spawn_blocking + 逐条 stat」：那是「房间数 × 历史条数」次跨网来回。
    let mut pending: Vec<(String, Vec<String>)> = Vec::new();
    let mut all_paths: Vec<String> = Vec::new();
    for rid in room_ids {
        let platform = live
            .iter()
            .find(|r| r.room_id.to_string() == *rid)
            .map(|r| r.platform.clone())
            .unwrap_or_else(|| "Bilibili".into());
        let Ok(items) = client.list_history(rid, &platform).await else { continue };
        let paths: Vec<String> = items.into_iter().map(|i| i.video_file).collect();
        all_paths.extend(paths.iter().cloned());
        pending.push((rid.clone(), paths));
    }
    if all_paths.is_empty() {
        return out;
    }
    let root_for_io = root.clone();
    let cr_for_io = container_root.clone();
    let resolved = tokio::task::spawn_blocking(move || {
        resolve_blt_paths(&root_for_io, &cr_for_io, &all_paths)
    })
    .await
    .unwrap_or_default();

    for (rid, paths) in pending {
        let mut s = crate::ingest_worker::RoomRecordings::default();
        for p in paths {
            let Some(Some((_, len, mtime))) = resolved.get(&p) else { continue };
            s.files += 1;
            s.bytes += *len;
            s.latest = s.latest.max(*mtime);
        }
        out.insert(rid, s);
    }
    out
}

/// `GET /api/rooms`：我的直播间 + 录制软件实时状态。
///
/// `available=false`（未对接 / 连不上）时前端把「配置直播间」置灰 —— 用户不该看到
/// 管理员才懂的那些配置指引。
pub(super) async fn rooms_list(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<crate::auth::CurrentUser>,
) -> ApiResult {
    let db = state.auth.db();
    let rows = db
        .rooms_of_user(user.id)
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("读取直播间失败: {e:#}")))?;
    let cfg = yekitrim_core::config::load_global_config();
    let all_admin = user.is_admin();
    let client = recorder_client();

    let mut live = Vec::new();
    let (available, reason) = match &client {
        None => (false, if all_admin { missing_hint(&cfg.ingest.recorder.kind) } else { "录制软件未接入".to_string() }),
        Some(c) => match c.list_rooms().await {
            Ok(rooms) => {
                live = rooms;
                (true, String::new())
            }
            Err(e) => {
                tracing::warn!("读录制软件房间列表失败: {e:#}");
                let why = if cfg.ingest.recorder.kind.trim() == "bililive-tools" {
                    "连不上 biliLive-tools（检查 api_base / 密钥）"
                } else {
                    "连不上录播姬（检查 api_base / Basic 账号）"
                };
                (false, if all_admin { why.to_string() } else { "录制软件暂时不可用".into() })
            }
        },
    };

    // 录像目录汇总：每行显示"已录 N 个 · 大小"（展开时再列明细，见 /api/rooms/:id/recordings）。
    // biliLive-tools 的路径里没有房间号，必须走它的录像历史；录播姬仍然扫目录。
    let blt = client.as_ref().is_some_and(|c| c.is_bililive_tools());
    // ⚠️ 这里曾写成 `if blt { root } else { root }` 的恒等分支（review P3）——
    // 两种后端的录像根**就是同一份配置**（[`[ingest.recorder] root`]），分支只会误导读者
    // 以为 blt 有另一个根。真有差异的是 `container_root`（见 recorder_objects）。
    let rec_root = cfg.ingest.recorder.root.as_str();
    // ⚠️ 扫描失败**不能只写日志**：`None` 会被前端渲染成「已录 0 个」，与"确实没录过"
    //    完全无法区分（跨机部署时这是最常见的报障形态）。所以把原因带出去。
    let mut recordings_error: Option<String> = None;
    let summaries = if rec_root.trim().is_empty() {
        None
    } else if available && blt {
        let ids: Vec<String> = rows.iter().map(|r| r.room_id.clone()).collect();
        Some(blt_summaries(&cfg, client.as_ref().unwrap(), &ids, &live).await)
    } else {
        let cfg_scan = cfg.clone();
        match tokio::task::spawn_blocking(move || crate::ingest_worker::list_recordings(&cfg_scan)).await {
            Ok(Ok(files)) => Some(crate::ingest_worker::room_summaries(&files)),
            Ok(Err(e)) => {
                tracing::warn!("读录像目录失败: {e:#}");
                recordings_error = Some(format!("{e:#}"));
                None
            }
            Err(e) => {
                tracing::warn!("读录像目录线程失败: {e}");
                recordings_error = Some(format!("扫描线程失败: {e}"));
                None
            }
        }
    };
    // 文案分两套：admin 看得到路径/OS 错误 + 探针提示（缓存里那次，**这里不探测**）；
    // 普通用户只拿中性结论，绝不回路径。
    let recordings_available = recordings_error.is_none();
    let recordings_error = recordings_error.map(|raw| {
        if all_admin {
            match crate::ingest::last_root_probe(rec_root) {
                Some(p) if !p.hint.is_empty() => format!("{raw} · {}", p.hint),
                _ => raw,
            }
        } else {
            "录像目录暂不可用".to_string()
        }
    });

    let rooms: Vec<serde_json::Value> = rows
        .iter()
        .map(|r| {
            let st = live.iter().find(|x| x.room_id.to_string() == r.room_id);
            let rec = summaries.as_ref().and_then(|m| m.get(&r.room_id)).copied();
            serde_json::json!({
                "room_id": r.room_id,
                "title": if r.title.is_empty() { st.map(|x| x.title.clone()).unwrap_or_default() } else { r.title.clone() },
                "enabled": r.enabled,
                "auto_pipeline": r.auto_pipeline,
                "source": r.source,
                "streaming": st.map(|x| x.streaming).unwrap_or(false),
                "recording": st.map(|x| x.recording).unwrap_or(false),
                "in_recorder": st.is_some(),
                "recordings": {
                    "files": rec.map(|s| s.files).unwrap_or(0),
                    "bytes": rec.map(|s| s.bytes).unwrap_or(0),
                    "latest": rec.map(|s| s.latest).unwrap_or(0),
                },
            })
        })
        .collect();
    let kind = client.as_ref().map(|c| c.kind()).unwrap_or_else(|| cfg.ingest.recorder.kind.trim());
    if all_admin {
        ok(serde_json::json!({
            "available": available,
            "reason": reason,
            "kind": kind,
            "rooms": rooms,
            "recordings_available": recordings_available,
            "recordings_error": recordings_error,
        }))
    } else {
        ok(serde_json::json!({
            "available": available,
            "kind": kind,
            "rooms": rooms,
            "recordings_available": recordings_available,
            "recordings_error": recordings_error,
        }))
    }
}

/// `POST /api/rooms` 的请求体：`{url}`（直播间链接）或 `{room_id}`（房间号）**二选一**。
///
/// ⚠️ 两个字段都**可选**（老脚本可能只给一个），但**至少要有一个非空** ——
/// 都没有时 handler 回 400 **点名这两个字段**，而不是让 `parse_room_id` 去抱怨空串。
#[derive(serde::Deserialize)]
pub(super) struct RoomAddBody {
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub room_id: Option<String>,
}

/// `POST /api/rooms {url|room_id}`：登记直播间。
///
/// - 录播姬：去它那边加房间（开播即录）；
/// - biliLive-tools：**只在 YekiTrim 里登记**，不碰它的录制配置（只读适配）。
pub(super) async fn rooms_add(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<crate::auth::CurrentUser>,
    Json(body): Json<serde_json::Value>,
) -> ApiResult {
    let RoomAddBody { url, room_id } = parse_body(body)?;
    let input = match url.or(room_id) {
        Some(v) if !v.trim().is_empty() => v,
        _ => {
            return Err(err(
                StatusCode::BAD_REQUEST,
                "缺少 url 或 room_id（直播间链接 / 房间号至少给一个）",
            ))
        }
    };
    let room_id = yekitrim_service::recorder::parse_room_id(&input)
        .map_err(|e| err(StatusCode::BAD_REQUEST, format!("{e}")))?;

    let Some(client) = recorder_client() else {
        let kind = yekitrim_core::config::load_global_config().ingest.recorder.kind;
        return Err(err(
            StatusCode::SERVICE_UNAVAILABLE,
            if user.is_admin() {
                format!("{}（设置 → 采集 → 录制软件）", missing_hint(&kind))
            } else {
                "录制软件未接入，请联系管理员".into()
            },
        ));
    };
    // 别人的房间不能被顶掉：先查归属
    let db = state.auth.db();
    if let Some(existing) = db
        .channel_for_room(&room_id.to_string())
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("查询归属失败: {e:#}")))?
    {
        if existing.user_id != user.id && !user.is_admin() {
            return Err(err(StatusCode::CONFLICT, "这个直播间已被其他用户登记"));
        }
    }

    let title = if client.is_bililive_tools() {
        // 只读适配：房间必须已经在 biliLive-tools 的录制列表里（我们不去改它的配置）
        let rooms = client
            .list_rooms()
            .await
            .map_err(|e| err(StatusCode::BAD_GATEWAY, format!("{e:#}")))?;
        let Some(found) = rooms.iter().find(|r| r.room_id.to_string() == room_id.to_string()) else {
            return Err(err(
                StatusCode::NOT_FOUND,
                "biliLive-tools 的录制列表里没有这个直播间：先在它那边添加（本页只登记，不改它的配置）",
            ));
        };
        found.title.clone()
    } else {
        let room = client
            .add_room(room_id)
            .await
            .map_err(|e| err(StatusCode::BAD_GATEWAY, format!("{e:#}")))?;
        room.title
    };
    let title = if title.is_empty() { String::new() } else { title };
    db.room_upsert(&room_id.to_string(), user.id, &title)
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("登记直播间失败: {e:#}")))?;
    let _ = db.audit(Some(user.id), "room_add", &format!("room={room_id}"));

    ok(serde_json::json!({
        "ok": true,
        "room": {
            "room_id": room_id.to_string(),
            "title": title,
            "in_recorder": true,
        }
    }))
}

/// `DELETE /api/rooms/:room_id`：注销登记（录播姬会顺带从它那边移除；biliLive-tools 不动它的配置）。
///
/// 录播姬那边删失败就**不动登记** —— 否则会留下"还在录但没有归属"的房间。
pub(super) async fn rooms_delete(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<crate::auth::CurrentUser>,
    AxumPath(room_id): AxumPath<String>,
) -> ApiResult {
    let db = state.auth.db();
    let room_id = room_id.trim().to_string();
    let owned = db
        .channel_for_room(&room_id)
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("查询归属失败: {e:#}")))?
        .ok_or_else(|| err(StatusCode::NOT_FOUND, "直播间未登记"))?;
    if owned.user_id != user.id && !user.is_admin() {
        return Err(err(StatusCode::FORBIDDEN, "这是别人登记的直播间"));
    }

    if let Some(client) = recorder_client() {
        if !client.is_bililive_tools() {
            let id = yekitrim_service::recorder::parse_room_id(&room_id)
                .map_err(|e| err(StatusCode::BAD_REQUEST, format!("{e}")))?;
            client
                .remove_room(id)
                .await
                .map_err(|e| err(StatusCode::BAD_GATEWAY, format!("从录播姬移除失败（登记未改动）：{e:#}")))?;
        }
    }
    let owner_filter = if user.is_admin() { None } else { Some(user.id) };
    let removed = db
        .room_delete(&room_id, owner_filter)
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("注销直播间失败: {e:#}")))?;
    // **删 0 行 = 没删到东西，必须如实回 404**（2026-11-14 后端 review 的 P1 的余项）。
    // 上面那次 `channel_for_room` 归属检查与这里不是原子的：中间被别的账号/PUT channel-map
    // 改走归属时，`DELETE ... WHERE user_id = ?` 会删 0 行，而旧代码照样回 200 `{ok:true}` ——
    // 界面显示"已移除"，列表里它却被别人占着。
    if removed == 0 {
        return Err(err(StatusCode::NOT_FOUND, "直播间未登记（或归属已变更）"));
    }
    let _ = db.audit(Some(user.id), "room_delete", &format!("room={room_id}"));
    ok(serde_json::json!({ "ok": true }))
}

/// `GET /api/recorder/objects`：首页「添加录像」用 —— 当前录制软件的**录制对象 + 目录**。
///
/// 只读：biliLive-tools 走它的录像历史；录播姬扫录像目录。`configured=false` 时前端显示
/// "未接录制软件 → 去设置"。
///
/// ⚠️ **归属过滤（2026-11-14 后端 review B2）**：普通用户只看**自己登记过的房间**
/// （`channel_map` 里 `user_id` 是他的那些）；admin 通吃。旧实现把全平台房间连同
/// 录像目录绝对路径一起回给任何登录用户。`root`（录像根）也只给 admin ——
/// 与 `/api/rooms` 的"普通用户不返回管理员指引"同一条口径。
pub(super) async fn recorder_objects(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<crate::auth::CurrentUser>,
    axum::extract::Query(q): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> ApiResult {
    let cfg = yekitrim_core::config::load_global_config();
    let _ = q; // 兼容旧的 `?kind=`，现在只有一个录播模块（kind 在配置里）
    let is_admin = user.is_admin();
    let db = state.auth.db();
    // 普通用户 = 只看他自己的 channel_map；admin = None（不过滤）
    let mine: Option<std::collections::HashSet<String>> = if is_admin {
        None
    } else {
        Some(
            db.rooms_of_user(user.id)
                .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("读取直播间失败: {e:#}")))?
                .into_iter()
                .map(|r| r.room_id)
                .collect(),
        )
    };
    let client = recorder_client();
    let Some(client) = client else {
        return ok(serde_json::json!({
            "configured": false,
            "available": false,
            "kind": null,
            "objects": [],
        }));
    };
    let kind = client.kind();
    let blt = client.is_bililive_tools();
    // ⚠️ 这里曾写成 `if blt { root } else { root }` 的恒等分支（review P3）——见 rooms_list 同款注释。
    let root = std::path::PathBuf::from(cfg.ingest.recorder.root.trim());
    let container_root = if blt {
        cfg.ingest.recorder.container_root.clone()
    } else {
        String::new()
    };

    let rooms = match client.list_rooms().await {
        Ok(r) => r,
        Err(e) => {
            return ok(serde_json::json!({
                "configured": true,
                "available": false,
                "kind": kind,
                "error": format!("{e:#}"),
                "objects": [],
            }))
        }
    };

    // 录播姬：扫一遍目录，按房间号归属；biliLive-tools：逐房间查录像历史
    let scan = if blt || root.as_os_str().is_empty() {
        None
    } else {
        let cfg_scan = cfg.clone();
        tokio::task::spawn_blocking(move || crate::ingest_worker::list_recordings(&cfg_scan))
            .await
            .ok()
            .and_then(|r| r.ok())
    };

    // 分两段：① 收历史（HTTP，几十毫秒/房间）② **一次**把全部路径解析到本地（blocking + 缓存）。
    // ⚠️ 逐条在 async handler 里 `map_webhook_path` + `metadata` = 129 条历史 ≈ 18 秒，
    //    而且期间占着 tokio worker（打开「添加录像」整块卡住就是这么来的）。
    let mut pends: Vec<(String, String, String, Vec<String>)> = Vec::new();
    let mut all_paths: Vec<String> = Vec::new();
    for r in rooms {
        let rid = r.room_id.to_string();
        // B2：不在本人 channel_map 里的房间，一个字都不回（不泄露目录在录什么）
        if !recorder_room_visible(mine.as_ref(), &rid) {
            continue;
        }
        let mut paths = Vec::new();
        if blt && !root.as_os_str().is_empty() {
            if let Ok(items) = client.list_history(&rid, &r.platform).await {
                paths = items.into_iter().map(|i| i.video_file).collect();
                all_paths.extend(paths.iter().cloned());
            }
        }
        pends.push((rid, r.title, r.platform, paths));
    }
    let resolved = if all_paths.is_empty() {
        std::collections::HashMap::new()
    } else {
        let root_for_io = root.clone();
        let cr_for_io = container_root.clone();
        tokio::task::spawn_blocking(move || resolve_blt_paths(&root_for_io, &cr_for_io, &all_paths))
            .await
            .unwrap_or_default()
    };

    let mut objects = Vec::new();
    for (rid, title, platform, paths) in pends {
        let mut dir = String::new();
        let mut files = 0usize;
        let mut bytes = 0u64;
        let mut latest = 0i64;
        if blt {
            for p in &paths {
                let Some(Some((abs, len, mtime))) = resolved.get(p) else { continue };
                if dir.is_empty() {
                    dir = abs.parent().map(yekitrim_core::path::display).unwrap_or_default();
                }
                files += 1;
                bytes += *len;
                latest = latest.max(*mtime);
            }
        } else if let Some(files_ref) = scan.as_ref() {
            for f in files_ref.iter().filter(|f| f.room.as_deref() == Some(rid.as_str())) {
                if dir.is_empty() {
                    dir = root.join(&f.rel).parent().map(yekitrim_core::path::display).unwrap_or_default();
                }
                files += 1;
                bytes += f.size;
                latest = latest.max(f.mtime);
            }
        }
        objects.push(serde_json::json!({
            "id": rid,
            "title": title,
            "platform": platform,
            "dir": dir,
            "files": files,
            "bytes": bytes,
            "latest": latest,
        }));
    }

    ok(serde_json::json!({
        "configured": true,
        "available": true,
        "kind": kind,
        // B2：录像根的绝对路径只给 admin（普通用户的 objects 已经按本人房间过滤）
        "root": if is_admin { yekitrim_core::path::display(&root) } else { String::new() },
        "objects": objects,
    }))
}

/// B2：这个房间要不要出现在普通用户的录制对象里。
///
/// `None`（admin）= 全部可见；`Some(自己的房间)` = 只认本人登记的。抽成纯函数是为了
/// 能在单测里钉住"admin 通吃 / 普通用户只看自己"这两条，不必真起一个录播姬。
fn recorder_room_visible(mine: Option<&std::collections::HashSet<String>>, room_id: &str) -> bool {
    match mine {
        None => true,
        Some(set) => set.contains(room_id),
    }
}

/// B2：`added` / `project_id` 只反映**调用者自己**的登记（admin 视角 = 全部）。
///
/// `project_by_config` 反查的是**任何人**登记的项目 —— 同一个目录被两个账号先后登记时，
/// 普通用户不该看到别人那条的 id。抽成纯函数是为了两处（blt / 扫描分支）共用 + 单测可钉。
fn added_project_of(
    db: &yekitrim_service::db::Db,
    is_admin: bool,
    uid: i64,
    config_file: &std::path::Path,
) -> (bool, Option<i64>) {
    let row = if is_admin {
        db.project_by_config(config_file).ok().flatten()
    } else {
        db.project_by_config_for(config_file, uid).ok().flatten()
    };
    match row {
        Some(p) => (true, Some(p.id)),
        None => (false, None),
    }
}

/// 录像目录不可用时的统一降级响应（200 + `available:false`）。
///
/// 与 `recorder_objects` 的 `available/error` 同口径：前端据此出一条**说明**，而不是把 500
/// 渲染成一屏错误。普通用户只拿中性文案（**绝不回路径 / OS 错误**）。
fn degraded_recordings(
    cfg: &yekitrim_core::config::Config,
    is_admin: bool,
    room_id: &str,
    raw: &str,
) -> serde_json::Value {
    let error = if is_admin {
        match crate::ingest::last_root_probe(cfg.ingest.recorder.root.trim()) {
            Some(p) if !p.hint.is_empty() => format!("{raw} · {}", p.hint),
            _ => raw.to_string(),
        }
    } else {
        "录像目录暂不可用".to_string()
    };
    serde_json::json!({
        "configured": true,
        "available": false,
        "room_id": room_id,
        "error": error,
        "recordings": [],
    })
}
/// `GET /api/recorder/recordings?room_id=`：某房间的录像列表（含 `added`，防重复添加）。
///
/// ⚠️ **归属校验（2026-11-14 后端 review B2）**：普通用户只能看**自己登记的**房间 ——
/// 与 `/api/rooms/:id/recordings` 的 [`crate::ingest::owned_room`] **同一份判据**
/// （同一语义两个端点一套有一道没有，正是 review 指出的问题）。不可见一律 404。
/// `added`/`project_id` 也只反映**调用者自己的**登记：别人在同一目录登记的项目，
/// id 不许回给普通用户（[`added_project_of`]）。
pub(super) async fn recorder_recordings(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<crate::auth::CurrentUser>,
    axum::extract::Query(q): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> ApiResult {
    let room_id = q.get("room_id").map(|s| s.trim().to_string()).unwrap_or_default();
    if room_id.is_empty() {
        return Err(err(StatusCode::BAD_REQUEST, "缺少 room_id"));
    }
    let is_admin = user.is_admin();
    let db = state.auth.db();
    // 归属校验**先于**一切（没接录制软件也要先 404 —— 不给普通用户"探房间存在性"的口子）
    if !is_admin {
        crate::ingest::owned_room(db, &user, &room_id)?;
    }
    let cfg = yekitrim_core::config::load_global_config();
    let Some(client) = recorder_client() else {
        return ok(serde_json::json!({ "configured": false, "recordings": [] }));
    };
    let root = std::path::PathBuf::from(cfg.ingest.recorder.root.trim());
    if root.as_os_str().is_empty() {
        return Err(err(StatusCode::BAD_REQUEST, "未配置录像目录"));
    }
    // 目录不可用（跨机没映射好 / 共享没挂上）→ 200 + `available:false`。
    // `is_dir()` 可能落在网络盘上 ⇒ 必须下 blocking 池。
    let root_for_check = root.clone();
    let root_is_dir = blocking(move || Ok(root_for_check.is_dir())).await?;
    if !root_is_dir {
        let raw = format!("录像目录不可用: {}", yekitrim_core::path::display(&root));
        return ok(degraded_recordings(&cfg, is_admin, &room_id, &raw));
    }
    let db = state.auth.db();
    let blt = client.is_bililive_tools();
    let container_root = cfg.ingest.recorder.container_root.clone();

    let mut rows: Vec<serde_json::Value> = Vec::new();

    if blt {
        let rooms = client
            .list_rooms()
            .await
            .map_err(|e| err(StatusCode::BAD_GATEWAY, format!("{e:#}")))?;
        let Some(st) = rooms.iter().find(|r| r.room_id.to_string() == room_id) else {
            return Err(err(StatusCode::NOT_FOUND, "biliLive-tools 的录制列表里没有这个直播间"));
        };
        let items = client
            .list_history(&room_id, &st.platform)
            .await
            .map_err(|e| err(StatusCode::BAD_GATEWAY, format!("{e:#}")))?;
        // 一场录像 = **一个目录**：size = 目录内全部视频之和（多分段以前只显示
        // 最后一段的大小），mtime = 最新一段的修改时间（= 录完时间）。
        // ⚠️ 数据源 = **biliLive-tools 历史 ∪ 磁盘扫描**：blt 的 record-history 只包含
        // **它自己录的**场次 —— 录播姬时代留在同一个主播目录里的老场次不在里面
        // （实测 26 个含视频的目录只列出 16 个，2026-11-15 用户口径："最低要求是有
        // 视频文件就行"）。归属按**子树**：场次目录的上一级（主播目录）整棵归这个房间。
        let mut agg: std::collections::HashMap<
            std::path::PathBuf,
            (u64, i64),
        > = std::collections::HashMap::new();
        let mut seen_files = std::collections::HashSet::new();
        let mut room_roots: Vec<std::path::PathBuf> = Vec::new();
        // ⚠️ 一批解析（blocking + 缓存）：这个房间有 129 条历史，逐条在 async 里摸 SMB
        //    就是「点一下房间要等十几秒」的全部原因。
        let paths: Vec<String> = items.iter().map(|i| i.video_file.clone()).collect();
        let (root_for_io, cr_for_io) = (root.clone(), container_root.clone());
        let paths_for_io = paths.clone();
        let resolved = tokio::task::spawn_blocking(move || {
            resolve_blt_paths(&root_for_io, &cr_for_io, &paths_for_io)
        })
        .await
        .unwrap_or_default();
        for p in &paths {
            let Some(Some((abs, len, mtime))) = resolved.get(p) else { continue };
            if !seen_files.insert(abs.clone()) {
                continue;
            }
            if let Some(session) = abs.parent() {
                if let Some(owner) = session.parent() {
                    for cand in [session.to_path_buf(), owner.to_path_buf()] {
                        if !room_roots.contains(&cand) {
                            room_roots.push(cand);
                        }
                    }
                }
            }
            let Some(dir) = abs.parent() else { continue };
            let e = agg.entry(dir.to_path_buf()).or_insert((0, 0));
            e.0 += *len;
            e.1 = e.1.max(*mtime);
        }
        // 同子树下、历史里没有的场次：扫盘补齐（尽力而为，扫失败不挡列表）
        let cfg_scan = cfg.clone();
        let root_scan = root.clone();
        let roots = room_roots.clone();
        let extra = tokio::task::spawn_blocking(move || {
            crate::ingest_worker::list_recordings(&cfg_scan).map(|files| {
                files
                    .into_iter()
                    .filter(|f| roots.iter().any(|r| root_scan.join(&f.rel).starts_with(r)))
                    .collect::<Vec<_>>()
            })
        })
        .await
        .ok();
        if let Some(Ok(files)) = extra {
            for f in files {
                let abs = root.join(&f.rel);
                if !seen_files.insert(abs.clone()) {
                    continue;
                }
                let Some(dir) = abs.parent() else { continue };
                let e = agg.entry(dir.to_path_buf()).or_insert((0, 0));
                e.0 += f.size;
                e.1 = e.1.max(f.mtime);
            }
        }
        for (dir, (size, mtime)) in agg {
            let rel = dir.strip_prefix(&root).map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
            let (added, project_id) = added_project_of(db, is_admin, user.id, &dir.join("yekitrim.toml"));
            rows.push(serde_json::json!({
                "name": dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
                "dir": yekitrim_core::path::display(&dir),
                "rel": rel,
                "size": size,
                "mtime": mtime,
                "added": added,
                "project_id": project_id,
            }));
        }
    } else {
        let cfg_scan = cfg.clone();
        let scanned = match tokio::task::spawn_blocking(move || {
            crate::ingest_worker::list_recordings(&cfg_scan)
        })
        .await
        {
            Ok(Ok(v)) => v,
            Ok(Err(e)) => {
                tracing::warn!("读录像目录失败: {e:#}");
                return ok(degraded_recordings(&cfg, is_admin, &room_id, &format!("{e:#}")));
            }
            Err(e) => {
                tracing::warn!("读录像目录线程失败: {e}");
                return ok(degraded_recordings(
                    &cfg,
                    is_admin,
                    &room_id,
                    &format!("扫描线程失败: {e}"),
                ));
            }
        };
        // 与 blt 分支同口径：按目录聚合（size 求和、mtime 取最新一段）
        let mut agg: std::collections::HashMap<
            std::path::PathBuf,
            (u64, i64, String),
        > = std::collections::HashMap::new();
        for f in scanned.iter().filter(|f| f.room.as_deref() == Some(room_id.as_str())) {
            let abs = root.join(&f.rel);
            let Some(dir) = abs.parent() else { continue };
            let e = agg
                .entry(dir.to_path_buf())
                .or_insert((0, 0, String::new()));
            e.0 += f.size;
            e.1 = e.1.max(f.mtime);
            if e.2.is_empty() {
                e.2 = dir
                    .strip_prefix(&root)
                    .map(|p| p.to_string_lossy().into_owned())
                    .unwrap_or_default();
            }
        }
        for (dir, (size, mtime, rel)) in agg {
            let (added, project_id) = added_project_of(db, is_admin, user.id, &dir.join("yekitrim.toml"));
            rows.push(serde_json::json!({
                "name": dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
                "dir": yekitrim_core::path::display(&dir),
                "rel": rel,
                "size": size,
                "mtime": mtime,
                "added": added,
                "project_id": project_id,
            }));
        }
    }
    rows.sort_by_key(|v| -v.get("mtime").and_then(|m| m.as_i64()).unwrap_or(0));
    ok(serde_json::json!({
        "configured": true,
        "available": true,
        "room_id": room_id,
        "recordings": rows,
    }))
}

/// `POST /api/admin/recorder/test`：对接自检 —— **连接**与**目录校验**分开报。
///
/// ⚠️ **body 里可以带未保存的当前输入**（2026-11-24 起）：前端「测试连接」原先只测**已存**配置，
/// 于是「刚改完密钥 → 点测试 → 报 401」实际验的是旧值，锅被甩给"网络/对方服务"（实测踩过：
/// biliLive-tools 的 passKey 被 `BILILIVE_TOOLS_PASSKEY` 环境变量覆盖，界面显示的是文件里那份）。
/// 现在 body 里给了 `kind`/`api_base`/`api_key`/`api_user`/`api_pass`/`root` 就**临时覆盖**
/// （不落盘），没给的沿用已存值。⚠️ 密钥只走 **body**，别塞 query —— `access_log` 会原样记 URL。
///
/// ⚠️ **目录那一半必须下阻塞池 + 超时**：死 UNC 的 `metadata` 能挂几十秒 —— 8 秒到点就按
/// "不可达"回，别让一次点击把请求拖死（也顺手把结论写进 [`crate::ingest::root_probe_cached`]）。
pub(super) async fn admin_recorder_test(body: axum::body::Bytes) -> ApiResult {
    let mut cfg = yekitrim_core::config::load_global_config();
    let over = if body.is_empty() {
        serde_json::Value::Null
    } else {
        serde_json::from_slice::<serde_json::Value>(&body).unwrap_or(serde_json::Value::Null)
    };
    let pick = |k: &str| -> Option<String> {
        over.get(k)
            .and_then(|x| x.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    };
    if let Some(kind) = pick("kind") {
        cfg.ingest.recorder.kind = kind;
    }
    if let Some(base) = pick("api_base") {
        cfg.ingest.recorder.api_base = base;
    }
    if let Some(key) = pick("api_key") {
        cfg.ingest.recorder.api_key = key;
    }
    if let Some(user) = pick("api_user") {
        cfg.ingest.recorder.api_user = user;
    }
    if let Some(pass) = pick("api_pass") {
        cfg.ingest.recorder.api_pass = pass;
    }
    // 「跨机先测再存」：共享路径还没保存时也要能验
    if let Some(root) = pick("root") {
        cfg.ingest.recorder.root = root;
    }

    // ① 连接（HTTP）
    let (kind, base, version, room_count, api_ok, api_err) =
        match yekitrim_service::recorder::RecorderBackend::from_config(&cfg.ingest) {
            None => (
                cfg.ingest.recorder.kind.clone(),
                String::new(),
                None,
                0usize,
                false,
                Some("未配置 [ingest.recorder] api_base".to_string()),
            ),
            Some(c) => {
                let base = c.base().to_string();
                let kind = c.kind().to_string();
                let version = c.version().await.ok();
                match c.list_rooms().await {
                    Ok(rooms) => (kind, base, version, rooms.len(), true, None),
                    Err(e) => (kind, base, version, 0, false, Some(format!("{e:#}"))),
                }
            }
        };

    // ② 目录校验（文件系统）
    let root = cfg.ingest.recorder.root.trim().to_string();
    let root_json = probe_root_with_timeout(root.clone()).await;
    let root_ok = root_json.get("ok").and_then(|v| v.as_bool()).unwrap_or(false);

    // ③ 路径映射（仅 biliLive-tools）：自动推导 `container_root`，并检查 `root` 是否选深了一层。
    //    两个值都只是**建议**：写入与否由前端一键确认，不悄悄改用户配置。
    let mut mapping = serde_json::Value::Null;
    let mut root_suggestion = serde_json::Value::Null;
    if kind == "bililive-tools" && root_ok {
        if let Some(c) = yekitrim_service::recorder::RecorderBackend::from_config(&cfg.ingest) {
            let save_path = c.save_path().await.unwrap_or_default();
            let samples: Vec<String> = match c.list_rooms().await {
                Ok(rooms) => match rooms.first() {
                    Some(r) => c
                        .list_history(&r.room_id.to_string(), &r.platform)
                        .await
                        .map(|items| items.into_iter().take(8).map(|i| i.video_file).collect())
                        .unwrap_or_default(),
                    None => Vec::new(),
                },
                Err(_) => Vec::new(),
            };
            if !samples.is_empty() {
                let root_path = std::path::PathBuf::from(&root);
                let (sp, sm) = (save_path.clone(), samples.clone());
                let derived = tokio::task::spawn_blocking(move || {
                    crate::ingest::derive_container_root(&root_path, &sp, &sm)
                })
                .await
                .unwrap_or_default();
                mapping = derived.to_json(&save_path);

                // 常见错法：`root` 填成了房间目录（Z:\Bilibili\耶斯yeki）。上一级能对上就建议改。
                if derived.matched == 0 {
                    if let Some(parent) = std::path::Path::new(&root).parent().map(|p| p.to_path_buf()) {
                        let (sp2, sm2, up_path) = (save_path.clone(), samples.clone(), parent.clone());
                        let up = tokio::task::spawn_blocking(move || {
                            crate::ingest::derive_container_root(&up_path, &sp2, &sm2)
                        })
                        .await
                        .unwrap_or_default();
                        if up.matched > 0 {
                            let mut j = up.to_json(&save_path);
                            j["path"] = serde_json::json!(yekitrim_core::path::display(&parent));
                            j["up"] = serde_json::json!(true);
                            root_suggestion = j;
                        }
                    }
                }
            }
        }
    }

    // `ok` = 三件事都成立（设置页据此决定弹"连接正常"还是出提示）
    ok(serde_json::json!({
        "ok": api_ok && root_ok,
        "kind": kind,
        "base": base,
        "version": version,
        "rooms": room_count,
        "api": { "ok": api_ok, "error": api_err },
        "root": root_json,
        "mapping": mapping,
        "root_suggestion": root_suggestion,
    }))
}

/// 目录校验：阻塞池 + 8s 超时。
async fn probe_root_with_timeout(root: String) -> serde_json::Value {
    if root.is_empty() {
        return yekitrim_service::recorder::probe_recorder_root("", true).to_json();
    }
    let path = root.clone();
    let probe = tokio::task::spawn_blocking(move || crate::ingest::root_probe_cached(&path, true));
    match tokio::time::timeout(std::time::Duration::from_secs(8), probe).await {
        Ok(Ok(p)) => p.to_json(),
        Ok(Err(e)) => probe_unavailable(&root, &format!("探测线程失败: {e}")),
        Err(_) => probe_unavailable(&root, "探测超时（网络路径可能不可达）"),
    }
}

/// 探测没跑出结论时的替身。
///
/// ⚠️ **不要再调一次探针**：那就等于在 super 慢的路径上再挂一遍（超时本来就是为了躲它）。
fn probe_unavailable(root: &str, why: &str) -> serde_json::Value {
    serde_json::json!({
        "path": root,
        "status": "missing",
        "ok": false,
        "unc": yekitrim_service::recorder::is_network_root(root),
        "readable": false,
        "writable": null,
        "entries": null,
        "os_error": why,
        "hint": "8 秒内没能打开这个目录：确认共享已开启、主机可达；Windows 上先用资源管理器访问一次。",
    })
}

/// `PUT /api/admin/invites/registration {enabled}`：注册总闸（关掉后不再受理注册请求）。
pub(super) async fn admin_registration_put(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<crate::auth::CurrentUser>,
    Json(body): Json<serde_json::Value>,
) -> ApiResult {
    let enabled = body
        .get("enabled")
        .and_then(|v| v.as_bool())
        .ok_or_else(|| err(StatusCode::BAD_REQUEST, "缺少 enabled"))?;
    yekitrim_service::invite::set_registration_enabled(enabled)
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("写入配置失败: {e:#}")))?;
    let _ = state.auth.db().audit(Some(user.id), "registration_switch", if enabled { "on" } else { "off" });
    ok(serde_json::json!({ "ok": true, "registration_enabled": enabled }))
}







#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::test_support::*;


    use axum::body::Body;
    use axum::http::Request as HttpRequest;
    use tower::util::ServiceExt;
/// 直播间接口：没对接录播姬时 `available=false`（**普通用户不返回管理员指引**），
/// 输入校验先于一切（假房间号 400），未对接时加房间 503 且不留登记。
#[tokio::test]
async fn rooms_endpoints_validate_and_report_availability() {
    let t = test_app().await;

    // admin：能看到"为什么不可用"（配置指引）
    let res = t.call(t.req("GET", "/rooms").body(Body::empty()).unwrap()).await;
    assert_eq!(res.status(), StatusCode::OK);
    let j = body_json(res).await;
    assert_eq!(j["available"], false, "{j}");
    assert!(j["reason"].as_str().unwrap_or("").contains("api_base"), "admin 应看到配置指引: {j}");
    assert_eq!(j["rooms"].as_array().unwrap().len(), 0);

    // 输入校验：链接里没有房间号 → 400（不碰录播姬、不留登记）
    let body = serde_json::json!({ "url": "https://example.com/abc" }).to_string();
    let res = t
        .call(
            t.req("POST", "/rooms")
                .header(axum::http::header::CONTENT_TYPE, "application/json")
                .body(Body::from(body))
                .unwrap(),
        )
        .await;
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);

    // 合法房间号但没对接录播姬 → 503，且不能留下半个登记
    let body = serde_json::json!({ "url": "https://live.bilibili.com/12345" }).to_string();
    let res = t
        .call(
            t.req("POST", "/rooms")
                .header(axum::http::header::CONTENT_TYPE, "application/json")
                .body(Body::from(body))
                .unwrap(),
        )
        .await;
    assert_eq!(res.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert!(t.auth.db().rooms_of_user(1).unwrap().is_empty(), "失败不该留下登记");

    // 普通用户：同样 200，但**不给**管理员向的 reason，也不泄露地址
    let hash = yekitrim_service::auth::hash_password("pass-12345678").unwrap();
    let uid = t.auth.db().create_user("plain2", &hash, "user", false, 0).unwrap();
    t.auth.db().room_upsert("777", uid, "他的房间").unwrap();
    let b = t.auth.login("plain2", "pass-12345678", Some("10.3.3.3"), None).unwrap();
    let res = t
        .app
        .clone()
        .oneshot(
            HttpRequest::builder()
                .method("GET")
                .uri("/rooms")
                .header(
                    axum::http::header::COOKIE,
                    format!("{}={}", yekitrim_service::auth::COOKIE_SID, b.session_id),
                )
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let j = body_json(res).await;
    assert_eq!(j["available"], false);
    assert!(j.get("reason").is_none(), "普通用户不显示管理员指引: {j}");
    assert_eq!(j["rooms"].as_array().unwrap().len(), 1, "只看得到自己的房间: {j}");
    assert_eq!(j["rooms"][0]["title"], "他的房间");
}

/// **B2（2026-11-14 后端 review）：recorder 两个端点的归属过滤（两用户）**。
///
/// 归属校验必须**先于**"录制软件可用性"检查 —— 否则普通用户能拿这个接口探
/// "某个房间号有没有被登记"（401/404/400 的差异就是存在性信号）。
#[tokio::test]
async fn recorder_recordings_ownership_gate_is_checked_before_anything_else() {
    let t = test_app().await;
    let db = t.auth.db();
    let hash = yekitrim_service::auth::hash_password("pass-12345678").unwrap();
    let a = db.create_user("room_a", &hash, "user", false, 0).unwrap();
    db.room_upsert("23058", a, "甲的房间").unwrap();
    let b = db.create_user("room_b", &hash, "user", false, 0).unwrap();
    let _ = b;
    let ba = t.auth.login("room_a", "pass-12345678", None, None).unwrap();
    let bb = t.auth.login("room_b", "pass-12345678", None, None).unwrap();

    // ① B 看 A 的房间：404（与 /api/rooms/:id/recordings 的 owned_room 同口径）
    let res = t
        .call(
            t.req_as(&bb.session_id, &bb.csrf, "GET", "/recorder/recordings?room_id=23058")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(res.status(), StatusCode::NOT_FOUND, "别人的房间不许看");

    // ② 没登记过的房间：普通用户同样 404（不许探存在性）
    let res = t
        .call(
            t.req_as(&bb.session_id, &bb.csrf, "GET", "/recorder/recordings?room_id=99999")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(res.status(), StatusCode::NOT_FOUND);

    // ③ 自己的房间：放行（测试环境没接录制软件 ⇒ 走到 configured:false 分支）
    let res = t
        .call(
            t.req_as(&ba.session_id, &ba.csrf, "GET", "/recorder/recordings?room_id=23058")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(res.status(), StatusCode::OK, "自己的房间要能看");
    let j = body_json(res).await;
    assert_eq!(j["configured"], false, "测试环境没有录制软件：{j}");

    // ④ admin：没登记的房间也放行（通吃，走到 configured:false）
    let res = t
        .call(t.req("GET", "/recorder/recordings?room_id=99999").body(Body::empty()).unwrap())
        .await;
    assert_eq!(res.status(), StatusCode::OK);
    let j = body_json(res).await;
    assert_eq!(j["configured"], false, "{j}");

    // ⑤ 普通用户 objects：可用位照回、objects 恒空（没有录制软件）且**不回 root 路径**
    let res = t
        .call(t.req_as(&bb.session_id, &bb.csrf, "GET", "/recorder/objects").body(Body::empty()).unwrap())
        .await;
    assert_eq!(res.status(), StatusCode::OK);
    let j = body_json(res).await;
    assert_eq!(j["configured"], false, "{j}");
}

/// B2 的纯函数判据：admin 通吃 / 普通用户只看自己登记的房间；
/// `added`/`project_id` 只回**本人**项目 —— 别人登记的 id 一个字都不回。
#[test]
fn recorder_visibility_and_project_id_are_owner_scoped() {
    use std::collections::HashSet;

    // 录制对象可见性
    assert!(recorder_room_visible(None, "123"), "admin 不过滤");
    let mine = HashSet::from(["123".to_string()]);
    assert!(recorder_room_visible(Some(&mine), "123"));
    assert!(!recorder_room_visible(Some(&mine), "456"), "别人的房间不可见");

    // added / project_id 按账号分
    let dir = tempfile::tempdir().unwrap();
    let db = yekitrim_service::db::Db::open(&dir.path().join("t.db")).unwrap();
    let a = db.create_user("a", "h", "user", false, 0).unwrap();
    let c = db.create_user("c", "h", "user", false, 0).unwrap();
    let proj = dir.path().join("a-proj");
    std::fs::create_dir_all(&proj).unwrap();
    let cfg_a = proj.join("yekitrim.toml");
    let pid_a = db
        .create_project(a, "a-proj", None, "local", None, &proj, &cfg_a, None)
        .unwrap();

    // 登记者本人：added:true + 自己的 id
    let (added, pid) = added_project_of(&db, false, a, &cfg_a);
    assert!(added);
    assert_eq!(pid, Some(pid_a));

    // 没登记的第三人：added:false + project_id:null（不泄露 A 的 id）
    let (added, pid) = added_project_of(&db, false, c, &cfg_a);
    assert!(!added, "别人的登记不该算到 C 头上");
    assert_eq!(pid, None, "别人的项目 id 不许回");

    // admin：回**任意人**的登记（他要能收拾全平台）
    let (added, pid) = added_project_of(&db, true, c, &cfg_a);
    assert!(added);
    assert_eq!(pid, Some(pid_a));
}

    /// **T9**：`RoomAddBody`（`POST /rooms`）—— `url` / `room_id` 二选一。
    ///
    /// 两个都缺 → 400 且**同时点名这两个字段**；`url` 类型错 → 400 点名 `url`。
    /// （改造前 `{"url": 5}` 会被 `as_str()` 静默当成空串，报的是「请填写直播间号或链接」，
    /// 用户看不出是自己把字段写成了数字。）
    #[tokio::test]
    async fn room_add_body_is_typed_and_names_the_field() {
        let t = test_app().await;

        let (status, text) = call_json(&t, "POST", "/rooms", serde_json::json!({})).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "url/room_id 都没给必须 400：{text}");
        assert!(text.contains("url"), "报错要点名 url：{text}");
        assert!(text.contains("room_id"), "报错要点名 room_id：{text}");

        let (status, text) = call_json(&t, "POST", "/rooms", serde_json::json!({ "url": 5 })).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "url 类型错必须 400：{text}");
        assert!(text.contains("url"), "报错要点名 url：{text}");
    }

    /// biliLive-tools 的路径解析：**整批解析 + 负结果也缓存**（历史里有大量已删场次）。
    /// 缓存命中后即使文件被删也返回旧结论 —— 这条钉的是「5 分钟内不重复摸盘」。
    #[test]
    fn blt_paths_resolve_in_one_batch_and_cache_negative_results() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join("Sinsa")).unwrap();
        std::fs::write(root.join("Sinsa").join("a.flv"), b"x").unwrap();

        let good = "/app/video/Bilibili/Sinsa/a.flv".to_string();
        let missing = "/app/video/Bilibili/Sinsa/none.flv".to_string();
        let paths = vec![good.clone(), missing.clone()];
        let m = resolve_blt_paths(root, "/app/video/Bilibili", &paths);
        assert!(matches!(m.get(&good), Some(Some(_))), "存在的文件要解析出来：{m:?}");
        assert!(matches!(m.get(&missing), Some(None)), "不存在的要回 None：{m:?}");

        // 文件被删掉之后仍然命中缓存（否则每次点房间都要把上百次失败重跑一遍）
        std::fs::remove_file(root.join("Sinsa").join("a.flv")).unwrap();
        let again = resolve_blt_paths(root, "/app/video/Bilibili", &paths);
        assert!(matches!(again.get(&good), Some(Some(_))), "5 分钟内必须命中缓存");
    }

    /// **自动推导 `container_root`**：savePath 之下挑「命中最多、同分取最浅」，
    /// 且命中的本地路径要能回显给用户看（"我们看到的是这条"）。
    #[test]
    fn derive_container_root_picks_the_shallowest_prefix_that_matches() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join("Sinsa").join("sess")).unwrap();
        std::fs::write(root.join("Sinsa").join("sess").join("a.flv"), b"x").unwrap();
        std::fs::create_dir_all(root.join("乙房间")).unwrap();
        std::fs::write(root.join("乙房间").join("b.flv"), b"x").unwrap();

        let samples = vec![
            "/app/video/Bilibili/Sinsa/sess/a.flv".to_string(),
            "/app/video/Bilibili/乙房间/b.flv".to_string(),
            "/app/video/Bilibili/没这个/b.flv".to_string(),
        ];
        let m = crate::ingest::derive_container_root(root, "/app/video", &samples);
        assert_eq!(m.container_root, "/app/video/Bilibili", "{m:?}");
        assert_eq!((m.matched, m.probed), (2, 3), "{m:?}");
        assert!(
            m.sample_local.as_deref().unwrap_or("").ends_with("a.flv"),
            "命中样本要能回显：{m:?}"
        );
    }
    /// **目录不可用时的降级文案**：admin 看得到路径/OS 错误 + 探针提示，普通用户只有一句中性话。
    #[test]
    fn degraded_recordings_hides_paths_from_normal_users() {
        let mut cfg = yekitrim_core::config::Config::default();
        cfg.ingest.recorder.root = "/definitely/not/here".into();
        let raw = "目录不存在: /definitely/not/here";
        let admin = degraded_recordings(&cfg, true, "23058", raw);
        assert_eq!(admin["available"], false);
        assert!(admin["error"].as_str().unwrap().contains("definitely"), "{admin}");

        let user = degraded_recordings(&cfg, false, "23058", raw);
        assert_eq!(user["available"], false);
        assert!(!user["error"].as_str().unwrap().contains("definitely"), "普通用户不许看到路径：{user}");
    }

    /// **测试连接**：接口与目录**分开报**，且 `root` 能测还没保存的输入。
    #[tokio::test]
    async fn recorder_test_reports_root_separately() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.flv"), b"x").unwrap();
        let body = axum::body::Bytes::from(
            serde_json::json!({ "root": dir.path().to_string_lossy() }).to_string(),
        );
        let v = admin_recorder_test(body).await.expect("handler").0;
        assert_eq!(v["root"]["ok"], true, "{v}");
        assert_eq!(v["root"]["writable"], true, "{v}");
        assert_eq!(v["api"]["ok"], false, "测试环境没有 api_base：{v}");
        assert_eq!(v["ok"], false, "接口没通时总 ok 必须为假：{v}");

        let missing = dir.path().join("nope");
        let body = axum::body::Bytes::from(
            serde_json::json!({ "root": missing.to_string_lossy() }).to_string(),
        );
        let v = admin_recorder_test(body).await.expect("handler").0;
        assert_eq!(v["root"]["ok"], false, "{v}");
        assert!(
            !v["root"]["hint"].as_str().unwrap_or("").is_empty(),
            "失败必须给下一步：{v}"
        );
    }

    /// 目录探针 60s 内复用结果 —— 否则设置页每次轮询都在打网络盘（死 UNC 能挂几十秒）。
    #[test]
    fn root_probe_cache_reuses_the_last_result() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.flv"), b"x").unwrap();
        let p1 = crate::ingest::root_probe_cached(&dir.path().to_string_lossy(), false);
        assert_eq!(p1.entries, Some(1));
        std::fs::remove_file(dir.path().join("a.flv")).unwrap();
        let p2 = crate::ingest::root_probe_cached(&dir.path().to_string_lossy(), false);
        assert_eq!(p2.entries, Some(1), "60s 内必须复用缓存，别再摸一次盘");
    }
}
