use std::path::Path;

use anyhow::Result;

use crate::types::{Timestamp, Transcript, TranscriptSegment};

/// 从 SRT 字符串解析 Transcript
pub fn parse_srt(content: &str) -> Result<Transcript> {
    let mut segments = Vec::new();
    let mut lines = content.lines().peekable();

    while lines.peek().is_some() {
        let _ = lines.next();
        let time_line = match lines.next() {
            Some(l) => l,
            None => break,
        };
        let (start_str, end_str) = time_line
            .split_once(" --> ")
            .ok_or_else(|| anyhow::anyhow!("SRT 时间格式无效: {}", time_line))?;
        let start = parse_srt_time(start_str.trim())?;
        let end = parse_srt_time(end_str.trim())?;
        let mut text = String::new();
        while let Some(l) = lines.next() {
            if l.trim().is_empty() {
                break;
            }
            if !text.is_empty() {
                text.push(' ');
            }
            text.push_str(l.trim());
        }
        if !text.is_empty() {
            segments.push(TranscriptSegment {
                start,
                end,
                text,
                confidence: 1.0,
            });
        }
    }

    Ok(Transcript {
        segments,
        language: "zh".into(),
    })
}

fn parse_srt_time(s: &str) -> Result<f64> {
    let s = s.replace(',', ".");
    let parts: Vec<&str> = s.split(':').collect();
    if parts.len() == 3 {
        let h: f64 = parts[0].parse().map_err(|e| anyhow::anyhow!("SRT 时间解析失败: {}", e))?;
        let m: f64 = parts[1].parse()?;
        let sec: f64 = parts[2].parse()?;
        Ok(h * 3600.0 + m * 60.0 + sec)
    } else {
        anyhow::bail!("SRT 时间格式异常: {}", s)
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
