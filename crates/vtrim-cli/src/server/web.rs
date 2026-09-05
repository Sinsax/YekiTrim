//! WebUI 静态资源服务。
//!
//! 优先从 `webui/dist`（Vue3 构建产物）读取；若目录不存在则回退到旧的单文件内嵌页面。

use std::path::{Path, PathBuf};

use axum::body::Body;
use axum::http::{Request, Response, StatusCode, header};
use axum::response::{Html, IntoResponse};

const INDEX_HTML: &str = include_str!("webui/index.html");

fn dist_dir() -> PathBuf {
    // 编译时定位仓库根：crates/vtrim-cli -> repo root -> webui/dist
    let from_manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("webui")
        .join("dist");
    if from_manifest.is_dir() {
        return from_manifest;
    }
    // 运行时兜底：从当前工作目录找
    PathBuf::from("webui/dist")
}

fn mime_for(path: &Path) -> String {
    match path.extension().and_then(|e| e.to_str()).map(|e| e.to_ascii_lowercase()).as_deref() {
        Some("html") => "text/html; charset=utf-8".to_string(),
        Some("js") | Some("mjs") => "text/javascript; charset=utf-8".to_string(),
        Some("css") => "text/css; charset=utf-8".to_string(),
        Some("json") => "application/json; charset=utf-8".to_string(),
        Some("svg") => "image/svg+xml".to_string(),
        Some("png") => "image/png".to_string(),
        Some("jpg") | Some("jpeg") => "image/jpeg".to_string(),
        Some("webp") => "image/webp".to_string(),
        Some("gif") => "image/gif".to_string(),
        Some("ico") => "image/x-icon".to_string(),
        Some("woff") => "font/woff".to_string(),
        Some("woff2") => "font/woff2".to_string(),
        Some("ttf") => "font/ttf".to_string(),
        Some("map") => "application/json".to_string(),
        _ => mime_guess::from_path(path).first_or_octet_stream().to_string(),
    }
}

fn index_html() -> Response<Body> {
    let dist = dist_dir();
    let index_path = dist.join("index.html");
    if index_path.is_file() {
        if let Ok(bytes) = std::fs::read(&index_path) {
            return Response::builder()
                .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
                .body(Body::from(bytes))
                .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response());
        }
    }
    Html(INDEX_HTML).into_response()
}

pub async fn webui_fallback(req: Request<Body>) -> Response<Body> {
    let dist = dist_dir();
    if !dist.is_dir() {
        return index_html();
    }

    let uri_path = req.uri().path();
    let rel = uri_path.trim_start_matches('/');
    let file = if rel.is_empty() {
        dist.join("index.html")
    } else {
        // 防目录穿越：只允许 dist 内部文件
        let candidate = dist.join(rel);
        if !candidate.starts_with(&dist) {
            return StatusCode::FORBIDDEN.into_response();
        }
        candidate
    };

    if file.is_file() {
        if let Ok(bytes) = std::fs::read(&file) {
            return Response::builder()
                .header(header::CONTENT_TYPE, mime_for(&file))
                .body(Body::from(bytes))
                .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response());
        }
    }

    // SPA 回退：hash 路由下通常不会请求深层路径，但保留 index 回退更稳妥
    index_html()
}
