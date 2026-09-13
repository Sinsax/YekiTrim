//! Media 访问（多用户服务 P2）：只服务**本人项目内**的文件。
//!
//! URL 形态：
//! - 新契约（推荐）`/api/media/<project_id>/<项目内相对路径>`，例如
//!   `/api/media/7/output/clips/clip-01/clip.mp4`；
//! - 过渡期兼容：`/api/media/<绝对路径>`（旧前端 `mediaUrl(dir, file)` 的写法）——
//!   只有在 canonicalize 后落在**调用者自己的某个项目根**之内时才放行，否则 403/404。
//!
//! 归属校验在 `api::media_get` 里完成（依赖当前登录用户）。

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

/// 拆分 `/api/media/<...>` 的路径部分。
///
/// 返回 `(Some(project_id), 项目内相对路径)` 或 `(None, 原始路径)`（过渡期绝对路径）。
pub fn split_project_media(path: &str) -> (Option<i64>, String) {
    match path.split_once('/') {
        Some((head, rest)) => match head.parse::<i64>() {
            Ok(id) if id > 0 => (Some(id), rest.to_string()),
            _ => (None, path.to_string()),
        },
        None => match path.parse::<i64>() {
            Ok(id) if id > 0 => (Some(id), String::new()),
            _ => (None, path.to_string()),
        },
    }
}

pub fn json_error(status: StatusCode, msg: &str) -> Response {
    (
        status,
        axum::Json(serde_json::json!({ "ok": false, "error": msg })),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_project_scoped_and_legacy_paths() {
        assert_eq!(
            split_project_media("7/output/clips/clip-01/clip.mp4"),
            (Some(7), "output/clips/clip-01/clip.mp4".to_string())
        );
        assert_eq!(split_project_media("12"), (Some(12), String::new()));
        // 旧前端：绝对路径（首段不是数字）
        let (id, rest) = split_project_media("F:/media/3d/output/clips/clip-01/clip.mp4");
        assert_eq!(id, None);
        assert_eq!(rest, "F:/media/3d/output/clips/clip-01/clip.mp4");
        // 0 / 负数不算项目 id，按旧路径处理
        assert_eq!(split_project_media("0/x").0, None);
        assert_eq!(split_project_media("-3/x").0, None);
    }
}
