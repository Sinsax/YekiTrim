use std::path::Path;

use anyhow::Result;

use crate::types::{Timestamp, Transcript, TranscriptSegment};

/// A raw SRT entry (start, end, text)
#[derive(Debug, Clone)]
pub struct SrtEntry {
    pub start: f64,
    pub end: f64,
    pub text: String,
}

/// 从 SRT 字符串解析 Transcript
pub fn parse_srt(content: &str) -> Result<Transcript> {
    let entries = parse_entries(content);
    Ok(Transcript {
        segments: entries
            .into_iter()
            .map(|e| TranscriptSegment {
                start: e.start,
                end: e.end,
                text: e.text,
                confidence: 1.0,
            })
            .collect(),
        language: "zh".into(),
        asr_metadata: None,
    })
}

/// 从 SRT 字符串解析为原始条目列表（非严格，跳过格式错误的块）
pub fn parse_entries(content: &str) -> Vec<SrtEntry> {
    let mut entries = Vec::new();
    let normalized = content.replace("\r\n", "\n");
    let blocks: Vec<&str> = normalized.trim().split("\n\n").collect();

    for block in blocks {
        let lines: Vec<&str> = block.trim().lines().collect();
        if lines.len() < 3 {
            continue;
        }
        let time_line = match lines.get(1) {
            Some(l) => l,
            None => continue,
        };
        let time_line = time_line.trim();
        let Some(delim) = time_line.find(" --> ") else { continue };
        let start_str = time_line[..delim].trim();
        let end_str = time_line[delim + 5..].trim();
        let Some(start) = parse_srt_time_loose(start_str) else { continue };
        let Some(end) = parse_srt_time_loose(end_str) else { continue };
        let text = lines[2..].join(" ").trim().to_string();
        if text.is_empty() {
            continue;
        }
        entries.push(SrtEntry {
            start,
            end,
            text,
        });
    }
    entries
}

/// 宽松的 SRT 时间解析（不返回错误，无法解析时返回 None）
pub fn parse_srt_time_loose(s: &str) -> Option<f64> {
    let s = s.replace(',', ".");
    let parts: Vec<&str> = s.split(':').collect();
    if parts.len() == 3 {
        let h: f64 = parts[0].parse().ok()?;
        let m: f64 = parts[1].parse().ok()?;
        let sec: f64 = parts[2].parse().ok()?;
        Some(h * 3600.0 + m * 60.0 + sec)
    } else {
        None
    }
}

pub fn write_srt(transcript: &Transcript, path: &Path) -> Result<()> {
    use std::io::Write;

    let mut f = std::fs::File::create(path)?;
    f.write_all(b"\xEF\xBB\xBF")?;
    for (i, seg) in transcript.segments.iter().enumerate() {
        let start = Timestamp::from_seconds(seg.start);
        let end = Timestamp::from_seconds(seg.end);
        writeln!(f, "{}", i + 1)?;
        writeln!(
            f,
            "{} --> {}",
            start.to_srt_format(),
            end.to_srt_format()
        )?;
        writeln!(f, "{}", seg.text)?;
        writeln!(f)?;
    }
    Ok(())
}
