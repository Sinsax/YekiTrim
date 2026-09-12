use axum::{
    Router,
    routing::{get, post},
    response::Response,
    body::Body,
    extract::Json,
    http::StatusCode,
};
use tower_http::services::ServeDir;
use std::path::{Path, PathBuf};

const INDEX_HTML: &str = include_str!("../../templates/index.html");

/// 构建 sticker 映射: stem → URL 路径
pub fn build_sticker_map(project_path: &Path) -> serde_json::Map<String, serde_json::Value> {
    let sticker_dir = project_path.join("assets").join("stickers");
    let mut map = serde_json::Map::new();
    if sticker_dir.exists()
        && let Ok(entries) = std::fs::read_dir(&sticker_dir) {
            for entry in entries.flatten() {
                if let Some(name) = entry.file_name().to_str()
                    && let Some(stem) = Path::new(name).file_stem().and_then(|s| s.to_str()) {
                        map.insert(stem.to_string(), serde_json::Value::String(format!("/assets/stickers/{}", name)));
                    }
            }
        }
    map
}

/// 在线编辑器保存接口：接收完整 segments 数组并写回 polish.toml
async fn save_config_handler(
    project_path: PathBuf,
    Json(payload): Json<serde_json::Value>,
) -> (StatusCode, Json<serde_json::Value>) {
    let segments: Vec<crate::project::Segment> = match payload.get("segments").cloned() {
        Some(v) => match serde_json::from_value(v) {
            Ok(s) => s,
            Err(e) => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({ "ok": false, "error": format!("segments 解析失败: {e}") })),
                );
            }
        },
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "ok": false, "error": "缺少 segments 字段" })),
            );
        }
    };

    // polish.toml 的 read-modify-write：与 WebUI / CLI 的其它写入串行化
    let polish_toml = project_path.join("polish.toml");
    let result = vtrim_core::filelock::with_path_lock(&polish_toml, || -> anyhow::Result<()> {
        let mut cfg = crate::project::load_project(&project_path)?;
        cfg.segments = segments;
        std::fs::write(&polish_toml, cfg.to_toml_commented()?)?;
        Ok(())
    })
    .await;

    match result {
        Ok(()) => (
            StatusCode::OK,
            Json(serde_json::json!({ "ok": true, "message": "已保存" })),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "ok": false, "error": e.to_string() })),
        ),
    }
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
        cfg["_assets"] = serde_json::json!({
            "sticker_map": build_sticker_map(project_path),
        });
        // 竖屏检测: 视频高 > 宽 时启用竖屏布局 (主视频左/中放置 + 模糊背景)
        if let Some(video_rel) = cfg["source"]["video"].as_str()
            && let Some(size) = crate::render::get_video_size(
                &project_path.join(video_rel).to_string_lossy())
            && size.1 > size.0
        {
            let pos = cfg["layout"]["vertical_position"]
                .as_str().unwrap_or("left").to_string();
            cfg["vertical"] = serde_json::json!(true);
            cfg["vertical_position"] = serde_json::json!(pos);
            tracing::info!("  v 竖屏视频 {}x{} → {} 布局 + 模糊背景", size.0, size.1, pos);
        }
        // 响度曲线: 注入浏览器供 preview 音效音量跟随视频 (与 render 混音同一数据)
        if let Some(video_rel) = cfg["source"]["video"].as_str() {
            let video_path = project_path.join(video_rel).to_string_lossy().to_string();
            let curve = crate::render::measure_loudness_curve(&video_path);
            if !curve.is_empty() {
                let points: Vec<serde_json::Value> = curve.iter()
                    .map(|(t, db)| serde_json::json!({"t": t, "db": db}))
                    .collect();
                cfg["loudness_curve"] = serde_json::json!(points);
            }
        }
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

    /// 启动服务；`build` 在**每次请求**时按磁盘上的配置重新生成 `index.html`，
    /// 因此复用同一个监听不会端出过期内容。
    pub async fn start_with<F>(&mut self, build: F) -> anyhow::Result<u16>
    where
        F: Fn(&Path) -> String + Send + Sync + Clone + 'static,
    {
        let project_path = self.project_path.clone();
        let project_path_sd = project_path.clone();
        let editor_path = project_path.clone();
        let app = Router::new()
            .route("/", get(move || {
                let html = build(&project_path);
                async move {
                    Response::builder()
                        .header("Content-Type", "text/html;charset=utf-8")
                        .body(Body::from(html))
                        .unwrap()
                }
            }))
            .route("/api/config", post(move |body| {
                save_config_handler(editor_path.clone(), body)
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

    pub async fn start(&mut self) -> anyhow::Result<u16> {
        self.start_with(Self::build_html).await
    }

    /// 封面预览服务（HTML 同样每次请求重建，避免复用端口后端出旧封面）。
    pub async fn start_cover(&mut self) -> anyhow::Result<u16> {
        self.start_with(Self::build_cover_html).await
    }

    /// 按当前 polish.toml 生成封面预览 HTML；加载失败时给一个可读错误页。
    pub fn build_cover_html(project_path: &Path) -> String {
        match crate::project::load_project(project_path) {
            Ok(config) => {
                let resolved = crate::render::resolve_paths(project_path, &config);
                crate::cover::build_cover_html(project_path, &config, &resolved)
            }
            Err(e) => format!(
                "<!doctype html><html><body style=\"font-family:sans-serif;padding:24px\">\
                 <h3>无法加载 polish.toml</h3><pre>{e}</pre></body></html>"
            ),
        }
    }
}
