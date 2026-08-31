use serde::{Deserialize, Serialize};

fn default_local_model() -> String {
    "sensevoice".into()
}
fn default_asr_model() -> String {
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
    #[serde(default)]
    pub sfx: SfxConfig,
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
    #[serde(default = "default_asr_language")]
    pub language: String,
    #[serde(default = "default_asr_model")]
    pub model: String,
    #[serde(default)]
    pub align_model: String,
    #[serde(default = "default_max_segment_duration")]
    pub max_segment_duration: f64,
    #[serde(default = "default_segment_merge_gap")]
    pub segment_merge_gap: f64,
    #[serde(default = "default_segment_padding")]
    pub segment_padding: f64,
    #[serde(default = "default_max_new_tokens")]
    pub max_new_tokens: u32,
    /// ASR 模式: "single"（SenseVoice 单模型）| "hybrid"（SenseVoice 打轴 + qwen3 填词）
    #[serde(default = "default_asr_mode")]
    pub mode: String,
}

fn default_asr_language() -> String { "zh".into() }
fn default_num_workers() -> u32 { 0 }
fn default_chunk_duration() -> f64 { 60.0 }
fn default_chunk_overlap() -> f64 { 10.0 }
fn default_segment_gap() -> f64 { 1.0 }
fn default_max_segment_duration() -> f64 { 15.0 }
fn default_segment_merge_gap() -> f64 { 0.6 }
fn default_segment_padding() -> f64 { 0.3 }
fn default_max_new_tokens() -> u32 { 2048 }
fn default_asr_mode() -> String { "single".into() }

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
            language: default_asr_language(),
            model: default_asr_model(),
            align_model: String::new(),
            max_segment_duration: default_max_segment_duration(),
            segment_merge_gap: default_segment_merge_gap(),
            segment_padding: default_segment_padding(),
            max_new_tokens: default_max_new_tokens(),
            mode: default_asr_mode(),
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
    /// 事件检测 LLM 最大并发窗口数（默认 3）
    #[serde(default = "default_detect_max_concurrent")]
    pub max_concurrent: usize,
}

fn default_danmaku_window_secs() -> f64 { 10.0 }
fn default_detect_max_concurrent() -> usize { 3 }

impl Default for DetectionConfig {
    fn default() -> Self {
        Self {
            danmaku_window_secs: default_danmaku_window_secs(),
            system_prompt: String::new(),
            prompt_template: String::new(),
            vtuber_name: String::new(),
            max_concurrent: default_detect_max_concurrent(),
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
pub struct SfxConfig {
    /// 音效屏蔽列表: 精确 id 屏蔽单个音效, "tag:xxx" 屏蔽整个标签族
    /// (vtrim-polish 使用, 全局层定义于此供跨工程共享)
    #[serde(default)]
    pub blacklist: Vec<String>,
}

impl Default for SfxConfig {
    fn default() -> Self {
        Self { blacklist: Vec::new() }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CorrectConfig {
    #[serde(default)]
    pub system_prompt: String,
    #[serde(default)]
    pub prompt: String,
    /// 是否启用思考模式（LLM 先推理再输出），默认 false
    #[serde(default)]
    pub thinking: bool,
    /// Agent 窗口大小（每次处理条数），默认 25
    #[serde(default = "default_correct_window_size")]
    pub window_size: usize,
    /// Agent 最大并发数，默认 3
    #[serde(default = "default_correct_max_concurrent")]
    pub max_concurrent: usize,
    /// 自定义热词（主播名/游戏/歌曲/梗等），优先修正为这些词
    #[serde(default)]
    pub hotwords: Vec<String>,
}

impl Default for CorrectConfig {
    fn default() -> Self {
        Self {
            system_prompt: String::new(),
            prompt: String::new(),
            thinking: false,
            window_size: default_correct_window_size(),
            max_concurrent: default_correct_max_concurrent(),
            hotwords: Vec::new(),
        }
    }
}

fn default_correct_window_size() -> usize { 25 }
fn default_correct_max_concurrent() -> usize { 3 }

