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
// 明文脱敏**只此一份实现**在 core（同一条也用在"解析失败的 WARN"上）
use yekitrim_core::config::redact_secrets;
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
    // **只此一份实现**在 core —— 别在这儿抄第二份（它也是"解析失败的 WARN 要脱敏"那条用的）。
    redact_secrets(raw)
}



/// 界面「按链路覆盖」要显示的**链路键名**（前端照它出行，**别自己写死四个**）。
///
/// 来源是 [`yekitrim_core::config::LlmLink::ui_visible`]：`detect/polish/title/utility`。
/// ⚠️ `correct` **不在这里**（用户口径："字幕订正删除"）—— 但代码与 `[llm.overrides.correct]`
/// 照旧生效（`asr/correct.rs` 还在用），只是不给界面一行。
fn override_link_keys() -> Vec<&'static str> {
    yekitrim_core::config::LlmLink::ui_visible().iter().map(|l| l.key()).collect()
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
    // 「有没有配 key」以后只由这个布尔回答（界面不再回显 key 本身）。
    // ⚠️ **v4 没有顶层 api_key 了**：判定必须走 `has_api_key()`（= 当前生效那份接口的 key）。
    // 读顶层字段会恒 false —— 那就是 2026-10-20 那族"明明配了却说未配置"的 bug。
    let api_key_set = llm_cfg.has_api_key();
    // ⚠️ **每一条接口都有一份自己的 key** —— 逐条判、逐条抹
    //    （多条时最容易漏的就是这里：只抹生效那把 = 其余 provider 的明文照出接口）。
    //    「有没有」的口径与该 provider 的构造口径同源（`key()`：显式 key 优先、其次 api_key_env）。
    let provider_key_sets: Vec<bool> = llm_cfg.providers.iter().map(|p| !p.key().is_empty()).collect();
    for p in llm_cfg.providers.iter_mut() {
        p.api_key.clear();
    }
    // v4 的形状由 serde 一份给出（`active` / `max_concurrent` / `defaults` / `extra_params` /
    // `providers` / `models` / `links`）—— **别再手工拼 v3 的字段名**：顶层
    // `api_key` / `base_url` / `model` / `params` / `overrides` / `models` 全都没有了。
    let mut llm_json = serde_json::to_value(&llm_cfg).unwrap_or_default();
    // 界面「按链路覆盖」的行 = 后端给的清单（Correct 只隐藏不删 —— 别让前端自己写死）
    if let Some(o) = llm_json.as_object_mut() {
        o.insert("override_links".into(), serde_json::json!(override_link_keys()));
        // 空容器也回出来：三个集合在 serde 里都是 `skip_serializing_if`（空就不出现），
        // 而前端是**照着它们画**（接口列表 / 模型清单 / 链路表）—— 缺键会变成 undefined。
        o.entry("providers").or_insert_with(|| serde_json::json!([]));
        o.entry("models").or_insert_with(|| serde_json::json!([]));
        o.entry("links").or_insert_with(|| serde_json::json!({}));
    }
    if let Some(arr) = llm_json.get_mut("providers").and_then(|v| v.as_array_mut()) {
        for (i, item) in arr.iter_mut().enumerate() {
            if let Some(o) = item.as_object_mut() {
                // 空串在 serde 里被 `skip_serializing_if` 丢掉了 ⇒ **显式补回空串**：
                // 前端按"键在、值为空"画表单，与 `llm_raw` 的抹值形状保持一致。
                o.insert("api_key".into(), serde_json::json!(""));
                o.insert(
                    "api_key_set".into(),
                    serde_json::json!(provider_key_sets.get(i).copied().unwrap_or(false)),
                );
            }
        }
    }
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
        "llm_config": llm_json,
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
        // 与 config.toml 同一条纪律（review B3）：改**临时副本** → 真反序列化验 → rename。
        // 项目文件在用户手上、还可能被别的工具手改，直接改真文件的代价是"写一半崩掉 = 整份工程配置失效"。
        let tmp = staged_copy(&ws.config_file)?;
        if let Err(e) = yekitrim_core::config::patch_config_file(&tmp, patch) {
            let _ = std::fs::remove_file(&tmp);
            return Err(err(StatusCode::BAD_REQUEST, format!("项目配置保存失败: {e:#}")));
        }
        return commit_validated_config(&tmp, &ws.config_file, "yekitrim.toml");
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
    let tmp = ws.config_file.with_extension("toml.guard");
    if let Some(parent) = ws.config_file.parent() { std::fs::create_dir_all(parent).map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("创建目录失败: {e}")))?; }
    std::fs::write(&tmp, raw).map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("保存失败: {e}")))?;
    commit_validated_config(&tmp, &ws.config_file, "yekitrim.toml")
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
    // ⚠️ **两条分支都必须"临时文件 → 真反序列化验 → rename"**（2026-11-14 后端 review B3）。
    // 以前 patch 直接改真文件、raw 直接 `fs::write`：语义坏值会让整份配置被忽略，
    // 而写一半崩掉（非原子）同样让文件截断。守卫与 llm.toml 那条同源。
    if let Some(patch) = body.get("patch") {
        let tmp = staged_copy(path)?;
        if let Err(e) = yekitrim_core::config::patch_config_file(&tmp, patch) {
            let _ = std::fs::remove_file(&tmp);
            return Err(err(StatusCode::BAD_REQUEST, format!("配置保存失败: {e:#}")));
        }
        return commit_validated_config(&tmp, path, "config.toml");
    }
    let raw = body.get("raw").and_then(|v| v.as_str()).or_else(|| body.get("content").and_then(|v| v.as_str()));
    let Some(raw) = raw else { return Err(err(StatusCode::BAD_REQUEST, "缺少 raw 或 patch 字段")); };
    // 语法错误会更早、更明确地报出来（"第几行不合法"比反序列化的"缺字段"好读），
    // 语义错误交给下面那道真反序列化
    yekitrim_service::validate_toml(raw)
        .map_err(|e| err(StatusCode::BAD_REQUEST, format!("config.toml 不是合法 TOML，未保存: {e}")))?;
    let tmp = path.with_extension("toml.guard");
    if let Some(parent) = path.parent() { std::fs::create_dir_all(parent).map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("创建目录失败: {e}")))?; }
    std::fs::write(&tmp, raw).map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("保存失败: {e}")))?;
    commit_validated_config(&tmp, path, "config.toml")
}

/// 保存 LLM 配置：`raw` 整份覆盖写，`patch` 合并写（保留注释与其他键）。
/// `GET /api/config/llm/effective?link=<detect|polish|correct|title|utility>`（L2）：该链路**合并后的生效值**。
///
/// 薄壳：读该账号自己的 `llm.toml`，再转 `LlmV4::link_snapshot(link)` —— 前端不再自己算规则
/// （L1 之前「界面显示的与真发的不一致」就是这么来的）。**明文 key 不出现**，只报 `api_key_set`。
pub(super) async fn config_llm_effective(
    Extension(user): Extension<crate::auth::CurrentUser>,
    Query(q): Query<LinkQuery>,
) -> ApiResult {
    let wanted = q.link.unwrap_or_else(|| "detect".to_string());
    let link = yekitrim_core::config::LlmLink::ALL
        .iter()
        .copied()
        .find(|l| l.key() == wanted.trim())
        .ok_or_else(|| {
            err(
                StatusCode::BAD_REQUEST,
                format!(
                    "link 认不出：{}（可选：{}）",
                    wanted,
                    yekitrim_core::config::LlmLink::ALL
                        .iter()
                        .map(|l| l.key())
                        .collect::<Vec<_>>()
                        .join(" / ")
                ),
            )
        })?;
    let owner = crate::auth::owner_of(&user);
    let llm = blocking(move || Ok(yekitrim_core::config::load_llm_config_for(owner))).await?;
    ok(llm.link_snapshot(link))
}

/// `?link=` 的查询串（缺省 = detect）。
#[derive(serde::Deserialize)]
pub(super) struct LinkQuery {
    #[serde(default)]
    pub link: Option<String>,
}

pub(super) async fn config_llm_put(
    Extension(user): Extension<crate::auth::CurrentUser>,
    State(_state): State<Arc<AppState>>,
    Json(body): Json<serde_json::Value>,
) -> ApiResult {
    let owner = crate::auth::owner_of(&user);
    let is_admin = user.is_admin();
    let path = yekitrim_core::owner::state_dir(owner).join("llm.toml");
    let _guard = yekitrim_core::filelock::lock_path(&path).await;
    blocking(move || config_llm_put_locked(&path, &body, is_admin)).await
}

// ---------------------------------------------------------------------------
// B8（2026-11-14 后端 review）：普通账号的 LLM `base_url` 禁止指向本机 / 内网。
//
// `PUT /api/config/llm`（用户层）+ `POST /api/llm/test` + `GET /api/llm/models`
// 三处入口**共用下面这一份纯函数判据** —— 保存时拦得住、测试/取模型也绕不过去，
// 否则"写不进但测得动"就是一条后门。
//
// 口径：**判"账号角色"，不判部署形态**。admin 不受限（公网部署的运维要能配内网中转）；
// 单人形态只有一个 admin 账号，所以本地 Ollama（`http://127.0.0.1:11434`）照常可用。
// 普通账号（多用户形态的 `role=user`）只许指向公网地址 —— 否则他能让服务器盲探
// loopback / 私网 / 云元数据（169.254.169.254）端口。
//
// 已知限制（如实记录）：**只认字面量**。DNS 主机名不做解析 —— `internal.example.com`
// 指向 10.x 时这里看不见；要堵那一层得在每次请求前做解析校验，代价与收益不成比例。
// ---------------------------------------------------------------------------

/// B8 判据：`base_url` 的主机是不是 loopback / 私网 / 链路本地 / 未指定地址。
///
/// 主机名 `localhost`（含 `*.localhost`，RFC 6761 恒指 loopback）一并算内网；
/// 其余主机名不做解析（见上面的"已知限制"）。IPv4-mapped IPv6（`::ffff:10.0.0.5`）
/// 按映射后的 v4 判 —— 那是绕过 v4 检查的常见写法。
fn base_url_host_is_local(base_url: &str) -> bool {
    let raw = base_url.trim();
    if raw.is_empty() {
        return false;
    }
    // scheme://[user@]host[:port][/path…] —— 没有 scheme 时把整段当 host 部分（老实处理）
    let rest = match raw.split_once("://") {
        Some((_, r)) => r,
        None => raw,
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or(rest);
    // 去 userinfo（`user@host`：rsplit —— userinfo 里也可能有 @）
    let hostport = match authority.rsplit_once('@') {
        Some((_, h)) => h,
        None => authority,
    };
    if hostport.is_empty() {
        return false;
    }
    // IPv6 字面量必带方括号（`[::1]:11434`）
    if let Some(inner) = hostport.strip_prefix('[').and_then(|s| s.split_once(']')) {
        return ip_is_local_str(inner.0);
    }
    // 剥端口：裸 IPv6（多个 ':'、无方括号）不能按"最后一个冒号"剥，整段拿去解析
    let host = if hostport.matches(':').count() > 1 {
        hostport
    } else if let Some((h, _)) = hostport.rsplit_once(':') {
        h
    } else {
        hostport
    };
    let lower = host.trim().to_ascii_lowercase();
    if lower.is_empty() {
        return false;
    }
    if lower == "localhost" || lower.ends_with(".localhost") {
        return true;
    }
    ip_is_local_str(&lower)
}

fn ip_is_local_str(s: &str) -> bool {
    match s.parse::<std::net::IpAddr>() {
        Ok(std::net::IpAddr::V4(v4)) => {
            v4.is_loopback() || v4.is_private() || v4.is_link_local() || v4.is_unspecified()
        }
        Ok(std::net::IpAddr::V6(v6)) => {
            if let Some(mapped) = v6.to_ipv4_mapped() {
                return mapped.is_loopback()
                    || mapped.is_private()
                    || mapped.is_link_local()
                    || mapped.is_unspecified();
            }
            v6.is_loopback()
                || v6.is_unspecified()
                // unique local fc00::/7 与 link-local fe80::/10（std 没有现成判据）
                || (v6.segments()[0] & 0xfe00) == 0xfc00
                || (v6.segments()[0] & 0xffc0) == 0xfe80
        }
        Err(_) => false,
    }
}

/// B8 入口判据：普通账号的 `base_url` 指向本机/内网 ⇒ **403**（admin 不受限）。
pub(super) fn assert_base_url_allowed(base_url: &str, is_admin: bool) -> Result<(), AppError> {
    if is_admin {
        return Ok(());
    }
    let raw = base_url.trim();
    // 空值放行：必填校验是各入口自己的事（清空 base_url = 用回默认，是合法操作）
    if raw.is_empty() {
        return Ok(());
    }
    // 解析不出主机：对非 admin 也拦（没法判定 = 不能放行），文案按 400 说清
    if raw.split_once("://").map(|(scheme, _)| scheme.is_empty()).unwrap_or(false) {
        return Err(err(StatusCode::BAD_REQUEST, format!("无法识别的 API 地址：{raw}")));
    }
    if base_url_host_is_local(raw) {
        return Err(err(
            StatusCode::FORBIDDEN,
            "API 地址不允许指向本机或内网地址（loopback / 私网 / 链路本地）；内网中转请让管理员配置",
        ));
    }
    Ok(())
}

/// 对**一份将成为 llm.toml 的文本**里的所有 base_url 过 B8 判据。
///
/// 在「写盘前的最终文本」上判（patch 合并完、raw 还原 provider key 之后）——
/// 只判 patch 里显式出现的那几个值会漏掉"某一份 provider 另带一个内网地址"的绕法。
///
/// ⚠️ **v4 的连接面只剩 `[[llm.providers]]`**：没有顶层 `base_url` 了（旧文件里残留的那个键
/// 在 v4 里根本没人读，判它等于给用户一个假的"拦住了"）。所以只扫 providers。
fn assert_llm_file_bases(text: &str, is_admin: bool) -> Result<(), AppError> {
    #[derive(serde::Deserialize)]
    struct LlmFile {
        #[serde(default)]
        llm: yekitrim_core::config::LlmConfigV4,
    }
    // 解析不过 = 语法/形状错误，由前序校验（validate_toml / check_provider_*）负责报
    let Ok(f) = toml::from_str::<LlmFile>(text) else {
        return Ok(());
    };
    for p in &f.llm.providers {
        assert_base_url_allowed(&p.base_url, is_admin)?;
    }
    Ok(())
}

/// patch 里 `api_key` 是**空串** ⇒ 把这个键剔掉（= 不修改）。
///
/// ⚠️ 为什么必须有这一步：`GET /api/config` 不再回显明文 key，设置页那一栏提交上来就是空串 ——
/// 直接深合并等于**每保存一次表单就清一次 key**。要**删** key 请发 `null`
/// （`patch_config_file` 的删键语义，是唯一能表达"清掉"的写法）。
///
/// ⚠️ **v4 的 key 住在一份份 provider 里** ⇒ 规则的主战场是 `providers[].api_key`（逐份判）。
/// 顶层那把只剩「老客户端 + [`mirror_legacy_into_active_provider`]」这条兼容路在用：
/// 老界面发的 `llm.api_key = ""` 一旦被镜像进 active provider，就等于把真 key 抹了 —— 仍然要剔。
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
    // **每一条 provider** 各判一次：空串 = 那条不修改（与顶层同一条口径）
    if let Some(arr) = out
        .get_mut("llm")
        .and_then(|l| l.as_object_mut())
        .and_then(|l| l.get_mut("providers"))
        .and_then(|p| p.as_array_mut())
    {
        for p in arr.iter_mut() {
            let Some(o) = p.as_object_mut() else { continue };
            let blank = o
                .get("api_key")
                .and_then(|k| k.as_str())
                .map(|s| s.trim().is_empty())
                .unwrap_or(false);
            if blank {
                o.remove("api_key");
            }
        }
    }
    out
}

/// 数组是**整体替换**的（`patch_config_file` 的深合并只对对象递归）——
/// 所以「空 key = 不修改」不能靠"没写这个键"来表达：provider 的 `api_key` 缺了，
/// 整份数组一换，那条的真 key 就**跟着丢了**。这里按 `id` 把已存的 key 填回去。
///
/// ⚠️ `api_key: null` = **明确删键**（不填）；只是"没带这个键"才填。

/// **老形状兼容（写侧）**：v3 之后 `model` / `base_url` / `api_key` 的生效值在 active provider 里。
///
/// 老界面仍然发 `llm.model` 这类键（P3 之前的所有客户端）—— 只写顶层会**静默不生效**
/// （读侧又回显 provider 的值，用户看到"改了又变回去"）。所以这里把这三把键**同时镜像进
/// 那一份 active provider**，其余 provider 原样带着（数组是整体替换，不能只发一份）。
///
/// 不镜像的情况：这次 patch 自己带了 `providers`（那是新客户端，它说了算）、文件里没有 active
/// provider（老文件，顶层就是生效值）、值是 `null`（"删键"只对顶层有意义）。
fn mirror_legacy_into_active_provider(patch: &serde_json::Value, stored_text: &str) -> serde_json::Value {
    let mut out = patch.clone();
    let Some(llm) = out.get_mut("llm").and_then(|l| l.as_object_mut()) else {
        return out;
    };
    if llm.contains_key("providers") {
        return out;
    }
    let Ok(v) = toml::from_str::<toml::Value>(stored_text) else { return out };
    let Some(stored) = v.get("llm") else { return out };
    let Some(arr) = stored.get("providers").and_then(|p| p.as_array()) else { return out };
    let active = stored.get("active").and_then(|a| a.as_str()).unwrap_or("").trim();
    if active.is_empty() {
        return out;
    }
    // providers 转成可写的 JSON（保持顺序），找到 active 那一份
    let mut providers: Vec<serde_json::Value> = arr
        .iter()
        .filter_map(|p| serde_json::to_value(p).ok())
        .collect();
    let Some(target) = providers
        .iter_mut()
        .find(|p| p.get("id").and_then(|i| i.as_str()).map(|i| i.trim()) == Some(active))
        .and_then(|p| p.as_object_mut())
    else {
        return out;
    };
    for key in ["model", "base_url", "api_key"] {
        match llm.get(key) {
            Some(v) if v.is_null() => {}          // 删键只有顶层语义
            Some(v) => {
                target.insert(key.to_string(), v.clone());
            }
            None => {}
        }
    }
    llm.insert("providers".into(), serde_json::json!(providers));
    out
}

fn fill_blank_provider_keys(patch: &serde_json::Value, stored_text: &str) -> serde_json::Value {
    let mut out = patch.clone();
    // 已存文件里 id → key
    let mut keys: std::collections::BTreeMap<String, String> = std::collections::BTreeMap::new();
    if let Ok(v) = toml::from_str::<toml::Value>(stored_text) {
        if let Some(arr) = v.get("llm").and_then(|l| l.get("providers")).and_then(|p| p.as_array()) {
            for p in arr {
                let id = p.get("id").and_then(|i| i.as_str()).unwrap_or("").trim();
                let key = p.get("api_key").and_then(|k| k.as_str()).unwrap_or("");
                if !id.is_empty() && !key.trim().is_empty() {
                    keys.insert(id.to_string(), key.to_string());
                }
            }
        }
    }
    if keys.is_empty() {
        return out;
    }
    if let Some(arr) = out
        .get_mut("llm")
        .and_then(|l| l.as_object_mut())
        .and_then(|l| l.get_mut("providers"))
        .and_then(|p| p.as_array_mut())
    {
        for p in arr.iter_mut() {
            let Some(o) = p.as_object_mut() else { continue };
            match o.get("api_key") {
                Some(v) if v.is_null() => continue, // 明确删键
                Some(_) => continue,                // 有值（空串已在 drop_blank_api_key 里剔掉）
                None => {}
            }
            let id = o.get("id").and_then(|i| i.as_str()).unwrap_or("").trim().to_string();
            if let Some(k) = keys.get(&id) {
                o.insert("api_key".into(), serde_json::json!(k));
            }
        }
    }
    out
}

/// 写通道要拦的「半写坏」：接口没 id / id 重复 / 协议认不出 / `active` 指错 /
/// 模型清单里 (接口, id) 重复或 provider 认不出。
///
/// ⚠️ **「一个接口都没有」在写通道不算错**：全新安装就是这个状态，用户也可以显式把
/// `providers` 清空（= 回到"什么都没配"）。那条判据是给「整份配置能不能跑」用的
/// （[`LlmConfigV4::problems`]），写通道照搬会把"删掉最后一份接口"变成 400。
fn write_channel_problems(cfg: &yekitrim_core::config::LlmConfigV4) -> Vec<String> {
    if cfg.providers.is_empty() {
        return Vec::new();
    }
    cfg.problems()
}

/// 写配置前拦下那几种**静默失效**（协议认不出 / id 重复 / active 指向不存在的那份 / 清单重复）。
///
/// ⚠️ 只在 patch **真的碰了 `providers`** 时校验 —— 只改 `defaults` / `links` 的 patch 零回归。
fn check_provider_patch(patch: &serde_json::Value) -> Result<(), AppError> {
    let Some(llm) = patch.get("llm") else { return Ok(()) };
    let Some(arr) = llm.get("providers") else { return Ok(()) };
    let providers: Vec<yekitrim_core::config::llm_v4::LlmProviderV4> = serde_json::from_value(arr.clone())
        .map_err(|e| err(StatusCode::BAD_REQUEST, format!("providers 形状不对：{e}")))?;
    let mut probe = yekitrim_core::config::LlmConfigV4::default();
    probe.providers = providers;
    // v4 的 `active` 是普通字符串（空 = 第一份）
    probe.active = llm.get("active").and_then(|a| a.as_str()).unwrap_or_default().to_string();
    let problems = write_channel_problems(&probe);
    if problems.is_empty() {
        Ok(())
    } else {
        Err(err(StatusCode::BAD_REQUEST, problems.join("；")))
    }
}

/// 同上，但校验**整份原文**（源文件模式）。
fn check_provider_raw(raw: &str) -> Result<(), AppError> {
    #[derive(serde::Deserialize)]
    struct File {
        #[serde(default)]
        llm: yekitrim_core::config::LlmConfigV4,
    }
    let Ok(f) = toml::from_str::<File>(raw) else {
        return Ok(()); // 语法错误由上面那条 validate_toml 报，这里只管语义
    };
    let problems = write_channel_problems(&f.llm);
    if problems.is_empty() {
        Ok(())
    } else {
        Err(err(StatusCode::BAD_REQUEST, problems.join("；")))
    }
}

/// 写 `llm.toml` 的**唯一通道**（patch / raw 都走这里）。
///
/// # ⚠️ 两条硬规矩（P0 事故 2026-10-19）
///
/// 1. **先写临时文件 → 用真正的反序列化器验一遍 → 再 `rename`**：
///    那次事故是保存路径把 `json_mode = ""`（字符串）写进了布尔字段 —— 文件**是合法 TOML**，
///    只有反序列化到 `LlmConfig` 才炸，而炸的后果是**整份 LLM 配置回退默认值**（界面"更糟了"）。
///    所以守卫必须在"合法 TOML"之上再加一层**真反序列化**，且**失败时一个字节都不许落到原文件上**。
/// 2. 失败回 **400 + 人话**（哪个字段 / 期望什么 / 实际什么），而那条人话里的 key 已脱敏
///    （见 `yekitrim_core::config::redact_secrets`）。
pub(super) fn config_llm_put_locked(path: &Path, body: &serde_json::Value, is_admin: bool) -> ApiResult {
    if let Some(patch) = body.get("patch") {
        let patch = drop_blank_api_key(patch);
        // **数组是整体替换** ⇒ 缺了 api_key 的那条要按 id 把已存的 key 填回去（否则静默丢 key）
        let stored = std::fs::read_to_string(path).unwrap_or_default();
        let patch = fill_blank_provider_keys(&patch, &stored);
        // 老客户端发 llm.model/base_url/api_key ⇒ 镜像进 active provider（否则"改了没生效"）
        let patch = mirror_legacy_into_active_provider(&patch, &stored);
        check_provider_patch(&patch)?;
        // 在**临时副本**上做 patch（原文件全程不动），验过再换上去
        let tmp = staged_copy(path)?;
        let written = yekitrim_core::config::patch_config_file(&tmp, &patch);
        if let Err(e) = written {
            let _ = std::fs::remove_file(&tmp);
            return Err(err(StatusCode::BAD_REQUEST, format!("LLM 配置保存失败: {e:#}")));
        }
        // B8：对**合并后的最终文本**过一遍内网地址判据（顶层 + 每条 provider 都在文本里，不会漏）
        if let Err(e) = assert_llm_file_bases(&std::fs::read_to_string(&tmp).unwrap_or_default(), is_admin) {
            let _ = std::fs::remove_file(&tmp);
            return Err(e);
        }
        return commit_validated(&tmp, path);
    }
    let raw = body.get("raw").and_then(|v| v.as_str()).or_else(|| body.get("content").and_then(|v| v.as_str()));
    let Some(raw) = raw else { return Err(err(StatusCode::BAD_REQUEST, "缺少 raw 或 patch 字段")); };
    yekitrim_service::validate_toml(raw)
        .map_err(|e| err(StatusCode::BAD_REQUEST, format!("llm.toml 不是合法 TOML，未保存: {e}")))?;
    // ⚠️ 源文件模式守**同一条**「空 api_key = 不修改」：`GET /api/config` 回的 `llm_raw` 是**抹过值**的，
    // 用户只想改别的键、原样存一次，不能顺手把真 key 抹掉（那是静默毁配置）。
    // 用户要改 key：在源文件里写真值即可（`restore_llm_provider_keys` 只填空的那些）。
    //
    // ⚠️ **v4 的 key 只在 `[[llm.providers]]` 里**（没有顶层 `[llm] api_key` 了）⇒ 这条规则
    //    只能按 provider 的 `id` **逐份**做；"补回顶层 api_key"那条老分支在 v4 里无处可写，已删。
    let stored_text = std::fs::read_to_string(path).unwrap_or_default();
    let restored = yekitrim_core::config::restore_llm_provider_keys(raw, &stored_text);
    let raw = restored.as_deref().unwrap_or(raw);
    check_provider_raw(raw)?;
    // B8：源文件模式同样不许普通账号把 base_url 指到本机/内网（判据只有这一份）
    assert_llm_file_bases(raw, is_admin)?;
    // 仍走"临时文件 → 验 → rename"：raw 这条路以前是直接覆盖原文件的
    let tmp = path.with_extension("toml.guard");
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("创建目录失败: {e}")))?;
    }
    std::fs::write(&tmp, raw).map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("保存失败: {e}")))?;
    commit_validated(&tmp, path)
}

/// 把现有文件拷进同目录的临时路径（不存在就建空的）—— 后续所有改动都在它上面做。
fn staged_copy(path: &Path) -> Result<std::path::PathBuf, AppError> {
    let tmp = path.with_extension("toml.guard");
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("创建目录失败: {e}")))?;
    }
    if path.exists() {
        std::fs::copy(path, &tmp)
            .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("准备临时文件失败: {e}")))?;
    } else {
        std::fs::write(&tmp, "")
            .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("准备临时文件失败: {e}")))?;
    }
    Ok(tmp)
}

/// **写盘前守卫**：解析临时文件 → 失败则删掉临时文件、**原文件一个字节都没动**，回 400；
/// 成功则 `rename` 换上去（同卷 rename 是原子的：要么旧文件完好、要么新内容已验证）。
fn commit_validated(tmp: &Path, path: &Path) -> ApiResult {
    let text = std::fs::read_to_string(tmp)
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("读临时文件失败: {e}")))?;
    if let Err(bad) = yekitrim_core::config::validate_llm_toml(&text) {
        let _ = std::fs::remove_file(tmp);
        return Err(err(
            StatusCode::BAD_REQUEST,
            format!("这份 llm.toml 解析不过，**没有保存**（原文件未改动）：{bad}"),
        ));
    }
    std::fs::rename(tmp, path).map_err(|e| {
        let _ = std::fs::remove_file(tmp);
        err(StatusCode::INTERNAL_SERVER_ERROR, format!("保存失败: {e}"))
    })?;
    ok(serde_json::json!({ "ok": true }))
}

/// **程序配置（`config.toml` / 项目 `yekitrim.toml`）的写盘前守卫**：与 llm 那份同款，
/// 但校验器是**真的反序列化到 `Config`**（2026-11-14 后端 review B3）。
///
/// 为什么必须是"真反序列化"而不是 `validate_toml`（只验语法）：
/// `load_config_for` 对解析失败是 **fail-soft**（`if let Ok(Some(overlay))` 才 merge）——
/// 一个类型写错的值（`num_workers = "4"`）语法完全合法，却会让**整份文件**被忽略、所有键
/// 一起回退默认值（实测：同文件里 `chunk_duration = 42` 也读回 60）。
///
/// 三条硬规矩（与 `commit_validated` 一致）：① 只改临时文件；② 验不过 ⇒ 删临时文件、
/// **原文件一个字节都不动** + 400；③ 验过 ⇒ 同卷 `rename`（原子替换）。
fn commit_validated_config(tmp: &Path, path: &Path, what: &str) -> ApiResult {
    let text = std::fs::read_to_string(tmp)
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("读临时文件失败: {e}")))?;
    // 错误文案先脱敏：TOML 的报错会带出错那一行的原文，而配置文件里可能有 api_key
    if let Err(bad) = toml::from_str::<yekitrim_core::config::Config>(&text) {
        let _ = std::fs::remove_file(tmp);
        return Err(err(
            StatusCode::BAD_REQUEST,
            format!(
                "这份 {what} 解析不过，**没有保存**（原文件未改动）：{}",
                yekitrim_core::config::redact_secrets(&bad.to_string())
            ),
        ));
    }
    std::fs::rename(tmp, path).map_err(|e| {
        let _ = std::fs::remove_file(tmp);
        err(StatusCode::INTERNAL_SERVER_ERROR, format!("保存失败: {e}"))
    })?;
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
    let BaseUrlQuery { base_url, api_key, provider } = q;
    let draft_base = base_url.clone().unwrap_or_default().trim().to_string();
    // B8：草稿地址同样过内网判据（这条接口虽然仅 admin，判据仍与另外两处同源 ——
    // 万一将来放开到用户层，这里不会成为后门）。
    assert_base_url_allowed(&draft_base, user.is_admin())?;

    // ① 内置表命中：**零网络请求**（连 api_key 都不需要 —— 也不需要读盘）。
    //    直接在表单里填了地址的就是这一档（最常见的路径，别为它去读盘）。
    if !draft_base.is_empty() {
        let known = yekitrim_llm::catalog::models_for(&draft_base);
        if !known.is_empty() {
            return ok(serde_json::json!({
                "source": "catalog",
                "models": known
                    .into_iter()
                    .map(|(id, name)| serde_json::json!({ "id": id, "name": name }))
                    .collect::<Vec<_>>(),
            }));
        }
    }

    // ② 解析"用哪套连接"：**表单草稿 > provider=<id> 那份 > legacy 顶层**（见 resolve_llm_conn）。
    //    key 取自那一份接口自己的已存值（用户就是踩在"只认顶层 key"上）；同步读盘下 blocking 池。
    let owner = crate::auth::owner_of(&user);
    let llm = blocking(move || Ok(yekitrim_core::config::load_llm_config_for(owner))).await?;
    let conn = resolve_llm_conn(&llm, provider.as_deref(), base_url.as_deref(), api_key.as_deref(), true)?;
    // B8：已存 provider 的地址同样判一遍（与保存时的文本级判据互为兜底）
    assert_base_url_allowed(&conn.base_url, user.is_admin())?;
    let base = conn.base_url.clone();
    // provider 的地址也先查一次表（省一次网络）
    let known = yekitrim_llm::catalog::models_for(&base);
    if !known.is_empty() {
        return ok(serde_json::json!({
            "source": "catalog",
            "models": known
                .into_iter()
                .map(|(id, name)| serde_json::json!({ "id": id, "name": name }))
                .collect::<Vec<_>>(),
        }));
    }
    match yekitrim_llm::discovery::discover(&conn.api_key, &base).await {
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


/// 这次请求**用哪一套连接参数**（取模型 / 读能力 / 测连接三处共用这一份）。
#[derive(Debug, Clone, PartialEq, Eq)]
struct LlmConn {
    /// 用了哪一份 provider（`None` = legacy 顶层字段）
    provider: Option<String>,
    api_key: String,
    base_url: String,
    model: String,
}

/// **唯一解析入口**：这次用哪把 key / 哪个地址。
///
/// 优先级（用户口径 2026-10-20：「取模型/测试连接要用**该接口自己的** api_key」）：
/// ① **表单草稿**：请求里显式给了 `base_url` / `api_key`（非空才算 —— 刚填还没保存的那种）；
/// ② **`provider=<id>` 那一份**（没传 id 就用 `active` 那份）—— **该接口自己的已存 key**。
///
/// ⚠️ **v4 没有「legacy 顶层」这一档了**：key / 地址 / 模型全在 `[[llm.providers]]` 里
/// （顶层 `api_key` / `base_url` / `model` 三个字段已经不存在）⇒「当前生效的那份」一律走
/// `effective_*`（唯一口径），只有显式给了 `provider=<id>` 时才另取那一份。
///
/// ⚠️ 报错文案按情形分：有接口时**绝不许**说 `[llm] api_key`（那是个用户在界面上看不见的字段，
/// 他明明已经在「API 密钥」里填过了）。
fn resolve_llm_conn(
    llm: &yekitrim_core::config::LlmConfigV4,
    provider_id: Option<&str>,
    draft_base: Option<&str>,
    draft_key: Option<&str>,
    // 这条路径**要不要 key**（读能力只用地址/模型，不需要 key）
    require_key: bool,
) -> Result<LlmConn, AppError> {
    let draft_base = draft_base.map(str::trim).filter(|s| !s.is_empty());
    let draft_key = draft_key.map(str::trim).filter(|s| !s.is_empty());
    // 选哪一份：显式 id > active（一份接口都没有 ⇒ 两个都是 None）
    let wanted = provider_id.map(str::trim).filter(|s| !s.is_empty());
    let explicit = match wanted {
        Some(id) => Some(llm.provider(id).ok_or_else(|| {
            err(StatusCode::BAD_REQUEST, format!("找不到这份配置：{id}"))
        })?),
        None => None,
    };

    let Some(p) = explicit.or_else(|| llm.active_provider()) else {
        // ── 一份接口都没有（全新安装 / 刚被清空）：只剩表单草稿可按，报错文案指到设置页
        let base_url = if require_key {
            probe_request(draft_base.unwrap_or(""), draft_key, llm)?
        } else {
            let b = draft_base.map(str::to_string).unwrap_or_else(|| llm.effective_base_url());
            let b = b.trim().to_string();
            if b.is_empty() {
                return Err(err(StatusCode::BAD_REQUEST, "缺少 base_url：要读哪支模型的能力？"));
            }
            b
        };
        return Ok(LlmConn {
            provider: None,
            api_key: draft_key.map(str::to_string).unwrap_or_else(|| llm.effective_api_key()),
            base_url,
            model: llm.effective_model(),
        });
    };

    // ── 有接口：草稿优先，其次**这份接口自己的**已存值
    // （`key()` 的取值口径：显式 key 优先，其次 `api_key_env`；显式 id 与 active 不同也不会串）
    let (saved_base, saved_key, saved_model) = match explicit {
        Some(x) => (x.base_url.trim().to_string(), x.key(), x.model.trim().to_string()),
        None => (
            llm.effective_base_url().trim().to_string(),
            llm.effective_api_key(),
            llm.effective_model().trim().to_string(),
        ),
    };
    let base_url = draft_base.map(str::to_string).unwrap_or(saved_base);
    let api_key = draft_key.map(str::to_string).unwrap_or(saved_key);
    if base_url.trim().is_empty() {
        return Err(err(StatusCode::BAD_REQUEST, "缺少 API 地址：要探哪个地址的模型清单？"));
    }
    if require_key && api_key.trim().is_empty() {
        return Err(err(
            StatusCode::BAD_REQUEST,
            format!(
                "这份配置还没填 API 密钥（{}）—— 在「API 密钥」那一栏填好再试",
                p.name.trim().if_empty_else(&p.id)
            ),
        ));
    }
    Ok(LlmConn { provider: Some(p.id.clone()), api_key, base_url, model: saved_model })
}

/// 显示名空着就用 id（只给上面的报错文案用）。
trait IfEmptyElse {
    fn if_empty_else<'a>(&'a self, fallback: &'a str) -> &'a str;
}
impl IfEmptyElse for str {
    fn if_empty_else<'a>(&'a self, fallback: &'a str) -> &'a str {
        if self.is_empty() { fallback } else { self }
    }
}

/// 一份接口都没有时的两道检查：**key 没配就别发一个必然 401 的请求**；两个 base_url 都空就没得探。
///
/// 拆成纯函数是为了**能钉住**：本机的真实程序目录（`target/debug/users/<uid>/.yekitrim/llm.toml`）
/// 里通常已经躺着一把真 key，HTTP 级用例钉不了"没配 key"那条分支（会真发一个网络请求）。
fn probe_request(
    asked: &str,
    draft_key: Option<&str>,
    llm: &yekitrim_core::config::LlmConfigV4,
) -> Result<String, AppError> {
    // ⚠️ 表单里刚填、**还没保存**的 key 也算数：用户就是在"先填地址+密钥 → 点获取模型 → 选模型 → 保存"
    // 这条路上踩过「还没配置 API key」——只认已存值等于逼他先保存一次才能选模型。
    let has_key = draft_key.map(str::trim).is_some_and(|k| !k.is_empty()) || llm.has_api_key();
    if !has_key {
        return Err(err(
            StatusCode::BAD_REQUEST,
            "还没配置 API key（设置 → LLM 配置 里的「API 密钥」）—— 填好再取模型",
        ));
    }
    // 请求里给了就用它（草稿），没给才回落到已存的那条
    let stored = llm.effective_base_url();
    let base = if asked.trim().is_empty() { stored.trim() } else { asked.trim() };
    if base.is_empty() {
        return Err(err(StatusCode::BAD_REQUEST, "缺少 base_url：要探哪个地址的模型清单？"));
    }
    Ok(base.to_string())
}



/// `GET /api/config/llm/protocols`：**协议注册表的清单**（前端「API 协议」下拉的数据源）。
///
/// 与 `/config/llm/dialects` 同一条口径：**从注册表现算**（`yekitrim_llm::protocol::PROTOCOLS`）、
/// 只读、不联网、不读盘 ⇒ 挂用户层（每个账号都要靠它画下拉）。
/// ⚠️ **前端别硬编码协议清单**：以后加一家协议只会加一条表项，这里自动跟着变。
pub(super) async fn llm_protocols() -> ApiResult {
    let list: Vec<serde_json::Value> = yekitrim_llm::protocol::PROTOCOLS
        .iter()
        .map(|p| serde_json::json!({ "id": p.id, "label": p.label }))
        .collect();
    ok(serde_json::json!({ "protocols": list }))
}

/// `GET /api/config/llm/dialects`：**方言注册表的清单**（设置页「模型能力覆盖」的下拉数据源）。
///
/// 三条硬口径：
/// 1. **从 `yekitrim_llm::dialect::DIALECTS` 现算** —— 这里**绝不抄一份清单**（那就是第二个腐坏源：
///    以后加一条方言，没人会记得回来改 server）；
/// 2. **按 id 去重**，但把该 id 支持的 `max_output_field` **全列出来**（同一套推理写法 +
///    不同输出字段名 = 两种标准；界面只需要"这家能手写哪两个值"）；
/// 3. **只读、不联网、不读盘**（纯静态数据，所以它和 `/config/llm` 同一层：每个账号都要能拿到）。
///
/// `none` 也在清单里（`none_id`）—— 前端用它表示"不发推理参数"，那是合法状态、不是"没选"。
pub(super) async fn llm_dialects() -> ApiResult {
    // id → (label, 该 id 支持的输出字段名，按 DIALECTS 顺序去重)
    let mut out: Vec<serde_json::Value> = Vec::new();
    for d in yekitrim_llm::dialect::DIALECTS {
        match out.iter_mut().find(|v| v["id"] == d.id) {
            Some(existing) => {
                let fields = existing["max_output_fields"]
                    .as_array_mut()
                    .expect("上面刚建的是数组");
                if !fields.iter().any(|f| f == d.max_output_field) {
                    fields.push(serde_json::json!(d.max_output_field));
                }
            }
            None => out.push(serde_json::json!({
                "id": d.id,
                "label": d.label,
                "max_output_fields": [d.max_output_field],
            })),
        }
    }
    ok(serde_json::json!({
        "dialects": out,
        // "不发推理参数"的那一条：**从注册表现取**，不在前端/这里写死
        "none_id": yekitrim_llm::dialect::none().id,
    }))
}

/// `GET /api/config/llm/catalog?base_url=…&model=…`（**仅 admin**）：这支模型**支持哪些旋钮**。
///
/// 设置页「按能力渲染控件」的唯一数据源：**只查内置表**（不联网、不读盘、不落盘）。
/// 表里没有这对 (host, model) 时回**保守默认 + `hit: false`**，**不是 404** ——
/// 界面照它画通用参数并写明"未收录"即可（回 404 会让人以为接口坏了）。
pub(super) async fn llm_catalog(
    Extension(user): Extension<crate::auth::CurrentUser>,
    Query(q): Query<CatalogQuery>,
) -> ApiResult {
    let draft_base = q.base_url.clone().unwrap_or_default();
    let draft_model = q.model.clone().unwrap_or_default();
    // 常见路径（表单里填了地址+模型）：**只查表、不读盘**，保持不变
    if !draft_base.trim().is_empty() && !draft_model.trim().is_empty() {
        return ok(serde_json::json!({
            "source": "catalog",
            "caps": yekitrim_llm::catalog::caps_json(&draft_base, &draft_model),
        }));
    }
    // 否则按"哪一份配置"补齐缺的那一半（读能力**不需要 key** ⇒ require_key = false）
    let owner = crate::auth::owner_of(&user);
    let llm = blocking(move || Ok(yekitrim_core::config::load_llm_config_for(owner))).await?;
    let conn = resolve_llm_conn(&llm, q.provider.as_deref(), Some(&draft_base), None, false)?;
    let base_url = if draft_base.trim().is_empty() { conn.base_url } else { draft_base };
    let model = if draft_model.trim().is_empty() { conn.model } else { draft_model };
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
    /// **哪一份配置文件**（`[[llm.providers]]` 的 id）。
    ///
    /// 不传就用 `active` 那份（就是用户当前在用的）；**两种都没有才是 legacy 顶层字段**。
    /// 用户实测踩的就是这条：provider 里填了密钥、探针却只认 `[llm] api_key`。
    #[serde(default)]
    pub provider: Option<String>,
}

/// `/api/config/llm/catalog` 的查询串。
#[derive(serde::Deserialize)]
pub(super) struct CatalogQuery {
    #[serde(default)]
    pub base_url: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    /// 哪一份配置文件（不传 = `active` 那份；都没有 = legacy 顶层）。
    #[serde(default)]
    pub provider: Option<String>,
}

/// \`/api/llm/test\` 的查询串：\`?provider=<id>\`（不传 = \`active\` 那份；都没有 = legacy）。
#[derive(serde::Deserialize)]
pub(super) struct TestQuery {
    #[serde(default)]
    pub provider: Option<String>,
}

pub(super) async fn llm_test(
    Extension(user): Extension<crate::auth::CurrentUser>,
    Query(q): Query<TestQuery>,
) -> ApiResult {
    // 只有"读配置"是同步 IO（HTTP 调用本身是 async，保持不动）
    let owner = crate::auth::owner_of(&user);
    let llm = blocking(move || Ok(yekitrim_core::config::load_llm_config_for(owner))).await?;
    // **用该接口自己的 key**（表单草稿 > provider=<id> > legacy）；报错文案按模式分
    let conn = resolve_llm_conn(&llm, q.provider.as_deref(), None, None, true)?;
    // B8：普通账号不许让服务器朝本机/内网发请求 —— 保存时拦得住，测试连接这里也拦
    // （判据与 config_llm_put / llm_models 同一份纯函数）。
    assert_base_url_allowed(&conn.base_url, user.is_admin())?;
    // 测连接只需要"这一套连接参数"：把解析结果落成**一份接口**再交给 test_llm。
    // ⚠️ 先 `for_link(Utility)` 把链路参数并进 defaults、并清掉 links —— 否则 test_llm 内部那次
    //    `for_link` 会按链路把接口/模型换成配置里那一份，测的就不是这次解析出来的连接了。
    let mut plain = llm
        .for_link(yekitrim_core::config::LlmLink::Utility)
        .config()
        .clone();
    plain.providers = vec![yekitrim_core::config::llm_v4::LlmProviderV4 {
        id: conn.provider.clone().unwrap_or_else(|| "test".to_string()),
        name: "测试连接".to_string(),
        base_url: conn.base_url.clone(),
        api_key: conn.api_key.clone(),
        model: conn.model.clone(),
        ..Default::default()
    }];
    plain.active = plain.providers[0].id.clone();
    let start = std::time::Instant::now();
    let test_ok = yekitrim_core::detection::test_llm(&plain, "ping").await.is_ok();
    let latency_ms = start.elapsed().as_millis() as u64;
    if test_ok {
        ok(serde_json::json!({ "ok": true, "latency_ms": latency_ms, "model": conn.model, "provider": conn.provider }))
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
    // 取消令牌：下载是分钟级的（qwen3 那个包 838 MB），只在阶段边界查等于"点了取消没反应"
    let cancel = state.jobs.cancel_token(&id).await;
    // 下载完要解包（tar.bz2，纯同步 IO）→ 任务线程
    spawn_job(jobs.clone(), job_id.clone(), move || async move {
        if handle.wait_start().await.is_err() {
            jobs.complete(&job_id).await;
            return;
        }
        // 进度桥：整批字节 → 任务的 progress（条）+ 文案（"下载 qwen3-asr"）
        let progress = crate::api::core::JobDownloadProgress::new(
            jobs.clone(),
            job_id.clone(),
            cancel,
        );
        let result = yekitrim_core::download::download_all_with(None, &progress).await;
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

    /// 建一个**落在指定 uid** 的非 admin 账号并返回它（先建占位账号把"下一个 id"顶到 `want`）。
    ///
    /// **为什么必须按号分片**：`owner::state_dir(Owner::User(uid))` 落在 **exe 旁的共享目录**
    /// （`target/debug/users/<uid>/.yekitrim`；测试二进制在 `target/debug/deps/`，是同一棵树），
    /// 而每个用例的账号账本都是**新建的临时库** ⇒ uid 一律从 2 数起。于是"两条用例各自建的第 1 个
    /// 用户"会指向**同一个** `users/2/.yekitrim/llm.toml`，并行跑就互相覆盖 —— 实测两种翻法：
    /// ① 读到对方写的公网 `base_url` ⇒ 真发请求 ⇒ 500（期望 403）；
    /// ② PUT 撞上对方残留的 `127.0.0.1` ⇒ 403（期望 200）。
    ///
    /// 凡是**会写**这个目录的用例都按下表取号，各拿专属 uid（断言写在调用点，uid 漂了要立刻红）：
    /// `cfg_p`=2 · `cfg_a`=3 · `cfg_b`=4 · `b8_user`=5。
    /// 只读、或根本在写盘前就被拦下的用例不必取号（`cfg_q` 的坏值 400 在校验期返回、`llm_plain` 只打 GET）。
    ///
    /// ⚠️ **别改成 `YEKITRIM_DATA_DIR`**：`test_support::test_app` 的注释写明"测试是并行跑的、改全局
    /// 环境变量会互相踩"（账号账本正因此走 tempdir 而不是环境变量）；uid 分片是本文件里唯一不碰
    /// 全局状态的做法。
    fn user_at_uid(t: &TestApp, want: i64, name: &str, hash: &str) -> i64 {
        let db = t.auth.db();
        let next_id = db.list_users().unwrap().iter().map(|u| u.id).max().unwrap_or(0) + 1;
        assert!(
            next_id <= want,
            "uid {want} 已被占用（下一个可用 id 是 {next_id}）—— 前面是不是多建了用户？"
        );
        // 占位账号把"下一个 id"一路顶到 want（名字带序号，不会重名）。
        for i in next_id..want {
            db.create_user(&format!("pad_{name}_{i}"), hash, "user", false, 0).unwrap();
        }
        let uid = db.create_user(name, hash, "user", false, 0).unwrap();
        assert_eq!(uid, want, "{name} 应落在 uid {want}");
        uid
    }

    /// 用例收尾：清掉该账号树（`users/<uid>/.yekitrim`）。
    ///
    /// 不清的话同一份 `llm.toml` 会留到**下一次跑测试**（uid 复用，见 `user_at_uid`），
    /// 于是"乙还没配 key"那类断言第二次跑就红（实测 left: true / right: false）。
    fn wipe_state_dir(uid: i64) {
        let _ = std::fs::remove_dir_all(yekitrim_core::owner::state_dir(
            yekitrim_core::owner::Owner::User(uid),
        ));
    }

    /// **方言清单必须从注册表现算**（server 里不许抄一份）：id 去重后的条数 == `DIALECTS` 的 id 数；
    /// 每条都有非空 label 与至少一个输出字段名；`none` 也在清单里（前端用它表示"不发推理参数"）。
    ///
    /// ⚠️ 期望值一律**从注册表现算**（不写死条数、不写死字段名字面量 —— 后者会撞
    /// `only_llm_knows_wire`，而且写死就等于又抄了一份清单）。
    #[tokio::test]
    async fn llm_dialects_are_computed_from_the_registry() {
        use yekitrim_llm::dialect;
        let Json(v) = llm_dialects().await.expect("纯静态数据，不该失败");
        let list = v["dialects"].as_array().expect("dialects 是数组");

        let mut expect_ids: Vec<&str> = dialect::DIALECTS.iter().map(|d| d.id).collect();
        expect_ids.sort_unstable();
        expect_ids.dedup();
        assert_eq!(list.len(), expect_ids.len(), "id 去重后的条数必须等于注册表：{list:?}");
        for id in &expect_ids {
            assert!(list.iter().any(|d| d["id"] == *id), "{id} 不在清单里：{list:?}");
        }
        for item in list {
            assert!(!item["label"].as_str().unwrap_or("").is_empty(), "每条都要有中文名：{item}");
            assert!(
                !item["max_output_fields"].as_array().map(|a| a.is_empty()).unwrap_or(true),
                "每条至少一个输出字段名：{item}"
            );
        }
        // "不发推理参数"那条也在（且 id 是现算的）
        assert_eq!(v["none_id"], dialect::none().id);
        assert!(list.iter().any(|d| d["id"] == dialect::none().id), "{list:?}");

        // 同一 id 的多行 → **一个条目、字段名全列**（拿表里行数最多的那个 id 验）
        let busiest = expect_ids
            .iter()
            .max_by_key(|id| dialect::DIALECTS.iter().filter(|d| d.id == **id).count())
            .expect("注册表非空");
        let entry = list.iter().find(|d| d["id"] == *busiest).expect("必须能找到");
        let mut got: Vec<&str> = entry["max_output_fields"]
            .as_array()
            .expect("数组")
            .iter()
            .filter_map(|f| f.as_str())
            .collect();
        got.sort_unstable();
        let mut want: Vec<&str> = dialect::DIALECTS
            .iter()
            .filter(|d| d.id == *busiest)
            .map(|d| d.max_output_field)
            .collect();
        want.sort_unstable();
        want.dedup();
        assert_eq!(got, want, "{busiest} 的输出字段名要列全且不重复");
    }

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
    /// $env:VT_LLM_BASE = "http://127.0.0.1:7863/v1"; $env:VT_LLM_KEY = $k
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
            Query(BaseUrlQuery { base_url: base, api_key: key, provider: None }),
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
        // P3：**试过哪些路径**也要看得见（路径级探测失败不阻断主流程，但必须留痕）。
        if v["source"] == "probe" {
            let tried: Vec<&str> = v["tried"]
                .as_array()
                .map(|a| a.iter().filter_map(|x| x.as_str()).collect())
                .unwrap_or_default();
            println!("TRIED={tried:?} CAPS_FROM={}", v["caps_from"]);
            assert!(tried.contains(&"/models"), "主路径必须记进 tried：{tried:?}");
            assert!(
                tried.contains(&"/model/info"),
                "LiteLLM 那条路必须**试过**（哪怕 404）：{tried:?}"
            );
            assert!(
                tried.contains(&"/api/tags"),
                "Ollama 那条路必须**试过**（哪怕 404）：{tried:?}"
            );
        }
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




    /// **数组是整体替换**（深合并只对对象递归）+ **空 key 按 id 填回**：文件级判据（不经 HTTP）。
    #[test]
    fn patch_replaces_provider_array_and_restores_keys_by_id() {
        let dir = std::env::temp_dir().join(format!(
            "vt-llm-patch-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("llm.toml");
        std::fs::write(
            &path,
            "[llm]\nactive = \"a\"\n\n[[llm.providers]]\nid = \"a\"\napi_key = \"ka\"\nmodel = \"m\"\n\n[[llm.providers]]\nid = \"b\"\napi_key = \"kb\"\nmodel = \"n\"\n",
        )
        .unwrap();
        // 只发一份（a），**不带 api_key** —— 数组整体替换后 a 的 key 必须还在
        let incoming = serde_json::json!({ "llm": { "active": "a", "providers": [
            { "id": "a", "name": "甲", "api": "openai-chat", "model": "m" }
        ] } });
        let stored = std::fs::read_to_string(&path).unwrap();
        let patch = fill_blank_provider_keys(&drop_blank_api_key(&incoming), &stored);
        assert_eq!(patch["llm"]["providers"][0]["api_key"], "ka", "已存的 key 要按 id 填回去：{patch}");
        let res = config_llm_put_locked(&path, &serde_json::json!({ "patch": patch }), true);
        assert!(res.is_ok(), "patch 应当成功");
        let text = std::fs::read_to_string(&path).unwrap();
        println!("PATCHED=\n{text}");
        assert!(text.contains("ka"), "已存的 key 要留在文件里：\n{text}");
        assert!(!text.contains("kb"), "被替换掉的 provider b 不该还在：\n{text}");
        let f: toml::Value = toml::from_str(&text).unwrap();
        let arr = f.get("llm").and_then(|l| l.get("providers")).and_then(|p| p.as_array()).unwrap();
        assert_eq!(arr.len(), 1, "数组是整体替换：\n{text}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// **协议清单从注册表现算**（server 里不许抄一份）：条数 == `PROTOCOLS`、每条有 id + 中文名。
    #[tokio::test]
    async fn llm_protocols_are_computed_from_the_registry() {
        let Json(v) = llm_protocols().await.expect("纯静态数据，不该失败");
        let list = v["protocols"].as_array().expect("protocols 是数组");
        assert_eq!(list.len(), yekitrim_llm::protocol::PROTOCOLS.len(), "{list:?}");
        for (item, want) in list.iter().zip(yekitrim_llm::protocol::PROTOCOLS) {
            assert_eq!(item["id"], want.id, "{item}");
            assert_eq!(item["label"], want.label, "{item}");
        }
        assert!(list.iter().any(|p| p["id"] == "openai-chat"), "本轮唯一的一条：{list:?}");
    }

    /// **多条 provider 的 api_key 一条都不许出接口**（顶层抹了、provider 里忘了，是最容易漏的那种）。
    #[tokio::test]
    async fn every_provider_key_is_redacted_and_blank_key_keeps_the_stored_one() {
        let t = test_app().await;
        let hash = yekitrim_service::auth::hash_password("pass-12345678").unwrap();
        let uid = user_at_uid(&t, 2, "cfg_p", &hash);
        assert_eq!(uid, 2, "cfg_p 固定占 uid 2（见 user_at_uid 的分片表）");
        let a = t.auth.login("cfg_p", "pass-12345678", None, None).unwrap();

        async fn put(t: &TestApp, sid: &str, csrf: &str, body: serde_json::Value) -> axum::response::Response {
            t.call(
                t.req_as(sid, csrf, "PUT", "/config/llm")
                    .header("content-type", "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
        }
        async fn cfg(t: &TestApp, sid: &str, csrf: &str) -> serde_json::Value {
            let res = t.call(t.req_as(sid, csrf, "GET", "/config").body(Body::empty()).unwrap()).await;
            body_json(res).await
        }

        // 先把遗留的顶层 key 清掉（测试数据目录按 uid 复用，上一轮/别的用例可能留着它）——
        // 否则"空 key = 不修改"那条会把旧 key 补回顶层，多出一行 api_key，断言条数就对不上了。
        let res = put(
            &t,
            &a.session_id,
            &a.csrf,
            serde_json::json!({ "patch": { "llm": {
                "api_key": null, "active": null, "providers": []
            } } }),
        )
        .await;
        assert_eq!(res.status(), 200, "清掉遗留 key / provider");

        let two = r#"[llm]
active = "a"

[[llm.providers]]
id = "a"
name = "甲"
api = "openai-chat"
base_url = "https://api.deepseek.com/v1"
api_key = "key-provider-a"
model = "deepseek-v4-pro"

[[llm.providers]]
id = "b"
name = "乙"
api = "openai-chat"
base_url = "https://api.moonshot.ai/v1"
api_key = "key-provider-b"
model = "kimi-k3"
"#;
        let res = put(&t, &a.session_id, &a.csrf, serde_json::json!({ "raw": two })).await;
        assert_eq!(res.status(), 200, "两份 provider 要能存");

        let j = cfg(&t, &a.session_id, &a.csrf).await;
        let dumped = j.to_string();
        assert!(!dumped.contains("key-provider-a"), "provider a 的明文不许出接口：{dumped}");
        assert!(!dumped.contains("key-provider-b"), "provider b 的明文不许出接口：{dumped}");
        let raw = j["llm_raw"].as_str().unwrap_or("");
        assert!(!raw.contains("key-provider-a") && !raw.contains("key-provider-b"), "llm_raw 也要逐行抹：{raw}");
        assert_eq!(raw.matches("api_key = \"\"").count(), 2, "两条 provider 的值都要抹（按行扫）：{raw}");
        let provs = j["llm_config"]["providers"].as_array().expect("providers");
        assert_eq!(provs.len(), 2, "{j}");
        assert_eq!(j["llm_config"]["active"], "a", "active 要回给前端：{j}");
        for p in provs {
            assert_eq!(p["api_key"], "", "每条 provider 的 key 都要抹：{p}");
            assert_eq!(p["api_key_set"], serde_json::json!(true), "每条各自给一个 api_key_set：{p}");
        }

        // **数组是整体替换**：只发一份 ⇒ 列表变成一份（不是合并成两份）
        let res = put(
            &t,
            &a.session_id,
            &a.csrf,
            serde_json::json!({ "patch": { "llm": {
                "active": "a",
                "providers": [{ "id": "a", "name": "甲", "api": "openai-chat",
                                "base_url": "https://api.deepseek.com/v1", "model": "deepseek-v4-pro" }]
            } } }),
        )
        .await;
        assert_eq!(res.status(), 200, "patch 要能存");
        let j = cfg(&t, &a.session_id, &a.csrf).await;
        let provs = j["llm_config"]["providers"].as_array().expect("providers");
        assert_eq!(provs.len(), 1, "数组是**整体替换**（不是按 id 合并）：{j}");
        assert_eq!(provs[0]["id"], "a");
        assert_eq!(
            provs[0]["api_key_set"], serde_json::json!(true),
            "⚠️ 没带 api_key = 不修改：那条已存的 key 要按 id 填回去（否则保存一次就抹了）：{j}"
        );
        assert!(!j.to_string().contains("key-provider-a"), "填回去的 key 也不许回显：{j}");
        wipe_state_dir(uid);
    }

    /// 协议写错要**报错**（不静默回退成 openai-chat —— 那会让人以为在走别的协议）。
    #[tokio::test]
    async fn unknown_api_is_rejected_on_write() {
        let t = test_app().await;
        let hash = yekitrim_service::auth::hash_password("pass-12345678").unwrap();
        t.auth.db().create_user("cfg_q", &hash, "user", false, 0).unwrap();
        let a = t.auth.login("cfg_q", "pass-12345678", None, None).unwrap();
        let res = t
            .call(
                t.req_as(&a.session_id, &a.csrf, "PUT", "/config/llm")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({ "patch": { "llm": {
                            "active": "x",
                            "providers": [{ "id": "x", "api": "gemini-native", "model": "m" }]
                        } } })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await;
        assert_eq!(res.status(), 400, "认不出的 api 要 400");
        let j = body_json(res).await;
        assert!(
            j["error"].as_str().unwrap_or("").contains("gemini-native"),
            "要点名那个认不出的协议（并给可选值）：{j}"
        );
    }

    /// **`PUT /api/config/global` 的坏值必须 400 且原文件一个字节都不动**（review B3）。
    ///
    /// 为什么这条重要：`load_config_for` 对解析失败是 fail-soft（整份文件被忽略）——
    /// 一个类型写错的值语法完全合法，却会让**所有**键一起回退默认值（实测：同文件里的
    /// `chunk_duration = 42` 也读回 60）。所以写盘前必须真反序列化到 `Config` 再 rename。
    ///
    /// 四条判据：① 语义坏值（`num_workers = "lots"`）→ 400 + 字节不变；② 语法坏值 → 400；
    /// ③ 合法 patch 生效且保留兄弟键；④ 语义坏的 patch 同样不落盘、不留临时文件。
    /// 走 `config_global_put_locked`（handler 的内核）而不是打 HTTP：全局配置路径在真实数据目录下，
    /// 用例必须落在 tempdir 里才不串。
    #[test]
    fn global_config_put_rejects_semantic_breakage_and_keeps_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let good = "config_version = 3\n\n[asr]\nnum_workers = 2\nchunk_duration = 42.0\n";
        std::fs::write(&path, good).unwrap();

        // ① 语义坏值：语法完全合法，但反序列化不过
        let bad = config_global_put_locked(
            &path,
            &serde_json::json!({ "raw": "config_version = 3\n\n[asr]\nnum_workers = \"lots\"\nchunk_duration = 7.0\n" }),
        );
        assert_eq!(bad.unwrap_err().0, StatusCode::BAD_REQUEST, "语义坏值必须 400");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), good, "坏值不许落盘（一个字节都不许动）");
        assert!(!path.with_extension("toml.guard").exists(), "失败后不许留临时文件");

        // ② 语法坏值
        let bad = config_global_put_locked(&path, &serde_json::json!({ "raw": "[asr\nnum_workers = 2" }));
        assert_eq!(bad.unwrap_err().0, StatusCode::BAD_REQUEST);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), good);

        // ③ 合法 patch：生效、且不碰兄弟键
        let _ = config_global_put_locked(
            &path,
            &serde_json::json!({ "patch": { "asr": { "chunk_duration": 30.0 } } }),
        )
        .expect("合法 patch 应当成功");
        let after = std::fs::read_to_string(&path).unwrap();
        assert!(after.contains("30.0"), "patch 要生效：{after}");
        assert!(after.contains("num_workers = 2"), "兄弟键不许丢：{after}");

        // ④ 语义坏的 patch：同样 400 且不落盘
        let before = std::fs::read_to_string(&path).unwrap();
        let bad = config_global_put_locked(
            &path,
            &serde_json::json!({ "patch": { "asr": { "num_workers": "lots" } } }),
        );
        assert_eq!(bad.unwrap_err().0, StatusCode::BAD_REQUEST);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), before, "坏 patch 不许落盘");
        assert!(!path.with_extension("toml.guard").exists());
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
    let uid_a = user_at_uid(&t, 3, "cfg_a", &hash);
    let uid_b = user_at_uid(&t, 4, "cfg_b", &hash);
    assert_eq!(
        (uid_a, uid_b),
        (3, 4),
        "cfg_a / cfg_b 固定占 uid 3 / 4（见 user_at_uid 的分片表）"
    );
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
    /// 当前生效的模型 id：**v4 只在 `providers[<active>].model` 里**（顶层 `model` 已经没有了）。
    fn model_of(j: &serde_json::Value) -> String {
        let active = j["llm_config"]["active"].as_str().unwrap_or("");
        j["llm_config"]["providers"]
            .as_array()
            .and_then(|arr| {
                arr.iter()
                    .find(|p| active.is_empty() || p["id"] == active)
                    .or_else(|| arr.first())
            })
            .and_then(|p| p["model"].as_str())
            .unwrap_or("")
            .to_string()
    }
    async fn get_cfg(t: &TestApp, sid: &str, csrf: &str) -> serde_json::Value {
        let res = t
            .call(t.req_as(sid, csrf, "GET", "/config").body(Body::empty()).unwrap())
            .await;
        body_json(res).await
    }

    // 甲写自己的 key（v4：连接面只有 `[[llm.providers]]`）。
    let res = put_llm(
        &t,
        &a.session_id,
        &a.csrf,
        "[llm]
active = \"default\"

[[llm.providers]]
id = \"default\"
base_url = \"https://api.example.com/v1\"
api_key = \"key-of-alice\"
model = \"m-a\"
",
    )
    .await;
    assert_eq!(res.status(), 200, "甲应能存自己的 LLM 配置");

    // 甲看得见**自己的配置**（model 是归属的判据 —— 明文 key 已经不回接口了）。
    let ja = get_cfg(&t, &a.session_id, &a.csrf).await;
    assert_eq!(model_of(&ja), "m-a", "甲应读到自己那份配置：{ja}");
    assert_eq!(ja["api_key_set"], serde_json::json!(true), "甲配了 key：{ja}");
    // ② 整份响应里**一个字**明文都不许有（llm_raw 与 llm_config 两处都要清）
    let dumped_a = ja.to_string();
    assert!(
        !dumped_a.contains("key-of-alice"),
        "明文 API key 不许出接口（llm_raw / llm_config 都得抹）：{dumped_a}"
    );
    assert_eq!(
        ja["llm_config"]["providers"][0]["api_key"], "",
        "provider 里的明文也要清（键名留着、值置空）：{ja}"
    );
    let raw_a = ja["llm_raw"].as_str().unwrap_or("");
    assert!(raw_a.contains("api_key = \"\""), "键与行位置要留着（源文件编辑器靠它）：{raw_a}");
    assert!(raw_a.contains("m-a"), "没被抹的键原样保留：{raw_a}");

    // 先把乙那份**清干净**再断言：测试的配置目录是 exe 同级的 users/<uid>/.yekitrim
    // （测试二进制在 target/debug/deps/ ⇒ 那边也有一份 users/），而 uid 在每次新建的临时账本里
    // **又从 1 数起** ⇒ 同一个 uid 的 llm.toml 会在**多次跑测试之间留着**。
    // 不清就断言"乙还没配 key"，第二次跑必然红（实测：left: true, right: false）。
    // 顺手也就把 null = 删键 这条路钉住了（它是唯一能表达"清掉 key"的写法）。
    // ⚠️ v4 的 key 只在 `[[llm.providers]]` 里 ⇒ 清它就等于清 key：
    //    `providers = []`（数组整体替换）+ `active = null`（指向不存在的接口该一起清掉）。
    let res = t
        .call(
            t.req_as(&b.session_id, &b.csrf, "PUT", "/config/llm")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({ "patch": { "llm": {
                        "active": null, "providers": []
                    } } })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await;
    assert_eq!(res.status(), 200, "null 应该能删掉 key");

    // 乙**看不见**甲的配置。
    let jb = get_cfg(&t, &b.session_id, &b.csrf).await;
    assert_eq!(jb["api_key_set"], serde_json::json!(false), "乙还没配 key：{jb}");
    assert_ne!(model_of(&jb), "m-a", "乙不该读到甲的配置：{jb}");
    assert!(
        !jb.to_string().contains("key-of-alice"),
        "乙的响应里更不该有甲的 key：{jb}"
    );

    // 乙写自己的，两边互不覆盖。
    let res = put_llm(&t, &b.session_id, &b.csrf, "[llm]
active = \"default\"

[[llm.providers]]
id = \"default\"
base_url = \"https://api.example.com/v1\"
api_key = \"key-of-bob\"
model = \"m-b\"
").await;
    assert_eq!(res.status(), 200);
    let ja2 = get_cfg(&t, &a.session_id, &a.csrf).await;
    assert_eq!(model_of(&ja2), "m-a", "乙写自己的配置**不该**动到甲的：{ja2}");
    assert_eq!(ja2["api_key_set"], serde_json::json!(true));
    assert!(!ja2.to_string().contains("key-of-bob"), "也不该串到甲的响应里：{ja2}");
    let jb2 = get_cfg(&t, &b.session_id, &b.csrf).await;
    assert_eq!(model_of(&jb2), "m-b", "{jb2}");
    assert_eq!(jb2["api_key_set"], serde_json::json!(true));

    // ③ **空 api_key = 不修改**，两条路都成立：
    //   patch 路 —— 界面"没动那一栏"（读回来是空的）保存一次，key 必须还在
    let res = t
        .call(
            t.req_as(&a.session_id, &a.csrf, "PUT", "/config/llm")
                .header("content-type", "application/json")
                .body(Body::from(
                    // v4：空 api_key = **那一份**不修改（按 id 把已存的 key 填回去）
                    serde_json::json!({ "patch": { "llm": { "providers": [
                        { "id": "default", "base_url": "https://api.example.com/v1",
                          "api_key": "", "model": "m-a2" }
                    ] } } })
                        .to_string(),
                ))
                .unwrap(),
        )
        .await;
    assert_eq!(res.status(), 200);
    let ja3 = get_cfg(&t, &a.session_id, &a.csrf).await;
    assert_eq!(model_of(&ja3), "m-a2", "别的键照常改：{ja3}");
    assert_eq!(ja3["api_key_set"], serde_json::json!(true), "空串不该把 key 清掉：{ja3}");

    //   raw 路 —— "编辑源文件"读到的是**抹过值**的原文，原样存回去同样不许动 key
    let masked = ja3["llm_raw"].as_str().unwrap_or("").to_string();
    assert!(!masked.contains("key-of-alice"), "拿到的原文本来就是抹过的：{masked}");
    let res = put_llm(&t, &a.session_id, &a.csrf, &masked).await;
    assert_eq!(res.status(), 200, "抹过的原文必须能原样存回去");
    let ja4 = get_cfg(&t, &a.session_id, &a.csrf).await;
    assert_eq!(ja4["api_key_set"], serde_json::json!(true), "原样保存不许把真 key 抹掉：{ja4}");
    assert_eq!(model_of(&ja4), "m-a2");

    wipe_state_dir(uid_a);
    wipe_state_dir(uid_b);
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
    // v4：参数基准只有一层 `[llm.defaults]`，链路覆盖在 `[llm.links.<link>]`
    std::fs::write(
        &p,
        "[llm]\n\n[llm.defaults]\nmax_tokens = 16384\n\n[llm.links.detect]\nmax_tokens = 8192\n\n[[llm.providers]]\nid = \"default\"\nbase_url = \"https://api.example.com/v1\"\napi_key = \"k\"\nmodel = \"m\"\n",
    )
    .unwrap();

    let body = serde_json::json!({
        "patch": {"llm": {"links": {"detect": {"max_tokens": null}}}}
    });
    let _ = config_llm_put_locked(&p, &body, true).unwrap();

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




/// **取模型/测连接用"该接口自己的" api_key**（P0 2026-10-20）：优先级 = 表单草稿 > `provider=<id>` > active。
///
/// 用户实测：provider 里填了密钥，探针却只认 `[llm] api_key` ⇒ 报"还没配置 API key"。
/// 这条钉的就是优先级 + **两种情形各自的报错文案**。
/// ⚠️ v4 **没有 legacy 顶层那一档了**（顶层 `api_key` / `base_url` / `model` 三个字段已不存在）：
/// 一份接口都没有就只能靠表单草稿，文案指到"设置 → LLM 配置"。
#[test]
fn resolve_llm_conn_uses_the_providers_own_key() {
    use yekitrim_core::config::llm_v4::LlmProviderV4;
    use yekitrim_core::config::LlmConfigV4;

    fn cfg() -> LlmConfigV4 {
        let mut c = LlmConfigV4::default();
        c.active = "a".into();
        c.providers = vec![
            LlmProviderV4 {
                id: "a".into(),
                name: "甲".into(),
                protocol: "openai-chat".into(),
                base_url: "https://a.example/v1".into(),
                api_key: "k-a".into(),
                model: "m-a".into(),
                ..Default::default()
            },
            LlmProviderV4 {
                id: "b".into(),
                name: "乙".into(),
                protocol: "openai-chat".into(),
                base_url: "https://b.example/v1".into(),
                api_key: String::new(), // ← 没填 key（要能给出对味的报错）
                model: "m-b".into(),
                ..Default::default()
            },
        ];
        c
    }

    // ① provider=<id> ⇒ **用那一份自己的 key**（不是 active 那把）
    let conn = resolve_llm_conn(&cfg(), Some("a"), None, None, true).expect("a 有 key");
    assert_eq!(conn.provider.as_deref(), Some("a"));
    assert_eq!(conn.api_key, "k-a", "必须是这份接口自己的 key");
    assert_eq!(conn.base_url, "https://a.example/v1");
    assert_eq!(conn.model, "m-a");

    // ② 该接口没 key ⇒ 400 + **「这份配置还没填 API 密钥」**，且**不许**提 `[llm] api_key`
    let e = resolve_llm_conn(&cfg(), Some("b"), None, None, true).expect_err("b 没 key 要报错");
    assert_eq!(e.0, StatusCode::BAD_REQUEST);
    let msg = e.1 .0["error"].as_str().unwrap_or("").to_string();
    assert!(msg.contains("这份配置还没填 API 密钥"), "{msg}");
    assert!(!msg.contains("[llm] api_key"), "别让用户去改一个看不见的字段：{msg}");

    // ③ 表单草稿优先于已存值（刚填还没保存的那种）
    let conn = resolve_llm_conn(&cfg(), Some("a"), Some("https://draft.example/v1"), Some("k-draft"), true)
        .expect("草稿齐了");
    assert_eq!(conn.api_key, "k-draft", "草稿 key 压过已存值");
    assert_eq!(conn.base_url, "https://draft.example/v1", "草稿地址也优先");

    // ④ 一份接口都没有（全新安装 / 刚清空）⇒ 没有已存值可回退，只能靠草稿；
    //    报错要指到用户能点的位置，**别再说一个已经不存在了的 `[llm] api_key`**。
    let mut empty = cfg();
    empty.providers.clear();
    empty.active.clear();
    let e = resolve_llm_conn(&empty, None, None, None, true).expect_err("一份接口都没有要报错");
    assert_eq!(e.0, StatusCode::BAD_REQUEST);
    let msg = e.1 .0["error"].as_str().unwrap_or("").to_string();
    assert!(msg.contains("API key"), "{msg}");
    assert!(msg.contains("设置"), "要指到用户能点的位置：{msg}");
    assert!(!msg.contains("[llm] api_key"), "顶层 key 已经不存在了：{msg}");
    let conn = resolve_llm_conn(&empty, None, Some("https://draft.example/v1"), Some("k-draft"), true)
        .expect("草稿齐了就能探");
    assert_eq!(conn.provider, None, "没有已存接口 ⇒ provider 位是 None");
    assert_eq!(conn.base_url, "https://draft.example/v1");
    assert_eq!(conn.api_key, "k-draft");

    // ⑤ 没传 provider ⇒ 用 **active** 那份（用户当前在用的就是它）
    let conn = resolve_llm_conn(&cfg(), None, None, None, true).expect("active 有 key");
    assert_eq!(conn.provider.as_deref(), Some("a"));
    assert_eq!(conn.api_key, "k-a");

    // ⑥ 明确要了不存在的那份 ⇒ 400 说清楚（别静默回退到别的 key 上）
    let e = resolve_llm_conn(&cfg(), Some("nope"), None, None, true).expect_err("必须报错");
    let msg = e.1 .0["error"].as_str().unwrap_or("").to_string();
    assert!(msg.contains("nope"), "{msg}");

    // ⑦ 读能力那条路**不需要 key**（catalog 只查表）
    let conn = resolve_llm_conn(&cfg(), Some("b"), None, None, false).expect("catalog 不需要 key");
    assert_eq!(conn.base_url, "https://b.example/v1");
    assert_eq!(conn.api_key, "", "没 key 就是空串，但不报错");
}

/// **写盘前守卫**（P0 事故 2026-10-19）：`json_mode = ""` 这种"合法 TOML 但反序列化不过"的内容
/// **不许落盘**，而且要回 400 + 人话；**原文件一个字节都不许动**（对比写前后字节）。
///
/// 事故现场：保存路径把字符串写进了布尔字段 ⇒ 整份 LLM 配置解析失败 ⇒ **回退默认值**
/// （用户看到的"界面更糟了"）。守卫必须用 `LlmConfig` 那层反序列化，`toml::Value` 挡不住。
#[test]
fn broken_patch_is_rejected_and_the_file_is_untouched() {
    let dir = std::env::temp_dir().join(format!(
        "vt-llm-guard-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("llm.toml");
    // 夹具里的 key 一律 "k"（**不许把真 key 写进仓库/输出**）
    // v4 形状：连接面在 `[[llm.providers]]`，参数面在 `[llm.defaults]`
    let original = "[llm]\n\n[[llm.providers]]\nid = \"default\"\nbase_url = \"https://api.example.com/v1\"\napi_key = \"k\"\nmodel = \"m\"\n";
    std::fs::write(&path, original).unwrap();

    let bad = serde_json::json!({ "patch": { "llm": { "defaults": { "json_mode": "" } } } });
    let e = config_llm_put_locked(&path, &bad, true).expect_err("这种 patch 必须被拦下");
    assert_eq!(e.0, StatusCode::BAD_REQUEST, "要回 400（不是 200、也不是 500）");
    let msg = e.1 .0["error"].as_str().unwrap_or("").to_string();
    assert!(msg.contains("json_mode"), "要点名哪个字段：{msg}");
    assert!(msg.contains("boolean"), "要说期望什么类型：{msg}");
    assert!(msg.contains("没有保存"), "要明说没保存：{msg}");
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        original,
        "**原文件一个字节都不许动**（先写临时文件、验过才 rename）"
    );
    assert!(
        !path.with_extension("toml.guard").exists(),
        "临时文件要清掉（别在配置目录里留垃圾）"
    );

    // 合法 patch ⇒ 200，且文件真的能反序列化
    let _ = config_llm_put_locked(
        &path,
        &serde_json::json!({ "patch": { "llm": { "providers": [
            { "id": "default", "base_url": "https://api.example.com/v1", "model": "m2" }
        ] } } }),
        true,
    )
    .expect("合法 patch 必须能存");
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.contains("m2"), "改动生效：\n{text}");
    assert!(
        yekitrim_core::config::validate_llm_toml(&text).is_ok(),
        "存完的文件必须解析得动：\n{text}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// **解析失败的文案必须脱敏**（安全）：TOML 的报错会带出错那一行的原文 ——
/// 一行写全的 `params = { … api_key = "sk-…" }` 就是明文泄漏（用户已经贴过一次日志）。
#[test]
fn parse_error_message_does_not_leak_the_api_key() {
    // 夹具：key 故意写在**出错那一行**上（事故现场的形状）
    let bad = "[llm]\ndefaults = { json_mode = \"\", api_key = \"sk-LEAK-FIXTURE\" }\n";
    let e = yekitrim_core::config::validate_llm_toml(bad).expect_err("必须解析不过");
    assert!(!e.contains("sk-LEAK-FIXTURE"), "错误文案里不许有明文 key：{e}");
    assert!(!e.contains("sk-"), "连前缀都不该出现：{e}");
    assert!(
        e.contains("json_mode") || e.contains("boolean"),
        "但判据要留着（哪个字段 / 期望什么类型）：{e}"
    );
}

/// 界面链路清单：**有 title、没有 correct**（用户口径：字幕订正删掉、加标题生成）。
/// 这一条钉的是"后端给清单"这件事本身 —— 前端不许自己写死四个。
#[test]
fn override_links_hide_correct_and_show_title() {
    let keys = override_link_keys();
    assert_eq!(keys, vec!["detect", "polish", "title", "utility"], "{keys:?}");
    assert!(keys.contains(&"title"), "标题生成要在界面上");
    assert!(!keys.contains(&"correct"), "字幕订正只从界面隐藏（代码与 TOML 里还在）：{keys:?}");
    // 但它在结构里**仍然存在**（asr/correct.rs 还在用）
    assert_eq!(yekitrim_core::config::LlmLink::Correct.key(), "correct");
}

/// **inline table 里的 key 也要抹**（多 provider 的 patch 会把 providers 写成一行一个 inline table）：
/// 老实现只认行首的 api_key，明文会从 llm_raw 原样漏出去 —— 而且看起来一切正常。
#[test]
fn redact_api_key_covers_inline_tables_and_every_provider() {
    let raw = "[llm]\nactive = \"a\"\nproviders = [{ id = \"a\", api_key = \"sk-a\" }, { id = \"b\", api_key = \"sk-b\" }]\n";
    let out = redact_api_key(raw);
    assert!(!out.contains("sk-a") && !out.contains("sk-b"), "inline 里的两条都要抹：\n{out}");
    assert!(out.contains("id = \"a\"") && out.contains("id = \"b\""), "别的键留着：\n{out}");
    assert_eq!(out.lines().count(), raw.lines().count(), "行数不变（编辑器靠行号对齐）");

    // 经典形态：每条 provider 一行 api_key（逐条抹，不是只抹第一条）
    let block = "[llm]\n\n[[llm.providers]]\nid = \"a\"\napi_key = \"sk-a\"\n\n[[llm.providers]]\nid = \"b\"\napi_key = \"sk-b\"\n";
    let out2 = redact_api_key(block);
    assert!(!out2.contains("sk-a") && !out2.contains("sk-b"), "{out2}");
    assert_eq!(out2.matches("api_key = \"\"").count(), 2, "{out2}");
}

/// `drop_blank_api_key` 的 provider 版：空串 = **那条**不修改（逐条判，不是只看顶层）。
#[test]
fn blank_api_key_is_dropped_for_every_provider() {
    let patch = serde_json::json!({ "llm": { "providers": [
        { "id": "a", "api_key": "  " },
        { "id": "b", "api_key": "sk-b" },
        { "id": "c" }
    ] } });
    let out = drop_blank_api_key(&patch);
    assert!(out["llm"]["providers"][0].get("api_key").is_none(), "空串要剔掉（= 不修改）：{out}");
    assert_eq!(out["llm"]["providers"][1]["api_key"], "sk-b", "真值照写：{out}");
    assert!(out["llm"]["providers"][2].get("api_key").is_none(), "本来就没有就保持没有：{out}");
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
    // v4：key 在 `[[llm.providers]]` 里（每份一把，按 id 认人）
    std::fs::write(
        &p,
        "[llm]\nactive = \"a\"\n\n[[llm.providers]]\nid = \"a\"\nbase_url = \"https://api.example.com/v1\"\napi_key = \"sk-real\"\nmodel = \"m1\"\n",
    )
    .unwrap();

    // ① 抹过值的原文原样存回：真 key 必须还在，别的改动照常落地
    let masked = "[llm]\nactive = \"a\"\n\n[[llm.providers]]\nid = \"a\"\nbase_url = \"https://api.example.com/v1\"\napi_key = \"\"\nmodel = \"m2\"\n";
    let _ = config_llm_put_locked(&p, &serde_json::json!({ "raw": masked }), true).unwrap();
    let after = std::fs::read_to_string(&p).unwrap();
    assert!(after.contains("api_key = \"sk-real\""), "空 api_key 不该覆盖真 key：\n{after}");
    assert!(after.contains("model = \"m2\""), "别的键要真的写进去：\n{after}");

    // ② 用户真写了新 key：照常换掉（"改 key"这条路不能被保护逻辑挡住）
    let _ = config_llm_put_locked(
        &p,
        &serde_json::json!({ "raw": "[llm]\nactive = \"a\"\n\n[[llm.providers]]\nid = \"a\"\nbase_url = \"https://api.example.com/v1\"\napi_key = \"sk-new\"\nmodel = \"m2\"\n" }),
        true,
    )
    .unwrap();
    let after = std::fs::read_to_string(&p).unwrap();
    assert!(after.contains("api_key = \"sk-new\"") && !after.contains("sk-real"), "\n{after}");

    // ③ 源文件里连那一行都没有：**按 id 逐份**补回来（别让"删一行"等于丢 key）
    let _ = config_llm_put_locked(
        &p,
        &serde_json::json!({ "raw": "[llm]\nactive = \"a\"\n\n[[llm.providers]]\nid = \"a\"\nbase_url = \"https://api.example.com/v1\"\nmodel = \"m3\"\n" }),
        true,
    )
    .unwrap();
    let after = std::fs::read_to_string(&p).unwrap();
    assert!(after.contains("api_key = \"sk-new\"") && after.contains("model = \"m3\""), "\n{after}");

    // ④ 原本就没 key（一份接口都没有）：不会被补出一个空的 api_key 行
    let q = dir.join("empty.toml");
    std::fs::write(&q, "[llm]\nmax_concurrent = 3\n").unwrap();
    let _ = config_llm_put_locked(&q, &serde_json::json!({ "raw": "[llm]\nmax_concurrent = 5\n" }), true).unwrap();
    let after = std::fs::read_to_string(&q).unwrap();
    assert!(!after.contains("api_key"), "没配过 key 就别凭空写一行：\n{after}");

    let _ = std::fs::remove_dir_all(&dir);
}

/// 真探之前的两道检查（纯函数：本机真程序目录里通常已有 key，HTTP 级钉不住这条分支）。
#[test]
fn 探之前先要_key_与_base_url() {
    use yekitrim_core::config::LlmConfigV4;

    // v4：地址与 key 都在那一份接口里 —— 没填 key 就是"没配"
    let no_key = LlmConfigV4::single("https://stored.example/v1", "m", "");
    let e = probe_request("https://asked.example/v1", None, &no_key).unwrap_err();
    assert_eq!(e.0, StatusCode::BAD_REQUEST, "没配 key 就别发一个必然 401 的请求");
    let msg = e.1 .0["error"].as_str().unwrap_or("").to_string();
    assert!(msg.contains("API key"), "{msg}");

    // ⚠️ 表单里刚填、还没保存的 key 也算数（否则用户必须先保存一次才能「获取模型」）
    assert_eq!(
        probe_request("https://asked.example/v1", Some("sk-draft"), &no_key).unwrap(),
        "https://asked.example/v1"
    );

    let llm = LlmConfigV4::single("https://stored.example/v1", "m", "sk-x");
    assert_eq!(probe_request("https://asked.example/v1", None, &llm).unwrap(), "https://asked.example/v1");
    assert_eq!(probe_request("  ", None, &llm).unwrap(), "https://stored.example/v1", "缺省用已存的那条");
    let no_base = LlmConfigV4::single("", "m", "sk-x");
    assert_eq!(probe_request("", None, &no_base).unwrap_err().0, StatusCode::BAD_REQUEST, "两个都空就没得探");
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

/// B8 判据本体：主机分类逐条钉死（这些就是"绕过 v4 检查"的常见写法）。
#[test]
fn base_url_local_host_classification() {
    // 本机 / 内网 / 链路本地 —— 全部要拦
    for bad in [
        "http://127.0.0.1:11434/v1",
        "http://localhost:11434",
        "http://LOCALHOST:8080/v1",
        "http://[::1]:11434/v1",
        "http://0.0.0.0/v1",
        "http://10.1.2.3/v1",
        "http://172.16.0.9:8000/v1",
        "http://172.31.255.255/v1",
        "http://192.168.1.10:30000/v1",
        "http://169.254.169.254/latest/meta-data",
        "http://[fd00::1]:11434/v1",
        "http://[fe80::1]:11434/v1",
        "http://[::ffff:10.0.0.5]:11434/v1",
        "http://sub.localhost/v1",
        "http://user@192.168.0.1/v1",
    ] {
        assert!(
            base_url_host_is_local(bad),
            "{bad} 必须判成内网/本机"
        );
        assert!(
            assert_base_url_allowed(bad, false).is_err(),
            "普通账号保存 {bad} 必须 403/400"
        );
    }
    // 公网地址 —— 放行
    for good in [
        "https://api.deepseek.com/v1",
        "https://api.openai.com/v1",
        "http://8.8.8.8/v1",
        "https://llm.example.com/v1",
        "http://[2001:db8::1]/v1",
    ] {
        assert!(!base_url_host_is_local(good), "{good} 不该被判成内网");
        assert!(assert_base_url_allowed(good, false).is_ok(), "{good} 普通账号应放行");
    }
    // admin 不受限：内网地址照样过（公网部署的运维要能配内网中转）
    assert!(assert_base_url_allowed("http://127.0.0.1:11434/v1", true).is_ok());
    // 空值放行（必填校验是各入口自己的事）
    assert!(assert_base_url_allowed("", false).is_ok());
    assert!(assert_base_url_allowed("   ", false).is_ok());
    // 403 的语义要稳：别悄悄变成 500/200
    let e = assert_base_url_allowed("http://127.0.0.1:11434/v1", false).unwrap_err();
    assert_eq!(e.0, StatusCode::FORBIDDEN, "{e:?}");
}

/// B8 @ `config_llm_put_locked`：普通账号**保存**带内网 base_url 的配置必须被拦，
/// 且**原文件一个字节不动**（守卫语义与 B3/P0 那两条一致）。
#[test]
fn llm_put_rejects_private_base_url_for_normal_user() {
    let dir = std::env::temp_dir().join(format!("yekitrim-b8-put-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let p = dir.join("llm.toml");
    // v4：地址只在 `[[llm.providers]]` 里（顶层 `base_url` 这个字段已经不存在了）
    let original = "[llm]\nactive = \"a\"\n\n[[llm.providers]]\nid = \"a\"\nname = \"甲\"\nbase_url = \"https://api.example.com/v1\"\napi_key = \"k\"\nmodel = \"m\"\n";
    std::fs::write(&p, original).unwrap();

    // ① patch 里**多份接口之一**带内网地址：数组整体替换也要逐份判 → 403，原文件不动、临时文件清掉
    let e = config_llm_put_locked(
        &p,
        &serde_json::json!({ "patch": { "llm": { "providers": [
            { "id": "a", "name": "甲", "protocol": "openai-chat",
              "base_url": "https://api.example.com/v1", "api_key": "k", "model": "m" },
            { "id": "b", "name": "乙", "protocol": "openai-chat",
              "base_url": "http://127.0.0.1:11434/v1", "model": "n" }
        ] } } }),
        false,
    )
    .expect_err("只要有一份指向本机，整份 patch 就要被拦");
    assert_eq!(e.0, StatusCode::FORBIDDEN, "{e:?}");
    assert_eq!(std::fs::read_to_string(&p).unwrap(), original, "原文件一个字节不动");
    assert!(!p.with_extension("toml.guard").exists(), "失败不留临时文件");

    // ② patch 里的 provider 带内网地址：同样 403（第一条就是内网也要拦）
    let e = config_llm_put_locked(
        &p,
        &serde_json::json!({ "patch": { "llm": { "providers": [
            { "id": "a", "name": "甲", "api": "openai-chat", "base_url": "http://192.168.1.5:8000/v1" }
        ] } } }),
        false,
    )
    .expect_err("provider 里的内网地址同样要拦");
    assert_eq!(e.0, StatusCode::FORBIDDEN, "{e:?}");
    assert_eq!(std::fs::read_to_string(&p).unwrap(), original);

    // ③ raw 源文件模式：同样 403
    let e = config_llm_put_locked(
        &p,
        &serde_json::json!({ "raw": "[llm]\nactive = \"a\"\n\n[[llm.providers]]\nid = \"a\"\nbase_url = \"http://169.254.169.254/\"\napi_key = \"k\"\nmodel = \"m\"\n" }),
        false,
    )
    .expect_err("raw 模式同样要拦");
    assert_eq!(e.0, StatusCode::FORBIDDEN, "{e:?}");
    assert_eq!(std::fs::read_to_string(&p).unwrap(), original);

    // ④ 同样的 patch，admin 放行且真的落盘
    let _ = config_llm_put_locked(
        &p,
        &serde_json::json!({ "patch": { "llm": { "providers": [
            { "id": "a", "name": "甲", "protocol": "openai-chat",
              "base_url": "http://127.0.0.1:11434/v1", "api_key": "k", "model": "m" }
        ] } } }),
        true,
    )
    .expect("admin 可以配内网中转");
    assert!(
        std::fs::read_to_string(&p).unwrap().contains("127.0.0.1"),
        "admin 的改动要真的生效"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// B8 @ `POST /api/llm/test`：普通账号用**已存的**内网配置点「测试连接」⇒ 403，
/// 且**不发出任何网络请求**（判据在 resolve 之后、test_llm 之前）。
///
/// 没有第二账号在 HTTP 层实测过"admin 能过"：那要真连一台本机服务才能证明成功，
/// 夹具环境做不到 —— admin 分支由 [`base_url_local_host_classification`] 在纯函数层钉住。
#[tokio::test]
async fn llm_test_rejects_private_base_url_for_normal_user() {
    let t = test_app().await;
    let hash = yekitrim_service::auth::hash_password("pass-12345678").unwrap();
    let uid = user_at_uid(&t, 5, "b8_user", &hash);
    assert_eq!(uid, 5, "b8_user 固定占 uid 5（见 user_at_uid 的分片表）");
    let b = t.auth.login("b8_user", "pass-12345678", None, None).unwrap();

    // 直接把"他自己的" llm.toml 写进他账号树的**真实解析位置**（`load_llm_config_for` 走
    // `owner::state_dir`，测试进程里解析到 exe 旁的程序目录 —— 绕开 PUT：保存那条已被拦）。
    // ⚠️ 用例结束必须清掉：这是构建产物目录里的账号树，不能留垃圾。
    let home = yekitrim_core::owner::state_dir(yekitrim_core::owner::Owner::User(uid));
    std::fs::create_dir_all(&home).unwrap();
    std::fs::write(
        home.join("llm.toml"),
        "[llm]\nactive = \"a\"\n\n[[llm.providers]]\nid = \"a\"\nbase_url = \"http://127.0.0.1:9/v1\"\napi_key = \"k\"\nmodel = \"m\"\n",
    )
    .unwrap();

    let res = t
        .call(t.req_as(&b.session_id, &b.csrf, "POST", "/llm/test").body(Body::empty()).unwrap())
        .await;
    assert_eq!(res.status(), StatusCode::FORBIDDEN, "已存的内网地址也不许测：{res:?}");

    // 公网地址的正常账号：判据放行（连不上是 500/502 的事，不是 403 —— 不发真请求没法验成功，
    // 这里只验"不是 403"，防止判据把公网用户一起拦掉）
    std::fs::write(
        home.join("llm.toml"),
        "[llm]\nactive = \"a\"\n\n[[llm.providers]]\nid = \"a\"\nbase_url = \"https://b8.invalid.example/v1\"\napi_key = \"k\"\nmodel = \"m\"\n",
    )
    .unwrap();
    let res = t
        .call(t.req_as(&b.session_id, &b.csrf, "POST", "/llm/test").body(Body::empty()).unwrap())
        .await;
    assert_ne!(res.status(), StatusCode::FORBIDDEN, "公网地址不该被 403：{res:?}");

    wipe_state_dir(uid);
}
}
