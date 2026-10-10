//! 爆点判据（总方案 §6）：**"哪里是爆点"只有一个判据表，全仓共用**。
//!
//! # 为什么必须有这个模块
//!
//! §6 / §7 / §8 三件事**都建立在同一个问题上**："爆点在哪"。
//!
//! | 项 | 它要这个判断干什么 |
//! |---|---|
//! | §6 爆点判据多信号 | 判得**准**（今天只有弹幕密度一个特征） |
//! | §7 静默分两类 | 爆点**之后**的停顿要保护（那是笑点发酵的地方） |
//! | §8 `role` 叙事角色 | 按"离爆点多近"给段分角色 |
//!
//! 三处各写一遍的话，`§7` 保护的那段停顿与 `§8` 当爆点的那一段**可能不是同一个爆点** ——
//! 而两者单独看都"说得通"。所以这里做成**一个纯函数、一份判据、一处阈值**。
//!
//! # ⚠️ 判据必须能"无弹幕工作"（规格 §2 的 D-1/D-2 那条修正）
//!
//! 原方案的判据是**弹幕密度**。而**切片不一定有弹幕文件** ——
//! 没有时密度全 0 ⇒ **所有段都判成非爆点** ⇒ 依赖它的三件事**静默失效**。
//! 那正是本仓 R1 那条禁忌。
//!
//! ⇒ 判据链按**可得性**排（规格原文那张表）：
//!
//! | 信号 | 可得性 | 这里的角色 |
//! |---|---|---|
//! | 字幕本身（句子密度、语气） | **必然有**（ASR 产物） | **主判据**（缺了它整个函数没意义） |
//! | 弹幕密度 | **不一定有** | 有则加权，**无则跳过** |
//! | 音效密度 | 有音频就有 | 有则加权，无则跳过 |
//!
//! 「无则跳过」不是"当成 0 分"—— 是**不参与归一化**。
//! 当成 0 分的后果正是上面那条静默失效（有字幕的爆点被弹幕那一路拉到阈值以下）。
//!
//! # 这条判据**不许**静默降级（规格 §2 规则 2）
//!
//! 无弹幕时调用方**必须**在日志里明说"本次无弹幕信号，爆点按字幕判"。
//! 所以 [`PunchDetector::signals()`] 会把"实际用上了哪几路"报出来，
//! 由调用方决定怎么播报 —— **别让调用方自己猜**（它会猜错的）。

use crate::parse::srt::Subtitle;
use crate::project::Segment;

/// 爆点判据用的**窗口宽度**（秒）。
///
/// 取 5s：短于它，一句完整的话会被切散（判据变成"哪几个字密"）；
/// 长于它，"爆点"会糊成一片（§7 要保护的"停顿"就在爆点边缘，糊了就保不住）。
pub const PUNCH_WINDOW_SECS: f64 = 5.0;

/// 判定为爆点的**归一化分**门槛（0~1）。
///
/// ⚠️ 这个数是**保守拍的**，理由与 `quality.rs` 那条"不做静默突变"同源：
/// 规格没给数，而拍一个进去等于把一条**没标定过**的判据混进结果。
/// 所以取 **0.6**（明显高于平均，但不是"最高那一档"）——
/// **宁可少判**：多判一个爆点 = §7 多保护一段不该保护的停顿（片子变拖沓，
/// 用户一眼能看出）；少判一个 = 少保护一段（与今天的行为相同，不倒退）。
pub const PUNCH_THRESHOLD: f64 = 0.6;

/// 实际用上了哪几路信号（给调用方播报用 —— 见模块头"不许静默降级"）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Signals {
    /// 字幕信号（**恒为真**：没有字幕就没有判据）。
    pub subtitles: bool,
    /// 弹幕密度信号（`data/danmaku.json` 非空才有）。
    pub danmaku: bool,
    /// 音效信号（有音频且探到了响度峰才有）。
    pub sfx: bool,
}

impl Signals {
    /// 给日志用的一句话。
    ///
    /// ⚠️ **没有弹幕时必须说出来**（规格 §2 规则 2）。这一句就是那个"说"。
    pub fn describe(&self) -> String {
        let mut used = vec!["字幕"];
        if self.danmaku {
            used.push("弹幕");
        }
        if self.sfx {
            used.push("音效");
        }
        let used = used.join(" + ");
        if self.danmaku {
            format!("爆点判据：{used}")
        } else {
            // 这一支是**降级**，措辞要让它在日志里显眼（与"正常运行"区分开）
            format!("⚠ 本次无弹幕信号，爆点按{used}判")
        }
    }
}

/// 一路信号的原始贡献：`(时间, 强度 0~1)`。
///
/// 强度是**这一路自己**归一化过的（各路的量纲完全不同：字幕是"每秒几个字"、
/// 弹幕是"每秒几条"、音效是"响度相对峰值"）—— 归一化必须在**进这个函数之前**做完，
/// 否则"哪一路分量大"就由量纲决定了（那正是今天只有弹幕一个特征时的毛病）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SignalPoint {
    pub at: f64,
    pub strength: f64,
}

/// 一处爆点。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Punch {
    /// 窗口起点（秒）。
    pub at: f64,
    /// 窗口终点（秒）。
    pub end: f64,
    /// 归一化分（0~1）。**放进结果是为了可解释** —— 界面/日志能说"这里是 0.82 的爆点"，
    /// 而不只是"这里是个爆点"（后者没法回答"为什么不是旁边那个"）。
    pub score: f64,
}

/// 爆点检测器：**一个纯函数、一份判据、一处阈值**。
///
/// 全部输入都是已归一化的 `SignalPoint` + 段落表（段落提供 `fx` 这个**LLM 已标好**的先验）。
/// 不读文件、不碰网络 ⇒ 可单测、可对拍。
#[derive(Debug, Clone)]
pub struct PunchDetector {
    /// 每路信号的采样点（按路分组；空的那路 = 没有这个信号，**不参与归一化**）。
    pub subtitle_points: Vec<SignalPoint>,
    pub danmaku_points: Vec<SignalPoint>,
    pub sfx_points: Vec<SignalPoint>,
}

impl PunchDetector {
    /// 实际参与了判定的信号（见 [`Signals`]）。
    pub fn signals(&self) -> Signals {
        Signals {
            subtitles: !self.subtitle_points.is_empty(),
            danmaku: !self.danmaku_points.is_empty(),
            sfx: !self.sfx_points.is_empty(),
        }
    }

    /// 每一路在**总分里的权重**。
    ///
    /// ⚠️ **权重按"这一路在不在"重新分配，而不是固定的三份**：
    /// 没有弹幕时如果把弹幕那一路按 0 分算，字幕判出来的爆点会被**拉低到阈值以下**
    /// —— 那就是"静默失效"。这里改成**只对在场的路归一化**。
    ///
    /// 权重本身按规格的"主判据/次判据"排：字幕是主（0.6），弹幕与音效是次（各 0.2）。
    /// 只有字幕时它独占 1.0 —— 这正是"无弹幕也能工作"的含义。
    fn weights(&self, s: Signals) -> (f64, f64, f64) {
        // 主判据恒在（没有字幕就没有判据）
        const W_SUB: f64 = 0.6;
        const W_DM: f64 = 0.2;
        const W_SFX: f64 = 0.2;
        let (mut w_sub, mut w_dm, mut w_sfx) = (W_SUB, W_DM, W_SFX);
        if !s.danmaku {
            w_dm = 0.0;
        }
        if !s.sfx {
            w_sfx = 0.0;
        }
        let total = w_sub + w_dm + w_sfx;
        if total <= 0.0 {
            return (1.0, 0.0, 0.0);
        }
        // 按在场的那几路重新归一化（**不是**补一个 0 给缺席的那路）
        w_sub /= total;
        w_dm /= total;
        w_sfx /= total;
        (w_sub, w_dm, w_sfx)
    }

    /// 某一路在 `[at, at+win)` 内的强度（取该窗内最大值；窗外没有 = 0）。
    ///
    /// 取**最大值**而不是平均：爆点是**瞬时**的（一句话、一次爆笑、一声爆音），
    /// 平均会被窗口里的安静部分稀释掉 —— 5s 窗里有 1s 的高能，平均之后只剩 20%，
    /// 而"这 1s 就是爆点"。判据要的是"这里有没有发生过"。
    fn peak_in(points: &[SignalPoint], at: f64, win: f64) -> f64 {
        points
            .iter()
            .filter(|p| p.at >= at - 1e-9 && p.at < at + win - 1e-9)
            .map(|p| p.strength.clamp(0.0, 1.0))
            .fold(0.0, f64::max)
    }

    /// 段落给的先验：`fx == "high"` 的段，中心处给满分的**加成**。
    ///
    /// ⚠️ 它是**加成**不是**唯一判据**（今天 `quality.rs` 就是这么用的 ——
    /// `fx == "high"` 一票定生死）。理由是 `fx` 是 **LLM 标的**，而 LLM 会标错、
    /// 也会在**没有弹幕**时凭字幕猜。加成让"LLM 说这是爆点"能推它过线，
    /// 但不至于让"LLM 说不是"的信号全部作废。
    fn segment_prior(segments: &[Segment], at: f64, win: f64) -> f64 {
        let mid = at + win / 2.0;
        if segments
            .iter()
            .any(|s| s.fx == "high" && s.start <= mid && mid < s.end)
        {
            0.35
        } else {
            0.0
        }
    }

    /// **检测爆点**。返回按时间升序的窗口（不重叠：连续过线的窗口会合并）。
    ///
    /// 判据：`0.6×字幕 + 0.2×弹幕 + 0.2×音效 + fx 加成 ≥ `[`PUNCH_THRESHOLD`]，
    /// 其中缺席的信号**不参与归一化**（见 [`Self::weights`]）。
    pub fn detect(&self, segments: &[Segment], duration: f64, window: f64) -> Vec<Punch> {
        let s = self.signals();
        if !s.subtitles || duration <= 0.0 || window <= 0.0 {
            // 没有字幕信号 ⇒ **一个爆点都不判**（而不是"全都不是爆点"地跑一遍）。
            // 调用方拿到的空集与"真的没有爆点"无法区分，所以 `signals()` 要一起看。
            return Vec::new();
        }
        let (w_sub, w_dm, w_sfx) = self.weights(s);

        let steps = ((duration / window).ceil() as usize).max(1);
        let mut scored: Vec<Punch> = Vec::with_capacity(steps);
        for i in 0..steps {
            let at = i as f64 * window;
            let end = duration.min(at + window);
            let raw = w_sub * Self::peak_in(&self.subtitle_points, at, window)
                + w_dm * Self::peak_in(&self.danmaku_points, at, window)
                + w_sfx * Self::peak_in(&self.sfx_points, at, window)
                + Self::segment_prior(segments, at, window);
            scored.push(Punch { at, end, score: raw.min(1.0) });
        }

        // 过线的留下，**相邻的合并**（一个爆点不该因为窗口边界被切成两半 ——
        // §7 要保护"爆点之后"的停顿，切成两半会让保护范围只覆盖前半截）。
        let mut out: Vec<Punch> = Vec::new();
        for p in scored.into_iter().filter(|p| p.score >= PUNCH_THRESHOLD) {
            match out.last_mut() {
                Some(last) if (p.at - last.end).abs() < 1e-9 => {
                    last.end = p.end;
                    // 合并后取**较高的那一分**：那是这个爆点的"强度"。
                    last.score = last.score.max(p.score);
                }
                _ => out.push(p),
            }
        }
        out
    }
}

// ===== 从真实数据构造信号（§6 的输入适配层）=====
//
// ⚠️ **归一化在这里做完**，别留给 `detect` —— 判据那一层只认 0~1，
// 而这三路的量纲完全不同（字幕是"每秒几个字"、弹幕是"每秒几条"、响度是 dB）。
// 量纲混进判据之后，"哪一路分量大"就由单位决定了 —— 正是今天只有弹幕一个特征时的毛病。

/// 字幕信号：按**每窗字数**归一化（分母是窗口宽度，不是说话时长 —— 见函数内的 ⚠️）。
///
/// ⚠️ 这里用"字数/秒"而不是"条数/秒"：ASR 会把**一口气重复的音节**切成连续碎片
/// （本仓实测：`21.92→21.97→22.02` 那种，0.05s 一条），按条数算的话这种碎片段
/// 会拿到和"一句完整长句"一样的分 —— 而后者才是真的信息密度高。
///
/// ⚠️ `max_cps` 是**该切片自己**的基准（取所有窗口的最大值），不是全局常数：
/// 不同主播语速差一倍，用常数会让慢速主播**永远判不出爆点**。
pub fn subtitle_signal(subtitles: &[Subtitle], window: f64) -> Vec<SignalPoint> {
    if subtitles.is_empty() || window <= 0.0 {
        return Vec::new();
    }
    let mut buckets: std::collections::BTreeMap<usize, f64> = Default::default();
    for sub in subtitles {
        // 字数按 `chars()` 数（CJK 一个字算一个）；空白先去掉，否则英文句子的空格会灌水。
        let chars = sub.text.chars().filter(|c| !c.is_whitespace()).count() as f64;
        let idx = (sub.start / window).floor() as usize;
        *buckets.entry(idx).or_insert(0.0) += chars;
    }
    // ⚠️⚠️ **分母是窗口宽度，不是"这一段说话的总时长"。**
    //
    // 用说话时长当分母是个**看起来更精确、实际更糟**的选择：碎片字幕的时长极短
    // （本仓实测 0.05s 一条），`字数 / 0.2s` 能到 20 字/秒，而一句 4 秒的正常话只有 2.5 字/秒
    // ⇒ **判据被碎片彻底劫持**，"信息密度高"变成"字幕切得碎"。
    // 用窗口宽度当分母之后，量纲是"这一窗里说了多少字" —— 那才是"这段信息量大不大"。
    // ⚠️ 这个错法是我自己的用例（`subtitle_signal_weights_characters_not_fragments`）抓出来的。
    let raw: Vec<(f64, f64)> = buckets
        .into_iter()
        .map(|(i, chars)| (i as f64 * window, chars / window))
        .collect();
    normalize_points(&raw)
}

/// 弹幕信号：按**每秒条数**归一化（该切片自己作基准）。
///
/// ⚠️ 只数条数会把**刷屏**当成高潮（规格 D-2）。这一步**不做**多特征
/// （平均消息长度 / 相似度）—— 规格明说那是"可选增强，不是前提"。
/// 但归一化至少让它与字幕信号**同量纲**，不再是一路压倒性的特征。
pub fn danmaku_signal(counts: &[(f64, f64)], window: f64) -> Vec<SignalPoint> {
    if counts.is_empty() || window <= 0.0 {
        return Vec::new();
    }
    let mut buckets: std::collections::BTreeMap<usize, f64> = Default::default();
    for (at, count) in counts {
        let idx = (at / window).floor() as usize;
        *buckets.entry(idx).or_insert(0.0) += count;
    }
    let raw: Vec<(f64, f64)> = buckets
        .into_iter()
        .map(|(i, c)| (i as f64 * window, c / window))
        .collect();
    normalize_points(&raw)
}

/// 响度信号：把 dB 曲线归一化成 0~1。
///
/// ⚠️ **用百分位而不是 min/max**：一次爆音（掌声/尖叫/爆音效）会让 max 高出一大截，
/// 于是"其余所有时刻"都被压到接近 0 —— 判据等于只看那一个尖峰。
/// 取 5%~95% 分位作基准，把极值**夹进**区间（不是丢掉），保留了"它确实很高"。
///
/// ⚠️ 也**不做**"相对静音底噪"的判定：那是 §7 的事（它要分"拖沓沉默/喜剧停顿"），
/// 这里只回答"这一刻响不响"。
pub fn loudness_signal(curve: &[(f64, f64)]) -> Vec<SignalPoint> {
    if curve.is_empty() {
        return Vec::new();
    }
    let mut dbs: Vec<f64> = curve.iter().map(|(_, db)| *db).collect();
    dbs.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let pick = |q: f64| -> f64 {
        let i = ((dbs.len() - 1) as f64 * q).round() as usize;
        dbs[i.min(dbs.len() - 1)]
    };
    let lo = pick(0.05);
    let hi = pick(0.95);
    let span = (hi - lo).abs();
    curve
        .iter()
        .map(|(at, db)| {
            let strength = if span < 1e-9 {
                // 整段一样响（或只有一个采样点）⇒ 没有"响度信息"，给一个中性值。
                // ⚠️ 给 0 会让这一路静默失效（"整段都响"被判成"整段都不响"）；
                //    给 1 会把它变成万能爆点。0.5 = 不参与推动，但也不是缺席。
                0.5
            } else {
                ((db - lo) / span).clamp(0.0, 1.0)
            };
            SignalPoint { at: *at, strength }
        })
        .collect()
}

/// 把 `(时间, 原始值)` 归一化到 0~1（按**最大值**作基准；全 0 时全部回 0）。
fn normalize_points(raw: &[(f64, f64)]) -> Vec<SignalPoint> {
    let max = raw.iter().map(|(_, v)| *v).fold(0.0, f64::max);
    raw.iter()
        .map(|(at, v)| SignalPoint {
            at: *at,
            strength: if max > 0.0 { (v / max).clamp(0.0, 1.0) } else { 0.0 },
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seg(start: f64, end: f64, fx: &str) -> Segment {
        Segment {
            role: String::new(),
            start,
            end,
            fx: fx.into(),
            note: String::new(),
            events: Vec::new(),
            extra: Default::default(),
        }
    }

    fn pts(v: &[(f64, f64)]) -> Vec<SignalPoint> {
        v.iter()
            .map(|(at, strength)| SignalPoint { at: *at, strength: *strength })
            .collect()
    }

    /// ① **只有字幕也能判出爆点**（这是"无弹幕不静默失效"的核心判据）。
    #[test]
    fn subtitles_alone_can_produce_a_punch() {
        let d = PunchDetector {
            subtitle_points: pts(&[(10.0, 0.9)]),
            danmaku_points: Vec::new(),
            sfx_points: Vec::new(),
        };
        let s = d.signals();
        assert!(s.subtitles && !s.danmaku && !s.sfx);
        assert!(
            s.describe().contains("无弹幕信号"),
            "无弹幕时那句话必须说出来（不许静默降级）: {}",
            s.describe()
        );
        let punches = d.detect(&[], 60.0, PUNCH_WINDOW_SECS);
        assert_eq!(punches.len(), 1, "字幕 0.9 应当过线: {punches:?}");
        assert!((punches[0].at - 10.0).abs() < 1e-9);
    }

    /// ② ⚠️ **同样强度的字幕信号，在"有弹幕"与"无弹幕"两种情况下都应当过线。**
    ///
    /// 这条抓的是那个最隐蔽的错法：把缺席的弹幕**当成 0 分**参与固定权重，
    /// 于是 `0.6 × 0.9 = 0.54 < 0.6` 判不出爆点 —— 而"有弹幕时"能判出。
    /// 表现就是"这个切片没有弹幕文件 ⇒ 整个叙事结构静默失效"。
    #[test]
    fn missing_danmaku_does_not_dilute_the_subtitle_signal() {
        // ⚠️ 强度取 **0.8** 而不是 0.9，这是**判据能翻面的关键**：
        //   固定权重（缺席当 0 分）下总分 = 0.6 × 0.8 = 0.48 < 0.6 ⇒ **判不出**；
        //   只对在场的路归一化后 = 1.0 × 0.8 = 0.80 ≥ 0.6 ⇒ **判得出**。
        //   取 0.9 时两种实现都过线（0.54 / 0.9 在阈值两侧的同一侧），判据抓不到变异 —— 实测踩过。
        let sub = pts(&[(10.0, 0.8)]);
        let no_dm = PunchDetector {
            subtitle_points: sub.clone(),
            danmaku_points: Vec::new(),
            sfx_points: Vec::new(),
        };
        // 对照组：有弹幕，但**这一窗弹幕也是空的**（强度 0）。
        // 固定权重实现会认为"有弹幕"并把 0.2 分给一个空信号，从而拉低总分。
        let empty_dm = PunchDetector {
            subtitle_points: sub.clone(),
            danmaku_points: pts(&[(200.0, 0.9)]),
            sfx_points: Vec::new(),
        };
        let a = no_dm.detect(&[], 60.0, PUNCH_WINDOW_SECS);
        let b = empty_dm.detect(&[], 60.0, PUNCH_WINDOW_SECS);
        assert_eq!(a.len(), 1, "无弹幕时字幕 0.9 必须自己就能过线: {a:?}");
        assert_eq!(
            b.len(),
            1,
            "有弹幕但这一窗没有弹幕时，字幕信号同样不该被稀释: {b:?}"
        );
        assert_eq!(a[0].at, b[0].at, "两种情况下判出的应当是同一个爆点");
    }

    /// ③ 弹幕**能加强**判据（有它的时候阈值更容易被推过）。
    #[test]
    fn danmaku_reinforces_a_borderline_punch() {
        // 字幕 0.5 = 只有它时过不了线（0.5 < 0.6）
        let base = PunchDetector {
            subtitle_points: pts(&[(10.0, 0.5)]),
            danmaku_points: Vec::new(),
            sfx_points: Vec::new(),
        };
        assert!(
            base.detect(&[], 60.0, PUNCH_WINDOW_SECS).is_empty(),
            "0.5 的字幕信号不该单独过线（阈值 0.6）"
        );
        // 加上满档弹幕 ⇒ 权重重新分配后应当过线
        let with_dm = PunchDetector {
            subtitle_points: pts(&[(10.0, 0.5)]),
            danmaku_points: pts(&[(10.0, 1.0)]),
            sfx_points: Vec::new(),
        };
        let p = with_dm.detect(&[], 60.0, PUNCH_WINDOW_SECS);
        assert_eq!(p.len(), 1, "弹幕满档应当把 0.5 的字幕推过线: {p:?}");
        assert!(p[0].score > PUNCH_THRESHOLD);
    }

    /// ④ **`fx == "high"` 是加成、不是唯一判据**（今天它一票定生死）。
    #[test]
    fn segment_fx_high_is_a_bonus_not_the_only_judge() {
        // 没有任何信号点，但 LLM 标了这个段是 high ⇒ 仅靠加成过不了线（0.35 < 0.6）
        let d = PunchDetector {
            subtitle_points: pts(&[(0.0, 0.01)]), // 有一路字幕信号，但强度极低
            danmaku_points: Vec::new(),
            sfx_points: Vec::new(),
        };
        let segs = vec![seg(8.0, 20.0, "high")];
        let p = d.detect(&segs, 60.0, PUNCH_WINDOW_SECS);
        assert!(
            p.is_empty(),
            "光有 `fx=high` 不该判出爆点（它是加成不是一票通过）: {p:?}"
        );
        // 字幕稍微强一点 + 加成 ⇒ 过线
        let d2 = PunchDetector {
            subtitle_points: pts(&[(10.0, 0.3)]),
            danmaku_points: Vec::new(),
            sfx_points: Vec::new(),
        };
        let p2 = d2.detect(&segs, 60.0, PUNCH_WINDOW_SECS);
        assert_eq!(p2.len(), 1, "0.3 + 0.35 加成应当过线: {p2:?}");
    }

    /// ⑤ **相邻窗口要合并**（一个爆点不许被窗口边界切成两半）。
    #[test]
    fn adjacent_windows_merge_into_one_punch() {
        let d = PunchDetector {
            // 覆盖 [10, 20) 两整窗
            subtitle_points: pts(&[(10.5, 0.9), (15.5, 0.9)]),
            danmaku_points: Vec::new(),
            sfx_points: Vec::new(),
        };
        let p = d.detect(&[], 60.0, PUNCH_WINDOW_SECS);
        assert_eq!(p.len(), 1, "相邻两窗该合成一个爆点: {p:?}");
        assert!((p[0].at - 10.0).abs() < 1e-9);
        assert!((p[0].end - 20.0).abs() < 1e-9, "合并后要盖住两窗");
    }

    /// ⑥ **没有字幕信号 ⇒ 一个爆点都不判**（而不是"全都不是爆点"地跑一遍）。
    #[test]
    fn no_subtitle_signal_yields_nothing() {
        let d = PunchDetector {
            subtitle_points: Vec::new(),
            danmaku_points: pts(&[(10.0, 1.0)]),
            sfx_points: Vec::new(),
        };
        assert!(
            d.detect(&[], 60.0, PUNCH_WINDOW_SECS).is_empty(),
            "判据的主信号是字幕；它不在就不该给结论"
        );
    }

    // ── 输入适配层（从真实数据构造信号）──────────────────────────

    fn sub(start: f64, end: f64, text: &str) -> Subtitle {
        Subtitle {
            index: 0,
            start,
            end,
            text: text.into(),
            html: String::new(),
        }
    }

    /// 字幕信号按**每秒字数**算 —— 碎片密但字数少的一段，不该拿到与长句同档的分。
    ///
    /// ⚠️ 这是真实数据里最常见的形态（本仓实测 0.05s 的碎片占 2.6%，
    /// ASR 把一口气重复的音节切成连续碎片）。按**条数**算的话这种碎片段会拿满分。
    #[test]
    fn subtitle_signal_weights_characters_not_fragments() {
        // 窗 0：一条 4s 的长句（10 个字 → 2.5 字/秒）
        // 窗 1：四条 0.05s 的碎片，共 4 个字 —— 条数多，但总字数少
        let subs = vec![
            sub(0.0, 4.0, "这是一句完整的话啊"),
            sub(5.0, 5.05, "啊"),
            sub(5.1, 5.15, "啊"),
            sub(5.2, 5.25, "啊"),
            sub(5.3, 5.35, "啊"),
        ];
        let pts = subtitle_signal(&subs, PUNCH_WINDOW_SECS);
        assert_eq!(pts.len(), 2, "两个窗各一个点: {pts:?}");
        let w0 = pts.iter().find(|p| p.at == 0.0).unwrap();
        let w1 = pts.iter().find(|p| p.at == 5.0).unwrap();
        assert_eq!(w0.strength, 1.0, "长句那个窗是基准（最强）");
        assert!(
            w1.strength < 0.5,
            "四条碎片（共 4 字/0.2s 跨度）不该与长句同档: {w1:?}"
        );
    }

    /// 响度信号用 **5%~95% 分位**归一化 —— 一个孤立尖峰不该把其余时刻全压到 0。
    #[test]
    fn loudness_signal_is_not_hijacked_by_a_single_spike() {
        // 20 个正常点（-30 ~ -10 dB）+ 1 个爆音（0 dB）
        let mut curve: Vec<(f64, f64)> = (0..20)
            .map(|i| (i as f64 * 0.5, -30.0 + i as f64))
            .collect();
        curve.push((10.0, 0.0)); // 爆音
        let pts = loudness_signal(&curve);
        assert_eq!(pts.len(), curve.len());
        // 中间的普通点若被尖峰压扁，strength 会接近 0；分位归一化应当让它们保持可分辨
        let mid = pts.iter().find(|p| (p.at - 4.5).abs() < 1e-9).unwrap();
        assert!(
            mid.strength > 0.1,
            "普通点被孤立尖峰压扁了（min/max 归一化的症状）: {mid:?}"
        );
        // 尖峰仍应是最高的（夹进区间，不是丢掉）
        let spike = pts.iter().find(|p| (p.at - 10.0).abs() < 1e-9).unwrap();
        assert_eq!(spike.strength, 1.0, "爆音必须仍是最强");
    }

    /// 弹幕信号按每秒条数归一化；**空输入 ⇒ 空信号**（= "本次无弹幕信号"）。
    #[test]
    fn danmaku_signal_is_empty_when_there_is_no_danmaku() {
        assert!(danmaku_signal(&[], PUNCH_WINDOW_SECS).is_empty());
        // 同一窗内多条要累加
        let pts = danmaku_signal(&[(0.0, 3.0), (1.0, 2.0), (6.0, 1.0)], PUNCH_WINDOW_SECS);
        assert_eq!(pts.len(), 2);
        let w0 = pts.iter().find(|p| p.at == 0.0).unwrap();
        let w1 = pts.iter().find(|p| p.at == 5.0).unwrap();
        assert_eq!(w0.strength, 1.0, "5 条那个窗是基准");
        assert!((w1.strength - 0.2).abs() < 1e-9, "1 条 = 5 条的 1/5: {w1:?}");
    }

    /// 整段一样响时**不给 0 也不给 1**：给 0 是静默失效，给 1 是万能爆点。
    #[test]
    fn flat_loudness_curve_is_neutral() {
        let pts = loudness_signal(&[(0.0, -20.0), (1.0, -20.0), (2.0, -20.0)]);
        assert_eq!(pts.len(), 3);
        for p in &pts {
            assert_eq!(p.strength, 0.5, "整段一样响 ⇒ 中性，不是 0 也不是 1");
        }
    }

    /// **端到端**：用真实形态的数据（字幕 + 弹幕 + 响度）跑一遍三个适配器 + 检测。
    #[test]
    fn adapters_feed_the_detector_end_to_end() {
        let subs = vec![
            sub(0.0, 4.0, "普通的一句话就这样"),
            sub(20.0, 24.0, "这里突然说了一大堆话哈哈哈哈哈"),
        ];
        // 弹幕只在 20s 附近爆
        let dm = danmaku_signal(&[(20.5, 30.0), (21.0, 25.0), (0.5, 1.0)], PUNCH_WINDOW_SECS);
        // 响度也在 20s 附近高
        let loud = loudness_signal(
            &(0..60)
                .map(|i| {
                    let t = i as f64;
                    let db = if (20.0..25.0).contains(&t) { -5.0 } else { -30.0 };
                    (t, db)
                })
                .collect::<Vec<_>>(),
        );
        let d = PunchDetector {
            subtitle_points: subtitle_signal(&subs, PUNCH_WINDOW_SECS),
            danmaku_points: dm,
            sfx_points: loud,
        };
        let s = d.signals();
        assert!(s.subtitles && s.danmaku && s.sfx, "三路都该有信号: {s:?}");
        assert!(!s.describe().contains("无弹幕"), "有弹幕时别报降级: {}", s.describe());
        let p = d.detect(&[], 60.0, PUNCH_WINDOW_SECS);
        assert!(
            p.iter().any(|x| (x.at - 20.0).abs() < 1e-9),
            "20s 那一窗三路齐高，必须判成爆点: {p:?}"
        );
    }

    /// ⑦ 强度要**夹到 0~1**（外面传进来的原始值可能是任意尺度）。
    #[test]
    fn strengths_are_clamped() {
        let d = PunchDetector {
            subtitle_points: pts(&[(10.0, 99.0)]),
            danmaku_points: Vec::new(),
            sfx_points: Vec::new(),
        };
        let p = d.detect(&[], 60.0, PUNCH_WINDOW_SECS);
        assert_eq!(p.len(), 1);
        assert!(p[0].score <= 1.0, "分不该超过 1.0: {}", p[0].score);
    }
}
