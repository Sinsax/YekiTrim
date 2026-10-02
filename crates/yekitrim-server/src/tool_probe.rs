//! 外部命令探测（进程级 30s TTL 缓存）。
//!
//! **为什么是独立模块**：`/api/health`、`vtrim doctor` 与**任务 worker** 都要探外部工具
//! （ffmpeg / ffprobe / yt-dlp；chrome 那个探针随 headless Chrome 链路一起删除了）。
//! 这几个函数原先住在 `api/mod.rs` 里，
//! 于是 worker 想探一个命令就得 `use crate::api`（web 层）—— 那是**反向依赖**。
//! 现在它是叶子模块：两边都用，谁也不用认识谁。
//!
//! 这里**只提供"跑一个进程取第一行 stdout"的通用能力，不认任何具体工具**：
//! 版本参数由调用方显式给出（ffmpeg/ffprobe 是 `-version`，yt-dlp 是 `--version`）。

use std::time::Duration;

/// 探测结果缓存。`Instant::now()` 不是 const → 只能惰性建（`OnceLock`）。
pub(crate) type ProbeCache =
    std::sync::Mutex<std::collections::HashMap<String, (std::time::Instant, Option<String>)>>;

pub(crate) fn probe_cache() -> &'static ProbeCache {
    static CACHE: std::sync::OnceLock<ProbeCache> = std::sync::OnceLock::new();
    CACHE.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()))
}

pub(crate) fn cache_get(key: &str, ttl: Duration) -> Option<Option<String>> {
    let cache = probe_cache().lock().unwrap_or_else(|e| e.into_inner());
    cache
        .get(key)
        .filter(|(t, _)| t.elapsed() < ttl)
        .map(|(_, v)| v.clone())
}

pub(crate) fn cache_put(key: String, value: Option<String>) {
    let mut cache = probe_cache().lock().unwrap_or_else(|e| e.into_inner());
    cache.insert(key, (std::time::Instant::now(), value));
}

/// 同步探测：跑 `cmd <version_arg>`，成功则返回 stdout 第一行。
///
/// ⚠️ **`version_arg` 必须由调用方给，别退回写死**。早先这里写死 `-version`，
/// 而 yt-dlp 只认 `--version` —— 后果是"明明装好了却被判成未安装"，
/// 且失败表现是 `None`（静默降级成"环境检查缺项"），很难归因。
///
/// 判据：对不存在的命令返回 `None`；对存在但参数不对的命令同样返回 `None`
/// （`-version` 在 yt-dlp 上是硬错误，退出码非 0）。
pub(crate) fn command_available_blocking(cmd: &str, version_arg: &str) -> Option<String> {
    let output = std::process::Command::new(cmd)
        .arg(version_arg)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let first = String::from_utf8_lossy(&output.stdout)
        .lines()
        .next()
        .map(|s| s.to_string())
        .unwrap_or_default();
    Some(first)
}

/// 带缓存的异步探测（探测放 `spawn_blocking`，避免占住 tokio worker）。
///
/// 缓存键含 `version_arg`：同一程序用不同参数探测是两个不同的问题。
pub(crate) async fn command_available(cmd: &str, version_arg: &str) -> Option<String> {
    let key = format!("cmd:{cmd}:{version_arg}");
    if let Some(cached) = cache_get(&key, Duration::from_secs(30)) {
        return cached;
    }
    let owned = cmd.to_string();
    let arg = version_arg.to_string();
    let value = tokio::task::spawn_blocking(move || command_available_blocking(&owned, &arg))
        .await
        .ok()
        .flatten();
    cache_put(key, value.clone());
    value
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 不存在的命令 → `None`，且不 panic。
    #[test]
    fn missing_command_is_none() {
        assert!(command_available_blocking("vtrim-no-such-cmd-xyz", "--version").is_none());
    }

    /// ⚠️ 回归钉子：**版本参数传错必须返回 `None`**。
    ///
    /// 拿 python 当例子（实测：`python --version` exit 0，`python -version` exit 2），
    /// 形状与 yt-dlp 那个坑完全一致。这一条挡住"把 version_arg 写死成 `-version`"的回退。
    /// 本机没有 `python` 时跳过 —— 不把环境依赖变成假失败。
    #[test]
    fn wrong_version_arg_is_none() {
        if command_available_blocking("python", "--version").is_none() {
            return;
        }
        assert!(
            command_available_blocking("python", "-version").is_none(),
            "`-version` 不是 python 的版本参数，必须判为不可用"
        );
    }
}
