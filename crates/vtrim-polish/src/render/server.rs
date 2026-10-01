//! polish::render::server：预览/出片用的本地 HTTP server（`ServeDir` 根 = 项目目录）。
//!
//! 入口：`RenderServer::{new, build_html, start_with, start, start_cover}`、`start_file_server`、`build_sticker_map`、`font_face_css(_in)`、`font_family_with_stack`、`SYSTEM_FONT_STACK`。
//! 边界：`/vt-fonts/:name` 两条建 router 的路都要挂（漏一条 = 预览有、成片没有）；`INDEX_HTML` 走 `include_str!`，改 `templates/index.html` 必须重建二进制 —— 见 `crates/vtrim-polish/src/render/AGENTS.md`

use axum::{
    Router,
    routing::{get, post},
    response::{IntoResponse, Response},
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

// ===== 字体库（<程序目录>/resources/fonts/）=====
//
// 出片页面是 headless Chrome 加载本服务器上的 HTML 的，而**浏览器只认系统已装字体** ——
// 用户丢进字体库的那个 ttf 它根本不知道。所以凡是选了库内字体，就要在页面里补一条
// `@font-face`，把 src 指到下面这条路由（同源，不涉及 CORS）。
//
// ⚠️ 只在这里挂路由还不够：**两条建 router 的路都得挂** ——
// `start_file_server`（chunk 抓帧 / 封面截图）与 `start_with`（预览 / CLI `polish render`）。

/// 字体库路由：`/vt-fonts/<文件名>`（只认库目录下的单段文件名，别的都 404）。
fn font_routes(app: Router) -> Router {
    app.route("/vt-fonts/:name", get(serve_library_font))
}

async fn serve_library_font(axum::extract::Path(name): axum::extract::Path<String>) -> Response {
    // 只认"纯文件名 + 字体扩展名"：`../x.ttf`、`a/b.ttf`、`..` 都在这里被挡掉
    let bare = Path::new(&name).file_name().and_then(|s| s.to_str()) == Some(name.as_str());
    if !bare || !vtrim_core::fonts::is_font_file_name(&name) {
        return StatusCode::NOT_FOUND.into_response();
    }
    let path = vtrim_core::path::font_dir().join(&name);
    // 字体动辄几十 MB，读盘别占着 async worker（这条服务器也要抓帧/截图）
    let bytes = match tokio::task::spawn_blocking(move || std::fs::read(path)).await {
        Ok(Ok(b)) => b,
        _ => return StatusCode::NOT_FOUND.into_response(),
    };
    Response::builder()
        .header("Content-Type", font_content_type(&name))
        .body(Body::from(bytes))
        .unwrap()
}

fn font_content_type(file: &str) -> &'static str {
    match Path::new(file)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .as_deref()
    {
        Some("otf") => "font/otf",
        Some("ttc") => "font/collection",
        _ => "font/ttf",
    }
}

/// 为配置里选中的字体生成 `@font-face` 片段（没选 / 不在库里 → 空串）。
///
/// **声明的字族名 = 配置里存的那个 id**，因为页面里读的就是它：
/// `index.html` 的 `.cap{font-family:"<style.subtitle.font>"…}`、`cover.html` 的
/// `__FONT_FAMILY__` 都直接把这一串当 CSS 字族名用。名字一样，那条规则一行都不用改。
pub fn font_face_css(font_ids: &[&str]) -> String {
    font_face_css_in(&vtrim_core::path::font_dir(), font_ids)
}

/// [`font_face_css`] 的目录版（测试用临时目录，免得往程序目录写字）。
pub fn font_face_css_in(root: &Path, font_ids: &[&str]) -> String {
    let mut css = String::new();
    for id in font_ids {
        let id = id.trim();
        if id.is_empty() || css.contains(&format!("font-family:{}", quote_css(id))) {
            continue;
        }
        let Some(path) = vtrim_core::fonts::resolve_file_at(root, id) else {
            // 解析不到就**什么都不注入**：老工程的 `font = "LXGW WenKai"` 走到这里，
            // 行为与从前一致（当系统字体族名用）。
            continue;
        };
        let Some(file) = path.file_name().and_then(|s| s.to_str()) else {
            continue;
        };
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
        let format = match ext.as_str() {
            "otf" => " format(\"opentype\")",
            "ttc" => "",
            _ => " format(\"truetype\")",
        };
        css.push_str(&format!(
            "@font-face{{font-family:{};src:url(\"/vt-fonts/{}\"){};font-display:block}}\n",
            quote_css(id),
            urlencoding::encode(file),
            format
        ));
    }
    if css.is_empty() {
        css
    } else {
        format!("<style>\n{css}</style>")
    }
}

/// 注进页面的配置 JSON 里的字幕字体（`[style.subtitle] font`）。
///
/// 单独拎出来是因为**键路径写错不会报错**：取不到就 `""`、不注入 @font-face，
/// 表现是"选了字体、出片没换"（预览那边另有 @font-face，看起来还是对的）——
/// 单测钉的就是这条路径（`subtitle_font_read_from_config_json`）。
fn subtitle_font_of(cfg: &serde_json::Value) -> &str {
    cfg["style"]["subtitle"]["font"].as_str().unwrap_or("")
}

/// CSS 字符串字面量：包双引号并转义 `\` 与 `"`（字体 id 来自文件名，可能有空格/括号/汉字）。
fn quote_css(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        if c == '"' || c == '\\' {
            out.push('\\');
        }
        out.push(c);
    }
    out.push('"');
    out
}

/// 没选库内字体时的字体栈（index.html / cover.html / webui 的 `--font` 同一套）。
///
/// ⚠️ **顺序有意义，且必须与 `webui/src/components/PolishPreview.vue` 的 `.cap` 栈同序**
/// （那里是 `"<项目字体>","Microsoft YaHei","Noto Sans SC",sans-serif`）：项目选的那支没装时，
/// **谁排前面谁就是用户在预览里看到的字体**。这里曾经把 `Noto Sans SC` 排在 `Microsoft YaHei`
/// 前、而预览是雅黑在前 —— 同一个工程：预览是雅黑、成片是 Noto（用户报的"字体不对"）。
/// 底座那条腿（`render/dhampir.rs::find_font`）直接读这个常量，所以改这一处三处一起变。
pub const SYSTEM_FONT_STACK: &str = "'LXGW WenKai','Microsoft YaHei','Noto Sans SC',sans-serif";

/// 页面用的 CSS `font-family` 串：选中的字体排在最前，后面跟系统栈兜底。
pub fn font_family_with_stack(id: &str) -> String {
    format!("{},{}", quote_css(id.trim()), SYSTEM_FONT_STACK)
}

/// 启动一个只提供静态文件的 HTTP server，绑定到随机端口
/// 返回 port。用于每个 chunk 独享的静态文件服务。
pub async fn start_file_server(project_path: PathBuf, injected_html: Option<String>) -> anyhow::Result<u16> {
    let path = project_path.clone();
    let sd_path = project_path.clone();
    let app = font_routes(Router::new()
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
        .fallback_service(ServeDir::new(&sd_path)));
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
        let raw_cfg = std::fs::read_to_string(&config_path).ok();
        let mut cfg: serde_json::Value = raw_cfg.as_deref()
            .and_then(|raw| toml::from_str(raw).ok())
            .unwrap_or(serde_json::Value::Object(Default::default()));
        cfg["_assets"] = serde_json::json!({
            "sticker_map": build_sticker_map(project_path),
        });
        // 画布契约：尺寸（横/竖画布）+ 画面构成（位置/背景）—— **唯一真相源**，
        // 由 `ProjectFile::canvas_contract` 推导。`api/polish.rs` 的预览注入读的是同一份，
        // 所以「出片」与「WebUI 预览」不可能算出不同的画布。
        // ⚠️ 只留这一份实现：这里曾经还有第二份「竖屏检测」，改画布方向时两边会不同步。
        // `vertical` 只表示「源视频是竖的」（特效用它做门控），摆放由 `_canvas.stage` 决定。
        let project: Option<crate::project::ProjectFile> =
            raw_cfg.as_deref().and_then(|raw| toml::from_str(raw).ok());
        if let Some(project) = project {
            let src_size = cfg["source"]["video"].as_str().and_then(|rel| {
                crate::render::get_video_size(&project_path.join(rel).to_string_lossy())
            });
            let contract = project.canvas_contract(src_size);
            let vertical = src_size.map(|(w, h)| h > w).unwrap_or(false);
            cfg["vertical"] = serde_json::json!(vertical);
            // 兼容：模板里贴纸的老分支仍按 `vertical_position` 取位置（P2 由 `[sticker] manual` 接管）
            cfg["vertical_position"] = contract["stage"]["position"].clone();
            if vertical || project.canvas.is_portrait() {
                tracing::info!(
                    "  画布 {}x{} · 源 {:?} → axis={} position={} bg={} (vertical={})",
                    contract["width"], contract["height"], src_size,
                    contract["stage"]["axis"], contract["stage"]["position"],
                    contract["stage"]["bg"], vertical
                );
            }
            cfg["_canvas"] = contract;
        }
        // 响度曲线: 注入浏览器供 preview 音效音量跟随视频 (与 render 混音同一数据)
        // 走带缓存的那份：这个 HTML 是**每个请求**重建的，不缓存就等于每刷一次页面烧一遍 ebur128
        if let Some(video_rel) = cfg["source"]["video"].as_str() {
            let video_path = project_path.join(video_rel);
            let curve = crate::render::measure_loudness_curve_cached(project_path, &video_path);
            if !curve.is_empty() {
                let points: Vec<serde_json::Value> = curve.iter()
                    .map(|(t, db)| serde_json::json!({"t": t, "db": db}))
                    .collect();
                cfg["loudness_curve"] = serde_json::json!(points);
            }
        }
        // 把 data/*.json 直接注入 HTML：出片时页面不再自己 fetch。
        // 网络盘（RaiDrive/WebDAV）上这几个文件可能很慢，甚至 fetch 悬住 ——
        // 旧实现会让页面在 60s 内永远等不到 init()，报一个与根因无关的“GSAP/CDN”超时。
        let data_dir = project_path.join("data");
        let read_data = |name: &str| -> serde_json::Value {
            let path = data_dir.join(name);
            match std::fs::read_to_string(&path) {
                Ok(raw) => match serde_json::from_str(&raw) {
                    Ok(v) => v,
                    Err(e) => {
                        tracing::warn!("读取 {} 失败（JSON 解析错误）: {e}", path.display());
                        serde_json::Value::Null
                    }
                },
                Err(e) => {
                    tracing::warn!("读取 {} 失败: {e}", path.display());
                    serde_json::Value::Null
                }
            }
        };
        let subtitles = read_data("subtitles.json");
        let danmaku = read_data("danmaku.json");
        let clip_meta = read_data("clip-meta.json");
        cfg["_data"] = serde_json::json!({
            "subtitles": if subtitles.is_null() { serde_json::json!([]) } else { subtitles },
            "danmaku": if danmaku.is_null() { serde_json::json!([]) } else { danmaku },
            "meta": if clip_meta.is_null() { serde_json::json!({}) } else { clip_meta },
        });

        let html = Self::inject_config_tag(&cfg, INDEX_HTML);
        // 字幕字体：选了字体库里的字体就补 @font-face（`.cap` 的 family 读的就是这一串）
        let font = subtitle_font_of(&cfg).to_string();
        html.replace("<!-- __FONT_FACE__ -->", &font_face_css(&[font.as_str()]))
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
        let app = font_routes(Router::new()
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
            .fallback_service(ServeDir::new(&project_path_sd)));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let port = listener.local_addr()?.port();
        self.port = port;
        tokio::spawn(async move {
            // ⚠️ detached task：`serve` 出错时 `unwrap()` 会直接 panic。
            // 出片/预览本身由调用方的超时与卡死守卫兜住，这里记一行日志就够 ——
            // 别让"本地静态服务退出"变成整个任务（历史上是整个进程）的崩溃。
            if let Err(e) = axum::serve(listener, app).await {
                tracing::error!("本地渲染服务退出: {e}");
            }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("vtrim-fontface-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// 库里的字体会被注入 @font-face，且**声明的名字就是配置里那串 id**
    /// （页面里 `.cap{font-family:"<id>"}` 全靠它才对得上）。
    #[test]
    fn injects_font_face_for_library_font() {
        let root = temp_root("inject");
        std::fs::write(root.join("My Font.ttf"), b"x").unwrap();
        let css = font_face_css_in(&root, &["My Font"]);
        assert!(css.contains("@font-face"), "{css}");
        assert!(css.contains("font-family:\"My Font\""), "{css}");
        assert!(css.contains("url(\"/vt-fonts/My%20Font.ttf\")"), "空格要转义: {css}");
        assert!(css.contains("format(\"truetype\")"), "{css}");
        // 同名字体在多处出现（字幕 + 封面）只注入一条
        let once = font_face_css_in(&root, &["My Font", "My Font"]);
        assert_eq!(once.matches("@font-face").count(), 1, "{once}");
        std::fs::remove_dir_all(&root).ok();
    }

    /// 解析不到的字体名（老工程的 `LXGW WenKai` 这类系统族名）**不注入**任何东西 ——
    /// 注入了就会变成一条 404 的 @font-face，白让浏览器去取。
    #[test]
    fn library_miss_injects_nothing() {
        let root = temp_root("miss");
        assert_eq!(font_face_css_in(&root, &["LXGW WenKai"]), "");
        assert_eq!(font_face_css_in(&root, &[""]), "");
        assert_eq!(font_face_css_in(&root, &["../etc/passwd"]), "");
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn css_quotes_are_escaped() {
        // 输入 `a"b\c` → 双引号包起来，并把里面的 `"` 与 `\` 各加一个反斜杠
        let expected: String = ['"', 'a', '\\', '"', 'b', '\\', '\\', 'c', '"'].iter().collect();
        assert_eq!(quote_css("a\"b\\c"), expected);
        assert_eq!(
            font_family_with_stack("LXGW WenKai"),
            // ⚠️ 顺序**不是随手写的**：它必须等于 `webui/src/components/PolishPreview.vue` 的 `.cap` 栈
            // （雅黑在 Noto 前）—— 项目选的那支没装时，这里的第一支就是用户在预览里看到的字体。
            // 改这条断言之前先确认另一边也跟着改了（`dhampir.rs::font_search_order` 有对应钉子）。
            "\"LXGW WenKai\",'LXGW WenKai','Microsoft YaHei','Noto Sans SC',sans-serif"
        );
    }

    /// 两个模板都得留着字体占位符 —— 少了它注入是静默失败的（页面照出，字体不生效）。
    #[test]
    fn templates_keep_font_placeholders() {
        assert!(INDEX_HTML.contains("<!-- __FONT_FACE__ -->"));
        assert!(crate::cover::COVER_HTML.contains("<!-- __FONT_FACE__ -->"));
        assert!(crate::cover::COVER_HTML.contains("font-family:__FONT_FAMILY__"));
    }

    /// 字幕字体的键路径：`polish.toml` 的 `[style.subtitle] font` 在注入的 JSON 里就是这一层。
    /// 取不到时是空串（不注入），**不报错** —— 所以它需要一条钉子。
    #[test]
    fn subtitle_font_read_from_config_json() {
        let v: serde_json::Value = toml::from_str("[style.subtitle]\nfont = \"simhei\"\n").unwrap();
        assert_eq!(subtitle_font_of(&v), "simhei");
        let empty: serde_json::Value = toml::from_str("").unwrap();
        assert_eq!(subtitle_font_of(&empty), "");
        let other: serde_json::Value = toml::from_str("[style]\nfoo = 1\n").unwrap();
        assert_eq!(subtitle_font_of(&other), "");
    }
}
