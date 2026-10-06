//! config：全站配置类型（`types.rs`）与两层合并 / 安全写入（本文件）。
//!
//! 入口：`load_config` / `load_config_at` / `load_global_config` / `load_llm_config`、`patch_config_file`（设置页唯一保存通道）、`write_project_config`、`save_*_path_in`、`resolve_reference_titles`。
//! 边界：改字段必须走四步（字段 + `merge()` 条件 + 手动 `Default` impl + 模板/注释）；`patch_config_file` 是递归深合并，别退回"一层 section" —— 见 `crates/yekitrim-core/src/config/AGENTS.md`

mod types;
pub mod version;
pub use types::*;
pub use version::{ConfigKind, CONFIG_VERSION, VERSION_KEY};

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::Deserialize;

use crate::path::PROJECT_CONFIG_FILE;

/// 解析参考标题：优先使用 `detection.reference_titles`；
/// 其次从 `prompt_template` 的“参考标题：”段落中兼容解析（旧配置）；
/// 都为空时回退到内建示例 `detection::DEFAULT_REFERENCE_TITLES`。
pub fn resolve_reference_titles(det: &DetectionConfig) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for raw in &det.reference_titles {
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
    let Some(pos) = det.prompt_template.find("参考标题：") else {
        return default_reference_titles();
    };
    let tail = &det.prompt_template[pos + "参考标题：".len()..];
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
config_version = 1

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
# 是否启用思考模式（LLM 先推理再输出）
thinking = false
# Agent 窗口大小（每次处理条数）
window_size = 25
# Agent 最大并发数
max_concurrent = 3
# 自定义热词（主播名/游戏/歌曲/梗，订正时优先使用）
hotwords = []

[detection]
# 主播名称 (非空时自动追加到切片标题末尾)
vtuber_name = ""
# 弹幕密度统计窗口(秒)
danmaku_window_secs = 10
# 标题+摘要 LLM 传播潜力评分权重（0 = 关闭该标准）
# 评分只喂 label+summary，按预测点击率/有趣程度/爆点强度给 0~100 分；
# 最终 popularity = (1-w)*弹幕/字幕热度 + w*LLM评分
llm_score_weight = 0.3
# 参考标题示例（检测和封面标题生成共用；每项一条，可含 emoji）
# 留空 = 使用内建示例标题（或旧版 prompt_template「参考标题：」段落）
reference_titles = [
]
# 检测 system_prompt（修改此处覆盖内建默认）
system_prompt = "{{DETECT_SYSTEM_PROMPT}}"
# 检测 user prompt_template（{segments} 会被替换；留空 = 恢复内建默认值）
prompt_template = "{{DETECT_USER_PROMPT}}"
# LLM 最大并发窗口数（默认 3；可根据 API 限流调大/调小）
max_concurrent = 3

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
# 是否覆盖已有文件
overwrite = false
# 静音检测噪声门限 (dB)，越低越敏感

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

[ingest.recorder]
# 录播姬输出目录（必须与 yekitrim server 同机或同可读盘）
root = ""
# 录播姬 HTTP API（用户端「配置直播间」靠它加房间）：
#   api_base 例 "http://127.0.0.1:2356"（桌面版/容器默认端口），**留空 = 未对接**
#   api_user/api_pass = 录播姬容器的 BREC_HTTP_BASIC_USER / BREC_HTTP_BASIC_PASS（没设就都留空）
api_base = ""
api_user = ""
api_pass = ""
# 兜底扫描间隔（分钟，0 = 关闭）：覆盖漏推送 / 录播姬重启 / 历史录播导入
watch_interval_mins = 10
# SessionEnded 之后再等多久没有新 FileClosed 就认为本场结束
finalize_idle_mins = 5
# 落位策略：true = 移入托管项目目录（同盘 rename，跨盘 copy+校验+删源）
move_into_managed = true
# 多分片会话合并为单个 source.mp4（ffmpeg -f concat -c copy）
concat_files = true
# 认定为录像的扩展名
extensions = ["flv", "mkv", "mp4", "ts"]
# 入库后自动执行到哪一步：none | transcribe | detect | cut
auto_pipeline = "detect"

[auth]
# 自助注册（邀请码）：开启后 /api/auth/register 才受理注册请求，
# 但**必须**有可用邀请码才能注册成功 —— 码本身才是凭证，这里只是总闸。
# 邀请码用 `yekitrim user invite` 生成（只显示一次明文，库里只存 selector + 哈希）
registration_enabled = false
"#;

/// LLM 配置模板（`yekitrim config init` 生成 .yekitrim/llm.toml 用）
pub const LLM_CONFIG_TOML: &str = r#"# YekiTrim LLM 配置 — 独立于工程配置
# 基于 OpenAI 兼容接口
# ⚠️ 配置结构版本，别手改（见 config/version.rs）
config_version = 1

[llm]
# API 基础地址
base_url = ""
# LLM API Key
api_key = ""
# 模型名称
model = ""
# 温度参数 (0=确定性输出，建议保持0)
temperature = 0
# 最大 Token 数：**没在下面 [llm.overrides.*] 里单独配的链路**用它。
# 这只是「跑飞的上限」——模型自己会停；检测窗口实测输出 1.9k~4.1k（思考占大头）。
max_tokens = 16384
# 并发请求数——事件检测 / 精修事件生成 / 字幕订正**共用这一个值**
# ([detection]/[correct] 各自的 max_concurrent 留空即跟随这里)。
# 实测中转站允许并发且接近线性，批处理时间基本按它反比缩短；触发 429 就调小。
max_concurrent = 3
# 额外请求参数 (seed 确保稳定输出；thinking enabled 启用思考模式，disabled 关闭)
# response_format=json_object 让模型不写解释性前言(需中转/服务端支持，不支持就删掉这行)
extra_params = { seed = 42, thinking = { type = "enabled" }, response_format = { type = "json_object" } }

# ── 按链路覆盖（缺一项 = 跟随上面 [llm] 的同名项）──
# 各链路要的输出长度天差地别，别用同一个上限互相迁就。想「恢复跟随全局」就把那一行删掉
# （设置页把输入框清空也会删掉它）。可选键：max_tokens / temperature / thinking。
[llm.overrides.detect]
# 事件检测窗口 + 标题摘要热度评分：实测输出 1.9k~4.1k token，8k 留 2× 余量
max_tokens = 8192
[llm.overrides.polish]
# 精修 LLM 剪辑（分段 + 逐段事件）：输出比检测长。
# thinking 不写 = 关闭思考（分段/逐段事件的既定行为）；要开就在这写 thinking = "enabled"
max_tokens = 32768
[llm.overrides.correct]
# 字幕订正：输出量随输入走
max_tokens = 8192
# [llm.overrides.utility]
# 零散单发（贴纸包/扫描标签、封面文案、连通测试）默认跟随 [llm]；要单独限就取消注释
# max_tokens = 8192
"#;

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

fn write_project_config_inner(
    cfg_path: &Path,
    input: &Path,
    danmaku: Option<&Path>,
    output_dir: Option<&str>,
    auto_discover_danmaku: bool,
) -> Result<()> {
    // 优先从全局配置复制，不存在则使用内建默认
    let global_path = crate::path::yekitrim_home().join("config.toml");
    let mut body = if global_path.exists() {
        tracing::info!("从全局配置复制: {}", global_path.display());
        std::fs::read_to_string(&global_path)?.replace(
            "# YekiTrim 全局默认配置 — 所有工程共享\n# 相同参数会被 yekitrim.toml 覆盖",
            "",
        )
    } else {
        GLOBAL_CONFIG_TOML.replace(
            "# YekiTrim 全局默认配置 — 所有工程共享\n# 相同参数会被 yekitrim.toml 覆盖",
            "",
        )
    };

    let mut doc: toml_edit::DocumentMut = body.parse()?;
    doc.remove("llm");
    doc.remove("correct");
    // 工程配置也带版本号（拷全局那份时通常已经带了；这里兜住"全局配置没构建过"的情形）
    doc.insert(VERSION_KEY, toml_edit::value(CONFIG_VERSION as i64));

    // 移除 [detection] 中的 prompt 字段，仅保留在 config.toml
    if let Some(detection) = doc.get_mut("detection") {
        if let Some(table) = detection.as_table_mut() {
            table.remove("system_prompt");
            table.remove("prompt_template");
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

    body = doc.to_string();

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

    if overlay.audio.sample_rate != 16000 {
        base.audio.sample_rate = overlay.audio.sample_rate;
    }
    if overlay.audio.channels != 1 {
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
    if overlay.asr.chunk_duration != 60.0 {
        base.asr.chunk_duration = overlay.asr.chunk_duration;
    }
    if overlay.asr.chunk_overlap != 10.0 {
        base.asr.chunk_overlap = overlay.asr.chunk_overlap;
    }
    if overlay.asr.segment_gap != 1.0 {
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
    if overlay.asr.segment_padding != 0.0 {
        base.asr.segment_padding = overlay.asr.segment_padding;
    }
    if overlay.asr.max_new_tokens != 256 {
        base.asr.max_new_tokens = overlay.asr.max_new_tokens;
    }
    if !overlay.asr.mode.is_empty() {
        base.asr.mode = overlay.asr.mode.clone();
    }
    if overlay.asr.breath_gap != 0.25 {
        base.asr.breath_gap = overlay.asr.breath_gap;
    }
    if overlay.asr.soft_cut_secs != 6.0 {
        base.asr.soft_cut_secs = overlay.asr.soft_cut_secs;
    }
    if overlay.asr.max_segment_chars != 30 {
        base.asr.max_segment_chars = overlay.asr.max_segment_chars;
    }
    if overlay.asr.snap_window != 0.3 {
        base.asr.snap_window = overlay.asr.snap_window;
    }
    // VAD：`Option<bool>` 与 `llm_correct` 同款 —— 只有**显式写了**才覆盖，
    // 这样"全局关、单个工程开"（或反过来）都表达得出来（bool 会掉进"≠默认才覆盖"的坑）。
    if let Some(v) = overlay.asr.vad {
        base.asr.vad = Some(v);
    }
    if overlay.asr.vad_threshold != 0.5 {
        base.asr.vad_threshold = overlay.asr.vad_threshold;
    }
    if overlay.asr.vad_rescue_db != 16.0 {
        base.asr.vad_rescue_db = overlay.asr.vad_rescue_db;
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
    if !overlay.detection.vtuber_name.is_empty() {
        base.detection.vtuber_name = overlay.detection.vtuber_name;
    }
    if !overlay.detection.reference_titles.is_empty() {
        base.detection.reference_titles = overlay.detection.reference_titles.clone();
    }
    if overlay.detection.llm_score_weight != 0.3 {
        base.detection.llm_score_weight = overlay.detection.llm_score_weight;
    }
    // Option 字段：Some 才覆盖（None = 跟随 llm.toml 的全局并发）
    if overlay.detection.max_concurrent.is_some() {
        base.detection.max_concurrent = overlay.detection.max_concurrent;
    }
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
    if overlay.correct.thinking {
        base.correct.thinking = true;
    }
    if overlay.correct.window_size != 25 {
        base.correct.window_size = overlay.correct.window_size;
    }
    if overlay.correct.max_concurrent.is_some() {
        base.correct.max_concurrent = overlay.correct.max_concurrent;
    }
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

    // 音效屏蔽列表: 追加合并 (全局 + 项目)
    for id in &overlay.sfx.blacklist {
        if !base.sfx.blacklist.contains(id) {
            base.sfx.blacklist.push(id.clone());
        }
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
    if !overlay.ingest.recorder.root.is_empty() {
        base.ingest.recorder.root = overlay.ingest.recorder.root.clone();
    }
    if overlay.ingest.recorder.watch_interval_mins != 10 {
        base.ingest.recorder.watch_interval_mins = overlay.ingest.recorder.watch_interval_mins;
    }
    if overlay.ingest.recorder.finalize_idle_mins != 5 {
        base.ingest.recorder.finalize_idle_mins = overlay.ingest.recorder.finalize_idle_mins;
    }
    if !overlay.ingest.recorder.move_into_managed {
        base.ingest.recorder.move_into_managed = false;
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
}

/// 从 .yekitrim/llm.toml 加载 LLM 配置（独立于工程配置和全局配置）
pub fn load_llm_config() -> LlmConfig {
    let path = crate::path::yekitrim_home().join("llm.toml");
    if !path.exists() {
        return LlmConfig::default();
    }
    // 读之前先跑版本迁移（老 llm.toml 缺 config_version ⇒ 补上；程序级会写回 + 备份）
    let content = match version::prepare(&path, ConfigKind::Llm) {
        Ok(p) => p.text,
        Err(e) => {
            tracing::warn!("读取 {} 失败: {}，使用默认 LLM 配置", path.display(), e);
            return LlmConfig::default();
        }
    };
    toml::from_str::<LlmFile>(&content)
        .map(|f| f.llm)
        .unwrap_or_else(|e| {
            tracing::warn!("解析 {} 失败: {}，使用默认 LLM 配置", path.display(), e);
            LlmConfig::default()
        })
}

#[derive(Deserialize)]
struct LlmFile {
    llm: LlmConfig,
}

pub fn load_config() -> Result<Config> {
    load_config_at(Path::new(PROJECT_CONFIG_FILE))
}

/// 从指定项目配置文件加载合并配置。
///
/// 与 `load_config()` 语义一致：
/// - 先加载全局 `.yekitrim/config.toml`
/// - 再用指定项目文件覆盖（项目文件不存在时按空项目处理，不报错）
/// - 最后独立加载 `.yekitrim/llm.toml`
pub fn load_config_at(project_file: &Path) -> Result<Config> {
    let mut config = Config::default();

    let global = crate::path::yekitrim_home().join("config.toml");
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

/// 校验 `.yekitrim/config.toml` / `.yekitrim/llm.toml` 能否解析，报出**第一个**出错的文件与原因。
///
/// 为什么需要它：`load_global_config` / `load_llm_config` 都把解析失败**吞掉**换成默认值
/// （它们不能因为一份坏配置就让整个进程起不来）。但界面必须知道"你看到的这些值是默认值，
/// 不是文件里的" —— 否则设置页**一旦保存就把内建默认值覆盖到真实配置文件上**。
///
/// 所以设置页的写入门闸（`SettingsView.vue::configError`）读的就是它，
/// 而这条错误**必须指向真实文件**（以前报的是 merged 配置的错误，指向不明）。
/// 两份都不存在 = 正常（`ensure_global_config` 会补齐），不算错。
pub fn global_config_parse_error() -> Option<String> {
    let home = crate::path::yekitrim_home();
    for (name, what) in [("config.toml", "程序配置"), ("llm.toml", "LLM 配置")] {
        let path = home.join(name);
        let Ok(raw) = std::fs::read_to_string(&path) else { continue };
        if let Err(e) = toml::from_str::<toml::Value>(&raw) {
            return Some(format!("{}（{}）解析失败，本次使用的是内建默认值: {e}", what, path.display()));
        }
    }
    None
}

/// 只读全局配置 (.yekitrim/config.toml), 不合并 yekitrim.toml。
/// 供不依赖项目 yekitrim.toml 的模块 (如 yekitrim-polish) 读取全局共享配置。
pub fn load_global_config() -> Config {
    let mut config = Config::default();
    let global = crate::path::yekitrim_home().join("config.toml");
    if let Ok(Some(overlay)) = load_from_file(&global, ConfigKind::Global) {
        merge(&mut config, overlay);
    }
    config
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
        let base = match version::migrate_text(kind, &content) {
            Ok(prepared) => {
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

    fn det(titles: Vec<String>, template: &str) -> DetectionConfig {
        DetectionConfig {
            reference_titles: titles,
            prompt_template: template.to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn config_titles_win_over_everything() {
        let d = det(vec!["自定义标题🤣".into()], "参考标题：\n▶ 旧示例");
        assert_eq!(
            resolve_reference_titles(&d),
            vec!["自定义标题🤣".to_string()]
        );
    }

    #[test]
    fn legacy_prompt_section_is_parsed() {
        let d = det(
            vec![],
            "前文\n参考标题：\n▶ 旧示例一🥵\n▶ 旧示例二😅\n\n输出：只输出 JSON",
        );
        assert_eq!(
            resolve_reference_titles(&d),
            vec!["旧示例一🥵".to_string(), "旧示例二😅".to_string()]
        );
    }

    #[test]
    fn placeholder_literal_is_skipped_and_builtin_used() {
        // init 写出的新版内建 prompt：该段落只剩待替换的占位符
        let d = det(vec![], crate::detection::DEFAULT_PROMPT);
        let resolved = resolve_reference_titles(&d);
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
        let d = det(vec![], "");
        assert_eq!(
            resolve_reference_titles(&d),
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
        let body = "\
# 顶部注释
[llm]
max_tokens = 16384

# 链路覆盖
[llm.overrides.detect]
max_tokens = 8192
thinking = \"disabled\"

[llm.overrides.polish]
max_tokens = 32768
";
        let p = write_tmp("null-del", body);
        patch_config_file(
            &p,
            &serde_json::json!({"llm": {"overrides": {"detect": {"max_tokens": null,
                                                                 "thinking": "enabled"}}}}),
        )
        .unwrap();
        let out = std::fs::read_to_string(&p).unwrap();

        assert!(!out.contains("8192"), "被 null 的键必须删掉：\n{out}");
        assert!(
            out.contains("thinking = \"enabled\""),
            "同一次 patch 里的其它键照常写入：\n{out}"
        );
        for keep in [
            "# 顶部注释",
            "# 链路覆盖",
            "max_tokens = 16384",
            "[llm.overrides.polish]",
            "max_tokens = 32768",
        ] {
            assert!(out.contains(keep), "丢了 {keep}：\n{out}");
        }
        // 删掉之后必须还能被解析（空串那版就是在这一步炸的），且该链路回到跟随 [llm]
        #[derive(serde::Deserialize)]
        struct File {
            llm: types::LlmConfig,
        }
        let text = std::fs::read_to_string(&p).unwrap();
        let f: File = toml::from_str(&text).expect("删键后的 llm.toml 仍须可解析");
        assert_eq!(
            f.llm.for_link(types::LlmLink::Detect).max_tokens,
            16384,
            "删掉覆盖 = 跟随 [llm].max_tokens"
        );
        assert_eq!(f.llm.for_link(types::LlmLink::Polish).max_tokens, 32768);
        let _ = std::fs::remove_dir_all(p.parent().unwrap());
    }

    /// 空子表 patch（`{}`）什么都不动 —— 别把整张表（含注释）当成"没提到的键"删掉。
    #[test]
    fn empty_subtable_patch_changes_nothing() {
        let body = "[llm]\nmax_tokens = 16384\n\n[llm.overrides.detect]\nmax_tokens = 8192\n";
        let p = write_tmp("empty-sub", body);
        patch_config_file(&p, &serde_json::json!({"llm": {"overrides": {"detect": {}}}})).unwrap();
        let out = std::fs::read_to_string(&p).unwrap();
        assert!(out.contains("max_tokens = 8192"), "{out}");
        assert!(out.contains("[llm.overrides.detect]"), "{out}");
        let _ = std::fs::remove_dir_all(p.parent().unwrap());
    }

    /// 模板必须是合法 TOML，且带着按链路的推荐值（新装的文件直接就是"检测 8k / 精修 32k"）。
    #[test]
    fn llm_config_template_parses_with_link_overrides() {
        #[derive(serde::Deserialize)]
        struct File {
            llm: types::LlmConfig,
        }
        let f: File = toml::from_str(LLM_CONFIG_TOML).expect("LLM_CONFIG_TOML 必须是合法 TOML");
        assert_eq!(
            f.llm.for_link(types::LlmLink::Detect).max_tokens,
            8192,
            "检测链路应带自己的上限"
        );
        assert_eq!(f.llm.for_link(types::LlmLink::Polish).max_tokens, 32768);
        assert_eq!(f.llm.for_link(types::LlmLink::Correct).max_tokens, 8192);
        // utility 没写覆盖 → 跟随 [llm]
        assert_eq!(
            f.llm.for_link(types::LlmLink::Utility).max_tokens,
            f.llm.max_tokens
        );
        // 精修/零散单发的默认关思考由链路默认给出
        assert_eq!(
            f.llm.for_link(types::LlmLink::Polish).thinking.as_deref(),
            Some("disabled")
        );
    }

/// `global_config_parse_error` 是设置页的**写入门闸**，必须指向真实文件、且只在真坏时报。
///
/// 它替代的是"读 merged 配置报错"那条路（2026-10-03 拆 `/api/config` 时发现：
/// `load_global_config` / `load_llm_config` 都把解析失败**吞掉**换成默认值，
/// 所以设置页原本拿不到原因 —— 那份配置被整体忽略、表单只剩内建默认值，用户不改也不知道；
/// 一旦点了保存，内建默认值就覆盖到真实文件上）。
#[test]
fn global_config_parse_error_points_at_the_real_file_and_needs_a_real_break() {
    // 这份 helper 读的是 `yekitrim_home()`，而它跟着 `YEKITRIM_DATA_DIR` 走 ——
    // 测试里指到一个临时目录，避免碰开发机上那份真实配置。
    //
    // ⚠️ **改全局环境变量必须持锁**：测试是并行跑的，`set_var` 是进程级的 ——
    // 不持锁就会踩到同期读配置的用例（`llm::tests` 那两条就是这么被偶发打红的）。
    // 同 `yekitrim-service/src/user.rs` 的专门警告，锁形态同 `service/src/db.rs`。
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
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
    let home = dir.join(".yekitrim");
    std::fs::create_dir_all(&home).unwrap();
    let old = std::env::var_os("YEKITRIM_DATA_DIR");
    std::env::set_var("YEKITRIM_DATA_DIR", &dir);

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
        Some(v) => std::env::set_var("YEKITRIM_DATA_DIR", v),
        None => std::env::remove_var("YEKITRIM_DATA_DIR"),
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
