//! `api::polish`：精修编辑器后端的**共享底座**（任务入口 + 成片产物清单）。
//!
//! 这个目录原来是一个 6,235 行的 `polish.rs`（39 个 handler）。按语义分文件：
//!
//! | 文件 | 语义 |
//! |---|---|
//! | `program` | 起任务：init / llm / llm-segment / process / render / cover（都是「校验 → `polish_spawn`」）|
//! | `project` | 精修工程本体（`polish.toml`）、字幕、情绪轨道 |
//! | `cover` | 封面：标题生成的一族纯函数 + 配置 / 背景 / 帧 |
//! | `fonts` | 字体库（按 `Owner` 分库）|
//! | `assets` | 素材：贴纸 / 视频库 / 音效 / emoji / 贴纸帧与预热 |
//! | `dhampir` | 底座出片通道：就绪位 / 工程转译缓存 / wasm 转发 |
//!
//! ⚠️ **模块之间只许显式 `use`**（整改 P4 T16）：拆分前 `polish.rs` 与
//! `project_data.rs` 互相 glob 引用、形成模块环（Rust 允许、编译不报错、门禁也抓不到），
//! 于是「按领域一文件一模块」失效。阶段 A 已把环断开（`validated_project_dir` 下沉到
//! `api/mod.rs`），本阶段再把文件按语义切开。
//!
//! 本文件只放**被两个以上子模块用到**的东西（成片产物清单、`polish_spawn`、`scope_pid`），
//! 以及给 `routes.rs` 的**显式**再导出（逐条点名，不许 `pub use x::*`）。

pub(super) mod program;
pub(super) mod project;
pub(super) mod cover;
pub(super) mod fonts;
pub(super) mod assets;
pub(super) mod dhampir;

// ---- 对外出口（`api/routes.rs` 的路由表）：**逐条点名** ----
// 这些是 endpoint 入口，所以是 `pub(crate)`（路由表要用）；域内 helper 一律 `pub(super)`。
pub(crate) use program::{
    polish_cover, polish_init, polish_llm, polish_llm_segment, polish_process, polish_render,
};
pub(crate) use project::{
    polish_emotion_get, polish_project_get, polish_project_put, polish_subtitles_get,
    polish_subtitles_put,
};
pub(crate) use cover::{
    polish_cover_background, polish_cover_config, polish_cover_frame, polish_cover_title,
    polish_select_version, polish_silence,
};
pub(crate) use fonts::{
    polish_font, polish_fonts, polish_fonts_builtin, polish_fonts_default, polish_fonts_delete,
    polish_fonts_import, polish_fonts_upload, FONT_UPLOAD_MAX_BYTES,
};
pub(crate) use assets::{
    polish_asset_file, polish_emoji_catalog, polish_sfx_catalog, polish_sticker_file,
    polish_sticker_frame, polish_sticker_library, polish_sticker_prewarm, polish_sticker_sources,
    polish_video_library, polish_video_thumb,
};
pub(crate) use dhampir::{polish_dhampir_project, polish_dhampir_status, polish_dhampir_wasm};


use std::path::Path;

use axum::extract::Extension;
use yekitrim_service::Workspace;

use crate::jobs::{JobClass, JobSpec};
use crate::model::AppState;

use super::*;
// 只点名要谁（不许 glob）：这两条是 polish 域对「管线 / 出片子进程」的**全部**依赖。
use super::pipeline::latest_numbered_file;
use super::render::run_polish_child;


// ===================== Polish =====================

/// 成片的**默认**文件名主干（`[project] output` 缺省）。与
/// `yekitrim_polish::project::config::ProjectMeta::default_output()` 是同一个值
/// （那边是 `vtuber-clip.mp4`）。
const DEFAULT_RENDER_STEM: &str = "vtuber-clip";


/// `[project] output` → 产物 stem（**去目录、去扩展名**）。空/无 stem ⇒ `None`。
///
/// 只取文件名部分：`latest_numbered_file` 扫的是**片段目录的直接子项**，
/// 而 `output` 允许写子目录（`out/my.mp4`）—— 那种情况这里仍返回 `my`，
/// 由调用方的默认名兜底（子目录产物的可见性不在本次修复范围内）。
fn render_stem_of(output: &str) -> Option<String> {
    let raw = output.trim();
    if raw.is_empty() {
        return None;
    }
    let stem = std::path::Path::new(raw)
        .file_stem()?
        .to_string_lossy()
        .trim()
        .to_string();
    if stem.is_empty() || stem == "." || stem == ".." {
        None
    } else {
        Some(stem)
    }
}


/// 这个片段目录里"最新那份成片"：stem **跟着工程自己的 `[project] output` 走**。
///
/// ⚠️ 这里曾经把 stem 写死成 `vtuber-clip` —— 用户改了输出名（精修页「渲染输出」/ 手改
/// `[project] output`）之后，成片明明写出来了（日志 `FINISHED: …/custom-out.mp4`），
/// 「产物」区却一直显示"导出成片或生成封面后在这里查看"、切片卡片也认为未出片
/// （review `polish.md` P1-2）。
///
/// 两段查询：先按工程配置的名字找，找不到再按**默认名**找 —— 后者保住
/// "先用默认名出过片、之后才改配置"的存量产物。
fn rendered_file_of(dir: &std::path::Path, configured_output: Option<&str>) -> Option<String> {
    if let Some(stem) = configured_output.and_then(render_stem_of) {
        if let Some(hit) = latest_numbered_file(dir, &stem, "mp4") {
            return Some(hit);
        }
        if stem == DEFAULT_RENDER_STEM {
            return None;
        }
    }
    latest_numbered_file(dir, DEFAULT_RENDER_STEM, "mp4")
}


/// 产物 stem 的候选集：**配置名在前、默认名垫底** —— 与 `rendered_file_of` 的兜底顺序同口径
/// （先按配置名找，找不到再看默认名，保住"先用默认名出过片、之后才改配置"的存量产物）。
fn render_stems(configured_output: Option<&str>) -> Vec<String> {
    let mut stems: Vec<String> = Vec::new();
    if let Some(stem) = configured_output.and_then(render_stem_of) {
        stems.push(stem);
    }
    if !stems.iter().any(|s| s == DEFAULT_RENDER_STEM) {
        stems.push(DEFAULT_RENDER_STEM.to_string());
    }
    stems
}


/// 某个 stem 的**全部版本**文件（`<stem>.<ext>` + `<stem>_<n>.<ext>`），按版本号升序。
///
/// 判定与 `latest_numbered_file` 逐条相同（同一套前后缀、同样的 `_N` 语义），区别只是**收全**
/// 而不是只留最大那份 —— 「产物」卡的文件夹视图要把历史版本也列出来。
fn numbered_family(dir: &Path, stem: &str, ext: &str) -> Vec<(u32, String)> {
    let mut out: Vec<(u32, String)> = Vec::new();
    let base = format!("{stem}.{ext}");
    if dir.join(&base).is_file() {
        out.push((0, base));
    }
    if let Ok(rd) = std::fs::read_dir(dir) {
        let prefix = format!("{stem}_");
        let suffix = format!(".{ext}");
        for entry in rd.flatten() {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            let name = entry.file_name().to_string_lossy().into_owned();
            // ⚠️ strip_prefix/strip_suffix 而不是算术切片：`cover_.jpg`（前后缀之间为空）在
            // 切片写法下会 panic（同 `latest_numbered_file` 的注释）。
            let Some(mid) = name.strip_prefix(&prefix).and_then(|rest| rest.strip_suffix(&suffix)) else {
                continue;
            };
            if let Ok(n) = mid.parse::<u32>() {
                out.push((n, name));
            }
        }
    }
    out.sort_by_key(|(n, _)| *n);
    out
}


/// 这个片段目录里的**全部产物文件**（成片每个版本 + 封面每个版本），按**产出先后**（旧 → 新）
/// 排好 —— 精修页「产物」卡右栏那个**文件夹视图**照它画：左列列文件、右侧预览选中的那份。
///
/// 为什么不直接用 `rendered_file` / `cover_file`：那两个是"**最新**那一份"的口径（切片卡片的
/// 「已导出」、任务日志都靠它们）；而界面上还要看得到历史版本 —— 成片**同名不覆盖**，第二次出片
/// 叫 `vtuber-clip_2.mp4`，以前除最新那份外都只能去文件管理器里翻。
///
/// ⚠️ **不能扫"目录里所有文件"**：同一个目录里还躺着 `clip.mp4`（源）、`_proxy_720.mp4`
/// （预览代理）、`_thumb.jpg`、`_cover_bg.jpg` / `_cover_frame.jpg`（封面设计器的中间图）与
/// `polish.toml` 等 —— 它们都不是产物。只认上面两族命名。
///
/// 排序键 = `(mtime, 类型, 版本号)`：mtime 是"先后"的事实来源；秒级 mtime 在部分文件系统上会
/// 并列（网络盘精度可能就是 1s），所以并列时按"成片在前、再按版本号"兜底，保证顺序稳定。
fn artifact_files_of(dir: &Path, configured_output: Option<&str>) -> Vec<serde_json::Value> {
    let mut found: Vec<(String, &'static str, u64)> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    for stem in render_stems(configured_output) {
        for (n, name) in numbered_family(dir, &stem, "mp4") {
            if seen.insert(name.clone()) {
                found.push((name, "video", n as u64));
            }
        }
    }
    for (n, name) in numbered_family(dir, "cover", "jpg") {
        if seen.insert(name.clone()) {
            found.push((name, "image", n as u64));
        }
    }
    let mut rows: Vec<(u64, u8, u64, String, &'static str, u64)> = found
        .into_iter()
        .map(|(name, kind, seq)| {
            let meta = std::fs::metadata(dir.join(&name)).ok();
            let bytes = meta.as_ref().map(|m| m.len()).unwrap_or(0);
            let mtime = meta
                .as_ref()
                .and_then(|m| m.modified().ok())
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs())
                .unwrap_or(0);
            (mtime, if kind == "video" { 0 } else { 1 }, seq, name, kind, bytes)
        })
        .collect();
    rows.sort_by(|a, b| (a.0, a.1, a.2).cmp(&(b.0, b.1, b.2)));
    rows.into_iter()
        .map(|(mtime, _, _, name, kind, bytes)| {
            serde_json::json!({ "name": name, "kind": kind, "bytes": bytes, "mtime": mtime })
        })
        .collect()
}


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
            // 工程只读一次：产物 stem（`[project] output`）与消音版判定都要它。
            let polish_cfg = if polish_ready {
                yekitrim_polish::project::load_project(&dir).ok()
            } else {
                None
            };
            // ⚠️ **stem 跟着工程配置走**（用户可改输出名），不能写死 `vtuber-clip` —— 见
            // `rendered_file_of` 的注释（review `polish.md` P1-2）。
            let rendered_file =
                rendered_file_of(&dir, polish_cfg.as_ref().map(|cfg| cfg.project.output.as_str()));
            let rendered = rendered_file.is_some();
            let cover_file = latest_numbered_file(&dir, "cover", "jpg");
            let cover = cover_file.is_some();
            // 产物卡的**文件夹视图**（2026-11-22）：全部版本、按产出先后。
            // ⚠️ 与上面两个"最新那份"字段并存 —— 别拿它去替换 `rendered`/`cover`：
            // 切片卡片的「已导出」判定与任务日志仍走那两个。
            let artifacts = artifact_files_of(
                &dir,
                polish_cfg.as_ref().map(|cfg| cfg.project.output.as_str()),
            );
            let mut version = "original";
            let mut mute_range_count = 0usize;
            if let Some(cfg) = polish_cfg.as_ref() {
                // 消音版 = 静音区间已写入屏蔽轨道（video 始终是未改动的 clip.mp4）
                if !cfg.silence_ranges.is_empty() { version = "silent"; }
                mute_range_count = cfg.mute_ranges.len();
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
                "artifacts": artifacts,
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
        // 精修 llm 无管线阶段框架：expected 空 → 任务行不显示 i/N（窗口进度走 Agent）
        let bridge = super::core::progress_bridge(job_id.clone(), jobs.clone(), token, Vec::new());
        // 取消闭包捕获**克隆**：bridge 本体还要以 Some(bridge.clone()) 传进 cmd_llm_for
        // 当窗口进度出口（2026-11-15 P2）—— 先 move 后 clone 就是 use-after-move。
        let bridge_for_cancel = bridge.clone();
        let cancel: Option<yekitrim_llm::agent::CancelProbe> = Some(std::sync::Arc::new(move || {
            yekitrim_core::progress::ProgressReporter::cancelled(&*bridge_for_cancel)
        }));
        let result = async {
            match kind.as_str() {
                "init" => {
                    // v2 L1：init 里要转 720p 预览代理（长视频分钟级），原来**零事件** ⇒
                    // 界面 120s 后误报"可能卡住"。前后各一条 log，配合 L2 心跳兜底。
                    jobs.log(&job_id, "info", "初始化精修工程…").await;
                    let r = yekitrim_polish::commands::cmd_init_with(&path, Some(&bridge)).await;
                    if r.is_ok() {
                        jobs.log(&job_id, "info", "精修工程就绪（含预览代理与响度缓存）").await;
                    }
                    r
                }
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
                    // 窗口进度（spec §1 P2）：桥透传进 cmd_llm_for → Agent 折成 stage_progress
                    yekitrim_polish::commands::cmd_llm_for(owner, &path, opts.full, cancel.clone(), Some(bridge.clone())).await?;
                    jobs.log(&job_id, "info", "LLM 完成，自动执行 process 后处理…").await;
                    yekitrim_polish::process::cmd_process_with(&path, Some(&bridge)).await
                }
                // 只重做某一段的事件（编辑器：时间轴 → 分段属性）。失败会**保留该段原有事件**，
                // 所以这里 fail 掉任务不会让用户丢东西；后处理照跑（别的段不动，幂等）。
                "llm_segment" => {
                    let idx = opts
                        .segment_index
                        .ok_or_else(|| anyhow::anyhow!("缺少 segment_index"))?;
                    jobs.log(&job_id, "info", format!("只重做分段 #{idx} 的事件…")).await;
                    yekitrim_polish::commands::cmd_llm_segment_for(owner, &path, idx, cancel.clone(), Some(bridge.clone())).await?;
                    jobs.log(&job_id, "info", "完成，自动执行 process 后处理…").await;
                    yekitrim_polish::process::cmd_process_with(&path, Some(&bridge)).await
                }
                "process" => yekitrim_polish::process::cmd_process_with(&path, Some(&bridge)).await,
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

#[cfg(test)]
mod tests {
    use super::*;

    // ⚠️ 这些用例是**按原字节搬来**的（含原本在文件顶层、缩进为 0 的那批）：
    //    统一重排缩进会动到多行字符串字面量，判据会因此悄悄变。


    /// ★ **P1-2 的钉子：成片 stem 跟着 `[project] output` 走。**
    ///
    /// 以前 `rendered_file` 的 stem 写死成 `vtuber-clip`：用户改了输出名之后，成片明明写出来了
    /// （日志 `FINISHED: …/custom-out.mp4`），「产物」区却一直显示"导出成片或生成封面后在这里查看"、
    /// 切片卡片也认为未出片（review `polish.md` P1-2）。
    /// 两段查询（配置名 → 默认名）同时钉住"默认名的存量行为不变"。
    #[test]
    fn rendered_file_follows_the_configured_output_name() {
        let dir = std::env::temp_dir().join(format!(
            "yekitrim-rendered-name-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        // ① 自定义输出名：目录里只有 my-cut_2.mp4
        std::fs::write(dir.join("my-cut_2.mp4"), b"x").unwrap();
        assert_eq!(
            rendered_file_of(&dir, Some("my-cut.mp4")).as_deref(),
            Some("my-cut_2.mp4"),
            "自定义输出名必须被认出来（同名不覆盖的 _n 后缀也算）"
        );
        // …而且改了配置名之后，先用**默认名**出过的片不能凭空消失
        std::fs::remove_file(dir.join("my-cut_2.mp4")).unwrap();
        std::fs::write(dir.join("vtuber-clip_1.mp4"), b"x").unwrap();
        assert_eq!(
            rendered_file_of(&dir, Some("my-cut.mp4")).as_deref(),
            Some("vtuber-clip_1.mp4"),
            "配置名找不到时必须回落默认名（存量产物）"
        );

        // ② 没有工程 / 名字是空白 ⇒ 默认名，行为与改造前逐字相同
        assert_eq!(
            rendered_file_of(&dir, None).as_deref(),
            Some("vtuber-clip_1.mp4"),
            "读不到工程时必须回落默认名"
        );
        assert_eq!(
            rendered_file_of(&dir, Some("  ")).as_deref(),
            Some("vtuber-clip_1.mp4"),
            "空名字也必须回落默认名"
        );

        // ③ stem 推导：去目录、去扩展名；只有空白 ⇒ None（否则会去扫 ".mp4" 这种名字）
        assert_eq!(render_stem_of("my-cut.mp4").as_deref(), Some("my-cut"));
        assert_eq!(render_stem_of("out/my-cut.mp4").as_deref(), Some("my-cut"));
        assert_eq!(render_stem_of("   "), None);
        assert_eq!(render_stem_of(""), None);

        let _ = std::fs::remove_dir_all(&dir);
    }


    /// ★ **产物卡的文件夹视图：全部版本 + 按产出先后 + 不混进同目录的非产物文件。**
    ///
    /// 用户 2026-11-22 把「产物」右半改成文件夹（左列表 / 右预览），列表要的是**每一个**版本
    /// （`vtuber-clip_1.mp4`…），不再只是"最新那一份"。这里钉三件事：
    /// ① 两族命名都在：成片（跟随 `[project] output` + 默认名兜底）与封面（`cover.jpg`/`cover_N.jpg`）；
    /// ② 顺序 = **产出先后**（mtime），不是文件名字典序 —— 字符串序会把 `cover.jpg` 排到最后，
    ///    而它其实比 `vtuber-clip_1.mp4` 先产出；
    /// ③ `clip.mp4` / `_proxy_720.mp4` / `_thumb.jpg` / `_cover_bg.jpg` / `_cover_frame.jpg` /
    ///    `polish.toml` 与 `cover_.jpg`（空版本号）这些**同目录的非产物**一个都不许出现。
    #[test]
    fn artifacts_list_every_version_in_production_order() {
        let dir = std::env::temp_dir().join(format!("yekitrim-artifacts-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // 显式写 mtime：创建顺序在同一秒内不可靠，而"先后"正是这条的判据
        let touch = |name: &str, bytes: usize, secs: u64| {
            let p = dir.join(name);
            std::fs::write(&p, vec![b'x'; bytes]).unwrap();
            let t = std::time::UNIX_EPOCH + std::time::Duration::from_secs(secs);
            std::fs::File::options()
                .write(true)
                .open(&p)
                .unwrap()
                .set_modified(t)
                .unwrap();
        };
        touch("vtuber-clip.mp4", 10, 1_700_000_100);
        touch("cover.jpg", 20, 1_700_000_200);
        touch("vtuber-clip_1.mp4", 30, 1_700_000_300);
        touch("cover_1.jpg", 40, 1_700_000_400);
        for noise in [
            "clip.mp4",
            "_proxy_720.mp4",
            "_thumb.jpg",
            "_cover_bg.jpg",
            "_cover_frame.jpg",
            "cover_.jpg",
            "polish.toml",
        ] {
            touch(noise, 5, 1_700_000_500);
        }

        let got: Vec<(String, String, u64, u64)> = artifact_files_of(&dir, None)
            .into_iter()
            .map(|v| {
                (
                    v["name"].as_str().unwrap().to_string(),
                    v["kind"].as_str().unwrap().to_string(),
                    v["bytes"].as_u64().unwrap(),
                    v["mtime"].as_u64().unwrap(),
                )
            })
            .collect();
        assert_eq!(
            got.iter().map(|(n, _, _, _)| n.as_str()).collect::<Vec<_>>(),
            vec!["vtuber-clip.mp4", "cover.jpg", "vtuber-clip_1.mp4", "cover_1.jpg"],
            "必须按产出先后（mtime），且同目录的 clip.mp4/_proxy/_thumb/_cover_*/polish.toml/cover_.jpg 都不算产物"
        );
        assert_eq!(got[1].1, "image", "封面是图片");
        assert_eq!(got[2].1, "video", "成片是视频");
        assert_eq!(got[3].2, 40, "bytes 要照实回（列表要显示大小）");
        assert_eq!(got[3].3, 1_700_000_400, "mtime 要照实回（产物先后的显示靠它）");

        // 配置了自定义输出名 ⇒ 那一族也在；默认名那种存量也保留（与 `rendered_file_of` 同口径）
        touch("my-cut.mp4", 7, 1_700_000_050);
        let names: Vec<String> = artifact_files_of(&dir, Some("my-cut.mp4"))
            .into_iter()
            .map(|v| v["name"].as_str().unwrap().to_string())
            .collect();
        assert_eq!(names.first().map(|s| s.as_str()), Some("my-cut.mp4"), "最早产出的排最前");
        assert!(
            names.contains(&"vtuber-clip_1.mp4".to_string()),
            "改过输出名之后，默认名那批存量产物也要在：{names:?}"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

}