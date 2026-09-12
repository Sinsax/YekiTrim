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
    // 多项目模式下，允许访问所有已记住项目的根目录。
    // 否则 F:ideo\... 这类不在默认 workspace_root 下的项目，媒体预览会 400。
    if let Some(state) = crate::server::persist::load_state() {
        if let Some(last) = state.last_config_file.as_deref() {
            if let Some(parent) = Path::new(last).parent() {
                roots.push(parent.to_path_buf());
            }
        }
        for cfg in &state.projects {
            if let Some(parent) = Path::new(cfg).parent() {
                roots.push(parent.to_path_buf());
            }
        }
    }
    roots
}

/// 路径校验结果：区分「不存在」（404）与「越权」（403），
/// 旧实现把两者合并成 400，客户端无法区分"还没生成"和"不许访问"。
pub enum SafePath {
    Allowed(PathBuf),
    NotFound,
    Forbidden,
}

/// 将查询参数中的相对/绝对路径规范化为绝对路径，并校验其位于允许根内。
pub fn safe_join(ws: &Workspace, workspace_root: &Path, requested: &str) -> SafePath {
    let path = if Path::new(requested).is_absolute() {
        PathBuf::from(requested)
    } else {
        ws.resolve(requested)
    };
    if !path.exists() {
        return SafePath::NotFound;
    }
    // 规范化后必须位于某个允许根下。
    for root in allowed_roots(ws, workspace_root) {
        if let Ok(rel) = path.strip_prefix(&root) {
            // 禁止逃逸 ../
            if !rel.components().any(|c| matches!(c, std::path::Component::ParentDir)) {
                return SafePath::Allowed(path);
            }
        }
    }
    SafePath::Forbidden
}

pub fn json_error(status: StatusCode, msg: &str) -> Response {
    (
        status,
        axum::Json(serde_json::json!({ "ok": false, "error": msg })),
    )
        .into_response()
}
