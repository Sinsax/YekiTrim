//! cutting::silence：静音检测（字幕间隔优先，音频分析兜底）。
//!
//! 入口：`detect_silence`、`detect_silence_from_srt`、`silence_gaps_from_segments`、`detect_silence_ranges`、`DEFAULT_MIN_GAP_SECS`、`DEFAULT_NOISE_DB`。
//! 边界：默认阈值 3.5s 是用户定的，别下调；乱序/重叠字幕必须先排序合并再取相邻间隔，否则会造出几十秒假静音 —— 见 `crates/yekitrim-core/src/cutting/AGENTS.md`

use std::path::Path;

use anyhow::{Context, Result};

/// 长静音判定阈值（秒）：超过此时长的连续静音视为可屏蔽区间。
///
/// 3.5s（用户要求）：**字幕之间 3.5s 以内的间隔一律不处理** —— 正常语气停顿也在这个量级，
/// 剪掉反而把对话切得很碎；再配上两侧各 0.8s 的气口（`yekitrim_polish::project::SILENCE_PAD_SECS`），
/// 实际被剪掉的只有 `间隔 − 1.6s`。
pub const DEFAULT_MIN_GAP_SECS: f64 = 3.5;

/// 音频分析默认噪声门限（dB）。
pub const DEFAULT_NOISE_DB: f64 = -30.0;

/// 合并后的相邻区间容差（秒）：首尾相接的两条字幕（`0.0–2.07` + `2.07–5.79`）之间不算静音。
const MERGE_EPS: f64 = 0.001;

pub fn detect_silence_from_srt(srt_path: &Path, threshold: f64) -> Result<Vec<(f64, f64)>> {
    let content = std::fs::read_to_string(srt_path)?;
    let transcript = crate::srt::parse_srt(&content)?;
    let segs: Vec<(f64, f64)> = transcript
        .segments
        .iter()
        .map(|s| (s.start, s.end))
        .collect();

    let segments = silence_gaps_from_segments(&segs, threshold);
    tracing::debug!(
        "SRT gap 检测: {} 段静音 (阈值={}s)",
        segments.len(),
        threshold,
    );
    Ok(segments)
}

/// 字幕之间的长间隔 = 静音。**先按时间排序并合并重叠区间**，再取相邻间隔。
///
/// ⚠ 不能直接按文件顺序取相邻差：字幕文件可能是乱序/重叠的 ——
/// 编辑器「拆分」曾经把后半段**追加到列表末尾**（`TimelineEditor.splitSelected`，已修），
/// 时间轴一旦跳变，那几处跳变会被当成几十秒的静音。
/// 实测 clip-19：13 段 / 121s 的假静音（半个片段都被判成静音），区间全落在说话中间，
/// 用户直接反馈"去静音完全对不上，像用了别的素材"。
pub fn silence_gaps_from_segments(segs: &[(f64, f64)], threshold: f64) -> Vec<(f64, f64)> {
    let mut sorted: Vec<(f64, f64)> = segs.iter().copied().filter(|(s, e)| e > s).collect();
    if sorted.len() < 2 {
        return Vec::new();
    }
    sorted.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));

    let mut merged: Vec<(f64, f64)> = Vec::with_capacity(sorted.len());
    for (s, e) in sorted {
        match merged.last_mut() {
            Some(last) if s <= last.1 + MERGE_EPS => {
                if e > last.1 {
                    last.1 = e;
                }
            }
            _ => merged.push((s, e)),
        }
    }

    merged
        .windows(2)
        .filter_map(|pair| {
            let gap = pair[1].0 - pair[0].1;
            (gap >= threshold).then_some((pair[0].1, pair[1].0))
        })
        .collect()
}

pub fn detect_silence(input: &Path, duration: f64, noise_db: f64) -> Result<Vec<(f64, f64)>> {
    let result = crate::process::cmd("ffmpeg")
        .arg("-y")
        .arg("-i")
        .arg(input)
        .arg("-af")
        .arg(format!("silencedetect=n={}dB:d={}", noise_db, duration))
        .arg("-f")
        .arg("null")
        .arg("-")
        .output()
        .with_context(|| "ffmpeg silencedetect 失败")?;

    let stderr = String::from_utf8_lossy(&result.stderr);
    if !result.status.success() {
        anyhow::bail!("ffmpeg silencedetect 失败:\n{}", stderr.trim());
    }

    let mut segments: Vec<(f64, f64)> = Vec::new();
    let mut current_start: Option<f64> = None;

    for line in stderr.lines() {
        let line = line.trim();
        if let Some(pos) = line.find("silence_start: ") {
            let val = line[pos + 14..].trim();
            if let Ok(t) = val.parse::<f64>() {
                current_start = Some(t);
            }
        } else if let Some(pos) = line.find("silence_end: ") {
            let after = &line[pos + 12..];
            if let Some(pipe) = after.find('|') {
                let t_str = after[..pipe].trim();
                if let (Ok(end), Some(start)) = (t_str.parse::<f64>(), current_start) {
                    segments.push((start, end));
                    current_start = None;
                }
            }
        }
    }

    Ok(segments)
}

/// 检测长静音区间，供「消音版 = 映射到屏蔽轨道」使用。
///
/// 优先按字幕间隔检测（快、可复现，无需解码视频）；没有字幕文件时才退回音频分析。
/// 结果只用于写入 `mute_ranges`，不产生任何新的视频/字幕文件。
pub fn detect_silence_ranges(
    video: &Path,
    srt: Option<&Path>,
    min_gap: f64,
    noise_db: f64,
) -> Result<Vec<(f64, f64)>> {
    match srt {
        Some(srt) if srt.exists() => {
            tracing::debug!("按字幕间隔检测静音: {}", srt.display());
            detect_silence_from_srt(srt, min_gap)
        }
        _ => {
            if !video.exists() {
                anyhow::bail!("缺少字幕与视频文件，无法检测静音");
            }
            tracing::debug!("按音频分析检测静音 (noise={}dB)", noise_db);
            detect_silence(video, min_gap, noise_db)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::silence_gaps_from_segments;

    /// 有序且不重叠的字幕：间隔 ≥ 阈值才算静音（原有行为）。
    #[test]
    fn sorted_segments_yield_real_gaps_only() {
        let segs = [(0.0, 2.0), (2.0, 4.0), (8.0, 9.0), (10.5, 12.0)];
        let gaps = silence_gaps_from_segments(&segs, 3.5);
        // 只有 4.0→8.0（4s）够长；2.0 相接、9.0→10.5 只有 1.5s 都不算
        assert_eq!(gaps, vec![(4.0, 8.0)]);
    }

    /// **乱序 + 重叠的字幕不能凭空造出静音**（clip-19 的真实形状）。
    ///
    /// 编辑器「拆分」曾把后半段追加到列表末尾 → 文件里 `124.62→127.71` 后面紧跟 `2.07→5.79`。
    /// 按文件顺序取相邻差会得到 13 段/121s 的假静音（而真实静音只有一处），这就是用户报的
    /// 「去静音完全对不上」。
    #[test]
    fn out_of_order_and_overlapping_segments_do_not_invent_silence() {
        let mut segs = vec![
            (0.0, 2.07),
            (7.08, 9.45),
            (99.76, 102.49),
            (124.62, 127.71),
        ];
        // 追加在末尾的「拆分后半段」：时间上落在开头，且与 0.0–2.07 首尾相接
        segs.push((2.07, 5.79));
        segs.push((5.79, 7.08));

        let gaps = silence_gaps_from_segments(&segs, 3.5);
        // 合并成 0.0–9.45 / 99.76–102.49 / 124.62–127.71 之后，只剩两处真间隔
        assert_eq!(gaps, vec![(9.45, 99.76), (102.49, 124.62)]);

        // 若按文件顺序硬算，会多出 5.79→7.08 之类的假区间（回归护栏）
        let naive: Vec<(f64, f64)> = segs
            .windows(2)
            .filter(|w| w[1].0 - w[0].1 >= 3.5)
            .map(|w| (w[0].1, w[1].0))
            .collect();
        assert!(naive.len() > gaps.len());
    }

    /// 完全被包含的重复条目（同一句被转了两遍）不该产生任何间隔。
    #[test]
    fn duplicated_covering_segment_creates_no_gap() {
        let segs = [(0.0, 10.0), (10.0, 20.0), (2.0, 8.0)];
        assert!(silence_gaps_from_segments(&segs, 3.5).is_empty());
    }
}

