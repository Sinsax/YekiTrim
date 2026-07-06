use std::path::Path;
use std::process::Command;
use std::collections::HashMap;
use base64::Engine;

use crate::project::ProjectFile;
use crate::render::chrome::ChromeRenderer;
use crate::render::ffmpeg::Ffmpeg;
use crate::{assets, cover, process, project, render};

pub async fn cmd_render(project_path: &Path) -> anyhow::Result<()> {
    // 自动先跑 process（幂等）
    if let Err(e) = process::cmd_process(project_path).await {
        tracing::info!("  ⚠ Process before render: {}", e);
    }
    tracing::info!("  Rendering pipeline...");
    let start_time = std::time::Instant::now();

    let config = project::load_project(project_path)?;
    let resolved = resolve_paths(project_path, &config);
    for (label, p) in [("Video", &resolved.video), ("SRT", &resolved.subtitles)] {
        if !Path::new(p).exists() { anyhow::bail!("{} not found: {}", label, p); }
    }
    tracing::info!("  Output: {}", resolved.output);

    let duration = std::fs::read_to_string(project_path.join("data").join("clip-meta.json")).ok()
        .and_then(|raw| serde_json::from_str::<serde_json::Value>(&raw).ok())
        .and_then(|v| v.get("duration").and_then(|d| d.as_f64()))
        .or_else(|| config.duration)
        .unwrap_or(0.0);
    if duration <= 0.0 { anyhow::bail!("Video duration unknown"); }
    tracing::info!("  Duration: {:.1}s", duration);

    let fps = 30;
    let parallelism = std::env::var("VTEDIT_PARALLELISM").ok().and_then(|v| v.parse().ok()).unwrap_or(4).max(1);
    let frame_count = (duration * fps as f64).ceil() as usize;
    tracing::info!("    {} frames at {} fps", frame_count, fps);

    // 启动单 HTTP server（供 sound data 提取 + 所有 chunk 使用）
    tracing::info!("\n  -- Server --");
    let injected_html = Some(render::server::RenderServer::build_html(project_path));
    let server_port = render::server::start_file_server(project_path.to_path_buf(), injected_html.clone()).await?;
    let server_url = format!("http://127.0.0.1:{}/?capture=1", server_port);

    // 提取 sound data
    tracing::info!("  -- Sound data --");
    let sfx_data = export_sound_data(&server_url).await?;
    tracing::info!("  v {} sound events", sfx_data.len());

    // 预解码 emoji GIF 帧
    tracing::info!("\n  -- Emoji GIF --");
    let emoji_data_json = prepare_emoji_data(project_path, &config);
    let emoji_count: usize = serde_json::from_str::<serde_json::Value>(&emoji_data_json)
        .map(|v| v.as_object().map(|o| o.len()).unwrap_or(0))
        .unwrap_or(0);
    tracing::info!("  v {} emojis with animated frames", emoji_count);

    tracing::info!("\n  -- Render chunks ({}) --", parallelism);
    let enc = Ffmpeg::detect_encoder();
    tracing::info!("  Encoder: {}", enc.codec);

    let chunk_dur = duration / parallelism as f64;
    let mut handles = vec![];
    for ci in 0..parallelism {
        let start_sec = ci as f64 * chunk_dur;
        let end_sec = ((ci + 1) as f64 * chunk_dur).min(duration);
        if start_sec >= duration { break; }
        let out = project_path.join(format!("_chunk_{}.mp4", ci));
        let cc = enc.codec.clone();
        let ca = enc.args.clone();
        let url = server_url.clone();
        let vp = resolved.video.clone();
        let edj = emoji_data_json.clone();
        handles.push(std::thread::spawn(move || {
            render_chunk_sync(&url, start_sec, end_sec, &out, &vp, &edj, &cc, &ca, fps, ci)
        }));
    }

    let mut chunk_files = vec![];
    for h in handles {
        match h.join() {
            Ok(Ok(f)) => chunk_files.push(f),
            Ok(Err(e)) => anyhow::bail!("Chunk: {}", e),
            Err(_) => anyhow::bail!("Chunk panicked"),
        }
    }

    tracing::info!("\n  -- Concat --");
    let tv = project_path.join("_temp_video.mp4");
    let cs: Vec<String> = chunk_files.iter().map(|p| p.to_string_lossy().to_string()).collect();
    Ffmpeg::concat(&cs, &tv.to_string_lossy())?;
    for f in &chunk_files { std::fs::remove_file(f).ok(); }

    tracing::info!("\n  -- Audio --");
    let ta = project_path.join("_temp_audio.aac");
    if let Err(e) = Ffmpeg::extract_audio(&resolved.video, &ta.to_string_lossy()) {
        let no_audio = format!("{}.noaudio.mp4", resolved.output.trim_end_matches(".mp4"));
        tracing::info!("  ⚠ Audio extraction failed: {}\n  → saving video without audio: {}", e, no_audio);
        std::fs::rename(&tv, &no_audio)?;
        print_final_stats(&no_audio, start_time);
        cleanup(project_path, &tv, &ta);
        return Ok(());
    }

    if !sfx_data.is_empty() {
        tracing::info!("  -- SFX mix ({}) --", sfx_data.len());
        let ma = project_path.join("_temp_audio_m.aac");
        let se: Vec<_> = sfx_data.iter().map(|sd| {
            let vol = sd.volume.unwrap_or(config.sfx.volume);
            let f = project_path.join("sfx").join(&sd.file.replace("sfx/", ""));
            render::ffmpeg::SfxEvent { time: sd.time, file: f.to_string_lossy().to_string(), volume: vol, duration: sd.duration }
        }).filter(|e| Path::new(&e.file).exists()).collect();
        if !se.is_empty() {
            Ffmpeg::mix_sfx(&ta.to_string_lossy(), &se, &ma.to_string_lossy())?;
            std::fs::rename(&ma, &ta)?;
        }
    }

    tracing::info!("\n  -- Mux --");
    let op = if Path::new(&resolved.output).is_absolute() { resolved.output.clone() }
             else { project_path.join(&resolved.output).to_string_lossy().to_string() };
    if let Err(e) = Ffmpeg::mux(&tv.to_string_lossy(), &ta.to_string_lossy(), &op) {
        let no_audio = format!("{}.noaudio.mp4", op.trim_end_matches(".mp4"));
        tracing::info!("  ⚠ Mux failed: {}\n  → saving video without audio: {}", e, no_audio);
        std::fs::rename(&tv, &no_audio)?;
    }
    cleanup(project_path, &tv, &ta);
    // 自动生成封面
    if let Err(e) = cover::cmd_cover(project_path, Some(&op)).await {
        tracing::info!("  ⚠ Cover generation skipped: {}", e);
    }
    print_final_stats(&op, start_time);
    Ok(())
}


pub fn render_chunk_sync(
    url: &str,
    start_sec: f64,
    end_sec: f64,
    output: &Path,
    video_path: &str,
    emoji_data_json: &str,
    codec: &str,
    codec_args: &[String],
    fps: usize,
    chunk_idx: usize,
) -> anyhow::Result<std::path::PathBuf> {
    let start_frame = (start_sec * fps as f64).ceil() as usize;
    let end_frame = (end_sec * fps as f64).ceil() as usize;
    let frame_count = end_frame - start_frame;
    if frame_count == 0 { anyhow::bail!("Empty chunk"); }

    let chunk_dur = end_sec - start_sec;
    tracing::debug!("  Chunk #{}: {:.1}s-{:.1}s ({} frames)", chunk_idx, start_sec, end_sec, frame_count);

    // ---- ffmpeg 帧提取 (pipe, 不写磁盘) ----
    tracing::debug!("    chunk#{}: extracting {} frames via ffmpeg...", chunk_idx, frame_count);
    let ff_extract = Command::new("ffmpeg")
        .args(["-ss", &start_sec.to_string(), "-i", video_path,
               "-t", &chunk_dur.to_string(),
               "-r", &fps.to_string(),
               "-f", "image2pipe", "-vcodec", "mjpeg", "-q:v", "2",
               "-frames:v", &frame_count.to_string(), "-"])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()?;
    use std::io::Read;

    // 读取 pipe 中所有 JPEG 字节, 按 FFD8/FFD9 分割帧
    let mut raw = Vec::new();
    ff_extract.stdout.unwrap().read_to_end(&mut raw)?;
    let mut jpeg_frames: Vec<Vec<u8>> = Vec::new();
    let mut pos = 0;
    while pos + 1 < raw.len() {
        if raw[pos] == 0xFF && raw[pos+1] == 0xD8 {
            let start = pos;
            pos += 2;
            while pos + 1 < raw.len() {
                if raw[pos] == 0xFF && raw[pos+1] == 0xD9 {
                    pos += 2;
                    jpeg_frames.push(raw[start..pos].to_vec());
                    break;
                }
                pos += 1;
            }
        } else {
            pos += 1;
        }
    }
    tracing::debug!("    chunk#{}: got {} frames from pipe", chunk_idx, jpeg_frames.len());
    if jpeg_frames.is_empty() {
        anyhow::bail!("ffmpeg produced 0 frames");
    }

    // ---- 启动 Chrome ----
    let browser = ChromeRenderer::launch()?;
    let tab = ChromeRenderer::create_tab(&browser, url)?;
    tab.set_default_timeout(std::time::Duration::from_secs(600));
    ChromeRenderer::wait_for_ready(&tab)?;

    // Reset + 发送帧 + 发送 emoji GIF 数据到浏览器
    tab.evaluate("window.__CAPTURE_RESET()", false)?;
    if !emoji_data_json.is_empty() && emoji_data_json != "{}" {
        tab.evaluate(&format!("window.__EMOJI_DATA = {}", emoji_data_json), false)?;
    }
        let b64_engine = base64::engine::general_purpose::STANDARD;
    let batch_size = 50;
    for (bi, chunk) in jpeg_frames.chunks(batch_size).enumerate() {
        let batch_b64: Vec<String> = chunk.iter()
            .map(|d| b64_engine.encode(d))
            .collect();
        let json = serde_json::to_string(&batch_b64)?;
        tab.evaluate(&format!("window.__LOAD_FRAMES({})", json), false)?;
        let pct = ((bi * batch_size + chunk.len()) as f64 / jpeg_frames.len() as f64 * 100.0) as usize;
        tracing::debug!("    chunk#{}: sent frames {:.0}%", chunk_idx, pct);
    }
    tracing::debug!("    chunk#{}: all frames sent", chunk_idx);

    // ---- 输出 ffmpeg (接收渲染后的帧) ----
    let mut ffmpeg_out = Command::new("ffmpeg")
        .args(["-y", "-f", "image2pipe", "-framerate", &fps.to_string(), "-i", "-",
               "-c:v", codec]).args(codec_args)
        .args(["-pix_fmt", "yuv420p", &output.to_string_lossy()])
        .stdin(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null()).stdout(std::process::Stdio::null())
        .spawn()?;
    let mut stdin = ffmpeg_out.stdin.take().unwrap();
    use std::io::Write;

    // 启动批量渲染
    tab.evaluate(&format!(
        "window.__CAPTURE_CHUNK({}, {}, {}).catch(function(e){{ window.__CAPTURE_ERROR = e.message; }})",
        start_frame, end_frame, fps,
    ), false)?;
    let chunk_start = std::time::Instant::now();

    // 轮询直到完成
    loop {
        let done_val = tab.evaluate("window.__CAPTURE_DONE === true", false)?;
        let done = done_val.value.and_then(|v| v.as_bool()).unwrap_or(false);
        if done { break; }

        let err_val = tab.evaluate("window.__CAPTURE_ERROR", false)?;
        let err = err_val.value.and_then(|v| v.as_str().map(|s| s.to_string()));
        if let Some(e) = err {
            if !e.is_empty() { anyhow::bail!("Render error: {}", e); }
        }

        let pct_val = tab.evaluate("window.__CAPTURE_PROGRESS || '0'", false)?;
        let pct = pct_val.value.and_then(|v| v.as_str().map(|s| s.to_string())).unwrap_or_default();
        let elapsed = chunk_start.elapsed().as_secs_f64();
        tracing::info!("    chunk#{}: {}%  {}s", chunk_idx, pct, elapsed as usize);

        std::thread::sleep(std::time::Duration::from_millis(1500));
    }
    tracing::info!("    chunk#{}: render complete, reading frames...", chunk_idx);

    // 批量读帧, pipe 到输出 ffmpeg
    let batch_size_read = 20;
    let mut offset = 0;
    let mut written = 0u32;
    while offset < frame_count {
        let js = format!("window.__GET_FRAMES({}, {})", offset, offset + batch_size_read);
        let result = tab.evaluate(&js, false)?;
        let json_str = result.value
            .and_then(|v| v.as_str().map(|s| s.to_string()))
            .unwrap_or_else(|| "[]".into());
        let frames: Vec<String> = serde_json::from_str(&json_str).unwrap_or_default();
        for b64 in &frames {
            stdin.write_all(&b64_engine.decode(b64.as_bytes())?)?;
            written += 1;
        }
        offset += batch_size_read;
        let elapsed = chunk_start.elapsed().as_secs_f64();
        let pct = offset.min(frame_count) as f64 / frame_count as f64 * 100.0;
        tracing::info!("    chunk#{}: mux {:.0}%  {} frames  {:.0}s",
            chunk_idx, pct, written, elapsed);
    }

    stdin.flush()?;
    drop(stdin);
    ffmpeg_out.wait()?;

    let elapsed = chunk_start.elapsed().as_secs_f64();
    tracing::info!("    chunk#{}: done  {} frames  {:.0}s", chunk_idx, written, elapsed);
    Ok(output.to_path_buf())
}
#[derive(serde::Deserialize, Debug)]
#[allow(dead_code)]
pub struct SoundEvent {
    time: f64,
    file: String,
    #[serde(rename = "type")]
    r#type: String,
    #[serde(default)]
    volume: Option<f64>,
    #[serde(default)]
    duration: Option<f64>,
}

pub async fn export_sound_data(url: &str) -> anyhow::Result<Vec<SoundEvent>> {
    let browser = ChromeRenderer::launch()?;
    let tab = ChromeRenderer::create_tab(&browser, url)?;
    tab.evaluate("window.__RNG_SEED=42;", false)?;

    // 等待 __TIMELINE 和 __SOUND_DATA
    for _ in 0..50 {
        let has_data = tab.evaluate("typeof window.__SOUND_DATA !== 'undefined'", false);
        if let Ok(val) = has_data {
            if val.value.map(|v| v.as_bool().unwrap_or(false)).unwrap_or(false) {
                let result = tab.evaluate("JSON.stringify(window.__SOUND_DATA||[])", false)?;
                let json = result.value
                    .and_then(|v| v.as_str().map(|s| s.to_string()))
                    .unwrap_or_else(|| "[]".into());
                let data: Vec<SoundEvent> = serde_json::from_str(&json)?;
                std::mem::drop(browser);
                return Ok(data);
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    }
    std::mem::drop(browser);
    Ok(vec![])
}

// ============================================================
// process
// ============================================================

pub struct ResolvedPaths {
    pub video: String,
    pub subtitles: String,
    pub danmaku: String,
    pub metadata: String,
    pub output: String,
}

pub fn resolve_paths(project_path: &Path, config: &ProjectFile) -> ResolvedPaths {
    let to_abs = |relative: &str| -> String {
        if relative.is_empty() {
            return String::new();
        }
        let p = Path::new(relative);
        if p.is_relative() {
            project_path.join(p).to_string_lossy().to_string()
        } else {
            p.to_string_lossy().to_string()
        }
    };

    ResolvedPaths {
        video: to_abs(&config.source.video),
        subtitles: to_abs(&config.source.subtitles),
        danmaku: to_abs(&config.source.danmaku),
        metadata: to_abs(&config.source.metadata),
        output: to_abs(&config.project.output),
    }
}

// ============================================================
// 辅助
// ============================================================
/// 注入资产路径到 config JSON (_assets 字段)
pub fn inject_assets(project_path: &Path, config_json: &str) -> String {
    match serde_json::from_str::<serde_json::Value>(config_json) {
        Ok(mut cfg) => {
            let downloader = assets::downloader::AssetDownloader::new(project_path);
            cfg["_assets"] = downloader.asset_map();
            serde_json::to_string(&cfg).unwrap_or_else(|_| config_json.to_string())
        }
        Err(_) => config_json.to_string(),
    }
}

/// 预解码所有 emoji GIF 为 PNG 帧，返回 JSON（按 name→{frames, delays}）
pub fn prepare_emoji_data(project_path: &Path, config: &project::ProjectFile) -> String {
        let b64_engine = base64::engine::general_purpose::STANDARD;
    let mut data: HashMap<String, serde_json::Value> = HashMap::new();
    for seg in &config.segments {
        for ev in &seg.events {
            if let project::Event::Emoji { name: Some(name), .. } = ev {
                if name.is_empty() || data.contains_key(name.as_str()) { continue; }
                let gif_path = project_path.join("assets").join("emoji").join(format!("{}.gif", name));
                if !gif_path.exists() { continue; }
                if let Ok(frames) = render::gif::decode_gif_frames(&gif_path) {
                    let frame_b64: Vec<String> = frames.iter().map(|f| b64_engine.encode(&f.png)).collect();
                    let delays: Vec<u16> = frames.iter().map(|f| f.delay_ms).collect();
                    data.insert(name.clone(), serde_json::json!({
                        "frames": frame_b64,
                        "delays": delays,
                    }));
                }
            }
        }
    }
    serde_json::to_string(&data).unwrap_or_else(|_| "{}".into())
}

/// 用 ffprobe 获取视频文件实际时长 (秒)
/// clip.json 的 duration 是 B站原始元数据含 padding，不可信
pub fn get_video_duration(video_path: &str) -> Option<f64> {
    let out = std::process::Command::new("ffprobe")
        .args(["-v", "error", "-show_entries", "format=duration", "-of", "csv=p=0", video_path])
        .output()
        .ok()?;
    if !out.status.success() { return None; }
    let s = std::str::from_utf8(&out.stdout).ok()?.trim().to_string();
    s.parse::<f64>().ok().filter(|d| *d > 0.0)
}

/// 递归复制目录
pub fn copy_dir_all(src: &Path, dst: &Path) -> std::io::Result<()> {
    if !dst.exists() { std::fs::create_dir_all(dst)?; }
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        if ty.is_dir() {
            copy_dir_all(&entry.path(), &dst.join(entry.file_name()))?;
        } else {
            std::fs::copy(entry.path(), dst.join(entry.file_name()))?;
        }
    }
    Ok(())
}

pub fn cleanup(_project_path: &Path, temp_video: &Path, temp_audio: &Path) {
    for p in [temp_video, temp_audio] {
        std::fs::remove_file(p).ok();
    }
}

pub fn print_final_stats(output: &str, start: std::time::Instant) {
    let elapsed = start.elapsed().as_secs_f64();
    let size = std::fs::metadata(output)
        .map(|m| m.len() as f64 / 1048576.0)
        .unwrap_or(0.0);
    tracing::info!(" FINISHED: {} ({:.1}MB, {:.0}s)", output, size, elapsed);
}