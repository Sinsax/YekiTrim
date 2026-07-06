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

/// 解析标准 SRT 文本
pub fn parse(content: &str) -> Vec<Subtitle> {
    let mut entries = Vec::new();
    let normalized = content.replace("\r\n", "\n");
    let blocks: Vec<&str> = normalized.trim().split("\n\n").collect();

    for block in blocks {
        let lines: Vec<&str> = block.trim().lines().collect();
        if lines.len() < 3 {
            continue;
        }

        // 时间行: 00:00:01,234 --> 00:00:04,567
        let time_match = lines.get(1).and_then(|line| {
            let re = regex_lite::Regex::new(
                r"(\d{2}):(\d{2}):(\d{2})[,.](\d{3})\s*-->\s*(\d{2}):(\d{2}):(\d{2})[,.](\d{3})",
            )
            .ok()?;
            re.captures(line)
        });

        let (start, end) = match time_match {
            Some(caps) => {
                let s = parse_time(&caps, 1);
                let e = parse_time(&caps, 5);
                (s, e)
            }
            None => continue,
        };

        let index: usize = lines[0].trim().parse().unwrap_or(entries.len() + 1);
        let text = lines[2..].join(" ").trim().to_string();
        if text.is_empty() {
            continue;
        }

        entries.push(Subtitle {
            index,
            start,
            end,
            html: text.clone(),
            text,
        });
    }

    entries
}

/// 从文件路径解析
pub fn parse_file(path: &str) -> anyhow::Result<Vec<Subtitle>> {
    let content = std::fs::read_to_string(path)?;
    Ok(parse(&content))
}

fn parse_time(caps: &regex_lite::Captures, offset: usize) -> f64 {
    let h: f64 = caps[offset + 0].parse().unwrap_or(0.0);
    let m: f64 = caps[offset + 1].parse().unwrap_or(0.0);
    let s: f64 = caps[offset + 2].parse().unwrap_or(0.0);
    let ms: f64 = caps[offset + 3].parse().unwrap_or(0.0);
    h * 3600.0 + m * 60.0 + s + ms / 1000.0
}
