use std::path::{Path, PathBuf};

/// 程序所在目录：`.vtrim/`（配置、模型、server.toml）与 `stickers/` 的父目录。
///
/// 取不到可执行文件路径时回退当前工作目录。这两个函数在 HTTP handler 里被频繁调用，
/// 一旦 panic 会连累整条调用链（任务体的 panic 由 `spawn_job` 兜住、但那次任务就失败了），
/// 所以这里绝不 `expect`。
pub(crate) fn exe_dir() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_else(|| PathBuf::from("."))
}

/// 用户主目录（`USERPROFILE` → `HOME`；取不到 = None）。模型与字体库的用户目录都挂它下面。
pub fn user_home() -> Option<PathBuf> {
    std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)
}

pub fn vtrim_home() -> PathBuf {
    exe_dir().join(".vtrim")
}

/// 预设贴纸文件夹路径 (程序目录下的 stickers/ 目录)
pub fn sticker_dir() -> PathBuf {
    exe_dir().join("stickers")
}

/// 字体库路径 —— **用户固定目录优先**：`~/.vtrim/fonts` 存在就用它；否则回落程序目录的
/// `resources/fonts/`（兼容老安装：手工丢进去的字体继续可见）。
///
/// ⚠️ 为什么要固定目录：字体这类程序级资产若只挂在 exe 旁，`target/debug` 与
/// `target/cl/debug`（换 `CARGO_TARGET_DIR`）就各有一份字体库 —— 下载过一次、
/// 换个构建目录就全部「未下载」（2026-10 实际发生过）。与 ASR 模型
/// （`asr::local::resolve_model_dir`）同一套思路；写路径一律落用户目录
/// （[`crate::fonts::ensure_dir`]，含老库的一次性搬家）。
/// 相关读写见 [`crate::fonts`]。
pub fn font_dir() -> PathBuf {
    font_dir_in(user_home().as_deref(), &exe_dir())
}

/// [`font_dir`] 的可注入版本（单测用，不碰环境变量与真实 exe 路径）。
pub fn font_dir_in(user_home: Option<&Path>, exe_home: &Path) -> PathBuf {
    if let Some(home) = user_home {
        let user = home.join(".vtrim").join("fonts");
        if user.is_dir() {
            return user;
        }
    }
    exe_home.join("resources").join("fonts")
}

/// 给"人要看的/前端要回传的"路径用的显示形式：去掉 Windows 的 `\\?\` 逐字前缀
/// （canonicalize 的产物）。
///
/// 逐字前缀一旦被前端拿去做字符串拼接（`dir + '/' + name`）或塞进 URL，就会变成
/// `\\?\F:\a/b` 这种非法形式（ERROR_INVALID_NAME 123），所以**出 JSON 的路径必须过这里**。
/// `\\?\UNC\server\share` 还原成 `\\server\share`。
pub fn display(path: &Path) -> String {
    let s = path.to_string_lossy();
    if let Some(rest) = s.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{rest}")
    } else if let Some(rest) = s.strip_prefix(r"\\?\") {
        rest.to_string()
    } else {
        s.into_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_strips_verbatim_prefix() {
        assert_eq!(display(Path::new(r"\\?\F:\media\a")), r"F:\media\a");
        assert_eq!(display(Path::new(r"\\?\UNC\srv\share\a")), r"\\srv\share\a");
        assert_eq!(display(Path::new(r"F:\media\a")), r"F:\media\a");
        assert_eq!(display(Path::new("/tmp/a")), "/tmp/a");
    }

    #[test]
    fn font_dir_prefers_user_dir_but_falls_back_to_exe_dir() {
        let exe = Path::new("/install");
        // 用户目录不存在 → 程序目录（老安装继续可见）
        assert_eq!(
            font_dir_in(Some(Path::new("/no-such-home")), exe),
            Path::new("/install/resources/fonts")
        );
        // 取不到主目录 → 程序目录
        assert_eq!(font_dir_in(None, exe), Path::new("/install/resources/fonts"));
        // 用户目录存在 → 固定目录（不再随构建/安装目录漂移）
        let home = std::env::temp_dir().join(format!("vtrim-fontdir-{}", std::process::id()));
        let user = home.join(".vtrim").join("fonts");
        std::fs::create_dir_all(&user).unwrap();
        assert_eq!(font_dir_in(Some(&home), exe), user);
        std::fs::remove_dir_all(&home).ok();
    }
}
