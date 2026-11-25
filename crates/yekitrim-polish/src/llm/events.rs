//! polish::llm::events：Step 2 逐段事件生成（走 `yekitrim_llm::agent::Agent` + checkpoint 续传 + 贴纸批量精挑）。
//!
//! 入口：`EventGenTask::{new, with_project, build_windows, apply_outputs}`、`run_event_agent`、`choose_sticker_files_batch`、`StickerChoiceContext`。
//! 边界：窗口 id 必须带时间段，否则重新分段后会命中旧窗口；`output == None`（失败）与 `Some([])`（空结果）都跳过硬、保留原有事件 —— 见 `crates/yekitrim-polish/src/llm/AGENTS.md`

use std::path::Path;

use anyhow::Result;
use async_trait::async_trait;
use serde_json::Value;

use crate::parse::srt::Subtitle;
use crate::project::{Event, MuteRange, Segment};
use yekitrim_llm::agent::{
    extract_json_array, AgentItem, AgentProgress, AgentTask, AgentWindow,
};
use yekitrim_llm::LlmClient;

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
    /// **这一趟不该产出事件的区间**（手剪屏蔽段）。默认空 = 老行为。
    mute_ranges: Vec<MuteRange>,
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
            mute_ranges: Vec::new(),
        }
    }

    /// 告诉任务「哪些区间已经被手剪掉了」（见 [`EventGenTask::parse_output`] 的剔除）。
    ///
    /// 为什么需要它：屏蔽段里的**字幕**在进 LLM 之前就已经被过滤掉了（`cmd_llm`），
    /// 但模型仍可能按上下文把事件写到那段时间里（比如照抄 ±3s 窗口外的邻居）。
    /// 那一段在成片里不存在 ⇒ 生成出来就是浪费，还会挤占后处理的预算。
    /// ⚠️ 这里只**丢弃这一趟的生成物**，`polish.toml` 里已有的照旧一个字节都不动。
    pub fn with_mute_ranges(mut self, ranges: Vec<MuteRange>) -> Self {
        self.mute_ranges = ranges;
        self
    }

    /// 用量记账归因（链路名 + 切片目录）。
    pub fn with_project(mut self, project: Option<&std::path::Path>) -> Self {
        self.project = project.map(yekitrim_core::path::display);
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

    fn usage_context(&self) -> yekitrim_llm::usage::UsageContext {
        yekitrim_llm::usage::UsageContext::new(self.name()).project(self.project.clone())
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
        yekitrim_llm::agent::fingerprint(&[&self.system_prompt()])
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
        // ⚠️ **逐事件容错**（P0 2026-10-20）：from_value::<Vec<Event>> 一个坏元素就让**整窗**失败
        // —— 实测「模型多写一条 sticker 但漏了 x」⇒ missing field x ⇒ 20.5s 窗口的全部产出归零。
        // 现在坏一条只跳过一条，好的照收。
        //
        // ⚠️ **全部失败仍然 = 这一窗没产出**（不覆盖旧事件，见 apply_outputs）：
        //    全部坏 ⇒ 返回 Err；模型真返回 [] ⇒ events 也是空（同样不覆盖）。
        let mut events = parse_events_tolerant(&value)?;
        // **屏蔽段里不产出事件**（2026-10-19）：那一段在成片里不存在，写出去只会挤占预算、
        // 还会在编辑器里显示成「这里怎么有个看不见的贴纸」。只丢这一趟的生成物 ——
        // `polish.toml` 里已有的一个字节都不动（屏蔽只做标记，见 `project/AGENTS.md`）。
        let before = events.len();
        events.retain(|ev| !crate::project::event_starts_muted(&self.mute_ranges, ev));
        if before > events.len() {
            tracing::info!(
                "  ✂ 屏蔽区间内不生成事件：剔除 {} 条（生成 {} 条）",
                before - events.len(),
                before
            );
        }
        Ok(events)
    }

    async fn process_window(
        &self,
        _all_items: &[AgentItem],
        window: &AgentWindow<Vec<Event>>,
        _global: &str,
        llm_cfg: &yekitrim_llm::LlmParams,
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

        let client = LlmClient::new(llm_cfg)?
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
        let mut windows: Vec<AgentWindow<Vec<Event>>> = serde_json::from_value(value)?;
        // **续传来的窗口也过同一道屏蔽闸**：checkpoint 可能是改了屏蔽范围之前存的
        // （甚至可能是上一版代码存的），命中之后拿回来的事件照样要落在"看不见的段"里 ——
        // 判据与 [`EventGenTask::parse_output`] 同一个谓词，别只在新生成那条路上过滤。
        for w in &mut windows {
            if let Some(out) = &mut w.output {
                out.retain(|ev| !crate::project::event_starts_muted(&self.mute_ranges, ev));
            }
        }
        Ok(windows)
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
    mute_ranges: &[MuteRange],
    llm_cfg: &yekitrim_core::config::LlmConfigV4,
    project: Option<&Path>,
    checkpoint_path: Option<&Path>,
    cancel: Option<yekitrim_llm::agent::CancelProbe>,
    // 窗口进度出口（spec §1 P2）：Some 时每个窗口完成折成 stage_progress(完成窗, 总窗)。
    // server 侧传 WebProgress（通道版桥），CLI 传 None（保持现状，不接进度条）。
    progress: Option<std::sync::Arc<dyn yekitrim_core::progress::ProgressReporter>>,
) -> anyhow::Result<Vec<SegmentGenStatus>> {
    // 链路解析：精修这条链路（见 [llm.overrides.polish]）。
    // 幂等：从 `run_llm_pipeline` 传进来的可能已经是解析过的配置。
    let linked = llm_cfg.for_link(yekitrim_core::config::LlmLink::Polish);
    let llm_cfg = &linked;
    let task = EventGenTask::new(
        segments.to_vec(),
        subtitles.to_vec(),
        candidate_texts,
        sticker_source.to_vec(),
        sfx_blacklist.to_vec(),
        local_sticker_text,
    )
    .with_project(project)
    .with_mute_ranges(mute_ranges.to_vec());
    let windows = task.build_windows::<Vec<Event>>();
    // 并发跟 .yekitrim/llm.toml 的 [llm] max_concurrent（精修没有自己的并发配置项）
    let agent = yekitrim_llm::agent::Agent::new(llm_cfg.clone())
        .with_max_concurrent(llm_cfg.concurrency(None));
    // ⚠️ 取消探针必须真的交给 Agent（窗口边界生效）。精修 llm 是分钟级链路：
    // 只在阶段边界查一次 = 用户点了停止、后台仍把每个窗口跑完 —— 而这条链路
    // 默认并发 3，剩下的窗口是**成批**继续发的，用户看到的就是"停止没反应"。
    let agent = match cancel {
        Some(probe) => agent.with_cancel(probe),
        None => agent,
    };

    // 窗口进度：AgentProgress 经通道折成 stage_progress。窗口以几十秒计，无需节流；
    // reporter 为 None 时通道照建（发不出去就算了），行为与旧版逐字节一致。
    let (progress_tx, mut progress_rx) =
        tokio::sync::mpsc::channel::<AgentProgress<Vec<Event>>>(16);
    let pump = progress.map(|p| {
        tokio::spawn(async move {
            while let Some(ap) = progress_rx.recv().await {
                yekitrim_core::progress::ProgressReporter::stage_progress(
                    p.as_ref(),
                    ap.completed_windows as u64,
                    ap.total_windows as u64,
                );
            }
        })
    });

    let completed = agent
        .run_windows(task, windows, checkpoint_path, Some(progress_tx))
        .await?;
    if let Some(handle) = pump {
        let _ = handle.await;
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
    llm_cfg: &yekitrim_core::config::LlmConfigV4,
    project: Option<&std::path::Path>,
    package_name: &str,
    tag: &str,
    candidates: &[String],
    contexts: &[StickerChoiceContext],
) -> anyhow::Result<Vec<(usize, usize, String)>> {
    use yekitrim_llm::agent::extract_json_object;

    if contexts.is_empty() || candidates.is_empty() {
        return Ok(vec![]);
    }

    // 链路解析：精修这条链路（见 [llm.overrides.polish]）
    let linked = llm_cfg.for_link(yekitrim_core::config::LlmLink::Polish);
    let llm_cfg = &linked;

    let client = LlmClient::new(llm_cfg)?
        .with_timeout(120)
        .with_usage_context(
            yekitrim_llm::usage::UsageContext::new(yekitrim_llm::tasks::POLISH_STICKER_PICK.label)
                .project(project.map(yekitrim_core::path::display)),
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


/// **逐事件容错解析**（P0 2026-10-20）：模型一个坏元素不许让**整窗**失败。
///
/// 事故：模型多写了一条 sticker 却漏了 `x` ⇒ `missing field x` ⇒ 20.5 秒窗口的全部产出归零
/// （用户贴的 WARN 就是它）。现在坏一条跳过一条、好的照收。
///
/// **语义边界（别改）**：坏条数 > 0 但还有好的 ⇒ 返回好的；**全部坏** ⇒ `Err`
///（= 本窗失败 ⇒ `apply_outputs` 那条「失败不覆盖」照旧生效）。模型真返回 `[]` ⇒ 返回空数组
/// （同样是"这一窗没产出"）。
///
/// 为什么不设"跳过率上限"：LLM 的输出质量是**逐窗**的，设一个比例阈值只会让"8 条里坏 3 条"
/// 这种可救的窗被整窗丢掉 —— 反过来全坏那条已经被 `Err` 兜住了。日志里报「N/M」足够定位。
fn parse_events_tolerant(value: &Value) -> Result<Vec<Event>> {
    let items = value
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("模型输出不是 JSON 数组"))?;
    let total = items.len();
    let mut events: Vec<Event> = Vec::with_capacity(total);
    let mut failures: Vec<(usize, String)> = Vec::new();
    for (idx, item) in items.iter().enumerate() {
        match serde_json::from_value::<Event>(item.clone()) {
            Ok(ev) => events.push(ev),
            Err(e) => failures.push((idx, e.to_string())),
        }
    }
    if !failures.is_empty() {
        let (first_idx, first_why) = &failures[0];
        tracing::warn!(
            "  ⚠ 窗口事件解析：{total} 条中 {} 条失败（{first_why} @ index {first_idx}），已跳过",
            failures.len()
        );
    }
    if total > 0 && events.is_empty() {
        anyhow::bail!("窗口事件全部解析失败（{total} 条）");
    }
    Ok(events)
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
            speed: None,
            tilt: None,
        }
    }


    /// **P0 2026-10-20：一条坏事件不许让整窗归零**。
    ///
    /// 变异自证：把 `parse_events_tolerant` 换回 `from_value::<Vec<Event>>` ⇒ 本条红（整窗 Err）。
    #[test]
    fn parse_events_skips_only_the_bad_ones() {
        let value: Value = serde_json::json!([
            {"type": "sticker", "time": "oops"},                       // 坏：time 不是数字
            {"type": "sticker", "time": 1.0, "keyword": "a", "x": "10%", "y": "20%"},
            {"type": "camera", "time": 2.0, "zoom": "中", "origin": "50% 50%"},
            {"type": "camera", "time": 3.0, "zoom": "近", "origin": "50% 50%"},
        ]);
        let events = super::parse_events_tolerant(&value).expect("坏一条不该整窗失败");
        assert_eq!(events.len(), 3, "坏的跳过、好的三条都在");
    }

    /// **全部坏 ⇒ Err**（= 本窗失败 ⇒ `apply_outputs` 那条"失败不覆盖"照旧生效）。
    #[test]
    fn parse_events_all_bad_is_an_error() {
        let value: Value = serde_json::json!([
            {"type": "sticker", "time": "oops"},
            {"type": "shake", "time": "also-bad"},
        ]);
        assert!(super::parse_events_tolerant(&value).is_err(), "全坏必须报错，不许当成空结果");
    }

    /// **sticker 缺 x/y 也能解析**（空串 = 还没落点，`process` Step 2b 会回填）。
    ///
    /// 变异自证：把 `Sticker` 里那两个 `#[serde(default)]` 去掉 ⇒ 本条红（missing field x）。
    #[test]
    fn sticker_without_xy_parses_as_unplaced() {
        let value: Value = serde_json::json!([
            {"type": "sticker", "time": 1.0, "keyword": "ずんだもん"},
        ]);
        let events = super::parse_events_tolerant(&value).expect("缺 x/y 不该失败");
        assert_eq!(events.len(), 1);
        match &events[0] {
            Event::Sticker { x, y, .. } => {
                assert_eq!((x.as_str(), y.as_str()), ("", ""), "空串 = 还没落点");
            }
            _ => panic!("应该是 sticker"),
        }
    }

    /// **R4**：有 x/y 的老事件序列化**逐字节不变**（照写，不 skip、不加字段）。
    #[test]
    fn sticker_xy_roundtrip_is_byte_identical() {
        let text = r#"{"type":"sticker","time":1.0,"keyword":"a","x":"12.5%","y":"70%","sfx":null,"sfx_volume":null}"#;
        let ev: Event = serde_json::from_str(text).expect("解析");
        let again = serde_json::to_string(&ev).expect("序列化");
        assert!(again.contains(r#""x":"12.5%""#), "有值照写：{again}");
        assert!(again.contains(r#""y":"70%""#), "有值照写：{again}");
        let ev2: Event = serde_json::from_str(&again).expect("再解析");
        assert_eq!(serde_json::to_string(&ev2).unwrap(), again, "往返稳定");
    }
    fn seg(start: f64, end: f64, events: Vec<Event>) -> Segment {
        Segment { start, end, fx: "med".into(), note: String::new(), events, role: String::new(), extra: Default::default() }
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
