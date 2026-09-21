//! polish::process::fx：事件默认值补齐与守卫（瞬时特效跨度 / 画中画配额与对齐 / 手持摇摆 / 贴纸对齐字幕）。
//!
//! 入口：`transient_duration_cap`、`transient_default_duration`、`normalize_event_ends`、`cap_pip_shots`、`align_pip_shots`、`close_pip_shots`、`cap_handheld_sway`、`align_stickers_to_subtitles`。
//! 边界：手工事件（`Event::is_time_manual()`）一律跳过改写；这些守卫在 `process/mod.rs` 里的调用顺序是硬约束，改一处要看整条 Step 序 —— 见 `crates/vtrim-polish/src/process/AGENTS.md`

use crate::parse::srt::Subtitle;
use crate::project::{Event, Segment};

/// 瞬时特效的 duration 上限（秒）。
/// 原则：瞬时特效只能“爆发一下”，不允许长时间驻留。
pub fn transient_duration_cap(event_type: &str) -> f64 {
    match event_type {
        "shake" => 0.5,
        "flash" => 0.3,
        "blur" => 0.4,
        "zoom_bounce" => 0.6,
        "stutter" => 0.35,
        "split" => 0.35,
        "noise" => 0.4,
        "pulse" => 0.5,
        "hue_shift" => 0.5,
        "color_shift" => 0.5,
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
        "zoom_bounce" => 0.4,
        "stutter" => 0.25,
        "split" => 0.25,
        "noise" => 0.3,
        "pulse" => 0.4,
        "hue_shift" => 0.35,
        "color_shift" => 0.35,
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
        Event::ZoomBounce { .. } => "zoom_bounce",
        Event::Stutter { .. } => "stutter",
        Event::Split { .. } => "split",
        Event::Noise { .. } => "noise",
        Event::Pulse { .. } => "pulse",
        Event::HueShift { .. } => "hue_shift",
        Event::ColorShift { .. } => "color_shift",
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

/// 表情贴纸对齐**最近的字幕**（用户要求：「表情贴纸也需要对齐最近的字幕」）。
///
/// 起点吸到那条字幕的开始、终点吸到它的结束 —— 贴纸正好盖住它反应的那句话，头尾都落在
/// 字幕边界上（与画中画的 `align_pip_shots` 同一套思路）。离最近字幕超过
/// [`STICKER_SNAP_MAX_SECS`] 的、以及短得没法盖（< 0.2s）的字幕都不动。
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
            let len = sub.end - sub.start;
            if len < 0.2 {
                continue;
            }
            let old_end = end.unwrap_or(*time + len);
            if (*time - sub.start).abs() < 1e-6 && (old_end - sub.end).abs() < 1e-6 {
                continue;
            }
            *time = (sub.start * 100.0).round() / 100.0;
            *end = Some((sub.end * 100.0).round() / 100.0);
            aligned += 1;
            tracing::info!(
                "  · 贴纸对齐字幕：{:.2}s → {:.2}s（原 {:.2}s → {:.2}s）",
                *time, sub.end, sub.start, old_end
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

        // 太短的字幕（0.1s）盖不住，跳过
        let short = vec![sub(1, 30.0, 30.1)];
        let mut segs = sticker_seg(sticker(30.05, 2.0));
        assert_eq!(align_stickers_to_subtitles(&mut segs, &short), 0);

        // 没有字幕 → 原样
        let mut segs = sticker_seg(sticker(5.0, 2.0));
        assert_eq!(align_stickers_to_subtitles(&mut segs, &[]), 0);
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
}
