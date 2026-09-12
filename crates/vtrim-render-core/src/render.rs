//! 逐帧渲染编排：共享给桌面 native 渲染与 wasm 实时预览。
//!
//! pass 结构：
//! 1. source pass —— 视频底图（相机 zoom/shake/split + blur/color_shift）→ source_view
//! 2. effect pass —— 噪声/闪白/hue/渐变 overlay/暗角/开场黑幕 → effect_view
//! 3. sprite pass —— 贴纸/字幕/弹幕 → effect_view（桌面）或 surface view（wasm 上屏）

use anyhow::Result;
use std::collections::HashMap;

use super::composite::{self, FontSpec};
use super::gif;
use super::draw;
use super::gpu::{FrameTargets, Gpu};
use super::sprite::SpriteFrameResources;
use crate::StickerAsset;
use super::timeline::{self, FrameState};
use crate::types::{CameraStyle, DanmakuStyle, Layout, SubtitleStyle};

/// 一次渲染会话的预处理状态（事件索引 + 样式 + 文本宽度），逐帧复用。
pub struct FramePipeline {
    pub idx: timeline::EventIndex,
    pub layout: Layout,
    pub sub_style: SubtitleStyle,
    pub dm_style: DanmakuStyle,
    pub camera_style: CameraStyle,
    pub subs: Vec<crate::types::Subtitle>,
    pub dms: Vec<crate::types::Danmaku>,
    pub dm_widths: Vec<f64>,
    pub vertical: bool,
    pub vertical_position: String,
    pub font: composite::FontSpec,
}

impl FramePipeline {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        segments: &[crate::types::Segment],
        layout: &Layout,
        sub_style: &SubtitleStyle,
        dm_style: &DanmakuStyle,
        camera_style: &CameraStyle,
        subs: Vec<crate::types::Subtitle>,
        dms: Vec<crate::types::Danmaku>,
        vertical: bool,
        vertical_position: &str,
        font: composite::FontSpec,
    ) -> Self {
        let dm_font_size = dm_style.font_size as f32;
        let dm_widths: Vec<f64> = dms
            .iter()
            .map(|d| composite::measure_text_width(&d.text, dm_font_size, &font))
            .collect();
        Self {
            idx: timeline::build_index(segments),
            layout: layout.clone(),
            sub_style: sub_style.clone(),
            dm_style: dm_style.clone(),
            camera_style: camera_style.clone(),
            subs,
            dms,
            dm_widths,
            vertical,
            vertical_position: vertical_position.to_string(),
            font,
        }
    }

    /// 求值 t 时刻的完整视觉状态。
    pub fn evaluate(&self, t: f64) -> FrameState {
        timeline::evaluate_frame(
            t,
            &self.idx,
            &self.layout,
            &self.sub_style,
            &self.dm_style,
            &self.camera_style,
            &self.subs,
            &self.dms,
            &self.dm_widths,
            self.vertical,
            &self.vertical_position,
        )
    }
}

/// 视频底图 pass：相机 zoom/shake/split + blur/color_shift，写入 targets.source_view。
/// `source_view` 由调用方准备（桌面：rgba 上传纹理；wasm：video 元素拷贝纹理）。
pub fn render_source_pass(
    gpu: &Gpu,
    source_view: &wgpu::TextureView,
    state: &FrameState,
    output_view: &wgpu::TextureView,
) -> Result<()> {
    let mut encoder = gpu
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    let mut buffers = Vec::new();
    let mut bind_groups = Vec::new();
    render_source_pass_into(
        gpu,
        source_view,
        state,
        output_view,
        &mut encoder,
        &mut buffers,
        &mut bind_groups,
    )?;
    gpu.queue.submit(Some(encoder.finish()));
    Ok(())
}

/// 批量版：把 source pass 记录进已有 encoder。
pub fn render_source_pass_into(
    gpu: &Gpu,
    source_view: &wgpu::TextureView,
    state: &FrameState,
    output_view: &wgpu::TextureView,
    encoder: &mut wgpu::CommandEncoder,
    buffers: &mut Vec<wgpu::Buffer>,
    bind_groups: &mut Vec<wgpu::BindGroup>,
) -> Result<()> {
    let (sepia, saturate) = parse_color_shift(&state.color_shift);
    gpu.source.render_into(
        gpu,
        source_view,
        &state.camera,
        (state.shake.x, state.shake.y),
        state.split.as_ref(),
        draw::SourceFx {
            blur: state.blur,
            sepia,
            saturate,
        },
        output_view,
        encoder,
        buffers,
        bind_groups,
    )
}

/// 特效 pass：读 source 纹理，写 output_view。
pub fn render_effect_pass(
    gpu: &Gpu,
    source_texture: &wgpu::Texture,
    state: &FrameState,
    output_view: &wgpu::TextureView,
) -> Result<()> {
    let mut encoder = gpu
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    let mut buffers = Vec::new();
    let mut bind_groups = Vec::new();
    render_effect_pass_into(
        gpu,
        source_texture,
        state,
        output_view,
        &mut encoder,
        &mut buffers,
        &mut bind_groups,
    )?;
    gpu.queue.submit(Some(encoder.finish()));
    Ok(())
}

/// 批量版：把 effect pass 记录进已有 encoder。
pub fn render_effect_pass_into(
    gpu: &Gpu,
    source_texture: &wgpu::Texture,
    state: &FrameState,
    output_view: &wgpu::TextureView,
    encoder: &mut wgpu::CommandEncoder,
    buffers: &mut Vec<wgpu::Buffer>,
    bind_groups: &mut Vec<wgpu::BindGroup>,
) -> Result<()> {
    gpu.effects
        .apply_into(gpu, state, source_texture, output_view, encoder, buffers, bind_groups)
}

/// 贴纸 + 字幕 + 弹幕 pass（叠加于当前画面之上）。
#[allow(clippy::too_many_arguments)]
pub fn render_sprite_pass(
    gpu: &Gpu,
    state: &FrameState,
    t: f64,
    sticker_assets: &HashMap<String, StickerAsset>,
    sub_style: &SubtitleStyle,
    dm_style: &DanmakuStyle,
    font: &composite::FontSpec,
    output_view: &wgpu::TextureView,
) -> Result<()> {
    let mut encoder = gpu
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    let mut resources = SpriteFrameResources::new();
    render_sprite_pass_into(
        gpu,
        state,
        t,
        sticker_assets,
        sub_style,
        dm_style,
        font,
        output_view,
        &mut encoder,
        &mut resources,
    )?;
    gpu.queue.submit(Some(encoder.finish()));
    Ok(())
}

/// 批量版：把贴纸/字幕/弹幕全部记录进同一个 encoder。
#[allow(clippy::too_many_arguments)]
pub fn render_sprite_pass_into(
    gpu: &Gpu,
    state: &FrameState,
    t: f64,
    sticker_assets: &HashMap<String, StickerAsset>,
    sub_style: &SubtitleStyle,
    dm_style: &DanmakuStyle,
    font: &composite::FontSpec,
    output_view: &wgpu::TextureView,
    encoder: &mut wgpu::CommandEncoder,
    resources: &mut SpriteFrameResources,
) -> Result<()> {
    gpu.sprites
        .draw_stickers_into(
            gpu,
            &state.stickers,
            sticker_assets,
            t,
            output_view,
            encoder,
            resources,
        )?;
    gpu.sprites
        .draw_captions_into(
            gpu,
            &state.captions,
            sub_style,
            font,
            output_view,
            encoder,
            resources,
        )?;
    gpu.sprites
        .draw_danmaku_into(
            gpu,
            &state.danmaku,
            dm_style,
            font,
            output_view,
            encoder,
            resources,
        )
}

/// 从内存字节解码贴纸资源（wasm：JS fetch 字节后传入）。
pub fn decode_sticker_asset(data: &[u8], is_gif: bool) -> Option<StickerAsset> {
    if is_gif {
        let gif = super::gif::decode_gif_frames_rgba_memory(data, Some((280, 280))).ok()?;
        let frames: Vec<Vec<u8>> = gif.frames.iter().map(|(px, _)| px.clone()).collect();
        let delays: Vec<u16> = gif.frames.iter().map(|(_, d)| *d).collect();
        if frames.is_empty() {
            None
        } else {
            Some(StickerAsset { frames, delays })
        }
    } else {
        let img = image::load_from_memory(data).ok()?;
        let resized = img.resize_exact(280, 280, image::imageops::FilterType::Nearest);
        Some(StickerAsset {
            frames: vec![resized.to_rgba8().into_raw()],
            delays: vec![100],
        })
    }
}

/// 解析 JS 风格的 color_shift 字符串 "sepia(x) saturate(y)" 为衰减系数。
pub fn parse_color_shift(s: &str) -> (f64, f64) {
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
