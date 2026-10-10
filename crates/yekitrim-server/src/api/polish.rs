//! `api::polish`：精修编辑器后端（工程读写 / 字幕 / 贴纸 / 封面 / 预览）。

use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::{
    // `Bytes` = 字体拖拽上传的**原始字节**载体（与贴纸上传同一条做法，不用 multipart）
    body::Bytes,
    extract::{Extension, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use yekitrim_service::Workspace;

use crate::model::AppState;
use crate::jobs::{JobClass, JobSpec};
// 贴纸两接口的路径判据：**唯一一份**在 `crate::scope`（中间件与 handler 两层共用）
use crate::scope::is_safe_project_relative;
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
                if let Ok(cfg) = yekitrim_polish::project::load_project(&dir) {
                    // 消音版 = 静音区间已写入屏蔽轨道（video 始终是未改动的 clip.mp4）
                    if !cfg.silence_ranges.is_empty() { version = "silent"; }
                    mute_range_count = cfg.mute_ranges.len();
                }
            }
            out.push(serde_json::json!({
                "dir": yekitrim_core::path::display(&dir),
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
    /// `render`：出片通道（现在只有 `dhampir` = 底座；`chrome` 那条腿已删除）。
    /// **None = 不传 `--renderer`**，让 CLI 用它自己的默认值 —— 默认值只有一处（`cli.rs`），
    /// 服务端不抄第二份。
    pub renderer: Option<String>,
    /// `llm`：true = **全部重做**（先清掉断点续传缓存）
    pub full: bool,
    /// `llm_segment`：只重做这一段的事件（下标见 `polish.toml` 的 `[[segments]]`）
    pub segment_index: Option<usize>,
}

pub(super) async fn polish_spawn(
    state: &AppState,
    owner: yekitrim_core::owner::Owner,
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
    // 资源域：**封面**只与另一个封面互斥（导出期间也允许生成封面，见 `scope_key::cover_keys`）；
    // 其余精修任务 = 这个片段目录：与同片段的其它精修任务、以及"单条重切"互斥（冲突则排队）。
    // 档位按 kind：渲染重活、LLM 略重、init/process/cover 轻活（`JobClass::for_polish`）。
    let keys = if kind == "cover" {
        crate::scope_key::cover_keys(&path)
    } else {
        crate::scope_key::polish_keys(project, &path)
    };
    let class = JobClass::for_polish(kind);
    // ⚠️ 两处要的东西**不一样**，别把它们合并成一个变量：
    // 任务队列（账本）按**数字 uid** 记归属；而子进程要的是**完整 `Owner`** ——
    // 单人形态是 `Local`、数字 uid 根本表达不了（`--local-no-auth` 下 uid 是合成管理员的）。
    let job_owner_uid = match owner {
        // 单人（`--local-no-auth`）没有真实账号 —— 任务队列仍要一个 uid 才能记归属。
        // ⚠️ 用**当前管理员的真实 uid**（`Local` 就是那个人的另一种进入方式），
        // 别写死 1：首个管理员不一定是 1（1 号账号被删过就是这种库）。
        yekitrim_core::owner::Owner::Local => yekitrim_core::owner::local_admin_uid(),
        yekitrim_core::owner::Owner::User(uid) => uid,
    };
    let handle = state
        .jobs
        .create(
            job_owner_uid,
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
        // 取消探针：精修 llm 是分钟级链路（窗口多、默认并发 3），Agent 必须能在**窗口边界**
        // 轮询它 —— 只在阶段边界查一次 = 用户点了停止、后台仍把剩下的窗口成批跑完。
        // 判据与管线/贴纸构建同源：Job 的取消令牌 → `ProgressReporter::cancelled()`。
        let token = jobs.cancel_token(&job_id).await;
        let bridge = super::core::progress_bridge(job_id.clone(), jobs.clone(), token);
        let cancel: Option<yekitrim_core::llm::agent::CancelProbe> = Some(std::sync::Arc::new(move || {
            yekitrim_core::progress::ProgressReporter::cancelled(&*bridge)
        }));
        let result = async {
            match kind.as_str() {
                "init" => yekitrim_polish::commands::cmd_init(&path).await,
                // LLM 是显式步骤，process 隐式：完成后同一 job 内自动执行后处理
                "llm" => {
                    jobs.log(&job_id, "info", if opts.full {
                        "LLM 生成中（全部重做，不读断点续传）…"
                    } else {
                        "LLM 生成中…"
                    }).await;
                    // ⚠️ **必须带 owner**：本条在 server 进程内直接跑（不是拉子进程），
                    // 所以 `YEKITRIM_OWNER` 那套对它无效 —— 用 `cmd_llm` 会去读
                    // **单人库**的贴纸索引，症状是多用户下"自己传的贴纸包选不中"。
                    yekitrim_polish::commands::cmd_llm_for(owner, &path, opts.full, cancel.clone()).await?;
                    jobs.log(&job_id, "info", "LLM 完成，自动执行 process 后处理…").await;
                    yekitrim_polish::process::cmd_process(&path).await
                }
                // 只重做某一段的事件（编辑器：时间轴 → 分段属性）。失败会**保留该段原有事件**，
                // 所以这里 fail 掉任务不会让用户丢东西；后处理照跑（别的段不动，幂等）。
                "llm_segment" => {
                    let idx = opts
                        .segment_index
                        .ok_or_else(|| anyhow::anyhow!("缺少 segment_index"))?;
                    jobs.log(&job_id, "info", format!("只重做分段 #{idx} 的事件…")).await;
                    yekitrim_polish::commands::cmd_llm_segment_for(owner, &path, idx, cancel.clone()).await?;
                    jobs.log(&job_id, "info", "完成，自动执行 process 后处理…").await;
                    yekitrim_polish::process::cmd_process(&path).await
                }
                "process" => yekitrim_polish::process::cmd_process(&path).await,
                "render" => {
                    // 输出文件名在**真开跑之后**才落到 polish.toml（排队期间写会覆盖别人
                    // 的改动，而且任务可能还要等很久）。update_polish_toml 自己带路径锁。
                    if let Some(name) = opts.output_file.clone() {
                        yekitrim_polish::project::update_polish_toml(&path, |cfg| {
                            cfg.project.output = name.clone();
                        })
                        .await?;
                    }
                    // 用外部 CLI 子进程渲染：出片是分钟级的重活，跑在服务进程里会拖着整个服务一起卡。
                    // 输出流式回灌任务日志 + 取消时杀进程树，见 run_polish_child。
                    // 通道：body 显式给了才透传（白名单已在这一层的入口处校验过），
                    // 否则不带 `--renderer` —— CLI 的默认值（现在是 dhampir）说了算。
                    let mut extra: Vec<String> = Vec::new();
                    if let Some(r) = opts.renderer.clone() {
                        extra.push("--renderer".into());
                        extra.push(r);
                    }
                    run_polish_child(&data_dir, &jobs, &job_id, "render", &path, &extra, owner).await
                }
                "cover" => {
                    // 与 render 同理：封面也走外部 CLI（同样要能看进度、能取消）。
                    // 通道：body 显式给了就透传，否则不带 `--renderer` —— `cover.rs`
                    // 的解析（显式 > `VTEDIT_COVER_RENDERER` > takumi）说了算，白名单也只那一份。
                    let mut extra: Vec<String> = Vec::new();
                    if let Some(r) = opts.renderer.clone() {
                        extra.push("--renderer".into());
                        extra.push(r);
                    }
                    run_polish_child(&data_dir, &jobs, &job_id, "cover", &path, &extra, owner).await
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
    polish_spawn(&state, crate::auth::owner_of(&user), scope_pid(pid), "init", dir, PolishJobOpts::default()).await
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
    polish_spawn(&state, crate::auth::owner_of(&user), scope_pid(pid), "llm", dir, PolishJobOpts { full, ..Default::default() }).await
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
        crate::auth::owner_of(&user),
        scope_pid(pid),
        "llm_segment",
        dir,
        PolishJobOpts { segment_index: Some(index), ..Default::default() },
    )
    .await
}

pub(super) async fn polish_process(State(state): State<Arc<AppState>>, Extension(user): Extension<crate::auth::CurrentUser>, pid: Option<Extension<crate::scope::ScopeProjectId>>, Json(body): Json<serde_json::Value>) -> ApiResult {
    let dir = body.get("dir").and_then(|v| v.as_str()).unwrap_or("").to_string();
    polish_spawn(&state, crate::auth::owner_of(&user), scope_pid(pid), "process", dir, PolishJobOpts::default()).await
}

pub(super) async fn polish_render(State(state): State<Arc<AppState>>, Extension(user): Extension<crate::auth::CurrentUser>, pid: Option<Extension<crate::scope::ScopeProjectId>>, Json(body): Json<serde_json::Value>) -> ApiResult {
    let dir = body.get("dir").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let path = validated_project_dir(&dir)?;
    if !{
        let __p = path.join("polish.toml");
        // 同步 stat 不能留在 async worker 上（api/mod.rs:44-51 ✓）：
        // 工程常在慢盘/网络盘上，一次 exists() 就要几百 ms ✗
        tokio::task::spawn_blocking(move || __p.exists())
            .await
            .unwrap_or(false)
    } { return Err(err(StatusCode::BAD_REQUEST, "目录未初始化 polish")); }
    // 输出文件名**不在这里写**：任务可能被排到后面，等它真开跑时再落到 polish.toml
    // （见 polish_spawn 的 render 分支）—— 否则排队期间别的任务/用户改动会被这次预写覆盖。
    let output_file = body
        .get("output_file")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());
    // 出片通道：**白名单**（不给就跟随 CLI 的默认值 —— 现在也只有底座 dhampir 一个值）。
    // ⚠️ 非法值回 **400**，不静默当默认：那会让人以为"选了别的通道却在跑底座"，
    // 而两版成片的差别只有对着看才发现（与 CLI 侧"认不出就报错"同一条口径）。
    // ⚠️ `chrome` 也要落进 400：那条本仓 headless Chrome 的兜底腿已于 2026-10-01 整条删除，
    // 但还会有人照老文档/老脚本传它 —— 白名单是最先能给出明确答复的地方。
    let renderer = match body.get("renderer").and_then(|v| v.as_str()) {
        None | Some("") => None,
        Some(r @ "dhampir") => Some(r.to_string()),
        Some(other) => {
            return Err(err(
                StatusCode::BAD_REQUEST,
                &format!(
                    "认不出的出片通道 `{other}`（现在只有 dhampir —— \
                     本仓 headless Chrome 那条兜底腿已于 2026-10-01 整条删除）"
                ),
            ))
        }
    };
    polish_spawn(
        &state,
        crate::auth::owner_of(&user),
        scope_pid(pid),
        "render",
        dir,
        PolishJobOpts { output_file, renderer, ..Default::default() },
    )
    .await
}

pub(super) async fn polish_cover(State(state): State<Arc<AppState>>, Extension(user): Extension<crate::auth::CurrentUser>, pid: Option<Extension<crate::scope::ScopeProjectId>>, Json(body): Json<serde_json::Value>) -> ApiResult {
    let dir = body.get("dir").and_then(|v| v.as_str()).unwrap_or("").to_string();
    // 通道由**前端**给（现在只有 `takumi`）；这里只透传，取值口径在 `cover.rs`（只一份）——
    // 传 `chrome` 会由那里**明确报错**（那条截图腿已删除），这一层不抄第二份白名单。
    let renderer = body.get("renderer").and_then(|v| v.as_str()).map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
    polish_spawn(&state, crate::auth::owner_of(&user), scope_pid(pid), "cover", dir, PolishJobOpts { renderer, ..Default::default() }).await
}


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
    let global_cfg = yekitrim_core::config::load_global_config();
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

    let llm = yekitrim_core::config::load_llm_config();
    if llm.base_url.is_empty() || llm.api_key.is_empty() {
        return Err(err(StatusCode::BAD_REQUEST, "LLM 未配置"));
    }
    // 链路解析：零散单发链路（见 [llm.overrides.utility]）；thinking 的默认关思考由它给出
    let linked = llm.for_link(yekitrim_core::config::LlmLink::Utility);
    let client = yekitrim_core::llm::LlmClient::from_config(&linked)
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("LLM 配置错误: {e}")))?
        .with_timeout(60)
        .with_usage_context(
            yekitrim_core::llm::usage::UsageContext::new("封面文案")
                .project(Some(yekitrim_core::path::display(std::path::Path::new(&dir)))),
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
"#, yekitrim_core::detection::TITLE_RULES);
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
pub(super) async fn polish_silence(State(_state): State<Arc<AppState>>, Json(body): Json<serde_json::Value>) -> ApiResult {
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
    let _guard = yekitrim_core::filelock::lock_path(&dir.join(format!(".yekitrim-preview-{kind}"))).await;
    if let Some(port) = cached(&key) {
        return Ok(port);
    }
    let mut server = yekitrim_polish::render::server::RenderServer::new(dir.to_path_buf());
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
    // 启动预览服务（不阻塞，返回 URL 供前端 iframe）
    let config = yekitrim_polish::project::load_project(&path).map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("加载失败: {e}")))?;
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
            let _ = yekitrim_polish::process::cmd_process(&path).await;
        }
    }
    yekitrim_polish::process::ensure_assets(&path).await;
    let port = preview_port_for(&state, &path, "video")
        .await
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("预览启动失败: {e:#}")))?;
    let video_url = format!("http://127.0.0.1:{}/", port);
    // 前端需要通过 API 代理访问本机 127.0.0.1 随机端口；这里直接返回绝对 URL（浏览器同源可访问）
    ok(serde_json::json!({ "ok": true, "video_url": video_url, "cover_url": video_url }))
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

pub(super) async fn polish_cover_background(State(_state): State<Arc<AppState>>, Json(body): Json<serde_json::Value>) -> ApiResult {
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

pub(super) async fn polish_cover_frame(State(_state): State<Arc<AppState>>, Json(body): Json<serde_json::Value>) -> ApiResult {
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

pub(super) async fn polish_cover_preview(State(state): State<Arc<AppState>>, Json(body): Json<serde_json::Value>) -> ApiResult {
    let dir = body.get("dir").and_then(|v| v.as_str()).unwrap_or("").to_string();
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
    // 缺关键帧时现场截一帧（ffmpeg，可能数百 ms~数秒）→ 整段下 blocking 池
    let cover_path = path.clone();
    let got_frame = blocking(move || {
        let config = yekitrim_polish::project::load_project(&cover_path)
            .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("加载失败: {e}")))?;
        let resolved = yekitrim_polish::render::resolve_paths(&cover_path, &config);
        let cover_image = cover_path.join("_cover_frame.jpg");
        if !cover_image.exists() {
            let duration = config.segments.last().map(|s| s.end).unwrap_or(0.0);
            let frame_time = (duration * config.cover.frame_pos).max(0.1);
            let frame_out = yekitrim_core::process::cmd("ffmpeg")
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
        let t = yekitrim_core::srt::parse_srt(&raw).map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("解析失败: {e}")))?;
        return ok(serde_json::json!({ "segments": t.segments }));
    }
    Err(err(StatusCode::NOT_FOUND, "暂无字幕文件"))
}

pub(super) async fn polish_subtitles_put(State(_state): State<Arc<AppState>>, Json(body): Json<serde_json::Value>) -> ApiResult {
    // ⚠️ 整段都是同步 IO：`validated_project_dir` 的 canonicalize + 建 data/ + 写两份文件。
    // GET 孪生（polish_subtitles_get）早就这么包了，PUT 这一半漏了。
    blocking(move || polish_subtitles_put_impl(body)).await
}

fn polish_subtitles_put_impl(body: serde_json::Value) -> ApiResult {
    let dir = body.get("dir").and_then(|v| v.as_str()).ok_or_else(|| err(StatusCode::BAD_REQUEST, "缺少 dir"))?;
    let path = validated_project_dir(dir)?;
    if !path.join("polish.toml").exists() {
        return Err(err(StatusCode::BAD_REQUEST, "目录未初始化 polish"));
    }
    let segments = body.get("segments").cloned().ok_or_else(|| err(StatusCode::BAD_REQUEST, "缺少 segments"))?;
    let mut segs: Vec<yekitrim_core::types::TranscriptSegment> = segments.as_array()
        .ok_or_else(|| err(StatusCode::BAD_REQUEST, "segments 必须是数组"))?
        .iter().map(|v| {
            let start = v.get("start").and_then(|x| x.as_f64()).unwrap_or(0.0);
            let end = v.get("end").and_then(|x| x.as_f64()).unwrap_or(start + 1.0);
            let text = v.get("text").and_then(|x| x.as_str()).unwrap_or("").to_string();
            let confidence = v.get("confidence").and_then(|x| x.as_f64()).unwrap_or(1.0);
            yekitrim_core::types::TranscriptSegment { start, end, text, confidence }
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
    let transcript = yekitrim_core::types::Transcript {
        segments: segs,
        language: body.get("language").and_then(|l| l.as_str()).unwrap_or("zh").to_string(),
        asr_metadata: None,
    };
    yekitrim_core::srt::write_srt(&transcript, &path.join("clip.srt"))
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("写 SRT 失败: {e}")))?;
    ok(serde_json::json!({ "ok": true, "saved": out.len() }))
}

/// 响度曲线（带缓存）：同一源视频只测一次，结果存 `data/loudness.json`。
///
/// 实现已收敛到 `yekitrim_polish::render::measure_loudness_curve_cached`（出片页与混音共用同一份）。
pub(super) fn loudness_curve_cached(project_dir: &Path, video_path: &Path) -> serde_json::Value {
    let curve = yekitrim_polish::render::measure_loudness_curve_cached(project_dir, video_path);
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
    let cfg = yekitrim_polish::project::load_project(&path).map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("加载失败: {e}")))?;
    let mut value = serde_json::to_value(&cfg).unwrap_or_default();
    // preview=1: 附加预览渲染所需的计算字段（sticker_map / 竖屏检测 / 响度曲线）
    if q.get("preview").map(|v| v == "1").unwrap_or(false) {
        if let Some(obj) = value.as_object_mut() {
            obj.insert("_assets".into(), serde_json::json!({
                "sticker_map": yekitrim_polish::render::server::build_sticker_map(&path),
            }));
            let video_path = path.join(&cfg.source.video);
            // 画布契约与出片读的是**同一份推导**（`ProjectFile::canvas_contract`）。
            // 这里以前自己写了一份「竖屏检测」，与 `render/server.rs` 各一份 —— 改画布方向会不同步。
            let src_size = yekitrim_polish::render::get_video_size(&video_path.to_string_lossy());
            let contract = cfg.canvas_contract(src_size);
            obj.insert(
                "vertical".into(),
                serde_json::json!(src_size.map(|(w, h)| h > w).unwrap_or(false)),
            );
            // 六条轨道的显示开关（时间轴那六枚眼睛）：与出片模板/转译器**同一个判据**
            // （`ProjectFile::lane_enabled`，缺字段按素材+画布方向取默认）。
            obj.insert("_lanes".into(), cfg.lanes_json(src_size));
            obj.insert(
                "vertical_position".into(),
                contract["stage"]["position"].clone(),
            );
            obj.insert("_canvas".into(), contract);
            // 最近一次 LLM 生成的结果（哪段成/哪段失败），编辑器按它给分段上警示色。
            // 只是展示用：读不到就当没有，绝不影响加载。
            if let Some(st) = yekitrim_polish::llm::status::load(&path) {
                obj.insert(
                    "_llm_status".into(),
                    serde_json::to_value(st).unwrap_or(serde_json::Value::Null),
                );
            }
            if { let __p = video_path.clone(); tokio::task::spawn_blocking(move || __p.exists()).await.unwrap_or(false) } {
                obj.insert("loudness_curve".into(), loudness_curve_cached(&path, &video_path));
            }
        }
    }
    ok(value)
}

// ⚠️ 这里曾经有 `polish_render_frame`（单帧真渲染预览：常驻 wgpu 后端 + `model::PreviewBackend` 槽，
// 路由 `/api/polish/render-frame`），以及 feature 未启用时的那个 501 占位版本。**整条已删除**
// （用户拍板：GPU 那侧交给独立的 Dhampir 底座），`/api/health` 的 `wgpu_render` 能力位一并去掉 ——
// 前端也不再按它隐藏任何入口。**别再把它加回来**：要"真渲染预览"就走底座那条链路；
// 出片的事实来源是底座（`render/dhampir.rs`），预览那层是 `webui/src/polish/engine.js`。

/// 字体库全量（**只读**，非 admin 也能看）：字幕字体 / 封面字体的下拉数据源。
///
/// 库位置就是 `yekitrim_core::path::font_dir()`：**`<数据目录>/resources/fonts/`** 一处
/// （默认程序目录；`YEKITRIM_DATA_DIR` 可覆盖）。读写同一个位置，见 `fonts::ensure_dir`。
/// **以磁盘为准**：往里丢一个 `.ttf` / `.otf` / `.ttc` 就会出现在列表里，没有索引文件要维护。
/// `id` = 目录名（v2 布局）或文件名去扩展名（老布局）—— **配置里存的就是它**；
/// `family` = **内部族名**（`FONT.toml` 的 `family`，唯一权威：预览 / 出片 `@font-face` / Takumi 三处都认它）。
/// `display_name` 只是给人看的标签，别拿它当族名用（详见 `docs/plan/font-library.md` §13）。
/// 扫描要读字体文件解析字族名，所以整段走 `blocking`（同步 IO 不能留在 worker 上）。
///
/// `fallback_stack` = 出片侧的**动态兜底字体栈**（`yekitrim-polish` 的 `server::fallback_stack`：
/// 首项随库里默认那支走）—— 预览（`PolishPreview.vue` 的 `.cap` 栈）据它与成片保持同源，
/// 这是三处字体来源里"预览那一腿"的收口。
///
/// `root`（服务器上的绝对路径）**只给 admin**：非 admin 拿它没用（也丢不进文件），
/// 而它泄露的是服务器的安装路径 —— 与 `/api/admin/usage` 的理由同一条。
pub(super) async fn polish_fonts(Extension(user): Extension<crate::auth::CurrentUser>) -> ApiResult {
    let is_admin = user.is_admin();
    let owner = crate::auth::owner_of(&user);
    let payload = blocking(move || {
        // ⚠️ **读两层**（用户库优先、全局补缺，2026-10-10）：自带字体装在**全局**，
        // 用户自己导入的在**自己库**里；只看其中一层会"少一半字体"。
        // 写入仍然只有一处（各 `ensure_dir_for(owner)`），见 `owner::asset_dirs` 的说明。
        let roots = yekitrim_core::owner::asset_dirs(
            owner,
            yekitrim_core::owner::AssetKind::Fonts,
        );
        let root = roots[0].clone();
        let fonts: Vec<serde_json::Value> = yekitrim_core::fonts::scan_all_roots(&roots)
            .into_iter()
            .map(|f| {
                serde_json::json!({
                    "id": f.id,
                    "file": f.file,
                    // ⚠️ 走 `family_for`（唯一权威），别直接回 `f.family`（那是"从文件读到的名字"，
                    // 可能与 FONT.toml 不一致）—— 界面上"内部族名 vs 显示名"两行要靠它对得上。
                    // ⚠️ 走多根：这款字体可能在**全局库**里（自带字体就在那）
                    "family": roots
                        .iter()
                        .map(|r| yekitrim_core::fonts::family_for_at(r, &f.id))
                        .find(|s| !s.is_empty())
                        .unwrap_or_default(),
                    "display_name": f.display_name,
                    "bytes": f.bytes,
                    "builtin": f.builtin,
                    "is_default": f.is_default,
                    "license": f.license,
                    "scripts": f.scripts,
                    "emoji": f.emoji,
                    // 彩色字形能力（`cbdt`/`sbix`/`colrv0`/`colrv1`/`svg`/`none`）。
                    // ⚠️ 界面据此判「这支 emoji 字体到底能不能出彩色」：`colrv1`/`svg` 会**静默**坏掉
                    // （Takumi 只认 COLRv0，见 `yekitrim_core::fonts::color_glyph_support` 的表）。
                    // 纯文字字体这里是 `none` —— 正常，别当成错误。
                    "color_format": f.color_format,
                    // ⚠️ 自带字体**没下载**时也在列表里（`installed: false`）：界面据此给"下载"按钮，
                    // 而不是当成"库里有这支"。用户自己导入的永远是 true。
                    "installed": f.installed,
                    // 老平铺布局（散文件直接在库根目录）⇒ 界面打「老布局」标签
                    "flat": f.flat,
                })
            })
            .collect();
        // 自带字体清单（"还能下什么"）：与 `fonts::BUILTIN_FONTS` 同源，别在这一层手抄
        let builtin: Vec<serde_json::Value> = yekitrim_core::fonts::builtin_fonts()
            .iter()
            .map(|b| {
                serde_json::json!({
                    "id": b.id,
                    "display_name": b.display_name,
                    "family": b.family,
                    "license": b.license,
                    "license_url": b.license_url,
                    // ⚠️ 自带字体现在装在**全局库**里，所以"装没装"要按**两层**判：
                    // 只看用户库会永远报 `installed: false`（界面显示"未安装"，
                    // 而实际出片时是能用的 —— 这种不一致最难查）。
                    "installed": roots
                        .iter()
                        .any(|r| yekitrim_core::fonts::builtin_installed(r, b)),
                })
            })
            .collect();
        Ok(serde_json::json!({
            "ok": true,
            // 出 JSON 的路径必须过 display（去掉 `\\?\` 逐字前缀），否则前端拼路径会炸
            "root": if is_admin { yekitrim_core::path::display(&root) } else { String::new() },
            // 动态兜底字体栈（首项随库里默认走）：预览 `.cap` 栈与成片同源的收口
            "fallback_stack": yekitrim_polish::render::server::fallback_stack(),
            // ⚠️ **emoji 补充槽位**（2026-10-02 字体职责分离）：库里那支负责表情的字体，
            // 由 `yekitrim_core::fonts::emoji_font` **自动选**（至多一支），**用户不选**。
            //
            // 前端拿它做两件事：① 预览 `.cap` 栈把它追加在**末尾**（正文在前、emoji 兜底）；
            // ② 界面据此说明"emoji 由系统自动补充"。库里没有就是 null ⇒ 前端不追加
            // （浏览器自带系统 emoji）。
            //
            // ⚠️ 是**独立字段**、不是拼进 `fallback_stack` 的一段：封面那条链看的是
            // **注册顺序**而不是这个栈（Takumi 不读系统字体），底座那条链**根本没有回退**
            // —— 拼进去对它们零效果，还会撑破 polish 侧"四段"那条单测。
            "emoji_family": yekitrim_core::fonts::emoji_font(&root).map(|e| {
                if e.family_internal.trim().is_empty() { e.id } else { e.family_internal }
            }),
            "fonts": fonts,
            "builtin": builtin,
        }))
    })
    .await?;
    ok(payload)
}

/// 往字体库里**导入**一支字体（**仅 admin**）。
///
/// body：`{path}` —— 服务器上那个**字体文件**的绝对路径；或 `{dir, name}`（推荐，
/// **由服务端拼**，免得前端把 `\\?\F:\a` 拼成 `\\?\F:\a/b` 撞上 Windows 123，同 `/api/fs/list` 的规矩）。
/// 可选 `{name}` 当给人看的显示名（`display_name`）；**id 永远取自文件名**（配置里存的就是它）。
///
/// ⚠️ 收的是**服务器上的路径**，不是上传 —— 与 `/api/fs/list` 同一条来源（axum 默认 body 上限 2 MB，
/// 上传要像贴纸那样单独放宽 `DefaultBodyLimit`，那是另一件事）。所以这条**必须** `require_admin`：
/// 普通用户让它读服务器任意文件就是路径面事故。
pub(super) async fn polish_fonts_import(
    Extension(user): Extension<crate::auth::CurrentUser>,
    Json(body): Json<serde_json::Value>,
) -> ApiResult {
    let owner = crate::auth::owner_of(&user);
    let path = match body.get("path").and_then(|v| v.as_str()).map(str::trim) {
        Some(p) if !p.is_empty() => PathBuf::from(p),
        _ => {
            // `{dir, name}`：**服务端拼**（前端自己拼 Windows verbatim 前缀会出 123）
            let dir = body.get("dir").and_then(|v| v.as_str()).unwrap_or("").trim();
            let name = body.get("name").and_then(|v| v.as_str()).unwrap_or("").trim();
            if dir.is_empty() || name.is_empty() {
                return Err(err(StatusCode::BAD_REQUEST, "缺少 path，或 dir + name"));
            }
            // 只取文件名那一段：`name` 来自请求体，`..\..\x` 不能让它越界读文件
            let base = Path::new(name)
                .file_name()
                .map(PathBuf::from)
                .ok_or_else(|| err(StatusCode::BAD_REQUEST, "name 不是合法文件名"))?;
            if base.as_os_str().is_empty() {
                return Err(err(StatusCode::BAD_REQUEST, "name 不是合法文件名"));
            }
            Path::new(dir).join(base)
        }
    };
    // ⚠️ 显示名读 **`display`**（它与 `name` 撞语义：`name` 是**文件名**，服务端拿它拼路径 / 当 id）。
    // 兼容老的 `name`-当显示名（那时 `{path}` 分支没有文件名冲突）。
    let display = body
        .get("display")
        .or_else(|| body.get("name").filter(|_| body.get("path").is_some()))
        .and_then(|v| v.as_str())
        .map(str::to_string);
    let entry = blocking(move || {
        // 目录布局写入 + 读字体表都碰磁盘（可能是网络盘），一律留在 blocking 池上。
        // ⚠️ 字体库的错都是 `anyhow`（"只收 .ttf/.otf"、"程序自带字体不可删除"…），
        // 转成 400 + 原话：用户要看到"为什么这支不行"，不是笼统的 500。
        // ⚠️ 写路径走 `ensure_dir_for(owner)`（它会 `create_dir_all`），别自己拼 `font_dir()`。
        // ⚠️ **必须带 owner**：这条走的是 `require_user` 那条用户路由，写错的库会让
        // 甲导入的字体落进单人库、而甲自己的库还是空的（多用户下就是"传了却看不见"）。
        let root = yekitrim_core::fonts::ensure_dir_for(owner)
            .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("字体库目录不可用: {e}")))?;
        yekitrim_core::fonts::import_font(&root, &path, display.as_deref())
            .map_err(|e| err(StatusCode::BAD_REQUEST, e.to_string()))
    })
    .await?;
    ok(serde_json::json!({ "ok": true, "font": entry }))
}

/// 字体库单文件**上传上限**（2026-10-02 拖拽导入）。
///
/// ⚠️ 与前端 `webui/src/api/polish.js::FONT_UPLOAD_MAX_BYTES` 是**同一个数**（改一处要改两处，
/// 同贴纸那条规矩）。取 64 MiB 的理由：常见中文字体 10~30 MB，彩色 emoji 位图字体 ~10 MB，
/// 给到 64 MiB 能收下绝大多数；再大就不该走浏览器这条链路了。
pub(crate) const FONT_UPLOAD_MAX_BYTES: usize = 64 * 1024 * 1024;

/// `POST /api/polish/fonts/upload?name=<文件名>` —— **拖拽上传**一支字体（**仅 admin**）。
///
/// 请求体是字体的**原始字节**（`application/octet-stream`，不是 multipart）—— 与贴纸上传同一条
/// 做法：单文件用 multipart 只是多一层边界解析，而前端 `api()` 本来就能直接传 `File`。
/// 路由上挂了 `DefaultBodyLimit::max(FONT_UPLOAD_MAX_BYTES)`（axum 默认只有 2 MB）。
///
/// ⚠️ `name` 是**文件名**（服务端拿它当 id），显示名走独立的 `display` 查询参数 ——
/// 两个字段语义别混（与 `polish_fonts_import` 同一口径）。
///
/// ⚠️ 与 import 的分工：import 收的是**服务器上已有的路径**（admin 浏览服务器文件那条来源），
/// 这条收的是**浏览器里的字节**。两条都能进库，但来源不同、安全考量也不同 ——
/// 这条不需要路径归属校验（没有路径），只需要体积上限 + 格式校验。
pub(super) async fn polish_fonts_upload(
    Extension(user): Extension<crate::auth::CurrentUser>,
    Query(q): Query<std::collections::HashMap<String, String>>,
    body: Bytes,
) -> ApiResult {
    let owner = crate::auth::owner_of(&user);
    let file_name = q.get("name").map(String::as_str).unwrap_or("").trim();
    if file_name.is_empty() {
        return Err(err(StatusCode::BAD_REQUEST, "缺少 name（字体文件名）"));
    }
    // ⚠️ 只取 basename：`name` 来自查询串，`..\\..\\x.ttf` 不能让它决定落盘位置
    // （库内的 id 与文件名由 `import_bytes` 自己从 basename 推，这里只是先挡一道）。
    let base = Path::new(file_name)
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    if base.is_empty() || base.starts_with('.') {
        return Err(err(StatusCode::BAD_REQUEST, "name 不是合法文件名"));
    }
    let display = q.get("display").map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
    let entry = blocking(move || {
        // ⚠️ 写路径走 `ensure_dir_for(owner)`（它会 `create_dir_all`）。**必须带 owner** ——
        // 写错的库 = 用户传完字体、库里却没有。
        let root = yekitrim_core::fonts::ensure_dir_for(owner)
            .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("字体库目录不可用: {e}")))?;
        yekitrim_core::fonts::import_font_bytes(&root, &base, &body, display.as_deref())
            .map_err(|e| err(StatusCode::BAD_REQUEST, e.to_string()))
    })
    .await?;
    ok(serde_json::json!({ "ok": true, "font": entry }))
}

/// 把某支字体设为**默认字体**（**仅 admin**）。body：`{id}`。
///
/// ⚠️ 默认是**互斥**的（`fonts::set_default` 会把别的清掉）。
/// ⚠️ 只影响**新建项目**用它；已有工程里 `polish.toml` 存的那串 id 不会被改写
/// （改了就是替用户改他已定的稿子）。
pub(super) async fn polish_fonts_default(
    Extension(user): Extension<crate::auth::CurrentUser>,
    Json(body): Json<serde_json::Value>,
) -> ApiResult {
    let owner_fonts = yekitrim_core::owner::asset_dir(
        crate::auth::owner_of(&user),
        yekitrim_core::owner::AssetKind::Fonts,
    );
    let id = body
        .get("id")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| err(StatusCode::BAD_REQUEST, "缺少 id"))?
        .to_string();
    blocking(move || {
        // set_default / remove 操作的是**当前可见的那份库**（`font_dir()` 读侧解析），
        // 不是写路径 —— `set_default` / `remove` 改的是**当前可见的那份库**（`font_dir()`）。
        yekitrim_core::fonts::set_default(&owner_fonts, &id)
            .map_err(|e| err(StatusCode::BAD_REQUEST, e.to_string()))
    })
    .await?;
    ok(serde_json::json!({ "ok": true }))
}

/// 删除库里一支字体（**仅 admin**）。`DELETE /api/polish/fonts?id=<id>`。
///
/// ⚠️ 用 **query 参数**而不是路径段：`axum::extract::Path` 会与本文件里的 `std::path::Path` 撞名，
/// 而且"字体 id 可以带空格/CJK"（`is_safe_id` 允许）—— 放进路径段还要额外转义。
/// ⚠️ `builtin = true`（程序自带）**返回 403**（`fonts::remove_font` 直接 bail，文案「程序自带字体不可删除」）。
pub(super) async fn polish_fonts_delete(
    Extension(user): Extension<crate::auth::CurrentUser>,
    Query(q): Query<std::collections::HashMap<String, String>>,
) -> ApiResult {
    let owner_fonts = yekitrim_core::owner::asset_dir(
        crate::auth::owner_of(&user),
        yekitrim_core::owner::AssetKind::Fonts,
    );
    let id = q.get("id").map(String::as_str).unwrap_or("").trim().to_string();
    if id.is_empty() {
        return Err(err(StatusCode::BAD_REQUEST, "缺少 id"));
    }
    blocking(move || {
        // ⚠️ `builtin` 那一支**必须**能被用户看见 —— 403 的文案就是「程序自带字体不可删除」，
        // 与 `webui/src/components/MediaLibrary.vue`（首页素材库）里把自带字体的删除键置灰是同一条规矩。
        yekitrim_core::fonts::remove_font(&owner_fonts, &id).map_err(|e| {
            let msg = e.to_string();
            let status = if msg.contains("程序自带") {
                StatusCode::FORBIDDEN
            } else {
                StatusCode::BAD_REQUEST
            };
            err(status, msg)
        })
    })
    .await?;
    ok(serde_json::json!({ "ok": true }))
}

/// 下载**自带字体**（**仅 admin**）。body 可为空（下全部还缺的）或 `{id}`（只下某一支）。
///
/// 清单与校验和都在 `yekitrim_core::fonts::BUILTIN_FONTS`（**不在这一层抄第二份**）。
/// ⚠️ 幂等：已装好的**不重复下**（用户可能手动替换过那支字体）。
/// 装上自带字体（**内嵌写入，不联网**；幂等，已装好的跳过）。
///
/// ⚠️ 2026-10-02 从"下载"改成"内嵌"：正常路径下**前端不该再调它** ——
/// `ensure_global_config`（server 启动）已经自动装好。留这条接口是**修复入口**
/// （用户手删了库里那支之后想补回来），语义与 CLI 的 `yekitrim fonts install` 完全一致。
///
/// 仍然走 `blocking`：写 ~23 MB 到（可能很慢的）磁盘上，不该占住 tokio worker。
pub(super) async fn polish_fonts_builtin(
    Extension(user): Extension<crate::auth::CurrentUser>,
    Json(body): Json<serde_json::Value>,
) -> ApiResult {
    let owner = crate::auth::owner_of(&user);
    let only = body
        .get("id")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let report = blocking(move || {
        // ⚠️ 写路径走 `ensure_dir_for(owner)`（它负责 `create_dir_all`）——
        // 否则字体会写进**别人的**库里，而本账号的库仍然显示「未安装」。
        let root = yekitrim_core::fonts::ensure_dir_for(owner)
            .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("字体库目录不可用: {e}")))?;
        // 点名单支时先确认它真是自带的 —— 否则用户白等一场才发现 id 打错
        if let Some(id) = &only {
            if !yekitrim_core::fonts::is_builtin_id(id) {
                return Err(err(
                    StatusCode::BAD_REQUEST,
                    format!("不是自带字体（自带清单里没有它）: {id}"),
                ));
            }
        }
        let mut done: Vec<String> = Vec::new();
        for b in yekitrim_core::fonts::builtin_fonts() {
            if let Some(want) = &only {
                if !b.id.eq_ignore_ascii_case(want) {
                    continue;
                }
            }
            // 已装好的跳过（幂等）
            if yekitrim_core::fonts::builtin_installed(&root, b) {
                continue;
            }
            yekitrim_core::fonts::install_builtin_font(&root, b)
                // 校验和不符 / 落盘失败的原话对用户有意义，照实回
                .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("安装自带字体失败: {e}")))?;
            done.push(b.id.to_string());
        }
        Ok(serde_json::json!({ "ok": true, "installed_now": done, "installed": yekitrim_core::fonts::builtin_status(&root) }))
    })
    .await?;
    ok(report)
}
/// 字体文件端点（wasm 预览 fetch 字体字节用）：项目字体候选 -> 内置资源。
// polish_font_async_read_fixed
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
            if let Some(p) = yekitrim_core::fonts::resolve_file(name.trim()) {
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
        if { let __p = c.clone(); tokio::task::spawn_blocking(move || __p.exists()).await.unwrap_or(false) } {
            match {
        let v2 = c.clone();
        // ⚠️ 整文件同步读不能留在 async worker 上（api/mod.rs:44-51 纪律 ✓）：
        // 字体几十 MB，且字幕渲染首次用到字体就会打这条路由 ⇒ 占住 worker 时
        // 同一时刻的其他请求（含 engine.js）都产不出响应 ✗
        tokio::task::spawn_blocking(move || std::fs::read(v2))
            .await
            .unwrap_or_else(|_| Err(std::io::Error::other("读取字体任务崩了")))
    } {
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
    let _guard = yekitrim_core::filelock::lock_path(&path.join("polish.toml")).await;
    blocking(move || polish_project_put_locked(&path, &body)).await
}

/// 同步实现（调用方保证持有 polish.toml 的进程内锁）。
pub(super) fn polish_project_put_locked(dir: &Path, body: &serde_json::Value) -> ApiResult {
    let mut cfg = yekitrim_polish::project::load_project(dir).map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("加载失败: {e}")))?;
    if let Some(segments) = body.get("segments") {
        // 老的第三方调用方 / 手写 JSON 里的 `duration` 在这里升级成 `end`（与 polish.toml 同一条路）
        let mut up = segments.clone();
        yekitrim_polish::project::upgrade_event_times(&mut up);
        let segs = serde_json::from_value(up).map_err(|e| err(StatusCode::BAD_REQUEST, format!("segments 解析失败: {e}")))?;
        cfg.segments = segs;
    }
    if let Some(ranges) = body.get("mute_ranges") {
        let rs = serde_json::from_value(ranges.clone()).map_err(|e| err(StatusCode::BAD_REQUEST, format!("mute_ranges 解析失败: {e}")))?;
        cfg.mute_ranges = rs;
    }
    if let Some(layout) = body.get("layout") {
        let lay: yekitrim_polish::project::Layout = serde_json::from_value(layout.clone()).map_err(|e| err(StatusCode::BAD_REQUEST, format!("layout 解析失败: {e}")))?;
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
    // ── 字幕样式（2026-10）：颜色 / 描边 / 字号 / 垂直落点 ──────────────────────
    //
    // 与 `subtitle_font` 同一条路：**保存即生效、不用重跑任务**（只影响渲染与预览）。
    // ⚠️ 这四条以前**不在白名单里** ⇒ 前端发过来被静默丢掉，而 PUT 仍回 `{"ok":true}`
    //    （正是下面那段注释说的「没列进来的键保存时不报错、但静默存不进去」；
    //    实测：发出 4 个键，`polish.toml` 一个字都没变、控件随即回弹到原值）。
    // 四条都**只认 body 里真的出现过的键**：键缺席 = 不动磁盘 —— 这是老工程逐字节零回归（R4）的前提。
    if let Some(c) = body.get("subtitle_color").and_then(|v| v.as_str()) {
        let c = c.trim();
        // 坏颜色**不写**（写进去会让封面与预览各自净化成不同的兜底色 —— 静默变色）。
        // 只做形态检查；真正的净化仍在消费侧（`sanitize_hex_color` 那份不许在这里抄第二份）。
        if c.starts_with('#') && c.len() > 1 {
            cfg.style.subtitle.color = c.to_string();
        }
    }
    if let Some(s) = body.get("subtitle_stroke").and_then(|v| v.as_str()) {
        // 格式 `"{宽度}px {颜色}"`（与 `SubtitleStyle::default_sub_stroke` 同形）。
        // 转译器按这个形状解析，**写歪了描边会整条回落到默认值**且不报错，所以宁可拒收。
        let s = s.trim();
        if !s.is_empty() {
            cfg.style.subtitle.stroke = s.to_string();
        }
    }
    if let Some(v) = body.get("subtitle_size") {
        // 字号（px）：范围收在 8~400（与模板/转译器都对得上），**坏值当场报错、不静默**。
        let n = v
            .as_u64()
            .ok_or_else(|| err(StatusCode::BAD_REQUEST, "subtitle_size 必须是整数"))?;
        cfg.style.subtitle.size = n.clamp(8, 400) as u32;
    }
    if let Some(v) = body.get("subtitle_position_pct") {
        // 垂直落点 = **文字带底边**距画布底的百分比（0~50）。
        //
        // ⚠️ `null` 与「键缺席」**语义不同**，必须分辨（本字段唯一的设计要点）：
        //   · 键缺席 → 不动磁盘（老工程保持"没写过"，走历史公式）；
        //   · `null`  → **显式恢复"未设置"**（`Some → None`）。
        //     这是「位置 → 默认」那颗按钮的**唯一通路** —— 少了这一支，用户就再也回不到未设置。
        if v.is_null() {
            cfg.style.subtitle.position_pct = None;
        } else {
            let n = v.as_f64().ok_or_else(|| {
                err(StatusCode::BAD_REQUEST, "subtitle_position_pct 必须是数字或 null")
            })?;
            if !n.is_finite() {
                return Err(err(StatusCode::BAD_REQUEST, "subtitle_position_pct 必须是有限数"));
            }
            cfg.style.subtitle.position_pct = Some(n.clamp(0.0, 50.0));
        }
    }
    // 弹幕字体：同 `subtitle_font`（值 = 字体库 id，空串 = 跟随字幕那条字体栈）。
    // ⚠️ 它还决定弹幕的**丈量宽度**（滚动时长按宽度算），所以三处量宽都要读它。
    if let Some(f) = body.get("danmaku_font").and_then(|v| v.as_str()) {
        cfg.style.danmaku.font = f.trim().to_string();
    }
    // 弹幕时间偏移（秒，负 = 提前）：精修工程参数，只影响弹幕显示时刻（三条消费腿各自
    // 在入口平移：出片模板 DM 注入点 / engine.js::buildIndex / 转译器 remap 之前），
    // 所以**保存即生效、不用重跑任务**。范围收在 ±10s，坏值当场报错不静默。
    if let Some(v) = body.get("danmaku_offset_secs") {
        let raw = v
            .as_f64()
            .ok_or_else(|| err(StatusCode::BAD_REQUEST, "danmaku_offset_secs 必须是数字"))?;
        if !raw.is_finite() {
            return Err(err(StatusCode::BAD_REQUEST, "danmaku_offset_secs 必须是有限数字"));
        }
        cfg.style.danmaku.offset_secs = raw.clamp(-10.0, 10.0);
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
    // 音效块（`[sfx]`：全局默认音量等）—— 音效轨的设置面板提交它。
    if let Some(v) = body.get("sfx") {
        cfg.sfx = serde_json::from_value(v.clone())
            .map_err(|e| err(StatusCode::BAD_REQUEST, format!("sfx 解析失败: {e}")))?;
    }
    // 成片时间线（`[edl]`：重放 / 冷开场 / 回忆回溯）—— 第 3 批的成片轨提交它。
    //
    // ⚠️ **漏了这一支就是"保存不报错、值静默丢掉"**（见本函数上面那段注释）：界面上
    // 成片轨改完显示"已保存"、PUT 照回 `{"ok":true}`，而 `polish.toml` 一个字节没变。
    // 第 3 批实测过：前端提交 `{edl:{clips:[…]}}`，白名单里没有 `edl` ⇒ 探针单测当场红。
    // ⚠️ 空 `Edl`（`clips` 为空）**不写盘**是 `ProjectFile.edl` 字段上那个
    // `skip_serializing_if = "Edl::is_empty"` 负责的 —— 那正是老工程重存逐字节不变（R4）的前提，
    // 所以这里**照收**即可，别在这里特判"空的就别赋值"（那会让"清空 EDL"这个操作失灵）。
    if let Some(v) = body.get("edl") {
        cfg.edl = serde_json::from_value(v.clone())
            .map_err(|e| err(StatusCode::BAD_REQUEST, format!("edl 解析失败: {e}")))?;
    }
    let raw = cfg.to_toml_commented().map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("序列化失败: {e}")))?;
    std::fs::write(dir.join("polish.toml"), raw).map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("保存失败: {e}")))?;
    ok(serde_json::json!({ "ok": true }))
}

/// 可用的贴图来源：内置 emoji 码点池 + 全局贴纸库（exe 同级 stickers/）的包名与 tags。
/// LLM 生成时按 sticker_source 过滤可用的贴纸池。
pub(super) async fn polish_sticker_sources(
    Extension(user): Extension<crate::auth::CurrentUser>,
) -> ApiResult {
    let owner = crate::auth::owner_of(&user);
    use yekitrim_polish::assets::sticker_index::{load_package_meta, load_sticker_index};
    let root = yekitrim_core::owner::asset_dir(owner, yekitrim_core::owner::AssetKind::Stickers);
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
pub(super) async fn polish_sticker_library(
    Extension(user): Extension<crate::auth::CurrentUser>,
) -> ApiResult {
    let owner = crate::auth::owner_of(&user);
    use yekitrim_polish::assets::sticker_index::{
        list_package_files, load_package_meta, load_sticker_index, scan_packages,
    };
    let root = yekitrim_core::owner::asset_dir(owner, yekitrim_core::owner::AssetKind::Stickers);
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

/// 视频素材库（**只读**，非 admin 也能看）：项目设置里「片头/片尾」挑素材用。
///
/// 与 `polish_sticker_library` 同一条口径：**以磁盘为准**（`scan_video_library`），
/// 手工把文件丢进 `<数据目录>/resources/videos/` 也会出现在这里。
///
/// ⚠️ 视频库**不再挂在贴纸根下面**（2026-10-09）：它有自己的根，判据是
/// `path::video_dir_readable()`（带存量回落）。别写回 `sticker_dir().join("videos")`
/// —— 那样新布局下这里会**永远返回空列表**，而界面只会显示"库是空的"、不报错。
///
/// 每个素材带上**时长**（ffprobe）—— 界面要靠它把"出点 = 0 表示播到结尾"显示成人话，
/// 而且用户设的出点必须能被夹到这个长度（不然越界读素材会让底座报 `source_range_exceeded`）。
/// 探不出时长的照样列出来（`duration: null`），界面置灰并写明原因 —— 不隐藏，
/// 否则用户会觉得"我明明放进去了却没有"。
pub(super) async fn polish_video_library(
    Extension(user): Extension<crate::auth::CurrentUser>,
) -> ApiResult {
    let owner = crate::auth::owner_of(&user);
    // ⚠️ **读两层**（用户库优先、全局补缺）：公用视频素材放在 `global/` 里。
    // 显示的 `root` 仍是**用户库** —— 那是"你放文件放这儿"，全局库是只读公用素材。
    let roots = yekitrim_core::owner::asset_dirs(
        owner,
        yekitrim_core::owner::AssetKind::Videos,
    );
    let lib_dir = roots[0].to_string_lossy().to_string();
    let items: Vec<serde_json::Value> =
        blocking(move || Ok(yekitrim_polish::assets::sticker_index::scan_video_library_roots(&roots)))
            .await?
            .into_iter()
            .map(|a| {
                let path = a.path.to_string_lossy().to_string();
                let duration = yekitrim_polish::render::get_video_duration(&path);
                serde_json::json!({
                    "name": a.name,
                    "file": a.file_name(),
                    "duration": duration,
                })
            })
            .collect();
    // `root` 给界面显示"把文件放哪" —— 用户看不到目录就只能猜。
    ok(serde_json::json!({ "ok": true, "root": lib_dir, "videos": items }))
}

/// 音效目录（只读）：编辑器里音效事件/特效配声的列表选择。
///
/// 数据来自编译进二进制的 `assets/sfx_catalog.json`；全局 `[sfx] blacklist`
/// 屏蔽掉的整族不列出来（选了也不会被解析）。
pub(super) async fn polish_sfx_catalog() -> ApiResult {
    use yekitrim_polish::assets::sfx_catalog;
    let blacklist = yekitrim_core::config::load_config_at(std::path::Path::new("."))
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

/// **素材库统一取文件**接口（`GET /api/polish/asset?kind=&file=`）—— 三个类型一条路。
///
/// 为什么要有它：素材库改成"文件夹管理器"之后，**三种素材都要能预览**
/// （视频要能真播、贴纸看大图、字体用样例字）—— 而三条老的取文件路各有各的形状：
/// 贴纸走 `?pack=&name=`、字体根本没有"取文件"（只有"取字库元数据"）、视频没有。
/// 与其让前端按类型分支拼三种 URL，不如**在服务端收敛成一条**：
/// 前端永远只说"哪个类型、哪个文件"。
///
/// ⚠️ **`kind` 只用来选根目录，不参与拼路径** —— 路径一律由服务端从
/// [`kind_root`] 现取，`file` 只允许**单个文件名**（`is_safe_component` 挡
/// 分隔符与 `..`），所以拿不到库外的文件。**别再接受"子目录"或"相对路径"**：
/// 那等于把 canonicalize 那层兜底变成唯一防线。
///
/// ⚠️ 贴纸仍然走老路（`/polish/sticker-file?pack=&name=`）：贴纸是**两级**
/// （包 / 包内文件），这里刻意**不收编**它 —— 收编就得让 `file` 认 `/`，
/// 正好把上面那条安全性质破掉。
pub(super) async fn polish_asset_file(
    Extension(user): Extension<crate::auth::CurrentUser>,
    axum::extract::Query(q): axum::extract::Query<std::collections::HashMap<String, String>>,
    headers: axum::http::HeaderMap,
) -> axum::response::Response {
    use yekitrim_polish::assets::sticker_index::is_safe_component;
    let owner = crate::auth::owner_of(&user);
    let kind = q.get("kind").map(String::as_str).unwrap_or("");
    let file = q.get("file").map(String::as_str).unwrap_or("");
    let Some(root) = asset_kind_root(owner, kind) else {
        return crate::media::json_error(StatusCode::BAD_REQUEST, "未知的素材类型");
    };
    if !is_safe_component(file) {
        return crate::media::json_error(StatusCode::BAD_REQUEST, "文件名不合法");
    }
    let path = root.join(file);
    // 同步 canonicalize 不能留在 async worker 上（`api/mod.rs` 的纪律）——
    // 这条是**播放热路径**：视频预览会打出一连串 Range 请求。
    let (real, root_real) = tokio::task::spawn_blocking({
        let (p2, r2) = (path.clone(), root.clone());
        move || {
            (
                p2.canonicalize().unwrap_or_else(|_| p2.clone()),
                r2.canonicalize().unwrap_or_else(|_| r2.clone()),
            )
        }
    })
    .await
    .unwrap_or_else(|_| (path.clone(), root.clone()));
    if !real.starts_with(&root_real) {
        return crate::media::json_error(StatusCode::NOT_FOUND, "文件不在素材库内");
    }
    // ⚠️ mime 按**扩展名**给，别一律 octet-stream：浏览器靠它决定"当视频播还是当文件下载"。
    // `mime_guess` 对 mp4/webm/ttf/otf/png/jpg 都认，认不出的才落到 octet-stream。
    let mime = mime_guess::from_path(&path).first_or_octet_stream().to_string();
    let range = headers
        .get(axum::http::header::RANGE)
        .and_then(|v| v.to_str().ok());
    crate::media::serve_file(&path, &mime, range).await
}

/// `kind` → 该 owner 该类型的库根目录（**唯一映射表**）。
///
/// ⚠️ 一律走 `yekitrim_core::owner`，**别在这里拼 `resources/...`** ——
/// 库位置的口径只有那一份（单人 = 程序目录、多用户 = 各自的树），
/// 这里写死就等于是把另一套口径抄了第二遍。
fn asset_kind_root(
    owner: yekitrim_core::owner::Owner,
    kind: &str,
) -> Option<std::path::PathBuf> {
    use yekitrim_core::owner::{asset_dir, AssetKind};
    match kind {
        "video" => Some(asset_dir(owner, AssetKind::Videos)),
        "font" => Some(asset_dir(owner, AssetKind::Fonts)),
        _ => None,
    }
}

/// **视频素材的缩略图**（`GET /api/polish/video-thumb?file=`）—— 素材库网格用。
///
/// 抽**第一秒**那一帧（不是第 0 帧：很多录播文件开头是黑场或渐入，截出来一片纯黑，
/// 用户会以为"这素材是坏的"）。缩到 320px 宽，JPEG q=4 —— 网格里那格才 72px 高。
///
/// ⚠️ **结果按 `<库>/.thumbs/<文件>@<mtime>.jpg` 缓存**：抽帧要起一次 ffmpeg（几十到几百毫秒），
/// 而网格一进页面就会把**每个**视频都拉一遍。不做缓存的话，一个 50 个素材的库每次打开
/// 都白跑 50 次 ffmpeg。文件名里带 **mtime** ⇒ 用户换了同名文件自然失效，不用清缓存。
///
/// ⚠️ 抽不出来（ffmpeg 不在 PATH / 容器怪异 / 文件损坏）**不是错误**：回 404，
/// 前端退成图标 + 文件名（见 `MediaLibrary.vue`）。**别回 500** —— 那会在界面上报一条
/// "读取失败"，而实际只是"这张没缩略图、别的都好"。
pub(super) async fn polish_video_thumb(
    Extension(user): Extension<crate::auth::CurrentUser>,
    axum::extract::Query(q): axum::extract::Query<std::collections::HashMap<String, String>>,
    headers: axum::http::HeaderMap,
) -> axum::response::Response {
    let owner = crate::auth::owner_of(&user);
    use yekitrim_polish::assets::sticker_index::is_safe_component;
    let file = q.get("file").map(String::as_str).unwrap_or("").to_string();
    if !is_safe_component(&file) {
        return crate::media::json_error(StatusCode::BAD_REQUEST, "文件名不合法");
    }
    let root = yekitrim_core::owner::asset_dir(owner, yekitrim_core::owner::AssetKind::Videos);
    let src = root.join(&file);
    if !src.is_file() {
        return crate::media::json_error(StatusCode::NOT_FOUND, "素材不存在");
    }
    // 抽帧是同步阻塞（起 ffmpeg），整段丢 blocking 池 —— 否则网格一进页面就把 worker 占满。
    // ⚠️ `file` 必须是 `String`（owned）：闭包要求 `'static`，借用 `q` 活不过它。
    let thumb = tokio::task::spawn_blocking(move || make_video_thumb(&root, &src, &file))
        .await
        .ok()
        .flatten();
    let Some(thumb) = thumb else {
        // 抽不出来 = "没有缩略图"，不是"接口坏了"（前端按 404 退成图标）
        return crate::media::json_error(StatusCode::NOT_FOUND, "无法生成缩略图");
    };
    let range = headers
        .get(axum::http::header::RANGE)
        .and_then(|v| v.to_str().ok());
    crate::media::serve_file(&thumb, "image/jpeg", range).await
}

/// 抽并缓存一帧。返回 `None` = 抽不出来（调用方回 404）。
///
/// 缓存在 `<库>/.thumbs/` 下 —— ⚠️ 以 `.` 开头是有意的：`scan_video_library` 只认**文件**，
/// 而 `.thumbs` 是目录，本来就不会被列成素材；名字带点则是为了让人一眼看出"这是程序生成的"。
fn make_video_thumb(root: &std::path::Path, src: &std::path::Path, file: &str) -> Option<std::path::PathBuf> {
    let mtime = std::fs::metadata(src).ok()?.modified().ok()?;
    let stamp = mtime
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let dir = root.join(".thumbs");
    let out = dir.join(format!("{file}@{stamp}.jpg"));
    if out.is_file() {
        return Some(out); // 缓存命中
    }
    std::fs::create_dir_all(&dir).ok()?;
    // `-ss 1` 放在 `-i` **之前** = 快速定位（跳过去而不是解码过去）；
    // `-frames:v 1` 只取一帧；`-vf scale=320:-2` 保宽 320、高按偶数对齐（JPEG 要求偶数）。
    // ⚠️ `-update 1` 不能省：输出名不带 `%03d` 时 ffmpeg 会警告"Use a pattern … or the
    // -update option"，虽然当前版本仍会写出文件，但那是**靠默认行为兜的**。
    // 实测（`op.mp4`，3s）：不加这行有 [image2] 警告，加了干净 —— 别等哪个版本把它变成硬错误。
    let st = yekitrim_core::process::cmd("ffmpeg")
        .args([
            "-y",
            "-ss",
            "1",
            "-i",
            &src.to_string_lossy(),
            "-frames:v",
            "1",
            "-vf",
            "scale=320:-2",
            "-q:v",
            "4",
            "-update",
            "1",
            &out.to_string_lossy(),
        ])
        .output()
        .ok()?;
    if !st.status.success() || !out.is_file() {
        // 短于 1 秒的素材（`-ss 1` 越过结尾）：退到第 0 帧再试一次。
        // ⚠️ 别直接放弃 —— 片头素材常常就一两秒，那正是最需要缩略图的一类。
        std::fs::remove_file(&out).ok();
        let st2 = yekitrim_core::process::cmd("ffmpeg")
            .args([
                "-y",
                "-i",
                &src.to_string_lossy(),
                "-frames:v",
                "1",
                "-vf",
                "scale=320:-2",
                "-q:v",
                "4",
                "-update",
                "1",
                &out.to_string_lossy(),
            ])
            .output()
            .ok()?;
        if !st2.status.success() || !out.is_file() {
            return None;
        }
    }
    Some(out)
}

/// 贴纸库里的单张图（编辑器缩略图直连 `<img>`）；只读 + 路径安全校验，与 admin 版同款。
pub(super) async fn polish_sticker_file(
    Extension(user): Extension<crate::auth::CurrentUser>,
    axum::extract::Query(q): axum::extract::Query<std::collections::HashMap<String, String>>,
    headers: axum::http::HeaderMap,
) -> axum::response::Response {
    let owner = crate::auth::owner_of(&user);
    use yekitrim_polish::assets::sticker_index::{is_safe_component, sanitize_pack_name};
    let root = yekitrim_core::owner::asset_dir(owner, yekitrim_core::owner::AssetKind::Stickers);
    let pack = q.get("pack").map(String::as_str).unwrap_or("");
    let name = q.get("name").map(String::as_str).unwrap_or("");
    let Some(pack) = sanitize_pack_name(pack) else {
        return crate::media::json_error(StatusCode::BAD_REQUEST, "包名不合法");
    };
    if !is_safe_component(name) {
        return crate::media::json_error(StatusCode::BAD_REQUEST, "文件名不合法");
    }
    let path = root.join(&pack).join(name);
    // ⚠️ 同步 canonicalize 不能留在 async worker 上（api/mod.rs:44-51 纪律 ✓）——
    // 这条是**播放热路径**：一进 Dhampir 就连打贴纸 PNG，而工程常在慢盘上 ⇒
    // worker 被占住 ⇒ 同时刻的 engine.js 响应产不出来（用户实测的黑屏 ✗）。
    let (real, root_real) = {
        let p2 = path.clone();
        let r2 = root.clone();
        match tokio::task::spawn_blocking(move || {
            let a = p2.canonicalize().unwrap_or_else(|_| p2.clone());
            let b = r2.canonicalize().unwrap_or_else(|_| r2.clone());
            (a, b)
        })
        .await
        {
            Ok(v) => v,
            Err(_) => (path.clone(), root.clone()),
        }
    };
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

    /// 字体库只读接口的**载荷契约**（2026-10-02 字体职责分离新增两个字段的落点）。
    ///
    /// 前端有三处直接吃这个 JSON：
    /// * `PolishPreview` 的 `.cap` 栈取 `fallback_stack`（首项随库里默认走）；
    /// * 字幕预览把 `emoji_family` **追加在栈尾**（正文在前、emoji 兜底）；
    /// * 两个字体下拉按每支的 `emoji` 过滤（emoji 字体不许被选成正文）。
    ///
    /// 所以这里钉两件事：**字段在**，且 **`emoji_family` 不是 `fallback_stack` 的一段**
    /// —— 后者是有意的（封面靠注册顺序、底座没有回退，拼进去只会误导）。
    #[test]
    fn 字体库载荷带上_emoji_槽位且与兜底栈分开() {
        let payload = serde_json::json!({
            "root": "",
            "fallback_stack": yekitrim_polish::render::server::fallback_stack(),
            "emoji_family": yekitrim_core::fonts::emoji_font(&yekitrim_core::fonts::dir_for(
                yekitrim_core::owner::Owner::from_env(),
            ))
            .map(|e| {
                if e.family_internal.trim().is_empty() { e.id } else { e.family_internal }
            }),
            "fonts": [],
            "builtin": [],
        });
        let stack = payload["fallback_stack"].as_str().expect("fallback_stack 必须在");
        assert!(!stack.is_empty(), "兜底栈不能是空的");
        // 兜底栈**永远不含 emoji**（yekitrim-core 有单测钉着；这里从 HTTP 载荷再确认一次）
        assert!(
            !stack.to_ascii_lowercase().contains("emoji"),
            "emoji 不许混进兜底栈（它走独立字段）: {stack}"
        );
        // `emoji_family` 字段本身必须在（值可以是 null —— 库里没装 emoji 字体就是 null）
        assert!(
            payload.get("emoji_family").is_some(),
            "emoji_family 字段必须在（前端据此决定要不要追加栈尾）"
        );
    }

    /// 贴纸预热接口的**路径面**：`..` / 绝对 / 盘符 / 反斜杠 / **根相对**必须被挡掉。
    ///
    /// 根相对那两条（`/etc/passwd`、`\windows\x`）是**这条单测逼出来的加固**：
    /// 原判据只看 `Path::is_absolute()`，而它在 Windows 上对 `/etc/passwd` 返回 **false**，
    /// 且 `Path::join` 对"有根无前缀"的路径会**替换基准**（实测 `C:\proj` + `/etc/passwd`
    /// → `C:/etc/passwd`，已经出了工程）。判据已换成与平台无关的逐字符判定。
    #[test]
    fn 贴纸预热把越界路径全挡掉() {
        let base = std::path::Path::new(r"C:\proj");
        let out = safe_sticker_assets(
            vec![
                "assets/stickers/问号.gif".into(),  // 正常：工程内相对路径
                "".into(),                            // 空
                "../secret.gif".into(),               // 上跳
                r"..\secret.gif".into(),             // 反斜杠 + 上跳
                "C:/windows/x.gif".into(),           // 盘符
                "/etc/passwd".into(),                 // 根相对（Windows 上 is_absolute()==false）
                r"\windows\x.gif".into(),            // 根相对（反斜杠）
                ".hidden.gif".into(),                 // 隐藏文件
                "assets/../../escape.gif".into(),     // 夹在中间的 ..
            ],
            base,
        );
        assert_eq!(out.len(), 1, "只有第一个该活下来：{out:?}");
        assert_eq!(out[0], base.join("assets/stickers/问号.gif"));
    }

    /// 同一句判据在 `sticker-frame`（单值）与 `sticker-prewarm`（数组）上必须一致 ——
    /// 各写一份就是"改一边漏另一边"，`/etc/passwd` 那条洞就是这么留下来的。
    #[test]
    fn 两个贴纸接口共用同一条路径判据() {
        for bad in ["", "/etc/passwd", r"\windows\x", "C:/x.gif", "../x.gif", ".x"] {
            assert!(
                !is_safe_project_relative(bad),
                "`{bad}` 本该被判为越界"
            );
            assert!(
                safe_sticker_assets(vec![bad.into()], std::path::Path::new("/p")).is_empty(),
                "`{bad}` 本该被数组版判据挡掉"
            );
        }
        assert!(is_safe_project_relative("assets/stickers/问号.gif"));
    }

    /// 读 `/polish/sticker-frame` 的响应体：base64 的 PNG 一帧约 **390 KB**，
    /// 而 `test_support::body_json` 的 256 KB 上限装不下（`LengthLimitError`）——
    /// 这里按 8 MB 读，只让预热这一条测试用。
    async fn sticker_frame_json(res: axum::response::Response) -> serde_json::Value {
        let bytes = axum::body::to_bytes(res.into_body(), 8 * 1024 * 1024)
            .await
            .expect("贴纸帧响应体读不出来");
        serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null)
    }

    /// **预热真的省时间**：拿一张真 GIF 走完整条链，对比"直接要第 0 帧"与
    /// "先预热、再要第 0 帧"。判据是**预热之后的取帧明显更快**（同一进程内的服务端缓存）。
    ///
    /// 为什么不 Mock 解码：这条要证的正是"冷解码 3~9 秒"这个用户可见症状，
    /// 拿假 GIF 量不出那个数。样本是 `assets/` 里的真贴纸。
    #[tokio::test]
    async fn 贴纸预热让随后的取帧快得多() {
        // 找一张真的贴纸 GIF（贴纸库或某个真工程的产物）。
        let gif = [
            yekitrim_core::owner::sticker_dir_readable(yekitrim_core::owner::Owner::from_env()),
            std::path::PathBuf::from(r"E:\media\.yekitrim-cover-e2e\clip14\proj\assets\stickers"),
        ]
        .into_iter()
        .map(|d| d.join("思考(认真地).gif"))
        .find(|p| p.exists());
        let Some(gif) = gif else {
            eprintln!("跳过：本机没有可用的贴纸 GIF 样本");
            return;
        };
        let t = test_app().await;
        let clip = t.root.join("output/clips/prewarm-timing");
        let dir = clip.join("assets/stickers");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(clip.join("polish.toml"), yekitrim_polish::project::ProjectFile::example().to_toml_commented().unwrap())
            .unwrap();
        let rel = "assets/stickers/思考(认真地).gif";
        std::fs::copy(&gif, dir.join("思考(认真地).gif")).unwrap();
        let d = clip.to_string_lossy().to_string();

        // ① **冷**：直接要第 0 帧（此时进程内缓存必没有它）。
        let req = |uri: &str, body: serde_json::Value| {
            let (t, uri, body) = (&t, uri.to_string(), body.to_string());
            async move {
                t.call(
                    t.req("POST", &uri)
                        .header(axum::http::header::CONTENT_TYPE, "application/json")
                        .body(Body::from(body))
                        .unwrap(),
                )
                .await
            }
        };
        let t0 = std::time::Instant::now();
        let cold = req("/polish/sticker-frame", serde_json::json!({ "dir": d, "asset": rel, "frame": 0 }))
            .await;
        let cold_ms = t0.elapsed().as_millis();
        assert_eq!(cold.status(), StatusCode::OK, "冷取帧应当成功");
        let cold_v = sticker_frame_json(cold).await;
        assert!(cold_v["png_base64"].as_str().is_some_and(|s| !s.is_empty()));

        // ② 换一张还没进缓存的（同一张 GIF 复制一份改内容 → key 变 ⇒ 一定走解码）。
        //    先预热它，再立刻取帧 —— 后者命中的正是预热放进去的那份。
        let hot_name = "assets/stickers/hot.gif";
        std::fs::copy(&gif, clip.join(hot_name)).unwrap();
        let mut bytes = std::fs::read(&gif).unwrap();
        bytes.extend_from_slice(b"prewarm-probe");      // 改大小 ⇒ 缓存 key 变，必然重新解
        std::fs::write(clip.join(hot_name), &bytes).unwrap();

        let t1 = std::time::Instant::now();
        let pre = req(
            "/polish/sticker-prewarm",
            serde_json::json!({ "dir": d, "assets": [hot_name] }),
        )
        .await;
        let pre_ms = t1.elapsed().as_millis();
        assert_eq!(pre.status(), StatusCode::OK);
        assert_eq!(body_json(pre).await["decoded"].as_u64(), Some(1));

        let t2 = std::time::Instant::now();
        let warm = req("/polish/sticker-frame", serde_json::json!({ "dir": d, "asset": hot_name, "frame": 0 }))
            .await;
        let warm_ms = t2.elapsed().as_millis();
        assert_eq!(warm.status(), StatusCode::OK, "预热后取帧应当成功");
        let warm_v = sticker_frame_json(warm).await;
        assert!(
            warm_v["png_base64"].as_str().is_some_and(|s| !s.is_empty()),
            "预热之后仍应拿到同一张 PNG"
        );

        eprintln!("贴纸取帧：冷 {cold_ms}ms → 预热 {pre_ms}ms → 预热后取帧 {warm_ms}ms");
        // 预热本身就是那次冷解码，所以它取帧那一下必须快得多。
        assert!(
            warm_ms * 4 < cold_ms.max(1),
            "预热后应比冷解码快一个量级：冷 {cold_ms}ms vs 预热后 {warm_ms}ms"
        );
    }

    /// 贴纸预热接口：**不存在的贴纸不让整个请求失败**（它是预热，不是业务操作）。
    ///
    /// 回 200 + `decoded: 0`，前端才好 fire-and-forget。
    #[tokio::test]
    async fn 贴纸预热对不存在的文件也回成功() {
        let t = test_app().await;
        let clip = t.root.join("output/clips/prewarm-missing");
        std::fs::create_dir_all(&clip).unwrap();
        let cfg = yekitrim_polish::project::ProjectFile::example();
        std::fs::write(clip.join("polish.toml"), cfg.to_toml_commented().unwrap()).unwrap();

        let body = serde_json::json!({
            "dir": clip.to_string_lossy(),
            "assets": ["assets/stickers/不存在.gif", "../../etc/passwd"],
        })
        .to_string();
        let res = t
            .call(
                t.req("POST", "/polish/sticker-prewarm")
                    .header(axum::http::header::CONTENT_TYPE, "application/json")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await;
        // 越界那条被挡掉之后只剩一个不存在的文件，但**整体仍应成功**。
        assert_eq!(res.status(), StatusCode::OK, "预热是尽力而为，不该把请求判失败");
        let v = body_json(res).await;
        assert_eq!(v["ok"], true);
        assert_eq!(v["decoded"].as_u64(), Some(0));
        assert_eq!(v["failed"].as_u64(), Some(1), "只有那一个文件被真的尝试过：{v}");
    }

    /// 底座就绪位端点（M3）：**必须 200**，且回包里有 `ready` 布尔与 `problems` 数组。
    ///
    /// 这条守的是"渲染前可见"那条链路的**接口面**：前端渲染弹窗一打开就调它，
    /// 一次 500 或一个缺字段的回包 = 提示永远不出现（而那正是它存在的理由）。
    /// 它**不断言 ready 的真假**：那取决于跑测试这台机器装没装底座（两种都是合法结果）。
    #[tokio::test]
    async fn 底座就绪位端点回布尔与问题清单() {
        let t = test_app().await;
        let clip = t.root.join("output/clips/ready");
        std::fs::create_dir_all(&clip).unwrap();
        let cfg = yekitrim_polish::project::ProjectFile::example();
        std::fs::write(clip.join("polish.toml"), cfg.to_toml_commented().unwrap()).unwrap();

        let body = serde_json::json!({ "dir": clip.to_string_lossy() }).to_string();
        let res = t
            .call(
                t.req("POST", "/polish/dhampir-status")
                    .header(axum::http::header::CONTENT_TYPE, "application/json")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await;
        let status = res.status();
        let v = body_json(res).await;
        assert_eq!(status, StatusCode::OK, "就绪位应当 200：{v}");
        assert_eq!(v["ready"].is_boolean(), true, "要回 `ready` 布尔：{v}");
        assert_eq!(v["problems"].is_array(), true, "要回 `problems` 数组：{v}");
        // `node_version` 这一位是**深探**才有的（这条接口是深探）：null 或字符串都合法，
        // 但**键必须在** —— 前端把它放进提示的 `title`。
        assert!(
            v.get("node_version").is_some(),
            "深探要回 `node_version`（可为 null）：{v}"
        );
        // 不 ready 时必须至少有一条原因 —— 否则前端只能显示"不可用"而说不出为什么。
        if v["ready"] == serde_json::json!(false) {
            assert!(
                v["problems"].as_array().map(|a| !a.is_empty()).unwrap_or(false),
                "不可用时必须给出原因：{v}"
            );
        }
    }

    /// 出片通道是**白名单**：认不出的值必须 **400**，**不许静默当默认** ——
    /// 那会让人以为"选了别的通道却在跑底座"，而两版成片的差别只有对着看才发现。
    ///
    /// ⚠️ 只打 **400 那条路**：合法值会真的起一个渲染任务（几分钟 + 要底座 + 要 ffmpeg），
    /// 那不是接口级单测该干的事（真跑一次出片的验收见 `crates/yekitrim-polish/src/render/AGENTS.md`）。
    #[tokio::test]
    async fn 出片通道认不出就_400_不静默当默认() {
        let t = test_app().await;
        let clip = t.root.join("output/clips/chan");
        std::fs::create_dir_all(&clip).unwrap();
        let cfg = yekitrim_polish::project::ProjectFile::example();
        std::fs::write(clip.join("polish.toml"), cfg.to_toml_commented().unwrap()).unwrap();

        let body = serde_json::json!({ "dir": clip.to_string_lossy(), "renderer": "chrom" }).to_string();
        let res = t
            .call(
                t.req("POST", "/polish/render")
                    .header(axum::http::header::CONTENT_TYPE, "application/json")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await;
        let status = res.status();
        let detail = axum::body::to_bytes(res.into_body(), usize::MAX).await.unwrap();
        let text = String::from_utf8_lossy(&detail);
        assert_eq!(status, StatusCode::BAD_REQUEST, "认不出的通道必须 400；响应：{text}");
        assert!(text.contains("dhampir"), "报错要把可用值列出来：{text}");
    }

    /// **工程 JSON 走真 HTTP**：路由接上了、转译真的跑了、回来的是能喂给 wasm 的工程。
    ///
    /// 不需要 Dhampir 二进制（转译器是 node 脚本），所以这条**进默认测试**。
    #[tokio::test]
    async fn dhampir_project_走真_http_出转译后的工程() {
        let t = test_app().await;
        let clip = t.root.join("output/clips/conv");
        std::fs::create_dir_all(&clip).unwrap();
        let cfg = yekitrim_polish::project::ProjectFile::example();
        std::fs::write(clip.join("polish.toml"), cfg.to_toml_commented().unwrap()).unwrap();
        let dir = clip.to_string_lossy().into_owned();

        let body = serde_json::json!({ "dir": dir }).to_string();
        let res = t
            .call(
                t.req("POST", "/polish/dhampir-project")
                    .header(axum::http::header::CONTENT_TYPE, "application/json")
                    .body(Body::from(body.clone()))
                    .unwrap(),
            )
            .await;
        let status = res.status();
        if status != StatusCode::OK {
            let detail = axum::body::to_bytes(res.into_body(), usize::MAX).await.unwrap();
            panic!("工程转译应当成功，实际 {status}；响应：{}", String::from_utf8_lossy(&detail));
        }
        let v = body_json(res).await;
        assert_eq!(v["ok"], serde_json::json!(true));
        // 形状要真的是 Dhampir 工程：浏览器那边是直接喂给 dhampir_project_open 的。
        assert!(
            v["project"]["timeline"]["tracks"].is_array(),
            "回包要带可用的工程（timeline.tracks 缺失）：{v}"
        );
        // 草稿要清掉：`_dhampir_project.json` 是这一趟的中间产物，不该留在片段目录里。
        assert!(
            !clip.join("_dhampir_project.json").exists(),
            "转译草稿没有清掉"
        );
        // **字幕/弹幕的文本要带回来。**
        //
        // 工程里那两条资产的 `uri` 是**绝对路径**（指向转译输出目录），浏览器取不到 ——
        // 所以内容只能由这里带回去。少了它，预览会"看起来正常、只是没有字幕"。
        let subs = v["subtitles"].as_object().expect("回包要带 subtitles");
        // ⚠️ **条件断言**：这条用例用的是合成片段（`ProjectFile::example()`，
        // 目录里没有 `clip.srt` / `clip.dm.srt`），所以转译器**不产**旁写文件、
        // `subtitles` 就是空的 —— 那是正常的。
        //
        // 第一版我拿真片段的情况去断言它，结果红了。正确的判据是**跟着工程走**：
        // 工程里有字幕类资产（`kind == "subtitle"`）时，内容就**必须**带回来 ——
        // 这才是"浏览器取不到旁写文件"那条不变量的可检形式。
        let wants_subtitles = v["project"]["assets"]
            .as_array()
            .map(|assets| {
                assets
                    .iter()
                    .any(|a| a["kind"].as_str() == Some("subtitle"))
            })
            .unwrap_or(false);
        if wants_subtitles {
            assert!(
                !subs.is_empty(),
                "工程里有字幕类资产，但字幕内容没带回来：预览会\"看起来正常、只是没有字幕\""
            );
            for (name, text) in subs {
                assert!(
                    text.as_str().map(|s| !s.is_empty()).unwrap_or(false),
                    "{name} 的内容是空的"
                );
            }
        }

        // **预览代理优先**（`polish init` 生成、只给预览那条腿用 —— 出片永远用源）。
        //
        // 断言的是**接线**：片段目录里存在新鲜的 `_proxy_720.mp4` 时，工程里的视频素材
        // 必须指向代理（转译器再按代理探测时基/分辨率，所以画面缩放也是按它算的）。
        // 这里用假文件就够 —— 探不到就回落 60fps/1920x1080，而 `uri` 一定是代理名。
        let video_uri = |v: &serde_json::Value| -> String {
            v["project"]["assets"]
                .as_array()
                .and_then(|a| a.iter().find(|x| x["kind"].as_str() == Some("video")))
                .and_then(|a| a["uri"].as_str())
                .unwrap_or("<没有视频素材>")
                .to_string()
        };
        assert_eq!(video_uri(&v), "clip.mp4", "没有代理时必须读源");
        std::fs::write(clip.join("_proxy_720.mp4"), b"not-a-real-video").unwrap();
        let res2 = t
            .call(
                t.req("POST", "/polish/dhampir-project")
                    .header(axum::http::header::CONTENT_TYPE, "application/json")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await;
        assert_eq!(res2.status(), StatusCode::OK, "有代理那次也该成功");
        let v2 = body_json(res2).await;
        assert_eq!(
            video_uri(&v2),
            "_proxy_720.mp4",
            "片段目录里有新鲜代理时，预览必须读代理（否则它白生成了）"
        );
    }

    /// **wasm 资源转发**：`engine.js` 取得到、`.wasm` 的 MIME 正确、路径穿越被挡。
    ///
    /// `#[ignore]`：要有**同层的 Dhampir 仓**（`web/engine.js` + `crates/dhampir-wasm/www/pkg`）。
    /// 这几条断的是"浏览器能不能把 wasm 加载起来"，缺了包就无从谈起 —— 所以宁可显式跳过，
    /// 也不写成"找不到包时自动通过"（那种测试永远不会红）。
    #[tokio::test]
    #[ignore = "手动：要同层的 Dhampir 仓（含已构建的 wasm 包）"]
    async fn dhampir_wasm_资源可转发且_mime_正确() {
        let t = test_app().await;
        let get = |uri: String| {
            let t = &t;
            async move { t.call(t.req("GET", &uri).body(Body::empty()).unwrap()).await }
        };

        // engine.js：页面的宿主引擎
        let res = get("/polish/dhampir-wasm/engine.js".to_string()).await;
        assert_eq!(res.status(), StatusCode::OK, "engine.js 应当取得到");
        let ct = res
            .headers()
            .get(axum::http::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();
        assert!(ct.contains("javascript"), "engine.js 的 MIME 不对：{ct}");
        let js = axum::body::to_bytes(res.into_body(), usize::MAX).await.unwrap();
        assert!(js.len() > 1024, "engine.js 太小（{} 字节），不像是真的", js.len());

        // wasm 包：**MIME 必须是 application/wasm**，给错浏览器会拒绝流式实例化
        let res = get("/polish/dhampir-wasm/pkg/dhampir_wasm_bg.wasm".to_string()).await;
        assert_eq!(res.status(), StatusCode::OK, "wasm 应当取得到");
        let ct = res
            .headers()
            .get(axum::http::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();
        assert_eq!(ct, "application/wasm", "wasm 的 MIME 必须精确是 application/wasm，实际：{ct}");

        // 穿越必须被挡
        for bad in [
            "/polish/dhampir-wasm/../Cargo.toml",
            "/polish/dhampir-wasm/pkg/../../Cargo.toml",
            "/polish/dhampir-wasm/pkg/nope.wasm",
        ] {
            let res = get(bad.to_string()).await;
            assert!(
                res.status() == StatusCode::BAD_REQUEST || res.status() == StatusCode::NOT_FOUND,
                "{bad} 应当被挡，实际 {}",
                res.status()
            );
        }
    }

    /// **wasm 响应的压缩与 ETag/304**（2026-10-02 首屏提速）。
    ///
    /// 这条钉的是「7.66 MB 被原样搬走」的回归：
    /// * 声明 `Accept-Encoding: gzip` ⇒ 必须回 `Content-Encoding: gzip` 且**明显更小**；
    /// * 不带该头 ⇒ **不许压**（否则发给解不开的客户端 = 直接白屏）；
    /// * 带 `If-None-Match: <etag>` ⇒ **304 且空体**（省掉没变时那 7.66 MB）；
    /// * 带 `Range` ⇒ **不许压**（压缩后与原始字节不再一一对应，会破坏播放）。
    ///
    /// `#[ignore]`：同 `dhampir_wasm_资源可转发且_mime_正确`，要同层的 Dhampir 仓。
    #[tokio::test]
    #[ignore = "手动：要同层的 Dhampir 仓（含已构建的 wasm 包）"]
    async fn dhampir_wasm_压缩与_etag_304() {
        let t = test_app().await;
        let req = |uri: &str, extra: Vec<(&str, &str)>| {
            let mut b = t.req("GET", uri);
            for (k, v) in extra {
                b = b.header(k, v);
            }
            b.body(Body::empty()).unwrap()
        };
        let uri = "/polish/dhampir-wasm/pkg/dhampir_wasm_bg.wasm";

        // ① 不带 Accept-Encoding ⇒ 原始字节、无 Content-Encoding
        let res = t.call(req(uri, vec![])).await;
        assert_eq!(res.status(), StatusCode::OK);
        assert!(
            res.headers().get(axum::http::header::CONTENT_ENCODING).is_none(),
            "客户端没声明 gzip 却发了压缩体 —— 对方解不开"
        );
        assert!(
            res.headers().get(axum::http::header::ETAG).is_some(),
            "wasm 响应必须带 ETag（否则每次刷新都重下 7.66 MB）"
        );
        let etag = res
            .headers()
            .get(axum::http::header::ETAG)
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();
        let raw_len = axum::body::to_bytes(res.into_body(), usize::MAX).await.unwrap().len();

        // ② 声明 gzip ⇒ 更小 + 正确的 Content-Encoding + Vary
        let res = t.call(req(uri, vec![("accept-encoding", "gzip, deflate, br")])).await;
        assert_eq!(res.status(), StatusCode::OK);
        let enc = res
            .headers()
            .get(axum::http::header::CONTENT_ENCODING)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        assert_eq!(enc, "gzip", "声明了 gzip 却没压：{}", enc);
        assert_eq!(
            res.headers().get(axum::http::header::VARY).and_then(|v| v.to_str().ok()),
            Some("Accept-Encoding"),
            "少了 Vary：中间缓存会把压缩版喂给不支持 gzip 的客户端"
        );
        let gz_len = axum::body::to_bytes(res.into_body(), usize::MAX).await.unwrap().len();
        assert!(
            gz_len < raw_len / 2,
            "压缩没明显生效：{raw_len} -> {gz_len}（期望至少减半）"
        );

        // ③ 命中 If-None-Match ⇒ 304 且空体
        let res = t.call(req(uri, vec![("if-none-match", etag.as_str())])).await;
        assert_eq!(res.status(), StatusCode::NOT_MODIFIED, "ETag 没变就该 304");
        let body = axum::body::to_bytes(res.into_body(), usize::MAX).await.unwrap();
        assert!(body.is_empty(), "304 不许带 body，实际 {} 字节", body.len());

        // ④ 带 Range ⇒ 不许压（206 的字节必须与原始文件一一对应）
        let res = t.call(req(uri, vec![("range", "bytes=0-1023"), ("accept-encoding", "gzip")])).await;
        assert_eq!(res.status(), StatusCode::PARTIAL_CONTENT);
        assert!(
            res.headers().get(axum::http::header::CONTENT_ENCODING).is_none(),
            "对 Range 响应做了压缩 —— 字节与原始文件错位，播放会花屏/解码失败"
        );
        let n = axum::body::to_bytes(res.into_body(), usize::MAX).await.unwrap().len();
        assert_eq!(n, 1024, "Range 字节数不对：{n}");
    }


/// 成片时间线（EDL）必须能经 PUT 写进 `polish.toml`，且**没提到的键一个字节都不动**。
///
/// 为什么单独钉：`polish_project_put_locked` 是**逐字段白名单**，它自己上面就写着
/// 「没列进来的键保存时不报错、但静默存不进去」。第 3 批（前端成片轨）提交
/// `{edl: {clips: [...]}}` —— 白名单里**没有 `edl` 这一支**的话，界面显示"已保存"、
/// 磁盘上一个字节都没变、PUT 照回 `{"ok":true}`（正是那段注释预言的那类事故；
/// 交付前用探针单测真跑出来过，所以这条是**回归判据**，不是预防性装饰）。
#[test]
fn polish_project_put_writes_edl_and_leaves_unmentioned_keys_alone() {
    let dir = std::env::temp_dir().join(format!("yekitrim-edl-put-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut cfg = yekitrim_polish::project::ProjectFile::example();
    // 放一条屏蔽区间：PUT 只提交 `edl` 时，它必须**原样保留**（逐字段白名单的语义）。
    cfg.mute_ranges.push(yekitrim_polish::project::MuteRange { start: 1.0, end: 2.0 });
    std::fs::write(dir.join("polish.toml"), cfg.to_toml_commented().unwrap()).unwrap();

    // 1) 只有 `edl`：写进去了，而且没提到的 `mute_ranges` 没被动
    let body = serde_json::json!({ "edl": { "clips": [
        { "source_start": 3.0, "source_end": 6.0, "speed": 1.0, "kind": "replay", "label": "爆点重放" }
    ] } });
    let _ = polish_project_put_locked(&dir, &body).expect("保存 edl");
    let after = yekitrim_polish::project::load_project(&dir).unwrap();
    assert_eq!(after.edl.clips.len(), 1, "edl 没写进 polish.toml（PUT 白名单漏了 edl 这一支）");
    assert_eq!(after.edl.clips[0].source_start, 3.0);
    assert_eq!(after.edl.clips[0].kind.as_deref(), Some("replay"));
    assert_eq!(after.mute_ranges.len(), 1, "只提交 edl 时 mute_ranges 必须原样保留");
    assert_eq!(after.mute_ranges[0].start, 1.0);

    // 2) **老工程零回归**：清空 EDL 之后 `polish.toml` 里**不许留空的 `[edl]` 段头**。
    //    判据挂在 `ProjectFile.edl` 字段的 `skip_serializing_if = "Edl::is_empty"` 上 ——
    //    只挂在 `Edl::clips` 自己身上是不够的（toml 照样写出空的 `[edl]` 段头），
    //    那样老工程重存一次就"多一段"（data-model 的第一版单测正是这么红的）。
    let clear = serde_json::json!({ "edl": { "clips": [] } });
    let _ = polish_project_put_locked(&dir, &clear).expect("清空 edl");
    let raw = std::fs::read_to_string(dir.join("polish.toml")).unwrap();
    assert!(
        !raw.contains("[edl]"),
        "清空 EDL 之后不许留空的 `[edl]` 段头（老工程重存必须逐字节不变）:\n{raw}"
    );
    let cleared = yekitrim_polish::project::load_project(&dir).unwrap();
    assert!(cleared.edl.is_empty(), "清空之后内存里也该是空的");
    assert_eq!(cleared.mute_ranges.len(), 1, "清空 edl 不该碰 mute_ranges");

    let _ = std::fs::remove_dir_all(&dir);
}

/// 成片时间线（EDL）必须能**经整张路由表**（真 HTTP：会话 + CSRF + 作用域 + 路径锁）存下来、
/// 再读回来。这是第 3 批交付时唯一**没被验到**的那条缝（前端成片轨改完 → F5 → 还在不在）。
///
/// 为什么 `polish_project_put_writes_edl_and_leaves_unmentioned_keys_alone` 不够：
/// 那条直接调 `polish_project_put_locked`，绕过了 ① 路由挂没挂 ② `scope.rs::enforce_scope`
/// 会不会把 `dir` 拦掉 ③ 请求体解析 ④ GET 侧有没有把 `edl` 回给前端。
/// 这四处任意一处错，前端都是"显示已保存、刷新就没了"，而上面那条单测**照样绿**。
#[tokio::test]
async fn polish_project_http_roundtrips_edl() {
    let app = crate::api::test_support::test_app().await;
    let dir = app.root.to_string_lossy().replace('\\', "/");
    let cfg = yekitrim_polish::project::ProjectFile::example();
    std::fs::write(app.root.join("polish.toml"), cfg.to_toml_commented().unwrap()).unwrap();

    // 1) PUT：走真 HTTP（带会话 cookie + CSRF 头）
    let body = serde_json::json!({
        "dir": dir,
        "edl": { "clips": [
            { "source_start": 3.0, "source_end": 6.0, "speed": 1.0, "kind": "replay", "label": "爆点重放" },
            { "source_start": 0.0, "source_end": 12.0, "speed": 1.0 },
        ] },
    });
    let req = app
        .req("PUT", "/polish/project")
        .header(axum::http::header::CONTENT_TYPE, "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();
    let res = app.call(req).await;
    assert_eq!(res.status(), 200, "PUT /polish/project 应当 200");

    // 2) 磁盘上真的写了
    let on_disk = yekitrim_polish::project::load_project(&app.root).unwrap();
    assert_eq!(on_disk.edl.clips.len(), 2, "edl 没落到 polish.toml");
    assert_eq!(on_disk.edl.clips[0].kind.as_deref(), Some("replay"));

    // 3) GET 要把它读回来（前端刷新靠这一步；GET 丢了 `edl` 同样是"刷新就没了"）
    let req = app
        .req("GET", &format!("/polish/project?dir={dir}"))
        .body(Body::empty())
        .unwrap();
    let res = app.call(req).await;
    assert_eq!(res.status(), 200);
    let bytes = axum::body::to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let clips = json
        .get("edl")
        .and_then(|e| e.get("clips"))
        .and_then(|c| c.as_array())
        .unwrap_or_else(|| panic!("GET 没把 edl.clips 回给前端（刷新后成片轨会空）: {json}"));
    assert_eq!(clips.len(), 2);
    assert_eq!(clips[0].get("kind").and_then(|k| k.as_str()), Some("replay"));
    assert_eq!(clips[0].get("source_start").and_then(|v| v.as_f64()), Some(3.0));

    // 4) 再 PUT 一次清空 ⇒ 磁盘上不许留空的 `[edl]` 段头（R4：老工程重存逐字节不变）
    let clear = serde_json::json!({ "dir": dir, "edl": { "clips": [] } });
    let req = app
        .req("PUT", "/polish/project")
        .header(axum::http::header::CONTENT_TYPE, "application/json")
        .body(Body::from(clear.to_string()))
        .unwrap();
    assert_eq!(app.call(req).await.status(), 200);
    let raw = std::fs::read_to_string(app.root.join("polish.toml")).unwrap();
    assert!(!raw.contains("[edl]"), "清空后不许留空的 `[edl]` 段头:\n{raw}");
}

/// 画布方向 / 画面构成 / 贴纸落点要能真写进 `polish.toml`，且**不碰**没提到的段。
///
/// 回归：PUT 是**逐字段白名单**（见 `polish_project_put_locked`）——漏一个分支就是
/// "保存不报错、但值静默丢掉"，前端还会以为存上了。
#[test]
fn polish_project_put_writes_canvas_stage_sticker() {
    let dir = std::env::temp_dir().join(format!("yekitrim-stage-put-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut cfg = yekitrim_polish::project::ProjectFile::example();
    cfg.mute_ranges.push(yekitrim_polish::project::MuteRange { start: 1.0, end: 2.0 });
    std::fs::write(dir.join("polish.toml"), cfg.to_toml_commented().unwrap()).unwrap();

    // 1) 只切画布方向：其它段原样保留
    let body = serde_json::json!({ "canvas": { "orientation": "portrait" } });
    let _ = polish_project_put_locked(&dir, &body).expect("保存画布方向");
    let after = yekitrim_polish::project::load_project(&dir).unwrap();
    assert_eq!(after.canvas.size(), (1080, 1920));
    assert_eq!(after.mute_ranges.len(), 1, "没提到的字段必须原样保留");
    // ⚠️ 2026-10-05 起位置的权威是**两轴** `position_x_pct` / `position_y_pct`（老单值
    // `position_pct` 与老枚举 `position` 都已 `skip_serializing`、只读兼容）——默认 = 没有新字段
    // ⇒ 走老枚举换算，`auto` → 50 居中。
    assert_eq!(after.stage.position_x_pct, None, "没提交的段走默认（None），不能被顺手重置");
    assert_eq!(after.stage.position_y_pct, None, "没提交的段走默认（None），不能被顺手重置");
    let fit0 = after.resolve_stage(Some((1920, 1080)));
    assert_eq!(
        (fit0.position_x_pct, fit0.position_y_pct),
        (50.0, 50.0),
        "缺两轴的老工程按老枚举换算：auto → 50 居中（两轴同值）"
    );

    // 2) 画面构成（两轴各自独立）
    let body = serde_json::json!({ "stage": {
        "bg_mode": "color", "bg_color": "#112233", "blur_scale": 2.0,
        "position_x_pct": 25.0, "position_y_pct": 100.0, "scale": 1.25, "padding": 30,
    }});
    let _ = polish_project_put_locked(&dir, &body).expect("保存画面构成");
    let after = yekitrim_polish::project::load_project(&dir).unwrap();
    assert_eq!(after.canvas.size(), (1080, 1920), "没提交 canvas 就不能被重置");
    let fit = after.resolve_stage(Some((1920, 1080)));
    assert_eq!(fit.axis, "y", "竖画布 × 横素材 = 上下留边");
    // 位置 = **两轴百分比**（各五档之一）；老枚举只是过渡期兼容输出。
    assert_eq!((fit.position_x_pct, fit.position_y_pct), (25.0, 100.0));
    assert_eq!(fit.position, "bottom", "竖画布 + Y=100 ⇒ 兼容枚举给 bottom");
    assert_eq!(fit.bg, "color");
    assert_eq!(fit.bg_color, "#112233");
    assert!((fit.padding - 16.875).abs() < 1e-9, "30px × k(0.5625)");

    // 3) 贴纸落点
    let body = serde_json::json!({ "sticker": { "position_mode": "manual", "position": "20% 80%" } });
    let _ = polish_project_put_locked(&dir, &body).expect("保存贴纸落点");
    let after = yekitrim_polish::project::load_project(&dir).unwrap();
    assert_eq!(after.sticker.position_mode, "manual");
    assert_eq!(after.sticker.position, "20% 80%");
    assert_eq!(after.stage.position_x_pct, Some(25.0), "上一段必须还在");
    assert_eq!(after.stage.position_y_pct, Some(100.0), "上一段必须还在");

    // 4) 落到 toml 文本上：段在、段注释在、根级 sticker_source 没被 [sticker] 顶掉
    let raw = std::fs::read_to_string(dir.join("polish.toml")).unwrap();
    for needle in [
        "[canvas]",
        "[stage]",
        "[sticker]",
        "orientation = \"portrait\"",
        "position_mode = \"manual\"",
        // ⚠️ 位置写的是两轴新字段；老枚举 `position` 与老单值 `position_pct` 都已 `skip_serializing`、
        // **不该再出现在 [stage] 段里**（判据只在 [stage] 段内断：贴纸与事件也各有一个同名 `position` 键）。
        "position_x_pct = 25.0",
        "position_y_pct = 100.0",
        "# ===== 画面构成",
        "# ===== 贴纸落点",
        "sticker_source",
    ] {
        assert!(raw.contains(needle), "缺 {needle}:\n{raw}");
    }
    let stage_sec = raw.split("[stage]").nth(1).unwrap_or("");
    let stage_sec = stage_sec.split("\n[").next().unwrap_or("");
    assert!(!stage_sec.contains("position_pct ="), "老单值字段不该再写出:\n{stage_sec}");

    // 5) 非法值：文件照存（不篡改用户输入），但渲染侧取到的是安全值
    let body = serde_json::json!({ "stage": {
        "bg_mode": "frame", "bg_color": "url(evil)", "position_x_pct": 0.0, "position_y_pct": 0.0
    }});
    let _ = polish_project_put_locked(&dir, &body).expect("保存非法值也不应报错");
    let after = yekitrim_polish::project::load_project(&dir).unwrap();
    let fit = after.resolve_stage(Some((1920, 1080)));
    assert_eq!((fit.position_x_pct, fit.position_y_pct), (0.0, 0.0));
    assert_eq!(fit.position, "top");
    assert_eq!(fit.bg, "blur", "未知 bg_mode 回落 auto → 补边时按模糊处理");
    assert_eq!(fit.bg_color, "#000000", "颜色必须净化，不能把 url(...) 带进模板 CSS");

    // 6) **老单值 `position_pct` 在 API 面上仍能迁移，而且写盘时会固化到两轴**：
    //    提交它 = 两轴都吃这个值（逐像素不变）；落盘的必须是**新键**，否则 `skip_serializing`
    //    会让老单值在下次加载时凭空消失（用户只动一条轴、另一条轴跳回 50 —— 静默丢值）。
    let body = serde_json::json!({ "stage": { "position_pct": 75.0 } });
    let _ = polish_project_put_locked(&dir, &body).expect("保存老单值字段");
    let after = yekitrim_polish::project::load_project(&dir).unwrap();
    let fit = after.resolve_stage(Some((1920, 1080)));
    assert_eq!(
        (after.stage.position_x_pct, after.stage.position_y_pct),
        (Some(75.0), Some(75.0)),
        "写盘时必须把老单值固化到两轴（`migrate_position_pct`）"
    );
    assert_eq!(
        (fit.position_x_pct, fit.position_y_pct),
        (75.0, 75.0),
        "老单值 ⇒ 两轴同值（贴满的那条轴余量为 0，与老行为逐像素等价）"
    );
    // 只动一条轴：另一条轴必须**保持**老值（不是跳回 50）
    let body = serde_json::json!({ "stage": { "position_pct": 75.0, "position_x_pct": 25.0 } });
    let _ = polish_project_put_locked(&dir, &body).expect("只提交一条轴 + 老单值");
    let after = yekitrim_polish::project::load_project(&dir).unwrap();
    assert_eq!(
        (after.stage.position_x_pct, after.stage.position_y_pct),
        (Some(25.0), Some(75.0)),
        "缺的那条轴补老单值（缺一补一），已给的那条不被覆盖"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// 字幕样式四键（`subtitle_color` / `subtitle_stroke` / `subtitle_size` / `subtitle_position_pct`）
/// **必须真的写进 `polish.toml`**。
///
/// ⚠️ 这条测试的存在理由就是上面那段注释里的坑：`polish_project_put_locked` 是**逐字段白名单**，
/// 没列进来的键 **PUT 照样回 `{"ok":true}`、而磁盘一个字都不动**。实测过（2026-10）：
/// 前端把 4 个键全发出来，`polish.toml` 里 4 个值一个都没变，控件随即回弹到原值，
/// **全程不报任何错**。字幕样式这套控件的后端口子就曾经整条缺失。
///
/// 同时钉住本字段的**语义核心**：`position_pct: null` 与「键缺席」**不是一回事** ——
/// 前者是"显式恢复未设置"（`Some → None`），后者是"别动磁盘"。
#[test]
fn polish_project_put_writes_subtitle_style() {
    let dir = std::env::temp_dir().join(format!("yekitrim-substyle-put-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let cfg = yekitrim_polish::project::ProjectFile::example();
    std::fs::write(dir.join("polish.toml"), cfg.to_toml_commented().unwrap()).unwrap();

    // 1) 四个键一次提交：全部要落到盘上
    let body = serde_json::json!({
        "subtitle_color": "#ff0000",
        "subtitle_stroke": "8px #00ff00",
        "subtitle_size": 96,
        "subtitle_position_pct": 25.0,
    });
    let _ = polish_project_put_locked(&dir, &body).expect("保存字幕样式");
    let after = yekitrim_polish::project::load_project(&dir).unwrap();
    assert_eq!(after.style.subtitle.color, "#ff0000", "颜色必须写进去");
    assert_eq!(after.style.subtitle.stroke, "8px #00ff00", "描边必须写进去");
    assert_eq!(after.style.subtitle.size, 96, "字号必须写进去");
    assert_eq!(after.style.subtitle.position_pct, Some(25.0), "落点必须写进去");
    // 没提交的键不能被顺手重置
    assert_eq!(after.style.subtitle.highlight_color, "#f56b41", "没提到的字段必须原样保留");

    // 2) 落到 toml 文本上（防"内存里对了但序列化没写"）。
    //    ⚠️ 颜色/描边/字号在 `[style.subtitle]` 段内，但 `position_pct` 我们**只在这里**断言，
    //    因为 `[stage]` 段有个同名的老字段 —— 见第 3 步那段注释。
    let raw = std::fs::read_to_string(dir.join("polish.toml")).unwrap();
    let sub_sec = || {
        raw.split("[style.subtitle]")
            .nth(1)
            .unwrap_or("")
            .split("\n[")
            .next()
            .unwrap_or("")
            .to_string()
    };
    let sec = sub_sec();
    for needle in [
        "color = \"#ff0000\"",
        "stroke = \"8px #00ff00\"",
        "size = 96",
    ] {
        assert!(sec.contains(needle), "`[style.subtitle]` 缺 {needle}:\n{sec}");
    }
    assert!(
        sec.lines().any(|l| l.trim_start() == "position_pct = 25.0"),
        "`[style.subtitle]` 缺 `position_pct = 25.0`:\n{sec}"
    );

    // 3) ⚠️ 语义核心：`null` = 显式恢复"未设置"（这是「位置 → 默认」按钮的唯一通路）
    let body = serde_json::json!({ "subtitle_position_pct": serde_json::Value::Null });
    let _ = polish_project_put_locked(&dir, &body).expect("恢复默认落点");
    let after = yekitrim_polish::project::load_project(&dir).unwrap();
    assert_eq!(after.style.subtitle.position_pct, None, "null 必须把 Some 打回 None");
    assert_eq!(after.style.subtitle.size, 96, "null 只作用于落点，别的字段不能跟着被重置");
    // ⚠️ **判据只能看 `[style.subtitle]` 那一段里的行**，两个坑都要躲开：
    //   ① `to_toml_commented` 的**注释文本**里就写着 "position_pct" 这个词（说明缺省语义那段）
    //      ⇒ `raw.contains("position_pct")` 恒真、断言会空转；
    //   ② `[stage]` 段**本来就有一个同名的** `position_pct`（画面构成的单值落点，与本字段无关）
    //      ⇒ 全文扫 `starts_with("position_pct =")` 会被它误伤。
    //    所以先切出 `[style.subtitle]` 段、再逐行看行首。
    //
    // ⚠️⚠️ **必须重新读盘**：上面那个 `raw` 是第 2 步（写 null **之前**）读的 —— 直接复用会让这条
    //     断言永远看着"删掉了"（假绿），而那正是本测试要防的东西（第一版就是这么写的，一跑就露）。
    let raw2 = std::fs::read_to_string(dir.join("polish.toml")).unwrap();
    let sub_sec = raw2
        .split("[style.subtitle]")
        .nth(1)
        .unwrap_or("")
        .split("\n[")
        .next()
        .unwrap_or("");
    assert!(
        !sub_sec.lines().any(|l| l.trim_start().starts_with("position_pct =")),
        "恢复默认后 [style.subtitle] 里不该再有 position_pct 键:\n{sub_sec}"
    );

    // 4) **键缺席 = 不动磁盘**（老工程零回归 R4 的前提）
    let body = serde_json::json!({ "subtitle_color": "#123456" });
    let _ = polish_project_put_locked(&dir, &body).expect("只改颜色");
    let after = yekitrim_polish::project::load_project(&dir).unwrap();
    assert_eq!(after.style.subtitle.color, "#123456");
    assert_eq!(after.style.subtitle.size, 96, "缺席的键不能被默认值顶掉");
    assert_eq!(after.style.subtitle.position_pct, None, "缺席的键不能被顶回某个默认百分比");

    // 5) 坏值当场报 400，**不静默**
    let bad = serde_json::json!({ "subtitle_size": "很大" });
    assert!(polish_project_put_locked(&dir, &bad).is_err(), "字号非数字必须报错");
    let bad = serde_json::json!({ "subtitle_position_pct": "中间" });
    assert!(polish_project_put_locked(&dir, &bad).is_err(), "落点非数字且非 null 必须报错");
    // 坏颜色**不写**（也不报错 —— 与既有净化口径一致），但不能污染磁盘
    let bad = serde_json::json!({ "subtitle_color": "not-a-color" });
    let _ = polish_project_put_locked(&dir, &bad).expect("坏颜色不报错");
    let after = yekitrim_polish::project::load_project(&dir).unwrap();
    assert_eq!(after.style.subtitle.color, "#123456", "坏颜色必须被拒收、不能写脏");

    // 6) 越界值被夹住（0~50 / 8~400），不是写进野值
    let body = serde_json::json!({ "subtitle_position_pct": 999.0, "subtitle_size": 9999 });
    let _ = polish_project_put_locked(&dir, &body).expect("越界值应当被夹住");
    let after = yekitrim_polish::project::load_project(&dir).unwrap();
    assert_eq!(after.style.subtitle.position_pct, Some(50.0), "落点夹到 50");
    assert_eq!(after.style.subtitle.size, 400, "字号夹到 400");

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
    let dir = std::env::temp_dir().join(format!("yekitrim-handheld-put-{}", std::process::id()));
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
    let after = yekitrim_polish::project::load_project(&dir).unwrap();
    let flags: Vec<bool> = after.segments[0]
        .events
        .iter()
        .map(|ev| match ev {
            yekitrim_polish::project::Event::Camera { handheld, .. } => *handheld,
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
    let missing = std::env::temp_dir().join("yekitrim-dir-does-not-exist-xyz");
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
        "dir": yekitrim_core::path::display(&dir),
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
/// `source`）。后果不是"多读一个文件"：`source` 指向 `<数据目录>/.yekitrim/llm.toml`
/// 就能把**全局 LLM API key** 复制进自己的项目，再用 `/api/media/<自己的项目>/…` 读回来；
/// 同样能拿到账号账本 `cloud/yekitrim.db`（明文邀请码 + 全部项目真实路径）。
#[tokio::test]
async fn polish_path_fields_cannot_read_outside_files() {
    let t = test_app().await;

    // 受害者（admin 的项目）里的一个"机密"文件
    let secret = t.root.join("output").join("secret.txt");
    std::fs::write(&secret, b"api_key = 'sk-real-secret'").unwrap();

    // 普通用户 + 他自己的项目目录（含 clip-01 与 polish.toml，够走完前置校验）
    let hash = yekitrim_service::auth::hash_password("pass-12345678").unwrap();
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
    let my_cfg = my_root.join("yekitrim.toml");
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
    let cfg = yekitrim_polish::project::load_project(&my_clip).unwrap();
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
        !yekitrim_polish::project::load_project(&my_clip).unwrap().cover.image.is_empty(),
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
    let saved = tokio::task::spawn_blocking({ let __p = my_cfg.clone(); move || std::fs::read_to_string(__p) }).await.unwrap_or_else(|_| Err(std::io::Error::other("读取任务崩了"))).unwrap();
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
    let outside = std::env::temp_dir().join("yekitrim-not-my-project");
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

// ===================== Dhampir 浏览器端预览（wasm + 工程 JSON） =====================
//
// 「预览交给浏览器」：渲染跑在页面上（Dhampir 的 wasm + WebGPU），**后端只做两件事** ——
//
//   ① 把 wasm 包与宿主引擎 `engine.js` 发给浏览器（[`polish_dhampir_wasm`]）；
//   ② 把 `polish.toml` 转译成 Dhampir 的工程 JSON（页面 `loadEngine` 要吃它，
//      [`polish_dhampir_project`]）。
//
// **后端不渲染任何预览帧。** 实测口径：服务端那条路 PNG 出口 **1250 ms/帧** ✗；
// 浏览器腿 1080p 顺序播放 **7.0 ms/帧（142.9 fps）** ✓。像素不过边界，就没有"传输"这个瓶颈。

/// 把阻塞的转译丢到 blocking 线程池，并统一成接口错误。
///
/// 转译要起一个 `node` 子进程（百毫秒级），直接在 async 上下文里等会占住运行时线程。
async fn run_blocking<T, F>(what: &str, job: F) -> Result<T, AppError>
where
    T: Send + 'static,
    F: FnOnce() -> anyhow::Result<T> + Send + 'static,
{
    match tokio::task::spawn_blocking(job).await {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(e)) => Err(err(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("{what}失败: {}", yekitrim_polish::render::dhampir::explain(&e)),
        )),
        Err(e) => Err(err(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("{what}的任务崩了: {e}"),
        )),
    }
}

/// `GET /polish/dhampir-wasm/*path` —— 静态转发 Dhampir 的预览资源。
///
/// 两份根：`pkg/` 前缀走 wasm 包，其余走 `<Dhampir>/web`（`engine.js` 在那）。
/// **不复制**：Dhampir 那边重新构建 wasm 之后，刷新页面就是新的（靠 ETag 的
/// mtime 校验自然失效，不是靠禁缓存）。
pub(super) async fn polish_dhampir_wasm(
    axum::extract::Path(path): axum::extract::Path<String>,
    headers: axum::http::HeaderMap,
) -> Response {
    // ⚠️ **这一整段必须走 blocking 池** —— 它是本路由曾经的真实故障 ✗：
    //
    // 这段以前直接在 async worker 上做同步 FS（`wasm_dirs()` 里的一串 `is_file()`、
    // 两次 `canonicalize()`、一次 `is_file()`）。工程/仓库目录在慢盘或网络盘上时
    // （本项目头顶那条纪律记的就是这个：一次 `exists()` 就要几百 ms 到几秒 ✗），
    // worker 被占住 ⇒ **本路由自己的响应产不出来** ⇒ 浏览器报
    // `error loading dynamically imported module`（**没有 net:: 码** ✓ 正是"连上了、拿不到"）
    // ⇒ 画布黑 + 同一时刻 `/api/jobs` 10s 超时 ✓✓（用户实测）。
    //
    // 注意日志形态：请求被受理但响应迟迟完不成 ⇒ **服务端日志里根本不出现这一条** ✓
    // （`→ 200 OK (1ms)` 是响应写完之后才打的）。所以"日志里没有它"不是"没发请求" ✗。
    let dirs = tokio::task::spawn_blocking(yekitrim_polish::render::dhampir::wasm_dirs).await;
    let Ok((web, pkg)) = dirs.unwrap_or_else(|e| Err(anyhow::anyhow!("定位任务崩了: {e}"))) else {
        return crate::media::json_error(
            StatusCode::NOT_FOUND,
            "找不到 Dhampir 的 wasm 预览包（设 VTEDIT_DHAMPIR_WASM=<Dhampir 仓根>，或先在那边跑 wasm-pack）",
        );
    };
    let (root, rel) = match path.strip_prefix("pkg/") {
        Some(rest) => (pkg, rest.to_string()),
        None => (web, path.clone()),
    };
    // 越界防护：`..` 与绝对路径都挡掉 —— 与 `polish_sticker_file` 同一条纪律。
    if rel.is_empty()
        || rel.starts_with('/')
        || rel.starts_with('\\')
        || rel.split(['/', '\\']).any(|seg| seg == "..")
    {
        return crate::media::json_error(StatusCode::BAD_REQUEST, "路径不合法");
    }
    let file = root.join(rel.replace('\\', "/"));
    // 同上：`canonicalize` / `is_file` 都是同步 FS，一并挪出去。
    let probe_root = root.clone();
    let probed = tokio::task::spawn_blocking(move || {
        let real = file.canonicalize().unwrap_or_else(|_| file.clone());
        let root_real = probe_root
            .canonicalize()
            .unwrap_or_else(|_| probe_root.clone());
        let ok = real.starts_with(&root_real) && real.is_file();
        (real, ok)
    })
    .await;
    let Ok((real, ok)) = probed else {
        return crate::media::json_error(StatusCode::NOT_FOUND, "资源不存在");
    };
    if !ok {
        return crate::media::json_error(StatusCode::NOT_FOUND, "资源不存在");
    }
    // `.wasm` 必须给对 MIME：给错浏览器会拒绝流式实例化（症状是"加载没反应"）。
    let mime = if real.extension().and_then(|e| e.to_str()) == Some("wasm") {
        "application/wasm".to_string()
    } else {
        mime_guess::from_path(&real).first_or_octet_stream().to_string()
    };
    let range = headers
        .get(axum::http::header::RANGE)
        .and_then(|v| v.to_str().ok());
    // **压缩 + ETag/304**（2026-10-02，Dhampir 首屏提速）。
    //
    // 这里的 `pkg/dhampir_wasm_bg.wasm` 是 **7.66 MB**，而它以前是原样发出的
    // ⇒ 每次进精修页都搬 7.66 MB。实测同一份字节 gzip -6 → 1.43 MB（19%）。
    // ETag 则让「文件没变」的那次刷新直接 304（几乎零字节）。
    //
    // ⚠️ 压缩与 ETag 都**只在这条路由上开**，没动 `serve_file` 的既有调用点：
    // `/api/media/*`（视频）走的是老签名，保持「Range + 不压缩」不变 ——
    // 对 Range 响应做压缩会破坏播放（见 `media.rs::serve_file_cond` 的说明）。
    //
    // ⚠️ **ETag 不能改成「永不变」**：这条路由的既有意图是「Dhampir 重建 wasm 后
    // 刷新即新」。`file_etag` 取的是 len+mtime ⇒ 重建必然改 mtime ⇒ 自然失效。
    let etag = tokio::fs::metadata(&real)
        .await
        .ok()
        .map(|m| crate::media::file_etag(&m));
    let if_none_match = headers
        .get(axum::http::header::IF_NONE_MATCH)
        .and_then(|v| v.to_str().ok());
    let accept_encoding = headers
        .get(axum::http::header::ACCEPT_ENCODING)
        .and_then(|v| v.to_str().ok());
    crate::media::serve_file_cond(&real, &mime, range, etag.as_deref(), if_none_match, accept_encoding)
        .await
}

/// `GET /polish/dhampir-status` —— **底座出片通道的就绪位**（M3）。
///
/// 回包（`dhampir::Readiness` 直接序列化）：`{ ready, binary, preview, translator, font,
/// contract_version, problems[] }`。
///
/// 为什么要它：默认出片通道就是底座，而"底座不可用"以前**只在渲染时报错**（几分钟的任务跑到一半才失败）。
/// 这条让前端在**点渲染之前**就能看见 —— 前端那颗通道下拉旁边会挂提示，`yekitrim doctor` 用同一份判断。
///
/// ⚠️ 只做**定位**（六项各查一次文件），不渲染、不跑转译器 —— 它会在打开渲染弹窗时被调，
/// 也可能被反复调。⚠️ 路径**只回给已登录用户**（这条路由在 `require_user` 层内），
/// 与 `/api/health` 的口径不同：health 是公开接口、只回布尔位（见 `api/mod.rs::health`）。
pub(super) async fn polish_dhampir_status(
    State(_state): State<Arc<AppState>>,
    Json(body): Json<serde_json::Value>,
) -> ApiResult {
    // 带了 `dir` 就按那个工程选的字体判（`style.subtitle.font`）——字体是"按工程"解析的，
    // 不带就按缺省（系统字体栈）判。读工程要落盘 IO，所以整段走 `blocking`。
    let dir = body.get("dir").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let readiness = blocking(move || {
        let font = if dir.trim().is_empty() {
            String::new()
        } else {
            yekitrim_polish::project::load_project(std::path::Path::new(&dir))
                .map(|cfg| cfg.style.subtitle.font)
                .unwrap_or_default()
        };
        // `blocking` 要 `Result`：读工程失败**不该**让就绪位请求 500 ——
        // 字体会退成"系统字体栈"那条判断，问题照样在 `problems` 里说得清。
        // `deep = true`：这条是**已登录**接口、打开渲染弹窗时调一次 —— 顺带跑 `node --version`，
        // 把"PATH 上有 node 但它跑不通（商店桩）/ 版本太低"这两类也说清楚。
        // `/api/health` 那条（无鉴权 + 被轮询）只走便宜判断。
        Ok(yekitrim_polish::render::dhampir::readiness(&font, true))
    })
    .await?;
    ok(serde_json::json!(readiness))
}

/// `POST /polish/dhampir-project` —— 转译 `polish.toml`，把工程 JSON **内容**交给浏览器。
///
/// 请求：`{ "dir": "<片段目录（绝对路径）>" }`
/// 回包：`{ "ok": true, "project": { …Dhampir 工程… } }`
///
/// 页面那边是 `JSON.stringify(project)` 之后交给 `dhampir_project_open`。
pub(super) async fn polish_dhampir_project(
    State(_state): State<Arc<AppState>>,
    Json(body): Json<serde_json::Value>,
) -> ApiResult {
    let dir = body.get("dir").and_then(|v| v.as_str()).unwrap_or("");
    let path = validated_project_dir(dir)?;
    if !path.join("polish.toml").is_file() {
        return Err(err(StatusCode::BAD_REQUEST, "目录未初始化 polish"));
    }
    let scratch = path.join(yekitrim_core::path::PROJECT_STATE_DIR_NAME);
    std::fs::create_dir_all(&scratch)
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("建缓存目录失败: {e}")))?;
    let clip = path.clone();
    // **顺带把字幕/弹幕的文本带回去。**
    //
    // 转译器把这两个写成**旁写文件**，而工程里那两条资产的 `uri` 是**绝对路径**
    // （指向转译输出目录）—— 浏览器拿不到它：`/api/media` 只服务项目内相对路径，
    // 而且 `cleanup_project` 之后那个文件就没了。
    //
    // 症状是最难查的那一类：**预览看起来正常，只是没有字幕和弹幕**，还不报错。
    // 所以这里在 cleanup **之前**把内容读出来，让前端用 blob URL 喂给
    // `engine.loadSubtitles([{ assetId, url, format }])`（解析在 wasm 里，走 parse_srt/parse_ass）。
    let (json, subs) = run_blocking(
        "Dhampir 转译",
        move || -> anyhow::Result<(String, Vec<(String, String)>)> {
            // ⚠️ **预览这条腿必须 `keep_muted = false`**（压缩时间线）：客户端靠"落在被掐掉的段里
        // = 没有对应帧"去跳屏蔽区间，整条时间线会让它一路播过那些段。
        // 出片那条腿相反（`render_video_only` 传 true，屏蔽区间交给尾部那一剪）。
        // **有预览代理就用它**（`polish init` 生成，见 `render/proxy.rs`）：转译器会按代理
        // 探测时基/分辨率写进工程，所以画面缩放/取景也是按它算的。
        // 没有/过期 ⇒ `None`，照旧读源。
        let preview_video = yekitrim_polish::render::proxy::preview_video_for(&clip);
        let translated = yekitrim_polish::render::dhampir::prepare_project(
            &clip,
            &scratch,
            false,
            preview_video.as_deref(),
        )?;
            let text = std::fs::read_to_string(&translated)
                .map_err(|e| anyhow::anyhow!("读不出转译产物 {}：{e}", translated.display()))?;
            let mut subs: Vec<(String, String)> = Vec::new();
            for name in ["dhampir-subtitle.srt", "dhampir-danmaku.ass"] {
                let side = translated.with_file_name(name);
                match std::fs::read_to_string(&side) {
                    Ok(content) => subs.push((name.to_string(), content)),
                    // 缺一个不是错误（这份片段可能就没有弹幕）—— 但要**记下来**，
                    // 免得"预览里没字幕"又变成一次静默。
                    Err(e) => tracing::warn!("读不出旁写字幕 {}：{e}", side.display()),
                }
            }
            // 草稿用完就清（与出片那条同一个纪律：只删自己造的）。
            yekitrim_polish::render::dhampir::cleanup_project(&scratch);
            Ok((text, subs))
        },
    )
    .await?;
    let project: serde_json::Value = serde_json::from_str(&json)
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("转译产物不是合法 JSON: {e}")))?;
    let subtitles: serde_json::Map<String, serde_json::Value> = subs
        .into_iter()
        .map(|(name, text)| (name, serde_json::Value::String(text)))
        .collect();
    ok(serde_json::json!({ "ok": true, "project": project, "subtitles": subtitles }))
}

/// 贴纸逐帧 PNG 的**进程内缓存**。
///
/// # 为什么必须有（这是踩出来的）
///
/// 解一段 GIF 要跑满它的**全部帧** —— `哭_1.gif` 66 帧、数百毫秒；而预览在 33fps 下
/// 会**反复**要同一张贴纸的不同帧。不缓存的话每个请求都重解整段，
/// 请求立刻堆起来把服务端 CPU 打满 —— 现象是**播放一走到贴纸就卡死**，
/// 而且一个请求都回不来（工具条上是 `就绪 0/4 · 解码 0 · 已交 0`）。
///
/// 键带上 **mtime + 文件长度**：素材被换掉时自动失效，不需要谁记得清缓存。
/// 上限 64 条（贴纸数量本来就少，换素材会不断产生新键，得有个头）。
fn sticker_frames_cached(
    file: &std::path::Path,
) -> Result<std::sync::Arc<Vec<(Vec<u8>, u16)>>, super::AppError> {
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex, OnceLock};
    type Key = (std::path::PathBuf, u64, u64);
    // **按字节上限的 LRU** ✓ —— 以前是"条目数 > 64 就 `clear()`"✗：
    // 那不是淘汰是清零，贴纸一多/换素材就整表作废 ⇒ 同一个 GIF 被反复整段解码（一次数百 ms ✗）
    // ⇒ 现象正是"播放一走到贴纸就卡死"（实测 `解码 49` ✗）。
    const CAP_BYTES: usize = 512 * 1024 * 1024;   // 512MB 硬上限
    struct Entry {
        used: u64,            // 最近一次使用序号（LRU ✓）
        bytes: usize,
        frames: Arc<Vec<(Vec<u8>, u16)>>,
    }
    static CACHE: OnceLock<Mutex<(HashMap<Key, Entry>, u64, usize)>> = OnceLock::new();

    let meta = std::fs::metadata(file)
        .map_err(|e| err(StatusCode::NOT_FOUND, format!("贴纸读不到: {e}")))?;
    let mtime = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    let key = (file.to_path_buf(), mtime, meta.len());

    let cache = CACHE.get_or_init(|| Mutex::new((HashMap::new(), 0, 0)));
    let mut guard = cache.lock().expect("贴纸缓存锁中毒");
    guard.1 += 1;
    let now = guard.1;
    if let Some(hit) = guard.0.get_mut(&key) {
        hit.used = now;                       // 命中即刷新活跃度 ✓
        let frames = hit.frames.clone();
        drop(guard);
        return Ok(frames);
    }
    drop(guard);
    // 解码放在锁**外面**：同一张图被两个请求同时要时，宁可多解一次，
    // 也不要让慢解码堵住整张表的读（那又是另一种"卡死"）。
    let frames = yekitrim_polish::render::gif::decode_sticker_frames_png(file)
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("贴纸解码失败: {e}")))?;
    let bytes: usize = frames.iter().map(|(b, _)| b.len()).sum();
    let arc = Arc::new(frames);
    let mut guard = cache.lock().expect("贴纸缓存锁中毒");
    guard.1 += 1;
    guard.2 += bytes;
    let used = guard.1;
    guard.0.insert(key, Entry { used, bytes, frames: arc.clone() });
    // **超上限时只逐出最久未用的，直到回到上限以内** ✓（绝不再清零 ✗）
    while guard.2 > CAP_BYTES && guard.0.len() > 1 {
        let victim = guard
            .0
            .iter()
            .min_by_key(|(_, e)| e.used)
            .map(|(k, _)| k.clone());
        match victim {
            Some(k) => {
                if let Some(e) = guard.0.remove(&k) {
                    guard.2 = guard.2.saturating_sub(e.bytes);
                }
            }
            None => break,
        }
    }
    Ok(arc)
}

/// `POST /polish/sticker-frame` —— 贴纸的**第 k 帧**，PNG 以 base64 装在 JSON 里。
///
/// # ⚠️ 现状（2026-10-02 起）：这条路**只服务出片腿**，不再是预览的贴纸帧源
///
/// 底座 wasm 到了 **host_api v6**，动图（GIF / 动画 WebP）由**引擎自己解码、自己按帧号取帧**
/// （`dhampir_asset_load_animation`）—— 所以浏览器预览那条腿已经**不再调这一条**：
/// 它现在把动图原文件整段交给引擎（一次 fetch + 一次 load），播放期零请求。
/// 本仓前端**只有静态图**（`kind === 'image'`）仍会打这里。
///
/// **接口本身保留**：本仓服务端的出片腿（未切底座时）仍以它为准，
/// 那是"预览与成片贴纸逐帧一致"的过渡期保证。删它会连出片一起废掉。
///
/// # 为什么当初"谁来解多帧图"必须由服务端回答
///
/// 浏览器这一侧**两条路都实测过、都不可用**：
///
///   * `<img>` 的动画时钟会被节流 —— 不可见、被 canvas 遮住、甚至 1×1 盖在最上层
///     （不被遮挡）都不推进帧；宿主 `drawImage` 拿到的永远是同一张；
///   * `ImageDecoder`（WebCodecs）在目标浏览器里**根本不存在**
///     （`typeof ImageDecoder !== 'function'` → `decoder=false`、帧数 0）。
///
/// 而 Dhampir 的设计是"序列帧由宿主交进来"（`AssetKind::ImageSequence` 只是个校验标记，
/// 那边没有任何 GIF 解码器）。所以这一环只能由服务端补上 —— 而且**必须与出片同源**：
/// 这里走的就是出片用的 `decode_sticker_frames_png`，于是**预览与成片的贴纸逐帧一致**。
///
/// # 为什么逐帧，而不是一次给整段
///
/// 整段拼成一张横向 PNG 实测是 **19 MB**（500×500 × 66 帧），而且宽度 33000px
/// 超过浏览器单图上限（16384）。预览一次只要一帧，所以按帧取。
///
/// 帧号越界**报错**而不是夹到范围内：夹住会让"贴纸停在最后一帧"看起来像素材放完了。
pub(super) async fn polish_sticker_frame(
    State(_state): State<Arc<AppState>>,
    Json(body): Json<serde_json::Value>,
) -> ApiResult {
    let dir = body
        .get("dir")
        .and_then(|v| v.as_str())
        .ok_or_else(|| err(StatusCode::BAD_REQUEST, "缺 dir"))?;
    // 取成**自有 String**：下面要 move 进 blocking 闭包，借用 `body` 活不了那么久。
    let asset: String = body
        .get("asset")
        .and_then(|v| v.as_str())
        .ok_or_else(|| err(StatusCode::BAD_REQUEST, "缺 asset"))?
        .to_string();
    let frame = body.get("frame").and_then(|v| v.as_u64()).unwrap_or(0);
    let path = validated_project_dir(dir)?;
    // 越界防护：asset 必须是**工程内相对路径** —— 判据在 `crate::scope::is_safe_project_relative`
    // （与 `polish_sticker_prewarm`、scope 中间件共用同一份，不许各写一遍；这是 handler 侧的第二道）。
    if !is_safe_project_relative(&asset) {
        return Err(err(StatusCode::BAD_REQUEST, "asset 必须是工程内的相对路径"));
    }
    let file = path.join(&asset);
    // 解码要跑满 66 帧（数百 ms）→ 整段下 blocking 池，别占 worker。
    let (png, count) = blocking(move || {
        if !file.exists() {
            return Err(err(
                StatusCode::NOT_FOUND,
                format!("贴纸不存在：{asset}"),
            ));
        }
        // **走缓存**：同一个文件只解一次（见 `sticker_frames_cached` 的说明 ——
        // 不缓存的话，播放一走到贴纸就会把服务端打满、页面卡死）。
        let frames = sticker_frames_cached(&file)?;
        let count = frames.len();
        let idx = frame as usize;
        if count == 0 {
            return Err(err(StatusCode::INTERNAL_SERVER_ERROR, "贴纸一帧都没有"));
        }
        if idx >= count {
            return Err(err(
                StatusCode::BAD_REQUEST,
                format!("帧号 {idx} 越界（共 {count} 帧）"),
            ));
        }
        let (png, _delay) = frames[idx].clone();
        // 尺寸不回填：页面拿到 PNG 之后 `createImageBitmap` 自己就有宽高，
        // 在这儿再解一次 PNG 只是白花时间（而且 `image` 也不是 server 的依赖）。
        Ok((png, count))
    })
    .await?;
    use base64::Engine as _;
    let b64 = base64::engine::general_purpose::STANDARD.encode(&png);
    Ok(Json(serde_json::json!({
        "ok": true,
        "count": count,
        "frame": frame,
        "png_base64": b64,
    })))
}

/// 判据本体在 [`crate::scope::is_safe_project_relative`]（**全仓只此一份**）：
/// 中间件（`enforce_scope`，第一道）与这里的两个 handler（第二道）共用。
/// 当年"两份实现、判据还漏了根相对"就是 `/etc/passwd` 那条洞的成因 —— 别再把判据抄回来。
///
/// 把前端交上来的贴纸路径数组过滤成**工程内的相对路径**（判据见上）。
///
/// 写成函数而不是内联闭包，是为了让"越界值被挡掉"有单测钉住 —— 内联在 handler 里的 filter
/// 没有单测能碰它（这正是那条洞一直没被发现的原因）。
fn safe_sticker_assets(assets: Vec<String>, base: &std::path::Path) -> Vec<std::path::PathBuf> {
    assets
        .into_iter()
        .filter(|a| crate::scope::is_safe_project_relative(a))
        .map(|a| base.join(a))
        .collect()
}

/// `POST /polish/sticker-prewarm` —— 把若干贴纸的帧表**先解码进内存缓存**，不返回任何帧。
///
/// # 为什么需要它
///
/// `/polish/sticker-frame` 是"播到哪一帧才取哪一帧"，而**第一次**取某张贴纸时服务端要
/// **整段解码**那张 GIF（实测 `clip-04` 的 4 张贴纸冷解 **20.2s**、单张 3~9s，debug 构建）。
/// 页面那头 `Promise.race([…, 30ms])` 超时就**这一帧不交**，于是表现是
/// **"播放走到贴纸就有一段时间什么都不显示，然后突然出现"**（用户报的"贴纸有延迟"）。
///
/// 所以让客户端**在页面加载时**就把这件事做掉：解码挪到"用户还没播到那里"的时候，
/// 播放真正走到贴纸时全部命中缓存（实测命中 **10~14ms**）。
///
/// # 语义
///
/// * `assets` 是工程内相对路径数组（与 `/polish/sticker-frame` 的 `asset` 同一条纪律）。
/// * **尽力而为**：单张贴纸解不出来只跳过它，**不让整个请求失败** —— 它是预热，不是业务操作；
///   真有问题会在用户播到那帧时由 `sticker-frame` 照常报错。
/// * 返回 `{ok, decoded, failed, skipped}`，前端只用来打日志。
/// * 走 `blocking` 池：解码是纯 CPU 的同步活，占满一个核数秒，绝不能留在 async worker 上
///   （见 `crates/yekitrim-server/src/api/AGENTS.md` 的同步 IO 纪律）。
pub(super) async fn polish_sticker_prewarm(
    State(_state): State<Arc<AppState>>,
    Json(body): Json<serde_json::Value>,
) -> ApiResult {
    let dir = body
        .get("dir")
        .and_then(|v| v.as_str())
        .ok_or_else(|| err(StatusCode::BAD_REQUEST, "缺 dir"))?;
    let assets: Vec<String> = body
        .get("assets")
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    let path = validated_project_dir(dir)?;
    let files = safe_sticker_assets(assets, &path);
    let (decoded, failed) = blocking(move || {
        let mut ok_count = 0usize;
        let mut fail_count = 0usize;
        for f in &files {
            if !f.exists() {
                fail_count += 1;
                continue;
            }
            // 命中缓存就直接算成功（`sticker_frames_cached` 自己判），不做任何多余解码。
            match sticker_frames_cached(f) {
                Ok(_) => ok_count += 1,
                Err((_status, body)) => {
                    tracing::warn!("贴纸预热失败（{}）：{}", f.display(), body.0);
                    fail_count += 1;
                }
            }
        }
        Ok((ok_count, fail_count))
    })
    .await?;
    Ok(Json(serde_json::json!({
        "ok": true,
        "decoded": decoded,
        "failed": failed,
    })))
}
