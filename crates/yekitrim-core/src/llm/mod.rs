//! llm：统一 LLM 客户端（OpenAI 兼容）与用量记账出口。
//!
//! 入口：`LlmClient`（`new` / `from_config` / `chat` / `send` / `with_usage_context`）；子模块 `agent`（批量窗口编排）、`usage`（用量记录与 sink）。
//! 边界：core 只 `emit` 用量记录，写库由 CLI / server 侧装的 sink 负责；各链路别自己写 `reqwest` —— 见 `crates/vtrim-core/src/llm/AGENTS.md`

use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Result;
use serde_json::Value;

use crate::config::LlmConfig;

pub mod agent;
pub mod usage;

use usage::{UsageRecord, UsageSink};

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
/// 浏览器 UA：部分网关/反爬会按 UA 放行，缺了容易被拒。
const BROWSER_UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0 Safari/537.36";

/// 建连超时：端点常经代理（Clash TUN 等）转发，代理侧偶发不响应。
/// 不设的话一次卡住的握手会吃满整个总超时（检测路径 180s），而不是早点失败进重试。
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// 空闲连接在多长时间后不再复用。代理会回收它那一侧的会话，
/// 我们留得比它短，就不会去复用一条已经死掉的连接（表现为
/// `error sending request` → 重试换新连接才成功）。
const POOL_IDLE_TIMEOUT: Duration = Duration::from_secs(30);

/// TCP keepalive 间隔：让内核去探活，别让空闲连接静默死掉。
const TCP_KEEPALIVE: Duration = Duration::from_secs(30);

/// 统一的 HTTP 客户端构造。**所有选项都只在这里设**——
/// `with_timeout` 会重建客户端，散着写就会像 UA 那样被静默丢掉。
fn build_http_client(total_timeout: Duration) -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(total_timeout)
        .connect_timeout(CONNECT_TIMEOUT)
        .pool_idle_timeout(POOL_IDLE_TIMEOUT)
        .tcp_keepalive(TCP_KEEPALIVE)
        .user_agent(BROWSER_UA)
        .build()
        .expect("创建 HTTP 客户端失败")
}

pub struct LlmClient {
    api_key: String,
    base_url: String,
    model: String,
    temperature: f64,
    max_tokens: Option<u32>,
    extra_params: Option<Value>,
    thinking: Option<String>,
    http: reqwest::Client,
    /// 用量归因（谁在调用）。不给就记成空 label（仍会记账，只是分不出链路）。
    usage_context: usage::UsageContext,
    /// 私有 sink；不给就用进程级默认 sink（CLI/server 启动时装的那个）。
    usage_sink: Option<Arc<dyn UsageSink>>,
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
            http: build_http_client(Duration::from_secs(120)),
            usage_context: usage::UsageContext::default(),
            usage_sink: None,
        }
    }

    /// 给用量记账挂上归因（链路名 + 项目目录）。**在 `send()` 之前调**，否则记成空 label。
    pub fn with_usage_context(mut self, ctx: usage::UsageContext) -> Self {
        self.usage_context = ctx;
        self
    }

    /// 注入私有 sink（测试用；生产走 `usage::set_default_sink`）。
    pub fn with_usage_sink(mut self, sink: Arc<dyn UsageSink>) -> Self {
        self.usage_sink = Some(sink);
        self
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
    ///
    /// 重建走 [`build_http_client`]，所以 UA 等选项不会在这一步丢掉
    /// （曾经这里只设 timeout，把 `new()` 里的浏览器 UA 静默丢了——而检测与
    /// 精修事件生成两条最忙的路径都会调它）。
    pub fn with_timeout(mut self, secs: u64) -> Self {
        self.http = build_http_client(Duration::from_secs(secs));
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
    ///
    /// **这里是全链路唯一的用量记账点**：不管成功还是最终失败，退出前都会 `emit` 一条
    /// `UsageRecord`（重试算同一次调用，`attempts` 记实际尝试次数）。
    /// 记账失败不影响返回值——记录仪里的错误由安装方自己吞。
    pub async fn send(&self, messages: Value) -> Result<Value> {
        let url = self.api_url();
        let body = self.build_body(messages);
        let started = Instant::now();

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
                    // 先把 body 解成 JSON：记账要读 usage，不能像从前那样直接把响应流交给调用方
                    let value: Value = match r.json().await {
                        Ok(v) => v,
                        Err(e) => {
                            // 200 但不是 JSON（网关插了一页 HTML 之类）：账上也要留一行，
                            // 否则"调用发生了却查无此事"，只能靠日志猜
                            let msg = format!("解析 LLM API 响应失败: {e}");
                            self.emit_usage(usage::UsageNumbers::default(), started, attempt + 1, false, None, Some(&msg));
                            return Err(anyhow::anyhow!(msg));
                        }
                    };
                    let nums = usage::parse_usage(&value);
                    self.emit_usage(nums, started, attempt + 1, true, None, None);
                    return Ok(value);
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
                    self.emit_usage(
                        usage::UsageNumbers::default(),
                        started,
                        attempt + 1,
                        false,
                        Some(status.as_u16()),
                        Some(&text),
                    );
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
                    self.emit_usage(
                        usage::UsageNumbers::default(),
                        started,
                        attempt + 1,
                        false,
                        None,
                        Some(&e.to_string()),
                    );
                    anyhow::bail!("LLM API 请求失败: {}", e);
                }
            }
        }

        Err(last_err.unwrap_or_else(|| anyhow::anyhow!("LLM API 请求失败")))
    }

    /// 组装并投递一条用量记录（没有 sink 时只留日志）。
    fn emit_usage(
        &self,
        nums: usage::UsageNumbers,
        started: Instant,
        attempts: u32,
        ok: bool,
        status: Option<u16>,
        error: Option<&str>,
    ) {
        usage::emit_with(
            self.usage_sink.as_ref(),
            UsageRecord {
                ts: usage::now_secs(),
                label: self.usage_context.label.clone(),
                project: self.usage_context.project.clone(),
                model: self.model.clone(),
                host: usage::host_of(&self.base_url),
                prompt_tokens: nums.prompt_tokens,
                completion_tokens: nums.completion_tokens,
                cached_tokens: nums.cached_tokens,
                reasoning_tokens: nums.reasoning_tokens,
                latency_ms: started.elapsed().as_millis() as u64,
                attempts,
                ok,
                status,
                error: error.map(usage::truncate_error),
            },
        );
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    struct Recorder(Mutex<Vec<UsageRecord>>);

    impl UsageSink for Recorder {
        fn record(&self, u: &UsageRecord) {
            self.0.lock().unwrap().push(u.clone());
        }
    }

    /// 极简假 OpenAI 服务：第 n 个请求回 `script[n]`（用完后不再受理）。
    /// 只为验证"发出去 → 记账"，不解析请求体。
    async fn fake_server(script: Vec<(u16, String)>) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            for (code, body) in script {
                let Ok((mut sock, _)) = listener.accept().await else { break };
                let mut buf = [0u8; 8192];
                let _ = sock.read(&mut buf).await;
                let reason = if code == 200 { "OK" } else { "ERR" };
                let resp = format!(
                    "HTTP/1.1 {code} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.as_bytes().len()
                );
                let _ = sock.write_all(resp.as_bytes()).await;
                let _ = sock.shutdown().await;
            }
        });
        format!("http://{addr}/v1")
    }

    fn recorder() -> Arc<Recorder> {
        Arc::new(Recorder(Mutex::new(Vec::new())))
    }

    /// 成功调用：记一行，带链路归因、输入/输出/缓存命中 token，attempts = 1。
    #[tokio::test]
    async fn send_records_usage_on_success() {
        let body = serde_json::json!({
            "choices": [{"message": {"content": "hi"}}],
            "usage": {"prompt_tokens": 100, "completion_tokens": 20, "prompt_cache_hit_tokens": 64}
        })
        .to_string();
        let base = fake_server(vec![(200, body)]).await;
        let rec = recorder();
        let client = LlmClient::new("k", &base, "m1")
            .with_usage_sink(rec.clone())
            .with_usage_context(usage::UsageContext::new("测试链路").project(Some("F:/proj")));

        assert_eq!(client.chat("sys", "user").await.unwrap(), "hi");

        let rows = rec.0.lock().unwrap();
        assert_eq!(rows.len(), 1, "一次调用只记一行");
        let r = &rows[0];
        assert_eq!(r.label, "测试链路");
        assert_eq!(r.project.as_deref(), Some("F:/proj"));
        assert_eq!(r.model, "m1");
        assert!(r.host.starts_with("127.0.0.1"), "host 应取 base_url 主机名: {}", r.host);
        assert_eq!((r.prompt_tokens, r.completion_tokens, r.cached_tokens), (100, 20, 64));
        assert_eq!(r.attempts, 1);
        assert!(r.ok && r.status.is_none() && r.error.is_none());
    }

    /// 硬失败（400 不重试）：也记一行，带状态码与错误串。
    #[tokio::test]
    async fn send_records_usage_on_hard_failure() {
        let base = fake_server(vec![(400, r#"{"error":"bad key"}"#.to_string())]).await;
        let rec = recorder();
        let client = LlmClient::new("k", &base, "m1").with_usage_sink(rec.clone());

        assert!(client.chat("sys", "user").await.is_err());

        let rows = rec.0.lock().unwrap();
        assert_eq!(rows.len(), 1);
        let r = &rows[0];
        assert!(!r.ok);
        assert_eq!(r.status, Some(400));
        assert_eq!(r.attempts, 1);
        assert!(r.error.as_deref().unwrap_or("").contains("bad key"), "错误串应带上响应体");
    }

    /// 429/5xx 重试后成功：仍是一行，`attempts` 记实际尝试次数。
    /// （重试退避 2s，这条测试会慢 2 秒。）
    #[tokio::test]
    async fn send_retries_then_records_attempts() {
        let ok_body = serde_json::json!({
            "choices": [{"message": {"content": "ok"}}],
            "usage": {"prompt_tokens": 5, "completion_tokens": 1}
        })
        .to_string();
        let base = fake_server(vec![(500, "{}".to_string()), (200, ok_body)]).await;
        let rec = recorder();
        let client = LlmClient::new("k", &base, "m1").with_usage_sink(rec.clone());

        assert_eq!(client.chat("sys", "user").await.unwrap(), "ok");

        let rows = rec.0.lock().unwrap();
        assert_eq!(rows.len(), 1, "重试不另起一行");
        assert_eq!(rows[0].attempts, 2);
        assert!(rows[0].ok);
        assert_eq!(rows[0].prompt_tokens, 5);
    }
}
