//! `api::core`：管线执行入口与进度桥。

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tokio::runtime::Handle;

use crate::jobs::JobManager;

/// WebUI 任务执行入口：复用 yekitrim_service::core_pipeline，只做 Job/进度桥接。
pub async fn run_stages(
    config_file: PathBuf,
    stages: Vec<String>,
    dry_run: bool,
    resume: bool,
    job_id: String,
    jobs: Arc<JobManager>,
    cancel: Option<Arc<std::sync::atomic::AtomicBool>>,
) -> anyhow::Result<()> {
    let progress: Arc<dyn yekitrim_core::progress::ProgressReporter> =
        Arc::new(WebProgress::new(job_id, jobs, cancel));
    yekitrim_service::run_stages(
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
) -> Arc<dyn yekitrim_core::progress::ProgressReporter> {
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
impl yekitrim_core::progress::ProgressReporter for WebProgress {
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
    /// `note()` = 任务框的一句提示，**不是**失败（trait 默认 no-op，见 core 的
    /// `ProgressReporter`）。管线侧的部分剪切失败/跳过汇总走这里 —— 不接线的话
    /// 「部分失败 N 条」只进日志、任务框什么都看不见。转成 `jobs.message`（一行文案），
    /// **绝不升格成 fail**（部分失败 ≠ 任务失败，口径见 core 的 note 注释）。
    fn note(&self, msg: &str) {
        let jobs = self.jobs.clone();
        let id = self.id.clone();
        let msg = msg.to_string();
        if let Ok(handle) = Handle::try_current() {
            let _ = handle.spawn(async move {
                jobs.message(&id, msg).await;
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

/// 模型下载进度桥：把 core 的字节回调翻译成任务的 `progress` 事件 + 文案。
///
/// ⚠️ **必须节流**：`download_file` 每个 TCP chunk 都回调一次（8 KB 一块）。150 KB/s 时约 20 次/秒，
/// 千兆宽带上就是每秒上万次 —— 全推给 `JobManager` 只会把 SSE 与前端刷爆，还会把 `Job.events`
/// 冲掉（1000 条上限）。250 ms 一次（≈4 Hz）足够让进度条看起来是连续的。
pub(super) struct JobDownloadProgress {
    jobs: Arc<JobManager>,
    id: String,
    /// 任务取消令牌：用户点「取消」时别真把 800 MB 下完才停
    cancel: Option<Arc<AtomicBool>>,
    last: Mutex<Instant>,
}

impl JobDownloadProgress {
    pub(super) fn new(
        jobs: Arc<JobManager>,
        id: String,
        cancel: Option<Arc<AtomicBool>>,
    ) -> Self {
        Self {
            jobs,
            id,
            cancel,
            // 从"一秒前"开始：第一块数据立刻报出去，别让用户先盯着空条 250 ms
            last: Mutex::new(Instant::now() - Duration::from_secs(1)),
        }
    }

    /// 换一句文案（每个文件一次，不需要节流）。
    fn note(&self, msg: String) {
        let (jobs, id) = (self.jobs.clone(), self.id.clone());
        if let Ok(handle) = Handle::try_current() {
            let _ = handle.spawn(async move {
                jobs.message(&id, msg).await;
            });
        }
    }
}

impl yekitrim_core::download::DownloadProgress for JobDownloadProgress {
    fn file_started(&self, label: &str) {
        self.note(format!("下载 {label}"));
    }

    fn extracting(&self, label: &str) {
        self.note(format!("解压 {label}"));
    }

    fn bytes(&self, current: u64, total: u64) {
        let Ok(mut last) = self.last.lock() else { return };
        if last.elapsed() < Duration::from_millis(250) {
            return;
        }
        *last = Instant::now();
        drop(last);
        let (jobs, id) = (self.jobs.clone(), self.id.clone());
        if let Ok(handle) = Handle::try_current() {
            let _ = handle.spawn(async move {
                jobs.progress(&id, current, total).await;
            });
        }
    }

    fn cancelled(&self) -> bool {
        self.cancel
            .as_ref()
            .map(|c| c.load(Ordering::SeqCst))
            .unwrap_or(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    // note() 是 trait 方法 —— 测试里直接调它要把 trait 拿进作用域
    use yekitrim_core::progress::ProgressReporter;

    /// `WebProgress` 必须把 `note()` 接到任务框的 message 上（2026-11-14 后端收口，
    /// pipeline-fix 交接）：管线部分剪切失败时 Cutter 发「部分失败 N 条」的汇总，
    /// trait 默认 no-op 的话任务框什么都看不见。**note 不是失败** —— 状态不许被升格。
    #[tokio::test]
    async fn web_progress_note_updates_the_job_message_without_failing_it() {
        let jobs = std::sync::Arc::new(crate::jobs::JobManager::new());
        let handle = jobs
            .create(
                1,
                crate::jobs::JobSpec::new(
                    "stage_cut",
                    "剪切",
                    vec![crate::scope_key::project_out(1)],
                    crate::jobs::JobClass::Heavy,
                ),
            )
            .await;
        let id = handle.id.clone();
        let _ = handle.wait_start().await;

        let progress = WebProgress::new(id.clone(), jobs.clone(), None);
        progress.note("部分失败 2 条（其余 12 条已出片）");
        // note() 是 spawn 出去的异步写入 —— 让出几个调度点等它落地
        for _ in 0..50 {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            if let Some(j) = jobs.get(&id).await {
                if j.message.contains("部分失败 2 条") {
                    break;
                }
            }
        }
        let j = jobs.get(&id).await.expect("任务还在");
        assert!(
            j.message.contains("部分失败 2 条"),
            "note 必须落到任务框 message：{}",
            j.message
        );
        assert_eq!(j.status, "running", "note() 绝不许把任务判成失败：{j:?}");
    }
}
