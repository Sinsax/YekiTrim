//! JobManager：任务状态、资源域互斥、三档并发闸门与 SSE 事件流。
//!
//! **资源域互斥**：任务创建时声明自己占用的资源键（见 `scope_key`），与**正在跑**的任务键
//! 相交就进 `Queued` 等（不再拒绝 —— 旧版是"全局单任务"，任何任务活动时新任务直接 409）。
//!
//! **三档并发闸门**（[`JobClass`]）：轻活不占闸门（资源不相交就直接开跑、并行数不限）；
//! 略重、重活各有独立名额，本档名额满了就排队。重活排满**不会**挡住后面的轻活/略重 ——
//! 所以档位豁免的是闸门名额，"写同一批文件"的物理冲突照旧互斥。
//!
//! **优先级**（[`JobPriority`]）只作用于同一档位的队列：系统自动发起的任务排在用户点按的
//! 任务后面，**不抢占**已经在跑的（半路掐断转写等于白跑）。
//!
//! 资源释放（`complete`/`fail`）或新任务入队时都会跑一次调度器。排队中的任务被取消 = 丢掉
//! 开跑信号，等工作协程 `wait_start()` 报错后直接收工（不干活）；正在跑的任务被取消则只置
//! 状态位，资源要等它真正退出（`worker_done`）才算释放 —— 否则"取消渲染后立刻再渲染"
//! 会两个进程同时写一个目录。
//!
//! 任务由 HTTP handler 创建，Job 状态保存在内存；历史任务仅保留最近若干条（只淘汰
//! 终态任务，活动任务绝不淘汰——否则 `/api/jobs` 与 SSE 订阅会突然失效）。

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
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

/// 任务档位：决定它占哪条闸门。
///
/// **按本地资源占用（CPU/GPU/磁盘）分，不按时长分** —— LLM 类任务墙钟很长但全程在等网络，
/// 不该跟渲染抢名额；真吃满 CPU 的活收到更小的闸门里。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum JobClass {
    /// 轻活：不占闸门（资源键不冲突就直接开跑），并行数不限。都是秒级本地小活。
    Light,
    /// 略重：最多 [`MEDIUM_CONCURRENT`] 个并行，满了排队。
    Medium,
    /// 重活：最多 [`HEAVY_CONCURRENT`] 个并行，满了排队。
    Heavy,
}

/// 略重档的并行上限：单条 ffmpeg，以及全部 LLM / 网络类任务（吃网络不吃 CPU）。
pub const MEDIUM_CONCURRENT: usize = 3;

/// 重活档的并行上限：转写、批量剪切、渲染、入库自动管线 —— 这些会抢满 CPU。
///
/// 与 [`MEDIUM_CONCURRENT`] 是**两条独立闸门**：重活排满不会挡住略重/轻活。
/// 两档相加是最坏并发，但重活这一档比旧的单一闸门（3 个不限种类任务）更严，
/// 整体 CPU 压力是下降的。**不是配置项** —— 要调就改这两个常量。
pub const HEAVY_CONCURRENT: usize = 2;

/// 单账号**同时活动**（运行中 + 排队中）的任务数上限。
///
/// 轻活不占档位名额（见 [`JobClass::Light`] 与 `docs/job-queue.md` §3.1）—— 那是有意为之：
/// 编辑器每次改动都会重跑 `polish_process`，让它排队等于编辑卡住。但"一条任务一个 OS 线程"
/// （`api::spawn_job`）配上"不限并发"，在**公网多租户**下就成了"一个账号能开出无界个线程"：
/// `docs/job-queue.md` §4 那句"前台任务都是人点出来的，频率有天然上限"对脚本化的客户端不成立。
///
/// 所以这里按**账号**限制活动任务总数（而不是给轻活加闸门名额 —— 那会连编辑器的高频
/// 自动重跑一起挡住）。正常人手点远够用，脚本刷就到此为止。
pub const MAX_ACTIVE_PER_OWNER: usize = 12;

/// 全服同时活动的任务数上限（所有账号合计）。
pub const MAX_ACTIVE_TOTAL: usize = 64;

/// 单账号**建任务频率**上限（每分钟）。
///
/// 比"活动任务数"更早生效的那一道：活动数够不到上限时，疯狂 create/complete 同样能
/// 把调度器与日志刷废。正常人一分钟点不出 60 个任务（编辑器自动重跑的 `polish_process`
/// 有 1.5s 合并防抖，上限约 40/分钟），脚本可以每秒几千个。
pub const MAX_CREATE_PER_MINUTE: u32 = 60;

impl JobClass {
    /// 全部档位。`slot()` 的下标**就是**它在这里的位置 —— 加档位时跟着改 `slot()`。
    pub const ALL: [JobClass; 3] = [Self::Light, Self::Medium, Self::Heavy];

    /// 档位数（调度器那张按档计数的表）。
    const COUNT: usize = Self::ALL.len();

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Light => "light",
            Self::Medium => "medium",
            Self::Heavy => "heavy",
        }
    }

    /// 该档的并行上限（`None` = 不限）。
    pub fn cap(&self) -> Option<usize> {
        match self {
            Self::Light => None,
            Self::Medium => Some(MEDIUM_CONCURRENT),
            Self::Heavy => Some(HEAVY_CONCURRENT),
        }
    }

    /// 名额计数用的槽位（调度器里那张 `[usize; COUNT]` 的下标，必须与 `ALL` 同序）。
    fn slot(self) -> usize {
        match self {
            Self::Light => 0,
            Self::Medium => 1,
            Self::Heavy => 2,
        }
    }

    /// 管线 / 入库自动管线按阶段列表分档：含转写、剪切或全流程 = 重活，其余（提取/检测）算略重。
    pub fn for_pipeline(stages: &[String]) -> Self {
        let has = |name: &str| stages.iter().any(|s| s == name);
        if has("transcribe") || has("process") || has("cut") {
            Self::Heavy
        } else {
            Self::Medium
        }
    }

    /// 精修任务按 `kind` 分档（`init` / `llm` / `process` / `render` / `cover`）。
    ///
    /// `init` 只建目录抽资源、`process` 是编辑器每次改动后自动重跑的后处理、`cover` 出一张图 ——
    /// 都短，不该排在闸门后面等几分钟；它们彼此之间仍由片段资源键互斥。
    pub fn for_polish(kind: &str) -> Self {
        match kind {
            "render" => Self::Heavy,
            // 单段事件重做 = 一次 LLM 调用 + 后处理，与整片 llm 同档
            "llm" | "llm_segment" => Self::Medium,
            _ => Self::Light,
        }
    }
}

/// 任务来源：同一档位内，前台插到后台前面（**不抢占**已经在跑的任务）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum JobPriority {
    /// 用户点按发起的任务（默认）。
    Foreground,
    /// 系统自动发起的任务：入库自动管线、模型下载、贴纸构建。
    Background,
}

impl JobPriority {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Foreground => "foreground",
            Self::Background => "background",
        }
    }

    /// 调度扫描的排序键（小的先跑）。同键由稳定排序保持创建顺序（FIFO）。
    fn order(self) -> u8 {
        match self {
            Self::Foreground => 0,
            Self::Background => 1,
        }
    }
}

/// 创建任务的入参：`kind` + 文案 + 资源键 + 档位 + 优先级。
///
/// 档位与优先级都得在**创建时**定下来（调度器立刻要用它们放行），所以不做"创建后再设"的 setter。
#[derive(Debug, Clone)]
pub struct JobSpec {
    pub kind: String,
    pub message: String,
    pub resources: Vec<String>,
    pub class: JobClass,
    pub priority: JobPriority,
}

impl JobSpec {
    pub fn new(
        kind: impl Into<String>,
        message: impl Into<String>,
        resources: Vec<String>,
        class: JobClass,
    ) -> Self {
        Self {
            kind: kind.into(),
            message: message.into(),
            resources,
            class,
            priority: JobPriority::Foreground,
        }
    }

    /// 标成后台任务：同一档位内排在用户点按的任务后面。
    pub fn background(mut self) -> Self {
        self.priority = JobPriority::Background;
        self
    }
}

/// 调度器自愈间隔。
pub const SCHED_TICK: Duration = Duration::from_secs(5);

/// 排队任务被挡住多久之后告警（同时是告警的限流间隔：最多这么久一条）。
pub const BLOCKED_WARN: Duration = Duration::from_secs(120);

/// 某个资源域已被占用（同步接口没有排队能力，只能当场拒绝）。
#[derive(Debug, Clone)]
pub struct BusyJob {
    pub id: String,
    pub kind: String,
    /// 冲突的资源键（如 `p3:clip:clip-07`），给调用方拼更具体的文案用。
    pub resource: String,
}

impl std::fmt::Display for BusyJob {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} 正在运行（{}），请等它结束或先取消", self.kind, self.id)
    }
}

impl std::error::Error for BusyJob {}

/// 资源键是否相交（**相等匹配**，不做前缀）。
pub fn resources_conflict(a: &[String], b: &[String]) -> bool {
    a.iter().any(|x| b.iter().any(|y| x == y))
}

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
    /// 最后一次事件的时间（epoch 秒，**数字**，前端直接相减算"多久没动静了"）。
    ///
    /// 与 `created_at`（字符串）不同型是刻意的：这个值就是拿来算差值的 ——
    /// 运行中的任务 N 分钟没有任何事件，通常意味着底下有个进程挂住了。
    pub updated_at: u64,
    /// 任务归属（多用户：`/api/jobs`、SSE、取消都按它过滤）
    pub owner_id: i64,
    /// 任务属于哪个项目（侧栏进度徽标、按项目过滤任务用；不是所有任务都有）
    pub project: Option<i64>,
    /// 占用的资源键（前端按作用域门控按钮：`p{pid}:clip:clip-07` 等，见 `scope_key`）
    pub resources: Vec<String>,
    /// 任务档位（`light` / `medium` / `heavy`）。
    pub class: String,
    /// 该档的并行名额上限；`null` = 不限（轻活）。
    pub class_cap: Option<usize>,
    /// 同档队列里的名次（1 起，`0` = 没在排队）。
    ///
    /// 是"同档里排第几"，不是"还要等几个人" —— 卡在资源键冲突上的任务也照算名次。
    pub queue_position: u32,
    /// 任务来源（`foreground` 用户点按 / `background` 系统自动）。
    pub priority: String,
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
    /// 创建时刻的 epoch 秒（`created_at` 的数值版，调度告警算等待时长用）。
    pub(crate) created_secs: u64,
    /// 最后一次事件时刻的 epoch 秒（`summary.updated_at` 的来源）。
    pub(crate) updated_secs: u64,
    pub events: Vec<JobEvent>,
    pub logs: VecDeque<LogLine>,
    pub notify: tokio::sync::broadcast::Sender<JobEvent>,
    pub all_tx: tokio::sync::broadcast::Sender<OwnedEvent>,
    /// 取消令牌：长任务在阶段/批次边界轮询它。
    pub cancel_flag: Arc<AtomicBool>,
    /// 占用的资源键（域不相交的任务可并行；见 [`JobManager::create`]）。
    pub resources: Vec<String>,
    /// 档位（决定它占哪条闸门，见 [`JobClass`]）。
    pub class: JobClass,
    /// 来源（决定同档内的排队次序，见 [`JobPriority`]）。
    pub priority: JobPriority,
    /// 排队任务的开跑信号；发车（或终态）后置为 `None`。
    start: Option<tokio::sync::oneshot::Sender<()>>,
    /// 工作协程是否已退出。取消后状态位立刻变 `Cancelled`，但**资源要等协程退出**
    /// 才释放 —— 否则"取消渲染后立刻再渲染"会两个进程同时写一个目录。
    worker_done: bool,
}

impl Job {
    fn new(
        id: String,
        owner_id: i64,
        spec: JobSpec,
        all_tx: tokio::sync::broadcast::Sender<OwnedEvent>,
    ) -> Self {
        let (tx, _) = tokio::sync::broadcast::channel(256);
        Self {
            id,
            kind: spec.kind,
            owner_id,
            project: None,
            status: JobStatus::Queued,
            message: spec.message,
            stage: None,
            current: 0,
            total: 0,
            error: None,
            created_at: now_str(),
            created_secs: epoch_secs(),
            updated_secs: epoch_secs(),
            events: Vec::new(),
            logs: VecDeque::new(),
            notify: tx,
            all_tx,
            cancel_flag: Arc::new(AtomicBool::new(false)),
            resources: spec.resources,
            class: spec.class,
            priority: spec.priority,
            start: None,
            worker_done: false,
        }
    }

    fn is_active(&self) -> bool {
        matches!(self.status, JobStatus::Queued | JobStatus::Running)
    }

    /// 是否还占着资源：Running，或已取消但协程尚未退出。
    fn holds_resources(&self) -> bool {
        match self.status {
            JobStatus::Running => true,
            JobStatus::Cancelled => !self.worker_done,
            _ => false,
        }
    }

    /// 能不能被历史淘汰：已终态**且**不再占任何资源。
    ///
    /// 单看 `is_active()` 会漏掉"已取消但工作协程还没退出"这一类 —— 它不是 active，
    /// 却仍占着资源键与档位名额。把它淘汰掉有两重后果：资源键不再被计入（两个任务可能
    /// 同时写一个目录），而且它之后真正的 `complete`/`fail` 会找不到这个 Job 直接 `return`
    /// —— **没人再触发调度器**，后面排队的任务就停在原地等下一次 `create`。
    fn evictable(&self) -> bool {
        !self.is_active() && !self.holds_resources()
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
        self.updated_secs = epoch_secs();
        self.bridge_tracing(&ev);
        let _ = self.notify.send(ev.clone());
        let _ = self.all_tx.send(OwnedEvent { owner_id: self.owner_id, event: ev });
    }

    /// 任务日志同时进服务端日志（控制台 / 设置页「运行日志」/ 日志文件）。
    ///
    /// 以前这些消息只躺在内存里等前端来读，而前端**从来不读** —— 出错后除了页面上
    /// 一句"失败"什么都没有。桥接后渲染失败的最后 20 行、`FINISHED:` 产物路径等
    /// 都能在日志里查到。
    ///
    /// **不桥接 `Progress`**：1% 一条的高频事件会把日志冲掉；阶段起止是低频的，
    /// 桥接后正好能看出"跑到哪一步断的"。
    fn bridge_tracing(&self, ev: &JobEvent) {
        let (level, text): (&str, String) = match ev {
            JobEvent::Log { level, message } => (level.as_str(), message.clone()),
            JobEvent::StageStarted { stage, .. } => ("info", format!("阶段开始: {stage}")),
            JobEvent::StageCompleted { stage } => ("info", format!("阶段完成: {stage}")),
            JobEvent::Completed => ("info", "任务完成".into()),
            JobEvent::Failed { error } => ("error", format!("任务失败: {error}")),
            JobEvent::Cancelled => ("warn", "任务已取消".into()),
            JobEvent::Progress { .. } => return,
        };
        let job = self.id.as_str();
        let kind = self.kind.as_str();
        match level {
            "error" => tracing::error!(target: "yekitrim_cli::job", job = %job, kind = %kind, "{}", text),
            "warn" => tracing::warn!(target: "yekitrim_cli::job", job = %job, kind = %kind, "{}", text),
            "debug" => tracing::debug!(target: "yekitrim_cli::job", job = %job, kind = %kind, "{}", text),
            _ => tracing::info!(target: "yekitrim_cli::job", job = %job, kind = %kind, "{}", text),
        }
    }

    fn summary(&self, with_logs: bool, queue_position: u32) -> JobSummary {
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
            updated_at: self.updated_secs,
            owner_id: self.owner_id,
            project: self.project,
            resources: self.resources.clone(),
            class: self.class.as_str().to_string(),
            class_cap: self.class.cap(),
            queue_position,
            priority: self.priority.as_str().to_string(),
            logs: if with_logs {
                self.logs.iter().rev().take(50).cloned().collect()
            } else {
                Vec::new()
            },
        }
    }
}

/// 任务句柄：`id` 立刻可用（前端拿它订阅进度），真正的开工要等 `wait_start()`。
#[derive(Debug)]
pub struct JobHandle {
    pub id: String,
    /// 创建时就被放行 = `false`；排队等资源 = `true`（前端据此提示"已排队"）。
    pub queued: bool,
    start: tokio::sync::oneshot::Receiver<()>,
}

impl JobHandle {
    /// 等工作协程被放行。`Err` = 排队期间被取消，调用方应**直接收工**（别干活）。
    pub async fn wait_start(self) -> Result<(), ()> {
        self.start.await.map_err(|_| ())
    }
}

#[derive(Debug)]
pub struct JobManager {
    jobs: RwLock<Vec<Job>>,
    all: tokio::sync::broadcast::Sender<OwnedEvent>,
    /// 上次"有任务被挡太久"告警的时刻（epoch 秒）—— 只用来限流，别让 [`JobManager::tick`] 刷屏。
    last_blocked_warn: AtomicU64,
    /// 建任务频率（每账号每分钟，见 [`MAX_CREATE_PER_MINUTE`]）。
    create_rate: super::ratelimit::RateLimiter,
}

impl Default for JobManager {
    fn default() -> Self {
        let (all, _) = tokio::sync::broadcast::channel(1024);
        Self {
            jobs: RwLock::new(Vec::new()),
            all,
            last_blocked_warn: AtomicU64::new(0),
            create_rate: super::ratelimit::RateLimiter::new(MAX_CREATE_PER_MINUTE, 60),
        }
    }
}

impl JobManager {
    pub fn new() -> Self {
        Self::default()
    }

    /// 历史任务裁剪：只淘汰"已终态且不再占资源"的，最多留 50 条。
    ///
    /// ⚠️ 活动任务与**仍占着资源的已取消任务**绝不淘汰 —— 否则 `/api/jobs`、SSE 订阅与
    /// 调度器的释放回调都会突然失效，见 [`Job::evictable`]。名单里**没有**可淘汰项时直接
    /// 停止（不能无限循环）。
    fn trim_history(jobs: &mut Vec<Job>) {
        while jobs.len() > 50 {
            match jobs.iter().position(|j| j.evictable()) {
                Some(pos) => {
                    jobs.remove(pos);
                }
                None => break,
            }
        }
    }

    /// 建任务配额（每账号频率 + 活动任务数，全服活动数）。超了返回给用户看的原因。
    ///
    /// 这些都是**公网多租户**才需要的护栏：单机自用时它们永远碰不到（人点不出这个频率）。
    fn quota_exceeded(&self, jobs: &[Job], owner_id: i64) -> Option<String> {
        if let Err(wait) = self.create_rate.check(&owner_id.to_string()) {
            return Some(format!(
                "建任务过于频繁（每分钟上限 {MAX_CREATE_PER_MINUTE} 个），请 {wait} 秒后再试"
            ));
        }
        let mine = jobs
            .iter()
            .filter(|j| j.owner_id == owner_id && j.is_active())
            .count();
        if mine >= MAX_ACTIVE_PER_OWNER {
            return Some(format!(
                "你的活动任务过多（上限 {MAX_ACTIVE_PER_OWNER} 个），等前面的任务跑完再试"
            ));
        }
        let total = jobs.iter().filter(|j| j.is_active()).count();
        if total >= MAX_ACTIVE_TOTAL {
            return Some(format!("服务器活动任务过多（上限 {MAX_ACTIVE_TOTAL} 个），请稍后再试"));
        }
        None
    }

    /// 新建任务（带归属、资源键、档位与优先级），冲突/名额满时**排队**而不是拒绝。
    ///
    /// 立即放行的条件：本档名额没满（轻活不限）且资源键与所有"还占着资源"的任务不相交。
    /// 否则进 `Queued`，等 `complete`/`fail` 释放资源后由调度器按 `(优先级, 创建顺序)` 放行。
    ///
    /// ⚠️ 唯一的**拒绝**路径是 [`JobManager::quota_exceeded`]（建任务频率 / 活动任务数超限，
    /// 见那几个常量）：超限的任务记为失败并**丢掉开跑信号** —— 工作协程 `wait_start()` 拿到
    /// `Err` 后直接收工。任务仍然留在列表里，前端一眼能看到"为什么没跑"，而不是点了没反应。
    pub async fn create(&self, owner_id: i64, spec: JobSpec) -> JobHandle {
        let mut jobs = self.jobs.write().await;
        let id = format!("{}-{}", gen_id(), time_id());
        let (tx, rx) = tokio::sync::oneshot::channel();
        let mut job = Job::new(id.clone(), owner_id, spec, self.all.clone());
        job.start = Some(tx);
        if let Some(why) = self.quota_exceeded(&jobs, owner_id) {
            job.status = JobStatus::Failed;
            job.error = Some(why.clone());
            job.message = why.clone();
            job.start = None; // 丢掉发送端 → 协程的 wait_start() 得到 Err → 不干活
            job.push(JobEvent::Failed { error: why }, Some("error"));
            jobs.push(job);
            Self::trim_history(&mut jobs);
            return JobHandle { id, queued: false, start: rx };
        }
        jobs.push(job);
        // 只淘汰"已终态且不再占资源"的任务（活动任务与**仍占着资源的已取消任务**绝不淘汰
        // —— 否则 `/api/jobs`、SSE 订阅与调度器的释放回调都会突然失效，见 `evictable`）
        Self::trim_history(&mut jobs);
        Self::schedule_locked(&mut jobs);
        let queued = jobs
            .iter()
            .find(|j| j.id == id)
            .map(|j| j.status != JobStatus::Running)
            .unwrap_or(false);
        JobHandle { id, queued, start: rx }
    }

    /// 调度器：按 `(优先级, 创建顺序)` 放行排队任务。
    ///
    /// 每个档位有**独立名额**（[`JobClass::cap`]）：名额满了只挡住本档后面的任务，
    /// 别的档照常放行 —— 所以撞上限是 `continue` 而不是 `break`。同档 FIFO 由
    /// "名额满则本档后续全都过不了这一关"自然保持；跨档的先后（前台/后台）由扫描顺序定。
    fn schedule_locked(jobs: &mut [Job]) {
        let mut used = [0usize; JobClass::COUNT];
        let mut held: Vec<Vec<String>> = Vec::new();
        for job in jobs.iter() {
            if !job.holds_resources() {
                continue;
            }
            used[job.class.slot()] += 1;
            held.push(job.resources.clone());
        }
        // `sort_by_key` 是稳定排序：同优先级保持创建顺序（FIFO）
        let mut order: Vec<usize> = (0..jobs.len())
            .filter(|&i| jobs[i].status == JobStatus::Queued)
            .collect();
        order.sort_by_key(|&i| jobs[i].priority.order());

        for i in order {
            let class = jobs[i].class;
            if let Some(cap) = class.cap() {
                if used[class.slot()] >= cap {
                    continue;
                }
            }
            if held.iter().any(|r| resources_conflict(r, &jobs[i].resources)) {
                continue;
            }
            jobs[i].status = JobStatus::Running;
            used[class.slot()] += 1;
            held.push(jobs[i].resources.clone());
            if let Some(tx) = jobs[i].start.take() {
                let _ = tx.send(());
            }
        }
    }

    /// 每个排队任务在**自己档位队列**里的名次（1 起），按 `(优先级, 创建顺序)` 算。
    ///
    /// 没在排队的（在跑 / 已结束）为 0。
    fn queue_positions(jobs: &[Job]) -> Vec<u32> {
        let mut positions = vec![0u32; jobs.len()];
        for slot in 0..JobClass::COUNT {
            let mut idx: Vec<usize> = (0..jobs.len())
                .filter(|&i| jobs[i].status == JobStatus::Queued && jobs[i].class.slot() == slot)
                .collect();
            idx.sort_by_key(|&i| jobs[i].priority.order());
            for (n, i) in idx.into_iter().enumerate() {
                positions[i] = n as u32 + 1;
            }
        }
        positions
    }

    /// 调度器**自愈**：低频重跑一次 `schedule_locked`，并点名"谁把谁挡住太久"。
    ///
    /// 调度是事件驱动的（`create` / `complete` / `fail` 各跑一次），正常情况下够用。
    /// 但只要有一个工作协程**永远不返回**（外部进程挂住、网络调用没有超时），它就会一直占着
    /// 资源键与档位名额，而**没有任何事件**会再来触发调度 —— 后面的任务无声地卡在 `Queued`，
    /// 界面上只看到一个停住的任务、查不出是谁挡的。这个 tick 不解决"那个任务为什么不停"，
    /// 它保证**其它任务不会因此永远不动**，并把"卡住"这件事明确写进日志。
    ///
    /// 由 `server::run` 起一个定时任务调用（见 `spawn_housekeeping`）。
    pub async fn tick(&self) {
        let mut jobs = self.jobs.write().await;
        Self::schedule_locked(&mut jobs);
        // 限流：tick 是 5 秒一次，不限流会把日志刷满
        let now = epoch_secs();
        if now.saturating_sub(self.last_blocked_warn.load(Ordering::Relaxed)) < BLOCKED_WARN.as_secs() {
            return;
        }
        let lines = Self::blocked_report(&jobs, now);
        if !lines.is_empty() {
            tracing::warn!(
                target: "yekitrim_cli::job",
                "有 {} 个任务排队超过 {}s 未开始 —— {}",
                lines.len(),
                BLOCKED_WARN.as_secs(),
                lines.join("；")
            );
            self.last_blocked_warn.store(now, Ordering::Relaxed);
        }
    }

    /// 排队超过 [`BLOCKED_WARN`] 的任务：返回"谁被谁挡住"的说明行（空 = 没有）。
    ///
    /// 纯函数（不读锁、不记日志），便于单测。找阻塞者时**先按资源冲突找**（那才是真的挡着），
    /// 找不到再退回"本档名额被占满"—— 两者的处置完全不同，文案要能区分开。
    fn blocked_report(jobs: &[Job], now: u64) -> Vec<String> {
        let wait = BLOCKED_WARN.as_secs();
        let held: Vec<&Job> = jobs.iter().filter(|j| j.holds_resources()).collect();
        let mut lines = Vec::new();
        for q in jobs
            .iter()
            .filter(|j| j.status == JobStatus::Queued && now.saturating_sub(j.created_secs) >= wait)
        {
            let conflicting = held.iter().find(|h| resources_conflict(&h.resources, &q.resources));
            let (b, reason) = match conflicting {
                Some(b) => (b, "占用同一批文件"),
                None => match held.iter().find(|h| h.class == q.class) {
                    Some(b) => (b, "占满本档名额"),
                    None => continue,
                },
            };
            lines.push(format!(
                "{}（{}）被 {}（{}）挡住 {}s：{}",
                q.id,
                q.kind,
                b.id,
                b.kind,
                now.saturating_sub(b.created_secs),
                reason
            ));
        }
        lines
    }

    /// 这些资源键现在被谁占着？（同步接口没法排队，只能 409 —— 由调用方决定文案）
    ///
    /// 排队中的任务也算占用：它迟早会写那批文件，现在放行同步写就是等着对撞。
    pub async fn busy_with(&self, resources: &[String]) -> Option<BusyJob> {
        let jobs = self.jobs.read().await;
        jobs.iter()
            .find(|j| (j.is_active() || !j.worker_done) && resources_conflict(&j.resources, resources))
            .map(|j| BusyJob {
                id: j.id.clone(),
                kind: j.kind.clone(),
                resource: j
                    .resources
                    .iter()
                    .find(|r| resources.contains(r))
                    .cloned()
                    .unwrap_or_default(),
            })
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

    /// 只改任务提示语（"正在下载 xxx"这类**阶段内的细粒度文案**）。
    ///
    /// 不推事件：进度另有 `progress()`（高频），这里换文案的频率是"每个文件一次"。
    /// 但**要记一次活动时间** —— 否则一份 800 MB 的包下完、解压那一两分钟里
    /// `updated_at` 不动，界面会误报"已 N 分钟没有新进展"。
    pub async fn message(&self, id: &str, msg: impl Into<String>) {
        let mut jobs = self.jobs.write().await;
        if let Some(job) = jobs.iter_mut().find(|j| j.id == id) {
            job.message = msg.into();
            job.updated_secs = epoch_secs();
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
        let Some(job) = jobs.iter_mut().find(|j| j.id == id) else {
            return;
        };
        // ⚠️ **只有"还没终态"的任务才能被标成成功**（Queued = 放行前就收工那条路）。
        //
        // 两条不能覆盖的终态：
        // ① `Cancelled`：迟到的完成回调不许把它翻成成功；
        // ② `Failed`：`create()` 的**超配额拒绝**分支写的就是 Failed（活动任务过量 / 建任务过频），
        //    而 9 个创建点的 `wait_start()` 失败分支都会调 `complete()`（本意只是"释放占位并唤醒
        //    后面的任务"，见 `api/polish.rs` 那处注释）。覆盖它 ⇒ 用户看到任务"完成"、错误被抹掉，
        //    实际一个字节都没产出（2026-11-14 后端 review 的 P1）。
        //
        // 无论是否迁状态，`worker_done` 都必须置位：拒单/取消的任务也一样要**真正释放资源键**，
        // 否则同项目/同片段的后续任务会永远排队。
        if matches!(job.status, JobStatus::Queued | JobStatus::Running) {
            job.status = JobStatus::Succeeded;
            job.message = "完成".into();
            job.error = None;
            job.push(JobEvent::Completed, Some("info"));
        }
        job.worker_done = true;
        job.start = None;
        Self::schedule_locked(&mut jobs);
    }

    pub async fn fail(&self, id: &str, err: String) {
        let mut jobs = self.jobs.write().await;
        let Some(job) = jobs.iter_mut().find(|j| j.id == id) else {
            return;
        };
        if job.status != JobStatus::Cancelled {
            job.status = JobStatus::Failed;
            job.error = Some(err.clone());
            job.message = format!("失败: {}", err);
            job.push(JobEvent::Failed { error: err }, Some("error"));
        }
        job.worker_done = true;
        job.start = None;
        Self::schedule_locked(&mut jobs);
    }

    /// 取消任务：置取消令牌 + 状态位，并推 `Cancelled` 事件（否则其它订阅者会一直等）。
    /// 真正的工作在下一个阶段/批次边界自行退出；**释放资源要等它退出**（见 `worker_done`）。
    pub async fn cancel(&self, id: &str) -> bool {
        let mut jobs = self.jobs.write().await;
        let Some(job) = jobs.iter_mut().find(|j| j.id == id) else {
            return false;
        };
        if !job.is_active() {
            return false;
        }
        job.cancel_flag.store(true, Ordering::SeqCst);
        job.status = JobStatus::Cancelled;
        job.message = "已取消".into();
        // 丢掉开跑信号：排队中的任务不会开工（协程 wait_start 报错后直接收工）
        job.start = None;
        job.push(JobEvent::Cancelled, Some("warn"));
        true
    }

    /// 无归属限制的读取（单测与 P5 管理页的全平台视图用；HTTP 侧一律走
    /// [`JobManager::get_owned`] / [`JobManager::list_scoped`]）。
    #[allow(dead_code)]
    pub async fn get(&self, id: &str) -> Option<JobSummary> {
        let jobs = self.jobs.read().await;
        let positions = Self::queue_positions(&jobs);
        jobs.iter()
            .enumerate()
            .find(|(_, j)| j.id == id)
            .map(|(i, j)| j.summary(true, positions[i]))
    }

    /// 按归属取任务：他人任务一律 `None`（上层转 404，不泄露存在性）。
    pub async fn get_owned(&self, id: &str, uid: i64, is_admin: bool) -> Option<JobSummary> {
        let jobs = self.jobs.read().await;
        let positions = Self::queue_positions(&jobs);
        jobs.iter()
            .enumerate()
            .find(|(_, j)| j.id == id && (is_admin || j.owner_id == uid))
            .map(|(i, j)| j.summary(true, positions[i]))
    }

    /// 任务列表（默认只列本人；`is_admin` 且 `all=true` 时列全部）。
    pub async fn list_scoped(&self, uid: i64, is_admin: bool, all: bool) -> Vec<JobSummary> {
        let jobs = self.jobs.read().await;
        let positions = Self::queue_positions(&jobs);
        jobs.iter()
            .enumerate()
            .rev()
            .filter(|(_, j)| (is_admin && all) || j.owner_id == uid)
            .map(|(i, j)| j.summary(false, positions[i]))
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
        let positions = Self::queue_positions(&jobs);
        jobs.iter()
            .enumerate()
            .rev()
            .map(|(i, j)| j.summary(false, positions[i]))
            .collect()
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

/// 起一个后台定时任务跑 [`JobManager::tick`]（由 `server::run` 调用一次）。
///
/// 只在服务端起：单测里的 `JobManager` 不该凭空多出一个定时器。
pub fn spawn_housekeeping(jobs: Arc<JobManager>) {
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(SCHED_TICK);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            ticker.tick().await;
            jobs.tick().await;
        }
    });
}

fn epoch_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_secs()
}

fn now_str() -> String {
    format!("{}", epoch_secs())
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

    fn keys(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    /// 建任务的简写：kind 与文案同值 —— 这些测试只关心档位与资源键。
    async fn mk(jm: &JobManager, resources: Vec<String>, class: JobClass) -> JobHandle {
        jm.create(1, JobSpec::new("test", "test", resources, class)).await
    }

    /// 下载任务的"正在下 xxx"文案：只换文字、不动进度与状态，
    /// 但**要**刷活动时间 —— 否则解压那一两分钟会被界面误报成"卡住了"。
    #[tokio::test]
    async fn message_only_touches_text_and_activity() {
        let jm = JobManager::new();
        let h = mk(&jm, keys(&["models"]), JobClass::Medium).await;
        let before = jm.get(&h.id).await.unwrap();
        jm.message(&h.id, "下载 qwen3-asr").await;
        let after = jm.get(&h.id).await.unwrap();
        assert_eq!(after.message, "下载 qwen3-asr");
        assert_eq!(after.status, before.status);
        assert_eq!(after.current, 0, "改文案不该动进度");
        assert!(after.updated_at >= before.updated_at);
    }

    #[tokio::test]
    async fn disjoint_resources_run_in_parallel() {
        let jm = JobManager::new();
        let a = mk(&jm, keys(&["p1:clip:clip-01"]), JobClass::Heavy).await;
        let b = mk(&jm, keys(&["p1:clip:clip-02"]), JobClass::Heavy).await;
        assert!(!a.queued, "资源不冲突应立刻开跑");
        assert!(!b.queued);
        let (ida, idb) = (a.id.clone(), b.id.clone());
        assert!(a.wait_start().await.is_ok());
        assert!(b.wait_start().await.is_ok());
        assert_eq!(jm.get(&ida).await.unwrap().status, "running");
        assert_eq!(jm.get(&idb).await.unwrap().status, "running");
    }

    /// 活动任务数上限：单账号开到上限后再 create 会被**拒绝**（记成失败 + 不开工）。
    ///
    /// 这道护栏只对**公网多租户**有意义：轻活不占档位名额是有意为之（编辑器每次改动都要
    /// 重跑 `polish_process`），但"不限并发"配上"每条任务一个 OS 线程"就是无界线程 ——
    /// 所以按账号给活动任务总数封顶，而不是给轻活加闸门名额（那会连编辑器一起挡住）。
    #[tokio::test]
    async fn active_jobs_are_capped_per_owner() {
        let jm = JobManager::new();
        // 资源键各不相同 → 互不冲突；轻活不占闸门 → 全部立刻开跑
        for i in 0..MAX_ACTIVE_PER_OWNER {
            let h = jm
                .create(1, JobSpec::new("t", "t", keys(&[&format!("r{i}:out")]), JobClass::Light))
                .await;
            assert!(!h.queued, "第 {i} 个还没到上限，应立刻开跑");
        }
        // 第 N+1 个：任务在列表里可见、状态是失败、开跑信号是 Err（协程收工不干活）
        let over = jm
            .create(1, JobSpec::new("t", "t", keys(&["over:out"]), JobClass::Light))
            .await;
        let id = over.id.clone();
        assert!(over.wait_start().await.is_err(), "超限任务不该开工");
        let s = jm.get(&id).await.unwrap();
        assert_eq!(s.status, "failed");
        assert!(
            s.error.clone().unwrap_or_default().contains("活动任务过多"),
            "错误文案应说明原因：{s:?}"
        );

        // 额度是按账号算的：另一个账号不受影响
        let other = jm
            .create(2, JobSpec::new("t", "t", keys(&["o:out"]), JobClass::Light))
            .await;
        assert!(other.wait_start().await.is_ok(), "别的账号应照常开工");
    }

    /// **超配额被拒的任务不许被后续的 `complete()` 刷成"完成"**（2026-11-14 后端 review 的 P1）。
    ///
    /// 生产路径是这样的：`create()` 记 Failed + 丢掉开跑信号 → 协程 `wait_start()` 拿到 `Err`
    /// → **每个创建点都会调 `jobs.complete()`**（本意只是释放占位并唤醒后面的任务，见
    /// `api/polish.rs` / `pipeline.rs` 的失败分支）。曾经的 `complete()` 只排除 `Cancelled`，
    /// 于是这条失败被覆盖成 `Succeeded`、`error` 被清空 —— 用户看到"完成"，实际一步没跑。
    ///
    /// 三条判据一起钉：① 状态仍是 failed；② 失败原因还在；③ 资源键**真的释放**了
    /// （拒单的任务也必须让出资源，否则同项目后续任务永远排队）。
    #[tokio::test]
    async fn quota_rejected_job_is_not_overwritten_by_complete() {
        let jm = JobManager::new();
        let mut running = Vec::new();
        for i in 0..MAX_ACTIVE_PER_OWNER {
            let h = jm
                .create(1, JobSpec::new("t", "t", keys(&[&format!("r{i}:out")]), JobClass::Light))
                .await;
            running.push(h.id.clone());
            assert!(h.wait_start().await.is_ok(), "第 {i} 个还没到上限，应立刻开跑");
        }
        // 第 N+1 个：撞活动任务上限 ⇒ Failed
        let over = jm
            .create(
                1,
                JobSpec::new("cut_one", "剪出片段 #9", keys(&["p3:clip:clip-09"]), JobClass::Medium),
            )
            .await;
        let over_id = over.id.clone();
        assert!(over.wait_start().await.is_err(), "超限任务不该开工");
        // 复刻创建点的失败分支
        jm.complete(&over_id).await;
        let s = jm.get(&over_id).await.expect("任务仍在列表里");
        assert_eq!(s.status, "failed", "超配额的任务不许被刷成完成：{s:?}");
        assert!(
            s.error.as_deref().unwrap_or("").contains("活动任务过多"),
            "失败原因不许被抹掉：{s:?}"
        );
        assert!(
            jm.busy_with(&keys(&["p3:clip:clip-09"])).await.is_none(),
            "拒单的任务也必须让出资源键"
        );

        // ---- 正常路径与取消路径照旧 ----
        // 放掉一个名额，新任务开跑 → complete() 仍然把 Running 迁到 Succeeded
        jm.complete(&running[0]).await;
        let ok = mk(&jm, keys(&["ok:out"]), JobClass::Light).await;
        let ok_id = ok.id.clone();
        assert!(ok.wait_start().await.is_ok());
        assert_eq!(jm.get(&ok_id).await.unwrap().status, "running");
        jm.complete(&ok_id).await;
        let done = jm.get(&ok_id).await.unwrap();
        assert_eq!(done.status, "succeeded", "Running → Succeeded 这条必须不变");
        assert!(done.error.is_none());

        // 取消过的任务：迟到的 complete() 不许把它翻成成功
        let canc = mk(&jm, keys(&["c:out"]), JobClass::Light).await;
        let canc_id = canc.id.clone();
        assert!(jm.cancel(&canc_id).await);
        jm.complete(&canc_id).await;
        assert_eq!(jm.get(&canc_id).await.unwrap().status, "cancelled");
    }

    /// 建任务频率上限：每个任务立刻完成（活动数永远不是瓶颈），只剩频率这一道。
    #[tokio::test]
    async fn job_creation_is_rate_limited() {
        let jm = JobManager::new();
        for i in 0..MAX_CREATE_PER_MINUTE {
            let h = jm.create(7, JobSpec::new("t", "t", Vec::new(), JobClass::Light)).await;
            let id = h.id.clone();
            assert!(h.wait_start().await.is_ok(), "第 {i} 个不该被限流");
            jm.complete(&id).await;
        }
        let over = jm.create(7, JobSpec::new("t", "t", Vec::new(), JobClass::Light)).await;
        let id = over.id.clone();
        assert!(over.wait_start().await.is_err(), "超过每分钟额度应拒绝");
        let s = jm.get(&id).await.unwrap();
        assert_eq!(s.status, "failed");
        assert!(
            s.error.clone().unwrap_or_default().contains("频繁"),
            "错误文案应说明原因：{s:?}"
        );
    }

    #[tokio::test]
    async fn same_resource_queues_then_runs_after_release() {
        let jm = JobManager::new();
        let a = mk(&jm, keys(&["p1:clip:clip-07"]), JobClass::Medium).await;
        let b = mk(&jm, keys(&["p1:clip:clip-07"]), JobClass::Medium).await;
        assert!(!a.queued);
        assert!(b.queued, "同一片段的第二个任务必须排队而不是报错");
        assert_eq!(jm.get(&b.id).await.unwrap().status, "queued");

        jm.complete(&a.id).await;
        let idb = b.id.clone();
        assert!(b.wait_start().await.is_ok(), "前一个结束应自动放行");
        assert_eq!(jm.get(&idb).await.unwrap().status, "running");
    }

    #[tokio::test]
    async fn full_cut_coexists_with_polish_but_not_with_itself() {
        let jm = JobManager::new();
        // 全切只占 out + idx：与精修（clip 键）不冲突
        let cut = mk(&jm, keys(&["p3:out", "p3:idx"]), JobClass::Heavy).await;
        let p = mk(&jm, keys(&["p3:clip:clip-02"]), JobClass::Heavy).await;
        assert!(!cut.queued && !p.queued, "全切与精修可并行");
        // 但两个剪切共写 publish_config.json / clips_order.txt → 排队
        // （这里故意用轻活：档位豁免的只是闸门名额，**资源冲突照旧要排**）
        let cut2 = mk(&jm, keys(&["p3:out", "p3:idx", "p3:clip:clip-09"]), JobClass::Light).await;
        assert!(cut2.queued, "撞上在跑任务的资源键，轻活也得排队");
    }

    #[tokio::test]
    async fn heavy_slots_do_not_block_lighter_classes() {
        let jm = JobManager::new();
        for i in 0..HEAVY_CONCURRENT {
            let h = mk(&jm, keys(&[&format!("h{i}:out")]), JobClass::Heavy).await;
            assert!(!h.queued);
        }
        let mut medium = Vec::new();
        for i in 0..MEDIUM_CONCURRENT {
            let h = mk(&jm, keys(&[&format!("m{i}:out")]), JobClass::Medium).await;
            assert!(!h.queued, "重活占满后，略重仍应立刻开跑（两条独立闸门）");
            medium.push(h.id);
        }
        // 两档名额都满了，轻活照样开跑 —— 它不占任何闸门
        let light = mk(&jm, keys(&["l:out"]), JobClass::Light).await;
        assert!(!light.queued, "轻活任何时候都不该因为闸门排队");

        let over = mk(&jm, keys(&["over:out"]), JobClass::Medium).await;
        assert!(over.queued, "略重名额满了才排队");
        let id_over = over.id.clone();
        jm.complete(&medium[0]).await;
        assert!(over.wait_start().await.is_ok(), "放掉一个名额应自动放行");
        assert_eq!(jm.get(&id_over).await.unwrap().status, "running");
    }

    #[tokio::test]
    async fn heavy_cap_queues_in_fifo_order() {
        let jm = JobManager::new();
        let mut ids = Vec::new();
        for i in 0..HEAVY_CONCURRENT {
            let h = mk(&jm, keys(&[&format!("p{i}:out")]), JobClass::Heavy).await;
            assert!(!h.queued);
            ids.push(h.id);
        }
        let over = mk(&jm, keys(&["p9:out"]), JobClass::Heavy).await;
        assert!(over.queued, "超过重活上限应排队");
        // 放掉一个名额，排队的那个应自动开跑
        jm.complete(&ids[0]).await;
        let id_over = over.id.clone();
        assert!(over.wait_start().await.is_ok());
        assert_eq!(jm.get(&id_over).await.unwrap().status, "running");
    }

    #[tokio::test]
    async fn foreground_jumps_ahead_of_background_in_the_same_class() {
        let jm = JobManager::new();
        let mut ids = Vec::new();
        for i in 0..MEDIUM_CONCURRENT {
            let h = mk(&jm, keys(&[&format!("m{i}:out")]), JobClass::Medium).await;
            ids.push(h.id);
        }
        let bg = jm
            .create(1, JobSpec::new("bg", "bg", keys(&["bg:out"]), JobClass::Medium).background())
            .await;
        let fg = mk(&jm, keys(&["fg:out"]), JobClass::Medium).await;
        assert!(bg.queued && fg.queued, "名额满了，两个都得排");

        jm.complete(&ids[0]).await;
        let (bg_id, fg_id) = (bg.id.clone(), fg.id.clone());
        assert!(fg.wait_start().await.is_ok(), "后入队的前台任务应插到后台前面");
        assert_eq!(jm.get(&fg_id).await.unwrap().status, "running");
        assert_eq!(jm.get(&bg_id).await.unwrap().status, "queued", "后台仍该在后面等");
    }

    #[tokio::test]
    async fn queue_position_counts_within_its_own_class() {
        let jm = JobManager::new();
        for i in 0..HEAVY_CONCURRENT {
            mk(&jm, keys(&[&format!("h{i}:out")]), JobClass::Heavy).await;
        }
        let h1 = mk(&jm, keys(&["hx:out"]), JobClass::Heavy).await;
        let running = mk(&jm, keys(&["l:out"]), JobClass::Light).await;
        let h2 = mk(&jm, keys(&["hy:out"]), JobClass::Heavy).await;

        assert_eq!(jm.get(&h1.id).await.unwrap().queue_position, 1, "重活队里第一个");
        assert_eq!(jm.get(&h2.id).await.unwrap().queue_position, 2, "重活队里第二个");
        assert_eq!(jm.get(&running.id).await.unwrap().queue_position, 0, "在跑的任务没有名次");
    }

    #[tokio::test]
    async fn queued_job_cancelled_before_start_never_runs() {
        let jm = JobManager::new();
        let a = mk(&jm, keys(&["p1:clip:clip-01"]), JobClass::Medium).await;
        let b = mk(&jm, keys(&["p1:clip:clip-01"]), JobClass::Medium).await;
        assert!(b.queued);
        let idb = b.id.clone();
        assert!(jm.cancel(&idb).await);
        assert!(b.wait_start().await.is_err(), "排队中被取消：协程应收到 Err 后收工");
        jm.complete(&a.id).await;
        assert_eq!(jm.get(&idb).await.unwrap().status, "cancelled");
    }

    #[tokio::test]
    async fn cancelled_running_job_keeps_its_resources_until_worker_exits() {
        let jm = JobManager::new();
        let a = mk(&jm, keys(&["p1:clip:clip-01"]), JobClass::Medium).await;
        assert!(jm.cancel(&a.id).await);
        // 取消瞬间状态位已终态，但渲染进程还在收尾 —— 同资源任务不能立刻开跑
        let b = mk(&jm, keys(&["p1:clip:clip-01"]), JobClass::Medium).await;
        assert!(b.queued, "取消 ≠ 资源已释放，必须等协程退出");
        let idb = b.id.clone();
        jm.fail(&a.id, "任务已取消".into()).await;
        assert!(b.wait_start().await.is_ok());
        assert_eq!(jm.get(&idb).await.unwrap().status, "running");
    }

    #[tokio::test]
    async fn busy_with_reports_holder_for_sync_handlers() {
        let jm = JobManager::new();
        assert!(jm.busy_with(&keys(&["p1:clip:clip-01"])).await.is_none());
        let a = mk(&jm, keys(&["p1:clip:clip-01"]), JobClass::Medium).await;
        let busy = jm.busy_with(&keys(&["p1:clip:clip-01"])).await.expect("应报占用");
        assert_eq!(busy.kind, "test");
        assert_eq!(busy.resource, "p1:clip:clip-01");
        assert!(busy.to_string().contains(&a.id));
        assert!(jm.busy_with(&keys(&["p1:clip:clip-02"])).await.is_none());
        jm.complete(&a.id).await;
        assert!(jm.busy_with(&keys(&["p1:clip:clip-01"])).await.is_none());
    }

    #[tokio::test]
    async fn history_is_capped_and_keeps_newest() {
        let jm = JobManager::new();
        let mut oldest = String::new();
        for i in 0..55 {
            let h = mk(&jm, keys(&[&format!("r{i}")]), JobClass::Light).await;
            if i == 0 {
                oldest = h.id.clone();
            }
            jm.complete(&h.id).await;
        }
        let list = jm.list().await;
        assert_eq!(list.len(), 50);
        assert!(!list.iter().any(|j| j.id == oldest), "最老的终态任务应被淘汰");
    }

    #[tokio::test]
    async fn created_at_is_stable_and_error_is_reported() {
        let jm = JobManager::new();
        let id = jm
            .create(1, JobSpec::new("cut_one", "剪切事件 #1", keys(&["p1:out"]), JobClass::Medium))
            .await
            .id;
        let first = jm.get(&id).await.unwrap();
        tokio::time::sleep(Duration::from_millis(1100)).await;
        let second = jm.get(&id).await.unwrap();
        assert_eq!(first.created_at, second.created_at, "created_at 不能每次查询都变");

        jm.fail(&id, "boom".into()).await;
        let s = jm.get(&id).await.unwrap();
        assert_eq!(s.status, "failed");
        assert_eq!(s.error.as_deref(), Some("boom"));
        assert!(!s.logs.is_empty(), "/api/jobs/:id 应带日志");
        assert!(jm.list().await[0].logs.is_empty(), "列表接口不应带日志");
        assert_eq!(s.resources, keys(&["p1:out"]), "资源键要回给前端做作用域门控");
        assert_eq!(s.class, "medium", "档位要回给前端展示");
        assert_eq!(s.class_cap, Some(MEDIUM_CONCURRENT));
        assert_eq!(s.priority, "foreground");
        assert!(s.updated_at > 0, "updated_at 要回给前端算'多久没进展'");
    }

    #[tokio::test]
    async fn cancel_sets_token_emits_event_and_is_final() {
        let jm = JobManager::new();
        let id = jm
            .create(1, JobSpec::new("cut_all", "一键全切", keys(&["p1:out", "p1:idx"]), JobClass::Heavy))
            .await
            .id;
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

    /// 取消 ≠ 资源已释放：仍占资源的已取消任务**不能被历史淘汰**。
    ///
    /// 它既不是 `is_active`（会走到 `else { return }`），又仍然占着资源键与档位名额 ——
    /// 被淘汰掉就有两重后果：资源键不再被计入（两个任务可能同时写一个目录），
    /// 而且它之后真正的 `complete`/`fail` 找不到这个 Job、**没人再触发调度器**，
    /// 排在后面的任务就停在原地等下一次 `create`。
    #[tokio::test]
    async fn cancelled_job_holding_resources_is_never_evicted() {
        let jm = JobManager::new();
        let a = mk(&jm, keys(&["p1:out"]), JobClass::Medium).await;
        assert!(jm.cancel(&a.id).await, "取消后状态位是终态，但协程还没退出");

        // 灌满历史：淘汰一定要跳过它，只吃别的已终态任务
        for i in 0..60 {
            let h = mk(&jm, keys(&[&format!("r{i}")]), JobClass::Light).await;
            jm.complete(&h.id).await;
        }

        assert!(jm.list().await.len() <= 50, "历史仍要压在上限内");
        let s = jm.get(&a.id).await.expect("仍占资源的已取消任务不该被淘汰");
        assert_eq!(s.status, "cancelled");
    }

    /// 自愈 tick 不该误放行：资源还在别人手里时，排队的就是得排队。
    #[tokio::test]
    async fn tick_does_not_promote_while_resources_are_held() {
        let jm = JobManager::new();
        let a = mk(&jm, keys(&["p1:out"]), JobClass::Medium).await;
        let b = mk(&jm, keys(&["p1:out"]), JobClass::Medium).await;
        assert!(b.queued);

        jm.tick().await;
        assert_eq!(jm.get(&b.id).await.unwrap().status, "queued", "资源没释放就不该开跑");

        jm.complete(&a.id).await;
        assert_eq!(jm.get(&b.id).await.unwrap().status, "running");
    }

    /// 卡住告警要**点名**：谁被谁挡住、因为什么 —— 否则界面上只看到一个不动了的任务。
    #[tokio::test]
    async fn blocked_report_names_the_holder_and_the_reason() {
        let jm = JobManager::new();
        let holder = jm
            .create(1, JobSpec::new("holder", "x", keys(&["p1:out"]), JobClass::Medium))
            .await;
        let waiter = jm
            .create(1, JobSpec::new("waiter", "x", keys(&["p1:out"]), JobClass::Medium))
            .await;
        assert!(waiter.queued);

        // 把创建时间往前挪十分钟，模拟"挡了很久"
        let old = epoch_secs() - 600;
        {
            let mut jobs = jm.jobs.write().await;
            for j in jobs.iter_mut() {
                j.created_secs = old;
            }
        }
        let lines = JobManager::blocked_report(&jm.jobs.read().await, epoch_secs());
        assert_eq!(lines.len(), 1, "{lines:?}");
        assert!(lines[0].contains(&waiter.id), "{lines:?}");
        assert!(lines[0].contains("holder"), "要点出是谁挡着: {lines:?}");
        assert!(lines[0].contains("占用同一批文件"), "{lines:?}");
        assert!(jm.get(&holder.id).await.is_some());
    }

    /// 名额满与文件冲突是两种不同的排队原因，文案必须分得开（处置方式完全不同）。
    #[tokio::test]
    async fn blocked_report_distinguishes_slot_from_file_conflict() {
        let jm = JobManager::new();
        for i in 0..HEAVY_CONCURRENT {
            jm.create(1, JobSpec::new("holder", "x", keys(&[&format!("h{i}:out")]), JobClass::Heavy))
                .await;
        }
        // 资源键与谁都不同 → 只能是"名额满了"才排队
        let waiter = jm
            .create(1, JobSpec::new("waiter", "x", keys(&["free:out"]), JobClass::Heavy))
            .await;
        assert!(waiter.queued);

        let old = epoch_secs() - 600;
        {
            let mut jobs = jm.jobs.write().await;
            for j in jobs.iter_mut() {
                j.created_secs = old;
            }
        }
        let lines = JobManager::blocked_report(&jm.jobs.read().await, epoch_secs());
        assert_eq!(lines.len(), 1, "{lines:?}");
        assert!(lines[0].contains("占满本档名额"), "{lines:?}");
        assert!(!lines[0].contains("同一批文件"), "{lines:?}");
    }

    #[test]
    fn pipeline_and_polish_kinds_are_classified() {        // 管线/入库：含转写、剪切或全流程 = 重活
        for stages in [
            vec!["transcribe"],
            vec!["extract", "transcribe", "detect", "cut"],
            vec!["cut"],
            vec!["process"],
        ] {
            let stages = keys(&stages);
            assert_eq!(JobClass::for_pipeline(&stages), JobClass::Heavy, "{stages:?}");
        }
        // 只剩提取/检测 = 略重
        for stages in [vec!["extract"], vec!["detect"], vec!["extract", "detect"]] {
            let stages = keys(&stages);
            assert_eq!(JobClass::for_pipeline(&stages), JobClass::Medium, "{stages:?}");
        }
        // 精修：渲染重活、LLM 略重、init/process/cover 轻活
        assert_eq!(JobClass::for_polish("render"), JobClass::Heavy);
        assert_eq!(JobClass::for_polish("llm"), JobClass::Medium);
        for kind in ["init", "process", "cover"] {
            assert_eq!(JobClass::for_polish(kind), JobClass::Light, "{kind} 该是轻活");
        }
        // 名额：轻活不限，其余与常量对齐
        assert_eq!(JobClass::Light.cap(), None);
        assert_eq!(JobClass::Medium.cap(), Some(MEDIUM_CONCURRENT));
        assert_eq!(JobClass::Heavy.cap(), Some(HEAVY_CONCURRENT));
        // 调度的按档计数表是按 `slot()` 当下标索引的：顺序错了就会算错名额
        for (i, c) in JobClass::ALL.iter().enumerate() {
            assert_eq!(c.slot(), i, "{c:?} 的 slot 与 ALL 的顺序不一致");
        }
    }
}
