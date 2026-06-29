use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Context;
use clap::Parser;
use cli::{stage_name, Cli, Command, ConfigAction};
use tracing_subscriber::EnvFilter;
use vtrim_core::download;
use vtrim_core::config::{self, Config, STAGE_AUDIO, STAGE_CUT, STAGE_DETECT, STAGE_TRANSCRIBE};

use crate::progress::IndicatifProgress;

const PROJECT_CONFIG_TOML: &str = r##"# V-Trim 项目配置 — 优先级高于 .vtrim/config.toml
# 相同的参数以本文件为准

[project]
# 输入视频文件路径 (由 `vtrim init` 写入)
input_file = ""
# 弹幕文件路径 (可选，不影响流程)
danmaku_file = ""
# 转写文件路径 (由 `vtrim transcribe` 自动写入)
transcript_file = ""
# 音频文件路径 (由 `vtrim extract-audio` 自动写入)
audio_file = ""
# 事件文件路径 (由 `vtrim detect-events` 自动写入)
events_file = ""
# 当前处理进度 (0=未开始, 1~4=各阶段编号, 由程序自动管理)
current_stage = 0

[asr]
# ASR 后端 (local / kuaishou / bcut)
backend = "local"
# CPU 并行转写线程数 (0=CPU/4取整, 上限4)
num_threads = 0
# 每块长度(秒)
chunk_duration = 60.0
# 块间重叠(秒)
chunk_overlap = 10.0
# LLM 订正转录文本 (需配置 [llm] api_key)
llm_correct = false

[llm]
# LLM API Key
api_key = ""
# API 基础地址
api_base = ""
# 模型名称（推荐使用非推理模型）
model = ""
# 温度参数
temperature = 0.1
# 最大 Token 数
max_tokens = 16384

[detection]
# 弹幕密度统计窗口(秒)
danmaku_window_secs = 10
system_prompt = "你是一个直播录像分析助手。分析转录文本，将内容划分为多个有看点的视频片段。标题要吸引B站观众，摘要要详尽描述内容。"

prompt_template = """请分析以下直播转录文本，将其划分为多个事件片段。要求：

1. 每个事件片段时长控制在1-3分钟，短于60秒的片段需与相邻片段合并
2. 标题(label)口语化、有槽点/反转/意想不到的瞬间，包含1-2个emoji🤣😱🥵😡😅🤭🤑👻，适当使用反问句或感叹号，总长度不超过40字
3. 摘要(summary)要详尽，完整概括该片段的核心内容和看点，50-100字
4. 事件类型(event_type)从以下选择: intro(开场), gameplay(游戏/操作), interaction(互动), qna(问答), break(休息), outro(结尾), other(其他)

参考标题风格：
  这个反应也太真实了吧？？直接笑死🤣
  粉丝打SC问谁攻谁受，结果全员沉默？？
  你管这叫唱歌？这分明是作法现场????
  被自己翻唱尬到脚趾抠地?? 这也太真实了吧😅
  这什么雷霆坐姿雷霆视角🥵羞死人了🥵

输出格式：只输出一个 JSON 数组，不要 markdown 或解释：

转录片段：
{segments}

只输出JSON数组：
[{"start": 0.0, "end": 120.0, "label": "这操作给我看傻了??🤣", "summary": "详尽描述这段内容的核心看点...", "event_type": "interaction"}]"""

[cutting]
# 输出目录
output_dir = "./output"
# 是否覆盖已有文件
overwrite = false
"##;

const GLOBAL_CONFIG_TOML: &str = r#"# V-Trim 全局默认配置 — 所有工程共享
# 相同参数会被 vtrim.toml 覆盖

[asr]
# CPU 并行转写线程数 (0=CPU/4取整, 上限4)
num_threads = 0
# 每块长度(秒)
chunk_duration = 60.0
# 块间重叠(秒)
chunk_overlap = 10.0
# LLM 订正转录文本
llm_correct = false

[llm]
# LLM API Key
api_key = ""
# API 基础地址
api_base = ""
# 模型名称（推荐使用非推理模型）
model = ""
# 温度参数
temperature = 0.1
# 最大 Token 数
max_tokens = 16384

[detection]
system_prompt = "你是一个直播录像分析助手。分析转录文本，将内容划分为多个有看点的视频片段。标题要吸引B站观众，摘要要详尽描述内容。"

prompt_template = """请分析以下直播转录文本，将其划分为多个事件片段。要求：

1. 每个事件片段时长控制在1-3分钟，短于60秒的片段需与相邻片段合并
2. 标题(label)口语化、有槽点/反转/意想不到的瞬间，包含1-2个emoji🤣😱🥵😡😅🤭🤑👻，适当使用反问句或感叹号，总长度不超过40字
3. 摘要(summary)要详尽，完整概括该片段的核心内容和看点，50-100字
4. 事件类型(event_type)从以下选择: intro(开场), gameplay(游戏/操作), interaction(互动), qna(问答), break(休息), outro(结尾), other(其他)

参考标题风格：
  这个反应也太真实了吧？？直接笑死🤣
  粉丝打SC问谁攻谁受，结果全员沉默？？
  你管这叫唱歌？这分明是作法现场????
  被自己翻唱尬到脚趾抠地?? 这也太真实了吧😅
  这什么雷霆坐姿雷霆视角🥵羞死人了🥵

输出格式：只输出一个 JSON 数组，不要 markdown 或解释：

转录片段：
{segments}

只输出JSON数组：
[{"start": 0.0, "end": 120.0, "label": "这操作给我看傻了??🤣", "summary": "详尽描述这段内容的核心看点...", "event_type": "interaction"}]"""
"#;

mod cli;
mod progress;

fn setup_tracing() {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();
}

fn resolve_input(cfg: &Config, cli_input: &Option<PathBuf>) -> anyhow::Result<PathBuf> {
    if let Some(input) = cli_input {
        return Ok(input.clone());
    }
    if let Some(ref path) = cfg.project.input_file {
        if !path.is_empty() {
            return Ok(PathBuf::from(path));
        }
    }
    anyhow::bail!(
        "未指定输入文件。请提供 -i <视频路径>，或先运行 `vtrim init <video>` 初始化工程"
    );
}

fn load_cfg(
    cli_config: &Option<PathBuf>,
) -> anyhow::Result<Config> {
    if let Some(path) = cli_config {
        let content = std::fs::read_to_string(path)?;
        Ok(toml::from_str(&content)?)
    } else {
        config::load_config()
    }
}

fn check_stage(stage_num: u32, current: u32, _cfg: &Config) -> anyhow::Result<()> {
    let mut needed: Vec<u32> = Vec::new();
    for n in current + 1..stage_num {
        if !vtrim_core::pipeline::stage_enabled(n, _cfg) {
            continue;
        }
        needed.push(n);
    }

    if !needed.is_empty() {
        anyhow::bail!(
            "无法执行阶段 '{}' (#{})，当前进度为 #{}\n请先完成前面的阶段：{}",
            stage_name(stage_num),
            stage_num,
            current,
            needed
                .iter()
                .map(|&n| format!("#{} {}", n, stage_name(n)))
                .collect::<Vec<_>>()
                .join(" → ")
        );
    }
    Ok(())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    setup_tracing();

    let cli = Cli::parse();
    let global_config = &cli.config;

    let progress: Arc<dyn vtrim_core::progress::ProgressReporter> = Arc::new(IndicatifProgress::new());

    match &cli.command {
        Command::Init {
            input,
            danmaku,
            output,
            config: cfg_path,
        } => {
            let out = cfg_path
                .clone()
                .unwrap_or_else(|| PathBuf::from("vtrim.toml"));

            let mut doc: toml_edit::DocumentMut = PROJECT_CONFIG_TOML.parse()?;
            doc["project"]["input_file"] = toml_edit::value(input.to_string_lossy().as_ref());
            if let Some(df) = &danmaku {
                doc["project"]["danmaku_file"] = toml_edit::value(df.to_string_lossy().as_ref());
            } else {
                let auto_ass = input.with_extension("ass");
                if auto_ass.exists() {
                    doc["project"]["danmaku_file"] = toml_edit::value(auto_ass.to_string_lossy().as_ref());
                    tracing::info!("自动发现弹幕文件: {}", auto_ass.display());
                }
            }
            if let Some(dir) = output {
                doc["cutting"]["output_dir"] = toml_edit::value(dir.to_string_lossy().as_ref());
            }
            std::fs::write(&out, doc.to_string())?;

            tracing::info!("视频工程已初始化: {}", input.display());
            tracing::info!("配置文件: {}", out.display());
        }

        Command::Extract { input, output } => {
            let cfg = load_cfg(global_config)?;
            let input_path = resolve_input(&cfg, input)?;
            let output_dir = output.clone().unwrap_or_else(|| {
                std::env::current_dir()
                    .unwrap_or_default()
                    .join(&cfg.cutting.output_dir)
            });

            let mut ctx = vtrim_core::types::PipelineContext::new(input_path, output_dir);
            let mut runner = vtrim_core::pipeline::PipelineRunner::new(cfg);
            runner.set_progress(progress.clone());
            runner.add_stage(Box::new(vtrim_core::audio::AudioExtractor));

            runner.run(&mut ctx).await?;
            if let Some(ref ap) = ctx.audio_path {
                let _ = config::save_audio_path(&ap.to_string_lossy());
            }
            tracing::info!("音频提取完成");
        }

        Command::Process {
            input,
            output: output_dir,
            stages: _stages,
            dry_run,
        } => {
            let cfg = load_cfg(global_config)?;
            let input_path = resolve_input(&cfg, input)?;
            let output = output_dir.clone().unwrap_or_else(|| {
                std::env::current_dir()
                    .unwrap_or_default()
                    .join(&cfg.cutting.output_dir)
            });

            let current = config::load_current_stage()?;
            let mut ctx = vtrim_core::types::PipelineContext::new(input_path, output.clone());

            // Step 1: 音频提取（未完成才跑）
            if current < STAGE_AUDIO {
                let mut extract_runner = vtrim_core::pipeline::PipelineRunner::new(cfg.clone());
                extract_runner.set_progress(progress.clone());
                extract_runner.add_stage(Box::new(vtrim_core::audio::AudioExtractor));
                extract_runner.run_until(&mut ctx, STAGE_AUDIO).await?;
                config::save_current_stage(STAGE_AUDIO)?;
            } else {
                tracing::info!("音频已提取，跳过");
            }

            // Step 2: 转写（未完成才跑）
            if current < STAGE_TRANSCRIBE {
                vtrim_core::download::ensure_models().await?;
                let mut transcribe_runner = vtrim_core::pipeline::PipelineRunner::new(cfg.clone());
                transcribe_runner.set_progress(progress.clone());
                transcribe_runner.add_stage(Box::new(vtrim_core::asr::Transcriber));
                transcribe_runner.run_until(&mut ctx, STAGE_TRANSCRIBE).await?;
                config::save_current_stage(STAGE_TRANSCRIBE)?;
            } else {
                // 从已保存的文件恢复 transcript
                let recovery_path = cfg.project.transcript_file
                    .as_ref()
                    .filter(|p| !p.is_empty())
                    .map(PathBuf::from)
                    .filter(|p| p.exists())
                    .or_else(|| {
                        let json = ctx.output_dir.join("transcript.json");
                        if json.exists() { Some(json) } else { None }
                    })
                    .or_else(|| {
                        let srt = ctx.output_dir.join("video.srt");
                        if srt.exists() { Some(srt) } else { None }
                    });

                match recovery_path {
                    Some(path) => {
                        let content = std::fs::read_to_string(&path)?;
                        let ext = path.extension()
                            .map(|e| e.to_string_lossy().to_lowercase())
                            .unwrap_or_default();
                        ctx.transcript = Some(if ext == "json" {
                            serde_json::from_str(&content)?
                        } else {
                            vtrim_core::asr::parse_srt(&content)?
                        });
                        tracing::info!("转写已存在: {}", path.display());
                    }
                    None => {
                        tracing::warn!("current_stage >= 2 但未找到转写文件，重新转写");
                        vtrim_core::download::ensure_models().await?;
                        let mut transcribe_runner = vtrim_core::pipeline::PipelineRunner::new(cfg.clone());
                        transcribe_runner.set_progress(progress.clone());
                        transcribe_runner.add_stage(Box::new(vtrim_core::asr::Transcriber));
                        transcribe_runner.run_until(&mut ctx, STAGE_TRANSCRIBE).await?;
                        config::save_current_stage(STAGE_TRANSCRIBE)?;
                    }
                }
            }

            // Step 3: LLM 订正（可选，不受 stage 限制）
            if cfg.asr.llm_correct && !cfg.llm.api_key.is_empty() {
                if let Some(ref transcript) = ctx.transcript {
                    tracing::info!("LLM 订正转录文本...");
                    match vtrim_core::asr::correct::correct_transcript(&transcript.segments, &cfg.llm).await {
                        Ok(corrected) => {
                            let new = vtrim_core::types::Transcript {
                                segments: corrected,
                                language: "zh".into(),
                            };
                            let llm_srt = ctx.output_dir.join("video_llm.srt");
                            vtrim_core::asr::write_srt(&new, &llm_srt)?;
                            config::save_transcript_path(&llm_srt.to_string_lossy())?;
                            ctx.transcript = Some(new);
                            tracing::info!("LLM 订正完成");
                        }
                        Err(e) => tracing::warn!("LLM 订正失败，使用原始转录: {}", e),
                    }
                }
            }

            // Step 4: 检测 + 剪切（未完成才跑）
            if current < STAGE_DETECT {
                let mut detect_runner = vtrim_core::pipeline::PipelineRunner::new(cfg.clone());
                detect_runner.set_progress(progress.clone());
                detect_runner.add_stage(Box::new(vtrim_core::detection::EventDetector));
                if *dry_run {
                    detect_runner.run_until(&mut ctx, STAGE_DETECT).await?;
                    tracing::info!("(--dry-run) 分析完成，未执行剪切。输出目录: {}", output.display());
                    return Ok(());
                }
                detect_runner.run_until(&mut ctx, STAGE_DETECT).await?;
                config::save_current_stage(STAGE_DETECT)?;
            } else {
                tracing::info!("事件检测已存在，跳过");
            }

            // Step 5: 剪切（未完成才跑）
            if current < STAGE_CUT {
                if *dry_run {
                    tracing::info!("(--dry-run) 未执行剪切。输出目录: {}", output.display());
                    return Ok(());
                }
                // 从文件恢复 events
                let events_path = ctx.output_dir.join("events.json");
                if events_path.exists() {
                    let content = std::fs::read_to_string(&events_path)?;
                    ctx.events = serde_json::from_str(&content)?;
                }
                let mut cut_runner = vtrim_core::pipeline::PipelineRunner::new(cfg);
                cut_runner.set_progress(progress.clone());
                cut_runner.add_stage(Box::new(vtrim_core::cutting::Cutter));
                cut_runner.run_until(&mut ctx, STAGE_CUT).await?;
                config::save_current_stage(STAGE_CUT)?;
            } else {
                tracing::info!("剪切已完成，跳过");
            }

            tracing::info!("处理完成! 输出目录: {}", output.display());
        }

        Command::Transcribe {
            input,
            output,
            backend,
        } => {
            let mut cfg = load_cfg(global_config)?;

            if let Some(backend) = backend {
                cfg.asr.backend = backend.clone();

                // 同步写入 vtrim.toml
                let config_path = global_config
                    .clone()
                    .unwrap_or_else(|| PathBuf::from("vtrim.toml"));
                if config_path.exists() {
                    if let Ok(content) = std::fs::read_to_string(&config_path) {
                        if let Ok(mut doc) = content.parse::<toml_edit::DocumentMut>() {
                            doc["asr"]["backend"] = toml_edit::value(backend.clone());
                            let _ = std::fs::write(&config_path, doc.to_string());
                            tracing::info!("vtrim.toml: asr.backend = {}", backend);
                        }
                    }
                }
            }

            let current = config::load_current_stage()?;
            check_stage(2, current, &cfg)?;
            let input_path = resolve_input(&cfg, input)?;
            let output = output.clone().unwrap_or_else(|| {
                std::env::current_dir()
                    .unwrap_or_default()
                    .join(&cfg.cutting.output_dir)
            });

            let mut ctx = vtrim_core::types::PipelineContext::new(input_path, output);
            let mut runner = vtrim_core::pipeline::PipelineRunner::new(cfg);
            runner.set_progress(progress.clone());
            runner.add_stage(Box::new(vtrim_core::asr::Transcriber));

            runner.run(&mut ctx).await?;
            // 保存转写文件路径到配置，供 detect-events 自动发现
            config::save_transcript_path(
                &ctx.output_dir
                    .join("video.srt")
                    .to_string_lossy(),
            )?;
            tracing::info!("语音转文字完成");
        }

        Command::DetectEvents { transcript } => {
            let cfg = load_cfg(global_config)?;
            let current = config::load_current_stage()?;
            check_stage(3, current, &cfg)?;

            let input_path = resolve_input(&cfg, &None)?;
            let output_dir = std::env::current_dir()
                .unwrap_or_default()
                .join(&cfg.cutting.output_dir);

            // 优先级: --transcript 参数 > config.project.transcript_file > 自动发现
            let transcript_path = transcript
                .clone()
                .or_else(|| {
                    cfg.project.transcript_file
                        .as_ref()
                        .filter(|p| !p.is_empty())
                        .map(PathBuf::from)
                })
                .or_else(|| {
                    let json = output_dir.join("transcript.json");
                    let srt = output_dir.join("video.srt");
                    if json.exists() {
                        tracing::info!("自动发现转写 JSON: {}", json.display());
                        Some(json)
                    } else if srt.exists() {
                        tracing::info!("自动发现转写 SRT: {}", srt.display());
                        Some(srt)
                    } else {
                        None
                    }
                })
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "未找到转写文件，请通过 --transcript 指定或先运行 vtrim transcribe"
                    )
                })?;

            let content = std::fs::read_to_string(&transcript_path)
                .with_context(|| format!("无法读取转写文件: {}", transcript_path.display()))?;

            let ext = transcript_path
                .extension()
                .map(|e| e.to_string_lossy().to_lowercase())
                .unwrap_or_default();
            let transcript = if ext == "json" {
                serde_json::from_str(&content)
                    .with_context(|| format!("解析转写 JSON 失败: {}", transcript_path.display()))?
            } else if ext == "srt" {
                vtrim_core::asr::parse_srt(&content)
                    .with_context(|| format!("解析 SRT 失败: {}", transcript_path.display()))?
            } else {
                anyhow::bail!("不支持的转写文件格式: .{} (仅支持 .json / .srt)", ext);
            };

            let mut ctx = vtrim_core::types::PipelineContext::new(input_path, output_dir.clone());
            ctx.transcript = Some(transcript);

            let mut runner = vtrim_core::pipeline::PipelineRunner::new(cfg);
            runner.set_progress(progress.clone());
            runner.add_stage(Box::new(vtrim_core::detection::EventDetector));

            runner.run(&mut ctx).await?;
            let ev_path = output_dir.join("events.json");
            let _ = config::save_events_path(&ev_path.to_string_lossy());
            tracing::info!("事件检测完成, 共 {} 个事件", ctx.events.len());
        }

        Command::Cut {
            input,
            events,
            output,
        } => {
            let cfg = load_cfg(global_config)?;
            let current = config::load_current_stage()?;
            check_stage(4, current, &cfg)?;
            let input_path = resolve_input(&cfg, input)?;
            let output_dir = output.clone().unwrap_or_else(|| {
                std::env::current_dir()
                    .unwrap_or_default()
                    .join(&cfg.cutting.output_dir)
            });

            let events_path = events
                .clone()
                .or_else(|| {
                    cfg.project.events_file
                        .as_ref()
                        .filter(|p| !p.is_empty())
                        .map(PathBuf::from)
                        .filter(|p| p.exists())
                })
                .or_else(|| {
                    let auto = output_dir.join("events.json");
                    if auto.exists() {
                        Some(auto)
                    } else {
                        None
                    }
                })
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "未找到事件 JSON 文件, 请通过 --events 指定或先运行 detect-events"
                    )
                })?;

            let content = std::fs::read_to_string(&events_path)
                .with_context(|| format!("无法读取事件文件: {}", events_path.display()))?;
            let events: Vec<vtrim_core::types::Event> = serde_json::from_str(&content)
                .with_context(|| format!("解析事件 JSON 失败: {}", events_path.display()))?;

            let mut ctx = vtrim_core::types::PipelineContext::new(input_path, output_dir);
            ctx.events = events;

            let mut runner = vtrim_core::pipeline::PipelineRunner::new(cfg);
            runner.set_progress(progress.clone());
            runner.add_stage(Box::new(vtrim_core::cutting::Cutter));

            runner.run(&mut ctx).await?;
            tracing::info!("剪切完成, 共 {} 个片段", ctx.segments.len());
        }

        Command::Status => {
            let current = config::load_current_stage()?;
            let cfg = load_cfg(global_config)?;
            if let Some(ref input_file) = cfg.project.input_file {
                if !input_file.is_empty() {
                    println!("视频工程: {}", input_file);
                }
            }
            println!("当前进度: Stage #{}", current);
            println!();
            for num in 1..=STAGE_CUT {
                let status = if num < current {
                    "✓"
                } else if num == current {
                    "→"
                } else {
                    "·"
                };
                let name = stage_name(num);
                println!("  [{}] {:<20} {}", status, name, status);
            }
        }

        Command::Config(action) => match action {
            ConfigAction::Init { path } => {
                let out = path.clone().unwrap_or_else(|| {
                    vtrim_core::download::vtrim_home().join("config.toml")
                });
                if let Some(parent) = out.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                std::fs::write(&out, GLOBAL_CONFIG_TOML)?;
                tracing::info!("全局配置文件已生成: {}", out.display());
            }
            ConfigAction::Show => {
                let cfg = load_cfg(global_config)?;
                println!("{}", toml::to_string_pretty(&cfg)?);
            }
        },

        Command::SilenceRemove { files, duration, output } => {
            let clips_dir = std::env::current_dir()
                .unwrap_or_default()
                .join("output")
                .join("clips");

            let targets: Vec<PathBuf> = if files.is_empty() {
                if clips_dir.exists() {
                    std::fs::read_dir(&clips_dir)?
                        .filter_map(|e| e.ok())
                        .map(|e| e.path())
                        .filter(|p| {
                            p.extension()
                                .map(|ext| ext == "mp4")
                                .unwrap_or(false)
                        })
                        .collect()
                } else {
                    anyhow::bail!("未指定文件且 {} 不存在", clips_dir.display());
                }
            } else {
                files.clone()
            };

            if targets.is_empty() {
                anyhow::bail!("没有找到要处理的视频文件");
            }

            tracing::info!("静音移除: duration={}s, 共 {} 个文件", duration, targets.len());

            for (i, input) in targets.iter().enumerate() {
                let out_dir = output.clone().unwrap_or_else(|| {
                    input.parent()
                        .expect("无法获取输入文件所在目录")
                        .to_path_buf()
                });
                std::fs::create_dir_all(&out_dir)?;
                let stem = input
                    .file_stem()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_else(|| format!("clip_{}", i));
                let output_path = out_dir.join(format!("{}_cleaned.mp4", stem));
                tracing::info!("[{}/{}] {} → {}", i + 1, targets.len(), input.display(), output_path.display());
                let srt_path = input.with_extension("srt");
                let srt_opt = if srt_path.exists() { Some(srt_path.as_path()) } else { None };
                let segments = vtrim_core::cutting::remove_silence(input, &output_path, *duration, srt_opt)?;

                if srt_path.exists() {
                    let out_srt = output_path.with_extension("srt");
                    vtrim_core::cutting::adjust_srt(&srt_path, &out_srt, &segments)?;
                    tracing::info!("  字幕已调整: {}", out_srt.display());
                }
            }

            tracing::info!("静音移除完成!");
        }

        Command::DownloadModels { model: _ } => {
            tracing::info!("下载 SenseVoice 模型...");
            download::download_all(None).await?;
        }

        Command::Correct { transcript } => {
            let cfg = load_cfg(global_config)?;
            let output_dir = std::env::current_dir()
                .unwrap_or_default()
                .join(&cfg.cutting.output_dir);

            let transcript_path = transcript
                .clone()
                .or_else(|| {
                    cfg.project.transcript_file
                        .as_ref()
                        .filter(|p| !p.is_empty())
                        .map(PathBuf::from)
                })
                .or_else(|| {
                    let json = output_dir.join("transcript.json");
                    if json.exists() { Some(json) } else { None }
                })
                .ok_or_else(|| {
                    anyhow::anyhow!("未找到转录文件，请通过 --transcript 指定或先运行 vtrim transcribe")
                })?;

            let content = std::fs::read_to_string(&transcript_path)
                .with_context(|| format!("无法读取转录文件: {}", transcript_path.display()))?;
            let ext = transcript_path.extension()
                .map(|e| e.to_string_lossy().to_lowercase())
                .unwrap_or_default();
            let mut transcript_data: vtrim_core::types::Transcript = if ext == "json" {
                serde_json::from_str(&content)
                    .with_context(|| "解析转录 JSON 失败")?
            } else {
                vtrim_core::asr::parse_srt(&content)
                    .with_context(|| "解析 SRT 失败")?
            };

            tracing::info!("LLM 订正 {} 个片段...", transcript_data.segments.len());
            match vtrim_core::asr::correct::correct_transcript(&transcript_data.segments, &cfg.llm).await {
                Ok(corrected) => {
                    transcript_data.segments = corrected;

                    let llm_srt = output_dir.join("video_llm.srt");
                    std::fs::create_dir_all(&output_dir)?;
                    vtrim_core::asr::write_srt(&transcript_data, &llm_srt)?;
                    config::save_transcript_path(&llm_srt.to_string_lossy())?;

                    tracing::info!("LLM 订正完成");
                    tracing::info!("  字幕: {}", llm_srt.display());
                }
                Err(e) => anyhow::bail!("LLM 订正失败: {}", e),
            }
        }

        Command::Version => {
            println!("vtrim {}", env!("CARGO_PKG_VERSION"));
        }
    }

    Ok(())
}
