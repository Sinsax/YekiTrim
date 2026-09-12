use std::path::PathBuf;

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
