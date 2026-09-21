//! 平台无关的 wgpu GPU 资源：渲染器集合 + 纹理/读回助手。
//!
//! 设备与队列由调用方创建（桌面：tokio block_on；wasm：WebGPU async），
//! 本模块只持有并复用它们。所有渲染器以 `&TextureView` 为输出目标，
//! 因此同一个 `Gpu` 既可以离屏渲染 + 读回（桌面出片），也可以直接
//! 渲染到 canvas surface（wasm 实时预览）。

use anyhow::{Context as _, Result};

use super::{draw, effects, sprite};

pub const OUTPUT_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/// 逐帧渲染目标：source = 视频底图 pass 输出；effect = 特效 pass 输出（桌面
/// 路径的读回源）。wasm 路径的 sprite 最终写入 surface view，effect 仅作中转。
pub struct FrameTargets {
    pub source_view: wgpu::TextureView,
    pub source_texture: wgpu::Texture,
    pub effect_view: wgpu::TextureView,
    pub effect_texture: wgpu::Texture,
}

/// 渲染器集合：设备/队列 + 三条渲染管线。
pub struct Gpu {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub format: wgpu::TextureFormat,
    pub width: u32,
    pub height: u32,
    pub source: draw::SourceRenderer,
    pub effects: effects::EffectRenderer,
    pub sprites: sprite::SpriteRenderer,
    readback_pool: std::sync::Mutex<Vec<wgpu::Buffer>>,
}

impl Gpu {
    pub fn from_parts(
        device: wgpu::Device,
        queue: wgpu::Queue,
        width: u32,
        height: u32,
        format: wgpu::TextureFormat,
    ) -> Result<Self> {
        Ok(Self {
            format,
            source: draw::SourceRenderer::new(&device, format)?,
            effects: effects::EffectRenderer::new(&device, format)?,
            sprites: sprite::SpriteRenderer::new(&device, format)?,
            device,
            queue,
            width,
            height,
            readback_pool: std::sync::Mutex::new(Vec::new()),
        })
    }

    /// 创建离屏渲染目标（桌面读回路径用）。
    pub fn create_output_texture(&self) -> (wgpu::Texture, wgpu::TextureView) {
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("vtrim-render-core output"),
            size: wgpu::Extent3d {
                width: self.width,
                height: self.height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: self.format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::COPY_SRC
                | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        (texture, view)
    }

    /// 逐帧双纹理目标：source（视频底图 pass 输出）+ effect（特效 pass 输出，
    /// sprite 叠加于此）。wasm 路径的 sprite 最终写入 surface view，effect 仅作中转。
    pub fn create_frame_targets(&self) -> FrameTargets {
        let (source_texture, source_view) = self.create_output_texture();
        let (effect_texture, effect_view) = self.create_output_texture();
        FrameTargets {
            source_view,
            source_texture,
            effect_view,
            effect_texture,
        }
    }

    /// 上传 CPU 侧 RGBA 像素为纹理（wasm 路径用 copy_external_image_to_texture 替代）。
    pub fn create_rgba_texture(&self, width: u32, height: u32, data: &[u8]) -> wgpu::Texture {
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("vtrim-render-core rgba input"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: OUTPUT_FORMAT,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        // wgpu 要求 bytes_per_row 对齐到 256 字节；不足时补齐并逐行复制。
        let unpadded = width * 4;
        let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let padded = unpadded.div_ceil(align) * align;
        if padded == unpadded {
            self.queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                data,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(padded),
                    rows_per_image: None,
                },
                wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
            );
        } else {
            let mut padded_data = Vec::with_capacity((padded as usize) * height as usize);
            for row in data.chunks(unpadded as usize) {
                padded_data.extend_from_slice(row);
                padded_data.resize(padded_data.len() + (padded - unpadded) as usize, 0);
            }
            self.queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                &padded_data,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(padded),
                    rows_per_image: None,
                },
                wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
            );
        }
        texture
    }

    /// 从渲染目标读回 RGBA 像素（复用 readback buffer 池）。
    ///
    /// 注意：WebGPU 后端 `Device::poll` 不阻塞等待，此方法在 wasm 显示路径
    /// 不可用——wasm 直接渲染到 canvas surface，无需读回。
    pub fn read_output_rgba(&self, texture: &wgpu::Texture) -> Result<Vec<u8>> {
        let bytes_per_pixel = 4u64;
        let unpadded_bytes_per_row = self.width as u64 * bytes_per_pixel;
        let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT as u64;
        let padded_bytes_per_row = unpadded_bytes_per_row.div_ceil(align) * align;
        let buffer_size = padded_bytes_per_row * self.height as u64;

        // 锁中毒也要取回缓冲池（与 persist.rs / logbus.rs 同口径）：`expect` 会让上一次
        // 崩溃残留把它变成"之后每次回读都 panic"。
        let buffer = if let Some(buf) = self.readback_pool.lock().unwrap_or_else(|e| e.into_inner()).pop() {
            buf
        } else {
            self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("vtrim-render-core readback"),
                size: buffer_size,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            })
        };

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(padded_bytes_per_row as u32),
                    rows_per_image: None,
                },
            },
            wgpu::Extent3d {
                width: self.width,
                height: self.height,
                depth_or_array_layers: 1,
            },
        );
        self.queue.submit(Some(encoder.finish()));

        let slice = buffer.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = tx.send(result);
        });
        self.device
            .poll(wgpu::PollType::wait_indefinitely())
            .map_err(|e| anyhow::anyhow!("wgpu poll failed: {e}"))?;
        rx.recv()
            .map_err(|_| anyhow::anyhow!("wgpu map callback dropped"))?
            .map_err(|e| anyhow::anyhow!("wgpu map failed: {e}"))?;

        let data = slice.get_mapped_range();
        let mut rgba = Vec::with_capacity((self.width * self.height * 4) as usize);
        for row in data.chunks(padded_bytes_per_row as usize) {
            rgba.extend_from_slice(&row[..unpadded_bytes_per_row as usize]);
        }
        drop(data);
        buffer.unmap();
        self.readback_pool.lock().unwrap_or_else(|e| e.into_inner()).push(buffer);
        Ok(rgba)
    }
}

/// 创建离屏 wgpu 设备+队列（桌面路径；wasm 由浏览器 WebGPU 提供设备）。
#[cfg(not(target_arch = "wasm32"))]
pub fn create_offscreen_device() -> Result<(wgpu::Device, wgpu::Queue)> {
    let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
        backends: wgpu::Backends::PRIMARY,
        flags: wgpu::InstanceFlags::empty(),
        ..Default::default()
    });
    let adapter = futures::executor::block_on(instance.request_adapter(
        &wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            force_fallback_adapter: false,
            compatible_surface: None,
        },
    ))
    .context("wgpu: no adapter available")?;
    let (device, queue) = futures::executor::block_on(adapter.request_device(
        &wgpu::DeviceDescriptor {
            label: Some("vtrim-render-core device"),
            required_features: wgpu::Features::empty(),
            required_limits: wgpu::Limits::downlevel_defaults(),
            experimental_features: wgpu::ExperimentalFeatures::default(),
            memory_hints: wgpu::MemoryHints::default(),
            trace: wgpu::Trace::Off,
        },
    ))
    .context("wgpu: request_device failed")?;
    Ok((device, queue))
}
