use crate::parse::srt::Subtitle;
use crate::project::Segment;
use super::client::LlmClient;

/// 调用 LLM 生成话题区间 + 段内事件
/// sticker_source 影响提示词中 sticker 事件的语言描述, 空数组则不提及 sticker
pub async fn generate_segments(
    client: &LlmClient,
    subtitles: &[Subtitle],
    event_type: &str,
    sticker_source: &[String],
) -> anyhow::Result<Vec<Segment>> {
    let system_prompt = build_system_prompt(sticker_source);
    let user_prompt = build_prompt(subtitles, event_type);
    let response = client.chat(&system_prompt, &user_prompt).await?;
    let json = extract_json_array(&response)?;
    let segments: Result<Vec<Segment>, _> = serde_json::from_str(&json);
    match segments {
        Ok(s) => Ok(s),
        Err(e) => {
            // 保存完整 JSON 到文件供调试
            if let Ok(mut f) = std::fs::File::create("_llm_debug.json") {
                use std::io::Write;
                let _ = f.write_all(json.as_bytes());
            }
            eprintln!("  ⚠ Full LLM response saved to _llm_debug.json");
            eprintln!("  ⚠ Error: {}", e);
            anyhow::bail!("Segments parse failed")
        }
    }
}

fn build_system_prompt(sticker_source: &[String]) -> String {
    let sticker_desc = if sticker_source.is_empty() {
        // 不使用 sticker, 不在提示词中提及
        String::new()
    } else {
        let keyword_lang = if sticker_source.iter().any(|s| s == "irasu") {
            "(日文优先，如「ゲーム」「驚き」「爆笑」)"
        } else {
            "(英文，如 \"laugh\" \"cry\" \"shock\" \"game\")"
        };
        format!("- sticker: 贴图表情/插画. 话题情绪爆发或提到具体事物/场景时使用.\
          \n  {{ time, duration, keyword{} , x/y(\"auto\"), sfx? }}\n", keyword_lang)
    };

    format!(r##"你是一个视频编辑助手。将字幕按话题切分成段，每段可包含事件。

每段输出: start, end, fx(low/med/high), note(话题备注)

事件类型 (放在 segments[].events[] 中):
- camera: 运镜. {{ time, duration, zoom, origin }}
  zoom = "中"(中景,框多个元素) / "近"(近景特写,单个元素)
- 视觉特效: shake(震动)/flash(闪白)/blur(模糊)/zoom_bounce(抽拉)/vignette(暗角)
  hue_shift(色洗)/stutter(抖动)/split(分裂)/noise(噪点)/pulse(心跳)
  color_shift(暖色). 时间字段 {{ time, intensity?, color?, sfx? }}
{}- overlay: 覆盖层. {{ time, duration, overlay_type(color/gradient/love), color?, opacity? }}
- sfx: 纯音效, 无视觉. {{ time, name(音效id), volume?, duration?(秒, 不填则播放完整文件) }}

使用规则:
- 每条 segment 至少有一个 camera event
- sticker 贴图数量请参考视频场景数量适度添加，总量不超过场景数量的一半
- 整个视频中不同段尽量用不同的关键词组合，避免重复
- x/y 始终写 "auto"
- 每个 segment 的 fx 根据话题情绪强度设 low/med/high
- 需要音效时，从下面"可用音效场景标签"中选择最匹配的，用 sfx: "tag:场景名" 引用
  例如: sfx: "tag:コメディ" 或 sfx: "tag:surprise"
"##, sticker_desc)
}

pub fn build_prompt(subtitles: &[Subtitle], event_type: &str) -> String {
    let mut prompt = format!("视频类型: {}\n\n字幕内容:\n", event_type);
    for sub in subtitles.iter().take(100) {
        prompt.push_str(&format!("[{:.1}s-{:.1}s] {}\n", sub.start, sub.end, sub.text));
    }
    prompt.push_str(&format!("\n{}", crate::assets::sfx_catalog::to_prompt_text()));
    prompt.push_str("\n输出 JSON 数组 (不要 markdown 包裹):");
    prompt
}

fn extract_json_array(response: &str) -> anyhow::Result<String> {
    let trimmed = response.trim();
    if let Some(start) = trimmed.find('[')
        && let Some(end) = trimmed.rfind(']') {
            return Ok(trimmed[start..=end].to_string());
        }
    anyhow::bail!("No JSON array found:\n{}", &trimmed[..200.min(trimmed.len())])
}
