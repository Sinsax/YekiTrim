//! `api::render`：渲染 / 封面出片子进程与其进度解析。

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;


use super::pipeline::*;

// ===================== 渲染/封面子进程 =====================

/// 子进程输出里值得进「运行日志」的里程碑片段；其余行只走 debug（留在任务卡上，不刷日志）。
pub(super) const RENDER_MILESTONES: &[&str] = &[
    "-- Server --",
    "-- Sound data --",
    "-- Sticker GIF --",
    "-- Dhampir",   // 底座那条腿（只出画面，音轨仍归本仓）
    "-- Render chunks",
    "-- Concat --",
    "-- Audio --",
    "-- SFX mix",
    "-- Mux --",
    "-- Cover --",
    "FINISHED:",
    "Cover saved:",
    "✂ 已导出去除",
    "⏱",
];

/// 跟随一个渲染/封面子进程的日志文件（边跑边读）。
#[derive(Default)]
pub(super) struct RenderOutput {
    path: PathBuf,
    offset: u64,
    carry: Vec<u8>,
    /// 最近 30 行：失败时进错误文案
    tail: std::collections::VecDeque<String>,
    /// 每个分块的最新百分比（进度 = 最慢的那块）
    chunk_pct: std::collections::BTreeMap<String, u32>,
    last_active: Option<std::time::Instant>,
    warned_silent: Option<std::time::Instant>,
}

impl RenderOutput {
    pub(super) fn new(path: PathBuf) -> Self {
        Self {
            path,
            last_active: Some(std::time::Instant::now()),
            ..Default::default()
        }
    }

    /// 读出自上次以来的完整行（不完整的尾巴留在 `carry` 里等下轮）。
    pub(super) fn drain(&mut self) -> Vec<String> {
        use std::io::{Read, Seek, SeekFrom};
        let Ok(mut f) = std::fs::File::open(&self.path) else {
            return Vec::new();
        };
        if f.seek(SeekFrom::Start(self.offset)).is_err() {
            return Vec::new();
        }
        let mut buf = Vec::new();
        if f.read_to_end(&mut buf).is_err() || buf.is_empty() {
            return Vec::new();
        }
        self.offset += buf.len() as u64;
        self.carry.extend_from_slice(&buf);
        let mut lines = Vec::new();
        let mut start = 0usize;
        for i in 0..self.carry.len() {
            if self.carry[i] == b'\n' {
                lines.push(
                    String::from_utf8_lossy(&self.carry[start..i])
                        .trim_end_matches('\r')
                        .to_string(),
                );
                start = i + 1;
            }
        }
        self.carry.drain(..start);
        // 极端情况：一直没有换行（比如 ffmpeg 进度条），别让缓冲区无限涨
        if self.carry.len() > 512 * 1024 {
            lines.push(String::from_utf8_lossy(&self.carry).to_string());
            self.carry.clear();
        }
        lines
    }

    pub(super) fn push_tail(&mut self, line: &str) {
        self.tail.push_back(line.to_string());
        while self.tail.len() > 30 {
            self.tail.pop_front();
        }
    }

    pub(super) fn tail_text(&self) -> String {
        self.tail.iter().cloned().collect::<Vec<_>>().join("\n")
    }
}

/// 解析 `frames 0..1991: 45%  120s` / `chunk#2: done  1991 frames` → (标识, 百分比)。
///
/// 标识 = 冒号前那一段（帧区间或 `chunk#N`）：4 个分块各报各的，父进程按**最小值**
/// 聚合成整体进度 —— 最慢的那块决定渲染什么时候结束。
pub(super) fn parse_chunk_progress(line: &str) -> Option<(String, u32)> {
    let after = line
        .split_once("frames ")
        .map(|(_, rest)| rest)
        .or_else(|| line.split_once("chunk").map(|(_, rest)| rest))?;
    let (tag, rest) = after.split_once(':')?;
    let tag = tag.trim();
    if tag.is_empty()
        || tag.len() > 24
        || !tag.chars().all(|c| c.is_ascii_digit() || c == '.' || c == '#')
    {
        return None;
    }
    if rest.contains("done") {
        return Some((tag.to_string(), 100));
    }
    if !rest.contains('%') {
        return None;
    }
    let pct_part = rest.split('%').next()?;
    let digits: String = pct_part.chars().filter(|c| c.is_ascii_digit()).collect();
    Some((tag.to_string(), digits.parse::<u32>().ok()?.min(100)))
}

/// 去掉 tracing 的前缀（时间/级别/target），只留消息本身。
pub(super) fn pretty_render_line(raw: &str) -> String {
    let plain = strip_ansi(raw);
    let is_tracing = |head: &str| {
        head.contains("::")
            || ["INFO", "WARN", "ERROR", "DEBUG", "TRACE"]
                .iter()
                .any(|l| head.contains(l))
    };
    let msg = match plain.split_once(": ") {
        Some((head, tail)) if is_tracing(head) => tail,
        _ => plain.as_str(),
    };
    let trimmed = msg.trim();
    if trimmed.chars().count() > 300 {
        format!("{}…", trimmed.chars().take(300).collect::<String>())
    } else {
        trimmed.to_string()
    }
}

pub(super) async fn emit_render_line(
    jobs: &Arc<crate::jobs::JobManager>,
    job_id: &str,
    out: &mut RenderOutput,
    line: &str,
) {
    let msg = pretty_render_line(line);
    if msg.is_empty() {
        return;
    }
    out.last_active = Some(std::time::Instant::now());
    out.push_tail(&msg);
    if let Some((tag, pct)) = parse_chunk_progress(&msg) {
        out.chunk_pct.insert(tag, pct);
        // 最小的那个 = 最慢的分块 = 整体进度（等最慢的块跑完才算完）
        let slowest = out.chunk_pct.values().copied().min().unwrap_or(0);
        jobs.progress(job_id, slowest as u64, 100).await;
        return; // 每 1.5s 一条的进度行不进日志，靠任务卡的进度条表达
    }
    // 底座那句「--no-audio 无声」是误导（声音由本仓混入），换掉原文
    if is_base_no_audio_notice(&msg) {
        jobs.log(job_id, "info", BASE_NO_AUDIO_REPLACEMENT).await;
        return;
    }
    if msg.contains('⚠') {
        jobs.log(job_id, "warn", &msg).await;
    } else if RENDER_MILESTONES.iter().any(|m| msg.contains(m)) {
        jobs.log(job_id, "info", &msg).await;
    } else {
        jobs.log(job_id, "debug", &msg).await;
    }
}

/// 底座带 `--no-audio` 出画面时，它会对着自己工程里登记的音轨（a0/a1）发一条
/// 「本次出片是无声的」—— 那句话在 YekiTrim 的管线里是**误导**：底座只负责画面，
/// 原声与音效由本仓 ffmpeg 在混音/合流阶段混入，**最终成片有声**。
/// 这里把原文拦下换成准确说法（匹配放宽到"两个特征同时命中"，措辞变了也能接住）。
fn is_base_no_audio_notice(msg: &str) -> bool {
    msg.contains("--no-audio") && msg.contains("无声")
}

const BASE_NO_AUDIO_REPLACEMENT: &str =
    "底座只出画面（--no-audio）：原声与音效由本仓 ffmpeg 在混音/合流阶段混入 —— 最终成片有声。";

/// 边跑边读子进程日志：里程碑进日志、`chunk#N: P%` 进进度条、长时间静默告警。
pub(super) async fn follow_render_output(
    jobs: Arc<crate::jobs::JobManager>,
    job_id: String,
    mut out: RenderOutput,
    stop: Arc<std::sync::atomic::AtomicBool>,
) -> RenderOutput {
    use std::sync::atomic::Ordering;
    loop {
        for line in out.drain() {
            emit_render_line(&jobs, &job_id, &mut out, &line).await;
        }
        if stop.load(Ordering::SeqCst) {
            for line in out.drain() {
                emit_render_line(&jobs, &job_id, &mut out, &line).await;
            }
            return out;
        }
        // 静默告警：子进程会周期性打分块进度，60s 一行都没有 = 很可能卡住了。
        // 这正是过去"渲染长时间无反应、日志里却什么都没有"的那段时间。
        let quiet = out.last_active.map(|t| t.elapsed().as_secs()).unwrap_or(0);
        if quiet >= 60
            && out
                .warned_silent
                .map(|w| w.elapsed().as_secs() >= 60)
                .unwrap_or(true)
        {
            out.warned_silent = Some(std::time::Instant::now());
            jobs.log(
                &job_id,
                "warn",
                format!("渲染子进程已 {quiet}s 无任何输出（可能卡在抽帧或某一分块上）"),
            )
            .await;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

/// 渲染中途失败/取消后清掉半成品：`_chunk_*.mp4` / `_temp_*`（成片与封面不动）。
pub(super) fn cleanup_render_leftovers(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with("_chunk_") || name.starts_with("_temp_") {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

/// 跑一个 `yekitrim polish <sub>` 子进程：输出**落文件**并流式回灌任务日志。
///
/// 为什么不是 `.output()`：子进程拉起的 ffmpeg（以及底座）会继承 stdout 句柄，只要还有一个
/// 活着，管道就永远不 EOF —— 子进程早就失败退出了，任务却永远停在"运行中"，连错误都看不到
/// （实测挂过两次，每次都留下 3 个渲染孙进程空转两小时）。落文件 + `wait()`
/// 判定退出就没有这个问题。
///
/// `extra` 是**透传给子命令的额外参数**（`render` / `cover` 用：`["--renderer", "dhampir"|"takumi"]`）。
/// 不放宽成任意参数的入口 —— 这个函数是"服务端起自己的一个 polish 子命令"，
/// 参数面越窄越好；白名单在 handler 那一侧（`polish_render`）。
///
/// 取消：子进程是**另一个进程**，看不到我们的进程内标志 —— 由 `cancel_watch`
/// 按 pid 杀掉整棵树（含 ffmpeg 与底座拉起的孙进程）。
pub(super) async fn run_polish_child(
    data_dir: &Path,
    jobs: &Arc<crate::jobs::JobManager>,
    job_id: &str,
    sub: &str,
    dir: &Path,
    extra: &[String],
) -> anyhow::Result<()> {
    use std::sync::atomic::{AtomicBool, Ordering};

    let log_path = crate::render_proc::log_file(data_dir, job_id);
    if let Some(parent) = log_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let file = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .append(true)
        .open(&log_path)
        .map_err(|e| anyhow::anyhow!("创建渲染日志失败 {}: {e}", log_path.display()))?;
    let err_file = file
        .try_clone()
        .map_err(|e| anyhow::anyhow!("复制日志句柄失败: {e}"))?;

    let exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("yekitrim"));
    let dir_str = dir.to_string_lossy().into_owned();
    let mut args: Vec<String> = vec!["polish".into(), sub.into(), dir_str];
    args.extend(extra.iter().cloned());
    let mut child = tokio::process::Command::new(&exe)
        .args(&args)
        .env("NO_COLOR", "1")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::from(file))
        .stderr(std::process::Stdio::from(err_file))
        .spawn()
        .map_err(|e| anyhow::anyhow!("{sub} 进程启动失败: {e}"))?;
    let pid = child.id().unwrap_or(0);
    if pid != 0 {
        crate::render_proc::register(data_dir, job_id, pid);
    }
    jobs.log(
        job_id,
        "info",
        format!("{sub} 子进程已启动（pid {pid}）；完整输出: {}", log_path.display()),
    )
    .await;

    let stop = Arc::new(AtomicBool::new(false));
    let follow = tokio::spawn(follow_render_output(
        jobs.clone(),
        job_id.to_string(),
        RenderOutput::new(log_path.clone()),
        stop.clone(),
    ));
    let cancel_flag = jobs.cancel_token(job_id).await;
    let watch_stop = stop.clone();
    let cancel_watch = tokio::spawn(async move {
        let Some(flag) = cancel_flag else { return };
        if pid == 0 {
            return;
        }
        loop {
            if watch_stop.load(Ordering::SeqCst) {
                return;
            }
            if flag.load(Ordering::SeqCst) {
                tracing::warn!("渲染任务被取消，结束子进程树 pid {pid}");
                crate::render_proc::kill_tree(pid);
                return;
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    });

    let status = child
        .wait()
        .await
        .map_err(|e| anyhow::anyhow!("等待 {sub} 进程失败: {e}"))?;
    stop.store(true, Ordering::SeqCst);
    crate::render_proc::unregister(data_dir, job_id);
    let out = follow.await.unwrap_or_default();
    cancel_watch.abort();

    if !status.success() {
        cleanup_render_leftovers(dir);
        let tail = out.tail_text();
        anyhow::bail!("{sub} 进程退出码: {:?}\n{}", status.code(), tail);
    }
    // 成功只报一句状态，**不带路径**（2026-09-21 用户要求）：写出的是
    // `\\?\E:\media\…\vtuber-clip_1.mp4` 这种 verbatim 长路径，塞进任务日志会撑满整行，
    // 而任务框只有一行高度。实际文件名在精修页「产物」区（`rendered_file` / `cover_file`，
    // 含 `_1`/`_2` 版本）与切片卡片上都能看到，日志不必重复。
    // `subprocess_note` 仍然要跑：它是「子进程真的写出了文件」的证据，不是装饰。
    let tail = out.tail_text();
    let (needle, label) = if sub == "cover" {
        ("Cover saved:", "封面已写出")
    } else {
        ("FINISHED:", "成片已写出")
    };
    if subprocess_note(&tail, needle).is_some() {
        jobs.log(job_id, "info", label).await;
    }
    Ok(())
}







#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::test_support::*;
    use crate::jobs::{JobClass, JobSpec};




/// 渲染进度行解析：4 个分块各报各的，父进程按最小值聚合。
///
/// 行格式直接取自真实渲染日志（`frames A..B: NN%  Ns`）。
#[test]
fn render_progress_lines_are_parsed() {
    let (tag, pct) = parse_chunk_progress("    frames 0..1969: 45%  120s").unwrap();
    assert_eq!(tag, "0..1969");
    assert_eq!(pct, 45);
    let (tag2, _) = parse_chunk_progress("    frames 1969..3938: 3%  4s").unwrap();
    assert_ne!(tag, tag2, "不同分块必须是不同的键，否则进度会互相覆盖");

    // 分块收尾行 = 100%
    let (tag3, pct3) = parse_chunk_progress("    chunk#0: done  1969 frames（抽帧 23.2s / 渲染 145.0s / 编码 8.5s）").unwrap();
    assert!(tag3.starts_with('#'));
    assert_eq!(pct3, 100);

    // 普通行不能被当成进度
    assert!(parse_chunk_progress("  -- Render chunks (4) --").is_none());
    assert!(parse_chunk_progress("  v 9 sound events  ⏱ 3.4s").is_none());
    assert!(parse_chunk_progress("    chunk#0: extracting 1969 frames via ffmpeg...").is_none());
}

/// 子进程日志的增量读取：半行要留在缓冲区，不能读一半就当一条日志发出去。
#[test]
fn render_output_drains_complete_lines_only() {
    let path = std::env::temp_dir().join(format!("yekitrim-render-out-{}.log", std::process::id()));
    let _ = std::fs::remove_file(&path);
    let mut out = RenderOutput::new(path.clone());

    std::fs::write(&path, b"first\nsecond-part").unwrap();
    assert_eq!(out.drain(), vec!["first".to_string()]);

    // 补上后半行 + 新的一行
    std::fs::write(&path, b"first\nsecond-part\nthird\n").unwrap();
    assert_eq!(
        out.drain(),
        vec!["second-part".to_string(), "third".to_string()]
    );
    assert!(out.drain().is_empty(), "没有新内容时不该重复吐行");

    // 失败文案取尾部若干行
    out.push_tail("a");
    out.push_tail("b");
    assert_eq!(out.tail_text(), "a\nb");
    let _ = std::fs::remove_file(&path);
}

/// 流式跟随：里程碑行进任务日志（info，会进运行日志面板），进度行进 progress 事件。
///
/// 这是"渲染期间日志一行没有"的直接修复点 —— 断言它在**子进程还在跑**的时候就有输出。
#[tokio::test]
async fn follow_render_output_emits_logs_and_progress_before_exit() {
    let jobs = Arc::new(crate::jobs::JobManager::new());
    let handle = jobs
        .create(1, JobSpec::new("polish_render", "polish render", vec![], JobClass::Heavy))
        .await;
    let id = handle.id.clone();
    handle.wait_start().await.expect("资源不相干，应该直接开跑");

    let path = std::env::temp_dir().join(format!("yekitrim-follow-{}.log", std::process::id()));
    let _ = std::fs::remove_file(&path);
    std::fs::write(
        &path,
        format!(
            "{}  INFO yekitrim_polish::render::pipeline:   -- Render chunks (4) --\n\
             {}  INFO yekitrim_polish::render::dhampir:    frames 0..1969: 45%  120s\n\
             {}  INFO dhampir::render: Starting render\n",
            now_stamp(),
            now_stamp(),
            now_stamp()
        ),
    )
    .unwrap();

    let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let follow = tokio::spawn(follow_render_output(
        jobs.clone(),
        id.clone(),
        RenderOutput::new(path.clone()),
        stop.clone(),
    ));
    // 让它至少跑一轮
    tokio::time::sleep(Duration::from_millis(700)).await;
    stop.store(true, std::sync::atomic::Ordering::SeqCst);
    let out = follow.await.unwrap();

    let summary = jobs.get(&id).await.expect("任务存在");
    assert_eq!(summary.current, 45, "分块进度要进 progress");
    assert_eq!(summary.total, 100);
    let logs: Vec<String> = summary
        .logs
        .iter()
        .map(|l| l.message.clone())
        .collect();
    assert!(
        logs.iter().any(|l| l.contains("-- Render chunks")),
        "里程碑行要进日志: {logs:?}"
    );
    assert!(
        logs.iter().any(|l| l.contains("Starting render")),
        "其余行进 debug 日志: {logs:?}"
    );
    assert!(
        logs.iter().all(|l| !l.contains("45%")),
        "进度行不该重复进日志: {logs:?}"
    );
    assert!(out.tail_text().contains("Starting render"), "尾部要有最后一行");
    let _ = std::fs::remove_file(&path);
}

    /// 底座对着自己工程音轨发的那句「--no-audio 无声」在 YekiTrim 管线里是误导
    /// （声音由本仓 ffmpeg 混入，**成片有声**）—— 必须被换成准确说法，原文不许进日志。
    #[tokio::test]
    async fn base_no_audio_notice_is_replaced_not_passed_through() {
        let jobs = Arc::new(crate::jobs::JobManager::new());
        let handle = jobs
            .create(1, JobSpec::new("polish_render", "polish render", vec![], JobClass::Heavy))
            .await;
        let id = handle.id.clone();
        handle.wait_start().await.expect("资源不相干，应该直接开跑");
        let mut out = RenderOutput::new(
            std::env::temp_dir().join(format!("yekitrim-noaudio-{}.log", std::process::id())),
        );
        emit_render_line(
            &jobs,
            &id,
            &mut out,
            "注意：你给了 --no-audio，本次出片是无声的（工程里有音轨：a0, a1）。",
        )
        .await;
        let summary = jobs.get(&id).await.expect("任务存在");
        let logs: Vec<String> = summary.logs.iter().map(|l| l.message.clone()).collect();
        assert!(logs.iter().any(|l| l.contains("最终成片有声")), "要回准确的说明: {logs:?}");
        assert!(logs.iter().all(|l| !l.contains("无声的")), "误导原文不许进日志: {logs:?}");
    }
}
