//! config::types：全站配置类型定义（`Config` / `ProjectConfig` 与各 section）。
//!
//! 入口：`Config`、`ProjectConfig`（`current_stage`）、`AsrConfig`、`DetectionConfig`、`LlmConfig`（`concurrency`）、`CuttingConfig`、`MaterialEntry`、`Materials`。
//! 边界：所有类型必须**手动**实现 `Default`（不能 derive），新增字段要同时改 `merge()` 与 `config/default.toml` —— 见 `crates/vtrim-core/src/config/AGENTS.md`

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
    /// 预处理页的**素材清单**（列表里能一键切换的候选项）。
    #[serde(default)]
    pub materials: Materials,
}

/// 预处理页「素材清单」：用户在列表里加过、能一键切换的候选。
///
/// ⚠️ 与 `transcript_file` / `danmaku_file` 的分工：那两个是**当前生效值**，
/// 下游所有读取入口（`Workspace::transcript_path` / `danmaku_path`、检测、切片）只认它们；
/// 这里只回答「列表里有哪些候选、各自哪来的」。
///
/// **ASR 链路产物**（`source = "asr"`）按模式占槽：`[asr] mode` 只有 hybrid / single 两种，
/// 所以最多两条，且不可删除 —— 这就是"输出一次绑定一次、只能两次"的落地形态。
/// 用户自己加的字幕/弹幕（`source` 为空或其它值）不占槽、可随时删除。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Materials {
    #[serde(default)]
    pub transcript: Vec<MaterialEntry>,
    #[serde(default)]
    pub danmaku: Vec<MaterialEntry>,
}

impl Materials {
    /// 某一类的条目（`"danmaku"` / 其它一律当 `"transcript"`）。
    pub fn of(&self, kind: &str) -> &[MaterialEntry] {
        if kind == "danmaku" {
            &self.danmaku
        } else {
            &self.transcript
        }
    }

    /// 某一类条目的可变引用。
    pub fn of_mut(&mut self, kind: &str) -> &mut Vec<MaterialEntry> {
        if kind == "danmaku" {
            &mut self.danmaku
        } else {
            &mut self.transcript
        }
    }
}

/// ASR 的**全部模式**（`[asr] mode` 只有这两个值）。
///
/// 素材清单里 ASR 产物**按模式占槽** —— 这里就是用户说的「输出一次绑定一次、
/// 只能两次」里的那个 2。加模式要同步改这里（和预处理页的下拉）。
pub const ASR_MODES: [&str; 2] = ["hybrid", "single"];

/// 素材清单里的一条。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MaterialEntry {
    /// 路径（相对**工程根**；工程外只可能是 admin，那时存绝对路径）。
    #[serde(default)]
    pub path: String,
    /// 来源：`"asr"` = ASR 链路产物（不可删、占一个模式槽）。
    ///
    /// 用 `String` 而不是 enum 是有意的：手改 toml 写错一个词**不该让整份配置解析失败**
    /// —— `load_config_at` 一旦报错，这个工程在服务端就整个不可用。未知值一律当用户素材。
    #[serde(default)]
    pub source: String,
    /// ASR 模式槽位：`hybrid` / `single`（只有 `source = "asr"` 的条目有）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
}

/// `source` 字段里代表「ASR 链路产物」的那个值。
pub const MATERIAL_SOURCE_ASR: &str = "asr";

impl MaterialEntry {
    pub fn asr(path: String, mode: &str) -> Self {
        Self {
            path,
            source: MATERIAL_SOURCE_ASR.to_string(),
            mode: Some(mode.trim().to_ascii_lowercase()),
        }
    }

    pub fn user(path: String) -> Self {
        Self {
            path,
            source: String::new(),
            mode: None,
        }
    }

    /// 是不是 ASR 链路产物（不可删、占模式槽）。
    pub fn is_asr(&self) -> bool {
        self.source.eq_ignore_ascii_case(MATERIAL_SOURCE_ASR)
    }

    /// 占用的 ASR 模式槽（小写；非 ASR 条目或没记模式时为空串 = 不占槽）。
    pub fn slot(&self) -> String {
        if !self.is_asr() {
            return String::new();
        }
        self.mode
            .as_deref()
            .unwrap_or("")
            .trim()
            .to_ascii_lowercase()
    }
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
    /// **B站链接通道（P4）**：下载器引擎。
    ///
    /// `"yt-dlp"` = 内置引擎（版本 + sha256 钉死在 `yekitrim_service::downloader::registry`，
    /// **运行时**由设置页/`ensure_ytdlp` 下到 `<程序目录>/.vtrim/bin/` —— 和 ASR 模型一样，
    /// **不进发行包**，所以许可声明走 `THIRD-PARTY-NOTICES.md` 的"运行时下载"那一节）；
    /// 其它值 = **逃生舱**：用户自备的「yt-dlp 兼容」可执行文件**绝对路径**
    /// （同样不随包分发，`cargo xtask scan` 扫不到）。
    /// 加第二个引擎 = 加一个适配器，server 侧不改（见 `yekitrim_service::downloader`）。
    #[serde(default = "default_downloader")]
    pub downloader: String,
    /// yt-dlp 可执行文件路径覆盖（留空 = 自动发现：配置目录 → `<程序目录>/.vtrim/bin/` → PATH）。
    ///
    /// ⚠️ 与 `downloader` 的分工：`downloader` 选**引擎**，这里只覆盖**二进制位置**
    /// （例如想让内置引擎用你自己更新的那份 yt-dlp）。
    #[serde(default)]
    pub ytdlp_path: String,
    /// 最大清晰度：`"1080p"` / `"720p"` / `"480p"` / `"best"`（`best` = 不限高）
    #[serde(default = "default_max_quality")]
    pub max_quality: String,
    /// 限速（KB/s，0 = 不限）→ `--limit-rate <n>K`
    #[serde(default)]
    pub rate_limit_kbps: u64,
    /// 同时下载数。**默认 1，别放大**：B站 412 风控与 IP 频次相关（选型文档 §8.5）。
    #[serde(default = "default_max_concurrent_downloads")]
    pub max_concurrent_downloads: u32,
    /// B站 cookie 文件（**Netscape 格式**；JSON 格式会被 yt-dlp 静默忽略）。
    /// 仅 admin 可写：无 cookie 时清晰度封顶 480/720p，部分视频直接 `NeedLogin`。
    #[serde(default)]
    pub cookie_file: String,
    /// 下载"卡死"判定：连续这么多秒**一行输出都没有**就 kill 并标失败。
    ///
    /// ⚠️ 这是**唯一的进程兜底**。`jobs` 那层只管 async 任务体，管不到子进程 ——
    /// 没有它，一个挂死的 yt-dlp 会永久占住下载槽（会话停在 `downloading`）。
    ///
    /// 别因为"大文件本来就慢"而调大：卡死不是慢，是**一行不出**（进度行也算输出）。
    /// B站的 WBI/风控偶尔会让请求吊住，120s 足够区分"慢"与"死"。
    #[serde(default = "default_download_idle_timeout")]
    pub download_idle_timeout_secs: u32,
}

fn default_downloader() -> String {
    "yt-dlp".into()
}
fn default_max_quality() -> String {
    "1080p".into()
}
fn default_max_concurrent_downloads() -> u32 {
    1
}
fn default_download_idle_timeout() -> u32 {
    120
}

impl Default for IngestConfig {
    fn default() -> Self {
        Self {
            webhook_enabled: false,
            token_hash: String::new(),
            allow_ips: Vec::new(),
            recorder: RecorderConfig::default(),
            downloader: default_downloader(),
            ytdlp_path: String::new(),
            max_quality: default_max_quality(),
            rate_limit_kbps: 0,
            max_concurrent_downloads: default_max_concurrent_downloads(),
            cookie_file: String::new(),
            download_idle_timeout_secs: default_download_idle_timeout(),
        }
    }
}

/// 清晰度档位 → 下载器 `-f` 里的高度上限（`None` = 不限）。
///
/// 放在 core 而不是下载器适配层：**它是配置语义**（`[ingest] max_quality` 的取值域），
/// 前端下拉、服务端校验、命令构造三处要认同一张表。
pub fn quality_height(raw: &str) -> Option<u32> {
    let s = raw.trim().to_ascii_lowercase();
    if s.is_empty() || s == "best" {
        return None;
    }
    s.trim_end_matches('p').parse::<u32>().ok().filter(|h| *h > 0)
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
    /// 磁盘水位百分比：**写完这一份之后**剩余空间低于该比例就拒绝新的入库/下载/渲染。
    ///
    /// 注意它不是"必须空出整盘的百分之几"：换算出的保留量会被钳在 **512 MiB ~ 8 GiB**
    /// （见 `yekitrim_service::ingest::capacity::reserve_bytes`）。比例只对中小容量卷起作用，
    /// 大容量数据盘上不会平白保留出几十上百 GB。`0` = 关闭水位检查。
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
    /// **single 模式**要不要先用 VAD（silero）只挑出语音段解码。
    ///
    /// 为什么值钱：SenseVoice 是非自回归模型，代价与喂进去的音频长度成正比，而固定分块会把
    /// 静音/BGM 一起送进去（解码量 = 音频 × 1.2）。实测三场真实录播"真有字"的时间只占 33~43%。
    ///
    /// `None`（没写这一项）= 跟随默认（开）；写 `true`/`false` 才是显式覆盖 ——
    /// 用 `Option` 而不是 `bool` 是因为 merge 的口径是"≠默认值才覆盖"，
    /// 那样全局关掉之后就**再也**没法在单个工程里打开。
    #[serde(default)]
    pub vad: Option<bool>,
    /// silero VAD 的语音概率阈值（0~1）。调高 = 更保守（多解码、少省时间），调低反之。
    #[serde(default = "default_vad_threshold")]
    pub vad_threshold: f64,
    /// 动态电平兜底：低于「本片语音电平 − 此值」的空档才允许跳过（dB，0 = 关掉兜底）。
    ///
    /// 兜底只做加法（VAD 判非语音、但电平够响的空档照样解码回来），所以它的失效方向是
    /// "多花点时间"，而不是"字幕少一条" —— 这是与单模型效果对齐的结构性保证。
    #[serde(default = "default_vad_rescue_db")]
    pub vad_rescue_db: f64,
}

impl AsrConfig {
    /// VAD 是否生效（`single` 模式）：没写这一项按开。
    pub fn vad_enabled(&self) -> bool {
        self.vad.unwrap_or(true)
    }
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
/// silero 官方默认阈值就是 0.5。
fn default_vad_threshold() -> f64 {
    0.5
}
/// 动态电平兜底的宽容度（dB）。
///
/// 判据是"比本片语音电平低一档"，所以对不同增益的录播（手机麦 / 声卡 / 录播姬直出）都成立。
/// **默认 16**：实测 184 分钟真实录播上，8dB 会漏掉几句短促的插话（"好冷啊空调关""哪有啊"），
/// 16dB 把它们捞回来，代价只是解码量 +1.6%（2.12× → 2.03×）。偏大只会多解码
/// （最坏退化成固定分块 = 现在的行为），偏小才会漏语音 —— 所以宁可偏大。
fn default_vad_rescue_db() -> f64 {
    16.0
}
/// ASR 模式**默认 hybrid**（用户要求默认双模型）。预处理页有下拉可改（写项目
/// `vtrim.toml`），设置页/项目配置也能改；读不到配置时按这个默认值显示。
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
            vad: None,
            vad_threshold: default_vad_threshold(),
            vad_rescue_db: default_vad_rescue_db(),
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

/// 链路标识：决定 `.vtrim/llm.toml` 的 `[llm.overrides.*]` 里哪一组覆盖生效，
/// 也是设置页「按链路覆盖」小表的行、日志与用量统计里能对上的那条链路。
///
/// `key()` 是 TOML 键名（英文，写进配置文件）；`label_zh()` 只给界面看。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LlmLink {
    /// 事件检测窗口 + 标题/摘要热度评分（同一次检测的两次批量调用）
    Detect,
    /// 精修 LLM 剪辑：分段、逐段事件、贴纸精挑、关键词
    Polish,
    /// 字幕订正
    Correct,
    /// 零散单发：贴纸包/扫描标签、封面文案、连通测试
    Utility,
}

impl LlmLink {
    /// 设置页那张表的行顺序 = 这个顺序。
    pub const ALL: [LlmLink; 4] = [
        LlmLink::Detect,
        LlmLink::Polish,
        LlmLink::Correct,
        LlmLink::Utility,
    ];

    /// TOML 键名（`[llm.overrides.<key>]`）。
    pub fn key(self) -> &'static str {
        match self {
            LlmLink::Detect => "detect",
            LlmLink::Polish => "polish",
            LlmLink::Correct => "correct",
            LlmLink::Utility => "utility",
        }
    }

    /// 界面文案。
    pub fn label_zh(self) -> &'static str {
        match self {
            LlmLink::Detect => "事件检测",
            LlmLink::Polish => "精修剪辑",
            LlmLink::Correct => "字幕订正",
            LlmLink::Utility => "零散单发",
        }
    }

    /// 该链路的**代码级默认** thinking（都没配时的兜底）。
    ///
    /// - `Polish` / `Utility` = `disabled`：精修分段/逐段事件、贴纸包标签、封面文案
    ///   历史上就在代码里写死了关思考，这里把它收成一处；
    /// - `Detect` = `None`：检测从来没强制过，交给 `extra_params.thinking`；
    /// - `Correct` = `None`：订正另有 `[correct] thinking` 老开关（见 `AgentTask::thinking`），
    ///   LLM 层没表态时它照旧生效。
    pub fn default_thinking(self) -> Option<&'static str> {
        match self {
            LlmLink::Detect | LlmLink::Correct => None,
            LlmLink::Polish | LlmLink::Utility => Some("disabled"),
        }
    }
}

/// 单条链路的 LLM 调用参数覆盖；**字段缺席 = 跟随 `[llm]` 的同名项**。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LlmOverride {
    #[serde(default)]
    pub max_tokens: Option<u32>,
    #[serde(default)]
    pub temperature: Option<f64>,
    /// `"enabled"` / `"disabled"`；其它值原样透传给 provider。
    #[serde(default)]
    pub thinking: Option<String>,
}

/// 按链路的覆盖表（`[llm.overrides]`）。未知键忽略，与其余配置类型一致 ——
/// 一个拼错的键不该让整份 llm.toml 被打回默认值。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LlmOverrides {
    #[serde(default)]
    pub detect: LlmOverride,
    #[serde(default)]
    pub polish: LlmOverride,
    #[serde(default)]
    pub correct: LlmOverride,
    #[serde(default)]
    pub utility: LlmOverride,
}

impl LlmOverrides {
    pub fn of(&self, link: LlmLink) -> &LlmOverride {
        match link {
            LlmLink::Detect => &self.detect,
            LlmLink::Polish => &self.polish,
            LlmLink::Correct => &self.correct,
            LlmLink::Utility => &self.utility,
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
    /// 全局思考开关。`None` = 不设置（由 `extra_params.thinking` 决定）。
    /// ⚠️ `build_body` 里 `self.thinking` 会**盖掉** `extra_params.thinking`，所以这里是更高优先级的开关。
    #[serde(default)]
    pub thinking: Option<String>,
    /// 按链路的调用参数覆盖（见 [`LlmConfig::for_link`]）。
    #[serde(default)]
    pub overrides: LlmOverrides,
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
            thinking: None,
            overrides: LlmOverrides::default(),
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

    /// 某链路的**有效** LLM 调用参数 —— **所有构造 `LlmClient` 的地方都必须走这里**，
    /// 别再裸 `from_config` + 手写 `with_max_tokens` / `with_temperature` / `with_thinking`
    /// （漏一个调用点 = 那个旋钮静默不生效）。
    ///
    /// 优先级（高 → 低）：
    /// - `max_tokens`：`overrides.<link>.max_tokens` → `[llm].max_tokens`（`0` 视为未设置）
    /// - `temperature`：`overrides.<link>.temperature` → `[llm].temperature`（非有限值忽略，夹 [0,2]）
    /// - `thinking`：`overrides.<link>.thinking` → `[llm].thinking` → [`LlmLink::default_thinking`]
    ///   → 都没给就不设置（此时 `extra_params.thinking` 生效）
    pub fn for_link(&self, link: LlmLink) -> LlmConfig {
        let ov = self.overrides.of(link);

        let max_tokens = match ov.max_tokens {
            // 0 = 没配（设置页的输入框清空后会写成 null，但不排除手改出 0）：别把 0 发给 provider
            Some(0) => {
                tracing::warn!(
                    "[llm.overrides.{}] max_tokens = 0 无效，按未设置处理（跟随 [llm].max_tokens = {}）",
                    link.key(),
                    self.max_tokens
                );
                self.max_tokens
            }
            Some(n) => n,
            None => self.max_tokens,
        };

        let temperature = match ov.temperature {
            Some(t) if t.is_finite() => t.clamp(0.0, 2.0),
            Some(t) => {
                tracing::warn!(
                    "[llm.overrides.{}] temperature = {t} 不是有限值，按未设置处理",
                    link.key()
                );
                self.temperature
            }
            None => self.temperature,
        };

        let thinking = ov
            .thinking
            .clone()
            .or_else(|| self.thinking.clone())
            .or_else(|| link.default_thinking().map(|s| s.to_string()));

        LlmConfig {
            max_tokens,
            temperature,
            thinking,
            ..self.clone()
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
mod llm_link_tests {
    use super::*;

    fn base() -> LlmConfig {
        let mut c = LlmConfig::default();
        c.model = "m".into();
        c.max_tokens = 16384;
        c.temperature = 0.1;
        c
    }

    /// 没配覆盖 = 逐项跟随 `[llm]`（既有安装升级上来行为不变）。
    #[test]
    fn for_link_inherits_global_when_no_override() {
        let c = base();
        let d = c.for_link(LlmLink::Detect);
        assert_eq!(d.max_tokens, 16384);
        assert_eq!(d.temperature, 0.1);
        assert_eq!(d.model, "m");
        assert_eq!(d.max_concurrent, c.max_concurrent);
        // 检测不强制思考（历史上就没有写死），交给 extra_params
        assert_eq!(d.thinking, None);
    }

    /// 覆盖 > 全局，且只影响自己那条链路。
    #[test]
    fn for_link_applies_override_only_to_that_link() {
        let mut c = base();
        c.overrides.detect.max_tokens = Some(8192);
        c.overrides.detect.temperature = Some(0.7);
        c.overrides.detect.thinking = Some("enabled".into());

        let d = c.for_link(LlmLink::Detect);
        assert_eq!(d.max_tokens, 8192);
        assert!((d.temperature - 0.7).abs() < 1e-9);
        assert_eq!(d.thinking.as_deref(), Some("enabled"));

        let p = c.for_link(LlmLink::Polish);
        assert_eq!(p.max_tokens, 16384, "别的链路不受影响");
        assert!((p.temperature - 0.1).abs() < 1e-9);
    }

    /// 链路代码默认：精修/零散单发关思考（历史写死的那两处），可被覆盖或全局顶掉。
    #[test]
    fn link_thinking_defaults_and_precedence() {
        let mut c = base();
        assert_eq!(c.for_link(LlmLink::Polish).thinking.as_deref(), Some("disabled"));
        assert_eq!(c.for_link(LlmLink::Utility).thinking.as_deref(), Some("disabled"));
        assert_eq!(c.for_link(LlmLink::Correct).thinking, None);

        // 链路覆盖 > 全局
        c.thinking = Some("enabled".into());
        assert_eq!(c.for_link(LlmLink::Polish).thinking.as_deref(), Some("enabled"));
        assert_eq!(c.for_link(LlmLink::Detect).thinking.as_deref(), Some("enabled"));
        c.overrides.polish.thinking = Some("disabled".into());
        assert_eq!(c.for_link(LlmLink::Polish).thinking.as_deref(), Some("disabled"));
    }

    /// 手改坏的值不能让整条链路炸：0 = 未设置，非有限 temperature 忽略，超范围夹回来。
    #[test]
    fn for_link_ignores_bogus_values() {
        let mut c = base();
        c.overrides.detect.max_tokens = Some(0);
        c.overrides.detect.temperature = Some(f64::NAN);
        let d = c.for_link(LlmLink::Detect);
        assert_eq!(d.max_tokens, 16384);
        assert_eq!(d.temperature, 0.1);

        c.overrides.correct.temperature = Some(5.0);
        assert_eq!(c.for_link(LlmLink::Correct).temperature, 2.0);
    }

    /// TOML 形态（= 用户手写的那张表）；未知键忽略，不把整份配置打回默认。
    #[test]
    fn parses_overrides_table_from_toml() {
        let text = r#"
api_key = "k"
max_tokens = 16384
thinking = "disabled"

[overrides.detect]
max_tokens = 8192
[overrides.polish]
max_tokens = 32768
temperature = 0.2
thinking = "enabled"
"#;
        let c: LlmConfig = toml::from_str(text).unwrap();
        assert_eq!(c.for_link(LlmLink::Detect).max_tokens, 8192);
        assert_eq!(c.for_link(LlmLink::Polish).max_tokens, 32768);
        assert_eq!(c.for_link(LlmLink::Polish).thinking.as_deref(), Some("enabled"));
        // utility 缺席 → 跟随全局
        assert_eq!(c.for_link(LlmLink::Utility).max_tokens, 16384);

        let typo: LlmConfig =
            toml::from_str("api_key = \"k\"\n[overrides.detectt]\nmax_tokens = 1\n").unwrap();
        assert_eq!(typo.api_key, "k");
        assert_eq!(
            typo.for_link(LlmLink::Detect).max_tokens,
            LlmConfig::default().max_tokens,
            "拼错的表名只被忽略，不该生效也不该撑大范围"
        );
    }

    /// 枚举 ↔ TOML 键名是契约（设置页与配置文件都按这个走）。
    #[test]
    fn link_keys_are_stable() {
        let keys: Vec<&str> = LlmLink::ALL.iter().map(|l| l.key()).collect();
        assert_eq!(keys, vec!["detect", "polish", "correct", "utility"]);
    }
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
