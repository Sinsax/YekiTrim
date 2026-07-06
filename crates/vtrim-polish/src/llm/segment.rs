use crate::parse::srt::Subtitle;
use crate::project::Segment;
use super::client::LlmClient;

/// 调用 LLM 生成话题区间 + 段内事件
pub async fn generate_segments(
    client: &LlmClient,
    subtitles: &[Subtitle],
    event_type: &str,
) -> anyhow::Result<Vec<Segment>> {
    let user_prompt = build_prompt(subtitles, event_type);
    let response = client.chat(SYSTEM_PROMPT, &user_prompt).await?;
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

const SYSTEM_PROMPT: &str = r##"你是一个视频编辑助手。将字幕按话题切分成段，每段可包含事件。

每段输出: start, end, fx(low/med/high), note(话题备注)

事件类型 (放在 segments[].events[] 中):
- camera: 运镜. { time, duration, zoom, origin }
  zoom = "中"(中景,框多个元素) / "近"(近景特写,单个元素)
- 视觉特效: shake(震动)/flash(闪白)/blur(模糊)/zoom_bounce(抽拉)/vignette(暗角)
  hue_shift(色洗)/stutter(抖动)/split(分裂)/noise(噪点)/pulse(心跳)
  color_shift(暖色). 时间字段 { time, intensity?, color?, sfx? }
- emoji: 动画表情. 话题情绪爆发时使用（大笑、大哭、震惊等）.
  { time, duration, keyword(英文如"laugh""cry""shock""angry"), x/y("auto"), sfx? }
- irasu: 静止插图. 话题提到具体事物/场景时使用.
  { time, duration, keywords(日文数组,如["ゲーム""コントローラ""テレビゲーム"]), x/y("auto"), sfx? }
- overlay: 覆盖层. { time, duration, overlay_type(color/gradient/love), color?, opacity? }
- sfx: 纯音效, 无视觉. { time, name(音效id), volume?, duration?(秒, 不填则播放完整文件) }

使用规则:
- 每条 segment 至少有一个 camera event
- 根据字幕段数酌情添加 emoji/irasu，总量控制在 3~5 个，两者尽量平均使用
- 每个 irasu 的 keywords 用 2~3 个相关词，下载时会依次尝试搜索
- 整个视频中不同段尽量用不同的关键词组合，避免重复
- x/y 始终写 "auto"
- 每个 segment 的 fx 根据话题情绪强度设 low/med/high
- 需要音效时，从下面"可用音效场景标签"中选择最匹配的，用 sfx: "tag:场景名" 引用
  例如: sfx: "tag:コメディ" 或 sfx: "tag:surprise"

示例输出 (不要 markdown 包裹):
[
  {"start":0,"end":5,"fx":"low","note":"开场","events":[{"time":0,"duration":5,"type":"camera","zoom":"中","origin":"50% 50%"}]},
  {"start":5,"end":12,"fx":"med","note":"游戏讨论","events":[
    {"time":5,"duration":7,"type":"camera","zoom":"中","origin":"50% 50%"},
    {"time":6,"duration":3,"type":"irasu","keywords":["ゲーム","コントローラ"],"x":"auto","y":"auto"}
  ]},
  {"start":12,"end":20,"fx":"high","note":"爆笑反应","events":[
    {"time":12,"duration":8,"type":"camera","zoom":"近","origin":"50% 50%"},
    {"time":13,"duration":1.5,"type":"emoji","keyword":"laugh","x":"auto","y":"auto"},
    {"time":14,"duration":2,"type":"shake","intensity":3,"sfx":"tag:コメディ"}
  ]},
  {"start":20,"end":28,"fx":"high","note":"惊讶反应","events":[
    {"time":20,"duration":8,"type":"camera","zoom":"近","origin":"50% 50%"},
    {"time":21,"duration":3,"type":"irasu","keywords":["驚き","びっくり"],"x":"auto","y":"auto"},
    {"time":21.5,"duration":0.5,"type":"flash","color":"#ffffff"}
  ]},
  {"start":28,"end":35,"fx":"med","note":"操作电脑","events":[
    {"time":28,"duration":7,"type":"camera","zoom":"中","origin":"50% 50%"},
    {"time":29,"duration":4,"type":"irasu","keywords":["パソコン","タイピング"],"x":"auto","y":"auto"}
  ]},
  {"start":35,"end":40,"fx":"low","note":"结尾","events":[{"time":35,"duration":5,"type":"camera","zoom":"中","origin":"50% 50%"}]}
]"##;

pub fn build_prompt(subtitles: &[Subtitle], event_type: &str) -> String {
    let mut prompt = format!("视频类型: {}\n\n字幕内容:\n", event_type);
    for sub in subtitles.iter().take(100) {
        prompt.push_str(&format!("[{:.1}s-{:.1}s] {}\n", sub.start, sub.end, sub.text));
    }
    prompt.push_str(&format!("\n{}", crate::assets::sfx_catalog::to_prompt_text()));
    prompt.push_str("\n输出 JSON 数组:");
    prompt
}

fn extract_json_array(response: &str) -> anyhow::Result<String> {
    let trimmed = response.trim();
    if let Some(start) = trimmed.find('[') {
        if let Some(end) = trimmed.rfind(']') {
            return Ok(trimmed[start..=end].to_string());
        }
    }
    anyhow::bail!("No JSON array found:\n{}", &trimmed[..200.min(trimmed.len())])
}
