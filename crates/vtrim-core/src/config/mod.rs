mod types;
pub use types::*;

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::Deserialize;

/// 全局默认配置模板（`vtrim config init` 生成 .vtrim/config.toml 用，`vtrim init` 的基底）
/// 包含 {{CORRECT_SYSTEM_PROMPT}} 等占位符，由生成方替换为内建默认 prompt。
pub const GLOBAL_CONFIG_TOML: &str = r#"# V-Trim 全局默认配置 — 所有工程共享
# 相同参数会被 vtrim.toml 覆盖

[asr]
# 当前方案：单模型 sensevoice
model = "sensevoice"
segmentation_model = "sensevoice"
timestamp_alignment = false
# 并行转写块数 (0=auto, 手动值不超过 CPU 核数)
num_workers = 0
# 音频分块长度(秒)
chunk_duration = 60
# 块间重叠(秒，仅固定分块模式有效)
chunk_overlap = 10.0
# 段间静音阈值(秒)，超过此间隔才切为两条字幕
segment_gap = 1.0
# LLM 订正转录文本（需配置 [llm]）
llm_correct = false

[correct]
# 订正 system_prompt（修改此处覆盖内建默认）
system_prompt = "{{CORRECT_SYSTEM_PROMPT}}"
# 订正 user_prompt（{context} 和 {segments} 会被自动替换）
prompt = "{{CORRECT_USER_PROMPT}}"

[detection]
# 主播名称 (非空时自动追加到切片标题末尾)
vtuber_name = ""
# 弹幕密度统计窗口(秒)
danmaku_window_secs = 10
# 检测 system_prompt（修改此处覆盖内建默认）
system_prompt = "{{DETECT_SYSTEM_PROMPT}}"
# 检测 user prompt_template（{segments} 会被替换；留空 = 恢复内建默认值）
prompt_template = "{{DETECT_USER_PROMPT}}"

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
# 最大 Token 数 (检测事件需要较大的输出空间)
max_tokens = 65536
# 额外请求参数 (seed 确保稳定输出；关闭 thinking 避免浪费 token 在推理上)
extra_params = { seed = 42, thinking = { type = "disabled" } }
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
    // 优先从全局配置复制，不存在则使用内建默认
    let global_path = crate::path::vtrim_home().join("config.toml");
    let mut body = if global_path.exists() {
        tracing::info!("从全局配置复制: {}", global_path.display());
        std::fs::read_to_string(&global_path)?
            .replace("# V-Trim 全局默认配置 — 所有工程共享\n# 相同参数会被 vtrim.toml 覆盖", "")
    } else {
        GLOBAL_CONFIG_TOML
            .replace("# V-Trim 全局默认配置 — 所有工程共享\n# 相同参数会被 vtrim.toml 覆盖", "")
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
    let danmaku_val = match danmaku {
        Some(df) => df.to_string_lossy().to_string(),
        None => {
            let auto_ass = input.with_extension("ass");
            if auto_ass.exists() {
                tracing::info!("自动发现弹幕文件: {}", auto_ass.display());
                auto_ass.to_string_lossy().to_string()
            } else {
                String::new()
            }
        }
    };

    let mut project = toml_edit::Table::new();
    project.set_implicit(true);
    let input_for_config = if let Ok(rel) = input.strip_prefix(cfg_path.parent().unwrap_or(Path::new("."))) {
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
        base.project.input_file.clone_from(&overlay.project.input_file);
    }
    if overlay.project.danmaku_file.is_some() {
        base.project.danmaku_file.clone_from(&overlay.project.danmaku_file);
    }
    if overlay.project.transcript_file.is_some() {
        base.project.transcript_file.clone_from(&overlay.project.transcript_file);
    }
    if overlay.project.audio_file.is_some() {
        base.project.audio_file.clone_from(&overlay.project.audio_file);
    }
    if overlay.project.events_file.is_some() {
        base.project.events_file.clone_from(&overlay.project.events_file);
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
    if !overlay.asr.provider.is_empty() {
        base.asr.provider = overlay.asr.provider.clone();
    }
    if !overlay.asr.language.is_empty() {
        base.asr.language = overlay.asr.language.clone();
    }
    if !overlay.asr.model.is_empty() {
        base.asr.model = overlay.asr.model.clone();
    }
    if overlay.asr.timestamp_alignment != true {
        base.asr.timestamp_alignment = overlay.asr.timestamp_alignment;
    }
    if !overlay.asr.segmentation_model.is_empty() {
        base.asr.segmentation_model = overlay.asr.segmentation_model.clone();
    }
    if overlay.asr.max_segment_duration != 15.0 {
        base.asr.max_segment_duration = overlay.asr.max_segment_duration;
    }
    if overlay.asr.segment_merge_gap != 0.6 {
        base.asr.segment_merge_gap = overlay.asr.segment_merge_gap;
    }
    if overlay.asr.segment_padding != 0.3 {
        base.asr.segment_padding = overlay.asr.segment_padding;
    }
    if overlay.asr.max_new_tokens != 1024 {
        base.asr.max_new_tokens = overlay.asr.max_new_tokens;
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

    if !overlay.correct.system_prompt.is_empty() {
        base.correct.system_prompt = overlay.correct.system_prompt;
    }
    if !overlay.correct.prompt.is_empty() {
        base.correct.prompt = overlay.correct.prompt;
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
    toml::from_str::<LlmFile>(&content).map(|f| f.llm).unwrap_or_else(|e| {
        tracing::warn!("解析 {} 失败: {}，使用默认 LLM 配置", path.display(), e);
        LlmConfig::default()
    })
}

#[derive(Deserialize)]
struct LlmFile {
    llm: LlmConfig,
}

pub fn load_config() -> Result<Config> {
    let mut config = Config::default();

    // 优先级: 先加载全局配置，再用 vtrim.toml 覆盖（vtrim.toml 优先级更高）
    let global = crate::path::vtrim_home().join("config.toml");
    if let Some(overlay) = load_from_file(&global)? {
        merge(&mut config, overlay);
    }

    let local = PathBuf::from("vtrim.toml");
    if let Some(overlay) = load_from_file(&local)? {
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

/// 用 toml_edit 将 events_file 写入 vtrim.toml
pub fn save_events_path(path: &str) -> Result<()> {
    edit_toml(|doc| {
        doc["project"]["events_file"] = toml_edit::value(path);
    })
}

/// 用 toml_edit 将 audio_file 写入 vtrim.toml
pub fn save_audio_path(path: &str) -> Result<()> {
    edit_toml(|doc| {
        doc["project"]["audio_file"] = toml_edit::value(path);
    })
}

/// 用 toml_edit 将 transcript_file 写入 vtrim.toml
pub fn save_transcript_path(path: &str) -> Result<()> {
    edit_toml(|doc| {
        doc["project"]["transcript_file"] = toml_edit::value(path);
    })
}

fn edit_toml(f: impl FnOnce(&mut toml_edit::DocumentMut)) -> Result<()> {
    let path = PathBuf::from("vtrim.toml");
    let content = if path.exists() {
        std::fs::read_to_string(&path)?
    } else {
        String::new()
    };
    let mut doc: toml_edit::DocumentMut = content
        .parse()
        .unwrap_or_else(|_| toml_edit::DocumentMut::new());
    f(&mut doc);
    std::fs::write(&path, doc.to_string())?;
    Ok(())
}
