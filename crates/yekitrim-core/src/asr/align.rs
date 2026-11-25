//! 断句器：**SenseVoice 提供时间轴，最终文本提供切点**。
//!
//! ## 为什么是这套分工（实测结论，2026-09）
//!
//! `diagnose_sensevoice_tokens` 在真实音频上 dump 了原始 token，两个事实推翻了旧口径：
//!
//! 1. **SenseVoice 完全不吐标点** —— 一块 60s 的全文是
//!    `…你们不用上班了吗你们不用赚钱了吗你们不用上学了吗`，没有一个 `。！？`。
//!    所以旧 `build_segments_from_timestamps` 里"句末标点强制切"那条规则
//!    **在真实数据上永远没有信号可用**（不是判断写得不对，是压根没有标点可判）。
//! 2. **token 是单字的**（89 个 token，0 个多字符），时间戳逐字可用。
//!    真正落在句边界上的信号是**气口**：问句之间是 `+0.42s` 的停顿，
//!    而普通字间只有 `0.12~0.24s` —— 旧的 `segment_gap`(1.0s) 把这层信号整个跳过了。
//!
//! 标点在 qwen3 那边（它有标点、没有时间戳），气口在 SenseVoice 这边（有时间戳、没标点）。
//! 于是本模块的职责就是**把两者合起来**：
//!
//! | 作用 | 信号 | 来源 |
//! |------|------|------|
//! | 决定**切成几条**（切分信号） | 句末/句中标点、强停顿 | 最终文本 + 时间轴 |
//! | 决定**切在哪一刻**（吸附信号） | 气口（`gap ≥ breath_gap`） | 时间轴（唯一有时间戳的一侧） |
//! | 兜底**不许超标**（硬上限） | 字数 | 最终文本 |
//!
//! 注意"气口"**只在标点已经决定要切时才用来定位切点**，它自己不作为独立切分信号 ——
//! 实测 `刷赌人车` 这种词中间也有 0.30s 的停顿，单靠气口会切碎。

use serde::Serialize;

use crate::config::AsrConfig;

/// 句末标点：必切
pub const HARD_ENDERS: [char; 6] = ['。', '！', '？', '.', '!', '?'];
/// 句中停顿标点：距上一条超过 `soft_cut_secs` 时才切
pub const SOFT_ENDERS: [char; 7] = ['，', '、', '；', '：', ',', ';', ':'];

/// 单条字幕尾巴的余韵上限（与旧 `cut_time` 同款语义）：
/// 停顿多长由两条字幕**之间**的间隙承担，不灌进前一条的尾巴 ——
/// 否则长停顿会让字幕停在纯静音上（实测 20s 停顿 → 10s 死气）。
const TRAIL_PAD: f64 = 1.0;

/// 两个切点至少要隔这么多**秒**，否则合并（防止切出碎片）。
///
/// ⚠️ 只对**来自时间轴**的切点（强停顿/字数上限）生效：标点切点是文本层的事实，
/// 任何时候都要留 —— 早期一律按时间判，把对齐差窗口里的三条问句并成了两条。
/// 见 `split_text_on_timeline` 里的判据。
const MIN_PIECE_SECS: f64 = 0.5;

/// 单条字幕的最短**显示**时长：低于它的条会被并进相邻条（见 `merge_too_short`）。
const MIN_VISIBLE_SECS: f64 = 0.3;

/// 相邻两个时刻之间的最小间隔（保证严格递增、不算出负时长；不是"最小时长"）。
const MIN_STEP_SECS: f64 = 0.05;

/// 有序夹取：`hi < lo` 时以 `lo` 为准。
///
/// ⚠️ **不要直接用 `f64::clamp`**：它在 `min > max` 时**panic**（`min > max, or either was NaN`），
/// 而这里的窗口边界全是算出来的 —— 空/退化窗口、浮点误差都能构造出这种入参。
/// 实测在 3 小时真实素材上崩过：某个边界的落点正好压到窗口末端（`start + 0.05 > span.1`），
/// 整个转写进程直接死在 `f64.rs` 里。
fn clamp_ordered(v: f64, lo: f64, hi: f64) -> f64 {
    v.min(hi.max(lo)).max(lo)
}

/// 重新同步时允许的错位范围（字）。
const RESYNC_LOOKAHEAD: usize = 4;

/// 字符级时间轴：SenseVoice 的逐字时间戳。`chars[i]` 发生在 `times[i]`。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CharTimeline {
    pub chars: Vec<char>,
    pub times: Vec<f64>,
}

impl CharTimeline {
    /// 从 sensevoice token 构建。token 是单字的（实测），但这里仍按"摊成单字、共享该
    /// token 的时间戳"处理 —— 换模型（多字符 token）时语义不用重定。
    pub fn from_tokens(tokens: &[String], times: &[f64]) -> Self {
        let mut chars = Vec::new();
        let mut out_times = Vec::new();
        for (tok, &t) in tokens.iter().zip(times.iter()) {
            for ch in tok.chars() {
                // 替换符是识别噪声，占时间轴没有意义（旧口径同样丢弃）
                if ch == '\u{FFFD}' {
                    continue;
                }
                chars.push(ch);
                out_times.push(t);
            }
        }
        Self { chars, times: out_times }
    }

    pub fn len(&self) -> usize {
        self.chars.len()
    }

    pub fn is_empty(&self) -> bool {
        self.chars.is_empty()
    }

    pub fn char_time(&self, i: usize) -> f64 {
        self.times.get(i).copied().unwrap_or_else(|| self.times.last().copied().unwrap_or(0.0))
    }

    /// 第 `i` 个字与上一个字之间的间隔（`i == 0` → 0）。
    /// token 内共享时间戳 ⇒ 间隔 0；**只有 token 之间才是真实的语音间隔**。
    pub fn gap_at(&self, i: usize) -> f64 {
        if i == 0 {
            0.0
        } else {
            (self.times[i] - self.times[i - 1]).max(0.0)
        }
    }

    /// 气口候选：停顿 ≥ `min_gap` 的位置。返回的是**停顿之后**那个字（index/time）。
    pub fn breath_points(&self, min_gap: f64) -> Vec<BreathPoint> {
        (1..self.len())
            .filter_map(|i| {
                let gap = self.gap_at(i);
                (gap >= min_gap).then_some(BreathPoint { index: i, time: self.times[i], gap })
            })
            .collect()
    }

    /// 跨 chunk 合并后强制时间单调：重叠区两块都报了同一段字，时间可能倒退（ROADMAP §233）。
    pub fn monotonic_fix(&mut self) {
        for i in 1..self.times.len() {
            if self.times[i] < self.times[i - 1] {
                self.times[i] = self.times[i - 1];
            }
        }
    }

    /// 取 `[start, end)` 之间的字（hybrid 用它把整片时间轴收窄到一个语音窗口，
    /// 好让 qwen3 的文本只跟这个窗口对齐）。
    pub fn slice_by_time(&self, start: f64, end: f64) -> Self {
        let mut out = Self::default();
        for (i, &t) in self.times.iter().enumerate() {
            if t >= start && t < end {
                out.chars.push(self.chars[i]);
                out.times.push(t);
            }
        }
        out
    }

    pub fn text(&self) -> String {
        self.chars.iter().collect()
    }
}

/// 一个气口：停顿长度为 `gap`，停顿之后那个字在 `index`、时刻 `time`。
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct BreathPoint {
    pub index: usize,
    pub time: f64,
    pub gap: f64,
}

/// 文本字 → 时间轴字 的对齐表。
#[derive(Debug, Clone, Default)]
pub struct Alignment {
    /// 文本第 i 个字 → 时间轴字下标（`None` = 没对上，时间靠插值）
    pub map: Vec<Option<usize>>,
    /// 命中的 (时间轴下标, 文本下标)，按时间轴下标升序
    pub pairs: Vec<(usize, usize)>,
    /// 重新同步次数（错字/漏字把锚点打断了几次）
    pub resyncs: usize,
}

impl Alignment {
    /// 文本第 `text_idx` 个字的时间。
    ///
    /// 有锚点用锚点；没有就在**前后锚点之间按字数线性插值**；两端缺锚点时夹到
    /// 窗口边界上（宁可误差大一点，也不要产出一个跑到窗口外面的时间）。
    pub fn time_of(&self, text_idx: usize, timeline: &CharTimeline, span: (f64, f64)) -> f64 {
        if let Some(Some(j)) = self.map.get(text_idx) {
            return timeline.char_time(*j);
        }
        let prev = (0..text_idx.min(self.map.len()))
            .rev()
            .find_map(|k| self.map[k].map(|j| (k, j)));
        let next = (text_idx + 1..self.map.len()).find_map(|k| self.map[k].map(|j| (k, j)));

        let t = match (prev, next) {
            (Some((pk, pj)), Some((nk, nj))) => {
                let t0 = timeline.char_time(pj);
                let t1 = timeline.char_time(nj);
                let frac = (text_idx.saturating_sub(pk)) as f64 / (nk - pk).max(1) as f64;
                t0 + (t1 - t0) * frac
            }
            (Some((_, pj)), None) => timeline.char_time(pj),
            (None, Some((_, nj))) => timeline.char_time(nj),
            (None, None) => span.0,
        };
        t.clamp(span.0, span.1.max(span.0))
    }

    /// 离时间轴下标 `tl_idx` 最近的**文本**下标（把气口/强停顿映射回文本位置用）。
    pub fn text_index_near(&self, tl_idx: usize) -> Option<usize> {
        if self.pairs.is_empty() {
            return None;
        }
        let at = self.pairs.partition_point(|(tj, _)| *tj < tl_idx);
        let mut best: Option<(usize, usize)> = None; // (距离, 文本下标)
        for c in [at.checked_sub(1), Some(at)].into_iter().flatten() {
            if let Some(&(tj, ti)) = self.pairs.get(c) {
                let d = tj.abs_diff(tl_idx);
                if best.is_none_or(|(bd, _)| d < bd) {
                    best = Some((d, ti));
                }
            }
        }
        best.map(|(_, ti)| ti)
    }
}

/// 把 `text` 对齐到 `timeline`。
///
/// 贪心 + 小范围重新同步：字符相等就前进；不等则在两侧各 `RESYNC_LOOKAHEAD` 个字内找
/// 代价最小的重合点拉回来。这样能同时吃下**错字**（1:1 不等，下一个字就对齐）、
/// **漏字**（时间轴多字）与**多字**（文本多字，不占时间）。不用 LCS 是因为中文这两份
/// 文本高度同构，贪心已经够稳，而 LCS 在长文本上反而碎片化（ROADMAP §258）。
pub fn align_to_timeline(text: &[char], timeline: &CharTimeline) -> Alignment {
    let n = text.len();
    let m = timeline.len();
    let mut map = vec![None; n];
    let mut pairs: Vec<(usize, usize)> = Vec::new();
    let mut resyncs = 0usize;
    let (mut i, mut j) = (0usize, 0usize);

    while i < n && j < m {
        if text[i] == timeline.chars[j] {
            map[i] = Some(j);
            pairs.push((j, i));
            i += 1;
            j += 1;
            continue;
        }
        let mut best: Option<(usize, usize)> = None;
        'search: for cost in 2..=(RESYNC_LOOKAHEAD * 2) {
            for k in 0..=RESYNC_LOOKAHEAD.min(cost) {
                let m2 = cost - k;
                if m2 > RESYNC_LOOKAHEAD {
                    continue;
                }
                if i + k < n && j + m2 < m && text[i + k] == timeline.chars[j + m2] {
                    best = Some((k, m2));
                    break 'search;
                }
            }
        }
        match best {
            Some((k, m2)) => {
                i += k;
                j += m2;
                resyncs += 1;
            }
            // 找不到重合同点：当作文本多出来的字，只推进文本一侧
            None => i += 1,
        }
    }

    Alignment { map, pairs, resyncs }
}

/// 断句参数（由 `[asr]` 配置投影而来）。
#[derive(Debug, Clone, Copy)]
pub struct SplitPolicy {
    /// 气口阈值：相邻字停顿 ≥ 此值算一个气口（**吸附用**，不是独立切分信号）
    pub breath_gap: f64,
    /// 强停顿：≥ 此值**独立**切一条（与标点无关），默认沿用 `segment_gap`
    pub strong_gap: f64,
    /// 句中标点软切：距上一条边界超过这个时长才切
    pub soft_cut_secs: f64,
    /// 字数硬上限（只在没有标点/气口可用时兜底）
    pub max_chars: usize,
    /// 标点边界吸附到气口的半径（秒）
    pub snap_window: f64,
}

impl Default for SplitPolicy {
    fn default() -> Self {
        Self {
            breath_gap: 0.25,
            strong_gap: 1.0,
            soft_cut_secs: 6.0,
            max_chars: 30,
            snap_window: 0.3,
        }
    }
}

impl SplitPolicy {
    pub fn from_config(cfg: &AsrConfig) -> Self {
        Self {
            breath_gap: cfg.breath_gap,
            strong_gap: cfg.segment_gap,
            soft_cut_secs: cfg.soft_cut_secs,
            max_chars: cfg.max_segment_chars.max(1),
            snap_window: cfg.snap_window,
        }
    }
}

/// 一条边界是**由什么决定的** —— 落进中间产物，方便事后判断断句为什么长这样。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BoundarySource {
    /// 标点边界，切点吸附到了真实气口
    PunctuationOnBreath,
    /// 标点边界，附近没有气口，用对齐插值的时间
    Punctuation,
    /// 强停顿（gap ≥ strong_gap）
    StrongGap,
    /// 字数硬上限兜底
    CharCap,
}

#[derive(Debug, Clone, Serialize)]
pub struct SplitPiece {
    pub text: String,
    pub start: f64,
    pub end: f64,
    /// 本条的**结束**边界由什么决定（最后一条为 `None` = 到窗口尾）
    pub boundary: Option<BoundarySource>,
}

/// 把最终文本按「标点 + 强停顿」切成若干条，切点时间取自时间轴（吸附到气口）。
///
/// `span` = 这段文本对应的音频区间（hybrid 下是语音窗口），用于给插值兜底夹边界。
pub fn split_text_on_timeline(
    text: &str,
    timeline: &CharTimeline,
    policy: &SplitPolicy,
    span: (f64, f64),
) -> Vec<SplitPiece> {
    let chars: Vec<char> = text.trim().chars().collect();
    if chars.is_empty() || timeline.is_empty() {
        return Vec::new();
    }
    let align = align_to_timeline(&chars, timeline);

    // ---- ① 文本侧切点（值 = 下一段的第一个字下标）----
    let mut cuts: Vec<(usize, BoundarySource)> = Vec::new();
    let mut piece_start = 0usize;
    for (i, ch) in chars.iter().enumerate() {
        let after = i + 1;
        if after >= chars.len() {
            break;
        }
        if HARD_ENDERS.contains(ch) {
            cuts.push((after, BoundarySource::Punctuation));
            piece_start = after;
        } else if SOFT_ENDERS.contains(ch) {
            let elapsed = align.time_of(i, timeline, span) - align.time_of(piece_start, timeline, span);
            if elapsed >= policy.soft_cut_secs {
                cuts.push((after, BoundarySource::Punctuation));
                piece_start = after;
            }
        }
    }

    // ---- ② 强停顿独立切（没有标点也切：这层信号以前靠 `segment_gap`，保留）----
    for bp in timeline.breath_points(policy.strong_gap) {
        if let Some(ti) = align.text_index_near(bp.index) {
            if ti > 0 && ti < chars.len() {
                cuts.push((ti, BoundarySource::StrongGap));
            }
        }
    }

    cuts.sort_by_key(|(i, _)| *i);
    cuts.dedup_by_key(|(i, _)| *i);
    // 去掉贴太近的切点（同一处被标点+强停顿各报一次）。
    //
    // 判据分两类，**不能一刀切**：
    //   · **标点切点是文本层的事实**（"这里该断"），就算对齐算出来的时间几乎相同也必须留 ——
    //     曾经按时间一刀切，结果对齐差的窗口里三条问句被并成两条（用户报的正是这个）。
    //   · 强停顿/字数上限来自时间轴，用**时间距**判（< MIN_PIECE_SECS 说明切出来也是个碎片）：
    //     实测这条口径在 3 小时素材上比按字距判多留约 150 个真实停顿边界
    //     （字距判会把「≥1s 停顿时夹着一个孤字」的两处边界误并）。
    let mut kept: Vec<(usize, BoundarySource)> = Vec::new();
    for (idx, src) in cuts {
        if let Some(&(prev, prev_src)) = kept.last() {
            let punct = |s: BoundarySource| {
                matches!(s, BoundarySource::Punctuation | BoundarySource::PunctuationOnBreath)
            };
            if !punct(src) && !punct(prev_src) {
                let dt = align.time_of(idx, timeline, span) - align.time_of(prev, timeline, span);
                if dt < MIN_PIECE_SECS {
                    continue;
                }
            }
        }
        kept.push((idx, src));
    }

    // ---- ③ 字数硬上限兜底 ----
    let mut pieces = build_ranges(&kept, chars.len());
    // 每轮都把「第一个超限的段」一分为二，段数 +1、每段至少 1 字 ⇒ 最多跑 `chars.len()` 轮。
    // 用一个**可证明的**上界而不是随手写的常数：写死 64 的话，长窗口（qwen3 有时会
    // 在一个窗口里吐出远超字数的内容）会在还有超长段时就提前退出。
    let mut guard = chars.len() + 1;
    loop {
        guard -= 1;
        if guard == 0 {
            break;
        }
        let Some(pos) = pieces
            .iter()
            .position(|(a, b)| b - a > policy.max_chars)
        else {
            break;
        };
        let (a, _) = pieces[pos];
        let cap = a + policy.max_chars;
        // 优先在软标点后切（读起来像一句话就断）；没有标点才按字数硬切
        let soft = (a + 1..cap).rfind(|&i| SOFT_ENDERS.contains(&chars[i - 1]));
        let at = soft.unwrap_or(cap);
        if at <= a || at >= chars.len() {
            break;
        }
        pieces.insert(pos, (a, at));
        pieces[pos + 1] = (at, pieces[pos + 1].1);
        cuts_cap_marker(&mut kept, at);
    }
    kept.sort_by_key(|(i, _)| *i);

    // ---- ④ 时间：标点说"在哪切"，气口说"切在哪一刻" ----
    let breath = timeline.breath_points(policy.breath_gap);
    let mut out: Vec<SplitPiece> = Vec::with_capacity(pieces.len());
    // 首条起点 = **窗口起点**。
    //
    // ⚠️ 别改成 `time_of(0)`：它对**没对齐上的字**返回的是**后一个锚点**的时间（见 `time_of`
    // 的 `(None, Some(..))` 分支），于是"qwen3 句首那几个字跟 SenseVoice 字轴对不上"时
    // （而这正是首音被削、听漏时的典型情形），这条字幕的起点会跳到第一个对得上的字 ——
    // 表现就是用户报的"句子开头几个字没有识别到"。窗口起点才是这段语音的真实起点。
    let mut next_start = span.0;
    for (k, (a, b)) in pieces.iter().enumerate() {
        let is_last = k + 1 == pieces.len();
        let start = next_start;
        let (end, src) = if is_last {
            // 尾条同理：用末字时间 + 一点余韵，而不是窗口末端
            let tail = align.time_of(b.saturating_sub(1), timeline, span) + 0.3;
            let hi = span.1.max(start);
            (clamp_ordered(tail, (start + MIN_STEP_SECS).min(hi), hi), None)
        } else {
            let raw = align.time_of(*b, timeline, span);
            let snapped = breath
                .iter()
                .min_by(|x, y| {
                    (x.time - raw).abs().partial_cmp(&(y.time - raw).abs()).unwrap()
                })
                .filter(|bp| {
                    (bp.time - raw).abs() <= policy.snap_window
                        && bp.time > start + MIN_PIECE_SECS
                })
                .map(|bp| bp.time);
            // 边界时刻 = 下一条的起点；本条的结束只保留 TRAIL_PAD 余韵，
            // 两者之差就是留给"停顿"的间隙（停顿不灌进字幕尾巴）。
            //
            // ⚠️ 切点**不做任何"份额/最小时长"的抬升**：早期为了让极短条好看，
            // 把切点按字数份额往后推，结果窗口的 `(start, end)` 不再覆盖它自己那些字 ——
            // hybrid 按 `slice_by_time(start, end)` 取字时取到空集，整片退回"一个窗口一条"，
            // 实测 1186 个窗口里 589 个丢了细分。短条问题改用下面的 `merge_too_short` 解决。
            let boundary_time =
                clamp_ordered(snapped.unwrap_or(raw), start + MIN_STEP_SECS, span.1.max(start));
            let prev_char = align.time_of(b.saturating_sub(1), timeline, span);
            let end = clamp_ordered(
                prev_char + ((raw - prev_char) / 2.0).min(TRAIL_PAD).max(0.0),
                start + MIN_STEP_SECS,
                boundary_time,
            );
            next_start = boundary_time;
            // 边界"由谁决定"取切点本身的来源；只有**标点**切点吸附到气口时才升级成
            // `PunctuationOnBreath`（强停顿/字数上限本来就是从时间轴来的，不该被改标签）
            let base = boundary_of(&kept, *b).unwrap_or(BoundarySource::Punctuation);
            let source = match base {
                BoundarySource::Punctuation if snapped.is_some() => {
                    BoundarySource::PunctuationOnBreath
                }
                other => other,
            };
            (end, Some(source))
        };
        out.push(SplitPiece {
            text: chars[*a..*b].iter().collect::<String>().trim().to_string(),
            start,
            end,
            boundary: src,
        });
    }

    out.retain(|p| !p.text.is_empty());
    merge_too_short(&mut out, policy.max_chars);
    out
}

/// 把短到"看不见"的条并进相邻条（**字数上限优先**）。
///
/// 为什么用**合并**而不是给时间加地板：一条字幕该不该存在是文本层的事实，
/// 而它的时间是算出来的 —— 对齐不可信时（qwen3 与 SenseVoice 差得多、或 qwen3 重复/胡说）
/// 会算出 0.05 秒这种跨度。给时间加地板试过，代价是扭曲切点、把窗口的
/// `(start, end)` 和它自己那些字弄得不一致（实测让近半窗口退回"一窗一条"）。
/// 合并只动文本归属，时间与分区都不受影响。
///
/// ⚠️ **并完超过 `max_chars` 就不并**：字数上限是用户定的硬线，而两个短条合起来的正文
/// 往往是"两句被粘回一条"（实测出现过 `怎么？你们都玩过吗？`）。两害相权，
/// 宁可留一条看得见但很短的条，也不要破坏断句与字数上限。
fn merge_too_short(pieces: &mut Vec<SplitPiece>, max_chars: usize) {
    let fits = |a: &SplitPiece, b: &SplitPiece| {
        a.text.chars().count() + b.text.chars().count() <= max_chars
    };
    let mut i = 0usize;
    // 每合并一次长度就减一，必然终止
    while i < pieces.len() && pieces.len() > 1 {
        if pieces[i].end - pieces[i].start >= MIN_VISIBLE_SECS {
            i += 1;
            continue;
        }
        if i == 0 {
            // 第一条太短就与后一条并（往后并，免得丢掉开头那句）
            if !fits(&pieces[0], &pieces[1]) {
                i += 1;
                continue;
            }
            let next = pieces.remove(1);
            pieces[0].text = format!("{}{}", pieces[0].text, next.text);
            pieces[0].end = next.end.max(pieces[0].end);
        } else {
            if !fits(&pieces[i - 1], &pieces[i]) {
                i += 1;
                continue;
            }
            let cur = pieces.remove(i);
            let prev = &mut pieces[i - 1];
            prev.text = format!("{}{}", prev.text, cur.text);
            prev.end = cur.end.max(prev.end);
            // 合并后的前一条可能仍然太短，退一格再看它
            i = (i - 1).max(1);
        }
    }
}

fn build_ranges(cuts: &[(usize, BoundarySource)], len: usize) -> Vec<(usize, usize)> {
    let mut ranges = Vec::with_capacity(cuts.len() + 1);
    let mut start = 0usize;
    for (idx, _) in cuts {
        if *idx > start && *idx < len {
            ranges.push((start, *idx));
            start = *idx;
        }
    }
    if start < len {
        ranges.push((start, len));
    }
    ranges
}

fn cuts_cap_marker(cuts: &mut Vec<(usize, BoundarySource)>, at: usize) {
    if !cuts.iter().any(|(i, _)| *i == at) {
        cuts.push((at, BoundarySource::CharCap));
    }
}

fn boundary_of(cuts: &[(usize, BoundarySource)], at: usize) -> Option<BoundarySource> {
    cuts.iter().find(|(i, _)| *i == at).map(|(_, s)| *s)
}

/// 用时间轴自身的文本切段（SenseVoice 的文本没有标点，于是切点只可能来自
/// **强停顿**与**字数硬上限**）。
///
/// hybrid 拿它的结果当喂给 qwen3 的窗口；single 直接把它当字幕段。
pub fn timeline_segments(timeline: &CharTimeline, policy: &SplitPolicy) -> Vec<SplitPiece> {
    if timeline.is_empty() {
        return Vec::new();
    }
    let span = (timeline.char_time(0), last_end(timeline));
    split_text_on_timeline(&timeline.text(), timeline, policy, span)
}

/// 时间轴末字的时间 + 一点余韵（窗口末端不能正好压在最后一个字上）。
pub fn last_end(timeline: &CharTimeline) -> f64 {
    if timeline.is_empty() {
        return 0.0;
    }
    let last = timeline.times[timeline.len() - 1];
    let gap = timeline.gap_at(timeline.len() - 1);
    last + gap.clamp(0.2, TRAIL_PAD)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tl(text: &str) -> CharTimeline {
        let chars: Vec<String> = text.chars().map(|c| c.to_string()).collect();
        // 每字 0.2s；空格视为 0.6s 的停顿（气口），用来在单测里摆布气口位置
        let mut times = Vec::new();
        let mut t = 0.0;
        for (i, c) in chars.iter().enumerate() {
            if i > 0 {
                t += if c == " " { 0.6 } else { 0.2 };
            }
            times.push(t);
        }
        CharTimeline::from_tokens(&chars, &times)
    }

    fn policy() -> SplitPolicy {
        SplitPolicy { breath_gap: 0.25, strong_gap: 1.0, soft_cut_secs: 6.0, max_chars: 30, snap_window: 0.3 }
    }

    /// 回归正题：三个问句的文本，SenseVoice 侧一个字标点都没有，
    /// 但气口正好落在问号处 —— 必须切成三条，且切点落在气口上。
    #[test]
    fn splits_three_questions_on_breaths() {
        let timeline = tl("你们不用上班了吗 你们不用赚钱了吗 你们不用上学了吗");
        let text = "你们不用上班的吗？你们不用赚钱的吗？你们不用上学的吗？";
        let span = (timeline.char_time(0), last_end(&timeline));
        let pieces = split_text_on_timeline(text, &timeline, &policy(), span);

        assert_eq!(pieces.len(), 3, "三个问句应切成三条: {pieces:#?}");
        assert!(pieces[0].text.ends_with('？'));
        assert!(pieces[1].text.ends_with('？'));
        assert_eq!(pieces[2].text, "你们不用上学的吗？");
        // 切点吸附到气口（不是按字数比例硬分）
        for p in &pieces[..2] {
            assert_eq!(p.boundary, Some(BoundarySource::PunctuationOnBreath), "{p:?}");
        }
    }

    /// 句末标点就算附近没有气口也要切（这时时间只能靠对齐插值）。
    #[test]
    fn hard_punctuation_splits_without_breath() {
        let timeline = tl("今天天气不错明天要下雨");
        let text = "今天天气不错。明天要下雨";
        let span = (timeline.char_time(0), last_end(&timeline));
        let pieces = split_text_on_timeline(text, &timeline, &policy(), span);
        assert_eq!(pieces.len(), 2, "{pieces:#?}");
        assert_eq!(pieces[0].text, "今天天气不错。");
        assert_eq!(pieces[0].boundary, Some(BoundarySource::Punctuation));
    }

    /// 逗号是软切：距上一条不够久就不切（避免"好，那，我"这种被切碎）。
    #[test]
    fn soft_punctuation_needs_elapsed_time() {
        let timeline = tl("好那我这就来");
        let text = "好，那我这就来";
        let span = (timeline.char_time(0), last_end(&timeline));
        let pieces = split_text_on_timeline(text, &timeline, &policy(), span);
        assert_eq!(pieces.len(), 1, "短句里的逗号不该切: {pieces:#?}");
    }

    /// 字数只是硬上限：没有标点也没有强停顿时才兜底切。
    #[test]
    fn char_cap_is_only_a_last_resort() {
        let text: String = "一二三四五六七八九十".repeat(4); // 40 字
        let timeline = tl(&text);
        let span = (timeline.char_time(0), last_end(&timeline));
        let pieces = split_text_on_timeline(&text, &timeline, &policy(), span);
        assert!(pieces.len() >= 2, "{pieces:#?}");
        for p in &pieces {
            assert!(p.text.chars().count() <= 30, "超过硬上限: {:?}", p.text);
        }
        assert_eq!(pieces[0].boundary, Some(BoundarySource::CharCap));
    }

    /// 强停顿在没有标点时独立切一条。
    #[test]
    fn strong_gap_splits_without_punctuation() {
        // "你好世界再见"：停顿摆在「界」与「再」之间（index 4，1.4s > strong_gap=1.0）
        let chars: Vec<String> = "你好世界再见".chars().map(|c| c.to_string()).collect();
        let times = vec![0.0, 0.2, 0.4, 0.6, 2.0, 2.2];
        let timeline = CharTimeline::from_tokens(&chars, &times);
        let span = (0.0, last_end(&timeline));
        let pieces = split_text_on_timeline("你好世界再见", &timeline, &policy(), span);
        assert_eq!(pieces.len(), 2, "{pieces:#?}");
        assert_eq!(pieces[0].text, "你好世界");
        assert_eq!(pieces[1].text, "再见");
        assert_eq!(pieces[0].boundary, Some(BoundarySource::StrongGap));
    }

    /// 切点必须严格递增、不重叠（重叠会连带毁掉静音检测与按时间二分取字幕）。
    #[test]
    fn pieces_are_monotonic_and_non_overlapping() {
        let timeline = tl("你们不用上班了吗 你们不用赚钱的吗 你们不用上学了吗");
        let text = "你们不用上班的吗？你们不用赚钱的吗？你们不用上学的吗？";
        let span = (timeline.char_time(0), last_end(&timeline));
        let pieces = split_text_on_timeline(text, &timeline, &policy(), span);
        for w in pieces.windows(2) {
            assert!(w[0].end <= w[1].start, "重叠: {:?} / {:?}", w[0], w[1]);
        }
    }

    /// 错字（qwen3 与 SenseVoice 不同字）要靠重新同步拉回来，不能整体错位。
    #[test]
    fn alignment_resyncs_over_typos() {
        let timeline = tl("刷抖音车你们不用上班");
        let text: Vec<char> = "刷赌人车你们不用上班".chars().collect();
        let a = align_to_timeline(&text, &timeline);
        assert!(a.resyncs >= 1);
        // 错字之后的字仍要对上（"你们不用上班" 末尾）
        let last_text = text.len() - 1;
        assert!(a.map[last_text].is_some(), "尾部没重新对齐上: {:?}", a.map);
    }

    /// 时间轴必须单调（跨 chunk 重叠区可能倒退）。
    #[test]
    fn monotonic_fix_repairs_backwards_overlap() {
        let mut timeline = CharTimeline {
            chars: vec!['你', '好', '世', '界'],
            times: vec![0.0, 1.0, 0.5, 2.0],
        };
        timeline.monotonic_fix();
        assert_eq!(timeline.times, vec![0.0, 1.0, 1.0, 2.0]);
    }

    /// 逐字时间戳 → 气口：只有字与字之间够长才算，token 内共享时间戳不算。
    #[test]
    fn breath_points_use_char_gaps() {
        let tokens = vec!["你".to_string(), "好".to_string(), "啊".to_string()];
        let timeline = CharTimeline::from_tokens(&tokens, &[0.0, 0.2, 1.0]);
        let breaths = timeline.breath_points(0.25);
        assert_eq!(breaths.len(), 1);
        assert_eq!(breaths[0].index, 2);
        assert!((breaths[0].gap - 0.8).abs() < 1e-9);
    }

    /// 多字符 token（换模型时的情形）也要摊成单字、共享时间戳。
    #[test]
    fn multi_char_tokens_are_flattened() {
        let tokens = vec!["你好".to_string(), "世界".to_string()];
        let timeline = CharTimeline::from_tokens(&tokens, &[0.0, 1.0]);
        assert_eq!(timeline.chars, vec!['你', '好', '世', '界']);
        assert_eq!(timeline.times, vec![0.0, 0.0, 1.0, 1.0]);
        // 同 token 内部是 0 间隔，不会造出假气口
        assert_eq!(timeline.gap_at(1), 0.0);
        assert_eq!(timeline.gap_at(2), 1.0);
    }

    /// 窗口切分：时间轴文字没标点，只能靠强停顿与硬上限。
    #[test]
    fn timeline_segments_split_on_strong_gaps() {
        let mut t = tl("你好世界再见");
        // 停顿摆在「界」与「再」之间（index 4）
        t.times = vec![0.0, 0.2, 0.4, 0.6, 2.0, 2.2];
        let pieces = timeline_segments(&t, &policy());
        assert_eq!(pieces.len(), 2, "{pieces:#?}");
        assert_eq!(pieces[0].text, "你好世界");
        assert_eq!(pieces[1].text, "再见");
        assert!(pieces[1].start >= pieces[0].end);
    }

    /// 合并**不许**突破字数硬上限，哪怕前者短到看不见。
    ///
    /// 两害相权：并起来就是"两句被粘回一条"（实测出现过 `怎么？你们都玩过吗？`），
    /// 而字数上限是用户明确定的硬线。
    #[test]
    fn merge_respects_char_cap() {
        let chars: Vec<String> = "怎么你们都玩过吗".chars().map(|c| c.to_string()).collect();
        let timeline = CharTimeline::from_tokens(&chars, &(0..chars.len()).map(|i| i as f64 * 0.1).collect::<Vec<_>>());
        let text = "怎么？你们都玩过吗？";
        let span = (0.0, 0.1); // 窗口极短 → 两条都短到看不见
        let mut p = SplitPolicy { max_chars: 30, ..policy() };
        p.max_chars = 6; // 故意压到很小的上限，模拟"并起来就超限"
        let pieces = split_text_on_timeline(text, &timeline, &p, span);
        for x in &pieces {
            assert!(
                x.text.chars().count() <= p.max_chars,
                "超过字数硬上限: {:?}",
                x.text
            );
        }
        let joined: String = pieces.iter().map(|x| x.text.as_str()).collect();
        assert_eq!(joined, text, "文本不许丢");
    }

    /// 对齐命中率低时（qwen3 与 SenseVoice 差得多 / qwen3 重复胡说），逐字时间会算得极短 ——
    /// 实测有窗口命中率 0.12、切出 **0.05 秒**的字幕。这类条应当被**并进相邻条**，
    /// 而不是靠给时间加地板（那会扭曲切点，让窗口与它自己的字不一致）。
    #[test]
    fn low_hit_rate_merges_undisplayable_pieces() {
        // 时间轴只有 4 个字；qwen3 却给出 12 个字的重复内容（几乎对不上）
        let chars: Vec<String> = "去不了呀".chars().map(|c| c.to_string()).collect();
        let timeline = CharTimeline::from_tokens(&chars, &[0.0, 0.1, 0.2, 0.3]);
        let text = "去不了。去不了。去不了。";
        let span = (0.0, 1.26); // 实际窗口 1.26s
        let pieces = split_text_on_timeline(text, &timeline, &policy(), span);

        assert!(!pieces.is_empty(), "{pieces:#?}");
        for (i, p) in pieces.iter().enumerate() {
            let dur = p.end - p.start;
            assert!(dur >= MIN_VISIBLE_SECS, "第 {i} 条只有 {dur:.3}s: {p:?}");
        }
        // 文本一个字都不能丢
        let joined: String = pieces.iter().map(|p| p.text.as_str()).collect();
        assert_eq!(joined, text);
        for w in pieces.windows(2) {
            assert!(w[0].end <= w[1].start, "重叠: {:?}", w);
        }
    }

    /// 退化窗口**不许 panic**：边界落点压到窗口末端时，
    /// `f64::clamp` 会因为 `min > max` 直接崩（实测在 3 小时真实素材上死过整个转写进程）。
    #[test]
    fn degenerate_spans_do_not_panic() {
        let timeline = tl("你们不用上班的吗你们不用赚钱了吗");
        let text = "你们不用上班的吗？你们不用赚钱的吗？";
        // 窗口末端正好压在最后一个字上（span.1 只比末字时间多一点点）
        let last = timeline.char_time(timeline.len() - 1);
        for span in [
            (timeline.char_time(0), last),          // 零余韵
            (timeline.char_time(0), last + 0.01),   // 一点点
            (last, last + 0.01),                    // 起点就贴着末端
            (0.0, 0.0),                             // 空窗口
            (5.0, 5.0),                             // 零宽窗口
        ] {
            let pieces = split_text_on_timeline(text, &timeline, &policy(), span);
            for (i, p) in pieces.iter().enumerate() {
                assert!(p.end >= p.start, "负时长: span={span:?} #{i} {p:?}");
            }
            for w in pieces.windows(2) {
                assert!(w[0].end <= w[1].start, "重叠: span={span:?} {:?}", w);
            }
        }
    }

    /// 字数上限必须对**很长的窗口**也成立：qwen3 有时在一个窗口里吐出远超字数的内容
    /// （实测一个窗口里会多出半句话）。这里 2000 字需要 66 次切分，
    /// 早点用"随手写的 64 轮就退出"会在还剩超长段时提前收手。
    #[test]
    fn char_cap_holds_for_very_long_window() {
        let text: String = "一二三四五六七八九十".repeat(200); // 2000 字，无标点
        let timeline = tl(&text);
        let span = (timeline.char_time(0), last_end(&timeline));
        let pieces = split_text_on_timeline(&text, &timeline, &policy(), span);
        assert!(pieces.len() >= 66, "切分轮数不够: {}", pieces.len());
        for p in &pieces {
            assert!(p.text.chars().count() <= 30, "仍有超长段: {}", p.text);
        }
    }

    /// ⚠️ **首条起点必须等于窗口起点**，不许用 `time_of(0)`。
    ///
    /// `time_of` 对没对齐上的字返回的是**后一个锚点**的时间 —— 实测 `你们不用上班的吗？`
    /// 这种"qwen3 句首与 SenseVoice 字轴对不上"的情形（首音被削时最常见），
    /// 起点会被推到第一个对得上的字，用户看到的就是"开头几个字没识别到"。
    #[test]
    fn first_piece_starts_at_window_start() {
        let timeline = tl("时看直播你们不用上班");
        // qwen3 只认了后半句（前半句它的字轴里没有对应）
        let text = "你们不用上班";
        let span = (timeline.char_time(0), last_end(&timeline));
        let pieces = split_text_on_timeline(text, &timeline, &policy(), span);
        assert_eq!(pieces.len(), 1, "{pieces:#?}");
        assert!(
            (pieces[0].start - span.0).abs() < 1e-9,
            "首条起点应等于窗口起点 {:.2}，实际 {:.2}",
            span.0,
            pieces[0].start
        );
        // 反向：起点绝不能晚于窗口起点（那才是"看起来丢了句首"）
        assert!(pieces[0].start <= span.0 + 1e-9);
    }
}
