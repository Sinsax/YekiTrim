use std::fs::File;
use std::io::BufReader;
use std::path::Path;

use image::codecs::gif::GifDecoder;
use image::{AnimationDecoder, ColorType, ImageEncoder, RgbaImage};

/// 共享解码：GIF → 逐帧 (RGBA 画布, 延迟毫秒)。
fn decode_frames(path: &Path) -> anyhow::Result<Vec<(RgbaImage, u16)>> {
    let file = BufReader::new(File::open(path)?);
    let decoder = GifDecoder::new(file)?;
    let mut frames = Vec::new();
    for frame in decoder.into_frames() {
        let frame = frame?;
        let (num, den) = frame.delay().numer_denom_ms();
        let delay_ms = if den > 0 { (num as f64 / den as f64).round() as u16 } else { 100 };
        frames.push((frame.into_buffer(), delay_ms));
    }
    Ok(frames)
}

/// 解码后的 GIF：每帧 RGBA 像素（已按需缩放）+ 延迟。wgpu 路径用。
pub struct GifFrames {
    /// (RGBA 像素（width×height×4）, 延迟毫秒)
    pub frames: Vec<(Vec<u8>, u16)>,
    pub width: u32,
    pub height: u32,
}

/// 解码 GIF 为多帧 RGBA（可选缩放到目标尺寸，Nearest 采样）。
/// 直接输出像素，不做 PNG 中转。
pub fn decode_gif_frames_rgba(path: &Path, resize: Option<(u32, u32)>) -> anyhow::Result<GifFrames> {
    let mut width = 0u32;
    let mut height = 0u32;
    let mut frames = Vec::new();
    for (buf, delay_ms) in decode_frames(path)? {
        (width, height) = buf.dimensions();
        let img = image::DynamicImage::ImageRgba8(buf);
        let rgba = match resize {
            Some((tw, th)) => {
                img.resize_exact(tw, th, image::imageops::FilterType::Nearest)
                    .to_rgba8()
                    .into_raw()
            }
            None => img.to_rgba8().into_raw(),
        };
        frames.push((rgba, delay_ms));
    }
    Ok(GifFrames { frames, width, height })
}

/// 解码 GIF 为多帧 PNG（Chrome 渲染路径：base64 后经页面 <img> 解码）。
pub fn decode_gif_frames_png(path: &Path) -> anyhow::Result<Vec<(Vec<u8>, u16)>> {
    let mut out = Vec::new();
    for (buf, delay_ms) in decode_frames(path)? {
        let (w, h) = buf.dimensions();
        let raw = buf.into_raw();
        let mut png = Vec::new();
        let encoder = image::codecs::png::PngEncoder::new(&mut png);
        encoder.write_image(&raw, w, h, ColorType::Rgba8.into())?;
        out.push((png, delay_ms));
    }
    Ok(out)
}
