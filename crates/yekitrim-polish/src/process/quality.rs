//! 剪辑质量分（总方案 §3）：给"这条切片剪得好不好"一个**自动可算的数**。
//!
//! # 出处
//!
//! DramaAD（短剧广告评测）用 [`Smoothness`] 衡量"观众多久被打断一次"：
//!
//! ```text
//! Smoothness = Duration(V) / (1 + Interruption(V))
//! Interruption  = 可感知的干扰次数
//! ```
//!
//! 它能给"剪得好不好"一个**不需要人评**的数 —— 规格 §7.1 把它列为本仓"可以立刻用上"的东西，
//! 本仓此前**完全没有**这类量化。
//!
//! ⚠️ **论文的阈值是广告投放场景的，本仓的口径不同**：那边 `Interruption` 含"钩子强度"
//! 这种要人评的项，这里**只取四条机器能判的**（规格 §7.1 那张表的右列）：
//!
//! | # | 干扰项 | 本仓怎么判 |
//! |---|---|---|
//! | 1 | 边界落在句子中间 | 字幕文本判（复用 [`crate::process::fx::is_sentence_end`]） |
//! | 2 | 切在爆点上 | `Segment.fx == "high"` 判（LLM 标的情绪档） |
//! | 3 | 静默突变 | 音轨判 ⇒ **见下面的 ⚠️** |
//! | 4 | 两次视觉变化间隔 < 0.8s | 事件表判（同轨相邻事件的间隔） |
//!
//! # ⚠️ 本趟**没做**第 3 条（静默突变），这是有意的
//!
//! 它有真实数据源（`data/loudness.json`），但要读文件 ⇒ 这个模块就不是纯函数了；
//! 而"什么时候算突变"需要先**标定阈值**（规格没给数），拍一个数进去等于把一条
//! **没验证过的判据**混进总分 —— 分数会因此偏低，而用户无从知道是为什么。
//! ⇒ 按"保守方案"先只算**能纯函数判的三条**，静默那一项留在 TODO（见保守决策记录）。
//! 分数会**偏高**（分母少一项），但每一项都站得住 —— 报一个偏低且说不清的分数更糟。
//!
//! # 这个分数**不是**"好不好看"
//!
//! 它只数**可感知的打断**。特效少、运镜平实的片子照样能拿高分 —— 那是设计意图：
//! 规格 §8 的结论正是"剪辑质量的差距主要不在特效，而在策略"。

use crate::parse::srt::Subtitle;
use crate::project::{Event, Segment};

/// 两次视觉变化间隔小于它就算一次"干扰"（秒）。
///
/// **0.8s 的出处**：人眼把连续的两次画面变化**知觉成一次**的门槛量级。
/// 规格 §7.1 直接给了这个数（"两次视觉变化间隔 < 0.8s"），本仓照用 ——
/// ⚠️ 但它是**论文口径的沿引、不是本仓实测标定的**。要调就调这里一处。
pub const MIN_VISUAL_GAP_SECS: f64 = 0.8;

/// 一次"干扰"的类别（报告里分组用；也方便将来只统计某一类）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InterruptionKind {
    /// 分段边界切在一句话中间。
    MidSentence,
    /// 分段起点落在爆点段内部。
    OnPunch,
    /// 同轨两次视觉变化太近。
    RapidChange,
}

impl InterruptionKind {
    /// 稳定英文标识（进 JSON，给前端/判据用）。
    pub fn key(self) -> &'static str {
        match self {
            Self::MidSentence => "mid_sentence",
            Self::OnPunch => "on_punch",
            Self::RapidChange => "rapid_change",
        }
    }

    /// 一句话人话理由（界面上"这条为什么扣分"）。
    pub fn label(self) -> &'static str {
        match self {
            Self::MidSentence => "分段边界切在一句话中间",
            Self::OnPunch => "分段从爆点中间开始",
            Self::RapidChange => "同一条轨道上两次特效挨得太近",
        }
    }
}

/// 一处具体的干扰（带位置，界面可跳转）。
#[derive(Debug, Clone, PartialEq)]
pub struct Interruption {
    pub kind: InterruptionKind,
    /// 发生时刻（秒）。
    pub at: f64,
}

/// 一条切片的剪辑质量（§3）。
#[derive(Debug, Clone, PartialEq)]
pub struct Quality {
    /// 片子时长（秒）。`Smoothness` 的分子。
    pub duration: f64,

    /// 视觉特效/覆盖层事件总数（**分母的"素材量"**，见 [`Self::smoothness`]）。
    pub visual_events: usize,

    /// 逐条干扰。
    pub interruptions: Vec<Interruption>,

    /// 分子：起止时间都在片内的分段数（`Segment` 就是一次"呈现"）。
    pub segments: usize,
}

impl Quality {
    /// 干扰总数（`Interruption(V)`）。
    pub fn interruption_count(&self) -> usize {
        self.interruptions.len()
    }

    /// **Smoothness** = `Duration / (1 + Interruption)`（论文原式）。
    ///
    /// 无干扰时 = `Duration`（越长越高），这与论文一致 —— 它衡量的是
    /// "观众平均被打断一次的间隔"，不是"每条片子的绝对好坏"。
    /// ⚠️ 所以**别拿它跨时长比**（60s 的片子天然比 10s 的高）；要横比就用
    /// [`Self::interruptions_per_minute`]。
    pub fn smoothness(&self) -> f64 {
        self.duration / (1.0 + self.interruption_count() as f64)
    }

    /// 每分钟干扰数 —— **跨时长可比**的那个数（越高越差）。
    ///
    /// 为什么还要它：`Smoothness` 的分子是时长，一条 60s 零干扰的片子（=60）
    /// 永远压过一条 10s 零干扰的片子（=10），但两者其实**都没有干扰**。
    /// 切片列表里横比要看这个。
    pub fn interruptions_per_minute(&self) -> f64 {
        if self.duration <= 0.0 {
            return 0.0;
        }
        self.interruption_count() as f64 * 60.0 / self.duration
    }

    /// 0~100 的**展示分**（高 = 好）。界面上用这个，别直接显示 `smoothness`
    /// （那个数没有上界、单位不直观）。
    ///
    /// 换算：每分钟 0 次 = 100 分；**每分钟 6 次 = 0 分**，线性。
    /// ⚠️ 6 是**拍的口径**（规格没给）—— 它把"每 10 秒就被打断一次"定为不可接受。
    /// 真实直播切片里这个档位对应"满屏乱抖"，是保守一侧的选择：宁可分数虚高
    /// 也别让用户为了凑分去删特效。
    pub fn score(&self) -> f64 {
        let per_min = self.interruptions_per_minute();
        (100.0 * (1.0 - per_min / 6.0)).clamp(0.0, 100.0)
    }

    /// 按类别分组的计数（报告/界面分组用）。
    pub fn by_kind(&self) -> Vec<(InterruptionKind, usize)> {
        let mut out: Vec<(InterruptionKind, usize)> = Vec::new();
        for k in [
            InterruptionKind::MidSentence,
            InterruptionKind::OnPunch,
            InterruptionKind::RapidChange,
        ] {
            let n = self.interruptions.iter().filter(|i| i.kind == k).count();
            if n > 0 {
                out.push((k, n));
            }
        }
        out
    }
}

/// 算一条切片的剪辑质量分（§3）。**纯函数** —— 没有 IO、没有 LLM，可单测、可重放。
///
/// # 参数
///
/// - `segments`：已经过 `process` 的分段表（边界是**最终**的 —— 分数必须反映成片，
///   不是反映 LLM 的草稿）；
/// - `subtitles`：这条片子的字幕（句子判据的唯一来源）；
/// - `duration`：片子时长（秒）。传 0 也能算（`Smoothness` 退化成 0，
///   `score` 靠 `interruptions_per_minute` 的除零保护给出 0 分 —— 见那里的 `if`）。
pub fn evaluate(segments: &[Segment], subtitles: &[Subtitle], duration: f64) -> Quality {
    let mut interruptions: Vec<Interruption> = Vec::new();

    // ① 边界切在句子中间。
    //
    // ⚠️ **只看"内部边界"**：首段的 start 与末段的 end 是片子的开头/结尾，
    //    不是剪辑点（没有"前一秒"可言）。把它们也算进去的话，**每条片子**
    //    都会因为"片头正好落在某句话中间"白扣一次 —— 那是片源决定的，不是剪辑决定的。
    for w in segments.windows(2) {
        let t = w[0].end;
        // 两段不接壤（有缝/重叠）⇒ 这里根本不是一个"切点"，不判。
        if (t - w[1].start).abs() > 1e-6 {
            continue;
        }
        if !boundary_feels_complete(t, subtitles) {
            interruptions.push(Interruption { kind: InterruptionKind::MidSentence, at: t });
        }
    }

    // ② 切在爆点上：分段**起点**落在 `fx == "high"` 的段内部。
    //
    // ⚠️ 判的是"起点落在**别的**爆点段内部"，不是"这段自己是爆点" ——
    //    后者是正常的（爆点当然要有爆点段），前者才是"从半截开始"。
    for seg in segments.iter() {
        let t = seg.start;
        let inside_other_punch = segments
            .iter()
            .any(|s| s.fx == "high" && s.start < t - 1e-6 && t < s.end - 1e-6);
        if inside_other_punch {
            interruptions.push(Interruption { kind: InterruptionKind::OnPunch, at: t });
        }
    }

    // ③ 同轨两次视觉变化间隔 < 0.8s。
    //
    // ⚠️ **按"事件类型"分轨，不按时间轴泳道**：泳道把 11 种特效都塞进 `fx` 一条，
    //    而"闪一下 + 抖一下"同时发生是**有意的组合**、不是干扰；
    //    同一种特效 0.3s 内来两次才是"看着在抽搐"。
    // ⇒ 先按类型分组，再在组内比相邻间隔。
    let mut by_type: Vec<(&'static str, Vec<f64>)> = Vec::new();
    for seg in segments {
        for ev in seg.events.iter().filter(|e| is_visual(e)) {
            let k = event_kind(ev);
            match by_type.iter_mut().find(|(name, _)| *name == k) {
                Some((_, v)) => v.push(ev.get_time()),
                None => by_type.push((k, vec![ev.get_time()])),
            }
        }
    }
    for (_, mut times) in by_type {
        times.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        for w in times.windows(2) {
            if w[1] - w[0] < MIN_VISUAL_GAP_SECS {
                interruptions.push(Interruption {
                    kind: InterruptionKind::RapidChange,
                    at: w[1],
                });
            }
        }
    }

    // 干扰按时间排序（报告按时间读更自然，也让"同一处多次"看起来是连续的）。
    interruptions.sort_by(|a, b| a.at.partial_cmp(&b.at).unwrap_or(std::cmp::Ordering::Equal));

    Quality {
        duration,
        visual_events: segments
            .iter()
            .flat_map(|s| s.events.iter())
            .filter(|e| is_visual(e))
            .count(),
        interruptions,
        segments: segments.len(),
    }
}

/// 这个切点**感觉上**是一句话说完了吗（§3 判据 ①）。
///
/// 复用 §2 的 [`crate::process::fx::is_sentence_end`]（**同一个判据，不许抄第二份**），
/// 但口径比它宽松一档：§2 是"要不要**动手挪**"（宁可不挪），这里是"用户**会不会觉得**被切断"
/// （宁可算成干净）。差异在空隙：§2 要求缝隙宽到能承载一个切点（1.0s），
/// 这里只要**落在一个缝隙里**就算没切断句子。
fn boundary_feels_complete(t: f64, subs: &[Subtitle]) -> bool {
    // ⚠️ 判据顺序：先"落在缝隙里"再"落在字幕内部" —— 字幕的**结束点**同时满足两个条件，
    //    先判包含就会把它当成"这句话的内部"（§2 踩过同一个坑，见 fx.rs 的注释）。
    let inside = subs.iter().any(|s| s.start < t && t < s.end);
    if inside {
        let s = subs
            .iter()
            .find(|s| s.start < t && t < s.end)
            .expect("上面 any 已确认存在");
        return crate::process::fx::is_sentence_end(&s.text);
    }
    true
}

/// 这个事件**在画面上有可见变化**吗（§3 判据 ③ 的输入）。
///
/// ⚠️ **音效不算**：它听得见、不改变画面，而这条判据数的是"视觉变化"。
/// 把 `Sfx` 算进来的话，一段配了音效的安静画面会被判成"抽搐"。
pub fn is_visual(ev: &Event) -> bool {
    !matches!(ev, Event::Sfx { .. })
}

/// 事件类型名（分轨用）。
///
/// ⚠️ **不复用 `fx::event_type_name`**（那个是 `pub(super)` 且语义是"瞬时特效跨度"）：
/// 两者现在值一样、将来未必 —— 那边要的是"哪几种特效有跨度上限"，这里要的是
/// "哪些事件画在同一个视觉通道上"。绑在一起改一边就得动另一边。
fn event_kind(ev: &Event) -> &'static str {
    match ev {
        Event::Sticker { .. } => "sticker",
        Event::Camera { .. } => "camera",
        Event::Shake { .. } => "shake",
        Event::Flash { .. } => "flash",
        Event::Blur { .. } => "blur",
        Event::Vignette { .. } => "vignette",
        Event::HueShift { .. } => "hue_shift",
        Event::Noise { .. } => "noise",
        Event::Pulse { .. } => "pulse",
        Event::Overlay { .. } => "overlay",
        Event::Sfx { .. } => "sfx",
        // ⚠️ 元素按**它自己的 `kind`** 分轨，而不是统一回一个 "element"：
        // 这个函数要回答的是"哪些事件画在同一个视觉通道上"，而文字与贴纸在成片里
        // 是**两种不同的东西**（文字是排版、贴纸是图片），质量分要能把它们分开统计。
        // 统一回 "element" 会让"这一段的元素密度"把两类混成一个数。
        Event::Element { kind, .. } => match kind.as_str() {
            "text" => "text",
            "sticker" => "sticker",
            // 认不出的 kind 仍回 `element`：既不是已知的哪一类，也**不许**被静默算进
            // 上面两类里（那会让质量分说"贴纸很多"而其实是写错了 kind 的元素）。
            _ => "element",
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::Segment;

    fn sub(i: usize, start: f64, end: f64, text: &str) -> Subtitle {
        Subtitle { index: i, start, end, text: text.into(), html: String::new() }
    }

    fn seg(start: f64, end: f64, fx: &str, events: Vec<Event>) -> Segment {
        Segment {
            role: String::new(),
            start,
            end,
            fx: fx.into(),
            note: String::new(),
            events,
            extra: Default::default(),
        }
    }

    fn sticker(time: f64) -> Event {
        Event::Sticker {
            time,
            end: Some(time + 0.5),
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

    /// **零干扰 ⇒ 满分**，且 `Smoothness` 退化成纯时长（论文原式）。
    #[test]
    fn clean_cut_scores_full_marks() {
        let subs = vec![sub(1, 0.0, 2.0, "第一句。"), sub(2, 4.0, 6.0, "第二句。")];
        // 边界 3.0 落在 2.0~4.0 的空隙里（没切断任何句子）
        let segs = vec![seg(0.0, 3.0, "low", vec![]), seg(3.0, 8.0, "med", vec![])];
        let q = evaluate(&segs, &subs, 8.0);
        assert_eq!(q.interruption_count(), 0, "不该有干扰：{:?}", q.interruptions);
        assert_eq!(q.smoothness(), 8.0, "无干扰时 Smoothness = Duration");
        assert_eq!(q.score(), 100.0, "零干扰 = 满分");
        assert!(q.by_kind().is_empty());
    }

    /// **判据 ①**：边界切在一句话中间 ⇒ 一次干扰。
    ///
    /// 变异自证：把 `boundary_feels_complete` 写成恒 `true` ⇒ 本条翻红。
    #[test]
    fn mid_sentence_boundary_is_an_interruption() {
        // 边界 1.0 落在 `0.5~3.0` 这条**内部**，且那句话没有句末标点
        let subs = vec![sub(1, 0.5, 3.0, "这话还没说完")];
        let segs = vec![seg(0.0, 1.0, "low", vec![]), seg(1.0, 5.0, "med", vec![])];
        let q = evaluate(&segs, &subs, 5.0);
        assert_eq!(q.interruption_count(), 1, "切在句子中间要算一次");
        assert_eq!(q.interruptions[0].kind, InterruptionKind::MidSentence);
        assert_eq!(q.interruptions[0].at, 1.0);
        assert!(q.score() < 100.0, "扣分了，分数不该还是满分");
    }

    /// **片头/片尾不算切点**：首段 start / 末段 end 落在句子中间**不扣分**
    /// （那是片源决定的，不是剪辑决定的）。
    ///
    /// ⚠️ 这条防的是"每条片子都白扣一次" —— 不防的话分数的**基线全体下移**，
    /// 用户会觉得这个数没用。
    #[test]
    fn clip_edges_are_not_edit_points() {
        // 首段 start=0.5 与末段 end=3.0 都落在同一条字幕内部
        let subs = vec![sub(1, 0.5, 3.0, "整条都没说完")];
        let segs = vec![seg(0.5, 2.0, "low", vec![]), seg(2.0, 3.0, "low", vec![])];
        let q = evaluate(&segs, &subs, 3.0);
        // 内部切点是 2.0 —— 它落在字幕内部且无句末标点 ⇒ 只该扣这**一次**
        assert_eq!(q.interruption_count(), 1, "只该扣内部那个切点：{:?}", q.interruptions);
        assert_eq!(q.interruptions[0].at, 2.0, "片头 0.5 与片尾 3.0 都不许算");
    }

    /// **不接壤的分段之间不是切点**（有缝/重叠 ⇒ 那里本来就没有"剪"这个动作）。
    #[test]
    fn non_adjacent_segments_are_not_cut_points() {
        let subs = vec![sub(1, 0.0, 10.0, "整条都没说完")];
        // 两段中间有 2s 的缝
        let segs = vec![seg(0.0, 3.0, "low", vec![]), seg(5.0, 10.0, "low", vec![])];
        let q = evaluate(&segs, &subs, 10.0);
        assert_eq!(q.interruption_count(), 0, "有缝 ⇒ 3.0 处不是切点：{:?}", q.interruptions);
    }

    /// **判据 ②**：分段起点落在**别的**爆点段内部 ⇒ 一次干扰。
    /// 但"这段自己就是爆点"**不算**（那正是爆点该有的样子）。
    #[test]
    fn starting_inside_another_punch_is_an_interruption() {
        // 爆点段 2.0~6.0；第二段从 4.0 开始 ⇒ 落在它内部
        let segs = vec![seg(2.0, 6.0, "high", vec![]), seg(4.0, 9.0, "med", vec![])];
        let q = evaluate(&segs, &[], 9.0);
        assert_eq!(q.interruption_count(), 1, "{:?}", q.interruptions);
        assert_eq!(q.interruptions[0].kind, InterruptionKind::OnPunch);

        // 反例：只有一段、它自己就是爆点 ⇒ 不许扣分
        let sole = vec![seg(0.0, 5.0, "high", vec![])];
        assert_eq!(
            evaluate(&sole, &[], 5.0).interruption_count(),
            0,
            "爆点段自己开头不算'从半截开始'"
        );
    }

    /// **判据 ③**：同一种特效 0.8s 内来两次 ⇒ 一次干扰。
    ///
    /// ⚠️ **不同特效同时发生不算**：那是**有意的组合**（闪 + 抖），不是抽搐。
    /// 变异自证：把分轨去掉、所有视觉事件混在一起比间隔 ⇒ 本测试的第二段翻红。
    #[test]
    fn rapid_repeats_are_interruptions_but_stacking_is_not() {
        // 同一种（sticker）0.3s 内两次 ⇒ 一次干扰
        let rapid = vec![seg(0.0, 10.0, "med", vec![sticker(1.0), sticker(1.3)])];
        let q = evaluate(&rapid, &[], 10.0);
        assert_eq!(q.interruption_count(), 1, "{:?}", q.interruptions);
        assert_eq!(q.interruptions[0].kind, InterruptionKind::RapidChange);
        assert_eq!(q.interruptions[0].at, 1.3);

        // 间隔够开 ⇒ 不算
        let spaced = vec![seg(0.0, 10.0, "med", vec![sticker(1.0), sticker(2.5)])];
        assert_eq!(evaluate(&spaced, &[], 10.0).interruption_count(), 0);

        // 不同**类型**挨着 ⇒ 不算（有意的组合）
        let stacked = vec![seg(
            0.0,
            10.0,
            "med",
            vec![
                sticker(1.0),
                Event::Shake {
                    time: 1.1,
                    end: Some(1.4),
                    intensity: 1.0,
                    anchor: None,
                    sfx: None,
                    sfx_volume: None,
                },
            ],
        )];
        assert_eq!(
            evaluate(&stacked, &[], 10.0).interruption_count(),
            0,
            "不同类型挨着是有意组合，不是干扰"
        );
    }

    /// **音效不算"视觉变化"**：只配音效不改画面，不该被判成抽搐。
    #[test]
    fn sfx_is_not_a_visual_change() {
        let ev = Event::Sfx {
            time: 1.0,
            end: None,
            name: "laugh".into(),
            volume: None,
        };
        assert!(!is_visual(&ev), "音效不改画面");
        let segs = vec![seg(0.0, 10.0, "med", vec![ev.clone(), ev])];
        assert_eq!(evaluate(&segs, &[], 10.0).interruption_count(), 0);
    }

    /// **跨时长可比**的那个数：`interruptions_per_minute`。
    ///
    /// 这条钉住"别拿 `smoothness` 横比" —— 一条 10s 零干扰（=10）与一条 60s
    /// 零干扰（=60）的 `smoothness` 差 6 倍，但两者其实**都没有干扰**。
    #[test]
    fn per_minute_is_the_comparable_number() {
        let long_clean = Quality {
            duration: 60.0,
            visual_events: 0,
            interruptions: vec![],
            segments: 2,
        };
        let short_clean = Quality {
            duration: 10.0,
            visual_events: 0,
            interruptions: vec![],
            segments: 2,
        };
        assert_ne!(
            long_clean.smoothness(),
            short_clean.smoothness(),
            "smoothness 天然带时长，两者本就不同"
        );
        assert_eq!(
            long_clean.interruptions_per_minute(),
            short_clean.interruptions_per_minute(),
            "真正可比的是每分钟干扰数 —— 两者都是 0"
        );
        assert_eq!(long_clean.score(), 100.0);
        assert_eq!(short_clean.score(), 100.0, "都是零干扰 ⇒ 都是满分");
    }

    /// **展示分**：每分钟 6 次 = 0 分，线性，且夹在 [0,100]。
    ///
    /// 变异自证：把 `clamp` 去掉 ⇒ `over_budget_is_floored_at_zero` 翻红。
    #[test]
    fn score_is_linear_and_clamped() {
        let mk = |n: usize, dur: f64| Quality {
            duration: dur,
            visual_events: 0,
            interruptions: (0..n)
                .map(|i| Interruption { kind: InterruptionKind::MidSentence, at: i as f64 })
                .collect(),
            segments: 1,
        };
        // 60s 里 6 次 ⇒ 每分钟 6 次 ⇒ 0 分
        assert!((mk(6, 60.0).score() - 0.0).abs() < 1e-9);
        // 60s 里 3 次 ⇒ 每分钟 3 次 ⇒ 50 分
        assert!((mk(3, 60.0).score() - 50.0).abs() < 1e-9);
        // 超预算 ⇒ 夹到 0（不是负数）
        assert_eq!(mk(20, 60.0).score(), 0.0, "超预算要夹到 0，不许是负数");
    }

    /// **除零保护**：时长为 0 不许 panic、也不许给 NaN。
    #[test]
    fn zero_duration_is_safe() {
        let q = evaluate(&[], &[], 0.0);
        assert_eq!(q.interruptions_per_minute(), 0.0);
        assert_eq!(q.score(), 100.0, "零时长 + 零干扰 = 满分（不是 NaN）");
        assert!(q.score().is_finite());
    }
}
