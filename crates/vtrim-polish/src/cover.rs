use std::path::Path;

use crate::project;
use crate::render::{chrome::ChromeRenderer, server};
use crate::rendering::{ResolvedPaths, resolve_paths};


const COVER_HTML: &str = include_str!("../templates/cover.html");

// ============================================================

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
    std::fs::write(project_path.join("polish.toml"), toml_str)?;
    tracing::info!("  ✓ [cover] preset: {}", template);
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
    let port = server::start_file_server(project_path.to_path_buf(), Some(html)).await?;
    let url = format!("http://127.0.0.1:{}", port);
    tracing::info!("  Cover URL: {}", url);
    std::process::Command::new("cmd").args(["/c", "start", "", &url]).spawn().ok();
    loop { tokio::time::sleep(std::time::Duration::from_secs(3600)).await; }
}

pub fn build_cover_html(_project_path: &Path, config: &project::ProjectFile, _resolved: &ResolvedPaths) -> String {
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
        clean.split(['，', '。', '！', '？', '、', ',', '.', '!'])
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


pub async fn cmd_cover(project_path: &Path, existing_video: Option<&str>) -> anyhow::Result<()> {
    tracing::info!("\n  -- Cover --");
    let config = project::load_project(project_path)?;
    let resolved = resolve_paths(project_path, &config);
    let cc = &config.cover;

    // 首次运行时自动写入 [cover] 节
    let raw_config = std::fs::read_to_string(project_path.join("polish.toml")).unwrap_or_default();
    if !raw_config.contains("[cover]") {
        let toml_str = config.to_toml_commented()?;
        std::fs::write(project_path.join("polish.toml"), toml_str)?;
        tracing::info!("  ✓ polish.toml: [cover] section added");
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
        tracing::info!("    using custom image: {}", cc.image);
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
    let _sub_color = &config.style.subtitle.color;
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
        "none" => "none".into(),
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
            .split(['，', '。', '！', '？', '、', ',', '.', '!'])
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .collect();
        if lines.len() > 1 {
            // 写入 polish.toml (仅首次)
            let mut cfg = config.clone();
            cfg.cover.title_lines = lines.clone();
            let toml_str = cfg.to_toml_commented()?;
            std::fs::write(project_path.join("polish.toml"), toml_str)?;
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

    let port = server::start_file_server(project_path.to_path_buf(), Some(cover_html)).await?;
    let url = format!("http://127.0.0.1:{}/", port);

    let browser = ChromeRenderer::launch()?;
    let tab = ChromeRenderer::create_tab(&browser, &url)?;
    tab.set_default_timeout(std::time::Duration::from_secs(30));

    for _ in 0..100 {
        let ready = tab.evaluate("window.__COVER_READY === true", false)?;
        if ready.value.and_then(|v| v.as_bool()).unwrap_or(false) { break; }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }

        let jpeg_data = tab.call_method(Page::CaptureScreenshot {
        format: Some(Page::CaptureScreenshotFormatOption::Jpeg),
        quality: Some(85),
        clip: None,
        from_surface: Some(true),
        capture_beyond_viewport: Some(true),
        optimize_for_speed: Some(false),
    })?;
    use base64::Engine;
use headless_chrome::protocol::cdp::Page;
    let jpeg = base64::engine::general_purpose::STANDARD.decode(&jpeg_data.data)?;

    let cover_file = project_path.join("cover.jpg");
    std::fs::write(&cover_file, &jpeg)?;
    std::fs::remove_file(project_path.join("_cover_frame.jpg")).ok();
    tracing::info!("  ✓ Cover saved: {}", cover_file.display());
    Ok(())
}