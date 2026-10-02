//! 项目作用域（多用户服务 P2）：把"任意路径"收敛成"本人项目的 id + 项目内相对路径"。
//!
//! 两条入口：
//! 1. [`Scope`] 提取器 —— `/api/project-data/*` 用：从 `?project=<id>`（**仅本人**）
//!    解析出项目与它的 `Workspace`；过渡期额外接受 `?config=<已登记的本人项目 vtrim.toml>`
//!    （只作为"路径→项目"的反查，**未登记的路径一律 404**，因此路径面已经关闭）。
//! 2. [`enforce_scope`] 中间件 —— `/api/polish/*` 用：先读请求里的 `dir`/`project`
//!    （query 或 JSON body），确认它落在**调用者自己的**某个项目根之内；不通过直接
//!    404，handler 里原有的 `dir` 逻辑保持不变（旧前端仍可用，但只能操作自己的项目）。
//!
//! ⚠️ **body 里所有会被 handler 拿去开文件的字段都必须在这里收口** ——
//! `dir` / `output_file` / `source` / `cover.image`，以及贴纸两接口的 `asset` / `assets`
//! （它们相对的是 **`dir`** 而不是项目根，走 [`is_safe_project_relative`] 单独收口）。
//! 漏一个就是一条任意文件读取：
//! 曾经 `source`（`polish_cover_background`）与 `cover.image`（`polish_cover_config`）
//! 就因为不在收口名单里，普通用户能把服务器上任意文件（例如 `.vtrim/llm.toml`，
//! 里面有 LLM API key）复制进自己的项目、再用 `/api/media` 读回来。

use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::extract::{Request, State};
use axum::http::{Method, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::Json;

use yekitrim_service::db::{Db, ProjectRow};
use yekitrim_service::Workspace;

use super::auth::CurrentUser;
use super::model::AppState;

type AppError = (StatusCode, Json<serde_json::Value>);

fn err(status: StatusCode, msg: impl Into<String>) -> AppError {
    (
        status,
        Json(serde_json::json!({ "ok": false, "error": msg.into() })),
    )
}

/// 已解析的项目作用域（提取器产出）。
#[derive(Debug, Clone)]
pub struct Scope {
    pub project: ProjectRow,
    pub ws: Workspace,
}

fn load_ws(project: &ProjectRow) -> Result<Workspace, AppError> {
    Workspace::load(Some(&project.config_file)).map_err(|e| {
        err(
            StatusCode::NOT_FOUND,
            format!("项目配置不可用（{}）: {e:#}", project.config_file.display()),
        )
    })
}

/// 按 id + 归属取项目（media 等需要"取到项目行"而非 `Workspace` 的场景）。
pub fn find_owned_project(
    db: &Db,
    user: &CurrentUser,
    id: i64,
) -> Result<Option<ProjectRow>, anyhow::Error> {
    yekitrim_service::projects::find_owned(db, user.id, user.is_admin(), id)
}

/// 按 `?project=<id>` 取项目并校验归属；他人项目一律 404。
pub fn project_scope(db: &Db, user: &CurrentUser, id: i64) -> Result<Scope, AppError> {
    let project = yekitrim_service::projects::find_owned(db, user.id, user.is_admin(), id)
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("读取项目失败: {e:#}")))?
        .ok_or_else(|| err(StatusCode::NOT_FOUND, "项目不存在或无权访问"))?;
    let ws = load_ws(&project)?;
    Ok(Scope { project, ws })
}

/// 过渡期兼容：`?config=<路径>` → 反查**已登记且属于本人**的项目（否则 404）。
pub fn scope_from_config(db: &Db, user: &CurrentUser, config: &str) -> Result<Scope, AppError> {
    let path = PathBuf::from(config);
    let project = db
        .project_by_config(&path)
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("读取项目失败: {e:#}")))?
        .filter(|p| user.is_admin() || p.owner_id == user.id)
        .ok_or_else(|| {
            err(
                StatusCode::NOT_FOUND,
                "项目不存在或无权访问（请使用 /api/projects 里的项目 id，不要直接传路径）",
            )
        })?;
    let ws = load_ws(&project)?;
    Ok(Scope { project, ws })
}

/// 从查询串里解析作用域：优先 `project=<id>`，其次兼容 `config=<路径>`。
pub fn scope_from_query(
    db: &Db,
    user: &CurrentUser,
    query: &std::collections::HashMap<String, String>,
) -> Result<Scope, AppError> {
    if let Some(id) = query
        .get("project")
        .and_then(|v| yekitrim_service::projects::parse_project_id(v))
    {
        return project_scope(db, user, id);
    }
    if let Some(config) = query.get("config").filter(|s| !s.trim().is_empty()) {
        return scope_from_config(db, user, config);
    }
    Err(err(
        StatusCode::BAD_REQUEST,
        "缺少 project 参数（项目 id 见 /api/projects）",
    ))
}

/// `Scope` 提取器：从 `?project=` / `?config=` 解析（`/api/project-data/*` 用）。
#[axum::async_trait]
impl axum::extract::FromRequestParts<Arc<AppState>> for Scope {
    type Rejection = AppError;

    async fn from_request_parts(
        parts: &mut axum::http::request::Parts,
        state: &Arc<AppState>,
    ) -> Result<Self, Self::Rejection> {
        let user = parts
            .extensions
            .get::<CurrentUser>()
            .cloned()
            .ok_or_else(|| err(StatusCode::UNAUTHORIZED, "未登录，请先登录"))?;
        let query: std::collections::HashMap<String, String> =
            parts.uri.query().map(parse_query).unwrap_or_default();
        scope_from_query(state.auth.db(), &user, &query)
    }
}

/// 极简 query 解析（只处理 `k=v&k=v`，值做 percent-decode）。
pub fn parse_query(raw: &str) -> std::collections::HashMap<String, String> {
    let mut out = std::collections::HashMap::new();
    for pair in raw.split('&') {
        if pair.is_empty() {
            continue;
        }
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        out.insert(percent_decode(k), percent_decode(v));
    }
    out
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 2 < bytes.len() => {
                let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or("");
                match u8::from_str_radix(hex, 16) {
                    Ok(b) => {
                        out.push(b);
                        i += 3;
                    }
                    Err(_) => {
                        out.push(bytes[i]);
                        i += 1;
                    }
                }
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// 在调用者自己的项目里按目录反查归属项目（旧前端的绝对 `dir` 靠它收敛）。
pub fn find_project_for_dir(db: &Db, user: &CurrentUser, dir: &Path) -> Result<Option<ProjectRow>, AppError> {
    let projects = db
        .list_projects(Some(user.id))
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("读取项目失败: {e:#}")))?;
    let target = dir.canonicalize().unwrap_or_else(|_| dir.to_path_buf());
    let mut best: Option<(usize, ProjectRow)> = None;
    for p in projects {
        let root = p.root_dir.canonicalize().unwrap_or_else(|_| p.root_dir.clone());
        if target.starts_with(&root) {
            let depth = root.components().count();
            if best.as_ref().map(|(d, _)| depth > *d).unwrap_or(true) {
                best = Some((depth, p));
            }
        }
    }
    Ok(best.map(|(_, p)| p))
}

/// 把请求里的 `dir` 收敛成"本项目内的绝对路径"。
///
/// - 相对路径 → `jail_existing` 到项目根；
/// - 绝对路径（旧前端） → 必须 canonicalize 后仍在本项目根内，否则 404；
/// - 空 → 项目根。
pub fn resolve_dir(project: &ProjectRow, dir: &str) -> Result<PathBuf, AppError> {
    let dir = dir.trim();
    if dir.is_empty() {
        return Ok(project.root_dir.clone());
    }
    let root = project
        .root_dir
        .canonicalize()
        .unwrap_or_else(|_| project.root_dir.clone());
    let candidate = Path::new(dir);
    if candidate.is_absolute() {
        let real = candidate
            .canonicalize()
            .map_err(|e| err(StatusCode::BAD_REQUEST, format!("目录不存在: {e}")))?;
        if !real.starts_with(&root) {
            return Err(err(StatusCode::NOT_FOUND, "目录不在该项目内"));
        }
        return Ok(real);
    }
    yekitrim_core::tenant::jail_existing(&project.root_dir, dir)
        .map_err(|e| err(StatusCode::BAD_REQUEST, format!("目录不可用: {e:#}")))
}

/// body 里**除 `dir` / `output_file` 之外**、同样会被 handler 当成路径拿去开文件的字段。
///
/// 点号表示嵌套键（`cover.image` 在 `cover-config` 的 body 里是 `{"cover":{"image":…}}`）。
/// 这里的每一项都由 [`enforce_scope`] 校验并归一化成"项目内绝对路径"。
///
/// ⚠️ **新增任何"拿 body 字段去开文件"的 handler 都要同步加进来** —— 漏一个就是一条
/// 任意文件读取（`source` 与 `cover.image` 就是这么漏的，见本模块顶部说明）。
/// ⚠️ 这张名单的语义是"**相对项目根**的路径字段"；贴纸两接口的 `asset` / `assets`
/// 相对的是 **`dir`**（clip 目录），基准不同进不了这个循环 —— 在 [`enforce_scope`]
/// 里用 [`is_safe_project_relative`] 单独收口（见 [`sanitize_sticker_paths`]）。
const EXTRA_PATH_FIELDS: [&str; 2] = ["source", "cover.image"];

/// **"工程内相对路径"的判据（贴纸两接口的 `asset` / `assets` 用）**。
///
/// # 为什么不能只用 `Path::is_absolute()`
///
/// 那一条在 **Windows 上是漏的**：`Path::new("/etc/passwd").is_absolute()` 返回 **false**
/// （无盘符/无前缀），于是 `/etc/passwd` 能穿过判据，而 `Path::join` 对"有根无前缀"的路径
/// **会替换掉基准**（实测 `Path::new(r"C:\proj").join("/etc/passwd")` → `C:/etc/passwd`
/// —— 已经跑到工程外去了）。反斜杠同理（`\windows\x` → `C:\windows\x`）。
///
/// 所以**显式逐字符判**：空 / 绝对 / 根相对（`/` `\` 开头）/ 盘符（`:`）/ `..` / 反斜杠
/// 全挡。它与平台无关、无 IO；**全仓只此一份**（`api/polish.rs` 的两个 handler 用它做
/// handler 侧兜底，本模块的中间件用它做第一道收口 —— 两层缺一不可，
/// `/etc/passwd` 那条洞就是"只有一份实现、且藏在 handler 里没有第二道"留下的）。
pub(crate) fn is_safe_project_relative(p: &str) -> bool {
    !p.is_empty()
        && !std::path::Path::new(p).is_absolute()
        && !p.starts_with('/')
        && !p.starts_with('\\')
        && !p.starts_with('.')   // `../x` 与隐藏目录都不该从请求里进来
        && !p.contains("..")
        && !p.contains(':')
        && !p.contains('\\')
}

/// 收口贴纸路径字段（[`is_safe_project_relative`] 的中间件入口）。
///
/// 看两个字段：`asset`（`/polish/sticker-frame`，单值）与 `assets`
/// （`/polish/sticker-prewarm`，字符串数组）。语义跟着各自的 handler 走：
/// - **单值坏 → `Err`（400）**：handler 对坏 `asset` 本来就是 400，这里只是提前一跳；
/// - **数组坏条目 → 就地剔除**：预热是"尽力而为"（单张贴纸失败只计数、**不让整个请求失败**，
///   有单测钉着）—— 但被剔除的值也**绝不允许到 handler 手里**，两层判据同一份。
///
/// **只做逐字符判定、不做 canonicalize**（无 IO，不把预热打成 400）。
fn sanitize_sticker_paths(v: &mut serde_json::Value) -> Result<(), String> {
    if let Some(a) = v.get("asset").and_then(|x| x.as_str()) {
        if !a.trim().is_empty() && !is_safe_project_relative(a) {
            return Err(a.to_string());
        }
    }
    if let Some(list) = v.get_mut("assets").and_then(|x| x.as_array_mut()) {
        list.retain(|x| {
            x.as_str()
                .map(|s| !s.trim().is_empty() && is_safe_project_relative(s))
                .unwrap_or(false)
        });
    }
    Ok(())
}

/// 读点号路径（不存在 / 不是字符串 → `None`）。
fn json_get_str<'a>(v: &'a serde_json::Value, path: &str) -> Option<&'a str> {
    let mut cur = v;
    for key in path.split('.') {
        cur = cur.get(key)?;
    }
    cur.as_str()
}

/// 就地改写点号路径（键不存在、或中间层不是对象时不写 —— 不给请求凭空补字段）。
fn json_set_str(v: &mut serde_json::Value, path: &str, val: &str) {
    let keys: Vec<&str> = path.split('.').collect();
    let mut cur = v;
    for key in &keys[..keys.len() - 1] {
        match cur.get_mut(*key) {
            Some(next) => cur = next,
            None => return,
        }
    }
    if let Some(last) = keys.last() {
        if cur.get(*last).is_some() {
            cur[*last] = serde_json::json!(val);
        }
    }
}

/// 把 `source` / `cover.image` 这类**文件**路径收敛成项目内的绝对路径。
///
/// 语义与 [`resolve_dir`] 一致（相对 → 项目根内解析；绝对 → 必须在项目根内），
/// 只是报错措辞按"文件"写、且要求目标**已存在**（这两个字段的用途都是"复制/读取一个
/// 已存在的文件"：`cover-background` 会 `fs::copy` 它，`cover` 渲染也会先断言存在）。
///
/// 这是 `/api/polish/*` 的路径面收口点：没有它，`source` 就能把服务器上任意文件
/// （例如 `<exe同级>/.vtrim/llm.toml`，里面有 LLM API key；或账号账本 `yekitrim.db`）
/// 复制进调用者自己的项目，再用 `/api/media/<自己的项目>/…` 读回来。
fn resolve_file_in_project(project: &ProjectRow, raw: &str, field: &str) -> Result<PathBuf, AppError> {
    let raw = raw.trim();
    let root = project
        .root_dir
        .canonicalize()
        .unwrap_or_else(|_| project.root_dir.clone());
    let candidate = Path::new(raw);
    if candidate.is_absolute() {
        let real = candidate.canonicalize().map_err(|e| {
            err(StatusCode::NOT_FOUND, format!("{field} 指向的文件不可用: {e}"))
        })?;
        if !real.starts_with(&root) {
            return Err(err(StatusCode::NOT_FOUND, format!("{field} 不在该项目内")));
        }
        return Ok(real);
    }
    yekitrim_core::tenant::jail_existing(&project.root_dir, raw)
        .map_err(|e| err(StatusCode::BAD_REQUEST, format!("{field} 不可用: {e:#}")))
}

/// 该请求是否带 body（写请求且不是明显的纯查询）。
fn is_write(method: &Method) -> bool {
    !matches!(*method, Method::GET | Method::HEAD | Method::OPTIONS)
}

/// 把同步解析搬出 async worker（与 `api::blocking` 同款，只是用本文件自己的 `AppError` 类型）。
///
/// ⚠️ 为什么这个中间件特别需要它：`enforce_scope` 挂在**整个 `/api/polish/*` 子树**上，
/// 每次请求都要走 SQLite 查询 + 若干次 `canonicalize()` —— 项目目录常在网络盘上
/// （RaiDrive/WebDAV），一次 canonicalize 就是几百毫秒到几秒。留在 worker 上做，
/// 前端一次页面加载打出的那一串 polish 请求就能把 worker 全占住：
/// runtime 连 accept 都排不上，日志里一行都没有（请求根本没处理完）。
/// 细则见 `src/api/AGENTS.md`「同步 IO 纪律」。
async fn off<T, F>(f: F) -> Result<T, AppError>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, AppError> + Send + 'static,
{
    match tokio::task::spawn_blocking(f).await {
        Ok(r) => r,
        Err(e) => Err(err(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("阻塞任务失败: {e}"),
        )),
    }
}

/// 中间件解析出的项目 id（`0` = 没能解析，例如无 dir/project 的全局 polish 接口）。
///
/// polish handler 用它给任务打项目标签（侧栏按项目显示进度），`0` 时不打标签。
#[derive(Debug, Clone, Copy)]
pub struct ScopeProjectId(pub i64);

/// `/api/polish/*` 的作用域守卫。
///
/// 做两件事（一次收口，18 个 polish handler 不用各自改）：
/// 1. **归属校验**：`project=<id>` 或路径类字段必须落在调用者自己的某个项目根之内，否则 404。
///    收口的字段是 [`EXTRA_PATH_FIELDS`]（相对项目根）加上单独处理的 `dir` / `output_file`
///    与贴纸的 `asset` / `assets`（相对 `dir`，[`sanitize_sticker_paths`]）——
///    **handler 里新出现"拿 body 字段去开文件"的写法时，必须同步收口**；
/// 2. **归一化**：把 `dir` 换成"该项目内的绝对路径"（相对路径按项目根解析），
///    写回 query 或 JSON body，于是 handler 里原有的 `validated_project_dir(dir)`
///    继续有效 —— 新前端可以只传 `{project, dir:"output/clips/clip-01"}`。
///    `source` / `cover.image` 同样被归一化：handler 里是 `PathBuf::from(v)`，
///    相对值会按**进程 CWD** 解析，归一化成绝对路径同时也修掉了这个坑。
pub async fn enforce_scope(
    State(state): State<Arc<AppState>>,
    req: Request,
    next: Next,
) -> Response {
    let Some(user) = req.extensions().get::<CurrentUser>().cloned() else {
        return err(StatusCode::UNAUTHORIZED, "未登录，请先登录").into_response();
    };
    // `db_arc()` 而不是 `db()`：同步解析要搬进 `spawn_blocking`，闭包必须是 'static，
    // 所以每次按需 clone 这个 Arc（见本文件 `off`）。
    let db = state.auth.db_arc();

    // 先解析作用域与"路径类"字段（不消费 body，body 在需要时再缓冲）
    let query = req.uri().query().map(parse_query).unwrap_or_default();
    let explicit = query
        .get("project")
        .and_then(|v| yekitrim_service::projects::parse_project_id(v));

    // 显式 project 优先；否则用绝对 dir 反查归属项目
    let mut scope: Option<ProjectRow> = match explicit {
        Some(id) => {
            let db = db.clone();
            let user = user.clone();
            match off(move || project_scope(&db, &user, id)).await {
                Ok(s) => Some(s.project),
                Err(e) => return e.into_response(),
            }
        }
        None => None,
    };
    if scope.is_none() {
        if let Some(dir) = query_path_field(&query) {
            if Path::new(&dir).is_absolute() {
                let db = db.clone();
                let user = user.clone();
                let dir_for_lookup = dir.clone();
                match off(move || find_project_for_dir(&db, &user, Path::new(&dir_for_lookup))).await
                {
                    Ok(Some(p)) => scope = Some(p),
                    Ok(None) => {
                        return err(StatusCode::NOT_FOUND, format!("路径不在你的任何项目内: {dir}"))
                            .into_response()
                    }
                    Err(e) => return e.into_response(),
                }
            }
        }
    }

    let rewritten_query = match (scope.as_ref(), query_path_field(&query)) {
        (Some(project), Some(dir)) => {
            let project = project.clone();
            match off(move || resolve_dir(&project, &dir)).await {
                Ok(abs) => Some(replace_query_value(
                    req.uri().query().unwrap_or(""),
                    "dir",
                    &abs.to_string_lossy(),
                )),
                Err(e) => return e.into_response(),
            }
        }
        (None, Some(dir)) => {
            return err(
                StatusCode::NOT_FOUND,
                format!("{dir:?} 不在你的任何项目内（相对路径需同时传 project=<id>）"),
            )
            .into_response()
        }
        _ => None,
    };

    // 写请求：缓冲 body，必要时把 dir/output_file 归一化后写回
    if !is_write(req.method()) {
        let mut req = req;
        if let Some(q) = rewritten_query {
            let path = req.uri().path().to_string();
            match format!("{path}?{q}").parse::<axum::http::Uri>() {
                Ok(uri) => *req.uri_mut() = uri,
                Err(e) => tracing::warn!("重写 query 失败: {e}"),
            }
        }
        req.extensions_mut().insert(ScopeProjectId(scope.map(|p| p.id).unwrap_or(0)));
        return next.run(req).await;
    }

    let (mut parts, body) = req.into_parts();
    let bytes = match axum::body::to_bytes(body, 64 * 1024 * 1024).await {
        Ok(b) => b,
        Err(e) => return err(StatusCode::BAD_REQUEST, format!("读取请求体失败: {e}")).into_response(),
    };
    let mut out_bytes = bytes.clone();
    if !bytes.is_empty() {        if let Ok(mut v) = serde_json::from_slice::<serde_json::Value>(&bytes) {
            // body 里的 project（新契约允许只放 body）
            if scope.is_none() {
                if let Some(id) = v
                    .get("project")
                    .and_then(|x| x.as_i64())
                    .and_then(|n| yekitrim_service::projects::parse_project_id(&n.to_string()))
                {
                    let db = db.clone();
                    let user = user.clone();
                    match off(move || project_scope(&db, &user, id)).await {
                        Ok(s) => scope = Some(s.project),
                        Err(e) => return e.into_response(),
                    }
                }
            }
            if let Some(dir) = v.get("dir").and_then(|x| x.as_str()).map(|s| s.to_string()) {
                // body 里只有绝对 dir（旧前端）：先按"落在本人哪个项目内"反查归属
                if scope.is_none() && Path::new(&dir).is_absolute() {
                    let db = db.clone();
                    let user = user.clone();
                    let dir_for_lookup = dir.clone();
                    match off(move || find_project_for_dir(&db, &user, Path::new(&dir_for_lookup)))
                        .await
                    {
                        Ok(Some(p)) => scope = Some(p),
                        Ok(None) => {
                            return err(
                                StatusCode::NOT_FOUND,
                                format!("路径不在你的任何项目内: {dir}"),
                            )
                            .into_response()
                        }
                        Err(e) => return e.into_response(),
                    }
                }
                match scope.as_ref() {
                    Some(project) => {
                        let project = project.clone();
                        let dir = dir.clone();
                        match off(move || resolve_dir(&project, &dir)).await {
                            Ok(abs) => v["dir"] = serde_json::json!(abs.to_string_lossy()),
                            Err(e) => return e.into_response(),
                        }
                    }
                    None => {
                        return err(
                            StatusCode::NOT_FOUND,
                            format!("{dir:?} 不在你的任何项目内（请先 GET /api/projects 拿项目 id）"),
                        )
                        .into_response()
                    }
                }
            }
            // `output_file` 必须是项目内相对文件名（P2：禁止任意输出路径）
            if let Some(out) = v.get("output_file").and_then(|x| x.as_str()).map(|s| s.to_string()) {
                if !out.trim().is_empty() {
                    let Some(project) = scope.as_ref() else {
                        return err(StatusCode::BAD_REQUEST, "output_file 需要 project 作用域")
                            .into_response();
                    };
                    let project_for_jail = project.clone();
                    let out_for_jail = out.clone();
                    // `jail_join` 内部要 canonicalize（网络盘上就是几百毫秒）→ 搬出 worker
                    let jail_ok = off(move || {
                        Ok(yekitrim_core::tenant::jail_join(
                            &project_for_jail.root_dir,
                            &out_for_jail,
                        )
                        .is_ok())
                    })
                    .await
                    .unwrap_or(false);
                    if !jail_ok {
                        return err(
                            StatusCode::BAD_REQUEST,
                            format!("output_file 必须是项目内相对路径: {out}"),
                        )
                        .into_response();
                    }
                }
            }
            // `source` / `cover.image`：handler 会拿它们去 `fs::copy` / 读文件，
            // 必须落在调用者自己的这个项目内（详见 EXTRA_PATH_FIELDS 的说明）
            for field in EXTRA_PATH_FIELDS {
                let Some(raw) = json_get_str(&v, field).map(|s| s.to_string()) else {
                    continue;
                };
                if raw.trim().is_empty() {
                    continue;
                }
                let Some(project) = scope.as_ref() else {
                    return err(
                        StatusCode::BAD_REQUEST,
                        format!("{field} 需要 project 作用域（路径类字段只接受项目内相对路径）"),
                    )
                    .into_response();
                };
                let project = project.clone();
                let raw_for_resolve = raw.clone();
                match off(move || resolve_file_in_project(&project, &raw_for_resolve, field)).await {
                    Ok(abs) => json_set_str(&mut v, field, &abs.to_string_lossy()),
                    Err(e) => return e.into_response(),
                }
            }
            // `asset` / `assets`（贴纸两接口）：相对 **`dir`** 的工程内相对路径 ——
            // 基准与 EXTRA_PATH_FIELDS 那批（相对项目根）不同，单独收口（判据见
            // [`sanitize_sticker_paths`]）。handler 里还有同一份判据兜底，两层缺一不可。
            if let Err(bad) = sanitize_sticker_paths(&mut v) {
                return err(
                    StatusCode::BAD_REQUEST,
                    format!("贴纸路径必须是工程内的相对路径: {bad}"),
                )
                .into_response();
            }
            out_bytes = axum::body::Bytes::from(serde_json::to_vec(&v).unwrap_or_else(|_| bytes.to_vec()));
        }
    }
    if let Some(q) = rewritten_query {
        let path = parts.uri.path().to_string();
        if let Ok(uri) = format!("{path}?{q}").parse::<axum::http::Uri>() {
            parts.uri = uri;
        }
    }
    let restored = Request::from_parts(parts, axum::body::Body::from(out_bytes));
    let mut restored = restored;
    restored
        .extensions_mut()
        .insert(ScopeProjectId(scope.map(|p| p.id).unwrap_or(0)));
    next.run(restored).await
}

/// query 里的第一个"路径类"字段（`dir` 为主，兼容 `path`）。
fn query_path_field(query: &std::collections::HashMap<String, String>) -> Option<String> {
    for key in ["dir", "path"] {
        if let Some(v) = query.get(key) {
            if !v.trim().is_empty() {
                return Some(v.clone());
            }
        }
    }
    None
}

/// 替换 query 串里某个键的值（percent-encode），键不存在时追加。
fn replace_query_value(raw: &str, key: &str, value: &str) -> String {
    let encoded = percent_encode(value);
    let mut parts: Vec<String> = Vec::new();
    let mut replaced = false;
    for pair in raw.split('&') {
        if pair.is_empty() {
            continue;
        }
        let (k, _) = pair.split_once('=').unwrap_or((pair, ""));
        if percent_decode(k) == key {
            parts.push(format!("{k}={encoded}"));
            replaced = true;
        } else {
            parts.push(pair.to_string());
        }
    }
    if !replaced {
        parts.push(format!("{key}={encoded}"));
    }
    parts.join("&")
}

fn percent_encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.as_bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(*b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_decodes_query() {
        let q = parse_query("project=3&config=F%3A%5Cmedia%5C3d%5Cyekitrim.toml&dir=a%2Fb+c");
        assert_eq!(q.get("project").map(|s| s.as_str()), Some("3"));
        assert_eq!(q.get("config").map(|s| s.as_str()), Some(r"F:\media\3d\yekitrim.toml"));
        assert_eq!(q.get("dir").map(|s| s.as_str()), Some("a/b c"));
        assert!(parse_query("").is_empty());
    }

    /// ⚠️ 中间件层的贴纸路径收口：Windows 上 `is_absolute()` 挡不住的"有根无前缀"路径
    /// （`/etc/passwd`、`\windows\x`）必须在这里与 handler 里**各挡一次** ——
    /// `/etc/passwd` 那条洞就是"只有 handler 一层、且判据漏了根相对"留下的。
    #[test]
    fn sticker_path_fields_are_rejected_at_middleware_layer() {
        for bad in [
            "", "/etc/passwd", r"\windows\x", "C:/x.gif", r"C:\x.gif", "../x.gif", "a/../../x", ".x",
        ] {
            assert!(!is_safe_project_relative(bad), "`{bad}` 本该被判为越界");
        }
        assert!(is_safe_project_relative("assets/stickers/问号.gif"));
        // asset 单值：坏 → 400（Err）
        assert_eq!(
            sanitize_sticker_paths(&mut serde_json::json!({ "asset": "/etc/passwd" })),
            Err("/etc/passwd".to_string())
        );
        assert_eq!(
            sanitize_sticker_paths(&mut serde_json::json!({ "asset": r"\windows\x" })),
            Err(r"\windows\x".to_string())
        );
        // assets 数组：坏条目**就地剔除**（预热尽力而为，但越界值绝不许到 handler）
        let mut v = serde_json::json!({ "assets": ["assets/stickers/问号.gif", "../x.gif", "ok.gif"] });
        sanitize_sticker_paths(&mut v).unwrap();
        assert_eq!(v["assets"], serde_json::json!(["assets/stickers/问号.gif", "ok.gif"]));
        // 全坏 ⇒ 剔成空数组（handler 对空数组照常 200，decoded/failed 都是 0）
        let mut v = serde_json::json!({ "assets": ["/etc/passwd"] });
        sanitize_sticker_paths(&mut v).unwrap();
        assert_eq!(v["assets"], serde_json::json!([]));
        // 合法值 / 字段缺席 → 都不拦
        let mut v = serde_json::json!({ "asset": "assets/stickers/问号.gif" });
        sanitize_sticker_paths(&mut v).unwrap();
        let mut v = serde_json::json!({ "dir": "x" });
        sanitize_sticker_paths(&mut v).unwrap();
    }

    #[test]
    fn resolve_dir_rejects_outside_absolute_paths() {
        let base = std::env::temp_dir().join(format!("vtrim-scope-{}", std::process::id()));
        let root = base.join("proj");
        std::fs::create_dir_all(root.join("output")).unwrap();
        let other = base.join("other");
        std::fs::create_dir_all(&other).unwrap();
        let project = ProjectRow {
            id: 1,
            owner_id: 1,
            name: "p".into(),
            room: None,
            source_kind: "local".into(),
            source_label: None,
            root_dir: root.clone(),
            config_file: root.join("yekitrim.toml"),
            session_id: None,
            created_at: 0,
            last_active_at: None,
            source_bytes: 0,
            output_bytes: 0,
            missing: false,
        };
        // 空 → 项目根
        assert_eq!(resolve_dir(&project, "").unwrap(), root);
        // 相对路径 → 项目内
        assert!(resolve_dir(&project, "output").is_ok());
        // 项目内绝对路径 → 允许（旧前端）
        assert!(resolve_dir(&project, &root.join("output").to_string_lossy()).is_ok());
        // 项目外绝对路径 → 404
        let e = resolve_dir(&project, &other.to_string_lossy()).unwrap_err();
        assert_eq!(e.0, StatusCode::NOT_FOUND);
        // 越界相对路径 → 400
        assert!(resolve_dir(&project, "../other").is_err());
        let _ = std::fs::remove_dir_all(&base);
    }

    fn test_project(root: &std::path::Path) -> ProjectRow {
        ProjectRow {
            id: 1,
            owner_id: 1,
            name: "p".into(),
            room: None,
            source_kind: "local".into(),
            source_label: None,
            root_dir: root.to_path_buf(),
            config_file: root.join("yekitrim.toml"),
            session_id: None,
            created_at: 0,
            last_active_at: None,
            source_bytes: 0,
            output_bytes: 0,
            missing: false,
        }
    }

    /// `source` / `cover.image` 的收口：项目内的（相对的、绝对的）放行，其余一律拒绝。
    ///
    /// 这条是"普通用户能读服务器上任意文件"那个洞的回归钉子：`source` 曾被
    /// `polish_cover_background` 直接 `PathBuf::from` + `fs::copy`，指向
    /// `<exe同级>/.vtrim/llm.toml` 就能把 LLM key 复制进自己的项目再读回来。
    #[test]
    fn resolve_file_in_project_jails_outside_paths() {
        let base = std::env::temp_dir().join(format!("vtrim-scope-file-{}", std::process::id()));
        let root = base.join("proj");
        std::fs::create_dir_all(&root).unwrap();
        let inside = root.join("bg.jpg");
        std::fs::write(&inside, b"x").unwrap();
        let other = base.join("other");
        std::fs::create_dir_all(&other).unwrap();
        let outside = other.join("secret.toml");
        std::fs::write(&outside, b"api_key = 'sk-real'").unwrap();
        let project = test_project(&root);

        // 项目内相对路径 → 归一化成项目内绝对路径
        // （比较要用 canonicalize 后的 root：Windows 上它带 `\\?\` 前缀，只解析一侧比不出来）
        let root_canon = root.canonicalize().unwrap();
        let got = resolve_file_in_project(&project, "bg.jpg", "source").unwrap();
        assert!(got.starts_with(&root_canon), "{}", got.display());
        // 项目内绝对路径 → 放行
        assert!(resolve_file_in_project(&project, &inside.to_string_lossy(), "source").is_ok());
        // 项目外绝对路径 → 404（这就是那个任意文件读取）
        let e = resolve_file_in_project(&project, &outside.to_string_lossy(), "source").unwrap_err();
        assert_eq!(e.0, StatusCode::NOT_FOUND);
        // 越界相对路径 → 400
        let e = resolve_file_in_project(&project, "../other/secret.toml", "source").unwrap_err();
        assert_eq!(e.0, StatusCode::BAD_REQUEST);
        // 盘符 / UNC 之类的词法越界 → 400
        assert!(resolve_file_in_project(&project, "C:/Windows/win.ini", "source").is_err());
        // 不存在 → 报错（两个字段的用途都是"读一个已存在的文件"）
        assert!(resolve_file_in_project(&project, "nope.jpg", "source").is_err());
        let _ = std::fs::remove_dir_all(&base);
    }

    /// 嵌套字段（`cover.image`）的读写：能取到、能就地改写，且不凭空新增字段。
    #[test]
    fn extra_path_fields_read_write_in_place() {
        let mut v = serde_json::json!({
            "dir": "output/clips/clip-01",
            "source": "bg.jpg",
            "cover": { "image": "_cover_frame.jpg", "font_size": 72 }
        });
        // 取嵌套值
        assert_eq!(json_get_str(&v, "cover.image"), Some("_cover_frame.jpg"));
        assert_eq!(json_get_str(&v, "source"), Some("bg.jpg"));
        // 缺失 / 类型不符 → None
        assert_eq!(json_get_str(&v, "cover.missing"), None);
        assert_eq!(json_get_str(&v, "cover.font_size"), None);
        assert_eq!(json_get_str(&v, "nope.image"), None);
        // 就地改写（不改其他键）
        json_set_str(&mut v, "source", "/abs/bg.jpg");
        json_set_str(&mut v, "cover.image", "/abs/frame.jpg");
        assert_eq!(json_get_str(&v, "source"), Some("/abs/bg.jpg"));
        assert_eq!(json_get_str(&v, "cover.image"), Some("/abs/frame.jpg"));
        assert_eq!(v["cover"]["font_size"], serde_json::json!(72));
        // 不凭空补字段：`cover` 不存在时什么都不做
        let mut bare = serde_json::json!({ "dir": "x" });
        json_set_str(&mut bare, "cover.image", "/abs/frame.jpg");
        assert!(bare.get("cover").is_none());
        assert_eq!(bare["dir"], serde_json::json!("x"));
    }
}
