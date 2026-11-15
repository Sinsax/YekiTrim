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
        Arc::new(WebProgress::new(job_id, jobs, cancel, expected_stages(&stages)));
    yekitrim_service::run_stages(
        &config_file,
        &stages,
        dry_run,
        resume,
        Some(progress),
    ).await
}

/// 本任务将**按序**运行的阶段号 —— spec §1 的 stage_index/stage_total 来源。
/// 与 `yekitrim_service::run_stages_with_ext` 的分派同口径（2026-11-15 修正后）：
/// 含 "process" → 全流程 1..4；**其余按请求列表逐个映射**（阶段子集不再误当全流程，
/// 旧行为把多阶段一律当 run_process，勾选「识别+查找事件」会连剪出一起跑）。
/// 未知键给 0：桥落 (1, 0)，前端对 total=0 不显示 i/N（宁可少显示也不撒谎）。
fn expected_stages(stages: &[String]) -> Vec<u32> {
    if stages.iter().any(|s| s == "process") {
        return vec![1, 2, 3, 4];
    }
    stages
        .iter()
        .map(|s| match s.as_str() {
            "extract" => 1,
            "transcribe" => 2,
            "detect" => 3,
            "cut" => 4,
            _ => 0,
        })
        .collect()
}

/// 进度桥的一次落库动作：桥（管线线程）→ 通道 → 主 runtime 上的消费者。
enum JobUpdate {
    Stage { stage: String, num: u32, index: u32, total: u32 },
    Progress { current: u64, total: u64 },
    StageDone { stage: String },
    Note(String),
    Fail(String),
}

/// 任务进度桥：把 `ProgressReporter` 的 `stage_*` 调用映射成 Job 事件。
///
/// ⚠️ **事件经通道在主 runtime 上落库，不在调用线程上 spawn**（2026-11-15 结构修，
/// 「准备中」悬案的根因）：管线体跑在专用线程的 current-thread runtime 上，
/// 提取音频等整段同步代码会把"当前"runtime 阻塞几分钟 —— 旧实现 spawn 出去的
/// 落库任务被饿死，任务消息停在创建文案、`updated_at` 冻结，120s 卡住提示误报。
/// 现在桥只往 unbounded 通道里投递（同步、永不阻塞），消费者由 [`JobManager`]
/// 存的主 runtime 句柄 spawn —— 管线线程阻塞多久都不影响事件生效。
///
/// 管线之外的异步任务（如表情包的 LLM 标签构建）也走这一份 —— 进度只该有一套桥。
/// `expected` = 本任务将**按序**运行的阶段号（spec §1；空 = 非管线任务，不显示 i/N）。
pub(crate) fn progress_bridge(
    job_id: String,
    jobs: Arc<JobManager>,
    cancel: Option<Arc<std::sync::atomic::AtomicBool>>,
    expected: Vec<u32>,
) -> Arc<dyn yekitrim_core::progress::ProgressReporter> {
    Arc::new(WebProgress::new(job_id, jobs, cancel, expected))
}

/// 阶段在本任务里的序位：`expected` = 将按序运行的阶段号清单。
/// 找不到（管线外用户 / 未知键）→ (1, 0)：前端对 total=0 不显示 i/N。
fn stage_pos(expected: &[u32], num: u32) -> (u32, u32) {
    match expected.iter().position(|&n| n == num) {
        Some(at) => (at as u32 + 1, expected.len() as u32),
        None => (1, 0),
    }
}

/// 阶段中文名 —— **唯一映射**（前端不再自己翻英文类名）。
/// 桥也被管线之外的任务复用（表情包标签构建，名字本身已是中文）—— 未知键原样回显，
/// 宁可露出原名也不许吞掉阶段名。测试钉住 `stage_num` 认识的键全覆盖。
fn stage_zh(name: &str) -> &str {
    match name {
        "AudioExtractor" => "提取音频",
        "Transcriber" => "识别声音",
        "EventDetector" => "查找事件",
        "Cutter" => "剪出片段",
        other => other,
    }
}

pub(super) struct WebProgress {
    /// 任务的取消令牌：管线在阶段边界轮询它。
    cancel: Option<Arc<std::sync::atomic::AtomicBool>>,
    /// 通往主 runtime 消费者的投递口（unbounded —— 桥侧永不阻塞；节奏由任务体节流）。
    /// `id`/`jobs` 不存字段：它们只在构造期用来起消费者，之后桥只剩投递职责。
    tx: tokio::sync::mpsc::UnboundedSender<JobUpdate>,
    /// 本任务将按序运行的阶段号（spec §1 的 stage_index/stage_total 来源）。
    expected: Vec<u32>,
}

impl WebProgress {
    fn new(
        id: String,
        jobs: Arc<JobManager>,
        cancel: Option<Arc<std::sync::atomic::AtomicBool>>,
        expected: Vec<u32>,
    ) -> Self {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<JobUpdate>();
        // 消费者跑在**主 runtime**（JobManager 存的就是它）：桥侧只投递，永不阻塞。
        let (cid, cjobs) = (id.clone(), jobs.clone());
        jobs.runtime_handle().spawn(async move {
            while let Some(update) = rx.recv().await {
                // 任务已终态就别再往上写：竞态下迟到的事件不许覆盖终态文案
                // （get 返回 Summary，status 是小写字符串 —— 与前端同口径）
                if let Some(job) = cjobs.get(&cid).await {
                    if job.status != "queued" && job.status != "running" {
                        break;
                    }
                }
                match update {
                    JobUpdate::Stage { stage, num, index, total } => {
                        cjobs
                            .stage_started(&cid, &stage, num, stage_zh(&stage), index, total)
                            .await;
                    }
                    JobUpdate::Progress { current, total } => {
                        cjobs.progress(&cid, current, total).await;
                    }
                    JobUpdate::StageDone { stage } => {
                        cjobs.stage_completed(&cid, &stage).await;
                    }
                    JobUpdate::Note(msg) => {
                        cjobs.message(&cid, msg).await;
                    }
                    JobUpdate::Fail(msg) => {
                        cjobs.fail(&cid, msg).await;
                        break;
                    }
                }
            }
        });
        Self { cancel, tx, expected }
    }
}

impl yekitrim_core::progress::ProgressReporter for WebProgress {
    fn cancelled(&self) -> bool {
        self.cancel
            .as_ref()
            .map(|c| c.load(std::sync::atomic::Ordering::SeqCst))
            .unwrap_or(false)
    }
    fn stage_started(&self, name: &str) {
        let num = stage_num(name);
        let (index, total) = stage_pos(&self.expected, num);
        let _ = self.tx.send(JobUpdate::Stage {
            stage: name.to_string(),
            num,
            index,
            total,
        });
    }
    fn stage_progress(&self, current: u64, total: u64) {
        let _ = self.tx.send(JobUpdate::Progress { current, total });
    }
    fn stage_completed(&self, name: &str) {
        let _ = self.tx.send(JobUpdate::StageDone { stage: name.to_string() });
    }
    /// `error` = 整条任务 fail（core trait 的口径）。经通道投递，消费者落库后退出 ——
    /// 竞态下随后迟到的进度事件不会再覆盖失败原因。
    fn error(&self, msg: &str) {
        let _ = self.tx.send(JobUpdate::Fail(msg.to_string()));
    }
    /// `note()` = 任务框的一句提示，**不是**失败（trait 默认 no-op，见 core 的
    /// `ProgressReporter`）。管线侧的部分剪切失败/跳过汇总走这里 —— 不接线的话
    /// 「部分失败 N 条」只进日志、任务框什么都看不见。转成 `jobs.message`（一行文案），
    /// **绝不升格成 fail**（部分失败 ≠ 任务失败，口径见 core 的 note 注释）。
    fn note(&self, msg: &str) {
        let _ = self.tx.send(JobUpdate::Note(msg.to_string()));
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

        let progress = WebProgress::new(id.clone(), jobs.clone(), None, Vec::new());
        progress.note("部分失败 2 条（其余 12 条已出片）");
        // note() 经通道由主 runtime 消费者落库 —— 让出几个调度点等它落地
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

    /// **验收判据 spec §3-1**：管线线程整段阻塞期间，阶段事件必须已经落库。
    ///
    /// 旧实现把落库 spawn 到"当前"runtime —— 提取音频等同步段把 runtime 阻塞几分钟，
    /// 事件被饿死（任务消息停在「准备中」、updated_at 冻结、120s 卡住提示误报）。
    /// 通道版桥下，事件投进 unbounded 通道，消费者跑在 JobManager 存的主 runtime
    /// 句柄上 —— 调用线程阻塞多久都不影响。变异自证：把 `tx.send` 换回
    /// `Handle::try_current().spawn` 本测试即红（调用线程 sleep 期间消费者饿死）。
    #[tokio::test(flavor = "multi_thread")]
    async fn stage_event_lands_while_pipeline_thread_blocks() {
        let jobs = std::sync::Arc::new(crate::jobs::JobManager::new());
        let handle = jobs
            .create(
                1,
                crate::jobs::JobSpec::new(
                    "stage_process",
                    "准备中",
                    vec![crate::scope_key::project_out(1)],
                    crate::jobs::JobClass::Heavy,
                ),
            )
            .await;
        let id = handle.id.clone();
        let _ = handle.wait_start().await;

        let progress = WebProgress::new(id.clone(), jobs.clone(), None, vec![1, 2, 3, 4]);
        // 模拟管线线程的同步段：阶段开始后调用线程阻塞 600ms（不 yield）
        progress.stage_started("AudioExtractor");
        std::thread::sleep(std::time::Duration::from_millis(600));

        let j = jobs.get(&id).await.expect("任务还在");
        assert_eq!(
            j.stage.as_deref(),
            Some("AudioExtractor"),
            "阻塞期间阶段事件必须已落库（稳定键）"
        );
        assert_eq!(j.stage_zh.as_deref(), Some("提取音频"), "阶段中文名必须就位");
        assert_eq!(j.stage_index, Some(1), "一键处理的第一阶段");
        assert_eq!(j.stage_total, Some(4), "一键处理共四阶段");
        assert_eq!(j.message, "", "阶段开始必须重置 message（明细语义，不回显状态）");
        assert_eq!(j.status, "running");
    }

    /// 阶段中文名是**唯一映射**：`stage_num` 认识的键必须有中文（缺一个就是英文类名
    /// 漏到界面上）；桥的管线外用户（表情包标签等，名字已是中文）原样回显。
    #[test]
    fn stage_zh_covers_every_known_stage() {
        for (name, num) in [
            ("AudioExtractor", 1u32),
            ("Transcriber", 2),
            ("EventDetector", 3),
            ("Cutter", 4),
        ] {
            assert_eq!(stage_num(name), num, "stage_num 回归钉子：{name}");
            assert_ne!(stage_zh(name), name, "stage_num 认识的键必须有中文：{name}");
        }
        assert_eq!(stage_zh("表情包标签"), "表情包标签", "管线外用户原样回显");
        assert_eq!(stage_zh("UnknownThing"), "UnknownThing", "未知键原样回显");
    }

    /// 序位规则：expected = 将按序运行的阶段号；找不到 → (1, 0)（前端对 total=0
    /// 不显示 i/N —— 宁可少显示也不撒谎）。
    #[test]
    fn stage_pos_rules() {
        assert_eq!(stage_pos(&[1, 2, 3, 4], 1), (1, 4));
        assert_eq!(stage_pos(&[1, 2, 3, 4], 4), (4, 4));
        assert_eq!(stage_pos(&[3], 3), (1, 1), "单阶段任务：阶段 1/1");
        assert_eq!(stage_pos(&[], 1), (1, 0), "非管线桥用户：不显示 i/N");
        assert_eq!(stage_pos(&[1, 2, 3, 4], 0), (1, 0), "未知键不显示 i/N");
    }

    /// expected_stages 与 `yekitrim_service::run_stages_with_ext` 的分派同口径：
    /// 单阶段（非 process）只跑那一个；process / 多阶段 = 全流程。
    #[test]
    fn expected_stages_matches_service_dispatch() {
        assert_eq!(expected_stages(&["process".to_string()]), vec![1, 2, 3, 4]);
        assert_eq!(expected_stages(&["extract".to_string()]), vec![1]);
        assert_eq!(expected_stages(&["transcribe".to_string()]), vec![2]);
        assert_eq!(expected_stages(&["detect".to_string()]), vec![3]);
        assert_eq!(expected_stages(&["cut".to_string()]), vec![4]);
        assert_eq!(
            expected_stages(&["transcribe".to_string(), "detect".to_string()]),
            vec![2, 3],
            "阶段子集按请求列表映射 —— 2026-11-15 分派修正后多阶段不再误当全流程"
        );
        assert_eq!(
            expected_stages(&["extract".to_string(), "transcribe".to_string(), "detect".to_string()]),
            vec![1, 2, 3],
            "步进区默认勾选（前三阶段）= 阶段 1/3 → 3/3"
        );
        assert_eq!(expected_stages(&["mystery".to_string()]), vec![0], "未知键给 0 → 不显示 i/N");
    }
}
