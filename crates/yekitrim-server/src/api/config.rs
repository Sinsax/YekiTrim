//! `api::config`：程序配置、ASR 模型与 LLM 测试。

use std::path::Path;
use std::sync::Arc;

use axum::{
    extract::{Extension, State},
    http::StatusCode,
    Json,
};
use yekitrim_service::Workspace;

use crate::model::AppState;
use crate::jobs::{JobClass, JobSpec};
use super::*;

pub(super) async fn models_get() -> ApiResult {
    // ⚠️ 同步 IO：`load_global_config` + 每个模型的 `model_install`（3 个候选目录 × N 个模型）都要
    // stat 磁盘 —— 项目/程序目录在网络盘上时就是秒级。搬出 async worker（同 `config_get` 的包法）。
    blocking(models_impl).await
}

fn models_impl() -> ApiResult {
    // 判定与运行时同源（`download::model_install`：配置目录 → 用户目录 → 程序目录 + marker），
    // 列表也来自注册表 `model_names()` —— 别再手写模型数组，也别只看程序目录：
    // 曾经界面按"<exe>/.vtrim/models 有没有这个目录"判，而 qwen3-asr 放在用户目录
    // `~/.vtrim/models/qwen3-asr`，于是显示"未下载"、实际转写一直在用它。
    let asr = yekitrim_core::config::load_global_config().asr;
    let out: Vec<serde_json::Value> = yekitrim_core::download::model_names().iter().map(|name| {
        let st = yekitrim_core::download::model_install(&asr, name);
        serde_json::json!({
            "name": name,
            "installed": st.installed,
            "dir": yekitrim_core::path::display(&st.dir),
            "source": st.source.as_str(),
            "source_label": st.source.label(),
            "size_label": match *name { "sensevoice"=>"165 MB", _=>"879 MB" },
            "note": match *name { "sensevoice"=>"快", _=>"最准最慢" },
        })
    }).collect();
    ok(serde_json::json!(out))
}

/// `GET /api/config`：**程序配置**（`.vtrim/config.toml` + `.vtrim/llm.toml`）的读取。
///
/// ⚠️ **这条路是"项目无关"的**（2026-10-03 随全局工程一起收拾干净）：设置页五块内容改的都是
/// **程序级**配置，编辑的是 global-only 的值 —— merged config 会被某个工程的 `vtrim.toml`
/// 覆盖，直接用就会出现「表单值和源文件对不上」。
///
/// 于是原来那三样项目相关的东西一并去掉，它们**没有任何消费方**：
///   - `config_file` / `root_dir`：前端从没读过（项目设置页走的是 `/api/project-data/config`，
///     它有自己那份 `yekitrim_raw`）；留着只会让人以为"这里能看当前工程"；
///   - `yekitrim_raw`：同上，那是项目设置页的字段，属于 `/api/project-data/config`；
///   - `config`：那份 **merged** 配置本来就是"当前工程"的概念，与"读程序配置"自相矛盾
///     （前端注释自己都写了 `global_config || config` —— 即 `config` 只是老响应缺字段时的兜底）。
///
/// 要读某个工程的配置：`GET /api/project-data/config?project=<id>`。
/// 要读合并后的生效配置：也走那条（它按项目解析）。
pub(super) fn config_get_impl() -> ApiResult {
    let home = yekitrim_core::path::vtrim_home();
    // ⚠️ 同步 IO：读 config.toml / llm.toml 两份（含 API key），调用方必须下 blocking 池。
    let global_raw = std::fs::read_to_string(home.join("config.toml")).unwrap_or_default();
    let llm_raw = std::fs::read_to_string(home.join("llm.toml")).unwrap_or_default();
    let global_cfg = yekitrim_core::config::load_global_config();
    let llm_cfg = yekitrim_core::config::load_llm_config();
    // ⚠️ **`config_error` 必须留着**：设置页表单的写入门闸就是它
    // （`SettingsView.vue::loadConfig` 把它存进 `configError`）—— 程序配置解析失败时那份配置会被
    // **整体忽略**、表单只剩内建默认值，用户不改也不知道；这时若还允许保存，
    // 一次「保存」就把内建默认值覆盖到真实配置文件上。所以这里如实报出原因。
    // 2026-10-03 把这条路改成"纯程序配置"时差点把它一起删掉 —— 它的语义跟着从
    // "merged 配置的解析错误"变成"程序配置的解析错误"，比原来更准确。
    let config_error = yekitrim_core::config::global_config_parse_error().unwrap_or_default();
    ok(serde_json::json!({
        "global_raw": global_raw,
        "llm_raw": llm_raw,
        "global_config": serde_json::to_value(global_cfg).unwrap_or_default(),
        "llm_config": serde_json::to_value(llm_cfg).unwrap_or_default(),
        "config_error": config_error,
    }))
}

pub(super) async fn config_get(State(_state): State<Arc<AppState>>) -> ApiResult {
    // ⚠️ 同步 IO：读 config.toml / llm.toml 两份 + 解析（含 llm.toml 的 API key）。
    // `project_data.rs` 里同一个 impl 早就这么包了，这条 `/api/config` 路由漏了。
    blocking(config_get_impl).await
}

/// 项目配置里会被当作**路径**的键：`(section, key)`。
///
/// ⚠️ 这些值会一路进 `Workspace::resolve`（`service/workspace.rs`），而它对**绝对路径原样放行**
/// （第 49-56 行），所以这里不校验就等于「普通用户把 `input_file` 指到别人的录像（转写完把
/// 字幕读出来）、把 `output_dir` 指到任意目录（产物写到项目外）」。**新增路径键时同步加进来。**
///
/// `section` 支持点号路径（如 `ingest.recorder`），与 TOML 的表名一一对应。
pub(super) const PROJECT_PATH_KEYS: [(&str, &str); 9] = [
    ("project", "input_file"),
    ("project", "danmaku_file"),
    ("project", "transcript_file"),
    ("project", "audio_file"),
    ("project", "events_file"),
    ("cutting", "output_dir"),
    // ⚠️ `[ingest]` 这几个是**后补的**：它们由 `config/mod.rs` 的 merge 从**项目** `vtrim.toml`
    // 并进生效配置（`ingest.ytdlp_path` / `cookie_file` / `recorder.root`），
    // 于是非 admin 可以经 `PUT /api/project-data/config` 写进去。
    // 今天没有实弹：消费侧（`download_worker` / `ingest`）读的是**全局**配置，而全局只有 admin 能写。
    // 但 `registry::find_ytdlp` 接受配置里的路径时**不做 sha256 校验**就进 `CommandSpec.program`
    // ——「下一次改成读工作区配置」的重构会让它变成任意程序执行。所以现在就把面收掉。
    ("ingest", "ytdlp_path"),
    ("ingest", "cookie_file"),
    ("ingest.recorder", "root"),
];

/// 校验配置里的路径键必须落在本项目内（**只对非 admin 生效**）。
///
/// admin 不受此限：用「服务器文件」建项目时 `input_file` 本来就是项目外的绝对路径
/// （`service/projects.rs` 对 admin 的绝对路径不做租户根限制），卡住它会打断既有工作流；
/// 而 admin 已经能浏览服务器目录、在任意路径建项目，这里收紧并不增加实际安全性。
pub(super) fn check_project_paths(ws: &Workspace, value: &serde_json::Value, is_admin: bool) -> Result<(), AppError> {
    if is_admin {
        return Ok(());
    }
    for (section, key) in PROJECT_PATH_KEYS {
        // `section` 允许点号路径（`ingest.recorder`）：逐级下钻，任一级缺失就跳过这条键
        let mut node = value;
        let mut missing = false;
        for part in section.split('.') {
            match node.get(part) {
                Some(next) => node = next,
                None => {
                    missing = true;
                    break;
                }
            }
        }
        if missing {
            continue;
        }
        let Some(raw) = node.get(key).and_then(|v| v.as_str()) else {
            continue;
        };
        let raw = raw.trim();
        if raw.is_empty() {
            continue;
        }
        // `jail_join` 同时挡住词法越界（绝对路径 / 盘符 / UNC / `..` 段）与符号链接越界
        yekitrim_core::tenant::jail_join(&ws.root_dir, raw).map_err(|e| {
            err(
                StatusCode::BAD_REQUEST,
                format!("[{section}] {key} 必须是项目内相对路径（{raw}）: {e:#}"),
            )
        })?;
    }
    check_material_paths(ws, value)
}

/// `[project.materials]` 里每条素材的 `path` 也是路径 —— 按同一口径校验。
///
/// 清单本身不改管线读什么（下游只认 `*_file`），但它是**要 stat 的**：让非 admin 往里塞
/// 一条 `C:\...\别人的文件.srt`，预处理页就会把那台机器上的文件存在性与大小回显出来。
/// 与 [`set_asset`]（选中时才 `jail_join`）是同一条规矩，只是这里堵的是直接改配置那条路。
///
/// [`set_asset`]: yekitrim_service::assets::set_asset
fn check_material_paths(ws: &Workspace, value: &serde_json::Value) -> Result<(), AppError> {
    let Some(materials) = value.get("project").and_then(|p| p.get("materials")) else {
        return Ok(());
    };
    for (kind, list) in materials.as_object().into_iter().flatten() {
        for raw in list.as_array().into_iter().flatten().filter_map(|e| {
            e.get("path")
                .and_then(|p| p.as_str())
                .or_else(|| e.as_str())
        }) {
            let raw = raw.trim();
            if raw.is_empty() {
                continue;
            }
            yekitrim_core::tenant::jail_join(&ws.root_dir, raw).map_err(|e| {
                err(
                    StatusCode::BAD_REQUEST,
                    format!("[project.materials] {kind} 的素材必须是项目内相对路径（{raw}）: {e:#}"),
                )
            })?;
        }
    }
    Ok(())
}

/// `GET /api/project-data/config`：**某个工程**的配置读取（项目设置页 / 预处理页）。
///
/// 与 `config_get_impl`（程序配置，项目无关）**故意分开**：这里返回的是这个工程的 `vtrim.toml`
/// 原文与**合并后**的生效配置（全局 + 工程 + llm），那才是"这个工程实际会用的值"。
/// 两者曾经共用一份实现，于是项目相关字段（`config_file`/`root_dir`/`yekitrim_raw`/`config`）
/// 被一并挂在项目无关的 `/api/config` 上 —— 2026-10-03 拆开。
pub(super) fn config_vtrim_get_impl(ws: &Workspace) -> ApiResult {
    // ⚠️ 同步 IO：读 vtrim.toml + 解析合并，调用方下 blocking 池。
    let yekitrim_raw = std::fs::read_to_string(&ws.config_file).unwrap_or_default();
    let (config, config_error) = match ws.load_config() {
        Ok(c) => (serde_json::to_value(c).unwrap_or_default(), String::new()),
        // 配置解析失败时配置会被整体忽略，必须让前端能显示原因，否则表单只会静默回退到默认值
        Err(e) => (serde_json::Value::Null, format!("{e:#}")),
    };
    ok(serde_json::json!({
        "config_file": yekitrim_core::path::display(&ws.config_file),
        "root_dir": yekitrim_core::path::display(&ws.root_dir),
        "yekitrim_raw": yekitrim_raw,
        "config": config,
        "config_error": config_error,
    }))
}

pub(super) fn config_vtrim_put_impl(ws: &Workspace, body: &serde_json::Value, is_admin: bool) -> ApiResult {
    // patch 模式：`{ section: { key: value } }` 合并写（保留注释与其他 section），
    // 前端设置表单用它，避免"读整份 TOML → 正则替换 → 写回"这种易碎做法。
    if let Some(patch) = body.get("patch") {
        check_project_paths(ws, patch, is_admin)?;
        yekitrim_core::config::patch_config_file(&ws.config_file, patch)
            .map_err(|e| err(StatusCode::BAD_REQUEST, format!("项目配置保存失败: {e:#}")))?;
        return ok(serde_json::json!({ "ok": true }));
    }
    let raw = body.get("raw").and_then(|v| v.as_str()).or_else(|| body.get("content").and_then(|v| v.as_str()));
    let Some(raw) = raw else { return Err(err(StatusCode::BAD_REQUEST, "缺少 raw 字段")); };
    // 与 config_global_put / config_llm_put 对齐：空内容或语法错误会把整个项目配置
    // 变成不可解析（= 配置被整体忽略），必须在落盘前拦下。
    if raw.trim().is_empty() {
        return Err(err(StatusCode::BAD_REQUEST, "内容为空，未保存"));
    }
    yekitrim_service::validate_toml(raw)
        .map_err(|e| err(StatusCode::BAD_REQUEST, format!("yekitrim.toml 不是合法 TOML，未保存: {e}")))?;
    // 源文件模式同样要过路径校验：TOML 解析出来的结构与 JSON 一致，直接复用上面那张键表
    if !is_admin {
        let parsed: toml::Value = toml::from_str(raw)
            .map_err(|e| err(StatusCode::BAD_REQUEST, format!("yekitrim.toml 解析失败，未保存: {e}")))?;
        let as_json = serde_json::to_value(&parsed).unwrap_or(serde_json::Value::Null);
        check_project_paths(ws, &as_json, is_admin)?;
    }
    if let Some(parent) = ws.config_file.parent() { std::fs::create_dir_all(parent).map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("创建目录失败: {e}")))?; }
    std::fs::write(&ws.config_file, raw).map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("保存失败: {e}")))?;
    ok(serde_json::json!({ "ok": true }))
}

/// 保存程序配置。
///
/// - `raw`：源文件模式，整份覆盖写；
/// - `patch`：表单模式，`{ section: { key: value } }` 合并写，未涉及的 section/注释原样保留。
pub(super) async fn config_global_put(State(_state): State<Arc<AppState>>, Json(body): Json<serde_json::Value>) -> ApiResult {
    let path = yekitrim_core::path::vtrim_home().join("config.toml");
    // patch / 源文件两种写法都会读改写整份配置，加锁避免并发提交互相覆盖；
    // 锁在 await 里持有，读改写下 blocking 池（`with_path_lock` 的闭包跑在当前线程）
    let _guard = yekitrim_core::filelock::lock_path(&path).await;
    blocking(move || config_global_put_locked(&path, &body)).await
}

pub(super) fn config_global_put_locked(path: &Path, body: &serde_json::Value) -> ApiResult {
    if let Some(patch) = body.get("patch") {
        yekitrim_core::config::patch_config_file(path, patch)
            .map_err(|e| err(StatusCode::BAD_REQUEST, format!("配置保存失败: {e:#}")))?;
        return ok(serde_json::json!({ "ok": true }));
    }
    let raw = body.get("raw").and_then(|v| v.as_str()).or_else(|| body.get("content").and_then(|v| v.as_str()));
    let Some(raw) = raw else { return Err(err(StatusCode::BAD_REQUEST, "缺少 raw 或 patch 字段")); };
    // 语法错误会令整份配置被忽略，这里直接拦下，别让用户存进一个坏文件
    yekitrim_service::validate_toml(raw)
        .map_err(|e| err(StatusCode::BAD_REQUEST, format!("config.toml 不是合法 TOML，未保存: {e}")))?;
    if let Some(parent) = path.parent() { std::fs::create_dir_all(parent).map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("创建目录失败: {e}")))?; }
    std::fs::write(path, raw).map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("保存失败: {e}")))?;
    ok(serde_json::json!({ "ok": true }))
}

/// 保存 LLM 配置：`raw` 整份覆盖写，`patch` 合并写（保留注释与其他键）。
pub(super) async fn config_llm_put(State(_state): State<Arc<AppState>>, Json(body): Json<serde_json::Value>) -> ApiResult {
    let path = yekitrim_core::path::vtrim_home().join("llm.toml");
    let _guard = yekitrim_core::filelock::lock_path(&path).await;
    blocking(move || config_llm_put_locked(&path, &body)).await
}

pub(super) fn config_llm_put_locked(path: &Path, body: &serde_json::Value) -> ApiResult {
    if let Some(patch) = body.get("patch") {
        yekitrim_core::config::patch_config_file(path, patch)
            .map_err(|e| err(StatusCode::BAD_REQUEST, format!("LLM 配置保存失败: {e:#}")))?;
        return ok(serde_json::json!({ "ok": true }));
    }
    let raw = body.get("raw").and_then(|v| v.as_str()).or_else(|| body.get("content").and_then(|v| v.as_str()));
    let Some(raw) = raw else { return Err(err(StatusCode::BAD_REQUEST, "缺少 raw 或 patch 字段")); };
    yekitrim_service::validate_toml(raw)
        .map_err(|e| err(StatusCode::BAD_REQUEST, format!("llm.toml 不是合法 TOML，未保存: {e}")))?;
    if let Some(parent) = path.parent() { std::fs::create_dir_all(parent).map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("创建目录失败: {e}")))?; }
    std::fs::write(path, raw).map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("保存失败: {e}")))?;
    ok(serde_json::json!({ "ok": true }))
}

pub(super) async fn llm_test() -> ApiResult {
    // 只有"读配置"是同步 IO（HTTP 调用本身是 async，保持不动）
    let llm = blocking(|| Ok(yekitrim_core::config::load_llm_config())).await?;
    if llm.api_key.is_empty() {
        return Err(err(StatusCode::BAD_REQUEST, "LLM API key 未配置"));
    }
    let start = std::time::Instant::now();
    let test_ok = yekitrim_core::detection::test_llm(&llm, "ping").await.is_ok();
    let latency_ms = start.elapsed().as_millis() as u64;
    if test_ok {
        ok(serde_json::json!({ "ok": true, "latency_ms": latency_ms, "model": llm.model }))
    } else {
        Err(err(StatusCode::INTERNAL_SERVER_ERROR, format!("LLM 连接失败 ({latency_ms}ms)")))
    }
}

/// 下载全部 ASR 模型（**仅 admin**：平台级资源，见 §5.3）。
///
/// 前端 `POST /api/admin/models/download` 不带 body（旧实现要求 `Json` 提取器，空 body 会被
/// axum 直接 400，两个模型界面都因此不可用）。这里只做任务编排，不读 body。
pub(super) async fn models_download(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<crate::auth::CurrentUser>,
) -> ApiResult {
    if !user.is_admin() {
        return Err(err(StatusCode::FORBIDDEN, "仅管理员可下载平台模型"));
    }
    // 全局模型目录：与任何会 ensure_models 的转写任务互斥（排队）。
    // 略重档 + 后台优先级：纯网络下载，不该插在用户点按的任务前面。
    let handle = state
        .jobs
        .create(
            user.id,
            JobSpec::new(
                "download_models",
                "下载 ASR 模型",
                vec![crate::scope_key::MODELS.to_string()],
                JobClass::Medium,
            )
            .background(),
        )
        .await;
    let id = handle.id.clone();
    let queued = handle.queued;
    let jobs = state.jobs.clone();
    let job_id = id.clone();
    // 下载完要解包（tar.bz2，纯同步 IO）→ 任务线程
    spawn_job(jobs.clone(), job_id.clone(), move || async move {
        if handle.wait_start().await.is_err() {
            jobs.complete(&job_id).await;
            return;
        }
        let result = yekitrim_core::download::download_all(None).await;
        if let Err(e) = result {
            jobs.fail(&job_id, format!("{e:#}")).await;
            return;
        }
        jobs.complete(&job_id).await;
    });
    ok(serde_json::json!({ "ok": true, "id": id, "queued": queued }))
}







#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::test_support::*;


    use axum::body::Body;


/// `/api/models` 的每条必须带上「实际路径 + 来源」：判定跟运行时同一套查找顺序
/// （配置 → 用户目录 → 程序目录），界面照它显示。曾经只回 `dir.exists()`（程序目录），
/// qwen3-asr 放在用户目录 → 界面说"未下载"却一直在用它（用户报过）。
#[tokio::test]
async fn models_endpoint_reports_runtime_resolved_dir_and_source() {
    let t = test_app().await;
    let res = t.call(t.req("GET", "/models").body(Body::empty()).unwrap()).await;
    assert_eq!(res.status(), StatusCode::OK);
    let v = body_json(res).await;
    let arr = v.as_array().expect("array");
    let names: Vec<&str> = arr.iter().filter_map(|m| m["name"].as_str()).collect();
    // 注册表驱动：firered-zh 已下线，不该再出现在清单里
    assert_eq!(names, vec!["sensevoice", "qwen3-asr"], "{v}");
    for m in arr {
        let name = m["name"].as_str().unwrap();
        assert!(m["installed"].is_boolean(), "{m}");
        let dir = m["dir"].as_str().unwrap_or("");
        assert!(Path::new(dir).ends_with(std::path::Path::new("models").join(name)), "{m}");
        assert!(["config", "user", "exe"].contains(&m["source"].as_str().unwrap_or("")), "{m}");
        assert!(!m["source_label"].as_str().unwrap_or("").is_empty(), "{m}");
    }
}

/// 项目配置里的路径键必须落在本项目内（非 admin）。
///
/// 回归钉子：`config_vtrim_put_impl` 以前只校验"是合法 TOML"，于是普通用户能把
/// `input_file` 改成别人的录像（转写完把字幕读出来）或把 `output_dir` 指到任意目录。
#[test]
fn project_config_paths_are_jailed_for_non_admin() {
    let root = std::env::temp_dir().join(format!("vtrim-cfg-paths-{}", std::process::id()));
    std::fs::create_dir_all(root.join("output")).unwrap();
    let ws = Workspace { config_file: root.join("yekitrim.toml"), root_dir: root.clone() };

    // 正常形态放行：相对路径、空串、缺键
    let ok = serde_json::json!({
        "project": { "input_file": "records/a.mp4", "danmaku_file": "" },
        "cutting": { "output_dir": "./output" }
    });
    assert!(check_project_paths(&ws, &ok, false).is_ok(), "{ok}");

    // 绝对路径 → 400（读别人录像 / 写到项目外）
    let abs_in = serde_json::json!({ "project": { "input_file": "C:/other/clip.mp4" } });
    assert_eq!(
        check_project_paths(&ws, &abs_in, false).unwrap_err().0,
        StatusCode::BAD_REQUEST
    );
    let abs_out = serde_json::json!({ "cutting": { "output_dir": "D:/tmp/out" } });
    assert_eq!(
        check_project_paths(&ws, &abs_out, false).unwrap_err().0,
        StatusCode::BAD_REQUEST
    );
    // `..` 越界 → 400
    let up = serde_json::json!({ "project": { "audio_file": "../../secret.wav" } });
    assert_eq!(
        check_project_paths(&ws, &up, false).unwrap_err().0,
        StatusCode::BAD_REQUEST
    );

    // admin 不受限：「服务器文件」来源建的项目里 input_file 本来就是项目外的绝对路径
    assert!(check_project_paths(&ws, &abs_in, true).is_ok());
    assert!(check_project_paths(&ws, &abs_out, true).is_ok());

    // ⚠️ `[ingest]` 的路径键（后补的）：它们在 `config/mod.rs` 里会从**项目**配置 merge 进
    // 生效配置，非 admin 曾经可以写进去。`ingest.recorder` 是**两级**表名，走点号下钻。
    let ingest_ytdlp = serde_json::json!({ "ingest": { "ytdlp_path": "C:/evil/yt-dlp.exe" } });
    assert_eq!(
        check_project_paths(&ws, &ingest_ytdlp, false).unwrap_err().0,
        StatusCode::BAD_REQUEST
    );
    let ingest_cookie = serde_json::json!({ "ingest": { "cookie_file": "../../cookies.txt" } });
    assert_eq!(
        check_project_paths(&ws, &ingest_cookie, false).unwrap_err().0,
        StatusCode::BAD_REQUEST
    );
    let ingest_root = serde_json::json!({ "ingest": { "recorder": { "root": "D:/brec" } } });
    assert_eq!(
        check_project_paths(&ws, &ingest_root, false).unwrap_err().0,
        StatusCode::BAD_REQUEST
    );
    // 相对且在项目内 → 放行；缺表 / 缺键不误报
    let ingest_ok = serde_json::json!({
        "ingest": { "ytdlp_path": "bin/yt-dlp.exe", "recorder": { "root": "records" } }
    });
    assert!(check_project_paths(&ws, &ingest_ok, false).is_ok(), "{ingest_ok}");
    // `[ingest.recorder]` 里非路径的兄弟键（api_base / api_user）不该被当路径校验
    let ingest_siblings = serde_json::json!({
        "ingest": { "recorder": { "api_base": "http://10.0.0.5:2356", "api_user": "u" } }
    });
    assert!(check_project_paths(&ws, &ingest_siblings, false).is_ok(), "{ingest_siblings}");
    // admin 不受限（全局采集配置本来就是绝对路径）
    assert!(check_project_paths(&ws, &ingest_ytdlp, true).is_ok());
    assert!(check_project_paths(&ws, &ingest_root, true).is_ok());

    // 素材清单里的 path 同口径：清单只是"要 stat 的候选"，塞一条项目外的进去
    // 就等于让预处理页回显那台机器上的文件存在性与大小
    let materials_ok = serde_json::json!({
        "project": { "materials": { "transcript": [{ "path": "output/video.srt", "source": "asr", "mode": "hybrid" }] } }
    });
    assert!(check_project_paths(&ws, &materials_ok, false).is_ok());
    let materials_abs = serde_json::json!({
        "project": { "materials": { "danmaku": [{ "path": "C:/other/dm.xml" }] } }
    });
    assert_eq!(
        check_project_paths(&ws, &materials_abs, false).unwrap_err().0,
        StatusCode::BAD_REQUEST
    );
    let materials_up = serde_json::json!({
        "project": { "materials": { "transcript": [{ "path": "../outside.srt" }] } }
    });
    assert_eq!(
        check_project_paths(&ws, &materials_up, false).unwrap_err().0,
        StatusCode::BAD_REQUEST
    );
    // admin 加的工程外素材（列表里那条绝对路径）不该被拦
    assert!(check_project_paths(&ws, &materials_abs, true).is_ok());

    let _ = std::fs::remove_dir_all(&root);
}

/// 设置页「清空 = 跟随全局」这条链路：patch 里的 `null` 一路走到 llm.toml 的**键删除**。
///
/// 回归钉子：patch 从前把 null 写成空串（`max_tokens = ""`）—— 文件看着合法，
/// 下次 `load_llm_config` 解析 u32 失败，整份 LLM 配置（含 api_key）被打回默认值。
#[test]
fn llm_put_null_patch_removes_override_key() {
    let dir = std::env::temp_dir().join(format!("vtrim-llm-put-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let p = dir.join("llm.toml");
    std::fs::write(
        &p,
        "[llm]\napi_key = \"k\"\nmax_tokens = 16384\n\n[llm.overrides.detect]\nmax_tokens = 8192\n",
    )
    .unwrap();

    let body = serde_json::json!({
        "patch": {"llm": {"overrides": {"detect": {"max_tokens": null}}}}
    });
    let _ = config_llm_put_locked(&p, &body).unwrap();

    let out = std::fs::read_to_string(&p).unwrap();
    assert!(!out.contains("8192"), "被 null 的覆盖值该删掉：\n{out}");
    assert!(out.contains("api_key = \"k\""), "其余键不能动：\n{out}");
    assert!(out.contains("max_tokens = 16384"), "全局值不能动：\n{out}");
    let _ = std::fs::remove_dir_all(&dir);
}
}
