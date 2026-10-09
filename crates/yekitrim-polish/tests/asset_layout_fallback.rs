//! 资产布局回落：**存量安装升级后库不许变空**。
//!
//! 2026-10-09 把素材库收拢到 `<数据目录>/resources/{stickers,videos}`、发布物收进 `runtime/`。
//! 但**存量安装里老位置还在**（`<数据目录>/stickers/…`、`tools/`、`spec/`），
//! 且用户是**手工搬**的 —— 所以"读两个位置"不是可选优化，是升级不坏的唯一保证。
//!
//! 这个文件用**真实目录树**验的是三件事（不是 `*_at()` 的纯拼接，那些在 `path.rs` 单测里）：
//! ① 只有老位置时，视频库扫描**仍能列出素材**（否则界面显示"库是空的"、且不报错）；
//! ② 新位置一出现就**以新位置为准**；
//! ③ 贴纸库同理。

use std::path::Path;

/// 造一个"存量安装"：素材只躺在**老位置**的 `stickers/videos/` 下。
fn legacy_install(root: &Path) {
    let old_videos = root.join("stickers").join("videos");
    std::fs::create_dir_all(&old_videos).unwrap();
    std::fs::write(old_videos.join("op.mp4"), b"legacy-video").unwrap();
    let old_pack = root.join("stickers").join("ysyk");
    std::fs::create_dir_all(&old_pack).unwrap();
    std::fs::write(old_pack.join("笑.gif"), b"legacy-sticker").unwrap();
}

/// ① 存量升级：**老位置仍要读得到**（这是本文件存在的全部理由）。
#[test]
fn legacy_layout_is_still_readable() {
    let tmp = tempfile::tempdir().unwrap();
    legacy_install(tmp.path());

    // 贴纸库：新位置不存在 ⇒ 回落老位置，包仍列得出来
    let sticker_root = yekitrim_core::path::sticker_dir_readable_at(tmp.path());
    assert_eq!(
        sticker_root,
        tmp.path().join("stickers"),
        "新位置没有时，贴纸库必须回落老位置"
    );
    let packs: Vec<String> = yekitrim_polish::assets::sticker_index::scan_packages(&sticker_root)
        .into_iter()
        .map(|p| p.name)
        .collect();
    assert_eq!(packs, vec!["ysyk"], "存量贴纸包必须还在（升级后库不许变空）");

    // 视频库：回落链要走到 `<data>/stickers/videos`，并且**真能扫出素材**
    let video_root = yekitrim_core::path::video_dir_readable_at(tmp.path());
    assert_eq!(
        video_root,
        tmp.path().join("stickers").join("videos"),
        "新位置没有时，视频库必须回落老位置"
    );
    let vids: Vec<String> = yekitrim_polish::assets::sticker_index::scan_video_library(&video_root)
        .into_iter()
        .map(|a| a.name)
        .collect();
    assert_eq!(vids, vec!["op"], "存量视频素材必须还在（否则片头/片尾选不到素材）");
}

/// ② 新位置一出现就**以新位置为准**（判据是"目录存在"，不看里面有没有东西）。
#[test]
fn new_layout_wins_once_it_exists() {
    let tmp = tempfile::tempdir().unwrap();
    legacy_install(tmp.path());

    // 用户手工把视频搬到新位置（老那份还留着，模拟搬了一半）
    let new_videos = tmp.path().join("resources").join("videos");
    std::fs::create_dir_all(&new_videos).unwrap();
    std::fs::write(new_videos.join("new-op.mp4"), b"new-video").unwrap();

    let video_root = yekitrim_core::path::video_dir_readable_at(tmp.path());
    assert_eq!(video_root, new_videos, "新位置存在 ⇒ 必须用它");
    let vids: Vec<String> = yekitrim_polish::assets::sticker_index::scan_video_library(&video_root)
        .into_iter()
        .map(|a| a.name)
        .collect();
    assert_eq!(vids, vec!["new-op"], "用的是新位置那份，不是老的那份");
}

/// ③ **视频库不再是贴纸库的子目录** —— 挪库要消灭的那个隐患。
///
/// 老形态下 `videos/` 是贴纸根下的普通子目录，`scan_packages` 会把它当包扫，
/// 只靠"里面一张图都没有 ⇒ 被丢掉"**碰巧**没污染列表。这里钉住新形态：
/// 贴纸库扫描对同级的 `videos/` **完全无感**。
#[test]
fn video_library_is_not_nested_under_stickers() {
    let tmp = tempfile::tempdir().unwrap();
    let resources = tmp.path().join("resources");
    let stickers = resources.join("stickers");
    let videos = resources.join("videos");
    std::fs::create_dir_all(stickers.join("ysyk")).unwrap();
    std::fs::write(stickers.join("ysyk").join("笑.gif"), b"x").unwrap();
    std::fs::create_dir_all(&videos).unwrap();
    std::fs::write(videos.join("op.mp4"), b"x").unwrap();

    // 两棵树并列、互不嵌套 —— 这是"结构上不可能污染"的来源
    assert!(!videos.starts_with(&stickers), "视频库不许再挂在贴纸库下面");

    let packs: Vec<String> = yekitrim_polish::assets::sticker_index::scan_packages(&stickers)
        .into_iter()
        .map(|p| p.name)
        .collect();
    assert_eq!(packs, vec!["ysyk"], "只有真贴纸包，videos/ 不出现");
}
