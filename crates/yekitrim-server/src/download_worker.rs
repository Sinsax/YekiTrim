//! B站下载 worker（P4）：把一个 `kind='bilibili'` 的会话变成项目。
//!
//! ## 与录播姬 worker 的关系：**并列，不互调**
//!
//! 两个 worker 结构对称但语义完全不同，共享部分已经下沉/上提（计划 §2）：
//!
//! | 步骤 | 录播姬 | B站 |
//! |---|---|---|
//! | 容量预检 | `yekitrim_service::ingest::capacity`（共用） | 同左 |
//! | 建项目登记 | `yekitrim_service::ingest::landing`（共用） | 同左 |
//! | 起管线 | `crate::pipeline_boot`（共用） | 同左 |
//! | 触发 | webhook（**无身份**，进待分配池） | 用户主动提交（**有身份**，直接归他） |
//! | 完结 | 等 `SessionEnded` + 空闲窗口猜 | **子进程退出码就是答案** |
//! | 文件 | 从录播目录**搬入**（跨盘要拷 + 校验） | 下载器**直接写进托管目录** |
//!
//! 所以本文件里**没有**：`is_finalized` / 待分配池 / `move_file` / 多分片合并。
//! 这些是录播姬的语义，抄过来只会让 B站通道平白无故多出一堆状态。
//! §2.4 的判据：本文件不许出现 `ingest_worker`。
//!
//! ## 唯一开子进程的地方
//!
//! `downloader` 适配层是纯函数（构造 argv / 解析一行），**spawn 在这里**。
//! 三条硬约束（都由实测得出，见 `p4-prerequisite-verification.md`）：
//! 1. `.env_remove` 清代理变量（本机代理会把 yt-dlp 的出网打挂）；
//! 2. `kill_on_drop(true)` + **无输出超时** —— 否则挂死的进程会永久占住下载槽；
//! 3. 进度与最终路径**都在 stdout**，只能靠哨兵前缀分流。
//!
//! ## 会话状态
//!
//! `queued` →（本 worker 置）`downloading` →（[`landing`] 置）`finalized` / `failed`。
//! `failed` 不会自动重试：用户在记录页点"重试"（会打回 `queued`）。

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Context, Result};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::sync::mpsc;

use futures_util::FutureExt;

use yekitrim_service::bilibili;
use yekitrim_service::db::IngestSessionRow;
use yekitrim_service::downloader::registry;
use yekitrim_service::downloader::{
    self, CommandSpec, DownloadErrorKind, DownloadRequest, Downloader, DownloaderKind, StreamEvent,
};
use yekitrim_service::ingest::{capacity, landing};
use yekitrim_service::projects;

use crate::jobs::{JobClass, JobSpec};
use crate::model::AppState;
use crate::pipeline_boot::{spawn_pipeline, Pipeline};

/// 会话状态：等待下载。
pub const STATE_QUEUED: &str = "queued";
/// 会话状态：下载中（本 worker 持有）。
pub const STATE_DOWNLOADING: &str = "downloading";

type Notify = mpsc::Sender<Option<String>>;

fn channel() -> &'static OnceLock<Notify> {
    static TX: OnceLock<Notify> = OnceLock::new();
    &TX
}

/// 进程内"事实表"：哪些会话的下载任务**真的**在跑。
///
/// 为什么不直接信 `ingest_sessions.state`：那个是 db 里的**意图**，进程崩溃/任务 panic 后
/// 会停在 `downloading` 骗人。这张表由 [`ActiveGuard`] 保证"任务活着一行在，死了必删"，
/// 于是"socket 槽位"与"僵尸回收"都能有一个不依赖时间的判据（见 [`tick`]）。
fn active() -> &'static Mutex<HashMap<String, Instant>> {
    static ACTIVE: OnceLock<Mutex<HashMap<String, Instant>>> = OnceLock::new();
    ACTIVE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn active_count() -> usize {
    active().lock().map(|m| m.len()).unwrap_or(0)
}

/// panic 载荷 → 一行可读文本（与 `api::spawn_job` 同一口径）。
fn panic_text(payload: &(dyn std::any::Any + Send)) -> String {
    payload
        .downcast_ref::<&str>()
        .map(|s| (*s).to_string())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "载荷不是字符串".into())
}

/// 跑一段 worker 任务体并**接住 panic**：返回 `(正常结果, panic 文本)`。
///
/// 为什么必须有这一层：这个 worker 是**单例**（`channel()` 只让启动一次），它的循环就是一个
/// tokio task —— 任务体 panic 会把整个循环 future 掀掉，**从此再没有任何东西消费下载队列**
/// （要重启 server 才恢复）。而且任务体创建的 Job 会永远停在 `Running`、占着
/// `dl:<session>` 资源键，让重新排队的那个会话永远排在它后面。
async fn catch_panic<F, X>(fut: F) -> (Option<Result<X>>, Option<String>)
where
    F: std::future::Future<Output = Result<X>>,
{
    match std::panic::AssertUnwindSafe(fut).catch_unwind().await {
        Ok(v) => (Some(v), None),
        Err(payload) => (None, Some(panic_text(&payload))),
    }
}

/// 替"因 panic 而停摆"的下载任务收尾（按资源键找出停在 `running` 的那个）。
async fn fail_stuck_download_job(state: &Arc<AppState>, session_id: &str, msg: &str) {
    let key = crate::scope_key::download_key(session_id);
    let stuck: Vec<String> = state
        .jobs
        .list()
        .await
        .into_iter()
        .filter(|j| j.status == "running" && j.resources.iter().any(|r| r == &key))
        .map(|j| j.id)
        .collect();
    for id in stuck {
        tracing::error!("下载任务 panic，替它收尾（job={id} session={session_id}）：{msg}");
        state
            .jobs
            .fail(&id, format!("下载任务内部错误（panic）: {msg}"))
            .await;
    }
}

/// RAII：任务结束（**含 panic 与提前 `?` 返回**）时把会话从活跃表里摘掉。
struct ActiveGuard(String);

impl Drop for ActiveGuard {
    fn drop(&mut self) {
        if let Ok(mut m) = active().lock() {
            m.remove(&self.0);
        }
    }
}

/// 唤醒 worker：`Some(session)` = 处理该会话，`None` = 全量扫一遍。
pub fn notify(state: Arc<AppState>, session_id: Option<String>) {
    match channel().get() {
        Some(tx) => {
            if let Err(e) = tx.try_send(session_id) {
                // 队列满/关闭都不该影响提交请求的 2xx：周期 tick 会兜底
                tracing::debug!("下载 worker 唤醒失败（周期扫描兜底）: {e}");
            }
        }
        None => {
            let _ = state;
        }
    }
}

/// 启动后台 worker（`server::run` 调用一次）。
pub fn start(state: Arc<AppState>) {
    let (tx, mut rx) = mpsc::channel::<Option<String>>(64);
    if channel().set(tx).is_err() {
        return; // 已经启动过
    }
    recover_stale(&state);
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(Duration::from_secs(10));
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                msg = rx.recv() => {
                    match msg {
                        Some(Some(id)) => {
                            let (res, panicked) = catch_panic(kick_one(&state, &id)).await;
                            if let Some(msg) = panicked {
                                tracing::error!("下载 worker 任务 panic（session={id}）：{msg}");
                                fail_stuck_download_job(&state, &id, &msg).await;
                            } else if let Some(Err(e)) = res {
                                tracing::warn!("下载启动失败 session={id}: {e:#}");
                            }
                        }
                        Some(None) => {
                            let (res, panicked) = catch_panic(tick(&state)).await;
                            if let Some(msg) = panicked {
                                tracing::error!("下载队列扫描 panic：{msg}");
                            } else if let Some(Err(e)) = res {
                                tracing::warn!("下载队列扫描失败: {e:#}");
                            }
                        }
                        None => break, // 所有发送端关闭
                    }
                }
                _ = ticker.tick() => {
                    let (res, panicked) = catch_panic(tick(&state)).await;
                    if let Some(msg) = panicked {
                        tracing::error!("下载队列周期扫描 panic：{msg}");
                    } else if let Some(Err(e)) = res {
                        tracing::warn!("下载队列周期扫描失败: {e:#}");
                    }
                }
            }
        }
    });
    tracing::info!("B站下载 worker 已启动（下载槽 + 每 10s 队列扫描 / 僵尸回收）");
}

/// 启动时的残留回收：把上次进程留下的 `downloading` 打回 `queued`。
///
/// ⚠️ **前提是单实例**（`instancelock` 已保证）。若哪天真要跑多实例，这一步会把别的实例
/// 正在跑的下载抢回来 —— 那时应当改成"按 instance_id 认领"。
///
/// 之所以只在启动时做而不在 `tick` 里做：见 [`tick`] 的注释。
fn recover_stale(state: &Arc<AppState>) {
    let db = state.auth.db();
    let rows = match db.list_ingest_sessions(None, &[STATE_DOWNLOADING], 500) {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!("残留下载会话扫描失败: {e:#}");
            return;
        }
    };
    let mut n = 0usize;
    for s in rows {
        if s.kind != "bilibili" {
            continue; // 录播姬的 downloading 语义是另一套，别碰
        }
        match db.set_ingest_state(&s.session_id, STATE_QUEUED, Some("服务重启，下载已重新排队")) {
            Ok(()) => n += 1,
            Err(e) => tracing::warn!("回收残留会话失败 session={}: {e:#}", s.session_id),
        }
    }
    if n > 0 {
        tracing::info!("下载队列：{n} 个残留会话已重新排队（上次进程未正常结束）");
    }
}

/// 遍历所有待下载的会话，把空出来的下载槽填满。
async fn tick(state: &Arc<AppState>) -> Result<()> {
    let db = state.auth.db();
    let cfg = yekitrim_core::config::load_global_config();

    // 槽位按**事实表**算，不按 db 里的 `downloading` 计数：
    // 后者在进程崩溃后会偏大，导致队列永远推不动。
    let limit = cfg.ingest.max_concurrent_downloads.max(1) as usize;
    let mut free = limit.saturating_sub(active_count());

    // 僵尸回收：db 说是 `downloading`，但事实表里没有 → 那个任务已经不在了
    // （panic 或被 abort），状态是脏的。
    //
    // 运行期做这一步是**安全**的，因为取数顺序与写入顺序都是固定的：
    // - 启动：先 `active().insert` 再 `set_ingest_state(downloading)`；
    // - 结束：先 `set_ingest_state(finalized|failed)` 再 `ActiveGuard::drop`。
    // 于是"db=downloading ∧ ∉ active"任何时刻都只可能是"任务已经没了"。
    //
    // 唯一的例外是"写 db 失败"：那会打回 queued 再下一次 —— 本来就是失败重试，可接受。
    for s in db.list_ingest_sessions(None, &[STATE_DOWNLOADING], 500)? {
        if s.kind != "bilibili" {
            continue;
        }
        let known = active().lock().map(|m| m.contains_key(&s.session_id)).unwrap_or(true);
        if !known {
            tracing::warn!("会话 {} 停在 downloading 但已无下载任务，重新排队", s.session_id);
            let _ = db.set_ingest_state(&s.session_id, STATE_QUEUED, Some("下载任务已不存在，已重新排队"));
        }
    }

    if free == 0 {
        return Ok(());
    }

    // 工具没就绪时**不消费队列**：直接把会话标失败会逼用户重试一遍，
    // 而这里只是"还没装"，前端有引导卡让他装完再来。
    if registry::find_ytdlp(&cfg).is_none() {
        let queued = db.list_ingest_sessions(None, &[STATE_QUEUED], 1)?;
        if !queued.is_empty() {
            tracing::debug!("有 {} 个待下载会话，但 yt-dlp 未就绪（等待用户在设置页安装）", queued.len());
        }
        return Ok(());
    }

    for s in db.list_ingest_sessions(None, &[STATE_QUEUED], 100)? {
        if free == 0 {
            break;
        }
        if s.kind != "bilibili" {
            continue; // 别的来源的 queued 与我们无关（将来别的 worker 自己认领）
        }
        // 先占事实表，再落状态：这两步之间 tick 重入也不会重复取这个会话
        {
            let mut m = active().lock().unwrap_or_else(|e| e.into_inner());
            if m.contains_key(&s.session_id) {
                continue;
            }
            m.insert(s.session_id.clone(), Instant::now());
        }
        db.set_ingest_state(&s.session_id, STATE_DOWNLOADING, None)?;
        free -= 1;
        spawn_download(state.clone(), s);
    }
    Ok(())
}

/// 唤醒单个会话（由 `notify` 触发）：接管为可下载状态后交给 [`spawn_download`]。
async fn kick_one(state: &Arc<AppState>, session_id: &str) -> Result<()> {
    let db = state.auth.db();
    let Some(s) = db.ingest_session(session_id)? else {
        return Ok(());
    };
    if s.kind != "bilibili" || s.project_id.is_some() {
        return Ok(());
    }
    if s.owner_id.is_none() {
        bail!("会话没有归属账号（B站下载应直接归提交者）");
    }
    let cfg = yekitrim_core::config::load_global_config();
    if registry::find_ytdlp(&cfg).is_none() {
        bail!("{}", DownloadErrorKind::ToolMissing.user_message());
    }
    if s.state != STATE_QUEUED && s.state != STATE_DOWNLOADING {
        db.set_ingest_state(session_id, STATE_QUEUED, None)?;
    }
    {
        let mut m = active().lock().unwrap_or_else(|e| e.into_inner());
        if m.contains_key(session_id) {
            return Ok(()); // 已经在跑了
        }
        m.insert(session_id.to_string(), Instant::now());
    }
    db.set_ingest_state(session_id, STATE_DOWNLOADING, None)?;
    spawn_download(state.clone(), s);
    Ok(())
}

/// 起一个后台任务跑完整个下载流程；失败写 `failed`，成功由 [`landing`] 写 `finalized`。
fn spawn_download(state: Arc<AppState>, session: IngestSessionRow) {
    let sid = session.session_id.clone();
    tokio::spawn(async move {
        let _guard = ActiveGuard(sid.clone());
        match download_one(&state, &session).await {
            Ok(project_id) => {
                tracing::info!("B站下载完成: session={sid} → project={project_id}");
            }
            Err(e) => {
                let msg = format!("{e:#}");
                tracing::warn!("B站下载失败 session={sid}: {msg}");
                let _ = state.auth.db().set_ingest_state(&sid, "failed", Some(&msg));
            }
        }
    });
}

/// 一个会话的完整下载流程。
///
/// 成功时返回项目 id（会话状态已由 [`landing`] 写成 `finalized`）。
async fn download_one(state: &Arc<AppState>, session: &IngestSessionRow) -> Result<i64> {
    let db = state.auth.db();
    let cfg = yekitrim_core::config::load_global_config();
    let Some(owner_id) = session.owner_id else {
        bail!("会话没有归属账号");
    };

    // 1) 来源引用 → (bvid, page)
    let raw = session
        .source_ref
        .as_deref()
        .ok_or_else(|| anyhow!("会话缺少来源标识（source_ref）"))?;
    let (bvid, page) = bilibili::parse_session_ref(raw)
        .ok_or_else(|| anyhow!("会话来源标识无法解析: {raw}"))?;

    // 2) view API 拿 cid（弹幕要靠它）+ 标题（项目名）
    let info = bilibili::fetch_view(&bvid)
        .await
        .map_err(|e| anyhow!("{}", e.user_facing()))
        .with_context(|| format!("获取视频信息失败（{bvid}）"))?;
    if info.pages.is_empty() {
        bail!("这个视频没有可下载的分P（B站返回了空的分P 列表）");
    }
    let want = page.unwrap_or(1);
    let row = info
        .pages
        .iter()
        .find(|p| p.index == want)
        .ok_or_else(|| anyhow!("这个视频没有第 {want} P（共 {} P）", info.pages.len()))?;

    // 3) 项目目录（先建，下载器直接写进来 —— B站不需要"搬入"这一步）
    let project_root = projects::new_project_dir(owner_id, &bvid);

    // 4) 下载器就绪 + 引擎装配（**唯一的装配点**）
    let install = registry::find_ytdlp(&cfg).ok_or_else(|| {
        anyhow!("{}", DownloadErrorKind::ToolMissing.user_message())
    })?;
    let kind = DownloaderKind::parse(&cfg.ingest.downloader);
    let engine = downloader::for_kind(&kind, install.path.clone())?;
    let spec = engine.download_spec(&request(&cfg, &info, row, &project_root))?;

    // 5) 容量预检（在建目录之前；体积只能粗估，宁可偏大）
    let incoming = estimate_bytes(row.duration_secs, cfg.ingest.max_quality.as_str()).unwrap_or(0);
    capacity::check_capacity(db, owner_id, incoming, &project_root, &cfg)?;

    // 6) 任务（进度/取消入口；档位按"吃网络不吃 CPU"算 Medium）
    let title = page_title(&info.title, row, info.pages.len());
    let job = state
        .jobs
        .create(
            owner_id,
            JobSpec::new(
                "download_bilibili",
                format!("下载 {title}"),
                vec![crate::scope_key::download_key(&session.session_id)],
                JobClass::Medium,
            )
            .background(),
        )
        .await;
    let job_id = job.id.clone();
    let cancel = state.jobs.cancel_token(&job_id).await;
    state
        .jobs
        .log(&job_id, "info", format!("目标：{title}（{}）", spec.display_line()))
        .await;
    if job.wait_start().await.is_err() {
        state.jobs.complete(&job_id).await;
        bail!("任务已在队列中被取消");
    }

    // 7) 目录 + 下载。失败要清掉半成品目录（否则列表页会多一个空项目目录）
    std::fs::create_dir_all(&project_root)
        .with_context(|| format!("创建项目目录失败: {}", project_root.display()))?;
    let idle = Duration::from_secs(cfg.ingest.download_idle_timeout_secs.max(30) as u64);
    let downloaded = run_download(
        engine.as_ref(),
        &spec,
        Some((state, &job_id, cancel)),
        idle,
    )
    .await;
    let downloaded = match downloaded {
        Ok(d) => d,
        Err(e) => {
            cleanup_dir(&project_root);
            state.jobs.fail(&job_id, format!("{e:#}")).await;
            return Err(e);
        }
    };

    // ⚠️ 不要靠文件名模板猜产物：B站标题里的 `/ \ : * ? " < >` 会被消毒。
    //    `--print after_move:filepath` 是唯一的真相来源。
    let final_path = downloaded
        .path
        .clone()
        .ok_or_else(|| anyhow!("下载器没有回报产物路径（--print after_move:filepath 无输出）"))?;
    if !final_path.is_file() {
        cleanup_dir(&project_root);
        let msg = format!("下载器回报的产物不存在: {}", final_path.display());
        state.jobs.fail(&job_id, msg.clone()).await;
        bail!("{msg}");
    }
    let input_file = match downloader::ytdlp::relative_input_path(&project_root, &final_path) {
        Ok(v) => v,
        Err(e) => {
            cleanup_dir(&project_root);
            state.jobs.fail(&job_id, format!("{e:#}")).await;
            return Err(e);
        }
    };
    if let Some(bytes) = std::fs::metadata(&final_path).ok().map(|m| m.len()) {
        tracing::info!("下载产物 {}（{} 字节）", final_path.display(), bytes);
    }

    // 8) 弹幕（**旁路**：不走 yt-dlp，自己按 cid GET）
    //
    // 失败**不阻塞**建项目：视频已经下完（可能几十分钟），为了弹幕把整件事推翻不合理，
    // 而且录播姬通路同样允许"没有弹幕"。但必须明确告诉用户排序会受影响。
    let danmaku_rel = match bilibili::fetch_danmaku(row.cid).await {
        Ok(f) => match bilibili::danmaku::save_danmaku(&project_root, &f) {
            Ok(rel) => {
                state
                    .jobs
                    .log(&job_id, "info", format!("弹幕 {} 条已写入", f.entries))
                    .await;
                Some(rel)
            }
            Err(e) => {
                tracing::warn!("弹幕落盘失败（项目仍可跑）: {e:#}");
                None
            }
        },
        Err(e) => {
            tracing::warn!("弹幕获取失败（项目仍可跑）: {e}");
            state
                .jobs
                .log(
                    &job_id,
                    "warn",
                    format!("弹幕获取失败，热度排序会受影响：{}", e.user_message()),
                )
                .await;
            None
        }
    };

    // 9) 落位（建项目 + 登记 + 回写会话）
    let label = source_label(&bvid, want, info.pages.len());
    let landed = landing::land_project(
        db,
        &landing::ProjectLanding {
            owner_id,
            name: title.clone(),
            source_kind: "bilibili",
            source_label: Some(&label),
            // ⚠️ bvid **绝不能**塞进 `projects.room`：前端把它当"直播间号"展示
            room: None,
            project_root: &project_root,
            input_file: &input_file,
            danmaku_file: danmaku_rel.as_deref(),
            session_id: Some(&session.session_id),
        },
    )?;

    // 10) 自动管线（与录播姬同一条通道）
    let pipeline = Pipeline::parse(
        session
            .auto_pipeline
            .as_deref()
            .unwrap_or(cfg.ingest.recorder.auto_pipeline.as_str()),
    );
    if pipeline != Pipeline::None {
        spawn_pipeline(
            state,
            owner_id,
            landed.project_id,
            landed.config_path,
            pipeline,
            "下载后自动处理",
        )
        .await;
    }

    state.jobs.complete(&job_id).await;
    Ok(landed.project_id)
}

/// 把配置 + 视频信息拼成引擎无关的下载请求。
fn request(
    cfg: &yekitrim_core::config::Config,
    info: &bilibili::BvInfo,
    page: &bilibili::BvPage,
    project_root: &Path,
) -> DownloadRequest {
    let mut req = DownloadRequest::new(
        // ⚠️ 用带 `?p=N` 的规范 URL，而不是用户粘的原文：
        // 用户可能粘的是 `b23.tv` 短链或带一堆追踪参数的形式。
        format!("https://www.bilibili.com/video/{}?p={}", info.bvid, page.index),
        project_root,
    );
    req.max_height = yekitrim_core::config::quality_height(&cfg.ingest.max_quality);
    req.rate_limit_kbps = (cfg.ingest.rate_limit_kbps > 0).then_some(cfg.ingest.rate_limit_kbps);
    // 多P 必须限定：否则 yt-dlp 会把整个合集都拖下来
    req.playlist_item = Some(page.index);
    req.cookie_file = {
        let p = cfg.ingest.cookie_file.trim();
        (!p.is_empty()).then(|| PathBuf::from(p))
    };
    req
}

/// 跑下载器子进程，逐行分流 stdout，**无输出超时兜底**。
///
/// ## 三条实测约束
///
/// 1. **进度与路径都在 stdout**（非 TTY 下 yt-dlp 默认不给进度条，加了 `--progress` 才给，
///    而那时它与 `--print` 混在同一个流里）→ 只能靠哨兵前缀分流，不能靠行位置。
/// 2. 错误在 **stderr**，所以两边都要收；分类用合并后的文本。
/// 3. `kill_on_drop(true)`：任务被 abort / 提前 `?` 返回时**不留孤儿进程**。
///
/// 超时只判"一行输出都没有"（`idle`），**不设总时长上限** —— 长录播几小时是正常的，
/// 卡死则表现为"一行不出"，两者用同一个阈值区分得开。
async fn run_download(
    engine: &dyn Downloader,
    spec: &CommandSpec,
    job: Option<(&Arc<AppState>, &str, Option<Arc<std::sync::atomic::AtomicBool>>)>,
    idle: Duration,
) -> Result<downloader::Downloaded> {
    use std::process::Stdio;

    let mut cmd = tokio::process::Command::new(&spec.program);
    yekitrim_core::process::hide_console_window(cmd.as_std_mut());
    cmd.args(&spec.args);
    for v in &spec.env_remove {
        cmd.env_remove(v);
    }
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);

    let mut child = cmd.spawn().map_err(|e| {
        anyhow!(
            "启动下载器失败（{}）：{e}",
            spec.program.display()
        )
    })?;

    let stdout = child.stdout.take().ok_or_else(|| anyhow!("下载器没有 stdout"))?;
    let stderr = child.stderr.take().ok_or_else(|| anyhow!("下载器没有 stderr"))?;

    // stderr 必须**并发**读：管道缓冲（约 64KB）写满后子进程会被阻塞，
    // 变成"看起来卡死"的活锁。
    let tail: Arc<Mutex<VecDeque<String>>> = Arc::new(Mutex::new(VecDeque::new()));
    let tail_reader = tail.clone();
    let err_task = tokio::spawn(async move {
        let mut lines = BufReader::new(stderr).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            let mut buf = tail_reader.lock().unwrap_or_else(|e| e.into_inner());
            if buf.len() >= 200 {
                buf.pop_front();
            }
            buf.push_back(line);
        }
    });

    let mut out = downloader::Downloaded::default();
    let mut last_pct = -1i32;
    let mut lines = BufReader::new(stdout).lines();
    let mut timed_out = false;

    loop {
        // 取消：用户点了"取消任务"
        if let Some((_, _, Some(flag))) = &job {
            if flag.load(std::sync::atomic::Ordering::Relaxed) {
                let _ = child.kill().await;
                bail!("下载已取消");
            }
        }
        let next = tokio::time::timeout(idle, lines.next_line()).await;
        match next {
            Err(_) => {
                timed_out = true;
                break;
            }
            Ok(Ok(None)) => break, // EOF：进程结束（或即将结束）
            Ok(Ok(Some(line))) => match engine.parse_stream_line(&line) {
                Some(StreamEvent::Progress { percent, speed, eta }) => {
                    // 节流：只在整数百分比变化时上报（进度行每秒好几条）
                    let pct = percent.round() as i32;
                    if pct != last_pct {
                        last_pct = pct;
                        if let Some((st, jid, _)) = &job {
                            st.jobs.progress(jid, pct.max(0) as u64, 100).await;
                            st.jobs
                                .log(jid, "info", format!("{pct}%  {speed}  剩余 {eta}"))
                                .await;
                        }
                    }
                }
                Some(StreamEvent::FinalPath(p)) => out.path = Some(p),
                Some(StreamEvent::Meta { resolution, vcodec, acodec }) => {
                    out.resolution = Some(resolution);
                    out.vcodec = Some(vcodec);
                    out.acodec = Some(acodec);
                }
                // 非哨兵行：可能是被打到 stdout 的告警，收进尾巴供分类用
                None => {
                    let mut buf = tail.lock().unwrap_or_else(|e| e.into_inner());
                    if buf.len() >= 200 {
                        buf.pop_front();
                    }
                    buf.push_back(line);
                }
            },
            Ok(Err(e)) => {
                let _ = child.kill().await;
                bail!("读取下载器输出失败: {e}");
            }
        }
    }

    if timed_out {
        // 杀掉剩下的：`kill_on_drop` 会在 drop 时兜底，但这里要立刻拿到退出码
        let _ = child.kill().await;
        let _ = child.wait().await;
        let _ = err_task.await;
        let hint = tail
            .lock()
            .ok()
            .and_then(|b| b.back().cloned())
            .unwrap_or_default();
        bail!(
            "下载器连续 {} 秒没有任何输出，已判定卡死并停止{}",
            idle.as_secs(),
            if hint.trim().is_empty() {
                String::new()
            } else {
                format!("（最后一行：{}）", preview(&hint, 160))
            }
        );
    }

    let status = child.wait().await.context("等待下载器退出失败")?;
    let _ = err_task.await;
    let combined = {
        let buf = tail.lock().unwrap_or_else(|e| e.into_inner());
        buf.iter().cloned().collect::<Vec<_>>().join("\n")
    };

    if !status.success() {
        let err = engine
            .classify(&combined)
            .unwrap_or_else(|| downloader::DownloadError::new(DownloadErrorKind::Other, combined.clone()));
        // 原始行只进日志（可能带 cookie 相关提示），界面拿到的是人话
        tracing::warn!(
            "下载器退出码 {}: {}",
            status.code().map(|c| c.to_string()).unwrap_or_else(|| "signal".into()),
            preview(&err.raw, 400)
        );
        bail!("{}", err.user_facing());
    }

    if out.path.is_none() {
        // 退出码 0 但没有产物路径：只有一种可能 —— 模板被人改过。明确报错，别猜路径。
        bail!("下载器正常退出但没有回报产物路径（检查 [ingest] 相关配置）");
    }
    Ok(out)
}

/// 清理刚建的半成品目录（失败路径）。
///
/// 只删**我们自己刚建的那个**（`new_project_dir` 生成的时间戳目录）；不递归删用户数据。
fn cleanup_dir(dir: &Path) {
    if let Err(e) = std::fs::remove_dir_all(dir) {
        tracing::warn!("清理半成品目录失败 {}: {e}", dir.display());
    }
}

/// 日志用的单行预览：压掉换行、按**字符**截断（不是字节 —— 中文会被切坏）。
///
/// 子进程的输出可能很长、也可能带上下载器自己打出来的路径，日志里不该整段灌进去。
fn preview(s: &str, max: usize) -> String {
    let flat = s.replace(['\r', '\n'], " ");
    let flat = flat.trim();
    let mut out: String = flat.chars().take(max).collect();
    if flat.chars().count() > max {
        out.push('…');
    }
    out
}

/// 项目名（多P 带上 P 号与分P 名，否则列表里几个 P 会长得一模一样）。
fn page_title(video_title: &str, page: &bilibili::BvPage, total: usize) -> String {
    if total <= 1 {
        return video_title.to_string();
    }
    let part = page.part.trim();
    if part.is_empty() {
        format!("{video_title} P{}", page.index)
    } else {
        format!("{video_title} P{} {part}", page.index)
    }
}

/// 来源标签（列表页副标题）。
fn source_label(bvid: &str, page: u32, total: usize) -> String {
    if total <= 1 {
        format!("B站 · {bvid}")
    } else {
        format!("B站 · {bvid} P{page}")
    }
}

/// 下载体积粗估（容量预检用）。
///
/// ⚠️ 这是**上界估计**，不是精确值：按"视频+音频"的经验码率取
/// 1080p≈300KB/s、720p≈150KB/s、480p≈80KB/s（AVC 混合流）。
/// 宁可偏大 —— 误拒的代价是用户重试一次，放过的代价是把磁盘写满。
///
/// 拿不到时长返回 `None`（**不是 0**）：调用方按"跳过预检"处理，
/// 而 0 会被当成"这次不占空间"从而完全绕过水位检查。
pub fn estimate_bytes(duration_secs: Option<f64>, max_quality: &str) -> Option<u64> {
    let d = duration_secs.filter(|d| *d > 0.0)?;
    let kb_per_sec = match yekitrim_core::config::quality_height(max_quality) {
        Some(h) if h <= 480 => 80u64,
        Some(h) if h <= 720 => 150,
        _ => 300,
    };
    Some((d.ceil() as u64).saturating_mul(kb_per_sec).saturating_mul(1024))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// panic 载荷的两种标准形态都要能读出来（第三行是"不是字符串"的兜底）。
    #[test]
    fn panic_text_reads_str_and_string_payloads() {
        let borrowed: &str = "boom";
        assert_eq!(panic_text(&borrowed as &(dyn std::any::Any + Send)), "boom");
        let owned = String::from("boom2");
        assert_eq!(panic_text(&owned as &(dyn std::any::Any + Send)), "boom2");
        assert_eq!(
            panic_text(&42u8 as &(dyn std::any::Any + Send)),
            "载荷不是字符串"
        );
    }

    /// panic 路径的收尾：停在 `running` 的下载任务必须被标失败并**释放 `dl:<session>` 键**。
    ///
    /// 不释放的后果很具体：`tick` 的僵尸回收会把会话打回 `queued`，但重试新建的任务
    /// 会撞上那把仍然被占着的资源键 —— 于是永远排队，而队列里只有一个"运行中"的任务什么都查不出来。
    /// 回归钉子对应 `start()` 里的 `catch_panic` + [`fail_stuck_download_job`]。
    #[tokio::test]
    async fn fail_stuck_download_job_releases_the_key() {
        use crate::api::test_support::test_app;
        let t = test_app().await;
        let session = "bili-sess-1";
        let key = crate::scope_key::download_key(session);

        let job = t
            .state
            .jobs
            .create(
                1,
                JobSpec::new("download_bilibili", "下载 测试", vec![key.clone()], JobClass::Medium),
            )
            .await;
        assert!(
            t.state.jobs.busy_with(&[key.clone()]).await.is_some(),
            "刚创建的任务应当占着 dl:<session> 键"
        );

        // 另一个会话的任务不该被误伤（收尾是按资源键精确匹配的）
        let other_key = crate::scope_key::download_key("bili-sess-2");
        let other = t
            .state
            .jobs
            .create(
                1,
                JobSpec::new("download_bilibili", "下载 别的", vec![other_key.clone()], JobClass::Medium),
            )
            .await;

        fail_stuck_download_job(&t.state, session, "boom").await;

        assert_eq!(t.state.jobs.get(&job.id).await.unwrap().status, "failed");
        assert!(
            t.state.jobs.busy_with(&[key]).await.is_none(),
            "收尾后资源键必须释放，否则重试的下载永远排队"
        );
        assert!(
            t.state.jobs.busy_with(&[other_key]).await.is_some(),
            "别的会话的任务不该被误伤"
        );
        assert_eq!(t.state.jobs.get(&other.id).await.unwrap().status, "running");
    }

    fn page(index: u32, part: &str, cid: i64) -> bilibili::BvPage {
        bilibili::BvPage {
            index,
            cid,
            part: part.to_string(),
            duration_secs: Some(600.0),
        }
    }

    /// 体积估算：拿不到时长必须是 `None`（不是 0 —— 0 会绕过水位检查）。
    #[test]
    fn estimate_is_none_without_duration() {
        assert_eq!(estimate_bytes(None, "1080p"), None);
        assert_eq!(estimate_bytes(Some(0.0), "1080p"), None);
        assert_eq!(estimate_bytes(Some(-5.0), "720p"), None);
    }

    /// 清晰度越低估算越小；`best`/不限按最高档算（宁可偏大）。
    #[test]
    fn estimate_scales_with_quality() {
        let d = Some(3600.0);
        let (a, b, c) = (
            estimate_bytes(d, "480p").unwrap(),
            estimate_bytes(d, "720p").unwrap(),
            estimate_bytes(d, "1080p").unwrap(),
        );
        assert!(a < b && b < c, "{a} {b} {c}");
        assert_eq!(estimate_bytes(d, "best").unwrap(), c, "不限高按最高档估");
        assert_eq!(estimate_bytes(d, "").unwrap(), c);
        // 一小时 1080p ≈ 1.0–1.1 GB，量级要对（别写成 KB 或 TB）
        assert!((1_000_000_000..1_300_000_000).contains(&c), "{c}");
    }

    /// 多P 的项目名要能区分（否则列表里几个 P 一模一样，用户没法选）。
    #[test]
    fn multi_page_titles_are_distinguishable() {
        let p3 = page(3, "第三话", 333);
        assert_eq!(page_title("标题", &p3, 1), "标题", "单P 不加后缀");
        assert_eq!(page_title("标题", &p3, 23), "标题 P3 第三话");
        assert_eq!(page_title("标题", &page(2, "  ", 1), 23), "标题 P2");
        assert_eq!(source_label("BV1xx", 1, 1), "B站 · BV1xx");
        assert_eq!(source_label("BV1xx", 3, 23), "B站 · BV1xx P3");
    }

    /// 请求要用**规范 URL**（带 `?p=`）并限定单P —— 用户粘的可能是短链/带追踪参数的形式。
    #[test]
    fn request_is_canonical_and_single_page() {
        let cfg = yekitrim_core::config::Config::default();
        let info = bilibili::BvInfo {
            bvid: "BV1GJ411x7h7".into(),
            aid: 1,
            title: "标题".into(),
            uploader: "up".into(),
            duration_secs: Some(600.0),
            cover: None,
            pages: vec![page(1, "a", 1), page(3, "c", 3)],
            danmaku_total: 0,
            view_total: 0,
        };
        let req = request(&cfg, &info, &info.pages[1], Path::new("/data/u/1/bili/x"));
        assert_eq!(req.url, "https://www.bilibili.com/video/BV1GJ411x7h7?p=3");
        assert_eq!(req.playlist_item, Some(3), "必须限定分P，否则会把整个合集拖下来");
        assert_eq!(req.max_height, Some(1080), "默认 1080p");
        assert_eq!(req.cookie_file, None, "默认不配 cookie");
        assert_eq!(req.rate_limit_kbps, None, "0 = 不限速");
    }

    /// 活跃表的 RAII 语义：guard 一 drop，会话就必须从表里消失
    /// （僵尸回收全靠这条不成立就永远推不动队列）。
    #[test]
    fn active_guard_removes_on_drop() {
        let sid = format!("test-guard-{}", std::process::id());
        active().lock().unwrap().insert(sid.clone(), Instant::now());
        assert!(active_count() >= 1);
        {
            let _g = ActiveGuard(sid.clone());
            assert!(active().lock().unwrap().contains_key(&sid));
        }
        assert!(
            !active().lock().unwrap().contains_key(&sid),
            "guard 离开作用域后必须摘掉"
        );
    }
}
