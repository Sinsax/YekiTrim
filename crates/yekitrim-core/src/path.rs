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

/// **程序资产的根目录**（用户口径 2026-10-02：*所有内容围绕程序目录来，不要用户目录*）。
///
/// 默认 = [`exe_dir`]，即 `target/debug/`（debug 构建）或安装目录（发行版）——
/// 字体库、ASR 模型、配置全都挂它下面。
///
/// ⚠️ **`VTRIM_DATA_DIR` 是唯一的口子**，留着它是为了应付一个具体的痛点：
/// `target/debug` 与 `target/release`（以及换 `CARGO_TARGET_DIR`）各是一份程序目录，
/// 那样 **1.1 GB 的 ASR 模型会被复制两份**。想共用就把这个变量指到同一处。
///
/// ⚠️ 别再退回"用户目录优先"：那条路是 2026-10 之前的设计（当年为了躲开构建目录漂移
/// 才把资产挂到 `~/.vtrim/`），代价是**同一个东西有了两个可能的位置**，于是界面上出过
/// "模型管理说没下载、实际转写一直在用"（`api/config.rs` 记着那次错位）。
/// 资产只有一个位置，就没有"只查了一处"的 bug。
pub fn data_dir() -> PathBuf {
    std::env::var_os("VTRIM_DATA_DIR")
        .map(PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(exe_dir)
}

/// [`data_dir`] 的可注入版本（单测用：不碰环境变量，edition 2024 下 `set_var` 是 unsafe）。
pub fn data_dir_in(override_dir: Option<&Path>, exe_home: &Path) -> PathBuf {
    match override_dir {
        Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
        _ => exe_home.to_path_buf(),
    }
}

/// 用户主目录（`USERPROFILE` → `HOME`；取不到 = None）。
///
/// ⚠️ **程序资产不再挂它下面**（2026-10-02 起，见 [`data_dir`]）。仅存两个用途：
/// ① 识别用户目录里**遗留**的老资产（`fonts::legacy_user_dir_report`）；
/// ② 老库巡检的报数。**别拿它当资产的落点**。
pub fn user_home() -> Option<PathBuf> {
    std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)
}

/// 程序状态目录（配置、模型、server.toml）—— 挂在 [`data_dir`] 下的 `.vtrim/`。
pub fn vtrim_home() -> PathBuf {
    data_dir().join(".vtrim")
}

/// 预设贴纸文件夹路径（程序目录下的 `stickers/` 目录）。
pub fn sticker_dir() -> PathBuf {
    exe_dir().join("stickers")
}

/// 字体库路径 —— **程序目录下**：`<数据目录>/resources/fonts/`。
///
/// ⚠️ **2026-10-02 由"用户固定目录优先"改成程序目录**（用户口径：*不要用户目录*）。
/// 历史：这条路径曾经优先取 `~/.vtrim/fonts`（为了躲开 `target/debug` 与
/// `target/cl/debug` 各存一份的问题），再用 `resources/fonts/` 当老安装的回落态。
/// 代价是**同一个库有两个可能的位置**，而"用户目录那份"与"程序目录那份"内容一旦分叉，
/// CLI 与 WebUI 就各看各的（实测：CLI 报 `target/debug/resources/fonts`、
/// 网页用 `~/.vtrim/fonts`，两边字体列表完全不同）。
///
/// 现在只有一处：`<数据目录>/resources/fonts/`。
/// 相关读写见 [`crate::fonts`]。
pub fn font_dir() -> PathBuf {
    font_dir_at(&data_dir())
}

/// [`font_dir`] 的可注入版本（单测用）。
pub fn font_dir_at(data_home: &Path) -> PathBuf {
    data_home.join("resources").join("fonts")
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

    /// **资产只有一个位置**：默认在程序目录，`VTRIM_DATA_DIR` 是唯一的口子。
    ///
    /// 回归的是 2026-10-02 的口径切换（*所有内容围绕程序目录来，不要用户目录*）：
    /// 字体库曾经优先取 `~/.vtrim/fonts`，导致 CLI 与 WebUI 各看一份库（实测两边
    /// 列出的字体完全不同）。现在 `font_dir` 里**不再有任何用户目录**参与。
    #[test]
    fn font_dir_lives_under_data_dir_no_user_home_involved() {
        let exe = Path::new("/install");
        // 没有覆盖 ⇒ 程序目录
        assert_eq!(data_dir_in(None, exe), Path::new("/install"));
        assert_eq!(font_dir_at(&data_dir_in(None, exe)), Path::new("/install/resources/fonts"));
        // 空串等同没设（环境变量为空时别把资产写到根目录去）
        assert_eq!(data_dir_in(Some(Path::new("")), exe), Path::new("/install"));
        // 显式覆盖 ⇒ 用覆盖值（debug/release 共用一份 1.1 GB 模型靠它）
        let shared = Path::new("/shared/vtrim-data");
        assert_eq!(data_dir_in(Some(shared), exe), shared);
        assert_eq!(font_dir_at(shared), Path::new("/shared/vtrim-data/resources/fonts"));
        // ⚠️ 判据是"路径里没有用户目录" —— 主目录取不到/取得到都不该改变结果
        assert!(!font_dir_at(&data_dir_in(None, exe)).to_string_lossy().contains(".vtrim/fonts"));
    }
}
