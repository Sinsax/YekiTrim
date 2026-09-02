//! 使用 wgpu 渲染视频底图（含 camera zoom 变换）并编码 JPEG。

use anyhow::{Context as _, Result};
use std::io::Cursor;

use image::{ImageFormat, RgbaImage};

use super::context::WgpuContext;
use super::shaders;
use super::timeline::CameraState;

pub struct SourceRenderer {
    pipeline: wgpu::RenderPipeline,
    bind_group_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
}

impl SourceRenderer {
    pub fn new(device: &wgpu::Device) -> Result<Self> {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("vtrim-polish source shader"),
            source: wgpu::ShaderSource::Wgsl(
                format!("{}\n{}", shaders::SOURCE_VERT, shaders::SOURCE_FRAG).into(),
            ),
        });

        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("source bind group layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX,
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
            label: Some("source pipeline layout"),
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("source pipeline"),
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
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview: None,
            cache: None,
        });

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("source sampler"),
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
        })
    }

    /// 渲染一帧视频底图。`rgba` 为解码后的 RGBA 原始像素。
    pub fn render(
        &self,
        ctx: &WgpuContext,
        rgba: &[u8],
        src_width: u32,
        src_height: u32,
        camera: &CameraState,
        shake: (f64, f64),
        output_view: &wgpu::TextureView,
    ) -> Result<()> {
        let source_texture = ctx.create_rgba_texture(src_width, src_height, rgba);
        let source_view = source_texture.create_view(&wgpu::TextureViewDescriptor::default());

        // uniform: mat4 以列主序存储（wgpu/math 约定）
        let transform = camera_transform(camera, shake, ctx.width as f32, ctx.height as f32);
        let uniform = ctx.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("source transform uniform"),
            size: std::mem::size_of::<[[f32; 4]; 4]>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        ctx.queue
            .write_buffer(&uniform, 0, bytemuck::cast_slice(&transform));

        let bind_group = ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("source bind group"),
            layout: &self.bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniform.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&source_view),
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
                label: Some("source pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: output_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
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
}

/// 计算与 Canvas 渲染语义一致的相机变换矩阵（列主序）。
pub fn camera_transform(
    camera: &CameraState,
    shake: (f64, f64),
    out_w: f32,
    out_h: f32,
) -> [[f32; 4]; 4] {
    let zoom = camera.zoom as f32;
    // 像素坐标 → NDC
    let ox = (camera.ox as f32 / out_w) * 2.0 - 1.0;
    let oy = 1.0 - (camera.oy as f32 / out_h) * 2.0;
    let s = zoom;
    // shake 位移（像素）→ NDC
    let shx = (shake.0 as f32 / out_w) * 2.0;
    let shy = -(shake.1 as f32 / out_h) * 2.0;

    // 围绕 NDC 原点缩放 + shake 平移
    [
        [s, 0.0, 0.0, 0.0],
        [0.0, s, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
        [ox * (1.0 - s) + shx, oy * (1.0 - s) + shy, 0.0, 1.0],
    ]
}

/// 将 RGBA 像素编码为 JPEG（质量 75，接近 Chrome 出片）。
pub fn encode_jpeg(rgba: &[u8], width: u32, height: u32) -> Result<Vec<u8>> {
    let img = RgbaImage::from_raw(width, height, rgba.to_vec())
        .context(format!("invalid RGBA frame dimensions: {}x{} len {}", width, height, rgba.len()))?;
    let rgb = image::DynamicImage::ImageRgba8(img).to_rgb8();
    let mut out = Cursor::new(Vec::new());
    rgb.write_to(&mut out, ImageFormat::Jpeg)
        .context(format!("encode JPEG: {}x{} len {}", width, height, rgba.len()))?;
    Ok(out.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_when_no_zoom() {
        let cam = CameraState {
            zoom: 1.0,
            ox: 960.0,
            oy: 540.0,
        };
        let m = camera_transform(&cam, (0.0, 0.0), 1920.0, 1080.0);
        assert!((m[0][0] - 1.0).abs() < 1e-6);
        assert!((m[1][1] - 1.0).abs() < 1e-6);
        assert!(m[3][0].abs() < 1e-6);
        assert!(m[3][1].abs() < 1e-6);
    }

    #[test]
    fn center_origin_is_pure_scale() {
        let cam = CameraState {
            zoom: 1.5,
            ox: 960.0,
            oy: 540.0,
        };
        let m = camera_transform(&cam, (0.0, 0.0), 1920.0, 1080.0);
        assert!((m[0][0] - 1.5).abs() < 1e-6);
        assert!((m[1][1] - 1.5).abs() < 1e-6);
        assert!(m[3][0].abs() < 1e-6);
        assert!(m[3][1].abs() < 1e-6);
    }
}
