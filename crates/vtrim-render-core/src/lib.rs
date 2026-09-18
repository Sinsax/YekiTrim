//! V-Trim 渲染核心：时间线数学 + wgpu 合成管线。
//!
//! 桌面 native 渲染与 WASM 浏览器预览共用此 crate，保证预览与出片效果一致。
//! 不依赖 vtrim-core / vtrim-polish / tokio——设备创建与资源加载由调用方完成。

pub mod easing;
pub mod legacy;
pub mod types;
pub mod timeline;

#[cfg(feature = "wgpu")]
pub mod gif;
#[cfg(feature = "wgpu")]
pub mod composite;
#[cfg(feature = "wgpu")]
pub mod draw;
#[cfg(feature = "wgpu")]
pub mod effects;
#[cfg(feature = "wgpu")]
pub mod gpu;
#[cfg(feature = "wgpu")]
pub mod render;
#[cfg(feature = "wgpu")]
pub mod shaders;
#[cfg(feature = "wgpu")]
pub mod sprite;

#[cfg(feature = "wgpu")]
/// 贴纸资源：多帧 GIF（每帧 RGBA + delay）或单帧 PNG。
pub struct StickerAsset {
    pub frames: Vec<Vec<u8>>,
    pub delays: Vec<u16>,
}
