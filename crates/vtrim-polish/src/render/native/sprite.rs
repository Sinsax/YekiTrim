//! GPU sprite pass: draw stickers/textures onto the target.
use anyhow::Result;
use bytemuck::{Pod, Zeroable};

use super::context::WgpuContext;
use super::timeline::{CaptionState, DanmakuState, StickerState};

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct SpriteUniform {
    pub dst_rect: [f32; 4],
    pub src_rect: [f32; 4],
    pub opacity: f32,
    pub rotation: f32,
    pub _pad: [f32; 6],
}

pub struct SpriteRenderer {
    pipeline: wgpu::RenderPipeline,
    bind_group_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    texture_cache: std::collections::HashMap<u64, (wgpu::Texture, wgpu::TextureView)>,
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
        rgba: &[u8],
        w: u32,
        h: u32,
    ) -> &wgpu::TextureView {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        let mut hsh = DefaultHasher::new();
        rgba.hash(&mut hsh);
        let key = hsh.finish();
        if !self.texture_cache.contains_key(&key) {
            let tex = ctx.create_rgba_texture(w, h, rgba);
            let view = tex.create_view(&wgpu::TextureViewDescriptor::default());
            self.texture_cache.insert(key, (tex, view));
        } else {
        }
        &self.texture_cache.get(&key).unwrap().1
    }

    pub fn draw_sprite(
        &mut self,
        ctx: &WgpuContext,
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
        let view = self.get_or_create_texture(ctx, rgba, src_w, src_h).clone();

        let uniform = SpriteUniform {
            dst_rect: [dst_x, dst_y, dst_w, dst_h],
            src_rect: [0.0, 0.0, 1.0, 1.0],
            opacity,
            rotation: rotation_deg.to_radians(),
            _pad: [0.0; 6],
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

    pub fn draw_stickers(
        &mut self,
        ctx: &WgpuContext,
        stickers: &[StickerState],
        frames: &[Vec<Vec<u8>>],
        delays: &[Vec<u16>],
        t: f64,
        output_view: &wgpu::TextureView,
    ) -> Result<()> {
        for st in stickers {
            if frames.is_empty() || st.index >= frames.len() {
                continue;
            }
            let frame_set = &frames[st.index];
            if frame_set.is_empty() {
                continue;
            }
            // GIF 动画帧选择：按 delays 循环取帧
            let frame_idx = if st.index < delays.len() {
                let d = &delays[st.index];
                let total: u32 = d.iter().map(|x| *x as u32).sum();
                if total > 0 {
                    let elapsed = ((t * 1000.0) as u32) % total;
                    let mut acc = 0u32;
                    let mut idx = 0usize;
                    for (i, delay) in d.iter().enumerate() {
                        acc += *delay as u32;
                        if elapsed < acc {
                            idx = i;
                            break;
                        }
                    }
                    idx
                } else {
                    0
                }
            } else {
                0
            };
            let frame = &frame_set[frame_idx.min(frame_set.len() - 1)];
            let src_w = 280u32;
            let src_h = 280u32;
            let w = st.w as f32;
            let h = st.h as f32;
            let x = st.x as f32 - w / 2.0;
            let y = st.y as f32 - h / 2.0;
            self.draw_sprite(
                ctx,
                frame,
                src_w,
                src_h,
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
        sub_color: [u8; 4],
        hl_color: [u8; 4],
        stroke_color: [u8; 4],
        stroke_width: f32,
        font_size: f32,
        out: &wgpu::TextureView,
    ) -> Result<()> {
        for cap in caps {
            // 逐段光栅化，保留高亮颜色差异。
            let mut total_w = 0.0f32;
            let mut rendered: Vec<(Vec<u8>, u32, u32, f32)> = Vec::new();
            for part in &cap.parts {
                let color = if part.hl { hl_color } else { sub_color };
                if let Some((rgba, w, h, adv)) = super::composite::render_text_rgba(
                    &part.text,
                    font_size,
                    color,
                    stroke_color,
                    stroke_width,
                ) {
                    total_w += w as f32;
                    rendered.push((rgba, w, h, adv));
                }
            }
            if rendered.is_empty() {
                continue;
            }
            let mut sx = ctx.width as f32 / 2.0 - total_w / 2.0;
            let y = ctx.height as f32 * 0.88 + cap.y_offset as f32;
            for (rgba, w, h, _adv) in rendered {
                self.draw_sprite(
                    ctx,
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
        out: &wgpu::TextureView,
    ) -> Result<()> {
        for dm in dms {
            if let Some((rgba, w, h, _adv)) = super::composite::render_text_rgba(
                &dm.text,
                42.0,
                [255, 255, 255, 255],
                [0, 0, 0, 255],
                2.0,
            ) {
                self.draw_sprite(
                    ctx,
                    &rgba,
                    w,
                    h,
                    dm.x as f32,
                    dm.y as f32,
                    w as f32,
                    h as f32,
                    0.0,
                    dm.opacity as f32,
                    out,
                )?;
            }
        }
        Ok(())
    }
}
