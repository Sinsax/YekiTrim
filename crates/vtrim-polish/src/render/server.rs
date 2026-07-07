use axum::{Router, routing::get, response::Response, body::Body};
use tower_http::services::ServeDir;
use std::path::{Path, PathBuf};

const INDEX_HTML: &str = include_str!("../../templates/index.html");

/// 构建 irasu 映射: stem → URL 路径
pub fn build_irasu_map(project_path: &Path) -> serde_json::Map<String, serde_json::Value> {
    let irasu_dir = project_path.join("assets").join("irasu");
    let mut map = serde_json::Map::new();
    if irasu_dir.exists() {
        if let Ok(entries) = std::fs::read_dir(&irasu_dir) {
            for entry in entries.flatten() {
                if let Some(name) = entry.file_name().to_str() {
                    if let Some(stem) = Path::new(name).file_stem().and_then(|s| s.to_str()) {
                        map.insert(stem.to_string(), serde_json::Value::String(format!("/assets/irasu/{}", name)));
                    }
                }
            }
        }
    }
    map
}

pub struct RenderServer {
    pub port: u16,
    project_path: PathBuf,
}

/// 启动一个只提供静态文件的 HTTP server，绑定到随机端口
/// 返回 port。用于每个 chunk 独享的静态文件服务。
pub async fn start_file_server(project_path: PathBuf, injected_html: Option<String>) -> anyhow::Result<u16> {
    let path = project_path.clone();
    let sd_path = project_path.clone();
    let app = Router::new()
        .route("/", get(move || {
            let p = path.clone();
            let html = injected_html.clone();
            async move {
                let body = match html {
                    Some(ref h) => h.clone(),
                    None => std::fs::read_to_string(p.join("index.html")).unwrap_or_default(),
                };
                Response::builder()
                    .header("Content-Type", "text/html;charset=utf-8")
                    .body(Body::from(body))
                    .unwrap()
            }
        }))
        .fallback_service(ServeDir::new(&sd_path));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let port = listener.local_addr()?.port();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    Ok(port)
}

impl RenderServer {
    pub fn new(project_path: PathBuf) -> Self {
        Self { port: 0, project_path }
    }

    pub fn build_html(project_path: &Path) -> String {
        let config_path = project_path.join("polish.toml");
        let mut cfg: serde_json::Value = std::fs::read_to_string(&config_path).ok()
            .and_then(|raw| toml::from_str(&raw).ok())
            .unwrap_or(serde_json::Value::Object(Default::default()));
        let assets_dir = project_path.join("assets");
        let emoji_dir = assets_dir.join("emoji");
        let has_local = emoji_dir.exists() && std::fs::read_dir(&emoji_dir).map(|mut e| e.any(|f| f.is_ok())).unwrap_or(false);
        cfg["_assets"] = serde_json::json!({
            "emoji_base": if has_local { "/assets/emoji/" } else { "https://fonts.gstatic.com/s/e/notoemoji/latest" },
            "emoji_local": has_local,
            "irasu_map": build_irasu_map(project_path),
        });
        Self::inject_config_tag(&cfg, INDEX_HTML)
    }

    fn inject_config_tag(cfg: &serde_json::Value, html: &str) -> String {
        let config_json = serde_json::to_string(cfg).unwrap_or_else(|_| "{}".to_string());
        let config_tag = format!(
            r#"<script id="polish-config" type="application/json">{}</script>"#,
            config_json
        );
        html.replace("<!-- __CONFIG__ -->", &config_tag)
    }

    pub async fn start(&mut self) -> anyhow::Result<u16> {
        let project_path = self.project_path.clone();
        let project_path_sd = project_path.clone();
        let app = Router::new()
            .route("/", get(move || {
                let html = Self::build_html(&project_path);
                async move {
                    Response::builder()
                        .header("Content-Type", "text/html;charset=utf-8")
                        .body(Body::from(html))
                        .unwrap()
                }
            }))
            .fallback_service(ServeDir::new(&project_path_sd));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let port = listener.local_addr()?.port();
        self.port = port;
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        Ok(port)
    }
}
