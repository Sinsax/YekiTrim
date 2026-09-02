//! wgpu 全屏特效 pass。
//!
//! 目前先把 flash / overlay / intro_black / vignette 这类简单全屏效果
//! 从 CPU 合成迁移到 GPU：通过一个全屏 shader 读取输入纹理并叠加效果。

use anyhow::Result;
use bytemuck::{Pod, Zeroable};

use super::context::WgpuContext;
use super::timeline::FrameState;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct EffectUniform {
    pub flash_color: [f32; 4],
    pub flash_alpha_pad: [f32; 4],
    pub overlay_color: [f32; 4],
    pub overlay_alpha_pad: [f32; 4],
    pub hue_color: [f32; 4],
    pub hue_alpha_pad: [f32; 4],
    pub misc_pad: [f32; 4],
    pub blur_color_shift: [f32; 4],
}

pub struct EffectRenderer {
    pipeline: wgpu::RenderPipeline,
    bind_group_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
}

impl EffectRenderer {
    pub fn new(device: &wgpu::Device) -> Result<Self> {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("vtrim-polish effects shader"),
            source: wgpu::ShaderSource::Wgsl(super::shaders::EFFECT_SHADER.into()),
        });

        let bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("effects bind group layout"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::FRAGMENT,
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
            label: Some("effects pipeline layout"),
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("effects pipeline"),
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
            label: Some("effects sampler"),
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

    /// 将上一 pass 输出纹理作为输入，应用全屏效果并写入 output_view。
    pub fn apply(
        &self,
        ctx: &WgpuContext,
        state: &FrameState,
        input_texture: &wgpu::Texture,
        output_view: &wgpu::TextureView,
    ) -> Result<()> {
        let input_view = input_texture.create_view(&wgpu::TextureViewDescriptor::default());

        // 构造 uniform
        let flash_color = crate::render::native::composite::parse_color(&state.flash.color);
        let overlay_color = match &state.overlay {
            Some(super::timeline::OverlayState::Color { color, .. }) => {
                crate::render::native::composite::parse_color(color)
            }
            Some(super::timeline::OverlayState::Gradient { colors, .. }) => {
                colors
                    .first()
                    .map(|c| crate::render::native::composite::parse_color(c))
                    .unwrap_or([0, 0, 0, 255])
            }
            None => [255, 255, 255, 255],
        };
        let overlay_alpha = match &state.overlay {
            Some(super::timeline::OverlayState::Color { opacity, .. }) => *opacity as f32,
            Some(super::timeline::OverlayState::Gradient { opacity, .. }) => *opacity as f32,
            None => 0.0,
        };

        let uniform_data = EffectUniform {
            flash_color: [
                flash_color[0] as f32 / 255.0,
                flash_color[1] as f32 / 255.0,
                flash_color[2] as f32 / 255.0,
                1.0,
            ],
            flash_alpha_pad: [state.flash.opacity as f32, 0.0, 0.0, 0.0],
            overlay_color: [
                overlay_color[0] as f32 / 255.0,
                overlay_color[1] as f32 / 255.0,
                overlay_color[2] as f32 / 255.0,
                1.0,
            ],
            overlay_alpha_pad: [overlay_alpha, 0.0, 0.0, 0.0],
            hue_color: {
                let c = state.hue_shift.as_ref().map(|h| crate::render::native::composite::parse_color(&h.color)).unwrap_or([255,0,64,255]);
                [c[0] as f32 / 255.0, c[1] as f32 / 255.0, c[2] as f32 / 255.0, 1.0]
            },
            hue_alpha_pad: [
                state.hue_shift.as_ref().map(|h| h.opacity as f32).unwrap_or(0.0),
                0.0, 0.0, 0.0,
            ],
            misc_pad: [
                state.intro_black as f32,
                state.vignette.opacity as f32,
                state.noise as f32,
                0.0,
            ],
            blur_color_shift: [
                state.blur as f32,
                if state.color_shift.is_empty() { 0.0 } else { 1.0 },
                0.0,
                0.0,
            ],
        };

        let uniform = ctx.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("effects uniform"),
            size: std::mem::size_of::<EffectUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        ctx.queue.write_buffer(&uniform, 0, bytemuck::bytes_of(&uniform_data));

        let bind_group = ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("effects bind group"),
            layout: &self.bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniform.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&input_view),
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
                label: Some("effects pass"),
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
