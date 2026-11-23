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
    for rid in room_ids {
        let platform = live
            .iter()
            .find(|r| r.room_id.to_string() == *rid)
            .map(|r| r.platform.clone())
            .unwrap_or_else(|| "Bilibili".into());
        let Ok(items) = client.list_history(rid, &platform).await else { continue };
        let paths: Vec<String> = items.into_iter().map(|i| i.video_file).collect();
        let root = root.clone();
        let cr = container_root.clone();
        let stat = tokio::task::spawn_blocking(move || {
            let mut s = crate::ingest_worker::RoomRecordings::default();
            for p in paths {
                let Ok(abs) = crate::ingest::map_webhook_path(&root, &cr, &p) else { continue };
                let Ok(md) = std::fs::metadata(&abs) else { continue };
                s.files += 1;
                s.bytes += md.len();
                let mtime = md
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_secs() as i64)
                    .unwrap_or(0);
                s.latest = s.latest.max(mtime);
            }
            s
        })
        .await;
        if let Ok(s) = stat {
            out.insert(rid.clone(), s);
        }
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
                None
            }
            Err(e) => {
                tracing::warn!("读录像目录线程失败: {e}");
                None
            }
        }
    };

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
        ok(serde_json::json!({ "available": available, "reason": reason, "kind": kind, "rooms": rooms }))
    } else {
        ok(serde_json::json!({ "available": available, "kind": kind, "rooms": rooms }))
    }
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
    let input = body
        .get("url")
        .or_else(|| body.get("room_id"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
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

    let mut objects = Vec::new();
    for r in rooms {
        let rid = r.room_id.to_string();
        // B2：不在本人 channel_map 里的房间，一个字都不回（不泄露目录在录什么）
        if !recorder_room_visible(mine.as_ref(), &rid) {
            continue;
        }
        let mut dir = String::new();
        let mut files = 0usize;
        let mut bytes = 0u64;
        let mut latest = 0i64;
        if blt {
            if !root.as_os_str().is_empty() {
                if let Ok(items) = client.list_history(&rid, &r.platform).await {
                    for it in items {
                        let Ok(abs) = crate::ingest::map_webhook_path(&root, &container_root, &it.video_file)
                        else { continue };
                        if dir.is_empty() {
                            dir = abs.parent().map(|p| yekitrim_core::path::display(p)).unwrap_or_default();
                        }
                        if let Ok(md) = std::fs::metadata(&abs) {
                            files += 1;
                            bytes += md.len();
                            latest = latest.max(
                                md.modified()
                                    .ok()
                                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                                    .map(|d| d.as_secs() as i64)
                                    .unwrap_or(0),
                            );
                        }
                    }
                }
            }
        } else if let Some(files_ref) = scan.as_ref() {
            for f in files_ref.iter().filter(|f| f.room.as_deref() == Some(rid.as_str())) {
                if dir.is_empty() {
                    dir = root.join(&f.rel).parent().map(|p| yekitrim_core::path::display(p)).unwrap_or_default();
                }
                files += 1;
                bytes += f.size;
                latest = latest.max(f.mtime);
            }
        }
        objects.push(serde_json::json!({
            "id": rid,
            "title": r.title,
            "platform": r.platform,
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
        for it in items {
            let Ok(abs) = crate::ingest::map_webhook_path(&root, &container_root, &it.video_file) else {
                continue;
            };
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
            let Ok(md) = std::fs::metadata(&abs) else { continue };
            let Some(dir) = abs.parent() else { continue };
            let mtime = md
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0);
            let e = agg.entry(dir.to_path_buf()).or_insert((0, 0));
            e.0 += md.len();
            e.1 = e.1.max(mtime);
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
        let scanned = tokio::task::spawn_blocking(move || crate::ingest_worker::list_recordings(&cfg_scan))
            .await
            .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("扫描线程失败: {e}")))?
            .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}")))?;
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

/// `POST /api/admin/recorder/test`：对接自检（版本 + 房间数），admin 用来看配置对不对。
///
/// ⚠️ **body 里可以带未保存的当前输入**（2026-11-24）：前端「测试连接」原先只测**已存**配置，
/// 于是「刚改完密钥 → 点测试 → 报 401」实际验的是旧值，锅被甩给"网络/对方服务"（实测踩过：
/// biliLive-tools 的 passKey 被 `BILILIVE_TOOLS_PASSKEY` 环境变量覆盖，界面显示的是文件里那份）。
/// 现在 body 里给了 `kind`/`api_base`/`api_key`/`api_user`/`api_pass` 就**临时覆盖**（不落盘），
/// 没给的沿用已存值。⚠️ 密钥只走 **body**，别塞 query —— `access_log` 会把 URL 原样记进日志。
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
    let client = yekitrim_service::recorder::RecorderBackend::from_config(&cfg.ingest);
    let Some(client) = client else {
        return ok(serde_json::json!({
            "ok": false,
            "error": "未配置 [ingest.recorder] api_base",
        }));
    };
    let kind = client.kind();
    let version = client.version().await.ok();
    match client.list_rooms().await {
        Ok(rooms) => ok(serde_json::json!({
            "ok": true,
            "kind": kind,
            "base": client.base(),
            "version": version,
            "rooms": rooms.len(),
        })),
        Err(e) => ok(serde_json::json!({
            "ok": false,
            "kind": kind,
            "base": client.base(),
            "version": version,
            "error": format!("{e:#}"),
        })),
    }
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
}
