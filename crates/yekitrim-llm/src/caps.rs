//! 能力**读取器注册表**：把 /v1/models 里各家的自描述字段读成一份能力片段。
//!
//! 与方言注册表（dialect.rs）同构 —— **加一家标准 = 写一个 read 函数 + 加一行表项**。
//!
//! # 三条硬规矩（写在这里，是因为踩过）
//!
//! 1. **PartialCaps 只说自己知道的**：每个字段 Option，没认出来就是 None。
//! 2. **合并时不许用 None 覆盖已有值**：否则"认出一半"会把表里其余全抹掉
//!    （前端 mergeCaps 踩过同一条）。[merge_into] 与 [apply_partial] 都只补不抹。
//! 3. **认不出的字段忽略**，绝不当成"支持"。
//!
//! # 为什么要注册表
//!
//! 每个网关自描述的方式都不一样（workbuddy 给 reasoning_supported_efforts、OpenRouter 给
//! architecture.input_modalities、裸 vLLM 什么都不给）。把这些形状各写成一个小 reader，
//! 每个条目**依次过一遍**、结果求并集，接新标准就只加一个函数。

use serde_json::Value;

use crate::params::{InputModalities, ModelCaps, ReasoningLevel, Support};

/// 一份**能力片段**：只包含某个 reader 真正读到的东西。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PartialCaps {
    /// 档位 → wire 拼写（`None` = 支持但不发送；**空列表 = 明确不支持推理**）。`None` = 没说。
    pub levels: Option<Vec<(ReasoningLevel, Option<String>)>>,
    pub input: Option<InputModalities>,
    pub temperature: Option<Support>,
    pub context_window: Option<u32>,
    pub max_output_ceiling: Option<u32>,
    pub can_disable: Option<bool>,
    pub only_reasoning: Option<bool>,
    pub default_level: Option<ReasoningLevel>,
    pub tool_call: Option<bool>,
    /// 推荐的推理方言 id（注册表里的 id）。`None` = 没说。
    pub dialect: Option<&'static str>,
}

impl PartialCaps {
    /// 一个字段都没读到。
    pub fn is_empty(&self) -> bool {
        self.levels.is_none()
            && self.input.is_none()
            && self.temperature.is_none()
            && self.context_window.is_none()
            && self.max_output_ceiling.is_none()
            && self.can_disable.is_none()
            && self.only_reasoning.is_none()
            && self.default_level.is_none()
            && self.tool_call.is_none()
            && self.dialect.is_none()
    }

    /// 渲染成**给界面看的形状**（键名与 catalog::caps_json 同源）。
    pub fn to_json(&self) -> Value {
        let mut levels = serde_json::Map::new();
        if let Some(list) = &self.levels {
            for (level, wire) in list {
                levels.insert(
                    level.as_str().to_string(),
                    match wire {
                        Some(w) => Value::String(w.clone()),
                        None => Value::Null,
                    },
                );
            }
        }
        serde_json::json!({
            "levels": levels,
            "temperature": self.temperature.map(|s| match s {
                Support::Supported => "supported",
                Support::Unsupported => "unsupported",
            }),
            "input": self.input.map(|m| m.to_json()),
            "context_window": self.context_window,
            "max_output_ceiling": self.max_output_ceiling,
            "reasoning": self.levels.as_ref().map(|l| !l.is_empty()),
            "can_disable": self.can_disable,
            "only_reasoning": self.only_reasoning.unwrap_or(false),
            "default_level": self.default_level.map(|l| l.as_str()),
            "tool_call": self.tool_call,
            "dialect": self.dialect,
        })
    }
}

/// 一条**读取器**：认一家网关的自描述形状。
pub struct CapsReader {
    /// 稳定 id（写日志/测试用）。
    pub id: &'static str,
    /// 给界面看的中文名。
    pub label: &'static str,
    /// 单位置条目 → 能力片段（**认不出就 None**）。
    pub read: fn(&Value) -> Option<PartialCaps>,
}

// ── 小工具 ──────────────────────────────────────────────────────────────────

fn as_bool(item: &Value, key: &str) -> Option<bool> {
    item.get(key).and_then(Value::as_bool)
}

fn as_u32(item: &Value, key: &str) -> Option<u32> {
    item.get(key).and_then(Value::as_u64).and_then(|n| u32::try_from(n).ok())
}

fn as_str_list<'a>(item: &'a Value, key: &str) -> Option<Vec<&'a str>> {
    item.get(key)
        .and_then(Value::as_array)
        .map(|arr| arr.iter().filter_map(Value::as_str).collect())
}

/// 把一串**档位词**解析成清单：认不出的词丢掉并记一条（**别整个 reader 失败**）。
///
/// `off` 不算"一个档位" ——它是"不推理"，只有明确列了才收（`list_off`）。
fn parse_levels(words: &[&str], list_off: bool) -> Vec<(ReasoningLevel, Option<String>)> {
    let mut out: Vec<(ReasoningLevel, Option<String>)> = Vec::new();
    for raw in words {
        match ReasoningLevel::parse(raw) {
            Some(ReasoningLevel::Off) => {
                if list_off {
                    out.push((ReasoningLevel::Off, None));
                }
            }
            Some(lv) => {
                if !out.iter().any(|(l, _)| *l == lv) {
                    // wire 拼写 = 档位名自己（effort 类方言就是这样；别的方言的 apply 不看这个值）
                    out.push((lv, Some(lv.as_str().to_string())));
                }
            }
            None => tracing::warn!("模型能力里有个认不出的档位词 {raw:?}（已丢掉，不影响其余字段）"),
        }
    }
    out.sort_by_key(|(l, _)| *l);
    out
}

/// 模态串（OpenRouter 的 `architecture.modality` 形如 `"text+image->text"`）→ 模态。
fn modalities_from_text(s: &str) -> InputModalities {
    let input = s.split("->").next().unwrap_or(s);
    let parts: Vec<&str> = input.split(['+', ',', '/']).map(str::trim).collect();
    InputModalities::parse(parts)
}

/// 把 `supported_parameters` 读成"能怎么推理"（OpenRouter 与部分中转都给这串）。
fn read_supported_parameters(item: &Value) -> Option<PartialCaps> {
    let params = item.get("supported_parameters").and_then(Value::as_array)?;
    let has = |needle: &str| {
        params
            .iter()
            .any(|p| p.as_str().map(|s| s.eq_ignore_ascii_case(needle)).unwrap_or(false))
    };
    let mut c = PartialCaps::default();
    if has("reasoning_effort") {
        // 能指定档位：至少 high 档；关 = 不发这个参数
        c.dialect = Some("openai_effort");
        c.levels = Some(vec![
            (ReasoningLevel::Off, None),
            (ReasoningLevel::High, Some("high".into())),
        ]);
    } else if has("reasoning") || has("include_reasoning") {
        // 只能开关
        c.dialect = Some("thinking_type");
        c.levels = Some(vec![
            (ReasoningLevel::Off, None),
            (ReasoningLevel::High, Some("high".into())),
        ]);
    }
    // ⚠️ 温度：**列表存在才下结论**（没有它不代表不支持）
    c.temperature = Some(if has("temperature") { Support::Supported } else { Support::Unsupported });
    Some(c)
}

// ── 各家 reader ─────────────────────────────────────────────────────────────

/// **workbuddy**（2026-10-19 实测那家）：自描述字段最全，一个都不用猜。
///
/// 认：`reasoning_supported_efforts` / `reasoning_default_effort` / `can_disable_thinking` /
/// `only_reasoning` / `supports_reasoning` / `supports_images` / `context_length` /
/// `max_output_tokens` / `supports_tool_call`。
fn read_workbuddy(item: &Value) -> Option<PartialCaps> {
    let mut c = PartialCaps::default();
    let mut any = false;

    if let Some(words) = as_str_list(item, "reasoning_supported_efforts") {
        any = true;
        c.levels = Some(parse_levels(&words, false));
    } else if let Some(false) = as_bool(item, "supports_reasoning") {
        // 明确说不支持推理 ⇒ 空清单（**不是 None**：这是"知道没有"）
        any = true;
        c.levels = Some(Vec::new());
    }
    if let Some(v) = as_bool(item, "supports_reasoning") {
        any = true;
        if v {
            // 支持推理但没给清单 ⇒ 具体档位**不说**（别编）
            c.dialect.get_or_insert("openai_effort");
        }
    }
    if let Some(v) = as_bool(item, "can_disable_thinking") {
        c.can_disable = Some(v);
        any = true;
    }
    if let Some(v) = as_bool(item, "only_reasoning") {
        c.only_reasoning = Some(v);
        any = true;
    }
    if let Some(s) = item.get("reasoning_default_effort").and_then(Value::as_str) {
        any = true;
        match ReasoningLevel::parse(s) {
            Some(lv) => c.default_level = Some(lv),
            None => tracing::warn!("默认档位词认不出：{s:?}（忽略，不影响其余字段）"),
        }
    }
    if let Some(v) = as_bool(item, "supports_images") {
        c.input = Some(InputModalities { text: true, image: v });
        any = true;
    }
    if let Some(n) = as_u32(item, "context_length") {
        c.context_window = Some(n);
        any = true;
    }
    if let Some(n) = as_u32(item, "max_output_tokens") {
        c.max_output_ceiling = Some(n);
        any = true;
    }
    if let Some(v) = as_bool(item, "supports_tool_call") {
        c.tool_call = Some(v);
        any = true;
    }

    any.then_some(c)
}

/// **OpenRouter**（以及抄它形状的中转）。
fn read_openrouter(item: &Value) -> Option<PartialCaps> {
    let mut c = PartialCaps::default();
    let mut any = false;
    let arch = item.get("architecture");
    if let Some(words) = arch.and_then(|a| a.get("input_modalities")).and_then(Value::as_array) {
        c.input = Some(InputModalities::parse(words.iter().filter_map(Value::as_str)));
        any = true;
    } else if let Some(s) = arch.and_then(|a| a.get("modality")).and_then(Value::as_str) {
        c.input = Some(modalities_from_text(s));
        any = true;
    }
    if let Some(n) = item
        .get("top_provider")
        .and_then(|t| t.get("context_length"))
        .and_then(Value::as_u64)
        .and_then(|n| u32::try_from(n).ok())
    {
        c.context_window = Some(n);
        any = true;
    }
    if let Some(params) = read_supported_parameters(item) {
        merge_into(&mut c, params);
        any = true;
    }
    any.then_some(c)
}

/// **兜底**：只认最通用的那几个键。
fn read_generic(item: &Value) -> Option<PartialCaps> {
    let mut c = PartialCaps::default();
    let mut any = false;
    let caps_obj = item.get("capabilities");
    if let Some(words) = as_str_list(item, "input_modalities")
        .or_else(|| as_str_list(item, "modalities"))
        .or_else(|| caps_obj.and_then(|c| as_str_list(c, "input_modalities")))
        .or_else(|| caps_obj.and_then(|c| as_str_list(c, "modalities")))
    {
        c.input = Some(InputModalities::parse(words));
        any = true;
    }
    if let Some(v) = caps_obj.and_then(|c| c.get("vision")).and_then(Value::as_bool) {
        c.input.get_or_insert(InputModalities { text: true, image: false }).image = v;
        any = true;
    }
    if let Some(false) = caps_obj.and_then(|c| c.get("reasoning")).and_then(Value::as_bool) {
        c.levels = Some(Vec::new()); // 明确说没有推理能力
        any = true;
    }
    if let Some(n) = as_u32(item, "context_window") {
        c.context_window = Some(n);
        any = true;
    }
    if let Some(n) = as_u32(item, "max_output_tokens") {
        c.max_output_ceiling = Some(n);
        any = true;
    }
    if c.context_window.is_none() {
        if let Some(n) = as_u32(item, "context_length") {
            c.context_window = Some(n);
            any = true;
        }
    }
    any.then_some(c)
}

/// 全部读取器，**按优先级**（先说的算：后面的只补前面没说的字段）。
pub const READERS: &[CapsReader] = &[
    CapsReader { id: "workbuddy", label: "workbuddy 自描述", read: read_workbuddy },
    CapsReader { id: "openrouter", label: "OpenRouter 形状", read: read_openrouter },
    CapsReader { id: "generic", label: "通用兜底", read: read_generic },
];

/// 把一个条目**依次过所有 reader**，求并集。一个字段都没读到 ⇒ None。
pub fn read_all(item: &Value) -> Option<PartialCaps> {
    let mut out: Option<PartialCaps> = None;
    for r in READERS {
        let Some(part) = (r.read)(item) else { continue };
        match &mut out {
            Some(acc) => merge_into(acc, part),
            None => out = Some(part),
        }
    }
    out.filter(|c| !c.is_empty())
}

/// **只补不抹**：把 `from` 里说过的字段填进 `into`（`None` 一律跳过）。
pub fn merge_into(into: &mut PartialCaps, from: PartialCaps) {
    macro_rules! fill {
        // ⚠️ 只填**空的**：先过的那条 reader 说的算（它是更专门的形状）
        ($($f:ident),* $(,)?) => { $( if into.$f.is_none() { into.$f = from.$f; } )* };
    }
    fill!(
        levels,
        input,
        temperature,
        context_window,
        max_output_ceiling,
        can_disable,
        only_reasoning,
        default_level,
        tool_call,
        dialect,
    );
}

/// 把读到的能力**补**进一份已有 caps（表里的值优先；只有 reader 说了的字段才动）。
///
/// ⚠️ 这是"只回 `supports_images` 时表里的档位/上下文照留"那条判据的实现。
pub fn apply_partial(caps: &mut ModelCaps, p: &PartialCaps) {
    if let Some(list) = &p.levels {
        // 服务端给的清单是**权威**（它说有哪些档就是哪些）—— 这一条会替换、不是合并
        caps.levels.clear();
        for (level, wire) in list {
            caps.levels.insert(*level, wire.clone());
        }
    }
    if let Some(m) = p.input {
        caps.input = m;
    }
    if let Some(s) = p.temperature {
        caps.temperature = s;
    }
    if let Some(n) = p.context_window {
        caps.context_window = Some(n);
    }
    if let Some(n) = p.max_output_ceiling {
        caps.max_output_ceiling = Some(n);
    }
    if let Some(v) = p.can_disable {
        caps.can_disable = Some(v);
    }
    if let Some(v) = p.only_reasoning {
        caps.only_reasoning = v;
    }
    if let Some(lv) = p.default_level {
        caps.default_level = Some(lv);
    }
    if let Some(v) = p.tool_call {
        caps.tool_call = Some(v);
    }
    if let Some(id) = p.dialect {
        caps.dialect = crate::dialect::resolve(Some(id), Some(caps.dialect.max_output_field));
    }
    // 关不掉的推理：把 off 从清单里去掉 —— 摆一个"关闭"给用户就是骗人
    if caps.can_disable == Some(false) || caps.only_reasoning {
        caps.levels.remove(&ReasoningLevel::Off);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn levels_of(c: &PartialCaps) -> Vec<&'static str> {
        c.levels
            .as_ref()
            .map(|l| l.iter().map(|(lv, _)| lv.as_str()).collect())
            .unwrap_or_default()
    }

    /// workbuddy 的完整条目（照实测字段抄的）→ 每一项都要读出来。
    #[test]
    fn workbuddy_fields_are_all_read() {
        let item = json!({
            "id": "cn:deepseek-v4.1-flash",
            "reasoning_supported_efforts": ["low", "high", "max"],
            "reasoning_default_effort": "high",
            "can_disable_thinking": true,
            "only_reasoning": true,
            "supports_reasoning": true,
            "supports_images": true,
            "context_length": 200000,
            "max_output_tokens": 65536,
            "supports_tool_call": true
        });
        let c = read_all(&item).expect("信息这么全，必须读出来");
        assert_eq!(levels_of(&c), vec!["low", "high", "max"], "max 必须被认出来");
        assert_eq!(c.default_level, Some(ReasoningLevel::High));
        assert_eq!(c.can_disable, Some(true));
        assert_eq!(c.only_reasoning, Some(true));
        assert_eq!(c.input.map(|m| m.image), Some(true));
        assert_eq!(c.context_window, Some(200000));
        assert_eq!(c.max_output_ceiling, Some(65536));
        assert_eq!(c.tool_call, Some(true));
    }

    /// 认不出的档位词**丢掉并继续**（别整个 reader 失败），其余字段照读。
    #[test]
    fn unknown_level_words_are_dropped_not_fatal() {
        let item = json!({
            "reasoning_supported_efforts": ["low", "galactic", "max"],
            "can_disable_thinking": false,
            "context_length": 4096
        });
        let c = read_all(&item).unwrap();
        assert_eq!(levels_of(&c), vec!["low", "max"], "认不出的词只丢它自己");
        assert_eq!(c.can_disable, Some(false));
        assert_eq!(c.context_window, Some(4096), "同一份回包里的别的字段不受影响");
    }

    /// 一个能力字段都没有（只有 id/name）⇒ **None**（别编）。
    #[test]
    fn no_capability_fields_means_none() {
        assert!(read_all(&json!({"id": "x", "name": "X"})).is_none());
    }

    /// **合并时不许 None 覆盖**：只回 `supports_images` 时，表里的档位/上下文必须照留。
    #[test]
    fn merge_never_lets_none_wipe_existing_values() {
        // 表里：有档位、有上下文、有输出上限（模拟 caps_for 的产物）
        let mut caps = ModelCaps::default();
        caps.levels.insert(ReasoningLevel::High, Some("high".into()));
        caps.levels.insert(ReasoningLevel::Medium, None);
        caps.context_window = Some(1_000_000);
        caps.max_output_ceiling = Some(384_000);
        caps.temperature = Support::Unsupported;

        // 探测只回了 supports_images（别的字段一个都没有）
        let part = read_all(&json!({"id": "m", "supports_images": true, "supports_tool_call": true}))
            .expect("supports_images 就是一条信息");
        apply_partial(&mut caps, &part);

        assert!(caps.input.image, "读到的字段要生效");
        assert_eq!(caps.tool_call, Some(true));
        assert_eq!(caps.levels.len(), 2, "**表里的档位不许被抹掉**：{:?}", caps.levels);
        assert_eq!(caps.levels.get(&ReasoningLevel::Medium), Some(&None), "null 档也照留");
        assert_eq!(caps.context_window, Some(1_000_000), "表里的上下文不许被抹掉");
        assert_eq!(caps.max_output_ceiling, Some(384_000));
        assert_eq!(caps.temperature, Support::Unsupported, "没说温度的也不许被改回默认");
    }

    /// 两个 reader 都说同一件事时：**先说的算**，后者只补空格。
    #[test]
    fn earlier_readers_win_and_later_ones_fill_gaps() {
        let mut acc = PartialCaps { context_window: Some(111), ..Default::default() };
        merge_into(&mut acc, PartialCaps { context_window: Some(222), max_output_ceiling: Some(333), ..Default::default() });
        assert_eq!(acc.context_window, Some(111), "先说的算");
        assert_eq!(acc.max_output_ceiling, Some(333), "空格由后者补");
    }

    /// 关不掉的推理：`can_disable=false` / `only_reasoning=true` ⇒ 清单里不许留 off。
    #[test]
    fn off_is_removed_when_it_cannot_be_disabled() {
        for (can_disable, only) in [(Some(false), false), (Some(true), true), (None, true)] {
            let mut caps = ModelCaps::default();
            caps.levels.insert(ReasoningLevel::Off, None);
            caps.levels.insert(ReasoningLevel::High, Some("high".into()));
            let part = PartialCaps { can_disable, only_reasoning: Some(only), ..Default::default() };
            apply_partial(&mut caps, &part);
            assert!(
                !caps.levels.contains_key(&ReasoningLevel::Off),
                "can_disable={can_disable:?} only_reasoning={only} ⇒ 不许再摆一个关闭档"
            );
            assert!(caps.levels.contains_key(&ReasoningLevel::High), "别的档照留");
        }
        // 未声明（None）⇒ 保持表里的行为（老工程零回归）
        let mut caps = ModelCaps::default();
        caps.levels.insert(ReasoningLevel::Off, None);
        apply_partial(&mut caps, &PartialCaps::default());
        assert!(caps.levels.contains_key(&ReasoningLevel::Off), "没声明就别动它");
    }

    /// OpenRouter 形状（模态两种写法 + supported_parameters + top_provider）。
    #[test]
    fn openrouter_shape_is_read() {
        let one = read_all(&json!({
            "id": "r1",
            "architecture": {"input_modalities": ["text", "image"]},
            "supported_parameters": ["temperature", "reasoning_effort"],
            "top_provider": {"context_length": 128000}
        }))
        .unwrap();
        assert_eq!(one.input.map(|m| m.image), Some(true));
        assert_eq!(one.dialect, Some("openai_effort"));
        assert_eq!(one.temperature, Some(Support::Supported));
        assert_eq!(one.context_window, Some(128000));
        assert_eq!(levels_of(&one), vec!["off", "high"]);

        // modality 单串写法（"text+image->text"）+ 没有 temperature
        let two = read_all(&json!({
            "id": "r2",
            "architecture": {"modality": "text+image->text"},
            "supported_parameters": ["reasoning"]
        }))
        .unwrap();
        assert_eq!(two.input.map(|m| m.image), Some(true));
        assert_eq!(two.dialect, Some("thinking_type"));
        assert_eq!(two.temperature, Some(Support::Unsupported));
    }

    /// 兜底形状：顶层模态 / capabilities.vision / context_window / context_length。
    #[test]
    fn generic_shape_is_read() {
        let c = read_all(&json!({
            "id": "g",
            "modalities": ["text"],
            "display_name": "g",
            "context_length": 32000,
            "max_output_tokens": 4096
        }))
        .unwrap();
        assert_eq!(c.input.map(|m| m.image), Some(false), "只写 text ⇒ 不吃图片");
        assert_eq!(c.context_window, Some(32000));
        assert_eq!(c.max_output_ceiling, Some(4096));
        assert!(c.levels.is_none(), "没提推理 ⇒ 不下结论");

        let vis = read_all(&json!({"id": "v", "capabilities": {"vision": true, "reasoning": false}})).unwrap();
        assert_eq!(vis.input.map(|m| m.image), Some(true));
        assert_eq!(vis.levels, Some(Vec::new()), "明确说没有推理能力 ⇒ 空清单");
    }

    /// `to_json` 的键名与 `catalog::caps_json` 同源（前端两条路读同一份）。
    #[test]
    fn to_json_shape_matches_caps_json() {
        let c = PartialCaps {
            levels: Some(vec![(ReasoningLevel::Off, None), (ReasoningLevel::Max, Some("max".into()))]),
            input: Some(InputModalities { text: true, image: true }),
            temperature: Some(Support::Supported),
            context_window: Some(1000),
            can_disable: Some(true),
            only_reasoning: Some(true),
            default_level: Some(ReasoningLevel::High),
            dialect: Some("openai_effort"),
            ..Default::default()
        };
        let v = c.to_json();
        assert_eq!(v["levels"]["max"], "max");
        assert!(v["levels"]["off"].is_null());
        assert_eq!(v["default_level"], "high");
        assert_eq!(v["can_disable"], true);
        assert_eq!(v["only_reasoning"], true);
        assert_eq!(v["input"], json!({"text": true, "image": true}));
        assert_eq!(v["temperature"], "supported");
        assert_eq!(v["reasoning"], true);
    }
}
