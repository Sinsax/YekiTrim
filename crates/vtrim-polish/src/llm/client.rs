/// LLM API 客户端
///
/// 支持 OpenAI-compatible API (OpenAI, DeepSeek, Qwen, etc.)
pub struct LlmClient {
    api_key: String,
    base_url: String,
    model: String,
    client: reqwest::Client,
}

impl LlmClient {
    pub fn new(api_key: String, base_url: Option<String>, model: Option<String>) -> Self {
        Self {
            api_key,
            base_url: base_url.unwrap_or_else(|| "https://api.openai.com/v1".into()),
            model: model.unwrap_or_else(|| "gpt-4o-mini".into()),
            client: reqwest::Client::new(),
        }
    }

    /// 发送聊天补全请求
    pub async fn chat(&self, system_prompt: &str, user_prompt: &str) -> anyhow::Result<String> {
        #[derive(serde::Serialize)]
        struct Request {
            model: String,
            messages: Vec<Message>,
            temperature: f64,
            #[serde(skip_serializing_if = "Option::is_none")]
            thinking: Option<ThinkingConfig>,
        }

        #[derive(serde::Serialize)]
        struct ThinkingConfig {
            #[serde(rename = "type")]
            r#type: String,
        }

        #[derive(serde::Serialize)]
        struct Message {
            role: String,
            content: String,
        }

        #[derive(serde::Deserialize)]
        struct Response {
            choices: Vec<Choice>,
        }

        #[derive(serde::Deserialize)]
        struct Choice {
            message: MessageContent,
        }

        #[derive(serde::Deserialize)]
        struct MessageContent {
            content: String,
        }

        let req = Request {
            model: self.model.clone(),
            messages: vec![
                Message {
                    role: "system".into(),
                    content: system_prompt.to_string(),
                },
                Message {
                    role: "user".into(),
                    content: user_prompt.to_string(),
                },
            ],
            temperature: 0.3,
            thinking: Some(ThinkingConfig { r#type: "disabled".into() }),
        };

        let url = format!("{}/chat/completions", self.base_url);
        let resp = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&req)
            .send()
            .await?;

        let body: Response = resp.json().await?;
        Ok(body.choices.into_iter().next()
            .map(|c| c.message.content)
            .unwrap_or_default())
    }
}
