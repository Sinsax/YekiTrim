//! polish::cover：封面生成（三层结构 文字/关键帧/背景 → HTML 构建 + headless Chrome 截图）。
//!
//! 入口：`cmd_cover`、`cmd_cover_template`、`cmd_preview_cover`、`build_cover_html`、`cover_bg_vars`、`cover_frame_vars`、`cover_blur_css`、`sanitize_hex_color`、`cover_bg_mode`。
//! 边界：`cover.image` 在消费侧要再做一次 `tenant::ensure_within`（纵深防御第③道，别删）；颜色进 HTML/CSS 前必须 `sanitize_hex_color`，公式只此一份（前端是镜像） —— 见 `crates/vtrim-polish/AGENTS.md`

use std::path::Path;

use crate::project;
use crate::render::{chrome::ChromeRenderer, server};
use crate::render::{ResolvedPaths, resolve_paths};

// `pub(crate)`：render/server.rs 的测试要断言模板里还留着字体占位符
pub(crate) const COVER_HTML: &str = include_str!("../templates/cover.html");

fn cover_text_align(cc: &crate::project::CoverConfig) -> (&'static str, &'static str) {
    match cc.text_align.as_str() {
        "left" => ("left", "flex-start"),
        "right" => ("right", "flex-end"),
        _ => ("center", "center"),
    }
}

fn cover_shadow_css(cc: &crate::project::CoverConfig) -> String {
    // 新语义：文字阴影只控制开关；方向感由背景渐变滤镜（gradient_dir）承担。
    // shadow_dir / shadow_intensity 仅用于兼容旧配置：旧配置关闭阴影时也能正确关闭。
    let legacy_off = cc.shadow_dir == "none" || cc.shadow_intensity <= 0.0;
    if cc.text_shadow && !legacy_off {
        "0 5px 22px rgba(0,0,0,0.78)".to_string()
    } else {
        "none".to_string()
    }
}

/// 封面字体：`[cover] font` 选中的库内字体。
///
/// - `family` 直接进 CSS `__FONT_FAMILY__`（选中字体 + 系统栈兜底）；
/// - `face` 是给页面 `<head>` 的 `@font-face` 片段（浏览器只认系统已装字体，
///   不注入这一条，库里的字体在出片时根本不会生效）。
struct CoverFont {
    family: String,
    face: String,
}

fn cover_font_css(cc: &crate::project::CoverConfig) -> CoverFont {
    let id = cc.font.trim();
    if id.is_empty() {
        // 老工程 / 没选：与从前完全一致，只是那行 font-family 由 Rust 侧填（模板里是占位符）
        return CoverFont { family: server::SYSTEM_FONT_STACK.to_string(), face: String::new() };
    }
    // 值不在字体库里时也照写（当系统字体族名用），与字幕字体同一条规矩
    CoverFont { family: server::font_family_with_stack(id), face: server::font_face_css(&[id]) }
}

fn cover_highlight_words(config: &crate::project::ProjectFile) -> Vec<String> {
    // 优先使用封面自己的高亮关键词；未设置时兼容旧的 filter.highlight。
    let mut out: Vec<String> = config
        .cover
        .highlight_keywords
        .iter()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    if out.is_empty() {
        out = config
            .filter
            .highlight
            .iter()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
    }
    out
}

/// `#rrggbb` / `#rgb` → RGB。非法值一律回 (0,0,0)。
///
/// ⚠ 颜色是**用户输入**（手改 polish.toml、老工程、前端取色器）：这里绝不能索引越界 panic。
/// 3 位简写按 CSS 语义展开（`#abc` = `#aabbcc`，与前端 `fxHexToRgb` 同款），
/// 非 ASCII 或长度不对直接回退。
fn hex_to_rgb(hex: &str) -> (u8, u8, u8) {
    let h = hex.trim().trim_start_matches('#');
    let full: String = match h.len() {
        3 if h.is_ascii() => h.chars().flat_map(|c| [c, c]).collect(),
        6 if h.is_ascii() => h.to_string(),
        _ => return (0, 0, 0),
    };
    let part = |i: usize| u8::from_str_radix(&full[i..i + 2], 16).unwrap_or(0);
    (part(0), part(2), part(4))
}

// ===== 封面背景模式（[cover] bg_mode / bg_color / blur_scale）=====

/// 模糊平铺的基准模糊半径（px，1920×1080 画布）；预览侧同源，改动要两边一起改。
pub const COVER_BLUR_BASE_PX: f64 = 28.0;
/// 模糊倍率的合法区间（滑杆与接口都按它夹取）。
pub const COVER_BLUR_SCALE_MIN: f64 = 0.2;
pub const COVER_BLUR_SCALE_MAX: f64 = 4.0;

/// 背景模式规范化：`color`（纯色，默认）/ `blur`（当前帧模糊平铺）/ `image`（导入的自定义背景图）。
/// 其它任何值（含老配置里的 `frame`）一律当 `color`。
///
/// ⚠️ `image` 还要**文件真的在**才成立 —— 缺文件时退回 `color`，见 [`cover_bg_vars`]。
pub fn cover_bg_mode(cc: &crate::project::CoverConfig) -> &'static str {
    match cc.bg_mode.trim().to_ascii_lowercase().as_str() {
        "blur" => "blur",
        "image" => "image",
        _ => "color",
    }
}

/// 只接受 `#rrggbb` / `#rgb`（注入 HTML 前必须净化：颜色直接进模板与 CSS，不能原样吃字符串）。
pub fn sanitize_hex_color(raw: &str, fallback: &str) -> String {
    let s = raw.trim();
    let hex = s.strip_prefix('#').unwrap_or(s);
    if (hex.len() == 6 || hex.len() == 3) && hex.chars().all(|c| c.is_ascii_hexdigit()) {
        format!("#{}", hex.to_ascii_lowercase())
    } else {
        fallback.to_string()
    }
}

/// 模糊倍率 → CSS 的 `blur(<px>)` 与铺满用的放大倍数。
///
/// 放大是为了把模糊溢出的边缘推出画面（blur 的有效扩散约 3σ，`blur_px` 就是 σ）。
/// 公式在预览（CoverDesigner.vue）里有一份镜像，改这里要同步改那边。
pub fn cover_blur_css(scale: f64) -> (u32, f64) {
    let s = if scale.is_finite() {
        scale.clamp(COVER_BLUR_SCALE_MIN, COVER_BLUR_SCALE_MAX)
    } else {
        1.0
    };
    let blur_px = COVER_BLUR_BASE_PX * s;
    let img_scale = 1.0 + blur_px / 150.0;
    (blur_px.round() as u32, (img_scale * 1000.0).round() / 1000.0)
}

/// 关键帧层（当前帧清晰图）的文件名。**只属于帧层** —— 背景层的图源见 [`COVER_BG_IMG`]。
pub const COVER_FRAME_IMG: &str = "_cover_frame.jpg";
/// 背景层「自定义图片」的文件名（导入背景图片写这里，不再覆盖关键帧）。
pub const COVER_BG_IMG: &str = "_cover_bg.jpg";

/// 背景层 → 模板变量。**预览与出片两个构建器共用**，别再各写一份（旧代码已有过重复分叉）。
/// `bg_image` = `_cover_bg.jpg` 是否存在（不存在时 `image` 模式退回 `color`，免得模板里挂个死图）。
pub struct CoverBgVars {
    pub class: String,
    pub color: String,
    pub blur: String,
    pub blur_scale: String,
    /// `#bg img` 的 src：blur 模式读关键帧、image 模式读背景图、color 模式随便（图不显示）
    pub src: String,
}

pub fn cover_bg_vars(config: &crate::project::ProjectFile, bg_image: bool) -> CoverBgVars {
    let cc = &config.cover;
    let color = sanitize_hex_color(&cc.bg_color, "#000000");
    let (blur_px, img_scale) = cover_blur_css(cc.blur_scale);
    let mut mode = cover_bg_mode(cc);
    if mode == "image" && !bg_image {
        mode = "color";
    }
    CoverBgVars {
        class: format!("bg-{mode}"),
        color,
        blur: format!("{blur_px}px"),
        blur_scale: format!("{img_scale}"),
        src: if mode == "image" { COVER_BG_IMG } else { COVER_FRAME_IMG }.to_string(),
    }
}

/// 帧层（当前帧清晰图）的位移/缩放 → 模板变量。
///
/// 原点取 `layout.liver` 的聚焦点（优先序见 `region_origin`：自由选框中心 > 格子 > 百分比；
/// 老逻辑：liver 在右半边 → 基准 1.4 倍，否则 1.3 倍），再乘 `[cover] bg_zoom`，
/// 最后按 `bg_x/bg_y`（px）平移。
/// **帧层与背景层是两回事**：背景（颜色/模糊帧）铺在帧之下，帧缩小/平移时才会露出来。
/// 前端 `CoverDesigner.vue` 的 `mediaStyle`/`liverOrigin`/`GRID_CENTERS` 是同一套公式的镜像，
/// 改一边要改另一边（漏改过的：自由选框优先序、bg_zoom 下限）。
pub struct CoverFrameVars {
    pub liver_x: String,
    pub liver_y: String,
    pub transform: String,
}

pub fn cover_frame_vars(config: &crate::project::ProjectFile) -> CoverFrameVars {
    let cc = &config.cover;
    let origin = crate::project::region_origin(&config.layout.liver).unwrap_or_else(|| "65% 50%".into());
    let nums: Vec<f64> = origin
        .split_whitespace()
        .filter_map(|s| s.trim_end_matches('%').parse().ok())
        .collect();
    let lx = nums.first().copied().unwrap_or(65.0);
    let ly = nums.get(1).copied().unwrap_or(50.0);
    let base_zoom = if lx >= 50.0 { 1.4 } else { 1.3 };
    // 夹取范围与设计器的滚轮/后端 patch（`api.rs` 的 `bg_zoom.clamp(0.5, 3.0)`）**一致**：
    // 以前这里是 0.2 起步，手写过 0.3 的工程出片会与预览对不上
    let zoom = base_zoom * cc.bg_zoom.clamp(0.5, 3.0);
    CoverFrameVars {
        liver_x: format!("{lx}%"),
        liver_y: format!("{ly}%"),
        transform: format!("translate({:.1}px, {:.1}px) scale({zoom:.3})", cc.bg_x, cc.bg_y),
    }
}

/// 出片时抓取的区域：就是 `#page` 的整块画布（配合模板里的 `html.export`，
/// 页面不再缩放/居中，左上角对齐视口原点）。不这么抓拿到的是浏览器视口尺寸
/// （实测 1904×929），页面被居中 → 成片四周黑边。
/// ⚠️ 画布**恒 1920×1080 横版**，不读 `[canvas] orientation`（平台封面就是横的）。
/// 竖屏素材不改比例靠模板的 `object-fit:contain`，改这两个常量前先看那个测试。

/// 进模板前必须转义：`fx.text`（emoji 字符）是用户输入，直接拼进 HTML 就是模板注入。
fn escape_html(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for ch in raw.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(ch),
        }
    }
    out
}

/// `kind = emoji` 的默认字符（没填 text 时贴这个）。
pub const COVER_EMOJI_DEFAULT: &str = "🙂";
/// emoji 文本最多留几个字符（带 ZWJ / 肤色修饰符的一个 emoji 就要 7~8 个码点）。
pub const COVER_EMOJI_MAX_CHARS: usize = 12;
/// emoji 尺寸区间（px，1920×1080 画布）：amount 0 → 48px，1 → 288px。
pub const COVER_EMOJI_MIN_PX: f64 = 48.0;
pub const COVER_EMOJI_MAX_PX: f64 = 288.0;

/// emoji 的字体栈：Windows 的 Segoe UI Emoji、macOS 的 Apple Color Emoji、Linux 的 Noto。
const EMOJI_FONT: &str = "'Segoe UI Emoji','Apple Color Emoji','Noto Color Emoji',sans-serif";

fn cover_fx_html(cc: &crate::project::CoverConfig) -> String {
    let mut out = String::new();
    for fx in &cc.effects {
        let x = fx.x.clamp(0.0, 100.0);
        let y = fx.y.clamp(0.0, 100.0);
        let a = fx.amount.clamp(0.0, 1.0);
        // 大多数特效是铺满画面的图层，没有内容；emoji 是"一个点 + 一串字符"
        let (style, content) = match fx.kind.as_str() {
            "glow" => (
                format!(
                    "position:absolute;inset:0;background:radial-gradient(circle at {x:.1}% {y:.1}%, rgba(255,220,180,{:.2}) 0%, transparent 60%);",
                    a.min(0.9)
                ),
                String::new(),
            ),
            "vignette" => (
                format!(
                    "position:absolute;inset:0;background:radial-gradient(circle at {x:.1}% {y:.1}%, transparent 40%, rgba(0,0,0,{:.2}) 100%);",
                    a
                ),
                String::new(),
            ),
            "blur" => (
                format!(
                    "position:absolute;inset:0;backdrop-filter:blur({}px);-webkit-mask-image:radial-gradient(circle at {x:.1}% {y:.1}%, black 0%, transparent 60%);mask-image:radial-gradient(circle at {x:.1}% {y:.1}%, black 0%, transparent 60%);",
                    (1.0 + a * 14.0) as u32
                ),
                String::new(),
            ),
            "overlay" => {
                let (r, g, b) = hex_to_rgb(&fx.color);
                (
                    format!(
                        "position:absolute;inset:0;background:rgba({r},{g},{b},{:.3});mix-blend-mode:overlay;",
                        a
                    ),
                    String::new(),
                )
            }
            "scanline" => (
                format!(
                    "position:absolute;inset:0;background:repeating-linear-gradient(0deg, rgba(255,255,255,{:.3}) 0 1px, transparent 1px 4px);",
                    (a * 0.45).min(0.5)
                ),
                String::new(),
            ),
            // 表情：x/y 是**字符中心**（画布百分比）。默认落在主播位置（见 cover_emoji_origin），
            // 帧层的缩放原点就是那里 —— 那个点是变换的不动点，所以提示贴上去后缩放也不会跑偏。
            "emoji" => {
                let raw = if fx.text.trim().is_empty() { COVER_EMOJI_DEFAULT } else { fx.text.trim() };
                // 只留几个字符（emoji 带 ZWJ / 肤色修饰符一个就 7~8 个码点），
                // 别让一长串字把封面糊了
                let text: String = raw.chars().take(COVER_EMOJI_MAX_CHARS).collect();
                let size = (COVER_EMOJI_MIN_PX + a * (COVER_EMOJI_MAX_PX - COVER_EMOJI_MIN_PX)).round() as u32;
                (
                    format!(
                        "position:absolute;left:{x:.1}%;top:{y:.1}%;transform:translate(-50%,-50%);font-size:{size}px;line-height:1;font-family:{EMOJI_FONT};filter:drop-shadow(0 6px 14px rgba(0,0,0,0.45));"
                    ),
                    escape_html(&text),
                )
            }
            _ => continue,
        };
        out.push_str(&format!("<div style=\"{style}\">{content}</div>"));
    }
    out
}


// ============================================================

pub async fn cmd_cover_template(project_path: &Path, template: &str) -> anyhow::Result<()> {
    // 读-改-写：只动 cover 预设字段，别把用户同时改的其它配置覆盖掉
    let tpl = template.to_string();
    project::update_polish_toml(project_path, move |cfg| {
        let cover = &mut cfg.cover;
        match tpl.as_str() {
            "center-text" => {
                cover.text_x = "50%".into();
                cover.text_width = 0.8;
                cover.gradient_dir = "radial".into();
                cover.vignette = 0.05;
                cover.text_align = "center".into();
                cover.shadow_dir = "center".into();
                cover.text_shadow = true;
            }
            "right-text" => {
                cover.text_x = "65%".into();
                cover.text_width = 0.62;
                cover.gradient_dir = "right".into();
                cover.text_align = "right".into();
                cover.shadow_dir = "right".into();
                cover.text_shadow = true;
            }
            "minimal" => {
                cover.text_x = "50%".into();
                cover.text_width = 0.7;
                cover.gradient_dir = "none".into();
                cover.vignette = 0.15;
                cover.text_align = "center".into();
                cover.shadow_dir = "none".into();
                cover.text_shadow = false;
            }
            _ => { // left-text / 未知: 恢复默认
                cover.text_x = "35%".into();
                cover.text_width = 0.62;
                cover.gradient_dir = "left".into();
                cover.vignette = 0.08;
                cover.text_align = "left".into();
                cover.shadow_dir = "left".into();
                cover.text_shadow = true;
            }
        }
    })
    .await?;
    tracing::info!("  ✓ [cover] preset: {}", template);
    Ok(())
}

pub async fn cmd_preview_cover(project_path: &Path) -> anyhow::Result<()> {
    let config = project::load_project(project_path)?;
    let resolved = resolve_paths(project_path, &config);
    let cover_image = project_path.join(COVER_FRAME_IMG);
    if !cover_image.exists() {
        // 没有封面帧则截一张
        let video_path = &resolved.video;
        let duration = config.segments.last().map(|s| s.end).unwrap_or(0.0);
        let frame_time = (duration * config.cover.frame_pos).max(0.1);
        let data = std::process::Command::new("ffmpeg")
            .args(["-ss", &frame_time.to_string(), "-i", video_path,
                   "-vframes", "1", "-f", "image2pipe", "-vcodec", "mjpeg", "-q:v", "3", "-"])
            .stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::null())
            .output()?.stdout;
        if !data.is_empty() { std::fs::write(&cover_image, &data)?; }
    }

    let html = build_cover_html(project_path, &config, &resolved);
    let port = server::start_file_server(project_path.to_path_buf(), Some(html)).await?;
    let url = format!("http://127.0.0.1:{}", port);
    tracing::info!("  Cover URL: {}", url);
    std::process::Command::new("cmd").args(["/c", "start", "", &url]).spawn().ok();
    loop { tokio::time::sleep(std::time::Duration::from_secs(3600)).await; }
}

/// 封面画布：**恒 1920×1080 横版**（不跟随 `[canvas] orientation` —— 平台封面就是横的）。
/// 竖屏素材在封面里靠模板的 `object-fit:contain` 保持原比例，画布尺寸本身不动。
pub const COVER_CAPTURE_W: f64 = 1920.0;
pub const COVER_CAPTURE_H: f64 = 1080.0;

pub fn build_cover_html(project_path: &Path, config: &project::ProjectFile, _resolved: &ResolvedPaths) -> String {
    let bg_image = project_path.join(COVER_BG_IMG).is_file();
    // 从 cmd_cover 中提取相同逻辑 (略缩版)
    let cc = &config.cover;
    // 封面画布**恒横版**（不读 `[canvas] orientation`）：平台封面就是横的。
    // 竖屏素材靠模板的 `object-fit:contain` 保持原比例，画布尺寸本身不动。
    let (cw, ch) = (COVER_CAPTURE_W, COVER_CAPTURE_H);

    // 渐变（走 hex_to_rgb：3 位简写/非法值都不能越界 panic）
    let hl_color = &config.style.subtitle.highlight_color;
    let (gr, gg, gb) = if !hl_color.is_empty() && hl_color.starts_with('#') {
        let (r, g, b) = hex_to_rgb(hl_color);
        let (r, g, b) = (r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0);
        let nr = ((1.0 - r) * 30.0) as u8;
        let ng = ((1.0 - g) * 25.0) as u8;
        let nb = ((1.0 - b) * 50.0) as u8;
        (nr.max(10), ng.max(10), nb.max(15))
    } else { (20, 10, 40) };
    let grad_from = format!("rgba({},{},{},0.55)", gr, gg, gb);
    let grad_to = format!("rgba({},{},{},0.15)", gr, gg, gb);
    let grad_bg = match cc.gradient_dir.as_str() {
        "right" => format!("linear-gradient(270deg,{} 0%,{} 50%,transparent 100%)", grad_from, grad_to),
        "top" => format!("linear-gradient(180deg,{} 0%,{} 50%,transparent 100%)", grad_from, grad_to),
        "bottom" => format!("linear-gradient(0deg,{} 0%,{} 50%,transparent 100%)", grad_from, grad_to),
        "radial" => format!("radial-gradient(ellipse at 30% 50%,{} 0%,{} 60%,transparent 100%)", grad_from, grad_to),
        _ if cc.gradient_dir == "none" => "none".into(),
        _ => format!("linear-gradient(90deg,{} 0%,{} 50%,transparent 100%)", grad_from, grad_to),
    };

    // 标题
    let raw = &config.project.name;
    let clean = raw.split('【').next().unwrap_or(raw);
    let hl_words = cover_highlight_words(config);
    let title_lines: Vec<String> = if !cc.title_lines.is_empty() {
        cc.title_lines.clone()
    } else {
        clean.split(['，', '。', '！', '？', '、', ',', '.', '!'])
            .map(|s| s.trim()).filter(|s| !s.is_empty()).map(|s| s.to_string()).collect()
    };
    let title_html = title_lines.iter().map(|line| {
        if hl_words.is_empty() { return line.to_string(); }
        let mut r = line.to_string();
        for w in &hl_words { if !w.is_empty() && r.contains(w.as_str()) { r = r.replace(w.as_str(), &format!("<span class=\"hl\">{}</span>", w)); } }
        r
    }).collect::<Vec<_>>().join("<br>");

    let (text_align, align_items) = cover_text_align(&cc);
    let text_shadow = cover_shadow_css(&cc);
    let fx_html = cover_fx_html(&cc);
    let bg = cover_bg_vars(config, bg_image);
    let frame = cover_frame_vars(config);
    let font = cover_font_css(&cc);
    COVER_HTML
        .replace("__COVER_W__", &cw.to_string())
        .replace("__COVER_H__", &ch.to_string())
        .replace("__TITLE__", &title_html)
        .replace("__FONT_FAMILY__", &font.family)
        .replace("__BG_CLASS__", &bg.class)
        .replace("__BG_COLOR__", &bg.color)
        .replace("__BG_BLUR__", &bg.blur)
        .replace("__BG_BLUR_SCALE__", &bg.blur_scale)
        .replace("__BG_SRC__", &bg.src)
        .replace("__LIVER_X__", &frame.liver_x)
        .replace("__LIVER_Y__", &frame.liver_y)
        .replace("__BG_TRANSFORM__", &frame.transform)
        .replace("__EXPORT__", "false")
        .replace("__GRAD_BG__", &grad_bg)
        .replace("__VIGNETTE_OP__", &cc.vignette.to_string())
        .replace("__FONT_SIZE__", &format!("{}px", cc.font_size))
        .replace("__STROKE__", &format!("{}px", cc.stroke))
        .replace("__HL_COLOR__", &cc.hl_color)
        .replace("__TEXT_WIDTH__", &format!("{}%", (cc.text_width * 100.0) as u32))
        .replace("__TEXT_X__", &cc.text_x)
        .replace("__TEXT_Y__", &cc.text_y)
        .replace("__LS__", &cc.letter_spacing)
        .replace("__LH__", &cc.line_height.to_string())
        .replace("__TEXT_ALIGN__", text_align)
        .replace("__TEXT_ALIGN_ITEMS__", align_items)
        .replace("__SHADOW__", &text_shadow)
        .replace("__FX_HTML__", &fx_html)
        .replace("<!-- __FONT_FACE__ -->", &font.face)
}


pub async fn cmd_cover(project_path: &Path, existing_video: Option<&str>) -> anyhow::Result<()> {
    tracing::info!("\n  -- Cover --");
    let config = project::load_project(project_path)?;
    let resolved = resolve_paths(project_path, &config);
    let cc = &config.cover;

    // 首次运行时自动写入 [cover] 节（读-改-写：磁盘上其余字段原样保留）
    let raw_config = std::fs::read_to_string(project_path.join("polish.toml")).unwrap_or_default();
    if !raw_config.contains("[cover]") {
        project::update_polish_toml(project_path, |_| {}).await?;
        tracing::info!("  ✓ polish.toml: [cover] section added");
    }

    // 背景图: 自定义 image 或 ffmpeg 截帧
    let duration = config.segments.last().map(|s| s.end).unwrap_or(0.0);
    if cc.image.is_empty() {
        // 优先使用已导出的关键帧（cover-frame 手动导出/导入背景生成），不要每次重新截帧覆盖
        let manual_frame = project_path.join(COVER_FRAME_IMG);
        if !manual_frame.exists() {
            let video_path = existing_video.unwrap_or(&resolved.video);
            let frame_time = (duration * cc.frame_pos).max(0.1);
            let frame_data = std::process::Command::new("ffmpeg")
                .args(["-ss", &frame_time.to_string(), "-i", video_path,
                       "-vframes", "1", "-f", "image2pipe", "-vcodec", "mjpeg", "-q:v", "3", "-"])
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::null())
                .output()?.stdout;
            if frame_data.is_empty() { anyhow::bail!("Frame extraction failed"); }
            std::fs::write(&manual_frame, &frame_data)?;
        }
    } else {
        // `cc.image` 是配置里的字符串（`polish.toml` 可手改），不能无条件 `join` 后直接复制：
        // 绝对路径下 `join` 会**原样返回它本身**，于是指向项目外的任意文件都会被复制进来
        // 当作封面关键帧。这里用项目根做一次路径囚笼 —— 与 `/api/polish/*` 的 scope 守卫
        // （`server/scope.rs::resolve_file_in_project`）是两道**互相独立**的防线，别删任何一道。
        let img_path = vtrim_core::tenant::ensure_within(project_path, Path::new(&cc.image))
            .map_err(|e| anyhow::anyhow!("Cover image 越出项目目录（{}）: {e:#}", cc.image))?;
        if !img_path.exists() { anyhow::bail!("Cover image not found: {}", img_path.display()); }
        std::fs::copy(&img_path, project_path.join(COVER_FRAME_IMG))?;
        tracing::info!("    using custom image: {}", cc.image);
    }

    // 渐变颜色: 基于字幕高亮色
    let hl_color = &config.style.subtitle.highlight_color;
    let _sub_color = &config.style.subtitle.color;
    let (grad_r, grad_g, grad_b) = if !hl_color.is_empty() && hl_color.starts_with('#') {
        // 走 hex_to_rgb：3 位简写 / 非法值都不能越界 panic（颜色是用户输入）
        let (r, g, b) = hex_to_rgb(hl_color);
        let (r, g, b) = (r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0);
        // complementary: invert and darken
        let nr = ((1.0 - r) * 30.0) as u8;
        let ng = ((1.0 - g) * 25.0) as u8;
        let nb = ((1.0 - b) * 50.0) as u8;
        (nr.max(10), ng.max(10), nb.max(15))
    } else {
        (20, 10, 40)
    };
    let grad_from = format!("rgba({},{},{},0.55)", grad_r, grad_g, grad_b);
    let grad_to = format!("rgba({},{},{},0.15)", grad_r, grad_g, grad_b);

    // 渐变背景 CSS
    let grad_bg = match cc.gradient_dir.as_str() {
        "right" => format!("linear-gradient(270deg,{} 0%,{} 50%,transparent 100%)", grad_from, grad_to),
        "top" => format!("linear-gradient(180deg,{} 0%,{} 50%,transparent 100%)", grad_from, grad_to),
        "bottom" => format!("linear-gradient(0deg,{} 0%,{} 50%,transparent 100%)", grad_from, grad_to),
        "radial" => format!("radial-gradient(ellipse at 30% 50%,{} 0%,{} 60%,transparent 100%)", grad_from, grad_to),
        "none" => "none".into(),
        _ => format!("linear-gradient(90deg,{} 0%,{} 50%,transparent 100%)", grad_from, grad_to),
    };

    // 标题: 先读 title_lines, 空则自动分行 + 写回 config
    let raw = &config.project.name;
    let clean = raw.split('【').next().unwrap_or(raw);
    let hl_words = cover_highlight_words(&config);

    let title_lines: Vec<String> = if !cc.title_lines.is_empty() {
        cc.title_lines.clone()
    } else {
        let lines: Vec<String> = clean
            .split(['，', '。', '！', '？', '、', ',', '.', '!'])
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .collect();
        if lines.len() > 1 {
            // 写入 polish.toml (仅首次)：读-改-写，只动 title_lines
            let to_write = lines.clone();
            project::update_polish_toml(project_path, move |cfg| {
                cfg.cover.title_lines = to_write;
            })
            .await?;
        }
        lines
    };

    // 关键词高亮
    let title_html = title_lines.iter().map(|line| {
        if hl_words.is_empty() { return line.to_string(); }
        let mut result = line.to_string();
        for w in &hl_words {
            if !w.is_empty() && result.contains(w.as_str()) {
                result = result.replace(w.as_str(), &format!("<span class=\"hl\">{}</span>", w));
            }
        }
        result
    }).collect::<Vec<_>>().join("<br>");

    let (text_align, align_items) = cover_text_align(&cc);
    let text_shadow = cover_shadow_css(&cc);
    let fx_html = cover_fx_html(&cc);
    let bg = cover_bg_vars(&config, project_path.join(COVER_BG_IMG).is_file());
    let frame = cover_frame_vars(&config);
    // 封面字体（`[cover] font` = 字体库里的 id）：与预览链同一份替换，**两条都要改**
    let font = cover_font_css(&cc);
    // 封面画布**恒横版**：与 build_cover_html 同一把尺（竖屏素材不改比例，见模板 contain）
    let (cw, ch) = (COVER_CAPTURE_W, COVER_CAPTURE_H);
    let cover_html = COVER_HTML
        .replace("__COVER_W__", &cw.to_string())
        .replace("__COVER_H__", &ch.to_string())
        .replace("__TITLE__", &title_html)
        .replace("__FONT_FAMILY__", &font.family)
        .replace("__BG_CLASS__", &bg.class)
        .replace("__BG_COLOR__", &bg.color)
        .replace("__BG_BLUR__", &bg.blur)
        .replace("__BG_BLUR_SCALE__", &bg.blur_scale)
        .replace("__BG_SRC__", &bg.src)
        .replace("__LIVER_X__", &frame.liver_x)
        .replace("__LIVER_Y__", &frame.liver_y)
        .replace("__BG_TRANSFORM__", &frame.transform)
        .replace("__EXPORT__", "true")
        .replace("__GRAD_BG__", &grad_bg)
        .replace("__VIGNETTE_OP__", &cc.vignette.to_string())
        .replace("__FONT_SIZE__", &format!("{}px", cc.font_size))
        .replace("__STROKE__", &format!("{}px", cc.stroke))
        .replace("__HL_COLOR__", &cc.hl_color)
        .replace("__TEXT_WIDTH__", &format!("{}%", (cc.text_width * 100.0) as u32))
        .replace("__TEXT_X__", &cc.text_x)
        .replace("__TEXT_Y__", &cc.text_y)
        .replace("__LS__", &cc.letter_spacing)
        .replace("__LH__", &cc.line_height.to_string())
        .replace("__TEXT_ALIGN__", text_align)
        .replace("__TEXT_ALIGN_ITEMS__", align_items)
        .replace("__SHADOW__", &text_shadow)
        .replace("__FX_HTML__", &fx_html)
        .replace("<!-- __FONT_FACE__ -->", &font.face);

    let port = server::start_file_server(project_path.to_path_buf(), Some(cover_html)).await?;
    let url = format!("http://127.0.0.1:{}/", port);

    // 封面画布**恒横版**（1920×1080），与成片方向解耦
    let browser = ChromeRenderer::launch((cw as u32, ch as u32))?;
    let tab = ChromeRenderer::create_tab(&browser, &url)?;
    tab.set_default_timeout(std::time::Duration::from_secs(30));

    // 视口必须**正好**是封面画布：headless 窗口名义尺寸实测视口更小（如 1904×929），
    // 视口之外的那截不会被绘制 —— 不设的话抓出来底部（以及左右）会是黑边。
    {
        use headless_chrome::protocol::cdp::Emulation;
        tab.call_method(Emulation::SetDeviceMetricsOverride {
            width: COVER_CAPTURE_W as u32,
            height: COVER_CAPTURE_H as u32,
            device_scale_factor: 1.0,
            mobile: false,
            scale: None,
            screen_width: None,
            screen_height: None,
            position_x: None,
            position_y: None,
            dont_set_visible_size: None,
            screen_orientation: None,
            viewport: None,
            display_feature: None,
            device_posture: None,
        })?;
    }

    for _ in 0..100 {
        let ready = tab.evaluate("window.__COVER_READY === true", false)?;
        if ready.value.and_then(|v| v.as_bool()).unwrap_or(false) { break; }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }

        let jpeg_data = tab.call_method(Page::CaptureScreenshot {
        format: Some(Page::CaptureScreenshotFormatOption::Jpeg),
        quality: Some(85),
        // 精确抓 `#page` 那块画布（模板的 `html.export` 已把它对齐到视口原点、不缩放）。
        // 之前 clip: None 抓到的是浏览器视口尺寸（实拍 1904×929），页面被居中
        // → 成片四周黑边，尺寸也不对。
        clip: Some(Page::Viewport {
            x: 0.0,
            y: 0.0,
            width: cw as f64,
            height: ch as f64,
            scale: 1.0,
        }),
        from_surface: Some(true),
        capture_beyond_viewport: Some(true),
        optimize_for_speed: Some(false),
    })?;
    use base64::Engine;
use headless_chrome::protocol::cdp::Page;
    let jpeg = base64::engine::general_purpose::STANDARD.decode(&jpeg_data.data)?;

    let cover_file = crate::render::unique_output_path(&project_path.join("cover.jpg"));
    std::fs::write(&cover_file, &jpeg)?;
    // ⚠️ **不要删 `_cover_frame.jpg`**：它就是「关键帧层」的图源（封面三层 = 文字 / 关键帧 / 背景）。
    // 删掉的话：① 用户导出的那一帧（含他选的时间点）白导；② 下次出片又按 frame_pos 重新截，
    // 同一份封面每次出片都可能换脸。背景层的图源是另一个文件（COVER_BG_IMG），互不影响。
    tracing::info!("  ✓ Cover saved: {}", cover_file.display());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 封面字体：没选（空串）→ 系统字体栈 + 不注入；选了库外的值 → 当系统族名用、仍要有兜底栈，
    /// 而且**不注入 @font-face**（注了就是一条 404）。
    #[test]
    fn cover_font_css_falls_back_to_system_stack() {
        let mut cc = crate::project::CoverConfig::default();
        let off = cover_font_css(&cc);
        assert_eq!(off.family, server::SYSTEM_FONT_STACK);
        assert_eq!(off.face, "");

        cc.font = "not-in-library-xyz".into();
        let miss = cover_font_css(&cc);
        assert!(miss.family.starts_with("\"not-in-library-xyz\","), "{}", miss.family);
        assert!(miss.family.contains("Microsoft YaHei"), "后面必须还有系统栈兜底: {}", miss.family);
        assert_eq!(miss.face, "");
    }

    /// 颜色是用户输入（手改 polish.toml / 老工程 / 取色器）：3 位简写、非 ASCII、空串都不能 panic。
    /// 曾经这里是 `&h[0..2]` 直接切片 —— `#f00` 这种合法短写法会越界 panic，出片直接崩。
    #[test]
    fn hex_to_rgb_never_panics_and_expands_short_hex() {
        assert_eq!(hex_to_rgb("#ffcc00"), (255, 204, 0));
        assert_eq!(hex_to_rgb("ffcc00"), (255, 204, 0));
        assert_eq!(hex_to_rgb("#f00"), (255, 0, 0)); // 3 位简写 = #ff0000
        assert_eq!(hex_to_rgb("#abc"), (0xaa, 0xbb, 0xcc));
        assert_eq!(hex_to_rgb(""), (0, 0, 0));
        assert_eq!(hex_to_rgb("#"), (0, 0, 0));
        assert_eq!(hex_to_rgb("#12345"), (0, 0, 0)); // 长度不对
        assert_eq!(hex_to_rgb("#啊啊啊啊"), (0, 0, 0)); // 非 ASCII（字节切分会崩）
        assert_eq!(hex_to_rgb("#zzzzzz"), (0, 0, 0)); // 非十六进制
    }

    fn cfg_with(bg_mode: &str, color: &str, scale: f64) -> crate::project::ProjectFile {
        let mut c: crate::project::ProjectFile =
            toml::from_str("[source]\nvideo = \"clip.mp4\"\nsubtitles = \"clip.srt\"\n").unwrap();
        c.cover.bg_mode = bg_mode.into();
        c.cover.bg_color = color.into();
        c.cover.blur_scale = scale;
        c
    }

    fn resolved() -> ResolvedPaths {
        ResolvedPaths {
            video: "clip.mp4".into(),
            subtitles: "clip.srt".into(),
            danmaku: String::new(),
            metadata: String::new(),
            output: String::new(),
        }
    }

    /// 背景模式要落进 HTML，模板里**不留任何 `__*__` 占位符**
    /// （漏替一个占位符，出片就会出现裸字面量或整块背景失效）。
    /// 帧层（`#frame`）与背景层（`#bg`）是两层：帧可位移缩放、背景在帧之下。
    #[test]
    fn cover_html_renders_all_bg_modes() {
        for (mode, class) in [("color", "bg-color"), ("blur", "bg-blur")] {
            let html = build_cover_html(Path::new("."), &cfg_with(mode, "#123456", 2.0), &resolved());
            assert!(html.contains(&format!("class=\"{class}\"")), "{mode} 的 #bg 类不对：{html}");
            assert!(
                html.contains("'--bg-color', '#123456'"),
                "{mode} 的背景色没注入：{html}"
            );
            // 帧层必须单独一层（背景是"加的一层"，不是替换帧），而且**排在 #bg 之后**（在它之上）
            let bg_at = html.find("id=\"bg\"").expect("缺少背景层");
            let frame_at = html.find("id=\"frame\"").expect("缺少帧层");
            assert!(bg_at < frame_at, "{mode} 帧层必须在背景层之上：{html}");
            assert!(!html.contains("__BG_"), "{mode} 还有未替换的占位符：{html}");
            assert!(!html.contains("__LIVER_") && !html.contains("__BG_TRANSFORM__"),
                "{mode} 帧层占位符没替换：{html}");
            // 预览构建器必须写 false（保持缩放铺满 + 刷新按钮）；出片走 cmd_cover 的 true
            assert!(
                html.contains("var EXPORT = 'false'"),
                "{mode} 的 __EXPORT__ 没替换：{html}"
            );
            // 字体栈占位符必须被替换掉 —— 留着就是一条非法的 font-family，整个字体设置静默失效
            assert!(!html.contains("__FONT_FAMILY__"), "{mode} 字体栈占位符没替换：{html}");
            assert!(html.contains(server::SYSTEM_FONT_STACK), "{mode} 缺系统字体栈兜底：{html}");
        }
    }

    /// 封面画布**恒 1920×1080 横版**：即使工程是竖屏（`[canvas] orientation = portrait`）
    /// 封面也不跟着变 —— 平台封面就是横的，竖屏素材靠模板 `object-fit:contain` 保原比例。
    /// （曾经跟随过 orientation，用户要求改回固定横版。）
    #[test]
    fn cover_html_page_size_is_always_landscape() {
        for (label, orientation) in [("默认", ""), ("显式横屏", "landscape"), ("竖屏", "portrait")] {
            let mut c = cfg_with("color", "#000000", 1.0);
            if !orientation.is_empty() {
                c.canvas.orientation = orientation.into();
            }
            let html = build_cover_html(Path::new("."), &c, &resolved());
            assert!(html.contains("width:1920px; height:1080px"),
                "{label}封面 #page 必须是 1920×1080：{html}");
            assert!(html.contains("window.innerWidth/1920"),
                "{label}封面缩放基准必须是 1920：{html}");
            assert!(!html.contains("__COVER_W__") && !html.contains("__COVER_H__"),
                "{label}封面画布占位符没替换：{html}");
        }
    }

    /// 帧层 transform：原点跟 `layout.liver`（右半边 1.4 倍、左半边 1.3 倍）× `bg_zoom`，
    /// 平移取 `bg_x/bg_y`；越界倍率夹到 0.2~3。
    #[test]
    fn cover_frame_transform_follows_liver_and_zoom() {
        let mut c = cfg_with("color", "#000000", 1.0);
        // grid/origin_grid 优先于 origin（region_origin 的规则），要测 origin 就先清掉
        c.layout.liver.grid.clear();
        c.layout.liver.origin_grid = None;
        c.layout.liver.origin = Some("65% 50%".into());
        let v = cover_frame_vars(&c);
        assert_eq!(v.liver_x, "65%");
        assert_eq!(v.liver_y, "50%");
        assert!(v.transform.contains("scale(1.400)"), "{}", v.transform);
        assert!(v.transform.starts_with("translate(0.0px, 0.0px)"), "{}", v.transform);

        c.cover.bg_zoom = 0.5;
        c.cover.bg_x = -120.0;
        c.cover.bg_y = 30.0;
        let v = cover_frame_vars(&c);
        assert!(v.transform.contains("scale(0.700)"), "{}", v.transform);
        assert!(v.transform.contains("translate(-120.0px, 30.0px)"), "{}", v.transform);

        c.layout.liver.origin = Some("20% 50%".into());
        c.cover.bg_zoom = 1.0;
        let v = cover_frame_vars(&c);
        assert_eq!(v.liver_x, "20%");
        assert!(v.transform.contains("scale(1.300)"), "{}", v.transform);

        // 倍率本身夹到 3（与接口写入侧的 0.5~3 一致），所以 1.3×3 = 3.9
        c.cover.bg_zoom = 99.0;
        let v = cover_frame_vars(&c);
        assert!(v.transform.contains("scale(3.900)"), "倍率封顶 3：{}", v.transform);
        c.cover.bg_zoom = 0.0;
        let v = cover_frame_vars(&c);
        // 下限 0.5（设计器滚轮 / 后端 patch 都按 0.5~3 夹）→ 1.3×0.5 = 0.65
        assert!(v.transform.contains("scale(0.650)"), "倍率下限 0.5：{}", v.transform);
    }

    /// 模糊倍率：1.0 = 基准 28px；越界夹取；NaN 退回 1.0。
    #[test]
    fn cover_blur_px_and_scale_follow_multiplier() {
        let (px, scale) = cover_blur_css(1.0);
        assert_eq!(px, COVER_BLUR_BASE_PX as u32);
        assert!(scale > 1.0, "放大倍数必须大于 1，否则模糊边缘会露黑边：{scale}");

        assert_eq!(
            cover_blur_css(99.0).0,
            (COVER_BLUR_BASE_PX * COVER_BLUR_SCALE_MAX).round() as u32
        );
        assert_eq!(
            cover_blur_css(0.0).0,
            (COVER_BLUR_BASE_PX * COVER_BLUR_SCALE_MIN).round() as u32
        );
        assert_eq!(cover_blur_css(f64::NAN).0, COVER_BLUR_BASE_PX as u32);
        // 倍率越大铺得越满（不然高模糊时四边会露出底色）
        assert!(cover_blur_css(3.0).1 > cover_blur_css(0.5).1);
    }

    /// 颜色是**直接注进 HTML/CSS** 的字符串，必须净化（否则等于模板注入）。
    #[test]
    fn hex_color_is_sanitized_before_html_injection() {
        assert_eq!(sanitize_hex_color("#AABBCC", "#000000"), "#aabbcc");
        assert_eq!(sanitize_hex_color("abc", "#000000"), "#abc");
        assert_eq!(sanitize_hex_color("  #0f0f0f ", "#000000"), "#0f0f0f");
        assert_eq!(sanitize_hex_color("red;background:url(x)", "#000000"), "#000000");
        assert_eq!(sanitize_hex_color("", "#000000"), "#000000");

        // 走完构建器也要是净化后的值
        let html = build_cover_html(
            Path::new("."),
            &cfg_with("color", "#abc\"/><script>alert(1)</script>", 1.0),
            &resolved(),
        );
        assert!(!html.contains("alert(1)"), "恶意颜色串进了模板：{html}");
        assert!(html.contains("'--bg-color', '#000000'"), "非法颜色应退回默认色：{html}");
    }

    /// emoji 特效：字符进模板、x/y 是字符中心、amount 映射字号；
    /// **text 是用户输入，必须转义**（否则就是往封面 HTML 里注模板）。
    #[test]
    fn cover_emoji_fx_renders_text_and_escapes_html() {
        fn fx(amount: f64, x: f64, y: f64, text: &str) -> crate::project::CoverEffect {
            crate::project::CoverEffect {
                kind: "emoji".into(),
                amount,
                x,
                y,
                color: "#000000".into(),
                text: text.into(),
            }
        }
        let mut c = cfg_with("color", "#000000", 1.0);
        c.cover.effects = vec![
            fx(0.5, 83.3, 50.0, "😆"),
            fx(1.0, 10.0, 20.0, "😆"),
            fx(0.0, 1.0, 2.0, "😆"),
            fx(0.3, 5.0, 6.0, "<b>粗</b>"),
            fx(0.3, 7.0, 8.0, "   "),
            fx(0.3, 9.0, 9.0, "😆😆😆😆😆😆😆😆😆😆😆😆😆😆😆"),
        ];
        let html = cover_fx_html(&c.cover);
        // 位置：x/y 是**字符中心**（translate(-50%,-50%)），缩放/字体栈都要在
        assert!(
            html.contains("left:83.3%;top:50.0%;transform:translate(-50%,-50%)"),
            "emoji 定位不对：{html}"
        );
        assert!(html.contains("Segoe UI Emoji"), "emoji 字体栈没写：{html}");
        // amount → 字号：0.5 → 168px，1 → 288px，0 → 48px
        assert!(html.contains("font-size:168px"), "{html}");
        assert!(html.contains("font-size:288px"), "{html}");
        assert!(html.contains("font-size:48px"), "{html}");
        assert!(html.contains(">😆<"), "表情字符没进模板：{html}");
        // 转义：<b> 必须变成实体，原样的 <b> 不能出现
        assert!(html.contains("&lt;b&gt;粗&lt;/b&gt;"), "{html}");
        assert!(!html.contains("<b>粗</b>"), "用户输入没转义：{html}");
        // 全空白 → 默认表情；超长截到 12 个字符
        assert!(html.contains(&format!(">{}<", COVER_EMOJI_DEFAULT)), "{html}");
        let long = "😆".repeat(COVER_EMOJI_MAX_CHARS);
        assert!(html.contains(&format!(">{long}<")), "长串没截断：{html}");
        assert!(!html.contains(&"😆".repeat(COVER_EMOJI_MAX_CHARS + 1)), "{html}");
    }

    /// 三种模式（color / blur / image）：未知值与老的 `frame` 都当纯色（默认黑）。
    #[test]
    fn unknown_bg_mode_falls_back_to_color() {
        assert_eq!(cover_bg_mode(&cfg_with("weird", "#000000", 1.0).cover), "color");
        assert_eq!(cover_bg_mode(&cfg_with("frame", "#000000", 1.0).cover), "color");
        assert_eq!(cover_bg_mode(&cfg_with("", "#000000", 1.0).cover), "color");
        assert_eq!(cover_bg_mode(&cfg_with("BLUR", "#000000", 1.0).cover), "blur");
        assert_eq!(cover_bg_mode(&cfg_with("IMAGE", "#000000", 1.0).cover), "image");
        // 缺省 = 纯色黑
        let c: crate::project::ProjectFile = toml::from_str("[source]
video=\"a.mp4\"
subtitles=\"a.srt\"
").unwrap();
        assert_eq!(c.cover.bg_mode, "color");
        assert_eq!(c.cover.bg_color, "#000000");
    }

    /// 背景层三种模式 + **背景图与关键帧是两个文件**（导入背景不能盖掉「关键帧层」）。
    #[test]
    fn cover_bg_image_mode_uses_its_own_file() {
        let dir = std::env::temp_dir().join(format!("vtrim-cover-bg-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();

        // 没有 _cover_bg.jpg 时：image 模式退回纯色，免得模板里挂个死图
        let html = build_cover_html(&dir, &cfg_with("image", "#123456", 1.0), &resolved());
        assert!(html.contains("class=\"bg-color\""), "缺背景图时应退回纯色：{html}");

        // 有 _cover_bg.jpg：走 image 模式，背景读它、关键帧读 _cover_frame.jpg
        std::fs::write(dir.join(COVER_BG_IMG), b"x").unwrap();
        let html = build_cover_html(&dir, &cfg_with("image", "#123456", 1.0), &resolved());
        assert!(html.contains("class=\"bg-image\""), "{html}");
        assert!(
            html.contains(&format!("<div id=\"bg\" class=\"bg-image\"><img src=\"{COVER_BG_IMG}\">")),
            "背景层该读自己的图：{html}"
        );
        assert!(
            html.contains(&format!("<div id=\"frame\"><img src=\"{COVER_FRAME_IMG}\">")),
            "关键帧层必须还是 _cover_frame.jpg：{html}"
        );

        // blur 模式读的仍是关键帧
        let html = build_cover_html(&dir, &cfg_with("blur", "#123456", 1.0), &resolved());
        assert!(html.contains("class=\"bg-blur\""), "{html}");
        assert!(html.contains(&format!("<img src=\"{COVER_FRAME_IMG}\">")), "{html}");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 视觉验收用：把三种模式的真实 HTML 落到 `target/tmp-cover/`，
    /// 配一张 `_cover_frame.jpg` 就能用浏览器 1:1 看效果（Chrome 出片就是这份 HTML）。
    /// 跑法：`cargo test -p vtrim-polish --lib dump_cover_variants -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn dump_cover_variants_for_visual_check() {
        let out = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/tmp-cover");
        std::fs::create_dir_all(&out).unwrap();
        for (mode, scale) in [("color", 1.0), ("blur", 0.5), ("blur", 1.0), ("blur", 3.0)] {
            let mut c = cfg_with(mode, "#171320", scale);
            c.cover.title_lines = vec!["测试标题第一行".into(), "第二行带高亮词".into()];
            c.cover.gradient_dir = "left".into();
            let html = build_cover_html(&out, &c, &resolved());
            let name = format!("{mode}-x{scale}.html");
            std::fs::write(out.join(&name), html).unwrap();
            println!("wrote {}", out.join(&name).display());
        }
        // 表情特效变体（贴主播位置：layout.liver 默认 mid-right = 83.34% 50%）
        let mut c = cfg_with("blur", "#171320", 1.0);
        c.cover.title_lines = vec!["测试标题第一行".into()];
        c.cover.effects = vec![
            crate::project::CoverEffect {
                kind: "emoji".into(),
                amount: 0.5,
                x: 83.3,
                y: 50.0,
                color: "#000000".into(),
                text: "😆".into(),
            },
            crate::project::CoverEffect {
                kind: "emoji".into(),
                amount: 0.2,
                x: 20.0,
                y: 25.0,
                color: "#000000".into(),
                text: "❤️".into(),
            },
        ];
        let html = build_cover_html(&out, &c, &resolved());
        std::fs::write(out.join("emoji.html"), html).unwrap();
        println!("wrote {}", out.join("emoji.html").display());
    }
}