//! §5 E4a —— 文字元素动效（**只做本仓自足的那一档**）。
//!
//! 规格见 `docs/plan/elements-spec.md` §4 的 T1–T6 表。本模块负责
//! **T2 打字机**（typewriter：字逐个出现）。
//!
//! # ⚠️ 为什么只有 T2
//!
//! 规格 §3 的 2026-10-10 更正写着"T1/T2/T3/T4/T6 全部本仓自足"，但**那句话的本意是
//! "不必等 `entrance`/`highlight_color` 两个字段"**，不等于"底座已经有逐帧能力"。
//! 拿底座的真实渲染路径对过一遍之后（结论记在保守决策 #125–#128）：
//!
//! | 动效 | 需要什么 | 底座字幕轨现状 |
//! |---|---|---|
//! | **T2 打字机** | 逐帧改**显示几个字** | ✅ 本仓把一条 cue 按帧拆开即可（见下） |
//! | T1 弹入 | `font_ratio` **逐帧缩放** | ❌ 没有这个通道 |
//! | T3 强调放大 | 换色 + 放大 | ⚠️ 换色 ✅（`highlight_color` 已在用）；放大 ❌ |
//! | T4 抖动 | **持续**逐帧纵向偏移 | ❌ `rise_in_px` 是**一次性入场位移**，不是持续抖动 |
//! | T6 底衬 | 文字背后的色块 | ❌ `force_style` 只拼五个键，**没有 `BorderStyle`/`BackColour`** |
//!
//! 后四项已合并成一份底座需求（`docs/plan/dhampir-elements-request.md`），
//! **不硬凑**（用几十~上百条 cue 假装"缩放"，观感与代价都不成立）。
//!
//! # ⚠️ 为什么拆 cue、而不是 ASS 的 `\k` 覆盖标签
//!
//! 底座解析字幕时会把 `{...}` 覆盖标签**整块剥掉**（`subtitle.rs::ass_text_to_plain`，
//! 只留颜色与 `\move`）。⇒ `\k`（libass 原生卡拉OK）**根本到不了 libass**，
//! 那条路是死的。拆 cue 是唯一可行的：**不改底座一行**，且逐帧精确
//! （成片与预览读的是同一份 cue 表，不会分叉）。
//!
//! # ⚠️ 硬约束：逐帧计算，不许 CSS transition / 状态机
//!
//! 规格 §4.1 明写。否则 `seek` 到第 100 帧与"播到第 100 帧"长得不一样
//! ⇒ 预览与导出分叉。本模块的 [`split_typewriter`] 是**纯函数**
//! （输入 cue + 帧率 ⇒ 输出 cue 列表），无累积状态 —— 与 `handheld_sway` 同源。

use serde::{Deserialize, Serialize};

/// 文字元素动效的配置（住在 `polish.toml` 的 `[elements]`）。
///
/// ⚠️ **默认全关** ⇒ 老工程零迁移（R4：存量逐字节不变）。
/// 不写这个段 ⇔ 行为与今天逐字节相同 —— 与 §11 BGM 同一套做法。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ElementsConfig {
    /// 打字机总开关。**默认关**。
    #[serde(default)]
    pub typewriter: bool,

    /// 每秒打几个字。默认 **14**。
    ///
    /// ⚠️ 这个数不是随便定的：太慢（<8）会让 12 个字的标题打 1.5 秒，
    /// 观众已经看到字幕了、标题还在慢慢冒；太快（>25）与"直接出现"没区别，
    /// 白拆一堆 cue。14 字/秒 ≈ 一个字 71ms，与参照的"逐字出现"节奏接近。
    #[serde(default = "default_chars_per_sec")]
    pub chars_per_sec: f64,

    /// 打完之后的**静止尾巴**（秒）。默认 **0.6**。
    ///
    /// ⚠️ 必须留：打完了立刻消失，观众来不及读完整句。
    /// 它也是"这条 cue 总时长够不够打完"的判据输入。
    #[serde(default = "default_hold_secs")]
    pub hold_secs: f64,
}

fn default_chars_per_sec() -> f64 {
    14.0
}
fn default_hold_secs() -> f64 {
    0.6
}

impl Default for ElementsConfig {
    fn default() -> Self {
        Self {
            typewriter: false,
            chars_per_sec: default_chars_per_sec(),
            hold_secs: default_hold_secs(),
        }
    }
}

impl ElementsConfig {
    /// 空配置（与 `skip_serializing_if` 配套：空 ⇔ 不写这个键 ⇔ 老行为）。
    pub fn is_empty(&self) -> bool {
        !self.typewriter
    }

    /// 每秒字数**夹取到可用范围**。
    ///
    /// ⚠️ 配置是手写的 ⇒ 0 / 负数 / NaN / 无穷都进得来。0 会让下面的除法炸
    /// （`chars_per_sec` 当除数），而"因为一个手滑的数字整条出片跑不出来"
    /// 的代价远大于"这条用了默认速度"。
    pub fn safe_chars_per_sec(&self) -> f64 {
        if self.chars_per_sec.is_finite() && self.chars_per_sec > 0.0 {
            self.chars_per_sec
        } else {
            default_chars_per_sec()
        }
    }

    /// 静止尾巴**夹取**（同样挡 NaN / 负数）。
    pub fn safe_hold_secs(&self) -> f64 {
        if self.hold_secs.is_finite() && self.hold_secs >= 0.0 {
            self.hold_secs
        } else {
            default_hold_secs()
        }
    }
}

/// 一条文字元素 cue（**拆之前**的形状）。
///
/// ⚠️ 字段与转译器那边的 cue 对齐（`start`/`end` 是**帧号**、`text` 是整句）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TextCue {
    pub start: u32,
    pub end: u32,
    pub text: String,
}

/// 把一条 cue 拆成"逐字出现"的多条（**T2 打字机**）。
///
/// # 返回
///
/// `Vec<TextCue>`：第 i 条的 `text` 是原句的**前 i 个字**，时间区间首尾相接。
///
/// # 为什么不在这里做"打完之后的静止尾巴"
///
/// 尾巴 = **最后一条** cue 的时长比其它条长。这样"打到第 k 个字"这个状态
/// 与"整句显示完"共用同一条 cue（文本相同），少一条 cue、也少一次栅格化
/// （底座的栅格化按**文本**缓存，见 `cache_hits/cache_misses`）。
///
/// # 边界（保守）
///
/// - 开关关着 ⇒ 原样返回一条（**逐字段等于输入**，存量零回归）；
/// - 文本 ≤ 1 个字 ⇒ 原样一条（没有"逐个"可言，拆了反而多一次栅格化）；
/// - cue 时长不够打完 ⇒ **按实际时长压缩**（不延长 cue —— 延长会盖住下一条），
///   且**至少每字 1 帧**（0 帧的 cue 在底座那边是不可见的）；
/// - `end <= start` ⇒ 原样一条（脏输入不该让这里 panic 或产出空列表）。
pub fn split_typewriter(cue: &TextCue, fps: f64, cfg: &ElementsConfig) -> Vec<TextCue> {
    if !cfg.typewriter {
        return vec![cue.clone()];
    }
    let chars: Vec<char> = cue.text.chars().collect();
    // 没有"逐个"可言。
    if chars.len() <= 1 || cue.end <= cue.start || !fps.is_finite() || fps <= 0.0 {
        return vec![cue.clone()];
    }

    let span = (cue.end - cue.start) as f64;
    let per_char_secs = 1.0 / cfg.safe_chars_per_sec();

    // ⚠️ **每字至少 1 帧**：帧不够时宁可**少打几个字**、也不产出 0 帧的 cue
    //    （0 帧 = 底座那边一帧都不显示 = 那个字**永远不会出现**，
    //     而报告里看着"拆了 N 条"，是"报告与事实相反"的又一种形态）。
    let max_steps = span.floor().max(1.0) as u32;
    let wanted = (span / (per_char_secs * fps)).floor() as u32;
    // 打字阶段占多少帧（至少 1 帧、至多"每字 1 帧"）。
    let typing_frames = wanted.clamp(1, max_steps.saturating_sub(0).max(1));

    // 每字推进多少帧（浮点累积，避免整数除法把总时长吃掉）。
    let mut out: Vec<TextCue> = Vec::new();
    let mut shown = 0usize;
    let mut frame = cue.start;
    let hold_frames = (cfg.safe_hold_secs() * fps).round() as u32;

    // 打字阶段：逐个吐字。
    for i in 0..typing_frames {
        // 这一步应该显示到第几个字（按比例推进，保证最后一步刚好是整句）。
        let target = (((i + 1) as f64 / typing_frames as f64) * chars.len() as f64).ceil() as usize;
        let target = target.clamp(1, chars.len());
        let next_frame = cue.start + ((i + 1) as f64 * span / typing_frames as f64).round() as u32;
        let next_frame = next_frame.max(frame + 1).min(cue.end);
        out.push(TextCue {
            start: frame,
            end: next_frame,
            text: chars[..target].iter().collect(),
        });
        shown = target;
        frame = next_frame;
        if frame >= cue.end {
            break;
        }
    }

    // 静止尾巴：整句保持到 cue 结束。
    if shown < chars.len() || frame < cue.end {
        // ⚠️ 尾巴够长才单独占一条；不够长就并入最后一条（多一条 0 帧 cue 没意义）。
        let tail_start = cue.end.saturating_sub(hold_frames);
        if tail_start > frame {
            out.push(TextCue {
                start: frame,
                end: tail_start,
                text: chars[..shown.max(1)].iter().collect(),
            });
            frame = tail_start;
        }
        if frame < cue.end {
            let full = chars[..].iter().collect::<String>();
            // 与最后一条文本相同时**合并**（不产出内容等价的两条 cue）。
            match out.last_mut() {
                Some(last) if last.text == full && last.end == frame => last.end = cue.end,
                _ => out.push(TextCue {
                    start: frame,
                    end: cue.end,
                    text: full,
                }),
            }
        }
    }

    if out.is_empty() {
        return vec![cue.clone()];
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg_on() -> ElementsConfig {
        ElementsConfig {
            typewriter: true,
            ..Default::default()
        }
    }

    fn cue(start: u32, end: u32, text: &str) -> TextCue {
        TextCue {
            start,
            end,
            text: text.to_string(),
        }
    }

    /// 开关关着 ⇒ **逐字段等于输入**（R4：存量零回归靠这一条）。
    #[test]
    fn 关着的时候一条都不拆() {
        let c = cue(10, 100, "这句话有八个字");
        let out = split_typewriter(&c, 30.0, &ElementsConfig::default());
        assert_eq!(out, vec![c], "关着开关必须原样返回");
    }

    /// 文本只有 1 个字 ⇒ 不拆（没有"逐个"可言）。
    #[test]
    fn 一个字不拆() {
        let c = cue(0, 30, "好");
        let out = split_typewriter(&c, 30.0, &cfg_on());
        assert_eq!(out, vec![c]);
    }

    /// 拆出来的 cue **首尾相接、不重叠、不掉帧**。
    #[test]
    fn 拆出来的区间首尾相接() {
        let c = cue(0, 90, "一二三四五六");
        let out = split_typewriter(&c, 30.0, &cfg_on());
        assert!(out.len() > 1, "应当拆成多条，实得 {}", out.len());
        assert_eq!(out[0].start, c.start, "第一条必须从原起点开始");
        assert_eq!(out.last().unwrap().end, c.end, "最后一条必须到原终点");
        for w in out.windows(2) {
            assert_eq!(w[0].end, w[1].start, "相邻两条必须首尾相接（不留缝、不重叠）");
        }
    }

    /// 每条的文字都是原句的**前缀**，且**逐条变长**（这就是"打字机"）。
    #[test]
    fn 文字逐条变长且都是前缀() {
        let text = "一二三四五六";
        let c = cue(0, 120, text);
        let out = split_typewriter(&c, 30.0, &cfg_on());
        let mut prev_len = 0usize;
        for (i, c2) in out.iter().enumerate() {
            assert!(
                text.starts_with(&c2.text),
                "第 {i} 条「{}」不是原句前缀",
                c2.text
            );
            assert!(
                c2.text.chars().count() >= prev_len,
                "第 {i} 条比上一条短了 —— 那不是打字机"
            );
            prev_len = c2.text.chars().count();
        }
        assert_eq!(
            out.last().unwrap().text, text,
            "最后一条必须是**整句**（否则结尾少字）"
        );
    }

    /// ⚠️ **不许产出 0 帧的 cue**：0 帧在底座那边一帧都不显示 ⇒ 那个字永不出现。
    #[test]
    fn 不许有零帧的_cue() {
        // 极短的 cue（3 帧）+ 长文本 ⇒ 帧不够打完，也必须每字 ≥1 帧。
        let c = cue(0, 3, "一二三四五六七八");
        let out = split_typewriter(&c, 30.0, &cfg_on());
        for (i, c2) in out.iter().enumerate() {
            assert!(c2.end > c2.start, "第 {i} 条是 0 帧（{}..{}）", c2.start, c2.end);
        }
        assert_eq!(out.first().unwrap().start, 0);
        assert_eq!(out.last().unwrap().end, 3);
    }

    /// 脏输入（`end <= start`）⇒ 原样一条，不 panic、不产出空列表。
    #[test]
    fn 脏区间原样返回() {
        for c in [cue(10, 10, "一二三"), cue(10, 5, "一二三")] {
            let out = split_typewriter(&c, 30.0, &cfg_on());
            assert_eq!(out, vec![c.clone()], "脏区间必须原样返回");
        }
    }

    /// 帧率是脏值 ⇒ 原样一条（不 panic）。
    #[test]
    fn 脏帧率原样返回() {
        let c = cue(0, 60, "一二三");
        for fps in [0.0, -30.0, f64::NAN, f64::INFINITY] {
            let out = split_typewriter(&c, fps, &cfg_on());
            assert_eq!(out, vec![c.clone()], "fps={fps} 应当原样返回");
        }
    }

    /// ⚠️ **同一输入必须逐字段可复现**（纯函数、无累积状态）——
    /// 这是规格 §4.1 那条硬约束的机器判据：seek 到第 N 帧与播到第 N 帧必须一样。
    #[test]
    fn 纯函数可复现() {
        let c = cue(7, 133, "十二个字的钩子标题哦");
        let a = split_typewriter(&c, 30.0, &cfg_on());
        let b = split_typewriter(&c, 30.0, &cfg_on());
        assert_eq!(a, b, "同一输入两次调用必须逐字段相同");
        // 再来一次，确认没有跨调用的隐藏状态。
        let c2 = split_typewriter(&c, 30.0, &cfg_on());
        assert_eq!(a, c2);
    }

    /// 配置夹取：0/负数/NaN 的速度与尾巴都退回默认值（不许让除法炸）。
    #[test]
    fn 脏配置被夹取() {
        let bad = ElementsConfig {
            typewriter: true,
            chars_per_sec: 0.0,
            hold_secs: f64::NAN,
        };
        assert_eq!(bad.safe_chars_per_sec(), default_chars_per_sec());
        assert_eq!(bad.safe_hold_secs(), default_hold_secs());
        // 负数也要挡。
        let neg = ElementsConfig {
            typewriter: true,
            chars_per_sec: -5.0,
            hold_secs: -1.0,
        };
        assert_eq!(neg.safe_chars_per_sec(), default_chars_per_sec());
        assert_eq!(neg.safe_hold_secs(), default_hold_secs());
        // 正常值原样通过。
        let ok = ElementsConfig {
            typewriter: true,
            chars_per_sec: 20.0,
            hold_secs: 1.2,
        };
        assert_eq!(ok.safe_chars_per_sec(), 20.0);
        assert_eq!(ok.safe_hold_secs(), 1.2);
    }

    /// 空配置的判据（与 `skip_serializing_if` 配套）。
    #[test]
    fn 空配置只在关着时成立() {
        assert!(ElementsConfig::default().is_empty(), "默认必须算空（不写进 toml）");
        assert!(
            !ElementsConfig {
                typewriter: true,
                ..Default::default()
            }
            .is_empty(),
            "开了开关就不算空 —— 否则这个键不会被写下来，下次跑就丢失了"
        );
    }
}
