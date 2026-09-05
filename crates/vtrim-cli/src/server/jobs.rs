//! JobManager：任务状态与 SSE 事件流。
//!
//! v1 采用“单活动任务”模型：同一时间只运行一个长任务，避免 tracing 日志串线。
//! 任务由 HTTP handler 创建，Job 状态保存在内存；历史任务仅保留最近若干条。

use std::collections::VecDeque;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::Serialize;
use tokio::sync::{oneshot, RwLock};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum JobStatus {
    Queued,
    Running,
    AwaitingInput,
    Succeeded,
    Failed,
    Cancelled,
}

impl JobStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Running => "running",
            Self::AwaitingInput => "awaiting_input",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
#[allow(dead_code)]
pub enum JobEvent {
    Log {
        level: String,
        message: String,
    },
    StageStarted {
        stage: String,
        stage_number: u32,
    },
    Progress {
        stage: Option<String>,
        current: u64,
        total: u64,
    },
    StageCompleted {
        stage: String,
    },
    AwaitingResume {
        checkpoint: String,
        question: String,
    },
    Completed,
    Failed {
        error: String,
    },
}

#[derive(Debug, Clone, Serialize)]
pub struct LogLine {
    pub level: String,
    pub message: String,
    pub time: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct JobSummary {
    pub id: String,
    pub kind: String,
    pub status: String,
    pub message: String,
    pub stage: Option<String>,
    pub current: u64,
    pub total: u64,
    pub created_at: String,
    pub logs: Vec<LogLine>,
}

#[derive(Debug)]
pub struct Job {
    pub id: String,
    pub kind: String,
    pub status: JobStatus,
    pub message: String,
    pub stage: Option<String>,
    pub current: u64,
    pub total: u64,
    pub error: Option<String>,
    pub events: Vec<JobEvent>,
    pub logs: VecDeque<LogLine>,
    pub notify: tokio::sync::broadcast::Sender<JobEvent>,
    pub all_tx: tokio::sync::broadcast::Sender<JobEvent>,
    /// 等待断点续传决策时保存 sender；正常任务为 None。
    pub decision_tx: Option<oneshot::Sender<bool>>,
}

impl Job {
    fn new(id: String, kind: String, all_tx: tokio::sync::broadcast::Sender<JobEvent>) -> Self {
        let (tx, _) = tokio::sync::broadcast::channel(256);
        Self {
            id,
            kind,
            status: JobStatus::Queued,
            message: String::new(),
            stage: None,
            current: 0,
            total: 0,
            error: None,
            events: Vec::new(),
            logs: VecDeque::new(),
            notify: tx,
            all_tx,
            decision_tx: None,
        }
    }

    fn push(&mut self, ev: JobEvent, log_level: Option<&str>) {
        if let Some(level) = log_level {
            let msg = match &ev {
                JobEvent::Log { message, .. } => message.clone(),
                JobEvent::StageStarted { stage, .. } => format!("[{}] started", stage),
                JobEvent::Progress { current, total, .. } => {
                    format!("progress {}/{}", current, total)
                }
                JobEvent::StageCompleted { stage } => format!("[{}] completed", stage),
                JobEvent::AwaitingResume { question, .. } => question.clone(),
                JobEvent::Completed => "completed".into(),
                JobEvent::Failed { error } => format!("failed: {}", error),
            };
            self.logs.push_back(LogLine {
                level: level.to_string(),
                message: msg,
                time: now_str(),
            });
            if self.logs.len() > 2000 {
                self.logs.pop_front();
            }
        }
        self.events.push(ev.clone());
        self.events.truncate(1000);
        let _ = self.notify.send(ev.clone());
        let _ = self.all_tx.send(ev);
    }

    fn summary(&self) -> JobSummary {
        JobSummary {
            id: self.id.clone(),
            kind: self.kind.clone(),
            status: self.status.as_str().to_string(),
            message: self.message.clone(),
            stage: self.stage.clone(),
            current: self.current,
            total: self.total,
            created_at: now_str(),
            logs: self.logs.iter().rev().take(200).cloned().collect(),
        }
    }
}

#[derive(Debug)]
pub struct JobManager {
    jobs: RwLock<Vec<Job>>,
    all: tokio::sync::broadcast::Sender<JobEvent>,
}

impl Default for JobManager {
    fn default() -> Self {
        let (all, _) = tokio::sync::broadcast::channel(1024);
        Self { jobs: RwLock::new(Vec::new()), all }
    }
}

impl JobManager {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn create(&self, kind: impl Into<String>) -> String {
        let id = format!("{}-{}", gen_id(), time_id());
        let mut jobs = self.jobs.write().await;
        jobs.push(Job::new(id.clone(), kind.into(), self.all.clone()));
        while jobs.len() > 50 {
            jobs.remove(0);
        }
        id
    }

    pub async fn mark_running(&self, id: &str, message: impl Into<String>) {
        let mut jobs = self.jobs.write().await;
        if let Some(job) = jobs.iter_mut().find(|j| j.id == id) {
            job.status = JobStatus::Running;
            job.message = message.into();
        }
    }

    #[allow(dead_code)]
    pub async fn log(&self, id: &str, level: &str, message: impl Into<String>) {
        let mut jobs = self.jobs.write().await;
        if let Some(job) = jobs.iter_mut().find(|j| j.id == id) {
            job.push(
                JobEvent::Log {
                    level: level.to_string(),
                    message: message.into(),
                },
                Some(level),
            );
        }
    }

    pub async fn stage_started(&self, id: &str, stage: &str, num: u32) {
        let mut jobs = self.jobs.write().await;
        if let Some(job) = jobs.iter_mut().find(|j| j.id == id) {
            job.stage = Some(stage.to_string());
            job.message = format!("{} 运行中", stage);
            job.push(
                JobEvent::StageStarted {
                    stage: stage.to_string(),
                    stage_number: num,
                },
                None,
            );
        }
    }

    pub async fn progress(&self, id: &str, current: u64, total: u64) {
        let mut jobs = self.jobs.write().await;
        if let Some(job) = jobs.iter_mut().find(|j| j.id == id) {
            job.current = current;
            job.total = total;
            job.push(
                JobEvent::Progress {
                    stage: job.stage.clone(),
                    current,
                    total,
                },
                None,
            );
        }
    }

    pub async fn stage_completed(&self, id: &str, stage: &str) {
        let mut jobs = self.jobs.write().await;
        if let Some(job) = jobs.iter_mut().find(|j| j.id == id) {
            job.message = format!("{} 完成", stage);
            job.push(
                JobEvent::StageCompleted {
                    stage: stage.to_string(),
                },
                None,
            );
        }
    }

    #[allow(dead_code)]
    pub async fn awaiting(&self, id: &str, checkpoint: &str, question: &str, tx: oneshot::Sender<bool>) {
        let mut jobs = self.jobs.write().await;
        if let Some(job) = jobs.iter_mut().find(|j| j.id == id) {
            job.status = JobStatus::AwaitingInput;
            job.message = question.to_string();
            job.decision_tx = Some(tx);
            job.push(
                JobEvent::AwaitingResume {
                    checkpoint: checkpoint.to_string(),
                    question: question.to_string(),
                },
                None,
            );
        }
    }

    pub async fn resume(&self, id: &str, decision: bool) -> bool {
        let mut jobs = self.jobs.write().await;
        let Some(job) = jobs.iter_mut().find(|j| j.id == id) else {
            return false;
        };
        if let Some(tx) = job.decision_tx.take() {
            let _ = tx.send(decision);
        }
        job.status = JobStatus::Running;
        job.message = if decision { "继续运行".into() } else { "重新开始".into() };
        true
    }

    pub async fn complete(&self, id: &str) {
        let mut jobs = self.jobs.write().await;
        if let Some(job) = jobs.iter_mut().find(|j| j.id == id) {
            job.status = JobStatus::Succeeded;
            job.message = "完成".into();
            job.decision_tx = None;
            job.push(JobEvent::Completed, None);
        }
    }

    pub async fn fail(&self, id: &str, err: String) {
        let mut jobs = self.jobs.write().await;
        if let Some(job) = jobs.iter_mut().find(|j| j.id == id) {
            job.status = JobStatus::Failed;
            job.error = Some(err.clone());
            job.message = format!("失败: {}", err);
            job.decision_tx = None;
            job.push(JobEvent::Failed { error: err }, None);
        }
    }

    pub async fn cancel(&self, id: &str) -> bool {
        let mut jobs = self.jobs.write().await;
        if let Some(job) = jobs.iter_mut().find(|j| j.id == id) {
            if job.status == JobStatus::Running || job.status == JobStatus::Queued || job.status == JobStatus::AwaitingInput {
                job.status = JobStatus::Cancelled;
                job.message = "已取消".into();
                if let Some(tx) = job.decision_tx.take() {
                    let _ = tx.send(false);
                }
                return true;
            }
        }
        false
    }

    pub async fn get(&self, id: &str) -> Option<JobSummary> {
        let jobs = self.jobs.read().await;
        jobs.iter().find(|j| j.id == id).map(|j| j.summary())
    }

    pub async fn list(&self) -> Vec<JobSummary> {
        let jobs = self.jobs.read().await;
        jobs.iter().rev().map(|j| j.summary()).collect()
    }

    pub async fn subscribe(&self, id: &str) -> Option<tokio::sync::broadcast::Receiver<JobEvent>> {
        let jobs = self.jobs.read().await;
        jobs.iter().find(|j| j.id == id).map(|j| j.notify.subscribe())
    }

    pub async fn subscribe_all(&self) -> tokio::sync::broadcast::Receiver<JobEvent> {
        self.all.subscribe()
    }
}

fn now_str() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_secs();
    format!("{}", secs)
}

fn gen_id() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    format!("{:04x}", N.fetch_add(1, Ordering::Relaxed))
}

fn time_id() -> String {
    chrono::Local::now().format("%H%M%S").to_string()
}
