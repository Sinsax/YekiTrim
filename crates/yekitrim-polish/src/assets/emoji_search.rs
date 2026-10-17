//! polish::assets::emoji_search：Noto 动态 emoji 的检索与下载。
//!
//! 入口：`EmojiCatalog::{fetch, search, all}`、`download_images`、`download_images_by_codepoints`、`collect_keywords`。
//! 边界：走 Google Fonts 的公开 API/CDN，目录拿不到时退回编译期嵌入的 `FALLBACK_MAP`；图片扩展名判据统一用 `sticker_index::is_image_file_name` —— 见 `crates/yekitrim-polish/src/assets/AGENTS.md`

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
    /// **只按「分类标签」建的分组索引**（`grouped()` 用它）。
    ///
    /// ⚠️ 为什么必须与 `index` 分开（2026-10-17 真 bug）：`index` 里同时有 `tag`（用户/LLM 的分类）
    /// 和 `shortcodes`（`:smile:` 这种官方短名）—— 先前 `grouped()` 直接遍历 `index` ⇒
    /// 选择器的「分类」把 2000 多个短名也列成了分类（`1st-place-medal` / `!!` / `+1` …）。
    /// 分组只该用 `tag`。
    tag_index: HashMap<String, Vec<String>>,
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

    /// **只用内嵌目录**建索引（不联网）。
    ///
    /// ⚠️ 给"本来就不该依赖网络"的调用方用（编辑器表情面板那条接口）：
    /// `fetch()` 会先试内嵌、再回落远端 API，而回落那一步需要 `reqwest::Client` ——
    /// 只想拿内嵌目录的调用方不该被迫去建一个 HTTP 客户端（甚至不该被迫依赖 reqwest）。
    pub fn fetch_embedded() -> anyhow::Result<Self> {
        Self::from_embedded()
    }

    /// 从编译时嵌入的 emoji_catalog.json 构建索引
    fn from_embedded() -> anyhow::Result<Self> {
        const CATALOG_JSON: &str = include_str!("../../assets/emoji_catalog.json");
        let entries: Vec<CatalogEntry> = serde_json::from_str(CATALOG_JSON)?;

        let mut tag_index: HashMap<String, Vec<String>> = HashMap::new();
        let mut index: HashMap<String, Vec<String>> = HashMap::new();
        let mut all_codepoints = Vec::new();

        for entry in &entries {
            let cp = entry.codepoint.clone();
            all_codepoints.push(cp.clone());

            // 用 LLM 标签建索引（主要搜索路径）
            for tag in &entry.tag {
                tag_index.entry(tag.to_lowercase()).or_default().push(cp.clone());
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

        Ok(EmojiCatalog { tag_index, index, all_codepoints })
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

        // 远端 API 没有 LLM 标签 ⇒ 直接给空表（`grouped()` 会回退到 `index`）
        Ok(EmojiCatalog { tag_index: HashMap::new(), index, all_codepoints })
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

    /// **给编辑器的表情面板用**：整份目录，按 `tag` 分组。
    ///
    /// 返回 `(tag, [codepoint, …])`，组内保持 catalog 的原始顺序、组间按 tag 字典序。
    ///
    /// ⚠️ 为什么要单开一条而不是让前端自己拼：`search()` 是**按一个关键词**查的
    /// （编辑器要的是"打开面板就有一片可点的表情"），而目录里 881 条、
    /// 前端拿不到 `emoji_catalog.json`（它经 `include_str!` 只在 Rust 侧）。
    ///
    /// ⚠️ 组是**tag 不是分类**：同一条 emoji 会落在多个 tag 下（`happy` + `emotion`），
    /// 所以各组加起来会多于 `all().len()` —— 这是有意的（用户按情绪找，不按树找）。
    pub fn grouped(&self) -> Vec<(String, Vec<String>)> {
        let mut groups: std::collections::BTreeMap<String, Vec<String>> = std::collections::BTreeMap::new();
        // ⚠️ **只按 tag 分组**（用户/LLM 的分类）。`index` 里还有 shortcodes（`:smile:`）——
        //    混进去会让「分类」多出 2000 多个官方短名（`1st-place-medal` / `!!` / `+1`…，2026-10-17 真 bug）。
        //    远端 API 那条路没有 LLM 标签 ⇒ `tag_index` 为空时回退到 `index`（老行为）。
        let src = if self.tag_index.is_empty() { &self.index } else { &self.tag_index };
        for (tag, cps) in src {
            groups.entry(tag.clone()).or_default().extend(cps.iter().cloned());
        }
        // 组内去重并保持原顺序（同一个 cp 可能在 index 里对同一 tag 命中两次：
        // 它既是 `tag` 又是 `shortcode`）
        groups
            .into_iter()
            .map(|(tag, cps)| {
                let mut seen = std::collections::HashSet::new();
                let uniq: Vec<String> = cps.into_iter().filter(|c| seen.insert(c.clone())).collect();
                (tag, uniq)
            })
            .collect()
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
    let mut keywords: Vec<String> = vec![];
    for seg in segments {
        for event in &seg.events {
            // 两个载体都认（编辑器「贴纸轨」建的是 `Element{kind:"sticker"}`）
            if let Some(keyword) = event.sticker_keyword() {
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

#[cfg(test)]
mod tests {
    use super::*;

    /// **编辑器表情面板的数据源**（`GET /api/polish/emoji-catalog`）。
    ///
    /// 钉三件事：① 内嵌目录真的解得开（`include_str!` 的路径与 JSON 形状）；
    /// ② `grouped()` 的每条 codepoint **都是能画的**；
    /// ③ 分组**不是**简单切分 —— 同一条 emoji 应当出现在多个 tag 下。
    ///
    /// ⚠️ ③ 是刻意的：用户按情绪找表情，同一个 😭 既是 `tears` 也是 `crying`。
    /// 谁要是把 `grouped()` 顺手优化成互斥分区，这条会红。
    #[test]
    fn embedded_catalog_groups_for_the_emoji_panel() {
        let cat = EmojiCatalog::fetch_embedded().expect("内嵌目录应当解得开");
        let groups = cat.grouped();
        assert!(!groups.is_empty(), "分组不许为空（面板会显示成「没有可选表情」）");

        // ① 组内去重、且每组都有内容
        for (tag, cps) in &groups {
            assert!(!cps.is_empty(), "空组 {tag:?} 不该出现在面板上");
            let mut seen = std::collections::HashSet::new();
            for cp in cps {
                assert!(seen.insert(cp), "组 {tag:?} 里 {cp} 重复了");
            }
        }

        // ② 每条都是**面板能画的**（`engine.js::safeEmoji` 按同一套规则解码，
        //    所以这里必须逐段校验，而不是当成单个十六进制串）。
        //
        //    ⚠️⚠️ **可能是多码点序列**，用 `_` 分隔 —— 目录 881 条里 **343 条**是这种
        //    （`203c_fe0f` = ‼️、`1f642_200d_2195_fe0f` = 🙂↕️）。
        //    这条断言最初按"单个十六进制"写，它**当场就红了** —— 也就是它抓出了
        //    "近四成表情在前端解不出来"这个真实缺陷。**别把它放宽成只看 `is_ascii_hexdigit`**：
        //    那样 343 条又变回"面板上有、点下去画不出来"。
        let mut seq_count = 0usize;
        for (tag, cps) in &groups {
            for cp in cps {
                let parts: Vec<&str> = cp.split('_').collect();
                if parts.len() > 1 { seq_count += 1; }
                for p in &parts {
                    let n = u32::from_str_radix(p, 16)
                        .unwrap_or_else(|_| panic!("{tag:?} 里的 {cp:?} 有一段不是十六进制: {p:?}"));
                    assert!(n > 0 && n <= 0x10FFFF, "{tag:?} 里的 {cp:?} 有段越界: {p:?}");
                    assert!(char::from_u32(n).is_some(), "{p:?} 不是合法 char");
                }
            }
        }
        assert!(
            seq_count > 0,
            "一个多码点序列都没有？目录里应当有三百多条（这条在防分组被改写）"
        );

        // ③ 同一条出现在多个 tag 下（分组是 tag 视图、不是互斥分区）
        let total: usize = groups.iter().map(|(_, cps)| cps.len()).sum();
        assert!(
            total > cat.all().len(),
            "各组之和({total})应当多于目录条数({})—— 同一条会落在多个 tag 下",
            cat.all().len()
        );
    }

    /// 面板上点一个码点 → `name` 就是这个字符串 ⇒ 它必须能被 `search()` 反查回来。
    /// （点完下次打开面板，选中态要认得出来。）
    #[test]
    fn a_panel_codepoint_is_searchable_back() {
        let cat = EmojiCatalog::fetch_embedded().expect("内嵌目录");
        let groups = cat.grouped();
        let (tag, cps) = groups.iter().find(|(_, c)| !c.is_empty()).expect("至少一组");
        // ⚠️ 用一个**面板上真实存在**的码点（不是编一个），否则这条测的是幻觉。
        let cp = &cps[0];
        let hits = cat.search(tag, 64);
        assert!(
            hits.contains(cp),
            "组 {tag:?} 的第一个码点 {cp} 用同一个 tag 反查不到（面板选中态会失效）"
        );
    }
}
