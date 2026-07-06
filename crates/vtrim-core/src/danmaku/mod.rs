mod parser;
pub use parser::*;

use std::path::Path;

use anyhow::Result;

use crate::types::DanmakuRef;

/// Load danmaku as a flat list (for correction context, etc.)
pub fn load_danmaku_refs(path: &Path) -> Result<Vec<DanmakuRef>> {
    let entries = parser::parse_ass(path)?;
    Ok(entries
        .into_iter()
        .map(|e| DanmakuRef {
            start: e.start,
            end: e.end,
            text: e.text,
        })
        .collect())
}

/// Per-window danmaku density.
#[derive(Debug, Clone)]
pub struct DensityWindow {
    pub start: f64,
    /// Normalized density 0.0 ~ 1.0
    pub density: f64,
    /// Raw danmaku count in this window
    pub count: u64,
}

/// Analyze danmaku file and produce a normalized density heatmap.
///
/// * `path` — path to ASS file
/// * `window_secs` — bucket size in seconds (e.g. 10.0)
/// * `total_duration` — total video duration in seconds (used for window count)
///
/// Returns a vector of density windows sorted by start time.
pub fn analyze(path: &Path, window_secs: f64, total_duration: f64) -> Result<Vec<DensityWindow>> {
    let entries = parser::parse_ass(path)?;
    if entries.is_empty() {
        return Ok(Vec::new());
    }

    let num_windows = (total_duration / window_secs).ceil() as usize;
    let mut counts = vec![0u64; num_windows];

    for entry in &entries {
        // Map the midpoint of each danmaku to a window
        let mid = (entry.start + entry.end) / 2.0;
        let idx = (mid / window_secs) as usize;
        if idx < num_windows {
            counts[idx] += 1;
        }
    }

    let max_count = counts.iter().copied().max().unwrap_or(1).max(1);

    let heatmap: Vec<DensityWindow> = counts
        .into_iter()
        .enumerate()
        .map(|(i, count)| DensityWindow {
            start: i as f64 * window_secs,
            density: count as f64 / max_count as f64,
            count,
        })
        .collect();

    Ok(heatmap)
}

/// Compute the average danmaku popularity for a time range [start, end).
pub fn average_density(heatmap: &[DensityWindow], start: f64, end: f64, window_secs: f64) -> f64 {
    if heatmap.is_empty() {
        return 0.0;
    }

    let start_idx = (start / window_secs) as usize;
    let end_idx = ((end / window_secs) as usize).min(heatmap.len().saturating_sub(1));

    if start_idx > end_idx {
        return 0.0;
    }

    let mut total = 0.0;
    let mut count = 0u32;
    for i in start_idx..=end_idx {
        total += heatmap[i].density;
        count += 1;
    }

    if count == 0 {
        0.0
    } else {
        total / count as f64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_analyze_empty() {
        let dir = std::env::temp_dir();
        let path = dir.join("empty_test.ass");
        std::fs::write(&path, "[Events]\nFormat: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\n").unwrap();
        let result = analyze(&path, 10.0, 100.0).unwrap();
        assert!(result.is_empty());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_density_window() {
        let windows = vec![
            DensityWindow { start: 0.0, density: 0.5, count: 5 },
            DensityWindow { start: 10.0, density: 1.0, count: 10 },
            DensityWindow { start: 20.0, density: 0.0, count: 0 },
        ];
        let avg = average_density(&windows, 5.0, 15.0, 10.0);
        assert!((avg - 0.75).abs() < 1e-6);
    }
}
