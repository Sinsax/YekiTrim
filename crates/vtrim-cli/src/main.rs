use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Context;
use clap::Parser;
use cli::{stage_name, Cli, Command, ConfigAction};
use tracing_subscriber::EnvFilter;
use vtrim_core::download;
use vtrim_core::config::{self, Config};
use vtrim_core::pipeline::{STAGE_AUDIO, STAGE_CUT, STAGE_DETECT, STAGE_TRANSCRIBE};


use crate::progress::IndicatifProgress;

const GLOBAL_CONFIG_TOML: &str = r#"# V-Trim 全局默认配置 — 所有工程共享
# 相同参数会被 vtrim.toml 覆盖

[asr]
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
"#;

const LLM_CONFIG_TOML: &str = r#"# V-Trim LLM 配置 — 独立于工程配置
# 基于 OpenAI 兼容接口

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

fn resolve_output(cfg: &Config, cli_output: &Option<PathBuf>) -> PathBuf {
    cli_output.clone().unwrap_or_else(|| {
        let dir = cfg.cutting.output_dir.trim_start_matches("./").trim_start_matches(".\\");
        std::env::current_dir()
            .unwrap_or_default()
            .join(dir)
    })
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

/// 加载弹幕文件供字幕订正使用（文件不存在则返回空 vec）
fn load_danmaku_for_correct(cfg: &Config) -> Vec<vtrim_core::types::DanmakuRef> {
    let path = cfg.project.danmaku_file.as_ref().map(PathBuf::from);
    match path {
        Some(p) if !p.as_os_str().is_empty() && p.exists() => {
            vtrim_core::danmaku::load_danmaku_refs(&p).unwrap_or_default()
        }
        _ => Vec::new(),
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

            // 优先从全局配置复制，不存在则使用内建默认
            let global_path = vtrim_core::path::vtrim_home().join("config.toml");
            let mut body = if global_path.exists() {
                tracing::info!("从全局配置复制: {}", global_path.display());
                let content = std::fs::read_to_string(&global_path)?
                    .replace("# V-Trim 全局默认配置 — 所有工程共享\n# 相同参数会被 vtrim.toml 覆盖", "");
                content
            } else {
                GLOBAL_CONFIG_TOML.to_string()
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

            if let Some(dir) = &output {
                doc["cutting"]["output_dir"] = toml_edit::value(dir.to_string_lossy().as_ref());
            }

            body = doc.to_string();

            // 构建 [project] 块（带注释），放在最前面
            let danmaku_val = if let Some(df) = &danmaku {
                df.to_string_lossy().to_string()
            } else {
                let auto_ass = input.with_extension("ass");
                if auto_ass.exists() {
                    tracing::info!("自动发现弹幕文件: {}", auto_ass.display());
                    auto_ass.to_string_lossy().to_string()
                } else {
                    String::new()
                }
            };

            let mut project = toml_edit::Table::new();
            project.set_implicit(true);
            project.insert("input_file", toml_edit::value(input.to_string_lossy().as_ref()));
            project.insert("danmaku_file", toml_edit::value(&danmaku_val));
            project.insert("transcript_file", toml_edit::value(""));
            project.insert("audio_file", toml_edit::value(""));
            project.insert("events_file", toml_edit::value(""));
            project.insert("current_stage", toml_edit::value(0));

            let mut header = toml_edit::DocumentMut::new();
            header.insert("project", toml_edit::Item::Table(project));
            let mut project_str = header.to_string();

            // 给 project 字段添加注释
            let comments = [
                ("input_file", "输入视频文件路径"),
                ("danmaku_file", "弹幕文件路径"),
                ("transcript_file", "转写文件路径 (由 transcribe 写入)"),
                ("audio_file", "音频文件路径 (由 extract-audio 写入)"),
                ("events_file", "事件文件路径 (由 detect-events 写入)"),
                ("current_stage", "当前处理进度 (0=未开始, 1~4=各阶段)"),
            ];
            for (key, comment) in &comments {
                let line = format!("{} = ", key);
                if project_str.contains(&line) {
                    let commented = format!("# {}\n{}", comment, line);
                    project_str = project_str.replace(&line, &commented);
                }
            }

            let full = format!("# V-Trim 项目配置 — 优先级高于 .vtrim/config.toml\n# 相同的参数以本文件为准\n\n{}", project_str.trim()) + "\n\n" + body.trim();
            std::fs::write(&out, full)?;

            tracing::info!("视频工程已初始化: {}", input.display());
            tracing::info!("配置文件: {}", out.display());
        }

        Command::Extract { input, output } => {
            let cfg = load_cfg(global_config)?;
            let input_path = resolve_input(&cfg, input)?;
            let output_dir = resolve_output(&cfg, output);

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
            let output = resolve_output(&cfg, output_dir);

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
                            vtrim_core::srt::parse_srt(&content)?
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

            // LLM 订正：仅当事件尚未检测时执行，避免重复订正已完成的检测结果
            if current < STAGE_DETECT && cfg.asr.llm_correct.unwrap_or(false) && !cfg.llm.api_key.is_empty() {
                if let Some(ref transcript) = ctx.transcript {
                    let danmaku = load_danmaku_for_correct(&cfg);
                    tracing::info!("LLM 订正转录文本...");
                    match vtrim_core::asr::correct::correct_transcript(&transcript.segments, &danmaku, &cfg.llm, &cfg.correct).await {
                        Ok(corrected) => {
                            let new = vtrim_core::types::Transcript {
                                segments: corrected,
                                language: "zh".into(),
                            };
                            let llm_srt = ctx.output_dir.join("video_llm.srt");
                            vtrim_core::srt::write_srt(&new, &llm_srt)?;
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
            ..
        } => {
            let cfg = load_cfg(global_config)?;
            let current = config::load_current_stage()?;
            check_stage(2, current, &cfg)?;
            let input_path = resolve_input(&cfg, input)?;
            let output = resolve_output(&cfg, output);

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
            let output_dir = resolve_output(&cfg, &None);

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
                vtrim_core::srt::parse_srt(&content)
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
            let output_dir = resolve_output(&cfg, output);

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

            let events = vtrim_core::detection::load_events(&events_path)
                .with_context(|| format!("无法读取或解析事件文件: {}", events_path.display()))?;

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
                    vtrim_core::path::vtrim_home().join("config.toml")
                });
                if let Some(parent) = out.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                use vtrim_core::{asr, detection};

                /// 将 Rust 字符串转为 TOML 值（多行用 """，单行用 "，转义内部引号）
                fn toml_val(s: &str) -> String {
                    // TOML 基本字符串中需要转义 \ 和 "
                    let escaped = s.replace('\\', "\\\\").replace('\"', "\\\"");
                    if s.contains('\n') {
                        format!("\"\"\"\n{}\n\"\"\"", escaped)
                    } else {
                        format!("\"{}\"", escaped)
                    }
                }

                let body = GLOBAL_CONFIG_TOML
                    // 注意：替换包含外围引号 "{{...}}"，避免模板的引号 + toml_val 的引号叠加
                    .replace(r#""{{CORRECT_SYSTEM_PROMPT}}""#, &toml_val(asr::correct::DEFAULT_SYSTEM_PROMPT))
                    .replace(r#""{{CORRECT_USER_PROMPT}}""#, &toml_val(asr::correct::DEFAULT_USER_PROMPT))
                    .replace(r#""{{DETECT_SYSTEM_PROMPT}}""#, &toml_val(detection::DEFAULT_SYSTEM_PROMPT))
                    .replace(r#""{{DETECT_USER_PROMPT}}""#, &toml_val(detection::DEFAULT_PROMPT));
                std::fs::write(&out, body)?;
                tracing::info!("全局配置文件已生成: {}", out.display());

                // 同时生成独立的 llm.toml
                let llm_out = out.parent().map(|p| p.join("llm.toml")).unwrap_or_else(|| {
                    PathBuf::from(".vtrim/llm.toml")
                });
                std::fs::write(&llm_out, LLM_CONFIG_TOML)?;
                tracing::info!("LLM 配置文件已生成: {}", llm_out.display());
            }
            ConfigAction::Show => {
                let cfg = load_cfg(global_config)?;
                println!("{}", toml::to_string_pretty(&cfg)?);
                println!();
                println!("# LLM 配置 (.vtrim/llm.toml)");
                let llm = config::load_llm_config();
                println!("{}", toml::to_string_pretty(&llm)?);
            }
        },

        Command::SilenceRemove { files, duration, output } => {
            let clips_dir = std::env::current_dir()
                .unwrap_or_default()
                .join("output")
                .join("clips");

            let targets: Vec<PathBuf> = if files.is_empty() {
                if clips_dir.exists() {
                    // Scan subdirectories for clip.mp4
                    let mut result: Vec<PathBuf> = std::fs::read_dir(&clips_dir)?
                        .filter_map(|e| e.ok())
                        .map(|e| e.path())
                        .filter(|p| p.is_dir())
                        .map(|d| d.join("clip.mp4"))
                        .filter(|p| p.exists())
                        .collect();
                    result.sort();
                    result
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
                    let parent = match input.parent() {
                        Some(p) => p.to_path_buf(),
                        None => std::env::current_dir().unwrap_or_default(),
                    };
                    parent
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

                // 同步处理弹幕字幕 clip.dm.srt（如果存在）
                let dm_srt_path = input.with_extension("dm.srt");
                if dm_srt_path.exists() {
                    let out_dm = output_path.with_extension("dm.srt");
                    vtrim_core::cutting::adjust_srt(&dm_srt_path, &out_dm, &segments)?;
                    tracing::info!("  弹幕字幕已调整: {}", out_dm.display());
                }
            }

            tracing::info!("静音移除完成!");
        }

        Command::DownloadModels { model: _ } => {
            tracing::info!("下载 SenseVoice 模型...");
            download::download_all(None).await?;
        }

        Command::LlmTest { prompt, output } => {
            let cfg = load_cfg(global_config)?;
            let prompt = prompt.clone().unwrap_or_else(|| "你好，请用一句话介绍你自己。".into());
            let out_path = output.clone().unwrap_or_else(|| PathBuf::from("llm_test_response.json"));

            if cfg.llm.api_key.is_empty() {
                let llm_path = vtrim_core::path::vtrim_home().join("llm.toml");
                let config_path = global_config.clone().unwrap_or(llm_path);
                anyhow::bail!("LLM API key 未配置，请先在 {} 中设置 base_url 和 api_key", config_path.display());
            }

            tracing::info!("调用 LLM (model={})...", if cfg.llm.model.is_empty() { "默认" } else { &cfg.llm.model });
            let response = vtrim_core::detection::test_llm(&cfg.llm, &prompt).await?;
            std::fs::write(&out_path, &response)?;
            tracing::info!("完整响应已保存: {}", out_path.display());
        }

        Command::Correct { transcript } => {
            let cfg = load_cfg(global_config)?;
            let output_dir = resolve_output(&cfg, &None);

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
                vtrim_core::srt::parse_srt(&content)
                    .with_context(|| "解析 SRT 失败")?
            };

            tracing::info!("LLM 订正 {} 个片段...", transcript_data.segments.len());
            let danmaku = load_danmaku_for_correct(&cfg);
            match vtrim_core::asr::correct::correct_transcript(&transcript_data.segments, &danmaku, &cfg.llm, &cfg.correct).await {
                Ok(corrected) => {
                    transcript_data.segments = corrected;

                    let llm_srt = output_dir.join("video_llm.srt");
                    std::fs::create_dir_all(&output_dir)?;
                    vtrim_core::srt::write_srt(&transcript_data, &llm_srt)?;
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

        Command::Polish(action) => {
            use vtrim_polish::{commands, cover, process, rendering};

            match action {
                cli::PolishAction::Init { input } => {
                    commands::cmd_init(input).await?;
                }
                cli::PolishAction::Llm { input } => {
                    commands::cmd_llm(input).await?;
                }
                cli::PolishAction::Process { input } => {
                    process::cmd_process(input).await?;
                }
                cli::PolishAction::Render { input } => {
                    rendering::cmd_render(input).await?;
                }
                cli::PolishAction::Cover { input, template } => {
                    if let Some(t) = template {
                        if !t.is_empty() {
                            cover::cmd_cover_template(input, t).await?;
                        }
                    }
                    cover::cmd_cover(input, None).await?;
                }
                cli::PolishAction::Preview { input, mode } => {
                    commands::cmd_preview(input, mode.as_deref().unwrap_or("")).await?;
                }
                cli::PolishAction::Doctor { input } => {
                    commands::cmd_doctor(input)?;
                }
                cli::PolishAction::Test { input } => {
                    commands::cmd_test(input).await?;
                }
            }
        }
    }

    Ok(())
}
