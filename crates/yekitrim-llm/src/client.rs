//! llm：统一 LLM 客户端（OpenAI 兼容）与用量记账出口。
//!
//! 入口：`LlmClient`（`new` / `from_config` / `chat` / `send` / `with_usage_context`）；子模块 `agent`（批量窗口编排）、`usage`（用量记录与 sink）。
//! 边界：core 只 `emit` 用量记录，写库由 CLI / server 侧装的 sink 负责；各链路别自己写 `reqwest` —— 见 `crates/yekitrim-core/src/llm/AGENTS.md`

use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Result;
use serde_json::Value;

use crate::params::LlmParams;
use crate::wire;


use crate::usage::{UsageRecord, UsageSink};

/// OpenAI 兼容 LLM 客户端
///
/// core（事件检测 / 转录订正）与 polish（精修段分割 / 关键词 / tags）共用。
/// 支持 OpenAI-compatible API (OpenAI, DeepSeek, Qwen, etc.)：
/// - `Authorization: Bearer` header
/// - `/chat/completions` 路径（自动补全）
/// - 429 / 5xx / 超时 / 连接错误自动重试（指数退避，最多 3 次）
///
/// 唯一的构造入口是 [`LlmClient::new`]：它吃任何 `Into<LlmParams>`。
///
/// 配置层（`yekitrim-core::config`）从 `LinkedLlm` 转出 `LlmParams`，所以业务侧必须先
/// `config.for_link(link)` 才拿得到能建客户端的值（S1：忘记套链路参数 = 编译错误）。
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
pub(crate) fn build_http_client(total_timeout: Duration) -> reqwest::Client {
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
    /// 一次调用的全部参数。**本 crate 不认识配置文件** —— 配置到参数的那一步在
    /// `yekitrim-core::config`（`impl From<&LlmConfig>`）。
    params: LlmParams,
    http: reqwest::Client,
    /// 用量归因（谁在调用）。不给就记成空 label（仍会记账，只是分不出链路）。
    usage_context: crate::usage::UsageContext,
    /// 私有 sink；不给就用进程级默认 sink（CLI/server 启动时装的那个）。
    usage_sink: Option<Arc<dyn UsageSink>>,
}

impl LlmClient {
    /// 从裸参数构造。base_url / model 为空时使用默认值（OpenAI / gpt-4o-mini）。
    /// 从裸参数构造（测试/探针用；生产走 [`LlmClient::new`]）。
    pub fn raw(api_key: &str, base_url: &str, model: &str) -> Self {
        Self {
            params: LlmParams {
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
                ..Default::default()
            },
            http: build_http_client(Duration::from_secs(120)),
            usage_context: crate::usage::UsageContext::default(),
            usage_sink: None,
        }
    }

    /// 给用量记账挂上归因（链路名 + 项目目录）。**在 `send()` 之前调**，否则记成空 label。
    pub fn with_usage_context(mut self, ctx: crate::usage::UsageContext) -> Self {
        self.usage_context = ctx;
        self
    }

    /// 注入私有 sink（测试用；生产走 `crate::usage::set_default_sink`）。
    pub fn with_usage_sink(mut self, sink: Arc<dyn UsageSink>) -> Self {
        self.usage_sink = Some(sink);
        self
    }

    /// 从**归一化参数**构造。`base_url` 为空时返回错误。
    ///
    /// 接受任何 `Into<LlmParams>`：实际传进来的是配置层解析出来的参数（`impl From<&LlmConfig>`
    /// 在 `yekitrim-core::config` 里）—— **本 crate 不认识那个类型**。
    pub fn new<P: Into<LlmParams>>(p: P) -> Result<Self> {
        let p = p.into();
        if p.base_url.is_empty() {
            anyhow::bail!("未配置 LLM API 基础地址 (llm.base_url)，请设置后重试");
        }
        // ⚠️ 这里**不**设 thinking：旧 from_config 也没设，thinking 由调用方按链路
        //    with_thinking(...) 显式给（搬迁期保持逐字节相同）。
        let mut client = Self::raw(&p.api_key, &p.base_url, &p.model);
        client.params = p;
        Ok(client)
    }

    pub fn with_temperature(mut self, t: f64) -> Self {
        self.params.temperature = t;
        self
    }

    pub fn with_max_tokens(mut self, n: u32) -> Self {
        self.params.max_tokens = Some(n);
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

    fn api_url(&self) -> String {
        let base = self.params.base_url.trim_end_matches('/');
        if base.ends_with("/chat/completions") {
            base.to_string()
        } else {
            format!("{}/chat/completions", base)
        }
    }

    fn build_body(&self, messages: Value) -> Value {
        // 请求体**只有这一处拼装**：全部方言都在 `wire::apply` 里（见那个模块的注释）。
        let mut body = serde_json::json!({ "messages": messages });
        wire::apply(&mut body, &self.params);
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
                .header("Authorization", format!("Bearer {}", self.params.api_key))
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
                            self.emit_usage(crate::usage::UsageNumbers::default(), started, attempt + 1, false, None, Some(&msg));
                            return Err(anyhow::anyhow!(msg));
                        }
                    };
                    let nums = crate::usage::parse_usage(&value);
                    self.emit_usage(nums, started, attempt + 1, true, None, None);
                    return Ok(value);
                }
                Ok(r) => {
                    let status = r.status();
                    let text = r.text().await.unwrap_or_default();
                    let retryable = status.as_u16() == 429 || status.as_u16() >= 500;
                    if retryable && attempt < 2 {
                        let delay = Duration::from_secs(2u64.pow(attempt + 1));
                        tracing::warn!("LLM API 返回 {}，{}s 后重试 ({}/3)", status, delay.as_secs(), attempt + 2);
                        tokio::time::sleep(delay).await;
                        last_err = Some(anyhow::anyhow!("LLM API 返回 {}: {}", status, text));
                        continue;
                    }
                    self.emit_usage(
                        crate::usage::UsageNumbers::default(),
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
                        let delay = Duration::from_secs(2u64.pow(attempt + 1));
                        tracing::warn!("LLM API 请求失败: {}，{}s 后重试 ({}/3)", e, delay.as_secs(), attempt + 2);
                        tokio::time::sleep(delay).await;
                        last_err = Some(anyhow::anyhow!("LLM API 请求失败: {}", e));
                        continue;
                    }
                    self.emit_usage(
                        crate::usage::UsageNumbers::default(),
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
        nums: crate::usage::UsageNumbers,
        started: Instant,
        attempts: u32,
        ok: bool,
        status: Option<u16>,
        error: Option<&str>,
    ) {
        crate::usage::emit_with(
            self.usage_sink.as_ref(),
            UsageRecord {
                ts: crate::usage::now_secs(),
                label: self.usage_context.label.clone(),
                project: self.usage_context.project.clone(),
                model: self.params.model.clone(),
                host: crate::usage::host_of(&self.params.base_url),
                prompt_tokens: nums.prompt_tokens,
                completion_tokens: nums.completion_tokens,
                cached_tokens: nums.cached_tokens,
                reasoning_tokens: nums.reasoning_tokens,
                latency_ms: started.elapsed().as_millis() as u64,
                attempts,
                ok,
                status,
                error: error.map(crate::usage::truncate_error),
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

    /// 极简假 OpenAI 服务：第 n 个请求回 `script[n]`（用完后回 500，见下）。
    /// 只为验证"发出去 → 记账"，不解析请求体。
    ///
    /// ⚠️ **两个坑，都会让这条链上的用例偶发红**（`cargo test --workspace` 并行下约 1/7，
    /// 实测 14 轮 2 红）。两个都要防，只防一个不行：
    ///
    /// ① **serve 循环不能在 script 用完后直接 `break`**。退出后 listener 仍在 listen
    ///    （调用方拿 `Arc` 持着），内核会把新连接放进 accept 队列、**但没人 accept** ⇒
    ///    客户端**连得上、永远收不到响应**。而 `LlmClient` 对 `is_connect() || is_timeout()`
    ///    **会重试**（`attempt < 2`）⇒ 每次尝试各挂一个超时，症状是
    ///    `send_records_usage_on_success` **卡 60+ 秒**才报 `error sending request`。
    ///    ⚠️ 那**不是** 502 —— 502 才是"连到了别人的服务"。看到 `sending request`
    ///    就该往"连上了但没人应答"这条线上想。
    ///    ⇒ **script 耗尽后继续 accept，回一个明确的 500**，让重试立刻收敛。
    ///
    /// ② **listener 不能被回收**：只把 port 拿出来、把 listener 丢进 `spawn` 的写法，
    ///    在重试退避的 2s 窗口里端口可能被**同期另一个测试**的 listener 拿去。
    ///    ⇒ 把 `Arc<TcpListener>` 还给调用方，由它持到测试结束。
    ///
    /// ⚠️ **新增用它的用例时，让 script 长度 ≥ 客户端最大尝试次数（3）**：
    /// 否则第 3 次尝试会吃到 ①补的那个 500，用例可能"绿得很偶然"。
    /// 只验证记账的用例给 3 个同样的响应最省心。
    async fn fake_server(script: Vec<(u16, String)>) -> (Arc<TcpListener>, String) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        // ⚠️ `tokio::net::TcpListener` 没有 `try_clone`（那个在 std 上）⇒ 用 `Arc` 共享，
        // 两边拿同一个 fd，端口在测试全程被占住（见上面 ②）。
        let listener = std::sync::Arc::new(listener);
        let serve = listener.clone();
        tokio::spawn(async move {
            let mut script = script.into_iter();
            loop {
                let Ok((mut sock, _)) = serve.accept().await else { break };
                // script 用完后**继续接受连接**、回一个明确的 500（见上面 ①）：
                // 直接 `break` 会让"连得上但没人应答"变成超时挂死。
                let (code, body) = script
                    .next()
                    .unwrap_or((500, r#"{"error":"fake_server script exhausted"}"#.to_string()));
                let mut buf = [0u8; 8192];
                let _ = sock.read(&mut buf).await;
                let reason = if code == 200 { "OK" } else { "ERR" };
                let resp = format!(
                    "HTTP/1.1 {code} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = sock.write_all(resp.as_bytes()).await;
                let _ = sock.shutdown().await;
            }
        });
        (listener, format!("http://{addr}/v1"))
    }

    fn recorder() -> Arc<Recorder> {
        Arc::new(Recorder(Mutex::new(Vec::new())))
    }

    /// 成功调用：记一行，带链路归因、输入/输出/缓存命中 token；attempts 记实际尝试次数
    /// （正常 1，但连接层偶发重试后成功会是 2）。
    #[tokio::test]
    async fn send_records_usage_on_success() {
        let body = serde_json::json!({
            "choices": [{"message": {"content": "hi"}}],
            "usage": {"prompt_tokens": 100, "completion_tokens": 20, "prompt_cache_hit_tokens": 64}
        })
        .to_string();
        // listener 活到测试结束 + script 备足 3 份（= 客户端最大尝试次数）：
        // 这两条缺一都会偶发红，见 fake_server 的 ⚠️
        let (_srv, base) = fake_server(vec![(200, body.clone()), (200, body.clone()), (200, body)]).await;
        let rec = recorder();
        let client = LlmClient::raw("k", &base, "m1")
            .with_usage_sink(rec.clone())
            .with_usage_context(crate::usage::UsageContext::new(crate::tasks::CONNECTION_TEST.label).project(Some("F:/proj")));

        assert_eq!(client.chat("sys", "user").await.unwrap(), "hi");

        let rows = rec.0.lock().unwrap();
        assert_eq!(rows.len(), 1, "一次调用只记一行");
        let r = &rows[0];
        assert_eq!(r.label, "测试链路");
        assert_eq!(r.project.as_deref(), Some("F:/proj"));
        assert_eq!(r.model, "m1");
        assert!(r.host.starts_with("127.0.0.1"), "host 应取 base_url 主机名: {}", r.host);
        assert_eq!((r.prompt_tokens, r.completion_tokens, r.cached_tokens), (100, 20, 64));
        // 同上：不断言 `== 1`。并行全量时偶发首连失败会走 `is_connect()` 重试分支，
        // 重试成功后 attempts 合法地是 2（本用例的 script 备了 3 份正是为此）。
        assert!(
            (1..=3).contains(&r.attempts),
            "attempts 应在 1..=3，实际 {}",
            r.attempts
        );
        assert!(r.ok && r.status.is_none() && r.error.is_none());
    }

    /// 硬失败（400 不重试）：也记一行，带状态码与错误串。
    #[tokio::test]
    async fn send_records_usage_on_hard_failure() {
        // 同上：备足 3 份（客户端最大尝试次数）。400 本身不可重试，
        // 但**给足 3 份**能让"万一真重试了"也拿到同样的 400，
        // `attempts` 断言不会因为吃到兜底 500 而偶发变化。
        let bad = r#"{"error":"bad key"}"#.to_string();
        let (_srv, base) =
            fake_server(vec![(400, bad.clone()), (400, bad.clone()), (400, bad)]).await;
        let rec = recorder();
        let client = LlmClient::raw("k", &base, "m1").with_usage_sink(rec.clone());

        assert!(client.chat("sys", "user").await.is_err());

        let rows = rec.0.lock().unwrap();
        assert_eq!(rows.len(), 1);
        let r = &rows[0];
        assert!(!r.ok);
        assert_eq!(r.status, Some(400));
        // ⚠️ **不要断言 `attempts == 1`**：400 本身不可重试，但 `LlmClient` 对
        // `is_connect() || is_timeout()` **也会重试**（`mod.rs` 的 `Err(e)` 分支，
        // `attempt < 2`）。并行跑全量时偶发首连失败 ⇒ attempts 合法地变成 2
        // （实测：`--workspace` 并行下约 7%，且**与 `config` 的锁无关** ——
        // 该锁只保护它自己的临界区，覆盖不到这里）。
        // 这条用例要钉的是"**硬失败也记账**"，不是"连接层从不重试"。
        // 想钉精确次数请用 `send_retries_then_records_attempts`（script 可控）。
        assert!(
            (1..=3).contains(&r.attempts),
            "attempts 应在 1..=3（最多 3 次尝试），实际 {}",
            r.attempts
        );
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
        // 重试要退避 2s ⇒ 这条尤其依赖 listener 不被回收（见 fake_server 的 ⚠️）
        // ⚠️ **script 备 3 份**（= 客户端最大尝试次数），见 fake_server 的 ⚠️。
        // 这份多余的第 3 项正是为了兜住"首连失败"那条路径：
        //   干净路径   attempt0 拿 500 → 重试 → attempt1 拿 200 ⇒ attempts == 2
        //   首连失败   attempt0 落 `Err(is_connect())` → attempt1 拿 500 → attempt2 拿 200
        //                                                          ⇒ attempts == 3
        // 只给 2 份时，首连失败的第 3 次会吃到兜底 500 ⇒ 用例以"脚本耗尽"形态报错（且要等退避）。
        let (_srv, base) = fake_server(vec![
            (500, "{}".to_string()),
            (200, ok_body.clone()),
            (200, ok_body),
        ])
        .await;
        let rec = recorder();
        let client = LlmClient::raw("k", &base, "m1").with_usage_sink(rec.clone());

        assert_eq!(client.chat("sys", "user").await.unwrap(), "ok");

        let rows = rec.0.lock().unwrap();
        assert_eq!(rows.len(), 1, "重试不另起一行");
        // ⚠️ **不要断言 ` == 2`**：本用例的真实契约是"**重试后成功仍只记一行、
        // 且 attempts 记实际尝试次数**"，不是"恰好重试一次"。首连失败（`is_connect()`）
        // 会让 attempts 合法地变成 3 —— 复核已用**确定性注入**（attempt0 指向死端口）
        // 复现出 `left: 3, right: 2`，证明这条断言同样暴露在传输层偶发下。
        // 契约仍被钉住：**下界 2 证明它确实重试了**（这正是"重试用例"的意义）。
        assert!(
            (2..=3).contains(&rows[0].attempts),
            "重试后 attempts 应在 2..=3，实际 {}",
            rows[0].attempts
        );
        assert!(rows[0].ok);
        assert_eq!(rows[0].prompt_tokens, 5);
    }
}
