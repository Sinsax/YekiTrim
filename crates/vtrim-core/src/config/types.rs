use serde::{Deserialize, Serialize};

fn default_local_model() -> String {
    "sensevoice".into()
}
fn default_output_dir() -> String {
    "./output".into()
}
fn default_temperature() -> f64 {
    0.1
}
fn default_max_tokens() -> u32 {
    65536
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProjectConfig {
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
}

impl ProjectConfig {
    /// 根据实际输出文件存在性推导当前进度
    pub fn current_stage(&self, output_dir: &std::path::Path) -> u32 {
        // 检查剪切输出（Stage 4）
        if output_dir.join("clips").join("clips_order.txt").exists() {
            return 4;
        }
        // 检查事件文件（Stage 3）
        let has_events = self.events_file.as_ref()
            .filter(|p| !p.is_empty())
            .map(std::path::PathBuf::from)
            .filter(|p| p.exists())
            .is_some();
        if has_events || output_dir.join("events.json").exists() {
            return 3;
        }
        // 检查转录文件（Stage 2）
        let has_transcript = self.transcript_file.as_ref()
            .filter(|p| !p.is_empty())
            .map(std::path::PathBuf::from)
            .filter(|p| p.exists())
            .is_some();
        if has_transcript || output_dir.join("transcript.json").exists() || output_dir.join("video.srt").exists() {
            return 2;
        }
        // 检查音频文件（Stage 1）
        let has_audio = self.audio_file.as_ref()
            .filter(|p| !p.is_empty())
            .map(std::path::PathBuf::from)
            .filter(|p| p.exists())
            .is_some();
        if has_audio || output_dir.join("video.wav").exists() {
            return 1;
        }
        0
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub project: ProjectConfig,
    #[serde(default)]
    pub audio: AudioConfig,
    #[serde(default)]
    pub asr: AsrConfig,
    #[serde(default)]
    pub correct: CorrectConfig,
    #[serde(default, skip)]
    pub llm: LlmConfig,
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

fn default_sample_rate() -> u32 { 16000 }
fn default_channels() -> u32 { 1 }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AsrConfig {
    #[serde(default = "default_local_model")]
    pub local_model: String,
    #[serde(default)]
    pub local_model_dir: String,
    #[serde(default = "default_num_workers")]
    pub num_workers: u32,
    #[serde(default = "default_chunk_duration")]
    pub chunk_duration: f64,
    #[serde(default = "default_chunk_overlap")]
    pub chunk_overlap: f64,
    #[serde(default = "default_segment_gap")]
    pub segment_gap: f64,
    #[serde(default)]
    pub llm_correct: Option<bool>,
    #[serde(default)]
    pub provider: String,
    #[serde(default = "default_asr_language")]
    pub language: String,
    #[serde(default)]
    pub vad: VadConfig,
}

fn default_asr_language() -> String { "zh".into() }
fn default_num_workers() -> u32 { 0 }
fn default_chunk_duration() -> f64 { 60.0 }
fn default_chunk_overlap() -> f64 { 10.0 }
fn default_segment_gap() -> f64 { 1.0 }

impl Default for AsrConfig {
    fn default() -> Self {
        Self {
            local_model: default_local_model(),
            local_model_dir: String::new(),
            num_workers: default_num_workers(),
            chunk_duration: default_chunk_duration(),
            chunk_overlap: default_chunk_overlap(),
            segment_gap: default_segment_gap(),
            llm_correct: None,
            provider: String::new(),
            language: default_asr_language(),
            vad: VadConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DetectionConfig {
    #[serde(default = "default_danmaku_window_secs")]
    pub danmaku_window_secs: f64,
    #[serde(default)]
    pub system_prompt: String,
    #[serde(default)]
    pub prompt_template: String,
    #[serde(default)]
    pub vtuber_name: String,
}

fn default_danmaku_window_secs() -> f64 { 10.0 }

impl Default for DetectionConfig {
    fn default() -> Self {
        Self {
            danmaku_window_secs: default_danmaku_window_secs(),
            system_prompt: String::new(),
            prompt_template: String::new(),
            vtuber_name: String::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmConfig {
    #[serde(default)]
    pub api_key: String,
    #[serde(default)]
    pub base_url: String,
    #[serde(default)]
    pub model: String,
    #[serde(default = "default_temperature")]
    pub temperature: f64,
    #[serde(default = "default_max_tokens")]
    pub max_tokens: u32,
    #[serde(default)]
    pub extra_params: Option<toml::Table>,
}

impl Default for LlmConfig {
    fn default() -> Self {
        Self {
            api_key: String::new(),
            base_url: String::new(),
            model: String::new(),
            temperature: default_temperature(),
            max_tokens: default_max_tokens(),
            extra_params: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CuttingConfig {
    #[serde(default = "default_output_dir")]
    pub output_dir: String,
    #[serde(default)]
    pub overwrite: Option<bool>,
    #[serde(default = "default_silence_noise_db")]
    pub silence_noise_db: f64,
}

fn default_silence_noise_db() -> f64 { -30.0 }

impl Default for CuttingConfig {
    fn default() -> Self {
        Self {
            output_dir: default_output_dir(),
            overwrite: None,
            silence_noise_db: default_silence_noise_db(),
        }
    }
}

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

fn default_vad_threshold() -> f64 { 0.5 }
fn default_vad_min_silence() -> f64 { 0.5 }
fn default_vad_min_speech() -> f64 { 0.25 }
fn default_vad_max_speech() -> f64 { 20.0 }

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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[derive(Default)]
pub struct CorrectConfig {
    #[serde(default)]
    pub system_prompt: String,
    #[serde(default)]
    pub prompt: String,
}

