//! polish::llm：精修侧 LLM 流水线的串联（全局分段 → 逐段事件 → 关键词 → 逐段状态快照）。
//!
//! 入口：`run_llm_pipeline`、`regenerate_segment_events`；子模块 `segment` / `events` / `keywords` / `status`。
//! 边界：一律走 `yekitrim_core::llm::LlmClient::from_config` 并挂 `with_usage_context`（不挂就是空 label）；空结果/失败一律不覆盖已有事件；落盘前的运镜与守卫与 `process` 是两份重复实现，要一起改 —— 见 `crates/yekitrim-polish/src/llm/AGENTS.md`

pub mod segment;
pub mod keywords;
pub mod events;
pub mod status;

use std::path::{Path, PathBuf};

use crate::parse::srt::Subtitle;
use crate::project::{Segment, Event};
use yekitrim_core::config::LlmConfig;
use yekitrim_core::danmaku::DanmakuEntry;
use yekitrim_core::llm::LlmClient;

use status::SegmentGenStatus;

/// 运行 LLM 全流程: 话题区间 + 关键词提取
/// LLM 直接在 segment.events 中生成事件, 不再经 reaction_rules 中转
/// sticker_source 控制提示词中是否提及 sticker 及其语言风格
/// sticker_budget/sfx_budget: 数量上限, 提示词中明确约束
/// sfx_blacklist: 被屏蔽的音效/标签, 注入 prompt
///
/// 返回 `(segments, 弹幕筛选詞, 高亮词, 逐段生成状态, 成片时间线)` —— 第 5 项是
/// **冷开场/回忆回溯意图落实出来的 `edl.clips`**（`None` = 这一段没有意图 ⇒ 调用方
/// **不许碰** `cfg.edl`）；最后一项供调用方落盘成快照，编辑器据此给"没产出事件的段"上警示色
/// （见 `llm::status`）。
/// ⚠️ **`owner` 必须显式传**：这个函数在 **server 进程内**被任务线程直接调用
/// （`cmd_llm` ← `polish_spawn` 的 `"llm"` 分支），而那里没有 `YEKITRIM_OWNER`
/// —— 用 `sticker_dir_readable()` 会去翻单人库，于是多用户下"选不中自己传的贴纸包"。
pub async fn run_llm_pipeline(
    owner: yekitrim_core::owner::Owner,
    llm_cfg: &LlmConfig,
    subtitles: &[Subtitle],
    danmaku: &[DanmakuEntry],
    event_type: &str,
    sticker_source: &[String],
    sticker_budget: usize,
    sfx_budget: usize,
    sfx_blacklist: &[String],
    // 屏蔽区间（§17 跳切的接缝判据读它）。传 `&[]` = 没有屏蔽 ⇒ 不认任何跳切。
    mute_ranges: &[crate::project::MuteRange],
    checkpoint_dir: Option<&PathBuf>,
    cancel: Option<yekitrim_core::llm::agent::CancelProbe>,
) -> anyhow::Result<(
    Vec<Segment>,
    Vec<String>,
    Vec<String>,
    Vec<SegmentGenStatus>,
    Option<Vec<crate::project::EdlClip>>,
)> {
    let project = checkpoint_dir.map(|d| d.parent().unwrap_or(d).to_path_buf());
    // 链路解析：精修这一整条（分段 / 逐段事件 / 贴纸精挑 / 关键词）共用 `[llm.overrides.polish]`。
    // thinking 的"默认关思考"也在这里给出，别在下面各调用点再写一遍 with_thinking。
    let linked = llm_cfg.for_link(yekitrim_core::config::LlmLink::Polish);
    let llm_cfg = &linked;
    // 用量归因：链路名 + 项目（切片）目录
    let usage = |label: &str| {
        yekitrim_core::llm::usage::UsageContext::new(label)
            .project(project.as_ref().map(|p| yekitrim_core::path::display(p)))
    };

    let client = std::sync::Arc::new(
        LlmClient::from_config(llm_cfg)?
            .with_usage_context(usage("精修分段")),
    );

    // Step 1: 分段 + 段情绪密度（全局单次调用，预算感知采样）
    println!("  ── LLM: Segmentation (type: {}) ──", event_type);
    // §10 **情绪热区** —— 让分段不只看话题。
    //
    // ⚠️ 用与 `process` 写 `data/emotion.json` **同一份判据**（弹幕优先、无弹幕退字幕），
    //    否则"分段按一种情绪切、时间轴徽标显示另一种"。
    // ⚠️ 窗口时长取 `detection.danmaku_window_secs`（全局配置）—— 与 `process::emotion_window_secs`
    //    同源；这里拿不到 `ProjectFile`，所以传全局那份。（分段时间轴与窗口时长无关，
    //    只有"写进提示词的秒数范围"用它。）
    let emo_w = yekitrim_core::config::load_global_config().detection.danmaku_window_secs;
    let emo_total = subtitles.last().map(|s| s.end).unwrap_or(0.0);
    let emotion_windows = if !danmaku.is_empty() && emo_total > 0.0 {
        yekitrim_core::danmaku::emotion::track(danmaku, emo_w, emo_total)
    } else if emo_total > 0.0 {
        // ⚠️ 无弹幕 ⇒ 用字幕语气（与 process 的 `emotion_from_subtitles` 同口径）。
        //    这里就地算，是因为那个函数是 polish::process 的私有项。
        segment::emotion_from_subtitles_local(subtitles, emo_w, emo_total)
    } else {
        Vec::new()
    };
    if !emotion_windows.is_empty() {
        println!("  · 情绪热区：{} 个窗口（窗口 {emo_w:.1}s）→ 喂给分段", emotion_windows.len());
    } else {
        println!("  · 情绪热区：无（没弹幕且字幕也判不出）—— 分段按纯话题切");
    }
    let mut segments = match segment::generate_segments(
        &client, subtitles, event_type, sticker_source, sticker_budget, sfx_budget, sfx_blacklist,
        &emotion_windows, emo_w,
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
    let sticker_root = yekitrim_core::owner::sticker_dir_readable(owner);
    let has_local = sticker_source.iter().any(|s| {
        crate::assets::sticker_index::is_package_source(&sticker_root, s)
    });
    let local_sticker_text = if has_local {
        crate::assets::sticker_index::local_sticker_candidates_text(&sticker_root)
    } else {
        String::new()
    };
    let statuses = events::run_event_agent(
        &mut segments,
        subtitles,
        candidate_texts,
        sticker_source,
        sfx_blacklist,
        local_sticker_text,
        llm_cfg,
        project.as_deref(),
        checkpoint_path.as_deref(),
        cancel.clone(),
    ).await?;
    println!(
        "  ✓ {} / {} 段生成成功",
        statuses.iter().filter(|s| s.ok).count(),
        statuses.len()
    );

    // Step 2b: 本地贴纸按 tag 批量精挑具体文件
    let sticker_root = yekitrim_core::owner::sticker_dir_readable(owner);
    let has_local = sticker_source.iter().any(|s| {
        crate::assets::sticker_index::is_package_source(&sticker_root, s)
    });
    if has_local {
        run_sticker_choice_step(owner, &mut segments, subtitles, llm_cfg, project.as_deref(), cancel.clone()).await?;
    }

    // Step 3: 关键词提取
    println!("  ── LLM: Keyword extraction ──");
    let sub_texts: Vec<String> = subtitles.iter().map(|s| s.text.clone()).collect();
    let dm_texts: Vec<String> = danmaku.iter().map(|d| d.text.clone()).collect();
    // 单开一个带自身归因的客户端：与分段那一路在用量表里分开统计
    let kw_client = LlmClient::from_config(llm_cfg)?
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
                // 手持摇摆是可选项：补默认运镜从不自己开（非近景也会被守卫剔除）
                handheld: false,
                manual: false,
                speed: None,
                tilt: None,
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
        // 句末边界守卫（总方案 §2）：与 `process/mod.rs` Step 0b 是**配对的两处**。
        // ⚠️ **排在 `cap_pip_shots` 那组之后**：那组按段边界对齐画中画，边界挪了就得重对齐；
        // 反过来排会让画中画停在旧边界上（编辑器里看着对、出片再被 process 挪一次就错位）。
        let moved = crate::process::align_segment_boundaries(&mut segments, subtitles);
        if moved > 0 {
            tracing::info!("  ✂ 句末边界守卫：挪了 {moved} 处分段边界（原来切在句子中间）");
        }
        // 跳切配额（§17）：与 `process/mod.rs` Step 1e 是**配对的两处**（§16/§17 的成对纪律）。
        // ⚠️ **排在句末边界守卫之后**：接缝判据读 `mute_ranges`（源时间轴，与段边界无关），
        // 但跳切**挂在哪一段**取决于段表 —— 边界刚挪过，这里算出来的才是最终归属。
        let jumps = crate::process::cap_jump_cuts(&mut segments, mute_ranges, subtitles);
        if jumps > 0 {
            tracing::info!("  ⚠ 跳切配额（§17）：{jumps} 处降级为平滑运镜");
        }
    }

    // Step 4c: 冷开场 / 回忆回溯意图 → `edl.clips`（第 5 批）。与 Step 4b 同一条理由：
    // LLM 是按段窗口生成的、数不出"整片几次"，所以配额在这里先兜一遍，
    // **编辑器里看到的就已经是合规的**（意图被收走、配额已裁）；`cmd_process` 里还会再兜一次（幂等）。
    //
    // ⚠️ 这里**只在真的落实了才动 EDL**（`changed()`）：一支意图都没有 ⇒ 一个新段都不加，
    // `edl.clips` 保持为空 —— 有 EDL 时 `mute_ranges` 整个被忽略（`ProjectFile::resolve_edl`），
    // 所以"没意图就别写"是 R4 存量零回归的判据本身。用户手拖过的 `clips` 在这里**读不到**
    // （本函数只拿得到 `segments`，写盘那一步在 `cmd_llm` 里按 `changed()` 决定要不要合并）。
    let materialized = {
        let mut edl = crate::project::Edl::default();
        let total_duration = segments.last().map(|s| s.end).unwrap_or(0.0);
        let m = crate::process::materialize_edl_intents(&mut segments, &mut edl.clips, total_duration);
        if m.changed() {
            tracing::info!(
                "  ➕ 成片时间线意图：冷开场 {} · 回忆回溯 {} 条",
                if m.cold_open { "1 条" } else { "无" },
                m.recalls
            );
            Some(edl.clips)
        } else {
            None
        }
    };

    // Step 5: emoji/irasu 位置计算 ("auto" → 计算值, 未来实现)
    // LLM 输出的 x/y 如果是 "auto", 由 cmd_process 统一计算

    Ok((segments, dm_filter, hl_words, statuses, materialized))
}

/// **只重做某一段**的事件（编辑器里分段属性面板上的那颗按钮）。
///
/// 与整片重做的区别（这些都是**有意**的）：
/// - **不重跑 Step1 分段**：用户点的是"重做这一段的事件"，段的时间边界必须原样保留；
/// - **不走 checkpoint**：单段重做的全部意义就是"这一次要重新问模型"，读缓存等于什么都不做；
/// - 只把该段的事件换掉，别的段一个字节都不动；
/// - 仍然跑 Step2b（贴纸精挑）与段级的对齐，否则新生成的事件缺 `name` / 没吸附字幕。
///
/// 生成失败（429 等）时**保留该段原有事件**并如实返回失败状态
/// —— `apply_outputs` 对空结果不覆盖，这里也不动 polish.toml（调用方只写成功的结果）。
pub async fn regenerate_segment_events(
    owner: yekitrim_core::owner::Owner,
    llm_cfg: &LlmConfig,
    subtitles: &[Subtitle],
    sticker_source: &[String],
    sfx_blacklist: &[String],
    event_type: &str,
    segments: &mut [Segment],
    index: usize,
    project: Option<&Path>,
    cancel: Option<yekitrim_core::llm::agent::CancelProbe>,
) -> anyhow::Result<SegmentGenStatus> {
    let Some(seg) = segments.get(index) else {
        anyhow::bail!("分段下标越界: {index}（当前只有 {} 段）", segments.len());
    };
    let (start, end) = (seg.start, seg.end);

    // 段内没有字幕就没什么可生成的（也与整片那条路同一口径：窗口字幕来自 ±3s 过滤）。
    // 提前返回是**有意的**：省一次调用，而且给出的原因比"模型返回空"准确。
    let has_subs = subtitles
        .iter()
        .any(|s| s.end >= start - 3.0 && s.start <= end + 3.0);
    if !has_subs {
        return Ok(SegmentGenStatus::failed(
            index, start, end, "这一段没有字幕，无法生成事件", seg.events.len(),
        ));
    }

    // 该段的音效候选：与整片那条路同一个入口（按 clip 方向筛过的候选清单）
    let tags = sfx_tags_for_event_type(event_type);
    let candidate_text = crate::assets::sfx_catalog::candidates_text(
        &tags,
        &std::collections::HashSet::new(),
        32,
        sfx_blacklist,
    );
    let sticker_root = yekitrim_core::path::sticker_dir_readable();
    // ⚠️ 用 §is_known_pack_source§（**磁盘 ∪ 内建 emoji 包**）而不是 §is_package_source§：
    //    后者是纯磁盘判据，而内建包不是目录 ⇒ 来源只写 §emoji§ 的工程会**整段候选文本都不注入**
    //    （那正是"LLM 不知道有哪些词可用"的一个入口）。
    let has_local = sticker_source
        .iter()
        .any(|s| crate::assets::sticker_index::is_known_pack_source(&sticker_root, s));
    let local_sticker_text = if has_local {
        crate::assets::sticker_index::local_sticker_candidates_text(&sticker_root)
    } else {
        String::new()
    };

    // 只把这一段做成一个窗口跑（checkpoint = None → 一定重新问模型）
    let mut one = vec![segments[index].clone()];
    let statuses = events::run_event_agent(
        &mut one,
        subtitles,
        vec![candidate_text],
        sticker_source,
        sfx_blacklist,
        local_sticker_text,
        llm_cfg,
        project,
        None,
        cancel.clone(),
    )
    .await?;
    let status = statuses
        .into_iter()
        .next()
        .unwrap_or_else(|| {
            SegmentGenStatus::failed(index, start, end, "没有拿到该段的结果", 0)
        });

    // 失败/空结果：不动 segments[index]（保持用户原来那一份）
    if !status.ok {
        tracing::warn!("  ⚠ 段 {index} 重做未产出事件：{}（保留原有事件）", status.reason);
        return Ok(status);
    }

    // Step2b：本地贴纸精挑（只在这一段上跑）
    if has_local {
        run_sticker_choice_step(owner, &mut one, subtitles, llm_cfg, project, cancel).await?;
    }
    // 与整片那条路同一批段级收尾：贴纸吸附最近字幕 + 画中画守卫
    crate::process::align_stickers_to_subtitles(&mut one, subtitles);
    crate::process::cap_pip_shots(&mut one);
    crate::process::align_pip_shots(&mut one, subtitles);
    crate::process::close_pip_shots(&mut one);

    let events = std::mem::take(&mut one[0].events);
    let count = events.len();
    segments[index].events = events;
    Ok(SegmentGenStatus::ok(index, start, end, count))
}

/// Step2b：读取本地贴纸包 TOML，按 `(包名, sticker_tag)` 分组批量精挑具体文件，写回 event.name/source。
/// emoji 精挑的候选标签：形如 `1f602.gif 😂`。
///
/// 抽成函数是为了**可测** —— 它把两条约定钉在一起：
/// ① 模型看得懂（带真实字符；否则 `1f602.gif` 这种文件名它认不出是哪个表情，精挑等于随机）；
/// ② `Path::file_stem()` 取回的**正好是码点** —— `choose_sticker_files_batch` 拿它当 `name`
///    写盘，`process` 与转译器再按码点去找文件。
fn emoji_candidate_label(cp: &str) -> String {
    format!("{cp}.gif {}", crate::assets::emoji_search::codepoint_to_chars(cp))
}

#[cfg(test)]
mod refine_tests {
    use super::*;

    /// 候选标签必须**双向对得上**：模型读得懂、取回来就是码点。
    ///
    /// 变异自证：把标签改成不带字符的 `{cp}.gif` ⇒ 第一条断言红；
    /// 改成 `{cp} 😂`（没有扩展名）⇒ 第二条断言红（`file_stem` 会带上空格与字符）。
    #[test]
    fn emoji_candidate_labels_round_trip_to_the_codepoint() {
        for cp in ["1f602", "203c_fe0f", "2764_fe0f"] {
            let label = emoji_candidate_label(cp);
            assert!(
                label.starts_with(&format!("{cp}.gif ")) && label.len() > cp.len() + 5,
                "标签要带可读字符: {label:?}"
            );
            assert_eq!(
                std::path::Path::new(&label).file_stem().and_then(|s| s.to_str()),
                Some(cp),
                "标签 {label:?} 取回的不是码点 —— 写盘就会变成一个不存在的文件名"
            );
        }
    }

    /// **只有维度查询才进 emoji 精挑**：老工程的自由词不打乱存量行为。
    ///
    /// （判据本体在 `emoji_taxonomy::parse_query`，这里钉的是"调用点用的是它"。）
    #[test]
    fn only_dimension_queries_are_treated_as_emoji_events() {
        for q in ["emotion:joy", "emotion:joy,usage:punchline", "usage:meme|reaction"] {
            assert!(
                !crate::assets::emoji_taxonomy::parse_query(q).is_empty(),
                "{q} 应当被当成 emoji 查询"
            );
        }
        for w in ["laugh", "angry", "ysyk", "emotion:ecstatic"] {
            assert!(
                crate::assets::emoji_taxonomy::parse_query(w).is_empty(),
                "{w} 不该进 emoji 精挑（老工程的自由词 / 词表外的词）"
            );
        }
    }
}

/// emoji 精挑喂给 LLM 的候选数：太多会把 prompt 撑长、太少挑不出想要的。
const EMOJI_REFINE_CANDIDATES: usize = 16;

async fn run_sticker_choice_step(
    owner: yekitrim_core::owner::Owner,
    segments: &mut [Segment],
    subtitles: &[Subtitle],
    llm_cfg: &LlmConfig,
    project: Option<&std::path::Path>,
    cancel: Option<yekitrim_core::llm::agent::CancelProbe>,
) -> anyhow::Result<()> {
    use crate::assets::sticker_index::{load_package_meta, tag_files};
    use crate::llm::events::{choose_sticker_files_batch, StickerChoiceContext};
    use std::collections::BTreeMap;

    let root = yekitrim_core::owner::sticker_dir_readable(owner);

    // 收集所有带 sticker_tag 的本地贴纸事件，按 (包名, tag) 分组
    // key: (package_name, tag)
    let mut groups: BTreeMap<(String, String), Vec<StickerChoiceContext>> = BTreeMap::new();
    let mut package_names: std::collections::HashSet<String> = std::collections::HashSet::new();
    for (si, seg) in segments.iter().enumerate() {
        for (ei, ev) in seg.events.iter().enumerate() {
            // ⚠️ **两个载体都要收**（`process/AGENTS.md`「贴纸有两个载体」）：只认 `Event::Sticker`
            // 时，编辑器「贴纸轨」建的贴纸**永远不参与 Step2b 精挑**。
            let Some(tag) = ev.sticker_tag() else { continue };
            if let Some(keyword) = ev.sticker_keyword() {
                let kw = keyword.trim().to_string();
                let tag = tag.trim().to_string();
                if kw.is_empty() || tag.is_empty() {
                    continue;
                }
                package_names.insert(kw.clone());
                let time = ev.sticker_time().unwrap_or(0.0);
                let ctx = StickerChoiceContext {
                    segment_index: si,
                    event_index: ei,
                    time,
                    context_text: subtitle_context_text(subtitles, time),
                };
                groups.entry((kw, tag)).or_default().push(ctx);
            }
        }
    }

    for (package_name, tag) in groups.keys() {
        // 每个 (包, tag) 都是一次独立 LLM 调用：逐个查取消，别把剩下的成批发完
        if cancel.as_ref().is_some_and(|c| c()) {
            anyhow::bail!("任务已取消");
        }
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
                    // 写回也走访问器（两个载体的 `name`/`source` 字段不同名）
                    if let Some(seg) = segments.get_mut(si)
                        && let Some(ev) = seg.events.get_mut(ei)
                        && ev.is_sticker()
                    {
                        ev.set_sticker_resolved(&package_name, &stem);
                    }
                }
            }
            Err(e) => {
                tracing::warn!("  ⚠ Step2b 批量选择失败 (tag={tag}): {e:#}");
            }
        }
    }


    // ------------------------------------------------------------------
    // emoji 那一档（2026-10-17）：它的 keyword 是一条**维度查询**
    // （emotion:joy,usage:punchline），而 sticker_tag 是空的 —— 所以上面那个
    // (包名, tag) 分组**永远收不到它**，这正是"emoji 是全链路唯一没有复核那一档"的直接原因。
    // 这里补上同款精挑：候选 = search_best(keyword, N)，让 LLM 挑一个具体表情。
    // ------------------------------------------------------------------
    {
        use crate::assets::emoji_search::EmojiCatalog;
        let mut emoji_groups: BTreeMap<String, Vec<StickerChoiceContext>> = BTreeMap::new();
        for (si, seg) in segments.iter().enumerate() {
            for (ei, ev) in seg.events.iter().enumerate() {
                if !ev.is_sticker() {
                    continue;
                }
                let Some(kw) = ev.sticker_keyword() else { continue };
                let kw = kw.trim();
                // 只认**新的维度查询**（含 dim:value）：老工程的自由词（laugh / angry）
                // 继续走原来的路 —— 不打乱存量行为，也不为一堆老事件白烧调用。
                if crate::assets::emoji_taxonomy::parse_query(kw).is_empty() {
                    continue;
                }
                let time = ev.sticker_time().unwrap_or(0.0);
                emoji_groups
                    .entry(kw.to_string())
                    .or_default()
                    .push(StickerChoiceContext {
                        segment_index: si,
                        event_index: ei,
                        time,
                        context_text: subtitle_context_text(subtitles, time),
                    });
            }
        }
        match EmojiCatalog::fetch_embedded() {
            Err(e) => {
                if !emoji_groups.is_empty() {
                    tracing::warn!("  ⚠ emoji 目录读不出来，跳过 emoji 精挑: {e}");
                }
            }
            Ok(cat) => {
                for (query, contexts) in &emoji_groups {
                    // 每个查询一次独立调用：逐个查取消，别把剩下的成批发完
                    if cancel.as_ref().is_some_and(|c| c()) {
                        anyhow::bail!("任务已取消");
                    }
                    let hits = cat.search_best(query, EMOJI_REFINE_CANDIDATES);
                    if hits.is_empty() {
                        tracing::warn!("  ⚠ emoji 查询「{query}」一个候选都没有，跳过精挑");
                        continue;
                    }
                    // 候选标签形如 "1f602.gif 😂"：模型认得出那是哪个表情，
                    // 而 choose_sticker_files_batch 用 Path::file_stem() 取回的**正好是码点**
                    // （= 写进 name 的东西，process 与转译器都按它找文件）。
                    let candidates: Vec<String> =
                        hits.iter().map(|cp| emoji_candidate_label(cp)).collect();
                    let chosen: Vec<(usize, usize, String)> = if candidates.len() == 1 {
                        // 只有一个候选 ⇒ 没必要花一次调用（能自己定的别问模型）
                        let stem = hits[0].clone();
                        contexts
                            .iter()
                            .map(|c| (c.segment_index, c.event_index, stem.clone()))
                            .collect()
                    } else {
                        match choose_sticker_files_batch(
                            llm_cfg, project, "emoji", query, &candidates, contexts,
                        )
                        .await
                        {
                            Ok(v) => v,
                            Err(e) => {
                                tracing::warn!("  ⚠ emoji 精挑失败 (query={query}): {e:#}");
                                Vec::new()
                            }
                        }
                    };
                    for (si, ei, stem) in chosen {
                        if let Some(seg) = segments.get_mut(si)
                            && let Some(ev) = seg.events.get_mut(ei)
                            && ev.is_sticker()
                        {
                            // 与 process Step 2 逐字同形：source = "emoji"、name = 码点
                            ev.set_sticker_resolved("emoji", &stem);
                        }
                    }
                }
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
        role: String::new(),
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
