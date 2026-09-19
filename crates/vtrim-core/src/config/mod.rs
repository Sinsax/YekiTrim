mod types;
pub use types::*;

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::Deserialize;

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

/// 全局默认配置模板（`vtrim config init` 生成 .vtrim/config.toml 用，`vtrim init` 的基底）
/// 包含 {{CORRECT_SYSTEM_PROMPT}} 等占位符，由生成方替换为内建默认 prompt。
pub const GLOBAL_CONFIG_TOML: &str = r#"# V-Trim 全局默认配置 — 所有工程共享
# 相同参数会被 vtrim.toml 覆盖

[asr]
# ASR 模式: "single"（SenseVoice 单模型）| "hybrid"（SenseVoice 打轴 + qwen3 填词）
# 默认 hybrid；准备页不给切换，要改就改这里或项目 vtrim.toml（设置页表单/源码）
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

[cutting]
# 输出目录
output_dir = "./output"
# 是否覆盖已有文件
overwrite = false
# 静音检测噪声门限 (dB)，越低越敏感
silence_noise_db = -30

[sfx]
# 音效屏蔽列表 (vtrim polish 使用, 全局共享)
# 精确 id 屏蔽单个音效; "tag:xxx" 屏蔽整个标签族 (如 "tag:surprise" 屏蔽所有恐怖系)
# 项目 polish.toml 的 [sfx] blacklist 可再追加
blacklist = []

[chrome]
# 自定义 Chrome/Chromium 可执行文件路径（留空自动检测）
path = ""

[storage]
# 服务端托管项目根目录（多用户；留空 = <exe_dir>/.vtrim/projects）
# 结构: <projects_root>/users/<uid>/<room>/<session>-<pid>/
projects_root = ""
# 全局磁盘上限（字节，0 = 不限）。只用于"入库/下载/渲染前预检"，
# 绝不自动删除用户数据（源录播默认保留）。
max_disk_bytes = 0
# 磁盘水位百分比：可用空间低于该比例时拒绝新的入库/下载/渲染
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
# token 用 `vtrim ingest token` 生成（只显示一次明文，这里只存哈希）
webhook_enabled = false
token_hash = ""
# 来源 IP 白名单（空 = 不限制；同机部署建议 ["127.0.0.1", "::1"]）
allow_ips = []

[ingest.recorder]
# 录播姬输出目录（必须与 vtrim server 同机或同可读盘）
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
# 邀请码用 `vtrim user invite` 生成（只显示一次明文，库里只存 selector + 哈希）
registration_enabled = false
"#;

/// LLM 配置模板（`vtrim config init` 生成 .vtrim/llm.toml 用）
pub const LLM_CONFIG_TOML: &str = r#"# V-Trim LLM 配置 — 独立于工程配置
# 基于 OpenAI 兼容接口

[llm]
# API 基础地址
base_url = ""
# LLM API Key
api_key = ""
# 模型名称
model = ""
# 温度参数 (0=确定性输出，建议保持0)
temperature = 0
# 最大 Token 数 (检测事件需要较大的输出空间；思考模式建议 16384+)
# 关了思考模式后这只是「跑飞的上限」——模型自己会停；一个检测窗口实测输出约 600 字符，
# 4096 已很宽裕。调小它可以在模型复读时更快收尾。
max_tokens = 16384
# 并发请求数——事件检测 / 精修事件生成 / 字幕订正**共用这一个值**
# ([detection]/[correct] 各自的 max_concurrent 留空即跟随这里)。
# 实测中转站允许并发且接近线性，批处理时间基本按它反比缩短；触发 429 就调小。
max_concurrent = 3
# 额外请求参数 (seed 确保稳定输出；thinking enabled 启用思考模式，disabled 关闭)
# response_format=json_object 让模型不写解释性前言(需中转/服务端支持，不支持就删掉这行)
extra_params = { seed = 42, thinking = { type = "enabled" }, response_format = { type = "json_object" } }
"#;

/// 生成 vtrim.toml 项目配置文件（`vtrim init`）。
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
/// ⚠️ 别把这条规则塞回 `write_project_config`：CLI 的 `vtrim init` 语义是"顺手带上同名 .ass"，
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
    let global_path = crate::path::vtrim_home().join("config.toml");
    let mut body = if global_path.exists() {
        tracing::info!("从全局配置复制: {}", global_path.display());
        std::fs::read_to_string(&global_path)?.replace(
            "# V-Trim 全局默认配置 — 所有工程共享\n# 相同参数会被 vtrim.toml 覆盖",
            "",
        )
    } else {
        GLOBAL_CONFIG_TOML.replace(
            "# V-Trim 全局默认配置 — 所有工程共享\n# 相同参数会被 vtrim.toml 覆盖",
            "",
        )
    };

    let mut doc: toml_edit::DocumentMut = body.parse()?;
    doc.remove("llm");
    doc.remove("correct");

    // 移除 [detection] 中的 prompt 字段，仅保留在 config.toml
    if let Some(detection) = doc.get_mut("detection") {
        if let Some(table) = detection.as_table_mut() {
            table.remove("system_prompt");
            table.remove("prompt_template");
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
        "# V-Trim 项目配置 — 优先级高于 .vtrim/config.toml\n# 相同的参数以本文件为准\n\n{}",
        project_str.trim()
    ) + "\n\n"
        + body.trim();
    std::fs::write(cfg_path, full)?;
    Ok(())
}

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
    if !overlay.asr.align_model.is_empty() {
        base.asr.align_model = overlay.asr.align_model.clone();
    }
    if overlay.asr.max_segment_duration != 15.0 {
        base.asr.max_segment_duration = overlay.asr.max_segment_duration;
    }
    if overlay.asr.segment_merge_gap != 0.6 {
        base.asr.segment_merge_gap = overlay.asr.segment_merge_gap;
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
    if overlay.cutting.silence_noise_db != -30.0 {
        base.cutting.silence_noise_db = overlay.cutting.silence_noise_db;
    }

    // 音效屏蔽列表: 追加合并 (全局 + 项目)
    for id in &overlay.sfx.blacklist {
        if !base.sfx.blacklist.contains(id) {
            base.sfx.blacklist.push(id.clone());
        }
    }

    if !overlay.chrome.path.is_empty() {
        base.chrome.path = overlay.chrome.path;
    }

    // [storage] 是服务端专用段（项目配置里不写），但仍按"非默认值覆盖"合并，
    // 以免有人在 vtrim.toml 里试写时行为诡异
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

    // [auth]：与 [ingest] 同一规矩 —— 只有显式写了 true 才覆盖（开关只可能被打开，不会被静默关掉）；
    // 真正生效的是全局配置，工程配置一般不该碰它。
    if overlay.auth.registration_enabled {
        base.auth.registration_enabled = true;
    }
}

/// 从 .vtrim/llm.toml 加载 LLM 配置（独立于工程配置和全局配置）
pub fn load_llm_config() -> LlmConfig {
    let path = crate::path::vtrim_home().join("llm.toml");
    if !path.exists() {
        return LlmConfig::default();
    }
    let content = match std::fs::read_to_string(&path) {
        Ok(c) => c,
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
    load_config_at(Path::new("vtrim.toml"))
}

/// 从指定项目配置文件加载合并配置。
///
/// 与 `load_config()` 语义一致：
/// - 先加载全局 `.vtrim/config.toml`
/// - 再用指定项目文件覆盖（项目文件不存在时按空项目处理，不报错）
/// - 最后独立加载 `.vtrim/llm.toml`
pub fn load_config_at(project_file: &Path) -> Result<Config> {
    let mut config = Config::default();

    let global = crate::path::vtrim_home().join("config.toml");
    if let Some(overlay) = load_from_file(&global)? {
        merge(&mut config, overlay);
    }

    let project = project_file.to_path_buf();
    if let Some(overlay) = load_from_file(&project)? {
        merge(&mut config, overlay);
    }

    // 独立加载 LLM 配置（llm.toml 已从 config.toml 分离）
    config.llm = load_llm_config();

    Ok(config)
}

/// 只读全局配置 (.vtrim/config.toml), 不合并 vtrim.toml。
/// 供不依赖项目 vtrim.toml 的模块 (如 vtrim-polish) 读取全局共享配置。
pub fn load_global_config() -> Config {
    let mut config = Config::default();
    let global = crate::path::vtrim_home().join("config.toml");
    if let Ok(Some(overlay)) = load_from_file(&global) {
        merge(&mut config, overlay);
    }
    config
}

/// 用 toml_edit 将 events_file 写入当前目录的 vtrim.toml
pub fn save_events_path(path: &str) -> Result<()> {
    save_events_path_in(&PathBuf::from("vtrim.toml"), path)
}

/// 用 toml_edit 将 audio_file 写入当前目录的 vtrim.toml
pub fn save_audio_path(path: &str) -> Result<()> {
    save_audio_path_in(&PathBuf::from("vtrim.toml"), path)
}

/// 用 toml_edit 将 transcript_file 写入当前目录的 vtrim.toml
pub fn save_transcript_path(path: &str) -> Result<()> {
    save_transcript_path_in(&PathBuf::from("vtrim.toml"), path)
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
        let mut doc: toml_edit::DocumentMut = match content.parse() {
            Ok(doc) => doc,
            Err(e) => {
                // 语法错误时**不能**静默从空文档开始：那样"改一个键"会把用户其余配置整段抹掉。
                // 先备份成 `*.toml.broken` 并告警，再基于空文档写入本次 patch。
                if !content.trim().is_empty() {
                    let backup = project_file.with_extension("toml.broken");
                    let _ = std::fs::copy(project_file, &backup);
                    tracing::warn!(
                        "配置 {} 不是合法 TOML（{e}），已备份为 {} 并只写入本次改动",
                        project_file.display(),
                        backup.display()
                    );
                }
                toml_edit::DocumentMut::new()
            }
        };
        f(&mut doc);
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
/// 项目目录整体移动后仍可继续使用；`vtrim init` / server 切换 / 保存阶段产物时调用。
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
        }
    })
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
        let dir = std::env::temp_dir().join(format!("vtrim-patch-{tag}-{}", std::process::id()));
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
}
