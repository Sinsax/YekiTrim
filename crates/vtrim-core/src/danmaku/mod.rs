mod parser;
pub use parser::*;

use std::path::Path;

use anyhow::{Context, Result};

/// Load danmaku from an ASS file as a flat list (for correction context, etc.)
pub fn load_danmaku(path: &Path) -> Result<Vec<DanmakuEntry>> {
    parser::parse_ass(path)
}

/// 解析 B站弹幕 SRT (含 {\move}, {\c} 标签)
pub fn parse_srt_danmaku(content: &str) -> Vec<DanmakuEntry> {
    let mut entries = Vec::new();
    let normalized = content.replace("\r\n", "\n");
    let blocks: Vec<&str> = normalized.trim().split("\n\n").collect();

    // 正则只编译一次（循环内编译是性能浪费）
    let time_re = regex_lite::Regex::new(
        r"(\d{2}):(\d{2}):(\d{2})[,.]\d{3}\s*-->\s*(\d{2}):(\d{2}):(\d{2})[,.]\d{3}",
    )
    .expect("SRT 时间正则编译失败");

    for block in blocks {
        let lines: Vec<&str> = block.trim().lines().collect();
        if lines.len() < 3 {
            continue;
        }

        let time_match = lines.get(1).and_then(|line| time_re.captures(line));

        let (start, end) = match time_match {
            Some(ref caps) => (parse_srt_time(caps, 1), parse_srt_time(caps, 4)),
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

/// 从 SRT 弹幕文件路径解析
pub fn parse_srt_file(path: &Path) -> Result<Vec<DanmakuEntry>> {
    let content =
        std::fs::read_to_string(path).with_context(|| format!("读取弹幕文件失败: {}", path.display()))?;
    Ok(parse_srt_danmaku(&content))
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

fn parse_srt_time(caps: &regex_lite::Captures, offset: usize) -> f64 {
    let h: f64 = caps[offset].parse().unwrap_or(0.0);
    let m: f64 = caps[offset + 1].parse().unwrap_or(0.0);
    let s: f64 = caps[offset + 2].parse().unwrap_or(0.0);
    h * 3600.0 + m * 60.0 + s
}

/// 自动检测弹幕文件格式（录播姬 XML / ASS / SRT）并解析。
/// - `.xml`（录播姬/B站标准弹幕 XML）→ [`parse_bililive_xml`]
/// - 含 `[Events]` + `Dialogue:` → ASS
/// - 其余按 SRT
pub fn load_danmaku_auto(path: &Path) -> Result<Vec<DanmakuEntry>> {
    let is_xml = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.eq_ignore_ascii_case("xml"))
        .unwrap_or(false);
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("读取弹幕文件失败: {}", path.display()))?;
    if is_xml || content.trim_start().starts_with("<?xml") || content.contains("<d p=") {
        return Ok(parse_bililive_xml(&content));
    }
    if content.contains("[Events]") && content.contains("Dialogue:") {
        parser::parse_ass(path)
    } else {
        Ok(parse_srt_danmaku(&content))
    }
}

/// 解析（录播姬写出的）B站标准弹幕 XML。
///
/// 结构：`<i><d p="时间,模式,字号,颜色,时间戳,弹幕池,用户,行号">文本</d>…</i>`
/// - 时间：秒（浮点）；模式：1/2/3 滚动、4 底端、5 顶端、6 逆向、7 高级、8 代码；
/// - 颜色：十进制 RGB（`16777215` = 白）；行号（第 8 段）直接当作 y 坐标。
///
/// 不引入 XML 依赖：只按 `<d p="…">…</d>` 提取，属性顺序不敏感，实体做最小反转义。
/// 得到的时间轴语义与 ASS/SRT 分支一致（`start`/`end`/`text`/`color`/`y`），
/// 因此 `popularity` 计算不需要任何改动。
pub fn parse_bililive_xml(content: &str) -> Vec<DanmakuEntry> {
    let re = regex_lite::Regex::new(r#"(?s)<d\s+p="([^"]*)"\s*>(.*?)</d>"#)
        .expect("弹幕 XML 正则编译失败");
    let mut entries = Vec::new();
    for caps in re.captures_iter(content) {
        let attrs = caps.get(1).map(|m| m.as_str()).unwrap_or("");
        let raw_text = caps.get(2).map(|m| m.as_str()).unwrap_or("");
        let text = unescape_xml(raw_text).trim().to_string();
        if text.is_empty() {
            continue;
        }
        let parts: Vec<&str> = attrs.split(',').collect();
        let start: f64 = parts.first().and_then(|s| s.trim().parse().ok()).unwrap_or(-1.0);
        if start < 0.0 {
            continue;
        }
        let mode: u32 = parts.get(1).and_then(|s| s.trim().parse().ok()).unwrap_or(1);
        let color_dec: u32 = parts.get(3).and_then(|s| s.trim().parse().ok()).unwrap_or(0xFF_FFFF);
        let row: f64 = parts
            .get(7)
            .and_then(|s| s.trim().parse().ok())
            .filter(|v: &f64| *v > 0.0)
            .unwrap_or(default_y_for_mode(mode));
        entries.push(DanmakuEntry {
            start,
            // XML 不提供时长：滚动弹幕按 8 秒展示，固定弹幕按 4 秒（与 ASS 默认一致）
            end: start + if matches!(mode, 4 | 5) { 4.0 } else { 8.0 },
            text,
            color: format!("#{:06x}", color_dec & 0xFF_FFFF),
            y: row.clamp(0.0, 1500.0),
            travel_duration: 8.0,
        });
    }
    entries.sort_by(|a, b| a.start.partial_cmp(&b.start).unwrap_or(std::cmp::Ordering::Equal));
    entries
}

fn default_y_for_mode(mode: u32) -> f64 {
    match mode {
        4 => 1000.0, // 底端
        5 => 100.0,  // 顶端
        _ => 400.0,  // 滚动（与 ASS/SRT 默认一致）
    }
}

/// 最小 XML 实体反转义（弹幕文本里会出现 `&lt;` `&amp;` 等与数字实体）。
fn unescape_xml(raw: &str) -> String {
    if !raw.contains('&') {
        return raw.to_string();
    }
    let mut out = String::with_capacity(raw.len());
    let mut rest = raw;
    while let Some(idx) = rest.find('&') {
        out.push_str(&rest[..idx]);
        let tail = &rest[idx..];
        let Some(semi) = tail.find(';').filter(|s| *s <= 10) else {
            out.push('&');
            rest = &tail[1..];
            continue;
        };
        let entity = &tail[1..semi];
        let replacement = match entity {
            "lt" => Some('<'.to_string()),
            "gt" => Some('>'.to_string()),
            "amp" => Some('&'.to_string()),
            "quot" => Some('"'.to_string()),
            "apos" => Some('\''.to_string()),
            _ => entity
                .strip_prefix('#')
                .and_then(|n| {
                    if let Some(hex) = n.strip_prefix('x').or_else(|| n.strip_prefix('X')) {
                        u32::from_str_radix(hex, 16).ok()
                    } else {
                        n.parse::<u32>().ok()
                    }
                })
                .and_then(char::from_u32)
                .map(|c| c.to_string()),
        };
        match replacement {
            Some(s) => out.push_str(&s),
            None => out.push_str(&tail[..=semi]),
        }
        rest = &tail[semi + 1..];
    }
    out.push_str(rest);
    out
}

/// Per-window danmaku density.
#[derive(Debug, Clone)]
pub struct DensityWindow {
    pub start: f64,
    /// Normalized density 0.0 ~ 1.0
    pub density: f64,
    /// Raw danmaku count in this window
    pub count: u64,
}

/// Analyze danmaku file and produce a normalized density heatmap.
///
/// * `path` — path to ASS file
/// * `window_secs` — bucket size in seconds (e.g. 10.0)
/// * `total_duration` — total video duration in seconds (used for window count)
///
/// Returns a vector of density windows sorted by start time.
pub fn analyze(path: &Path, window_secs: f64, total_duration: f64) -> Result<Vec<DensityWindow>> {
    let entries = load_danmaku_auto(path)?;
    if entries.is_empty() {
        return Ok(Vec::new());
    }

    let num_windows = (total_duration / window_secs).ceil() as usize;
    let mut counts = vec![0u64; num_windows];

    for entry in &entries {
        // Map the midpoint of each danmaku to a window
        let mid = (entry.start + entry.end) / 2.0;
        let idx = (mid / window_secs) as usize;
        if idx < num_windows {
            counts[idx] += 1;
        }
    }

    let max_count = counts.iter().copied().max().unwrap_or(1).max(1);

    let heatmap: Vec<DensityWindow> = counts
        .into_iter()
        .enumerate()
        .map(|(i, count)| DensityWindow {
            start: i as f64 * window_secs,
            density: count as f64 / max_count as f64,
            count,
        })
        .collect();

    Ok(heatmap)
}

/// Compute the average danmaku popularity for a time range [start, end).
pub fn average_density(heatmap: &[DensityWindow], start: f64, end: f64, window_secs: f64) -> f64 {
    if heatmap.is_empty() {
        return 0.0;
    }

    let start_idx = (start / window_secs) as usize;
    // end 恰在窗口边界时不包含该窗口（如 end=30.0 时不含 30-40 窗口）
    let end_idx = (((end - 1e-9) / window_secs) as usize).min(heatmap.len().saturating_sub(1));

    if start_idx > end_idx {
        return 0.0;
    }

    let mut total = 0.0;
    let mut count = 0u32;
    for w in heatmap.iter().take(end_idx + 1).skip(start_idx) {
        total += w.density;
        count += 1;
    }

    if count == 0 {
        0.0
    } else {
        total / count as f64
    }
}

/// 事件时间范围 [start, end) 内的弹幕峰值窗口密度
/// （峰值比均值更能代表"名场面"：事件内 30s 的爆发不会被整体时长稀释）
pub fn peak_density(heatmap: &[DensityWindow], start: f64, end: f64, window_secs: f64) -> f64 {
    if heatmap.is_empty() {
        return 0.0;
    }

    let start_idx = (start / window_secs) as usize;
    let end_idx = (((end - 1e-9) / window_secs) as usize).min(heatmap.len().saturating_sub(1));

    if start_idx > end_idx {
        return 0.0;
    }

    heatmap
        .iter()
        .take(end_idx + 1)
        .skip(start_idx)
        .map(|w| w.density)
        .fold(0.0, f64::max)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_peak_density() {
        let windows = vec![
            DensityWindow { start: 0.0, density: 0.5, count: 5 },
            DensityWindow { start: 10.0, density: 1.0, count: 10 },
            DensityWindow { start: 20.0, density: 0.0, count: 0 },
            DensityWindow { start: 30.0, density: 0.8, count: 8 },
        ];
        // 峰值 = 事件内最大窗口密度
        assert_eq!(peak_density(&windows, 5.0, 25.0, 10.0), 1.0);
        assert_eq!(peak_density(&windows, 0.0, 10.0, 10.0), 0.5);
        assert_eq!(peak_density(&windows, 30.0, 40.0, 10.0), 0.8);
        // 空范围
        assert_eq!(peak_density(&windows, 40.0, 35.0, 10.0), 0.0);
        // 空热力图
        assert_eq!(peak_density(&[], 0.0, 10.0, 10.0), 0.0);
    }

    #[test]
    fn test_analyze_empty() {
        let dir = std::env::temp_dir();
        let path = dir.join("empty_test.ass");
        std::fs::write(&path, "[Events]\nFormat: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\n").unwrap();
        let result = analyze(&path, 10.0, 100.0).unwrap();
        assert!(result.is_empty());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_density_window() {
        let windows = vec![
            DensityWindow { start: 0.0, density: 0.5, count: 5 },
            DensityWindow { start: 10.0, density: 1.0, count: 10 },
            DensityWindow { start: 20.0, density: 0.0, count: 0 },
        ];
        let avg = average_density(&windows, 5.0, 15.0, 10.0);
        assert!((avg - 0.75).abs() < 1e-6);
    }

    #[test]
    fn test_parse_srt_danmaku_basic() {
        let content = "1\n00:00:01,000 --> 00:00:05,000\n第一条弹幕\n\n2\n00:00:06,000 --> 00:00:08,000\n第二条弹幕\n";
        let entries = parse_srt_danmaku(content);
        assert_eq!(entries.len(), 2);
        assert!((entries[0].start - 1.0).abs() < 1e-6);
        assert!((entries[0].end - 5.0).abs() < 1e-6);
        assert_eq!(entries[0].text, "第一条弹幕");
        assert_eq!(entries[0].color, "#ffffff");
        assert!((entries[0].y - 400.0).abs() < 1e-6);
        assert!((entries[1].start - 6.0).abs() < 1e-6);
    }

    #[test]
    fn test_parse_srt_danmaku_tags() {
        // {\c&H3366FF} 蓝色 (BGR) → #FF6633; {\move} 提取 y; 标签被移除
        let content = "1\n00:00:01,000 --> 00:00:05,000\n{\\c&H3366FF}{\\move(100,300,500,300)}蓝色弹幕\n";
        let entries = parse_srt_danmaku(content);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].text, "蓝色弹幕");
        assert_eq!(entries[0].color, "#FF6633");
        assert!((entries[0].y - 300.0).abs() < 1e-6);
    }

    #[test]
    fn test_parse_srt_danmaku_empty_and_junk() {
        assert!(parse_srt_danmaku("").is_empty());
        assert!(parse_srt_danmaku("\n\n\n").is_empty());
        // 无效时间行 → 跳过
        let bad = "1\nnot-a-time\n文本\n";
        assert!(parse_srt_danmaku(bad).is_empty());
        // 纯标签无文本 → 跳过
        let only_tags = "1\n00:00:01,000 --> 00:00:05,000\n{\\move(100,300,500,300)}\n";
        assert!(parse_srt_danmaku(only_tags).is_empty());
    }

    /// 录播姬写出的 B站标准弹幕 XML（官方样例结构）
    const BILILIVE_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<i>
  <chatserver>chat.bilibili.com</chatserver>
  <chatid>0</chatid>
  <mission>0</mission>
  <maxlimit>0</maxlimit>
  <state>0</state>
  <real_name>0</real_name>
  <source>k-v</source>
  <d p="1.5,1,25,16777215,1700000000,0,abc,120">第一条 &lt;滚动&gt; 弹幕</d>
  <d p="12.25,4,25,16711935,1700000010,0,abc,1000">底端弹幕</d>
  <d p="20,5,25,255,1700000020,0,abc,100">顶端弹幕</d>
  <d p="30,1,25,16711680,1700000030,0,abc,340"></d>
</i>"#;

    #[test]
    fn test_parse_bililive_xml() {
        let entries = parse_bililive_xml(BILILIVE_XML);
        // 空文本的一条被跳过
        assert_eq!(entries.len(), 3, "entries = {entries:?}");

        assert!((entries[0].start - 1.5).abs() < 1e-6);
        assert_eq!(entries[0].text, "第一条 <滚动> 弹幕", "XML 实体必须反转义");
        assert_eq!(entries[0].color, "#ffffff");
        assert!((entries[0].y - 120.0).abs() < 1e-6, "行号直接作为 y");
        assert!((entries[0].end - 9.5).abs() < 1e-6, "滚动弹幕显示 8 秒");

        // 底端（mode=4）：固定 4 秒 + 默认 y
        assert!((entries[1].start - 12.25).abs() < 1e-6);
        assert_eq!(entries[1].text, "底端弹幕");
        assert_eq!(entries[1].color, "#ff00ff");
        assert!((entries[1].end - 16.25).abs() < 1e-6);

        // 顶端（mode=5）+ 颜色 255 = #0000ff
        assert_eq!(entries[2].color, "#0000ff");
        assert!((entries[2].y - 100.0).abs() < 1e-6);

        // 按时间排序
        assert!(entries.windows(2).all(|w| w[0].start <= w[1].start));
    }

    #[test]
    fn test_parse_bililive_xml_junk() {
        assert!(parse_bililive_xml("").is_empty());
        assert!(parse_bililive_xml("<i></i>").is_empty());
        // 缺 p 或时间非法 → 跳过
        assert!(parse_bililive_xml(r#"<i><d>无属性</d></i>"#).is_empty());
        assert!(parse_bililive_xml(r#"<i><d p="abc,1,25,1,0,0,x,1">x</d></i>"#).is_empty());
        // 数字实体与未知实体
        let e = parse_bililive_xml(r#"<i><d p="1,1,25,16777215,0,0,u,10">A&#65;&unknown;B</d></i>"#);
        assert_eq!(e[0].text, "AA&unknown;B");
    }

    #[test]
    fn test_load_danmaku_auto_detects_xml_by_extension() {
        let dir = std::env::temp_dir().join(format!("vtrim-danmaku-xml-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("danmaku.xml");
        std::fs::write(&path, BILILIVE_XML).unwrap();
        let entries = load_danmaku_auto(&path).unwrap();
        assert_eq!(entries.len(), 3, "按扩展名分派到 XML 解析");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
