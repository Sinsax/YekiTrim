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
    entries: &mut [vtrim_core::danmaku::DanmakuEntry],
) {
    const TRACK_YS: [f64; 7] = [300.0, 500.0, 700.0, 900.0, 1100.0, 1300.0, 1500.0];
    let mut track_release: [f64; 7] = [0.0; 7];

    // 按时间排序
    entries.sort_by(|a, b| a.start.partial_cmp(&b.start).unwrap());

    for entry in entries.iter_mut() {
        let travel = (1080.0 + entry.text.len() as f64 * 32.0) / 150.0;
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

        track_release[best_track] = entry.start + travel;
        entry.y = TRACK_YS[best_track];
    }
}
