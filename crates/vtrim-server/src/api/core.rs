//! `api::core`：管线执行入口与进度桥。

use std::path::PathBuf;
use std::sync::Arc;



use tokio::runtime::Handle;

use crate::jobs::JobManager;

/// WebUI 任务执行入口：复用 vtrim_service::core_pipeline，只做 Job/进度桥接。
pub async fn run_stages(
    config_file: PathBuf,
    stages: Vec<String>,
    dry_run: bool,
    resume: bool,
    job_id: String,
    jobs: Arc<JobManager>,
    cancel: Option<Arc<std::sync::atomic::AtomicBool>>,
) -> anyhow::Result<()> {
    let progress: Arc<dyn vtrim_core::progress::ProgressReporter> =
        Arc::new(WebProgress::new(job_id, jobs, cancel));
    vtrim_service::run_stages(
        &config_file,
        &stages,
        dry_run,
        resume,
        Some(progress),
    ).await
}

/// 任务进度桥：把 `ProgressReporter` 的 `stage_*` 调用映射成 Job 事件。
///
/// 管线之外的异步任务（如表情包的 LLM 标签构建）也走这一份 —— 进度只该有一套桥。
pub(crate) fn progress_bridge(
    job_id: String,
    jobs: Arc<JobManager>,
    cancel: Option<Arc<std::sync::atomic::AtomicBool>>,
) -> Arc<dyn vtrim_core::progress::ProgressReporter> {
    Arc::new(WebProgress::new(job_id, jobs, cancel))
}

pub(super) struct WebProgress {
    id: String,
    jobs: Arc<JobManager>,
    /// 任务的取消令牌：管线在阶段边界轮询它。
    cancel: Option<Arc<std::sync::atomic::AtomicBool>>,
}

impl WebProgress {
    fn new(
        id: String,
        jobs: Arc<JobManager>,
        cancel: Option<Arc<std::sync::atomic::AtomicBool>>,
    ) -> Self {
        Self { id, jobs, cancel }
    }
}

#[allow(clippy::let_underscore_future)]
impl vtrim_core::progress::ProgressReporter for WebProgress {
    fn cancelled(&self) -> bool {
        self.cancel
            .as_ref()
            .map(|c| c.load(std::sync::atomic::Ordering::SeqCst))
            .unwrap_or(false)
    }
    fn stage_started(&self, name: &str) {
        let jobs = self.jobs.clone();
        let id = self.id.clone();
        let name = name.to_string();
        if let Ok(handle) = Handle::try_current() {
            let _ = handle.spawn(async move {
                jobs.stage_started(&id, &name, stage_num(&name)).await;
            });
        }
    }
    fn stage_progress(&self, current: u64, total: u64) {
        let jobs = self.jobs.clone();
        let id = self.id.clone();
        if let Ok(handle) = Handle::try_current() {
            let _ = handle.spawn(async move {
                jobs.progress(&id, current, total).await;
            });
        }
    }
    fn stage_completed(&self, name: &str) {
        let jobs = self.jobs.clone();
        let id = self.id.clone();
        let name = name.to_string();
        if let Ok(handle) = Handle::try_current() {
            let _ = handle.spawn(async move {
                jobs.stage_completed(&id, &name).await;
            });
        }
    }
    fn error(&self, msg: &str) {
        let jobs = self.jobs.clone();
        let id = self.id.clone();
        let msg = msg.to_string();
        if let Ok(handle) = Handle::try_current() {
            let _ = handle.spawn(async move {
                jobs.fail(&id, msg).await;
            });
        }
    }
}

pub(super) fn stage_num(name: &str) -> u32 {
    match name {
        "AudioExtractor" => 1,
        "Transcriber" => 2,
        "EventDetector" => 3,
        "Cutter" => 4,
        _ => 0,
    }
}
