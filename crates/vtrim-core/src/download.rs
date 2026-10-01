//! download：ASR 模型下载与就位检查（`vtrim download` 与转写前的 `ensure_models`）。
//!
//! 入口：`model_names`、`model_install`、`download_all`、`download_vad_model`、`ensure_models`；
//! 另有两个**通用**下载件给别处复用：`fetch_url_to` / `fetch_text_to`（自带字体下载用，见 `fonts::BUILTIN_FONTS`）。
//! 边界：模型落在 `<程序目录>/.vtrim/models/`（debug / release 各一份，互不相干）；许可上别把 espeak-ng 等 GPL 组件带回来 —— 见 `crates/vtrim-core/AGENTS.md`

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

/// silero VAD 权重（裸 onnx 文件，不走 tar.bz2 归档那条路）。
///
/// 只下 int8（209KB）：判定质量与 fp32（629KB）在实听上无可见差别，体积小一个量级。
/// 想换 fp32 就把 `silero_vad.onnx` 丢进同一个目录 —— `asr::vad::find_vad_model`
/// 两个文件名都认，且优先 int8。
const VAD_MODEL_FILES: &[(&str, &str)] = &[(
    "silero_vad.int8.onnx",
    "https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/silero_vad.int8.onnx",
)];

/// 下载 silero VAD 权重到 `<models_dir>/silero-vad/`（已有则跳过）。
pub async fn download_vad_model(models_dir: &Path) -> Result<()> {
    let dir = models_dir.join(crate::asr::vad::MODEL_DIR_NAME);
    if crate::asr::vad::find_vad_model(&dir).is_some() {
        tracing::info!("silero VAD 模型已存在，跳过下载");
        return Ok(());
    }
    tokio::fs::create_dir_all(&dir)
        .await
        .context("创建 VAD 模型目录失败")?;
    for (name, url) in VAD_MODEL_FILES {
        let dest = dir.join(name);
        if dest.exists() {
            continue;
        }
        let part = dir.join(format!("{name}.part"));
        tracing::info!("下载 silero VAD 模型 {name} ...");
        download_file(url, &part).await?;
        tokio::fs::rename(&part, &dest)
            .await
            .with_context(|| format!("重命名 {name} 失败"))?;
        tracing::info!("  保存: {}", dest.display());
    }
    Ok(())
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
    // VAD 是 single 模式的加速器：一起下（209KB），别让用户"以为开了 VAD 其实没有"
    download_vad_model(&home.join("models")).await?;

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

    // ⚠️ 别 `unwrap()`：`model_marker` 与 `model_archive` 是两张表，用户手改
    // `[asr] model` 后完全可能只命中一张 —— 那是配置错误，该报错而不是 panic。
    let archive = model_archive(model)
        .ok_or_else(|| anyhow::anyhow!("未知模型: {}", model))?;
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

/// **通用**下载：把 `url` 的内容落到 `dest`（先写 `dest`，调用方自己决定用不用 `.part`）。
///
/// 与 `download_file` 的区别：那个是"模型那条路"的带续传 + 进度条实现（写 `.part` 再改名），
/// 这个是给**别的调用方**（自带字体）用的最小件 —— 同一套 `reqwest` 与错误口径，不再各写一份。
/// ⚠️ 目标目录必须已存在（调用方建），失败**不留半个文件**。
pub async fn fetch_url_to(url: &str, dest: &Path) -> Result<()> {
    let resp = reqwest::Client::new()
        .get(url)
        .send()
        .await
        .with_context(|| format!("请求失败: {url}"))?;
    let status = resp.status();
    if !status.is_success() {
        bail!("下载返回 {} ({url})", status);
    }
    let bytes = resp.bytes().await.with_context(|| format!("读响应体失败: {url}"))?;
    if bytes.is_empty() {
        bail!("下载到 0 字节 ({url})");
    }
    tokio::fs::write(dest, &bytes)
        .await
        .with_context(|| format!("写入失败: {}", dest.display()))?;
    Ok(())
}

/// **通用**下载：取一段**文本**（许可原文这类）落到 `dest`。
///
/// 与 [`fetch_url_to`] 分开是因为"文本"与"二进制"的失败语义不同：文本拿不到通常只该警告
/// （字体本体已校验过、合法），不该把整件事判失败。
pub async fn fetch_text_to(url: &str, dest: &Path) -> Result<()> {
    let text = reqwest::Client::new()
        .get(url)
        .send()
        .await
        .with_context(|| format!("请求失败: {url}"))?
        .error_for_status()
        .with_context(|| format!("下载返回非 2xx: {url}"))?
        .text()
        .await
        .with_context(|| format!("读文本失败: {url}"))?;
    if text.trim().is_empty() {
        bail!("下载到空文本 ({url})");
    }
    tokio::fs::write(dest, text.as_bytes())
        .await
        .with_context(|| format!("写入失败: {}", dest.display()))?;
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
    for name in required_models(asr, model) {
        ensure_one_model(asr, name).await?;
    }
    // VAD 权重同样要在转写前就位：不然跑到转写里才发现没有，静默回退固定分块 ——
    // 用户以为 VAD 生效了、实际一点没省。失败只告警（VAD 是加速器，不是必要条件）。
    if asr.vad_enabled() && asr.mode.trim().eq_ignore_ascii_case("single") {
        if let Err(e) = ensure_vad_model(asr).await {
            tracing::warn!("silero VAD 模型准备失败（{e:#}），本次转写将回退固定分块");
        }
    }
    Ok(())
}

/// 确保 VAD 权重落在**运行时真正会读的**目录里（与 `vad_model_path` 同一套查找顺序）。
async fn ensure_vad_model(asr: &AsrConfig) -> Result<()> {
    let (dir, source) = crate::asr::local::resolve_model_dir(asr, crate::asr::vad::MODEL_DIR_NAME);
    if crate::asr::vad::find_vad_model(&dir).is_some() {
        tracing::info!("silero VAD 已就绪（{}）", dir.display());
        return Ok(());
    }
    tokio::fs::create_dir_all(&dir).await?;
    tracing::info!(
        "下载 silero VAD 模型到 {}（{}）",
        dir.display(),
        source.label()
    );
    download_vad_model(dir.parent().unwrap_or(&dir)).await?;
    if crate::asr::vad::find_vad_model(&dir).is_none() {
        bail!("VAD 模型下载后仍未找到（{}）", dir.display());
    }
    Ok(())
}

/// 这次转写**真正要读**的模型（判据与 `asr/mod.rs` 的 `mode == "single"` 一致）。
///
/// `single` 只用 SenseVoice 直出；其余（含空值）= hybrid：SenseVoice 打轴 +
/// **成片字幕的文本来自 qwen3-asr** —— 缺后者时 `create_qwen3_recognizer` 会在
/// 打轴**跑完之后**才失败，而普通用户无权在设置页下载模型，等于白等一场。
/// 所以两个都要在同一处确保（这也是转写任务占着 `models` 资源键的由来）。
fn required_models<'a>(asr: &AsrConfig, model: &'a str) -> Vec<&'a str> {
    let mut out = vec![model];
    if asr.mode.trim() != "single" && model != "qwen3-asr" {
        out.push("qwen3-asr");
    }
    out
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

    /// 转写前要确保的模型随 `[asr] mode` 变：hybrid 缺 qwen3-asr 就是「打轴跑完才失败」。
    ///
    /// 回归钉子：`ensure_models` 曾经只按 `[asr] model`（默认 sensevoice）下载，而
    /// hybrid 成片字幕的文本来自 qwen3-asr → 全新部署上第一次转写必然在 SenseVoice
    /// 打轴之后报"创建 Qwen3-ASR recognizer 失败"，普通用户又无权在设置页下载。
    #[test]
    fn hybrid_also_requires_qwen3_model() {
        let mut cfg = AsrConfig::default();
        assert_eq!(cfg.mode, "hybrid", "默认模式变了就要同步这条测试");
        assert_eq!(required_models(&cfg, "sensevoice"), ["sensevoice", "qwen3-asr"]);

        // 空值按 hybrid 走（与 `asr/mod.rs` 的 `mode == "single"` 判据一致）
        cfg.mode = String::new();
        assert_eq!(required_models(&cfg, "sensevoice"), ["sensevoice", "qwen3-asr"]);

        // single 只要 SenseVoice
        cfg.mode = "single".into();
        assert_eq!(required_models(&cfg, "sensevoice"), ["sensevoice"]);

        // `[asr] model = qwen3-asr` 时别重复下载同一份
        cfg.mode = "hybrid".into();
        assert_eq!(required_models(&cfg, "qwen3-asr"), ["qwen3-asr"]);
    }
}
