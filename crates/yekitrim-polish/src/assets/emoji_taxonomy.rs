//! polish::assets::emoji_taxonomy：emoji 多维分类的**唯一词表真源**。
//!
//! 入口：`taxonomy()` / `find_dim(key)` / `is_valid_value(dim, value)` /
//! `prompt_vocab_text()` / `parse_query(text)` / `guess_dims(word)`。
//!
//! # 为什么要有它（2026-10-17）
//!
//! 旧数据是**一层扁平的 85 个 tag**，实测平均每条 emoji 只挂 **1.15** 个、其中 **32 个
//! 只覆盖 1 个表情**（`alien` / `bow` / `cheer` / `robot`…）—— 那是"某个表情的别名"，
//! 不是分类：既筛不动（每组只有 1 条），也选不准。
//!
//! 而**选取**那一侧的缺陷在整条链上：LLM 只被告知"写英文，如 laugh/cry/shock/game"
//! （`llm/segment.rs::sticker_lang_desc`）——**它不知道有哪些词可用**；`process` 拿到
//! 这个词后走 `EmojiCatalog::search` 的**子串 ⊇ 模糊**匹配（tag 含这个词的**所有**表情，
//! 按**目录顺序**取前 8，无评分）；而本地贴纸包那条路有"候选 tag 列表 + tag 内 LLM 精挑"
//! 两步，**emoji 一步都没有**。
//!
//! 所以这里定义**六维、每维闭集**：LLM 只能从词表里选（闭合性由 `is_valid_value` 保证），
//! 查询按维度打分（`emoji_search::search_dims`），面板按**主维度**分区。
//!
//! ⚠️ **词表是代码里的真源**（`assets/emoji_taxonomy.json`，`include_str!` 进来）：
//! prompt 文本、构建工具的校验、查询打分全都读它。**别在 prompt 字符串里另抄一份词表** ——
//! 抄一份就必然漂，而漂的表现是"LLM 写了一个查不到的词"（静默选错表情）。

use std::sync::OnceLock;

use serde::Deserialize;

/// `include_str!` 进来的词表（与 `emoji_catalog.json` 同目录、同为编译期输入）。
const TAXONOMY_JSON: &str = include_str!("../../assets/emoji_taxonomy.json");

/// 一个维度。`values` 是该维的**闭集**（LLM 只能从中选）。
#[derive(Debug, Clone, Deserialize)]
pub struct DimSpec {
    /// 维度键（小写 ASCII，同时是 JSON 字段名与查询里的前缀，如 `emotion`）。
    pub key: String,
    /// 中文标签（面板/日志给人看的那一份）。
    pub label: String,
    /// 给 LLM 的一句话说明（`prompt_vocab_text` 用它）。
    #[serde(default)]
    pub desc: String,
    /// 可多选吗。⚠️ **`multi = false` 的维度就是"分区维度"** ——
    /// 面板（`grouped()`）按第一个 `multi = false` 的维度分区。
    #[serde(default)]
    pub multi: bool,
    /// 打分权重（`search_dims`）。含义：这个维度对上"有多值钱"。
    #[serde(default = "default_weight")]
    pub weight: f32,
    /// 闭集取值。
    pub values: Vec<String>,
}

fn default_weight() -> f32 {
    1.0
}

/// 整份词表。
#[derive(Debug, Clone, Deserialize)]
pub struct Taxonomy {
    #[serde(default)]
    pub version: u32,
    pub dims: Vec<DimSpec>,
}

static TAXONOMY: OnceLock<Taxonomy> = OnceLock::new();

/// 整份词表（首次访问解析一次）。
///
/// ⚠️ 解析失败**不 panic**：回一份空词表 —— 上层（prompt / 查询）会退化成旧行为，
/// 而不是让整个 process 炸掉。词表坏了这件事由 `taxonomy_is_sane` 那条单测当场抓住。
pub fn taxonomy() -> &'static Taxonomy {
    TAXONOMY.get_or_init(|| {
        serde_json::from_str(TAXONOMY_JSON).unwrap_or_else(|e| {
            tracing::error!("emoji 词表解析失败（退化成旧行为）: {e}");
            Taxonomy { version: 0, dims: Vec::new() }
        })
    })
}

/// 按维度键找维度。
pub fn find_dim(key: &str) -> Option<&'static DimSpec> {
    let k = key.trim().to_ascii_lowercase();
    taxonomy().dims.iter().find(|d| d.key == k)
}

/// 面板的**分区维度**：第一个 `multi = false` 的维度（现在 = `emotion`）。
///
/// ⚠️ 面板按它分区，而不是按"所有 tag 的并集" —— 并集是 86 个值、又变回"几十个分类"，
/// 正是用户否掉的那种形状（2026-10-17：「当前 emoji 包的类型有几十个，这根本不对」）。
pub fn primary_dim() -> Option<&'static DimSpec> {
    taxonomy().dims.iter().find(|d| !d.multi)
}

/// `value` 是 `dim` 的合法取值吗（大小写不敏感）。
pub fn is_valid_value(dim: &str, value: &str) -> bool {
    let Some(d) = find_dim(dim) else { return false };
    let v = value.trim().to_ascii_lowercase();
    d.values.iter().any(|x| *x == v)
}

/// 拼给 LLM 的词表文本（**唯一一份**，别在 prompt 里另抄）。
pub fn prompt_vocab_text() -> String {
    let mut out = String::new();
    for d in &taxonomy().dims {
        out.push_str(&format!(
            "- {} ({}，{}): {}\n",
            d.key,
            d.label,
            if d.multi { "可多选，没有就留空 []" } else { "单选，必须给一个" },
            d.values.join(" ")
        ));
        if !d.desc.is_empty() {
            out.push_str(&format!("  {}\n", d.desc));
        }
    }
    out
}

/// 解析一条紧凑查询：`emotion:joy,intensity:3,usage:punchline|reaction`
/// → `[("emotion","joy"), ("intensity","3"), ("usage","punchline"), ("usage","reaction")]`。
///
/// ⚠️ **认不出的一律丢掉**（不猜）：`dim` 不在词表里、或 value 不是该维合法取值 → 丢。
/// 调用方拿它判"这条查询能不能走分维打分"，丢空了就回落到旧的自由词匹配。
pub fn parse_query(text: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for part in text.split([',', '，', ';', '；']) {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        let Some((dim, rest)) = part.split_once([':', '：']) else { continue };
        for v in rest.split('|') {
            let v = v.trim();
            if is_valid_value(dim, v) {
                out.push((dim.trim().to_ascii_lowercase(), v.to_ascii_lowercase()));
            }
        }
    }
    out
}

/// 自由词 → 词表里能对上的取值（旧工程 / LLM 不听话时用）。
///
/// 先精确、再子串（`game` 能对上 `game`；`gam` 也能对上它）。
/// ⚠️ 子串只单向（输入 ⊆ 词表值），**不做反向** —— 那会让 `happy` 命中 `joy` 这种
/// 没有依据的联想，而"静默选到无关表情"正是这次要修的东西。
pub fn guess_dims(word: &str) -> Vec<(String, String)> {
    let w = word.trim().to_ascii_lowercase();
    if w.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    for d in &taxonomy().dims {
        for v in &d.values {
            if *v == w {
                out.push((d.key.clone(), v.clone()));
            }
        }
    }
    if !out.is_empty() {
        return out;
    }
    for d in &taxonomy().dims {
        for v in &d.values {
            if v.contains(&w) {
                out.push((d.key.clone(), v.clone()));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 词表本身得是**说得通**的：非空、键唯一、取值唯一且形状规范。
    ///
    /// 变异自证：把某个维度的 `values` 清空 → 当场红（prompt 会给出一个空词表、
    /// 闭合性判据也无从谈起）。
    #[test]
    fn taxonomy_is_sane() {
        let t = taxonomy();
        assert!(!t.dims.is_empty(), "词表空了（include_str! 路径或 JSON 坏了）");
        let mut keys = std::collections::HashSet::new();
        for d in &t.dims {
            assert!(keys.insert(d.key.clone()), "维度键重复: {}", d.key);
            assert!(!d.values.is_empty(), "维度 {} 没有取值", d.key);
            assert!(d.weight > 0.0, "维度 {} 权重必须为正", d.key);
            let mut seen = std::collections::HashSet::new();
            for v in &d.values {
                assert!(seen.insert(v.clone()), "{} 的取值重复: {v}", d.key);
                assert!(
                    v.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'),
                    "{} 的取值 {v:?} 不是小写 ASCII（它同时是 JSON 键与查询里的词）",
                    d.key
                );
            }
        }
        assert!(primary_dim().is_some(), "至少要有一个单选维度（面板按它分区）");
        assert!(
            t.dims.iter().filter(|d| d.multi).count() >= 2,
            "可多选的维度太少 —— 交叉分类的意义就在它们上面"
        );
    }

    /// 词表文本是 prompt 用的：每个维度、每个取值都要出现（漏了 = LLM 学不到那个词）。
    #[test]
    fn prompt_vocab_text_covers_every_value() {
        let text = prompt_vocab_text();
        for d in &taxonomy().dims {
            assert!(text.contains(&d.key), "词表文本里没有维度 {}", d.key);
            for v in &d.values {
                assert!(
                    text.contains(v.as_str()),
                    "词表文本里没有 {}.{v}（LLM 就永远写不出它）",
                    d.key
                );
            }
        }
    }

    /// 闭合性判据的**正反面**：合法取值必须收，越界/乱写必须丢。
    #[test]
    fn parse_query_only_accepts_vocabulary() {
        let q = parse_query("emotion:joy,intensity:3,usage:punchline|reaction,tone:positive");
        assert_eq!(
            q,
            vec![
                ("emotion".to_string(), "joy".to_string()),
                ("intensity".to_string(), "3".to_string()),
                ("usage".to_string(), "punchline".to_string()),
                ("usage".to_string(), "reaction".to_string()),
                ("tone".to_string(), "positive".to_string()),
            ]
        );
        // 大小写 / 空格 / 中文逗号都收
        assert_eq!(parse_query(" Emotion : JOY ， tone : Negative ").len(), 2);
        // 词表外的维度、词表外的取值、没有冒号的自由词 —— 一律丢（不猜）
        assert!(parse_query("mood:happy").is_empty(), "mood 不是维度");
        assert!(parse_query("emotion:ecstatic").is_empty(), "ecstatic 不在词表里");
        assert!(parse_query("laugh").is_empty(), "自由词没有维度前缀");
    }

    /// 自由词回落：精确优先、子串兜底、认不出就空。
    #[test]
    fn guess_dims_falls_back_to_substring() {
        assert_eq!(guess_dims("joy"), vec![("emotion".to_string(), "joy".to_string())]);
        assert!(guess_dims("game").iter().any(|(d, v)| d == "object" && v == "game"));
        assert!(guess_dims("gam").iter().any(|(d, v)| d == "object" && v == "game"));
        assert!(guess_dims("zzzzz").is_empty());
        assert!(guess_dims("").is_empty());
    }
}
