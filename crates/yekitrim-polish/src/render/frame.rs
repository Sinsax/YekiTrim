use anyhow::Context;
use std::io::Read;
use std::process::{Command, Stdio};

/// 已解码的 RGBA 视频帧。
#[derive(Debug, Clone)]
pub struct VideoFrame {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// 后端无关的输入帧。
///
/// - ChromeBackend 优先消费 `jpeg`（`need_rgba = false`，零回归路径）；
/// - `rgba` 现在只有基准/诊断在用（当年的 WgpuBackend 已删除），`jpeg` 也一起留着供对拍。
#[derive(Debug, Clone)]
pub struct RenderFrame {
    pub width: u32,
    pub height: u32,
    pub jpeg: Option<Vec<u8>>,
    pub rgba: Option<Vec<u8>>,
}

/// 一帧及其在整片中的绝对帧号/时间。
#[derive(Debug, Clone)]
pub struct TimedFrame {
    pub index: usize,
    pub t: f64,
    pub frame: RenderFrame,
}

/// 从 ffmpeg 管道提取 chunk 内所有帧。
///
/// `need_rgba=true` 时会把 JPEG 解码为 RGBA（wgpu 路径需要）；
/// Chrome 路径可传 `false`，保持现有的 JPEG → base64 → 页面解码流程。
pub fn extract_chunk_frames(
    video_path: &str,
    start_sec: f64,
    end_sec: f64,
    fps: usize,
    need_rgba: bool,
) -> anyhow::Result<Vec<TimedFrame>> {
    let start_frame = (start_sec * fps as f64).ceil() as usize;
    let end_frame = (end_sec * fps as f64).ceil() as usize;
    let frame_count = end_frame - start_frame;
    if frame_count == 0 {
        anyhow::bail!("Empty chunk");
    }

    let chunk_dur = end_sec - start_sec;
    let ff_extract = Command::new("ffmpeg")
        .args([
            "-ss",
            &start_sec.to_string(),
            "-i",
            video_path,
            "-t",
            &chunk_dur.to_string(),
            "-r",
            &fps.to_string(),
            "-f",
            "image2pipe",
            "-vcodec",
            "mjpeg",
            "-q:v",
            "2",
            "-frames:v",
            &frame_count.to_string(),
            "-",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .context("spawn ffmpeg frame extractor")?;

    let mut raw = Vec::new();
    ff_extract
        .stdout
        .unwrap()
        .read_to_end(&mut raw)
        .context("read ffmpeg frame pipe")?;

    let jpegs = split_jpeg_frames(&raw);
    if jpegs.is_empty() {
        anyhow::bail!("ffmpeg produced 0 frames");
    }

    let mut frames = Vec::with_capacity(jpegs.len());
    for (i, jpeg) in jpegs.into_iter().enumerate() {
        let (width, height, rgba) = if need_rgba {
            let decoded = decode_jpeg_rgba(&jpeg).context("decode JPEG frame")?;
            (decoded.width, decoded.height, Some(decoded.rgba))
        } else {
            (0, 0, None)
        };
        frames.push(TimedFrame {
            index: start_frame + i,
            t: (start_frame + i) as f64 / fps as f64,
            frame: RenderFrame {
                width,
                height,
                jpeg: Some(jpeg),
                rgba,
            },
        });
    }

    Ok(frames)
}

/// 按 FFD8/FFD9 分割 JPEG 流。
pub fn split_jpeg_frames(raw: &[u8]) -> Vec<Vec<u8>> {
    let mut frames = Vec::new();
    let mut pos = 0;
    while pos + 1 < raw.len() {
        if raw[pos] == 0xFF && raw[pos + 1] == 0xD8 {
            let start = pos;
            pos += 2;
            while pos + 1 < raw.len() {
                if raw[pos] == 0xFF && raw[pos + 1] == 0xD9 {
                    pos += 2;
                    frames.push(raw[start..pos].to_vec());
                    break;
                }
                pos += 1;
            }
        } else {
            pos += 1;
        }
    }
    frames
}

/// 用 image crate 将 JPEG 解码为 RGBA。
fn decode_jpeg_rgba(jpeg: &[u8]) -> anyhow::Result<VideoFrame> {
    use std::io::Cursor;

    let reader = image::ImageReader::new(Cursor::new(jpeg))
        .with_guessed_format()
        .context("guess JPEG format")?;
    let img = reader.decode().context("decode JPEG")?;
    let rgba = img.to_rgba8();
    Ok(VideoFrame {
        width: rgba.width(),
        height: rgba.height(),
        rgba: rgba.into_raw(),
    })
}
