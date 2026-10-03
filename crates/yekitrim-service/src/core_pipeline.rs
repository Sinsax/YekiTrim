//! Core pipeline 编排：CLI 与 WebUI 共用，避免两套流程逻辑漂移。
//!
//! CLI 原本把 extract/transcribe/detect/cut/process 编排写在 main.rs；
//! Server 又在 api.rs::core 复制了一份。这里收敛为同一个 Service 实现。

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Context;
use yekitrim_core::config::Config;
use yekitrim_core::pipeline::{PipelineRunner, Stage, STAGE_AUDIO, STAGE_CUT, STAGE_DETECT, STAGE_TRANSCRIBE};
use yekitrim_core::progress::ProgressReporter;
use yekitrim_core::types::PipelineContext;

use super::Workspace;

/// 将绝对/相对路径转为相对项目配置文件所在目录的路径（可移植，项目可移动）。
///
/// ⚠️ 只在**没能绑进素材清单**的兜底路径上用（见 [`save_transcript_after_asr`]）：
/// 正常落账走 [`crate::assets::config_path`]，那才是与预处理页同一份口径。
fn rel_to_config(config_file: &Path, path: &Path) -> String {
    if let Some(dir) = config_file.parent() {
        if let Ok(rel) = path.strip_prefix(dir) {
            return rel.to_string_lossy().into_owned();
        }
    }
    path.to_string_lossy().into_owned()
}

/// ASR 跑完之后把字幕落账：**绑定分模式产物**并写 `transcript_file`。
///
/// 用户的口径是「输出一次绑定一次、只能两次」：每个 ASR 模式（hybrid / single）各占
/// 素材清单里的一个槽，槽里的条目带 `asr` 标记、不可删，用掉的模式在预处理页置灰。
/// 绑定本身由 [`crate::assets::bind_asr_artifact`] 完成（它连 `transcript_file` 一起写，
/// 同一个 patch 里落盘 —— 路径锁非重入，不能分开写）。
///
/// **绑不上就退回 `output/video.srt`**：产物没落盘（`[asr] mode` 被手改成奇怪的值）、
/// 或写配置失败时，少一条清单条目可以接受，`transcript_file` 指到一个不存在的文件不行
/// —— 那会让检测直接报"未找到转写文件"。
fn save_transcript_after_asr(
    ws: &Workspace,
    config_file: &Path,
    output: &Path,
    cfg: &Config,
) -> anyhow::Result<()> {
    if crate::assets::bind_asr_artifact(ws, output, cfg.asr.mode.trim()).is_some() {
        return Ok(());
    }
    yekitrim_core::config::save_transcript_path_in(
        config_file,
        &rel_to_config(config_file, &output.join("video.srt")),
    )
}

/// LLM 订正产物（`output/video_llm.srt`）的落账：登记成**普通**清单条目（不占 ASR 槽）。
///
/// 它不来自某一次 ASR，而是订正"当前这份字幕"的结果 —— 占槽会撒谎（跟 hybrid / single
/// 对不上），不登记又会从列表里消失（用户切走一次就再也切不回来）。
fn save_corrected_transcript(
    ws: &Workspace,
    config_file: &Path,
    output: &Path,
) -> anyhow::Result<()> {
    let llm_srt = output.join("video_llm.srt");
    if crate::assets::bind_derived_material(ws, crate::assets::AssetKind::Transcript, &llm_srt)
        .is_some()
    {
        return Ok(());
    }
    yekitrim_core::config::save_transcript_path_in(config_file, &rel_to_config(config_file, &llm_srt))
}


/// 「剪切收尾」：给一个**刚剪出来的**片段目录补精修工程（`polish.toml` + `data/`）。
///
/// 用户口径：**剪切这个动作本身就包含精修 init** —— 剪完的片段立刻能被精修页打开。
/// 切片页的「精修」按钮（`goPolish`）与首页「切片总览」卡片都只看
/// `polish_ready`（= `polish.toml` 存在）。放在剪切收尾里而不是等前端进页面时再补，
/// 是为了消掉那条竞态：前端发完 init 请求就跳转，工程还没落盘 → 精修页读到
/// 「片段配置读取失败：polish.toml not found」（慢盘、或 init 被资源键排队时必现）。
///
/// ⚠️ **副作用：`polish_ready` 不再是"用户进过精修"的信号** —— 每个已剪片段都为真。
/// 所以切片页的二级筛选（精修中 / 已导出）看的是**导出产物**（`rendered || cover`），
/// 不是 `polish_ready`：拿后者当筛选条件等于不筛（旧「未精修 / 已精修」就是这么退化的）。
/// 判据在 `webui/src/components/ProjectClipsSection.vue::isExported`，
/// 口径说明在 `webui/src/views/AGENTS.md`。
///
/// - **已有 `polish.toml` 一律不动**（`cmd_init` 自己也会报"已存在"）—— 绝不覆盖用户改过的工程；
/// - 目录里没有 `clip.mp4` 就跳过（`cmd_init` 会 `create_dir_all`，不能拿一条没剪成的事件建工程）；
/// - 并发初始化抢先建好（前端 / 去静音接口）时按"已经有了"处理，不报错。
///
/// 返回 `Ok(true)` = 这次真的新建了工程；`Ok(false)` = 无事可做。
pub async fn init_polish_for_clip(clip_dir: &Path) -> anyhow::Result<bool> {
    if clip_dir.join("polish.toml").exists() || !clip_dir.join("clip.mp4").exists() {
        return Ok(false);
    }
    match yekitrim_polish::commands::cmd_init(clip_dir).await {
        Ok(()) => {
            tracing::info!("剪切收尾：已创建精修工程 {}", clip_dir.display());
            Ok(true)
        }
        // 建完才发现已经有了 = 别人（前端 / 去静音接口）抢先建好，不算失败
        Err(e) => {
            if clip_dir.join("polish.toml").exists() {
                Ok(false)
            } else {
                Err(e)
            }
        }
    }
}

/// 剪切收尾的第二件小事：把精修页要用的**整段响度曲线**量好落盘（`<片段>/data/loudness.json`）。
///
/// `preview=1` 那次工程请求会带上这条曲线，而它是整段 ebur128（秒级）。页面 `load()` 是
/// **串行 await** 的（`PolishView.vue`）—— 少了这一步，每个片段第一次进页都必然白等它量完。
/// 挪到剪切收尾里预热之后，进页面那一步就只剩读文件。
///
/// **「只补缺失」**：缓存判定是 `clip.mp4` 的「文件名 + mtime + 字节数」（`measure_loudness_curve_cached`）——
/// 命中就**一个字节都不动**（用户/CLI 处理过一半的文件也照原样留着），只有真没缓存
/// （或引入 `key` 之前的旧格式，永远 miss）才当场量一次、就地补上。重剪过的话 mtime 变了，
/// 自然按新片段重算，不需要额外清理。
///
/// 失败**静默 warn**：剪切本身已经成功，量不到曲线顶多让页面退回"现场重算"的老行为
/// （与 `init_polish_for_clip` 只 warn 同口径）。
fn warm_loudness_cache(clip_dir: &Path) {
    let clip = clip_dir.join("clip.mp4");
    if !clip.exists() {
        // 与 init 同一条判据：事件没剪成时连 `data/` 都不该建
        return;
    }
    let curve = yekitrim_polish::render::measure_loudness_curve_cached(clip_dir, &clip);
    if curve.is_empty() {
        tracing::warn!(
            "剪切收尾：响度曲线测量失败 {}（不影响剪切结果，进精修时会重试）",
            clip_dir.display()
        );
    } else {
        tracing::debug!(
            "剪切收尾：响度曲线就绪 {}（{} 个采样点）",
            clip_dir.display(),
            curve.len()
        );
    }
}

/// 上面那条的批量版：按 `cutting::Cutter` 的同一套目录命名算出每个事件的目标目录，
/// 逐个补精修工程 + 预热响度曲线（[`warm_loudness_cache`]，只补缺失、命中零成本）。
/// **只 warn 不失败** —— 剪切本身已经成功，前端进精修时还有一次兜底
/// init（`yekitrim-server` 的 `ensure_polish_project`）。返回新建条数。
pub async fn init_polish_for_events(
    output_dir: &Path,
    events: &[yekitrim_core::types::Event],
) -> usize {
    let mut created = 0usize;
    for ev in events {
        let dir = output_dir
            .join("clips")
            .join(yekitrim_core::cutting::clip_dir_name_for_event(ev));
        match init_polish_for_clip(&dir).await {
            Ok(true) => created += 1,
            Ok(false) => {}
            Err(e) => tracing::warn!(
                "剪切收尾：创建精修工程失败 {}（不影响剪切结果）: {e:#}",
                dir.display()
            ),
        }
        // 工程在不在都要预热：重剪过同一片段时 `polish.toml` 已存在，但 `clip.mp4` 换了，
        // 缓存 key 跟着失效 —— 只判"有没有工程"会把这种片段漏掉。
        warm_loudness_cache(&dir);
    }
    created
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
        yekitrim_core::config::normalize_project_paths_in(config_file)?;
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
    let cfg = yekitrim_core::config::load_config_at(config_file)?;
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
            runner.add_stage(Box::new(yekitrim_core::audio::AudioExtractor));
            runner.run(&mut ctx).await?;
            if let Some(ap) = ctx.audio_path.as_ref() {
                yekitrim_core::config::save_audio_path_in(config_file, &rel_to_config(config_file, ap))?;
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
                r.add_stage(Box::new(yekitrim_core::audio::AudioExtractor));
                r.run(&mut ctx).await?;
                if let Some(ap) = ctx.audio_path.as_ref() {
                    yekitrim_core::config::save_audio_path_in(config_file, &rel_to_config(config_file, ap))?;
                }
            }
            yekitrim_core::download::ensure_models(&cfg.asr).await?;
            runner.add_stage(Box::new(yekitrim_core::asr::Transcriber));
            runner.run(&mut ctx).await?;
            save_transcript_after_asr(&ws, config_file, &output, &cfg)?;
        }
        "detect" => {
            // 字幕来源：`-t` 覆盖 → `transcript_file` → output/transcript.json → output/video.srt。
            // ⚠️ **必须认 `transcript_file`**：预处理页允许用户选一份外部字幕（跳过 ASR），
            // 只认 output/ 的话"换字幕后重新检测"读到的还是旧的那份。
            let tpath = transcript_override.or_else(|| ws.transcript_path(&cfg));
            let Some(tpath) = tpath else {
                anyhow::bail!("未找到转写文件（可在预处理页选择字幕文件）");
            };
            let raw = std::fs::read_to_string(&tpath)
                .with_context(|| format!("读取转写文件失败: {}", tpath.display()))?;
            let ext = tpath.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
            ctx.transcript = Some(if ext == "json" {
                serde_json::from_str(&raw)
                    .with_context(|| format!("转写 JSON 解析失败: {}", tpath.display()))?
            } else {
                yekitrim_core::srt::parse_srt(&raw)
                    .with_context(|| format!("字幕解析失败: {}", tpath.display()))?
            });
            ctx.detect_resume = Some(resume);
            runner.add_stage(Box::new(yekitrim_core::detection::EventDetector));
            runner.run(&mut ctx).await?;
            yekitrim_core::config::save_events_path_in(config_file, &rel_to_config(config_file, &output.join("events.json")))?;
        }
        "cut" => {
            let ev_path = events_override.unwrap_or_else(|| output.join("events.json"));
            if !ev_path.exists() {
                anyhow::bail!("events.json 不存在");
            }
            ctx.events = yekitrim_core::detection::load_events(&ev_path)?;
            runner.add_stage(Box::new(yekitrim_core::cutting::Cutter));
            runner.run(&mut ctx).await?;
            // 剪切收尾：新片段立刻带上精修工程（见 `init_polish_for_clip`）
            init_polish_for_events(&ctx.output_dir, &ctx.events).await;
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
    let cfg = yekitrim_core::config::load_config_at(config_file)?;
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
        run_until(cfg.clone(), progress, &mut ctx, STAGE_AUDIO, Box::new(yekitrim_core::audio::AudioExtractor)).await?;
        if let Some(ap) = ctx.audio_path.as_ref() {
            yekitrim_core::config::save_audio_path_in(config_file, &rel_to_config(config_file, ap))?;
        }
    } else {
        tracing::info!("音频已提取，跳过");
    }

    if current < STAGE_TRANSCRIBE {
        yekitrim_core::download::ensure_models(&cfg.asr).await?;
        run_until(cfg.clone(), progress, &mut ctx, STAGE_TRANSCRIBE, Box::new(yekitrim_core::asr::Transcriber)).await?;
        save_transcript_after_asr(&ws, config_file, &output, &cfg)?;
    } else {
        let recovery = ws.transcript_path(&cfg);
        match recovery {
            Some(path) => {
                let content = std::fs::read_to_string(&path)?;
                let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
                ctx.transcript = Some(if ext == "json" { serde_json::from_str(&content)? } else { yekitrim_core::srt::parse_srt(&content)? });
                tracing::info!("转写已存在: {}", path.display());
            }
            None => {
                yekitrim_core::download::ensure_models(&cfg.asr).await?;
                run_until(cfg.clone(), progress, &mut ctx, STAGE_TRANSCRIBE, Box::new(yekitrim_core::asr::Transcriber)).await?;
                save_transcript_after_asr(&ws, config_file, &output, &cfg)?;
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
                let (tx, mut rx) = tokio::sync::mpsc::channel::<yekitrim_core::llm::agent::AgentProgress<Vec<yekitrim_core::llm::agent::AgentOutput>>>(64);
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
                // 取消探针：订正跑在管线里（分钟级），用户在阶段边界点的停止必须能穿透到窗口边界
                let cancel: Option<yekitrim_core::llm::agent::CancelProbe> = progress.map(|p| {
                    let p = p.clone();
                    std::sync::Arc::new(move || p.cancelled()) as yekitrim_core::llm::agent::CancelProbe
                });
                let result = yekitrim_core::asr::correct::correct_transcript_with_checkpoint(
                    &segments, &danmaku, &cfg.llm, &cfg.correct,
                    ctx.output_dir.parent(), checkpoint_path, Some(tx), cancel,
                ).await;
                let _ = partial_task.await;
                if let Ok(corrected) = result {
                    let new = yekitrim_core::types::Transcript { segments: corrected, language: "zh".into(), asr_metadata: None };
                    let llm_srt = ctx.output_dir.join("video_llm.srt");
                    yekitrim_core::srt::write_srt(&new, &llm_srt)?;
                    save_corrected_transcript(&ws, config_file, &ctx.output_dir)?;
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
        run_until(cfg.clone(), progress, &mut ctx, STAGE_DETECT, Box::new(yekitrim_core::detection::EventDetector)).await?;
        yekitrim_core::config::save_events_path_in(config_file, &rel_to_config(config_file, &output.join("events.json")))?;
        if dry_run {
            tracing::info!("(--dry-run) 分析完成，未执行剪切。输出目录: {}", output.display());
            return Ok(());
        }
    } else {
        tracing::info!("事件检测已存在，跳过");
        let events_path = output.join("events.json");
        if events_path.exists() {
            ctx.events = yekitrim_core::detection::load_events(&events_path)?;
        }
    }

    if current < STAGE_CUT {
        if dry_run {
            tracing::info!("(--dry-run) 未执行剪切。输出目录: {}", output.display());
            return Ok(());
        }
        run_until(cfg, progress, &mut ctx, STAGE_CUT, Box::new(yekitrim_core::cutting::Cutter)).await?;
        // 剪切收尾：新片段立刻带上精修工程（与单阶段 `cut` 同一条路，见 `init_polish_for_clip`）
        init_polish_for_events(&ctx.output_dir, &ctx.events).await;
    } else {
        tracing::info!("剪切已完成，跳过");
    }

    Ok(())
}


/// 订正用的弹幕：`danmaku_file` → 视频同目录的 `danmu.ass` / `danmaku.ass`。
///
/// ⚠️ 用 `load_danmaku_auto`（XML / ASS / SRT 都认）而不是 `load_danmaku`（只认 ASS）——
/// 预处理页现在可以选 `.xml` / `.srt` 弹幕，走 ASS-only 的解析器等于"选了没有弹幕"。
fn load_danmaku_for_correct(cfg: &yekitrim_core::config::Config, ws: &Workspace) -> Vec<yekitrim_core::danmaku::DanmakuEntry> {
    let Some(path) = ws.danmaku_path(cfg) else {
        return Vec::new();
    };
    yekitrim_core::danmaku::load_danmaku_auto(&path).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 造一个"刚剪出来"的片段目录（`clip.mp4` + `clip.json`，够 `cmd_init` 认出标题）。
    fn made_clip(root: &Path) -> PathBuf {
        let dir = root.join("clips").join("clip-01");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("clip.mp4"), b"").unwrap();
        std::fs::write(
            dir.join("clip.json"),
            r#"{"title":"测试片段","duration":12.5,"rank":1}"#,
        )
        .unwrap();
        dir
    }

    /// 剪切收尾：没工程的片段补上，且**只建一次**（第二次不报错、不覆盖）。
    ///
    /// 这条是切片页「精修」按钮能直接进页面的前提（`polish_ready` = `polish.toml` 存在）。
    #[tokio::test]
    async fn init_polish_for_clip_creates_once_and_keeps_existing() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = made_clip(tmp.path());
        assert!(init_polish_for_clip(&dir).await.unwrap(), "首次应当建出 polish.toml");
        assert!(dir.join("polish.toml").exists());
        // 幂等：第二次是"已经有了"，既不是新建也不报错 —— 用户改过的工程不会被重做
        assert!(!init_polish_for_clip(&dir).await.unwrap());
        let raw = std::fs::read_to_string(dir.join("polish.toml")).unwrap();
        assert!(raw.contains("测试片段"), "工程名应取自 clip.json: {raw}");
    }

    /// 目录里没有 `clip.mp4`（事件没剪成）时一个文件都不许建 —— `cmd_init` 自己会
    /// `create_dir_all`，拿错目录去建工程会凭空造出一个片段目录。
    #[tokio::test]
    async fn init_polish_for_clip_skips_without_clip_mp4() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("clips").join("clip-01");
        std::fs::create_dir_all(&dir).unwrap();
        assert!(!init_polish_for_clip(&dir).await.unwrap());
        assert!(!dir.join("polish.toml").exists());
        assert!(!tmp.path().join("clips").join("clip-02").exists());
    }

    fn event(rank: u32, clip_dir: Option<&str>) -> yekitrim_core::types::Event {
        use yekitrim_core::types::{Event, EventSource, EventType, Timestamp};
        Event {
            start: Timestamp::from_seconds(0.0),
            end: Timestamp::from_seconds(10.0),
            label: "测试片段".into(),
            summary: String::new(),
            event_type: EventType::Other,
            source: EventSource::Rules,
            confidence: 1.0,
            popularity: 0.0,
            rank,
            clip_dir: clip_dir.map(str::to_string),
            enabled: true,
        }
    }

    /// 批量版必须按 **Cutter 的同一套目录命名**去找目录：`clip_dir` 优先、旧事件回落 `clip-{rank:02}`。
    #[tokio::test]
    async fn init_polish_for_events_follows_cutter_dir_naming() {
        let tmp = tempfile::tempdir().unwrap();
        for name in ["clip-01", "夏日杂谈"] {
            let dir = tmp.path().join("clips").join(name);
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("clip.mp4"), b"").unwrap();
        }
        let created = init_polish_for_events(
            tmp.path(),
            &[event(1, None), event(2, Some("夏日杂谈"))],
        )
        .await;
        assert_eq!(created, 2, "两个刚剪出来的目录都该补上工程");
        assert!(tmp.path().join("clips").join("clip-01").join("polish.toml").exists());
        assert!(tmp.path().join("clips").join("夏日杂谈").join("polish.toml").exists());
        // 幂等：再跑一次不再新建
        assert_eq!(
            init_polish_for_events(tmp.path(), &[event(1, None)]).await,
            0
        );
    }

    /// 预热**只补缺失**：缓存有效（key = 文件名 + mtime + 字节数，对得上）时一个字节都不动
    /// —— 包括用户/CLI 已经处理过一半的文件；反过来，没有 `clip.mp4` 的事件目录
    /// 连 `data/` 都不许建（与 init 同一条判据）。
    #[tokio::test]
    async fn warm_loudness_cache_keeps_valid_cache_and_skips_uncut_dirs() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = made_clip(tmp.path());
        let meta = std::fs::metadata(dir.join("clip.mp4")).unwrap();
        let mtime = mtime_secs(&meta);
        let cache = dir.join("data").join("loudness.json");
        std::fs::create_dir_all(cache.parent().unwrap()).unwrap();
        let raw = format!(
            r#"{{"key":"clip.mp4|{mtime}|{}","points":[{{"t":0.1,"db":-20.0}}]}}"#,
            meta.len()
        );
        std::fs::write(&cache, &raw).unwrap();

        init_polish_for_events(tmp.path(), &[event(1, None), event(2, None)]).await;

        assert_eq!(
            std::fs::read_to_string(&cache).unwrap(),
            raw,
            "命中缓存不许重写（连带 key 的格式都不动）"
        );
        assert!(
            !tmp.path().join("clips").join("clip-02").exists(),
            "没有 clip.mp4 的事件目录一个文件都不该建"
        );
    }

    /// 真 ffmpeg 的端到端钉子（`#[ignore]`：需要本机有 ffmpeg，手动跑）：
    /// 剪切收尾对新片段**真的量出一条带 key 的响度曲线**，二次收尾命中缓存一动不动。
    ///
    /// ```text
    /// cargo test -p yekitrim-service --lib warm_loudness_cache_with_real_ffmpeg -- --ignored --nocapture
    /// ```
    #[tokio::test]
    #[ignore]
    async fn warm_loudness_cache_with_real_ffmpeg() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("clips").join("clip-01");
        std::fs::create_dir_all(&dir).unwrap();
        let clip = dir.join("clip.mp4");
        let status = std::process::Command::new("ffmpeg")
            .args([
                "-y", "-hide_banner", "-loglevel", "error",
                "-f", "lavfi", "-i", "testsrc2=size=320x240:rate=15",
                "-f", "lavfi", "-i", "sine=frequency=440",
                "-t", "6", "-c:v", "libx264", "-preset", "ultrafast",
                "-pix_fmt", "yuv420p", "-c:a", "aac", "-shortest",
            ])
            .arg(&clip)
            .status()
            .expect("这条钉子需要本机 ffmpeg（`--ignored` 才跑）");
        assert!(status.success(), "造测试片段失败");
        std::fs::write(
            dir.join("clip.json"),
            r#"{"title":"测试片段","duration":6.0,"rank":1}"#,
        )
        .unwrap();

        let t0 = std::time::Instant::now();
        init_polish_for_events(tmp.path(), &[event(1, None)]).await;
        let cold = t0.elapsed();

        let cache = dir.join("data").join("loudness.json");
        let raw = std::fs::read_to_string(&cache).expect("剪切收尾应当量出 loudness.json");
        let v: serde_json::Value = serde_json::from_str(&raw).unwrap();
        let meta = std::fs::metadata(&clip).unwrap();
        let want_key = format!("clip.mp4|{}|{}", mtime_secs(&meta), meta.len());
        assert_eq!(
            v["key"].as_str().unwrap(),
            want_key,
            "新写的缓存必须按当前 clip.mp4 算 key（这就是页面读的那份）"
        );
        let points = v["points"].as_array().unwrap().len();
        assert!(points > 0, "曲线不能是空的");
        assert!(dir.join("polish.toml").exists());

        let mtime_before = std::fs::metadata(&cache).unwrap().modified().unwrap();
        let t1 = std::time::Instant::now();
        init_polish_for_events(tmp.path(), &[event(1, None)]).await;
        let hot = t1.elapsed();
        assert_eq!(std::fs::read_to_string(&cache).unwrap(), raw, "命中不许重写");
        assert_eq!(
            std::fs::metadata(&cache).unwrap().modified().unwrap(),
            mtime_before,
            "命中连 mtime 都不该动"
        );
        println!(
            "冷收尾（init + 量曲线）{:?} / 热收尾（命中）{:?} / {points} 个采样点",
            cold, hot
        );
    }

    fn mtime_secs(meta: &std::fs::Metadata) -> u64 {
        meta.modified()
            .unwrap()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs()
    }
}

