//! LLM 用量记账：每次调用的 token / 缓存命中 / 耗时 / 成败。
//!
//! **采集点在 `LlmClient::send` 一处**（所有链路——事件检测、字幕订正、精修分段/事件/关键词、
//! 封面文案、贴纸 tags——都从那里出去），调用方不用各写一遍。core 只负责"产出记录"，
//! 写到哪（SQLite 账本）由 CLI/server 侧装一个 [`UsageSink`] 决定——
//! 与 `progress::ProgressReporter` 同一个思路，只是因为 LLM 调用发生在很深的窗口任务里，
//! 这里用进程级默认 sink 而不是逐层传参。
//!
//! 记账**绝不能影响调用结果**：sink 里的错误由安装方自己吞掉并 warn，
//! `emit` 本身也不返回错误。

use std::sync::{Arc, Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::Value;

/// 一次 LLM 调用（含重试）的最终结果。
///
/// `attempts` 是这个数字：重试 3 次才成功记 3 —— **一次调用只记一行**，
/// 不是每次尝试一行，否则统计里"调用次数"会被重试放大。
#[derive(Debug, Clone, Default)]
pub struct UsageRecord {
    /// unix 秒。`SystemTime` 取不到时是 0（不 panic）。
    pub ts: i64,
    /// 链路名（"事件检测" / "字幕订正" / "精修段事件" / …）。
    pub label: String,
    /// 项目（或切片）目录，用于按项目汇总；拿不到的链路是 None。
    pub project: Option<String>,
    pub model: String,
    /// base_url 的主机名：区分官方与被中转站转发（同一个模型名价格不同）。
    pub host: String,
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    /// 命中 provider 前缀缓存的输入 token（DeepSeek `prompt_cache_hit_tokens` /
    /// OpenAI `prompt_tokens_details.cached_tokens`）。没有这个能力的中转一直是 0。
    pub cached_tokens: u64,
    /// 思维链 token（`completion_tokens_details.reasoning_tokens`），已含在 completion 里。
    pub reasoning_tokens: u64,
    pub latency_ms: u64,
    pub attempts: u32,
    pub ok: bool,
    /// 失败时的 HTTP 状态码（连接失败/超时是 None）。
    pub status: Option<u16>,
    /// 失败简述（已截断，避免把整页响应写进库）。
    pub error: Option<String>,
}

/// 用量记录的去处。安装方（CLI/server）实现它写库；测试可注入自己的实现。
pub trait UsageSink: Send + Sync {
    fn record(&self, usage: &UsageRecord);
}

/// 链路归因：谁会出现在 `UsageRecord.label` / `project` 上。
#[derive(Debug, Clone, Default)]
pub struct UsageContext {
    pub label: String,
    pub project: Option<String>,
}

impl UsageContext {
    pub fn new(label: impl Into<String>) -> Self {
        Self { label: label.into(), project: None }
    }
    pub fn project(mut self, project: Option<impl Into<String>>) -> Self {
        self.project = project.map(|p| p.into());
        self
    }
}

static DEFAULT_SINK: OnceLock<Mutex<Option<Arc<dyn UsageSink>>>> = OnceLock::new();

fn slot() -> &'static Mutex<Option<Arc<dyn UsageSink>>> {
    DEFAULT_SINK.get_or_init(|| Mutex::new(None))
}

/// 装进程级默认 sink（服务端启动、CLI 跑 LLM 命令前各调一次）。
pub fn set_default_sink(sink: Arc<dyn UsageSink>) {
    if let Ok(mut guard) = slot().lock() {
        *guard = Some(sink);
    }
}

/// 取消防默认监听（测试用：避免并行测试互相看到对方的记录）。
pub fn clear_default_sink() {
    if let Ok(mut guard) = slot().lock() {
        *guard = None;
    }
}

/// 当前默认 sink（没有就返回 None）。
pub fn default_sink() -> Option<Arc<dyn UsageSink>> {
    slot().lock().ok().and_then(|g| g.clone())
}

/// 记录一次调用（走进程默认 sink）。没有 sink 时只打日志——**不影响调用方**。
pub fn emit(record: UsageRecord) {
    emit_with(None, record);
}

/// 同上，但优先用调用方自带的 sink（`LlmClient::with_usage_sink`，测试用），
/// 其次才是进程默认 sink。
pub fn emit_with(sink: Option<&Arc<dyn UsageSink>>, record: UsageRecord) {
    log_line(&record);
    if let Some(sink) = sink.cloned().or_else(default_sink) {
        sink.record(&record);
    }
}

/// 每次调用一行结构化日志。logbus 会把字段渲染成 `k=v` 追加在正文里，
/// 于是设置页「运行日志」里也能看到用量（成功路径原本一行都没有）。
fn log_line(r: &UsageRecord) {
    if r.ok {
        tracing::info!(
            label = %r.label,
            model = %r.model,
            host = %r.host,
            prompt_tokens = r.prompt_tokens,
            completion_tokens = r.completion_tokens,
            cached_tokens = r.cached_tokens,
            latency_ms = r.latency_ms,
            attempts = r.attempts,
            "LLM 调用"
        );
    } else {
        tracing::warn!(
            label = %r.label,
            model = %r.model,
            host = %r.host,
            latency_ms = r.latency_ms,
            attempts = r.attempts,
            status = ?r.status,
            "LLM 调用失败"
        );
    }
}

/// 从响应里取用量。各家字段名不一致，这里一并认：
/// - OpenAI 标准：`usage.prompt_tokens` / `usage.completion_tokens`
/// - 前缀缓存：DeepSeek 的 `usage.prompt_cache_hit_tokens`、OpenAI 的 `usage.prompt_tokens_details.cached_tokens`
/// - 思维链：`usage.completion_tokens_details.reasoning_tokens`
///
/// 一个都没有 → 全 0（老中转、或响应里没有 usage，不算错误）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct UsageNumbers {
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub cached_tokens: u64,
    pub reasoning_tokens: u64,
}

pub fn parse_usage(body: &Value) -> UsageNumbers {
    let usage = &body["usage"];
    let get = |path: &[&str]| -> Option<u64> {
        let mut cur = usage;
        for k in path {
            cur = cur.get(k)?;
        }
        cur.as_u64()
    };
    UsageNumbers {
        prompt_tokens: get(&["prompt_tokens"]).unwrap_or(0),
        completion_tokens: get(&["completion_tokens"]).unwrap_or(0),
        cached_tokens: get(&["prompt_cache_hit_tokens"])
            .or_else(|| get(&["prompt_tokens_details", "cached_tokens"]))
            .unwrap_or(0),
        reasoning_tokens: get(&["completion_tokens_details", "reasoning_tokens"]).unwrap_or(0),
    }
}

/// base_url → 主机名（"https://api.deepseek.com/v1" → "api.deepseek.com"）。
/// 解析不出来就原样返回去掉 scheme 的部分，只为在统计里区分端点。
pub fn host_of(base_url: &str) -> String {
    let s = base_url.trim();
    let s = s.split("://").last().unwrap_or(s);
    let host = s.split('/').next().unwrap_or(s);
    if host.is_empty() { s.to_string() } else { host.to_string() }
}

/// 错误串截断：库里的 `error` 列只留能看懂的那一段。
pub fn truncate_error(msg: &str) -> String {
    const MAX: usize = 200;
    if msg.chars().count() <= MAX {
        return msg.to_string();
    }
    let mut out: String = msg.chars().take(MAX).collect();
    out.push('…');
    out
}

pub fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parse_usage_reads_openai_and_deepseek_shapes() {
        let openai = json!({"usage": {"prompt_tokens": 120, "completion_tokens": 30,
            "prompt_tokens_details": {"cached_tokens": 64},
            "completion_tokens_details": {"reasoning_tokens": 12}}});
        assert_eq!(
            parse_usage(&openai),
            UsageNumbers { prompt_tokens: 120, completion_tokens: 30, cached_tokens: 64, reasoning_tokens: 12 }
        );

        let deepseek = json!({"usage": {"prompt_tokens": 900, "completion_tokens": 40,
            "prompt_cache_hit_tokens": 768, "prompt_cache_miss_tokens": 132}});
        assert_eq!(
            parse_usage(&deepseek),
            UsageNumbers { prompt_tokens: 900, completion_tokens: 40, cached_tokens: 768, reasoning_tokens: 0 }
        );
    }

    #[test]
    fn parse_usage_tolerates_missing_or_partial_fields() {
        // 完全没有 usage（老中转）
        assert_eq!(parse_usage(&json!({"choices": []})), UsageNumbers::default());
        // 只有 total_tokens 之类对不上的字段
        assert_eq!(parse_usage(&json!({"usage": {"total_tokens": 500}})), UsageNumbers::default());
        // 只有输入
        assert_eq!(
            parse_usage(&json!({"usage": {"prompt_tokens": 7}})),
            UsageNumbers { prompt_tokens: 7, ..Default::default() }
        );
    }

    #[test]
    fn host_of_strips_scheme_and_path() {
        assert_eq!(host_of("https://api.deepseek.com/v1"), "api.deepseek.com");
        assert_eq!(host_of("http://127.0.0.1:8080/v1/chat/completions"), "127.0.0.1:8080");
        assert_eq!(host_of(""), "");
    }

    #[test]
    fn emit_reaches_default_sink() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static HITS: AtomicUsize = AtomicUsize::new(0);
        struct Counting;
        impl UsageSink for Counting {
            fn record(&self, _u: &UsageRecord) {
                HITS.fetch_add(1, Ordering::SeqCst);
            }
        }
        set_default_sink(Arc::new(Counting));
        emit(UsageRecord { label: "test".into(), ok: true, ..Default::default() });
        assert_eq!(HITS.load(Ordering::SeqCst), 1);
        clear_default_sink();
        assert!(default_sink().is_none());
    }
}
