//! 项目路径囚笼（多用户服务的路径面归零，设计稿 §5.2）。
//!
//! 多用户下所有接口只接受**项目内相对路径**（`output/clips/clip-01`），由服务端
//! 自己拼出绝对路径。拼接必须满足两条不变量：
//!
//! 1. **词法不越界**：拒绝绝对路径、UNC（`\\server\share`、`\\?\C:\`）、盘符相对路径
//!    （`C:foo`）、任何 `..` 段与空串；
//! 2. **解析后不越界**：对已存在的部分做 `canonicalize`，符号链接/junction 指到项目
//!    目录之外时同样拒绝（Windows 上 `canonicalize` 会带 `\\?\` 前缀，两端都由同一
//!    个 root 推导，比较自洽）。
//!
//! 调用方拿到的路径已保证在 `root` 之内，可以直接做 IO。

use std::path::{Component, Path, PathBuf};

use anyhow::{bail, Context, Result};

/// 判断"看起来是绝对路径/越界写法"的输入。
fn is_escape_attempt(rel: &str) -> Option<&'static str> {
    let trimmed = rel.trim();
    if trimmed.is_empty() {
        return Some("路径不能为空");
    }
    if trimmed.starts_with("\\\\") || trimmed.starts_with("//") {
        return Some("不接受 UNC / 网络路径");
    }
    if trimmed.starts_with('\\') || trimmed.starts_with('/') {
        return Some("只接受项目内相对路径");
    }
    // `C:foo`（盘符相对）与 `C:\foo` 都会带冒号；正常相对路径不会
    if trimmed.contains(':') {
        return Some("不接受盘符路径");
    }
    if trimmed
        .split(['/', '\\'])
        .any(|seg| seg == "..")
    {
        return Some("路径不能包含 .. 段");
    }
    if trimmed.contains('\0') {
        return Some("路径含非法字符");
    }
    None
}

/// 词法归一化（不访问文件系统）：消掉冗余分隔符，`..` 已在入口被拒绝。
fn lexically_clean(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for comp in path.components() {
        match comp {
            Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// 逐级 canonicalize：对**已存在的最深祖先**做解析（解掉符号链接/junction），
/// 再拼回尚不存在的尾部。保证返回值与 `Path::canonicalize` 的表示一致
/// （Windows 上是 `\\?\` 前缀，两端由同一个 root 推导，比较自洽）。
fn canonicalize_deepest(path: &Path) -> Result<PathBuf> {
    let clean = lexically_clean(path);
    let mut cursor: &Path = &clean;
    let mut tail: Vec<std::ffi::OsString> = Vec::new();
    loop {
        if cursor.exists() {
            let mut out = cursor
                .canonicalize()
                .with_context(|| format!("无法解析路径 {}", cursor.display()))?;
            for seg in tail.iter().rev() {
                out.push(seg);
            }
            return Ok(out);
        }
        match (cursor.file_name(), cursor.parent()) {
            (Some(name), Some(parent)) => {
                tail.push(name.to_os_string());
                cursor = parent;
            }
            _ => bail!("无法解析路径 {}", path.display()),
        }
    }
}

/// 把项目内相对路径拼成绝对路径（不要求该路径已存在，例如写入新文件）。
pub fn jail_join(root: &Path, rel: impl AsRef<str>) -> Result<PathBuf> {
    let rel = rel.as_ref();
    if let Some(reason) = is_escape_attempt(rel) {
        bail!("非法路径 {rel:?}：{reason}");
    }
    let root_canon = root
        .canonicalize()
        .with_context(|| format!("项目目录不存在: {}", root.display()))?;
    let candidate = lexically_clean(&root_canon.join(rel));
    let resolved = canonicalize_deepest(&candidate)?;
    if !resolved.starts_with(&root_canon) {
        bail!("路径越出项目目录: {}", resolved.display());
    }
    Ok(resolved)
}

/// 同 [`jail_join`]，但要求目标**已存在**（读文件、列目录）。
pub fn jail_existing(root: &Path, rel: impl AsRef<str>) -> Result<PathBuf> {
    let path = jail_join(root, rel)?;
    if !path.exists() {
        bail!("路径不存在: {}", path.display());
    }
    Ok(path)
}

/// 校验一个**外部传入的路径**（如录播姬 webhook 的 `RelativePath`）确实在
/// `root` 之内；返回规范化后的绝对路径。
///
/// 与 `jail_join` 的区别：调用方给的是完整路径（可能带 `..`），这里用
/// "解析后必须 starts_with(root)" 兜底，而不是词法拒绝。
pub fn ensure_within(root: &Path, candidate: &Path) -> Result<PathBuf> {
    let root_canon = root
        .canonicalize()
        .with_context(|| format!("根目录不存在: {}", root.display()))?;
    let target = if candidate.is_absolute() {
        candidate.to_path_buf()
    } else {
        root_canon.join(candidate)
    };
    let resolved = canonicalize_deepest(&target)?;
    if !resolved.starts_with(&root_canon) {
        bail!("路径越出根目录: {}", resolved.display());
    }
    Ok(resolved)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 自建临时目录（vtrim-core 不依赖 tempfile；用 pid + 纳秒保证并发测试不撞名）。
    struct TmpDir(PathBuf);

    impl TmpDir {
        fn new(tag: &str) -> Self {
            let nanos = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0);
            let p = std::env::temp_dir().join(format!(
                "vtrim-tenant-{tag}-{}-{nanos}",
                std::process::id()
            ));
            std::fs::create_dir_all(&p).unwrap();
            TmpDir(p)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TmpDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn setup() -> (TmpDir, PathBuf) {
        let dir = TmpDir::new("proj");
        let root = dir.path().join("proj");
        std::fs::create_dir_all(root.join("output/clips")).unwrap();
        std::fs::write(root.join("yekitrim.toml"), "input_file = \"a.mp4\"\n").unwrap();
        (dir, root)
    }

    #[test]
    fn accepts_normal_relative_paths() {
        let (_d, root) = setup();
        let p = jail_join(&root, "output/clips").unwrap();
        assert!(p.ends_with("clips"));
        assert!(jail_existing(&root, "yekitrim.toml").is_ok());
        // 尾部斜杠 / 前导 ./ 都是正常写法
        assert!(jail_join(&root, "output/").is_ok());
        assert!(jail_join(&root, "./output/clips").is_ok());
        // 尚不存在的文件（写入场景）
        assert!(jail_join(&root, "output/events.json").is_ok());
    }

    #[test]
    fn rejects_escape_attempts() {
        let (_d, root) = setup();
        for bad in [
            "",
            "   ",
            "..",
            "../yekitrim.toml",
            "output/../../etc/passwd",
            "./../x",
            "/etc/passwd",
            "\\windows\\system32",
            "\\\\server\\share\\x",
            "//server/share",
            "C:\\Windows\\notepad.exe",
            "C:foo",
            "output/..",
        ] {
            assert!(
                jail_join(&root, bad).is_err(),
                "必须拒绝越界路径: {bad:?}"
            );
        }
    }

    #[test]
    fn existing_requires_presence() {
        let (_d, root) = setup();
        assert!(jail_existing(&root, "output/nope.json").is_err());
        assert!(jail_existing(&root, "output/clips").is_ok());
    }

    #[test]
    fn ensure_within_handles_absolute_and_relative() {
        let (_d, root) = setup();
        let inside = root.join("output").join("clips");
        assert!(ensure_within(&root, &inside).is_ok());
        assert!(ensure_within(&root, Path::new("output/clips")).is_ok());
        assert!(ensure_within(&root, Path::new("../outside")).is_err());
        let outside = root.parent().unwrap().join("other");
        std::fs::create_dir_all(&outside).unwrap();
        assert!(ensure_within(&root, &outside).is_err());
    }

    #[test]
    fn rejects_symlink_escape() {
        let dir = TmpDir::new("symlink");
        let root = dir.path().join("proj");
        let outside = dir.path().join("outside");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(outside.join("secret.txt"), "top secret").unwrap();

        let link = root.join("link");
        let created = {
            #[cfg(unix)]
            {
                std::os::unix::fs::symlink(&outside, &link).is_ok()
            }
            #[cfg(windows)]
            {
                std::os::windows::fs::symlink_dir(&outside, &link).is_ok()
            }
            #[cfg(not(any(unix, windows)))]
            {
                false
            }
        };
        if !created {
            // Windows 未开启开发者模式/非管理员时建不了符号链接：跳过（CI 上是 Linux）
            eprintln!("跳过符号链接用例：当前环境不允许创建符号链接");
            return;
        }
        assert!(
            jail_join(&root, "link/secret.txt").is_err(),
            "经符号链接读到项目外文件必须被拒绝"
        );
        assert!(jail_existing(&root, "link").is_err() || jail_join(&root, "link").is_err());
    }
}
