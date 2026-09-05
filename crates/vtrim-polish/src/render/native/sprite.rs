//! GPU sprite pass: draw stickers/textures onto the target.
use anyhow::Result;
use bytemuck::{Pod, Zeroable};

use super::composite::{self, FontSpec};
use super::context::WgpuContext;
use super::timeline::{CaptionState, DanmakuState, StickerState};
use crate::project::{DanmakuStyle, SubtitleStyle};

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct SpriteUniform {
    pub dst_rect: [f32; 4],
    pub src_rect: [f32; 4],
    // x=opacity, y=rotation(rad), z=输出宽, w=输出高（打包成 vec4 保证 WGSL 对齐一致）
    pub meta: [f32; 4],
}

pub struct SpriteRenderer {
    pipeline: wgpu::RenderPipeline,
    bind_group_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    texture_cache: std::collections::HashMap<u64, (wgpu::Texture, wgpu::TextureView)>,
}

/// 纹理缓存上限：超出后整体清空（文字/贴纸纹理随时间增长，防止驻留 GPU 内存）。
const TEXTURE_CACHE_CAP: usize = 512;

/// 计算稳定的纹理缓存 key（内容哈希开销大，改为调用方提供语义 key）。
fn tex_key(kind: &str, id: &str, variant: u32) -> u64 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut h = DefaultHasher::new();
    kind.hash(&mut h);
    id.hash(&mut h);
    variant.hash(&mut h);
    h.finish()
}

impl SpriteRenderer {
    pub fn new(device: &wgpu::Device) -> Result<Self> {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("vtrim-polish sprite shader"),
            source: wgpu::ShaderSource::Wgsl(super::shaders::SPRITE_SHADER.into()),
        });

        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("sprite bind group layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("sprite pipeline layout"),
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("sprite pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleStrip,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: super::context::OUTPUT_FORMAT,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview: None,
            cache: None,
        });

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("sprite sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });

        Ok(Self {
            pipeline,
            bind_group_layout,
            sampler,
            texture_cache: std::collections::HashMap::new(),
        })
    }

    fn get_or_create_texture(
        &mut self,
        ctx: &WgpuContext,
        key: u64,
        rgba: &[u8],
        w: u32,
        h: u32,
    ) -> &wgpu::TextureView {
        if self.texture_cache.len() >= TEXTURE_CACHE_CAP {
            self.texture_cache.clear();
        }
        if !self.texture_cache.contains_key(&key) {
            let tex = ctx.create_rgba_texture(w, h, rgba);
            let view = tex.create_view(&wgpu::TextureViewDescriptor::default());
            self.texture_cache.insert(key, (tex, view));
        }
        &self.texture_cache.get(&key).unwrap().1
    }

    pub fn draw_sprite(
        &mut self,
        ctx: &WgpuContext,
        key: u64,
        rgba: &[u8],
        src_w: u32,
        src_h: u32,
        dst_x: f32,
        dst_y: f32,
        dst_w: f32,
        dst_h: f32,
        rotation_deg: f32,
        opacity: f32,
        output_view: &wgpu::TextureView,
    ) -> Result<()> {
        let view = self.get_or_create_texture(ctx, key, rgba, src_w, src_h).clone();

        let uniform = SpriteUniform {
            dst_rect: [dst_x, dst_y, dst_w, dst_h],
            src_rect: [0.0, 0.0, 1.0, 1.0],
            meta: [
                opacity,
                rotation_deg.to_radians(),
                ctx.width as f32,
                ctx.height as f32,
            ],
        };
        let buf = ctx.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("sprite uniform"),
            size: std::mem::size_of::<SpriteUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        ctx.queue.write_buffer(&buf, 0, bytemuck::bytes_of(&uniform));

        let bind_group = ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("sprite bind group"),
            layout: &self.bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        });

        let mut encoder = ctx
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("sprite pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: output_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &bind_group, &[]);
            pass.draw(0..4, 0..1);
        }
        ctx.queue.submit(Some(encoder.finish()));
        Ok(())
    }

    /// 绘制贴纸：按 `state.name` 从资源表取帧，GIF 帧相位按事件相对时间（与 JS 一致）。
    pub fn draw_stickers(
        &mut self,
        ctx: &WgpuContext,
        stickers: &[StickerState],
        assets: &std::collections::HashMap<String, super::StickerAsset>,
        t: f64,
        output_view: &wgpu::TextureView,
    ) -> Result<()> {
        for st in stickers {
            let Some(asset) = assets.get(&st.name) else {
                continue;
            };
            let frame_set = &asset.frames;
            if frame_set.is_empty() {
                continue;
            }
            // GIF 动画帧选择：按事件相对时间循环（对齐 JS (t-et)*1000 % total）
            let frame_idx = if asset.delays.len() > 1 {
                super::sticker_frame_index(st.et, t, &asset.delays)
            } else {
                0
            };
            let frame = &frame_set[frame_idx.min(frame_set.len() - 1)];
            let w = st.w as f32;
            let h = st.h as f32;
            let x = st.x as f32 - w / 2.0;
            let y = st.y as f32 - h / 2.0;
            let key = tex_key("sticker", &format!("{}|{}", st.name, frame_idx), 0);
            self.draw_sprite(
                ctx,
                key,
                frame,
                280,
                280,
                x,
                y,
                w * st.scale as f32,
                h * st.scale as f32,
                st.rot as f32,
                st.opacity as f32,
                output_view,
            )?;
        }
        Ok(())
    }

    pub fn draw_captions(
        &mut self,
        ctx: &WgpuContext,
        caps: &[CaptionState],
        sub: &SubtitleStyle,
        font: &FontSpec,
        out: &wgpu::TextureView,
    ) -> Result<()> {
        let sub_color = composite::parse_color(&sub.color);
        let hl_color = composite::parse_color(&sub.highlight_color);
        let stroke_color = composite::parse_color(sub.stroke.split_whitespace().last().unwrap_or("#403c3b"));
        let stroke_width = sub
            .stroke
            .split_whitespace()
            .next()
            .and_then(|s| s.trim_end_matches("px").parse::<f32>().ok())
            .unwrap_or(12.0);
        let font_size = sub.size as f32;
        // Canvas 字幕纵向中心在 cy=960（1920×1080 空间）
        let cy = ctx.height as f32 * (960.0 / 1080.0);
        for cap in caps {
            // 逐段光栅化，保留高亮颜色差异；两遍：先量总宽，再逐段带缓存 key 绘制。
            let mut total_w = 0.0f32;
            let mut rendered: Vec<(Vec<u8>, u32, u32, u64)> = Vec::new();
            for part in &cap.parts {
                let color = if part.hl { hl_color } else { sub_color };
                if let Some((rgba, w, h, _adv)) = composite::render_text_rgba(
                    &part.text,
                    font_size,
                    color,
                    stroke_color,
                    stroke_width,
                    font,
                ) {
                    let key = tex_key(
                        "cap",
                        &format!("{}|{}|{}|{}", part.text, w, h, part.hl),
                        0,
                    );
                    total_w += w as f32;
                    rendered.push((rgba, w, h, key));
                }
            }
            if rendered.is_empty() {
                continue;
            }
            let mut sx = ctx.width as f32 / 2.0 - total_w / 2.0;
            for (rgba, w, h, key) in rendered {
                // 文本基线居中于 cy：贴图左上角 = cy - h/2 + 淡入淡出位移
                let y = cy - h as f32 / 2.0 + cap.y_offset as f32;
                self.draw_sprite(
                    ctx,
                    key,
                    &rgba,
                    w,
                    h,
                    sx,
                    y,
                    w as f32,
                    h as f32,
                    0.0,
                    cap.opacity as f32,
                    out,
                )?;
                sx += w as f32;
            }
        }
        Ok(())
    }

    pub fn draw_danmaku(
        &mut self,
        ctx: &WgpuContext,
        dms: &[DanmakuState],
        dm: &DanmakuStyle,
        font: &FontSpec,
        out: &wgpu::TextureView,
    ) -> Result<()> {
        let font_size = dm.font_size as f32;
        let color = composite::parse_color(&dm.color);
        let stroke_color = composite::parse_color(dm.stroke.split_whitespace().last().unwrap_or("#000000"));
        let stroke_width = dm
            .stroke
            .split_whitespace()
            .next()
            .and_then(|s| s.trim_end_matches("px").parse::<f32>().ok())
            .unwrap_or(2.0);
        for d in dms {
            if let Some((rgba, w, h, _adv)) = composite::render_text_rgba(
                &d.text,
                font_size,
                color,
                stroke_color,
                stroke_width,
                font,
            ) {
                // Canvas textBaseline=middle：dm.y 为文字中心
                let y = d.y as f32 - h as f32 / 2.0;
                let key = tex_key("dm", &format!("{}|{}", d.x, d.text), 0);
                self.draw_sprite(
                    ctx,
                    key,
                    &rgba,
                    w,
                    h,
                    d.x as f32,
                    y,
                    w as f32,
                    h as f32,
                    0.0,
                    d.opacity as f32,
                    out,
                )?;
            }
        }
        Ok(())
    }
}
