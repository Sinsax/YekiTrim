use std::time::Duration;

use anyhow::{Context, Result};
use serde_json::Value;

use crate::config::LlmConfig;

/// OpenAI 兼容 LLM 客户端
///
/// core（事件检测 / 转录订正）与 polish（精修段分割 / 关键词 / tags）共用。
/// 支持 OpenAI-compatible API (OpenAI, DeepSeek, Qwen, etc.)：
/// - `Authorization: Bearer` header
/// - `/chat/completions` 路径（自动补全）
/// - 429 / 5xx / 超时 / 连接错误自动重试（指数退避，最多 3 次）
///
/// 两条构造路径：
/// - [`LlmClient::from_config`] — 从 `LlmConfig`（vtrim.toml / .vtrim/config.toml）构造，
///   携带 temperature / max_tokens / extra_params（含 thinking 等透传参数）
/// - [`LlmClient::new`] — 从裸参数构造（polish 用），默认 temperature 0.3、不发送 max_tokens
pub struct LlmClient {
    api_key: String,
    base_url: String,
    model: String,
    temperature: f64,
    max_tokens: Option<u32>,
    extra_params: Option<Value>,
    thinking: Option<String>,
    http: reqwest::Client,
}

impl LlmClient {
    /// 从裸参数构造。base_url / model 为空时使用默认值（OpenAI / gpt-4o-mini）。
    pub fn new(api_key: &str, base_url: &str, model: &str) -> Self {
        Self {
            api_key: api_key.to_string(),
            base_url: if base_url.is_empty() {
                "https://api.openai.com/v1".into()
            } else {
                base_url.to_string()
            },
            model: if model.is_empty() {
                "gpt-4o-mini".into()
            } else {
                model.to_string()
            },
            temperature: 0.3,
            max_tokens: None,
            extra_params: None,
            thinking: None,
            http: reqwest::Client::builder()
                .timeout(Duration::from_secs(120))
                .build()
                .expect("创建 HTTP 客户端失败"),
        }
    }

    /// 从 `LlmConfig` 构造。`base_url` 为空时返回错误。
    pub fn from_config(llm: &LlmConfig) -> Result<Self> {
        if llm.base_url.is_empty() {
            anyhow::bail!("未配置 LLM API 基础地址 (llm.base_url)，请设置后重试");
        }
        let mut client = Self::new(&llm.api_key, &llm.base_url, &llm.model);
        client.temperature = llm.temperature;
        client.max_tokens = Some(llm.max_tokens);
        client.extra_params = llm
            .extra_params
            .as_ref()
            .and_then(|t| serde_json::to_value(t).ok());
        Ok(client)
    }

    pub fn with_temperature(mut self, t: f64) -> Self {
        self.temperature = t;
        self
    }

    pub fn with_max_tokens(mut self, n: u32) -> Self {
        self.max_tokens = Some(n);
        self
    }

    /// 设置请求超时（秒），重建底层 HTTP 客户端以应用新超时。
    pub fn with_timeout(mut self, secs: u64) -> Self {
        self.http = reqwest::Client::builder()
            .timeout(Duration::from_secs(secs))
            .build()
            .expect("创建 HTTP 客户端失败");
        self
    }

    /// 设置 thinking 参数（DeepSeek 等支持）。`Some("disabled")` 显式关闭深度思考。
    pub fn with_thinking(mut self, mode: Option<&str>) -> Self {
        self.thinking = mode.map(|m| m.to_string());
        self
    }

    fn api_url(&self) -> String {
        let base = self.base_url.trim_end_matches('/');
        if base.ends_with("/chat/completions") {
            base.to_string()
        } else {
            format!("{}/chat/completions", base)
        }
    }

    fn build_body(&self, messages: Value) -> Value {
        let mut body = serde_json::json!({
            "temperature": self.temperature,
            "messages": messages,
        });
        if !self.model.is_empty() {
            body["model"] = serde_json::json!(self.model);
        }
        if let Some(n) = self.max_tokens {
            body["max_tokens"] = serde_json::json!(n);
        }
        if let Some(Value::Object(extra_map)) = &self.extra_params {
            for (k, v) in extra_map {
                body[k] = v.clone();
            }
        }
        if let Some(ref mode) = self.thinking {
            body["thinking"] = serde_json::json!({"type": mode});
        }
        body
    }

    /// 发送聊天补全请求，返回完整响应 JSON（带重试）。
    pub async fn send(&self, messages: Value) -> Result<Value> {
        let url = self.api_url();
        let body = self.build_body(messages);

        let mut last_err = None;
        for attempt in 0..3 {
            let resp = self
                .http
                .post(&url)
                .header("Authorization", format!("Bearer {}", self.api_key))
                .json(&body)
                .send()
                .await;

            match resp {
                Ok(r) if r.status().is_success() => {
                    return r.json().await.context("解析 LLM API 响应失败");
                }
                Ok(r) => {
                    let status = r.status();
                    let text = r.text().await.unwrap_or_default();
                    let retryable = status.as_u16() == 429 || status.as_u16() >= 500;
                    if retryable && attempt < 2 {
                        let delay = Duration::from_secs(2u64.pow(attempt as u32 + 1));
                        tracing::warn!("LLM API 返回 {}，{}s 后重试 ({}/3)", status, delay.as_secs(), attempt + 2);
                        tokio::time::sleep(delay).await;
                        last_err = Some(anyhow::anyhow!("LLM API 返回 {}: {}", status, text));
                        continue;
                    }
                    anyhow::bail!("LLM API 返回 {}: {}", status, text);
                }
                Err(e) => {
                    let retryable = e.is_timeout() || e.is_connect();
                    if retryable && attempt < 2 {
                        let delay = Duration::from_secs(2u64.pow(attempt as u32 + 1));
                        tracing::warn!("LLM API 请求失败: {}，{}s 后重试 ({}/3)", e, delay.as_secs(), attempt + 2);
                        tokio::time::sleep(delay).await;
                        last_err = Some(anyhow::anyhow!("LLM API 请求失败: {}", e));
                        continue;
                    }
                    anyhow::bail!("LLM API 请求失败: {}", e);
                }
            }
        }

        Err(last_err.unwrap_or_else(|| anyhow::anyhow!("LLM API 请求失败")))
    }

    /// system + user 两消息发送，返回 `choices[0].message.content` 文本。
    pub async fn chat(&self, system_prompt: &str, user_prompt: &str) -> Result<String> {
        let messages = serde_json::json!([
            {"role": "system", "content": system_prompt},
            {"role": "user", "content": user_prompt},
        ]);
        let body = self.send(messages).await?;
        body["choices"][0]["message"]["content"]
            .as_str()
            .map(|s| s.to_string())
            .ok_or_else(|| anyhow::anyhow!("LLM 响应缺少 choices[0].message.content"))
    }
}
