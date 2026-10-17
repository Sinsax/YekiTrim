//! polish::commands：`yekitrim polish *` 各子命令的实现（CLI 与 WebUI 共用同一份）。
//!
//! 入口：`cmd_init`、`cmd_parse`、`cmd_llm`、`cmd_llm_segment`、`cmd_preview`、`cmd_test`、`cmd_doctor`、`cmd_stickers_list` / `cmd_stickers_test` / `cmd_stickers_build` / `cmd_stickers_scan`、`build_package_tags`。
//! 边界：长任务写 `polish.toml` 一律走 `project::update_polish_toml`（锁内重新 load + 只覆盖自己负责的字段），不许"开头 load、结尾整份写回" —— 见 `crates/yekitrim-polish/AGENTS.md`

use crate::{cover, process, render};

use std::path::{Path, PathBuf};

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

    // **新工程的字幕字体 = 字体库里「设为默认」的那支**（用户 2026-10-06 报：
    // 「每次切出切片，字体都不是字体库设置的默认字体」）。
    //
    // ⚠️ 以前这里就是 `ProjectFile::example()` 里那个写死的 `"Noto Sans SC"`
    // （`render_core::types::default_font()`），**从不查字体库** ⇒ 用户设的默认位对
    // 新切出来的片段完全无效。剪切收尾会自动对每个新片段跑 init
    // （`yekitrim_service::core_pipeline::init_polish_for_clip`），所以每个新片段都中招。
    //
    // ⚠️ 写**族名**而不是 id：`SubtitleStyle::font` 两条路都认（库内按 id 解析、库外当系统族名），
    // 而"库里默认哪支"与 `fallback_stack()` 首项是同一个语义（都由
    // `fonts::default_text_family` 给出）。写 id 会让预览的 `.cap` 栈与成片的
    // `font_search_order` 首项对不上那支的内部族名。
    //
    // ⚠️ 库里没有任何一支设过默认 ⇒ **保持 example 的出厂值不动**（自带 Noto Sans SC），
    // 那是设计好的空库回落态（与 `SYSTEM_FONT_STACK` 首项同名，有单测钉着）。
    // ⚠️ **按归属取库**（2026-10-16）：`fonts::dir()` 是"单人/当前管理员"那条入口，
    //    而 server 可能正为**别的账号**切片段 —— 读错库的后果是"你设的默认位对别人的新片段生效"。
    let font_owner = yekitrim_core::owner::Owner::from_env();
    let _ = yekitrim_core::fonts::ensure_dir_for(font_owner);
    if let Some(family) =
        yekitrim_core::fonts::default_text_family(&yekitrim_core::fonts::dir_for(font_owner))
    {
        tracing::info!("  ✓ 字幕字体 = 库里的默认字体：{family}");
        example.style.subtitle.font = family;
    }

    // 自动检测 clip.json (来自 yekitrim cut)
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
    yekitrim_core::filelock::with_path_lock(&config_path, move || -> anyhow::Result<()> {
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

    // **预览代理**（720p / GOP10）：预览定位从"长 GOP 源每次重解"变成"最近的 10 帧"，
    // 实测 165ms → 18ms；体积约源的一半。**只给预览那条腿读**，出片永远用源。
    // 失败只 warn、绝不阻断 init（预览回落源，功能不受影响）。
    let _ = crate::render::proxy::ensure_preview_proxy(path, &example.source.video);

    tracing::info!("");
    tracing::info!("Next steps:");
    tracing::info!("  1. Edit .yekitrim/llm.toml — add your LLM API key");
    tracing::info!("  2. Run: yekitrim polish (full pipeline)");
    tracing::info!("  3. Run: yekitrim polish --preview  (or yekitrim polish -i <dir>)");
    Ok(())
}

// ============================================================
// parse
// ============================================================

/// 把 ASR 的**字级时间戳**搬进精修工程（`<片段>/data/char-timeline.json`）。
///
/// 数据源是 Stage 2 落的 `<output_dir>/asr/timeline.json`（逐字时间轴），
/// 目录关系与候选顺序见 `parse::charlog` 模块头 —— ⚠️ **`output_dir` 与片段目录不是同一个**。
///
/// # 三条纪律
///
/// 1. **没有就没有**：找不到 / 读不动 / 形状不合法 ⇒ 静默返回，**不报错、不阻断**
///    （调用方 `cmd_parse` / `cmd_init` 照常往下走）。存量工程绝大部分没有那份中间产物。
/// 2. **没数据不落文件**：只有真读到字级时间才写 `data/char-timeline.json`。
///    落一份空的是**错的** —— 存量工程目录里会凭空多出一个文件，"删掉 asr 文件 ⇒ 行为逐字节
///    相同"这条 R4 判据当场就假了。
/// 3. **只在真变了才写**：重跑 `parse`（预览/出片会隐式触发链路上的东西）不该把文件 mtime
///    刷新一遍 —— 首页「切片总览」按片段目录里最新文件的 mtime 排序，无条件重写会把老片段
///    一直顶到最前面。与 `process::normalize_danmaku_windows` 同一条纪律。
///
/// 对齐结果本身（哪些字属于哪条字幕）由纯函数 `parse::charlog::align_chars_to_subtitles` 算，
/// 与字幕的对应关系**不落盘**（它是可重算的派生值，落盘就有两个真源）。这里落的是
/// **ASR 原样的字级时间轴** —— 一句话：搬数据，不搬结论。
fn sync_char_timeline(project_path: &Path) {
    let Some(log) = parse::charlog::load_char_timeline(project_path) else {
        // 存量工程的常态。debug 级：一天几十条工程，每条都 info 会把日志刷花。
        tracing::debug!("无字级时间戳（asr/timeline.json 不在），跳过 —— 不影响工程");
        return;
    };
    let path = project_path.join(parse::charlog::CHAR_TIMELINE_REL);
    let Ok(pretty) = serde_json::to_string_pretty(&log) else {
        tracing::debug!("字级时间戳序列化失败，跳过");
        return;
    };
    // 「只在真变了才写」：内容一致就一个字节都不碰（含 mtime）。
    if std::fs::read_to_string(&path).map(|old| old == pretty).unwrap_or(false) {
        tracing::debug!("字级时间戳未变化，跳过写入");
        return;
    }
    if let Err(e) = std::fs::write(&path, &pretty) {
        // 写不进去也不许阻断 parse：字级时间戳是**增强**，不是工程成立的前提。
        tracing::warn!("字级时间戳写入失败（不影响工程）: {} {e}", path.display());
        return;
    }
    tracing::info!("  ✓ 字级时间戳: {} 字 → {}", log.char_count(), parse::charlog::CHAR_TIMELINE_REL);
}

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
        match yekitrim_core::danmaku::load_danmaku_auto(std::path::Path::new(&resolved.danmaku)) {
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

    // **字级时间戳搬运**（ASR `asr/timeline.json` → `data/char-timeline.json`）。
    //
    // ⚠️ **没有就没有**：存量工程的绝大部分都没有那份 ASR 中间产物，所以这里
    // 「找不到 ⇒ 什么都不做」。四个不许：不许报错、不许阻断 init/parse、
    // **不许落任何文件**（落空文件 = 存量工程目录里凭空多一个文件，破坏 R4 逐字节口径）。
    // 与 `_proxy_720.mp4` 那条「失败不阻断 init」同一个口径。
    //
    // 目录关系与候选顺序见 `parse::charlog` 模块头（`output/` ≠ 片段目录，是父子）。
    sync_char_timeline(project_path);

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
    cancel: Option<yekitrim_core::llm::agent::CancelProbe>,
) -> anyhow::Result<()> {
    cmd_llm_for(
        yekitrim_core::owner::Owner::from_env(),
        project_path,
        full,
        cancel,
    )
    .await
}

/// [`cmd_llm`] 的实现体，**带 owner**。
///
/// ⚠️ **server 必须走这一个**：`polish_spawn` 的 `"llm"` 分支是**在 server 进程内直接调**
/// （不是拉子进程），所以 `YEKITRIM_OWNER` 那套对它无效 —— 用 `cmd_llm` 会去读
/// **单人库**的贴纸索引，症状是多用户下"自己传的贴纸包选不中"。
pub async fn cmd_llm_for(
    owner: yekitrim_core::owner::Owner,
    project_path: &Path,
    full: bool,
    cancel: Option<yekitrim_core::llm::agent::CancelProbe>,
) -> anyhow::Result<()> {
    tracing::info!("🤖  LLM config generation... (full={full})");

    // 全部重做：先把 checkpoint 拿掉，之后每个窗口都会真问模型一次
    if full {
        let cp = project_path.join(yekitrim_core::path::PROJECT_STATE_DIR_NAME).join("polish_events.checkpoint.json");
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
        if !p.exists() { anyhow::bail!("Run 'yekitrim polish init' first — missing {}", p.display()); }
        serde_json::from_str(&std::fs::read_to_string(&p)?)?
    };

    let mut danmaku: Vec<yekitrim_core::danmaku::DanmakuEntry> = {
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

    // 从 .yekitrim/llm.toml 读取 LLM 配置（与 yekitrim 自身一致）
    // ⚠️ 按 **owner** 读：无 owner 版读的是顶层 `.yekitrim/llm.toml`，
    // 迁移后顶层那份已经搬进 `users/<uid>/`，用它会永远读到默认值。
    let llm_cfg = yekitrim_core::config::load_llm_config_for(owner);
    if llm_cfg.api_key.is_empty() || llm_cfg.api_key == "sk-your-api-key-here" {
        tracing::info!("  ⚠ No LLM API key configured. Edit .yekitrim/llm.toml");
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
    let (segments, dm_filter, hl_words, gen_status, edl_clips) = llm::run_llm_pipeline(
        owner,
        &llm_cfg,
        &subtitles, &danmaku, &event_type, &config.sticker_source,
        sticker_budget, sfx_budget, &sfx_blacklist,
        // §17 跳切的接缝判据要读它 —— 与 `cmd_process` 同一份 `mute_ranges`。
        &config.mute_ranges,
        Some(&project_path.join(yekitrim_core::path::PROJECT_STATE_DIR_NAME)),
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
    // 冷开场/回忆回溯落到 `edl.clips`（第 5 批）：`None` = 本轮没有意图 ⇒ **不许碰** `cfg.edl`。
    // ⚠️ 有 EDL 时 `mute_ranges` 整个被忽略（`ProjectFile::resolve_edl`、规格 §2.3），
    // 所以"没意图就别写"是 R4 存量零回归的判据本身；用户手拖过的 clips 在这里被 `None` 保住。
    project::update_polish_toml(project_path, move |cfg| {
        cfg.segments = segments;
        cfg.filter.danmaku = dm_filter;
        cfg.filter.highlight = hl_words;
        if let Some(clips) = edl_clips {
            cfg.edl.clips = clips;
        }
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
    cancel: Option<yekitrim_core::llm::agent::CancelProbe>,
) -> anyhow::Result<()> {
    cmd_llm_segment_for(
        yekitrim_core::owner::Owner::from_env(),
        project_path,
        index,
        cancel,
    )
    .await
}

/// [`cmd_llm_segment`] 的实现体，**带 owner**（理由同 [`cmd_llm_for`]）。
pub async fn cmd_llm_segment_for(
    owner: yekitrim_core::owner::Owner,
    project_path: &Path,
    index: usize,
    cancel: Option<yekitrim_core::llm::agent::CancelProbe>,
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

    // ⚠️ 按 **owner** 读：无 owner 版读的是顶层 `.yekitrim/llm.toml`，
    // 迁移后顶层那份已经搬进 `users/<uid>/`，用它会永远读到默认值。
    let llm_cfg = yekitrim_core::config::load_llm_config_for(owner);
    if llm_cfg.api_key.is_empty() || llm_cfg.api_key == "sk-your-api-key-here" {
        // 单段重做是用户明确点的动作：不做就报错，别静默返回"成功"
        anyhow::bail!("未配置 LLM API key（设置页「LLM 配置」或 .yekitrim/llm.toml）");
    }

    let event_type: String = std::fs::read_to_string(data_dir.join("clip-meta.json"))
        .ok()
        .and_then(|raw| serde_json::from_str::<serde_json::Value>(&raw).ok())
        .and_then(|v| v.get("event_type").and_then(|e| e.as_str().map(|s| s.to_string())))
        .unwrap_or_default();
    let sfx_blacklist = crate::process::load_sfx_blacklist(&config);

    let mut segments = config.segments.clone();
    let status = llm::regenerate_segment_events(
        owner,
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
        tracing::info!("  ⚠ No parsed data found. Run `yekitrim polish init` first.");
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
            if let Ok(frame_data) = yekitrim_core::process::cmd("ffmpeg")
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
            yekitrim_core::process::cmd("cmd")
                .args(["/c", "start", "", url])
                .spawn()
        } else if cfg!(target_os = "macos") {
            yekitrim_core::process::cmd("open").arg(url).spawn()
        } else {
            yekitrim_core::process::cmd("xdg-open").arg(url).spawn()
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
            let ok = yekitrim_core::process::cmd("ffmpeg")
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

    let dm_entries = yekitrim_core::danmaku::parse_srt_file(
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
intensity = 14
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
    match yekitrim_core::process::cmd("ffmpeg").arg("-version").output() {
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
    let llm_cfg = yekitrim_core::config::load_llm_config();
    if !llm_cfg.api_key.is_empty() && llm_cfg.api_key != "sk-your-api-key-here" {
        tracing::info!("  ✓ LLM: model={}, base={}", llm_cfg.model, llm_cfg.base_url);
    } else {
        tracing::info!("  ⚠ LLM: no API key in .yekitrim/llm.toml");
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
        ("ffmpeg", &ready.ffmpeg),
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
    // 契约面：**产品版本也要印**。两个"版本"是两回事 —— `project_schema` 管工程文件能不能读，
    // `host_api` 管 wasm 导出面对不对得上，而"这一版是哪一版"（0.1.0 / 0.2.0）**只有 `version=` 能回答**，
    // 且**没有运行时 API**（wasm 侧只有 `dhampir_host_api_version()`）。
    // 0.2.0 恰好是「契约号不动、字幕像素变了」的第一版 —— 用户报"字幕看着不一样"时，
    // 第一件要确认的就是他装的是哪一版底座。
    if let Some(v) = &ready.product_version {
        tracing::info!("  · 底座产品版本 version={v}");
    }
    match (&ready.contract_version, &ready.host_api_version) {
        (Some(s), Some(h)) => tracing::info!("  · 版本契约 project_schema={s} / host_api={h}"),
        (Some(s), None) => tracing::info!("  · 版本契约 project_schema={s}"),
        _ => {}
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

    tracing::info!("\n  yekitrim-polish v{}", env!("CARGO_PKG_VERSION"));
    Ok(())
}

// ============================================================
// stickers — 本地贴纸索引工具 (list / test / scan)
// ============================================================

/// 列出本地贴纸索引: 组名 | 成员数 | tags
pub fn cmd_stickers_list() -> anyhow::Result<()> {
    use crate::assets::sticker_index::StickerIndex;
    let dir = yekitrim_core::owner::sticker_dir_readable(yekitrim_core::owner::Owner::from_env());
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
    let dir = yekitrim_core::owner::sticker_dir_readable(yekitrim_core::owner::Owner::from_env());
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
    build_package_tags(package, &yekitrim_core::progress::NoopProgress)
        .await
        .map(|_| ())
}

/// `cmd_stickers_build` 的实现体（身份取 `YEKITRIM_OWNER`）。
///
/// ⚠️ 这是**子进程**那条腿（`yekitrim stickers build`）。server 里跑构建任务的那条腿
/// 走 [`build_package_tags_for`]，**必须**显式带 owner —— 见那里的注释。
pub async fn build_package_tags(
    package: &str,
    progress: &dyn yekitrim_core::progress::ProgressReporter,
) -> anyhow::Result<PackageBuildReport> {
    build_package_tags_for(
        yekitrim_core::owner::Owner::from_env(),
        package,
        progress,
    )
    .await
}

/// `cmd_stickers_build` 的实现体。WebUI 传入进度桥（复用 `ProgressReporter`：
/// 每批 LLM 调用上报 `stage_progress`，并在**批次边界**轮询 `cancelled()`）。
///
/// ⚠️ **`owner` 必须由调用方传进来**，别在这里用 `Owner::from_env()`：
/// 这个函数在 **server 进程内**被任务线程直接调用，而 server 进程的环境变量是它**自己**
/// 启动时那份（`YEKITRIM_OWNER` 只在"server 拉起 polish 子进程"时才有意义）——
/// 于是多用户下构建会去翻**单人库**，报「贴纸包不存在」。
/// 实测症状（2026-10-10）：admin 的包在 `users/1/resources/stickers/testpack`，
/// 构建却去 `resources/stickers/testpack` 找，任务失败、而列表里那个包好好地在着。
pub async fn build_package_tags_for(
    owner: yekitrim_core::owner::Owner,
    package: &str,
    progress: &dyn yekitrim_core::progress::ProgressReporter,
) -> anyhow::Result<PackageBuildReport> {
    use crate::assets::sticker_index::{
        list_package_files, package_meta_to_toml, build_package_meta_from_tags,
    };
    let dir = yekitrim_core::owner::asset_dir(owner, yekitrim_core::owner::AssetKind::Stickers);
    let package_dir = dir.join(package);
    if !package_dir.is_dir() {
        anyhow::bail!("贴纸包不存在: {}", package_dir.display());
    }

    let files = list_package_files(&package_dir);
    if files.is_empty() {
        anyhow::bail!("贴纸包内没有图片文件: {}", package_dir.display());
    }

    // ⚠️ **按 owner 读**（不是 `load_llm_config()` —— 那个读顶层 `.yekitrim/llm.toml`）。
    // 迁移之后顶层那份**已经搬走了**，用无 owner 版会永远读到默认值 ⇒ 明明配了 key
    // 却报"未配置 LLM API key"。
    let llm_cfg = yekitrim_core::config::load_llm_config_for(owner);
    if llm_cfg.api_key.is_empty() || llm_cfg.api_key == "sk-your-api-key-here" {
        anyhow::bail!(
            "未配置 LLM API key，无法 build 贴纸包 tags              (编辑 {} 里的 llm.toml)",
            yekitrim_core::owner::state_dir(owner).display()
        );
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
    cfg: &yekitrim_core::config::LlmConfig,
    files: &[String],
    progress: &dyn yekitrim_core::progress::ProgressReporter,
) -> anyhow::Result<Vec<(String, Vec<String>)>> {
    use yekitrim_core::llm::agent::extract_json_object;
    use yekitrim_core::llm::LlmClient;

    // 链路解析：零散单发链路（见 [llm.overrides.utility]）；thinking 的默认关思考由它给出
    let linked = cfg.for_link(yekitrim_core::config::LlmLink::Utility);
    let client = LlmClient::from_config(&linked)?
        .with_usage_context(yekitrim_core::llm::usage::UsageContext::new("贴纸包标签"));
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

// ============================================================
// emoji 多维分类（开发期工具）
// ============================================================

/// 每批喂给 LLM 的 emoji 条数。
///
/// 40 条 × 六维 —— 与贴纸包那条（`PACKAGE_TAG_BATCH`=120）不同：那边每条只出一两个 tag，
/// 这边每条要出 6 个维度、输出体积大得多，同一条教训（太大输出会被截断、截断就是静默丢标签）。
pub const EMOJI_TAG_BATCH: usize = 40;

/// `polish emoji build` 的结果（打印给人看 + 单测断言用）。
#[derive(Debug, Default)]
pub struct EmojiBuildReport {
    /// 处理了多少条
    pub total: usize,
    /// LLM 调用次数
    pub batches: usize,
    /// 真的拿到分类的条数（没拿到的保留原有 tag，不写 dim）
    pub classified: usize,
    /// ``维度 ` 取值`, 命中条数`` —— **含 0**（0 就是"这个词没有任何表情"，闭合性判据要它）
    pub coverage: Vec<(String, usize)>,
    /// 被丢掉的非法取值（`维度:取值`），逐条报出来 —— 它们是"LLM 造了词"的证据
    pub dropped: Vec<String>,
    /// 写回路径（`--dry-run` 时为 None）
    pub out_path: Option<std::path::PathBuf>,
}

/// **开发期工具**：用 LLM 把内嵌 emoji 目录重做成**六维闭集分类**，写回**仓库里**的
/// `crates/yekitrim-polish/assets/emoji_catalog.json`。
///
/// ⚠️ 它写的是**编译期输入**（`emoji_search.rs` 的 `include_str!`）⇒ 跑完必须 `cargo build`。
/// ⚠️ **只在开发机上能跑**：路径取自编译期常量 `CARGO_MANIFEST_DIR`；发行二进制里那个路径
/// 不存在 —— 于是它**明确报错**，而不是往用户数据目录写一份没人读的 JSON。
/// ⚠️ 写回前**先备份** `<文件>.bak`：不满意就换回来（git 上也有一份）。
///
/// 词表真源在 `assets::emoji_taxonomy`（prompt 与查询共用，**不许在这里另抄一份**）。
pub async fn cmd_emoji_build(
    catalog: Option<std::path::PathBuf>,
    dry_run: bool,
    limit: Option<usize>,
    drop_legacy_tags: bool,
    // 默认**增量**（已经有 §dim§ 的条目跳过）—— 一次全量是 23 次调用，
    // 补漏/换词表时不该再付一遍。要彻底重做就打开它。
    reclassify_all: bool,
) -> anyhow::Result<EmojiBuildReport> {
    use yekitrim_core::llm::LlmClient;

    let path = catalog.unwrap_or_else(|| {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("assets")
            .join("emoji_catalog.json")
    });
    if !path.is_file() {
        anyhow::bail!(
            "emoji 目录不存在: {} —— 这条是**开发期工具**（路径取编译期 CARGO_MANIFEST_DIR），\
             发行二进制里跑不了；要跑就显式传 --catalog <路径>",
            path.display()
        );
    }
    let raw = std::fs::read_to_string(&path)?;
    let mut entries: Vec<serde_json::Value> = serde_json::from_str(&raw)?;
    if entries.is_empty() {
        anyhow::bail!("emoji 目录是空的: {}", path.display());
    }
    if let Some(n) = limit {
        entries.truncate(n);
    }

    let owner = yekitrim_core::owner::Owner::from_env();
    let llm_cfg = yekitrim_core::config::load_llm_config_for(owner);
    if llm_cfg.api_key.is_empty() || llm_cfg.api_key == "sk-your-api-key-here" {
        anyhow::bail!(
            "未配置 LLM API key（编辑 {} 里的 llm.toml）",
            yekitrim_core::owner::state_dir(owner).display()
        );
    }

    tracing::info!(
        "  🤖 emoji 多维分类：{} 条 / 每批 {} 条（模型 {}）…",
        entries.len(),
        EMOJI_TAG_BATCH,
        llm_cfg.model
    );
    let linked = llm_cfg.for_link(yekitrim_core::config::LlmLink::Utility);
    let client = LlmClient::from_config(&linked)?
        .with_usage_context(yekitrim_core::llm::usage::UsageContext::new("emoji 多维分类"));

    let system = emoji_classify_system_prompt();
    let mut report = EmojiBuildReport { total: entries.len(), ..Default::default() };
    let mut hit: std::collections::HashMap<String, usize> = std::collections::HashMap::new();

    // ⚠️ **两轮**：第一轮全量；第二轮只补第一轮没匹配上的（批更小）。
    //    为什么必须有第二轮（2026-10-17 实测）：第一轮 881 条只落了 478 —— 未分类的是
    //    **6 段连续区**（40–199 / 240–279 / 320–359 / 440–559 / 600–639），也就是**整批**
    //    没匹配上（模型把结果包了一层、或键换了写法），而 `map.get()` 直接落空、**一声不响**。
    //    静默的部分成功比彻底失败更坏 —— 所以这一轮既补数据，也把"哪批没对上"打到日志里。
    let mut pending: Vec<String> = Vec::new();
    // 批一次比一次小：40 → 20 → 10。第三轮给"怎么问都说不好"的那几条收尾
    // （实测前两轮之后仍剩 20 条没分类；它们不阻塞使用，但补上更好排查）。
    for round in 0..3 {
        let batch = match round {
            0 => EMOJI_TAG_BATCH,
            1 => EMOJI_TAG_BATCH / 2,
            _ => EMOJI_TAG_BATCH / 4,
        };
        let mut work: Vec<&mut serde_json::Value> = if round == 0 {
            entries
                .iter_mut()
                .filter(|e| reclassify_all || e.get("dim").is_none())
                .collect()
        } else {
            entries
                .iter_mut()
                .filter(|e| {
                    let cp = e.get("codepoint").and_then(|v| v.as_str()).unwrap_or("");
                    pending.iter().any(|p| p == cp)
                })
                .collect()
        };
        if work.is_empty() {
            break;
        }
        tracing::info!("  · 第 {} 轮：{} 条（每批 {}）", round + 1, work.len(), batch);
        pending = Vec::new();
        for chunk in work.chunks_mut(batch) {
            report.batches += 1;
            let missed = classify_emoji_batch(
                &client,
                &system,
                chunk,
                drop_legacy_tags,
                &mut hit,
                &mut report.dropped,
            )
            .await?;
            if missed.len() == chunk.len() {
                tracing::warn!(
                    "  ⚠ 第 {} 批（{} 条）**一条都没匹配上** —— 模型换了键写法或包了一层？",
                    report.batches,
                    chunk.len()
                );
            }
            pending.extend(missed);
        }
    }
    // 到这里 `pending` = 两轮之后仍然没分类的（保持原样、不写 dim）
    if !pending.is_empty() {
        tracing::warn!("  ⚠ {} 条两轮都没分类成功，保持原样", pending.len());
    }
    report.classified = entries
        .iter()
        .filter(|e| e.get("dim").is_some())
        .count();

    // 覆盖表：**每个取值都列**（0 也要列 —— 闭合性判据就是看有没有 0）
    for d in &crate::assets::emoji_taxonomy::taxonomy().dims {
        for v in &d.values {
            report.coverage.push((format!("{}.{}", d.key, v), *hit.get(v).unwrap_or(&0)));
        }
    }
    report.dropped.sort();
    report.dropped.dedup();

    if dry_run {
        tracing::info!("  · --dry-run：不写盘");
        return Ok(report);
    }

    let bak = path.with_extension("json.bak");
    std::fs::copy(&path, &bak)?;
    let mut text = serde_json::to_string_pretty(&entries)?;
    text.push('\n');
    std::fs::write(&path, text)?;
    tracing::info!("  ✓ 已写回 {}（备份 {}）", path.display(), bak.display());
    report.out_path = Some(path);
    Ok(report)
}

/// 把 LLM **一批**的输出落进 chunk，返回**没匹配上**的 codepoint。
///
/// 抽出它是为了两件事：① 两轮（全量 + 补漏）共用同一段逻辑；
/// ② **整批没对上必须能被报出来** —— 第一版只在调用点做 map.get(cp)，
/// 实测 881 条只落 478（未分类的是 6 段连续区 = 整批落空），而日志里一个字都没有。
/// 静默的部分成功比彻底失败更坏。
async fn classify_emoji_batch(
    client: &yekitrim_core::llm::LlmClient,
    system: &str,
    chunk: &mut [&mut serde_json::Value],
    drop_legacy_tags: bool,
    hit: &mut std::collections::HashMap<String, usize>,
    dropped: &mut Vec<String>,
) -> anyhow::Result<Vec<String>> {
    use yekitrim_core::llm::agent::extract_json_object;

    let input: Vec<serde_json::Value> = chunk
        .iter()
        .map(|e| {
            serde_json::json!({
                "codepoint": e.get("codepoint").and_then(|v| v.as_str()).unwrap_or(""),
                // 官方短名是最有用的上下文（:joy: / :smile:）；name 字段是 emoji_u1f602 这种，没用
                "shortcodes": e.get("shortcodes").cloned().unwrap_or(serde_json::json!([])),
                // **把现有分类当提示喂进去**（用户口径「根据当前的进行处理」）：
                // 它是参考、不是约束 —— 词表才是约束。
                "current": e.get("tag").cloned().unwrap_or(serde_json::json!([])),
            })
        })
        .collect();
    let user = format!(
        "emoji 列表（codepoint 是唯一键）：\n{}\n\n\
         只输出一个 JSON 对象：key = codepoint（原样回传），value = 各维度的字符串数组（没有就空数组）。",
        serde_json::to_string(&input)?
    );
    let resp = client.chat(system, &user).await?;
    let json = extract_json_object(&resp)?;
    let map: serde_json::Map<String, serde_json::Value> = serde_json::from_str(&json)?;

    let mut missed: Vec<String> = Vec::new();
    for e in chunk.iter_mut() {
        let cp = e.get("codepoint").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let Some(obj) = find_dims_for(&map, &cp) else {
            missed.push(cp);
            continue;
        };
        // ① 归一化（纯函数：只留词表里的取值、单选只留一个、没给的补空数组）
        let (dim, dim_union, d) = normalize_emoji_dims(obj);
        dropped.extend(d);
        if dim.values().all(|v| v.as_array().is_some_and(|a| a.is_empty())) {
            // 一条都没认出来：保留原有 tag、不写 dim（宁可漏，不可错），并记进没对上
            missed.push(cp);
            continue;
        }
        // ② tag = 各维取值并集（派生字段）。默认并上旧 tag：那些别名是老工程的 keyword
        //    还能命中的唯一依据（search 走 index = tag ∪ shortcodes）。
        let legacy: Vec<String> = if drop_legacy_tags {
            Vec::new()
        } else {
            e.get("tag")
                .and_then(|v| v.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|v| v.as_str())
                        .map(|t| t.trim().to_ascii_lowercase())
                        .filter(|t| !t.is_empty())
                        .collect()
                })
                .unwrap_or_default()
        };
        let tag = derive_emoji_tag(&dim_union, &legacy);
        for v in &tag {
            *hit.entry(v.clone()).or_insert(0) += 1;
        }
        let obj_mut = e.as_object_mut().expect("catalog 条目必须是对象");
        obj_mut.insert("dim".to_string(), serde_json::Value::Object(dim));
        obj_mut.insert("tag".to_string(), serde_json::json!(tag));
    }
    Ok(missed)
}

/// 在 LLM 返回的 JSON 里找 cp 的分类 —— 三层兜底。
///
/// 2026-10-17 实测的两种走样：模型把结果包了一层（多一个顶层键）、
/// 或把键写成 emoji_u1f602（拿 name 字段当键）。调用点原来只有 map.get(cp) 这一手，
/// 于是整批落空。三层：① 直接键；② 任意一层嵌套对象里的键；③ 归一化键
/// （去 emoji_u / emoji_ 前缀、大小写）+ 数组里按 codepoint 字段找。
fn find_dims_for<'a>(
    map: &'a serde_json::Map<String, serde_json::Value>,
    cp: &str,
) -> Option<&'a serde_json::Map<String, serde_json::Value>> {
    fn norm(k: &str) -> String {
        k.trim()
            .trim_start_matches("emoji_u")
            .trim_start_matches("emoji_")
            .to_ascii_lowercase()
    }
    let want = norm(cp);
    if let Some(v) = map.get(cp).and_then(|v| v.as_object()) {
        return Some(v);
    }
    for (k, v) in map {
        if norm(k) == want {
            if let Some(o) = v.as_object() {
                return Some(o);
            }
        }
        match v {
            serde_json::Value::Object(o) => {
                for (k2, v2) in o {
                    if norm(k2) == want {
                        if let Some(o2) = v2.as_object() {
                            return Some(o2);
                        }
                    }
                }
            }
            // 数组形态：[{"codepoint":"1f602","emotion":[…],…}, …]
            serde_json::Value::Array(a) => {
                for it in a {
                    let Some(o) = it.as_object() else { continue };
                    if o.get("codepoint").and_then(|c| c.as_str()).map(norm).as_deref()
                        == Some(want.as_str())
                    {
                        return Some(o);
                    }
                }
            }
            _ => {}
        }
    }
    None
}

/// 分类用的 system prompt。
///
/// ⚠️ 词表**只从 `emoji_taxonomy::prompt_vocab_text()` 来** —— 在这里抄一份就必然漂。
fn emoji_classify_system_prompt() -> String {
    format!(
        "你是 emoji 语义分类助手。为每个 emoji 在**给定词表**里选标签。\n\
         \n\
         ## 词表（**只能从中选**，不许造词；造的词会被丢弃）\n\
         {}\n\
         ## 要求\n\
         - 输出**一个 JSON 对象**：key = codepoint（原样回传），value = 各维度**字符串数组**\n\
         - 单选维度（标注了「单选」的）**只给一个**值；确实说不好就给最接近的那个，别空着除非真的无关\n\
         - 可多选维度：**看到就给，别只给显眼的那些**（2026-10-17 实测：模型只给 hand/body/animal，\n         把 🎸 的 object.music、🚩 的 object.flag、🏠 的 object.building、⏰ 的 object.time 全漏了，\n         而这些题材**目录里都有** ⇒ 那一维的大多数取值零覆盖、查询等于空转）\n         - ⚠️ `[]` **只留给真的没有那一维的表情**（😀 是纯情绪，object 才该是空）；\n         有具体东西（动物/食物/手/乐器/球/房子/旗子/钟表）就必须给 object，\n         有明确动作（挥手/点赞/鼓掌/祈祷/写字/打电话/睡觉/吃东西）就必须给 action\n\
         - `intensity` 看情绪的**爆发程度**：微笑=1 / 大笑·惊讶=2 / 崩溃·暴怒·狂喜=3\n\
         - `usage` 想的是「做直播切片时这个表情该出现在哪」，不是表情本身长什么样\n\
         - 只输出 JSON，不要 markdown、不要解释\n",
        crate::assets::emoji_taxonomy::prompt_vocab_text()
    )
}

/// 把 LLM 给的一条分类**归一化**（纯函数，好单测）。
///
/// 三条规矩，缺一条都会静默出错：
/// ① **只留词表里的取值** —— 造的词丢掉并记进 `dropped`（否则它会永远选不到表情）；
/// ② **单选维度只留一个** —— 多给了就按它给的顺序取首个（不静默留一堆让打分变成"全都沾一点"）；
/// ③ **没给的维度补 `[]`** —— 消费侧不必判 `Option`，`dim.emotion` 永远是数组。
///
/// 返回 `(dim 对象, 取值并集, 被丢掉的非法取值)`。
pub fn normalize_emoji_dims(
    raw: &serde_json::Map<String, serde_json::Value>,
) -> (
    serde_json::Map<String, serde_json::Value>,
    Vec<String>,
    Vec<String>,
) {
    let mut dim = serde_json::Map::new();
    let mut union: Vec<String> = Vec::new();
    let mut dropped: Vec<String> = Vec::new();
    for d in &crate::assets::emoji_taxonomy::taxonomy().dims {
        let mut picked: Vec<String> = Vec::new();
        let raws: Vec<String> = match raw.get(&d.key) {
            Some(serde_json::Value::Array(a)) => a
                .iter()
                .map(|v| match v {
                    serde_json::Value::String(s) => s.clone(),
                    other => other.to_string().trim_matches('"').to_string(),
                })
                .collect(),
            // 模型偶尔给单值而不是数组（`"intensity": "2"`）—— 收，别当没给
            Some(serde_json::Value::String(s)) => vec![s.clone()],
            _ => Vec::new(),
        };
        for v in raws {
            let low = v.trim().to_ascii_lowercase();
            if low.is_empty() {
                continue;
            }
            if crate::assets::emoji_taxonomy::is_valid_value(&d.key, &low) {
                if !picked.contains(&low) {
                    picked.push(low);
                }
            } else {
                dropped.push(format!("{}:{}", d.key, low));
            }
        }
        if !d.multi {
            picked.truncate(1);
        }
        if !picked.is_empty() {
            union.extend(picked.iter().cloned());
        }
        dim.insert(d.key.clone(), serde_json::json!(picked));
    }
    (dim, union, dropped)
}

/// 派生 `tag` = 各维取值并集 ∪ 旧 tag（去重排序）。
///
/// ⚠️ 它是**派生字段**，不是第二份真源：面板按主维度分区（`primary_dim`）、查询走 `dim`，
/// `tag` 只剩"旧工程的自由词还能命中"这一个用途（`index` = tag ∪ shortcodes）。
pub fn derive_emoji_tag(dim_union: &[String], legacy: &[String]) -> Vec<String> {
    let mut v: Vec<String> = dim_union.to_vec();
    v.extend(legacy.iter().cloned());
    v.sort();
    v.dedup();
    v
}

#[cfg(test)]
mod emoji_build_tests {
    use super::*;

    fn dims_of(json: serde_json::Value) -> serde_json::Map<String, serde_json::Value> {
        json.as_object().expect("测试输入是对象").clone()
    }

    /// **闭合性**：词表外的取值必须丢掉（否则"那个词永远选不到表情"）。
    ///
    /// 变异自证：把 `is_valid_value` 那道判断去掉 → 当场红。
    #[test]
    fn normalize_emoji_dims_enforces_the_vocabulary() {
        let raw = dims_of(serde_json::json!({
            "emotion": ["joy", "ecstatic"],
            "intensity": ["3", "9"],
            "action": ["laugh", "vibe"],
            "tone": ["positive"],
        }));
        let (dim, union, dropped) = normalize_emoji_dims(&raw);
        assert_eq!(dim["emotion"], serde_json::json!(["joy"]));
        assert_eq!(dim["intensity"], serde_json::json!(["3"]));
        assert_eq!(dim["action"], serde_json::json!(["laugh"]));
        assert!(union.contains(&"joy".to_string()));
        assert!(!union.contains(&"ecstatic".to_string()), "词表外的词不许进 tag");
        assert!(dropped.contains(&"emotion:ecstatic".to_string()), "丢掉的要报出来: {dropped:?}");
        assert!(dropped.contains(&"intensity:9".to_string()));
        assert!(dropped.contains(&"action:vibe".to_string()));
    }

    /// **单选维度只留一个**；可多选维度照收；没给的维度补空数组。
    ///
    /// 变异自证：删掉 `picked.truncate(1)` → 第一条断言红。
    #[test]
    fn normalize_emoji_dims_keeps_one_per_single_choice_dim() {
        let raw = dims_of(serde_json::json!({
            "emotion": ["joy", "love"],
            "intensity": "2",
            "usage": ["punchline", "reaction"],
        }));
        let (dim, union, _) = normalize_emoji_dims(&raw);
        assert_eq!(dim["emotion"], serde_json::json!(["joy"]));
        assert_eq!(dim["intensity"], serde_json::json!(["2"]));
        assert_eq!(dim["usage"], serde_json::json!(["punchline", "reaction"]));
        assert_eq!(dim["object"], serde_json::json!([]));
        assert_eq!(dim["tone"], serde_json::json!([]));
        assert!(!union.contains(&"love".to_string()), "被截掉的单选值不许进 tag");
    }

    /// 派生 tag = 并集 ∪ 旧 tag，去重且有序。
    ///
    /// 变异自证：去掉 `legacy` 那一半 → 旧工程的 keyword 命中不了（断言红）。
    #[test]
    fn derive_emoji_tag_unions_dim_values_and_legacy() {
        let tag = derive_emoji_tag(
            &["joy".to_string(), "laugh".to_string()],
            &["happy".to_string(), "joy".to_string()],
        );
        assert_eq!(tag, vec!["happy", "joy", "laugh"]);
    }

    /// **开发期工具的手跑入口**（与其它诊断同规矩：`#[ignore]` + 环境变量门控）。
    ///
    /// ```bash
    /// VT_EMOJI_LIMIT=40 cargo test -p yekitrim-polish --lib emoji_build_dev_tool -- --ignored --nocapture
    /// VT_EMOJI_BUILD=1 cargo test -p yekitrim-polish --lib emoji_build_dev_tool -- --ignored --nocapture
    /// ```
    ///
    /// * `VT_EMOJI_BUILD=1` = **真写盘**（不设则只算不写）
    /// * `VT_EMOJI_LIMIT=N` = 只跑前 N 条
    /// * `VT_EMOJI_CATALOG=<path>` = 指定目录 JSON
    /// * `VT_EMOJI_DROP_LEGACY=1` = 丢掉旧 tag
    ///
    /// ⚠️ 写盘 = 改**编译期输入**（`assets/emoji_catalog.json`）⇒ 跑完要 `cargo build`。
    #[tokio::test]
    #[ignore = "开发期工具：真调 LLM；VT_EMOJI_BUILD=1 才写盘"]
    async fn emoji_build_dev_tool() {
        let dry = std::env::var("VT_EMOJI_BUILD").ok().as_deref() != Some("1");
        let limit = std::env::var("VT_EMOJI_LIMIT").ok().and_then(|v| v.parse().ok());
        let catalog = std::env::var("VT_EMOJI_CATALOG").ok().map(std::path::PathBuf::from);
        let drop_legacy = std::env::var("VT_EMOJI_DROP_LEGACY").ok().as_deref() == Some("1");
        let all = std::env::var("VT_EMOJI_ALL").ok().as_deref() == Some("1");
        println!(
            "用法: VT_EMOJI_LIMIT=40 [VT_EMOJI_BUILD=1] cargo test -p yekitrim-polish --lib \
             emoji_build_dev_tool -- --ignored --nocapture   (dry_run={dry} limit={limit:?})"
        );
        let r = super::cmd_emoji_build(catalog, dry, limit, drop_legacy, all)
            .await
            .expect("emoji 分类失败");
        println!(
            "total={} batches={} classified={} out={:?}",
            r.total, r.batches, r.classified, r.out_path
        );
        for d in crate::assets::emoji_taxonomy::taxonomy().dims.iter() {
            let prefix = format!("{}.", d.key);
            let row: Vec<String> = r
                .coverage
                .iter()
                .filter(|(k, _)| k.starts_with(&prefix))
                .map(|(k, n)| format!("{}={}", k.trim_start_matches(&prefix), n))
                .collect();
            println!("  [{}] {}", d.key, row.join(" "));
        }
        let zero: Vec<&String> = r.coverage.iter().filter(|(_, n)| *n == 0).map(|(k, _)| k).collect();
        println!("零覆盖 {} 个: {:?}", zero.len(), zero);
        println!("词表外被丢 {} 条: {:?}", r.dropped.len(), r.dropped);
    }
}

/// 扫描本地贴纸目录生成/更新 stickers.json 清单
pub async fn cmd_stickers_scan(llm: bool) -> anyhow::Result<()> {
    use crate::assets::sticker_index::{StickerIndex, StickerMeta};
    let dir = yekitrim_core::owner::sticker_dir_readable(yekitrim_core::owner::Owner::from_env());
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
        let llm_cfg = yekitrim_core::config::load_llm_config();
        if llm_cfg.api_key.is_empty() || llm_cfg.api_key == "sk-your-api-key-here" {
            tracing::info!("  ⚠ 未配置 LLM API key (编辑 .yekitrim/llm.toml), tags 留空");
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
    cfg: &yekitrim_core::config::LlmConfig,
    metas: &mut [crate::assets::sticker_index::StickerMeta],
    cancel: Option<yekitrim_core::llm::agent::CancelProbe>,
) -> anyhow::Result<()> {
    use yekitrim_core::llm::LlmClient;
    // 链路解析：零散单发链路（见 [llm.overrides.utility]）。
    // 以前这里是裸 `LlmClient::new`（不发送 max_tokens/temperature/extra_params）——
    // 于是这条链路既不受 [llm] 也不受链路覆盖约束，改成与其它调用点同一条路。
    let linked = cfg.for_link(yekitrim_core::config::LlmLink::Utility);
    let client = LlmClient::from_config(&linked)?
        .with_usage_context(yekitrim_core::llm::usage::UsageContext::new("贴纸扫描标签"));
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

/// `yekitrim fonts list`：列出字体库（含**没下载的自带字体**）。
///
/// ⚠️ 判据全走 `yekitrim_core::fonts`（`scan_all` / `family_for_at` / `builtin_status`），
/// 本层只负责排版 —— 库的形状只有一份，别在这里重新解释一遍。
///
/// ⚠️ "未安装"必须**看得见**：`scan_all` 会把清单里有、盘上没有的自带字体也列出来，
/// 这里要把它们标成 `未安装（可下载）`，否则用户以为库里空空如也。
pub fn cmd_fonts_list() -> anyhow::Result<()> {
    // ⚠️ **两层**（用户库优先、全局补缺）：自带字体装在 `global/` 里，
    // 只看用户库会把它们列成"未安装（可下载）"—— 用户会去点那个按钮，而它其实早就装好了。
    let owner = yekitrim_core::owner::Owner::from_env();
    let roots = yekitrim_core::owner::asset_dirs(owner, yekitrim_core::owner::AssetKind::Fonts);
    let root = roots[0].clone();
    tracing::info!("🔤  字体库: {}", root.display());
    if let Some(g) = roots.get(1) {
        tracing::info!("     公用（全局）: {}", g.display());
    }
    let list = yekitrim_core::fonts::scan_all_roots(&roots);
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
            roots
                .iter()
                .map(|r| yekitrim_core::fonts::family_for_at(r, &e.id))
                .find(|s| !s.is_empty())
                .unwrap_or_default(),
            e.display_name,
            if e.installed { format!("{}KB", e.bytes / 1024) } else { "-".to_string() },
            tags
        );
    }
    if installed == 0 {
        tracing::info!("  💡 一支都没有？跑 `yekitrim fonts install` 装上自带字体（已内嵌在程序里，不需要联网）");
    }
    Ok(())
}

/// `yekitrim fonts install [--id X]`：装上自带字体（把**内嵌字节**写进字体库，幂等、不联网）。
///
/// ⚠️ 根目录走 `ensure_dir_for(Owner::from_env())`（它负责 `create_dir_all`）。
/// **必须带 owner**：这条与 server 的 `/polish/fonts/builtin`（修复入口）是同一件事的
/// 两个入口，装到不同的库上就会出现"CLI 说装好了、界面说没有"。
/// 身份经 `YEKITRIM_OWNER` 从父进程继承（单人下就是 `Local`）。
///
/// ⚠️ 2026-10-02 从"下载"改成"内嵌"：正常情况下**根本不该需要这条命令** ——
/// 首次启动 `ensure_builtin_fonts` 已经自动装好了。它留作**修复入口**
/// （用户手删了库里那支、或落盘失败之后想补回来）。
pub fn cmd_fonts_install(id: Option<&str>) -> anyhow::Result<()> {
    let root = yekitrim_core::fonts::ensure_dir_for(yekitrim_core::owner::Owner::from_env())?;
    let want: Vec<&yekitrim_core::fonts::BuiltinFont> = match id {
        Some(id) => {
            let hit: Vec<_> = yekitrim_core::fonts::builtin_fonts()
                .iter()
                .filter(|b| b.id.eq_ignore_ascii_case(id.trim()))
                .collect();
            if hit.is_empty() {
                anyhow::bail!("不是自带字体（自带清单里没有它）: {id}");
            }
            hit
        }
        None => yekitrim_core::fonts::builtin_fonts().iter().collect(),
    };
    let mut n = 0usize;
    for b in want {
        if yekitrim_core::fonts::builtin_installed(&root, b) {
            tracing::info!("  ✓ {} 已安装，跳过", b.id);
            continue;
        }
        tracing::info!("  ⬇ 安装 {}（{}）…", b.display_name, b.license);
        yekitrim_core::fonts::install_builtin_font(&root, b)?;
        n += 1;
        tracing::info!("  ✓ {} 已安装到 {}/", b.id, root.display());
    }
    if n == 0 {
        tracing::info!("  没有需要安装的字体（都装好了）");
    } else {
        tracing::info!("  🎉 完成：本次安装 {n} 支");
    }
    Ok(())
}
