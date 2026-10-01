use std::path::Path;

use anyhow::Result;

use crate::project::ProjectFile;

use super::context::RenderContext;
use super::frame::{RenderFrame, TimedFrame};

/// 一帧渲染器的后端抽象。
///
/// 实现者负责「给定一个 chunk 的解码帧，产出编码后的帧（JPEG 字节）」。
/// 默认按帧调用 `render_frame`；批量后端可覆盖 `render_chunk`。
///
/// ⚠️ **现在没有任何实现**：唯一的逐帧后端曾是 headless Chrome（`chrome.rs`），整条已删除
/// （2026-10-01，见 `RenderBackendKind`）。**这个 seam 是留着的** —— 将来若有"逐帧、进程内"的
/// 后端要接，接在这里；但**不要再接回一个需要外部浏览器的实现**。
pub trait FrameRenderer: Send + Sync {
    /// 后端名，用于日志与 `--renderer` 校验。
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
///
/// ⚠️ **曾经有 `Wgpu`**（`vtrim-render-core` 的 wgpu 合成管线 + `render/native/`）——
/// 整条已删除（用户拍板：GPU 出片交给独立的 Dhampir 底座）。
/// ⚠️ **曾经有 `Chrome`**（本仓 headless Chrome 逐帧 canvas，`render/chrome.rs` + `headless_chrome`
/// 依赖）—— 2026-10-01 整条删除（用户拍板：出片只有底座一条腿，不再要求用户装浏览器）。
///
/// 所以 `--renderer` / `VTEDIT_RENDERER` 现在**只认 `dhampir`**；老命令里的 `wgpu` / `native`
/// 仍视作 `dhampir`（它们本来就是"用 GPU 出片"的意思）；`chrome` 与拼错的 `chome` **认不出**
/// —— 调用方按"认不出就报错"处理（见 `pipeline.rs`），**别把 `chrome` 归到 `dhampir` 上**：
/// 那是静默换路，而两版成片的差别只有对着看才发现。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenderBackendKind {
    /// **Dhampir 底座**（独立仓，走二进制）。
    ///
    /// 它**不是** `FrameRenderer` 的实现 —— 那条 trait 是**逐帧**的
    /// （`render_frame(t, frame)`），而 Dhampir 的二进制是**整段一起出**
    /// （`render --from --to --out`）。硬塞进逐帧接口就得每帧起一个进程，
    /// 那比当年那条 Chrome 逐帧腿还慢。
    ///
    /// 所以它走**出片主流程里的另一条分支**（`pipeline.rs`），
    /// 产物落在**同一个中间文件**上，后面的音轨链路一行不改。
    Dhampir,
}

impl RenderBackendKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Dhampir => "dhampir",
        }
    }

    /// 认不出的值返回 `None`（含已删除的 `wgpu` / `native` / `chrome`）—— 由调用方决定怎么报。
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "dhampir" | "wgpu" | "native" => Some(Self::Dhampir),
            // ⚠️ `chrome` / 老命令里拼错的 `chome` **不再被认成任何东西**：那条腿已删除，
            // 认成 dhampir 就是"选了 Chrome 却在跑底座"的静默换路。报错文案由 `pipeline.rs` 给。
            _ => None,
        }
    }
}

/// 创建渲染后端。
///
/// ⚠️ 唯一的逐帧后端（Chrome）已删除，所以**这里恒返回 Err** —— 保留这个函数是为了
/// 出片主流程的调用点形状不变（`render_chunk_sync` 走的是"先选后端再渲染"那条老路）。
/// 真有第二个后端时在这里接，别再把调用点改成特判。
pub fn create_backend(
    kind: RenderBackendKind,
    _project_path: &Path,
    _config: &ProjectFile,
) -> Result<Box<dyn FrameRenderer>> {
    match kind {
        // Dhampir **不是逐帧后端**（见枚举上的说明）—— 有人走到这里说明调用方
        // 用错了路：出片主流程应当在选完渲染器之后就走另一条分支。
        RenderBackendKind::Dhampir => Err(anyhow::anyhow!(
            "Dhampir 是整段出片的后端，不走逐帧的 FrameRenderer 接口（见 render/dhampir.rs）"
        )),
    }
}
