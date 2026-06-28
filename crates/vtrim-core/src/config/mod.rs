mod types;
pub use types::*;

use std::path::PathBuf;

use anyhow::{Context, Result};

/// 阶段编号常量
pub const STAGE_AUDIO: u32 = 1;
pub const STAGE_TRANSCRIBE: u32 = 3;
pub const STAGE_DETECT: u32 = 4;
pub const STAGE_CUT: u32 = 5;

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
    if overlay.audio.sample_rate != 16000 {
        base.audio.sample_rate = overlay.audio.sample_rate;
    }
    if overlay.audio.channels != 1 {
        base.audio.channels = overlay.audio.channels;
    }

    if overlay.asr.split_silence_secs != 10.0 {
        base.asr.split_silence_secs = overlay.asr.split_silence_secs;
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
    if overlay.asr.min_chunk_secs != 15.0 {
        base.asr.min_chunk_secs = overlay.asr.min_chunk_secs;
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
    if overlay.asr.vad.threshold != 0.5 {
        base.asr.vad.threshold = overlay.asr.vad.threshold;
    }
    if overlay.asr.vad.min_silence_duration != 0.5 {
        base.asr.vad.min_silence_duration = overlay.asr.vad.min_silence_duration;
    }
    if overlay.asr.vad.min_speech_duration != 0.25 {
        base.asr.vad.min_speech_duration = overlay.asr.vad.min_speech_duration;
    }
    if overlay.asr.vad.max_speech_duration != 20.0 {
        base.asr.vad.max_speech_duration = overlay.asr.vad.max_speech_duration;
    }

    if overlay.detection.danmaku_window_secs != 10.0 {
        base.detection.danmaku_window_secs = overlay.detection.danmaku_window_secs;
    }
    if overlay.detection.silence_threshold_secs != 3.0 {
        base.detection.silence_threshold_secs = overlay.detection.silence_threshold_secs;
    }
    if overlay.detection.min_segment_secs != 30.0 {
        base.detection.min_segment_secs = overlay.detection.min_segment_secs;
    }
    if overlay.detection.max_segment_secs != 900.0 {
        base.detection.max_segment_secs = overlay.detection.max_segment_secs;
    }

    if !overlay.detection.llm.provider.is_empty() {
        base.detection.llm.provider = overlay.detection.llm.provider;
    }
    if !overlay.detection.llm.api_key.is_empty() {
        base.detection.llm.api_key = overlay.detection.llm.api_key;
    }
    if !overlay.detection.llm.model.is_empty() {
        base.detection.llm.model = overlay.detection.llm.model;
    }
    if !overlay.detection.llm.api_base.is_empty() {
        base.detection.llm.api_base = overlay.detection.llm.api_base.clone();
    }
    if overlay.detection.llm.temperature != 0.1 {
        base.detection.llm.temperature = overlay.detection.llm.temperature;
    }
    if overlay.detection.llm.max_tokens != 4096 {
        base.detection.llm.max_tokens = overlay.detection.llm.max_tokens;
    }
    if overlay.detection.llm.batch_size != 25 {
        base.detection.llm.batch_size = overlay.detection.llm.batch_size;
    }
    if overlay.detection.llm.batch_overlap != 3 {
        base.detection.llm.batch_overlap = overlay.detection.llm.batch_overlap;
    }

    if overlay.input_file.is_some() {
        base.input_file.clone_from(&overlay.input_file);
    }
    if overlay.danmaku_file.is_some() {
        base.danmaku_file.clone_from(&overlay.danmaku_file);
    }
    if overlay.transcript_file.is_some() {
        base.transcript_file.clone_from(&overlay.transcript_file);
    }
    if overlay.audio_file.is_some() {
        base.audio_file.clone_from(&overlay.audio_file);
    }
    if overlay.events_file.is_some() {
        base.events_file.clone_from(&overlay.events_file);
    }

    if !overlay.cutting.preserve_source {
        base.cutting.preserve_source = false;
    }
    if overlay.cutting.max_parallel != 4 {
        base.cutting.max_parallel = overlay.cutting.max_parallel;
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

    let local = PathBuf::from("vtrim.toml");
    if let Some(overlay) = load_from_file(&local)? {
        config.current_stage = overlay.current_stage;
        config.input_file.clone_from(&overlay.input_file);
        merge(&mut config, overlay);
    }

    let global = crate::download::vtrim_home().join("config.toml");
    if let Some(overlay) = load_from_file(&global)? {
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
        .get("current_stage")
        .and_then(|v| v.as_integer())
        .unwrap_or(0) as u32)
}

/// 用 toml_edit 将 current_stage 写入 vtrim.toml，保留注释和格式
pub fn save_current_stage(stage: u32) -> Result<()> {
    edit_toml(|doc| {
        doc["current_stage"] = toml_edit::value(stage as i64);
    })
}

/// 用 toml_edit 将 events_file 写入 vtrim.toml
pub fn save_events_path(path: &str) -> Result<()> {
    edit_toml(|doc| {
        doc["events_file"] = toml_edit::value(path);
    })
}

/// 用 toml_edit 将 audio_file 写入 vtrim.toml
pub fn save_audio_path(path: &str) -> Result<()> {
    edit_toml(|doc| {
        doc["audio_file"] = toml_edit::value(path);
    })
}

/// 用 toml_edit 将 transcript_file 写入 vtrim.toml
pub fn save_transcript_path(path: &str) -> Result<()> {
    edit_toml(|doc| {
        doc["transcript_file"] = toml_edit::value(path);
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
