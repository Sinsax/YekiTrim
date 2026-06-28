use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Context;
use clap::Parser;
use cli::{stage_name, Cli, Command, ConfigAction};
use tracing_subscriber::EnvFilter;
use vtrim_core::download;
use vtrim_core::config::{self, Config, STAGE_CUT};

use crate::progress::IndicatifProgress;

const PROJECT_CONFIG_TOML: &str = r##"# V-Trim 项目配置文件

# 输入视频文件路径 (由 `vtrim init` 写入)
input_file = ""

# 弹幕文件路径 (可选，不影响流程)
danmaku_file = ""

# 转写文件路径 (由 `vtrim transcribe` 自动写入)
transcript_file = ""

# 音频文件路径 (由 `vtrim init` 自动写入)
audio_file = ""

# 事件文件路径 (由 `vtrim detect-events` 自动写入)
events_file = ""

# 当前处理进度 (0=未开始, 1~5=各阶段编号, 由程序自动管理)
current_stage = 0

[asr]
# ASR 后端 (local / kuaishou / bcut)
backend = "local"
# 静音分割阈值(秒): 超过此时长的静音将作为音频分段的切割边界
split_silence_secs = 10.0
# 最小片段长度(秒): 短于此长度的片段将与下一段合并
min_chunk_secs = 15.0

[asr.vad]
# Silero VAD 参数 (仅 VAD+ASR 管线使用)
threshold = 0.5              # 语音概率阈值 (越高越严格)
min_silence_duration = 0.5   # 句间最小静音(秒), 增大可减少分句
min_speech_duration = 0.25   # 最小语音段(秒)
max_speech_duration = 20.0   # 最大语音段(秒), 超过后强制分割

[detection]
# 弹幕密度统计窗口(秒)
danmaku_window_secs = 10
# 静音超过此时长(秒)即为事件边界
silence_threshold_secs = 3.0
# 最短片段(秒)
min_segment_secs = 30
# 最长片段(秒)
max_segment_secs = 900

[cutting]
# 是否保留源编码格式 (true=流复制, false=重新编码)
preserve_source = true
# 并行剪切进程数
max_parallel = 4
# 输出目录
output_dir = "./output"
# 是否覆盖已有文件
overwrite = false
"##;

const GLOBAL_CONFIG_TOML: &str = r#"# V-Trim 全局配置
# 设置后所有工程共享此配置

[asr]
# CPU 并行转写线程数 (0=CPU/4取整, 上限4)
num_threads = 0
# 每块长度(秒)
chunk_duration = 60.0
# 块间重叠(秒)
chunk_overlap = 10.0
# 人声分离: none / spleeter (需先 vtrim setup-spleeter)
separation = "none"

[detection.llm]
# LLM 服务提供商
provider = "openai"
# LLM API Key (也可用 VTRIM_LLM_KEY 环境变量)
api_key = ""
# API 基础地址 (留空=OpenAI, DeepSeek="https://api.deepseek.com/v1")
api_base = ""
# 模型名称
model = "gpt-4o-mini"
# 温度参数 (越低越确定)
temperature = 0.1
# 最大 Token 数 (响应较长时建议改到 16384)
max_tokens = 16384
# 每批处理的片段数
batch_size = 25
# 批次间重叠片段数 (用于合并连续性)
batch_overlap = 3
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
    if let Some(ref path) = cfg.input_file {
        if !path.is_empty() {
            return Ok(PathBuf::from(path));
        }
    }
    anyhow::bail!(
        "未指定输入文件。请提供 -i <视频路径>，或先运行 `vtrim init <video>` 初始化工程"
    );
}

fn build_pipeline_runner(cfg: Config, progress: Arc<dyn vtrim_core::progress::ProgressReporter>) -> vtrim_core::pipeline::PipelineRunner {
    let mut runner = vtrim_core::pipeline::PipelineRunner::new(cfg);
    runner.set_progress(progress);
    runner.add_stage(Box::new(vtrim_core::audio::AudioExtractor));
    runner.add_stage(Box::new(vtrim_core::asr::Transcriber));
    runner.add_stage(Box::new(vtrim_core::detection::EventDetector));
    runner.add_stage(Box::new(vtrim_core::cutting::Cutter));
    runner
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
        let skipped = n == 2 || !vtrim_core::pipeline::stage_enabled(n, _cfg);
        if !skipped {
            needed.push(n);
        }
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
            doc["input_file"] = toml_edit::value(input.to_string_lossy().as_ref());
            if let Some(df) = danmaku {
                doc["danmaku_file"] = toml_edit::value(df.to_string_lossy().as_ref());
            }
            if let Some(dir) = output {
                doc["cutting"]["output_dir"] = toml_edit::value(dir.to_string_lossy().as_ref());
            }
            std::fs::write(&out, doc.to_string())?;

            tracing::info!("视频工程已初始化: {}", input.display());
            tracing::info!("配置文件: {}", out.display());

            // 立即执行音频提取
            let output_dir = output.clone().unwrap_or_else(|| {
                std::env::current_dir()
                    .unwrap_or_default()
                    .join("./output")
            });
            let mut ctx =
                vtrim_core::types::PipelineContext::new(input.clone(), output_dir);
            let mut runner = vtrim_core::pipeline::PipelineRunner::new(config::load_config()?);
            runner.set_progress(progress.clone());
            runner.add_stage(Box::new(vtrim_core::audio::AudioExtractor));
            runner.run(&mut ctx).await?;
            // 记录音频文件路径
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

            let mut ctx = vtrim_core::types::PipelineContext::new(input_path, output.clone());
            let runner = build_pipeline_runner(cfg, progress.clone());

            if *dry_run {
                runner.run_until(&mut ctx, STAGE_CUT - 1).await?;
                tracing::info!("(--dry-run) 分析完成，未执行剪切。输出目录: {}", output.display());
            } else {
                runner.run_all(&mut ctx).await?;
                tracing::info!("处理完成! 输出目录: {}", output.display());
            }
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
            check_stage(3, current, &cfg)?;
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
                    .join(format!("{}.srt", ctx.input_stem))
                    .to_string_lossy(),
            )?;
            tracing::info!("语音转文字完成");
        }

        Command::DetectEvents { transcript } => {
            let cfg = load_cfg(global_config)?;
            let current = config::load_current_stage()?;
            check_stage(4, current, &cfg)?;

            let input_path = resolve_input(&cfg, &None)?;
            let output_dir = std::env::current_dir()
                .unwrap_or_default()
                .join(&cfg.cutting.output_dir);

            // 优先级: --transcript 参数 > config.transcript_file > 自动发现
            let transcript_path = transcript
                .clone()
                .or_else(|| {
                    cfg.transcript_file
                        .as_ref()
                        .filter(|p| !p.is_empty())
                        .map(PathBuf::from)
                })
                .or_else(|| {
                    let stem = input_path
                        .file_stem()
                        .map(|s| s.to_string_lossy().to_string())
                        .unwrap_or_else(|| "unknown".into());
                    let json = output_dir.join(format!("{}_transcript.json", stem));
                    let srt = output_dir.join(format!("{}.srt", stem));
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
            let ev_path = output_dir.join(format!("{}_events.json", ctx.input_stem));
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
            check_stage(5, current, &cfg)?;
            let input_path = resolve_input(&cfg, input)?;
            let output_dir = output.clone().unwrap_or_else(|| {
                std::env::current_dir()
                    .unwrap_or_default()
                    .join(&cfg.cutting.output_dir)
            });

            let events_path = events
                .clone()
                .or_else(|| {
                    cfg.events_file
                        .as_ref()
                        .filter(|p| !p.is_empty())
                        .map(PathBuf::from)
                        .filter(|p| p.exists())
                })
                .or_else(|| {
                    let stem = input_path
                        .file_stem()
                        .map(|s| s.to_string_lossy().to_string())
                        .unwrap_or_else(|| "unknown".into());
                    let auto = output_dir.join(format!("{}_events.json", stem));
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
            if let Some(ref input_file) = cfg.input_file {
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
                if num == 2 {
                    println!("  [{}] {:<20} {} (可选)", status, name, status);
                } else {
                    println!("  [{}] {:<20} {}", status, name, status);
                }
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
            let out_dir = output.clone().unwrap_or_else(|| {
                std::env::current_dir()
                    .unwrap_or_default()
                    .join("output")
                    .join("clips_cleaned")
            });
            std::fs::create_dir_all(&out_dir)?;

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
                let stem = input
                    .file_stem()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_else(|| format!("clip_{}", i));
                let output_path = out_dir.join(format!("{}_cleaned.mp4", stem));
                tracing::info!("[{}/{}] {} → {}", i + 1, targets.len(), input.display(), output_path.display());
                vtrim_core::cutting::remove_silence(input, &output_path, *duration)?;
            }

            tracing::info!("静音移除完成! 输出目录: {}", out_dir.display());
        }

        Command::DownloadModels { model: _ } => {
            tracing::info!("下载 SenseVoice 模型...");
            download::download_all(None).await?;
        }

        Command::SetupSpleeter => {
            tracing::info!("安装 Spleeter 人声分离...");
            let home = vtrim_core::download::vtrim_home();
            vtrim_core::download::setup_spleeter(&home).await?;
            tracing::info!("Spleeter 安装完成");
        }

        Command::Separate { input, output } => {
            let cfg = load_cfg(global_config)?;
            let input_path = match input {
                Some(p) => p.clone(),
                None => {
                    let audio = cfg.audio_file
                        .as_ref()
                        .filter(|p| !p.is_empty())
                        .map(PathBuf::from)
                        .or_else(|| {
                            let fallback = std::env::current_dir().unwrap_or_default()
                                .join("output").join("audio_16khz.wav");
                            if fallback.exists() { Some(fallback) } else { None }
                        })
                        .ok_or_else(|| anyhow::anyhow!(
                            "未指定输入文件，且未找到音频文件 (config.audio_file 或 output/audio_16khz.wav)"
                        ))?;
                    audio
                }
            };

            let home = vtrim_core::download::vtrim_home();
            let stem = input_path
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| "audio".into());
            let out_dir = output.clone().unwrap_or_else(|| {
                std::env::current_dir().unwrap_or_default().join("output").join("separated")
            });
            match vtrim_core::asr::separate::separate(&input_path, &home, &out_dir, &stem) {
                Ok(vocals) => {
                    let _ = config::save_vocal_path(&vocals.to_string_lossy());
                    tracing::info!("人声分离完成: {}", vocals.display());
                }
                Err(e) => anyhow::bail!("人声分离失败: {}", e),
            }
        }

        Command::Version => {
            println!("vtrim {}", env!("CARGO_PKG_VERSION"));
        }
    }

    Ok(())
}
