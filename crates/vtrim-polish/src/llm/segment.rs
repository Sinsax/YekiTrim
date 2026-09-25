//! polish::llm::segment：Step 1 全局分段（一窗一段，产出 `Segment`）与分段/事件的提示词拼装。
//!
//! 入口：`generate_segments`、`build_segmentation_system_prompt`、`build_segmentation_user_prompt`、`build_events_system_prompt`、`build_events_user_prompt`、`normalize_segments_json`。
//! 边界：覆盖层示例必须给 `colors` 数组（四处求值器只认 `colors`，只写 `color` 会导致覆盖层全画不出来）；改内建 prompt 要同步用户配置里的副本 —— 见 `crates/vtrim-polish/src/llm/AGENTS.md`

use crate::parse::srt::Subtitle;
use crate::project::Segment;
use vtrim_core::llm::agent::extract_json_array;
use vtrim_core::llm::LlmClient;

/// 调用 LLM 生成话题区间 + 段内事件 + 音效判断
/// sticker_source 影响提示词中 sticker 事件的语言描述, 空数组则不提及 sticker
/// sticker_budget/sfx_budget 为数量上限 (来自 content_budget), 提示词中明确约束
/// sfx_blacklist: 被屏蔽的音效/标签, 注入 prompt 让 LLM 主动避开
pub async fn generate_segments(
    client: &LlmClient,
    subtitles: &[Subtitle],
    event_type: &str,
    sticker_source: &[String],
    sticker_budget: usize,
    sfx_budget: usize,
    sfx_blacklist: &[String],
) -> anyhow::Result<Vec<Segment>> {
    let system_prompt = build_segmentation_system_prompt(sticker_source);
    let user_prompt = build_segmentation_user_prompt(subtitles, event_type, sticker_budget, sfx_budget, sfx_blacklist);

    // 解析失败时把错误回传 LLM 重试 1 次（P4 第二部分）
    let mut last_err = None;
    let mut current_prompt = user_prompt;
    for attempt in 0..2 {
        let response = client.chat(&system_prompt, &current_prompt).await?;
        let json = extract_json_array(&response)?;
        let segments: Result<Vec<Segment>, _> = serde_json::from_str(&json);
        match segments {
            Ok(s) if !s.is_empty() => return Ok(s),
            Ok(_) => {
                last_err = Some(anyhow::anyhow!("Segments empty"));
                current_prompt = format!(
                    "{current_prompt}\n\n上一次输出是空数组。请按实际字幕话题切分并输出至少一个 Segment。"
                );
            }
            Err(e) => {
                last_err = Some(anyhow::anyhow!("Segments parse failed: {e}"));
                if attempt == 0 {
                    current_prompt = format!(
                        "{current_prompt}\n\n上一次输出无法解析：{e}\n请只输出 JSON 数组，不要 markdown。"
                    );
                }
            }
        }
    }
    Err(last_err.unwrap_or_else(|| anyhow::anyhow!("Segments failed")))
}

/// Step2 system prompt: 事件生成（音效只能从候选选择, 数量受预算约束）
pub(crate) fn build_events_system_prompt(
    sticker_source: &[String],
    sfx_blacklist: &[String],
    local_sticker_text: &str,
) -> String {
    let sticker_desc = sticker_lang_desc(sticker_source);
    let blacklist_line = if sfx_blacklist.is_empty() {
        String::new()
    } else {
        format!("- 以下音效被屏蔽, 禁止使用: {}
", sfx_blacklist.join(", "))
    };
    let local_line = if local_sticker_text.is_empty() {
        String::new()
    } else {
        format!("\n{}", local_sticker_text)
    };
    format!(r##"你是一个直播切片事件助手。为指定时间段生成视觉特效/贴纸/音效事件。

事件时间一律写成 **time(开始秒) + end(结束秒)** 两个绝对时间, 不要写"时长";
两者之差才是持续时间。

事件类型 (每个事件对象以 "type" 字段开头):
- camera: 运镜. {{"type":"camera","time":0.5,"end":3.5,"zoom":"中","anchor":"liver"}}
  zoom = "远"/"中"/"近"（远=画面缩小成画中画）
  anchor = 聚焦谁: "liver"(主播) / "danmaku"(弹幕) / "sticker"(贴纸) / "center"(画面中央)
  想从一处移到另一处, 再写 anchor_to (同样的取值); 不写就是原地聚焦
  想**硬切**(切镜, 瞬间换构图、没有推移动画)再加 "cut":true —— 只在刻意切镜时用, 不是常规手段
  近景想要"手持相机"的临场晃动感, 再加 "handheld":true —— **只在 zoom:"近" 上有效**（其他景别会被系统剔除）;
  它是叠加在运镜上的**持续状态**: 开了就一直摇到下一条不带它的运镜, 不是一次性特效
  例: {{"type":"camera","time":20.0,"end":24.0,"zoom":"近","anchor":"liver","handheld":true}}
  例: 主播→弹幕→回主播 = 两条事件
    {{"type":"camera","time":5.0,"end":9.0,"zoom":"中","anchor":"liver","anchor_to":"danmaku"}}
    {{"type":"camera","time":9.0,"end":12.0,"zoom":"中","anchor":"danmaku","anchor_to":"liver"}}
- sticker: 贴图表情. {{"type":"sticker","time":10.0,"end":12.0,"keyword":"ysyk","sticker_tag":"害怕","x":"auto","y":"auto"}}
  keyword 语言: {}
- 视觉特效: shake/flash/blur/zoom_bounce/vignette/hue_shift/stutter/split/noise/pulse/color_shift
  {{"type":"shake","time":8.0,"end":8.3,"intensity":0.5,"sfx":"tag:爆発"}}
- overlay: 覆盖层(氛围色). 渐变写 colors(两档起步色/终色), 纯色写 color
  {{"type":"overlay","time":0.0,"end":5.0,"overlay_type":"gradient","colors":["#ff0000","#000000"],"shape":"linear","opacity":0.3}}
  {{"type":"overlay","time":8.0,"end":10.0,"overlay_type":"color","color":"#ff0000","opacity":0.25}}
  颜色用 6 位十六进制(#rrggbb); 透明度归 opacity, 不要写进颜色里
  只写 color 不写 colors 时按「该色 → 透明」渲染, 但**优先写 colors**
  fade_in/fade_out(黑场淡入/淡出, 整块时长=渐变时长)只用于片头片尾, 片头那份系统只在初次后处理时自动补, 不要重复写
- sfx: 纯音效. {{"type":"sfx","time":3.0,"end":4.0,"name":"explosion","volume":0.5}}

选择规则:
- 音效 name/sfx 只能从"可用音效候选"中选择具体 id, 或写 tag:xxx 由系统匹配; 候选外的 id 无效
- 音效只放在情绪爆发/转折/名场面时刻, 普通叙述不要加
- 贴纸在情绪点使用; 起止时间会被系统对齐到**最近的一条字幕**（头尾都落在字幕边界上），
  你只要挑对是"哪句话"的情绪点即可; **如果提供了"可用本地贴纸包", keyword 必须从这些包名中选择**, 且必须从该包列的 tags 里选一个写进 sticker_tag; 候选外的 keyword/tag 无效
- 特效分两类:
  * 瞬时特效 shake/flash/blur/zoom_bounce/vignette/hue_shift/stutter/split/noise/pulse/color_shift:
    它们的 end-time 由系统接管, 你写不写都会被 clamp 到 0.1~0.6s; 只表现"爆发瞬间", 不要长时间驻留
  * 持续效果 camera/overlay/sticker: end 才有意义, overlay end-time 0.5~5s, sticker end-time 1.5~2.5s
- blur 只用于瞬间失焦/眩晕/被击中/闪回, 严禁长时间氛围模糊
- 运镜跟着内容走: 情绪爆发/名场面推近景("近"), 普通叙述中景("中")
- **主角是主播**: 视角移到弹幕或贴纸之后必须移回来(再写一条 anchor_to:"liver" 的事件);
  不要整段停在弹幕上
- "远"(画中画)只在尴尬、冷场、幽默反差的时刻用, **整片最多一条**（系统会兜底只留最早的一条）;
  它是**一次短暂的戏剧性停顿**: end - time 就是它在屏幕上待多久（写 1.5~3s, 到点系统会硬切回
  上一个镜头）, 别用它铺长时间; 用它时 anchor 一般写 "center"; 起点终点要落在字幕的边界上（别切在句子中间）
- 硬切(cut:true)同理: 整片偶尔一次就好, 连续硬切会让画面很跳
- handheld 是**可选的附加变种**（近景专用）: 只在正想要"手持镜头/临场纪实"的紧张、沉浸时刻用,
  **不是每条近景都要摇**; 它是持续状态 —— 想切回平稳镜头就再写一条不带 handheld 的运镜
- x/y 始终写 "auto"
- **优先为每个特效/贴纸从候选清单配一个音效** (候选已按当前 clip 方向筛选, 配错风险低);
  仅当候选清单确实没有合适音效时才留空
{}
{}
"##, sticker_desc, local_line, blacklist_line)
}

/// Step2 user prompt: 段信息 + 候选清单 + 段内字幕
pub(crate) fn build_events_user_prompt(segment: &Segment, subtitles: &[Subtitle], candidate_text: &str) -> String {
    let mut prompt = format!("段时间: [{:.1}s - {:.1}s]
", segment.start, segment.end);
    prompt.push_str(&format!("段情绪密度: {}
", segment.fx));
    if !candidate_text.is_empty() {
        prompt.push_str(&format!("
{}
", candidate_text));
    }
    prompt.push_str("
段内字幕:
");
    for sub in subtitles.iter().filter(|s| s.end >= segment.start - 3.0 && s.start <= segment.end + 3.0) {
        prompt.push_str(&format!("[{:.1}s-{:.1}s] {}
", sub.start, sub.end, sub.text));
    }
    prompt.push_str("
输出 JSON 数组 (不要 markdown 包裹):");
    prompt
}

/// 贴纸 keyword 语言说明（Step1/Step2 共用）
fn sticker_lang_desc(sticker_source: &[String]) -> String {
    if sticker_source.is_empty() {
        return "（不生成贴纸）".to_string();
    }
    if sticker_source.iter().any(|s| s == "emoji") {
        "(英文，如 \"laugh\" \"cry\" \"shock\" \"game\")".to_string()
    } else if sticker_source.iter().any(|s| s == "irasu") {
        "(日文优先，如「ゲーム」「驚き」「爆笑」)".to_string()
    } else {
        "(英文，如 \"laugh\" \"cry\" \"shock\" \"game\")".to_string()
    }
}

/// LLM 可能漏掉事件对象的 type 字段 (serde tagged enum 必需)。
/// 解析前按字段特征推断补全, 提高不同模型的输出容错性。
/// 推断规则: zoom/origin/anchor/anchor_to/cut/handheld→camera, overlay_type→overlay, keyword→sticker,
/// name→sfx, intensity/color→shake。无法推断的保持原样（由 serde 报错）。
pub(crate) fn normalize_segments_json(json: &str) -> String {
    let infer_type = |obj: &serde_json::Map<String, serde_json::Value>| -> Option<&'static str> {
        if obj.contains_key("zoom")
            || obj.contains_key("origin")
            || obj.contains_key("anchor")
            || obj.contains_key("anchor_to")
            || obj.contains_key("cut")
            || obj.contains_key("handheld")
        {
            return Some("camera");
        }
        if obj.contains_key("overlay_type") || obj.contains_key("colors") {
            return Some("overlay");
        }
        if obj.contains_key("keyword") {
            return Some("sticker");
        }
        if obj.contains_key("name") {
            return Some("sfx");
        }
        if obj.contains_key("intensity") || obj.contains_key("color") {
            return Some("shake");
        }
        None
    };

    match serde_json::from_str::<serde_json::Value>(json) {
        Ok(serde_json::Value::Array(mut segs)) => {
            for seg in segs.iter_mut() {
                if let Some(evs) = seg.get_mut("events").and_then(|e| e.as_array_mut()) {
                    for ev in evs.iter_mut() {
                        if let Some(obj) = ev.as_object_mut() {
                            if !obj.contains_key("type")
                                && let Some(t) = infer_type(obj) {
                                    obj.insert("type".into(), serde_json::Value::String(t.into()));
                                }
                        }
                    }
                }
            }
            serde_json::to_string(&segs).unwrap_or_else(|_| json.to_string())
        }
        _ => json.to_string(),
    }
}

/// Step1 system prompt: 分段 + 每段情绪密度
pub fn build_segmentation_system_prompt(sticker_source: &[String]) -> String {
    let sticker_desc = sticker_lang_desc(sticker_source);
    format!(r#"你是一个直播切片助手。将单个切片内字幕按话题切分成段。

每段输出 (JSON 对象):
{{
  "start": 0.0,
  "end": 85.0,
  "fx": "high",
  "note": "子话题备注"
}}

分段规则:
- 目标段长 8~30 秒，整片目标段数 = ceil(视频时长/20)，允许上下浮动 1 段
- 优先在字幕间隙 >2s 或话题明显变化处切段
- 同一话题连续叙述合并，不硬切；<4s 的碎片并入前一段
- 爆点/名场面单独成段
- fx: low=安静叙述/少特效, med=日常, high=爆点/密集特效
- note: 只写该段在讲什么子话题，例如"发现视角不对"、"尝试换场景"，不要概括整个视频
- 不要输出 direction/budget/effects 等字段
- {} 仅在贴纸可用时说明 keyword 语言；分段本身不选贴纸
"#, sticker_desc)
}

/// Step1 user prompt: 视频类型 + 预算感知采样字幕
pub fn build_segmentation_user_prompt(
    subtitles: &[Subtitle],
    event_type: &str,
    sticker_budget: usize,
    sfx_budget: usize,
    sfx_blacklist: &[String],
) -> String {
    let mut prompt = format!("视频类型: {}
视频时长(秒): {}
目标段数: {}
", event_type, video_duration(subtitles), target_segment_count(subtitles));

    prompt.push_str(&crate::assets::sfx_catalog::categories_text());
    if !sfx_blacklist.is_empty() {
        prompt.push_str(&format!("
音效黑名单(禁止使用): {}
", sfx_blacklist.join(", ")));
    }
    prompt.push_str(&format!("
全片预算: sticker 总数不超过 {} 个, sfx 事件总数不超过 {} 个
", sticker_budget, sfx_budget));
    prompt.push_str("
字幕内容 (每行: [开始秒-结束秒] 文本):
");
    for sub in sample_subtitles(subtitles, 200) {
        prompt.push_str(&format!("[{:.1}s-{:.1}s] {}
", sub.start, sub.end, sub.text));
    }
    prompt.push_str("
输出 JSON 数组 (不要 markdown 包裹):");
    prompt
}

/// 预算感知采样：首尾保留，中部均匀采样到 max 条。
fn sample_subtitles<'a>(subtitles: &'a [Subtitle], max: usize) -> Vec<&'a Subtitle> {
    let n = subtitles.len();
    if n <= max {
        return subtitles.iter().collect();
    }
    let head = (max / 4).max(20);
    let tail = (max / 4).max(20);
    let head = head.min(n);
    let tail = tail.min(n.saturating_sub(head));
    let middle_max = max.saturating_sub(head + tail);
    let mut out: Vec<&Subtitle> = Vec::with_capacity(max);
    out.extend(subtitles[..head].iter());
    if middle_max > 0 {
        let middle_total = n - head - tail;
        for i in 0..middle_max {
            let idx = head + i * (middle_total - 1).max(0) / (middle_max - 1).max(1);
            out.push(&subtitles[idx]);
        }
    }
    out.extend(subtitles[n - tail..].iter());
    out
}

fn video_duration(subtitles: &[Subtitle]) -> f64 {
    subtitles.last().map(|s| s.end).unwrap_or(0.0)
}

fn target_segment_count(subtitles: &[Subtitle]) -> usize {
    let dur = video_duration(subtitles);
    ((dur / 20.0).ceil() as usize).clamp(2, 8)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 回归测试: LLM 输出缺 type 字段时按字段特征补全 (实际失败样例)
    #[test]
    fn normalize_infers_missing_type() {
        let raw = r#"[
          {"start":0.0,"end":8.6,"fx":"low","note":"t","events":[
            {"time":0.5,"end":3.5,"zoom":"中","origin":"center"},
            {"time":8.0,"end":8.5,"intensity":0.3,"sfx":"tag:爆発"},
            {"type":"camera","time":2.0,"end":3.0,"zoom":"近","origin":"50% 50%"},
            {"time":10.0,"end":12.0,"keyword":"laugh","x":"auto","y":"auto"},
            {"time":3.0,"name":"pop","volume":0.5},
            {"time":12.0,"end":15.0,"anchor":"liver"},
            {"time":20.0,"end":24.0,"handheld":true}
          ]}
        ]"#;
        let norm = normalize_segments_json(raw);
        let segments: Vec<crate::project::Segment> = serde_json::from_str(&norm).unwrap();
        let evs = &segments[0].events;
        assert_eq!(evs.len(), 7);
        assert!(matches!(evs[0], crate::project::Event::Camera { .. }));
        assert!(matches!(evs[1], crate::project::Event::Shake { .. }));
        assert!(matches!(evs[2], crate::project::Event::Camera { .. }));
        assert!(matches!(evs[3], crate::project::Event::Sticker { .. }));
        assert!(matches!(evs[4], crate::project::Event::Sfx { .. }));
        // 只写 anchor（没写 zoom/origin）也要认得是 camera
        assert!(
            matches!(&evs[5], crate::project::Event::Camera { anchor: Some(a), .. } if a == "liver")
        );
        // 只写 handheld 同理（附加变种常常不重复景别之外的字段）
        assert!(
            matches!(&evs[6], crate::project::Event::Camera { handheld: true, .. }),
            "只写 handheld 的应推断成 camera: {norm}"
        );
    }

    /// 已有 type 的事件不被改动
    #[test]
    fn normalize_keeps_existing_type() {
        let raw = r#"[{"start":0.0,"end":1.0,"fx":"low","note":"","events":[
            {"type":"shake","time":0.5,"intensity":0.9}
        ]}]"#;
        let norm = normalize_segments_json(raw);
        let segments: Vec<crate::project::Segment> = serde_json::from_str(&norm).unwrap();
        assert!(matches!(segments[0].events[0], crate::project::Event::Shake { .. }));
    }
}
