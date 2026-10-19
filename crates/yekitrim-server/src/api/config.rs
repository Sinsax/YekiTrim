//! `api::config`：程序配置、ASR 模型与 LLM 测试。

use std::path::Path;
use std::sync::Arc;

use axum::{
    extract::{Extension, Query, State},
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
    // 曾经界面按"<exe>/.yekitrim/models 有没有这个目录"判，而 qwen3-asr 放在用户目录
    // `~/.yekitrim/models/qwen3-asr`，于是显示"未下载"、实际转写一直在用它。
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

/// `GET /api/config`：**程序配置**（`.yekitrim/config.toml` + `.yekitrim/llm.toml`）的读取。
///
/// ⚠️ **这条路是"项目无关"的**（2026-10-03 随全局工程一起收拾干净）：设置页五块内容改的都是
/// **程序级**配置，编辑的是 global-only 的值 —— merged config 会被某个工程的 `yekitrim.toml`
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
/// 把 `llm.toml` 原文里 `api_key` 的**值**抹成空串（键名、缩进、行号、其它键一律不动）。
///
/// 为什么要抹：这份原文是设置页「编辑源文件」的内容，**明文 key 一个字都不许出接口**。
/// 为什么在**文本**上做而不是重序列化：重序列化（`toml::to_string`）会把注释全丢掉，
/// 而模板里大段说明都写在注释里 —— 那等于把源文件编辑器毁了。
/// 为什么连**注释掉的老 key** 也抹：`# api_key = "sk-…"` 同样是明文。
/// 行尾注释一并抹掉（注释里也可能写着 key）—— 宁可多抹一个注解，不可漏一个字。
fn redact_api_key(raw: &str) -> String {
    raw.split('\n')
        .map(|line| {
            // 逐层剥：缩进 → 注释符 → 缩进；剥不动就原样返回（注释行、别的键都走这条）
            let body = line.trim_start().trim_start_matches('#').trim_start();
            let Some(rest) = body.strip_prefix("api_key") else { return line.to_string() };
            let rest = rest.trim_start();
            let Some(rest) = rest.strip_prefix('=') else { return line.to_string() };
            // 等号后面那段空白**跟着等号一起留**（写成 api_key = 双引号，读起来才像原样）；
            // rest 始终是 line 的连续后缀 ⇒ 长度差就是切点，不用自己数列
            let rest = rest.trim_start();
            let head = &line[..line.len() - rest.len()];
            format!("{head}\"\"")
        })
        .collect::<Vec<_>>()
        .join("\n")
}

pub(super) fn config_get_impl(owner: yekitrim_core::owner::Owner) -> ApiResult {
    // ⚠️ 按 **Owner** 取目录：单人 = 程序目录（现状），多用户 = 该账号自己的 `.yekitrim/`。
    // **别退回 `path::yekitrim_home()`** —— 那会让所有账号共用同一份配置（含明文 LLM key）。
    let home = yekitrim_core::owner::state_dir(owner);
    // ⚠️ 同步 IO：读 config.toml / llm.toml 两份（含 API key），调用方必须下 blocking 池。
    let global_raw = std::fs::read_to_string(home.join("config.toml")).unwrap_or_default();
    // ⚠️ **明文 key 一个字都不出接口**（2026-10-19）：`llm_raw`（源文件编辑器）里那行的值在这里抹掉。
    // 配套的**写侧**口径「空 api_key = 不修改」在 `config_llm_put_locked` —— 两份合起来，
    // 「读出来（抹过）→ 原样存回去」不会把真 key 抹掉。**只删一边就是静默毁配置。**
    let llm_raw = redact_api_key(&std::fs::read_to_string(home.join("llm.toml")).unwrap_or_default());
    let global_cfg = yekitrim_core::config::load_config_for(owner);
    let mut llm_cfg = yekitrim_core::config::load_llm_config_for(owner);
    // 「有没有配 key」以后只由这个布尔回答（界面不再回显 key 本身）：真值 = llm.toml 里非空
    let api_key_set = !llm_cfg.api_key.trim().is_empty();
    // 解析出来的那份结构里也有一份明文（JSON 字段 `llm_config.api_key`），同样出接口 —— 一起清掉
    llm_cfg.api_key.clear();
    // ⚠️ **`config_error` 必须留着**：设置页表单的写入门闸就是它
    // （`SettingsView.vue::loadConfig` 把它存进 `configError`）—— 程序配置解析失败时那份配置会被
    // **整体忽略**、表单只剩内建默认值，用户不改也不知道；这时若还允许保存，
    // 一次「保存」就把内建默认值覆盖到真实配置文件上。所以这里如实报出原因。
    // 2026-10-03 把这条路改成"纯程序配置"时差点把它一起删掉 —— 它的语义跟着从
    // "merged 配置的解析错误"变成"程序配置的解析错误"，比原来更准确。
    let config_error = yekitrim_core::config::config_parse_error_for(owner).unwrap_or_default();
    // ⚠️ **`reference_titles_default` = 设置页"留空实际会用哪几条"**：
    //    这几条写死在 `detection::DEFAULT_REFERENCE_TITLES`，`config.toml` 里的
    //    `reference_titles = []` **不代表没有参考标题** —— 运行时照样注入内建那几条。
    //    不给前端看，用户在设置页只会看到一个空框、以为没配（实测被问过）。
    //    ⚠️ 前端**只拿它做灰字预览，绝不预填进输入框**：预填会让用户"以为是自己的"，
    //    一保存就把内建那几条固化进 config.toml，从此再也回不到内建更新。
    let reference_titles_default: Vec<&str> =
        yekitrim_core::detection::DEFAULT_REFERENCE_TITLES.to_vec();
    ok(serde_json::json!({
        "global_raw": global_raw,
        "llm_raw": llm_raw,
        "global_config": serde_json::to_value(global_cfg).unwrap_or_default(),
        "llm_config": serde_json::to_value(llm_cfg).unwrap_or_default(),
        // ⚠️ 表单面**只回「有没有」、不回明文**（与采集配置的 `api_pass_set` 同一条规矩）：
        // 界面靠它显示"已设置/未设置"，值本身永远不会回给前端
        "api_key_set": api_key_set,
        "config_error": config_error,
        "reference_titles_default": reference_titles_default,
    }))
}

pub(super) async fn config_get(
    Extension(user): Extension<crate::auth::CurrentUser>,
    State(_state): State<Arc<AppState>>,
) -> ApiResult {
    // ⚠️ 同步 IO：读 config.toml / llm.toml 两份 + 解析（含 llm.toml 的 API key）。
    // `project_data.rs` 里同一个 impl 早就这么包了，这条 `/api/config` 路由漏了。
    let owner = crate::auth::owner_of(&user);
    blocking(move || config_get_impl(owner)).await
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
    // ⚠️ `[ingest]` 这几个是**后补的**：它们由 `config/mod.rs` 的 merge 从**项目** `yekitrim.toml`
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
/// 与 `config_get_impl`（程序配置，项目无关）**故意分开**：这里返回的是这个工程的 `yekitrim.toml`
/// 原文与**合并后**的生效配置（全局 + 工程 + llm），那才是"这个工程实际会用的值"。
/// 两者曾经共用一份实现，于是项目相关字段（`config_file`/`root_dir`/`yekitrim_raw`/`config`）
/// 被一并挂在项目无关的 `/api/config` 上 —— 2026-10-03 拆开。
pub(super) fn config_yekitrim_get_impl(ws: &Workspace) -> ApiResult {
    // ⚠️ 同步 IO：读 yekitrim.toml + 解析合并，调用方下 blocking 池。
    let yekitrim_raw = std::fs::read_to_string(&ws.config_file).unwrap_or_default();
    let (config, config_error) = match ws.load_config() {
        Ok(c) => (serde_json::to_value(c).unwrap_or_default(), String::new()),
        // 配置解析失败时配置会被整体忽略，必须让前端能显示原因，否则表单只会静默回退到默认值
        Err(e) => (serde_json::Value::Null, format!("{e:#}")),
    };
    // `config` 是**合并后**的生效值（项目覆盖全局），看不出"这一项到底是项目写的还是继承来的"。
    // 预处理页要区分这两者（界面上是"留空 = 跟随全局"），所以再回一份**项目文件自己**的解析结果。
    let project_config = toml::from_str::<toml::Value>(&yekitrim_raw)
        .ok()
        .and_then(|v| serde_json::to_value(v).ok())
        .unwrap_or(serde_json::Value::Null);
    ok(serde_json::json!({
        "config_file": yekitrim_core::path::display(&ws.config_file),
        "root_dir": yekitrim_core::path::display(&ws.root_dir),
        "yekitrim_raw": yekitrim_raw,
        "config": config,
        "project_config": project_config,
        "config_error": config_error,
    }))
}

pub(super) fn config_yekitrim_put_impl(ws: &Workspace, body: &serde_json::Value, is_admin: bool) -> ApiResult {
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
pub(super) async fn config_global_put(
    Extension(user): Extension<crate::auth::CurrentUser>,
    State(_state): State<Arc<AppState>>,
    Json(body): Json<serde_json::Value>,
) -> ApiResult {
    let owner = crate::auth::owner_of(&user);
    let path = yekitrim_core::owner::state_dir(owner).join("config.toml");
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
pub(super) async fn config_llm_put(
    Extension(user): Extension<crate::auth::CurrentUser>,
    State(_state): State<Arc<AppState>>,
    Json(body): Json<serde_json::Value>,
) -> ApiResult {
    let owner = crate::auth::owner_of(&user);
    let path = yekitrim_core::owner::state_dir(owner).join("llm.toml");
    let _guard = yekitrim_core::filelock::lock_path(&path).await;
    blocking(move || config_llm_put_locked(&path, &body)).await
}

/// 从 `llm.toml` 文本里读 `[llm] api_key`（解析不了 / 没这个键都当空）。
fn llm_api_key_in(text: &str) -> String {
    toml::from_str::<toml::Value>(text)
        .ok()
        .and_then(|v| {
            v.get("llm")
                .and_then(|l| l.get("api_key"))
                .and_then(|k| k.as_str())
                .map(str::to_string)
        })
        .unwrap_or_default()
}

/// patch 里 `llm.api_key` 是**空串** ⇒ 把这个键剔掉（= 不修改）。
///
/// ⚠️ 为什么必须有这一步：`GET /api/config` 不再回显明文 key，设置页那一栏提交上来就是空串 ——
/// 直接深合并等于**每保存一次表单就清一次 key**。要**删** key 请发 `null`
/// （`patch_config_file` 的删键语义，是唯一能表达"清掉"的写法）。
fn drop_blank_api_key(patch: &serde_json::Value) -> serde_json::Value {
    let mut out = patch.clone();
    let blank = out
        .get("llm")
        .and_then(|l| l.get("api_key"))
        .and_then(|k| k.as_str())
        .map(|s| s.trim().is_empty())
        .unwrap_or(false);
    if blank {
        if let Some(llm) = out.get_mut("llm").and_then(|l| l.as_object_mut()) {
            llm.remove("api_key");
        }
    }
    out
}

pub(super) fn config_llm_put_locked(path: &Path, body: &serde_json::Value) -> ApiResult {
    if let Some(patch) = body.get("patch") {
        let patch = drop_blank_api_key(patch);
        yekitrim_core::config::patch_config_file(path, &patch)
            .map_err(|e| err(StatusCode::BAD_REQUEST, format!("LLM 配置保存失败: {e:#}")))?;
        return ok(serde_json::json!({ "ok": true }));
    }
    let raw = body.get("raw").and_then(|v| v.as_str()).or_else(|| body.get("content").and_then(|v| v.as_str()));
    let Some(raw) = raw else { return Err(err(StatusCode::BAD_REQUEST, "缺少 raw 或 patch 字段")); };
    yekitrim_service::validate_toml(raw)
        .map_err(|e| err(StatusCode::BAD_REQUEST, format!("llm.toml 不是合法 TOML，未保存: {e}")))?;
    // ⚠️ 源文件模式守**同一条**「空 api_key = 不修改」：`GET /api/config` 回的 `llm_raw` 是**抹过值**的，
    // 用户只想改别的键、原样存一次，不能顺手把真 key 抹掉（那是静默毁配置）。
    // 用户要改 key：在源文件里写真值即可（下面的分支就不会触发）。
    let incoming_key = llm_api_key_in(raw);
    let stored_key = std::fs::read_to_string(path).map(|t| llm_api_key_in(&t)).unwrap_or_default();
    if let Some(parent) = path.parent() { std::fs::create_dir_all(parent).map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("创建目录失败: {e}")))?; }
    std::fs::write(path, raw).map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("保存失败: {e}")))?;
    if incoming_key.trim().is_empty() && !stored_key.trim().is_empty() {
        // 整份写完之后再把已存的 key 补回去：走 patch 通道（会自己建 `[llm]` 表 / 补键），
        // 比在文本上做"插一行"的手术稳得多（源文件可能把那一行整个删了）
        yekitrim_core::config::patch_config_file(path, &serde_json::json!({ "llm": { "api_key": stored_key } }))
            .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("LLM 配置保存失败: {e:#}")))?;
    }
    ok(serde_json::json!({ "ok": true }))
}

/// `GET /api/llm/models?base_url=…`（**仅 admin**）：填好 base_url + api_key 就能拿到模型清单。
///
/// 三条语义（口径见 `docs/plan/llm-isolation-2026-10-19.md`）：
/// 1. **先查内置表**：认识的 route 直接回 `source: catalog`，**一个网络请求都不发**
///    —— 这是"方便"的来源，也是抄 pi-ai 的那条（认识的 route 不联网）；
/// 2. 表里没有 **才** 探：用**已存的那把 key**（`llm.toml` 的 `[llm] api_key`）打
///    `GET {base}/models`，回 `source: probe`。失败回 **502** + 已分类的文案
///    （key 不对 / 这个地址没有 /models / 连不上 / 其它，实现只在 `yekitrim_llm::discovery`）；
/// 3. **一律不落盘**：请求带的 `base_url` 只是**草稿**、回的是候选 —— `llm.toml` 仍然只由
///    `PUT /config/llm` 决定。
///
/// 响应形状：
/// * `{"source":"catalog","models":[{id,name}]}` —— 表命中，**不探、不回 caps**（零网络）；
/// * `{"source":"probe","models":[{id,name,caps}]}` —— 真探；`caps` 只在这份回包**带了能力**时才非 null
///   （见 `yekitrim_llm::discovery::DiscoverCaps`，键名与 `/config/llm/catalog` 的 caps 同源）。
pub(super) async fn llm_models(
    Extension(user): Extension<crate::auth::CurrentUser>,
    Query(q): Query<BaseUrlQuery>,
) -> ApiResult {
    let BaseUrlQuery { base_url, api_key } = q;
    let asked = base_url.unwrap_or_default().trim().to_string();

    // ① 内置表命中：**零网络请求**（连 api_key 都不需要 —— 也不需要读盘）
    let known = yekitrim_llm::catalog::models_for(&asked);
    if !known.is_empty() {
        return ok(serde_json::json!({
            "source": "catalog",
            "models": known
                .into_iter()
                .map(|(id, name)| serde_json::json!({ "id": id, "name": name }))
                .collect::<Vec<_>>(),
        }));
    }

    // ② 表里没有 → 探。key 取**已存的那把**（请求里不带 key，草稿一律不落盘）、同步读盘下 blocking 池
    let owner = crate::auth::owner_of(&user);
    let llm = blocking(move || Ok(yekitrim_core::config::load_llm_config_for(owner))).await?;
    let base = probe_request(&asked, &llm)?;
    // 草稿 key 优先（刚填还没保存的那种）；空串等于没给。
    let key = api_key
        .map(|k| k.trim().to_string())
        .filter(|k| !k.is_empty())
        .unwrap_or_else(|| llm.api_key.clone());
    match yekitrim_llm::discovery::discover(&key, &base).await {
        // ⚠️ 200 + 空列表**不是失败**：连上了、地址也对，只是它没报模型 —— 别把它说成"不支持"
        Ok(d) if d.models.is_empty() => ok(serde_json::json!({
            "source": "probe",
            "models": [],
            "note": yekitrim_llm::discovery::empty_hint(&base),
            // 空清单也要说清"我试过哪些路径"（P3：LiteLLM / Ollama 那两条）
            "tried": d.tried,
        })),
        Ok(d) => ok(serde_json::json!({
            "source": "probe",
            // 试过哪些路径（含 `/model/info` / `/api/tags` / `/api/show`）；"试过"≠"成功"
            "tried": d.tried,
            // 能力是哪条路径级探测给的（空 = 只来自 /v1/models 的自描述）
            "caps_from": d.caps_from,
            "models": d.models
                .into_iter()
                .map(|m| {
                    // **能获取就获取**：回包里带了能力就一并回（界面据此画控件）；没带就 null，**不编**。
                    // 形状走与 /config/llm/catalog **同一个渲染器**：表里的底 + 探测结果**只补不抹**
                    // （reader 没说的字段一律保留表里的值）。
                    // ⚠️ 这只是"这一趟探到的"，**不落盘**（草稿不落盘那条口径没变）。
                    let caps = m
                        .caps
                        .as_ref()
                        .map(|p| yekitrim_llm::catalog::caps_json_merged(&base, &m.id, Some(p)));
                    serde_json::json!({ "id": m.id, "name": m.name, "caps": caps })
                })
                .collect::<Vec<_>>(),
        })),
        Err(e) => Err(err(StatusCode::BAD_GATEWAY, e)),
    }
}

/// 真探之前的两道检查：**key 没配就别发一个必然 401 的请求**；两个 base_url 都空就没得探。
///
/// 拆成纯函数是为了**能钉住**：本机的真实程序目录（`target/debug/users/<uid>/.yekitrim/llm.toml`）
/// 里通常已经躺着一把真 key，HTTP 级用例钉不了"没配 key"那条分支（会真发一个网络请求）。
fn probe_request(asked: &str, llm: &yekitrim_core::config::LlmConfig) -> Result<String, AppError> {
    if llm.api_key.trim().is_empty() {
        return Err(err(
            StatusCode::BAD_REQUEST,
            "还没配置 API key（llm.toml 的 [llm] api_key）—— 填好再取模型",
        ));
    }
    // 请求里给了就用它（草稿），没给才回落到已存的那条
    let base = if asked.trim().is_empty() { llm.base_url.trim() } else { asked.trim() };
    if base.is_empty() {
        return Err(err(StatusCode::BAD_REQUEST, "缺少 base_url：要探哪个地址的模型清单？"));
    }
    Ok(base.to_string())
}

/// `GET /api/config/llm/catalog?base_url=…&model=…`（**仅 admin**）：这支模型**支持哪些旋钮**。
///
/// 设置页「按能力渲染控件」的唯一数据源：**只查内置表**（不联网、不读盘、不落盘）。
/// 表里没有这对 (host, model) 时回**保守默认 + `hit: false`**，**不是 404** ——
/// 界面照它画通用参数并写明"未收录"即可（回 404 会让人以为接口坏了）。
pub(super) async fn llm_catalog(Query(q): Query<CatalogQuery>) -> ApiResult {
    let base_url = q.base_url.unwrap_or_default();
    let model = q.model.unwrap_or_default();
    ok(serde_json::json!({
        "source": "catalog",
        "caps": yekitrim_llm::catalog::caps_json(&base_url, &model),
    }))
}

/// `/api/llm/models` 的查询串（`base_url` 缺省 = 用 llm.toml 里已存的那条）。
#[derive(serde::Deserialize)]
pub(super) struct BaseUrlQuery {
    #[serde(default)]
    pub base_url: Option<String>,
    /// **草稿 key**（可选）：用户刚填进界面、还没保存时，探针要用它。
    ///
    /// 不传就退回**已存的那把**（`llm.toml`）。为什么不强求：探针在保存前就该能用；
    /// 但只传草稿会让「已配好、只是改地址」那一步 401。
    #[serde(default)]
    pub api_key: Option<String>,
}

/// `/api/config/llm/catalog` 的查询串。
#[derive(serde::Deserialize)]
pub(super) struct CatalogQuery {
    #[serde(default)]
    pub base_url: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
}

pub(super) async fn llm_test(
    Extension(user): Extension<crate::auth::CurrentUser>,
) -> ApiResult {
    // 只有"读配置"是同步 IO（HTTP 调用本身是 async，保持不动）
    let owner = crate::auth::owner_of(&user);
    let llm = blocking(move || Ok(yekitrim_core::config::load_llm_config_for(owner))).await?;
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

    /// **真机验证**（默认不跑：要真端点、真 key、真网络）：打的是 `GET /api/llm/models` 的**真 handler**
    /// （probe_request → `discovery::discover` → 读取器注册表 → caps 合并，一条都不少）。
    ///
    /// 为什么是 handler 级而不是另起一个进程：**本机 26320 上跑着用户的实例**，
    /// `target/debug/yekitrim.exe` 被它占着 ⇒ cargo 换不掉那个文件（`os error 5`）。
    /// 停服后要复现进程级那条路：`cargo build -p yekitrim-cli && yekitrim server --port 26321 --local-no-auth`
    /// 再 `curl "http://127.0.0.1:26321/api/llm/models"`。
    ///
    /// PowerShell 跑法（key 从 llm.toml 现取，**别写进代码/日志**）：
    /// ```text
    /// $k = (Select-String -Path target/debug/users/1/.yekitrim/llm.toml -Pattern 'api_key\s*=\s*"([^"]+)"').Matches[0].Groups[1].Value
    /// $env:VT_LLM_BASE = "http://192.168.0.107:7863/v1"; $env:VT_LLM_KEY = $k
    /// cargo test -p yekitrim-server --lib -- real_endpoint --ignored --nocapture
    /// ```
    #[tokio::test]
    #[ignore = "要真端点与真 key（VT_LLM_BASE / VT_LLM_KEY）"]
    async fn real_endpoint_models_carry_caps() {
        // 两种跑法：① 只给 YEKITRIM_DATA_DIR（=程序目录）⇒ 用 llm.toml 里那把 key；
        // ② 给 VT_LLM_KEY 草稿 key —— ⚠️ 这条还要求 llm.toml 里**已经有**一把 key
        //    （probe_request 先查它，草稿只覆盖"用哪把"）。
        let base = std::env::var("VT_LLM_BASE").ok();
        let key = std::env::var("VT_LLM_KEY").ok();
        let user = crate::auth::CurrentUser {
            id: 1,
            username: "admin".into(),
            role: "admin".into(),
            must_change_password: false,
            session_id: "real-endpoint-probe".into(),
        };
        let Json(v) = llm_models(
            Extension(user),
            Query(BaseUrlQuery { base_url: base, api_key: key }),
        )
        .await
        .expect("真端点应当回 200（502/400 会在这里炸）");

        let models = v["models"].as_array().expect("models 是数组");
        let with_caps = models.iter().filter(|m| !m["caps"].is_null()).count();
        let with_levels = models
            .iter()
            .filter(|m| m["caps"]["levels"].as_object().map(|o| !o.is_empty()).unwrap_or(false))
            .count();
        println!("SOURCE={} TOTAL={} WITH_CAPS={} WITH_LEVELS={}", v["source"], models.len(), with_caps, with_levels);
        // 目标那支 + **档位清单缺字段的**（证明"没这个字段"不会被猜成支持）
        let mut shown_missing = 0;
        for m in models {
            let no_levels = m["caps"]["levels"]
                .as_object()
                .map(|o| o.is_empty())
                .unwrap_or(true);
            if m["id"] == "cn:deepseek-v4.1-flash" {
                println!("TARGET={} CAPS={}", m["id"], m["caps"]);
            } else if no_levels && shown_missing < 2 {
                shown_missing += 1;
                println!("NO_LEVELS={} CAPS={}", m["id"], m["caps"]);
            }
        }
        assert_eq!(v["source"], "probe", "这条路径应当真探（不是查表）");
        assert!(models.len() >= 20, "真端点有 21 支");
        assert_eq!(with_caps, models.len(), "每支都带能力（这家自描述）");
        assert!(with_levels >= 18, "绝大多数带档位清单：{with_levels}");
    }




/// **程序配置按账号分开 + 明文 key 一个字都不出接口 + 空值不覆盖真 key**。
///
/// 三件事一起钉（它们是同一个契约的三面）：
/// ① `Owner` 落到配置面：在引入 `Owner` 之前 `config.toml` / `llm.toml` 是**全局一份**，
///    所有账号共用同一个 `api_key`；现在甲写自己的、乙读不到。
///    判据从"甲看得见自己的 key"换成"**甲看得见自己的 model**"—— 明文 key 已经不回接口了。
/// ② `GET /api/config` **不回明文**：整份响应的 JSON 里都不许出现那把 key（`llm_raw` 与
///    `llm_config.api_key` 两处都清掉），只回 `api_key_set` 布尔。
/// ③ 配套的**写侧**：`api_key` 空串 = 不修改 —— 否则界面"读回来是空的、保存一次"就把真 key 抹了。
///    这条对 `patch` 与 `raw` **两条路都要成立**（raw 那条就是"编辑源文件后原样保存"）。
///
/// 用 `/api/config/llm` 的 `raw` 整份覆盖写（那条路不依赖模板里的具体键名，
/// 免得测试与 `LLM_CONFIG_TOML` 的字段名绑死），再用 `/api/config` 读回来比对。
#[tokio::test]
async fn config_is_per_account_and_llm_key_does_not_leak() {
    let t = test_app().await;
    let hash = yekitrim_service::auth::hash_password("pass-12345678").unwrap();
    t.auth.db().create_user("cfg_a", &hash, "user", false, 0).unwrap();
    t.auth.db().create_user("cfg_b", &hash, "user", false, 0).unwrap();
    let a = t.auth.login("cfg_a", "pass-12345678", None, None).unwrap();
    let b = t.auth.login("cfg_b", "pass-12345678", None, None).unwrap();

    async fn put_llm(
        t: &TestApp,
        sid: &str,
        csrf: &str,
        raw: &str,
    ) -> axum::response::Response {
        t.call(
            t.req_as(sid, csrf, "PUT", "/config/llm")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::json!({ "raw": raw }).to_string()))
                .unwrap(),
        )
        .await
    }
    async fn get_cfg(t: &TestApp, sid: &str, csrf: &str) -> serde_json::Value {
        let res = t
            .call(t.req_as(sid, csrf, "GET", "/config").body(Body::empty()).unwrap())
            .await;
        body_json(res).await
    }

    // 甲写自己的 key。
    let res = put_llm(
        &t,
        &a.session_id,
        &a.csrf,
        "[llm]
api_key = \"key-of-alice\"
model = \"m-a\"
",
    )
    .await;
    assert_eq!(res.status(), 200, "甲应能存自己的 LLM 配置");

    // 甲看得见**自己的配置**（model 是归属的判据 —— 明文 key 已经不回接口了）。
    let ja = get_cfg(&t, &a.session_id, &a.csrf).await;
    assert_eq!(ja["llm_config"]["model"], "m-a", "甲应读到自己那份配置：{ja}");
    assert_eq!(ja["api_key_set"], serde_json::json!(true), "甲配了 key：{ja}");
    // ② 整份响应里**一个字**明文都不许有（llm_raw 与 llm_config 两处都要清）
    let dumped_a = ja.to_string();
    assert!(
        !dumped_a.contains("key-of-alice"),
        "明文 API key 不许出接口（llm_raw / llm_config 都得抹）：{dumped_a}"
    );
    assert_eq!(ja["llm_config"]["api_key"], "", "解析出来的那份也要清：{ja}");
    let raw_a = ja["llm_raw"].as_str().unwrap_or("");
    assert!(raw_a.contains("api_key = \"\""), "键与行位置要留着（源文件编辑器靠它）：{raw_a}");
    assert!(raw_a.contains("m-a"), "没被抹的键原样保留：{raw_a}");

    // 先把乙那份**清干净**再断言：测试的配置目录是 exe 同级的 users/<uid>/.yekitrim
    // （测试二进制在 target/debug/deps/ ⇒ 那边也有一份 users/），而 uid 在每次新建的临时账本里
    // **又从 1 数起** ⇒ 同一个 uid 的 llm.toml 会在**多次跑测试之间留着**。
    // 不清就断言"乙还没配 key"，第二次跑必然红（实测：left: true, right: false）。
    // 顺手也就把 null = 删键 这条路钉住了（它是唯一能表达"清掉 key"的写法）。
    let res = t
        .call(
            t.req_as(&b.session_id, &b.csrf, "PUT", "/config/llm")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({ "patch": { "llm": { "api_key": null } } }).to_string(),
                ))
                .unwrap(),
        )
        .await;
    assert_eq!(res.status(), 200, "null 应该能删掉 key");

    // 乙**看不见**甲的配置。
    let jb = get_cfg(&t, &b.session_id, &b.csrf).await;
    assert_eq!(jb["api_key_set"], serde_json::json!(false), "乙还没配 key：{jb}");
    assert_ne!(jb["llm_config"]["model"], "m-a", "乙不该读到甲的配置：{jb}");
    assert!(
        !jb.to_string().contains("key-of-alice"),
        "乙的响应里更不该有甲的 key：{jb}"
    );

    // 乙写自己的，两边互不覆盖。
    let res = put_llm(&t, &b.session_id, &b.csrf, "[llm]
api_key = \"key-of-bob\"
model = \"m-b\"
").await;
    assert_eq!(res.status(), 200);
    let ja2 = get_cfg(&t, &a.session_id, &a.csrf).await;
    assert_eq!(ja2["llm_config"]["model"], "m-a", "乙写自己的配置**不该**动到甲的：{ja2}");
    assert_eq!(ja2["api_key_set"], serde_json::json!(true));
    assert!(!ja2.to_string().contains("key-of-bob"), "也不该串到甲的响应里：{ja2}");
    let jb2 = get_cfg(&t, &b.session_id, &b.csrf).await;
    assert_eq!(jb2["llm_config"]["model"], "m-b", "{jb2}");
    assert_eq!(jb2["api_key_set"], serde_json::json!(true));

    // ③ **空 api_key = 不修改**，两条路都成立：
    //   patch 路 —— 界面"没动那一栏"（读回来是空的）保存一次，key 必须还在
    let res = t
        .call(
            t.req_as(&a.session_id, &a.csrf, "PUT", "/config/llm")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({ "patch": { "llm": { "api_key": "", "model": "m-a2" } } })
                        .to_string(),
                ))
                .unwrap(),
        )
        .await;
    assert_eq!(res.status(), 200);
    let ja3 = get_cfg(&t, &a.session_id, &a.csrf).await;
    assert_eq!(ja3["llm_config"]["model"], "m-a2", "别的键照常改：{ja3}");
    assert_eq!(ja3["api_key_set"], serde_json::json!(true), "空串不该把 key 清掉：{ja3}");

    //   raw 路 —— "编辑源文件"读到的是**抹过值**的原文，原样存回去同样不许动 key
    let masked = ja3["llm_raw"].as_str().unwrap_or("").to_string();
    assert!(!masked.contains("key-of-alice"), "拿到的原文本来就是抹过的：{masked}");
    let res = put_llm(&t, &a.session_id, &a.csrf, &masked).await;
    assert_eq!(res.status(), 200, "抹过的原文必须能原样存回去");
    let ja4 = get_cfg(&t, &a.session_id, &a.csrf).await;
    assert_eq!(ja4["api_key_set"], serde_json::json!(true), "原样保存不许把真 key 抹掉：{ja4}");
    assert_eq!(ja4["llm_config"]["model"], "m-a2");
}

/// 单人形态（`Owner::Local`）读的就是**现状那两个文件**，零迁移。
///
/// 直接对 `config_get_impl` 断言路径来源。
///
/// ⚠️ **2026-10-10 改口径**：单人（`Local`）的配置目录**不再是** `<data>/.yekitrim`，
/// 而是**管理员**的 `<data>/users/<uid>/.yekitrim` —— 与"admin 登录后看到的那份"
/// 必须是同一份。原来那条断言钉的是"Local = 顶层"，正是"同一个人两套配置"的根。
#[test]
fn local_owner_config_dir_is_the_admin_user_tree() {
    let got = yekitrim_core::owner::state_dir(yekitrim_core::owner::Owner::Local);
    let admin = yekitrim_core::owner::local_admin_uid();
    assert_eq!(
        got,
        yekitrim_core::path::data_dir()
            .join("users")
            .join(admin.to_string())
            .join(yekitrim_core::path::STATE_DIR_NAME),
        "单人形态的配置目录必须落在当前管理员名下（与 admin 登录后是同一份）"
    );
    assert_ne!(
        got,
        yekitrim_core::path::yekitrim_home(),
        "不许再解析到顶层 <数据目录>/.yekitrim"
    );
}

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
/// 回归钉子：`config_yekitrim_put_impl` 以前只校验"是合法 TOML"，于是普通用户能把
/// `input_file` 改成别人的录像（转写完把字幕读出来）或把 `output_dir` 指到任意目录。
#[test]
fn project_config_paths_are_jailed_for_non_admin() {
    let root = std::env::temp_dir().join(format!("yekitrim-cfg-paths-{}", std::process::id()));
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
    let dir = std::env::temp_dir().join(format!("yekitrim-llm-put-{}", std::process::id()));
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

/// `redact_api_key`：只抹**值**，键名 / 缩进 / 其它键一律不动；**注释掉的老 key 也抹**。
#[test]
fn redact_api_key_只抹值() {
    let raw = "# 顶注\n[llm]\n  api_key = \"sk-real\"   # 我的 key\nmodel = \"m\"\n# api_key = \"sk-old\"\napi_key_extra = \"留着\"\n";
    let out = redact_api_key(raw);
    assert!(!out.contains("sk-real"), "行尾注释里也可能是 key：\n{out}");
    assert!(!out.contains("sk-old"), "注释掉的老 key 同样是明文：\n{out}");
    assert!(out.contains("  api_key = \"\""), "键名与缩进原样保留：\n{out}");
    assert!(out.contains("# api_key = \"\""), "注释符也保留（只是把值抹了）：\n{out}");
    assert!(out.contains("model = \"m\""), "别的键一个都不许动：\n{out}");
    assert!(out.contains("api_key_extra = \"留着\""), "键名只是前缀相同的不能误伤：\n{out}");
    assert!(out.contains("# 顶注"), "注释整行保留：\n{out}");
    // 行数不变（源文件编辑器靠行号对齐）
    assert_eq!(out.lines().count(), raw.lines().count());
}

/// `drop_blank_api_key`：**空串 = 剔掉这个键（不修改）**；`null` 与非空值都不动语义。
#[test]
fn patch_空_api_key_等于不修改() {
    let blank = serde_json::json!({ "llm": { "api_key": "   ", "model": "m" } });
    let out = drop_blank_api_key(&blank);
    assert!(out["llm"].get("api_key").is_none(), "空串要剔掉：{out}");
    assert_eq!(out["llm"]["model"], "m", "别的键照旧：{out}");

    // 真值照常写；null 照常表达"删键"（这是唯一能表达"清掉 key"的写法）
    let real = serde_json::json!({ "llm": { "api_key": "sk-new" } });
    assert_eq!(drop_blank_api_key(&real)["llm"]["api_key"], "sk-new");
    let del = serde_json::json!({ "llm": { "api_key": null } });
    assert!(drop_blank_api_key(&del)["llm"]["api_key"].is_null(), "null 是删键，别动它");
}

/// `config_llm_put_locked` 的 **raw** 路：空 api_key = 不修改（与 patch 同口径）。
///
/// 回归钉子：`GET /api/config` 回的 `llm_raw` 是**抹过值**的 —— 少了这条，
/// 用户"打开源文件编辑器、改一个别的键、保存"就会把真 key 静默抹掉。
#[test]
fn raw_保存时空_api_key_不会清掉已存的_key() {
    let dir = std::env::temp_dir().join(format!("yekitrim-llm-raw-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let p = dir.join("llm.toml");
    std::fs::write(&p, "[llm]\napi_key = \"sk-real\"\nmodel = \"m1\"\n").unwrap();

    // ① 抹过值的原文原样存回：真 key 必须还在，别的改动照常落地
    let masked = "[llm]\napi_key = \"\"\nmodel = \"m2\"\n";
    let _ = config_llm_put_locked(&p, &serde_json::json!({ "raw": masked })).unwrap();
    let after = std::fs::read_to_string(&p).unwrap();
    assert!(after.contains("api_key = \"sk-real\""), "空 api_key 不该覆盖真 key：\n{after}");
    assert!(after.contains("model = \"m2\""), "别的键要真的写进去：\n{after}");

    // ② 用户真写了新 key：照常换掉（"改 key"这条路不能被保护逻辑挡住）
    let _ = config_llm_put_locked(
        &p,
        &serde_json::json!({ "raw": "[llm]\napi_key = \"sk-new\"\nmodel = \"m2\"\n" }),
    )
    .unwrap();
    let after = std::fs::read_to_string(&p).unwrap();
    assert!(after.contains("api_key = \"sk-new\"") && !after.contains("sk-real"), "\n{after}");

    // ③ 源文件里连那一行都没有：也要补回来（走 patch 通道补键，别让"删一行"等于丢 key）
    let _ = config_llm_put_locked(&p, &serde_json::json!({ "raw": "[llm]\nmodel = \"m3\"\n" })).unwrap();
    let after = std::fs::read_to_string(&p).unwrap();
    assert!(after.contains("api_key = \"sk-new\"") && after.contains("model = \"m3\""), "\n{after}");

    // ④ 原本就没 key：不会被补出一个空的 api_key 行
    let q = dir.join("empty.toml");
    std::fs::write(&q, "[llm]\nmodel = \"m\"\n").unwrap();
    let _ = config_llm_put_locked(&q, &serde_json::json!({ "raw": "[llm]\nmodel = \"m4\"\n" })).unwrap();
    let after = std::fs::read_to_string(&q).unwrap();
    assert!(!after.contains("api_key"), "没配过 key 就别凭空写一行：\n{after}");

    let _ = std::fs::remove_dir_all(&dir);
}

/// 真探之前的两道检查（纯函数：本机真程序目录里通常已有 key，HTTP 级钉不住这条分支）。
#[test]
fn 探之前先要_key_与_base_url() {
    let mut llm = yekitrim_core::config::LlmConfig::default();
    llm.base_url = "https://stored.example/v1".to_string();
    let e = probe_request("https://asked.example/v1", &llm).unwrap_err();
    assert_eq!(e.0, StatusCode::BAD_REQUEST, "没配 key 就别发一个必然 401 的请求");
    let msg = e.1 .0["error"].as_str().unwrap_or("").to_string();
    assert!(msg.contains("API key"), "{msg}");

    llm.api_key = "sk-x".to_string();
    assert_eq!(probe_request("https://asked.example/v1", &llm).unwrap(), "https://asked.example/v1");
    assert_eq!(probe_request("  ", &llm).unwrap(), "https://stored.example/v1", "缺省用已存的那条");
    llm.base_url.clear();
    assert_eq!(probe_request("", &llm).unwrap_err().0, StatusCode::BAD_REQUEST, "两个都空就没得探");
}

/// `GET /api/llm/models`：**认识的 route ⇒ 一个网络请求都不发**（回 `source: catalog`）。
///
/// "不联网"在判据上怎么钉：表命中那一段在**碰任何 IO 之前**就返回了（不读盘、不发 HTTP），
/// 所以 `source: catalog` 就是"没探过"的证据 —— 真探过的话 source 只会是 probe，
/// 或干脆 400/502。
///
/// ⚠️ 这条用例**只读**程序目录，不写任何东西：那边（`target/debug/users/1/.yekitrim/llm.toml`）
/// 是本机**真实**的 LLM 配置，测试写它等于毁开发者的配置。
#[tokio::test]
async fn llm_models_表命中就回_catalog() {
    let t = test_app().await;
    let res = t
        .call(
            t.req("GET", "/llm/models?base_url=https://api.deepseek.com/v1")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(res.status(), StatusCode::OK);
    let v = body_json(res).await;
    assert_eq!(v["source"], "catalog", "认识的 route 必须走内置表：{v}");
    let models = v["models"].as_array().expect("models 是数组");
    let one = models
        .iter()
        .find(|m| m["id"] == "deepseek-v4-pro")
        .unwrap_or_else(|| panic!("表里有这支：{v}"));
    assert_eq!(one["name"], "DeepSeek V4 Pro", "name 是给人看的：{one}");
    assert!(models.len() > 1, "整家 provider 的模型都要给：{v}");
    // 形状就是契约：`{source, models:[{id,name}]}`（name 可以是 null，但键要在）
    for m in models {
        assert!(m["id"].is_string(), "{m}");
        assert!(m.get("name").is_some(), "name 键必须在（没有也得是 null）：{m}");
    }
}

/// `GET /api/config/llm/catalog`：设置页「按能力渲染控件」的数据源（**只查表**）。
///
/// 认不出 (base_url, model) 时回**保守默认 + hit:false**，**不是 404** —— 404 会让人以为接口坏了。
#[tokio::test]
async fn llm_catalog_回档位与能力() {
    let t = test_app().await;
    let res = t
        .call(
            t.req(
                "GET",
                "/config/llm/catalog?base_url=https://api.deepseek.com/v1&model=deepseek-v4-pro",
            )
            .body(Body::empty())
            .unwrap(),
        )
        .await;
    assert_eq!(res.status(), StatusCode::OK);
    let v = body_json(res).await;
    assert_eq!(v["source"], "catalog", "{v}");
    let caps = &v["caps"];
    assert_eq!(caps["hit"], serde_json::json!(true), "{v}");
    assert_eq!(caps["temperature"], "supported", "{v}");
    assert_eq!(
        caps["levels"].as_object().map(|m| m.len()),
        Some(4),
        "档位下拉的选项就是它的键：{v}"
    );
    assert_eq!(caps["max_output_ceiling"].as_u64(), Some(384_000), "{v}");

    let res = t
        .call(
            t.req("GET", "/config/llm/catalog?base_url=https://unknown.example.com/v1&model=nope")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(res.status(), StatusCode::OK, "认不出不是错误（别回 404）");
    let v = body_json(res).await;
    assert_eq!(v["caps"]["hit"], serde_json::json!(false), "{v}");
    assert_eq!(v["caps"]["levels"].as_object().map(|m| m.len()), Some(0), "{v}");
}

/// 两条 LLM 模型接口都**仅 admin**（它们会用到已存的那把 key / 服务于设置页）。
#[tokio::test]
async fn llm_model_endpoints_are_admin_only() {
    let t = test_app().await;
    let hash = yekitrim_service::auth::hash_password("pass-12345678").unwrap();
    t.auth.db().create_user("llm_plain", &hash, "user", false, 0).unwrap();
    let b = t.auth.login("llm_plain", "pass-12345678", None, None).unwrap();
    for uri in [
        "/llm/models?base_url=https://api.deepseek.com/v1",
        "/config/llm/catalog?base_url=https://api.deepseek.com/v1&model=deepseek-v4-pro",
    ] {
        let res = t
            .call(t.req_as(&b.session_id, &b.csrf, "GET", uri).body(Body::empty()).unwrap())
            .await;
        assert_eq!(res.status(), StatusCode::FORBIDDEN, "{uri} 应仅 admin");
        // ⚠️ 匿名请求必须自己建（`t.req` 自带 admin 的 cookie，那样测的是"已登录"）
        let res = t
            .call(
                axum::http::Request::builder()
                    .method("GET")
                    .uri(uri)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await;
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED, "{uri} 未登录必须 401");
    }
}
}
