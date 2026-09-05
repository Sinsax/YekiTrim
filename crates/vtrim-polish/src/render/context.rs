use std::path::PathBuf;

use crate::project::ProjectFile;

/// 渲染后端初始化所需的上下文。
///
/// 目前包含整段渲染的静态信息；后续 timeline/sound 等后端无关数据也可放进来。
#[derive(Debug, Clone)]
pub struct RenderContext {
    pub project_path: PathBuf,
    pub config: ProjectFile,

    /// 视频尺寸（输出画布尺寸，默认 1920×1080）。
    pub width: u32,
    pub height: u32,
    pub fps: usize,

    /// Chrome 后端使用的页面 URL；wgpu 后端不需要。
    pub server_url: Option<String>,

    /// 预解码贴纸 GIF 数据 JSON（Chrome 路径保持现状）。
    pub sticker_data_json: String,

    /// 竖屏布局（源视频高>宽时启用：模糊背景 + 主画面按 vertical_position 摆放）。
    pub vertical: bool,
    pub vertical_position: String,
}

impl RenderContext {
    pub fn new(project_path: PathBuf, config: ProjectFile) -> Self {
        Self {
            project_path,
            config,
            width: 1920,
            height: 1080,
            fps: 30,
            server_url: None,
            sticker_data_json: "{}".into(),
            vertical: false,
            vertical_position: "left".into(),
        }
    }
}
