mod types;
pub use types::*;

use std::path::PathBuf;

use anyhow::{Context, Result};

/// 阶段编号常量
pub const STAGE_AUDIO: u32 = 1;
pub const STAGE_TRANSCRIBE: u32 = 2;
pub const STAGE_DETECT: u32 = 3;
pub const STAGE_CUT: u32 = 4;

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

    if overlay.asr.backend != "local" {
        base.asr.backend = overlay.asr.backend.clone();
    }
    if !overlay.asr.local_model.is_empty() {
        base.asr.local_model = overlay.asr.local_model;
    }
    if !overlay.asr.local_model_dir.is_empty() {
        base.asr.local_model_dir = overlay.asr.local_model_dir;
    }
    if overlay.asr.num_threads != 0 {
        base.asr.num_threads = overlay.asr.num_threads;
    }
    if overlay.asr.chunk_duration != 60.0 {
        base.asr.chunk_duration = overlay.asr.chunk_duration;
    }
    if overlay.asr.chunk_overlap != 10.0 {
        base.asr.chunk_overlap = overlay.asr.chunk_overlap;
    }
    if overlay.asr.llm_correct {
        base.asr.llm_correct = true;
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

    if !overlay.llm.api_key.is_empty() {
        base.llm.api_key = overlay.llm.api_key;
    }
    if !overlay.llm.api_base.is_empty() {
        base.llm.api_base = overlay.llm.api_base;
    }
    if !overlay.llm.model.is_empty() {
        base.llm.model = overlay.llm.model;
    }
    if overlay.llm.temperature != 0.1 {
        base.llm.temperature = overlay.llm.temperature;
    }
    if overlay.llm.max_tokens != 16384 {
        base.llm.max_tokens = overlay.llm.max_tokens;
    }

    if !overlay.cutting.output_dir.is_empty() {
        base.cutting.output_dir = overlay.cutting.output_dir;
    }
    if overlay.cutting.overwrite {
        base.cutting.overwrite = true;
    }
}

pub fn load_config() -> Result<Config> {
    let mut config = Config::default();

    // 优先级: 先加载全局配置，再用 vtrim.toml 覆盖（vtrim.toml 优先级更高）
    let global = crate::download::vtrim_home().join("config.toml");
    if let Some(overlay) = load_from_file(&global)? {
        merge(&mut config, overlay);
    }

    let local = PathBuf::from("vtrim.toml");
    if let Some(overlay) = load_from_file(&local)? {
        config.project.current_stage = overlay.project.current_stage;
        config.project.input_file.clone_from(&overlay.project.input_file);
        merge(&mut config, overlay);
    }

    Ok(config)
}

/// 从 vtrim.toml 读取 current_stage
pub fn load_current_stage() -> Result<u32> {
    let path = PathBuf::from("vtrim.toml");
    if !path.exists() {
        return Ok(0);
    }
    let content = std::fs::read_to_string(&path)?;
    let value: toml::Value = toml::from_str(&content)?;
    Ok(value
        .get("project")
        .and_then(|v| v.get("current_stage"))
        .and_then(|v| v.as_integer())
        .unwrap_or(0) as u32)
}

/// 用 toml_edit 将 current_stage 写入 vtrim.toml
pub fn save_current_stage(stage: u32) -> Result<()> {
    edit_toml(|doc| {
        doc["project"]["current_stage"] = toml_edit::value(stage as i64);
    })
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
