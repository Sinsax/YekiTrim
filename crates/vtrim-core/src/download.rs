use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use reqwest::StatusCode;
use tokio::io::AsyncWriteExt;

const VERSION: &str = "1.13.3";

pub fn vtrim_home() -> PathBuf {
    std::env::current_exe()
        .expect("无法获取可执行文件路径")
        .parent()
        .expect("可执行文件路径异常")
        .join(".vtrim")
}

fn binary_url() -> String {
    let base = format!("https://github.com/k2-fsa/sherpa-onnx/releases/download/v{VERSION}");
    if cfg!(target_os = "windows") {
        format!("{base}/sherpa-onnx-v{VERSION}-win-x64-static-MD-Release.tar.bz2")
    } else if cfg!(all(target_os = "linux", target_arch = "aarch64")) {
        format!("{base}/sherpa-onnx-v{VERSION}-linux-aarch64-shared-cpu.tar.bz2")
    } else if cfg!(target_os = "linux") {
        format!("{base}/sherpa-onnx-v{VERSION}-linux-x64-shared.tar.bz2")
    } else if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        format!("{base}/sherpa-onnx-v{VERSION}-osx-arm64-shared.tar.bz2")
    } else if cfg!(target_os = "macos") {
        format!("{base}/sherpa-onnx-v{VERSION}-osx-x64-shared.tar.bz2")
    } else {
        panic!("unsupported platform");
    }
}

fn model_download_url() -> &'static str {
    "https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/sherpa-onnx-sense-voice-zh-en-ja-ko-yue-int8-2025-09-09.tar.bz2"
}

pub fn cpu_binary_name() -> &'static str {
    if cfg!(target_os = "windows") {
        "sherpa-onnx-offline.exe"
    } else {
        "sherpa-onnx-offline"
    }
}

pub fn cpu_binary_path() -> PathBuf {
    vtrim_home().join("bin").join(cpu_binary_name())
}

pub async fn download_all(target_dir: Option<&Path>) -> Result<()> {
    let home = target_dir.map_or_else(vtrim_home, |p| p.to_path_buf());
    let bin_dir = home.join("bin");
    let models_dir = home.join("models").join("sensevoice");

    tokio::fs::create_dir_all(&bin_dir)
        .await
        .context("创建 binary 目录失败")?;
    tokio::fs::create_dir_all(&models_dir)
        .await
        .context("创建模型目录失败")?;

    download_cpu_binary(&bin_dir).await?;
    download_model(&models_dir).await?;

    tracing::info!("下载完成！");
    tracing::info!("  binary: {}", bin_dir.join(cpu_binary_name()).display());
    tracing::info!("  模型:   {}", models_dir.display());
    tracing::info!("运行 `vtrim transcribe` 开始语音识别");

    Ok(())
}

async fn download_cpu_binary(target: &Path) -> Result<()> {
    let url = binary_url();
    let target_path = target.join(cpu_binary_name());

    if target_path.exists() {
        tracing::info!("sherpa-onnx CPU binary 已存在，跳过下载");
        return Ok(());
    }

    tracing::info!("下载 sherpa-onnx CPU (v{VERSION})...");
    let archive_path = target.join("_sherpa-onnx.tar.bz2");
    let part_path = target.join("_sherpa-onnx.tar.bz2.part");
    download_file(&url, &part_path).await?;
    tokio::fs::rename(&part_path, &archive_path).await
        .context("重命名临时文件失败")?;

    tracing::info!("  解压 binary...");
    extract_binaries(&archive_path, target)?;
    tokio::fs::remove_file(&archive_path).await.ok();

    set_executable(&target_path);
    Ok(())
}

fn extract_binaries(archive: &Path, target: &Path) -> Result<()> {
    let file = std::fs::File::open(archive)?;
    let decoder = bzip2::read::BzDecoder::new(file);
    let mut archive_tar = tar::Archive::new(decoder);

    let cpu_name = cpu_binary_name();

    for entry in archive_tar.entries()? {
        let mut entry = entry?;
        let path = entry.path()?.to_path_buf();
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();

        if matches_name(&name, "sherpa-onnx-offline") {
            let target_path = target.join(cpu_name);
            let mut outfile = std::fs::File::create(&target_path)?;
            std::io::copy(&mut entry, &mut outfile)?;
            tracing::info!("  解压: {} -> {}", name, cpu_name);
        }
    }

    if !target.join(cpu_name).exists() {
        bail!("在归档中未找到 sherpa-onnx-offline binary");
    }

    Ok(())
}

fn matches_name(name: &str, base: &str) -> bool {
    name == base || name == &format!("{}.exe", base)
}

async fn download_model(target: &Path) -> Result<()> {
    if target.join("tokens.txt").exists() {
        tracing::info!("SenseVoice 模型已存在，跳过下载");
        return Ok(());
    }

    let url = model_download_url();
    tracing::info!("下载 SenseVoice 模型...");

    let archive_path = target.join("_model.tar.bz2");
    let part_path = target.join("_model.tar.bz2.part");
    download_file(url, &part_path).await?;
    tokio::fs::rename(&part_path, &archive_path).await
        .context("重命名临时模型文件失败")?;

    tracing::info!("  解压模型...");
    extract_model_tarbz2(&archive_path, target)?;
    tokio::fs::remove_file(&archive_path).await.ok();
    Ok(())
}

fn extract_model_tarbz2(archive: &Path, target: &Path) -> Result<()> {
    let file = std::fs::File::open(archive)?;
    let decoder = bzip2::read::BzDecoder::new(file);
    let mut archive_tar = tar::Archive::new(decoder);

    for entry in archive_tar.entries()? {
        let mut entry = entry?;
        let path = entry.path()?.to_path_buf();

        let components: Vec<_> = path.components().skip(1).collect();
        if components.is_empty() {
            continue;
        }

        let rel_path: PathBuf = components.iter().collect();
        let target_path = target.join(&rel_path);

        if entry.header().entry_type().is_dir() {
            std::fs::create_dir_all(&target_path)?;
            continue;
        }

        if let Some(parent) = target_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let mut outfile = std::fs::File::create(&target_path)?;
        std::io::copy(&mut entry, &mut outfile)?;
        tracing::info!("  解压: {}", rel_path.display());
    }

    Ok(())
}

async fn download_file(url: &str, part_path: &Path) -> Result<()> {
    let mut existing = std::fs::metadata(part_path)
        .map(|m| m.len())
        .unwrap_or(0);

    let client = reqwest::Client::new();
    let mut req = client.get(url);
    if existing > 0 {
        req = req.header("Range", format!("bytes={}-", existing));
    }
    let resp = req
        .send()
        .await
        .with_context(|| format!("下载失败: {url}"))?;

    let status = resp.status();
    let total = if status == StatusCode::PARTIAL_CONTENT {
        existing + resp.content_length().unwrap_or(0)
    } else if status == StatusCode::OK {
        if existing > 0 {
            std::fs::remove_file(part_path).ok();
            existing = 0;
        }
        resp.content_length().unwrap_or(0)
    } else if status == StatusCode::RANGE_NOT_SATISFIABLE {
        return Ok(());
    } else {
        bail!("下载返回 {} ({url})", status);
    };

    let bar = indicatif::ProgressBar::new(total.max(existing));
    bar.set_position(existing);
    bar.set_style(
        indicatif::ProgressStyle::default_bar()
            .template("{spinner:.green} [{elapsed_precise}] {bar:40.cyan/blue} {bytes}/{total_bytes} ({bytes_per_sec}, {eta})")
            .unwrap()
            .progress_chars("##-"),
    );

    let mut file = tokio::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .append(existing > 0)
        .open(part_path)
        .await
        .with_context(|| format!("创建临时文件失败: {}", part_path.display()))?;

    let mut stream = resp;
    while let Some(chunk) = stream.chunk().await? {
        file.write_all(&chunk).await?;
        bar.inc(chunk.len() as u64);
    }

    bar.finish_and_clear();
    Ok(())
}

#[cfg(unix)]
fn set_executable(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    if let Ok(meta) = std::fs::metadata(path) {
        let mut perms = meta.permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(path, perms).ok();
    }
}

#[cfg(not(unix))]
fn set_executable(_path: &Path) {}

pub async fn ensure_models() -> Result<()> {
    let bin_dir = vtrim_home().join("bin");
    let model_path = vtrim_home().join("models").join("sensevoice");

    let need_bin = !cpu_binary_path().exists();
    let need_model = !model_path.join("tokens.txt").exists();

    if need_bin {
        tokio::fs::create_dir_all(&bin_dir).await?;
        tracing::info!("下载 sherpa-onnx binary...");
        download_cpu_binary(&bin_dir).await?;
    }
    if need_model {
        tokio::fs::create_dir_all(&model_path).await?;
        tracing::info!("下载 SenseVoice 模型到 {}", model_path.display());
        download_model(&model_path).await?;
    }
    if !need_bin && !need_model {
        tracing::info!("本地 ASR 模型已就绪");
    }

    Ok(())
}


