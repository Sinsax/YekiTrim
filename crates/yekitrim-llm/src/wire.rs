//! ★ **唯一的方言适配层**：把归一化参数翻译成各家 API 的 wire 形态。
//!
//! # 为什么只有这一层能写 wire 字面量
//!
//! `reasoning_effort` / `thinking.type` / `enable_thinking` / `reasoning.effort` /
//! `max_completion_tokens` / `response_format` —— 这些是**别家的协议**。它们一旦散到业务 crate 里，
//! 就会出现「同一个旋钮在两处各写一半」，而且换个模型要改五个文件。
//! 机器判据：`scripts/check_module_boundaries.py` 的 `only_llm_knows_wire`。
//!
//! # 顺序（有讲究，别调）
//!
//! `model → max_output → temperature → top_p → json_mode → extra → reasoning`
//!
//! * `extra` 排在结构化的那几项**之后**：它是 2026-10 之前的老逃逸口，当年就是最后盖 ——
//!   改顺序会让存量 `llm.toml`（里面写死了 `response_format`）的请求体悄悄变样。
//! * `reasoning` 再排最后：旧实现里 `[llm] thinking` 也是最后一个盖 `extra_params.thinking` 的。

use crate::params::{LlmParams, ReasoningLevel, Support};
use serde_json::{json, Value};

/// 把参数写进请求体。`body` 里已经有 `messages`。
pub fn apply(body: &mut Value, p: &LlmParams) {
    if !p.model.is_empty() {
        body["model"] = json!(p.model);
    }
    // 输出上限：**字段名由方言给**（一家一个；o 系/gpt-5 只认 max_completion_tokens）；None = 不发送
    if let Some(n) = p.max_tokens {
        body[p.caps.dialect.max_output_field] = json!(n);
    }
    // 温度：模型不接受就**不发**（有的是 400，有的是静默忽略 —— 不发两边都不炸）
    if p.caps.temperature == Support::Supported {
        body["temperature"] = json!(p.temperature);
    }
    if let Some(top_p) = p.top_p {
        body["top_p"] = json!(top_p);
    }
    if p.json_mode {
        body["response_format"] = json!({ "type": "json_object" });
    }
    // 老逃逸口（最后盖结构化项）
    if let Some(Value::Object(extra)) = &p.extra_params {
        for (k, v) in extra {
            body[k] = v.clone();
        }
    }
    apply_reasoning(body, p);
}

/// 推理参数：**只发表里声明过的档位**；缺档 = 不支持 ⇒ 什么都不发。
///
/// ⚠️ `p.reasoning == None` = **没表态** ⇒ 一个字都不发（这是 F7 之后唯一的"不表态"表达方式：
/// 以前靠 `reasoning_explicit` 这个布尔，与档位值分居两处，谁读错都会把链路默认顶掉）。
fn apply_reasoning(body: &mut Value, p: &LlmParams) {
    let Some(level) = p.reasoning else {
        return;
    };
    if !p.caps.reasoning_supported() {
        // 表里没有这支模型（自建网关/新模型）：表态过就用最保守的老形态
        // （`thinking{type}` 是各家兼容层的事实标准），免得把老工程的档位一升上来就静默失效。
        let kind = if level == ReasoningLevel::Off { "disabled" } else { "enabled" };
        body["thinking"] = json!({ "type": kind });
        return;
    }
    let Some(slot) = p.caps.wire_for(level) else {
        return; // 该档不支持 ⇒ 不发（不是发个错的）
    };
    match slot {
        // 支持但"不发送"（多数 provider 的"不思考"就是参数缺席）
        None => {}
        Some(value) => emit(body, p, value),
    }
}

/// 按**方言**把一档写成具体的键。
///
/// ⚠️ 这里**不认识任何一家 API**：怎么写全由 `dialect.apply` 决定（注册表在 [`crate::dialect`]）。
/// 未知方言（`dialect::none()`）的 apply 是空函数 ⇒ 什么都不发。
fn emit(body: &mut Value, p: &LlmParams, value: &str) {
    let Some(level) = p.reasoning else { return };
    if let Some(map) = body.as_object_mut() {
        // 预算：用户配了就用它，没配按**档位映射表**（那张表只在 dialect::budget_for 一处）
        let budget = p
            .thinking_budget
            .or_else(|| crate::dialect::budget_for(level));
        (p.caps.dialect.apply)(map, level, value, budget);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dialect::{self, Dialect};
    use crate::params::{ModelCaps, ReasoningLevel, Support};

    fn params(levels: &[(ReasoningLevel, Option<&str>)], d: &'static Dialect) -> LlmParams {
        let mut caps = ModelCaps::default();
        caps.dialect = d;
        for (lv, v) in levels {
            caps.levels.insert(*lv, v.map(|s| s.to_string()));
        }
        LlmParams { caps, ..Default::default() }
    }

    /// 取一条已登记的表项（id + 输出字段名）。
    fn dlg(id: &str, field: &str) -> &'static Dialect {
        dialect::resolve(Some(id), Some(field))
    }

    #[test]
    fn 表里没有的档位一个都不发() {
        let p = params(&[(ReasoningLevel::High, Some("high"))], dlg("openai_effort", "max_tokens"));
        let mut body = json!({});
        apply(&mut body, &LlmParams { reasoning: Some(ReasoningLevel::Low), ..p });
        assert!(body.get("reasoning_effort").is_none(), "Low 不在表里就不许发：{body}");
    }

    #[test]
    fn null_档位表示支持但不发送() {
        let p = params(&[(ReasoningLevel::Off, None)], dlg("thinking_type", "max_tokens"));
        let mut body = json!({});
        apply(&mut body, &LlmParams { reasoning: Some(ReasoningLevel::Off), ..p });
        assert!(body.get("thinking").is_none(), "off 的 wire 是 null（缺席）⇒ 不许写键：{body}");
    }

    #[test]
    fn 温度不支持的模型不发_temperature() {
        let mut p = params(&[], dialect::none());
        p.caps.temperature = Support::Unsupported;
        let mut body = json!({});
        apply(&mut body, &p);
        assert!(body.get("temperature").is_none(), "温度不支持就别发：{body}");
    }

    #[test]
    fn max_output_按能力换字段名() {
        let mut p = params(&[], dlg("none", "max_completion_tokens"));
        p.max_tokens = Some(4096);
        let mut body = json!({});
        apply(&mut body, &p);
        assert_eq!(body["max_completion_tokens"], 4096);
        assert!(body.get("max_tokens").is_none());
    }

    #[test]
    fn extra_最后盖结构化项而_reasoning_再盖_extra() {
        let p = LlmParams {
            extra_params: Some(json!({"temperature": 1.5, "thinking": {"type": "disabled"}})),
            ..params(&[(ReasoningLevel::High, Some("high"))], dlg("thinking_type", "max_tokens"))
        };
        let mut body = json!({});
        apply(&mut body, &LlmParams { reasoning: Some(ReasoningLevel::High), ..p });
        assert_eq!(body["temperature"], 1.5, "extra 要盖掉结构化的温度（老行为）");
        assert_eq!(body["thinking"]["type"], "enabled", "reasoning 再盖掉 extra（老行为）");
    }

    /// **加一家新标准 = 加一条表项**：这里临时定义一条方言（推理写成 `test_reasoning`、
    /// 输出上限发 `test_max_out`）—— `wire.rs` 与 `params.rs` **一行都没改**，它照样生效。
    ///
    /// 这是"方便后续适配更多标准 api"那条要求的判据：如果哪天有人把"怎么写"又焊回
    /// `wire` 的 match 里，这条会红（新表项不再被认识）。
    fn apply_test_dialect(
        body: &mut serde_json::Map<String, serde_json::Value>,
        _level: ReasoningLevel,
        value: &str,
        _budget: Option<u32>,
    ) {
        body.insert("test_reasoning".into(), serde_json::json!(value));
    }

    static TEST_DIALECT: Dialect = Dialect {
        id: "test_dialect",
        label: "测试方言",
        max_output_field: "test_max_out",
        apply: apply_test_dialect,
    };

    #[test]
    fn new_dialect_needs_only_a_table_row() {
        let p = params(&[(ReasoningLevel::High, Some("high"))], &TEST_DIALECT);
        let mut body = json!({});
        apply(
            &mut body,
            &LlmParams { max_tokens: Some(4096), reasoning: Some(ReasoningLevel::High), ..p },
        );
        assert_eq!(body["test_reasoning"], "high", "新方言的推理写法直接生效（wire 不认识它）");
        assert_eq!(body["test_max_out"], 4096, "新方言的输出字段名直接生效");
        assert!(body.get("reasoning_effort").is_none(), "别串到别家方言上：{body}");
        assert!(body.get("max_tokens").is_none(), "{body}");
    }

    /// 预算：**用户配了就用它**，没配按档位映射表（表只在 `dialect::budget_for` 一处）。
    #[test]
    fn configured_budget_wins_over_the_level_map() {
        let p = params(&[(ReasoningLevel::High, Some("high"))], dlg("thinking_budget", "max_tokens"));
        let mut body = json!({});
        apply(
            &mut body,
            &LlmParams { reasoning: Some(ReasoningLevel::High), thinking_budget: Some(2048), ..p.clone() },
        );
        assert_eq!(body["thinking"]["budget_tokens"], 2048, "配了就用配的那个数：{body}");

        let mut body2 = json!({});
        apply(&mut body2, &LlmParams { reasoning: Some(ReasoningLevel::High), ..p });
        assert_eq!(body2["thinking"]["budget_tokens"], 16384, "没配 ⇒ 按 high 的映射值：{body2}");
    }

    #[test]
    fn 未知方言降级成不发() {
        let p = params(&[(ReasoningLevel::High, Some("high"))], dialect::none());
        let mut body = json!({});
        apply(&mut body, &LlmParams { reasoning: Some(ReasoningLevel::High), ..p });
        assert!(body.get("reasoning_effort").is_none());
        assert!(body.get("thinking").is_none());
        assert!(body.get("enable_thinking").is_none());
    }
}
