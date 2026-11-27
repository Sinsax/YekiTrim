//! config：全站配置类型（`types.rs`）与两层合并 / 安全写入（本文件）。
//!
//! 入口：`load_config` / `load_config_at` / `load_global_config` / `load_llm_config`、`patch_config_file`（设置页唯一保存通道）、`write_project_config`、`save_*_path_in`、`resolve_reference_titles`。
//! 边界：改字段必须走四步（字段 + `merge()` 条件 + 手动 `Default` impl + 模板/注释）；`patch_config_file` 是递归深合并，别退回"一层 section" —— 见 `crates/yekitrim-core/src/config/AGENTS.md`

mod types;
pub mod llm_v4;
pub mod version;
pub use types::*;
pub use version::{clean_retired_keys_in, ConfigKind, CONFIG_VERSION, VERSION_KEY};

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::Deserialize;

use crate::path::PROJECT_CONFIG_FILE;

/// 解析参考标题：优先使用 `[llm.links.detect.prompt].reference_titles`；
/// 其次从它的 `prompt_template` 的“参考标题：”段落中兼容解析（老文件搬过来的那种格式）；
/// 都为空时回退到内建示例 `detection::DEFAULT_REFERENCE_TITLES`。
///
/// ⚠️ S4 起这些文本住 `llm.toml`（`[llm.links.detect.prompt]`），入参从 `&DetectionConfig`
/// 换成了 `Option<&LlmPromptCfg>`（`None` = 没配 ⇒ 内建默认）。**兼容解析那一段要留着**：
/// 老文件的 prompt_template 里确实可能有“参考标题：”段落，迁移只是搬文本、不重写内容。
pub fn resolve_reference_titles(prompts: Option<&crate::config::llm_v4::LlmPromptCfg>) -> Vec<String> {
    let empty = crate::config::llm_v4::LlmPromptCfg::default();
    let prompt = prompts.unwrap_or(&empty);
    let mut out: Vec<String> = Vec::new();
    for raw in &prompt.reference_titles {
        for line in raw.lines() {
            let line = line.trim().trim_start_matches('▶').trim();
            if !line.is_empty() {
                out.push(line.to_string());
            }
        }
    }
    if !out.is_empty() {
        return out;
    }
    let Some(pos) = prompt.prompt_template.find("参考标题：") else {
        return default_reference_titles();
    };
    let tail = &prompt.prompt_template[pos + "参考标题：".len()..];
    for line in tail.lines() {
        let line = line.trim();
        if line.is_empty() {
            if !out.is_empty() {
                break;
            }
            continue;
        }
        if line.starts_with("输出") || line.starts_with("转录片段") || line.starts_with("只输出")
        {
            break;
        }
        // 新版内建 prompt 该段落是待替换的占位符：标题在运行时注入，
        // 静态段落视为空，停在占位符处（其后的正文不能当成标题）
        if line == "{reference_titles}" {
            break;
        }
        let cleaned = line.trim_start_matches('▶').trim();
        if !cleaned.is_empty() {
            out.push(cleaned.to_string());
        }
    }
    if out.is_empty() {
        return default_reference_titles();
    }
    out
}

fn default_reference_titles() -> Vec<String> {
    crate::detection::DEFAULT_REFERENCE_TITLES
        .iter()
        .map(|s| s.to_string())
        .collect()
}

/// 全局默认配置模板（`yekitrim config init` 生成 .yekitrim/config.toml 用，`yekitrim init` 的基底）
/// 包含 {{CORRECT_SYSTEM_PROMPT}} 等占位符，由生成方替换为内建默认 prompt。
pub const GLOBAL_CONFIG_TOML: &str = r#"# YekiTrim 全局默认配置 — 所有工程共享
# 相同参数会被 yekitrim.toml 覆盖
# ⚠️ config_version 是**配置结构版本**：程序读它决定要不要跑迁移。
#    手工改配置时**别动这一行**（改了会让迁移跑错；见 config/version.rs）。
config_version = 7

[asr]
# ASR 模式: "single"（SenseVoice 单模型）| "hybrid"（SenseVoice 打轴 + qwen3 填词）
# 默认 hybrid；预处理页有下拉可直接改（写进项目 yekitrim.toml），设置页表单/源码也能改
mode = "hybrid"
# 并行转写块数 (0=auto, 手动值不超过 CPU 核数)
num_workers = 0
# 音频分块长度(秒)
chunk_duration = 60
# 块间重叠(秒，仅固定分块模式有效)
chunk_overlap = 10.0
# 强停顿阈值(秒)：相邻字停顿超过它就直接切一条（与标点无关）
segment_gap = 1.0
# 气口阈值(秒)：相邻字停顿达到它算一个「气口」。断句时只用它来定位切点
# （标点决定"在哪切"，气口决定"切在哪一刻"），它本身不单独切分
breath_gap = 0.25
# 句中标点（，、；：）软切的时间阈值(秒)：距上一条边界超过它才在这些标点处切
soft_cut_secs = 6.0
# 单条字幕字数硬上限（只在没有标点/气口可用时兜底）
max_segment_chars = 30
# 标点边界吸附到气口的半径(秒)
snap_window = 0.3
# 切给识别器的音频前后余量(秒)。**默认 0 = 关** —— 实测 0.3s 会扰动识别文本、
# 对齐命中率反而下降（净效果不明），所以不默认开。想试就设 0.1~0.3。
segment_padding = 0.0
# qwen3 全局输出上限(token)，固定值。上游默认 128；本项目取 256 作为稳定折中。
# 如果以后调大 max_segment_chars，需要同步调大这个值。
max_new_tokens = 256
# LLM 订正转录文本（需配置 [llm]）
llm_correct = false
# single 模式先用 VAD(silero) 只挑语音段解码：静音/BGM 不解，快 2~3 倍。
# hybrid 不受影响（它的时间几乎全在 qwen3 上）。模型随 `yekitrim download` 一起下（约 209KB）。
vad = true
# silero 语音概率阈值(0~1)：调高=更保守（多解码、少省时间）
vad_threshold = 0.5
# 动态电平兜底(dB)：低于「本片语音电平 − 此值」的空档才允许跳过，0=关掉兜底。
# 兜底只会多解码（宁可多花时间也不漏字幕）；实测 16 能捞回 8 漏掉的短促插话，代价 +1.6% 解码量。
vad_rescue_db = 16.0

[correct]
# 订正 system_prompt（修改此处覆盖内建默认）
system_prompt = "{{CORRECT_SYSTEM_PROMPT}}"
# 订正 user_prompt（{global_context}/{window_context}/{segments} 会被自动替换）
prompt = "{{CORRECT_USER_PROMPT}}"
# ⚠️ thinking / window_size / max_concurrent 已搬到 .yekitrim/llm.toml 的
#    [llm.links.correct]（思考档位）与 [llm.links.correct.run]（窗口 / 并发）—— S4
# 自定义热词（主播名/游戏/歌曲/梗，订正时优先使用）
hotwords = []

[detection]
# 主播名称 (非空时自动追加到切片标题末尾)
vtuber_name = ""
# 弹幕密度统计窗口(秒)
danmaku_window_secs = 10
# ⚠️ 这一节只剩与 LLM 无关的项。提示词三件（system_prompt / prompt_template /
#    reference_titles）、评分权重 llm_score_weight、段级并发 max_concurrent
#    已搬到 .yekitrim/llm.toml 的 [llm.links.detect.prompt] / [llm.links.detect.run]（S4）——
#    「LLM 怎么跑」只在一个文件里找。别在这里加回来（迁移只跑一次，写回来也没人读）。

# ── 分散事件合并（检测第二趟；2026-10-06 新增，默认关）──
# 第一趟只收"完整话题"（硬标准 ≥30s）；同一个话题被别的话题打断成几小段时会被吞并或不报。
# 打开后**再跑一趟低标准检测**，把小块都报出来（每块带 topic 归组键），全部窗口跑完后按
# topic + 时间相邻合并成一条多段事件：无字幕的空隙**保留**成连续区间（交给「去静音」屏蔽），
# 有内容的空隙用 ranges 丢掉；合并后**仍不达标就丢弃**。详见 docs/plan/scattered-events-merge.md
[detection.scattered]
enabled = false
# 单块时长下限/上限（秒）：低于下限直接丢；上限只写进 prompt 当引导
min_secs = 8.0
max_secs = 60.0
# 单块字幕条数下限（硬判据）与弹幕密度下限（0 = 不看弹幕；与条数是"或"关系）
min_subs = 2
dm_floor = 0.15
# 判定"两块之间那一段算有内容"的门槛：字幕秒数超过它、或弹幕密度超过它 ⇒ 空隙要丢掉
gap_sub_secs = 0.5
gap_dm_floor = 0.3
# 合并：同话题相邻块的最大间隔（秒）/ 合并后内容时长下限 / 字幕条数下限 / 跨度上限
merge_gap_secs = 45.0
merge_min_secs = 20.0
merge_min_subs = 3
max_merge_secs = 240.0
# 要求 LLM 给每块输出 topic 归组键（合并靠它，不靠标题字面）
topic_key = true
# 第二趟的 prompt（留空 = 内建默认；这两项只留在本文件，不写进项目 yekitrim.toml）
system_prompt = ""
prompt_template = ""

[cutting]
# 输出目录
output_dir = "./output"
# 已有片段要不要覆盖：false = 目标目录里已经是这条事件的成片就**跳过**（不覆盖、不重剪，
# 日志里汇总一句）；true = 一律重剪。切片页点「剪切」的单条重切**不受它约束**（永远覆盖）。
# 别人的片段（标题与区间都对不上）无论这个开关怎样都会被拦下，不会覆盖。
overwrite = false
# 磁盘预检阈值（GB）：管线每个阶段开工前、以及切片的每个批次边界，都会检查输出盘
# （output_dir 所在卷）的可用空间；低于它就**停止整个阶段**并明确报错，剩余片段不派发
# —— 宁可停批也不能把盘写穿、被外部磁盘看门狗按最旧优先把源录播删掉
# （2026-11-15 事故：全切写穿数据盘 → diskguard 删源）。机器上还跑着别的看门狗时，
# 这个值必须设得**比看门狗阈值更高**才拦得住。0 = 关闭预检。
min_free_gb = 2

[sfx]
# 音效屏蔽列表 (yekitrim polish 使用, 全局共享)
# 精确 id 屏蔽单个音效; "tag:xxx" 屏蔽整个标签族 (如 "tag:surprise" 屏蔽所有恐怖系)
# 项目 polish.toml 的 [sfx] blacklist 可再追加
blacklist = []

[storage]
# 服务端托管项目根目录（多用户；留空 = <exe_dir>/.yekitrim/projects）
# 结构: <projects_root>/users/<uid>/<room>/<session>-<pid>/
projects_root = ""
# 全局磁盘上限（字节，0 = 不限）。只用于"入库/下载/渲染前预检"，
# 绝不自动删除用户数据（源录播默认保留）。
max_disk_bytes = 0
# 磁盘水位百分比：写完这一份后剩余空间低于该比例才拒绝新的入库/下载/渲染
# 保留量会被钳在 512 MiB ~ 8 GiB——比例只对中小容量卷起作用，
# 大容量数据盘上不会要求保留几十上百 GB（0 = 关闭水位检查）
disk_watermark_pct = 10
# 单用户默认配额（字节，0 = 不限；users.quota_bytes 优先）
default_quota_bytes = 0
# 管理员"从服务器本机文件新建项目"的**快捷入口**目录（绝对路径）
# 管理员可以直接浏览/选择任意位置（与单用户时代一致），这里只是给前端下拉预置常用根
# 例: local_roots = ["F:/media", "D:/BililiveRecorder"]
# 该功能只读取服务器上已存在的文件，永不接受上传；普通用户仍只能用 <projects_root>/users/<uid>
local_roots = []
# ---- 保留期（服务端每日维护：启动后 10 分钟跑一次，之后每 24 小时一次）----
# 只删**超过保留期**的行/文件，并打一条 info 说明删了多少；0 = 关闭该项清理（一个都不删）。
# audit 是安全审计用途，默认留 90 天（要长期留档请自己导出，别设 0 —— 那张表会一直长）。
audit_keep_days = 90        # audit 表保留天数（登录/改密/邀请码等动作）
usage_keep_days = 365       # llm_usage 表保留天数（设置页「LLM 用量」按年看）
log_keep_days = 30          # logs/yekitrim-YYYYMMDD.log 保留天数（render-*.log 另有一套，按份数）

[stickers]
# 贴纸包（zip）的体积上限，单位 MiB。默认 1 GiB 上传 / 2 GiB 解压总量。
# 上传走 WebUI「素材库 → 贴纸包」（POST /api/polish/stickers/upload），
# 解压闸门见 yekitrim-polish::assets::sticker_zip（条目数固定 2000）。
# 0 或非法值按默认值处理；上传上限的硬顶是 4096 MiB（路由层 DefaultBodyLimit 在启动时定死，
# 改完这个值要重启服务才生效）。
upload_max_mb = 1024        # 单个 zip 上传的请求体上限（1 GiB）
extract_max_mb = 2048       # 解压后所有文件合计上限（2 GiB）
file_max_mb = 64            # 解压后单个文件上限（64 MiB）

[ingest]
# 录播姬 webhook：启用后 POST /api/ingest/bililive/<token> 才会被接受
# token 用 `yekitrim ingest token` 生成（只显示一次明文，这里只存哈希）
webhook_enabled = false
token_hash = ""
# 来源 IP 白名单（空 = 不限制；同机部署建议 ["127.0.0.1", "::1"]）
allow_ips = []

# B站链接通道（P4）：
# 下载器引擎。"yt-dlp" = 内置（版本 + sha256 钉死，随包分发）；
# 其它值 = 逃生舱，写「yt-dlp 兼容可执行文件的绝对路径」（用户自备，不随包分发）。
downloader = "yt-dlp"
# yt-dlp 可执行文件位置覆盖（留空 = 自动发现：配置目录 → <程序目录>/.yekitrim/bin/ → PATH）
ytdlp_path = ""
# 最大清晰度："1080p" | "720p" | "480p" | "best"（best = 不限高）
max_quality = "1080p"
# 限速 KB/s（0 = 不限）→ --limit-rate <n>K
rate_limit_kbps = 0
# 同时下载数。**别放大**：B站 412 风控与 IP 频次相关
max_concurrent_downloads = 1
# B站 cookie 文件（**Netscape 格式**；JSON 会静默失效）。
# 无 cookie 时清晰度封顶 480/720p，部分视频直接需要登录。
cookie_file = ""
# 下载卡死判定：连续这么多秒一行输出都没有就 kill（唯一的子进程兜底）
download_idle_timeout_secs = 120

# 出网代理（下载 yt-dlp 二进制这类境外大文件用；留空 = 直连）
#   支持 http:// / https:// / socks5:// / socks5h://（socks5h = 远程 DNS，墙外建议用它）
#   例：proxy = "socks5h://127.0.0.1:20170"
proxy = ""

[ingest.recorder]
# 录制软件：bililive（录播姬）| bililive-tools
#   bililive      → api_base + api_user/api_pass（HTTP Basic），目录按路径里的房间号归属
#   bililive-tools → api_base + api_key（Authorization: <key>）+ container_root，用户/录像走它的接口
kind = "bililive"
# 录像根目录（必须与 yekitrim server 同机或同可读盘）
root = ""
# 录制软件的 HTTP API（用户端「配置直播间 / 列录像」靠它）：
#   bililive: api_base 例 "http://127.0.0.1:2356"；api_user/api_pass = BREC_HTTP_BASIC_USER/PASS
#   bililive-tools: api_base 例 "http://127.0.0.1:18010"；api_key = 它的访问密钥（Authorization 原样）
api_base = ""
api_user = ""
api_pass = ""
api_key = ""
# 仅 bililive-tools：它接口里的容器路径前缀（例 /app/video），剥掉后拼到 root 上
container_root = ""

[ingest.bililive_tools]
# biliLive-tools（另一个录制软件，只读对接）：填 API 地址 + 访问密钥即可列录制用户与录像；
# root/container_root 用于把它的容器路径映射到宿主机（YekiTrim 要读得到）
api_base = ""
api_key = ""
root = ""
container_root = ""
# 兜底扫描间隔（分钟，0 = 关闭）：覆盖漏推送 / 录播姬重启 / 历史录播导入
watch_interval_mins = 10
# SessionEnded 之后再等多久没有新 FileClosed 就认为本场结束
finalize_idle_mins = 5
# 落位：项目与产物**建在录像所在目录**（不搬源文件，故该目录需要可写）
# 多分片会话合并为单个 source.mp4（ffmpeg -f concat -c copy）
concat_files = true
# 认定为录像的扩展名
extensions = ["flv", "mkv", "mp4", "ts"]
# 入库后自动执行到哪一步：none | transcribe | detect | cut
auto_pipeline = "detect"

[ingest.webhook]
# 通用"录制完成"入口：POST /api/ingest/webhook/<token>
# 与录播姬共用 [ingest] 的 webhook_enabled / token_hash / allow_ips。
# 录像根目录（宿主机绝对路径）；空 = 未启用。事件里的路径必须落在它下面。
root = ""
# 如果录制软件给的是容器内路径（例 biliLive-tools 的 /app/video/...），把容器前缀填这里；
# 非空时先剥掉前缀，再按 root 解析。
container_root = ""
# 建项目后自动执行到哪一步：none | transcribe | detect | cut
# 默认 none：通用入口不知道来源贵不贵，先建项目、让用户自己点。
auto_pipeline = "none"

[publish.bililive_tools]
# 出片回传：YekiTrim 把成片复制到 biliLive-tools 看得见的暂存目录，再调它的投稿 API。
# 空 = 未启用。B站登录/分区/标签/封面/重试/分P 全在 biliLive-tools 里配。
base_url = ""
# 600 权限文件，内容是一行 BILILIVE_TOOLS_PASSKEY（别把密钥写进这个文件）
passkey_file = ""
# biliLive-tools 的上传预设 id（在它的 UI 里维护分区/标签/合集）
upload_preset_id = "default"
# 成片暂存目录（宿主机路径；biliLive-tools 容器要能读到）
staging_dir = ""
# 同一个目录在 biliLive-tools 容器内的路径（例 /app/video/_yekitrim_publish）
container_staging_root = ""
# B站账号 uid（0 = 自动取 /user/list 的第一个）
uid = 0

[auth]
# 自助注册（邀请码）：开启后 /api/auth/register 才受理注册请求，
# 但**必须**有可用邀请码才能注册成功 —— 码本身才是凭证，这里只是总闸。
# 邀请码用 `yekitrim user invite` 生成（只显示一次明文，库里只存 selector + 哈希）
registration_enabled = false
# 多人形态（默认 false = 单人优先）：关着时界面隐藏邀请码 / 账号配额 / 注册入口 / 任务指派对象
# （接口与 CLI 全都还在，只是不露出）。一个人用就别开 —— 前端经 /api/health.multi_user 读它。
multi_user = false
"#;

/// LLM 配置模板（`yekitrim config init` 生成 .yekitrim/llm.toml 用）。
///
/// ⚠️ 形状是 **v2**：**请求参数面**（`[llm.params]` + `[llm.overrides.<link>]`）与
/// **能力面**（`[llm.models."<id>"]`）分开。老文件由 `version.rs` 的 v1→v2 搬过来，不用手改。
/// 这里的 `config_version = 4` 必须与 [`super::version::CONFIG_VERSION`] 一致 ——
/// 写小了的话新装的文件会被"升级"一次（多一个 .bak-v1），写大了则是本程序读不懂自己的模板。
pub const LLM_CONFIG_TOML: &str = r#"# YekiTrim LLM 配置 — 独立于工程配置（v5：接口 / 模型清单 / 默认参数 / 链路覆盖 + 链路提示词与行为）
# ⚠️ 配置结构版本，别手改（见 config/version.rs）
config_version = 7

[llm]
# 当前用哪一份接口（= 下面 [[llm.providers]] 的 id；留空 = 第一份）
active = "main"
# 并发请求数——事件检测 / 精修事件生成 / 字幕订正**共用这一个值**
#（各链路要单独压，用 [llm.links.<link>.run] max_concurrent）
max_concurrent = 3
# ⚠️ 额外请求参数**按模型**写在 [[llm.models]].extra_params（2026-11-26 起取消全局那份）。

# ── 请求参数：唯一基准（**省略 = 不发送**）──────────────────────────────
[llm.defaults]
# 温度（0 = 确定性输出）；**删掉这一行 = 不发 temperature**（交给服务端默认）
temperature = 0
# 输出上限：没被 [llm.links.*] 覆盖的链路用它。这只是「跑飞的上限」，模型自己会停。
max_tokens = 16384
# 推理档位：off|minimal|low|medium|high|xhigh|max（省略 = 不表态）
# reasoning = "high"
# 预算型方言的 token 数（怎么发由模型能力面的方言决定）；省略 = 按档位给默认
# thinking_budget = 16384
# top_p = 1.0
# json_mode = true

# ── 接口：只描述「怎么连」──────────────────────────────────────────
[[llm.providers]]
id = "main"
name = "主力接口"
# 协议 id（可选：openai-chat）；留空 = 默认那条
protocol = "openai-chat"
base_url = ""
# 明文 key 与「从环境变量读」二选一；**明文永不出接口**（设置页只显示已设置/未设置）
api_key = ""
# api_key_env = "MY_LLM_KEY"
# 这份接口的默认模型（[llm.links.*].model 没写时用它）
model = ""

# ── 模型清单：有哪些模型、属谁、能力如何（唯一真相）──────────────────
# ⚠️ 内置能力表只做「建议值」；这里写了的才说了算。(provider, id) 唯一。
# [[llm.models]]
# id = "example-model"
# provider = "main"
# label = "显示名（可空）"
# overrides = { thinking_wire = "openai_effort", max_output_wire = "max_tokens" }
# 这一支模型专用的额外请求参数（整表原样进请求体；只放上面表达不了的私有键）
# extra_params = { repetition_penalty = 1.05, seed = 42 }

# ── 按链路覆盖：这条链路用哪个接口 / 哪个模型 / 怎么发 ─────────────────
# 留空 = 跟随 [llm] active 与那份接口的默认模型；参数缺一项 = 跟随 [llm.defaults]。
[llm.links.detect]
max_tokens = 8192

# ── 检测链路的**提示词**（S4：从 config.toml 的 [detection] 搬来）──────────
# ⚠️ 它们**不是请求参数**（不进请求体）；参考标题也喂给封面标题生成。
# 清空 = 用内建默认值（改内建默认只对新装生效，这里是副本）。
[llm.links.detect.prompt]
# 检测 system_prompt
system_prompt = "{{DETECT_SYSTEM_PROMPT}}"
# 检测 user prompt_template（{segments} / {reference_titles} 会被替换）
prompt_template = "{{DETECT_USER_PROMPT}}"
# 参考标题示例（每项一条，可含 emoji；留空 = 内建示例）
reference_titles = [
]

# ── 检测链路的**本地行为参数**（S4：从 config.toml 的 [detection] 搬来）────
# ⚠️ 同样不进请求体：它们决定"本机怎么跑"，不是"发给模型什么"。
[llm.links.detect.run]
# 标题+摘要 LLM 评分在最终热度里的权重（0 = 关闭该标准、也不再发那次请求）
# 最终 popularity = (1-w)*弹幕/字幕热度 + w*LLM评分
llm_score_weight = 0.3
# 段级并发（留空 = 跟随 [llm] max_concurrent）
# max_concurrent = 3

[llm.links.polish]
max_tokens = 32768

[llm.links.correct]
max_tokens = 8192
# 思考档位（off|minimal|low|medium|high|xhigh|max；留空 = 不表态）
# S4 之前这里是 config.toml 的 [correct] thinking = true/false：
#   true ⇒ reasoning = "high"；false ⇒ **什么都不写**（写 "off" 会变成"显式关思考"）
# reasoning = "high"

# ── 订正链路的本地行为参数 ────────────────────────────────────────
[llm.links.correct.run]
# Agent 窗口大小（每次处理条数；留空 = 代码表缺省 25）
# window_size = 25
# 段级并发（留空 = 跟随 [llm] max_concurrent）
# max_concurrent = 3

# [llm.links.title]
# 换一条链路用便宜模型示例：
# provider = "cheap"
# model = "small-model"
# [llm.links.utility]"#;

/// 生成 yekitrim.toml 项目配置文件（`yekitrim init`）。
///
/// * `cfg_path` — 输出配置文件路径
/// * `input` — 输入视频路径（相对 cfg_path 目录时写相对路径）
/// * `danmaku` — 弹幕路径；None 时自动发现同目录同名 .ass
/// * `output_dir` — cutting.output_dir 覆盖值
pub fn write_project_config(
    cfg_path: &Path,
    input: &Path,
    danmaku: Option<&Path>,
    output_dir: Option<&str>,
) -> Result<()> {
    write_project_config_inner(cfg_path, input, danmaku, output_dir, true)
}

/// 同 [`write_project_config`]，但**不做 `.ass` 自动发现**：`danmaku = None` 就是真的空。
///
/// 给"按来源类型预载"的调用方用 —— 服务器本机文件来源（`local`）按用户要求不预载弹幕，
/// 由预处理页列候选让用户自己选（`service::assets::preload_policy`）。
/// ⚠️ 别把这条规则塞回 `write_project_config`：CLI 的 `yekitrim init` 语义是"顺手带上同名 .ass"，
/// 两条路要分开。
pub fn write_project_config_exact(
    cfg_path: &Path,
    input: &Path,
    danmaku: Option<&Path>,
    output_dir: Option<&str>,
) -> Result<()> {
    write_project_config_inner(cfg_path, input, danmaku, output_dir, false)
}

/// 由**全局程序配置文本**生成本工程配置的正文（`[project]` 块之外的部分）。
///
/// ⚠️ **工程配置只允许带"按工程变化"的段**：`[asr]` / `[detection]` / `[cutting]` / `[sfx]`。
/// 其余段一律剥掉，原因是**安全 + 逻辑**双重的：
///
/// 1. 工程目录会被共享、搬到 NAS、发给别人（webhook 模式下它甚至直接建在录像目录里）。
///    整份拷贝会把 `[ingest] token_hash` / `[ingest.recorder].api_pass` /
///    `[publish.bililive_tools] passkey_file` / `[auth]` / `[storage]` 抄进每个工程
///    —— 实测 demo 工程的 `yekitrim.toml` 里就是这些。
/// 2. `merge()` 把工程段当**覆盖值**：管理员后来改了全局口令，老工程会拿旧副本顶回去
///    （项目优先），于是"设置页改了没生效"这句话永远查不到原因。
///
/// 剥离清单：`[llm]`（独立 llm.toml）/ `[correct]`（订正 prompt 只住程序配置）/
/// `[auth]` / `[publish]` / `[storage]` / `[stickers]` / `[ingest]`（含 `ingest.*` 子表）。
/// `[detection]` 里的 prompt 也剥掉 —— S4 起它们干脆不住 config.toml 了（住 llm.toml 的
/// `[llm.links.detect.prompt]`），但老工程文件里可能还留着副本，这里照旧清一遍。
fn project_config_body(global_text: &str, output_dir: Option<&str>) -> Result<String> {
    let mut doc: toml_edit::DocumentMut = global_text.parse()?;
    // 程序级段（其中多个带凭证）：不是工程该覆盖的东西，别抄进工程文件。
    for section in ["llm", "correct", "auth", "publish", "storage", "stickers", "ingest"] {
        doc.remove(section);
    }
    // ⚠️ **版本键不在这里插**：本函数的输出会被拼在 `[project]` 表头**之后**，而 TOML 里
    // 表头之后的键一律归那张表 —— 版本键会落成 `project.config_version`，根层反而没有。
    // 根层那一行由 `write_project_config_inner_with` 统一负责（并顺手摘掉这里残留的旧键）。

    // 老工程文件里可能还抄着这些「LLM 行为键」：S4 起它们**不属于工程级**，能剥就剥。
    // （新模板里已经没有这几行了，这里只是不把老副本继续往下带。）
    if let Some(detection) = doc.get_mut("detection") {
        if let Some(table) = detection.as_table_mut() {
            for k in [
                "system_prompt",
                "prompt_template",
                "reference_titles",
                "llm_score_weight",
                "max_concurrent",
            ] {
                table.remove(k);
            }
            // 分散事件第二趟的两份 prompt 同理：只留在 `.yekitrim/config.toml`
            if let Some(sc) = table.get_mut("scattered").and_then(|v| v.as_table_mut()) {
                sc.remove("system_prompt");
                sc.remove("prompt_template");
            }
        }
    }

    if let Some(dir) = output_dir {
        doc["cutting"]["output_dir"] = toml_edit::value(dir);
    }

    Ok(doc.to_string())
}

fn write_project_config_inner(
    cfg_path: &Path,
    input: &Path,
    danmaku: Option<&Path>,
    output_dir: Option<&str>,
    auto_discover_danmaku: bool,
) -> Result<()> {
    write_project_config_inner_with(
        cfg_path,
        input,
        danmaku,
        output_dir,
        auto_discover_danmaku,
        &project_global_config_text(),
    )
}

/// 生成工程配置时「从哪份程序配置抄」——**唯一权威** = `config::global_config_path()`
/// （= 当前 owner 的 `users/<uid>/.yekitrim/config.toml`）。
///
/// ⚠️ 这里以前读的是**已退役的顶层** `<数据>/.yekitrim/config.toml`（`path::yekitrim_home()`）：
/// 那份在 2026-11-13 收敛时已改名成 `*.migrated-<日期>.bak`，于是新建工程永远回退内建模板；
/// 而模板里 `[asr] mode = "hybrid"` 这类**字符串**在 `merge()` 里是「非空即覆盖」⇒ 用户全局设的
/// `single` 会被新工程顶回去（实测）。来源必须与读写两侧同一个函数，别再自己拼路径。
fn project_global_config_text() -> String {
    project_global_config_text_from(&global_config_path())
}

/// [`project_global_config_text`] 的可注入版本（单测用：不碰环境变量与真实数据目录）。
fn project_global_config_text_from(path: &Path) -> String {
    if path.is_file() {
        match std::fs::read_to_string(path) {
            Ok(text) => {
                tracing::info!("从程序配置复制: {}", path.display());
                return text;
            }
            Err(e) => tracing::warn!(
                "读取程序配置失败（{}），工程模板回退内建默认: {e:#}",
                path.display()
            ),
        }
    } else {
        tracing::info!("程序配置尚不存在（{}），工程模板回退内建默认", path.display());
    }
    GLOBAL_CONFIG_TOML.to_string()
}

fn write_project_config_inner_with(
    cfg_path: &Path,
    input: &Path,
    danmaku: Option<&Path>,
    output_dir: Option<&str>,
    auto_discover_danmaku: bool,
    global_config_text: &str,
) -> Result<()> {
    let global_body = global_config_text.replace(
        "# YekiTrim 全局默认配置 — 所有工程共享\n# 相同参数会被 yekitrim.toml 覆盖",
        "",
    );
    // ⚠️ **只保留允许工程覆盖的段**（安全：程序级凭证不许抄进工程文件；逻辑：
    // 项目段会在 merge() 里盖住全局，管理员改全局口令后老工程拿旧副本顶回去）。
    // 程序配置语法坏掉不该让 init 整体失败（`ensure_global_config` 那边是备份后重建）：
    // 告警并回退内建模板，至少工程建得出来。
    let body = match project_config_body(&global_body, output_dir) {
        Ok(body) => body,
        Err(e) => {
            tracing::warn!("程序配置无法用作工程模板（{e:#}），回退内建模板");
            project_config_body(GLOBAL_CONFIG_TOML, output_dir)?
        }
    };

    // 构建 [project] 块（带注释），放在最前面
    let config_dir = cfg_path.parent().unwrap_or(Path::new("."));
    let path_to_rel = |p: &Path| -> String {
        if let Ok(rel) = p.strip_prefix(config_dir) {
            rel.to_string_lossy().to_string()
        } else {
            p.to_string_lossy().to_string()
        }
    };
    let danmaku_val = match danmaku {
        Some(df) => path_to_rel(df),
        None if auto_discover_danmaku => {
            let auto_ass = input.with_extension("ass");
            if auto_ass.exists() {
                tracing::info!("自动发现弹幕文件: {}", auto_ass.display());
                path_to_rel(&auto_ass)
            } else {
                String::new()
            }
        }
        None => String::new(),
    };

    let mut project = toml_edit::Table::new();
    project.set_implicit(true);
    let input_for_config =
        if let Ok(rel) = input.strip_prefix(cfg_path.parent().unwrap_or(Path::new("."))) {
            rel.to_string_lossy().to_string()
        } else {
            input.to_string_lossy().to_string()
        };
    project.insert("input_file", toml_edit::value(&input_for_config));
    project.insert("danmaku_file", toml_edit::value(&danmaku_val));
    project.insert("transcript_file", toml_edit::value(""));
    project.insert("audio_file", toml_edit::value(""));
    project.insert("events_file", toml_edit::value(""));

    let mut header = toml_edit::DocumentMut::new();
    header.insert("project", toml_edit::Item::Table(project));
    let mut project_str = header.to_string();

    // 给 project 字段添加注释
    let comments = [
        ("input_file", "输入视频文件路径"),
        ("danmaku_file", "弹幕文件路径"),
        ("transcript_file", "转写文件路径 (由 transcribe 写入)"),
        ("audio_file", "音频文件路径 (由 extract 写入)"),
        ("events_file", "事件文件路径 (由 detect 写入)"),
    ];
    for (key, comment) in &comments {
        let line = format!("{} = ", key);
        if project_str.contains(&line) {
            let commented = format!("# {}\n{}", comment, line);
            project_str = project_str.replace(&line, &commented);
        }
    }

    let full = format!(
        "# YekiTrim 项目配置 — 优先级高于 .yekitrim/config.toml\n# 相同的参数以本文件为准\n\n{}",
        project_str.trim()
    ) + "\n\n"
        + body.trim();
    // ⚠️ **`config_version` 必须在根层**：TOML 里表头之后的键一律归那张表，而 `[project]` 在前面
    // —— 直接拼就会写成语义上的 `project.config_version`，根层反而没有（`version_of` 读到 0，
    // 每次读都被当成 v0 重跑迁移，且「文件版本更高就不动」那道保护永不触发）。
    // 所以拼完解析一遍：把 `[project]` 里可能残留的版本键摘掉，再补到**根层最前面**。
    let mut doc: toml_edit::DocumentMut = full
        .parse()
        .context("生成的工程配置不是合法 TOML（内部错误）")?;
    if let Some(project) = doc.get_mut("project").and_then(|v| v.as_table_mut()) {
        project.remove(VERSION_KEY);
    }
    let full = version::ensure_version_line(&doc.to_string());
    std::fs::write(cfg_path, full)?;
    Ok(())
}

/// 读一份配置并解析。**读之前先跑版本迁移**（`config::version`）：
/// 老文件会在这里补上 `config_version` 并清掉已下线的键（程序级还会写回 + 备份）。
fn load_from_file(path: &PathBuf, kind: ConfigKind) -> Result<Option<Config>> {
    if !path.exists() {
        return Ok(None);
    }
    let prepared = version::prepare(path, kind)
        .with_context(|| format!("failed to read {}", path.display()))?;
    let config: Config = toml::from_str(&prepared.text)
        .with_context(|| format!("failed to parse {}", path.display()))?;
    Ok(Some(config))
}

fn merge(base: &mut Config, overlay: Config) {
    // ⚠️ 默认值一律从类型自己拿（`*Config::default()`），**别再写字面量**：改默认值时漏改这里，
    // 症状是「配置里写了也不生效 / 默认值把用户值打回去」，而且一声不响（没有报错、没有日志）。
    let audio_d = crate::config::types::AudioConfig::default();
    let asr_d = crate::config::types::AsrConfig::default();
    let det_d = crate::config::types::DetectionConfig::default();
    // Project fields
    if overlay.project.input_file.is_some() {
        base.project
            .input_file
            .clone_from(&overlay.project.input_file);
    }
    if overlay.project.danmaku_file.is_some() {
        base.project
            .danmaku_file
            .clone_from(&overlay.project.danmaku_file);
    }
    if overlay.project.transcript_file.is_some() {
        base.project
            .transcript_file
            .clone_from(&overlay.project.transcript_file);
    }
    if overlay.project.audio_file.is_some() {
        base.project
            .audio_file
            .clone_from(&overlay.project.audio_file);
    }
    if overlay.project.events_file.is_some() {
        base.project
            .events_file
            .clone_from(&overlay.project.events_file);
    }
    // 素材清单是**项目级**的（全局 config.toml 不放它）：项目文件里非空才覆盖。
    // ⚠️ 新增 `project.*` 字段时必须在这里补一条 —— 漏了不会报错，
    // 只是项目文件里写了也永远读不到（merge 后仍是默认值）。
    if !overlay.project.materials.transcript.is_empty() {
        base.project
            .materials
            .transcript
            .clone_from(&overlay.project.materials.transcript);
    }
    if !overlay.project.materials.danmaku.is_empty() {
        base.project
            .materials
            .danmaku
            .clone_from(&overlay.project.materials.danmaku);
    }

    if overlay.audio.sample_rate != audio_d.sample_rate {
        base.audio.sample_rate = overlay.audio.sample_rate;
    }
    if overlay.audio.channels != audio_d.channels {
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
    if overlay.asr.chunk_duration != asr_d.chunk_duration {
        base.asr.chunk_duration = overlay.asr.chunk_duration;
    }
    if overlay.asr.chunk_overlap != asr_d.chunk_overlap {
        base.asr.chunk_overlap = overlay.asr.chunk_overlap;
    }
    if overlay.asr.segment_gap != asr_d.segment_gap {
        base.asr.segment_gap = overlay.asr.segment_gap;
    }
    if let Some(v) = overlay.asr.llm_correct {
        base.asr.llm_correct = Some(v);
    }
    if !overlay.asr.language.is_empty() {
        base.asr.language = overlay.asr.language.clone();
    }
    if !overlay.asr.model.is_empty() {
        base.asr.model = overlay.asr.model.clone();
    }
    // `segment_padding` = 切给识别器的音频余量，**默认 0（关）**：
    // 实测 0.3s 会扰动文本、对齐命中率反而下降，净效果不明（见 types.rs 的字段注释）
    if overlay.asr.segment_padding != asr_d.segment_padding {
        base.asr.segment_padding = overlay.asr.segment_padding;
    }
    if overlay.asr.max_new_tokens != asr_d.max_new_tokens {
        base.asr.max_new_tokens = overlay.asr.max_new_tokens;
    }
    if !overlay.asr.mode.is_empty() {
        base.asr.mode = overlay.asr.mode.clone();
    }
    if overlay.asr.breath_gap != asr_d.breath_gap {
        base.asr.breath_gap = overlay.asr.breath_gap;
    }
    if overlay.asr.soft_cut_secs != asr_d.soft_cut_secs {
        base.asr.soft_cut_secs = overlay.asr.soft_cut_secs;
    }
    if overlay.asr.max_segment_chars != asr_d.max_segment_chars {
        base.asr.max_segment_chars = overlay.asr.max_segment_chars;
    }
    if overlay.asr.snap_window != asr_d.snap_window {
        base.asr.snap_window = overlay.asr.snap_window;
    }
    // VAD：`Option<bool>` 与 `llm_correct` 同款 —— 只有**显式写了**才覆盖，
    // 这样"全局关、单个工程开"（或反过来）都表达得出来（bool 会掉进"≠默认才覆盖"的坑）。
    if let Some(v) = overlay.asr.vad {
        base.asr.vad = Some(v);
    }
    if overlay.asr.vad_threshold != asr_d.vad_threshold {
        base.asr.vad_threshold = overlay.asr.vad_threshold;
    }
    if overlay.asr.vad_rescue_db != asr_d.vad_rescue_db {
        base.asr.vad_rescue_db = overlay.asr.vad_rescue_db;
    }
    if overlay.detection.danmaku_window_secs != det_d.danmaku_window_secs {
        base.detection.danmaku_window_secs = overlay.detection.danmaku_window_secs;
    }
    if !overlay.detection.vtuber_name.is_empty() {
        base.detection.vtuber_name = overlay.detection.vtuber_name;
    }
    // ⚠️ S4：提示词 / 评分权重 / 段级并发**不在这里 merge** —— 它们住 llm.toml
    // （`[llm.links.detect.prompt]` / `[llm.links.detect.run]`），而 `[llm]` 本来就不参与
    // 工程级合并（`load_config_at` 最后一句无条件覆盖）。别再给它们加回覆盖条件。
    // 分散事件合并（`[detection.scattered]`）：项目里写过的才覆盖程序默认。
    // 口径与本节其余字段一致：bool 取**或**（不能靠"关"去覆盖程序里的开）、
    // 数值/字符串"非默认才覆盖"（否则项目里的空段会把程序配置抹平）。
    {
        let o = &overlay.detection.scattered;
        let b = &mut base.detection.scattered;
        let d = crate::config::types::ScatteredConfig::default();
        if o.enabled {
            b.enabled = true;
        }
        if o.min_secs != d.min_secs {
            b.min_secs = o.min_secs;
        }
        if o.max_secs != d.max_secs {
            b.max_secs = o.max_secs;
        }
        if o.min_subs != d.min_subs {
            b.min_subs = o.min_subs;
        }
        if o.dm_floor != d.dm_floor {
            b.dm_floor = o.dm_floor;
        }
        if o.gap_sub_secs != d.gap_sub_secs {
            b.gap_sub_secs = o.gap_sub_secs;
        }
        if o.gap_dm_floor != d.gap_dm_floor {
            b.gap_dm_floor = o.gap_dm_floor;
        }
        if o.merge_gap_secs != d.merge_gap_secs {
            b.merge_gap_secs = o.merge_gap_secs;
        }
        if o.merge_min_secs != d.merge_min_secs {
            b.merge_min_secs = o.merge_min_secs;
        }
        if o.merge_min_subs != d.merge_min_subs {
            b.merge_min_subs = o.merge_min_subs;
        }
        if o.max_merge_secs != d.max_merge_secs {
            b.max_merge_secs = o.max_merge_secs;
        }
        if o.topic_key {
            b.topic_key = true;
        }
        if !o.system_prompt.is_empty() {
            b.system_prompt = o.system_prompt.clone();
        }
        if !o.prompt_template.is_empty() {
            b.prompt_template = o.prompt_template.clone();
        }
    }

    if !overlay.correct.system_prompt.is_empty() {
        base.correct.system_prompt = overlay.correct.system_prompt;
    }
    if !overlay.correct.prompt.is_empty() {
        base.correct.prompt = overlay.correct.prompt;
    }
    // ⚠️ S4：`thinking` / `window_size` / `max_concurrent` 已搬进 llm.toml 的
    // `[llm.links.correct]` 与 `[llm.links.correct.run]` —— 这里不再有对应的覆盖条件。
    if !overlay.correct.hotwords.is_empty() {
        // 热词追加合并（全局 + 项目）
        for w in &overlay.correct.hotwords {
            if !base.correct.hotwords.contains(w) {
                base.correct.hotwords.push(w.clone());
            }
        }
    }

    if !overlay.cutting.output_dir.is_empty() {
        base.cutting.output_dir = overlay.cutting.output_dir;
    }
    if let Some(v) = overlay.cutting.overwrite {
        base.cutting.overwrite = Some(v);
    }
    // 磁盘预检阈值：非默认才覆盖 —— 写 0 是"显式关闭"，要能盖过全局的开启
    if overlay.cutting.min_free_gb != CuttingConfig::default().min_free_gb {
        base.cutting.min_free_gb = overlay.cutting.min_free_gb;
    }

    // 音效屏蔽列表: 追加合并 (全局 + 项目)
    for id in &overlay.sfx.blacklist {
        if !base.sfx.blacklist.contains(id) {
            base.sfx.blacklist.push(id.clone());
        }
    }

    // [stickers] 是服务端专用段（项目配置里不写，见 project_config_body 的剥离清单），
    // 仍按"非默认值覆盖"合并 —— 与 [storage] 同一条口径。
    let stickers_d = crate::config::types::StickersConfig::default();
    if overlay.stickers.upload_max_mb != stickers_d.upload_max_mb {
        base.stickers.upload_max_mb = overlay.stickers.upload_max_mb;
    }
    if overlay.stickers.extract_max_mb != stickers_d.extract_max_mb {
        base.stickers.extract_max_mb = overlay.stickers.extract_max_mb;
    }
    if overlay.stickers.file_max_mb != stickers_d.file_max_mb {
        base.stickers.file_max_mb = overlay.stickers.file_max_mb;
    }


    // [storage] 是服务端专用段（项目配置里不写），但仍按"非默认值覆盖"合并，
    // 以免有人在 yekitrim.toml 里试写时行为诡异
    if !overlay.storage.projects_root.is_empty() {
        base.storage.projects_root = overlay.storage.projects_root;
    }
    if overlay.storage.max_disk_bytes != 0 {
        base.storage.max_disk_bytes = overlay.storage.max_disk_bytes;
    }
    if overlay.storage.disk_watermark_pct != 10 {
        base.storage.disk_watermark_pct = overlay.storage.disk_watermark_pct;
    }
    if overlay.storage.default_quota_bytes != 0 {
        base.storage.default_quota_bytes = overlay.storage.default_quota_bytes;
    }
    for root in &overlay.storage.local_roots {
        if !base.storage.local_roots.contains(root) {
            base.storage.local_roots.push(root.clone());
        }
    }
    // 保留期：与上面同一条口径（非默认值才覆盖）。默认值必须从 `Default` 现取 ——
    // 写成字面量 90/365/30 的话，以后改默认天数就只在"没有项目覆盖"那条路上生效。
    let storage_d = crate::config::types::StorageConfig::default();
    if overlay.storage.audit_keep_days != storage_d.audit_keep_days {
        base.storage.audit_keep_days = overlay.storage.audit_keep_days;
    }
    if overlay.storage.usage_keep_days != storage_d.usage_keep_days {
        base.storage.usage_keep_days = overlay.storage.usage_keep_days;
    }
    if overlay.storage.log_keep_days != storage_d.log_keep_days {
        base.storage.log_keep_days = overlay.storage.log_keep_days;
    }

    // [ingest]：同样只在项目配置显式写了非默认值时才覆盖
    if overlay.ingest.webhook_enabled {
        base.ingest.webhook_enabled = true;
    }
    if !overlay.ingest.token_hash.is_empty() {
        base.ingest.token_hash = overlay.ingest.token_hash.clone();
    }
    for ip in &overlay.ingest.allow_ips {
        if !base.ingest.allow_ips.contains(ip) {
            base.ingest.allow_ips.push(ip.clone());
        }
    }
    // `kind` 的默认是 "bililive"（不是空串）⇒ 只有写成另一家才覆盖，否则默认值会把值打回去
    if !overlay.ingest.recorder.kind.trim().is_empty() && overlay.ingest.recorder.kind != "bililive" {
        base.ingest.recorder.kind = overlay.ingest.recorder.kind.clone();
    }
    if !overlay.ingest.recorder.root.is_empty() {
        base.ingest.recorder.root = overlay.ingest.recorder.root.clone();
    }
    if !overlay.ingest.recorder.api_key.is_empty() {
        base.ingest.recorder.api_key = overlay.ingest.recorder.api_key.clone();
    }
    if !overlay.ingest.recorder.container_root.is_empty() {
        base.ingest.recorder.container_root = overlay.ingest.recorder.container_root.clone();
    }
    if overlay.ingest.recorder.watch_interval_mins != 10 {
        base.ingest.recorder.watch_interval_mins = overlay.ingest.recorder.watch_interval_mins;
    }
    if overlay.ingest.recorder.finalize_idle_mins != 5 {
        base.ingest.recorder.finalize_idle_mins = overlay.ingest.recorder.finalize_idle_mins;
    }
    if !overlay.ingest.recorder.concat_files {
        base.ingest.recorder.concat_files = false;
    }
    if !overlay.ingest.recorder.extensions.is_empty() {
        for ext in &overlay.ingest.recorder.extensions {
            if !base.ingest.recorder.extensions.contains(ext) {
                base.ingest.recorder.extensions.push(ext.clone());
            }
        }
    }
    if overlay.ingest.recorder.auto_pipeline != "detect" {
        base.ingest.recorder.auto_pipeline = overlay.ingest.recorder.auto_pipeline.clone();
    }
    // biliLive-tools（独立于录播姬）：非空才覆盖
    if !overlay.ingest.bililive_tools.api_base.is_empty() {
        base.ingest.bililive_tools.api_base = overlay.ingest.bililive_tools.api_base.clone();
    }
    if !overlay.ingest.bililive_tools.api_key.is_empty() {
        base.ingest.bililive_tools.api_key = overlay.ingest.bililive_tools.api_key.clone();
    }
    if !overlay.ingest.bililive_tools.root.is_empty() {
        base.ingest.bililive_tools.root = overlay.ingest.bililive_tools.root.clone();
    }
    if !overlay.ingest.bililive_tools.container_root.is_empty() {
        base.ingest.bililive_tools.container_root = overlay.ingest.bililive_tools.container_root.clone();
    }
    // 出网代理：非空才覆盖
    if !overlay.ingest.proxy.is_empty() {
        base.ingest.proxy = overlay.ingest.proxy.clone();
    }
    // 通用 webhook（与录播姬是两套根目录；`root` 空 = 未启用）
    if !overlay.ingest.webhook.root.is_empty() {
        base.ingest.webhook.root = overlay.ingest.webhook.root.clone();
    }
    if !overlay.ingest.webhook.container_root.is_empty() {
        base.ingest.webhook.container_root = overlay.ingest.webhook.container_root.clone();
    }
    if overlay.ingest.webhook.auto_pipeline != "none" {
        base.ingest.webhook.auto_pipeline = overlay.ingest.webhook.auto_pipeline.clone();
    }

    // 出片回传（biliLive-tools 投稿）；空 = 未启用
    if !overlay.publish.bililive_tools.base_url.is_empty() {
        base.publish.bililive_tools.base_url = overlay.publish.bililive_tools.base_url.clone();
    }
    if !overlay.publish.bililive_tools.passkey_file.is_empty() {
        base.publish.bililive_tools.passkey_file = overlay.publish.bililive_tools.passkey_file.clone();
    }
    if overlay.publish.bililive_tools.upload_preset_id != "default" {
        base.publish.bililive_tools.upload_preset_id = overlay.publish.bililive_tools.upload_preset_id.clone();
    }
    if !overlay.publish.bililive_tools.staging_dir.is_empty() {
        base.publish.bililive_tools.staging_dir = overlay.publish.bililive_tools.staging_dir.clone();
    }
    if !overlay.publish.bililive_tools.container_staging_root.is_empty() {
        base.publish.bililive_tools.container_staging_root = overlay.publish.bililive_tools.container_staging_root.clone();
    }
    if overlay.publish.bililive_tools.uid != 0 {
        base.publish.bililive_tools.uid = overlay.publish.bililive_tools.uid;
    }

    if !overlay.ingest.recorder.api_base.is_empty() {
        base.ingest.recorder.api_base = overlay.ingest.recorder.api_base.clone();
    }
    if !overlay.ingest.recorder.api_user.is_empty() {
        base.ingest.recorder.api_user = overlay.ingest.recorder.api_user.clone();
    }
    if !overlay.ingest.recorder.api_pass.is_empty() {
        base.ingest.recorder.api_pass = overlay.ingest.recorder.api_pass.clone();
    }

    // [ingest] 的 B站下载通道（P4）：同样是"非空才覆盖"。
    // `max_quality` / `rate_limit_kbps` / `max_concurrent_downloads` 属于**平台级资源策略**
    // （风控与磁盘），工程配置不该改它 —— 与 [storage] 同规矩，不参与 overlay。
    if !overlay.ingest.downloader.is_empty() && overlay.ingest.downloader != "yt-dlp" {
        base.ingest.downloader = overlay.ingest.downloader.clone();
    }
    if !overlay.ingest.ytdlp_path.is_empty() {
        base.ingest.ytdlp_path = overlay.ingest.ytdlp_path.clone();
    }
    if !overlay.ingest.cookie_file.is_empty() {
        base.ingest.cookie_file = overlay.ingest.cookie_file.clone();
    }

    // [auth]：与 [ingest] 同一规矩 —— 只有显式写了 true 才覆盖（开关只可能被打开，不会被静默关掉）；
    // 真正生效的是全局配置，工程配置一般不该碰它。
    if overlay.auth.registration_enabled {
        base.auth.registration_enabled = true;
    }
    // 多人形态开关同理：**只认 true**（默认单人）。写反了（非空/!= false 才覆盖）会让
    // `Config::default()` 的 false 在合并时把用户设的 true 打回去 —— 前端会静默退回单人界面。
    if overlay.auth.multi_user {
        base.auth.multi_user = true;
    }
}


// ── 明文脱敏 + 写盘前守卫（P0，2026-10-19）────────────────────────────────────

/// 把一段**可能夹着明文 key** 的文本抹成"可以打日志/回接口"的样子。
///
/// 为什么要它在 core 里：TOML 解析错误**会带出错那一行的原文**（
/// `params = { json_mode = "", api_key = "sk-…" }` 这种一行写全的配置就是明文泄漏）——
/// 而这种错误文本出现在**两个 crate** 的日志里（core 读配置、server 写配置）。
/// **只此一份实现**（server 的 `redact_api_key` 已改成转调这里）。
///
/// 口径：行首的、inline table 里的、注释掉的 `api_key = "…"` 全抹；键名/缩进/行数一律保留。
pub fn redact_secrets(raw: &str) -> String {
    raw.split('\n').map(redact_secrets_line).collect::<Vec<_>>().join("\n")
}

fn redact_secrets_line(line: &str) -> String {
    const TOKEN: &str = "api_key";
    let mut out = String::with_capacity(line.len());
    let mut rest = line;
    while let Some(pos) = rest.find(TOKEN) {
        let before = &rest[..pos];
        out.push_str(before);
        out.push_str(TOKEN);
        let mut tail = &rest[pos + TOKEN.len()..];
        // 键名边界：`api_key_extra` 这种前缀相同的不许误伤
        let boundary = !before
            .chars()
            .last()
            .map(|c| c.is_alphanumeric() || c == '_')
            .unwrap_or(false);
        let trimmed = tail.trim_start_matches([' ', '\t']);
        let ws = tail.len() - trimmed.len();
        if !boundary || !trimmed.starts_with('=') {
            rest = tail;
            continue;
        }
        out.push_str(&tail[..ws]);
        out.push('=');
        tail = &trimmed[1..];
        let trimmed = tail.trim_start_matches([' ', '\t']);
        out.push_str(&tail[..tail.len() - trimmed.len()]);
        // 值：带引号 ⇒ 只抹引号里的内容（引号/键名/缩进原样留）
        if let Some(q) = trimmed.chars().next().filter(|c| *c == '"' || *c == '\'') {
            if let Some(rel) = trimmed[1..].find(q) {
                out.push(q);
                out.push(q);
                let after = &trimmed[1 + rel + 1..];
                let after_trimmed = after.trim_start();
                // 行尾注释里也可能写着 key ⇒ 一并抹掉；inline table 的后续键继续扫
                if after_trimmed.starts_with('#') || after_trimmed.is_empty() {
                    return out;
                }
                rest = after;
                continue;
            }
        }
        // 裸值 / 说不清 ⇒ 抹到行尾（宁可多抹一个注解，不可漏一个字）
        return out;
    }
    out.push_str(rest);
    out
}

/// **写盘前守卫**：把 `llm.toml` 文本用**真正的反序列化器**吃一遍。
///
/// 为什么必须有（P0 事故 2026-10-19）：保存路径把 `json_mode = ""`（字符串）写进了布尔字段，
/// 文件本身是**合法 TOML**（`toml::Value` 解析得动），只有反序列化到 `LlmConfig` 才会炸 ——
/// 而炸的后果是**整份 LLM 配置回退默认值**（界面"更糟了"）。所以守卫必须用 `LlmConfig` 这层。
///
/// 返回 `Err` 时给的是**脱敏后**的人话（哪个字段 / 期望什么 / 实际什么）。
pub fn validate_llm_toml(text: &str) -> Result<(), String> {
    toml::from_str::<LlmFile>(text).map(|_| ()).map_err(|e| redact_secrets(&e.to_string()))
}

/// 从 `llm.toml` 加载 LLM 配置（独立于工程配置和全局配置）。
///
/// ⚠️ **按 owner 读**（2026-11-13）：`llm.toml` 住在 `users/<uid>/.yekitrim/`，而这里的
/// `Owner` 由 `YEKITRIM_OWNER` 继承（server 进程里是 `Local` = 当前管理员）。
/// **别再退回顶层 `<数据>/.yekitrim/llm.toml`** —— 那份是单人时代的旧位置，
/// 设置页早就不写了，读它 = 管线的 `cfg.llm` 永远没有 key（检测静默退化成"一个整段"）。
pub fn load_llm_config() -> LlmConfigV4 {
    load_llm_config_for(crate::owner::Owner::from_env())
}

#[derive(Deserialize)]
struct LlmFile {
    llm: LlmConfigV4,
}

pub fn load_config() -> Result<Config> {
    load_config_at(Path::new(PROJECT_CONFIG_FILE))
}

/// 从指定项目配置文件加载合并配置。
///
/// 与 `load_config()` 语义一致：
/// - 先加载**程序配置**（`global_config_path()` = 当前 owner 的 `config.toml`）
/// - 再用指定项目文件覆盖（项目文件不存在时按空项目处理，不报错）
/// - 最后独立加载该 owner 的 `llm.toml`
///
/// ⚠️ **2026-11-13 收敛**：程序配置只有一个位置（`owner::config_path`），不再是顶层
/// `<数据>/.yekitrim/config.toml`。这条同时是**管线**（转写/检测/剪切/订正）的读路径 ——
/// 以前它读顶层，于是「设置 → 处理」保存的参考标题、以及账号 `llm.toml` 里的 key
/// **管线一样都拿不到**（检测静默退化成"一个整段"，界面却显示配好了）。
pub fn load_config_at(project_file: &Path) -> Result<Config> {
    let mut config = Config::default();

    let global = global_config_path();
    if let Some(overlay) = load_from_file(&global, ConfigKind::Global)? {
        merge(&mut config, overlay);
    }

    // ⚠️ 工程文件**只在内存里升级**（每个工程一份、还可能在网络盘上）：见 config::version 的说明
    let project = project_file.to_path_buf();
    if let Some(overlay) = load_from_file(&project, ConfigKind::Project)? {
        merge(&mut config, overlay);
    }

    // 独立加载 LLM 配置（llm.toml 已从 config.toml 分离）
    config.llm = load_llm_config();

    Ok(config)
}

/// **程序配置的唯一位置**（`config.toml`）—— `owner::config_path(Owner::from_env())`。
///
/// 单人/托盘/CLI = `Local` = **当前管理员**（`users/<uid>/.yekitrim/config.toml`）；
/// 多人的子进程经 `YEKITRIM_OWNER` 继承身份。写侧（设置页 / 接入页 / `config init`）与
/// 读侧（`load_global_config` / `load_config_at` / `ensure_global_config`）**必须都用它** ——
/// 这就是 2026-11-13「两个位置、各被一半消费者读到」那族 bug 的收口点。
///
/// ⚠️ **别再新增"顶层 `.yekitrim/config.toml`"的第二个候选位置**：那正是这次要消灭的东西。
pub fn global_config_path() -> PathBuf {
    crate::owner::config_path(crate::owner::Owner::from_env())
}

/// 校验 `.yekitrim/config.toml` / `.yekitrim/llm.toml` 能否解析，报出**第一个**出错的文件与原因。
///
/// 为什么需要它：`load_global_config` / `load_llm_config` 都把解析失败**吞掉**换成默认值
/// （它们不能因为一份坏配置就让整个进程起不来）。但界面必须知道"你看到的这些值是默认值，
/// 不是文件里的" —— 否则设置页**一旦保存就把内建默认值覆盖到真实配置文件上**。
///
/// 所以设置页的写入门闸（`SettingsView.vue::configError`）读的就是它，
/// 而这条错误**必须指向真实文件**（以前报的是 merged 配置的错误，指向不明）。
/// 两份都不存在 = 正常（`ensure_global_config` 会补齐），不算错。
///
/// ⚠️ 与 [`config_parse_error_for`] 同一形状，只是它用**当前进程的 owner**（`YEKITRIM_OWNER`）。
pub fn global_config_parse_error() -> Option<String> {
    config_parse_error_for(crate::owner::Owner::from_env())
}

/// 只读**程序配置**（当前 owner 的 `config.toml`），不合并 `yekitrim.toml`。
/// 供不依赖项目 `yekitrim.toml` 的模块（接入 / 发布 / 容量 / 精修 / doctor …）读取。
///
/// ⚠️ **2026-11-13 起读的是 owner 路径**（`global_config_path()`），不再是顶层那份。
/// 以前它读顶层，而设置页写账号树 ⇒ 一半设置"保存成功但不生效"。
/// 等价的注入版是 [`load_config_for`]（单测用它，不碰环境变量）。
pub fn load_global_config() -> Config {
    load_config_for(crate::owner::Owner::from_env())
}

// ============================================================================
// 按归属读配置（`Owner`）—— 见 `crate::owner`
// ============================================================================
//
// ⚠️ **这里是"程序配置按用户分"的收口点**：配置路径一律经
// `crate::owner::config_path` / `state_dir`，**别在别处拼 `.yekitrim/config.toml`**。
//
// 与素材库同一条规矩：单人（`Owner::Local`）解析到**现状位置**（零迁移），
// 多用户解析到各自的 `users/<uid>/.yekitrim/`。

/// 按归属读程序配置（`config.toml`），不合并项目文件。
///
/// 单人（[`crate::owner::Owner::Local`]）等价于 [`load_global_config`] ——
/// 这条等价性有单测钉住，是"单人零迁移"的判据之一。
pub fn load_config_for(owner: crate::owner::Owner) -> Config {
    let mut config = Config::default();
    let path = crate::owner::config_path(owner);
    if let Ok(Some(overlay)) = load_from_file(&path, ConfigKind::Global) {
        merge(&mut config, overlay);
    }
    config
}

/// 按归属读 LLM 配置（`llm.toml`）。
///
/// ⚠️ 这个文件里有**明文 api_key**。按用户分之后，每个账号只读到自己那份 ——
/// 这是相对"全局一份、所有人共用同一个 key"的**安全净改善**。
pub fn load_llm_config_for(owner: crate::owner::Owner) -> LlmConfigV4 {
    let path = crate::owner::llm_config_path(owner);
    if !path.exists() {
        return LlmConfigV4::default();
    }
    let content = match version::prepare(&path, ConfigKind::Llm) {
        Ok(p) => p.text,
        Err(e) => {
            tracing::warn!("读取 {} 失败: {}，使用默认 LLM 配置", path.display(), e);
            return LlmConfigV4::default();
        }
    };
    match toml::from_str::<LlmFile>(&content) {
        Ok(f) => f.llm,
        Err(e) => {
            // ⚠️ **脱敏**：同上（错误文本会带出错行的原文）
            let why = redact_secrets(&e.to_string());
            tracing::warn!("解析 {} 失败: {}，使用默认 LLM 配置", path.display(), why);
            LlmConfigV4::default()
        }
    }
}

/// 按归属做"解析失败就说出来"的检查（设置页写入门闸的数据源）。
///
/// 单人时指向现状文件；多用户时指向该账号自己的两份。语义与
/// [`global_config_parse_error`] 完全一致，只是换了 base 目录。
pub fn config_parse_error_for(owner: crate::owner::Owner) -> Option<String> {
    let home = crate::owner::owner_root(owner);
    for (name, what) in [("config.toml", "程序配置"), ("llm.toml", "LLM 配置")] {
        let path = home.join(name);
        let Ok(raw) = std::fs::read_to_string(&path) else { continue };
        if let Err(e) = toml::from_str::<toml::Value>(&raw) {
            return Some(format!("{}（{}）解析失败，本次使用的是内建默认值: {e}", what, path.display()));
        }
    }
    None
}

// ============================================================================
// 单人 → 「一个 owner 一个位置」的一次性迁移（2026-11-13，方案 A）
// ============================================================================
//
// 背景：2026-10-10 起设置页把程序配置写进 `users/<uid>/.yekitrim/`，但 `ensure_global_config`
// 仍在生成**顶层** `<数据>/.yekitrim/config.toml`，接入页也写它 —— 两份由同一个模板长出、
// 都带全部 section，各被一半消费者读到。症状是「设置保存成功、行为完全没变」
// （用户实测：参考标题配了 6 条，检测一直用内建默认；llm.toml 账号那份有 key、管线读顶层那份空的）。
//
// 本轮的口径：**程序配置只有一个位置**（`global_config_path()` = `owner::config_path`）。
// 顶层那两份由本函数一次性搬走（**合并**后 rename 留档，不删用户数据）。

/// 顶层那份里**以它为准**的 section（运行时一直读顶层，用户在那里配的是真值）。
///
/// 其余 section（`[asr]` / `[detection]` / `[cutting]` / `[correct]` / `[audio]` /
/// `[project]` / `[sfx]`）以**账号那份**为准，只补账号缺的键 —— 设置页一直写的是它。
const TOP_LEVEL_WINS_SECTIONS: [&str; 4] = ["ingest", "storage", "publish", "auth"];

/// 把一条顶层配置文本合并进目标文本（**纯函数**，单测直接打它）。
///
/// - `dst = None`（目标不存在）⇒ 原样返回 `src`（整份搬过去，注释也保留）；
/// - 目标存在 ⇒ 逐键合并：
///   * [`TOP_LEVEL_WINS_SECTIONS`] 里的段：**src 的键覆盖 dst**（同名覆盖、缺的补上），
///     dst 独有的键保留（不丢新版本字段）；
///   * 其余段 / 顶层标量：**只补 dst 缺的键**（dst 赢）。
///
/// 为什么不是"整段覆盖"：`[ingest]` 下还有 `[ingest.recorder]` 这类子表，整段覆盖会把
/// 目标里更新版本的字段一起抹掉。逐键最保守。
fn merge_top_level_into(src_text: &str, dst_text: Option<&str>) -> Result<String> {
    let src: toml_edit::DocumentMut = src_text
        .parse()
        .context("顶层配置文件不是合法 TOML（保持原状，不迁移）")?;
    let Some(dst_text) = dst_text else {
        return Ok(src_text.to_string());
    };
    let mut dst: toml_edit::DocumentMut = dst_text
        .parse()
        .context("目标配置文件不是合法 TOML（保持原状，不迁移）")?;

    for (key, src_item) in src.iter() {
        let top_wins = TOP_LEVEL_WINS_SECTIONS.contains(&key);
        match (src_item.as_table(), dst.get_mut(key).and_then(|i| i.as_table_mut())) {
            (Some(src_tbl), Some(dst_tbl)) => {
                // 契约：先收集再写，避免同时持有 src 的借用与 dst 的可变借用
                let pairs: Vec<(String, toml_edit::Item)> = src_tbl
                    .iter()
                    .map(|(k, v)| (k.to_string(), v.clone()))
                    .collect();
                for (k, v) in pairs {
                    if top_wins || !dst_tbl.contains_key(&k) {
                        dst_tbl.insert(&k, v);
                    }
                }
            }
            // 目标没有这一段 ⇒ 整段搬（含子表与注释）
            _ => {
                if top_wins || !dst.contains_key(key) {
                    dst.insert(key, src_item.clone());
                }
            }
        }
    }
    Ok(dst.to_string())
}

/// 迁移时给顶层文件起的留档名：`config.toml.migrated-20261113.bak`（重名追加 `-2`、`-3`……）。
fn migration_archive_path(src: &Path) -> PathBuf {
    let name = src
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "config.toml".into());
    let stamp = chrono::Local::now().format("%Y%m%d");
    let dir = src.parent().unwrap_or_else(|| Path::new("."));
    let base = format!("{name}.migrated-{stamp}.bak");
    let mut candidate = dir.join(&base);
    let mut n = 1;
    while candidate.exists() && n < 100 {
        n += 1;
        candidate = dir.join(format!("{name}.migrated-{stamp}-{n}.bak"));
    }
    candidate
}

/// 把单人时代的**顶层** `<data>/.yekitrim/{config,llm}.toml` 归到 `owner` 名下。
///
/// ## 语义（幂等、不删数据、失败可回滚）
///
/// 1. 顶层文件不在 ⇒ 直接返回（跑第二遍是 no-op；全新安装天然满足）；
/// 2. 与目标**合并**（规则见 [`merge_top_level_into`]：平台段顶层赢、内容段账号赢）；
/// 3. 顶层文件 **rename** 成 `*.migrated-<日期>.bak` —— 不删除，出问题改回来就是旧形态；
/// 4. 目标文件不存在时整份搬过去（注释一并保留）。
///
/// ⚠️ **只处理 `Owner::User`**：`Local` 就是"当前管理员"，没有独立目录可归；
/// 调用方先 `set_local_admin(uid)` 再传 `Owner::User(uid)`，避免默认 uid=1 写错人。
/// ⚠️ 失败时**调用方只该 warn**：迁移不动数据，启动必须照常。
pub fn migrate_top_level_config_to(
    owner: crate::owner::Owner,
    data_home: &Path,
) -> Result<Vec<PathBuf>> {
    if !matches!(owner, crate::owner::Owner::User(_)) {
        return Ok(Vec::new());
    }
    let top = data_home.join("legacy-config");
    let dest = crate::owner::owner_root_at(owner, data_home);
    if top == dest {
        return Ok(Vec::new());
    }
    let mut archived = Vec::new();
    for name in ["config.toml", "llm.toml"] {
        let src = top.join(name);
        if !src.is_file() {
            continue;
        }
        let src_text = std::fs::read_to_string(&src)
            .with_context(|| format!("读取 {} 失败", src.display()))?;
        let dst = dest.join(name);
        let dst_text = std::fs::read_to_string(&dst).ok();
        let merged = merge_top_level_into(&src_text, dst_text.as_deref())?;
        std::fs::create_dir_all(&dest)
            .with_context(|| format!("创建配置目录失败: {}", dest.display()))?;
        // 只有真变了才写：没变化时一个字节都不动（用户的格式/注释原样保留）
        if dst_text.as_deref() != Some(merged.as_str()) {
            // 先写临时文件再 rename —— 写一半的文件会让**整份**配置被当成解析失败
            let tmp = dst.with_extension("toml.migrating");
            std::fs::write(&tmp, &merged)
                .with_context(|| format!("写入 {} 失败", tmp.display()))?;
            std::fs::rename(&tmp, &dst)
                .with_context(|| format!("落盘 {} 失败", dst.display()))?;
        }
        let bak = migration_archive_path(&src);
        std::fs::rename(&src, &bak)
            .with_context(|| format!("留档 {} 失败", src.display()))?;
        archived.push(bak);
    }
    Ok(archived)
}

/// 用 toml_edit 将 events_file 写入当前目录的 yekitrim.toml
pub fn save_events_path(path: &str) -> Result<()> {
    save_events_path_in(&PathBuf::from(PROJECT_CONFIG_FILE), path)
}

/// 用 toml_edit 将 audio_file 写入当前目录的 yekitrim.toml
pub fn save_audio_path(path: &str) -> Result<()> {
    save_audio_path_in(&PathBuf::from(PROJECT_CONFIG_FILE), path)
}

/// 用 toml_edit 将 transcript_file 写入当前目录的 yekitrim.toml
pub fn save_transcript_path(path: &str) -> Result<()> {
    save_transcript_path_in(&PathBuf::from(PROJECT_CONFIG_FILE), path)
}

/// 用 toml_edit 将 events_file 写入指定项目配置文件
pub fn save_events_path_in(project_file: &Path, path: &str) -> Result<()> {
    edit_toml_at(project_file, |doc| {
        doc["project"]["events_file"] = toml_edit::value(path);
    })
}

/// 用 toml_edit 将 audio_file 写入指定项目配置文件
pub fn save_audio_path_in(project_file: &Path, path: &str) -> Result<()> {
    edit_toml_at(project_file, |doc| {
        doc["project"]["audio_file"] = toml_edit::value(path);
    })
}

/// 用 toml_edit 将 transcript_file 写入指定项目配置文件
pub fn save_transcript_path_in(project_file: &Path, path: &str) -> Result<()> {
    edit_toml_at(project_file, |doc| {
        doc["project"]["transcript_file"] = toml_edit::value(path);
    })
}

fn edit_toml_at(project_file: &Path, f: impl FnOnce(&mut toml_edit::DocumentMut)) -> Result<()> {
    // 整段读-改-写放进同步路径锁：管线（同步上下文）与设置页保存会同时改这份文件，
    // 无锁时后写的会把先写的键整段覆盖（丢 `transcript_file` 之类的后果是状态机错乱）。
    crate::filelock::with_path_lock_sync(project_file, || {
        let content = if project_file.exists() {
            std::fs::read_to_string(project_file)?
        } else {
            String::new()
        };
        // ⚠️ **先跑版本迁移再改**：否则"改一个键"会把一份老配置原样留在老版本上 ——
        // 那个文件永远升不上来，里面已下线的键也永远清不掉（patch 只合并不删）。
        // 迁移失败（坏 TOML）时保持原样交给下面那段"备份 + 从空文档起"的既有处理。
        let kind = ConfigKind::of_path(project_file);
        // 读-改-写这条路同样要先跑 v4→v5 的**跨文件**搬家（config.toml → llm.toml），
        // 否则「保存一次设置」会把 LLM 行为键当历史键删掉、却一个都没搬过去（只删不搬）。
        let content = if kind == ConfigKind::Global {
            version::migrate_global_pair(project_file, &content).unwrap_or_else(|_| content.clone())
        } else {
            content
        };
        let base = match version::migrate_text(kind, &content) {
            Ok(prepared) => {
                // ⚠️ `migrate_text` 是纯函数、**不打日志**（工程级文件不写回，在纯函数里报就是刷屏）。
                //    这条路**会**把文件写回去 ⇒ 由它来报（`prepare` 那条读路径同口径）。
                version::report_retired(project_file, kind, &prepared);
                if prepared.changed && kind.persists() {
                    if let Err(e) = version::backup_before_upgrade(project_file, prepared.from) {
                        tracing::warn!("升级前备份 {} 失败（继续）：{e:#}", project_file.display());
                    }
                }
                prepared.text
            }
            Err(_) => content.clone(),
        };
        let mut doc: toml_edit::DocumentMut = match base.parse() {
            Ok(doc) => doc,
            Err(e) => {
                // 语法错误时**不能**静默从空文档开始：那样"改一个键"会把用户其余配置整段抹掉。
                // 先备份成 `*.toml.broken` 并告警，再基于空文档写入本次 patch。
                if !content.trim().is_empty() {
                    let backup = project_file.with_extension("toml.broken");
                    let _ = std::fs::copy(project_file, &backup);
                    tracing::warn!(
                        "配置 {} 不是合法 TOML（e={e}），已备份为 {} 并只写入本次改动",
                        project_file.display(),
                        backup.display()
                    );
                }
                toml_edit::DocumentMut::new()
            }
        };
        f(&mut doc);
        // 空文件初次写入（新建工程）也要带上版本号
        if version::version_of(&doc) < CONFIG_VERSION {
            doc.insert(VERSION_KEY, toml_edit::value(CONFIG_VERSION as i64));
        }
        if let Some(parent) = project_file.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(project_file, doc.to_string())?;
        Ok(())
    })
}

/// 用 toml_edit 把嵌套 patch（`{ section: { key: value } }`）合并进配置文件。
///
/// 设置页表单只覆盖常用字段，这里**只改动 patch 里出现的键**：
/// 其他 section（如 `[correct]` 订正 prompt）、注释、键顺序全部原样保留。
/// 直接用表单重建整个文件会把没暴露的配置整段删掉，这正是本函数要避免的。
///
/// **支持两层以上**（`{"ingest": {"recorder": {"api_base": …}}}`）：值是对象时递归进子表合并，
/// 不会把 `[ingest.recorder]` 里本次没提到的键丢掉。见 [`merge_into_table`] 的说明。
/// **源文件模式**的「空 key = 不修改」：按 provider 的 `id` 把已存的 key 填回**抹过值**的文本里。
///
/// 为什么需要它：`GET /api/config` 回的 `llm_raw` 把**每一条** provider 的 `api_key` 都抹成
/// `""`（见 server 的 `redact_api_key`）。用户只想改别的键、把这份原文原样存回来时，
/// 不填回去就是**一次保存把所有 provider 的真 key 全抹了**（静默毁配置）。
///
/// 口径与顶层那条一致：**空串 = 不修改**；要真删 key 就写 `null`（或把整份 provider 删掉）。
/// 找不到同 id 的已存 provider ⇒ 不动（那是新加的一份，空 key 就是空）。
///
/// 返回 `None` = 没有要填的（一个空都没匹配上 / 这份文本没有 `providers`）。
pub fn restore_llm_provider_keys(incoming: &str, stored: &str) -> Option<String> {
    use toml_edit::{DocumentMut, Item};

    // 已存文件里 id → 明文 key（只收非空的）
    let mut keys: std::collections::BTreeMap<String, String> = std::collections::BTreeMap::new();
    if let Ok(v) = toml::from_str::<toml::Value>(stored) {
        if let Some(arr) = v.get("llm").and_then(|l| l.get("providers")).and_then(|p| p.as_array()) {
            for p in arr {
                let id = p.get("id").and_then(|i| i.as_str()).unwrap_or("").trim();
                let key = p.get("api_key").and_then(|k| k.as_str()).unwrap_or("");
                if !id.is_empty() && !key.trim().is_empty() {
                    keys.insert(id.to_string(), key.to_string());
                }
            }
        }
    }
    if keys.is_empty() {
        return None;
    }
    let mut doc = incoming.parse::<DocumentMut>().ok()?;
    let arr = doc.get_mut("llm").and_then(Item::as_table_like_mut)?.get_mut("providers")?;
    let mut changed = false;
    // ⚠️ **两种形态都要认**：`[[llm.providers]]`（数组表，人写的）与
    // `providers = [{ … }]`（inline 数组，服务端 patch 写出来的）。只认前者的话，
    // 保存一次表单之后"空 key 不修改"就失效了（key 被静默抹掉）。
    let blank_key = |id: &str, existing: Option<&str>| {
        keys.get(id).filter(|_| existing.map(|s| s.trim().is_empty()).unwrap_or(true)).cloned()
    };
    match arr {
        Item::ArrayOfTables(tables) => {
            for t in tables.iter_mut() {
                let id = t.get("id").and_then(Item::as_str).unwrap_or("").trim().to_string();
                let existing = t.get("api_key").and_then(Item::as_str);
                if let Some(k) = blank_key(&id, existing) {
                    t.insert("api_key", toml_edit::value(k));
                    changed = true;
                }
            }
        }
        Item::Value(toml_edit::Value::Array(a)) => {
            for v in a.iter_mut() {
                let toml_edit::Value::InlineTable(t) = v else { continue };
                let id = t.get("id").and_then(|x| x.as_str()).unwrap_or("").trim().to_string();
                let existing = t.get("api_key").and_then(|x| x.as_str());
                if let Some(k) = blank_key(&id, existing) {
                    t.insert("api_key", toml_edit::Value::from(k));
                    changed = true;
                }
            }
        }
        _ => return None,
    }
    changed.then(|| doc.to_string())
}

pub fn patch_config_file(path: &Path, patch: &serde_json::Value) -> Result<()> {
    let sections = patch
        .as_object()
        .context("patch 必须是 { section: { key: value } } 形式的对象")?;
    edit_toml_at(path, |doc| {
        for (section, fields) in sections {
            let Some(fields) = fields.as_object() else {
                continue;
            };
            if !doc.get(section).map(|i| i.is_table()).unwrap_or(false) {
                let mut table = toml_edit::Table::new();
                table.set_implicit(false);
                doc.insert(section, toml_edit::Item::Table(table));
            }
            let Some(table) = doc.get_mut(section).and_then(|i| i.as_table_mut()) else {
                continue;
            };
            merge_into_table(table, fields);
        }
    })
}

/// 把一层 JSON 字段合并进一张 toml 表（值为对象时递归进子表）。
///
/// ⚠️ **子表必须递归下去，不能把对象一律转成 inline table 后整项 `insert`**：
/// `[ingest]` 下的 `recorder` 就是子表，整项替换会把本次 patch 里没提到的键
/// （`root` / `api_user` / `api_pass` / `watch_interval_mins` …）与**段内注释静默丢掉** ——
/// 曾经就是这个行为，而它不报错（文件仍是合法 TOML），只有下次读配置时才发现值没了。
fn merge_into_table(
    table: &mut toml_edit::Table,
    fields: &serde_json::Map<String, serde_json::Value>,
) {
    for (key, value) in fields {
        // `null` = **删掉这个键**。patch 是合并语义，没有别的办法表达"取消覆盖 / 回到跟随全局"
        // （设置页把链路覆盖输入框清空走的就是这条）。从前 null 会落到 `json_to_toml_value` 的
        // Null 分支被写成空串 —— `max_tokens = ""` 下次读配置直接解析失败。
        if value.is_null() {
            table.remove(key);
            continue;
        }
        // 值是对象：目标是子表 → 递归合并；其它形态（字符串/数组/inline table）或不存在
        // → 整体替换成一张**真子表**（渲染成 `[section.sub]`，下次还能继续递归）
        if let Some(sub) = value.as_object() {
            let already_table = matches!(table.get(key), Some(toml_edit::Item::Table(_)));
            if already_table {
                if let Some(toml_edit::Item::Table(existing)) = table.get_mut(key) {
                    merge_into_table(existing, sub);
                }
            } else {
                let mut t = toml_edit::Table::new();
                t.set_implicit(false);
                merge_into_table(&mut t, sub);
                table.insert(key, toml_edit::Item::Table(t));
            }
            continue;
        }
        let new_value = json_to_toml_value(value);
        // 就地替换 value：键的 decor（上方注释/缩进）挂在 Key 上，
        // 整项 insert 会把注释一起替换掉，保存一次注释就没了
        if let Some(existing) = table.get_mut(key).and_then(|i| i.as_value_mut()) {
            let decor = existing.decor().clone();
            *existing = new_value;
            *existing.decor_mut() = decor;
        } else {
            table.insert(key, toml_edit::Item::Value(new_value));
        }
    }
}

fn json_to_toml_value(v: &serde_json::Value) -> toml_edit::Value {
    match v {
        // `patch_config_file` 走的是"null = 删键"（见 `merge_into_table`），这里只是
        // 数组元素等边角形态的兜底 —— 别把这条路当成 null 的语义。
        serde_json::Value::Null => toml_edit::Value::from(""),
        serde_json::Value::Bool(b) => toml_edit::Value::from(*b),
        serde_json::Value::Number(n) => match n.as_i64() {
            Some(i) => toml_edit::Value::from(i),
            None => toml_edit::Value::from(n.as_f64().unwrap_or(0.0)),
        },
        serde_json::Value::String(s) => toml_edit::Value::from(s.as_str()),
        serde_json::Value::Array(arr) => {
            let mut out = toml_edit::Array::new();
            for item in arr {
                out.push(json_to_toml_value(item));
            }
            toml_edit::Value::Array(out)
        }
        serde_json::Value::Object(map) => {
            let mut out = toml_edit::InlineTable::new();
            for (k, val) in map {
                out.insert(k, json_to_toml_value(val));
            }
            toml_edit::Value::InlineTable(out)
        }
    }
}

fn rel_path_for(project_file: &Path, value: &str) -> String {
    let p = Path::new(value);
    if p.is_absolute() {
        if let Some(dir) = project_file.parent() {
            if let Ok(rel) = p.strip_prefix(dir) {
                return rel.to_string_lossy().into_owned();
            }
        }
    }
    value.to_string()
}

/// 将项目配置文件里已记录的绝对产物路径统一改写为相对路径。
///
/// 项目目录整体移动后仍可继续使用；`yekitrim init` / server 切换 / 保存阶段产物时调用。
pub fn normalize_project_paths_in(project_file: &Path) -> Result<()> {
    edit_toml_at(project_file, |doc| {
        let project = doc.get_mut("project").and_then(|p| p.as_table_mut());
        if let Some(project) = project {
            for key in [
                "input_file",
                "danmaku_file",
                "audio_file",
                "transcript_file",
                "events_file",
            ] {
                if let Some(val) = project
                    .get_mut(key)
                    .and_then(|v| v.as_str().map(|s| s.to_string()))
                {
                    if !val.is_empty() {
                        project[key] = toml_edit::value(rel_path_for(project_file, &val));
                    }
                }
            }
            // 素材清单里的路径同样要相对化：清单是写进 `yekitrim.toml` 的候选项，
            // 项目目录搬走后得跟着走（同样是「新增路径键要同步加」的那张表）。
            if let Some(materials) = project
                .get_mut("materials")
                .and_then(|v| v.as_table_mut())
            {
                for key in ["transcript", "danmaku"] {
                    let Some(arr) = materials.get_mut(key).and_then(|v| v.as_array_mut()) else {
                        continue;
                    };
                    for item in arr.iter_mut() {
                        // 清单条目由 `patch_config_file` 写成 inline table（`{ path = …, source = … }`）
                        let Some(table) = item.as_inline_table_mut() else {
                            continue;
                        };
                        let Some(raw) = table.get("path").and_then(|v| v.as_str()) else {
                            continue;
                        };
                        if raw.is_empty() {
                            continue;
                        }
                        let rel = rel_path_for(project_file, raw);
                        if rel != raw {
                            table.insert("path", toml_edit::Value::from(rel));
                        }
                    }
                }
            }
        }
    })
}

/// **改 `YEKITRIM_DATA_DIR` 的测试必须持这把锁**（进程级环境变量 + 测试并行跑）。
///
/// ⚠️ 2026-11-13 实测：`global_config_parse_error_…` 与 `load_config_at_reads_the_owner_config_and_llm`
/// 各自声明了一把 `static LOCK` —— **两个不同的 Mutex 等于没锁**，于是并行跑时互相把
/// `YEKITRIM_DATA_DIR` 指到对方目录，偶发红（跑了三次红一次）。锁必须只有一把、放在父模块。
#[cfg(test)]
pub(crate) static DATA_DIR_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(test)]
mod vad_merge_tests {
    use super::*;

    /// VAD 的合并口径：开关是 `Option<bool>`（**显式写了才覆盖**），阈值/兜底按既有的
    /// "≠默认才覆盖"。用 Option 的理由就在这里 —— 用 bool 的话"全局关、单工程开"表达不出来。
    #[test]
    fn vad_switch_is_explicit_and_thresholds_follow_default_rule() {
        let mut base = Config::default();
        base.asr.vad = Some(false);
        base.asr.vad_threshold = 0.8;
        base.asr.vad_rescue_db = 4.0;

        // 工程层写 vad = true → 必须覆盖全局的 false
        let mut overlay = Config::default();
        overlay.asr.vad = Some(true);
        merge(&mut base, overlay);
        assert_eq!(base.asr.vad, Some(true), "显式 true 覆盖全局 false");
        assert_eq!(base.asr.vad_threshold, 0.8, "工程层没写就不动全局值");
        assert_eq!(base.asr.vad_rescue_db, 4.0);

        // 工程层写了非默认的阈值/兜底 → 覆盖
        let mut overlay = Config::default();
        overlay.asr.vad_threshold = 0.7;
        overlay.asr.vad_rescue_db = 20.0;
        merge(&mut base, overlay);
        assert_eq!(base.asr.vad_threshold, 0.7);
        assert_eq!(base.asr.vad_rescue_db, 20.0);
        assert_eq!(base.asr.vad, Some(true), "工程层没写 vad 就保持不变");

        // 全是默认值的 overlay 不该改动任何 VAD 项
        let mut base2 = Config::default();
        base2.asr.vad_threshold = 0.9;
        base2.asr.vad = Some(false);
        merge(&mut base2, Config::default());
        assert_eq!(base2.asr.vad_threshold, 0.9);
        assert_eq!(base2.asr.vad, Some(false));
        assert_eq!(base2.asr.vad_rescue_db, 16.0, "默认兜底 = 16dB");
    }
}

#[cfg(test)]
mod reference_titles_tests {
    use super::*;

    fn prompts(titles: Vec<String>, template: &str) -> crate::config::llm_v4::LlmPromptCfg {
        crate::config::llm_v4::LlmPromptCfg {
            system_prompt: String::new(),
            prompt_template: template.to_string(),
            reference_titles: titles,
        }
    }

    #[test]
    fn config_titles_win_over_everything() {
        let d = prompts(vec!["自定义标题🤣".into()], "参考标题：\n▶ 旧示例");
        assert_eq!(
            resolve_reference_titles(Some(&d)),
            vec!["自定义标题🤣".to_string()]
        );
    }

    #[test]
    fn legacy_prompt_section_is_parsed() {
        let d = prompts(
            vec![],
            "前文\n参考标题：\n▶ 旧示例一🥵\n▶ 旧示例二😅\n\n输出：只输出 JSON",
        );
        assert_eq!(
            resolve_reference_titles(Some(&d)),
            vec!["旧示例一🥵".to_string(), "旧示例二😅".to_string()]
        );
    }

    #[test]
    fn placeholder_literal_is_skipped_and_builtin_used() {
        // init 写出的新版内建 prompt：该段落只剩待替换的占位符
        let d = prompts(vec![], crate::detection::DEFAULT_PROMPT);
        let resolved = resolve_reference_titles(Some(&d));
        assert_eq!(
            resolved,
            crate::detection::DEFAULT_REFERENCE_TITLES
                .iter()
                .map(|s| s.to_string())
                .collect::<Vec<_>>()
        );
        assert!(resolved.iter().all(|t| !t.contains("{reference_titles}")));
    }

    #[test]
    fn empty_everything_falls_back_to_builtin() {
        let d = prompts(vec![], "");
        assert_eq!(
            resolve_reference_titles(Some(&d)),
            crate::detection::DEFAULT_REFERENCE_TITLES
                .iter()
                .map(|s| s.to_string())
                .collect::<Vec<_>>()
        );
    }
}

/// `patch_config_file` 的合并行为（表单保存的底层契约：只动 patch 里出现的键）。
#[cfg(test)]
mod patch_config_tests {
    use super::*;

    fn write_tmp(tag: &str, body: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("yekitrim-patch-{tag}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("config.toml");
        std::fs::write(&p, body).unwrap();
        p
    }

    const SAMPLE: &str = "\
# 顶部注释不能丢
[ingest]
# webhook 开关
webhook_enabled = true
token_hash = \"scrypt$hash\"

[ingest.recorder]
# 录像目录
root = \"D:/rec\"
api_base = \"http://127.0.0.1:2356\"
api_user = \"admin\"
api_pass = \"secret\"
watch_interval_mins = 7

[detection]
vtuber_name = \"主播\"
";

    /// 嵌套 patch：只改 `[ingest.recorder]` 里提到的键，**同表其它键与注释一个都不能少**。
    ///
    /// 回归钉子：合并逻辑曾经不认识子表 —— `recorder` 被整项替换成 inline table，
    /// `root`/`api_user`/`api_pass`/`watch_interval_mins` 与段内注释静默消失（文件仍是合法
    /// TOML，所以不报错，只有下次读配置才发现值没了）。
    #[test]
    fn nested_patch_keeps_sibling_keys_and_comments() {
        let p = write_tmp("nested", SAMPLE);
        patch_config_file(
            &p,
            &serde_json::json!({"ingest": {"webhook_enabled": false,
                                           "recorder": {"api_base": "http://10.0.0.5:2356",
                                                        "api_pass": "newsecret"}}}),
        )
        .unwrap();
        let out = std::fs::read_to_string(&p).unwrap();

        // 提到的键被改
        assert!(out.contains("api_base = \"http://10.0.0.5:2356\""), "{out}");
        assert!(out.contains("api_pass = \"newsecret\""), "{out}");
        assert!(out.contains("webhook_enabled = false"), "{out}");
        // 没提到的兄弟键全在
        for keep in [
            "root = \"D:/rec\"",
            "api_user = \"admin\"",
            "watch_interval_mins = 7",
        ] {
            assert!(out.contains(keep), "丢了 {keep}：\n{out}");
        }
        // 注释与其它 section 原样
        for keep in [
            "# 顶部注释不能丢",
            "# webhook 开关",
            "# 录像目录",
            "vtuber_name = \"主播\"",
        ] {
            assert!(out.contains(keep), "丢了 {keep}：\n{out}");
        }
        assert!(
            out.contains("[ingest.recorder]"),
            "子表应保持表头形式：\n{out}"
        );
        assert!(
            !out.contains("recorder = {"),
            "不该退化成 inline table：\n{out}"
        );
        let _ = std::fs::remove_dir_all(p.parent().unwrap());
    }

    /// 文件里还没有该子表时：建出**真子表** `[ingest.recorder]`，而不是 inline table。
    /// （真子表才能被后续 patch 继续递归合并。）
    #[test]
    fn nested_patch_creates_real_subtable() {
        let p = write_tmp("newsub", "[ingest]\nwebhook_enabled = true\n");
        patch_config_file(
            &p,
            &serde_json::json!({"ingest": {"recorder": {"root": "D:/rec", "api_base": "http://x:1"}}}),
        )
        .unwrap();
        let out = std::fs::read_to_string(&p).unwrap();
        assert!(out.contains("[ingest.recorder]"), "{out}");
        assert!(!out.contains("recorder = {"), "{out}");
        // 再 patch 一次仍能命中同一张子表（合并而非追加第二张）
        patch_config_file(
            &p,
            &serde_json::json!({"ingest": {"recorder": {"api_user": "u"}}}),
        )
        .unwrap();
        let out2 = std::fs::read_to_string(&p).unwrap();
        assert_eq!(out2.matches("[ingest.recorder]").count(), 1, "{out2}");
        assert!(
            out2.contains("root = \"D:/rec\"") && out2.contains("api_user = \"u\""),
            "{out2}"
        );
        let _ = std::fs::remove_dir_all(p.parent().unwrap());
    }

    /// 一层 patch（现有全部调用方的形态）行为不变。
    #[test]
    fn flat_patch_still_works() {
        let p = write_tmp(
            "flat",
            "[cutting]\noutput_dir = \"./out\"\noverwrite = false\n",
        );
        patch_config_file(&p, &serde_json::json!({"cutting": {"overwrite": true}})).unwrap();
        let out = std::fs::read_to_string(&p).unwrap();
        assert!(out.contains("output_dir = \"./out\""), "{out}");
        assert!(out.contains("overwrite = true"), "{out}");
        // 新 section / 新键会补出来
        patch_config_file(
            &p,
            &serde_json::json!({"ingest": {"allow_ips": ["127.0.0.1"]}}),
        )
        .unwrap();
        let out = std::fs::read_to_string(&p).unwrap();
        assert!(
            out.contains("[ingest]") && out.contains("allow_ips = [\"127.0.0.1\"]"),
            "{out}"
        );
        let _ = std::fs::remove_dir_all(p.parent().unwrap());
    }

    /// `null` = **删键**（设置页把链路覆盖清空 = 回到"跟随全局"）。
    ///
    /// 回归钉子：从前 null 会落到 `json_to_toml_value` 的 Null 分支被写成空串 ——
    /// `max_tokens = ""` 是合法 TOML 但读不出 u32，`load_llm_config` 直接整份打回默认值。
    #[test]
    fn null_patch_removes_key_and_keeps_siblings() {
        // S4 起链路覆盖住 [llm.links.<link>]（与 [llm.defaults] 两级）
        let body = "\
# 顶部注释
[llm]
active = \"main\"

[llm.defaults]
max_tokens = 16384

# 链路覆盖
[llm.links.detect]
max_tokens = 8192
reasoning = \"off\"

[llm.links.polish]
max_tokens = 32768
";
        let p = write_tmp("null-del", body);
        patch_config_file(
            &p,
            &serde_json::json!({"llm": {"links": {"detect": {"max_tokens": null,
                                                              "reasoning": "high"}}}}),
        )
        .unwrap();
        let out = std::fs::read_to_string(&p).unwrap();

        assert!(!out.contains("8192"), "被 null 的键必须删掉：\n{out}");
        assert!(
            out.contains("reasoning = \"high\""),
            "同一次 patch 里的其它键照常写入：\n{out}"
        );
        for keep in [
            "# 顶部注释",
            "# 链路覆盖",
            "max_tokens = 16384",
            "[llm.links.polish]",
            "max_tokens = 32768",
        ] {
            assert!(out.contains(keep), "丢了 {keep}：\n{out}");
        }
        // 删掉之后必须还能被解析（空串那版就是在这一步炸的），且该链路回到跟随 [llm.defaults]
        #[derive(serde::Deserialize)]
        struct File {
            llm: types::LlmConfigV4,
        }
        let text = std::fs::read_to_string(&p).unwrap();
        let f: File = toml::from_str(&text).expect("删键后的 llm.toml 仍须可解析");
        assert_eq!(
            f.llm.params_for(types::LlmLink::Detect).max_tokens,
            Some(16384),
            "删掉覆盖 = 跟随 [llm.defaults].max_tokens"
        );
        assert_eq!(f.llm.params_for(types::LlmLink::Polish).max_tokens, Some(32768));
        let _ = std::fs::remove_dir_all(p.parent().unwrap());
    }

    /// **模型清单里带点号/冒号的 id 必须原样 round-trip**（v4 是 `[[llm.models]]` 的数组元素）。
    ///
    /// ⚠️ 为什么还留着这条：v3 时代模型 id 是**表键**（`[llm.models."cn:…"]`），toml_edit 对
    /// "带点号的键"有两种解释，解释错了就是**整份 llm.toml 被打回默认值**。v4 把它变成数组元素之后
    /// 这个坑理论上消失，但 id 仍是字符串常量、仍要逐字 round-trip（真实 id 就长这样）。
    #[test]
    fn patch_keeps_model_ids_with_dots_and_colons() {
        let p = write_tmp("llm-models-key", "config_version = 4\n[llm]\nactive = \"main\"\n\n[[llm.providers]]\nid = \"main\"\n");
        patch_config_file(
            &p,
            &serde_json::json!({"llm": {"models": [{
                "id": "cn:deepseek-v4.1-flash",
                "provider": "main",
                "overrides": {"thinking_wire": "thinking_type"}
            }]}}),
        )
        .unwrap();
        let out = std::fs::read_to_string(&p).unwrap();
        #[derive(serde::Deserialize)]
        struct File {
            llm: types::LlmConfigV4,
        }
        let f: File = toml::from_str(&out).expect("模型清单写进去之后，llm.toml 仍须可解析");
        let entry = f
            .llm
            .models
            .iter()
            .find(|m| m.id == "cn:deepseek-v4.1-flash")
            .expect("带点号/冒号的模型 id 必须原样 round-trip");
        assert_eq!(
            entry.overrides.as_ref().and_then(|o| o.thinking_wire.as_deref()),
            Some("thinking_type")
        );
        let _ = std::fs::remove_dir_all(p.parent().unwrap());
    }

    /// 空子表 patch（`{}`）什么都不动 —— 别把整张表（含注释）当成"没提到的键"删掉。
    #[test]
    fn empty_subtable_patch_changes_nothing() {
        let body = "[llm]\n[llm.defaults]\nmax_tokens = 16384\n\n[llm.links.detect]\nmax_tokens = 8192\n";
        let p = write_tmp("empty-sub", body);
        patch_config_file(&p, &serde_json::json!({"llm": {"links": {"detect": {}}}})).unwrap();
        let out = std::fs::read_to_string(&p).unwrap();
        assert!(out.contains("max_tokens = 8192"), "{out}");
        assert!(out.contains("[llm.links.detect]"), "{out}");
        let _ = std::fs::remove_dir_all(p.parent().unwrap());
    }

    /// 模板顶上的 config_version 必须与 CONFIG_VERSION 一致（两份模板都要）。
    ///
    /// 写小了：新装的文件第一次读就被"升级"一遍（还多一个 .bak-v1）；
    /// 写大了：本程序读不懂自己写出来的模板（投影是"每次启动都重新迁移"）。
    #[test]
    fn templates_stamp_the_current_config_version() {
        let want = format!("config_version = {CONFIG_VERSION}");
        for (name, text) in [("config.toml", GLOBAL_CONFIG_TOML), ("llm.toml", LLM_CONFIG_TOML)] {
            assert!(text.contains(&want), "{name} 模板里没有 {want}（模板与人手写的版本号会漂）");
        }
    }

    /// 模板必须是合法 TOML，且带着按链路的推荐值（新装的文件直接就是"检测 8k / 精修 32k"）。
    #[test]
    fn llm_config_template_parses_with_link_overrides() {
        #[derive(serde::Deserialize)]
        struct File {
            llm: types::LlmConfigV4,
        }
        let f: File = toml::from_str(LLM_CONFIG_TOML).expect("LLM_CONFIG_TOML 必须是合法 TOML");
        assert_eq!(
            f.llm.for_link(types::LlmLink::Detect).defaults.max_tokens,
            Some(8192),
            "检测链路应带自己的上限"
        );
        assert_eq!(f.llm.for_link(types::LlmLink::Polish).defaults.max_tokens, Some(32768));
        assert_eq!(f.llm.for_link(types::LlmLink::Correct).defaults.max_tokens, Some(8192));
        // utility 没写覆盖 → 跟随 [llm.defaults]
        assert_eq!(
            f.llm.for_link(types::LlmLink::Utility).defaults.max_tokens,
            f.llm.defaults.max_tokens
        );
        // 精修/零散单发的默认关思考由链路默认给出
        assert_eq!(
            f.llm.for_link(types::LlmLink::Polish).defaults.reasoning.as_deref(),
            Some("off")
        );
        // 模板必须自带一份接口 + 一个 active（v4 硬要求：没有接口就是非法配置）
        assert_eq!(f.llm.active, "main");
        assert!(f.llm.problems().is_empty(), "模板本身不该有配置问题: {:?}", f.llm.problems());
    }

/// `global_config_parse_error` 是设置页的**写入门闸**，必须指向真实文件、且只在真坏时报。
///
/// 它替代的是"读 merged 配置报错"那条路（2026-10-03 拆 `/api/config` 时发现：
/// `load_global_config` / `load_llm_config` 都把解析失败**吞掉**换成默认值，
/// 所以设置页原本拿不到原因 —— 那份配置被整体忽略、表单只剩内建默认值，用户不改也不知道；
/// 一旦点了保存，内建默认值就覆盖到真实文件上）。
#[test]
fn global_config_parse_error_points_at_the_real_file_and_needs_a_real_break() {
    // 这份 helper 读的是 **owner 路径**（`users/<uid>/.yekitrim/`），而 `data_dir` 跟着
    // `YEKITRIM_DATA_DIR` 走 —— 测试里指到一个临时目录，避免碰开发机上那份真实配置。
    //
    // ⚠️ 2026-11-13 起文件位置是 `users/<admin>/.yekitrim/`（不再是顶层 `.yekitrim/`）：
    // 顶层的两份已退役，写成顶层的话这条判据**测的是一份没人读的文件**。
    //
    // ⚠️ **改全局环境变量必须持锁**：测试是并行跑的，`set_var` 是进程级的 ——
    // 不持锁就会踩到同期读配置的用例（`llm::tests` 那两条就是这么被偶发打红的）。
    // ⚠️ 锁必须是 **`super::DATA_DIR_ENV_LOCK`（全文件唯一一把）**：自己再 `static` 一把
    // 等于没锁（2026-11-13 与迁移用例互相踩，三次红一次）。
    let _guard = super::DATA_DIR_ENV_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    // ⚠️ **目录名必须每轮唯一，不能只用 pid**：pid 会被系统回收，而本目录是**留在盘上的**
    // ⇒ 复用同一个 pid 的下一次运行会**继承上一轮写坏的 `llm.toml`**，
    // 于是"② 合法 TOML = 不报错"这一步会读到上一轮的残留、**偶发**报错
    // （实测：`left: Some("llm.toml 解析失败…也坏 [[[")` / `right: None`）。
    // 加上进程内自增序号 + 起始时清一次目录，保证每轮都是干净起点。
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let seq = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "yekitrim-parse-err-{}-{}",
        std::process::id(),
        seq
    ));
    // 起点干净（同 pid 同 seq 理论上不会撞，这里是纵深防御）
    let _ = std::fs::remove_dir_all(&dir);
    let uid = crate::owner::local_admin_uid();
    let home = dir.join("users").join(uid.to_string());
    std::fs::create_dir_all(&home).unwrap();
    // 顶层那份已退役：放一份**坏**的进去，确认它不再参与判定（否则这条判据会假绿/假红）
    std::fs::create_dir_all(dir.join(".yekitrim")).unwrap();
    std::fs::write(dir.join(".yekitrim/config.toml"), "顶层已退役 [[[\n").unwrap();
    let old = std::env::var_os("YEKITRIM_DATA_DIR");
    unsafe { std::env::set_var("YEKITRIM_DATA_DIR", &dir) };

    // ① 文件都不在 = 正常（`ensure_global_config` 会补齐），不是错
    assert_eq!(super::global_config_parse_error(), None, "缺失不该报错");

    // ② 合法 TOML = 不报错
    std::fs::write(home.join("config.toml"), "[asr]\nmode = \"single\"\n").unwrap();
    assert_eq!(super::global_config_parse_error(), None, "合法配置不该报错");

    // ③ 坏 TOML = 报错，且**文案里必须能看出是哪个文件**
    std::fs::write(home.join("config.toml"), "这不是 TOML [[[\n").unwrap();
    let msg = super::global_config_parse_error().expect("坏配置必须报出来");
    assert!(msg.contains("config.toml"), "要指出是哪个文件: {msg}");
    assert!(msg.contains("程序配置"), "要说清是哪一类配置: {msg}");

    // ④ llm.toml 坏了同样要报（它含 api_key，打回默认值同样是静默失效）
    std::fs::write(home.join("config.toml"), "[asr]\nmode = \"single\"\n").unwrap();
    std::fs::write(home.join("llm.toml"), "也坏 [[[\n").unwrap();
    let msg = super::global_config_parse_error().expect("坏 llm.toml 必须报出来");
    assert!(msg.contains("llm.toml"), "要指出是哪个文件: {msg}");

    match old {
        Some(v) => unsafe { std::env::set_var("YEKITRIM_DATA_DIR", v) },
        None => unsafe { std::env::remove_var("YEKITRIM_DATA_DIR") },
    }
    // 收尾清干净：不清的话每跑一轮就往 temp 里留一个目录（实测攒到过 68 个）
    let _ = std::fs::remove_dir_all(&dir);
}


}

/// 「删掉的配置字段」相关的判据单独放一节：它们不是 patch/合并的测试，而是**兼容性**钉子。
#[cfg(test)]
mod removed_fields_tests {
    use super::*;

    /// 删掉 4 个废弃字段之后，**老 toml 里还留着它们的行**必须照样能读。
    ///
    /// 这 4 个（`align_model` / `max_segment_duration` / `segment_merge_gap` /
    /// `silence_noise_db`）从来没有读取方，删掉是为了别让后来人反复猜"这键到底有没有用"。
    /// ⚠️ 兼容性靠的是**三处结构体都没有** `deny_unknown_fields`（只有 polish 的 `polish.toml` 有，
    /// 那份确实不能多键）：顶层 `Config`、`AsrConfig`、`CuttingConfig`。**哪一处加上了，
    /// 老工程目录一升级就直接读不出配置** —— 所以这条判据同时钉三处：
    /// 顶层给一个野 section、`[asr]`/`[cutting]` 给已删的键。
    #[test]
    fn legacy_toml_with_removed_keys_still_loads() {
        let raw = r#"
# 顶级的未知键（老版本可能写过、后来整节被删）
[bogus_section]
whatever = 1

[asr]
align_model = "sensevoice"
max_segment_duration = 20.0
segment_merge_gap = 1.0
mode = "single"

[cutting]
silence_noise_db = -42
output_dir = "./out"
"#;
        let c: Config = toml::from_str(raw).expect("老 toml 里的已删键不能让整份配置解析失败");
        assert_eq!(c.asr.mode, "single", "其余键照常生效");
        assert_eq!(c.cutting.output_dir, "./out");
    }
}

#[cfg(test)]
mod scattered_config_tests {
    use super::*;

    /// 默认值就是文档里那张表 —— 改了字段默认值而没改模板/文档，这条会红。
    #[test]
    fn scattered_defaults_match_documentation() {
        let s = crate::config::types::ScatteredConfig::default();
        assert!(!s.enabled, "默认必须是关（不开就完全按老口径跑）");
        assert_eq!(s.min_secs, 8.0);
        assert_eq!(s.max_secs, 60.0);
        assert_eq!(s.min_subs, 2);
        assert_eq!(s.dm_floor, 0.15);
        assert_eq!(s.gap_sub_secs, 0.5);
        assert_eq!(s.gap_dm_floor, 0.3);
        assert_eq!(s.merge_gap_secs, 45.0);
        assert_eq!(s.merge_min_secs, 20.0);
        assert_eq!(s.merge_min_subs, 3);
        assert_eq!(s.max_merge_secs, 240.0);
        assert!(s.topic_key);
        assert!(s.system_prompt.is_empty() && s.prompt_template.is_empty());
    }

    /// 程序模板里必须**真有**这一段（否则设置页/手工改配置都找不到入口，等于没这个功能）。
    #[test]
    fn program_template_documents_scattered_block() {
        assert!(GLOBAL_CONFIG_TOML.contains("[detection.scattered]"));
        for key in [
            "min_secs",
            "max_secs",
            "min_subs",
            "dm_floor",
            "gap_sub_secs",
            "gap_dm_floor",
            "merge_gap_secs",
            "merge_min_secs",
            "merge_min_subs",
            "max_merge_secs",
            "topic_key",
        ] {
            assert!(
                GLOBAL_CONFIG_TOML.contains(key),
                "程序模板缺键 {key}（配置五步的第 4 步）"
            );
        }
        // 模板本身必须能解析（占位符替换后）
        let parsed: Config = toml::from_str(GLOBAL_CONFIG_TOML).expect("程序模板必须能解析");
        assert!(!parsed.detection.scattered.enabled);
        assert_eq!(parsed.detection.scattered.min_secs, 8.0);
    }

    /// 项目里写过的那几项要能覆盖程序默认；没写的保持默认（不能把程序配置抹平）。
    #[test]
    fn scattered_project_overlay_merges_field_by_field() {
        let overlay: Config = toml::from_str(
            r#"
[detection.scattered]
enabled = true
min_secs = 5.0
merge_gap_secs = 90.0
"#,
        )
        .expect("项目段必须能解析");
        let mut base = Config::default();
        assert!(!base.detection.scattered.enabled);
        merge(&mut base, overlay);
        assert!(base.detection.scattered.enabled, "enabled 取或");
        assert_eq!(base.detection.scattered.min_secs, 5.0, "写过的值覆盖");
        assert_eq!(base.detection.scattered.merge_gap_secs, 90.0);
        // 没写的保持默认
        assert_eq!(base.detection.scattered.max_secs, 60.0);
        assert_eq!(base.detection.scattered.merge_min_subs, 3);
        assert!(base.detection.scattered.topic_key);
    }

    /// 这段是**嵌套表**：序列化出来必须是 `[detection.scattered]` 一整个块（用户照着改配置），
    /// 且往返之后一个字段都不丢。
    #[test]
    fn scattered_round_trips_through_toml() {
        let mut cfg = Config::default();
        cfg.detection.scattered.enabled = true;
        cfg.detection.scattered.min_secs = 6.5;
        cfg.detection.scattered.merge_min_subs = 4;
        let text = toml::to_string(&cfg).expect("序列化");
        assert!(text.contains("[detection.scattered]"), "必须是独立的嵌套表");
        let back: Config = toml::from_str(&text).expect("反序列化");
        assert!(back.detection.scattered.enabled);
        assert_eq!(back.detection.scattered.min_secs, 6.5);
        assert_eq!(back.detection.scattered.merge_min_subs, 4);
        assert_eq!(back.detection.scattered.merge_gap_secs, 45.0, "没写过的保持默认");
    }
}

#[cfg(test)]
mod project_template_tests {
    use super::*;

    /// 临时工程目录（并行跑也不撞名：tag + pid + 纳秒）。
    fn tmp_project(tag: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let dir = std::env::temp_dir().join(format!(
            "yekitrim-projcfg-{tag}-{}-{nanos}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("建临时工程目录");
        dir
    }

    /// **回归钉子**：生成的 `yekitrim.toml` 必须在**根层**带 `config_version`。
    ///
    /// 现场：`[project]` 表头在前面，而版本键夹在 body 的最前面 ⇒ TOML 语义下它变成
    /// `project.config_version`，根层没有版本键。后果不是"读不出来"而是**更隐蔽的**：
    /// 工程级配置 `persists()=false`（只在内存里升），`version_of` 恒读到 0 ⇒ 每次读都被当 v0
    /// 重跑迁移，且「文件版本更高 ⇒ 一个字节都不动」那道保护对这类文件永不触发。
    #[test]
    fn generated_project_file_has_root_level_config_version() {
        let dir = tmp_project("version-root");
        let cfg = dir.join("yekitrim.toml");
        let input = dir.join("video.mp4");
        std::fs::write(&input, b"x").unwrap();
        write_project_config_inner_with(&cfg, &input, None, None, false, GLOBAL_CONFIG_TOML)
            .expect("生成工程配置");
        let text = std::fs::read_to_string(&cfg).unwrap();
        let doc: toml_edit::DocumentMut = text.parse().expect("生成的配置必须是合法 TOML");
        assert_eq!(
            version::version_of(&doc),
            CONFIG_VERSION,
            "根层必须是当前版本（落进 [project] 就相当于没有）:\n{text}"
        );
        assert!(
            !doc["project"]
                .as_table()
                .expect("[project] 必须是表")
                .contains_key(VERSION_KEY),
            "版本键不许留在 [project] 里:\n{text}"
        );
        // 版本行摆第一行（与 polish.toml 同口径）：一眼能看出这份文件是哪个结构版本。
        assert!(
            text.starts_with(VERSION_KEY),
            "第一行应是 config_version:\n{text}"
        );
        // 读路径也要看见它：新生成的工程配置不该再触发迁移。
        let prepared = version::prepare(&cfg, ConfigKind::Project).expect("读工程配置");
        assert!(!prepared.changed, "新生成的工程配置不该再触发迁移:\n{text}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// **回归钉子**：工程配置模板来自**账号树那份**程序配置（`global_config_path()`），
    /// 不是已退役的顶层 `<数据>/.yekitrim/config.toml`，也不是内建模板。
    ///
    /// 现场：设置页把 `[asr] mode` 改成 `single`（写账号树），新建工程却写进内建模板的
    /// `hybrid`；而 `merge()` 对字符串是「非空即覆盖」⇒ 用户的 single 被新工程顶回去。
    #[test]
    fn project_template_copies_the_account_tree_global_config() {
        let dir = tmp_project("global-src");
        let global = dir.join("users").join("1").join(".yekitrim").join("config.toml");
        std::fs::create_dir_all(global.parent().unwrap()).unwrap();
        std::fs::write(&global, "[asr]\nmode = \"single\"\n").unwrap();

        // ① 来源：读的就是这一份（存在就读它）
        let text = project_global_config_text_from(&global);
        assert!(
            text.contains("mode = \"single\""),
            "程序配置来源必须读账号树那份:\n{text}"
        );
        // ② 缺失时回退内建模板（新装机 / 刚迁走）
        let missing = project_global_config_text_from(&dir.join("nope.toml"));
        assert!(
            missing.contains("mode = \"hybrid\""),
            "程序配置缺失时应回退内建模板:\n{missing}"
        );

        // ③ 落盘：生成出来的工程配置里也是 single（用户全局设置没被顶掉）
        let cfg = dir.join("yekitrim.toml");
        let input = dir.join("video.mp4");
        std::fs::write(&input, b"x").unwrap();
        write_project_config_inner_with(&cfg, &input, None, None, false, &text).expect("生成工程配置");
        let written = std::fs::read_to_string(&cfg).unwrap();
        assert!(
            written.contains("mode = \"single\""),
            "新建工程必须继承用户全局的 ASR 模式:\n{written}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 新建工程配置**不许带程序级/凭证段**（安全回归，2026-10-30）。
    ///
    /// 根因：生成器把**整份全局 config.toml** 复制进 `yekitrim.toml`，只删了 `llm` / `correct`
    /// —— demo 工程实测带出 `[ingest] token_hash` / `[ingest.recorder].api_pass` /
    /// `[publish.bililive_tools] passkey_file` / `[auth]` / `[storage]`。工程目录会被共享、
    /// 搬到 NAS、发给别人（webhook 模式下它甚至直接建在录像目录里）。
    #[test]
    fn new_project_config_never_carries_program_level_sections() {
        // ① 内建模板（全局 config.toml 不存在时走它）
        let body = project_config_body(GLOBAL_CONFIG_TOML, Some("./output")).unwrap();
        assert_no_program_level_leak(&body);
        for key in ["[asr]", "[detection]", "[cutting]", "[sfx]"] {
            assert!(body.contains(key), "允许工程覆盖的段不能丢: {key}\n{body}");
        }

        // ② 模拟"已有全局 config.toml"（里面是真凭证）⇒ 同样剥干净，且活段照旧
        let global = r#"config_version = 4
[asr]
mode = "hybrid"
[detection]
danmaku_window_secs = 10
system_prompt = "程序配置里的 prompt"
[cutting]
output_dir = "./output"
[sfx]
blacklist = ["tag:x"]
[storage]
projects_root = "/srv/yekitrim-projects"
[ingest]
token_hash = "scrypt$abc$def"
webhook_enabled = true
[ingest.recorder]
root = "/rec"
api_pass = "brec-plain-pass"
[ingest.webhook]
root = "/rec/webhook"
[publish.bililive_tools]
base_url = "http://127.0.0.1:4321"
passkey_file = "/home/u/.blt-passkey"
[auth]
registration_enabled = true
"#;
        let body = project_config_body(global, Some("./out2")).unwrap();
        assert_no_program_level_leak(&body);
        for key in ["[asr]", "[detection]", "[cutting]", "[sfx]"] {
            assert!(body.contains(key), "允许工程覆盖的段不能丢: {key}\n{body}");
        }
        assert!(
            !body.contains("程序配置里的 prompt"),
            "[detection] 的 prompt 只住程序配置（改 prompt 要同步用户那份）:\n{body}"
        );
        assert!(body.contains("output_dir = \"./out2\""), "本工程的输出目录要写进去:\n{body}");
    }

    /// 判据：这些程序级/凭证痕迹一个都不许出现在生成的项目配置里。
    fn assert_no_program_level_leak(body: &str) {
        // 程序级段整段不在。按 TOML 解析判定，避免误伤注释里提到的 `[llm]` 这类字样
        // （`[asr]` 的注释就写着「需配置 [llm]」—— 那不是配置段）。
        let parsed: toml::Value = body.parse().expect("生成的工程配置必须是合法 TOML");
        for section in ["llm", "correct", "auth", "publish", "storage", "ingest"] {
            assert!(
                parsed.get(section).is_none(),
                "工程配置里不许出现程序级段 [{section}]:\n{body}"
            );
        }
        // 凭证/主机级字面量一个都不许留（含注释里漏抄的）
        for needle in ["token_hash", "api_pass", "passkey_file", "cookie_file", "projects_root"] {
            assert!(
                !body.contains(needle),
                "新建工程配置里不许出现程序级/凭证内容 {needle:?}:\n{body}"
            );
        }
    }
}

#[cfg(test)]
mod recorder_backend_merge_tests {
    use super::*;

    /// `[ingest.recorder]` 的**新字段**必须进 `merge()`：漏一条的后果是文件里写了、
    /// `load_global_config()` 却永远读成默认值（设置页保存成功、行为完全没变）。
    #[test]
    fn merge_copies_bililive_tools_fields() {
        let mut base = Config::default();
        let mut overlay = Config::default();
        overlay.ingest.recorder.kind = "bililive-tools".into();
        overlay.ingest.recorder.api_key = "key".into();
        overlay.ingest.recorder.container_root = "/app/video".into();
        overlay.ingest.bililive_tools.api_base = "http://127.0.0.1:18010".into();
        overlay.ingest.bililive_tools.api_key = "k".into();
        overlay.ingest.bililive_tools.root = "/v".into();
        overlay.ingest.bililive_tools.container_root = "/app/video".into();
        merge(&mut base, overlay);
        assert_eq!(base.ingest.recorder.kind, "bililive-tools");
        assert_eq!(base.ingest.recorder.api_key, "key");
        assert_eq!(base.ingest.recorder.container_root, "/app/video");
        assert_eq!(base.ingest.bililive_tools.api_base, "http://127.0.0.1:18010");
        assert_eq!(base.ingest.bililive_tools.api_key, "k");
        assert_eq!(base.ingest.bililive_tools.root, "/v");

        // 默认值（bililive / 空）不能把已有的值打回去
        let mut base2 = Config::default();
        base2.ingest.recorder.kind = "bililive-tools".into();
        merge(&mut base2, Config::default());
        assert_eq!(base2.ingest.recorder.kind, "bililive-tools");
    }
}

#[cfg(test)]
mod cutting_min_free_gb_merge_tests {
    use super::*;

    /// `[cutting] min_free_gb`（磁盘预检阈值）的合并口径：非默认才覆盖 ——
    /// 写 `0` 是"显式关闭"，必须能盖过全局的开启；缺席 = 不改。
    #[test]
    fn merge_follows_non_default_and_zero_overrides() {
        let mut base = Config::default();
        let mut overlay = Config::default();
        overlay.cutting.min_free_gb = 20;
        merge(&mut base, overlay);
        assert_eq!(base.cutting.min_free_gb, 20, "调高阈值要生效");

        let mut base = Config::default();
        base.cutting.min_free_gb = 20;
        let mut overlay = Config::default();
        overlay.cutting.min_free_gb = 0;
        merge(&mut base, overlay);
        assert_eq!(base.cutting.min_free_gb, 0, "显式 0 = 关闭预检，要能盖过开启");

        let mut base = Config::default();
        base.cutting.min_free_gb = 20;
        merge(&mut base, Config::default());
        assert_eq!(base.cutting.min_free_gb, 20, "缺席 = 不改（不能被默认值打回去）");
    }

    /// 模板里的默认值必须可解析、与代码默认一致；字节换算要饱和
    /// （runner/cutter 测试用 `u64::MAX` 当"必失败"阈值，不能回绕成小数）。
    #[test]
    fn template_default_and_byte_conversion() {
        let parsed: Config = toml::from_str(GLOBAL_CONFIG_TOML).unwrap();
        assert_eq!(parsed.cutting.min_free_gb, 2, "模板默认 = 代码默认");
        assert_eq!(
            CuttingConfig::default().min_free_bytes(),
            2 * 1024 * 1024 * 1024
        );
        let mut maxed = CuttingConfig::default();
        maxed.min_free_gb = u64::MAX;
        assert_eq!(maxed.min_free_bytes(), u64::MAX, "饱和乘法不许回绕");
    }
}

#[cfg(test)]
mod storage_retention_tests {
    use super::*;

    /// 保留期三键的**读入**：老配置文件里没有它们 ⇒ 必须落到 90/365/30 的默认天数。
    ///
    /// ⚠️ 这三个字段刻意用 `#[serde(default = "default_*")]` 而不是裸 `#[serde(default)]`：
    /// 裸的那个给 u32 落 **0 = 关闭清理** —— 于是"升级上来的老配置"会静默变成"永不清理"，
    /// 问题原样保留、日志里还一个错都不报。
    #[test]
    fn legacy_file_without_keys_gets_the_default_days() {
        let legacy: Config = toml::from_str("[storage]\nprojects_root = \"/data/projects\"\n").unwrap();
        assert_eq!(legacy.storage.audit_keep_days, 90);
        assert_eq!(legacy.storage.usage_keep_days, 365);
        assert_eq!(legacy.storage.log_keep_days, 30);
        // 模板里的值必须与代码默认一致（改默认只改一处，别只改模板）
        let tpl: Config = toml::from_str(GLOBAL_CONFIG_TOML).unwrap();
        assert_eq!(
            (tpl.storage.audit_keep_days, tpl.storage.usage_keep_days, tpl.storage.log_keep_days),
            (90, 365, 30)
        );
    }

    /// 三个保留期都按 `merge()` 的既有口径：**非默认才覆盖**。
    /// 显式写 `0`（关闭清理）是合法意图，要能盖过全局的 90/365/30。
    #[test]
    fn merge_follows_the_non_default_rule() {
        let mut base = Config::default();
        let mut overlay = Config::default();
        overlay.storage.audit_keep_days = 7;
        overlay.storage.log_keep_days = 3;
        merge(&mut base, overlay);
        assert_eq!((base.storage.audit_keep_days, base.storage.usage_keep_days, base.storage.log_keep_days), (7, 365, 3));
        assert_eq!(base.storage.usage_keep_days, 365, "没写的那项不许被动");

        let mut base = Config::default();
        base.storage.audit_keep_days = 7;
        let mut overlay = Config::default();
        overlay.storage.audit_keep_days = 0;
        merge(&mut base, overlay);
        assert_eq!(base.storage.audit_keep_days, 0, "显式 0 = 关闭清理，必须盖过 7");
    }
}

#[cfg(test)]
mod config_migration_tests {
    use super::*;
    use crate::owner::Owner;

    /// 临时数据目录（core 没有 tempfile 依赖，与 `patch_config_tests` 同一套做法）。
    fn tmp_home(tag: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let dir = std::env::temp_dir().join(format!(
            "yekitrim-cfg-migrate-{tag}-{}-{nanos}",
            std::process::id()
        ));
        std::fs::create_dir_all(dir.join(".yekitrim")).unwrap();
        dir
    }

    /// **核心判据**：平台段（接入/发布/容量/账号）以**顶层**为准，内容段（检测/ASR/剪切）以**账号**为准。
    ///
    /// 这条就是 2026-11-13 那次事故的正面描述：
    /// - 顶层 `webhook_enabled = true` + token 是真值 ⇒ 迁移后必须还在；
    /// - 账号那份的 6 条参考标题是真值 ⇒ 不能被顶层的 `[]` 打回空。
    #[test]
    fn platform_sections_follow_top_level_content_sections_follow_account() {
        let top = "config_version = 4\n[ingest]\nwebhook_enabled = true\n[ingest.recorder]\nkind = \"bililive-tools\"\nroot = \"/v\"\n[detection]\nreference_titles = []\n[asr]\nmode = \"hybrid\"\n";
        let account = "config_version = 4\n[detection]\nreference_titles = [\"标题A\", \"标题B\"]\n[asr]\nmodel = \"sensevoice\"\n";
        let merged = merge_top_level_into(top, Some(account)).unwrap();
        let v: toml::Value = toml::from_str(&merged).unwrap();
        // 平台段：顶层赢（账号那份根本没见过这些键）
        assert_eq!(v["ingest"]["webhook_enabled"].as_bool(), Some(true));
        assert_eq!(v["ingest"]["recorder"]["kind"].as_str(), Some("bililive-tools"));
        assert_eq!(v["ingest"]["recorder"]["root"].as_str(), Some("/v"));
        // 内容段：账号赢 —— 顶层的空数组**不许**把用户配的 6 条打回去
        let titles = v["detection"]["reference_titles"].as_array().unwrap();
        assert_eq!(titles.len(), 2, "账号那份的参考标题被顶层覆盖了");
        // 账号缺的键由顶层补齐
        assert_eq!(v["asr"]["mode"].as_str(), Some("hybrid"));
        assert_eq!(v["asr"]["model"].as_str(), Some("sensevoice"));
    }

    /// 目标不存在 ⇒ 整份搬（连注释一起），一个字节都不该丢。
    #[test]
    fn missing_target_is_a_verbatim_copy() {
        let src = "# 注释要保住\nconfig_version = 4\n[ingest]\nwebhook_enabled = true\n";
        assert_eq!(merge_top_level_into(src, None).unwrap(), src);
    }

    /// **LLM 那条更要命**：账号那份有真 key，顶层那份的 `api_key = ""` **不能**覆盖它。
    #[test]
    fn top_level_llm_never_clobbers_the_real_key() {
        let top = "[llm]\napi_key = \"\"\nmodel = \"\"\nmax_concurrent = 3\n";
        let account = "[llm]\napi_key = \"sk-real\"\nmodel = \"cn:deepseek\"\n";
        let merged = merge_top_level_into(top, Some(account)).unwrap();
        let v: toml::Value = toml::from_str(&merged).unwrap();
        assert_eq!(v["llm"]["api_key"].as_str(), Some("sk-real"));
        assert_eq!(v["llm"]["model"].as_str(), Some("cn:deepseek"));
        // 账号缺的键由顶层补
        assert_eq!(v["llm"]["max_concurrent"].as_integer(), Some(3));
    }

    /// 文件级：搬完顶层文件**改名留档**（不删），跑第二遍是 no-op。
    #[test]
    fn migration_archives_top_level_files_and_is_idempotent() {
        let home = tmp_home("archive");
        let uid = 7;
        let dest = crate::owner::owner_root_at(Owner::User(uid), &home);
        std::fs::create_dir_all(&dest).unwrap();
        // 迁移源（现在是 layout 迁移留下的 legacy-config）
        std::fs::create_dir_all(home.join("legacy-config")).unwrap();
        std::fs::write(
            home.join("legacy-config/config.toml"),
            "[ingest]\nwebhook_enabled = true\n[detection]\nreference_titles = []\n",
        )
        .unwrap();
        std::fs::write(home.join("legacy-config/llm.toml"), "[llm]\napi_key = \"\"\n").unwrap();
        // 账号那份（设置页写的那份）
        std::fs::write(
            dest.join("config.toml"),
            "[detection]\nreference_titles = [\"A\"]\n[ingest]\nwebhook_enabled = false\n",
        )
        .unwrap();
        std::fs::write(dest.join("llm.toml"), "[llm]\napi_key = \"sk-real\"\n").unwrap();

        let archived = migrate_top_level_config_to(Owner::User(uid), &home).unwrap();
        assert_eq!(archived.len(), 2, "config 与 llm 各留档一份");
        for bak in &archived {
            assert!(bak.exists(), "留档文件必须真的在: {}", bak.display());
        }
        // 迁移源不再有 config.toml / llm.toml（否则 `ensure_global_config` 会以为还没迁完）
        assert!(!home.join("legacy-config/config.toml").exists());
        assert!(!home.join("legacy-config/llm.toml").exists());
        // 合并结果：平台段顶层赢、内容段账号赢、key 没被抹
        let cfg: toml::Value = toml::from_str(&std::fs::read_to_string(dest.join("config.toml")).unwrap()).unwrap();
        assert_eq!(cfg["ingest"]["webhook_enabled"].as_bool(), Some(true));
        assert_eq!(cfg["detection"]["reference_titles"].as_array().unwrap().len(), 1);
        let llm: toml::Value = toml::from_str(&std::fs::read_to_string(dest.join("llm.toml")).unwrap()).unwrap();
        assert_eq!(llm["llm"]["api_key"].as_str(), Some("sk-real"));

        // 幂等：第二遍什么都不做
        let again = migrate_top_level_config_to(Owner::User(uid), &home).unwrap();
        assert!(again.is_empty(), "第二遍必须 no-op");
        let _ = std::fs::remove_dir_all(&home);
    }

    /// **管线读的就是 owner 那份**（这条是整轮收敛的目的）：`load_config_at` 的全局层与 `config.llm`
    /// 必须来自 `users/<uid>/.yekitrim/`，顶层那份（已退役）放同样的键做诱饵、值不同 ——
    /// 读到了诱饵就说明又退回旧位置了。
    #[test]
    fn load_config_at_reads_the_owner_config_and_llm() {
        // 改全局环境变量必须持**全文件唯一**的那把锁（自己再 static 一把等于没锁）
        let _guard = super::DATA_DIR_ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let home = tmp_home("load-at");
        let uid = crate::owner::local_admin_uid();
        let state = home.join("users").join(uid.to_string());
        std::fs::create_dir_all(&state).unwrap();
        std::fs::write(state.join("config.toml"), "[detection]\n").unwrap();
        // ⚠️ S4：提示词已经住 llm.toml 的 [llm.links.detect.prompt] —— 账号那份才是权威。
        // 这里用 v4 形状写（不再靠迁移，免得这条用例同时测两件事）。
        std::fs::write(
            state.join("llm.toml"),
            "config_version = 4\n[llm]\nactive = \"main\"\n             [[llm.providers]]\nid = \"main\"\napi_key = \"sk-owner\"\n             base_url = \"http://owner/v1\"\nmodel = \"m\"\n             [llm.links.detect.prompt]\nreference_titles = [\"账号标题\"]\n",
        )
        .unwrap();
        // 诱饵：已退役的顶层那份（值不同）
        std::fs::write(home.join(".yekitrim/config.toml"), "[detection]\n").unwrap();
        std::fs::write(
            home.join(".yekitrim/llm.toml"),
            "config_version = 4\n[llm]\nactive = \"main\"\n             [[llm.providers]]\nid = \"main\"\nbase_url = \"http://top/v1\"\n             [llm.links.detect.prompt]\nreference_titles = [\"顶层标题\"]\n",
        )
        .unwrap();
        let proj = home.join("proj");
        std::fs::create_dir_all(&proj).unwrap();
        let proj_file = proj.join("yekitrim.toml");
        std::fs::write(&proj_file, "input_file = \"a.mp4\"\n").unwrap();

        let old = std::env::var_os("YEKITRIM_DATA_DIR");
        unsafe { std::env::set_var("YEKITRIM_DATA_DIR", &home) };
        let cfg = load_config_at(&proj_file).unwrap();
        match old {
            Some(v) => unsafe { std::env::set_var("YEKITRIM_DATA_DIR", v) },
            None => unsafe { std::env::remove_var("YEKITRIM_DATA_DIR") },
        }

        assert_eq!(
            resolve_reference_titles(cfg.llm.prompt_for(crate::config::LlmLink::Detect)),
            vec!["账号标题".to_string()],
            "提示词只认账号 llm.toml（读顶层那份诱饵就说明又退回旧位置了）"
        );
        assert_eq!(
            cfg.llm.effective_api_key(),
            "sk-owner",
            "管线的 cfg.llm 必须来自账号 llm.toml，否则检测静默不调模型"
        );
        assert_eq!(cfg.llm.effective_base_url(), "http://owner/v1");
        let _ = std::fs::remove_dir_all(&home);
    }

    /// `Owner::Local` 没有独立目录可归 —— 直接 no-op（防呆：别让调用方把数据搬进 users/1）。
    #[test]
    fn local_owner_is_a_no_op() {
        let home = tmp_home("local");
        std::fs::create_dir_all(home.join("legacy-config")).unwrap();
        std::fs::write(home.join("legacy-config/config.toml"), "[asr]\nmode = \"single\"\n").unwrap();
        assert!(migrate_top_level_config_to(Owner::Local, &home).unwrap().is_empty());
        assert!(home.join("legacy-config/config.toml").exists(), "Local 不该动迁移源文件");
        let _ = std::fs::remove_dir_all(&home);
    }
}
