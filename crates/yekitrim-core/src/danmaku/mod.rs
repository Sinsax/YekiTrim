//! danmaku：弹幕解析（ASS / 录播姬 XML / SRT）与时间窗密度（热力图）。
//!
//! 入口：`load_danmaku_auto`（唯一该用的格式自适应入口）、`analyze`、`average_density`、`peak_density`、`parse_bililive_xml`、`parse_srt_file`、`emotion::track`（§10 情绪轨道）；`parser` 是私有子模块。
//! 边界：**别用 `parse_ass` 直读用户的 `danmaku_file`**（真实弹幕是 XML，会静默吐 0 条还不报错），格式判据只在 `load_danmaku_auto` 里有一份 —— 见 `crates/yekitrim-core/src/danmaku/AGENTS.md`

mod parser;
pub use parser::*;

/// 情绪轨道（§10）：从弹幕反推「观众情绪」。
///
/// ⚠️ **公开**（不像 `parser` 那样藏起来）：它的消费方是 `detection` 与
/// `yekitrim-polish` 里的演出/编排，不是"本模块内部实现细节"。
pub mod emotion;

use std::path::Path;

use anyhow::{bail, Context, Result};

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

/// 从 **SRT** 弹幕文件路径解析。
///
/// ⚠️ 这个函数**只认 SRT**：`danmaku_file` 指向 `.xml`/`.ass` 时，正确入口是
/// [`load_danmaku_auto`]。历史上有调用点用错（精修 `cmd_parse`），真实 B站 XML 被
/// SRT 分支解析成 **0 条**且不报错 —— 所以这里加一道护栏：内容看着像弹幕 XML 就直接
/// 报错，把"静默 0 条"变成"一眼可见的失败"。
pub fn parse_srt_file(path: &Path) -> Result<Vec<DanmakuEntry>> {
    let content =
        std::fs::read_to_string(path).with_context(|| format!("读取弹幕文件失败: {}", path.display()))?;
    let head = content.trim_start_matches('\u{feff}').trim_start();
    if head.starts_with("<?xml") || content.contains("<d p=") {
        bail!(
            "{} 是弹幕 XML，不是 SRT —— 请改用 load_danmaku_auto（XML/ASS/SRT 都认）",
            path.display()
        );
    }
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

/// 解析 B站标准弹幕 XML（`<i><d p="…">文本</d>…</i>`）。
///
/// **两种方言都要吃**，`p` 的段数与第 8 段含义不同：
///
/// | 来源 | 段数 | `[7]` 是什么 |
/// |---|---|---|
/// | 主站 `comment.bilibili.com/{cid}.xml` | **9** | 弹幕数据库 ID（17–19 位 dmid），`[8]` 是 4..10 的枚举 |
/// | 录播姬 `BililiveRecorder` | **8** | **恒为 `0`**（rec.danmuji.org/user/danmaku 字段说明） |
///
/// 两边一致的段（本函数只用这三个）：`[0]` 时间(秒,浮点) `[1]` 模式 `[3]` 颜色(十进制 RGB)。
/// 模式：1/2/3 滚动、4 底端、5 顶端、6 逆向、7 高级、8 代码。
///
/// ⚠️ **`[7]` 不作为 y 坐标** —— 它在两种方言里都不是行号，原因见函数内注释。
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
        // ⚠️ `parts[7]` **不是行号**，不要拿它当 y：
        //   - 主站 XML 放的是弹幕数据库 ID（实测 17 位，如 `22772981386706944`）；
        //   - 录播姬放的是固定 `0`（rec.danmuji.org/user/danmaku 字段说明）。
        // 早先这里写的是「第 8 段直接当 y」+ `.clamp(0.0, 1500.0)`：
        // 录播姬路径上因为恒 0 被 `filter(> 0.0)` 挡掉而"碰巧"没暴露，
        // 一旦吃主站 XML，`clamp` 会把**每一条**都压到 y = 1500（实测 3600/3600 条）。
        // 所以一律走 `default_y_for_mode`，与 ASS/SRT 分支的默认取值一致。
        let y = default_y_for_mode(mode);
        entries.push(DanmakuEntry {
            start,
            // XML 不提供时长：滚动弹幕按 8 秒展示，固定弹幕按 4 秒（与 ASS 默认一致）
            end: start + if matches!(mode, 4 | 5) { 4.0 } else { 8.0 },
            text,
            color: format!("#{:06x}", color_dec & 0xFF_FFFF),
            y,
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

/// 弹幕淡入的时长（秒）—— 渲染侧 `getActiveDms` 的 `fadeIn`
/// （`crates/yekitrim-polish/templates/index.html`，同一份也在 `webui/src/polish/engine.js`）。
pub const DANMAKU_FADE_IN_SECS: f64 = 0.3;

/// 窗口比较的容差（秒）：**1 ns**。
///
/// ⚠️ 不写严格相等是因为 `serde_json` 读 f64 会**差 1 ulp**（实测
/// `"10.700000000000001".parse::<f64>()` 的位是 `…6667`、而 `serde_json::from_str::<f64>()`
/// 是 `…6666`）。于是"算出来的需要长度"永远比刚写回去的那个值大 1 ulp，每次 process 都
/// 判定要抬一次 —— 内容一个字节没变、mtime 却一直翻新，而首页「切片总览」按 mtime 排序，
/// 等于把老片段一直顶在最前面（这正是 `normalize_danmaku_windows` 要避开的那个坑）。
/// 1 ns 远小于一帧（60fps = 16.7ms），不会吞掉真实的窗口差额。
const WINDOW_EPS_SECS: f64 = 1e-9;

/// 滚动弹幕**走完全程**要多久（秒）。
///
/// 唯一一份公式：`(1080 + UTF-8 字节数 × 32) / 150`。
/// ⚠️ `text.len()` 是 **UTF-8 字节数**（一个汉字 3 字节）—— 按字符数算会让中文弹幕滚快一倍，
/// 而"看起来差不多"正是这类错最难查的地方。`tools/polish-to-dhampir.mjs` 抄的是同一份
/// （那边必须先剥掉 `{\move(...)}` 标签再数字节）。
pub fn travel_duration_for(text: &str) -> f64 {
    (1080.0 + text.len() as f64 * 32.0) / 150.0
}

/// 把弹幕的**在屏窗口末尾**抬到"至少够它滚到左边出画"，返回被改动的条数。
///
/// # 为什么必须抬
///
/// 渲染侧是**两口独立的钟**：滚动由 `travel_duration` 决定
/// （`progress = (el - fadeIn) / travel`、`min(progress, 1)`），而**渐隐钉在窗口末尾**
/// （`d.end - 0.2` 起渐隐、`t >= d.end` 整条丢掉）—— 两者之间本来没有任何约束，
/// 于是窗口短于"淡入 + travel"的弹幕会在**还没走到左边缘**时提前渐隐，
/// 唯一的"已出屏"判据（`x < -textW - 50`）永远轮不到执行。
///
/// 实测：录播姬 XML 给的窗口是**固定 8 秒**（[`parse_bililive_xml`]），而 5 个汉字的
/// `travel` 就有 10.4 秒 —— 「还能续约吗」在距左边缘 **337px** 处开始渐隐，
/// 13 个汉字那条（39 字节、travel 15.52s）在**屏幕中段 728px** 处就没了。
/// 中文弹幕（1 个汉字 = 3 字节）几乎全部命中，越长越早。
///
/// `assign_danmaku_tracks` 占轨道用的本来就是 `start + travel`（分配器早就认为一条弹幕
/// 在屏 `travel` 秒），只有 `end` 这个字段没跟上 —— 这个函数把它对齐。
///
/// # 口径
///
/// * 抬到 `start + DANMAKU_FADE_IN_SECS + travel`：渐隐整段落在字**已经出画之后**。
/// * 只**抬**不压：素材里显式写了更长窗口的（滚到左边停一会儿再消失）原样保留。
/// * `travel_duration` 为 0 时按 **8.0** 兜底 —— 与渲染侧 `d.travel_duration || 8` 同口径。
/// * **幂等**（跨 `serde_json` 往返也逐字节不变）：比较带 1 ns 容差，见 [`WINDOW_EPS_SECS`]。
pub fn clamp_display_windows(entries: &mut [DanmakuEntry]) -> usize {
    let mut changed = 0;
    for entry in entries.iter_mut() {
        let travel = if entry.travel_duration > 0.0 {
            entry.travel_duration
        } else {
            8.0
        };
        let need = entry.start + DANMAKU_FADE_IN_SECS + travel;
        if entry.end + WINDOW_EPS_SECS < need {
            entry.end = need;
            changed += 1;
        }
    }
    changed
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
    // ⚠️ 窗口长来自配置，是**手改得动的**：0 / 负数 / NaN / inf 会让 `total / window` 变成 inf，
    // `inf.ceil() as usize` 饱和成 `usize::MAX`，紧接着那句 `vec![0u64; usize::MAX]` 触发
    // **容量溢出（进程 abort）** —— 任务连收尾都来不及。与 `emotion::track` 同口径：
    // 非法窗口 = 没有热力图（检测侧会记一条 warn 并退回字幕密度），绝不是崩掉。
    if !(window_secs.is_finite() && window_secs > 0.0) {
        bail!("弹幕统计窗口必须 > 0 秒（当前 {window_secs}）");
    }
    if !total_duration.is_finite() || total_duration <= 0.0 {
        return Ok(Vec::new());
    }
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

    /// ⚠️ 幂等的判据是"**不触发写回**"：`serde_json` 读 f64 会**差 1 ulp**
    /// （`"10.700000000000001".parse::<f64>()` 是 `…6667`，`serde_json::from_str::<f64>()` 是 `…6666`），
    /// 所以严格相等的话，"算出来的需要长度"永远比刚写回去的值大 1 ulp → 每次 process 都判定要抬一次，
    /// 内容一个字节没变、mtime 却一直翻新，而首页「切片总览」按 mtime 排序 —— 老片段被一直顶在最前面。
    ///
    /// 注意**不能**断言"往返逐字节相同"：`serde_json` 的往返本来就不是逐字节稳定的
    /// （上面那个 1 ulp 就足以让 ryu 打出另一个串）。这里钉的是"第二次跑不再判定要抬"。
    #[test]
    fn clamp_display_windows_is_idempotent_across_json_roundtrip() {
        let mut entries = vec![DanmakuEntry {
            start: 0.0,
            end: 1.0,
            text: "还能续约吗".into(),
            color: "#ffffff".into(),
            y: 300.0,
            travel_duration: 10.4,
        }];
        assert_eq!(clamp_display_windows(&mut entries), 1);
        let json = serde_json::to_string(&entries).unwrap();
        let mut again: Vec<DanmakuEntry> = serde_json::from_str(&json).unwrap();
        assert_eq!(
            clamp_display_windows(&mut again),
            0,
            "读回来之后不许再判定要抬（serde_json 的 f64 差 1 ulp）—— 否则每次 process 都重写"
        );
        // 再跑一遍仍然不抬：这就是"process 每次都调它、但只有真的变了才落盘"的完整判据。
        assert_eq!(clamp_display_windows(&mut again), 0);
    }

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

    /// 窗口长是配置项、**手改得动**：0 / 负数 / NaN / inf 曾让 `ceil() as usize` 饱和成
    /// `usize::MAX`，紧接着 `vec![0u64; usize::MAX]` 触发容量溢出（进程 abort，连任务收尾
    /// 都来不及）。现在非法值返回 Err（检测侧 warn + 退回字幕密度），与 `emotion::track` 同口径。
    #[test]
    fn analyze_rejects_invalid_window_instead_of_allocating_usize_max() {
        let path = std::env::temp_dir().join(format!(
            "yekitrim-analyze-window-{}.ass",
            std::process::id()
        ));
        std::fs::write(
            &path,
            "[Events]\nFormat: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\n\
             Dialogue: 0,0:00:01.00,0:00:02.00,Default,,0,0,0,,x\n",
        )
        .unwrap();
        for bad in [0.0, -10.0, f64::NAN, f64::INFINITY] {
            let err = analyze(&path, bad, 100.0).unwrap_err().to_string();
            assert!(err.contains("窗口"), "非法窗口长必须报错而不是分配: {bad} → {err}");
        }
        // 时长为 0 / NaN / inf ⇒ 没有窗口数可言 = 空热力图（不是 usize::MAX 个窗口）
        assert!(analyze(&path, 10.0, 0.0).unwrap().is_empty());
        assert!(analyze(&path, 10.0, f64::NAN).unwrap().is_empty());
        assert!(analyze(&path, 10.0, f64::INFINITY).unwrap().is_empty());
        // 合法值照旧
        assert_eq!(analyze(&path, 10.0, 100.0).unwrap().len(), 10);
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

    /// `travel_duration_for` 数的是 **UTF-8 字节**：5 个汉字 = 15 字节 → 10.4 秒。
    /// 按字符数算会得到 6.53 秒（滚快一倍），这条把口径钉住。
    #[test]
    fn travel_duration_is_byte_based() {
        assert!((travel_duration_for("还能续约吗") - 10.4).abs() < 1e-6, "15 字节");
        assert!((travel_duration_for("abc") - 7.84).abs() < 1e-6, "3 字节");
        // 同样的"5 个字"，全角与半角的字节数不同 —— 差的就是这里。
        assert!(travel_duration_for("还有五個字") > travel_duration_for("abcde"));
    }

    /// 录播姬 XML 的 8 秒窗口对中文弹幕**不够**：抬到 `start + 0.3 + travel`。
    #[test]
    fn clamp_display_windows_extends_short_xml_window() {
        let mut entries = vec![DanmakuEntry {
            start: 0.0,
            end: 8.0, // parse_bililive_xml 给滚动弹幕的固定窗口
            text: "还能续约吗".into(),
            color: "#ffffff".into(),
            y: 300.0,
            travel_duration: travel_duration_for("还能续约吗"), // 10.4s
        }];
        assert_eq!(clamp_display_windows(&mut entries), 1);
        assert!((entries[0].end - 10.7).abs() < 1e-6, "0.3 + 10.4");
        // 幂等：再跑一次不再改、也不再计数（process 每次都会调它）。
        assert_eq!(clamp_display_windows(&mut entries), 0);
    }

    /// 素材显式写了**更长**窗口的（滚到左边停一会儿再消失）只抬不压。
    #[test]
    fn clamp_display_windows_never_shortens() {
        let mut entries = vec![DanmakuEntry {
            start: 5.0,
            end: 40.0,
            text: "短".into(), // 3 字节 → travel 7.84s，窗口本来就够
            color: "#ffffff".into(),
            y: 300.0,
            travel_duration: travel_duration_for("短"),
        }];
        assert_eq!(clamp_display_windows(&mut entries), 0);
        assert!((entries[0].end - 40.0).abs() < 1e-6);
    }

    /// `travel_duration` 为 0（老/手写数据）按 8.0 兜底 —— 与渲染侧 `|| 8` 同口径。
    #[test]
    fn clamp_display_windows_zero_travel_falls_back_to_eight() {
        let mut entries = vec![DanmakuEntry {
            start: 10.0,
            end: 12.0,
            text: "x".into(),
            color: "#ffffff".into(),
            y: 300.0,
            travel_duration: 0.0,
        }];
        assert_eq!(clamp_display_windows(&mut entries), 1);
        assert!((entries[0].end - 18.3).abs() < 1e-6, "10 + 0.3 + 8");
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

    /// 主站 `comment.bilibili.com/{cid}.xml` 的真实样本。
    ///
    /// `p` 属性**逐字节取自** 2026-09-19 实测抓取的 `BV1TE411f7f1`（cid=122300483）：
    /// 9 段、`[7]` 是 17 位 dmid、`[8]` 恒为 `10`。
    /// 文本换成中性内容 —— 本固件要验的是**字段结构**，不是内容。
    const BILIBILI_MAIN_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<i><chatserver>chat.bilibili.com</chatserver><chatid>122300483</chatid><mission>0</mission><maxlimit>3000</maxlimit><state>0</state><real_name>0</real_name><source>k-v</source>
<d p="723.86400,5,25,16776960,1570578189,0,b89a0835,22772981386706944,10">顶端弹幕 &lt;带实体&gt;</d>
<d p="910.57100,1,25,16777215,1570552938,0,ea1655b6,22759742137958404,10">滚动弹幕一</d>
<d p="997.12000,1,25,16707842,1570644088,0,c4ad06c5,22807531222990850,10">带尾空格 </d>
<d p="1128.56300,4,25,15138834,1570604421,0,8a7f4628,22786734369013762,10">底端弹幕</d>
<d p="130.59400,5,25,15138834,1570580838,0,fe37d44d,22774369895841792,10"></d>
</i>"#;

    /// 录播姬写出的弹幕 XML：**8 段、`[7]` 恒为 `0`**。
    ///
    /// 字段顺序按 rec.danmuji.org/user/danmaku 的官方说明构造
    /// （第 7 段 = 发送者 UID 的 CRC32；第 8 段 = 固定 0，即主站那边的数据库 ID 位）。
    const BILILIVE_RECORDER_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<i>
  <chatserver>chat.bilibili.com</chatserver>
  <chatid>0</chatid>
  <maxlimit>0</maxlimit>
  <state>0</state>
  <source>k-v</source>
  <d p="1.500,1,25,16777215,1700000000,0,3d4a1b2c,0">滚动弹幕</d>
  <d p="12.250,4,25,16711935,1700000010,0,3d4a1b2c,0">底端弹幕</d>
  <d p="20.000,5,25,255,1700000020,0,3d4a1b2c,0">顶端弹幕</d>
  <d p="30.000,1,25,16711680,1700000030,0,3d4a1b2c,0"></d>
</i>"#;

    #[test]
    fn test_parse_main_site_xml_nine_fields() {
        let entries = parse_bililive_xml(BILIBILI_MAIN_XML);
        // 空文本那条被跳过
        assert_eq!(entries.len(), 4, "entries = {entries:?}");

        assert!((entries[0].start - 723.864).abs() < 1e-6);
        assert_eq!(entries[0].text, "顶端弹幕 <带实体>", "XML 实体必须反转义");
        assert_eq!(entries[0].color, "#ffff00");
        assert!((entries[0].end - 727.864).abs() < 1e-6, "固定弹幕显示 4 秒");

        // ⚠️ 回归钉子：`[7]` 是 17 位 dmid，**绝不能被当成 y**。
        // 旧实现把它 clamp 到 1500，导致所有条目 y 全相等（实测 3600/3600 条）。
        let ys: Vec<f64> = entries.iter().map(|e| e.y).collect();
        assert!(!ys.windows(2).all(|w| w[0] == w[1]), "y 不能全塌成同一个值：{ys:?}");
        assert!(ys.iter().all(|y| *y < 1500.0), "dmid 被当成 y 了：{ys:?}");

        assert_eq!(entries[2].text, "带尾空格", "文本要 trim");
        assert!((entries[0].y - 100.0).abs() < 1e-6, "mode=5 顶端默认 y");
        assert!((entries[3].y - 1000.0).abs() < 1e-6, "mode=4 底端默认 y");

        // 按时间排序
        assert!(entries.windows(2).all(|w| w[0].start <= w[1].start));
    }

    #[test]
    fn test_parse_recorder_xml_eight_fields_unchanged() {
        let entries = parse_bililive_xml(BILILIVE_RECORDER_XML);
        assert_eq!(entries.len(), 3, "entries = {entries:?}");
        // `[7]` 恒为 0 → 旧实现也被 `filter(> 0.0)` 挡掉、走 `default_y_for_mode`，
        // 所以修 y 取值对录播姬是**行为等价**的。这里钉住"行为不变"。
        assert!((entries[0].y - 400.0).abs() < 1e-6, "滚动默认 y");
        assert!((entries[1].y - 1000.0).abs() < 1e-6, "底端默认 y");
        assert!((entries[2].y - 100.0).abs() < 1e-6, "顶端默认 y");
        assert_eq!(entries[0].color, "#ffffff");
        assert_eq!(entries[1].color, "#ff00ff");
        assert_eq!(entries[2].color, "#0000ff");
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
        let dir = std::env::temp_dir().join(format!("yekitrim-danmaku-xml-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("danmaku.xml");
        std::fs::write(&path, BILILIVE_RECORDER_XML).unwrap();
        let entries = load_danmaku_auto(&path).unwrap();
        assert_eq!(entries.len(), 3, "按扩展名分派到 XML 解析");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// ⚠️ 回归钉子：把**弹幕 XML 喂给 `parse_srt_file` 必须报错**，不能静默 0 条。
    ///
    /// 这个坑真发生过：精修 `cmd_parse` 用 `parse_srt_file` 读 `danmaku_file`，
    /// 而录播姬/B站写的都是 `.xml` —— 实测 4 个真实样本（1200~3600 条）全被
    /// SRT 分支解析成 0 条且没有任何报错，表现只是"精修页没有弹幕"。
    #[test]
    fn test_parse_srt_file_refuses_xml_loudly() {
        let dir = std::env::temp_dir().join(format!("yekitrim-danmaku-guard-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();

        // 带 XML 声明的主站形态
        let main = dir.join("main.xml");
        std::fs::write(&main, BILIBILI_MAIN_XML).unwrap();
        let e = parse_srt_file(&main).unwrap_err();
        assert!(format!("{e}").contains("load_danmaku_auto"), "错误里要给出正确入口: {e}");

        // 没有声明、但含 `<d p=` 的录播姬形态也要挡住
        let rec = dir.join("rec.xml");
        std::fs::write(&rec, BILILIVE_RECORDER_XML).unwrap();
        assert!(parse_srt_file(&rec).is_err(), "含 <d p= 的也必须报错");

        // 真正的 SRT 不受影响
        let srt = dir.join("dm.srt");
        std::fs::write(&srt, "1\n00:00:01,000 --> 00:00:09,000\n{\\move(2000,0,0,0)}hi\n").unwrap();
        assert_eq!(parse_srt_file(&srt).unwrap().len(), 1);

        let _ = std::fs::remove_dir_all(&dir);
    }
}
