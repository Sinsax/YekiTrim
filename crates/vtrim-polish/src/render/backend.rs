use std::path::Path;

use anyhow::Result;

use crate::project::ProjectFile;

use super::context::RenderContext;
use super::frame::{RenderFrame, TimedFrame};

/// 一帧渲染器的后端抽象。
///
/// 实现者负责「给定一个 chunk 的解码帧，产出编码后的帧（JPEG 字节）」。
/// 默认按帧调用 `render_frame`；批量后端（如 Chrome）可覆盖 `render_chunk`。
pub trait FrameRenderer: Send + Sync {
    /// 后端名，用于日志与 --renderer 校验。
    fn name(&self) -> &str;

    /// 初始化整段渲染所需的静态资源（贴纸纹理、字体、效果编译等）。
    fn prepare(&mut self, _ctx: &RenderContext) -> Result<()> {
        Ok(())
    }

    /// 渲染一个 chunk 的全部帧，返回与输入等长的 JPEG 字节。
    fn render_chunk(&mut self, frames: &[TimedFrame]) -> Result<Vec<Vec<u8>>> {
        frames
            .iter()
            .map(|tf| self.render_frame(tf.t, &tf.frame))
            .collect()
    }

    /// 渲染单帧。默认实现由 `render_chunk` 调用，批量后端应优先覆盖 `render_chunk`。
    fn render_frame(&self, t: f64, frame: &RenderFrame) -> Result<Vec<u8>> {
        let _ = (t, frame);
        anyhow::bail!("backend {} does not implement render_frame", self.name())
    }

    /// 通知后端上一帧结果可回收（用于帧缓冲池）。
    fn frame_consumed(&self, _t: f64) {}
}

/// 可选后端类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenderBackendKind {
    Chrome,
    Wgpu,
}

impl RenderBackendKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Chrome => "chrome",
            Self::Wgpu => "wgpu",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "chrome" | "chome" => Some(Self::Chrome),
            "wgpu" | "native" | "native-render" => Some(Self::Wgpu),
            _ => None,
        }
    }
}

/// 创建渲染后端。
///
/// `wgpu` 后端未启用编译时，`create_backend(Wgpu)` 会给出明确的编译期/运行期错误提示。
pub fn create_backend(
    kind: RenderBackendKind,
    project_path: &Path,
    config: &ProjectFile,
) -> Result<Box<dyn FrameRenderer>> {
    match kind {
        RenderBackendKind::Chrome => Ok(Box::new(super::chrome::ChromeBackend::new(
            project_path,
            config,
        )?)),
        RenderBackendKind::Wgpu => {
            #[cfg(feature = "wgpu-render")]
            {
                return Ok(Box::new(super::native::WgpuBackend::new(
                    project_path,
                    config,
                )?));
            }
            #[cfg(not(feature = "wgpu-render"))]
            {
                anyhow::bail!(
                    "wgpu render backend is not enabled in this build; \
                     rebuild with feature `wgpu-render` or use `--renderer chrome`"
                )
            }
        }
    }
}
