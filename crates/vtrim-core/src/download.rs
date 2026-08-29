use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use reqwest::StatusCode;
use tokio::io::AsyncWriteExt;

use crate::config::AsrConfig;
use crate::path::vtrim_home;

const ASR_MODELS_BASE: &str = "https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models";

/// 支持的 ASR 模型: (配置名, 模型目录名, 下载文件名, 已安装标记文件)
const MODEL_REGISTRY: &[(&str, &str, &str, &str)] = &[
    (
        "sensevoice",
        "sensevoice",
        "sherpa-onnx-sense-voice-zh-en-ja-ko-yue-int8-2025-09-09.tar.bz2",
        "tokens.txt",
    ),
    (
        "firered-zh",
        "firered-zh",
        "sherpa-onnx-fire-red-asr2-ctc-zh_en-int8-2026-02-25.tar.bz2",
        "model.int8.onnx",
    ),
    (
        "qwen3-asr",
        "qwen3-asr",
        "sherpa-onnx-qwen3-asr-0.6B-int8-2026-03-25.tar.bz2",
        "tokenizer/tokenizer_config.json",
    ),
];

fn model_archive(name: &str) -> Option<&'static str> {
    MODEL_REGISTRY
        .iter()
        .find(|(cfg, _, _, _)| *cfg == name)
        .map(|(_, _, archive, _)| *archive)
}

fn model_marker(name: &str) -> Option<&'static str> {
    MODEL_REGISTRY
        .iter()
        .find(|(cfg, _, _, _)| *cfg == name)
        .map(|(_, _, _, marker)| *marker)
}

fn hr_base_url() -> &'static str {
    "https://github.com/k2-fsa/sherpa-onnx/releases/download/hr-files"
}

pub async fn download_all(target_dir: Option<&Path>) -> Result<()> {
    let home = target_dir.map_or_else(vtrim_home, |p| p.to_path_buf());

    for (cfg, dir, _, _) in MODEL_REGISTRY {
        let models_dir = home.join("models").join(dir);
        tokio::fs::create_dir_all(&models_dir)
            .await
            .context("创建模型目录失败")?;
        download_model(&models_dir, cfg).await?;
        if *cfg == "sensevoice" {
            download_hr_files(&models_dir).await?;
        }
    }

    tracing::info!("下载完成！");
    tracing::info!("模型目录: {}", home.join("models").display());
    tracing::info!("运行 `vtrim transcribe` 开始语音识别");

    Ok(())
}

async fn download_model(target: &Path, model: &str) -> Result<()> {
    let marker = model_marker(model)
        .ok_or_else(|| anyhow::anyhow!("未知模型: {}", model))?;
    if target.join(marker).exists() {
        tracing::info!("模型 {model} 已存在，跳过下载");
        return Ok(());
    }

    let archive = model_archive(model).unwrap();
    let url = format!("{ASR_MODELS_BASE}/{archive}");
    tracing::info!("下载模型 {model} ...");

    let archive_path = target.join("_model.tar.bz2");
    let part_path = target.join("_model.tar.bz2.part");
    download_file(&url, &part_path).await?;
    tokio::fs::rename(&part_path, &archive_path).await
        .context("重命名临时模型文件失败")?;

    tracing::info!("  解压模型...");
    let archive_clone = archive_path.clone();
    let target_clone = target.to_path_buf();
    tokio::task::spawn_blocking(move || extract_model_tarbz2(&archive_clone, &target_clone))
        .await
        .context("模型解压线程异常")??;
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

/// 下载同音纠错文件 (lexicon.txt + replace.fst)
async fn download_hr_files(target: &Path) -> Result<()> {
    if target.join("replace.fst").exists() {
        tracing::info!("同音纠错文件已存在，跳过下载");
        return Ok(());
    }

    let base = hr_base_url();
    let files = ["lexicon.txt", "replace.fst"];

    for name in &files {
        let url = format!("{base}/{name}");
        let dest = target.join(name);
        let part = target.join(format!("{name}.part"));

        tracing::info!("下载同音纠错文件 {name}...");
        download_file(&url, &part).await?;
        tokio::fs::rename(&part, &dest).await
            .with_context(|| format!("重命名 {name} 失败"))?;
        tracing::info!("  保存: {}", dest.display());
    }

    Ok(())
}

pub async fn ensure_models(asr: &AsrConfig) -> Result<()> {
    let text_model = if asr.model.is_empty() { "sensevoice" } else { &asr.model };
    ensure_one_model(text_model).await?;

    let seg_model = if asr.segmentation_model.is_empty() {
        "sensevoice"
    } else {
        &asr.segmentation_model
    };
    if seg_model != text_model {
        ensure_one_model(seg_model).await?;
    }

    Ok(())
}

async fn ensure_one_model(model: &str) -> Result<()> {
    let home = vtrim_home();
    let marker = model_marker(model)
        .ok_or_else(|| anyhow::anyhow!("未知 ASR 模型: {} (可选: sensevoice / firered-zh / qwen3-asr)", model))?;
    let model_path = home.join("models").join(model);

    if !model_path.join(marker).exists() {
        tokio::fs::create_dir_all(&model_path).await?;
        tracing::info!("下载模型 {model} 到 {}", model_path.display());
        download_model(&model_path, model).await?;
    } else {
        tracing::info!("本地 ASR 模型已就绪 ({model})");
    }

    if model == "sensevoice" {
        // 静默下载同音纠错文件（不影响主流程）
        download_hr_files(&model_path).await?;
    }

    Ok(())
}
