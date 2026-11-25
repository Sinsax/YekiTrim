//! polish::assets::sfx_catalog：内建音效目录（编译期嵌入 `sfx_catalog.json`）与查询 / 屏蔽 / 候选与模糊搜索。
//!
//! 入口：`all`、`lookup`、`lookup_variant`、`lookup_label`、`is_blacklisted`、`search_by_tags`、`candidates_for_tags`、`candidates_text`、`fuzzy_search`、`categories_text`、`all_tags`、`CATEGORY_ORDER`、`SfxEntry`。
//! 边界：屏蔽语义 = 精确 id 屏蔽单个、`tag:xxx` 屏蔽整个标签族；项目级与全局黑名单由 `process::load_sfx_blacklist` 求并集 —— 见 `crates/yekitrim-polish/src/assets/AGENTS.md`

/// 音效数据库 — 从 assets/sfx_catalog.json 编译时嵌入（小森平/効果音ラボ全量 + 变体归组）
use serde::{Deserialize, Serialize};
use std::sync::OnceLock;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SfxEntry {
    /// 组 id（变体归组后的语义名，LLM 看到的就是它）
    pub id: String,
    /// 大分类（如 "① 主播情绪反应"）
    pub cat: String,
    /// 日文原始标题（辅助理解）
    pub label: String,
    /// 源页面目录名（用于拼下载 URL）
    pub page: String,
    /// 该组下的变体文件名列表（如 ["knock_on_mtl_door1.mp3", ...]）
    pub variants: Vec<String>,
    /// 策展 tag
    pub tags: Vec<String>,
}

const CATALOG_JSON: &str = include_str!("../../assets/sfx_catalog.json");

fn catalog() -> &'static Vec<SfxEntry> {
    static CATALOG: OnceLock<Vec<SfxEntry>> = OnceLock::new();
    CATALOG.get_or_init(|| serde_json::from_str(CATALOG_JSON).unwrap_or_default())
}

pub fn all() -> &'static Vec<SfxEntry> {
    catalog()
}

/// 小森平音效源 URL 规则: https://taira-komori.net/sound/{page}/{variant}
pub fn variant_url(entry: &SfxEntry, variant: &str) -> String {
    format!("https://taira-komori.net/sound/{}/{}", entry.page, variant)
}

/// 按 id（组 id）查找音效条目
pub fn lookup(id: &str) -> Option<&'static SfxEntry> {
    catalog().iter().find(|e| e.id == id)
}

/// 按变体文件名反查条目（resolve 后事件 name 存的是变体文件名）
pub fn lookup_variant(file_stem: &str) -> Option<&'static SfxEntry> {
    catalog().iter().find(|e| e.variants.iter().any(|v| {
        v.strip_suffix(".mp3").map(|s| s == file_stem).unwrap_or(false)
    }))
}

/// 按中文 label 反查条目（LLM 只输入中文时用；重复 label 取第一个）
pub fn lookup_label(label: &str) -> Option<&'static SfxEntry> {
    catalog().iter().find(|e| e.label == label)
}

/// 判断音效是否在屏蔽列表中
/// 条目语义: 精确 id (如 "fear-1") 屏蔽单个音效; "tag:xxx" (如 "tag:surprise") 屏蔽整个标签族
pub fn is_blacklisted(id: &str, blacklist: &[String]) -> bool {
    for entry in blacklist {
        if let Some(tag) = entry.strip_prefix("tag:") {
            if let Some(e) = lookup(id) {
                if e.tags.iter().any(|t| t == tag) {
                    return true;
                }
            }
        } else if entry == id {
            return true;
        }
    }
    false
}

/// 按 tags 查找音效（任意 tag 匹配）
pub fn search_by_tags(target_tags: &[String]) -> Vec<&'static SfxEntry> {
    catalog().iter()
        .filter(|e| e.tags.iter().any(|t| target_tags.contains(t)))
        .collect()
}

/// 取指定 tag 下的候选条目（配额取样 + 未使用优先，供 LLM 候选注入）
/// 名额按选中 tag 平均分配（limit ÷ tag 数），避免大 tag 垄断；每 tag 至少 3 个。
pub fn candidates_for_tags(target_tags: &[String], used: &std::collections::HashSet<String>, limit: usize, blacklist: &[String]) -> Vec<&'static SfxEntry> {
    if target_tags.is_empty() {
        return Vec::new();
    }
    let per = (limit / target_tags.len()).max(3);
    let mut out: Vec<&'static SfxEntry> = Vec::new();
    for t in target_tags {
        let pool: Vec<&'static SfxEntry> = catalog().iter()
            .filter(|e| e.tags.iter().any(|tag| tag == t))
            .filter(|e| !is_blacklisted(&e.id, blacklist))
            .collect();
        // 已用过的音效排除（避免整片重复），候选不足时回退填充
        let mut picked: Vec<&'static SfxEntry> = pool.iter()
            .filter(|e| !used.contains(&e.id))
            .copied()
            .take(per)
            .collect();
        if picked.len() < per {
            picked.extend(pool.iter()
                .filter(|e| used.contains(&e.id))
                .copied()
                .take(per - picked.len()));
        }
        out.extend(picked);
    }
    out.truncate(limit);
    out
}

/// 大分类导航顺序（决定 Step1 prompt 中的展示顺序）
pub const CATEGORY_ORDER: &[&str] = &[
    "① 主播情绪反应", "② 游戏战斗", "③ 演出节目", "④ 生活日常",
    "⑤ 饮食水声", "⑥ 环境氛围", "⑦ 交通出行",
];

/// 生成 Step1 的大分类 → 细分 tag(数量) 导航文本（从 catalog 聚合）
pub fn categories_text() -> String {
    use std::collections::HashMap;
    // cat → (tag → count)
    let mut map: HashMap<&str, HashMap<&str, usize>> = HashMap::new();
    for e in catalog() {
        let m = map.entry(e.cat.as_str()).or_default();
        for t in &e.tags {
            *m.entry(t.as_str()).or_insert(0) += 1;
        }
    }
    let mut text = String::from("## 大分类与细分 tag

");
    for cat in CATEGORY_ORDER {
        let Some(tags) = map.get(cat) else { continue };
        let mut items: Vec<(&str, usize)> = tags.iter().map(|(t, n)| (*t, *n)).collect();
        items.sort_by(|a, b| b.1.cmp(&a.1));
        let joined: Vec<String> = items.iter().map(|(t, n)| format!("{}({})", t, n)).collect();
        text.push_str(&format!("{}: {}
", cat, joined.join(" ")));
    }
    text
}

/// 生成 LLM prompt 可用的候选音效清单文本（按 tag 分组, 附 label 与变体数）
/// 例: "爆炸: atomic_bomb(核爆発), destruction(破壊１,3变体)"
pub fn candidates_text(target_tags: &[String], used: &std::collections::HashSet<String>, limit: usize, blacklist: &[String]) -> String {
    let cands = candidates_for_tags(target_tags, used, limit, blacklist);
    if cands.is_empty() {
        return String::new();
    }
    let mut text = format!("## 可用音效候选（共 {} 个，只能从以下选择）

", cands.len());
    for t in target_tags {
        let group: Vec<&&SfxEntry> = cands.iter().filter(|e| e.tags.iter().any(|tag| tag == t)).collect();
        if group.is_empty() {
            continue;
        }
        let items: Vec<String> = group.iter().map(|e| {
            let multi = e.variants.len() >= 2;
            let v = if multi { format!("({}变体)", e.variants.len()) } else { String::new() };
            format!("{}{}", e.label, v)
        }).collect();
        text.push_str(&format!("{}: {}
", t, items.join(" ")));
    }
    text.push_str("
用中文名称引用，如 \"sfx\": \"核爆炸\" 或 \"name\": \"重击1\"；也可以写 tag:xxx 由系统选择。
");
    text
}

/// 模糊匹配: query 按非字母数字分词, 与 id/label/tags 拼接文本做包含计分
/// id 分词精确 +5, tag 精确 +5, 任意拼接文本包含子串 +2, id/tag 完全相等额外 +5;
/// 最高分 > 0 返回
pub fn fuzzy_search(query: &str) -> Option<&'static SfxEntry> {
    fuzzy_search_excluding(query, &[])
}

/// 模糊匹配 + 排除屏蔽条目 (黑名单)
/// 分词时忽略短词 (<3 字符) 与纯数字 token, 避免 "at"/"1" 等噪音词污染匹配
pub fn fuzzy_search_excluding(query: &str, blacklist: &[String]) -> Option<&'static SfxEntry> {
    let q = query.to_lowercase();
    let q_tokens: Vec<&str> = q
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| t.len() >= 3 && !t.chars().all(|c| c.is_ascii_digit()))
        .collect();
    if q_tokens.is_empty() {
        return None;
    }

    let mut best: Option<(&'static SfxEntry, i32)> = None;
    for entry in catalog() {
        if is_blacklisted(&entry.id, blacklist) {
            continue;
        }
        let mut score = 0i32;
        let id_lower = entry.id.to_lowercase();
        let haystack = format!("{} {} {}", id_lower, entry.label, entry.tags.join(" ")).to_lowercase();
        let id_tokens: Vec<&str> = id_lower.split('-').collect();
        let tags_lower: Vec<String> = entry.tags.iter().map(|t| t.to_lowercase()).collect();

        for t in &q_tokens {
            if id_tokens.contains(t) {
                score += 5;
            }
            if tags_lower.iter().any(|tag| tag == t) {
                score += 5;
            }
            if haystack.contains(t) {
                score += 2;
            }
        }
        if id_lower == q || tags_lower.contains(&q) {
            score += 5;
        }
        if score > 0 && best.is_none_or(|(_, s)| score > s) {
            best = Some((entry, score));
        }
    }
    best.map(|(e, _)| e)
}

/// 获取所有去重 tag，按出现次数降序
pub fn all_tags() -> Vec<(&'static str, usize)> {
    let mut counts: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
    for e in catalog() {
        for t in &e.tags {
            *counts.entry(t.as_str()).or_insert(0) += 1;
        }
    }
    let mut result: Vec<_> = counts.into_iter().collect();
    result.sort_by(|a, b| b.1.cmp(&a.1));
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fuzzy_matches_llm_hallucinated_names() {
        // LLM 编造的自由描述词 → 应匹配到 catalog 中相近的音效
        assert!(fuzzy_search("night_ambient").is_some());   // night_urban_road
        assert!(fuzzy_search("iron_door").is_some());       // knock_on_mtl_door
        assert!(fuzzy_search("crash").is_some());
        assert!(fuzzy_search("explosion").is_some());
    }

    #[test]
    fn fuzzy_prefers_exact_id_token_over_substring() {
        // "explosion" 精确命中优先于 "game_explosion" 之类的子串包含
        let e = fuzzy_search("explosion").unwrap();
        assert_eq!(e.id, "explosion", "精确 token 应优先, got {}", e.id);
    }

    #[test]
    fn fuzzy_empty_or_garbage_returns_none() {
        assert!(fuzzy_search("").is_none());
        assert!(fuzzy_search("zzzz_qwerty_xyz_987").is_none());
    }

}
