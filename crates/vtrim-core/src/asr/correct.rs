use std::sync::Arc;

use anyhow::{Context, Result};
use indicatif::{ProgressBar, ProgressStyle};
use tokio::sync::Semaphore;
use tokio::task::JoinSet;

use crate::config::{CorrectConfig, LlmConfig};
use crate::danmaku::DanmakuEntry;
use crate::types::TranscriptSegment;

const BATCH_SIZE: usize = 25;
const CONTEXT_OVERLAP: usize = 15;
const MAX_CONCURRENT: usize = 3;

pub const DEFAULT_SYSTEM_PROMPT: &str = "\
你是虚拟主播直播转录文本的订正助手。\
修正 ASR 常见的错别字、主播名/VTuber名、游戏名、歌曲名、网络用语。\
保留口语风格和语气词（啊、哦、嗯、嘛、呗），不要书面化润色。\
只以 JSON 对象格式输出有修正的条目。";

pub const DEFAULT_USER_PROMPT: &str = "\
请修正以下语音转文字转录，注意修正：
 - 错别字和语音识别常见错误（同音字、谐音字）
 - 主播名、VTuber 名、嘉宾名
 - 游戏名、动画名、歌曲名（如\"法环\"不会写成\"发还\"）
 - 日韩文娱名称（歌手/乐队/偶像/动画曲/V家）：如椎名林檎、YOASOBI、Ado、NewJeans
 - V 圈 / B 站常用术语（SC、舰长、提督、舰礼、切片、串、纯良、米线等）
 - 网络流行语和常见缩写（xp、yyds、kpi、sc、dlc、op、ww等）
 - 语速快、口糊、吞音导致的识别错误，根据上下文还原正确文字
 - 保留口语风格：不要去掉啊、哦、嗯、嘛、呗、啦等语气词
 - 保留重复和停顿的修辞效果（如\"我我我真的\"、\"然后然后\"）
 - 保留情绪化表达（大笑、尖叫、哭腔、破音等通过文字体现）

修正示例：
输入：法环太好玩了，我要去b站键长看切偏
输出：法环太好玩了，我要去B站舰长看切片

输入：椎名临勤的歌真的很好听，我超喜欢YOASOBI
输出：椎名林檎的歌真的很好听，我超喜欢YOASOBI

输入：这个sc说的是什么，破房了家人们，年度最佳预订
输出：这个SC说的是什么，破防了家人们，年度最佳预订

{context}

需要订正的转录（JSON 数组，每个元素包含 index 和 text）：
{segments}

只输出【有修正】的条目，没有修正的条目不要输出。
输出格式为 JSON 对象，键是原 index，值是该条修正后的完整文本：
{\"3\": \"修正后的文本\", \"7\": \"另一条修正后的文本\"}

如果全部都不需要修正，只输出：
{}

不要输出 markdown 代码块，不要在 JSON 外附加任何其他文字。";

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

pub async fn correct_transcript(
    segments: &[TranscriptSegment],
    danmaku: &[DanmakuEntry],
    llm: &LlmConfig,
    correct: &CorrectConfig,
) -> Result<Vec<TranscriptSegment>> {
    if segments.is_empty() {
        return Ok(Vec::new());
    }

    let total = segments.len();
    let total_batches = total.div_ceil(BATCH_SIZE);
    let llm = Arc::new(llm.clone());
    let correct = Arc::new(correct.clone());
    let danmaku = Arc::new(danmaku.to_vec());
    let sem = Arc::new(Semaphore::new(MAX_CONCURRENT));
    let mut set: JoinSet<(usize, usize, Result<Vec<String>>)> = JoinSet::new();

    let pb = ProgressBar::new(total_batches as u64);
    if let Ok(style) = ProgressStyle::with_template(
        "{spinner:.green} [{elapsed_precise}] 订正 {bar:40} {percent}%",
    ) {
        pb.set_style(style.progress_chars("##-"));
    }
    pb.enable_steady_tick(std::time::Duration::from_millis(100));

    for (batch_idx, chunk) in segments.chunks(BATCH_SIZE).enumerate() {
        let offset = batch_idx * BATCH_SIZE;
        let batch_num = batch_idx + 1;
        let chunk_len = chunk.len();

        // 从上一批末尾取 CONTEXT_OVERLAP 段作为前文上下文
        let context: Vec<TranscriptSegment> = if batch_idx > 0 {
            let ctx_start = offset.saturating_sub(CONTEXT_OVERLAP);
            segments[ctx_start..offset].to_vec()
        } else {
            Vec::new()
        };

        let chunk = chunk.to_vec();
        let llm = llm.clone();
        let correct = correct.clone();
        let sem = sem.clone();
        let danmaku = danmaku.clone();

        set.spawn(async move {
            let _permit = sem.acquire().await.expect("Semaphore 仅在本函数内使用，不会被关闭");
            tracing::info!("  批次 {} 订正中 ({} 段)", batch_num, chunk_len);
            (
                offset,
                batch_num,
                send_batch(&chunk, &context, &danmaku, &llm, &correct, offset).await,
            )
        });
    }

    // 收集订正结果，未返回/失败的条目保持原文
    let mut corrected_texts = segments.iter().map(|s| s.text.clone()).collect::<Vec<_>>();
    while let Some(result) = set.join_next().await {
        match result {
            Ok((offset, batch_num, Ok(texts))) => {
                for (i, text) in texts.iter().enumerate() {
                    if i < texts.len() && offset + i < corrected_texts.len() {
                        corrected_texts[offset + i].clone_from(text);
                    }
                }
                pb.inc(1);
                tracing::info!("  批次 {} 订正完成", batch_num);
            }
            Ok((_, batch_num, Err(e))) => {
                pb.inc(1);
                tracing::warn!("批次 {} 订正失败: {}，保留原文", batch_num, e);
            }
            Err(e) => {
                tracing::warn!("并行任务失败: {}", e);
            }
        }
    }

    pb.finish_with_message("订正完成");

    let mut result = segments.to_vec();
    for (i, seg) in result.iter_mut().enumerate() {
        seg.text = corrected_texts[i].clone();
    }
    Ok(result)
}

async fn send_batch(
    segments: &[TranscriptSegment],
    context: &[TranscriptSegment],
    danmaku: &[DanmakuEntry],
    llm: &LlmConfig,
    correct: &CorrectConfig,
    offset: usize,
) -> Result<Vec<String>> {
    // 构建上下文文本
    let context_text = if !context.is_empty() {
        let mut ctx = String::from("\n--- 前文上下文（仅作参考，无需修正） ---\n");
        for seg in context {
            ctx.push_str(&format!("[{:.1}-{:.1}] {}\n", seg.start, seg.end, seg.text));
        }
        ctx
    } else {
        String::new()
    };

    // 构建弹幕上下文（时间重叠的弹幕，最多 15 条）
    let dm_text = if !danmaku.is_empty() {
        let mut dm = String::from("\n--- 当前时段弹幕（供参考，弹幕文字不会出错） ---\n");
        let start_time = segments.first().map(|s| s.start).unwrap_or(0.0);
        let end_time = segments.last().map(|s| s.end).unwrap_or(0.0);
        let mut count = 0;
        for d in danmaku {
            if count >= 15 {
                break;
            }
            // 弹幕与当前批次时间范围有重叠
            if d.start < end_time && d.end > start_time {
                dm.push_str(&format!("[{:.1}-{:.1}] {}\n", d.start, d.end, d.text));
                count += 1;
            }
        }
        dm
    } else {
        String::new()
    };

    // 构建待订正批次的 JSON
    let seg_json: Vec<serde_json::Value> = segments
        .iter()
        .enumerate()
        .map(|(i, seg)| {
            serde_json::json!({"index": offset + i + 1, "text": seg.text})
        })
        .collect();
    let seg_text = serde_json::to_string_pretty(&seg_json)
        .context("序列化转录段为 JSON 失败")?;

    // 组装用户 prompt
    let user_prompt = effective_user_prompt(correct);
    let replaced = user_prompt
        .replace("{context}", &format!("{}{}", context_text, dm_text))
        .replace("{segments}", &seg_text);

    // 系统消息
    let system_msg = effective_system_prompt(correct);

    // 初始结果全部为原文；只有 LLM 明确返回的条目才会被替换
    let mut result = segments.iter().map(|s| s.text.clone()).collect::<Vec<_>>();
    let corrected = send_and_parse(&replaced, &system_msg, llm, offset, segments.len()).await?;
    for (i, text) in corrected.iter().enumerate() {
        if !text.is_empty() && i < result.len() {
            result[i].clone_from(text);
        }
    }
    Ok(result)
}

async fn send_and_parse(
    user_prompt: &str,
    system_prompt: &str,
    llm: &LlmConfig,
    offset: usize,
    expected: usize,
) -> Result<Vec<String>> {
    let client = crate::llm::LlmClient::from_config(llm)?.with_timeout(120);
    let messages = serde_json::json!([
        {"role": "system", "content": system_prompt},
        {"role": "user", "content": user_prompt}
    ]);
    let resp_body = client.send(messages).await?;
    let content = resp_body["choices"][0]["message"]["content"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("LLM 响应缺少 choices[0].message.content"))?;

    match parse_correction_result(content, offset, expected) {
        Ok(result) => Ok(result),
        Err(e) => {
            tracing::warn!("订正 JSON 解析失败，重试一次: {e:#}");
            let retry_prompt = format!(
                "{}\n\n注意：上次输出无法解析，请只输出 JSON 对象，不要包含任何其他文字或代码块。",
                user_prompt
            );
            let messages = serde_json::json!([
                {"role": "system", "content": system_prompt},
                {"role": "user", "content": retry_prompt}
            ]);
            let resp_body = client.send(messages).await?;
            let content = resp_body["choices"][0]["message"]["content"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("LLM 重试响应缺少 choices[0].message.content"))?;
            parse_correction_result(content, offset, expected)
        }
    }
}

/// 解析订正结果。
///
/// 优先支持 JSON 对象：`{"3": "修正文本", ...}`。
/// 兼容旧的 JSON 数组：`[{"index":3,"text":"修正文本"}, ...]`。
fn parse_correction_result(content: &str, offset: usize, expected: usize) -> Result<Vec<String>> {
    let mut result = vec![String::new(); expected];
    let json_str = extract_json(content);
    let value: serde_json::Value =
        serde_json::from_str(json_str).context("订正 JSON 格式无效")?;

    match value {
        serde_json::Value::Object(map) => {
            for (key, val) in map {
                let Ok(idx) = key.parse::<usize>() else {
                    continue;
                };
                if let Some(text) = val.as_str() {
                    if idx > offset && idx <= offset + expected {
                        result[idx - offset - 1] = text.to_string();
                    }
                }
            }
        }
        serde_json::Value::Array(arr) => {
            for item in arr {
                let idx = item
                    .get("index")
                    .and_then(|v| v.as_u64())
                    .map(|n| n as usize);
                let text = item.get("text").and_then(|v| v.as_str());
                if let (Some(idx), Some(text)) = (idx, text) {
                    if idx > offset && idx <= offset + expected {
                        result[idx - offset - 1] = text.to_string();
                    }
                }
            }
        }
        _ => anyhow::bail!("订正结果不是 JSON 对象或数组"),
    }

    Ok(result)
}

fn extract_json(text: &str) -> &str {
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

    // 兼容对象或数组：取第一个 { 或 [ 到最后一个 } 或 ]
    let start = text
        .find(['{', '['])
        .unwrap_or(0);
    let end = text
        .rfind(['}', ']'])
        .map(|i| i + 1)
        .unwrap_or(text.len());
    &text[start..end]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_object_corrections() {
        let out = parse_correction_result(r#"{"2":"法环","5":"B站舰长"}"#, 0, 10).unwrap();
        assert_eq!(out[1], "法环");
        assert_eq!(out[4], "B站舰长");
        assert!(out[0].is_empty());
    }

    #[test]
    fn parse_array_corrections() {
        let out = parse_correction_result(
            r#"[{"index":3,"text":"破防了"},{"index":7,"text":"寄！"}]"#,
            0,
            10,
        )
        .unwrap();
        assert_eq!(out[2], "破防了");
        assert_eq!(out[6], "寄！");
    }

    #[test]
    fn parse_empty_object() {
        let out = parse_correction_result("{}", 0, 5).unwrap();
        assert!(out.iter().all(|s| s.is_empty()));
    }

    #[test]
    fn extract_json_strips_code_fence() {
        let s = "```json\n{\"1\":\"你好\"}\n```";
        assert_eq!(extract_json(s), "{\"1\":\"你好\"}");
    }
}
