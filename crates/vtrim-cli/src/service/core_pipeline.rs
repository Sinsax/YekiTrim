//! Core pipeline 编排：CLI 与 WebUI 共用，避免两套流程逻辑漂移。
//!
//! CLI 原本把 extract/transcribe/detect/cut/process 编排写在 main.rs；
//! Server 又在 api.rs::core 复制了一份。这里收敛为同一个 Service 实现。

use std::path::{Path, PathBuf};
use std::sync::Arc;

use vtrim_core::config::Config;
use vtrim_core::pipeline::{PipelineRunner, Stage, STAGE_AUDIO, STAGE_CUT, STAGE_DETECT, STAGE_TRANSCRIBE};
use vtrim_core::progress::ProgressReporter;
use vtrim_core::types::PipelineContext;

use super::Workspace;

/// 将绝对/相对路径转为相对项目配置文件所在目录的路径（可移植，项目可移动）。
fn rel_to_config(config_file: &Path, path: &Path) -> String {
    if let Some(dir) = config_file.parent() {
        if let Ok(rel) = path.strip_prefix(dir) {
            return rel.to_string_lossy().into_owned();
        }
    }
    path.to_string_lossy().into_owned()
}


/// 运行单阶段（extract/transcribe/detect/cut）或全流程。
pub async fn run_stages(
    config_file: &Path,
    stages: &[String],
    dry_run: bool,
    resume: bool,
    progress: Option<Arc<dyn ProgressReporter>>,
) -> anyhow::Result<()> {
    run_stages_with(config_file, stages, dry_run, resume, None, None, progress).await
}

fn normalize_if_exists(config_file: &Path) -> anyhow::Result<()> {
    if config_file.exists() {
        vtrim_core::config::normalize_project_paths_in(config_file)?;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub async fn run_stages_with(
    config_file: &Path,
    stages: &[String],
    dry_run: bool,
    resume: bool,
    input_override: Option<PathBuf>,
    output_override: Option<PathBuf>,
    progress: Option<Arc<dyn ProgressReporter>>,
) -> anyhow::Result<()> {
    run_stages_with_ext(
        config_file, stages, dry_run, resume,
        input_override, output_override, None, None, progress,
    ).await
}

#[allow(clippy::too_many_arguments)]
pub async fn run_stages_with_ext(
    config_file: &Path,
    stages: &[String],
    dry_run: bool,
    resume: bool,
    input_override: Option<PathBuf>,
    output_override: Option<PathBuf>,
    transcript_override: Option<PathBuf>,
    events_override: Option<PathBuf>,
    progress: Option<Arc<dyn ProgressReporter>>,
) -> anyhow::Result<()> {
    normalize_if_exists(config_file)?;
    // 单阶段
    if stages.len() == 1 && stages[0] != "process" {
        return run_stage(config_file, &stages[0], resume, input_override, output_override, transcript_override, events_override, progress.as_ref()).await;
    }

    // 全流程
    run_process(config_file, dry_run, resume, input_override, output_override, progress.as_ref()).await
}

#[allow(clippy::too_many_arguments)]
async fn run_stage(
    config_file: &Path,
    stage: &str,
    resume: bool,
    input_override: Option<PathBuf>,
    output_override: Option<PathBuf>,
    transcript_override: Option<PathBuf>,
    events_override: Option<PathBuf>,
    progress: Option<&Arc<dyn ProgressReporter>>,
) -> anyhow::Result<()> {
    let cfg = vtrim_core::config::load_config_at(config_file)?;
    let ws = Workspace::load(Some(config_file))?;
    let output = output_override.unwrap_or_else(|| ws.output_dir());
    let input = match input_override {
        Some(p) => p,
        None => cfg.project.input_file.as_ref().filter(|s| !s.is_empty()).map(|s| ws.resolve(s))
            .ok_or_else(|| anyhow::anyhow!("未初始化工程 input_file"))?,
    };
    let mut ctx = PipelineContext::new(input.clone(), output.clone());
    let mut runner = PipelineRunner::new(cfg.clone());
    if let Some(p) = progress {
        runner.set_progress(p.clone());
    }

    match stage {
        "extract" => {
            runner.add_stage(Box::new(vtrim_core::audio::AudioExtractor));
            runner.run(&mut ctx).await?;
            if let Some(ap) = ctx.audio_path.as_ref() {
                vtrim_core::config::save_audio_path_in(config_file, &rel_to_config(config_file, ap))?;
            }
        }
        "transcribe" => {
            if let Some(a) = cfg.project.audio_file.as_ref().filter(|s| !s.is_empty()) {
                ctx.audio_path = Some(ws.resolve(a));
            } else if output.join("video.wav").exists() {
                ctx.audio_path = Some(output.join("video.wav"));
            }
            // WebUI 点“开始转写”时不需要手动先跑 extract：缺少音频则自动提取
            if ctx.audio_path.is_none() {
                let mut r = PipelineRunner::new(cfg.clone());
                r.add_stage(Box::new(vtrim_core::audio::AudioExtractor));
                r.run(&mut ctx).await?;
                if let Some(ap) = ctx.audio_path.as_ref() {
                    vtrim_core::config::save_audio_path_in(config_file, &rel_to_config(config_file, ap))?;
                }
            }
            vtrim_core::download::ensure_models(&cfg.asr).await?;
            runner.add_stage(Box::new(vtrim_core::asr::Transcriber));
            runner.run(&mut ctx).await?;
            vtrim_core::config::save_transcript_path_in(config_file, &rel_to_config(config_file, &output.join("video.srt")))?;
        }
        "detect" => {
            let tpath = transcript_override.unwrap_or_else(|| output.join("transcript.json"));
            if tpath.exists() {
                let raw = std::fs::read_to_string(&tpath)?;
                let ext = tpath.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
                ctx.transcript = Some(if ext == "json" { serde_json::from_str(&raw)? } else { vtrim_core::srt::parse_srt(&raw)? });
            } else {
                let srt = output.join("video.srt");
                if srt.exists() {
                    let raw = std::fs::read_to_string(&srt)?;
                    ctx.transcript = Some(vtrim_core::srt::parse_srt(&raw)?);
                }
            }
            if ctx.transcript.is_none() { anyhow::bail!("未找到转写文件"); }
            ctx.detect_resume = Some(resume);
            runner.add_stage(Box::new(vtrim_core::detection::EventDetector));
            runner.run(&mut ctx).await?;
            vtrim_core::config::save_events_path_in(config_file, &rel_to_config(config_file, &output.join("events.json")))?;
        }
        "cut" => {
            let ev_path = events_override.unwrap_or_else(|| output.join("events.json"));
            if !ev_path.exists() {
                anyhow::bail!("events.json 不存在");
            }
            ctx.events = vtrim_core::detection::load_events(&ev_path)?;
            runner.add_stage(Box::new(vtrim_core::cutting::Cutter));
            runner.run(&mut ctx).await?;
        }
        other => anyhow::bail!("未知 stage: {}", other),
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn run_process(
    config_file: &Path,
    dry_run: bool,
    resume: bool,
    input_override: Option<PathBuf>,
    output_override: Option<PathBuf>,
    progress: Option<&Arc<dyn ProgressReporter>>,
) -> anyhow::Result<()> {
    let cfg = vtrim_core::config::load_config_at(config_file)?;
    let ws = Workspace::load(Some(config_file))?;
    let output = output_override.unwrap_or_else(|| ws.output_dir());
    let current = cfg.project.current_stage(&output);
    let input = match input_override {
        Some(p) => p,
        None => cfg.project.input_file.as_ref().filter(|s| !s.is_empty()).map(|s| ws.resolve(s))
            .ok_or_else(|| anyhow::anyhow!("未初始化工程 input_file"))?,
    };
    let mut ctx = PipelineContext::new(input.clone(), output.clone());

    async fn run_until(
        cfg: Config,
        progress: Option<&Arc<dyn ProgressReporter>>,
        ctx: &mut PipelineContext,
        stage_num: u32,
        stage: Box<dyn Stage>,
    ) -> anyhow::Result<()> {
        let mut r = PipelineRunner::new(cfg);
        if let Some(p) = progress {
            r.set_progress(p.clone());
        }
        r.add_stage(stage);
        r.run_until(ctx, stage_num).await
    }

    if current < STAGE_AUDIO {
        run_until(cfg.clone(), progress, &mut ctx, STAGE_AUDIO, Box::new(vtrim_core::audio::AudioExtractor)).await?;
        if let Some(ap) = ctx.audio_path.as_ref() {
            vtrim_core::config::save_audio_path_in(config_file, &rel_to_config(config_file, ap))?;
        }
    } else {
        tracing::info!("音频已提取，跳过");
    }

    if current < STAGE_TRANSCRIBE {
        vtrim_core::download::ensure_models(&cfg.asr).await?;
        run_until(cfg.clone(), progress, &mut ctx, STAGE_TRANSCRIBE, Box::new(vtrim_core::asr::Transcriber)).await?;
        vtrim_core::config::save_transcript_path_in(config_file, &rel_to_config(config_file, &output.join("video.srt")))?;
    } else {
        let recovery = cfg.project.transcript_file.as_ref().filter(|p| !p.is_empty()).map(|p| ws.resolve(p)).filter(|p| p.exists())
            .or_else(|| { let j = output.join("transcript.json"); if j.exists() { Some(j) } else { None } })
            .or_else(|| { let s = output.join("video.srt"); if s.exists() { Some(s) } else { None } });
        match recovery {
            Some(path) => {
                let content = std::fs::read_to_string(&path)?;
                let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
                ctx.transcript = Some(if ext == "json" { serde_json::from_str(&content)? } else { vtrim_core::srt::parse_srt(&content)? });
                tracing::info!("转写已存在: {}", path.display());
            }
            None => {
                vtrim_core::download::ensure_models(&cfg.asr).await?;
                run_until(cfg.clone(), progress, &mut ctx, STAGE_TRANSCRIBE, Box::new(vtrim_core::asr::Transcriber)).await?;
                vtrim_core::config::save_transcript_path_in(config_file, &rel_to_config(config_file, &output.join("video.srt")))?;
            }
        }
    }


    // LLM 订正（可配置，与 CLI 语义保持一致；Web 自动继续断点）
    if current < STAGE_DETECT && cfg.asr.llm_correct.unwrap_or(false) && !cfg.llm.api_key.is_empty() {
        let already_corrected = cfg.project.transcript_file.as_ref()
            .filter(|p| !p.is_empty())
            .map(|p| ws.resolve(p))
            .filter(|p| p.exists())
            .map(|p| p.file_name().map(|f| f == "video_llm.srt").unwrap_or(false))
            .unwrap_or(false);
        if !already_corrected {
            if let Some(segments) = ctx.transcript.as_ref().map(|t| t.segments.clone()) {
                let danmaku = load_danmaku_for_correct(&cfg, &ws);
                let checkpoint = ctx.output_dir.join("correct.checkpoint.json");
                let partial_path = ctx.output_dir.join("correct.partial.json");
                if !resume && checkpoint.exists() {
                    let _ = std::fs::remove_file(&checkpoint);
                }
                let checkpoint_path = if checkpoint.exists() { Some(checkpoint.as_path()) } else { None };
                let (tx, mut rx) = tokio::sync::mpsc::channel::<vtrim_core::llm::agent::AgentProgress<Vec<vtrim_core::llm::agent::AgentOutput>>>(64);
                let partial_path_writer = partial_path.clone();
                let partial_task = tokio::spawn(async move {
                    use std::collections::BTreeMap;
                    let mut partial: BTreeMap<usize, String> = BTreeMap::new();
                    while let Some(progress) = rx.recv().await {
                        if let Some(outputs) = progress.output {
                            for out in outputs { partial.insert(out.index, out.text); }
                            let _ = std::fs::write(&partial_path_writer, serde_json::to_string_pretty(&partial).unwrap_or_default());
                        }
                    }
                });
                let result = vtrim_core::asr::correct::correct_transcript_with_checkpoint(
                    &segments, &danmaku, &cfg.llm, &cfg.correct, checkpoint_path, Some(tx),
                ).await;
                let _ = partial_task.await;
                if let Ok(corrected) = result {
                    let new = vtrim_core::types::Transcript { segments: corrected, language: "zh".into(), asr_metadata: None };
                    let llm_srt = ctx.output_dir.join("video_llm.srt");
                    vtrim_core::srt::write_srt(&new, &llm_srt)?;
                    vtrim_core::config::save_transcript_path_in(config_file, &rel_to_config(config_file, &llm_srt))?;
                    ctx.transcript = Some(new);
                    let _ = std::fs::remove_file(&checkpoint);
                    let _ = std::fs::remove_file(&partial_path);
                    tracing::info!("LLM 订正完成");
                } else {
                    tracing::warn!("LLM 订正失败，使用原始转录");
                }
            }
        }
    }

    // 事件检测
    if current < STAGE_DETECT {
        let checkpoint = output.join("events.checkpoint.json");
        if checkpoint.exists() && !resume {
            let _ = std::fs::remove_file(&checkpoint);
        }
        ctx.detect_resume = Some(checkpoint.exists() && resume);
        run_until(cfg.clone(), progress, &mut ctx, STAGE_DETECT, Box::new(vtrim_core::detection::EventDetector)).await?;
        vtrim_core::config::save_events_path_in(config_file, &rel_to_config(config_file, &output.join("events.json")))?;
        if dry_run {
            tracing::info!("(--dry-run) 分析完成，未执行剪切。输出目录: {}", output.display());
            return Ok(());
        }
    } else {
        tracing::info!("事件检测已存在，跳过");
        let events_path = output.join("events.json");
        if events_path.exists() {
            ctx.events = vtrim_core::detection::load_events(&events_path)?;
        }
    }

    if current < STAGE_CUT {
        if dry_run {
            tracing::info!("(--dry-run) 未执行剪切。输出目录: {}", output.display());
            return Ok(());
        }
        run_until(cfg, progress, &mut ctx, STAGE_CUT, Box::new(vtrim_core::cutting::Cutter)).await?;
    } else {
        tracing::info!("剪切已完成，跳过");
    }

    Ok(())
}


fn load_danmaku_for_correct(cfg: &vtrim_core::config::Config, ws: &Workspace) -> Vec<vtrim_core::danmaku::DanmakuEntry> {
    if let Some(p) = cfg.project.danmaku_file.as_ref().filter(|s| !s.is_empty()) {
        let p = ws.resolve(p);
        if p.exists() {
            return vtrim_core::danmaku::load_danmaku(&p).unwrap_or_default();
        }
    }
    let input = cfg.project.input_file.as_ref().filter(|s| !s.is_empty()).map(|s| ws.resolve(s));
    if let Some(input_path) = input {
        if let Some(dir) = input_path.parent() {
            for name in ["danmu.ass", "danmaku.ass"] {
                let candidate = dir.join(name);
                if candidate.exists() {
                    return vtrim_core::danmaku::load_danmaku(&candidate).unwrap_or_default();
                }
            }
        }
    }
    Vec::new()
}

