use std::path::Path;

use crate::{assets, parse, project};

pub async fn cmd_process(project_path: &Path) -> anyhow::Result<()> {
    tracing::info!("🔧  Post-processing...");

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
            tracing::info!("  ⚠ emoji/irasu {} 个超出限制({}), 删除 {} 个", visuals.len(), max_visuals, removed);
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
        let _irasu_dir = project_path.join("assets").join("irasu");

        for seg in &mut config.segments {
            for event in &mut seg.events {
                match event {
                    crate::project::Event::Emoji { keyword: Some(kw), name, .. } if !kw.is_empty() => {
                        // 如果 name 还没有被设置，从 keyword 解析
                        let needs_name = match name {
                            Some(n) => n.is_empty() || n == "auto",
                            None => true,
                        };
                        if needs_name
                            && let Some(ref cat) = catalog {
                                let results = cat.search(kw, 1);
                                if let Some(cp) = results.first() {
                                    *name = Some(cp.clone());
                                    tracing::info!("  ✓ emoji keyword「{}」→ codepoint {}", kw, cp);
                            }
                        }
                }
                    crate::project::Event::Irasu { keywords: Some(kws), name, .. } if !kws.is_empty() => {
                        let needs_name = match name {
                            Some(n) => n.is_empty() || n == "auto",
                            None => true,
                        };
                        if needs_name {
                            let irasu_dir = project_path.join("assets").join("irasu");
                            let found_stem = kws.iter()
                                .map(|k| sanitize_irasu_name(k))
                                .find(|stem| {
                                    let pattern = format!("{}_", stem);
                                    std::fs::read_dir(&irasu_dir).ok()
                                        .is_some_and(|e| e.flatten().any(|f| f.file_name().to_string_lossy().starts_with(&pattern)))
                                })
                                .unwrap_or_else(|| sanitize_irasu_name(kws.first().map(|s| s.as_str()).unwrap_or("")));
                            *name = Some(format!("{}_1", found_stem));
                            tracing::info!("  ✓ irasu → name {} (keywords={})", found_stem, kws.join(", "));
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
            tracing::info!("  ✓ emoji/irasu 位置已计算 (sticker_area, liver对侧 y对齐origin)");
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
    std::fs::write(project_path.join("polish.toml"), toml_str)?;
    tracing::info!("  ✓ polish.toml updated");
    Ok(())
}


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
        tracing::info!("  ✓ sfx tag「{}」→ id {}", tag, entry.id);
        Some(entry.id.clone())
    } else {
        tracing::info!("  ⚠ sfx tag「{}」未匹配到音效，使用原值", tag);
        None
    }
}

// ============================================================
// download
// ============================================================
pub async fn cmd_download(project_path: &Path) -> anyhow::Result<()> {
    tracing::info!("📥  Downloading assets...");
    let downloader = assets::downloader::AssetDownloader::new(project_path);

    let mut total_errors = 0;
    if let Ok(cfg) = project::load_project(project_path)
        && !cfg.segments.is_empty() {
            total_errors += download_keyword_assets(&downloader, &cfg).await;
        }

    if total_errors > 0 {
        tracing::info!("  ⚠ {} download errors", total_errors);
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
            tracing::info!("  ✓ emoji: {} new, {} cached", r.emoji_downloaded, r.emoji_existing);
        }
        total_errors += r.errors.len();
    }

    let has_irasu = config.segments.iter().any(|seg| {
        seg.events.iter().any(|ev| matches!(ev, project::Event::Irasu { keywords: Some(kws), .. } if !kws.is_empty()))
    });
    if has_irasu {
        let r = downloader.download_irasu_keywords(&config.segments).await;
        if r.irasu_downloaded > 0 || r.irasu_existing > 0 {
            tracing::info!("  ✓ irasu: {} new, {} cached", r.irasu_downloaded, r.irasu_existing);
        }
        total_errors += r.errors.len();
    }

    let has_sfx = !assets::downloader::collect_sfx_ids(&config.segments).is_empty();
    if has_sfx {
        let r = downloader.download_sfx_for_segments(&config.segments).await;
        if r.sfx_downloaded > 0 || r.sfx_existing > 0 || !r.errors.is_empty() {
            tracing::info!("  ✓ sfx: {} new, {} cached, {} errors", r.sfx_downloaded, r.sfx_existing, r.errors.len());
        }
        total_errors += r.errors.len();
    }

    total_errors
}

/// Auto-download missing assets (called by preview/render)
pub async fn ensure_assets(project_path: &Path) {
    let downloader = assets::downloader::AssetDownloader::new(project_path);

    if let Ok(config) = project::load_project(project_path)
        && !config.segments.is_empty() {
            let total_errors = download_keyword_assets(&downloader, &config).await;
            if total_errors > 0 {
                tracing::info!("  ⚠ {} download errors (will use CDN fallback where available)", total_errors);
            }
        }
}

// ============================================================
// preview
// ============================================================