use std::io::Write;
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

const SPLEETER_PROBE: &str = ".probe";

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

fn write_probe(model_dir: &Path) -> Result<()> {
    let probe_path = model_dir.join(SPLEETER_PROBE);
    if !probe_path.exists() {
        std::fs::write(&probe_path, "OK")?;
    }
    Ok(())
}

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

fn get_audio_duration(path: &Path) -> Result<f64> {
    let output = Command::new("ffprobe")
        .args([
            "-v", "error",
            "-show_entries", "format=duration",
            "-of", "default=noprint_wrappers=1:nokey=1",
            path.to_str().unwrap(),
        ])
        .output()
        .with_context(|| "ffprobe 获取音频时长失败")?;
    let s = String::from_utf8_lossy(&output.stdout);
    s.trim()
        .parse::<f64>()
        .map_err(|e| anyhow::anyhow!("无法解析音频时长 '{}': {}", s.trim(), e))
}

fn spleeter_path(home: &Path) -> PathBuf {
    if cfg!(target_os = "windows") {
        home.join("venv").join("Scripts").join("spleeter.exe")
    } else {
        home.join("venv").join("bin").join("spleeter")
    }
}

/// 对单个音频文件运行 spleeter，返回输出 vocals.wav 的路径
fn run_spleeter(input: &Path, output_dir: &Path, home: &Path) -> Result<PathBuf> {
    let spleeter = spleeter_path(home);

    let cache_dir = home.join("pretrained_models");
    let model_dir = cache_dir.join("2stems");
    ensure_model_ready(&model_dir)?;

    std::fs::create_dir_all(output_dir)?;
    clean_cwd_residue();

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
        .with_context(|| format!("spleeter 执行失败 (输入: {})", input.display()))?;

    clean_cwd_residue();

    if !status.success() {
        bail!("spleeter 人声分离失败 (输入: {})", input.display());
    }

    let input_stem = input
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "audio".into());
    let spleeter_output = output_dir.join(&input_stem).join("vocals.wav");
    if !spleeter_output.exists() {
        bail!(
            "spleeter 未生成 vocals.wav (期望路径: {})",
            spleeter_output.display()
        );
    }
    Ok(spleeter_output)
}

/// 对超长音频分块处理：切块 → 逐块分离 → 拼接 vocals
fn separate_chunked(
    input: &Path,
    home: &Path,
    output_dir: &Path,
    total_duration: f64,
    chunk_secs: f64,
) -> Result<PathBuf> {
    let temp_dir = std::env::temp_dir().join("vtrim_spleeter_chunks");
    let chunks_dir = temp_dir.join("segments");
    let vocals_dir = temp_dir.join("vocals");
    std::fs::create_dir_all(&chunks_dir)?;
    std::fs::create_dir_all(&vocals_dir)?;

    let num_chunks = (total_duration / chunk_secs).ceil() as u32;
    tracing::info!(
        "音频较长 ({:.0}s)，将分 {} 块处理 (每块 {:.0}s)",
        total_duration,
        num_chunks,
        chunk_secs,
    );

    // 分块
    tracing::info!("切分音频...");
    let split_status = Command::new("ffmpeg")
        .arg("-y")
        .arg("-loglevel")
        .arg("error")
        .arg("-i")
        .arg(input)
        .arg("-f")
        .arg("segment")
        .arg("-segment_time")
        .arg(chunk_secs.to_string())
        .arg("-c")
        .arg("copy")
        .arg(chunks_dir.join("chunk_%04d.wav"))
        .output()
        .with_context(|| "ffmpeg 音频分块失败")?;

    if !split_status.status.success() {
        let stderr = String::from_utf8_lossy(&split_status.stderr);
        bail!("ffmpeg 音频分块失败: {}", stderr.trim());
    }

    // 收集实际生成的 chunk 文件
    let mut chunk_files: Vec<PathBuf> = std::fs::read_dir(&chunks_dir)?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().map(|ext| ext == "wav").unwrap_or(false))
        .collect();
    chunk_files.sort();

    if chunk_files.is_empty() {
        bail!("音频分块后未生成任何文件");
    }

    tracing::info!("分块完成: {} 个文件", chunk_files.len());

    // 逐块分离
    let mut vocal_chunks: Vec<PathBuf> = Vec::new();
    let bar = indicatif::ProgressBar::new(chunk_files.len() as u64);
    bar.set_style(
        indicatif::ProgressStyle::default_bar()
            .template("{spinner:.green} [{bar:40}] {pos}/{len} 块 ({msg})")
            .unwrap()
            .progress_chars("=> "),
    );
    for (i, chunk) in chunk_files.iter().enumerate() {
        bar.set_message(format!("chunk {}/{}", i + 1, chunk_files.len()));
        let chunk_out = vocals_dir.join(format!("chunk_{:04}", i));
        std::fs::create_dir_all(&chunk_out)?;

        let stem = chunk
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| format!("chunk_{:04}", i));

        // spleeter 输出路径: chunk_out/{stem}/vocals.wav
        let spleeter_out = run_spleeter(chunk, &chunk_out, home)?;

        // 重命名为统一名称: vocals_chunk_0000.wav
        let vocal_chunk_path = vocals_dir.join(format!("vocals_chunk_{:04}.wav", i));
        std::fs::rename(&spleeter_out, &vocal_chunk_path)?;

        // 清理 spleeter 的子目录
        let _ = std::fs::remove_dir_all(chunk_out.join(&stem));

        vocal_chunks.push(vocal_chunk_path);
        bar.inc(1);
    }
    bar.finish_and_clear();

    // 拼接
    tracing::info!("拼接 {} 个 vocal 分块...", vocal_chunks.len());
    let list_path = temp_dir.join("concat_list.txt");
    {
        let mut f = std::fs::File::create(&list_path)?;
        for vc in &vocal_chunks {
            let abs = std::fs::canonicalize(vc)
                .unwrap_or_else(|_| vc.clone());
            writeln!(f, "file '{}'", abs.display())?;
        }
    }

    let vocals = output_dir.join("vocals.wav");
    let concat_status = Command::new("ffmpeg")
        .arg("-y")
        .arg("-loglevel")
        .arg("error")
        .arg("-f")
        .arg("concat")
        .arg("-safe")
        .arg("0")
        .arg("-i")
        .arg(&list_path)
        .arg("-c")
        .arg("copy")
        .arg(&vocals)
        .output()
        .with_context(|| "ffmpeg 拼接 vocal 分块失败")?;

    // 清理临时文件
    let _ = std::fs::remove_dir_all(&temp_dir);

    if !concat_status.status.success() {
        let stderr = String::from_utf8_lossy(&concat_status.stderr);
        bail!("ffmpeg 拼接 vocal 分块失败: {}", stderr.trim());
    }

    // 验证输出时长
    let out_duration = get_audio_duration(&vocals).unwrap_or(0.0);
    let ratio = out_duration / total_duration;
    if ratio < 0.8 {
        tracing::warn!(
            "人声分离输出时长 ({:.0}s) 远小于输入 ({:.0}s)，可能截断",
            out_duration,
            total_duration,
        );
    }

    tracing::info!(
        "人声分离完成: {} ({:.0}s, {:.1}% 时长)",
        vocals.display(),
        out_duration,
        ratio * 100.0,
    );
    Ok(vocals)
}

pub fn separate(input: &Path, home: &Path, output_dir: &Path) -> Result<PathBuf> {
    let spleeter = spleeter_path(home);
    if !spleeter.exists() {
        anyhow::bail!(
            "Spleeter 未安装，请运行 `vtrim setup-spleeter` (查找路径: {})",
            spleeter.display()
        );
    }

    let cache_dir = home.join("pretrained_models");
    let model_dir = cache_dir.join("2stems");
    ensure_model_ready(&model_dir)?;

    std::env::set_var("MODEL_PATH", cache_dir.to_string_lossy().as_ref());

    let total_duration = get_audio_duration(input)?;
    let meta = std::fs::metadata(input)?;
    tracing::info!(
        "人声分离输入: {} ({:.0}s, {:.1} MB)",
        input.display(),
        total_duration,
        meta.len() as f64 / 1_000_000.0,
    );

    const CHUNK_SECS: f64 = 600.0;
    if total_duration > CHUNK_SECS {
        return separate_chunked(input, home, output_dir, total_duration, CHUNK_SECS);
    }

    let bar = indicatif::ProgressBar::new_spinner();
    bar.set_style(
        indicatif::ProgressStyle::default_spinner()
            .template("{spinner:.green} {msg}")
            .unwrap(),
    );
    bar.set_message("人声分离中 (spleeter)...");
    bar.enable_steady_tick(std::time::Duration::from_millis(100));

    let spleeter_output = run_spleeter(input, output_dir, home)?;

    bar.finish_and_clear();

    let vocals = output_dir.join("vocals.wav");
    std::fs::rename(&spleeter_output, &vocals)?;

    let input_stem = input
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "audio".into());
    let _ = std::fs::remove_dir_all(output_dir.join(&input_stem));

    let out_duration = get_audio_duration(&vocals).unwrap_or(0.0);
    tracing::info!(
        "人声分离完成: {} ({:.0}s, {:.1}% 时长)",
        vocals.display(),
        out_duration,
        out_duration / total_duration * 100.0,
    );
    Ok(vocals)
}
