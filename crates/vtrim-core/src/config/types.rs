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
        // `*_file` 是**相对工程根**写进 vtrim.toml 的（`vtrim init` 的 `path_to_rel`），
        // 所以相对路径要先按 output_dir 的父目录（= 工程根）拼一次再判存在性。
        // ⚠ 直接 `PathBuf::from(p).exists()` 是"相对进程 CWD"：`vtrim server` 是长驻进程，
        // CWD 与具体项目无关 —— 用户上传了外部字幕、又没跑过 ASR 时这个工程会被判成 stage 0
        // （重跑/门槛判断全跟着错）。
        // `Path::new("").join(x) == x`，所以 output_dir 本身是相对路径时（CLI 老口径）
        // 行为与从前逐字一致。
        let root = output_dir.parent();
        let exists = |raw: Option<&String>| -> bool {
            let Some(raw) = raw.filter(|p| !p.is_empty()) else {
                return false;
            };
            let p = std::path::PathBuf::from(raw);
            let p = if p.is_absolute() {
                p
            } else {
                match root {
                    Some(root) => root.join(&p),
                    None => p,
                }
            };
            p.exists()
        };
        // 检查剪切输出（Stage 4）
        if output_dir.join("clips").join("clips_order.txt").exists() {
            return 4;
        }
        // 检查事件文件（Stage 3）
        if exists(self.events_file.as_ref()) || output_dir.join("events.json").exists() {
            return 3;
        }
        // 检查转录文件（Stage 2）
        if exists(self.transcript_file.as_ref())
            || output_dir.join("transcript.json").exists()
            || output_dir.join("video.srt").exists()
        {
            return 2;
        }
        // 检查音频文件（Stage 1）
        if exists(self.audio_file.as_ref()) || output_dir.join("video.wav").exists() {
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
    #[serde(default)]
    pub chrome: ChromeConfig,
    /// 多用户服务：项目托管与容量（P2 起用 `projects_root`，P3/P5 用上限与水位）
    #[serde(default)]
    pub storage: StorageConfig,
    /// 多用户服务 P3：采集（录播姬 webhook + 兜底扫描）
    #[serde(default)]
    pub ingest: IngestConfig,
    /// 自助注册（邀请码）
    #[serde(default)]
    pub auth: AuthConfig,
}

/// `[auth]` — 账号与自助注册。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthConfig {
    /// 是否开放邀请码注册。默认关；`vtrim user invite` 生成码时置 true，
    /// `vtrim user invite --disable` 关闭。**开关打开也必须有可用邀请码**才能注册成功
    /// —— 码本身才是凭证，开关只是总闸。
    #[serde(default)]
    pub registration_enabled: bool,
}

impl Default for AuthConfig {
    fn default() -> Self {
        Self {
            registration_enabled: false,
        }
    }
}

/// `[ingest]` — 采集通道（录播姬 webhook + 兜底扫描）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IngestConfig {
    /// 是否启用 `POST /api/ingest/bililive/<token>`
    #[serde(default)]
    pub webhook_enabled: bool,
    /// webhook token 的 KDF 哈希串（明文只在 `vtrim ingest token` 生成时显示一次）
    #[serde(default)]
    pub token_hash: String,
    /// 来源 IP 白名单（空 = 不限制；录播姬与 server 同机时填 127.0.0.1）
    #[serde(default)]
    pub allow_ips: Vec<String>,
    #[serde(default)]
    pub recorder: RecorderConfig,
}

impl Default for IngestConfig {
    fn default() -> Self {
        Self {
            webhook_enabled: false,
            token_hash: String::new(),
            allow_ips: Vec::new(),
            recorder: RecorderConfig::default(),
        }
    }
}

/// `[ingest.recorder]` — 录播姬工作目录与入库策略。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecorderConfig {
    /// 录播姬输出目录（必须与 `vtrim server` 同机/同可读盘）
    #[serde(default)]
    pub root: String,
    /// 录播姬 HTTP API 地址（例 `http://127.0.0.1:2356`；桌面版默认 `http://127.0.0.1:2356`）。
    /// **空 = 未对接**：用户端不能自助添加直播间（只能走"待分配池"兜底）。
    #[serde(default)]
    pub api_base: String,
    /// 录播姬 HTTP Basic 用户名（对应容器环境变量 `BREC_HTTP_BASIC_USER`）
    #[serde(default)]
    pub api_user: String,
    /// 录播姬 HTTP Basic 密码（对应 `BREC_HTTP_BASIC_PASS`）
    #[serde(default)]
    pub api_pass: String,
    /// 兜底扫描间隔（分钟，0 = 关闭）：覆盖漏推送 / 录播姬重启 / 历史录播导入
    #[serde(default = "default_watch_interval_mins")]
    pub watch_interval_mins: u32,
    /// `SessionEnded` 之后再等多久没有新的 `FileClosed` 视为"本场结束"
    #[serde(default = "default_finalize_idle_mins")]
    pub finalize_idle_mins: u32,
    /// 落位策略：true = 移入托管项目目录（同盘 rename，跨盘 copy+校验+删源）
    #[serde(default = "default_true")]
    pub move_into_managed: bool,
    /// 多分片会话是否合并为单个 `source.mp4`（ffmpeg -f concat -c copy）
    #[serde(default = "default_true")]
    pub concat_files: bool,
    /// 认定为录像的扩展名
    #[serde(default = "default_recorder_extensions")]
    pub extensions: Vec<String>,
    /// 入库后自动执行到哪一步：none | transcribe | detect | cut（channel_map 可覆盖）
    #[serde(default = "default_auto_pipeline")]
    pub auto_pipeline: String,
}

fn default_watch_interval_mins() -> u32 {
    10
}
fn default_finalize_idle_mins() -> u32 {
    5
}
fn default_true() -> bool {
    true
}
fn default_recorder_extensions() -> Vec<String> {
    vec!["flv".into(), "mkv".into(), "mp4".into(), "ts".into()]
}
fn default_auto_pipeline() -> String {
    "detect".into()
}

impl Default for RecorderConfig {
    fn default() -> Self {
        Self {
            root: String::new(),
            api_base: String::new(),
            api_user: String::new(),
            api_pass: String::new(),
            watch_interval_mins: default_watch_interval_mins(),
            finalize_idle_mins: default_finalize_idle_mins(),
            move_into_managed: true,
            concat_files: true,
            extensions: default_recorder_extensions(),
            auto_pipeline: default_auto_pipeline(),
        }
    }
}

/// `[storage]` — 项目托管目录与磁盘上限（只在服务端生效，CLI 单机路径不受影响）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StorageConfig {
    /// 托管项目根目录；空 = `<exe_dir>/.vtrim/projects`
    #[serde(default)]
    pub projects_root: String,
    /// 全局磁盘上限（0 = 不限）；由服务端统一预检，绝不自动删用户数据
    #[serde(default)]
    pub max_disk_bytes: u64,
    /// 磁盘水位百分比：低于该比例时拒绝新的入库/下载/渲染
    #[serde(default = "default_watermark_pct")]
    pub disk_watermark_pct: u32,
    /// 单用户默认配额（0 = 不限）；`users.quota_bytes` 优先
    #[serde(default)]
    pub default_quota_bytes: u64,
    /// 管理员"从服务器本机文件新建项目"允许的根目录（绝对路径）。
    ///
    /// 默认空 = 只能用租户根（`<projects_root>/users/<uid>`）；填上例如
    /// `["F:/media", "D:/BililiveRecorder"]` 后，**仅管理员**可以在这几个根里
    /// 浏览并选目录建项目。任何情况下都**不接受上传**、不接受白名单之外的路径。
    #[serde(default)]
    pub local_roots: Vec<String>,
}

fn default_watermark_pct() -> u32 {
    10
}

impl Default for StorageConfig {
    fn default() -> Self {
        Self {
            projects_root: String::new(),
            max_disk_bytes: 0,
            disk_watermark_pct: default_watermark_pct(),
            default_quota_bytes: 0,
            local_roots: Vec::new(),
        }
    }
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

fn default_sample_rate() -> u32 {
    16000
}
fn default_channels() -> u32 {
    1
}

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
    /// **已废弃**（没有任何读取方，留着只为老 toml 不炸）：打轴模型已固定为 SenseVoice。
    #[serde(default)]
    pub align_model: String,
    /// **已废弃**（没有任何读取方）：单段时长上限从未生效，最长时长交给 LLM 按话题完整性决定。
    #[serde(default = "default_max_segment_duration")]
    pub max_segment_duration: f64,
    /// **已废弃**（没有任何读取方）：短段合并从未接上。
    #[serde(default = "default_segment_merge_gap")]
    pub segment_merge_gap: f64,
    /// 切给识别器的音频**前后余量**(秒)。**默认 0（关）**。
    ///
    /// 机制：窗口边界取自 SenseVoice 的逐字时间戳，从那一帧硬切会削掉首音，所以解码时
    /// 前后各留一点（前置余量只吃停顿、不碰上一个字的语音，见 `asr/mod.rs::decode_span`）。
    ///
    /// ⚠️ **为什么默认关**：0.3s 在真实音频上做过 A/B（`diagnose_head_padding`），
    /// 17 个窗口里 11 个文本变了，平均对齐命中率 **0.473 → 0.430**（更差），
    /// 换来的只有 2 个"整窗从空到有内容"。净效果不明确，所以不默认开 ——
    /// 想试就设 0.1~0.3，并用那个诊断在**你的**素材上复测。
    #[serde(default = "default_segment_padding")]
    pub segment_padding: f64,
    /// qwen3 单次解码的**全局输出上限**（token），固定值。
    ///
    /// 当前默认 256：30 字窗口实测正常输出约 20~40 token，余量充足；
    /// 同时比 512 更早截断音乐/噪声段的失控输出。
    /// 上游 sherpa-onnx 对 qwen3-asr 的默认是 **128**；如果以后调大
    /// `max_segment_chars`，需要同步调大这个值。
    /// 改默认值时要同步 `config/mod.rs` 的 merge 条件。
    #[serde(default = "default_max_new_tokens")]
    pub max_new_tokens: u32,
    /// ASR 模式: "single"（SenseVoice 单模型）| "hybrid"（SenseVoice 打轴 + qwen3 填词）
    ///
    /// 只有这两个值（firered-zh 已下线）：hybrid 的填词模型固定是 qwen3-asr。
    #[serde(default = "default_asr_mode")]
    pub mode: String,
    /// 气口阈值(秒)：相邻字之间的停顿 ≥ 此值算一个「气口」。
    ///
    /// 断句时**只用它来定位切点**（标点决定"在哪切"，气口决定"切在哪一刻"），
    /// 它本身不是独立切分信号 —— 实测词中间也有 0.30s 的停顿，单靠它会切碎。
    #[serde(default = "default_breath_gap")]
    pub breath_gap: f64,
    /// 句中标点（，、；：）软切的时间阈值：距上一条边界超过这么久才在这些标点处切。
    #[serde(default = "default_soft_cut_secs")]
    pub soft_cut_secs: f64,
    /// 单条字幕的**字数硬上限**（只在没有标点/气口可用时兜底）。
    #[serde(default = "default_max_segment_chars")]
    pub max_segment_chars: usize,
    /// 标点边界吸附到气口的半径(秒)。
    #[serde(default = "default_snap_window")]
    pub snap_window: f64,
}

fn default_asr_language() -> String {
    "zh".into()
}
fn default_num_workers() -> u32 {
    0
}
fn default_chunk_duration() -> f64 {
    60.0
}
fn default_chunk_overlap() -> f64 {
    10.0
}
fn default_segment_gap() -> f64 {
    1.0
}
fn default_max_segment_duration() -> f64 {
    15.0
}
fn default_segment_merge_gap() -> f64 {
    0.6
}
fn default_segment_padding() -> f64 {
    0.0
}
fn default_max_new_tokens() -> u32 {
    256
}
fn default_breath_gap() -> f64 {
    0.25
}
fn default_soft_cut_secs() -> f64 {
    6.0
}
fn default_max_segment_chars() -> usize {
    30
}
fn default_snap_window() -> f64 {
    0.3
}
/// ASR 模式**默认 hybrid**（用户要求固定用双模型；准备页不再给下拉，
/// 要改只能改 `.vtrim/config.toml` / 项目 `vtrim.toml` 的 `[asr] mode`，即设置页）。
fn default_asr_mode() -> String {
    "hybrid".into()
}

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
            breath_gap: default_breath_gap(),
            soft_cut_secs: default_soft_cut_secs(),
            max_segment_chars: default_max_segment_chars(),
            snap_window: default_snap_window(),
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
    /// 参考标题示例（检测 + 封面标题生成共用；每项一条，可含 emoji）
    #[serde(default)]
    pub reference_titles: Vec<String>,
    /// 标题+摘要 LLM 传播潜力评分在最终热度里的权重。
    ///
    /// 评分阶段只把 `label` + `summary` 发给 LLM（不带转录/时间/类型），
    /// 按预测点击率、有趣程度、爆点强度综合给 0~100 分；最终热度：
    /// `popularity = (1 - w) * 弹幕/字幕热度 + w * LLM评分`。
    /// `0` = 关闭该标准（也不再发这次 LLM 请求）。
    #[serde(default = "default_llm_score_weight")]
    pub llm_score_weight: f64,
    /// 事件检测并发窗口数。**留空 = 跟随 `.vtrim/llm.toml` 的 `[llm] max_concurrent`**；
    /// 填了才以本段为准（一般是排查限流时才需要单独压这里）。
    #[serde(default)]
    pub max_concurrent: Option<usize>,
}

fn default_danmaku_window_secs() -> f64 {
    10.0
}
fn default_llm_score_weight() -> f64 {
    0.3
}

impl Default for DetectionConfig {
    fn default() -> Self {
        Self {
            danmaku_window_secs: default_danmaku_window_secs(),
            system_prompt: String::new(),
            prompt_template: String::new(),
            vtuber_name: String::new(),
            reference_titles: Vec::new(),
            llm_score_weight: default_llm_score_weight(),
            max_concurrent: None,
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
    /// 并发请求数——**所有 LLM 调用方共用这一个旋钮**（事件检测 / 精修事件生成 / 字幕订正）。
    /// 各段自己的 `max_concurrent` 留空即跟随这里；填了则那一段以自己的为准。
    #[serde(default = "default_llm_max_concurrent")]
    pub max_concurrent: usize,
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
            max_concurrent: default_llm_max_concurrent(),
        }
    }
}

fn default_llm_max_concurrent() -> usize {
    3
}

impl LlmConfig {
    /// 并发数的唯一解析入口：段自己的覆盖优先，否则用 `[llm] max_concurrent`，至少 1。
    /// 检测 / 精修 / 订正三条链路都走这里，别再各写一份 `.max(1)`。
    pub fn concurrency(&self, section_override: Option<usize>) -> usize {
        section_override.unwrap_or(self.max_concurrent).max(1)
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

fn default_silence_noise_db() -> f64 {
    -30.0
}

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
        Self {
            blacklist: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChromeConfig {
    /// 自定义 Chrome/Chromium 可执行文件路径（留空自动检测）
    #[serde(default)]
    pub path: String,
}

impl Default for ChromeConfig {
    fn default() -> Self {
        Self {
            path: String::new(),
        }
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
    /// 订正并发数。**留空 = 跟随 `.vtrim/llm.toml` 的 `[llm] max_concurrent`**。
    #[serde(default)]
    pub max_concurrent: Option<usize>,
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
            max_concurrent: None,
            hotwords: Vec::new(),
        }
    }
}

fn default_correct_window_size() -> usize {
    25
}

#[cfg(test)]
mod current_stage_tests {
    use super::*;
    use std::path::PathBuf;

    /// 自建临时目录（vtrim-core 不依赖 tempfile；pid + 纳秒保证并发测试不撞名）。
    struct TmpDir(PathBuf);

    impl TmpDir {
        fn new(tag: &str) -> Self {
            let nanos = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0);
            let p = std::env::temp_dir()
                .join(format!("vtrim-stage-{tag}-{}-{nanos}", std::process::id()));
            std::fs::create_dir_all(&p).unwrap();
            TmpDir(p)
        }

        fn path(&self) -> &std::path::Path {
            &self.0
        }
    }

    impl Drop for TmpDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// 配置里的 `*_file` 是相对**工程根**（= output_dir 的父目录）写的，
    /// 不是相对进程 CWD —— 只有外部字幕、没跑过 ASR 的工程必须判成 stage 2。
    #[test]
    fn relative_project_files_resolve_against_project_root() {
        let dir = TmpDir::new("root");
        let root = dir.path();
        std::fs::create_dir_all(root.join("output")).unwrap();
        let output = root.join("output");
        assert_eq!(ProjectConfig::default().current_stage(&output), 0);

        std::fs::write(
            root.join("sub.srt"),
            "1\n00:00:01,000 --> 00:00:02,000\nhi\n",
        )
        .unwrap();
        let cfg = ProjectConfig {
            transcript_file: Some("sub.srt".to_string()),
            ..Default::default()
        };
        assert_eq!(cfg.current_stage(&output), 2);

        // 子目录里的素材同样算数
        std::fs::create_dir_all(root.join("sub")).unwrap();
        std::fs::write(root.join("sub/events.json"), "[]").unwrap();
        let cfg = ProjectConfig {
            transcript_file: Some("sub.srt".to_string()),
            events_file: Some("sub/events.json".to_string()),
            ..Default::default()
        };
        assert_eq!(cfg.current_stage(&output), 3);

        // 指到不存在的文件不能冒充已完成
        let cfg = ProjectConfig {
            transcript_file: Some("nope.srt".to_string()),
            ..Default::default()
        };
        assert_eq!(cfg.current_stage(&output), 0);
    }

    /// output/ 里的产物判据不受影响。
    #[test]
    fn output_artifacts_still_drive_stage() {
        let dir = TmpDir::new("artifacts");
        let output = dir.path().join("output");
        std::fs::create_dir_all(output.join("clips")).unwrap();
        std::fs::write(output.join("video.wav"), b"x").unwrap();
        let cfg = ProjectConfig::default();
        assert_eq!(cfg.current_stage(&output), 1);
        std::fs::write(output.join("video.srt"), "1\n").unwrap();
        assert_eq!(cfg.current_stage(&output), 2);
        std::fs::write(output.join("events.json"), "[]").unwrap();
        assert_eq!(cfg.current_stage(&output), 3);
        std::fs::write(output.join("clips/clips_order.txt"), "").unwrap();
        assert_eq!(cfg.current_stage(&output), 4);
    }
}
