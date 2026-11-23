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
    /// 贴纸包（zip）上传 / 解压的体积上限（默认 1 GiB 上传、2 GiB 解压总量）
    #[serde(default)]
    pub stickers: StickersConfig,
    /// 多用户服务：项目托管与容量（P2 起用 `projects_root`，P3/P5 用上限与水位）
    #[serde(default)]
    pub storage: StorageConfig,
    /// 多用户服务 P3：采集（录播姬 webhook + 兜底扫描）
    #[serde(default)]
    pub ingest: IngestConfig,
    /// 出片回传：把成片交给 biliLive-tools 投稿（YekiTrim 自己不做 B站上传）。
    #[serde(default)]
    pub publish: PublishConfig,
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
    /// **多人形态开关**（2026-11-13 用户拍板：默认单人）。
    ///
    /// 默认 `false` = **单人优先**：界面隐藏邀请码 / 账号配额 / 注册入口 / 任务指派对象
    /// （接口与 CLI 全部保留，只是不露出）。置 `true` 才把多用户那套 UI 拿出来。
    ///
    /// ⚠️ 前端经 `/api/health.multi_user` 读它（health 是公开接口，只回布尔）。
    /// ⚠️ **不是权限开关**：`true` 也不会让普通账号变成管理员；它只回答"这部署按几人用"。
    #[serde(default)]
    pub multi_user: bool,
}

impl Default for AuthConfig {
    fn default() -> Self {
        Self {
            registration_enabled: false,
            multi_user: false,
        }
    }
}

/// `[ingest.bililive_tools]` — biliLive-tools 的**只读**对接（列录制用户 + 录像历史）。
///
/// 与 `[ingest.recorder]`（录播姬）**分开配置**：两套 API 不兼容，界面上也是两块独立的设置。
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct BiliLiveToolsConfig {
    /// 它的 HTTP API 地址（例 `http://127.0.0.1:18010`）。**空 = 未对接**。
    #[serde(default)]
    pub api_base: String,
    /// 访问密钥：原样放进 `Authorization`（不是 Bearer）。
    #[serde(default)]
    pub api_key: String,
    /// 录像根目录（宿主机路径，YekiTrim 要读得到）。
    #[serde(default)]
    pub root: String,
    /// 它接口里的容器路径前缀（例 `/app/video`），剥掉后拼到 `root` 上。
    #[serde(default)]
    pub container_root: String,
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
    /// biliLive-tools 的只读对接（独立于录播姬）
    #[serde(default)]
    pub bililive_tools: BiliLiveToolsConfig,
    /// 通用 webhook（任意录制软件把"录制完成"推过来，不要求它长得像录播姬）。
    #[serde(default)]
    pub webhook: WebhookConfig,
    /// **出网代理**（下载 yt-dlp 二进制这类境外大文件用；留空 = 直连）。
    ///
    /// 支持 `http://` / `https://` / `socks5://` / `socks5h://`（远程 DNS，墙外建议用它）。
    /// ⚠️ **不能只靠环境变量**：WebUI 服务常由 systemd / 计划任务拉起，环境里没有你 shell
    /// 里 `export` 的那份代理 —— 症状是"我明明配了代理，它还是直连超时"。这里是显式配置项。
    #[serde(default)]
    pub proxy: String,
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
            bililive_tools: BiliLiveToolsConfig::default(),
            webhook: WebhookConfig::default(),
            proxy: String::new(),
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
    /// 录制软件：`bililive`（录播姬，默认）| `bililive-tools`。
    ///
    /// 两者 API 不兼容：录播姬是 `/api/room` + HTTP Basic；biliLive-tools 是 `/recorder/list` +
    /// `Authorization: <passkey>`。**目录语义一样**（`root` 是录像根），其余字段按 kind 取用。
    #[serde(default = "default_recorder_kind")]
    pub kind: String,
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
    /// biliLive-tools 的 API 密钥：原样放进 `Authorization`（**不是 Bearer**）
    #[serde(default)]
    pub api_key: String,
    /// biliLive-tools 的容器路径前缀（例 `/app/video`）：接口返回的路径按它剥掉再拼 `root`。
    #[serde(default)]
    pub container_root: String,
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

fn default_recorder_kind() -> String {
    "bililive".into()
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
            kind: default_recorder_kind(),
            root: String::new(),
            api_base: String::new(),
            api_user: String::new(),
            api_pass: String::new(),
            api_key: String::new(),
            container_root: String::new(),
            watch_interval_mins: default_watch_interval_mins(),
            finalize_idle_mins: default_finalize_idle_mins(),
            move_into_managed: true,
            concat_files: true,
            extensions: default_recorder_extensions(),
            auto_pipeline: default_auto_pipeline(),
        }
    }
}

/// `[ingest.webhook]` — **通用**"录制完成"入口（任意录制软件都能推）。
///
/// 与 `[ingest.recorder]` 的区别：录播姬那条有会话/分片/空闲窗口语义；这里只认
/// "一个已经写完的文件 + 可选的同名弹幕"，直接建一个项目。录制软件只要能发一条
/// JSON（或它自带 webhook，经 §4.1 的字段适配）就能接进来。
///
/// ⚠️ 项目**建在录像原目录**（不搬不删源文件）：biliLive-tools/录播姬的录制历史
/// 还引用原路径，且两个盘经常不是同一个文件系统（硬链接不可用）。代价是 YekiTrim
/// 的 `output/` 也写在那个目录里 —— 磁盘水位要自己盯。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebhookConfig {
    /// 录像根目录（宿主机绝对路径）。**空 = 未启用**。
    #[serde(default)]
    pub root: String,
    /// 事件里路径的容器前缀（例 `/app/video`）：非空时先剥掉，再按 [`Self::root`] 解析。
    /// 空 = 事件里直接给宿主机路径（或相对 [`Self::root`] 的路径）。
    #[serde(default)]
    pub container_root: String,
    /// 建项目后自动执行到哪一步：none | transcribe | detect | cut。
    /// 默认 **none**：通用入口不知道来源会不会很贵，先建项目、让用户自己点。
    #[serde(default = "default_webhook_auto_pipeline")]
    pub auto_pipeline: String,
}

fn default_webhook_auto_pipeline() -> String {
    "none".into()
}

impl Default for WebhookConfig {
    fn default() -> Self {
        Self {
            root: String::new(),
            container_root: String::new(),
            auto_pipeline: default_webhook_auto_pipeline(),
        }
    }
}

/// `[publish.bililive_tools]` — 出片回传给 biliLive-tools 投稿。
///
/// YekiTrim 只负责"把成片放到 biliLive-tools 容器看得见的目录 + 调它的投稿 API"，
/// B站登录/分区/标签/封面/重试/分P 全部由 biliLive-tools 自己管。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PublishConfig {
    #[serde(default)]
    pub bililive_tools: BiliLivePublishConfig,
}

impl Default for PublishConfig {
    fn default() -> Self {
        Self {
            bililive_tools: BiliLivePublishConfig::default(),
        }
    }
}

/// ⚠️ **只从全局 `config.toml` 读**（`load_global_config()`，server 的发布接口还要求 admin）：
/// 不参与项目级 `merge()`，`staging_dir` / `passkey_file` 也**不在** `api/config.rs::PROJECT_PATH_KEYS`
/// 的 jail 名单里 —— 这两个键是**宿主机任意路径**（写文件 / 读凭证），将来若要改成项目级配置，
/// 必须先给它们补路径面防线②，否则普通账号就能让服务端到处写文件、或把别的文件当 passkey 读走。
/// 同理：新建工程配置时 `[publish]` 整段被 `project_config_body` 剥掉（含路径），见 config 卡。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BiliLivePublishConfig {
    /// API 地址（例 `http://127.0.0.1:18010`）；空 = 未启用
    #[serde(default)]
    pub base_url: String,
    /// 600 权限文件，内容是一行 `BILILIVE_TOOLS_PASSKEY`；空 = 未启用
    #[serde(default)]
    pub passkey_file: String,
    /// biliLive-tools 的上传预设 id（在它的 UI 里维护分区/标签/合集等）
    #[serde(default = "default_upload_preset")]
    pub upload_preset_id: String,
    /// 成片暂存目录（**宿主机**路径；会被 biliLive-tools 看到）
    #[serde(default)]
    pub staging_dir: String,
    /// 暂存目录在 biliLive-tools **容器内**的对应路径（例 `/app/video/_yekitrim_publish`）
    #[serde(default)]
    pub container_staging_root: String,
    /// B站账号 uid（0 = 自动取 `/user/list` 的第一个）
    #[serde(default)]
    pub uid: u64,
}

fn default_upload_preset() -> String {
    "default".into()
}

impl Default for BiliLivePublishConfig {
    fn default() -> Self {
        Self {
            base_url: String::new(),
            passkey_file: String::new(),
            upload_preset_id: default_upload_preset(),
            staging_dir: String::new(),
            container_staging_root: String::new(),
            uid: 0,
        }
    }
}

/// `[stickers]` —— 贴纸包（一个 zip 解压成一堆贴纸）的体积上限，单位 **MiB**。
///
/// 为什么做成配置：贴纸包从"十几张小图"到"几百张 GIF"差一个数量级，
/// 写死一个数总有一边难受（2026-11-19 实测有 500 MB 的包）。
///
/// 三个值都是"0 或非法值按默认值处理"（见 [`StickersConfig::upload_max_bytes`] 等），
/// 不是"0 = 不限" —— 不限会让一个 zip 炸弹把磁盘/内存吃干净。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StickersConfig {
    /// 上传请求体（zip 原始字节）上限。默认 1024 MiB（1 GiB）。
    #[serde(default = "default_sticker_upload_max_mb")]
    pub upload_max_mb: u32,
    /// 解压后**所有文件合计**的上限。默认 2048 MiB（zip 里是压缩过的，解压后通常更大）。
    #[serde(default = "default_sticker_extract_max_mb")]
    pub extract_max_mb: u32,
    /// 解压后**单个文件**的上限。默认 64 MiB。
    #[serde(default = "default_sticker_file_max_mb")]
    pub file_max_mb: u32,
}

impl StickersConfig {
    /// 路由层 `DefaultBodyLimit` 的硬顶：它在**启动时**就要定死，而 `[stickers]` 可以之后改。
    /// 所以"配置值"只在这个硬顶之内有效，超过按硬顶算（`/polish/stickers/limits` 会回实际生效值）。
    pub const HARD_MAX_UPLOAD_MB: u32 = 4096;

    /// 上传请求体上限（字节）。`0` 或超过硬顶 ⇒ 走默认值 / 硬顶。
    pub fn upload_max_bytes(&self) -> usize {
        let mb = match self.upload_max_mb {
            0 => default_sticker_upload_max_mb(),
            v => v.min(Self::HARD_MAX_UPLOAD_MB),
        };
        mb as usize * 1024 * 1024
    }

    /// 解压后总量上限（字节）。`0` ⇒ 默认值。
    pub fn extract_max_bytes(&self) -> u64 {
        let mb = if self.extract_max_mb == 0 {
            default_sticker_extract_max_mb()
        } else {
            self.extract_max_mb
        };
        mb as u64 * 1024 * 1024
    }

    /// 解压后单文件上限（字节）。`0` ⇒ 默认值。
    pub fn file_max_bytes(&self) -> u64 {
        let mb = if self.file_max_mb == 0 {
            default_sticker_file_max_mb()
        } else {
            self.file_max_mb
        };
        mb as u64 * 1024 * 1024
    }
}

impl Default for StickersConfig {
    fn default() -> Self {
        Self {
            upload_max_mb: default_sticker_upload_max_mb(),
            extract_max_mb: default_sticker_extract_max_mb(),
            file_max_mb: default_sticker_file_max_mb(),
        }
    }
}

fn default_sticker_upload_max_mb() -> u32 {
    1024
}
fn default_sticker_extract_max_mb() -> u32 {
    2048
}
fn default_sticker_file_max_mb() -> u32 {
    64
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

    // ---- 保留期（服务端每日维护 `spawn_daily_maintenance` 用）----
    //
    // ⚠️ 三个都**不是** `#[serde(default)]`（那会给 u32 落 0 = "关闭清理"）：老配置文件里
    // 没有这几个键时，必须落到下面那几个 `default_*` 的**默认天数**，否则升级上来的机器
    // 会静默变成"永不清理"（问题原样保留，还没人看得出来）。
    /// `audit` 表保留多少天（更早的行由每日维护删掉）。`0` = **关闭清理**。
    ///
    /// 审计是安全用途：默认给 90 天。要长期留档请自己导出，别把这里设成 0 ——
    /// 那张表只会一直长（每次登录/改密/邀请码动作都写一行）。
    #[serde(default = "default_audit_keep_days")]
    pub audit_keep_days: u32,
    /// `llm_usage` 表保留多少天（`0` = 关闭清理）。默认 365 —— 用量是**按年**看的。
    #[serde(default = "default_usage_keep_days")]
    pub usage_keep_days: u32,
    /// `<数据目录>/logs/yekitrim-YYYYMMDD.log` 保留多少天（`0` = 关闭清理）。默认 30。
    ///
    /// 一天一份，判据是**文件名里的日期**（见 `yekitrim_service::logbus::prune_old_logs`）。
    /// ⚠️ 只管 `yekitrim-*.log`：`render-*.log` 由 `render_proc::prune_logs` 按份数管。
    #[serde(default = "default_log_keep_days")]
    pub log_keep_days: u32,
}

fn default_watermark_pct() -> u32 {
    10
}

fn default_audit_keep_days() -> u32 {
    90
}

fn default_usage_keep_days() -> u32 {
    365
}

fn default_log_keep_days() -> u32 {
    30
}

impl Default for StorageConfig {
    fn default() -> Self {
        Self {
            projects_root: String::new(),
            max_disk_bytes: 0,
            disk_watermark_pct: default_watermark_pct(),
            default_quota_bytes: 0,
            local_roots: Vec::new(),
            audit_keep_days: default_audit_keep_days(),
            usage_keep_days: default_usage_keep_days(),
            log_keep_days: default_log_keep_days(),
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
    ///
    /// ⚠️ **只从界面上隐藏，代码与 TOML 都保留**：`asr/correct.rs` 还在用它，
    /// `[llm.overrides.correct]` 也必须继续生效（R4）。界面行由 [`LlmLink::ui_visible`] 决定。
    Correct,
    /// **标题生成**（封面文案）：短输出、一次一发（2026-10-19 从 Utility 拆出来）
    Title,
    /// 零散单发：贴纸包/扫描标签、连通测试（封面文案已归 [`LlmLink::Title`]）
    Utility,
}

impl LlmLink {
    /// **全部**链路（含界面上隐藏的 `Correct`）。顺序 = 枚举声明顺序。
    ///
    /// ⚠️ 会遍历它做"每条链路都…"的地方要**想清楚**：界面行请用 [`Self::ui_visible`]，
    /// 金标准请求体对拍请用 [`Self::LEGACY_V1`]。
    pub const ALL: [LlmLink; 5] = [
        LlmLink::Detect,
        LlmLink::Polish,
        LlmLink::Correct,
        LlmLink::Title,
        LlmLink::Utility,
    ];

    /// **v1 时代的四条**（`detect/polish/correct/utility`）—— 顺序就是
    /// `testdata/llm-v1-bodies.json` 里那 12 条金色请求体的顺序。
    ///
    /// ⚠️ **别改成 `ALL`**：加了新链路就会去找第 5 条（数据文件里没有），
    /// 而那份数据是"迁移前的老代码真跑出来的"，**一个字都不许动**。
    pub const LEGACY_V1: [LlmLink; 4] = [
        LlmLink::Detect,
        LlmLink::Polish,
        LlmLink::Correct,
        LlmLink::Utility,
    ];

    /// **界面「按链路覆盖」那张表要显示的行**（用户口径 2026-10-19：字幕订正删掉、加标题生成）。
    ///
    /// `Correct` **只隐藏不删**（`asr/correct.rs` 还在用、TOML 里也还在生效）。
    /// 后端的 `GET /config` 把它给前端（`override_links`）—— **别让前端自己写死四个**。
    pub fn ui_visible() -> &'static [LlmLink] {
        &[LlmLink::Detect, LlmLink::Polish, LlmLink::Title, LlmLink::Utility]
    }

    /// TOML 键名（`[llm.overrides.<key>]`）。
    pub fn key(self) -> &'static str {
        match self {
            LlmLink::Detect => "detect",
            LlmLink::Polish => "polish",
            LlmLink::Correct => "correct",
            LlmLink::Title => "title",
            LlmLink::Utility => "utility",
        }
    }

    /// 界面文案。
    pub fn label_zh(self) -> &'static str {
        match self {
            LlmLink::Detect => "事件检测",
            LlmLink::Polish => "精修剪辑",
            LlmLink::Correct => "字幕订正",
            LlmLink::Title => "标题生成",
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
            // Title 也走"代码级关思考"：封面文案本来挂在 Utility 上，那里就是关的
            LlmLink::Polish | LlmLink::Title | LlmLink::Utility => Some("off"),
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
    /// 推理档位：`off|minimal|low|medium|high|xhigh|max`（v1 的 `enabled`/`disabled` 也认）；
    /// **省略 = 不表态**（此时 `extra_params.thinking` 说了算，各链路的代码默认也照旧生效）。
    #[serde(default)]
    pub reasoning: Option<String>,
    /// **预算型方言的 token 数**（`thinking_budget` / `reasoning_max_tokens` /
    /// `thinking_token_budget` 这三种"要一个数"的方言用）。
    ///
    /// **省略 = 按档位映射表给默认**（minimal 1024 / low 4096 / medium 8192 /
    /// high 16384 / xhigh 32768 / max 65536；那张表只在 `yekitrim_llm::dialect::budget_for`）。
    /// 用户显式写 0 = 没配（同 `max_tokens` 的口径，不把 0 发给 provider）。
    #[serde(default)]
    pub thinking_budget: Option<u32>,
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
    /// 一个键都没写（`[llm.providers.params]` 用它决定要不要落这个表）。
    pub fn is_empty(&self) -> bool {
        self.temperature.is_none()
            && self.max_tokens.is_none()
            && self.reasoning.is_none()
            && self.thinking_budget.is_none()
            && self.top_p.is_none()
            && self.json_mode.is_none()
    }

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
        // 0 = 没配（手改坏文件的现场，与 max_tokens 同一条）
        let thinking_budget = match ov.thinking_budget {
            Some(0) => self.thinking_budget,
            Some(n) => Some(n),
            None => self.thinking_budget,
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
            thinking_budget,
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
    /// **输入模态**（`["text","image"]`）：这个模型吃不吃图片。
    ///
    /// ⚠️ 表里没收录、又确实是多模态的网关靠它手填；没写 = 跟随表（表也没有 ⇒ 只吃文字）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input: Option<Vec<String>>,
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
    /// 标题生成（封面文案）。键名 `title`。
    #[serde(default)]
    pub title: LlmOverride,
    #[serde(default)]
    pub utility: LlmOverride,
}

impl LlmOverrides {
    pub fn of(&self, link: LlmLink) -> &LlmOverride {
        match link {
            LlmLink::Detect => &self.detect,
            LlmLink::Polish => &self.polish,
            LlmLink::Correct => &self.correct,
            LlmLink::Title => &self.title,
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
    /// **哪一份 provider 在用**（`[[llm.providers]]` 的 id）。
    ///
    /// ⚠️ **空 = 走 legacy 顶层字段**（`api_key/base_url/model` + `[llm.params]`）——
    /// 那是 R4：老文件一个字节都不用改、行为逐字节不变。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active: Option<String>,
    /// 多份 provider（`[[llm.providers]]`）。空 = 没有这一套（legacy）。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub providers: Vec<LlmProvider>,
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
            active: None,
            providers: Vec::new(),
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
        let merged = self.effective_params().merged_over(ov, link);
        let mut out = LlmConfig { params: merged.clone(), ..self.clone() };
        // **active 那一份的 params 也要跟着换**：`From<&LlmConfig>` 在 active 分支读的是
        // provider 自己的 params（不是顶层那份），只改 `out.params` 等于没改。
        if let Some(id) = self.active_provider().map(|p| p.id.clone()) {
            if let Some(p) = out.providers.iter_mut().find(|p| p.id == id) {
                p.params = merged;
            }
        }
        out
    }

    /// 当前生效的那份 provider。`active` 空 / 指向不存在的 id ⇒ `None`（= 走 legacy 顶层字段）。
    ///
    /// ⚠️ 指向不存在的 id 时**不是报错**：读配置要能活下来（写接口那边才拦，见 [`Self::provider_problems`]）。
    pub fn active_provider(&self) -> Option<&LlmProvider> {
        let id = self.active.as_deref()?.trim();
        if id.is_empty() {
            return None;
        }
        self.providers.iter().find(|p| p.id == id)
    }

    /// 当前生效的**请求参数面**：active 那份写了就用它，否则顶层 `[llm.params]`。
    ///
    /// 「写了没有」= [`LlmParamsToml::is_empty`]。为什么允许回落：迁移（v3）**只搬三个标量**
    /// （`api_key`/`base_url`/`model`），`[llm.params]` 留在顶层当**所有 provider 的缺省参数面**
    /// —— 这样老文件的注释/形状一个字不动（R4），而手写的多 provider 配置里每份仍可自带 params。
    pub fn effective_params(&self) -> &LlmParamsToml {
        match self.active_provider() {
            Some(p) if !p.params.is_empty() => &p.params,
            _ => &self.params,
        }
    }

    /// **写配置时要拦下来的问题**（空 = 没问题）。
    ///
    /// 三条都会让"配了但没生效"，而且**不报错**（最难查）：
    /// 1. `api` 认不出 —— **别静默回退成 openai-chat**（用户以为在走别的协议）；
    /// 2. `active` 指向一个不存在的 id —— 会静默落回 legacy 顶层字段（读到的是另一份配置）；
    /// 3. 同一份文件里 `id` 重复 —— 改了一份、另一份覆盖，行为取决于顺序。
    pub fn provider_problems(&self) -> Vec<String> {
        let mut out = Vec::new();
        let mut seen: Vec<&str> = Vec::new();
        for p in &self.providers {
            if p.id.trim().is_empty() {
                out.push(format!("有一份 provider 没有 id（name = {:?}）", p.name));
            } else if seen.contains(&p.id.as_str()) {
                out.push(format!("provider id 重复：{}", p.id));
            } else {
                seen.push(&p.id);
            }
            let api = p.api.trim();
            if !api.is_empty() && yekitrim_llm::protocol::protocol(api).is_none() {
                out.push(format!(
                    "provider {} 的 api 认不出：{api}（可选：{}）",
                    if p.id.is_empty() { "?" } else { p.id.as_str() },
                    yekitrim_llm::protocol::PROTOCOLS
                        .iter()
                        .map(|x| x.id)
                        .collect::<Vec<_>>()
                        .join(" / ")
                ));
            }
        }
        if let Some(active) = self.active.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            if !self.providers.iter().any(|p| p.id == active) {
                out.push(format!("active 指向不存在的 provider：{active}"));
            }
        }
        out
    }
}


/// **一份 provider**（`[[llm.providers]]`）：一家 LLM 服务的完整配置（地址 / key / 模型目录 / 请求参数）。
///
/// 为什么要多份：用户要"能配置多个、可折叠"（参考 DSH）。**并发仍是全局**的
/// （`[llm] max_concurrent`：那是进程级旋钮，不分 provider）。
///
/// ⚠️ **R4**：`[llm] active` 为空、或整份文件没有 `[[llm.providers]]` ⇒ 走**老顶层字段**
/// （`[llm] api_key/base_url/model` + `[llm.params]` + `[llm.models.*"]`），行为逐字节不变。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct LlmProvider {
    /// **稳定 ASCII id**（引用/目录名/删除时认人用；大小写敏感）。
    #[serde(default)]
    pub id: String,
    /// 显示名（可中文）。
    #[serde(default)]
    pub name: String,
    /// **协议 id**（`yekitrim_llm::protocol::PROTOCOLS` 的 id，如 `openai-chat`）。
    /// 空 = 用默认那条；**写错要报错**（见 `LlmConfig::provider_problems`），不静默回退。
    #[serde(default)]
    pub api: String,
    #[serde(default)]
    pub base_url: String,
    /// 明文 key。⚠️ **永不出接口**（`GET /api/config` 逐条抹值 + 给 `api_key_set`）。
    #[serde(default)]
    pub api_key: String,
    /// 从环境变量取 key（`api_key` 为空时才用它；名字本身不是秘密）。
    #[serde(default)]
    pub api_key_env: String,
    /// 当前选中的模型 id。
    #[serde(default)]
    pub model: String,
    /// 这一份的**请求参数面**（与 `[llm.params]` 同形状）。
    #[serde(default, skip_serializing_if = "LlmParamsToml::is_empty")]
    pub params: LlmParamsToml,
    /// 这一份的**模型目录**（表里没收录的模型也能在这里手填能力；见 [LlmModel]）。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub models: Vec<LlmModel>,
}

/// 模型目录里的一条：这个模型**能怎样**（能力面，不是"这一趟发什么"）。
///
/// 与 `[llm.models."<id>"]`（[ModelOverrideToml]）的分工：那份是**表外手填覆盖**（老形状、键值对），
/// 这里是**provider 自带的目录**（跟随 provider 走 —— 换一份 provider 就换一套模型）。
/// 两者同时命中时**后者（老覆盖）优先**：那是用户明确的"我要覆盖这个模型"。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct LlmModel {
    #[serde(default)]
    pub id: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_window: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_output: Option<u32>,
    /// 输入模态（`["text","image"]`）；空 = 按表/保守默认（只吃文字）。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub input: Vec<String>,
    /// 这个模型支持推理吗（DSH 那个"推理等级"的开关）。
    #[serde(default)]
    pub reasoning: bool,
    /// 档位目录（`[[llm.providers.models.levels]]`）：档位 → API 参数值。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub levels: Vec<LlmLevel>,
}

/// 一个档位：内部档位名（`off|minimal|…|max`）+ wire 上要写的值。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct LlmLevel {
    #[serde(default)]
    pub level: String,
    #[serde(default)]
    pub wire_value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CuttingConfig {
    #[serde(default = "default_output_dir")]
    pub output_dir: String,
    /// 已有片段要不要**覆盖**（设置页「覆盖已有文件」）。`None` = 跟随默认（不覆盖）。
    ///
    /// 判据在 `cutting::effective_overwrite`：`true` 一律重剪；`false`/未写时，目标目录里
    /// 已经是**这条事件自己的**成片（`clip.json` 的标题相同或区间 IoU ≥ 0.5，且 `clip.mp4` 在）
    /// ⇒ 跳过并 warn，不覆盖。**单条剪切例外**（切片页点「剪切」= 显式重剪，永远覆盖）。
    /// 别人的片段无论这个开关怎样都由 `foreign_clip_conflict` 拦下。
    #[serde(default)]
    pub overwrite: Option<bool>,
    /// **磁盘预检阈值（GB，1024³）**：管线每个阶段开工前（`PipelineRunner`）与切片的每个
    /// 批次边界（`Cutter`）都会检查 `output_dir` 所在卷的可用空间，低于它就**停止整个阶段**
    /// 并明确报错，剩余片段不再派发（已剪的保留；重跑会跳过已有 clip.mp4 自动续作）。
    ///
    /// 宁可停批也不能把盘写穿 —— 2026-11-15 事故：一键全切把数据盘写到 13.88 GB，
    /// 机器上的外部看门狗（diskguard，min_free 15 GB）按最旧优先删掉了 6 场录播的源文件，
    /// 其中两场正是正在处理的工程输入。**机器上还跑着别的看门狗时，这个值必须设得
    /// 比看门狗的阈值更高才拦得住。** `0` = 关闭预检。
    #[serde(default = "default_min_free_gb")]
    pub min_free_gb: u64,
}

fn default_min_free_gb() -> u64 {
    2
}

impl CuttingConfig {
    /// 阈值换算成字节（饱和乘法：`u64::MAX` 的 GB 值不会溢出成小数）。
    pub fn min_free_bytes(&self) -> u64 {
        self.min_free_gb.saturating_mul(1024 * 1024 * 1024)
    }
}

impl Default for CuttingConfig {
    fn default() -> Self {
        Self {
            output_dir: default_output_dir(),
            overwrite: None,
            min_free_gb: default_min_free_gb(),
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

    /// **预算型方言的 N**（P2）：`[llm.params] thinking_budget` → `LlmParams.thinking_budget`；
    /// 链路覆盖优先；0 = 没配（与 max_tokens 同一口径）；不配就是 None（老工程零回归）。
    #[test]
    fn thinking_budget_reaches_llm_params() {
        let mut c = base();
        let p: yekitrim_llm::LlmParams = (&c).into();
        assert_eq!(p.thinking_budget, None, "不配 ⇒ None：按档位映射表给默认，请求体与从前逐字节一致");

        c.params.thinking_budget = Some(16384);
        let p: yekitrim_llm::LlmParams = (&c).into();
        assert_eq!(p.thinking_budget, Some(16384));

        // 链路覆盖 > 全局
        c.overrides.polish.thinking_budget = Some(4096);
        let polish = c.for_link(LlmLink::Polish);
        assert_eq!(polish.params.thinking_budget, Some(4096));
        let detect = c.for_link(LlmLink::Detect);
        assert_eq!(detect.params.thinking_budget, Some(16384), "别的链路照旧跟随全局");

        // 0 = 没配（手改坏文件的现场）：不把 0 传下去，跟随全局；全局也是 0 ⇒ None
        c.overrides.polish.thinking_budget = Some(0);
        assert_eq!(c.for_link(LlmLink::Polish).params.thinking_budget, Some(16384));
        c.params.thinking_budget = Some(0);
        let p: yekitrim_llm::LlmParams = (&c).into();
        assert_eq!(p.thinking_budget, None, "0 不发（同 max_tokens 的口径）");
    }

    /// **多 provider**：`active` 切过去，请求体真的跟着变；**没写 active ⇒ 仍走 legacy**（R4）。
    #[test]
    fn active_provider_switches_the_request_body() {
        fn body(c: &LlmConfig) -> serde_json::Value {
            let p: yekitrim_llm::LlmParams = c.into();
            let mut b = serde_json::json!({ "messages": [{ "role": "user", "content": "hi" }] });
            yekitrim_llm::wire::apply(&mut b, &p);
            b
        }
        let mut c = LlmConfig::default();
        c.api_key = "legacy-key".into();
        c.base_url = "https://unknown.example.com/v1".into();
        c.model = "m".into();
        assert!(c.active_provider().is_none(), "没有 providers ⇒ active_provider 是 None");
        let legacy = body(&c);

        c.providers = vec![
            LlmProvider {
                id: "a".into(),
                name: "A".into(),
                api: "openai-chat".into(),
                base_url: "https://api.deepseek.com/v1".into(),
                api_key: "ka".into(),
                model: "deepseek-v4-pro".into(),
                params: LlmParamsToml { reasoning: Some("high".into()), ..Default::default() },
                models: vec![LlmModel {
                    id: "deepseek-v4-pro".into(),
                    context_window: Some(12345),
                    max_output: Some(999),
                    input: vec!["text".into(), "image".into()],
                    reasoning: true,
                    levels: vec![LlmLevel { level: "high".into(), wire_value: "high".into() }],
                    ..Default::default()
                }],
                ..Default::default()
            },
            LlmProvider {
                id: "b".into(),
                name: "B".into(),
                api: "openai-chat".into(),
                base_url: "https://unknown.example.com/v1".into(),
                api_key: "kb".into(),
                model: "m".into(),
                params: LlmParamsToml { temperature: Some(0.9), max_tokens: Some(42), ..Default::default() },
                ..Default::default()
            },
        ];
        // ① **没写 active ⇒ 仍走 legacy**（老文件零回归的那半边）
        assert!(c.active_provider().is_none());
        assert_eq!(body(&c), legacy, "没写 active 就不许走 provider");

        // ② active = a ⇒ 请求体跟着换（key / 模型 / 参数 / provider 模型目录里的能力）
        c.active = Some("a".into());
        let pa: yekitrim_llm::LlmParams = (&c).into();
        assert_eq!(pa.api_key, "ka", "key 用 active 那份的");
        assert_eq!(pa.model, "deepseek-v4-pro");
        assert_eq!(pa.reasoning, yekitrim_llm::ReasoningLevel::High);
        assert_eq!(pa.caps.context_window, Some(12345), "provider 模型目录的上下文要生效");
        assert_eq!(pa.caps.max_output_ceiling, Some(999));
        assert!(pa.caps.input.image, "provider 模型目录的 input 要生效");
        assert_ne!(body(&c), legacy, "切了 provider 请求体必须变");

        // ③ active = b ⇒ 换到另一份（参数面完全不同）
        c.active = Some("b".into());
        let pb: yekitrim_llm::LlmParams = (&c).into();
        assert_eq!(pb.api_key, "kb");
        assert_eq!(pb.max_tokens, Some(42));
        assert!((pb.temperature - 0.9).abs() < 1e-9);
        assert_eq!(pb.reasoning, yekitrim_llm::ReasoningLevel::Off, "b 没配 reasoning");

        // ④ active 指向不存在的 id ⇒ **读配置要活着**（落回 legacy），写接口那边才拦
        c.active = Some("nope".into());
        assert!(c.active_provider().is_none());
        assert_eq!(body(&c), legacy);
        assert!(!c.provider_problems().is_empty(), "写接口要能拦下它");
    }

    /// 写配置时的三条拦截：`api` 认不出 / `id` 重复 / `active` 指向不存在的那份。
    #[test]
    fn provider_problems_catch_the_silent_breakage() {
        let mut c = LlmConfig::default();
        assert!(c.provider_problems().is_empty(), "空配置没问题");
        c.providers = vec![
            LlmProvider { id: "a".into(), api: "openai-chat".into(), ..Default::default() },
            LlmProvider { id: "a".into(), api: "".into(), ..Default::default() },
            LlmProvider { id: "b".into(), api: "gemini-native".into(), ..Default::default() },
        ];
        let problems = c.provider_problems();
        assert!(problems.iter().any(|p| p.contains("重复")), "{problems:?}");
        assert!(problems.iter().any(|p| p.contains("gemini-native")), "认不出的 api 要报：{problems:?}");
        assert_eq!(problems.len(), 2, "空 api 用默认那条、不算错（只有重复 + 认不出的两条）：{problems:?}");
        c.active = Some("missing".into());
        assert!(c.provider_problems().iter().any(|p| p.contains("active")), "{problems:?}");
    }

    /// `api_key_env`：显式 key 优先；只有 env 时读环境变量；都没有 ⇒ 空串（不报错）。
    #[test]
    fn provider_key_prefers_explicit_then_env() {
        let mut p = LlmProvider { api_key: "direct".into(), api_key_env: "SHOULD_NOT_READ".into(), ..Default::default() };
        assert_eq!(provider_key(&p), "direct");
        p.api_key.clear();
        std::env::set_var("VT_TEST_LLM_KEY_ENV", "from-env");
        p.api_key_env = "VT_TEST_LLM_KEY_ENV".into();
        assert_eq!(provider_key(&p), "from-env");
        p.api_key_env = "VT_TEST_LLM_KEY_MISSING".into();
        assert_eq!(provider_key(&p), "", "变量没设 ⇒ 空串（别 panic）");
    }

    /// **新增的 `Title` 链路**：`[llm.overrides.title]` 真的生效，且**默认关思考**（沿用它原来挂在
    /// Utility 上的行为）；`Correct` 那条照旧（只从界面上隐藏，代码里没动）。
    #[test]
    fn title_link_has_its_own_override() {
        let mut c = base();
        assert_eq!(c.for_link(LlmLink::Title).params.reasoning.as_deref(), Some("off"), "标题生成默认关思考");
        assert_eq!(c.for_link(LlmLink::Title).params.max_tokens, Some(16384), "没配 ⇒ 跟随全局");

        c.overrides.title.max_tokens = Some(512);
        c.overrides.title.temperature = Some(0.9);
        let t = c.for_link(LlmLink::Title);
        assert_eq!(t.params.max_tokens, Some(512), "title 覆盖真的生效");
        assert!((t.params.temperature.unwrap() - 0.9).abs() < 1e-9);
        // 别的链路不受影响
        assert_eq!(c.for_link(LlmLink::Utility).params.max_tokens, Some(16384));
        assert_eq!(c.for_link(LlmLink::Correct).params.max_tokens, Some(16384));
        // Correct 的覆盖也还在（**界面隐藏 ≠ 删掉**）
        c.overrides.correct.max_tokens = Some(8192);
        assert_eq!(c.for_link(LlmLink::Correct).params.max_tokens, Some(8192));
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
        // 先拿一支**表里认识**的模型当底（deepseek：thinking_type + 非默认的输出字段名）
        let mut c = LlmConfig::default();
        c.base_url = "https://api.deepseek.com/v1".into();
        c.model = "deepseek-v4-pro".into();
        let before: yekitrim_llm::LlmParams = (&c).into();
        assert_eq!(before.caps.dialect.id, "thinking_type");
        // 这里不写 wire 字面量（门禁 only_llm_knows_wire：那种拼写只许住 llm crate）。
        // 判据走**语义**：覆盖前后的方言必须真的换了一条（比断言某个具体串更结实，也不怕换表）。
        let before_dialect = before.caps.dialect;
        assert!(!before.caps.input.image, "deepseek 只吃文字");

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
                // 输入模态也走同一张覆盖表（表里说只吃文字，这里手填成多模态）
                input: Some(vec!["text".to_string(), "image".to_string()]),
            },
        );
        let after: yekitrim_llm::LlmParams = (&c).into();
        assert_eq!(after.caps.dialect.id, "openai_effort", "推理写法被换掉了");
        assert_ne!(
            after.caps.dialect.max_output_field, before_dialect.max_output_field,
            "输出上限字段名也被改回来了（覆盖真的生效）"
        );
        assert!(after.caps.input.image, "手填的 input 必须真的进 caps");
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
        assert_eq!(ci.caps.dialect.id, "openai_effort");

        // 表里**完全没收录**的那对：手填照样生效（这就是这块存在的理由）
        let mut u = LlmConfig::default();
        u.base_url = "https://unknown.example.com/v1".into();
        u.model = "my-gateway-model".into();
        let before_u: yekitrim_llm::LlmParams = (&u).into();
        assert_eq!(before_u.caps.dialect.id, "none", "未知模型保守默认");
        assert!(!before_u.caps.input.image, "未知模型不许被猜成多模态");
        u.models.insert(
            "my-gateway-model".into(),
            ModelOverrideToml { thinking_wire: Some("openai_effort".into()), ..Default::default() },
        );
        let after_u: yekitrim_llm::LlmParams = (&u).into();
        assert_eq!(after_u.caps.dialect.id, "openai_effort");
    }

    /// 枚举 ↔ TOML 键名是契约（设置页与配置文件都按这个走）。
    #[test]
    fn link_keys_are_stable() {
        let keys: Vec<&str> = LlmLink::ALL.iter().map(|l| l.key()).collect();
        assert_eq!(keys, vec!["detect", "polish", "correct", "title", "utility"]);
        // v1 的四条（金标准对拍用）—— 顺序不许变
        let legacy: Vec<&str> = LlmLink::LEGACY_V1.iter().map(|l| l.key()).collect();
        assert_eq!(legacy, vec!["detect", "polish", "correct", "utility"]);
        // 界面行：**有 Title、没有 Correct**
        let ui: Vec<&str> = LlmLink::ui_visible().iter().map(|l| l.key()).collect();
        assert_eq!(ui, vec!["detect", "polish", "title", "utility"]);
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
fn model_override_by<'a>(c: &'a LlmConfig, model: &str) -> Option<&'a ModelOverrideToml> {
    let id = model.trim();
    if id.is_empty() {
        return None;
    }
    c.models
        .get(id)
        .or_else(|| c.models.iter().find(|(k, _)| k.eq_ignore_ascii_case(id)).map(|(_, v)| v))
}

/// 把手填能力盖到内置表查出来的 caps 上（**只覆盖写了的字段**）。
fn apply_model_override(caps: &mut yekitrim_llm::ModelCaps, ov: &ModelOverrideToml) {
    use yekitrim_llm::params::Support;
    // 推理写法 / 输出字段名：**任一改了就按 (写法, 字段名) 重新查一条方言**
    // （"怎么发"只由 llm crate 的注册表决定，这里不写任何 wire 字面量）。
    if ov.thinking_wire.is_some() || ov.max_output_wire.is_some() {
        let id = ov.thinking_wire.as_deref().unwrap_or(caps.dialect.id);
        let field = ov.max_output_wire.as_deref().unwrap_or(caps.dialect.max_output_field);
        caps.dialect = yekitrim_llm::dialect::resolve(Some(id), Some(field));
    }
    // 输入模态：认不出的串忽略；一个都认不出 ⇒ 回到"只有文字"
    if let Some(items) = &ov.input {
        caps.input = yekitrim_llm::InputModalities::parse(items.iter().map(String::as_str));
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

impl LlmProvider {
    /// 这份配置的明文 key：显式 `api_key` 优先，其次 `api_key_env` 指的环境变量。
    ///
    /// ⚠️ 两条都空 ⇒ 空串（**别报错**：测试连接 / 探针要能自己给一句人话）。
    /// 这是**唯一**取值实现 —— `From<&LlmConfig>`、server 的 `resolve_llm_conn`、精修各命令的
    /// "配没配 key" 判定都读它，别再自己去拼 `api_key` / `api_key_env`。
    pub fn provider_key(&self) -> String {
        if !self.api_key.trim().is_empty() {
            return self.api_key.clone();
        }
        let var = self.api_key_env.trim();
        if var.is_empty() {
            return String::new();
        }
        std::env::var(var).unwrap_or_default()
    }
}

/// 旧名字（`From<&LlmConfig>` 里还在用）：转调 [`LlmProvider::provider_key`]。
fn provider_key(p: &LlmProvider) -> String {
    p.provider_key()
}

/// 模板里那个占位 key（老 llm.toml 模板带的）：**等于没配**。
const PLACEHOLDER_API_KEY: &str = "sk-your-api-key-here";

impl LlmConfig {
    /// **当前生效的那把 key**：`active` 那份 provider 的（`LlmProvider::provider_key`），否则 legacy 顶层。
    ///
    /// ⚠️ 多配置（`config_version = 3`）之后密钥在 `providers[].api_key` 里、**顶层恒空** ——
    /// 任何"配没配 key"的判断都必须走这里（或 [`Self::has_api_key`]），**别再直接读 `self.api_key`**：
    /// 2026-10-20 的事故就是"LLM 剪辑读顶层 key ⇒ 恒空 ⇒ 静默空跑、作业显示完成"。
    pub fn effective_api_key(&self) -> String {
        match self.active_provider() {
            Some(p) => p.provider_key(),
            None => self.api_key.clone(),
        }
    }

    /// **配没配 key** 的唯一判定入口（占位值 = 没配，与老行为一致）。
    pub fn has_api_key(&self) -> bool {
        let k = self.effective_api_key();
        let k = k.trim();
        !k.is_empty() && k != PLACEHOLDER_API_KEY
    }

    /// **当前生效的模型 id**（active 那份的，否则顶层）。日志/doctor 用，别拿空的 `self.model` 去打印。
    pub fn effective_model(&self) -> &str {
        match self.active_provider() {
            Some(p) => p.model.as_str(),
            None => self.model.as_str(),
        }
    }

    /// **当前生效的 base_url**（同上）。
    pub fn effective_base_url(&self) -> &str {
        match self.active_provider() {
            Some(p) => p.base_url.as_str(),
            None => self.base_url.as_str(),
        }
    }
}

/// 把 provider **模型目录**里那一条盖到 caps 上（只覆盖它说了的字段）。
///
/// 顺序：内置表 → **provider 的模型目录**（这里）→ [llm.models."<id>"] 老覆盖（最后、优先级最高）。
/// 后者留着的理由：它是用户明确的"我要覆盖这个模型"，而且与 provider 无关（换 provider 也照用）。
fn apply_provider_model(caps: &mut yekitrim_llm::ModelCaps, m: &LlmModel) {
    if let Some(n) = m.context_window {
        caps.context_window = Some(n);
    }
    if let Some(n) = m.max_output {
        caps.max_output_ceiling = Some(n);
    }
    if !m.input.is_empty() {
        caps.input = yekitrim_llm::InputModalities::parse(m.input.iter().map(String::as_str));
    }
    if !m.reasoning {
        // 明确说"这个模型不推理" ⇒ 档位表清空（**不是留 None**：那是"没说"）
        caps.levels.clear();
    } else if !m.levels.is_empty() {
        caps.levels.clear();
        for l in &m.levels {
            match yekitrim_llm::ReasoningLevel::parse(&l.level) {
                Some(lv) => {
                    let wire = l.wire_value.trim();
                    // 空串 = 支持但不发送（与表里 null 同义）
                    caps.levels.insert(lv, (!wire.is_empty()).then(|| wire.to_string()));
                }
                None => tracing::warn!(
                    "provider 模型 {} 的档位名认不出：{:?}（已丢掉，不影响其余档位）",
                    m.id,
                    l.level
                ),
            }
        }
    }
}

impl From<&LlmConfig> for yekitrim_llm::LlmParams {
    fn from(c: &LlmConfig) -> Self {
        use yekitrim_llm::params::Support;
        // ── 选源头（R4 的唯一分叉）──────────────────────────────────────────
        // active 命中 ⇒ 那一份 provider；否则**完全走原来的路径**（顶层字段，语义一个都不改）。
        let provider = c.active_provider();
        let (api_key, base_url, model) = match provider {
            Some(p) => (provider_key(p), p.base_url.clone(), p.model.clone()),
            None => (c.api_key.clone(), c.base_url.clone(), c.model.clone()),
        };
        // 请求参数面：provider 自己写了就用它，否则顶层 [llm.params]（见 effective_params）
        let p = c.effective_params();
        // 档位：老拼写（enabled / disabled）与档位名都认 —— 手改的文件不该因此静默失效。
        // ⚠️ reasoning_explicit 必须跟着置位：链路自己的老开关（如 [correct] thinking）
        //    只在 LLM 层**没表态**时才生效（2026-09 修过的一个真 bug）。
        let reasoning = p
            .reasoning
            .as_deref()
            .and_then(yekitrim_llm::ReasoningLevel::from_thinking_flag);
        // 能力：**内置表打底 → provider 模型目录 → 手填覆盖**（表里没收录的模型靠后两层）。
        let mut caps = yekitrim_llm::catalog::caps_for(&base_url, &model);
        if let Some(prov) = provider {
            if let Some(m) = prov
                .models
                .iter()
                .find(|m| m.id.trim().eq_ignore_ascii_case(model.trim()))
            {
                apply_provider_model(&mut caps, m);
            }
        }
        if let Some(ov) = model_override_by(c, &model) {
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
            api_key,
            base_url,
            model,
            temperature: p.temperature.filter(|t| t.is_finite()).unwrap_or_default(),
            // 省略 = 不发（v1 那个"永远会发出去的默认值"由迁移落成显式键，见 version.rs）
            max_tokens: p.max_tokens,
            reasoning: reasoning.unwrap_or_default(),
            // 预算：省略 = None ⇒ 传输层按档位映射表给默认（见 dialect::budget_for）
            thinking_budget: p.thinking_budget.filter(|n| *n > 0),
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
