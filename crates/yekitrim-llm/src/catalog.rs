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

use crate::dialect;
use crate::params::{InputModalities, ModelCaps, ReasoningLevel, Support};
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
    /// **输入模态**（`["text"]` / `["text","image"]`）；缺失 = 只有文字（保守）。
    #[serde(default)]
    input: Option<Vec<String>>,
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

/// 在表里找这对 (host, model)。**匹配是严格的**（2026-10-20 用户质疑后收紧）：
///
/// 1. **整串精确**（大小写不敏感）—— 绝不按后缀 / 裸名模糊命中：
///    `cn:deepseek-v4.1-flash` 与 `deepseek-v4-flash` 是**两支不同的模型**，
///    把公开版的上限（8192/384k）安到自建网关的模型上，就是"用户被告知上限只有 8192"那类事故；
/// 2. **host 命中某一家 ⇒ 只在那一家里找**（找不到就是"未收录"，不跨家猜 ——
///    `moonshot` 的 kimi 与 `openrouter` 的同名模型能力不同，跨家猜等于用别家的数字）；
/// 3. host 认不出 ⇒ 允许全表找，但**必须唯一**（同一个 id 出现在多家 ⇒ 说不清是哪家 ⇒ 不猜）。
fn find_model(base_url: &str, model: &str) -> Option<(&'static ProviderEntry, &'static ModelEntry)> {
    let model = model.trim();
    if model.is_empty() {
        return None;
    }
    let cat = catalog();
    let matches = |p: &'static ProviderEntry| -> Option<&'static ModelEntry> {
        p.models.iter().find(|m| m.id.eq_ignore_ascii_case(model))
    };
    if let Some(p) = find_provider(base_url) {
        // ② 认识的 host：**只认这一家**
        return matches(p).map(|m| (p, m));
    }
    // ③ 认不出的 host：全表找，但只有**唯一命中**才认
    let mut hits = cat.providers.iter().filter_map(|p| matches(p).map(|m| (p, m)));
    let first = hits.next()?;
    if hits.next().is_some() {
        return None; // 有歧义 ⇒ 不猜
    }
    Some(first)
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
    // 方言 = (推理写法, 输出字段名) 这一对 → 注册表里的一行。协议族不对 ⇒ 推理写法按"不发"走，
    // 但**字段名照表**（那是 provider 契约，与推理无关）。未知组合由 dialect::resolve 降级。
    caps.dialect = if openai_compat {
        dialect::resolve(thinking, max_output)
    } else {
        dialect::resolve(None, max_output)
    };
    // 输入模态：表里有 `input` 就按它，**没有就只吃文字**（保守）
    if let Some(items) = &entry.input {
        caps.input = InputModalities::parse(items.iter().map(String::as_str));
    }
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
    caps_json_of(&caps_for(base_url, model), find_model(base_url, model).is_some())
}

/// 同上，但**先把探测结果补进来**。
///
/// # 能力来源的**优先级**（只在这里实现，别再各写一份）
///
/// **探针自描述（probe） > 内置表（catalog） > 保守默认** —— 2026-10-20 用户质疑后钉死：
/// 服务器自己说了 `max_output_tokens = 393216`，就不许被表里的 8192 挡住。
/// 实现顺序 = 先 `caps_for`（表 + 默认）再 `apply_partial`（探针**覆盖**它说了的字段）；
/// `apply_partial` 对"reader 没说"的字段保持不动，所以表/默认该保留的仍保留。
///
/// ⚠️ 表里**没有**的 id（例如自建网关的 `cn:deepseek-v4.1-flash`）走的是"保守默认"，
/// 不是"照公开版猜一个数"—— 见 [`find_model`] 的严格匹配口径。
pub fn caps_json_merged(
    base_url: &str,
    model: &str,
    partial: Option<&crate::caps::PartialCaps>,
) -> serde_json::Value {
    let mut caps = caps_for(base_url, model);
    if let Some(p) = partial {
        crate::caps::apply_partial(&mut caps, p);
    }
    caps_json_of(&caps, find_model(base_url, model).is_some())
}

/// 把一份**解析好的**能力渲染成给界面用的 JSON（`caps_json` / `caps_json_merged` 共用这一处）。
pub fn caps_json_of(caps: &ModelCaps, hit: bool) -> serde_json::Value {
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
        "hit": hit,
        "levels": levels,
        "temperature": match caps.temperature {
            Support::Supported => "supported",
            Support::Unsupported => "unsupported",
        },
        "max_output_ceiling": caps.max_output_ceiling,
        "context_window": caps.context_window,
        "reasoning": caps.reasoning_supported(),
        // 输入模态（文字/图片）：老网关缺省就是 `{text: true, image: false}` —— 别猜它支持图片
        "input": caps.input.to_json(),
        // 关不关得掉（null = 未声明 —— **别默认 false**）、只会推理、服务端给的默认档
        "can_disable": caps.can_disable,
        "only_reasoning": caps.only_reasoning,
        "default_level": caps.default_level.map(|l| l.as_str()),
        "tool_call": caps.tool_call,
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
        assert_eq!(caps.dialect.id, "none", "不认识就不发推理参数");
        assert_eq!(caps.dialect.max_output_field, "max_tokens");
        assert_eq!(caps.input, InputModalities::default(), "不认识 ⇒ 只有文字（不许猜图片）");
    }

    #[test]
    fn 模型级方言优先于_provider_级并集() {
        // `api.moonshot.ai` 这家在表里同时出现两种方言（provider 级是**并集**，单看它无法消歧）。
        // 这条钉的就是「逐模型覆盖真的被读到了」——kimi-k3 与 kimi-k2-0711-preview 必须拿到不同的 wire。
        let a = caps_for("https://api.moonshot.ai/v1", "kimi-k3");
        assert_eq!(a.dialect.id, "openai_effort");
        let b = caps_for("https://api.moonshot.ai/v1", "kimi-k2-0711-preview");
        assert_eq!(b.dialect.id, "thinking_type");
        assert_ne!(a.dialect.id, b.dialect.id, "两家不同方言必须能分辨");
        assert_eq!(a.dialect.max_output_field, "max_tokens", "这家用的是老字段名");
    }


/// **能力来源优先级 = 探针 > 内置表 > 保守默认**（2026-10-20 用户质疑后钉死）。
///
/// 服务器自述 `max_output_tokens = 393216`，表里同一支模型是 384000 —— **必须给 393216**。
#[test]
fn probe_wins_over_catalog_for_the_same_model() {
    // 表里认识的一支（Control：只查表 = 表的值）
    let base = "https://api.deepseek.com/v1";
    let by_table = caps_json(base, "deepseek-v4-pro");
    assert_eq!(by_table["hit"], serde_json::json!(true), "{by_table}");
    assert_eq!(by_table["max_output_ceiling"].as_u64(), Some(384_000), "{by_table}");

    // 探针说 393216 ⇒ 最终必须是探针的
    let partial = crate::caps::read_all(&serde_json::json!({
        "id": "deepseek-v4-pro",
        "max_output_tokens": 393216
    }))
    .expect("max_output_tokens 是一条信息");
    let merged = caps_json_merged(base, "deepseek-v4-pro", Some(&partial));
    assert_eq!(
        merged["max_output_ceiling"].as_u64(),
        Some(393_216),
        "**探针压过表**（服务器自己说的才算）：{merged}"
    );
    // 探针没说的字段仍取表的（不是整体让某一方赢）
    assert_eq!(merged["context_window"], by_table["context_window"], "{merged}");
    assert_eq!(merged["hit"], serde_json::json!(true), "{merged}");
}

/// 表里**没有**这个 id ⇒ 给"未收录"，**不许猜一个数**（尤其别照同后缀的公开版猜）。
#[test]
fn unknown_id_gets_no_guessed_limits() {
    for base in ["http://192.168.0.107:7863/v1", "https://api.deepseek.com/v1"] {
        let v = caps_json(base, "cn:deepseek-v4.1-flash");
        assert_eq!(v["hit"], serde_json::json!(false), "{base}: {v}");
        assert!(v["max_output_ceiling"].is_null(), "不许猜上限：{base}: {v}");
        assert!(v["context_window"].is_null(), "不许猜上下文：{base}: {v}");
    }
}

/// **`cn:` 前缀的 id 不会命中表里任何无前缀同名模型**（严格整串匹配）。
#[test]
fn prefixed_custom_ids_never_match_bare_table_names() {
    let base = "https://api.deepseek.com/v1";
    let custom = caps_for(base, "cn:deepseek-v4-pro");
    assert!(custom.max_output_ceiling.is_none(), "自建网关的模型拿不到公开版的数字");
    assert!(custom.levels.is_empty(), "也不该拿到公开版的档位");
    assert_eq!(custom.dialect.id, "none", "方言也不许借过来");
    // 对照组：裸名**在认识的那家**里照样命中（不是把匹配整体关掉）
    let native = caps_for(base, "deepseek-v4-pro");
    assert_eq!(native.max_output_ceiling, Some(384_000));
    assert!(!native.levels.is_empty());
}

/// host 认不出 + 同一个 id 在**多家**都有 ⇒ 说不清是哪家 ⇒ **不猜**（宁可"未收录"）。
#[test]
fn ambiguous_bare_id_on_an_unknown_host_is_not_guessed() {
    // deepseek-v4-pro 在 deepseek / qwen-token-plan(-cn) 三家都有
    let unknown_host = "https://my-gateway.example/v1";
    let v = caps_json(unknown_host, "deepseek-v4-pro");
    assert_eq!(v["hit"], serde_json::json!(false), "有歧义就别猜：{v}");
    // 但**认识的 host** 只在那一家里找 ⇒ 照样命中
    let v = caps_json("https://api.deepseek.com/v1", "deepseek-v4-pro");
    assert_eq!(v["hit"], serde_json::json!(true), "{v}");
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
        // 输入模态（2026-10-19 新增）：deepseek 只吃文字
        assert_eq!(v["input"], serde_json::json!({"text": true, "image": false}), "{v}");
        // 三件"服务端说了才算"的事：表里没有这些维度 ⇒ **null / false**，不是"关不掉"
        assert!(v["can_disable"].is_null(), "表里没这个维度 ⇒ 未声明（别默认 false）：{v}");
        assert_eq!(v["only_reasoning"], serde_json::json!(false), "{v}");
        assert!(v["default_level"].is_null(), "{v}");
        assert!(v["tool_call"].is_null(), "{v}");

        // 表没命中：hit=false + 保守默认（**不是**让它画出一堆档位）
        let u = caps_json("https://unknown.example.com/v1", "no-such-model");
        assert_eq!(u["hit"], serde_json::json!(false), "{u}");
        assert_eq!(u["reasoning"], serde_json::json!(false), "{u}");
        assert_eq!(u["temperature"], "supported", "温度照发（最坏是对方拒）: {u}");
        assert_eq!(u["levels"].as_object().map(|m| m.len()), Some(0), "{u}");
        assert!(u["max_output_ceiling"].is_null(), "{u}");
        assert_eq!(u["input"], serde_json::json!({"text": true, "image": false}), "未收录 ⇒ 只吃文字：{u}");
    }

    /// **输入模态**：表里写了 `["text","image"]` 的模型必须真的读到 image；
    /// 只写 `["text"]` 的不能"顺手"支持图片。
    #[test]
    fn input_modalities_come_from_the_table() {
        let img = caps_for("https://api.anthropic.com/v1", "claude-opus-4-1");
        assert!(img.input.text, "文字永远支持");
        assert!(img.input.image, "这份 `input` 里写了 image");
        let txt = caps_for("https://api.deepseek.com/v1", "deepseek-v4-pro");
        assert!(txt.input.text);
        assert!(!txt.input.image, "只写 text 的不许被猜成支持图片");
        // 用户覆盖（[llm.models."<id>"]）走的是另一步，见 core 的 config 测试
    }

    /// **探测结果只补不抹**（后端同口径；前端 mergeCaps 也踩过这条）：
    /// 只回 `supports_images` 时，**表里的档位/上下文/输出上限/温度必须照留**。
    #[test]
    fn probe_caps_merge_never_wipes_table_values() {
        let base = caps_json("https://api.deepseek.com/v1", "deepseek-v4-pro");
        let levels_before = base["levels"].as_object().expect("有档位").len();
        let part = crate::caps::read_all(&serde_json::json!({
            "id": "deepseek-v4-pro",
            "supports_images": true,
            "can_disable_thinking": true,
            "reasoning_default_effort": "high"
        }))
        .expect("supports_images 就是一条信息");

        let merged = caps_json_merged("https://api.deepseek.com/v1", "deepseek-v4-pro", Some(&part));
        assert_eq!(merged["hit"], serde_json::json!(true), "表里认识它：{merged}");
        assert_eq!(merged["input"], serde_json::json!({"text": true, "image": true}), "读到的要生效");
        assert_eq!(merged["can_disable"], serde_json::json!(true));
        assert_eq!(merged["default_level"], "high");
        assert_eq!(merged["levels"].as_object().unwrap().len(), levels_before, "表里的档位不许被抹掉：{merged}");
        assert_eq!(merged["context_window"], base["context_window"], "表里的上下文不许被抹掉");
        assert_eq!(merged["max_output_ceiling"], base["max_output_ceiling"]);
        assert_eq!(merged["temperature"], base["temperature"], "没说温度 ⇒ 表里的值照留");

        // 没探到任何东西（None）⇒ 与不传完全一样
        assert_eq!(caps_json_merged("https://api.deepseek.com/v1", "deepseek-v4-pro", None), base);
    }

    /// 关不掉的推理：`can_disable=false` ⇒ **从 levels 里去掉 off**（摆一个"关闭"就是骗人）。
    #[test]
    fn off_level_is_dropped_when_probe_says_it_cannot_be_disabled() {
        let mut caps = caps_for("https://api.deepseek.com/v1", "deepseek-v4-pro");
        caps.levels.insert(ReasoningLevel::Off, None); // 先人为放一档 off
        let part = crate::caps::PartialCaps { can_disable: Some(false), ..Default::default() };
        crate::caps::apply_partial(&mut caps, &part);
        let v = caps_json_of(&caps, true);
        assert!(v["levels"].get("off").is_none(), "关不掉就不许有 off：{v}");
        assert_eq!(v["can_disable"], serde_json::json!(false));
        assert!(v["levels"].as_object().unwrap().len() >= 3, "别的档照留：{v}");
    }

    /// 空/垃圾模态串：忽略未知、`text` 仍在（"没提 textual" ≠ "不支持文字"）。
    #[test]
    fn unknown_modalities_are_ignored() {
        let m = InputModalities::parse(["IMAGE", "audio", "video"]);
        assert!(m.image, "大写 image 也认");
        assert!(m.text, "只说了 image ⇒ 文字仍然可用");
        assert_eq!(m.to_json(), serde_json::json!({"text": true, "image": true}));
        assert_eq!(InputModalities::parse(Vec::<String>::new()), InputModalities::default());
    }
}
