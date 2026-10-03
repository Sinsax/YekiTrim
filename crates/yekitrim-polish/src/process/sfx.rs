//! polish::process::sfx：音效规则引擎（字幕关键词 → 音效 tag）与解析 / 屏蔽。
//!
//! 入口：`match_sfx_rule`、`resolve_event_sfx`、`resolve_sfx_value`、`resolve_sfx_tag`、`load_sfx_blacklist`、`find_uncovered_sfx_moments`、`subtitle_context`。
//! 边界：规则未命中的事件按"宁缺毋滥"清空音效；屏蔽语义（精确 id 屏蔽单个、`tag:xxx` 屏蔽整个标签族）在 `assets/sfx_catalog.rs::is_blacklisted` —— 见 `crates/yekitrim-polish/src/process/AGENTS.md`

use crate::parse;
use crate::project;

/// 字幕关键词 → 音效 tag 引用 (规则引擎, 按顺序匹配, 首个命中生效)
/// 规则未命中的事件按"宁缺毋滥"策略删除/清空音效;
/// 命中的字幕时刻若 LLM 未放事件, 由 Step 3 自动补位创建 sfx 事件
const SFX_RULES: &[(&[&str], &str)] = &[
    (&["地震", "晃动", "震动", "震"], "tag:爆発"),
    (&["爆炸", "炸弹", "炸"], "tag:爆発"),
    (&["玻璃", "碎", "塌", "倒塌", "崩塌", "压塌"], "tag:破壊"),
    (&["摔", "撞", "砸", "坠", "掉落", "掉下去", "掉下来", "掉到", "掉进", "往下掉", "掉地"], "tag:打撃"),
    (&["红温", "急了", "生气", "气死", "怒了"], "tag:打撃"),
    (&["太爽", "爽到", "高手", "太强", "厉害"], "tag:強調"),
    (&["尖叫", "喊", "吓", "惊"], "tag:驚き"),
    (&["鬼", "噩梦", "恐怖", "阴森"], "tag:恐怖"),
    (&["笑", "哈哈", "呵呵", "嘿嘿", "搞笑", "整活", "调侃", "滑稽", "破防", "绷不住"], "tag:コメディ"),
    (&["敲门", "关门", "开门", "门"], "tag:ドア"),
    (&["电话", "手机", "来电", "消息", "私信", "发消息", "回消息", "回复"], "tag:電話"),
    (&["下雨", "雨", "打雷", "雷声", "闪电"], "tag:雨"),
    (&["洗澡", "洗脸", "洗头"], "tag:雨"),
    (&["风声", "刮风", "起风"], "tag:wind"),
    (&["魔法", "技能", "施法", "圣光"], "tag:魔法"),
    (&["电脑", "键盘", "打字"], "tag:機械"),
    (&["脚步", "走路", "走廊", "脚步声"], "tag:歩く"),
    (&["跑", "追"], "tag:走る"),
    (&["钱", "付费", "打钱"], "tag:金銭"),
    (&["可爱", "卖萌"], "tag:可愛い"),
    (&["刀", "剑", "武器", "砍"], "tag:武器"),
    (&["鸟叫", "狗叫", "猫叫", "动物"], "tag:動物"),
    (&["唱歌", "音乐", "当当"], "tag:演出"),
];

/// 匹配字幕文本中的音效关键词, 返回规则指定的 tag 引用 (如 "tag:爆発")
pub fn match_sfx_rule(text: &str) -> Option<&'static str> {
    for (keywords, tag) in SFX_RULES {
        if keywords.iter().any(|k| text.contains(k)) {
            return Some(tag);
        }
    }
    None
}

/// 事件是否已带音效引用 (LLM 写的 tag:xxx 或 catalog id, 尚未解析落地)
/// 提取事件中的音效引用 (Sfx.name 或特效/贴纸的 sfx 字段), 用于跨段未使用集合
pub fn event_sfx_ref(event: &crate::project::Event) -> Option<&str> {
    use crate::project::Event;
    match event {
        Event::Sfx { name, .. } if !name.is_empty() => Some(name.as_str()),
        Event::Sticker { sfx, .. }
        | Event::Shake { sfx, .. }
        | Event::Flash { sfx, .. }
        | Event::Blur { sfx, .. }
        | Event::Vignette { sfx, .. }
        | Event::HueShift { sfx, .. }
        | Event::Noise { sfx, .. }
        | Event::Pulse { sfx, .. }
        | Event::Overlay { sfx, .. } => sfx.as_deref(),
        _ => None,
    }
}

pub fn event_has_sfx_ref(event: &crate::project::Event) -> bool {
    match event {
        crate::project::Event::Sfx { name, .. } => !name.is_empty(),
        crate::project::Event::Sticker { sfx, .. }
        | crate::project::Event::Shake { sfx, .. }
        | crate::project::Event::Flash { sfx, .. }
        | crate::project::Event::Blur { sfx, .. }
        | crate::project::Event::Vignette { sfx, .. }
        | crate::project::Event::HueShift { sfx, .. }
        | crate::project::Event::Noise { sfx, .. }
        | crate::project::Event::Pulse { sfx, .. }
        | crate::project::Event::Overlay { sfx, .. } => sfx.is_some(),
        _ => false,
    }
}

/// 事件是否携带声音 (独立 sfx 事件有 name, 或特效/贴纸有 sfx 引用)
pub fn event_has_sound(event: &crate::project::Event) -> bool {
    match event {
        crate::project::Event::Sfx { name, .. } => !name.is_empty(),
        crate::project::Event::Sticker { sfx, .. }
        | crate::project::Event::Shake { sfx, .. }
        | crate::project::Event::Flash { sfx, .. }
        | crate::project::Event::Blur { sfx, .. }
        | crate::project::Event::Vignette { sfx, .. }
        | crate::project::Event::HueShift { sfx, .. }
        | crate::project::Event::Noise { sfx, .. }
        | crate::project::Event::Pulse { sfx, .. }
        | crate::project::Event::Overlay { sfx, .. } => sfx.is_some(),
        _ => false,
    }
}

/// 自动补位: 扫描字幕找规则命中但 LLM 未放声音事件的时刻, 返回 (时间, tag 引用) 列表
/// - 时间 = 字幕起点 + 0.2s
/// - 去重: 与已有"带声音事件"或已选中的补位点时间差 < 2s 的跳过 (连续字幕只取第一处)
/// - 按时间升序, 最多 budget 个
pub fn find_uncovered_sfx_moments(
    subtitles: &[parse::srt::Subtitle],
    events: &[crate::project::Event],
    budget: usize,
) -> Vec<(f64, &'static str)> {
    if budget == 0 {
        return vec![];
    }
    // 只有带声音的事件才算占用 (camera 等视觉事件不挡音效补位)
    let sound_times: Vec<f64> = events.iter()
        .filter(|e| event_has_sound(e))
        .map(|e| e.get_time())
        .collect();
    let occupied = |t: f64, planned: &[(f64, &'static str)]| -> bool {
        sound_times.iter().any(|st| (st - t).abs() < 2.0)
            || planned.iter().any(|(pt, _)| (pt - t).abs() < 2.0)
    };

    let mut out: Vec<(f64, &'static str)> = vec![];
    for sub in subtitles {
        if out.len() >= budget {
            break;
        }
        let Some(tag) = match_sfx_rule(&sub.text) else { continue };
        let t = sub.start + 0.2;
        if occupied(t, &out) {
            continue;
        }
        out.push((t, tag));
    }
    out
}

/// 取事件时间 ±3s 内的字幕文本, 按距离升序 (优先包含事件时间的字幕)
/// 规则匹配时依次尝试, 首个命中关键词生效 — 事件常提前于动作发生, 窗口可捕获
pub fn subtitle_context<'a>(subtitles: &'a [parse::srt::Subtitle], t: f64) -> Vec<&'a str> {
    let mut items: Vec<(f64, &'a str)> = vec![];
    for s in subtitles {
        let dist = if t >= s.start && t < s.end {
            0.0
        } else if t < s.start {
            s.start - t
        } else {
            t - s.end
        };
        if dist <= 3.0 {
            items.push((dist, s.text.as_str()));
        }
    }
    items.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
    items.into_iter().map(|(_, text)| text).collect()
}

/// 设置/清空特效事件 (含 sticker/overlay) 的 sfx 字段
pub(super) fn set_sfx_field(event: &mut crate::project::Event, value: Option<String>) {
    use crate::project::Event;
    match event {
        Event::Sticker { sfx, .. }
        | Event::Shake { sfx, .. }
        | Event::Flash { sfx, .. }
        | Event::Blur { sfx, .. }
        | Event::Vignette { sfx, .. }
        | Event::HueShift { sfx, .. }
        | Event::Noise { sfx, .. }
        | Event::Pulse { sfx, .. }
        | Event::Overlay { sfx, .. } => *sfx = value,
        _ => {}
    }
}

/// 解析事件中的 sfx 引用: catalog id / tag:xxx / 裸 tag / 模糊匹配 → 变体文件名
/// 无法解析时移除并警告 (避免渲染时静默缺失)
/// blacklist 中已存在的 id 会被重新解析替换 (老项目自动迁移)
/// used: 本片已用变体集合 (未使用优先, 避免同一变体重复)
pub fn resolve_event_sfx(event: &mut crate::project::Event, blacklist: &[String], used: &mut std::collections::HashSet<String>) {
    use crate::project::Event;
    let sfx_opt: Option<&mut Option<String>> = match event {
        Event::Sticker { sfx, .. }
        | Event::Shake { sfx, .. }
        | Event::Flash { sfx, .. }
        | Event::Blur { sfx, .. }
        | Event::Vignette { sfx, .. }
        | Event::HueShift { sfx, .. }
        | Event::Noise { sfx, .. }
        | Event::Pulse { sfx, .. }
        | Event::Overlay { sfx, .. } => Some(sfx),
        _ => None,
    };
    if let Some(sfx) = sfx_opt {
        let resolved = sfx.as_deref().and_then(|v| resolve_sfx_value(v, blacklist, used));
        match resolved {
            Some(id) => *sfx = Some(id),
            None => {
                if let Some(v) = sfx.as_deref() {
                    tracing::info!("  ⚠ sfx 「{}」: 无法匹配到音效，已移除", v);
                }
                *sfx = None;
            }
        }
    }
    if let Event::Sfx { name, .. } = event {
        if !name.is_empty() {
            match resolve_sfx_value(name, blacklist, used) {
                Some(resolved) => *name = resolved,
                None => {
                    tracing::info!("  ⚠ sfx 「{}」: 无法匹配到音效，已移除", name);
                    name.clear();
                }
            }
        }
    }
}

/// 三级解析: ① catalog id 直查 ② tag (tag: 前缀或裸 tag) ③ 全 catalog 模糊匹配
/// 被屏蔽的条目在各级都被跳过
/// 从条目的变体列表中选一个 (未使用优先轮换)，返回变体文件名 stem
/// 变体耗尽时返回 None (宁缺毋滥, 避免同一音效整片无限重复)
fn pick_variant(entry: &crate::assets::sfx_catalog::SfxEntry, used: &mut std::collections::HashSet<String>) -> Option<String> {
    for v in &entry.variants {
        let stem = v.strip_suffix(".mp3").unwrap_or(v);
        if !used.contains(stem) {
            used.insert(stem.to_string());
            return Some(stem.to_string());
        }
    }
    None
}

/// 解析 sfx 引用: catalog id / tag:xxx / 裸 tag / 模糊匹配 → 变体文件名
/// 同一组被多次引用时按变体未使用优先轮换, 避免听觉重复
pub fn resolve_sfx_value(value: &str, blacklist: &[String], used: &mut std::collections::HashSet<String>) -> Option<String> {
    if value.is_empty() {
        return None;
    }
    // ① catalog id 直查 (被屏蔽则继续解析)
    if let Some(entry) = crate::assets::sfx_catalog::lookup(value)
        && !crate::assets::sfx_catalog::is_blacklisted(&entry.id, blacklist)
    {
        return pick_variant(entry, used);
    }
    // ② 变体名直查 (已解析过的 name 重新 resolve 时保持; 被屏蔽则继续解析走兜底)
    if let Some(entry) = crate::assets::sfx_catalog::lookup_variant(value) {
        if !crate::assets::sfx_catalog::is_blacklisted(&entry.id, blacklist) {
            return Some(value.to_string());
        }
    }
    // ③ 中文 label 直查 (LLM 只输入中文名; 重复 label 取第一个)
    if let Some(entry) = crate::assets::sfx_catalog::lookup_label(value)
        && !crate::assets::sfx_catalog::is_blacklisted(&entry.id, blacklist)
    {
        return pick_variant(entry, used);
    }
    if let Some(tag_name) = value.strip_prefix("tag:") {
        // tag: 引用失败直接返回 None, 不降级到模糊匹配 (避免隐藏日文标签 fuzzy 出无关音效)
        return resolve_sfx_tag(tag_name, blacklist, used);
    }
    if let Some(id) = resolve_sfx_tag(value, blacklist, used) {
        return Some(id);
    }
    if let Some(entry) = crate::assets::sfx_catalog::fuzzy_search_excluding(value, blacklist) {
        tracing::info!("  ✓ sfx 模糊匹配「{}」→ {}", value, entry.id);
        return pick_variant(entry, used);
    }
    None
}

/// 按 tag 查找候选条目并选变体 (未使用优先轮换)
pub fn resolve_sfx_tag(tag: &str, blacklist: &[String], used: &mut std::collections::HashSet<String>) -> Option<String> {
    if tag.is_empty() {
        return None;
    }
    let candidates = crate::assets::sfx_catalog::search_by_tags(&[tag.to_string()]);
    // 遍历候选组: 变体耗尽的组跳过, 换下一组 (避免同一音效反复出现)
    for entry in candidates {
        if crate::assets::sfx_catalog::is_blacklisted(&entry.id, blacklist) {
            continue;
        }
        if let Some(variant) = pick_variant(entry, used) {
            tracing::info!("  ✓ sfx tag「{}」→ {} (变体 {})", tag, entry.id, variant);
            return Some(variant);
        }
    }
    None
}

// ============================================================
// download
// ============================================================

/// 加载音效屏蔽列表: 全局 (.yekitrim/config.toml [sfx] blacklist) ∪ 项目 (polish.toml [sfx] blacklist)
pub fn load_sfx_blacklist(config: &project::ProjectFile) -> Vec<String> {
    let mut list = yekitrim_core::config::load_global_config().sfx.blacklist.clone();
    for id in &config.sfx.blacklist {
        if !list.contains(id) {
            list.push(id.clone());
        }
    }
    list
}
