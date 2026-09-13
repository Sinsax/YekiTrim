//! JobManager：任务状态与 SSE 事件流。
//!
//! **单活动任务模型**：同一时间只允许一个长任务（`create` 在存在活动任务时返回
//! `BusyJob`，调用方转 409），避免并发任务写同一批文件、也让 tracing 日志不串线。
//! 任务由 HTTP handler 创建，Job 状态保存在内存；历史任务仅保留最近若干条（只淘汰
//! 终态任务，活动任务绝不淘汰——否则 `/api/jobs` 与 SSE 订阅会突然失效）。

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::Serialize;
use tokio::sync::RwLock;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum JobStatus {
    Queued,
    Running,
    Succeeded,
    Failed,
    Cancelled,
}

impl JobStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Running => "running",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }
}

/// 已有活动任务，拒绝创建新任务。
#[derive(Debug, Clone)]
pub struct BusyJob {
    pub id: String,
    pub kind: String,
}

impl std::fmt::Display for BusyJob {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "已有任务在运行: {} ({})，请等待其结束或先取消", self.kind, self.id)
    }
}

impl std::error::Error for BusyJob {}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
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
    Completed,
    Failed {
        error: String,
    },
    /// 任务被取消（状态位已置 cancelled）。订阅方需据此结束自己的等待。
    Cancelled,
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
    /// 失败原因（成功/取消时为 None）。前端 `onDone` 会读它。
    pub error: Option<String>,
    pub created_at: String,
    /// 任务归属（多用户：`/api/jobs`、SSE、取消都按它过滤）
    pub owner_id: i64,
    /// 任务属于哪个项目（侧栏进度徽标、按项目过滤任务用；不是所有任务都有）
    pub project: Option<i64>,
    /// 仅在 `/api/jobs/:id` 返回；列表接口为空（避免 50 个任务各带 200 行日志）。
    pub logs: Vec<LogLine>,
}

/// 全局事件流里的一条：带归属，供 `/api/stream` 过滤。
#[derive(Debug, Clone)]
pub struct OwnedEvent {
    pub owner_id: i64,
    pub event: JobEvent,
}

#[derive(Debug)]
pub struct Job {
    pub id: String,
    pub kind: String,
    pub owner_id: i64,
    /// 关联项目（`None` = 平台级任务，如下载模型）
    pub project: Option<i64>,
    pub status: JobStatus,
    pub message: String,
    pub stage: Option<String>,
    pub current: u64,
    pub total: u64,
    pub error: Option<String>,
    pub created_at: String,
    pub events: Vec<JobEvent>,
    pub logs: VecDeque<LogLine>,
    pub notify: tokio::sync::broadcast::Sender<JobEvent>,
    pub all_tx: tokio::sync::broadcast::Sender<OwnedEvent>,
    /// 取消令牌：长任务在阶段/批次边界轮询它。
    pub cancel_flag: Arc<AtomicBool>,
}

impl Job {
    fn new(
        id: String,
        owner_id: i64,
        kind: String,
        all_tx: tokio::sync::broadcast::Sender<OwnedEvent>,
    ) -> Self {
        let (tx, _) = tokio::sync::broadcast::channel(256);
        Self {
            id,
            kind,
            owner_id,
            project: None,
            status: JobStatus::Queued,
            message: String::new(),
            stage: None,
            current: 0,
            total: 0,
            error: None,
            created_at: now_str(),
            events: Vec::new(),
            logs: VecDeque::new(),
            notify: tx,
            all_tx,
            cancel_flag: Arc::new(AtomicBool::new(false)),
        }
    }

    fn is_active(&self) -> bool {
        matches!(self.status, JobStatus::Queued | JobStatus::Running)
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
                JobEvent::Completed => "completed".into(),
                JobEvent::Failed { error } => format!("failed: {}", error),
                JobEvent::Cancelled => "cancelled".into(),
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
        let _ = self.all_tx.send(OwnedEvent { owner_id: self.owner_id, event: ev });
    }

    fn summary(&self, with_logs: bool) -> JobSummary {
        JobSummary {
            id: self.id.clone(),
            kind: self.kind.clone(),
            status: self.status.as_str().to_string(),
            message: self.message.clone(),
            stage: self.stage.clone(),
            current: self.current,
            total: self.total,
            error: self.error.clone(),
            created_at: self.created_at.clone(),
            owner_id: self.owner_id,
            project: self.project,
            logs: if with_logs {
                self.logs.iter().rev().take(50).cloned().collect()
            } else {
                Vec::new()
            },
        }
    }
}

#[derive(Debug)]
pub struct JobManager {
    jobs: RwLock<Vec<Job>>,
    all: tokio::sync::broadcast::Sender<OwnedEvent>,
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

    /// 新建任务（带归属）；已有活动任务时返回 `Err(BusyJob)`。
    pub async fn create(&self, owner_id: i64, kind: impl Into<String>) -> Result<String, BusyJob> {
        let mut jobs = self.jobs.write().await;
        if let Some(busy) = jobs.iter().find(|j| j.is_active()) {
            return Err(BusyJob { id: busy.id.clone(), kind: busy.kind.clone() });
        }
        let id = format!("{}-{}", gen_id(), time_id());
        jobs.push(Job::new(id.clone(), owner_id, kind.into(), self.all.clone()));
        // 只淘汰终态任务
        while jobs.len() > 50 {
            match jobs.iter().position(|j| !j.is_active()) {
                Some(pos) => {
                    jobs.remove(pos);
                }
                None => break,
            }
        }
        Ok(id)
    }

    /// 是否存在活动任务（供 `/polish/preview` 这类同步 handler 判断是否该跑隐式 process）。
    pub async fn has_active(&self) -> bool {
        self.jobs.read().await.iter().any(|j| j.is_active())
    }

    /// 给任务打上项目标签（创建后由 handler 调用；侧栏按项目显示进度用它）。
    pub async fn attach_project(&self, id: &str, project: i64) {
        let mut jobs = self.jobs.write().await;
        if let Some(job) = jobs.iter_mut().find(|j| j.id == id) {
            job.project = Some(project);
        }
    }

    /// 取任务的取消令牌（长任务用它轮询 `ProgressReporter::cancelled`）。
    pub async fn cancel_token(&self, id: &str) -> Option<Arc<AtomicBool>> {
        let jobs = self.jobs.read().await;
        jobs.iter().find(|j| j.id == id).map(|j| j.cancel_flag.clone())
    }

    pub async fn mark_running(&self, id: &str, message: impl Into<String>) {
        let mut jobs = self.jobs.write().await;
        if let Some(job) = jobs.iter_mut().find(|j| j.id == id) {
            job.status = JobStatus::Running;
            job.message = message.into();
        }
    }

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

    pub async fn complete(&self, id: &str) {
        let mut jobs = self.jobs.write().await;
        if let Some(job) = jobs.iter_mut().find(|j| j.id == id) {
            // 已取消的任务不再被迟到的完成回调覆盖为“成功”
            if job.status == JobStatus::Cancelled {
                return;
            }
            job.status = JobStatus::Succeeded;
            job.message = "完成".into();
            job.error = None;
            job.push(JobEvent::Completed, Some("info"));
        }
    }

    pub async fn fail(&self, id: &str, err: String) {
        let mut jobs = self.jobs.write().await;
        if let Some(job) = jobs.iter_mut().find(|j| j.id == id) {
            if job.status == JobStatus::Cancelled {
                return;
            }
            job.status = JobStatus::Failed;
            job.error = Some(err.clone());
            job.message = format!("失败: {}", err);
            job.push(JobEvent::Failed { error: err }, Some("error"));
        }
    }

    /// 取消任务：置取消令牌 + 状态位，并推 `Cancelled` 事件（否则其它订阅者会一直等）。
    /// 真正的工作在下一个阶段/批次边界自行退出。
    pub async fn cancel(&self, id: &str) -> bool {
        let mut jobs = self.jobs.write().await;
        if let Some(job) = jobs.iter_mut().find(|j| j.id == id) {
            if job.is_active() {
                job.cancel_flag.store(true, Ordering::SeqCst);
                job.status = JobStatus::Cancelled;
                job.message = "已取消".into();
                job.push(JobEvent::Cancelled, Some("warn"));
                return true;
            }
        }
        false
    }

    /// 无归属限制的读取（单测与 P5 管理页的全平台视图用；HTTP 侧一律走
    /// [`JobManager::get_owned`] / [`JobManager::list_scoped`]）。
    #[allow(dead_code)]
    pub async fn get(&self, id: &str) -> Option<JobSummary> {
        let jobs = self.jobs.read().await;
        jobs.iter().find(|j| j.id == id).map(|j| j.summary(true))
    }

    /// 按归属取任务：他人任务一律 `None`（上层转 404，不泄露存在性）。
    pub async fn get_owned(&self, id: &str, uid: i64, is_admin: bool) -> Option<JobSummary> {
        let jobs = self.jobs.read().await;
        jobs.iter()
            .find(|j| j.id == id && (is_admin || j.owner_id == uid))
            .map(|j| j.summary(true))
    }

    /// 任务列表（默认只列本人；`is_admin` 且 `all=true` 时列全部）。
    pub async fn list_scoped(&self, uid: i64, is_admin: bool, all: bool) -> Vec<JobSummary> {
        let jobs = self.jobs.read().await;
        jobs.iter()
            .rev()
            .filter(|j| (is_admin && all) || j.owner_id == uid)
            .map(|j| j.summary(false))
            .collect()
    }

    /// 该任务是否属于这个调用者（SSE 订阅与取消都用它）。
    pub async fn owned_by(&self, id: &str, uid: i64, is_admin: bool) -> bool {
        let jobs = self.jobs.read().await;
        jobs.iter()
            .any(|j| j.id == id && (is_admin || j.owner_id == uid))
    }

    #[allow(dead_code)]
    pub async fn list(&self) -> Vec<JobSummary> {
        let jobs = self.jobs.read().await;
        jobs.iter().rev().map(|j| j.summary(false)).collect()
    }

    /// 订阅单个任务：返回 (历史事件, 实时接收端)。
    ///
    /// 先 `subscribe()` 再在同一把读锁下快照历史，二者之间不可能插入新事件
    /// （`push` 需要写锁），因此「订阅前已结束」的快任务也能被回放，不会永久挂起。
    pub async fn subscribe(
        &self,
        id: &str,
    ) -> Option<(Vec<JobEvent>, tokio::sync::broadcast::Receiver<JobEvent>)> {
        let jobs = self.jobs.read().await;
        let job = jobs.iter().find(|j| j.id == id)?;
        let rx = job.notify.subscribe();
        let history = job.events.clone();
        Some((history, rx))
    }

    pub async fn subscribe_all(&self) -> tokio::sync::broadcast::Receiver<OwnedEvent> {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn only_one_active_job_at_a_time() {
        let jm = JobManager::new();
        let id = jm.create(1, "pipeline").await.expect("首个任务应被接受");
        jm.mark_running(&id, "run").await;

        let busy = jm.create(1, "polish_llm").await.expect_err("活动任务期间必须拒绝新任务");
        assert_eq!(busy.kind, "pipeline");
        assert!(busy.to_string().contains(&id));

        jm.complete(&id).await;
        assert!(jm.create(1, "polish_llm").await.is_ok(), "终态后应可再建");
    }

    #[tokio::test]
    async fn history_is_capped_and_keeps_newest() {
        let jm = JobManager::new();
        let mut oldest = String::new();
        for i in 0..55 {
            let id = jm.create(1, "polish_process").await.expect("无活动任务时应可建");
            if i == 0 {
                oldest = id.clone();
            }
            jm.mark_running(&id, "x").await;
            jm.complete(&id).await;
        }
        let list = jm.list().await;
        assert_eq!(list.len(), 50);
        assert!(!list.iter().any(|j| j.id == oldest), "最老的终态任务应被淘汰");
    }

    #[tokio::test]
    async fn created_at_is_stable_and_error_is_reported() {
        let jm = JobManager::new();
        let id = jm.create(1, "cut_one").await.unwrap();
        let first = jm.get(&id).await.unwrap();
        tokio::time::sleep(Duration::from_millis(1100)).await;
        let second = jm.get(&id).await.unwrap();
        assert_eq!(first.created_at, second.created_at, "created_at 不能每次查询都变");

        jm.mark_running(&id, "r").await;
        jm.fail(&id, "boom".into()).await;
        let s = jm.get(&id).await.unwrap();
        assert_eq!(s.status, "failed");
        assert_eq!(s.error.as_deref(), Some("boom"));
        assert!(!s.logs.is_empty(), "/api/jobs/:id 应带日志");
        assert!(jm.list().await[0].logs.is_empty(), "列表接口不应带日志");
    }

    #[tokio::test]
    async fn cancel_sets_token_emits_event_and_is_final() {
        let jm = JobManager::new();
        let id = jm.create(1, "cut_all").await.unwrap();
        jm.mark_running(&id, "r").await;
        let token = jm.cancel_token(&id).await.expect("应有取消令牌");

        assert!(jm.cancel(&id).await);
        assert!(token.load(Ordering::SeqCst), "取消令牌必须置位（管线靠它退出）");
        assert_eq!(jm.get(&id).await.unwrap().status, "cancelled");

        // 迟到的完成回调不能把取消覆盖成成功
        jm.complete(&id).await;
        assert_eq!(jm.get(&id).await.unwrap().status, "cancelled");

        // 订阅历史里必须能看到 Cancelled（否则其它 watcher 会一直等）
        let (history, _rx) = jm.subscribe(&id).await.unwrap();
        assert!(history.iter().any(|e| matches!(e, JobEvent::Cancelled)));
        assert!(!jm.cancel(&id).await, "已取消的任务再次取消应返回 false");
    }
}
