//! **分散事件合并**（检测第二趟的收尾）：把低标准报出来的小块按话题归组、拼成一条多段事件。
//!
//! 完整来龙去脉见 `docs/plan/scattered-events-merge.md`。这里只放**纯函数**（可单测、不带 IO）：
//!
//! - [`merge_scattered`]：按时间序 + 同 `topic` + 间隔 ≤ `merge_gap_secs` 归组；
//!   **无字幕的空隙保留**（拼成一条连续区间，交给后处理「去静音」变屏蔽区间），
//!   **有内容的空隙用 `Event::ranges` 丢掉**；合并后仍不达标 ⇒ **丢弃**。
//! - [`absorb_scattered`]：把活下来的分散事件并回普通事件列表 —— 被普通事件盖住（重叠 ≥50%）
//!   或紧贴（间隔 ≤1s）的一律**丢掉**（普通事件的标题与切点是权威，不许被小块改写）。
//!
//! ⚠️ 两条纪律：
//! ① 判据全在**代码**里（时长/字幕条数/弹幕密度），不靠 prompt 自觉；
//! ② 单块时长下限 `min_secs` 与合并下限 `merge_min_secs` 是**两道不同的门**：
//!    前者过滤"一小段来回"的噪声块，后者决定"这条合并结果值不值得切"。

use crate::config::ScatteredConfig;
use crate::danmaku::DensityWindow;
use crate::types::{Event, EventRange, EventSource, TranscriptSegment};
#[cfg(test)]
use crate::types::Timestamp;

/// 合并统计（给日志与验收用；也让"到底丢在哪一道门"有数可查）。
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ScatteredMergeStat {
    /// 进门的候选块数（第二趟跨窗口合并后的条数）。
    pub input: usize,
    /// 比 `min_secs` 还短、直接丢掉的块。
    pub dropped_too_short: usize,
    /// 归成了几组（单块也算一组）。
    pub groups: usize,
    /// 活下来的条数（= 单块保住的 + 多块合并的）。
    pub kept: usize,
    /// 其中"由多块合并"的条数。
    pub merged_from_pieces: usize,
    /// 其中"带多段 `ranges`（空隙有内容被丢掉）"的条数。
    pub multi_range: usize,
    /// 合并后内容时长不足 `merge_min_secs` 而丢掉的组。
    pub dropped_content_short: usize,
    /// 合并后字幕条数不足 `merge_min_subs` 而丢掉的组。
    pub dropped_few_subs: usize,
    /// 并回普通事件时被盖住/紧贴而丢掉的条数。
    pub absorbed_by_main: usize,
}

/// 合并所需的上下文（字幕 + 弹幕热度图），纯数据、方便单测直接摆。
pub struct ScatteredCtx<'a> {
    pub cfg: &'a ScatteredConfig,
    pub segments: &'a [TranscriptSegment],
    pub heatmap: Option<&'a [DensityWindow]>,
    pub danmaku_window_secs: f64,
}

impl<'a> ScatteredCtx<'a> {
    pub fn new(cfg: &'a ScatteredConfig, segments: &'a [TranscriptSegment]) -> Self {
        Self {
            cfg,
            segments,
            heatmap: None,
            danmaku_window_secs: 10.0,
        }
    }

    pub fn with_heatmap(mut self, heatmap: Option<&'a [DensityWindow]>, window_secs: f64) -> Self {
        self.heatmap = heatmap;
        self.danmaku_window_secs = window_secs;
        self
    }

    /// 一段区间里"有内容"吗（判"空隙能不能保留成连续区间"）。
    ///
    /// 无字幕、无弹幕 ⇒ **无内容** ⇒ 空隙保留（拼成连续区间，之后由「去静音」变屏蔽区间）。
    /// 只要字幕够多或者弹幕够炸 ⇒ **有内容** ⇒ 空隙必须丢掉（走 `ranges`），
    /// 否则拼出来的那一段就是**另一个话题**。
    pub fn gap_has_content(&self, start: f64, end: f64) -> bool {
        if end <= start {
            return false;
        }
        // ① 字幕：区间内各 ASR 段与之的重叠秒数之和
        let mut sub_secs = 0.0;
        for seg in self.segments {
            let lo = seg.start.max(start);
            let hi = seg.end.min(end);
            if hi > lo {
                sub_secs += hi - lo;
            }
        }
        if sub_secs > self.cfg.gap_sub_secs {
            return true;
        }
        // ② 弹幕：区间内任一窗口密度达到门槛
        if let Some(heatmap) = self.heatmap {
            for w in heatmap {
                let we = w.start + self.danmaku_window_secs;
                if w.start < end && we > start && w.density >= self.cfg.gap_dm_floor {
                    return true;
                }
            }
        }
        false
    }

    /// 落在这些区间里的字幕条数（按"重叠"算，与切片侧同一条口径：只重叠一点也算）。
    fn subtitle_count(&self, ranges: &[EventRange]) -> usize {
        self.segments
            .iter()
            .filter(|seg| {
                ranges
                    .iter()
                    .any(|r| seg.start < r.end && seg.end > r.start)
            })
            .count()
    }

    /// 区间内弹幕密度峰值（0 = 没弹幕数据）。用于"单块够不够格"这道门。
    fn peak_density(&self, start: f64, end: f64) -> f64 {
        let Some(heatmap) = self.heatmap else {
            return 0.0;
        };
        let mut peak = 0.0_f64;
        for w in heatmap {
            let we = w.start + self.danmaku_window_secs;
            if w.start < end && we > start {
                peak = peak.max(w.density);
            }
        }
        peak
    }
}

/// 一个候选块的两道门：时长 ≥ `min_secs`，且（字幕 ≥ `min_subs` **或** 弹幕密度 ≥ `dm_floor`）。
fn piece_qualifies(ev: &Event, ctx: &ScatteredCtx) -> bool {
    let dur = ev.end.to_seconds() - ev.start.to_seconds();
    if dur + 1e-6 < ctx.cfg.min_secs {
        return false;
    }
    let subs = ctx.subtitle_count(&ev.cut_ranges());
    if subs >= ctx.cfg.min_subs {
        return true;
    }
    // 弹幕那一路：没配 `dm_floor`（0）就当"不看弹幕"，不让短块靠它混进来
    ctx.cfg.dm_floor > 0.0
        && ctx.peak_density(ev.start.to_seconds(), ev.end.to_seconds()) >= ctx.cfg.dm_floor
}

/// 规范化 topic 键（大小写/空白不敏感；空串 = 没有键）。
fn topic_key_of(ev: &Event) -> Option<String> {
    ev.topic
        .as_deref()
        .map(|t| t.trim().to_lowercase())
        .filter(|t| !t.is_empty())
}

/// 两个块能不能并成一组（时间相邻 + 同话题）。
fn same_group(group_topic: &Option<String>, ev: &Event, group_end: f64, ctx: &ScatteredCtx) -> bool {
    let start = ev.start.to_seconds();
    // 间隔（允许轻微重叠：重叠按 0 处理）
    if start - group_end > ctx.cfg.merge_gap_secs {
        return false;
    }
    if !ctx.cfg.topic_key {
        return true;
    }
    match (group_topic, topic_key_of(ev)) {
        // 两边都有 topic：必须一致
        (Some(a), Some(b)) => *a == b,
        // 有一边没给 topic（LLM 漏了）：不放行 —— 宁可少并，也别把两个话题焊在一起
        _ => false,
    }
}

/// 主入口：把第二趟的候选块归组、拼成多段事件。
///
/// 返回 `(事件, 统计)`；输出按时间排序，`ranges` 已归一化（首段起、末段末）。
pub fn merge_scattered(mut events: Vec<Event>, ctx: &ScatteredCtx) -> (Vec<Event>, ScatteredMergeStat) {
    let mut stat = ScatteredMergeStat {
        input: events.len(),
        ..Default::default()
    };

    // ① 单块两道门（时长 + 字幕/弹幕）：低于 `min_secs` 的直接丢 —— 那是"一小段来回"的噪声
    events.retain(|e| e.end.to_seconds() > e.start.to_seconds());
    let before = events.len();
    events.retain(|e| piece_qualifies(e, ctx));
    stat.dropped_too_short = before - events.len();

    events.sort_by(|a, b| {
        a.start
            .to_seconds()
            .partial_cmp(&b.start.to_seconds())
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    // ② 按时间序归组：同话题 + 间隔 ≤ `merge_gap_secs` + 跨度不超 `max_merge_secs`
    let mut groups: Vec<Vec<Event>> = Vec::new();
    for ev in events {
        let fits = match groups.last() {
            Some(g) => {
                let group_start = g[0].start.to_seconds();
                let group_end = g.iter().map(|e| e.end.to_seconds()).fold(f64::MIN, f64::max);
                let group_topic = topic_key_of(&g[0]);
                let would_span = ev.end.to_seconds() - group_start;
                would_span <= ctx.cfg.max_merge_secs && same_group(&group_topic, &ev, group_end, ctx)
            }
            None => false,
        };
        if fits {
            groups.last_mut().expect("刚判过 last").push(ev);
        } else {
            groups.push(vec![ev]);
        }
    }
    stat.groups = groups.len();

    // ③ 每组拼区间 + 判达标
    let mut out: Vec<Event> = Vec::new();
    for mut group in groups {
        let pieces = group.len();
        let killed = merge_group(&mut group, ctx);
        let Some(mut ev) = killed else {
            // 丢了：分清是"时长不够"还是"字幕不够"，日志里好归因
            let mut probe = group[0].clone();
            probe.normalize_ranges();
            if probe.content_secs() + 1e-6 < ctx.cfg.merge_min_secs {
                stat.dropped_content_short += 1;
            } else {
                stat.dropped_few_subs += 1;
            }
            continue;
        };
        ev.normalize_ranges();
        stat.kept += 1;
        if pieces > 1 {
            stat.merged_from_pieces += 1;
            ev.source = EventSource::Merged;
        }
        if ev.cut_ranges().len() > 1 {
            stat.multi_range += 1;
        }
        out.push(ev);
    }

    out.sort_by(|a, b| {
        a.start
            .to_seconds()
            .partial_cmp(&b.start.to_seconds())
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    (out, stat)
}

/// 把一组合成一条事件；不达标返回 `None`（调用方据此记统计）。
fn merge_group(group: &mut Vec<Event>, ctx: &ScatteredCtx) -> Option<Event> {
    // 取信息量最大的那块当"脸"（本轮还没有热度，用时长最长的；并列取最早的）
    let face_idx = group
        .iter()
        .enumerate()
        .max_by(|(_, a), (_, b)| {
            let da = a.end.to_seconds() - a.start.to_seconds();
            let db = b.end.to_seconds() - b.start.to_seconds();
            da.partial_cmp(&db)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| {
                    b.start
                        .to_seconds()
                        .partial_cmp(&a.start.to_seconds())
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
        })
        .map(|(i, _)| i)
        .unwrap_or(0);
    let mut ev = group[face_idx].clone();

    // 拼区间：无内容的空隙**保留**（连成一段），有内容的空隙**丢掉**（新起一段）
    let mut ranges: Vec<EventRange> = Vec::new();
    for piece in group.iter() {
        let ps = piece.start.to_seconds();
        let pe = piece.end.to_seconds();
        match ranges.last_mut() {
            Some(last) => {
                if ctx.gap_has_content(last.end, ps) {
                    ranges.push(EventRange::new(ps, pe));
                } else {
                    last.end = last.end.max(pe);
                }
            }
            None => ranges.push(EventRange::new(ps, pe)),
        }
    }
    ev.ranges = ranges;
    ev.normalize_ranges();

    // 达标判据：**内容时长**（不含空隙）+ 字幕条数
    if ev.content_secs() + 1e-6 < ctx.cfg.merge_min_secs {
        return None;
    }
    if ctx.subtitle_count(&ev.cut_ranges()) < ctx.cfg.merge_min_subs {
        return None;
    }
    Some(ev)
}

/// 把分散事件并回普通事件列表：被普通事件**盖住**（重叠 ≥ 50%）或**紧贴**（间隔 ≤1s）的丢掉。
///
/// ⚠️ 不并集、不改普通事件的切点与标题 —— 普通事件是权威；小块只在"普通事件没覆盖到的地方"
/// 才有资格成为一条独立片段。
pub fn absorb_scattered(main: &mut Vec<Event>, scattered: Vec<Event>, stat: &mut ScatteredMergeStat) {
    for ev in scattered {
        let (s, e) = (ev.start.to_seconds(), ev.end.to_seconds());
        let dur = (e - s).max(1e-6);
        let mut drop = false;
        for m in main.iter() {
            let (ms, me) = (m.start.to_seconds(), m.end.to_seconds());
            let overlap = (me.min(e) - ms.max(s)).max(0.0);
            let gap = if s > me {
                s - me
            } else if ms > e {
                ms - e
            } else {
                0.0
            };
            if overlap / dur >= 0.5 || gap <= 1.0 {
                drop = true;
                break;
            }
        }
        if drop {
            stat.absorbed_by_main += 1;
            continue;
        }
        main.push(ev);
    }
    main.sort_by(|a, b| {
        a.start
            .to_seconds()
            .partial_cmp(&b.start.to_seconds())
            .unwrap_or(std::cmp::Ordering::Equal)
    });
}

/// 只读诊断：把一份 events.json 的分散合并结果打出来（`VT_EVENTS_JSON=<路径> cargo test … --ignored`）。
#[cfg(test)]
pub fn diag_report(events: &[Event], ctx: &ScatteredCtx) -> String {
    let (merged, stat) = merge_scattered(events.to_vec(), ctx);
    let mut out = format!("{stat:?}\n");
    for e in merged {
        out.push_str(&format!(
            "  {:.1}-{:.1} 内容 {:.1}s {} 段 话题={:?} {}\n",
            e.start.to_seconds(),
            e.end.to_seconds(),
            e.content_secs(),
            e.cut_ranges().len(),
            e.topic,
            e.label
        ));
    }
    out
}

/// 便捷构造：给单测摆一条事件。
#[cfg(test)]
fn ev(start: f64, end: f64, label: &str, topic: Option<&str>) -> Event {
    Event {
        start: Timestamp::from_seconds(start),
        end: Timestamp::from_seconds(end),
        label: label.to_string(),
        summary: String::new(),
        event_type: crate::types::EventType::Chat,
        source: EventSource::LLM,
        confidence: 1.0,
        popularity: 0.0,
        rank: 0,
        clip_dir: None,
        enabled: true,
        ranges: Vec::new(),
        topic: topic.map(str::to_string),
    }
}

#[cfg(test)]
fn seg(start: f64, end: f64) -> TranscriptSegment {
    TranscriptSegment {
        start,
        end,
        text: "x".into(),
        confidence: 1.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 字幕稠密的默认上下文（空隙里有字幕 ⇒ 有内容）。
    fn ctx_dense<'a>(cfg: &'a ScatteredConfig, segs: &'a [TranscriptSegment]) -> ScatteredCtx<'a> {
        ScatteredCtx::new(cfg, segs)
    }

    /// 同话题、间隔 20s、空隙里没字幕 ⇒ 合成**一条连续区间**（`ranges` 仍是 1 段）。
    #[test]
    fn merges_same_topic_across_subtitle_free_gap_into_one_span() {
        let cfg = ScatteredConfig {
            enabled: true,
            ..Default::default()
        };
        // 每块两条字幕（过单块的 `min_subs`），而 120~140 的空隙里**一条都没有**
        let segs = vec![
            seg(100.0, 110.0),
            seg(110.0, 120.0),
            seg(140.0, 150.0),
            seg(150.0, 160.0),
        ];
        let ctx = ctx_dense(&cfg, &segs);
        let pieces = vec![
            ev(100.0, 120.0, "块A", Some("唱歌")),
            ev(140.0, 160.0, "块B", Some("唱歌")),
        ];
        let (out, stat) = merge_scattered(pieces, &ctx);
        assert_eq!(out.len(), 1, "{stat:?}");
        assert_eq!(out[0].cut_ranges().len(), 1, "无字幕空隙要连成一段");
        assert_eq!(out[0].start.to_seconds(), 100.0);
        assert_eq!(out[0].end.to_seconds(), 160.0);
        assert_eq!(out[0].content_secs(), 60.0);
        assert_eq!(stat.merged_from_pieces, 1);
        assert_eq!(stat.multi_range, 0);
    }

    /// 空隙里**有字幕**（别的话题夹在中间）⇒ 用 `ranges` 丢中间那段。
    #[test]
    fn keeps_ranges_when_gap_has_subtitles() {
        let cfg = ScatteredConfig {
            enabled: true,
            ..Default::default()
        };
        let segs = vec![
            seg(100.0, 110.0),
            seg(110.0, 120.0),
            seg(125.0, 135.0), // ← 空隙里的内容（10s ≫ gap_sub_secs）
            seg(140.0, 150.0),
            seg(150.0, 160.0),
        ];
        let ctx = ctx_dense(&cfg, &segs);
        let pieces = vec![
            ev(100.0, 120.0, "块A", Some("唱歌")),
            ev(140.0, 160.0, "块B", Some("唱歌")),
        ];
        let (out, stat) = merge_scattered(pieces, &ctx);
        assert_eq!(out.len(), 1, "{stat:?}");
        let r = out[0].cut_ranges();
        assert_eq!(r.len(), 2, "有内容的空隙要丢掉");
        assert_eq!((r[0].start, r[0].end), (100.0, 120.0));
        assert_eq!((r[1].start, r[1].end), (140.0, 160.0));
        // 内容时长 40s（不是跨度 60s）—— 达标判据用的就是它
        assert_eq!(out[0].content_secs(), 40.0);
        assert_eq!((out[0].start.to_seconds(), out[0].end.to_seconds()), (100.0, 160.0));
        assert_eq!(stat.multi_range, 1);
    }

    /// 不同话题**不并**（哪怕时间紧挨着）。
    #[test]
    fn different_topics_stay_apart() {
        let cfg = ScatteredConfig {
            enabled: true,
            ..Default::default()
        };
        let segs = vec![seg(100.0, 110.0), seg(112.0, 122.0)];
        let ctx = ctx_dense(&cfg, &segs);
        let pieces = vec![
            ev(100.0, 110.0, "块A", Some("唱歌")),
            ev(112.0, 122.0, "块B", Some("游戏")),
        ];
        let (out, _) = merge_scattered(pieces, &ctx);
        // 两块各自都不够 merge_min_secs=20 ⇒ 都被丢；关键是**没有被并成一条**
        assert!(out.is_empty(), "不同话题不许并：{out:?}");
    }

    /// 间隔超过 `merge_gap_secs` 不并。
    #[test]
    fn gap_beyond_limit_does_not_merge() {
        let cfg = ScatteredConfig {
            enabled: true,
            min_secs: 8.0,
            merge_gap_secs: 5.0,
            ..Default::default()
        };
        let segs = vec![
            seg(100.0, 110.0),
            seg(110.0, 120.0),
            seg(120.0, 130.0),
            seg(200.0, 210.0),
            seg(210.0, 220.0),
            seg(220.0, 230.0),
        ];
        let ctx = ctx_dense(&cfg, &segs);
        let pieces = vec![
            ev(100.0, 130.0, "块A", Some("唱歌")),
            ev(200.0, 230.0, "块B", Some("唱歌")),
        ];
        let (out, stat) = merge_scattered(pieces, &ctx);
        assert_eq!(out.len(), 2, "{stat:?}");
        assert_eq!(stat.merged_from_pieces, 0);
    }

    /// 单块低于 `min_secs` 直接丢（噪声块）；够格的单块自己活。
    #[test]
    fn short_piece_dropped_long_single_survives() {
        let cfg = ScatteredConfig {
            enabled: true,
            ..Default::default()
        };
        let segs = vec![seg(30.0, 40.0), seg(40.0, 50.0), seg(50.0, 60.0)];
        let ctx = ctx_dense(&cfg, &segs);
        let pieces = vec![
            ev(0.0, 5.0, "太短", Some("x")),
            ev(30.0, 60.0, "够长", Some("y")),
        ];
        let (out, stat) = merge_scattered(pieces, &ctx);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].label, "够长");
        assert_eq!(stat.dropped_too_short, 1);
        assert_eq!(stat.kept, 1);
    }

    /// 合并后内容时长仍不足 `merge_min_secs` ⇒ 丢（"依旧不达标就丢弃"）。
    #[test]
    fn merged_group_below_min_content_is_dropped() {
        let cfg = ScatteredConfig {
            enabled: true,
            min_secs: 8.0,
            merge_min_secs: 50.0, // 抬高门槛，逼出"并了也不够"的情形
            merge_min_subs: 1,
            ..Default::default()
        };
        let segs = vec![
            seg(10.0, 15.0),
            seg(15.0, 20.0),
            seg(30.0, 35.0),
            seg(35.0, 40.0),
        ];
        let ctx = ctx_dense(&cfg, &segs);
        let pieces = vec![
            ev(10.0, 20.0, "块A", Some("t")),
            ev(30.0, 40.0, "块B", Some("t")),
        ];
        let (out, stat) = merge_scattered(pieces, &ctx);
        assert!(out.is_empty(), "{out:?}");
        assert_eq!(stat.dropped_content_short, 1);
    }

    /// 字幕条数不足 `merge_min_subs` ⇒ 丢。
    #[test]
    fn merged_group_with_few_subtitles_is_dropped() {
        let cfg = ScatteredConfig {
            enabled: true,
            min_secs: 8.0,
            merge_min_secs: 10.0,
            merge_min_subs: 5,
            ..Default::default()
        };
        let segs = vec![seg(10.0, 15.0), seg(15.0, 20.0)]; // 块内 2 条（过单块门），但 < 5
        let ctx = ctx_dense(&cfg, &segs);
        let pieces = vec![ev(10.0, 20.0, "字幕太少", Some("t"))];
        let (out, stat) = merge_scattered(pieces, &ctx);
        assert!(out.is_empty(), "{out:?}");
        assert_eq!(stat.dropped_few_subs, 1);
    }

    /// `max_merge_secs` 截断：超了就另起一组（不许一路并成整场）。
    #[test]
    fn max_merge_span_splits_group() {
        let cfg = ScatteredConfig {
            enabled: true,
            min_secs: 8.0,
            merge_min_secs: 20.0,
            merge_min_subs: 1,
            max_merge_secs: 100.0,
            ..Default::default()
        };
        let segs = vec![
            seg(0.0, 20.0),
            seg(20.0, 40.0),
            seg(42.0, 62.0),
            seg(62.0, 82.0),
            seg(84.0, 104.0),
            seg(104.0, 124.0),
        ];
        let ctx = ctx_dense(&cfg, &segs);
        let pieces = vec![
            ev(0.0, 40.0, "A", Some("t")),
            ev(42.0, 82.0, "B", Some("t")),
            ev(84.0, 124.0, "C", Some("t")),
        ];
        let (out, stat) = merge_scattered(pieces, &ctx);
        assert_eq!(stat.groups, 2, "第三块加进来会超 100s ⇒ 另起一组：{stat:?}");
        assert_eq!(out.len(), 2, "{out:?}");
        assert!(out[0].end.to_seconds() <= 82.0 + 1e-6);
    }

    /// 弹幕信号门槛：字幕不够但弹幕炸 ⇒ 这块算够格。
    #[test]
    fn piece_qualifies_by_danmaku_when_subtitles_are_thin() {
        let cfg = ScatteredConfig {
            enabled: true,
            merge_min_secs: 10.0,
            merge_min_subs: 99, // 字幕这条永远不满足
            dm_floor: 0.4,
            ..Default::default()
        };
        let segs: Vec<TranscriptSegment> = Vec::new();
        let heat = vec![
            DensityWindow { start: 0.0, density: 0.9, count: 90 },
            DensityWindow { start: 10.0, density: 0.05, count: 3 },
        ];
        let ctx = ScatteredCtx::new(&cfg, &segs).with_heatmap(Some(&heat), 10.0);
        let pieces = vec![ev(0.0, 30.0, "弹幕炸", Some("t"))];
        let (out, _) = merge_scattered(pieces, &ctx);
        // 单块靠弹幕过 piece 那道门；但合并后的"字幕条数"这道门（99）过不去 ⇒ 丢
        assert!(out.is_empty());
        // 换一个宽松的字幕门槛：同一块就能活
        let cfg2 = ScatteredConfig {
            merge_min_subs: 0,
            ..cfg.clone()
        };
        let ctx2 = ScatteredCtx::new(&cfg2, &segs).with_heatmap(Some(&heat), 10.0);
        let (out2, _) = merge_scattered(vec![ev(0.0, 30.0, "弹幕炸", Some("t"))], &ctx2);
        assert_eq!(out2.len(), 1);
    }

    /// 并回普通事件：被盖住（重叠 ≥50%）或紧贴（≤1s）的丢掉，别处的小块留下。
    #[test]
    fn absorb_drops_covered_and_touching_keeps_far() {
        let mut main = vec![ev(100.0, 200.0, "普通", None)];
        let scattered = vec![
            ev(120.0, 160.0, "被盖住", Some("a")),   // 40/40 = 100% ⇒ 丢
            ev(200.5, 230.0, "紧贴", Some("b")),     // gap 0.5s ⇒ 丢
            ev(300.0, 360.0, "中间的独立话题", Some("c")), // 无关 ⇒ 留
        ];
        let mut stat = ScatteredMergeStat::default();
        absorb_scattered(&mut main, scattered, &mut stat);
        assert_eq!(main.len(), 2);
        assert_eq!(main[1].label, "中间的独立话题");
        assert_eq!(stat.absorbed_by_main, 2);
    }

    /// 空隙判据本身：字幕/弹幕任一够 ⇒ 有内容。
    #[test]
    fn gap_content_detection() {
        let cfg = ScatteredConfig::default();
        let segs = vec![seg(10.0, 12.0)];
        let heat = vec![DensityWindow { start: 30.0, density: 0.5, count: 50 }];
        let ctx = ScatteredCtx::new(&cfg, &segs).with_heatmap(Some(&heat), 10.0);
        assert!(ctx.gap_has_content(9.0, 13.0), "字幕够 ⇒ 有内容");
        assert!(!ctx.gap_has_content(20.0, 25.0), "这块什么都没有");
        assert!(ctx.gap_has_content(28.0, 35.0), "弹幕窗口命中 ⇒ 有内容");
        assert!(!ctx.gap_has_content(5.0, 5.0), "空区间");
    }
}

#[cfg(test)]
mod diagnose {
    use super::*;

    /// **只读诊断**：拿一份真实的 `events.json` 当"候选块"跑一遍归组，看看会并成什么样。
    ///
    /// 注意这是**纯函数**演练（不是真跑第二趟 LLM）—— 用真实的标题/时间/热度，
    /// 观察"哪些块会被并成一条、哪些会因太短被丢"，用来校准 `[detection.scattered]` 那几个数。
    ///
    /// ```text
    /// VT_EVENTS_JSON=<项目>/output/events.json \
    ///   cargo test -p yekitrim-core --lib diagnose_scattered_merge -- --ignored --nocapture
    /// ```
    #[test]
    #[ignore]
    fn diagnose_scattered_merge() {
        let Ok(path) = std::env::var("VT_EVENTS_JSON") else {
            eprintln!("用法: VT_EVENTS_JSON=<events.json> cargo test ... diagnose_scattered_merge -- --ignored --nocapture");
            return;
        };
        let raw = std::fs::read_to_string(&path).expect("读取 events.json");
        let store: crate::types::EventStore = serde_json::from_str(&raw).expect("解析 events.json");
        let segments = match std::env::var("VT_TRANSCRIPT_JSON") {
            Ok(p) => {
                let t: crate::types::Transcript =
                    serde_json::from_str(&std::fs::read_to_string(&p).expect("读 transcript"))
                        .expect("解析 transcript");
                t.segments
            }
            Err(_) => Vec::new(),
        };
        // 分散片段靠 `topic` 归组；真实的老 events.json 没有 topic ⇒ 关掉 topic 那道门，
        // 只按"时间相邻 + 时长/字幕门槛"演练（看清门槛本身的效果）。
        let cfg = ScatteredConfig {
            topic_key: false,
            ..Default::default()
        };
        let ctx = ScatteredCtx::new(&cfg, &segments);
        println!("{}", path);
        println!("{}", diag_report(&store.events, &ctx));
    }
}
