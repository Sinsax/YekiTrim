use std::sync::Arc;

use anyhow::{Context, Result};
use indicatif::{ProgressBar, ProgressStyle};
use tokio::sync::Semaphore;
use tokio::task::JoinSet;

use crate::config::LlmConfig;
use crate::types::TranscriptSegment;

const BATCH_SIZE: usize = 500;
const MAX_CONCURRENT: usize = 3;

pub async fn correct_transcript(
    segments: &[TranscriptSegment],
    llm: &LlmConfig,
) -> Result<Vec<TranscriptSegment>> {
    let total = segments.len();
    let total_batches = (total + BATCH_SIZE - 1) / BATCH_SIZE;
    let mut corrected = segments.to_vec();
    let llm = Arc::new(llm.clone());
    let sem = Arc::new(Semaphore::new(MAX_CONCURRENT));
    let mut set: JoinSet<(usize, usize, Result<Vec<String>>)> = JoinSet::new();

    let pb = ProgressBar::new(total_batches as u64);
    if let Ok(style) = ProgressStyle::with_template(
        "{spinner} 订正: [{bar:24}] {pos}/{len} 批 ({percent}%) [{elapsed_precise}]",
    ) {
        pb.set_style(style.progress_chars("##-"));
    }
    pb.enable_steady_tick(std::time::Duration::from_millis(100));

    for (batch_idx, chunk) in segments.chunks(BATCH_SIZE).enumerate() {
        let chunk = chunk.to_vec();
        let llm = llm.clone();
        let sem = sem.clone();
        let offset = batch_idx * BATCH_SIZE;
        let batch_num = batch_idx + 1;
        let chunk_len = chunk.len();

        set.spawn(async move {
            let _permit = sem.acquire().await.unwrap();
            tracing::info!("  批次 {} 订正中 ({} 段)", batch_num, chunk_len);
            (offset, batch_num, send_batch(&chunk, &llm, offset).await)
        });
    }

    while let Some(result) = set.join_next().await {
        match result {
            Ok((offset, batch_num, Ok(texts))) => {
                for (i, text) in texts.iter().enumerate() {
                    if i < BATCH_SIZE.min(total.saturating_sub(offset)) {
                        corrected[offset + i].text.clone_from(text);
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
    Ok(corrected)
}

async fn send_batch(
    segments: &[TranscriptSegment],
    llm: &LlmConfig,
    offset: usize,
) -> Result<Vec<String>> {
    let mut prompt = String::from(
        "请修正以下语音转文字转录中的错别字、人名、专业术语。\n\
         按原样返回每一行，只修改文字内容，不要修改序号。\n\
         格式：序号|修正后文字\n\n\
         转录内容：\n",
    );
    for (i, seg) in segments.iter().enumerate() {
        prompt.push_str(&format!("{}|{}\n", offset + i + 1, seg.text));
    }

    if llm.api_base.is_empty() {
        anyhow::bail!("未配置 LLM API 基础地址 (llm.api_base)，请设置后重试");
    }
    let api_url = {
        let base = llm.api_base.trim_end_matches('/');
        if base.ends_with("/chat/completions") {
            base.to_string()
        } else {
            format!("{}/chat/completions", base)
        }
    };

    let mut body = serde_json::json!({
        "temperature": llm.temperature,
        "max_tokens": llm.max_tokens,
        "messages": [
            {"role": "system", "content": "你是语音转文字订正助手。修正错别字、人名、术语，保持序号不变，只输出行。"},
            {"role": "user", "content": prompt}
        ]
    });
    if !llm.model.is_empty() {
        body["model"] = serde_json::json!(llm.model);
    }
    let client = reqwest::Client::new();
    let resp = client
        .post(&api_url)
        .header("Authorization", format!("Bearer {}", llm.api_key))
        .json(&body)
        .send()
        .await
        .context("LLM 订正 API 请求失败")?;

    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        anyhow::bail!("LLM 订正 API 返回 {}: {}", status, body);
    }

    let resp_body: serde_json::Value = resp.json().await.context("解析 LLM 响应失败")?;
    let content = resp_body["choices"][0]["message"]["content"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("LLM 响应缺少 choices[0].message.content"))?;

    parse_lines(content, offset, segments.len())
}

fn parse_lines(content: &str, offset: usize, expected: usize) -> Result<Vec<String>> {
    let mut result = vec![String::new(); expected];

    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let parts: Vec<&str> = line.splitn(2, '|').collect();
        if parts.len() != 2 {
            continue;
        }
        let idx: usize = match parts[0].trim().parse() {
            Ok(n) => n,
            Err(_) => continue,
        };
        if idx < offset + 1 || idx > offset + expected {
            continue;
        }
        result[idx - offset - 1] = parts[1].trim().to_string();
    }

    for item in result.iter_mut() {
        if item.is_empty() {
            item.push_str("...");
        }
    }

    Ok(result)
}