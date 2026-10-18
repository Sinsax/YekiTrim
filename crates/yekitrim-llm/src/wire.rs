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

use crate::params::{LlmParams, MaxOutputWire, ReasoningLevel, Support, ThinkingWire};
use serde_json::{json, Value};

/// 把参数写进请求体。`body` 里已经有 `messages`。
pub fn apply(body: &mut Value, p: &LlmParams) {
    if !p.model.is_empty() {
        body["model"] = json!(p.model);
    }
    // 输出上限：字段名按模型能力（o 系/gpt-5 只认 `max_completion_tokens`）；`None` = 不发送
    if let Some(n) = p.max_tokens {
        match p.caps.max_output_wire {
            MaxOutputWire::MaxTokens => body["max_tokens"] = json!(n),
            MaxOutputWire::MaxCompletionTokens => body["max_completion_tokens"] = json!(n),
        }
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
fn apply_reasoning(body: &mut Value, p: &LlmParams) {
    if !p.caps.reasoning_supported() {
        // 表里没有这支模型（自建网关/新模型）：只在**显式表过态**时用最保守的老形态，
        // 免得把老工程的 `[llm] thinking` 一升上来就静默失效。
        if p.reasoning_explicit {
            let kind = if p.reasoning == ReasoningLevel::Off { "disabled" } else { "enabled" };
            body["thinking"] = json!({ "type": kind });
        }
        return;
    }
    let Some(slot) = p.caps.wire_for(p.reasoning) else {
        return; // 该档不支持 ⇒ 不发（不是发个错的）
    };
    match slot {
        // 支持但"不发送"（多数 provider 的"不思考"就是参数缺席）
        None => {}
        Some(value) => emit(body, p.caps.thinking_wire, p.reasoning, value),
    }
}

/// 按方言把一档写成具体的键。未知方言（[`ThinkingWire::None`]）什么都不做。
fn emit(body: &mut Value, wire: ThinkingWire, level: ReasoningLevel, value: &str) {
    match wire {
        ThinkingWire::OpenAIEffort => body["reasoning_effort"] = json!(value),
        ThinkingWire::ThinkingType => {
            let kind = if level == ReasoningLevel::Off { "disabled" } else { "enabled" };
            body["thinking"] = json!({ "type": kind });
        }
        ThinkingWire::QwenEnableThinking => {
            body["enable_thinking"] = json!(level != ReasoningLevel::Off);
        }
        ThinkingWire::OpenRouterEffort => body["reasoning"] = json!({ "effort": value }),
        ThinkingWire::None => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::params::{ModelCaps, ReasoningLevel, Support, ThinkingWire};

    fn params(levels: &[(ReasoningLevel, Option<&str>)], wire: ThinkingWire) -> LlmParams {
        let mut caps = ModelCaps::default();
        caps.thinking_wire = wire;
        for (lv, v) in levels {
            caps.levels.insert(*lv, v.map(|s| s.to_string()));
        }
        LlmParams { caps, ..Default::default() }
    }

    #[test]
    fn 表里没有的档位一个都不发() {
        let p = params(&[(ReasoningLevel::High, Some("high"))], ThinkingWire::OpenAIEffort);
        let mut body = json!({});
        apply(&mut body, &LlmParams { reasoning: ReasoningLevel::Low, ..p });
        assert!(body.get("reasoning_effort").is_none(), "Low 不在表里就不许发：{body}");
    }

    #[test]
    fn null_档位表示支持但不发送() {
        let p = params(&[(ReasoningLevel::Off, None)], ThinkingWire::ThinkingType);
        let mut body = json!({});
        apply(&mut body, &LlmParams { reasoning: ReasoningLevel::Off, ..p });
        assert!(body.get("thinking").is_none(), "off 的 wire 是 null（缺席）⇒ 不许写键：{body}");
    }

    #[test]
    fn 温度不支持的模型不发_temperature() {
        let mut p = params(&[], ThinkingWire::None);
        p.caps.temperature = Support::Unsupported;
        let mut body = json!({});
        apply(&mut body, &p);
        assert!(body.get("temperature").is_none(), "温度不支持就别发：{body}");
    }

    #[test]
    fn max_output_按能力换字段名() {
        let mut p = params(&[], ThinkingWire::None);
        p.caps.max_output_wire = MaxOutputWire::MaxCompletionTokens;
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
            ..params(&[(ReasoningLevel::High, Some("high"))], ThinkingWire::ThinkingType)
        };
        let mut body = json!({});
        apply(&mut body, &LlmParams { reasoning: ReasoningLevel::High, ..p });
        assert_eq!(body["temperature"], 1.5, "extra 要盖掉结构化的温度（老行为）");
        assert_eq!(body["thinking"]["type"], "enabled", "reasoning 再盖掉 extra（老行为）");
    }

    #[test]
    fn 未知方言降级成不发() {
        let p = params(&[(ReasoningLevel::High, Some("high"))], ThinkingWire::None);
        let mut body = json!({});
        apply(&mut body, &LlmParams { reasoning: ReasoningLevel::High, ..p });
        assert!(body.get("reasoning_effort").is_none());
        assert!(body.get("thinking").is_none());
        assert!(body.get("enable_thinking").is_none());
    }
}
