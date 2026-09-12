use std::path::Path;

use crate::{assets, parse, project};

mod sfx;
mod download;
mod fx;
pub use sfx::*;
pub use download::*;
pub use fx::*;
use self::download::try_resolve_sticker;

pub async fn cmd_process(project_path: &Path) -> anyhow::Result<()> {
    tracing::info!("🔧  Post-processing...");

    let mut config = project::load_project(project_path)?;
    let data_dir = project_path.join("data");

    // 音效屏蔽列表: 全局 (.vtrim/config.toml) ∪ 项目 (polish.toml [sfx])
    let sfx_blacklist = load_sfx_blacklist(&config);

    // 读取字幕数据 (用于后续处理)
    let mut subtitles: Vec<parse::srt::Subtitle> = {
        let p = data_dir.join("subtitles.json");
        if !p.exists() { anyhow::bail!("Run 'vtrim polish init' first — missing {}", p.display()); }
        serde_json::from_str(&std::fs::read_to_string(&p)?)?
    };

    // 手动屏蔽区间内的字幕不参与后续预算/位置计算
    if !config.mute_ranges.is_empty() {
        let before = subtitles.len();
        subtitles.retain(|s| !config.mute_ranges.iter().any(|r| r.overlaps(s.start, s.end)));
        if before != subtitles.len() {
            tracing::info!("  ✂ 已过滤 {} 条屏蔽区字幕", before - subtitles.len());
        }
    }

    // 先把屏蔽区间从分段中切开/移除，后续所有 position/budget/process 都基于清理后的 segments
    if !config.mute_ranges.is_empty() {
        let before = config.segments.len();
        let ranges = config.mute_ranges.clone();
        project::split_segments_by_mute_ranges(&mut config.segments, &ranges);
        if config.segments.len() != before {
            tracing::info!("  ✂ 已从 segments 中分离屏蔽区间（{} -> {} 段）", before, config.segments.len());
        }
    }

    // Step 0: 归一化瞬时特效 duration（blur 等只允许短暂脉冲）
    normalize_fx_durations(&mut config.segments);

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

    // Step 1b: 根据字幕数 + 视频时长限制 sticker/sfx 总数（均匀抽稀保留）
    {
        let total_duration = config.segments.last().map(|s| s.end).unwrap_or(0.0);
        let seg_count = config.segments.len();
        let (sticker_budget, sfx_budget) = content_budget(subtitles.len(), total_duration, Some(seg_count));

        let removed_stickers = cap_events(
            &mut config.segments,
            |ev| matches!(ev, crate::project::Event::Sticker { .. }),
            sticker_budget,
        );
        if removed_stickers > 0 {
            tracing::info!("  ⚠ sticker 超出限制({}): 删除 {} 个 (保留均匀分布)", sticker_budget, removed_stickers);
        }

        let removed_sfx = cap_events(
            &mut config.segments,
            |ev| matches!(ev, crate::project::Event::Sfx { .. }),
            sfx_budget,
        );
        if removed_sfx > 0 {
            tracing::info!("  ⚠ sfx 超出限制({}): 删除 {} 个 (保留均匀分布)", sfx_budget, removed_sfx);
        }
    }

    // Step 2: 根据 sticker_source 分配来源 + 解析 keyword → name (轮询 + 逐源兜底)
    {
        let sources = config.sticker_source.clone();
        if !sources.is_empty() {
            let client = reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(15))
                .user_agent("vtrim-polish/0.1")
                .build()
                .unwrap_or_default();
            let catalog = crate::assets::emoji_search::EmojiCatalog::fetch(&client).await.ok();
            let sticker_dir = project_path.join("assets").join("stickers");
            std::fs::create_dir_all(&sticker_dir).ok();

            // 已使用的 emoji codepoint (含历史 name, 保证去重幂等)
            let mut used_emojis: std::collections::HashSet<String> = config.segments.iter()
                .flat_map(|seg| seg.events.iter())
                .filter_map(|ev| match ev {
                    crate::project::Event::Sticker { name: Some(n), .. }
                        if !n.is_empty() && n.len() <= 8 && n.chars().all(|c| c.is_ascii_hexdigit()) => Some(n.clone()),
                    _ => None,
                })
                .collect();

            // 轮询指针: 每个 sticker 从指针位置依序尝试各来源, 首个成功者生效; 指针每次 +1 保证均衡
            let mut rotation = 0usize;
            let mut distribution: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
            let mut unresolved: Vec<String> = vec![];

            for seg in &mut config.segments {
                for event in &mut seg.events {
                    let crate::project::Event::Sticker { keyword: kw, name, source, time, .. } = event else { continue };
                    if kw.is_empty() { continue; }
                    let needs_name = match name {
                        Some(n) => n.is_empty() || n == "auto",
                        None => true,
                    };
                    if !needs_name { continue; }

                    let mut chosen: Option<(String, String)> = None;
                    for k in 0..sources.len() {
                        let src = sources[(rotation + k) % sources.len()].clone();
                        if let Some(name) = try_resolve_sticker(
                            &src, kw, *time, &catalog, &sticker_dir, &client, &mut used_emojis,
                        ).await {
                            chosen = Some((src, name));
                            break;
                        }
                    }
                    rotation += 1;

                    match chosen {
                        Some((src, resolved_name)) => {
                            *source = Some(src.clone());
                            *name = Some(resolved_name.clone());
                            *distribution.entry(src.clone()).or_insert(0) += 1;
                            tracing::info!("  ✓ sticker[{}] 「{}」→ {}", src, kw, resolved_name);
                        }
                        None => {
                            unresolved.push(kw.clone());
                            tracing::info!("  ⚠ sticker 「{}」: 所有来源均未找到匹配", kw);
                        }
                    }
                }
            }

            // Phase 2: 再平衡 — 单来源不超过 ceil(已解析/来源数), 超限来源把贴纸让给最缺的来源
            // (仅当接收方能匹配该关键词时才让渡, 不牺牲匹配质量)
            let total_resolved: usize = distribution.values().sum();
            if total_resolved > 0 && sources.len() >= 2 {
                let cap = total_resolved.div_ceil(sources.len());
                let mut rebalanced = 0usize;
                let mut dead: std::collections::HashSet<String> = std::collections::HashSet::new();
                loop {
                    let from = distribution.iter()
                        .filter(|(_, c)| **c > cap)
                        .max_by_key(|(_, c)| **c)
                        .map(|(k, _)| k.clone());
                    let to = sources.iter()
                        .filter(|s| !dead.contains(*s))
                        .filter(|s| distribution.get(*s).copied().unwrap_or(0) < cap)
                        .min_by_key(|s| distribution.get(*s).copied().unwrap_or(0))
                        .cloned();
                    let (Some(from), Some(to)) = (from, to) else { break };
                    if from == to { break; }

                    let mut swapped = false;
                    'swap: for seg in &mut config.segments {
                        for event in &mut seg.events {
                            let crate::project::Event::Sticker { keyword: kw, name, source, time, .. } = event else { continue };
                            if source.as_deref() != Some(from.as_str()) || kw.is_empty() { continue; }
                            if let Some(new_name) = try_resolve_sticker(
                                &to, kw, *time, &catalog, &sticker_dir, &client, &mut used_emojis,
                            ).await {
                                *source = Some(to.clone());
                                *name = Some(new_name.clone());
                                distribution.entry(from.clone()).and_modify(|c| *c = c.saturating_sub(1));
                                *distribution.entry(to.clone()).or_insert(0) += 1;
                                tracing::info!("  ⇄ sticker「{}」{} → {} ({})", kw, from, to, new_name);
                                swapped = true;
                                rebalanced += 1;
                                break 'swap;
                            }
                        }
                    }
                    if swapped {
                        dead.clear(); // 分布变化, 重新评估所有来源
                    } else {
                        dead.insert(to.clone()); // 该来源无法吸收, 标记枯竭
                    }
                }
                if rebalanced > 0 {
                    let dist: Vec<String> = distribution.iter()
                        .map(|(k, v)| format!("{} {}", k, v))
                        .collect();
                    tracing::info!("  ⇄ 再平衡完成 (上限 {}): {}", cap, dist.join(" / "));
                }
            }

            if !distribution.is_empty() || !unresolved.is_empty() {
                let dist: Vec<String> = distribution.iter()
                    .map(|(k, v)| format!("{} {}", k, v))
                    .collect();
                tracing::info!("  ✓ 贴纸分布: {}; 未解析 {} 个: {:?}", dist.join(" / "), unresolved.len(), unresolved);
            }
        }
    }

    // Step 2b: 计算 sticker 的 auto 位置
    // 使用 sticker_area 的九宫格独立矩形列表，随机选一个格子并在其内落点；
    // 不再用 liver 反推，避免多格子合并把 center 也包含进去。
    {
        let parse_pct = |s: &str| -> f64 {
            s.trim_end_matches('%').parse::<f64>().unwrap_or(50.0)
        };
        let sticker_rects = crate::project::region_rects(&config.layout.sticker_area);
        let fallback_rect = (
            crate::project::Point { x: "5%".into(), y: "5%".into() },
            crate::project::Point { x: "95%".into(), y: "95%".into() },
        );

        for seg in &mut config.segments {
            for (idx, event) in seg.events.iter_mut().enumerate() {
                let (x, y) = match event {
                    crate::project::Event::Sticker { x, y, .. } => (x, y),
                    _ => continue,
                };
                // process 每次都强制重新计算位置，不保留旧百分比
                let seed = (idx as f64 * 3.7 + seg.start * 1.3).fract();
                let rects = if sticker_rects.is_empty() { vec![fallback_rect.clone()] } else { sticker_rects.clone() };
                let (tl, br) = &rects[((seed * 1000.0) as usize) % rects.len()];
                let x0 = parse_pct(&tl.x);
                let y0 = parse_pct(&tl.y);
                let x1 = parse_pct(&br.x);
                let y1 = parse_pct(&br.y);

                let px = x0 + seed * (x1 - x0 - 8.0) + 4.0;
                let py = y0 + ((seed * 7.3).fract()) * (y1 - y0 - 8.0) + 4.0;

                *x = format!("{:.0}%", px.clamp(x0 + 2.0, (x1 - 2.0).max(x0 + 2.0)));
                *y = format!("{:.0}%", py.clamp(y0 + 2.0, (y1 - 2.0).max(y0 + 2.0)));
            }
        }

        // Step 2c: sticker duration 自适应
        for seg in &mut config.segments {
            for event in &mut seg.events {
                let et = event.get_time();
                let duration = match event {
                    crate::project::Event::Sticker { duration, .. } => Some(duration),
                    _ => None,
                };
                if let Some(dur) = duration {
                    let base_end = et + *dur;
                    let max_extension = *dur * 0.5;
                    if let Some(ns) = subtitles.iter()
                        .find(|s| s.end > base_end - 0.001
                            && s.end <= base_end + max_extension
                            && s.start > et)
                    {
                        // 取整到 2 位小数, 避免浮点噪声 (如 2.639999999999999)
                        *dur = ((ns.end - et) * 100.0).round() / 100.0;
                    }
                }
            }
        }
        if config.segments.iter().any(|seg| seg.events.iter().any(|e| matches!(e, crate::project::Event::Sticker { .. }))) {
            tracing::info!("  ✓ sticker 位置已计算 (sticker_area, liver对侧 y对齐origin)");
        }
    }

    // Step 3: 规则引擎决定音效 — 按字幕上下文匹配关键词, 命中则设置引用并解析,
    // 未命中则删除独立 sfx 事件 / 清空特效的 sfx (宁缺毋滥, 所有声音必须有字幕依据)
    {
        let mut removed_sfx = 0usize;
        let mut cleared_fx = 0usize;
        // 本片已用音效变体（未使用优先轮换，避免听觉重复）
        let mut used_sfx: std::collections::HashSet<String> = std::collections::HashSet::new();
        for seg in &mut config.segments {
            let mut kept: Vec<crate::project::Event> = Vec::with_capacity(seg.events.len());
            for mut event in seg.events.drain(..) {
                let has_sfx_slot = match &event {
                    crate::project::Event::Sfx { .. }
                    | crate::project::Event::Sticker { .. }
                    | crate::project::Event::Shake { .. }
                    | crate::project::Event::Flash { .. }
                    | crate::project::Event::Blur { .. }
                    | crate::project::Event::ZoomBounce { .. }
                    | crate::project::Event::Vignette { .. }
                    | crate::project::Event::HueShift { .. }
                    | crate::project::Event::Stutter { .. }
                    | crate::project::Event::Split { .. }
                    | crate::project::Event::Noise { .. }
                    | crate::project::Event::Pulse { .. }
                    | crate::project::Event::ColorShift { .. }
                    | crate::project::Event::Overlay { .. } => true,
                    _ => false,
                };
                if !has_sfx_slot {
                    kept.push(event);
                    continue;
                }

                let et = event.get_time();
                let is_standalone_sfx = matches!(event, crate::project::Event::Sfx { .. });

                // 事件是否已带音效引用 (LLM 判断选择): 优先解析 LLM 的选择, 规则只兜底
                let has_ref = event_has_sfx_ref(&event);

                if has_ref {
                    // ① LLM 已选音效 → 直接解析 (tag→id, 黑名单过滤, 失败移除/清空)
                    resolve_event_sfx(&mut event, &sfx_blacklist, &mut used_sfx);
                    if is_standalone_sfx
                        && let crate::project::Event::Sfx { name, .. } = &event
                        && name.is_empty()
                    {
                        removed_sfx += 1;
                        continue; // 解析失败 → 删除整个事件
                    }
                    kept.push(event);
                    continue;
                }

                // ② 无引用 → 规则引擎按字幕上下文兜底
                let rule_tag = subtitle_context(&subtitles, et)
                    .iter()
                    .find_map(|text| match_sfx_rule(text))
                    .map(|s| s.to_string());

                match rule_tag {
                    Some(tag) => {
                        // 设置音效引用, 然后正常解析
                        match &mut event {
                            crate::project::Event::Sfx { name, .. } => *name = tag,
                            _ => set_sfx_field(&mut event, Some(tag)),
                        }
                        resolve_event_sfx(&mut event, &sfx_blacklist, &mut used_sfx);
                        if is_standalone_sfx
                            && let crate::project::Event::Sfx { name, .. } = &event
                            && name.is_empty()
                        {
                            removed_sfx += 1;
                            continue; // 解析失败 → 删除整个事件
                        }
                        kept.push(event);
                    }
                    None => {
                        if is_standalone_sfx {
                            removed_sfx += 1;
                            tracing::info!("  ✂ sfx @{:.1}s: 字幕无匹配关键词, 删除事件", et);
                        } else {
                            cleared_fx += 1;
                            set_sfx_field(&mut event, None);
                            tracing::info!("  ✂ 特效 @{:.1}s: 字幕无匹配关键词, 移除音效", et);
                            kept.push(event);
                        }
                    }
                }
            }
            seg.events = kept;
        }
        if removed_sfx > 0 {
            tracing::info!("  ✂ 删除 {} 个无匹配的 sfx 事件", removed_sfx);
        }
        if cleared_fx > 0 {
            tracing::info!("  ✂ 清空 {} 个特效的未匹配音效", cleared_fx);
        }

        // Step 3b: 自动补位 — 字幕命中规则但 LLM 未放事件的时刻, 自动创建 sfx 事件 (预算内)
        let total_duration = config.segments.last().map(|s| s.end).unwrap_or(0.0);
        let existing_sfx_count: usize = config.segments.iter()
            .flat_map(|seg| seg.events.iter())
            .filter(|e| matches!(e, crate::project::Event::Sfx { .. }))
            .count();
        let (_sticker_budget, sfx_budget) = content_budget(subtitles.len(), total_duration, Some(config.segments.len()));
        let remaining = sfx_budget.saturating_sub(existing_sfx_count);
        if remaining > 0 {
            let all_events: Vec<crate::project::Event> = config.segments.iter()
                .flat_map(|seg| seg.events.iter().cloned())
                .collect();
            let moments = find_uncovered_sfx_moments(&subtitles, &all_events, remaining);
            let mut added = 0usize;
            for (t, tag) in moments {
                let mut ev = crate::project::Event::Sfx {
                    time: t,
                    name: tag.to_string(),
                    volume: Some(0.3),
                    duration: Some(1.0),
                };
                resolve_event_sfx(&mut ev, &sfx_blacklist, &mut used_sfx);
                if let crate::project::Event::Sfx { name, .. } = &ev {
                    if name.is_empty() {
                        continue; // 解析失败 (如整族被屏蔽)
                    }
                }
                // 放入事件时间对应的 segment (按 start/end 范围)
                if let Some(seg) = config.segments.iter_mut().find(|s| t >= s.start && t < s.end) {
                    seg.events.push(ev);
                    added += 1;
                }
            }
            if added > 0 {
                tracing::info!("  ➕ 自动补位 {} 个 sfx 事件 (预算余量 {})", added, remaining);
            }
        }
    }

    // Step 4: 自动下载 sticker + SFX
    let downloader = assets::downloader::AssetDownloader::new(project_path);
    download_keyword_assets(&downloader, &config, &sfx_blacklist).await;

    // 标记 process 已完成（preview 依赖此标记决定是否先跑 process）
    config.processed = true;
    let toml_str = config.to_toml_commented()?;
    std::fs::write(project_path.join("polish.toml"), toml_str)?;
    tracing::info!("  ✓ polish.toml updated");
    Ok(())
}

/// seg_count: Some(n) 时上限 max(1, n-1) (至少留一个 segment 无特效); None (LLM prompt) 时省略
pub fn content_budget(subtitle_count: usize, duration: f64, seg_count: Option<usize>) -> (usize, usize) {
    let cap = |v: usize, seg_count: Option<usize>| -> usize {
        let v = seg_count.map_or(v, |n| v.min(n.saturating_sub(1)));
        v.max(1)
    };
    // 密度: 每 5 条字幕/12s 一个贴纸, 每 8 条字幕/15s 一个 sfx (切片节奏更紧凑)
    let sticker = (subtitle_count as f64 / 5.0).round() as usize;
    let sticker = sticker.min((duration / 12.0).ceil() as usize);
    let sfx = (subtitle_count as f64 / 8.0).round() as usize;
    let sfx = sfx.min((duration / 15.0).ceil() as usize);
    (cap(sticker, seg_count), cap(sfx, seg_count))
}

/// 将匹配 predicate 的事件抽稀到 budget 个 (均匀分布保留), 返回删除数量
pub fn cap_events(
    segments: &mut [crate::project::Segment],
    is_target: impl Fn(&crate::project::Event) -> bool,
    budget: usize,
) -> usize {
    let mut targets: Vec<(usize, usize)> = vec![];
    for (si, seg) in segments.iter().enumerate() {
        for (ei, ev) in seg.events.iter().enumerate() {
            if is_target(ev) {
                targets.push((si, ei));
            }
        }
    }
    if targets.len() <= budget {
        return 0;
    }

    let mut keep = std::collections::HashSet::new();
    for i in 0..budget {
        let idx = if budget == 1 {
            targets.len() / 2
        } else {
            i * (targets.len() - 1) / (budget - 1)
        };
        keep.insert(targets[idx]);
    }

    for (si, seg) in segments.iter_mut().enumerate() {
        let old_events = std::mem::take(&mut seg.events);
        for (ei, ev) in old_events.into_iter().enumerate() {
            if is_target(&ev) && !keep.contains(&(si, ei)) {
                continue;
            }
            seg.events.push(ev);
        }
    }
    targets.len() - budget
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::{Event, Segment};

    fn seg_with(events: Vec<Event>) -> Segment {
        Segment { start: 0.0, end: 10.0, fx: "low".into(), note: String::new(), events, extra: Default::default() }
    }

    fn sticker() -> Event {
        Event::Sticker {
            time: 1.0, duration: 2.0, keyword: "laugh".into(),
            sticker_tag: None, name: None, source: None, x: "auto".into(), y: "auto".into(),
            sfx: None, sfx_volume: None,
        }
    }

    fn sfx_ev() -> Event {
        Event::Sfx { time: 1.0, name: "pop".into(), volume: Some(0.5), duration: Some(1.0) }
    }

    fn camera() -> Event {
        Event::Camera { time: 0.0, duration: 3.0, zoom: "中".into(), origin: "50% 50%".into() }
    }

    #[test]
    fn budget_scales_with_subtitles_and_duration() {
        // clip-01 参考: 62 字幕 / 141s → 新公式 sticker 8, sfx 8
        assert_eq!(content_budget(62, 141.4, Some(9)), (8, 8));
        // 字幕少 → 预算少
        assert_eq!(content_budget(5, 30.0, Some(3)), (1, 1));
        // 长视频 + 多字幕 → 预算更多但受 segment 数约束
        assert_eq!(content_budget(300, 500.0, Some(4)), (3, 3));
        // 无 segment 约束 (LLM prompt 用): 省略 seg_cap
        assert_eq!(content_budget(300, 500.0, None), (42, 34));
        // 边界: 空输入也至少 1
        assert_eq!(content_budget(0, 0.0, Some(1)), (1, 1));
    }

    #[test]
    fn cap_events_keeps_evenly_spaced() {
        let events: Vec<Event> = (0..9).map(|_| sfx_ev()).collect();
        let mut segments = vec![seg_with(events)];
        let removed = cap_events(&mut segments, |e| matches!(e, Event::Sfx { .. }), 4);
        assert_eq!(removed, 5);
        assert_eq!(segments[0].events.len(), 4);
        // 均匀: 首尾保留
        assert!(matches!(segments[0].events[0], Event::Sfx { .. }));
        assert!(matches!(segments[0].events[3], Event::Sfx { .. }));
        // 非目标事件不受影响
        let mut segments = vec![seg_with(vec![sticker(), camera(), sfx_ev()])];
        let removed = cap_events(&mut segments, |e| matches!(e, Event::Sfx { .. }), 0);
        assert_eq!(removed, 1);
        assert_eq!(segments[0].events.len(), 2);
        assert!(matches!(segments[0].events[0], Event::Sticker { .. }));
    }

    #[test]
    fn cap_events_noop_when_within_budget() {
        let mut segments = vec![seg_with(vec![sticker(), camera()])];
        let removed = cap_events(&mut segments, |e| matches!(e, Event::Sticker { .. }), 5);
        assert_eq!(removed, 0);
        assert_eq!(segments[0].events.len(), 2);
    }

    #[test]
    fn sfx_resolution_tiers() {
        let empty: Vec<String> = vec![];
        let mut used = std::collections::HashSet::new();
        // ① catalog id 直查 (组 id)
        assert!(resolve_sfx_value("explosion", &empty, &mut used).is_some());
        // ② tag: 前缀
        let r = resolve_sfx_value("tag:爆笑搞笑", &empty, &mut used);
        assert!(r.is_some(), "tag:コメディ 应解析到 id, got {:?}", r);
        // ③ 模糊匹配: LLM 编造的自由描述词
        let r = resolve_sfx_value("night_ambient", &empty, &mut used);
        assert!(r.is_some(), "night_ambient 应模糊匹配到 id, got {:?}", r);
        // 完全无法匹配 → None
        assert!(resolve_sfx_value("zzz_zzz_zzz_zzz_zzz", &empty, &mut used).is_none());
        // 空值 → None
        assert!(resolve_sfx_value("", &empty, &mut used).is_none());
    }

    #[test]
    fn sfx_blacklist_blocks_and_skips() {
        // 屏蔽单个 id: tag:恐怖 的首个候选被屏蔽 → 换下一个
        let single = vec!["1000hz_noise".to_string()];
        let mut used = std::collections::HashSet::new();
        let r = resolve_sfx_value("tag:恐怖氛围", &single, &mut used);
        assert!(r.is_some());
        assert_ne!(r.as_deref(), Some("1000hz_noise"), "1000hz_noise 应被屏蔽");

        // 屏蔽整族: tag:恐怖 全部候选被屏蔽 → None
        let family = vec!["tag:恐怖氛围".to_string()];
        let mut used = std::collections::HashSet::new();
        let r = resolve_sfx_value("tag:恐怖氛围", &family, &mut used);
        assert!(r.is_none(), "整族屏蔽后不应解析出音效, got {:?}", r);

        // 已解析的 id 被屏蔽 → 重新解析 (走模糊匹配)
        let mut used = std::collections::HashSet::new();
        let r = resolve_sfx_value("1000hz_noise", &single, &mut used);
        assert!(r.is_some());
        assert_ne!(r.as_deref(), Some("1000hz_noise"));

        // 屏蔽不相关的音效不影响解析
        let unrelated = vec!["1000hz_noise".to_string()];
        let mut used = std::collections::HashSet::new();
        assert!(resolve_sfx_value("tag:恐怖氛围", &unrelated, &mut used).is_some());
    }

    #[test]
    fn sfx_variant_rotation_avoids_repeat() {
        // 同一组多次引用 → 变体未使用优先, 变体耗尽后拒绝 (宁缺毋滥)
        let empty: Vec<String> = vec![];
        let mut used = std::collections::HashSet::new();
        let entry = crate::assets::sfx_catalog::lookup("explosion").unwrap();
        let n = entry.variants.len();
        let mut resolved = 0;
        for _ in 0..n {
            if resolve_sfx_value("explosion", &empty, &mut used).is_some() {
                resolved += 1;
            }
        }
        assert_eq!(resolved, n, "变体应恰好全部用一遍");
        // 变体耗尽 → None (不再无限重复)
        assert!(resolve_sfx_value("explosion", &empty, &mut used).is_none());
        // 不同组不受影响
        let mut used2 = std::collections::HashSet::new();
        assert!(resolve_sfx_value("Ikkyu_san", &empty, &mut used2).is_some());
    }

    #[test]
    fn sfx_blacklist_matches_tag_family() {
        let family = vec!["tag:异世界恐怖".to_string()];
        assert!(crate::assets::sfx_catalog::is_blacklisted("1000hz_noise", &family));
        assert!(!crate::assets::sfx_catalog::is_blacklisted("explosion", &family));
        let family2 = vec!["tag:精神攻击".to_string()];
        assert!(crate::assets::sfx_catalog::is_blacklisted("going_mad", &family2));
    }

    #[test]
    fn sfx_rules_match_keywords() {
        // 场景关键词 → tag
        assert_eq!(match_sfx_rule("然后地震了"), Some("tag:爆発"));
        assert_eq!(match_sfx_rule("感觉大楼在塌"), Some("tag:破壊"));
        assert_eq!(match_sfx_rule("然后摔下去了"), Some("tag:打撃"));
        assert_eq!(match_sfx_rule("哈哈哈笑死我了"), Some("tag:コメディ"));
        assert_eq!(match_sfx_rule("然后突然有人喊"), Some("tag:驚き"));
        assert_eq!(match_sfx_rule("感觉做噩梦了"), Some("tag:恐怖"));
        assert_eq!(match_sfx_rule("有人在敲门"), Some("tag:ドア"));
        assert_eq!(match_sfx_rule("电话响了"), Some("tag:電話"));
        assert_eq!(match_sfx_rule("私信骚扰"), Some("tag:電話"));
        assert_eq!(match_sfx_rule("外面在下雨"), Some("tag:雨"));
        assert_eq!(match_sfx_rule("现在不用洗澡啊"), Some("tag:雨"), "洗澡配雨声");
        assert_eq!(match_sfx_rule("施法了"), Some("tag:魔法"));
        assert_eq!(match_sfx_rule("听到脚步声"), Some("tag:歩く"));
        assert_eq!(match_sfx_rule("对吧呵呵"), Some("tag:コメディ"));
        // VTuber 情绪词
        assert_eq!(match_sfx_rule("我要进入红温状态"), Some("tag:打撃"));
        assert_eq!(match_sfx_rule("气死了"), Some("tag:打撃"));
        assert_eq!(match_sfx_rule("太爽啦"), Some("tag:強調"));
        assert_eq!(match_sfx_rule("我感觉我是大发明家高手啊"), Some("tag:強調"));
        assert_eq!(match_sfx_rule("绷不住了"), Some("tag:コメディ"));
        assert_eq!(match_sfx_rule("当当当当"), Some("tag:演出"));
        // 无匹配 → None (宁缺毋滥)
        assert_eq!(match_sfx_rule("说我这都不会"), None);
        assert_eq!(match_sfx_rule("老师就生气了"), Some("tag:打撃"), "生气→打撃 规则");
        assert_eq!(match_sfx_rule("apex启动"), None);
        assert_eq!(match_sfx_rule("后面就走不掉了"), None, "走不掉 不应匹配掉落");
        assert_eq!(match_sfx_rule("碰到她雷点了"), None, "雷点是比喻, 不应触发雷声");
        assert_eq!(match_sfx_rule("倒了杯水"), None, "catalog 无水声");
        // 优先级: 吓 → 驚き 优先于 恐怖
        assert_eq!(match_sfx_rule("好恐怖吓死我了"), Some("tag:驚き"));
    }

    #[test]
    fn sfx_auto_fill_finds_uncovered_moments() {
        let subs = vec![
            crate::parse::srt::Subtitle { index: 1, start: 0.0, end: 3.0, text: "有人在敲门".into(), html: String::new() },
            crate::parse::srt::Subtitle { index: 2, start: 5.0, end: 8.0, text: "有人敲门吗".into(), html: String::new() },
            crate::parse::srt::Subtitle { index: 3, start: 10.0, end: 13.0, text: "然后地震了".into(), html: String::new() },
            crate::parse::srt::Subtitle { index: 4, start: 15.0, end: 18.0, text: "今天不用洗澡".into(), html: String::new() },
        ];
        // 无已有事件: 0.2s 处第一个命中, 5.2s (与 0.2 差 >2s) 第二处, 10.2s 第三处, 15.2s 第四处
        let moments = find_uncovered_sfx_moments(&subs, &[], 4);
        assert_eq!(moments.len(), 4);
        assert_eq!(moments[0].0, 0.2);
        assert_eq!(moments[0].1, "tag:ドア");
        assert_eq!(moments[1].0, 5.2);
        assert_eq!(moments[2].1, "tag:爆発");
        assert_eq!(moments[3].1, "tag:雨");
        // 预算截断
        let moments = find_uncovered_sfx_moments(&subs, &[], 2);
        assert_eq!(moments.len(), 2);
        // 已有事件覆盖 ±2s 内 → 跳过 (只有带声音的事件才算占用)
        let existing = vec![crate::project::Event::Sfx { time: 0.5, name: "x".into(), volume: None, duration: None }];
        let moments = find_uncovered_sfx_moments(&subs, &existing, 4);
        assert_eq!(moments.len(), 3, "0.2s 与 0.5s 声音事件冲突被跳过");
        assert_eq!(moments[0].0, 5.2);
        // camera 等无声音事件不挡补位
        let cam = vec![crate::project::Event::Camera { time: 1.0, duration: 3.0, zoom: "中".into(), origin: "50% 50%".into() }];
        let moments = find_uncovered_sfx_moments(&subs, &cam, 4);
        assert_eq!(moments[0].0, 0.2, "camera 不占用音效时刻");
        // 预算 0 → 空
        assert!(find_uncovered_sfx_moments(&subs, &[], 0).is_empty());
    }

    #[test]
    fn sfx_llm_reference_detection() {
        // LLM 写的 tag 引用 / catalog id 都算"已有引用"
        assert!(event_has_sfx_ref(&Event::Sfx { time: 1.0, name: "tag:爆発".into(), volume: None, duration: None }));
        assert!(event_has_sfx_ref(&Event::Sfx { time: 1.0, name: "battery1".into(), volume: None, duration: None }));
        assert!(!event_has_sfx_ref(&Event::Sfx { time: 1.0, name: String::new(), volume: None, duration: None }));
        assert!(event_has_sfx_ref(&Event::Shake { time: 1.0, intensity: 0.5, duration: None, anchor: None, sfx: Some("tag:爆発".into()), sfx_volume: None }));
        assert!(!event_has_sfx_ref(&Event::Shake { time: 1.0, intensity: 0.5, duration: None, anchor: None, sfx: None, sfx_volume: None }));
        assert!(!event_has_sfx_ref(&Event::Camera { time: 1.0, duration: 3.0, zoom: "中".into(), origin: "50% 50%".into() }));
    }

    #[test]
    fn llm_prompt_contains_sound_judgment_rules() {
        let sources = vec!["ysyk".to_string()];
        let blacklist = vec!["fear-1".to_string(), "tag:surprise".to_string()];
        // Step1 分段 prompt: 简化规则 + 黑名单
        let prompt = crate::llm::segment::build_segmentation_system_prompt(&sources);
        assert!(prompt.contains("分段规则"));
        assert!(prompt.contains("fx"));
        assert!(!prompt.contains("\"direction\":"));
        // Step1 user prompt: 大分类导航 + 黑名单
        let user_prompt = crate::llm::segment::build_segmentation_user_prompt(&[], "测试", 5, 4, &blacklist);
        assert!(user_prompt.contains("大分类与细分 tag"));
        assert!(user_prompt.contains("主播情绪反应"));
        assert!(user_prompt.contains("fear-1"));
        assert!(user_prompt.contains("禁止使用"));
    }

    #[test]
    fn sfx_context_ordered_by_distance() {
        let subs = vec![
            crate::parse::srt::Subtitle { index: 1, start: 0.0, end: 2.0, text: "开场".into(), html: String::new() },
            crate::parse::srt::Subtitle { index: 2, start: 5.0, end: 7.0, text: "然后地震了".into(), html: String::new() },
            crate::parse::srt::Subtitle { index: 3, start: 10.0, end: 12.0, text: "哈哈哈".into(), html: String::new() },
        ];
        // 命中包含事件时间的字幕
        let ctx = subtitle_context(&subs, 6.0);
        assert_eq!(ctx.first().copied(), Some("然后地震了"));
        // 最近字幕优先
        let ctx = subtitle_context(&subs, 4.5);
        assert_eq!(ctx.first().copied(), Some("然后地震了"));
        let ctx = subtitle_context(&subs, 9.0);
        assert_eq!(ctx.first().copied(), Some("哈哈哈"));
        // 窗口内捕获: 事件早于动作 3s 内也能匹配到
        let ctx = subtitle_context(&subs, 7.5);
        assert!(ctx.contains(&"然后地震了"));
        // 超窗 → 空
        assert!(subtitle_context(&subs, 20.0).is_empty());
        assert!(subtitle_context(&subs, -5.0).is_empty());
    }
}
