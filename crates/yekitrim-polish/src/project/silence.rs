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
    /// 本次检测到的静音区间
    pub ranges: Vec<(f64, f64)>,
    /// 真正新加入屏蔽轨道的区间数（检测结果减掉当前屏蔽状态之后剩下的）
    pub added: usize,
    /// 应用后屏蔽轨道总区间数
    pub total: usize,
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
    let detected = detect_ranges_with(project_path, min_gap, noise_db, true)?;
    let before = pairs(&cfg.mute_ranges);
    let prev = pairs(&cfg.silence_ranges); // 上次「去静音」加进去的，重复点击时别把撤销依据丢掉
    let fresh = subtract_ranges(&detected, &before);
    let merged = union_ranges(&before, &detected);

    cfg.mute_ranges = to_mute_ranges(&merged);
    cfg.silence_ranges = to_mute_ranges(&union_ranges(&fresh, &prev));
    save_project(project_path, &cfg)?;
    Ok(SilenceApply {
        ranges: detected,
        added: fresh.len(),
        total: merged.len(),
    })
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
                "vtrim-silence-{tag}-{}-{:?}",
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
}
