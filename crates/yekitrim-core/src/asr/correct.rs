//! asr::correct：LLM 订正转写文本（分窗走 `Agent`，带 checkpoint 续传）。
//!
//! 入口：`correct_transcript`、`correct_transcript_with_checkpoint`、`DEFAULT_SYSTEM_PROMPT`、`DEFAULT_USER_PROMPT`。
//! 边界：订正稿是**普通素材**，走 `bind_derived_material`、不占 ASR 槽；改内建 prompt 对 `.yekitrim/config.toml` 里的副本无效 —— 见 `crates/yekitrim-core/src/asr/AGENTS.md`

use std::path::Path;

use anyhow::{Context, Result};
use async_trait::async_trait;

use crate::config::{CorrectConfig};
use crate::danmaku::DanmakuEntry;
use yekitrim_llm::agent::{
    extract_json, items_to_json, Agent, AgentItem, AgentOutput, AgentProgress, AgentTask,
    AgentWindow,
};
use crate::types::TranscriptSegment;

pub const DEFAULT_SYSTEM_PROMPT: &str = "\
你是直播字幕订正助手。只修 ASR 错别字、人名/游戏/歌曲/梗的正确写法，\
保留口语语气词和重复，不要润色书面化。只输出 JSON，不要解释。";

pub const DEFAULT_USER_PROMPT: &str = "\
订正以下转录中的 ASR 错误。规则：
- 优先使用热词/弹幕/上下文中的正确写法
- 只改明显错误的字词，不重写句子
- 保留：啊/哦/嗯/嘛/呗/啦、重复、口头禅
- 歌词/外语片段不确定就不改

{global_context}

{window_context}

待订正 JSON：
{segments}

只输出有修正的 JSON 对象，键为 index，值为修正后文本：
{\"3\": \"修正文本\"}
无修正输出 {}
不要 markdown。";

fn effective_user_prompt(correct: &CorrectConfig) -> String {
    if correct.prompt.is_empty() || correct.prompt.contains("JSON 数组格式返回修正结果") {
        DEFAULT_USER_PROMPT.to_string()
    } else {
        correct.prompt.clone()
    }
}

fn effective_system_prompt(correct: &CorrectConfig) -> String {
    if correct.system_prompt.is_empty() || correct.system_prompt.contains("只以 JSON 格式输出修正结果") {
        DEFAULT_SYSTEM_PROMPT.to_string()
    } else {
        correct.system_prompt.clone()
    }
}

/// 订正任务：实现 AgentTask。
struct CorrectTask {
    danmaku: Vec<DanmakuEntry>,
    correct: CorrectConfig,
    /// 项目根目录，只用于用量记账归因。
    project: Option<String>,
}

#[async_trait]
impl AgentTask<Vec<AgentOutput>> for CorrectTask {
    fn name(&self) -> &str {
        "字幕订正"
    }

    fn usage_context(&self) -> yekitrim_llm::usage::UsageContext {
        yekitrim_llm::usage::UsageContext::new(self.name()).project(self.project.clone())
    }

    fn build_global_context(&self, items: &[AgentItem]) -> String {
        let _ = items;
        let mut global = String::from("--- 全局参考 ---\n");

        // 自定义热词（最高优先级）
        if !self.correct.hotwords.is_empty() {
            global.push_str(&format!("热词(优先使用正确写法): {}\n", self.correct.hotwords.join("、")));
        }

        // 弹幕高频词（取前 20，作为线索）
        let mut dm_freq: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
        for d in &self.danmaku {
            for word in d.text.split(|c: char| !c.is_alphanumeric() && c != '_') {
                let w = word.trim();
                if w.chars().count() >= 2 && w.chars().count() <= 12 {
                    *dm_freq.entry(w.to_string()).or_default() += 1;
                }
            }
        }
        let mut dm_words: Vec<(String, usize)> = dm_freq.into_iter().collect();
        dm_words.sort_by(|a, b| b.1.cmp(&a.1));
        let top_dm: Vec<String> = dm_words.iter().take(20).map(|(w, _)| w.clone()).collect();
        if !top_dm.is_empty() {
            global.push_str(&format!("弹幕高频词: {}\n", top_dm.join("、")));
        }

        global
    }

    fn build_window_context(&self, items: &[AgentItem], window: &[AgentItem], global: &str) -> String {
        let _ = items;
        let mut ctx = String::new();

        // 前文/后文上下文（window 已包含前后 3+2 条）
        let ctx_items: Vec<&AgentItem> = window.iter().collect();
        if !ctx_items.is_empty() {
            ctx.push_str("\n--- 局部上下文（仅作参考，无需修正） ---\n");
            for it in ctx_items {
                ctx.push_str(&format!("[{:.1}-{:.1}] {}\n", it.start, it.end, it.text));
            }
        }

        // 弹幕上下文（与当前窗口时间重叠，最多 15 条）
        if !self.danmaku.is_empty() {
            let start_time = window.first().map(|s| s.start).unwrap_or(0.0);
            let end_time = window.last().map(|s| s.end).unwrap_or(0.0);
            let mut dm = String::from("\n--- 当前时段弹幕（供参考，弹幕文字不会出错） ---\n");
            let mut count = 0;
            for d in &self.danmaku {
                if count >= 15 {
                    break;
                }
                if d.start < end_time && d.end > start_time {
                    dm.push_str(&format!("[{:.1}-{:.1}] {}\n", d.start, d.end, d.text));
                    count += 1;
                }
            }
            if count > 0 {
                ctx.push_str(&dm);
            }
        }

        let _ = global;
        ctx
    }

    fn system_prompt(&self) -> String {
        effective_system_prompt(&self.correct)
    }

    fn user_prompt(&self, global: &str, window_context: &str, window: &[AgentItem]) -> String {
        let user_prompt = effective_user_prompt(&self.correct);
        // 这里以前是 `.expect("序列化失败")`：一个窗口的序列化失败会直接 panic 掉整条订正任务。
        // 退化成空数组即可 —— prompt 少一段窗口上下文，比整个任务失败好。
        let seg_json = items_to_json(window).unwrap_or_else(|e| {
            tracing::warn!("订正窗口序列化失败（退化为空数组）: {e:#}");
            serde_json::Value::Array(Vec::new())
        });
        let seg_text = serde_json::to_string_pretty(&seg_json).unwrap_or_default();
        let combined_context = format!("{}{}", global, window_context);

        user_prompt
            .replace("{global_context}", global)
            .replace("{window_context}", window_context)
            // 兼容旧模板的 {context}
            .replace("{context}", &combined_context)
            .replace("{segments}", &seg_text)
    }

    fn parse_output(&self, content: &str, window: &[AgentItem]) -> Result<Vec<AgentOutput>> {
        let json_str = extract_json(content);
        let value: serde_json::Value = serde_json::from_str(json_str)
            .context("订正 JSON 格式无效")?;

        let mut outputs = Vec::new();
        match value {
            serde_json::Value::Object(map) => {
                for (key, val) in map {
                    let Ok(idx) = key.parse::<usize>() else { continue };
                    if let Some(text) = val.as_str() {
                        // 只保留窗口内存在的 index
                        if window.iter().any(|w| w.index == idx) && !text.is_empty() {
                            outputs.push(AgentOutput { index: idx, text: text.to_string() });
                        }
                    }
                }
            }
            serde_json::Value::Array(arr) => {
                for item in arr {
                    let idx = item.get("index").and_then(|v| v.as_u64()).map(|n| n as usize);
                    let text = item.get("text").and_then(|v| v.as_str());
                    if let (Some(idx), Some(text)) = (idx, text) {
                        if window.iter().any(|w| w.index == idx) && !text.is_empty() {
                            outputs.push(AgentOutput { index: idx, text: text.to_string() });
                        }
                    }
                }
            }
            _ => anyhow::bail!("订正结果不是 JSON 对象或数组"),
        }
        Ok(outputs)
    }

    fn save_windows(&self, windows: &[AgentWindow<Vec<AgentOutput>>]) -> Result<serde_json::Value> {
        Ok(serde_json::to_value(windows)?)
    }

    fn load_windows(&self, value: serde_json::Value) -> Result<Vec<AgentWindow<Vec<AgentOutput>>>> {
        let windows: Vec<AgentWindow<Vec<AgentOutput>>> = serde_json::from_value(value)?;
        Ok(windows)
    }
}

/// 订正转录文本。
///
/// `checkpoint_path` 非空时支持断点续传：已存在则跳过已完成窗口，运行中每完成一窗立即保存。
/// `progress_tx` 非空时每完成一窗发送进度（边处理边输出）。
/// `cancel` 非空时 Agent 在**窗口边界**轮询它（`Agent::with_cancel`）：不传 = 跑完为止。
/// `project` 只用于用量记账归因（填项目根目录）。
pub async fn correct_transcript_with_checkpoint(
    segments: &[TranscriptSegment],
    danmaku: &[DanmakuEntry],
    llm: &crate::config::LlmConfigV4,
    correct: &CorrectConfig,
    project: Option<&Path>,
    checkpoint_path: Option<&Path>,
    progress_tx: Option<tokio::sync::mpsc::Sender<AgentProgress<Vec<AgentOutput>>>>,
    cancel: Option<yekitrim_llm::agent::CancelProbe>,
) -> Result<Vec<TranscriptSegment>> {
    if segments.is_empty() {
        return Ok(Vec::new());
    }

    let items: Vec<AgentItem> = segments
        .iter()
        .enumerate()
        .map(|(i, seg)| AgentItem {
            index: i + 1,
            start: seg.start,
            end: seg.end,
            text: seg.text.clone(),
        })
        .collect();

    let task = CorrectTask {
        danmaku: danmaku.to_vec(),
        correct: correct.clone(),
        project: project.map(crate::path::display),
    };

    // 链路解析：订正走 `[llm.links.correct]`（Agent 的 llm 会传给默认的 `process_window` 建客户端）
    // ⚠️ S4：窗口 / 并发从 `for_link` 的产物里取（`LinkedLlm::run()`）—— 它们已经从 config.toml 的
    //    `[correct] window_size/max_concurrent` 搬进 `[llm.links.correct.run]`。别再读 `correct.*`。
    let llm = llm.for_link(crate::config::LlmLink::Correct);
    let run = llm.run();
    // 库内 indicatif 进度条关掉：调用方（CLI/server）自带进度面，窗口进度经 progress_tx 桥接出去。
    let agent = Agent::new(llm.clone())
        .with_window_size(run.window_size)
        .with_max_concurrent(run.max_concurrent)
        .without_internal_progress();
    // ⚠️ 探针必须真的交给 Agent：只在阶段边界查一次等于"用户点了停止、后台仍把窗口一个个跑完"。
    // 订正是分钟级链路（窗口多、每次调用几十秒），这条以前漏了 —— 与检测链路同一个坑。
    let agent = match cancel {
        Some(probe) => agent.with_cancel(probe),
        None => agent,
    };

    let windows = agent
        .run_with_checkpoint(task, &items, checkpoint_path, progress_tx)
        .await?;

    let mut result = segments.to_vec();
    for window in windows {
        if let Some(outputs) = window.output {
            for out in outputs {
                if let Some(seg) = result.get_mut(out.index - 1) {
                    seg.text = out.text;
                }
            }
        }
    }
    Ok(result)
}

/// 订正转录文本（不启用断点续传/进度回调的便捷入口）。
pub async fn correct_transcript(
    segments: &[TranscriptSegment],
    danmaku: &[DanmakuEntry],
    llm: &crate::config::LlmConfigV4,
    correct: &CorrectConfig,
) -> Result<Vec<TranscriptSegment>> {
    correct_transcript_with_checkpoint(segments, danmaku, llm, correct, None, None, None, None).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_object_corrections() {
        let task = CorrectTask {
            danmaku: vec![],
            correct: CorrectConfig::default(),
            project: None,
        };
        let window = vec![
            AgentItem { index: 1, start: 0.0, end: 1.0, text: "你好".into() },
            AgentItem { index: 2, start: 1.0, end: 2.0, text: "法环".into() },
        ];
        let out = task.parse_output(r#"{"2":"法环","5":"B站舰长"}"#, &window).unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].index, 2);
        assert_eq!(out[0].text, "法环");
    }

    #[test]
    fn parse_array_corrections() {
        let task = CorrectTask {
            danmaku: vec![],
            correct: CorrectConfig::default(),
            project: None,
        };
        let window = vec![
            AgentItem { index: 3, start: 0.0, end: 1.0, text: "破房了".into() },
            AgentItem { index: 7, start: 1.0, end: 2.0, text: "寄".into() },
        ];
        let out = task.parse_output(
            r#"[{"index":3,"text":"破防了"},{"index":7,"text":"寄！"}]"#,
            &window,
        )
        .unwrap();
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].text, "破防了");
        assert_eq!(out[1].text, "寄！");
    }

    #[test]
    fn parse_empty_object() {
        let task = CorrectTask {
            danmaku: vec![],
            correct: CorrectConfig::default(),
            project: None,
        };
        let window = vec![AgentItem { index: 1, start: 0.0, end: 1.0, text: "你好".into() }];
        let out = task.parse_output("{}", &window).unwrap();
        assert!(out.is_empty());
    }

    #[test]
    fn extract_json_strips_code_fence() {
        assert_eq!(extract_json("```json\n{\"1\":\"你好\"}\n```"), "{\"1\":\"你好\"}");
    }
}

#[cfg(test)]
mod checkpoint_tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    /// 极简假 OpenAI 服务：每个请求回一条固定订正结果，并计数。
    ///
    /// `Arc<TcpListener>` 由调用方持到测试结束（否则端口可能被同期别的 listener 拿走，
    /// 表现为偶发的「连上了但没人应答」—— 与 `yekitrim-llm` 的 fake_server 同一个坑）。
    async fn fake_correct_server(hits: Arc<AtomicUsize>) -> (Arc<tokio::net::TcpListener>, String) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("绑定本地端口");
        let addr = listener.local_addr().expect("取地址");
        let listener = Arc::new(listener);
        let serve = listener.clone();
        tokio::spawn(async move {
            loop {
                let Ok((mut sock, _)) = serve.accept().await else { break };
                hits.fetch_add(1, Ordering::SeqCst);
                let mut buf = [0u8; 8192];
                let _ = sock.read(&mut buf).await;
                let body = serde_json::json!({
                    "choices": [{ "message": { "content": serde_json::json!({ "1": "甲" }).to_string() } }],
                    "usage": { "prompt_tokens": 10, "completion_tokens": 5 }
                })
                .to_string();
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = sock.write_all(resp.as_bytes()).await;
                let _ = sock.shutdown().await;
            }
        });
        (listener, format!("http://{addr}/v1"))
    }

    /// **回归钉子**：订正的断点续传必须端到端真的生效。
    ///
    /// 两个失败面各钉一半：① 传了路径就**必须写盘**（`Agent::run_windows` 的契约）；
    /// ② 同一份断点再跑一次**一个请求都不发**（续传的意义）。
    /// 上层（`core_pipeline` / `service::correct`）曾经把路径算成 `if exists { Some } else { None }`，
    /// 于是首次运行传 None、文件永不出现 —— 这条用例把「传了就必须写」这一半钉死。
    #[tokio::test]
    async fn correction_writes_checkpoint_and_resumes_without_calling_again() {
        let hits = Arc::new(AtomicUsize::new(0));
        let (_srv, base) = fake_correct_server(hits.clone()).await;

        // v4：接口/模型/key 都在一份 provider 里（没有顶层 api_key 了）
        let llm = crate::config::LlmConfigV4::single(&base, "test-model", "test-key");
        let correct = CorrectConfig::default();
        let segments = vec![
            TranscriptSegment { start: 0.0, end: 1.0, text: "你好".into(), confidence: 1.0 },
            TranscriptSegment { start: 1.0, end: 2.0, text: "世界".into(), confidence: 1.0 },
        ];

        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let dir = std::env::temp_dir().join(format!(
            "yekitrim-correct-ckpt-{}-{nanos}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).expect("建临时目录");
        let checkpoint = dir.join("correct.checkpoint.json");

        // ① 首跑：必须写断点（两个段落在一个窗口 ⇒ 恰好一次调用）
        let first = correct_transcript_with_checkpoint(
            &segments,
            &[],
            &llm,
            &correct,
            None,
            Some(&checkpoint),
            None,
            None,
        )
        .await
        .expect("首跑应成功");
        assert!(
            checkpoint.exists(),
            "传了断点路径就必须写盘: {}",
            checkpoint.display()
        );
        assert_eq!(hits.load(Ordering::SeqCst), 1, "一个窗口 = 一次调用");
        assert_eq!(first[0].text, "甲", "订正结果要落到第 1 条");

        // ② 续传：同一份断点再跑 ⇒ 一个请求都不发
        let second = correct_transcript_with_checkpoint(
            &segments,
            &[],
            &llm,
            &correct,
            None,
            Some(&checkpoint),
            None,
            None,
        )
        .await
        .expect("续传应成功");
        assert_eq!(
            hits.load(Ordering::SeqCst),
            1,
            "断点命中就不许再发请求（以前这里会翻倍到 2）"
        );
        assert_eq!(second[0].text, "甲", "续传结果与首跑一致");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
