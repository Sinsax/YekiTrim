use std::fs::File;
use std::io::BufReader;
use std::path::Path;

use image::codecs::gif::GifDecoder;
use image::codecs::png::PngEncoder;
use image::{AnimationDecoder, ColorType, ImageEncoder};

/// GIF 帧: (PNG bytes, 延迟毫秒)
pub struct GifFrame {
    pub png: Vec<u8>,
    pub delay_ms: u16,
}

/// 解码 GIF 文件为多帧 PNG
pub fn decode_gif_frames(path: &Path) -> anyhow::Result<Vec<GifFrame>> {
    let file = BufReader::new(File::open(path)?);
    let decoder = GifDecoder::new(file)?;
    let mut frames = Vec::new();
    for frame in decoder.into_frames() {
        let frame = frame?;
        let (num, den) = frame.delay().numer_denom_ms();
        let delay_ms = if den > 0 { (num as f64 / den as f64).round() as u16 } else { 100 };
        let buf = frame.into_buffer();
        let (w, h) = buf.dimensions();
        let raw = buf.into_raw();
        let mut png = Vec::new();
        let encoder = PngEncoder::new(&mut png);
        encoder.write_image(&raw, w, h, ColorType::Rgba8.into())?;
        frames.push(GifFrame { png, delay_ms });
    }
    Ok(frames)
}
