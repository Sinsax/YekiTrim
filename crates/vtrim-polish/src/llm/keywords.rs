use vtrim_core::llm::agent::extract_json_object;
use vtrim_core::llm::LlmClient;

/// 调用 LLM 提取关键词
pub async fn extract_keywords(
    client: &LlmClient,
    subtitle_texts: &[String],
    danmaku_texts: &[String],
) -> anyhow::Result<(Vec<String>, Vec<String>)> {
    let user_prompt = build_prompt(subtitle_texts, danmaku_texts);
    let response = client.chat(SYSTEM_PROMPT, &user_prompt).await?;
    let json = extract_json_object(&response)?;
    let result: KeywordResult = serde_json::from_str(&json)?;
    Ok((result.danmaku_filter, result.highlight_words))
}

#[derive(serde::Deserialize)]
struct KeywordResult {
    danmaku_filter: Vec<String>,
    highlight_words: Vec<String>,
}

const SYSTEM_PROMPT: &str = r#"你是一个视频编辑助手。分析以下字幕和弹幕内容，提取两类关键词:

1. danmaku_filter: 弹幕过滤关键词 — 与视频内容相关的弹幕保留关键词
2. highlight_words: 高亮关键词 — 需要显示高亮的主题词

输出 JSON 对象 (不要 markdown 包裹): {"danmaku_filter": [...], "highlight_words": [...]}
"#;

pub fn build_prompt(subtitle_texts: &[String], danmaku_texts: &[String]) -> String {
    let mut prompt = String::from("字幕内容:\n");
    for t in subtitle_texts.iter().take(80) {
        prompt.push_str(t);
        prompt.push('\n');
    }
    prompt.push_str("\n弹幕内容:\n");
    for t in danmaku_texts.iter().take(40) {
        prompt.push_str(t);
        prompt.push('\n');
    }
    prompt
}


