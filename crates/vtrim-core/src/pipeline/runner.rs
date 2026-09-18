use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{bail, Result};

use super::stage::Stage;
use crate::config::Config;
use crate::progress::ProgressReporter;
use crate::types::PipelineContext;

/// 解析 `danmaku_file`：相对路径按**工程根**（= `output_dir` 的父目录）解析，
/// 找不到再退回视频同目录。
///
/// `danmaku_file` 是相对工程目录写进 vtrim.toml 的（`vtrim init` 的 `path_to_rel`，
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

    /// 受 state 约束的运行，单 stage 命令使用
    pub async fn run(&self, ctx: &mut PipelineContext) -> Result<()> {
        inject_config(ctx, &self.config);
        self.inject_progress(ctx);
        let current = self.config.project.current_stage(&ctx.output_dir);

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
            ctx.progress.stage_started(name);
            tracing::info!("[{}] started", name);
            stage.run(ctx).await?;
            tracing::info!("[{}] completed", name);
            ctx.progress.stage_completed(name);
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
        let current = self.config.project.current_stage(&ctx.output_dir);

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
            ctx.progress.stage_started(name);
            tracing::info!("[{}] started", name);
            stage.run(ctx).await?;
            tracing::info!("[{}] completed", name);
            ctx.progress.stage_completed(name);
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
        let dir = std::env::temp_dir().join(format!("vtrim-danmaku-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("video.mp4"), b"x").unwrap();
        std::fs::write(dir.join("dm.ass"), b"[Events]\n").unwrap();

        // 相对路径 = 相对工程目录（vtrim.toml 就是这么存的）→ 必须解析成绝对路径且找得到
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
        let dir = std::env::temp_dir().join(format!("vtrim-danmaku-nested-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("videos")).unwrap();
        std::fs::write(dir.join("videos/video.mp4"), b"x").unwrap();
        std::fs::write(dir.join("dm.xml"), b"<i/>").unwrap();

        let mut ctx = PipelineContext::new(dir.join("videos/video.mp4"), dir.join("output"));
        inject_config(&mut ctx, &config_with_danmaku("dm.xml"));
        assert_eq!(ctx.danmaku_path.unwrap(), dir.join("dm.xml"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 真实工程只读诊断：把 `vtrim.toml` + 视频目录喂进解析逻辑，打印解析出的弹幕路径
    /// 与热力图概况（**不写任何文件**）。用来核对"检测到底读到没读到弹幕"。
    ///
    /// ```text
    /// VT_PROJECT_DIR=<项目目录> cargo test -p vtrim-core --lib diagnose_danmaku_resolution -- --ignored --nocapture
    /// ```
    #[test]
    #[ignore]
    fn diagnose_danmaku_resolution() {
        let Ok(dir) = std::env::var("VT_PROJECT_DIR") else {
            eprintln!("用法: VT_PROJECT_DIR=<项目目录> cargo test -p vtrim-core --lib diagnose_danmaku_resolution -- --ignored --nocapture");
            return;
        };
        let dir = PathBuf::from(dir);
        let config = crate::config::load_config_at(&dir.join("vtrim.toml")).expect("读取 vtrim.toml");
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
}
