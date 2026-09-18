use std::path::{Path, PathBuf};

/// 程序所在目录：`.vtrim/`（配置、模型、server.toml）与 `stickers/` 的父目录。
///
/// 取不到可执行文件路径时回退当前工作目录。这两个函数在 HTTP handler 里被频繁调用，
/// 而 release 构建是 `panic = "abort"`——任何 panic 都会直接终止整个 server 进程，
/// 所以这里绝不 `expect`。
fn exe_dir() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_else(|| PathBuf::from("."))
}

pub fn vtrim_home() -> PathBuf {
    exe_dir().join(".vtrim")
}

/// 预设贴纸文件夹路径 (程序目录下的 stickers/ 目录)
pub fn sticker_dir() -> PathBuf {
    exe_dir().join("stickers")
}

/// 字体库路径 (程序目录下的 `resources/fonts/`)。
///
/// 往里丢 `.ttf` / `.otf` / `.ttc` 就能在界面上选作字幕/封面字体 —— 库本身不随包分发
/// 任何字体文件（授权原因，见 README「字体」），所以这个目录是**用户自己填**的。
/// 相关读写见 [`crate::fonts`]。
pub fn font_dir() -> PathBuf {
    exe_dir().join("resources").join("fonts")
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
}
