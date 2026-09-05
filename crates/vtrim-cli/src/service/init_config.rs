//! 初始化工程与配置管理 Service。

use std::path::{Path, PathBuf};

pub fn init_project(
    input: &Path,
    danmaku: Option<&Path>,
    output_dir: Option<&str>,
    cfg_path: &Path,
) -> anyhow::Result<()> {
    if input.is_dir() {
        anyhow::bail!("'{}' 是目录路径。vtrim init 需要视频文件路径", input.display());
    }
    let valid_ext = ["mp4", "mkv", "mov", "avi", "flv", "webm", "mts", "m2ts"];
    let ext = input.extension().and_then(|e| e.to_str()).map(|e| e.to_lowercase()).unwrap_or_default();
    if !valid_ext.contains(&ext.as_str()) {
        anyhow::bail!("'{}' 不是支持的视频格式。支持: {}", input.display(), valid_ext.join(", "));
    }
    if !input.exists() {
        anyhow::bail!("视频文件不存在: {}", input.display());
    }
    vtrim_core::config::write_project_config(cfg_path, input, danmaku, output_dir)?;
    Ok(())
}

pub fn init_global_config(path: Option<&Path>) -> anyhow::Result<()> {
    let out = path.map(PathBuf::from).unwrap_or_else(|| vtrim_core::path::vtrim_home().join("config.toml"));
    if let Some(parent) = out.parent() { std::fs::create_dir_all(parent)?; }
    let body = vtrim_core::config::GLOBAL_CONFIG_TOML
        .replace(r#""{{CORRECT_SYSTEM_PROMPT}}""#, &toml_val(vtrim_core::asr::correct::DEFAULT_SYSTEM_PROMPT))
        .replace(r#""{{CORRECT_USER_PROMPT}}""#, &toml_val(vtrim_core::asr::correct::DEFAULT_USER_PROMPT))
        .replace(r#""{{DETECT_SYSTEM_PROMPT}}""#, &toml_val(vtrim_core::detection::DEFAULT_SYSTEM_PROMPT))
        .replace(r#""{{DETECT_USER_PROMPT}}""#, &toml_val(vtrim_core::detection::DEFAULT_PROMPT));
    std::fs::write(&out, body)?;
    let llm_out = out.parent().map(|p| p.join("llm.toml")).unwrap_or_else(|| PathBuf::from(".vtrim/llm.toml"));
    std::fs::write(&llm_out, vtrim_core::config::LLM_CONFIG_TOML)?;
    std::fs::create_dir_all(vtrim_core::path::sticker_dir())?;
    tracing::info!("全局配置已生成: {}", out.display());
    Ok(())
}

pub fn show_config(config_file: Option<&Path>) -> anyhow::Result<String> {
    let cfg = if let Some(path) = config_file {
        let content = std::fs::read_to_string(path)?;
        toml::from_str(&content)?
    } else {
        vtrim_core::config::load_config()?
    };
    let llm = vtrim_core::config::load_llm_config();
    Ok(format!("{}\n\n# LLM 配置 (.vtrim/llm.toml)\n{}", toml::to_string_pretty(&cfg)?, toml::to_string_pretty(&llm)?))
}

fn toml_val(s: &str) -> String {
    let escaped = s.replace('\\', "\\\\").replace('\"', "\\\"");
    if s.contains('\n') {
        format!("\"\"\"\n{}\n\"\"\"", escaped)
    } else {
        format!("\"{}\"", escaped)
    }
}

pub async fn download_models() -> anyhow::Result<()> {
    tracing::info!("下载 ASR 模型 (sensevoice / firered-zh / qwen3-asr)...");
    vtrim_core::download::download_all(None).await
}
