//! llm::agent：把条目切成窗口、并发跑 LLM 任务的通用编排器（含 checkpoint 续传）。
//!
//! 入口：`Agent`（`new` / `with_window_size` / `with_max_concurrent` / `with_cancel` / `run` / `run_with_checkpoint` / `run_windows`）、`AgentTask`、`AgentItem`、`AgentWindow`、`CancelProbe`、`extract_json_array` / `extract_json_object`、`windows_from_checkpoint`。
//! 边界：checkpoint 按提示词指纹失效（`prompt_fingerprint`），别绕过；并发只由 `LlmConfig::concurrency` 决定，别写死数字 —— 见 `crates/vtrim-core/src/llm/AGENTS.md`

use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result};
use async_trait::async_trait;
use serde_json::Value;
use tokio::sync::{mpsc, Semaphore};
use tokio::task::JoinSet;

use crate::config::LlmConfig;
use crate::llm::LlmClient;

/// 取消探针：返回 true = 任务已被请求取消。
///
/// `Agent` 在**派发每个窗口之前**与**每收一个窗口之后**检查它：命中就不再起新窗口，
/// 并 `abort_all()` 掉在飞的窗口（在飞的那次 HTTP 请求会随 future 一起被丢弃）。
/// 没有这道闸，"取消"只在**整个阶段跑完之后**才生效 —— 用户点了停止，LLM 还在后台
/// 一个窗口接一个窗口地跑（实测 18 个窗口要再跑十几分钟）。
pub type CancelProbe = Arc<dyn Fn() -> bool + Send + Sync>;

/// Agent 输入项：带时间戳的文本片段（字幕段、事件候选等）。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct AgentItem {
    pub index: usize,
    pub start: f64,
    pub end: f64,
    pub text: String,
}

/// 订正等按条目输出时的结果。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AgentOutput {
    /// 条目索引（原始 index）
    pub index: usize,
    /// 输出文本（订正后文本等）
    pub text: String,
}

/// 一个 Agent 窗口。
///
/// `T` 是窗口级输出类型：
/// - 订正：`Vec<AgentOutput>`（窗口内各条目的修正）
/// - 事件检测：`Vec<Event>`（该窗口分析出的事件）
#[derive(Debug, Clone, serde::Serialize)]
pub struct AgentWindow<T> {
    /// 稳定窗口 ID（用于 checkpoint 续传）
    pub id: String,
    /// 窗口内第一个条目在全局 items 中的偏移
    pub offset: usize,
    pub items: Vec<AgentItem>,
    /// 窗口附加上下文（如 polish 的 segment_index/direction/budget），可空
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub meta: Option<Value>,
    /// 已完成的输出；未完成时为 None
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output: Option<T>,
}

impl<'de, T: serde::Deserialize<'de>> serde::Deserialize<'de> for AgentWindow<T> {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(serde::Deserialize)]
        #[serde(bound(deserialize = "T: serde::Deserialize<'de>"))]
        struct Inner<T> {
            id: String,
            offset: usize,
            items: Vec<AgentItem>,
            #[serde(default)]
            meta: Option<Value>,
            #[serde(default)]
            output: Option<T>,
        }
        let inner = Inner::deserialize(deserializer)?;
        Ok(Self {
            id: inner.id,
            offset: inner.offset,
            items: inner.items,
            meta: inner.meta,
            output: inner.output,
        })
    }
}

/// 进度信息：已完成窗口数 / 总窗口数，以及刚完成窗口的输出。
#[derive(Debug, Clone)]
pub struct AgentProgress<O> {
    pub completed_windows: usize,
    pub total_windows: usize,
    /// 刚完成窗口的输出（供边处理边保存/展示）
    pub output: Option<O>,
}

/// 通用 Agent 任务定义。
///
/// 不同任务（订正、事件识别、摘要）实现此 trait，复用 Agent 执行器。
#[async_trait]
pub trait AgentTask<T: Send + Sync + Clone + 'static>: Send + Sync {
    /// 任务名称（日志 / 进度显示）
    fn name(&self) -> &str;

    /// 用量记账的归因（链路名 + 项目目录）。默认用 `name()`；认得出项目的任务覆写它。
    fn usage_context(&self) -> crate::llm::usage::UsageContext {
        crate::llm::usage::UsageContext::new(self.name())
    }

    /// 是否启用思考模式（LLM 先输出 reasoning 再输出结果）
    fn thinking(&self) -> bool {
        false
    }

    /// 构建全局上下文（一次性提取，所有窗口共享）
    fn build_global_context(&self, _items: &[AgentItem]) -> String {
        String::new()
    }

    /// 构建单个窗口的局部上下文（前文/后文/弹幕等）
    fn build_window_context(&self, _items: &[AgentItem], _window: &[AgentItem], _global: &str) -> String {
        String::new()
    }

    /// 构建发给 LLM 的 system prompt
    fn system_prompt(&self) -> String;

    /// 构建发给 LLM 的 user prompt（包含全局上下文、窗口上下文、待处理 JSON）
    fn user_prompt(&self, global: &str, window_context: &str, window: &[AgentItem]) -> String;

    /// 解析 LLM 输出，返回窗口级结果。
    fn parse_output(&self, content: &str, window: &[AgentItem]) -> Result<T>;

    /// 处理一个窗口（默认实现：构建 prompt → 调用 LLM → parse）。
    async fn process_window(
        &self,
        all_items: &[AgentItem],
        window: &AgentWindow<T>,
        global: &str,
        llm_cfg: &LlmConfig,
    ) -> Result<T> {
        // 局部上下文：当前窗口前 3 条 + 后 2 条（供参考）
        let ctx_start = window.offset.saturating_sub(3);
        let ctx_end = (window.offset + window.items.len() + 2).min(all_items.len());
        let ctx_items = &all_items[ctx_start..ctx_end];

        let window_context = self.build_window_context(all_items, ctx_items, global);
        let system_prompt = self.system_prompt();
        let user_prompt = self.user_prompt(global, &window_context, &window.items);

        let client = LlmClient::from_config(llm_cfg)?
            .with_timeout(120)
            .with_usage_context(self.usage_context());
        // `AgentTask::thinking()` 是**订正那条链路的老开关**（`[correct] thinking`）。
        // LLM 层已经表态时（`[llm].thinking` 或 `[llm.overrides.<link>].thinking`，由
        // `LlmConfig::for_link` 解析出来）以 LLM 层为准 —— 否则它会把解析结果顶掉。
        let client = if self.thinking() && llm_cfg.thinking.is_none() {
            client.with_thinking(Some("enabled"))
        } else {
            client
        };

        let messages = serde_json::json!([
            {"role": "system", "content": system_prompt},
            {"role": "user", "content": user_prompt}
        ]);
        let resp_body = client.send(messages).await?;
        let content = resp_body["choices"][0]["message"]["content"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("LLM 响应缺少 choices[0].message.content"))?;

        self.parse_output(content, &window.items)
    }

    /// 提示词指纹：折进窗口 id，参与 checkpoint 命中判断。
    ///
    /// checkpoint 的复用条件是 `id 相同 && items 相同`——它只保证「字幕没变」，
    /// 保不了「提示词没变」。调 prompt 后重跑会命中旧窗口、拿回旧结果。
    /// 把 system/user prompt 的指纹折进 id 就能让提示词一变、全部窗口失效。
    /// 默认空（不参与）；实现了它的任务会得到内容感知的缓存。
    fn prompt_fingerprint(&self) -> String {
        String::new()
    }

    /// 将已完成窗口持久化为 JSON（checkpoint 文件内容）。
    fn save_windows(&self, windows: &[AgentWindow<T>]) -> Result<Value>;

    /// 从 checkpoint JSON 恢复已完成窗口。
    fn load_windows(&self, value: Value) -> Result<Vec<AgentWindow<T>>>;
}

/// 把参与 prompt 的片段哈希成一个短串，给 checkpoint 做失效判断用。
///
/// 用 `DefaultHasher`（SipHash，键固定）→ 跨进程稳定；不追求抗碰撞，
/// 它只用来判断「和上次是不是同一套提示词」。
pub fn fingerprint(parts: &[&str]) -> String {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    for p in parts {
        p.hash(&mut h);
        // 分隔符：避免 ("ab","c") 与 ("a","bc") 撞成同一个指纹
        0xffu8.hash(&mut h);
    }
    format!("{:016x}", h.finish())
}

/// 通用 Agent 执行器。
pub struct Agent {
    llm: LlmConfig,
    window_size: usize,
    max_concurrent: usize,
    /// 取消探针（不给 = 不支持取消，跑完为止）。
    cancel: Option<CancelProbe>,
}

impl Agent {
    pub fn new(llm: LlmConfig) -> Self {
        // 默认并发跟随 `[llm] max_concurrent`，别在这里另写一个常数
        let max_concurrent = llm.concurrency(None);
        Self {
            llm,
            window_size: 25,
            max_concurrent,
            cancel: None,
        }
    }

    /// 注入取消探针（见 [`CancelProbe`]）。不给时行为与从前一致（跑完为止）。
    pub fn with_cancel(mut self, cancel: CancelProbe) -> Self {
        self.cancel = Some(cancel);
        self
    }

    /// 现在是否已被请求取消。
    fn cancelled(&self) -> bool {
        self.cancel.as_ref().is_some_and(|c| c())
    }

    pub fn with_window_size(mut self, n: usize) -> Self {
        self.window_size = n.max(1);
        self
    }

    pub fn with_max_concurrent(mut self, n: usize) -> Self {
        self.max_concurrent = n.max(1);
        self
    }

    /// 按默认固定窗口切分 items。
    pub fn build_windows<O>(&self, items: &[AgentItem]) -> Vec<AgentWindow<O>> {
        items
            .chunks(self.window_size)
            .enumerate()
            .map(|(win_idx, chunk)| {
                let offset = win_idx * self.window_size;
                AgentWindow {
                    id: format!("win_{offset}"),
                    offset,
                    items: chunk.to_vec(),
                    meta: None,
                    output: None,
                }
            })
            .collect()
    }

    /// 运行 Agent 任务。
    pub async fn run<A, O>(
        &self,
        task: A,
        items: &[AgentItem],
    ) -> Result<Vec<AgentWindow<O>>>
    where
        A: AgentTask<O> + Send + Sync + 'static,
        O: Send + Sync + Clone + 'static,
    {
        let windows = self.build_windows(items);
        self.run_windows(task, windows, None, None).await
    }

    /// 带 checkpoint / 进度回调运行。
    ///
    /// - `checkpoint_path`：已存在时跳过其中已完成窗口；每完成一个窗口立即原子写回。
    /// - `progress_tx`：非空时每完成一个窗口发送进度（边处理边输出）。
    pub async fn run_with_checkpoint<A, O>(
        &self,
        task: A,
        items: &[AgentItem],
        checkpoint_path: Option<&Path>,
        progress_tx: Option<mpsc::Sender<AgentProgress<O>>>,
    ) -> Result<Vec<AgentWindow<O>>>
    where
        A: AgentTask<O> + Send + Sync + 'static,
        O: Send + Sync + Clone + 'static,
    {
        let windows = self.build_windows(items);
        self.run_windows(task, windows, checkpoint_path, progress_tx).await
    }

    /// 执行已构建好的窗口列表（支持自定义窗口切分，如事件检测的重叠窗口）。
    pub async fn run_windows<A, O>(
        &self,
        task: A,
        mut windows: Vec<AgentWindow<O>>,
        checkpoint_path: Option<&Path>,
        progress_tx: Option<mpsc::Sender<AgentProgress<O>>>,
    ) -> Result<Vec<AgentWindow<O>>>
    where
        A: AgentTask<O> + Send + Sync + 'static,
        O: Send + Sync + Clone + 'static,
    {
        if windows.is_empty() {
            return Ok(windows);
        }

        // 把提示词指纹折进窗口 id：checkpoint 的命中条件是 `id && items`，
        // 单靠 items 挡不住「字幕没变但 prompt 改了」这种情况（会拿回旧结果）。
        let fingerprint = task.prompt_fingerprint();
        if !fingerprint.is_empty() {
            for w in &mut windows {
                w.id = format!("{}@{}", w.id, fingerprint);
            }
        }

        // 恢复 checkpoint：跳过已完成窗口
        if let Some(path) = checkpoint_path {
            if path.exists() {
                match std::fs::read_to_string(path) {
                    Ok(content) => match serde_json::from_str::<Value>(&content) {
                        Ok(value) => match task.load_windows(value) {
                            Ok(completed) => {
                                let completed_ids: std::collections::HashSet<String> = completed
                                    .iter()
                                    .filter(|w| w.output.is_some())
                                    .map(|w| w.id.clone())
                                    .collect();
                                if !completed_ids.is_empty() {
                                    tracing::info!(
                                        "{} 续传: 已从 checkpoint 跳过 {} 个窗口",
                                        task.name(),
                                        completed_ids.len()
                                    );
                                    let mut skipped = 0usize;
                                    for w in &mut windows {
                                        if let Some(c) = completed
                                            .iter()
                                            .find(|c| c.id == w.id && c.items == w.items)
                                        {
                                            w.output = c.output.clone();
                                            skipped += 1;
                                        }
                                    }
                                    tracing::info!(
                                        "{} 续传: 实际跳过 {} 个窗口（内容匹配）",
                                        task.name(),
                                        skipped
                                    );
                                }
                            }
                            Err(e) => {
                                tracing::warn!(
                                    "{} checkpoint 解析失败，忽略后重新开始: {}",
                                    task.name(),
                                    e
                                );
                            }
                        },
                        Err(e) => {
                            tracing::warn!(
                                "{} checkpoint JSON 解析失败，忽略后重新开始: {}",
                                task.name(),
                                e
                            );
                        }
                    },
                    Err(e) => {
                        tracing::warn!("{} checkpoint 读取失败: {}", task.name(), e);
                    }
                }
            }
        }

        let total_windows = windows.len();
        let global_items: Vec<AgentItem> = windows.iter().flat_map(|w| w.items.iter().cloned()).collect();
        let global = task.build_global_context(&global_items);
        let task = Arc::new(task);
        let windows = Arc::new(std::sync::Mutex::new(windows));
        let llm = Arc::new(self.llm.clone());
        let task_name = task.name().to_string();
        let sem = Arc::new(Semaphore::new(self.max_concurrent));
        let mut set: JoinSet<(usize, Result<O>)> = JoinSet::new();

        let completed_initial = {
            let guard = windows.lock().unwrap_or_else(|e| e.into_inner());
            guard.iter().filter(|w| w.output.is_some()).count()
        };

        let pb = indicatif::ProgressBar::new(total_windows as u64);
        if let Ok(style) = indicatif::ProgressStyle::with_template(
            "{spinner:.green} [{elapsed_precise}] {} {bar:40} {percent}%",
        ) {
            pb.set_style(style.progress_chars("##-"));
        }
        pb.enable_steady_tick(std::time::Duration::from_millis(100));
        pb.inc(completed_initial as u64);

        // 收集需要处理的窗口索引（已完成的不 spawn）
        let pending: Vec<usize> = {
            let guard = windows.lock().unwrap_or_else(|e| e.into_inner());
            guard
                .iter()
                .enumerate()
                .filter(|(_, w)| w.output.is_none())
                .map(|(i, _)| i)
                .collect()
        };

        for idx in pending {
            // 派发前先看取消：已经点了停止就别再起新窗口（每个窗口都是一次几十秒的 LLM 调用）
            if self.cancelled() {
                tracing::warn!("{} 已被取消：停止派发剩余窗口", task.name());
                pb.abandon_with_message(format!("{} 已取消", task_name));
                return Err(anyhow::anyhow!("任务已取消"));
            }
            let task = task.clone();
            let llm = llm.clone();
            let sem = sem.clone();
            let windows = windows.clone();
            let global = global.clone();

            set.spawn(async move {
                let _permit = sem.acquire().await.expect("Semaphore 仅在本函数内使用");
                let (all_items, window) = {
                    let guard = windows.lock().unwrap_or_else(|e| e.into_inner());
                    // 用 `get` 而不是 `guard[idx]`：idx 出自同一把锁下的 pending 列表、
                    // 且 windows 不会被裁剪，所以越界"不该发生"—— 但真发生时该是**一次窗口失败**，
                    // 不是一个 panic（panic 会毒化这把锁，把整批窗口一起带走）。
                    let Some(window) = guard.get(idx).cloned() else {
                        return (idx, Err(anyhow::anyhow!("窗口下标越界: {idx}")));
                    };
                    let all_items: Vec<AgentItem> = guard
                        .iter()
                        .flat_map(|w| w.items.iter().cloned())
                        .collect();
                    (all_items, window)
                };

                tracing::info!(
                    "窗口 {} 处理中 ({} 条)",
                    window.id,
                    window.items.len()
                );
                let result = task.process_window(&all_items, &window, &global, &llm).await;
                (idx, result)
            });
        }

        let mut completed_count = completed_initial;
        let mut last_err: Option<anyhow::Error> = None;

        while let Some(joined) = set.join_next().await {
            match joined {
                Ok((idx, Ok(output))) => {
                    {
                        let mut guard = windows.lock().unwrap_or_else(|e| e.into_inner());
                        if let Some(w) = guard.get_mut(idx) {
                            w.output = Some(output);
                        }
                    }
                    completed_count += 1;
                    pb.inc(1);
                    tracing::info!("窗口 {} 完成", {
                        let guard = windows.lock().unwrap_or_else(|e| e.into_inner());
                        guard.get(idx).map(|w| w.id.clone()).unwrap_or_else(|| format!("#{idx}"))
                    });

                    let output = {
                        let guard = windows.lock().unwrap_or_else(|e| e.into_inner());
                        guard.get(idx).and_then(|w| w.output.clone())
                    };
                    if let Some(tx) = &progress_tx {
                        let _ = tx.send(AgentProgress {
                            completed_windows: completed_count,
                            total_windows,
                            output,
                        }).await;
                    }

                    if let Some(path) = &checkpoint_path {
                        save_checkpoint(&task, &windows, path);
                    }
                }
                Ok((idx, Err(e))) => {
                    pb.inc(1);
                    tracing::warn!("窗口 {} 失败: {e:#}", {
                        let guard = windows.lock().unwrap_or_else(|e| e.into_inner());
                        guard.get(idx).map(|w| w.id.clone()).unwrap_or_else(|| format!("#{idx}"))
                    });
                    last_err = Some(e);
                }
                Err(e) => {
                    tracing::warn!("并行任务失败: {}", e);
                }
            }

            // 记完这一个窗口（含 checkpoint）再看取消：命中就 abort 在飞的（连带丢弃
            // 它们的 HTTP 请求）并报「任务已取消」，而不是把剩下十几个窗口跑完。
            // 已经完成的窗口留在 checkpoint 里，重跑时续传不重做。
            if self.cancelled() {
                let inflight = set.len();
                tracing::warn!("{} 已被取消：中止 {inflight} 个在飞窗口", task.name());
                set.abort_all();
                pb.abandon_with_message(format!("{} 已取消", task_name));
                return Err(anyhow::anyhow!("任务已取消"));
            }
        }

        pb.finish_with_message(format!("{} 完成", task_name));

        let final_windows = match Arc::try_unwrap(windows) {
            Ok(mutex) => mutex.into_inner().unwrap_or_else(|p| p.into_inner()),
            // 锁中毒也要能取回数据（与 persist.rs / logbus.rs 同口径）：
            // `unwrap()` 会让"一次窗口任务 panic"升级成"之后每一次加锁都 panic"。
            Err(arc) => arc.lock().unwrap_or_else(|e| e.into_inner()).clone(),
        };

        // 失败窗口保留 output None，由调用方决定兜底策略
        if last_err.is_some() && final_windows.iter().all(|w| w.output.is_none()) {
            return Err(last_err.unwrap());
        }

        Ok(final_windows)
    }
}

/// 原子写 checkpoint：先写临时文件再 rename。
fn save_checkpoint<A, O>(
    task: &Arc<A>,
    windows: &Arc<std::sync::Mutex<Vec<AgentWindow<O>>>>,
    path: &Path,
) where
    A: AgentTask<O> + Send + Sync + 'static,
    O: Send + Sync + Clone + 'static,
{
    let guard = match windows.lock() {
        Ok(g) => g,
        Err(poisoned) => poisoned.into_inner(),
    };
    match task.save_windows(&guard) {
        Ok(value) => {
            if let Some(parent) = path.parent() {
                if !parent.as_os_str().is_empty() {
                    let _ = std::fs::create_dir_all(parent);
                }
            }
            // 临时名带 pid：同一目录可能有别的进程在写同一份 checkpoint（CLI 直跑 + 服务端），
            // 共用 `*.tmp` 时一方 open-truncate、另一方正好 rename，会把截断的 JSON 换上去
            let tmp = path.with_extension(format!("tmp{}", std::process::id()));
            match std::fs::write(&tmp, serde_json::to_vec_pretty(&value).unwrap_or_default()) {
                Ok(()) => {
                    if let Err(e) = std::fs::rename(&tmp, path) {
                        tracing::warn!("checkpoint 写入失败: {}", e);
                    }
                }
                Err(e) => {
                    tracing::warn!("checkpoint 临时写入失败: {}", e);
                }
            }
        }
        Err(e) => {
            tracing::warn!("checkpoint 序列化失败: {}", e);
        }
    }
}

/// 从 LLM 输出中提取 JSON（支持 markdown code fence）。
pub fn extract_json(text: &str) -> &str {
    let text = text.trim();
    let text = if let Some(pos) = text.find("```json") {
        let after = &text[pos + 7..];
        after.split("```").next().unwrap_or(text)
    } else if let Some(pos) = text.find("```") {
        let after = &text[pos + 3..];
        after.split("```").next().unwrap_or(text)
    } else {
        text
    };
    let text = text.trim();
    let start = text.find(['{', '[']).unwrap_or(0);
    let end = text.rfind(['}', ']']).map(|i| i + 1).unwrap_or(text.len());
    &text[start..end]
}

/// 从 LLM 输出中提取 JSON 数组（支持 markdown code fence）。
pub fn extract_json_array(text: &str) -> anyhow::Result<String> {
    let trimmed = text.trim();
    if let Some(start) = trimmed.find('[') {
        if let Some(end) = trimmed.rfind(']') {
            return Ok(trimmed[start..=end].to_string());
        }
    }
    anyhow::bail!("No JSON array found:\n{}", &trimmed[..200.min(trimmed.len())])
}

/// 从 LLM 输出中提取 JSON 对象（支持 markdown code fence）。
pub fn extract_json_object(text: &str) -> anyhow::Result<String> {
    let trimmed = text.trim();
    if let Some(start) = trimmed.find('{') {
        if let Some(end) = trimmed.rfind('}') {
            return Ok(trimmed[start..=end].to_string());
        }
    }
    anyhow::bail!("No JSON object found:\n{}", &trimmed[..200.min(trimmed.len())])
}

/// 将条目列表序列化为 JSON 数组（供 user prompt 使用）。
pub fn items_to_json(items: &[AgentItem]) -> Result<Value> {
    let arr: Vec<Value> = items
        .iter()
        .map(|it| {
            serde_json::json!({
                "index": it.index,
                "start": it.start,
                "end": it.end,
                "text": it.text,
            })
        })
        .collect();
    Ok(Value::Array(arr))
}

/// 从 checkpoint 文件内容反序列化为已完成窗口（供调用方读取最终结果）。
pub fn windows_from_checkpoint<T>(path: &Path) -> Result<Vec<AgentWindow<T>>>
where
    T: serde::de::DeserializeOwned,
{
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("读取 checkpoint 失败: {}", path.display()))?;
    let value: Value = serde_json::from_str(&content)
        .with_context(|| format!("解析 checkpoint JSON 失败: {}", path.display()))?;
    serde_json::from_value(value)
        .with_context(|| format!("解析 checkpoint 窗口失败: {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_windows_uses_window_size() {
        let agent = Agent::new(LlmConfig::default()).with_window_size(3);
        let items: Vec<AgentItem> = (0..7)
            .map(|i| AgentItem {
                index: i + 1,
                start: i as f64,
                end: i as f64 + 1.0,
                text: format!("t{}", i),
            })
            .collect();
        let wins = agent.build_windows::<AgentOutput>(&items);
        assert_eq!(wins.len(), 3);
        assert_eq!(wins[0].items.len(), 3);
        assert_eq!(wins[1].items.len(), 3);
        assert_eq!(wins[2].items.len(), 1);
        assert_eq!(wins[0].id, "win_0");
        assert_eq!(wins[1].id, "win_3");
    }

    #[test]
    fn extract_json_strips_code_fence() {
        assert_eq!(extract_json("```json\n{\"1\":\"你好\"}\n```"), "{\"1\":\"你好\"}");
    }

    /// 取消探针生效：已跑完的窗口算数，剩下的**不再跑**，返回「任务已取消」。
    ///
    /// 回归的是"点了停止、后台还在一个窗口接一个窗口地跑 LLM"——旧实现只在**阶段边界**
    /// 检查取消，而 `Agent` 内部从来不查，整段检测（18 个窗口、十几分钟）取消不了。
    #[tokio::test]
    async fn run_windows_stops_when_cancel_probe_turns_true() {
        use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

        #[derive(Clone)]
        struct FakeTask {
            entered: Arc<AtomicUsize>,
            completed: Arc<AtomicUsize>,
        }

        #[async_trait]
        impl AgentTask<AgentOutput> for FakeTask {
            fn name(&self) -> &str {
                "fake"
            }

            fn build_global_context(&self, _items: &[AgentItem]) -> String {
                String::new()
            }

            fn build_window_context(
                &self,
                _items: &[AgentItem],
                _window: &[AgentItem],
                _global: &str,
            ) -> String {
                String::new()
            }

            fn system_prompt(&self) -> String {
                String::new()
            }

            fn user_prompt(
                &self,
                _global: &str,
                _window_context: &str,
                _window: &[AgentItem],
            ) -> String {
                String::new()
            }

            fn parse_output(&self, content: &str, _window: &[AgentItem]) -> Result<AgentOutput> {
                Ok(AgentOutput { index: content.parse()?, text: String::new() })
            }

            async fn process_window(
                &self,
                _all_items: &[AgentItem],
                window: &AgentWindow<AgentOutput>,
                _global: &str,
                _llm_cfg: &LlmConfig,
            ) -> Result<AgentOutput> {
                // 第一个窗口立刻完成；其余模拟"在飞的 LLM 调用"：一直挂着，直到被 abort
                if self.entered.fetch_add(1, Ordering::SeqCst) > 0 {
                    tokio::time::sleep(std::time::Duration::from_secs(600)).await;
                }
                self.completed.fetch_add(1, Ordering::SeqCst);
                Ok(AgentOutput { index: window.items[0].index, text: "done".into() })
            }

            fn save_windows(
                &self,
                windows: &[AgentWindow<AgentOutput>],
            ) -> Result<serde_json::Value> {
                Ok(serde_json::to_value(windows)?)
            }

            fn load_windows(
                &self,
                value: serde_json::Value,
            ) -> Result<Vec<AgentWindow<AgentOutput>>> {
                Ok(serde_json::from_value(value)?)
            }
        }

        let items: Vec<AgentItem> = (1..=4)
            .map(|i| AgentItem {
                index: i,
                start: i as f64,
                end: i as f64 + 1.0,
                text: format!("seg{i}"),
            })
            .collect();

        let entered = Arc::new(AtomicUsize::new(0));
        let completed = Arc::new(AtomicUsize::new(0));
        let task = FakeTask {
            entered: entered.clone(),
            completed: completed.clone(),
        };
        // 第一个窗口一跑完就当作"用户点了停止"
        let cancelled = Arc::new(AtomicBool::new(false));
        let probe_flag = cancelled.clone();
        let probe_completed = completed.clone();
        let agent = Agent::new(LlmConfig::default())
            .with_window_size(1)
            .with_max_concurrent(1)
            .with_cancel(Arc::new(move || {
                if probe_completed.load(Ordering::SeqCst) >= 1 {
                    probe_flag.store(true, Ordering::SeqCst);
                }
                probe_flag.load(Ordering::SeqCst)
            }));

        let windows = agent.build_windows(&items);
        let err = agent.run_windows(task, windows, None, None).await.unwrap_err();

        assert!(err.to_string().contains("任务已取消"), "错误应是取消而不是失败: {err}");
        assert_eq!(completed.load(Ordering::SeqCst), 1, "取消后剩下的窗口不该再跑完");
    }

    #[tokio::test]
    async fn run_windows_resumes_from_checkpoint_file() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        #[derive(Clone)]
        struct FakeTask {
            calls: Arc<AtomicUsize>,
        }

        #[async_trait]
        impl AgentTask<AgentOutput> for FakeTask {
            fn name(&self) -> &str {
                "fake"
            }

            fn build_global_context(&self, _items: &[AgentItem]) -> String {
                String::new()
            }

            fn build_window_context(
                &self,
                _items: &[AgentItem],
                _window: &[AgentItem],
                _global: &str,
            ) -> String {
                String::new()
            }

            fn system_prompt(&self) -> String {
                String::new()
            }

            fn user_prompt(
                &self,
                _global: &str,
                _window_context: &str,
                _window: &[AgentItem],
            ) -> String {
                String::new()
            }

            fn parse_output(&self, content: &str, _window: &[AgentItem]) -> Result<AgentOutput> {
                let idx = content.parse::<usize>()?;
                Ok(AgentOutput {
                    index: idx,
                    text: "done".into(),
                })
            }

            async fn process_window(
                &self,
                _all_items: &[AgentItem],
                window: &AgentWindow<AgentOutput>,
                _global: &str,
                _llm_cfg: &LlmConfig,
            ) -> Result<AgentOutput> {
                self.calls.fetch_add(1, Ordering::SeqCst);
                Ok(AgentOutput {
                    index: window.items[0].index,
                    text: format!("processed_{}", window.id),
                })
            }

            fn save_windows(
                &self,
                windows: &[AgentWindow<AgentOutput>],
            ) -> Result<serde_json::Value> {
                Ok(serde_json::to_value(windows)?)
            }

            fn load_windows(
                &self,
                value: serde_json::Value,
            ) -> Result<Vec<AgentWindow<AgentOutput>>> {
                Ok(serde_json::from_value(value)?)
            }
        }

        let items = vec![
            AgentItem { index: 1, start: 0.0, end: 1.0, text: "a".into() },
            AgentItem { index: 2, start: 1.0, end: 2.0, text: "b".into() },
        ];
        let dir = std::env::temp_dir().join(format!("vtrim_agent_test_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let cp = dir.join("checkpoint.json");

        // 第一次运行：两个窗口都处理，checkpoint 写入全部
        let calls = Arc::new(AtomicUsize::new(0));
        let task = FakeTask { calls: calls.clone() };
        let agent = Agent::new(LlmConfig::default()).with_window_size(1);
        let windows = agent.build_windows(&items);
        let _ = agent
            .run_windows(task, windows, Some(&cp), None)
            .await
            .unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        assert!(cp.exists());

        // 第二次运行：从 checkpoint 恢复，应跳过所有已完成窗口
        let calls2 = Arc::new(AtomicUsize::new(0));
        let task2 = FakeTask { calls: calls2.clone() };
        let windows2 = agent.build_windows(&items);
        let result2 = agent
            .run_windows(task2, windows2, Some(&cp), None)
            .await
            .unwrap();
        assert_eq!(calls2.load(Ordering::SeqCst), 0);
        assert_eq!(result2.len(), 2);
        assert_eq!(result2[0].output.as_ref().unwrap().text, "processed_win_0");
        assert_eq!(result2[1].output.as_ref().unwrap().text, "processed_win_1");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn run_windows_skips_completed_checkpoint_windows() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        #[derive(Clone)]
        struct FakeTask {
            calls: Arc<AtomicUsize>,
        }

        #[async_trait]
        impl AgentTask<AgentOutput> for FakeTask {
            fn name(&self) -> &str {
                "fake"
            }

            fn build_global_context(&self, _items: &[AgentItem]) -> String {
                String::new()
            }

            fn build_window_context(
                &self,
                _items: &[AgentItem],
                _window: &[AgentItem],
                _global: &str,
            ) -> String {
                String::new()
            }

            fn system_prompt(&self) -> String {
                String::new()
            }

            fn user_prompt(
                &self,
                _global: &str,
                _window_context: &str,
                _window: &[AgentItem],
            ) -> String {
                String::new()
            }

            fn parse_output(&self, content: &str, _window: &[AgentItem]) -> Result<AgentOutput> {
                let idx = content.parse::<usize>()?;
                Ok(AgentOutput {
                    index: idx,
                    text: "done".into(),
                })
            }

            async fn process_window(
                &self,
                _all_items: &[AgentItem],
                window: &AgentWindow<AgentOutput>,
                _global: &str,
                _llm_cfg: &LlmConfig,
            ) -> Result<AgentOutput> {
                self.calls.fetch_add(1, Ordering::SeqCst);
                Ok(AgentOutput {
                    index: window.items[0].index,
                    text: window.id.clone(),
                })
            }

            fn save_windows(
                &self,
                windows: &[AgentWindow<AgentOutput>],
            ) -> Result<serde_json::Value> {
                Ok(serde_json::to_value(windows)?)
            }

            fn load_windows(
                &self,
                value: serde_json::Value,
            ) -> Result<Vec<AgentWindow<AgentOutput>>> {
                Ok(serde_json::from_value(value)?)
            }
        }

        let items = vec![
            AgentItem { index: 1, start: 0.0, end: 1.0, text: "a".into() },
            AgentItem { index: 2, start: 1.0, end: 2.0, text: "b".into() },
        ];
        // 一个已完成窗口 + 一个未完成窗口
        let windows = vec![
            AgentWindow {
                id: "win_0".into(),
                offset: 0,
                items: vec![items[0].clone()],
                meta: None,
                output: Some(AgentOutput { index: 1, text: "cached".into() }),
            },
            AgentWindow {
                id: "win_1".into(),
                offset: 1,
                items: vec![items[1].clone()],
                meta: None,
                output: None,
            },
        ];
        let calls = Arc::new(AtomicUsize::new(0));
        let task = FakeTask { calls: calls.clone() };
        let agent = Agent::new(LlmConfig::default());
        let result = agent.run_windows(task, windows, None, None).await.unwrap();
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].output.as_ref().unwrap().text, "cached");
        assert_eq!(result[1].output.as_ref().unwrap().text, "win_1");
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn fingerprint_is_stable_and_sensitive() {
        // 同一组片段 → 同一指纹（跨进程稳定，才能当 checkpoint 的失效判据）
        assert_eq!(fingerprint(&["a", "b"]), fingerprint(&["a", "b"]));
        // 任一片段变化 → 指纹变化
        assert_ne!(fingerprint(&["a", "b"]), fingerprint(&["a", "c"]));
        // 分隔符：不能把 ("ab","c") 和 ("a","bc") 当成同一套提示词
        assert_ne!(fingerprint(&["ab", "c"]), fingerprint(&["a", "bc"]));
        // 空集也有确定的指纹（不会退化成空串而悄悄跳过失效判断）
        assert!(!fingerprint(&[]).is_empty());
    }
}
