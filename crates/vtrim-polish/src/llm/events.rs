//! polish::llm::events：Step 2 逐段事件生成（走 `vtrim_core::llm::agent::Agent` + checkpoint 续传 + 贴纸批量精挑）。
//!
//! 入口：`EventGenTask::{new, with_project, build_windows, apply_outputs}`、`run_event_agent`、`choose_sticker_files_batch`、`StickerChoiceContext`。
//! 边界：窗口 id 必须带时间段，否则重新分段后会命中旧窗口；`output == None`（失败）与 `Some([])`（空结果）都跳过硬、保留原有事件 —— 见 `crates/vtrim-polish/src/llm/AGENTS.md`

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

use super::status::SegmentGenStatus;

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
    /// 切片目录，只用于用量记账归因。
    project: Option<String>,
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
            project: None,
        }
    }

    /// 用量记账归因（链路名 + 切片目录）。
    pub fn with_project(mut self, project: Option<&std::path::Path>) -> Self {
        self.project = project.map(vtrim_core::path::display);
        self
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
                    // ⚠️ id 里**必须带段时间区间**：checkpoint 的命中条件是 `id && items`，
                    // 而 `build_windows` 是按**下标**命名的（seg_0/seg_1…）。重新分段之后
                    // 「新的 seg_1」与「旧的 seg_1」是完全不同的时间段，可它们的 ±3s 字幕窗口
                    // 有可能一字不差 —— 只靠 items 会**误命中**：把上一轮给别的时间段生成的事件
                    // 套到这一段上（用户报过「续传之后事件乱了/显示不正常」）。
                    // 带上区间之后，重新分段必然失配、老老实实重算。
                    id: format!("seg_{idx}_{:.2}-{:.2}", seg.start, seg.end),
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
    ///
    /// ⚠️ **产不出事件的窗口一律不覆盖**（2026-09-20 用户报「续传把事件全删了、只剩一个 auto」）：
    /// - `output == None`（429 / 超时 / 解析失败）→ 原来就没有东西可写，跳过；
    /// - `output == Some(空数组)`（模型回了个 `[]`）→ **保留段内原有事件**。
    ///
    /// 这条以前是 `seg.events = output.clone()` 无条件覆盖：一次限流就能把整段已有事件抹成 0，
    /// 然后 process 兜底补一条整段长的中景 —— 界面上就是「事件全没了、只剩一个 auto」，
    /// 而且**看起来像是模型写得少**，完全查不出是调用失败。空结果是"这一次没生成"，
    /// 不是"这一段应该有 0 条事件"，所以不能拿它覆盖。
    pub fn apply_outputs(&self, windows: &[AgentWindow<Vec<Event>>], segments: &mut Vec<Segment>) {
        for w in windows {
            let Some(output) = &w.output else { continue };
            let Some(meta) = &w.meta else { continue };
            let Some(idx) = meta.get("segment_index").and_then(|v| v.as_u64()) else {
                continue;
            };
            let Some(seg) = segments.get_mut(idx as usize) else { continue };
            if output.is_empty() {
                if !seg.events.is_empty() {
                    tracing::warn!(
                        "段 {idx}：LLM 返回空事件，保留原有 {} 条（空结果不覆盖）",
                        seg.events.len()
                    );
                }
                continue;
            }
            seg.events = output.clone();
        }
    }
}

#[async_trait]
impl AgentTask<Vec<Event>> for EventGenTask {
    fn name(&self) -> &str {
        "精修段事件"
    }

    fn usage_context(&self) -> vtrim_core::llm::usage::UsageContext {
        vtrim_core::llm::usage::UsageContext::new(self.name()).project(self.project.clone())
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

    /// system prompt 的指纹 → 折进窗口 id：改了 prompt 模板 / 贴纸来源 / 音效屏蔽之后
    /// 重跑，checkpoint 里的旧窗口会失配重算，而不是拿回上一版的结果。
    fn prompt_fingerprint(&self) -> String {
        vtrim_core::llm::agent::fingerprint(&[&self.system_prompt()])
    }

    fn user_prompt(&self, _global: &str, _window_context: &str, _window: &[AgentItem]) -> String {
        // process_window 已覆写，本方法不会被调用；返回空占位。
        String::new()
    }

    fn parse_output(&self, content: &str, _window: &[AgentItem]) -> Result<Vec<Event>> {
        let json = extract_json_array(content)?;
        let json = crate::llm::segment::normalize_segments_json(&json);
        // 模型偶尔仍按老字段写 `duration` → 统一升级成 `end`（与老配置同一条路）
        let mut value: Value = serde_json::from_str(&json)?;
        crate::project::upgrade_event_times(&mut value);
        let events: Vec<Event> = serde_json::from_value(value)?;
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
            .with_usage_context(self.usage_context());
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
        // 老 checkpoint 里的事件还是 `duration` 字段 → 升级后再解析
        let mut value = value;
        crate::project::upgrade_event_times(&mut value);
        Ok(serde_json::from_value(value)?)
    }
}

/// 运行 Step2 逐段事件生成（封装 core Agent）。
///
/// 返回**逐段结果**（`SegmentGenStatus`）：哪一段产出了事件、哪一段没有以及原因。
/// 失败段保留空 events（或原有事件），由 process 兜底；但失败**必须被调用方记录下来**
/// —— 以前只返回一个成功计数，界面完全看不出一段是"模型写得少"还是"调用失败了"。
pub async fn run_event_agent(
    segments: &mut [Segment],
    subtitles: &[Subtitle],
    candidate_texts: Vec<String>,
    sticker_source: &[String],
    sfx_blacklist: &[String],
    local_sticker_text: String,
    llm_cfg: &LlmConfig,
    project: Option<&Path>,
    checkpoint_path: Option<&Path>,
) -> anyhow::Result<Vec<SegmentGenStatus>> {
    // 链路解析：精修这条链路（见 [llm.overrides.polish]）。
    // 幂等：从 `run_llm_pipeline` 传进来的可能已经是解析过的配置。
    let linked = llm_cfg.for_link(vtrim_core::config::LlmLink::Polish);
    let llm_cfg = &linked;
    let task = EventGenTask::new(
        segments.to_vec(),
        subtitles.to_vec(),
        candidate_texts,
        sticker_source.to_vec(),
        sfx_blacklist.to_vec(),
        local_sticker_text,
    )
    .with_project(project);
    let windows = task.build_windows::<Vec<Event>>();
    // 并发跟 .vtrim/llm.toml 的 [llm] max_concurrent（精修没有自己的并发配置项）
    let agent = vtrim_core::llm::agent::Agent::new(llm_cfg.clone())
        .with_max_concurrent(llm_cfg.concurrency(None));

    let completed = agent
        .run_windows(task, windows, checkpoint_path, None::<tokio::sync::mpsc::Sender<AgentProgress<Vec<Event>>>>)
        .await?;

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

    // 逐段状态：以窗口的 output 为准（None = 调用失败；Some(空) = 模型返回空）
    let mut statuses: Vec<SegmentGenStatus> = completed
        .iter()
        .filter_map(|w| {
            let idx = w
                .meta
                .as_ref()
                .and_then(|m| m.get("segment_index"))
                .and_then(|v| v.as_u64())? as usize;
            let seg = segs.get(idx)?;
            Some(match &w.output {
                Some(v) if !v.is_empty() => SegmentGenStatus::ok(idx, seg.start, seg.end, seg.events.len()),
                Some(_) => SegmentGenStatus::failed(
                    idx, seg.start, seg.end, "模型返回空事件", seg.events.len(),
                ),
                None => SegmentGenStatus::failed(
                    idx, seg.start, seg.end, "LLM 调用失败（429/超时/解析失败，详见任务日志）", seg.events.len(),
                ),
            })
        })
        .collect();
    statuses.sort_by_key(|s| s.index);
    let failed = statuses.iter().filter(|s| !s.ok).count();
    if failed > 0 {
        tracing::warn!("⚠ {} 段没有产出事件（编辑器里会用警示色标出来）", failed);
    }
    Ok(statuses)
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
    project: Option<&std::path::Path>,
    package_name: &str,
    tag: &str,
    candidates: &[String],
    contexts: &[StickerChoiceContext],
) -> anyhow::Result<Vec<(usize, usize, String)>> {
    use vtrim_core::llm::agent::extract_json_object;

    if contexts.is_empty() || candidates.is_empty() {
        return Ok(vec![]);
    }

    // 链路解析：精修这条链路（见 [llm.overrides.polish]）
    let linked = llm_cfg.for_link(vtrim_core::config::LlmLink::Polish);
    let llm_cfg = &linked;

    let client = LlmClient::from_config(llm_cfg)?
        .with_timeout(120)
        .with_usage_context(
            vtrim_core::llm::usage::UsageContext::new("精修贴纸挑选")
                .project(project.map(vtrim_core::path::display)),
        );

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::{Event, Segment};

    fn cam(time: f64, end: f64) -> Event {
        Event::Camera {
            time,
            end: Some(end),
            zoom: "中".into(),
            origin: "50% 50%".into(),
            anchor: Some("liver".into()),
            anchor_to: None,
            to_origin: None,
            cut: false,
            handheld: false,
            manual: false,
        }
    }

    fn seg(start: f64, end: f64, events: Vec<Event>) -> Segment {
        Segment { start, end, fx: "med".into(), note: String::new(), events, extra: Default::default() }
    }

    fn window(idx: usize, output: Option<Vec<Event>>) -> AgentWindow<Vec<Event>> {
        AgentWindow {
            id: format!("seg_{idx}"),
            offset: idx,
            items: vec![],
            meta: Some(serde_json::json!({ "segment_index": idx })),
            output,
        }
    }

    fn task_for(segments: Vec<Segment>) -> EventGenTask {
        EventGenTask::new(segments, vec![], vec![], vec![], vec![], String::new())
    }

    /// **回归钉子**（2026-09-20 用户报「断点续传把所有事件删了，只剩一个 auto」）：
    /// 模型返回空数组是"这一次没生成"，**不是**"这一段应该有 0 条事件" —— 不许覆盖。
    #[test]
    fn empty_output_never_wipes_existing_events() {
        let mut segments = vec![seg(0.0, 10.0, vec![cam(1.0, 5.0)])];
        let w = vec![window(0, Some(vec![]))];
        task_for(segments.clone()).apply_outputs(&w, &mut segments);
        assert_eq!(segments[0].events.len(), 1, "空数组不得清空已有事件");
    }

    /// 调用失败（429/超时/解析失败）留下的 `output = None` 同样一个字都不许动。
    #[test]
    fn failed_window_keeps_existing_events() {
        let mut segments = vec![seg(0.0, 10.0, vec![cam(1.0, 5.0), cam(6.0, 9.0)])];
        let w = vec![window(0, None)];
        task_for(segments.clone()).apply_outputs(&w, &mut segments);
        assert_eq!(segments[0].events.len(), 2, "失败窗口不得清空已有事件");
    }

    /// 正常输出照旧整体替换（生成的结果才是这一段该有的样子）。
    #[test]
    fn non_empty_output_replaces() {
        let mut segments = vec![seg(0.0, 10.0, vec![cam(1.0, 5.0)])];
        let w = vec![window(0, Some(vec![cam(2.0, 4.0), cam(5.0, 8.0)]))];
        task_for(segments.clone()).apply_outputs(&w, &mut segments);
        assert_eq!(segments[0].events.len(), 2);
    }

    /// 窗口 id 必须带**时间段**：重新分段之后，「新的 seg_1」与「旧的 seg_1」可能是完全
    /// 不同的时间区间，而它们的 ±3s 字幕窗口（`items`，checkpoint 的另一个命中条件）
    /// 有可能一字不差 —— 只靠 items 会误命中，把上一轮给别的时间段生成的事件套到这一段上。
    #[test]
    fn window_id_carries_span_so_resegmentation_never_reuses() {
        let subs = vec![Subtitle { index: 1, start: 2.0, end: 3.0, text: "同一句".into(), html: String::new() }];
        let segs = vec![seg(0.0, 10.0, vec![]), seg(0.0, 20.0, vec![])];
        let task = EventGenTask::new(segs, subs, vec![], vec![], vec![], String::new());
        let ws = task.build_windows::<Vec<Event>>();
        assert_eq!(ws[0].items, ws[1].items, "夹具前提：两段看到的字幕窗口完全相同");
        assert_ne!(ws[0].id, ws[1].id, "id 不带时间段 → checkpoint 会跨分段误命中");
        assert!(ws[0].id.contains("0.00-10.00"), "id 里要能看出区间: {}", ws[0].id);
        assert!(ws[1].id.contains("0.00-20.00"), "id 里要能看出区间: {}", ws[1].id);
    }
}
