mod types;
pub use types::*;

use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::Deserialize;

fn load_from_file(path: &PathBuf) -> Result<Option<Config>> {
    if !path.exists() {
        return Ok(None);
    }
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read {}", path.display()))?;
    let config: Config =
        toml::from_str(&content).with_context(|| format!("failed to parse {}", path.display()))?;
    Ok(Some(config))
}

fn merge(base: &mut Config, overlay: Config) {
    // Project fields
    if overlay.project.input_file.is_some() {
        base.project.input_file.clone_from(&overlay.project.input_file);
    }
    if overlay.project.danmaku_file.is_some() {
        base.project.danmaku_file.clone_from(&overlay.project.danmaku_file);
    }
    if overlay.project.transcript_file.is_some() {
        base.project.transcript_file.clone_from(&overlay.project.transcript_file);
    }
    if overlay.project.audio_file.is_some() {
        base.project.audio_file.clone_from(&overlay.project.audio_file);
    }
    if overlay.project.events_file.is_some() {
        base.project.events_file.clone_from(&overlay.project.events_file);
    }

    if overlay.audio.sample_rate != 16000 {
        base.audio.sample_rate = overlay.audio.sample_rate;
    }
    if overlay.audio.channels != 1 {
        base.audio.channels = overlay.audio.channels;
    }

    if !overlay.asr.local_model.is_empty() {
        base.asr.local_model = overlay.asr.local_model;
    }
    if !overlay.asr.local_model_dir.is_empty() {
        base.asr.local_model_dir = overlay.asr.local_model_dir;
    }
    if overlay.asr.num_workers != 0 {
        base.asr.num_workers = overlay.asr.num_workers;
    }
    if overlay.asr.chunk_duration != 60.0 {
        base.asr.chunk_duration = overlay.asr.chunk_duration;
    }
    if overlay.asr.chunk_overlap != 10.0 {
        base.asr.chunk_overlap = overlay.asr.chunk_overlap;
    }
    if overlay.asr.segment_gap != 1.0 {
        base.asr.segment_gap = overlay.asr.segment_gap;
    }
    if let Some(v) = overlay.asr.llm_correct {
        base.asr.llm_correct = Some(v);
    }
    if !overlay.asr.provider.is_empty() {
        base.asr.provider = overlay.asr.provider.clone();
    }
    if !overlay.asr.language.is_empty() {
        base.asr.language = overlay.asr.language.clone();
    }
    if overlay.detection.danmaku_window_secs != 10.0 {
        base.detection.danmaku_window_secs = overlay.detection.danmaku_window_secs;
    }
    if !overlay.detection.system_prompt.is_empty() {
        base.detection.system_prompt = overlay.detection.system_prompt;
    }
    if !overlay.detection.prompt_template.is_empty() {
        base.detection.prompt_template = overlay.detection.prompt_template;
    }
    if !overlay.detection.vtuber_name.is_empty() {
        base.detection.vtuber_name = overlay.detection.vtuber_name;
    }

    if !overlay.correct.system_prompt.is_empty() {
        base.correct.system_prompt = overlay.correct.system_prompt;
    }
    if !overlay.correct.prompt.is_empty() {
        base.correct.prompt = overlay.correct.prompt;
    }

    if !overlay.cutting.output_dir.is_empty() {
        base.cutting.output_dir = overlay.cutting.output_dir;
    }
    if let Some(v) = overlay.cutting.overwrite {
        base.cutting.overwrite = Some(v);
    }
}

/// 从 .vtrim/llm.toml 加载 LLM 配置（独立于工程配置和全局配置）
pub fn load_llm_config() -> LlmConfig {
    let path = crate::path::vtrim_home().join("llm.toml");
    if !path.exists() {
        return LlmConfig::default();
    }
    let content = match std::fs::read_to_string(&path) {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!("读取 {} 失败: {}，使用默认 LLM 配置", path.display(), e);
            return LlmConfig::default();
        }
    };
    toml::from_str::<LlmFile>(&content).map(|f| f.llm).unwrap_or_else(|e| {
        tracing::warn!("解析 {} 失败: {}，使用默认 LLM 配置", path.display(), e);
        LlmConfig::default()
    })
}

#[derive(Deserialize)]
struct LlmFile {
    llm: LlmConfig,
}

pub fn load_config() -> Result<Config> {
    let mut config = Config::default();

    // 优先级: 先加载全局配置，再用 vtrim.toml 覆盖（vtrim.toml 优先级更高）
    let global = crate::path::vtrim_home().join("config.toml");
    if let Some(overlay) = load_from_file(&global)? {
        merge(&mut config, overlay);
    }

    let local = PathBuf::from("vtrim.toml");
    if let Some(overlay) = load_from_file(&local)? {
        merge(&mut config, overlay);
    }

    // 独立加载 LLM 配置（llm.toml 已从 config.toml 分离）
    config.llm = load_llm_config();

    Ok(config)
}

/// 用 toml_edit 将 events_file 写入 vtrim.toml
pub fn save_events_path(path: &str) -> Result<()> {
    edit_toml(|doc| {
        doc["project"]["events_file"] = toml_edit::value(path);
    })
}

/// 用 toml_edit 将 audio_file 写入 vtrim.toml
pub fn save_audio_path(path: &str) -> Result<()> {
    edit_toml(|doc| {
        doc["project"]["audio_file"] = toml_edit::value(path);
    })
}

/// 用 toml_edit 将 transcript_file 写入 vtrim.toml
pub fn save_transcript_path(path: &str) -> Result<()> {
    edit_toml(|doc| {
        doc["project"]["transcript_file"] = toml_edit::value(path);
    })
}

fn edit_toml(f: impl FnOnce(&mut toml_edit::DocumentMut)) -> Result<()> {
    let path = PathBuf::from("vtrim.toml");
    let content = if path.exists() {
        std::fs::read_to_string(&path)?
    } else {
        String::new()
    };
    let mut doc: toml_edit::DocumentMut = content
        .parse()
        .unwrap_or_else(|_| toml_edit::DocumentMut::new());
    f(&mut doc);
    std::fs::write(&path, doc.to_string())?;
    Ok(())
}
