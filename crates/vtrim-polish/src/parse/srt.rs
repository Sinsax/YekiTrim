use serde::{Deserialize, Serialize};

/// 单条字幕
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Subtitle {
    pub index: usize,
    pub start: f64,
    pub end: f64,
    pub text: String,
    #[serde(default)]
    pub html: String,
}

/// 解析标准 SRT 文本为字幕列表
pub fn parse(content: &str) -> Vec<Subtitle> {
    let entries = vtrim_core::srt::parse_entries(content);
    entries
        .into_iter()
        .enumerate()
        .map(|(i, e)| Subtitle {
            index: i + 1,
            start: e.start,
            end: e.end,
            html: e.text.clone(),
            text: e.text,
        })
        .collect()
}

/// 从文件路径解析
pub fn parse_file(path: &str) -> anyhow::Result<Vec<Subtitle>> {
    let content = std::fs::read_to_string(path)?;
    Ok(parse(&content))
}
