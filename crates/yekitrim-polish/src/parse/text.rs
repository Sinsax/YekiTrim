/// ASR 纠错 — 基于规则的文本修正
pub fn correct_text(text: &str, corrections: &[(String, String)]) -> String {
    let mut result = text.to_string();
    for (pattern, replacement) in corrections {
        result = result.replace(pattern, replacement);
    }
    result
}

/// 关键词高亮 — 给匹配词加 HTML 标签
pub fn apply_highlights(text: &str, highlight_words: &[String]) -> String {
    // 长词优先
    let mut sorted: Vec<&String> = highlight_words.iter().collect();
    sorted.sort_by_key(|b| std::cmp::Reverse(b.len()));

    let mut result = text.to_string();
    let mut count = 0;
    let max_highlights = 2;

    for word in sorted {
        if count >= max_highlights {
            break;
        }
        if let Some(pos) = result.find(word) {
            let before = &result[..pos];
            let after = &result[pos + word.len()..];
            result = format!(
                "{}<span class=\"hl\">{}</span>{}",
                before, word, after
            );
            count += 1;
        }
    }

    result
}

/// 检测 Hook — 找情绪最强烈的字幕段
pub fn detect_hook(subtitles: &[crate::parse::srt::Subtitle], intensity_words: &[&str]) -> Option<(usize, f64)> {
    let mut best_idx = None;
    let mut best_score = -1.0f64;

    for (i, sub) in subtitles.iter().enumerate() {
        let mut score = 0.0;
        for word in intensity_words {
            if sub.text.contains(word) {
                score += 10.0;
            }
        }
        // 短句加分
        if sub.text.len() < 10 {
            score += 3.0;
        }
        if score > best_score {
            best_score = score;
            best_idx = Some(i);
        }
    }

    best_idx.map(|idx| (idx, best_score))
}

/// 弹幕轨道分配 (碰撞避免)
pub fn assign_danmaku_tracks(
    entries: &mut [yekitrim_core::danmaku::DanmakuEntry],
) {
    const TRACK_YS: [f64; 7] = [300.0, 500.0, 700.0, 900.0, 1100.0, 1300.0, 1500.0];
    let mut track_release: [f64; 7] = [0.0; 7];

    // 按时间排序
    entries.sort_by(|a, b| a.start.partial_cmp(&b.start).unwrap());

    for entry in entries.iter_mut() {
        // 滚动时长只有一份公式（`yekitrim_core::danmaku::travel_duration_for`）：
        // `(1080 + UTF-8 字节数 × 32) / 150` 秒。`tools/polish-to-dhampir.mjs` 抄的是同一份。
        let travel = yekitrim_core::danmaku::travel_duration_for(&entry.text);
        entry.travel_duration = travel;

        // 找最早可用的轨道
        let mut best_track = 0;
        let mut best_release = f64::MAX;
        for (t, &release) in track_release.iter().enumerate() {
            if release <= entry.start {
                best_track = t;
                break;
            }
            if release < best_release {
                best_release = release;
                best_track = t;
            }
        }

        // 占轨道用的就是 `start + travel` —— 分配器本来就认为一条弹幕在屏 `travel` 秒
        track_release[best_track] = entry.start + travel;
        entry.y = TRACK_YS[best_track];
    }

    // ⚠️ 分配完轨道之后，把**在屏窗口的末尾**抬到"至少够它滚完"（`end` 这个字段先前没跟上
    // `start + travel` 的口径）。不抬的话渲染侧会在弹幕**还没走到左边缘**时渐隐：
    // 渲染的渐隐钉在窗口末尾（`d.end - 0.2`），而滚动由 `travel` 决定，两口钟互不相干。
    // 录播姬 XML 给的窗口是固定 8 秒，而中文弹幕的 travel 普遍 8.5~16 秒 —— 实测
    // 「还能续约吗」在距左边缘 337px 处开始渐隐、13 个汉字那条在屏幕中段 728px 处就没了。
    // 明细与判据见 `yekitrim_core::danmaku::clamp_display_windows`。
    let extended = yekitrim_core::danmaku::clamp_display_windows(entries);
    if extended > 0 {
        tracing::info!("  ✓ 弹幕在屏窗口已抬到「滚完为止」{} 条", extended);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use yekitrim_core::danmaku::DanmakuEntry;

    fn entry(start: f64, end: f64, text: &str) -> DanmakuEntry {
        DanmakuEntry {
            start,
            end,
            text: text.into(),
            color: "#ffffff".into(),
            y: 0.0,
            travel_duration: 0.0,
        }
    }

    /// 分轨之后**窗口必须够它滚完**：录播姬 XML 的 8 秒窗口对中文弹幕不够，
    /// 不抬的话渲染侧会在弹幕还没到左边缘时渐隐（`d.end - 0.2` 起渐隐）。
    #[test]
    fn assign_danmaku_tracks_extends_window_to_arrival() {
        let mut entries = vec![entry(0.0, 8.0, "还能续约吗"), entry(30.0, 60.0, "困在雨季")];
        assign_danmaku_tracks(&mut entries);
        // 15 字节 → travel 10.4s → 窗口末尾 0.3 + 10.4 = 10.7s
        assert!((entries[0].travel_duration - 10.4).abs() < 1e-6);
        assert!((entries[0].end - 10.7).abs() < 1e-6, "实得 {}", entries[0].end);
        // 第二条 30 秒的窗口本来就够（travel 9.76s）—— **只抬不压**，原样留着
        assert!((entries[1].travel_duration - 9.76).abs() < 1e-6);
        assert!((entries[1].end - 60.0).abs() < 1e-6, "实得 {}", entries[1].end);
    }

    /// 分轨仍然是"上一条离开之后才复用"：`start + travel` 那条也没被这次改动碰掉。
    #[test]
    fn assign_danmaku_tracks_still_avoids_overlap() {
        let mut entries = vec![entry(0.0, 8.0, "第一条"), entry(0.5, 8.5, "第二条")];
        assign_danmaku_tracks(&mut entries);
        assert_ne!(entries[0].y, entries[1].y, "同刻的两条不许落在同一条泳道");
    }
}
