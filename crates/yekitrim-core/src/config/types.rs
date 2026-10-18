//! config::types：全站配置类型定义（`Config` / `ProjectConfig` 与各 section）。
//!
//! 入口：`Config`、`ProjectConfig`（`current_stage`）、`AsrConfig`、`DetectionConfig`、`LlmConfig`（`concurrency`）、`CuttingConfig`、`MaterialEntry`、`Materials`。
//! 边界：所有类型必须**手动**实现 `Default`（不能 derive），新增字段要同时改 `merge()` 与 `config/default.toml` —— 见 `crates/yekitrim-core/src/config/AGENTS.md`

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
/// v1 的代码级默认值（`[llm] temperature` 缺席时那个数**真的会发出去**）。
///
/// ⚠️ 现在唯一的用途是 **v1→v2 迁移时把它落成显式键**（`version.rs`）——
/// 新形状里"缺席"的含义是"不发送"，不落实现值就等于升级一次、请求体少一个键。
pub(super) fn default_temperature() -> f64 {
    0.1
}
/// 同 `default_temperature`：v1 的 `[llm] max_tokens` 默认值（迁移时落成显式键）。
pub(super) fn default_max_tokens() -> u32 {
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
        // `*_file` 是**相对工程根**写进 yekitrim.toml 的（`yekitrim init` 的 `path_to_rel`），
        // 所以相对路径要先按 output_dir 的父目录（= 工程根）拼一次再判存在性。
        // ⚠ 直接 `PathBuf::from(p).exists()` 是"相对进程 CWD"：`yekitrim server` 是长驻进程，
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
    /// 是否开放邀请码注册。默认关；`yekitrim user invite` 生成码时置 true，
    /// `yekitrim user invite --disable` 关闭。**开关打开也必须有可用邀请码**才能注册成功
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
    /// webhook token 的 KDF 哈希串（明文只在 `yekitrim ingest token` 生成时显示一次）
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
    /// **运行时**由设置页/`ensure_ytdlp` 下到 `<程序目录>/.yekitrim/bin/` —— 和 ASR 模型一样，
    /// **不进发行包**，所以许可声明走 `THIRD-PARTY-NOTICES.md` 的"运行时下载"那一节）；
    /// 其它值 = **逃生舱**：用户自备的「yt-dlp 兼容」可执行文件**绝对路径**
    /// （同样不随包分发，`cargo xtask scan` 扫不到）。
    /// 加第二个引擎 = 加一个适配器，server 侧不改（见 `yekitrim_service::downloader`）。
    #[serde(default = "default_downloader")]
    pub downloader: String,
    /// yt-dlp 可执行文件路径覆盖（留空 = 自动发现：配置目录 → `<程序目录>/.yekitrim/bin/` → PATH）。
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
    /// 录播姬输出目录（必须与 `yekitrim server` 同机/同可读盘）
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
    /// 托管项目根目录；空 = `<exe_dir>/.yekitrim/projects`
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
/// `yekitrim.toml`），设置页/项目配置也能改；读不到配置时按这个默认值显示。
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

/// **分散事件合并**（2026-10-06 新增）：检测第二趟的"低标准"与合并口径。
///
/// 为什么要它：第一趟只收"完整话题"（硬标准 30s + prompt 里 0.5~3 分钟），
/// 于是**同一个话题被别的话题打断成几小段**时（电台/连麦里很常见）全都被吞并或干脆不报。
/// 第二趟按更低的门槛把小块都报出来，检测结束后按 `topic` 归组合并：
/// 无字幕的空隙**保留**（拼成一条连续区间，交给后处理「去静音」变屏蔽区间），
/// 有内容的空隙用 `Event::ranges` **丢掉**；合并后仍不达标的**丢弃**。
///
/// 完整来龙去脉见 `docs/plan/scattered-events-merge.md`。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScatteredConfig {
    /// 总开关。**默认 false** —— 不开就完全按老口径跑（一次调用都不多发）。
    #[serde(default)]
    pub enabled: bool,
    /// 单块时长下限（秒）：低于它的候选块直接丢（太短切出来没法看）。
    #[serde(default = "default_scatter_min_secs")]
    pub min_secs: f64,
    /// 单块时长上限（秒）：只写进 prompt 当"小块"的引导，不做硬裁剪。
    #[serde(default = "default_scatter_max_secs")]
    pub max_secs: f64,
    /// 单块字幕条数下限（硬判据，进合并前先过这道）。
    #[serde(default = "default_scatter_min_subs")]
    pub min_subs: usize,
    /// 单块弹幕密度下限（0 = 不看弹幕）：与 `min_subs` 是**或**关系 —— 字幕少但弹幕炸的也算。
    #[serde(default = "default_scatter_dm_floor")]
    pub dm_floor: f64,
    /// 判定"两个相邻块之间那一段算**有内容**"的字幕秒数门槛（≤ 它 = 无字幕空隙 ⇒ 保留成连续区间）。
    #[serde(default = "default_scatter_gap_sub_secs")]
    pub gap_sub_secs: f64,
    /// 判定"空隙有内容"的弹幕密度门槛（超过它也算有内容 ⇒ 空隙要丢掉）。
    #[serde(default = "default_scatter_gap_dm_floor")]
    pub gap_dm_floor: f64,
    /// 同话题两块的**最大间隔**（秒）：超过它就不再并成一条。
    #[serde(default = "default_scatter_merge_gap_secs")]
    pub merge_gap_secs: f64,
    /// 合并后**内容时长**下限（秒）：不足 ⇒ 丢弃。
    #[serde(default = "default_scatter_merge_min_secs")]
    pub merge_min_secs: f64,
    /// 合并后字幕条数下限：不足 ⇒ 丢弃。
    #[serde(default = "default_scatter_merge_min_subs")]
    pub merge_min_subs: usize,
    /// 合并后跨度上限（秒）：超了按"从最热的块往两边收"截断，防一路并成整场。
    #[serde(default = "default_scatter_max_merge_secs")]
    pub max_merge_secs: f64,
    /// 要求 LLM 给每块输出 `topic` 归组键（合并靠它，不靠标题字面）。关掉则只按时间相邻合并。
    #[serde(default = "default_true")]
    pub topic_key: bool,
    /// 第二趟的 system prompt（留空 = 内建 `SCATTER_SYSTEM_PROMPT`）。
    #[serde(default)]
    pub system_prompt: String,
    /// 第二趟的 user prompt 模板（留空 = 内建 `SCATTER_PROMPT`）。
    #[serde(default)]
    pub prompt_template: String,
}

fn default_scatter_min_secs() -> f64 {
    8.0
}
fn default_scatter_max_secs() -> f64 {
    60.0
}
fn default_scatter_min_subs() -> usize {
    2
}
fn default_scatter_dm_floor() -> f64 {
    0.15
}
fn default_scatter_gap_sub_secs() -> f64 {
    0.5
}
fn default_scatter_gap_dm_floor() -> f64 {
    0.3
}
fn default_scatter_merge_gap_secs() -> f64 {
    45.0
}
fn default_scatter_merge_min_secs() -> f64 {
    20.0
}
fn default_scatter_merge_min_subs() -> usize {
    3
}
fn default_scatter_max_merge_secs() -> f64 {
    240.0
}

impl Default for ScatteredConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            min_secs: default_scatter_min_secs(),
            max_secs: default_scatter_max_secs(),
            min_subs: default_scatter_min_subs(),
            dm_floor: default_scatter_dm_floor(),
            gap_sub_secs: default_scatter_gap_sub_secs(),
            gap_dm_floor: default_scatter_gap_dm_floor(),
            merge_gap_secs: default_scatter_merge_gap_secs(),
            merge_min_secs: default_scatter_merge_min_secs(),
            merge_min_subs: default_scatter_merge_min_subs(),
            max_merge_secs: default_scatter_max_merge_secs(),
            topic_key: true,
            system_prompt: String::new(),
            prompt_template: String::new(),
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
    /// 事件检测并发窗口数。**留空 = 跟随 `.yekitrim/llm.toml` 的 `[llm] max_concurrent`**；
    /// 填了才以本段为准（一般是排查限流时才需要单独压这里）。
    #[serde(default)]
    pub max_concurrent: Option<usize>,
    /// **分散事件合并**（第二趟低标准检测 + 合并）。默认关 —— 见 `ScatteredConfig`。
    #[serde(default)]
    pub scattered: ScatteredConfig,
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
            scattered: ScatteredConfig::default(),
        }
    }
}

/// 链路标识：决定 `.yekitrim/llm.toml` 的 `[llm.overrides.*]` 里哪一组覆盖生效，
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

    /// 该链路的**代码级默认**推理档位（全局与覆盖都没配时的兜底）。
    ///
    /// - `Polish` / `Utility` = `"off"`：精修分段/逐段事件、贴纸包标签、封面文案
    ///   历史上就在代码里写死了关思考，这里把它收成一处；
    /// - `Detect` = `None`：检测从来没强制过，交给 `extra_params.thinking`；
    /// - `Correct` = `None`：订正另有 `[correct] thinking` 老开关（见 `AgentTask::thinking`），
    ///   LLM 层没表态时它照旧生效。
    ///
    /// ⚠️ 值的拼写用**档位名**（`off`），不是 v1 的 `disabled`：这个字符串会落进
    /// `LlmParamsToml.reasoning`，两种拼写混在一起迟早有人按"档位名"去解析。
    /// `from_thinking_flag` 两个都认，所以行为与 v1 逐字节相同。
    pub fn default_thinking(self) -> Option<&'static str> {
        match self {
            LlmLink::Detect | LlmLink::Correct => None,
            LlmLink::Polish | LlmLink::Utility => Some("off"),
        }
    }
}

/// **请求参数面**（`[llm.params]` 与 `[llm.overrides.<link>]` 用的是**同一套字段**）。
///
/// ⚠️ **每个字段都是 `Option`：「省略」= 不表态（那个键不进请求体）。**
/// 这是与 v1 最大的语义差别 —— v1 的 `[llm] temperature` / `max_tokens` 是**带默认值的必填项**
/// （0.1 / 65536，每次请求都真的发出去），所以老文件迁移时会把生效值**落成显式键**
/// （见 [`super::version`] 的 v1→v2），否则"升级一次请求体就变了"。
///
/// 与**能力**分开：能力在 `[llm.models."<id>"]`（[ModelOverrideToml]），这里只说"这一趟发什么"。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct LlmParamsToml {
    /// 采样温度；**省略 = 不发**（`temperature = 0` 是"要发 0"，不是"没配"）。
    #[serde(default)]
    pub temperature: Option<f64>,
    /// 输出上限；**省略 = 不发**（字段名由该模型的能力决定：`max_tokens` / `max_completion_tokens`）。
    #[serde(default)]
    pub max_tokens: Option<u32>,
    /// 推理档位：`off|minimal|low|medium|high|xhigh`（v1 的 `enabled`/`disabled` 也认）；
    /// **省略 = 不表态**（此时 `extra_params.thinking` 说了算，各链路的代码默认也照旧生效）。
    #[serde(default)]
    pub reasoning: Option<String>,
    #[serde(default)]
    pub top_p: Option<f64>,
    /// 要求 JSON 输出（wire 上写 `response_format`）；省略 = 不主动打开
    /// （老工程仍由 `extra_params.response_format` 表达，那是优先级最低的逃逸口）。
    #[serde(default)]
    pub json_mode: Option<bool>,
}

/// 单条链路的覆盖：**与 `[llm.params]` 同一套字段**（缺席 = 跟随全局）。
pub type LlmOverride = LlmParamsToml;

impl LlmParamsToml {
    /// 链路覆盖合并：**覆盖里写了的字段优先**，其余继承全局。
    ///
    /// 三个 v1 就有的兜底原样搬过来（**别删**，都是手改坏文件的现场）：
    /// `max_tokens = 0` 视为没配、非有限 temperature 忽略、超范围夹回 [0,2]。
    fn merged_over(&self, ov: &Self, link: LlmLink) -> Self {
        let max_tokens = match ov.max_tokens {
            // 0 = 没配（设置页清空会写成 null，但不排除手改出 0）：别把 0 发给 provider
            Some(0) => {
                tracing::warn!(
                    "[llm.overrides.{}] max_tokens = 0 无效，按未设置处理（跟随 [llm.params] 的 {:?}）",
                    link.key(),
                    self.max_tokens
                );
                self.max_tokens
            }
            Some(n) => Some(n),
            None => self.max_tokens,
        };
        let temperature = match ov.temperature {
            Some(t) if t.is_finite() => Some(t.clamp(0.0, 2.0)),
            Some(t) => {
                tracing::warn!(
                    "[llm.overrides.{}] temperature = {t} 不是有限值，按未设置处理",
                    link.key()
                );
                self.temperature
            }
            None => self.temperature,
        };
        Self {
            temperature,
            max_tokens,
            // 链路覆盖 → 全局 → 链路代码默认（精修/零散单发 = 关思考）
            reasoning: ov
                .reasoning
                .clone()
                .or_else(|| self.reasoning.clone())
                .or_else(|| link.default_thinking().map(str::to_string)),
            top_p: ov.top_p.or(self.top_p),
            json_mode: ov.json_mode.or(self.json_mode),
        }
    }
}

/// `[llm.models."<id>"]`：**手填能力**（内置表里没有的模型靠它），只覆盖写了的字段。
///
/// 与参数面分开的理由：能力是"这个模型**能**怎样"，参数是"这一趟**要**怎样"。
/// 混在一张表里就会出现"改了全局温度、顺手把某模型的能力也改了"。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ModelOverrideToml {
    /// `openai_effort` / `thinking_type` / `qwen_enable_thinking` / `openrouter_effort`；
    /// 认不出的值降级成"不发推理参数"（宁可不发也不瞎发）。
    #[serde(default)]
    pub thinking_wire: Option<String>,
    /// `max_tokens` / `max_completion_tokens`。
    #[serde(default)]
    pub max_output_wire: Option<String>,
    /// `"supported"` / `"unsupported"`（后者 = 这个模型不收 temperature，别发）。
    #[serde(default)]
    pub temperature: Option<String>,
    /// 档位 → wire 拼写；**空串 = 支持但不发送**（与内置表里的 `null` 同义）。
    #[serde(default)]
    pub reasoning: Option<std::collections::BTreeMap<String, String>>,
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

/// `.yekitrim/llm.toml` 的形状（v2）。
///
/// **三块各管各的**（别混进同一张表）：
/// * 连接面：`api_key` / `base_url` / `model` / `max_concurrent`；
/// * **请求参数面**：[`LlmParamsToml`]（全局 `params` + 四条链路的 `overrides`）；
/// * **能力面**：`models["<id>"]`（[`ModelOverrideToml`]）；`extra_params` 是优先级最低的老逃逸口。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmConfig {
    #[serde(default)]
    pub api_key: String,
    #[serde(default)]
    pub base_url: String,
    #[serde(default)]
    pub model: String,
    /// 并发请求数——**所有 LLM 调用方共用这一个旋钮**（事件检测 / 精修事件生成 / 字幕订正）。
    /// 各段自己的 `max_concurrent` 留空即跟随这里；填了则那一段以自己的为准。
    #[serde(default = "default_llm_max_concurrent")]
    pub max_concurrent: usize,
    /// **请求参数面**（全局）。不写 = 那几个键都不进请求体。
    #[serde(default)]
    pub params: LlmParamsToml,
    /// **能力面**：表里没收录的模型在这里手填（键 = 模型 id，匹配大小写不敏感）。
    #[serde(default)]
    pub models: std::collections::BTreeMap<String, ModelOverrideToml>,
    /// 老逃逸口：整表原样进请求体，**优先级最低**（结构化项之后、reasoning 之前）。
    #[serde(default)]
    pub extra_params: Option<toml::Table>,
    /// 按链路的**请求参数**覆盖（见 [`LlmConfig::for_link`]）。
    #[serde(default)]
    pub overrides: LlmOverrides,
}

impl Default for LlmConfig {
    fn default() -> Self {
        Self {
            api_key: String::new(),
            base_url: String::new(),
            model: String::new(),
            max_concurrent: default_llm_max_concurrent(),
            params: LlmParamsToml::default(),
            models: std::collections::BTreeMap::new(),
            extra_params: None,
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

    /// 某链路的**有效**调用参数 —— **所有构造 `LlmClient` 的地方都必须走这里**，
    /// 别再裸 `From` + 手写 `with_max_tokens` / `with_temperature` / `with_thinking`
    /// （漏一个调用点 = 那个旋钮静默不生效）。
    ///
    /// 只改写 `params`（请求参数面）：覆盖里写了的字段优先，其余继承全局；**能力面 `models` 原样带过去**
    /// （能力跟模型走，不跟链路走）。逐字段优先级见 [`LlmParamsToml::merged_over`]。
    pub fn for_link(&self, link: LlmLink) -> LlmConfig {
        let ov = self.overrides.of(link);
        LlmConfig { params: self.params.merged_over(ov, link), ..self.clone() }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CuttingConfig {
    #[serde(default = "default_output_dir")]
    pub output_dir: String,
    #[serde(default)]
    pub overwrite: Option<bool>,
}

impl Default for CuttingConfig {
    fn default() -> Self {
        Self {
            output_dir: default_output_dir(),
            overwrite: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SfxConfig {
    /// 音效屏蔽列表: 精确 id 屏蔽单个音效, "tag:xxx" 屏蔽整个标签族
    /// (yekitrim-polish 使用, 全局层定义于此供跨工程共享)
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
    /// 订正并发数。**留空 = 跟随 `.yekitrim/llm.toml` 的 `[llm] max_concurrent`**。
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
        c.params.max_tokens = Some(16384);
        c.params.temperature = Some(0.1);
        c
    }

    /// 没配覆盖 = 逐项跟随 `[llm.params]`（既有安装升级上来行为不变）。
    #[test]
    fn for_link_inherits_global_when_no_override() {
        let c = base();
        let d = c.for_link(LlmLink::Detect);
        assert_eq!(d.params.max_tokens, Some(16384));
        assert_eq!(d.params.temperature, Some(0.1));
        assert_eq!(d.model, "m");
        assert_eq!(d.max_concurrent, c.max_concurrent);
        // 检测不强制推理档位（历史上就没有写死），交给 extra_params
        assert_eq!(d.params.reasoning, None);
    }

    /// 覆盖 > 全局，且只影响自己那条链路。
    #[test]
    fn for_link_applies_override_only_to_that_link() {
        let mut c = base();
        c.overrides.detect.max_tokens = Some(8192);
        c.overrides.detect.temperature = Some(0.7);
        c.overrides.detect.reasoning = Some("high".into());

        let d = c.for_link(LlmLink::Detect);
        assert_eq!(d.params.max_tokens, Some(8192));
        assert!((d.params.temperature.unwrap() - 0.7).abs() < 1e-9);
        assert_eq!(d.params.reasoning.as_deref(), Some("high"));

        let p = c.for_link(LlmLink::Polish);
        assert_eq!(p.params.max_tokens, Some(16384), "别的链路不受影响");
        assert!((p.params.temperature.unwrap() - 0.1).abs() < 1e-9);
    }

    /// 链路代码默认：精修/零散单发关思考（历史写死的那两处），可被覆盖或全局顶掉。
    #[test]
    fn link_thinking_defaults_and_precedence() {
        let mut c = base();
        assert_eq!(c.for_link(LlmLink::Polish).params.reasoning.as_deref(), Some("off"));
        assert_eq!(c.for_link(LlmLink::Utility).params.reasoning.as_deref(), Some("off"));
        assert_eq!(c.for_link(LlmLink::Correct).params.reasoning, None);

        // 链路覆盖 > 全局
        c.params.reasoning = Some("high".into());
        assert_eq!(c.for_link(LlmLink::Polish).params.reasoning.as_deref(), Some("high"));
        assert_eq!(c.for_link(LlmLink::Detect).params.reasoning.as_deref(), Some("high"));
        c.overrides.polish.reasoning = Some("off".into());
        assert_eq!(c.for_link(LlmLink::Polish).params.reasoning.as_deref(), Some("off"));
    }

    /// 手改坏的值不能让整条链路炸：0 = 未设置，非有限 temperature 忽略，超范围夹回来。
    #[test]
    fn for_link_ignores_bogus_values() {
        let mut c = base();
        c.overrides.detect.max_tokens = Some(0);
        c.overrides.detect.temperature = Some(f64::NAN);
        let d = c.for_link(LlmLink::Detect);
        assert_eq!(d.params.max_tokens, Some(16384));
        assert_eq!(d.params.temperature, Some(0.1));

        c.overrides.correct.temperature = Some(5.0);
        assert_eq!(c.for_link(LlmLink::Correct).params.temperature, Some(2.0));
    }

    /// TOML 形态（= 用户手写的那两段）；未知键忽略，不把整份配置打回默认。
    #[test]
    fn parses_params_and_overrides_from_toml() {
        let text = r#"
api_key = "k"

[params]
max_tokens = 16384
reasoning = "off"

[overrides.detect]
max_tokens = 8192
[overrides.polish]
max_tokens = 32768
temperature = 0.2
reasoning = "high"
"#;
        let c: LlmConfig = toml::from_str(text).unwrap();
        assert_eq!(c.for_link(LlmLink::Detect).params.max_tokens, Some(8192));
        assert_eq!(c.for_link(LlmLink::Polish).params.max_tokens, Some(32768));
        assert_eq!(c.for_link(LlmLink::Polish).params.reasoning.as_deref(), Some("high"));
        // utility 缺席 → 跟随全局
        assert_eq!(c.for_link(LlmLink::Utility).params.max_tokens, Some(16384));

        let typo: LlmConfig =
            toml::from_str("api_key = \"k\"\n[overrides.detectt]\nmax_tokens = 1\n").unwrap();
        assert_eq!(typo.api_key, "k");
        assert_eq!(
            typo.for_link(LlmLink::Detect).params.max_tokens,
            LlmConfig::default().params.max_tokens,
            "拼错的表名只被忽略，不该生效也不该撑大范围"
        );
    }

    /// **模型级能力覆盖**：内置表怎么说是一回事，手填的必须真的进 `LlmParams.caps`。
    ///
    /// 这条是"表里没收录的模型也能用"的判据 —— 少了它，用户填了 `[llm.models."x"]` 却毫无效果
    /// （而且**不报错**，最难查）。
    #[test]
    fn model_level_caps_override_reaches_llm_params() {
        use yekitrim_llm::params::MaxOutputWire;
        // 先拿一支**表里认识**的模型当底（deepseek：thinking_type + 非默认的输出字段名）
        let mut c = LlmConfig::default();
        c.base_url = "https://api.deepseek.com/v1".into();
        c.model = "deepseek-v4-pro".into();
        let before: yekitrim_llm::LlmParams = (&c).into();
        assert_eq!(before.caps.thinking_wire, yekitrim_llm::ThinkingWire::ThinkingType);
        assert_eq!(before.caps.max_output_wire, MaxOutputWire::MaxCompletionTokens);

        c.models.insert(
            "deepseek-v4-pro".into(),
            ModelOverrideToml {
                thinking_wire: Some("openai_effort".into()),
                max_output_wire: Some("max_tokens".into()),
                temperature: Some("unsupported".into()),
                reasoning: Some(
                    [("off".to_string(), String::new()), ("high".to_string(), "high".to_string())]
                        .into_iter()
                        .collect(),
                ),
            },
        );
        let after: yekitrim_llm::LlmParams = (&c).into();
        assert_eq!(after.caps.thinking_wire, yekitrim_llm::ThinkingWire::OpenAIEffort);
        assert_eq!(after.caps.max_output_wire, MaxOutputWire::MaxTokens, "输出上限字段名被改回来了");
        assert!(after.caps.reasoning_supported(), "手填的档位要算数");
        assert_eq!(after.caps.wire_for(yekitrim_llm::ReasoningLevel::High), Some(&Some("high".into())));
        assert_eq!(
            after.caps.wire_for(yekitrim_llm::ReasoningLevel::Off),
            Some(&None),
            "空串 = 支持但不发送"
        );
        assert_eq!(after.caps.temperature, yekitrim_llm::params::Support::Unsupported);

        // 键的大小写不该决定生效与否（模型 id 各家拼法不一）
        c.model = "DeepSeek-V4-Pro".into();
        let ci: yekitrim_llm::LlmParams = (&c).into();
        assert_eq!(ci.caps.thinking_wire, yekitrim_llm::ThinkingWire::OpenAIEffort);

        // 表里**完全没收录**的那对：手填照样生效（这就是这块存在的理由）
        let mut u = LlmConfig::default();
        u.base_url = "https://unknown.example.com/v1".into();
        u.model = "my-gateway-model".into();
        let before_u: yekitrim_llm::LlmParams = (&u).into();
        assert_eq!(before_u.caps.thinking_wire, yekitrim_llm::ThinkingWire::None, "未知模型保守默认");
        u.models.insert(
            "my-gateway-model".into(),
            ModelOverrideToml { thinking_wire: Some("openai_effort".into()), ..Default::default() },
        );
        let after_u: yekitrim_llm::LlmParams = (&u).into();
        assert_eq!(after_u.caps.thinking_wire, yekitrim_llm::ThinkingWire::OpenAIEffort);
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

    /// 自建临时目录（yekitrim-core 不依赖 tempfile；pid + 纳秒保证并发测试不撞名）。
    struct TmpDir(PathBuf);

    impl TmpDir {
        fn new(tag: &str) -> Self {
            let nanos = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0);
            let p = std::env::temp_dir()
                .join(format!("yekitrim-stage-{tag}-{}-{nanos}", std::process::id()));
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

/// **配置 → LLM 传输层参数**的唯一换算（2026-10-19 起 LLM 抽成独立 crate）。
///
/// # 为什么转换写在这一侧
///
/// `yekitrim-llm` **不认识配置文件**（机器判据 `llm_is_a_leaf`）：它只吃
/// [`yekitrim_llm::LlmParams`]。配置的形状（TOML、四条链路的 overrides、版本迁移）属于本模块，
/// 所以「配置 → 参数」这一步只能在这里做 —— 于是依赖方向是 **config → llm**，反过来就成环。
///
/// ⚠️ 调用方仍然必须先 `for_link(LlmLink)` 再转：`for_link` 解的是**链路覆盖**，
/// 本 impl 只是把解完的结果换一个类型，**不改任何取值**。
impl From<LlmConfig> for yekitrim_llm::LlmParams {
    fn from(c: LlmConfig) -> Self {
        (&c).into()
    }
}

/// `[llm.models."<id>"]` 里那条覆盖（**大小写不敏感**：模型 id 各家拼法不一，
/// 手写一个大小写不同的键就静默不生效，是最难查的那种坑）。
fn model_override_for<'a>(c: &'a LlmConfig) -> Option<&'a ModelOverrideToml> {
    let id = c.model.trim();
    if id.is_empty() {
        return None;
    }
    c.models
        .get(id)
        .or_else(|| c.models.iter().find(|(k, _)| k.eq_ignore_ascii_case(id)).map(|(_, v)| v))
}

/// 把手填能力盖到内置表查出来的 caps 上（**只覆盖写了的字段**）。
fn apply_model_override(caps: &mut yekitrim_llm::ModelCaps, ov: &ModelOverrideToml) {
    use yekitrim_llm::params::{MaxOutputWire, Support, ThinkingWire};
    if let Some(w) = ov.thinking_wire.as_deref() {
        caps.thinking_wire = ThinkingWire::parse(w);
    }
    if let Some(w) = ov.max_output_wire.as_deref() {
        caps.max_output_wire = MaxOutputWire::parse(w);
    }
    if let Some(t) = ov.temperature.as_deref() {
        // 与内置表同一套写法（unsupported / no / false 都算不支持）
        let t = t.trim().to_ascii_lowercase();
        caps.temperature = if matches!(t.as_str(), "unsupported" | "no" | "false") {
            Support::Unsupported
        } else {
            Support::Supported
        };
    }
    if let Some(levels) = &ov.reasoning {
        for (k, wire) in levels {
            if let Some(lv) = yekitrim_llm::ReasoningLevel::parse(k) {
                // 空串 = 支持但不发送（与内置表里的 null 同义）
                caps.levels.insert(lv, (!wire.is_empty()).then(|| wire.clone()));
            }
        }
    }
}

impl From<&LlmConfig> for yekitrim_llm::LlmParams {
    fn from(c: &LlmConfig) -> Self {
        use yekitrim_llm::params::Support;
        let p = &c.params;
        // 档位：老拼写（enabled / disabled）与档位名都认 —— 手改的文件不该因此静默失效。
        // ⚠️ reasoning_explicit 必须跟着置位：链路自己的老开关（如 [correct] thinking）
        //    只在 LLM 层**没表态**时才生效（2026-09 修过的一个真 bug）。
        let reasoning = p
            .reasoning
            .as_deref()
            .and_then(yekitrim_llm::ReasoningLevel::from_thinking_flag);
        // 能力：**内置表打底 → 手填覆盖**（表里没收录的模型靠 [llm.models]）。
        let mut caps = yekitrim_llm::catalog::caps_for(&c.base_url, &c.model);
        if let Some(ov) = model_override_for(c) {
            apply_model_override(&mut caps, ov);
        }
        // ⚠️ 「省略 temperature = 不发」只能借 caps 这一个出口表达：LlmParams.temperature 不是
        //    Option，而 wire 只在 caps.temperature == Supported 时才写那个键。借它是安全的 ——
        //    本函数算出来的 caps 只喂给这一次请求（界面看的能力走 GET /config/llm/catalog）。
        //    顺序：手填覆盖在前、这条在后 —— 没写值时就**没有值可发**，不该凭空塞一个默认温度。
        if p.temperature.is_none() {
            caps.temperature = Support::Unsupported;
        }
        Self {
            api_key: c.api_key.clone(),
            base_url: c.base_url.clone(),
            model: c.model.clone(),
            temperature: p.temperature.filter(|t| t.is_finite()).unwrap_or_default(),
            // 省略 = 不发（v1 那个"永远会发出去的默认值"由迁移落成显式键，见 version.rs）
            max_tokens: p.max_tokens,
            reasoning: reasoning.unwrap_or_default(),
            reasoning_explicit: reasoning.is_some(),
            top_p: p.top_p.filter(|v| v.is_finite()),
            // extra_params.response_format 仍是老工程的表达（原样保留）；这里是新界面的一等公民键。
            json_mode: p.json_mode.unwrap_or(false),
            // extra_params 是 TOML 表，传输层只认 JSON —— 在这里转一次。
            extra_params: c.extra_params.as_ref().and_then(|t| serde_json::to_value(t).ok()),
            max_concurrent: c.concurrency(None),
            caps,
        }
    }
}
