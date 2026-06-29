use std::sync::Arc;

use anyhow::{bail, Result};

use super::stage::{stage_enabled, Stage, STAGE_CUT};
use crate::config::{self, Config};
use crate::progress::ProgressReporter;
use crate::types::PipelineContext;

fn inject_config(ctx: &mut PipelineContext, config: &Arc<Config>) {
    ctx.config = config.clone();
    ctx.danmaku_path = config
        .project
        .danmaku_file
        .as_ref()
        .filter(|p| !p.is_empty())
        .map(std::path::PathBuf::from);
}

fn advance_past_disabled(mut current: u32, config: &Config) -> Result<u32> {
    while current < STAGE_CUT && !stage_enabled(current + 1, config) {
        current += 1;
        config::save_current_stage(current)?;
    }
    Ok(current)
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
        let mut current = advance_past_disabled(config::load_current_stage()?, &self.config)?;

        for stage in &self.stages {
            let num = stage.stage_number();
            if !stage.is_enabled(&self.config) {
                tracing::info!("[{}] skipped (disabled)", stage.name());
                if num > current {
                    config::save_current_stage(num)?;
                    current = num;
                }
                continue;
            }

            if current + 1 < num {
                bail!(
                    "阶段 '{}' (#{}) 需要先完成前面的阶段 (当前: #{})",
                    stage.name(),
                    num,
                    current
                );
            }

            let name = stage.name();
            ctx.progress.stage_started(name);
            tracing::info!("[{}] started", name);
            stage.run(ctx).await?;
            tracing::info!("[{}] completed", name);
            ctx.progress.stage_completed(name);

            if num > current {
                config::save_current_stage(num)?;
                current = num;
            }
        }

        Ok(())
    }

    /// 全量运行，无视 state 检查，process 命令使用
    pub async fn run_all(&self, ctx: &mut PipelineContext) -> Result<()> {
        inject_config(ctx, &self.config);
        self.inject_progress(ctx);
        for stage in &self.stages {
            let name = stage.name();
            if !stage.is_enabled(&self.config) {
                tracing::info!("[{}] skipped (disabled)", name);
                continue;
            }
            ctx.progress.stage_started(name);
            tracing::info!("[{}] started", name);
            stage.run(ctx).await?;
            tracing::info!("[{}] completed", name);
            ctx.progress.stage_completed(name);
        }

        config::save_current_stage(STAGE_CUT)?;
        Ok(())
    }

    /// 运行到指定阶段为止（用于 --dry-run）
    pub async fn run_until(&self, ctx: &mut PipelineContext, max_stage: u32) -> Result<()> {
        inject_config(ctx, &self.config);
        self.inject_progress(ctx);
        let mut current = advance_past_disabled(config::load_current_stage()?, &self.config)?;

        for stage in &self.stages {
            let num = stage.stage_number();
            let name = stage.name();
            if num > max_stage {
                tracing::info!("[{}] stopped by --dry-run", name);
                break;
            }
            if !stage.is_enabled(&self.config) {
                tracing::info!("[{}] skipped (disabled)", name);
                if num > current {
                    config::save_current_stage(num)?;
                    current = num;
                }
                continue;
            }

            if current + 1 < num {
                bail!(
                    "阶段 '{}' (#{}) 需要先完成前面的阶段 (当前: #{})",
                    name,
                    num,
                    current
                );
            }

            ctx.progress.stage_started(name);
            tracing::info!("[{}] started", name);
            stage.run(ctx).await?;
            tracing::info!("[{}] completed", name);
            ctx.progress.stage_completed(name);

            if num > current {
                config::save_current_stage(num)?;
                current = num;
            }
        }

        Ok(())
    }
}
