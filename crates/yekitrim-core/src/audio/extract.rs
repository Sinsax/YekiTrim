//! Stage 1 的子进程面：ffprobe 读 `VideoInfo` + ffmpeg 提取 16kHz mono WAV。
//!
//! 进度口径（docs/plan/job-progress-spec-2026-11-15.md P1②）见 `extract_audio`；
//! `-progress` 行解析与节流都是本模块内的纯函数，单测钉住空行 / 坏行 /
//! `out_time_us`-`out_time_ms` 两种字段 / 节流口径 / 分母守卫。

use std::io::{BufRead, BufReader, Read};
use std::path::Path;
use std::process::Stdio;

use anyhow::{Context, Result};

use crate::progress::ProgressReporter;
use crate::types::VideoInfo;

/// 失败信息里保留的 stderr **尾部**长度（字符）：ffmpeg 的报错在输出末尾，
/// 而这条消息会进任务库/弹给前端 —— 别把整段几 MB 的输出塞进去。
const STDERR_TAIL_CHARS: usize = 2000;

/// 从 ffmpeg `-progress` 的 `key=value` 行解析**已处理秒数**。
///
/// 只认 `out_time_us` / `out_time_ms` 两个键，**都按微秒换算** ——
/// ⚠️ `out_time_ms` 名不副实：它装的是微秒，与 `out_time_us` 同值（实测 ffmpeg
/// n9.0.2 对 3s 输出两键都是 `3000000`；旧版只发 `out_time_ms`）。按毫秒换算会把
/// 进度放大 1000 倍冲爆分母。见 ffmpeg trac #7553。
///
/// 其余行（`frame` / `total_size` / `speed` / `progress=continue|end` /
/// `out_time=HH:MM:SS` 串）返回 `None`，让节流层静默跳过，不当成错误。
pub fn parse_out_time_secs(line: &str) -> Option<f64> {
    let (key, value) = line.split_once('=')?;
    let micros: i64 = match key.trim() {
        "out_time_us" | "out_time_ms" => value.trim().parse().ok()?,
        _ => return None,
    };
    if micros < 0 {
        return None;
    }
    Some(micros as f64 / 1_000_000.0)
}

/// 把「已处理秒数 + 总秒数」换算成 `stage_progress` 的整数对 `(current, total)`。
///
/// 分母向下取整；`total <= 0`（含 NaN）或取整后为 0（<1s 的视频）→ `None`：
/// **没有可靠分母就不报数值进度**，绝不拿 0 当分母。`current` 向下取整、非负、
/// 并夹回分母内（ffmpeg 的 `out_time` 尾部可能比 ffprobe 时长多出容器取整差）。
fn progress_pair(secs: f64, total: f64) -> Option<(u64, u64)> {
    let total = total.floor();
    if !(total > 0.0) {
        return None;
    }
    let total = total as u64;
    let current = secs.floor().max(0.0) as u64;
    Some((current.min(total), total))
}

/// 阶段内进度节流（spec §1 阶段内节奏「≥每 5s 或每变化 ≥1% 至少一个事件」的上限面）：
/// **同一秒且同一百分比**的重复事件不上报 —— 秒或百分比任一变化就放行。
/// （反面读法「两个都变才报」在长视频上会饿死节奏下限：1% = 几十秒无事件。）
struct ProgressThrottle {
    /// 进度分母（ffprobe 总秒数）；`None` = 拿不到时长，整场不报数值进度。
    total: Option<f64>,
    /// 上次已上报的 `(已处理秒, 整数百分比)`。
    last: Option<(u64, u64)>,
}

impl ProgressThrottle {
    fn new(total: Option<f64>) -> Self {
        Self { total, last: None }
    }

    /// 喂一行 `-progress` 输出；需要上报时返回 `Some((current, total))`。
    fn step_line(&mut self, line: &str) -> Option<(u64, u64)> {
        self.report_secs(parse_out_time_secs(line)?)
    }

    /// 节流判定本体（抽出来单测）：`secs` = 本行解析出的已处理秒数。
    fn report_secs(&mut self, secs: f64) -> Option<(u64, u64)> {
        let total_secs = self.total?;
        let (current, total) = progress_pair(secs, total_secs)?;
        // 百分比按**原始秒数**算（不是取整后的 current）：同一秒内跨过 1% 也要放行，
        // 否则百分比成了 current 的派生量、两个去重维度塌成一个。
        let percent = ((secs / total_secs) * 100.0)
            .floor()
            .clamp(0.0, 100.0) as u64;
        let key = (current, percent);
        if self.last == Some(key) {
            return None;
        }
        self.last = Some(key);
        Some((current, total))
    }
}

/// 取 `s` 的尾部最多 `max_chars` 个字符（按字符数截、落回字符边界）。
fn stderr_tail(s: &str, max_chars: usize) -> &str {
    let total = s.chars().count();
    if total <= max_chars {
        return s;
    }
    let start = s
        .char_indices()
        .nth(total - max_chars)
        .map(|(i, _)| i)
        .unwrap_or(0);
    &s[start..]
}

/// 用 ffmpeg 从视频提取 16kHz mono WAV。
///
/// 进度口径（P1②）：
/// * ffmpeg 加 `-progress pipe:1 -nostats`，从 stdout 读 `key=value` 行换算已处理秒数，
///   经节流后报 `progress.stage_progress(已处理秒, 总秒)`；
/// * 分母 = `total_secs`（ffprobe 时长，由 `AudioExtractor::run` 传入）；`None` =
///   拿不到时长，**整场不报数值进度**（保持零进度的现状语义，绝不拿 0 当分母）。
///
/// ⚠️ **双管道死锁**：stdout（`-progress`）与 stderr 都被接管后必须**并发排空** ——
/// stderr 丢进专用线程 `read_to_end` 留作失败原因，本线程逐行读 stdout。顺序读的话，
/// 长视频里 stderr 塞满管道缓冲区会把 ffmpeg 卡死，`lines()` 永远等不到 EOF。
///
/// ⚠️ **阻塞模型**：管线体跑在专用线程上，本函数保持同步实现（进度桥负责把事件落到库）；
/// 别改成 async。取消语义不变：进行中的单次 ffmpeg 调用不被中断。
pub fn extract_audio(
    input: &Path,
    output_path: &Path,
    progress: &dyn ProgressReporter,
    total_secs: Option<f64>,
) -> Result<()> {
    let mut child = crate::process::cmd("ffmpeg")
        .arg("-y")
        .arg("-nostats")
        .arg("-loglevel")
        .arg("error")
        .arg("-progress")
        .arg("pipe:1")
        .arg("-i")
        .arg(input)
        .arg("-vn")
        .arg("-acodec")
        .arg("pcm_s16le")
        .arg("-ar")
        .arg("16000")
        .arg("-ac")
        .arg("1")
        .arg(output_path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("ffmpeg 未找到，请确保 ffmpeg 已在 PATH 中")?;

    // stderr 并发排空：read_to_end 一直读到 EOF（= 进程退出），字节留作失败原因。
    let stderr = child.stderr.take().context("无法接管 ffmpeg 的 stderr")?;
    let stderr_reader = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let mut reader = BufReader::new(stderr);
        let _ = reader.read_to_end(&mut buf);
        buf
    });

    let stdout = child.stdout.take().context("无法接管 ffmpeg 的 stdout")?;
    let mut throttle = ProgressThrottle::new(total_secs);
    for line in BufReader::new(stdout).lines() {
        let Ok(line) = line else { break };
        if let Some((current, total)) = throttle.step_line(&line) {
            progress.stage_progress(current, total);
        }
    }

    let stderr_bytes = stderr_reader.join().unwrap_or_default();
    let status = child.wait().context("等待 ffmpeg 退出失败")?;
    if !status.success() {
        let stderr = String::from_utf8_lossy(&stderr_bytes);
        anyhow::bail!(
            "ffmpeg 音频提取失败: {}",
            stderr_tail(stderr.trim(), STDERR_TAIL_CHARS)
        );
    }
    Ok(())
}

/// 用 ffprobe 获取视频信息
pub fn get_video_info(input: &Path) -> Result<VideoInfo> {
    let output = crate::process::cmd("ffprobe")
        .arg("-v")
        .arg("quiet")
        .arg("-print_format")
        .arg("json")
        .arg("-show_format")
        .arg("-show_streams")
        .arg(input)
        .output()
        .context("ffprobe 未找到，请确保 ffprobe 已在 PATH 中")?;

    if !output.status.success() {
        anyhow::bail!("ffprobe 失败 (exit: {})", output.status);
    }

    let probe: ProbeOutput = serde_json::from_slice(&output.stdout)
        .context("无法解析 ffprobe 输出")?;

    let fmt_name = probe.format.as_ref().map(|f| f.format_name.clone()).unwrap_or_default();
    let video = probe.streams.iter().find(|s| s.codec_type == "video");
    let audio = probe.streams.iter().find(|s| s.codec_type == "audio");

    Ok(VideoInfo {
        duration_secs: probe
            .format
            .as_ref()
            .and_then(|f| f.duration.parse::<f64>().ok())
            .unwrap_or(0.0),
        width: video.map(|s| s.width).unwrap_or(0),
        height: video.map(|s| s.height).unwrap_or(0),
        video_codec: video.map(|s| s.codec_name.clone()).unwrap_or_default(),
        audio_codec: audio.map(|s| s.codec_name.clone()).unwrap_or_default(),
        format: fmt_name,
    })
}

#[derive(serde::Deserialize)]
struct ProbeOutput {
    streams: Vec<StreamInfo>,
    format: Option<FormatInfo>,
}

#[derive(serde::Deserialize)]
struct StreamInfo {
    codec_type: String,
    codec_name: String,
    #[serde(default)]
    width: u32,
    #[serde(default)]
    height: u32,
}

#[derive(serde::Deserialize)]
struct FormatInfo {
    format_name: String,
    duration: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- parse_out_time_secs：空行 / 坏行 / us 与 ms 两种字段 ----

    #[test]
    fn parse_rejects_empty_and_malformed_lines() {
        assert_eq!(parse_out_time_secs(""), None);
        assert_eq!(parse_out_time_secs("garbage-no-equals"), None);
        assert_eq!(parse_out_time_secs("=12"), None);
        assert_eq!(parse_out_time_secs("out_time_us"), None);
    }

    #[test]
    fn parse_rejects_bad_values_and_unrelated_keys() {
        assert_eq!(parse_out_time_secs("out_time_us="), None);
        assert_eq!(parse_out_time_secs("out_time_us=abc"), None);
        assert_eq!(parse_out_time_secs("out_time_us=-5"), None);
        assert_eq!(parse_out_time_secs("progress=continue"), None);
        assert_eq!(parse_out_time_secs("progress=end"), None);
        assert_eq!(parse_out_time_secs("out_time=00:00:03.000000"), None);
        assert_eq!(parse_out_time_secs("total_size=48078"), None);
        assert_eq!(parse_out_time_secs("frame=42"), None);
        assert_eq!(parse_out_time_secs("speed=453x"), None);
        assert_eq!(parse_out_time_secs("bitrate=128.2kbits/s"), None);
    }

    /// ⚠️ `out_time_ms` 名不副实：装的是**微秒**（实测 ffmpeg n9.0.2 两个键同值，
    /// 旧版只发 `out_time_ms`）。两个键都必须按微秒换算 —— 按毫秒会把 37s 算成
    /// 37000s 冲爆分母。
    #[test]
    fn parse_reads_both_out_time_fields_as_micros() {
        assert!((parse_out_time_secs("out_time_us=3000000").unwrap() - 3.0).abs() < 1e-9);
        assert!((parse_out_time_secs("out_time_ms=3000000").unwrap() - 3.0).abs() < 1e-9);
        assert!((parse_out_time_secs("out_time_us=37000000").unwrap() - 37.0).abs() < 1e-9);
        assert!((parse_out_time_secs("out_time_ms=37000000").unwrap() - 37.0).abs() < 1e-9);
    }

    // ---- progress_pair：分母守卫与夹取 ----

    #[test]
    fn progress_pair_guards_the_denominator() {
        assert_eq!(progress_pair(37.0, 192.4), Some((37, 192)));
        assert_eq!(progress_pair(0.0, 300.0), Some((0, 300)));
        assert_eq!(progress_pair(-3.0, 300.0), Some((0, 300)));
        // out_time 尾部超出 ffprobe 时长（容器取整差）→ 夹回分母内
        assert_eq!(progress_pair(192.9, 192.4), Some((192, 192)));
        // 分母拿不到 / ≤0 / <1s（取整为 0）→ 不报数值进度，绝不拿 0 当分母
        assert_eq!(progress_pair(37.0, 0.0), None);
        assert_eq!(progress_pair(37.0, -5.0), None);
        assert_eq!(progress_pair(37.0, 0.8), None);
        assert_eq!(progress_pair(37.0, f64::NAN), None);
    }

    // ---- 节流：同一秒且同一百分比的重复事件不上报 ----

    #[test]
    fn throttle_drops_duplicates_of_same_second_and_percent() {
        let mut t = ProgressThrottle::new(Some(300.0));
        assert_eq!(t.report_secs(10.4), Some((10, 300)));
        // 同一秒 + 同一百分比 → 压掉
        assert_eq!(t.report_secs(10.6), None);
        // 秒变了 → 放行
        assert_eq!(t.report_secs(11.0), Some((11, 300)));
    }

    /// 同一秒内跨过 1% 也要放行 —— 百分比按原始秒数算，否则长视频上
    /// 节奏下限（spec §1）会被"只认秒变化"饿死。
    #[test]
    fn throttle_passes_percent_change_within_the_same_second() {
        let mut t = ProgressThrottle::new(Some(50.0));
        assert_eq!(t.report_secs(10.0), Some((10, 50))); // 20%
        assert_eq!(t.report_secs(10.4), None); // 仍 20%，同秒同百分比
        assert_eq!(t.report_secs(10.5), Some((10, 50))); // 21%，秒未变也放行
    }

    /// 拿不到时长（分母 `None`）→ 整场不报数值进度，保持零进度的现状语义。
    #[test]
    fn throttle_without_a_denominator_never_reports() {
        let mut t = ProgressThrottle::new(None);
        assert_eq!(t.step_line("out_time_us=30000000"), None);
        assert_eq!(t.report_secs(30.0), None);
    }

    /// `step_line` = 解析 + 节流的组合口径；无关键不动节流状态。
    #[test]
    fn step_line_parses_and_throttles_together() {
        let mut t = ProgressThrottle::new(Some(300.0));
        assert_eq!(t.step_line("out_time_us=30000000"), Some((30, 300)));
        assert_eq!(t.step_line("total_size=48078"), None);
        assert_eq!(t.step_line("out_time_us=30000000"), None); // 同值重复压掉
        assert_eq!(t.step_line("out_time_ms=31000000"), Some((31, 300)));
        assert_eq!(t.step_line("progress=end"), None);
    }

    // ---- stderr_tail ----

    #[test]
    fn stderr_tail_keeps_short_strings_whole() {
        assert_eq!(stderr_tail("abc", 10), "abc");
        assert_eq!(stderr_tail("", 10), "");
        assert_eq!(stderr_tail("abc", 3), "abc");
    }

    #[test]
    fn stderr_tail_cuts_at_char_boundary() {
        let s = "错误信息很长很长很长";
        assert_eq!(stderr_tail(s, 4), "很长很长");
        // 任意长度都不 panic（中文字符 3 字节，按字节截会炸在字符边界上）
        for n in 0..=s.chars().count() {
            let _ = stderr_tail(s, n);
        }
    }
}
