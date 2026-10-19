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
//! 1. 写一个 fn apply_xxx(body, level, value, budget)；
//! 2. 在 DIALECTS 里加**两行**（id = 能力表里 thinking_wire 要写的那个串；
//!    两个 max_output_field 各一行 —— 只登记一种字段名的话，另一家的用户在覆盖里选它
//!    会掉进"保住字段名、丢掉推理"那条降级路，静默不生效）；
//! 3. 在 norm_id 里认下这个 id（否则 resolve 认不出它 ⇒ 永远降级成 none）。
//!
//! ModelCaps.dialect 拿到它之后，wire 与界面都不用改。
//!
//! # 预算型方言的 N
//!
//! thinking_budget / reasoning_max_tokens / thinking_token_budget 都要一个数：
//! 用户配了 `[llm.params] thinking_budget` 就用它，没配按 [budget_for] 的档位映射表给默认。
//! ⚠️ **映射表只有那一处**（别在 apply 里各写一份）。

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
    /// 把一档写进请求体：(body, level, wire 拼写, 预算)。
    ///
    /// 第四个参数只有预算型方言（thinking_budget / reasoning_max_tokens /
    /// thinking_token_budget）会用；其余忽略。`None` = 这一档没有预算（别硬编一个数）。
    pub apply: fn(&mut Map<String, Value>, ReasoningLevel, &str, Option<u32>),
}

/// 推理写法 id 的别名（历史拼法 / 用户手写）。None = 不表态。
fn norm_id(id: Option<&str>) -> Option<&'static str> {
    match id.map(|s| s.trim().to_ascii_lowercase()).as_deref() {
        Some("openai_effort" | "reasoning_effort") => Some("openai_effort"),
        Some("thinking_type" | "thinking") => Some("thinking_type"),
        Some("qwen_enable_thinking" | "enable_thinking") => Some("qwen_enable_thinking"),
        Some("openrouter_effort") => Some("openrouter_effort"),
        // P2 补的六条（别名一起认：用户手写时常常写"像 wire 的那个"名字）
        Some("thinking_budget" | "anthropic_thinking" | "budget_tokens") => Some("thinking_budget"),
        Some("reasoning_max_tokens" | "openrouter_max_tokens") => Some("reasoning_max_tokens"),
        Some("chat_template_kwargs" | "vllm_enable_thinking" | "sglang_enable_thinking") => {
            Some("chat_template_kwargs")
        }
        Some("thinking_token_budget" | "sglang_token_budget") => Some("thinking_token_budget"),
        Some("ollama_think" | "think") => Some("ollama_think"),
        Some("gemini_thinking" | "generation_config_thinking") => Some("gemini_thinking"),
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

/// **档位 → 预算 token** 的唯一映射表（P2；`[llm.params] thinking_budget` 没配时用它）。
///
/// `None` = 这一档没有预算可言（`off` 不发预算；由各方言的 apply 决定"关"长什么样）。
/// ⚠️ 只在 [budget_for] 这一处写数字，apply 里不许再抄一份。
pub fn budget_for(level: ReasoningLevel) -> Option<u32> {
    Some(match level {
        ReasoningLevel::Off => return None,
        ReasoningLevel::Minimal => 1024,
        ReasoningLevel::Low => 4096,
        ReasoningLevel::Medium => 8192,
        ReasoningLevel::High => 16384,
        ReasoningLevel::XHigh => 32768,
        ReasoningLevel::Max => 65536,
    })
}

// ── 各家标准的 apply 实现（**新标准就照这个加一个**）──────────────────────────

/// reasoning_effort: "high"（OpenAI o 系 / gpt-5 / google / anthropic 兼容层…）
fn apply_openai_effort(
    body: &mut Map<String, Value>,
    _level: ReasoningLevel,
    value: &str,
    _budget: Option<u32>,
) {
    body.insert("reasoning_effort".into(), Value::String(value.to_string()));
}

/// thinking: { type: "enabled" | "disabled" }（DeepSeek / Anthropic 兼容层）
fn apply_thinking_type(
    body: &mut Map<String, Value>,
    level: ReasoningLevel,
    _value: &str,
    _budget: Option<u32>,
) {
    let kind = if level == ReasoningLevel::Off { "disabled" } else { "enabled" };
    body.insert("thinking".into(), serde_json::json!({ "type": kind }));
}

/// enable_thinking: true|false（Qwen / DashScope）
fn apply_qwen_enable_thinking(
    body: &mut Map<String, Value>,
    level: ReasoningLevel,
    _value: &str,
    _budget: Option<u32>,
) {
    body.insert("enable_thinking".into(), Value::Bool(level != ReasoningLevel::Off));
}

/// reasoning: { effort: "high" }（OpenRouter）
fn apply_openrouter_effort(
    body: &mut Map<String, Value>,
    _level: ReasoningLevel,
    value: &str,
    _budget: Option<u32>,
) {
    body.insert("reasoning".into(), serde_json::json!({ "effort": value }));
}

/// **什么都不发**（表里没收录 / 认不出的方言）：最保守的那一档。
fn apply_none(
    _body: &mut Map<String, Value>,
    _level: ReasoningLevel,
    _value: &str,
    _budget: Option<u32>,
) {}

/// thinking: { type: "enabled", budget_tokens: N }（Anthropic 原生 / Cohere）
///
/// `off` ⇒ `{"type":"disabled"}`（这一族的语义就是显式关掉，不是"不发"）。
fn apply_thinking_budget(
    body: &mut Map<String, Value>,
    level: ReasoningLevel,
    _value: &str,
    budget: Option<u32>,
) {
    if level == ReasoningLevel::Off {
        body.insert("thinking".into(), serde_json::json!({ "type": "disabled" }));
        return;
    }
    let Some(n) = budget else { return }; // 没预算 ⇒ 不发（别编一个数）
    body.insert("thinking".into(), serde_json::json!({ "type": "enabled", "budget_tokens": n }));
}

/// reasoning: { max_tokens: N }（OpenRouter 另一档）
///
/// `off` ⇒ `{"enabled": false}`（OpenRouter 的关就是它）。
fn apply_reasoning_max_tokens(
    body: &mut Map<String, Value>,
    level: ReasoningLevel,
    _value: &str,
    budget: Option<u32>,
) {
    if level == ReasoningLevel::Off {
        body.insert("reasoning".into(), serde_json::json!({ "enabled": false }));
        return;
    }
    let Some(n) = budget else { return };
    body.insert("reasoning".into(), serde_json::json!({ "max_tokens": n }));
}

/// chat_template_kwargs: { enable_thinking: bool }（vLLM / SGLang 的模板开关）
fn apply_chat_template_kwargs(
    body: &mut Map<String, Value>,
    level: ReasoningLevel,
    _value: &str,
    _budget: Option<u32>,
) {
    body.insert(
        "chat_template_kwargs".into(),
        serde_json::json!({ "enable_thinking": level != ReasoningLevel::Off }),
    );
}

/// thinking_token_budget: N（SGLang）
///
/// `off` ⇒ **不发**（这一族是"给了预算才开"，没有预算就等于没开）。
fn apply_thinking_token_budget(
    body: &mut Map<String, Value>,
    level: ReasoningLevel,
    _value: &str,
    budget: Option<u32>,
) {
    if level == ReasoningLevel::Off {
        return;
    }
    let Some(n) = budget else { return };
    body.insert("thinking_token_budget".into(), serde_json::json!(n));
}

/// think: true | "high"（Ollama）—— **值就是档位名**，`off` ⇒ `false`。
fn apply_ollama_think(
    body: &mut Map<String, Value>,
    level: ReasoningLevel,
    value: &str,
    _budget: Option<u32>,
) {
    if level == ReasoningLevel::Off {
        body.insert("think".into(), Value::Bool(false));
        return;
    }
    body.insert("think".into(), Value::String(value.to_string()));
}

/// **注册了，但明确不发**（Gemini）。
///
/// Gemini 的推理开关在 `generationConfig.thinkingConfig` 里，是**另一族 API**
/// （请求体、鉴权头、流式全都不同，见方案文档 P5）。我们现在的请求体是 `chat/completions`
/// ⇒ 这一条**什么键都不写**。
///
/// ⚠️ **别在这里假装支持**：发一个对方不认的键，比不发更坏（它可能 400，也可能被静默忽略
/// 而用户以为生效了 —— 2026-10-19 实测那家网关正是后一种）。
fn apply_gemini_thinking(
    _body: &mut Map<String, Value>,
    _level: ReasoningLevel,
    _value: &str,
    _budget: Option<u32>,
) {
}

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
    // ── P2 补的六条标准 ──────────────────────────────────────────────────────
    //
    // ⚠️ 前五条**两个字段名各一行**：只登记一种的话，另一家的用户在
    //    [llm.models."x"].thinking_wire 里选它会掉进"保住字段名、丢掉推理"那条降级路
    //    （不报错、只是推理参数静默不发）。
    Dialect {
        id: "thinking_budget",
        label: "thinking.budget_tokens（Anthropic/Cohere）",
        max_output_field: "max_tokens",
        apply: apply_thinking_budget,
    },
    Dialect {
        id: "thinking_budget",
        label: "thinking.budget_tokens（Anthropic/Cohere）",
        max_output_field: "max_completion_tokens",
        apply: apply_thinking_budget,
    },
    Dialect {
        id: "reasoning_max_tokens",
        label: "reasoning.max_tokens（OpenRouter）",
        max_output_field: "max_tokens",
        apply: apply_reasoning_max_tokens,
    },
    Dialect {
        id: "reasoning_max_tokens",
        label: "reasoning.max_tokens（OpenRouter）",
        max_output_field: "max_completion_tokens",
        apply: apply_reasoning_max_tokens,
    },
    Dialect {
        id: "chat_template_kwargs",
        label: "chat_template_kwargs（vLLM/SGLang）",
        max_output_field: "max_tokens",
        apply: apply_chat_template_kwargs,
    },
    Dialect {
        id: "chat_template_kwargs",
        label: "chat_template_kwargs（vLLM/SGLang）",
        max_output_field: "max_completion_tokens",
        apply: apply_chat_template_kwargs,
    },
    Dialect {
        id: "thinking_token_budget",
        label: "thinking_token_budget（SGLang）",
        max_output_field: "max_tokens",
        apply: apply_thinking_token_budget,
    },
    Dialect {
        id: "thinking_token_budget",
        label: "thinking_token_budget（SGLang）",
        max_output_field: "max_completion_tokens",
        apply: apply_thinking_token_budget,
    },
    Dialect {
        id: "ollama_think",
        label: "think（Ollama）",
        max_output_field: "max_tokens",
        apply: apply_ollama_think,
    },
    Dialect {
        id: "ollama_think",
        label: "think（Ollama）",
        max_output_field: "max_completion_tokens",
        apply: apply_ollama_think,
    },
    // Gemini：**注册但明确不发**（另一族 API；见 apply_gemini_thinking 的注释）。
    // 它什么都不写 ⇒ 一条就够（字段名对它没有意义）。
    Dialect {
        id: "gemini_thinking",
        label: "Gemini thinkingConfig（注册不发送）",
        max_output_field: "max_tokens",
        apply: apply_gemini_thinking,
    },
];

/// 最保守的那条（不认识任何东西时就指向它）。
///
/// ⚠️ 按 (id, 字段名)**查**而不是写下标：表里加行不该动这里。
pub fn none() -> &'static Dialect {
    DIALECTS
        .iter()
        .find(|d| d.id == "none" && d.max_output_field == "max_tokens")
        .expect("注册表里必须有 none/max_tokens 这一条")
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

    /// 按某条方言**发一档**（与 wire::apply 同一条调用约定：预算由调用方给）。
    fn fire(id: &str, field: &str, level: ReasoningLevel, budget: Option<u32>) -> serde_json::Value {
        let d = resolve(Some(id), Some(field));
        assert_eq!(d.id, id, "{id} 得在表里");
        let mut body = serde_json::Map::new();
        (d.apply)(&mut body, level, level.as_str(), budget);
        serde_json::Value::Object(body)
    }

    /// 预算映射表：**越强的档给越多的预算**，`off` 没有预算。
    #[test]
    fn budget_table_is_the_single_source() {
        assert_eq!(budget_for(ReasoningLevel::Off), None, "off 不发预算");
        assert_eq!(budget_for(ReasoningLevel::Minimal), Some(1024));
        assert_eq!(budget_for(ReasoningLevel::Low), Some(4096));
        assert_eq!(budget_for(ReasoningLevel::Medium), Some(8192));
        assert_eq!(budget_for(ReasoningLevel::High), Some(16384));
        assert_eq!(budget_for(ReasoningLevel::XHigh), Some(32768));
        assert_eq!(budget_for(ReasoningLevel::Max), Some(65536));
        // 单调（档位越强预算越大）—— 表被改乱时这条会红
        let seq = [
            ReasoningLevel::Minimal,
            ReasoningLevel::Low,
            ReasoningLevel::Medium,
            ReasoningLevel::High,
            ReasoningLevel::XHigh,
            ReasoningLevel::Max,
        ];
        for w in seq.windows(2) {
            assert!(budget_for(w[0]) < budget_for(w[1]), "{:?} → {:?}", w[0], w[1]);
        }
    }

    /// `thinking_budget`：Anthropic/Cohere 形状；off = **显式 disabled**；没预算不发。
    #[test]
    fn thinking_budget_shape() {
        assert_eq!(
            fire("thinking_budget", "max_tokens", ReasoningLevel::High, Some(16384)),
            serde_json::json!({"thinking": {"type": "enabled", "budget_tokens": 16384}})
        );
        assert_eq!(
            fire("thinking_budget", "max_tokens", ReasoningLevel::High, None),
            serde_json::json!({}),
            "没预算 ⇒ 一个键都不发（别编一个数）"
        );
        assert_eq!(
            fire("thinking_budget", "max_tokens", ReasoningLevel::Off, Some(16384)),
            serde_json::json!({"thinking": {"type": "disabled"}}),
            "这一族的关是显式 disabled"
        );
        assert_eq!(
            resolve(Some("thinking_budget"), Some("max_completion_tokens")).id,
            "thinking_budget",
            "两种输出字段名都要认得（否则另一家的用户选它会被静默降级）"
        );
    }

    /// `reasoning_max_tokens`：OpenRouter 的预算档；off = `{"enabled": false}`。
    #[test]
    fn reasoning_max_tokens_shape() {
        assert_eq!(
            fire("reasoning_max_tokens", "max_tokens", ReasoningLevel::Low, Some(4096)),
            serde_json::json!({"reasoning": {"max_tokens": 4096}})
        );
        assert_eq!(
            fire("reasoning_max_tokens", "max_tokens", ReasoningLevel::Off, Some(4096)),
            serde_json::json!({"reasoning": {"enabled": false}})
        );
        assert_eq!(fire("reasoning_max_tokens", "max_tokens", ReasoningLevel::High, None), serde_json::json!({}));
    }

    /// `chat_template_kwargs`：vLLM/SGLang 的模板开关（布尔，不看预算）。
    #[test]
    fn chat_template_kwargs_shape() {
        assert_eq!(
            fire("chat_template_kwargs", "max_tokens", ReasoningLevel::High, None),
            serde_json::json!({"chat_template_kwargs": {"enable_thinking": true}})
        );
        assert_eq!(
            fire("chat_template_kwargs", "max_tokens", ReasoningLevel::Off, None),
            serde_json::json!({"chat_template_kwargs": {"enable_thinking": false}})
        );
    }

    /// `thinking_token_budget`：SGLang 的预算键；off = **不发**。
    #[test]
    fn thinking_token_budget_shape() {
        assert_eq!(
            fire("thinking_token_budget", "max_tokens", ReasoningLevel::Max, Some(65536)),
            serde_json::json!({"thinking_token_budget": 65536})
        );
        assert_eq!(
            fire("thinking_token_budget", "max_tokens", ReasoningLevel::Off, Some(65536)),
            serde_json::json!({}),
            "这一族是「给了预算才开」，没有预算就等于没开"
        );
        assert_eq!(fire("thinking_token_budget", "max_tokens", ReasoningLevel::High, None), serde_json::json!({}));
    }

    /// `ollama_think`：**值就是档位名**；off = `false`。
    #[test]
    fn ollama_think_shape() {
        assert_eq!(
            fire("ollama_think", "max_tokens", ReasoningLevel::High, None),
            serde_json::json!({"think": "high"})
        );
        assert_eq!(
            fire("ollama_think", "max_tokens", ReasoningLevel::Max, None),
            serde_json::json!({"think": "max"})
        );
        assert_eq!(
            fire("ollama_think", "max_tokens", ReasoningLevel::Off, None),
            serde_json::json!({"think": false})
        );
    }

    /// `gemini_thinking`：**注册了但明确不发**（另一族 API；假装支持比不发更坏）。
    #[test]
    fn gemini_is_registered_but_sends_nothing() {
        let d = dialect("gemini_thinking").expect("要在表里（界面/文档看得到它）");
        assert!(d.label.contains("注册不发送"), "标签要写明：{}", d.label);
        for level in [ReasoningLevel::Off, ReasoningLevel::High, ReasoningLevel::Max] {
            assert_eq!(
                fire("gemini_thinking", "max_tokens", level, Some(16384)),
                serde_json::json!({}),
                "{level:?} 也不许发（要接 Gemini 得先有协议适配）"
            );
        }
    }

    /// 每个新 id 都能被 dialect() 认出来（否则 resolve 认不出它 ⇒ 永远降级成 none）。
    #[test]
    fn new_ids_are_recognised_by_id_lookup() {
        for id in [
            "thinking_budget",
            "reasoning_max_tokens",
            "chat_template_kwargs",
            "thinking_token_budget",
            "ollama_think",
            "gemini_thinking",
        ] {
            let d = dialect(id).unwrap_or_else(|| panic!("{id} 必须能被 dialect() 认出来"));
            assert_eq!(d.id, id);
            assert!(!d.label.is_empty());
        }
    }

    /// 表里每个 id 都能被 dialect() 认出来，且都有中文名（界面要用）。
    #[test]
    fn every_registered_id_is_findable_and_labelled() {
        for d in DIALECTS {
            assert!(!d.id.is_empty() && !d.label.is_empty(), "{d:?}");
            let got = dialect(d.id).expect("dialect() 必须能认出表里登记过的 id");
            assert_eq!(got.id, d.id);
        }
        assert_eq!(ids().len(), 11, "id 去重后应当是 11 条（5 条原有 + P2 六条，含 none）");
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
