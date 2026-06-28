use serde::{Deserialize, Serialize};

fn default_sample_rate() -> u32 {
    16000
}
fn default_channels() -> u32 {
    1
}
fn default_provider() -> String {
    "openai".into()
}
fn default_llm_model() -> String {
    "gpt-4o-mini".into()
}
fn default_silence_threshold() -> f64 {
    3.0
}
fn default_min_segment() -> f64 {
    30.0
}
fn default_max_segment() -> f64 {
    900.0
}
fn default_temperature() -> f64 {
    0.1
}
fn default_max_tokens() -> u32 {
    4096
}
fn default_split_silence_secs() -> f64 {
    10.0
}
fn default_vad_threshold() -> f64 {
    0.5
}
fn default_vad_min_silence() -> f64 {
    0.5
}
fn default_vad_min_speech() -> f64 {
    0.25
}
fn default_vad_max_speech() -> f64 {
    20.0
}
fn default_batch_size() -> usize {
    25
}
fn default_batch_overlap() -> usize {
    3
}
fn default_local_model() -> String {
    "sensevoice".into()
}
fn default_min_chunk_secs() -> f64 {
    15.0
}
fn default_true() -> bool {
    true
}
fn default_max_parallel() -> usize {
    4
}
fn default_output_dir() -> String {
    "./output".into()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub input_file: Option<String>,
    #[serde(default)]
    pub danmaku_file: Option<String>,
    #[serde(default)]
    pub transcript_file: Option<String>,
    #[serde(default)]
    pub audio_file: Option<String>,
    #[serde(default)]
    pub events_file: Option<String>,
    #[serde(default)]
    pub vocal_file: Option<String>,
    #[serde(default)]
    pub current_stage: u32,
    #[serde(default)]
    pub audio: AudioConfig,
    #[serde(default)]
    pub asr: AsrConfig,
    #[serde(default)]
    pub detection: DetectionConfig,
    #[serde(default)]
    pub cutting: CuttingConfig,
}

impl Default for Config {
    fn default() -> Self {
        toml::from_str("").expect("default config should parse")
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioConfig {
    #[serde(default = "default_sample_rate")]
    pub sample_rate: u32,
    #[serde(default = "default_channels")]
    pub channels: u32,
}

impl Default for AudioConfig {
    fn default() -> Self {
        Self {
            sample_rate: default_sample_rate(),
            channels: default_channels(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AsrConfig {
    #[serde(default = "default_asr_backend")]
    pub backend: String,
    #[serde(default = "default_split_silence_secs")]
    pub split_silence_secs: f64,
    #[serde(default = "default_local_model")]
    pub local_model: String,
    #[serde(default)]
    pub local_model_dir: String,
    #[serde(default = "default_min_chunk_secs")]
    pub min_chunk_secs: f64,
    #[serde(default)]
    pub vad: VadConfig,
    #[serde(default = "default_num_threads")]
    pub num_threads: u32,
    #[serde(default = "default_chunk_duration")]
    pub chunk_duration: f64,
    #[serde(default = "default_chunk_overlap")]
    pub chunk_overlap: f64,
    #[serde(default = "default_separation")]
    pub separation: String,
}

fn default_num_threads() -> u32 { 0 }
fn default_chunk_duration() -> f64 { 60.0 }
fn default_chunk_overlap() -> f64 { 10.0 }
fn default_separation() -> String { "none".into() }

impl Default for AsrConfig {
    fn default() -> Self {
        Self {
            backend: default_asr_backend(),
            split_silence_secs: default_split_silence_secs(),
            local_model: default_local_model(),
            local_model_dir: String::new(),
            min_chunk_secs: default_min_chunk_secs(),
            vad: VadConfig::default(),
            num_threads: default_num_threads(),
            chunk_duration: default_chunk_duration(),
            chunk_overlap: default_chunk_overlap(),
            separation: default_separation(),
        }
    }
}

fn default_asr_backend() -> String { "local".into() }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VadConfig {
    #[serde(default = "default_vad_threshold")]
    pub threshold: f64,
    #[serde(default = "default_vad_min_silence")]
    pub min_silence_duration: f64,
    #[serde(default = "default_vad_min_speech")]
    pub min_speech_duration: f64,
    #[serde(default = "default_vad_max_speech")]
    pub max_speech_duration: f64,
}

impl Default for VadConfig {
    fn default() -> Self {
        Self {
            threshold: default_vad_threshold(),
            min_silence_duration: default_vad_min_silence(),
            min_speech_duration: default_vad_min_speech(),
            max_speech_duration: default_vad_max_speech(),
        }
    }
}

fn default_danmaku_window_secs() -> f64 { 10.0 }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DetectionConfig {
    #[serde(default = "default_danmaku_window_secs")]
    pub danmaku_window_secs: f64,
    #[serde(default = "default_silence_threshold")]
    pub silence_threshold_secs: f64,
    #[serde(default = "default_min_segment")]
    pub min_segment_secs: f64,
    #[serde(default = "default_max_segment")]
    pub max_segment_secs: f64,
    #[serde(default)]
    pub llm: LlmConfig,
}

impl Default for DetectionConfig {
    fn default() -> Self {
        Self {
            danmaku_window_secs: default_danmaku_window_secs(),
            silence_threshold_secs: default_silence_threshold(),
            min_segment_secs: default_min_segment(),
            max_segment_secs: default_max_segment(),
            llm: LlmConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmConfig {
    #[serde(default = "default_provider")]
    pub provider: String,
    #[serde(default)]
    pub api_key: String,
    #[serde(default = "default_llm_model")]
    pub model: String,
    #[serde(default = "default_llm_base_url")]
    pub api_base: String,
    #[serde(default = "default_temperature")]
    pub temperature: f64,
    #[serde(default = "default_max_tokens")]
    pub max_tokens: u32,
    #[serde(default = "default_batch_size")]
    pub batch_size: usize,
    #[serde(default = "default_batch_overlap")]
    pub batch_overlap: usize,
}

impl Default for LlmConfig {
    fn default() -> Self {
        Self {
            provider: default_provider(),
            api_key: String::new(),
            model: default_llm_model(),
            api_base: default_llm_base_url(),
            temperature: default_temperature(),
            max_tokens: default_max_tokens(),
            batch_size: default_batch_size(),
            batch_overlap: default_batch_overlap(),
        }
    }
}

fn default_llm_base_url() -> String {
    String::new()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CuttingConfig {
    #[serde(default = "default_true")]
    pub preserve_source: bool,
    #[serde(default = "default_max_parallel")]
    pub max_parallel: usize,
    #[serde(default = "default_output_dir")]
    pub output_dir: String,
    #[serde(default)]
    pub overwrite: bool,
}

impl Default for CuttingConfig {
    fn default() -> Self {
        Self {
            preserve_source: default_true(),
            max_parallel: default_max_parallel(),
            output_dir: default_output_dir(),
            overwrite: false,
        }
    }
}
