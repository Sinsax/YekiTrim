//! pipeline::runner：管线编排器（顺序驱动 stage，按文件存在性决定跑不跑、从哪跑）。
//!
//! 入口：`PipelineRunner::{new, add_stage, config, set_progress, run, run_all, run_until}`。
//! 边界：三种运行模式各对应一类命令，别各写一套；弹幕相对路径解析只在这里的 `resolve_danmaku`（工程根 → 视频父目录，两个基准都试） —— 见 `crates/yekitrim-core/src/pipeline/AGENTS.md`

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{bail, Result};

use super::stage::{Stage, STAGE_AUDIO, STAGE_DETECT, STAGE_TRANSCRIBE};
use crate::config::Config;
use crate::progress::ProgressReporter;
use crate::types::PipelineContext;

/// 解析 `danmaku_file`：相对路径按**工程根**（= `output_dir` 的父目录）解析，
/// 找不到再退回视频同目录。
///
/// `danmaku_file` 是相对工程目录写进 yekitrim.toml 的（`yekitrim init` 的 `path_to_rel`，
/// 写入侧一律按工程根），所以**工程根是权威基准**；视频同目录只是老习惯的兜底
/// （项目里的素材通常和视频放在一起）。只按视频目录解析会在
/// 「视频在 `root/videos/`、弹幕在 `root/dm.xml`」这类布局下静默失效。
/// ⚠ 少了这一步，相对路径会被当成"相对进程 CWD"：弹幕永远找不到，
/// 热度静默退化成纯字幕密度、切片也不会生成 `clip.dm.srt`。
fn resolve_danmaku(ctx: &PipelineContext, raw: &str) -> PathBuf {
    let path = PathBuf::from(raw);
    if path.is_absolute() {
        return path;
    }
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Some(root) = ctx.output_dir.parent() {
        candidates.push(root.join(&path));
    }
    if let Some(dir) = ctx.input_path.parent() {
        candidates.push(dir.join(&path));
    }
    for c in &candidates {
        if c.is_file() {
            return c.clone();
        }
    }
    candidates.into_iter().next().unwrap_or(path)
}

fn inject_config(ctx: &mut PipelineContext, config: &Arc<Config>) {
    ctx.config = config.clone();
    ctx.danmaku_path = config
        .project
        .danmaku_file
        .as_ref()
        .filter(|p| !p.is_empty())
        .map(|raw| resolve_danmaku(ctx, raw));
}

pub struct PipelineRunner {
    stages: Vec<Box<dyn Stage>>,
    config: Arc<Config>,
    progress: Option<Arc<dyn ProgressReporter>>,
}

impl PipelineRunner {
    pub fn new(config: Config) -> Self {
        Self {
            stages: Vec::new(),
            config: Arc::new(config),
            progress: None,
        }
    }

    pub fn add_stage(&mut self, stage: Box<dyn Stage>) {
        self.stages.push(stage);
    }

    pub fn config(&self) -> &Config {
        &self.config
    }

    pub fn set_progress(&mut self, progress: Arc<dyn ProgressReporter>) {
        self.progress = Some(progress);
    }

    fn inject_progress(&self, ctx: &mut PipelineContext) {
        if let Some(ref p) = self.progress {
            ctx.progress = p.clone();
        }
    }

    /// **阶段预检**：输出盘（`output_dir` 所在卷）的可用空间必须 ≥ `[cutting] min_free_gb`
    /// （`0` = 关闭）。不达标就在这里**停下整个阶段**（`disk::FreeSpaceError`，批量调用方
    /// 可 downcast 识别后停整批），而不是让 stage 切到一半把盘写穿、被外部磁盘看门狗
    /// 按最旧优先把源录播删掉（2026-11-15 事故：一键全切写穿数据盘 → diskguard 删源）。
    fn ensure_stage_disk_free(&self, ctx: &PipelineContext) -> Result<()> {
        crate::disk::ensure_free(&ctx.output_dir, self.config.cutting.min_free_bytes())
    }

    /// `PipelineContext` 里**已经装好**的输入能证明的最高阶段。
    ///
    /// 服务层在调 `run()` 之前会把 `-t` / `-e` 覆盖或用户选中的素材读进 ctx
    /// （`ctx.transcript` / `ctx.events`）。那些文件**不一定在 `output/` 下**
    /// （外部字幕、外部 events），只按 `current_stage(&output_dir)` 的文件判据会把它判成
    /// stage 0，于是 `detect`(3) / `cut`(4) 被自己的状态机拒掉 —— 用户明明已经把输入递进来了。
    fn ctx_stage_floor(ctx: &PipelineContext) -> u32 {
        if !ctx.events.is_empty() {
            STAGE_DETECT
        } else if ctx.transcript.is_some() {
            STAGE_TRANSCRIBE
        } else if ctx.audio_path.is_some() {
            STAGE_AUDIO
        } else {
            0
        }
    }

    /// 受 state 约束的运行，单 stage 命令使用
    pub async fn run(&self, ctx: &mut PipelineContext) -> Result<()> {
        inject_config(ctx, &self.config);
        self.inject_progress(ctx);
        let mut current = self.config.project.current_stage(&ctx.output_dir);
        current = current.max(Self::ctx_stage_floor(ctx));

        for stage in &self.stages {
            let num = stage.stage_number();
            if current + 1 < num {
                bail!(
                    "阶段 '{}' (#{}) 需要先完成前面的阶段 (当前: #{})",
                    stage.name(),
                    num,
                    current
                );
            }

            let name = stage.name();
            if ctx.progress.cancelled() {
                bail!("任务已取消");
            }
            // 阶段预检：空间不足 = 整个阶段不开工（自动处理也一样，见 `ensure_stage_disk_free`）
            self.ensure_stage_disk_free(ctx)?;
            ctx.progress.stage_started(name);
            tracing::info!("[{}] started", name);
            stage.run(ctx).await?;
            tracing::info!("[{}] completed", name);
            ctx.progress.stage_completed(name);
            // 「完成后 stage 自动推进」：同一个 runner 里的下一个 stage 不该再拿旧基准判门槛，
            // 否则多 stage 链（`run_until(ctx, 4)`）在空工程上跑到第 2 个就被自己拦下。
            current = current.max(num);
        }

        Ok(())
    }

    /// 无视文件检查从头跑到底（用于 process 命令）
    pub async fn run_all(&self, ctx: &mut PipelineContext) -> Result<()> {
        inject_config(ctx, &self.config);
        self.inject_progress(ctx);

        for stage in &self.stages {
            let name = stage.name();
            if ctx.progress.cancelled() {
                bail!("任务已取消");
            }
            // 阶段预检：空间不足 = 整个阶段不开工（自动处理也一样，见 `ensure_stage_disk_free`）
            self.ensure_stage_disk_free(ctx)?;
            ctx.progress.stage_started(name);
            tracing::info!("[{}] started", name);
            stage.run(ctx).await?;
            tracing::info!("[{}] completed", name);
            ctx.progress.stage_completed(name);
        }

        Ok(())
    }

    /// 运行到指定阶段为止（用于 --dry-run）
    pub async fn run_until(&self, ctx: &mut PipelineContext, max_stage: u32) -> Result<()> {
        inject_config(ctx, &self.config);
        self.inject_progress(ctx);
        let mut current = self.config.project.current_stage(&ctx.output_dir);
        current = current.max(Self::ctx_stage_floor(ctx));

        for stage in &self.stages {
            let num = stage.stage_number();
            let name = stage.name();
            if num > max_stage {
                tracing::info!("[{}] stopped by --dry-run", name);
                break;
            }
            if current + 1 < num {
                bail!(
                    "阶段 '{}' (#{}) 需要先完成前面的阶段 (当前: #{})",
                    name,
                    num,
                    current
                );
            }

            if ctx.progress.cancelled() {
                bail!("任务已取消");
            }
            // 阶段预检：空间不足 = 整个阶段不开工（自动处理也一样，见 `ensure_stage_disk_free`）
            self.ensure_stage_disk_free(ctx)?;
            ctx.progress.stage_started(name);
            tracing::info!("[{}] started", name);
            stage.run(ctx).await?;
            tracing::info!("[{}] completed", name);
            ctx.progress.stage_completed(name);
            // 同 `run()`：跑完就推进基准，下一阶段的门槛按「前面已经完成」来算。
            current = current.max(num);
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use std::path::Path;

    fn ctx_in(dir: &Path) -> PipelineContext {
        PipelineContext::new(dir.join("video.mp4"), dir.join("output"))
    }

    fn config_with_danmaku(value: &str) -> Arc<Config> {
        let mut config = Config::default();
        config.project.danmaku_file = Some(value.to_string());
        Arc::new(config)
    }

    #[test]
    fn relative_danmaku_file_resolves_against_video_dir() {
        let dir = std::env::temp_dir().join(format!("yekitrim-danmaku-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("video.mp4"), b"x").unwrap();
        std::fs::write(dir.join("dm.ass"), b"[Events]\n").unwrap();

        // 相对路径 = 相对工程目录（yekitrim.toml 就是这么存的）→ 必须解析成绝对路径且找得到
        let mut ctx = ctx_in(&dir);
        inject_config(&mut ctx, &config_with_danmaku("dm.ass"));
        let resolved = ctx.danmaku_path.clone().unwrap();
        assert!(resolved.is_absolute(), "相对路径必须解析成绝对路径: {}", resolved.display());
        assert!(resolved.exists(), "应按视频所在目录解析: {}", resolved.display());

        // 绝对路径原样保留
        let mut ctx = ctx_in(&dir);
        let abs = dir.join("dm.ass").to_string_lossy().into_owned();
        inject_config(&mut ctx, &config_with_danmaku(&abs));
        assert_eq!(ctx.danmaku_path.unwrap(), dir.join("dm.ass"));

        // 空配置 → 没有弹幕（不 panic、不产生空路径）
        let mut ctx = ctx_in(&dir);
        inject_config(&mut ctx, &config_with_danmaku(""));
        assert!(ctx.danmaku_path.is_none());

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 视频在子目录、弹幕在工程根：`danmaku_file` 是相对**工程根**写的，
    /// 只按视频目录解析会静默失效（热度退化成纯字幕密度、`clip.dm.srt` 也没有）。
    #[test]
    fn danmaku_in_project_root_resolves_when_video_is_nested() {
        let dir = std::env::temp_dir().join(format!("yekitrim-danmaku-nested-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("videos")).unwrap();
        std::fs::write(dir.join("videos/video.mp4"), b"x").unwrap();
        std::fs::write(dir.join("dm.xml"), b"<i/>").unwrap();

        let mut ctx = PipelineContext::new(dir.join("videos/video.mp4"), dir.join("output"));
        inject_config(&mut ctx, &config_with_danmaku("dm.xml"));
        assert_eq!(ctx.danmaku_path.unwrap(), dir.join("dm.xml"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 真实工程只读诊断：把 `yekitrim.toml` + 视频目录喂进解析逻辑，打印解析出的弹幕路径
    /// 与热力图概况（**不写任何文件**）。用来核对"检测到底读到没读到弹幕"。
    ///
    /// ```text
    /// VT_PROJECT_DIR=<项目目录> cargo test -p yekitrim-core --lib diagnose_danmaku_resolution -- --ignored --nocapture
    /// ```
    #[test]
    #[ignore]
    fn diagnose_danmaku_resolution() {
        let Ok(dir) = std::env::var("VT_PROJECT_DIR") else {
            eprintln!("用法: VT_PROJECT_DIR=<项目目录> cargo test -p yekitrim-core --lib diagnose_danmaku_resolution -- --ignored --nocapture");
            return;
        };
        let dir = PathBuf::from(dir);
        let config = crate::config::load_config_at(&dir.join(crate::path::PROJECT_CONFIG_FILE)).expect("读取 yekitrim.toml");
        let input = config
            .project
            .input_file
            .as_ref()
            .filter(|s| !s.is_empty())
            .map(|s| dir.join(s))
            .unwrap_or_else(|| dir.join("video.mp4"));
        println!("项目目录: {}", dir.display());
        println!("配置 danmaku_file: {:?}", config.project.danmaku_file);

        let mut ctx = PipelineContext::new(input, dir.join("output"));
        inject_config(&mut ctx, &Arc::new(config.clone()));
        match &ctx.danmaku_path {
            Some(p) => println!("解析结果: {} （存在={}）", p.display(), p.exists()),
            None => {
                println!("解析结果: 无（配置为空）");
                return;
            }
        }
        let path = ctx.danmaku_path.clone().unwrap();
        let Ok(entries) = crate::danmaku::load_danmaku_auto(&path) else {
            println!("⚠ 弹幕解析失败");
            return;
        };
        let total_end = entries.iter().map(|e| e.end).fold(0.0, f64::max);
        let windows = crate::danmaku::analyze(&path, config.detection.danmaku_window_secs, total_end)
            .unwrap_or_default();
        let peak = windows.iter().map(|w| w.count).max().unwrap_or(0);
        println!(
            "弹幕 {} 条，覆盖到 {:.1}s；热力图 {} 个窗口（{:.0}s/窗），单窗峰值 {} 条",
            entries.len(),
            total_end,
            windows.len(),
            config.detection.danmaku_window_secs,
            peak
        );
        assert!(!windows.is_empty(), "修复后热力图不该为空");
    }

    /// 记录被调用过的 stage 编号。
    struct FakeStage {
        num: u32,
        ran: std::sync::Arc<std::sync::Mutex<Vec<u32>>>,
    }

    #[async_trait::async_trait]
    impl Stage for FakeStage {
        fn name(&self) -> &'static str {
            "fake"
        }
        fn stage_number(&self) -> u32 {
            self.num
        }
        async fn run(&self, _ctx: &mut PipelineContext) -> Result<()> {
            self.ran.lock().unwrap().push(self.num);
            Ok(())
        }
    }

    /// `run_until(max)` 的契约是「跑到指定 stage 为止」，可基准以前只在进入循环前算一次：
    /// 空工程上 stage 1 跑完，stage 2 仍按 `current = 0` 判 `0 + 1 < 2` ⇒ 直接 bail。
    /// 这条钉住「跑完一个 stage 基准就推进」。
    #[tokio::test]
    async fn run_until_advances_the_state_machine_between_stages() {
        let dir = std::env::temp_dir().join(format!("yekitrim-runner-chain-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("output")).unwrap();

        let ran = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let mut runner = PipelineRunner::new(Config::default());
        for num in 1..=4u32 {
            runner.add_stage(Box::new(FakeStage { num, ran: ran.clone() }));
        }
        let mut ctx = PipelineContext::new(dir.join("video.mp4"), dir.join("output"));
        runner
            .run_until(&mut ctx, 4)
            .await
            .expect("空工程上 run_until(4) 应跑完 1..=4（基准必须随完成阶段推进）");
        assert_eq!(*ran.lock().unwrap(), vec![1, 2, 3, 4]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// `-t` / `-e` 把输入读进 ctx 之后，状态机不该再按 `output/` 下的文件判成 stage 0：
    /// 服务层 `detect -t <外部转录>` 在 output 还空着时会被自己拒掉（"需要先完成前面的阶段"）。
    #[tokio::test]
    async fn preloaded_context_input_satisfies_the_stage_gate() {
        let ran = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));

        // 外部转录已读进 ctx ⇒ detect(3) 可跑
        let mut ctx = PipelineContext::new(
            PathBuf::from("/nonexistent/video.mp4"),
            PathBuf::from("/nonexistent/yekitrim-output"),
        );
        ctx.transcript = Some(crate::types::Transcript {
            segments: vec![crate::types::TranscriptSegment {
                start: 0.0,
                end: 1.0,
                text: "你好".into(),
                confidence: 1.0,
            }],
            language: "zh".into(),
            asr_metadata: None,
        });
        let mut runner = PipelineRunner::new(Config::default());
        runner.add_stage(Box::new(FakeStage { num: 3, ran: ran.clone() }));
        runner
            .run(&mut ctx)
            .await
            .expect("ctx.transcript 已就位，detect 不该被门槛拦下");
        assert_eq!(*ran.lock().unwrap(), vec![3]);

        // 外部 events 已读进 ctx ⇒ cut(4) 可跑
        let ran_cut = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let mut ctx = PipelineContext::new(
            PathBuf::from("/nonexistent/video.mp4"),
            PathBuf::from("/nonexistent/yekitrim-output"),
        );
        ctx.events = vec![crate::types::Event {
            start: crate::types::Timestamp::from_seconds(0.0),
            end: crate::types::Timestamp::from_seconds(10.0),
            label: "t".into(),
            summary: String::new(),
            event_type: crate::types::EventType::Other,
            source: crate::types::EventSource::LLM,
            confidence: 1.0,
            popularity: 0.0,
            rank: 0,
            clip_dir: None,
            enabled: true,
            ranges: Vec::new(),
            topic: None,
        }];
        let mut runner = PipelineRunner::new(Config::default());
        runner.add_stage(Box::new(FakeStage { num: 4, ran: ran_cut.clone() }));
        runner
            .run(&mut ctx)
            .await
            .expect("ctx.events 已就位，cut 不该被门槛拦下");
        assert_eq!(*ran_cut.lock().unwrap(), vec![4]);
    }

    /// **阶段预检**：输出盘余量低于 `[cutting] min_free_gb` ⇒ runner 在 stage 开工前就停，
    /// stage 一次都不执行，错误是可 downcast 的 `disk::FreeSpaceError`
    /// （一键全切等批量调用方靠它把「没让剪」与「剪切失败」分开，然后停整批）。
    #[tokio::test]
    async fn runner_aborts_stage_before_it_runs_when_disk_precheck_fails() {
        let dir = std::env::temp_dir().join(format!("yekitrim-disk-guard-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let ran = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let mut config = Config::default();
        // 阈值拉满（saturating 乘法不回绕）⇒ 任何真实磁盘都必然不足
        config.cutting.min_free_gb = u64::MAX;
        let mut runner = PipelineRunner::new(config);
        runner.add_stage(Box::new(FakeStage { num: 1, ran: ran.clone() }));
        let mut ctx = ctx_in(&dir);
        let err = runner
            .run_all(&mut ctx)
            .await
            .expect_err("阈值 u64::MAX ⇒ 预检必失败");
        // ⚠️ 查**整条错误链**（to_string() 只有最外层 context）
        assert!(format!("{err:#}").contains("磁盘空间不足"), "{err:#}");
        assert!(
            err.downcast_ref::<crate::disk::FreeSpaceError>().is_some(),
            "必须是可识别的 FreeSpaceError"
        );
        assert!(
            ran.lock().unwrap().is_empty(),
            "预检不过 ⇒ stage 一次都不能执行"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 健康磁盘（默认阈值）上预检必须放行 —— 守卫不能把正常管线拦死。
    #[tokio::test]
    async fn runner_disk_precheck_passes_on_healthy_disk() {
        let ran = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let mut runner = PipelineRunner::new(Config::default());
        runner.add_stage(Box::new(FakeStage { num: 1, ran: ran.clone() }));
        let mut ctx = PipelineContext::new(
            PathBuf::from("/nonexistent/video.mp4"),
            std::env::temp_dir().join("yekitrim-disk-guard-ok"),
        );
        runner
            .run_all(&mut ctx)
            .await
            .expect("默认阈值下 temp 盘应放行");
        assert_eq!(*ran.lock().unwrap(), vec![1]);
    }
}
