//! 录播姬入库 worker（多用户服务 P3b，设计稿 §4.3 / §6 / §7）。
//!
//! 职责（**串行**，等价于"入库槽 max_concurrent_ingest = 1"）：
//! 1. 完结判定：`SessionEnded` 已到 且（所有 `FileOpening` 都有对应 `FileClosed`，
//!    或此后 `finalize_idle_mins` 内无新事件）；没有任何 `FileClosed` 的会话不建项目。
//! 2. 归属：`owner_id` 为空 → 进"待分配池"（`state=unassigned`），admin 指派后自动继续。
//! 3. 水位预检：全局上限 / 每用户配额 / 磁盘水位，不足则 `state=failed` 并给出
//!    "预计需要 / 可用"文案，**绝不自动删任何东西**（§7）。
//! 4. 落位：默认移入托管目录（同盘 `rename` 瞬时；跨盘 `copy + 校验 + 删源`），
//!    `move_into_managed=false` 时项目目录＝录制目录。
//! 5. 多分片：默认 `ffmpeg -f concat -c copy` 合并为 `source.mp4`（失败回退重编码）；
//!    原始分片保留（源不删策略）。
//! 6. 弹幕：同目录同名 `.xml`（录播姬）/`.ass` 一并搬入项目并写进 `yekitrim.toml`。
//! 7. 建项目 + 按 `auto_pipeline`（none|transcribe|detect|cut）自动跑管线。
//!
//! 任何一步失败都回滚刚建的托管目录并把会话标为 `failed`（错误文案直接给用户看）。
//!
//! ## 与 B站下载通道（`download_worker`）的边界
//!
//! 两者**不许互相调用**。共用的东西已经按用途分流（计划 §2）：
//! - 水位/配额预检 → `yekitrim_service::ingest::capacity`
//! - 建项目 + 登记 → `yekitrim_service::ingest::landing`
//! - 起管线 → `crate::pipeline_boot`
//!
//! 留在这里的是**录播姬独有**的语义：待分配池、`is_finalized` 空闲窗口、
//! 多分片合并、从录像目录搬文件。B站是"用户主动提交、进程退出即完结、下载器直接
//! 写进托管目录"，这些一个都用不上。

use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use tokio::sync::mpsc;

use yekitrim_service::db::IngestSessionRow;
use yekitrim_service::ingest::{capacity, landing};
use yekitrim_service::projects;

use crate::pipeline_boot::{spawn_pipeline, Pipeline};

use super::model::AppState;

type Notify = mpsc::Sender<Option<String>>;

fn channel() -> &'static OnceLock<Notify> {
    static TX: OnceLock<Notify> = OnceLock::new();
    &TX
}

/// 唤醒 worker：`Some(session)` = 处理该会话，`None` = 全量扫一遍。
pub fn notify(state: Arc<AppState>, session_id: Option<String>) {
    match channel().get() {
        Some(tx) => {
            if let Err(e) = tx.try_send(session_id) {
                // 队列满/关闭都不该影响 webhook 的 2xx：周期 tick 会兜底
                tracing::debug!("ingest worker 唤醒失败（周期扫描兜底）: {e}");
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
    tokio::spawn(async move {
        // 周期 tick：兜住"SessionEnded 后空闲窗口"这种纯时间条件
        let mut ticker = tokio::time::interval(Duration::from_secs(60));
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                msg = rx.recv() => {
                    match msg {
                        Some(Some(id)) => {
                            if let Err(e) = process_one(&state, &id).await {
                                tracing::warn!("入库失败 session={id}: {e:#}");
                            }
                        }
                        Some(None) => {
                            if let Err(e) = tick(&state).await {
                                tracing::warn!("入库扫描失败: {e:#}");
                            }
                        }
                        None => break, // 所有发送端关闭
                    }
                }
                _ = ticker.tick() => {
                    if let Err(e) = tick(&state).await {
                        tracing::warn!("入库周期扫描失败: {e:#}");
                    }
                }
            }
        }
    });
    tracing::info!("录播姬入库 worker 已启动（串行入库槽 + 每 60s 兜底扫描）");
}

/// 遍历所有"可入库"的会话。
async fn tick(state: &Arc<AppState>) -> Result<()> {
    let db = state.auth.db();
    let cfg = yekitrim_core::config::load_global_config();
    let recorder_root = cfg.ingest.recorder.root.trim().to_string();
    if recorder_root.is_empty() {
        return Ok(()); // 未配置录播姬目录：什么都不做（前端有引导卡）
    }

    // 兜底扫描（按 watch_interval_mins 节流）：覆盖漏推送 / 录播姬重启 / 历史录播导入
    let interval = cfg.ingest.recorder.watch_interval_mins as i64;
    if interval > 0 {
        let now = yekitrim_service::db::now_secs();
        let last = LAST_SCAN.load(std::sync::atomic::Ordering::Relaxed);
        if now - last >= interval * 60 {
            LAST_SCAN.store(now, std::sync::atomic::Ordering::Relaxed);
            // 递归 read_dir + 逐文件 metadata（录播根可能在网络盘上）→ 下 blocking 池
            let st = state.clone();
            let cfg_for_scan = cfg.clone();
            let scanned = tokio::task::spawn_blocking(move || scan_recorder(&st, &cfg_for_scan)).await;
            match scanned {
                Ok(Ok(n)) if n > 0 => tracing::info!("兜底扫描：新登记 {n} 个完整录像"),
                Ok(Ok(_)) => {}
                Ok(Err(e)) => tracing::warn!("兜底扫描失败: {e:#}"),
                Err(e) => tracing::warn!("兜底扫描线程失败: {e}"),
            }
        }
    }

    let sessions = db.list_ingest_sessions(None, &[], 200)?;
    let now = yekitrim_service::db::now_secs();
    for s in sessions {
        if s.project_id.is_some() {
            continue;
        }
        if s.owner_id.is_none() {
            if s.state != "unassigned" {
                db.set_ingest_state(&s.session_id, "unassigned", None)?;
            }
            continue;
        }
        if s.state == "failed" {
            continue; // 失败要 admin/用户点"重试"（会把它推回 closing）
        }
        if !is_finalized(&s, cfg.ingest.recorder.finalize_idle_mins, now) {
            continue;
        }
        if let Err(e) = ingest_one(state, &s, &cfg).await {
            let msg = format!("{e:#}");
            tracing::warn!("入库失败 session={}: {msg}", s.session_id);
            let _ = db.set_ingest_state(&s.session_id, "failed", Some(&msg));
        }
    }
    Ok(())
}

/// 上次兜底扫描时间（秒）。
static LAST_SCAN: std::sync::atomic::AtomicI64 = std::sync::atomic::AtomicI64::new(0);

/// 兜底扫描：把 `recorder_root` 下"两轮大小/时间都没变"的文件登记成可入库会话。
///
/// 覆盖三类场景：webhook 漏推送、录播姬重启后没补发、把历史录播直接丢进目录。
/// 返回本次新登记的完整文件数。
pub fn scan_recorder(state: &Arc<AppState>, cfg: &yekitrim_core::config::Config) -> Result<usize> {
    let db = state.auth.db();
    let root = PathBuf::from(cfg.ingest.recorder.root.trim());
    if !root.is_dir() {
        bail!("录播姬目录不存在: {}", root.display());
    }
    let exts: Vec<String> = cfg
        .ingest
        .recorder
        .extensions
        .iter()
        .map(|e| e.trim_start_matches('.').to_ascii_lowercase())
        .collect();

    let mut files: Vec<(String, PathBuf, u64, i64)> = Vec::new();
    collect_recordings(&root, &root, &exts, 0, &mut files)?;

    let mut registered = 0usize;
    for (rel, abs, size, mtime) in files {
        let rel = rel.replace('\\', "/");
        let prev = db.ingest_file(&rel)?;
        match prev {
            None => {
                db.upsert_ingest_file(&rel, None, None, size, mtime, "seen")?;
            }
            Some(p) if p.size == size && p.mtime == mtime && p.state == "seen" => {
                db.upsert_ingest_file(&rel, None, None, size, mtime, "stable")?;
                if register_scan_session(state, &root, &abs, &rel, cfg)? {
                    registered += 1;
                }
            }
            Some(p) if p.size == size && p.mtime == mtime => {
                // 已 stable/ingested：幂等，什么都不做
                let _ = p;
            }
            Some(_) => {
                // 还在写：更新为 seen，等下一轮再判稳定
                db.upsert_ingest_file(&rel, None, None, size, mtime, "seen")?;
            }
        }
    }
    Ok(registered)
}

/// 递归收集录像文件（限制深度，避免误扫巨树）；跳过 `.yekitrim`/隐藏目录。
fn collect_recordings(
    root: &Path,
    dir: &Path,
    exts: &[String],
    depth: usize,
    out: &mut Vec<(String, PathBuf, u64, i64)>,
) -> Result<()> {
    if depth > 4 {
        return Ok(());
    }
    let Ok(entries) = std::fs::read_dir(dir) else { return Ok(()) };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with('.') {
            continue;
        }
        if path.is_dir() {
            collect_recordings(root, &path, exts, depth + 1, out)?;
            continue;
        }
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        if !exts.iter().any(|e| e == &ext) {
            continue;
        }
        let Ok(meta) = entry.metadata() else { continue };
        let mtime = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        let rel = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .to_string_lossy()
            .into_owned();
        out.push((rel, path, meta.len(), mtime));
    }
    Ok(())
}

/// 一条录播姬目录里的录像文件（**只读**扫描结果）。
#[derive(Debug, Clone)]
pub struct RecordedFile {
    /// 相对 `[ingest.recorder] root` 的路径（正斜杠，与 `ingest_files.path` 同一口径）
    pub rel: String,
    /// 文件名（列表里显示用）
    pub name: String,
    pub size: u64,
    pub mtime: i64,
    /// 从路径解析出的房间号（解析不出 = `None`，这种文件不属于任何已知房间）
    pub room: Option<String>,
}

/// 某个房间在录像目录里的汇总（房间行上显示"已录 N 个 · 大小"）。
#[derive(Debug, Clone, Copy, Default)]
pub struct RoomRecordings {
    pub files: usize,
    pub bytes: u64,
    /// 最新一个文件的 mtime（0 = 没有）
    pub latest: i64,
}

/// 列出录像目录里的全部录像（**只读**：不登记会话、不改任何库表）。
///
/// 递归 `read_dir` + 逐文件 `metadata` 在网盘上可能要几秒 → **调用方必须下 blocking 池**。
/// 首页「我的直播间」每行要 `已录 N 个 · 大小`、展开又要列文件，同一次页面加载会调两次，
/// 所以这里带一层短 TTL 缓存（与 `api::probe_cache` 同一个模式，进程级、按 root+扩展名区分）。
pub fn list_recordings(cfg: &yekitrim_core::config::Config) -> Result<Vec<RecordedFile>> {
    let root = PathBuf::from(cfg.ingest.recorder.root.trim());
    if !root.is_dir() {
        bail!("录播姬目录不存在: {}", root.display());
    }
    let exts: Vec<String> = cfg
        .ingest
        .recorder
        .extensions
        .iter()
        .map(|e| e.trim_start_matches('.').to_ascii_lowercase())
        .collect();
    let sig = format!("{}|{}", root.display(), exts.join(","));
    {
        let (cached_sig, at, files) = recordings_cache().lock().unwrap_or_else(|e| e.into_inner()).clone();
        if cached_sig == sig && at.elapsed() < RECORDINGS_TTL {
            return Ok(files);
        }
    }

    let mut raw: Vec<(String, PathBuf, u64, i64)> = Vec::new();
    collect_recordings(&root, &root, &exts, 0, &mut raw)?;
    let files: Vec<RecordedFile> = raw
        .into_iter()
        .map(|(rel, abs, size, mtime)| {
            let rel = rel.replace('\\', "/");
            let name = abs
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            let room = yekitrim_service::recorder::room_id_from_rel_path(&rel);
            RecordedFile { rel, name, size, mtime, room }
        })
        .collect();
    *recordings_cache().lock().unwrap_or_else(|e| e.into_inner()) =
        (sig, std::time::Instant::now(), files.clone());
    Ok(files)
}

/// 扫描结果的短 TTL 缓存：`(签名, 时刻, 文件)`。
type RecordingsCache = (String, std::time::Instant, Vec<RecordedFile>);
static RECORDINGS_CACHE: std::sync::OnceLock<std::sync::Mutex<RecordingsCache>> =
    std::sync::OnceLock::new();

/// `Instant::now()` 不是 const，缓存只能惰性建（`OnceLock`，与 `api::probe_cache` 同一模式）。
fn recordings_cache() -> &'static std::sync::Mutex<RecordingsCache> {
    RECORDINGS_CACHE.get_or_init(|| {
        std::sync::Mutex::new((String::new(), std::time::Instant::now(), Vec::new()))
    })
}
const RECORDINGS_TTL: std::time::Duration = std::time::Duration::from_secs(8);

/// 按房间汇总（只统计能解析出房间号的文件）。
pub fn room_summaries(files: &[RecordedFile]) -> std::collections::HashMap<String, RoomRecordings> {
    let mut out: std::collections::HashMap<String, RoomRecordings> = std::collections::HashMap::new();
    for f in files {
        let Some(room) = f.room.as_ref() else { continue };
        let e = out.entry(room.clone()).or_default();
        e.files += 1;
        e.bytes += f.size;
        e.latest = e.latest.max(f.mtime);
    }
    out
}

/// 给扫描出来的文件补一个可入库会话（归属按**路径里的房间号**解析后查 channel_map）。
///
/// ⚠️ 房间号必须从路径里**解析**出来，不能拿整个目录名去查：录播姬默认目录名是
/// `<房间号>-<主播名>`（实测 `26320007-耶斯yeki`），而 `channel_map` 里存的是 `26320007`
/// —— 直接比会永远匹配不上，扫到的真实录像全被判成"没登记"、掉进待分配池。
fn register_scan_session(
    state: &Arc<AppState>,
    root: &Path,
    abs: &Path,
    rel: &str,
    cfg: &yekitrim_core::config::Config,
) -> Result<bool> {
    let db = state.auth.db();
    // 目录名只用来拼 session_id（保持既有格式，避免升级后把同一份录像重复登记成两条会话）
    let dir = abs
        .parent()
        .and_then(|p| p.strip_prefix(root).ok())
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
        .unwrap_or_default();
    // 归属用的房间号：优先从相对路径解析，解析不出才退回目录名（保持原行为）
    let room = yekitrim_service::recorder::room_id_from_rel_path(rel)
        .unwrap_or_else(|| dir.clone());
    let stem = abs
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "rec".to_string());
    let session_id = format!("scan-{}-{}", if dir.is_empty() { "root" } else { &dir }, stem);

    if let Some(existing) = db.ingest_session(&session_id)? {
        if existing.project_id.is_some() {
            return Ok(false); // 已经入库过
        }
    }
    let rule = if room.is_empty() {
        None
    } else {
        db.channel_for_room(&room)?
    };
    db.ingest_session_touch(&session_id, Some(&room), None)?;
    db.ingest_session_add_file(&session_id, rel, true)?;
    db.ingest_session_add_file(&session_id, rel, false)?;
    let meta = std::fs::metadata(abs).ok();
    let size = meta.as_ref().map(|m| m.len()).unwrap_or(0);
    db.upsert_ingest_file(rel, Some(&session_id), Some(&room), size, 0, "ingested")?;
    let already_owned = db
        .ingest_session(&session_id)?
        .and_then(|s| s.owner_id)
        .is_some();
    match rule {
        Some(r) if r.enabled => {
            // 只在没有归属时按规则指派：admin 手动指派过的会话不该被扫描一轮就翻转回去
            if !already_owned {
                db.assign_ingest_session(&session_id, r.user_id, Some(&r.auto_pipeline))?;
            }
            db.set_ingest_state(&session_id, "closing", None)?;
        }
        _ => {
            // 同样别打回待分配：扫描每两轮就会走到这里，无脑置 unassigned 会把已经
            // 指派好的会话一次次踢回池子（`assign` 只把 unassigned→closing，来回震荡）
            if !already_owned {
                db.set_ingest_state(&session_id, "unassigned", None)?;
            }
        }
    }
    let _ = db.audit(
        None,
        "ingest_scan_register",
        &format!("session={session_id} file={rel} auto={}", cfg.ingest.recorder.auto_pipeline),
    );
    tracing::info!("兜底扫描登记: session={session_id} file={rel} room={room}");
    Ok(true)
}

/// 处理单个会话（webhook 唤醒时用；不满足完结条件就直接返回）。
async fn process_one(state: &Arc<AppState>, session_id: &str) -> Result<()> {
    let db = state.auth.db();
    let cfg = yekitrim_core::config::load_global_config();
    let Some(s) = db.ingest_session(session_id)? else {
        return Ok(());
    };
    if s.project_id.is_some() {
        return Ok(());
    }
    if s.owner_id.is_none() {
        db.set_ingest_state(session_id, "unassigned", None)?;
        return Ok(());
    }
    let now = yekitrim_service::db::now_secs();
    if !is_finalized(&s, cfg.ingest.recorder.finalize_idle_mins, now) {
        return Ok(());
    }
    if let Err(e) = ingest_one(state, &s, &cfg).await {
        let msg = format!("{e:#}");
        db.set_ingest_state(session_id, "failed", Some(&msg))?;
        return Err(e);
    }
    Ok(())
}

/// 完结判定：`SessionEnded` 已到 **且**（全部 FileOpening 都有 FileClosed，或空闲窗口已过）。
pub fn is_finalized(s: &IngestSessionRow, idle_mins: u32, now: i64) -> bool {
    if s.state != "closing" {
        return false;
    }
    if s.closed_files.is_empty() {
        return false; // 一个完整文件都没有：不建空项目
    }
    let all_closed = !s.opened_files.is_empty()
        && s.opened_files.iter().all(|p| s.closed_files.contains(p));
    if all_closed {
        return true;
    }
    let idle = now - s.last_event_at.unwrap_or(s.created_at);
    idle >= (idle_mins.max(1) as i64) * 60
}

/// 录播姬目录 + 相对路径 → 绝对路径（**囚笼**：§5.2 webhook 路径校验）。
pub(crate) fn resolve_recorder_path(recorder_root: &Path, rel: &str) -> Result<PathBuf> {
    let abs = yekitrim_core::tenant::ensure_within(recorder_root, Path::new(rel))
        .with_context(|| format!("录像路径越出录播姬目录: {rel}"))?;
    if !abs.is_file() {
        bail!("录像文件不存在: {}", abs.display());
    }
    Ok(abs)
}

/// 同目录同名弹幕（录播姬写 `.xml`；手工导入可能是 `.ass`）。
fn find_danmaku(video: &Path) -> Option<PathBuf> {
    let stem = video.file_stem()?.to_string_lossy().to_string();
    let dir = video.parent()?;
    for ext in ["xml", "ass", "srt"] {
        let cand = dir.join(format!("{stem}.{ext}"));
        if cand.is_file() {
            return Some(cand);
        }
    }
    None
}

fn file_size(p: &Path) -> u64 {
    std::fs::metadata(p).map(|m| m.len()).unwrap_or(0)
}


/// 把文件搬进项目目录：同盘 `rename`，跨盘 `copy + 校验 + 删源`。
fn move_file(src: &Path, dst: &Path) -> Result<()> {
    if let Some(parent) = dst.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let size = file_size(src);
    match std::fs::rename(src, dst) {
        Ok(()) => return Ok(()),
        Err(_) => {
            // 跨盘（或目标已存在）：拷贝后校验再删源
            std::fs::copy(src, dst)
                .with_context(|| format!("拷贝失败: {} → {}", src.display(), dst.display()))?;
            let copied = file_size(dst);
            if copied != size {
                let _ = std::fs::remove_file(dst);
                bail!(
                    "拷贝校验失败（源 {} 字节，目标 {} 字节）: {}",
                    size,
                    copied,
                    dst.display()
                );
            }
            std::fs::remove_file(src)
                .with_context(|| format!("删除源文件失败: {}", src.display()))?;
            Ok(())
        }
    }
}

/// 多分片合并（`-c copy`，失败回退重编码）。返回合并后的文件路径。
async fn concat_fragments(fragments: &[PathBuf], out: &Path) -> Result<()> {
    let list_path = out.with_extension("concat.txt");
    let mut body = String::new();
    for f in fragments {
        // ffmpeg concat 列表：路径里的单引号需要转义，反斜杠统一成正斜杠
        let p = f.to_string_lossy().replace('\\', "/").replace('\'', r"'\''");
        body.push_str(&format!("file '{p}'\n"));
    }
    std::fs::write(&list_path, body)?;

    let run = |extra: &[&str]| {
        let args: Vec<String> = std::iter::once("-y".to_string())
            .chain(std::iter::once("-f".to_string()))
            .chain(std::iter::once("concat".to_string()))
            .chain(std::iter::once("-safe".to_string()))
            .chain(std::iter::once("0".to_string()))
            .chain(std::iter::once("-i".to_string()))
            .chain(std::iter::once(list_path.to_string_lossy().into_owned()))
            .chain(extra.iter().map(|s| s.to_string()))
            .chain(std::iter::once(out.to_string_lossy().into_owned()))
            .collect();
        async move {
            let mut c = tokio::process::Command::new("ffmpeg");
            yekitrim_core::process::hide_console_window(c.as_std_mut());
            let output = c
                .args(&args)
                .output()
                .await
                .context("无法启动 ffmpeg")?;
            Ok::<_, anyhow::Error>(output)
        }
    };

    let first = run(&["-c", "copy", "-movflags", "+faststart"]).await?;
    let ok = first.status.success() && out.is_file() && file_size(out) > 0;
    if ok {
        let _ = std::fs::remove_file(&list_path);
        return Ok(());
    }
    tracing::warn!(
        "concat -c copy 失败，回退重编码: {}",
        String::from_utf8_lossy(&first.stderr).lines().last().unwrap_or("")
    );
    let second = run(&["-c:v", "libx264", "-preset", "veryfast", "-crf", "20", "-c:a", "aac"]).await?;
    let _ = std::fs::remove_file(&list_path);
    if !second.status.success() || !out.is_file() || file_size(out) == 0 {
        bail!(
            "多分片合并失败（拷贝与重编码都失败）: {}",
            String::from_utf8_lossy(&second.stderr).lines().last().unwrap_or("")
        );
    }
    Ok(())
}

/// 单个会话的完整入库流程。
async fn ingest_one(
    state: &Arc<AppState>,
    session: &IngestSessionRow,
    cfg: &yekitrim_core::config::Config,
) -> Result<i64> {
    let db = state.auth.db();
    let Some(owner_id) = session.owner_id else {
        bail!("会话尚未指派归属");
    };
    let recorder_root = PathBuf::from(cfg.ingest.recorder.root.trim());
    if !recorder_root.is_dir() {
        bail!("录播姬目录不存在: {}", recorder_root.display());
    }

    // 1) 解析录像文件（囚笼 + 去重 + 按文件名排序 = 分片顺序）
    let mut sources: Vec<PathBuf> = Vec::new();
    for rel in &session.closed_files {
        match resolve_recorder_path(&recorder_root, rel) {
            Ok(p) => {
                if !sources.contains(&p) {
                    sources.push(p);
                }
            }
            Err(e) => tracing::warn!("跳过无法使用的录像 {rel}: {e:#}"),
        }
    }
    if sources.is_empty() {
        bail!("本场没有可用的录像文件（路径都在录播姬目录之外或已不存在）");
    }
    sources.sort();
    let incoming: u64 = sources.iter().map(|p| file_size(p)).sum();

    let room = session
        .room_id
        .clone()
        .unwrap_or_else(|| "unassigned".to_string());
    let project_root = if cfg.ingest.recorder.move_into_managed {
        projects::new_project_dir(owner_id, &room)
    } else {
        sources[0]
            .parent()
            .map(|p| p.to_path_buf())
            .ok_or_else(|| anyhow!("无法确定项目目录"))?
    };

    // 2) 水位/配额预检（在建目录之前）
    capacity::check_capacity(db, owner_id, incoming, &project_root, cfg)?;

    // 3) 落位（失败时清掉刚建的托管目录）
    let created_dir = cfg.ingest.recorder.move_into_managed;
    if created_dir {
        std::fs::create_dir_all(&project_root)
            .with_context(|| format!("创建项目目录失败: {}", project_root.display()))?;
    }
    let result = ingest_into(state, session, owner_id, &sources, &project_root, cfg).await;
    match result {
        Ok(project_id) => Ok(project_id),
        Err(e) => {
            if created_dir {
                if let Err(cleanup) = std::fs::remove_dir_all(&project_root) {
                    tracing::warn!("清理半成品目录失败 {}: {cleanup}", project_root.display());
                }
            }
            Err(e)
        }
    }
}

/// 落位 + 建项目 + 自动管线（失败由 [`ingest_one`] 回滚目录）。
async fn ingest_into(
    state: &Arc<AppState>,
    session: &IngestSessionRow,
    owner_id: i64,
    sources: &[PathBuf],
    project_root: &Path,
    cfg: &yekitrim_core::config::Config,
) -> Result<i64> {
    let db = state.auth.db();
    let mut moved: Vec<PathBuf> = Vec::new();

    // 1) 搬文件（保留原始文件名，源不删策略的"删源"只发生在跨盘搬运时）。
    //    跨盘时要整段拷贝 GB 级录像 —— 同步做在 async 里会白占一个 HTTP worker 几十秒，
    //    所以每搬一个都下 blocking 池。
    for src in sources {
        let name = src
            .file_name()
            .ok_or_else(|| anyhow!("录像文件名异常: {}", src.display()))?;
        let dst = project_root.join(name);
        if dst == *src {
            moved.push(dst); // move_into_managed=false：文件本来就在项目目录
            continue;
        }
        let (s, d) = (src.clone(), dst.clone());
        tokio::task::spawn_blocking(move || move_file(&s, &d))
            .await
            .context("搬运线程失败")??;
        moved.push(dst);
    }

    // 2) 弹幕：同目录同名 .xml/.ass/.srt 一并搬入（缺失不阻塞）
    let mut danmaku_rel: Option<String> = None;
    for src in sources {
        if let Some(dm) = find_danmaku(src) {
            let name = dm
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| "danmaku.xml".to_string());
            let dst = project_root.join(&name);
            if dst != dm {
                let (s, d) = (dm.clone(), dst.clone());
                let moved_dm = tokio::task::spawn_blocking(move || move_file(&s, &d))
                    .await
                    .context("弹幕搬运线程失败")?;
                if let Err(e) = moved_dm {
                    tracing::warn!("弹幕搬运失败（项目仍可跑）: {e:#}");
                    continue;
                }
            }
            danmaku_rel = Some(name);
            break;
        }
    }

    // 3) 多分片合并（原始分片保留）
    let concat = cfg.ingest.recorder.concat_files && moved.len() > 1;
    let input_name = if concat {
        let out = project_root.join("source.mp4");
        concat_fragments(&moved, &out).await?;
        "source.mp4".to_string()
    } else {
        moved[0]
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .ok_or_else(|| anyhow!("录像文件名异常"))?
    };

    // 4) 建项目 + 登记（**共用逻辑已下沉**：`yekitrim_service::ingest::landing`）
    let name = session
        .room_id
        .clone()
        .map(|r| format!("房间 {r}"))
        .unwrap_or_else(|| "录播".to_string());
    let label = format!(
        "录播姬 · 房间 {}",
        session.room_id.clone().unwrap_or_else(|| "-".into())
    );
    let landed = landing::land_project(
        db,
        &landing::ProjectLanding {
            owner_id,
            name,
            source_kind: "recorder",
            source_label: Some(&label),
            room: session.room_id.as_deref(),
            project_root,
            input_file: &input_name,
            danmaku_file: danmaku_rel.as_deref(),
            session_id: Some(&session.session_id),
        },
    )?;
    let project_id = landed.project_id;
    tracing::info!(
        "入库完成: session={} → project={project_id}（{} 个文件，合并={concat}，新建={}）",
        session.session_id,
        moved.len(),
        landed.created
    );

    // 5) 自动管线
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
            project_id,
            landed.config_path,
            pipeline,
            "入库后自动处理",
        )
        .await;
    }
    Ok(project_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session(state: &str, opened: &[&str], closed: &[&str], last: i64) -> IngestSessionRow {
        IngestSessionRow {
            session_id: "s1".into(),
            room_id: Some("23058".into()),
            short_id: None,
            owner_id: Some(1),
            state: state.into(),
            opened_files: opened.iter().map(|s| s.to_string()).collect(),
            closed_files: closed.iter().map(|s| s.to_string()).collect(),
            first_event_at: Some(0),
            last_event_at: Some(last),
            project_id: None,
            auto_pipeline: None,
            error: None,
            created_at: 0,
            kind: "recorder".into(),
            source_ref: None,
        }
    }

    #[test]
    fn finalize_requires_session_ended_and_completion() {
        // 还在录制中 → 不完结
        assert!(!is_finalized(&session("open", &["a.flv"], &[], 0), 5, 10_000));
        // closing 但没有任何完整文件 → 不完结（不建空项目）
        assert!(!is_finalized(&session("closing", &["a.flv"], &[], 0), 5, 10_000));
        // closing + 所有 FileOpening 都有 FileClosed → 立即完结
        assert!(is_finalized(&session("closing", &["a.flv"], &["a.flv"], 9_999), 5, 10_000));
        // closing + 有文件但打开列表为空 → 等空闲窗口
        assert!(!is_finalized(&session("closing", &[], &["a.flv"], 9_999), 5, 10_000));
        assert!(is_finalized(&session("closing", &[], &["a.flv"], 9_000), 5, 10_000));
        // 乱序：FileOpening 比 FileClosed 多一个 → 等空闲窗口
        assert!(!is_finalized(&session("closing", &["a.flv", "b.flv"], &["a.flv"], 9_999), 5, 10_000));
        assert!(is_finalized(&session("closing", &["a.flv", "b.flv"], &["a.flv"], 9_000), 5, 10_000));
    }

}
