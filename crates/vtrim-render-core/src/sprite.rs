//! GPU sprite pass: draw stickers/textures onto the target.
use anyhow::{anyhow, Result};
use bytemuck::{Pod, Zeroable};

use super::composite::{self, FontSpec};
use super::gpu::Gpu;
use super::timeline::{CaptionState, DanmakuState, StickerState};
use crate::types::{DanmakuStyle, SubtitleStyle};

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct SpriteUniform {
    pub dst_rect: [f32; 4],
    pub src_rect: [f32; 4],
    // x=opacity, y=rotation(rad), z=输出宽, w=输出高（打包成 vec4 保证 WGSL 对齐一致）
    pub meta: [f32; 4],
}

/// 一次 sprite pass 内创建的 GPU 资源；在 `encoder.finish()` 后、`queue.submit()` 前必须保持存活。
pub struct SpriteFrameResources {
    buffers: Vec<wgpu::Buffer>,
    bind_groups: Vec<wgpu::BindGroup>,
}

impl SpriteFrameResources {
    pub fn new() -> Self {
        Self {
            buffers: Vec::new(),
            bind_groups: Vec::new(),
        }
    }
}

struct CachedTexture {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    width: u32,
    height: u32,
}

pub struct SpriteRenderer {
    pipeline: wgpu::RenderPipeline,
    bind_group_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    texture_cache: std::sync::Mutex<std::collections::HashMap<u64, CachedTexture>>,
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
    pub fn new(device: &wgpu::Device, target_format: wgpu::TextureFormat) -> Result<Self> {
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
                    format: target_format,
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
            texture_cache: std::sync::Mutex::new(std::collections::HashMap::new()),
        })
    }

    fn get_or_create_texture(
        &self,
        gpu: &Gpu,
        key: u64,
        rgba: &[u8],
        w: u32,
        h: u32,
    ) -> (wgpu::TextureView, u32, u32) {
        let mut cache = self.texture_cache.lock().expect("sprite texture cache");
        if cache.len() >= TEXTURE_CACHE_CAP {
            cache.clear();
        }
        let entry = cache.entry(key).or_insert_with(|| {
            let tex = gpu.create_rgba_texture(w, h, rgba);
            let view = tex.create_view(&wgpu::TextureViewDescriptor::default());
            CachedTexture {
                texture: tex,
                view,
                width: w,
                height: h,
            }
        });
        (entry.view.clone(), entry.width, entry.height)
    }

    fn get_texture(&self, key: u64) -> Option<(wgpu::TextureView, u32, u32)> {
        let cache = self.texture_cache.lock().expect("sprite texture cache");
        cache.get(&key).map(|c| (c.view.clone(), c.width, c.height))
    }

    fn texture_size(&self, key: u64) -> Option<(u32, u32)> {
        let cache = self.texture_cache.lock().expect("sprite texture cache");
        cache.get(&key).map(|c| (c.width, c.height))
    }

    pub fn draw_sprite(
        &self,
        gpu: &Gpu,
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
        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        let mut resources = SpriteFrameResources::new();
        self.draw_sprite_into(
            gpu,
            key,
            Some(rgba),
            src_w,
            src_h,
            dst_x,
            dst_y,
            dst_w,
            dst_h,
            rotation_deg,
            opacity,
            output_view,
            &mut encoder,
            &mut resources,
        )?;
        gpu.queue.submit(Some(encoder.finish()));
        Ok(())
    }

    /// 把单个 sprite 记录进已有 encoder。`rgba == None` 表示纹理已缓存，只按 key 取用。
    #[allow(clippy::too_many_arguments)]
    pub fn draw_sprite_into(
        &self,
        gpu: &Gpu,
        key: u64,
        rgba: Option<&[u8]>,
        src_w: u32,
        src_h: u32,
        dst_x: f32,
        dst_y: f32,
        dst_w: f32,
        dst_h: f32,
        rotation_deg: f32,
        opacity: f32,
        output_view: &wgpu::TextureView,
        encoder: &mut wgpu::CommandEncoder,
        resources: &mut SpriteFrameResources,
    ) -> Result<()> {
        let (view, _, _) = match rgba {
            Some(data) => self.get_or_create_texture(gpu, key, data, src_w, src_h),
            None => self
                .get_texture(key)
                .ok_or_else(|| anyhow!("sprite texture {key} not cached"))?,
        };

        let uniform = SpriteUniform {
            dst_rect: [dst_x, dst_y, dst_w, dst_h],
            src_rect: [0.0, 0.0, 1.0, 1.0],
            meta: [
                opacity,
                rotation_deg.to_radians(),
                gpu.width as f32,
                gpu.height as f32,
            ],
        };
        let buf = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("sprite uniform"),
            size: std::mem::size_of::<SpriteUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        gpu.queue.write_buffer(&buf, 0, bytemuck::bytes_of(&uniform));

        let bind_group = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
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

        resources.buffers.push(buf);
        resources.bind_groups.push(bind_group);
        Ok(())
    }

    /// 绘制贴纸：按 `state.name` 从资源表取帧，GIF 帧相位按事件相对时间（与 JS 一致）。
    pub fn draw_stickers(
        &self,
        gpu: &Gpu,
        stickers: &[StickerState],
        assets: &std::collections::HashMap<String, crate::StickerAsset>,
        t: f64,
        output_view: &wgpu::TextureView,
    ) -> Result<()> {
        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        let mut resources = SpriteFrameResources::new();
        self.draw_stickers_into(
            gpu,
            stickers,
            assets,
            t,
            output_view,
            &mut encoder,
            &mut resources,
        )?;
        gpu.queue.submit(Some(encoder.finish()));
        Ok(())
    }

    /// 批量版：所有贴纸记录到同一个 encoder。
    pub fn draw_stickers_into(
        &self,
        gpu: &Gpu,
        stickers: &[StickerState],
        assets: &std::collections::HashMap<String, crate::StickerAsset>,
        t: f64,
        output_view: &wgpu::TextureView,
        encoder: &mut wgpu::CommandEncoder,
        resources: &mut SpriteFrameResources,
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
                crate::timeline::sticker_frame_index_for(st.et, t, &asset.delays)
            } else {
                0
            };
            let frame = &frame_set[frame_idx.min(frame_set.len() - 1)];
            let w = st.w as f32;
            let h = st.h as f32;
            let x = st.x as f32 - w / 2.0;
            let y = st.y as f32 - h / 2.0;
            let key = tex_key("sticker", &format!("{}|{}", st.name, frame_idx), 0);
            self.draw_sprite_into(
                gpu,
                key,
                Some(frame),
                280,
                280,
                x,
                y,
                w * st.scale as f32,
                h * st.scale as f32,
                st.rot as f32,
                st.opacity as f32,
                output_view,
                encoder,
                resources,
            )?;
        }
        Ok(())
    }

    pub fn draw_captions(
        &self,
        gpu: &Gpu,
        caps: &[CaptionState],
        sub: &SubtitleStyle,
        font: &FontSpec,
        out: &wgpu::TextureView,
    ) -> Result<()> {
        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        let mut resources = SpriteFrameResources::new();
        self.draw_captions_into(
            gpu,
            caps,
            sub,
            font,
            out,
            &mut encoder,
            &mut resources,
        )?;
        gpu.queue.submit(Some(encoder.finish()));
        Ok(())
    }

    /// 批量版：先查纹理缓存，未缓存才做 CPU 文字光栅化；所有字幕记录到同一个 encoder。
    pub fn draw_captions_into(
        &self,
        gpu: &Gpu,
        caps: &[CaptionState],
        sub: &SubtitleStyle,
        font: &FontSpec,
        out: &wgpu::TextureView,
        encoder: &mut wgpu::CommandEncoder,
        resources: &mut SpriteFrameResources,
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
        let cy = gpu.height as f32 * (960.0 / 1080.0);
        for cap in caps {
            let mut total_w = 0.0f32;
            let mut rendered: Vec<(Option<Vec<u8>>, u32, u32, u64)> = Vec::new();
            for part in &cap.parts {
                let key = tex_key("cap", &format!("{}|{}", part.text, part.hl), 0);
                if let Some((w, h)) = self.texture_size(key) {
                    total_w += w as f32;
                    rendered.push((None, w, h, key));
                    continue;
                }
                let color = if part.hl { hl_color } else { sub_color };
                if let Some((rgba, w, h, _adv)) = composite::render_text_rgba(
                    &part.text,
                    font_size,
                    color,
                    stroke_color,
                    stroke_width,
                    font,
                ) {
                    total_w += w as f32;
                    rendered.push((Some(rgba), w, h, key));
                }
            }
            if rendered.is_empty() {
                continue;
            }
            let mut sx = gpu.width as f32 / 2.0 - total_w / 2.0;
            for (rgba, w, h, key) in rendered {
                // 文本基线居中于 cy：贴图左上角 = cy - h/2 + 淡入淡出位移
                let y = cy - h as f32 / 2.0 + cap.y_offset as f32;
                self.draw_sprite_into(
                    gpu,
                    key,
                    rgba.as_deref(),
                    w,
                    h,
                    sx,
                    y,
                    w as f32,
                    h as f32,
                    0.0,
                    cap.opacity as f32,
                    out,
                    encoder,
                    resources,
                )?;
                sx += w as f32;
            }
        }
        Ok(())
    }

    pub fn draw_danmaku(
        &self,
        gpu: &Gpu,
        dms: &[DanmakuState],
        dm: &DanmakuStyle,
        font: &FontSpec,
        out: &wgpu::TextureView,
    ) -> Result<()> {
        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        let mut resources = SpriteFrameResources::new();
        self.draw_danmaku_into(
            gpu,
            dms,
            dm,
            font,
            out,
            &mut encoder,
            &mut resources,
        )?;
        gpu.queue.submit(Some(encoder.finish()));
        Ok(())
    }

    /// 批量版：弹幕纹理按文本缓存（与 x 无关），移动弹幕不会每帧新建/上传纹理。
    pub fn draw_danmaku_into(
        &self,
        gpu: &Gpu,
        dms: &[DanmakuState],
        dm: &DanmakuStyle,
        font: &FontSpec,
        out: &wgpu::TextureView,
        encoder: &mut wgpu::CommandEncoder,
        resources: &mut SpriteFrameResources,
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
            let key = tex_key("dm", &format!("{}|{}", d.text, font_size as u32), 0);
            let (rgba, w, h) = if let Some((w, h)) = self.texture_size(key) {
                (None, w, h)
            } else if let Some((rgba, w, h, _adv)) = composite::render_text_rgba(
                &d.text,
                font_size,
                color,
                stroke_color,
                stroke_width,
                font,
            ) {
                (Some(rgba), w, h)
            } else {
                continue;
            };
            // Canvas textBaseline=middle：dm.y 为文字中心
            let y = d.y as f32 - h as f32 / 2.0;
            self.draw_sprite_into(
                gpu,
                key,
                rgba.as_deref(),
                w,
                h,
                d.x as f32,
                y,
                w as f32,
                h as f32,
                0.0,
                d.opacity as f32,
                out,
                encoder,
                resources,
            )?;
        }
        Ok(())
    }
}
