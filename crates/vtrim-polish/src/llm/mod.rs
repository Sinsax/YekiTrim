pub mod segment;
pub mod keywords;
pub mod events;

use std::path::PathBuf;

use crate::parse::srt::Subtitle;
use crate::project::{Segment, Event};
use vtrim_core::config::LlmConfig;
use vtrim_core::danmaku::DanmakuEntry;
use vtrim_core::llm::LlmClient;

/// 运行 LLM 全流程: 话题区间 + 关键词提取
/// LLM 直接在 segment.events 中生成事件, 不再经 reaction_rules 中转
/// sticker_source 控制提示词中是否提及 sticker 及其语言风格
/// sticker_budget/sfx_budget: 数量上限, 提示词中明确约束
/// sfx_blacklist: 被屏蔽的音效/标签, 注入 prompt
pub async fn run_llm_pipeline(
    llm_cfg: &LlmConfig,
    subtitles: &[Subtitle],
    danmaku: &[DanmakuEntry],
    event_type: &str,
    sticker_source: &[String],
    sticker_budget: usize,
    sfx_budget: usize,
    sfx_blacklist: &[String],
    checkpoint_dir: Option<&PathBuf>,
) -> anyhow::Result<(Vec<Segment>, Vec<String>, Vec<String>)> {
    let project = checkpoint_dir.map(|d| d.parent().unwrap_or(d).to_path_buf());
    // 用量归因：链路名 + 项目（切片）目录
    let usage = |label: &str| {
        vtrim_core::llm::usage::UsageContext::new(label)
            .project(project.as_ref().map(|p| vtrim_core::path::display(p)))
    };

    let client = std::sync::Arc::new(
        LlmClient::from_config(llm_cfg)?
            .with_thinking(Some("disabled"))
            .with_usage_context(usage("精修分段")),
    );

    // Step 1: 分段 + 段情绪密度（全局单次调用，预算感知采样）
    println!("  ── LLM: Segmentation (type: {}) ──", event_type);
    let mut segments = match segment::generate_segments(
        &client, subtitles, event_type, sticker_source, sticker_budget, sfx_budget, sfx_blacklist,
    ).await {
        Ok(s) => { println!("  ✓ {} segments", s.len()); s }
        Err(e) => { println!("  ⚠ Segmentation failed: {}. Using default.", e); vec![default_segment()] }
    };

    // Step 2: 逐段事件生成 (core Agent, 并发 ≤3, checkpoint 续传)
    println!("  ── LLM: Per-segment events (Agent) ──");
    let candidate_texts: Vec<String> = {
        // 根据 event_type 提供通用 SFX 候选方向；Step2 仍可在候选内按字幕语义选择
        let tags = sfx_tags_for_event_type(event_type);
        let mut used_sfx: std::collections::HashSet<String> = std::collections::HashSet::new();
        segments
            .iter()
            .map(|_seg| {
                let text = crate::assets::sfx_catalog::candidates_text(&tags, &used_sfx, 32, sfx_blacklist);
                used_sfx.extend(
                    text.lines().flat_map(|l| l.split_whitespace())
                        .filter(|w| w.ends_with(".mp3"))
                        .map(|w| w.to_string()),
                );
                text
            })
            .collect()
    };

    let checkpoint_path = checkpoint_dir.map(|d| d.join("polish_events.checkpoint.json"));
    // 本地贴纸候选（只暴露真实存在的贴纸包名）
    let sticker_root = vtrim_core::path::sticker_dir();
    let has_local = sticker_source.iter().any(|s| {
        crate::assets::sticker_index::is_package_source(&sticker_root, s)
    });
    let local_sticker_text = if has_local {
        crate::assets::sticker_index::local_sticker_candidates_text(&sticker_root)
    } else {
        String::new()
    };
    let ok = events::run_event_agent(
        &mut segments,
        subtitles,
        candidate_texts,
        sticker_source,
        sfx_blacklist,
        local_sticker_text,
        llm_cfg,
        project.as_deref(),
        checkpoint_path.as_deref(),
    ).await?;
    println!("  ✓ {} segments generated", ok);

    // Step 2b: 本地贴纸按 tag 批量精挑具体文件
    let sticker_root = vtrim_core::path::sticker_dir();
    let has_local = sticker_source.iter().any(|s| {
        crate::assets::sticker_index::is_package_source(&sticker_root, s)
    });
    if has_local {
        run_sticker_choice_step(&mut segments, subtitles, llm_cfg, project.as_deref()).await?;
    }

    // Step 3: 关键词提取
    println!("  ── LLM: Keyword extraction ──");
    let sub_texts: Vec<String> = subtitles.iter().map(|s| s.text.clone()).collect();
    let dm_texts: Vec<String> = danmaku.iter().map(|d| d.text.clone()).collect();
    // 单开一个带自身归因的客户端：与分段那一路在用量表里分开统计
    let kw_client = LlmClient::from_config(llm_cfg)?
        .with_thinking(Some("disabled"))
        .with_usage_context(usage("精修关键词"));
    let (dm_filter, hl_words) = match keywords::extract_keywords(&kw_client, &sub_texts, &dm_texts).await {
        Ok(k) => { println!("  ✓ Keywords extracted"); k }
        Err(e) => { println!("  ⚠ Keywords failed: {}. Using defaults.", e); (vec!["OBS".into(), "游戏".into()], vec!["OBS".into()]) }
    };

    // Step 4: 补默认 Camera event (如果 LLM 没有生成)
    for seg in &mut segments {
        if seg.end - seg.start < 1.0 { continue; }
        if !seg.events.iter().any(|e| matches!(e, Event::Camera { .. })) {
            seg.events.push(Event::Camera {
                time: seg.start, end: Some(seg.end),
                zoom: "中".into(), origin: "50% 50%".into(),
                // 聚焦主播：由 process 解析成坐标（origin 是解析前的占位值）
                anchor: Some("liver".into()), anchor_to: None, to_origin: None, cut: false,
                manual: false,
            });
        }
    }

    // Step 4b: 画中画三件事（降频 + 对齐字幕边界 + 到点收工；LLM 是按段窗口生成的，
    // 数不出"整片几次"、也看不见下一条运镜隔多远）—— 落盘前先裁一遍，编辑器里看到的就已经是
    // 合规的；cmd_process 里还会再兜一次（幂等）。
    {
        let demoted = crate::process::cap_pip_shots(&mut segments);
        if demoted > 0 {
            tracing::info!("  ⚠ 画中画降级 {} 条（整片只允许 {} 条）",
                demoted, crate::process::MAX_PIP_SHOTS);
        }
        let aligned = crate::process::align_pip_shots(&mut segments, subtitles);
        if aligned > 0 {
            tracing::info!("  · 画中画已对齐字幕边界 {} 条", aligned);
        }
        let closed = crate::process::close_pip_shots(&mut segments);
        if closed > 0 {
            tracing::info!("  · 画中画到点收工，已补回镜头 {} 条", closed);
        }
        let stickers = crate::process::align_stickers_to_subtitles(&mut segments, subtitles);
        if stickers > 0 {
            tracing::info!("  · 贴纸已对齐最近字幕 {} 条", stickers);
        }
    }

    // Step 5: emoji/irasu 位置计算 ("auto" → 计算值, 未来实现)
    // LLM 输出的 x/y 如果是 "auto", 由 cmd_process 统一计算

    Ok((segments, dm_filter, hl_words))
}

/// Step2b：读取本地贴纸包 TOML，按 `(包名, sticker_tag)` 分组批量精挑具体文件，写回 event.name/source。
async fn run_sticker_choice_step(
    segments: &mut [Segment],
    subtitles: &[Subtitle],
    llm_cfg: &LlmConfig,
    project: Option<&std::path::Path>,
) -> anyhow::Result<()> {
    use crate::assets::sticker_index::{load_package_meta, tag_files};
    use crate::llm::events::{choose_sticker_files_batch, StickerChoiceContext};
    use std::collections::BTreeMap;

    let root = vtrim_core::path::sticker_dir();

    // 收集所有带 sticker_tag 的本地贴纸事件，按 (包名, tag) 分组
    // key: (package_name, tag)
    let mut groups: BTreeMap<(String, String), Vec<StickerChoiceContext>> = BTreeMap::new();
    let mut package_names: std::collections::HashSet<String> = std::collections::HashSet::new();
    for (si, seg) in segments.iter().enumerate() {
        for (ei, ev) in seg.events.iter().enumerate() {
            if let Event::Sticker {
                keyword,
                sticker_tag: Some(tag),
                time,
                ..
            } = ev
            {
                let kw = keyword.trim().to_string();
                let tag = tag.trim().to_string();
                if kw.is_empty() || tag.is_empty() {
                    continue;
                }
                package_names.insert(kw.clone());
                let ctx = StickerChoiceContext {
                    segment_index: si,
                    event_index: ei,
                    time: *time,
                    context_text: subtitle_context_text(subtitles, *time),
                };
                groups.entry((kw, tag)).or_default().push(ctx);
            }
        }
    }

    for (package_name, tag) in groups.keys() {
        let Some(meta) = load_package_meta(&root, package_name) else {
            tracing::warn!("  ⚠ 贴纸包 TOML 不存在: {package_name}.toml，跳过 Step2b");
            continue;
        };
        let candidates = tag_files(&meta, tag);
        if candidates.is_empty() {
            tracing::warn!("  ⚠ 贴纸包 {package_name} 无 tag「{tag}」，跳过 Step2b");
            continue;
        }
        let contexts = groups.get(&(package_name.clone(), tag.clone())).cloned().unwrap_or_default();
        match choose_sticker_files_batch(llm_cfg, project, package_name, tag, &candidates, &contexts).await {
            Ok(chosen) => {
                for (si, ei, stem) in chosen {
                    if let Some(seg) = segments.get_mut(si)
                        && let Some(Event::Sticker { name, source, .. }) = seg.events.get_mut(ei)
                    {
                        *name = Some(stem.clone());
                        *source = Some(package_name.clone());
                    }
                }
            }
            Err(e) => {
                tracing::warn!("  ⚠ Step2b 批量选择失败 (tag={tag}): {e:#}");
            }
        }
    }

    let _ = package_names; // 保留占位，后续可用于校验
    Ok(())
}

/// 取事件时间 ±3s 内字幕文本（供 Step2b 判断）。
fn subtitle_context_text(subtitles: &[Subtitle], t: f64) -> String {
    subtitles
        .iter()
        .filter(|s| s.end >= t - 3.0 && s.start <= t + 3.0)
        .map(|s| s.text.as_str())
        .collect::<Vec<_>>()
        .join(" ")
}

fn default_segment() -> Segment {
    Segment {
        start: 0.0, end: 10.0, fx: "low".into(),
        note: "Auto".into(), events: vec![],
        extra: Default::default(),
    }
}

/// 根据 clip 的 event_type 选择通用 SFX tag 方向（确定性规则，LLM 不再输出）。
fn sfx_tags_for_event_type(event_type: &str) -> Vec<String> {
    match event_type {
        "reaction" | "rant" => vec![
            "爆笑搞笑".into(), "惊吓事件".into(), "胜负强调".into(),
            "精神攻击".into(), "打击格斗".into(),
        ],
        "gameplay" => vec![
            "爆炸".into(), "激光射击".into(), "魔法".into(),
            "游戏系统音".into(), "怪物咆哮".into(),
        ],
        "music" => vec!["乐器演奏".into(), "演奏会".into(), "戏剧转场".into()],
        "irl" => vec![
            "人体动作".into(), "房门开合".into(), "房间杂物".into(),
            "脚步声".into(), "电话".into(),
        ],
        "interaction" | "chat" => vec![
            "人声效果".into(), "爆笑搞笑".into(), "敲击".into(), "开合物品".into(),
        ],
        "storytelling" | "discussion" => vec![
            "恐怖氛围".into(), "风".into(), "鸟".into(), "雨雷".into(),
        ],
        _ => vec![
            "爆笑搞笑".into(), "惊吓事件".into(), "胜负强调".into(),
            "人声效果".into(), "开合物品".into(),
        ],
    }
}
