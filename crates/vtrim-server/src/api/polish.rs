//! `api::polish`：精修编辑器后端（工程读写 / 字幕 / 贴纸 / 封面 / 预览）。

use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::{
    extract::{Extension, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use vtrim_service::Workspace;

use crate::model::AppState;
use crate::jobs::{JobClass, JobSpec};
use super::*;
use super::{pipeline::*, project_data::*, render::*};

// ===================== Polish =====================

pub(super) fn polish_projects_impl(ws: &Workspace) -> ApiResult {
    let clips = ws.output_dir().join("clips");
    let mut out = vec![];
    if clips.exists() {
        for entry in std::fs::read_dir(&clips).map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("读取失败: {e}")))? {
            let entry = entry.map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("读取失败: {e}")))?;
            let dir = entry.path();
            if !dir.is_dir() { continue; }
            let title = std::fs::read_to_string(dir.join("clip.json")).ok()
                .and_then(|raw| serde_json::from_str::<serde_json::Value>(&raw).ok())
                .and_then(|v| v.get("title").and_then(|t| t.as_str()).map(|s| s.to_string()))
                .unwrap_or_default();
            let polish_ready = dir.join("polish.toml").exists();
            let processed = polish_ready && std::fs::read_to_string(dir.join("polish.toml")).ok()
                .map(|raw| raw.contains("processed = true")).unwrap_or(false);
            let rendered_file = latest_numbered_file(&dir, "vtuber-clip", "mp4");
            let rendered = rendered_file.is_some();
            let cover_file = latest_numbered_file(&dir, "cover", "jpg");
            let cover = cover_file.is_some();
            let mut version = "original";
            let mut mute_range_count = 0usize;
            if polish_ready {
                if let Ok(cfg) = vtrim_polish::project::load_project(&dir) {
                    // 消音版 = 静音区间已写入屏蔽轨道（video 始终是未改动的 clip.mp4）
                    if !cfg.silence_ranges.is_empty() { version = "silent"; }
                    mute_range_count = cfg.mute_ranges.len();
                }
            }
            out.push(serde_json::json!({
                "dir": vtrim_core::path::display(&dir),
                "title": title,
                "polish_ready": polish_ready,
                "processed": processed,
                "rendered": rendered,
                "rendered_file": rendered_file,
                "cover": cover,
                "cover_file": cover_file,
                "version": version,
                "mute_range_count": mute_range_count,
            }));
        }
    }
    out.sort_by(|a,b| a["dir"].as_str().unwrap_or("").cmp(b["dir"].as_str().unwrap_or("")));
    ok(serde_json::json!(out))
}

/// polish 任务的附加参数（不同 kind 用法不同；用一个结构体是为了别再加第 7 个位置参数）。
#[derive(Default, Clone)]
pub(super) struct PolishJobOpts {
    /// `render`：输出文件名（真开跑时才落盘）
    pub output_file: Option<String>,
    /// `llm`：true = **全部重做**（先清掉断点续传缓存）
    pub full: bool,
    /// `llm_segment`：只重做这一段的事件（下标见 `polish.toml` 的 `[[segments]]`）
    pub segment_index: Option<usize>,
}

pub(super) async fn polish_spawn(
    state: &AppState,
    owner: i64,
    project: Option<i64>,
    kind: &str,
    dir: String,
    opts: PolishJobOpts,
) -> ApiResult {
    // dir 必须先校验：`dir: ""` 会让 cmd_init 在 server 进程的 CWD 里写出
    // polish.toml / data / sfx，任意绝对路径也会被当作合法工程目录。
    // 归属由 `scope::enforce_scope` 中间件保证（只能是自己项目内的目录）。
    let path = validated_project_dir(&dir)?;
    let dir_name = path
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| dir.clone());
    // 资源域 = 这个片段目录：与同片段的其它精修任务、以及"单条重切"互斥（冲突则排队）。
    // 档位按 kind：渲染重活、LLM 略重、init/process/cover 轻活（`JobClass::for_polish`）。
    let keys = crate::scope_key::polish_keys(project, &path);
    let class = JobClass::for_polish(kind);
    let handle = state
        .jobs
        .create(
            owner,
            JobSpec::new(format!("polish_{}", kind), format!("polish {kind} · {dir_name}"), keys, class),
        )
        .await;
    let id = handle.id.clone();
    let queued = handle.queued;
    if let Some(pid) = project {
        state.jobs.attach_project(&id, pid).await;
    }
    let jobs = state.jobs.clone();
    let job_id = id.clone();
    let kind = kind.to_string();
    // 子进程日志落在服务数据目录下；data_dir 先克隆出来（spawn 的任务要 'static）
    let data_dir = state.auth.data_dir().clone();
    // init/llm/process 都是整段同步代码（读改 polish.toml、逐段后处理），跑专用线程；
    // render/cover 是子进程 + 流式跟随，本来就不占 worker，跟着一起进去也无妨
    spawn_job(jobs.clone(), job_id.clone(), move || async move {
        if handle.wait_start().await.is_err() {
            // 排队期间被取消：没开工。complete 只为释放占位并唤醒后面的任务
            jobs.complete(&job_id).await;
            return;
        }
        let result = async {
            match kind.as_str() {
                "init" => vtrim_polish::commands::cmd_init(&path).await,
                // LLM 是显式步骤，process 隐式：完成后同一 job 内自动执行后处理
                "llm" => {
                    jobs.log(&job_id, "info", if opts.full {
                        "LLM 生成中（全部重做，不读断点续传）…"
                    } else {
                        "LLM 生成中…"
                    }).await;
                    vtrim_polish::commands::cmd_llm(&path, opts.full).await?;
                    jobs.log(&job_id, "info", "LLM 完成，自动执行 process 后处理…").await;
                    vtrim_polish::process::cmd_process(&path).await
                }
                // 只重做某一段的事件（编辑器：时间轴 → 分段属性）。失败会**保留该段原有事件**，
                // 所以这里 fail 掉任务不会让用户丢东西；后处理照跑（别的段不动，幂等）。
                "llm_segment" => {
                    let idx = opts
                        .segment_index
                        .ok_or_else(|| anyhow::anyhow!("缺少 segment_index"))?;
                    jobs.log(&job_id, "info", format!("只重做分段 #{idx} 的事件…")).await;
                    vtrim_polish::commands::cmd_llm_segment(&path, idx).await?;
                    jobs.log(&job_id, "info", "完成，自动执行 process 后处理…").await;
                    vtrim_polish::process::cmd_process(&path).await
                }
                "process" => vtrim_polish::process::cmd_process(&path).await,
                "render" => {
                    // 输出文件名在**真开跑之后**才落到 polish.toml（排队期间写会覆盖别人
                    // 的改动，而且任务可能还要等很久）。update_polish_toml 自己带路径锁。
                    if let Some(name) = opts.output_file.clone() {
                        vtrim_polish::project::update_polish_toml(&path, |cfg| {
                            cfg.project.output = name.clone();
                        })
                        .await?;
                    }
                    // 用外部 CLI 子进程渲染：headless Chrome 在 WebUI 服务进程内启动会超时。
                    // 输出流式回灌任务日志 + 取消时杀进程树，见 run_polish_child。
                    run_polish_child(&data_dir, &jobs, &job_id, "render", &path).await
                }
                "cover" => {
                    // 与 render 同理：封面也走外部 CLI（同样要能看进度、能取消）
                    run_polish_child(&data_dir, &jobs, &job_id, "cover", &path).await
                }
                _ => anyhow::bail!("未知 kind"),
            }
        }.await;
        if let Err(e) = result {
            jobs.fail(&job_id, format!("{e:#}")).await;
        } else {
            jobs.complete(&job_id).await;
        }
    });
    ok(serde_json::json!({ "ok": true, "id": id, "queued": queued }))
}

/// polish 路由的作用域项目 id（由 `scope::enforce_scope` 注入；0 = 未解析）。
pub(super) fn scope_pid(ext: Option<axum::extract::Extension<crate::scope::ScopeProjectId>>) -> Option<i64> {
    ext.map(|e| e.0.0).filter(|v| *v > 0)
}

pub(super) async fn polish_init(State(state): State<Arc<AppState>>, Extension(user): Extension<crate::auth::CurrentUser>, pid: Option<Extension<crate::scope::ScopeProjectId>>, Json(body): Json<serde_json::Value>) -> ApiResult {
    let dir = body.get("dir").and_then(|v| v.as_str()).unwrap_or("").to_string();
    polish_spawn(&state, user.id, scope_pid(pid), "init", dir, PolishJobOpts::default()).await
}

/// LLM 剪辑（整片）。
///
/// `full`（默认 **true**）= 全部重做：清掉断点续传缓存、重新分段 + 逐段生成。
/// 用户点这颗按钮的意图就是"重做一遍"；半吊子的续传反而会让事件看起来被换乱
/// （2026-09-20 用户报的「续传把所有事件删了只留一个 auto」）。
/// 想省调用请用**单段重做**（`/api/polish/llm-segment`）。
pub(super) async fn polish_llm(State(state): State<Arc<AppState>>, Extension(user): Extension<crate::auth::CurrentUser>, pid: Option<Extension<crate::scope::ScopeProjectId>>, Json(body): Json<serde_json::Value>) -> ApiResult {
    let dir = body.get("dir").and_then(|v| v.as_str()).unwrap_or("").to_string();
    // 缺省 true：宁可重做，也不要静默续传
    let full = body.get("full").and_then(|v| v.as_bool()).unwrap_or(true);
    polish_spawn(&state, user.id, scope_pid(pid), "llm", dir, PolishJobOpts { full, ..Default::default() }).await
}

/// 只重做某一段的事件（编辑器：时间轴 → 选中分段 → 「只重做本段事件」）。
pub(super) async fn polish_llm_segment(State(state): State<Arc<AppState>>, Extension(user): Extension<crate::auth::CurrentUser>, pid: Option<Extension<crate::scope::ScopeProjectId>>, Json(body): Json<serde_json::Value>) -> ApiResult {
    let dir = body.get("dir").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let index = body
        .get("index")
        .and_then(|v| v.as_u64())
        .ok_or_else(|| err(StatusCode::BAD_REQUEST, "缺少分段下标 index"))? as usize;
    polish_spawn(
        &state,
        user.id,
        scope_pid(pid),
        "llm_segment",
        dir,
        PolishJobOpts { segment_index: Some(index), ..Default::default() },
    )
    .await
}

pub(super) async fn polish_process(State(state): State<Arc<AppState>>, Extension(user): Extension<crate::auth::CurrentUser>, pid: Option<Extension<crate::scope::ScopeProjectId>>, Json(body): Json<serde_json::Value>) -> ApiResult {
    let dir = body.get("dir").and_then(|v| v.as_str()).unwrap_or("").to_string();
    polish_spawn(&state, user.id, scope_pid(pid), "process", dir, PolishJobOpts::default()).await
}

pub(super) async fn polish_render(State(state): State<Arc<AppState>>, Extension(user): Extension<crate::auth::CurrentUser>, pid: Option<Extension<crate::scope::ScopeProjectId>>, Json(body): Json<serde_json::Value>) -> ApiResult {
    let dir = body.get("dir").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let path = validated_project_dir(&dir)?;
    if !path.join("polish.toml").exists() { return Err(err(StatusCode::BAD_REQUEST, "目录未初始化 polish")); }
    // 输出文件名**不在这里写**：任务可能被排到后面，等它真开跑时再落到 polish.toml
    // （见 polish_spawn 的 render 分支）—— 否则排队期间别的任务/用户改动会被这次预写覆盖。
    let output_file = body
        .get("output_file")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());
    polish_spawn(&state, user.id, scope_pid(pid), "render", dir, PolishJobOpts { output_file, ..Default::default() }).await
}

pub(super) async fn polish_cover(State(state): State<Arc<AppState>>, Extension(user): Extension<crate::auth::CurrentUser>, pid: Option<Extension<crate::scope::ScopeProjectId>>, Json(body): Json<serde_json::Value>) -> ApiResult {
    let dir = body.get("dir").and_then(|v| v.as_str()).unwrap_or("").to_string();
    polish_spawn(&state, user.id, scope_pid(pid), "cover", dir, PolishJobOpts::default()).await
}


pub(super) fn cover_title_event_context(
    cfg: &vtrim_polish::project::ProjectFile,
    subtitles: &[vtrim_core::types::TranscriptSegment],
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
        let _ = writeln!(out, "");
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
    s = s.trim_start_matches(|c: char| matches!(c, '▶' | '▸' | '•' | '·')).trim();
    for prefix in ["- ", "* ", "• ", "· "] {
        if let Some(rest) = s.strip_prefix(prefix) {
            s = rest.trim();
            break;
        }
    }
    if let Some(pos) = s.find(|c: char| c == '.' || c == '、' || c == ')' || c == '）') {
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

pub(super) async fn polish_cover_title(State(_state): State<Arc<AppState>>, Json(body): Json<serde_json::Value>) -> ApiResult {
    let dir = body.get("dir").and_then(|v| v.as_str()).ok_or_else(|| err(StatusCode::BAD_REQUEST, "缺少 dir"))?;
    let path = validated_project_dir(dir)?;
    if !path.join("polish.toml").exists() {
        return Err(err(StatusCode::BAD_REQUEST, "目录未初始化 polish"));
    }

    // 只使用 clip.srt + polish.toml 作为输入；
    // project.name 仅作为“参考标题”参与生成，不读取 data/subtitles.json / clip-meta.json / clip.json。
    let cfg = vtrim_polish::project::load_project(&path)
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("加载 polish.toml 失败: {e}")))?;
    let srt_path = path.join("clip.srt");
    if !srt_path.exists() {
        return Err(err(StatusCode::BAD_REQUEST, "clip.srt 不存在"));
    }
    let raw = std::fs::read_to_string(&srt_path)
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("读取 clip.srt 失败: {e}")))?;
    let transcript = vtrim_core::srt::parse_srt(&raw)
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("解析 clip.srt 失败: {e}")))?;
    if transcript.segments.is_empty() {
        return Err(err(StatusCode::BAD_REQUEST, "clip.srt 暂无字幕，无法生成标题"));
    }
    let context = cover_title_event_context(&cfg, &transcript.segments);
    let global_cfg = vtrim_core::config::load_global_config();
    let reference_titles = vtrim_core::config::resolve_reference_titles(&global_cfg.detection);
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

    let llm = vtrim_core::config::load_llm_config();
    if llm.base_url.is_empty() || llm.api_key.is_empty() {
        return Err(err(StatusCode::BAD_REQUEST, "LLM 未配置"));
    }
    // 链路解析：零散单发链路（见 [llm.overrides.utility]）；thinking 的默认关思考由它给出
    let linked = llm.for_link(vtrim_core::config::LlmLink::Utility);
    let client = vtrim_core::llm::LlmClient::from_config(&linked)
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("LLM 配置错误: {e}")))?
        .with_timeout(60)
        .with_usage_context(
            vtrim_core::llm::usage::UsageContext::new("封面文案")
                .project(Some(vtrim_core::path::display(std::path::Path::new(&dir)))),
        );

    let system = format!(r#"你是直播切片封面标题助手。为当前切片生成封面大字标题，必须沿用事件检测阶段的标题规则：
{}
封面补充要求：
- 输出 1~3 行，适合封面大字排版；整组标题总字数严格 8~40 字，不要超长
- 只输出封面标题本身，不要输出 ▶ / 事件 / 分段 / 备注 / 上下文原文
- 必须且只能包含 1 个 emoji，放在句尾，不要放开头；参考标题有多个 emoji 也不要照搬
- 不要引号、不要解释、不要 markdown
- 参考标题只用于提取信息点，必须重新组织，不要照抄
- 只能依据输入的 polish.toml 逐个事件/分段与其对应的 clip.srt 字幕，不要引用输入之外的信息
- 输出 JSON：{{"title_lines": ["第一行", "第二行"]}}
"#, vtrim_core::detection::TITLE_RULES);
    let mut prompt_text = String::new();
    if !reference_text.is_empty() {
        prompt_text.push_str("参考标题示例（来自内建/全局配置，只参考句式、信息密度和 emoji，不要照抄）：\n");
        prompt_text.push_str(&reference_text);
        prompt_text.push_str("\n\n");
    }
    prompt_text.push_str("以下是 polish.toml 中的逐个事件/分段，以及 clip.srt 中对应字幕：\n");
    prompt_text.push_str(&context);
    prompt_text.push_str("\n请按上述事件标题规则生成封面标题。");
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

pub(super) async fn polish_cover_config(State(_state): State<Arc<AppState>>, Json(body): Json<serde_json::Value>) -> ApiResult {
    let dir = body.get("dir").and_then(|v| v.as_str()).ok_or_else(|| err(StatusCode::BAD_REQUEST, "缺少 dir"))?;
    let path = validated_project_dir(dir)?;
    if !path.join("polish.toml").exists() { return Err(err(StatusCode::BAD_REQUEST, "未初始化 polish")); }
    // 整个 read-modify-write 期间持锁：封面设计保存与时间轴保存会并发写同一份 polish.toml
    let _guard = vtrim_core::filelock::lock_path(&path.join("polish.toml")).await;
    let mut cfg = vtrim_polish::project::load_project(&path).map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("加载失败: {e}")))?;
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
        if let Some(v) = cover.get("stroke").and_then(|v| v.as_u64()) { cfg.cover.stroke = v as u32; }
        if let Some(v) = cover.get("hl_color").and_then(|v| v.as_str()) { cfg.cover.hl_color = v.to_string(); }
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
            cfg.cover.bg_color = vtrim_polish::cover::sanitize_hex_color(v, "#000000");
        }
        if let Some(v) = cover.get("blur_scale").and_then(|v| v.as_f64()) {
            cfg.cover.blur_scale = v.clamp(
                vtrim_polish::cover::COVER_BLUR_SCALE_MIN,
                vtrim_polish::cover::COVER_BLUR_SCALE_MAX,
            );
        }
        if let Some(v) = cover.get("effects").and_then(|v| v.as_array()) {
            cfg.cover.effects = v.iter().filter_map(|e| {
                let kind = e.get("kind").and_then(|x| x.as_str())?.to_string();
                let amount = e.get("amount").and_then(|x| x.as_f64()).unwrap_or(0.3);
                let x = e.get("x").and_then(|x| x.as_f64()).unwrap_or(50.0);
                let y = e.get("y").and_then(|x| x.as_f64()).unwrap_or(50.0);
                let color = e.get("color").and_then(|x| x.as_str()).unwrap_or("#000000").to_string();
                let text = e.get("text").and_then(|x| x.as_str()).unwrap_or("").to_string();
                Some(vtrim_polish::project::CoverEffect { kind, amount, x, y, color, text })
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
pub(super) async fn polish_silence(State(_state): State<Arc<AppState>>, Json(body): Json<serde_json::Value>) -> ApiResult {
    let dir = body.get("dir").and_then(|v| v.as_str()).ok_or_else(|| err(StatusCode::BAD_REQUEST, "缺少 dir"))?;
    let action = body.get("action").and_then(|v| v.as_str()).unwrap_or("apply");
    let path = validated_project_dir(dir)?;
    if !path.join("polish.toml").exists() { return Err(err(StatusCode::BAD_REQUEST, "未初始化 polish")); }
    // 去静音要跑 ffmpeg silencedetect（整个视频）+ 读改写 polish.toml，全是同步重活：
    // 锁在 await 里持有，活儿下 blocking 池（`with_path_lock` 的闭包是在当前线程跑的）
    let lock_file = path.join("polish.toml");
    let _guard = vtrim_core::filelock::lock_path(&lock_file).await;
    let action = action.to_string();
    blocking(move || match action.as_str() {
        "clear" => {
            let removed = vtrim_polish::project::clear_silence(&path)
                .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("移除静音屏蔽失败: {e:#}")))?;
            ok(serde_json::json!({ "ok": true, "applied": false, "removed": removed, "total": vtrim_polish::project::mute_range_count(&path) }))
        }
        "apply" => {
            let applied = vtrim_polish::project::apply_silence(&path)
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
pub(super) async fn polish_select_version(State(state): State<Arc<AppState>>, Json(body): Json<serde_json::Value>) -> ApiResult {
    let version = body.get("version").and_then(|v| v.as_str()).unwrap_or("original");
    let action = if version == "silent" { "apply" } else { "clear" };
    let mut forwarded = body.clone();
    forwarded["action"] = serde_json::json!(action);
    polish_silence(State(state), Json(forwarded)).await
}

/// 取（或启动）该工程目录的预览服务端口，按 `dir|kind` 复用。
///
/// `RenderServer` 起的监听无法关闭：旧实现每次 `/polish/preview`、
/// `/polish/cover-preview` 都新起一个，端口与后台任务会无限泄漏。
/// 服务端在每次请求时按磁盘配置重建 HTML，所以复用不会端出过期内容。
pub(super) async fn preview_port_for(state: &AppState, dir: &Path, kind: &str) -> anyhow::Result<u16> {
    let key = format!("{}|{kind}", dir.display());
    let cached = |key: &str| -> Option<u16> {
        state
            .preview_servers
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(key)
            .copied()
    };
    if let Some(port) = cached(&key) {
        return Ok(port);
    }
    // 同目录串行：并发首访不要同时起两个监听
    let _guard = vtrim_core::filelock::lock_path(&dir.join(format!(".vtrim-preview-{kind}"))).await;
    if let Some(port) = cached(&key) {
        return Ok(port);
    }
    let mut server = vtrim_polish::render::server::RenderServer::new(dir.to_path_buf());
    let port = if kind == "cover" {
        server.start_cover().await?
    } else {
        server.start().await?
    };
    state
        .preview_servers
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(key, port);
    Ok(port)
}

pub(super) async fn polish_preview(State(state): State<Arc<AppState>>, Json(body): Json<serde_json::Value>) -> ApiResult {
    let dir = body.get("dir").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let path = validated_project_dir(dir)?;
    if !path.join("polish.toml").exists() {
        return Err(err(StatusCode::BAD_REQUEST, "目录未初始化 polish"));
    }
    // 启动预览服务（不阻塞，返回 URL 供前端 iframe）
    let config = vtrim_polish::project::load_project(&path).map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("加载失败: {e}")))?;
    if !config.processed {
        // 隐式 process 只在这个片段目录空闲时才跑：**别的**片段/项目的任务不该挡住它
        // （以前只要有任何任务在跑就跳过），同目录有任务才真的会撞。
        if state
            .jobs
            .busy_with(&crate::scope_key::polish_keys(None, &path))
            .await
            .is_some()
        {
            tracing::info!("该片段有任务在运行，跳过预览前的隐式 process: {}", path.display());
        } else {
            let _ = vtrim_polish::process::cmd_process(&path).await;
        }
    }
    vtrim_polish::process::ensure_assets(&path).await;
    let port = preview_port_for(&state, &path, "video")
        .await
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("预览启动失败: {e:#}")))?;
    let video_url = format!("http://127.0.0.1:{}/", port);
    // 前端需要通过 API 代理访问本机 127.0.0.1 随机端口；这里直接返回绝对 URL（浏览器同源可访问）
    ok(serde_json::json!({ "ok": true, "video_url": video_url, "cover_url": video_url }))
}

pub(super) async fn polish_cover_background(State(_state): State<Arc<AppState>>, Json(body): Json<serde_json::Value>) -> ApiResult {
    let dir = body.get("dir").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let path = validated_project_dir(dir)?;
    if !path.join("polish.toml").exists() {
        return Err(err(StatusCode::BAD_REQUEST, "目录未初始化 polish"));
    }
    if let Some(data) = body.get("image_data").and_then(|v| v.as_str()) {
        // 与其它 polish.toml 写入串行化
        let _guard = vtrim_core::filelock::lock_path(&path.join("polish.toml")).await;
        use base64::Engine as _;
        let bytes = base64::engine::general_purpose::STANDARD.decode(data)
            .map_err(|e| err(StatusCode::BAD_REQUEST, format!("图片解码失败: {e}")))?;
        if bytes.len() > 32 * 1024 * 1024 {
            return Err(err(StatusCode::BAD_REQUEST, "背景图过大（上限 32MB）"));
        }
        // ⚠️ 写**背景层自己的文件** `_cover_bg.jpg`：封面三层 = 文字 / 关键帧(_cover_frame.jpg) / 背景，
        // 导入背景不能把关键帧那层盖掉（用户报过"又把关键帧那一层给我删了"）。
        std::fs::write(path.join(vtrim_polish::cover::COVER_BG_IMG), bytes)
            .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("写入失败: {e}")))?;
        let mut cfg = vtrim_polish::project::load_project(&path)
            .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("加载失败: {e}")))?;
        // 导入即切到「自定义图片」背景；关键帧与 cover.image 都不动
        cfg.cover.bg_mode = "image".into();
        let raw = cfg.to_toml_commented().map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("序列化失败: {e}")))?;
        std::fs::write(path.join("polish.toml"), raw).map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("写入失败: {e}")))?;
        return ok(serde_json::json!({ "ok": true, "bg_mode": "image" }));
    }
    if let Some(src) = body.get("source").and_then(|v| v.as_str()) {
        // 与其它 polish.toml 写入串行化
        let _guard = vtrim_core::filelock::lock_path(&path.join("polish.toml")).await;
        let sp = PathBuf::from(src);
        if !sp.exists() { return Err(err(StatusCode::BAD_REQUEST, "背景文件不存在")); }
        // 同上：走背景层的文件，别碰关键帧
        std::fs::copy(&sp, path.join(vtrim_polish::cover::COVER_BG_IMG))
            .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("复制失败: {e}")))?;
        let mut cfg = vtrim_polish::project::load_project(&path)
            .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("加载失败: {e}")))?;
        cfg.cover.bg_mode = "image".into();
        let raw = cfg.to_toml_commented().map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("序列化失败: {e}")))?;
        std::fs::write(path.join("polish.toml"), raw).map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("写入失败: {e}")))?;
        return ok(serde_json::json!({ "ok": true }));
    }
    Err(err(StatusCode::BAD_REQUEST, "需要 image_data 或 source"))
}

pub(super) async fn polish_cover_frame(State(_state): State<Arc<AppState>>, Json(body): Json<serde_json::Value>) -> ApiResult {
    let dir = body.get("dir").and_then(|v| v.as_str()).ok_or_else(|| err(StatusCode::BAD_REQUEST, "缺少 dir"))?;
    let path = validated_project_dir(dir)?;
    if !path.join("polish.toml").exists() {
        return Err(err(StatusCode::BAD_REQUEST, "目录未初始化 polish"));
    }
    // 与其它 polish.toml 写入串行化（本函数最后会写回 cover.image）；
    // 截帧要跑 ffmpeg（网络盘上数百 ms 到数秒）→ 整段下 blocking 池，别占 worker
    let _guard = vtrim_core::filelock::lock_path(&path.join("polish.toml")).await;
    let want_time = body
        .get("time")
        .and_then(|v| v.as_f64())
        .or_else(|| body.get("sec").and_then(|v| v.as_f64()));
    let time = blocking(move || {
        let config = vtrim_polish::project::load_project(&path)
            .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("加载失败: {e}")))?;
        let resolved = vtrim_polish::render::resolve_paths(&path, &config);
        let duration = config.segments.last().map(|s| s.end).unwrap_or(0.0);
        let time = want_time.unwrap_or(duration * config.cover.frame_pos).max(0.1);
        let frame_out = std::process::Command::new("ffmpeg")
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
        let mut cfg = vtrim_polish::project::load_project(&path)
            .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("加载失败: {e}")))?;
        cfg.cover.image = String::new();
        let raw = cfg.to_toml_commented().map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("序列化失败: {e}")))?;
        std::fs::write(path.join("polish.toml"), raw).map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("写入失败: {e}")))?;
        Ok(time)
    })
    .await?;
    ok(serde_json::json!({ "ok": true, "time": time }))
}

pub(super) async fn polish_cover_preview(State(state): State<Arc<AppState>>, Json(body): Json<serde_json::Value>) -> ApiResult {
    let dir = body.get("dir").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let path = validated_project_dir(dir)?;
    if !path.join("polish.toml").exists() {
        return Err(err(StatusCode::BAD_REQUEST, "目录未初始化 polish"));
    }
    // 缺关键帧时现场截一帧（ffmpeg，可能数百 ms~数秒）→ 整段下 blocking 池
    let cover_path = path.clone();
    let got_frame = blocking(move || {
        let config = vtrim_polish::project::load_project(&cover_path)
            .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("加载失败: {e}")))?;
        let resolved = vtrim_polish::render::resolve_paths(&cover_path, &config);
        let cover_image = cover_path.join("_cover_frame.jpg");
        if !cover_image.exists() {
            let duration = config.segments.last().map(|s| s.end).unwrap_or(0.0);
            let frame_time = (duration * config.cover.frame_pos).max(0.1);
            let frame_out = std::process::Command::new("ffmpeg")
                .args(["-ss", &frame_time.to_string(), "-i", &resolved.video,
                       "-vframes", "1", "-f", "image2pipe", "-vcodec", "mjpeg", "-q:v", "3", "-"])
                .stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::null())
                .output().map(|o| o.stdout);
            if let Ok(frame_data) = frame_out {
                if !frame_data.is_empty() {
                    std::fs::write(&cover_image, frame_data).ok();
                }
            }
        }
        Ok(cover_image.exists())
    })
    .await?;
    if !got_frame {
        return Err(err(StatusCode::BAD_REQUEST, "无法截取封面帧"));
    }
    // 封面服务按 dir|cover 复用；HTML 每次请求按当前 polish.toml 重建，改完设计刷新即可
    let port = preview_port_for(&state, &path, "cover")
        .await
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("封面预览启动失败: {e:#}")))?;
    let url = format!("http://127.0.0.1:{}/", port);
    ok(serde_json::json!({ "ok": true, "cover_url": url }))
}

/// 读精修字幕（`data/subtitles.json` 或 `clip.srt`）：项目目录上的同步 IO，下 blocking 池。
pub(super) async fn polish_subtitles_get(State(state): State<Arc<AppState>>, Query(q): Query<std::collections::HashMap<String,String>>) -> ApiResult {
    blocking(move || block_on_here(polish_subtitles_get_impl(State(state), Query(q)))).await
}

pub(super) async fn polish_subtitles_get_impl(
    State(_state): State<Arc<AppState>>,
    Query(q): Query<std::collections::HashMap<String,String>>,
) -> ApiResult {
    let dir = q.get("dir").ok_or_else(|| err(StatusCode::BAD_REQUEST, "缺少 dir"))?;
    let path = validated_project_dir(dir)?;
    if !path.join("polish.toml").exists() {
        return Err(err(StatusCode::BAD_REQUEST, "目录未初始化 polish"));
    }
    let json_path = path.join("data").join("subtitles.json");
    if json_path.exists() {
        let raw = std::fs::read_to_string(&json_path).map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("读取失败: {e}")))?;
        let v: serde_json::Value = serde_json::from_str(&raw).unwrap_or(serde_json::Value::Null);
        let segments = if let Some(arr) = v.as_array() {
            arr.clone()
        } else if let Some(arr) = v.get("segments").and_then(|x| x.as_array()) {
            arr.clone()
        } else {
            Vec::new()
        };
        return ok(serde_json::json!({ "segments": segments }));
    }
    let srt_path = path.join("clip.srt");
    if srt_path.exists() {
        let raw = std::fs::read_to_string(&srt_path).map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("读取失败: {e}")))?;
        let t = vtrim_core::srt::parse_srt(&raw).map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("解析失败: {e}")))?;
        return ok(serde_json::json!({ "segments": t.segments }));
    }
    Err(err(StatusCode::NOT_FOUND, "暂无字幕文件"))
}

pub(super) async fn polish_subtitles_put(State(_state): State<Arc<AppState>>, Json(body): Json<serde_json::Value>) -> ApiResult {
    let dir = body.get("dir").and_then(|v| v.as_str()).ok_or_else(|| err(StatusCode::BAD_REQUEST, "缺少 dir"))?;
    let path = validated_project_dir(dir)?;
    if !path.join("polish.toml").exists() {
        return Err(err(StatusCode::BAD_REQUEST, "目录未初始化 polish"));
    }
    let segments = body.get("segments").cloned().ok_or_else(|| err(StatusCode::BAD_REQUEST, "缺少 segments"))?;
    let mut segs: Vec<vtrim_core::types::TranscriptSegment> = segments.as_array()
        .ok_or_else(|| err(StatusCode::BAD_REQUEST, "segments 必须是数组"))?
        .iter().map(|v| {
            let start = v.get("start").and_then(|x| x.as_f64()).unwrap_or(0.0);
            let end = v.get("end").and_then(|x| x.as_f64()).unwrap_or(start + 1.0);
            let text = v.get("text").and_then(|x| x.as_str()).unwrap_or("").to_string();
            let confidence = v.get("confidence").and_then(|x| x.as_f64()).unwrap_or(1.0);
            vtrim_core::types::TranscriptSegment { start, end, text, confidence }
        }).collect();
    // **按开始时间排序后再落盘**：字幕轨道必须有序 —— 去静音按相邻间隔算静音、
    // 预览按时间二分取当前字幕都依赖它。编辑器「拆分」曾经把后半段追加到数组末尾
    // （`TimelineEditor.splitSelected`，已修），存过的那种文件是乱序的；
    // 这里顺手归一化，用户再存一次就自愈（老数据不用手工修）。
    segs.sort_by(|a, b| a.start.partial_cmp(&b.start).unwrap_or(std::cmp::Ordering::Equal));
    let data_dir = path.join("data");
    std::fs::create_dir_all(&data_dir).map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("创建目录失败: {e}")))?;
    // 写回 data/subtitles.json，保持 polish 使用的列表结构
    let out: Vec<serde_json::Value> = segs.iter().enumerate().map(|(i, seg)| {
        serde_json::json!({
            "index": i + 1,
            "start": seg.start,
            "end": seg.end,
            "text": seg.text,
            "html": seg.text,
        })
    }).collect();
    std::fs::write(data_dir.join("subtitles.json"), serde_json::to_string_pretty(&out).unwrap_or_default())
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("写入失败: {e}")))?;
    // 同步写回 clip.srt，方便外部工具读取
    let transcript = vtrim_core::types::Transcript {
        segments: segs,
        language: body.get("language").and_then(|l| l.as_str()).unwrap_or("zh").to_string(),
        asr_metadata: None,
    };
    vtrim_core::srt::write_srt(&transcript, &path.join("clip.srt"))
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("写 SRT 失败: {e}")))?;
    ok(serde_json::json!({ "ok": true, "saved": out.len() }))
}

/// 响度曲线（带缓存）：同一源视频只测一次，结果存 `data/loudness.json`。
///
/// 实现已收敛到 `vtrim_polish::render::measure_loudness_curve_cached`（出片页与混音共用同一份）。
pub(super) fn loudness_curve_cached(project_dir: &Path, video_path: &Path) -> serde_json::Value {
    let curve = vtrim_polish::render::measure_loudness_curve_cached(project_dir, video_path);
    let points: Vec<serde_json::Value> = curve
        .iter()
        .map(|(t, db)| serde_json::json!({ "t": t, "db": db }))
        .collect();
    serde_json::json!(points)
}

/// 读一个精修工程（`polish.toml` + 贴纸映射 + 探测视频尺寸 + 响度曲线）——
/// 全是同步 IO / ffprobe，工程又在项目目录里（可能网络盘）。整段下 blocking 池：
/// 精修页每次加载与每次保存后都会打这个接口。
pub(super) async fn polish_project_get(State(state): State<Arc<AppState>>, Query(q): Query<std::collections::HashMap<String,String>>) -> ApiResult {
    blocking(move || block_on_here(polish_project_get_impl(State(state), Query(q)))).await
}

pub(super) async fn polish_project_get_impl(
    State(_state): State<Arc<AppState>>,
    Query(q): Query<std::collections::HashMap<String,String>>,
) -> ApiResult {
    let dir = q.get("dir").ok_or_else(|| err(StatusCode::BAD_REQUEST, "缺少 dir"))?;
    let path = validated_project_dir(dir)?;
    let cfg = vtrim_polish::project::load_project(&path).map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("加载失败: {e}")))?;
    let mut value = serde_json::to_value(&cfg).unwrap_or_default();
    // preview=1: 附加预览渲染所需的计算字段（sticker_map / 竖屏检测 / 响度曲线）
    if q.get("preview").map(|v| v == "1").unwrap_or(false) {
        if let Some(obj) = value.as_object_mut() {
            obj.insert("_assets".into(), serde_json::json!({
                "sticker_map": vtrim_polish::render::server::build_sticker_map(&path),
            }));
            let video_path = path.join(&cfg.source.video);
            // 画布契约与出片读的是**同一份推导**（`ProjectFile::canvas_contract`）。
            // 这里以前自己写了一份「竖屏检测」，与 `render/server.rs` 各一份 —— 改画布方向会不同步。
            let src_size = vtrim_polish::render::get_video_size(&video_path.to_string_lossy());
            let contract = cfg.canvas_contract(src_size);
            obj.insert(
                "vertical".into(),
                serde_json::json!(src_size.map(|(w, h)| h > w).unwrap_or(false)),
            );
            obj.insert(
                "vertical_position".into(),
                contract["stage"]["position"].clone(),
            );
            obj.insert("_canvas".into(), contract);
            // 最近一次 LLM 生成的结果（哪段成/哪段失败），编辑器按它给分段上警示色。
            // 只是展示用：读不到就当没有，绝不影响加载。
            if let Some(st) = vtrim_polish::llm::status::load(&path) {
                obj.insert(
                    "_llm_status".into(),
                    serde_json::to_value(st).unwrap_or(serde_json::Value::Null),
                );
            }
            if video_path.exists() {
                obj.insert("loudness_curve".into(), loudness_curve_cached(&path, &video_path));
            }
        }
    }
    ok(value)
}

/// 单帧真渲染预览：用常驻 wgpu 后端渲染 dir 项目在 t 时刻的合成画面，返回 JPEG。
#[cfg(feature = "wgpu-render")]
pub(super) async fn polish_render_frame(
    State(state): State<Arc<AppState>>,
    Json(body): Json<serde_json::Value>,
) -> Response {
    use vtrim_polish::render::backend::FrameRenderer as _;

    let Some(dir) = body.get("dir").and_then(|v| v.as_str()).map(PathBuf::from) else {
        return (StatusCode::BAD_REQUEST, Json(serde_json::json!({"ok": false, "error": "缺少 dir"}))).into_response();
    };
    let t = body.get("t").and_then(|v| v.as_f64()).unwrap_or(0.0).max(0.0);
    let fps = 30usize;

    let cfg = {
        let t0 = std::time::Instant::now();
        let c = vtrim_polish::project::load_project(&dir);
        tracing::info!("render-frame: config load {}ms", t0.elapsed().as_millis());
        match c {
            Ok(c) => c,
            Err(e) => {
                return (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({"ok": false, "error": format!("加载失败: {e:#}")}))).into_response();
            }
        }
    };
    let video_path = dir.join(&cfg.source.video);

    let result = tokio::task::spawn_blocking(move || -> anyhow::Result<Vec<u8>> {
        let mut slot = state.preview.0.lock().expect("preview slot poisoned");
        // 目录切换 / 首次请求：重建后端（wgpu 设备常驻复用）
        let rebuild = match slot.as_ref() {
            Some(b) => b.dir != dir,
            None => true,
        };
        if rebuild {
            let backend = vtrim_polish::render::native::WgpuBackend::new(&dir, &cfg)?;
            *slot = Some(crate::model::PreviewBackend {
                dir: dir.clone(),
                backend,
                last_t: -1.0,
                last_jpeg: None,
                video_size: None,
            });
        }
        let pb = slot.as_mut().unwrap();
        // 时间取整到帧网格，同帧命中缓存
        let round_t = (t * fps as f64).round() / fps as f64;
        if (pb.last_t - round_t).abs() < 1e-9 && pb.last_jpeg.is_some() {
            return Ok(pb.last_jpeg.clone().unwrap());
        }
        let mut ctx = vtrim_polish::render::RenderContext::new(dir.clone(), cfg.clone());
        ctx.fps = fps;
        // ffprobe 结果缓存（省 ~50ms/请求）
        if pb.video_size.is_none() {
            pb.video_size = vtrim_polish::render::get_video_size(&video_path.to_string_lossy());
        }
        let t_vs = std::time::Instant::now();
        if let Some((vw, vh)) = pb.video_size {
            if vh > vw {
                ctx.vertical = true;
            }
        }
        tracing::info!("render-frame: video size {}ms", t_vs.elapsed().as_millis());
        pb.backend.prepare(&ctx)?;
        let t_ex = std::time::Instant::now();
        let mut frames = vtrim_polish::render::frame::extract_chunk_frames(
            &video_path.to_string_lossy(),
            round_t,
            round_t + 1.0 / fps as f64,
            fps,
            true,
        )?;
        frames.truncate(1);
        if frames.is_empty() {
            anyhow::bail!("抽帧为空");
        }
        tracing::info!("render-frame: extract {}ms ({} frames)", t_ex.elapsed().as_millis(), frames.len());
        let t_rd = std::time::Instant::now();
        let jpeg = pb.backend.render_chunk(&frames)?.remove(0);
        tracing::info!("render-frame: render {}ms", t_rd.elapsed().as_millis());
        pb.last_t = round_t;
        pb.last_jpeg = Some(jpeg.clone());
        Ok(jpeg)
    })
    .await;

    match result {
        Ok(Ok(jpeg)) => ([(axum::http::header::CONTENT_TYPE, "image/jpeg")], jpeg).into_response(),
        Ok(Err(e)) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"ok": false, "error": format!("{e:#}")})),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"ok": false, "error": format!("任务失败: {e}")})),
        )
            .into_response(),
    }
}

/// feature 未启用时的占位响应。
#[cfg(not(feature = "wgpu-render"))]
pub(super) async fn polish_render_frame(
    State(state): State<Arc<AppState>>,
    Json(body): Json<serde_json::Value>,
) -> Response {
    let _ = (state, body);
    (
        StatusCode::NOT_IMPLEMENTED,
        Json(serde_json::json!({"ok": false, "error": "wgpu-render feature 未启用（cargo build --features wgpu-render）"})),
    )
        .into_response()
}


/// 字体库全量（**只读**，非 admin 也能看）：字幕字体 / 封面字体的下拉数据源。
///
/// 库位置 = `<程序目录>/resources/fonts/`（`vtrim_core::path::font_dir`）。**以磁盘为准**：
/// 往里丢一个 `.ttf` / `.otf` / `.ttc` 就会出现在列表里，没有索引文件要维护。
/// `id` = 文件名去扩展名（配置里存的就是它）；`family` 是从字体 `name` 表读出来的展示名。
/// 扫描要读字体文件解析字族名，所以整段走 `blocking`（同步 IO 不能留在 worker 上）。
///
/// `root`（服务器上的绝对路径）**只给 admin**：非 admin 拿它没用（也丢不进文件），
/// 而它泄露的是服务器的安装路径 —— 与 `/api/admin/usage` 的理由同一条。
pub(super) async fn polish_fonts(Extension(user): Extension<crate::auth::CurrentUser>) -> ApiResult {
    let is_admin = user.is_admin();
    let payload = blocking(move || {
        let root = vtrim_core::path::font_dir();
        let fonts: Vec<serde_json::Value> = vtrim_core::fonts::scan()
            .into_iter()
            .map(|f| {
                serde_json::json!({
                    "id": f.id,
                    "file": f.file,
                    "family": f.family,
                    "bytes": f.bytes,
                })
            })
            .collect();
        Ok(serde_json::json!({
            "ok": true,
            // 出 JSON 的路径必须过 display（去掉 `\\?\` 逐字前缀），否则前端拼路径会炸
            "root": if is_admin { vtrim_core::path::display(&root) } else { String::new() },
            "fonts": fonts,
        }))
    })
    .await?;
    ok(payload)
}

/// 字体文件端点（wasm 预览 fetch 字体字节用）：项目字体候选 -> 内置资源。
pub(super) async fn polish_font(Query(q): Query<std::collections::HashMap<String, String>>) -> Response {
    let name = q.get("name").cloned().unwrap_or_default();
    let dir = q.get("dir").map(std::path::PathBuf::from).unwrap_or_default();
    let mut candidates: Vec<PathBuf> = Vec::new();
    if !name.trim().is_empty() {
        // 只取文件名部分：name 来自查询参数，`..\..\x` 这类路径不能让字体接口越界读文件
        let base = Path::new(name.trim())
            .file_name()
            .map(PathBuf::from)
            .unwrap_or_default();
        if !base.as_os_str().is_empty() {
            for d in [
                dir.join("assets").join("fonts"),
                dir.join("fonts"),
                dir.clone(),
            ] {
                candidates.push(d.join(&base).with_extension("ttf"));
                candidates.push(d.join(&base).with_extension("otf"));
                candidates.push(d.join(&base).with_extension("ttc"));
            }
            // 程序级字体库（resources/fonts/）：`name` 就是字体 id（文件名去扩展名）。
            // 放在项目候选之后 —— 项目里自带的字体优先，库只是"别处都没有"时的来源。
            if let Some(p) = vtrim_core::fonts::resolve_file(name.trim()) {
                candidates.push(p);
            }
            candidates.push(Path::new("C:/Windows/Fonts").join(&base).with_extension("ttf"));
        }
    }
    candidates.push(std::path::Path::new("C:/Windows/Fonts/msyh.ttc").to_path_buf());
    // ⚠️ 这里**不再有内置字体文件**（发行包也刻意不带，见 README「字体」与 THIRD-PARTY-NOTICES.md）：
    // 候选链到系统字体（微软雅黑）就是最后一档兜底了。
    // 想用圆润可爱的标题风格，请**用户自行安装**乐米波波体（LemiBoBoTi，杭州乐米网络科技，
    // 官方授权地址 https://font.leminet.cn/）——它的授权是「免费商用，但**不包括嵌入式用途**」，
    // 随软件分发/内嵌字体文件必须另行授权，所以别再把 ttf 塞回仓库或发布包。
    for c in &candidates {
        if c.exists() {
            match std::fs::read(c) {
                Ok(bytes) => {
                    return ([(axum::http::header::CONTENT_TYPE, font_content_type(c))], bytes)
                        .into_response();
                }
                Err(_) => continue,
            }
        }
    }
    StatusCode::NOT_FOUND.into_response()
}

/// 字体文件的 Content-Type 按扩展名给：以前恒 `font/ttf`，`.otf` / `.ttc` 会被标错
/// （浏览器按 MIME 决定要不要当字体用，别赌它容错）。
pub(super) fn font_content_type(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .as_deref()
    {
        Some("otf") => "font/otf",
        Some("ttc") => "font/collection",
        Some("woff") => "font/woff",
        Some("woff2") => "font/woff2",
        _ => "font/ttf",
    }
}

pub(super) async fn polish_project_put(State(_state): State<Arc<AppState>>, Json(body): Json<serde_json::Value>) -> ApiResult {
    let dir = body.get("dir").and_then(|v| v.as_str()).ok_or_else(|| err(StatusCode::BAD_REQUEST, "缺少 dir"))?;
    let path = validated_project_dir(dir)?;
    // polish.toml 的 read-modify-write：时间轴保存 / 屏蔽区间 / 贴纸配置可能同时提交，
    // 无锁时后写者会覆盖先写者（前端确实会并发发多个 PUT）。
    // 锁在 await 里持有，读改写下 blocking 池
    let _guard = vtrim_core::filelock::lock_path(&path.join("polish.toml")).await;
    blocking(move || polish_project_put_locked(&path, &body)).await
}

/// 同步实现（调用方保证持有 polish.toml 的进程内锁）。
pub(super) fn polish_project_put_locked(dir: &Path, body: &serde_json::Value) -> ApiResult {
    let mut cfg = vtrim_polish::project::load_project(dir).map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("加载失败: {e}")))?;
    if let Some(segments) = body.get("segments") {
        // 老的第三方调用方 / 手写 JSON 里的 `duration` 在这里升级成 `end`（与 polish.toml 同一条路）
        let mut up = segments.clone();
        vtrim_polish::project::upgrade_event_times(&mut up);
        let segs = serde_json::from_value(up).map_err(|e| err(StatusCode::BAD_REQUEST, format!("segments 解析失败: {e}")))?;
        cfg.segments = segs;
    }
    if let Some(ranges) = body.get("mute_ranges") {
        let rs = serde_json::from_value(ranges.clone()).map_err(|e| err(StatusCode::BAD_REQUEST, format!("mute_ranges 解析失败: {e}")))?;
        cfg.mute_ranges = rs;
    }
    if let Some(layout) = body.get("layout") {
        let lay: vtrim_polish::project::Layout = serde_json::from_value(layout.clone()).map_err(|e| err(StatusCode::BAD_REQUEST, format!("layout 解析失败: {e}")))?;
        cfg.layout = lay;
    }
    if let Some(ss) = body.get("sticker_source") {
        let list: Vec<String> = serde_json::from_value(ss.clone()).map_err(|e| err(StatusCode::BAD_REQUEST, format!("sticker_source 解析失败: {e}")))?;
        cfg.sticker_source = list;
    }
    // 字幕字体：值 = 字体库里的 id（文件名去扩展名），空串 = 回到系统字体栈。
    // 只影响渲染与预览（process 的贴纸/运镜重算不读它），所以**保存即生效、不用重跑任务**。
    if let Some(f) = body.get("subtitle_font").and_then(|v| v.as_str()) {
        cfg.style.subtitle.font = f.trim().to_string();
    }
    // 输出画布 / 画面构成 / 贴纸落点：整块提交（前端给全量字段，缺键走 serde 默认）。
    // ⚠️ 这里是**逐字段白名单**：没列进来的键保存时不报错、但静默存不进去。
    // `sticker` 只落点策略（来源仍是根级 `sticker_source`，见上一段）。
    if let Some(v) = body.get("canvas") {
        cfg.canvas = serde_json::from_value(v.clone())
            .map_err(|e| err(StatusCode::BAD_REQUEST, format!("canvas 解析失败: {e}")))?;
    }
    if let Some(v) = body.get("stage") {
        cfg.stage = serde_json::from_value(v.clone())
            .map_err(|e| err(StatusCode::BAD_REQUEST, format!("stage 解析失败: {e}")))?;
    }
    if let Some(v) = body.get("sticker") {
        cfg.sticker = serde_json::from_value(v.clone())
            .map_err(|e| err(StatusCode::BAD_REQUEST, format!("sticker 解析失败: {e}")))?;
    }
    let raw = cfg.to_toml_commented().map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("序列化失败: {e}")))?;
    std::fs::write(dir.join("polish.toml"), raw).map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("保存失败: {e}")))?;
    ok(serde_json::json!({ "ok": true }))
}

/// 可用的贴图来源：内置 emoji 码点池 + 全局贴纸库（exe 同级 stickers/）的包名与 tags。
/// LLM 生成时按 sticker_source 过滤可用的贴纸池。
pub(super) async fn polish_sticker_sources() -> ApiResult {
    use vtrim_polish::assets::sticker_index::{load_package_meta, load_sticker_index};
    let root = vtrim_core::path::sticker_dir();
    let index = load_sticker_index(&root);
    let mut out = vec![serde_json::json!({
        "name": "emoji",
        "builtin": true,
        "description": "内置 emoji 码点池（按情绪匹配）",
        "tags": [],
    })];
    for name in &index.packages {
        let meta = load_package_meta(&root, name);
        let (desc, tags) = match meta {
            Some(m) => (m.description, m.tags.iter().map(|t| t.tag.clone()).collect::<Vec<_>>()),
            None => (String::new(), Vec::new()),
        };
        out.push(serde_json::json!({
            "name": name,
            "builtin": false,
            "description": desc,
            "tags": tags,
        }));
    }
    ok(serde_json::json!({ "sources": out }))
}

/// 贴纸库全量（**只读**，非 admin 也能看）：编辑器里「keyword 一级 → 贴纸二级」选择用。
///
/// 以磁盘为准（`scan_packages`），每个包给齐 `tags[].files`（`<包名>.toml` 的反向索引）
/// 与全部文件名 —— 未构建的包也给文件，只是没有 tag（前端把它们平铺成一级条目）。
pub(super) async fn polish_sticker_library() -> ApiResult {
    use vtrim_polish::assets::sticker_index::{
        list_package_files, load_package_meta, load_sticker_index, scan_packages,
    };
    let root = vtrim_core::path::sticker_dir();
    let index = load_sticker_index(&root);
    let packs: Vec<serde_json::Value> = scan_packages(&root)
        .into_iter()
        .map(|pkg| {
            let meta = load_package_meta(&root, &pkg.name);
            let files = list_package_files(&root.join(&pkg.name));
            let tags: Vec<serde_json::Value> = meta
                .as_ref()
                .map(|m| {
                    m.tags
                        .iter()
                        .map(|t| serde_json::json!({ "tag": t.tag, "files": t.files }))
                        .collect()
                })
                .unwrap_or_default();
            serde_json::json!({
                "name": pkg.name,
                "built": meta.is_some(),
                "registered": index.packages.iter().any(|n| n == &pkg.name),
                "description": meta.as_ref().map(|m| m.description.clone()).unwrap_or_default(),
                "tags": tags,
                "files": files,
            })
        })
        .collect();
    ok(serde_json::json!({ "ok": true, "packs": packs }))
}

/// 音效目录（只读）：编辑器里音效事件/特效配声的列表选择。
///
/// 数据来自编译进二进制的 `assets/sfx_catalog.json`；全局 `[sfx] blacklist`
/// 屏蔽掉的整族不列出来（选了也不会被解析）。
pub(super) async fn polish_sfx_catalog() -> ApiResult {
    use vtrim_polish::assets::sfx_catalog;
    let blacklist = vtrim_core::config::load_config_at(std::path::Path::new("."))
        .map(|c| c.sfx.blacklist)
        .unwrap_or_default();
    let entries: Vec<serde_json::Value> = sfx_catalog::all()
        .iter()
        .filter(|e| !sfx_catalog::is_blacklisted(&e.id, &blacklist))
        .map(|e| {
            serde_json::json!({
                "id": e.id,
                "label": e.label,
                "cat": e.cat,
                "tags": e.tags,
            })
        })
        .collect();
    ok(serde_json::json!({ "ok": true, "entries": entries }))
}

/// 贴纸库里的单张图（编辑器缩略图直连 `<img>`）；只读 + 路径安全校验，与 admin 版同款。
pub(super) async fn polish_sticker_file(
    axum::extract::Query(q): axum::extract::Query<std::collections::HashMap<String, String>>,
    headers: axum::http::HeaderMap,
) -> axum::response::Response {
    use vtrim_polish::assets::sticker_index::{is_safe_component, sanitize_pack_name};
    let root = vtrim_core::path::sticker_dir();
    let pack = q.get("pack").map(String::as_str).unwrap_or("");
    let name = q.get("name").map(String::as_str).unwrap_or("");
    let Some(pack) = sanitize_pack_name(pack) else {
        return crate::media::json_error(StatusCode::BAD_REQUEST, "包名不合法");
    };
    if !is_safe_component(name) {
        return crate::media::json_error(StatusCode::BAD_REQUEST, "文件名不合法");
    }
    let path = root.join(&pack).join(name);
    let real = path.canonicalize().unwrap_or_else(|_| path.clone());
    let root_real = root.canonicalize().unwrap_or_else(|_| root.clone());
    if !real.starts_with(&root_real) {
        return crate::media::json_error(StatusCode::NOT_FOUND, "文件不在贴纸库内");
    }
    let mime = mime_guess::from_path(&path).first_or_octet_stream().to_string();
    let range = headers
        .get(axum::http::header::RANGE)
        .and_then(|v| v.to_str().ok());
    crate::media::serve_file(&path, &mime, range).await
}







#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::test_support::*;


    use axum::body::Body;


/// 换字幕字体只改 `[style.subtitle] font` 一个字段：别的段（分段/屏蔽/封面）原样保留
/// —— 它是"保存即生效"的同步接口，写坏了会把整个片段的精修配置一起带走。
#[test]
fn polish_project_put_writes_subtitle_font_only() {
    let dir = std::env::temp_dir().join(format!("vtrim-font-put-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut cfg = vtrim_polish::project::ProjectFile::example();
    cfg.mute_ranges.push(vtrim_polish::project::MuteRange { start: 1.0, end: 2.0 });
    std::fs::write(dir.join("polish.toml"), cfg.to_toml_commented().unwrap()).unwrap();

    let body = serde_json::json!({ "subtitle_font": "simhei" });
    let _ = polish_project_put_locked(&dir, &body).expect("保存字幕字体");

    let after = vtrim_polish::project::load_project(&dir).unwrap();
    assert_eq!(after.style.subtitle.font, "simhei");
    assert_eq!(after.mute_ranges.len(), 1, "没提到的字段必须原样保留");

    // 空串 = 回到系统字体栈
    let _ = polish_project_put_locked(&dir, &serde_json::json!({ "subtitle_font": "" })).unwrap();
    assert_eq!(vtrim_polish::project::load_project(&dir).unwrap().style.subtitle.font, "");
    let _ = std::fs::remove_dir_all(&dir);
}

/// 画布方向 / 画面构成 / 贴纸落点要能真写进 `polish.toml`，且**不碰**没提到的段。
///
/// 回归：PUT 是**逐字段白名单**（见 `polish_project_put_locked`）——漏一个分支就是
/// "保存不报错、但值静默丢掉"，前端还会以为存上了。
#[test]
fn polish_project_put_writes_canvas_stage_sticker() {
    let dir = std::env::temp_dir().join(format!("vtrim-stage-put-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut cfg = vtrim_polish::project::ProjectFile::example();
    cfg.mute_ranges.push(vtrim_polish::project::MuteRange { start: 1.0, end: 2.0 });
    std::fs::write(dir.join("polish.toml"), cfg.to_toml_commented().unwrap()).unwrap();

    // 1) 只切画布方向：其它段原样保留
    let body = serde_json::json!({ "canvas": { "orientation": "portrait" } });
    let _ = polish_project_put_locked(&dir, &body).expect("保存画布方向");
    let after = vtrim_polish::project::load_project(&dir).unwrap();
    assert_eq!(after.canvas.size(), (1080, 1920));
    assert_eq!(after.mute_ranges.len(), 1, "没提到的字段必须原样保留");
    assert_eq!(after.stage.position, "auto", "没提交的段走默认，不能被顺手重置");

    // 2) 画面构成
    let body = serde_json::json!({ "stage": {
        "bg_mode": "color", "bg_color": "#112233", "blur_scale": 2.0,
        "position": "bottom", "scale": 1.25, "padding": 30,
    }});
    let _ = polish_project_put_locked(&dir, &body).expect("保存画面构成");
    let after = vtrim_polish::project::load_project(&dir).unwrap();
    assert_eq!(after.canvas.size(), (1080, 1920), "没提交 canvas 就不能被重置");
    let fit = after.resolve_stage(Some((1920, 1080)));
    assert_eq!(fit.axis, "y", "竖画布 × 横素材 = 上下留边");
    assert_eq!(fit.position, "bottom");
    assert_eq!(fit.bg, "color");
    assert_eq!(fit.bg_color, "#112233");
    assert!((fit.padding - 16.875).abs() < 1e-9, "30px × k(0.5625)");

    // 3) 贴纸落点
    let body = serde_json::json!({ "sticker": { "position_mode": "manual", "position": "20% 80%" } });
    let _ = polish_project_put_locked(&dir, &body).expect("保存贴纸落点");
    let after = vtrim_polish::project::load_project(&dir).unwrap();
    assert_eq!(after.sticker.position_mode, "manual");
    assert_eq!(after.sticker.position, "20% 80%");
    assert_eq!(after.stage.position, "bottom", "上一段必须还在");

    // 4) 落到 toml 文本上：段在、段注释在、根级 sticker_source 没被 [sticker] 顶掉
    let raw = std::fs::read_to_string(dir.join("polish.toml")).unwrap();
    for needle in [
        "[canvas]",
        "[stage]",
        "[sticker]",
        "orientation = \"portrait\"",
        "position_mode = \"manual\"",
        "# ===== 画面构成",
        "# ===== 贴纸落点",
        "sticker_source",
    ] {
        assert!(raw.contains(needle), "缺 {needle}:\n{raw}");
    }

    // 5) 非法值：文件照存（不篡改用户输入），但渲染侧取到的是安全值
    let body = serde_json::json!({ "stage": {
        "bg_mode": "frame", "bg_color": "url(evil)", "position": "top"
    }});
    let _ = polish_project_put_locked(&dir, &body).expect("保存非法值也不应报错");
    let after = vtrim_polish::project::load_project(&dir).unwrap();
    let fit = after.resolve_stage(Some((1920, 1080)));
    assert_eq!(fit.position, "top");
    assert_eq!(fit.bg, "blur", "未知 bg_mode 回落 auto → 补边时按模糊处理");
    assert_eq!(fit.bg_color, "#000000", "颜色必须净化，不能把 url(...) 带进模板 CSS");

    let _ = std::fs::remove_dir_all(&dir);
}

/// 手持摇摆（`Event::Camera::handheld`）随 `segments` 整体收下 ——
/// 运镜新字段走的是既有的 segments 通道（不是逐字段白名单里的新分支），
/// 但正因如此必须**断言它真的落到 toml 上**：反序列化若把未知字段静默丢掉，
/// PUT 依旧返回 ok、前端以为存上了（同一个坑见上一测试的注释）。
///
/// 同时钉住「老配置零迁移」：`handheld` 缺省 false 不落盘（`skip_serializing_if`），
/// 没开摇摆的事件在 toml 里不该多出 `handheld = false` 这行噪音。
#[test]
fn polish_project_put_roundtrips_handheld_sway() {
    let dir = std::env::temp_dir().join(format!("vtrim-handheld-put-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("polish.toml"),
        "[source]\nvideo = \"clip.mp4\"\nsubtitles = \"clip.srt\"\n",
    )
    .unwrap();

    let body = serde_json::json!({ "segments": [
        { "start": 0.0, "end": 30.0, "fx": "high", "note": "",
          "events": [
            { "type": "camera", "time": 1.0, "end": 3.0, "zoom": "近", "origin": "50% 50%", "handheld": true },
            { "type": "camera", "time": 5.0, "end": 7.0, "zoom": "中", "origin": "50% 50%" }
          ] }
    ]});
    let _ = polish_project_put_locked(&dir, &body).expect("保存运镜事件");

    // 1) 重新加载：值还在
    let after = vtrim_polish::project::load_project(&dir).unwrap();
    let flags: Vec<bool> = after.segments[0]
        .events
        .iter()
        .map(|ev| match ev {
            vtrim_polish::project::Event::Camera { handheld, .. } => *handheld,
            other => panic!("应当是运镜事件: {other:?}"),
        })
        .collect();
    assert_eq!(flags, vec![true, false]);

    // 2) 落盘文本：true 必须写出、缺省 false 不写
    let raw = std::fs::read_to_string(dir.join("polish.toml")).unwrap();
    assert!(raw.contains("handheld = true"), "手持摇摆没落到 toml:\n{raw}");
    assert!(!raw.contains("handheld = false"), "缺省值不该落盘:\n{raw}");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn project_dir_validation_rejects_empty_and_missing() {
    assert!(validated_project_dir("").is_err());
    assert!(validated_project_dir("   ").is_err());
    let missing = std::env::temp_dir().join("vtrim-dir-does-not-exist-xyz");
    assert!(validated_project_dir(&missing).is_err());
    assert!(validated_project_dir(std::env::temp_dir()).is_ok());
    assert!(validated_project_dir(std::env::temp_dir().to_string_lossy().to_string()).is_ok());
}

/// 字幕保存要**按开始时间排序**后再落盘（去静音/预览都依赖字幕有序）。
///
/// 回归：编辑器「拆分」曾把后半段 push 到数组末尾，存出来的字幕文件是乱序的 ——
/// 去静音按文件顺序取相邻间隔，于是判出 13 段/121s 的假静音（实测 clip-19，
/// 用户"去静音完全对不上，像用了别的素材"）。乱序的旧数据**再存一次就自愈**。
#[tokio::test]
async fn subtitle_save_normalizes_time_order() {
    let t = test_app().await;
    let dir = t.root.join("output").join("clips").join("clip-19");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("polish.toml"),
        "[source]\nvideo = \"clip.mp4\"\nsubtitles = \"clip.srt\"\n",
    )
    .unwrap();

    // 乱序：末尾那条其实是开头那段的「拆分后半段」（就是出问题的形状）
    let body = serde_json::json!({
        "dir": vtrim_core::path::display(&dir),
        "segments": [
            { "start": 0.0,  "end": 2.07,  "text": "前半" },
            { "start": 99.7, "end": 102.4, "text": "后面一句" },
            { "start": 2.07, "end": 5.79,  "text": "拆分出来的后半" },
        ],
    });
    let res = t
        .call(
            t.req("PUT", "/polish/subtitles")
                .header(axum::http::header::CONTENT_TYPE, "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await;
    assert_eq!(res.status(), StatusCode::OK);

    let saved: Vec<f64> = {
        let raw = std::fs::read_to_string(dir.join("data").join("subtitles.json")).unwrap();
        serde_json::from_str::<serde_json::Value>(&raw)
            .unwrap()
            .as_array()
            .unwrap()
            .iter()
            .map(|s| s["start"].as_f64().unwrap())
            .collect()
    };
    assert_eq!(saved, vec![0.0, 2.07, 99.7], "落盘顺序要按开始时间");

    // clip.srt 同步归一化（外部工具读它）
    let srt = std::fs::read_to_string(dir.join("clip.srt")).unwrap();
    let order: Vec<&str> = srt
        .lines()
        .filter(|l| l.contains("-->"))
        .map(|l| l.split_whitespace().next().unwrap())
        .collect();
    assert_eq!(order, vec!["00:00:00,000", "00:00:02,070", "00:01:39,700"], "{srt}");
}

/// 三个"路径类 body 字段"都不能把项目外的文件拉进来（普通用户）。
///
/// 回归钉子：`cover-background` 的 `source` 与 `cover-config` 的 `cover.image` 此前完全
/// 没做归属校验（`scope.rs` 只归一化 `dir` / `output_file`，而模块注释却声称覆盖了
/// `source`）。后果不是"多读一个文件"：`source` 指向 `<数据目录>/.vtrim/llm.toml`
/// 就能把**全局 LLM API key** 复制进自己的项目，再用 `/api/media/<自己的项目>/…` 读回来；
/// 同样能拿到账号账本 `cloud/vtrim.db`（明文邀请码 + 全部项目真实路径）。
#[tokio::test]
async fn polish_path_fields_cannot_read_outside_files() {
    let t = test_app().await;

    // 受害者（admin 的项目）里的一个"机密"文件
    let secret = t.root.join("output").join("secret.txt");
    std::fs::write(&secret, b"api_key = 'sk-real-secret'").unwrap();

    // 普通用户 + 他自己的项目目录（含 clip-01 与 polish.toml，够走完前置校验）
    let hash = vtrim_service::auth::hash_password("pass-12345678").unwrap();
    let uid = t.auth.db().create_user("plain_paths", &hash, "user", false, 0).unwrap();
    let my_root = t.root.join("plain-proj");
    let my_clip = my_root.join("output/clips/clip-01");
    std::fs::create_dir_all(&my_clip).unwrap();
    std::fs::write(
        my_clip.join("polish.toml"),
        "[source]\nvideo = \"clip.mp4\"\nsubtitles = \"clip.srt\"\n",
    )
    .unwrap();
    let my_own_bg = my_root.join("bg.jpg");
    std::fs::write(&my_own_bg, b"not a secret").unwrap();
    let my_cfg = my_root.join("vtrim.toml");
    std::fs::write(&my_cfg, "input_file = \"a.mp4\"\n").unwrap();
    let my_pid = t
        .auth
        .db()
        .create_project(uid, "plain", None, "local", Some("测试"), &my_root, &my_cfg, None)
        .unwrap();
    let b = t.auth.login("plain_paths", "pass-12345678", Some("10.1.1.1"), None).unwrap();
    let (sid, csrf) = (b.session_id.clone(), b.csrf.clone());

    let clip = my_clip.to_string_lossy().to_string();

    // ① cover-background：项目外的 source → 404，且**不许**复制出文件
    let body = serde_json::json!({ "dir": clip, "source": secret.to_string_lossy() }).to_string();
    let res = t
        .call(
            t.req_as(&sid, &csrf, "POST", "/polish/cover-background")
                .header(axum::http::header::CONTENT_TYPE, "application/json")
                .body(Body::from(body))
                .unwrap(),
        )
        .await;
    assert_eq!(res.status(), StatusCode::NOT_FOUND, "项目外 source 必须 404");
    assert!(
        !my_clip.join("_cover_bg.jpg").exists(),
        "被拒的请求不得留下复制出来的文件"
    );
    // 正对照：自己项目内的文件照常可用（证明不是"一律拒绝"）
    let body = serde_json::json!({ "dir": clip, "source": my_own_bg.to_string_lossy() }).to_string();
    let res = t
        .call(
            t.req_as(&sid, &csrf, "POST", "/polish/cover-background")
                .header(axum::http::header::CONTENT_TYPE, "application/json")
                .body(Body::from(body))
                .unwrap(),
        )
        .await;
    assert_eq!(res.status(), StatusCode::OK, "自己项目内的背景图应可用");

    // ② cover-config：cover.image 指向项目外 → 404
    let body = serde_json::json!({ "dir": clip, "cover": { "image": secret.to_string_lossy() } })
        .to_string();
    let res = t
        .call(
            t.req_as(&sid, &csrf, "POST", "/polish/cover-config")
                .header(axum::http::header::CONTENT_TYPE, "application/json")
                .body(Body::from(body))
                .unwrap(),
        )
        .await;
    assert_eq!(res.status(), StatusCode::NOT_FOUND, "项目外 cover.image 必须 404");
    let cfg = vtrim_polish::project::load_project(&my_clip).unwrap();
    assert!(cfg.cover.image.is_empty(), "被拒的请求不得写进 polish.toml");
    // 正对照：自己项目内的图能存进去（证明 404 来自归属校验，不是别的校验在挡）
    let body = serde_json::json!({ "dir": clip, "cover": { "image": my_own_bg.to_string_lossy() } })
        .to_string();
    let res = t
        .call(
            t.req_as(&sid, &csrf, "POST", "/polish/cover-config")
                .header(axum::http::header::CONTENT_TYPE, "application/json")
                .body(Body::from(body))
                .unwrap(),
        )
        .await;
    assert_eq!(res.status(), StatusCode::OK, "自己项目内的图应能存进配置");
    assert!(
        !vtrim_polish::project::load_project(&my_clip).unwrap().cover.image.is_empty(),
        "正对照应当真的写进 polish.toml"
    );

    // ③ project-data/config：绝对的 input_file / output_dir → 400
    for patch in [
        serde_json::json!({ "project": { "input_file": secret.to_string_lossy() } }),
        serde_json::json!({ "cutting": { "output_dir": t.root.to_string_lossy() } }),
    ] {
        let body = serde_json::json!({ "patch": patch }).to_string();
        let res = t
            .call(
                t.req_as(&sid, &csrf, "PUT", &format!("/project-data/config?project={my_pid}"))
                    .header(axum::http::header::CONTENT_TYPE, "application/json")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await;
        assert_eq!(res.status(), StatusCode::BAD_REQUEST, "绝对路径键必须 400：{patch}");
    }
    let saved = std::fs::read_to_string(&my_cfg).unwrap();
    assert!(!saved.contains("secret.txt"), "被拒的配置不得落盘：{saved}");
    // 正对照：项目内相对路径照常能存
    let body = serde_json::json!({ "patch": { "project": { "input_file": "a.mp4" } } }).to_string();
    let res = t
        .call(
            t.req_as(&sid, &csrf, "PUT", &format!("/project-data/config?project={my_pid}"))
                .header(axum::http::header::CONTENT_TYPE, "application/json")
                .body(Body::from(body))
                .unwrap(),
        )
        .await;
    assert_eq!(res.status(), StatusCode::OK, "项目内相对路径应能保存");
}

/// polish 的 `dir` 只能落在本人项目内：项目外绝对路径 → 404。
#[tokio::test]
async fn polish_dir_must_be_inside_own_project() {
    let t = test_app().await;
    let outside = std::env::temp_dir().join("vtrim-not-my-project");
    std::fs::create_dir_all(&outside).unwrap();
    let body = serde_json::json!({ "dir": outside.to_string_lossy() }).to_string();
    let res = t
        .call(
            t.req("POST", "/polish/init")
                .header(axum::http::header::CONTENT_TYPE, "application/json")
                .body(Body::from(body))
                .unwrap(),
        )
        .await;
    assert_eq!(res.status(), StatusCode::NOT_FOUND, "项目外目录必须 404");
    let _ = std::fs::remove_dir_all(&outside);
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

    let cfg = vtrim_polish::project::load_project(&clip).unwrap();
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
    let cfg = vtrim_polish::project::load_project(&clip).unwrap();
    assert_eq!(cfg.cover.bg_color, "#000000", "非法颜色必须被净化成默认黑");
    assert!((cfg.cover.blur_scale - 4.0).abs() < 1e-9, "倍率要夹到上限");
    // 保存后仍能在配置文件里看到这些键（注释模板会带上说明）
    let raw = std::fs::read_to_string(clip.join("polish.toml")).unwrap();
    assert!(raw.contains("bg_mode") && raw.contains("blur_scale"), "polish.toml 缺键：{raw}");
}
}
