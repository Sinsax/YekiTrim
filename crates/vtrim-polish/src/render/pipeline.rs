use base64::Engine;
use std::collections::HashMap;
use std::path::Path;
use std::process::Command;

use crate::project::ProjectFile;
use crate::render::backend::{RenderBackendKind, create_backend};
use crate::render::chrome::ChromeRenderer;
use crate::render::context::RenderContext;
use crate::render::ffmpeg::Ffmpeg;
use crate::render::frame::extract_chunk_frames;
use crate::render::sound::collect_sound_events;
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
        if !Path::new(p).exists() {
            anyhow::bail!("{} not found: {}", label, p);
        }
    }
    tracing::info!("  Output: {}", resolved.output);

    let duration = std::fs::read_to_string(project_path.join("data").join("clip-meta.json"))
        .ok()
        .and_then(|raw| serde_json::from_str::<serde_json::Value>(&raw).ok())
        .and_then(|v| v.get("duration").and_then(|d| d.as_f64()))
        .or(config.duration)
        .unwrap_or(0.0);
    if duration <= 0.0 {
        anyhow::bail!("Video duration unknown");
    }
    tracing::info!("  Duration: {:.1}s", duration);

    let fps = 30;
    let parallelism = std::env::var("VTEDIT_PARALLELISM")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(4)
        .max(1);
    let frame_count = (duration * fps as f64).ceil() as usize;
    tracing::info!("    {} frames at {} fps", frame_count, fps);

    // 选择渲染后端
    let renderer = std::env::var("VTEDIT_RENDERER")
        .ok()
        .and_then(|v| RenderBackendKind::parse(&v))
        .unwrap_or(RenderBackendKind::Chrome);
    tracing::info!("  Renderer: {}", renderer.as_str());

    // 网页服务器仅预览/Chrome 出片需要；wgpu 出片不需要
    let server_url = if renderer == RenderBackendKind::Chrome {
        tracing::info!("\n  -- Server --");
        let injected_html = Some(render::server::RenderServer::build_html(project_path));
        let server_port =
            render::server::start_file_server(project_path.to_path_buf(), injected_html.clone())
                .await?;
        Some(format!("http://127.0.0.1:{}/?capture=1", server_port))
    } else {
        None
    };

    // 提取 sound data：wgpu 路径用 Rust 纯函数，Chrome 路径仍走页面协议
    tracing::info!("  -- Sound data --");
    let sfx_data = if renderer == RenderBackendKind::Chrome {
        export_sound_data(server_url.as_deref().unwrap()).await?
    } else {
        collect_sound_events(&config)
    };
    tracing::info!("  v {} sound events", sfx_data.len());

    // 预解码 sticker GIF 帧
    tracing::info!("\n  -- Sticker GIF --");
    let sticker_data_json = prepare_sticker_data(project_path, &config);
    let sticker_count: usize = serde_json::from_str::<serde_json::Value>(&sticker_data_json)
        .map(|v| v.as_object().map(|o| o.len()).unwrap_or(0))
        .unwrap_or(0);
    tracing::info!("  v {} stickers with animated frames", sticker_count);

    tracing::info!("\n  -- Render chunks ({}) --", parallelism);
    let enc = Ffmpeg::detect_encoder();
    tracing::info!("  Encoder: {}", enc.codec);

    let chunk_dur = duration / parallelism as f64;
    let mut handles = vec![];
    for ci in 0..parallelism {
        let start_sec = ci as f64 * chunk_dur;
        let end_sec = ((ci + 1) as f64 * chunk_dur).min(duration);
        if start_sec >= duration {
            break;
        }
        let out = project_path.join(format!("_chunk_{}.mp4", ci));
        let cc = enc.codec.clone();
        let ca = enc.args.clone();
        let url = server_url.clone();
        let vp = resolved.video.clone();
        let sdj = sticker_data_json.clone();
        let cfg = config.clone();
        let pp = project_path.to_path_buf();
        let need_rgba = renderer == RenderBackendKind::Wgpu;
        let kind = renderer;
        handles.push(std::thread::spawn(move || {
            render_chunk_sync(
                &pp,
                &cfg,
                kind,
                url.as_deref(),
                start_sec,
                end_sec,
                &out,
                &vp,
                &sdj,
                &cc,
                &ca,
                fps,
                ci,
                need_rgba,
            )
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
    let cs: Vec<String> = chunk_files
        .iter()
        .map(|p| p.to_string_lossy().to_string())
        .collect();
    Ffmpeg::concat(&cs, &tv.to_string_lossy())?;
    for f in &chunk_files {
        std::fs::remove_file(f).ok();
    }

    tracing::info!("\n  -- Audio --");
    let ta = project_path.join("_temp_audio.aac");
    if let Err(e) = Ffmpeg::extract_audio(&resolved.video, &ta.to_string_lossy()) {
        let no_audio = format!("{}.noaudio.mp4", resolved.output.trim_end_matches(".mp4"));
        tracing::info!(
            "  ⚠ Audio extraction failed: {}\n  → saving video without audio: {}",
            e,
            no_audio
        );
        std::fs::rename(&tv, &no_audio)?;
        print_final_stats(&no_audio, start_time);
        cleanup(project_path, &tv, &ta);
        return Ok(());
    }

    if !sfx_data.is_empty() {
        tracing::info!("  -- SFX mix ({}) --", sfx_data.len());
        // 实时响度曲线 (ebur128 每 0.1s): 音效按所在时刻的视频响度动态调整
        let loudness_curve = measure_loudness_curve(&ta.to_string_lossy());
        // 段级响度作为曲线测量失败的兜底
        let seg_loudness: Vec<(f64, f64, Option<f64>)> = config
            .segments
            .iter()
            .map(|s| {
                let db = measure_segment_loudness(&ta.to_string_lossy(), s.start, s.end - s.start);
                (s.start, s.end, db)
            })
            .collect();
        if !loudness_curve.is_empty() {
            tracing::info!("  v 响度曲线 {} 个采样点 (实时跟随)", loudness_curve.len());
        }
        // LLM volume → 相对强度系数: 0.5=正常, 0.3=轻, 0.9=响 (影响 ±20%)
        let llm_factor =
            |v: Option<f64>| -> f64 { v.map(|x| (0.5 + x).clamp(0.4, 1.2)).unwrap_or(1.0) };
        let ma = project_path.join("_temp_audio_m.aac");
        // 通用响度对齐: 每个音效文件测量自身响度 → gain = 视频时刻响度 - 8dB - 文件响度
        // 无论音效文件本身多响多轻, 最终都落在「视频响度 - 8dB」, 且实时跟随视频
        let se: Vec<_> = sfx_data
            .iter()
            .map(|sd| {
                let f = project_path.join("sfx").join(sd.file.replace("sfx/", ""));
                let file_db = measure_audio_loudness(&f.to_string_lossy()).unwrap_or(-14.0);
                // 事件时刻的视频实时响度 (曲线优先, 段级兜底)
                let video_db = loudness_curve
                    .iter()
                    .rev()
                    .find(|(t, _)| sd.time >= *t)
                    .map(|(_, d)| *d)
                    .or_else(|| {
                        seg_loudness
                            .iter()
                            .find(|(s, e, _)| sd.time >= *s && sd.time < *e)
                            .and_then(|(_, _, d)| *d)
                    })
                    .unwrap_or(-26.0);
                // 目标: 音效最终响度 = 视频响度 - 8dB (点缀不盖人声); 再乘 LLM 相对系数
                let target = video_db - 8.0;
                let vol = 10.0_f64.powf((target - file_db) / 20.0) * llm_factor(sd.volume);
                let vol = vol.clamp(0.005, 1.0);
                render::ffmpeg::SfxEvent {
                    time: sd.time,
                    file: f.to_string_lossy().to_string(),
                    volume: vol,
                    duration: sd.duration,
                }
            })
            .filter(|e| Path::new(&e.file).exists())
            .collect();
        if !se.is_empty() {
            // 混音 + 限幅保护 (防止音效峰值爆音)
            Ffmpeg::mix_sfx(&ta.to_string_lossy(), &se, &ma.to_string_lossy())?;
            let lim = project_path.join("_temp_audio_l.aac");
            Ffmpeg::apply_limiter(&ma.to_string_lossy(), &lim.to_string_lossy())?;
            std::fs::rename(&lim, &ma)?;
            std::fs::rename(&ma, &ta)?;
        }
    }

    tracing::info!("\n  -- Mux --");
    let op = if Path::new(&resolved.output).is_absolute() {
        resolved.output.clone()
    } else {
        project_path
            .join(&resolved.output)
            .to_string_lossy()
            .to_string()
    };
    if let Err(e) = Ffmpeg::mux(&tv.to_string_lossy(), &ta.to_string_lossy(), &op) {
        let no_audio = format!("{}.noaudio.mp4", op.trim_end_matches(".mp4"));
        tracing::info!(
            "  ⚠ Mux failed: {}\n  → saving video without audio: {}",
            e,
            no_audio
        );
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

#[allow(clippy::too_many_arguments)]
pub fn render_chunk_sync(
    project_path: &Path,
    config: &ProjectFile,
    kind: RenderBackendKind,
    server_url: Option<&str>,
    start_sec: f64,
    end_sec: f64,
    output: &Path,
    video_path: &str,
    sticker_data_json: &str,
    codec: &str,
    codec_args: &[String],
    fps: usize,
    chunk_idx: usize,
    need_rgba: bool,
) -> anyhow::Result<std::path::PathBuf> {
    let start_frame = (start_sec * fps as f64).ceil() as usize;
    let end_frame = (end_sec * fps as f64).ceil() as usize;
    let frame_count = end_frame - start_frame;
    if frame_count == 0 {
        anyhow::bail!("Empty chunk");
    }

    tracing::debug!(
        "  Chunk #{}: {:.1}s-{:.1}s ({} frames)",
        chunk_idx,
        start_sec,
        end_sec,
        frame_count
    );

    // ---- ffmpeg 帧提取 (pipe, 不写磁盘) ----
    tracing::debug!(
        "    chunk#{}: extracting {} frames via ffmpeg...",
        chunk_idx,
        frame_count
    );
    let frames = extract_chunk_frames(video_path, start_sec, end_sec, fps, need_rgba)?;
    if frames.is_empty() {
        anyhow::bail!("ffmpeg produced 0 frames");
    }
    tracing::debug!(
        "    chunk#{}: got {} frames from pipe",
        chunk_idx,
        frames.len()
    );

    // ---- 后端渲染 ----
    let mut ctx = RenderContext::new(project_path.to_path_buf(), config.clone());
    ctx.fps = fps;
    ctx.server_url = server_url.map(|s| s.to_string());
    ctx.sticker_data_json = sticker_data_json.to_string();

    let mut backend = create_backend(kind, project_path, config)?;
    backend.prepare(&ctx)?;

    let rendered = backend.render_chunk(&frames)?;
    if rendered.len() != frames.len() {
        anyhow::bail!(
            "backend returned {} frames, expected {}",
            rendered.len(),
            frames.len()
        );
    }

    // ---- 输出 ffmpeg (接收渲染后的帧) ----
    let mut ffmpeg_out = Command::new("ffmpeg")
        .args([
            "-y",
            "-f",
            "image2pipe",
            "-framerate",
            &fps.to_string(),
            "-i",
            "-",
            "-c:v",
            codec,
        ])
        .args(codec_args)
        .args(["-pix_fmt", "yuv420p", &output.to_string_lossy()])
        .stdin(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .spawn()?;
    {
        use std::io::Write;
        let mut stdin = ffmpeg_out.stdin.take().unwrap();
        for jpeg in &rendered {
            stdin.write_all(jpeg)?;
        }
        stdin.flush()?;
    }
    ffmpeg_out.wait()?;

    tracing::info!("    chunk#{}: done  {} frames", chunk_idx, rendered.len());
    Ok(output.to_path_buf())
}

/// 与 `render::sound::SoundEvent` 相同；保留旧 pub 名避免破坏外部引用。
pub type SoundEvent = crate::render::sound::SoundEvent;

pub async fn export_sound_data(url: &str) -> anyhow::Result<Vec<SoundEvent>> {
    let browser = ChromeRenderer::launch()?;
    let tab = ChromeRenderer::create_tab(&browser, url)?;
    tab.evaluate("window.__RNG_SEED=42;", false)?;

    // 等待 __TIMELINE 和 __SOUND_DATA
    for _ in 0..50 {
        let has_data = tab.evaluate("typeof window.__SOUND_DATA !== 'undefined'", false);
        if let Ok(val) = has_data
            && val
                .value
                .map(|v| v.as_bool().unwrap_or(false))
                .unwrap_or(false)
        {
            let result = tab.evaluate("JSON.stringify(window.__SOUND_DATA||[])", false)?;
            let json = result
                .value
                .and_then(|v| v.as_str().map(|s| s.to_string()))
                .unwrap_or_else(|| "[]".into());
            let data: Vec<SoundEvent> = serde_json::from_str(&json)?;
            std::mem::drop(browser);
            return Ok(data);
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

/// 预解码所有 sticker 的 GIF 帧为 PNG，返回 JSON（按 name→{frames, delays}）
/// 仅 "emoji" 来源的 GIF 有动画帧，"irasu" 和 "local" 来源是静态图
pub fn prepare_sticker_data(project_path: &Path, config: &project::ProjectFile) -> String {
    let b64_engine = base64::engine::general_purpose::STANDARD;
    let mut data: HashMap<String, serde_json::Value> = HashMap::new();
    for seg in &config.segments {
        for ev in &seg.events {
            if let project::Event::Sticker {
                name: Some(name), ..
            } = ev
            {
                if name.is_empty() || data.contains_key(name.as_str()) {
                    continue;
                }
                let gif_path = project_path
                    .join("assets")
                    .join("stickers")
                    .join(format!("{}.gif", name));
                if !gif_path.exists() {
                    continue;
                }
                if let Ok(frames) = render::gif::decode_gif_frames(&gif_path) {
                    let frame_b64: Vec<String> =
                        frames.iter().map(|f| b64_engine.encode(&f.png)).collect();
                    let delays: Vec<u16> = frames.iter().map(|f| f.delay_ms).collect();
                    data.insert(
                        name.clone(),
                        serde_json::json!({
                            "frames": frame_b64,
                            "delays": delays,
                        }),
                    );
                }
            }
        }
    }
    serde_json::to_string(&data).unwrap_or_else(|_| "{}".into())
}

/// 用 ffprobe 获取视频文件实际时长 (秒)
/// clip.json 的 duration 是 B站原始元数据含 padding，不可信
/// 获取视频宽高 (ffprobe stream 信息), 竖屏检测用
pub fn get_video_size(video_path: &str) -> Option<(u32, u32)> {
    let out = std::process::Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-select_streams",
            "v:0",
            "-show_entries",
            "stream=width,height",
            "-of",
            "csv=p=0",
            video_path,
        ])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let s = std::str::from_utf8(&out.stdout).ok()?.trim().to_string();
    let mut parts = s.split(',');
    let w = parts.next()?.trim().parse::<u32>().ok()?;
    let h = parts.next()?.trim().parse::<u32>().ok()?;
    if w == 0 || h == 0 {
        return None;
    }
    Some((w, h))
}

pub fn get_video_duration(video_path: &str) -> Option<f64> {
    let out = std::process::Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-show_entries",
            "format=duration",
            "-of",
            "csv=p=0",
            video_path,
        ])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let s = std::str::from_utf8(&out.stdout).ok()?.trim().to_string();
    s.parse::<f64>().ok().filter(|d| *d > 0.0)
}

/// 递归复制目录
pub fn copy_dir_all(src: &Path, dst: &Path) -> std::io::Result<()> {
    if !dst.exists() {
        std::fs::create_dir_all(dst)?;
    }
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

/// 用 ffmpeg volumedetect 检测音频响度，返回 mean_volume (dB)
/// 检测音频某时间段的响度 (mean_volume dB) — 段级响度对齐用
pub fn measure_segment_loudness(audio_path: &str, start: f64, dur: f64) -> Option<f64> {
    if dur <= 0.0 {
        return None;
    }
    let output = Command::new("ffmpeg")
        .args([
            "-ss",
            &format!("{:.2}", start),
            "-t",
            &format!("{:.2}", dur),
            "-i",
            audio_path,
            "-af",
            "volumedetect",
            "-f",
            "null",
            "-",
        ])
        .stderr(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .output()
        .ok()?;
    let stderr = String::from_utf8_lossy(&output.stderr);
    for line in stderr.lines() {
        if let Some(val_str) = line.split("mean_volume: ").nth(1)
            && let Some(db_str) = val_str.trim().strip_suffix(" dB")
            && let Ok(db) = db_str.parse::<f64>()
        {
            return Some(db);
        }
    }
    None
}

/// 用 ebur128 测量音频瞬时响度曲线 (每 ~0.1s 一个点, LUFS)
/// 返回 [(时间, 响度)] — 音效按所在时刻的响度实时调整音量
pub fn measure_loudness_curve(audio_path: &str) -> Vec<(f64, f64)> {
    let output = Command::new("ffmpeg")
        .args([
            "-i",
            audio_path,
            "-af",
            "ebur128=peak=true",
            "-f",
            "null",
            "-",
        ])
        .stderr(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .output()
        .ok();
    let Some(output) = output else {
        return Vec::new();
    };
    let stderr = String::from_utf8_lossy(&output.stderr);
    let mut curve: Vec<(f64, f64)> = Vec::new();
    for line in stderr.lines() {
        // [Parsed_ebur128_0 @ ...] t: 0.399312  TARGET:-23 LUFS    M: -27.6
        let Some(t_pos) = line.find(" t: ") else {
            continue;
        };
        let Some(m_pos) = line.find(" M:") else {
            continue;
        };
        let t_str: String = line[t_pos + 4..]
            .split_whitespace()
            .next()
            .unwrap_or("")
            .to_string();
        let m_str: String = line[m_pos + 3..]
            .split_whitespace()
            .next()
            .unwrap_or("")
            .to_string();
        let (Ok(t), Ok(m)) = (t_str.parse::<f64>(), m_str.parse::<f64>()) else {
            continue;
        };
        // 跳过静音窗口 (-120 表示无信号)
        if m > -100.0 {
            curve.push((t, m));
        }
    }
    curve
}

/// 检测音频整体响度 (mean_volume dB)
pub fn measure_audio_loudness(audio_path: &str) -> Option<f64> {
    let output = Command::new("ffmpeg")
        .args(["-i", audio_path, "-af", "volumedetect", "-f", "null", "-"])
        .stderr(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .output()
        .ok()?;
    let stderr = String::from_utf8_lossy(&output.stderr);
    for line in stderr.lines() {
        if let Some(val_str) = line.split("mean_volume: ").nth(1)
            && let Some(db_str) = val_str.trim().strip_suffix(" dB")
            && let Ok(db) = db_str.parse::<f64>()
        {
            return Some(db);
        }
    }
    None
}

pub fn print_final_stats(output: &str, start: std::time::Instant) {
    let elapsed = start.elapsed().as_secs_f64();
    let size = std::fs::metadata(output)
        .map(|m| m.len() as f64 / 1048576.0)
        .unwrap_or(0.0);
    tracing::info!(" FINISHED: {} ({:.1}MB, {:.0}s)", output, size, elapsed);
}
