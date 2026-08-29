use std::path::{Path, PathBuf};
use std::collections::HashMap;
use serde::Deserialize;

const API_URL: &str = "https://googlefonts.github.io/noto-emoji-animation/data/api.json";
const CDN_BASE: &str = "https://fonts.gstatic.com/s/e/notoemoji/latest";

/// 编译时嵌入的 emoji catalog 条目
#[derive(Debug, Deserialize)]
struct CatalogEntry {
    #[allow(dead_code)]
    name: String,
    codepoint: String,
    #[allow(dead_code)]
    url: String,
    shortcodes: Vec<String>,
    tag: Vec<String>,
}

/// API 返回的单个 emoji 条目（远端 fallback 用）
#[derive(Debug, Deserialize)]
struct ApiIcon {
    #[allow(dead_code)]
    name: String,
    codepoint: String,
    tags: Vec<String>,
}

/// API 返回的完整结构（远端 fallback 用）
#[derive(Debug, Deserialize)]
struct ApiResponse {
    #[allow(dead_code)]
    icons: Vec<ApiIcon>,
}

/// 关键词 → codepoint 硬编码回退（当 API 搜索无匹配时使用）
const FALLBACK_MAP: &[(&str, &str)] = &[
    ("confused", "1f92f"),
    ("deny", "1f645"),
    ("realization", "1f4a1"),
    ("frustrated", "1f629"),
    ("angry", "1f621"),
    ("explain", "1f4ac"),
    ("shock", "1f631"),
    ("sweat", "1f629"),
    ("laugh", "1f606"),
    ("question", "2753"),
    ("cry", "1f62d"),
    ("scream", "1f631"),
    ("shy", "1f633"),
    ("nervous", "1f630"),
    ("happy", "1f600"),
    ("sad", "1f62d"),
    ("love", "1f970"),
    ("fire", "1f525"),
    ("party", "1f389"),
    ("thumbsup", "1f44d"),
    ("skull", "1f480"),
    ("handshake", "1f91d"),
    ("thinking", "1f914"),
    ("pout", "1f621"),
    ("anxiety", "1f630"),
    ("panic", "1f631"),
    ("scared", "1f631"),
    ("frightened", "1f631"),
    ("fall", "1f635"),
    ("dizzy", "1f635"),
    ("faint", "1f635"),
    ("game", "1f3ae"),
    ("dream", "1f634"),
    ("nightmare", "1f631"),
    ("sleep", "1f634"),
    ("yawn", "1f971"),
    ("relief", "1f60c"),
    ("surprise", "1f62e"),
    ("wow", "1f632"),
    ("amazing", "1f31f"),
    ("star", "2b50"),
    ("excited", "1f929"),
    ("yay", "1f973"),
    ("applause", "1f44f"),
    ("clap", "1f44f"),
    ("rock", "1f918"),
    ("dance", "1f483"),
    ("music", "1f3b5"),
    ("bomb", "1f4a3"),
    ("explosion", "1f4a5"),
    ("lightning", "26a1"),
    ("rain", "1f327"),
    ("snow", "2744"),
    ("cold", "1f976"),
    ("hot", "1f975"),
    ("sick", "1f912"),
    ("fever", "1f912"),
    ("vomit", "1f92e"),
    ("worry", "1f62c"),
    ("awkward", "1f613"),
    ("annoyed", "1f612"),
    ("tired", "1f62b"),
    ("heartbreak", "1f494"),
    ("broken", "1f494"),
    ("pray", "1f64f"),
    ("despair", "1f635"),
    ("tears", "1f62d"),
    ("crying", "1f62d"),
    ("victory", "270c"),
    ("peace", "270c"),
    ("ok", "1f44c"),
    ("yes", "1f44d"),
    ("no", "1f645"),
    ("stop", "270b"),
    ("hello", "1f44b"),
    ("bye", "1f44b"),
    ("wink", "1f609"),
    ("blush", "1f60a"),
    ("kiss", "1f618"),
    ("hungry", "1f35b"),
    ("eat", "1f35b"),
    ("drink", "1f37b"),
    ("cook", "1f373"),
    ("birthday", "1f382"),
    ("gift", "1f381"),
    ("money", "1f4b0"),
    ("rich", "1f4b8"),
    ("crown", "1f451"),
    ("king", "1f451"),
    ("ghost", "1f47b"),
    ("devil", "1f608"),
    ("angel", "1f607"),
    ("cat", "1f431"),
    ("dog", "1f436"),
    ("fox", "1f98a"),
    ("rabbit", "1f430"),
    ("panda", "1f43c"),
    ("frog", "1f438"),
    ("heart", "1f493"),
];

/// Emoji 搜索索引：keyword → [codepoint, ...]
pub struct EmojiCatalog {
    /// 规范化 keyword → 匹配的 codepoint 列表
    index: HashMap<String, Vec<String>>,
    /// 所有 codepoint 集合（用于无关键词时 fallback）
    all_codepoints: Vec<String>,
}

impl EmojiCatalog {
    /// 获取搜索索引：优先用本地嵌入 catalog，失败则 fallback 远端 API
    pub async fn fetch(client: &reqwest::Client) -> anyhow::Result<Self> {
        // 尝试本地嵌入 catalog
        match Self::from_embedded() {
            Ok(catalog) => return Ok(catalog),
            Err(e) => println!("  ⚠ emoji 本地目录加载失败: {}，尝试远端 API", e),
        }
        // fallback 远端 API
        Self::from_api(client).await
    }

    /// 从编译时嵌入的 emoji_catalog.json 构建索引
    fn from_embedded() -> anyhow::Result<Self> {
        const CATALOG_JSON: &str = include_str!("../../assets/emoji_catalog.json");
        let entries: Vec<CatalogEntry> = serde_json::from_str(CATALOG_JSON)?;

        let mut index: HashMap<String, Vec<String>> = HashMap::new();
        let mut all_codepoints = Vec::new();

        for entry in &entries {
            let cp = entry.codepoint.clone();
            all_codepoints.push(cp.clone());

            // 用 LLM 标签建索引（主要搜索路径）
            for tag in &entry.tag {
                index.entry(tag.to_lowercase()).or_default().push(cp.clone());
            }

            // 也保留短代码索引（向后兼容）
            for sc in &entry.shortcodes {
                let kw = sc.trim_matches(':').to_lowercase();
                if !kw.is_empty() {
                    index.entry(kw).or_default().push(cp.clone());
                }
            }
        }

        Ok(EmojiCatalog { index, all_codepoints })
    }

    /// 从远端 API 构建索引（fallback）
    async fn from_api(client: &reqwest::Client) -> anyhow::Result<Self> {
        let resp = client.get(API_URL).send().await?;
        let api: ApiResponse = resp.json().await?;

        let mut index: HashMap<String, Vec<String>> = HashMap::new();
        let mut all_codepoints = Vec::new();

        for icon in &api.icons {
            all_codepoints.push(icon.codepoint.clone());

            for tag in &icon.tags {
                let kw = tag.trim_matches(':').to_lowercase();
                if kw.is_empty() {
                    continue;
                }
                index.entry(kw).or_default().push(icon.codepoint.clone());
            }
        }

        Ok(EmojiCatalog { index, all_codepoints })
    }

    /// 搜索匹配关键词的 emoji codepoint 列表
    pub fn search(&self, keyword: &str, max: usize) -> Vec<String> {
        let kw = keyword.to_lowercase().trim().to_string();

        // 1. 精确匹配 tag/短代码
        if let Some(results) = self.index.get(&kw) {
            let mut r = results.clone();
            r.truncate(max);
            return r;
        }

        // 2. 模糊匹配 tag 中包含关键词的
        let fuzzy: Vec<&String> = self.all_codepoints.iter()
            .filter(|cp| {
                self.index.iter().any(|(tag, cps)| {
                    tag.contains(&kw) && cps.contains(cp)
                })
            })
            .collect();
        if !fuzzy.is_empty() {
            let mut r = fuzzy.into_iter().cloned().collect::<Vec<_>>();
            r.truncate(max);
            return r;
        }

        // 3. 硬编码回退映射
        for (key, cp) in FALLBACK_MAP {
            if *key == kw {
                return vec![cp.to_string()];
            }
        }

        vec![]
    }

    /// 获取所有可用 emoji 的 codepoint 列表
    #[allow(dead_code)]
    pub fn all(&self) -> &[String] {
        &self.all_codepoints
    }
}

/// 下载匹配关键词的 emoji GIF
/// 返回 (新下载文件列表, 已存在文件数)
pub async fn download_images(
    keywords: &[String],
    dest_dir: &Path,
    client: &reqwest::Client,
    max_per_keyword: usize,
) -> (Vec<PathBuf>, usize) {
    if keywords.is_empty() || max_per_keyword == 0 {
        return (vec![], 0);
    }

    let catalog = match EmojiCatalog::fetch(client).await {
        Ok(c) => c,
        Err(e) => {
            println!("  ⚠ emoji 目录获取失败: {}", e);
            return (vec![], 0);
        }
    };

    let mut downloaded = vec![];
    let mut existing = 0;
    for keyword in keywords {
        if keyword.trim().is_empty() {
            continue;
        }
        let codepoints = catalog.search(keyword, max_per_keyword);
        if codepoints.is_empty() {
            println!("  ⚠ emoji 「{}」: 未找到匹配", keyword);
            continue;
        }

        for cp in &codepoints {
            let path = dest_dir.join(format!("{}.gif", cp));
            if path.exists() {
                existing += 1;
                continue;
            }

            let url = format!("{}/{}/512.gif", CDN_BASE, cp);
            match download_file(client, &url, &path).await {
                Ok(()) => {
                    println!("  ✓ emoji {} → {}", cp, path.display());
                    downloaded.push(path);
                }
                Err(e) => {
                    println!("  ⚠ emoji {} 下载失败: {}", cp, e);
                }
            }
        }
    }

    (downloaded, existing)
}

/// 下载指定 codepoint 的 emoji GIF (process 已解析出具体 codepoint 时用, 保证与 name 一致)
/// 返回 (新下载文件列表, 已存在文件数)
pub async fn download_images_by_codepoints(
    codepoints: &[String],
    dest_dir: &Path,
    client: &reqwest::Client,
) -> (Vec<PathBuf>, usize) {
    let mut downloaded = vec![];
    let mut existing = 0;
    for cp in codepoints {
        if cp.is_empty() || !cp.chars().all(|c| c.is_ascii_hexdigit()) {
            continue;
        }
        let path = dest_dir.join(format!("{}.gif", cp));
        if path.exists() {
            existing += 1;
            continue;
        }
        let url = format!("{}/{}/512.gif", CDN_BASE, cp);
        match download_file(client, &url, &path).await {
            Ok(()) => {
                println!("  ✓ emoji {} → {}", cp, path.display());
                downloaded.push(path);
            }
            Err(e) => {
                println!("  ⚠ emoji {} 下载失败: {}", cp, e);
            }
        }
    }
    (downloaded, existing)
}

/// 从项目的所有 segments 中收集唯一的 emoji keywords
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

async fn download_file(client: &reqwest::Client, url: &str, dest: &Path) -> anyhow::Result<()> {
    let response = client.get(url).send().await?;
    if !response.status().is_success() {
        anyhow::bail!("HTTP {}", response.status());
    }
    let bytes = response.bytes().await?;
    std::fs::write(dest, &bytes)?;
    Ok(())
}
