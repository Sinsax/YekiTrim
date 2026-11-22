//! 表情包（本地贴纸库）管理：列表 / 上传导入 / LLM 压缩构建 / 删除 / 预览图。
//!
//! 库的位置**按归属**（`yekitrim_core::owner`）：单人（托盘 / `--local-no-auth`）=
//! `<数据目录>/resources/stickers/`；多用户 = `<数据目录>/users/<uid>/resources/stickers/`。
//! 一个子文件夹 = 一个包（`<包名>/*.gif`），包名即 LLM 选贴纸时的 `keyword`。
//!
//! ⚠️ **2026-10-09 起这些接口在用户路由组里，登录用户都能调**（原为 `require_admin`）。
//! 当时的理由写的是「素材库是用户级、跟随用户」；**2026-10-10 起换成了更硬的那条**：
//! 库真的按账号分了（`Owner`），所以「能不能改到别人的贴纸」这件事**结构上已不成立** ——
//! 甲改的是甲自己那棵树。
//!
//! 路由挂在 `routes.rs` 那个**不带 `enforce_scope`** 的 `assets` 组上（库不属于任何工程），
//! 未登录仍是 401。
//! ⚠️ **别再单方面往这一层加回 admin 门控** —— 那会让多用户下普通账号
//! **连自己的贴纸都传不了**。要限制就限制在"某个账号自己的库"上，而不是把接口对整个角色关掉。
//!
//! 「构建」复用 CLI 的那一份实现（`yekitrim_polish::commands::build_package_tags`）：
//! LLM 读文件名产出 10~20 个语义 tag 的反向索引 → 写 `stickers/<包名>.toml` +
//! 注册进 `stickers/sticker.toml`。**未构建的包不进索引**，也就不会被自动加进项目的
//! 贴纸来源（`fill_default_local_packages` 只认索引里的包）。
//!
//! 「添加」有两条入口，**都不合并、都不覆盖**已有的包（目标存在就报错让人换名字）：
//!
//! | 入口 | 来源 | UI |
//! |---|---|---|
//! | `POST /polish/stickers/upload` | 浏览器上传的 zip（请求体原始字节） | 有（首页「表情包」区块） |
//! | `POST /polish/stickers/import` | 服务器上一个已有目录（复制，来源原样留着） | 无，留给 curl/本地目录 |
//!
//! ⚠️ 上传那条是全站**第二处**有意接收用户输入的字节流（第一处是封面背景图），
//! 闸门（请求体上限 + 解压限值 + 只取 basename）都在 `pack_upload` 与
//! `yekitrim_polish::assets::sticker_zip` 里，别按「全站不接受上传」的旧规矩把它删掉。

use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Extension, Path as AxumPath, Query, Request, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

use yekitrim_polish::assets::sticker_index as si;
use yekitrim_polish::assets::sticker_zip as sz;

use super::auth::CurrentUser;
use super::model::AppState;

type AppError = (StatusCode, Json<serde_json::Value>);
type ApiResult<T = serde_json::Value> = Result<Json<T>, AppError>;

/// 上传请求体的**硬顶**（4 GiB）：`DefaultBodyLimit` 在路由构建时就要定死，
/// 而 `[stickers] upload_max_mb` 可以之后改。真正生效的上限由
/// `yekitrim_core::config::StickersConfig::upload_max_bytes` 算（默认 1 GiB），
/// 超过硬顶按硬顶算 —— 前端经 `GET /polish/stickers/limits` 拿实际值，
/// 超限的请求由 [`upload_limit_guard`] 在读 body 之前就拒掉。
///
/// 为什么要有前端预检：axum 超限时回的是 **413 + 纯文本**，而 `client.js` 只认 JSON，
/// 会把超限报成"接口不存在或返回格式错误"，用户看不出是文件太大。
pub(crate) const UPLOAD_HARD_MAX_BYTES: usize =
    yekitrim_core::config::StickersConfig::HARD_MAX_UPLOAD_MB as usize * 1024 * 1024;

/// 读**该 owner** 的 `[stickers]`（配置按账号分；同步 IO ⇒ 必须走 blocking 池）。
async fn sticker_limits(
    owner: yekitrim_core::owner::Owner,
) -> Result<yekitrim_core::config::StickersConfig, AppError> {
    super::api::blocking(move || Ok(yekitrim_core::config::load_config_for(owner).stickers)).await
}

/// 上传前的体积预检：`DefaultBodyLimit` 只有硬顶，配置值比硬顶小时必须**先看
/// `Content-Length` 再决定要不要读 body** —— 等 `Bytes` 提取器读完再判就晚了
/// （500 MB 的包已经躺在内存里了）。
///
/// 没有 `Content-Length`（chunked）时只能放过：那条路由仍有 4 GiB 的硬顶兜着。
pub async fn upload_limit_guard(
    Extension(user): Extension<CurrentUser>,
    req: Request,
    next: axum::middleware::Next,
) -> Response {
    let declared = req
        .headers()
        .get(axum::http::header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.trim().parse::<u64>().ok());
    if let Some(len) = declared {
        let owner = crate::auth::owner_of(&user);
        if let Ok(cfg) = sticker_limits(owner).await {
            let limit = cfg.upload_max_bytes();
            if len > limit as u64 {
                return (
                    StatusCode::PAYLOAD_TOO_LARGE,
                    Json(serde_json::json!({
                        "ok": false,
                        "error": format!(
                            "贴纸包超过 {} MB 上限（config.toml 的 [stickers] upload_max_mb）",
                            limit / 1024 / 1024
                        ),
                    })),
                )
                    .into_response();
            }
        }
    }
    next.run(req).await
}

/// `GET /api/polish/stickers/limits` — 当前账号的贴纸包体积上限（前端上传前预检用）。
pub async fn pack_limits(Extension(user): Extension<CurrentUser>) -> ApiResult {
    let cfg = sticker_limits(crate::auth::owner_of(&user)).await?;
    ok(serde_json::json!({
        "ok": true,
        "upload_max_bytes": cfg.upload_max_bytes(),
        "extract_max_bytes": cfg.extract_max_bytes(),
        "file_max_bytes": cfg.file_max_bytes(),
    }))
}

fn err(status: StatusCode, msg: impl Into<String>) -> AppError {
    (
        status,
        Json(serde_json::json!({ "ok": false, "error": msg.into() })),
    )
}

fn ok<T: serde::Serialize>(v: T) -> ApiResult<T> {
    Ok(Json(v))
}

/// 包目录：包名先过 `sanitize_pack_name`（挡 `..` / 分隔符 / Windows 保留名），
/// 已存在的目录再要求 canonicalize 后仍落在库里（防符号链接指到库外）。
fn pack_dir(root: &Path, pack: &str) -> Result<PathBuf, AppError> {
    let name = si::sanitize_pack_name(pack)
        .ok_or_else(|| err(StatusCode::BAD_REQUEST, format!("包名不合法: {pack}")))?;
    let dir = root.join(&name);
    if dir.exists() {
        let real = dir.canonicalize().unwrap_or_else(|_| dir.clone());
        let root_real = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
        if !real.starts_with(&root_real) {
            return Err(err(StatusCode::FORBIDDEN, "包目录不在贴纸库内"));
        }
    }
    Ok(dir)
}

/// 起一个 LLM 标签构建任务。资源域 = 全局贴纸库（`<程序目录>/stickers` 的
/// `<包名>.toml` 与 `sticker.toml` 索引），与另一个构建任务互斥（排队）。
///
/// 略重档 + 后台优先级：整段在等 LLM 网络，不该跟渲染抢重活名额，也不该插在用户点按的任务前面。
async fn spawn_build(
    state: &Arc<AppState>,
    owner: i64,
    data_owner: yekitrim_core::owner::Owner,
    pack: &str,
) -> Result<String, String> {
    let handle = state
        .jobs
        .create(
            owner,
            super::jobs::JobSpec::new(
                "sticker_build",
                format!("用 LLM 构建「{pack}」的标签"),
                vec![super::scope_key::STICKERS.to_string()],
                super::jobs::JobClass::Medium,
            )
            .background(),
        )
        .await;
    let id = handle.id.clone();
    let jobs = state.jobs.clone();
    let job_id = id.clone();
    let cancel = jobs.cancel_token(&id).await;
    // 表情包构建无管线阶段框架：expected 空（它经 stage_started("表情包标签") 自报阶段）
    let progress = super::api::core::progress_bridge(job_id.clone(), jobs.clone(), cancel, Vec::new());
    let pack = pack.to_string();
    // 贴纸标签构建要读整个包 + 跑多批 LLM（同步段很长）→ 专用任务线程
    super::api::spawn_job(jobs.clone(), job_id.clone(), move || async move {
        if handle.wait_start().await.is_err() {
            jobs.complete(&job_id).await;
            return;
        }
        // ⚠️ **必须传 `data_owner`，不能用 `build_package_tags`（那个走 `Owner::from_env()`）**：
        // 本任务跑在 **server 进程**里，而 server 的环境变量是它自己启动时那份 ——
        // 多用户下会去翻单人库，报「贴纸包不存在」，而列表里那个包好好地在着。
        match yekitrim_polish::commands::build_package_tags_for(
            data_owner,
            &pack,
            progress.as_ref(),
        )
        .await
        {
            Ok(r) => {
                jobs.log(
                    &job_id,
                    "info",
                    format!(
                        "「{pack}」标签已生成：{} 个文件 / {} 批 / {} 个 tag",
                        r.files, r.batches, r.tags
                    ),
                )
                .await;
                jobs.complete(&job_id).await;
            }
            // 用户取消时状态已是 Cancelled，fail() 会被忽略（jobs.rs）
            Err(e) => jobs.fail(&job_id, format!("{e:#}")).await,
        }
    });
    Ok(id)
}

/// `GET /api/polish/stickers` — 包列表（**以磁盘为准**：手工丢进库里的文件夹也会出现）。
pub async fn packs_list(
    Extension(user): Extension<CurrentUser>,
) -> ApiResult {
    let owner = crate::auth::owner_of(&user);
    let root = yekitrim_core::owner::asset_dir(owner, yekitrim_core::owner::AssetKind::Stickers);
    // ⚠️ **读两层**（用户库优先、全局补缺，2026-10-10）：公用贴纸包放在 `global/` 里，
    // 只看用户库会把它们整批漏掉（界面上表现为"公用素材凭空消失"）。
    let roots = yekitrim_core::owner::asset_dirs(
        owner,
        yekitrim_core::owner::AssetKind::Stickers,
    );
    let index = si::load_sticker_index(&root);
    let packs: Vec<serde_json::Value> = si::scan_packages_roots(&roots)
        .into_iter()
        .map(|pkg| {
            let meta = si::load_package_meta(&root, &pkg.name);
            let bytes: u64 = pkg
                .files
                .iter()
                .filter_map(|f| std::fs::metadata(f).ok())
                .map(|m| m.len())
                .sum();
            let tags: Vec<serde_json::Value> = meta
                .as_ref()
                .map(|m| {
                    m.tags
                        .iter()
                        .map(|t| serde_json::json!({ "tag": t.tag, "count": t.files.len() }))
                        .collect()
                })
                .unwrap_or_default();
            serde_json::json!({
                "name": pkg.name,
                "count": pkg.file_count(),
                "bytes": bytes,
                "built": meta.is_some(),
                "registered": index.packages.iter().any(|n| n == &pkg.name),
                "description": meta.as_ref().map(|m| m.description.clone()).unwrap_or_default(),
                "tags": tags,
            })
        })
        .collect();
    ok(serde_json::json!({
        "ok": true,
        "root": yekitrim_core::path::display(&root),
        "packs": packs,
    }))
}

/// `GET /api/polish/stickers/:pack` — 包内文件 + 每个文件命中的 tags（展开预览时才拉）。
pub async fn pack_detail(
    Extension(user): Extension<CurrentUser>,
    AxumPath(pack): AxumPath<String>,
) -> ApiResult {
    let owner = crate::auth::owner_of(&user);
    let root = yekitrim_core::owner::asset_dir(owner, yekitrim_core::owner::AssetKind::Stickers);
    let dir = pack_dir(&root, &pack)?;
    if !dir.is_dir() {
        return Err(err(StatusCode::NOT_FOUND, format!("贴纸包不存在: {pack}")));
    }
    let tag_map = si::file_tags_map(&root, &pack);
    let files: Vec<serde_json::Value> = si::list_package_files(&dir)
        .into_iter()
        .map(|name| {
            let size = std::fs::metadata(dir.join(&name)).map(|m| m.len()).unwrap_or(0);
            let tags = tag_map.get(&name).cloned().unwrap_or_default();
            serde_json::json!({ "name": name, "size": size, "tags": tags })
        })
        .collect();
    ok(serde_json::json!({ "ok": true, "pack": pack, "files": files }))
}

/// `POST /api/polish/stickers/import` — 把服务器上一个目录里的图片复制成新包。
///
/// `{dir, name?, build?}`：`name` 缺省取来源目录名；`build=true` 时顺带起构建任务，
/// 但**导入成功与构建任务是两件事** —— 构建起不来（已有任务在跑）只回一句 `job_error`。
pub async fn pack_import(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<CurrentUser>,
    Json(body): Json<serde_json::Value>,
) -> ApiResult {
    let owner = crate::auth::owner_of(&user);
    let dir = body
        .get("dir")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    if dir.is_empty() {
        return Err(err(StatusCode::BAD_REQUEST, "缺少来源目录 dir"));
    }
    let want_build = body.get("build").and_then(|v| v.as_bool()).unwrap_or(false);
    // 与「服务器文件」建项目同一条解析路径（管理员可指服务器上任意绝对路径）
    let src = yekitrim_service::projects::resolve_target_dir(user.id, "", &dir, true)
        .map_err(|e| err(StatusCode::BAD_REQUEST, format!("{e:#}")))?;
    let raw_name = body
        .get("name")
        .and_then(|v| v.as_str())
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .unwrap_or_else(|| {
            src.file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("")
                .to_string()
        });
    let name = si::sanitize_pack_name(&raw_name).ok_or_else(|| {
        err(
            StatusCode::BAD_REQUEST,
            format!("包名不合法: {raw_name}（1~32 个字母/数字/下划线/短横线/汉字）"),
        )
    })?;

    let root = yekitrim_core::owner::asset_dir(owner, yekitrim_core::owner::AssetKind::Stickers);
    let report = si::import_package(&root, &name, &src)
        .map_err(|e| err(StatusCode::BAD_REQUEST, format!("{e:#}")))?;

    let mut out = serde_json::json!({
        "ok": true,
        "name": name,
        "copied": report.copied,
        "skipped": report.skipped,
    });
    if want_build {
        match spawn_build(&state, user.id, crate::auth::owner_of(&user), &name).await {
            Ok(id) => out["job"] = serde_json::json!(id),
            Err(e) => out["job_error"] = serde_json::json!(e),
        }
    }
    Ok(Json(out))
}

/// `?name=<包名>&build=<0|1>` —— 走查询串而不是 JSON body，因为 body 被压缩包字节占满了。
#[derive(Debug, serde::Deserialize)]
pub struct UploadQuery {
    #[serde(default)]
    name: String,
    /// `1` / `true`（前端传 `1`，手写 curl 也方便）
    #[serde(default)]
    build: Option<String>,
}

/// 入参校验（纯函数，便于单测）：回 `(包名, 要不要构建)`。
fn parse_upload_args(q: &UploadQuery, body_len: usize) -> Result<(String, bool), AppError> {
    let raw_name = q.name.trim();
    if raw_name.is_empty() {
        return Err(err(StatusCode::BAD_REQUEST, "缺少包名 name"));
    }
    let name = si::sanitize_pack_name(raw_name).ok_or_else(|| {
        err(
            StatusCode::BAD_REQUEST,
            format!("包名不合法: {raw_name}（1~32 个字母/数字/下划线/短横线/汉字）"),
        )
    })?;
    if body_len == 0 {
        return Err(err(StatusCode::BAD_REQUEST, "上传内容为空"));
    }
    let build = matches!(
        q.build.as_deref().map(str::trim),
        Some("1") | Some("true") | Some("yes")
    );
    Ok((name, build))
}

/// `POST /api/polish/stickers/upload?name=<包名>&build=<0|1>` — 把上传的 zip 解压成新包。
///
/// 请求体就是压缩包的**原始字节**（`application/octet-stream`，不是 multipart）：
/// 单文件上传用 multipart 只是多一层边界解析，而前端 `api()` 本来就能直接传 `File`。
/// 体积上限分两层：路由挂 `DefaultBodyLimit::max(UPLOAD_HARD_MAX_BYTES)`（4 GiB 硬顶，
/// axum 默认只有 2 MB），真正生效的值来自 `[stickers] upload_max_mb`（默认 1 GiB），
/// 由 [`upload_limit_guard`] 在读 body 前按 `Content-Length` 预检。
///
/// 限额与安全规则全在 `yekitrim_polish::assets::sticker_zip`：只取 basename（zip-slip 天然失效）、
/// 条目数/单文件/总量三重闸门（后两层按 `[stickers]` 配置）、先解到暂存目录再 `rename`（不会留半个包）。
pub async fn pack_upload(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<CurrentUser>,
    Query(q): Query<UploadQuery>,
    body: Bytes,
) -> ApiResult {
    let owner = crate::auth::owner_of(&user);
    let cfg = sticker_limits(owner).await?;
    let limit = cfg.upload_max_bytes();
    if body.len() > limit {
        return Err(err(
            StatusCode::PAYLOAD_TOO_LARGE,
            format!(
                "贴纸包超过 {} MB 上限（config.toml 的 [stickers] upload_max_mb）",
                limit / 1024 / 1024
            ),
        ));
    }
    let (name, want_build) = parse_upload_args(&q, body.len())?;
    // 解压 + 写盘是同步 IO（现在可能到 GB 级），交给 blocking 池；Bytes 提取本身在 async 侧（那是网络读取）
    let root = yekitrim_core::owner::asset_dir(owner, yekitrim_core::owner::AssetKind::Stickers);
    let pack = name.clone();
    let zip_limits = sz::ZipLimits::with_sizes(cfg.file_max_bytes(), cfg.extract_max_bytes());
    let report = super::api::blocking(move || {
        sz::import_package_from_zip_with(&root, &pack, &body, &zip_limits)
            .map_err(|e| err(StatusCode::BAD_REQUEST, format!("{e:#}")))
    })
    .await?;

    let mut out = serde_json::json!({
        "ok": true,
        "name": name,
        "copied": report.copied,
        "skipped": report.skipped,
        "total_entries": report.total_entries,
    });
    // 与 pack_import 同一条规矩：导入成功与构建任务是两件事，构建起不来只回一句 job_error
    if want_build {
        match spawn_build(&state, user.id, crate::auth::owner_of(&user), &name).await {
            Ok(id) => out["job"] = serde_json::json!(id),
            Err(e) => out["job_error"] = serde_json::json!(e),
        }
    }
    Ok(Json(out))
}

/// `POST /api/polish/stickers/:pack/build` — LLM 压缩构建（覆盖 `<包名>.toml` 并注册）。
pub async fn pack_build(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<CurrentUser>,
    AxumPath(pack): AxumPath<String>,
) -> ApiResult {
    let owner = crate::auth::owner_of(&user);
    let root = yekitrim_core::owner::asset_dir(owner, yekitrim_core::owner::AssetKind::Stickers);
    let dir = pack_dir(&root, &pack)?;
    if !dir.is_dir() {
        return Err(err(StatusCode::NOT_FOUND, format!("贴纸包不存在: {pack}")));
    }
    let id = spawn_build(&state, user.id, crate::auth::owner_of(&user), &pack)
        .await
        .map_err(|e| err(StatusCode::CONFLICT, e))?;
    ok(serde_json::json!({ "ok": true, "id": id }))
}

/// `DELETE /api/polish/stickers/:pack` — 删掉整个包（目录 + 清单 + 索引项）。
pub async fn pack_delete(
    Extension(user): Extension<CurrentUser>,
    AxumPath(pack): AxumPath<String>,
) -> ApiResult {
    let owner = crate::auth::owner_of(&user);
    let root = yekitrim_core::owner::asset_dir(owner, yekitrim_core::owner::AssetKind::Stickers);
    let name = si::sanitize_pack_name(&pack)
        .ok_or_else(|| err(StatusCode::BAD_REQUEST, format!("包名不合法: {pack}")))?;
    let removed = si::remove_package(&root, &name)
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}")))?;
    ok(serde_json::json!({ "ok": true, "name": name, "removed": removed }))
}

/// `GET /api/polish/stickers/:pack/files/:name` — 包内图片（预览用，`<img>` 直连）。
pub async fn pack_file(
    Extension(user): Extension<CurrentUser>,
    AxumPath((pack, name)): AxumPath<(String, String)>,
    headers: axum::http::HeaderMap,
) -> Response {
    let owner = crate::auth::owner_of(&user);
    let root = yekitrim_core::owner::asset_dir(owner, yekitrim_core::owner::AssetKind::Stickers);
    let dir = match pack_dir(&root, &pack) {
        Ok(d) => d,
        Err(e) => return e.into_response(),
    };
    if !si::is_safe_component(&name) {
        return super::media::json_error(StatusCode::BAD_REQUEST, "文件名不合法");
    }
    let path = dir.join(&name);
    let real = path.canonicalize().unwrap_or_else(|_| path.clone());
    let root_real = root.canonicalize().unwrap_or_else(|_| root.clone());
    if !real.starts_with(&root_real) {
        return super::media::json_error(StatusCode::NOT_FOUND, "文件不在贴纸库内");
    }
    let mime = mime_guess::from_path(&path).first_or_octet_stream().to_string();
    super::media::serve_file_from_headers(&path, &mime, &headers).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pack_dir_rejects_traversal_and_accepts_plain_names() {
        let root = std::env::temp_dir();
        // 有分隔符 / `..` / 保留名一律是 400，不会拼到库外面去
        for bad in ["..", "../x", "a/b", "a\\b", "C:pack", ".hidden", ""] {
            let e = pack_dir(&root, bad).unwrap_err();
            assert_eq!(e.0, StatusCode::BAD_REQUEST, "{bad}");
        }
        let ok = pack_dir(&root, "ysyk").unwrap();
        assert!(ok.starts_with(&root));
    }

    fn q(name: &str, build: Option<&str>) -> UploadQuery {
        UploadQuery {
            name: name.to_string(),
            build: build.map(|s| s.to_string()),
        }
    }

    /// 上传入参：包名缺失/非法、body 为空都是 400；包名会被规范化（去空白）。
    #[test]
    fn upload_args_validate_name_and_body() {
        assert_eq!(parse_upload_args(&q("ysyk", None), 10).unwrap(), ("ysyk".into(), false));
        assert_eq!(
            parse_upload_args(&q("  komori  ", Some("1")), 10).unwrap(),
            ("komori".into(), true)
        );
        for truthy in ["1", "true", "yes", " true "] {
            assert!(parse_upload_args(&q("a", Some(truthy)), 1).unwrap().1, "{truthy}");
        }
        for falsy in ["0", "false", "", "no", "2"] {
            assert!(!parse_upload_args(&q("a", Some(falsy)), 1).unwrap().1, "{falsy}");
        }

        assert_eq!(parse_upload_args(&q("", None), 10).unwrap_err().0, StatusCode::BAD_REQUEST);
        assert_eq!(parse_upload_args(&q("   ", None), 10).unwrap_err().0, StatusCode::BAD_REQUEST);
        assert_eq!(
            parse_upload_args(&q("../x", None), 10).unwrap_err().0,
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            parse_upload_args(&q("a/b", None), 10).unwrap_err().0,
            StatusCode::BAD_REQUEST
        );
        // 名字合法但一个字节都没上传（前端不该发，服务端也不能当成功）
        assert_eq!(parse_upload_args(&q("ysyk", None), 0).unwrap_err().0, StatusCode::BAD_REQUEST);
    }

    /// 上限是配置项：默认 1 GiB、硬顶 4 GiB（`DefaultBodyLimit` 只能按硬顶挂）。
    #[test]
    fn upload_limit_defaults_to_1_gib_with_4_gib_ceiling() {
        let d = yekitrim_core::config::StickersConfig::default();
        assert_eq!(d.upload_max_bytes(), 1024 * 1024 * 1024);
        assert_eq!(d.extract_max_bytes(), 2048 * 1024 * 1024);
        assert_eq!(d.file_max_bytes(), 64 * 1024 * 1024);
        assert_eq!(UPLOAD_HARD_MAX_BYTES, 4096 * 1024 * 1024);
        // 0 与超硬顶都收敛到有界值（不许出现"0 = 不限"）
        let zero = yekitrim_core::config::StickersConfig {
            upload_max_mb: 0,
            extract_max_mb: 0,
            file_max_mb: 0,
        };
        assert_eq!(zero.upload_max_bytes(), 1024 * 1024 * 1024);
        let huge = yekitrim_core::config::StickersConfig {
            upload_max_mb: 999_999,
            ..Default::default()
        };
        assert_eq!(huge.upload_max_bytes(), UPLOAD_HARD_MAX_BYTES);
    }
}
