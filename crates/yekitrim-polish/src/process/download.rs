use std::path::Path;

use crate::assets;
use crate::project;
use super::sfx::load_sfx_blacklist;

pub fn sanitize_sticker_name(keyword: &str) -> String {
    keyword
        .chars()
        .map(|c| if c.is_alphanumeric() || c == '_' || c == '-' { c } else { '_' })
        .collect::<String>()
        .trim_matches('_')
        .to_string()
}

/// 尝试用指定来源解析 sticker keyword → 文件名。失败返回 None。
/// 供 Step 2 轮询 (phase 1) 与再平衡 (phase 2) 共用。
/// 来源：emoji / 具体贴纸包名（如 ysyk）。
#[allow(clippy::too_many_arguments)]
pub(super) async fn try_resolve_sticker(
    src: &str,
    kw: &str,
    time: f64,
    catalog: &Option<crate::assets::emoji_search::EmojiCatalog>,
    sticker_dir: &std::path::Path,
    client: &reqwest::Client,
    used_emojis: &mut std::collections::HashSet<String>,
) -> Option<String> {
    match src {
        "emoji" => {
            if let Some(cat) = catalog {
                // ⚠️ **走 `search_best` 而不是 `search`**（2026-10-17）：
                //    `search` 只有"精确 + 子串 ⊇"两档，命中谁**由目录顺序决定**、无评分 ——
                //    那是"LLM 写 game，结果拿到 video-game 那一组的前 8 个"的来源。
                //    `search_best` 先试**分维查询**（`emotion:joy,usage:punchline` 这种紧凑串，
                //    或能对上词表的自由词）并按维度权重打分排序，再落回 `search` 兜底 ——
                //    所以老工程那些词表外的 keyword（`lol`/`angry`）照样能命中。
                // 取第一个未被使用的 codepoint, 避免同一 emoji 重复出现
                cat.search_best(kw, 8).into_iter()
                    .find(|cp| !used_emojis.contains(cp))
                    .map(|cp| { used_emojis.insert(cp.clone()); cp })
            } else {
                None
            }
        }
        "irasu" => {
            // 实际查询 irasutoya 验证有图 (带缓存), 避免写入 name 但下载时无图
            let urls = crate::assets::irasu_search::resolve_image_urls(kw, sticker_dir, client).await;
            if urls.is_empty() { None } else { Some(sanitize_sticker_name(kw)) }
        }
        // 贴纸包名（如 "ysyk"）：keyword 通常是包名，直接从包内选文件
        src => {
            let packages = crate::assets::sticker_index::scan_packages(
                &yekitrim_core::owner::sticker_dir_readable(yekitrim_core::owner::Owner::from_env()),
            );
            if packages.iter().any(|p| p.name == src) {
                crate::assets::sticker_index::pick_package_file(&packages, src, time)
            } else {
                None
            }
        }
    }
}

pub async fn cmd_download(project_path: &Path) -> anyhow::Result<()> {
    tracing::info!("📥  Downloading assets...");
    let downloader = assets::downloader::AssetDownloader::new(project_path);

    let mut total_errors = 0;
    if let Ok(cfg) = project::load_project(project_path)
        && !cfg.segments.is_empty() {
            let blacklist = load_sfx_blacklist(&cfg);
            total_errors += download_keyword_assets(&downloader, &cfg, &blacklist).await;
        }

    if total_errors > 0 {
        tracing::info!("  ⚠ {} download errors", total_errors);
    }
    Ok(())
}

/// 下载 sticker + sfx 资产 (blacklist 中的 sfx 跳过下载)
pub async fn download_keyword_assets(
    downloader: &assets::downloader::AssetDownloader,
    config: &project::ProjectFile,
    blacklist: &[String],
) -> usize {
    let mut total_errors = 0;

    let has_stickers = config.segments.iter().any(|seg| {
        seg.events.iter().any(|ev| matches!(ev, project::Event::Sticker { keyword: kw, .. } if !kw.is_empty()))
    });
    // ⚠️ **不再用「`sticker_source` 非空」当门控**：只要事件引用了贴纸，就要把它**实际用的那个文件**
    // 复制进 `<项目>/assets/stickers/` —— 出片与预览只读项目内那份（`prepare_sticker_data` /
    // `build_sticker_map` 都只扫项目目录），这是"单个 polish 项目能独立复现"的唯一保证。
    //
    // 判据取**事件自己的 `source`**（process Step 2 解析并落过盘的那个）；`sticker_source`
    // 只用来给"没有 `source` 字段"的老工程兜底轮询。以前这里是 `&& !config.sticker_source.is_empty()`：
    // 用户把来源列表清空之后**一个文件都不复制**，项目就悄悄不再自包含。
    if has_stickers {
        let mut assignments: Vec<(usize, usize, String)> = vec![];
        let sources = &config.sticker_source;
        let mut no_source = 0usize;
        for (si, seg) in config.segments.iter().enumerate() {
            for (ei, event) in seg.events.iter().enumerate() {
                let project::Event::Sticker { source, keyword, .. } = event else {
                    continue;
                };
                if keyword.trim().is_empty() {
                    continue;
                }
                match source {
                    Some(src) if !src.is_empty() => assignments.push((si, ei, src.clone())),
                    // 老工程没写 source：只能按来源列表轮询
                    _ if !sources.is_empty() => {
                        let src = sources[assignments.len() % sources.len()].clone();
                        assignments.push((si, ei, src));
                    }
                    _ => no_source += 1,
                }
            }
        }
        if no_source > 0 {
            tracing::warn!(
                "  ⚠ {no_source} 条贴纸事件既没有 source、来源列表也为空 —— 无法把贴纸复制进项目，\
                 这几条在出片时会缺（先在编辑器里选一次贴纸来源，重跑 process）"
            );
        }
        let r = downloader.download_stickers(&config.segments, &assignments).await;
        if r.sticker_downloaded > 0 || r.sticker_existing > 0 {
            tracing::info!("  ✓ stickers: {} new, {} existing", r.sticker_downloaded, r.sticker_existing);
        }
        // 复制失败会让项目**不再自包含**（出片缺图），所以逐条报出来，不并进"下载失败"里混着看
        for e in &r.errors {
            tracing::warn!("  ⚠ 贴纸没进项目: {e}");
        }
        total_errors += r.errors.len();
    }

    let has_sfx = !assets::downloader::collect_sfx_ids(&config.segments).is_empty();
    if has_sfx {
        let r = downloader.download_sfx_for_segments(&config.segments, blacklist).await;
        if r.sfx_downloaded > 0 || r.sfx_existing > 0 || !r.errors.is_empty() {
            tracing::info!("  ✓ sfx: {} new, {} cached, {} errors", r.sfx_downloaded, r.sfx_existing, r.errors.len());
        }
        total_errors += r.errors.len();
    }

    // 第 6 批：EDL 里引用的**视频素材**也要复制进项目。
    //
    // ⚠️ **这是"单个工程能独立复现"的唯一保证**（`assets/AGENTS.md`）：出片与预览只读项目目录，
    // 引用素材库的绝对路径会在**换机器/库改名**之后直接出不了片，而且**不报错**。
    // 与贴纸同一条纪律 —— 判据取**这一段自己那一份**（这里就是 EDL 段自己的 `asset_id`）。
    //
    // ⚠️ 同步函数（素材是本地拷贝，不是下载），**别加 await**。
    let edl_asset_ids = assets::downloader::collect_edl_asset_ids(&config.edl.clips);
    if !edl_asset_ids.is_empty() {
        let r = downloader.download_videos(&edl_asset_ids);
        if r.video_downloaded > 0 || r.video_existing > 0 {
            tracing::info!("  ✓ videos: {} new, {} existing", r.video_downloaded, r.video_existing);
        }
        // 缺一个素材 ⇒ 那一段直接缺画面。逐条报出来，别并进"下载失败"里混着看。
        for e in &r.errors {
            tracing::warn!("  ⚠ 视频素材没进项目: {e}");
        }
        total_errors += r.errors.len();
    }

    total_errors
}

/// `ensure_assets` 该不该去跑一遍素材复制 —— **唯一判据**（抽成纯函数是为了能直接测）。
///
/// 两条来源**各自独立**，缺一不可：
/// - 贴纸 / SFX 挂在 `segments` 上（老的 `!segments.is_empty()`）；
/// - **视频素材挂在 `edl.clips` 上**（第 6 批）。
///
/// ⚠️ 只写前半条 ⇒ "只有 EDL、没有段"（段表空、或段全被去静音切掉）的工程被整个跳过，
/// 素材从没被复制进项目，出片时**那一段缺画面且不报错**。
pub fn ensure_assets_should_run(
    segments: &[project::Segment],
    edl_clips: &[yekitrim_render_core::types::EdlClip],
) -> bool {
    !segments.is_empty() || !assets::downloader::collect_edl_asset_ids(edl_clips).is_empty()
}

/// Auto-download missing assets (called by preview/render)
pub async fn ensure_assets(project_path: &Path) {
    let downloader = assets::downloader::AssetDownloader::new(project_path);

    if let Ok(config) = project::load_project(project_path)
        && ensure_assets_should_run(&config.segments, &config.edl.clips)
    {
        let blacklist = load_sfx_blacklist(&config);
        let total_errors = download_keyword_assets(&downloader, &config, &blacklist).await;
        if total_errors > 0 {
            tracing::info!("  ⚠ {} download errors (will use CDN fallback where available)", total_errors);
        }
    }
}

