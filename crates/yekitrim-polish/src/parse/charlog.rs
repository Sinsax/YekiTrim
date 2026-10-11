//! 字级时间戳搬运：把 ASR 的 `asr/timeline.json` 接进精修工程，并对到字幕上。
//!
//! # 为什么要有这个模块
//!
//! 总表一度把「真卡拉OK（T5）」记为"需要词级时间戳（本仓数据缺口）"—— **那个说法不成立**：
//! 数据从 2026-09 起就有，而且比词级更细（**逐字**）：
//!
//! | 环节 | 位置 |
//! |---|---|
//! | SenseVoice 逐字时间戳 → `CharTimeline` | `yekitrim-core/src/asr/align.rs:70`（`chars[i]` 发生在 `times[i]`） |
//! | 落盘 `<output_dir>/asr/timeline.json` | `asr/artifacts.rs:74` `timeline_record` + `:119` 写入 |
//! | 由转写收尾调用 | `asr/mod.rs:338` |
//!
//! 真正卡住的是：**`yekitrim-polish` 一行都不读 `asr/`**。本模块补的就是这一段搬运。
//!
//! # ⚠️ 目录关系：**两个目录不是同一个**
//!
//! 这是本模块最容易做错的地方，实测核对过：
//!
//! | 概念 | 实际路径 |
//! |---|---|
//! | `ctx.output_dir`（管线侧） | `<工程根>/output`（`yekitrim.toml [cutting] output_dir`，默认 `./output`） |
//! | **片段目录**（= polish 的 `project_path`） | `<工程根>/output/clips/<dir_name>` |
//!
//! 依据：`cutting/mod.rs:47` `let clips_dir = ctx.output_dir.join("clips")`、`:101`
//! `let clip_dir = cd.join(&dir_name)`。所以 `片段目录 = output_dir/clips/<名>`，是**结构性**父子关系。
//! 实测样本 `.workbuddy/edl-replay/output/clips/demo-clip/`（内含 `polish.toml` + `data/`）与
//! 同级的 `output/` 并存 —— 两个目录确实不同。
//!
//! ⚠️ **polish 的入口拿不到 `ctx`**（签名只有 `project_path`），而 `output_dir` 是**可配置**的，
//! 所以只能用**路径反推**。口径照抄本仓已有的 `resolve_danmaku`（`pipeline/runner.rs:24`）：
//! **多个候选基准，第一个存在的赢**。
//!
//! # ⚠️ 缺文件必须优雅（与 `_proxy_720.mp4` 同口径）
//!
//! 存量工程的**绝大部分**都没有 `asr/timeline.json`（它只在真跑过 Stage 2 的工程里有）。
//! 所以这里的规矩是**「没有就没有」**：不报错、不阻断 `init`/`process`、**不写任何文件**。
//! ⚠️ **"不写文件"是 R4 逐字节零回归的一部分**：如果没数据也落一份空的
//! `data/char-timeline.json`，存量工程目录里就会**多出一个文件**，"删掉 asr 文件 ⇒ 行为与今天
//! 逐字节相同"这条判据当场就假了。

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::srt::Subtitle;

/// 精修工程里字级时间戳的落点（相对片段目录），与 `data/subtitles.json` 同级。
pub const CHAR_TIMELINE_FILE: &str = "char-timeline.json";

/// `data/` 下这一份的名字（拼给调用方用）。
pub const CHAR_TIMELINE_REL: &str = "data/char-timeline.json";

/// 字级时间轴：`chars[i]` 发生在 `times[i]`（**秒**，与字幕同一基准）。
///
/// 这是 `yekitrim-core::asr::align::CharTimeline` 的**只读镜像** —— core 那个类型没有
/// `Deserialize`（它是管线内部的活对象），而这里要的是"读一份已经落盘的 JSON"，
/// 所以字段照它的落盘形态（`artifacts::TimelineRecord`）声明，不直接复用那个类型。
///
/// ⚠️ **加 `deny_unknown_fields` 是有意的取舍**：`timeline.json` 是**本仓自己的中间产物**、
/// 不是用户手改的配置文件，所以宁可"多一个字段就当场读不出来"（暴露上游改了形状），
/// 也不要静默丢掉不认识的内容。这一点与 `ProjectFile`（用户会手写，必须宽容）**方向相反**。
/// 真出现"底座/上游加了字段"的情况，会是一句明确的 serde 报错，而不是一个安静的错误结果。
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct CharTimelineLog {
    /// 全部字拼成的字符串（`CharTimeline::text()`，`asr/align.rs:147`）。
    #[serde(default)]
    pub text: String,
    /// 字数（= `text.chars().count()`，也 = `times.len()`）。
    ///
    /// ⚠️ 上游 `from_tokens` 会**丢掉替换符 `\u{FFFD}`**（`align.rs:83`），
    /// 所以这是**过滤后**的字数，与 `timeline.json` 里的 `text` 逐字对应。
    #[serde(default)]
    pub chars: usize,
    /// 逐字时间戳（秒）。长度与 `text` 的字符数一致。
    #[serde(default)]
    pub times: Vec<f64>,
    /// 判定气口的间隔阈值（秒）—— 原样带过来，只作参考。
    #[serde(default)]
    pub breath_gap: f64,
    /// 气口位置（停顿 ≥ `breath_gap` 的地方）。
    #[serde(default)]
    pub breaths: Vec<Breath>,
}

/// 一个气口：停顿之后那个字的 `index` / `time`，以及停顿长度 `gap`。
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct Breath {
    #[serde(default)]
    pub index: usize,
    #[serde(default)]
    pub time: f64,
    #[serde(default)]
    pub gap: f64,
}

impl CharTimelineLog {
    /// 字表：`text` 与 `times` 都非空且长度一致时给出逐字 (字, 秒)。
    ///
    /// 长度对不上（截断的 JSON、上游改了形状）⇒ `None` = **当作没有字级数据**。
    /// 这里**不猜测、不补齐** —— 对不齐的时间轴比没有时间轴更坏（会静默画错）。
    pub fn chars_with_times(&self) -> Option<Vec<(char, f64)>> {
        let chars: Vec<char> = self.text.chars().collect();
        if chars.is_empty() || chars.len() != self.times.len() {
            return None;
        }
        Some(chars.into_iter().zip(self.times.iter().copied()).collect())
    }
}

/// 去哪找 `asr/timeline.json`（**从上往下**，第一个存在的赢）。
///
/// ① 片段目录的**上两级** + `asr/` —— 规范布局：`<工程根>/output/clips/<名>` ⇒ `<工程根>/output/asr/`
/// ② 片段目录的**上一级** + `asr/` —— 万一 `clips` 那层不存在（`output_dir` 直接就是父目录）
/// ③ 片段目录**自己**下的 `asr/` —— 手工搭的工程（把 asr 放在片段里）
///
/// ⚠️ **顺序有判据**（`candidate_order_prefers_canonical_layout`）：多个候选**同时存在**时
/// 必须①赢。顺序写错了在正常布局下**根本看不出来**（因为通常只有一个存在）——
/// 这正是本仓反复踩的"跑得通、结果不对"。
///
/// ⚠️ 与 `resolve_danmaku` 同一套写法：**先收集所有候选、再挑第一个存在的**，
/// 而不是"边走边返回"（那样顺序错了也没法测）。
pub fn char_timeline_candidates(clip_dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::with_capacity(3);
    if let Some(root) = clip_dir.parent().and_then(|p| p.parent()) {
        out.push(root.join("asr").join("timeline.json"));
    }
    if let Some(parent) = clip_dir.parent() {
        out.push(parent.join("asr").join("timeline.json"));
    }
    out.push(clip_dir.join("asr").join("timeline.json"));
    out
}

/// 读字级时间轴。**没有就是没有** —— 任何失败（找不到 / 读不动 / 解析不了 / 长度对不上）
/// 一律返回 `None`，**不报错**。
///
/// 调用方只该在 `Some` 时才落盘 `data/char-timeline.json`（见模块头的 R4 说明）。
pub fn load_char_timeline(clip_dir: &Path) -> Option<CharTimelineLog> {
    for candidate in char_timeline_candidates(clip_dir) {
        if !candidate.is_file() {
            continue;
        }
        let raw = match std::fs::read_to_string(&candidate) {
            Ok(r) => r,
            Err(e) => {
                tracing::debug!("字级时间戳读取失败（跳过，不影响工程）: {} {e}", candidate.display());
                continue;
            }
        };
        match serde_json::from_str::<CharTimelineLog>(&raw) {
            Ok(log) => {
                if log.chars_with_times().is_none() {
                    tracing::debug!(
                        "字级时间戳形状不合法（{} 字 vs {} 个时间）—— 当作没有: {}",
                        log.char_count(),
                        log.times.len(),
                        candidate.display()
                    );
                    continue;
                }
                return Some(log);
            }
            Err(e) => {
                tracing::debug!("字级时间戳解析失败（跳过，不影响工程）: {} {e}", candidate.display());
            }
        }
    }
    None
}

impl CharTimelineLog {
    /// 字数（`text` 的字符数）—— 诊断/日志用。
    pub fn char_count(&self) -> usize {
        self.text.chars().count()
    }
}

/// 一条字幕上的逐字时间：`chars[i]` 发生在 `times[i]`。
#[derive(Debug, Clone, PartialEq)]
pub struct SubtitleChars {
    /// 对应字幕的 `index`（`Subtitle::index`）。
    pub subtitle_index: usize,
    pub chars: Vec<char>,
    pub times: Vec<f64>,
}

/// **字幕文本 ↔ 字级时间轴**的对应（纯函数）。
///
/// # 为什么不能"逐字相等"去对
///
/// 两条链路的文本**本来就不一样**：
/// - `timeline.json` 记的是 `asr::text::normalize` **之前**的原文（`artifacts.rs:13` 明说有意如此）；
/// - `clip.srt` 是 normalize **之后**的排版结果：行尾 `。.` `，、；：,;:` 被去掉、转述引号被删
///   （`asr/text.rs::normalize`）。
///
/// 所以直接按 `==` 去对必然大面积为空。
///
/// # 算法：**子序列贪心 + 命中率闸**
///
/// `normalize` 只**删**字符（行尾标点、引号）与把 `：` 换成 `，`，**从不重排** ——
/// 即字幕文本是时间轴文本的**子序列**（个别字符可能被替换/写错）。所以对每条字幕：
///
/// 1. 从时间轴的 `cursor` 起，**按序**走字幕的每个非空白字：
///    在**前方 `SEARCH_AHEAD` 个字以内**找第一个相等的字 —— 找到就跳过去（消化掉删/插），
///    找不到就**原地吃一个**（当作"替换"，字幕那个字按时间轴当前位置计时）；
/// 2. 走完这一条后算**命中率** = 真正相等的字数 ÷ 字数；
///    ⚠️ 低于 [`MIN_HIT_RATE`] ⇒ **整条作废、游标回滚**，不产出条目；
/// 3. 通过的字条把 `cursor` 留在最后吃到的位置 + 1（**单调、不回头**）。
///
/// ⚠️ **命中率闸是必需的，不是保险丝**：没有它，任何一段文本都能靠"原地吃替身"凑出一个
/// 结果 —— 于是"字幕与时间轴毫无关系"（工程拼错、字幕被人工改过）也会产出一串**看起来正常
/// 但全是错的时间**。那正是本仓最怕的"跑得通、结果不对"。加了闸之后，对不上就是**没有**。
///
/// ⚠️ **单调是硬要求**：字幕按时间有序，字也必须只往后走 —— 否则第二条字幕会把第一条的
/// 字再吃一遍，时间凭空回退。
///
/// ⚠️ **作废的字条回滚游标**：不回滚的话，"一条对不上的字幕"会白白吃掉时间轴上一段字，
/// 把它后面**本来能对上**的字也顶掉（错一条连累一片）。
///
/// ⚠️ `chars` 与 `times` 长度不一致、或时间轴为空 ⇒ 返回**空数组**（= 没有字级数据）。
pub fn align_chars_to_subtitles(log: &CharTimelineLog, subs: &[Subtitle]) -> Vec<SubtitleChars> {
    let Some(tl) = log.chars_with_times() else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let mut cursor = 0usize;

    for sub in subs {
        let start_cursor = cursor;
        let mut chars = Vec::new();
        let mut times = Vec::new();
        let mut matched = 0usize;
        // 跳过空白：字幕里的空格/换行不占时间轴（SRT 的 `\N` 折行等）
        for c in sub.text.chars().filter(|c| !c.is_whitespace()) {
            if cursor >= tl.len() {
                break;
            }
            let (tc, t) = tl[cursor];
            if tc == c {
                chars.push(tc);
                times.push(t);
                matched += 1;
                cursor += 1;
            } else {
                // 错字 / normalize 改过的标点：只在**前方一小段**里找得回来才跳，
                // 否则按"替换"原地吃掉当前位置（时间不漂）。
                let found = tl[cursor..].iter().take(SEARCH_AHEAD + 1).position(|(tc2, _)| *tc2 == c);
                match found {
                    Some(off) => {
                        cursor += off;
                        let (tc2, t2) = tl[cursor];
                        chars.push(tc2);
                        times.push(t2);
                        matched += 1;
                        cursor += 1;
                    }
                    None => {
                        chars.push(tc);
                        times.push(t);
                        cursor += 1;
                    }
                }
            }
        }

        // 命中率不足 ⇒ 整条作废并**回滚游标**（不连累后面的字条）
        let hit_rate = if chars.is_empty() { 0.0 } else { matched as f64 / chars.len() as f64 };
        if chars.is_empty() || hit_rate < MIN_HIT_RATE {
            cursor = start_cursor;
            continue;
        }
        out.push(SubtitleChars { subtitle_index: sub.index, chars, times });
    }
    out
}

/// 错字时往后找多远。太大 = 一条字幕能吃掉后面几条的字（时间整体前移），
/// 太小 = 偶发错字就退化成"替身"。4 是本仓既有的错位容忍量级
/// （`asr/align.rs` 的 `RESYNC_LOOKAHEAD = 4`）。
const SEARCH_AHEAD: usize = 4;

/// 一条字幕至少要有的**真实命中率**才认它（见 [`align_chars_to_subtitles`] 的 ⚠️）。
///
/// `normalize` 只删标点/引号，实测那类差异占行内字符的很小一部分；留 0.5 是**故意宽松**的
/// （宁可多对几条半准的，也不要因为一两个错字整条丢掉），同时把"完全无关"（命中率≈0）挡在外面。
const MIN_HIT_RATE: f64 = 0.5;


#[cfg(test)]
mod tests {
    use super::*;

    fn sub(index: usize, text: &str) -> Subtitle {
        Subtitle { index, start: 0.0, end: 1.0, text: text.into(), html: String::new() }
    }

    /// 造一份 `timeline.json`（形状照 `artifacts::timeline_record`：逐字给 0.1s 间隔）。
    /// ⚠️ 这是**按结构造的样本**，不是真链路产出的 —— 真跑 ASR 要模型与音频。
    ///
    /// ⚠️ 时间用 `i / 10.0` 而不是 `i as f64 * 0.1`：后者在 f64 上会积出
    /// `0.30000000000000004`，让断言里那些"应该是 0.3"的字面量对不上 —— 那是**测试自己的**
    /// 浮点噪声，不是被测代码的问题。真链路的 `times` 来自模型，本来就不是十进制整数。
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

    // ===== 结构镜像：字段名/类型必须与 `timeline_record()` 一致 =====

    /// `TimelineRecord`（`asr/artifacts.rs:59-72`）的真实落盘形状必须能读进来。
    /// 字段名逐字抄自那个结构体 + `BreathRecord{index,time,gap}`。
    #[test]
    fn matches_timeline_record_shape() {
        let raw = r#"{
            "text": "你好世界",
            "chars": 4,
            "times": [0.0, 0.32, 0.65, 0.9],
            "breath_gap": 0.4,
            "breaths": [{ "index": 2, "time": 0.65, "gap": 0.33 }]
        }"#;
        let log: CharTimelineLog = serde_json::from_str(raw).expect("真链路形状必须可读");
        assert_eq!(log.text, "你好世界");
        assert_eq!(log.chars, 4);
        assert_eq!(log.times, vec![0.0, 0.32, 0.65, 0.9]);
        assert_eq!(log.breath_gap, 0.4);
        assert_eq!(log.breaths.len(), 1);
        assert_eq!(log.breaths[0].index, 2);
        assert_eq!(log.breaths[0].gap, 0.33);
        assert_eq!(log.chars_with_times().unwrap().len(), 4);
    }

    /// ⚠️ 有意加严：`deny_unknown_fields`（这是本仓自己的中间产物，不是用户手改的配置）。
    /// 上游加了字段 ⇒ **当场报错**，而不是静默丢内容。
    #[test]
    fn unknown_field_is_rejected_on_purpose() {
        let raw = r#"{"text":"a","chars":1,"times":[0.0],"breath_gap":0.4,"breaths":[],"words":[]}"#;
        let err = serde_json::from_str::<CharTimelineLog>(raw).expect_err("多字段必须报错");
        assert!(err.to_string().contains("words"), "要点名是哪个键: {err}");
    }

    /// 长度对不上 ⇒ 当作**没有**字级数据（不猜、不补齐）。
    #[test]
    fn mismatched_lengths_degrade_to_none() {
        let mut log = log_from("你好");
        log.times.pop();
        assert!(log.chars_with_times().is_none());
        assert!(align_chars_to_subtitles(&log, &[sub(1, "你好")]).is_empty());
        // 空时间轴同理
        let empty = CharTimelineLog::default();
        assert!(empty.chars_with_times().is_none());
        assert!(align_chars_to_subtitles(&empty, &[sub(1, "你")]).is_empty());
    }

    // ===== 纯函数：字幕 ↔ 字级时间轴 =====

    /// 完全一致时逐字对齐，时间就是时间轴上的值。
    #[test]
    fn aligns_exactly_when_texts_match() {
        let log = log_from("你好世界");
        let got = align_chars_to_subtitles(&log, &[sub(1, "你好"), sub(2, "世界")]);
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].subtitle_index, 1);
        assert_eq!(got[0].chars, vec!['你', '好']);
        assert_eq!(got[0].times, vec![0.0, 0.1]);
        assert_eq!(got[1].chars, vec!['世', '界']);
        assert_eq!(got[1].times, vec![0.2, 0.3], "第二条要接着往后走，不能回头");
    }

    /// **真实差异**：`normalize` 删掉了引号、把 `：` 换成 `，`、收敛行尾句号
    /// （`asr/text.rs`）。时间轴上还留着那些字符 ⇒ 字幕文本是它的**子序列**。
    ///
    /// ⚠️ 注意 `，` 在时间轴上是 `：` —— 它**不同**，但前方 4 个字内找不到 `，`，
    /// 所以按"替换"吃掉当前的 `：`（这正是本函数对付 normalize 改写的方式：
    /// **占一个位置、拿时间轴上的时间**，而位置由"跳过被删掉的引号"来纠正）。
    #[test]
    fn tolerance_to_normalize_deletions() {
        // 时间轴 = normalize 之前的原文
        let log = log_from("他说：“你好。”");
        // 落盘字幕 = normalize 之后的
        let got = align_chars_to_subtitles(&log, &[sub(1, "他说，你好")]);
        assert_eq!(got.len(), 1, "{got:?}");
        let s = &got[0];
        assert_eq!(s.chars, vec!['他', '说', '：', '你', '好']);
        // 时间轴索引:      他0 说1 ：2 “3 你4 好5 。6 ”7
        // `“` 被**跳过**（搜回来），所以「你」吃到的是索引 4 的时间 0.4，不是数下来的 0.3
        assert_eq!(s.times, vec![0.0, 0.1, 0.2, 0.4, 0.5]);
    }

    /// 时间**单调不回退**：第二条字幕不许把第一条的字再吃一遍。
    #[test]
    fn cursor_never_goes_backwards() {
        let log = log_from("你好世界你好");
        let got = align_chars_to_subtitles(&log, &[sub(1, "你好"), sub(2, "世界"), sub(3, "你好")]);
        assert_eq!(got.len(), 3);
        let flat: Vec<f64> = got.iter().flat_map(|s| s.times.clone()).collect();
        assert!(flat.windows(2).all(|w| w[0] < w[1]), "时间必须严格递增: {flat:?}");
        assert_eq!(got[2].times, vec![0.4, 0.5], "第三次出现的是**后面**那两个，不是开头那两个");
    }

    /// 错字（替换）不把整条字幕带偏：搜不回来就按"替身"吃掉**时间轴上当前位置那个字**，
    /// 位置与时间都不漂；命中率够高照常通过。
    #[test]
    fn substitution_stays_local() {
        let log = log_from("你好世界");
        // 字幕写错了一个字（时间轴是「好」，字幕是「妈」）—— 4 个字里 3 个命中 = 0.75 ≥ 0.5
        let got = align_chars_to_subtitles(&log, &[sub(1, "你妈世界")]);
        assert_eq!(got.len(), 1, "{got:?}");
        let s = &got[0];
        assert_eq!(s.chars[1], '好', "吃掉的是**时间轴**当前位置那个字（替身），不是字幕那个字");
        assert_eq!(s.times[1], 0.1, "位置不变，时间不漂");
        assert_eq!(s.chars.last(), Some(&'界'), "后面的字照常接上");
        assert_eq!(s.times, vec![0.0, 0.1, 0.2, 0.3]);
    }

    /// ⚠️ **命中率闸**：字幕与时间轴**完全无关**（工程拼错 / 字幕被人工换过）⇒
    /// 整条作废，**不许**靠"原地吃替身"凑出一串看着正常、其实全错的时间。
    #[test]
    fn unrelated_subtitle_is_rejected_by_hit_rate() {
        let log = log_from("你好世界");
        let got = align_chars_to_subtitles(&log, &[sub(1, "abcdef")]);
        assert!(got.is_empty(), "毫无关系的字幕必须作废: {got:?}");
    }

    /// 作废的字条必须**回滚游标** —— 否则一条坏字幕会白白吃掉一段字轴，
    /// 把它后面本来能对上的字幕一起顶掉（"错一条连累一片"）。
    #[test]
    fn rejected_subtitle_rolls_back_cursor() {
        let log = log_from("你好世界");
        // 第 1 条完全对不上；第 2 条本该从字轴**开头**对上
        let got = align_chars_to_subtitles(&log, &[sub(1, "abcdef"), sub(2, "你好")]);
        assert_eq!(got.len(), 1, "只有第 2 条该留下: {got:?}");
        assert_eq!(got[0].subtitle_index, 2);
        assert_eq!(got[0].times, vec![0.0, 0.1], "游标回滚后必须从字轴开头对上");
    }

    /// 空白不占位（字幕里的空格 / SRT 的 `\N` 折行）。
    #[test]
    fn whitespace_does_not_consume_timeline() {
        let log = log_from("你好世界");
        let got = align_chars_to_subtitles(&log, &[sub(1, "你好\n世界")]);
        assert_eq!(got[0].chars, vec!['你', '好', '世', '界']);
        assert_eq!(got[0].times, vec![0.0, 0.1, 0.2, 0.3]);
    }

    /// 时间轴用量超出 ⇒ 剩下的字不再产出，但**已产出的部分保持正确**（不 panic）。
    #[test]
    fn subtitle_longer_than_timeline_stops_cleanly() {
        let log = log_from("你好");
        let got = align_chars_to_subtitles(&log, &[sub(1, "你好世界")]);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].chars, vec!['你', '好']);
        assert_eq!(got[0].times, vec![0.0, 0.1]);
    }

    /// 空字幕列表 / 空文本 ⇒ 空结果。
    #[test]
    fn empty_inputs_give_empty_output() {
        let log = log_from("你好");
        assert!(align_chars_to_subtitles(&log, &[]).is_empty());
        assert!(align_chars_to_subtitles(&log, &[sub(1, "")]).is_empty());
    }

    // ===== 候选表：路径反推与**顺序** =====

    /// ⚠️ **顺序判据**（Lead 点名要）：①②③ **同时存在**时，必须①（规范布局）赢。
    ///
    /// 顺序写错了在正常布局下**看不出来**（通常只有一个存在）—— 所以这里真造出三个都存在的场景。
    #[test]
    fn candidate_order_prefers_canonical_layout() {
        let root = std::env::temp_dir().join(format!("vtedit-charlog-order-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let clip = root.join("output").join("clips").join("clip-01");
        std::fs::create_dir_all(&clip).unwrap();

        let canonical = root.join("output").join("asr").join("timeline.json");
        let sibling = root.join("output").join("clips").join("asr").join("timeline.json");
        let inside = clip.join("asr").join("timeline.json");
        for (p, text) in [(&canonical, "规范"), (&sibling, "同级"), (&inside, "片段内")] {
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, serde_json::to_string(&log_from(text)).unwrap()).unwrap();
        }

        let picked = load_char_timeline(&clip).expect("三个都在，必须挑得出一个");
        assert_eq!(picked.text, "规范", "① 规范布局（上两级）必须赢");

        // 候选表顺序本身也钉住（防"边走边返回"式改写把顺序弄丢）
        let list = char_timeline_candidates(&clip);
        assert_eq!(list, vec![canonical.clone(), sibling.clone(), inside.clone()]);

        // ② 存在、① 不存在 ⇒ ②赢
        std::fs::remove_file(&canonical).unwrap();
        assert_eq!(load_char_timeline(&clip).unwrap().text, "同级");
        // 只有 ③ ⇒ ③赢
        std::fs::remove_file(&sibling).unwrap();
        assert_eq!(load_char_timeline(&clip).unwrap().text, "片段内");
        // 都没有 ⇒ 优雅地 None（**不是**报错）
        std::fs::remove_file(&inside).unwrap();
        assert!(load_char_timeline(&clip).is_none());

        let _ = std::fs::remove_dir_all(&root);
    }

    /// **缺文件 / 坏文件一律优雅**：不存在、非法 JSON、形状不合法都返回 `None`，**不 panic**。
    /// 这条就是"不许阻断 init/process"的机器判据（变异自证：把 `is_file()` 检查去掉、
    /// 或把 `Err(_) => continue` 改成 `unwrap`，这条会红/炸）。
    #[test]
    fn missing_or_broken_file_is_graceful() {
        let root = std::env::temp_dir().join(format!("vtedit-charlog-grace-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let clip = root.join("output").join("clips").join("clip-01");
        std::fs::create_dir_all(&clip).unwrap();

        // ① 一个都没有 —— 存量工程的绝大多数情况
        assert!(load_char_timeline(&clip).is_none());

        // ② 非法 JSON
        let p = clip.join("asr").join("timeline.json");
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, "{ 这不是 json").unwrap();
        assert!(load_char_timeline(&clip).is_none());

        // ③ 合法 JSON 但形状不合法（长度对不上）⇒ 当作没有
        let mut bad = log_from("你好");
        bad.times.clear();
        std::fs::write(&p, serde_json::to_string(&bad).unwrap()).unwrap();
        assert!(load_char_timeline(&clip).is_none());

        // ④ 未知字段（上游改了形状）⇒ 优雅跳过，不是 panic
        std::fs::write(&p, r#"{"text":"a","chars":1,"times":[0.0],"breath_gap":0.1,"breaths":[],"x":1}"#).unwrap();
        assert!(load_char_timeline(&clip).is_none());

        // ⑤ 一个目录占着那个名字（不是文件）也要跳过
        std::fs::remove_file(&p).unwrap();
        std::fs::create_dir_all(&p).unwrap();
        assert!(load_char_timeline(&clip).is_none());

        let _ = std::fs::remove_dir_all(&root);
    }

    /// 序列化回写：读进来、写出去、再读回来**逐值相同**（幂等，落 `data/` 后能自愈重读）。
    #[test]
    fn roundtrip_is_idempotent() {
        let log = log_from("你好世界");
        let json = serde_json::to_string_pretty(&log).unwrap();
        let back: CharTimelineLog = serde_json::from_str(&json).unwrap();
        assert_eq!(back.text, log.text);
        assert_eq!(back.chars, log.chars);
        assert_eq!(back.times, log.times);
        assert_eq!(back.chars_with_times(), log.chars_with_times());
    }
}
