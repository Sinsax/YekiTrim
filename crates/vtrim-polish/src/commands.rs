use crate::{cover, process, rendering};

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::project::ProjectFile;
use crate::{llm, parse, project, render};


// init
// ============================================================
pub async fn cmd_init(path: &Path) -> anyhow::Result<()> {
    tracing::info!("📁  Initializing project at: {}", path.display());
    let config_path = path.join("polish.toml");
    if config_path.exists() {
        anyhow::bail!("polish.toml already exists at {}", config_path.display());
    }
    std::fs::create_dir_all(path)?;
    std::fs::create_dir_all(path.join("data"))?;
    std::fs::create_dir_all(path.join("sfx"))?;

    let mut example = ProjectFile::example();

    // 自动检测 clip.json (来自 vtrim cut)
    let clip_json = path.join("clip.json");
    if clip_json.exists() {
        if let Ok(raw) = std::fs::read_to_string(&clip_json) {
            if let Ok(meta) = serde_json::from_str::<serde_json::Value>(&raw) {
                if let Some(title) = meta.get("title").and_then(|v| v.as_str()) {
                    example.project.name = title.to_string();
                    tracing::info!("  ✓ Detected title: {}", title);
                }
                if let Some(dur) = meta.get("duration").and_then(|v| v.as_f64()) {
                    example.duration = Some(dur);
                    tracing::info!("  ✓ Detected duration: {:.0}s", dur);
                }
                // 自动设置素材路径
                example.source.video = "clip.mp4".into();
                example.source.subtitles = "clip.srt".into();
                example.source.danmaku = "clip.dm.srt".into();
                example.source.metadata = "clip.json".into();
                tracing::info!("  ✓ Auto-set source paths (clip.mp4, clip.srt, clip.dm.srt)");
            }
        }
    }

    let toml_str = example.to_toml_commented()?;
    std::fs::write(&config_path, toml_str)?;
    tracing::info!("  ✓ Created polish.toml");

    tracing::info!("  ✓ Created data/  (for parsed output)");
    tracing::info!("  ✓ Created sfx/   (for sound effects)");

    // 自动解析素材文件
    tracing::info!("");
    if let Err(e) = cmd_parse(path).await {
        tracing::info!("  ⚠ Parse 警告: {} (请检查素材文件)", e);
    }

    tracing::info!("");
    tracing::info!("Next steps:");
    tracing::info!("  1. Edit .vtrim/llm.toml — add your LLM API key");
    tracing::info!("  2. Run: vtrim polish (full pipeline)");
    tracing::info!("  3. Run: vtrim polish --preview  (or vtrim polish -i <dir>)");
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
            tracing::info!("  ⚠ {}", w);
        }
    }

    tracing::info!("📄  Parsing assets...");
    let resolved = rendering::resolve_paths(project_path, &config);
    tracing::info!("  SRT:      {}", &config.source.subtitles);
    tracing::info!("  Danmaku:  {}", &config.source.danmaku);

    let subtitles = parse::srt::parse_file(&resolved.subtitles)?;
    let danmaku = if resolved.danmaku.is_empty() {
        vec![]
    } else {
        let mut raw = parse::danmaku::parse_file(&resolved.danmaku)?;
        parse::text::assign_danmaku_tracks(&mut raw);
        raw
    };

    tracing::info!("  ✓ {} subtitles", subtitles.len());
    tracing::info!("  ✓ {} danmaku entries", danmaku.len());

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
    let actual_duration = rendering::get_video_duration(&resolved.video).unwrap_or(0.0);
    let duration = if actual_duration > 0.0 {
        if clip_duration > 0.0 && (clip_duration - actual_duration).abs() > 1.0 {
            tracing::info!("  ⚠ clip.json duration ({:.0}s) != actual video ({:.0}s), using actual", clip_duration, actual_duration);
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

    // 更新 polish.toml 的 project.name（从 clip.json title 同步）
    if !title.is_empty() {
        let mut cfg = project::load_project(project_path)?;
        cfg.project.name = title.clone();
        let toml_str = cfg.to_toml_commented()?;
        std::fs::write(project_path.join("polish.toml"), toml_str)?;
        tracing::info!("  ✓ polish.toml: project.name ← {}", title);
    }

    tracing::info!("\n✓ Parse complete. data/subtitles.json, data/danmaku.json, data/clip-meta.json");
    if duration > 0.0 {
        tracing::info!("  Duration: {:.1}s | Title: {} | Type: {}", duration, title, event_type);
    }
    Ok(())
}

// ============================================================
// llm
// ============================================================
pub async fn cmd_llm(project_path: &Path) -> anyhow::Result<()> {
    tracing::info!("🤖  LLM config generation...");

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
        tracing::info!("  ⚠ No LLM API key configured. Edit .vtrim/llm.toml");
        return Ok(());
    }
    tracing::info!("  Model: {}", llm_cfg.model);
    tracing::info!("  Base:  {}", llm_cfg.base_url);

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
    let video_path = rendering::resolve_paths(project_path, &config).video;
    let video_duration = rendering::get_video_duration(&video_path)
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
            tracing::info!("  ⚠ Removed {} segment(s) beyond video duration ({:.0}s)", before - after, video_duration);
        }
    }
    new_config.filter.danmaku = dm_filter;
    new_config.filter.highlight = hl_words;

    let toml_str = new_config.to_toml_commented()?;
    std::fs::write(project_path.join("polish.toml"), toml_str)?;

    tracing::info!("\n✓ polish.toml updated with LLM-generated segments and keywords");
    Ok(())
}

// ============================================================
// render
// ============================================================

pub async fn cmd_preview(project_path: &Path, mode: &str) -> anyhow::Result<()> {
    tracing::info!("👁️  Preview mode");

    // 先自动跑 process（幂等）
    if let Err(e) = process::cmd_process(project_path).await {
        tracing::info!("  ⚠ Process: {}", e);
    }
    process::ensure_assets(project_path).await;

    let config = project::load_project(project_path)?;
    let data_dir = project_path.join("data");
    if !data_dir.join("subtitles.json").exists() {
        tracing::info!("  ⚠ No parsed data found. Run `vtrim polish init` first.");
    }

    let show_video = mode.is_empty() || mode == "video";
    let show_cover = mode.is_empty() || mode == "cover";

    // 启动 video server
    let video_url = if show_video {
        let mut server = render::server::RenderServer::new(project_path.to_path_buf());
        let port = server.start().await?;
        let url = format!("http://127.0.0.1:{}", port);
        tracing::info!("  Video: {}", url);
        Some(url)
    } else {
        None
    };

    // 启动 cover server
    let cover_url = if show_cover {
        let resolved = rendering::resolve_paths(project_path, &config);
        let cover_image = project_path.join("_cover_frame.jpg");
        if !cover_image.exists() {
            let video_path = &resolved.video;
            let duration = config.segments.last().map(|s| s.end).unwrap_or(0.0);
            let frame_time = (duration * config.cover.frame_pos).max(0.1);
            if let Ok(frame_data) = std::process::Command::new("ffmpeg")
                .args(["-ss", &frame_time.to_string(), "-i", video_path,
                       "-vframes", "1", "-f", "image2pipe", "-vcodec", "mjpeg", "-q:v", "3", "-"])
                .stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::null())
                .output().map(|o| o.stdout)
            {
                if !frame_data.is_empty() { std::fs::write(&cover_image, &frame_data).ok(); }
            }
        }
        let html = cover::build_cover_html(project_path, &config, &resolved);
        let port = render::server::start_file_server(project_path.to_path_buf(), Some(html)).await?;
        let url = format!("http://127.0.0.1:{}/", port);
        tracing::info!("  Cover: {}", url);
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
            Ok(_) => tracing::info!("  ✓ Opened: {}", url),
            Err(e) => tracing::info!("  ⚠ Cannot open {}: {}", url, e),
        }
    }

    tracing::info!("\n  Press Ctrl+C to stop all servers.");
    loop {
        tokio::time::sleep(std::time::Duration::from_secs(3600)).await;
    }
}

// ============================================================
// test — 自包含测试环境：生成目录 + 素材 + 配置 → process → preview
// ============================================================
pub async fn cmd_test(project_path: &Path) -> anyhow::Result<()> {
    tracing::info!("🧪  Play Test mode");

    // 确定工作目录：有 polish.toml 就用项目目录，否则创建 test_workspace/
    let work = if project_path.join("polish.toml").exists() {
        tracing::info!("  📁 使用项目目录: {}", project_path.display());
        project_path.to_path_buf()
    } else {
        let ws = project_path.join("test_workspace");
        if !ws.exists() {
            std::fs::create_dir_all(&ws)?;
            std::fs::create_dir_all(ws.join("data"))?;
            std::fs::create_dir_all(ws.join("sfx"))?;
            std::fs::create_dir_all(ws.join("assets").join("emoji"))?;
            std::fs::create_dir_all(ws.join("assets").join("irasu"))?;
            tracing::info!("  📁 创建测试目录: {}", ws.display());
        }
        ws
    };

    // 找视频源：项目配置已有 → 父目录 video_cut → 同目录 clip.mp4 → ffmpeg 生成
    let _has_video = if project_path.join("polish.toml").exists() {
        let cfg = project::load_project(project_path)?;
        let v = cfg.source.video.clone();
        let vp = if Path::new(&v).is_relative() {
            project_path.join(&v)
        } else { PathBuf::from(&v) };
        if vp.exists() {
            // 复制到工作目录作为 clip.mp4
            std::fs::copy(&vp, work.join("clip.mp4"))?;
            tracing::info!("  ✓ 视频已复制到工作目录");
            true
        } else { false }
    } else {
        // 先在已知位置找已有视频
        let mut found = false;
        for c in [project_path.join("../video_cut/clip.mp4"), project_path.join("clip.mp4")] {
            if c.exists() {
                std::fs::copy(&c, work.join("clip.mp4"))?;
                tracing::info!("  ✓ 视频已复制到工作目录");
                found = true;
                break;
            }
        }
        if !found {
            // ffmpeg 生成 15s 黑底测试视频
            tracing::info!("  🎬 生成测试视频 (ffmpeg)...");
            let out_vid = work.join("clip.mp4");
            let ok = std::process::Command::new("ffmpeg")
                .args(["-f", "lavfi", "-i", "color=c=black:s=1920x1080:d=15",
                       "-c:v", "libx264", "-pix_fmt", "yuv420p",
                       &out_vid.to_string_lossy()])
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false);
            if ok { tracing::info!("  ✓ 测试视频已生成: clip.mp4"); }
            else { tracing::info!("  ⚠ 无法生成测试视频（需要 ffmpeg）"); }
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
    tracing::info!("  ✓ 测试字幕/弹幕/元数据已生成");

    // —— 2. 生成 polish.toml ——
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
    let cfg_path = work.join("polish.toml");
    std::fs::write(&cfg_path, &cfg)?;
    tracing::info!("  ✓ polish.toml 已生成");

    // —— 3. run process ——
    if let Err(e) = process::cmd_process(&work).await {
        tracing::info!("  ⚠ Process 步骤警告 (继续): {}", e);
    }

    // —— 4. run preview ——
    tracing::info!("\n  ── 启动测试预览 ──");
    cmd_preview(&work, "video").await
}

// ============================================================
// doctor
// ============================================================

pub fn cmd_doctor(_project_path: &Path) -> anyhow::Result<()> {
    tracing::info!("🔍  System check...\n");

    // Check ffmpeg
    match Command::new("ffmpeg").arg("-version").output() {
        Ok(out) => {
            let ver = String::from_utf8_lossy(&out.stdout);
            let first = ver.lines().next().unwrap_or("unknown");
            tracing::info!("  ✓ ffmpeg: {}", first);
        }
        Err(_) => tracing::info!("  ✗ ffmpeg: NOT FOUND"),
    }

    // Check Chrome
    let chrome_check = if cfg!(target_os = "windows") {
        Command::new("where").arg("chrome").output()
    } else {
        Command::new("which").arg("google-chrome").output()
    };
    if chrome_check.map(|o| o.status.success()).unwrap_or(false) {
        tracing::info!("  ✓ Chrome: found in PATH");
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
            tracing::info!("  ✓ Chrome: found (common path)");
        } else {
            tracing::info!("  ✗ Chrome: NOT FOUND in PATH or common locations");
        }
    }

    // Check LLM config
    let llm_cfg = vtrim_core::config::load_llm_config();
    if !llm_cfg.api_key.is_empty() && llm_cfg.api_key != "sk-your-api-key-here" {
        tracing::info!("  ✓ LLM: model={}, base={}", llm_cfg.model, llm_cfg.base_url);
    } else {
        tracing::info!("  ⚠ LLM: no API key in .vtrim/llm.toml");
    }

    tracing::info!("\n  vtrim-polish v{}", env!("CARGO_PKG_VERSION"));
    Ok(())
}

// ============================================================
// 路径解析
// ============================================================

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
