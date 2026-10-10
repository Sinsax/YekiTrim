//! 消音版 Service：CLI / WebUI 共用。
//!
//! 静音区间不再通过重编码写到 `*_cleaned.mp4`，而是映射为精修工程的「屏蔽轨道」
//! （polish.toml 的 `mute_ranges`），由 process/render 在导出阶段剪掉。
//! 原始视频与字幕文件保持不变；未初始化的切片会先自动 init。

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

/// 对一批切片应用消音版（静音 → 屏蔽轨道）。
///
/// `files` 为空时扫描 `output/clips/*/clip.mp4`。
/// `duration` 为长静音判定阈值（秒），`threshold` 为音频分析噪声门限（dB）。
pub async fn run_silence(
    files: &[PathBuf],
    duration: f64,
    threshold: Option<f64>,
) -> Result<()> {
    let noise_db = threshold.unwrap_or(yekitrim_core::cutting::DEFAULT_NOISE_DB);
    let targets = resolve_targets(files)?;
    if targets.is_empty() {
        anyhow::bail!("没有找到要处理的切片目录");
    }

    let mut changed = 0usize;
    for dir in &targets {
        // 未初始化的切片先建精修工程，否则没有「屏蔽轨道」可写
        if !dir.join("polish.toml").exists() {
            yekitrim_polish::commands::cmd_init(dir)
                .await
                .with_context(|| format!("初始化精修工程失败: {}", dir.display()))?;
        }
        let name = dir.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        match yekitrim_polish::project::apply_silence_with(dir, duration, noise_db) {
            Ok(applied) if applied.ranges.is_empty() => {
                tracing::info!("  · {name}: 未检测到 {}s 以上的静音段", duration);
            }
            Ok(applied) if applied.added == 0 => {
                // 与当前屏蔽相加之后没有新东西（检测到的静音本来就屏掉了）
                tracing::info!(
                    "  · {name}: 检测到 {} 段静音，屏蔽轨道里已有（相加后共 {} 段）",
                    applied.ranges.len(), applied.total
                );
            }
            Ok(applied) => {
                // §7：被保护下来的那些（紧跟爆点之后的"喜剧停顿"）**必须报出来** ——
                // 不报的话用户看到"有一段没被剪"，界面上没有任何解释，
                // 那看上去就是**功能没生效**（而不是"它有意保留了"）。
                if applied.protected.is_empty() {
                    tracing::info!(
                        "  ✓ {name}: 静音 {} 段 → 与现有屏蔽相加（新增 {} / 共 {}）",
                        applied.ranges.len(), applied.added, applied.total
                    );
                } else {
                    tracing::info!(
                        "  ✓ {name}: 静音 {} 段 → 与现有屏蔽相加（新增 {} / 共 {}）；                         §7 保护了 {} 段紧跟爆点之后的停顿（喜剧节拍）",
                        applied.ranges.len(), applied.added, applied.total,
                        applied.protected.len()
                    );
                }
                changed += 1;
            }
            Err(e) => tracing::warn!("  ⚠ {name}: {e:#}"),
        }
    }

    tracing::info!(
        "完成: {} 个切片已写入屏蔽轨道（视频文件未改动）；重新 process / render 后生效",
        changed
    );
    Ok(())
}

/// 解析目标：显式传入的视频文件 → 其所在目录；默认扫描 `output/clips/`。
fn resolve_targets(files: &[PathBuf]) -> Result<Vec<PathBuf>> {
    if files.is_empty() {
        let clips = std::env::current_dir().unwrap_or_default().join("output").join("clips");
        let mut list = Vec::new();
        if clips.exists() {
            for e in std::fs::read_dir(&clips)? {
                let e = e?;
                if e.path().is_dir() && e.path().join("clip.mp4").exists() {
                    list.push(e.path());
                }
            }
        }
        list.sort();
        return Ok(list);
    }

    let mut list = Vec::new();
    for f in files {
        let path = if f.is_dir() {
            f.clone()
        } else {
            f.parent().map(Path::to_path_buf).unwrap_or_else(|| f.clone())
        };
        if !list.contains(&path) {
            list.push(path);
        }
    }
    Ok(list)
}
