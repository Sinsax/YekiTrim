use serde::{Deserialize, Serialize};

/// 单条弹幕
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct DanmakuEntry {
    pub start: f64,
    pub end: f64,
    pub text: String,
    #[serde(default = "default_color")]
    pub color: String,
    #[serde(default)]
    pub y: f64,
    #[serde(default)]
    pub travel_duration: f64,
}

fn default_color() -> String {
    "#ffffff".into()
}

/// 解析 B站弹幕 SRT (含 {\move}, {\c} 标签)
pub fn parse(content: &str) -> Vec<DanmakuEntry> {
    let mut entries = Vec::new();
    let normalized = content.replace("\r\n", "\n");
    let blocks: Vec<&str> = normalized.trim().split("\n\n").collect();

    for block in blocks {
        let lines: Vec<&str> = block.trim().lines().collect();
        if lines.len() < 3 {
            continue;
        }

        let time_match = lines.get(1).and_then(|line| {
            let re = regex_lite::Regex::new(
                r"(\d{2}):(\d{2}):(\d{2})[,.](\d{3})\s*-->\s*(\d{2}):(\d{2}):(\d{2})[,.](\d{3})",
            )
            .ok()?;
            re.captures(line)
        });

        let (start, end) = match time_match {
            Some(ref caps) => (parse_time(caps, 1), parse_time(caps, 5)),
            None => continue,
        };

        let raw = lines[2..].join(" ").trim().to_string();
        if raw.is_empty() {
            continue;
        }

        // 解析颜色 {\c&HBBGGRR}
        let color = extract_color(&raw);

        // 解析位置 {\move(x1,y1,x2,y2)}
        let y = extract_y(&raw);

        // 去除标签
        let clean = remove_tags(&raw);
        if clean.is_empty() {
            continue;
        }

        entries.push(DanmakuEntry {
            start,
            end,
            text: clean,
            color,
            y,
            travel_duration: 8.0, // 默认
        });
    }

    entries
}

pub fn parse_file(path: &str) -> anyhow::Result<Vec<DanmakuEntry>> {
    let content = std::fs::read_to_string(path)?;
    Ok(parse(&content))
}

fn extract_color(raw: &str) -> String {
    let re = regex_lite::Regex::new(r"\\c&H([0-9A-Fa-f]{6})&?").ok();
    if let Some(caps) = re.and_then(|r| r.captures(raw)) {
        let bgr = caps.get(1).map(|m| m.as_str()).unwrap_or("FFFFFF");
        // BGR → RGB
        let rgb = format!(
            "#{}{}{}",
            &bgr[4..6],
            &bgr[2..4],
            &bgr[0..2]
        );
        return rgb;
    }
    "#ffffff".into()
}

fn extract_y(raw: &str) -> f64 {
    let re = regex_lite::Regex::new(r"\\move\((\d+),(\d+),(-?\d+),(-?\d+)\)").ok();
    if let Some(caps) = re.and_then(|r| r.captures(raw)) {
        if let Some(y) = caps.get(2) {
            if let Ok(val) = y.as_str().parse::<f64>() {
                return val.clamp(50.0, 1500.0);
            }
        }
    }
    400.0
}

fn remove_tags(raw: &str) -> String {
    let re = regex_lite::Regex::new(r"\{[^}]+\}").unwrap();
    re.replace_all(raw, "").trim().to_string()
}

fn parse_time(caps: &regex_lite::Captures, offset: usize) -> f64 {
    let h: f64 = caps[offset + 0].parse().unwrap_or(0.0);
    let m: f64 = caps[offset + 1].parse().unwrap_or(0.0);
    let s: f64 = caps[offset + 2].parse().unwrap_or(0.0);
    let ms: f64 = caps[offset + 3].parse().unwrap_or(0.0);
    h * 3600.0 + m * 60.0 + s + ms / 1000.0
}
