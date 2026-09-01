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
                // 取第一个未被使用的 codepoint, 避免同一 emoji 重复出现
                cat.search(kw, 8).into_iter()
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
            let packages = crate::assets::sticker_index::scan_packages(&vtrim_core::path::sticker_dir());
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
    if has_stickers && !config.sticker_source.is_empty() {
        // 按 (segment_idx, event_idx, source) 精确到事件级分配:
        // 优先用 process Step 2 已持久化的 source, 旧工程无 source 字段时回退轮询
        let mut assignments: Vec<(usize, usize, String)> = vec![];
        let sources = &config.sticker_source;
        for (si, seg) in config.segments.iter().enumerate() {
            for (ei, event) in seg.events.iter().enumerate() {
                if let project::Event::Sticker { source: Some(src), .. } = event {
                    assignments.push((si, ei, src.clone()));
                } else if matches!(event, project::Event::Sticker { .. }) {
                    let source = sources[assignments.len() % sources.len()].clone();
                    assignments.push((si, ei, source));
                }
            }
        }
        let r = downloader.download_stickers(&config.segments, &assignments).await;
        if r.sticker_downloaded > 0 || r.sticker_existing > 0 {
            tracing::info!("  ✓ stickers: {} new, {} existing", r.sticker_downloaded, r.sticker_existing);
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

    total_errors
}

/// Auto-download missing assets (called by preview/render)
pub async fn ensure_assets(project_path: &Path) {
    let downloader = assets::downloader::AssetDownloader::new(project_path);

    if let Ok(config) = project::load_project(project_path)
        && !config.segments.is_empty() {
            let blacklist = load_sfx_blacklist(&config);
            let total_errors = download_keyword_assets(&downloader, &config, &blacklist).await;
            if total_errors > 0 {
                tracing::info!("  ⚠ {} download errors (will use CDN fallback where available)", total_errors);
            }
        }
}

