//! 磁盘空间与可写性探测（`yekitrim doctor` 用）。
//!
//! 空间查询的实现在 `yekitrim_core::disk`（2026-11-15 下沉到 core：管线阶段预检与
//! 切片批次边界要用同一份，而 core 不许反向依赖本 crate）—— 这里只是委托转发。
//! 探测失败一律返回 `None`（诊断命令不该因为拿不到空间而失败）。

use std::path::Path;

/// 目标路径所在卷的 `(可用字节, 总字节)`。
pub fn space_bytes(path: &Path) -> Option<(u64, u64)> {
    yekitrim_core::disk::space_bytes(path)
}

/// 目录是否可写（真实写一个探针文件再删掉；不存在会先创建）。
pub fn writable(dir: &Path) -> bool {
    if std::fs::create_dir_all(dir).is_err() {
        return false;
    }
    let probe = dir.join(".yekitrim-write-probe");
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
    yekitrim_core::disk::human_bytes(bytes)
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
        assert!(!dir.path().join(".yekitrim-write-probe").exists());
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
