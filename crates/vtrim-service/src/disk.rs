//! 磁盘空间与可写性探测（`vtrim doctor` 用）。
//!
//! 不引入额外依赖：Windows 直接调 `GetDiskFreeSpaceExW`，其它平台退回 `df -k`。
//! 探测失败一律返回 `None`（诊断命令不该因为拿不到空间而失败）。

use std::path::Path;

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
    let ok = unsafe {
        GetDiskFreeSpaceExW(wide.as_ptr(), &mut avail, &mut total, &mut free)
    };
    if ok == 0 {
        None
    } else {
        Some((avail, total))
    }
}

/// 非 Windows：解析 `df -k <path>` 的第 2 行（可用 KB / 总 KB）。
#[cfg(not(windows))]
pub fn space_bytes(path: &Path) -> Option<(u64, u64)> {
    let out = std::process::Command::new("df").arg("-k").arg(path).output().ok()?;
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

/// 目录是否可写（真实写一个探针文件再删掉；不存在会先创建）。
pub fn writable(dir: &Path) -> bool {
    if std::fs::create_dir_all(dir).is_err() {
        return false;
    }
    let probe = dir.join(".vtrim-write-probe");
    match std::fs::write(&probe, b"ok") {
        Ok(()) => {
            let _ = std::fs::remove_file(&probe);
            true
        }
        Err(_) => false,
    }
}

/// 人类可读字节数（`1.2 GB` / `512 MB` / `900 B`）。
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn human_bytes_formats_units() {
        assert_eq!(human_bytes(512), "512 B");
        assert_eq!(human_bytes(2048), "2.0 KB");
        assert_eq!(human_bytes(5 * 1024 * 1024), "5.0 MB");
        assert_eq!(human_bytes(3 * 1024 * 1024 * 1024), "3.0 GB");
    }

    #[test]
    fn writable_detects_real_writability() {
        let dir = tempfile::tempdir().unwrap();
        assert!(writable(dir.path()));
        // 用文件冒充目录 → 不可写
        let file = dir.path().join("f.txt");
        std::fs::write(&file, b"x").unwrap();
        assert!(!writable(&file));
        // 探针文件不会残留
        assert!(!dir.path().join(".vtrim-write-probe").exists());
    }

    #[test]
    fn space_bytes_reports_something_or_none() {
        // 不强制要求平台支持：拿到值时必须是"可用 ≤ 总量"
        if let Some((avail, total)) = space_bytes(&std::env::temp_dir()) {
            assert!(total > 0);
            assert!(avail <= total, "avail={avail} total={total}");
        }
    }
}
