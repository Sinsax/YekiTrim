//! 「去静音」= 静音区间 → 屏蔽轨道（mute_ranges）。
//!
//! 历史实现会调用 ffmpeg 重编码出一份 `*_cleaned.mp4` 并把 `source.video` 切过去；
//! 现在改为：检测到的长静音段写进 `polish.toml` 的 `mute_ranges`（时间轴「屏蔽」轨道），
//! 由 process / render 在导出阶段剪掉，原始 `clip.mp4` / `clip.srt` 完全不动。
//!
//! **与当前屏蔽状态是「相加」**（用户要求）：应用 = `屏蔽轨道 ∪ 检测结果`（重叠/相邻的合成一段，
//! 不会在已有屏蔽上叠出重复红块）；撤销 = `屏蔽轨道 − 当初自动加进去的那些区间`（差集，
//! 手动添加的部分原样留下）。`silence_ranges` 记录**自动加进去的那部分**，撤销按它精确相减。

use std::path::{Path, PathBuf};

use super::{load_project, MuteRange, ProjectFile};

/// 区间比较容差（秒）：检测结果可复现，留一点余量防止浮点误差。
const RANGE_EPS: f64 = 0.01;

/// 把静音区间并入 `mute_ranges` 后的结果。
#[derive(Debug, Clone, Default)]
pub struct SilenceApply {
    /// 本次检测到的静音区间（= 实际会加进屏蔽轨道的那部分，**已剔除被保护的**）
    pub ranges: Vec<(f64, f64)>,
    /// 真正新加入屏蔽轨道的区间数（检测结果减掉当前屏蔽状态之后剩下的）
    pub added: usize,
    /// 应用后屏蔽轨道总区间数
    pub total: usize,
    /// §7：被**保护**下来（判为"喜剧停顿"）的区间 —— 本来会剪、这次不剪的那些。
    ///
    /// ⚠️ 报出来是**必须的**：不算的话用户会看到"点了去静音，有一段没被剪"，
    /// 而界面上没有任何解释 —— 那看上去就是**功能没生效**。
    pub protected: Vec<(f64, f64)>,
}

// ===== 静默分两类（总方案 §7）=====

/// 一段静默的**性质**。
///
/// 规格 §3.4 的判据表（外部调研的核心发现，两个独立来源 + HotClip 都指向同一条）：
///
/// > **爆点之后的"短沉默"是笑点发酵的地方，必须保护；该剪的是"拖沓的沉默"。**
///
/// ⚠️ **这不是"更聪明地剪"，是"不要剪错"**：今天 `DEFAULT_MIN_GAP_SECS = 3.5s`
/// 一刀切，于是一个 4 秒的包袱停顿被剪成 2.4 秒 —— **正好砍掉喜剧节拍**，
/// 而且这是**自动**发生的（用户点一下「应用」就触发）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SilenceKind {
    /// **拖沓沉默** —— 前后没有爆点信号 ⇒ 照旧剪。
    Dragging,
    /// **喜剧停顿** —— 紧跟在爆点之后 ⇒ 保护（不剪，或只留一个呼吸口）。
    Comic,
}

impl SilenceKind {
    pub fn key(self) -> &'static str {
        match self {
            Self::Dragging => "dragging",
            Self::Comic => "comic",
        }
    }
}

/// 爆点之后**多久之内**的静默算"喜剧停顿"（秒）。
///
/// ⚠️ 取 4.0 而不是"整段只要在爆点后面就算"：一个爆点在 10s、静默在 60s，
/// 那不是"笑点发酵"，那是**真的拖沓**。窗口的意义就是把"紧跟"量化出来。
///
/// 规格说"紧跟在爆点之后"，没给秒数 —— 这个值按**本仓实测口径**取：
/// 真实切片里中位字幕时长 2.64s，4s ≈ 一句半的话；再长就该是拖沓了。
pub const COMIC_AFTER_PUNCH_SECS: f64 = 4.0;

/// 判一段静默是不是"喜剧停顿"。
///
/// - `start` / `end`：候选静默区间。
/// - `punches`：爆点区间（来自 §6 的 `punch::detect`）。**空 = 一个爆点都没有** ⇒ 全部按拖沓处理
///   （与今天行为一致，**不倒退**）。
///
/// 判据：静默区间的**起点**落在某个爆点区间**结束之后** `COMIC_AFTER_PUNCH_SECS` 之内。
///
/// ⚠️ 用**起点**而不是"与爆点有重叠"：静默**本身**就与爆点不重叠（爆点里有人在说话/有声音，
/// 那不叫静默）。要判的是"它是不是**紧接在**爆点后面"，所以锚在起点上。
///
/// ⚠️ 爆点**内部**或**之前**的静默一律不是喜剧停顿：
/// 爆点前面那一段是**铺垫**，剪掉它反而让包袱来得更快（那是剪辑技巧，不是破坏）。
pub fn classify_silence(
    start: f64,
    end: f64,
    punches: &[crate::process::punch::Punch],
) -> SilenceKind {
    if end <= start {
        return SilenceKind::Dragging;
    }
    let is_comic = punches.iter().any(|p| {
        // 起点必须**在爆点结束之后**（含容差），且在保护窗之内
        start >= p.end - RANGE_EPS && start <= p.end + COMIC_AFTER_PUNCH_SECS
    });
    if is_comic {
        SilenceKind::Comic
    } else {
        SilenceKind::Dragging
    }
}

/// 把检测出的静默区间**按两类分开**（§7）。返回 `(该剪的, 该保护的)`，各自保持原顺序。
///
/// ⚠️ 两个列表**加起来等于输入**（没有区间被"丢掉"）—— 这条是判据：
/// 漏掉一段的症状是"那一段既没剪也没记录"，而用户无从知道它去哪了。
pub fn split_silence_by_kind(
    ranges: &[(f64, f64)],
    punches: &[crate::process::punch::Punch],
) -> (Vec<(f64, f64)>, Vec<(f64, f64)>) {
    let mut dragging = Vec::new();
    let mut comic = Vec::new();
    for (s, e) in ranges {
        match classify_silence(*s, *e, punches) {
            SilenceKind::Dragging => dragging.push((*s, *e)),
            SilenceKind::Comic => comic.push((*s, *e)),
        }
    }
    (dragging, comic)
}

/// 区间并集：重叠或相邻（`RANGE_EPS` 内）的合成一段，按 start 排序。
///
/// 「去静音」要**和当前屏蔽轨道相加**，所以这里是并集而不是追加 ——
/// 追加会把 16.5s 的静音盖在手动 10~20s 的屏蔽上，轨道里出现两块重叠红块。
pub fn union_ranges(a: &[(f64, f64)], b: &[(f64, f64)]) -> Vec<(f64, f64)> {
    let mut all: Vec<(f64, f64)> = a
        .iter()
        .chain(b.iter())
        .copied()
        .filter(|(s, e)| e > s)
        .collect();
    all.sort_by(|x, y| x.0.partial_cmp(&y.0).unwrap_or(std::cmp::Ordering::Equal));
    let mut out: Vec<(f64, f64)> = Vec::new();
    for (s, e) in all {
        match out.last_mut() {
            Some(last) if s <= last.1 + RANGE_EPS => {
                if e > last.1 {
                    last.1 = e;
                }
            }
            _ => out.push((s, e)),
        }
    }
    out
}

/// 区间差集 `base − cut`：`cut` 覆盖到的部分从 `base` 里挖掉（一段可能变成两段）。
pub fn subtract_ranges(base: &[(f64, f64)], cut: &[(f64, f64)]) -> Vec<(f64, f64)> {
    let cuts = union_ranges(cut, &[]);
    let mut out: Vec<(f64, f64)> = Vec::new();
    for (s, e) in base.iter().copied() {
        if e <= s {
            continue;
        }
        let mut pieces = vec![(s, e)];
        for (cs, ce) in &cuts {
            if *ce <= s || *cs >= e {
                continue; // 这一刀跟本段不相交（对每段都成立时整段原样留下）
            }
            let mut next: Vec<(f64, f64)> = Vec::new();
            for (ps, pe) in pieces {
                if *ce <= ps || *cs >= pe {
                    next.push((ps, pe));
                    continue;
                }
                if ps < *cs {
                    next.push((ps, cs.min(pe)));
                }
                if *ce < pe {
                    next.push((ce.max(ps), pe));
                }
            }
            pieces = next;
        }
        for (ps, pe) in pieces {
            if pe - ps > RANGE_EPS {
                out.push((ps, pe));
            }
        }
    }
    out.sort_by(|x, y| x.0.partial_cmp(&y.0).unwrap_or(std::cmp::Ordering::Equal));
    out
}

fn pairs(ranges: &[MuteRange]) -> Vec<(f64, f64)> {
    ranges.iter().filter_map(|r| r.normalized()).collect()
}

fn to_mute_ranges(pairs: &[(f64, f64)]) -> Vec<MuteRange> {
    pairs.iter().map(|(s, e)| MuteRange { start: *s, end: *e }).collect()
}

/// 检测该精修工程对应片段的静音区间（不写盘）。
///
/// `allow_audio_fallback=false` 时只按字幕间隔检测（列表接口用，避免逐条解码视频）。
pub fn detect_ranges(project_path: &Path, allow_audio_fallback: bool) -> anyhow::Result<Vec<(f64, f64)>> {
    detect_ranges_with(
        project_path,
        yekitrim_core::cutting::DEFAULT_MIN_GAP_SECS,
        yekitrim_core::cutting::DEFAULT_NOISE_DB,
        allow_audio_fallback,
    )
}

/// 带阈值的静音检测：`min_gap` 为长静音阈值（秒），`noise_db` 仅在音频分析时生效。
///
/// 结果两侧各留 `SILENCE_PAD_SECS` 的气口（用户要求：贴着字幕边界剪会把首尾音节削掉）。
pub fn detect_ranges_with(
    project_path: &Path,
    min_gap: f64,
    noise_db: f64,
    allow_audio_fallback: bool,
) -> anyhow::Result<Vec<(f64, f64)>> {
    let cfg = load_project(project_path)?;
    let video = resolve(project_path, &cfg.source.video);
    let srt = resolve(project_path, &cfg.source.subtitles);
    let srt = if srt.exists() { Some(srt) } else { None };
    if srt.is_none() && !allow_audio_fallback {
        return Ok(Vec::new());
    }
    if srt.is_none() && !video.exists() {
        anyhow::bail!("缺少视频/字幕文件，无法检测静音");
    }
    let ranges = yekitrim_core::cutting::detect_silence_ranges(&video, srt.as_deref(), min_gap, noise_db)?;
    Ok(pad_ranges(ranges))
}

/// 「去静音」在每段静音两侧留的气口（秒）。
///
/// 字幕时间戳本身有 ±0.1~0.2s 的误差，贴着边界剪会把一个字的起音/收音削掉，听起来很突兀；
/// 往内收一点，剪完两头各剩一口气，听感自然（用户要求"和相邻字幕之间留一点气口"）。
/// 0.35 → 0.8（用户要求加大）：呼吸感明显了，代价是**有效阈值 = `min_gap` + 2×0.8s** ——
/// 默认 2.0s 阈值下，2.0~3.6s 的短停顿只会被剪掉一点点；把 `--duration` 调到 1.6s 以下
/// 这些短区间会被收没（`pad_ranges` 直接丢掉），那是预期的边界行为。
pub const SILENCE_PAD_SECS: f64 = 0.8;

/// 静音区间两侧各收 `SILENCE_PAD_SECS`；收没了（区间太短）就整段丢掉。
fn pad_ranges(ranges: Vec<(f64, f64)>) -> Vec<(f64, f64)> {
    ranges
        .into_iter()
        .filter_map(|(start, end)| {
            let s = (start + SILENCE_PAD_SECS).max(0.0);
            let e = (end - SILENCE_PAD_SECS).max(0.0);
            if e - s > 0.05 { Some((s, e)) } else { None }
        })
        .collect()
}

/// 应用消音版：静音区间**并入**当前屏蔽轨道（含 `silence_ranges` 记录）。
pub fn apply_silence(project_path: &Path) -> anyhow::Result<SilenceApply> {
    apply_silence_with(
        project_path,
        yekitrim_core::cutting::DEFAULT_MIN_GAP_SECS,
        yekitrim_core::cutting::DEFAULT_NOISE_DB,
    )
}

/// 带阈值的消音版应用。
///
/// `屏蔽轨道 ← 屏蔽轨道 ∪ 检测结果`，重叠/相邻的合成一段（与当前屏蔽状态**相加**，不是替换也不是叠加）；
/// `silence_ranges` 只记**真正自动加进去的那部分**（检测结果减掉本来就有的屏蔽），
/// 这样撤销时相减不会有副作用：本来就手动屏掉的地方不会被撤销挖出一个洞。
pub fn apply_silence_with(project_path: &Path, min_gap: f64, noise_db: f64) -> anyhow::Result<SilenceApply> {
    let mut cfg = load_project(project_path)?;
    let all_detected = detect_ranges_with(project_path, min_gap, noise_db, true)?;

    // §7 静默分两类：把"紧跟在爆点之后"的那些**保护**下来（不剪）。
    //
    // ⚠️ 爆点由 §6 的**同一份判据**算出来（`process::punch`），不在这里另写一套 ——
    //    两套的话"§7 保护的那段停顿"与"§8 当爆点的那一段"可能不是同一个爆点，
    //    而两边单独看都说得通。
    let punches = detect_punches(project_path, &cfg);
    let (detected, protected) = if punches.is_empty() {
        // ⚠️ 一个爆点都没有 ⇒ **不加任何保护**（与今天行为逐字节相同，不倒退）。
        //    这里显式写出来是为了让"没爆点 ≠ 全保护"成为代码里可见的事实 ——
        //    反过来写会让无弹幕切片的去静音**整个失效**。
        (all_detected, Vec::new())
    } else {
        split_silence_by_kind(&all_detected, &punches)
    };
    if !protected.is_empty() {
        tracing::info!(
            "  · 去静音：保护 {} 段紧跟在爆点之后的停顿（§7 喜剧节拍），剪掉 {} 段拖沓静默",
            protected.len(),
            detected.len()
        );
    }

    let before = pairs(&cfg.mute_ranges);
    let prev = pairs(&cfg.silence_ranges); // 上次「去静音」加进去的，重复点击时别把撤销依据丢掉
    let fresh = subtract_ranges(&detected, &before);
    let merged = union_ranges(&before, &detected);

    cfg.mute_ranges = to_mute_ranges(&merged);
    cfg.silence_ranges = to_mute_ranges(&union_ranges(&fresh, &prev));
    save_project(project_path, &cfg)?;
    Ok(SilenceApply {
        added: fresh.len(),
        total: merged.len(),
        protected,
        ranges: detected,
    })
}

/// 算这一趟的爆点（§6 判据），供 §7 与 §8 共用。
///
/// ⚠️ **失败一律回空**（而不是往外抛）：去静音是用户点一下就该生效的操作，
/// 而爆点判据只是一层**优化**（判不出来时行为与今天相同）。为了判据读不到字幕而
/// 让"去静音"整个失败，是把优化做成了前置条件 —— 那是本末倒置。
fn detect_punches(
    project_path: &Path,
    cfg: &ProjectFile,
) -> Vec<crate::process::punch::Punch> {
    use crate::process::punch as P;

    let Ok(raw) = std::fs::read_to_string(project_path.join("data").join("subtitles.json"))
    else {
        return Vec::new();
    };
    let Ok(subs) = serde_json::from_str::<Vec<crate::parse::srt::Subtitle>>(&raw) else {
        return Vec::new();
    };
    if subs.is_empty() {
        return Vec::new();
    }

    let window = P::PUNCH_WINDOW_SECS;
    let duration = cfg
        .segments
        .iter()
        .map(|s| s.end)
        .fold(0.0_f64, f64::max)
        .max(subs.iter().map(|s| s.end).fold(0.0_f64, f64::max));

    // 弹幕：与 `process` Step 1f 同一口径（起点归窗、缺文件 ⇒ 空 = 不参与归一化）
    let danmaku = read_danmaku_counts(project_path, window);
    let loudness = crate::render::read_loudness_curve_checked(
        project_path,
        &project_path.join("clip.mp4"),
    );

    let detector = P::PunchDetector {
        subtitle_points: P::subtitle_signal(&subs, window),
        danmaku_points: P::danmaku_signal(&danmaku, window),
        sfx_points: P::loudness_signal(&loudness),
    };
    detector.detect(&cfg.segments, duration, window)
}

/// 每个 `window` 秒窗口的弹幕条数（§7 的爆点判据用；与 `process::read_danmaku_counts` 同口径）。
///
/// ⚠️ 这里**必须**与 `process/mod.rs` 那份口径一致（起点归窗 + 缺文件回空）。
/// 两份漂移的症状是"process 报的爆点"与"去静音保护的停顿"对不上，而两边都"有输出"。
/// 真正的收敛办法是把它挪进 `punch.rs`（那才是"一份判据"的落点）——
/// 本趟先按保守方案就地复制并**在两边都写清这条约束**（见保守决策记录）。
fn read_danmaku_counts(project_path: &Path, window: f64) -> Vec<(f64, f64)> {
    let path = project_path.join("data").join("danmaku.json");
    let Ok(raw) = std::fs::read_to_string(&path) else { return Vec::new() };
    #[derive(serde::Deserialize)]
    struct Row {
        #[serde(default)]
        start: f64,
    }
    let Ok(rows) = serde_json::from_str::<Vec<Row>>(&raw) else { return Vec::new() };
    if rows.is_empty() || window <= 0.0 {
        return Vec::new();
    }
    let mut buckets: std::collections::BTreeMap<i64, f64> = Default::default();
    for r in &rows {
        let idx = (r.start / window).floor() as i64;
        *buckets.entry(idx).or_insert(0.0) += 1.0;
    }
    buckets.into_iter().map(|(i, c)| (i as f64 * window, c)).collect()
}

/// 取消消音版：从屏蔽轨道里**减掉** `silence_ranges` 记录的区间，手动屏蔽区间保持不变。
///
/// 返回被移除的自动区间数（叠在手动屏蔽上的那些也算 —— 修掉的是这一小段）。
pub fn clear_silence(project_path: &Path) -> anyhow::Result<usize> {
    let mut cfg = load_project(project_path)?;
    if cfg.silence_ranges.is_empty() {
        return Ok(0);
    }
    let recorded = std::mem::take(&mut cfg.silence_ranges);
    let before = pairs(&cfg.mute_ranges);
    let removed = recorded
        .iter()
        .filter_map(|r| r.normalized())
        .filter(|(s, e)| before.iter().any(|(bs, be)| bs < e && s < be))
        .count();
    cfg.mute_ranges = to_mute_ranges(&subtract_ranges(&before, &pairs(&recorded)));
    save_project(project_path, &cfg)?;
    Ok(removed)
}

/// 该精修工程当前是否处于消音版（含静音区间记录）。
pub fn is_silent(project_path: &Path) -> bool {
    if !project_path.join("polish.toml").exists() {
        return false;
    }
    load_project(project_path)
        .map(|cfg| !cfg.silence_ranges.is_empty())
        .unwrap_or(false)
}

/// 屏蔽轨道区间数（列表展示用）。
pub fn mute_range_count(project_path: &Path) -> usize {
    load_project(project_path)
        .map(|cfg| cfg.mute_ranges.len())
        .unwrap_or(0)
}

fn resolve(project_path: &Path, value: &str) -> PathBuf {
    let p = Path::new(value);
    if p.is_absolute() { p.to_path_buf() } else { project_path.join(p) }
}

fn save_project(project_path: &Path, cfg: &ProjectFile) -> anyhow::Result<()> {
    let raw = cfg.to_toml_commented()?;
    super::write_atomic(&project_path.join("polish.toml"), &raw)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 临时精修工程：polish.toml（`example()` 起手）+ 一条带长间隔的 srt。
    /// 静音检测优先按字幕间隔（不需要 ffmpeg / 视频文件）。
    struct TmpProject(PathBuf);

    impl TmpProject {
        fn new(tag: &str, srt: &str, mutes: &[(f64, f64)]) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "yekitrim-silence-{tag}-{}-{:?}",
                std::process::id(),
                std::thread::current().id()
            ));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("clip.srt"), srt).unwrap();
            let mut cfg = ProjectFile::example();
            cfg.mute_ranges = mutes.iter().map(|(s, e)| MuteRange { start: *s, end: *e }).collect();
            std::fs::write(dir.join("polish.toml"), cfg.to_toml_commented().unwrap()).unwrap();
            TmpProject(dir)
        }

        fn load(&self) -> ProjectFile {
            load_project(&self.0).unwrap()
        }

        fn mutes(&self) -> Vec<(f64, f64)> {
            pairs(&self.load().mute_ranges)
        }

        fn recorded(&self) -> Vec<(f64, f64)> {
            pairs(&self.load().silence_ranges)
        }
    }

    impl Drop for TmpProject {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// 两条字幕之间留 10s 空档（0-2s 说话、12-14s 说话 → 静音 2.0~12.0）
    const SRT_WITH_GAP: &str = "1\n00:00:00,000 --> 00:00:02,000\n甲\n\n2\n00:00:12,000 --> 00:00:14,000\n乙\n";

    #[test]
    fn union_merges_overlaps_and_adjacent() {
        // 重叠 → 一段；相邻（容差内）→ 一段；分开 → 两段
        assert_eq!(union_ranges(&[(0.0, 10.0)], &[(5.0, 15.0)]), vec![(0.0, 15.0)]);
        assert_eq!(union_ranges(&[(0.0, 10.0)], &[(10.005, 15.0)]), vec![(0.0, 15.0)]);
        assert_eq!(union_ranges(&[(0.0, 10.0)], &[(11.0, 15.0)]), vec![(0.0, 10.0), (11.0, 15.0)]);
        // 完全包住 + 乱序输入
        assert_eq!(union_ranges(&[(20.0, 30.0), (0.0, 8.0)], &[(1.0, 25.0)]), vec![(0.0, 30.0)]);
    }

    #[test]
    fn subtract_cuts_holes() {
        assert_eq!(subtract_ranges(&[(0.0, 10.0)], &[(2.0, 4.0)]), vec![(0.0, 2.0), (4.0, 10.0)]);
        assert_eq!(subtract_ranges(&[(0.0, 10.0)], &[(9.0, 20.0)]), vec![(0.0, 9.0)]);
        assert_eq!(subtract_ranges(&[(0.0, 10.0)], &[(0.0, 10.0)]), Vec::<(f64, f64)>::new());
        assert_eq!(subtract_ranges(&[(0.0, 10.0)], &[]), vec![(0.0, 10.0)]);
    }

    /// 默认阈值 3.5s（用户要求：字幕间隔 3.5s 以内一律不处理），气口仍是 0.8s。
    const SRT_SHORT_GAP: &str = "1\n00:00:00,000 --> 00:00:02,000\n甲\n\n2\n00:00:05,000 --> 00:00:07,000\n乙\n";

    #[test]
    fn detect_leaves_breathing_room_next_to_subtitles() {
        // 字幕间隔 2.0~12.0 → 两侧各留 0.8s 气口（用户要求：贴边剪会削掉首尾音节）
        let p = TmpProject::new("pad", SRT_WITH_GAP, &[]);
        assert_eq!(detect_ranges(&p.0, true).unwrap(), vec![(2.8, 11.2)]);
        // 收没了就整段丢掉：把阈值调到比间隔还长 → 检测不到
        assert!(detect_ranges_with(&p.0, 20.0, -30.0, true).unwrap().is_empty());
        // 气口大于半个区间时该段直接消失（不会产生空/倒置区间）
        assert!(pad_ranges(vec![(2.0, 2.3)]).is_empty());
    }

    #[test]
    fn short_subtitle_gap_is_left_alone() {
        // 间隔 3.0s < 3.5s 默认阈值 → 不产生任何屏蔽区间（用户要求：3.5s 内不处理）
        let p = TmpProject::new("short", SRT_SHORT_GAP, &[]);
        assert!(
            detect_ranges(&p.0, true).unwrap().is_empty(),
            "3.0s 的字幕间隔不该被剪：{:?}",
            detect_ranges(&p.0, true)
        );
        // 阈值显式调小到 2.0 才剪，而且气口 0.8s 之后只剩 1.4s
        assert_eq!(detect_ranges_with(&p.0, 2.0, -30.0, true).unwrap(), vec![(2.8, 4.2)]);
    }

    #[test]
    fn apply_adds_to_current_mute_track_and_undo_restores_it() {
        let p = TmpProject::new("sum", SRT_WITH_GAP, &[(30.0, 40.0)]);
        let res = apply_silence_with(&p.0, 2.0, -30.0).unwrap();
        assert_eq!(res.ranges, vec![(2.8, 11.2)], "检测结果（带气口）");
        assert_eq!(res.added, 1, "新增区间数");
        assert_eq!(res.total, 2, "总段数 = 手动 1 + 新增 1");
        // 手动屏蔽原样留着，检测出的并进来（相加，不是替换）
        assert_eq!(p.mutes(), vec![(2.8, 11.2), (30.0, 40.0)]);
        assert_eq!(p.recorded(), vec![(2.8, 11.2)], "只记自动加进去的");
        assert!(is_silent(&p.0));

        let removed = clear_silence(&p.0).unwrap();
        assert_eq!(removed, 1);
        assert_eq!(p.mutes(), vec![(30.0, 40.0)], "撤销后只剩手动屏蔽");
        assert!(p.recorded().is_empty());
        assert!(!is_silent(&p.0));
    }

    #[test]
    fn apply_coalesces_when_detection_lands_on_existing_mute() {
        // 手动已把整段静音屏掉：相加后仍是一段，不该叠出第二块红块；
        // 也没"新加"任何东西 → 撤销不该在手动块上挖洞（用户要求：和当前屏蔽状态相加）
        let p = TmpProject::new("coalesce", SRT_WITH_GAP, &[(0.0, 15.0)]);
        let res = apply_silence_with(&p.0, 2.0, -30.0).unwrap();
        assert_eq!(res.added, 0);
        assert_eq!(res.total, 1);
        assert_eq!(p.mutes(), vec![(0.0, 15.0)]);
        assert!(p.recorded().is_empty(), "没有新加的东西就不用记撤销依据");

        assert_eq!(clear_silence(&p.0).unwrap(), 0);
        assert_eq!(p.mutes(), vec![(0.0, 15.0)], "撤销不能把手动屏蔽挖出洞");
    }

    #[test]
    fn apply_twice_keeps_undo_record_and_is_idempotent() {
        let p = TmpProject::new("twice", SRT_WITH_GAP, &[(30.0, 40.0)]);
        apply_silence_with(&p.0, 2.0, -30.0).unwrap();
        apply_silence_with(&p.0, 2.0, -30.0).unwrap();
        assert_eq!(p.mutes(), vec![(2.8, 11.2), (30.0, 40.0)], "重复应用不叠加");
        assert_eq!(p.recorded(), vec![(2.8, 11.2)], "撤销依据不丢");
        clear_silence(&p.0).unwrap();
        assert_eq!(p.mutes(), vec![(30.0, 40.0)]);
    }
    // ── §7 静默分两类 ──────────────────────────────────────────────

    fn punch(at: f64, end: f64) -> crate::process::punch::Punch {
        crate::process::punch::Punch { at, end, score: 0.8 }
    }

    /// ⚠️ **核心判据**：爆点**之后** 4s 内的静默 = 喜剧停顿（保护）；其余 = 拖沓（剪）。
    ///
    /// 这条直接对应规格 §3.4 的那句"该剪的是拖沓的沉默"——
    /// 判反了的症状是**"该保护的被剪了、该剪的留下了"**，而两边都不报错。
    #[test]
    fn silence_right_after_a_punch_is_protected() {
        let punches = vec![punch(10.0, 15.0)];
        // 紧接爆点之后（15.0 → 19.0，起点正好在爆点结束处）
        assert_eq!(classify_silence(15.0, 19.0, &punches), SilenceKind::Comic);
        assert_eq!(classify_silence(17.5, 21.0, &punches), SilenceKind::Comic, "保护窗内也算");
        // 超出保护窗（15 + 4 = 19 之后）
        assert_eq!(
            classify_silence(19.5, 23.0, &punches),
            SilenceKind::Dragging,
            "离爆点太远 = 真拖沓，该剪"
        );
        // 爆点**之前**（铺垫期）
        assert_eq!(
            classify_silence(2.0, 9.0, &punches),
            SilenceKind::Dragging,
            "爆点前是铺垫，剪掉它让包袱来得更快 —— 不是破坏"
        );
    }

    /// **没有爆点 ⇒ 全部按拖沓处理**（= 与今天行为一致，不倒退）。
    ///
    /// ⚠️ 这条防的是"判据反过来用"：如果没爆点时就**保护一切**，
    /// 那么在没有弹幕、字幕也判不出爆点的切片上，**去静音会整个失效** ——
    /// 用户点了按钮却什么都没发生，而日志里一句都没有。
    #[test]
    fn no_punches_means_everything_is_dragging() {
        let ranges = vec![(5.0, 12.0), (30.0, 40.0)];
        let (dragging, comic) = split_silence_by_kind(&ranges, &[]);
        assert_eq!(dragging, ranges, "没有爆点时应全部照旧剪");
        assert!(comic.is_empty(), "没有爆点时不该保护任何一段");
    }

    /// ⚠️ **两类的并集必须等于输入**（没有区间会"凭空消失"）。
    ///
    /// 漏掉一段的症状是"那段既没剪也没记录" —— 用户无从知道它去哪了。
    #[test]
    fn every_detected_range_lands_in_exactly_one_class() {
        let punches = vec![punch(10.0, 15.0), punch(40.0, 45.0)];
        let ranges = vec![
            (2.0, 8.0),    // 爆点前 → 拖沓
            (15.5, 20.0),  // 紧跟爆点 → 喜剧
            (22.0, 28.0),  // 离爆点远 → 拖沓
            (46.0, 50.0),  // 紧跟第二个爆点 → 喜剧
        ];
        let (dragging, comic) = split_silence_by_kind(&ranges, &punches);
        let mut all: Vec<(f64, f64)> = dragging.iter().chain(comic.iter()).copied().collect();
        all.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
        assert_eq!(all, ranges, "两类的并集必须等于检测结果");
        assert_eq!(dragging, vec![(2.0, 8.0), (22.0, 28.0)]);
        assert_eq!(comic, vec![(15.5, 20.0), (46.0, 50.0)]);
    }

    /// ⚠️⚠️ **端到端**：真的跑一次 `apply_silence_with`，看保护段有没有**落到磁盘上**。
    ///
    /// 上面那几条都是纯函数判据 —— "算对了但没接上"是**它们全都不会红**的错法。
    /// 这条走真实路径：写 `polish.toml` + `clip.srt`（`detect_silence_from_srt` **不需要音频**），
    /// 然后调 `apply_silence_with`，最后**读回 `polish.toml`** 检查 `mute_ranges` 里
    /// 确实没有那段"紧跟爆点之后的停顿"。
    #[test]
    fn apply_silence_with_really_protects_the_comic_pause() {
        let dir = std::env::temp_dir().join(format!("vt-sil7-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("data")).unwrap();

        // 字幕：0~5 一句、12~18 一句、30~35 一句。
        // ⇒ 每段间隔里都有 >3.5s 的静音，正是"会被剪"的那些。
        let srt = "1
00:00:00,000 --> 00:00:05,000
第一句话在这里

                   2
00:00:12,000 --> 00:00:18,000
第二句话在这里

                   3
00:00:30,000 --> 00:00:35,000
第三句话在这里

                   4
00:00:40,000 --> 00:00:45,000
紧接爆点之后的一句话

                   5
00:01:40,000 --> 00:01:45,000
很久以后的一句话

";
        std::fs::write(dir.join("clip.srt"), srt).unwrap();

        // `subtitles.json` 给爆点判据用（§6 的适配器读它）。
        // ⚠️ 18~30 那一段之后的静默（35s 之后没有）—— 这里让**第二个间隔**紧跟爆点。
        let subs = serde_json::json!([
            {"index":0,"start":0.0,"end":5.0,"text":"第一句话在这里","html":""},
            {"index":1,"start":12.0,"end":18.0,"text":"这里突然说了一大堆话哈哈哈哈哈","html":""},
            {"index":2,"start":30.0,"end":35.0,"text":"第三句话在这里","html":""}
        ]);
        std::fs::write(dir.join("data/subtitles.json"), subs.to_string()).unwrap();

        let mut cfg = ProjectFile::example();
        // ⚠️ 60s 之后是**远离任何爆点**的长静默（爆点只可能落在有字幕的地方），
        //    它必须被**判成拖沓、真剪掉** —— 否则这条用例只剩"保护"这一半，
        //    而"保护"与"整个没生效"在磁盘上长得一模一样（都是"那段不在 mute_ranges 里"）。
        cfg.duration = Some(120.0);
        cfg.source.subtitles = "clip.srt".into();
        cfg.source.video = "clip.mp4".into(); // 不存在 ⇒ 走 srt 检测（不需要音频）
        cfg.segments = vec![crate::project::Segment {
            start: 0.0,
            end: 60.0,
            fx: "high".into(),
            note: String::new(),
            role: String::new(),
            events: Vec::new(),
            extra: Default::default(),
        }];
        save_project(&dir, &cfg).unwrap();

        // ⚠️ 阈值取 3.0 让那些间隔都够得上（默认 3.5 也行，但 3.0 让用例不依赖默认值）
        let applied = apply_silence_with(&dir, 3.0, -30.0).expect("应用去静音");

        println!("检测到 {} 段，保护 {} 段", applied.ranges.len(), applied.protected.len());
        for (s, e) in &applied.ranges {
            println!("  剪: {s:.2}~{e:.2}");
        }
        for (s, e) in &applied.protected {
            println!("  保护: {s:.2}~{e:.2}");
        }

        // 读回磁盘，确认落盘的 `mute_ranges` **与 `applied.ranges` 一致**（保护段不在里面）。
        let back = load_project(&dir).unwrap();
        let written: Vec<(f64, f64)> = back
            .mute_ranges
            .iter()
            .filter_map(|r| r.normalized())
            .collect();
        for (s, e) in &applied.protected {
            let overlapped = written.iter().any(|(ws, we)| *ws < *e - RANGE_EPS && *s < *we - RANGE_EPS);
            assert!(
                !overlapped,
                "被保护的 {s:.2}~{e:.2} 竟然落进了磁盘上的 mute_ranges —— 保护没接上"
            );
        }
        assert_eq!(
            written.len(),
            applied.ranges.len(),
            "落盘的屏蔽段数应当等于 `applied.ranges`（保护段被剔除了）"
        );
        // ⚠️⚠️ **正面判据**（这是本条用例唯一能杀死"保护没生效"的地方）：
        //   上面那些断言全是"被保护的段**不在**磁盘上" —— 而**保护一旦整个失效**，
        //   `protected` 就是空的，那些断言会**全部空转通过**（实测：把整段保护删掉，
        //   用例仍然绿）。所以必须正面钉住"确实有一部分被保护下来了、且确实有一部分被剪了"。
        assert!(
            !applied.protected.is_empty(),
            "一段都没被保护 ⇒ §7 的分类没生效（保护与没生效长得一样）: {applied:?}"
        );
        // ⚠️ **必须至少有一段真被剪**：全都被保护的话，"保护"与"整个没生效"分不出来
        //   （两边都是"磁盘上没有那一段"）—— 这条断言就是用来排除那种**空转**的。
        assert!(
            !applied.ranges.is_empty(),
            "一段都没剪 ⇒ 这条用例证明不了任何事（保护与没生效长得一样）: {applied:?}"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 空区间 / 反向区间一律按拖沓（不参与保护）。
    #[test]
    fn degenerate_ranges_are_dragging() {
        let punches = vec![punch(10.0, 15.0)];
        assert_eq!(classify_silence(15.0, 15.0, &punches), SilenceKind::Dragging, "零长区间");
        assert_eq!(classify_silence(19.0, 15.0, &punches), SilenceKind::Dragging, "反向区间");
    }

}
