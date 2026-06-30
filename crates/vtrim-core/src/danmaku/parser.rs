use std::path::Path;

use anyhow::{Context, Result};

/// A single danmaku entry with timing and text.
#[derive(Debug, Clone)]
pub struct DanmakuEntry {
    pub start: f64,
    pub end: f64,
    pub text: String,
}

/// Parse an ASS subtitle file and extract danmaku entries.
/// Supports the standard ASS format with [Events] section.
pub fn parse_ass(path: &Path) -> Result<Vec<DanmakuEntry>> {
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("读取弹幕文件失败: {}", path.display()))?;

    let mut entries = Vec::new();
    let mut in_events = false;

    for line in content.lines() {
        let line = line.trim();

        if line.starts_with("[Events]") {
            in_events = true;
            continue;
        }

        if !in_events || line.is_empty() || line.starts_with(';') || line.starts_with("Format:") {
            continue;
        }

        if line.starts_with("Dialogue:") {
            if let Some(entry) = parse_dialogue_line(line) {
                entries.push(entry);
            }
        }
    }

    Ok(entries)
}

/// Parse a single ASS Dialogue line.
/// Format: Dialogue: 0,0:00:01.00,0:00:05.00,Style,,0,0,0,,text
fn parse_dialogue_line(line: &str) -> Option<DanmakuEntry> {
    let body = line.strip_prefix("Dialogue:")?;
    let fields: Vec<&str> = body.splitn(10, ',').collect();
    if fields.len() < 10 {
        return None;
    }

    // fields[0] = layer, fields[1] = start, fields[2] = end, fields[9] = text
    let start = parse_ass_time(fields[1].trim())?;
    let end = parse_ass_time(fields[2].trim())?;
    let text = replace_ass_newlines(fields[9]);

    // skip empty or pure-newline text
    if text.trim().is_empty() {
        return None;
    }

    Some(DanmakuEntry { start, end, text })
}

/// Replace ASS line break markers `\N` and `\n` with real newlines (single pass).
fn replace_ass_newlines(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.peek() {
                Some(&'N') | Some(&'n') => {
                    chars.next();
                    result.push('\n');
                    continue;
                }
                _ => {}
            }
        }
        result.push(c);
    }
    result
}

/// Parse ASS time format: 0:00:01.00 or 0:00:01,00
fn parse_ass_time(s: &str) -> Option<f64> {
    let s = s.replace(',', ".");
    let parts: Vec<&str> = s.split(':').collect();
    if parts.len() != 3 {
        return None;
    }
    let h: f64 = parts[0].parse().ok()?;
    let m: f64 = parts[1].parse().ok()?;
    let sec: f64 = parts[2].parse().ok()?;
    Some(h * 3600.0 + m * 60.0 + sec)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_ass_time() {
        assert!((parse_ass_time("0:00:01.00").unwrap() - 1.0).abs() < 1e-3);
        assert!((parse_ass_time("1:02:03.45").unwrap() - 3723.45).abs() < 1e-3);
        assert!((parse_ass_time("0:00:00,50").unwrap() - 0.5).abs() < 1e-3);
    }

    #[test]
    fn test_parse_dialogue() {
        let line = "Dialogue: 0,0:00:01.00,0:00:05.00,Default,,0,0,0,,Hello world";
        let entry = parse_dialogue_line(line).unwrap();
        assert!((entry.start - 1.0).abs() < 1e-3);
        assert!((entry.end - 5.0).abs() < 1e-3);
        assert_eq!(entry.text, "Hello world");
    }

    #[test]
    fn test_parse_dialogue_with_comma() {
        let line = "Dialogue: 0,0:00:10.00,0:00:15.00,Default,,0,0,0,,Hello, world!";
        let entry = parse_dialogue_line(line).unwrap();
        assert!((entry.start - 10.0).abs() < 1e-3);
        assert_eq!(entry.text, "Hello, world!");
    }
}
