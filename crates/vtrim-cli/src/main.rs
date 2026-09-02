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

// (GLOBAL_CONFIG_TOML / LLM_CONFIG_TOML 模板已移入 vtrim_core::config)

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
/// LLM 订正转录文本：成功时写入 video_llm.srt 并更新 transcript_file / ctx.transcript。
/// 失败仅告警不中断管线，保留原始转录。
async fn llm_correct(
    cfg: &Config,
    ctx: &mut vtrim_core::types::PipelineContext,
    segments: &[vtrim_core::types::TranscriptSegment],
) -> anyhow::Result<()> {
    let danmaku = load_danmaku_for_correct(cfg);
    tracing::info!("LLM 订正转录文本...");

    // 断点续传：存在 correct.checkpoint.json 时询问是否继续
    let checkpoint = ctx.output_dir.join("correct.checkpoint.json");
    let partial_path = ctx.output_dir.join("correct.partial.json");
    let use_checkpoint = if checkpoint.exists() {
        confirm_resume("订正", &checkpoint)
    } else {
        true
    };
    if !use_checkpoint {
        remove_checkpoint(&checkpoint);
        let _ = std::fs::remove_file(&partial_path);
    }
    let checkpoint_path = if use_checkpoint && checkpoint.exists() {
        Some(checkpoint.as_path())
    } else {
        None
    };

    // 边分析边写扁平 partial：每个窗口完成后更新 correct.partial.json
    let (progress_tx, mut progress_rx) = tokio::sync::mpsc::channel::<
        vtrim_core::llm::agent::AgentProgress<Vec<vtrim_core::llm::agent::AgentOutput>>,
    >(64);
    let partial_path_writer = partial_path.clone();
    let partial_task = tokio::spawn(async move {
        use std::collections::BTreeMap;
        let mut partial: BTreeMap<usize, String> = BTreeMap::new();
        while let Some(progress) = progress_rx.recv().await {
            if let Some(outputs) = progress.output {
                for out in outputs {
                    partial.insert(out.index, out.text);
                }
                let _ = std::fs::write(
                    &partial_path_writer,
                    serde_json::to_string_pretty(&partial).unwrap_or_default(),
                );
            }
        }
    });

    let correct_result = vtrim_core::asr::correct::correct_transcript_with_checkpoint(
        segments,
        &danmaku,
        &cfg.llm,
        &cfg.correct,
        checkpoint_path,
        Some(progress_tx),
    )
    .await;
    let _ = partial_task.await;

    match correct_result {
        Ok(corrected) => {
            let new = vtrim_core::types::Transcript {
                segments: corrected,
                language: "zh".into(),
                asr_metadata: None,
            };
            let llm_srt = ctx.output_dir.join("video_llm.srt");
            vtrim_core::srt::write_srt(&new, &llm_srt)?;
            config::save_transcript_path(&llm_srt.to_string_lossy())?;
            ctx.transcript = Some(new);
            // 订正成功，清理断点
            let _ = std::fs::remove_file(&checkpoint);
            let _ = std::fs::remove_file(&partial_path);
            tracing::info!("LLM 订正完成");
        }
        Err(e) => tracing::warn!("LLM 订正失败，使用原始转录: {}", e),
    }
    Ok(())
}

/// 询问用户是否从 checkpoint 续传。
/// 返回 true=续传，false=重新开始。
fn confirm_resume(kind: &str, checkpoint: &std::path::Path) -> bool {
    use std::io::Write;

    println!("发现 {} 断点文件: {}", kind, checkpoint.display());
    print!("是否继续（跳过已完成窗口）？[y/N] ");
    let _ = std::io::stdout().flush();
    let mut line = String::new();
    let _ = std::io::stdin().read_line(&mut line);
    let ans = line.trim().to_lowercase();
    ans == "y" || ans == "yes"
}

/// 删除 checkpoint（用户选择重新开始时调用）。
fn remove_checkpoint(path: &std::path::Path) {
    match std::fs::remove_file(path) {
        Ok(_) => tracing::info!("已删除旧断点: {}", path.display()),
        Err(e) => tracing::warn!("删除断点失败: {}", e),
    }
}

fn load_danmaku_for_correct(cfg: &Config) -> Vec<vtrim_core::danmaku::DanmakuEntry> {
    // 1. 配置指定路径
    if let Some(p) = cfg.project.danmaku_file.as_ref() {
        let p = PathBuf::from(p);
        if !p.as_os_str().is_empty() && p.exists() {
            return vtrim_core::danmaku::load_danmaku(&p).unwrap_or_default();
        }
    }

    // 2. 自动发现：项目目录下的 danmu.ass / danmaku.ass
    let input = cfg.project.input_file.as_ref().map(PathBuf::from);
    if let Some(input_path) = input {
        if let Some(dir) = input_path.parent() {
            for name in ["danmu.ass", "danmaku.ass"] {
                let candidate = dir.join(name);
                if candidate.exists() {
                    tracing::info!("自动发现弹幕文件: {}", candidate.display());
                    return vtrim_core::danmaku::load_danmaku(&candidate).unwrap_or_default();
                }
            }
        }
    }

    Vec::new()
}

fn check_stage(stage_num: u32, current: u32, _cfg: &Config) -> anyhow::Result<()> {
    let needed: Vec<u32> = (current + 1..stage_num).collect();

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
            if input.is_dir() {
                anyhow::bail!(
                    "'{}' 是目录路径。\nvtrim init 需要视频文件路径，例如:\n  vtrim init {}\\clip.mp4",
                    input.display(),
                    input.display(),
                );
            }

            let valid_ext = ["mp4", "mkv", "mov", "avi", "flv", "webm", "mts", "m2ts"];
            let ext = input
                .extension()
                .and_then(|e| e.to_str())
                .map(|e| e.to_lowercase())
                .unwrap_or_default();
            if !valid_ext.contains(&ext.as_str()) {
                anyhow::bail!(
                    "'{}' 不是支持的视频格式。\n支持的扩展名: {}",
                    input.display(),
                    valid_ext.join(", "),
                );
            }

            if !input.exists() {
                anyhow::bail!("视频文件不存在: {}", input.display());
            }

            let out = cfg_path
                .clone()
                .or_else(|| {
                    input.parent().map(|p| p.join("vtrim.toml"))
                })
                .unwrap_or_else(|| PathBuf::from("vtrim.toml"));

            vtrim_core::config::write_project_config(
                &out,
                &input,
                danmaku.as_deref(),
                output.as_deref().map(|d| d.to_string_lossy().to_string()).as_deref(),
            )?;

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
            dry_run,
        } => {
            let cfg = load_cfg(global_config)?;
            let input_path = resolve_input(&cfg, input)?;
            let output = resolve_output(&cfg, output_dir);

            let current = cfg.project.current_stage(&output);
            let mut ctx = vtrim_core::types::PipelineContext::new(input_path, output.clone());

            // Step 1: 音频提取（未完成才跑）
            if current < STAGE_AUDIO {
                let mut extract_runner = vtrim_core::pipeline::PipelineRunner::new(cfg.clone());
                extract_runner.set_progress(progress.clone());
                extract_runner.add_stage(Box::new(vtrim_core::audio::AudioExtractor));
                extract_runner.run_until(&mut ctx, STAGE_AUDIO).await?;
                // 将音频路径写入 vtrim.toml，供后续阶段自动发现
                if let Some(ref ap) = ctx.audio_path {
                    let _ = config::save_audio_path(&ap.to_string_lossy());
                }
            } else {
                tracing::info!("音频已提取，跳过");
            }

            // Step 2: 转写（未完成才跑）
            if current < STAGE_TRANSCRIBE {
                vtrim_core::download::ensure_models(&cfg.asr).await?;
                let mut transcribe_runner = vtrim_core::pipeline::PipelineRunner::new(cfg.clone());
                transcribe_runner.set_progress(progress.clone());
                transcribe_runner.add_stage(Box::new(vtrim_core::asr::Transcriber));
                transcribe_runner.run_until(&mut ctx, STAGE_TRANSCRIBE).await?;
                // 将转写路径写入 vtrim.toml，供 detect/cut 自动发现
                config::save_transcript_path(
                    &ctx.output_dir.join("video.srt").to_string_lossy(),
                )?;
            } else {
                // 从已保存的文件恢复 transcript（若尚未订正过，下方统一补跑 LLM 订正）
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
                        // 配置尚未记录转写路径时补写（已指向 video_llm.srt 等订正文件时保留）
                        if cfg
                            .project
                            .transcript_file
                            .as_deref()
                            .map(|p| p.is_empty())
                            .unwrap_or(true)
                        {
                            let _ = config::save_transcript_path(&path.to_string_lossy());
                        }
                        tracing::info!("转写已存在: {}", path.display());
                    }
                    None => {
                        tracing::warn!("未找到转写文件，重新转写");
                        vtrim_core::download::ensure_models(&cfg.asr).await?;
                        let mut transcribe_runner = vtrim_core::pipeline::PipelineRunner::new(cfg.clone());
                        transcribe_runner.set_progress(progress.clone());
                        transcribe_runner.add_stage(Box::new(vtrim_core::asr::Transcriber));
                        transcribe_runner.run_until(&mut ctx, STAGE_TRANSCRIBE).await?;
                        config::save_transcript_path(
                            &ctx.output_dir.join("video.srt").to_string_lossy(),
                        )?;
                    }
                }
            }

            // LLM 订正：刚转写完成或恢复已有转写时统一执行。
            // 仅当 transcript_file 已指向存在的 video_llm.srt（此前已订正成功）才跳过，
            // 避免首次订正失败（transcript_file 仍指向原始文件）后永久丢失订正机会。
            let already_corrected = cfg.project.transcript_file
                .as_deref()
                .filter(|p| !p.is_empty())
                .map(PathBuf::from)
                .filter(|p| p.exists())
                .map(|p| p.file_name().map(|f| f == "video_llm.srt").unwrap_or(false))
                .unwrap_or(false);
            if current < STAGE_DETECT
                && cfg.asr.llm_correct.unwrap_or(false)
                && !cfg.llm.api_key.is_empty()
                && !already_corrected
            {
                if let Some(segments) = ctx.transcript.as_ref().map(|t| t.segments.clone()) {
                    llm_correct(&cfg, &mut ctx, &segments).await?;
                }
            }

            // Step 4: 检测 + 剪切（未完成才跑）
            if current < STAGE_DETECT {
                // 事件检测断点询问
                let events_checkpoint = output.join("events.checkpoint.json");
                if events_checkpoint.exists() {
                    ctx.detect_resume = Some(confirm_resume("事件检测", &events_checkpoint));
                    if ctx.detect_resume == Some(false) {
                        remove_checkpoint(&events_checkpoint);
                    }
                }

                let mut detect_runner = vtrim_core::pipeline::PipelineRunner::new(cfg.clone());
                detect_runner.set_progress(progress.clone());
                detect_runner.add_stage(Box::new(vtrim_core::detection::EventDetector));
                detect_runner.run_until(&mut ctx, STAGE_DETECT).await?;
                // 将事件文件路径写入 vtrim.toml，供 cut 自动发现
                let _ = config::save_events_path(
                    &ctx.output_dir.join("events.json").to_string_lossy(),
                );
                if *dry_run {
                    tracing::info!("(--dry-run) 分析完成，未执行剪切。输出目录: {}", output.display());
                    return Ok(());
                }
            } else {
                // 事件检测已存在：从 events.json 恢复事件，供后续剪切使用
                tracing::info!("事件检测已存在，跳过");
                let events_path = output.join("events.json");
                if events_path.exists() {
                    let events = vtrim_core::detection::load_events(&events_path)
                        .with_context(|| format!("无法读取事件文件: {}", events_path.display()))?;
                    ctx.events = events;
                    tracing::info!("已加载 {} 个事件", ctx.events.len());
                } else {
                    anyhow::bail!(
                        "事件检测已标记完成，但未找到 {}，请重新运行 vtrim detect",
                        events_path.display()
                    );
                }
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
            let out = resolve_output(&cfg, output);
            let current = cfg.project.current_stage(&out);
            check_stage(2, current, &cfg)?;
            let input_path = resolve_input(&cfg, input)?;
            let output = out;
            std::fs::create_dir_all(&output)?;

            // 确保 ASR 模型存在（双模型时打轴+识别模型都要）
            vtrim_core::download::ensure_models(&cfg.asr).await?;

            let mut ctx = vtrim_core::types::PipelineContext::new(input_path, output);
            let mut runner = vtrim_core::pipeline::PipelineRunner::new(cfg);
            runner.set_progress(progress.clone());
            runner.add_stage(Box::new(vtrim_core::asr::Transcriber));

            runner.run(&mut ctx).await?;
            // 保存转写文件路径到配置，供 detect 自动发现
            config::save_transcript_path(
                &ctx.output_dir
                    .join("video.srt")
                    .to_string_lossy(),
            )?;
            tracing::info!("语音转文字完成");
        }

        Command::Detect { transcript } => {
            let cfg = load_cfg(global_config)?;
            let output_dir = resolve_output(&cfg, &None);
            let current = cfg.project.current_stage(&output_dir);
            check_stage(3, current, &cfg)?;

            let input_path = resolve_input(&cfg, &None)?;
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

            // 事件检测断点询问
            let events_checkpoint = output_dir.join("events.checkpoint.json");
            if events_checkpoint.exists() {
                ctx.detect_resume = Some(confirm_resume("事件检测", &events_checkpoint));
                if ctx.detect_resume == Some(false) {
                    remove_checkpoint(&events_checkpoint);
                }
            }

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
            let output_dir = resolve_output(&cfg, output);
            let current = cfg.project.current_stage(&output_dir);
            check_stage(4, current, &cfg)?;
            let input_path = resolve_input(&cfg, input)?;

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
                        "未找到事件 JSON 文件, 请通过 --events 指定或先运行 detect"
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
            tracing::info!("剪切完成, 共 {} 个片段", ctx.events.len());
        }

        Command::Status => {
            let cfg = load_cfg(global_config)?;
            let output = resolve_output(&cfg, &None);
            let current = cfg.project.current_stage(&output);
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

            let transcript_json = output.join("transcript.json");
            if transcript_json.exists() {
                if let Ok(content) = std::fs::read_to_string(&transcript_json) {
                    if let Ok(value) = serde_json::from_str::<serde_json::Value>(&content) {
                        if let Some(meta) = value.get("asr_metadata") {
                            println!();
                            println!("ASR 元数据:");
                            if let Some(model) = meta.get("model").and_then(|m| m.as_str()) {
                                println!("  模型: {}", model);
                            }
                            if let Some(anchors) = meta.get("anchors").and_then(|a| a.as_u64()) {
                                println!("  锚点数: {}", anchors);
                            }
                            if let Some(cov) = meta
                                .get("anchor_coverage")
                                .and_then(|c| c.as_f64())
                            {
                                println!("  锚点覆盖率: {:.1}%", cov * 100.0);
                            }
                            if let Some(span) = meta
                                .get("max_interp_span")
                                .and_then(|s| s.as_u64())
                            {
                                println!("  最大插值跨度: {}", span);
                            }
                            if let Some(reason) = meta
                                .get("fallback_reason")
                                .and_then(|r| r.as_str())
                            {
                                println!("  降级原因: {}", reason);
                            }
                        }
                    }
                }
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

                let body = config::GLOBAL_CONFIG_TOML
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
                std::fs::write(&llm_out, config::LLM_CONFIG_TOML)?;
                tracing::info!("LLM 配置文件已生成: {}", llm_out.display());

                // 生成本地贴纸库目录（可执行文件旁 stickers/）
                let sticker_dir = vtrim_core::path::sticker_dir();
                std::fs::create_dir_all(&sticker_dir)?;
                tracing::info!("本地贴纸目录已准备: {}", sticker_dir.display());
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

        Command::Silence { files, duration, threshold, output } => {
            let cfg = load_cfg(global_config)?;
            let noise_db = threshold.unwrap_or(cfg.cutting.silence_noise_db);
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
                let segments = vtrim_core::cutting::remove_silence(input, &output_path, *duration, srt_opt, noise_db)?;

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

        Command::Download => {
            tracing::info!("下载 ASR 模型 (sensevoice / firered-zh / qwen3-asr)...");
            download::download_all(None).await?;
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

            let checkpoint = output_dir.join("correct.checkpoint.json");
            let partial_path = output_dir.join("correct.partial.json");
            let use_checkpoint = if checkpoint.exists() {
                confirm_resume("订正", &checkpoint)
            } else {
                true
            };
            if !use_checkpoint {
                remove_checkpoint(&checkpoint);
                let _ = std::fs::remove_file(&partial_path);
            }
            let checkpoint_path = if use_checkpoint && checkpoint.exists() {
                Some(checkpoint.as_path())
            } else {
                None
            };

            // 边分析边写扁平 partial
            let (progress_tx, mut progress_rx) = tokio::sync::mpsc::channel::<
                vtrim_core::llm::agent::AgentProgress<Vec<vtrim_core::llm::agent::AgentOutput>>,
            >(64);
            let partial_path_writer = partial_path.clone();
            let partial_task = tokio::spawn(async move {
                use std::collections::BTreeMap;
                let mut partial: BTreeMap<usize, String> = BTreeMap::new();
                while let Some(progress) = progress_rx.recv().await {
                    if let Some(outputs) = progress.output {
                        for out in outputs {
                            partial.insert(out.index, out.text);
                        }
                        let _ = std::fs::write(
                            &partial_path_writer,
                            serde_json::to_string_pretty(&partial).unwrap_or_default(),
                        );
                    }
                }
            });

            let correct_result = vtrim_core::asr::correct::correct_transcript_with_checkpoint(
                &transcript_data.segments,
                &danmaku,
                &cfg.llm,
                &cfg.correct,
                checkpoint_path,
                Some(progress_tx),
            )
            .await;
            let _ = partial_task.await;

            match correct_result {
                Ok(corrected) => {
                    transcript_data.segments = corrected;

                    let llm_srt = output_dir.join("video_llm.srt");
                    std::fs::create_dir_all(&output_dir)?;
                    vtrim_core::srt::write_srt(&transcript_data, &llm_srt)?;
                    config::save_transcript_path(&llm_srt.to_string_lossy())?;
                    let _ = std::fs::remove_file(&checkpoint);
                    let _ = std::fs::remove_file(&partial_path);

                    tracing::info!("LLM 订正完成");
                    tracing::info!("  字幕: {}", llm_srt.display());
                }
                Err(e) => anyhow::bail!("LLM 订正失败: {}", e),
            }
        }

        Command::Polish(action) => {
            use vtrim_polish::{commands, cover, process, render};

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
                cli::PolishAction::Render { input, renderer } => {
                    std::env::set_var("VTEDIT_RENDERER", renderer);
                    render::cmd_render(input).await?;
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
                cli::PolishAction::Stickers { action } => {
                    match action {
                        cli::StickerAction::List => commands::cmd_stickers_list()?,
                        cli::StickerAction::Test { keywords } => {
                            commands::cmd_stickers_test(&keywords)?
                        }
                        cli::StickerAction::Scan { llm } => {
                            commands::cmd_stickers_scan(*llm).await?
                        }
                        cli::StickerAction::Build { package } => {
                            commands::cmd_stickers_build(&package).await?
                        }
                    }
                }
            }
        }
    }

    Ok(())
}
