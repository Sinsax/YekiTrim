//! **请求体 / 回包映射**（每家 API 标准一份）。
//!
//! 与 [`crate::protocol`]（发到哪、怎么鉴权）分工：这里管「消息数组 ↔ 各家请求体」与
//! 「各家回包 ↔ 纯文本」。**这是 wire 字面量的合法居所**（`only_llm_knows_wire` 门禁：
//! 这些形状不许散到业务 crate）。
//!
//! ⚠️ 推理参数（`reasoning`）当前**只在 OpenAI 兼容族发送**（`wire::apply`）：
//! Anthropic 的 thinking（预算型）、Gemini 的 thinkingConfig 是另一套形状，接之前
//! 宁可不发（模型照常工作，只是没有扩展思考）—— 发错形状会被 400。

use serde_json::{json, Value};

use crate::params::LlmParams;
use crate::protocol::Protocol;

/// 一个请求需要的东西：URL 路径（已替换 `{model}`）、鉴权与请求体。
pub struct Prepared {
    pub path: String,
    /// `Some(query)` = 把 key 拼成 `?key=<key>`（Gemini）；`None` = 用鉴权头。
    pub key_query: bool,
    pub body: Value,
}

/// 按协议把「消息数组 + 参数」拼成请求。**openai 族走 wire::apply**（方言全在那里）。
pub fn encode(params: &LlmParams, proto: &Protocol, messages: &Value) -> Prepared {
    let path = proto.chat_path.replace("{model}", &params.model);
    match proto.id {
        "gemini-native" => Prepared { path, key_query: true, body: encode_gemini(params, messages) },
        "openai-responses" => Prepared { path, key_query: false, body: encode_responses(params, messages) },
        // openai-chat 与一切未知的都按 OpenAI 兼容走（unknown 在 client 入口已报错，到不了这里）
        _ => {
            let mut body = json!({ "messages": messages, "model": params.model });
            crate::wire::apply(&mut body, params);
            Prepared { path, key_query: false, body }
        }
    }
}

/// 从回包里抽**纯文本**。认不出形状 ⇒ `None`（调用方报「响应缺少文本」）。
pub fn extract_text(proto_id: &str, body: &Value) -> Option<String> {
    match proto_id {
        // Gemini：`candidates[0].content.parts[*].text`
        "gemini-native" => {
            let parts = body.get("candidates")?.get(0)?.get("content")?.get("parts")?.as_array()?;
            let mut out = String::new();
            for p in parts {
                if let Some(s) = p.get("text").and_then(|t| t.as_str()) {
                    out.push_str(s);
                }
            }
            Some(out).filter(|s| !s.is_empty())
        }
        // Responses：`output[*]（type=message）.content[*]（type=output_text）.text`
        "openai-responses" => {
            let arr = body.get("output")?.as_array()?;
            let mut out = String::new();
            for item in arr {
                if item.get("type").and_then(|t| t.as_str()) != Some("message") {
                    continue;
                }
                if let Some(parts) = item.get("content").and_then(|c| c.as_array()) {
                    for p in parts {
                        if p.get("type").and_then(|t| t.as_str()) == Some("output_text") {
                            if let Some(s) = p.get("text").and_then(|t| t.as_str()) {
                                out.push_str(s);
                            }
                        }
                    }
                }
            }
            Some(out).filter(|s| !s.is_empty())
        }
        // OpenAI 兼容：`choices[0].message.content`
        _ => body.get("choices")?.get(0)?.get("message")?.get("content")?.as_str().map(|s| s.to_string()).filter(|s| !s.is_empty()),
    }
}

fn encode_gemini(p: &LlmParams, messages: &Value) -> Value {
    // role 映射：assistant→model；system 单独走 systemInstruction
    let mut system = String::new();
    let mut contents = Vec::new();
    if let Some(arr) = messages.as_array() {
        for m in arr {
            let role = m.get("role").and_then(|r| r.as_str()).unwrap_or("user");
            let text = m.get("content").and_then(|c| c.as_str()).unwrap_or_default();
            match role {
                "system" => {
                    if !system.is_empty() {
                        system.push('\n');
                    }
                    system.push_str(text);
                }
                "assistant" => contents.push(json!({ "role": "model", "parts": [{ "text": text }] })),
                _ => contents.push(json!({ "role": "user", "parts": [{ "text": text }] })),
            }
        }
    }
    let mut body = json!({ "contents": contents });
    if !system.is_empty() {
        body["systemInstruction"] = json!({ "parts": [{ "text": system }] });
    }
    let mut gen_cfg = json!({});
    if p.temperature.is_finite() && p.temperature != 1.0 {
        gen_cfg["temperature"] = json!(p.temperature);
    }
    if let Some(n) = p.max_tokens {
        gen_cfg["maxOutputTokens"] = json!(n);
    }
    if !gen_cfg.as_object().map(|o| o.is_empty()).unwrap_or(true) {
        body["generationConfig"] = gen_cfg;
    }
    body
}

fn encode_responses(p: &LlmParams, messages: &Value) -> Value {
    // system → 顶层 instructions；input = 消息数组（content 要带 type）
    let mut instructions = String::new();
    let mut input = Vec::new();
    if let Some(arr) = messages.as_array() {
        for m in arr {
            let role = m.get("role").and_then(|r| r.as_str()).unwrap_or("user");
            let text = m.get("content").and_then(|c| c.as_str()).unwrap_or_default();
            if role == "system" {
                if !instructions.is_empty() {
                    instructions.push('\n');
                }
                instructions.push_str(text);
            } else {
                let ty = if role == "assistant" { "output_text" } else { "input_text" };
                input.push(json!({ "role": role, "content": [{ "type": ty, "text": text }] }));
            }
        }
    }
    let mut body = json!({ "model": p.model, "input": input });
    if !instructions.is_empty() {
        body["instructions"] = json!(instructions);
    }
    if p.temperature.is_finite() && p.temperature != 1.0 {
        body["temperature"] = json!(p.temperature);
    }
    if let Some(n) = p.max_tokens {
        body["max_output_tokens"] = json!(n);
    }
    body
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::protocol;

    fn params(model: &str) -> LlmParams {
        LlmParams { model: model.to_string(), max_tokens: Some(1234), ..Default::default() }
    }

    fn msgs() -> Value {
        json!([{ "role": "system", "content": "sys" }, { "role": "user", "content": "hi" }])
    }

    #[test]
    fn gemini_maps_roles_and_path_template() {
        let p = protocol("gemini-native").unwrap();
        let prep = encode(&params("gemini-2.0-flash"), p, &msgs());
        assert_eq!(prep.path, "/models/gemini-2.0-flash:generateContent");
        assert!(prep.key_query);
        assert_eq!(prep.body["systemInstruction"]["parts"][0]["text"], "sys");
        assert_eq!(prep.body["contents"][0]["role"], "user");
        assert_eq!(prep.body["generationConfig"]["maxOutputTokens"], 1234);
    }

    #[test]
    fn responses_split_instructions_and_typed_content() {
        let p = protocol("openai-responses").unwrap();
        let b = encode(&params("gpt-x"), p, &msgs()).body;
        assert_eq!(b["instructions"], "sys");
        assert_eq!(b["input"][0]["content"][0]["type"], "input_text");
        assert_eq!(b["max_output_tokens"], 1234);
    }

    #[test]
    fn extract_text_per_family() {
        let gem = json!({ "candidates": [ { "content": { "parts": [ {"text":"hi"} ] } } ] });
        assert_eq!(extract_text("gemini-native", &gem).as_deref(), Some("hi"));
        let rsp = json!({ "output": [ {"type":"message","content":[{"type":"output_text","text":"ok"}]} ] });
        assert_eq!(extract_text("openai-responses", &rsp).as_deref(), Some("ok"));
        let oai = json!({ "choices": [ { "message": { "content": "c" } } ] });
        assert_eq!(extract_text("openai-chat", &oai).as_deref(), Some("c"));
    }
}
