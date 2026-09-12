//! 「去静音」= 静音区间 → 屏蔽轨道（mute_ranges）。
//!
//! 历史实现会调用 ffmpeg 重编码出一份 `*_cleaned.mp4` 并把 `source.video` 切过去；
//! 现在改为：检测到的长静音段写进 `polish.toml` 的 `mute_ranges`（时间轴「屏蔽」轨道），
//! 由 process / render 在导出阶段剪掉，原始 `clip.mp4` / `clip.srt` 完全不动。
//!
//! `silence_ranges` 记录本次检测结果，撤销去静音时按它精确回退，
//! 不会误删用户手动添加的屏蔽区间。

use std::path::{Path, PathBuf};

use super::{load_project, MuteRange, ProjectFile};

/// 区间比较容差（秒）：检测结果可复现，留一点余量防止浮点误差。
const RANGE_EPS: f64 = 0.01;

/// 把静音区间并入 `mute_ranges` 后的结果。
#[derive(Debug, Clone, Default)]
pub struct SilenceApply {
    /// 本次检测到的静音区间
    pub ranges: Vec<(f64, f64)>,
    /// 新加入屏蔽轨道的数量（已存在的不重复添加）
    pub added: usize,
    /// 应用后屏蔽轨道总区间数
    pub total: usize,
}

/// 检测该精修工程对应片段的静音区间（不写盘）。
///
/// `allow_audio_fallback=false` 时只按字幕间隔检测（列表接口用，避免逐条解码视频）。
pub fn detect_ranges(project_path: &Path, allow_audio_fallback: bool) -> anyhow::Result<Vec<(f64, f64)>> {
    detect_ranges_with(
        project_path,
        vtrim_core::cutting::DEFAULT_MIN_GAP_SECS,
        vtrim_core::cutting::DEFAULT_NOISE_DB,
        allow_audio_fallback,
    )
}

/// 带阈值的静音检测：`min_gap` 为长静音阈值（秒），`noise_db` 仅在音频分析时生效。
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
    vtrim_core::cutting::detect_silence_ranges(&video, srt.as_deref(), min_gap, noise_db)
}

/// 应用消音版：静音区间写入屏蔽轨道（含 `silence_ranges` 记录）。
pub fn apply_silence(project_path: &Path) -> anyhow::Result<SilenceApply> {
    apply_silence_with(
        project_path,
        vtrim_core::cutting::DEFAULT_MIN_GAP_SECS,
        vtrim_core::cutting::DEFAULT_NOISE_DB,
    )
}

/// 带阈值的消音版应用。
pub fn apply_silence_with(project_path: &Path, min_gap: f64, noise_db: f64) -> anyhow::Result<SilenceApply> {
    let mut cfg = load_project(project_path)?;
    let detected = detect_ranges_with(project_path, min_gap, noise_db, true)?;
    let mut added = 0usize;
    for (start, end) in &detected {
        if start >= end {
            continue;
        }
        let exists = cfg
            .mute_ranges
            .iter()
            .any(|r| matches_range(r, *start, *end));
        if !exists {
            cfg.mute_ranges.push(MuteRange { start: *start, end: *end });
            added += 1;
        }
    }
    cfg.mute_ranges.sort_by(|a, b| a.start.partial_cmp(&b.start).unwrap_or(std::cmp::Ordering::Equal));
    cfg.silence_ranges = detected
        .iter()
        .map(|(start, end)| MuteRange { start: *start, end: *end })
        .collect();
    save_project(project_path, &cfg)?;
    Ok(SilenceApply {
        ranges: detected,
        added,
        total: cfg.mute_ranges.len(),
    })
}

/// 取消消音版：只移除 `silence_ranges` 记录的区间，手动屏蔽区间保持不变。
pub fn clear_silence(project_path: &Path) -> anyhow::Result<usize> {
    let mut cfg = load_project(project_path)?;
    if cfg.silence_ranges.is_empty() {
        return Ok(0);
    }
    let recorded = std::mem::take(&mut cfg.silence_ranges);
    let before = cfg.mute_ranges.len();
    cfg.mute_ranges.retain(|r| {
        !recorded
            .iter()
            .any(|s| matches_range(r, s.start, s.end))
    });
    let removed = before - cfg.mute_ranges.len();
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

fn matches_range(range: &MuteRange, start: f64, end: f64) -> bool {
    (range.start - start).abs() <= RANGE_EPS && (range.end - end).abs() <= RANGE_EPS
}

fn resolve(project_path: &Path, value: &str) -> PathBuf {
    let p = Path::new(value);
    if p.is_absolute() { p.to_path_buf() } else { project_path.join(p) }
}

fn save_project(project_path: &Path, cfg: &ProjectFile) -> anyhow::Result<()> {
    let raw = cfg.to_toml_commented()?;
    std::fs::write(project_path.join("polish.toml"), raw)?;
    Ok(())
}
