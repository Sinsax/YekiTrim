//! `api::project_data::fs`：`POST /api/fs/list`（服务器已有文件的浏览，仅 admin）。

use std::path::Path;

use axum::http::StatusCode;

use axum::Json;

// 跨子模块 / 外部域：显式点名（不许 glob）
use axum::extract::Extension;

use super::*;

/// `POST /api/fs/list {dir}`：**仅 admin**，浏览服务器任意目录（与单用户时代一致）。
///
/// - `dir` 可以是**任意绝对路径**（管理员），缺省 → 租户根；也兼容 `{root, path}`
///   （path 相对所选根，囚笼在内）；
/// - 只做目录列举，**永不接受上传**；
/// - 响应回传 `roots`（租户根 + `[storage] local_roots`，作为快捷入口）、`dir` 与 `parent`
///   （绝对父目录，供"上一级"一路走到盘符根）。
/// 可供浏览的**卷/盘符**（给"服务器文件"浏览器顶部的盘符入口）。
///
/// ⚠️ 这只是**展示用的候选**，**不构成访问限制** —— `/api/fs/list` 仍然允许 admin 浏览任意绝对路径，
/// `[storage] local_roots` 也仍然只是快捷入口。没有它的话，浏览器的目录树只能从"当前路径所在那个盘"
/// 往上展开（`parent` 到盘根就断了），用户**换不了盘**。
///
/// Windows 枚举 `A:`–`Z:` 里真实存在的；其它平台只给文件系统根 `/`。
fn available_volumes() -> Vec<serde_json::Value> {
    #[cfg(windows)]
    {
        let mut out = Vec::new();
        for c in b'A'..=b'Z' {
            let letter = c as char;
            let root = format!("{letter}:\\");
            if std::path::Path::new(&root).is_dir() {
                out.push(serde_json::json!({ "path": root, "label": format!("{letter}:") }));
            }
        }
        out
    }
    #[cfg(not(windows))]
    {
        vec![serde_json::json!({ "path": "/", "label": "/" })]
    }
}

pub(crate) async fn fs_list(
    Extension(user): Extension<crate::auth::CurrentUser>,
    Json(body): Json<serde_json::Value>,
) -> ApiResult {
    if !user.is_admin() {
        return Err(err(StatusCode::FORBIDDEN, "仅管理员可浏览服务器文件"));
    }
    // 允许的根 = 租户根 + [storage] local_roots（这些只是"快捷入口"；管理员可用任意绝对路径）
    //
    // ⚠️ 这里的"租户根"是**项目托管根** `<projects_root>/users/<uid>`（= `local_roots[0]`），
    // 不是按账号分的素材/配置根 `<data>/users/<uid>`（`owner::owner_root`）—— 两者是不同目录。
    // 用后者去建目录会把**默认浏览根**留在不存在的位置：`dir=""` 时 `pick_local_root` 取前者，
    // 随后 `!target.is_dir()` 直接回 400「只能浏览目录」。全新部署（还没有任何项目）必现，
    // 2026-10 的"服务器文件 / 浏览录像打不开"就是这么来的。
    let tenant = yekitrim_service::projects::tenant_root(user.id);
    std::fs::create_dir_all(&tenant)
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("创建租户目录失败: {e}")))?;
    let roots = yekitrim_service::projects::local_roots(user.id);
    let is_admin = user.is_admin();
    let dir_hint = body
        .get("dir")
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let root_hint = body.get("root").and_then(|v| v.as_str()).unwrap_or("");
    let path_hint = body.get("path").and_then(|v| v.as_str()).unwrap_or("");
    let bad = |e: anyhow::Error| err(StatusCode::BAD_REQUEST, format!("{e:#}"));
    let target = match dir_hint {
        // 新契约：直接给目录（绝对路径 = 任意位置；相对路径 = 相对快捷根）
        Some(d) => {
            yekitrim_service::projects::resolve_target_dir(user.id, root_hint, &d, is_admin)
                .map_err(bad)?
        }
        // 兼容契约：{root, path}
        None if Path::new(path_hint.trim()).is_absolute() => {
            yekitrim_service::projects::resolve_target_dir(user.id, root_hint, path_hint, is_admin)
                .map_err(bad)?
        }
        None => {
            let root = yekitrim_service::projects::pick_local_root(user.id, root_hint, is_admin)
                .map_err(bad)?;
            if path_hint.trim().is_empty() {
                root
            } else {
                yekitrim_service::projects::resolve_target_dir(user.id, "", path_hint, is_admin)
                    .map_err(bad)?
            }
        }
    };
    // `enter`：进入下一级目录（**名字由服务端拼接**，前端不再做字符串加法，
    // 否则 `\\?\F:\a` + "/b" 会拼成非法路径 → ERROR_INVALID_NAME 123）
    let target = match body
        .get("enter")
        .and_then(|v| v.as_str())
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
    {
        None => target,
        Some(name) => {
            if name.contains('/')
                || name.contains('\\')
                || name.contains(':')
                || name == "."
                || name == ".."
            {
                return Err(err(StatusCode::BAD_REQUEST, format!("非法目录名: {name}")));
            }
            let child = target.join(name);
            if !child.is_dir() {
                return Err(err(
                    StatusCode::BAD_REQUEST,
                    format!("目录不存在: {name}（在 {} 下）", yekitrim_core::path::display(&target)),
                ));
            }
            child
        }
    };
    if !target.is_dir() {
        return Err(err(StatusCode::BAD_REQUEST, "只能浏览目录"));
    }
    let cur = target.canonicalize().unwrap_or_else(|_| target.clone());
    // **回到"人能读、能回传"的形式**：去掉 canonicalize 产生的 `\\?\` 前缀
    let cur_str = yekitrim_core::path::display(&cur);
    // 绝对父目录：让前端"上一级"能一路走到盘符根（管理员随意浏览）
    let parent = cur
        .parent()
        .map(|p| yekitrim_core::path::display(p))
        .unwrap_or_default();
    // 目录列举是阻塞 IO（网络盘/大目录会明显卡顿），放 spawn_blocking
    let scan_dir = target.clone();
    let (dirs, files, volumes) = tokio::task::spawn_blocking(move || -> std::io::Result<(Vec<String>, Vec<String>, Vec<serde_json::Value>)> {
        let mut files = vec![]; let mut dirs = vec![];
        for e in std::fs::read_dir(&scan_dir)? {
            let e = e?;
            if e.file_type().map(|t| t.is_dir()).unwrap_or(false) { dirs.push(e.file_name().to_string_lossy().to_string()); }
            else { files.push(e.file_name().to_string_lossy().to_string()); }
        }
        dirs.sort(); files.sort();
        // 盘符探测也是阻塞 IO（每个候选要 stat 一次），跟目录列举一起放这里
        Ok((dirs, files, available_volumes()))
    })
    .await
    .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("读取失败: {e}")))?
    .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("读取失败: {e}")))?;
    let root_list: Vec<serde_json::Value> = roots
        .iter()
        .map(|r| {
            // 显示形式（去掉 \\?\ 前缀）；前端原样回传即可命中快捷入口
            let canon = r.canonicalize().unwrap_or_else(|_| r.clone());
            serde_json::json!({
                "path": yekitrim_core::path::display(&canon),
                "label": yekitrim_service::projects::root_label(user.id, &canon),
                "tenant": canon == tenant.canonicalize().unwrap_or_else(|_| tenant.clone()),
            })
        })
        .collect();
    // 当前目录若不是快捷入口之一，也追加进下拉（下拉始终能显示"当前位置"）
    let mut root_list = root_list;
    if !root_list.iter().any(|x| x["path"].as_str() == Some(cur_str.as_str())) {
        root_list.push(serde_json::json!({
            "path": cur_str,
            "label": cur
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| cur_str.clone()),
            "tenant": false,
        }));
    }
    // 兼容旧前端：相对某个快捷根的路径
    let rel = roots
        .iter()
        .filter_map(|r| r.canonicalize().ok())
        .find_map(|r| {
            cur.strip_prefix(&r)
                .ok()
                .map(|p| p.to_string_lossy().replace('\\', "/"))
        })
        .unwrap_or_default();
    ok(serde_json::json!({
        "dir": cur_str,
        "root": cur_str,
        "root_label": yekitrim_service::projects::root_label(user.id, &cur),
        "roots": root_list,
        // 盘符候选：前端把它铺在目录树顶部，否则用户换不了盘（见 available_volumes 注释）
        "volumes": volumes,
        "tenant_root": yekitrim_core::path::display(&tenant),
        "path": rel,
        "parent": parent,
        "dirs": dirs,
        "files": files,
    }))
}
