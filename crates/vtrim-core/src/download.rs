use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use reqwest::StatusCode;
use tokio::io::AsyncWriteExt;

use crate::config::AsrConfig;
use crate::path::vtrim_home;

const ASR_MODELS_BASE: &str = "https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models";

/// 支持的 ASR 模型: (配置名, 模型目录名, 下载文件名, 已安装标记文件)
///
/// 只有真正有推理入口的模型才在这里：`single` 用 sensevoice、`hybrid` 用
/// sensevoice 打轴 + qwen3-asr 填词（见 `asr::local` / `asr::mod` 的模式分派）。
/// firered-zh 已下线（CTC 时间戳在 BGM 段造幽灵时间戳，见 ROADMAP）。
const MODEL_REGISTRY: &[(&str, &str, &str, &str)] = &[
    (
        "sensevoice",
        "sensevoice",
        "sherpa-onnx-sense-voice-zh-en-ja-ko-yue-int8-2025-09-09.tar.bz2",
        "tokens.txt",
    ),
    (
        "qwen3-asr",
        "qwen3-asr",
        "sherpa-onnx-qwen3-asr-0.6B-int8-2026-03-25.tar.bz2",
        "tokenizer/tokenizer_config.json",
    ),
];

/// 注册表里的模型名（界面/诊断列表的唯一来源，别再各处手写数组）。
pub fn model_names() -> Vec<&'static str> {
    MODEL_REGISTRY.iter().map(|(cfg, _, _, _)| *cfg).collect()
}

/// 一个模型的安装情况（**运行时视角**）。
pub struct ModelInstall {
    /// 运行时实际会用的目录（`asr::resolve_model_dir` 的解析结果）
    pub dir: PathBuf,
    /// 这个目录是从哪一路找来的
    pub source: crate::asr::local::ModelDirSource,
    /// 目录里是否已经有注册表要求的标记文件
    pub installed: bool,
}

/// 查某个模型是否可用 + 用的是哪一份。
///
/// ⚠️ 判定必须与运行时（`asr::local::resolve_model_dir`）**同一套查找顺序**，
/// 否则会出现"模型管理说没下载、转写其实一直在用用户目录那份"的错位；
/// `installed` 认的是注册表 marker，不是"目录存在"（半下载的目录不算装好）。
pub fn model_install(cfg: &AsrConfig, model: &str) -> ModelInstall {
    let (dir, source) = crate::asr::local::resolve_model_dir(cfg, model);
    let installed = model_marker(model).map(|m| dir.join(m).exists()).unwrap_or(false);
    ModelInstall { dir, source, installed }
}

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
    let configured = if asr.model.is_empty() { "sensevoice" } else { asr.model.as_str() };
    let model = if model_marker(configured).is_some() {
        configured
    } else {
        // 老配置里可能还写着已下线的模型（如 firered-zh）：只告警并回退，
        // 别把整个转写任务直接打死（`create_recognizer` 本来就固定用 SenseVoice）。
        tracing::warn!(
            "[asr] model = {configured:?} 不是受支持的模型（可选: {}），这次按 sensevoice 走",
            model_names().join(" / ")
        );
        "sensevoice"
    };
    ensure_one_model(asr, model).await?;
    Ok(())
}

/// 确保**运行时真正会读的那一份**模型目录存在且完整。
///
/// ⚠️ 目录一律走 `model_install`（= 运行时的 `asr::resolve_model_dir`），
/// **不要**用 `vtrim_home()`。曾经这里直接拼程序目录，而运行时优先用户目录，后果有两个：
///   ① 同一份模型被下载两次（程序目录那份永远不会被读）；
///   ② 同音纠错文件（`lexicon.txt` + `replace.fst`）下到程序目录，而识别器读的是用户目录
///      那份 → **同音纠错从未生效**（`create_recognizer` 只看到"没有这两个文件"）。
/// 这正是上面 `model_install` 注释里要求"与运行时同一套查找顺序"的那条。
async fn ensure_one_model(asr: &AsrConfig, model: &str) -> Result<()> {
    if model_marker(model).is_none() {
        anyhow::bail!(
            "未知 ASR 模型: {} (可选: {})",
            model,
            model_names().join(" / ")
        );
    }
    let install = model_install(asr, model);
    let model_path = install.dir.clone();

    if !install.installed {
        tokio::fs::create_dir_all(&model_path).await?;
        tracing::info!(
            "下载模型 {model} 到 {}（{}）",
            model_path.display(),
            install.source.label()
        );
        download_model(&model_path, model).await?;
    } else {
        tracing::info!("本地 ASR 模型已就绪 ({model}, {})", model_path.display());
    }

    if model == "sensevoice" {
        // 静默下载同音纠错文件（不影响主流程）——下到同一个被读的目录
        download_hr_files(&model_path).await?;
    }

    Ok(())
}

#[cfg(test)]
mod model_install_tests {
    use super::*;

    struct TmpDir(PathBuf);

    impl TmpDir {
        fn new(tag: &str) -> Self {
            let base = std::env::temp_dir().join(format!(
                "vtrim-model-{tag}-{}-{:?}",
                std::process::id(),
                std::thread::current().id()
            ));
            std::fs::create_dir_all(&base).unwrap();
            TmpDir(base)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TmpDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn cfg_with(local_model_dir: &Path) -> AsrConfig {
        AsrConfig {
            local_model_dir: local_model_dir.to_string_lossy().into_owned(),
            ..Default::default()
        }
    }

    /// 界面/诊断的"已安装"必须和运行时**同一套查找顺序**（配置 → 用户目录 → 程序目录），
    /// 而且认的是注册表 marker：目录在、marker 不在 = 没装好（下到一半的 `.part` 目录）。
    #[test]
    fn model_install_follows_runtime_lookup_and_marker() {
        let t = TmpDir::new("marker");
        let models = t.path().join("models");
        let qwen = models.join("qwen3-asr");
        std::fs::create_dir_all(&qwen).unwrap();

        let cfg = cfg_with(&models);
        // 目录在、marker 不在 → 未安装（旧实现只看 dir.exists()，会把半下载判成已装）
        let st = model_install(&cfg, "qwen3-asr");
        assert!(!st.installed, "没有 marker 不应算已安装");
        assert!(st.dir.ends_with("qwen3-asr"), "{}", st.dir.display());

        // 补上注册表要求的 marker → 已安装，且来源是配置目录
        let tk = qwen.join("tokenizer");
        std::fs::create_dir_all(&tk).unwrap();
        std::fs::write(tk.join("tokenizer_config.json"), b"{}").unwrap();
        let st = model_install(&cfg, "qwen3-asr");
        assert!(st.installed);
        assert_eq!(st.source, crate::asr::local::ModelDirSource::Config);

        // sensevoice 的 marker 是 tokens.txt
        let sv = models.join("sensevoice");
        std::fs::create_dir_all(&sv).unwrap();
        assert!(!model_install(&cfg, "sensevoice").installed);
        std::fs::write(sv.join("tokens.txt"), b"a 1\n").unwrap();
        assert!(model_install(&cfg, "sensevoice").installed);

        // 未知模型 → 不 panic、判未安装
        assert!(!model_install(&cfg, "firered-zh").installed);
        assert!(!model_names().contains(&"firered-zh"));
    }
}
