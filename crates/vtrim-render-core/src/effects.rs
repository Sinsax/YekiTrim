//! wgpu 全屏特效 pass。
//!
//! 目前先把 flash / overlay / intro_black / vignette 这类简单全屏效果
//! 从 CPU 合成迁移到 GPU：通过一个全屏 shader 读取输入纹理并叠加效果。

use anyhow::Result;
use bytemuck::{Pod, Zeroable};

use super::gpu::Gpu;
use super::timeline::FrameState;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct EffectUniform {
    pub flash_color: [f32; 4],
    pub flash_alpha_pad: [f32; 4],
    pub hue_color: [f32; 4],
    pub hue_alpha_pad: [f32; 4],
    // x=intro black, y=vignette, z=noise(白色叠加 alpha), w=overlay 渐变 stop 数
    pub misc: [f32; 4],
    // x=overlay 透明度, y=shape(0 linear / 1 radial)
    pub overlay_meta: [f32; 4],
    pub grad0: [f32; 4],
    pub grad1: [f32; 4],
    pub grad2: [f32; 4],
    pub grad3: [f32; 4],
}

pub struct EffectRenderer {
    pipeline: wgpu::RenderPipeline,
    bind_group_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
}

impl EffectRenderer {
    pub fn new(device: &wgpu::Device, target_format: wgpu::TextureFormat) -> Result<Self> {
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
                    format: target_format,
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
        gpu: &Gpu,
        state: &FrameState,
        input_texture: &wgpu::Texture,
        output_view: &wgpu::TextureView,
    ) -> Result<()> {
        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        let mut buffers = Vec::new();
        let mut bind_groups = Vec::new();
        self.apply_into(
            gpu,
            state,
            input_texture,
            output_view,
            &mut encoder,
            &mut buffers,
            &mut bind_groups,
        )?;
        gpu.queue.submit(Some(encoder.finish()));
        Ok(())
    }

    /// 批量版：把 effect pass 记录进已有 encoder，资源由调用方保持到 submit。
    pub fn apply_into(
        &self,
        gpu: &Gpu,
        state: &FrameState,
        input_texture: &wgpu::Texture,
        output_view: &wgpu::TextureView,
        encoder: &mut wgpu::CommandEncoder,
        buffers: &mut Vec<wgpu::Buffer>,
        bind_groups: &mut Vec<wgpu::BindGroup>,
    ) -> Result<()> {
        let input_view = input_texture.create_view(&wgpu::TextureViewDescriptor::default());

        // 构造 uniform
        let flash_color = crate::composite::parse_color(&state.flash.color);
        // overlay：纯色 = 单 stop；渐变 = 最多 4 个 stop（对齐 Canvas 多 stop 渐变）
        let (grad_stops, overlay_opacity, overlay_shape): (Vec<[u8; 4]>, f32, f32) = match &state.overlay {
            Some(super::timeline::OverlayState::Color { color, opacity }) => (
                vec![crate::composite::parse_color(color)],
                *opacity as f32,
                0.0,
            ),
            Some(super::timeline::OverlayState::Gradient { colors, shape, opacity }) => (
                colors
                    .iter()
                    .take(4)
                    .map(|c| crate::composite::parse_color(c))
                    .collect(),
                *opacity as f32,
                if shape == "radial" { 1.0 } else { 0.0 },
            ),
            None => (Vec::new(), 0.0, 0.0),
        };
        let stop = |i: usize| -> [f32; 4] {
            let c = grad_stops.get(i).copied().unwrap_or([0, 0, 0, 255]);
            [c[0] as f32 / 255.0, c[1] as f32 / 255.0, c[2] as f32 / 255.0, 1.0]
        };

        let uniform_data = EffectUniform {
            flash_color: [
                flash_color[0] as f32 / 255.0,
                flash_color[1] as f32 / 255.0,
                flash_color[2] as f32 / 255.0,
                1.0,
            ],
            flash_alpha_pad: [state.flash.opacity as f32, 0.0, 0.0, 0.0],
            hue_color: {
                let c = state.hue_shift.as_ref().map(|h| crate::composite::parse_color(&h.color)).unwrap_or([255,0,64,255]);
                [c[0] as f32 / 255.0, c[1] as f32 / 255.0, c[2] as f32 / 255.0, 1.0]
            },
            hue_alpha_pad: [
                state.hue_shift.as_ref().map(|h| h.opacity as f32).unwrap_or(0.0),
                0.0, 0.0, 0.0,
            ],
            misc: [
                state.intro_black as f32,
                state.vignette.opacity as f32,
                state.noise as f32,
                grad_stops.len() as f32,
            ],
            overlay_meta: [overlay_opacity, overlay_shape, 0.0, 0.0],
            grad0: stop(0),
            grad1: stop(1),
            grad2: stop(2),
            grad3: stop(3),
        };

        let uniform = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("effects uniform"),
            size: std::mem::size_of::<EffectUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        gpu.queue.write_buffer(&uniform, 0, bytemuck::bytes_of(&uniform_data));

        let bind_group = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
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

        buffers.push(uniform);
        bind_groups.push(bind_group);
        Ok(())
    }
}
