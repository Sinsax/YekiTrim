//! polish::process::fx：事件默认值补齐与守卫（瞬时特效跨度 / 画中画配额与对齐 / 手持摇摆 / 贴纸对齐字幕）。
//!
//! 入口：`transient_duration_cap`、`transient_default_duration`、`normalize_event_ends`、`cap_pip_shots`、`align_pip_shots`、`close_pip_shots`、`cap_handheld_sway`、`align_stickers_to_subtitles`。
//! 边界：手工事件（`Event::is_time_manual()`）一律跳过改写；这些守卫在 `process/mod.rs` 里的调用顺序是硬约束，改一处要看整条 Step 序 —— 见 `crates/yekitrim-polish/src/process/AGENTS.md`

use crate::parse::srt::Subtitle;
use crate::project::{Decision, EdlClip, Event, MuteRange, Segment};

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

/// 事件对应的字符串类型名（**瞬时特效判定的唯一入口**：`audit` 也用它，
/// 别在别处再抄一份"哪些类型算瞬时"）。
pub(super) fn event_type_name(event: &Event) -> &'static str {
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

/// 画中画（`zoom = "远"`）在**整片**里最多几条（2026-10-20 运镜导演化 §三⑧：1 → 每段 1 条 / 全片 ≤3）。
pub const MAX_PIP_SHOTS_TOTAL: usize = 3;

/// 画中画在**同一段**里最多几条（段内互斥：同段两条仍降级）。
pub const MAX_PIP_SHOTS_PER_SEGMENT: usize = 1;

/// 旧的"整片只有一次"配额。**保留作兼容常量**（老判据/文档还在引用它）：
/// 现在它只表示"一个**段**里的上限"，与 [`MAX_PIP_SHOTS_PER_SEGMENT`] 同值。
pub const MAX_PIP_SHOTS: usize = MAX_PIP_SHOTS_PER_SEGMENT;

/// 画中画降频守卫（**每段 1 条 / 全片 ≤3**，2026-10-20 运镜导演化 §三⑧）。
///
/// # 为什么从"整片只有一次"放宽
///
/// 旧配额（整片 1 条）的本意是"画中画是罕见的戏剧手段"，但它同时把「同一段里有两处同屏信息源
/// （对比/揭穿）」也一起禁掉了 —— 而那正是画中画**最合适**的用法。新口径：**段内互斥、全片限量**。
///
/// * 段内：同一段最多 [`MAX_PIP_SHOTS_PER_SEGMENT`] 条（同段两条仍**降级**，保住互斥）；
/// * 全片：最多 [`MAX_PIP_SHOTS_TOTAL`] 条（按**时间序**取最早的，超出降级）。
///
/// **LLM 是按段窗口生成的，数不出"整片几次"**，所以配额只能在这里兜。
/// 降级 = **只改 zoom 成「中」**，`anchor`/`origin` 原样留着（效果仍在，只是不再是画中画）。
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

    // 段内计数 + 全片计数（两个上限各自独立：段内 1 条、全片 3 条）。
    let mut demoted = 0usize;
    let mut kept_total = 0usize;
    let mut per_seg: std::collections::HashMap<usize, usize> = std::collections::HashMap::new();
    for (si, ei, time) in order {
        let is_pip = matches!(
            &segments[si].events[ei],
            Event::Camera { zoom, .. } if zoom.trim() == PIP_LABEL
        );
        if !is_pip {
            continue;
        }
        let in_seg = *per_seg.get(&si).unwrap_or(&0);
        let over_seg = in_seg >= MAX_PIP_SHOTS_PER_SEGMENT;
        let over_total = kept_total >= MAX_PIP_SHOTS_TOTAL;
        if over_seg || over_total {
            if let Event::Camera { zoom, .. } = &mut segments[si].events[ei] {
                *zoom = "中".into();
            }
            demoted += 1;
            let why = if over_seg {
                format!("同段已有 {in_seg} 条（段内上限 {MAX_PIP_SHOTS_PER_SEGMENT}）")
            } else {
                format!("全片已有 {kept_total} 条（上限 {MAX_PIP_SHOTS_TOTAL}）")
            };
            tracing::info!("  ⚠ 画中画太密（{time:.1}s · {why}）：降级为中景");
        } else {
            kept_total += 1;
            per_seg.insert(si, in_seg + 1);
        }
    }
    demoted
}

/// **③ 切镜必须「变化一个维度」**（2026-10-20 运镜导演化 §一③）。
///
/// 一条 `cut: true` 的运镜若**景别与焦点都没变**，切过去和上一条一模一样 ——
/// 观众只看到"画面抖了一下"，而它占掉了硬切这个手法。
/// ⇒ 把这类事件的 `cut` **去掉**（退回平滑），**不删事件**（时间/焦点可能还有用）。
///
/// "变了" ＝ **景别不同** 或 **聚焦目标/落点不同** 之一：
///   * 景别：`zoom` 字面量不同；
///   * 焦点：`anchor` 不同，或（解析后的）`origin` 不同。
///     两处都看是有意的 —— Step 2d 之前 `origin` 还是占位（`50% 50%`），之后才是真坐标；
///     **本函数必须在 2d 之后调用**，否则"同一锚点换坐标"会被误判成没变。
///
/// ⚠️ **`manual` 事件不碰**（用户自己摆的运镜，后处理一律不改写）。
/// ⚠️ **第一条运镜不动**：没有"上一条"可比，去掉 cut 反而让开场变缓。
/// ⚠️ 幂等：去掉的 `cut` 第二趟仍是 `false` ⇒ 不再计数、也不再改。
pub fn demote_cut_without_change(segments: &mut [Segment]) -> usize {
    // 按**时间**收集（段之间会重叠，文件顺序不可靠 —— 与 `cap_pip_shots` 同一条教训）。
    let mut order: Vec<(usize, usize, f64)> = Vec::new();
    for (si, seg) in segments.iter().enumerate() {
        for (ei, ev) in seg.events.iter().enumerate() {
            if let Event::Camera { time, .. } = ev {
                order.push((si, ei, *time));
            }
        }
    }
    order.sort_by(|a, b| a.2.partial_cmp(&b.2).unwrap_or(std::cmp::Ordering::Equal));

    #[derive(Clone, PartialEq)]
    struct State {
        zoom: String,
        anchor: Option<String>,
        origin: String,
    }

    let mut demoted = 0usize;
    let mut prev: Option<State> = None;
    for (si, ei, time) in order {
        let (cur, manual, cut) = match &segments[si].events[ei] {
            Event::Camera { zoom, anchor, origin, cut, manual, .. } => (
                State {
                    zoom: zoom.trim().to_string(),
                    anchor: anchor.as_ref().map(|a| a.trim().to_string()),
                    origin: origin.trim().to_string(),
                },
                *manual,
                *cut,
            ),
            _ => continue,
        };
        if cut && !manual {
            let changed = match &prev {
                None => true,
                Some(p) => p.zoom != cur.zoom || p.anchor != cur.anchor || p.origin != cur.origin,
            };
            if !changed {
                if let Event::Camera { cut, .. } = &mut segments[si].events[ei] {
                    *cut = false;
                }
                demoted += 1;
                tracing::info!(
                    "  ⚠ 运镜 @{time:.1}s：cut 但景别/焦点都没变 -> 退回平滑（切镜必须变化一个维度）"
                );
            }
        }
        prev = Some(cur);
    }
    demoted
}

// ═══════════════════════════════════════════════════════════════════════════
// 运镜导演化（2026-10-20 §一）—— **全部只在 [camera] direct = true 时调用**
//
// ⚠️ 为什么整批放在一个开关后面：这些函数都会**新增或改写事件**，无条件跑一次
//    就会让每个存量工程变样（R4）。门在 `project::CameraDirectingConfig`。
// ⚠️ 五条共同纪律：**幂等**（第二趟 0 条）、**确定性**（纯函数、无随机）、
//    **`manual` 事件一律不碰**、**不改事件的 `time`**（只增/改 zoom·cut·sfx）。
// ═══════════════════════════════════════════════════════════════════════════

/// 某段里覆盖 `t` 的那条运镜的 zoom（时间上最后一条 `time <= t`）——**只读**。
fn zoom_state_at(seg: &Segment, t: f64) -> Option<String> {
    let mut cur: Option<(f64, String)> = None;
    for ev in &seg.events {
        if let Event::Camera { time, zoom, .. } = ev {
            if *time <= t + 1e-9 && cur.as_ref().map(|(bt, _)| time >= bt).unwrap_or(true) {
                cur = Some((*time, zoom.trim().to_string()));
            }
        }
    }
    cur.map(|(_, z)| z)
}

/// **① 爆点驱动景别**（§一①）：按段内爆点分数分配景别，**不用 LLM、无随机**。
///
/// 只改每段**最早那条非 manual 运镜**。分档口径（相对全片均值，不设绝对阈值）：
/// `score >= 1.6 × mean` ⇒ **近**；`>= 0.9 × mean` ⇒ **中**；其余 ⇒ **全**。
///
/// ⚠️ **输入打平 ⇒ 全部回落「全」**：这条就是验收里的变异自证。
/// ⚠️ 幂等：目标值只由 `punches` 决定，与当前 zoom 无关 ⇒ 第二趟 0 条变更。
pub fn assign_punch_zooms(segments: &mut [Segment], punches: &[(f64, f64)]) -> usize {
    if segments.is_empty() {
        return 0;
    }
    let scores: Vec<f64> = segments
        .iter()
        .map(|seg| {
            punches
                .iter()
                .filter(|(at, _)| *at >= seg.start - 1e-9 && *at < seg.end + 1e-9)
                .map(|(_, s)| *s)
                .fold(0.0_f64, f64::max)
        })
        .collect();
    let mean = scores.iter().sum::<f64>() / scores.len() as f64;
    let flat = mean <= 1e-9 || scores.iter().all(|s| (s - scores[0]).abs() < 1e-9);
    let mut changed = 0usize;
    for (i, seg) in segments.iter_mut().enumerate() {
        let target = if flat {
            "全".to_string()
        } else if scores[i] >= mean * 1.6 {
            "近".to_string()
        } else if scores[i] >= mean * 0.9 {
            "中".to_string()
        } else {
            "全".to_string()
        };
        let mut best: Option<(usize, f64)> = None;
        for (ei, ev) in seg.events.iter().enumerate() {
            if let Event::Camera { time, manual, .. } = ev {
                if *manual {
                    continue;
                }
                if best.as_ref().map(|(_, t)| time < t).unwrap_or(true) {
                    best = Some((ei, *time));
                }
            }
        }
        let Some((ei, t)) = best else { continue };
        let done = match &mut seg.events[ei] {
            Event::Camera { zoom, .. } if zoom.trim() != target => {
                *zoom = target.clone();
                true
            }
            _ => false,
        };
        if done {
            changed += 1;
            tracing::info!("  🎥 爆点驱动景别：段{i} @{t:.1}s -> {target}");
        }
    }
    changed
}

/// **② 句读级微推**（§一②）：段内每 3–6s 一次极轻推拉（`微`=1.06 ↔ 段基准档），方向交替。
///
/// 切点优先取「距上一拍 ≥3s 的最近一条字幕开始」，没有就 4.5s 一拍；距段尾 <1s 不补。
/// ⚠️ 只加新事件；同 ±0.05s 已有运镜 ⇒ 跳过（幂等）。
pub fn add_micro_push(segments: &mut [Segment], subtitles: &[Subtitle]) -> usize {
    let mut added = 0usize;
    for seg in segments.iter_mut() {
        if seg.end - seg.start < 3.0 {
            continue;
        }
        let base = zoom_state_at(seg, seg.start).unwrap_or_else(|| "全".to_string());
        let other = if base == "微" { "全".to_string() } else { base.clone() };
        let mut cursor = seg.start + 3.0;
        let mut flip = false;
        while cursor < seg.end - 1.0 {
            let zoom = if flip { other.clone() } else { "微".to_string() };
            flip = !flip;
            let exists = seg.events.iter().any(|ev| {
                matches!(ev, Event::Camera { time, .. } if (time - cursor).abs() < 0.05)
            });
            if !exists {
                let end = (cursor + 2.0).min(seg.end);
                seg.events.push(Event::Camera {
                    time: cursor,
                    end: Some(end),
                    zoom,
                    origin: "50% 50%".into(),
                    anchor: None,
                    anchor_to: None,
                    to_origin: None,
                    cut: false,
                    handheld: false,
                    manual: false,
                    speed: None,
                    tilt: None,
                });
                added += 1;
            }
            let sub_next = subtitles
                .iter()
                .map(|s| s.start)
                .filter(|t| *t >= cursor + 3.0 - 1e-9 && *t < seg.end)
                .fold(None::<f64>, |acc, t| match acc {
                    None => Some(t),
                    Some(a) => Some(a.min(t)),
                });
            cursor = match sub_next {
                Some(t) => t.min(cursor + 6.0),
                None => cursor + 4.5,
            };
        }
    }
    added
}

/// **⑤ 段首尾呼吸**（§一⑤）：段首接得平滑（不吃 cut）、段尾 0.3s 微拉远收束。
///
/// 幂等：段首已 `cut=false` 不计数；段尾在 [end−0.35, end−0.05] 已有事件就跳过。
pub fn segment_breathing(segments: &mut [Segment]) -> usize {
    let weaker = |z: &str| -> String {
        match z.trim() {
            "特写" => "近",
            "近" => "中",
            "中" => "全",
            _ => "全",
        }
        .to_string()
    };
    let mut changed = 0usize;
    for seg in segments.iter_mut() {
        if seg.end - seg.start < 2.0 {
            continue;
        }
        for ev in seg.events.iter_mut() {
            if let Event::Camera { time, cut, manual, .. } = ev {
                if (*time - seg.start).abs() < 0.05 && *cut && !*manual {
                    *cut = false;
                    changed += 1;
                }
            }
        }
        let tail = seg.end - 0.3;
        let exists = seg.events.iter().any(|ev| {
            matches!(ev, Event::Camera { time, .. } if (time - tail).abs() < 0.3 && *time < seg.end - 0.04)
        });
        if !exists {
            let cur = zoom_state_at(seg, tail).unwrap_or_else(|| "全".to_string());
            let target = weaker(&cur);
            if target != cur {
                seg.events.push(Event::Camera {
                    time: tail,
                    end: Some(seg.end),
                    zoom: target,
                    origin: "50% 50%".into(),
                    anchor: None,
                    anchor_to: None,
                    to_origin: None,
                    cut: false,
                    handheld: false,
                    manual: false,
                    speed: None,
                    tilt: None,
                });
                changed += 1;
            }
        }
    }
    changed
}

/// **④ 运镜与音效同步**（§一④）：每个 cut / snap 起点 0.3s 内保证一条 sfx。
///
/// ⚠️ **必须排在 Step 3（音效规则引擎）之后**：规则引擎会删掉"字幕里找不到依据"的独立 sfx，
///    而这条是运镜驱动的、没有台词依据 —— 排前面会被它当场删掉。
/// ⚠️ 幂等：0.3s 内已有任意 Sfx 就跳过。
pub fn sync_camera_sfx(segments: &mut [Segment]) -> usize {
    let mut wanted: Vec<(usize, f64)> = Vec::new();
    for (si, seg) in segments.iter().enumerate() {
        for ev in &seg.events {
            if let Event::Camera { time, cut, speed, .. } = ev {
                let is_snap = *cut
                    || speed.as_deref().map(|s| s.trim().eq_ignore_ascii_case("snap")).unwrap_or(false);
                if !is_snap {
                    continue;
                }
                let has = seg.events.iter().any(|x| {
                    matches!(x, Event::Sfx { time: st, .. } if (st - time).abs() <= 0.3)
                });
                if !has {
                    wanted.push((si, *time));
                }
            }
        }
    }
    let n = wanted.len();
    for (si, time) in wanted {
        segments[si].events.push(Event::Sfx {
            time,
            name: "whoosh".into(),
            volume: None,
            end: None,
        });
    }
    n
}

/// **⑨ 竖素材纵向运镜**（§一⑨）：竖屏**只做纵向**推拉/位移，不做横向（横摇必露边）。
///
/// 判据只读**工程里已有的字段**（`canvas.orientation` / `stage`），**不在 process 里开子进程探源**。
/// 做法：每段那条运镜的 `origin` 只动 **y**（42% ↔ 58% 交替）、zoom 在 `全`/`微` 之间交替。
/// ⚠️ 幂等：目标值由段序号决定，第二趟算出同一组值。
pub fn direct_vertical_portrait(segments: &mut [Segment]) -> usize {
    let mut changed = 0usize;
    for (i, seg) in segments.iter_mut().enumerate() {
        let y = if i % 2 == 0 { 42.0 } else { 58.0 };
        let zoom = if i % 2 == 0 { "全" } else { "微" };
        for ev in seg.events.iter_mut() {
            if let Event::Camera { time, zoom: z, origin, manual, .. } = ev {
                if *manual || (*time - seg.start).abs() >= 1.0 {
                    continue;
                }
                let want = format!("50% {y}%");
                let mut here = 0;
                if origin.trim() != want {
                    *origin = want.clone();
                    here += 1;
                }
                if z.trim() != zoom {
                    *z = zoom.to_string();
                    here += 1;
                }
                if here > 0 {
                    changed += 1;
                }
            }
        }
    }
    changed
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
            speed: None,
            tilt: None,
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

// ── 特写档（§18-A）────────────────────────────────────────────────

/// 特写档位的档位名（与 `placement::CLOSEUP_LABEL` 同源，别各写一份字面量）。
const CLOSEUP_LABEL: &str = crate::process::placement::CLOSEUP_LABEL;

/// 主播焦点锚点名（`anchor = "liver"`）—— 特写档的**唯一**合法焦点。
const ANCHOR_LIVER: &str = "liver";

/// 特写焦点守卫（规格 `camera-and-noise-spec.md` §3.3 末段，用户拍板）：
/// **特写档强制 `anchor = liver`**。
///
/// # ⚠️ 为什么这是硬约束而不是提示
///
/// 特写把画面放到 **1.8 倍**。聚焦到弹幕/贴纸时，"不动点"会跟着移到那个目标上 ——
/// 而弹幕区在 1.8 倍下**基本被挤出画面**（1080 高的 1.8 倍只看得到中间 600px）。
/// 症状是"切了特写，主播不见了，只剩弹幕在一片糊背景上"。
///
/// ⇒ 这一档下**任何非 `liver` 的聚焦意图都不成立**，直接改回 `liver`。
///
/// # 只改 `anchor`，不动 `origin`
///
/// `origin` 是 Step 2d 由 `anchor` 解析出来的**结果坐标**，每次跑 process 都会重算覆盖
/// （见 `resolve_camera_anchors`）—— 所以这里只改因、不碰果。
///
/// ⚠️ `anchor_to`（终点聚焦）**也一起管**：它是"从这里移到那里"的终点，
/// 特写下移到弹幕同样看不到。两头都锁死在 `liver`。
///
/// ⚠️ **只碰 `特写` 档**：`中`/`近`/`远` 一个字节不动（R4）。返回被改的条数。
pub fn cap_closeup_anchor(segments: &mut [Segment]) -> usize {
    let mut changed = 0usize;
    for seg in segments.iter_mut() {
        for ev in seg.events.iter_mut() {
            let Event::Camera { zoom, anchor, anchor_to, .. } = ev else { continue };
            if zoom.trim() != CLOSEUP_LABEL {
                continue;
            }
            // `None` 也要补上：特写没有一个"合理的默认焦点"，缺省就是主播。
            let anchor_ok = anchor.as_deref().map(str::trim) == Some(ANCHOR_LIVER);
            if !anchor_ok {
                *anchor = Some(ANCHOR_LIVER.to_string());
                changed += 1;
            }
            // 终点：写了别的就拉回来；本来就 `None` 时**不动**（原地聚焦是合法的，
            // 补一个等值的 `anchor_to` 只会让 polish.toml 多一行噪声）。
            if let Some(to) = anchor_to.as_deref() {
                if to.trim() != ANCHOR_LIVER {
                    *anchor_to = Some(ANCHOR_LIVER.to_string());
                    changed += 1;
                }
            }
        }
    }
    changed
}

// ── 荷兰角（§19）──────────────────────────────────────────────────

/// 荷兰角（`tilt`）的角度上限（度）—— 规格 §4.3 定的"防滥用"值。
///
/// 超过就像"拍摄事故"；6° 已是明显倾斜。⚠️ 这个上限**有几何依据、不是拍脑袋**：
/// 整帧旋转 6° 后要盖满画布只需缩放 `1.1804`，而 `中` 档给 1.3、`近` 档给 1.5，
/// 余量都很充足（规格 §4.4b 有完整推导与四角包含检验）。
pub const MAX_DUTCH_ANGLE: f64 = 6.0;

/// 荷兰角在**整片里最多几次**（规格 §4.3）—— 照 `MAX_PIP_SHOTS` 的先例。
///
/// 理由与画中画同一条：这类"特殊手段"一旦不受限就不特殊了。
pub const MAX_DUTCH_SHOTS: usize = 2;

/// 荷兰角守卫（规格 §4.3 + §4.4b）：**夹角度 + 限额 + 禁配画中画**。
///
/// # ⚠️ 为什么必须禁配「远」（画中画）
///
/// 整帧旋转 θ 后要盖满 1920×1080，需要的缩放是
/// `max((W·cosθ + H·sinθ)/W, (W·sinθ + H·cosθ)/H)`。反解各档最大安全角：
///
/// | 档位 | 倍率 | 最大安全角 |
/// |---|---|---|
/// | **远（画中画）** | **0.55** | **0.00°** |
/// | 中 | 1.30 | 10.24° |
/// | 近 | 1.50 | 17.98° |
///
/// 「远」的画面**本来就只有画布的 55%**（左右各 432px 黑边），再转必然露出更多 ——
/// **一格都转不得**。而规格 §4.2 的场景表第 3 条恰恰写着"画中画配合……两者叠加最自然"
/// （§4.4b 记录了这处自相矛盾）。两条硬约束里**几何是硬的那条**，所以按它来。
///
/// # 三条规则
///
/// 1. **夹到 ±[`MAX_DUTCH_ANGLE`]**；`0` / 非有限值 ⇒ 清成 `None`（不写出）。
/// 2. **与「远」同段** ⇒ 直接剔除（见上）。⚠️ 判据是**同一段里有 `zoom == "远"` 的运镜**，
///    不是"这条运镜自己是远" —— 状态机是持续的，同段任何一条画中画都会让它露边。
/// 3. **全片最多 [`MAX_DUTCH_SHOTS`] 条**，超出按 `time` 最早的保留（确定性）。
///
/// ⚠️ **只剥不补**：没写 `tilt` 的工程一个字节都不动（R4）。返回被剥/被夹的条数。
pub fn cap_dutch_angle(segments: &mut [Segment]) -> usize {
    let mut changed = 0usize;
    // (段下标, 事件下标, time) —— 只在"活下来"的倾斜上收集，供配额那条用
    let mut kept: Vec<(usize, usize, f64)> = Vec::new();

    for (si, seg) in segments.iter_mut().enumerate() {
        // 这一段里有没有画中画（任意一条运镜）
        let has_wide = seg
            .events
            .iter()
            .any(|ev| matches!(ev, Event::Camera { zoom, .. } if zoom.trim() == PIP_LABEL));
        for (ei, ev) in seg.events.iter_mut().enumerate() {
            let Event::Camera { tilt, time, .. } = ev else { continue };
            let Some(deg) = *tilt else { continue };
            // ① 非有限 / 零 ⇒ 清掉（写进 toml 或交给转译器都会出问题）
            if !deg.is_finite() || deg == 0.0 {
                *tilt = None;
                changed += 1;
                continue;
            }
            // ② 与画中画同段 ⇒ 剔除（会露黑边）
            if has_wide {
                *tilt = None;
                changed += 1;
                continue;
            }
            // ① 夹到上限（负数合法：两个方向都是荷兰角）
            let clamped = deg.clamp(-MAX_DUTCH_ANGLE, MAX_DUTCH_ANGLE);
            if (clamped - deg).abs() > f64::EPSILON {
                *tilt = Some(clamped);
                changed += 1;
            }
            kept.push((si, ei, *time));
        }
    }

    // ③ 全片配额：按 `time` 最早保留，其余剥掉
    if kept.len() > MAX_DUTCH_SHOTS {
        kept.sort_by(|a, b| a.2.partial_cmp(&b.2).unwrap_or(std::cmp::Ordering::Equal));
        for (si, ei, _) in kept.into_iter().skip(MAX_DUTCH_SHOTS) {
            if let Event::Camera { tilt, .. } = &mut segments[si].events[ei] {
                if tilt.is_some() {
                    *tilt = None;
                    changed += 1;
                }
            }
        }
    }
    changed
}

// ── 文字元素的**候补**守卫（2026-10-19 用户口径）────────────────────
//
// 用户的定位：文字效果**突出且指向性强**（写一句话，观众一定会读它），滥用会适得其反。
// 它应当是**候补**——只有在「没有更合适的贴纸/特效」或「其它效果已经用得太密、再叠一个会腻」
// 时才拿来换一种表达。LLM 是按段窗口生成的、数不出「这一段已经放了几张贴纸」，
// 所以互斥与配额只能在这里兜（与 `cap_pip_shots` 同一条先例：LLM 只有表达权）。

/// 文字元素与贴纸**同一时刻**时谁留下：**贴纸优先**（用户原话「尽量不要和贴纸同时出现」）。
///
/// 三条守卫（只碰文字，贴纸/运镜一个字都不动）：
/// 1. 文字窗口与任何贴纸窗口**有交叠** ⇒ 删掉文字（贴纸是具象的，文字是解释性的；
///    两个一起上就是「一句话盖在表情包上」，比只有其中一个更乱）；
/// 2. **同一段最多留一条**（留最早的那条）：一段里连刷两句字，等于把字幕又写了一遍；
/// 3. `manual` 的一律不参与（用户自己摆的文字不被后处理改写）；
/// 4. **屏蔽区间里的文字不参与**（它在这一趟里看不见，不该占名额、也不该被改写）。
///
/// ⚠️ 这是**删事件**，与「屏蔽只做标记」不冲突：那条规矩说的是**屏蔽区间**不得删事件；
/// 这里是配额/互斥守卫，与 `cap_pip_shots`（降级）、`cap_handheld_sway` / `cap_dutch_angle`
/// （剔除）同一类，而且 `manual` 有豁免。
///
/// ⚠️ **调用点成对**：`cmd_process` Step 1d2b 与 `llm/mod.rs` Step 4b（落盘前那一遍）——
/// 只改一处的话，编辑器里看到的与出片时的不一样。
/// 每段最多留几条文字元素（2026-10-20 用户口径「视觉特效可以放宽一点」：**1 → 2**）。
///
/// 只放宽**配额**：贴纸互斥、manual 豁免、屏蔽段跳过，以及 ELEMENT_MIN/MAX_SECS
/// 那套合法性一律不动。
pub const MAX_TEXT_ELEMENTS_PER_SEGMENT: usize = 2;

pub fn cap_text_elements(segments: &mut [Segment], mute_ranges: &[MuteRange]) -> usize {
    // 全片贴纸窗口（**两个载体都要收**：老 `Sticker` 与 `Element{kind:"sticker"}`）。
    let mut sticker_spans: Vec<(f64, f64)> = Vec::new();
    for seg in segments.iter() {
        for ev in &seg.events {
            // 屏蔽区间里的贴纸这一趟看不见 ⇒ 不该把可见区里的文字挤掉
            if crate::project::event_starts_muted(mute_ranges, ev) {
                continue;
            }
            if let Some((_, _, s, e)) = ev.sticker_span() {
                sticker_spans.push((s, e));
            }
        }
    }

    let mut removed = 0usize;
    for seg in segments.iter_mut() {
        let mut kept_text = 0usize;
        let old = std::mem::take(&mut seg.events);
        let mut kept: Vec<Event> = Vec::with_capacity(old.len());
        for ev in old {
            if !ev.is_text_element() || ev.is_time_manual() {
                kept.push(ev);
                continue;
            }
            // 屏蔽区间里看不见 —— 不占名额、不改写（留在文件里，撤销屏蔽后原样还在）
            if crate::project::event_starts_muted(mute_ranges, &ev) {
                kept.push(ev);
                continue;
            }
            let (s, e) = (ev.get_time(), ev.end().unwrap_or(ev.get_time() + 1.0));
            let clashes_sticker = sticker_spans.iter().any(|(ss, se)| *ss < e && *se > s);
            // 每段最多 MAX_TEXT_ELEMENTS_PER_SEGMENT 条（2026-10-20：1 → 2，配额放宽）
            if clashes_sticker || kept_text >= MAX_TEXT_ELEMENTS_PER_SEGMENT {
                removed += 1;
                continue;
            }
            kept_text += 1;
            kept.push(ev);
        }
        seg.events = kept;
    }
    removed
}

// ── 空窗补位（2026-10-20 用户口径「太平行、少内容」）──────────────────────
//
// 用户实测 clip-15：内容事件 0.13/s、段 1 有 **14 秒零内容事件**、camera 占 57%。
// prompt 已加下限（每 8 秒一个 / 空窗 ≤6 秒必须补），但 LLM 不一定听话 ——
// 这一段是**确定性兜底**：不花 token、不联网、幂等。

/// 自动补位事件的**标记**（幂等判据）。落在 `anchor` 上。
///
/// ⚠️ **为什么用标记，而不是"窗口里有没有事件"**：后者会被**自己补的那一条**满足 ——
/// 只能证明"上一次跑过"，证明不了"这一趟该不该再补"（用户删掉补的那条再跑，判据会说
/// "有事件了"）。标记是持久的、能跨文件往返的。
///
/// ⚠️ 为什么落在 `anchor` 上而不是新增字段：瞬时效（pulse…）的 `anchor` **渲染侧完全不读**
/// （只有贴纸 `stickerAnchor` 与 camera 读它），所以它既是可持久化的既有字段、又不影响任何像素，
/// 也**不用**给 `Event` 加字段（那会动 `polish.toml` 的文件格式）。
pub const AUTO_FILL_ANCHOR: &str = "auto_fill";

/// 「太久没有内容事件」的门槛（秒）。
///
/// prompt 里写的是「任何连续 ≥6 秒没有视觉事件的窗口都必须补一个」；这里的确定性兜底
/// **同口径取 6 秒** —— 它不是宽松版，而是「LLM 没做到就由程序补上」的那一半。
/// 实测目标（clip-15）：最长无内容窗口 ≤6s。
pub const VISUAL_GAP_SECS: f64 = 5.0;

/// 补位用的特效时长（瞬时档的量级，transient_duration_cap 也会再夹一次）。
const AUTO_FILL_SECS: f64 = 0.35;

/// 单段最多补几条（防跑飞：一个 60s 空心段理论上要 10 条，8 条够用）。
const MAX_AUTO_FILLS_PER_SEGMENT: usize = 8;

/// 内容事件：视觉上真的有东西 / 有声音的事件。**camera 不算** ——
/// 运镜在 process 里是全程开着的（每段至少补一条），拿它凑数等于没补。
fn is_content_event(ev: &Event) -> bool {
    !matches!(ev, Event::Camera { .. })
}

/// 是不是「我们补的」那一条。两种温和特效都带同一个标记。
fn is_auto_fill(ev: &Event) -> bool {
    match ev {
        Event::Pulse { anchor, .. } | Event::Blur { anchor, .. } => {
            anchor.as_deref() == Some(AUTO_FILL_ANCHOR)
        }
        _ => false,
    }
}

/// **空窗补位守卫**：每段内反复找「最长且 ≥VISUAL_GAP_SECS 没有内容事件」的窗口，
/// 在窗口**靠前**的位置补一个**最温和**的瞬时特效（pulse / blur 轮换）。
///
/// 为什么是「反复找 + 靠前放」而不是「每个窗口只补一条」：clip-15 实测有一个 14s 的空窗，
/// 只补中间一条会留下两个 7s 的空窗（仍 >6s）。放靠前（窗口起点 +3s 内，优先吸附到一句
/// 未屏蔽字幕的起点）能保证**收敛**：一补就把窗口劈成「≤3s + 剩余」，剩余 ≥6s 时再补一条。
///
/// 四条规矩：
/// 1. **不跨段边界**（按段扫）；
/// 2. **屏蔽区间不补**（窗口里挑不到未屏蔽字幕就跳过这个窗口）；
/// 3. **manual 事件不改写**（这里只新增，不动任何既有事件）；
/// 4. **幂等**：窗口里已经有标记事件 ⇒ 不再补。判据用 [a, b] **闭区间** ——
///    补的那条正好落在子窗口边界上，用开区间会让它看不见，第二趟又会补一条。
///
/// 返回补了几条。
pub fn fill_visual_gaps(
    segments: &mut [Segment],
    subtitles: &[Subtitle],
    mute_ranges: &[MuteRange],
) -> usize {
    let mut added = 0usize;
    for seg in segments.iter_mut() {
        let mut made = 0usize;
        loop {
            if made >= MAX_AUTO_FILLS_PER_SEGMENT {
                break;
            }
            let mut marks: Vec<(f64, bool)> = seg
                .events
                .iter()
                .filter(|e| is_content_event(e))
                .map(|e| (e.get_time(), is_auto_fill(e)))
                .collect();
            marks.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
            // 切出所有空窗，挑**最长**的那个（同长取最早）
            let mut best: Option<(f64, f64)> = None;
            let mut cursor = seg.start;
            for (t, _) in marks.iter() {
                let (a, b) = (cursor, *t);
                if b - a >= VISUAL_GAP_SECS
                    && !marks.iter().any(|(m, filled)| *filled && *m >= a && *m <= b)
                    && best.is_none_or(|(ba, bb): (f64, f64)| (b - a) > (bb - ba))
                {
                    best = Some((a, b));
                }
                cursor = cursor.max(*t);
            }
            {
                let (a, b) = (cursor, seg.end);
                if b - a >= VISUAL_GAP_SECS
                    && !marks.iter().any(|(m, filled)| *filled && *m >= a && *m <= b)
                    && best.is_none_or(|(ba, bb): (f64, f64)| (b - a) > (bb - ba))
                {
                    best = Some((a, b));
                }
            }
            let Some((a, b)) = best else { break };
            // 落点 = 窗口中点（**对半劈**保证收敛），能吸附到 ±2s 内一条未屏蔽字幕的起点就吸
            let mid = (a + b) * 0.5;
            if mute_ranges.iter().any(|r| r.overlaps(mid, mid + AUTO_FILL_SECS)) {
                // 窗口中心本身在屏蔽区里 ⇒ 这一趟不补它（跳过、继续找别的窗口没意义，直接收手）
                break;
            }
            let at = subtitles
                .iter()
                .filter(|s| !mute_ranges.iter().any(|r| r.overlaps(s.start, s.end)))
                .filter(|s| s.start >= a && s.end <= b)
                .filter(|s| (s.start - mid).abs() <= 2.0)
                .min_by(|x, y| {
                    (x.start - mid).abs().partial_cmp(&(y.start - mid).abs()).unwrap_or(std::cmp::Ordering::Equal)
                })
                .map(|s| s.start)
                .unwrap_or(mid);
            let at = at.min(b - AUTO_FILL_SECS).max(a);
            let marker = Some(AUTO_FILL_ANCHOR.to_string());
            // 轮换：交替 pulse / blur（两类都温和，也让用到的特效种类 ≥4）
            let ev = if made % 2 == 0 {
                Event::Pulse { time: at, end: Some(at + AUTO_FILL_SECS), anchor: marker, sfx: None, sfx_volume: None }
            } else {
                Event::Blur { time: at, end: Some(at + AUTO_FILL_SECS), anchor: marker, sfx: None, sfx_volume: None }
            };
            seg.events.push(ev);
            made += 1;
            added += 1;
        }
        if made > 0 {
            seg.events.sort_by(|x, y| {
                x.get_time().partial_cmp(&y.get_time()).unwrap_or(std::cmp::Ordering::Equal)
            });
        }
    }
    added
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
    /// 修掉了一份**只有重放段、没有正片时间线**的旧版 EDL（2026-10-19）
    pub repaired: bool,
}

impl MaterializedEdl {
    /// 本趟有没有真的改 EDL（`false` ⇒ 调用方**一个字节都不许写**）。
    pub fn changed(&self) -> bool {
        self.cold_open || self.recalls > 0 || self.repaired
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
/// > **`existing` 非空时只增不改**；一支意图都没有 ⇒ 返回 [`MaterializedEdl::default`]
/// > （`changed() == false`）⇒ 调用方**不写 `polish.toml`** ⇒ `edl.clips` 保持为空
/// > ⇒ 老工程逐字节零回归（R4）。
///
/// ⚠️ **`existing` 为空、但有意图要落实时，先按 `mute_ranges` 的补集把正片时间线种进去**
/// （2026-10-19）。不种的话：`clips` 一旦非空，`mute_ranges` 就**整个被忽略**（规则见上），
/// 成片只剩冷开场那几秒、而原来屏蔽掉的段**全部回来** —— 用户报的「LLM 剪辑之后屏蔽段没被
/// 过滤」就是这一条。种进去之后：`clips` = 正片补集 + 意图段，与「没有 EDL 时」的成片时间线一致。
///
/// 用户在前端手拖过的 `edl.clips` 一律**原样保留**（只在它前面插冷开场、不改它一根毫毛）。
///
/// ⚠️ **唯一例外：修复旧版的坏 EDL**（2026-10-19）。旧版落实时只写意图段、不种正片，
/// 于是存量工程里可能留着「每一段都是 `cold_open`/`recall`」的时间线 —— 那不是用户能排出来的
/// 形状（手排的段 `kind` 为空），判据安全。清空后当作「没有 EDL」重建（没活意图 ⇒ 写回空 EDL
/// ⇒ 自动退回 `mute_ranges` 补集），并在 [`MaterializedEdl::repaired`] 里报给调用方去落盘。
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
    mute_ranges: &[MuteRange],
) -> MaterializedEdl {
    // 0) **修复旧版留下的坏 EDL**（2026-10-19）。旧版的落实只写意图段、不种正片时间线，
    //    于是一支冷开场就把整条片子换成那几秒（有屏蔽时更糟：屏蔽段全部回来）——
    //    用户报的「LLM 剪辑之后屏蔽段没被过滤」很多就是这种存量工程。
    //    形状只有一种：**每一段都是意图 kind**（`cold_open` / `recall`）；用户手排/手拖的
    //    EDL 不可能长这样（`kind` 为空），所以按这个判据修不会误伤。
    //    清空之后：没有活意图 ⇒ 调用方写回空 EDL（字段被 `skip_serializing_if` 吃掉）⇒
    //    自动退回 `mute_ranges` 补集，正是正确的成片时间线。
    let only_intent_clips = !edl.is_empty()
        && edl
            .iter()
            .all(|c| matches!(c.kind.as_deref(), Some(EDL_KIND_COLD_OPEN) | Some(EDL_KIND_RECALL)));
    let mut repaired = false;
    if only_intent_clips {
        tracing::warn!(
            "  ⚠ 旧版 EDL 只有重放段、没有正片时间线（{} 段）：按屏蔽补集重建",
            edl.len()
        );
        edl.clear();
        repaired = true;
    }

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
        return MaterializedEdl { repaired, ..Default::default() };
    }

    // 4) 落实。新段一律 `speed = 1.0`（本批只做"再放一次"，变速是第 4 批的独立维度）、
    //    `source_kind`/`asset_id`/`transition_in` 留 `None`（= 本条片段、硬切）。
    let mut result = MaterializedEdl { repaired, ..Default::default() };
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


    // 4b) **一个能落实的都没有 ⇒ 一个字节都不动**（含「不往下种正片」）：
    //     `clamp_span` 失败（时长未知 / 意图短于下限）时返回 `changed() == false`，
    //     调用方据此不写盘 —— 这里如果先把正片种进 `edl`，就变成「没落实却改了时间线」。
    if cold_clip.is_none() && recall_clips.is_empty() {
        return result;
    }

    // 4c) **空 EDL 先把正片时间线种进去**（2026-10-19）。
    //
    // ⚠️ 这一步就是「LLM 剪辑有没有正确过滤屏蔽段」的判据本身：
    //     `ProjectFile::resolve_edl` 的规则 3 是「`clips` 非空 ⇒ `mute_ranges` **整个被忽略**」
    //     （规格 §2.3）。所以一支冷开场意图落到**本来没有 EDL** 的工程上时，若只写进那一条
    //     意图，正片时间线会被无声换成「只剩冷开场那几秒」，而原来屏蔽掉的段**全部回来了**
    //     —— 用户看到的正是「LLM 剪辑之后屏蔽段没被过滤」。
    //     空 EDL 的正确起点 = `mute_ranges` 的补集（即今天没有 EDL 时的成片时间线），
    //     意图只是在它前后多插一段。
    //
    // ⚠️ 只在 `edl` 为空时种：用户手拖过的 `edl.clips`（非空）**一个字节都不碰**。
    if edl.is_empty() {
        let main = crate::project::edl_clips_from_mute_complement(mute_ranges, total_duration);
        if !main.is_empty() {
            *edl = main;
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
fn fit_element_span(start: f64, end: f64, subs: &[Subtitle], mute_ranges: &[MuteRange]) -> f64 {
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
            // **不要跨进屏蔽区间**（2026-10-19）：那一段在成片里根本不存在，
            // 元素撑过去等于在半空里悬着 —— 与「不跨静音空隙」是同一条理由的另一半。
            // 判据用 `overlaps(e, s.end)`：屏蔽区间的起点落在 (e, s.end) 里就停下。
            if mute_ranges.iter().any(|r| r.overlaps(e, s.end)) {
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
pub fn align_stickers_to_subtitles(
    segments: &mut [Segment],
    subtitles: &[Subtitle],
    mute_ranges: &[MuteRange],
) -> usize {
    let mut aligned = 0usize;
    for seg in segments.iter_mut() {
        for ev in seg.events.iter_mut() {
            // 手工定过时间的贴纸不动（面板上「跟随字幕」清掉标记后才恢复自动吸附）
            if ev.is_time_manual() {
                continue;
            }
            // **屏蔽区间里的贴纸不参与**（2026-10-19）：那一段在成片里不存在，
            // 把它的时间吸到字幕上只会让它「跨进」可见区（撤销屏蔽后还会变成另一条）。
            if crate::project::event_starts_muted(mute_ranges, ev) {
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
            let fitted = fit_element_span(start, sub.end, subtitles, mute_ranges);
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

// ── 跳切手法（§17）──────────────────────────────────────────────────
//
// 规格 `camera-and-noise-spec.md` §2，**用户已拍板**：*"跳切手法可以，当前已有硬切运镜可以支持"*。
//
// ⚠️ **本仓其实"已经在跳切"了**（调研实测：三段 mute 共 17.49s，成片正好短 17.49s）——
// `mute_ranges` + `Ffmpeg::cut_ranges` 就是在做**时间轴裁剪**。
// **缺的不是能力，是"把它当节奏工具"的配额与统计。**
//
// 采用规格**方案 A**（**零新字段**）：跳切在语义上就是"接缝处的构图跳"，
// 而 `cut: true` 已经表达了"构图跳" ⇒ 不需要 `jump: true` 这种新字段。
// LLM 在接缝处写 `cut`（**表达意图**），这里识别"接缝 + cut" = 跳切（**落实 + 管配额**）。
//
// ⚠️ **这与本仓架构一致**：LLM 写意图、`process` 做确定性翻译
// （同 `anchor`→`origin`、`keyword`→贴纸名 的既有约定）。
// **别改成"后处理自己在接缝处插"** —— 那会绕开 LLM，规格明确否掉了那种写法。

/// 单段内最多允许几次跳切（规格 §2.3 的建议配额）。
///
/// 规格给这条配额的理由值得留在这里：既有守卫 `cap_pip_shots`（画中画整片 ≤1）与
/// `cap_handheld_sway` 的先例 ⇒ **跳切必须有配额**，否则"连续硬切会让画面很跳"
/// （LLM 提示词原话已经这么说）。
/// 2026-10-20 用户口径「视觉特效可以放宽一点」：**2 → 3**（只放宽配额，合法性不动）。
pub const MAX_JUMPS_PER_SEGMENT: usize = 3;

/// 相邻两次跳切至少隔这么久（秒）—— 即"全片约每 20s 一次"（规格 §2.3）。
///
/// 实现成"按时间扫、两次至少隔 [`JUMP_MIN_GAP_SECS`]"而不是"整片 ≤ 时长/20"：
/// 两者在**均匀分布**时等价，但后者依赖总时长 —— 时长未知（`duration == 0`）
/// 或极端长片下会算出一个没意义的配额。
/// 2026-10-20：**20 → 4 秒**（20 秒那档太狠 —— 78s 的片子里最多只放得下 3 次跳切，
/// 而用户要的是"更有内容"）。接缝判定、夹到段内这些合法性一律不动。
pub const JUMP_MIN_GAP_SECS: f64 = 4.0;

/// 接缝判定的容差（秒）。取 1 帧量级：`cut` 的 `time` 与 mute 边界计算路径不同，
/// 会有微小浮点差；取 0 = 只有**完全相同**才算，实际会漏掉几乎全部真实接缝。
pub const SEAM_EPS_SECS: f64 = 0.05;

/// 一个"跳切"在语法上是什么。
///
/// ⚠️ **它不是一个新效果**，是**一段接缝的标注** —— 结构上只有这两件事：
/// ① `at` 是接缝时刻（**源时间轴**，与 `mute_ranges` 同一坐标系）；② `seg` 落在哪一段。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct JumpCut {
    /// 接缝时刻（源时间轴）。
    pub at: f64,
    /// 落在 `segments` 的第几段。
    pub seg: usize,
}

/// 找出所有**跳切接缝**：落在「被剪掉的区间边界」上的硬切运镜。
///
/// 纯函数（不修改任何东西），便于先按配额决定剔谁。
///
/// 判据：某条 `cut: true` 的 camera 的 `time` 落在某个 `mute_range` 的**边界**上
/// （误差 ≤ [`SEAM_EPS_SECS`]）—— 贴 `start` 或 `end` 都算，
/// 因为"剪掉中间一段"会在**两侧各留一个接缝**。
///
/// `manual` 事件不参与（与 `cap_pip_shots` / `align_stickers_to_subtitles` 同一条规则：
/// 用户手工放的东西后处理一律不改写）。
pub fn find_jump_seams(segments: &[Segment], mute_ranges: &[MuteRange]) -> Vec<JumpCut> {
    let mut out: Vec<JumpCut> = Vec::new();
    // ⚠️ **这里有两道互相遮蔽的闸**，与 §3 `clip_quality` 那条同一形状：
    // ① 这个早期 return（没有屏蔽区间 ⇒ 不可能有接缝）；② 下面逐边界的 `on_seam` 判据。
    // 单独把①改成恒真时 `no_mute_ranges_means_no_jump_cuts` **照样绿**（②还在拦）；
    // 只有**两道一起拆**才会真的翻红（实测过）。所以变异自证要同时改这两处，
    // 否则会得到"这条用例抓不住"的**错误结论**。
    if mute_ranges.is_empty() {
        return out;
    }
    for (si, seg) in segments.iter().enumerate() {
        for ev in seg.events.iter() {
            if ev.is_time_manual() {
                continue;
            }
            let Event::Camera { time, cut, .. } = ev else { continue };
            if !*cut {
                continue;
            }
            let on_seam = mute_ranges
                .iter()
                .filter_map(|m| m.normalized())
                .any(|(s, e)| {
                    (*time - s).abs() <= SEAM_EPS_SECS || (*time - e).abs() <= SEAM_EPS_SECS
                });
            if on_seam {
                out.push(JumpCut { at: *time, seg: si });
            }
        }
    }
    // 按时间排序后处理 —— 不依赖文件顺序（`cap_pip_shots` 那条教训：段之间时间会重叠）。
    out.sort_by(|a, b| a.at.partial_cmp(&b.at).unwrap_or(std::cmp::Ordering::Equal));
    out
}

/// 跳切**配额守卫**：按规格 §2.3 剔掉超额的跳切。
///
/// 两道闸（**独立**，规格各给了一个数）：
/// ① **单段 ≤ [`MAX_JUMPS_PER_SEGMENT`]**；
/// ② **相邻两次至少隔 [`JUMP_MIN_GAP_SECS`]**。
///
/// "剔掉"= 把那条 camera 的 `cut` 改回 `false`（**降级成平滑运镜**、不是删事件）——
/// 与 `cap_pip_shots` 把「远」降级成「中」同一手法：**保留事件本身**，只收回那个"手法"资格。
/// `speed`/`anchor`/`origin` 一个字都不动。
///
/// 幂等：降级后 `cut` 为假 ⇒ 下次再跑既不会重复计数、也不会再降。
/// 返回被降级的条数。
pub fn cap_jump_cuts(
    segments: &mut [Segment],
    mute_ranges: &[MuteRange],
    _subtitles: &[Subtitle],
) -> usize {
    let seams = find_jump_seams(segments, mute_ranges);
    if seams.is_empty() {
        return 0;
    }
    // 本轮要降级的 `(seg, at)`：按 `(段, 时刻)` 精确匹配回事件
    // （同一段里可能有多条同刻的 cut，下标匹配更脆）。
    let mut demote: Vec<(usize, f64)> = Vec::new();
    let mut per_seg: std::collections::HashMap<usize, usize> = std::collections::HashMap::new();
    let mut last_kept: Option<f64> = None;
    for seam in &seams {
        let count = per_seg.entry(seam.seg).or_insert(0);
        let too_many_in_segment = *count >= MAX_JUMPS_PER_SEGMENT;
        let too_soon = last_kept.is_some_and(|prev| seam.at - prev < JUMP_MIN_GAP_SECS);
        if too_many_in_segment || too_soon {
            demote.push((seam.seg, seam.at));
            continue;
        }
        *count += 1;
        last_kept = Some(seam.at);
    }
    if demote.is_empty() {
        return 0;
    }
    let mut demoted = 0usize;
    for (si, at) in &demote {
        if let Some(seg) = segments.get_mut(*si) {
            for ev in seg.events.iter_mut() {
                if let Event::Camera { time, cut, .. } = ev {
                    if *cut && (*time - *at).abs() <= SEAM_EPS_SECS {
                        *cut = false;
                        demoted += 1;
                    }
                }
            }
        }
    }
    demoted
}

// ===== §8 叙事形状守卫（`role` 三档）=====

/// 整片**至多一个**钩子。
pub const MAX_HOOKS: usize = 1;
/// `setup` 类（这里 = "其余"）最多占整片的比例 —— 规格 §3.3 的表给的 40%。
pub const MAX_SETUP_RATIO: f64 = 0.4;

/// `role` 三档的分布（给日志/判据看）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct NarrativeShape {
    pub hook: usize,
    pub punch: usize,
    pub other: usize,
    /// 被守卫改动的段数（降级 / 提拔）。
    pub changed: usize,
}

/// **叙事形状守卫**（§8，规格 §3.3 ①）。
///
/// # 为什么必须有它
///
/// `role` 只是**给 LLM 的提示**，它不一定听话 —— 与 `cap_pip_shots` / `cap_handheld_sway`
/// 完全同一条理由：**LLM 会写错、用户也会改，守卫是唯一兜底**。
///
/// 规格给的三条规则，这里实现两条半（第三条见下面的 ⚠️）：
///
/// | 守卫 | 规则 | 这里怎么做 |
/// |---|---|---|
/// | `hook` | 至多一段（多则只留最早的） | 按**时间序**留最早那条，其余清成"其余" |
/// | `punch` | 全片至少一个（没有就把 `fx="high"` 的段提拔上来） | 没有 `punch` 时，把**时长最长**的 `fx=="high"` 段提拔成 `punch` |
/// | `setup` | 不许超过整片 40% | ❌ **不做**，见下 |
///
/// # ⚠️ 为什么不做"`setup` 不超过 40%"
///
/// 三档最小版里**没有 `setup` 这个值**（规格 §3.6 明确："只分 `hook` / `punch` / 其余三类"）。
/// 而"不写 `role`"的就是**其余** —— 它包含了铺垫、过渡、收尾，**天然就是大多数段**。
/// 拿 40% 去卡它会**必然误伤**：一条正常的片子里"其余"占 60~80% 才是对的。
///
/// ⇒ 这一条要等**六种角色**那一版才有意义（那时 `setup` 与 `afterglow`/`cta` 分得开）。
/// 硬做的话症状是：**每一趟 process 都在改段的 role**，而用户从界面上看不出为什么。
///
/// # 幂等
///
/// 跑第二趟必须是 0 改动：`hook` 已唯一、`punch` 已存在 ⇒ 两条规则都不触发。
pub fn enforce_narrative_shape(segments: &mut [crate::project::Segment]) -> NarrativeShape {
    let mut shape = NarrativeShape::default();

    // ① `hook` 至多一段：按**时间序**留最早的那条。
    //
    // ⚠️ 按时间序而不是文件顺序：段之间时间上会重叠（LLM 按窗口生成 + 去静音切段），
    // 而"最早的钩子"只有在排序之后才说得清（同 `close_pip_shots` 那条教训）。
    let mut hooks: Vec<usize> = segments
        .iter()
        .enumerate()
        .filter(|(_, s)| s.is_hook())
        .map(|(i, _)| i)
        .collect();
    hooks.sort_by(|a, b| {
        segments[*a]
            .start
            .partial_cmp(&segments[*b].start)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    if hooks.len() > MAX_HOOKS {
        // 留最早的一条（hooks[0]），其余降级成"其余"（**保留段，只清 role**）
        for &i in hooks.iter().skip(MAX_HOOKS) {
            segments[i].role.clear();
            shape.changed += 1;
        }
        shape.hook = MAX_HOOKS;
    } else {
        shape.hook = hooks.len();
    }

    // ② `punch` 至少一段：没有就把**时长最长**的 `fx == "high"` 段提拔上来。
    //
    // ⚠️ 取"时长最长"而不是"第一条 high"：短促的 high 段可能只是插了一句惊呼，
    // 而长的那一段才是真的名场面。且**并列时取时间最早的**（确定性 —— 否则
    // 同一份输入两次跑出不同结果，而 process 是每次预览/出片都隐式触发的）。
    shape.punch = segments.iter().filter(|s| s.is_punch()).count();
    if shape.punch == 0 {
        let mut best: Option<(usize, f64, f64)> = None; // (下标, 时长, 起点)
        for (i, s) in segments.iter().enumerate() {
            if s.fx != "high" {
                continue;
            }
            let len = s.end - s.start;
            if len <= 0.0 {
                continue;
            }
            let better = match best {
                None => true,
                Some((_, blen, bstart)) => {
                    len > blen + 1e-9 || ((len - blen).abs() <= 1e-9 && s.start < bstart)
                }
            };
            if better {
                best = Some((i, len, s.start));
            }
        }
        if let Some((i, _, _)) = best {
            segments[i].role = "punch".into();
            shape.punch = 1;
            shape.changed += 1;
        }
        // ⚠️ **一个 `fx == "high"` 都没有时，什么都不做**：
        //    硬提拔一条"最长段"会让一条本来平实的片子凭空长出一个"爆点"，
        //    而那个标签会往下游传（§7 会因此保护一段不该保护的停顿）。
        //    空手而归在这里是**正确答案**，不是失败。
    }

    shape.other = segments.len() - shape.hook - shape.punch;
    shape
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
            role: String::new(),
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
            role: String::new(),
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
            role: String::new(),
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
                    manual_pos: None,
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
                    speed: None,
                    tilt: None,
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
            role: String::new(),
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
                    manual_pos: None,
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
                    speed: None,
                    tilt: None,
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
            speed: None,
            tilt: None,
        }
    }


    // ── ③ 切镜必须变化一个维度（2026-10-20）─────────────────────────
    /// 造一条运镜：可以指定 cut / manual / origin / anchor。
    fn cam_film(time: f64, zoom: &str, origin: &str, anchor: Option<&str>, cut: bool, manual: bool) -> Event {
        Event::Camera {
            time,
            end: Some(time + 2.0),
            zoom: zoom.into(),
            origin: origin.into(),
            anchor: anchor.map(str::to_string),
            anchor_to: None,
            to_origin: None,
            cut,
            handheld: false,
            manual,
            speed: None,
            tilt: None,
        }
    }

    /// ★ 判据：**景别与焦点都没变的 cut 必须被降级成平滑**。
    #[test]
    fn cut_without_any_change_is_demoted() {
        let mut segs = vec![Segment {
            start: 0.0,
            end: 30.0,
            events: vec![
                cam_film(0.0, "中", "50% 50%", None, false, false),
                cam_film(5.0, "中", "50% 50%", None, true, false), // 同一景别同一焦点 ⇒ 假切镜
            ],
            fx: "low".into(),
            note: String::new(),
            role: String::new(),
            extra: Default::default(),
        }];
        let n = demote_cut_without_change(&mut segs);
        assert_eq!(n, 1, "零变化的 cut 必须被降级");
        assert!(matches!(&segs[0].events[1], Event::Camera { cut: false, .. }),
            "降级方式是把 cut 去掉（事件本身保留）");
    }

    /// 反向：**变了景别** 或 **变了焦点** 的 cut 一律保留（判据不能一刀切）。
    #[test]
    fn cut_with_a_changed_dimension_is_kept() {
        let mut segs = vec![Segment {
            start: 0.0,
            end: 30.0,
            events: vec![
                cam_film(0.0, "中", "50% 50%", None, false, false),
                cam_film(5.0, "近", "50% 50%", None, true, false), // 景别变了
                cam_film(9.0, "近", "65% 40%", None, true, false), // 焦点变了
                cam_film(13.0, "近", "65% 40%", Some("danmaku"), true, false), // 锚点变了
            ],
            fx: "low".into(),
            note: String::new(),
            role: String::new(),
            extra: Default::default(),
        }];
        assert_eq!(demote_cut_without_change(&mut segs), 0, "变了维度就不该动它");
        for i in 1..4 {
            assert!(matches!(&segs[0].events[i], Event::Camera { cut: true, .. }),
                "第 {i} 条 cut 应当保留");
        }
    }

    /// ⚠️ `manual` 不碰；第一条没有可比对象也不动；**幂等**（第二趟 0 条）。
    #[test]
    fn cut_guard_is_idempotent_and_spares_manual_and_first() {
        let mut segs = vec![Segment {
            start: 0.0,
            end: 30.0,
            events: vec![
                cam_film(0.0, "中", "50% 50%", None, true, false), // 第一条：保持
                cam_film(4.0, "中", "50% 50%", None, true, true),  // manual：保持
                cam_film(8.0, "中", "50% 50%", None, true, false), // 零变化非 manual：降级
            ],
            fx: "low".into(),
            note: String::new(),
            role: String::new(),
            extra: Default::default(),
        }];
        assert_eq!(demote_cut_without_change(&mut segs), 1);
        assert!(matches!(&segs[0].events[0], Event::Camera { cut: true, .. }), "第一条不动");
        assert!(matches!(&segs[0].events[1], Event::Camera { cut: true, .. }), "manual 不动");
        assert!(matches!(&segs[0].events[2], Event::Camera { cut: false, .. }));
        assert_eq!(demote_cut_without_change(&mut segs), 0, "幂等：第二趟不该再改");
    }

    // ── ⑧ 画中画配额：每段 1 条 / 全片 ≤3 ──────────────────────────
    #[test]
    fn pip_quota_is_per_segment_and_total_three() {
        // 4 段各 1 条 ⇒ 全片上限 3：最后一条降级成「中」。
        let mut segs: Vec<Segment> = (0..4)
            .map(|i| Segment {
                start: i as f64 * 10.0,
                end: i as f64 * 10.0 + 10.0,
                events: vec![cam(i as f64 * 10.0 + 1.0, "远")],
                fx: "low".into(),
                note: String::new(),
                role: String::new(),
                extra: Default::default(),
            })
            .collect();
        assert_eq!(cap_pip_shots(&mut segs), 1, "全片 4 条里只允许 3 条");
        let pips = segs.iter().flat_map(|s| s.events.iter())
            .filter(|e| matches!(e, Event::Camera { zoom, .. } if zoom == "远")).count();
        assert_eq!(pips, MAX_PIP_SHOTS_TOTAL);
        assert_eq!(cap_pip_shots(&mut segs), 0, "幂等");
    }

    #[test]
    fn pip_is_still_mutually_exclusive_inside_one_segment() {
        let mut segs = vec![Segment {
            start: 0.0,
            end: 20.0,
            events: vec![cam(1.0, "远"), cam(8.0, "远")],
            fx: "low".into(),
            note: String::new(),
            role: String::new(),
            extra: Default::default(),
        }];
        assert_eq!(cap_pip_shots(&mut segs), 1, "同段两条：留一条、降一条");
        assert!(matches!(&segs[0].events[1], Event::Camera { zoom, .. } if zoom == "中"));
    }
    // ── 运镜导演化（① ② ④ ⑤ ⑨，2026-10-20）──────────────────────
    fn seg_ev(start: f64, end: f64, events: Vec<Event>) -> Segment {
        Segment {
            start,
            end,
            events,
            fx: "low".into(),
            note: String::new(),
            role: String::new(),
            extra: Default::default(),
        }
    }

    /// ① 爆点段升档；**输入打平 ⇒ 全部回落「全」**（这就是变异自证的形态）。
    #[test]
    fn punch_zooms_follow_heat_and_flat_input_falls_back() {
        let mut segs = vec![
            seg_ev(0.0, 10.0, vec![cam(0.0, "全")]),
            seg_ev(10.0, 20.0, vec![cam(10.0, "全")]),
            seg_ev(20.0, 30.0, vec![cam(20.0, "全")]),
        ];
        // 中间那段的爆点分数明显最高 ⇒ 它必须是「近」，其余「全」。
        let punches = vec![(1.0, 0.2), (12.0, 0.9), (21.0, 0.2)];
        assert_eq!(assign_punch_zooms(&mut segs, &punches), 1);
        let zooms: Vec<&str> = segs.iter().map(|s| match &s.events[0] {
            Event::Camera { zoom, .. } => zoom.as_str(), _ => unreachable!() }).collect();
        assert_eq!(zooms, vec!["全", "近", "全"]);
        assert_eq!(assign_punch_zooms(&mut segs, &punches), 0, "幂等");
        // 打平（**全 0.5**，不是全 0 —— 全 0 会被 mean<=1e-9 那条顺带兜住，测不到这条契约）⇒ 退回默认档。
        let flat = vec![(1.0, 0.5), (12.0, 0.5), (21.0, 0.5)];
        assign_punch_zooms(&mut segs, &flat);
        for s in &segs {
            assert!(matches!(&s.events[0], Event::Camera { zoom, .. } if zoom == "全"),
                "热度打平 ⇒ 必须回落默认档「全」");
        }
    }

    /// ② 长段必须被切分（>6s 无变化 = 0）；且幂等。
    #[test]
    fn micro_push_fills_static_windows_and_is_idempotent() {
        let mut segs = vec![seg_ev(0.0, 20.0, vec![cam(0.0, "全")])];
        let n = add_micro_push(&mut segs, &[]);
        assert!(n >= 2, "20s 的段至少要补两拍（实得 {n}）");
        let mut times: Vec<f64> = segs[0].events.iter()
            .filter_map(|e| match e { Event::Camera { time, .. } => Some(*time), _ => None }).collect();
        times.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let max_gap = times.windows(2).map(|w| w[1] - w[0]).fold(0.0_f64, f64::max);
        assert!(max_gap <= 6.0 + 1e-9, "切分后最长静止窗口 {max_gap}s 应 ≤6s");
        assert_eq!(add_micro_push(&mut segs, &[]), 0, "幂等：第二趟一条都不加");
    }

    /// ⑤ 段首吃掉 cut、段尾补一条收束；只动边界，段中不加。
    #[test]
    fn breathing_touches_boundaries_only_and_is_idempotent() {
        let mut segs = vec![seg_ev(0.0, 10.0, vec![
            Event::Camera { time: 0.0, end: Some(10.0), zoom: "中".into(),
                origin: "50% 50%".into(), anchor: None, anchor_to: None, to_origin: None,
                cut: true, handheld: false, manual: false, speed: None, tilt: None },
        ])];
        let n = segment_breathing(&mut segs);
        assert!(n >= 2, "段首去 cut + 段尾收束（实得 {n}）");
        assert!(matches!(&segs[0].events[0], Event::Camera { cut: false, .. }), "段首不再硬切");
        let tail = segs[0].events.iter().filter_map(|e| match e {
            Event::Camera { time, .. } if *time > 9.0 => Some(*time), _ => None }).count();
        assert_eq!(tail, 1, "段尾恰好一条收束");
        assert_eq!(segment_breathing(&mut segs), 0, "幂等");
    }

    /// ④ 每个 cut/snap 起点 0.3s 内恰好一条 sfx，且不会重复补。
    #[test]
    fn camera_sfx_is_synced_once() {
        let mut segs = vec![seg_ev(0.0, 10.0, vec![
            Event::Camera { time: 2.0, end: Some(4.0), zoom: "近".into(),
                origin: "50% 50%".into(), anchor: None, anchor_to: None, to_origin: None,
                cut: true, handheld: false, manual: false, speed: None, tilt: None },
        ])];
        assert_eq!(sync_camera_sfx(&mut segs), 1);
        let sfx: Vec<f64> = segs[0].events.iter().filter_map(|e| match e {
            Event::Sfx { time, .. } => Some(*time), _ => None }).collect();
        assert_eq!(sfx, vec![2.0], "音效必须落在运镜起点上");
        assert_eq!(sync_camera_sfx(&mut segs), 0, "幂等");
    }

    /// ⑨ 竖屏只动 y（x 恒 50%），且幂等。
    #[test]
    fn vertical_portrait_only_moves_the_vertical_axis() {
        let mut segs = vec![
            seg_ev(0.0, 10.0, vec![cam(0.0, "中")]),
            seg_ev(10.0, 20.0, vec![cam(10.0, "中")]),
        ];
        assert_eq!(direct_vertical_portrait(&mut segs), 2);
        for (i, s) in segs.iter().enumerate() {
            match &s.events[0] {
                Event::Camera { origin, zoom, .. } => {
                    assert!(origin.starts_with("50% "), "x 必须恒为 50%（竖屏横摇必露边）：{origin}");
                    let want_y = if i % 2 == 0 { "50% 42%" } else { "50% 58%" };
                    assert_eq!(origin, want_y);
                    assert!(zoom == "全" || zoom == "微", "竖屏只用全/微两档：{zoom}");
                }
                _ => unreachable!(),
            }
        }
        assert_eq!(direct_vertical_portrait(&mut segs), 0, "幂等");
    }

    /// 带 `anchor`（可空）的运镜 —— 验特写焦点守卫用。
    fn cam_a(time: f64, zoom: &str, anchor: Option<&str>, anchor_to: Option<&str>) -> Event {
        let Event::Camera {
            time,
            end,
            zoom: z,
            origin,
            cut,
            handheld,
            manual,
            speed,
            tilt,
            ..
        } = cam(time, zoom)
        else {
            unreachable!()
        };
        Event::Camera {
            time,
            end,
            zoom: z,
            origin,
            anchor: anchor.map(str::to_string),
            anchor_to: anchor_to.map(str::to_string),
            to_origin: None,
            cut,
            handheld,
            manual,
            speed,
            tilt,
        }
    }

    fn seg_with(events: Vec<Event>) -> Segment {
        Segment {
            start: 0.0,
            end: 30.0,
            fx: "med".into(),
            note: String::new(),
            role: String::new(),
            events,
            extra: Default::default(),
        }
    }

    /// 带荷兰角（`tilt`）的运镜。
    fn cam_t(time: f64, zoom: &str, tilt: f64) -> Event {
        let Event::Camera {
            time,
            end,
            zoom: z,
            origin,
            anchor,
            anchor_to,
            to_origin,
            cut,
            handheld,
            manual,
            speed,
            ..
        } = cam(time, zoom)
        else {
            unreachable!()
        };
        Event::Camera {
            time,
            end,
            zoom: z,
            origin,
            anchor,
            anchor_to,
            to_origin,
            cut,
            handheld,
            manual,
            speed,
            tilt: Some(tilt),
        }
    }

    fn segs_of(times_zooms: &[(f64, &str)]) -> Vec<Segment> {
        // 一条段里放全部事件即可（守卫按时间排序遍历，跟段边界无关）
        vec![Segment {
            start: 0.0,
            end: 600.0,
            fx: "med".into(),
            note: String::new(),
            role: String::new(),
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
            speed: None,
            tilt: None,
        };
        for to_origin in [Some("70% 30%"), None] {
            let mut segs = vec![Segment {
                start: 0.0,
                end: 60.0,
                fx: "med".into(),
                note: String::new(),
                role: String::new(),
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
            role: String::new(),
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
                role: String::new(),
                events: vec![cam(0.0, "中"), cam(20.0, "远")],
                extra: Default::default(),
            },
            Segment {
                start: 40.0,
                end: 60.0,
                fx: "med".into(),
                note: String::new(),
                role: String::new(),
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
            role: String::new(),
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
            manual_pos: None,
        }
    }

    fn sticker_seg(ev: Event) -> Vec<Segment> {
        vec![Segment {
            start: 0.0,
            end: 60.0,
            fx: "med".into(),
            note: String::new(),
            role: String::new(),
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
        assert_eq!(align_stickers_to_subtitles(&mut segs, &subs, &[]), 1);
        assert_eq!(sticker_span(&segs), (10.0, 12.5));

        // 起点落在两条之间的空隙、离后一条更近（13.8 → 14.0 差 0.2，离前一条 1.3）→ 吸到后一条
        let mut segs = sticker_seg(sticker(13.8, 2.0));
        assert_eq!(align_stickers_to_subtitles(&mut segs, &subs, &[]), 1);
        assert_eq!(sticker_span(&segs), (14.0, 16.0));

        // 已经对齐 → 不动
        let mut segs = sticker_seg(sticker(14.0, 2.0));
        assert_eq!(align_stickers_to_subtitles(&mut segs, &subs, &[]), 0);

        // 离最近字幕超过 2s（纯视觉反应）→ 保持原位
        let mut segs = sticker_seg(sticker(20.0, 2.0));
        assert_eq!(align_stickers_to_subtitles(&mut segs, &subs, &[]), 0);
        assert_eq!(sticker_span(&segs), (20.0, 22.0));

        // 很碎的字幕（0.1s）**不再跳过，而是被最短时长规矩撑开**（2026-10-11 改）：
        // 旧行为是"盖不住就跳过"，但那样最该被治的碎片反而没人管。
        // 这里只有一条字幕、后面没有可吞的，于是落在它自己的结尾上（见
        // `element_accepts_short_span_when_no_more_subtitles`）。
        let short = vec![sub(1, 30.0, 30.1)];
        let mut segs = sticker_seg(sticker(30.05, 2.0));
        assert_eq!(align_stickers_to_subtitles(&mut segs, &short, &[]), 1);
        assert_eq!(sticker_span(&segs), (30.0, 30.1));

        // 没有字幕 → 原样
        let mut segs = sticker_seg(sticker(5.0, 2.0));
        assert_eq!(align_stickers_to_subtitles(&mut segs, &[], &[]), 0);
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
        align_stickers_to_subtitles(&mut segs, &subs, &[]);
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
        align_stickers_to_subtitles(&mut segs, &tight, &[]);
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
        assert_eq!(align_stickers_to_subtitles(&mut segs, &subs, &[]), 1);
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
        align_stickers_to_subtitles(&mut segs, &subs, &[]);
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
            align_stickers_to_subtitles(&mut segs, &subs, &[]);
            let first = sticker_span(&segs);
            let n = align_stickers_to_subtitles(&mut segs, &subs, &[]);
            assert_eq!(sticker_span(&segs), first, "第二轮不该再变（t={t}）");
            assert_eq!(n, 0, "第二轮不该再报对齐条数（t={t}）");
        }
    }

    // ── 文字元素的**候补**守卫（2026-10-19 用户口径）──────────────────
    //
    // 用户对文字效果的定位：突出、指向性强 ⇒ 只当**候补**（没有更合适的贴纸/特效时才用），
    // 而且**不要和贴纸同时出现**。LLM 在段窗口里数不出这些，所以由 `cap_text_elements` 兜。
    /// 造一条文字元素 —— 走**真**反序列化（`type = "element"` 就是盘上的写法）。
    fn text_el(time: f64, dur: f64, s: &str) -> Event {
        toml::from_str(&format!(
            "type = \"element\"\ntime = {time}\nend = {}\nkind = \"text\"\ntext = \"{s}\"\nstyle = \"comment\"\n",
            time + dur
        ))
        .expect("text element")
    }

    /// 编辑器「贴纸轨」新建的那一支（`Element{kind:"sticker"}`）—— 守卫也必须认它。
    fn element_sticker_el(time: f64, dur: f64) -> Event {
        toml::from_str(&format!(
            "type = \"element\"\ntime = {time}\nend = {}\nkind = \"sticker\"\ntext = \"emoji\"\n",
            time + dur
        ))
        .expect("element sticker")
    }

    fn text_seg(events: Vec<Event>) -> Vec<Segment> {
        vec![Segment {
            start: 0.0,
            end: 60.0,
            fx: "med".into(),
            note: String::new(),
            role: String::new(),
            events,
            extra: Default::default(),
        }]
    }

    fn text_bodies(segs: &[Segment]) -> Vec<String> {
        segs.iter()
            .flat_map(|s| s.events.iter())
            .filter_map(|e| match e {
                Event::Element { kind, text, .. } if kind == "text" => Some(text.clone()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn text_element_yields_to_sticker() {
        // ① 老载体 `Event::Sticker`
        let mut segs = text_seg(vec![text_el(2.0, 2.0, "名场面"), sticker(3.0, 1.5)]);
        assert_eq!(cap_text_elements(&mut segs, &[]), 1, "与贴纸时间重叠 ⇒ 文字让位");
        assert!(text_bodies(&segs).is_empty(), "文字被删掉，贴纸留下");
        assert_eq!(segs[0].events.len(), 1, "贴纸一条都不许动");
        // ② 编辑器新建的 `Element{kind:"sticker"}` —— 只认老载体的话这里会漏
        let mut segs = text_seg(vec![text_el(2.0, 2.0, "名场面"), element_sticker_el(2.5, 1.0)]);
        assert_eq!(cap_text_elements(&mut segs, &[]), 1, "两个载体都要认");
        assert!(text_bodies(&segs).is_empty());
    }

    /// 每段最多留 MAX_TEXT_ELEMENTS_PER_SEGMENT 条（2026-10-20：**1 → 2**）。
    ///
    /// 变异自证：把常量改回 1 ⇒ 本条红（会只剩"第一条"）。
    #[test]
    fn text_element_is_capped_per_segment() {
        let mut segs = text_seg(vec![
            text_el(2.0, 1.0, "第一条"),
            text_el(10.0, 1.0, "第二条"),
            text_el(20.0, 1.0, "第三条"),
        ]);
        assert_eq!(cap_text_elements(&mut segs, &[]), 1, "一段最多留两条");
        assert_eq!(
            text_bodies(&segs),
            vec!["第一条".to_string(), "第二条".to_string()],
            "留最早的两条"
        );
        assert_eq!(cap_text_elements(&mut segs, &[]), 0, "幂等（process 会被隐式重跑）");
    }

    #[test]
    fn manual_text_element_is_never_touched() {
        let mut manual = text_el(2.0, 2.0, "用户写的");
        manual.set_time_manual(true);
        let mut segs = text_seg(vec![manual, sticker(2.5, 1.0)]);
        assert_eq!(cap_text_elements(&mut segs, &[]), 0, "manual 不参与守卫");
        assert_eq!(text_bodies(&segs), vec!["用户写的".to_string()]);
    }

    #[test]
    fn muted_text_element_does_not_take_the_slot() {
        // 0~10s 被屏蔽：里面的文字看不见 —— 不该被删，也不该挤掉可见区里的名额
        let mutes = [MuteRange { start: 0.0, end: 10.0 }];
        let mut segs = text_seg(vec![text_el(2.0, 1.0, "看不见"), text_el(20.0, 1.0, "看得见")]);
        assert_eq!(cap_text_elements(&mut segs, &mutes), 0);
        assert_eq!(
            text_bodies(&segs),
            vec!["看不见".to_string(), "看得见".to_string()]
        );

        // 屏蔽区里的**贴纸**同样看不见 —— 不许把可见区里的文字挤掉
        let mut segs = text_seg(vec![sticker(2.0, 1.0), text_el(20.0, 1.0, "看得见")]);
        assert_eq!(cap_text_elements(&mut segs, &mutes), 0, "屏蔽段的贴纸不算障碍");
        assert_eq!(text_bodies(&segs), vec!["看得见".to_string()]);
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
        assert_eq!(align_stickers_to_subtitles(&mut segs, &subs, &[]), 0, "manual 不该被吸附");
        assert_eq!(sticker_span(&segs), (10.4, 11.9), "时间原样保留");

        // 清掉标记 → 恢复自动吸附（面板「跟随字幕」走的就是这条）
        segs[0].events[0].set_time_manual(false);
        assert_eq!(align_stickers_to_subtitles(&mut segs, &subs, &[]), 1);
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
            start: 8.0, end: 20.0, fx: "med".into(), note: String::new(), role: String::new(), events: vec![ev], extra: Default::default(),
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
            vec![Segment { start: 8.0, end: 20.0, fx: "med".into(), note: String::new(), events: vec![ev], role: String::new(), extra: Default::default() }]
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
            role: String::new(),
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
            role: String::new(),
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
            role: String::new(),
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
            role: String::new(),
            start: 0.0, end: 60.0, fx: "med".into(), note: String::new(),
            events: vec![cam_h(0.0, "近", true), cam(10.0, "远")],
            extra: Default::default(),
        }];
        assert_eq!(close_pip_shots(&mut segs), 1);
        assert_eq!(handheld_flags(&segs), vec![true, false, true], "回镜头承接前一条的手持摇摆");

        // 回到非近景（前一条被手改/LLM 写成中景 + handheld）：回镜头也承接，
        // 但两条中景都会被 cap_handheld_sway 剥掉 —— cmd_process 里守卫就排在收工之后
        let mut segs = vec![Segment {
            role: String::new(),
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
            speed: None,
            tilt: None,
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
                role: String::new(),
                events: vec![intent_cam(2.0, 6.0, INTENT_COLD_OPEN)], extra: Default::default() },
            Segment { start: 20.0, end: 40.0, fx: "med".into(), note: String::new(),
                role: String::new(),
                events: vec![intent_cam(22.0, 26.0, INTENT_COLD_OPEN)], extra: Default::default() },
            Segment { start: 40.0, end: 60.0, fx: "med".into(), note: String::new(),
                role: String::new(),
                events: vec![intent_cam(42.0, 46.0, INTENT_COLD_OPEN)], extra: Default::default() },
        ];
        let mut clips: Vec<EdlClip> = Vec::new();
        let m = materialize_edl_intents(&mut segs, &mut clips, 60.0, &[]);
        assert_eq!(m.recalls, 0, "这一趟一条回忆回溯都没有");
        assert!(m.cold_open, "有一条活下来");
        let cold: Vec<&EdlClip> = clips
            .iter()
            .filter(|c| c.kind.as_deref() == Some(EDL_KIND_COLD_OPEN))
            .collect();
        assert_eq!(cold.len(), 1, "3 个候选 ⇒ 只剩 1 个：{clips:?}");
        assert_eq!((cold[0].source_start, cold[0].source_end), (2.0, 6.0),
            "留下的是**最早**那条（与 cap_pip_shots 留最早一条「远」同规则）");
        // ⚠️ 空 EDL 的起点 = `mute_ranges` 补集（这里没有屏蔽 ⇒ 整条正片一段）。
        // 不种的话 `clips` 就只有冷开场那几秒 —— 正片时间线**整条消失**，
        // 而 `mute_ranges` 也会随着 EDL 非空被整个忽略（用户报的「屏蔽段没被过滤」）。
        assert_eq!(clips.len(), 2, "冷开场 + 正片补集：{clips:?}");
        assert_eq!((clips[1].source_start, clips[1].source_end), (0.0, 60.0));
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
            role: String::new(),
            start: 0.0, end: 30.0, fx: "med".into(), note: String::new(),
            events: vec![
                intent_cam(2.0, 6.0, INTENT_COLD_OPEN),
                intent_cam(12.0, 16.0, INTENT_RECALL),
                intent_cam(20.0, 24.0, INTENT_RECALL),
            ],
            extra: Default::default(),
        }];
        let mut clips: Vec<EdlClip> = Vec::new();

        let first = materialize_edl_intents(&mut segs, &mut clips, 30.0, &[]);
        assert!(first.cold_open);
        assert_eq!(first.recalls, 2);
        // 冷开场插在最前、**正片补集紧跟其后**（无屏蔽 ⇒ 整条一段），回忆回溯追加在末尾
        assert_eq!(
            kinds(&clips),
            vec![Some(EDL_KIND_COLD_OPEN), None, Some(EDL_KIND_RECALL), Some(EDL_KIND_RECALL)]
        );
        let after_first = clips.clone();

        // 第二趟：没有任何意图了 ⇒ 一个字节都不加，`changed()` 必须为假
        let second = materialize_edl_intents(&mut segs, &mut clips, 30.0, &[]);
        assert!(!second.changed(), "第二趟不许再落实任何东西");
        assert_eq!(clips, after_first, "EDL 逐值不变（不叠加）");

        // 第三趟以防万一（幂等要能反复成立，不是"两次碰巧一样"）
        let third = materialize_edl_intents(&mut segs, &mut clips, 30.0, &[]);
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
        let m = materialize_edl_intents(&mut segs, &mut clips, 60.0, &[]);
        assert!(!m.changed(), "没有意图 ⇒ 不许改 EDL");
        assert!(clips.is_empty());
        let cams = cam_spans(&segs);
        assert_eq!(cams.len(), 3, "普通运镜一条都不许动");

        // 别的 anchor 也不许被误当成意图、更不许被"顺手清掉"（只清自己认得的两个值）
        let mut segs = vec![Segment {
            role: String::new(),
            start: 0.0, end: 30.0, fx: "med".into(), note: String::new(),
            events: vec![Event::Camera {
                time: 0.0, end: Some(5.0), zoom: "中".into(), origin: "50% 50%".into(),
                anchor: Some("liver".into()), anchor_to: None, to_origin: None,
                cut: false, handheld: false, manual: false, speed: None,
                tilt: None,
            }], extra: Default::default() }];
        let before: Vec<Option<String>> = segs.iter().flat_map(|s| s.events.iter())
            .filter_map(|e| match e { Event::Camera { anchor, .. } => Some(anchor.clone()), _ => None })
            .collect();
        let mut clips: Vec<EdlClip> = Vec::new();
        assert!(!materialize_edl_intents(&mut segs, &mut clips, 30.0, &[]).changed());
        let after: Vec<Option<String>> = segs.iter().flat_map(|s| s.events.iter())
            .filter_map(|e| match e { Event::Camera { anchor, .. } => Some(anchor.clone()), _ => None })
            .collect();
        assert_eq!(before, after, "不认得的 anchor 原样留着");

        // 用户手拖过的 EDL：**原样保留**，只在最前面插冷开场
        let mut segs = vec![Segment {
            role: String::new(),
            start: 0.0, end: 30.0, fx: "med".into(), note: String::new(),
            events: vec![intent_cam(5.0, 9.0, INTENT_COLD_OPEN)], extra: Default::default() }];
        let mut clips = vec![clip_of(0.0, 30.0, Some("main"))];
        let m = materialize_edl_intents(&mut segs, &mut clips, 30.0, &[]);
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
            role: String::new(),
            start: 0.0, end: 30.0, fx: "med".into(), note: String::new(),
            events: vec![ev], extra: Default::default() }];
        let mut clips: Vec<EdlClip> = Vec::new();
        let m = materialize_edl_intents(&mut segs, &mut clips, 30.0, &[]);
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
            role: String::new(),
            events: vec![intent_cam(2.0, 6.0, INTENT_COLD_OPEN)], extra: Default::default() }];
        let mut clips: Vec<EdlClip> = Vec::new();
        assert!(!materialize_edl_intents(&mut segs, &mut clips, 0.0, &[]).changed(), "时长未知 ⇒ 不落实");
        assert!(clips.is_empty());

        // 窗口短于 MIN_COLD_OPEN_SECS：
        let mut segs = vec![Segment { start: 0.0, end: 30.0, fx: "med".into(), note: String::new(),
            role: String::new(),
            events: vec![intent_cam(2.0, 2.4, INTENT_COLD_OPEN)], extra: Default::default() }];
        assert!(!materialize_edl_intents(&mut segs, &mut clips, 30.0, &[]).changed(), "太短 ⇒ 不落实");
        assert!(clips.is_empty());

        // 端点夹到 [0, total_duration]：起点为负、终点越界都夹回来
        let mut segs = vec![Segment { start: 0.0, end: 30.0, fx: "med".into(), note: String::new(),
            role: String::new(),
            events: vec![intent_cam(-3.0, 99.0, INTENT_COLD_OPEN)], extra: Default::default() }];
        assert!(materialize_edl_intents(&mut segs, &mut clips, 30.0, &[]).cold_open);
        assert_eq!((clips[0].source_start, clips[0].source_end), (0.0, 30.0), "夹进 [0, 总时长]");
    }

    /// **屏蔽段必须被过滤**（2026-10-19 用户报「LLM 剪辑之后屏蔽段没被过滤」）。
    ///
    /// 判据落在**空 EDL + 一支意图**这个组合上：`clips` 一旦非空，`mute_ranges` 就整个被
    /// 忽略（`ProjectFile::resolve_edl` 规则 3）。所以落实意图时必须先把**正片补集**种进去，
    /// 否则成片只剩下冷开场那几秒（正片消失）+ 原来屏蔽掉的段**全部回来**。
    #[test]
    fn cold_open_on_empty_edl_seeds_the_mute_complement() {
        let mut segs = vec![Segment {
            role: String::new(),
            start: 0.0, end: 10.0, fx: "med".into(), note: String::new(),
            events: vec![intent_cam(5.0, 7.0, INTENT_COLD_OPEN)],
            extra: Default::default(),
        }];
        let mutes = vec![MuteRange { start: 2.0, end: 4.0 }];
        let mut clips: Vec<EdlClip> = Vec::new();
        let m = materialize_edl_intents(&mut segs, &mut clips, 10.0, &mutes);
        assert!(m.cold_open);
        let got: Vec<(f64, f64, Option<&str>)> = clips
            .iter()
            .map(|c| (c.source_start, c.source_end, c.kind.as_deref()))
            .collect();
        assert_eq!(
            got,
            vec![
                (5.0, 7.0, Some(EDL_KIND_COLD_OPEN)),
                (0.0, 2.0, None),
                (4.0, 10.0, None),
            ],
            "冷开场插最前 + 正片 = 屏蔽补集（[2,4] 被剪掉）：{clips:?}"
        );
    }

    /// 用户手拖过的 EDL **非空**时，一个字节都不许被补集覆盖（只在最前面插冷开场）。
    #[test]
    fn non_empty_edl_is_never_reseeded() {
        let mut segs = vec![Segment {
            role: String::new(),
            start: 0.0, end: 10.0, fx: "med".into(), note: String::new(),
            events: vec![intent_cam(5.0, 7.0, INTENT_COLD_OPEN)],
            extra: Default::default(),
        }];
        let mut clips = vec![clip_of(1.0, 9.0, Some("main"))];
        let mutes = vec![MuteRange { start: 2.0, end: 4.0 }];
        let m = materialize_edl_intents(&mut segs, &mut clips, 10.0, &mutes);
        assert!(m.cold_open);
        assert_eq!(
            kinds(&clips),
            vec![Some(EDL_KIND_COLD_OPEN), Some("main")],
            "用户那条原样保留，屏蔽补集**不参与**（EDL 非空时 mute_ranges 本来就被忽略）"
        );
        assert_eq!((clips[1].source_start, clips[1].source_end), (1.0, 9.0));
    }
    /// **旧版留下的坏 EDL**（只有重放段、没有正片）必须被修掉（2026-10-19）——
    /// 不修的话这种存量工程永远是「整片只剩那几秒 + 屏蔽段全部回来」。
    #[test]
    fn intent_only_edl_from_old_version_is_rebuilt() {
        // 意图早被旧版清掉了（`anchor` 已空）⇒ 这一趟没有活意图，但坏形状要认出来
        let mut segs = vec![Segment {
            role: String::new(),
            start: 0.0, end: 10.0, fx: "med".into(), note: String::new(),
            events: vec![], extra: Default::default(),
        }];
        let mut clips = vec![clip_of(5.0, 7.0, Some(EDL_KIND_COLD_OPEN))];
        let m = materialize_edl_intents(&mut segs, &mut clips, 10.0, &[]);
        assert!(m.changed(), "修好也是一次写盘（把 `[edl]` 整段去掉）");
        assert!(m.repaired);
        assert!(clips.is_empty(), "清空 ⇒ 写回空 EDL ⇒ 自动退回屏蔽补集：{clips:?}");
    }

    /// 坏 EDL + 新意图：先修成正片补集，再把意图插进去（顺序与没有 EDL 时一致）。
    #[test]
    fn intent_only_edl_is_repaired_then_seeded() {
        let mut segs = vec![Segment {
            role: String::new(),
            start: 0.0, end: 10.0, fx: "med".into(), note: String::new(),
            events: vec![intent_cam(5.0, 7.0, INTENT_RECALL)],
            extra: Default::default(),
        }];
        let mutes = vec![MuteRange { start: 2.0, end: 4.0 }];
        let mut clips = vec![clip_of(8.0, 9.0, Some(EDL_KIND_COLD_OPEN))];
        let m = materialize_edl_intents(&mut segs, &mut clips, 10.0, &mutes);
        assert!(m.repaired);
        assert_eq!(m.recalls, 1);
        let got: Vec<(f64, f64, Option<&str>)> = clips
            .iter()
            .map(|c| (c.source_start, c.source_end, c.kind.as_deref()))
            .collect();
        assert_eq!(
            got,
            vec![(0.0, 2.0, None), (4.0, 10.0, None), (5.0, 7.0, Some(EDL_KIND_RECALL))],
            "正片补集在前、回忆回溯追加在末尾：{clips:?}"
        );
    }

    /// **混合 EDL 一律不修**（用户手排的段 `kind` 为空，形状永远不是「全是意图段」）。
    #[test]
    fn mixed_edl_is_never_treated_as_broken() {
        let mut segs = vec![Segment {
            role: String::new(),
            start: 0.0, end: 10.0, fx: "med".into(), note: String::new(),
            events: vec![], extra: Default::default(),
        }];
        let mut clips = vec![
            clip_of(5.0, 7.0, Some(EDL_KIND_COLD_OPEN)),
            clip_of(0.0, 10.0, Some("main")),
        ];
        let before = clips.clone();
        let m = materialize_edl_intents(&mut segs, &mut clips, 10.0, &[]);
        assert!(!m.changed(), "没有意图、也不是坏形状 ⇒ 一个字节都不许动");
        assert_eq!(clips, before);
    }
    /// 回忆回溯上限是 [`MAX_RECALL`]（比冷开场松一档，但也是有上限的）。
    #[test]
    fn recall_is_capped() {
        let events: Vec<Event> = (0..5).map(|i| intent_cam(i as f64 * 10.0, i as f64 * 10.0 + 3.0, INTENT_RECALL)).collect();
        let mut segs = vec![Segment { start: 0.0, end: 60.0, fx: "med".into(), note: String::new(),
            role: String::new(),
            events, extra: Default::default() }];
        let mut clips: Vec<EdlClip> = Vec::new();
        let m = materialize_edl_intents(&mut segs, &mut clips, 60.0, &[]);
        assert_eq!(m.recalls, MAX_RECALL, "5 个候选 ⇒ 只剩 MAX_RECALL 个");
        let recall_clips: Vec<&EdlClip> = clips
            .iter()
            .filter(|c| c.kind.as_deref() == Some(EDL_KIND_RECALL))
            .collect();
        assert_eq!(recall_clips.len(), MAX_RECALL);
        // 留最早的几条
        let starts: Vec<f64> = recall_clips.iter().map(|c| c.source_start).collect();
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
            role: String::new(),
            events: Vec::new(),
            extra: Default::default(),
        }
    }

    // ── §8 叙事形状守卫 ────────────────────────────────────────────

    fn nseg(start: f64, end: f64, fx: &str, role: &str) -> Segment {
        Segment {
            start,
            end,
            fx: fx.into(),
            note: String::new(),
            role: role.into(),
            events: Vec::new(),
            extra: Default::default(),
        }
    }

    /// ① `hook` **至多一段**（多则只留最早的），且**保留段本身**（只清 role）。
    ///
    /// ⚠️ "只清 role、不删段"是刻意的：段是**时间轴结构**，删掉会让整条片子的
    /// 分段结果变样；而 `role` 只是一个标签。守卫的职责是**修标签**，不是改结构。
    #[test]
    fn only_one_hook_survives() {
        let mut segs = vec![
            nseg(30.0, 40.0, "med", "hook"), // 时间上第二条
            nseg(0.0, 10.0, "med", "hook"),  // 最早 —— 这条要留下
            nseg(50.0, 60.0, "med", "hook"),
            nseg(10.0, 20.0, "med", "punch"),
        ];
        let shape = enforce_narrative_shape(&mut segs);
        assert_eq!(shape.hook, 1, "hook 只该留一条");
        assert_eq!(shape.punch, 1, "已有的 punch 不许被碰");
        assert_eq!(shape.changed, 2, "改动的是多余的两条");
        assert_eq!(segs.len(), 4, "段不许被删（只清标签）");
        assert!(segs[1].is_hook(), "留下的是**时间最早**那条（下标 1，start=0.0）");
        assert!(!segs[0].is_hook() && !segs[2].is_hook(), "其余两条已降级");
    }

    /// ② 一段 `punch` 都没有时，把**时长最长**的 `fx=="high"` 段提拔上来。
    #[test]
    fn longest_high_segment_is_promoted_when_no_punch_exists() {
        let mut segs = vec![
            nseg(0.0, 3.0, "high", ""),   // 短促的惊呼
            nseg(10.0, 30.0, "med", ""),  // 时长最长但**不是** high ⇒ 不许提拔
            nseg(40.0, 55.0, "high", ""), // 15s，最长的 high ⇒ 这条
            nseg(60.0, 62.0, "high", ""),
        ];
        let shape = enforce_narrative_shape(&mut segs);
        assert_eq!(shape.punch, 1);
        assert_eq!(shape.changed, 1);
        assert!(segs[2].is_punch(), "提拔的应当是最长的 high 段: {segs:?}");
        assert!(!segs[1].is_punch(), "med 段不该被提拔成爆点");
    }

    /// ③ ⚠️ **一个 `fx == "high"` 都没有 ⇒ 什么都不做**（不许凭空造一个爆点）。
    ///
    /// 硬提拔一条"最长段"的后果是**往下游传**：§7 会因此保护一段不该保护的停顿，
    /// 而用户从任何界面都看不出那个标签是哪来的。
    #[test]
    fn no_high_segment_means_no_punch_is_invented() {
        let mut segs = vec![nseg(0.0, 30.0, "low", ""), nseg(30.0, 60.0, "med", "")];
        let shape = enforce_narrative_shape(&mut segs);
        assert_eq!(shape.punch, 0, "不该凭空造爆点");
        assert_eq!(shape.changed, 0);
        assert!(segs.iter().all(|s| s.role().is_empty()));
    }

    /// ④ **幂等**：跑第二趟必须 0 改动。
    ///
    /// ⚠️ `process` 是**每次预览/出片都被隐式触发**的 —— 不幂等的守卫会每跑一次
    /// 就改一次，而用户看到的是"role 自己在变"。
    #[test]
    fn narrative_shape_is_idempotent() {
        let mut segs = vec![
            nseg(0.0, 10.0, "med", "hook"),
            nseg(10.0, 20.0, "med", "hook"),
            nseg(20.0, 40.0, "high", ""),
        ];
        let first = enforce_narrative_shape(&mut segs);
        assert!(first.changed > 0, "第一趟应当有改动");
        let second = enforce_narrative_shape(&mut segs);
        assert_eq!(second.changed, 0, "第二趟必须收敛: {second:?}");
        assert_eq!(second.hook, 1);
        assert_eq!(second.punch, 1);
        let third = enforce_narrative_shape(&mut segs);
        assert_eq!(third, second, "再跑也不许变");
    }

    /// ⑤ 已经有 `punch` 时**不许再提拔**别的段（否则每趟都会多一个爆点）。
    #[test]
    fn existing_punch_is_never_added_to() {
        let mut segs = vec![
            nseg(0.0, 10.0, "high", "punch"),
            nseg(10.0, 40.0, "high", ""), // 更长，但不该被提拔
        ];
        let shape = enforce_narrative_shape(&mut segs);
        assert_eq!(shape.punch, 1);
        assert_eq!(shape.changed, 0);
        assert!(segs[0].is_punch() && !segs[1].is_punch());
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

    // ── 跳切（§17）──────────────────────────────────────────────────
    //
    // ⚠️ 判据要覆盖**两个方向**：接缝处该留的必须留（别把用户要的手法一起剔掉），
    // 超额的必须剔（否则"连续硬切让画面很跳"）。只测一边的话，
    // "守卫恒返回 0"与"守卫把什么都剔光"都能骗过它。

    /// 一条带 `cut` 的运镜（跳切的候选）。
    fn cam_cut(time: f64, cut: bool) -> Event {
        Event::Camera {
            time,
            end: Some(time + 2.0),
            zoom: "中".into(),
            origin: "50% 50%".into(),
            anchor: None,
            anchor_to: None,
            to_origin: None,
            cut,
            handheld: false,
            manual: false,
            speed: None,
            tilt: None,
        }
    }

    fn segs_with(events: Vec<Event>) -> Vec<Segment> {
        vec![Segment {
            start: 0.0,
            end: 600.0,
            fx: "med".into(),
            note: String::new(),
            role: String::new(),
            events,
            extra: Default::default(),
        }]
    }

    fn mute(s: f64, e: f64) -> MuteRange {
        MuteRange { start: s, end: e }
    }

    fn cuts(segs: &[Segment]) -> Vec<bool> {
        segs.iter()
            .flat_map(|s| s.events.iter())
            .filter_map(|e| match e {
                Event::Camera { cut, .. } => Some(*cut),
                _ => None,
            })
            .collect()
    }

    /// ① **接缝处该留的必须留**：`cut` 落在 mute 边界上 ⇒ 认得出、且不被误剔。
    #[test]
    fn jump_seam_is_recognised_at_mute_boundaries() {
        // 两个接缝：mute 4.0→6.0 的两侧
        let segs = segs_with(vec![cam_cut(0.0, false), cam_cut(4.0, true), cam_cut(6.0, true)]);
        let seams = find_jump_seams(&segs, &[mute(4.0, 6.0)]);
        assert_eq!(seams.len(), 2, "两侧各一个接缝: {seams:?}");
        assert_eq!(seams[0].at, 4.0);
        assert_eq!(seams[1].at, 6.0);

        // 两个接缝相隔 2s < 20s ⇒ 第二个要被剔（密度闸）
        let mut segs2 = segs.clone();
        let demoted = cap_jump_cuts(&mut segs2, &[mute(4.0, 6.0)], &[]);
        assert_eq!(demoted, 1, "相隔 2s 的第二次跳切超出密度闸");
        assert_eq!(cuts(&segs2), vec![false, true, false]);
    }

    /// ② **不落在接缝上的 `cut` 不算跳切**（它就是普通的切镜，不占跳切配额）。
    #[test]
    fn ordinary_cut_is_not_a_jump() {
        let segs = segs_with(vec![cam_cut(1.0, true), cam_cut(3.0, true)]);
        let seams = find_jump_seams(&segs, &[mute(4.0, 6.0)]);
        assert!(seams.is_empty(), "1.0/3.0 都不在 4.0/6.0 上: {seams:?}");
        // 没有接缝 ⇒ 一个字都不改
        let mut segs2 = segs.clone();
        assert_eq!(cap_jump_cuts(&mut segs2, &[mute(4.0, 6.0)], &[]), 0);
        assert_eq!(cuts(&segs2), vec![true, true], "普通切镜不许被碰");
    }

    /// ③ **单段配额**：同一段内第 4 条跳切才降级（2026-10-20 配额 2 → 3；
    /// 间隔闸在这里被时间拉开、不干扰）。
    #[test]
    fn per_segment_quota_demotes_the_fourth_jump() {
        let segs = segs_with(vec![
            cam_cut(10.0, true),
            cam_cut(40.0, true),
            cam_cut(70.0, true),
            cam_cut(100.0, true), // 第 4 条：超出 MAX_JUMPS_PER_SEGMENT(3)
        ]);
        // 四条 mute 的边界恰好落在这些时刻上
        let mutes = [mute(9.0, 10.0), mute(39.0, 40.0), mute(69.0, 70.0), mute(99.0, 100.0)];
        let seams = find_jump_seams(&segs, &mutes);
        assert_eq!(seams.len(), 4);
        let mut segs2 = segs.clone();
        assert_eq!(cap_jump_cuts(&mut segs2, &mutes, &[]), 1);
        assert_eq!(cuts(&segs2), vec![true, true, true, false], "只剔第 4 条");
    }

    /// ④ **密度闸**：跨段也算 —— 两段各一条、但只隔 5s ⇒ 第二条被剔
    /// （配额是"整片节奏"，不是"每段各算各的"）。
    #[test]
    fn density_gate_applies_across_segments() {
        let mk = |start: f64, at: f64| Segment {
            role: String::new(),
            start,
            end: start + 30.0,
            fx: "med".into(),
            note: String::new(),
            events: vec![cam_cut(at, true)],
            extra: Default::default(),
        };
        // 2026-10-20：间隔门槛 20s → 4s —— 5s 现在是**合法**的
        let mut segs = vec![mk(0.0, 10.0), mk(30.0, 15.0)]; // 相隔 5s
        let mutes = [mute(9.5, 10.0), mute(14.5, 15.0)];
        let mut segs_clone = segs.clone();
        let demoted = cap_jump_cuts(&mut segs_clone, &mutes, &[]);
        assert_eq!(demoted, 0, "5s ≥ 门槛 4s，跨段也不该剔");
        // 但 3s（< 4s）仍要剔 —— 密度闸本身不能失效
        let mut tight = vec![mk(0.0, 10.0), mk(30.0, 13.0)];
        let mutes_tight = [mute(9.5, 10.0), mute(12.5, 13.0)];
        assert_eq!(cap_jump_cuts(&mut tight, &mutes_tight, &[]), 1, "跨段的第二次太密（3s）");
        let mut segs_clone = tight.clone();
        let demoted = cap_jump_cuts(&mut segs_clone, &mutes_tight, &[]);
        assert_eq!(demoted, 0, "再跑一遍不该又剔（幂等）");
        assert_eq!(segs_clone[0].events.len(), 1, "事件本身要留着（只收回手法资格）");
        assert_eq!(segs_clone[1].events.len(), 1);
        let _ = &mut segs;
    }

    /// ⑤ **幂等**：跑两遍与跑一遍结果相同（process 每次预览/出片都会被隐式触发）。
    #[test]
    fn jump_cap_is_idempotent() {
        // 2026-10-20：门槛 4s ⇒ 用 2s 间隔才触发（10.0 与 12.0）
        let mutes = [mute(9.0, 10.0), mute(11.0, 12.0)];
        let mut segs = segs_with(vec![cam_cut(10.0, true), cam_cut(12.0, true)]);
        let first = cap_jump_cuts(&mut segs, &mutes, &[]);
        let after_first = cuts(&segs);
        let second = cap_jump_cuts(&mut segs, &mutes, &[]);
        assert_eq!(second, 0, "第二趟没有可剔的了");
        assert_eq!(cuts(&segs), after_first, "第二趟不许再改");
        assert_eq!(first, 1);
    }

    /// ⑥ **手工事件不参与**（与 `cap_pip_shots` / 贴纸对齐同一条规则）。
    #[test]
    fn manual_jump_is_never_touched() {
        let mut manual = cam_cut(10.0, true);
        manual.set_time_manual(true);
        let mut segs = segs_with(vec![manual, cam_cut(15.0, true)]);
        let mutes = [mute(9.5, 10.0), mute(14.5, 15.0)];
        let demoted = cap_jump_cuts(&mut segs, &mutes, &[]);
        // 手工那条根本不进接缝清单 ⇒ 不算配额；自动那条也不超
        assert_eq!(demoted, 0, "手工事件既不计数也不被改");
        match &segs[0].events[0] {
            Event::Camera { cut, .. } => assert!(*cut, "手工的 cut 必须原样"),
            _ => unreachable!(),
        }
    }

    /// ⑦ **没有 mute 区间 ⇒ 一条接缝都没有**（存量工程零回归：不许凭空动 cut）。
    #[test]
    fn no_mute_ranges_means_no_jump_cuts() {
        let segs = segs_with(vec![cam_cut(0.0, true), cam_cut(10.0, true)]);
        assert!(find_jump_seams(&segs, &[]).is_empty());
        let mut segs2 = segs.clone();
        assert_eq!(cap_jump_cuts(&mut segs2, &[], &[]), 0);
        assert_eq!(cuts(&segs2), vec![true, true]);
    }

    // ── §19 荷兰角守卫 ────────────────────────────────────────────────

    fn tilt_of(segs: &[Segment], si: usize, ei: usize) -> Option<f64> {
        match &segs[si].events[ei] {
            Event::Camera { tilt, .. } => *tilt,
            _ => panic!("不是运镜事件"),
        }
    }

    /// ① ⚠️⚠️ **禁配画中画**：与「远」同段的倾斜必须被剔除。
    ///
    /// 这是规格 §4.4b 那处自相矛盾的落点 —— 场景表推荐"画中画配合"，
    /// 而几何说「远」(0.55) 的最大安全角是 **0.00°**。按几何走。
    /// ⚠️ 判据必须用**同一个段里另有一条「远」**的情形：只看"这条自己是不是远"会漏掉
    /// 状态机的持续性（倾斜跟着到下一条运镜，中间夹一条画中画照样露边）。
    #[test]
    fn dutch_angle_is_stripped_when_the_segment_has_pip() {
        let mut segs = vec![
            // 同一段里：先一条普通倾斜的运镜，再来一条画中画
            Segment {
                start: 0.0,
                end: 20.0,
                fx: "med".into(),
                note: String::new(),
                role: String::new(),
                events: vec![cam_t(0.0, "中", 5.0), cam(10.0, "远")],
                extra: Default::default(),
            },
        ];
        assert_eq!(cap_dutch_angle(&mut segs), 1, "同段有画中画 ⇒ 倾斜必须被剔除");
        assert_eq!(tilt_of(&segs, 0, 0), None);
    }

    /// ② ⚠️ **但另一段里的画中画不该误伤**：判据是"同段"，不是"全片".
    ///
    /// 写成全片扫描会变成"只要片子里有画中画就一个荷兰角都用不了" ——
    /// 而配额是 2 条，那等于**整条特性静默失效**（不报错、只是永远不生效）。
    #[test]
    fn pip_in_another_segment_does_not_strip_the_angle() {
        let mut segs = vec![
            Segment {
                start: 0.0,
                end: 10.0,
                fx: "med".into(),
                note: String::new(),
                role: String::new(),
                events: vec![cam_t(0.0, "中", 5.0)],
                extra: Default::default(),
            },
            Segment {
                start: 10.0,
                end: 20.0,
                fx: "med".into(),
                note: String::new(),
                role: String::new(),
                events: vec![cam(10.0, "远")],
                extra: Default::default(),
            },
        ];
        assert_eq!(cap_dutch_angle(&mut segs), 0, "别的段有画中画不该影响这一段");
        assert_eq!(tilt_of(&segs, 0, 0), Some(5.0));
    }

    /// ③ **夹到 ±6°**（两个方向都要夹）。
    #[test]
    fn dutch_angle_is_clamped_in_both_directions() {
        let mut segs = vec![
            Segment {
                start: 0.0,
                end: 10.0,
                fx: "med".into(),
                note: String::new(),
                role: String::new(),
                events: vec![cam_t(0.0, "中", 15.0), cam_t(4.0, "中", -9.0)],
                extra: Default::default(),
            },
        ];
        assert_eq!(cap_dutch_angle(&mut segs), 2);
        assert_eq!(tilt_of(&segs, 0, 0), Some(6.0), "正向夹到 +6");
        assert_eq!(tilt_of(&segs, 0, 1), Some(-6.0), "负向夹到 −6（两个方向都是荷兰角）");
    }

    /// ④ ⚠️ **全片最多 2 条，超出留 `time` 最早的**。
    ///
    /// 取"最早"而不是"最先出现"是为**确定性**：段表里段的顺序与时间顺序**不是一回事**
    /// （LLM 按窗口生成 + 去静音切段，段之间时间会重叠）。
    #[test]
    fn dutch_angle_is_capped_at_two_shots() {
        // 故意让数组顺序与时间顺序相反
        let mut segs = vec![
            Segment {
                start: 30.0,
                end: 40.0,
                fx: "med".into(),
                note: String::new(),
                role: String::new(),
                events: vec![cam_t(30.0, "中", 4.0)], // 最晚
                extra: Default::default(),
            },
            Segment {
                start: 0.0,
                end: 10.0,
                fx: "med".into(),
                note: String::new(),
                role: String::new(),
                events: vec![cam_t(0.0, "中", 4.0)], // 最早 ⇒ 留
                extra: Default::default(),
            },
            Segment {
                start: 15.0,
                end: 25.0,
                fx: "med".into(),
                note: String::new(),
                role: String::new(),
                events: vec![cam_t(15.0, "中", 4.0)], // 居中 ⇒ 留
                extra: Default::default(),
            },
        ];
        assert_eq!(cap_dutch_angle(&mut segs), 1, "三条 ⇒ 剥掉一条");
        assert_eq!(tilt_of(&segs, 1, 0), Some(4.0), "最早的留下");
        assert_eq!(tilt_of(&segs, 2, 0), Some(4.0), "次早的留下");
        assert_eq!(tilt_of(&segs, 0, 0), None, "最晚的被剥掉");
    }

    /// ⑤ **幂等**：process 每次预览/出片都被隐式触发，第二趟必须 0 改动。
    #[test]
    fn dutch_angle_cap_is_idempotent() {
        let mut segs = vec![
            Segment {
                start: 0.0,
                end: 10.0,
                fx: "med".into(),
                note: String::new(),
                role: String::new(),
                events: vec![cam_t(0.0, "中", 99.0), cam_t(4.0, "中", 3.0), cam_t(6.0, "中", 2.0)],
                extra: Default::default(),
            },
        ];
        let first = cap_dutch_angle(&mut segs);
        assert!(first > 0, "第一趟该有改动");
        let snapshot: Vec<Option<f64>> = (0..3).map(|i| tilt_of(&segs, 0, i)).collect();
        assert_eq!(cap_dutch_angle(&mut segs), 0, "第二趟必须收敛");
        let after: Vec<Option<f64>> = (0..3).map(|i| tilt_of(&segs, 0, i)).collect();
        assert_eq!(snapshot, after, "再跑也不许变");
    }

    /// ⑥ ⚠️ **零 / 非有限值一律清成 `None`**（不许写出 `tilt = 0` 或 `NaN`）。
    ///
    /// `NaN` 进 toml 会让**整个工程读不出来**；`0.0` 则是"写了个没用的键"
    /// （零倾斜 = 不倾斜，写了只会让 `polish.toml` 多一行噪声）。
    #[test]
    fn dutch_angle_drops_zero_and_non_finite() {
        let mut segs = vec![
            Segment {
                start: 0.0,
                end: 10.0,
                fx: "med".into(),
                note: String::new(),
                role: String::new(),
                events: vec![
                    cam_t(0.0, "中", 0.0),
                    cam_t(2.0, "中", f64::NAN),
                    cam_t(4.0, "中", f64::INFINITY),
                ],
                extra: Default::default(),
            },
        ];
        assert_eq!(cap_dutch_angle(&mut segs), 3);
        for i in 0..3 {
            assert_eq!(tilt_of(&segs, 0, i), None, "第 {i} 条该被清掉");
        }
    }

    /// ⑦ ⚠️ **没写 `tilt` 的工程一个字节都不动**（R4）。
    #[test]
    fn no_tilt_means_no_change_at_all() {
        let mut segs = vec![
            Segment {
                start: 0.0,
                end: 10.0,
                fx: "med".into(),
                note: String::new(),
                role: String::new(),
                events: vec![cam(0.0, "中"), cam(4.0, "近"), cam(6.0, "远")],
                extra: Default::default(),
            },
        ];
        assert_eq!(cap_dutch_angle(&mut segs), 0);
        for i in 0..3 {
            assert_eq!(tilt_of(&segs, 0, i), None);
        }
    }


    // ── §18-A 特写焦点守卫 ────────────────────────────────────────────

    fn anchor_of(segs: &[Segment], i: usize) -> Option<String> {
        match &segs[0].events[i] {
            Event::Camera { anchor, .. } => anchor.clone(),
            _ => panic!("不是运镜事件"),
        }
    }

    /// ① ⚠️ **特写下非 liver 的聚焦一律改回 liver**（用户拍板）。
    ///
    /// 1.8 倍下弹幕/贴纸区基本被挤出画面，聚焦过去就是"主播不见了"。
    #[test]
    fn closeup_forces_liver_anchor() {
        let mut segs = vec![seg_with(vec![
            cam_a(0.0, "特写", Some("danmaku"), None),
            cam_a(5.0, "特写", Some("sticker"), None),
            cam_a(9.0, "特写", Some("center"), None),
        ])];
        assert_eq!(cap_closeup_anchor(&mut segs), 3);
        for i in 0..3 {
            assert_eq!(anchor_of(&segs, i).as_deref(), Some("liver"), "第 {i} 条");
        }
    }

    /// ② ⚠️ **终点焦点 `anchor_to` 也一起锁**。
    ///
    /// 它是"从这里移到那里"的终点 —— 特写下移到弹幕同样看不到。
    /// ⚠️ 只锁 `anchor` 不锁 `anchor_to` 的症状是"推的时候回来了、推完又跑掉"。
    #[test]
    fn closeup_forces_liver_on_both_ends() {
        let mut segs = vec![seg_with(vec![cam_a(0.0, "特写", Some("danmaku"), Some("center"))])];
        assert_eq!(cap_closeup_anchor(&mut segs), 2, "起点与终点各算一处");
        match &segs[0].events[0] {
            Event::Camera { anchor, anchor_to, .. } => {
                assert_eq!(anchor.as_deref(), Some("liver"));
                assert_eq!(anchor_to.as_deref(), Some("liver"));
            }
            _ => panic!(),
        }
    }

    /// ③ ⚠️ **只碰「特写」档**（R4）。
    ///
    /// ⚠️ 判据必须**同时**验"特写被改了"和"其余档位没被改" ——
    /// 只验前者的话，把守卫写成"全档位都强制 liver"照样绿。
    #[test]
    fn closeup_guard_leaves_other_tiers_alone() {
        let mut segs = vec![seg_with(vec![
            cam_a(0.0, "中", Some("danmaku"), None),
            cam_a(5.0, "近", Some("danmaku"), None),
            cam_a(9.0, "远", Some("danmaku"), None),
            cam_a(12.0, "特写", Some("danmaku"), None),
        ])];
        assert_eq!(cap_closeup_anchor(&mut segs), 1, "只有特写那条该被改");
        assert_eq!(anchor_of(&segs, 0).as_deref(), Some("danmaku"), "中景不许动");
        assert_eq!(anchor_of(&segs, 1).as_deref(), Some("danmaku"), "近景不许动");
        assert_eq!(anchor_of(&segs, 2).as_deref(), Some("danmaku"), "画中画不许动");
        assert_eq!(anchor_of(&segs, 3).as_deref(), Some("liver"), "特写该被改");
    }

    /// ④ **缺省（没写 anchor）也要补成 liver**，但幂等。
    ///
    /// ⚠️ `None` 与 `Some("liver")` 在这里必须**行为一致**（都算"已经是 liver"），
    /// 否则每跑一次 process 都会多改一次 —— 而 process 每次预览/出片都被隐式触发，
    /// 症状是"日志每次都刷一行、polish.toml 每次都被重写"。
    #[test]
    fn closeup_guard_is_idempotent() {
        let mut segs = vec![seg_with(vec![
            cam_a(0.0, "特写", None, None),
            cam_a(5.0, "特写", Some("liver"), None),
        ])];
        let first = cap_closeup_anchor(&mut segs);
        assert_eq!(first, 1, "只有缺省那条要补");
        assert_eq!(anchor_of(&segs, 0).as_deref(), Some("liver"));
        assert_eq!(cap_closeup_anchor(&mut segs), 0, "第二趟必须收敛");
    }

    /// ⑤ 档位名要**去空白**再比（LLM/编辑器可能带空格）。
    #[test]
    fn closeup_label_is_trimmed() {
        let mut segs = vec![seg_with(vec![
            cam_a(0.0, " 特写 ", Some("danmaku"), None),
            cam_a(5.0, "近 ", Some("danmaku"), None),
        ])];
        assert_eq!(cap_closeup_anchor(&mut segs), 1);
        assert_eq!(anchor_of(&segs, 0).as_deref(), Some("liver"), "带空白的特写也要认");
        assert_eq!(anchor_of(&segs, 1).as_deref(), Some("danmaku"), "带空白的近景仍不许动");
    }

}

#[cfg(test)]
mod auto_fill_tests {
    use super::*;
    use crate::parse::srt::Subtitle;
    use crate::project::{MuteRange, Segment};

    fn seg(start: f64, end: f64) -> Segment {
        Segment {
            role: String::new(),
            start,
            end,
            fx: "med".into(),
            note: String::new(),
            events: vec![],
            extra: Default::default(),
        }
    }
    fn sub(start: f64, end: f64) -> Subtitle {
        Subtitle { index: 0, start, end, text: "一句话".into(), html: String::new() }
    }

    /// **空窗补位**：20s 的段没有任何内容事件 ⇒ 在窗口里最居中的那条字幕上补一条 pulse，
    /// **第二趟不再补**（幂等）。变异自证：删掉幂等判据 ⇒ 第二趟又多一条，本条红。
    #[test]
    fn fill_visual_gaps_adds_one_per_empty_window_and_is_idempotent() {
        let mut segs = vec![seg(0.0, 20.0)];
        let subs = vec![sub(2.0, 3.0), sub(10.1, 11.1)];
        let first = fill_visual_gaps(&mut segs, &subs, &[]);
        assert!(first >= 1, "20s 空段要补上（实际 {first} 条）");
        assert!(segs[0].events.iter().all(is_auto_fill), "补出来的必须都带标记");
        let n = segs[0].events.len();
        assert_eq!(fill_visual_gaps(&mut segs, &subs, &[]), 0, "第二趟必须 0（process 会被隐式重跑）");
        assert_eq!(segs[0].events.len(), n, "第二趟一条都不许加");
    }

    /// 屏蔽区间里的字幕**不当落点**（那一段成片里不存在）。
    #[test]
    fn fill_visual_gaps_skips_muted_windows() {
        let mut segs = vec![seg(0.0, 20.0)];
        let subs = vec![sub(10.0, 11.0)];
        let mutes = vec![MuteRange { start: 9.0, end: 12.0 }];
        assert_eq!(fill_visual_gaps(&mut segs, &subs, &mutes), 0, "唯一可选的字幕在屏蔽区里");
    }

    /// **camera 不算内容事件**（运镜全程开着）：只有 camera 的段照样要补。
    #[test]
    fn camera_does_not_count_as_content() {
        let mut s = seg(0.0, 20.0);
        s.events.push(Event::Camera {
            time: 0.0,
            end: Some(20.0),
            zoom: "中".into(),
            origin: "50% 50%".into(),
            anchor: None,
            anchor_to: None,
            to_origin: None,
            cut: false,
            handheld: false,
            manual: false,
            speed: None,
            tilt: None,
        });
        let mut segs = vec![s];
        let subs = vec![sub(10.0, 11.0)];
        assert_eq!(fill_visual_gaps(&mut segs, &subs, &[]), 1, "光有运镜不算有内容");
    }

    /// **幂等判据本身**：窗口里已经有带标记的补位事件 ⇒ **不再补**（哪怕窗口仍 > 门槛）。
    ///
    /// 这条才是"标记"的判据（另一条只证明收敛性）。变异自证：把 `is_auto_fill` 那个过滤
    /// 从窗口判据里去掉 ⇒ 这里会补出 2 条，本条红。
    #[test]
    fn marked_window_is_never_refilled_even_if_wide() {
        let mut s = seg(0.0, 20.0);
        s.events.push(Event::Pulse {
            time: 10.0,
            end: Some(10.35),
            anchor: Some(AUTO_FILL_ANCHOR.to_string()),
            sfx: None,
            sfx_volume: None,
        });
        let mut segs = vec![s];
        let subs = vec![sub(4.0, 5.0), sub(10.0, 11.0)];
        assert_eq!(fill_visual_gaps(&mut segs, &subs, &[]), 0, "带标记的窗口不许再补");
    }
}

