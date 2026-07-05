use std::sync::Arc;

use anyhow::{Context, Result};
use indicatif::{ProgressBar, ProgressStyle};
use tokio::sync::Semaphore;
use tokio::task::JoinSet;

use crate::config::{CorrectConfig, LlmConfig};
use crate::types::TranscriptSegment;

const BATCH_SIZE: usize = 100;
const CONTEXT_OVERLAP: usize = 15;
const MAX_CONCURRENT: usize = 3;

pub const DEFAULT_SYSTEM_PROMPT: &str = "\
你是虚拟主播直播转录文本的订正助手。\
修正 ASR 常见的错别字、主播名/VTuber名、游戏名、歌曲名、网络用语。\
保留口语风格和语气词（啊、哦、嗯、嘛、呗），不要书面化润色。\
只以 JSON 格式输出修正结果。";

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

输入：我我我真的要被气笑了，然后然后他就跑了，寄！
输出：我我我真的要被气笑了，然后然后他就跑了，寄！

输入：这波操作太难绷了，血压高了，主播老吃家了
输出：这波操作太难绷了，血压高了，主播老吃家了

{context}

需要修正的转录（JSON 格式，每个元素包含 index 和 text）：
{segments}

请以 JSON 数组格式返回修正结果：
[{\"index\": 序号, \"text\": \"修正后文字\"}, ...]
不要在 JSON 外附加任何其他文字，不要使用 markdown 代码块。";

pub async fn correct_transcript(
    segments: &[TranscriptSegment],
    llm: &LlmConfig,
    correct: &CorrectConfig,
) -> Result<Vec<TranscriptSegment>> {
    // 第一步：预合并短段，让 LLM 能看⻅更多上下文
    let (merged, merge_groups) = merge_short_segments(segments);
    let total = merged.len();
    let total_batches = (total + BATCH_SIZE - 1) / BATCH_SIZE;
    let llm = Arc::new(llm.clone());
    let correct = Arc::new(correct.clone());
    let sem = Arc::new(Semaphore::new(MAX_CONCURRENT));
    let mut set: JoinSet<(usize, usize, Result<Vec<String>>)> = JoinSet::new();

    if total != segments.len() {
        tracing::info!("LLM 订正前预合并: {} 段 → {} 组", segments.len(), total);
    }

    let pb = ProgressBar::new(total_batches as u64);
    if let Ok(style) = ProgressStyle::with_template(
        "{spinner:.green} [{elapsed_precise}] 订正 {bar:40} {percent}%",
    ) {
        pb.set_style(style.progress_chars("##-"));
    }
    pb.enable_steady_tick(std::time::Duration::from_millis(100));

    for (batch_idx, chunk) in merged.chunks(BATCH_SIZE).enumerate() {
        let offset = batch_idx * BATCH_SIZE;
        let batch_num = batch_idx + 1;
        let chunk_len = chunk.len();

        // 从上一批末尾取 CONTEXT_OVERLAP 段作为前文上下文
        let context: Vec<TranscriptSegment> = if batch_idx > 0 {
            let ctx_start = offset.saturating_sub(CONTEXT_OVERLAP);
            merged[ctx_start..offset].to_vec()
        } else {
            Vec::new()
        };

        let chunk = chunk.to_vec();
        let llm = llm.clone();
        let correct = correct.clone();
        let sem = sem.clone();

        set.spawn(async move {
            let _permit = sem.acquire().await.unwrap();
            tracing::info!("  批次 {} 订正中 ({} 段)", batch_num, chunk_len);
            (
                offset,
                batch_num,
                send_batch(&chunk, &context, &llm, &correct, offset).await,
            )
        });
    }

    // 收集订正结果（按合并后的索引）
    let mut corrected_texts = merged.iter().map(|s| s.text.clone()).collect::<Vec<_>>();
    while let Some(result) = set.join_next().await {
        match result {
            Ok((offset, batch_num, Ok(texts))) => {
                for (i, text) in texts.iter().enumerate() {
                    if i < BATCH_SIZE.min(total.saturating_sub(offset)) {
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

    // 第二步：将合并组的订正结果按比例拆分回原始分段
    Ok(expand_merged(&corrected_texts, segments, &merge_groups))
}

/// 合并短段（≤5 字、时长 <3s、间距 <1.0s）到前一段，让 LLM 看⻅完整上下文
fn merge_short_segments(segments: &[TranscriptSegment]) -> (Vec<TranscriptSegment>, Vec<(usize, usize)>) {
    let mut merged: Vec<TranscriptSegment> = Vec::new();
    let mut groups: Vec<(usize, usize)> = Vec::new();
    let mut i = 0;
    while i < segments.len() {
        let group_start = i;
        let mut text = segments[i].text.clone();
        let start = segments[i].start;
        let mut end = segments[i].end;

        // 尝试向前合并短段
        while i + 1 < segments.len() {
            let dur = segments[i + 1].end - segments[i + 1].start;
            let text_len = segments[i + 1].text.trim().chars().count();
            let gap = segments[i + 1].start - segments[i].end;

            let is_short = dur < 1.0 || (dur < 3.0 && text_len <= 5);
            if !is_short || gap >= 1.0 {
                break;
            }
            text = format!("{}{}", text.trim(), segments[i + 1].text.trim());
            end = segments[i + 1].end;
            i += 1;
        }

        merged.push(TranscriptSegment {
            start,
            end,
            text,
            confidence: segments[group_start].confidence,
        });
        groups.push((group_start, i));
        i += 1;
    }
    (merged, groups)
}

/// 将合并组的订正结果按比例拆回原始分段数
fn expand_merged(
    corrected: &[String],
    segments: &[TranscriptSegment],
    groups: &[(usize, usize)],
) -> Vec<TranscriptSegment> {
    let mut result = segments.to_vec();
    for (group_idx, &(start, end)) in groups.iter().enumerate() {
        let corr = &corrected[group_idx];
        if start == end {
            result[start].text = corr.clone();
        } else {
            // 按原始文本⻓度比例拆分
            let orig_lens: Vec<usize> = (start..=end)
                .map(|k| segments[k].text.trim().chars().count())
                .collect();
            let total_orig: usize = orig_lens.iter().sum();
            let corr_len = corr.chars().count();
            let mut char_offset = 0;
            for (k, &orig_len) in (start..=end).zip(&orig_lens) {
                let part = if total_orig > 0 {
                    (corr_len as f64 * orig_len as f64 / total_orig as f64).round() as usize
                } else {
                    1
                };
                let o = char_offset.min(corr_len);
                let e = (char_offset + part).min(corr_len);
                result[k].text = corr[o..e].to_string();
                char_offset = e;
            }
        }
    }
    result
}

async fn send_batch(
    segments: &[TranscriptSegment],
    context: &[TranscriptSegment],
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

    // 构建待修正批次的 JSON
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
    let user_prompt = if !correct.prompt.is_empty() {
        correct.prompt.clone()
    } else {
        DEFAULT_USER_PROMPT.to_string()
    };
    let prompt = user_prompt
        .replace("{context}", &context_text)
        .replace("{segments}", &seg_text);

    // 系统消息
    let system_msg = if !correct.system_prompt.is_empty() {
        correct.system_prompt.clone()
    } else {
        DEFAULT_SYSTEM_PROMPT.to_string()
    };

    let messages = serde_json::json!([
        {"role": "system", "content": system_msg},
        {"role": "user", "content": prompt}
    ]);
    let body = crate::detection::build_request_body(llm, messages);
    let resp_body = crate::detection::send_request(llm, &body, 120).await?;
    let content = resp_body["choices"][0]["message"]["content"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("LLM 响应缺少 choices[0].message.content"))?;

    let mut corrected = parse_json_result(content, offset, segments.len())?;

    // 对未返回的条目保留原文
    for (i, text) in corrected.iter_mut().enumerate() {
        if text.is_empty() || text == "..." {
            if i < segments.len() {
                text.clone_from(&segments[i].text);
            }
        }
    }

    Ok(corrected)
}

/// 从 LLM 返回内容中解析 JSON 格式的订正结果
fn parse_json_result(content: &str, offset: usize, expected: usize) -> Result<Vec<String>> {
    let mut result = vec![String::new(); expected];

    // 提取 JSON 数组（处理可能的首尾空白或包裹的 markdown）
    let content = content.trim();
    let json_str = if let Some(pos) = content.find("```json") {
        let after = &content[pos + 7..];
        after.split("```").next().unwrap_or(content).trim()
    } else if let Some(pos) = content.find('[') {
        let end = content.rfind(']').map(|i| i + 1).unwrap_or(content.len());
        content[pos..end].trim()
    } else {
        content
    };

    let parsed: Vec<serde_json::Value> = match serde_json::from_str(json_str) {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!(
                "JSON 解析失败 (offset={}, expected={}): {}",
                offset,
                expected,
                e
            );
            return Ok(result);
        }
    };

    for item in parsed {
        let idx = item
            .get("index")
            .and_then(|v| v.as_u64())
            .map(|n| n as usize);
        let text = item.get("text").and_then(|v| v.as_str());

        if let (Some(idx), Some(text)) = (idx, text) {
            if idx >= offset + 1 && idx <= offset + expected {
                result[idx - offset - 1] = text.to_string();
            }
        }
    }

    Ok(result)
}
