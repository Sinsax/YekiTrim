use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{bail, Context, Result};

fn clean_cwd_residue() {
    if let Ok(cwd) = std::env::current_dir() {
        let p = cwd.join("pretrained_models");
        if p.exists() {
            let _ = std::fs::remove_dir_all(&p);
        }
    }
}

/// spleeter 通过 `.probe` 文件判断模型是否已就绪
const SPLEETER_PROBE: &str = ".probe";

/// 修复旧版 `setup-spleeter` 造成的双层嵌套问题:
/// `pretrained_models/2stems/2stems/checkpoint` → `pretrained_models/2stems/checkpoint`
fn fix_nested_model_dir(model_dir: &Path) -> Result<()> {
    let nested = model_dir.join("2stems");
    if !nested.exists() {
        return Ok(());
    }
    tracing::warn!("检测到模型目录结构异常 (双层嵌套)，正在修复...");
    for entry in std::fs::read_dir(&nested)? {
        let entry = entry?;
        let target = model_dir.join(entry.file_name());
        if !target.exists() {
            std::fs::rename(entry.path(), &target)?;
        }
    }
    std::fs::remove_dir_all(&nested)?;
    tracing::info!("模型目录已修复");
    Ok(())
}

/// 在模型目录写入 `.probe` 文件，告知 spleeter 模型已就绪
fn write_probe(model_dir: &Path) -> Result<()> {
    let probe_path = model_dir.join(SPLEETER_PROBE);
    if !probe_path.exists() {
        std::fs::write(&probe_path, "OK")?;
    }
    Ok(())
}

/// 检查模型目录是否就绪，尝试自动修复，并写入 `.probe`
fn ensure_model_ready(model_dir: &Path) -> Result<()> {
    if !model_dir.exists() {
        bail!(
            "模型目录不存在 ({}), 请运行 `vtrim setup-spleeter` 下载模型",
            model_dir.display()
        );
    }

    let checkpoint = model_dir.join("checkpoint");
    if checkpoint.exists() {
        write_probe(model_dir)?;
        return Ok(());
    }

    // 检测并修复双层嵌套: 2stems/2stems/checkpoint
    if model_dir.join("2stems").join("checkpoint").exists() {
        fix_nested_model_dir(model_dir)?;
        if checkpoint.exists() {
            write_probe(model_dir)?;
            return Ok(());
        }
    }

    if model_dir.read_dir()?.next().is_some() {
        bail!(
            "模型目录损坏 ({}), 缺少 checkpoint 文件，请重新运行 `vtrim setup-spleeter`",
            model_dir.display()
        );
    }

    bail!(
        "模型目录为空 ({}), 请运行 `vtrim setup-spleeter` 下载模型",
        model_dir.display()
    );
}

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

    let cache_dir = home.join("pretrained_models");
    let model_dir = cache_dir.join("2stems");
    ensure_model_ready(&model_dir)?;

    std::fs::create_dir_all(output_dir)?;

    clean_cwd_residue();

    let bar = indicatif::ProgressBar::new_spinner();
    bar.set_style(
        indicatif::ProgressStyle::default_spinner()
            .template("{spinner:.green} {msg}")
            .unwrap()
    );
    bar.set_message("人声分离中 (spleeter)...");
    bar.enable_steady_tick(std::time::Duration::from_millis(100));

    std::env::set_var("MODEL_PATH", cache_dir.to_string_lossy().as_ref());
    let status = Command::new(&spleeter)
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
    clean_cwd_residue();

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
