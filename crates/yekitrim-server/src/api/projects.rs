//! `api::projects`：项目托管（登记 / 建项目 / 注销 / 删除源）。

use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::{
    extract::{Extension, Path as AxumPath, State},
    http::StatusCode,
    Json,
};
use yekitrim_service::Workspace;

use crate::model::AppState;
use crate::persist;
use super::*;

/// 打开/新建项目后**只记一笔"上次用的是哪个工程"**（`server.toml` 的 `last_config_file`）。
///
/// ⚠️ **这里曾经还往 `AppState` 里塞一份"全局当前工程"**（`set_workspace`），2026-10-03 连同
/// 那份状态一起删了：每个请求本来就会经 `scope::Scope` 按 `?project=<id>` 现场解析出
/// 自己那份 `Workspace`，全局那份**没有任何读取方**，只会跟着"上次打开的工程"漂。
///
/// 留下的这一笔**还有用**：`yekitrim server` 不带 `--config` 启动时靠它恢复上次的工程
/// （`lib.rs` 的启动流程 + `persist::last_config_file`）。它**只是记忆、不是状态** ——
/// 记忆允许过期（文件没了 `last_config_file()` 自己会过滤掉），状态不行。
pub(super) fn remember_last_project(ws: &Workspace) {
    persist::remember_last(&ws.config_file);
}

/// 旧版项目摘要（`server.toml` 时代的列表形态）。
///
/// P2 起项目列表走 DB（`/api/projects`），这个函数只在过渡期别名的 stub 里出现，
/// 保留是为了让 `yekitrim status` 与旧前端 JSON 结构继续可用。
#[allow(dead_code)]
pub(super) fn project_summary(ws: &Workspace) -> serde_json::Value {
    yekitrim_service::project_summary(ws)
}

// ---- 项目托管（P2，§5 / §8） ----

/// 取项目并校验归属；他人项目一律 404（不泄露"存在但无权限"）。
pub(super) fn owned_project(
    db: &yekitrim_service::db::Db,
    user: &crate::auth::CurrentUser,
    id: i64,
) -> Result<yekitrim_service::db::ProjectRow, AppError> {
    match yekitrim_service::projects::find_owned(db, user.id, user.is_admin(), id) {
        Ok(Some(p)) => Ok(p),
        Ok(None) => Err(err(StatusCode::NOT_FOUND, "项目不存在或无权访问")),
        Err(e) => Err(err(StatusCode::INTERNAL_SERVER_ERROR, format!("读取项目失败: {e:#}"))),
    }
}

pub(super) fn owner_name(db: &yekitrim_service::db::Db, owner_id: i64) -> Option<String> {
    db.list_users()
        .ok()?
        .into_iter()
        .find(|u| u.id == owner_id)
        .map(|u| u.username)
}

/// `GET /api/projects`：只列本人项目；admin 额外看到所有人的（含 owner 列）。
pub(super) async fn projects_list(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<crate::auth::CurrentUser>,
) -> ApiResult {
    let db = state.auth.db();
    // 始终只回本人项目：admin 的"全平台项目"视图属于 P5 管理页（`/api/admin/*`）
    let rows = db
        .list_projects(Some(user.id))
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("读取项目列表失败: {e:#}")))?;

    // 每个项目的摘要都要读它的 `yekitrim.toml` + 对产物目录做十几次 `exists()`
    // （`summary_json` → `project_snapshot`）。项目在网络盘上时这就是十几秒级的往返，
    // 而前端一次页面加载会成串地打这个接口 —— 整段算在 blocking 池里，别占 worker。
    let st = state.clone();
    let out = blocking(move || {
        let db = st.auth.db();
        let out: Vec<serde_json::Value> = rows
            .iter()
            .map(|p| {
                let json = yekitrim_service::projects::summary_json(p, owner_name(db, p.owner_id).as_deref());
                // 项目目录/yekitrim.toml 被外部删掉时落库标记，列表里显示"缺失"而不是静默消失
                if json["missing"].as_bool() == Some(true) && !p.missing {
                    let _ = db.set_project_missing(p.id, true);
                }
                json
            })
            .collect();
        Ok(out)
    })
    .await?;
    ok(serde_json::json!({ "ok": true, "projects": out }))
}

/// `POST /api/projects`：新建项目。
///
/// 目前只支持 `source.kind = "local"`（**仅 admin**，§0.2.7：单人/本机部署保留
/// "服务器端已有文件"来源，路径囚笼在租户根内，**永不接受上传**）。
///
/// ⚠️ `recorder` / `bilibili` **不该走这个端点**：它们建项目的时间点不是"用户点新建"，
/// 而是**素材落位之后**（`ingest_worker` / `download_worker` 里调
/// `yekitrim_service::ingest::landing`）。在这里放行只会建出一个**没有视频的空项目**。
/// 提交入口在 `POST /api/ingest/bilibili`。
pub(super) async fn projects_create(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<crate::auth::CurrentUser>,
    Json(body): Json<serde_json::Value>,
) -> ApiResult {
    let source = body.get("source").cloned().unwrap_or(serde_json::Value::Null);
    let kind = source.get("kind").and_then(|v| v.as_str()).unwrap_or("");
    match kind {
        "local" => {}
        "bilibili" => {
            return Err(err(
                StatusCode::BAD_REQUEST,
                "B站链接请用「B站链接」提交入口（POST /api/ingest/bilibili）：\
                 下载完成后会自动建项目，不在这里建",
            ))
        }
        "recorder" => {
            return Err(err(
                StatusCode::BAD_REQUEST,
                "录播姬来源的项目由采集入库自动创建（POST /api/ingest/bililive/<token>）；\
                 请到「采集记录」页指派与查看",
            ))
        }
        other => {
            return Err(err(
                StatusCode::BAD_REQUEST,
                format!("未知来源 kind={other:?}（支持 local）"),
            ))
        }
    }
    if !user.is_admin() {
        return Err(err(StatusCode::FORBIDDEN, "仅管理员可指定服务器本机文件"));
    }
    create_local_project(
        &state,
        &user,
        source.get("root").and_then(|v| v.as_str()).unwrap_or(""),
        source.get("dir").and_then(|v| v.as_str()).unwrap_or(""),
        source.get("input").and_then(|v| v.as_str()).unwrap_or(""),
        source.get("danmaku").and_then(|v| v.as_str()),
        source.get("output_dir").and_then(|v| v.as_str()),
    )
    .await
}

/// 用"服务器本机文件"建项目的公共实现（`POST /api/projects` 与过渡期的
/// `POST /api/project/init` 共用）。
///
/// 管理员可以给**任意绝对目录**（与单用户时代一致），非管理员只能是租户根内的相对路径；
/// 目录里已经存在 `yekitrim.toml` 时**只登记/打开，不覆盖**（避免把已有工程的设置冲掉）。
pub(super) async fn create_local_project(
    state: &Arc<AppState>,
    user: &crate::auth::CurrentUser,
    root_hint: &str,
    rel_dir: &str,
    input_hint: &str,
    danmaku_hint: Option<&str>,
    output_dir: Option<&str>,
) -> ApiResult {
    let tenant = yekitrim_service::projects::tenant_root(user.id);
    std::fs::create_dir_all(&tenant)
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("创建租户目录失败: {e}")))?;
    let dir = yekitrim_service::projects::resolve_target_dir(
        user.id,
        root_hint,
        rel_dir,
        user.is_admin(),
    )
    .map_err(|e| err(StatusCode::BAD_REQUEST, format!("目录不可用: {e:#}")))?;
    if !dir.is_dir() {
        return Err(err(StatusCode::BAD_REQUEST, "只能选择目录"));
    }
    let config_path = dir.join(yekitrim_core::path::PROJECT_CONFIG_FILE);
    let is_existing_project = config_path.is_file();

    // 已有 yekitrim.toml：只登记/打开，不重写（否则会覆盖用户手改过的工程设置）
    if !is_existing_project {
        let hint = input_hint.trim();
        let input = if hint.is_empty() {
            // 未指定文件时自动挑目录里第一个视频
            find_first_video(&dir).ok_or_else(|| {
                err(StatusCode::BAD_REQUEST, "目录里没有可用的视频文件（mp4/flv/mkv/ts）")
            })?
        } else {
            yekitrim_core::tenant::jail_existing(&dir, hint)
                .map_err(|e| err(StatusCode::BAD_REQUEST, format!("视频不可用: {e:#}")))?
        };
        let danmaku = danmaku_hint
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .map(|rel| {
                yekitrim_core::tenant::jail_existing(&dir, &rel)
                    .map_err(|e| err(StatusCode::BAD_REQUEST, format!("弹幕不可用: {e:#}")))
            })
            .transpose()?;
        let output_rel = output_dir.map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
        if let Some(rel) = &output_rel {
            yekitrim_core::tenant::jail_join(&dir, rel)
                .map_err(|e| err(StatusCode::BAD_REQUEST, format!("输出目录不可用: {e:#}")))?;
        }
        // `local` 来源按策略**不预载弹幕**（用户要求"自由选择"）：没显式给弹幕就留空，
        // 由预处理页列候选让用户自己挑（`yekitrim_service::assets::preload_policy`）。
        // 所以这里用 `_exact`，不做"同目录同名 .ass"自动发现 —— 那条是 `yekitrim init` 的语义。
        let auto_danmaku = matches!(
            yekitrim_service::assets::preload_policy("local"),
            yekitrim_service::assets::Preload::Danmaku
        );
        let write_cfg: fn(&Path, &Path, Option<&Path>, Option<&str>) -> anyhow::Result<()> = if auto_danmaku {
            yekitrim_core::config::write_project_config
        } else {
            yekitrim_core::config::write_project_config_exact
        };
        write_cfg(&config_path, &input, danmaku.as_deref(), output_rel.as_deref())
            .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("初始化失败: {e:#}")))?;
    }
    let ws = Workspace::load(Some(&config_path))
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("加载失败: {e:#}")))?;

    let db = state.auth.db();
    let name = dir
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "project".to_string());
    let in_tenant = dir.starts_with(&tenant);
    let label = if in_tenant {
        dir.strip_prefix(&tenant)
            .ok()
            .map(|rel| format!("本机文件 · {}", rel.display()))
            .unwrap_or_else(|| "本机文件".to_string())
    } else {
        // 任意位置（管理员）：用父目录名 + 目录名拼一个能认出来的标签
        let parent_name = dir
            .parent()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        format!("本机 · {parent_name}/{}", name)
    };
    let id = match db.project_by_config(&config_path) {
        Ok(Some(existing)) => existing.id,
        _ => db
            .create_project(
                user.id,
                &name,
                None,
                "local",
                Some(&label),
                &dir,
                &config_path,
                None,
            )
            .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("登记项目失败: {e:#}")))?,
    };
    if let Ok(Some(p)) = db.project_by_id(id) {
        let _ = yekitrim_service::projects::refresh_bytes(db, &p);
    }
    let _ = db.audit(Some(user.id), "project_create", &format!("id={id} kind=local"));
    remember_last_project(&ws);
    match db.project_by_id(id) {
        Ok(Some(p)) => ok(serde_json::json!({
            "ok": true,
            "id": id,
            "project": yekitrim_service::projects::summary_json(&p, Some(&user.username)),
        })),
        _ => Err(err(StatusCode::INTERNAL_SERVER_ERROR, "项目登记后读取失败")),
    }
}

/// 目录里第一个可用视频（mp4/flv/mkv/ts，按文件名排序，确定性）。
pub(super) fn find_first_video(dir: &Path) -> Option<PathBuf> {
    let mut candidates: Vec<PathBuf> = std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.is_file()
                && p.extension()
                    .and_then(|e| e.to_str())
                    .map(|e| matches!(e.to_ascii_lowercase().as_str(), "mp4" | "flv" | "mkv" | "ts"))
                    .unwrap_or(false)
        })
        .collect();
    candidates.sort();
    candidates.into_iter().next()
}

/// 注销项目绑定：**只动服务器的账本与状态文件，项目目录里一个文件都不碰**。
///
/// 两个入口（`DELETE /api/projects/:id` 与过渡别名 `POST /project/delete`）都走这里 ——
/// 这段逻辑以前是两份拷贝，两边各删一次 `yekitrim.toml`；要改就一起改，否则会各处半对。
///
/// ⚠️ `forget_project` 会**同时清掉 `last_config_file`**，这一步不是顺手做的：
/// 启动流程每次都会把"当前工程"重新写回 `projects[]`（`lib.rs`），
/// 而 `import_legacy_projects` 会把 `projects[]` 里"文件还在、DB 没行"的路径重新登记 ——
/// 光删 DB 行的话，移除过的项目会在**第二次重启**时自己回来（详见 `persist::forget_project`）。
///
/// 媒体文件与 `output/` 产物一律保留（那是「删除源录播」那条独立路径的事）。
pub(super) fn unregister_project(
    db: &yekitrim_service::db::Db,
    user_id: i64,
    project: &yekitrim_service::db::ProjectRow,
) -> ApiResult {
    db.delete_project(project.id)
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("注销失败: {e:#}")))?;
    persist::forget_project(&project.config_file);
    let _ = db.audit(Some(user_id), "project_unregister", &format!("id={}", project.id));
    ok(serde_json::json!({ "ok": true, "deleted": project.id }))
}

/// `DELETE /api/projects/:id`：**只注销登记**，项目目录里零文件改动（媒体/产物/`yekitrim.toml` 全保留）。
///
/// 想再挂回来就重新导入同一个目录（`POST /api/projects`）—— 目录里已有 `yekitrim.toml` 时
/// 只登记、不覆盖（见 `create_local_project`），所以手改过的工程设置不会丢。
pub(super) async fn projects_delete(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<crate::auth::CurrentUser>,
    AxumPath(id): AxumPath<String>,
) -> ApiResult {
    let id = yekitrim_service::projects::parse_project_id(&id)
        .ok_or_else(|| err(StatusCode::BAD_REQUEST, "项目 id 无效"))?;
    let db = state.auth.db();
    let project = owned_project(db, &user, id)?;
    unregister_project(db, user.id, &project)
}

/// `DELETE /api/projects/:id/source`：删源录播（**唯一的手动释放空间手段**，§7）。
///
/// 产物（切片/字幕/事件/封面）与项目本身都保留；只删 `input_file` 与根目录下的
/// 分片/弹幕文件，并刷新占用。
pub(super) async fn projects_source_delete(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<crate::auth::CurrentUser>,
    AxumPath(id): AxumPath<String>,
) -> ApiResult {
    let id = yekitrim_service::projects::parse_project_id(&id)
        .ok_or_else(|| err(StatusCode::BAD_REQUEST, "项目 id 无效"))?;
    let db = state.auth.db();
    let project = owned_project(db, &user, id)?;

    let mut removed: Vec<String> = Vec::new();
    let mut freed = 0u64;
    let ws = Workspace::load(Some(&project.config_file)).ok();
    let mut targets: Vec<PathBuf> = Vec::new();
    if let Some(ws) = &ws {
        if let Ok(cfg) = ws.load_config() {
            for raw in [cfg.project.input_file.clone().unwrap_or_default(), cfg.project.danmaku_file.clone().unwrap_or_default()] {
                if !raw.trim().is_empty() {
                    targets.push(ws.resolve(&raw));
                }
            }
        }
    }
    // 根目录下的分片（录播姬多分片会话）也算源
    if let Ok(entries) = std::fs::read_dir(&project.root_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
            if matches!(ext.as_str(), "mp4" | "flv" | "mkv" | "ts") {
                targets.push(path);
            }
        }
    }
    // 去重后逐个删除（只删项目根目录内的文件）
    let mut seen: std::collections::HashSet<PathBuf> = std::collections::HashSet::new();
    let root = project.root_dir.canonicalize().unwrap_or_else(|_| project.root_dir.clone());
    for target in targets {
        let real = target.canonicalize().unwrap_or_else(|_| target.clone());
        if !seen.insert(real.clone()) {
            continue;
        }
        if !real.starts_with(&root) {
            tracing::warn!("跳过项目目录外的源文件: {}", real.display());
            continue;
        }
        if !real.is_file() {
            continue;
        }
        let size = std::fs::metadata(&real).map(|m| m.len()).unwrap_or(0);
        match std::fs::remove_file(&real) {
            Ok(()) => {
                freed += size;
                removed.push(real.to_string_lossy().into_owned());
            }
            Err(e) => tracing::warn!("删除源文件失败 {}: {e}", real.display()),
        }
    }
    let (source_bytes, output_bytes) = yekitrim_service::projects::refresh_bytes(db, &project)
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("刷新占用失败: {e:#}")))?;
    let _ = db.audit(
        Some(user.id),
        "project_source_delete",
        &format!("id={id} files={} bytes={freed}", removed.len()),
    );
    ok(serde_json::json!({
        "ok": true,
        "removed": removed,
        "freed_bytes": freed,
        "source_bytes": source_bytes,
        "output_bytes": output_bytes,
    }))
}

/// **过渡期别名**：旧前端按 `config_path` 注销项目；只允许操作**已登记且属于自己**的
/// `yekitrim.toml`（P2 之后前端改用 `DELETE /api/projects/:id`）。
///
/// 与 `DELETE /api/projects/:id` **是同一件事**，共用 `unregister_project`
/// —— 别再在这里单独写一份"删除"（以前两份拷贝各删一次 `yekitrim.toml`）。
pub(super) async fn project_delete(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<crate::auth::CurrentUser>,
    Json(body): Json<serde_json::Value>,
) -> ApiResult {
    let raw = body
        .get("config_path")
        .and_then(|v| v.as_str())
        .ok_or_else(|| err(StatusCode::BAD_REQUEST, "缺少 config_path"))?;
    let path = PathBuf::from(raw);
    let is_project_file = path
        .file_name()
        .and_then(|n| n.to_str())
        .map(|n| n.eq_ignore_ascii_case(yekitrim_core::path::PROJECT_CONFIG_FILE))
        .unwrap_or(false);
    if !is_project_file {
        return Err(err(StatusCode::BAD_REQUEST, "只允许操作 yekitrim.toml"));
    }
    let db = state.auth.db();
    let project = db
        .project_by_config(&path)
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("读取项目失败: {e:#}")))?
        .filter(|p| user.is_admin() || p.owner_id == user.id)
        .ok_or_else(|| err(StatusCode::NOT_FOUND, "项目不存在或无权访问"))?;
    unregister_project(db, user.id, &project)
}

/// **过渡期别名**：旧前端的项目列表（`/api/projects` 的早期形态）。
/// 现在直接读 DB 里**自己的**项目，不再扫描磁盘或读取 `server.toml`。
///
/// ⚠️ 响应里的 `workspace_root` 字段 2026-10-03 删了 —— 它就来自那份已删掉的全局工程状态，
/// 前端没有任何消费方（`/api/project/list` 全仓只剩这个 deprecated 别名在用）。
pub(super) async fn project_list(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<crate::auth::CurrentUser>,
) -> ApiResult {
    let db = state.auth.db();
    let rows = db
        .list_projects(Some(user.id))
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("读取项目列表失败: {e:#}")))?;
    // 同 `/api/projects`：摘要是对项目目录的同步 IO，下 blocking 池
    let user_name = user.username.clone();
    let out = blocking(move || {
        Ok(rows
            .iter()
            .map(|p| yekitrim_service::projects::summary_json(p, Some(&user_name)))
            .collect::<Vec<serde_json::Value>>())
    })
    .await?;
    ok(serde_json::json!({
        "projects": out,
        "deprecated": "请改用 GET /api/projects（本项目列表只回本人项目）",
    }))
}







pub(super) fn project_snapshot(ws: &Workspace) -> serde_json::Value {
    yekitrim_service::project_snapshot(ws)
}

pub(super) fn project_status_impl(ws: &Workspace) -> ApiResult {
    ok(project_snapshot(ws))
}

/// **过渡期别名**（P2 之后前端改用 `POST /api/projects {source:{kind:"local"}}`）：
/// 旧前端传的是绝对 `input`/`config_path`，这里把它当成"相对租户根"的提示，
/// 走同一条囚笼 + 登记流程，**不再接受任意绝对路径**。
pub(super) async fn project_init(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<crate::auth::CurrentUser>,
    Json(body): Json<serde_json::Value>,
) -> ApiResult {
    if !user.is_admin() {
        return Err(err(StatusCode::FORBIDDEN, "仅管理员可指定服务器本机文件"));
    }
    let raw = body
        .get("input")
        .or_else(|| body.get("config_path"))
        .or_else(|| body.get("dir"))
        .and_then(|v| v.as_str())
        .ok_or_else(|| err(StatusCode::BAD_REQUEST, "缺少 input"))?;
    let tenant = yekitrim_service::projects::tenant_root(user.id);
    let abs = PathBuf::from(raw);
    // 旧值可能是绝对路径：只接受"落在自己租户根内"的那种，其余按相对路径处理
    let rel_dir = if abs.is_absolute() {
        let parent = if abs.is_dir() { abs.clone() } else { abs.parent().map(|p| p.to_path_buf()).unwrap_or_else(|| tenant.clone()) };
        match yekitrim_core::tenant::ensure_within(&tenant, &parent) {
            Ok(inside) => inside
                .strip_prefix(&tenant)
                .map(|p| p.to_string_lossy().replace('\\', "/"))
                .unwrap_or_default(),
            Err(_) => raw.to_string(),
        }
    } else {
        raw.to_string()
    };
    let danmaku = body.get("danmaku").and_then(|v| v.as_str());
    let output = body.get("output_dir").and_then(|v| v.as_str());
    let created = create_local_project(&state, &user, "", &rel_dir, "", danmaku, output).await?;
    // 旧前端期待项目快照（含 current_stage / files）
    let ws = match created.0.get("id").and_then(|v| v.as_i64()).and_then(|id| state.auth.db().project_by_id(id).ok().flatten()) {
        Some(p) => Workspace::load(Some(&p.config_file))
            .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("加载失败: {e:#}")))?,
        None => return Err(err(StatusCode::INTERNAL_SERVER_ERROR, "项目登记后读取失败")),
    };
    ok(project_snapshot(&ws))
}

/// **过渡期别名**：按 `config_path`（已登记的本人项目）激活工作区并回项目快照。
pub(super) async fn project_open(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<crate::auth::CurrentUser>,
    Json(body): Json<serde_json::Value>,
) -> ApiResult {
    let raw = body.get("config_path").and_then(|v| v.as_str()).unwrap_or("").trim().to_string();
    let by_id = body.get("project").and_then(|v| v.as_i64());
    let project = match (by_id, raw.is_empty()) {
        (Some(id), _) => yekitrim_service::projects::find_owned(state.auth.db(), user.id, user.is_admin(), id)
            .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("读取项目失败: {e:#}")))?,
        (None, false) => state
            .auth
            .db()
            .project_by_config(Path::new(&raw))
            .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("读取项目失败: {e:#}")))?
            .filter(|p| user.is_admin() || p.owner_id == user.id),
        _ => None,
    };
    let Some(project) = project else {
        return Err(err(
            StatusCode::NOT_FOUND,
            "项目不存在或无权访问（请用 /api/projects 的项目 id）",
        ));
    };
    let ws = Workspace::load(Some(&project.config_file))
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("加载失败: {e:#}")))?;
    remember_last_project(&ws);
    ok(project_snapshot(&ws))
}







#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::test_support::*;


    use axum::body::Body;

/// 移除项目 = **只注销绑定**：目录里一个文件都不动，而且能重新导入同一个目录。
///
/// 回归：这条路以前会 `remove_file(yekitrim.toml)`（`projects_delete` 与过渡别名各删一次）。
/// 留着 `yekitrim.toml` 也正是重新绑定的前提 —— `create_local_project` 见到已有
/// `yekitrim.toml` 只登记、不覆盖，所以手改过的工程设置不会丢。
///
/// ⚠️ 本用例会经 `forget_project` 写一份 `server.toml`，路径是
/// `<测试可执行文件旁>/.yekitrim/`（即 `target/debug/deps/.yekitrim/`）——
/// 与开发实例的 `target/debug/.yekitrim/` 不在一处，不会干扰本机跑着的服务。
#[tokio::test]
async fn unregister_keeps_every_file_and_can_rebind() {
    let t = test_app().await;
    let cfg = t.root.join("yekitrim.toml");
    let cfg_before = std::fs::read_to_string(&cfg).unwrap();
    // 造一个"产物"：移除后它必须还在
    let clip = t.root.join("output").join("clips").join("clip-01").join("clip.mp4");
    std::fs::create_dir_all(clip.parent().unwrap()).unwrap();
    std::fs::write(&clip, b"clip-bytes").unwrap();

    // 先取一次基线：列表里确实有它（否则"注销后不在列表里"这句会因为接口返回空而白通过）
    let before = project_ids(&t).await;
    assert!(before.contains(&1), "注销前应该能看到项目 1: {before:?}");

    let res = t
        .call(t.req("DELETE", "/projects/1").body(Body::empty()).unwrap())
        .await;
    assert_eq!(res.status(), StatusCode::OK);

    // ① 零文件改动：配置 / 源媒体 / 产物全部原样
    assert!(cfg.exists(), "yekitrim.toml 必须留着（移除 ≠ 删目录）");
    assert_eq!(std::fs::read_to_string(&cfg).unwrap(), cfg_before, "内容也不该变");
    assert!(t.root.join("a.mp4").exists(), "源媒体必须留着");
    assert!(clip.exists(), "产物必须留着");

    // ② 服务器列表里已经没有它了
    let after = project_ids(&t).await;
    assert!(!after.contains(&1), "注销后不该再出现在列表里: {after:?}");

    // ③ 重新导入同一个目录 → 重新绑定成功，且**不覆盖**已有配置
    let body = serde_json::json!({
        "source": { "kind": "local", "dir": yekitrim_core::path::display(&t.root) }
    });
    let res = t
        .call(
            t.req("POST", "/projects")
                .header(axum::http::header::CONTENT_TYPE, "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await;
    assert_eq!(res.status(), StatusCode::OK, "同一个目录要能重新导入");
    let again = body_json(res).await;
    assert!(again.get("id").and_then(|v| v.as_u64()).is_some(), "{again}");
    assert_eq!(
        std::fs::read_to_string(&cfg).unwrap(),
        cfg_before,
        "重新导入不能改已有配置（只登记、不覆盖）"
    );
}
}
