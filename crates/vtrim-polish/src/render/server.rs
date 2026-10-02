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

/// 字体库路由：`/vt-fonts/<id>`（**按库 id 解析**，只认单段名字，别的都 404）。
///
/// ⚠️ 以前这里认的是**文件名**（`<库根>/<name>` 直接 join）—— 那只在老平铺布局下成立：
/// v2 目录布局（`<slug>/font.ttf`）下注入的 URL 会 **404**（文件在子目录里），
/// 而且库里每支字体都叫 `font.ttf` 之后，文件名**根本不足以定位一支字体**。
/// 所以改成按 id 走 `fonts::resolve_all`：两种布局 + “id 住在 FONT.toml 里”那条新形状都认。
fn font_routes(app: Router) -> Router {
    app.route("/vt-fonts/:name", get(serve_library_font))
}

async fn serve_library_font(axum::extract::Path(name): axum::extract::Path<String>) -> Response {
    // 只认单段名字（`../x`、`a/b` 在这里被挡掉），再让 `is_safe_id` 挡一次穿越/控制字符
    let bare = Path::new(&name).file_name().and_then(|s| s.to_str()) == Some(name.as_str());
    if !bare || !vtrim_core::fonts::is_safe_id(&name) {
        return StatusCode::NOT_FOUND.into_response();
    }
    // 字体动辄几十 MB，读盘别占着 async worker（这条服务器也要抓帧/截图）
    let loaded = tokio::task::spawn_blocking(move || {
        let root = vtrim_core::path::font_dir();
        let path = vtrim_core::fonts::resolve_all(&root, &name)?;
        let file = path.file_name()?.to_str()?.to_string();
        std::fs::read(&path).ok().map(|b| (b, file))
    })
    .await;
    let Ok(Some((bytes, file))) = loaded else {
        return StatusCode::NOT_FOUND.into_response();
    };
    Response::builder()
        .header("Content-Type", font_content_type(&file))
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
        // ⚠️ **必须走 `resolve_all`**（两种布局都认：v2 目录 `<slug>/font.ttf` + 老平铺文件）。
        // 以前这里是 `resolve_file_at`（**只认老平铺**）⇒ v2 目录布局下**什么都不注入**，
        // 于是"选了库里的字体、出片没换"（而预览那边另有 @font-face，看着还是对的）。
        let Some(path) = vtrim_core::fonts::resolve_all(root, id) else {
            // 解析不到就**什么都不注入**：老工程的 `font = "LXGW WenKai"` 走到这里，
            // 行为与从前一致（当系统字体族名用）。
            continue;
        };
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
        let format = match ext.as_str() {
            "otf" => " format(\"opentype\")",
            "ttc" => "",
            _ => " format(\"truetype\")",
        };
        // ⚠️ **声明的族名 = 配置里那一串 id**（与页面的 `font-family`、预览的 `FontFace` 同源）。
        // 别改成"内部族名"：模板 `.cap{font-family:"<config[id]>"}` 与
        // `webui/src/polish/fonts.js` 的 `new FontFace(id, …)` 都按 **id** 请求，
        // 声明成别的名字就会"注入了 @font-face 但页面请求的是另一个名字"——浏览器静默回退系统字体。
        // Takumi 那条腿走**别名注册**（`render/takumi.rs`）来对齐，而不是靠改这里。
        // ⚠️ URL 里放的是 **id**（不是文件名）：路由按 id 解析，而且库里每支的字体文件
        // 都叫 `font.ttf` —— 用文件名既 404（v2 在子目录里）又互相撞。
        css.push_str(&format!(
            "@font-face{{font-family:{};src:url(\"/vt-fonts/{}\"){};font-display:block}}\n",
            quote_css(id),
            urlencoding::encode(id),
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

/// 没选库内字体时的字体栈（**空库回落态**）：`'Noto Sans SC','Microsoft YaHei','Noto Sans SC',sans-serif`。
///
/// ⚠️ **首项与第三项字面量相同，这是有意的、不是笔误**：首项是「库里那支」（可能被用户换成别的，
/// 见 [`preferred_fallback_family`]），第三项是「系统里装的 Noto Sans SC」—— 两者可能不是同一支。
/// ⚠️ **`'LXGW WenKai'` 已从链里去掉**（2026-10-02 自带中文换成 Noto Sans SC）：
/// 我们不再自带它，继续留一个「刚决定不用」的字体在栈里只会让结果不确定 ——
/// 用户系统恰好装了霞鹜文楷时，落到的观感与库里那支不一致，而没人预期到这一点。
///
/// ⚠️ **这不再是"用哪支"的权威** —— 权威是 [`fallback_stack()`]（首项随库里默认那支走）。
/// 本常量现在只扮演两个角色：① `fallback_stack()` 在**库里没有可用中文**时的回落态
/// （`stack_head_tracks_installed_library_font` 单测钉着"回落时逐字相同"，存量工程的像素不变）；
/// ② 文档锚点：尾部顺序必须与 `webui/src/components/PolishPreview.vue` 的 `.cap` 栈同序
/// （那里是 `"<项目字体>",<fallback_stack>`）。
pub const SYSTEM_FONT_STACK: &str = "'Noto Sans SC','Microsoft YaHei','Noto Sans SC',sans-serif";

/// 字体栈的**首项**：装没装自带中文字体，这里不一样。
///
/// ⚠️ **为什么不能只写死 `'LXGW WenKai'`**（P1-f 实测）：自带那支字体的**内部族名是
/// `霞鹜文楷 轻便版`**（中文），而栈里写的是英文名 `LXGW WenKai` —— 两者**永远匹配不上**，
/// 于是"装了自带字体"这件事在页面上**完全没有效果**（静默回退雅黑），
/// 而用户明明在字体库里看到了它。
///
/// 所以首项按"库里默认那支 / 内置那支"算：解析得到就返回它的**内部族名**，
/// 否则回落到原来的字面量（系统里真装了 `LXGW WenKai` 的人照旧命中）。
/// ⚠️ 顺序不能变（仍然排在 `Microsoft YaHei` 前），否则就是上面那条老 bug 的翻版。
///
/// ⚠️ **emoji 字体进不了这里**（2026-10-02 职责分离）：两处拦截都在 `vtrim-core::fonts` ——
/// `scan_dirs` 对 `emoji==true` 强制 `is_default=false`（所以上面的 `.find(|e| e.is_default)` 选不到它），
/// `builtin_fonts` 那条也显式滤掉了 id 含 emoji 的。**别再在这里补判据**（那就又是"各处各写一份"）。
pub fn preferred_fallback_family() -> String {
    let root = vtrim_core::fonts::dir();
    // ① 库里显式设过默认的那支（用户意图最强）
    let preferred = vtrim_core::fonts::scan_all(&root)
        .into_iter()
        .filter(|e| e.installed)
        .find(|e| e.is_default)
        .map(|e| e.family_internal.trim().to_string())
        // ② 否则内置那支中文（自带字体里只有它不是 emoji）
        .or_else(|| {
            vtrim_core::fonts::builtin_fonts()
                .iter()
                .find(|b| {
                    !b.id.to_ascii_lowercase().contains("emoji")
                        && vtrim_core::fonts::builtin_installed(&root, b)
                })
                .map(|b| vtrim_core::fonts::family_for_at(&root, b.id))
        })
        .unwrap_or_default();
    if preferred.trim().is_empty() {
        // 库里没有可用中文 ⇒ 回落到**系统里那支 `Noto Sans SC`**（不再是 `LXGW WenKai`：
        // 我们已不自带它，见 `SYSTEM_FONT_STACK` 上的说明）
        quote_css("Noto Sans SC")
    } else {
        quote_css(preferred.trim())
    }
}

/// **兜底字体栈（动态版）**：`preferred_fallback_family()` + [`TAIL_STACK`]。
///
/// 这是"没选库内字体时用哪支"的**唯一权威**：库里设了默认 ⇒ 首项就是它的内部族名；
/// 库里没有可用中文 ⇒ 与 [`SYSTEM_FONT_STACK`] 常量逐字相同（单测钉着，存量零回归）。
/// 消费者：① 底座那条腿（`render/dhampir.rs::font_search_order`，成片按它找字体文件）；
/// ② 封面没选字体时的 `font-family`（`cover.rs::cover_font_css`）；③ `/api/polish/fonts`
/// 回给前端的 `fallback_stack`（预览 `.cap` 栈据此与成片同源）。
pub fn fallback_stack() -> String {
    format!("{},{}", preferred_fallback_family(), authority_tail())
}

/// 页面用的 CSS `font-family` 串：选中的字体排在最前，后面跟系统栈兜底（首项是
/// [`preferred_fallback_family`]，其余与 [`SYSTEM_FONT_STACK`] 同序）。
pub fn font_family_with_stack(id: &str) -> String {
    // ⚠️ 与 `font_face_css` 声明的名字**必须同一个**（两处都是**配置里那串 id**）：
    // 一处按 id、一处按内部族名，就会出现"注入了 @font-face 但页面请求的是另一个名字"
    // —— 浏览器静默回退系统字体，只有像素能看出（P1-e 收敛的就是这条）。
    format!("{},{},{}", quote_css(id.trim()), preferred_fallback_family(), TAIL_STACK)
}

/// [`SYSTEM_FONT_STACK`] 里首项**之后**的部分（首项由 [`preferred_fallback_family`] 动态给）。
///
/// ⚠️ **数据源是 `vtrim_core::fonts::fallback_chain()`（唯一权威）**，本常量只是它的字面量投影 ——
/// 两者必须逐字一致（有单测钉着）。改兜底链**只改 core 那一处**，这里跟着走。
///
/// ⚠️ 为什么仍然保留这个 `const` 而不是每次调用 `fallback_chain()`：它是**编译期字面量**，
/// 让下面几条单测能直接对字符串做断言（`concat!` 需要编译期值），也当文档锚点用。
/// 运行时的拼接仍走 `fallback_chain()`（见 `fallback_stack`），所以不存在"两边各跑一套"。
const TAIL_STACK: &str = "'Microsoft YaHei','Noto Sans SC',sans-serif";

/// 权威兜底链的**首项之后的字面量投影**（给单测做"两份定义不许漂"的判据）。
///
/// ⚠️ 为什么要有这个函数：`TAIL_STACK` 是编译期常量、`vtrim_core::fonts::fallback_chain()`
/// 是运行时的唯一权威 —— 两者**必须有机器判据钉住**。只靠注释提醒的话，改了一边忘另一边
/// **不会报错**，症状是"库里设了默认，成片和预览差一段"（正是 P1-f 收敛过的那类）。
fn authority_tail() -> String {
    let chain = vtrim_core::fonts::fallback_chain();
    // 权威链 = [首项, 尾巴...]；这里取**首项之后**的部分，拼成与 TAIL_STACK 同形状的串。
    //
    // ⚠️ **通用族（`sans-serif` / `serif` / `monospace`）不能加引号**：
    // `'sans-serif'` 在 CSS 里被解释成"一个叫 sans-serif 的**字体族名**"而不是通用族
    // —— 浏览器不认识这个名字就丢掉这一项，兜底链**从最后一个保险变成没有保险**。
    // （这条是单测 `tail_stack_matches_core_authority` 抓出来的：第一版统一走 `quote_css`，
    // 拼出来是 `'sans-serif'`，与 TAIL_STACK 对不上。）
    let is_generic = |s: &str| matches!(s.to_ascii_lowercase().as_str(), "sans-serif" | "serif" | "monospace");
    chain
        .iter()
        .skip(1)
        .map(|f| if is_generic(f) { f.clone() } else { quote_css(f) })
        .collect::<Vec<_>>()
        .join(",")
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
        // ⚠️ URL 里是 **id**（不是文件名）：路由按 id 解析，文件名在 v2 布局里既 404 又会撞
        assert!(css.contains("url(\"/vt-fonts/My%20Font\")"), "空格要转义: {css}");
        assert!(css.contains("format(\"truetype\")"), "{css}");
        // 同名字体在多处出现（字幕 + 封面）只注入一条
        let once = font_face_css_in(&root, &["My Font", "My Font"]);
        assert_eq!(once.matches("@font-face").count(), 1, "{once}");
        std::fs::remove_dir_all(&root).ok();
    }

    /// ⚠️ **v2 目录布局也必须注入 @font-face**（P1-e 发现的老 bug）。
    ///
    /// 以前这里走 `resolve_file_at`（**只认老平铺文件**），而 v2 布局是
    /// `<slug>/font.ttf` ⇒ **一条都不注入**。症状："选了库里的字体、出片没换"，
    /// 而预览那边另有 `FontFace`（它也解析得到）⇒ 看起来还是对的，只有成片不对。
    #[test]
    fn injects_font_face_for_dir_layout_too() {
        let root = temp_root("inject-dir");
        let slug = root.join("my-font");
        std::fs::create_dir_all(&slug).unwrap();
        std::fs::write(slug.join("font.ttf"), b"x").unwrap();
        let css = font_face_css_in(&root, &["my-font"]);
        assert!(css.contains("@font-face"), "v2 目录布局没注入：{css}");
        // 声明的族名 = 配置里那串 id（与模板 / 预览同源）
        assert!(css.contains("font-family:\"my-font\""), "{css}");
        assert!(css.contains("url(\"/vt-fonts/my-font\")"), "{css}");
        std::fs::remove_dir_all(&root).ok();
    }

    /// ⚠️ **id 住在 `FONT.toml` 里、目录名是 ASCII slug 的形状也要注入**（2026-10-02）。
    ///
    /// 落盘：`font-<hash>/font.ttf` + `FONT.toml{id="乐米波波体"}`；配置里存的是那串汉字。
    /// 解析与 URL 都必须按 **id** 走 —— 拿目录名或文件名都会得到一条 404 的 @font-face。
    #[test]
    fn injects_font_face_for_id_indexed_slug_dir() {
        let root = temp_root("inject-slug");
        let slug = root.join("font-9c1d2e3f4a5b");
        std::fs::create_dir_all(&slug).unwrap();
        std::fs::write(slug.join("font.ttf"), b"x").unwrap();
        std::fs::write(
            slug.join("FONT.toml"),
            "id = \"乐米波波体\"\ndisplay_name = \"乐米波波体\"\n",
        )
        .unwrap();
        let css = font_face_css_in(&root, &["乐米波波体"]);
        assert!(css.contains("@font-face"), "{css}");
        assert!(css.contains("font-family:\"乐米波波体\""), "{css}");
        assert!(
            css.contains("url(\"/vt-fonts/%E4%B9%90%E7%B1%B3%E6%B3%A2%E6%B3%A2%E4%BD%93\")"),
            "URL 里必须是 id（百分号编码）：{css}"
        );
        // 目录名（内部 slug）不该出现在 URL 里 —— 它是落盘形状，不是配置值
        assert!(!css.contains("9c1d2e3f4a5b"), "{css}");
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
        // ⚠️ 断言里**不能写死首项**：首项由 `preferred_fallback_family()` 按"库里装了什么"算
        // （装了自带中文就是 `霞鹜文楷 轻便版`，没装才是 `LXGW WenKai`）—— 写死就会在
        // "开发机装了字体、CI 没装"时随机红。这里钉的是**结构与尾部顺序**。
        let stack = font_family_with_stack("LXGW WenKai");
        assert!(stack.starts_with("\"LXGW WenKai\","), "{stack}");
        // ⚠️ 尾部顺序**不是随手写的**：它必须等于 `webui/src/components/PolishPreview.vue` 的 `.cap` 栈
        // （雅黑在 Noto 前）—— 项目选的那支没装时，栈里第一支就是用户在预览里看到的字体。
        // 改这条断言之前先确认另一边也跟着改了（`dhampir.rs::font_search_order` 有对应钉子）。
        assert!(stack.ends_with(",'Microsoft YaHei','Noto Sans SC',sans-serif"), "{stack}");
        assert_eq!(stack.matches(',').count(), 4, "四段：项目字体 + 首项 + 雅黑 + Noto: {stack}");
    }

    /// ⚠️ **P1-f 的钉子**：字体栈首项必须跟着"库里装了什么"走。
    ///
    /// 自带那支的内部族名是 **`霞鹜文楷 轻便版`**（中文），而常量里写的是英文 `LXGW WenKai` ——
    /// 两者**永远匹配不上**，于是"装了自带字体"在页面上**完全没有效果**（静默回退雅黑），
    /// 而用户明明在字体库里看得到它。这条测试把"首项会不会跟着变"钉住。
    #[test]
    fn stack_head_tracks_installed_library_font() {
        // 这台机器上装没装自带中文，决定首项 —— 两种情况都必须自洽
        let head = preferred_fallback_family();
        assert!(!head.trim().is_empty(), "首项不能是空的（整串会多出个空段）");
        let stack = font_family_with_stack("x");
        assert!(stack.contains(&head), "栈里必须含首项本身: {head} vs {stack}");
        // 回落态（库里没有可用中文）必须与老常量**同一套族名与顺序**，否则存量工程的像素会变。
        // ⚠️ 比的是**族名与顺序**，不是引号风格：`TAIL_STACK` 保留单引号（与真模板一致）、
        // 首项走 `quote_css` 的双引号 —— CSS 里 '阿里' 与 "阿里" 完全等价，逐字比必红
        // （这条断言第一版就是这么写错的，两次）。
        if head == quote_css("LXGW WenKai") {
            let norm = |s: &str| s.replace('\'', "\"").replace(" ", "");
            assert_eq!(
                norm(&format!("{},{}", head, TAIL_STACK)),
                norm(SYSTEM_FONT_STACK),
                "回落时必须与老常量同族名同顺序（不然存量工程跟着变）"
            );
        }
    }

    /// ⚠️ **两份兜底链定义不许漂**（2026-10-02 输入层收敛的钉子）。
    ///
    /// `TAIL_STACK`（本文件的编译期常量）与 `vtrim_core::fonts::fallback_chain()`（唯一权威）
    /// 必须逐字一致。这条单测就是"改了一边忘另一边"的机器判据 —— 没有它，症状是
    /// "库里设了默认，成片和预览差一段"，而且**不报错**。
    #[test]
    fn tail_stack_matches_core_authority() {
        let norm = |s: &str| s.replace('"', "'").replace(' ', "");
        assert_eq!(
            norm(&authority_tail()),
            norm(TAIL_STACK),
            "TAIL_STACK 与 vtrim_core::fonts::fallback_chain() 漂了 —— 改兜底链只改 core 那一处"
        );
    }

    /// ⚠️ **emoji 不许进兜底链**（职责分离的钉子）。
    ///
    /// 一是语义（emoji 是独立槽位，不是兜底的一段），二是**它撑破下面那条"四段"断言**，
    /// 三是它对底座（Dhampir）那条腿**零效果**（它没有逐字形回退，见 dhampir.rs 的说明）。
    #[test]
    fn fallback_stack_never_carries_emoji() {
        let stack = fallback_stack();
        assert!(
            !stack.to_lowercase().contains("emoji"),
            "兜底链里出现了 emoji 族名：{stack}"
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
