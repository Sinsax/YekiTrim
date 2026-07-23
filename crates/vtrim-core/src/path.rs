use std::path::PathBuf;

pub fn vtrim_home() -> PathBuf {
    std::env::current_exe()
        .expect("无法获取可执行文件路径")
        .parent()
        .expect("可执行文件路径异常")
        .join(".vtrim")
}

/// 预设贴纸文件夹路径 (程序目录下的 stickers/ 目录)
pub fn sticker_dir() -> PathBuf {
    std::env::current_exe()
        .expect("无法获取可执行文件路径")
        .parent()
        .expect("可执行文件路径异常")
        .join("stickers")
}
