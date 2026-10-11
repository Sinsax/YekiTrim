//! §11 BGM：引入背景音乐（**只做引入，不做卡点**）。
//!
//! 规格见 `docs/plan/editing-techniques-catalog.md` §3c 与总方案 §11。
//!
//! ⚠️ **为什么不做卡点（beat sync）**：卡点要"切点落在音乐强拍上"，
//! 而强拍要从**音乐的节拍网格**里来。本仓此前**没有 BGM** ⇒ **没有网格**。
//! 规格明写这是"**不适用项**，不要假装能做" —— 没有网格的"卡点"
//! 就是"随便找几个切点"，而那比不做更坏（观众能听出来不对拍）。
//!
//! ⚠️ **音轨归本仓**（红线 §3），所以 BGM 混音走本仓既有的 ffmpeg 链，
//! **不需要底座改一行**。

use std::path::{Path, PathBuf};

/// BGM 的**选曲**配置（住在 `polish.toml` 的 `[bgm]`）。
///
/// ⚠️ **默认 `enabled = false`** ⇒ 老工程零迁移（R4）。
/// 这也是"音轨全是直播原声"那个既有事实的显式化 —— 不写这个键 ⇔ 行为与今天逐字节相同。
///
/// ⚠️⚠️ **`deny_unknown_fields` 是必须的**（2026-10-11 与 `[elements]` 同批发现）：
/// 少了它，`volume` 拼成 `volumn` 会被 serde **静默丢掉**，而 `enabled = true` 照旧生效 ——
/// 表现是"开了 BGM、音量不是我写的那个"，**不报错、日志正常**。
/// 顶层 `ProjectFile` 本来就是 `deny_unknown_fields`，同段加它**不新增口径**。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BgmConfig {
    /// 总开关。**默认关** —— 不写这个键的老工程行为不变。
    #[serde(default)]
    pub enabled: bool,

    /// BGM 文件（相对工程目录或绝对路径；**走 `source` 那套路径口径**）。
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub file: String,

    /// BGM **自己的**基准音量（0.0~1.0）。默认 0.22。
    ///
    /// ⚠️ 这个数**不是随便定的**：直播原声是有内容的（说话/游戏声），
    /// 而 BGM 要"垫"在人声下面。实测口径与 `mix_sfx` 的"视频响度 − 8dB"同一族
    /// （那一条是音效的：点缀不盖人声）。BGM 比音效更靠后 ⇒ 再低 6dB 左右。
    #[serde(default = "default_bgm_volume")]
    pub volume: f64,

    /// 有人声时把 BGM 压到多低（0.0~1.0）。默认 0.35（= 压到基准的 35%）。
    ///
    /// ⚠️ **这是 BGM 能不能用的关键**（规格 §C：直播切片的人声是主角）：
    /// 不闪避的 BGM 会和主播抢话，"背景音乐"变成"盖住主播的音乐"。
    #[serde(default = "default_duck_ratio")]
    pub duck_ratio: f64,

    /// 淡入时长（秒）。默认 1.0 —— 硬切进来的音乐会**吓人一跳**。
    #[serde(default = "default_fade_in")]
    pub fade_in: f64,

    /// 淡出时长（秒）。默认 2.0 —— 收尾要比开头更缓，硬切掉会显得"没做完"。
    #[serde(default = "default_fade_out")]
    pub fade_out: f64,

    /// 循环模式：素材比片子短时是否循环铺满。
    ///
    /// ⚠️ 默认 **true**：切片通常比一首歌短，不循环的话后半段会**突然没音乐**
    /// （而那听起来像 bug，不像风格）。
    #[serde(default = "default_true")]
    pub loop_until_end: bool,
}

fn default_bgm_volume() -> f64 {
    0.22
}
fn default_duck_ratio() -> f64 {
    0.35
}
fn default_fade_in() -> f64 {
    1.0
}
fn default_fade_out() -> f64 {
    2.0
}
fn default_true() -> bool {
    true
}

impl Default for BgmConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            file: String::new(),
            volume: default_bgm_volume(),
            duck_ratio: default_duck_ratio(),
            fade_in: default_fade_in(),
            fade_out: default_fade_out(),
            loop_until_end: true,
        }
    }
}

impl BgmConfig {
    /// 空配置（与 `skip_serializing_if` 配套：空 ⇔ 不写这个键 ⇔ 老行为）。
    pub fn is_empty(&self) -> bool {
        !self.enabled && self.file.is_empty()
    }

    /// 配置是否**可用**（开了、且指了文件）。
    ///
    /// ⚠️ 与 `is_empty` **不是一件事**：`enabled = true` 但 `file` 空是
    /// **配错了**（用户开了开关、忘了选曲）—— 那种情况必须**报出来**，
    /// 不能静默当"没开"（R1：报告里一行、成片里没有）。
    pub fn is_usable(&self) -> bool {
        self.enabled && !self.file.trim().is_empty()
    }
}

/// 音乐库目录（`<工程>/bgm/`）里的候选曲目。
///
/// ⚠️ 与 `sfx/` 分开：音效是**点缀**（短、事件对齐），BGM 是**垫底**（长、整片铺）。
/// 混在一起会让人误以为"音效库里也能挑 BGM"，而两者的用法完全不同。
pub fn library(project_path: &Path) -> Vec<PathBuf> {
    let dir = project_path.join("bgm");
    let Ok(rd) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut out: Vec<PathBuf> = rd
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.is_file() && is_audio(p))
        .collect();
    // ⚠️ 排序：目录遍历顺序**不保证稳定**（不同文件系统不同），
    // 而不稳定的顺序会让"默认选第一个"变成不可复现的选择。
    out.sort();
    out
}

fn is_audio(p: &Path) -> bool {
    matches!(
        p.extension().and_then(|e| e.to_str()).map(|e| e.to_ascii_lowercase()).as_deref(),
        Some("mp3" | "m4a" | "aac" | "wav" | "flac" | "ogg" | "opus")
    )
}

/// 拼 BGM 那一级的 `-filter_complex` 片段。
///
/// 输入编号 `idx` 是 BGM 在 ffmpeg 命令行里的输入序号（音效之后）。
///
/// 链：`volume` → `afade in` → `afade out` → 打标签。
///
/// ⚠️ **闪避（ducking）不在这里做**：它要读**原声的响度**，
/// 而那是一条**并行**的输入。见 [`duck_gain_expr`]。
pub fn bgm_filter_chain(
    idx: usize,
    total_secs: f64,
    cfg: &BgmConfig,
) -> String {
    let vol = format!("volume={:.4}", cfg.volume.clamp(0.0, 1.0));
    let fade_in = cfg.fade_in.max(0.0);
    let fade_out = cfg.fade_out.max(0.0);
    let mut chain = format!("[{idx}:a]{vol}");
    if fade_in > 0.0 {
        chain.push_str(&format!(",afade=t=in:st=0:d={fade_in:.3}"));
    }
    if fade_out > 0.0 && total_secs > fade_out {
        chain.push_str(&format!(
            ",afade=t=out:st={:.3}:d={fade_out:.3}",
            total_secs - fade_out
        ));
    }
    chain.push_str("[bgm]");
    chain
}

/// BGM 的**闪避**增益表达式（规格 §C 的核心）。
///
/// # 为什么用 `volume` + 表达式，而不是 `sidechaincompress`
///
/// ⚠️ `sidechaincompress` 是 ffmpeg 里现成的闪避滤镜，但它要**两路输入**
/// 且**引入自己的包络释放时间**（`release`）—— 那个时间是"压缩器"语义，
/// 而我们要的是"有人声就压低"这个**判据本身**。用表达式可以**直接**把
/// "什么时候有人声"这件事写出来（我们**已经**有响度曲线了，见 §21），
/// 于是闪避与 §21 的音效闪避**共用同一份响度数据**，不会出现两套判据打架。
///
/// * `segments`：人声段 `(start, end, db)` —— 与 §21 的 `seg_loudness` 同一份；
/// * `cfg.duck_ratio`：有人声时压到基准的多少。
///
/// ⚠️ 表达式里的 `t` 是**输出时间轴**上的秒数。
pub fn duck_gain_expr(segments: &[(f64, f64, Option<f64>)], duck_ratio: f64) -> String {
    let ratio = duck_ratio.clamp(0.0, 1.0);
    if segments.is_empty() {
        return "1".to_string();
    }
    // `if(between(t,a,b), <有人声>, 1)` 的链：
    // 段很少（切片通常几段），链式表达式完全够用，且比查表更直观。
    //
    // ⚠️ **反向拼**：`if(between(...), ratio, 后面那些)` —— 最后兜底 1。
    //
    // ⚠️⚠️ **表达式里的逗号必须转义成 `\,`** —— 这是实测踩到的：
    //    `-filter_complex` 用**逗号分隔滤镜**，所以 `if(between(t,1,3),0.35,1)`
    //    里的逗号会被当成"滤镜分隔符"，ffmpeg 直接报
    //    `Error parsing a filter description`。
    //    ★ 而那次的**表现极具误导性**：`mix_bgm` 返回 `Err`，
    //    被调用方的"BGM 失败不该让整片失败"那条 `warn` 吞掉 ⇒
    //    **日志里没有 BGM 那行、也没有错误**，成片只是**没有音乐**。
    //    只有去量输出响度（源 −91dB、成片仍 −91dB）才看得出来。
    //    转义之后：−91dB → **−37.6dB**（音乐真的进去了）。
    let mut expr = "1".to_string();
    for (s, e, _) in segments.iter().rev() {
        if e <= s {
            continue;
        }
        expr = format!("if(between(t\\,{s:.3}\\,{e:.3})\\,{ratio:.3}\\,{expr})");
    }
    expr
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 默认是关的且不写键() {
        let c = BgmConfig::default();
        assert!(!c.enabled, "默认必须是关的 —— 否则老工程会突然多出背景音乐");
        assert!(c.is_empty(), "默认配置必须算空（老工程零迁移靠这个）");
        assert!(!c.is_usable(), "默认配置不可用");
    }

    #[test]
    fn 开了但没选曲要报出来而不是当没开() {
        // ⚠️ 这是"静默降级"的边界：用户明明开了开关，只是忘了选曲。
        // 把它当"没开"会让用户以为"BGM 功能坏了"，而报告里一行都没有。
        let c = BgmConfig { enabled: true, file: String::new(), ..Default::default() };
        assert!(!c.is_usable(), "没选曲时不可用");
        assert!(!c.is_empty(), "但它**不是空的** —— 调用方据此报'开了但没选曲'");
    }

    #[test]
    fn 空白的文件名不算选了曲() {
        let c = BgmConfig { enabled: true, file: "   ".into(), ..Default::default() };
        assert!(!c.is_usable(), "只有空格的文件名不该当'选了曲'");
    }

    #[test]
    fn 闪避表达式只压人声段() {
        let segs = vec![(1.0, 3.0, Some(-20.0)), (5.0, 7.0, Some(-18.0))];
        let e = duck_gain_expr(&segs, 0.35);
        // 两个人都声段都要出现。
        assert!(e.contains(r"between(t\,1.000\,3.000)"), "第一段没进表达式：{e}");
        assert!(e.contains(r"between(t\,5.000\,7.000)"), "第二段没进表达式：{e}");
        // 兜底必须是 1（没覆盖到的时间**不压**）——
        // 写成 0.35 的话整片 BGM 都是小声的，"闪避"退化成"全局调低"。
        // ⚠️ 判据不能写 `ends_with("1")`：表达式尾部是 `...),1))`，会连着右括号。
        // 第一版就是这么写错的 —— 报出来像"兜底坏了"，其实是断言太粗。
        // 正解：**最后那个逗号之后**的那一项必须是 1。
        let tail = e.rsplit(',').next().unwrap_or("");
        assert_eq!(
            tail.trim_end_matches(')'),
            "1",
            "兜底不是 1（会退化成全局调低）：{e}"
        );
        assert!(e.contains("0.350"), "闪避比例没进表达式：{e}");
    }

    /// ⚠️ **表达式里的逗号必须转义**（这条是实测踩出来的，而且**极难发现**）。
    ///
    /// `-filter_complex` 用**逗号分隔滤镜** ⇒ `if(between(t,1,3),0.35,1)` 里那些逗号
    /// 被当成滤镜分隔符，ffmpeg 报 `Error parsing a filter description`。
    /// ★ 那次的表现是：`mix_bgm` 返回 `Err` → 被调用方那条
    ///   "BGM 失败不该让整片失败"的 `warn` 吞掉 ⇒ **日志里既没有 BGM 那行、也没有错误**，
    ///   成片只是**没有音乐**。只有去量输出响度才发现
    ///   （源 −91dB、成片仍 −91dB；转义后 −37.6dB）。
    ///
    /// 判据：**裸逗号一个都不许有**（除了表达式最外层那两处结构逗号 —— 它们也在
    /// `if(...)` 内部，所以一个裸逗号都不该出现）。
    #[test]
    fn 闪避表达式里的逗号必须转义() {
        let segs = vec![(1.0, 3.0, None), (5.0, 7.0, None)];
        let e = duck_gain_expr(&segs, 0.35);
        // 逐字符扫：出现 `,` 且前一个字符不是 `\` ⇒ 会被 ffmpeg 当滤镜分隔符。
        for (i, c) in e.char_indices() {
            if c == ',' {
                let prev = e[..i].chars().last();
                assert_eq!(
                    prev,
                    Some('\\'),
                    "第 {i} 个字符是裸逗号（前一个字符是 {prev:?}）—— ffmpeg 会把它当滤镜分隔符，整条滤镜串解析失败：{e}"
                );
            }
        }
    }

    #[test]
    fn 没有人声段时不压() {
        assert_eq!(duck_gain_expr(&[], 0.35), "1");
    }

    #[test]
    fn 非法段被跳过() {
        // `e <= s` 的段（空段/倒置段）不许进表达式 —— 进了会得到一个恒假的
        // `between`，而"恒假"看着像"这里不压"，其实是配置坏了。
        let segs = vec![(3.0, 3.0, None), (5.0, 4.0, None), (1.0, 2.0, Some(-9.0))];
        let e = duck_gain_expr(&segs, 0.5);
        assert!(e.contains(r"between(t\,1.000\,2.000)"));
        assert!(!e.contains(r"between(t\,3.000\,3.000)"), "空段进了表达式：{e}");
        assert!(!e.contains(r"between(t\,5.000\,4.000)"), "倒置段进了表达式：{e}");
    }

    #[test]
    fn 闪避比例被夹在合法区间() {
        let segs = vec![(1.0, 2.0, None)];
        assert!(duck_gain_expr(&segs, 1.5).contains("1.000"), "超过 1 的比例没被夹住");
        assert!(duck_gain_expr(&segs, -0.5).contains("0.000"), "负比例没被夹住");
    }

    #[test]
    fn 滤波链包含淡入淡出() {
        let c = BgmConfig {
            enabled: true,
            file: "a.mp3".into(),
            volume: 0.2,
            fade_in: 1.0,
            fade_out: 2.0,
            ..Default::default()
        };
        let chain = bgm_filter_chain(3, 20.0, &c);
        assert!(chain.starts_with("[3:a]volume=0.2000"), "链头不对：{chain}");
        assert!(chain.contains("afade=t=in:st=0:d=1.000"), "缺淡入：{chain}");
        assert!(chain.contains("afade=t=out:st=18.000:d=2.000"), "淡出起点不对：{chain}");
        assert!(chain.ends_with("[bgm]"), "没打标签：{chain}");
    }

    #[test]
    fn 片子比淡出还短时不写淡出() {
        // 总长 1s、淡出 2s ⇒ 淡出起点是负数，写出来 ffmpeg 会报错。
        let c = BgmConfig { fade_out: 2.0, ..Default::default() };
        let chain = bgm_filter_chain(1, 1.0, &c);
        assert!(!chain.contains("afade=t=out"), "短片子不该写淡出：{chain}");
    }

    #[test]
    fn 音轨扩展名判定() {
        for n in ["a.mp3", "b.M4A", "c.wav", "d.flac", "e.ogg"] {
            assert!(is_audio(Path::new(n)), "{n} 应当算音频");
        }
        for n in ["a.mp4", "b.txt", "c", "d.jpg"] {
            assert!(!is_audio(Path::new(n)), "{n} 不该算音频");
        }
    }
}
