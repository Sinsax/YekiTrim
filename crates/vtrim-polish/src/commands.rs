//! polish::commands：`vtrim polish *` 各子命令的实现（CLI 与 WebUI 共用同一份）。
//!
//! 入口：`cmd_init`、`cmd_parse`、`cmd_llm`、`cmd_llm_segment`、`cmd_preview`、`cmd_test`、`cmd_doctor`、`cmd_stickers_list` / `cmd_stickers_test` / `cmd_stickers_build` / `cmd_stickers_scan`、`build_package_tags`。
//! 边界：长任务写 `polish.toml` 一律走 `project::update_polish_toml`（锁内重新 load + 只覆盖自己负责的字段），不许"开头 load、结尾整份写回" —— 见 `crates/vtrim-polish/AGENTS.md`

use crate::{cover, process, render};

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::project::ProjectFile;
use crate::{llm, parse, project};


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
    if clip_json.exists()
        && let Ok(raw) = std::fs::read_to_string(&clip_json)
            && let Ok(meta) = serde_json::from_str::<serde_json::Value>(&raw) {
                if let Some(title) = meta.get("title").and_then(|v| v.as_str()) {
                    example.project.name = title.to_string();
                    tracing::info!("  ✓ Detected title: {}", title);
                }
                if let Some(dur) = meta.get("duration").and_then(|v| v.as_f64()) {
                    example.duration = Some(dur);
                    tracing::info!("  ✓ Detected duration: {:.0}s", dur);
                }
                // 自动设置素材路径 (danmaku 不硬编码, 见下方探测)
                example.source.video = "clip.mp4".into();
                example.source.subtitles = "clip.srt".into();
                example.source.danmaku = String::new();
                example.source.metadata = "clip.json".into();
                // 弹幕可选: 探测常见文件名, 找不到则留空, 避免配置指向不存在的文件
                const DANMAKU_CANDIDATES: [&str; 4] = ["clip.dm.srt", "clip.ass", "danmaku.ass", "danmaku.xml"];
                if let Some(dm) = DANMAKU_CANDIDATES.iter().find(|c| path.join(c).exists()) {
                    example.source.danmaku = (*dm).into();
                    tracing::info!("  ✓ Auto-detected danmaku: {}", dm);
                } else {
                    tracing::info!("  ⚠ No danmaku file found (弹幕可选, 可手动设置 source.danmaku)");
                }
                tracing::info!("  ✓ Auto-set source paths (clip.mp4, clip.srt, clip.json)");
            }

    let toml_str = example.to_toml_commented()?;
    // 落盘放进 polish.toml 的路径锁：任务与同步 handler（缩略图/静音接口的隐式初始化）
    // 并发初始化同一个目录时，只有一个能建，另一个老老实实报"已存在"。
    let write_path = config_path.clone();
    vtrim_core::filelock::with_path_lock(&config_path, move || -> anyhow::Result<()> {
        if write_path.exists() {
            anyhow::bail!("polish.toml already exists at {}", write_path.display());
        }
        crate::project::write_atomic(&write_path, &toml_str)?;
        Ok(())
    })
    .await?;
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
    let resolved = render::resolve_paths(project_path, &config);
    tracing::info!("  SRT:      {}", &config.source.subtitles);
    tracing::info!("  Danmaku:  {}", &config.source.danmaku);

    let subtitles = parse::srt::parse_file(&resolved.subtitles)?;
    let danmaku = if resolved.danmaku.is_empty() {
        vec![]
    } else {
        // 弹幕是可选的: 文件缺失/解析失败不中断 SRT 解析, 降级为无弹幕
        //
        // ⚠️ 必须用 `load_danmaku_auto`（**XML / ASS / SRT 都认**），不能用 `parse_srt_file`：
        // 后者只走 SRT 分支，而录播姬与 B站下载写的都是 `.xml` —— 实测真实 XML
        // （4 个样本，1200~3600 条）会被 SRT 分支全部解析成 **0 条**，而且不报错。
        // 后果是精修页"没有弹幕"：LLM 看不到弹幕、`danmaku_band` 为空（贴纸不避让）。
        // 这正是 `cutting/mod.rs` 记过的那类"换了解析器就静默变差"的坑，别退回。
        match vtrim_core::danmaku::load_danmaku_auto(std::path::Path::new(&resolved.danmaku)) {
            Ok(mut raw) => {
                parse::text::assign_danmaku_tracks(&mut raw);
                raw
            }
            Err(e) => {
                tracing::info!("  ⚠ Danmaku parse failed ({}), continuing without danmaku", e);
                vec![]
            }
        }
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
    let actual_duration = render::get_video_duration(&resolved.video).unwrap_or(0.0);
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

    // 更新 polish.toml 的 project.name（从 clip.json title 同步）。
    // 必须走 `update_polish_toml`（带路径锁）：直接整份覆写会和编辑器/后处理的保存互相
    // 覆盖 —— 那正是全 crate 统一用它的原因，这里是唯一的漏网之鱼。
    if !title.is_empty() {
        let name = title.clone();
        project::update_polish_toml(project_path, move |cfg| {
            cfg.project.name = name;
        })
        .await?;
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
/// LLM 剪辑（整片）。
///
/// `full = true`（编辑器按钮的默认语义）= **全部重做**：先清掉断点续传缓存，
/// 再去重新分段 + 逐段生成事件。`full = false` 才允许复用 checkpoint。
///
/// 为什么默认要 "全部重做"：断点续传的命中条件（`id && items`）在**重新分段之后**
/// 会给出"跳过 4 个窗口、实际只匹配 1 个"这种半吊子状态，用户看到的是
/// 「点一次 LLM 剪辑，事件反而被换乱了」。重做是用户点这颗按钮的意图；
/// 想省调用就该用**单段重做**（时间轴分段的属性面板），而不是赌缓存。
pub async fn cmd_llm(
    project_path: &Path,
    full: bool,
    cancel: Option<vtrim_core::llm::agent::CancelProbe>,
) -> anyhow::Result<()> {
    tracing::info!("🤖  LLM config generation... (full={full})");

    // 全部重做：先把 checkpoint 拿掉，之后每个窗口都会真问模型一次
    if full {
        let cp = project_path.join(".vtrim").join("polish_events.checkpoint.json");
        match std::fs::remove_file(&cp) {
            Ok(()) => tracing::info!("  ♻ 全部重做：已清空断点续传缓存 {}", cp.display()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => tracing::warn!("  ⚠ 清空断点续传缓存失败（继续重做）: {e}"),
        }
    }

    let config = project::load_project(project_path)?;
    let data_dir = project_path.join("data");

    // 读取解析数据
    let mut subtitles: Vec<parse::srt::Subtitle> = {
        let p = data_dir.join("subtitles.json");
        if !p.exists() { anyhow::bail!("Run 'vtrim polish init' first — missing {}", p.display()); }
        serde_json::from_str(&std::fs::read_to_string(&p)?)?
    };

    let mut danmaku: Vec<vtrim_core::danmaku::DanmakuEntry> = {
        let p = data_dir.join("danmaku.json");
        if !p.exists() {
            vec![]
        } else {
            serde_json::from_str(&std::fs::read_to_string(&p)?)?
        }
    };

    // 手动屏蔽区间内的字幕/弹幕不参与 LLM 生成
    if !config.mute_ranges.is_empty() {
        let before_sub = subtitles.len();
        subtitles.retain(|s| !config.mute_ranges.iter().any(|r| r.overlaps(s.start, s.end)));
        danmaku.retain(|d| !config.mute_ranges.iter().any(|r| r.overlaps(d.start, d.end)));
        if before_sub != subtitles.len() {
            tracing::info!("  ✂ 已从 LLM 输入过滤 {} 条屏蔽区字幕", before_sub - subtitles.len());
        }
    }

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

    // 视频时长 (用于数量预算 + 截断)
    let video_path = render::resolve_paths(project_path, &config).video;
    let video_duration = render::get_video_duration(&video_path)
        .or_else(|| {
            // 降级: 从 clip-meta.json 读 (可能不准，B站元数据含padding)
            std::fs::read_to_string(data_dir.join("clip-meta.json")).ok()
                .and_then(|raw| serde_json::from_str::<serde_json::Value>(&raw).ok())
                .and_then(|v| v.get("duration").and_then(|d| d.as_f64()))
        })
        .unwrap_or(0.0);

    // 数量预算: 字幕数 + 视频时长 → sticker/sfx 上限 (无 segment 数约束, 由 process 兜底)
    let (sticker_budget, sfx_budget) = crate::process::content_budget(
        subtitles.len(),
        video_duration.max(subtitles.last().map(|s| s.end).unwrap_or(0.0)),
        None,
    );
    tracing::info!("  预算: sticker ≤ {}, sfx ≤ {} (字幕 {}, 时长 {:.0}s)", sticker_budget, sfx_budget, subtitles.len(), video_duration);

    // 音效黑名单: 注入 prompt 让 LLM 主动避开
    let sfx_blacklist = crate::process::load_sfx_blacklist(&config);
    if !sfx_blacklist.is_empty() {
        tracing::info!("  音效黑名单: {}", sfx_blacklist.join(", "));
    }

    // 运行 LLM pipeline
    let (segments, dm_filter, hl_words, gen_status) = llm::run_llm_pipeline(
        &llm_cfg,
        &subtitles, &danmaku, &event_type, &config.sticker_source,
        sticker_budget, sfx_budget, &sfx_blacklist,
        Some(&project_path.join(".vtrim")),
        cancel,
    ).await?;

    // 屏蔽区间**不切段、不删事件**（用户要求：屏蔽只做标记）—— 出片时 `Ffmpeg::cut_ranges`
    // 会把这几段剪掉、预览也会跳过；事件留在文件里，用户撤销屏蔽之后内容原样还在。

    // 截断 segments 到视频实际时长
    let mut new_config = config;
    new_config.segments = segments;
    if video_duration > 0.0 {
        let before = new_config.segments.len();
        project::clamp_segments_to_duration(&mut new_config.segments, video_duration);
        let after = new_config.segments.len();
        if after < before {
            tracing::info!("  ⚠ Removed {} segment(s) beyond video duration ({:.0}s)", before, video_duration);
        }
    }
    new_config.filter.danmaku = dm_filter;
    new_config.filter.highlight = hl_words;

    // 只覆盖本次负责的三个字段：LLM 跑的过程中用户改的选框 / 封面 / 屏蔽轨道
    // （前端是自动保存的）必须留在 polish.toml 里，不能被这次整份写回吞掉。
    //
    // 先留一份最终 segments：下面的状态快照要按**它**映射（上面砍过超出视频时长的段，
    // 窗口下标与最终段下标已经对不上，不对齐界面就会给错段标色）。
    // 注：屏蔽区间**不再**改段的构成（只做标记，见上），所以这里只可能因时长截断而对不上。
    let final_segments = new_config.segments.clone();
    let (segments, dm_filter, hl_words) = (new_config.segments, new_config.filter.danmaku, new_config.filter.highlight);
    project::update_polish_toml(project_path, move |cfg| {
        cfg.segments = segments;
        cfg.filter.danmaku = dm_filter;
        cfg.filter.highlight = hl_words;
    })
    .await?;

    tracing::info!("\n✓ polish.toml updated with LLM-generated segments and keywords");
    let remapped = llm::status::remap_to_segments(&gen_status, &final_segments);
    // 失败段要在**任务日志**里也点名（编辑器里另有琥珀色 + 面板原因；
    // 只写进快照的话，看日志排查时还是一头雾水）
    let failed: Vec<String> = remapped
        .iter()
        .filter(|s| !s.ok)
        .map(|s| format!("#{}（{}）", s.index + 1, s.reason))
        .collect();
    if !failed.is_empty() {
        tracing::warn!(
            "⚠ {} 段没有产出事件：{} —— 可在时间轴上点那一段「只重做本段事件」",
            failed.len(),
            failed.join("；")
        );
    }
    if let Err(e) = llm::status::save(project_path, &llm::status::LlmRunStatus::from_full(remapped)) {
        // 快照只服务界面提示，写不进去不能算这次生成失败
        tracing::warn!("  ⚠ 生成状态快照写入失败: {e}");
    }
    Ok(())
}

/// **只重做某一段**的事件（编辑器：时间轴 → 分段属性 → 「只重做本段事件」）。
///
/// 只换 `segments[index].events`，别的段与筛选词/高亮词一律不动；
/// 失败时（429/超时/空结果）**保留该段原有事件**并如实报错，绝不把事件清空。
pub async fn cmd_llm_segment(
    project_path: &Path,
    index: usize,
    cancel: Option<vtrim_core::llm::agent::CancelProbe>,
) -> anyhow::Result<()> {
    tracing::info!("🤖  重做分段 #{index} 的事件...");

    let config = project::load_project(project_path)?;
    if index >= config.segments.len() {
        anyhow::bail!("分段下标越界: {index}（当前 {} 段）", config.segments.len());
    }
    let data_dir = project_path.join("data");

    // 与 cmd_llm 同一份输入：字幕按屏蔽区间过滤
    let mut subtitles: Vec<parse::srt::Subtitle> = {
        let p = data_dir.join("subtitles.json");
        if !p.exists() { anyhow::bail!("缺少 {}，先跑一次「初始化」", p.display()); }
        serde_json::from_str(&std::fs::read_to_string(&p)?)?
    };
    if !config.mute_ranges.is_empty() {
        subtitles.retain(|s| !config.mute_ranges.iter().any(|r| r.overlaps(s.start, s.end)));
    }

    let llm_cfg = vtrim_core::config::load_llm_config();
    if llm_cfg.api_key.is_empty() || llm_cfg.api_key == "sk-your-api-key-here" {
        // 单段重做是用户明确点的动作：不做就报错，别静默返回"成功"
        anyhow::bail!("未配置 LLM API key（设置页「LLM 配置」或 .vtrim/llm.toml）");
    }

    let event_type: String = std::fs::read_to_string(data_dir.join("clip-meta.json"))
        .ok()
        .and_then(|raw| serde_json::from_str::<serde_json::Value>(&raw).ok())
        .and_then(|v| v.get("event_type").and_then(|e| e.as_str().map(|s| s.to_string())))
        .unwrap_or_default();
    let sfx_blacklist = crate::process::load_sfx_blacklist(&config);

    let mut segments = config.segments.clone();
    let status = llm::regenerate_segment_events(
        &llm_cfg,
        &subtitles,
        &config.sticker_source,
        &sfx_blacklist,
        &event_type,
        &mut segments,
        index,
        Some(project_path),
        cancel,
    )
    .await?;

    if !status.ok {
        // 保持原样（连状态快照也要标失败，界面才知道这一段没换成）
        let _ = llm::status::upsert_segment(project_path, status.clone());
        anyhow::bail!("分段 #{} 重做失败：{}", index, status.reason);
    }

    // 只写回这一段的事件（其余字段由 update_polish_toml 的读改写保证不变）
    let events = segments[index].events.clone();
    let n = events.len();
    project::update_polish_toml(project_path, move |cfg| {
        if let Some(seg) = cfg.segments.get_mut(index) {
            seg.events = events;
        }
    })
    .await?;
    if let Err(e) = llm::status::upsert_segment(project_path, status) {
        tracing::warn!("  ⚠ 生成状态快照写入失败: {e}");
    }
    tracing::info!("  ✓ 分段 #{index} 已重做：{n} 个事件");
    Ok(())
}

// ============================================================
// render
// ============================================================

pub async fn cmd_preview(project_path: &Path, mode: &str) -> anyhow::Result<()> {
    tracing::info!("👁️  Preview mode");

    // 如果还没 process 过，先执行一次 process（保证位置/素材是最新）
    {
        let config = project::load_project(project_path)?;
        if !config.processed {
            tracing::info!("  ⚠ 尚未 process，先自动执行 process...");
            if let Err(e) = process::cmd_process(project_path).await {
                tracing::info!("  ⚠ Process: {}", e);
            }
        }
    }

    // 只预览：不重算位置/不跑 process，仅确保资源存在
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
        let resolved = render::resolve_paths(project_path, &config);
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
                && !frame_data.is_empty() { std::fs::write(&cover_image, &frame_data).ok(); }
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
            std::fs::create_dir_all(ws.join("assets").join("stickers"))?;
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

    let dm_entries = vtrim_core::danmaku::parse_srt_file(
        &work.join("test_danmaku.dm.srt"))?;
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
type = "sticker"
time = 1.0
duration = 2.0
keyword = "laugh"
x = "auto"
y = "auto"

[[segments.events]]
type = "sticker"
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
    std::fs::write(&cfg_path, cfg)?;
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

    // ⚠️ 这里曾经探 Chrome（PATH + 常见安装路径）—— 出片与封面那两条 headless Chrome 腿
    // 都已整条删除（2026-10-01），本仓不再需要外部浏览器。**别把它加回来**：doctor 里报一条
    // "✗ Chrome: NOT FOUND" 会让人去装一个已经没有任何消费者、也没有任何出片路径会用的东西。

    // Check LLM config
    let llm_cfg = vtrim_core::config::load_llm_config();
    if !llm_cfg.api_key.is_empty() && llm_cfg.api_key != "sk-your-api-key-here" {
        tracing::info!("  ✓ LLM: model={}, base={}", llm_cfg.model, llm_cfg.base_url);
    } else {
        tracing::info!("  ⚠ LLM: no API key in .vtrim/llm.toml");
    }

    // ---- Dhampir 出片通道（默认通道）----
    //
    // 这一段是"底座不可用要在渲染前可见"的命令行那一半（另一半是 `/api/polish/dhampir`
    // 与前端那颗通道下拉）。**只做定位、不渲染**：六项各查一次文件。
    tracing::info!("\n  -- Dhampir（默认出片通道）--");
    let ready = crate::render::dhampir::readiness("", true);
    for (label, value) in [
        ("二进制", &ready.binary),
        ("转译器", &ready.translator),
        ("字体", &ready.font),
        ("字体粗体", &ready.font_bold),
        ("预览产物", &ready.preview),
        ("Node", &ready.node_version),
    ] {
        match value {
            Some(p) => tracing::info!("  ✓ {label}: {p}"),
            None => tracing::info!("  ✗ {label}: 没找到"),
        }
    }
    // 词汇表单独印（**它不是"没找到就错"的那一类**）：包内没有时会退到编译期内嵌那份，
    // 行为等价 —— 但它与前三件一样是**包内交付物**（`xtask package` 会带 `spec/effects.json`），
    // 也是那道"兜底通道冻结闸门"的输入，所以要让人看得见它在哪。
    match &ready.spec {
        Some(p) => tracing::info!("  ✓ 效果词汇表: {p}"),
        None => tracing::info!("  · 效果词汇表: 内嵌（包内没有 spec/effects.json，用编译期那份）"),
    }
    if let Some(v) = &ready.contract_version {
        tracing::info!("  · 版本契约 project_schema={v}");
    }
    for problem in &ready.problems {
        tracing::info!("  ✗ {problem}");
    }
    if ready.ready {
        tracing::info!("  ✓ 底座可用（出片只有它一条腿）");
    } else {
        tracing::info!(
            "  ⚠ 底座**当前不可用** —— 出片会失败并报出上面这些原因。\
             出片只有底座这一条腿（本仓那条逐帧腿已整条删除），先把上面那几项补齐。"
        );
    }

    tracing::info!("\n  vtrim-polish v{}", env!("CARGO_PKG_VERSION"));
    Ok(())
}

// ============================================================
// stickers — 本地贴纸索引工具 (list / test / scan)
// ============================================================

/// 列出本地贴纸索引: 组名 | 成员数 | tags
pub fn cmd_stickers_list() -> anyhow::Result<()> {
    use crate::assets::sticker_index::StickerIndex;
    let dir = vtrim_core::path::sticker_dir();
    tracing::info!("📋  本地贴纸索引: {}", dir.display());
    let idx = StickerIndex::scan(&dir);
    let groups = idx.groups();
    if groups.is_empty() {
        tracing::info!("  (空 — 请先把贴纸文件放入该目录)");
        return Ok(());
    }
    let total: usize = groups.iter().map(|g| g.all_files().len()).sum();
    tracing::info!("  共 {} 组 / {} 个文件", groups.len(), total);
    for g in groups {
        let tags = if g.tags.is_empty() {
            String::new()
        } else {
            format!("  tags:[{}]", g.tags.join(", "))
        };
        let vs: Vec<&str> = g.variant_names().iter().filter(|v| !v.is_empty()).copied().collect();
        let vs = if vs.is_empty() { String::new() } else { format!("  ({})", vs.join("/")) };
        tracing::info!("  {} ×{}{}{}", g.name, g.all_files().len(), vs, tags);
    }
    Ok(())
}

/// 测试 keyword → 贴纸匹配 (可多个 keyword)
pub fn cmd_stickers_test(keywords: &[String]) -> anyhow::Result<()> {
    use crate::assets::sticker_index::StickerIndex;
    let dir = vtrim_core::path::sticker_dir();
    tracing::info!("🎯  本地贴纸匹配测试: {}", dir.display());
    let idx = StickerIndex::scan(&dir);
    for kw in keywords {
        match idx.match_keyword(kw) {
            Some(m) => {
                let g = &idx.groups()[m.group_index];
                tracing::info!(
                    "  ✓ 「{}」→ {} (score {} via {}, {} 帧)",
                    kw, g.name, m.score, m.via, g.all_files().len()
                );
            }
            None => {
                tracing::info!("  ✗ 「{}」无命中。候选:", kw);
                for (score, gi, via) in idx.candidates(kw, 3) {
                    tracing::info!("      {} (score {} via {})", idx.groups()[gi].name, score, via);
                }
            }
        }
    }
    Ok(())
}

/// 贴纸包 tag 构建的产出概要。
#[derive(Debug, Clone, Copy)]
pub struct PackageBuildReport {
    /// 参与打标的文件数
    pub files: usize,
    /// 实际发出的 LLM 批次数
    pub batches: usize,
    /// 生成的 tag 数量
    pub tags: usize,
}

/// 预处理贴纸包：用 LLM 分析包内文件并生成 `stickers/<包名>.toml` tags 候选清单。
pub async fn cmd_stickers_build(package: &str) -> anyhow::Result<()> {
    build_package_tags(package, &vtrim_core::progress::NoopProgress)
        .await
        .map(|_| ())
}

/// `cmd_stickers_build` 的实现体。WebUI 传入进度桥（复用 `ProgressReporter`：
/// 每批 LLM 调用上报 `stage_progress`，并在**批次边界**轮询 `cancelled()`）。
pub async fn build_package_tags(
    package: &str,
    progress: &dyn vtrim_core::progress::ProgressReporter,
) -> anyhow::Result<PackageBuildReport> {
    use crate::assets::sticker_index::{
        list_package_files, package_meta_to_toml, build_package_meta_from_tags,
    };
    let dir = vtrim_core::path::sticker_dir();
    let package_dir = dir.join(package);
    if !package_dir.is_dir() {
        anyhow::bail!("贴纸包不存在: {}", package_dir.display());
    }

    let files = list_package_files(&package_dir);
    if files.is_empty() {
        anyhow::bail!("贴纸包内没有图片文件: {}", package_dir.display());
    }

    let llm_cfg = vtrim_core::config::load_llm_config();
    if llm_cfg.api_key.is_empty() || llm_cfg.api_key == "sk-your-api-key-here" {
        anyhow::bail!("未配置 LLM API key，无法 build 贴纸包 tags (编辑 .vtrim/llm.toml)");
    }

    tracing::info!("  🤖 用 LLM 生成 tags ({} 个文件)...", files.len());
    progress.stage_started("表情包标签");
    let batches = files.len().div_ceil(PACKAGE_TAG_BATCH);

    let file_tags = llm_generate_package_tags(&llm_cfg, &files, progress).await?;
    let meta = build_package_meta_from_tags(package, &file_tags)
        .ok_or_else(|| anyhow::anyhow!("LLM 未生成有效 tags"))?;

    let toml_str = package_meta_to_toml(&meta)?;
    let out = dir.join(format!("{package}.toml"));
    std::fs::write(&out, toml_str)?;
    crate::assets::sticker_index::register_package(&dir, package)?;
    progress.stage_completed("表情包标签");
    tracing::info!("  ✓ 已生成贴纸包清单: {} ({} 个 tag)", out.display(), meta.tags.len());
    Ok(PackageBuildReport {
        files: files.len(),
        batches,
        tags: meta.tags.len(),
    })
}

/// 每批喂给 LLM 的文件数：太大输出会被截断，太小则调用次数爆炸。
const PACKAGE_TAG_BATCH: usize = 120;

/// 调用 LLM 为贴纸包内每个文件生成语义 tags，返回 `(文件名, tags)` 列表。
async fn llm_generate_package_tags(
    cfg: &vtrim_core::config::LlmConfig,
    files: &[String],
    progress: &dyn vtrim_core::progress::ProgressReporter,
) -> anyhow::Result<Vec<(String, Vec<String>)>> {
    use vtrim_core::llm::agent::extract_json_object;
    use vtrim_core::llm::LlmClient;

    // 链路解析：零散单发链路（见 [llm.overrides.utility]）；thinking 的默认关思考由它给出
    let linked = cfg.for_link(vtrim_core::config::LlmLink::Utility);
    let client = LlmClient::from_config(&linked)?
        .with_usage_context(vtrim_core::llm::usage::UsageContext::new("贴纸包标签"));
    let system = "你是贴纸语义标签助手。分析贴纸文件名，输出一个精简的语义 tag 索引。要求：\
        \n- 输出 JSON 对象，key 为语义 tag（情绪/动作/场景词），value 为该 tag 下的文件列表\
        \n- tag 数量控制在 10~20 个，必须是常见情绪/动作词（如 笑、哭、生气、害怕、开心、害羞、呆、紧张、惊讶、得意 等）\
        \n- 同一个文件可以出现在多个 tag 下\
        \n- 合并近义词：害羞/不好意思 只保留一个；生气/愤怒 只保留一个；害怕/恐惧 只保留一个；哭/悲伤 只保留一个\
        \n- 只输出 JSON，不要其他文字";

    // 一次调用输出反向索引；文件过多时仍分批，但每批独立输出 tag 索引，最终由聚合层合并同名 tag
    let total = files.len().div_ceil(PACKAGE_TAG_BATCH) as u64;
    let mut done = 0u64;
    let mut result = Vec::with_capacity(files.len());
    for chunk in files.chunks(PACKAGE_TAG_BATCH) {
        // 取消只在批次边界生效（进行中的那次 LLM 调用照常跑完）
        if progress.cancelled() {
            anyhow::bail!("任务已取消");
        }
        progress.stage_progress(done, total);
        let file_list = serde_json::to_string(chunk)?;
        let user = format!(
            "贴纸文件列表:\n{}\n\n输出 JSON 对象，例如:\n{{\"害怕\": [\"害怕_1.gif\", \"紧张_1.gif\"], \"笑\": [\"笑.gif\"], \"生气\": [\"生气.gif\"]}}",
            file_list
        );
        let resp = client.chat(system, &user).await?;
        let json = extract_json_object(&resp)?;
        let map: serde_json::Map<String, serde_json::Value> = serde_json::from_str(&json)?;
        for (tag, arr) in map {
            if let Some(files_arr) = arr.as_array() {
                let tag_files: Vec<String> = files_arr
                    .iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect();
                if !tag_files.is_empty() {
                    for f in tag_files {
                        if chunk.iter().any(|c| c == &f) {
                            result.push((f, vec![tag.clone()]));
                        }
                    }
                }
            }
        }
        done += 1;
        progress.stage_progress(done, total);
    }
    Ok(result)
}

/// 扫描本地贴纸目录生成/更新 stickers.json 清单
pub async fn cmd_stickers_scan(llm: bool) -> anyhow::Result<()> {
    use crate::assets::sticker_index::{StickerIndex, StickerMeta};
    let dir = vtrim_core::path::sticker_dir();
    tracing::info!("🔍  扫描本地贴纸目录: {}", dir.display());
    let idx = StickerIndex::scan(&dir);
    let mut metas: Vec<StickerMeta> = idx
        .groups()
        .iter()
        .filter(|g| !g.all_files().is_empty())
        .map(|g| StickerMeta {
            name: g.name.clone(),
            files: g
                .all_files()
                .iter()
                .filter_map(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
                .collect(),
            tags: vec![],
            aliases: vec![],
        })
        .collect();

    if llm {
        let llm_cfg = vtrim_core::config::load_llm_config();
        if llm_cfg.api_key.is_empty() || llm_cfg.api_key == "sk-your-api-key-here" {
            tracing::info!("  ⚠ 未配置 LLM API key (编辑 .vtrim/llm.toml), tags 留空");
        } else {
            tracing::info!("  🤖 用 LLM 生成 tags ({})...", llm_cfg.model);
            if let Err(e) = llm_generate_tags(&llm_cfg, &mut metas, None).await {
                tracing::info!("  ⚠ LLM tags 生成失败: {} (tags 留空)", e);
            }
        }
    }

    let json = serde_json::to_string_pretty(&metas)?;
    std::fs::write(dir.join("stickers.json"), json)?;
    tracing::info!("  ✓ 已写入 {}/stickers.json ({} 组)", dir.display(), metas.len());
    Ok(())
}

/// LLM 批量生成 tags: 分批调用 (每批 BATCH 组, 避免一次输出截断), 输出 {组名: [tags]}
const TAG_BATCH_SIZE: usize = 40;

async fn llm_generate_tags(
    cfg: &vtrim_core::config::LlmConfig,
    metas: &mut [crate::assets::sticker_index::StickerMeta],
    cancel: Option<vtrim_core::llm::agent::CancelProbe>,
) -> anyhow::Result<()> {
    use vtrim_core::llm::LlmClient;
    // 链路解析：零散单发链路（见 [llm.overrides.utility]）。
    // 以前这里是裸 `LlmClient::new`（不发送 max_tokens/temperature/extra_params）——
    // 于是这条链路既不受 [llm] 也不受链路覆盖约束，改成与其它调用点同一条路。
    let linked = cfg.for_link(vtrim_core::config::LlmLink::Utility);
    let client = LlmClient::from_config(&linked)?
        .with_usage_context(vtrim_core::llm::usage::UsageContext::new("贴纸扫描标签"));
    let system = "你是贴纸标签生成助手。为每个贴纸生成 3-6 个英文 tags（情绪/动作/场景词，供英文 keyword 匹配，如 love happy laugh shock game）。";

    let mut tagged = 0;
    for batch in metas.chunks_mut(TAG_BATCH_SIZE) {
        // 每个批次一次 LLM 调用：逐个查取消（与其它批量循环同口径）
        if cancel.as_ref().is_some_and(|c| c()) {
            anyhow::bail!("任务已取消");
        }
        let names: Vec<String> = batch.iter().map(|m| m.name.clone()).collect();
        let user = format!(
            "贴纸列表 (JSON 数组):\n{}\n\n输出 JSON 对象: {{\"贴纸名\": [\"tag1\",\"tag2\",...]}}\n只输出 JSON，不要多余文字。",
            serde_json::to_string(&names)?
        );
        let resp = client.chat(system, &user).await?;
        // 提取最外层 JSON 对象 (LLM 可能带 markdown 包裹)
        let resp_trim = resp.trim();
        let body = match (resp_trim.find('{'), resp_trim.rfind('}')) {
            (Some(s), Some(e)) if e > s => &resp_trim[s..=e],
            _ => resp_trim,
        };
        let map: serde_json::Map<String, serde_json::Value> = serde_json::from_str(body)?;
        for m in batch.iter_mut() {
            if let Some(v) = map.get(&m.name)
                && let Some(arr) = v.as_array()
            {
                m.tags = arr
                    .iter()
                    .filter_map(|t| t.as_str().map(|s| s.to_lowercase()))
                    .collect();
                tagged += 1;
            }
        }
    }
    tracing::info!("  ✓ {} 组已生成 tags", tagged);
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

/// `vtrim fonts list`：列出字体库（含**没下载的自带字体**）。
///
/// ⚠️ 判据全走 `vtrim_core::fonts`（`scan_all` / `family_for_at` / `builtin_status`），
/// 本层只负责排版 —— 库的形状只有一份，别在这里重新解释一遍。
///
/// ⚠️ "未安装"必须**看得见**：`scan_all` 会把清单里有、盘上没有的自带字体也列出来，
/// 这里要把它们标成 `未安装（可下载）`，否则用户以为库里空空如也。
pub fn cmd_fonts_list() -> anyhow::Result<()> {
    let root = vtrim_core::path::font_dir();
    tracing::info!("🔤  字体库: {}", root.display());
    let list = vtrim_core::fonts::scan_all(&root);
    let installed = list.iter().filter(|e| e.installed).count();
    tracing::info!("  共 {} 支（已安装 {} 支 / 待下载 {} 支）", list.len(), installed, list.len() - installed);
    for e in &list {
        let mut tags: Vec<String> = Vec::new();
        if e.builtin {
            tags.push("自带".into());
        }
        if e.is_default {
            tags.push("默认".into());
        }
        if e.emoji {
            tags.push("emoji".into());
        }
        if !e.installed {
            tags.push("未安装（可下载）".into());
        }
        let tags = if tags.is_empty() { String::new() } else { format!("  [{}]", tags.join("·")) };
        // 内部族名是**唯一权威**（预览/出片/Takumi 三处都认它），显示名只是标签 —— 两个都打出来
        tracing::info!(
            "  {}  族名={:?}  显示名={:?}  {}{}",
            e.id,
            vtrim_core::fonts::family_for_at(&root, &e.id),
            e.display_name,
            if e.installed { format!("{}KB", e.bytes / 1024) } else { "-".to_string() },
            tags
        );
    }
    if installed == 0 {
        tracing::info!("  💡 一支都没有？跑 `vtrim fonts download` 下自带字体（OFL 许可，可商用）");
    }
    Ok(())
}

/// `vtrim fonts download [--id X]`：下载自带字体（幂等，已装好的跳过）。
///
/// ⚠️ 根目录必须走 `ensure_dir()`（**写路径** = 用户固定目录，含老库搬家）——
/// 直接拿 `font_dir()` 当写目标会退回"读侧解析"，把字体下进程序目录、
/// 换个构建目录就全部「未下载」。
pub async fn cmd_fonts_download(id: Option<&str>) -> anyhow::Result<()> {
    let root = vtrim_core::fonts::ensure_dir()?;
    let want: Vec<&vtrim_core::fonts::BuiltinFont> = match id {
        Some(id) => {
            let hit: Vec<_> = vtrim_core::fonts::builtin_fonts()
                .iter()
                .filter(|b| b.id.eq_ignore_ascii_case(id.trim()))
                .collect();
            if hit.is_empty() {
                anyhow::bail!("不是自带字体（自带清单里没有它）: {id}");
            }
            hit
        }
        None => vtrim_core::fonts::builtin_fonts().iter().collect(),
    };
    let mut n = 0usize;
    for b in want {
        if vtrim_core::fonts::builtin_installed(&root, b) {
            tracing::info!("  ✓ {} 已安装，跳过", b.id);
            continue;
        }
        tracing::info!("  ⬇ 下载 {}（{}）…", b.display_name, b.license);
        vtrim_core::fonts::download_builtin_font(&root, b).await?;
        n += 1;
        tracing::info!("  ✓ {} 已安装到 {}/", b.id, root.display());
    }
    if n == 0 {
        tracing::info!("  没有需要下载的字体（都装好了）");
    } else {
        tracing::info!("  🎉 完成：本次下载 {n} 支");
    }
    Ok(())
}
