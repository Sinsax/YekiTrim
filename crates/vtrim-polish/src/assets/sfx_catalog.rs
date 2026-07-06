/// 音效数据库 — 从 assets/sfx_catalog.json 编译时嵌入
use serde::{Deserialize, Serialize};
use std::sync::OnceLock;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SfxEntry {
    pub id: String,
    pub url: String,
    pub label: String,
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

/// 按 id 查找音效条目
pub fn lookup(id: &str) -> Option<&'static SfxEntry> {
    catalog().iter().find(|e| e.id == id)
}

/// 按 tags 查找音效（任意 tag 匹配）
pub fn search_by_tags(target_tags: &[String]) -> Vec<&'static SfxEntry> {
    catalog().iter()
        .filter(|e| e.tags.iter().any(|t| target_tags.contains(t)))
        .collect()
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

/// 生成 LLM prompt 可用的音效场景标签列表
pub fn to_prompt_text() -> String {
    let tags = all_tags();
    let mut text = String::from("## 可用音效场景标签\n\n选择最匹配视频情绪/场景的标签:\n\n");
    for (tag, count) in &tags {
        text.push_str(&format!("  {} ({}音效)\n", tag, count));
    }
    text.push_str("\n在事件的 sfx 字段中用 \"tag:场景名\" 引用，如 \"sfx\": \"tag:格闘\" 或 \"sfx\": \"tag:comedy\"\n");
    text.push_str("系统会自动匹配合适的音效文件。\n");
    text
}
