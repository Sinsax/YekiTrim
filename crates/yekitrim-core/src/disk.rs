//! 磁盘空间查询与**阶段预检**(零依赖:Windows `GetDiskFreeSpaceExW` / 其它平台 `df -k`)。
//!
//! 2026-11-15 事故的产物:一键全切把数据盘写穿(47 GB → 13.88 GB),机器上的外部磁盘
//! 看门狗(diskguard,min_free 15 GB)按「最旧优先」删掉了 6 场录播的源文件,其中两场
//! 正是正在处理的工程输入。管线从这次起有了自己的预检:任何阶段开工前
//! (`pipeline::PipelineRunner`)与切片的每个批次边界(`cutting::Cutter`)都会确认
//! `output_dir` 所在卷还有 `[cutting] min_free_gb` 的余量,不足就**停止整个阶段** ——
//! 宁可少剪,也绝不把盘写穿、把源文件拱手送给看门狗。
//!
//! 预检**探测失败时放行**(fail-open):守卫的职责是拦"可预见的写穿",
//! 不是因为 `df` 缺失 / 特殊挂载把整条管线卡死 —— 与 `service::disk`(doctor 用)同一口径。

use std::fmt;
use std::path::Path;

use anyhow::Result;

/// 目标路径所在卷的 `(可用字节, 总字节)`。
#[cfg(windows)]
pub fn space_bytes(path: &Path) -> Option<(u64, u64)> {
    #[link(name = "kernel32")]
    extern "system" {
        fn GetDiskFreeSpaceExW(
            lpdirectoryname: *const u16,
            lpfreebytesavailabletocaller: *mut u64,
            lptotalnumberofbytes: *mut u64,
            lptotalnumberoffreebytes: *mut u64,
        ) -> i32;
    }

    let mut wide: Vec<u16> = path.as_os_str().to_string_lossy().encode_utf16().collect();
    wide.push(0);
    let mut avail: u64 = 0;
    let mut total: u64 = 0;
    let mut free: u64 = 0;
    // SAFETY: 传入的是以 NUL 结尾的 UTF-16 路径与三个有效可写指针
    let ok = unsafe { GetDiskFreeSpaceExW(wide.as_ptr(), &mut avail, &mut total, &mut free) };
    if ok == 0 {
        None
    } else {
        Some((avail, total))
    }
}

/// 非 Windows:解析 `df -k <path>` 的第 2 行(可用 KB / 总 KB)。
#[cfg(not(windows))]
pub fn space_bytes(path: &Path) -> Option<(u64, u64)> {
    let out = crate::process::cmd("df").arg("-k").arg(path).output().ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let line = text.lines().nth(1)?;
    let cols: Vec<&str> = line.split_whitespace().collect();
    // Filesystem 1K-blocks Used Available Capacity Mounted-on
    let total: u64 = cols.get(1)?.parse().ok()?;
    let avail: u64 = cols.get(3)?.parse().ok()?;
    Some((avail.saturating_mul(1024), total.saturating_mul(1024)))
}

/// 人类可读字节数(`1.2 GB` / `512 MB` / `900 B`)。
pub fn human_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit + 1 < UNITS.len() {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

/// 磁盘预检未过的**类型化标记**:批量调用方(一键全切)据此把这类失败与
/// 真正的"剪切失败"分开 —— 前者是「没让剪」,要停整批;后者照旧逐条汇总。
#[derive(Debug)]
pub struct FreeSpaceError {
    /// 预检时的可用字节。
    pub free: u64,
    /// 要求保留的字节(`[cutting] min_free_gb` 换算)。
    pub min: u64,
}

impl fmt::Display for FreeSpaceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "磁盘空间不足:可用 {},需保留 {}",
            human_bytes(self.free),
            human_bytes(self.min)
        )
    }
}

impl std::error::Error for FreeSpaceError {}

/// **阶段预检**:`dir` 所在卷的可用空间必须 ≥ `min_bytes`。
///
/// - `min_bytes == 0` = 预检关闭(`[cutting] min_free_gb = 0`),恒过;
/// - 探测失败(**fail-open**)→ 放行 + warn,不因拿不到空间信息卡死管线;
/// - 不达标 → `Err`,内层是 [`FreeSpaceError`](**可 downcast**,调用方据此停整批),
///   外层 context 带目录与两侧读数。
pub fn ensure_free(dir: &Path, min_bytes: u64) -> Result<()> {
    if min_bytes == 0 {
        return Ok(());
    }
    let Some((avail, _total)) = space_bytes_of_nearest_existing(dir) else {
        tracing::warn!("磁盘空间预检不可用({}),放行", dir.display());
        return Ok(());
    };
    if avail >= min_bytes {
        return Ok(());
    }
    Err(anyhow::Error::new(FreeSpaceError { free: avail, min: min_bytes })
        .context(format!("{} 所在卷预检未过", dir.display())))
}

/// `dir` 可能**尚不存在**(新工程的 `output/` 就是这样)—— `df` 对不存在的路径会失败，
/// 预检会在最需要它的新工程上静默失效。所以向上找**最近的已存在祖先**（同一卷）再探测。
fn space_bytes_of_nearest_existing(dir: &Path) -> Option<(u64, u64)> {
    let mut probe = Some(dir.to_path_buf());
    while let Some(p) = probe {
        if p.exists() {
            return space_bytes(&p);
        }
        probe = p.parent().map(|x| x.to_path_buf());
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn space_bytes_reports_something_or_none() {
        // 不强制要求平台支持:拿到值时必须是"可用 ≤ 总量"
        if let Some((avail, total)) = space_bytes(&std::env::temp_dir()) {
            assert!(total > 0);
            assert!(avail <= total, "avail={avail} total={total}");
        }
    }

    #[test]
    fn human_bytes_formats_units() {
        assert_eq!(human_bytes(512), "512 B");
        assert_eq!(human_bytes(2048), "2.0 KB");
        assert_eq!(human_bytes(5 * 1024 * 1024), "5.0 MB");
        assert_eq!(human_bytes(3 * 1024 * 1024 * 1024), "3.0 GB");
    }

    #[test]
    fn ensure_free_disabled_by_zero() {
        let dir = std::env::temp_dir();
        ensure_free(&dir, 0).expect("min_bytes = 0 ⇒ 预检关闭,恒过");
    }

    #[test]
    fn ensure_free_blocks_and_types_the_error_when_min_exceeds_free() {
        let dir = std::env::temp_dir();
        let err = ensure_free(&dir, u64::MAX).expect_err("阈值 u64::MAX ⇒ 必不足");
        let typed = err
            .downcast_ref::<FreeSpaceError>()
            .expect("必须能 downcast 成 FreeSpaceError,批量调用方靠它分类");
        assert_eq!(typed.min, u64::MAX);
        assert!(typed.free < typed.min);
        // ⚠️ 查**整条错误链**：to_string() 只有最外层 context（"...所在卷预检未过"）
        let text = format!("{err:#}");
        assert!(text.contains("磁盘空间不足"), "{text}");
        assert!(text.contains("需保留"), "{text}");
    }

    #[test]
    fn ensure_free_passes_on_healthy_disk() {
        // 真实机器的 temp 盘拿不出 u64::MAX,但拿得出 1 KB(除非真的满了 —— 那样这条失败也合理)
        let dir = std::env::temp_dir();
        ensure_free(&dir, 1024).expect("temp 盘应有 ≥1KB 余量");
    }
}
