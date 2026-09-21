//! polish::assets::irasu_search：いらすとや 插图的检索（RSS feed）与下载。
//!
//! 入口：`download_images`、`resolve_image_urls`、`collect_keywords`、`PLACEHOLDER_PNG`。
//! 边界：解析不到图必须退回 `PLACEHOLDER_PNG` 而不是留空；图片扩展名判据统一用 `sticker_index::is_image_file_name` —— 见 `crates/vtrim-polish/src/assets/AGENTS.md`

use std::path::{Path, PathBuf};
use std::collections::HashMap;

const FEED_URL: &str = "https://www.irasutoya.com/feeds/posts/summary";

/// 1x1 紫色 RGBA PNG 占位图（#8B5CF6）
pub const PLACEHOLDER_PNG: &[u8] = &[
    0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1f, 0x15, 0xc4,
    0x89, 0x00, 0x00, 0x00, 0x0c, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9c, 0xeb, 0x8e, 0xf9, 0xf6, 0x1f,
    0x00, 0x06, 0x2f, 0x02, 0xdd, 0x13, 0x46, 0x7d, 0xeb, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4e,
    0x44, 0xae, 0x42, 0x60, 0x82,
];

/// 搜索 irasutoya，下载前 max 张匹配图片到 dest_dir
/// 返回 (新下载文件列表, 已存在文件数)
pub async fn download_images(
    keyword: &str,
    dest_dir: &Path,
    client: &reqwest::Client,
    max: usize,
) -> (Vec<PathBuf>, usize) {
    if max == 0 {
        return (vec![], 0);
    }

    // 0. 检查本地是否已有该 keyword 的文件（用户可能手动放置）
    let stem = sanitize_filename(keyword);
    let local_files: Vec<PathBuf> = match std::fs::read_dir(dest_dir) {
        Ok(entries) => entries.flatten()
            .filter(|e| {
                let name = e.file_name().to_string_lossy().into_owned();
                // 兼容旧格式 {stem}_N.ext 与新格式 {stem}.ext
                name.starts_with(&format!("{}_", stem))
                    || Path::new(&name).file_stem().map(|s| s == stem.as_str()).unwrap_or(false)
            })
            .map(|e| e.path())
            .collect(),
        Err(_) => vec![],
    };
    let existing_count = local_files.len();
    if existing_count > 0 {
        return (vec![], existing_count);
    }

    // 1. 获取图片 URL（优先缓存，无缓存则调 Blogger API）
    let image_urls = resolve_image_urls(keyword, dest_dir, client).await;
    if image_urls.is_empty() {
        return (vec![], 0);
    }

    // 2. 下载图片
    let mut downloaded = vec![];
    let mut existing = 0;
    let mut any_failed = false;

    for (i, url) in image_urls.iter().enumerate().take(max) {
        let ext = guess_extension(url);
        // max==1 时省略序号后缀，使文件名与 process Step 2 解析出的 name 一致（STICKER_MAP 按 stem 匹配）
        let fname = if max == 1 {
            format!("{}{}", sanitize_filename(keyword), ext)
        } else {
            format!("{}_{}{}", sanitize_filename(keyword), i + 1, ext)
        };
        let path = dest_dir.join(&fname);

        if path.exists() {
            existing += 1;
            continue;
        }

        match download_file(client, url, &path).await {
            Ok(()) => {
                println!("  ✓ irasu {} → {}", fname, path.display());
                downloaded.push(path);
            }
            Err(e) => {
                // 下载失败 → 写入紫色占位图
                println!("  ⚠ irasu {} 下载失败({}), 使用占位图", fname, e);
                std::fs::write(&path, PLACEHOLDER_PNG).ok();
                downloaded.push(path);
                any_failed = true;
            }
        }
    }

    if any_failed {
        let mut index = load_index(dest_dir);
        if index.contains_key(keyword) {
            index.remove(keyword);
            save_index(dest_dir, &index);
            println!("  ⚠ irasu「{}」缓存已清除，下次将重新搜索", keyword);
        }
    }

    (downloaded, existing)
}

// ============================================================
// Blogger JSON Feed API
// ============================================================

/// 搜索 Blogger API，返回匹配的图片条目
async fn fetch_entries(keyword: &str, client: &reqwest::Client) -> Vec<String> {
    let url = format!("{}?alt=json&q={}&max-results=20", FEED_URL, urlencoding(keyword));

    let json: serde_json::Value = match client.get(&url).send().await {
        Ok(resp) => match resp.json().await {
            Ok(v) => v,
            Err(e) => {
                println!("  ⚠ irasu API 响应解析失败: {}", e);
                return vec![];
            }
        },
        Err(e) => {
            println!("  ⚠ irasu API 请求失败: {}", e);
            return vec![];
        }
    };

    let entries = match json["feed"]["entry"].as_array() {
        Some(arr) => arr,
        None => return vec![],
    };

    let mut results: Vec<String> = vec![];
    for entry in entries {
        // 从 media$thumbnail.url 取缩略图，替换 s72-c → s800 得原图
        let thumb_url = match entry["media$thumbnail"]["url"].as_str() {
            Some(u) => u.to_string(),
            None => continue,
        };
        let image_url = upgrade_image_size(&thumb_url);
        results.push(image_url);
    }

    // 优先保留 PNG，没有 PNG 时接受 JPG
    let has_png = results.iter().any(|u| u.to_lowercase().ends_with(".png"));
    if has_png {
        results.retain(|u| u.to_lowercase().ends_with(".png"));
    }

    results
}

// ============================================================
// 搜索结果缓存
// ============================================================

fn index_path(dest_dir: &Path) -> PathBuf {
    let parent = dest_dir.parent().unwrap_or(dest_dir);
    parent.join("irasu_index.json")
}

fn load_index(dest_dir: &Path) -> HashMap<String, Vec<String>> {
    let path = index_path(dest_dir);
    if !path.exists() {
        return HashMap::new();
    }
    match std::fs::read_to_string(&path) {
        Ok(text) => serde_json::from_str(&text).unwrap_or_default(),
        Err(_) => HashMap::new(),
    }
}

fn save_index(dest_dir: &Path, index: &HashMap<String, Vec<String>>) {
    let path = index_path(dest_dir);
    if let Ok(text) = serde_json::to_string_pretty(index) {
        std::fs::write(&path, text).ok();
    }
}

/// 获取 keyword 对应的图片 URL：
/// 优先缓存，无缓存则调 API，写入缓存
/// (process 用它预验证 keyword 是否有图, 支持来源兜底)
pub async fn resolve_image_urls(
    keyword: &str,
    dest_dir: &Path,
    client: &reqwest::Client,
) -> Vec<String> {
    let index = load_index(dest_dir);
    if let Some(cached) = index.get(keyword)
        && !cached.is_empty() {
            return cached.clone();
        }

    let urls = fetch_entries(keyword, client).await;
    if urls.is_empty() {
        println!("  ⚠ irasu「{}」: 未找到匹配的图片", keyword);
        return vec![];
    }

    // 写入缓存
    let mut new_index = index;
    new_index.insert(keyword.to_string(), urls.clone());
    save_index(dest_dir, &new_index);

    urls
}

// ============================================================
// 工具函数
// ============================================================

/// 替换 Blogger 缩略图尺寸为 s800
fn upgrade_image_size(url: &str) -> String {
    let re = regex_lite::Regex::new(r"/s\d+(-c)?/").unwrap();
    re.replace(url, "/s800/").to_string()
}

/// 从 URL 猜测文件扩展名
fn guess_extension(url: &str) -> String {
    let base = url.split('?').next().unwrap_or(url);
    let base_lower = base.to_lowercase();
    if base_lower.ends_with(".png") {
        ".png".into()
    } else if base_lower.ends_with(".jpg") || base_lower.ends_with(".jpeg") {
        ".jpg".into()
    } else if base_lower.ends_with(".gif") {
        ".gif".into()
    } else if base_lower.ends_with(".webp") {
        ".webp".into()
    } else if let Some(last) = base.rsplit('.').next() {
        if !last.is_empty() && !last.contains('/') && last.len() <= 5 {
            format!(".{}", last)
        } else {
            ".png".into()
        }
    } else {
        ".png".into()
    }
}

/// URL 编码
fn urlencoding(keyword: &str) -> String {
    urlencoding::encode(keyword).to_string()
}

/// 下载文件，失败时写入紫色占位图
async fn download_file(client: &reqwest::Client, url: &str, dest: &Path) -> anyhow::Result<()> {
    let response = client.get(url).send().await?;
    if !response.status().is_success() {
        anyhow::bail!("HTTP {}", response.status());
    }
    let bytes = response.bytes().await?;
    std::fs::write(dest, &bytes)?;
    Ok(())
}

/// 清理文件名中的特殊字符
fn sanitize_filename(keyword: &str) -> String {
    keyword
        .chars()
        .map(|c| if c.is_alphanumeric() || c == '_' || c == '-' { c } else { '_' })
        .collect::<String>()
        .trim_matches('_')
        .to_string()
}

/// 从项目的所有 segments 中收集 irasu keywords
pub fn collect_keywords(segments: &[crate::project::Segment]) -> Vec<String> {
    use crate::project::Event;
    let mut keywords: Vec<String> = vec![];
    for seg in segments {
        for event in &seg.events {
            if let Event::Sticker { keyword, .. } = event {
                let kw = keyword.trim().to_string();
                if !kw.is_empty() && !keywords.contains(&kw) {
                    keywords.push(kw);
                }
            }
        }
    }
    keywords
}
