//! Media / 文件访问工具：受限路径校验，避免 HTTP 客户端读取任意文件。

use std::path::{Path, PathBuf};

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

use crate::service::Workspace;

/// 判定某个请求路径是否允许被静态/媒体服务访问。
/// 允许范围：
/// - 工作区根目录
/// - 工程根目录
/// - 工程输出目录
/// - 输入视频所在目录
pub fn allowed_roots(ws: &Workspace, workspace_root: &Path) -> Vec<PathBuf> {
    let mut roots = vec![workspace_root.to_path_buf(), ws.root_dir.clone()];
    if let Ok(cfg) = ws.load_config() {
        if let Some(p) = cfg.project.input_file.as_ref().filter(|s| !s.is_empty()) {
            let abs = ws.resolve(p);
            if let Some(parent) = abs.parent() {
                roots.push(parent.to_path_buf());
            }
        }
        let out = ws.resolve(&cfg.cutting.output_dir);
        roots.push(out);
    }
    roots
}

/// 将查询参数中的相对/绝对路径规范化为绝对路径，并在允许根内。
pub fn safe_join(ws: &Workspace, workspace_root: &Path, requested: &str) -> Option<PathBuf> {
    let path = if Path::new(requested).is_absolute() {
        PathBuf::from(requested)
    } else {
        ws.resolve(requested)
    };
    if !path.exists() {
        return None;
    }
    // 规范化后必须位于某个允许根下。
    for root in allowed_roots(ws, workspace_root) {
        if let Ok(rel) = path.strip_prefix(&root) {
            // 禁止逃逸 ../
            if !rel.components().any(|c| matches!(c, std::path::Component::ParentDir)) {
                return Some(path);
            }
        }
    }
    None
}

pub fn error_response(msg: &str) -> Response {
    (
        StatusCode::BAD_REQUEST,
        axum::Json(serde_json::json!({ "ok": false, "error": msg })),
    )
        .into_response()
}
