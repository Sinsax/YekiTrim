//! polish::assets::sfx_catalog：内建音效目录（编译期嵌入 `sfx_catalog.json`）与查询 / 屏蔽 / 候选与模糊搜索。
//!
//! 入口：`all`、`lookup`、`lookup_variant`、`lookup_label`、`is_blacklisted`、`search_by_tags`、`candidates_for_tags`、`candidates_text`、`fuzzy_search`、`categories_text`、`all_tags`、`pack_of`、`UNCATEGORIZED_PACK`、`CATEGORY_ORDER`、`SfxEntry`。
//! 边界：屏蔽语义 = 精确 id 屏蔽单个、`tag:xxx` 屏蔽整个标签族；项目级与全局黑名单由 `process::load_sfx_blacklist` 求并集 —— 见 `crates/yekitrim-polish/src/assets/AGENTS.md`

/// 音效数据库 — 从 assets/sfx_catalog.json 编译时嵌入（小森平/効果音ラボ全量 + 变体归组）
use serde::{Deserialize, Serialize};
use std::sync::OnceLock;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SfxEntry {
    /// 组 id（变体归组后的语义名，LLM 看到的就是它）
    pub id: String,
    /// 大分类（如 "① 主播情绪反应"）—— 素材库左栏那 7 个
    pub cat: String,
    /// 小分类（如 "敲·门·门口"）—— 素材库音效页的**「包」**，145 个。
    ///
    /// ⚠️ 与贴纸的「包」同粒度：一个 subcat 平均 8 组，点开才列这一包里的音效。
    /// 1213 组平铺成网格是没法用的（要翻几十屏），所以这一页按它分包。
    ///
    /// ⚠️ **缺省不是错误**：10 条噪音类（`base_noise` / `radio_noise` …）在源站
    /// 就没有小分类 ⇒ `None`。消费侧（[`pack_of`]）把它们归进「其他」包，
    /// **不要**在这里编一个空串或占位名 —— 那会让"真没有"与"值就是空"分不开。
    #[serde(default)]
    pub subcat: Option<String>,
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

/// 按 id（组 id）查找音效条目
pub fn lookup(id: &str) -> Option<&'static SfxEntry> {
    catalog().iter().find(|e| e.id == id)
}

/// 小森平音效源 URL 规则: https://taira-komori.net/sound/{page}/{variant}
pub fn variant_url(entry: &SfxEntry, variant: &str) -> String {
    format!("https://taira-komori.net/sound/{}/{}", entry.page, variant)
}

/// **变体文件的本地键**（不带扩展名）—— 入库事件、落盘文件名、预览 URL 全用它。
///
/// # ⚠️ 为什么不直接用变体名
///
/// catalog 里 **56 个变体名自带子目录**（法/美街道那几页是 `Crowds/xxx.mp3`、
/// `City_french/xxx.mp3`，共 20 组）。直接拿变体名当文件名有两个后果，且**都不报错**：
/// - 下载：`sfx/Crowds/xxx.mp3` —— 目录不存在 ⇒ 写盘失败或落进子目录；
/// - 出片：`project.join("sfx").join("Crowds/xxx.mp3")` 在 Windows 上也可能被当成子路径，
///   而 `collect_sound_events` 拼的是 `format!("sfx/{}.mp3", name)`，和文件名对不上 ⇒
///   `pipeline.rs` 只 warn 一句「音效文件缺失」，成片少一声。
///
/// 所以**唯一的扁平化规则在这里**：去掉扩展名、`/` 与 `\` 换成 `-`。
/// 结果必须能被 `is_safe_component` 接受（无分隔符、无 `..`）。
///
/// ⚠️ 已解析事件里存的就是这个键，**改名等于让存量工程的音效全部对不上**。
pub fn variant_stem(variant: &str) -> String {
    variant
        .strip_suffix(".mp3")
        .unwrap_or(variant)
        .replace(['/', '\\'], "-")
}

/// 按变体文件名反查条目（resolve 后事件 name 存的是 [`variant_stem`]）
pub fn lookup_variant(file_stem: &str) -> Option<&'static SfxEntry> {
    catalog().iter().find(|e| e.variants.iter().any(|v| variant_stem(v.as_str()) == file_stem))
}

/// 变体的**原始清单条目**（试听用）：`(原始变体名, 源 URL)`。
///
/// 与 [`variant_stem`] 成对：库里/事件里存的是扁平键，而源站**只认原始变体名**
/// （`Crowds/xxx.mp3` 那一层子目录是站点结构，不能拍平）。
/// 所以「试听」这条路上，**键 → 原始名 → URL** 必须一次算清，别让调用方自己拼。
pub fn variant_source(entry_id: &str, stem: &str) -> Option<(&'static str, String)> {
    let e = lookup(entry_id)?;
    let raw = e.variants.iter().find(|v| variant_stem(v.as_str()) == stem)?;
    Some((raw.as_str(), variant_url(e, raw)))
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

/// 没有 `subcat` 的条目归进哪一个细分组。
///
/// ⚠️ 这条兜底**只此一份**：素材库音效页与任何按细分聚合的地方都读 [`group_of`]，
/// 别在前端再写一遍 `subcat || '其他'` —— 两处各写一次就必然有一天漂成两个组名。
pub const UNCATEGORIZED_GROUP: &str = "其他";

/// 内建预设音效库的名字。
///
/// # 为什么包 = **一个库**，不是 7 个大类
///
/// 素材库左侧是**「音效库」的位置**（与贴纸页左侧放"贴纸包"同理）：现在只有一个内建库
/// （小森平 1213 组），以后用户导入的包会一张张往外长。把 7 个大类当包铺上去，
/// 左侧立刻被占满 —— 新库来了没地方放（用户原话：「你左边直接沾满时何意味」）。
///
/// 大类（`cat`）与 `subcat` 都是**右侧的多级细分**：
///   一级 = 大类（① 主播情绪反应 … 7 个）
///   二级 = 小类（`subcat`，145 个）
///   三级 = 音效组（1213）
///   四级 = 变体（2118，可试听）
pub const PRESET_PACK_NAME: &str = "小森平";

/// 条目所属的**音效库**。
///
/// 预设条目一律归 [`PRESET_PACK_NAME`]；用户导入的包由 `sfx_packs` 那条路单独列，
/// 不经过这里。补 `&'static str` 而不是 `&str`：它现在是个常量，调用方少一层借用。
pub fn pack_of(_entry: &SfxEntry) -> &'static str {
    PRESET_PACK_NAME
}

/// 条目的一级细分（= 大分类 `cat` 去掉序号）。
///
/// ⚠️ 与 [`pack_of`] 的区别：`pack_of` 是"哪个库"，这里是"库里哪一个类"。
/// 两者都去序号用的是**同一条规则**（`category_bare_name`），别各写一份。
pub fn group_of(entry: &SfxEntry) -> &str {
    crate::assets::sfx_packs::category_bare_name(&entry.cat)
}

/// 条目的二级细分（= `subcat`，缺省落到 [`UNCATEGORIZED_GROUP`]）。
///
/// 对应贴纸页里"包内按 tag 筛图"那一排标签。
pub fn subgroup_of(entry: &SfxEntry) -> &str {
    entry
        .subcat
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(UNCATEGORIZED_GROUP)
}

/// 生成 Step1 的大分类 → 细分 tag(数量) 导航文本（从 catalog 聚合）
pub fn categories_text() -> String {    use std::collections::HashMap;
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

    // ---- 变体键（`variant_stem`）：目录形态的变体名不许带出路径 ----

    #[test]
    fn variant_stem_flattens_subdirs_and_stays_safe() {
        use crate::assets::sticker_index::is_safe_component;
        assert_eq!(variant_stem("poka.mp3"), "poka");
        // catalog 里真实存在的目录形态（french01 / us01 那 20 组）
        assert_eq!(
            variant_stem("Crowds/Amb_Terrasse_Resto.mp3"),
            "Crowds-Amb_Terrasse_Resto"
        );
        assert_eq!(variant_stem("City_french/Voix_Ext_Rue_Pigalle1.mp3"), "City_french-Voix_Ext_Rue_Pigalle1");
        // 反斜杠也拍平（Windows 路径同样会被当成子目录）
        assert_eq!(variant_stem("a\\b.mp3"), "a-b");
        assert!(is_safe_component(&variant_stem("Crowds/Amb_Terrasse_Resto.mp3")));
    }

    /// **全量往返守卫**：2118 个变体，每一个的键都必须
    /// ① 能过 `is_safe_component`（否则下载/渲染会拼出库外路径）；
    /// ② 能被 `lookup_variant` 反查回**它自己那一条**（否则事件里的 name 查不到源 URL）。
    ///
    /// 这条是"目录形态变体"那次缺陷的常驻判据：当时 56 个变体带着 `/`，
    /// 下载落进子目录、出片侧找不到文件，全链路只 warn 一句。
    #[test]
    fn every_variant_stem_is_safe_and_round_trips() {
        use crate::assets::sticker_index::is_safe_component;
        let mut checked = 0;
        for e in catalog() {
            for v in &e.variants {
                let stem = variant_stem(v.as_str());
                assert!(
                    is_safe_component(&stem),
                    "变体 `{v}` 的键 `{stem}` 不是安全单文件名（下载会拼出库外/子目录路径）"
                );
                let back = lookup_variant(&stem).unwrap_or_else(|| {
                    panic!("变体键 `{stem}`（来自 `{v}`）反查不到条目")
                });
                assert_eq!(back.id, e.id, "变体键 `{stem}` 反查到了另一组：{}", back.id);
                checked += 1;
            }
        }
        assert_eq!(checked, 2118, "catalog 变体数变了（现在是 {checked}）—— 同步更新这条断言");
    }

    /// 带 `tag:` 前缀的屏蔽语义不能误伤 id：`tag:xxx` 只屏蔽该族、不屏蔽别组。
    #[test]
    fn blacklist_tag_family_does_not_leak_into_other_ids() {
        let family = vec!["tag:恐怖氛围".to_string()];
        assert!(is_blacklisted("white_noise", &family));
        assert!(!is_blacklisted("explosion", &family));
    }

    /// **`subcat` 必须真的从 JSON 读进来**（素材库音效页按它切"包内细分组"）。
    ///
    /// 判据不钉"某些 id 属于某个组"（策展会变），只钉三件结构事实：
    /// ① 组数落在一个合理区间（≈145：多了说明 JSON 里的字段被改了名、少了说明丢了值）；
    /// ② 每条要么有 `subcat`、要么明确落到 [`UNCATEGORIZED_GROUP`]（**没有第三条路**）；
    /// ③ 每个组名都非空、无首尾空白（它会被当 key 与显示名两用）。
    #[test]
    fn every_entry_belongs_to_exactly_one_group() {
        let mut groups = std::collections::HashSet::new();
        let mut uncategorized = 0;
        for e in catalog() {
            let g = subgroup_of(e);
            assert!(!g.is_empty(), "条目 `{}` 的细分组名是空的", e.id);
            assert_eq!(g, g.trim(), "条目 `{}` 的细分组名有首尾空白：`{g}`", e.id);
            if e.subcat.as_deref().map(str::trim).unwrap_or("").is_empty() {
                uncategorized += 1;
                assert_eq!(g, UNCATEGORIZED_GROUP, "缺 subcat 却没落到兜底组（`{}`）", e.id);
            }
            groups.insert(g.to_string());
        }
        // 1213 组 / 145 个细分（+1 兜底）。区间放宽到 ±20% —— 策展改动能过，字段改名过不了。
        assert!(
            (120..=180).contains(&groups.len()),
            "细分组数 {} 不在 120~180 之间 —— `subcat` 是不是没读进来？",
            groups.len()
        );
        // 缺 subcat 的那 10 条噪音类是**已知**的（源站就没有小分类），不是"越来越多"。
        assert!(
            uncategorized <= 20,
            "缺 subcat 的条目涨到 {uncategorized} 条 —— 源数据变了，先核对再放宽这条"
        );
    }

    /// **预设只有一个库**（[`PRESET_PACK_NAME`]），大类是**库里的一级细分**。
    ///
    /// 素材库左侧是"音效库"的位置：现在只有小森平，以后用户导入的包会一张张往外长。
    /// ⚠️ 别把大类当包铺到左侧 —— 7 张卡片会把左列占满、新库没地方放（用户明确否过）。
    #[test]
    fn preset_is_one_pack_and_categories_are_subdivisions() {
        let packs: std::collections::HashSet<&str> = catalog().iter().map(pack_of).collect();
        assert_eq!(packs.len(), 1, "预设只应有一个库，实际: {packs:?}");
        assert_eq!(packs.into_iter().next(), Some(PRESET_PACK_NAME));

        // 一级细分 = 7 个大类（去序号）
        let mut cats: Vec<&str> = catalog().iter().map(group_of).collect();
        cats.sort_unstable();
        cats.dedup();
        assert_eq!(cats.len(), CATEGORY_ORDER.len(), "一级细分数应等于大类数：{cats:?}");
        for cat in CATEGORY_ORDER {
            let bare = crate::assets::sfx_packs::category_bare_name(cat);
            assert!(cats.contains(&bare), "大类 `{cat}`（={bare}）在 catalog 里一个条目都没有");
            let n = catalog().iter().filter(|e| group_of(e) == bare).count();
            assert!(n > 50, "大类 `{cat}` 只有 {n} 组 —— 是不是 cat 字段被改了？");
        }
    }
}
