use std::path::Path;

use anyhow::Result;
use async_trait::async_trait;
use serde_json::Value;

use crate::parse::srt::Subtitle;
use crate::project::{Event, Segment};
use vtrim_core::config::LlmConfig;
use vtrim_core::llm::agent::{
    extract_json_array, AgentItem, AgentProgress, AgentTask, AgentWindow,
};
use vtrim_core::llm::LlmClient;

/// Step2 逐段事件生成任务。
///
/// 每个 `AgentWindow` 对应一个 Segment：
/// - `window.id` = `seg_{index}`（checkpoint 稳定 ID）
/// - `window.meta` = `{"segment_index": i, "candidate_text": "..."}` 等附加上下文
/// - `window.items` = 该段 ±3s 内的字幕（转成 `AgentItem`）
///
/// 该任务由 core `Agent::run_windows` 执行，获得并发、checkpoint、进度与错误容忍。
pub struct EventGenTask {
    segments: Vec<Segment>,
    subtitles: Vec<Subtitle>,
    candidate_texts: Vec<String>,
    sticker_source: Vec<String>,
    sfx_blacklist: Vec<String>,
    local_sticker_text: String,
}

impl EventGenTask {
    pub fn new(
        segments: Vec<Segment>,
        subtitles: Vec<Subtitle>,
        candidate_texts: Vec<String>,
        sticker_source: Vec<String>,
        sfx_blacklist: Vec<String>,
        local_sticker_text: String,
    ) -> Self {
        Self {
            segments,
            subtitles,
            candidate_texts,
            sticker_source,
            sfx_blacklist,
            local_sticker_text,
        }
    }

    /// 构建一个 segment 一个窗口的 `AgentWindow` 列表。
    pub fn build_windows<O>(&self) -> Vec<AgentWindow<O>> {
        self.segments
            .iter()
            .enumerate()
            .map(|(idx, seg)| {
                let items: Vec<AgentItem> = self
                    .subtitles
                    .iter()
                    .filter(|s| s.end >= seg.start - 3.0 && s.start <= seg.end + 3.0)
                    .enumerate()
                    .map(|(i, s)| AgentItem {
                        index: i + 1,
                        start: s.start,
                        end: s.end,
                        text: s.text.clone(),
                    })
                    .collect();
                AgentWindow {
                    id: format!("seg_{idx}"),
                    offset: idx,
                    items,
                    meta: Some(serde_json::json!({
                        "segment_index": idx,
                        "candidate_text": self.candidate_texts.get(idx).cloned().unwrap_or_default(),
                    })),
                    output: None,
                }
            })
            .collect()
    }

    /// 从已完成窗口恢复 `segments.events`（供调用方写回）。
    pub fn apply_outputs(&self, windows: &[AgentWindow<Vec<Event>>], segments: &mut Vec<Segment>) {
        for w in windows {
            let Some(output) = &w.output else { continue };
            let Some(meta) = &w.meta else { continue };
            let Some(idx) = meta.get("segment_index").and_then(|v| v.as_u64()) else {
                continue;
            };
            if let Some(seg) = segments.get_mut(idx as usize) {
                seg.events = output.clone();
            }
        }
    }
}

#[async_trait]
impl AgentTask<Vec<Event>> for EventGenTask {
    fn name(&self) -> &str {
        "精修段事件"
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
        crate::llm::segment::build_events_system_prompt(
            &self.sticker_source,
            &self.sfx_blacklist,
            &self.local_sticker_text,
        )
    }

    fn user_prompt(&self, _global: &str, _window_context: &str, _window: &[AgentItem]) -> String {
        // process_window 已覆写，本方法不会被调用；返回空占位。
        String::new()
    }

    fn parse_output(&self, content: &str, _window: &[AgentItem]) -> Result<Vec<Event>> {
        let json = extract_json_array(content)?;
        let json = crate::llm::segment::normalize_segments_json(&json);
        let events: Vec<Event> = serde_json::from_str(&json)?;
        Ok(events)
    }

    async fn process_window(
        &self,
        _all_items: &[AgentItem],
        window: &AgentWindow<Vec<Event>>,
        _global: &str,
        llm_cfg: &LlmConfig,
    ) -> Result<Vec<Event>> {
        let seg_index = window
            .meta
            .as_ref()
            .and_then(|m| m.get("segment_index"))
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as usize;
        let segment = &self.segments[seg_index];
        let candidate_text = window
            .meta
            .as_ref()
            .and_then(|m| m.get("candidate_text"))
            .and_then(|v| v.as_str())
            .unwrap_or("");

        let client = LlmClient::from_config(llm_cfg)?
            .with_timeout(120)
            .with_thinking(Some("disabled"));
        let system_prompt = self.system_prompt();
        let user_prompt =
            crate::llm::segment::build_events_user_prompt(segment, &self.subtitles, candidate_text);
        let response = client.chat(&system_prompt, &user_prompt).await?;
        self.parse_output(&response, &window.items)
    }

    fn save_windows(&self, windows: &[AgentWindow<Vec<Event>>]) -> Result<Value> {
        Ok(serde_json::to_value(windows)?)
    }

    fn load_windows(&self, value: Value) -> Result<Vec<AgentWindow<Vec<Event>>>> {
        Ok(serde_json::from_value(value)?)
    }
}

/// 运行 Step2 逐段事件生成（封装 core Agent）。
///
/// 返回成功生成的段数；失败段保留空 events，由 process 兜底。
pub async fn run_event_agent(
    segments: &mut [Segment],
    subtitles: &[Subtitle],
    candidate_texts: Vec<String>,
    sticker_source: &[String],
    sfx_blacklist: &[String],
    local_sticker_text: String,
    llm_cfg: &LlmConfig,
    checkpoint_path: Option<&Path>,
) -> anyhow::Result<usize> {
    let task = EventGenTask::new(
        segments.to_vec(),
        subtitles.to_vec(),
        candidate_texts,
        sticker_source.to_vec(),
        sfx_blacklist.to_vec(),
        local_sticker_text,
    );
    let windows = task.build_windows::<Vec<Event>>();
    let agent = vtrim_core::llm::agent::Agent::new(llm_cfg.clone()).with_max_concurrent(3);

    let completed = agent
        .run_windows(task, windows, checkpoint_path, None::<tokio::sync::mpsc::Sender<AgentProgress<Vec<Event>>>>)
        .await?;

    let mut ok = 0usize;
    for w in &completed {
        if w.output.is_some() {
            ok += 1;
        }
    }
    // 将输出写回调用方持有的 segments
    // 因为 run_event_agent 拿的是 &mut [Segment]，需要先构造一个临时 Vec 再回填
    let mut segs = segments.to_vec();
    let task_for_apply = EventGenTask::new(
        segs.clone(),
        subtitles.to_vec(),
        Vec::new(),
        Vec::new(),
        Vec::new(),
        String::new(),
    );
    task_for_apply.apply_outputs(&completed, &mut segs);
    for (dst, src) in segments.iter_mut().zip(segs.iter()) {
        *dst = src.clone();
    }

    Ok(ok)
}

/// 一个待精挑的贴纸事件上下文。
#[derive(Debug, Clone)]
pub struct StickerChoiceContext {
    /// 事件所在 segment 索引
    pub segment_index: usize,
    /// 事件在 segment.events 中的索引
    pub event_index: usize,
    /// 事件时间
    pub time: f64,
    /// 该事件附近的字幕文本（用于 LLM 判断）
    pub context_text: String,
}

/// Step2b：按 tag 批量精挑具体贴纸文件。
///
/// 对同一 `(包名, tag)` 下的多个 sticker 事件，一次 LLM 调用为每个事件选一个文件。
/// 返回 `(segment_index, event_index, 文件名 stem)` 列表。
pub async fn choose_sticker_files_batch(
    llm_cfg: &LlmConfig,
    package_name: &str,
    tag: &str,
    candidates: &[String],
    contexts: &[StickerChoiceContext],
) -> anyhow::Result<Vec<(usize, usize, String)>> {
    use vtrim_core::llm::agent::extract_json_object;

    if contexts.is_empty() || candidates.is_empty() {
        return Ok(vec![]);
    }

    let client = LlmClient::from_config(llm_cfg)?
        .with_timeout(120)
        .with_thinking(Some("disabled"));

    let system = format!(
        "你是直播切片贴纸选择助手。贴纸包「{}」的 tag「{}」下有以下候选文件（必须从其中选择）：\n{}",
        package_name,
        tag,
        candidates.join("\n")
    );

    let mut prompt = String::from("为以下每个贴纸时刻从候选文件中选择最贴合的一个。\n\n");
    for (i, ctx) in contexts.iter().enumerate() {
        prompt.push_str(&format!(
            "[{}] time={:.1}s\n字幕上下文: {}\n\n",
            i,
            ctx.time,
            if ctx.context_text.is_empty() { "(无)" } else { &ctx.context_text }
        ));
    }
    prompt.push_str("输出 JSON 对象：{\"0\":\"文件名.gif\",\"1\":\"文件名.gif\",...}，key 为上面编号。只输出 JSON。");

    let resp = client.chat(&system, &prompt).await?;
    let json = extract_json_object(&resp)?;
    let map: serde_json::Map<String, serde_json::Value> = serde_json::from_str(&json)?;

    let mut out = Vec::new();
    for (i, ctx) in contexts.iter().enumerate() {
        let chosen = map
            .get(&i.to_string())
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        // 必须真实存在于候选列表，否则跳过（由调用方 fallback）
        if let Some(file) = chosen {
            if candidates.iter().any(|c| c == &file) {
                let stem = Path::new(&file)
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .map(|s| s.to_string())
                    .unwrap_or(file);
                out.push((ctx.segment_index, ctx.event_index, stem));
            }
        }
    }
    Ok(out)
}
