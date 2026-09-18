//! WebUI 静态资源服务。
//!
//! 只服务 `webui/dist`（Vue3 构建产物），构建命令：
//! `cd webui && npm install && npm run build`。
//!
//! 目录缺失时返回 503 + 构建指引：**不再回退到内嵌单文件页面**（那份页面长期
//! 与 API 漂移，已删除；见 AGENTS「WebUI」一节）。

use std::path::{Component, Path, PathBuf};

use axum::body::Body;
use axum::http::{Request, Response, StatusCode, header};
use axum::response::IntoResponse;

/// `dist` 候选目录，按优先级：
/// 1. 仓库内 `webui/dist`（开发/源码运行）
/// 2. 可执行文件同级 `webui/dist`（发行包）
/// 3. 当前工作目录下 `webui/dist`
///
/// ⚠️ 第 1 条的 `../..` 是**按 crate 深度**算的：`crates/vtrim-server` → 仓库根。
/// 本 crate 从 `crates/vtrim-cli/src/server/` 搬出来时深度没变（都是 `crates/<crate>`），
/// 所以路径照旧。**别把本 crate 挪到更深的目录**（如 `crates/server/vtrim-server`）——
/// 那时 `../..` 指到 `crates/`，WebUI 会静默 404（构建与测试都不报错）。
fn candidates() -> Vec<PathBuf> {
    let mut out = vec![PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("webui")
        .join("dist")];
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            out.push(dir.join("webui").join("dist"));
        }
    }
    out.push(PathBuf::from("webui").join("dist"));
    out
}

/// 实际使用的 dist 目录：第一个含 `index.html` 的候选；都不存在时返回第一个候选
/// （仅用于错误提示，调用方应先检查 [`dist_ready`]）。
pub fn dist_dir() -> PathBuf {
    let cands = candidates();
    for c in &cands {
        if c.join("index.html").is_file() {
            return c.clone();
        }
    }
    cands.into_iter().next().unwrap_or_else(|| PathBuf::from("webui/dist"))
}

/// 启动时判断 WebUI 是否可用（缺失只提示，不阻止 server 启动）。
pub fn dist_ready() -> bool {
    dist_dir().join("index.html").is_file()
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

/// 缺少构建产物时的可读指引（503）。
fn build_hint_page() -> Response<Body> {
    let locations = candidates()
        .iter()
        .map(|p| format!("<li><code>{}</code></li>", p.display()))
        .collect::<String>();
    let body = format!(
        r#"<!doctype html>
<html lang="zh-CN"><head><meta charset="utf-8"><title>V-Trim WebUI 未构建</title>
<style>
 body{{font-family:system-ui,"Microsoft YaHei",sans-serif;max-width:760px;margin:64px auto;padding:0 20px;line-height:1.8;color:#1f2937}}
 code,pre{{background:#f3f4f6;border-radius:6px}}
 code{{padding:2px 6px}}
 pre{{padding:12px 14px;overflow:auto}}
 h1{{font-size:20px}}
</style></head><body>
<h1>WebUI 静态资源未就绪</h1>
<p>服务端只提供 Vue 构建产物 <code>webui/dist</code>（已不再内嵌单文件页面），但没有找到 <code>index.html</code>。</p>
<p>查找位置（按优先级）：</p>
<ol>{locations}</ol>
<p>请在仓库根目录执行：</p>
<pre><code>cd webui
npm install
npm run build</code></pre>
<p>发行包请把 <code>webui/dist</code> 放到可执行文件同级目录；源码运行时直接在仓库根目录执行 <code>vtrim server</code> 即可。</p>
</body></html>"#
    );
    Response::builder()
        .status(StatusCode::SERVICE_UNAVAILABLE)
        .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
        .body(Body::from(body))
        .unwrap_or_else(|_| StatusCode::SERVICE_UNAVAILABLE.into_response())
}

fn index_html(dist: &Path) -> Response<Body> {
    if let Ok(bytes) = std::fs::read(dist.join("index.html")) {
        // no-cache：入口 html 必须每次回源校验，否则发版后浏览器启发式缓存会
        // 让用户停在旧界面（入口引用的是内容哈希资源名，资源本身可长缓存）。
        return Response::builder()
            .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
            .header(header::CACHE_CONTROL, "no-cache")
            .body(Body::from(bytes))
            .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response());
    }
    build_hint_page()
}

/// SPA 静态资源 + index 回退。
pub async fn webui_fallback(req: Request<Body>) -> Response<Body> {
    let dist = dist_dir();
    if !dist_ready() {
        return build_hint_page();
    }

    let uri_path = req.uri().path();
    let rel = uri_path.trim_start_matches('/');
    let file = if rel.is_empty() {
        dist.join("index.html")
    } else {
        // 防目录穿越：只允许 dist 内部的**普通路径分量**。
        // 仅比较 `starts_with` 不够——`dist/../../secret` 在词法上仍以 dist 开头。
        let rel_path = Path::new(rel);
        if rel_path.components().any(|c| !matches!(c, Component::Normal(_))) {
            return StatusCode::FORBIDDEN.into_response();
        }
        let candidate = dist.join(rel_path);
        if !candidate.starts_with(&dist) {
            return StatusCode::FORBIDDEN.into_response();
        }
        candidate
    };

    if file.is_file() {
        if let Ok(bytes) = std::fs::read(&file) {
            let mut b = Response::builder().header(header::CONTENT_TYPE, mime_for(&file));
            // 入口 html（含 SPA 回退命中 index.html 的请求）必须 no-cache，理由见 index_html
            if file.file_name().and_then(|n| n.to_str()) == Some("index.html") {
                b = b.header(header::CACHE_CONTROL, "no-cache");
            }
            return b
                .body(Body::from(bytes))
                .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response());
        }
    }

    // SPA 回退：hash 路由下通常不会请求深层路径，但保留 index 回退更稳妥
    index_html(&dist)
}
