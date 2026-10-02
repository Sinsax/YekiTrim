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

/// 贴纸图 → 逐帧 PNG（Chrome 出片路径的统一入口）。
///
/// GIF 走多帧；**PNG / JPEG 等静态图当一帧**—— 贴纸库允许 `gif/png/jpg/jpeg/webp`
/// （`sticker_index::is_image_file_name` 是唯一判据），而出片路径以前只认 GIF：
/// 那些贴纸会被复制进项目、DOM 预览的 `<img>` 也能显示，**成片里却一张都没有**
/// （`prepare_sticker_data` 找不到 `<name>.gif` 就跳过，日志里一个字都没有）。
///
/// WebP 没有开解码 feature → 返回 Err，由调用方报出来（宁可报，不可静默缺图）。
pub fn decode_sticker_frames_png(path: &Path) -> anyhow::Result<Vec<(Vec<u8>, u16)>> {
    let is_gif = path
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("gif"));
    if is_gif {
        return decode_gif_frames_png(path);
    }
    let img = image::open(path)?.to_rgba8();
    let (w, h) = img.dimensions();
    let mut png = Vec::new();
    image::codecs::png::PngEncoder::new(&mut png)
        .write_image(img.as_raw(), w, h, ColorType::Rgba8.into())?;
    // 静态图没有延迟；页面按帧索引取，永远只会用到第 0 帧
    Ok(vec![(png, 100)])
}

/// 贴纸长条：N 帧**横向拼成一张 PNG**，外加每帧宽高与延迟。
pub struct StickerStrip {
    pub png: Vec<u8>,
    /// 单帧宽高（页面上按这个切片：第 k 帧 = `drawImage(strip, k*w, 0, w, h, ...)`）。
    pub width: u32,
    pub height: u32,
    pub delays_ms: Vec<u16>,
}

impl StickerStrip {
    pub fn frame_count(&self) -> usize {
        self.delays_ms.len()
    }
}

/// **贴纸长条**：把整段动画拼成一张横向 PNG，交给浏览器按格子切片。
///
/// # 为什么"谁来解 GIF"必须由服务端回答
///
/// 浏览器这一侧**没有可靠的多帧图解码**，两条路都实测过：
///
///   * `<img>` 的动画时钟会被节流 —— 不可见、被 canvas 遮住、哪怕 1×1 盖在最上层
///     （不被遮挡），目标浏览器都**不推进 GIF 帧**；
///   * `ImageDecoder`（WebCodecs）在目标浏览器里**根本不存在**
///     （`typeof ImageDecoder !== 'function'`，于是 `decoder=false`、帧数 0）。
///
/// 所以序列帧只能由服务端给。**关键是它必须与出片同源** —— 这里走的是
/// [`decode_sticker_frames_png`] 用的那份 `decode_frames`，于是
/// **预览里贴纸的每一帧与成片逐帧一致**（换第二套解码器就迟早对不上）。
///
/// # 为什么拼一条而不是逐帧返回
///
/// 一次取回、页面按格子 `drawImage` 切片：60fps 的预览里不再产生逐帧请求，
/// 也不用在页面侧做 base64/Blob 的反复转换。
pub fn sticker_strip_png(path: &Path) -> anyhow::Result<StickerStrip> {
    let is_gif = path
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("gif"));
    let frames: Vec<(RgbaImage, u16)> = if is_gif {
        decode_frames(path)?
    } else {
        // 与 `decode_sticker_frames_png` 同一口径：静态图当一帧。
        vec![(image::open(path)?.to_rgba8(), 100)]
    };
    let Some((first, _)) = frames.first() else {
        anyhow::bail!("贴纸 {path:?} 一帧都解不出来");
    };
    let (w, h) = first.dimensions();
    if w == 0 || h == 0 {
        anyhow::bail!("贴纸 {path:?} 的尺寸是 {w}x{h}");
    }
    let mut strip = RgbaImage::new(w * frames.len() as u32, h);
    for (i, (buf, _)) in frames.iter().enumerate() {
        // **尺寸不一致直接报错**，不裁不缩：静默缩放会让贴纸"糊一点"、
        // 看起来像素材本身的问题，而不是"第 i 帧不对"。
        if buf.dimensions() != (w, h) {
            anyhow::bail!(
                "贴纸 {path:?} 的第 {i} 帧是 {}x{}，与首帧 {w}x{h} 不一致",
                buf.width(),
                buf.height()
            );
        }
        image::imageops::replace(&mut strip, buf, (i as i64) * (w as i64), 0);
    }
    let mut png = Vec::new();
    image::codecs::png::PngEncoder::new(&mut png)
        .write_image(strip.as_raw(), strip.width(), strip.height(), ColorType::Rgba8.into())?;
    Ok(StickerStrip {
        png,
        width: w,
        height: h,
        delays_ms: frames.into_iter().map(|(_, d)| d).collect(),
    })
}
