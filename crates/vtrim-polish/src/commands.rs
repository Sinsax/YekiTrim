use std::path::{Path, PathBuf};
use std::process::Command;
use std::collections::HashMap;

use crate::project::{Event, ProjectFile};
use crate::render::chrome::ChromeRenderer;
use crate::render::ffmpeg::Ffmpeg;
use crate::{assets, llm, parse, project, render};

/// 内嵌的 index.html 模板
const INDEX_HTML: &str = include_str!("../templates/index.html");
/// 内嵌的 cover.html 模板
const COVER_HTML: &str = include_str!("../templates/cover.html");

// ============================================================
// init
// ============================================================
pub async fn cmd_init(path: &Path) -> anyhow::Result<()> {
    println!("📁  Initializing project at: {}", path.display());
    let config_path = path.join("config.toml");
    if config_path.exists() {
        anyhow::bail!("config.toml already exists at {}", config_path.display());
    }
    std::fs::create_dir_all(path)?;
    std::fs::create_dir_all(path.join("data"))?;
    std::fs::create_dir_all(path.join("sfx"))?;

    let example = ProjectFile::example();
    let toml_str = example.to_toml_commented()?;
    std::fs::write(&config_path, toml_str)?;
    println!("  ✓ Created config.toml");

    let llm_path = path.join("llm.toml");
    if !llm_path.exists() {
        std::fs::write(&llm_path, project::example_llm_toml())?;
        println!("  ✓ Created llm.toml");
    }

    println!("  ✓ Created data/  (for parsed output)");
    println!("  ✓ Created sfx/   (for sound effects)");

    // 自动解析素材文件
    println!("");
    if let Err(e) = cmd_parse(path).await {
        println!("  ⚠ Parse 警告: {} (请检查素材文件)", e);
    }

    println!("");
    println!("Next steps:");
    println!("  1. Edit llm.toml — add your LLM API key");
    println!("  2. Run: vtrim polish (full pipeline)");
    println!("  3. Run: vtrim polish --preview  (or vtrim polish -i <dir>)");
    Ok(())
}

// ============================================================
// parse
// ============================================================
pub async fn cmd_parse(project_path: &Path) -> anyhow::Result<()> {
    let config = project::load_project(project_path)?;
    let warnings = project::validate_project(&config, project_path);
    for w in &warnings {
        if w.contains("not found") {
            println!("  ⚠ {}", w);
        }
    }

    println!("📄  Parsing assets...");
    let resolved = resolve_paths(project_path, &config);
    println!("  SRT:      {}", &config.source.subtitles);
    println!("  Danmaku:  {}", &config.source.danmaku);

    let subtitles = parse::srt::parse_file(&resolved.subtitles)?;
    let danmaku = if resolved.danmaku.is_empty() {
        vec![]
    } else {
        let mut raw = parse::danmaku::parse_file(&resolved.danmaku)?;
        parse::text::assign_danmaku_tracks(&mut raw);
        raw
    };

    println!("  ✓ {} subtitles", subtitles.len());
    println!("  ✓ {} danmaku entries", danmaku.len());

    // 关键词高亮 (使用 config 中的 highlight_words)
    let subtitles: Vec<parse::srt::Subtitle> = subtitles.into_iter().map(|mut s| {
        if !config.filter.highlight.is_empty() {
            let words: Vec<String> = config.filter.highlight.iter().map(|w| w.to_string()).collect();
            s.html = parse::text::apply_highlights(&s.text, &words);
        } else {
            s.html = s.text.clone();
        }
        s
    }).collect();

    let data_dir = project_path.join("data");
    std::fs::create_dir_all(&data_dir)?;

    std::fs::write(
        data_dir.join("subtitles.json"),
        serde_json::to_string_pretty(&subtitles)?,
    )?;
    std::fs::write(
        data_dir.join("danmaku.json"),
        serde_json::to_string_pretty(&danmaku)?,
    )?;

    // 读取 clip.json (元数据), 生成 clip-meta.json
    let meta_path = if !resolved.metadata.is_empty() && Path::new(&resolved.metadata).exists() {
        Some(PathBuf::from(&resolved.metadata))
    } else {
        // 降级: 尝试项目根目录的 clip.json
        let fallback = project_path.join("clip.json");
        if fallback.exists() { Some(fallback) } else { None }
    };
    let clip_meta = match meta_path {
        Some(ref p) => {
            let raw = std::fs::read_to_string(p)?;
            serde_json::from_str::<serde_json::Value>(&raw).ok()
        }
        None => None,
    };

    let clip_duration = clip_meta.as_ref()
        .and_then(|m| m.get("duration").and_then(|v| v.as_f64()))
        .unwrap_or(0.0);
    // 用 ffprobe 获取实际视频时长 (clip.json 的 duration 是 B站原始元数据含padding，不可信)
    let actual_duration = get_video_duration(&resolved.video).unwrap_or(0.0);
    let duration = if actual_duration > 0.0 {
        if clip_duration > 0.0 && (clip_duration - actual_duration).abs() > 1.0 {
            println!("  ⚠ clip.json duration ({:.0}s) != actual video ({:.0}s), using actual", clip_duration, actual_duration);
        }
        actual_duration
    } else {
        clip_duration
    };
    let title = clip_meta.as_ref()
        .and_then(|m| m.get("title").and_then(|v| v.as_str()))
        .unwrap_or("")
        .to_string();
    let event_type = clip_meta.as_ref()
        .and_then(|m| m.get("event_type").and_then(|v| v.as_str()))
        .unwrap_or("")
        .to_string();

    let meta = serde_json::json!({
        "duration": duration,
        "title": title,
        "event_type": event_type,
        "subtitles_count": subtitles.len(),
        "danmaku_count": danmaku.len(),
    });
    std::fs::write(data_dir.join("clip-meta.json"), serde_json::to_string_pretty(&meta)?)?;

    // 更新 config.toml 的 project.name（从 clip.json title 同步）
    if !title.is_empty() {
        let mut cfg = project::load_project(project_path)?;
        cfg.project.name = title.clone();
        let toml_str = cfg.to_toml_commented()?;
        std::fs::write(project_path.join("config.toml"), toml_str)?;
        println!("  ✓ config.toml: project.name ← {}", title);
    }

    println!("\n✓ Parse complete. data/subtitles.json, data/danmaku.json, data/clip-meta.json");
    if duration > 0.0 {
        println!("  Duration: {:.1}s | Title: {} | Type: {}", duration, title, event_type);
    }
    Ok(())
}

// ============================================================
// llm
// ============================================================
pub async fn cmd_llm(project_path: &Path) -> anyhow::Result<()> {
    println!("🤖  LLM config generation...");

    let config = project::load_project(project_path)?;
    let data_dir = project_path.join("data");

    // 读取解析数据
    let subtitles: Vec<parse::srt::Subtitle> = {
        let p = data_dir.join("subtitles.json");
        if !p.exists() { anyhow::bail!("Run 'vtrim polish init' first — missing {}", p.display()); }
        serde_json::from_str(&std::fs::read_to_string(&p)?)?
    };

    let danmaku: Vec<parse::danmaku::DanmakuEntry> = {
        let p = data_dir.join("danmaku.json");
        if !p.exists() {
            vec![]
        } else {
            serde_json::from_str(&std::fs::read_to_string(&p)?)?
        }
    };

    // 从 .vtrim/llm.toml 读取 LLM 配置（与 vtrim 自身一致）
    let llm_cfg = vtrim_core::config::load_llm_config();
    if llm_cfg.api_key.is_empty() || llm_cfg.api_key == "sk-your-api-key-here" {
        println!("  ⚠ No LLM API key configured. Edit .vtrim/llm.toml");
        return Ok(());
    }
    println!("  Model: {}", llm_cfg.model);
    println!("  Base:  {}", llm_cfg.base_url);

    // 读取 event_type 用于运镜节奏
    let event_type: String = match std::fs::read_to_string(data_dir.join("clip-meta.json")) {
        Ok(raw) => serde_json::from_str::<serde_json::Value>(&raw).ok()
            .and_then(|v| v.get("event_type").and_then(|e| e.as_str().map(|s| s.to_string())))
            .unwrap_or_default(),
        Err(_) => String::new(),
    };

    // 运行 LLM pipeline
    let (segments, dm_filter, hl_words) = llm::run_llm_pipeline(
        &llm_cfg.api_key, &llm_cfg.base_url, &llm_cfg.model,
        &subtitles, &danmaku, &event_type,
    ).await?;

    // 截断 segments 到视频实际时长
    let video_path = resolve_paths(project_path, &config).video;
    let video_duration = get_video_duration(&video_path)
        .or_else(|| {
            // 降级: 从 clip-meta.json 读 (可能不准，B站元数据含padding)
            std::fs::read_to_string(data_dir.join("clip-meta.json")).ok()
                .and_then(|raw| serde_json::from_str::<serde_json::Value>(&raw).ok())
                .and_then(|v| v.get("duration").and_then(|d| d.as_f64()))
        })
        .unwrap_or(0.0);

    let mut new_config = config;
    new_config.segments = segments;
    if video_duration > 0.0 {
        let before = new_config.segments.len();
        project::clamp_segments_to_duration(&mut new_config.segments, video_duration);
        let after = new_config.segments.len();
        if after < before {
            println!("  ⚠ Removed {} segment(s) beyond video duration ({:.0}s)", before - after, video_duration);
        }
    }
    new_config.filter.danmaku = dm_filter;
    new_config.filter.highlight = hl_words;

    let toml_str = new_config.to_toml_commented()?;
    std::fs::write(project_path.join("config.toml"), toml_str)?;

    println!("\n✓ config.toml updated with LLM-generated segments and keywords");
    Ok(())
}

// ============================================================
// render
// ============================================================
pub async fn cmd_render(project_path: &Path) -> anyhow::Result<()> {
    // 自动先跑 process（幂等）
    if let Err(e) = cmd_process(project_path).await {
        println!("  ⚠ Process before render: {}", e);
    }
    println!("  Rendering pipeline...");
    let start_time = std::time::Instant::now();

    let config = project::load_project(project_path)?;
    let resolved = resolve_paths(project_path, &config);
    for (label, p) in [("Video", &resolved.video), ("SRT", &resolved.subtitles)] {
        if !Path::new(p).exists() { anyhow::bail!("{} not found: {}", label, p); }
    }
    println!("  Output: {}", resolved.output);

    let duration = std::fs::read_to_string(project_path.join("data").join("clip-meta.json")).ok()
        .and_then(|raw| serde_json::from_str::<serde_json::Value>(&raw).ok())
        .and_then(|v| v.get("duration").and_then(|d| d.as_f64()))
        .or_else(|| config.duration)
        .unwrap_or(0.0);
    if duration <= 0.0 { anyhow::bail!("Video duration unknown"); }
    println!("  Duration: {:.1}s", duration);

    let fps = 30;
    let parallelism = std::env::var("VTEDIT_PARALLELISM").ok().and_then(|v| v.parse().ok()).unwrap_or(4).max(1);
    let frame_count = (duration * fps as f64).ceil() as usize;
    println!("    {} frames at {} fps", frame_count, fps);

    // 启动单 HTTP server（供 sound data 提取 + 所有 chunk 使用）
    println!("\n  -- Server --");
    let injected_html = Some(render::server::RenderServer::build_html(project_path));
    let server_port = render::server::start_file_server(project_path.to_path_buf(), injected_html.clone()).await?;
    let server_url = format!("http://127.0.0.1:{}/?capture=1", server_port);

    // 提取 sound data
    println!("  -- Sound data --");
    let sfx_data = export_sound_data(&server_url).await?;
    println!("  v {} sound events", sfx_data.len());

    // 预解码 emoji GIF 帧
    println!("\n  -- Emoji GIF --");
    let emoji_data_json = prepare_emoji_data(project_path, &config);
    let emoji_count: usize = serde_json::from_str::<serde_json::Value>(&emoji_data_json)
        .map(|v| v.as_object().map(|o| o.len()).unwrap_or(0))
        .unwrap_or(0);
    println!("  v {} emojis with animated frames", emoji_count);

    println!("\n  -- Render chunks ({}) --", parallelism);
    let enc = Ffmpeg::detect_encoder();
    println!("  Encoder: {}", enc.codec);

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

    println!("\n  -- Concat --");
    let tv = project_path.join("_temp_video.mp4");
    let cs: Vec<String> = chunk_files.iter().map(|p| p.to_string_lossy().to_string()).collect();
    Ffmpeg::concat(&cs, &tv.to_string_lossy())?;
    for f in &chunk_files { std::fs::remove_file(f).ok(); }

    println!("\n  -- Audio --");
    let ta = project_path.join("_temp_audio.aac");
    if let Err(e) = Ffmpeg::extract_audio(&resolved.video, &ta.to_string_lossy()) {
        let no_audio = format!("{}.noaudio.mp4", resolved.output.trim_end_matches(".mp4"));
        println!("  ⚠ Audio extraction failed: {}\n  → saving video without audio: {}", e, no_audio);
        std::fs::rename(&tv, &no_audio)?;
        print_final_stats(&no_audio, start_time);
        cleanup(project_path, &tv, &ta);
        return Ok(());
    }

    if !sfx_data.is_empty() {
        println!("  -- SFX mix ({}) --", sfx_data.len());
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

    println!("\n  -- Mux --");
    let op = if Path::new(&resolved.output).is_absolute() { resolved.output.clone() }
             else { project_path.join(&resolved.output).to_string_lossy().to_string() };
    if let Err(e) = Ffmpeg::mux(&tv.to_string_lossy(), &ta.to_string_lossy(), &op) {
        let no_audio = format!("{}.noaudio.mp4", op.trim_end_matches(".mp4"));
        println!("  ⚠ Mux failed: {}\n  → saving video without audio: {}", e, no_audio);
        std::fs::rename(&tv, &no_audio)?;
    }
    cleanup(project_path, &tv, &ta);
    // 自动生成封面
    if let Err(e) = cmd_cover(project_path, Some(&op)).await {
        println!("  ⚠ Cover generation skipped: {}", e);
    }
    print_final_stats(&op, start_time);
    Ok(())
}

pub async fn cmd_cover_template(project_path: &Path, template: &str) -> anyhow::Result<()> {
    let mut config = project::load_project(project_path)?;
    match template {
        "center-text" => {
            config.cover.text_x = "50%".into();
            config.cover.text_width = 0.8;
            config.cover.gradient_dir = "radial".into();
            config.cover.vignette = 0.05;
        }
        "right-text" => {
            config.cover.text_x = "65%".into();
            config.cover.text_width = 0.62;
            config.cover.gradient_dir = "right".into();
        }
        "minimal" => {
            config.cover.text_x = "50%".into();
            config.cover.text_width = 0.7;
            config.cover.gradient_dir = "none".into();
            config.cover.vignette = 0.15;
        }
        _ => { // left-text / 未知: 恢复默认
            config.cover.text_x = "35%".into();
            config.cover.text_width = 0.62;
            config.cover.gradient_dir = "left".into();
            config.cover.vignette = 0.08;
        }
    }
    let toml_str = config.to_toml_commented()?;
    std::fs::write(project_path.join("config.toml"), toml_str)?;
    println!("  ✓ [cover] preset: {}", template);
    Ok(())
}

pub async fn cmd_preview_cover(project_path: &Path) -> anyhow::Result<()> {
    let config = project::load_project(project_path)?;
    let resolved = resolve_paths(project_path, &config);
    let cover_image = project_path.join("_cover_frame.jpg");
    if !cover_image.exists() {
        // 没有封面帧则截一张
        let video_path = &resolved.video;
        let duration = config.segments.last().map(|s| s.end).unwrap_or(0.0);
        let frame_time = (duration * config.cover.frame_pos).max(0.1);
        let data = std::process::Command::new("ffmpeg")
            .args(["-ss", &frame_time.to_string(), "-i", video_path,
                   "-vframes", "1", "-f", "image2pipe", "-vcodec", "mjpeg", "-q:v", "3", "-"])
            .stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::null())
            .output()?.stdout;
        if !data.is_empty() { std::fs::write(&cover_image, &data)?; }
    }

    let html = build_cover_html(project_path, &config, &resolved);
    let port = render::server::start_file_server(project_path.to_path_buf(), Some(html)).await?;
    let url = format!("http://127.0.0.1:{}", port);
    println!("  Cover URL: {}", url);
    std::process::Command::new("cmd").args(["/c", "start", "", &url]).spawn().ok();
    loop { tokio::time::sleep(std::time::Duration::from_secs(3600)).await; }
}

pub fn build_cover_html(project_path: &Path, config: &project::ProjectFile, resolved: &ResolvedPaths) -> String {
    // 从 cmd_cover 中提取相同逻辑 (略缩版)
    let cc = &config.cover;

    // liver 位置
    let def_origin = config.layout.liver.origin.as_deref().unwrap_or("65% 50%");
    let lp: Vec<f64> = def_origin.split_whitespace()
        .filter_map(|s| s.trim_end_matches('%').parse().ok()).collect();
    let lx = lp.first().copied().unwrap_or(65.0);
    let ly = lp.get(1).copied().unwrap_or(50.0);
    let zoom = if lx >= 50.0 { 1.4 } else { 1.3 };

    // 渐变
    let hl_color = &config.style.subtitle.highlight_color;
    let (gr, gg, gb) = if !hl_color.is_empty() && hl_color.starts_with('#') {
        let h = hl_color.trim_start_matches('#');
        let r = u8::from_str_radix(&h[0..2], 16).unwrap_or(220) as f32 / 255.0;
        let g = u8::from_str_radix(&h[2..4], 16).unwrap_or(189) as f32 / 255.0;
        let b = u8::from_str_radix(&h[4..6], 16).unwrap_or(160) as f32 / 255.0;
        let nr = ((1.0 - r) * 30.0) as u8;
        let ng = ((1.0 - g) * 25.0) as u8;
        let nb = ((1.0 - b) * 50.0) as u8;
        (nr.max(10), ng.max(10), nb.max(15))
    } else { (20, 10, 40) };
    let grad_from = format!("rgba({},{},{},0.55)", gr, gg, gb);
    let grad_to = format!("rgba({},{},{},0.15)", gr, gg, gb);
    let grad_bg = match cc.gradient_dir.as_str() {
        "right" => format!("linear-gradient(270deg,{} 0%,{} 50%,transparent 100%)", grad_from, grad_to),
        "top" => format!("linear-gradient(180deg,{} 0%,{} 50%,transparent 100%)", grad_from, grad_to),
        "bottom" => format!("linear-gradient(0deg,{} 0%,{} 50%,transparent 100%)", grad_from, grad_to),
        "radial" => format!("radial-gradient(ellipse at 30% 50%,{} 0%,{} 60%,transparent 100%)", grad_from, grad_to),
        _ if cc.gradient_dir == "none" => "none".into(),
        _ => format!("linear-gradient(90deg,{} 0%,{} 50%,transparent 100%)", grad_from, grad_to),
    };

    // 标题
    let raw = &config.project.name;
    let clean = raw.split('【').next().unwrap_or(raw);
    let hl_words: Vec<&str> = config.filter.highlight.iter().map(|s| s.as_str()).collect();
    let title_lines: Vec<String> = if !cc.title_lines.is_empty() {
        cc.title_lines.clone()
    } else {
        clean.split(|c: char| matches!(c, '，' | '。' | '！' | '？' | '、' | ',' | '.' | '!'))
            .map(|s| s.trim()).filter(|s| !s.is_empty()).map(|s| s.to_string()).collect()
    };
    let title_html = title_lines.iter().map(|line| {
        if hl_words.is_empty() { return line.to_string(); }
        let mut r = line.to_string();
        for w in &hl_words { if !w.is_empty() && r.contains(w) { r = r.replace(w, &format!("<span class=\"hl\">{}</span>", w)); } }
        r
    }).collect::<Vec<_>>().join("<br>");

    COVER_HTML
        .replace("__TITLE__", &title_html)
        .replace("__LIVER_X__", &format!("{}%", lx))
        .replace("__LIVER_Y__", &format!("{}%", ly))
        .replace("__ZOOM__", &zoom.to_string())
        .replace("__GRAD_BG__", &grad_bg)
        .replace("__VIGNETTE_OP__", &cc.vignette.to_string())
        .replace("__FONT_SIZE__", &format!("{}px", cc.font_size))
        .replace("__STROKE__", &format!("{}px", cc.stroke))
        .replace("__HL_COLOR__", &cc.hl_color)
        .replace("__TEXT_WIDTH__", &format!("{}%", (cc.text_width * 100.0) as u32))
        .replace("__TEXT_X__", &cc.text_x)
        .replace("__TEXT_Y__", &cc.text_y)
        .replace("__LS__", &cc.letter_spacing)
        .replace("__LH__", &cc.line_height.to_string())
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
    eprintln!("  Chunk #{}: {:.1}s-{:.1}s ({} frames)", chunk_idx, start_sec, end_sec, frame_count);

    // ---- ffmpeg 帧提取 (pipe, 不写磁盘) ----
    eprintln!("    chunk#{}: extracting {} frames via ffmpeg...", chunk_idx, frame_count);
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
    eprintln!("    chunk#{}: got {} frames from pipe", chunk_idx, jpeg_frames.len());
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
    use base64::Engine;
    let b64_engine = base64::engine::general_purpose::STANDARD;
    let batch_size = 50;
    for (bi, chunk) in jpeg_frames.chunks(batch_size).enumerate() {
        let batch_b64: Vec<String> = chunk.iter()
            .map(|d| b64_engine.encode(d))
            .collect();
        let json = serde_json::to_string(&batch_b64)?;
        tab.evaluate(&format!("window.__LOAD_FRAMES({})", json), false)?;
        let pct = ((bi * batch_size + chunk.len()) as f64 / jpeg_frames.len() as f64 * 100.0) as usize;
        eprintln!("    chunk#{}: sent frames {:.0}%", chunk_idx, pct);
    }
    eprintln!("    chunk#{}: all frames sent", chunk_idx);

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
        println!("    chunk#{}: {}%  {}s", chunk_idx, pct, elapsed as usize);

        std::thread::sleep(std::time::Duration::from_millis(1500));
    }
    println!("    chunk#{}: render complete, reading frames...", chunk_idx);

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
        println!("    chunk#{}: mux {:.0}%  {} frames  {:.0}s",
            chunk_idx, pct, written, elapsed);
    }

    stdin.flush()?;
    drop(stdin);
    ffmpeg_out.wait()?;

    let elapsed = chunk_start.elapsed().as_secs_f64();
    println!("    chunk#{}: done  {} frames  {:.0}s", chunk_idx, written, elapsed);
    Ok(output.to_path_buf())
}
#[derive(serde::Deserialize, Debug)]
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
pub async fn cmd_process(project_path: &Path) -> anyhow::Result<()> {
    println!("🔧  Post-processing...");

    let mut config = project::load_project(project_path)?;
    let data_dir = project_path.join("data");

    // 读取字幕数据 (用于后续处理)
    let subtitles: Vec<parse::srt::Subtitle> = {
        let p = data_dir.join("subtitles.json");
        if !p.exists() { anyhow::bail!("Run 'vtrim polish init' first — missing {}", p.display()); }
        serde_json::from_str(&std::fs::read_to_string(&p)?)?
    };

    // Step 1: 为每段生成 Camera event (如果还没有)
    for seg in &mut config.segments {
        let sd = seg.end - seg.start;
        if sd < 1.0 { continue; }
        if seg.events.iter().any(|e| matches!(e, crate::project::Event::Camera { .. })) { continue; }
        seg.events.push(crate::project::Event::Camera {
            time: seg.start, duration: sd,
            zoom: "中".into(), origin: "50% 50%".into(),
        });
    }

    // Step 1b: 根据视频时长限制 emoji/irasu 总数（平方根递减，时长越长增长越慢）
    {
        let total_duration = config.segments.last().map(|s| s.end).unwrap_or(0.0);
        let max_visuals = std::cmp::max(1, (total_duration / 7.0).sqrt().ceil() as usize);

        // 收集所有 emoji/irasu 事件的位置
        let mut visuals: Vec<(usize, usize)> = vec![];
        for (si, seg) in config.segments.iter().enumerate() {
            for (ei, ev) in seg.events.iter().enumerate() {
                if matches!(ev, crate::project::Event::Emoji { .. } | crate::project::Event::Irasu { .. }) {
                    visuals.push((si, ei));
                }
            }
        }

        if visuals.len() > max_visuals {
            // 均匀保留 max_visuals 个（首尾可能保留，中间均匀分布）
            let mut keep = std::collections::HashSet::new();
            for i in 0..max_visuals {
                let idx = if max_visuals == 1 {
                    visuals.len() / 2
                } else {
                    i * (visuals.len() - 1) / (max_visuals - 1)
                };
                keep.insert(visuals[idx]);
            }

            let removed = visuals.len() - max_visuals;
            for (si, seg) in config.segments.iter_mut().enumerate() {
                let old_events = std::mem::take(&mut seg.events);
                for (ei, ev) in old_events.into_iter().enumerate() {
                    if matches!(ev, crate::project::Event::Emoji { .. } | crate::project::Event::Irasu { .. }) {
                        if keep.contains(&(si, ei)) {
                            seg.events.push(ev);
                        }
                        // else: drop this visual event
                    } else {
                        seg.events.push(ev);
                    }
                }
            }
            println!("  ⚠ emoji/irasu {} 个超出限制({}), 删除 {} 个", visuals.len(), max_visuals, removed);
        }
    }

    // Step 2: 解析 emoji keyword → codepoint name
    {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(15))
            .user_agent("vtrim-polish/0.1")
            .build()
            .unwrap_or_default();
        let catalog = crate::assets::emoji_search::EmojiCatalog::fetch(&client).await.ok();
        let irasu_dir = project_path.join("assets").join("irasu");

        for seg in &mut config.segments {
            for event in &mut seg.events {
                match event {
                    crate::project::Event::Emoji { keyword: Some(kw), name, .. } if !kw.is_empty() => {
                        // 如果 name 还没有被设置，从 keyword 解析
                        let needs_name = match name {
                            Some(n) => n.is_empty() || n == "auto",
                            None => true,
                        };
                        if needs_name {
                            if let Some(ref cat) = catalog {
                                let results = cat.search(kw, 1);
                                if let Some(cp) = results.first() {
                                    *name = Some(cp.clone());
                                    println!("  ✓ emoji keyword「{}」→ codepoint {}", kw, cp);
                            }
                        }
                    }
                }
                    crate::project::Event::Irasu { keywords: Some(kws), name, .. } if !kws.is_empty() => {
                        let needs_name = match name {
                            Some(n) => n.is_empty() || n == "auto",
                            None => true,
                        };
                        if needs_name {
                            // irasu 下载后文件名为 {sanitized}_{n}.png
                            let kw = kws.first().map(|s| s.as_str()).unwrap_or("");
                            let stem = sanitize_irasu_name(kw);
                            *name = Some(format!("{}_1", stem));
                            println!("  ✓ irasu keyword「{}」→ name {}", kw, name.as_deref().unwrap_or_default());
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    // Step 2b: 计算 emoji/irasu 的 auto 位置
    // 根据 liver 位置动态计算 sticker_area: liver 在左→放右, liver 在右→放左, y 与 origin 对齐
    {
        let liver = &config.layout.liver;
        let parse_pct = |s: &str| -> f64 {
            s.trim_end_matches('%').parse::<f64>().unwrap_or(50.0)
        };
        // 解析 liver origin
        let origin_xy: Vec<f64> = liver.origin.as_deref().unwrap_or("50% 50%")
            .split_whitespace().filter_map(|s| parse_pct(s).into()).collect();
        let ox = *origin_xy.first().unwrap_or(&50.0);
        let oy = *origin_xy.get(1).unwrap_or(&50.0);
        // 计算 sticker_area: 相对 liver 的对侧, y 对齐 origin
        let (sx0, sx1) = if ox < 50.0 {
            ((ox + 8.0).min(90.0), 95.0) // 左→放右
        } else {
            (5.0, (ox - 8.0).max(5.0))   // 右→放左
        };
        let sy0 = (oy - 22.0).clamp(5.0, 75.0);
        let sy1 = (oy + 22.0).clamp(25.0, 95.0);

        for seg in &mut config.segments {
            for (idx, event) in seg.events.iter_mut().enumerate() {
                let (x, y) = match event {
                    crate::project::Event::Emoji { x, y, .. }
                    | crate::project::Event::Irasu { x, y, .. } => (x, y),
                    _ => continue,
                };
                let needs_x = x.trim() == "auto";
                let needs_y = y.trim() == "auto";
                if !needs_x && !needs_y { continue; }

                // 在 sticker_area 内随机散布，用种子保证每次不同但不重叠
                let seed = (idx as f64 * 3.7 + seg.start * 1.3).fract();
                let px = sx0 + seed * (sx1 - sx0 - 16.0) + 5.0;
                let py = sy0 + ((seed * 7.3).fract()) * (sy1 - sy0 - 16.0) + 5.0;

                if needs_x { *x = format!("{:.0}%", px.clamp(5.0, 95.0)); }
                if needs_y { *y = format!("{:.0}%", py.clamp(5.0, 95.0)); }
            }
        }

        // Step 2c: emoji/irasu duration 对齐到后一个字幕的结束时间
        for seg in &mut config.segments {
            for event in &mut seg.events {
                let et = event.get_time();
                let duration = match event {
                    crate::project::Event::Emoji { duration, .. }
                    | crate::project::Event::Irasu { duration, .. } => Some(duration),
                    _ => None,
                };
                if let Some(dur) = duration {
                    // 找下一个在该事件之后开始的字幕
                    let next_sub = subtitles.iter()
                        .find(|s| s.start > et + 0.1);
                    if let Some(ns) = next_sub {
                        let new_dur = (ns.end - et).max(1.0);
                        if (*dur - new_dur).abs() > 0.3 {
                            *dur = new_dur;
                        }
                    }
                }
            }
        }
        if config.segments.iter().any(|seg| seg.events.iter().any(|e| matches!(e, crate::project::Event::Emoji { .. } | crate::project::Event::Irasu { .. }))) {
            println!("  ✓ emoji/irasu 位置已计算 (sticker_area, liver对侧 y对齐origin)");
        }
    }

    // Step 3: 自动下载 keyword 对应的资产 + 按需 SFX（在解析 tag 之前，下载器会展开下载全部匹配 SFX）
    let downloader = assets::downloader::AssetDownloader::new(project_path);
    download_keyword_assets(&downloader, &config).await;

    // Step 4: 解析 tag 形式的 sfx 引用 → 实际 id（下载完成后执行，确保下载器已展开下载全部匹配 SFX）
    for seg in &mut config.segments {
        for event in &mut seg.events {
            resolve_event_sfx(event);
        }
    }

    let toml_str = config.to_toml_commented()?;
    std::fs::write(project_path.join("config.toml"), toml_str)?;
    println!("  ✓ config.toml updated");
    Ok(())
}

pub async fn cmd_cover(project_path: &Path, existing_video: Option<&str>) -> anyhow::Result<()> {
    println!("\n  -- Cover --");
    let config = project::load_project(project_path)?;
    let resolved = resolve_paths(project_path, &config);
    let cc = &config.cover;

    // 首次运行时自动写入 [cover] 节
    let raw_config = std::fs::read_to_string(project_path.join("config.toml")).unwrap_or_default();
    if !raw_config.contains("[cover]") {
        let toml_str = config.to_toml_commented()?;
        std::fs::write(project_path.join("config.toml"), toml_str)?;
        println!("  ✓ config.toml: [cover] section added");
    }

    // 背景图: 自定义 image 或 ffmpeg 截帧
    let duration = config.segments.last().map(|s| s.end).unwrap_or(0.0);
    if cc.image.is_empty() {
        let video_path = existing_video.unwrap_or(&resolved.video);
        let frame_time = (duration * cc.frame_pos).max(0.1);
        let frame_data = std::process::Command::new("ffmpeg")
            .args(["-ss", &frame_time.to_string(), "-i", video_path,
                   "-vframes", "1", "-f", "image2pipe", "-vcodec", "mjpeg", "-q:v", "3", "-"])
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .output()?.stdout;
        if frame_data.is_empty() { anyhow::bail!("Frame extraction failed"); }
        std::fs::write(project_path.join("_cover_frame.jpg"), &frame_data)?;
    } else {
        let img_path = project_path.join(&cc.image);
        if !img_path.exists() { anyhow::bail!("Cover image not found: {}", img_path.display()); }
        std::fs::copy(&img_path, project_path.join("_cover_frame.jpg"))?;
        println!("    using custom image: {}", cc.image);
    }

    // liver 位置 → 帧缩放中心
    let def_origin = config.layout.liver.origin.as_deref().unwrap_or("65% 50%");
    let lp: Vec<f64> = def_origin.split_whitespace()
        .filter_map(|s| s.trim_end_matches('%').parse().ok()).collect();
    let lx = lp.first().copied().unwrap_or(65.0);
    let ly = lp.get(1).copied().unwrap_or(50.0);
    // zoom: 让右侧 liver 区域放大
    let zoom = if lx >= 50.0 { 1.4 } else { 1.3 };

    // 渐变颜色: 基于字幕高亮色
    let hl_color = &config.style.subtitle.highlight_color;
    let sub_color = &config.style.subtitle.color;
    let (grad_r, grad_g, grad_b) = if !hl_color.is_empty() && hl_color.starts_with('#') {
        let h = hl_color.trim_start_matches('#');
        let r = u8::from_str_radix(&h[0..2], 16).unwrap_or(220) as f32 / 255.0;
        let g = u8::from_str_radix(&h[2..4], 16).unwrap_or(189) as f32 / 255.0;
        let b = u8::from_str_radix(&h[4..6], 16).unwrap_or(160) as f32 / 255.0;
        // complementary: invert and darken
        let nr = ((1.0 - r) * 30.0) as u8;
        let ng = ((1.0 - g) * 25.0) as u8;
        let nb = ((1.0 - b) * 50.0) as u8;
        (nr.max(10), ng.max(10), nb.max(15))
    } else {
        (20, 10, 40)
    };
    let grad_from = format!("rgba({},{},{},0.55)", grad_r, grad_g, grad_b);
    let grad_to = format!("rgba({},{},{},0.15)", grad_r, grad_g, grad_b);

    // 渐变背景 CSS
    let grad_bg = match cc.gradient_dir.as_str() {
        "right" => format!("linear-gradient(270deg,{} 0%,{} 50%,transparent 100%)", grad_from, grad_to),
        "top" => format!("linear-gradient(180deg,{} 0%,{} 50%,transparent 100%)", grad_from, grad_to),
        "bottom" => format!("linear-gradient(0deg,{} 0%,{} 50%,transparent 100%)", grad_from, grad_to),
        "radial" => format!("radial-gradient(ellipse at 30% 50%,{} 0%,{} 60%,transparent 100%)", grad_from, grad_to),
        _ => format!("linear-gradient(90deg,{} 0%,{} 50%,transparent 100%)", grad_from, grad_to),
    };

    // 标题: 先读 title_lines, 空则自动分行 + 写回 config
    let raw = &config.project.name;
    let clean = raw.split('【').next().unwrap_or(raw);
    let hl_words: Vec<&str> = config.filter.highlight.iter().map(|s| s.as_str()).collect();

    let title_lines: Vec<String> = if !cc.title_lines.is_empty() {
        cc.title_lines.clone()
    } else {
        let lines: Vec<String> = clean
            .split(|c: char| matches!(c, '，' | '。' | '！' | '？' | '、' | ',' | '.' | '!'))
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .collect();
        if lines.len() > 1 {
            // 写入 config.toml (仅首次)
            let mut cfg = config.clone();
            cfg.cover.title_lines = lines.clone();
            let toml_str = cfg.to_toml_commented()?;
            std::fs::write(project_path.join("config.toml"), toml_str)?;
        }
        lines
    };

    // 关键词高亮
    let title_html = title_lines.iter().map(|line| {
        if hl_words.is_empty() { return line.to_string(); }
        let mut result = line.to_string();
        for w in &hl_words {
            if !w.is_empty() && result.contains(w) {
                result = result.replace(w, &format!("<span class=\"hl\">{}</span>", w));
            }
        }
        result
    }).collect::<Vec<_>>().join("<br>");

    let cover_html = COVER_HTML
        .replace("__TITLE__", &title_html)
        .replace("__LIVER_X__", &format!("{}%", lx))
        .replace("__LIVER_Y__", &format!("{}%", ly))
        .replace("__ZOOM__", &zoom.to_string())
        .replace("__GRAD_BG__", &grad_bg)
        .replace("__VIGNETTE_OP__", &cc.vignette.to_string())
        .replace("__FONT_SIZE__", &format!("{}px", cc.font_size))
        .replace("__STROKE__", &format!("{}px", cc.stroke))
        .replace("__HL_COLOR__", &cc.hl_color)
        .replace("__TEXT_WIDTH__", &format!("{}%", (cc.text_width * 100.0) as u32))
        .replace("__TEXT_X__", &cc.text_x)
        .replace("__TEXT_Y__", &cc.text_y)
        .replace("__LS__", &cc.letter_spacing)
        .replace("__LH__", &cc.line_height.to_string());

    let port = render::server::start_file_server(project_path.to_path_buf(), Some(cover_html)).await?;
    let url = format!("http://127.0.0.1:{}/", port);

    let browser = ChromeRenderer::launch()?;
    let tab = ChromeRenderer::create_tab(&browser, &url)?;
    tab.set_default_timeout(std::time::Duration::from_secs(30));

    for _ in 0..100 {
        let ready = tab.evaluate("window.__COVER_READY === true", false)?;
        if ready.value.and_then(|v| v.as_bool()).unwrap_or(false) { break; }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }

    use headless_chrome::protocol::cdp::Page;
    let jpeg_data = tab.call_method(Page::CaptureScreenshot {
        format: Some(Page::CaptureScreenshotFormatOption::Jpeg),
        quality: Some(85),
        clip: None,
        from_surface: Some(true),
        capture_beyond_viewport: Some(true),
        optimize_for_speed: Some(false),
    })?;
    use base64::Engine;
    let jpeg = base64::engine::general_purpose::STANDARD.decode(&jpeg_data.data)?;

    let cover_file = project_path.join("cover.jpg");
    std::fs::write(&cover_file, &jpeg)?;
    std::fs::remove_file(project_path.join("_cover_frame.jpg")).ok();
    println!("  ✓ Cover saved: {}", cover_file.display());
    Ok(())
}

/// 清理 irasu keyword 为合法文件名
pub fn sanitize_irasu_name(keyword: &str) -> String {
    keyword
        .chars()
        .map(|c| if c.is_alphanumeric() || c == '_' || c == '-' { c } else { '_' })
        .collect::<String>()
        .trim_matches('_')
        .to_string()
}

/// 解析事件中的 tag 形式 sfx 引用 (如 "tag:コメディ" → "humorous-scene-1")
pub fn resolve_event_sfx(event: &mut crate::project::Event) {
    use crate::project::Event;
    match event {
        Event::Emoji { sfx: Some(s), .. }
        | Event::Irasu { sfx: Some(s), .. }
        | Event::Shake { sfx: Some(s), .. }
        | Event::Flash { sfx: Some(s), .. }
        | Event::Blur { sfx: Some(s), .. }
        | Event::ZoomBounce { sfx: Some(s), .. }
        | Event::Vignette { sfx: Some(s), .. }
        | Event::HueShift { sfx: Some(s), .. }
        | Event::Stutter { sfx: Some(s), .. }
        | Event::Split { sfx: Some(s), .. }
        | Event::Noise { sfx: Some(s), .. }
        | Event::Pulse { sfx: Some(s), .. }
        | Event::ColorShift { sfx: Some(s), .. }
        | Event::Overlay { sfx: Some(s), .. } => {
            if let Some(resolved) = resolve_sfx_tag(s) {
                *s = resolved;
            }
        }
        Event::Sfx { name, .. } => {
            if let Some(resolved) = resolve_sfx_tag(name) {
                *name = resolved;
            }
        }
        _ => {}
    }
}

/// 如果值是 "tag:xxx" 格式，解析为实际 SFX id，否则原样返回
pub fn resolve_sfx_tag(value: &str) -> Option<String> {
    let tag_prefix = "tag:";
    if !value.starts_with(tag_prefix) {
        return None; // 不是 tag 引用，不动
    }
    let tag = value.trim_start_matches(tag_prefix);
    if tag.is_empty() {
        return None;
    }

    let candidates = crate::assets::sfx_catalog::search_by_tags(&[tag.to_string()]);
    if let Some(entry) = candidates.first() {
        println!("  ✓ sfx tag「{}」→ id {}", tag, entry.id);
        Some(entry.id.clone())
    } else {
        println!("  ⚠ sfx tag「{}」未匹配到音效，使用原值", tag);
        None
    }
}

// ============================================================
// download
// ============================================================
pub async fn cmd_download(project_path: &Path) -> anyhow::Result<()> {
    println!("📥  Downloading assets...");
    let downloader = assets::downloader::AssetDownloader::new(project_path);

    let mut total_errors = 0;
    if let Some(cfg) = project::load_project(project_path).ok() {
        if !cfg.segments.is_empty() {
            total_errors += download_keyword_assets(&downloader, &cfg).await;
        }
    }

    if total_errors > 0 {
        println!("  ⚠ {} download errors", total_errors);
    }
    Ok(())
}

/// 下载 keyword 类型的资产（emoji/irasu/sfx）
pub async fn download_keyword_assets(downloader: &assets::downloader::AssetDownloader, config: &project::ProjectFile) -> usize {
    let mut total_errors = 0;

    let has_emoji = config.segments.iter().any(|seg| {
        seg.events.iter().any(|ev| matches!(ev, project::Event::Emoji { keyword: Some(kw), .. } if !kw.is_empty()))
    });
    if has_emoji {
        let r = downloader.download_emoji_keywords(&config.segments).await;
        if r.emoji_downloaded > 0 || r.emoji_existing > 0 {
            println!("  ✓ emoji: {} new, {} cached", r.emoji_downloaded, r.emoji_existing);
        }
        total_errors += r.errors.len();
    }

    let has_irasu = config.segments.iter().any(|seg| {
        seg.events.iter().any(|ev| matches!(ev, project::Event::Irasu { keywords: Some(kws), .. } if !kws.is_empty()))
    });
    if has_irasu {
        let r = downloader.download_irasu_keywords(&config.segments).await;
        if r.irasu_downloaded > 0 || r.irasu_existing > 0 {
            println!("  ✓ irasu: {} new, {} cached", r.irasu_downloaded, r.irasu_existing);
        }
        total_errors += r.errors.len();
    }

    let has_sfx = !assets::downloader::collect_sfx_ids(&config.segments).is_empty();
    if has_sfx {
        let r = downloader.download_sfx_for_segments(&config.segments).await;
        if r.sfx_downloaded > 0 || r.sfx_existing > 0 || !r.errors.is_empty() {
            println!("  ✓ sfx: {} new, {} cached, {} errors", r.sfx_downloaded, r.sfx_existing, r.errors.len());
        }
        total_errors += r.errors.len();
    }

    total_errors
}

/// Auto-download missing assets (called by preview/render)
pub async fn ensure_assets(project_path: &Path) {
    let downloader = assets::downloader::AssetDownloader::new(project_path);

    if let Ok(config) = project::load_project(project_path) {
        if !config.segments.is_empty() {
            let total_errors = download_keyword_assets(&downloader, &config).await;
            if total_errors > 0 {
                println!("  ⚠ {} download errors (will use CDN fallback where available)", total_errors);
            }
        }
    }
}

// ============================================================
// preview
// ============================================================
pub async fn cmd_preview(project_path: &Path, mode: &str) -> anyhow::Result<()> {
    println!("👁️  Preview mode");

    // 先自动跑 process（幂等）
    if let Err(e) = cmd_process(project_path).await {
        println!("  ⚠ Process: {}", e);
    }
    ensure_assets(project_path).await;

    let config = project::load_project(project_path)?;
    let data_dir = project_path.join("data");
    if !data_dir.join("subtitles.json").exists() {
        println!("  ⚠ No parsed data found. Run `vtrim polish init` first.");
    }

    let show_video = mode.is_empty() || mode == "video";
    let show_cover = mode.is_empty() || mode == "cover";

    // 启动 video server
    let video_url = if show_video {
        let mut server = render::server::RenderServer::new(project_path.to_path_buf());
        let port = server.start().await?;
        let url = format!("http://127.0.0.1:{}", port);
        println!("  Video: {}", url);
        Some(url)
    } else {
        None
    };

    // 启动 cover server
    let cover_url = if show_cover {
        let resolved = resolve_paths(project_path, &config);
        let html = build_cover_html(project_path, &config, &resolved);
        let port = render::server::start_file_server(project_path.to_path_buf(), Some(html)).await?;
        let url = format!("http://127.0.0.1:{}/", port);
        println!("  Cover: {}", url);
        Some(url)
    } else {
        None
    };

    // 打开浏览器
    for url in video_url.iter().chain(cover_url.iter()) {
        let opened = if cfg!(target_os = "windows") {
            std::process::Command::new("cmd")
                .args(["/c", "start", "", url])
                .spawn()
        } else if cfg!(target_os = "macos") {
            std::process::Command::new("open").arg(url).spawn()
        } else {
            std::process::Command::new("xdg-open").arg(url).spawn()
        };
        match opened {
            Ok(_) => println!("  ✓ Opened: {}", url),
            Err(e) => println!("  ⚠ Cannot open {}: {}", url, e),
        }
    }

    println!("\n  Press Ctrl+C to stop all servers.");
    loop {
        tokio::time::sleep(std::time::Duration::from_secs(3600)).await;
    }
}

// ============================================================
// test — 自包含测试环境：生成目录 + 素材 + 配置 → process → preview
// ============================================================
pub async fn cmd_test(project_path: &Path) -> anyhow::Result<()> {
    println!("🧪  Play Test mode");

    // 确定工作目录：有 config.toml 就用项目目录，否则创建 test_workspace/
    let work = if project_path.join("config.toml").exists() {
        println!("  📁 使用项目目录: {}", project_path.display());
        project_path.to_path_buf()
    } else {
        let ws = project_path.join("test_workspace");
        if !ws.exists() {
            std::fs::create_dir_all(&ws)?;
            std::fs::create_dir_all(ws.join("data"))?;
            std::fs::create_dir_all(ws.join("sfx"))?;
            std::fs::create_dir_all(ws.join("assets").join("emoji"))?;
            std::fs::create_dir_all(ws.join("assets").join("irasu"))?;
            println!("  📁 创建测试目录: {}", ws.display());
        }
        ws
    };

    // 找视频源：项目配置已有 → 父目录 video_cut → 同目录 clip.mp4 → ffmpeg 生成
    let has_video = if project_path.join("config.toml").exists() {
        let cfg = project::load_project(project_path)?;
        let v = cfg.source.video.clone();
        let vp = if Path::new(&v).is_relative() {
            project_path.join(&v)
        } else { PathBuf::from(&v) };
        if vp.exists() {
            // 复制到工作目录作为 clip.mp4
            std::fs::copy(&vp, work.join("clip.mp4"))?;
            println!("  ✓ 视频已复制到工作目录");
            true
        } else { false }
    } else {
        // 先在已知位置找已有视频
        let mut found = false;
        for c in [project_path.join("../video_cut/clip.mp4"), project_path.join("clip.mp4")] {
            if c.exists() {
                std::fs::copy(&c, work.join("clip.mp4"))?;
                println!("  ✓ 视频已复制到工作目录");
                found = true;
                break;
            }
        }
        if !found {
            // ffmpeg 生成 15s 黑底测试视频
            println!("  🎬 生成测试视频 (ffmpeg)...");
            let out_vid = work.join("clip.mp4");
            let ok = std::process::Command::new("ffmpeg")
                .args(["-f", "lavfi", "-i", "color=c=black:s=1920x1080:d=15",
                       "-c:v", "libx264", "-pix_fmt", "yuv420p",
                       &out_vid.to_string_lossy()])
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false);
            if ok { println!("  ✓ 测试视频已生成: clip.mp4"); }
            else { println!("  ⚠ 无法生成测试视频（需要 ffmpeg）"); }
            found = ok;
        }
        found
    };

    let dur = 15.0;

    // —— 1. 测试素材 ——
    let sub_txt = "1\n00:00:00,000 --> 00:00:12,000\n测试字幕 游戏效果验证\n";
    std::fs::write(work.join("test_subtitles.srt"), sub_txt)?;

    let dm_txt = "1\n00:00:00,000 --> 00:00:12,000\n{\\move(2000,0,0,0)}{\\c&H4FEDFF}测试弹幕 游戏\n";
    std::fs::write(work.join("test_danmaku.dm.srt"), dm_txt)?;

    // 解析字幕/弹幕 → data/
    let sub_entries = crate::parse::srt::parse_file(
        &work.join("test_subtitles.srt").to_string_lossy())?;
    std::fs::write(work.join("data").join("subtitles.json"),
        serde_json::to_string_pretty(&sub_entries)?)?;

    let dm_entries = crate::parse::danmaku::parse_file(
        &work.join("test_danmaku.dm.srt").to_string_lossy())?;
    std::fs::write(work.join("data").join("danmaku.json"),
        serde_json::to_string_pretty(&dm_entries)?)?;

    let meta = serde_json::json!({
        "duration": dur, "title": "Play Test",
        "event_type": "test",
        "subtitles_count": sub_entries.len(),
        "danmaku_count": dm_entries.len(),
    });
    std::fs::write(work.join("data").join("clip-meta.json"),
        serde_json::to_string_pretty(&meta)?)?;
    println!("  ✓ 测试字幕/弹幕/元数据已生成");

    // —— 2. 生成 config.toml ——
    let cfg = r##"[project]
name = "Play Test"
output = "play-test.mp4"
created = "2026-07-04"
version = 1

[source]
video = "clip.mp4"
subtitles = "test_subtitles.srt"
danmaku = "test_danmaku.dm.srt"
metadata = ""

[filter]
danmaku = ["测试", "游戏"]
highlight = ["测试", "游戏"]

[style.subtitle]
font = "LXGW WenKai"
size = 72
color = "#dcbda0"
stroke = "12px #403c3b"
highlight_color = "#f56b41"
entrance = "fade-up"
position = "bottom"

[style.danmaku]
font_size = 42
color = "#ffffff"
stroke = "2px #000"
highlight_color = "#ff6bcb"
speed = 8.0
area = "danmaku_area"
scroll_direction = "rtl"

[sfx]
volume = 0.1
reactions_enabled = true
shake_enabled = true

[[segments]]
start = 0.0
end = 15.0
fx = "med"
note = "全类型效果测试"

[[segments.events]]
type = "camera"
time = 0.0
duration = 15.0
zoom = "中"
origin = "50% 50%"

[[segments.events]]
type = "emoji"
time = 1.0
duration = 2.0
keyword = "laugh"
x = "auto"
y = "auto"

[[segments.events]]
type = "irasu"
time = 3.0
duration = 2.5
keyword = "game"
x = "auto"
y = "auto"

[[segments.events]]
type = "shake"
time = 5.0
intensity = 10
sfx = "tag:surprise"

[[segments.events]]
type = "flash"
time = 7.0
color = "#ffffff"
duration = 0.3

[[segments.events]]
type = "blur"
time = 9.0

[[segments.events]]
type = "overlay"
time = 11.0
duration = 1.5
overlay_type = "color"
color = "rgba(255,50,50,0.3)"
opacity = 1.0

[[segments.events]]
type = "sfx"
time = 12.5
name = "tag:highlight"

[[segments.events]]
type = "pulse"
time = 14.0
"##;
    let cfg_path = work.join("config.toml");
    std::fs::write(&cfg_path, &cfg)?;
    println!("  ✓ config.toml 已生成");

    // —— 3. run process ——
    if let Err(e) = cmd_process(&work).await {
        println!("  ⚠ Process 步骤警告 (继续): {}", e);
    }

    // —— 4. run preview ——
    println!("\n  ── 启动测试预览 ──");
    cmd_preview(&work, "video").await
}

// ============================================================
// doctor
// ============================================================
pub fn cmd_doctor(project_path: &Path) -> anyhow::Result<()> {
    println!("🔍  System check...\n");

    // Check ffmpeg
    match Command::new("ffmpeg").arg("-version").output() {
        Ok(out) => {
            let ver = String::from_utf8_lossy(&out.stdout);
            let first = ver.lines().next().unwrap_or("unknown");
            println!("  ✓ ffmpeg: {}", first);
        }
        Err(_) => println!("  ✗ ffmpeg: NOT FOUND"),
    }

    // Check Chrome
    let chrome_check = if cfg!(target_os = "windows") {
        Command::new("where").arg("chrome").output()
    } else {
        Command::new("which").arg("google-chrome").output()
    };
    if chrome_check.map(|o| o.status.success()).unwrap_or(false) {
        println!("  ✓ Chrome: found in PATH");
    } else {
        // Try common paths
        let common_paths = if cfg!(target_os = "windows") {
            vec![
                r"C:\Program Files\Google\Chrome\Application\chrome.exe",
                r"C:\Program Files (x86)\Google\Chrome\Application\chrome.exe",
            ]
        } else {
            vec!["/usr/bin/google-chrome", "/usr/bin/chromium-browser"]
        };
        let found = common_paths.iter().any(|p| Path::new(p).exists());
        if found {
            println!("  ✓ Chrome: found (common path)");
        } else {
            println!("  ✗ Chrome: NOT FOUND in PATH or common locations");
        }
    }

    // Check LLM config
    let llm_cfg = vtrim_core::config::load_llm_config();
    if !llm_cfg.api_key.is_empty() && llm_cfg.api_key != "sk-your-api-key-here" {
        println!("  ✓ LLM: model={}, base={}", llm_cfg.model, llm_cfg.base_url);
    } else {
        println!("  ⚠ LLM: no API key in .vtrim/llm.toml");
    }

    println!("\n  vtrim-polish v{}", env!("CARGO_PKG_VERSION"));
    Ok(())
}

// ============================================================
// 路径解析
// ============================================================
pub struct ResolvedPaths {
    video: String,
    subtitles: String,
    danmaku: String,
    metadata: String,
    output: String,
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
    use base64::Engine;
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

pub fn cleanup(project_path: &Path, temp_video: &Path, temp_audio: &Path) {
    for p in [temp_video, temp_audio] {
        std::fs::remove_file(p).ok();
    }
}

pub fn print_final_stats(output: &str, start: std::time::Instant) {
    let elapsed = start.elapsed().as_secs_f64();
    let size = std::fs::metadata(output)
        .map(|m| m.len() as f64 / 1048576.0)
        .unwrap_or(0.0);
    println!("\n═══════════════════════════════════");
    println!("  ✅ {} ({:.1}MB, {:.0}s)", output, size, elapsed);
    println!("═══════════════════════════════════\n");
}
