//! `api::polish::cover`：封面（标题 / 配置 / 背景 / 帧）与「用哪个版本的成片」。

use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;

use crate::model::AppState;

use super::*;



pub(super) fn cover_title_event_context(
    cfg: &yekitrim_polish::project::ProjectFile,
    subtitles: &[yekitrim_core::types::TranscriptSegment],
) -> String {
    use std::fmt::Write as _;

    let mut out = String::new();
    let muted = |start: f64, end: f64| cfg.mute_ranges.iter().any(|r| r.overlaps(start, end));

    if cfg.segments.is_empty() {
        let _ = writeln!(out, "polish.toml 暂无分段，按整片 clip.srt 字幕：");
        for s in subtitles.iter().filter(|s| !muted(s.start, s.end)) {
            let _ = writeln!(out, "[{:.2}-{:.2}] {}", s.start, s.end, s.text);
        }
        return out;
    }

    for (i, seg) in cfg.segments.iter().enumerate() {
        if muted(seg.start, seg.end) {
            continue;
        }
        let _ = write!(out, "[事件/分段 {}] {:.2}s-{:.2}s", i + 1, seg.start, seg.end);
        if !seg.note.trim().is_empty() {
            let _ = write!(out, " 备注: {}", seg.note.trim());
        }
        if !seg.fx.trim().is_empty() {
            let _ = write!(out, " 情绪: {}", seg.fx.trim());
        }
        let _ = writeln!(out);
        if !seg.events.is_empty() {
            let evs: Vec<String> = seg
                .events
                .iter()
                .map(|e| serde_json::to_string(e).unwrap_or_else(|_| "{}".into()))
                .collect();
            let _ = writeln!(out, "  特效事件: {}", evs.join(" | "));
        }
        let _ = writeln!(out, "  有效字幕:");
        let mut has = false;
        for sub in subtitles
            .iter()
            .filter(|s| s.end >= seg.start - 0.05 && s.start <= seg.end + 0.05)
            .filter(|s| !muted(s.start, s.end))
        {
            has = true;
            let _ = writeln!(out, "    [{:.2}-{:.2}] {}", sub.start, sub.end, sub.text);
        }
        if !has {
            let _ = writeln!(out, "    （无）");
        }
        if out.chars().count() > 8000 {
            let _ = writeln!(out, "（上下文过长，已截断）");
            break;
        }
    }
    out
}


pub(super) fn sanitize_cover_title_line(line: &str) -> String {
    let mut s = line.trim().trim_matches(|c: char| {
        matches!(c, '"' | '“' | '”' | '`' | '【' | '】' | '[' | ']')
    });
    s = s.trim();
    s = s.trim_start_matches(['▶', '▸', '•', '·']).trim();
    for prefix in ["- ", "* ", "• ", "· "] {
        if let Some(rest) = s.strip_prefix(prefix) {
            s = rest.trim();
            break;
        }
    }
    if let Some(pos) = s.find(['.', '、', ')', '）']) {
        let head = s[..pos].trim();
        if !head.is_empty() && head.chars().all(|c| c.is_ascii_digit()) {
            let step = s[pos..].chars().next().map(|c| c.len_utf8()).unwrap_or(0);
            s = s[pos + step..].trim();
        }
    }
    s.trim().to_string()
}


pub(super) fn looks_like_cover_title_context(line: &str) -> bool {
    line.starts_with("[事件/分段")
        || line.starts_with("事件/分段")
        || line.starts_with("备注:")
        || line.starts_with("情绪:")
        || line.starts_with("特效事件:")
        || line.starts_with("有效字幕")
        || line.contains("title_lines")
        || (line.starts_with('{') && line.ends_with('}'))
        || (line.starts_with('[') && line.ends_with(']'))
}


pub(super) fn push_cover_title_text(out: &mut Vec<String>, text: &str) {
    for part in text.lines() {
        let line = sanitize_cover_title_line(part);
        if !line.is_empty() && !looks_like_cover_title_context(&line) {
            out.push(line);
        }
    }
}


pub(super) fn title_lines_from_json_value(value: &serde_json::Value) -> Option<Vec<String>> {
    if let Some(arr) = value.get("title_lines").and_then(|x| x.as_array()) {
        let mut out = Vec::new();
        for item in arr {
            if let Some(text) = item.as_str() {
                push_cover_title_text(&mut out, text);
            }
        }
        if !out.is_empty() {
            return Some(out);
        }
    }
    if let Some(text) = value.get("title_lines").and_then(|x| x.as_str()) {
        let mut out = Vec::new();
        push_cover_title_text(&mut out, text);
        if !out.is_empty() {
            return Some(out);
        }
    }
    if let Some(arr) = value.as_array() {
        let mut out = Vec::new();
        for item in arr {
            if let Some(text) = item.as_str() {
                push_cover_title_text(&mut out, text);
            } else if let Some(inner) = title_lines_from_json_value(item) {
                return Some(inner);
            }
        }
        if !out.is_empty() {
            return Some(out);
        }
    }
    if let Some(obj) = value.as_object() {
        for child in obj.values() {
            if let Some(inner) = title_lines_from_json_value(child) {
                return Some(inner);
            }
        }
    }
    None
}


pub(super) fn find_matching_json_end(text: &str, start: usize) -> Option<usize> {
    let open = *text.as_bytes().get(start)?;
    let close = if open == b'{' { b'}' } else { b']' };
    let mut depth = 0i32;
    let mut in_string = false;
    let mut escaped = false;
    for (i, b) in text.bytes().enumerate().skip(start) {
        if in_string {
            if escaped {
                escaped = false;
            } else if b == 92 {
                escaped = true;
            } else if b == b'"' {
                in_string = false;
            }
            continue;
        }
        if b == b'"' {
            in_string = true;
        } else if b == open {
            depth += 1;
        } else if b == close {
            depth -= 1;
            if depth == 0 {
                return Some(i);
            }
        }
    }
    None
}


pub(super) fn parse_cover_title_lines(content: &str) -> Vec<String> {
    let text = content.trim();
    let mut candidates: Vec<String> = vec![text.to_string()];
    if let Some(pos) = text.find("```") {
        let body = &text[pos + 3..];
        let end = body.find("```").unwrap_or(body.len());
        let fenced = body[..end].trim().trim_start_matches("json").trim();
        if !fenced.is_empty() {
            candidates.push(fenced.to_string());
        }
    }
    for open in ['{', '['] {
        if let Some(start) = text.find(open) {
            if let Some(end) = find_matching_json_end(text, start) {
                candidates.push(text[start..=end].to_string());
            }
        }
    }
    for cand in &candidates {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(cand) {
            if let Some(lines) = title_lines_from_json_value(&v) {
                if !lines.is_empty() {
                    return lines;
                }
            }
        }
    }
    let mut out = Vec::new();
    for line in text.lines() {
        let line = sanitize_cover_title_line(line);
        if !line.is_empty() && !looks_like_cover_title_context(&line) {
            out.push(line);
        }
    }
    out
}


pub(super) fn is_emoji_char(c: char) -> bool {
    let u = c as u32;
    (0x1F300..=0x1FAFF).contains(&u)
        || (0x1F000..=0x1F2FF).contains(&u)
        || (0x2600..=0x27BF).contains(&u)
        || (0x2190..=0x21FF).contains(&u)
        || (0x2B00..=0x2BFF).contains(&u)
}


pub(super) fn first_emoji(s: &str) -> Option<char> {
    s.chars().find(|c| is_emoji_char(*c))
}


pub(super) fn count_emoji_in_lines(lines: &[String]) -> usize {
    lines
        .iter()
        .map(|l| l.chars().filter(|c| is_emoji_char(*c)).count())
        .sum()
}


pub(super) fn title_char_count(lines: &[String]) -> usize {
    lines
        .iter()
        .flat_map(|l| l.chars())
        .filter(|c| !c.is_whitespace())
        .count()
}


pub(super) fn trim_title_lines(lines: &mut Vec<String>, max_chars: usize) {
    if title_char_count(lines) <= max_chars {
        return;
    }
    let mut budget = max_chars;
    let mut out: Vec<String> = Vec::new();
    for line in lines.iter() {
        if budget == 0 {
            break;
        }
        let mut s = String::new();
        for ch in line.chars() {
            if ch.is_whitespace() {
                s.push(ch);
                continue;
            }
            if budget == 0 {
                break;
            }
            s.push(ch);
            budget -= 1;
        }
        let s = s.trim_end().to_string();
        if !s.is_empty() {
            out.push(s);
        }
    }
    *lines = out;
}


pub(super) fn keep_single_emoji(lines: &mut Vec<String>) {
    let mut seen = false;
    for line in lines.iter_mut() {
        let mut out = String::new();
        for ch in line.chars() {
            if is_emoji_char(ch) {
                if seen {
                    continue;
                }
                seen = true;
            }
            out.push(ch);
        }
        *line = out;
    }
}


/// 封面标题的 system prompt（**风格优先**：参考标题是「口吻基准」，不是资料）。
///
/// 2026-10-19 用户报「没有那种幽默串子、让人想点进去的欲望」：旧 prompt 只写
/// 「参考标题用于提取信息点」+ 一列格式约束，模型于是写成四平八稳的概述。
/// 现在把「必须模仿参考标题的口吻」写成硬要求，并明确禁止概述腔。
pub(super) fn cover_title_system_prompt() -> String {
    format!(r#"你是直播切片封面标题助手。为当前切片生成封面大字标题，必须沿用事件检测阶段的标题规则：
{}
封面补充要求（**风格优先于信息完整**）：
- **风格基准 = 下面给出的「参考标题」**：必须模仿它们的口气与抖梗方式（口语、有钩子、有反差/反转、像评论区/弹幕串子），越像越对；只学语气，不要照抄原句
- **硬性禁止平铺直叙**：「XX 在直播里聊了…」「XX 的精彩瞬间」「一次关于…的讨论」这类概述一律不合格，也不能是四平八稳的总结
- **第一行必须是钩子**：悬念 / 反差 / 离谱 / 破防 / 嘴硬被打脸 任选其一；第 2~3 行可接反转或吐槽
- **优先引用片段里最出圈的那句原话或梗**（那种能单独截图传播的句子），不要概括它
- 输出 1~3 行，适合封面大字排版；整组标题总字数严格 8~40 字
- 只输出封面标题本身，不要输出 ▶ / 事件 / 分段 / 备注 / 上下文原文
- 必须且只能包含 1 个 emoji，放在句尾；参考标题有多个 emoji 也不要照搬
- 不要引号、不要解释、不要 markdown
- 只能依据输入的 polish.toml 逐个事件/分段与其对应的 clip.srt 字幕，不要引用输入之外的信息
- 输出 JSON：{{"title_lines": ["第一行", "第二行"]}}
"#, yekitrim_core::detection::TITLE_RULES)
}


/// 封面标题的 user prompt：参考标题在前（**它是风格基准**）、素材上下文在后。
pub(super) fn cover_title_user_prompt(reference_text: &str, context: &str) -> String {
    let mut out = String::new();
    if !reference_text.is_empty() {
        out.push_str("参考标题（这几条就是用户认可的口吻 —— **风格必须像它们**：口语、有梗、让人想点进去；只学语气，不要抄原句）：\n");
        out.push_str(reference_text);
        out.push_str("\n\n");
    }
    out.push_str("以下是 polish.toml 中的逐个事件/分段，以及 clip.srt 中对应字幕（优先找那句最出圈的原话）：\n");
    out.push_str(context);
    out.push_str("\n请按上述要求写封面标题：第一行是钩子，风格像参考标题，不要写概述。");
    out
}

/// 封面标题取样温度的解析（**唯一一处**：设计器「温度」= `[cover] title_temperature`）。
///
/// - 显式设过 ⇒ **就是它**（夹 0~2）：想完全稳定就该拿到 0，不许再被下限顶回去；
/// - 没设 ⇒ 链路温度（`[llm.params] temperature` / `[llm.overrides.utility]`）**抬到下限** 0.8。
///   v2 里"省略"= 不表态（不发送）⇒ 调用方按 0 传进来，于是照样落到下限 —— 与 v1 的模板默认 0 同效。

/// 封面文案（标题生成）走哪条链路 —— **一处定义**，测试与调用都看它。
///
/// 2026-10-19 用户口径：「链路覆盖里字幕订正删除，增加一个标题生成」。封面文案原来挂在
/// `Utility`（零散单发）上，与贴纸包/扫描标签共用一组覆盖；拆成 `Title` 之后它有自己的
/// `[llm.overrides.title]`（默认仍然关思考，所以行为不变）。
///
/// ⚠️ 贴纸包标签 / emoji 分类 / 贴纸扫描标签**仍属 `Utility`**（它们才是"零散单发"）。
fn cover_title_link() -> yekitrim_core::config::LlmLink {
    yekitrim_core::config::LlmLink::Title
}


pub(super) fn cover_title_temperature(cover: Option<f64>, link: f64) -> f64 {
    const COVER_TITLE_MIN_TEMPERATURE: f64 = 0.8;
    match cover {
        Some(t) => t.clamp(0.0, 2.0),
        None => link.max(COVER_TITLE_MIN_TEMPERATURE),
    }
}


pub(crate) async fn polish_cover_title(
    State(_state): State<Arc<AppState>>,
    Extension(user): Extension<crate::auth::CurrentUser>,
    Json(body): Json<serde_json::Value>,
) -> ApiResult {
    let dir = body.get("dir").and_then(|v| v.as_str()).ok_or_else(|| err(StatusCode::BAD_REQUEST, "缺少 dir"))?;
    let path = validated_project_dir(dir)?;
    if !{
        let __p = path.join("polish.toml");
        // 同步 stat 不能留在 async worker 上（api/mod.rs:44-51 ✓）：
        // 工程常在慢盘/网络盘上，一次 exists() 就要几百 ms ✗
        tokio::task::spawn_blocking(move || __p.exists())
            .await
            .unwrap_or(false)
    } {
        return Err(err(StatusCode::BAD_REQUEST, "目录未初始化 polish"));
    }

    // 只使用 clip.srt + polish.toml 作为输入；
    // project.name 仅作为“参考标题”参与生成，不读取 data/subtitles.json / clip-meta.json / clip.json。
    let cfg = yekitrim_polish::project::load_project(&path)
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("加载 polish.toml 失败: {e}")))?;
    let srt_path = path.join("clip.srt");
    if !srt_path.exists() {
        return Err(err(StatusCode::BAD_REQUEST, "clip.srt 不存在"));
    }
    let raw = std::fs::read_to_string(&srt_path)
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("读取 clip.srt 失败: {e}")))?;
    let transcript = yekitrim_core::srt::parse_srt(&raw)
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("解析 clip.srt 失败: {e}")))?;
    if transcript.segments.is_empty() {
        return Err(err(StatusCode::BAD_REQUEST, "clip.srt 暂无字幕，无法生成标题"));
    }
    let context = cover_title_event_context(&cfg, &transcript.segments);
    // ⚠️ **参考标题必须按 owner 读**（2026-10-19 用户问「LLM 真的用了设置里的参考标题吗」）：
    // 设置页写的是 `users/<uid>/.yekitrim/config.toml`，而无 owner 的 `load_global_config()`
    // 读顶层旧文件 —— 和封面那条 llm.toml 是同一个坑（那里也是 100% 复现）。
    let cfg_owner = crate::auth::owner_of(&user);
    let global_cfg = blocking(move || Ok(yekitrim_core::config::load_config_for(cfg_owner))).await?;
    let reference_titles = yekitrim_core::config::resolve_reference_titles(&global_cfg.detection);
    let reference_text = if !reference_titles.is_empty() {
        reference_titles.join("\n")
    } else {
        let raw = cfg.project.name.trim();
        let cleaned = raw.split('【').next().unwrap_or(raw).trim();
        if !cleaned.is_empty() {
            cleaned.to_string()
        } else {
            cfg.cover.title_lines.join(" / ")
        }
    };

    // ⚠️ **必须按账号读**（2026-10-19 用户报「用的和设置里的 API 服务不一致」）：
    // `load_llm_config()` 读的是**顶层** `<数据>/.yekitrim/llm.toml`（单人时代的旧位置），
    // 而设置页（`GET/PUT /api/config/llm`）与精修链路读写的都是
    // `users/<uid>/.yekitrim/llm.toml` ⇒ 用错那一支就会拿一份**早就没人写**的旧配置
    // （日志里 base_url/model 与设置页对不上，且 100% 复现）。
    // 读配置是同步 IO ⇒ 走 blocking 池（`api/AGENTS.md` 的同步 IO 纪律）。
    let owner = crate::auth::owner_of(&user);
    let llm = blocking(move || Ok(yekitrim_core::config::load_llm_config_for(owner))).await?;
    // ⚠️ 判定走 provider-aware 的入口：多配置下顶层 `api_key` **恒空** ——
    // 读顶层会让封面文案**恒报"LLM 未配置"**（用户明明在 provider 里填了密钥）。
    // 这里**本来就是 400**，保持报错；只是文案要指到用户能点的那个位置。
    if llm.effective_base_url().trim().is_empty() {
        return Err(err(StatusCode::BAD_REQUEST, "LLM 未配置 API 地址：请在 设置 → LLM 配置 里填好「API 地址」"));
    }
    if !llm.has_api_key() {
        return Err(err(
            StatusCode::BAD_REQUEST,
            "LLM 未配置 API 密钥：请在 设置 → LLM 配置 里为当前配置填好「API 密钥」",
        ));
    }
    // 链路解析：**标题生成**（见 [llm.overrides.title]）；thinking 的默认关思考由那条链路的
    // `default_thinking` 给出（与它原来挂在零散单发上时一致 —— 行为不变）。
    let linked = llm.for_link(cover_title_link());
    // ⚠️ **温度下限**（2026-10-19 用户报「没有幽默串子那种想点进去的欲望」）：
    // `[llm] temperature` 的模板默认是 **0**（确定性）—— 拿它写标题只会得到四平八稳的概述。
    // 只在**低于**下限时抬上来；用户自己配了更高温度就照用（不覆盖用户意图）。
    // v2：`[llm.params] temperature` 省略 = 不表态 ⇒ 按 0 交给下面那条下限规则（0.8）
    let temp = cover_title_temperature(cfg.cover.title_temperature, linked.params.temperature.unwrap_or(0.0));
    let client = yekitrim_llm::LlmClient::new(&linked)
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("LLM 配置错误: {e}")))?
        .with_temperature(temp)
        .with_timeout(60)
        .with_usage_context(
            yekitrim_llm::usage::UsageContext::new("封面文案")
                .project(Some(yekitrim_core::path::display(std::path::Path::new(&dir)))),
        );

    let system = cover_title_system_prompt();
    let prompt_text = cover_title_user_prompt(&reference_text, &context);
    let content = client.chat(&system, &prompt_text).await
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("LLM 生成失败: {e}")))?;

    let mut lines = parse_cover_title_lines(&content);
    // 封面最多 3 行 + 总字数 8~40，避免模型不守格式导致排版溢出
    lines.truncate(3);
    if lines.is_empty() {
        return Err(err(StatusCode::INTERNAL_SERVER_ERROR, "LLM 返回空标题"));
    }
    trim_title_lines(&mut lines, 40);
    keep_single_emoji(&mut lines);
    // 模型偶尔会漏 emoji：优先沿用参考标题里的 emoji，没有就补一个
    if count_emoji_in_lines(&lines) == 0 {
        trim_title_lines(&mut lines, 39);
        if let Some(last) = lines.last_mut() {
            let emoji = first_emoji(&reference_text).unwrap_or('✨');
            last.push(' ');
            last.push(emoji);
        }
    }
    ok(serde_json::json!({ "ok": true, "title_lines": lines }))
}


pub(crate) async fn polish_cover_config(State(_state): State<Arc<AppState>>, Json(body): Json<serde_json::Value>) -> ApiResult {
    let dir = body.get("dir").and_then(|v| v.as_str()).ok_or_else(|| err(StatusCode::BAD_REQUEST, "缺少 dir"))?;
    let path = validated_project_dir(dir)?;
    if !{
        let __p = path.join("polish.toml");
        // 同步 stat 不能留在 async worker 上（api/mod.rs:44-51 ✓）：
        // 工程常在慢盘/网络盘上，一次 exists() 就要几百 ms ✗
        tokio::task::spawn_blocking(move || __p.exists())
            .await
            .unwrap_or(false)
    } { return Err(err(StatusCode::BAD_REQUEST, "未初始化 polish")); }
    // 整个 read-modify-write 期间持锁：封面设计保存与时间轴保存会并发写同一份 polish.toml
    let _guard = yekitrim_core::filelock::lock_path(&path.join("polish.toml")).await;
    let mut cfg = yekitrim_polish::project::load_project(&path).map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("加载失败: {e}")))?;
    if let Some(fp) = body.get("frame_pos").and_then(|v| v.as_f64()) { cfg.cover.frame_pos = fp.clamp(0.0,1.0); }
    if let Some(lines) = body.get("title_lines").and_then(|v| v.as_array()) {
        let ls = lines.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect::<Vec<_>>();
        cfg.cover.title_lines = ls;
    }
    if let Some(text) = body.get("title_text").and_then(|v| v.as_str()) {
        cfg.project.name = text.to_string();
        cfg.cover.title_lines = vec![text.to_string()];
    }
    if let Some(cover) = body.get("cover").and_then(|v| v.as_object()) {
        if let Some(v) = cover.get("frame_pos").and_then(|v| v.as_f64()) { cfg.cover.frame_pos = v.clamp(0.0,1.0); }
        // 封面字体：值 = 字体库里的 id，空串 = 回到系统字体栈（与旧工程一致）
        if let Some(v) = cover.get("font").and_then(|v| v.as_str()) { cfg.cover.font = v.trim().to_string(); }
        if let Some(v) = cover.get("font_size").and_then(|v| v.as_u64()) { cfg.cover.font_size = v as u32; }
        // 封面标题的取样温度（**null / 缺省 = 自动**）：只影响「LLM 生成标题」那一次调用。
        // 显式给了就存（夹 0~2），不在这里套 0.8 下限 —— 想完全稳定就该能设 0。
        if let Some(v) = cover.get("title_temperature") {
            cfg.cover.title_temperature = v.as_f64().map(|t| t.clamp(0.0, 2.0));
        }
        if let Some(v) = cover.get("stroke").and_then(|v| v.as_u64()) { cfg.cover.stroke = v as u32; }
        if let Some(v) = cover.get("hl_color").and_then(|v| v.as_str()) { cfg.cover.hl_color = v.to_string(); }
        // 高亮词相对字号：夹到 [0.5, 2.0]（超出这个区间不是排版、是事故）
        if let Some(v) = cover.get("hl_font_scale").and_then(|v| v.as_f64()) { cfg.cover.hl_font_scale = v.clamp(0.5, 2.0); }
        if let Some(v) = cover.get("text_width").and_then(|v| v.as_f64()) { cfg.cover.text_width = v; }
        if let Some(v) = cover.get("text_x").and_then(|v| v.as_str()) { cfg.cover.text_x = v.to_string(); }
        if let Some(v) = cover.get("text_y").and_then(|v| v.as_str()) { cfg.cover.text_y = v.to_string(); }
        if let Some(v) = cover.get("gradient_dir").and_then(|v| v.as_str()) { cfg.cover.gradient_dir = v.to_string(); }
        if let Some(v) = cover.get("vignette").and_then(|v| v.as_f64()) { cfg.cover.vignette = v; }
        if let Some(v) = cover.get("letter_spacing").and_then(|v| v.as_str()) { cfg.cover.letter_spacing = v.to_string(); }
        if let Some(v) = cover.get("line_height").and_then(|v| v.as_f64()) { cfg.cover.line_height = v; }
        if let Some(v) = cover.get("text_align").and_then(|v| v.as_str()) { cfg.cover.text_align = v.to_string(); }
        if let Some(v) = cover.get("shadow_dir").and_then(|v| v.as_str()) { cfg.cover.shadow_dir = v.to_string(); }
        if let Some(v) = cover.get("shadow_intensity").and_then(|v| v.as_f64()) { cfg.cover.shadow_intensity = v.clamp(0.0, 1.0); }
        if let Some(v) = cover.get("text_shadow").and_then(|v| v.as_bool()) { cfg.cover.text_shadow = v; }
        if let Some(arr) = cover.get("highlight_keywords").and_then(|v| v.as_array()) {
            cfg.cover.highlight_keywords = arr.iter()
                .filter_map(|v| v.as_str().map(|s| s.trim().to_string()))
                .filter(|s| !s.is_empty())
                .collect();
        }
        if let Some(v) = cover.get("bg_x").and_then(|v| v.as_f64()) { cfg.cover.bg_x = v; }
        if let Some(v) = cover.get("bg_y").and_then(|v| v.as_f64()) { cfg.cover.bg_y = v; }
        if let Some(v) = cover.get("bg_zoom").and_then(|v| v.as_f64()) { cfg.cover.bg_zoom = v.clamp(0.5, 3.0); }
        // 背景模式 / 纯色 / 模糊倍率：颜色必须净化（它直接进 HTML 模板），
        // 倍率按 cover.rs 的同一组边界夹取，避免滑杆与服务端各夹各的
        if let Some(v) = cover.get("bg_mode").and_then(|v| v.as_str()) {
            cfg.cover.bg_mode = v.trim().to_ascii_lowercase();
        }
        if let Some(v) = cover.get("bg_color").and_then(|v| v.as_str()) {
            cfg.cover.bg_color = yekitrim_polish::cover::sanitize_hex_color(v, "#000000");
        }
        if let Some(v) = cover.get("blur_scale").and_then(|v| v.as_f64()) {
            cfg.cover.blur_scale = v.clamp(
                yekitrim_polish::cover::COVER_BLUR_SCALE_MIN,
                yekitrim_polish::cover::COVER_BLUR_SCALE_MAX,
            );
        }
        if let Some(v) = cover.get("effects").and_then(|v| v.as_array()) {
            cfg.cover.effects = v.iter().filter_map(|e| {
                // 图层栈用 `fx:<id>` 指向这条特效；设计器新建时给个短 id。
                // 老数据为空 = 栈里按数组序兜底（`resolve_cover_stack`），不报错也不丢层。
                let id = e.get("id").and_then(|x| x.as_str()).unwrap_or("").trim().to_string();
                let kind = e.get("kind").and_then(|x| x.as_str())?.to_string();
                let amount = e.get("amount").and_then(|x| x.as_f64()).unwrap_or(0.3);
                let x = e.get("x").and_then(|x| x.as_f64()).unwrap_or(50.0);
                let y = e.get("y").and_then(|x| x.as_f64()).unwrap_or(50.0);
                let color = e.get("color").and_then(|x| x.as_str()).unwrap_or("#000000").to_string();
                let text = e.get("text").and_then(|x| x.as_str()).unwrap_or("").to_string();
                Some(yekitrim_polish::project::CoverEffect { id, kind, amount, x, y, color, text })
            }).collect();
        }
        // 图层栈（顺序 + 可见性）：**整数组替换**。给空数组 = 回到默认栈（与老工程同义）。
        // ⚠️ 顺序与可见性**只住在这一个字段里**（`effects` 只带 id、不带 visible），别再开第二个开关。
        if let Some(arr) = cover.get("layers").and_then(|v| v.as_array()) {
            cfg.cover.layers = arr.iter().filter_map(|l| {
                let id = l.get("id").and_then(|x| x.as_str())?.trim().to_string();
                if id.is_empty() { return None; }
                let visible = l.get("visible").and_then(|x| x.as_bool()).unwrap_or(true);
                Some(yekitrim_polish::project::CoverLayer { id, visible })
            }).collect();
        }
        if let Some(lines) = cover.get("title_lines").and_then(|v| v.as_array()) {
            cfg.cover.title_lines = lines.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect();
        }
        if let Some(v) = cover.get("image").and_then(|v| v.as_str()) { cfg.cover.image = v.to_string(); }
    }
    let raw = cfg.to_toml_commented().map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("序列化失败: {e}")))?;
    std::fs::write(path.join("polish.toml"), raw).map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("写入失败: {e}")))?;
    ok(serde_json::json!({ "ok": true }))
}


/// 「去静音」快捷操作：把检测到的长静音段写进 / 移出 `mute_ranges`（时间轴「屏蔽」轨道）。
///
/// 不重编码、不改动视频文件；`action` 为 `apply`（默认）或 `clear`。
pub(crate) async fn polish_silence(State(_state): State<Arc<AppState>>, Json(body): Json<serde_json::Value>) -> ApiResult {
    let dir = body.get("dir").and_then(|v| v.as_str()).ok_or_else(|| err(StatusCode::BAD_REQUEST, "缺少 dir"))?;
    let action = body.get("action").and_then(|v| v.as_str()).unwrap_or("apply");
    let path = validated_project_dir(dir)?;
    if !{
        let __p = path.join("polish.toml");
        // 同步 stat 不能留在 async worker 上（api/mod.rs:44-51 ✓）：
        // 工程常在慢盘/网络盘上，一次 exists() 就要几百 ms ✗
        tokio::task::spawn_blocking(move || __p.exists())
            .await
            .unwrap_or(false)
    } { return Err(err(StatusCode::BAD_REQUEST, "未初始化 polish")); }
    // 去静音要跑 ffmpeg silencedetect（整个视频）+ 读改写 polish.toml，全是同步重活：
    // 锁在 await 里持有，活儿下 blocking 池（`with_path_lock` 的闭包是在当前线程跑的）
    let lock_file = path.join("polish.toml");
    let _guard = yekitrim_core::filelock::lock_path(&lock_file).await;
    let action = action.to_string();
    blocking(move || match action.as_str() {
        "clear" => {
            let removed = yekitrim_polish::project::clear_silence(&path)
                .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("移除静音屏蔽失败: {e:#}")))?;
            ok(serde_json::json!({ "ok": true, "applied": false, "removed": removed, "total": yekitrim_polish::project::mute_range_count(&path) }))
        }
        "apply" => {
            let applied = yekitrim_polish::project::apply_silence(&path)
                .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("写入屏蔽轨道失败: {e:#}")))?;
            // 与当前屏蔽状态**相加**：added = 真正新加进去的区间，total = 相加后的轨道总段数
            ok(serde_json::json!({
                "ok": true,
                "applied": !applied.ranges.is_empty(),
                "detected": applied.ranges.len(),
                "added": applied.added,
                "total": applied.total,
                "ranges": applied.ranges.iter().map(|(s, e)| serde_json::json!({ "start": s, "end": e })).collect::<Vec<_>>(),
                "message": if applied.ranges.is_empty() {
                    "未检测到长静音段"
                } else if applied.added == 0 {
                    "检测到的静音已在屏蔽轨道里"
                } else {
                    ""
                },
            }))
        }
        _ => Err(err(StatusCode::BAD_REQUEST, "action 必须是 apply 或 clear")),
    })
    .await
}


/// 兼容旧接口：version=silent/original 等价于「去静音」的 apply/clear。
pub(crate) async fn polish_select_version(State(state): State<Arc<AppState>>, Json(body): Json<serde_json::Value>) -> ApiResult {
    let version = body.get("version").and_then(|v| v.as_str()).unwrap_or("original");
    let action = if version == "silent" { "apply" } else { "clear" };
    let mut forwarded = body.clone();
    forwarded["action"] = serde_json::json!(action);
    polish_silence(State(state), Json(forwarded)).await
}


/// 写完背景图之后的收尾：切 `bg_mode = "image"` 并回写 `polish.toml`（**同步 IO，调用方须在 blocking 里**）。
fn set_bg_image_mode(path: &Path) -> Result<(), AppError> {
    let mut cfg = yekitrim_polish::project::load_project(path)
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("加载失败: {e}")))?;
    // 导入即切到「自定义图片」背景；关键帧与 cover.image 都不动
    cfg.cover.bg_mode = "image".into();
    let raw = cfg
        .to_toml_commented()
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("序列化失败: {e}")))?;
    std::fs::write(path.join("polish.toml"), raw)
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("写入失败: {e}")))?;
    Ok(())
}


pub(crate) async fn polish_cover_background(State(_state): State<Arc<AppState>>, Json(body): Json<serde_json::Value>) -> ApiResult {
    let dir = body.get("dir").and_then(|v| v.as_str()).unwrap_or("").to_string();
    // `validated_project_dir` 内部要 canonicalize（网络盘上几百毫秒）→ blocking
    let path = blocking(move || validated_project_dir(&dir)).await?;
    let polish_toml = path.join("polish.toml");
    if !blocking({
        let polish_toml = polish_toml.clone();
        move || Ok(polish_toml.exists())
    })
    .await?
    {
        return Err(err(StatusCode::BAD_REQUEST, "目录未初始化 polish"));
    }
    if let Some(data) = body.get("image_data").and_then(|v| v.as_str()) {
        use base64::Engine as _;
        let bytes = base64::engine::general_purpose::STANDARD.decode(data)
            .map_err(|e| err(StatusCode::BAD_REQUEST, format!("图片解码失败: {e}")))?;
        if bytes.len() > 32 * 1024 * 1024 {
            return Err(err(StatusCode::BAD_REQUEST, "背景图过大（上限 32MB）"));
        }
        // 与其它 polish.toml 写入串行化
        let _guard = yekitrim_core::filelock::lock_path(&polish_toml).await;
        return blocking(move || {
            // ⚠️ 写**背景层自己的文件** `_cover_bg.jpg`：封面三层 = 文字 / 关键帧(_cover_frame.jpg) / 背景，
            // 导入背景不能把关键帧那层盖掉（用户报过"又把关键帧那一层给我删了"）。
            std::fs::write(path.join(yekitrim_polish::cover::COVER_BG_IMG), bytes)
                .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("写入失败: {e}")))?;
            set_bg_image_mode(&path)?;
            ok(serde_json::json!({ "ok": true, "bg_mode": "image" }))
        })
        .await;
    }
    if let Some(src) = body.get("source").and_then(|v| v.as_str()) {
        let src = src.to_string();
        // 与其它 polish.toml 写入串行化
        let _guard = yekitrim_core::filelock::lock_path(&polish_toml).await;
        return blocking(move || {
            let sp = PathBuf::from(&src);
            if !sp.exists() { return Err(err(StatusCode::BAD_REQUEST, "背景文件不存在")); }
            // 同上：走背景层的文件，别碰关键帧
            std::fs::copy(&sp, path.join(yekitrim_polish::cover::COVER_BG_IMG))
                .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("复制失败: {e}")))?;
            set_bg_image_mode(&path)?;
            ok(serde_json::json!({ "ok": true }))
        })
        .await;
    }
    Err(err(StatusCode::BAD_REQUEST, "需要 image_data 或 source"))
}


pub(crate) async fn polish_cover_frame(State(_state): State<Arc<AppState>>, Json(body): Json<serde_json::Value>) -> ApiResult {
    let dir = body.get("dir").and_then(|v| v.as_str()).ok_or_else(|| err(StatusCode::BAD_REQUEST, "缺少 dir"))?;
    let path = validated_project_dir(dir)?;
    if !{
        let __p = path.join("polish.toml");
        // 同步 stat 不能留在 async worker 上（api/mod.rs:44-51 ✓）：
        // 工程常在慢盘/网络盘上，一次 exists() 就要几百 ms ✗
        tokio::task::spawn_blocking(move || __p.exists())
            .await
            .unwrap_or(false)
    } {
        return Err(err(StatusCode::BAD_REQUEST, "目录未初始化 polish"));
    }
    // 与其它 polish.toml 写入串行化（本函数最后会写回 cover.image）；
    // 截帧要跑 ffmpeg（网络盘上数百 ms 到数秒）→ 整段下 blocking 池，别占 worker
    let _guard = yekitrim_core::filelock::lock_path(&path.join("polish.toml")).await;
    let want_time = body
        .get("time")
        .and_then(|v| v.as_f64())
        .or_else(|| body.get("sec").and_then(|v| v.as_f64()));
    let time = blocking(move || {
        let config = yekitrim_polish::project::load_project(&path)
            .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("加载失败: {e}")))?;
        let resolved = yekitrim_polish::render::resolve_paths(&path, &config);
        let duration = config.segments.last().map(|s| s.end).unwrap_or(0.0);
        let time = want_time.unwrap_or(duration * config.cover.frame_pos).max(0.1);
        let frame_out = yekitrim_core::process::cmd("ffmpeg")
            .args(["-ss", &time.to_string(), "-i", &resolved.video,
                   "-vframes", "1", "-f", "image2pipe", "-vcodec", "mjpeg", "-q:v", "3", "-"])
            .stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::null())
            .output().map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("ffmpeg 启动失败: {e}")))?;
        if frame_out.stdout.is_empty() {
            return Err(err(StatusCode::INTERNAL_SERVER_ERROR, "截帧失败，视频可能不可读或时间超出范围"));
        }
        std::fs::write(path.join("_cover_frame.jpg"), &frame_out.stdout)
            .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("写入封面帧失败: {e}")))?;
        // 使用截取的帧作为封面背景，并清空自定义 image，保证后续 cover 使用该帧
        let mut cfg = yekitrim_polish::project::load_project(&path)
            .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("加载失败: {e}")))?;
        cfg.cover.image = String::new();
        let raw = cfg.to_toml_commented().map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("序列化失败: {e}")))?;
        std::fs::write(path.join("polish.toml"), raw).map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("写入失败: {e}")))?;
        Ok(time)
    })
    .await?;
    ok(serde_json::json!({ "ok": true, "time": time }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::test_support::*;
    use axum::body::Body;

    // ⚠️ 这些用例是**按原字节搬来**的（含原本在文件顶层、缩进为 0 的那批）：
    //    统一重排缩进会动到多行字符串字面量，判据会因此悄悄变。


    /// **封面文案挂的是 `Title` 链路**（不是 Utility）：用户口径「加一个标题生成」，
    /// 而贴纸包/扫描标签仍属零散单发 —— 挂错的话两者会共用一组覆盖，改一个动一片。
    #[test]
    fn cover_title_uses_the_title_link() {
        assert_eq!(
            cover_title_link(),
            yekitrim_core::config::LlmLink::Title,
            "封面文案要挂标题生成链路"
        );
        assert_ne!(cover_title_link(), yekitrim_core::config::LlmLink::Utility);
        // 与它原来挂在 Utility 上时**行为一致**：默认关思考（R4）
        assert_eq!(
            yekitrim_core::config::LlmLink::Title.default_thinking(),
            yekitrim_core::config::LlmLink::Utility.default_thinking()
        );
    }


    /// 封面标题温度：显式设了就是它（0 也要能拿到 0），没设才抬到 0.8 下限。
    #[test]
    fn cover_title_temperature_respects_explicit_value() {
        assert_eq!(cover_title_temperature(None, 0.0), 0.8, "没设 = 下限 0.8");
        assert_eq!(cover_title_temperature(None, 1.1), 1.1, "用户配得更高就照用");
        assert_eq!(cover_title_temperature(Some(0.0), 0.0), 0.0, "显式 0 = 完全稳定，不许被下限顶回去");
        assert_eq!(cover_title_temperature(Some(1.2), 0.0), 1.2);
        assert_eq!(cover_title_temperature(Some(9.0), 0.0), 2.0, "夹到 2");
    }


    /// 封面标题的 prompt 必须**把参考标题当口吻基准**（2026-10-19 用户报「没有幽默串子那种想点进去的欲望」）。
    ///
    /// 旧 prompt 只写「参考标题用于提取信息点」+ 一列格式约束，模型于是写四平八稳的概述。
    /// 这两条钉住的是**判据本身**：提风格、禁概述、第一行是钩子、优先引原话。
    #[test]
    fn cover_title_prompt_demands_reference_style() {
        let s = cover_title_system_prompt();
        for needle in ["风格基准", "模仿", "串子", "平铺直叙", "第一行必须是钩子", "最出圈的那句原话"] {
            assert!(s.contains(needle), "封面标题 prompt 缺「{needle}」：{s}");
        }
        // 格式约束不能丢（封面排版靠它）
        for needle in ["1~3 行", "8~40 字", "1 个 emoji", "title_lines"] {
            assert!(s.contains(needle), "格式约束缺「{needle}」");
        }
    }


    /// 参考标题要**真的进 user prompt**，并且排在素材之前（它是风格基准，不是附录）。
    #[test]
    fn cover_title_user_prompt_puts_reference_first() {
        let u = cover_title_user_prompt("这也能赢？😂\n嘴硬到最后一秒", "[事件/分段 1] 0.00s-10.00s\n  有效字幕:\n  [1.00-2.00] 我裂开了");
        assert!(u.contains("这也能赢"), "参考标题必须原样进 prompt：{u}");
        assert!(u.contains("风格必须像它们"), "必须把参考标题说明成风格基准：{u}");
        assert!(u.contains("最出圈的原话"), "要提示从字幕里挑原话：{u}");
        let ref_at = u.find("这也能赢").unwrap();
        let ctx_at = u.find("[事件/分段 1]").unwrap();
        assert!(ref_at < ctx_at, "参考标题必须排在素材之前");
    }


/// 封面背景设置（bg_mode / bg_color / blur_scale）必须真的落进 polish.toml：
/// 设计器保存走的是**白名单 patch**，漏一个键就是"点了没反应"。
/// 颜色还要在服务端净化（它直接进 HTML 模板）。
#[tokio::test]
async fn cover_config_persists_background_mode_and_sanitizes_color() {
    let t = test_app().await;
    let clip = t.root.join("output/clips/clip-01");
    std::fs::create_dir_all(&clip).unwrap();
    std::fs::write(clip.join("polish.toml"), "[source]\nvideo = \"clip.mp4\"\nsubtitles = \"clip.srt\"\n").unwrap();
    let dir = clip.to_string_lossy().to_string();

    let body = serde_json::json!({
        "dir": dir,
        "cover": { "bg_mode": "blur", "bg_color": "#AABBCC", "blur_scale": 2.5 }
    })
    .to_string();
    let res = t
        .call(
            t.req("POST", "/polish/cover-config")
                .header(axum::http::header::CONTENT_TYPE, "application/json")
                .body(Body::from(body))
                .unwrap(),
        )
        .await;
    assert_eq!(res.status(), StatusCode::OK);

    let cfg = yekitrim_polish::project::load_project(&clip).unwrap();
    assert_eq!(cfg.cover.bg_mode, "blur");
    assert_eq!(cfg.cover.bg_color, "#aabbcc", "颜色应被规范化成小写 #rrggbb");
    assert!((cfg.cover.blur_scale - 2.5).abs() < 1e-9);

    // 非法颜色退回默认；倍率夹到上限
    let body = serde_json::json!({
        "dir": dir,
        "cover": { "bg_mode": "color", "bg_color": "red;background:url(x)", "blur_scale": 99.0 }
    })
    .to_string();
    let res = t
        .call(
            t.req("POST", "/polish/cover-config")
                .header(axum::http::header::CONTENT_TYPE, "application/json")
                .body(Body::from(body))
                .unwrap(),
        )
        .await;
    assert_eq!(res.status(), StatusCode::OK);
    let cfg = yekitrim_polish::project::load_project(&clip).unwrap();
    assert_eq!(cfg.cover.bg_color, "#000000", "非法颜色必须被净化成默认黑");
    assert!((cfg.cover.blur_scale - 4.0).abs() < 1e-9, "倍率要夹到上限");
    // 保存后仍能在配置文件里看到这些键（注释模板会带上说明）
    let raw = std::fs::read_to_string(clip.join("polish.toml")).unwrap();
    assert!(raw.contains("bg_mode") && raw.contains("blur_scale"), "polish.toml 缺键：{raw}");

    // 高亮词字号：白名单必须收这个键，且夹到 [0.5, 2.0]。
    // 漏收 = 用户在设计器里调完、重开又变回默认（"改了没反应"），而这条链上没有别的报错点。
    for (sent, want) in [(1.4_f64, 1.4_f64), (9.0, 2.0), (0.1, 0.5)] {
        let body = serde_json::json!({ "dir": dir, "cover": { "hl_font_scale": sent } }).to_string();
        let res = t
            .call(
                t.req("POST", "/polish/cover-config")
                    .header(axum::http::header::CONTENT_TYPE, "application/json")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await;
        assert_eq!(res.status(), StatusCode::OK);
        let cfg = yekitrim_polish::project::load_project(&clip).unwrap();
        assert!(
            (cfg.cover.hl_font_scale - want).abs() < 1e-9,
            "hl_font_scale 传 {sent} 应落成 {want}，实际 {}",
            cfg.cover.hl_font_scale
        );
    }
    let raw = std::fs::read_to_string(clip.join("polish.toml")).unwrap();
    assert!(raw.contains("hl_font_scale"), "polish.toml 缺 hl_font_scale 键：{raw}");
}


/// 图层栈（`[cover] layers`）与特效 id 也必须真的落进 `polish.toml`：设计器保存走的是
/// **白名单 patch**，漏一个键就是"拖了半天没反应、重开又变回原样"。
#[tokio::test]
async fn cover_config_persists_layer_stack_and_effect_ids() {
    let t = test_app().await;
    let clip = t.root.join("output/clips/clip-02");
    std::fs::create_dir_all(&clip).unwrap();
    std::fs::write(clip.join("polish.toml"), "[source]\nvideo = \"clip.mp4\"\nsubtitles = \"clip.srt\"\n").unwrap();
    let dir = clip.to_string_lossy().to_string();

    let body = serde_json::json!({
        "dir": dir,
        "cover": {
            "effects": [
                { "id": "e1", "kind": "glow", "amount": 0.5, "x": 10.0, "y": 20.0 },
                { "kind": "emoji", "amount": 0.4, "x": 65.0, "y": 50.0, "text": "😆" }
            ],
            "layers": [
                { "id": "title", "visible": true },
                { "id": "bg", "visible": false },
                { "id": "fx:e1", "visible": true },
                { "id": "", "visible": true },
                { "id": "nope", "visible": true }
            ]
        }
    })
    .to_string();
    let res = t
        .call(
            t.req("POST", "/polish/cover-config")
                .header(axum::http::header::CONTENT_TYPE, "application/json")
                .body(Body::from(body))
                .unwrap(),
        )
        .await;
    assert_eq!(res.status(), StatusCode::OK);

    let cfg = yekitrim_polish::project::load_project(&clip).unwrap();
    assert_eq!(cfg.cover.effects.len(), 2);
    assert_eq!(cfg.cover.effects[0].id, "e1");
    assert_eq!(cfg.cover.effects[1].id, "", "没给 id 的特效保持空（解析侧按数组序兜底）");
    let ids: Vec<&str> = cfg.cover.layers.iter().map(|l| l.id.as_str()).collect();
    assert_eq!(ids, vec!["title", "bg", "fx:e1", "nope"], "空 id 丢掉、其余原样留下：{ids:?}");
    assert!(!cfg.cover.layers[1].visible, "visible=false 要落盘");

    // 落盘后再解析：未知 id 忽略、缺的层补齐、顺序照栈（title 在栈底 = 第一条）
    let stack: Vec<String> = yekitrim_polish::cover::resolve_cover_stack(&cfg.cover)
        .into_iter()
        .map(|l| l.id)
        .collect();
    assert_eq!(stack.first().map(String::as_str), Some("title"), "{stack:?}");
    assert!(stack.contains(&"frame".to_string()) && stack.contains(&"filter".to_string()), "{stack:?}");
    assert!(!stack.iter().any(|s| s == "nope"), "未知 id 不该出现在解析结果里：{stack:?}");
    let raw = std::fs::read_to_string(clip.join("polish.toml")).unwrap();
    assert!(raw.contains("layers"), "polish.toml 缺 layers 键：{raw}");
}

}