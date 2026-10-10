//! §9 多版本成片：同一素材出 N 条不同**开头/结尾**的片子，用户挑一条。
//!
//! 规格见 `docs/plan/editing-decision-layer.md` §5.2 / §5.3。
//!
//! ⚠️ **本模块只算候选，不出片**：出片复用既有的 `cmd_render`
//! （规格 §5.3 明写"多版本不需要改这套 —— 就是跑 N 次，每次覆盖 selected"）。
//! 在这里另造一套并行出片机制 = 两条渲染路径，而它们的像素必须一致。
//!
//! 本模块**纯函数、无 IO**（候选只依赖字幕与爆点），所以可单测、可复现。

use serde::{Deserialize, Serialize};

/// 一条字幕的最简形态（只要判"是否句末"需要的字段）。
///
/// ⚠️ **不引 `parse::srt::Subtitle`**：那会把本模块与解析层绑死，
/// 而它需要的只是 `(start, end, text)` —— 调用方一行 `map` 就能给。
#[derive(Debug, Clone, PartialEq)]
pub struct Cue {
    pub start: f64,
    pub end: f64,
    pub text: String,
}

/// 一个候选边界。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Candidate {
    /// 边界所在时刻（秒）。
    pub at: f64,
    /// 落在这条边界的**理由**（规格 §5.2 的 `reason` 同源：用户要能问"凭什么是这里"）。
    pub reason: String,
}

/// 句末标点（规格 §5.2 的"不切断句子"）。
///
/// ⚠️ 含英文句点 `.` 会把 "3.5 秒" 这种也判成句末 —— 但这里判的是
/// **一整条字幕文本的结尾**，"3.5 秒"结尾不是 `.` ⇒ 不误判。
/// 反过来，漏掉 `.` 会让英文句末漏判 ⇒ 宁可留着。
const SENTENCE_END: &[char] = &['。', '！', '？', '…', '!', '?', ';', '；', '.'];

/// 两条字幕之间的**静默间隔**达到这个值，即使没有句末标点也算一个自然边界。
///
/// ⚠️ 0.35s 来自规格原话（"'与下一条间隔 > 0.35s'"）。它不是"观感调参"：
/// 中文叙述里 0.3s 以下的停顿是**句内换气**，切开会让听感断在词中间。
pub const SILENCE_GAP_SECS: f64 = 0.35;

/// 边界**离爆点太近**的禁入半径（规格 §5.2："爆点窗口内不许设边界"）。
///
/// ⚠️ 用**前后都禁**而不是"只禁窗口内"：边界紧贴在爆点前面同样毁掉爆点
/// （观众还没被炸到就先切了）。留 0.5s 的余量。
pub const HOTSPOT_KEEPOUT_SECS: f64 = 0.5;

/// 这条字幕是不是**句末**（规格 §5.2 的核心判据）。
///
/// ⚠️ 规格特别指出这条**不需要词级时间戳** —— 字幕自己有 `text` 就够了
/// （对比：真卡拉OK才要词级）。所以本项成本很低。
pub fn is_sentence_end(text: &str) -> bool {
    let t = text.trim_end();
    t.chars().last().is_some_and(|c| SENTENCE_END.contains(&c))
}

/// 这个时刻是不是一个**可切**的自然边界。
///
/// 判据（**两条满足其一**，规格 §5.2）：
/// ① 该字幕以句末标点结尾；② 与下一条字幕的间隔 > [`SILENCE_GAP_SECS`]。
///
/// ⚠️ **最后一条字幕不是自动合法的边界**（这条我第一版写错了）：`cues` 里没有
/// "下一条"**不等于**"下一条离得很远" —— 它也可能只是调用方只传了一段字幕
/// （按时间窗切过的、或者只解析了一部分）。原来那个 `None => true` 会让
/// **每一条"数组里最后一条"都成为可切点**，于是"间隔 0.1s 且没有句末标点"
/// 这种明显**切在句子中间**的位置也会出候选。
/// 现在：最后一条**必须自己有句末标点**才算边界，否则一律不算
/// （保守方向 —— 少给一个候选，好过切在句子中间）。
pub fn is_natural_boundary(cues: &[Cue], idx: usize) -> bool {
    let Some(cur) = cues.get(idx) else { return false };
    if is_sentence_end(&cur.text) {
        return true;
    }
    match cues.get(idx + 1) {
        // 没有下一条 ⇒ **不能**据此认为"离得远"（见上面的 ⚠️），保守地判否。
        None => false,
        Some(next) => next.start - cur.end > SILENCE_GAP_SECS,
    }
}

/// 这个时刻是不是**踩在爆点上**（规格 §5.2："爆点窗口内不许设边界"）。
fn near_hotspot(at: f64, hotspots: &[(f64, f64)]) -> bool {
    hotspots
        .iter()
        .any(|(s, e)| at >= s - HOTSPOT_KEEPOUT_SECS && at <= e + HOTSPOT_KEEPOUT_SECS)
}

/// 算**开头**候选（规格 §5.1 的 `[editing.openings] candidates`）。
///
/// * `cues` —— 字幕（已按时间排序，调用方保证）；
/// * `hotspots` —— 爆点窗口 `(start, end)`（来自 §6 的爆点判据）；
/// * `max` —— 最多给几个（给多了用户挑不过来，也给"跑 N 次"设了上限）。
///
/// ⚠️ **返回按时间升序**：调用方要拿它当"版本 0 / 1 / 2"，
/// 而"版本号 ↔ 哪个候选"必须是**稳定的**（同一工程两次跑出同样的顺序），
/// 否则用户说"第二条更好"、下次跑出来是另一条。
pub fn opening_candidates(cues: &[Cue], hotspots: &[(f64, f64)], max: usize) -> Vec<Candidate> {
    candidates_from(cues, hotspots, max, true)
}

/// 算**结尾**候选。
///
/// ⚠️ 与开头**同一套判据**（句末/静默/避爆点）—— 头尾的性质是对称的，
/// 分成两套规则会让"开头合法、结尾不合法"这种不对称悄悄出现。
pub fn ending_candidates(cues: &[Cue], hotspots: &[(f64, f64)], max: usize) -> Vec<Candidate> {
    candidates_from(cues, hotspots, max, false)
}

fn candidates_from(
    cues: &[Cue],
    hotspots: &[(f64, f64)],
    max: usize,
    openings: bool,
) -> Vec<Candidate> {
    if cues.is_empty() || max == 0 {
        return Vec::new();
    }
    let mut out = Vec::new();
    for i in 0..cues.len() {
        if !is_natural_boundary(cues, i) {
            continue;
        }
        // 开头取字幕的**起点**、结尾取字幕的**终点** ——
        // 取反了会在句中间切（开头落在上一句末尾、结尾落在下一句开头）。
        let at = if openings { cues[i].start } else { cues[i].end };
        if near_hotspot(at, hotspots) {
            continue;
        }
        let reason = if is_sentence_end(&cues[i].text) {
            "句末".to_string()
        } else {
            format!("静默 {:.2}s", cues[i].end - cues[i].start)
        };
        out.push(Candidate { at, reason });
    }
    // ⚠️ 按时间升序（不依赖 `cues` 已排序这件事 —— 调用方说排了，但
    // "版本号 ↔ 哪个候选"的稳定性不该建立在这个口头约定上）。
    out.sort_by(|a, b| a.at.partial_cmp(&b.at).unwrap_or(std::cmp::Ordering::Equal));
    out.dedup_by(|a, b| (a.at - b.at).abs() < 1e-9);
    // 开头取**最早**的几处、结尾取**最晚**的几处：
    // 开头的意义是"从哪里开始抓人"，越靠前越像一个开场；
    // 结尾取最晚的几处，否则"结尾"会被切在片子中段。
    if out.len() > max {
        if openings {
            out.truncate(max);
        } else {
            out.drain(..out.len() - max);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cue(start: f64, end: f64, text: &str) -> Cue {
        Cue { start, end, text: text.to_string() }
    }

    /// 一组连续字幕：每句 2 秒、句间无静默，全部以句号结尾。
    fn sentences(n: usize) -> Vec<Cue> {
        (0..n)
            .map(|i| {
                let s = i as f64 * 2.0;
                cue(s, s + 1.8, "这是一句话。")
            })
            .collect()
    }

    #[test]
    fn 句末标点判据() {
        for t in ["结束。", "真的吗！", "什么？", "未完…", "end.", "a;"] {
            assert!(is_sentence_end(t), "「{t}」应当判成句末");
        }
        for t in ["没有标点", "3.5 秒", ""] {
            assert!(!is_sentence_end(t), "「{t}」不该判成句末");
        }
    }

    #[test]
    fn 静默间隔也算自然边界() {
        // 第二条**没有**句末标点，但后面有 1 秒静默 ⇒ 它是自然边界。
        let cues = vec![
            cue(0.0, 2.0, "有句号。"),
            cue(3.0, 5.0, "没有标点"),
            cue(7.0, 9.0, "也没有标点"),
        ];
        assert!(is_natural_boundary(&cues, 1), "1 后面有 2 秒静默，应当是自然边界");
        // ⚠️ 第三条是最后一条、**且自己没有句末标点** ⇒ 按保守规则**不算**边界。
        assert!(
            !is_natural_boundary(&cues, 2),
            "最后一条没有句末标点时不该算边界（数组到末尾 ≠ 后面离得远）"
        );
        // 但它**有**句末标点时算（句末标点那条判据先命中）。
        let ended = vec![cue(0.0, 2.0, "有句号。"), cue(3.0, 5.0, "也有句号。")];
        assert!(is_natural_boundary(&ended, 1), "最后一条有句末标点 ⇒ 算边界");
        // 造一个**贴得很紧**的：间隔 0.1s 且无标点 ⇒ 不是边界。
        let tight = vec![cue(0.0, 2.0, "有句号。"), cue(2.1, 4.0, "没有标点")];
        assert!(!is_natural_boundary(&tight, 1), "间隔 0.1s 且无标点，不该算边界");
    }

    #[test]
    fn 爆点附近不出候选() {
        let cues = sentences(6);
        // 在第二条（t=2.0）上盖一个爆点 ⇒ 它必须被排除。
        let hs = vec![(1.8, 4.0)];
        let got = opening_candidates(&cues, &hs, 10);
        assert!(
            !got.iter().any(|c| (c.at - 2.0).abs() < 1e-9),
            "爆点窗口内的 t=2.0 仍然出了候选：{got:?}"
        );
        // ⚠️ 另一头：**离得远的照常出** —— 只验"排除"会让"全都排除了"也通过。
        assert!(
            got.iter().any(|c| (c.at - 0.0).abs() < 1e-9),
            "t=0.0 离爆点够远，应当仍然有候选：{got:?}"
        );
    }

    #[test]
    fn 爆点前后都禁() {
        let cues = sentences(6);
        // 爆点 (4.0, 5.0)：t=2.0 距它 2 秒（可以），t=4.0 在窗口内（禁）。
        let hs = vec![(4.0, 5.0)];
        let got = opening_candidates(&cues, &hs, 10);
        assert!(!got.iter().any(|c| (c.at - 4.0).abs() < 1e-9));
        assert!(got.iter().any(|c| (c.at - 2.0).abs() < 1e-9));
        // 紧贴在爆点**前面**的也不行（留 0.5s 余量）：t=4.0 已经在窗口内，
        // 这里验 t=3.8 那种贴边的（不在本用例的字幕点上，改用窗口边界验）。
        assert!(near_hotspot(3.6, &hs), "距爆点起点 0.4s 应当算贴得太近");
        assert!(!near_hotspot(3.4, &hs), "距爆点起点 0.6s 应当放行");
    }

    #[test]
    fn 抬头取最早尾取最晚() {
        let cues = sentences(10); // t = 0, 2, 4, ..., 18
        let opens = opening_candidates(&cues, &[], 3);
        assert_eq!(opens.len(), 3);
        assert_eq!(opens[0].at, 0.0, "开头候选必须从最早开始");
        assert_eq!(opens[2].at, 4.0);
        let ends = ending_candidates(&cues, &[], 3);
        assert_eq!(ends.len(), 3);
        // 结尾取**最晚**的三处：t = 14, 16, 18（取的是 cue.end = 15.8/17.8/19.8）。
        assert!(ends[2].at > ends[0].at, "结尾候选必须按时间升序");
        assert!(ends[2].at > 19.0, "结尾候选必须落在片子靠后的位置，实得 {}", ends[2].at);
        assert!(ends[0].at > 14.0, "结尾候选不该出现在中段，实得 {}", ends[0].at);
    }

    #[test]
    fn 候选按时间升序且去重() {
        // 打乱输入顺序 ⇒ 输出仍须升序（"版本号 ↔ 哪个候选"要稳定）。
        let cues = vec![
            cue(8.0, 10.0, "第三句。"),
            cue(0.0, 2.0, "第一句。"),
            cue(4.0, 6.0, "第二句。"),
        ];
        let got = opening_candidates(&cues, &[], 10);
        let times: Vec<f64> = got.iter().map(|c| c.at).collect();
        let mut sorted = times.clone();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
        assert_eq!(times, sorted, "候选没有按时间升序：{times:?} —— 版本号会不稳定");
    }

    #[test]
    fn 空输入与零上限() {
        assert!(opening_candidates(&[], &[], 3).is_empty());
        assert!(ending_candidates(&[], &[], 3).is_empty());
        assert!(opening_candidates(&sentences(3), &[], 0).is_empty());
    }

    #[test]
    fn 理由字段可解释() {
        let cues = vec![cue(0.0, 2.0, "有句号。"), cue(3.0, 5.0, "无标点")];
        let got = opening_candidates(&cues, &[], 10);
        assert!(got.iter().any(|c| c.reason == "句末"), "句末那条的理由应当是「句末」");
        // ⚠️ 第二条的理由是"静默 x" —— 但它自己后面没有下一条 ⇒ 按末尾算，理由仍是"静默"。
        // ⚠️ 第二条是最后一条且无标点 ⇒ 现在**不出候选**了（见 `is_natural_boundary` 的 ⚠️）。
        // 换一个"静默"的情形来验理由文案：三条都是无标点，中间那条后面有长静默。
        let three = vec![
            cue(0.0, 1.5, "无标点甲"),
            cue(3.0, 4.5, "无标点乙"),
            cue(6.0, 7.5, "无标点丙"),
        ];
        let got2 = opening_candidates(&three, &[], 10);
        assert!(
            got2.iter().any(|c| c.reason.starts_with("静默")),
            "靠静默成立的边界，理由应当写明是静默：{got2:?}"
        );
    }
}
