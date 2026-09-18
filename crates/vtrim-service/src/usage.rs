//! LLM 用量：落库（`DbSink`）与汇总（`summarize`）。
//!
//! 采集点在 `vtrim_core::llm::LlmClient::send`——那里是全链路唯一出口，
//! 所以这里只需要实现一个 [`UsageSink`] 并装上去，不用碰任何调用方。
//!
//! - 服务端：`server::run` 启动时 [`install`]（复用 `AuthService` 手上的同一个 Db）
//! - CLI：`vtrim process / detect / correct / polish *` 之前 [`try_install_cli_sink`]
//!
//! 写库失败**绝不影响调用**：sink 里自己 warn 掉。

use anyhow::Result;
use serde::Serialize;
use std::sync::Arc;
use vtrim_core::llm::usage::{UsageRecord, UsageSink};

use super::db::Db;

/// 把用量写进账本。`Send + Sync`：Agent 并发窗口会同时调进来，
/// SQLite 单连接由 `Db` 内部的锁串行化（一次 INSERT 亚毫秒级）。
pub struct DbSink {
    db: Arc<Db>,
}

impl DbSink {
    pub fn new(db: Arc<Db>) -> Self {
        Self { db }
    }
}

impl UsageSink for DbSink {
    fn record(&self, u: &UsageRecord) {
        if let Err(e) = self.db.insert_llm_usage(u) {
            // 记账坏了不能影响跑管线，只留一行警告
            tracing::warn!("LLM 用量写库失败: {e}");
        }
    }
}

/// 装进程级默认 sink（重复调用以最后一次为准）。
pub fn install(db: Arc<Db>) {
    vtrim_core::llm::usage::set_default_sink(Arc::new(DbSink::new(db)));
}

/// CLI 入口用：开账本并装上 sink。失败只 warn（没有账本不该拦住 `vtrim correct` 这类命令）。
pub fn try_install_cli_sink() {
    match Db::open_default() {
        Ok(db) => install(Arc::new(db)),
        Err(e) => tracing::warn!("LLM 用量记账未启用（打开账本失败）: {e}"),
    }
}

/// 一个分组的汇总。
#[derive(Debug, Clone, Default, Serialize)]
pub struct UsageGroup {
    /// 分组键：日期（YYYY-MM-DD）/ 链路名 / 项目路径 / 模型名
    pub key: String,
    pub calls: i64,
    pub failed: i64,
    pub prompt_tokens: i64,
    pub completion_tokens: i64,
    pub cached_tokens: i64,
    pub reasoning_tokens: i64,
    pub latency_ms: i64,
    pub attempts: i64,
}

/// 总量 + 四张分组表。
#[derive(Debug, Clone, Default, Serialize)]
pub struct UsageSummary {
    pub days: i64,
    pub totals: UsageGroup,
    pub by_day: Vec<UsageGroup>,
    pub by_label: Vec<UsageGroup>,
    pub by_project: Vec<UsageGroup>,
    pub by_model: Vec<UsageGroup>,
}

const COLS: &str = "COUNT(*) AS calls,
     COALESCE(SUM(CASE WHEN ok = 0 THEN 1 ELSE 0 END), 0) AS failed,
     COALESCE(SUM(prompt_tokens), 0) AS prompt_tokens,
     COALESCE(SUM(completion_tokens), 0) AS completion_tokens,
     COALESCE(SUM(cached_tokens), 0) AS cached_tokens,
     COALESCE(SUM(reasoning_tokens), 0) AS reasoning_tokens,
     COALESCE(SUM(latency_ms), 0) AS latency_ms,
     COALESCE(SUM(attempts), 0) AS attempts";

/// 汇总最近 `days` 天的用量（`days` 会夹到 1..=3650）。
///
/// 全部用 SQL 聚合：这张表会一直长（一次检测几十行），不在 Rust 里扫全表。
pub fn summarize(db: &Db, days: i64) -> Result<UsageSummary> {
    let days = days.clamp(1, 3650);
    let since = vtrim_core::llm::usage::now_secs() - days * 86_400;
    let mut summary = UsageSummary { days, ..Default::default() };

    let totals = db.query_llm_usage_row(
        &format!("SELECT '' AS key, {COLS} FROM llm_usage WHERE ts >= ?1"),
        since,
    )?;
    summary.totals = totals.unwrap_or_default();
    summary.totals.key = format!("{days} 天");

    summary.by_day = db.query_llm_usage_groups(
        &format!(
            "SELECT date(ts, 'unixepoch', 'localtime') AS key, {COLS}
             FROM llm_usage WHERE ts >= ?1 GROUP BY key ORDER BY key DESC"
        ),
        since,
    )?;
    summary.by_label = db.query_llm_usage_groups(
        &format!(
            "SELECT label AS key, {COLS} FROM llm_usage WHERE ts >= ?1
             GROUP BY key ORDER BY prompt_tokens + completion_tokens DESC"
        ),
        since,
    )?;
    summary.by_project = db.query_llm_usage_groups(
        &format!(
            "SELECT COALESCE(project, '(未归类)') AS key, {COLS} FROM llm_usage WHERE ts >= ?1
             GROUP BY key ORDER BY prompt_tokens + completion_tokens DESC LIMIT 50"
        ),
        since,
    )?;
    summary.by_model = db.query_llm_usage_groups(
        &format!(
            "SELECT model || CASE WHEN host = '' THEN '' ELSE '@' || host END AS key, {COLS}
             FROM llm_usage WHERE ts >= ?1 GROUP BY key ORDER BY prompt_tokens + completion_tokens DESC"
        ),
        since,
    )?;
    Ok(summary)
}

/// `vtrim usage`：把汇总打成三张小表（按天 / 按链路 / 按项目）。
pub fn cmd_usage(days: i64) -> Result<()> {
    let db = Db::open_default()?;
    let s = summarize(&db, days)?;
    println!("LLM 用量（最近 {} 天）", s.days);
    if s.totals.calls == 0 {
        println!("  还没有记录。跑一次事件检测 / 精修 LLM 之后就有了。");
        return Ok(());
    }
    let t = &s.totals;
    println!(
        "  调用 {} 次（失败 {}）｜ 输入 {} tok ｜ 输出 {} tok ｜ 缓存命中 {} tok（缓存率 {}）｜ 平均 {:.1}s",
        t.calls,
        t.failed,
        t.prompt_tokens,
        t.completion_tokens,
        t.cached_tokens,
        fmt_rate(t),
        avg_secs(t),
    );
    print_table("按链路", &s.by_label);
    print_table("按项目", &s.by_project);
    print_table("按天（最近 14 行）", &s.by_day.iter().take(14).cloned().collect::<Vec<_>>());
    if !s.by_model.is_empty() {
        print_table("按模型", &s.by_model);
    }
    Ok(())
}

fn avg_secs(g: &UsageGroup) -> f64 {
    if g.calls == 0 { 0.0 } else { g.latency_ms as f64 / 1000.0 / g.calls as f64 }
}

/// 缓存率（%）= Σ缓存命中 ÷ Σ输入，**只算输入侧、先求和再相除**。
///
/// `cached_tokens` 是 `prompt_tokens` 的子集（DeepSeek 的 hit+miss、OpenAI 的
/// `prompt_tokens_details` 都是这个关系），所以比率天然落在 0~100。
/// 分母不能掺 `completion_tokens`：缓存只作用于输入，混进输出会让"模型话多话少"影响这个数。
///
/// `prompt_tokens == 0` 时返回 `None` —— 那是"这次没报 usage"（老中转，或失败行的 token 全 0），
/// 不是"一次都没命中"，显示 `—` 而不是 `0%`。
fn cache_rate(g: &UsageGroup) -> Option<f64> {
    if g.prompt_tokens <= 0 {
        return None;
    }
    Some(g.cached_tokens as f64 / g.prompt_tokens as f64 * 100.0)
}

/// 给人看的缓存率：没有输入 token 时是 `—`。
fn fmt_rate(g: &UsageGroup) -> String {
    match cache_rate(g) {
        Some(r) => format!("{r:.1}%"),
        None => "—".to_string(),
    }
}

fn print_table(title: &str, rows: &[UsageGroup]) {
    if rows.is_empty() {
        return;
    }
    println!("\n  {title}");
    println!(
        "    {:<32} {:>6} {:>4} {:>10} {:>10} {:>10} {:>7} {:>8}",
        "key", "调用", "失败", "输入tok", "输出tok", "缓存tok", "缓存率", "均耗时"
    );
    for g in rows {
        let key: String = if g.key.chars().count() > 32 {
            let mut k: String = g.key.chars().take(29).collect();
            k.push_str("...");
            k
        } else {
            g.key.clone()
        };
        println!(
            "    {:<32} {:>6} {:>4} {:>10} {:>10} {:>10} {:>7} {:>7.1}s",
            key,
            g.calls,
            g.failed,
            g.prompt_tokens,
            g.completion_tokens,
            g.cached_tokens,
            fmt_rate(g),
            avg_secs(g)
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn group(prompt: i64, cached: i64) -> UsageGroup {
        UsageGroup { prompt_tokens: prompt, cached_tokens: cached, ..Default::default() }
    }

    #[test]
    fn cache_rate_is_input_side_only_and_weighted() {
        // 分母是输入 token（SUM 聚合后的值），不是"每次调用比率的平均"：
        // 一次 1000 输入命中 768 → 76.8%，与调用次数无关
        assert!((cache_rate(&group(1000, 768)).unwrap() - 76.8).abs() < 1e-9);
        assert_eq!(fmt_rate(&group(1000, 768)), "76.8%");
        assert_eq!(fmt_rate(&group(500, 500)), "100.0%");
        assert_eq!(fmt_rate(&group(500, 0)), "0.0%");
        // 输出 token 不参与这个公式：算出 0% 时 completion 是多少都不影响
        let with_output = UsageGroup { completion_tokens: 9999, ..group(500, 0) };
        assert_eq!(fmt_rate(&with_output), "0.0%");
    }

    #[test]
    fn cache_rate_is_dash_without_prompt_tokens() {
        // 老中转没报 usage / 失败行（token 全 0）→ 没有分母，显示 — 而不是 0%
        assert_eq!(cache_rate(&group(0, 0)), None);
        assert_eq!(fmt_rate(&group(0, 0)), "—");
        assert_eq!(fmt_rate(&UsageGroup::default()), "—");
    }
}
