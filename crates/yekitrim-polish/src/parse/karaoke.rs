//! T5 真卡拉OK：**把对齐结论落成派生产物** `data/karaoke-cues.json`。
//!
//! # 它是什么、为什么要有它
//!
//! E4b 把 ASR 的**字级时间轴**搬进了工程（`data/char-timeline.json`，那是**数据**）。
//! 但"哪个字属于哪条字幕、每个字在第几秒"这一步（`charlog::align_chars_to_subtitles`）
//! 以前**没有任何消费者** —— 产物写出来了、没人读、不报错。
//! 本模块就是那个消费者：把**对齐结论**落盘，给转译器（`tools/polish-to-dhampir.mjs`）读。
//!
//! # ⚠️⚠️ 为什么它落 `data/`，而**不能**落进 `polish.toml`
//!
//! `commands.rs::sync_char_timeline` 的纪律原文是"**搬数据，不搬结论**"，
//! 理由是对齐结果"是可重算的派生值，落盘就有两个真源"。
//! 本模块**没有**推翻那条纪律，而是把它分层：
//!
//! ⚠️ **围栏必须标 `text`**（2026-10-13）：裸 ` ``` ` 会被 rustdoc 当成 **Rust 代码**去编译，
//! 而这段是**树形示意图**（里面有 `←`）⇒ 报 `unknown start of token: \u{2190}`，
//! `cargo test -p yekitrim-polish --doc` 恒红。**恒红的判据比没有判据更坏**。
//! ```text
//! data/char-timeline.json   ← ASR 原样的字级时间轴      （【数据】，E4b 落的）
//! data/karaoke-cues.json    ← 对齐后的逐字切分结论      （【结论】，本模块落的）
//! ```
//!
//! **为什么结论不写进 `polish.toml`**（两条，任一条都够否掉它）：
//!
//! 1. **`polish.toml` 是用户会手改的配置文件。** 结论混进去之后，用户一改就与 ASR 不一致，
//!    而且**没人知道谁对**（是用户改的还是 ASR 算的？）。配置文件里不该有"派生且会被覆盖"的段。
//! 2. **它是 R4 敏感面。** 写进 `polish.toml` 意味着**每个存量工程都会被追加一段派生数据** ⇒
//!    "不配 `karaoke` 时逐字节不变"这条红线当场就假了。
//!    `data/` 下的派生产物不参与 R4（存量工程本来就有 `data/`，且**没数据不落文件**，见下）。
//!
//! ⇒ 一句话：**结论可以落盘（它是派生的、放在 `data/` 里），但绝不能落进用户的配置**。
//!
//! # 三条纪律（与 `sync_char_timeline` 逐条对齐）
//!
//! 1. **没数据不落文件**：读不到 `char-timeline.json` / 一条都对不上 ⇒
//!    **不写这个文件**，调用方明说降级（"文件不存在"就是降级信号本身）。
//! 2. **只在真变了才写**：内容一致就一个字节都不碰（含 mtime）——
//!    首页「切片总览」按片段目录里最新文件的 mtime 排序，无条件重写会把老片段一直顶到最前面。
//! 3. **写失败只 warn**：卡拉OK是**增强**，不是工程成立的前提，不许阻断 `process`。
//!
//! # ⚠️ 与字幕文本的关系（一条已知差异，本期不处理）
//!
//! Rust `parse/srt.rs` 的多行文本是 `lines[2..].join(" ")`，而转译器（JS）的 `parseSrt`
//! 是 `join('\N')` ⇒ **多行字幕两边文本不同**。本模块的输入是 Rust 侧的
//! `data/subtitles.json`（Rust 口径），所以**内部自洽**；
//! ⚠️ 但**不要**把 JS 那侧的 `\N` 文本喂进这里的算法 —— `\N` **不是空白符**，
//! 会被当成两个"字"参与对齐、把游标顶歪。**本期不动这个差异**（它牵动字幕文本口径 = R4 面）。

use std::path::Path;

use serde::{Deserialize, Serialize};

use super::charlog::{self, CharTimelineLog};
use super::srt::Subtitle;

/// 派生产物的相对路径（片段目录下）。
pub const KARAOKE_CUES_REL: &str = "data/karaoke-cues.json";

/// 一条字幕的**逐字切分结论**。
///
/// ⚠️ 时间的单位是**秒**、且是**源时间轴**的时间（与 `char-timeline.json` 同一口径）——
/// **重映射到成片时间轴是转译器的事**（它才知道 `keepRanges`/EDL）。
/// 本模块只搬"哪个字在第几秒"，**不碰成片时间**。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KaraokeCue {
    /// 对应 `data/subtitles.json` 里那条字幕的 `index`（1 起）。
    pub subtitle_index: usize,
    /// 这一条字幕**在原字幕里的起止**（秒，源时间轴）—— 转译器用它找到是哪一条。
    pub start: f64,
    pub end: f64,
    /// 逐字文本（与 `times` 等长、逐字对应）。
    pub chars: Vec<String>,
    /// 每个字的**出现时刻**（秒，源时间轴、单调不减）。
    pub times: Vec<f64>,
}

/// 派生产物的整体形状。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KaraokeCues {
    /// 生成它的那份 `char-timeline.json` 的字数（改了它 ⇒ 结论该重算）。
    ///
    /// ⚠️ 这是**过期检测**用的：转译器/人都能一眼看出"结论是不是跟着数据走"。
    pub source_chars: usize,
    /// 逐字切分结论（**只含对得上的那些字幕**；对不上的**不在里面**）。
    pub cues: Vec<KaraokeCue>,
    /// **被质量闸退掉**的字幕条数（对不上的那些）。
    ///
    /// ⚠️ 必须落盘、必须被报出来：`karaoke` 开着却"一条都没切"时，
    /// 用户唯一能看到的解释就是这个数。**沉默地退回普通字幕是最坏的形态**
    /// （用户以为开着、其实没开）。
    pub skipped: usize,
}

/// 把对齐结论算成派生产物（**纯函数**，不碰磁盘 —— 便于判据逐值钉）。
///
/// 对不上任何一条 ⇒ 返回 `None`（= 没有结论，调用方**不落文件**、明说降级）。
///
/// ⚠️ `subs` 必须是**与转译器看到的那份一致**的字幕集合：
/// `process` 在读 `data/subtitles.json` 之后会**按 `mute_ranges` 过滤**，
/// 本函数的调用方必须传**过滤后**的那一份（否则结论里的 `subtitle_index` 指向的字幕
/// 在成片里根本不存在，转译器找不到它）。
pub fn build_karaoke_cues(log: &CharTimelineLog, subs: &[Subtitle]) -> Option<KaraokeCues> {
    // 长度对不上 / 空时间轴 ⇒ 当作没有字级数据（`charlog` 已有的口径，这里不重新判）。
    let all = log.chars_with_times()?;
    if all.is_empty() {
        return None;
    }
    let aligned = charlog::align_chars_to_subtitles(log, subs);
    // 一条都对不上 ⇒ **没有结论**（不是"结论是空的"）：不落文件。
    if aligned.is_empty() {
        return None;
    }
    // 退了几条 = 参与对齐的字幕数 - 真出结论的条数。
    // ⚠️ 这里数的是"**经过质量闸**之后"的差 —— 与 `align_chars_to_subtitles` 的回滚语义一致。
    let skipped = subs.len().saturating_sub(aligned.len());

    let by_index: std::collections::HashMap<usize, &Subtitle> =
        subs.iter().map(|s| (s.index, s)).collect();

    let mut cues = Vec::with_capacity(aligned.len());
    for a in &aligned {
        // ⚠️ 找不到对应字幕就**跳过这一条**，不 panic、不猜时间：
        // 那是"调用方传了过滤后的字幕、而 index 对不上"的内部不一致，
        // 丢掉它比编一个时间安全（编出来的时间会**看起来正常**）。
        let Some(sub) = by_index.get(&a.subtitle_index) else {
            continue;
        };
        cues.push(KaraokeCue {
            subtitle_index: a.subtitle_index,
            start: sub.start,
            end: sub.end,
            chars: a.chars.iter().map(|c| c.to_string()).collect(),
            times: a.times.clone(),
        });
    }
    if cues.is_empty() {
        return None;
    }
    Some(KaraokeCues {
        source_chars: all.len(),
        cues,
        skipped,
    })
}

/// 落盘 `data/karaoke-cues.json`（**只在真变了才写**，见模块头纪律 2）。
///
/// 返回写没写（`true` = 真写了）—— 调用方据此打日志。
/// ⚠️ **写失败只 warn**，不返回 `Err`：卡拉OK是增强，不许阻断 `process`。
pub fn write_karaoke_cues(project_path: &Path, cues: &KaraokeCues) -> bool {
    let path = project_path.join(KARAOKE_CUES_REL);
    if let Some(parent) = path.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            tracing::warn!("  ⚠ 逐字切分结论：建目录失败（{e}）—— 跳过写入");
            return false;
        }
    }
    let Ok(pretty) = serde_json::to_string_pretty(cues) else {
        tracing::warn!("  ⚠ 逐字切分结论：序列化失败 —— 跳过写入");
        return false;
    };
    // 「只在真变了才写」：内容一致就一个字节都不碰（含 mtime）。
    if std::fs::read_to_string(&path).map(|old| old == pretty).unwrap_or(false) {
        tracing::debug!("逐字切分结论未变化，跳过写入");
        return false;
    }
    match crate::project::write_atomic(&path, &pretty) {
        Ok(()) => true,
        Err(e) => {
            tracing::warn!("  ⚠ 逐字切分结论写入失败（不影响工程）: {} {e}", path.display());
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sub(index: usize, start: f64, end: f64, text: &str) -> Subtitle {
        Subtitle { index, start, end, text: text.into(), html: String::new() }
    }

    /// 逐字给 0.1s 间隔的时间轴（与 `charlog` 的测试同款口径）。
    fn log_from(text: &str) -> CharTimelineLog {
        let times: Vec<f64> = (0..text.chars().count()).map(|i| i as f64 / 10.0).collect();
        CharTimelineLog {
            text: text.into(),
            chars: text.chars().count(),
            times,
            breath_gap: 0.4,
            breaths: vec![],
        }
    }

    /// 完全对得上：逐字时间来自时间轴，条数与字数一致。
    #[test]
    fn builds_per_char_times_from_the_log() {
        let log = log_from("你好世界");
        let got = build_karaoke_cues(&log, &[sub(1, 0.0, 1.0, "你好"), sub(2, 1.0, 2.0, "世界")])
            .expect("对得上就该有结论");
        assert_eq!(got.source_chars, 4);
        assert_eq!(got.skipped, 0);
        assert_eq!(got.cues.len(), 2);
        assert_eq!(got.cues[0].chars, vec!["你", "好"]);
        assert_eq!(got.cues[0].times, vec![0.0, 0.1]);
        assert_eq!(got.cues[0].start, 0.0);
        assert_eq!(got.cues[0].end, 1.0);
        assert_eq!(got.cues[1].times, vec![0.2, 0.3], "第二条接着往后走");
    }

    /// ⚠️ 时间**不是均分**：把时间轴拉开成不等距，结论必须跟着数据走。
    ///
    /// 这是"逐字切分"与"打字机均分"的**分界判据** —— 均分实现会在这里红。
    #[test]
    fn times_are_not_uniformly_spread() {
        let log = CharTimelineLog {
            text: "ABCD".into(),
            chars: 4,
            // 故意不等距：0 / 0.05 / 0.9 / 0.95
            times: vec![0.0, 0.05, 0.9, 0.95],
            breath_gap: 0.4,
            breaths: vec![],
        };
        let got = build_karaoke_cues(&log, &[sub(1, 0.0, 1.0, "ABCD")]).expect("有结论");
        assert_eq!(got.cues[0].times, vec![0.0, 0.05, 0.9, 0.95]);
        // 均分的会是 0/0.25/0.5/0.75 —— 明确钉住"不是它"
        assert_ne!(got.cues[0].times, vec![0.0, 0.25, 0.5, 0.75]);
    }

    /// 完全对不上 ⇒ **没有结论**（`None`）⇒ 调用方不落文件、明说降级。
    #[test]
    fn no_alignment_yields_no_artifact() {
        // 时间轴是「ABCD」，字幕是毫不相干的「甲乙丙丁」⇒ 命中率 0 ⇒ 全被闸掉
        let log = log_from("ABCD");
        assert!(build_karaoke_cues(&log, &[sub(1, 0.0, 1.0, "甲乙丙丁")]).is_none());
    }

    /// 一条对不上、一条对得上 ⇒ 结论只含对得上的那条，**并把退掉的条数报出来**。
    #[test]
    fn partial_alignment_reports_skipped_count() {
        let log = log_from("你好世界");
        let got = build_karaoke_cues(
            &log,
            &[sub(1, 0.0, 1.0, "你好"), sub(2, 1.0, 2.0, "甲乙丙丁")],
        )
        .expect("至少有一条对得上");
        assert_eq!(got.cues.len(), 1);
        assert_eq!(got.cues[0].subtitle_index, 1);
        assert_eq!(got.skipped, 1, "退了一条必须报出来");
    }

    /// ⚠️ **单调**：同一个字不许在不同条字幕里出现两次（游标不回退）。
    #[test]
    fn times_are_monotonic_across_cues() {
        let log = log_from("你好世界");
        let got = build_karaoke_cues(&log, &[sub(1, 0.0, 1.0, "你好"), sub(2, 1.0, 2.0, "世界")])
            .expect("有结论");
        let flat: Vec<f64> = got.cues.iter().flat_map(|c| c.times.iter().copied()).collect();
        for w in flat.windows(2) {
            assert!(w[1] >= w[0], "时间不许回退: {flat:?}");
        }
    }

    /// 空时间轴 / 长度对不上 ⇒ 没有结论（照 `charlog` 已有的口径，不在这里另判一套）。
    #[test]
    fn degenerate_log_yields_nothing() {
        let empty = CharTimelineLog::default();
        assert!(build_karaoke_cues(&empty, &[sub(1, 0.0, 1.0, "你")]).is_none());

        let mut mismatched = log_from("你好");
        mismatched.times.pop();
        assert!(build_karaoke_cues(&mismatched, &[sub(1, 0.0, 1.0, "你好")]).is_none());
    }

    /// `subtitle_index` 找不到对应字幕 ⇒ **丢掉那一条**，不 panic、不编时间。
    #[test]
    fn missing_subtitle_is_dropped_not_invented() {
        let log = log_from("你好世界");
        // 只有 index=2 的字幕，而对齐是按传入顺序算的 —— 传进去的就是它自己
        let got = build_karaoke_cues(&log, &[sub(2, 1.0, 2.0, "你好")]).expect("有结论");
        assert_eq!(got.cues.len(), 1);
        assert_eq!(got.cues[0].subtitle_index, 2);
    }

    /// 结论会**逐字带上原文**（不是序号、不是索引）—— 转译器要拿它拼 cue 文本。
    #[test]
    fn chars_are_the_actual_glyphs_in_order() {
        let log = log_from("你好世界");
        let got = build_karaoke_cues(&log, &[sub(1, 0.0, 2.0, "你好世界")]).expect("有结论");
        assert_eq!(got.cues[0].chars, vec!["你", "好", "世", "界"]);
    }

    /// 序列化能往返（转译器读的就是这份 JSON）。
    #[test]
    fn roundtrips_through_json() {
        let log = log_from("你好");
        let got = build_karaoke_cues(&log, &[sub(1, 0.0, 1.0, "你好")]).expect("有结论");
        let text = serde_json::to_string_pretty(&got).expect("能序列化");
        let back: KaraokeCues = serde_json::from_str(&text).expect("能读回来");
        assert_eq!(back, got);
        // 字段名是**接口**（转译器按名字读）—— 改名就是破坏兼容，钉住它。
        assert!(text.contains("\"source_chars\""));
        assert!(text.contains("\"cues\""));
        assert!(text.contains("\"skipped\""));
        assert!(text.contains("\"subtitle_index\""));
        assert!(text.contains("\"times\""));
    }
}
