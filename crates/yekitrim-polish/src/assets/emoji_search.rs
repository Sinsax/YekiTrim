//! polish::assets::emoji_search：Noto 动态 emoji 的检索与下载。
//!
//! 入口：`EmojiCatalog::{fetch, search, all}`、`download_images`、`download_images_by_codepoints`、`collect_keywords`。
//! 边界：走 Google Fonts 的公开 API/CDN，目录拿不到时退回编译期嵌入的 `FALLBACK_MAP`；图片扩展名判据统一用 `sticker_index::is_image_file_name` —— 见 `crates/yekitrim-polish/src/assets/AGENTS.md`

use std::path::{Path, PathBuf};
use std::collections::HashMap;
use serde::Deserialize;

use super::emoji_taxonomy;

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
    /// **六维分类**（2026-10-17）：`{emotion:[…], intensity:[…], action:[…], object:[…], usage:[…], tone:[…]}`。
    /// 维度键与取值由 `assets/emoji_taxonomy.json` 定义（**词表真源**），这里只做形状校验。
    /// 老数据（没有 `dim`）照样能读 —— 那时 `search_dims` 空转、`grouped()` 走 tag 老路。
    #[serde(default)]
    dim: HashMap<String, Vec<String>>,
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
    /// codepoint → 维度 → 取值（`search_dims` 打分用）。空 = 老数据（没有 `dim`）。
    dims: HashMap<String, HashMap<String, Vec<String>>>,
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
        let mut dims: HashMap<String, HashMap<String, Vec<String>>> = HashMap::new();

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

            // 六维分类（`search_dims` 用）。⚠️ **只收词表里的取值** ——
            // 数据是构建工具写进来的，但"词表改了、JSON 没重跑"是常态，
            // 收下词表外的值只会让查询悄悄命中一个谁也没定义过的维度。
            if !entry.dim.is_empty() {
                let mut kept: HashMap<String, Vec<String>> = HashMap::new();
                for (dim, values) in &entry.dim {
                    let vals: Vec<String> = values
                        .iter()
                        .map(|v| v.trim().to_ascii_lowercase())
                        .filter(|v| !v.is_empty() && emoji_taxonomy::is_valid_value(dim, v))
                        .collect();
                    if !vals.is_empty() {
                        kept.insert(dim.trim().to_ascii_lowercase(), vals);
                    }
                }
                if !kept.is_empty() {
                    dims.insert(cp.clone(), kept);
                }
            }
        }

        Ok(EmojiCatalog { tag_index, index, all_codepoints, dims })
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

        // 远端 API 既没有 LLM 标签、也没有六维分类 ⇒ 三张表都给空
        // （`grouped()` 回退到 `index`、`search_dims` 空转、`search` 照旧）
        Ok(EmojiCatalog {
            tag_index: HashMap::new(),
            index,
            all_codepoints,
            dims: HashMap::new(),
        })
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

    /// **按维度查询**打分排序，返回最贴合的 `max` 个 codepoint。
    ///
    /// `query` 来自 `emoji_taxonomy::parse_query`（如 `emotion:joy,usage:punchline`）——
    /// **只含词表内的取值**，所以打分不会碰上"谁也没定义过的维度"。
    /// 打分 = 命中的每个 (维度, 取值) 加**该维度的权重**；0 分的直接淘汰。
    /// 同分按**目录顺序**（稳定：同一份数据、同一条查询，结果永远一样）。
    pub fn search_dims(&self, query: &[(String, String)], max: usize) -> Vec<String> {
        if query.is_empty() || max == 0 || self.dims.is_empty() {
            return Vec::new();
        }
        let mut scored: Vec<(f32, usize, String)> = Vec::new();
        for (i, cp) in self.all_codepoints.iter().enumerate() {
            let Some(d) = self.dims.get(cp) else { continue };
            let mut score = 0.0f32;
            for (dim, value) in query {
                if d.get(dim).is_some_and(|vs| vs.iter().any(|v| v == value)) {
                    score += emoji_taxonomy::find_dim(dim).map(|s| s.weight).unwrap_or(1.0);
                }
            }
            if score > 0.0 {
                scored.push((score, i, cp.clone()));
            }
        }
        scored.sort_by(|a, b| {
            b.0.partial_cmp(&a.0)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(a.1.cmp(&b.1))
        });
        scored.into_iter().take(max).map(|(_, _, cp)| cp).collect()
    }

    /// **选取入口**（`process` Step 2 调它）：把 LLM 写的关键词变成候选表情。
    ///
    /// 三级，从"最准"到"最宽"：
    /// ① **分维查询** —— 关键词是 `emotion:joy,usage:punchline` 这种紧凑串，
    ///    或者是一个能对上词表的自由词（`joy`/`game`，走 `guess_dims`）⇒ `search_dims` 打分排序；
    /// ② 旧的**精确 + 子串**匹配（`search`）—— 老工程与词表外的新词都靠它兜底；
    /// ③ 都没有 ⇒ 空（调用方去试下一个来源）。
    ///
    /// ⚠️ ①**不吞掉**②：即使分维查询有结果也不再走 ②（打分已经排过序了），
    /// 但分维查询**没结果**时必须落回 ② —— 否则一条老工程的 `keyword = "angry"`
    /// （词表里只有 `anger`/`annoyance`）会解析不出表情。
    pub fn search_best(&self, keyword: &str, max: usize) -> Vec<String> {
        let mut q = emoji_taxonomy::parse_query(keyword);
        if q.is_empty() {
            q = emoji_taxonomy::guess_dims(keyword);
        }
        if !q.is_empty() {
            let hits = self.search_dims(&q, max);
            if !hits.is_empty() {
                return hits;
            }
        }
        self.search(keyword, max)
    }

    /// 目录里**有没有**六维分类（构建工具 `polish emoji build` 跑过没有）。
    /// 老数据 / 远端 API 都是 `false` ⇒ 面板走 tag 老路、`search_dims` 空转。
    pub fn has_dims(&self) -> bool {
        !self.dims.is_empty()
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
        // ⚠️ **有六维分类时按「主维度」分区**（2026-10-17）：这才是"分类"该有的形状 ——
        //    一个维度、每组几十条。按"所有 tag 的并集"分区会退回"几十个标签、三成只有 1 条"
        //    （用户口径：「当前 emoji 包的类型有几十个，这根本不对」）。
        //    没有 dim（老数据 / 远端 API）⇒ 走下面的 tag 老路（零迁移）。
        if !self.dims.is_empty() && emoji_taxonomy::primary_dim().is_some() {
            let primary = emoji_taxonomy::primary_dim().expect("上面刚判过").key.clone();
            let mut grouped: std::collections::BTreeMap<String, Vec<String>> =
                std::collections::BTreeMap::new();
            let mut ungrouped: Vec<String> = Vec::new();
            for cp in &self.all_codepoints {
                match self
                    .dims
                    .get(cp)
                    .and_then(|d| d.get(&primary))
                    .and_then(|vs| vs.first())
                {
                    Some(v) => grouped.entry(v.clone()).or_default().push(cp.clone()),
                    // ⚠️ **没分到组的一个都不许丢**：面板的「全部（不限）」是从各组拼出来的，
                    //    丢掉它们 = 这些表情在面板上永远看不见（静默消失）。
                    None => ungrouped.push(cp.clone()),
                }
            }
            let mut out: Vec<(String, Vec<String>)> = grouped.into_iter().collect();
            if !ungrouped.is_empty() {
                out.push(("uncategorized".to_string(), ungrouped));
            }
            return out;
        }
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

/// `cp` 能不能当"下载键"用 —— **多码点序列（`203c_fe0f`）必须放行**。
///
/// ⚠️⚠️ 判据要**逐段**看：目录 881 条里 **343 条**是多码点（`_` 分隔）。
/// 写成 `cp.chars().all(|c| c.is_ascii_hexdigit())` 会把它们**整批静默丢掉**
/// （走 `continue`，一条日志都没有）—— 而调用点 `downloader.rs` 刚刚**特意**把
/// `_` 放行（那里的注释写着"否则 39% 的表情会退回按 keyword 搜首个"），
/// 于是两边各自看都对：调用点以为传进来了，这里一声不响地不要。
///
/// 症状链（2026-10-17 实测）：`([], 0)` ⇒ `<片段>/assets/stickers/` 里没有这个文件
/// ⇒ 转译器 `findStickerFile` 找不到 → **DROP**（成片里没有它）
/// ⇒ DOM 预览 `stickerSrc` 返回空 → 退回 `<span>{{ st.emoji }}</span>`（**静态字符**）。
/// 用户看到的就是"表情是静态的"。
///
/// 文件名本身就带 `_`（`203c_fe0f.gif`），与 `index.html` / `engine.js::safeEmoji`
/// 的解码口径一致。
fn is_downloadable_key(cp: &str) -> bool {
    !cp.is_empty()
        && cp.split('_')
            .all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_hexdigit()))
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
        if !is_downloadable_key(cp) {
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

        // ③ **分组口径两档**（2026-10-17 改）：
        //    有六维分类 ⇒ 按**主维度**分区，主维度是单选 ⇒ 每组互斥、**各组之和 == 目录条数**；
        //    没有 dim（老数据 / 远端 API）⇒ 走 tag 老路，那才是重叠视图（同一条落在多个 tag 下）。
        //
        //    ⚠️ 这里原先写死的是「各组之和 > 目录条数」—— 那是旧口径，而**重叠正是**
        //    「几十个标签、三成只有 1 条」的来源（用户口径：「类型有几十个，这根本不对」）。
        //    两档都钉住：谁把分区改回重叠、或把老路改成互斥，都会红。
        let total: usize = groups.iter().map(|(_, cps)| cps.len()).sum();
        if cat.has_dims() {
            assert_eq!(
                total,
                cat.all().len(),
                "主维度是单选 ⇒ 分组必须是分区（各组之和必须等于目录条数）"
            );
        } else {
            assert!(
                total > cat.all().len(),
                "没有 dim 时走 tag 老路，那时才是重叠视图（和为 {total}、条数 {}）",
                cat.all().len()
            );
        }
    }

    /// **面板的「分类」页必须是用户的 LLM 分类标签，不是官方短名**（2026-10-17 修）。
    ///
    /// 这条抓的是一个真实事故：`grouped()` 原先遍历 `index`，而 `index` 里**同时**装着
    /// 作者的 `tag` 与官方 `shortcodes`（`:smile:` → `smile`、`:1st-place-medal:` →
    /// `1st-place-medal`）⇒ 面板「分类」页变成 **686 个**短名（`!!` / `+1` / `100` /
    /// `1st-place-medal`…），作者那套分类被淹没在里面（用户报的就是这个）。
    /// 修法 = `tag_index`（**只由 `tag` 建**）+ `grouped()` 只认它。
    ///
    /// ⚠️ 判据钉的是**形状**（分类标签一律以字母开头，官方短名常以数字/符号开头），
    /// 不是条数 —— 重新分类时条数会变，但"短名混进来"永远是错的。
    /// **变异自证**：把 `grouped()` 里的 `src` 换回 `&self.index` ⇒ 当场红。
    #[test]
    fn emoji_panel_groups_are_llm_tags_not_official_shortcodes() {
        let cat = EmojiCatalog::fetch_embedded().expect("内嵌目录");
        let groups = cat.grouped();
        let bad: Vec<&String> = groups
            .iter()
            .map(|(t, _)| t)
            .filter(|t| !t.starts_with(|c: char| c.is_ascii_lowercase()))
            .collect();
        assert!(
            bad.is_empty(),
            "「分类」页混进了 {} 个官方短名（应当只按 LLM tag 分组）: {:?}",
            bad.len(),
            &bad[..bad.len().min(8)]
        );
        assert!(
            groups.len() < 200,
            "分类有 {} 组 —— 正常是几十组（用户那套分类）；上百组基本就是短名又漏进来了",
            groups.len()
        );
        // 打印一次，肉眼可控（`--nocapture` 时可见）
        let mut names: Vec<&String> = groups.iter().map(|(t, _)| t).collect();
        names.sort();
        println!(
            "grouped() = {} 组，目录 {} 条：{:?} …",
            groups.len(),
            cat.all().len(),
            &names[..names.len().min(10)]
        );
    }

/// 造一份**合成目录**。
    ///
    /// ⚠️ 刻意不依赖 include_str! 那份真实数据：它有没有 dim 取决于构建工具跑没跑过，
    /// 用它当判据 = 用例会随数据状态时红时绿。这里的形状与真实数据同构即可。
    ///
    /// all_codepoints 有意把 1f622 排在 1f602 **前面** —— 这样"同分按目录顺序"
    /// 与"高分优先"两条断言会给出**不同**的顺序，两条都不空转。
    fn synthetic() -> EmojiCatalog {
        let mut dims: HashMap<String, HashMap<String, Vec<String>>> = HashMap::new();
        dims.insert(
            "1f602".to_string(),
            HashMap::from([
                ("emotion".to_string(), vec!["joy".to_string()]),
                ("usage".to_string(), vec!["punchline".to_string(), "reaction".to_string()]),
                ("tone".to_string(), vec!["positive".to_string()]),
            ]),
        );
        dims.insert(
            "1f622".to_string(),
            HashMap::from([
                ("emotion".to_string(), vec!["sadness".to_string()]),
                ("usage".to_string(), vec!["reaction".to_string()]),
            ]),
        );
        dims.insert(
            "1f621".to_string(),
            HashMap::from([
                ("emotion".to_string(), vec!["anger".to_string()]),
                ("intensity".to_string(), vec!["3".to_string()]),
            ]),
        );
        EmojiCatalog {
            tag_index: HashMap::new(),
            // 老工程那条兜底路（词表外的自由词）要有东西可命中
            index: HashMap::from([("lol".to_string(), vec!["1f621".to_string()])]),
            all_codepoints: vec![
                "1f622".to_string(),
                "1f602".to_string(),
                "1f621".to_string(),
                "1f600".to_string(),
            ],
            dims,
        }
    }

    /// 打分按**维度权重**排、同分按**目录顺序**（稳定）。
    ///
    /// 变异自证：把权重加成改成固定的 +1.0 → 第一条断言红（两个候选会变成同分）。
    #[test]
    fn search_dims_ranks_by_weight_then_directory_order() {
        let cat = synthetic();
        let hits = cat.search_dims(
            &[
                ("emotion".to_string(), "joy".to_string()),
                ("usage".to_string(), "reaction".to_string()),
            ],
            8,
        );
        assert_eq!(hits, vec!["1f602", "1f622"], "分高的必须排前面");
        let tie = cat.search_dims(&[("usage".to_string(), "reaction".to_string())], 8);
        assert_eq!(tie, vec!["1f622", "1f602"], "同分要按目录顺序，保证稳定");
        assert!(cat.search_dims(&[("emotion".to_string(), "fear".to_string())], 8).is_empty());
        assert!(cat.search_dims(&[], 8).is_empty());
        assert!(cat.search_dims(&[("emotion".to_string(), "joy".to_string())], 0).is_empty());
    }

    /// 选取入口三级：紧凑查询 → 词表自由词 → 旧的自由词兜底。
    ///
    /// 变异自证：把最后那句 self.search(keyword, max) 去掉 → 第三条断言红
    /// （老工程那条 keyword = lol 会解析不出表情）。
    #[test]
    fn search_best_takes_query_then_vocabulary_then_legacy() {
        let cat = synthetic();
        assert_eq!(cat.search_best("emotion:joy", 8), vec!["1f602"]);
        assert_eq!(cat.search_best("joy", 8), vec!["1f602"]);
        assert_eq!(cat.search_best("lol", 8), vec!["1f621"], "旧工程的 keyword 必须还能命中");
        assert!(cat.search_best("zzzz", 8).is_empty());
    }

    /// 面板按**主维度**分区，且**一个 codepoint 都不许丢**（没分到组的进 uncategorized）。
    ///
    /// 变异自证：删掉 ungrouped 那一段 → 第二条断言红
    /// （那个没分类的表情会在面板上永远看不见 —— 静默消失）。
    #[test]
    fn grouped_by_primary_dim_never_drops_a_codepoint() {
        let cat = synthetic();
        let groups = cat.grouped();
        let names: Vec<&String> = groups.iter().map(|(t, _)| t).collect();
        assert!(
            names.contains(&&"joy".to_string()),
            "分组键应当是主维度的取值: {names:?}"
        );
        assert!(
            names.contains(&&"uncategorized".to_string()),
            "没有 dim 的 codepoint 必须落到 uncategorized: {names:?}"
        );
        let total: usize = groups.iter().map(|(_, c)| c.len()).sum();
        assert_eq!(total, cat.all().len(), "分组不许丢 codepoint（面板的全部靠它拼）");
        let mut seen = std::collections::HashSet::new();
        for (_, cps) in &groups {
            for cp in cps {
                assert!(seen.insert(cp.clone()), "{cp} 出现在多个组里 —— 主维度应当是单选");
            }
        }
    }

/// **数据质量门禁**：六维分类真的落进目录了吗、词表是不是"闭合"的。
    ///
    /// 与上面几条用**合成目录**的用例相反，这条读的是 include_str! 那份**真实数据** ——
    /// 它就是"构建工具跑完之后的验收判据"。
    ///
    /// 变异自证：把目录里的 dim 字段删掉 ⇒ 第一条红；
    /// 把某个维度的取值砍掉大半 ⇒ 第三条红（零覆盖超标）。
    #[test]
    fn embedded_catalog_is_multi_dimensional_and_closed() {
        let cat = EmojiCatalog::fetch_embedded().expect("内嵌目录");
        assert!(
            cat.has_dims(),
            "目录里没有 dim —— 构建工具没跑过（polish emoji build）"
        );
        let missing = cat.all().iter().filter(|cp| !cat.dims.contains_key(*cp)).count();
        assert_eq!(missing, 0, "有 {missing} 条没分类（面板上它们只会落进 uncategorized）");

        let mut zero: Vec<String> = Vec::new();
        let mut total = 0usize;
        for d in &emoji_taxonomy::taxonomy().dims {
            for v in &d.values {
                total += 1;
                let hit = cat
                    .dims
                    .values()
                    .any(|m| m.get(&d.key).is_some_and(|vs| vs.iter().any(|x| x == v)));
                if !hit {
                    zero.push(format!("{}.{}", d.key, v));
                }
            }
        }
        // 允许极少数取值在本目录里没有成员（实测 1/85：action.salute —— 🫡 其实在目录里，
        // 是模型没给），但**不许成片为零**：那些词写进 prompt 就是"能写、永远选不到"。
        assert!(
            zero.len() * 20 <= total,
            "零覆盖的取值太多（{} / {total}）：{zero:?}",
            zero.len()
        );
        // 主维度是面板的分区维度：它的取值数决定面板有几组，太少就是"分类退化"。
        let primary = emoji_taxonomy::primary_dim().expect("词表必须有主维度");
        let groups = primary
            .values
            .iter()
            .filter(|v| {
                cat.dims
                    .values()
                    .any(|m| m.get(&primary.key).is_some_and(|vs| vs.iter().any(|x| x == *v)))
            })
            .count();
        assert!(groups >= 10, "{} 只有 {groups} 组有成员 —— 分类退化了", primary.key);
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

    /// **自动下载那条路的入口判据**：目录里每一条都要能被下载器认下来。
    ///
    /// ⚠️ 这条抓的是"面板上有、项目里没有"：认不下来就 `continue`（**静默**），
    /// 于是 GIF 从没落进 `<片段>/assets/stickers/` ⇒ 转译器 DROP、DOM 预览退回静态字符。
    /// **变异自证**：把 `is_downloadable_key` 换回老写法
    /// `!cp.is_empty() && cp.chars().all(|c| c.is_ascii_hexdigit())` ⇒ 当场红（343 条）。
    #[test]
    fn every_catalog_codepoint_is_downloadable() {
        let cat = EmojiCatalog::fetch_embedded().expect("内嵌目录应当解得开");
        let bad: Vec<&String> = cat
            .all()
            .iter()
            .filter(|c| !is_downloadable_key(c))
            .collect();
        assert!(
            bad.is_empty(),
            "这些码点下载器不认（会被**静默**丢弃，成片缺图）: {bad:?}"
        );
        // 多码点那 343 条正是这条用例的意义所在 —— 一条都没有就说明目录退化了、用例在空转。
        let seq = cat.all().iter().filter(|c| c.contains('_')).count();
        assert!(seq > 100, "目录里应当有三百多条多码点序列，实测 {seq}");
    }

    /// 判据本身的正反面（不依赖目录内容）。
    #[test]
    fn downloadable_key_accepts_sequences_and_rejects_junk() {
        for ok in ["1f62d", "203c_fe0f", "1f642_200d_2195_fe0f"] {
            assert!(is_downloadable_key(ok), "{ok:?} 应当可下载");
        }
        for bad in ["", "_", "abc_", "_abc", "zz", "1f_zz", "1f62d_", " 1f62d"] {
            assert!(!is_downloadable_key(bad), "{bad:?} 不该被当成码点");
        }
        // ⚠️ **别把老写法当等价物**：这正是那次静默丢失的形状。
        assert!(
            !"203c_fe0f".chars().all(|c| c.is_ascii_hexdigit()),
            "老写法对多码点是 false —— 它就是 343 条被丢的原因"
        );
    }
}
