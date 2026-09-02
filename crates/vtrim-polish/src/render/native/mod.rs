#![allow(dead_code)]

//! wgpu 原生渲染后端（P2 起逐步实现）。
//!
//! 当前骨架只提供后端工厂入口；最小渲染在后续阶段补齐。

use std::path::Path;

use crate::project::ProjectFile;

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


/// 读取贴纸 GIF 并解码为 RGBA 帧（每帧 Vec<u8>）与 delays。
fn load_sticker_data(ctx: &RenderContext) -> (Vec<Vec<Vec<u8>>>, Vec<Vec<u16>>) {
    let mut frames_all = Vec::new();
    let mut delays_all = Vec::new();
    for seg in &ctx.config.segments {
        for ev in &seg.events {
            if let crate::project::Event::Sticker { name: Some(name), .. } = ev {
                let path = ctx.project_path.join("assets").join("stickers").join(format!("{}.gif", name));
                if !path.exists() { continue; }
                if let Ok(gif) = crate::render::gif::decode_gif_frames(&path) {
                    let mut frames = Vec::new();
                    let mut delays = Vec::new();
                    for f in gif {
                        delays.push(f.delay_ms);
                        if let Ok(img) = image::load_from_memory(&f.png) {
                            let resized = img.resize_exact(280, 280, image::imageops::FilterType::Triangle);
                            frames.push(resized.to_rgba8().into_raw());
                        }
                    }
                    if !frames.is_empty() {
                        frames_all.push(frames);
                        delays_all.push(delays);
                    }
                }
            }
        }
    }
    (frames_all, delays_all)
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
    _project_path: std::path::PathBuf,
    _config: ProjectFile,
    _ctx: Option<RenderContext>,
}

impl WgpuBackend {
    pub fn new(project_path: &Path, config: &ProjectFile) -> anyhow::Result<Self> {
        Ok(Self {
            _project_path: project_path.to_path_buf(),
            _config: config.clone(),
            _ctx: None,
        })
    }
}

impl FrameRenderer for WgpuBackend {
    fn name(&self) -> &str {
        "wgpu"
    }

    fn prepare(&mut self, ctx: &RenderContext) -> anyhow::Result<()> {
        self._ctx = Some(ctx.clone());
        Ok(())
    }

    fn render_chunk(&mut self, frames: &[TimedFrame]) -> anyhow::Result<Vec<Vec<u8>>> {
        let ctx = self
            ._ctx
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("WgpuBackend::prepare must be called first"))?
            .clone();
        if frames.is_empty() {
            return Ok(vec![]);
        }

        // 每个 chunk 只创建一次 wgpu 设备与渲染器，逐帧复用。
        let mut wgpu = context::WgpuContext::new(ctx.width, ctx.height)?;
        let renderer = draw::SourceRenderer::new(&wgpu.device)?;
        let effect_renderer = effects::EffectRenderer::new(&wgpu.device)?;
        let mut sprite_renderer = sprite::SpriteRenderer::new(&wgpu.device)?;
        let (sticker_frames, sticker_delays) = load_sticker_data(&ctx);
        let (subs, dms) = load_text_data(&ctx);
        let sub_color = composite::parse_color(&ctx.config.style.subtitle.color);
        let hl_color = composite::parse_color(&ctx.config.style.subtitle.highlight_color);
        let stroke_color = composite::parse_color(
            ctx.config.style.subtitle.stroke.split_whitespace().last().unwrap_or("#403c3b"),
        );
        let stroke_width = ctx.config.style.subtitle.stroke.split_whitespace().next()
            .and_then(|s| s.trim_end_matches("px").parse::<f32>().ok())
            .unwrap_or(12.0);
        let font_size = ctx.config.style.subtitle.size as f32;
        let idx = timeline::build_index(&ctx.config);
        let total_frames = frames.len();
        let mut out_rgba_frames = Vec::with_capacity(total_frames);
        let chunk_start = std::time::Instant::now();

        for (fi, tf) in frames.iter().enumerate() {
            let rgba = tf
                .frame
                .rgba
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("wgpu backend requires decoded RGBA frames"))?;
            if tf.frame.width == 0 || tf.frame.height == 0 {
                anyhow::bail!("wgpu backend requires valid frame dimensions");
            }

            let state = timeline::evaluate_frame(
                tf.t,
                &idx,
                &ctx.config.layout,
                &ctx.config.style.subtitle,
                &ctx.config.style.danmaku,
                &ctx.config.style.camera,
                &subs,
                &dms,
            );

            let (source_texture, source_view) = wgpu.create_output_texture();
            let mut camera = timeline::get_active_camera(tf.t, &idx.camera, &ctx.config.layout, &ctx.config.style.camera);
            // 叠加 zoom_bounce / pulse / stutter / split 到相机状态
            camera.zoom *= state.zoom_bounce.max(0.0);
            if state.pulse > 0.0 {
                camera.zoom = state.pulse;
            }
            let mut shx = state.shake.x;
            let mut shy = state.shake.y;
            if let Some((sx, sy)) = state.stutter {
                shx += sx;
                shy += sy;
            }
            if let Some(sp) = &state.split {
                shx += sp.x;
                // split 的 scale/skew 暂以 zoom 轻微变化近似
                camera.zoom *= sp.scale;
            }
            renderer.render(
                &wgpu,
                rgba,
                tf.frame.width,
                tf.frame.height,
                &camera,
                (shx, shy),
                &source_view,
            )?;

            // GPU 特效 pass：flash/overlay/intro/vignette/noise
            let (output_texture, output_view) = wgpu.create_output_texture();
            effect_renderer.apply(&wgpu, &state, &source_texture, &output_view)?;

            sprite_renderer.draw_stickers(
                &wgpu,
                &state.stickers,
                &sticker_frames,
                &sticker_delays,
                tf.t,
                &output_view,
            )?;

            sprite_renderer.draw_captions(
                &wgpu,
                &state.captions,
                sub_color,
                hl_color,
                stroke_color,
                stroke_width,
                font_size,
                &output_view,
            )?;
            sprite_renderer.draw_danmaku(
                &wgpu,
                &state.danmaku,
                &output_view,
            )?;

            let mut out_rgba = wgpu.read_output_rgba(&output_texture)?;
            composite::composite_frame(
                &mut out_rgba,
                ctx.width,
                ctx.height,
                &state,
                &ctx.config.style.subtitle,
                &ctx.config.style.danmaku,
                &state.stickers,
            );

            out_rgba_frames.push(out_rgba);

            if fi == 0 || (fi + 1) % 10 == 0 || fi + 1 == total_frames {
                let pct = (fi + 1) as f64 / total_frames as f64 * 100.0;
                let elapsed = chunk_start.elapsed().as_secs_f64();
                tracing::info!(
                    "    wgpu chunk progress: {}/{} frames ({:.0}%)  {:.1}s",
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
