use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{bail, Context, Result};

/// Call spleeter from .vtrim/venv/ to separate vocals.
/// Returns path to vocals.wav.
pub fn separate(input: &Path, home: &Path, output_dir: &Path, stem: &str) -> Result<PathBuf> {
    let spleeter = if cfg!(target_os = "windows") {
        home.join("venv").join("Scripts").join("spleeter.exe")
    } else {
        home.join("venv").join("bin").join("spleeter")
    };

    if !spleeter.exists() {
        anyhow::bail!(
            "Spleeter 未安装，请运行 `vtrim setup-spleeter` (查找路径: {})",
            spleeter.display()
        );
    }

    std::fs::create_dir_all(output_dir)?;

    let bar = indicatif::ProgressBar::new_spinner();
    bar.set_style(
        indicatif::ProgressStyle::default_spinner()
            .template("{spinner:.green} {msg}")
            .unwrap()
    );
    bar.set_message("人声分离中 (spleeter)...");
    bar.enable_steady_tick(std::time::Duration::from_millis(100));

    let status = Command::new(&spleeter)
        .env("SPLEETER_CACHE_DIR", home.join("pretrained_models").to_string_lossy().as_ref())
        .args([
            "separate",
            "-p",
            "spleeter:2stems",
            "-o",
            output_dir.to_string_lossy().as_ref(),
            input.to_string_lossy().as_ref(),
        ])
        .status()
        .context("spleeter 执行失败")?;

    bar.finish_and_clear();

    if !status.success() {
        bail!("spleeter 人声分离失败");
    }

    let vocals = output_dir.join(stem).join("vocals.wav");
    if !vocals.exists() {
        bail!("spleeter 未生成 vocals.wav (期望路径: {})", vocals.display());
    }

    tracing::info!("人声分离完成: {}", vocals.display());
    Ok(vocals)
}
