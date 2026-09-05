//! WebUI 内嵌静态资源与回退路由。

use axum::response::Html;

const INDEX_HTML: &str = include_str!("webui/index.html");

pub async fn webui_fallback() -> Html<&'static str> {
    Html(INDEX_HTML)
}

