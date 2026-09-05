#![allow(dead_code)]

//! wgpu 原生渲染后端（P2 起逐步实现）。
//!
//! 当前骨架只提供后端工厂入口；最小渲染在后续阶段补齐。

use std::path::{Path, PathBuf};

use crate::project::{Event, ProjectFile};

use super::backend::FrameRenderer;
use super::context::RenderContext;
use super::frame::TimedFrame;

pub mod composite;
pub mod context;
pub mod draw;
pub mod easing;
pub mod effects;
pub mod shaders;

pub mod sprite;
pub mod timeline;

/// wgpu 渲染后端。
///
/// 尚未实现 `render_chunk` 时，会通过默认 trait 方法落到 `render_frame` 并返回明确错误。


/// 贴纸资源：多帧 GIF（每帧 RGBA + delay）或单帧 PNG。
pub struct StickerAsset {
    pub frames: Vec<Vec<u8>>,
    pub delays: Vec<u16>,
}

/// 贴纸解码结果全局缓存：key = (文件路径, mtime)。预览重复请求与多 chunk 并发共享。
static STICKER_DECODE_CACHE: std::sync::OnceLock<
    std::sync::Mutex<std::collections::HashMap<(PathBuf, std::time::SystemTime), StickerAsset>>,
> = std::sync::OnceLock::new();

fn file_mtime(path: &Path) -> Option<std::time::SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

fn decode_gif_cached(path: &Path) -> Option<StickerAsset> {
    let mtime = file_mtime(path)?;
    let key = (path.to_path_buf(), mtime);
    {
        let cache = STICKER_DECODE_CACHE.get_or_init(Default::default);
        if let Ok(m) = cache.lock() {
            if let Some(v) = m.get(&key) {
                return Some(StickerAsset { frames: v.frames.clone(), delays: v.delays.clone() });
            }
        }
    }
    let gif = crate::render::gif::decode_gif_frames_rgba(path, Some((280, 280))).ok()?;
    let frames: Vec<Vec<u8>> = gif.frames.iter().map(|(px, _)| px.clone()).collect();
    let delays: Vec<u16> = gif.frames.iter().map(|(_, d)| *d).collect();
    if frames.is_empty() {
        return None;
    }
    let asset = StickerAsset { frames, delays };
    if let Ok(mut m) = STICKER_DECODE_CACHE.get_or_init(Default::default).lock() {
        m.insert(key, StickerAsset { frames: asset.frames.clone(), delays: asset.delays.clone() });
    }
    Some(asset)
}

fn decode_static_cached(path: &Path) -> Option<StickerAsset> {
    let mtime = file_mtime(path)?;
    let key = (path.to_path_buf(), mtime);
    {
        let cache = STICKER_DECODE_CACHE.get_or_init(Default::default);
        if let Ok(m) = cache.lock() {
            if let Some(v) = m.get(&key) {
                return Some(StickerAsset { frames: v.frames.clone(), delays: v.delays.clone() });
            }
        }
    }
    let img = image::open(path).ok()?;
    let resized = img.resize_exact(280, 280, image::imageops::FilterType::Nearest);
    let asset = StickerAsset {
        frames: vec![resized.to_rgba8().into_raw()],
        delays: vec![100],
    };
    if let Ok(mut m) = STICKER_DECODE_CACHE.get_or_init(Default::default).lock() {
        m.insert(key, StickerAsset { frames: asset.frames.clone(), delays: asset.delays.clone() });
    }
    Some(asset)
}

/// 收集时间轴上实际用到的贴纸资源，按名字索引。
/// 支持 GIF（多帧动画）与 PNG（静态单帧）；emoji 码点贴纸若项目 assets/stickers
/// 已下载对应文件则同样可用，否则跳过（离线路径不回退远程 CDN）。
fn load_sticker_assets(ctx: &RenderContext) -> std::collections::HashMap<String, StickerAsset> {
    use std::collections::HashMap;
    let mut out: HashMap<String, StickerAsset> = HashMap::new();
    let dir = ctx.project_path.join("assets").join("stickers");
    for seg in &ctx.config.segments {
        for ev in &seg.events {
            let Event::Sticker {
                name: Some(name), ..
            } = ev
            else {
                continue;
            };
            if name.is_empty() || out.contains_key(name) {
                continue;
            }
            let gif_path = dir.join(format!("{name}.gif"));
            let png_path = dir.join(format!("{name}.png"));
            if gif_path.exists() {
                if let Some(asset) = decode_gif_cached(&gif_path) {
                    out.insert(name.clone(), asset);
                }
            } else if png_path.exists() {
                if let Some(asset) = decode_static_cached(&png_path) {
                    out.insert(name.clone(), asset);
                }
            } else if u32::from_str_radix(name, 16).is_ok() {
                tracing::debug!("wgpu: emoji 贴纸 {name} 无本地资源，跳过（离线不回退远程 CDN）");
            } else {
                tracing::debug!("wgpu: 贴纸 {name} 无本地资源（gif/png 均不存在）");
            }
        }
    }
    out
}

/// 获取贴纸事件对应的 RGBA 帧数组索引。
fn sticker_frame_index(event_start: f64, t: f64, delays: &[u16]) -> usize {
    if delays.is_empty() { return 0; }
    let total: u32 = delays.iter().map(|d| *d as u32).sum();
    if total == 0 { return 0; }
    let elapsed_ms = ((t - event_start).max(0.0) * 1000.0) as u32;
    let loop_pos = elapsed_ms % total;
    let mut acc = 0u32;
    for (i, d) in delays.iter().enumerate() {
        acc += *d as u32;
        if loop_pos < acc { return i; }
    }
    0
}


/// 并行 JPEG 编码：把 RGBA 帧转 JPEG。
fn encode_all_parallel(frames: Vec<Vec<u8>>, width: u32, height: u32) -> Vec<anyhow::Result<Vec<u8>>> {
    let n = frames.len();
    if n == 0 { return vec![]; }
    let workers = std::thread::available_parallelism().map(|x| x.get()).unwrap_or(4).min(8).max(1);
    let chunk_size = (n + workers - 1) / workers;
    let mut results: Vec<Option<anyhow::Result<Vec<u8>>>> = (0..n).map(|_| None).collect();
    std::thread::scope(|scope| {
        let mut handles = Vec::new();
        for c in 0..workers {
            let start = c * chunk_size;
            let end = ((c + 1) * chunk_size).min(n);
            if start >= end { continue; }
            let mut local = Vec::new();
            for i in start..end {
                local.push(frames[i].clone());
            }
            handles.push(scope.spawn(move || {
                let mut out = Vec::new();
                for rgba in local {
                    out.push(draw::encode_jpeg(&rgba, width, height));
                }
                (start, out)
            }));
        }
        for h in handles {
            let (start, out) = h.join().unwrap();
            for (i, item) in out.into_iter().enumerate() {
                results[start + i] = Some(item);
            }
        }
    });
    results.into_iter().map(|x| x.unwrap()).collect()
}

/// 从项目 data 目录读取字幕与弹幕 JSON。
fn load_text_data(
    ctx: &RenderContext,
) -> (
    Vec<crate::parse::srt::Subtitle>,
    Vec<vtrim_core::danmaku::DanmakuEntry>,
) {
    let subs: Vec<crate::parse::srt::Subtitle> =
        std::fs::read_to_string(ctx.project_path.join("data").join("subtitles.json"))
            .ok()
            .and_then(|raw| serde_json::from_str(&raw).ok())
            .unwrap_or_default();
    let dms: Vec<vtrim_core::danmaku::DanmakuEntry> =
        std::fs::read_to_string(ctx.project_path.join("data").join("danmaku.json"))
            .ok()
            .and_then(|raw| serde_json::from_str(&raw).ok())
            .unwrap_or_default();
    (subs, dms)
}

pub struct WgpuBackend {
    project_path: std::path::PathBuf,
    config: ProjectFile,
    ctx: Option<RenderContext>,
    /// 常驻 wgpu 上下文与渲染器：prepare 后创建，render_chunk 逐帧复用。
    gpu: Option<Gpu>,
}

struct Gpu {
    wgpu: context::WgpuContext,
    source: draw::SourceRenderer,
    effects: effects::EffectRenderer,
    sprites: sprite::SpriteRenderer,
}

impl WgpuBackend {
    pub fn new(project_path: &Path, config: &ProjectFile) -> anyhow::Result<Self> {
        Ok(Self {
            project_path: project_path.to_path_buf(),
            config: config.clone(),
            ctx: None,
            gpu: None,
        })
    }

    /// 单帧渲染（预览用）：传入 t 时刻的解码帧，返回合成后的 JPEG。
    pub fn render_single(&mut self, t: f64, rgba: &[u8], width: u32, height: u32) -> anyhow::Result<Vec<u8>> {
        let frames = [TimedFrame {
            index: 0,
            t,
            frame: super::frame::RenderFrame {
                width,
                height,
                jpeg: None,
                rgba: Some(rgba.to_vec()),
            },
        }];
        Ok(self.render_chunk(&frames)?.remove(0))
    }

    fn ensure_gpu(&mut self) -> anyhow::Result<&mut Gpu> {
        if self.gpu.is_none() {
            let ctx = self
                .ctx
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("WgpuBackend::prepare must be called first"))?
                .clone();
            let wgpu = context::WgpuContext::new(ctx.width, ctx.height)?;
            let source = draw::SourceRenderer::new(&wgpu.device)?;
            let effects = effects::EffectRenderer::new(&wgpu.device)?;
            let sprites = sprite::SpriteRenderer::new(&wgpu.device)?;
            self.gpu = Some(Gpu { wgpu, source, effects, sprites });
        }
        Ok(self.gpu.as_mut().unwrap())
    }
}

impl FrameRenderer for WgpuBackend {
    fn name(&self) -> &str {
        "wgpu"
    }

    fn prepare(&mut self, ctx: &RenderContext) -> anyhow::Result<()> {
        self.ctx = Some(ctx.clone());
        Ok(())
    }

    fn render_chunk(&mut self, frames: &[TimedFrame]) -> anyhow::Result<Vec<Vec<u8>>> {
        if frames.is_empty() {
            return Ok(vec![]);
        }
        let ctx = self
            .ctx
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("WgpuBackend::prepare must be called first"))?
            .clone();
        let t0 = std::time::Instant::now();
        let Gpu { wgpu, source: renderer, effects: effect_renderer, sprites: sprite_renderer } =
            self.ensure_gpu()?;
        let t1 = std::time::Instant::now();
        let sticker_assets = load_sticker_assets(&ctx);
        let t2 = std::time::Instant::now();
        let (subs, dms) = load_text_data(&ctx);
        let sub_style = ctx.config.style.subtitle.clone();
        let dm_style = ctx.config.style.danmaku.clone();
        let font = composite::FontSpec::new(&ctx.project_path, &sub_style.font);
        // 弹幕滚动距离依赖实测文字宽度（对齐 JS measureText）
        let dm_font_size = dm_style.font_size as f32;
        let dm_widths: Vec<f64> = dms
            .iter()
            .map(|d| composite::measure_text_width(&d.text, dm_font_size, &font))
            .collect();
        let idx = timeline::build_index(&ctx.config);
        let t3 = std::time::Instant::now();
        tracing::info!(
            "wgpu chunk timing: gpu_init={:?} stickers={:?} text={}ms widths={}ms index={}ms",
            t1.duration_since(t0),
            t2.duration_since(t1),
            t3.duration_since(t2).as_millis(),
            0,
            0
        );
        let total_frames = frames.len();
        let mut out_rgba_frames = Vec::with_capacity(total_frames);
        let chunk_start = std::time::Instant::now();
        let mut eval_ms = 0u128;
        let mut read_ms = 0u128;

        for (fi, tf) in frames.iter().enumerate() {
            let rgba = tf
                .frame
                .rgba
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("wgpu backend requires decoded RGBA frames"))?;
            if tf.frame.width == 0 || tf.frame.height == 0 {
                anyhow::bail!("wgpu backend requires valid frame dimensions");
            }

            let te = std::time::Instant::now();
            let state = timeline::evaluate_frame(
                tf.t,
                &idx,
                &ctx.config.layout,
                &sub_style,
                &dm_style,
                &ctx.config.style.camera,
                &subs,
                &dms,
                &dm_widths,
                ctx.vertical,
                &ctx.vertical_position,
            );
            eval_ms += te.elapsed().as_millis();

            let (source_texture, source_view) = wgpu.create_output_texture();
            let mut camera = timeline::get_active_camera(tf.t, &idx.camera, &ctx.config.layout, &ctx.config.style.camera);
            // zoom_bounce / pulse 已含基准 zoom，最终缩放 = max（对齐 JS finalZoom = max(zb, pl)）
            camera.zoom = state.zoom_bounce.max(state.pulse);
            let mut shx = state.shake.x;
            let mut shy = state.shake.y;
            if let Some((sx, sy)) = state.stutter {
                shx += sx;
                shy += sy;
            }
            if let Some(sp) = &state.split {
                shx += sp.x;
                // split.scale 以乘法叠加（Canvas ctx.scale 语义），skew 由相机矩阵处理
                camera.zoom *= sp.scale;
            }
            // blur / color_shift 作用于视频底图（对齐 Canvas ctx.filter 时机）
            let (sepia, saturate) = parse_color_shift(&state.color_shift);
            let fx = draw::SourceFx {
                blur: state.blur,
                sepia,
                saturate,
            };
            renderer.render(
                &wgpu,
                rgba,
                tf.frame.width,
                tf.frame.height,
                &camera,
                (shx, shy),
                state.split.as_ref(),
                fx,
                &source_view,
            )?;

            // GPU 特效 pass：noise/flash/hue/overlay/intro/vignette
            let (output_texture, output_view) = wgpu.create_output_texture();
            effect_renderer.apply(&wgpu, &state, &source_texture, &output_view)?;

            sprite_renderer.draw_stickers(
                &wgpu,
                &state.stickers,
                &sticker_assets,
                tf.t,
                &output_view,
            )?;

            sprite_renderer.draw_captions(
                &wgpu,
                &state.captions,
                &sub_style,
                &font,
                &output_view,
            )?;
            sprite_renderer.draw_danmaku(
                &wgpu,
                &state.danmaku,
                &dm_style,
                &font,
                &output_view,
            )?;

            let t_read = std::time::Instant::now();
            let rgba_out = wgpu.read_output_rgba(&output_texture)?;
            read_ms += t_read.elapsed().as_millis();
            out_rgba_frames.push(rgba_out);

            if fi == 0 || (fi + 1) % 10 == 0 || fi + 1 == total_frames {
                let pct = (fi + 1) as f64 / total_frames as f64 * 100.0;
                let elapsed = chunk_start.elapsed().as_secs_f64();
                tracing::info!(
                    "    wgpu chunk progress: {}/{} frames ({:.0}%)  {:.1}s | eval={eval_ms}ms read={read_ms}ms",
                    fi + 1,
                    total_frames,
                    pct,
                    elapsed,
                );
            }
        }
        // 并行 JPEG 编码加速
        let encoded = encode_all_parallel(out_rgba_frames, ctx.width, ctx.height);
        let mut out = Vec::with_capacity(encoded.len());
        for item in encoded {
            out.push(item?);
        }
        Ok(out)
    }
}

/// 解析 JS 风格的 color_shift 字符串 "sepia(x) saturate(y)" 为衰减系数。
fn parse_color_shift(s: &str) -> (f64, f64) {
    let mut sepia = 0.0;
    let mut saturate = 1.0;
    for part in s.split_whitespace() {
        let (name, val) = match part.split_once('(') {
            Some((n, v)) => (n, v.trim_end_matches(')').trim()),
            None => continue,
        };
        if name == "sepia" {
            sepia = val.parse().unwrap_or(0.0);
        } else if name == "saturate" {
            saturate = val.parse().unwrap_or(1.0);
        }
    }
    (sepia, saturate)
}
