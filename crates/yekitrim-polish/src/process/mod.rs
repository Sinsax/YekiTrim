//! polish::process：`polish process` 后处理主流程（LLM 写完事件之后、出片之前的全部确定性处理）。
//!
//! 入口：`cmd_process`、`content_budget`、`cap_events`；子模块 `sfx`（音效规则）、`fx`（守卫与对齐）、`download`（贴纸/SFX 下载，重导出）、`placement`（落点几何，私有）。
//! 边界：Step 顺序是硬约束（2b1 先于 2b、2d 后于 2b/2c、1c 的配额守卫后于 Step 1）；带 `manual` 的手工事件不被后处理改写，抽稀也要跳过 —— 见 `crates/yekitrim-polish/src/process/AGENTS.md`

use std::path::Path;

use crate::{assets, parse, project};

mod sfx;
mod download;
mod fx;
mod placement;
/// 剪辑质量分（总方案 §3）：纯函数、无 IO。**不做成 `pub use`** —— 它不是后处理步骤，
/// 不参与 `cmd_process`，只是"算出来给人看"的一层，调用方显式走 `process::quality::`。
pub mod quality;
pub use sfx::*;
pub use download::*;
pub use fx::*;
use self::download::try_resolve_sticker;

pub async fn cmd_process(project_path: &Path) -> anyhow::Result<()> {
    tracing::info!("🔧  Post-processing...");

    let mut config = project::load_project(project_path)?;
    let data_dir = project_path.join("data");

    // 音效屏蔽列表: 全局 (.yekitrim/config.toml) ∪ 项目 (polish.toml [sfx])
    let sfx_blacklist = load_sfx_blacklist(&config);

    // 读取字幕数据 (用于后续处理)
    let mut subtitles: Vec<parse::srt::Subtitle> = {
        let p = data_dir.join("subtitles.json");
        if !p.exists() { anyhow::bail!("Run 'yekitrim polish init' first — missing {}", p.display()); }
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

    // **屏蔽区间只做标记，不切段、不删事件**（用户要求）：`mute_ranges` 由**消费侧**生效 ——
    // 出片在合流之后 `Ffmpeg::cut_ranges` 把这几段剪掉、预览跳过它、字幕在上面已经过滤过。
    //
    // ⚠️ 这里以前会 `split_segments_by_mute_ranges`：把屏蔽区里的事件**从 polish.toml 里删掉**
    // （段一起切碎），而 process 会被预览/出片**隐式触发** —— 于是"随手屏蔽一段"等于
    // **永久删掉那一段的全部事件**，事后把屏蔽撤掉也回不来。**别再把这个调用加回来。**

    // 前置（Step 0 之前）：把 `data/danmaku.json` 的在屏窗口抬到"至少够它滚完"。
    // 必须排在最前 —— Step 2b 的"真实弹幕带"障碍读的就是这份文件，窗口不对，
    // 贴纸避让的那条带也就跟着不对。老工程靠这一步自愈，明细见函数注释。
    let extended = normalize_danmaku_windows(&data_dir);
    if extended > 0 {
        tracing::info!("  ✓ 弹幕在屏窗口已抬到「滚完为止」{extended} 条（data/danmaku.json）");
    }

    // Step 0: 归一化事件时间跨度（缺 end 的补默认、瞬时特效应只允许短暂脉冲）
    normalize_event_ends(&mut config.segments);
    // Step 0a: 运镜倍率迁移（旧内建默认按值换新）。那些值早被序列化写进老工程的
    // polish.toml 了，只改 `Default` 对它们无效；手改过别的一定保留。
    for (label, from, to) in migrate_camera_zoom(&mut config.style.camera) {
        tracing::info!("  · [style.camera] {label} {from} → {to}（新内建默认；要改回去手改 polish.toml）");
    }

    // Step 0b: 句末边界守卫（总方案 §2）—— 分段边界必须落在句末，不再"话说一半就切"。
    //
    // ⚠️ **必须排在这里**（Step 1 补默认运镜**之前**）：运镜/贴纸/音效的事件都按段边界生成，
    // 边界挪了就得重算；排在后面等于让所有事件挂在一个已经不存在的边界上。
    // 与 `llm/mod.rs` Step 4b 是**配对的两处**（老工程只有 process 这条腿能重跑）——
    // 同 `cap_pip_shots` 那条先例：LLM 那边管"刚生成就对齐"，这里管"每次后处理都收敛"。
    //
    // 明细带回来记进**决策理由**（§1）：用户看得见"哪一处、从哪挪到哪、凭什么"。
    let boundary_moves = align_segment_boundaries_detailed(&mut config.segments, &subtitles);
    if !boundary_moves.is_empty() {
        tracing::info!(
            "  ✂ 句末边界守卫：挪了 {} 处分段边界（原来切在句子中间）",
            boundary_moves.len()
        );
        for m in &boundary_moves {
            tracing::info!("     · {:.2}s → {:.2}s{}", m.0, m.1, if m.2 { "（句末标点）" } else { "（字幕收尾）" });
        }
    }
    // 决策理由（§1）的**顺延规则**。
    //
    // ⚠️ 为什么不能"每趟清空重算"：有**两类**决策天然只在某一趟发生，但描述的是
    //    **已经改过、且仍然生效**的事实 —— 清掉就等于用户每跑一次 process 丢一次解释：
    //    · **边界类**：守卫幂等 —— 第一次挪到句末，第二次看到"已经干净"返回 0 条；
    //    · **片头淡入类**：`intro_fade_seeded` 一次性补种 —— 补过就置位，不再命中。
    //    实测：第一趟 2 条 → 第二趟 0 条。
    // ⇒ 做法 = **旧的两类先接过来**，再叠加本趟新产生的，最后按 `kind` 去重（本趟的胜）。
    //   次数型决策（画中画降级）不在顺延范围：它描述"本趟降了几条"，上一趟的数字已经过时。
    const CARRY_OVER: [&str; 2] = [
        decision_kind::BOUNDARY_SENTENCE_END,
        decision_kind::INTRO_FADE_SEEDED,
    ];
    let mut decisions: Vec<crate::project::Decision> = config
        .decisions
        .iter()
        .filter(|d| CARRY_OVER.contains(&d.kind.as_str()))
        .cloned()
        .collect();
    decisions.extend(boundary_moves.iter().map(boundary_move_decision));
    dedup_decisions(&mut decisions);

    // Step 1: 为每段生成 Camera event (如果还没有)
    for seg in &mut config.segments {
        let sd = seg.end - seg.start;
        if sd < 1.0 { continue; }
        if seg.events.iter().any(|e| matches!(e, crate::project::Event::Camera { .. })) { continue; }
        seg.events.push(crate::project::Event::Camera {
            time: seg.start, end: Some(seg.end),
            zoom: "中".into(), origin: "50% 50%".into(),
            // 聚焦主播：由 resolve_camera_anchors 解析成坐标（origin 是解析前的占位值）
            anchor: Some("liver".into()), anchor_to: None, to_origin: None, cut: false,
            // 手持摇摆是可选项：补默认运镜从不自己开（非近景也会被守卫剔除）
            handheld: false,
            manual: false,
            speed: None,
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

    // Step 1c: 画中画三件事 —— 降频 + 对齐字幕边界 + 到点收工（用户要求：一个切片只能有一次；
    // 要对字幕/事件、不能没头没尾地断开；到点要硬切回原镜头，不能一直挂着）。
    // 配额守卫必须排在 Step 1 补默认运镜之后；对齐与收工都只作用于活下来的那条。
    // LLM 是按段窗口生成的、数不出"整片几次"，所以这条确定性守卫才是真正的兜底。
    {
        let demoted = cap_pip_shots(&mut config.segments);
        if demoted > 0 {
            tracing::info!("  ⚠ 画中画降级 {} 条（整片只允许 {} 条）", demoted, MAX_PIP_SHOTS);
            // §1 决策理由：这一条最容易被误解成"效果没做"—— 说清是**配额**而不是失败。
            decisions.push(crate::project::Decision::whole(
                decision_kind::PIP_DEMOTED,
                format!("画中画整片只允许 {} 条，超出的 {demoted} 条已降级成中景", MAX_PIP_SHOTS),
                Some("效果仍在，只是不再是画中画".to_string()),
            ));
        }
        let aligned = align_pip_shots(&mut config.segments, &subtitles);
        if aligned > 0 {
            tracing::info!("  · 画中画已对齐字幕边界 {} 条", aligned);
        }
        // 到点收工：画中画是状态机上的一个景别，不补回镜头就会一直挂到下一条运镜
        // （用户要求：画中画是短暂的戏剧效果，被间隔拖长之后很诡异）
        let closed = close_pip_shots(&mut config.segments);
        if closed > 0 {
            tracing::info!("  · 画中画到点收工，已补回镜头 {} 条", closed);
        }
    }

    // Step 1d: 手持摇摆只允许挂在近景上（用户定的门槛：那是近景专用的附加变种）。
    // 放在画中画收工之后 —— 那一步补出来的回镜头（可能承接了前一条的 handheld）也一起吃这道守卫。
    // 与画中画同一条道理：LLM 会写错景别、用户也会在编辑器里把景别改掉，靠 prompt 和界面都拦不住。
    {
        let stripped = cap_handheld_sway(&mut config.segments);
        if stripped > 0 {
            tracing::info!("  ⚠ 手持摇摆只在近景可用，已剔除 {stripped} 条（非近景）");
        }
    }

    // Step 2: 根据 sticker_source 分配来源 + 解析 keyword → name (轮询 + 逐源兜底)
    {
        let sources = config.sticker_source.clone();
        if !sources.is_empty() {
            let client = reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(15))
                .user_agent("yekitrim-polish/0.1")
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

    // Step 2b1: 贴纸对齐最近的字幕（用户要求：贴纸也要跟字幕对齐）——
    // 头尾吸到它反应的那句话上。**必须排在 2b（落点）之前**：落点是按贴纸的时间窗口
    // 算相机/字幕/其他贴纸障碍的，时间挪了就得重算。
    {
        let n = align_stickers_to_subtitles(&mut config.segments, &subtitles);
        if n > 0 {
            tracing::info!("  · 贴纸已对齐最近字幕 {} 条", n);
        }
    }

    // Step 2b: 推导贴纸落点 —— 从 人物 / 字幕 / 弹幕 反选安全区，在安全区中心撒点
    //
    // 障碍：layout.liver(人物) 与 layout.danmaku_area(弹幕区域) 在**源帧**坐标里框选
    // （选框模式下运镜暂停/缩放复位），出片时视频被运镜整体缩放，所以先按该贴纸可见
    // 窗口内的相机状态投到输出帧；字幕带与真实弹幕带在输出帧固定不动。
    // 落点写 ev.x/ev.y = 贴纸盒子（`280 × sticker_scale`，竖屏也是 280 —— 短边基准）**左上角**
    // 百分比（模板 DOM/canvas 与 webui engine.js 一致）。⚠️ `[sticker] position`（manual）
    // 是**盒子中心**口径，两者不同基准，见 placement.rs 模块头。
    // 细节见 process/placement.rs。
    {
        // 源视频尺寸：只有「主画面避让」这一项障碍需要它（决定视频在画布上占多大）。
        // 取不到不算失败 —— 该项障碍退化成不计入，落点照旧。
        let video_size = crate::render::get_video_size(
            &project_path.join(&config.source.video).to_string_lossy(),
        );
        if video_size.is_none() {
            tracing::info!("  · 读不到源视频尺寸(ffprobe), 贴纸不避让主画面");
        }
        let obstacles = sticker_obstacles(&config, &data_dir, video_size);
        if obstacles.allow.is_empty() && !config.layout.sticker_area.grid.is_empty() {
            tracing::info!("  · 忽略 layout.sticker_area.grid（落点改为按 人物/字幕/弹幕 动态推导）");
        }

        let mut placed: Vec<(f64, f64, placement::Rect)> = Vec::new();
        let mut relaxed = 0usize;
        let mut count = 0usize;
        let mut min_clearance = f64::MAX;
        let (mut min_x, mut max_x, mut min_y, mut max_y) = (f64::MAX, f64::MIN, f64::MAX, f64::MIN);

        for seg in &mut config.segments {
            for (idx, event) in seg.events.iter_mut().enumerate() {
                let (x, y, time, end) = match event {
                    crate::project::Event::Sticker { x, y, time, end, .. } => {
                        (x, y, *time, end.unwrap_or(*time))
                    }
                    _ => continue,
                };
                let blocked = obstacles.blocked_at(time, end, &placed);
                let seed = (idx as f64 * 3.7 + seg.start * 1.3).fract();
                let pick = placement::pick_sticker_position(&blocked, &obstacles.allow, seed, obstacles.frame);
                *x = pick.x_pct(obstacles.frame);
                *y = pick.y_pct(obstacles.frame);
                if pick.relaxed {
                    relaxed += 1;
                    tracing::info!("  ⚠ sticker @{time:.1}s: 帧内无安全落点, 取重叠最小 ({}, {})", &*x, &*y);
                }
                placed.push((time, end + 0.65, placement::sticker_box(pick.x, pick.y, obstacles.frame)));
                count += 1;
                min_clearance = min_clearance.min(pick.clearance);
                min_x = min_x.min(pick.x);
                max_x = max_x.max(pick.x);
                min_y = min_y.min(pick.y);
                max_y = max_y.max(pick.y);
            }
        }

        if count > 0 {
            // 障碍摘要拼成一个串，别塞进 format! —— 可选的那一项（主画面）会让占位符数量
            // 和参数对不上，而这类错误只在编译期暴露一次、改格式串时极易再犯。
            let mut avoided = format!(
                "{}×{} {}×{} {} + {}×{}",
                placement::ObstacleKind::Liver.label(),
                obstacles.liver_count,
                placement::ObstacleKind::DanmakuArea.label(),
                obstacles.panel_count,
                placement::ObstacleKind::Captions.label(),
                placement::ObstacleKind::DanmakuBand.label(),
                obstacles.danmaku_rows,
            );
            if obstacles.main_frame {
                avoided.push_str(&format!(" + {}", placement::ObstacleKind::MainFrame.label()));
            }
            tracing::info!(
                "  ✓ sticker 落点 {} 个 (安全区中心撒点{}, 最小余量 {:.0}px): 避让 {}; x {:.0}~{:.0}% y {:.0}~{:.0}%",
                count,
                if relaxed > 0 { format!(", {relaxed} 个兜底") } else { String::new() },
                min_clearance,
                avoided,
                min_x / obstacles.frame.w * 100.0,
                max_x / obstacles.frame.w * 100.0,
                min_y / obstacles.frame.h * 100.0,
                max_y / obstacles.frame.h * 100.0,
            );
            if min_clearance < 20.0 {
                tracing::info!("  ⚠ 贴纸安全区余量最小仅 {min_clearance:.0}px: 人物/弹幕区域选框贴得太近, 建议放宽选框或减少贴纸数量");
            }
        }
    }

    // Step 2c: sticker end 自适应 —— 尾帧落在紧随其后的字幕结束点, 贴纸不早退
    // （手工定过时间的贴纸跳过：改过的结束时间不再被延长）
    {
        for seg in &mut config.segments {
            for event in &mut seg.events {
                if event.is_time_manual() {
                    continue;
                }
                let et = event.get_time();
                let end = match event {
                    crate::project::Event::Sticker { end, .. } => *end,
                    _ => None,
                };
                if let Some(cur_end) = end {
                    let dur = (cur_end - et).max(0.0);
                    let base_end = et + dur;
                    let max_extension = dur * 0.5;
                    if let Some(ns) = subtitles.iter()
                        .find(|s| s.end > base_end - 0.001
                            && s.end <= base_end + max_extension
                            && s.start > et)
                    {
                        // 取整到 2 位小数, 避免浮点噪声 (如 2.639999999999999)
                        event.set_end(Some((ns.end * 100.0).round() / 100.0));
                    }
                }
            }
        }
    }

    // Step 2d: 解析运镜锚点（"聚焦主播/弹幕/贴纸" → 具体坐标）
    // 必须排在贴纸落点(2b)与时长自适应(2c)之后：anchor="sticker" 要读最终位置
    resolve_camera_anchors(&mut config, &data_dir);

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
                    | crate::project::Event::Vignette { .. }
                    | crate::project::Event::HueShift { .. }
                    | crate::project::Event::Noise { .. }
                    | crate::project::Event::Pulse { .. }
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
                    end: Some(t + 1.0),
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

    // Step 3c: 首段补一条片头淡入覆盖事件 —— 原内建片头黑场（固定 0.5s）退役成普通
    // overlay 事件之后，"开头渐入"这个默认观感就靠这一步兜住。**只在初次后处理补**：判定过片头就
    // 置位 `intro_fade_seeded`（随下面的写回落盘），往后时间轴上改/拖/删都由用户说了算 ——
    // 删掉之后再跑 process 也不会补回来（那是用户意图，不是"默认观感没生效"）。
    //
    // 为什么排在 Step 3 之后而不是 Step 0b：音效规则引擎会把**没有音效引用**的覆盖事件也扫一遍，
    // 排它前面的话这条纯视觉的黑场每跑一次都会多记一句「特效无匹配、移除音效」，字幕恰好命中
    // 规则时还会凭空给它挂一个音效 —— 补默认事件不该去动声音。
    if ensure_intro_fade_in(&mut config) {
        tracing::info!("  ➕ 首段补片头淡入（fade_in {INTRO_FADE_SECS}s，可在时间轴上改/删）");
        decisions.push(crate::project::Decision::whole(
            decision_kind::INTRO_FADE_SEEDED,
            format!("首段补了一条 {INTRO_FADE_SECS}s 的黑场淡入（只在初次后处理补一次）"),
            Some("往后改/删都归你，重跑 process 不会补回来".to_string()),
        ));
        // 顺延来的那条（上趟补的）与上面这条同类 —— 去重，保留本趟这条。
        dedup_decisions(&mut decisions);
    }
    // §1：决策理由**每趟重算后落盘**（与其它字段一起走下面的 update_polish_toml）。
    config.decisions = decisions;

    // Step 4: 自动下载 sticker + SFX
    let downloader = assets::downloader::AssetDownloader::new(project_path);
    download_keyword_assets(&downloader, &config, &sfx_blacklist).await;

    // Step 5: 冷开场 / 回忆回溯意图 → `edl.clips`（第 5 批）。
    //
    // LLM 只在段窗口内写意图（`anchor = "cold_open"` / `"recall"`），**数不出"整片几次"** ——
    // 配额与落实都在这里兜，与 `cap_pip_shots`（Step 1c）同一条先例、同一套理由。
    //
    // ⚠️ **只在"真的落实了"时才写 EDL**（`changed()`）：`edl.clips` 一旦非空，
    // `mute_ranges` 就**整个被忽略**（`ProjectFile::resolve_edl`、规格 §2.3），
    // 所以一支意图都没有的老工程**必须保持 `clips` 为空**，否则它的时间线会被无声换掉。
    // `materialize_edl_intents` 的注释里写着这条硬前提，改之前先读它。
    //
    // 排在 Step 4 之后：它只读 `segments` 的意图 + 写 `edl`，与下载/音效都不相交；
    // 位置要紧的是它在**写回之前**（`edl` 必须跟着 `segments` 一起落盘）。
    let mut edl_intent_done = false;
    {
        let total_duration = config.segments.last().map(|s| s.end).unwrap_or(0.0);
        let materialized = materialize_edl_intents(
            &mut config.segments,
            &mut config.edl.clips,
            total_duration,
        );
        if materialized.changed() {
            tracing::info!(
                "  ➕ 成片时间线：冷开场 {} · 回忆回溯 {} 条（共 {} 段）",
                if materialized.cold_open { "1 条" } else { "无" },
                materialized.recalls,
                config.edl.clips.len()
            );
            edl_intent_done = true;
        }
    }

    // 标记 process 已完成（preview 依赖此标记决定是否先跑 process）。
    // 只写回 process 负责的字段：这中间用户存的选框 / 封面 / 屏蔽轨道不能被吞掉。
    let segments = config.segments;
    let intro_fade_seeded = config.intro_fade_seeded; // Step 3c 用掉的补种时机（只从 false 变 true）
    // §1 决策理由：**必须显式写回** —— `update_polish_toml` 只覆盖它闭包里点名的字段，
    // 漏掉这一行就是"算出来了但一个字节都没落盘"（§4 那次「PUT 说已保存、磁盘一字不动」
    // 是同一个形状的坑）。空 vec 会把旧理由清掉，这正是"每趟重算"的语义。
    let decisions = config.decisions;
    // Step 5 的产物：`None` = 本趟没有落实任何意图 ⇒ **不许碰** `cfg.edl`
    // （用户在前端的编辑、以及"老工程保持为空"的 R4 判据都靠这一条）。
    let edl_clips = writeback_edl(edl_intent_done, config.edl.clips);
    project::update_polish_toml(project_path, move |cfg| {
        cfg.segments = segments;
        cfg.intro_fade_seeded = intro_fade_seeded;
        cfg.decisions = decisions;
        if let Some(clips) = edl_clips {
            cfg.edl.clips = clips;
        }
        cfg.processed = true;
    })
    .await?;
    tracing::info!("  ✓ polish.toml updated");
    Ok(())
}

/// Step 5 的**写回门**：本趟真的落实了冷开场/回忆回溯意图时，才把 `clips` 交给
/// `update_polish_toml`；否则给 `None`（调用方**一个字节都不碰** `cfg.edl`）。
///
/// 抽成具名纯函数不是为了漂亮：它是 R4 那两层的**上层**，而端到端测试**咬不住它**
/// （空 `Edl` 会被 `ProjectFile::edl` 的 `skip_serializing_if = "Edl::is_empty"` 静默吃掉，
/// 把 `then_some` 改成无条件 `Some` 之后端到端测试照样绿）。所以它必须有自己的观察点 ——
/// `process_writeback_gate_only_touches_edl_when_materialized`。
/// 同一个教训见 `docs/plan/edl-retiming-spec.md` §11.14（`ensure_assets_should_run`）。
///
/// ⚠️ 门开着也**只是"允许写"**：里面的 `clips` 已经包含用户手拖过的段（Step 5 只在它前后插），
/// 所以这个门不负责保用户数据，负责保的是"**没意图时别去动它**"。
fn writeback_edl(materialized: bool, clips: Vec<project::EdlClip>) -> Option<Vec<project::EdlClip>> {
    materialized.then_some(clips)
}

/// 贴纸避让上下文：按项目算一次，逐张贴纸再按自己的可见窗口取障碍。
struct StickerObstacles {
    /// 输出画布（`[canvas]` 推导；所有几何的坐标系）
    frame: placement::Frame,
    /// 输出帧固定障碍：字幕带 + 真实弹幕带（该片没有弹幕时不含弹幕带）
    fixed: Vec<placement::Blocked>,
    /// **源帧**障碍（人物 / 弹幕区域），需按相机状态投射到输出帧
    source: Vec<(placement::Rect, placement::ObstacleKind)>,
    shots: Vec<placement::CameraShot>,
    default_origin: (f64, f64),
    /// 手写的 sticker_area 自由选框（只作偏好，安全优先）
    allow: Vec<placement::Rect>,
    liver_count: usize,
    panel_count: usize,
    /// 该片真实弹幕条数（0 = 弹幕图层不画）
    danmaku_rows: usize,
    /// 主画面矩形是否已计入障碍（视频没铺满画布时才计）
    main_frame: bool,
}

impl StickerObstacles {
    /// 某张贴纸在 `[t0, t1]` 可见窗口内的全部障碍
    fn blocked_at(&self, t0: f64, t1: f64, placed: &[(f64, f64, placement::Rect)]) -> Vec<placement::Blocked> {
        let mut blocked = self.fixed.clone();
        for ((ox, oy), zoom) in placement::camera_states_for(&self.shots, self.default_origin, t0, t1) {
            for (rect, kind) in &self.source {
                let hit = placement::camera_expand(*rect, ox, oy, zoom).clamp_to_frame(self.frame);
                if !hit.is_degenerate() {
                    blocked.push(placement::Blocked::new(hit, *kind));
                }
            }
        }
        // 同刻已落位的贴纸互不相压
        for (p0, p1, rect) in placed {
            if *p0 < t1 && t0 < *p1 {
                blocked.push(placement::Blocked::new(*rect, placement::ObstacleKind::Sticker));
            }
        }
        blocked
    }
}

/// 汇集贴纸落点推导所需的障碍（人物 / 字幕 / 弹幕区域 / 真实弹幕带 / 主画面）
///
/// `video_size` = 源视频尺寸（ffprobe）；拿不到就传 `None` —— 只影响「主画面」这一项障碍
/// （算不出摆放就退化成改造前的行为），不会让落点失败。
fn sticker_obstacles(
    config: &project::ProjectFile,
    data_dir: &Path,
    video_size: Option<(u32, u32)>,
) -> StickerObstacles {
    let (cw, ch) = config.canvas_size();
    let frame = placement::Frame::new(cw, ch);
    // W1：`layout.<区域>` 的百分比是**主画面矩形 `sr`** 的百分比（不是整块画布）。
    // 唯一真源是模板 `stageDrawRect` 的 Rust 镜像 `project::stage_draw_rect`
    // （经 `placement::stage_rect`）—— 拿不到源尺寸时它退化成整块画布，与改造前同行为。
    let stage = config.resolve_stage(video_size);
    let sr = placement::stage_rect(&stage, video_size, (cw, ch));
    let liver_source = placement::layout_region_rects(&config.layout.liver, sr);
    let liver_rects = if liver_source.is_empty() {
        // 没框过人物 → 用运镜聚焦中心兜一个保守盒子（宁可多让）
        match project::region_origin(&config.layout.liver)
            .and_then(|o| placement::parse_origin_in_stage(&o, sr))
        {
            Some((ox, oy)) => {
                tracing::info!("  ⚠ layout.liver 无区域, 按运镜中心 ({ox:.0},{oy:.0}) 估 35%×50% 盒子");
                vec![placement::Rect::new(
                    ox - 0.175 * sr.width(), oy - 0.25 * sr.height(),
                    ox + 0.175 * sr.width(), oy + 0.25 * sr.height(),
                )]
            }
            None => Vec::new(),
        }
    } else {
        liver_source
    };
    let panel_rects = placement::layout_region_rects(&config.layout.danmaku_area, sr);

    let mut fixed = vec![placement::Blocked::new(
        placement::caption_band(
            frame,
            config.style.subtitle.size as f64,
            placement::parse_px(&config.style.subtitle.stroke),
        ),
        placement::ObstacleKind::Captions,
    )];
    let danmaku_ys = read_danmaku_ys(data_dir);
    if let Some(band) = placement::danmaku_band(
        frame,
        config.style.danmaku.font_size as f64,
        placement::parse_px(&config.style.danmaku.stroke),
        &danmaku_ys,
    ) {
        fixed.push(placement::Blocked::new(band, placement::ObstacleKind::DanmakuBand));
    }

    // 主画面：只有视频**没铺满**画布（axis = x / y）时才算障碍 ——
    // 铺满时整帧都是画面，拿它当障碍等于「处处不安全」，只会把贴纸全推进 relaxed 兜底。
    // 所以改造前的横素材项目（axis = fill）行为逐像素不变；受益的是
    // 「横画布 × 竖素材」这类主画面只占一侧的场景（原先把贴纸放到主视频身上）。
    let main_rect = placement::main_frame_rect(&stage, video_size, (cw, ch));
    if let Some(rect) = main_rect {
        fixed.push(placement::Blocked::new(rect, placement::ObstacleKind::MainFrame));
    }

    // 不动点也是 `sr` 的百分比（模板 `:1888-1889` 把它映射进 `sr`）；没框过人物时
    // 模板的兜底是 `65% 50%`（同一坐标系），所以兜底值也要按 `sr` 算。
    let default_origin = project::region_origin(&config.layout.liver)
        .and_then(|o| placement::parse_origin_in_stage(&o, sr))
        .unwrap_or((sr.x0 + sr.width() * 0.65, sr.y0 + sr.height() * 0.5));

    let mut source: Vec<(placement::Rect, placement::ObstacleKind)> = liver_rects
        .iter()
        .map(|r| (*r, placement::ObstacleKind::Liver))
        .collect();
    source.extend(panel_rects.iter().map(|r| (*r, placement::ObstacleKind::DanmakuArea)));

    StickerObstacles {
        frame,
        fixed,
        source,
        shots: camera_shots(&config.segments, default_origin, &config.style.camera, sr),
        default_origin,
        // `layout.sticker_area` 同样是**源帧**坐标（W1 覆盖全部 `layout.<区域>`），
        // 只是它的 grid 不参与、自由选框只作偏好。
        allow: project::free_rect(&config.layout.sticker_area)
            .map(|(tl, br)| vec![placement::stage_region_rect(&tl, &br, sr)])
            .unwrap_or_default(),
        liver_count: liver_rects.len(),
        panel_count: panel_rects.len(),
        danmaku_rows: danmaku_ys.len(),
        main_frame: main_rect.is_some(),
    }
}

/// 收集运镜（Camera 事件）→ 贴纸避让用的「源帧 → 输出帧」变换表。
/// zoom 系数取自 `[style.camera]`（与预览、出片同一张表）再乘瞬时特效预留。
/// `sr` = 主画面矩形：**事件里的 origin 百分比是 `sr` 的百分比**（W1），由
/// [`placement::parse_origin_in_stage`] 映射到画布坐标。
fn camera_shots(
    segments: &[project::Segment],
    default_origin: (f64, f64),
    camera_style: &project::CameraStyle,
    sr: placement::Rect,
) -> Vec<placement::CameraShot> {
    // 档位名 + 是否硬切要留到最后才用得上：过渡长度取决于**相邻两段**的档位与本条自己的 cut
    let mut pairs: Vec<(bool, String, placement::CameraShot)> = segments
        .iter()
        .flat_map(|seg| seg.events.iter())
        .filter_map(|ev| match ev {
            crate::project::Event::Camera { time, end, zoom, origin, to_origin, cut, .. } => {
                let shot = placement::CameraShot {
                    time: *time,
                    end: end.unwrap_or(*time),
                    origin: placement::parse_origin_in_stage(origin, sr).unwrap_or(default_origin),
                    origin_to: to_origin
                        .as_deref()
                        .and_then(|o| placement::parse_origin_in_stage(o, sr))
                        .unwrap_or_else(|| {
                            placement::parse_origin_in_stage(origin, sr).unwrap_or(default_origin)
                        }),
                    zoom: camera_style.factor(zoom) * placement::TRANSIENT_ZOOM,
                    transition: placement::TRANSITION_SECS,
                };
                Some((*cut, zoom.clone(), shot))
            }
            _ => None,
        })
        .collect();
    pairs.sort_by(|a, b| a.2.time.partial_cmp(&b.2.time).unwrap_or(std::cmp::Ordering::Equal));
    pairs
        .iter()
        .enumerate()
        .map(|(i, (cut, label, shot))| {
            let prev_label = if i == 0 { "" } else { pairs[i - 1].1.as_str() };
            placement::CameraShot {
                transition: placement::transition_secs(label, prev_label, *cut),
                ..*shot
            }
        })
        .collect()
}

/// 把 `data/danmaku.json` 的**在屏窗口**抬到"至少够它滚完"（见
/// `yekitrim_core::danmaku::clamp_display_windows`）。
///
/// # 为什么在这里、而不只在 `cmd_parse`
///
/// `polish.toml` 已存在的工程**没有 CLI / WebUI 入口能重跑 parse**（`cmd_init` 见到
/// `polish.toml` 直接 bail），而这些工程的 `data/danmaku.json` 是修复**之前**写的。
/// 于是消费侧那份数据只会越来越旧 —— process 是预览 / 出片 / WebUI 都会（隐式）跑一步的那道口，
/// 在这里归一化，老工程下一次精修就自愈。
///
/// ⚠️ **只在真的变了的时候写回**：首页「切片总览」按片段目录里最新文件的 mtime 排序
/// （`yekitrim_service::project_data::last_touched_secs`），每次 process 都无条件写一次
/// 会把老片段一直顶到最前面。归一化是幂等的，第二次进来 `changed == 0`、一个字节都不动。
///
/// 读不出来 / 解析失败一律**不动**（与 `read_danmaku_ys` 的降级口径一致）。
fn normalize_danmaku_windows(data_dir: &Path) -> usize {
    let path = data_dir.join("danmaku.json");
    let Ok(raw) = std::fs::read_to_string(&path) else { return 0 };
    let Ok(mut entries) = serde_json::from_str::<Vec<yekitrim_core::danmaku::DanmakuEntry>>(&raw) else {
        return 0;
    };
    let changed = yekitrim_core::danmaku::clamp_display_windows(&mut entries);
    if changed == 0 {
        return 0;
    }
    match serde_json::to_string_pretty(&entries) {
        Ok(text) => {
            if let Err(e) = crate::project::write_atomic(&path, &text) {
                tracing::info!("  ⚠ danmaku.json 在屏窗口归一化写回失败: {e}");
                return 0;
            }
            changed
        }
        Err(e) => {
            tracing::info!("  ⚠ danmaku.json 序列化失败: {e}");
            0
        }
    }
}

/// `data/danmaku.json` 的轨道高度（空/缺失/解析失败 → 无弹幕障碍）
fn read_danmaku_ys(data_dir: &Path) -> Vec<f64> {
    #[derive(serde::Deserialize)]
    struct Row {
        #[serde(default)]
        y: f64,
    }
    let path = data_dir.join("danmaku.json");
    let Ok(raw) = std::fs::read_to_string(&path) else { return Vec::new() };
    match serde_json::from_str::<Vec<Row>>(&raw) {
        Ok(rows) => rows.into_iter().map(|r| r.y).collect(),
        Err(e) => {
            tracing::info!("  ⚠ danmaku.json 解析失败 ({}), 贴纸不避让弹幕带", e);
            Vec::new()
        }
    }
}

// ===== 运镜锚点解析 =====

/// 锚点解析的只读输入。提前取好，避免与 `config.segments` 的可变遍历打架。
struct AnchorCtx {
    /// `layout.liver` 的聚焦中心（自由选框 > origin_grid > 首个格子）
    liver: Option<String>,
    /// `layout.danmaku_area` 的聚焦中心（真实弹幕缺失时的兜底）
    danmaku_area: Option<String>,
    /// 贴纸盒子中心 `(x%, y%, 起, 止)`
    stickers: Vec<(f64, f64, f64, f64)>,
    /// 弹幕轨道 `(y, 起, 止)`
    danmaku: Vec<(f64, f64, f64)>,
    /// 弹幕字号 / 描边宽（弹幕带厚度靠它算，与贴纸避让用的是同一组值）
    danmaku_font: f64,
    danmaku_stroke: f64,
    /// 输出画布（`[canvas]` 推导；坐标换算用）
    frame: placement::Frame,
}

impl AnchorCtx {
    fn collect(config: &project::ProjectFile, data_dir: &Path) -> Self {
        let frame = placement::Frame::new(config.canvas_size().0, config.canvas_size().1);
        let stickers = config
            .segments
            .iter()
            .flat_map(|seg| seg.events.iter())
            .filter_map(|ev| match ev {
                crate::project::Event::Sticker { time, end, x, y, .. } => {
                    let (cx, cy) = sticker_center_pct(x, y, frame)?;
                    Some((cx, cy, *time, end.unwrap_or(*time)))
                }
                _ => None,
            })
            .collect();
        Self {
            liver: project::region_origin(&config.layout.liver),
            danmaku_area: project::region_origin(&config.layout.danmaku_area),
            stickers,
            danmaku: read_danmaku_rows(data_dir),
            danmaku_font: config.style.danmaku.font_size as f64,
            danmaku_stroke: placement::parse_px(&config.style.danmaku.stroke),
            frame,
        }
    }

    /// 锚点名 → `"x% y%"`。未知名字返回 `None`（调用方保留原 origin 并告警）。
    fn resolve(&self, name: &str, t: f64) -> Option<String> {
        match name.trim() {
            "liver" => self.liver.clone(),
            "center" => Some("50.00% 50.00%".into()),
            // 拍那一刻真的在飞的弹幕；那一刻没有弹幕就退回用户框的弹幕区域
            "danmaku" => self.danmaku_center(t).or_else(|| self.danmaku_area.clone()),
            "sticker" => self.nearest_sticker(t),
            _ => None,
        }
    }

    /// 时刻 `t` 在屏弹幕组成的轨道带的中心。
    fn danmaku_center(&self, t: f64) -> Option<String> {
        let ys: Vec<f64> = self
            .danmaku
            .iter()
            .filter(|(_, s, e)| *e <= *s || (t >= *s - 0.001 && t <= *e + 0.001))
            .map(|(y, _, _)| *y)
            .collect();
        let band = placement::danmaku_band(self.frame, self.danmaku_font, self.danmaku_stroke, &ys)?;
        let (cx, cy) = band.center();
        Some(pct_pair(cx, cy, self.frame))
    }

    /// 时间上离 `t` 最近的贴纸盒子中心（覆盖 `t` 的优先）。
    fn nearest_sticker(&self, t: f64) -> Option<String> {
        let (mut best_d, mut best) = (f64::MAX, None);
        for (cx, cy, s, e) in &self.stickers {
            let d = if t >= *s && t <= *e {
                0.0
            } else {
                (s - t).abs().min((e - t).abs())
            };
            if d < best_d {
                best_d = d;
                best = Some((*cx, *cy));
            }
        }
        let (cx, cy) = best?;
        Some(pct_pair(
            cx / 100.0 * self.frame.w,
            cy / 100.0 * self.frame.h,
            self.frame,
        ))
    }
}

/// 贴纸 x/y（盒子**左上角**百分比，盒子边长 = `280 × sticker_scale`）→ 盒子中心百分比。
/// 未解析（"auto"）返回 None。
fn sticker_center_pct(x: &str, y: &str, frame: placement::Frame) -> Option<(f64, f64)> {
    let (xs, ys) = (x.trim(), y.trim());
    if !xs.ends_with('%') || !ys.ends_with('%') {
        return None;
    }
    let half_w = frame.sticker_size() / 2.0 / frame.w * 100.0;
    let half_h = frame.sticker_size() / 2.0 / frame.h * 100.0;
    Some((project::parse_pct(xs) + half_w, project::parse_pct(ys) + half_h))
}

fn pct_pair(x_px: f64, y_px: f64, frame: placement::Frame) -> String {
    format!(
        "{:.2}% {:.2}%",
        x_px / frame.w * 100.0,
        y_px / frame.h * 100.0
    )
}

/// `data/danmaku.json` 的 `(y, start, end)`（空/缺失/解析失败 → 空）。
/// 与 `read_danmaku_ys` 的区别：保留时间范围，供「运镜移到弹幕」按时刻取在屏轨道。
fn read_danmaku_rows(data_dir: &Path) -> Vec<(f64, f64, f64)> {
    #[derive(serde::Deserialize)]
    struct Row {
        #[serde(default)]
        y: f64,
        #[serde(default)]
        start: f64,
        #[serde(default)]
        end: f64,
    }
    let path = data_dir.join("danmaku.json");
    let Ok(raw) = std::fs::read_to_string(&path) else { return Vec::new() };
    match serde_json::from_str::<Vec<Row>>(&raw) {
        Ok(rows) => rows.into_iter().map(|r| (r.y, r.start, r.end)).collect(),
        Err(_) => Vec::new(),
    }
}

/// 历史默认聚焦中心。两个默认生成器与 LLM prompt 示例都写死这个值，
/// 从来没有 UI 能改它——所以它等价于「没指定」，可以安全迁移成聚焦主播。
/// `process::fx` 里补画中画回镜头时也用它当"没有前一条运镜"时的占位坐标（同样靠 anchor 解析）。
pub(super) const PRISTINE_ORIGIN: &str = "50% 50%";

/// 运镜三档倍率的**旧内建默认**按值迁移到新默认（画中画 0.7 → 0.55；
/// 中景 1.15 → 1.3、近景 1.3 → 1.5）。与 `resolve_camera_anchors` 里的
/// `PRISTINE_ORIGIN` 同理：这些值只可能来自默认值（UI 从来没有入口，只能手改 polish.toml），
/// 所以恰好等于它就可以安全迁移。手改过别的一定保留。
///
/// 返回实际改掉的 `(字段名, 旧值, 新值)`，供调用方打日志。幂等。
fn migrate_camera_zoom(camera: &mut project::CameraStyle) -> Vec<(&'static str, f64, f64)> {
    let new = project::CameraStyle::default();
    let mut changed = Vec::new();
    for (label, field, legacy, target) in [
        ("zoom_mid", &mut camera.zoom_mid, LEGACY_ZOOM_MID, new.zoom_mid),
        ("zoom_close", &mut camera.zoom_close, LEGACY_ZOOM_CLOSE, new.zoom_close),
        ("zoom_wide", &mut camera.zoom_wide, LEGACY_ZOOM_WIDE, new.zoom_wide),
    ] {
        if (*field - legacy).abs() < 1e-9 {
            changed.push((label, *field, target));
            *field = target;
        }
    }
    changed
}

/// 片头淡入的默认时长（秒）。原内建片头黑场（`intro_black`，固定 0.5s 正弦淡出）退役成普通
/// overlay 事件后，这个默认观感由 [`ensure_intro_fade_in`] 在**初次** `process` 时补出来；
/// 渲染侧的语义见 `yekitrim-render-core/src/timeline.rs::get_overlay_state`。
pub const INTRO_FADE_SECS: f64 = 0.5;

/// 首段补一条片头淡入覆盖事件（`overlay_type = "fade_in"`、纯黑、满不透明度）。
///
/// `fade_in` 的**整窗就是过渡本身**（起点全遮 → 末端全透，线性；不叠常规 0.4s 进出包络），
/// `opacity` = 遮罩色的最大不透明度，`color` 缺省黑。复用 overlay 通道 ⇒ 不需要新字段，
/// 也不进 `default_end` 兜底（本条自带 `end`）。
///
/// **补种时机只有一次**（用户报的"删掉之后再跑 process 又回来了"）：判据是
/// `ProjectFile::intro_fade_seeded` —— 判定过片头就置位落盘，往后片头有没有这条事件、长什么样
/// 都由用户说了算；想重新拿回默认片头就删掉 polish.toml 里那一行。**别改回"看窗口有没有覆盖事件"**：
/// 那等于"用户每删一次就再补一次"。
///
/// 补的条件（全部满足才补）：
/// - 取**时间最早**且起点 < 0.5s 的段 —— 段之间时间会重叠（LLM 按窗口生成 + 去静音切段），
///   不能拿文件顺序当片头；
/// - 该段跨度 ≥ 0.2s（塞不下一条 0.5s 淡入的段不补）；
/// - **整片**没有任何覆盖事件与这条淡入的窗口重叠（本段有、或别的段跨过来盖住它，都算片头已有表现）。
///
/// 标记**用掉**的时机 = 片头窗口真的判定过：补了、或已被覆盖事件盖着（那是用户/LLM 自己写的片头，
/// 补种时机同样用掉）。段表为空 / 最早段起点 ≥ 0.5s / 段跨度 < 0.2s 时**不**用掉 —— 那时片头还不存在
/// （分段还没生成、或整片从 0.5s 之后才开始），下次 process 照旧能补。
/// 返回是否补了。
pub fn ensure_intro_fade_in(config: &mut project::ProjectFile) -> bool {
    if config.intro_fade_seeded {
        return false; // 补种时机已经用掉：这条事件现在归用户
    }
    // 段之间时间上会重叠，所以按时间挑最早的段，不按文件顺序
    let Some(idx) = config
        .segments
        .iter()
        .enumerate()
        .filter(|(_, s)| s.start < 0.5)
        .min_by(|(_, a), (_, b)| a.start.partial_cmp(&b.start).unwrap_or(std::cmp::Ordering::Equal))
        .map(|(i, _)| i)
    else {
        return false;
    };
    let (start, end) = (config.segments[idx].start, config.segments[idx].end);
    if end - start < 0.2 {
        return false;
    }
    let fade_end = (start + INTRO_FADE_SECS).min(end);
    let covered = config.segments.iter().any(|s| {
        s.events.iter().any(|e| {
            matches!(e, crate::project::Event::Overlay { .. })
                && e.get_time() < fade_end
                && e.end().unwrap_or_else(|| e.get_time()) > start
        })
    });
    // 片头窗口到这儿就判定过了：空着就补一条，盖着就认用户那份 —— 两种都算补种时机用完。
    config.intro_fade_seeded = true;
    if covered {
        return false;
    }
    config.segments[idx].events.push(crate::project::Event::Overlay {
        time: start,
        end: Some(fade_end),
        overlay_type: "fade_in".into(),
        color: Some("#000000".into()),
        opacity: Some(1.0),
        colors: None,
        shape: None,
        angle: None,
        sfx: None,
        sfx_volume: None,
        anchor: None,
    });
    true
}

/// 把 `anchor`/`anchor_to` 解析成 `origin`/`to_origin`。
///
/// 与贴纸的 keyword→name 同一套约定：锚点是**意图**（LLM 或人写的），每次 process 重算，
/// 所以改了选框/挪了贴纸不用手改坐标；解析结果才进渲染侧，四条渲染路径都不需要认识锚点。
/// 认不出的锚点只告警、保留原 `origin`，不让整条管线失败。
fn resolve_camera_anchors(config: &mut project::ProjectFile, data_dir: &Path) {
    let ctx = AnchorCtx::collect(config, data_dir);
    let mut unknown: Vec<String> = Vec::new();
    let mut resolved = 0usize;
    let mut migrated = 0usize;

    for seg in &mut config.segments {
        for ev in &mut seg.events {
            let crate::project::Event::Camera {
                time,
                anchor,
                anchor_to,
                origin,
                to_origin,
                ..
            } = ev
            else {
                continue;
            };
            // 老工程迁移：`"50% 50%"` 从来不是用户能设的值——它是 LLM prompt 示例与两个
            // 默认生成器硬编码的常量，一直被当作「没指定」。补成聚焦主播，让这次新增的
            // 「聚焦」对**已有**工程也生效（否则老切片重跑还是对着画面正中推）。
            if anchor.is_none() && to_origin.is_none() && origin.trim() == PRISTINE_ORIGIN {
                *anchor = Some("liver".into());
                migrated += 1;
            }
            if let Some(name) = anchor.as_deref() {
                match ctx.resolve(name, *time) {
                    Some(o) => {
                        *origin = o;
                        resolved += 1;
                    }
                    None => unknown.push(name.to_string()),
                }
            }
            *to_origin = match anchor_to.as_deref() {
                Some(name) => match ctx.resolve(name, *time) {
                    Some(o) => {
                        resolved += 1;
                        Some(o)
                    }
                    None => {
                        unknown.push(name.to_string());
                        None
                    }
                },
                None => None,
            };
        }
    }

    if migrated > 0 {
        tracing::info!("  🎯 {migrated} 条默认运镜已改为聚焦主播（原 origin 50% 50% 是历史常量，非用户设置）");
    }
    if resolved > 0 {
        tracing::info!("  🎯 运镜锚点已解析 {resolved} 个");
    }
    if !unknown.is_empty() {
        unknown.sort();
        unknown.dedup();
        tracing::info!("  ⚠ 运镜锚点无法识别（保持原 origin）: {}", unknown.join(", "));
    }
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

/// 将匹配 predicate 的事件抽稀到 budget 个 (均匀分布保留), 返回删除数量。
///
/// **手工放的事件（`manual`）不参与抽稀**：额度是拿来约束 LLM 生成量的 —— 用户在编辑器里
/// 亲手加的贴纸/运镜，数量本来就由他自己控制。不豁免的话，中间位置的会被均匀采样漏掉，
/// 表现就是"我新建一个贴纸，过两秒它自己没了"（用户报过；编辑器现在新建/复制时也会打这个标记）。
pub fn cap_events(
    segments: &mut [crate::project::Segment],
    is_target: impl Fn(&crate::project::Event) -> bool,
    budget: usize,
) -> usize {
    let is_target = |ev: &crate::project::Event| is_target(ev) && !ev.is_time_manual();
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
    use crate::project::{EdlClip, Event, MuteRange, ProjectFile, Segment};

    fn seg_with(events: Vec<Event>) -> Segment {
        Segment { start: 0.0, end: 10.0, fx: "low".into(), note: String::new(), events, extra: Default::default() }
    }

    /// 第 6 批：**EDL 引用的视频素材必须走"复制进项目"那条路**，而且**段表为空时也要走**。
    ///
    /// 为什么单独钉这一条：贴纸/SFX 都挂在 `segments` 上，所以"没有段 ⇒ 没有素材要下"
    /// 曾经成立；但**视频素材挂在 `edl.clips` 上**。`ensure_assets` 里那道
    /// `!config.segments.is_empty()` 的门会把"只有 EDL、没有段"的工程整个跳过 ⇒
    /// 出片在**没有素材拷贝**的情况下开跑，症状是"那一段缺画面"（不报错）。
    ///
    /// ⚠️ 判据只能落在**错误里提到了这个素材**，不能落在"文件复制成功"上：
    /// `download_videos` 从 `path::sticker_dir()` 取库根（**不是** `data_dir()`，环境变量改不动它），
    /// 单测没法把一个临时库注进去。真库当然没有 `mate` 这个素材 ⇒ 它必然报一条
    /// 「库里没有」——**而这条报错本身就是"接线通了"的证据**（没接线的话一条错都不会有）。
    /// "复制成功 + 幂等 + 内容对"由 `assets::downloader` 里的
    /// `download_videos_copies_into_project_and_reports_missing` 用可注入的库根验。
    /// 第 6 批：`ensure_assets` 的**门**不许把"只有 EDL、没有段"的工程挡在外面。
    ///
    /// `ensure_assets` 原来写的是 `!config.segments.is_empty()` —— 贴纸/SFX 挂在 `segments` 上，
    /// 那个条件对它们是对的；但**视频素材挂在 `edl.clips` 上**，所以一个段表为空（或全被去静音
    /// 切掉）的工程会被整个跳过，出片时**素材从没被复制过** ⇒ 那一段缺画面，且不报错。
    ///
    /// ⚠️ 判据刻意**不接 `tracing` 订阅者**：那要 `set_default`（**线程局部**），
    /// 而 cargo 默认多线程跑用例 ⇒ 别的用例的事件会让它翻来覆去（实测：单跑绿、全量红）。
    /// 换成**同一个谓词**的纯函数判据（下面那条 `?`），与门里的条件**同源**、且是确定性的。
    #[test]
    fn ensure_assets_gate_open_for_edl_only_project() {
        let gate = crate::process::download::ensure_assets_should_run;

        // 只有 EDL 素材、一条段都没有 —— 第 6 批要保证的正是这一档
        assert!(
            gate(&[], &[asset_clip("mate")]),
            "段表为空但 EDL 引用素材 ⇒ 必须仍然去复制素材（否则出片缺这一段画面）",
        );
        // 老路子照旧：有段就跑
        assert!(gate(&[seg_with(vec![])], &[]), "有段 ⇒ 照旧要跑");
        // 段 + EDL 素材同时有
        assert!(gate(&[seg_with(vec![])], &[asset_clip("mate")]), "两者都有 ⇒ 跑");
        // 真的什么都没有 ⇒ 不必白跑一趟（这条防的是把门写成恒 `true`）
        assert!(!gate(&[], &[]), "既没有段、也没有 EDL 素材 ⇒ 不该跑");
        // 有 EDL 段但都是**本条片段**（非素材）⇒ 没有素材要复制，不必跑
        let mut plain = asset_clip("mate");
        plain.source_kind = None;
        plain.asset_id = None;
        assert!(!gate(&[], &[plain]), "EDL 段不指向素材 ⇒ 没有素材要复制");
    }

    /// 一个"指向素材库"的 EDL 段。
    fn asset_clip(id: &str) -> yekitrim_render_core::types::EdlClip {
        yekitrim_render_core::types::EdlClip {
            source_start: 0.0,
            source_end: 2.0,
            speed: 1.0,
            kind: None,
            label: None,
            source_kind: Some(yekitrim_render_core::types::EDL_SOURCE_ASSET.into()),
            asset_id: Some(id.into()),
            transition_in: None,
        }
    }

    #[tokio::test]
    async fn ensure_assets_attempts_edl_video_copy_even_without_segments() {
        let dir = tempfile::tempdir().expect("临时目录");
        let path = dir.path();
        std::fs::create_dir_all(path.join("data")).expect("建 data/");

        let mut cfg = ProjectFile::example();
        cfg.segments.clear(); // 前提：段表为空，只有 EDL 引用素材
        cfg.edl.clips = vec![yekitrim_render_core::types::EdlClip {
            source_start: 0.0,
            source_end: 2.0,
            speed: 1.0,
            kind: None,
            label: None,
            source_kind: Some(yekitrim_render_core::types::EDL_SOURCE_ASSET.into()),
            asset_id: Some("lead-probe-nonexistent-material".into()),
            transition_in: None,
        }];
        assert!(cfg.segments.is_empty(), "前提：这个工程一条段都没有");
        std::fs::write(path.join("polish.toml"), cfg.to_toml_commented().expect("序列化")).expect("落盘");

        let downloader = crate::assets::downloader::AssetDownloader::new(path);
        let blacklist = crate::process::sfx::load_sfx_blacklist(&cfg);
        let errs = crate::process::download::download_keyword_assets(&downloader, &cfg, &blacklist).await;

        // 接线通了 ⇒ 它去查了库、没查到、报出来（1 条）。没接线 ⇒ 0 条（静默什么都不做）。
        assert_eq!(
            errs, 1,
            "段表为空但有 EDL 素材时，也必须去把素材复制进项目；             报错条数 = {errs}（0 表示这条路压根没被走到 —— 出片会缺这一段画面）",
        );
    }

    /// **屏蔽只做标记**：`cmd_process` 不许删掉屏蔽区间里的事件，也不许把段切碎。
    ///
    /// 历史行为（已删）：`split_segments_by_mute_ranges` 会把屏蔽区内的事件从 segments 里删掉
    /// 并落盘，而 `cmd_process` 会被预览/出片**隐式触发** —— 于是"随手屏蔽一段"等于永久丢事件，
    /// 事后撤销屏蔽也回不来（用户报的就是这个）。出片由 `Ffmpeg::cut_ranges` 剪、预览跳过，
    /// 事件留在文件里不动。
    ///
    /// 用 `manual: true` 的贴纸是为了把判据与"预算抽稀"分开：这一条只问"屏蔽有没有删事件"。
    #[tokio::test]
    async fn mute_range_keeps_events() {
        let dir = tempfile::tempdir().expect("临时目录");
        let path = dir.path();
        std::fs::create_dir_all(path.join("data")).expect("建 data/");
        // cmd_process 要求 data/subtitles.json 存在；屏蔽区内外各一条
        let subs = serde_json::json!([
            {"index": 1, "start": 1.0, "end": 3.0, "text": "屏蔽区外", "html": ""},
            {"index": 2, "start": 4.5, "end": 5.5, "text": "屏蔽区里", "html": ""},
        ]);
        std::fs::write(path.join("data").join("subtitles.json"), subs.to_string()).expect("写字幕");

        let mut cfg = ProjectFile::example();
        cfg.sticker_source = Vec::new(); // 空来源表：Step 2 不解析关键字、不联网
        cfg.mute_ranges = vec![MuteRange { start: 4.0, end: 6.0 }];
        cfg.segments = vec![Segment {
            start: 0.0,
            end: 10.0,
            fx: "low".into(),
            note: String::new(),
            events: vec![Event::Sticker {
                time: 5.0,
                end: Some(6.0),
                keyword: "ysyk".into(),
                sticker_tag: None,
                name: Some("生气".into()),
                source: Some("ysyk".into()),
                x: "auto".into(),
                y: "auto".into(),
                sfx: None,
                sfx_volume: None,
                manual: true,
            }],
            extra: Default::default(),
        }];
        std::fs::write(path.join("polish.toml"), cfg.to_toml_commented().expect("序列化"))
            .expect("写 polish.toml");

        cmd_process(path).await.expect("后处理");

        let after = crate::project::load_project(path).expect("重新读 polish.toml");
        let kept: Vec<f64> = after
            .segments
            .iter()
            .flat_map(|s| s.events.iter())
            .filter(|e| matches!(e, Event::Sticker { name: Some(n), .. } if n == "生气"))
            .map(|e| e.get_time())
            .collect();
        assert_eq!(kept, vec![5.0], "屏蔽区间里的事件被删掉了 —— 屏蔽只做标记，不许删事件");

        let spans: Vec<(f64, f64)> = after.segments.iter().map(|s| (s.start, s.end)).collect();
        assert_eq!(spans, vec![(0.0, 10.0)], "屏蔽区间不许把分段切碎");
    }

    /// **§1 决策理由真的落盘了**（端到端，走真 `cmd_process` + 真 `polish.toml`）。
    ///
    /// 为什么要端到端：`update_polish_toml` 是**只覆盖闭包里点名字段**的写回 ——
    /// 算出来的理由漏写一行就"一个字节都不落盘"，而**单元测试里直接调守卫永远看不出来**
    /// （守卫自己是对的）。这正是 §4 那次「PUT 说已保存、磁盘一字不动」的形状。
    ///
    /// 构造一个"边界切在句子中间"的工程，断言：
    /// ① `decisions` 非空且含边界类决策；② 磁盘上真能读回来（round-trip）；
    /// ③ 重跑 process 幂等（理由不叠加）。
    #[tokio::test]
    async fn decisions_are_recorded_and_round_trip_to_disk() {
        let dir = tempfile::tempdir().expect("临时目录");
        let path = dir.path();
        std::fs::create_dir_all(path.join("data")).expect("建 data/");
        // 三条字幕全无句末标点（真实 ASR 的常见形态），边界 5.0 落在第二条**内部**
        let subs = serde_json::json!([
            {"index": 1, "start": 0.0, "end": 2.0, "text": "第一句", "html": ""},
            {"index": 2, "start": 4.0, "end": 6.0, "text": "还在说", "html": ""},
            {"index": 3, "start": 10.0, "end": 12.0, "text": "第三句", "html": ""},
        ]);
        std::fs::write(path.join("data").join("subtitles.json"), subs.to_string()).expect("写字幕");

        let mut cfg = ProjectFile::example();
        cfg.sticker_source = Vec::new();
        cfg.segments = vec![
            Segment { start: 0.0, end: 5.0, fx: "med".into(), note: String::new(), events: vec![], extra: Default::default() },
            Segment { start: 5.0, end: 15.0, fx: "med".into(), note: String::new(), events: vec![], extra: Default::default() },
        ];
        std::fs::write(path.join("polish.toml"), cfg.to_toml_commented().expect("序列化"))
            .expect("写 polish.toml");

        cmd_process(path).await.expect("后处理");

        // ② 从**磁盘**读回来（不是读内存）—— 这才叫"落盘了"
        let after = crate::project::load_project(path).expect("重新读 polish.toml");
        assert!(
            !after.decisions.is_empty(),
            "边界被挪过就必须留下决策理由（落盘失败？update_polish_toml 漏写字段？）"
        );
        assert!(
            after.decisions.iter().any(|d| d.kind == decision_kind::BOUNDARY_SENTENCE_END),
            "要有边界类决策，实得：{:?}",
            after.decisions.iter().map(|d| &d.kind).collect::<Vec<_>>()
        );
        // 理由必须是**人话**（界面直接显示），不是空串或只有 kind
        let dm = after
            .decisions
            .iter()
            .find(|d| d.kind == decision_kind::BOUNDARY_SENTENCE_END)
            .expect("上面刚断言存在");
        assert!(!dm.text.trim().is_empty(), "理由文案不许为空");
        assert!(dm.at.is_some(), "边界类决策要带位置（界面据此定位）");
        // 边界确实挪到了第二条字幕的结束点 6.0
        assert_eq!((after.segments[0].end, after.segments[1].start), (6.0, 6.0));

        // ③ 幂等：重跑一次，**理由不叠加也不丢失**。
        //
        // ⚠️ 这一条踩过一次：守卫是幂等的（第二次看到边界已干净就返回 0 条挪动），
        // 若理由也跟着"本趟没挪 ⇒ 清空"，用户每跑一次 process 就**丢掉一次解释**，
        // 而那个被挪过的边界还在原地（实测 2 条 → 0 条）。所以边界类理由要**顺延**。
        let n_first = after.decisions.len();
        assert!(n_first > 0, "前提：第一趟要有理由");
        cmd_process(path).await.expect("第二次后处理");
        let again = crate::project::load_project(path).expect("再读");
        assert_eq!(
            again.decisions.len(),
            n_first,
            "重复 process 既不许让理由越攒越多、也不许把它们丢掉：{n_first} → {}",
            again.decisions.len()
        );
        // 边界位置也没漂（幂等的另一半）
        assert_eq!(
            (again.segments[0].end, again.segments[1].start),
            (6.0, 6.0),
            "第二次 process 不许再把边界挪一次"
        );
    }

    /// **顺延的边界**：本趟挪了**新的**边界时，同位置的旧理由被本趟覆盖（不重复），
    /// 位置不同的旧理由保留（那个边界仍是它挪过的，解释不能丢）。
    #[tokio::test]
    async fn decisions_do_not_duplicate_on_repeat_runs() {
        let dir = tempfile::tempdir().expect("临时目录");
        let path = dir.path();
        std::fs::create_dir_all(path.join("data")).expect("建 data/");
        let subs = serde_json::json!([
            {"index": 1, "start": 0.0, "end": 2.0, "text": "第一句", "html": ""},
            {"index": 2, "start": 4.0, "end": 6.0, "text": "还在说", "html": ""},
            {"index": 3, "start": 10.0, "end": 12.0, "text": "第三句", "html": ""},
        ]);
        std::fs::write(path.join("data").join("subtitles.json"), subs.to_string()).expect("写字幕");
        let mut cfg = ProjectFile::example();
        cfg.sticker_source = Vec::new();
        cfg.segments = vec![
            Segment { start: 0.0, end: 5.0, fx: "med".into(), note: String::new(), events: vec![], extra: Default::default() },
            Segment { start: 5.0, end: 15.0, fx: "med".into(), note: String::new(), events: vec![], extra: Default::default() },
        ];
        std::fs::write(path.join("polish.toml"), cfg.to_toml_commented().expect("序列化")).expect("写");

        // 连跑三趟：条数必须**恒等**（既不叠加也不丢）
        cmd_process(path).await.expect("第 1 趟");
        let n1 = crate::project::load_project(path).expect("读").decisions.len();
        cmd_process(path).await.expect("第 2 趟");
        let n2 = crate::project::load_project(path).expect("读").decisions.len();
        cmd_process(path).await.expect("第 3 趟");
        let n3 = crate::project::load_project(path).expect("读").decisions.len();
        assert_eq!((n1, n2, n3), (n1, n1, n1), "三趟跑下来条数必须恒定，实得 {n1}/{n2}/{n3}");

        // 同一位置不许出现两条边界理由
        let ats: Vec<Option<f64>> = crate::project::load_project(path)
            .expect("读")
            .decisions
            .iter()
            .filter(|d| d.kind == decision_kind::BOUNDARY_SENTENCE_END)
            .map(|d| d.at)
            .collect();
        let mut sorted = ats.clone();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
        sorted.dedup();
        assert_eq!(sorted.len(), ats.len(), "同一位置出现了重复理由：{ats:?}");
    }

    /// **老工程零回归**：没有决策时 `[decisions]` 不许出现在 `polish.toml` 里。
    ///
    /// `skip_serializing_if = "Vec::is_empty"` 是 R4 的判据 —— 空 vec 若照写，
    /// 每个老工程重新保存一次就会多出一段空表（逐字节回归判据立刻红）。
    #[test]
    fn empty_decisions_are_not_serialized() {
        let cfg = ProjectFile::example();
        let toml = cfg.to_toml_commented().expect("序列化");
        assert!(
            !toml.contains("[[decisions]]"),
            "空 decisions 不许写出表头（R4：老工程逐字节零回归）"
        );
    }



    /// **第 5 批 · 老工程零回归（R4 的端到端形态）**：没意图 ⇒ `edl.clips` 保持为空。
    ///
    /// 为什么这一条必须是**真跑 `cmd_process`**：判据不是"函数返回空"，而是
    /// **`polish.toml` 里不许出现 `[edl]` 段**。有 EDL 时 `mute_ranges` 整个被忽略
    /// （`ProjectFile::resolve_edl`、规格 §2.3），所以"写下去一个空 `[edl]`"这种
    /// 看起来无害的事**会真的改变成片时间线**（第 1 批实测：toml 照样写出空段头，
    /// 所以判据挂在 `Edl::is_empty` 上而不是 `clips` 自己的 `skip_serializing_if`）。
    ///
    /// ⚠️ **R4 是两层挡的，这条测试只咬得住其中一层。** 实测记录（`--mutate` 式自证）：
    ///
    /// | 把哪一层摘掉 | 本测试 |
    /// |---|---|
    /// | `ProjectFile::edl` 的 `skip_serializing_if = "Edl::is_empty"` | **红** ✅ |
    /// | `cmd_process` 里 `edl_intent_done.then_some(...)` 那个门 | **绿** ❌ —— 空 `Edl` 被 serde 那层静默吃掉 |
    ///
    /// 所以这一条**不能**独自充当"写回门"的判据。写回门由
    /// [`process_writeback_gate_only_touches_edl_when_materialized`] 用**纯谓词**钉住
    /// （与 `docs/plan/edl-retiming-spec.md` §11.14 里 `ensure_assets_should_run` 同一个教训：
    /// 两层判据要各自有观察点，否则拆掉一层毫无声响）。
    ///
    /// 同一条测试顺带钉住**幂等**：连跑两次 `cmd_process`，`edl` 一次都不许冒出来。
    #[tokio::test]
    async fn process_without_edl_intent_never_writes_edl() {
        let dir = tempfile::tempdir().expect("临时目录");
        let path = dir.path();
        std::fs::create_dir_all(path.join("data")).expect("建 data/");
        std::fs::write(
            path.join("data").join("subtitles.json"),
            serde_json::json!([
                {"index": 1, "start": 1.0, "end": 3.0, "text": "普通叙述", "html": ""},
            ]).to_string(),
        ).expect("写字幕");

        let mut cfg = ProjectFile::example();
        cfg.sticker_source = Vec::new(); // 空来源表：Step 2 不解析关键字、不联网
        cfg.mute_ranges = vec![MuteRange { start: 4.0, end: 6.0 }];
        // 普通运镜（没有 cold_open / recall 意图）
        cfg.segments = vec![Segment {
            start: 0.0, end: 10.0, fx: "low".into(), note: String::new(),
            events: vec![Event::Camera {
                time: 0.0, end: Some(10.0), zoom: "中".into(), origin: "50% 50%".into(),
                anchor: Some("liver".into()), anchor_to: None, to_origin: None,
                cut: false, handheld: false, manual: false, speed: None,
            }],
            extra: Default::default(),
        }];
        std::fs::write(path.join("polish.toml"), cfg.to_toml_commented().expect("序列化"))
            .expect("写 polish.toml");

        cmd_process(path).await.expect("后处理 #1");
        let after1 = crate::project::load_project(path).expect("重读");
        assert!(after1.edl.clips.is_empty(), "没有意图 ⇒ edl.clips 必须保持为空");
        let text1 = std::fs::read_to_string(path.join("polish.toml")).expect("读回 toml");
        assert!(
            !text1.contains("[edl]"),
            "没有意图就**一个字节都不许写 EDL**（空的 [edl] 段头会让 mute_ranges 整个被忽略）\n{text1}"
        );

        // 幂等：再跑一次，还是一样
        cmd_process(path).await.expect("后处理 #2");
        let text2 = std::fs::read_to_string(path.join("polish.toml")).expect("读回 toml");
        assert!(!text2.contains("[edl]"), "跑第二遍同样不许写 EDL");
        assert!(crate::project::load_project(path).expect("重读").edl.clips.is_empty());
    }

    /// **写回门本身**的判据（上一条测试咬不住的那一层）。
    ///
    /// `cmd_process` 结尾原来写的是"整份写回"时代留下的形状，第 5 批给它加了第三个字段，
    /// 门写成 `edl_intent_done.then_some(config.edl.clips)`：**只有本趟真的落实了意图，
    /// 才把 `clips` 交给 `update_polish_toml`**，否则 `cfg.edl` 一个字节都不碰
    /// （用户在时间轴上手拖过的 `clips` 靠这一条活着）。
    ///
    /// ⚠️ 为什么单开一条**纯谓词**测试：把 `then_some` 改成无条件 `Some(...)` 之后，
    /// 上一条端到端测试**照样绿** —— 空 `Edl` 被 `skip_serializing_if = "Edl::is_empty"`
    /// 那层静默吃掉，症状完全相同。两层判据必须各有各的观察点。
    /// 谓词是 `fn(bool, Vec<EdlClip>) -> Option<Vec<EdlClip>>`，与 `cmd_process` 里
    /// 那一行**同源**（`writeback_edl` 就是它的具名实现）。
    #[test]
    fn process_writeback_gate_only_touches_edl_when_materialized() {
        let user_clips = vec![EdlClip {
            source_start: 1.0, source_end: 2.0, speed: 1.0,
            kind: Some("main".into()), label: None,
            source_kind: None, asset_id: None, transition_in: None,
        }];

        // 没落实 ⇒ 门必须给出 `None`（= 调用方不许碰 `cfg.edl`）
        assert_eq!(
            writeback_edl(false, user_clips.clone()), None,
            "本趟没落实意图 ⇒ 不许把 clips 交给写回（手拖过的 EDL 会被空 vec 覆盖成 0 段）"
        );
        // 落实了 ⇒ 原样交出去
        assert_eq!(
            writeback_edl(true, user_clips.clone()), Some(user_clips.clone()),
            "落实了 ⇒ 必须交出去，否则冷开场生成了却不落盘"
        );
        // 落实了但结果确实是空（理论上到不了；钉住它而不是断言 unreachable）
        assert_eq!(writeback_edl(true, Vec::new()), Some(Vec::new()));
    }

    fn sticker() -> Event {
        Event::Sticker {
            time: 1.0, end: Some(3.0), keyword: "laugh".into(),
            sticker_tag: None, name: None, source: None, x: "auto".into(), y: "auto".into(),
            sfx: None, sfx_volume: None, manual: false,
        }
    }

    fn sfx_ev() -> Event {
        Event::Sfx { time: 1.0, name: "pop".into(), volume: Some(0.5), end: Some(2.0) }
    }

    fn camera() -> Event {
        Event::Camera { time: 0.0, end: Some(3.0), zoom: "中".into(), origin: "50% 50%".into(), anchor: None, anchor_to: None, to_origin: None, cut: false, handheld: false, manual: false, speed: None }
    }

    /// 覆盖事件（默认 1s 窗口）。
    fn overlay_at(time: f64) -> Event {
        Event::Overlay {
            time, end: Some(time + 1.0), overlay_type: "color".into(), color: Some("#ff3232".into()),
            opacity: Some(0.3), colors: None, shape: None, angle: None, sfx: None, sfx_volume: None, anchor: None,
        }
    }

    fn seg_span(start: f64, end: f64, events: Vec<Event>) -> Segment {
        Segment { start, end, ..seg_with(events) }
    }

    /// 片头淡入用的最小工程：换掉段表，其余取 `example()`（`[source]` 是必填项）。
    fn cfg_with(segments: Vec<Segment>) -> project::ProjectFile {
        project::ProjectFile { segments, ..project::ProjectFile::example() }
    }

    fn overlays(segments: &[Segment]) -> Vec<&Event> {
        segments.iter().flat_map(|s| s.events.iter())
            .filter(|e| matches!(e, Event::Overlay { .. }))
            .collect()
    }

    #[test]
    fn intro_fade_in_is_added_once_to_the_head_segment() {
        let mut cfg = cfg_with(vec![seg_span(0.0, 10.0, vec![camera()]), seg_span(10.0, 20.0, vec![sfx_ev()])]);
        assert!(ensure_intro_fade_in(&mut cfg));
        assert!(cfg.intro_fade_seeded, "补种时机应置位（随 process 写回 polish.toml）");
        let Event::Overlay { time, end, overlay_type, color, opacity, .. } =
            cfg.segments[0].events.last().expect("补在首段的事件末尾")
        else {
            panic!("补的应该是覆盖事件");
        };
        assert_eq!(overlay_type, "fade_in");
        assert_eq!(color.as_deref(), Some("#000000"));
        assert_eq!(*opacity, Some(1.0));
        assert_eq!((*time, *end), (0.0, Some(0.5)));
        assert_eq!(cfg.segments[1].events.len(), 1, "别的段不碰");

        // 标记置位后不再补（同一次 process 里再调一次也不重复）
        assert!(!ensure_intro_fade_in(&mut cfg));
        assert_eq!(overlays(&cfg.segments).len(), 1);
    }

    /// 用户报的那条：**删掉片头淡入之后再跑 process 不许补回来**；把标记删掉才拿回默认片头。
    #[test]
    fn intro_fade_in_is_not_restored_after_the_user_deletes_it() {
        let mut cfg = cfg_with(vec![seg_span(0.0, 10.0, vec![])]);
        assert!(ensure_intro_fade_in(&mut cfg));
        // 用户在时间轴上把它删了（或手改 polish.toml）
        cfg.segments[0].events.clear();
        assert!(!ensure_intro_fade_in(&mut cfg), "补过之后不许再补");
        assert!(cfg.segments[0].events.is_empty());
        assert!(cfg.intro_fade_seeded, "标记保持置位");

        // 反向逃生门：删掉 polish.toml 里那一行 → 下次 process 又能拿回默认片头
        cfg.intro_fade_seeded = false;
        assert!(ensure_intro_fade_in(&mut cfg));
    }

    #[test]
    fn intro_fade_in_skips_when_the_head_is_already_covered() {
        // 本段已有覆盖事件
        let mut cfg = cfg_with(vec![seg_span(0.0, 10.0, vec![overlay_at(0.0)])]);
        assert!(!ensure_intro_fade_in(&mut cfg));
        assert_eq!(cfg.segments[0].events.len(), 1);
        assert!(cfg.intro_fade_seeded, "片头已有表现 = 判定过，补种时机同样用掉");

        // 别的段跨过来盖住片头（段之间时间会重叠，判据是整个窗口而不是"本段有没有"）
        let mut cfg = cfg_with(vec![seg_span(0.0, 10.0, vec![]), seg_span(0.1, 0.4, vec![overlay_at(0.1)])]);
        assert!(!ensure_intro_fade_in(&mut cfg));
        assert!(cfg.intro_fade_seeded);

        // 窗口不相交（0.6s 起，本条的淡入是 [0.0, 0.5)）→ 片头仍是空的，照补
        let mut cfg = cfg_with(vec![seg_span(0.0, 10.0, vec![overlay_at(0.6)])]);
        assert!(ensure_intro_fade_in(&mut cfg));
        assert_eq!(overlays(&cfg.segments).len(), 2);
    }

    /// 片头还不存在时**不烧标记** —— 那不是"补过"，下次 process 照旧能补。
    #[test]
    fn intro_fade_in_keeps_the_marker_unused_when_there_is_no_head() {
        // 起点已经在 0.5s 之外：这不是"片子的开头"
        let mut cfg = cfg_with(vec![seg_span(3.0, 9.0, vec![])]);
        assert!(!ensure_intro_fade_in(&mut cfg));
        // 段太短（< 0.2s）：塞不下淡入
        let mut cfg_short = cfg_with(vec![seg_span(0.0, 0.15, vec![])]);
        assert!(!ensure_intro_fade_in(&mut cfg_short));
        // 空段表
        let mut cfg_empty = cfg_with(vec![]);
        assert!(!ensure_intro_fade_in(&mut cfg_empty));
        assert!(
            !cfg.intro_fade_seeded && !cfg_short.intro_fade_seeded && !cfg_empty.intro_fade_seeded,
            "没判定过片头就不该用掉补种时机"
        );

        // 分段补上来了 → 同一份 config 再跑就能补（"分段还没生成时先跑过一次 process"）
        cfg.segments = vec![seg_span(0.0, 9.0, vec![])];
        assert!(ensure_intro_fade_in(&mut cfg));
        assert_eq!(overlays(&cfg.segments).len(), 1);
    }

    #[test]
    fn intro_fade_in_picks_the_earliest_segment_not_the_first_in_file() {
        let mut cfg = cfg_with(vec![seg_span(5.0, 9.0, vec![]), seg_span(0.0, 4.0, vec![])]);
        assert!(ensure_intro_fade_in(&mut cfg));
        assert_eq!(cfg.segments[0].events.len(), 0);
        assert_eq!(cfg.segments[1].events.len(), 1);
    }

    #[test]
    fn intro_fade_in_is_clipped_to_a_short_head_segment() {
        // 首段只有 0.3s：淡入止步于段尾，窗口不许拖到段外
        let mut cfg = cfg_with(vec![seg_span(0.0, 0.3, vec![])]);
        assert!(ensure_intro_fade_in(&mut cfg));
        let Event::Overlay { time, end, .. } = &cfg.segments[0].events[0] else {
            panic!("补的应该是覆盖事件");
        };
        assert_eq!((*time, *end), (0.0, Some(0.3)));
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

    /// 手工放的事件（`manual`）**永远不被抽稀删掉**，只有 LLM 生成的那些参与均匀采样。    /// 反例（修之前）：新建一个贴纸 → 自动后处理跑一遍 → 中间那个被采样漏掉 → "我加的东西自己没了"。
    #[test]
    fn cap_events_never_removes_manual_events() {
        let manual_sticker = |t: f64| {
            let mut ev = sticker();
            if let Event::Sticker { time, manual, .. } = &mut ev {
                *time = t;
                *manual = true;
            }
            ev
        };
        // 9 个手工贴纸 + 预算 2：一个都不能删（预算只管非手工的）
        let events: Vec<Event> = (0..9).map(|i| manual_sticker(i as f64)).collect();
        let mut segments = vec![seg_with(events)];
        let removed = cap_events(&mut segments, |e| matches!(e, Event::Sticker { .. }), 2);
        assert_eq!(removed, 0);
        assert_eq!(segments[0].events.len(), 9, "手工事件必须原样保留");

        // 手工 + 自动混在一起：只抽非手工的，手工的几个一个不少
        let mut events: Vec<Event> = vec![manual_sticker(0.0), manual_sticker(1.0)];
        events.extend((0..6).map(|_| sticker())); // 6 个 LLM 贴纸
        events.push(manual_sticker(9.0));
        let mut segments = vec![seg_with(events)];
        let removed = cap_events(&mut segments, |e| matches!(e, Event::Sticker { .. }), 2);
        assert_eq!(removed, 4, "6 个自动贴纸抽到 2 个");
        assert_eq!(segments[0].events.len(), 5, "3 个手工 + 2 个自动");
        let manual_left = segments[0]
            .events
            .iter()
            .filter(|e| e.is_time_manual())
            .count();
        assert_eq!(manual_left, 3, "手工的一个都不能少");
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
        let existing = vec![crate::project::Event::Sfx { time: 0.5, name: "x".into(), volume: None, end: None }];
        let moments = find_uncovered_sfx_moments(&subs, &existing, 4);
        assert_eq!(moments.len(), 3, "0.2s 与 0.5s 声音事件冲突被跳过");
        assert_eq!(moments[0].0, 5.2);
        // camera 等无声音事件不挡补位
        let cam = vec![crate::project::Event::Camera { time: 1.0, end: Some(4.0), zoom: "中".into(), origin: "50% 50%".into(), anchor: None, anchor_to: None, to_origin: None, cut: false, handheld: false, manual: false, speed: None }];
        let moments = find_uncovered_sfx_moments(&subs, &cam, 4);
        assert_eq!(moments[0].0, 0.2, "camera 不占用音效时刻");
        // 预算 0 → 空
        assert!(find_uncovered_sfx_moments(&subs, &[], 0).is_empty());
    }

    #[test]
    fn sfx_llm_reference_detection() {
        // LLM 写的 tag 引用 / catalog id 都算"已有引用"
        assert!(event_has_sfx_ref(&Event::Sfx { time: 1.0, name: "tag:爆発".into(), volume: None, end: None }));
        assert!(event_has_sfx_ref(&Event::Sfx { time: 1.0, name: "battery1".into(), volume: None, end: None }));
        assert!(!event_has_sfx_ref(&Event::Sfx { time: 1.0, name: String::new(), volume: None, end: None }));
        assert!(event_has_sfx_ref(&Event::Shake { time: 1.0, intensity: 0.5, end: None, anchor: None, sfx: Some("tag:爆発".into()), sfx_volume: None }));
        assert!(!event_has_sfx_ref(&Event::Shake { time: 1.0, intensity: 0.5, end: None, anchor: None, sfx: None, sfx_volume: None }));
        assert!(!event_has_sfx_ref(&Event::Camera {
            time: 1.0, end: Some(4.0), zoom: "中".into(), origin: "50% 50%".into(),
            anchor: None, anchor_to: None, to_origin: None, cut: false, handheld: false, manual: false,
            speed: None,
        }));
    }

    /// 运镜倍率迁移：三档的旧内建默认（0.7 / 1.15 / 1.3）换成新默认；手改过的值一律保留。
    #[test]
    fn camera_zoom_migrates_old_defaults_only() {
        // 老工程：三个值都是当年的内建默认（polish.toml 里被序列化写死过）→ 全部换新
        let mut cam = crate::project::CameraStyle {
            zoom_mid: LEGACY_ZOOM_MID, zoom_close: LEGACY_ZOOM_CLOSE, zoom_wide: LEGACY_ZOOM_WIDE,
            ..Default::default()
        };
        let changed = migrate_camera_zoom(&mut cam);
        assert_eq!(changed.len(), 3, "{changed:?}");
        assert_eq!(cam.zoom_mid, 1.3);
        assert_eq!(cam.zoom_close, 1.5);
        assert_eq!(cam.zoom_wide, 0.55);
        assert!(migrate_camera_zoom(&mut cam).is_empty(), "幂等：迁移过的不再动");

        // 手改过任意一档 → 该档纹丝不动
        let mut custom = crate::project::CameraStyle {
            zoom_mid: 1.2, zoom_close: 1.4, zoom_wide: 0.78, ..Default::default()
        };
        assert!(migrate_camera_zoom(&mut custom).is_empty());
        assert_eq!((custom.zoom_mid, custom.zoom_close, custom.zoom_wide), (1.2, 1.4, 0.78));
    }

    /// 运镜 prompt 要写明画中画「整片最多一条」与 cut（硬切）的用法。
    #[test]
    fn camera_prompt_states_pip_rules_and_cut() {
        let p = crate::llm::segment::build_events_system_prompt(&[], &[], "");
        assert!(p.contains("画中画"), "缺少画中画说明：{p}");
        assert!(p.contains("整片最多一条"), "缺少配额提示：{p}");
        assert!(p.contains("落在字幕的边界上"), "缺少对齐要求：{p}");
        assert!(p.contains("到点系统会硬切回"), "缺少画中画到点收工说明：{p}");
        assert!(p.contains("1.5~3s"), "缺少画中画时长建议：{p}");
        assert!(p.contains("对齐到**最近的一条字幕**"), "贴纸 prompt 缺少对齐说明：{p}");
        assert!(p.contains("\"cut\":true"), "缺少硬切用法：{p}");
    }

    /// 覆盖层 prompt 必须教 `colors` 数组。曾经只给 `"overlay_type":"gradient","color":"#ff0000"`
    /// 这个示例 —— LLM 老老实实照做，而四个求值器都只认 `colors`，于是真实工程里 8 个覆盖层
    /// （4 个项目，全部是 gradient + 单个 color）**一个都没画出来**。
    #[test]
    fn overlay_prompt_documents_colors_array() {
        let p = crate::llm::segment::build_events_system_prompt(&[], &[], "");
        assert!(p.contains("\"colors\":["), "覆盖层示例缺少 colors 数组：{p}");
        assert!(p.contains("\"overlay_type\":\"gradient\""), "覆盖层示例缺少渐变：{p}");
        assert!(p.contains("\"overlay_type\":\"color\""), "覆盖层示例缺少纯色：{p}");
        assert!(p.contains("该色 → 透明"), "缺少单色兜底语义说明：{p}");
    }

    #[test]
    fn llm_prompt_contains_sound_judgment_rules() {        let sources = vec!["ysyk".to_string()];
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

    /// ⑤ 4a：`layout.liver` 是**源帧**框，出片时它先被画面矩形 `sr` 投到画布，再按相机变换缩放。
    ///
    /// 判据走**生产路径**（`sticker_obstacles`），所以漏掉 `sr` 这一步就会红：
    /// 4:3 源（1440×1080）进 16:9 画布（1920×1080）→ `sr = (240,0,1440,1080)`，
    /// liver 框 10–40% ⇒ 真障碍 **`[384,816]`×`[108,432]`**；
    /// 按整帧读（改造前的模型值）是 **`[192,768]`×`[108,432]`** —— 差 192px，
    /// 贴着模型右沿的临界候选会压住主播 `816-769 = 47px`。
    #[test]
    fn liver_obstacle_is_projected_through_the_main_frame_rect() {
        let mut cfg = ProjectFile::example();
        cfg.layout.liver = free_region("10%", "10%", "40%", "40%");
        // 4:3 源：axis=x，sr = (240,0,1440,1080)
        let ob = sticker_obstacles(&cfg, Path::new("yekitrim-no-such-dir"), Some((1440, 1080)));
        let liver = obstacle_rects(&ob, placement::ObstacleKind::Liver);
        assert_eq!(liver, vec![placement::Rect::new(384.0, 108.0, 816.0, 432.0)]);
        // 旧口径（整帧）与真值的差——真值是 [384,816]
        let model = placement::layout_region_rects(
            &cfg.layout.liver,
            placement::Rect::new(0.0, 0.0, 1920.0, 1080.0),
        );
        assert_eq!(model, vec![placement::Rect::new(192.0, 108.0, 768.0, 432.0)]);
        // 人物框必须**跟着画面走**：拿不到源尺寸时退化成整块画布（= 改造前行为）
        let ob_none = sticker_obstacles(&cfg, Path::new("yekitrim-no-such-dir"), None);
        assert_eq!(
            obstacle_rects(&ob_none, placement::ObstacleKind::Liver),
            vec![placement::Rect::new(192.0, 108.0, 768.0, 432.0)],
        );
    }

    /// ⑤ 4a（`[stage] scale = 1.6` 那条）：`scale` 绕**盒心**放大 `sr` 之后，源帧框跟着放大 ——
    /// 4:3 源进 16:9 画布、`scale=1.6`、`pct=50` ⇒ `sr = (-192,-324,2304,1728)`，
    /// liver 框 10–40% 的真障碍是 `(38.4,-151.2)–(729.6,367.2)`（越出画布，由 `blocked_at` 钳）。
    ///
    /// 改造前的模型值 `(192,108)–(768,432)` 完全在画布内、且与真值只重叠一部分 ——
    /// 这就是「真障碍比模型大一截，而 `relaxed` 不触发、零告警」的来源。
    #[test]
    fn stage_scale_moves_the_source_frame_obstacles() {
        let mut cfg = ProjectFile::example();
        cfg.stage.scale = 1.6;
        cfg.layout.liver = free_region("10%", "10%", "40%", "40%");
        let ob = sticker_obstacles(&cfg, Path::new("yekitrim-no-such-dir"), Some((1440, 1080)));
        let liver = obstacle_rects(&ob, placement::ObstacleKind::Liver);
        assert_eq!(liver.len(), 1);
        let r = liver[0];
        for (got, want) in [(r.x0, 38.4), (r.y0, -151.2), (r.x1, 729.6), (r.y1, 367.2)] {
            assert!((got - want).abs() < 1e-9, "sr 放大后源帧框应同步放大: {r:?}");
        }
        // 钳进画布之后仍然是「上半部分一大块」——不再是模型值那个小盒子
        // ⚠️ f64 精确相等在这里是脆的：`240 + 0.1*1440` 这类组合会差最后一位
        // （实测 `38.400000000000006 != 38.4`），所以与上面那条一样走容差。
        let hit = r.clamp_to_frame(ob.frame);
        for (got, want) in [(hit.x0, 38.4), (hit.y0, 0.0), (hit.x1, 729.6), (hit.y1, 367.2)] {
            assert!((got - want).abs() < 1e-6, "钳进画布后的真障碍: {hit:?}");
        }
        // 真障碍比「改造前按整帧算的模型值」明显宽 —— 这正是 ⑤ 4a 的实测量：
        // 模型值 (192,108)–(768,432) = 576px 宽；真障碍钳进画布后 691.2px，x 方向差 192px。
        let model = placement::Rect::new(192.0, 108.0, 768.0, 432.0);
        assert!(
            hit.width() > model.width(),
            "真障碍（{hit:?}）必须比模型值（{model:?}）宽"
        );
        assert!((hit.width() - 691.2).abs() < 1e-6, "真障碍宽 = 691.2px: {hit:?}");
    }

    /// 生产路径的贴纸落点**不许**在零告警的情况下压住人物：4:3 源 + `scale=1.6`，
    /// 真障碍挡住左上大半；候选真被挡光时必须走 `Pick::relaxed`（调用方会打警告）。
    #[test]
    fn scaled_stage_never_places_a_sticker_on_the_star_without_a_warning() {
        let mut cfg = ProjectFile::example();
        cfg.stage.scale = 1.6;
        cfg.layout.liver = free_region("10%", "10%", "40%", "40%");
        let ob = sticker_obstacles(&cfg, Path::new("yekitrim-no-such-dir"), Some((1440, 1080)));
        let liver = obstacle_rects(&ob, placement::ObstacleKind::Liver)[0].clamp_to_frame(ob.frame);
        for seed in [0.0, 0.13, 0.5, 0.77, 0.99] {
            let blocked = ob.blocked_at(0.0, 3.0, &[]);
            let p = placement::pick_sticker_position(&blocked, &ob.allow, seed, ob.frame);
            let box_ = placement::sticker_box(p.x, p.y, ob.frame);
            assert!(
                p.relaxed || !box_.intersects(&liver),
                "seed {seed}: 压住人物却 relaxed=false（= 生产里零告警）: {p:?} vs {liver:?}",
            );
        }
    }

    /// `layout.<区域>` 的自由选框（= `region_rects` 的输入）。
    fn free_region(x0: &str, y0: &str, x1: &str, y1: &str) -> project::LayoutRegion {
        project::LayoutRegion {
            grid: Vec::new(),
            origin_grid: None,
            origin: None,
            top_left: Some(project::Point { x: x0.into(), y: y0.into() }),
            bottom_right: Some(project::Point { x: x1.into(), y: y1.into() }),
            extra: Default::default(),
        }
    }

    /// 某一类**源帧**障碍在构建期算出的矩形（未经相机变换）。
    fn obstacle_rects(
        ob: &StickerObstacles,
        kind: placement::ObstacleKind,
    ) -> Vec<placement::Rect> {
        ob.source.iter().filter(|(_, k)| *k == kind).map(|(r, _)| *r).collect()
    }

    /// 只读诊断：扫描真实工程，打印按新规则推导出的贴纸落点（不写任何文件）。
    ///
    /// `VT_PROBE_ROOT=<media 目录> cargo test -p yekitrim-polish --lib diagnose_sticker_placement -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn diagnose_sticker_placement() {
        let Ok(root) = std::env::var("VT_PROBE_ROOT") else { return };
        fn walk(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
            let Ok(rd) = std::fs::read_dir(dir) else { return };
            for e in rd.flatten() {
                let p = e.path();
                if p.is_dir() {
                    walk(&p, out);
                } else if p.file_name().map(|n| n == "polish.toml").unwrap_or(false) {
                    out.push(p);
                }
            }
        }
        let mut files = Vec::new();
        walk(std::path::Path::new(&root), &mut files);
        files.sort();
        let (mut n_proj, mut n_sticker, mut n_relaxed) = (0, 0, 0);
        for f in files {
            let dir = f.parent().unwrap();
            let Ok(cfg) = crate::project::load_project(dir) else { continue };
            // 与 cmd_process 同口径：读真实源尺寸，主画面障碍才是生产里的那一个
            let video_size = crate::render::get_video_size(
                &dir.join(&cfg.source.video).to_string_lossy(),
            );
            let ob = sticker_obstacles(&cfg, &dir.join("data"), video_size);
            let mut placed = Vec::new();
            let mut rows = Vec::new();
            for seg in &cfg.segments {
                for (idx, ev) in seg.events.iter().enumerate() {
                    let crate::project::Event::Sticker { time, end, .. } = ev else { continue };
                    let sticker_end = end.unwrap_or(*time);
                    let blocked = ob.blocked_at(*time, sticker_end, &placed);
                    let seed = (idx as f64 * 3.7 + seg.start * 1.3).fract();
                    let pick = placement::pick_sticker_position(&blocked, &ob.allow, seed, ob.frame);
                    placed.push((*time, sticker_end + 0.65, placement::sticker_box(pick.x, pick.y, ob.frame)));
                    n_sticker += 1;
                    if pick.relaxed {
                        n_relaxed += 1;
                    }
                    rows.push(format!(
                        "      t={time:5.1} x={:>4.0}% y={:>4.0}% clearance={:>5.0} relaxed={}",
                        pick.x / ob.frame.w * 100.0,
                        pick.y / ob.frame.h * 100.0,
                        pick.clearance,
                        pick.relaxed,
                    ));
                }
            }
            if rows.is_empty() {
                continue;
            }
            n_proj += 1;
            println!(
                "  {} (liver={} panel={} dm_rows={} allow={} shots={})",
                dir.display(),
                ob.liver_count,
                ob.panel_count,
                ob.danmaku_rows,
                ob.allow.len(),
                ob.shots.len(),
            );
            for r in rows {
                println!("{r}");
            }
        }
        println!("== 工程 {n_proj} 个 / 贴纸 {n_sticker} 个 / 兜底 {n_relaxed} 个 ==");
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

    // ===== 运镜锚点解析 =====

    fn anchor_ctx() -> AnchorCtx {
        AnchorCtx {
            liver: Some("83.33% 66.67%".into()),
            danmaku_area: Some("16.67% 16.67%".into()),
            // 「贴纸A」覆盖 10~14s、「贴纸B」覆盖 30~32s
            stickers: vec![(50.0, 50.0, 10.0, 14.0), (20.0, 80.0, 30.0, 32.0)],
            // 轨道 y=300 在 0~5s 在屏；y=900 在 20~25s 在屏
            danmaku: vec![(300.0, 0.0, 5.0), (900.0, 20.0, 25.0)],
            danmaku_font: 42.0,
            danmaku_stroke: 2.0,
            frame: placement::Frame::landscape(),
        }
    }

    #[test]
    fn anchor_liver_and_center_resolve() {
        let ctx = anchor_ctx();
        assert_eq!(ctx.resolve("liver", 0.0).as_deref(), Some("83.33% 66.67%"));
        assert_eq!(ctx.resolve("center", 0.0).as_deref(), Some("50.00% 50.00%"));
        // 未知名字 → None（调用方保留原 origin 并告警，不让管线失败）
        assert_eq!(ctx.resolve("game_area", 0.0), None);
        assert_eq!(ctx.resolve("", 0.0), None);
    }

    /// 弹幕锚点要按**时刻**取当刻在屏的轨道，而不是整片所有轨道的并集。
    #[test]
    fn anchor_danmaku_follows_time_and_falls_back() {
        let ctx = anchor_ctx();
        // 0~5s：只有 y=300 在屏 → 带子靠上
        let early = ctx.resolve("danmaku", 2.0).expect("早期应有弹幕");
        let y_early = project::parse_pct(early.split_whitespace().nth(1).unwrap());
        // 20~25s：y=900 在屏 → 带子明显更靠下
        let late = ctx.resolve("danmaku", 22.0).expect("后期应有弹幕");
        let y_late = project::parse_pct(late.split_whitespace().nth(1).unwrap());
        assert!(y_late > y_early, "后期弹幕更靠下: {y_early} vs {y_late}");

        // 那一刻没有弹幕 → 退回用户框的弹幕区域
        assert_eq!(ctx.resolve("danmaku", 12.0).as_deref(), Some("16.67% 16.67%"));
        // 连弹幕区域也没有 → 只留 None
        let mut bare = anchor_ctx();
        bare.danmaku.clear();
        bare.danmaku_area = None;
        assert_eq!(bare.resolve("danmaku", 12.0), None);
    }

    /// 贴纸锚点取时间上最近的贴纸盒子中心（覆盖当前时刻的优先）。
    #[test]
    fn anchor_sticker_picks_nearest_and_uses_box_center() {
        let ctx = anchor_ctx();
        // 12s 落在「贴纸A」窗口内（10~14s）→ 取 A 的盒子中心
        assert_eq!(ctx.resolve("sticker", 12.0).as_deref(), Some("50.00% 50.00%"));
        // 29s 离 B(30~32) 比离 A(10~14) 近 → 取 B
        assert_eq!(ctx.resolve("sticker", 29.0).as_deref(), Some("20.00% 80.00%"));
        // 完全没有贴纸 → None
        let mut bare = anchor_ctx();
        bare.stickers.clear();
        assert_eq!(bare.resolve("sticker", 12.0), None);
    }

    /// 贴纸 x/y 是盒子**左上角**百分比，锚点要的是中心，别把角落当成中心。
    #[test]
    fn sticker_center_offsets_by_half_box() {
        let f = placement::Frame::landscape();
        let (cx, cy) = sticker_center_pct("10%", "20%", f).unwrap();
        let half_w = f.sticker_size() / 2.0 / f.w * 100.0;
        let half_h = f.sticker_size() / 2.0 / f.h * 100.0;
        assert!((cx - (10.0 + half_w)).abs() < 1e-9, "{cx}");
        assert!((cy - (20.0 + half_h)).abs() < 1e-9, "{cy}");
        // 还没被落点解析过的 "auto" 不能当成坐标用
        assert_eq!(sticker_center_pct("auto", "auto", f), None);
        assert_eq!(sticker_center_pct("10%", "auto", f), None);
    }
}

/// 真实工程上的回归：跑一遍完整后处理，**手工放的事件（`manual`）一个都不能少**
/// （数量抽稀只约束 LLM 生成的那些），同时把 LLM 那批的抽稀结果打出来便于对照。
///
/// ⚠️ 这个会**写回** polish.toml（`segments` + `processed`）—— 只在临时/测试工程上跑。
///
/// ```bash
/// VT_POLISH_DIR=<片段目录(含 polish.toml 与 data/)> \
///   cargo test -p yekitrim-polish --lib diagnose_manual_events_survive_process -- --ignored --nocapture
/// ```
#[cfg(test)]
mod diagnose_manual_events_survive_process {
    fn sticker_times(config: &crate::project::ProjectFile, manual: bool) -> Vec<f64> {
        config
            .segments
            .iter()
            .flat_map(|s| s.events.iter())
            .filter(|e| matches!(e, crate::project::Event::Sticker { .. }) && e.is_time_manual() == manual)
            .map(|e| e.get_time())
            .collect()
    }

    #[tokio::test]
    #[ignore]
    async fn diagnose_manual_events_survive_process() {
        let Ok(dir) = std::env::var("VT_POLISH_DIR") else {
            eprintln!("用法: VT_POLISH_DIR=<片段目录> cargo test -p yekitrim-polish --lib diagnose_manual_events_survive_process -- --ignored --nocapture");
            return;
        };
        let path = std::path::PathBuf::from(&dir);
        let before = crate::project::load_project(&path).expect("读取 polish.toml");
        let manual = sticker_times(&before, true);
        let auto_before = sticker_times(&before, false);
        assert!(!manual.is_empty(), "这个工程里没有手工贴纸，测不出东西");

        super::cmd_process(&path).await.expect("后处理");

        let after = crate::project::load_project(&path).expect("重新读取 polish.toml");
        let manual_after = sticker_times(&after, true);
        let auto_after = sticker_times(&after, false);
        println!("\n项目: {dir}");
        println!("  手工贴纸 {manual:?} → {manual_after:?}");
        println!("  自动贴纸 {auto_before:?} → {auto_after:?}");
        assert_eq!(manual_after, manual, "手工贴纸必须原样保留（含时间）");
        assert!(auto_after.len() <= auto_before.len(), "自动贴纸只可能被抽稀");
    }
}

/// 只读诊断（不写任何文件）：在**真实**工程上干跑运镜锚点解析，
/// 看看每条 camera 事件会聚焦到哪个坐标、有没有认不出的锚点名。
///
/// ```bash
/// VT_POLISH_DIR=<项目目录(含 polish.toml 与 data/)> \
///   cargo test -p yekitrim-polish --lib diagnose_camera_anchors -- --ignored --nocapture
/// ```
#[cfg(test)]
mod diagnose_camera_anchors {
    #[test]
    #[ignore]
    fn diagnose_camera_anchors() {
        let Ok(dir) = std::env::var("VT_POLISH_DIR") else {
            eprintln!("用法: VT_POLISH_DIR=<项目目录> cargo test -p yekitrim-polish --lib diagnose_camera_anchors -- --ignored --nocapture");
            return;
        };
        let path = std::path::PathBuf::from(&dir);
        let mut config = crate::project::load_project(&path).expect("读取 polish.toml");
        println!("\n项目: {dir}");
        println!(
            "layout.liver 解析中心 = {:?}",
            crate::project::region_origin(&config.layout.liver)
        );

        let ctx = super::AnchorCtx::collect(&config, &path.join("data"));
        println!(
            "锚点数据源: 贴纸 {} 条 / 弹幕轨道 {} 条 / 弹幕区域 {:?}",
            ctx.stickers.len(),
            ctx.danmaku.len(),
            ctx.danmaku_area
        );

        super::resolve_camera_anchors(&mut config, &path.join("data"));

        let mut n = 0;
        for seg in &config.segments {
            for ev in &seg.events {
                let crate::project::Event::Camera {
                    time, zoom, anchor, anchor_to, origin, to_origin, ..
                } = ev
                else {
                    continue;
                };
                n += 1;
                let tail = match to_origin {
                    Some(t) => format!("  → {t}"),
                    None => String::new(),
                };
                println!(
                    "  [{:6.1}s] zoom={} anchor={:?} anchor_to={:?}  origin={}{}",
                    time, zoom, anchor, anchor_to, origin, tail
                );
            }
        }
        println!("camera 事件共 {n} 条");
    }
}

/// 只读诊断：在**真实工程**上看「句末边界守卫」（总方案 §2）到底挪了几处、挪了多少。
///
/// 为什么要有它：守卫的全部风险是"找不到合格点就放弃" ⇒ 形同虚设，而单元测试用的是合成字幕，
/// 看不出**真实字幕里句末标点的比例**。这个探针把真实数据上的命中率、挪动幅度、
/// 以及"判为干净 / 被挪 / 放弃"三类各多少条打出来。
///
/// ⚠️ **只读**：只 `load_project` + 读 `data/subtitles.json`，**不写回**（与
/// `diagnose_manual_events_survive_process` 那条"会写回"的探针不同）。
///
/// ```bash
/// VT_POLISH_DIR=<项目目录(含 polish.toml 与 data/)> \
///   cargo test -p yekitrim-polish --lib diagnose_boundaries -- --ignored --nocapture
/// ```
#[cfg(test)]
mod diagnose_boundaries {
    #[test]
    #[ignore]
    fn diagnose_boundaries() {
        let Ok(dir) = std::env::var("VT_POLISH_DIR") else {
            eprintln!("用法: VT_POLISH_DIR=<项目目录> cargo test -p yekitrim-polish --lib diagnose_boundaries -- --ignored --nocapture");
            return;
        };
        let path = std::path::PathBuf::from(&dir);
        let config = crate::project::load_project(&path).expect("读取 polish.toml");
        let subs: Vec<crate::parse::srt::Subtitle> = serde_json::from_str(
            &std::fs::read_to_string(path.join("data/subtitles.json")).expect("读 data/subtitles.json"),
        )
        .expect("解析 subtitles.json");

        println!("\n项目: {dir}");
        println!("字幕 {} 条 · 分段 {} 个", subs.len(), config.segments.len());

        // ① 字幕本身的句末比例：守卫的上限就是它（没有句末点可挪 ⇒ 只能落到空隙）。
        let ends = subs.iter().filter(|s| super::fx::is_sentence_end(&s.text)).count();
        println!(
            "\n[句末标点] {ends}/{} 条判为句末（{:.1}%）—— 守卫的可挪点数上限",
            subs.len(),
            if subs.is_empty() { 0.0 } else { ends as f64 * 100.0 / subs.len() as f64 }
        );

        // ② 逐个共享边界看原本干不干净、挪去哪。
        let segs = config.segments.clone();
        let mut order: Vec<usize> = (0..segs.len()).collect();
        order.sort_by(|a, b| segs[*a].start.partial_cmp(&segs[*b].start).unwrap_or(std::cmp::Ordering::Equal));
        let mut clean = 0usize;
        let mut moved: Vec<(f64, f64, f64)> = Vec::new();
        let mut gave_up = 0usize;
        let mut refused_by_min: Vec<f64> = Vec::new();
        for w in order.windows(2) {
            let (a, b) = (w[0], w[1]);
            let shared = segs[a].end;
            if (shared - segs[b].start).abs() > 1e-6 {
                continue;
            }
            let before_ok = super::fx::boundary_is_clean(shared, &subs);
            if before_ok {
                clean += 1;
                continue;
            }
            match super::fx::best_boundary_near(shared, &subs) {
                // ⚠️ **同一道最小段长闸**：守卫会对"挪完把某段吃到 < 0.5s"的候选弃权，
                // 探针不跟着判就会**报出比实跑更多的挪动数**（实测踩到：探针说 1 处、
                // 实跑 0 处 —— 两者对不上会让人以为守卫坏了）。
                Some(t)
                    if t > segs[a].start + super::fx::MIN_SEGMENT_SECS
                        && t < segs[b].end - super::fx::MIN_SEGMENT_SECS =>
                {
                    moved.push((shared, t, t - shared))
                }
                Some(_) => refused_by_min.push(shared),
                None => gave_up += 1,
            }
        }

        println!("\n[共享边界] 本来就干净 {clean} 处");
        println!("           被挪走 {} 处", moved.len());
        println!("           找不到合格点而放弃 {gave_up} 处");
        println!("           挪了会把段吃到 <{:.1}s 而弃权 {} 处", super::fx::MIN_SEGMENT_SECS, refused_by_min.len());
        for (from, to, d) in moved.iter().take(20) {
            println!("             {from:8.2}s → {to:8.2}s  （{:+.2}s）", d);
        }
        for at in refused_by_min.iter().take(10) {
            println!("             弃权：{at:8.2}s 挪过去后某段不足 {:.1}s", super::fx::MIN_SEGMENT_SECS);
        }
        let total = clean + moved.len() + gave_up;
        if total > 0 {
            println!(
                "\n[命中率] 需要挪的占 {:.1}%（{} / {}）",
                (moved.len() + gave_up) as f64 * 100.0 / total as f64,
                moved.len() + gave_up,
                total
            );
        }

        // ③ 真跑一次守卫，确认端到端挪了几处、且第一段起点与末段终点没被动过。
        // ⚠️ 用**全新的副本**：上面的循环改的是 `segs`（把每一处都试挪了一遍），
        // 直接拿它跑"实跑"会得到 0 —— 那是探针自己造成的，不是守卫的行为。
        let mut fresh = config.segments.clone();
        let orig_first = fresh.first().map(|s| s.start);
        let orig_last = fresh.last().map(|s| s.end);
        let n = super::align_segment_boundaries(&mut fresh, &subs);
        println!("\n[实跑] align_segment_boundaries 在原始段表上挪了 {n} 处");
        assert_eq!(fresh.first().map(|s| s.start), orig_first, "首段起点不许被动");
        assert_eq!(fresh.last().map(|s| s.end), orig_last, "末段终点不许被动");

        // ④ 幂等：再跑一次必须是 0。
        let again = super::align_segment_boundaries(&mut fresh, &subs);
        println!("[幂等] 第二次挪了 {again} 处（必须为 0）");
        assert_eq!(again, 0, "守卫必须收敛");
    }
}

/// 只读诊断：在**真实工程**上看「剪辑质量分」（总方案 §3）的实际分布。
///
/// ⚠️ 这个探针在**把分数给用户看之前**必须先跑：分数是"给人看的数"，
/// 而 §3 的判据（边界/爆点/间隔）全是**第一次**用在真实数据上 ——
/// 不先看分布，就无法判断"85 分"到底是好还是烂，也无从发现某条判据在真实数据里
/// 恒真/恒假（那会让分数退化成常数，界面上看着有数、其实什么都没量）。
///
/// ```text
/// VT_POLISH_DIR=<项目目录(绝对路径)> cargo test -p yekitrim-polish --lib \
///   diagnose_quality -- --ignored --nocapture
/// ```
#[cfg(test)]
mod diagnose_quality {
    #[test]
    #[ignore]
    fn diagnose_quality() {
        let Ok(dir) = std::env::var("VT_POLISH_DIR") else {
            eprintln!("用法: VT_POLISH_DIR=<项目目录> cargo test -p yekitrim-polish --lib diagnose_quality -- --ignored --nocapture");
            return;
        };
        let path = std::path::PathBuf::from(&dir);
        let config = crate::project::load_project(&path).expect("读取 polish.toml");
        let subs: Vec<crate::parse::srt::Subtitle> = serde_json::from_str(
            &std::fs::read_to_string(path.join("data/subtitles.json")).expect("读 data/subtitles.json"),
        )
        .expect("解析 subtitles.json");

        // 片子时长：优先 clip.json 的 duration，退到末段 end。
        let duration = std::fs::read_to_string(path.join("clip.json"))
            .ok()
            .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
            .and_then(|v| v.get("duration").and_then(|d| d.as_f64()))
            .or_else(|| config.segments.last().map(|s| s.end))
            .unwrap_or(0.0);

        let q = super::quality::evaluate(&config.segments, &subs, duration);
        println!("\n项目: {dir}");
        println!("时长 {duration:.2}s · 分段 {} · 视觉事件 {}", q.segments, q.visual_events);
        println!("\n[Smoothness] {:.2}（= 时长 / (1 + 干扰)）", q.smoothness());
        println!("[每分钟干扰] {:.2}", q.interruptions_per_minute());
        println!("[展示分] {:.1} / 100", q.score());

        println!("\n[干扰明细] 共 {} 条", q.interruption_count());
        for (k, n) in q.by_kind() {
            println!("  {:<14} {} 条 —— {}", k.key(), n, k.label());
        }
        for i in q.interruptions.iter().take(20) {
            println!("    {:>8.2}s  {}", i.at, i.kind.key());
        }

        // ⚠️ **判据不许恒真/恒假**：三条判据至少各命中过一次，或至少"确实可能命中"。
        //    全 0 不一定是 bug（片子可能真的剪得干净），但**值得看一眼** —— 所以只提示不失败。
        if q.interruption_count() == 0 {
            println!("\n⚠ 零干扰。先确认这不是判据失效：字幕 {} 条、事件 {} 个", subs.len(), q.visual_events);
        }
    }
}

/// 只读诊断：在**真实工程**上看「去静音阈值/气口」与「画中画配额 + 对齐字幕边界」的实际效果。
///
/// ```bash
/// VT_POLISH_DIR=<项目目录(含 polish.toml 与 data/)> \
///   cargo test -p yekitrim-polish --lib diagnose_pip_and_silence -- --ignored --nocapture
/// ```
#[cfg(test)]
mod diagnose_pip_and_silence {
    #[test]
    #[ignore]
    fn diagnose_pip_and_silence() {
        let Ok(dir) = std::env::var("VT_POLISH_DIR") else {
            eprintln!("用法: VT_POLISH_DIR=<项目目录> cargo test -p yekitrim-polish --lib diagnose_pip_and_silence -- --ignored --nocapture");
            return;
        };
        let path = std::path::PathBuf::from(&dir);
        let mut config = crate::project::load_project(&path).expect("读取 polish.toml");
        println!("\n项目: {dir}");

        // ── 去静音 ────────────────────────────────────────────────
        println!(
            "\n[去静音] 阈值 {:.1}s（字幕间隔小于它一律不动）· 两侧气口 {:.2}s → 实际剪掉 = 间隔 − {:.1}s",
            yekitrim_core::cutting::DEFAULT_MIN_GAP_SECS,
            crate::project::SILENCE_PAD_SECS,
            crate::project::SILENCE_PAD_SECS * 2.0
        );
        match crate::project::detect_ranges(&path, true) {
            Ok(ranges) => {
                println!("  当前会写入屏蔽轨道 {} 段：", ranges.len());
                for (s, e) in ranges.iter().take(20) {
                    println!("    {s:7.2}s → {e:7.2}s（{:.2}s）", e - s);
                }
                if ranges.len() > 20 {
                    println!("    … 其余 {} 段略", ranges.len() - 20);
                }
                // 阈值对照：同一份字幕在不同阈值下剪掉几段/多少秒（看这次调整的实际影响）
                let mut row = String::from("  阈值对照（段数 / 总秒数）：");
                for thr in [2.0_f64, 3.0, 3.5, 5.0] {
                    if let Ok(rs) = crate::project::detect_ranges_with(&path, thr, -30.0, true) {
                        let total: f64 = rs.iter().map(|(s, e)| e - s).sum();
                        row.push_str(&format!(" {thr:.1}s→{}/{:>.1}s ", rs.len(), total));
                    }
                }
                println!("{row}");
            }
            Err(e) => println!("  检测失败：{e:#}"),
        }

        // ── 画中画配额 + 对齐 ─────────────────────────────────────
        let subtitles: Vec<crate::parse::srt::Subtitle> = {
            let p = path.join("data").join("subtitles.json");
            std::fs::read_to_string(&p)
                .ok()
                .and_then(|raw| serde_json::from_str(&raw).ok())
                .unwrap_or_default()
        };
        let before: Vec<(f64, f64, String)> = config
            .segments
            .iter()
            .flat_map(|s| s.events.iter())
            .filter_map(|e| match e {
                crate::project::Event::Camera { time, end, zoom, .. } if zoom == "远" => {
                    Some((*time, end.unwrap_or(*time), zoom.clone()))
                }
                _ => None,
            })
            .collect();
        let demoted = super::cap_pip_shots(&mut config.segments);
        let aligned = super::align_pip_shots(&mut config.segments, &subtitles);
        println!(
            "\n[画中画] 字幕 {} 条 · 配额 {} 条 · 降级 {demoted} 条 · 对齐 {aligned} 条",
            subtitles.len(),
            super::MAX_PIP_SHOTS
        );
        if before.is_empty() {
            println!("  这个工程没有「远」事件");
        }
        // 实际挂了多久：运镜是状态机，画中画一直挂到**下一条运镜**，不是挂到自己的终点
        let mut cam_times: Vec<f64> = config
            .segments
            .iter()
            .flat_map(|s| s.events.iter())
            .filter_map(|e| match e {
                crate::project::Event::Camera { time, .. } => Some(*time),
                _ => None,
            })
            .collect();
        cam_times.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        for (t0, t1, _) in &before {
            let until = cam_times
                .iter()
                .copied()
                .find(|t| *t > t0 + 1e-6 && *t >= t1 - 1e-6)
                .map_or_else(|| "片尾".to_string(), |t| format!("{t:.2}s"));
            println!("  原 {t0:7.2}s → {t1:7.2}s（实际挂到 {until}）");
        }
        let closed = super::close_pip_shots(&mut config.segments);
        println!("  到点收工：补回镜头 {closed} 条");
        for seg in &config.segments {
            for (i, ev) in seg.events.iter().enumerate() {
                if let crate::project::Event::Camera { time, end, zoom, .. } = ev {
                    if zoom == "远" {
                        let pip_end = end.unwrap_or(*time);
                        println!("  后 {time:7.2}s → {pip_end:7.2}s（段 {:.2}~{:.2}）", seg.start, seg.end);
                        // 紧跟其后、起点正好接在画中画终点上的那条就是补出来的回镜头
                        if let Some(crate::project::Event::Camera { time: t2, zoom: z2, origin: o2, .. }) =
                            seg.events.get(i + 1)
                        {
                            if (t2 - pip_end).abs() < 0.01 {
                                // 它的 end 只写在段内的那点余量；景别真正保持到下一条运镜
                                let held = cam_times
                                    .iter()
                                    .copied()
                                    .find(|t| *t > t2 + 1e-6)
                                    .map_or_else(|| "片尾".to_string(), |t| format!("{t:.2}s"));
                                println!("    ↳ 到点回镜头 {t2:.2}s 起（{z2} @ {o2}，持到 {held}）");
                            }
                        }
                    }
                }
            }
        }
        // VT_CAM_JSON=1：把处理后的运镜轨道打成一行 JSON，喂给前端的求值器做端到端对拍
        if std::env::var("VT_CAM_JSON").is_ok() {
            let evs: Vec<serde_json::Value> = config
                .segments
                .iter()
                .flat_map(|s| s.events.iter())
                .filter_map(|e| match e {
                    crate::project::Event::Camera { time, end, zoom, origin, cut, .. } => {
                        Some(serde_json::json!({
                            "time": time, "end": end, "dur": end.map(|e| e - *time), "zoom": zoom, "origin": origin, "cut": cut
                        }))
                    }
                    _ => None,
                })
                .collect();
            if let Some(pip) = before.first() {
                println!("CAM_JSON={}", serde_json::json!({
                    "before": serde_json::json!({"time": pip.0, "end": pip.1, "dur": pip.1 - pip.0, "zoom": "远"}),
                    "after": evs,
                    "zoom_wide": config.style.camera.zoom_wide,
                }));
            }
        }

        // ── 贴纸对齐最近字幕 ─────────────────────────────────────
        let before_stickers: Vec<(f64, f64, String)> = config
            .segments
            .iter()
            .flat_map(|s| s.events.iter())
            .filter_map(|e| match e {
                crate::project::Event::Sticker { time, end, name, keyword, .. } => {
                    Some((*time, end.unwrap_or(*time), name.clone().unwrap_or_else(|| keyword.clone())))
                }
                _ => None,
            })
            .collect();
        let stickers = super::align_stickers_to_subtitles(&mut config.segments, &subtitles);
        println!("\n[贴纸] 共 {} 条 · 对齐 {stickers} 条（离最近字幕 > {:.1}s 的不动）", before_stickers.len(), super::STICKER_SNAP_MAX_SECS);
        for (i, (t0, t1, name)) in before_stickers.iter().enumerate() {
            let after = config
                .segments
                .iter()
                .flat_map(|s| s.events.iter())
                .filter_map(|e| match e {
                    crate::project::Event::Sticker { time, end, .. } => Some((*time, end.unwrap_or(*time))),
                    _ => None,
                })
                .nth(i);
            let (a0, a1) = after.unwrap_or((*t0, *t1));
            let mark = if (a0 - t0).abs() > 1e-6 || (a1 - t1).abs() > 1e-6 { "→" } else { "=" };
            println!("  [{i:2}] {t0:7.2}~{t1:7.2}s {mark} {a0:7.2}~{a1:7.2}s  {name}");
        }
    }
}

/// 只读诊断：扫真实工程的覆盖层事件，打印「原始字段 → 解析后的有效色档」。
///
/// 用来验证老工程（LLM 只写了单个 `color`）现在能不能画出东西 —— 这正是
/// 「覆盖层渐变没效果」的病灶：真实数据 4 个项目 8 条覆盖层全是
/// `overlay_type = "gradient"` + 单个 `color`，而渲染侧只认 `colors`，于是全都没画。
/// 修复后单色按「该色 → 透明」兜底，**不需要重跑 LLM**。
///
/// ```text
/// VT_POLISH_DIR=<项目目录> cargo test -p yekitrim-polish --lib \
///   diagnose_overlay_stops -- --ignored --nocapture
/// ```
///
/// ⚠ 路径要写**绝对路径**：cargo test 把测试进程的 CWD 设在 **crate 根**
/// （`crates/yekitrim-polish`），不是 workspace 根，写相对路径会找不到 `polish.toml`。
#[cfg(test)]
mod diagnose_overlay_stops {
    use crate::project::Event;

    #[test]
    #[ignore]
    fn diagnose_overlay_stops() {
        let Ok(dir) = std::env::var("VT_POLISH_DIR") else {
            eprintln!("用法: VT_POLISH_DIR=<项目目录> cargo test -p yekitrim-polish --lib diagnose_overlay_stops -- --ignored --nocapture");
            return;
        };
        let path = std::path::PathBuf::from(&dir);
        let config = crate::project::load_project(&path).expect("读取 polish.toml");
        println!("\n项目: {dir}");

        let mut total = 0usize;
        let mut broken = 0usize;
        for seg in &config.segments {
            for ev in &seg.events {
                let Event::Overlay {
                    time,
                    end,
                    overlay_type,
                    color,
                    opacity,
                    colors,
                    shape,
                    ..
                } = ev
                else {
                    continue;
                };
                total += 1;
                let stops = yekitrim_render_core::timeline::overlay_stops(color, colors);
                let verdict = match (&stops, overlay_type.as_str()) {
                    (Some(s), _) if s.len() >= 2 => format!("→ {}", s.join(" , ")),
                    (_, other) => format!("→ 不渲染（类型 {other:?} 且没有可用色档）"),
                };
                if let (true, None) = (overlay_type == "gradient", &stops) {
                    broken += 1;
                }
                println!(
                    "  [{:6.1}~{:6.1}s] type={:<8} opacity={:<5?} shape={:<8?} color={:<12?} colors={:?}\n            {verdict}",
                    time,
                    end.unwrap_or(*time),
                    overlay_type,
                    opacity,
                    shape,
                    color,
                    colors
                );
            }
        }
        println!("\n共 {total} 条覆盖层；其中渐变为空且无 color 的 {broken} 条");
        if total > 0 {
            println!("（单色按「该色 → 透明」渲染；色档数组原样使用。四处实现：timeline.rs / index.html / engine.js / PolishPreview.vue）");
        }
    }
}
