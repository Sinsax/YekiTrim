//! **方言注册表**：一种 API 标准 = 一条表项。
//!
//! # 它解决什么
//!
//! 以前每加一家标准要改三处（enum 变体 / parse / wire::emit 的 match）。现在只有两处：
//! **写一个 apply 函数 + 在 DIALECTS 里加一行** —— params / catalog / 配置层都不用动。
//!
//! # 一条标准是什么
//!
//! 能力表把推理写法（thinking_wire）与输出上限字段名（max_output_wire）**分开记**（那是
//! pi-ai 的 compat 形状）。注册表把它们**合成一条**：同一套推理写法 + 不同的输出字段名 =
//! **两种标准**（字段名是 provider 契约的一部分）。所以表项的键是 **(id, max_output_field)**；
//! id 相同的两行靠 max_output_field 区分。
//!
//! # 三条纪律
//!
//! 1. **认不出的推理串 ⇒ none**（什么都不发）：宁可不发，也不瞎发。
//! 2. 推理串认得、但这一对 (写法, 字段名) 没登记 ⇒ **保住字段名、丢掉推理参数**（见 resolve）——
//!    发错字段名会被 provider 直接 400，而少发一个推理参数最坏只是"思考关不掉"。
//! 3. apply 只往请求体 map 里写自己的键，**顺序与覆盖关系由 wire::apply 统一管**。
//!
//! # 加一家新标准
//!
//! 1. 写一个 fn apply_xxx(body, level, value)；
//! 2. 在 DIALECTS 里加一行（id = 能力表里 thinking_wire 要写的那个串）。
//!
//! 就这两步。ModelCaps.dialect 拿到它之后，wire 与界面都不用改。

use serde_json::{Map, Value};

use crate::params::ReasoningLevel;

/// 一条**方言**（一家 API 标准）：推理参数怎么写 + 输出上限发哪个字段名。
#[derive(Debug, Clone, Copy)]
pub struct Dialect {
    /// 表里的推理写法（thinking_wire 的取值）：openai_effort / thinking_type / …
    pub id: &'static str,
    /// 给界面看的中文名。
    pub label: &'static str,
    /// 输出上限的字段名（max_tokens / max_completion_tokens）。
    pub max_output_field: &'static str,
    /// 把一档写进请求体：(body, level, wire 拼写)。
    pub apply: fn(&mut Map<String, Value>, ReasoningLevel, &str),
}

/// 推理写法 id 的别名（历史拼法 / 用户手写）。None = 不表态。
fn norm_id(id: Option<&str>) -> Option<&'static str> {
    match id.map(|s| s.trim().to_ascii_lowercase()).as_deref() {
        Some("openai_effort" | "reasoning_effort") => Some("openai_effort"),
        Some("thinking_type" | "thinking") => Some("thinking_type"),
        Some("qwen_enable_thinking" | "enable_thinking") => Some("qwen_enable_thinking"),
        Some("openrouter_effort") => Some("openrouter_effort"),
        // 显式写"不发"，或没写 ⇒ 都归到 none
        Some("" | "none") | None => None,
        // 认不出的串 ⇒ 不发（宁可不发，也不瞎发）
        Some(_) => Some("__unknown__"),
    }
}

/// 输出上限字段名归一化：**认不出的一律 max_tokens**（与旧 MaxOutputWire::parse 同口径）。
fn norm_field(field: Option<&str>) -> &'static str {
    match field.map(|s| s.trim().to_ascii_lowercase()).as_deref() {
        Some("max_completion_tokens") => "max_completion_tokens",
        _ => "max_tokens",
    }
}

// ── 各家标准的 apply 实现（**新标准就照这个加一个**）──────────────────────────

/// reasoning_effort: "high"（OpenAI o 系 / gpt-5 / google / anthropic 兼容层…）
fn apply_openai_effort(body: &mut Map<String, Value>, _level: ReasoningLevel, value: &str) {
    body.insert("reasoning_effort".into(), Value::String(value.to_string()));
}

/// thinking: { type: "enabled" | "disabled" }（DeepSeek / Anthropic 兼容层）
fn apply_thinking_type(body: &mut Map<String, Value>, level: ReasoningLevel, _value: &str) {
    let kind = if level == ReasoningLevel::Off { "disabled" } else { "enabled" };
    body.insert("thinking".into(), serde_json::json!({ "type": kind }));
}

/// enable_thinking: true|false（Qwen / DashScope）
fn apply_qwen_enable_thinking(body: &mut Map<String, Value>, level: ReasoningLevel, _value: &str) {
    body.insert("enable_thinking".into(), Value::Bool(level != ReasoningLevel::Off));
}

/// reasoning: { effort: "high" }（OpenRouter）
fn apply_openrouter_effort(body: &mut Map<String, Value>, _level: ReasoningLevel, value: &str) {
    body.insert("reasoning".into(), serde_json::json!({ "effort": value }));
}

/// **什么都不发**（表里没收录 / 认不出的方言）：最保守的那一档。
fn apply_none(_body: &mut Map<String, Value>, _level: ReasoningLevel, _value: &str) {}

// ── 注册表 ──────────────────────────────────────────────────────────────────

/// 全部方言。**加一家标准 = 加一行**（id 取能力表里 thinking_wire 要写的串）。
pub const DIALECTS: &[Dialect] = &[
    Dialect {
        id: "openai_effort",
        label: "推理档位（reasoning_effort）",
        max_output_field: "max_completion_tokens",
        apply: apply_openai_effort,
    },
    Dialect {
        id: "openai_effort",
        label: "推理档位（reasoning_effort）",
        max_output_field: "max_tokens",
        apply: apply_openai_effort,
    },
    Dialect {
        id: "thinking_type",
        label: "thinking.type（开/关）",
        max_output_field: "max_completion_tokens",
        apply: apply_thinking_type,
    },
    Dialect {
        id: "thinking_type",
        label: "thinking.type（开/关）",
        max_output_field: "max_tokens",
        apply: apply_thinking_type,
    },
    Dialect {
        id: "qwen_enable_thinking",
        label: "enable_thinking（通义/百炼）",
        max_output_field: "max_completion_tokens",
        apply: apply_qwen_enable_thinking,
    },
    Dialect {
        id: "openrouter_effort",
        label: "reasoning.effort（OpenRouter）",
        max_output_field: "max_completion_tokens",
        apply: apply_openrouter_effort,
    },
    // 不发推理参数的保守档：两个字段名各一条（字段名来自能力表、与推理无关）
    Dialect {
        id: "none",
        label: "不发推理参数",
        max_output_field: "max_tokens",
        apply: apply_none,
    },
    Dialect {
        id: "none",
        label: "不发推理参数",
        max_output_field: "max_completion_tokens",
        apply: apply_none,
    },
];

/// 最保守的那条（不认识任何东西时就指向它）。
pub fn none() -> &'static Dialect {
    &DIALECTS[6]
}

/// 按 id 找一条（同 id 多行时返回第一行；要精确请用 [resolve]）。
pub fn dialect(id: &str) -> Option<&'static Dialect> {
    // 别名归一（历史拼法），但 "none" / 空串要**指向 none 那一行**而不是返回 None
    let want = norm_id(Some(id)).unwrap_or("none");
    if want == "__unknown__" {
        return None;
    }
    DIALECTS.iter().find(|d| d.id == want)
}

/// 把所有 id 列出来（给界面下拉/文档用；不重复）。
pub fn ids() -> Vec<&'static str> {
    let mut out: Vec<&'static str> = Vec::new();
    for d in DIALECTS {
        if !out.contains(&d.id) {
            out.push(d.id);
        }
    }
    out
}

/// **表里的 (thinking_wire, max_output_wire) → 一条表项**。永不失败（最坏给 none）。
///
/// 降级顺序：
/// 1. 这一对**精确登记过** ⇒ 用它；
/// 2. 推理写法认得、但这一对没登记 ⇒ **保住字段名、丢掉推理参数**（理由见模块头第 2 条）；
/// 3. 推理串认不出 / 字段名也认不出 ⇒ none。
pub fn resolve(thinking_wire: Option<&str>, max_output_wire: Option<&str>) -> &'static Dialect {
    let field = norm_field(max_output_wire);
    if let Some(id) = norm_id(thinking_wire) {
        if id != "__unknown__" {
            if let Some(d) = DIALECTS.iter().find(|d| d.id == id && d.max_output_field == field) {
                return d;
            }
        }
    }
    DIALECTS
        .iter()
        .find(|d| d.id == "none" && d.max_output_field == field)
        .unwrap_or_else(none)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 表里每个 id 都能被 dialect() 认出来，且都有中文名（界面要用）。
    #[test]
    fn every_registered_id_is_findable_and_labelled() {
        for d in DIALECTS {
            assert!(!d.id.is_empty() && !d.label.is_empty(), "{d:?}");
            let got = dialect(d.id).expect("dialect() 必须能认出表里登记过的 id");
            assert_eq!(got.id, d.id);
        }
        assert_eq!(ids().len(), 5, "id 去重后应当是 5 条（含 none）");
    }

    /// 表里的六种真实组合各落到哪条 —— 这是"行为不变"的地基。
    #[test]
    fn table_combinations_resolve_to_the_expected_rows() {
        let cases = [
            ("openai_effort", "max_completion_tokens", "openai_effort"),
            ("openai_effort", "max_tokens", "openai_effort"),
            ("thinking_type", "max_completion_tokens", "thinking_type"),
            ("thinking_type", "max_tokens", "thinking_type"),
            ("qwen_enable_thinking", "max_completion_tokens", "qwen_enable_thinking"),
            ("openrouter_effort", "max_completion_tokens", "openrouter_effort"),
        ];
        for (wire, field, want) in cases {
            let d = resolve(Some(wire), Some(field));
            assert_eq!(d.id, want, "{wire}+{field}");
            assert_eq!(d.max_output_field, field, "{wire}+{field} 的字段名");
        }
    }

    /// 认不出的推理串 ⇒ none；字段名一个都别丢。
    #[test]
    fn unknown_wire_degrades_to_none_but_keeps_the_field() {
        let d = resolve(Some("brand_new_thing"), Some("max_completion_tokens"));
        assert_eq!(d.id, "none", "认不出的串必须降级成不发推理参数");
        assert_eq!(d.max_output_field, "max_completion_tokens", "字段名要保住");
        assert_eq!(resolve(None, None).max_output_field, "max_tokens");
        assert_eq!(resolve(None, Some("whatever")).max_output_field, "max_tokens");
        assert_eq!(resolve(None, Some("max_completion_tokens")).id, "none");
    }

    /// 别名（历史拼法 / 用户手写）也能认出来。
    #[test]
    fn historical_aliases_still_resolve() {
        assert_eq!(resolve(Some("reasoning_effort"), None).id, "openai_effort");
        assert_eq!(resolve(Some("thinking"), None).id, "thinking_type");
        assert_eq!(
            resolve(Some("enable_thinking"), Some("max_completion_tokens")).id,
            "qwen_enable_thinking"
        );
    }
}
