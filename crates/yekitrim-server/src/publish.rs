//! 出片回传：把 YekiTrim 的成片交给 **biliLive-tools** 投稿。
//!
//! YekiTrim 自己**不做 B站上传**（登录、分区、标签、封面、分P、重试全在 biliLive-tools 里）。
//! 这里只做三件事：
//! 1. 把选中的 `clip.mp4` 复制到 biliLive-tools **容器看得见**的暂存目录；
//! 2. 调它的 `/preset/video/<id>` 取上传预设，再 `/bili/validUploadParams` + `/bili/upload`；
//! 3. 把 `taskId` / 容器路径写进 `<项目>/output/publish_state.json`，查询时用 `/task`
//!    回填状态。
//!
//! ⚠️ **路径有两个面**：宿主机面 `staging_dir`（写文件用）与容器面
//! `container_staging_root`（传给 biliLive-tools 的 `videos[].path`）。两者必须是同一个目录，
//! 只换前缀 —— 写错时 biliLive-tools 会报「文件不存在」，而不是在这里报错。
//!
//! ⚠️ 源文件只 **copy** 不 move/删：录像目录是 biliLive-tools 的录制历史，不能动。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use axum::extract::State;
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::api::{blocking, err, ok, ApiResult};
use crate::auth::CurrentUser;
use crate::model::AppState;
use crate::scope::Scope;

/// 状态文件名（落在 `<项目>/output/` 下，与 `publish_config.json` 同级）。
const STATE_FILE: &str = "publish_state.json";
const MAX_CLIPS: usize = 50;

#[derive(Debug, Deserialize)]
pub(crate) struct PublishBody {
    /// 要发布的片段：rank 数字或目录名（`clip-01`）；空 = 全部。
    #[serde(default)]
    clips: Vec<Value>,
    /// 覆盖配置里的上传预设 id（可选）。
    #[serde(default)]
    preset_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct PublishConfigFile {
    #[serde(default)]
    clips: Vec<ClipEntry>,
}

#[derive(Debug, Clone, Deserialize)]
struct ClipEntry {
    #[serde(default)]
    file: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    summary: String,
    #[serde(default)]
    rank: u32,
    #[serde(default)]
    directory: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ClipPublishState {
    title: String,
    container_path: String,
    #[serde(default)]
    task_id: Option<String>,
    #[serde(default)]
    status: String,
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    submitted_at: i64,
    #[serde(default)]
    updated_at: i64,
    #[serde(default)]
    response: Option<Value>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct PublishState {
    #[serde(default)]
    clips: BTreeMap<String, ClipPublishState>,
}

struct StagedClip {
    directory: String,
    title: String,
    summary: String,
    container_path: String,
}

/// `POST /api/publish/bililive-tools?project=<id> {clips:[...],preset_id?}`。
///
/// 权限：仅 **admin**（B站账号 / 上传预设 / passkey 都是程序级配置）。
///
/// ⚠️ 这里是**创建后台任务**，不是同步上传：复制 + 预检 + 投稿最多 50 条、每条 30s 超时，
/// 挂在一个 HTTP 请求里会没有进度、没法取消、还容易被反代掐断。
/// 任务占 `p:out` + `p:idx` 两个资源键（与剪切/管线互斥：剪切正在重写 clip.mp4 时
/// 读到半截文件会传出一支损坏的成片，而且**不会报错**）；POST 只回 `{id, queued, pending}`，
/// 前端 `watchJob(id)` 到终态后再 GET 发布状态。
pub(crate) async fn publish(
    State(state): State<Arc<AppState>>,
    axum::extract::Extension(user): axum::extract::Extension<CurrentUser>,
    scope: Scope,
    Json(body): Json<PublishBody>,
) -> ApiResult {
    // ⚠️ **仅管理员**：[publish.bililive_tools] 是**程序级**配置（B站登录、上传预设、
    // passkey 全在同一份全局 config.toml 里，且只有 admin 能写）。普通账号若能调这条，
    // 就是「把自己的切片投到这台机器管理员的 B站账号」—— 权限语义上不成立。
    ensure_publisher(&user)?;
    let project_id = scope.project.id;
    let cfg = yekitrim_core::config::load_global_config();
    // 拷贝出来（不是借用）：任务体是 `'static`，借用 `cfg` 会编译不过；
    // 这些键都只是在建任务前做校验/取值用，拷一份几 KB 的成本可以忽略。
    let pc = cfg.publish.bililive_tools.clone();
    ensure_configured(&pc)?;

    // `output_dir()` 会读 `yekitrim.toml` 再合并全局配置（同步 IO），而工程目录经常在
    // 网络盘上 —— 按 api 卡「同步 IO 纪律」下 blocking 池。
    let ws_for_out = scope.ws.clone();
    let output = blocking(move || Ok(ws_for_out.output_dir())).await?;
    let publish_file = output.join("publish_config.json");
    let file = blocking(move || {
        let raw = std::fs::read_to_string(&publish_file).map_err(|e| {
            err(
                axum::http::StatusCode::NOT_FOUND,
                format!("还没有发布索引（先剪切）：{e}"),
            )
        })?;
        let parsed: PublishConfigFile = serde_json::from_str(&raw)
            .map_err(|e| err(axum::http::StatusCode::BAD_REQUEST, format!("发布索引解析失败: {e}")))?;
        Ok(parsed)
    })
    .await?;

    let selected = select_clips(&file.clips, &body.clips)?;
    if selected.is_empty() {
        return Err(err(
            axum::http::StatusCode::BAD_REQUEST,
            "没有可发布的片段（索引里没有 clip.mp4）",
        ));
    }

    let preset_id = body
        .preset_id
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(pc.upload_preset_id.trim())
        .to_string();
    if preset_id.is_empty() || preset_id.contains('/') {
        return Err(err(
            axum::http::StatusCode::BAD_REQUEST,
            "上传预设 id 为空或非法",
        ));
    }
    // ⚠️ `read_passkey` 是 `metadata` + `read_to_string`（同步 IO）：走 blocking 池，
    // 别让一次死盘/网盘上的 stat 占住 tokio worker（api 卡「同步 IO 纪律」）。
    let pc_for_key = pc.clone();
    let key = blocking(move || {
        read_passkey(&pc_for_key).map_err(|e| err(axum::http::StatusCode::BAD_REQUEST, e))
    })
    .await?;
    // ⚠️ **后台任务**：复制 + 预检 + 投稿是分钟级的（最多 50 条、每条 30s 超时），
    // 不再挂在一个 HTTP 请求里。校验只做「能不能发」，真正的活在 run_publish_job；
    // 前端 POST 后 watchJob(id)，任务终态再 GET 状态（publish_state.json 逐条落盘）。
    let base = pc.base_url.trim_end_matches('/').to_string();
    let staging_dir = PathBuf::from(pc.staging_dir.trim());
    let container_root = pc.container_staging_root.trim().trim_end_matches('/').to_string();
    let state_path = output.join(STATE_FILE);
    let pending = selected.len();

    let spec = crate::jobs::JobSpec::new(
        "publish",
        format!("发布 {pending} 条切片到 B站"),
        vec![
            crate::scope_key::project_out(project_id),
            crate::scope_key::project_index(project_id),
        ],
        crate::jobs::JobClass::Medium,
    );
    let handle = state.jobs.create(user.id, spec).await;
    let id = handle.id.clone();
    let queued = handle.queued;
    state.jobs.attach_project(&id, project_id).await;
    let jobs = state.jobs.clone();
    let job_id = id.clone();
    crate::api::spawn_job(jobs.clone(), job_id.clone(), move || async move {
        if handle.wait_start().await.is_err() {
            jobs.complete(&job_id).await;
            return;
        }
        let result = run_publish_job(
            &jobs,
            &job_id,
            project_id,
            output,
            state_path,
            base,
            key,
            preset_id,
            pc.uid,
            staging_dir,
            container_root,
            selected,
        )
        .await;
        match result {
            Ok(()) => jobs.complete(&job_id).await,
            Err(msg) => jobs.fail(&job_id, msg).await,
        }
    });
    ok(json!({
        "ok": true,
        "id": id,
        "queued": queued,
        "pending": pending,
    }))
}

/// 后台发布任务体：复制成片 → 逐条预检/投稿 → 逐条落 publish_state.json。
///
/// ⚠️ 网络调用全在 publish_state.json 的锁外；只有读-改-写那一段持锁。
/// ⚠️ 每提交一条就落一次盘：任务被取消/进程崩了也能看到已经提交的部分。
#[allow(clippy::too_many_arguments)]
async fn run_publish_job(
    jobs: &Arc<crate::jobs::JobManager>,
    job_id: &str,
    project_id: i64,
    output: PathBuf,
    state_path: PathBuf,
    base: String,
    key: String,
    preset_id: String,
    configured_uid: u64,
    staging_dir: PathBuf,
    container_root: String,
    selected: Vec<ClipEntry>,
) -> Result<(), String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|e| format!("HTTP 客户端失败: {e}"))?;

    let staged = blocking(move || {
        stage_clips(&output, project_id, &selected, &staging_dir, &container_root)
    })
    .await
    .map_err(|e| err_text(&e))?;
    if staged.is_empty() {
        return Err("没有可发布的片段（索引里没有 clip.mp4）".into());
    }
    let total = staged.len() as u64;
    jobs.log(job_id, "info", format!("开始发布 {total} 条切片"))
        .await;

    let preset = get_json(&client, &base, &format!("/preset/video/{preset_id}"), &key)
        .await
        .map_err(|e| err_text(&e))?;
    let preset_config = preset
        .get("config")
        .cloned()
        .filter(|v| v.is_object())
        .ok_or_else(|| format!("biliLive-tools 没有返回预设 {preset_id} 的 config"))?;
    let uid = resolve_uid(&client, &base, &key, configured_uid)
        .await
        .map_err(|e| err_text(&e))?;

    let cancel = jobs.cancel_token(job_id).await;
    let is_cancelled = || {
        cancel
            .as_ref()
            .is_some_and(|f| f.load(std::sync::atomic::Ordering::SeqCst))
    };

    let mut ok_count = 0usize;
    let mut errors: Vec<String> = Vec::new();
    for (i, clip) in staged.iter().enumerate() {
        if is_cancelled() {
            return Err("任务已取消".into());
        }
        let mut config = preset_config.clone();
        if let Some(obj) = config.as_object_mut() {
            obj.insert("title".into(), json!(clip.title));
            obj.insert("desc".into(), json!(clip.summary));
        }
        let attempted: Result<Value, String> = match post_precheck(
            &client,
            &base,
            "/bili/validUploadParams",
            &key,
            config.clone(),
        )
        .await
        {
            Ok(_) => {
                let upload_body = json!({
                    "uid": uid,
                    "videos": [{ "path": clip.container_path, "title": clip.title }],
                    "config": config,
                    "options": { "removeOriginAfterUploadCheck": false }
                });
                post_json(&client, &base, "/bili/upload", &key, upload_body)
                    .await
                    .map_err(|e| format!("{}: 投稿失败: {}", clip.directory, err_text(&e)))
            }
            Err(e) => Err(format!("{}: 预检失败: {}", clip.directory, err_text(&e))),
        };
        let entry = match attempted {
            Ok(resp) => {
                ok_count += 1;
                jobs.log(job_id, "info", format!("已提交 {}", clip.directory))
                    .await;
                ClipPublishState {
                    title: clip.title.clone(),
                    container_path: clip.container_path.clone(),
                    task_id: task_id_of(&resp),
                    status: "submitted".into(),
                    error: None,
                    submitted_at: now_secs(),
                    updated_at: now_secs(),
                    response: Some(resp),
                }
            }
            Err(msg) => {
                errors.push(msg.clone());
                jobs.log(job_id, "error", msg.clone()).await;
                ClipPublishState {
                    title: clip.title.clone(),
                    container_path: clip.container_path.clone(),
                    task_id: None,
                    status: "error".into(),
                    error: Some(msg),
                    submitted_at: now_secs(),
                    updated_at: now_secs(),
                    response: None,
                }
            }
        };
        let dir = clip.directory.clone();
        update_state(&state_path, move |s| {
            s.clips.insert(dir, entry);
            true
        })
        .await
        .map_err(|e| err_text(&e))?;
        jobs.progress(job_id, (i + 1) as u64, total).await;
    }

    if !errors.is_empty() {
        let brief: Vec<&str> = errors.iter().take(3).map(String::as_str).collect();
        return Err(format!(
            "{} 条失败 / {} 条成功：{}",
            errors.len(),
            ok_count,
            brief.join("；")
        ));
    }
    jobs.log(job_id, "info", format!("发布完成：{ok_count} 条"))
        .await;
    Ok(())
}


/// `GET /api/publish/bililive-tools?project=<id>`：读取发布状态，并用 biliLive-tools
/// `/task` 把 `taskId` 对应的状态回填。
pub(crate) async fn status(
    State(_state): State<Arc<AppState>>,
    axum::extract::Extension(user): axum::extract::Extension<CurrentUser>,
    scope: Scope,
) -> ApiResult {
    // 与 publish 同一条权限：状态里含 taskId / 标题 / 容器路径，也是程序级发布面的读口。
    ensure_publisher(&user)?;
    let cfg = yekitrim_core::config::load_global_config();
    let pc = &cfg.publish.bililive_tools;
    // 与 `publish` 用**同一个**判据：配置不全时前端不显示发布按钮，而不是显示了、
    // 点了才回 400。只在这里另写一份 `base_url.is_empty()` 会让「缺 staging_dir」这类
    // 半个配置在界面上显示成"可用"（点了必失败）。
    if let Err(e) = ensure_configured(pc) {
        return ok(json!({ "ok": true, "configured": false, "error": err_text(&e), "clips": {} }));
    }
    let base = pc.base_url.trim_end_matches('/').to_string();
    // 与 `publish` 同一个理由：读 passkey 与算 output_dir 都是同步 IO → blocking 池。
    let pc_for_key = pc.clone();
    let key = match blocking(move || Ok(read_passkey(&pc_for_key))).await? {
        Ok(k) => k,
        Err(e) => {
            return ok(json!({ "ok": true, "configured": false, "error": e, "clips": {} }))
        }
    };
    let ws_for_out = scope.ws.clone();
    let state_path = blocking(move || Ok(ws_for_out.output_dir()))
        .await?
        .join(STATE_FILE);
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(20))
        .build()
        .map_err(|e| err(axum::http::StatusCode::INTERNAL_SERVER_ERROR, format!("HTTP 客户端失败: {e}")))?;
    let tasks = get_json(&client, &base, "/task", &key).await.unwrap_or(Value::Null);
    let list = tasks
        .get("list")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    // 写状态也要拿锁：这条是 GET，但它在改 publish_state.json —— 不拿锁的话
    // 「发布进行中刷新界面」会把自己读到的旧快照整份盖回去（刚投的稿又变回"未投稿"）。
    // ⚠️ `/task` 的网络调用在锁外，只有下面这段读-改-写在锁里。
    let state = update_state(&state_path, move |state| {
        let mut changed = false;
        for (dir, entry) in state.clips.iter_mut() {
            let mut entry_changed = false;
            let found = list.iter().find(|t| {
                let id_match = entry
                    .task_id
                    .as_deref()
                    .and_then(|id| t.get("taskId").and_then(|v| v.as_str()).map(|x| x == id))
                    .unwrap_or(false);
                let output_match = t
                    .get("output")
                    .and_then(|v| v.as_str())
                    .map(|x| x == entry.container_path)
                    .unwrap_or(false);
                id_match || output_match
            });
            if let Some(t) = found {
                let status = t
                    .get("status")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown")
                    .to_string();
                if entry.status != status {
                    entry.status = status;
                    entry_changed = true;
                }
                if let Some(id) = t.get("taskId").and_then(|v| v.as_str()) {
                    if entry.task_id.as_deref() != Some(id) {
                        entry.task_id = Some(id.to_string());
                        entry_changed = true;
                    }
                }
                let error = t
                    .get("error")
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.is_empty())
                    .map(str::to_string);
                if entry.error != error {
                    entry.error = error;
                    entry_changed = true;
                }
            }
            // 没变化就**一个字节都不写**（前端会轮询这条接口）。
            if entry_changed {
                entry.updated_at = now_secs();
                changed = true;
            }
            tracing::debug!("发布状态: {dir} -> {}", entry.status);
        }
        changed
    })
    .await?;
    ok(json!({ "ok": true, "configured": true, "clips": state.clips }))
}

fn err_text(e: &crate::api::AppError) -> String {
    e.1 .0
        .get("error")
        .and_then(|v| v.as_str())
        .unwrap_or("未知错误")
        .to_string()
}

/// 出片回传的权限判据（两个 handler 共用一份，别各写一行）。
///
/// 理由：发布配置是**程序级**的（全局 config.toml + 一个 passkey 文件），
/// 写这些键的只有 admin；普通账号能投稿 = 能借管理员的 B站账号发视频。
fn ensure_publisher(user: &CurrentUser) -> Result<(), crate::api::AppError> {
    if !user.is_admin() {
        return Err(err(
            axum::http::StatusCode::FORBIDDEN,
            "出片回传仅管理员可用（B站账号与上传预设是全局配置）",
        ));
    }
    Ok(())
}

fn ensure_configured(pc: &yekitrim_core::config::BiliLivePublishConfig) -> Result<(), crate::api::AppError> {
    if pc.base_url.trim().is_empty()
        || pc.passkey_file.trim().is_empty()
        || pc.staging_dir.trim().is_empty()
        || pc.container_staging_root.trim().is_empty()
    {
        return Err(err(
            axum::http::StatusCode::BAD_REQUEST,
            "未配置 [publish.bililive_tools]（base_url / passkey_file / staging_dir / container_staging_root 都必须填）",
        ));
    }
    Ok(())
}

fn read_passkey(
    pc: &yekitrim_core::config::BiliLivePublishConfig,
) -> Result<String, String> {
    let path = PathBuf::from(pc.passkey_file.trim());
    let meta = std::fs::metadata(&path)
        .map_err(|e| format!("读不到 passkey_file {}: {e}", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if meta.permissions().mode() & 0o077 != 0 {
            return Err(format!(
                "passkey_file 权限过宽（{:o}）：请 chmod 600 {}",
                meta.permissions().mode() & 0o777,
                path.display()
            ));
        }
    }
    let raw = std::fs::read_to_string(&path).map_err(|e| format!("读取 passkey_file 失败: {e}"))?;
    let key = raw.trim().to_string();
    if key.is_empty() {
        return Err("passkey_file 内容为空".into());
    }
    Ok(key)
}

fn select_clips(all: &[ClipEntry], wanted: &[Value]) -> Result<Vec<ClipEntry>, crate::api::AppError> {
    let mut out: Vec<ClipEntry> = if wanted.is_empty() {
        all.iter()
            .filter(|c| !c.file.trim().is_empty() && !c.directory.trim().is_empty())
            .cloned()
            .collect()
    } else {
        let mut picked = Vec::new();
        for w in wanted {
            let hit = if let Some(rank) = w.as_u64() {
                all.iter().find(|c| c.rank as u64 == rank)
            } else if let Some(dir) = w.as_str() {
                all.iter().find(|c| c.directory == dir || c.file == dir)
            } else {
                None
            };
            let Some(hit) = hit else {
                return Err(err(
                    axum::http::StatusCode::BAD_REQUEST,
                    format!("发布索引里没有这个片段: {w}"),
                ));
            };
            if !hit.file.trim().is_empty() && !hit.directory.trim().is_empty() {
                picked.push(hit.clone());
            }
        }
        picked
    };
    out.sort_by_key(|c| c.rank);
    // ⚠️ **不许静默截断**：以前 `truncate(MAX_CLIPS)` 让"发布全部"在 51 条时悄悄只发前 50 条，
    //    界面回"已提交"、用户以为全发出去了。超限就明确拒绝，让用户分批（或改上限）。
    if out.len() > MAX_CLIPS {
        return Err(err(
            axum::http::StatusCode::BAD_REQUEST,
            format!("一次最多发布 {MAX_CLIPS} 条（本次 {} 条），请分批提交", out.len()),
        ));
    }
    Ok(out)
}

fn stage_clips(
    root: &Path,
    project_id: i64,
    clips: &[ClipEntry],
    staging_dir: &Path,
    container_root: &str,
) -> Result<Vec<StagedClip>, crate::api::AppError> {
    if staging_dir.as_os_str().is_empty() {
        return Err(err(
            axum::http::StatusCode::BAD_REQUEST,
            "staging_dir 为空",
        ));
    }
    if container_root.is_empty() {
        return Err(err(
            axum::http::StatusCode::BAD_REQUEST,
            "container_staging_root 为空",
        ));
    }
    let dir = staging_dir.join(project_id.to_string());
    std::fs::create_dir_all(&dir).map_err(|e| {
        err(
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            format!("创建暂存目录失败 {}: {e}", dir.display()),
        )
    })?;
    let mut out = Vec::new();
    for c in clips {
        let name = safe_component(&c.directory);
        if name.is_empty() {
            return Err(err(
                axum::http::StatusCode::BAD_REQUEST,
                format!("片段目录名非法: {}", c.directory),
            ));
        }
        let src = yekitrim_core::tenant::ensure_within(root, Path::new(&c.file))
            .map_err(|e| err(axum::http::StatusCode::BAD_REQUEST, format!("片段路径越界: {e:#}")))?;
        if !src.is_file() {
            return Err(err(
                axum::http::StatusCode::BAD_REQUEST,
                format!("片段文件不存在: {}", src.display()),
            ));
        }
        let dst = dir.join(format!("{name}.mp4"));
        // ⚠️ **只比大小是不够的**：重剪/重渲染出**同样字节数**的新片时，旧暂存文件会被
        // 当成"已经是最新的"直接复用 ⇒ 用户发出去的是**上一版**，而且不会报错。
        // `fs::copy` 不保留源 mtime（dst 的 mtime = 复制那一刻），所以「src 比 dst 新」
        // 恰好就是"源文件在复制之后又变过"，这也是唯一不额外存清单的成本。
        let need_copy = match (std::fs::metadata(&src), std::fs::metadata(&dst)) {
            (Ok(a), Ok(b)) => {
                let src_not_newer = match (a.modified(), b.modified()) {
                    (Ok(sm), Ok(dm)) => sm <= dm,
                    // 拿不到时间戳就别省这次复制（宁可多拷一次，也别发上一版）
                    _ => false,
                };
                a.len() != b.len() || !src_not_newer
            }
            _ => true,
        };
        if need_copy {
            std::fs::copy(&src, &dst).map_err(|e| {
                err(
                    axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                    format!("复制片段到暂存目录失败 {}: {e}", dst.display()),
                )
            })?;
        }
        let container_path = format!("{container_root}/{project_id}/{name}.mp4");
        out.push(StagedClip {
            directory: name,
            title: if c.title.trim().is_empty() {
                c.directory.clone()
            } else {
                c.title.clone()
            },
            summary: c.summary.clone(),
            container_path,
        });
    }
    Ok(out)
}

fn safe_component(raw: &str) -> String {
    let s = raw.trim();
    if s.is_empty()
        || s.contains('/')
        || s.contains('\\')
        || s.contains("..")
        || s.contains(':')
    {
        return String::new();
    }
    s.chars()
        .filter(|c| !c.is_control())
        .take(80)
        .collect()
}

/// 发布状态的**唯一**写入通道：`lock_path` → load → 闭包改 → （真改了才）落盘。
///
/// 两个消费者都在写同一个 `publish_state.json`（POST 提交 / GET 回填状态），
/// 所以读-改-写必须整段在同一把锁里；而**网络调用一律在锁外**（见 `publish` 的循环）。
/// 闭包返回 `false` = 没有任何字段变化 ⇒ 不写盘（GET 轮询不该每 3 秒重写一次文件）。
async fn update_state<F>(
    path: &Path,
    mutate: F,
) -> Result<PublishState, crate::api::AppError>
where
    F: FnOnce(&mut PublishState) -> bool,
{
    let _guard = yekitrim_core::filelock::lock_path(path).await;
    let mut state = load_state(path).await?;
    if mutate(&mut state) {
        save_state(path, &state).await?;
    }
    Ok(state)
}

async fn load_state(path: &Path) -> Result<PublishState, crate::api::AppError> {
    let path = path.to_path_buf();
    let state = blocking(move || {
        match std::fs::read_to_string(&path) {
            Ok(raw) => Ok(serde_json::from_str::<PublishState>(&raw).unwrap_or_default()),
            Err(_) => Ok(PublishState::default()),
        }
    })
    .await?;
    Ok(state)
}

async fn save_state(path: &Path, state: &PublishState) -> Result<(), crate::api::AppError> {
    let path = path.to_path_buf();
    let raw = serde_json::to_string_pretty(state)
        .map_err(|e| err(axum::http::StatusCode::INTERNAL_SERVER_ERROR, format!("状态序列化失败: {e}")))?;
    blocking(move || {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                err(
                    axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                    format!("创建状态目录失败: {e}"),
                )
            })?;
        }
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, raw).map_err(|e| {
            err(
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                format!("写发布状态失败 {}: {e}", tmp.display()),
            )
        })?;
        std::fs::rename(&tmp, &path).map_err(|e| {
            err(
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                format!("替换发布状态失败 {}: {e}", path.display()),
            )
        })?;
        Ok(())
    })
    .await?;
    Ok(())
}

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn task_id_of(v: &Value) -> Option<String> {
    let obj = v.as_array().and_then(|a| a.first()).unwrap_or(v);
    for key in ["taskId", "task_id", "id"] {
        if let Some(id) = obj.get(key).and_then(|x| x.as_str()) {
            return Some(id.to_string());
        }
    }
    None
}

async fn get_json(
    client: &reqwest::Client,
    base: &str,
    path: &str,
    key: &str,
) -> Result<Value, crate::api::AppError> {
    let url = format!("{base}{path}");
    let resp = client
        .get(&url)
        .header("Authorization", key)
        .send()
        .await
        .map_err(|e| err(axum::http::StatusCode::BAD_GATEWAY, format!("请求 biliLive-tools 失败: {e}")))?;
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(err(
            axum::http::StatusCode::BAD_GATEWAY,
            format!(
                "biliLive-tools {path} 返回 {status}: {}",
                text.chars().take(300).collect::<String>()
            ),
        ));
    }
    serde_json::from_str(&text)
        .map_err(|e| err(axum::http::StatusCode::BAD_GATEWAY, format!("biliLive-tools 响应不是 JSON: {e}")))
}

/// `/bili/validUploadParams` 的**纯文本**契约（biliLive-tools 3.22.2，实测）：
/// 成功 = `200` + `text/plain` + 正文 `success`（7 字节）；失败 = `400`/`500` + 纯文本原因
/// （`packages/http/src/routes/bili.ts`）。**它不回 JSON**。
///
/// ⚠️ 别拿 `post_json` 调它：那会让**每一片**都死在「biliLive-tools 响应不是 JSON」上，
/// `/bili/upload` 一步都走不到（2026-10-30 实测：预检 200 + `success`，而代码在找 JSON）。
async fn post_precheck(
    client: &reqwest::Client,
    base: &str,
    path: &str,
    key: &str,
    body: Value,
) -> Result<(), crate::api::AppError> {
    let url = format!("{base}{path}");
    let resp = client
        .post(&url)
        .header("Authorization", key)
        .json(&body)
        .send()
        .await
        .map_err(|e| err(axum::http::StatusCode::BAD_GATEWAY, format!("请求 biliLive-tools 失败: {e}")))?;
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    let brief = text.trim().chars().take(300).collect::<String>();
    if !status.is_success() {
        return Err(err(
            axum::http::StatusCode::BAD_GATEWAY,
            format!("biliLive-tools {path} 返回 {status}: {brief}"),
        ));
    }
    // 约定正文是 `success`；空正文也当通过（老版本可能什么都不回）。
    // 其余 2xx 正文一律当失败 —— 那多半就是"参数没通过"，静默放过等于把坏参数送进投稿。
    if !brief.is_empty() && !brief.eq_ignore_ascii_case("success") {
        return Err(err(
            axum::http::StatusCode::BAD_GATEWAY,
            format!("biliLive-tools 预检未通过: {brief}"),
        ));
    }
    Ok(())
}

async fn post_json(
    client: &reqwest::Client,
    base: &str,
    path: &str,
    key: &str,
    body: Value,
) -> Result<Value, crate::api::AppError> {
    let url = format!("{base}{path}");
    let resp = client
        .post(&url)
        .header("Authorization", key)
        .json(&body)
        .send()
        .await
        .map_err(|e| err(axum::http::StatusCode::BAD_GATEWAY, format!("请求 biliLive-tools 失败: {e}")))?;
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(err(
            axum::http::StatusCode::BAD_GATEWAY,
            format!(
                "biliLive-tools {path} 返回 {status}: {}",
                text.chars().take(300).collect::<String>()
            ),
        ));
    }
    if text.trim().is_empty() {
        return Ok(Value::Null);
    }
    serde_json::from_str(&text)
        .map_err(|e| err(axum::http::StatusCode::BAD_GATEWAY, format!("biliLive-tools 响应不是 JSON: {e}")))
}

async fn resolve_uid(
    client: &reqwest::Client,
    base: &str,
    key: &str,
    configured: u64,
) -> Result<u64, crate::api::AppError> {
    if configured != 0 {
        return Ok(configured);
    }
    let users = get_json(client, base, "/user/list", key).await?;
    users
        .as_array()
        .and_then(|a| a.first())
        .and_then(|u| u.get("uid"))
        .and_then(|v| v.as_u64())
        .ok_or_else(|| {
            err(
                axum::http::StatusCode::BAD_GATEWAY,
                "biliLive-tools 里没有已登录的 B站账号（/user/list 为空）",
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 出片回传**仅管理员**：普通账号不能借这台机器的 B站登录/上传预设投稿。
    ///
    /// 变异自证：去掉两个 handler 里的 `ensure_publisher(&user)?` ⇒ 本条红
    /// （GET 会走到 `configured:false` 的 200，POST 会走到 400/409，都不再是 403）。
    #[tokio::test]
    async fn publish_endpoints_are_admin_only() {
        use crate::api::test_support::{body_json, test_app};
        let t = test_app().await;
        let hash = yekitrim_service::auth::hash_password("bob-pass-1234").unwrap();
        let bob = t
            .auth
            .db()
            .create_user("bob", &hash, "user", false, 0)
            .unwrap();
        // ⚠️ 必须给 bob **自己的项目**再测：拿 admin 的 `project=1` 会先在 Scope 里
        // 被归属校验挡成 404（那是对的，但测不到 ensure_publisher）。
        let bob_root = t._dir.path().join("bobproj");
        std::fs::create_dir_all(bob_root.join("output/clips")).unwrap();
        std::fs::write(bob_root.join("a.mp4"), b"fake").unwrap();
        let bob_cfg = bob_root.join("yekitrim.toml");
        std::fs::write(&bob_cfg, "input_file = \"a.mp4\"\n").unwrap();
        let pid = t
            .auth
            .db()
            .create_project(bob, "bobproj", None, "local", Some("bob"), &bob_root, &bob_cfg, None)
            .unwrap();
        let bundle = t.auth.login("bob", "bob-pass-1234", None, None).unwrap();
        let uri = format!("/publish/bililive-tools?project={pid}");
        for (method, body) in [("GET", ""), ("POST", "{}")] {
            let res = t
                .call(
                    t.req_as(
                        &bundle.session_id,
                        &bundle.csrf,
                        method,
                        &uri,
                    )
                    .header(axum::http::header::CONTENT_TYPE, "application/json")
                    .body(axum::body::Body::from(body))
                    .unwrap(),
                )
                .await;
            assert_eq!(
                res.status(),
                axum::http::StatusCode::FORBIDDEN,
                "{method} 必须拒绝非管理员"
            );
            let _ = body_json(res).await;
        }
    }
    fn clip(dir: &str, rank: u32) -> ClipEntry {
        ClipEntry {
            file: format!("clips/{dir}/clip.mp4"),
            title: format!("标题{rank}"),
            summary: format!("摘要{rank}"),
            rank,
            directory: dir.into(),
        }
    }

    #[test]
    fn select_clips_accepts_rank_and_directory() {
        let all = vec![clip("clip-01", 1), clip("clip-02", 2)];
        let picked = select_clips(&all, &[json!(2), json!("clip-01")]).unwrap();
        assert_eq!(picked.len(), 2);
        assert_eq!(picked[0].directory, "clip-01");
        assert!(select_clips(&all, &[json!(99)]).is_err());
    }

    #[test]
    fn select_clips_rejects_more_than_the_cap_instead_of_silently_truncating() {
        let all: Vec<ClipEntry> = (1..=(MAX_CLIPS as u32 + 1))
            .map(|i| clip(&format!("clip-{i:02}"), i))
            .collect();
        let e = select_clips(&all, &[]).expect_err("超限必须报错，不许静默只发前 50 条");
        let text = format!("{e:?}");
        assert!(text.contains("最多发布"), "错误文案要让人知道是上限：{text}");
    }
    #[test]
    fn safe_component_rejects_traversal() {
        assert_eq!(safe_component("clip-01"), "clip-01");
        assert!(safe_component("../x").is_empty());
        assert!(safe_component("a/b").is_empty());
        assert!(safe_component("a\\b").is_empty());
    }

    /// 起一个只回一条固定响应的假 HTTP 上游（std only：不引测试依赖、端口随机）。
    fn spawn_fake_upstream(status_line: &'static str, body: &'static str) -> String {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            if let Ok((mut sock, _)) = listener.accept() {
                let mut buf = [0u8; 8192];
                let _ = sock.read(&mut buf);
                let resp = format!(
                    "{status_line}\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = sock.write_all(resp.as_bytes());
            }
        });
        format!("http://{addr}")
    }

    /// **预检的响应是纯文本**（实测 biliLive-tools 3.22.2：`200` + `text/plain` + `success`）。
    ///
    /// 这条曾经走 `post_json` ⇒ 每一片都死在「biliLive-tools 响应不是 JSON」，
    /// `/bili/upload` 一步都走不到（投稿功能整个不可用）。改回 JSON 解析这条立刻红。
    #[tokio::test]
    async fn precheck_accepts_plain_text_success() {
        let base = spawn_fake_upstream("HTTP/1.1 200 OK", "success");
        let client = reqwest::Client::new();
        post_precheck(
            &client,
            &base,
            "/bili/validUploadParams",
            "key",
            json!({"tid": 27}),
        )
        .await
        .expect("200 + `success` 必须算通过");
    }

    /// 失败时 biliLive-tools 回纯文本原因 —— 要把它带进错误，别只说"预检失败"。
    #[tokio::test]
    async fn precheck_reports_the_text_reason_on_failure() {
        let base = spawn_fake_upstream("HTTP/1.1 400 Bad Request", "分区不存在");
        let client = reqwest::Client::new();
        let e = post_precheck(
            &client,
            &base,
            "/bili/validUploadParams",
            "key",
            json!({}),
        )
        .await
        .expect_err("400 必须报错");
        let msg = err_text(&e);
        assert!(msg.contains("400") && msg.contains("分区不存在"), "错误里要带原文：{msg}");
    }

    /// 2xx 但正文不是 `success` ⇒ 当失败（那正是"参数没通过"的形状），不许静默放过。
    #[tokio::test]
    async fn precheck_rejects_2xx_that_is_not_success() {
        let base = spawn_fake_upstream("HTTP/1.1 200 OK", "invalid tid");
        let client = reqwest::Client::new();
        let e = post_precheck(&client, &base, "/bili/validUploadParams", "key", json!({}))
            .await
            .expect_err("2xx + 非 success 正文不算通过");
        assert!(err_text(&e).contains("invalid tid"), "{}", err_text(&e));
    }

    /// 同样字节数的新片**必须重拷**（重剪/重渲染），源没变则**不许**重拷（GB 级复制）。
    #[test]
    fn stage_clips_recopies_when_source_is_newer_even_with_same_len() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("output");
        let clips = root.join("clips").join("clip-01");
        std::fs::create_dir_all(&clips).unwrap();
        let src = clips.join("clip.mp4");
        let staging = dir.path().join("staging");
        std::fs::create_dir_all(&staging).unwrap();
        let entry = ClipEntry {
            file: "clips/clip-01/clip.mp4".into(),
            title: "标题".into(),
            summary: String::new(),
            rank: 1,
            directory: "clip-01".into(),
        };
        let staged_file = staging.join("7").join("clip-01.mp4");
        let stage = || {
            stage_clips(
                &root,
                7,
                std::slice::from_ref(&entry),
                &staging,
                "/app/video/_yekitrim_publish",
            )
            .unwrap()
        };

        std::fs::write(&src, b"AAAA").unwrap();
        stage();
        assert_eq!(std::fs::read(&staged_file).unwrap(), b"AAAA");

        // 同字节数的新版本：只比大小时这里会被当成"已是最新"，发出去的是上一版。
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(&src, b"BBBB").unwrap();
        stage();
        assert_eq!(
            std::fs::read(&staged_file).unwrap(),
            b"BBBB",
            "源比暂存新时，同样字节数也必须重拷"
        );

        // 源没变 ⇒ 一个字节都不重拷（暂存 mtime 不动）。
        let before = std::fs::metadata(&staged_file).unwrap().modified().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        stage();
        assert_eq!(
            std::fs::metadata(&staged_file).unwrap().modified().unwrap(),
            before,
            "源没变时不许重拷（GB 级视频）"
        );
    }

    fn sample_entry(dir: &str) -> ClipPublishState {
        ClipPublishState {
            title: format!("标题{dir}"),
            container_path: format!("/app/video/_publish/1/{dir}.mp4"),
            task_id: None,
            status: "submitted".into(),
            error: None,
            submitted_at: now_secs(),
            updated_at: now_secs(),
            response: None,
        }
    }

    /// 状态文件只能经 `update_state` 改：**落盘** + **没变化就不写**。
    ///
    /// 两个消费者（POST 发布 / GET 状态回填）都在写同一个文件：GET 那条原来不拿锁、
    /// 而且每次轮询都无条件重写 —— 一边发布一边刷新会把刚投的稿盖回"未投稿"。
    #[tokio::test]
    async fn update_state_persists_and_skips_untouched_writes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(STATE_FILE);

        let s = update_state(&path, |s| {
            s.clips.insert("clip-01".into(), sample_entry("clip-01"));
            true
        })
        .await
        .unwrap();
        assert_eq!(s.clips.len(), 1);
        // 真落盘了（不是只在内存里）
        let on_disk = std::fs::read_to_string(&path).unwrap();
        assert!(on_disk.contains("clip-01"), "状态必须落盘：{on_disk}");

        // 闭包说"没改" ⇒ 文件一个字节都不动（GET 轮询不该每 3 秒重写一次）
        let s = update_state(&path, |_| false).await.unwrap();
        assert_eq!(s.clips.len(), 1);
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            on_disk,
            "没有字段变化时不许重写状态文件"
        );

        // 第二次真的改（新增一条）⇒ 覆盖写回，且旧条目还在（读-改-写不是整份替换）
        let s = update_state(&path, |s| {
            s.clips.insert("clip-02".into(), sample_entry("clip-02"));
            true
        })
        .await
        .unwrap();
        assert_eq!(s.clips.len(), 2);
        let raw = std::fs::read_to_string(&path).unwrap();
        assert!(raw.contains("clip-01") && raw.contains("clip-02"), "旧条目不许被吞：{raw}");
    }

    #[test]
    fn task_id_parses_common_shapes() {
        assert_eq!(task_id_of(&json!({"taskId":"a"})).as_deref(), Some("a"));
        assert_eq!(task_id_of(&json!([{"id":"b"}])).as_deref(), Some("b"));
        assert!(task_id_of(&json!({"ok":true})).is_none());
    }
}
