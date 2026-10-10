//! polish::process::fx：事件默认值补齐与守卫（瞬时特效跨度 / 画中画配额与对齐 / 手持摇摆 / 贴纸对齐字幕）。
//!
//! 入口：`transient_duration_cap`、`transient_default_duration`、`normalize_event_ends`、`cap_pip_shots`、`align_pip_shots`、`close_pip_shots`、`cap_handheld_sway`、`align_stickers_to_subtitles`。
//! 边界：手工事件（`Event::is_time_manual()`）一律跳过改写；这些守卫在 `process/mod.rs` 里的调用顺序是硬约束，改一处要看整条 Step 序 —— 见 `crates/yekitrim-polish/src/process/AGENTS.md`

use crate::parse::srt::Subtitle;
use crate::project::{Decision, EdlClip, Event, Segment};

/// 瞬时特效的 duration 上限（秒）。
/// 原则：瞬时特效只能“爆发一下”，不允许长时间驻留。
pub fn transient_duration_cap(event_type: &str) -> f64 {
    match event_type {
        "shake" => 0.5,
        "flash" => 0.3,
        "blur" => 0.4,
        "noise" => 0.4,
        "pulse" => 0.5,
        "hue_shift" => 0.5,
        "vignette" => 0.6,
        _ => f64::INFINITY,
    }
}

/// 瞬时特效的默认 duration（LLM 未写时使用）。
pub fn transient_default_duration(event_type: &str) -> f64 {
    match event_type {
        "shake" => 0.3,
        "flash" => 0.25,
        "blur" => 0.3,
        "noise" => 0.3,
        "pulse" => 0.4,
        "hue_shift" => 0.35,
        "vignette" => 0.4,
        _ => 0.5,
    }
}

/// 事件对应的字符串类型名。
fn event_type_name(event: &Event) -> &'static str {
    match event {
        Event::Shake { .. } => "shake",
        Event::Flash { .. } => "flash",
        Event::Blur { .. } => "blur",
        Event::Noise { .. } => "noise",
        Event::Pulse { .. } => "pulse",
        Event::HueShift { .. } => "hue_shift",
        Event::Vignette { .. } => "vignette",
        _ => "",
    }
}

/// 归一化所有事件的时间跨度（`time` + `end`）：
/// - 缺 `end` 的按类型补默认（`Event::default_end`）；`sfx` 例外 —— 缺省 = 用文件原始长度
/// - 瞬时特效的跨度 clamp 到 [0.1, 上限]（上限见 [`transient_duration_cap`]）
pub fn normalize_event_ends(segments: &mut [Segment]) {
    for seg in segments {
        for event in &mut seg.events {
            normalize_event_end(event);
        }
    }
}

/// 画中画（`zoom = "远"`）在**整片**里最多几条（用户要求：一个切片只能有一次）。
pub const MAX_PIP_SHOTS: usize = 1;

/// 画中画降频守卫（用户要求：一个切片只能有一次）。
///
/// **LLM 是按段窗口生成的，数不出"整片几次"**，所以配额只能在这里兜：按时间序遍历所有运镜事件，
/// 只保留**最早的一条**「远」，其余降级成「中」（只改 zoom，anchor/origin 原样留着）。
/// 上一版还有"不能连续 / 间隔 ≥60s"两条规则 —— 配额收到 1 之后被完全包含，删掉了。
///
/// 放在 `cmd_process` 补默认运镜**之后**调用。幂等：降级后的轨道再跑一次不会再变。
/// 返回被降级的条数。
pub fn cap_pip_shots(segments: &mut [Segment]) -> usize {
    // 收集**全部**运镜事件并按时间排序：要能分辨"哪条最早"
    let mut order: Vec<(usize, usize, f64)> = Vec::new();
    for (si, seg) in segments.iter().enumerate() {
        for (ei, ev) in seg.events.iter().enumerate() {
            if let Event::Camera { time, .. } = ev {
                order.push((si, ei, *time));
            }
        }
    }
    order.sort_by(|a, b| a.2.partial_cmp(&b.2).unwrap_or(std::cmp::Ordering::Equal));

    let mut demoted = 0usize;
    let mut kept = 0usize;
    for (si, ei, time) in order {
        let is_pip = matches!(
            &segments[si].events[ei],
            Event::Camera { zoom, .. } if zoom.trim() == PIP_LABEL
        );
        if !is_pip {
            continue;
        }
        if kept >= MAX_PIP_SHOTS {
            if let Event::Camera { zoom, .. } = &mut segments[si].events[ei] {
                *zoom = "中".into();
            }
            demoted += 1;
            tracing::info!("  ⚠ 画中画太密（{:.1}s · 整片只允许 {} 条）：降级为中景", time, MAX_PIP_SHOTS);
        } else {
            kept += 1;
        }
    }
    demoted
}

/// 画中画切镜的**最短时长**（秒）：对齐后短于这个就往后借下一条字幕，借不满就保持原样。
pub const MIN_PIP_SECS: f64 = 1.5;

/// 画中画切镜对齐到**字幕边界**（没有字幕就用所属分段的边界兜底）——用户要求：
/// 「要对其字幕或者事件，不能没头没尾的断开」。
///
/// - 起点吸到**包含它的那条字幕的开始**；起点落在字幕空隙里就吸到下一条字幕的开始。
/// - 终点吸到**包含它的那条字幕的结束**；终点落在空隙里就吸到上一条字幕的结束。
/// - 结果夹在所属分段 `[start, end]` 内（分段就是这一段的"事件"边界，切镜不该跨段）。
/// - 对齐后不足 [`MIN_PIP_SECS`]：往后延到下一条字幕结束（仍然对齐），到段尾还不足就保持原样。
///
/// 只动 `time`/`end`，`zoom`/`anchor`/`origin` 不碰。返回被调整过的条数。
pub fn align_pip_shots(segments: &mut [Segment], subtitles: &[Subtitle]) -> usize {
    let mut aligned = 0usize;
    for seg in segments.iter_mut() {
        let (seg_start, seg_end) = (seg.start, seg.end);
        for ev in seg.events.iter_mut() {
            // 手工拖过的画中画不动（与贴纸同一条规则）
            if ev.is_time_manual() {
                continue;
            }
            let Event::Camera { time, end, zoom, .. } = ev else { continue };
            if zoom.trim() != PIP_LABEL {
                continue;
            }
            let (t0, t1) = (*time, end.unwrap_or(*time + 1.0));
            let start = snap_start(t0, subtitles).clamp(seg_start, seg_end);
            let mut end_at = snap_end(t1, subtitles).clamp(seg_start, seg_end);
            if end_at - start < MIN_PIP_SECS {
                end_at = stretch_end(end_at, subtitles).clamp(seg_start, seg_end);
            }
            if end_at - start < 0.1 {
                continue; // 段太短/字幕缺失，别把事件写成空的
            }
            if (start - t0).abs() > 1e-6 || (end_at - t1).abs() > 1e-6 {
                *time = (start * 100.0).round() / 100.0;
                *end = Some((end_at * 100.0).round() / 100.0);
                aligned += 1;
                tracing::info!(
                    "  · 画中画对齐字幕边界：{:.2}s → {:.2}s（原 {:.2}s → {:.2}s）",
                    start, end_at, t0, t1
                );
            }
        }
    }
    aligned
}

/// 起点吸附：包含 t 的那条字幕的开始；落在空隙里取之后第一条字幕的开始；都没有就用 t。
fn snap_start(t: f64, subs: &[Subtitle]) -> f64 {
    if let Some(s) = subs.iter().find(|s| s.start <= t && t <= s.end) {
        return s.start;
    }
    subs.iter().filter(|s| s.start > t).map(|s| s.start).next().unwrap_or(t)
}

/// 终点吸附：包含 t 的那条字幕的结束；落在空隙里取之前最后一条字幕的结束；都没有就用 t。
fn snap_end(t: f64, subs: &[Subtitle]) -> f64 {
    if let Some(s) = subs.iter().find(|s| s.start <= t && t <= s.end) {
        return s.end;
    }
    subs.iter().filter(|s| s.end < t).map(|s| s.end).next_back().unwrap_or(t)
}

/// 对齐后太短时往后借：一条条字幕的结束往后够，够 [`MIN_PIP_SECS`] 就停。
fn stretch_end(from: f64, subs: &[Subtitle]) -> f64 {
    let mut out = from;
    for e in subs.iter().map(|s| s.end).filter(|e| *e > from + 1e-6) {
        if out - from >= MIN_PIP_SECS {
            break;
        }
        out = e;
    }
    out
}

/// 画中画到点收工后，回镜头还要看得见的最短时长（秒）：比这更短就让下一条运镜接手，不补。
pub const MIN_RETURN_SECS: f64 = 0.5;

/// 画中画「到点收工」：`远` 在屏幕上的时长 = 它自己的 `end - time`。
///
/// 运镜是**状态机** —— 事件只写起点，景别一直保持到下一条运镜事件。这对 `远` 是灾难：
/// LLM 把一次两秒的"戏剧性停顿"写在两条运镜之间很宽的间隔里，画面就一直缩成画中画挂到
/// 下一条运镜（实测 clip-23 那条 2.5s 的画中画挂了 15.7 秒，看着很诡异）。
/// 别的景别的时长只是"多久推到位"，画中画却等于**它该待多久**，所以这里给它补一条
/// 回镜头的运镜事件：落在画中画之前那个镜头的**落点**（同景别、同不动点），整条轨道因此
/// 无缝，画中画到点就收。回镜头是硬切 —— 「远」进出本来就是硬切（见
/// [`crate::process::placement::transition_secs`]），四条求值路径都不需要新语义。
///
/// 补的是**真事件**（不是渲染侧特例）：模板 canvas / GSAP 两条出片路径、`engine.js` 预览、
/// 时间轴编辑器、贴纸避让看到的完全一致，用户也能在编辑器里直接改或删。幂等：补过之后再跑
/// 一次不会再补（第二遍时下一条运镜就落在画中画终点上，间隔为 0）。
/// 返回补出来的条数。
pub fn close_pip_shots(segments: &mut [Segment]) -> usize {
    /// 回镜头要复原的落点 = 画中画之前那个镜头的终点状态
    struct Shot {
        zoom: String,
        origin: String,
        anchor: Option<String>,
        /// 前一条镜头带不带手持摇摆：回镜头把它一并复原（景别不对时由守卫剥掉）
        handheld: bool,
    }

    // 全部运镜事件按**时间**排序（跨段）：求值器就是按时间排序再走的，所以"前一条镜头"和
    // "下一条运镜"都得按时间算 —— 段之间时间上可能重叠（LLM 按窗口生成），文件顺序不等于时间序。
    let mut order: Vec<(usize, usize, f64)> = Vec::new();
    for (si, seg) in segments.iter().enumerate() {
        for (ei, ev) in seg.events.iter().enumerate() {
            if let Event::Camera { time, .. } = ev {
                order.push((si, ei, *time));
            }
        }
    }
    order.sort_by(|a, b| a.2.partial_cmp(&b.2).unwrap_or(std::cmp::Ordering::Equal));

    // 第一遍只读：算出每条画中画要补的回镜头（(段, 事件下标, 新事件)），下标是插入位置
    let mut prev: Option<Shot> = None;
    let mut inserts: Vec<(usize, usize, Event)> = Vec::new();
    for (i, (si, ei, time)) in order.iter().enumerate() {
        let Event::Camera { end, zoom, origin, anchor, anchor_to, to_origin, handheld, .. } =
            &segments[*si].events[*ei]
        else {
            continue;
        };
        if zoom.trim() != PIP_LABEL {
            // 记下这个镜头的**落点**：写了 anchor_to/to_origin 就是终点那一侧
            prev = Some(Shot {
                zoom: zoom.clone(),
                origin: to_origin.clone().unwrap_or_else(|| origin.clone()),
                anchor: anchor_to.clone().or_else(|| anchor.clone()),
                handheld: *handheld,
            });
            continue;
        }
        // 收工时刻 = 画中画自己的终点（夹在段内，别把事件写到段外）
        let seg = &segments[*si];
        let ret_at = end.unwrap_or(*time).clamp(seg.start, seg.end);
        // 下一条运镜来得够晚才补回镜头；马上就来（< MIN_RETURN_SECS）就让它接手
        let horizon = order.get(i + 1).map_or(seg.end, |(_, _, t)| *t);
        if horizon - ret_at < MIN_RETURN_SECS {
            continue;
        }
        // end 只写到"本段内还剩多久"（与其它运镜事件一致，不跨段尾）：状态机本来就会一直
        // 保持到下一条运镜，`clamp_camera_end` 与贴纸避让都不看它
        let ret_end = ret_at + (horizon.min(seg.end) - ret_at).max(0.1);
        let restore = Event::Camera {
            time: ret_at,
            end: Some(ret_end),
            zoom: prev.as_ref().map_or_else(|| "中".to_string(), |s| s.zoom.clone()),
            origin: prev
                .as_ref()
                .map_or_else(|| super::PRISTINE_ORIGIN.to_string(), |s| s.origin.clone()),
            // 没有前一条就照 Step 1 的默认运镜：聚焦主播
            anchor: prev
                .as_ref()
                .and_then(|s| s.anchor.clone())
                .or_else(|| Some("liver".to_string())),
            anchor_to: None,
            to_origin: None,
            cut: false,
            // 回镜头复原的是「前一条那条镜头」，手持摇摆也一并复原（非近景由守卫剥掉）
            handheld: prev.as_ref().is_some_and(|s| s.handheld),
            manual: false,
        };
        tracing::info!(
            "  · 画中画到点收工：{:.2}s 起回镜头（持到 {:.2}s · {}）",
            ret_at,
            horizon,
            restore_zoom(&restore)
        );
        inserts.push((*si, *ei, restore));
    }
    if inserts.is_empty() {
        return 0;
    }

    // 第二遍：把回镜头插在各自画中画事件的**后面**（同段下标从大到小，避免插入错位）
    inserts.sort_by(|a, b| (b.0, b.1).cmp(&(a.0, a.1)));
    let inserted = inserts.len();
    for (si, ei, ev) in inserts {
        segments[si].events.insert(ei + 1, ev);
    }
    inserted
}

/// 只给日志用：取一下刚构造出来的回镜头景别。
fn restore_zoom(ev: &Event) -> &str {
    match ev {
        Event::Camera { zoom, .. } => zoom,
        _ => "",
    }
}

/// 手持摇摆（`handheld`）只允许挂在**近景**上（用户定的门槛：这是近景专用的附加变种）。
///
/// 兜底理由与画中画同一条：LLM 会写错景别，用户也会在编辑器里把景别改掉（改成「中」之后
/// `handheld` 还留在事件上），两边都拦不住，所以这里做确定性剔除。**只剥不补**：非近景的
/// `handheld` 一律改回 `false`；近景上没写就是没有这个效果（它是可选项，系统不替用户开）。
///
/// 放在 `cmd_process` Step 1 补默认运镜**之后**、画中画收工之后调用（回镜头也一起吃这道守卫）。
/// 幂等；返回被剥掉的条数。
pub fn cap_handheld_sway(segments: &mut [Segment]) -> usize {
    let mut stripped = 0usize;
    for seg in segments.iter_mut() {
        for ev in seg.events.iter_mut() {
            let Event::Camera { zoom, handheld, .. } = ev else { continue };
            if *handheld && zoom.trim() != CLOSE_LABEL {
                *handheld = false;
                stripped += 1;
            }
        }
    }
    stripped
}

// ── 冷开场 / 回忆回溯（第 5 批）──────────────────────────────────────
//
// 「爆点提前播一次」（冷开场）与「爆点后面重提」（回忆回溯）在本仓是**同一件事的两个落点**：
// 都是"把源时间轴上某一段，在成片时间轴上**再放一次**"—— 也就是 `[[edl.clips]]` 里
// `source_start`/`source_end` 与另一段**完全相同**。所以这一批**没有新原语**：
// 复用的是第 2 批（重放）已经打好、逐像素验过的多 layer 画面 + 分段音轨那套。

/// 冷开场在整片里**最多几次**（用户要求：一个切片只能有一次）。
///
/// 与 [`MAX_PIP_SHOTS`] 同一条理由 —— 冷开场是**开头那一下**的戏剧手段，
/// 每次窗口都来一段就等于没有开头。
pub const MAX_COLD_OPEN: usize = 1;

/// 回忆回溯在整片里**最多几次**。比冷开场松一档：它落在正片中间，可以多点。
pub const MAX_RECALL: usize = 2;

/// 冷开场的**最短素材时长**（秒）：比这更短的片段单独提到片头只是闪一下，不像"爆点"。
pub const MIN_COLD_OPEN_SECS: f64 = 1.0;

/// LLM 在 `Event::Camera` 上表达「这段值得当冷开场 / 值得在后面重提」用的 `anchor` 取值。
///
/// ⚠️ **为什么挂在 `anchor` 上而不是新事件类型**：`Event` 是 `deny`-less 的 tagged enum，
/// 加一个变体会牵动**四处求值器**（模板 canvas / GSAP / `engine.js` / Rust）与词汇表门禁；
/// 而这里要表达的只是**意图**、活不过 `process` 这一趟（下一节就见分晓）。
/// `anchor` 是现成的自由字符串字段，认不出的值本来就被 `resolve_camera_anchors` 忽略，
/// 所以老工程、老 LLM 输出零影响。
pub const INTENT_COLD_OPEN: &str = "cold_open";
/// 见 [`INTENT_COLD_OPEN`]。回忆回溯 = 在正片里再放一次。
pub const INTENT_RECALL: &str = "recall";

/// `EdlClip::kind` 的两个本批新取值（UI/统计用，**不参与任何推导**）。
pub const EDL_KIND_COLD_OPEN: &str = "cold_open";
pub const EDL_KIND_RECALL: &str = "recall";

/// 一次冷开场/回忆回溯的**落实结果**（`cmd_process` 拿它打日志、判"要不要落盘"）。
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct MaterializedEdl {
    /// 冷开场真的加了一条（配额内）
    pub cold_open: bool,
    /// 回忆回溯加了几条
    pub recalls: usize,
}

impl MaterializedEdl {
    /// 本趟有没有真的改 EDL（`false` ⇒ 调用方**一个字节都不许写**）。
    pub fn changed(&self) -> bool {
        self.cold_open || self.recalls > 0
    }
}

/// 句末判据：这条字幕是不是**说完整了**。
///
/// 依据 `docs/plan/editing-decision-layer.md` §5.2：**"不必上词级时间戳"** ——
/// 字幕自带 `text` + `start/end`，"这条是不是以句末标点结尾"就够了。
///
/// 认三种形态（都是**句末**，不是句中）：
/// - **句末标点**：`。！？…` 与它们的全角/半角混用（`!?` `.` 在中文 ASR 里也会出现在句末）；
/// - **省略号**：`…` / `...` —— "话没说完但停住了"，正是可以切的地方；
/// - ⚠️ **引号/括号收尾**：`"好。"` 这种**标点被引号包住**的形态，
///   所以先剥掉尾部的成对包裹符再看标点（不剥的话最常见的对白引号会被判成"没说完"）。
///
/// ⚠️ **逗号、顿号、分号一律不算**（`,` / `，` / `、` / `；` / `:`）——
/// 那些正是"话说一半"的标志，把切点放上去就是本守卫要防的事。
pub fn is_sentence_end(text: &str) -> bool {
    let t = text.trim_end();
    // 尾部包裹符可以叠（`"'…"'` 这种），循环剥干净再判标点。
    let mut core = t;
    loop {
        let stripped = core
            .trim_end()
            .trim_end_matches(['"', '\'', '”', '’', '」', '』', '）', ')', '】', '》', '〉']);
        if stripped.len() == core.len() {
            break;
        }
        core = stripped;
    }
    if core.is_empty() {
        return false;
    }
    // `...` 要整串认（尾字符是 `.` 时逐个剥会把它当句号，但那也是句末，等价）。
    core.ends_with(['。', '！', '？', '…', '!', '?', '.'])
}

/// 句末边界守卫（总方案 §2）：**切片/分段边界必须落在句末**，不再"一句话说到一半就切"。
///
/// 依据 HIVE 论文（[arXiv 2507.02790](https://arxiv.org/html/2507.02790v1)）：
/// *"ensure that clip boundaries do not interrupt ongoing dialogue"* ——
/// **去掉边界选择，Hook Rate 从 0.87 掉到 0.28**（掉 2/3）。
///
/// # 判据（两条取其一，都在**字幕**上判，不碰事件）
///
/// 1. **句末标点**：该时刻所在那条字幕以 [`is_sentence_end`] 为真；
/// 2. **足够长的空隙**：该时刻落在两条字幕之间且**间隙 > [`BOUNDARY_GAP_SECS`]** ——
///    没人说话时切，当然不会切断句子。
///
/// # 怎么改（**结构性约束**）
///
/// 只挪**段的 `start`/`end`**，且：
/// - 段是**首尾相接**的一条链（每个 `end` = 下一个 `start`），所以一次改动要**成对**做：
///   把边界从 `t` 挪到 `t'` ⇒ 前一段的 `end` 与后一段的 `start` 一起改成 `t'`；
/// - **单段工程 / 首段起点 / 末段终点一律不动**（整片就是个段时无处可挪，
///   首尾是片子的开头结尾，挪了就是剪掉片头片尾）；
/// - 按时间排序后处理，**不依赖文件顺序**（段之间时间会重叠 —— 见 `cap_pip_shots` 那条教训）；
/// - `manual` 段**不参与**（用户自己锁的边界，照 `cap_pip_shots` 的先例）。
///
/// # 落点怎么选
///
/// 在「原位置 ± [`BOUNDARY_SEARCH_SECS`]」这个窗口里找**最近的合格点**：
/// 优先句末字幕的**结束点**（那句话说完的位置），其次窗口内的长空隙中点。
/// **两个方向都找不到就原样不动** —— 宁可不切，也不把一个更差的点按上去。
///
/// 返回被挪动的边界个数（0 = 本来就都在句末）。日志由调用方报。
///
/// 需要**逐条明细**（记决策理由）用 [`align_segment_boundaries_detailed`]。
pub fn align_segment_boundaries(segments: &mut [Segment], subtitles: &[Subtitle]) -> usize {
    align_segment_boundaries_detailed(segments, subtitles).len()
}

/// 一次边界挪动的明细：`(原位置, 新位置, 新位置是不是句末标点)`。
///
/// 第三个字段给**决策理由**用：句末标点与"字幕收尾兜底"是两种不同的判据，
/// 理由文案要分得清（否则用户看到"落在句末"而那条字幕根本没有句末标点，会觉得系统在瞎说）。
pub type BoundaryMove = (f64, f64, bool);

/// 同 [`align_segment_boundaries`]，但**带回每一条挪动的明细**（总方案 §1 要拿它写理由）。
pub fn align_segment_boundaries_detailed(
    segments: &mut [Segment],
    subtitles: &[Subtitle],
) -> Vec<BoundaryMove> {
    if segments.len() < 2 || subtitles.is_empty() {
        return Vec::new();
    }
    // 按 start 排序拿"相邻"关系（段之间时间会重叠，文件顺序不算数）。
    // ⚠️ 记的是**原下标**：改的是 `segments[idx]` 本体，不是排序后的副本。
    let mut order: Vec<usize> = (0..segments.len()).collect();
    order.sort_by(|a, b| {
        segments[*a]
            .start
            .partial_cmp(&segments[*b].start)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    // 先把每条"共享边界"的候选算出来，再统一落盘 —— 边改边读会让后面的判据看到半成品。
    let mut moves: Vec<(usize, usize, BoundaryMove)> = Vec::new(); // (前段 idx, 后段 idx, 明细)
    for w in order.windows(2) {
        let (a, b) = (w[0], w[1]);
        // 两段必须真的首尾相接才谈得上"共享边界"；有缝/重叠就让它们保持原样。
        let shared = segments[a].end;
        if (shared - segments[b].start).abs() > 1e-6 {
            continue;
        }
        if boundary_is_clean(shared, subtitles) {
            continue;
        }
        let Some(better) = best_boundary_near(shared, subtitles) else {
            continue;
        };
        // 挪了之后得仍是合法段（不能把某一段吃掉）。
        if better <= segments[a].start + MIN_SEGMENT_SECS
            || better >= segments[b].end - MIN_SEGMENT_SECS
        {
            continue;
        }
        // 落点是不是"句末标点那句"的结尾 —— 决定理由文案（见 `BoundaryMove` 的说明）。
        let by_punct = subtitles
            .iter()
            .any(|s| (s.end - better).abs() < 1e-6 && is_sentence_end(&s.text));
        moves.push((a, b, (shared, better, by_punct)));
    }

    for (a, b, (_, t, _)) in &moves {
        segments[*a].end = *t;
        segments[*b].start = *t;
    }
    moves.into_iter().map(|(_, _, m)| m).collect()
}

/// 这个时刻算不算"切得干净"（见 [`align_segment_boundaries`] 的两条判据）。
///
/// `pub(crate)` 供诊断探针（`diagnose_boundaries`）读数；生产路径只有
/// [`align_segment_boundaries`] 一个入口。
pub(crate) fn boundary_is_clean(t: f64, subs: &[Subtitle]) -> bool {
    // ⚠️ **先判"落在空隙里"，再判"落在字幕内部"** —— 顺序不能反。
    // 字幕的**结束点**同时满足两个条件（`start <= t <= end`），先判包含就会把它
    // 当成"这条字幕内部"、进而要求句末标点；而它其实是**两句话之间的位置**。
    // 实测后果：真实 ASR 字幕没有终句标点 ⇒ 字幕结束点全被判成"不干净" ⇒
    // 兜底候选自己把自己滤光、守卫永远放弃（`boundary_falls_back_...` 那条测试钉住它）。
    // 判据用"严格在外"（`t >= s.end` 与 `t <= s.start`），让结束点归到空隙那一侧。
    let inside = subs.iter().any(|s| s.start < t && t < s.end);
    if inside {
        // 落在某条字幕**内部**（严格意义）：只有整条说完才算干净。
        let s = subs
            .iter()
            .find(|s| s.start < t && t < s.end)
            .expect("上面 any 已确认存在");
        return is_sentence_end(&s.text);
    }
    // 落在空隙 / 边界点上：够宽就行。算到"上一条说完"与"下一条开口"之间的距离。
    let prev_end = subs
        .iter()
        .filter(|s| s.end <= t)
        .map(|s| s.end)
        .next_back();
    let next_start = subs.iter().filter(|s| s.start >= t).map(|s| s.start).next();
    match (prev_end, next_start) {
        (Some(p), Some(n)) => (n - p) > BOUNDARY_GAP_SECS,
        // 片头之前 / 片尾之后：没有对话可切断。
        _ => true,
    }
}

/// 在 `t` 附近找最近的合格切点（两个方向都找，取更近的那个）。
///
/// # 两类候选（**上限是真实数据的现实，不是设计偏好**）
///
/// ① **句末字幕的结束点** —— 首选。天然满足"不切断句子"，落点是**那句话说完的瞬间**，
///    正是剪辑上该切的位置。
/// ② ⚠️ **任意字幕的结束点** —— 兜底。**这一条不是可选的**：实测本仓可得的字幕样本里
///    句末标点占比 **0/13 = 0%**（ASR 输出与手写 demo 都不带终句标点），
///    只认 ① 的话守卫在真实数据上**永远找不到点、永远放弃** —— 那等于没做这一项。
///    字幕结束点弱于句末标点，但仍**强于"句中任意位置"**：它是 ASR 判断的"这一口气说完了"，
///    而我们能拿到的信号里没有比它更细的（规格 §5.2 明说"不必上词级时间戳"）。
///    ⇒ 保险起见只做**同一条字幕内部的收尾**，不跨到别的字幕上。
///
/// 两类一起按"离 `t` 的绝对距离"排序，最近的胜出；`t` 自己若已干净
/// （[`boundary_is_clean`]）就不会走到这里。
pub(crate) fn best_boundary_near(t: f64, subs: &[Subtitle]) -> Option<f64> {
    // ① 句末标点优先（距离相同时它排前面 —— 排序稳定，先入者先出）。
    let mut cands: Vec<(f64, u8)> = subs
        .iter()
        .filter(|s| is_sentence_end(&s.text))
        .map(|s| (s.end, 0u8))
        .filter(|(e, _)| (e - t).abs() <= BOUNDARY_SEARCH_SECS)
        .filter(|(e, _)| boundary_is_clean(*e, subs))
        .collect();
    // ② 兜底：任意字幕的结束点（见上面第 ② 条的理由）。
    cands.extend(
        subs.iter()
            .map(|s| (s.end, 1u8))
            .filter(|(e, _)| (e - t).abs() <= BOUNDARY_SEARCH_SECS)
            .filter(|(e, _)| boundary_is_clean(*e, subs)),
    );
    // 按距离排；距离相同看类别（0 = 句末标点，更优）。
    cands.sort_by(|a, b| {
        (a.0 - t)
            .abs()
            .partial_cmp(&(b.0 - t).abs())
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.1.cmp(&b.1))
    });
    cands.first().map(|(e, _)| *e)
}

/// 边界前后找替代点的搜索半径（秒）。
///
/// 4.0s 的取舍：太小 ⇒ 大量边界"找不到就放弃"，守卫形同虚设；
/// 太大 ⇒ 挪动幅度盖过一个语气停顿，看着像"切错地方了"。真实字幕中位时长 2.64s，
/// 4.0s ≈ 一句话多一点，正好够到"上一句说完 / 下一句说完"。
pub const BOUNDARY_SEARCH_SECS: f64 = 4.0;

/// 多大的字幕间隙算"没人说话"（秒）。窄于它就不是一句说完了，只是换气。
///
/// ⚠️ 与 §2 规格给的 `0.35s` **不同**：那个数是"判定句子结束"用的（很短就够了），
/// 这里还要求它宽到**能承载一个切点** —— 0.35s 的缝切下去，前后两个字幕几乎连着，
/// 观感与"句中切"无异。取与 `STICKER_SNAP_MAX_SECS`(2.0s) 同一个量级。见保守决策记录。
pub const BOUNDARY_GAP_SECS: f64 = 1.0;

/// 挪边界之后每段至少要剩下这么长（秒），否则不挪 —— 免得把一段吃到只剩一帧。
pub const MIN_SEGMENT_SECS: f64 = 0.5;

/// **决策理由的类别**（总方案 §1）。写进 `polish.toml` 的 `[[decisions]]` 的 `kind`。
///
/// ⚠️ **这些串是稳定标识**：界面按它分组/着色、判据按它 grep。
/// 改文案改 [`Decision::text`]，**别改这里**（改了就等于换了一种决策）。
pub mod decision_kind {
    /// 分段边界被句末守卫挪动（§2 的产物）。
    pub const BOUNDARY_SENTENCE_END: &str = "boundary_sentence_end";
    /// 分段边界本来就切得干净、没动。
    pub const BOUNDARY_ALREADY_CLEAN: &str = "boundary_already_clean";
    /// 分段边界想挪但找不到合格点，**保持原样**（§2 的弃权）。
    pub const BOUNDARY_KEPT_AS_IS: &str = "boundary_kept_as_is";
    /// 给没有运镜的段补了默认运镜。
    pub const CAMERA_DEFAULT_ADDED: &str = "camera_default_added";
    /// 画中画超出配额、被降级成中景。
    pub const PIP_DEMOTED: &str = "pip_demoted";
    /// 元素（贴纸/文字）时长被规矩调整。
    pub const ELEMENT_SPAN_FITTED: &str = "element_span_fitted";
    /// 片头淡入被补上。
    pub const INTRO_FADE_SEEDED: &str = "intro_fade_seeded";
}

/// 把 §2 的一次边界挪动翻成一条**人话理由**。
///
/// ⚠️ **两种落点要分开说**：落在句末标点上 vs 退到字幕收尾 —— 后者那条字幕根本没有句末标点，
/// 一律写"落在句末"就是骗人（用户点开一看会发现那句话没说完，从此不信整个面板）。
pub fn boundary_move_decision(m: &BoundaryMove) -> Decision {
    let (from, to, by_punct) = *m;
    let text = if by_punct {
        "切点原来落在一句话中间；已挪到上一句说完的位置（句末标点）"
    } else {
        "切点原来落在一句话中间；附近没有句末标点，已挪到最近一条字幕的结束处"
    };
    Decision {
        kind: decision_kind::BOUNDARY_SENTENCE_END.to_string(),
        at: Some(to),
        start: None,
        end: None,
        text: text.to_string(),
        detail: Some(format!("{from:.2}s → {to:.2}s")),
    }
}

/// **按 `kind` 去重**（后面的胜出）—— 顺延来的旧理由与"本趟新产生的"会撞车。
///
/// 判据只有 `kind` 一项、**不比位置**：这些类别每个至多描述一件事
/// （片头淡入整片一条；边界类每处一条，而"同一处"在顺延与新增里必然同名同位置）。
/// ⚠️ 边界类若同一 `kind` 有多处（多个切点都挪过），这条 `retain` 会把它们**误删** ——
/// 所以这里按 `(kind, at)` 判重，`at` 为 `None` 的（整片级）按 `kind` 判重。
pub fn dedup_decisions(list: &mut Vec<Decision>) {
    let mut seen: Vec<(String, Option<i64>)> = Vec::new();
    let mut out: Vec<Decision> = Vec::with_capacity(list.len());
    // 从后往前：**本趟新产生的在尾部**，遇重复先保留它（后面的胜出）。
    for d in list.drain(..).rev() {
        // 位置量化到毫秒，避免 f64 相等判定抖动。
        let key = (d.kind.clone(), d.at.map(|t| (t * 1000.0).round() as i64));
        if seen.contains(&key) {
            continue;
        }
        seen.push(key);
        out.push(d);
    }
    out.reverse();
    *list = out;
}

/// 把 LLM 的「冷开场 / 回忆回溯」意图**落实成 `edl.clips`**，并兜住配额。
///
/// # 照抄的先例
///
/// 形状与 [`cap_pip_shots`] 完全一致，包括它存在的**全部理由**：
/// **LLM 是按段窗口生成的，数不出"整片几次"** —— 所以"最多一次"这条配额只能在这里兜。
/// LLM 只负责在它看得见的那一段上写下意图（`anchor = "cold_open"` / `"recall"`），
/// 删掉/降级的决定权在这里。
///
/// # 触发条件（**要紧的一条**）
///
/// ⚠️ 有 EDL 时 `mute_ranges` **整个被忽略**（`ProjectFile::resolve_edl`、规格 §2.3），
/// 所以"生成 EDL"= **把老工程的时间线无声地换掉**。因此这里有一条硬前提：
///
/// > **`existing` 非空时只增不改**；`existing` 为空时**只看本轮意图**：
/// > 一支意图都没有 ⇒ 返回 [`MaterializedEdl::default`]（`changed() == false`）
/// > ⇒ 调用方**不写 `polish.toml`** ⇒ `edl.clips` 保持为空 ⇒ 老工程逐字节零回归（R4）。
///
/// 用户在前端手拖过的 `edl.clips` 一律**原样保留**（只在它前面插冷开场、不改它一根毫毛）。
///
/// # 配额与幂等
///
/// - 冷开场：**整片最多 [`MAX_COLD_OPEN`] 条**。候选按时间序排，只留**最早的**那条
///   （与 `cap_pip_shots` 留最早一条「远」同一条规则）；其余**只清意图、不加段**。
/// - 回忆回溯：最多 [`MAX_RECALL`] 条，同样按时间序取最早的那几条。
/// - **幂等**：落实之后**意图被就地清掉**（`anchor` 置 `None`）—— 这是"意图是**一次性
///   触发条件**、不是持续状态"的落点。再跑一次没有任何候补、也没有新意图 ⇒ 什么都不加。
///   实测必要性：`process` 会被预览与出片**隐式重跑**，靠"看 EDL 里有没有这一段"去判重
///   在用户删掉重放段之后就会把它复活（`intro_fade_seeded` 那条教训一模一样的形状）。
///
/// # 手工事件
///
/// 带 `manual: true` 的运镜**不参与**（用户自己摆的事件不被后处理改写，见
/// `crates/yekitrim-polish/src/process/AGENTS.md`）—— 也就意味着**用户手工加的重放段
/// 不会被本函数删掉**，它只在 `existing` 里、原样保留。
///
/// # 时间取值
///
/// 重放段的源区间 = 该意图所属**意图窗口**（该段上第一个意图事件的 `time` 到其 `end`），
/// 夹到 `[0, total_duration]`。冷开场插在**数组最前**（成片第一段）；
/// 回忆回溯插在**它所属那一段之后**（在成片里的对应位置重提一次）。
///
/// 返回落实结果；`changed()` 为假时调用方不许动 `polish.toml`。
pub fn materialize_edl_intents(
    segments: &mut [Segment],
    edl: &mut Vec<EdlClip>,
    total_duration: f64,
) -> MaterializedEdl {
    // 1) 收集全部候选（按时间序）：要能分辨"哪条最早" —— 与 `cap_pip_shots` 同一套做法。
    let mut intents: Vec<(usize, Intent, f64, f64)> = Vec::new();
    for (si, seg) in segments.iter().enumerate() {
        for ev in seg.events.iter() {
            // 用户手工摆的运镜不参与（它的事件、它的意图都由用户说了算）
            if ev.is_time_manual() {
                continue;
            }
            let Some(intent) = anchor_intent(ev) else { continue };
            let time = ev.get_time();
            let end = ev.end().unwrap_or(time);
            intents.push((si, intent, time, end));
        }
    }
    intents.sort_by(|a, b| a.2.partial_cmp(&b.2).unwrap_or(std::cmp::Ordering::Equal));

    // 2) 按配额挑出活下来的：冷开场一条、回忆回溯 N 条（都是"最早优先"）。
    //    没活下来的只**清意图**，不往 EDL 里加东西。
    let mut live: Vec<(usize, Intent, f64, f64)> = Vec::new();
    let (mut cold_left, mut recall_left) = (MAX_COLD_OPEN, MAX_RECALL);
    for cand in intents.iter().copied() {
        match cand.1 {
            Intent::ColdOpen if cold_left > 0 => {
                cold_left -= 1;
                live.push(cand);
            }
            Intent::Recall if recall_left > 0 => {
                recall_left -= 1;
                live.push(cand);
            }
            _ => {
                tracing::info!(
                    "  ⚠ {}太密（{:.1}s · 整片只允许 {} 条）：本段不再重复播放",
                    cand.1.label(),
                    cand.2,
                    cand.1.quota()
                );
            }
        }
    }

    // 3) **清掉全部意图**（活的、死的都清）—— 幂等就靠这一步。
    //    清的是"我这一趟看到的那几条"的意图；换了 kind 的 anchor 不在这里。
    for (si, seg) in segments.iter_mut().enumerate() {
        for ev in seg.events.iter_mut() {
            if ev.is_time_manual() {
                continue;
            }
            if anchor_intent(ev).is_some() {
                clear_anchor(ev);
                let _ = si;
            }
        }
    }

    if live.is_empty() {
        return MaterializedEdl::default();
    }

    // 4) 落实。新段一律 `speed = 1.0`（本批只做"再放一次"，变速是第 4 批的独立维度）、
    //    `source_kind`/`asset_id`/`transition_in` 留 `None`（= 本条片段、硬切）。
    let mut result = MaterializedEdl::default();
    // 冷开场：**插在数组最前**。数组顺序 = 成片播放顺序（§2.1），所以插最前就是"提前播"。
    let colds: Vec<_> = live.iter().filter(|c| c.1 == Intent::ColdOpen).collect();
    let (mut cold_clip, mut recall_clips) = (None, Vec::new());
    for (_, _, time, end) in colds.into_iter().copied() {
        if let Some((s, e)) = clamp_span(time, end, total_duration) {
            cold_clip = Some(EdlClip {
                source_start: s,
                source_end: e,
                speed: 1.0,
                kind: Some(EDL_KIND_COLD_OPEN.into()),
                label: Some("冷开场".into()),
                source_kind: None,
                asset_id: None,
                transition_in: None,
            });
        }
    }
    for (_, _, time, end) in live.iter().filter(|c| c.1 == Intent::Recall).copied() {
        if let Some((s, e)) = clamp_span(time, end, total_duration) {
            recall_clips.push(EdlClip {
                source_start: s,
                source_end: e,
                speed: 1.0,
                kind: Some(EDL_KIND_RECALL.into()),
                label: Some("回忆回溯".into()),
                source_kind: None,
                asset_id: None,
                transition_in: None,
            });
        }
    }

    if let Some(clip) = cold_clip {
        // 冷开场同理：**插在数组最前**，原 EDL 一个字节不动地跟在后面。
        edl.insert(0, clip);
        result.cold_open = true;
    }
    for clip in recall_clips {
        // 回忆回溯插在**末尾**：正片播完再提一次，语义上不需要"落在某一处"的精确性，
        // 而"插在中间"要先把意图映射回"它落在成片第几段"—— 那是第 2 份推导，本批不做。
        edl.push(clip);
        result.recalls += 1;
    }
    result
}

/// 一个候选意图的两种档。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Intent {
    ColdOpen,
    Recall,
}

impl Intent {
    fn label(self) -> &'static str {
        match self {
            Intent::ColdOpen => "冷开场",
            Intent::Recall => "回忆回溯",
        }
    }
    fn quota(self) -> usize {
        match self {
            Intent::ColdOpen => MAX_COLD_OPEN,
            Intent::Recall => MAX_RECALL,
        }
    }
}

/// 这条事件有没有表达冷开场/回忆回溯意图（读 `anchor`）。
fn anchor_intent(ev: &Event) -> Option<Intent> {
    let Event::Camera { anchor, .. } = ev else { return None };
    match anchor.as_deref().map(str::trim) {
        Some(INTENT_COLD_OPEN) => Some(Intent::ColdOpen),
        Some(INTENT_RECALL) => Some(Intent::Recall),
        _ => None,
    }
}

/// 清掉意图（只清这两个值 —— 别的 anchor 不碰）。
fn clear_anchor(ev: &mut Event) {
    if let Event::Camera { anchor, .. } = ev
        && matches!(anchor.as_deref().map(str::trim), Some(INTENT_COLD_OPEN) | Some(INTENT_RECALL))
    {
        *anchor = None;
    }
}

/// 把 `[time, end]` 夹进 `[0, total_duration]`，并保证长于 [`MIN_COLD_OPEN_SECS`]。
/// 推不出合法区间（时长未知 / 太短 / 起止相等）⇒ `None`（宁缺毋滥，别往 EDL 里塞空段
/// —— 空段会被 `Edl::resolve_clips` 跳过，但它会**留在 `polish.toml` 里**，看着像有东西）。
fn clamp_span(time: f64, end: f64, total_duration: f64) -> Option<(f64, f64)> {
    if !total_duration.is_finite() || total_duration <= 0.0 {
        return None;
    }
    let s = time.max(0.0);
    let e = end.min(total_duration);
    if !(e - s >= MIN_COLD_OPEN_SECS) {
        return None;
    }
    Some((s, e))
}

/// 运镜倍率的**旧默认值**（都只可能来自内建 Default，UI 从来没有入口改它们——
/// 只能手改 polish.toml，所以「恰好等于旧默认」等价于「没指定」，可以安全迁移）。
/// 见 `migrate_camera_zoom`。
/// 画中画 0.7 之前是 0.55；中景 1.15 → 1.3、近景 1.3 → 1.5（放大档位改过一次）。
pub const LEGACY_ZOOM_WIDE: f64 = 0.7;
pub const LEGACY_ZOOM_MID: f64 = 1.15;
pub const LEGACY_ZOOM_CLOSE: f64 = 1.3;

/// 贴纸对齐最近字幕的**最大距离**（秒）：只有起点离某条字幕这么近才去对齐；
/// 离得远的（纯视觉反应、没有台词可依附）保持原位。
pub const STICKER_SNAP_MAX_SECS: f64 = 2.0;

/// 元素（贴纸/文字）的**最短在屏时长**（秒）。
///
/// 用户 2026-10-11 口径：「需要最小时间，不足就**延续到超过最小时间的且覆盖下一个字幕的结尾**」。
///
/// 为什么需要它 —— 现网实测（73 个片段 / 909 条字幕）字幕时长 **min 0.05s、max 16.30s**，
/// 跨度 326×；`<0.5s` 的占 2.6%。ASR 会把一口气重复的音节切成 0.05s 的连续碎片
/// （真实样本：`21.92→21.97→22.02` 三条，每条都是「凯，凯，凯…」），
/// 元素吸到这种碎片上只是**闪一下**，等于没放。
pub const ELEMENT_MIN_SECS: f64 = 1.2;

/// 元素（贴纸/文字）的**最长在屏时长**（秒）。
///
/// 用户同一条口径的另半边：「同时也有最长时间，不然前面时零碎的字幕，
/// **后面满足时一句话很长**就显得很奇怪」—— 前面被最短时长撑开、后面又很长，反差更大。
/// 实测 >5s 的字幕占 9.6%（最长 16.30s），所以这一档确实够得着。
///
/// ⚠️ **只截元素、不碰字幕**（用户明确：「不对字幕处理，只对其他元素处理」）：
/// 元素是贴图/贴字，截短不丢内容；字幕截短会吃字，那是另一件事。
pub const ELEMENT_MAX_SECS: f64 = 5.0;

/// 把一条元素的时间窗口**按最短/最长时长规矩**规整，返回规整后的 `end`。
///
/// 语义（用户 2026-10-11 拍板，三条一起才是完整规则）：
///
/// 1. **起点不动** —— 元素挂在哪句话上是 LLM 的意图，不因为时长不够就往前挪。
/// 2. **不足 [`ELEMENT_MIN_SECS`] 就往后「吞」字幕**：end 沿字幕序列**逐条**往后延，
///    一直延到「已经盖满 ≥ [`ELEMENT_MIN_SECS`]，**且**落在某条字幕的结尾上」为止。
///    这就是用户说的「延续到超过最小时间的且覆盖下一个字幕的结尾」——
///    不只看够不够长，还要**停在字幕边界**，否则元素会断在半句话中间。
/// 3. **超过 [`ELEMENT_MAX_SECS`] 就硬截到最长**（不丢内容，因为截的是元素不是字幕）。
///
/// 「没有更多字幕可吞、仍然不足最短」时**接受现状**（不凭空造时长：字幕之间的静音段
/// 是真实留白，元素硬撑到那儿只会盖住没人说话的画面）。
///
/// 只有 `end` 会变（`start` 由调用方决定），返回的是**四舍五入到厘秒**之后的值，
/// 与其余时间字段同一精度口径。
/// 元素往后「吞」字幕时**允许跨过的最大空隙**（秒）。
///
/// 字幕之间是有**真实静音空隙**的（ASR 切出来的碎片本来就不连续）。实测真实碎片链：
/// `…→22.67`，然后下一条从 `23.20` 起 —— 中间空了 **3.13s**。
/// 不带这道闸的话，"吞到够长"会一头扎进静音段：元素从 21.92 一直撑到 25.80
/// （累计 3.88s），**在没人说话的画面上悬着 3 秒**。
/// 那正是用户要避免的"很奇怪"，只是从另一头冒出来。
///
/// 所以：下一段字幕的**起点**离当前 end 超过这个值就不再往后吞，**接受现状**
/// （哪怕还没到 [`ELEMENT_MIN_SECS`]）—— 短一点也比盖住静音段强。
pub const ELEMENT_MAX_GAP_SECS: f64 = 0.5;

/// 把一条元素的时间窗口**按最短/最长时长规矩**规整，返回规整后的 `end`。
///
/// 语义（用户 2026-10-11 拍板，四条一起才是完整规则）：
///
/// 1. **起点不动** —— 元素挂在哪句话上是 LLM 的意图，不因为时长不够就往前挪。
/// 2. **不足 [`ELEMENT_MIN_SECS`] 就往后「吞」字幕**：end 沿字幕序列**逐条**往后延，
///    一直延到「已经盖满 ≥ [`ELEMENT_MIN_SECS`]，**且**落在某条字幕的结尾上」为止。
///    这就是用户说的「延续到超过最小时间的且覆盖下一个字幕的结尾」——
///    不只看够不够长，还要**停在字幕边界**，否则元素会断在半句话中间。
/// 3. **不跨静音空隙**：下一段字幕的起点离当前 end 超过 [`ELEMENT_MAX_GAP_SECS`] 就停下
///    （见该常量的说明 —— 否则元素会在没人说话的画面上一悬三秒）。
/// 4. **超过 [`ELEMENT_MAX_SECS`] 就硬截到最长**（不丢内容，因为截的是元素不是字幕）。
///
/// 「没有更多字幕可吞、仍然不足最短」时**接受现状**（不凭空造时长）。
///
/// 只有 `end` 会变（`start` 由调用方决定），返回的是**四舍五入到厘秒**之后的值，
/// 与其余时间字段同一精度口径。
fn fit_element_span(start: f64, end: f64, subs: &[Subtitle]) -> f64 {
    let mut e = end;
    // ②/③ 不足最短：顺着字幕往后吞，直到「够长 且 停在字幕结尾」，但不跨静音空隙
    if e - start < ELEMENT_MIN_SECS {
        // ⚠️ 按**起点**排序的候选（`subs` 未必有序：段之间时间会重叠，调用方传进来的
        // 可能是拼接结果）。只考虑起点在当前位置之后的。
        let mut cands: Vec<&Subtitle> = subs.iter().filter(|s| s.start > e - 1e-6).collect();
        cands.sort_by(|a, b| a.start.partial_cmp(&b.start).unwrap_or(std::cmp::Ordering::Equal));
        for s in cands {
            // 空隙太宽 ⇒ 停下（接受现状，别在静音段上悬着）
            if s.start - e > ELEMENT_MAX_GAP_SECS {
                break;
            }
            e = s.end;
            if e - start >= ELEMENT_MIN_SECS {
                break;
            }
        }
    }
    // ④ 超过最长：硬截（截的是元素，不丢内容）
    if e - start > ELEMENT_MAX_SECS {
        e = start + ELEMENT_MAX_SECS;
    }
    (e * 100.0).round() / 100.0
}

/// 表情贴纸对齐**最近的字幕**（用户要求：「表情贴纸也需要对齐最近的字幕」）。
///
/// 起点吸到那条字幕的开始、终点吸到它的结束 —— 贴纸正好盖住它反应的那句话，头尾都落在
/// 字幕边界上（与画中画的 `align_pip_shots` 同一套思路）。离最近字幕超过
/// [`STICKER_SNAP_MAX_SECS`] 的、以及短得没法盖（< 0.2s）的字幕都不动。
///
/// 吸完之后再过一道 [`fit_element_span`] 的最短/最长时长规矩（2026-10-11 新增）：
/// 吸到 0.05s 碎片上的会被往后撑到盖满 ≥ [`ELEMENT_MIN_SECS`]，吸到长句上的被截到
/// ≤ [`ELEMENT_MAX_SECS`]。
///
/// 只改 `time`/`end`；排在 `process` Step 2c（尾帧自适应）之前，2c 还能往后接一句。
pub fn align_stickers_to_subtitles(segments: &mut [Segment], subtitles: &[Subtitle]) -> usize {
    let mut aligned = 0usize;
    for seg in segments.iter_mut() {
        for ev in seg.events.iter_mut() {
            // 手工定过时间的贴纸不动（面板上「跟随字幕」清掉标记后才恢复自动吸附）
            if ev.is_time_manual() {
                continue;
            }
            let Event::Sticker { time, end, .. } = ev else { continue };
            let Some(sub) = nearest_subtitle(*time, subtitles) else { continue };
            let dist = dist_to(*time, sub);
            if dist > STICKER_SNAP_MAX_SECS {
                continue;
            }
            // ⚠️ **这里曾经有一道 `sub 短于 0.2s 就跳过` 的守卫**（2026-10-11 删）：
            // 它是"没有补救手段"年代的产物 —— 那时吸到碎字幕上只能闪一下，所以干脆不吸。
            // 现在 [`fit_element_span`] 就是来治这件事的（往后吞到 ≥ 最短时长），
            // 留着那道守卫会把**最该被治的碎片**（0.05s，现网占 2.6%）全部放过。
            let len = sub.end - sub.start;
            let old_end = end.unwrap_or(*time + len);
            let start = (sub.start * 100.0).round() / 100.0;
            let fitted = fit_element_span(start, sub.end, subtitles);
            if (*time - start).abs() < 1e-6 && (old_end - fitted).abs() < 1e-6 {
                continue;
            }
            *time = start;
            *end = Some(fitted);
            aligned += 1;
            tracing::info!(
                "  · 贴纸对齐字幕：{:.2}s → {:.2}s（原 {:.2}s → {:.2}s{}{}）",
                *time,
                fitted,
                sub.start,
                old_end,
                if fitted > sub.end + 1e-6 { "，不足最短往后吞" } else { "" },
                if fitted + 1e-6 < sub.end { "，超最长已截" } else { "" }
            );
        }
    }
    aligned
}

/// 离 t 最近的字幕：在字幕里就是它；在空隙里取两端距离更近的那条。
fn nearest_subtitle(t: f64, subs: &[Subtitle]) -> Option<&Subtitle> {
    subs.iter()
        .min_by(|a, b| dist_to(t, a).partial_cmp(&dist_to(t, b)).unwrap_or(std::cmp::Ordering::Equal))
}

/// 点到字幕区间的距离（落在区间内为 0）。
fn dist_to(t: f64, s: &Subtitle) -> f64 {
    if t < s.start {
        s.start - t
    } else if t > s.end {
        t - s.end
    } else {
        0.0
    }
}

/// 画中画档位的档位名（与 `placement::WIDE_LABEL` 同源，别各写一份字面量）。
const PIP_LABEL: &str = crate::process::placement::WIDE_LABEL;

/// 近景档位的档位名（与 `placement::CLOSE_LABEL` 同源）。
const CLOSE_LABEL: &str = crate::process::placement::CLOSE_LABEL;

fn normalize_event_end(event: &mut Event) {
    if event.end().is_none() {
        // 音效缺 `end` 是「放完文件为止」的合法语义，不补
        if matches!(event, Event::Sfx { .. }) {
            return;
        }
        event.set_end(Some(event.default_end()));
    }

    let type_name = event_type_name(event);
    if type_name.is_empty() {
        return; // sticker / camera / overlay：只补缺省，不夹跨度
    }
    let cap = transient_duration_cap(type_name);
    let (time, end) = (event.get_time(), event.end().unwrap_or_else(|| event.default_end()));
    let span = (end - time).clamp(0.1, cap);
    event.set_end(Some(time + span));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn blur(end: Option<f64>) -> Event {
        Event::Blur {
            time: 0.0,
            end,
            anchor: None,
            sfx: None,
            sfx_volume: None,
        }
    }

    #[test]
    fn blur_is_capped_to_transient() {
        let mut segs = vec![Segment {
            start: 0.0,
            end: 10.0,
            fx: "high".into(),
            note: String::new(),
            events: vec![blur(Some(2.5))],
            extra: Default::default(),
        }];
        normalize_event_ends(&mut segs);
        let Event::Blur { end, .. } = &segs[0].events[0] else {
            panic!("expected blur");
        };
        assert_eq!(end.unwrap(), 0.4);
    }

    #[test]
    fn blur_default_is_applied() {
        let mut segs = vec![Segment {
            start: 0.0,
            end: 10.0,
            fx: "high".into(),
            note: String::new(),
            events: vec![blur(None)],
            extra: Default::default(),
        }];
        normalize_event_ends(&mut segs);
        let Event::Blur { end, .. } = &segs[0].events[0] else {
            panic!("expected blur");
        };
        assert_eq!(end.unwrap(), 0.3);
    }

    #[test]
    fn sticker_and_camera_spans_untouched() {
        let mut segs = vec![Segment {
            start: 0.0,
            end: 10.0,
            fx: "high".into(),
            note: String::new(),
            events: vec![
                Event::Sticker {
                    time: 1.0,
                    end: Some(6.0),
                    keyword: "laugh".into(),
                    sticker_tag: None,
                    name: None,
                    source: None,
                    x: "auto".into(),
                    y: "auto".into(),
                    sfx: None,
                    sfx_volume: None,
                    manual: false,
                },
                Event::Camera {
                    time: 0.0,
                    end: Some(10.0),
                    zoom: "中".into(),
                    origin: "50% 50%".into(),
                    anchor: None,
                    anchor_to: None,
                    to_origin: None, cut: false,
                    handheld: false,
                    manual: false,
                },
            ],
            extra: Default::default(),
        }];
        normalize_event_ends(&mut segs);
        match &segs[0].events[0] {
            Event::Sticker { time, end, .. } => {
                assert_eq!((*time, *end), (1.0, Some(6.0)), "贴纸/运镜只补缺省，不夹跨度");
            }
            _ => panic!("expected sticker"),
        }
    }

    #[test]
    fn missing_ends_are_filled_by_type() {
        // 缺 end 的事件补默认：贴纸 2.0s / 运镜 1.0s / 覆盖层 1.5s；音效不补（= 文件长度）
        let mut segs = vec![Segment {
            start: 0.0,
            end: 30.0,
            fx: "high".into(),
            note: String::new(),
            events: vec![
                Event::Sticker {
                    time: 1.0,
                    end: None,
                    keyword: "laugh".into(),
                    sticker_tag: None,
                    name: None,
                    source: None,
                    x: "auto".into(),
                    y: "auto".into(),
                    sfx: None,
                    sfx_volume: None,
                    manual: false,
                },
                Event::Camera {
                    time: 4.0,
                    end: None,
                    zoom: "中".into(),
                    origin: "50% 50%".into(),
                    anchor: None,
                    anchor_to: None,
                    to_origin: None,
                    cut: false,
                    handheld: false,
                    manual: false,
                },
                Event::Overlay {
                    time: 8.0,
                    end: None,
                    overlay_type: "color".into(),
                    color: None,
                    opacity: None,
                    colors: None,
                    shape: None,
                    angle: None,
                    sfx: None,
                    sfx_volume: None,
                    anchor: None,
                },
                Event::Sfx { time: 12.0, name: "pop-notice".into(), volume: None, end: None },
            ],
            extra: Default::default(),
        }];
        normalize_event_ends(&mut segs);
        let ends: Vec<Option<f64>> = segs[0].events.iter().map(|e| e.end()).collect();
        assert_eq!(ends, vec![Some(3.0), Some(5.0), Some(9.5), None]);
    }

    // ── 画中画降频守卫 ──────────────────────────────────────────────
    fn cam(time: f64, zoom: &str) -> Event {
        cam_h(time, zoom, false)
    }

    /// 带手持摇摆的运镜（`handheld` 只在近景上合法，测试里自行组合景别来验守卫）。
    fn cam_h(time: f64, zoom: &str, handheld: bool) -> Event {
        Event::Camera {
            time,
            end: Some(time + 2.0),
            zoom: zoom.into(),
            origin: "50% 50%".into(),
            anchor: None,
            anchor_to: None,
            to_origin: None,
            cut: false,
            handheld,
            manual: false,
        }
    }

    fn segs_of(times_zooms: &[(f64, &str)]) -> Vec<Segment> {
        // 一条段里放全部事件即可（守卫按时间排序遍历，跟段边界无关）
        vec![Segment {
            start: 0.0,
            end: 600.0,
            fx: "med".into(),
            note: String::new(),
            events: times_zooms.iter().map(|(t, z)| cam(*t, z)).collect(),
            extra: Default::default(),
        }]
    }

    fn zooms(segs: &[Segment]) -> Vec<String> {
        segs.iter()
            .flat_map(|s| s.events.iter())
            .filter_map(|e| match e {
                Event::Camera { zoom, .. } => Some(zoom.clone()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn pip_only_one_per_clip() {
        // 三条「远」→ 只留最早那条，其余降级（用户要求：一个切片只能有一次）
        let mut segs = segs_of(&[(0.0, "远"), (100.0, "远"), (200.0, "远")]);
        assert_eq!(cap_pip_shots(&mut segs), 2);
        assert_eq!(zooms(&segs), vec!["远", "中", "中"]);
        assert_eq!(cap_pip_shots(&mut segs), 0, "幂等：再跑一次不再动");

        // 中间夹一条「中」也一样 —— 配额按整片算，不是只看相邻
        let mut segs = segs_of(&[(0.0, "远"), (5.0, "中"), (70.0, "远")]);
        assert_eq!(cap_pip_shots(&mut segs), 1);
        assert_eq!(zooms(&segs), vec!["远", "中", "中"]);

        // 连着两条 → 后一条降级（同一条配额规则覆盖）
        let mut segs = segs_of(&[(0.0, "远"), (5.0, "远")]);
        assert_eq!(cap_pip_shots(&mut segs), 1);
        assert_eq!(zooms(&segs), vec!["远", "中"]);
    }

    // ── 画中画到点收工 ──────────────────────────────────────────────
    /// 全部运镜事件：(起点, 跨度, 景别, origin, anchor)
    #[allow(clippy::type_complexity)]
    fn cam_spans(segs: &[Segment]) -> Vec<(f64, f64, String, String, Option<String>)> {
        segs.iter()
            .flat_map(|s| s.events.iter())
            .filter_map(|e| match e {
                Event::Camera { time, end, zoom, origin, anchor, .. } => Some((
                    *time,
                    end.map(|e| e - *time).unwrap_or(0.0),
                    zoom.clone(),
                    origin.clone(),
                    anchor.clone(),
                )),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn pip_closes_back_to_previous_shot() {
        // 2 秒的画中画，下一条运镜在 30 秒后 —— clip-23 的真实形态：不补就挂成 30 秒
        let mut segs = segs_of(&[(0.0, "中"), (10.0, "远"), (40.0, "中")]);
        assert_eq!(close_pip_shots(&mut segs), 1);
        let cams = cam_spans(&segs);
        assert_eq!(cams.len(), 4);
        assert_eq!(cams[2].0, 12.0, "到点（10+2）就回镜头");
        assert_eq!(cams[2].2, "中", "回到画中画之前的景别");
        assert_eq!(cams[2].3, "50% 50%", "落在前一个镜头的落点上");
        assert_eq!(cams[2].1, 28.0, "回镜头一直持到下一条运镜");
        assert_eq!(close_pip_shots(&mut segs), 0, "幂等：再跑一次不再补");
    }

    #[test]
    fn pip_close_followed_right_away_is_left_alone() {
        // 下一条运镜紧接着（< MIN_RETURN_SECS）→ 让它接手，不补（画中画最多多挂 0.5s）
        let mut segs = segs_of(&[(10.0, "远"), (12.2, "中")]);
        assert_eq!(close_pip_shots(&mut segs), 0);
        assert_eq!(cam_spans(&segs).len(), 2);

        // 下一条运镜落在画中画自己区间里 → 也不补
        let mut segs = segs_of(&[(10.0, "远"), (11.0, "中")]);
        assert_eq!(close_pip_shots(&mut segs), 0);
        assert_eq!(cam_spans(&segs).len(), 2);
    }

    #[test]
    fn pip_close_uses_previous_shot_landing_point() {
        // 前一条是「中」从主播移到弹幕：画中画收工要接在**终点**那一侧，不是起点
        let mover = |to_origin: Option<&str>| Event::Camera {
            time: 0.0,
            end: Some(5.0),
            zoom: "中".into(),
            origin: "10% 20%".into(),
            anchor: Some("liver".into()),
            anchor_to: Some("danmaku".into()),
            to_origin: to_origin.map(String::from),
            cut: false,
            handheld: false,
            manual: false,
        };
        for to_origin in [Some("70% 30%"), None] {
            let mut segs = vec![Segment {
                start: 0.0,
                end: 60.0,
                fx: "med".into(),
                note: String::new(),
                events: vec![mover(to_origin), cam(10.0, "远")],
                extra: Default::default(),
            }];
            assert_eq!(close_pip_shots(&mut segs), 1);
            let cams = cam_spans(&segs);
            if to_origin.is_some() {
                assert_eq!(cams[2].3, "70% 30%", "落点用解析出来的 to_origin");
            } else {
                assert_eq!(cams[2].3, "10% 20%", "还没解析（老工程/LLM 原始输出）→ 保留坐标字段");
            }
            assert_eq!(cams[2].4.as_deref(), Some("danmaku"), "锚点取终点意图，交回 resolve 解析");
        }
    }

    #[test]
    fn pip_close_without_previous_shot_uses_default_shot() {
        // 画中画就是全片第一条运镜 → 回 Step 1 的默认运镜（聚焦主播）
        let mut segs = segs_of(&[(10.0, "远")]);
        assert_eq!(close_pip_shots(&mut segs), 1);
        let cams = cam_spans(&segs);
        assert_eq!(cams[1].2, "中");
        assert_eq!(cams[1].3, "50% 50%");
        assert_eq!(cams[1].4.as_deref(), Some("liver"));
    }

    #[test]
    fn pip_close_respects_segment_boundary() {
        // 画中画自己跨出段尾：收工时刻夹回段尾 → 没有可补的余量
        let mut segs = vec![Segment {
            start: 0.0,
            end: 11.0,
            fx: "med".into(),
            note: String::new(),
            events: vec![cam(10.0, "远")],
            extra: Default::default(),
        }];
        assert_eq!(close_pip_shots(&mut segs), 0);
        assert_eq!(cam_spans(&segs).len(), 1);
    }

    #[test]
    fn pip_close_spans_segments() {
        // 画中画在本段末尾、下一条运镜在下一段里：回镜头按**全片**时刻算，跨段持过去
        let mut segs = vec![
            Segment {
                start: 0.0,
                end: 30.0,
                fx: "med".into(),
                note: String::new(),
                events: vec![cam(0.0, "中"), cam(20.0, "远")],
                extra: Default::default(),
            },
            Segment {
                start: 40.0,
                end: 60.0,
                fx: "med".into(),
                note: String::new(),
                events: vec![cam(45.0, "近")],
                extra: Default::default(),
            },
        ];
        assert_eq!(close_pip_shots(&mut segs), 1);
        let cams = cam_spans(&segs);
        assert_eq!((cams[2].0, cams[2].1), (22.0, 8.0), "回镜头从 22s 起、duration 夹在本段内（跨段由状态机保持）");
        assert_eq!(cams[2].2, "中");
    }

    #[test]
    fn pip_close_orders_by_time_not_by_file_order() {
        // 段之间时间上可以重叠（LLM 按窗口生成）→ 前后镜头按**时间**算，不看文件顺序：
        // 文件里画中画所在段排在前面，但时间上它之前是后一段那条「近」
        let mk = |start: f64, end: f64, ev: Event| Segment {
            start,
            end,
            fx: "med".into(),
            note: String::new(),
            events: vec![ev],
            extra: Default::default(),
        };
        let mut segs = vec![mk(100.0, 130.0, cam(105.0, "远")), mk(0.0, 60.0, cam(10.0, "近"))];
        assert_eq!(close_pip_shots(&mut segs), 1);
        let cams = cam_spans(&segs);
        assert_eq!(cams[1].0, 107.0, "回镜头插在画中画后面（同段）");
        assert_eq!(cams[1].2, "近", "回到时间上在它之前的那条「近」，不是默认中景");
        assert_eq!(cams[1].1, 23.0, "没有更晚的运镜 → 持到段尾 130s");
    }

    // ── 贴纸对齐最近的字幕 ──────────────────────────────────────────
    /// 起点 + 跨度（内部换算成 end，调用点读起来还是「几秒长」）
    fn sticker(time: f64, dur: f64) -> Event {
        Event::Sticker {
            time,
            end: Some(time + dur),
            keyword: "emoji".into(),
            sticker_tag: None,
            name: Some("laugh".into()),
            source: None,
            x: "50%".into(),
            y: "50%".into(),
            sfx: None,
            sfx_volume: None,
            manual: false,
        }
    }

    fn sticker_seg(ev: Event) -> Vec<Segment> {
        vec![Segment {
            start: 0.0,
            end: 60.0,
            fx: "med".into(),
            note: String::new(),
            events: vec![ev],
            extra: Default::default(),
        }]
    }

    fn sticker_span(segs: &[Segment]) -> (f64, f64) {
        match &segs[0].events[0] {
            Event::Sticker { time, end, .. } => (*time, end.unwrap_or(*time)),
            _ => panic!("expected sticker"),
        }
    }

    #[test]
    fn sticker_aligns_to_nearest_subtitle() {
        use crate::parse::srt::Subtitle;
        fn sub(i: usize, start: f64, end: f64) -> Subtitle {
            Subtitle { index: i, start, end, text: String::new(), html: String::new() }
        }
        let subs = vec![sub(1, 10.0, 12.5), sub(2, 14.0, 16.0)];

        // 起点落在第一条字幕里、时长比它短 → 头尾都吸到这条字幕（10.0~12.5）
        let mut segs = sticker_seg(sticker(10.4, 1.5));
        assert_eq!(align_stickers_to_subtitles(&mut segs, &subs), 1);
        assert_eq!(sticker_span(&segs), (10.0, 12.5));

        // 起点落在两条之间的空隙、离后一条更近（13.8 → 14.0 差 0.2，离前一条 1.3）→ 吸到后一条
        let mut segs = sticker_seg(sticker(13.8, 2.0));
        assert_eq!(align_stickers_to_subtitles(&mut segs, &subs), 1);
        assert_eq!(sticker_span(&segs), (14.0, 16.0));

        // 已经对齐 → 不动
        let mut segs = sticker_seg(sticker(14.0, 2.0));
        assert_eq!(align_stickers_to_subtitles(&mut segs, &subs), 0);

        // 离最近字幕超过 2s（纯视觉反应）→ 保持原位
        let mut segs = sticker_seg(sticker(20.0, 2.0));
        assert_eq!(align_stickers_to_subtitles(&mut segs, &subs), 0);
        assert_eq!(sticker_span(&segs), (20.0, 22.0));

        // 很碎的字幕（0.1s）**不再跳过，而是被最短时长规矩撑开**（2026-10-11 改）：
        // 旧行为是"盖不住就跳过"，但那样最该被治的碎片反而没人管。
        // 这里只有一条字幕、后面没有可吞的，于是落在它自己的结尾上（见
        // `element_accepts_short_span_when_no_more_subtitles`）。
        let short = vec![sub(1, 30.0, 30.1)];
        let mut segs = sticker_seg(sticker(30.05, 2.0));
        assert_eq!(align_stickers_to_subtitles(&mut segs, &short), 1);
        assert_eq!(sticker_span(&segs), (30.0, 30.1));

        // 没有字幕 → 原样
        let mut segs = sticker_seg(sticker(5.0, 2.0));
        assert_eq!(align_stickers_to_subtitles(&mut segs, &[]), 0);
    }

    /// **最短时长**：吸到碎字幕上的元素要往后「吞」到「够长 **且** 停在字幕结尾」。
    ///
    /// 用的是**现网实测的真实碎片链**（73 片段 / 909 条字幕里挑出来的那一串）：
    /// ASR 把一口气重复的音节切成 0.05s 的连续碎片 ——
    /// `21.92→21.97→22.02`，空隙，`22.22→22.27→22.32→22.37→22.42`，空隙，`22.47→22.52`，
    /// 空隙，`22.62→22.67`。元素吸到这种碎片上原本只闪 0.05s。
    #[test]
    fn element_extends_past_min_duration_and_lands_on_subtitle_end() {
        use crate::parse::srt::Subtitle;
        fn sub(i: usize, start: f64, end: f64) -> Subtitle {
            Subtitle { index: i, start, end, text: String::new(), html: String::new() }
        }
        // 真实碎片链（照抄实测的 `21.92→21.97→22.02` / `22.22→…→22.42`）——
        // ⚠️ 中间的**空隙是真实的**（ASR 切出来的碎片本来就不连续），别为了凑够 1.2s
        // 把它们填平：那会掩盖"字幕之间是静音留白、吞不过去"这个真实约束。
        let subs = vec![
            sub(1, 21.92, 21.97),
            sub(2, 21.97, 22.02),
            sub(3, 22.22, 22.27),
            sub(4, 22.27, 22.32),
            sub(5, 22.32, 22.37),
            sub(6, 22.37, 22.42),
            sub(7, 22.47, 22.52),
            sub(8, 22.62, 22.67),
            sub(9, 23.20, 25.80), // 再往后一条正常长度的，保证有得吞
        ];
        // 元素吸到第一条碎片：起点 21.92，原 end 21.97（只有 0.05s）
        let mut segs = sticker_seg(sticker(21.95, 0.02));
        align_stickers_to_subtitles(&mut segs, &subs);
        let (t0, t1) = sticker_span(&segs);
        assert_eq!(t0, 21.92, "起点吸到那条碎片字幕的开始");
        assert!(
            t1 > 21.97,
            "至少要吞过那 0.05s 的碎片（实得 {:.2}s）",
            t1 - t0
        );
        // ⚠️ 关键：**落在某条字幕的结尾上**（不能断在半句话中间）
        assert!(
            subs.iter().any(|s| (s.end - t1).abs() < 1e-6),
            "end 必须停在字幕边界上，实得 {t1}（字幕结尾：{:?}）",
            subs.iter().map(|s| s.end).collect::<Vec<_>>()
        );
        // 21.92 + 1.2 = 23.12 —— 但**够不到**：22.67 之后下一条从 23.20 起，
        // 中间空了 3.13s > [`ELEMENT_MAX_GAP_SECS`]，于是停在 22.67（不足 1.2s 也停）。
        // 宁可短，也不要在没人说话的画面上悬 3 秒。
        assert_eq!(t1, 22.67, "不该跨过 3.13s 的静音空隙去凑够 1.2s");

        // 反例：碎片之间挨得近（空隙 < 0.5s）时，就该一路吞到够长
        let tight = vec![
            sub(1, 30.00, 30.40),
            sub(2, 30.50, 30.90),
            sub(3, 31.00, 31.50),
            sub(4, 31.60, 32.30), // 30.00 + 1.2 = 31.20 ⇒ 第一条够长的是 32.30? 不：31.50 就够
        ];
        let mut segs = sticker_seg(sticker(30.05, 0.3));
        align_stickers_to_subtitles(&mut segs, &tight);
        assert_eq!(
            sticker_span(&segs),
            (30.0, 31.5),
            "空隙都很小 ⇒ 吞到第一个 ≥1.2s 的边界（31.5 - 30.0 = 1.5s）"
        );
    }

    /// **最长时长**：吸到长句上的元素要硬截到 [`ELEMENT_MAX_SECS`]。
    ///
    /// 实测现网有 16.30s 的字幕（>5s 的占 9.6%）——用户原话「后面满足时一句话很长就显得很奇怪」。
    #[test]
    fn element_is_capped_at_max_duration() {
        use crate::parse::srt::Subtitle;
        fn sub(i: usize, start: f64, end: f64) -> Subtitle {
            Subtitle { index: i, start, end, text: String::new(), html: String::new() }
        }
        // 真实样本：16.30s 那条（`[141.96, 158.26]`）
        let subs = vec![sub(1, 141.96, 158.26)];
        let mut segs = sticker_seg(sticker(142.0, 1.0));
        assert_eq!(align_stickers_to_subtitles(&mut segs, &subs), 1);
        let (t0, t1) = sticker_span(&segs);
        assert_eq!(t0, 141.96);
        assert_eq!(t1 - t0, ELEMENT_MAX_SECS, "应硬截到最长 {ELEMENT_MAX_SECS}s");
    }

    /// 字幕不够吞（后面没有更多字幕）时**接受现状**，不凭空造时长。
    ///
    /// 字幕之间的静音段是真实留白 —— 元素硬撑到那儿只会盖住没人说话的画面。
    #[test]
    fn element_accepts_short_span_when_no_more_subtitles() {
        use crate::parse::srt::Subtitle;
        fn sub(i: usize, start: f64, end: f64) -> Subtitle {
            Subtitle { index: i, start, end, text: String::new(), html: String::new() }
        }
        // 只有一条 0.6s 的字幕，后面再没有别的 → 吞不动，停在它自己的结尾
        let subs = vec![sub(1, 50.0, 50.6)];
        let mut segs = sticker_seg(sticker(50.05, 0.5));
        align_stickers_to_subtitles(&mut segs, &subs);
        assert_eq!(sticker_span(&segs), (50.0, 50.6), "没有更多字幕可吞就接受现状");
    }

    /// **幂等**：`cmd_process` 每次预览/出片都会被隐式触发，同一条元素反复规整必须收敛。
    #[test]
    fn element_fit_is_idempotent() {
        use crate::parse::srt::Subtitle;
        fn sub(i: usize, start: f64, end: f64) -> Subtitle {
            Subtitle { index: i, start, end, text: String::new(), html: String::new() }
        }
        let subs = vec![
            sub(1, 21.92, 21.97),
            sub(2, 21.97, 22.02),
            sub(3, 22.22, 22.27),
            sub(4, 22.27, 22.32),
            sub(5, 141.96, 158.26),
        ];
        for t in [21.95f64, 22.24, 142.0, 50.0] {
            let mut segs = sticker_seg(sticker(t, 1.0));
            align_stickers_to_subtitles(&mut segs, &subs);
            let first = sticker_span(&segs);
            let n = align_stickers_to_subtitles(&mut segs, &subs);
            assert_eq!(sticker_span(&segs), first, "第二轮不该再变（t={t}）");
            assert_eq!(n, 0, "第二轮不该再报对齐条数（t={t}）");
        }
    }

    /// 手工定过时间的贴纸**既不吸附也不延长**（用户报过"调整结束时间没成功"：
    /// 编辑器每次改动后自动跑一次 process，那时这里会把他的时间吸回字幕边界）。
    #[test]
    fn manual_sticker_keeps_its_time() {
        use crate::parse::srt::Subtitle;
        fn sub(i: usize, start: f64, end: f64) -> Subtitle {
            Subtitle { index: i, start, end, text: String::new(), html: String::new() }
        }
        let subs = vec![sub(1, 10.0, 12.5), sub(2, 14.0, 16.0)];

        let mut ev = sticker(10.4, 1.5);
        ev.set_time_manual(true);
        let mut segs = sticker_seg(ev);
        assert_eq!(align_stickers_to_subtitles(&mut segs, &subs), 0, "manual 不该被吸附");
        assert_eq!(sticker_span(&segs), (10.4, 11.9), "时间原样保留");

        // 清掉标记 → 恢复自动吸附（面板「跟随字幕」走的就是这条）
        segs[0].events[0].set_time_manual(false);
        assert_eq!(align_stickers_to_subtitles(&mut segs, &subs), 1);
        assert_eq!(sticker_span(&segs), (10.0, 12.5));
    }

    /// 手工拖过的画中画同样不被对齐回字幕边界。
    #[test]
    fn manual_pip_keeps_its_time() {
        use crate::parse::srt::Subtitle;
        fn sub(i: usize, start: f64, end: f64) -> Subtitle {
            Subtitle { index: i, start, end, text: String::new(), html: String::new() }
        }
        let subs = vec![sub(1, 9.0, 12.5), sub(2, 13.5, 16.0)];
        let mut ev = cam(10.0, "远");
        ev.set_time_manual(true);
        let mut segs = vec![Segment {
            start: 8.0, end: 20.0, fx: "med".into(), note: String::new(), events: vec![ev], extra: Default::default(),
        }];
        assert_eq!(align_pip_shots(&mut segs, &subs), 0, "manual 的画中画不该被对齐");
        let Event::Camera { time, end, .. } = &segs[0].events[0] else { panic!() };
        assert_eq!((*time, *end), (10.0, Some(12.0)), "时间原样保留");
    }

    #[test]
    fn pip_aligns_to_subtitle_boundaries() {
        use crate::parse::srt::Subtitle;
        fn sub(i: usize, start: f64, end: f64) -> Subtitle {
            Subtitle { index: i, start, end, text: String::new(), html: String::new() }
        }
        fn seg_with(ev: Event) -> Vec<Segment> {
            vec![Segment { start: 8.0, end: 20.0, fx: "med".into(), note: String::new(), events: vec![ev], extra: Default::default() }]
        }
        let subs = vec![sub(1, 9.0, 12.5), sub(2, 13.5, 16.0)];

        // 切镜 [10, 13]：起点在第一条字幕里 → 吸到它的开始；终点落在空隙 → 吸到上一条的结束
        let mut segs = seg_with(cam(10.0, "远"));
        if let Event::Camera { time, end, .. } = &mut segs[0].events[0] { *end = Some(*time + 3.0); }
        assert_eq!(align_pip_shots(&mut segs, &subs), 1);
        match &segs[0].events[0] {
            Event::Camera { time, end, zoom, .. } => {
                assert_eq!(*time, 9.0, "起点对到字幕开始");
                assert_eq!(*end, Some(12.5), "终点对到字幕结束（9.0~12.5）");
                assert_eq!(zoom, "远", "只动时间，档位不碰");
            }
            _ => panic!("expected camera"),
        }

        // 已经对齐的 → 不动
        let mut segs = seg_with(cam(9.0, "远"));
        if let Event::Camera { time, end, .. } = &mut segs[0].events[0] { *end = Some(*time + 3.5); }
        assert_eq!(align_pip_shots(&mut segs, &subs), 0);
    }

    #[test]
    fn pip_alignment_stretches_short_shot() {
        use crate::parse::srt::Subtitle;
        fn sub(i: usize, start: f64, end: f64) -> Subtitle {
            Subtitle { index: i, start, end, text: String::new(), html: String::new() }
        }
        // 切镜整段卡在一条 1.2s 的字幕里（< MIN_PIP_SECS）→ 往后借到下一条字幕结束
        let subs = vec![sub(1, 10.0, 11.2), sub(2, 11.5, 13.2)];
        let mut segs = vec![Segment {
            start: 0.0, end: 30.0, fx: "med".into(), note: String::new(),
            events: vec![cam(10.2, "远")], extra: Default::default(),   // cam 自带 duration 2.0
        }];
        assert_eq!(align_pip_shots(&mut segs, &subs), 1);
        match &segs[0].events[0] {
            Event::Camera { time, end, .. } => {
                assert_eq!(*time, 10.0);
                let span = end.unwrap() - *time;
                assert!(span >= MIN_PIP_SECS, "对齐后不该短于最短时长：{span}");
                assert!((span - 3.2).abs() < 1e-9, "借到下一条字幕结束（10.0~13.2）：{span}");
                assert_eq!(*end, Some(13.2), "终点就落在字幕结束点上");
            }
            _ => panic!("expected camera"),
        }
    }

    #[test]
    fn pip_alignment_clamps_to_segment_without_subtitles() {
        // 没有字幕 → 用分段边界兜底：跨过段尾的切镜被夹回段内
        let mut segs = vec![Segment {
            start: 0.0, end: 12.0, fx: "med".into(), note: String::new(),
            events: vec![cam(10.0, "远")], extra: Default::default(),
        }];
        if let Event::Camera { time, end, .. } = &mut segs[0].events[0] { *end = Some(*time + 5.0); }  // [10, 15]
        assert_eq!(align_pip_shots(&mut segs, &[]), 1);
        match &segs[0].events[0] {
            Event::Camera { time, end, .. } => {
                assert_eq!(*time, 10.0);
                assert_eq!(end.unwrap(), 12.0, "夹到段尾（事件边界）");
            }
            _ => panic!("expected camera"),
        }
    }

    #[test]
    fn pip_single_shot_untouched() {
        let mut segs = segs_of(&[(3.0, "中"), (10.0, "远"), (40.0, "近")]);
        assert_eq!(cap_pip_shots(&mut segs), 0);
        assert_eq!(zooms(&segs), vec!["中", "远", "近"]);
        // 「远」的坐标/锚点降级时不动：降级只改 zoom
        let mut segs = segs_of(&[(0.0, "远"), (2.0, "远")]);
        if let Event::Camera { origin, .. } = &mut segs[0].events[0] {
            *origin = "83.30% 50.00%".into();
        }
        cap_pip_shots(&mut segs);
        match &segs[0].events[1] {
            Event::Camera { origin, zoom, .. } => {
                assert_eq!(zoom, "中");
                assert_eq!(origin, "50% 50%", "降级只改档位，坐标留着");
            }
            _ => panic!("expected camera"),
        }
    }

    // ── 手持摇摆守卫 ────────────────────────────────────────────────
    fn handheld_flags(segs: &[Segment]) -> Vec<bool> {
        segs.iter()
            .flat_map(|s| s.events.iter())
            .filter_map(|e| match e {
                Event::Camera { handheld, .. } => Some(*handheld),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn handheld_only_survives_on_close_up() {
        // 近景（含带空格的写法）保留，其余景别一律剥掉 —— 用户定的门槛：只允许近景
        let mut segs = vec![Segment {
            start: 0.0, end: 60.0, fx: "med".into(), note: String::new(),
            events: vec![
                cam_h(0.0, "近", true),
                cam_h(5.0, " 近 ", true),
                cam_h(10.0, "中", true),
                cam_h(15.0, "远", true),
                cam_h(20.0, "", true),
                cam_h(25.0, "近", false),
            ],
            extra: Default::default(),
        }];
        assert_eq!(cap_handheld_sway(&mut segs), 3, "中 / 远 / 空标签各剥一条");
        assert_eq!(handheld_flags(&segs), vec![true, true, false, false, false, false]);
        // 幂等：再跑一次没有可剥的了
        assert_eq!(cap_handheld_sway(&mut segs), 0);
    }

    #[test]
    fn handheld_guard_is_strip_only() {
        // 中景不会被自动补上（可选项：系统不替用户开），近景没写就是没有
        let mut segs = segs_of(&[(0.0, "中"), (10.0, "近")]);
        assert_eq!(cap_handheld_sway(&mut segs), 0);
        assert_eq!(handheld_flags(&segs), vec![false, false]);
    }

    #[test]
    fn pip_close_restores_handheld_from_previous_shot() {
        // 前一条是「近」+ 手持摇摆：画中画收工回到近景，摇摆跟着回来
        let mut segs = vec![Segment {
            start: 0.0, end: 60.0, fx: "med".into(), note: String::new(),
            events: vec![cam_h(0.0, "近", true), cam(10.0, "远")],
            extra: Default::default(),
        }];
        assert_eq!(close_pip_shots(&mut segs), 1);
        assert_eq!(handheld_flags(&segs), vec![true, false, true], "回镜头承接前一条的手持摇摆");

        // 回到非近景（前一条被手改/LLM 写成中景 + handheld）：回镜头也承接，
        // 但两条中景都会被 cap_handheld_sway 剥掉 —— cmd_process 里守卫就排在收工之后
        let mut segs = vec![Segment {
            start: 0.0, end: 60.0, fx: "med".into(), note: String::new(),
            events: vec![cam_h(0.0, "中", true), cam(10.0, "远")],
            extra: Default::default(),
        }];
        assert_eq!(close_pip_shots(&mut segs), 1);
        assert_eq!(handheld_flags(&segs), vec![true, false, true]);
        assert_eq!(cap_handheld_sway(&mut segs), 2, "前一条与回镜头都剥掉");
        assert_eq!(handheld_flags(&segs), vec![false, false, false]);
    }

    // ── 冷开场 / 回忆回溯配额守卫（第 5 批）──────────────────────────
    /// 一条表达"冷开场/回忆回溯"意图的运镜事件（`anchor` 是意图、不是景别）。
    fn intent_cam(time: f64, end: f64, anchor: &str) -> Event {
        Event::Camera {
            time,
            end: Some(end),
            zoom: "中".into(),
            origin: "50% 50%".into(),
            anchor: Some(anchor.into()),
            anchor_to: None,
            to_origin: None,
            cut: false,
            handheld: false,
            manual: false,
        }
    }

    fn clip_of(s: f64, e: f64, kind: Option<&str>) -> EdlClip {
        EdlClip {
            source_start: s,
            source_end: e,
            speed: 1.0,
            kind: kind.map(String::from),
            label: None,
            source_kind: None,
            asset_id: None,
            transition_in: None,
        }
    }

    fn kinds(clips: &[EdlClip]) -> Vec<Option<&str>> {
        clips.iter().map(|c| c.kind.as_deref()).collect()
    }

    /// **配额守卫有判据**：3 个冷开场候选 ⇒ 落实后**只剩 1 个**（任务书的准入判据）。
    ///
    /// 与 `cap_pip_shots` 的 `pip_only_one_per_clip` 同一条：LLM 按段窗口生成，
    /// 三个窗口各写一次"这段是爆点"是完全可能的，而整片只允许一次。
    #[test]
    fn cold_open_only_one_per_clip() {
        let mut segs = vec![
            Segment { start: 0.0, end: 20.0, fx: "med".into(), note: String::new(),
                events: vec![intent_cam(2.0, 6.0, INTENT_COLD_OPEN)], extra: Default::default() },
            Segment { start: 20.0, end: 40.0, fx: "med".into(), note: String::new(),
                events: vec![intent_cam(22.0, 26.0, INTENT_COLD_OPEN)], extra: Default::default() },
            Segment { start: 40.0, end: 60.0, fx: "med".into(), note: String::new(),
                events: vec![intent_cam(42.0, 46.0, INTENT_COLD_OPEN)], extra: Default::default() },
        ];
        let mut clips: Vec<EdlClip> = Vec::new();
        let m = materialize_edl_intents(&mut segs, &mut clips, 60.0);
        assert_eq!(m.recalls, 0, "这一趟一条回忆回溯都没有");
        assert!(m.cold_open, "有一条活下来");
        assert_eq!(clips.len(), 1, "3 个候选 ⇒ 只剩 1 个：{clips:?}");
        assert_eq!(clips[0].kind.as_deref(), Some(EDL_KIND_COLD_OPEN));
        assert_eq!((clips[0].source_start, clips[0].source_end), (2.0, 6.0),
            "留下的是**最早**那条（与 cap_pip_shots 留最早一条「远」同规则）");
        // 全部意图都被清掉（活的死的都清）—— 幂等的前提
        let leftover: Vec<&str> = segs.iter().flat_map(|s| s.events.iter())
            .filter_map(|e| match e { Event::Camera { anchor, .. } => anchor.as_deref(), _ => None })
            .collect();
        assert!(leftover.is_empty(), "落实之后意图必须全部收走，仍有：{leftover:?}");
    }

    /// **幂等**：同一轨道跑两次，`edl.clips` **不再叠加**（任务书的准入判据）。
    ///
    /// `process` 会被预览与出片**隐式重跑**，所以"加一次就不再加"是硬要求 ——
    /// 不幂等的话用户每预览一次时间线就长一段。靠的是"意图是**一次性触发条件**"：
    /// 落实时把 `anchor` 收走，第二趟就没有候补了。
    #[test]
    fn cold_open_materialize_is_idempotent() {
        let mut segs = vec![Segment {
            start: 0.0, end: 30.0, fx: "med".into(), note: String::new(),
            events: vec![
                intent_cam(2.0, 6.0, INTENT_COLD_OPEN),
                intent_cam(12.0, 16.0, INTENT_RECALL),
                intent_cam(20.0, 24.0, INTENT_RECALL),
            ],
            extra: Default::default(),
        }];
        let mut clips: Vec<EdlClip> = Vec::new();

        let first = materialize_edl_intents(&mut segs, &mut clips, 30.0);
        assert!(first.cold_open);
        assert_eq!(first.recalls, 2);
        assert_eq!(kinds(&clips), vec![Some(EDL_KIND_COLD_OPEN), Some(EDL_KIND_RECALL), Some(EDL_KIND_RECALL)]);
        let after_first = clips.clone();

        // 第二趟：没有任何意图了 ⇒ 一个字节都不加，`changed()` 必须为假
        let second = materialize_edl_intents(&mut segs, &mut clips, 30.0);
        assert!(!second.changed(), "第二趟不许再落实任何东西");
        assert_eq!(clips, after_first, "EDL 逐值不变（不叠加）");

        // 第三趟以防万一（幂等要能反复成立，不是"两次碰巧一样"）
        let third = materialize_edl_intents(&mut segs, &mut clips, 30.0);
        assert!(!third.changed());
        assert_eq!(clips, after_first);
    }

    /// **老工程零回归**：没有意图 ⇒ `edl.clips` 保持为空 ⇒ `changed()` 为假（调用方不写盘）。
    ///
    /// 判据刻意落在 `changed()` 上，而不是"`clips` 是空的"：真正要挡的是
    /// **写盘那一步** —— `cmd_process` 只在 `changed()` 为真时才把 `clips` 交给
    /// `update_polish_toml`。有 EDL 时 `mute_ranges` 整个被忽略（`resolve_edl`、规格 §2.3），
    /// 所以"没意图就别写"就是 R4 本身。
    #[test]
    fn no_intent_keeps_edl_untouched() {
        // 普通运镜（没有 cold_open/recall）—— 都不算意图
        let mut segs = segs_of(&[(0.0, "中"), (10.0, "远"), (20.0, "近")]);
        let mut clips: Vec<EdlClip> = Vec::new();
        let m = materialize_edl_intents(&mut segs, &mut clips, 60.0);
        assert!(!m.changed(), "没有意图 ⇒ 不许改 EDL");
        assert!(clips.is_empty());
        let cams = cam_spans(&segs);
        assert_eq!(cams.len(), 3, "普通运镜一条都不许动");

        // 别的 anchor 也不许被误当成意图、更不许被"顺手清掉"（只清自己认得的两个值）
        let mut segs = vec![Segment {
            start: 0.0, end: 30.0, fx: "med".into(), note: String::new(),
            events: vec![Event::Camera {
                time: 0.0, end: Some(5.0), zoom: "中".into(), origin: "50% 50%".into(),
                anchor: Some("liver".into()), anchor_to: None, to_origin: None,
                cut: false, handheld: false, manual: false,
            }], extra: Default::default() }];
        let before: Vec<Option<String>> = segs.iter().flat_map(|s| s.events.iter())
            .filter_map(|e| match e { Event::Camera { anchor, .. } => Some(anchor.clone()), _ => None })
            .collect();
        let mut clips: Vec<EdlClip> = Vec::new();
        assert!(!materialize_edl_intents(&mut segs, &mut clips, 30.0).changed());
        let after: Vec<Option<String>> = segs.iter().flat_map(|s| s.events.iter())
            .filter_map(|e| match e { Event::Camera { anchor, .. } => Some(anchor.clone()), _ => None })
            .collect();
        assert_eq!(before, after, "不认得的 anchor 原样留着");

        // 用户手拖过的 EDL：**原样保留**，只在最前面插冷开场
        let mut segs = vec![Segment {
            start: 0.0, end: 30.0, fx: "med".into(), note: String::new(),
            events: vec![intent_cam(5.0, 9.0, INTENT_COLD_OPEN)], extra: Default::default() }];
        let mut clips = vec![clip_of(0.0, 30.0, Some("main"))];
        let m = materialize_edl_intents(&mut segs, &mut clips, 30.0);
        assert!(m.cold_open);
        assert_eq!(clips.len(), 2);
        assert_eq!(kinds(&clips), vec![Some(EDL_KIND_COLD_OPEN), Some("main")],
            "冷开场插在**数组最前**（数组顺序 = 成片播放顺序），用户那条原样跟在后面");
        assert_eq!((clips[1].source_start, clips[1].source_end), (0.0, 30.0), "用户那段没被改");
    }

    /// 手工运镜不参与：用户自己摆的运镜不被后处理改写（`manual` 那条硬规矩）。
    #[test]
    fn manual_camera_intent_is_ignored() {
        let mut ev = intent_cam(2.0, 6.0, INTENT_COLD_OPEN);
        if let Event::Camera { manual, .. } = &mut ev { *manual = true; }
        let mut segs = vec![Segment {
            start: 0.0, end: 30.0, fx: "med".into(), note: String::new(),
            events: vec![ev], extra: Default::default() }];
        let mut clips: Vec<EdlClip> = Vec::new();
        let m = materialize_edl_intents(&mut segs, &mut clips, 30.0);
        assert!(!m.changed(), "手工事件不参与（也不替用户清 anchor）");
        assert!(clips.is_empty());
        assert!(
            matches!(&segs[0].events[0], Event::Camera { anchor: Some(a), .. } if a == INTENT_COLD_OPEN),
            "用户手写的东西原样留着",
        );
    }

    /// 区间兜底：总时长未知 / 意图窗口太短 ⇒ 宁缺毋滥，一个空段都不往 EDL 里塞。
    ///
    /// 塞了的话它**进得了 `polish.toml`**（`resolve_clips` 才会跳过它），
    /// 看着像"有东西"而实际什么都不播。
    #[test]
    fn materialize_skips_impossible_spans() {
        // 总时长未知
        let mut segs = vec![Segment { start: 0.0, end: 30.0, fx: "med".into(), note: String::new(),
            events: vec![intent_cam(2.0, 6.0, INTENT_COLD_OPEN)], extra: Default::default() }];
        let mut clips: Vec<EdlClip> = Vec::new();
        assert!(!materialize_edl_intents(&mut segs, &mut clips, 0.0).changed(), "时长未知 ⇒ 不落实");
        assert!(clips.is_empty());

        // 窗口短于 MIN_COLD_OPEN_SECS：
        let mut segs = vec![Segment { start: 0.0, end: 30.0, fx: "med".into(), note: String::new(),
            events: vec![intent_cam(2.0, 2.4, INTENT_COLD_OPEN)], extra: Default::default() }];
        assert!(!materialize_edl_intents(&mut segs, &mut clips, 30.0).changed(), "太短 ⇒ 不落实");
        assert!(clips.is_empty());

        // 端点夹到 [0, total_duration]：起点为负、终点越界都夹回来
        let mut segs = vec![Segment { start: 0.0, end: 30.0, fx: "med".into(), note: String::new(),
            events: vec![intent_cam(-3.0, 99.0, INTENT_COLD_OPEN)], extra: Default::default() }];
        assert!(materialize_edl_intents(&mut segs, &mut clips, 30.0).cold_open);
        assert_eq!((clips[0].source_start, clips[0].source_end), (0.0, 30.0), "夹进 [0, 总时长]");
    }

    /// 回忆回溯上限是 [`MAX_RECALL`]（比冷开场松一档，但也是有上限的）。
    #[test]
    fn recall_is_capped() {
        let events: Vec<Event> = (0..5).map(|i| intent_cam(i as f64 * 10.0, i as f64 * 10.0 + 3.0, INTENT_RECALL)).collect();
        let mut segs = vec![Segment { start: 0.0, end: 60.0, fx: "med".into(), note: String::new(),
            events, extra: Default::default() }];
        let mut clips: Vec<EdlClip> = Vec::new();
        let m = materialize_edl_intents(&mut segs, &mut clips, 60.0);
        assert_eq!(m.recalls, MAX_RECALL, "5 个候选 ⇒ 只剩 MAX_RECALL 个");
        assert_eq!(clips.len(), MAX_RECALL);
        // 留最早的几条
        let starts: Vec<f64> = clips.iter().map(|c| c.source_start).collect();
        assert_eq!(starts, vec![0.0, 10.0], "按时间序取最早的那几条");
    }

    // ===== §2 句末边界守卫 =====

    fn ssub(i: usize, start: f64, end: f64, text: &str) -> Subtitle {
        Subtitle { index: i, start, end, text: text.into(), html: String::new() }
    }

    fn seg_of(start: f64, end: f64) -> Segment {
        Segment {
            start,
            end,
            fx: "med".into(),
            note: String::new(),
            events: Vec::new(),
            extra: Default::default(),
        }
    }

    /// **句末判据本身**：句末标点认、句中标点不认。
    ///
    /// 变异自证：把 `is_sentence_end` 里的 `'，'` 加进结尾字符集 ⇒ `逗号不是句末` 翻红。
    #[test]
    fn sentence_end_detects_terminal_punctuation_only() {
        // 句末：认
        for t in ["好。", "真的吗？", "太好了！", "我没想到…", "等等...", "ok.", "what?"] {
            assert!(is_sentence_end(t), "应判为句末：{t:?}");
        }
        // **句中：一律不认**（这些正是"话说一半"的标志）
        for t in ["你好，", "然后、", "因为；", "注意：", "没有标点", ""] {
            assert!(!is_sentence_end(t), "不该判为句末：{t:?}");
        }
        // ⚠️ **标点被引号包住**：ASR 的对白引号极常见，剥掉包裹符才认得出。
        // 不剥的话最常见的形态会被判成"没说完" ⇒ 守卫几乎失效。
        for t in ["他说\"好。\"", "「真的吗？」", "（笑）！", "'ok.'"] {
            assert!(is_sentence_end(t), "引号/括号收尾也要认得出句末：{t:?}");
        }
    }

    /// **守卫的主判据**：切在句子中间 ⇒ 挪到最近的"说完"位置。
    ///
    /// 场景：边界落在 `14.0~16.0` 这条字幕**内部**（那句话还没说完，无句末标点）。
    /// 两个候选：`12.5`（上一句说完，距 15.0 有 2.5s）与 `16.0`（这条字幕说完，距 1.0s）
    /// ⇒ 取**更近的 16.0**（少动为妙）。两段仍必须首尾相接。
    #[test]
    fn boundary_moves_off_mid_sentence_to_previous_sentence_end() {
        let subs = vec![
            ssub(1, 10.0, 12.5, "先说第一句。"),
            ssub(2, 14.0, 16.0, "这话还没说完呢接下来还有"), // 无句末标点
            ssub(3, 20.0, 22.0, "第二句说完了。"),
        ];
        let mut segs = vec![seg_of(0.0, 15.0), seg_of(15.0, 30.0)];
        let moved = align_segment_boundaries(&mut segs, &subs);
        assert_eq!(moved, 1, "应当挪动 1 处边界");
        assert_eq!(segs[0].end, 16.0, "挪到**最近**的说完位置（16.0 比 12.5 近）");
        assert_eq!(segs[1].start, 16.0, "后一段的 start 必须跟着一起改（首尾相接不变）");
    }

    /// **近的那一侧胜出**：候选在左右都有时取距离更小的（不是"一律往前"）。
    ///
    /// 这条把"方向偏好"钉死：守卫**不偏向任何一侧**，只按距离。
    /// 边界切在 `12.0`（一条长字幕内部），左侧候选 `7.0`（距 5.0 ❌超半径）、
    /// 右侧候选 `12.5`（距 0.5 ✅）⇒ 只能往右。
    /// 反过来的场景见下一条测试，两者一起证明**没有固定方向**。
    #[test]
    fn boundary_picks_the_nearer_side_not_a_fixed_direction() {
        // 边界 12.0 落在 `11.0~13.0` 这条**内部**（无句末标点 ⇒ 不干净）
        let subs = vec![
            ssub(1, 5.0, 7.0, "早一句"),   // end 7.0，距 12.0 = 5.0（超 4s 半径）
            ssub(2, 11.0, 13.0, "还在说"), // end 13.0，距 12.0 = 1.0 ✅
        ];
        let mut segs = vec![seg_of(0.0, 12.0), seg_of(12.0, 20.0)];
        assert_eq!(align_segment_boundaries(&mut segs, &subs), 1);
        assert_eq!(segs[0].end, 13.0, "只有右侧候选在半径内 ⇒ 往右");

        // 镜像场景：边界落在 `3.0~5.0` 内部，右侧候选远、左侧候选近 ⇒ 往左
        let subs2 = vec![
            ssub(1, 1.0, 2.0, "早一句"), // end 2.0，距 4.0 = 2.0 ✅
            ssub(2, 3.0, 5.0, "还在说"), // end 5.0，距 4.0 = 1.0 ✅（更近）
        ];
        let mut segs2 = vec![seg_of(0.0, 4.0), seg_of(4.0, 20.0)];
        assert_eq!(align_segment_boundaries(&mut segs2, &subs2), 1);
        assert_eq!(
            segs2[0].end, 5.0,
            "两侧都在半径内 ⇒ 取更近的（5.0 距 1.0 < 2.0 距 2.0）"
        );
    }

    /// **本来就在句末 ⇒ 一个字节都不动**（守卫的"零副作用"判据）。
    ///
    /// 落盘前后逐字节相同是 R4 的一部分：守卫不该把已经切干净的项目改出差异来。
    #[test]
    fn boundary_leaves_clean_cuts_untouched() {
        let subs = vec![
            ssub(1, 10.0, 12.5, "第一句。"),
            ssub(2, 14.0, 16.0, "第二句。"),
        ];
        // 边界就在 12.5（上一句结尾）
        let mut segs = vec![seg_of(0.0, 12.5), seg_of(12.5, 30.0)];
        let before: Vec<(f64, f64)> = segs.iter().map(|s| (s.start, s.end)).collect();
        assert_eq!(align_segment_boundaries(&mut segs, &subs), 0);
        let after: Vec<(f64, f64)> = segs.iter().map(|s| (s.start, s.end)).collect();
        assert_eq!(before, after, "已经切在句末的工程必须逐值不变");

        // 边界落在**宽空隙**里也算干净（没人说话，切不伤句子）
        let mut segs2 = vec![seg_of(0.0, 18.0), seg_of(18.0, 30.0)];
        // 12.5~14.0 只有 1.5s；用 17.5 那个真正的宽缝
        let subs2 = vec![
            ssub(1, 10.0, 12.5, "第一句。"),
            ssub(2, 20.0, 22.0, "第二句。"),
        ];
        assert_eq!(align_segment_boundaries(&mut segs, &subs2), 0);
        assert_eq!(segs[0].end, 12.5);
        assert_eq!(
            align_segment_boundaries(&mut segs2, &[]),
            0,
            "没有字幕时无可判 ⇒ 原样不动"
        );
    }

    /// **找不到合格点就不动** —— 宁可不切，也不按一个更差的点上去。
    ///
    /// 相邻两句都离得很远（超出 [`BOUNDARY_SEARCH_SECS`]）时，边界保持原值。
    /// ⚠️ 这条同时钉住"兜底候选也受同一个半径约束"：兜底放宽的是**标点要求**，不是**距离**。
    #[test]
    fn boundary_gives_up_when_no_clean_point_is_near() {
        let subs = vec![
            ssub(1, 0.0, 2.0, "很早的一句。"),
            ssub(2, 50.0, 52.0, "很晚的一句。"),
        ];
        let mut segs = vec![seg_of(0.0, 30.0), seg_of(30.0, 60.0)];
        assert_eq!(
            align_segment_boundaries(&mut segs, &subs),
            0,
            "30.0 附近 4s 内没有字幕结束点（句末或兜底都没有）⇒ 不许乱挪"
        );
        assert_eq!((segs[0].end, segs[1].start), (30.0, 30.0));
    }

    /// ⚠️ **兜底候选**：真实 ASR 字幕**常常没有句末标点**（本仓可得的样本里实测 0/13 = 0%）。
    ///
    /// 只认句末标点的话，这个守卫在真实数据上会**永远放弃** ⇒ 等于没做这一项。
    /// 所以没有句末点时退到"**字幕结束点**"—— 它仍强于"句中任意位置"。
    ///
    /// 变异自证：把 `best_boundary_near` 里第 ② 类候选整段删掉 ⇒ 本测试翻红
    /// （边界会停在 15.0 不动，而那里正切在一句话中间）。
    #[test]
    fn boundary_falls_back_to_subtitle_end_when_no_terminal_punctuation() {
        // 三条字幕**全都没有**句末标点（模拟真实 ASR 输出）
        let subs = vec![
            ssub(1, 10.0, 12.5, "先说第一句"),
            ssub(2, 14.0, 16.0, "这话还没说完"),
            ssub(3, 20.0, 22.0, "第二句说完了"),
        ];
        assert!(
            subs.iter().all(|s| !is_sentence_end(&s.text)),
            "前提：这批字幕一个句末标点都没有"
        );
        let mut segs = vec![seg_of(0.0, 15.0), seg_of(15.0, 30.0)];
        let moved = align_segment_boundaries(&mut segs, &subs);
        assert_eq!(moved, 1, "没有句末标点也要能挪（退到字幕结束点）");
        assert_eq!(
            (segs[0].end, segs[1].start),
            (16.0, 16.0),
            "15.0 切在「这话还没说完」中间 ⇒ 退到它说完的 16.0（比 12.5 更近）"
        );
    }

    /// **单段工程 / 有空隙的段表一律不动**（边界结构的合法性）。
    #[test]
    fn boundary_respects_segment_structure() {
        let subs = vec![ssub(1, 10.0, 12.5, "一句。")];
        // 只有一个段：无处可挪（挪了就是剪片头片尾）
        let mut one = vec![seg_of(0.0, 15.0)];
        assert_eq!(align_segment_boundaries(&mut one, &subs), 0);
        assert_eq!((one[0].start, one[0].end), (0.0, 15.0));

        // 两段**不接壤**（中间有缝）：不是"共享边界"，不许动
        let mut gap = vec![seg_of(0.0, 15.0), seg_of(20.0, 30.0)];
        assert_eq!(
            align_segment_boundaries(&mut gap, &subs),
            0,
            "有缝的两段之间不是共享边界"
        );
        assert_eq!((gap[0].end, gap[1].start), (15.0, 20.0));
    }

    /// **幂等**：`process` 每次预览/出片都被隐式重跑，守卫必须收敛。
    #[test]
    fn boundary_guard_is_idempotent() {
        let subs = vec![
            ssub(1, 10.0, 12.5, "第一句。"),
            ssub(2, 14.0, 16.0, "没说完的话"),
            ssub(3, 20.0, 22.0, "第二句。"),
        ];
        let mut segs = vec![seg_of(0.0, 15.0), seg_of(15.0, 30.0)];
        let first = align_segment_boundaries(&mut segs, &subs);
        assert_eq!(first, 1);
        let snapshot: Vec<(f64, f64)> = segs.iter().map(|s| (s.start, s.end)).collect();
        assert_eq!(
            align_segment_boundaries(&mut segs, &subs),
            0,
            "第二次必须不再挪（已经落在句末）"
        );
        let again: Vec<(f64, f64)> = segs.iter().map(|s| (s.start, s.end)).collect();
        assert_eq!(snapshot, again, "幂等：值也不许漂");
    }

    /// **不许把段吃到只剩一帧**：挪动后每段仍需 ≥ [`MIN_SEGMENT_SECS`]。
    #[test]
    fn boundary_never_shrinks_a_segment_below_minimum() {
        let subs = vec![ssub(1, 0.4, 0.6, "很短的一句。")];
        // 边界在 2.0；唯一的合格点在 0.6，挪过去会把前段压到 0.6（> 0.5）……
        // 换成 0.2 起点的段：挪到 0.6 后前段只有 0.4s < MIN ⇒ 必须放弃
        let mut segs = vec![seg_of(0.2, 2.0), seg_of(2.0, 10.0)];
        let moved = align_segment_boundaries(&mut segs, &subs);
        assert_eq!(moved, 0, "会把前段吃到 0.4s（低于下限）⇒ 必须放弃");
        assert_eq!((segs[0].end, segs[1].start), (2.0, 2.0));
    }
}
