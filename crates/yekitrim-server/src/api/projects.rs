//! `api::projects`：项目托管（登记 / 建项目 / 注销 / 删除源）。

use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::{
    extract::{Extension, Path as AxumPath, State},
    http::StatusCode,
    Json,
};
use yekitrim_service::Workspace;

use crate::auth::owner_of;
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

/// id → 用户名 的映射，**只查一次**。
///
/// ⚠️ 这里曾是 `owner_name(db, id)` = 每个项目调一次 `db.list_users()`（**全表扫**）——
/// `projects_list` 里 10 个项目就是 10 次全表扫，而且**都持同一把** `Db { conn: Mutex<Connection> }`
/// 的锁（DB 是单连接，见 `db.rs`）。项目一多，列表接口就在"锁 + 全表扫"上排队（整改 T11 的 N+1）。
fn owner_name_map(db: &yekitrim_service::db::Db) -> Result<std::collections::HashMap<i64, String>, AppError> {
    let users = db
        .list_users()
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("读取账号列表失败: {e:#}")))?;
    Ok(users.into_iter().map(|u| (u.id, u.username)).collect())
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
    // 用户表进闭包前查一次（映射是纯内存查表，闭包里不再碰 DB）
    let owner_names = owner_name_map(db)?;
    let out = blocking(move || {
        let out: Vec<serde_json::Value> = rows
            .iter()
            .map(|p| {
                // `missing` 由 `summary_json` 按**磁盘当场算**（目录不在 / 配置解析失败），
                // 这里**不再顺手写库**（读接口有副作用是最容易踩的一类坑，review P3）。
                // DB 里那一列由启动时的 `projects::refresh_all_bytes` 双向校准。
                yekitrim_service::projects::summary_json(
                    p,
                    owner_names.get(&p.owner_id).map(|s| s.as_str()),
                )
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
/// `POST /api/projects` 的请求体。
///
/// ⚠️ 目前只有 `source.kind = "local"` 一条路（其它 kind 由 handler 明确回 400 并指路），
/// 所以这些字段名就是**契约**：前端 `stores/project.js::createLocalProject` 与
/// `WorkbenchView.addRecording` 都按这组名字发（改名字 = 同时改前端）。
#[derive(serde::Deserialize, Default)]
pub(super) struct CreateProjectBody {
    #[serde(default)]
    pub source: CreateProjectSource,
}

/// `CreateProjectBody.source` 那一段。
#[derive(serde::Deserialize, Default)]
pub(super) struct CreateProjectSource {
    /// `local`（`recorder` / `bilibili` 走各自的入库入口，这个端点会明确拒绝）。
    #[serde(default)]
    pub kind: String,
    /// 服务器上的根提示（相对租户根）。只有 CLI 与过渡别名会用到。
    #[serde(default)]
    pub root: String,
    #[serde(default)]
    pub dir: String,
    /// 显式指定的视频（相对 `dir`）；留空 = 自动挑目录里第一个视频。
    #[serde(default)]
    pub input: String,
    /// 可选弹幕（相对 `dir`）。`local` 来源**不自动发现**同名 .ass，只认显式给的。
    #[serde(default)]
    pub danmaku: Option<String>,
    #[serde(default)]
    pub output_dir: Option<String>,
    /// 目录里已有 `yekitrim.toml` 时是否**显式**重建（默认 false = 只登记、不覆盖）。
    #[serde(default)]
    pub overwrite: bool,
}

pub(super) async fn projects_create(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<crate::auth::CurrentUser>,
    Json(body): Json<serde_json::Value>,
) -> ApiResult {
    let CreateProjectBody { source } = parse_body(body)?;
    let kind = source.kind.as_str();
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
    // ⚠️ 重建（覆盖已有 `yekitrim.toml`）必须由调用方**显式**要求：
    // 前端在"目录已有工程、但用户又手动挑了视频"时弹二次确认，确认后才带这个位。
    // 默认 false —— 不带它的调用（含老前端、CLI 别名）行为与以前**一模一样**。
    create_local_project(
        &state,
        &user,
        &source.root,
        &source.dir,
        &source.input,
        source.danmaku.as_deref(),
        source.output_dir.as_deref(),
        source.overwrite,
    )
    .await
}

/// 用"服务器本机文件"建项目的公共实现（`POST /api/projects` 与过渡期的
/// `POST /api/project/init` 共用）。
///
/// 管理员可以给**任意绝对目录**（与单用户时代一致），非管理员只能是租户根内的相对路径；
/// 目录里已经存在 `yekitrim.toml` 时**只登记/打开，不覆盖**（避免把已有工程的设置冲掉）。
///
/// ## ⚠️ 整个落盘段都在 blocking 池上（T4b）
///
/// 项目目录常常在**网络盘**上：`is_dir` / `is_file` / `read_dir` / `write_project_config` /
/// `Workspace::load` / `refresh_bytes`（递归计量）/ `server.toml` 回写 —— 每一个都可能几百毫秒，
/// 十几个并发就把 tokio worker 全占住（"建项目时整个服务器连不上"）。T4 只把
/// `create_dir_all` 与 `find_first_video` 两处搬下去，剩下的照样能把 worker 按住，
/// 所以这里改成**异步侧只算归属 + 抄参数，剩下整段一个闭包**：
///
/// * async 侧：`owner_of` → `owner_root`（纯路径运算）、把入参抄成 `'static` 的 String、
///   拿一份 `Arc<Db>`（`AuthService::db_arc`，不能借 `&Db`）；
/// * blocking 侧：选目录 → 判定 → 写配置 → 登记 → 计量 → 记 `server.toml` → 拼摘要。
///
/// ⚠️ **别在这里留裸 `std::fs` / `Db` 调用**：这条不变式只有一条判据，
/// `create_project_runs_sync_io_off_the_worker`（单 worker + 心跳，回退成同步写法它必红）。
pub(super) async fn create_local_project(
    state: &Arc<AppState>,
    user: &crate::auth::CurrentUser,
    root_hint: &str,
    rel_dir: &str,
    input_hint: &str,
    danmaku_hint: Option<&str>,
    output_dir: Option<&str>,
    overwrite: bool,
) -> ApiResult {
    // ---- async 侧：只有"归属"与"抄参数"（全是内存操作）----
    let tenant = yekitrim_core::owner::owner_root(owner_of(user));
    let uid = user.id;
    let is_admin = user.is_admin();
    // `username` 要进返回的摘要；`Arc<Db>` 是因为闭包必须 `'static`（不能借 `&state`）。
    let username = user.username.clone();
    let db = state.auth.db_arc();
    let root_hint = root_hint.to_string();
    let rel_dir = rel_dir.to_string();
    let input_hint = input_hint.to_string();
    let danmaku_hint = danmaku_hint.map(str::to_string);
    let output_dir = output_dir.map(str::to_string);

    // ---- blocking 侧：整段落盘（判目录 / 写配置 / 登记 / 计量 / 记忆 / 摘要）----
    blocking(move || {
        // `create_dir_all` 是同步 IO（租户根在 `[storage] projects_root` 里，可能是网络盘）。
        std::fs::create_dir_all(&tenant)
            .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("创建租户目录失败: {e}")))?;

        // ---- 这里**曾经**放过一道容量预检，已删除（2026-10-10）----
        //
        // 当时的写法是 `if !user.is_admin() { check_capacity(...) }`，理由是"手工建项目这条路
        // 没查过配额"。**实测它一次都不会触发**：`source.kind = "local"` 在 `projects_create`
        // 与 `/project/init` 两处入口都先挡了 `if !user.is_admin() → 403`，所以能走到这里的
        // **只有管理员**，而闸门又豁免管理员 ⇒ 条件恒假。
        //
        // 起过的验证（隔离数据目录、上限设 1 字节、已登记占用 11 字节）：
        // 管理员仍能建项目、返回 200 —— 确认是死代码，不是"没测到"。
        //
        // 结论：**手工建项目这条路上没有需要补的配额闸门** ——
        // 非管理员建项目的三条路（B站链接 / 录播姬 / 下载）**本来就各自调了**
        // `capacity::check_capacity`（`ingest.rs` / `ingest_worker.rs` / `download_worker.rs`），
        // 而管理员**按设计**不受配额约束（他要能收拾一个已经超额的账号）。
        // 别再把这段加回来 —— 要么真让它对某些人生效，要么就别写。

        let dir = yekitrim_service::projects::resolve_target_dir(uid, &root_hint, &rel_dir, is_admin)
            .map_err(|e| err(StatusCode::BAD_REQUEST, format!("目录不可用: {e:#}")))?;
        if !dir.is_dir() {
            return Err(err(StatusCode::BAD_REQUEST, "只能选择目录"));
        }
        let config_path = dir.join(yekitrim_core::path::PROJECT_CONFIG_FILE);
        let is_existing_project = config_path.is_file();

        // 已有 yekitrim.toml：**默认只登记/打开、不重写**（否则会覆盖用户手改过的工程设置）。
        // ⚠️ 只有调用方**显式**给了 `overwrite`（前端"目录已有工程 + 用户又手动挑了视频"那条路，
        // 弹窗二次确认后才带）才重写。别把它改成默认 true —— 那等于把用户手改的配置随手冲掉。
        if !is_existing_project || overwrite {
            if overwrite && is_existing_project {
                tracing::warn!(
                    "按调用方显式要求重建工程配置: {}（原内容将被覆盖）",
                    config_path.display()
                );
            }
            let hint = input_hint.trim();
            let input = if hint.is_empty() {
                // 未指定文件时自动挑目录里第一个视频。`find_first_video` 里是 `read_dir` +
                // 逐项 `is_file` + 逐项 `extension` —— **这里已经在 blocking 池上**，直接调即可
                // （T4 时期它还得单独下一次 blocking，因为那会儿函数体还在 async 侧）。
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
                    uid,
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
            let _ = yekitrim_service::projects::refresh_bytes(&db, &p);
        }
        let _ = db.audit(
            Some(uid),
            "project_create",
            &format!("id={id} kind=local overwrite={overwrite}"),
        );
        remember_last_project(&ws);
        match db.project_by_id(id) {
            Ok(Some(p)) => ok(serde_json::json!({
                "ok": true,
                "id": id,
                "project": yekitrim_service::projects::summary_json(&p, Some(&username)),
            })),
            _ => Err(err(StatusCode::INTERNAL_SERVER_ERROR, "项目登记后读取失败")),
        }
    })
    .await
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
/// 只登记、不覆盖（见 `create_local_project`；**除非调用方显式 `overwrite`**），
/// 所以手改过的工程设置不会丢。
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

    // 这一段全是同步文件访问：`Workspace::load` / `load_config` 读工程配置，`read_dir` 扫项目根
    // 目录的分片（录播姬多分片会话能有几十个），逐个 `canonicalize` + `metadata` + `remove_file`
    // 删源，最后 `refresh_bytes` 还要递归 `read_dir` 累计源/产物大小。项目目录常在网络盘上，
    // 整段（含计量）一起下 blocking 池 —— 别把它留在 worker 上。
    let scan_root = project.root_dir.clone();
    let cfg_file = project.config_file.clone();
    let row = project.clone();
    let st = state.clone();
    let (removed, freed, (source_bytes, output_bytes)) = blocking(move || {
        let mut removed: Vec<String> = Vec::new();
        let mut freed = 0u64;
        let ws = Workspace::load(Some(&cfg_file)).ok();
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
        if let Ok(entries) = std::fs::read_dir(&scan_root) {
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
        let root = scan_root.canonicalize().unwrap_or_else(|_| scan_root.clone());
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
        // 计量同样要递归遍历源/产物目录 —— 跟删源在同一个闭包里一次算完（写库也是同步的）
        let db = st.auth.db();
        let (source_bytes, output_bytes) = yekitrim_service::projects::refresh_bytes(db, &row)
            .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("刷新占用失败: {e:#}")))?;
        Ok((removed, freed, (source_bytes, output_bytes)))
    })
    .await?;
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
    let tenant = yekitrim_core::owner::owner_root(owner_of(&user));
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
    // 过渡期别名**不重建**：它以前就不覆盖，行为保持不变
    let created = create_local_project(&state, &user, "", &rel_dir, "", danmaku, output, false).await?;
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

/// `source.overwrite`：**只有显式带这个位**才重建已有工程的配置；不带就一个字节都不动。
///
/// 这条不变式是"用户手改过的工程设置不能被随手冲掉"的唯一防线
/// （前端在"目录已有工程 + 用户又手动挑了视频"时弹二次确认，确认后才带这个位）。
#[tokio::test]
async fn overwrite_rebuilds_config_only_when_explicitly_asked() {
    let t = test_app().await;
    let cfg = t.root.join("yekitrim.toml");
    let cfg_before = std::fs::read_to_string(&cfg).unwrap();
    let dir = yekitrim_core::path::display(&t.root);

    let post = |source: serde_json::Value| {
        let t = &t;
        let source = source.clone();
        async move {
            t.call(
                t.req("POST", "/projects")
                    .header(axum::http::header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        serde_json::json!({ "source": source }).to_string(),
                    ))
                    .unwrap(),
            )
            .await
        }
    };

    // ① 不带 overwrite（老前端 / 默认）→ 配置原样
    std::fs::write(t.root.join("b.mp4"), b"x").unwrap();
    let res = post(serde_json::json!({
        "kind": "local", "dir": dir, "input": "b.mp4"
    }))
    .await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(
        std::fs::read_to_string(&cfg).unwrap(),
        cfg_before,
        "没带 overwrite 时**绝不能**改已有配置"
    );

    // ② 显式 overwrite + 指定另一个视频 → 配置被重写成新输入
    let res = post(serde_json::json!({
        "kind": "local", "dir": dir, "input": "b.mp4", "overwrite": true
    }))
    .await;
    assert_eq!(res.status(), StatusCode::OK, "显式 overwrite 应当成功");
    let after = std::fs::read_to_string(&cfg).unwrap();
    assert_ne!(after, cfg_before, "显式 overwrite 必须真的重写");
    assert!(after.contains("b.mp4"), "重写后的输入应当是刚选的那个视频: {after}");
}

/// 整改 T11：`GET /api/projects` 的 owner 名**不再逐项目查一次用户表**（N+1）。
///
/// 以前 `owner_name(db, p.owner_id)` 在闭包里逐项目调 `db.list_users()`（全表扫 + 同一把
/// `Mutex<Connection>`），10 个项目 = 10 次全表扫。现在闭包外查一次、闭包里纯内存查表。
///
/// 判据取"**每条都要有 owner 名**"：映射接错（键类型 / 键错列）时的症状是整列变成 `null`，
/// 而"少查了几次"本身没法在 server 侧计数 —— `yekitrim-service` 作为依赖被编译时**没有** `cfg(test)`，
/// 所以计划里推荐的 `#[cfg(test)]` 计数器在这个断言侧看不见（要么污染生产代码常驻一个计数器，
/// 要么就靠这条功能判据）。这里选后者，并把理由写在这里，免得下一个人以为漏了。
#[tokio::test]
async fn projects_list_resolves_every_owner_name() {
    let t = test_app().await;
    for i in 0..10 {
        let dir = t.root.join(format!("p{i}"));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.mp4"), b"x").unwrap();
        let body = serde_json::json!({
            "source": {
                "kind": "local",
                "dir": yekitrim_core::path::display(&dir),
                "input": "a.mp4",
            }
        });
        let res = t
            .call(
                t.req("POST", "/projects")
                    .header(axum::http::header::CONTENT_TYPE, "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await;
        assert_eq!(res.status(), StatusCode::OK, "第 {i} 个项目应当建成");
    }

    let res = t
        .call(t.req("GET", "/projects").body(Body::empty()).unwrap())
        .await;
    assert_eq!(res.status(), StatusCode::OK);
    let v = body_json(res).await;
    let projects = v["projects"].as_array().expect("projects 必须是数组");
    assert_eq!(
        projects.len(),
        11,
        "夹具自带 1 个 + 新建 10 个；实际 {}",
        projects.len()
    );
    for p in projects {
        assert_eq!(
            p["owner"].as_str(),
            Some("admin"),
            "每条项目都必须解析出 owner 名（映射接错的症状是整列 null）: {p}"
        );
    }
}
/// 整改 T4a：`DELETE /api/projects/:id/source` 此前**零测试覆盖**（T4 只做机械搬移）。
///
/// 判据三件事：①源被删（含根目录分片）②产物与项目本身**一个都不动** ③计量真的刷新
/// （`source_bytes` 归 0、`freed_bytes` = 删掉那几份的字节数）。
#[tokio::test]
async fn source_delete_removes_sources_but_keeps_products() {
    let t = test_app().await;
    // 夹具自带项目 1（root + a.mp4）。再加一个分片（录播姬多分片会话的形态）与一份产物。
    let shard = t.root.join("shard-02.mp4");
    std::fs::write(&shard, vec![0u8; 4096]).unwrap();
    let clip = t.root.join("output").join("clips").join("clip-01").join("clip.mp4");
    std::fs::create_dir_all(clip.parent().unwrap()).unwrap();
    std::fs::write(&clip, b"product-bytes").unwrap();
    let cfg = t.root.join("yekitrim.toml");
    let cfg_before = std::fs::read_to_string(&cfg).unwrap();

    let res = t
        .call(t.req("DELETE", "/projects/1/source").body(Body::empty()).unwrap())
        .await;
    assert_eq!(res.status(), StatusCode::OK);
    let v = body_json(res).await;

    // ① 源没了（a.mp4 是夹具里的 input_file；shard-02.mp4 是根目录分片）
    assert!(!t.root.join("a.mp4").exists(), "input_file 必须被删: {v}");
    assert!(!shard.exists(), "根目录下的分片也算源，必须被删: {v}");
    // ② 产物、工程配置、项目登记一个都不动
    assert!(clip.exists(), "产物必须留着（删源 ≠ 删产物）: {v}");
    assert_eq!(std::fs::read_to_string(&cfg).unwrap(), cfg_before, "工程配置不许变");
    assert!(project_ids(&t).await.contains(&1), "项目还在（删源 ≠ 移除项目）");
    // ③ 计量刷新：源归零、释放量 > 0
    assert_eq!(v["source_bytes"].as_u64(), Some(0), "删完源占用应当归 0: {v}");
    assert!(
        v["freed_bytes"].as_u64().unwrap_or(0) > 0,
        "释放字节数必须大于 0: {v}"
    );
    assert!(
        v["removed"].as_array().map(|a| !a.is_empty()).unwrap_or(false),
        "至少要报一份被删的文件: {v}"
    );
}

/// **T9**：`CreateProjectBody` / `CreateProjectSource`（`POST /projects`）。
///
/// 字段名是**契约**：前端 `stores/project.js::createLocalProject` 与 `WorkbenchView.addRecording`
/// 都按这组名字发。判据：`source` 或其字段类型错 → 400 且报错点名；缺 `kind` 仍走原来那条
/// 「未知来源 kind」400（宽松解析 + 原有语义都保住）。
#[tokio::test]
async fn create_project_body_is_typed_and_names_the_field() {
    let t = test_app().await;

    // source 整个不是对象（⚠️ 别用 `[]`：serde 允许**结构体按位置**从数组反序列化，
    // `[]` 会被当成"所有字段都用默认值"，于是走到下游的「未知来源」而不是类型错）
    let (status, text) = call_json(&t, "POST", "/projects", serde_json::json!({ "source": 5 })).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "source 类型错必须 400：{text}");
    assert!(text.contains("source"), "报错要点名 source：{text}");

    // source.dir 类型错（数字）
    let (status, text) = call_json(
        &t,
        "POST",
        "/projects",
        serde_json::json!({ "source": { "kind": "local", "dir": 5 } }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "source.dir 类型错必须 400：{text}");
    assert!(text.contains("source"), "报错要点名 source（顶层键）：{text}");

    // 宽松解析：多传的陌生键被忽略 ⇒ 走到原来的「未知来源」（而不是解析错）
    let (status, text) = call_json(
        &t,
        "POST",
        "/projects",
        serde_json::json!({ "source": { "kind": "nope", "junk": true } }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(text.contains("未知来源"), "多余字段被忽略，于是走原有校验：{text}");
}

/// 整改 T4b：`create_local_project` 的**整段落盘**（选目录 / 写配置 / 登记账本 / 递归计量 / 记 server.toml / 拼摘要）
/// 都在 blocking 池上 —— 单 worker runtime 下，建项目期间心跳必须照跳。
///
/// fixture = 临时目录 + 3,000 个几字节文件（`refresh_bytes` 要递归 `read_dir` 统计），**不依赖网络盘**。
/// 把那个闭包整段搬回 async 侧，这条用例立刻红（`during == 0`）。
/// ⚠️ 必须 `tokio::spawn` 到 worker 再 await：`#[tokio::test]` 的测试体跑在 block_on 的 main 线程上，
/// 直接 await 的话被卡住的是 main 线程、心跳在 worker 上照跳，测不出任何东西。
#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn create_project_runs_sync_io_off_the_worker() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::time::{sleep, Duration};

    let tmp = tempfile::tempdir().unwrap();
    let auth = crate::auth::AuthService::open(
        tmp.path().join("auth"),
        crate::auth::Listening::Loopback,
    )
    .unwrap();
    let hash = yekitrim_service::auth::hash_password("initial-pass-1234").unwrap();
    let uid = auth
        .db()
        .create_user("admin", &hash, "admin", false, 0)
        .unwrap();
    let state = Arc::new(AppState::with_auth(auth));
    let user = crate::auth::CurrentUser {
        id: uid,
        username: "admin".to_string(),
        role: "admin".to_string(),
        must_change_password: false,
        session_id: "test-session".to_string(),
    };

    // 待登记的目录：一个视频 + 3,000 个几字节文件（让 read_dir 扫描与递归计量都有真实耗时）
    let dir = tmp.path().join("proj");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("v.mp4"), vec![0u8; 1024]).unwrap();
    for i in 0..3000 {
        std::fs::write(dir.join(format!("f{i:05}.bin")), b"x").unwrap();
    }
    let cfg = dir
        .canonicalize()
        .unwrap()
        .join(yekitrim_core::path::PROJECT_CONFIG_FILE);

    // ---- 心跳：只有 1 个 worker，同步 IO 若留在 worker 上它就再也跳不动 ----
    let ticks = Arc::new(AtomicUsize::new(0));
    let beats = ticks.clone();
    let beat = tokio::spawn(async move {
        loop {
            sleep(Duration::from_millis(1)).await;
            beats.fetch_add(1, Ordering::SeqCst);
        }
    });
    sleep(Duration::from_millis(50)).await;
    assert!(ticks.load(Ordering::SeqCst) > 0, "心跳应当已经跳起来");

    let before = ticks.load(Ordering::SeqCst);
    let t0 = std::time::Instant::now();
    let st = state.clone();
    let d = yekitrim_core::path::display(&dir);
    let call = tokio::spawn(async move {
        create_local_project(&st, &user, "", &d, "", None, None, false).await
    });
    let res = call.await.expect("建项目任务不该 panic");
    let elapsed = t0.elapsed();
    let during = ticks.load(Ordering::SeqCst) - before;
    beat.abort();

    // 功能面：项目登记进账本 + 落出 yekitrim.toml
    assert!(res.is_ok(), "建项目应当成功: {:?}", res.as_ref().err());
    assert!(cfg.is_file(), "应当写出 yekitrim.toml");
    assert!(
        state
            .auth
            .db()
            .project_by_config(&cfg)
            .unwrap()
            .is_some(),
        "项目必须登记进账本"
    );

    // 时序判据：建项目期间 worker 一直被让出来
    assert!(
        elapsed.as_millis() >= 5,
        "fixture 太小（建项目只用了 {elapsed:?}）：测不出 worker 是否被占，请加大文件数"
    );
    assert!(
        during >= 3,
        "建项目期间 worker 必须还能跑别的任务（心跳只跳了 {during} 次 / 用时 {elapsed:?}）\
         —— 同步 IO 又回到 worker 上了？"
    );
}
}
