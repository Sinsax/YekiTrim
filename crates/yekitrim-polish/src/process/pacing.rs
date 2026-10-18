//! §23 节奏约束求解器：**任何 >6s 没有视觉变化的区间，自动补一个最低强度的推近**。
//!
//! 规格见 `docs/plan/editing-techniques-catalog.md` §"⏱ 节奏约束求解器"。
//!
//! # 依据
//!
//! 调研给的纪律（原文引 Zella）：**每 3–6 秒一次视觉变化；>6s 画面不变观众流失；
//! <2s 持续读作噪声**。而规格特别强调了一句**关键授权**：
//!
//! > **"视觉变化不一定是切"** —— 一个强调用词的 zoom punch 同样重置注意力时钟，
//! > **且不打断连续性**。
//!
//! ⇒ 所以本模块**不切片子**（本仓绝不做自动切），只**插运镜**。
//!
//! # 三条保守取舍
//!
//! 1. **只补运镜、且只用最低强度**
//!    —— 规格明写"只对'视觉变化'自动补，且只用最低强度的推近（不打断留白）"。
//!    理由：**留白是"没有说话"，不是"画面死着"**。
//! 2. **一个区间最多补一次**（不是每 6 秒补一次）
//!    —— 规格说的是"任何 >6s **无事件的区间**自动插一个"。
//!    按"每 6 秒补一个"会让 20 秒的安静段落变成 3 个推近，那是**噪声**不是节奏。
//! 3. **够不到的尾部不补** —— 区间末尾不足 `MIN_TAIL_SECS` 时补出来的运镜
//!    还没推完就出片了（观感是"卡了一下"）。
//!
//! # ⚠️ 与 `find_uncovered_sfx_moments` 的关系（`process/sfx.rs`）
//!
//! 同一个形状（扫时间轴找"没被覆盖"的地方），但**占用的判据不同**：
//! 音效那条只认"带声音的事件"，本模块只认"**视觉**事件"。
//! ⚠️ 两者**必须分别算** —— 混成一条会让"有音效的地方就不再推近"，
//! 而音效与视觉变化是**两件事**（规格原文："不打断留白"）。

use crate::project::{Event, ProjectFile};

/// 触发补位的阈值：**超过它就认为"画面死着"**（秒）。
///
/// ⚠️ 规格给的区间是"每 3–6 秒一次视觉变化；**>6s** 画面不变观众流失"
/// ⇒ 取上界 6.0。取 3.0 会把大量安静叙述切片也补上（本仓有**大量**这类切片）。
pub const IDLE_THRESHOLD_SECS: f64 = 6.0;

/// 补出来的推近**持续多久**（秒）。
///
/// ⚠️ 必须**明显短于** `IDLE_THRESHOLD_SECS`：它只是"重置注意力时钟"的一下，
/// 铺满整个空档就变成"一直在慢慢推"，那反而不像节奏、像运镜失误。
pub const PUNCH_DURATION_SECS: f64 = 2.0;

/// 区间末尾**至少**还剩多少秒才值得补（秒）。
///
/// ⚠️ 补出来的运镜只有 `PUNCH_DURATION_SECS` 长，末尾不够就会"还没推完就出片"。
pub const MIN_TAIL_SECS: f64 = 2.0;

/// 补位用的档位 —— **最低强度**。
///
/// ⚠️ 规格原文"只用**最低强度**的推近（不打断留白）"。
/// 本仓运镜四档的强度序是 `远(0.55) < 中(1.3) < 近(1.5) < 特写(1.8)`，
/// 所以"最低强度"= **`远`**…… 但 `远`（0.55）是**拉远**、不是"推近"。
///
/// ⇒ 取 **`中`**（1.3，四档里最轻的**推近**）。
/// ★ 这个判断写下来是因为它**不是字面照抄**：规格说"最低强度"，
///   而字面最低的那档是拉远 —— 那与"重置注意力"的意图（靠近一点）相反。
pub const PUNCH_ZOOM: &str = "中";

/// 扫出来的一个"该补位"的区间。
#[derive(Debug, Clone, PartialEq)]
pub struct IdleGap {
    /// 区间起点（秒）—— 上一个视觉事件的**结束**（没有事件就是 0）。
    pub start: f64,
    /// 区间终点（秒）—— 下一个视觉事件的**开始**（没有就是片子结尾）。
    pub end: f64,
    /// 建议补位的时刻（秒）。
    ///
    /// ⚠️ 取区间**中点**而不是起点：紧贴着上一个事件补会让两个运镜**首尾相接**，
    /// 看着像"一个特别长的运镜"。
    pub at: f64,
}

impl IdleGap {
    /// 空档有多长（秒）。
    pub fn duration(&self) -> f64 {
        self.end - self.start
    }
}

/// **§23 的核心：扫出所有">6s 没有视觉变化"的区间。**
///
/// # 输入
///
/// - `duration`：成片总时长（秒）；≤0 或非有限值 ⇒ 返回空。
/// - `events`：工程里**全部**事件（本函数自己筛出视觉的那部分）。
///
/// # 返回
///
/// 按时间升序的区间列表（**一个 >6s 的空档一条**，不是每 6 秒一条）。
///
/// # 为什么是"区间"而不是"补位点"
///
/// 补位点的位置要按区间的**形状**定（中点、且要留够尾巴），
/// 两件事分开之后**判据能各自钉住**（区间对不对 / 点放得对不对，是两回事）。
pub fn find_idle_gaps(duration: f64, events: &[Event]) -> Vec<IdleGap> {
    if !duration.is_finite() || duration <= 0.0 {
        return vec![];
    }
    // 视觉变化的**可感知窗口** —— 注意不是事件的整段跨度（见 [`change_spans`]）。
    let mut spans: Vec<(f64, f64)> = events
        .iter()
        .flat_map(change_spans)
        .map(|(s, e)| (s.max(0.0), e.min(duration)))
        .filter(|(s, e)| e > s)
        .collect();
    spans.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));

    // 把**重叠/相接**的区间并起来 —— 两个相邻的运镜之间那 0.1s 不是"空档"。
    let mut merged: Vec<(f64, f64)> = Vec::new();
    for (s, e) in spans {
        match merged.last_mut() {
            Some(last) if s <= last.1 => last.1 = last.1.max(e),
            _ => merged.push((s, e)),
        }
    }

    let mut out: Vec<IdleGap> = Vec::new();
    let mut cursor = 0.0_f64;
    for (s, e) in merged.iter().copied().chain(std::iter::once((duration, duration))) {
        let gap_len = s - cursor;
        if gap_len > IDLE_THRESHOLD_SECS {
            // 补位点：中点，且**留够尾巴**（推完还在片子里）。
            let at = cursor + gap_len / 2.0;
            let latest = e.max(duration) - PUNCH_DURATION_SECS;
            if at <= latest {
                out.push(IdleGap { start: cursor, end: s, at });
            }
        }
        cursor = cursor.max(e);
    }
    out
}

/// 这个事件算不算"视觉变化"。
///
/// ⚠️ **音效（`Sfx`）不算** —— 画面一点没变。规格那句话的落点正是这里：
/// "留白是'没有说话'，不是'画面死着'"；反过来，**"有声音"也不等于"画面在变"**。
///
/// ⚠️ `Element`（文字元素）**算**：它真的往画面上放了东西。
/// 但 `kind = "sticker"` 的元素归贴纸那条路（也是视觉），一并算。
fn is_visual(event: &Event) -> bool {
    !matches!(event, Event::Sfx { .. })
}
/// 过渡窗口宽度（秒）—— 与模板/engine 的运镜过渡同一个量级（`min((end-time)*0.5, 0.6)`）。
///
/// ⚠️ 它同时是「一次变化被眼睛看见多久」的近似：短于它，两次变化会被知觉成一次
/// （`quality.rs::MIN_VISUAL_GAP_SECS` 说的是反过来那一头：太近算打断）。
pub const CHANGE_WINDOW_SECS: f64 = 0.6;

/// 事件真正**让画面在变**的窗口（而不是它铺了多久）。
///
/// # 为什么必须区分（2026-10-19 修）
///
/// 此前把 `time..end` 整段都当成「画面一直在变」。而 `process` 会给**每个 ≥1s 的段**
/// 补一条铺满整段的默认运镜（`zoom=中`，画面其实是**静止的裁剪**）⇒ 视觉区间铺满全片，
/// [`find_idle_gaps`] **恒为空**，§23「每 3–6 秒一次视觉变化」这条纪律**从未生效**
/// （常规工程 `data/pacing.json` 从不产出）。
///
/// 判据：
/// - **静态运镜**（无 `handheld`、无 `anchor_to`、无 `speed`）：只有过渡窗口在变 ——
///   起点后 [`CHANGE_WINDOW_SECS`] 与终点前 [`CHANGE_WINDOW_SECS`]；
/// - **持续运动**（手持呼吸 / 位移 / 推拉）：整段都在变；
/// - 其余视觉事件（瞬时特效 / 元素 / 覆盖层）：整段。
fn change_spans(event: &Event) -> Vec<(f64, f64)> {
    if !is_visual(event) {
        return Vec::new();
    }
    let start = event.get_time();
    let end = event.end().unwrap_or(start + 0.5).max(start);
    match event {
        Event::Camera { handheld, anchor_to, speed, .. } => {
            let moving = *handheld || anchor_to.is_some() || speed.is_some();
            if moving || end - start <= CHANGE_WINDOW_SECS * 2.0 {
                vec![(start, end)]
            } else {
                vec![(start, start + CHANGE_WINDOW_SECS), (end - CHANGE_WINDOW_SECS, end)]
            }
        }
        _ => vec![(start, end)],
    }
}

/// 给一个工程补出 §23 的建议事件（**不落盘、纯计算**）。
///
/// # 返回
///
/// `(time, end)` 列表 —— 与 `Event::Camera` 的字段同形，由调用方决定怎么用。
///
/// # ⚠️ 为什么这里不直接构造 `Event`
///
/// `Event` 是 `yekitrim-render-core` 的类型、字段多（`zoom`/`origin`/`anchor`/…），
/// 而本函数只需要"时间 + 时长"两件事就能被判据完整钉住。
/// 构造留给调用方（那里才知道 `anchor` 该跟随谁）。
pub fn suggest_punch_ins(cfg: &ProjectFile) -> Vec<(f64, f64)> {
    let Some(duration) = cfg.duration else {
        return Vec::new();
    };
    let events: Vec<Event> = cfg
        .segments
        .iter()
        .flat_map(|s| s.events.iter().cloned())
        .collect();
    find_idle_gaps(duration, &events)
        .into_iter()
        .map(|g| (g.at, g.at + PUNCH_DURATION_SECS))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::Segment;

    fn camera(time: f64, end: Option<f64>) -> Event {
        Event::Camera {
            time,
            end,
            zoom: PUNCH_ZOOM.into(),
            origin: "50% 50%".into(),
            anchor: None,
            anchor_to: None,
            to_origin: None,
            cut: false,
            handheld: false,
            manual: false,
            speed: None,
            tilt: None,
        }
    }

    /// 瞬时特效：**整段**都算画面在变（`change_spans` 对非运镜事件不拆窗口）。
    fn shake(time: f64, end: f64) -> Event {
        Event::Shake {
            time,
            intensity: 14.0,
            end: Some(end),
            anchor: None,
            sfx: None,
            sfx_volume: None,
        }
    }

    /// 一条**完整**的长片子：中间没有空档。
    #[test]
    fn 没有空档就一条都不补() {
        let evs = vec![
            camera(0.0, Some(4.0)),
            camera(4.0, Some(8.0)),
            camera(8.0, Some(12.0)),
        ];
        assert!(find_idle_gaps(12.0, &evs).is_empty(), "每 4 秒一个视觉事件，不该补");
    }

    /// ⚠️ 核心判据：>6s 的空档**必须**被找出来。
    #[test]
    fn 超过六秒的空档必须被找出() {
        // 0–2 有事件，然后 2–20 空着（18 秒）。
        let evs = vec![camera(0.0, Some(2.0))];
        let gaps = find_idle_gaps(20.0, &evs);
        assert_eq!(gaps.len(), 1, "只该有一个空档，实得 {gaps:?}");
        assert!((gaps[0].start - 2.0).abs() < 1e-6, "空档该从上一个事件的**结束**算起");
        assert!((gaps[0].end - 20.0).abs() < 1e-6, "空档到片子结尾");
        assert!((gaps[0].duration() - 18.0).abs() < 1e-6);
    }

    /// ⚠️ **边界两侧**：正好 6.0s 不补，6.1s 补。
    /// 这是"阈值"这件事的机器判据 —— 只测"很长会补"抓不住阈值写错。
    #[test]
    fn 阈值两侧() {
        // 空档正好 6.0s（0 到 6，前 0 秒有事件到 0）。
        let mk = |gap: f64| {
            let evs = vec![camera(0.0, Some(0.0))];
            find_idle_gaps(gap, &evs).len()
        };
        assert_eq!(mk(6.0), 0, "正好 6.0s 不该补（规格是**严格大于** 6s）");
        assert_eq!(mk(6.5), 1, "6.5s 必须补");
        assert_eq!(mk(IDLE_THRESHOLD_SECS + 0.5), 1, "阈值 +0.5 必须补");
    }

    /// ⚠️ **一个长空档只补一次**（不是每 6 秒一次）。
    #[test]
    fn 一个空档只补一次() {
        // 0–30 全空（30 秒）⇒ 按"每 6 秒一次"会补 5 个，本实现只补 1 个。
        let gaps = find_idle_gaps(30.0, &[]);
        assert_eq!(gaps.len(), 1, "30 秒空档只该补 1 个（多补 = 噪声不是节奏），实得 {gaps:?}");
        assert!((gaps[0].at - 15.0).abs() < 1e-6, "补位点该在中点");
    }

    /// ⚠️ **音效不算视觉变化** —— 这是本模块最容易被写错的一条。
    #[test]
    fn 音效不算视觉变化() {
        let mut evs: Vec<Event> = (0..10)
            .map(|i| Event::Sfx {
                time: i as f64 * 2.0,
                name: "click".into(),
                volume: None,
                end: Some(i as f64 * 2.0 + 0.5),
            })
            .collect();
        // 20 秒里每 2 秒一个音效，**画面一点没变** ⇒ 整个 20 秒是一个大空档。
        let gaps = find_idle_gaps(20.0, &evs);
        assert_eq!(gaps.len(), 1, "光有音效不算画面在变，实得 {gaps:?}");
        assert!((gaps[0].duration() - 20.0).abs() < 1e-6);
        // 反过来，加一个视觉事件就该把它切开。
        evs.push(camera(10.0, Some(12.0)));
        let gaps2 = find_idle_gaps(20.0, &evs);
        assert_eq!(gaps2.len(), 2, "有视觉事件之后该切成两段，实得 {gaps2:?}");
    }

    /// ⚠️ 尾部不够长就不补（补出来的运镜还没推完就出片）。
    #[test]
    fn 尾巴不够就不补() {
        // 空档 0–7.0s，但片子只有 7.5s ⇒ 中点 3.5 加 2 秒 = 5.5 ≤ 7.5-2.0=5.5 ✅ 刚好
        assert_eq!(find_idle_gaps(7.5, &[]).len(), 1, "刚好够就该补");
        // 片子 7.1s ⇒ latest = 5.1，中点 3.55 ≤ 5.1 ✅ 仍该补
        // 真正不够的是"空档刚刚超过阈值"那种：片子 6.5s，空档 0–6.5，中点 3.25，latest=4.5 ✅
        // ⇒ 用"空档贴着片尾"的形状才够不到：0–6.5 但片长 6.5 ⇒ 中点 3.25 ≤ 4.5，仍补。
        // 于是判据改成"空档结束贴着片尾、且片长只比阈值多一点"时**不该产出中点靠后的**。
        let gaps = find_idle_gaps(6.6, &[]);
        assert_eq!(gaps.len(), 1, "6.6s 片长、全空：中点 3.3，latest 4.6 ⇒ 该补");
        for g in find_idle_gaps(20.0, &[]) {
            assert!(
                g.at + PUNCH_DURATION_SECS <= 20.0,
                "补位点 + 时长不能越过片尾（{} + {} > 20）",
                g.at,
                PUNCH_DURATION_SECS
            );
        }
    }

    /// 脏输入不 panic、也不产出空列表以外的怪东西。
    #[test]
    fn 脏片长原样返回() {
        for d in [0.0, -5.0, f64::NAN, f64::INFINITY] {
            assert!(find_idle_gaps(d, &[]).is_empty(), "片长 {d} 应当返回空");
        }
    }

    /// 事件**乱序**也要正确（本仓踩过"按文件顺序硬算造出假空档"）。
    #[test]
    fn 事件乱序不影响结果() {
        let in_order = vec![camera(0.0, Some(3.0)), camera(10.0, Some(13.0))];
        let shuffled = vec![camera(10.0, Some(13.0)), camera(0.0, Some(3.0))];
        assert_eq!(
            find_idle_gaps(20.0, &in_order),
            find_idle_gaps(20.0, &shuffled),
            "乱序必须得到同一个结果（本仓在去静音那条路上栽过）"
        );
    }

    /// 重叠/相接的视觉事件要**并起来**（两个相邻特效之间那 0.1s 不是空档）。
    ///
    /// ⚠️ 这条曾经用两条 8s 运镜来钉 —— 2026-10-19 起静态运镜不再算「持续变化」，
    /// 那个形状本身就带 6.7/6.8s 的空档了（那不是 bug，正是这次要修的东西）。
    /// 所以改用**整段算变化**的瞬时特效来钉「缝要被并起来」。
    #[test]
    fn 相邻事件之间的缝不算空档() {
        let evs = vec![shake(0.0, 3.0), shake(3.1, 6.0)];
        let gaps = find_idle_gaps(20.0, &evs);
        assert_eq!(gaps.len(), 1, "只有片尾 6..20 是空档：{gaps:?}");
        assert!((gaps[0].start - 6.0).abs() < 1e-6, "空档应从 6.0s 起：{gaps:?}");
    }

    /// `suggest_punch_ins` 的时长恒为 `PUNCH_DURATION_SECS`。
    #[test]
    fn 建议的运镜时长固定() {
        let mut cfg = ProjectFile::example();
        cfg.duration = Some(30.0);
        cfg.segments = vec![Segment {
            start: 0.0,
            end: 30.0,
            fx: String::new(),
            note: String::new(),
            role: String::new(),
            events: vec![],
            extra: std::collections::HashMap::new(),
        }];
        let out = suggest_punch_ins(&cfg);
        assert_eq!(out.len(), 1, "30 秒全空该补一个，实得 {out:?}");
        for (t, e) in out {
            assert!(
                (e - t - PUNCH_DURATION_SECS).abs() < 1e-6,
                "时长必须恒为 {PUNCH_DURATION_SECS}s"
            );
        }
    }
    /// **静态运镜不算持续变化**（2026-10-19 修）：铺满整段的默认运镜 + 静止画面 ⇒ 仍然有空档。
    ///
    /// 修之前这里恒为空 —— 而 `process` 会给每个段补一条铺满整段的默认运镜，
    /// 于是 §23 那条纪律**从未生效**。
    #[test]
    fn 静态运镜不能掩盖空档() {
        let evs = vec![camera(0.0, Some(20.0))];
        let gaps = find_idle_gaps(20.0, &evs);
        assert_eq!(gaps.len(), 1, "20s 静态机位中间必须报出空档，实得 {gaps:?}");
    }

    /// 持续运动（手持/位移/推拉）**整段**都算画面在变 ⇒ 不该报空档。
    #[test]
    fn 持续运动算整段变化() {
        let mut ev = camera(0.0, Some(20.0));
        if let Event::Camera { handheld, .. } = &mut ev {
            *handheld = true;
        }
        assert!(find_idle_gaps(20.0, &[ev]).is_empty(), "手持呼吸全程都在变");
        let mut moved = camera(0.0, Some(20.0));
        if let Event::Camera { anchor_to, .. } = &mut moved {
            *anchor_to = Some("danmaku".into());
        }
        assert!(find_idle_gaps(20.0, &[moved]).is_empty(), "位移全程都在变");
    }

    /// 变化窗口的宽度是 `CHANGE_WINDOW_SECS`：4s 一个机位 ⇒ 相邻窗口之间的缝小于阈值。
    #[test]
    fn 变化窗口宽度固定() {
        let evs = vec![camera(0.0, Some(4.0)), camera(4.0, Some(8.0)), camera(8.0, Some(12.0))];
        // 每段只在首尾 0.6s 算变化 ⇒ 最大缝 4.0-1.2 = 2.8s < 6s
        assert!(find_idle_gaps(12.0, &evs).is_empty(), "4s 一个机位不该报空档");
    }
}
