//! CPU composite fallback for wgpu backend.
//!
//! This is a temporary pixel compositor so the wgpu path can render a complete
//! video before all effects/text are moved into wgpu passes.

use std::path::{Path, PathBuf};

use crate::project::SubtitleStyle;
use crate::render::native::timeline::{
    CaptionState, DanmakuState, FrameState, StickerState,
};

pub fn parse_color(s: &str) -> [u8; 4] {
    let s = s.trim();
    if let Some(hex) = s.strip_prefix('#') {
        if hex.len() == 6 {
            let r = u8::from_str_radix(&hex[0..2], 16).unwrap_or(0);
            let g = u8::from_str_radix(&hex[2..4], 16).unwrap_or(0);
            let b = u8::from_str_radix(&hex[4..6], 16).unwrap_or(0);
            return [r, g, b, 255];
        }
        if hex.len() == 3 {
            let r = u8::from_str_radix(&hex[0..1], 16).unwrap_or(0) * 17;
            let g = u8::from_str_radix(&hex[1..2], 16).unwrap_or(0) * 17;
            let b = u8::from_str_radix(&hex[2..3], 16).unwrap_or(0) * 17;
            return [r, g, b, 255];
        }
    }
    match s.to_lowercase().as_str() {
        "black" => [0, 0, 0, 255],
        "white" => [255, 255, 255, 255],
        "red" => [255, 0, 0, 255],
        "green" => [0, 255, 0, 255],
        "blue" => [0, 0, 255, 255],
        _ => [255, 255, 255, 255],
    }
}

fn blend_solid(rgba: &mut [u8], width: u32, height: u32, color: [u8; 4], alpha: f64) {
    if alpha <= 0.0 {
        return;
    }
    let a = (alpha.clamp(0.0, 1.0) * 255.0) as u32;
    let inv = 255 - a;
    for y in 0..height {
        for x in 0..width {
            let i = ((y * width + x) * 4) as usize;
            rgba[i] = ((color[0] as u32 * a + rgba[i] as u32 * inv) / 255) as u8;
            rgba[i + 1] = ((color[1] as u32 * a + rgba[i + 1] as u32 * inv) / 255) as u8;
            rgba[i + 2] = ((color[2] as u32 * a + rgba[i + 2] as u32 * inv) / 255) as u8;
            rgba[i + 3] = 255;
        }
    }
}

fn apply_vignette(rgba: &mut [u8], width: u32, height: u32, opacity: f64) {
    if opacity <= 0.0 {
        return;
    }
    let cx = width as f32 / 2.0;
    let cy = height as f32 / 2.0;
    let max_d = (cx * cx + cy * cy).sqrt();
    for y in 0..height {
        for x in 0..width {
            let dx = x as f32 - cx;
            let dy = y as f32 - cy;
            let d = (dx * dx + dy * dy).sqrt() / max_d;
            let t = ((d - 0.55) / 0.45).clamp(0.0, 1.0);
            let alpha = t as f64 * 0.5 * opacity;
            if alpha <= 0.0 {
                continue;
            }
            let i = ((y * width + x) * 4) as usize;
            let a = (alpha * 255.0) as u32;
            let inv = 255 - a;
            rgba[i] = (rgba[i] as u32 * inv / 255) as u8;
            rgba[i + 1] = (rgba[i + 1] as u32 * inv / 255) as u8;
            rgba[i + 2] = (rgba[i + 2] as u32 * inv / 255) as u8;
        }
    }
}

fn blend_pixel(rgba: &mut [u8], width: u32, x: u32, y: u32, color: [u8; 4], opacity: f64) {
    let i = ((y * width + x) * 4) as usize;
    let a = (opacity.clamp(0.0, 1.0) * 255.0) as u32;
    let inv = 255 - a;
    rgba[i] = ((color[0] as u32 * a + rgba[i] as u32 * inv) / 255) as u8;
    rgba[i + 1] = ((color[1] as u32 * a + rgba[i + 1] as u32 * inv) / 255) as u8;
    rgba[i + 2] = ((color[2] as u32 * a + rgba[i + 2] as u32 * inv) / 255) as u8;
}

/// 字体解析描述：项目路径 + `style.subtitle.font` / `style.danmaku` 字体名。
#[derive(Debug, Clone)]
pub struct FontSpec {
    pub project_path: std::path::PathBuf,
    pub name: String,
}

impl FontSpec {
    pub fn new(project_path: &Path, name: &str) -> Self {
        Self {
            project_path: project_path.to_path_buf(),
            name: name.to_string(),
        }
    }
}

/// 字体字节缓存：按 (项目路径, 字体名) 解析，进程内复用。
static FONTS: std::sync::OnceLock<std::sync::Mutex<std::collections::HashMap<String, Option<Vec<u8>>>>> =
    std::sync::OnceLock::new();

fn fonts() -> &'static std::sync::Mutex<std::collections::HashMap<String, Option<Vec<u8>>>> {
    FONTS.get_or_init(Default::default)
}

/// 解析字体文件字节：优先项目字体目录（assets/fonts、fonts），回退系统字体，
/// 最后回退内置资源 lemi-bobo.ttf。结果按 (项目, 字体名) 缓存。
pub fn load_font_bytes(font: &FontSpec) -> Option<Vec<u8>> {
    let key = format!("{}#{}", font.project_path.display(), font.name);
    if let Some(cached) = fonts().lock().ok().and_then(|m| m.get(&key).cloned()) {
        return cached;
    }
    let name = font.name.trim();
    let mut candidates: Vec<PathBuf> = Vec::new();
    if !name.is_empty() {
        let base = Path::new(&font.name);
        for dir in [
            font.project_path.join("assets").join("fonts"),
            font.project_path.join("fonts"),
            font.project_path.clone(),
        ] {
            candidates.push(dir.join(base).with_extension("ttf"));
            candidates.push(dir.join(base).with_extension("otf"));
        }
        // Windows 系统字体按文件名直查（如 "微软雅黑" → msyh.ttc 由下方兜底覆盖）
        candidates.push(Path::new("C:/Windows/Fonts").join(base).with_extension("ttf"));
        candidates.push(Path::new("C:/Windows/Fonts").join(base).with_extension("otf"));
    }
    // 系统常见中文字体兜底（style 字体名通常匹配不到文件名）
    candidates.push(Path::new("C:/Windows/Fonts/msyh.ttc").to_path_buf());
    candidates.push(Path::new("C:/Windows/Fonts/simhei.ttf").to_path_buf());
    // 内置资源最后回退
    candidates.push(Path::new("crates/vtrim-core/resources/lemi-bobo.ttf").to_path_buf());
    candidates.push(font.project_path.join("assets").join("fonts").join("lemi-bobo.ttf"));

    let found = candidates
        .iter()
        .find(|c| c.exists())
        .and_then(|c| std::fs::read(c).ok());
    fonts().lock().ok().map(|mut m| m.insert(key, found.clone()));
    if found.is_none() {
        tracing::warn!("wgpu: 未找到可用字体文件，字幕/弹幕将无法渲染");
    }
    found
}

/// 量取一行文本在给定字号下的渲染宽度（swash shaping，结果缓存）。
static MEASURE_CACHE: std::sync::OnceLock<std::sync::Mutex<std::collections::HashMap<String, f64>>> =
    std::sync::OnceLock::new();

pub fn measure_text_width(text: &str, size: f32, font: &FontSpec) -> f64 {
    let key = format!(
        "{}#{}#{}",
        text,
        (size * 10.0) as u32,
        font.project_path.display()
    );
    if let Some(cached) = MEASURE_CACHE
        .get_or_init(Default::default)
        .lock()
        .ok()
        .and_then(|m| m.get(&key).copied())
    {
        return cached;
    }
    let width = load_font_bytes(font)
        .and_then(|data| {
            let f = swash::FontRef::from_index(&data, 0)?;
            let mut shape_ctx = swash::shape::ShapeContext::new();
            let mut shaper = shape_ctx
                .builder(f)
                .size(size)
                .script(swash::text::Script::Han)
                .build();
            shaper.add_str(text);
            let mut total = 0.0f32;
            shaper.shape_with(|cluster| total += cluster.advance());
            Some(total as f64)
        })
        .unwrap_or_else(|| (text.chars().count() as f64) * (size as f64));
    MEASURE_CACHE
        .get_or_init(Default::default)
        .lock()
        .ok()
        .map(|mut m| m.insert(key, width));
    width
}

/// 用 swash 把一行文本光栅化为 RGBA 图。
/// 简化实现：逐字符渲染，不做复杂 shaping；中文/英文都能显示。
static RENDER_SOURCES: [swash::scale::Source; 1] = [
    swash::scale::Source::Outline,
];

static TEXT_CACHE: std::sync::OnceLock<std::sync::Mutex<std::collections::HashMap<String, (Vec<u8>, u32, u32, f32)>>> =
    std::sync::OnceLock::new();

fn text_cache_key(font: &FontSpec, text: &str, size: f32, color: [u8; 4], stroke: [u8; 4], stroke_width: f32) -> String {
    format!(
        "{}|{}|{}|{}|{:02x}{:02x}{:02x}{:02x}|{:02x}{:02x}{:02x}{:02x}|{}",
        font.project_path.display(),
        font.name,
        text,
        (size * 10.0) as u32,
        color[0], color[1], color[2], color[3],
        stroke[0], stroke[1], stroke[2], stroke[3],
        (stroke_width * 10.0) as u32,
    )
}

pub fn render_text_rgba(
    text: &str,
    size: f32,
    color: [u8; 4],
    stroke: [u8; 4],
    stroke_width: f32,
    font: &FontSpec,
) -> Option<(Vec<u8>, u32, u32, f32)> {
    let key = text_cache_key(font, text, size, color, stroke, stroke_width);
    if let Some(cached) = TEXT_CACHE.get_or_init(Default::default).lock().ok().and_then(|m| m.get(&key).cloned()) {
        return Some(cached);
    }
    let font_data = load_font_bytes(font)?;
    let font_ref = swash::FontRef::from_index(&font_data, 0)?;
    let mut scale_ctx = swash::scale::ScaleContext::new();
    let mut scaler = scale_ctx.builder(font_ref).size(size).hint(false).build();
    let mut shape_ctx = swash::shape::ShapeContext::new();
    let mut shaper = shape_ctx
        .builder(font_ref)
        .size(size)
        .script(swash::text::Script::Han)
        .build();
    shaper.add_str(text);
    let metrics = shaper.metrics();
    let mut glyphs: Vec<(u16, f32, f32)> = Vec::new();
    let mut pen_x = 0.0f32;
    shaper.shape_with(|cluster| {
        for g in cluster.glyphs {
            glyphs.push((g.id, g.x + pen_x, g.y));
        }
        pen_x += cluster.advance();
    });
    let total_w = pen_x.max(1.0);
    let pad = (stroke_width.max(1.0) + 2.0) as u32;
    let mut width = (total_w.ceil() as u32) + pad * 2;
    let mut height = ((metrics.ascent + metrics.descent).ceil() as u32) + pad * 2;
    let mut out = vec![0u8; (width * height * 4) as usize];

    // 收集所有 glyph 的 outline 并手动光栅化到 alpha buffer
    let mut alpha = vec![0u8; (width * height) as usize];
    let mut scratch = swash::zeno::Scratch::new();
    for (gid, gx, gy) in glyphs {
        if let Some(outline) = scaler.scale_outline(gid) {
            let path = outline.path();
            let (mask_buf, placement) = swash::zeno::Mask::with_scratch(path, &mut scratch)
                .format(swash::zeno::Format::Alpha)
                .origin(swash::zeno::Origin::TopLeft)
                .render();

            // 手动平移到目标位置
            let dst_x = (gx + pad as f32) as i32 + placement.left;
            let dst_y = (gy + pad as f32) as i32 + placement.top;
            for yy in 0..placement.height {
                for xx in 0..placement.width {
                    let a = mask_buf[(yy * placement.width + xx) as usize];
                    if a == 0 {
                        continue;
                    }
                    let dx = dst_x + xx as i32;
                    let dy = dst_y + yy as i32;
                    if dx < 0 || dy < 0 || (dx as u32) >= width || (dy as u32) >= height {
                        continue;
                    }
                    let idx = (dy as u32 * width + dx as u32) as usize;
                    alpha[idx] = alpha[idx].max(a);
                }
            }
        }
    }
    // 垂直翻转 alpha，使最终纹理方向与 sprite shader 一致（修复字幕倒置）
    let mut flipped_alpha = vec![0u8; (width * height) as usize];
    for y in 0..height {
        let src_row = y;
        let dst_row = height - 1 - y;
        for x in 0..width {
            flipped_alpha[(dst_row * width + x) as usize] = alpha[(src_row * width + x) as usize];
        }
    }
    let alpha = flipped_alpha;

    // 先画描边：可分离膨胀（水平+垂直两趟），等价方形邻域但 O(n·sw) 而非 O(n·sw²)
    let sw = stroke_width.max(1.0).round() as i32;
    let mut tmp = vec![0u8; (width * height) as usize];
    // 水平趟：alpha -> tmp
    for y in 0..height {
        let row = (y * width) as usize;
        for x in 0..width {
            let base_a = alpha[row + x as usize] as u32;
            if base_a == 0 {
                continue;
            }
            let x0 = (x as i32 - sw).max(0) as u32;
            let x1 = (x as i32 + sw).min(width as i32 - 1) as u32;
            for xx in x0..=x1 {
                let di = row + xx as usize;
                if tmp[di] == 0 {
                    tmp[di] = 255;
                }
            }
        }
    }
    // 垂直趟：tmp -> alpha（膨胀结果回填 alpha），描边色写入 out
    for x in 0..width {
        let col = x as usize;
        for y in 0..height {
            let base_a = tmp[(y * width) as usize + col] as u32;
            if base_a == 0 {
                continue;
            }
            let y0 = (y as i32 - sw).max(0) as u32;
            let y1 = (y as i32 + sw).min(height as i32 - 1) as u32;
            for yy in y0..=y1 {
                let si = ((yy * width) as usize + col) as usize;
                if alpha[si] == 0 {
                    let idx = si * 4;
                    out[idx] = stroke[0];
                    out[idx + 1] = stroke[1];
                    out[idx + 2] = stroke[2];
                    out[idx + 3] = 255;
                }
            }
        }
    }
    // 再画正文：直接覆盖描边
    for y in 0..height {
        for x in 0..width {
            let a = alpha[(y * width + x) as usize] as u32;
            if a == 0 {
                continue;
            }
            let idx = ((y * width + x) * 4) as usize;
            out[idx] = color[0];
            out[idx+1] = color[1];
            out[idx+2] = color[2];
            out[idx+3] = a as u8;
        }
    }
    // 裁剪到非透明包围盒，去掉多余 padding，使文字位置准确
    let mut min_x = width;
    let mut min_y = height;
    let mut max_x = 0u32;
    let mut max_y = 0u32;
    for y in 0..height {
        for x in 0..width {
            if out[((y * width + x) * 4 + 3) as usize] > 0 {
                min_x = min_x.min(x);
                min_y = min_y.min(y);
                max_x = max_x.max(x);
                max_y = max_y.max(y);
            }
        }
    }
    if max_x >= min_x && max_y >= min_y {
        let cw = max_x - min_x + 1;
        let ch = max_y - min_y + 1;
        let mut cropped = vec![0u8; (cw * ch * 4) as usize];
        for y in 0..ch {
            for x in 0..cw {
                let si = (((min_y + y) * width + (min_x + x)) * 4) as usize;
                let di = ((y * cw + x) * 4) as usize;
                cropped[di..di + 4].copy_from_slice(&out[si..si + 4]);
            }
        }
        out = cropped;
        width = cw;
        height = ch;
    }

    let result = (out, width, height, total_w);
    if let Ok(mut m) = TEXT_CACHE.get_or_init(Default::default).lock() {
        // 上限防长视频撑爆内存：超出后整体清空（重新光栅化即可）
        if m.len() >= 512 {
            m.clear();
        }
        m.insert(key, result.clone());
    }
    Some(result)
}

pub fn composite_frame(
    _rgba: &mut [u8],
    _width: u32,
    _height: u32,
    _state: &FrameState,
    _sub_style: &SubtitleStyle,
    _dm_style: &crate::project::DanmakuStyle,
    _stickers: &[StickerState],
) {
    // 所有特效/贴纸/字幕/弹幕已迁移到 GPU pass，CPU 合成层不再做任何绘制。
}
#[allow(dead_code)]
pub fn dummy(_: &CaptionState, _: &DanmakuState) {}
