//! 内置**模型能力表**：某个 (base_url, model) 支不支持推理、档位怎么拼、温度能不能带。
//!
//! # 数据从哪来
//!
//! `data/llm-models.json` 由 `scripts/gen-llm-models.mjs` 从开源库 **`@earendil-works/pi-ai`（MIT）**
//! 的 `dist/providers/data/*.json` 抽出来，只保留与短视频切片有关的那几家。它的字段形状本来就和
//! 我们要的同构（`thinkingLevelMap` / `compat.thinkingFormat` / `compat.maxTokensField`），
//! 所以这一层是「翻译」，不是「设计」。
//!
//! # 三条纪律
//!
//! 1. **表是数据，不是代码**：改它不碰 Rust；过期就用脚本重生成（`--check` 会对不上而报红）。
//! 2. **表没命中就保守**：不发推理参数、温度照发、`max_tokens` —— **最坏是少发，绝不瞎发**。
//! 3. **JSON 解析失败也不许崩**：退回空目录（等于全部走保守路径）并 warn —— 一次手改坏数据文件
//!    不该让整个程序起不来。

use crate::params::{MaxOutputWire, ModelCaps, ReasoningLevel, Support, ThinkingWire};
use serde::Deserialize;
use std::collections::BTreeMap;
use std::sync::OnceLock;

const DATA: &str = include_str!("../data/llm-models.json");

#[derive(Debug, Deserialize, Default)]
struct CatalogFile {
    #[serde(default)]
    generated_from: Option<GeneratedFrom>,
    #[serde(default)]
    providers: Vec<ProviderEntry>,
}

#[derive(Debug, Deserialize, Clone)]
struct GeneratedFrom {
    package: String,
    version: String,
}

/// 一个 wire 字段可能写成字符串、也可能写成数组。
///
/// ⚠️ 为什么会有数组：能力表是从 pi-ai 的 `compat` 折出来的，而那份 `compat` 是**模型级**字段 ——
/// 同一家 provider 里可能有的模型用 A 方言、有的用 B（实测 moonshotai 的 kimi-k3 与其余 9 个 Kimi）。
/// 数组只在长度 1 时可用（那代表全provider一致）；长度 > 1 必须由**模型级**字段来消歧。
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
enum WireList {
    One(String),
    Many(Vec<String>),
}

impl WireList {
    fn single(&self) -> Option<&str> {
        match self {
            Self::One(s) => Some(s.as_str()),
            Self::Many(v) if v.len() == 1 => v.first().map(String::as_str),
            // 长度 > 1 又没给模型级覆盖 ⇒ 不知道这支该用哪个方言 ⇒ 交给调用方降级成"不发"。
            _ => None,
        }
    }
}

#[derive(Debug, Deserialize)]
struct ProviderEntry {
    id: String,
    #[serde(default)]
    hosts: Vec<String>,
    #[serde(default)]
    base_url: Option<String>,
    /// 协议族（`openai-completions` / `anthropic-messages` / `openai-responses` …）。
    /// 只有 OpenAI 兼容那一族，推理参数的 wire 语义才是我们这套；其余按"不发"处理。
    #[serde(default)]
    api: Option<String>,
    #[serde(default)]
    thinking_wire: Option<WireList>,
    #[serde(default)]
    max_output_wire: Option<WireList>,
    #[serde(default)]
    temperature: String,
    #[serde(default)]
    models: Vec<ModelEntry>,
}

#[derive(Debug, Deserialize)]
struct ModelEntry {
    id: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    context_window: Option<u32>,
    #[serde(default)]
    max_output: Option<u32>,
    #[serde(default)]
    thinking_levels: Option<BTreeMap<String, Option<String>>>,
    /// 模型级方言（优先于 provider 级数组）。
    #[serde(default)]
    thinking_wire: Option<String>,
    #[serde(default)]
    max_output_wire: Option<String>,
}

fn catalog() -> &'static CatalogFile {
    static CACHE: OnceLock<CatalogFile> = OnceLock::new();
    CACHE.get_or_init(|| match serde_json::from_str::<CatalogFile>(DATA) {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!("内置模型能力表解析失败（{e}）—— 全部模型按保守默认处理：只发 max_tokens/temperature");
            CatalogFile::default()
        }
    })
}

/// 从 `base_url` 取 host（去 scheme、去端口、去路径、小写）。
pub fn host_of(base_url: &str) -> Option<String> {
    let rest = base_url.split("://").nth(1).unwrap_or(base_url);
    let host = rest.split(['/', '?', '#']).next()?.split('@').next_back()?;
    let host = host.split(':').next()?.trim().to_ascii_lowercase();
    (!host.is_empty()).then_some(host)
}

fn find_provider(base_url: &str) -> Option<&'static ProviderEntry> {
    let host = host_of(base_url)?;
    catalog().providers.iter().find(|p| {
        p.hosts.iter().any(|h| h.eq_ignore_ascii_case(&host))
    })
}

fn find_model(base_url: &str, model: &str) -> Option<(&'static ProviderEntry, &'static ModelEntry)> {
    if model.trim().is_empty() {
        return None;
    }
    let cat = catalog();
    let prov = find_provider(base_url);
    // 先在**这家**里找（同名模型在不同家的能力可能不同），再全局找一次。
    if let Some(p) = prov {
        if let Some(m) = p.models.iter().find(|m| m.id.eq_ignore_ascii_case(model)) {
            return Some((p, m));
        }
    }
    cat.providers.iter().find_map(|p| {
        p.models
            .iter()
            .find(|m| m.id.eq_ignore_ascii_case(model))
            .map(|m| (p, m))
    })
}

fn parse_support(s: &str) -> Support {
    match s.trim().to_ascii_lowercase().as_str() {
        "unsupported" | "no" | "false" => Support::Unsupported,
        _ => Support::Supported,
    }
}

/// 解析一个 (base_url, model) 的能力。**没命中就返回保守默认**（不是 None —— 调用方不该分叉）。
pub fn caps_for(base_url: &str, model: &str) -> ModelCaps {
    let mut caps = ModelCaps::default();
    let Some((prov, entry)) = find_model(base_url, model) else {
        return caps;
    };
    // 协议族不对（anthropic-messages / openai-responses …）：我们只会说 chat/completions，
    // 那些 wire 语义不成立 ⇒ 推理参数一律不发（保守）。
    let openai_compat = prov
        .api
        .as_deref()
        .is_none_or(|a| a.eq_ignore_ascii_case("openai-completions"));
    // 模型级优先，其次 provider 级（只在长度 1 时有意义），最后降级成"不发"。
    let thinking = entry
        .thinking_wire
        .as_deref()
        .or_else(|| prov.thinking_wire.as_ref().and_then(WireList::single));
    let max_output = entry
        .max_output_wire
        .as_deref()
        .or_else(|| prov.max_output_wire.as_ref().and_then(WireList::single));
    caps.thinking_wire = if openai_compat {
        thinking.map(ThinkingWire::parse).unwrap_or_default()
    } else {
        ThinkingWire::None
    };
    caps.max_output_wire = max_output.map(MaxOutputWire::parse).unwrap_or_default();
    caps.temperature = parse_support(&prov.temperature);
    caps.context_window = entry.context_window;
    caps.max_output_ceiling = entry.max_output;
    if let Some(levels) = &entry.thinking_levels {
        for (k, v) in levels {
            if let Some(lv) = ReasoningLevel::parse(k) {
                caps.levels.insert(lv, v.clone());
            }
        }
    }
    caps
}

/// 这个 host 在表里对应的 provider id（没有就 `None`）。
pub fn provider_id(base_url: &str) -> Option<String> {
    find_provider(base_url).map(|p| p.id.clone())
}

/// 表里给这个 host 登记的默认 base_url（用户只填了 host 时补全用）。
pub fn catalog_base_url(base_url: &str) -> Option<String> {
    find_provider(base_url).and_then(|p| p.base_url.clone())
}

/// 这个 host 在表里登记了哪些模型（给 discovery / 界面下拉用；**不联网**）。
pub fn models_for(base_url: &str) -> Vec<(String, Option<String>)> {
    let host = match host_of(base_url) {
        Some(h) => h,
        None => return Vec::new(),
    };
    catalog()
        .providers
        .iter()
        .filter(|p| p.hosts.iter().any(|h| h.eq_ignore_ascii_case(&host)))
        .flat_map(|p| p.models.iter().map(|m| (m.id.clone(), m.name.clone())))
        .collect()
}

/// 把 `(base_url, model)` 的能力渲染成**给界面用的 JSON**（设置页按能力渲染控件的唯一数据源）。
///
/// **只查表、不联网**（与 `models_for` 同一条口径）。键名就是前端契约：
/// * `hit`：表里真的认识这对 (host, model)；`false` 时其余字段是**保守默认**（界面写明"未收录"）；
/// * `levels`：推理**档位 → wire 拼写**。值是 `null` = 该档支持但这一档**不发送**；
///   **缺档 = 这个模型不提供这一档** —— 界面下拉的选项就是它的键，别自己补齐；
/// * `temperature`：`"supported"` / `"unsupported"`（后者界面要禁用温度控件并写明原因）；
/// * `max_output_ceiling` / `context_window`：**能力上限**（不是每请求默认值，默认值归配置）。
///
/// ⚠️ **不外泄 `thinking_wire` / `max_output_wire`**：那是"怎么发"，是本 crate 的内部知识，
/// 界面拿它也没用（该显示的只有档位与"能不能带温度"）。
pub fn caps_json(base_url: &str, model: &str) -> serde_json::Value {
    let caps = caps_for(base_url, model);
    let mut levels = serde_json::Map::new();
    for (level, wire) in &caps.levels {
        levels.insert(
            level.as_str().to_string(),
            match wire {
                Some(w) => serde_json::Value::String(w.clone()),
                None => serde_json::Value::Null,
            },
        );
    }
    serde_json::json!({
        "hit": find_model(base_url, model).is_some(),
        "levels": levels,
        "temperature": match caps.temperature {
            Support::Supported => "supported",
            Support::Unsupported => "unsupported",
        },
        "max_output_ceiling": caps.max_output_ceiling,
        "context_window": caps.context_window,
        "reasoning": caps.reasoning_supported(),
    })
}

/// 表里登记了几家 provider / 多少支模型（诊断用）。
pub fn stats() -> (usize, usize) {
    let cat = catalog();
    (cat.providers.len(), cat.providers.iter().map(|p| p.models.len()).sum())
}

/// 这张表是从哪个包、哪个版本抽出来的（诊断/界面显示用）。
pub fn generated_from() -> Option<(String, String)> {
    catalog()
        .generated_from
        .as_ref()
        .map(|g| (g.package.clone(), g.version.clone()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_解析去掉_scheme_端口与路径() {
        assert_eq!(host_of("https://api.deepseek.com/v1").as_deref(), Some("api.deepseek.com"));
        assert_eq!(host_of("http://127.0.0.1:8080/v1").as_deref(), Some("127.0.0.1"));
        assert_eq!(host_of("api.deepseek.com/v1").as_deref(), Some("api.deepseek.com"));
        assert_eq!(host_of(""), None);
    }

    #[test]
    fn 表没命中的模型走保守默认() {
        let caps = caps_for("https://unknown.example.com/v1", "no-such-model");
        assert!(!caps.reasoning_supported(), "不认识就不发推理参数");
        assert_eq!(caps.temperature, Support::Supported, "温度照发（最坏是对方拒）");
        assert_eq!(caps.max_output_wire, MaxOutputWire::MaxTokens);
    }

    #[test]
    fn 模型级方言优先于_provider_级并集() {
        // `api.moonshot.ai` 这家在表里同时出现两种方言（provider 级是**并集**，单看它无法消歧）。
        // 这条钉的就是「逐模型覆盖真的被读到了」——kimi-k3 与 kimi-k2-0711-preview 必须拿到不同的 wire。
        let a = caps_for("https://api.moonshot.ai/v1", "kimi-k3");
        assert_eq!(a.thinking_wire, ThinkingWire::parse("openai_effort"));
        let b = caps_for("https://api.moonshot.ai/v1", "kimi-k2-0711-preview");
        assert_eq!(b.thinking_wire, ThinkingWire::parse("thinking_type"));
        assert_ne!(a.thinking_wire, b.thinking_wire, "两家不同方言必须能分辨");
    }

    /// 界面契约：`caps_json` 给的是**档位与温度**，不是方言。
    ///
    /// 回归钉子：设置页的下拉选项数 == `levels` 的条数（P4 的几何断言），所以这里必须钉住
    /// "**缺档 = 不提供**、`null` = 支持但不发送"这两件事在 JSON 里长什么样。
    #[test]
    fn caps_json_给界面的是档位与温度() {
        let v = caps_json("https://api.deepseek.com/v1", "deepseek-v4-pro");
        assert_eq!(v["hit"], serde_json::json!(true), "{v}");
        assert_eq!(v["temperature"], "supported", "{v}");
        assert_eq!(v["max_output_ceiling"].as_u64(), Some(384_000), "{v}");
        assert_eq!(v["context_window"].as_u64(), Some(1_000_000), "{v}");
        let levels = v["levels"].as_object().expect("levels 必须是对象");
        assert_eq!(levels.len(), 4, "deepseek-v4-pro 提供 4 档：{levels:?}");
        assert!(levels["high"].is_string(), "high 有 wire 拼写：{levels:?}");
        assert!(levels["medium"].is_null(), "medium 支持但不发送（null）：{levels:?}");
        assert!(levels.get("off").is_none(), "缺档 = 不提供，界面别自己补一档：{levels:?}");
        // 方言不外泄：界面照 keys 画就行，怎么发是本 crate 的事
        assert!(v.get("thinking_wire").is_none(), "{v}");
        assert!(v.get("max_output_wire").is_none(), "{v}");

        // 表没命中：hit=false + 保守默认（**不是**让它画出一堆档位）
        let u = caps_json("https://unknown.example.com/v1", "no-such-model");
        assert_eq!(u["hit"], serde_json::json!(false), "{u}");
        assert_eq!(u["reasoning"], serde_json::json!(false), "{u}");
        assert_eq!(u["temperature"], "supported", "温度照发（最坏是对方拒）: {u}");
        assert_eq!(u["levels"].as_object().map(|m| m.len()), Some(0), "{u}");
        assert!(u["max_output_ceiling"].is_null(), "{u}");
    }
}
