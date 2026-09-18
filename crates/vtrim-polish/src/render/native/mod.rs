#![allow(dead_code)]

//! wgpu 原生渲染后端（P2 起逐步实现）。
//!
//! 当前骨架只提供后端工厂入口；最小渲染在后续阶段补齐。

use std::path::{Path, PathBuf};

use crate::project::{Event, ProjectFile};

use super::backend::FrameRenderer;
use super::context::RenderContext;
use super::frame::TimedFrame;
use vtrim_render_core as rcore;



/// wgpu 渲染后端。
///
/// 尚未实现 `render_chunk` 时，会通过默认 trait 方法落到 `render_frame` 并返回明确错误。


pub use vtrim_render_core::StickerAsset;

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
                    out.push(rcore::draw::encode_jpeg(&rgba, width, height));
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
    ctx: Option<RenderContext>,
    pipeline: Option<rcore::render::FramePipeline>,
    gpu: Option<rcore::gpu::Gpu>,
}

impl WgpuBackend {
    pub fn new(project_path: &Path, config: &ProjectFile) -> anyhow::Result<Self> {
        Ok(Self {
            ctx: None,
            pipeline: None,
            gpu: None,
        })
    }

    /// 单帧渲染（预览用）：传入 t 时刻的解码帧，返回合成后的 JPEG。
    pub fn render_single(
        &mut self,
        t: f64,
        rgba: &[u8],
        width: u32,
        height: u32,
    ) -> anyhow::Result<Vec<u8>> {
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

    fn ensure_gpu(&mut self) -> anyhow::Result<&mut rcore::gpu::Gpu> {
        if self.gpu.is_none() {
            let (device, queue) = rcore::gpu::create_offscreen_device()?;
            let format = wgpu::TextureFormat::Rgba8Unorm;
            self.gpu = Some(rcore::gpu::Gpu::from_parts(device, queue, self.ctx_width(), self.ctx_height(), format)?);
        }
        Ok(self.gpu.as_mut().unwrap())
    }

    fn ctx_width(&self) -> u32 {
        self.ctx.as_ref().map(|c| c.width).unwrap_or(1920)
    }

    fn ctx_height(&self) -> u32 {
        self.ctx.as_ref().map(|c| c.height).unwrap_or(1080)
    }

    fn prepare_pipeline(&mut self) -> anyhow::Result<()> {
        let ctx = self
            .ctx
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("WgpuBackend::prepare must be called first"))?
            .clone();
        // 字节解析（桌面候选链） -> 注册表
        let font = rcore::composite::FontSpec::new(&ctx.config.style.subtitle.font);
        if rcore::composite::load_font_bytes(&font).is_none() {
            let bytes = load_font_file(&ctx.project_path, &ctx.config.style.subtitle.font);
            rcore::composite::set_font_bytes(&ctx.config.style.subtitle.font, bytes);
        }
        let (polish_subs, polish_dms) = load_text_data(&ctx);
        let subs = polish_subs
            .into_iter()
            .map(|s| rcore::types::Subtitle { start: s.start, end: s.end, text: s.text, html: s.html })
            .collect();
        let dms = polish_dms
            .into_iter()
            .map(|d| rcore::types::Danmaku {
                text: d.text,
                start: d.start,
                end: d.end,
                y: d.y,
                travel_duration: d.travel_duration,
            })
            .collect();
        let sticker_assets = load_sticker_assets(&ctx);
        self.pipeline = Some(rcore::render::FramePipeline::new(
            &ctx.config.segments,
            &ctx.config.layout,
            &ctx.config.style.subtitle,
            &ctx.config.style.danmaku,
            &ctx.config.style.camera,
            subs,
            dms,
            ctx.vertical,
            &ctx.vertical_position,
            font,
        ));
        Ok(())
    }
}

impl FrameRenderer for WgpuBackend {
    fn name(&self) -> &str {
        "wgpu"
    }

    fn prepare(&mut self, ctx: &RenderContext) -> anyhow::Result<()> {
        self.ctx = Some(ctx.clone());
        self.pipeline = None;
        Ok(())
    }

    fn render_chunk(&mut self, frames: &[TimedFrame]) -> anyhow::Result<Vec<Vec<u8>>> {
        if frames.is_empty() {
            return Ok(vec![]);
        }
        if self.pipeline.is_none() {
            self.prepare_pipeline()?;
        }
        self.ensure_gpu()?;
        let pipeline = self.pipeline.as_ref().unwrap();
        let gpu = self.gpu.as_ref().unwrap();
        let sticker_assets = load_sticker_assets(&self.ctx.as_ref().unwrap());

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
            let state = pipeline.evaluate(tf.t);
            eval_ms += te.elapsed().as_millis();

            let targets = gpu.create_frame_targets();
            // 视频帧 RGBA 上传为纹理（wasm 路径换 video 拷贝纹理）
            let source_view = gpu
                .create_rgba_texture(tf.frame.width, tf.frame.height, rgba)
                .create_view(&wgpu::TextureViewDescriptor::default());
            rcore::render::render_source_pass(gpu, &source_view, &state, &targets.source_view)?;
            rcore::render::render_effect_pass(gpu, &targets.source_texture, &state, &targets.effect_view)?;
            rcore::render::render_sprite_pass(
                gpu,
                &state,
                tf.t,
                &sticker_assets,
                &pipeline.sub_style,
                &pipeline.dm_style,
                &pipeline.font,
                &targets.effect_view,
            )?;

            let t_read = std::time::Instant::now();
            let rgba_out = gpu.read_output_rgba(&targets.effect_texture)?;
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
        let encoded = encode_all_parallel(out_rgba_frames, self.ctx_width(), self.ctx_height());
        let mut out = Vec::with_capacity(encoded.len());
        for item in encoded {
            out.push(item?);
        }
        Ok(out)
    }
}

/// 桌面字体候选链：项目字体目录 -> 程序级字体库 -> 系统字体。
fn load_font_file(project_path: &Path, name: &str) -> Option<Vec<u8>> {
    let mut candidates: Vec<std::path::PathBuf> = Vec::new();
    if !name.trim().is_empty() {
        let base = std::path::Path::new(name);
        for dir in [
            project_path.join("assets").join("fonts"),
            project_path.join("fonts"),
            project_path.to_path_buf(),
        ] {
            candidates.push(dir.join(base).with_extension("ttf"));
            candidates.push(dir.join(base).with_extension("otf"));
            candidates.push(dir.join(base).with_extension("ttc"));
        }
        // 程序级字体库（`<程序目录>/resources/fonts/`）：`name` 就是字体 id。
        // 放在项目候选之后、系统字体之前 —— "别处都没有"时才用库里的。
        if let Some(p) = vtrim_core::fonts::resolve_file(name.trim()) {
            candidates.push(p);
        }
        candidates.push(std::path::Path::new("C:/Windows/Fonts").join(base).with_extension("ttf"));
    }
    candidates.push(std::path::Path::new("C:/Windows/Fonts/msyh.ttc").to_path_buf());
    candidates.push(std::path::Path::new("C:/Windows/Fonts/simhei.ttf").to_path_buf());
    // 不再内置字体文件（见 README「字体」）：系统字体是最后一档兜底。
    // 想用乐米波波体请让用户自行安装 —— 它的授权「免费商用但不含嵌入式」，不能随包分发。
    candidates
        .iter()
        .find(|c| c.exists())
        .and_then(|c| std::fs::read(c).ok())
}
