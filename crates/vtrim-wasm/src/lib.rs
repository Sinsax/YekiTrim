//! V-Trim wgpu 渲染核心的 WASM 绑定。
//!
//! JS 侧流程：
//! 1. `init_renderer(canvas, configJson, subsJson, dmsJson, ...)` —— 初始化
//!    WebGPU 设备 + surface + 事件索引（一次）
//! 2. `set_sticker(name, data, isGif)` —— 注入贴纸字节（可多次）
//! 3. 每个 rAF tick：`render_frame(t, videoElement)` —— video 帧拷贝进 GPU，
//!    走与桌面出片完全相同的合成管线后直接 present 到 canvas。
//!
//! 浏览器要求 WebGPU（Chrome/Edge 113+ / Firefox 141+ / Safari 26+）。

use std::collections::HashMap;

use wasm_bindgen::prelude::*;
use web_sys::{HtmlCanvasElement, HtmlVideoElement};

use vtrim_render_core as rcore;

fn js_err(e: impl std::fmt::Display) -> JsValue {
    JsValue::from_str(&e.to_string())
}

/// 将 /api/polish/project 返回的 JSON 解析为渲染核心所需输入。
#[derive(serde::Deserialize)]
struct WasmConfig {
    #[serde(default)]
    segments: Vec<rcore::types::Segment>,
    #[serde(default)]
    layout: rcore::types::Layout,
    #[serde(default)]
    style: rcore::types::StyleRoot,
}

#[derive(serde::Deserialize)]
struct WasmSubtitles {
    #[serde(default)]
    segments: Vec<rcore::types::Subtitle>,
}

#[wasm_bindgen]
pub struct WasmRenderer {
    gpu: rcore::gpu::Gpu,
    pipeline: rcore::render::FramePipeline,
    surface: wgpu::Surface<'static>,
    format: wgpu::TextureFormat,
    width: u32,
    height: u32,
    // 视频帧拷贝目标（COPY_DST | TEXTURE_BINDING | RENDER_ATTACHMENT）
    video_texture: wgpu::Texture,
    video_view: wgpu::TextureView,
    source_texture: wgpu::Texture,
    source_view: wgpu::TextureView,
    effect_texture: wgpu::Texture,
    effect_view: wgpu::TextureView,
    sticker_assets: HashMap<String, rcore::StickerAsset>,
    canvas: HtmlCanvasElement,
}

/// 初始化：WebGPU 设备 + surface + 事件索引。
/// 贴纸以平行数组传入：names[i] ↔ datas[i]（Uint8Array）↔ is_gifs[i]（0/1）。
#[allow(clippy::too_many_arguments)]
#[wasm_bindgen]
pub async fn init_renderer(
    canvas: HtmlCanvasElement,
    config_json: &str,
    subs_json: &str,
    dms_json: &str,
    font_name: String,
    font_bytes: Option<Vec<u8>>,
    sticker_names: Vec<String>,
    sticker_datas: Vec<js_sys::Uint8Array>,
    sticker_is_gifs: Vec<u8>,
    width: u32,
    height: u32,
    vertical: bool,
    vertical_position: String,
) -> Result<WasmRenderer, JsValue> {
        console_error_panic_hook::set_once();

        let mut cfg: serde_json::Value =
            serde_json::from_str(config_json).map_err(|e| js_err(format!("config 解析失败: {e}")))?;
        // 老 `duration` 字段 → `end`（与 polish.toml 同一条升级路径）
        rcore::legacy::upgrade_event_times(&mut cfg);
        let cfg: WasmConfig =
            serde_json::from_value(cfg).map_err(|e| js_err(format!("config 解析失败: {e}")))?;
        let subs: Vec<rcore::types::Subtitle> = serde_json::from_str(subs_json)
            .map_err(|e| js_err(format!("subtitles 解析失败: {e}")))?;
        let dms: Vec<rcore::types::Danmaku> =
            serde_json::from_str(dms_json).map_err(|e| js_err(format!("danmaku 解析失败: {e}")))?;

        // 字体注册（JS 已 fetch 字节）
        if let Some(bytes) = font_bytes {
            rcore::composite::set_font_bytes(&font_name, Some(bytes));
        }

        // 贴纸资源：平行数组（wasm-bindgen 原生传递，无 JSON 字节膨胀）
        let mut sticker_assets: HashMap<String, rcore::StickerAsset> = HashMap::new();
        for (i, name) in sticker_names.iter().enumerate() {
            if name.is_empty() {
                continue;
            }
            if let Some(data) = sticker_datas.get(i) {
                let is_gif = sticker_is_gifs.get(i).copied().unwrap_or(0) != 0;
                if let Some(asset) = rcore::render::decode_sticker_asset(&data.to_vec(), is_gif) {
                    sticker_assets.insert(name.clone(), asset);
                }
            }
        }

        // WebGPU 设备 + surface
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends: wgpu::Backends::BROWSER_WEBGPU,
            flags: wgpu::InstanceFlags::empty(),
            ..Default::default()
        });
        let surface = instance
            .create_surface(wgpu::SurfaceTarget::Canvas(canvas.clone()))
            .map_err(|e| js_err(format!("create_surface 失败: {e}")))?;
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                force_fallback_adapter: false,
                compatible_surface: Some(&surface),
            })
            .await
            .map_err(|_| js_err("WebGPU: no adapter available（浏览器不支持 WebGPU？）"))?;
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("vtrim-wasm device"),
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits::downlevel_defaults(),
                experimental_features: wgpu::ExperimentalFeatures::default(),
                memory_hints: wgpu::MemoryHints::default(),
                trace: wgpu::Trace::Off,
            })
            .await
            .map_err(|e| js_err(format!("request_device 失败: {e}")))?;

        let caps = surface.get_capabilities(&adapter);
        let format = caps.formats[0];
        surface.configure(
            &device,
            &wgpu::SurfaceConfiguration {
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                format,
                width: width.max(1),
                height: height.max(1),
                present_mode: wgpu::PresentMode::Fifo,
                alpha_mode: caps.alpha_modes[0],
                view_formats: vec![],
                desired_maximum_frame_latency: 2,
            },
        );

        let gpu = rcore::gpu::Gpu::from_parts(device, queue, width, height, format)
            .map_err(|e| js_err(format!("Gpu 构建失败: {e:#}")))?;

        // 视频帧拷贝目标 + 中间纹理（固定 1920×1080 合成空间，源帧缩放适配）
        let video_texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("vtrim-wasm video"),
            size: wgpu::Extent3d {
                width: gpu.width,
                height: gpu.height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: rcore::gpu::OUTPUT_FORMAT,
            usage: wgpu::TextureUsages::COPY_DST
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let video_view = video_texture.create_view(&wgpu::TextureViewDescriptor::default());
        let (source_texture, source_view) = gpu.create_output_texture();
        let (effect_texture, effect_view) = gpu.create_output_texture();

        let pipeline = rcore::render::FramePipeline::new(
            &cfg.segments,
            &cfg.layout,
            &cfg.style.subtitle,
            &cfg.style.danmaku,
            &cfg.style.camera,
            subs,
            dms,
            vertical,
            &vertical_position,
            rcore::composite::FontSpec::new(&font_name),
        );

        Ok(WasmRenderer {
            gpu,
            pipeline,
            surface,
            format,
            width,
            height,
            video_texture,
            video_view,
            source_texture,
            source_view,
            effect_texture,
            effect_view,
            sticker_assets,
            canvas,
        })
    }

#[wasm_bindgen]
impl WasmRenderer {
    /// 注入贴纸资源（JS fetch 字节后调用；同名覆盖）。
    pub fn set_sticker(&mut self, name: String, data: Vec<u8>, is_gif: bool) {
        if let Some(asset) = rcore::render::decode_sticker_asset(&data, is_gif) {
            self.sticker_assets.insert(name, asset);
        }
    }

    /// 热更新字幕：精修编辑器里修改字幕时无需重建渲染器，直接替换 pipeline 的字幕数据。
    pub fn set_subtitles(&mut self, subs_json: &str) -> Result<(), JsValue> {
        let parsed: WasmSubtitles =
            serde_json::from_str(subs_json).map_err(|e| js_err(format!("subtitles 解析失败: {e}")))?;
        self.pipeline.subs = parsed.segments;
        Ok(())
    }

    /// 每帧调用：video 当前帧拷贝进 GPU，合成后 present 到 canvas。
    pub fn render_frame(&mut self, t: f64, video: HtmlVideoElement) -> Result<(), JsValue> {
        // video 当前帧 → 1920×1080 纹理（flip_y 对齐纹理坐标）
        self.gpu.queue.copy_external_image_to_texture(
            &wgpu::CopyExternalImageSourceInfo {
                source: wgpu::ExternalImageSource::HTMLVideoElement(video),
                origin: wgpu::Origin2d::ZERO,
                flip_y: true,
            },
            wgpu::CopyExternalImageDestInfo {
                texture: &self.video_texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
                color_space: wgpu::PredefinedColorSpace::Srgb,
                premultiplied_alpha: false,
            },
            wgpu::Extent3d {
                width: self.gpu.width,
                height: self.gpu.height,
                depth_or_array_layers: 1,
            },
        );

        let state = self.pipeline.evaluate(t);
        let frame = self
            .surface
            .get_current_texture()
            .map_err(|e| js_err(format!("get_current_texture 失败: {e}")))?;
        let view = frame.texture.create_view(&wgpu::TextureViewDescriptor::default());

        // 所有 pass 共用一个 encoder，最后只 submit 一次，减少 wasm 实时预览的 GPU 提交开销。
        let mut encoder = self
            .gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        let mut buffers: Vec<wgpu::Buffer> = Vec::new();
        let mut bind_groups: Vec<wgpu::BindGroup> = Vec::new();
        let mut resources = rcore::sprite::SpriteFrameResources::new();

        // source 先离屏渲染，effect 再读 source 合成到 surface；与桌面出片 pass 顺序一致。
        rcore::render::render_source_pass_into(
            &self.gpu,
            &self.video_view,
            &state,
            &self.source_view,
            &mut encoder,
            &mut buffers,
            &mut bind_groups,
        )
        .map_err(|e| js_err(format!("source pass 失败: {e:#}")))?;
        rcore::render::render_effect_pass_into(
            &self.gpu,
            &self.source_texture,
            &state,
            &view,
            &mut encoder,
            &mut buffers,
            &mut bind_groups,
        )
        .map_err(|e| js_err(format!("effect pass 失败: {e:#}")))?;
        // sprite 直接叠加到 surface（视频+特效已在 surface 上）
        rcore::render::render_sprite_pass_into(
            &self.gpu,
            &state,
            t,
            &self.sticker_assets,
            &self.pipeline.sub_style,
            &self.pipeline.dm_style,
            &self.pipeline.font,
            &view,
            &mut encoder,
            &mut resources,
        )
        .map_err(|e| js_err(format!("sprite pass 失败: {e:#}")))?;

        self.gpu.queue.submit(Some(encoder.finish()));
        frame.present();
        Ok(())
    }

    /// 降级：不可用/出错时由 JS 调用以清理。
    pub fn dispose(&mut self) {
        self.sticker_assets.clear();
    }

    /// canvas 引用（JS 侧挂载到 DOM 用）。
    pub fn canvas(&self) -> HtmlCanvasElement {
        self.canvas.clone()
    }
}
