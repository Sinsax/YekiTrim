//! 采集配置的**读视图**（永不回明文口令 / token）、保存通道（`patch_config_file` 深合并）、令牌轮换。
//!
//! ⚠️ 视图只回 `api_pass_set` 布尔，绝不回 `api_pass` / `token_hash` —— 要看原文走 `GET /api/config`
//! 的 `global_raw`（admin 专用）。审计只记**键名**（值里可能有口令）。

use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;

use crate::auth::CurrentUser;
use crate::auth::ClientIp;
use crate::model::AppState;

use super::*;
use super::patch::build_ingest_patch;

/// 配置概览的 JSON（`GET /api/admin/ingest` 与两个写接口的响应共用）。
///
/// ⚠️ **`api_pass` 只回"是否已设置"，`token_hash` 一个字都不回** —— 表单回填不需要明文，
/// 而把口令塞进表单字段的后果是"用户没改也会被原样写回"。要看原文走 `GET /api/config`
/// 的 `global_raw`（admin 专用，也是源码编辑器 round-trip 的前提）。
pub(super) fn config_view_json() -> serde_json::Value {
    let cfg = yekitrim_core::config::load_global_config();
    serde_json::json!({
        "ok": true,
        // 设置页「通用 Webhook」地址用：Docker/另一台机器不能用 127.0.0.1
        "lan_ip": local_lan_ip(),
        "proxy": cfg.ingest.proxy,
        "webhook_enabled": cfg.ingest.webhook_enabled,
        "token_configured": !cfg.ingest.token_hash.trim().is_empty(),
        "allow_ips": cfg.ingest.allow_ips,
        "bililive_tools": {
            "api_base": cfg.ingest.recorder.api_base,
            "api_key_set": !cfg.ingest.recorder.api_key.trim().is_empty(),
            "root": cfg.ingest.recorder.root,
            "container_root": cfg.ingest.recorder.container_root,
        },
        "recorder": {
            "kind": cfg.ingest.recorder.kind,
            "root": cfg.ingest.recorder.root,
            "api_base": cfg.ingest.recorder.api_base,
            "api_user": cfg.ingest.recorder.api_user,
            "api_pass_set": !cfg.ingest.recorder.api_pass.is_empty(),
            "api_key_set": !cfg.ingest.recorder.api_key.trim().is_empty(),
            "container_root": cfg.ingest.recorder.container_root,
            "watch_interval_mins": cfg.ingest.recorder.watch_interval_mins,
            "finalize_idle_mins": cfg.ingest.recorder.finalize_idle_mins,
            "move_into_managed": cfg.ingest.recorder.move_into_managed,
            "concat_files": cfg.ingest.recorder.concat_files,
            "extensions": cfg.ingest.recorder.extensions,
            "auto_pipeline": cfg.ingest.recorder.auto_pipeline,
        },
        // 通用 webhook（任意录制软件）：与 [ingest] 共用 token / allow_ips。
        "webhook": {
            "root": cfg.ingest.webhook.root,
            "container_root": cfg.ingest.webhook.container_root,
            "auto_pipeline": cfg.ingest.webhook.auto_pipeline,
            "path": "/api/ingest/webhook/<token>",
            "enabled": cfg.ingest.webhook_enabled
                && !cfg.ingest.token_hash.trim().is_empty()
                && !cfg.ingest.webhook.root.trim().is_empty(),
        },
        // B站下载通道（P4）。前端表单只要这几项；`cookie_file` 是**路径**，
        // 但它是本机配置项（不是会话 cookie 内容），且这个接口本身仅 admin 可读。
        //
        // ⚠️ 分组名叫 `download` 而**不是** `downloader`：`[ingest] downloader` 是一个
        // **字符串键**（引擎名），把它当成子表分组名会让读代码的人以为配置里有个
        // `[ingest.downloader]` 子表 —— 那正是 `build_ingest_patch` 要小心的坑。
        "download": {
            "engine": cfg.ingest.downloader,
            "ytdlp_path": cfg.ingest.ytdlp_path,
            "max_quality": cfg.ingest.max_quality,
            "rate_limit_kbps": cfg.ingest.rate_limit_kbps,
            "max_concurrent_downloads": cfg.ingest.max_concurrent_downloads,
            "cookie_file": cfg.ingest.cookie_file,
            "idle_timeout_secs": cfg.ingest.download_idle_timeout_secs,
        },
        // 前端拿 `location.origin` 拼完整地址；录播姬不在本机时把主机名换成 YekiTrim 的局域网 IP
        "webhook_path": "/api/ingest/bililive/<token>",
        // 项目存放位置（`[storage] projects_root`）。**这是"下载/入库落到哪"的唯一开关**，
        // 与 B站通道无关（录播姬入库、本机导入同一个根），但设置页把它放在下载器卡里 ——
        // 用户问"下载到哪"时先在那一块找。
        //
        // `projects_root` 是配置原值（空 = 用默认），`projects_root_effective` 是**解析后**
        // 的真实落点：只回值不回结构，前端照原样展示即可（别让它自己拼默认路径）。
        "storage": {
            "projects_root": cfg.storage.projects_root,
            "projects_root_effective": yekitrim_core::path::display(
                &yekitrim_service::projects::projects_root(),
            ),
        },
    })
}

/// 本机默认出口 IP（给设置页拼「局域网 Webhook 地址」用）。
///
/// UDP `connect` 只选路由、不发包；不依赖外部服务。无默认路由/只有回环时返回 None，
/// 前端会退回 `location.origin` 并提示用户手填。
fn local_lan_ip() -> Option<String> {
    let sock = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    sock.connect("8.8.8.8:80").ok()?;
    let ip = sock.local_addr().ok()?.ip();
    if ip.is_loopback() || ip.is_unspecified() {
        None
    } else {
        Some(ip.to_string())
    }
}

/// `GET /api/admin/ingest`：录播姬配置概览（不含 token 明文，只报是否已配置）。
pub async fn config_view() -> ApiResult {
    ok(config_view_json())
}

/// 校验 `[storage] projects_root`，返回**要写进配置的值**（`""` = 回默认）。
///
/// 与 `recorder.root` 那条"不可达只 warning"不同 —— 那个目录是**别人写的**（录播姬），
/// 我们只能读、不能替它建；而这是**我们自己要往里写**的根，所以在这里就把它建出来：
/// 等到下载跑完才发现目录建不了，用户已经白等了。
///
/// 判定顺序（先便宜后昂贵）：
/// 1. 空 → 回默认（程序目录下的 `.yekitrim/projects`）；
/// 2. 必须是**绝对路径** —— 相对路径的基准是进程 cwd，服务重启后可能换目录，落点会漂；
/// 3. 已存在但不是目录 → 拒（填成了某个文件）；
/// 4. `create_dir_all` + 真实写一个探针文件再删（`disk::writable`，与 `yekitrim doctor` 同一套判据）
///    → 建不出来或写不进去就拒。
pub(super) fn check_projects_root(raw: &str) -> Result<String, AppError> {
    let s = raw.trim();
    if s.is_empty() {
        return Ok(String::new());
    }
    let p = std::path::Path::new(s);
    if !p.is_absolute() {
        return Err(err(
            StatusCode::BAD_REQUEST,
            format!("项目存放位置必须是绝对路径（例如 F:/yekitrim-projects，不要用相对路径）：{s}"),
        ));
    }
    if p.exists() && !p.is_dir() {
        return Err(err(
            StatusCode::BAD_REQUEST,
            format!("项目存放位置指向的是一个文件，不是目录：{}", yekitrim_core::path::display(p)),
        ));
    }
    if !yekitrim_service::disk::writable(p) {
        return Err(err(
            StatusCode::BAD_REQUEST,
            format!(
                "项目存放位置不可用（建不出目录或写不进去）：{} —— 换个存在的盘符或有写权限的目录",
                yekitrim_core::path::display(p)
            ),
        ));
    }
    Ok(yekitrim_core::path::display(p))
}

/// `PUT /api/admin/ingest`：保存采集/录播姬配置（仅 admin）。
///
/// 校验与语义见 [`build_ingest_patch`]；写入走 `patch_config_file`（toml_edit 合并）→
/// 只动这几个键，其余 section、键顺序、**注释**全部原样保留
/// （`[ingest.recorder]` 是子表，靠的就是它现在支持递归合并）。
pub async fn put_config(
    State(state): State<Arc<AppState>>,
    axum::extract::Extension(user): axum::extract::Extension<CurrentUser>,
    ClientIp(ip): ClientIp,
    Json(body): Json<serde_json::Value>,
) -> ApiResult {
    let (ingest, storage, warnings) = build_ingest_patch(&body)?;

    if ingest.is_empty() && storage.is_empty() {
        return ok(serde_json::json!({ "ok": true, "warnings": warnings, "changed": [] }));
    }

    // 审计只记**键名**：把 api_pass 的值写进 audit 表等于把口令抄一份到数据库里
    let changed = {
        let mut keys = audit_keys("ingest", &ingest);
        keys.extend(audit_keys("storage", &storage));
        keys.join(", ")
    };
    write_global_config(ingest, storage).await?;
    let _ = state
        .auth
        .db()
        .audit_ip(Some(user.id), "ingest_config_save", &changed, ip.as_deref());

    let mut out = config_view_json();
    out["warnings"] = serde_json::json!(warnings);
    out["changed"] = serde_json::json!(changed);
    ok(out)
}

/// 组装要合并进全局配置的补丁：`{ section: { key: value } }`。
///
/// **空的 section 不写进去** —— 否则一次"只改下载参数"的保存会凭空造出一个空的 `[storage]`。
pub(super) fn config_patch(
    ingest: serde_json::Map<String, serde_json::Value>,
    storage: serde_json::Map<String, serde_json::Value>,
) -> serde_json::Value {
    let mut patch = serde_json::Map::new();
    if !ingest.is_empty() {
        patch.insert("ingest".into(), serde_json::Value::Object(ingest));
    }
    if !storage.is_empty() {
        patch.insert("storage".into(), serde_json::Value::Object(storage));
    }
    serde_json::Value::Object(patch)
}

/// 把 `[ingest]` / `[storage]` 的字段合并进**程序配置**（`config::global_config_path()`）。
///
/// ⚠️ **与运行时读的那份必须是同一个文件**（2026-11-13）：以前这里写顶层
/// `<数据>/.yekitrim/config.toml`，而设置页写 `users/<uid>/…` —— 两份长得一模一样，
/// 一半设置保存成功却不生效。现在两侧都走 `config::global_config_path()`。
async fn write_global_config(
    ingest: serde_json::Map<String, serde_json::Value>,
    storage: serde_json::Map<String, serde_json::Value>,
) -> Result<(), AppError> {
    write_global_config_at(
        &yekitrim_core::config::global_config_path(),
        config_patch(ingest, storage),
    )
    .await
}

/// [`write_global_config`] 的带路径版本（单测直接打临时文件，不碰真实全局配置）。
///
/// 与 `api.rs::config_global_put` 同一套：读改写会与设置页保存、CLI 并发，
/// 锁在 await 里持有，真正写盘那段搬到 blocking 线程。
pub(super) async fn write_global_config_at(
    path: &std::path::Path,
    patch: serde_json::Value,
) -> Result<(), AppError> {
    let _guard = yekitrim_core::filelock::lock_path(path).await;
    let p = path.to_path_buf();
    let res = tokio::task::spawn_blocking(move || yekitrim_core::config::patch_config_file(&p, &patch))
        .await
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("保存线程失败: {e}")))?;
    res.map_err(|e| err(StatusCode::BAD_REQUEST, format!("保存配置失败: {e:#}")))
}

/// 这次改了哪些键（用于审计与响应里的 `changed`）。只列键名（带 section 前缀），不带值。
pub(super) fn audit_keys(section: &str, map: &serde_json::Map<String, serde_json::Value>) -> Vec<String> {
    let mut keys: Vec<String> = Vec::new();
    for (k, v) in map {
        if let Some(sub) = v.as_object() {
            for sk in sub.keys() {
                keys.push(format!("{section}.{k}.{sk}"));
            }
        } else {
            keys.push(format!("{section}.{k}"));
        }
    }
    keys
}

/// `POST /api/admin/ingest/token {action:"generate"|"disable"}`：生成/轮换/关闭 webhook token。
///
/// 明文**只在这一个响应里回一次**（与 CLI `yekitrim ingest token` 同一语义），
/// 配置里只留 scrypt 哈希；`token_hash` 永远不回给前端。
pub async fn rotate_token(
    State(state): State<Arc<AppState>>,
    axum::extract::Extension(user): axum::extract::Extension<CurrentUser>,
    ClientIp(ip): ClientIp,
    Json(body): Json<serde_json::Value>,
) -> ApiResult {
    let action = body.get("action").and_then(|v| v.as_str()).unwrap_or("generate");
    let mut ingest = serde_json::Map::new();
    let mut plaintext: Option<String> = None;
    match action {
        // 关闭：只关开关，**保留** token（下次开回来还是同一个，除非重新生成）
        "disable" => {
            ingest.insert("webhook_enabled".into(), serde_json::json!(false));
        }
        "generate" => {
            let token = yekitrim_service::ingest::generate_token();
            // scrypt（N=2^15）是 CPU 密集：别占住 tokio worker
            let t = token.clone();
            let hash = tokio::task::spawn_blocking(move || yekitrim_service::auth::hash_password(&t))
                .await
                .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("生成线程失败: {e}")))?
                .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("生成 token 失败: {e:#}")))?;
            ingest.insert("token_hash".into(), serde_json::json!(hash));
            ingest.insert("webhook_enabled".into(), serde_json::json!(true));
            plaintext = Some(token);
        }
        other => {
            return Err(err(
                StatusCode::BAD_REQUEST,
                format!("未知 action: {other}（可用 generate / disable）"),
            ))
        }
    }

    write_global_config(ingest, serde_json::Map::new()).await?;
    // 审计**不记 token 明文**
    let detail = if plaintext.is_some() { "generated" } else { "disabled" };
    let _ = state
        .auth
        .db()
        .audit_ip(Some(user.id), "ingest_token", detail, ip.as_deref());

    let mut out = config_view_json();
    if let Some(t) = plaintext {
        // 只有这一条响应里有明文；前端要提示用户"只显示这一次"
        out["token"] = serde_json::json!(t);
    }
    ok(out)
}

// ---------------------------------------------------------------------------
// B站链接通道（P4）
// ---------------------------------------------------------------------------
//
// 这一节与上面的录播姬 webhook 是**两个来源**，但共用同一张 `ingest_sessions` 表
// （靠 `kind` 列区分）。放在同一个模块是因为它们是同一件事的两半：把外部素材接进来。
//
// 分工：
// - 本模块 = **HTTP 入口**（校验、预检、建会话、唤醒 worker）；
// - `crate::download_worker` = **执行**（spawn 下载器、落弹幕、建项目）。
//   两者之间只有 `ingest_sessions` 一行数据，不共享任何函数 —— 这是刻意的（计划 §2）。

/// 存放位置门禁：`None` = 放行；`Some(msg)` = 拒绝理由（**按角色**给文案）。
///
/// B站下载是唯一"替用户往磁盘写大量数据"的入口，落点必须有人显式选过 ——
/// 没配置就静默落到程序目录旁的默认位置，用户根本不知道东西下到哪了。
/// 文案分角色：管理员给去处（设置页）；普通用户只说联系管理员
/// （不暴露管理端入口，与前文 `dl.ready === false` 那张卡的措辞同一套）。
pub(super) fn projects_root_gate(role: &str, projects_root: &str) -> Option<&'static str> {
    if !projects_root.trim().is_empty() {
        return None;
    }
    Some(if role == "admin" {
        "尚未配置项目存放位置：请在 设置 → 下载 里配置后重试"
    } else {
        "管理员尚未配置下载存放位置，暂时无法下载；请联系管理员"
    })
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::ingest::patch::build_ingest_patch;
    use crate::ingest::webhook::token_shape_ok;

    /// 存放位置门禁：未配置时**按角色**给不同文案；已配置（含看似空白）放行。
    #[test]
    fn projects_root_gate_by_role() {
        // 未配置：管理员给去处，普通用户不暴露管理端
        let a = projects_root_gate("admin", "").unwrap();
        assert!(a.contains("设置"), "{a}");
        let u = projects_root_gate("user", "").unwrap();
        assert!(u.contains("联系管理员") && !u.contains("设置"), "{u}");
        // 空白串（用户存了 " "）也算未配置
        assert!(projects_root_gate("admin", "   ").is_some());
        // 已配置：放行
        assert!(projects_root_gate("admin", "F:/media/yekitrim").is_none());
        assert!(projects_root_gate("user", "D:\\x").is_none());
    }
    /// 项目存放位置（`[storage] projects_root`）—— 下载与入库的**共同**落点。
    ///
    /// 三条边界各对应一个真实事故：相对路径的基准是进程 cwd，服务换个启动目录落点就漂；
    /// 指向文件会让 `create_dir_all` 等到**下载阶段**才炸（用户白等几分钟）；
    /// 盘符写错是最常见的手滑，现场建一下目录就能立刻发现。
    #[test]
    fn projects_root_is_validated_and_created() {
        // ① 键缺席 = 不动它（不许凭空造出一个空 [storage] 段）
        let (_, st, _) = build_ingest_patch(&serde_json::json!({"webhook_enabled": true})).unwrap();
        assert!(st.is_empty(), "没提 storage 就不该动它：{st:?}");

        // ② 空串 = 显式回默认（程序目录下的 .yekitrim/projects）
        let (_, st, _) =
            build_ingest_patch(&serde_json::json!({"storage": {"projects_root": ""}})).unwrap();
        assert_eq!(st["projects_root"], serde_json::json!(""));

        // ③ 相对路径：拒
        for bad in ["yekitrim-projects", "./out", "../out"] {
            let e = build_ingest_patch(&serde_json::json!({"storage": {"projects_root": bad}}))
                .unwrap_err();
            assert_eq!(e.0, StatusCode::BAD_REQUEST, "{bad}");
            let msg = serde_json::to_string(&e.1 .0).unwrap();
            assert!(msg.contains("绝对路径"), "{bad} → {msg}");
        }

        // ④ 指向一个已存在的文件：拒
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("not-a-dir.txt");
        std::fs::write(&file, b"x").unwrap();
        let e = build_ingest_patch(
            &serde_json::json!({"storage": {"projects_root": file.to_string_lossy()}}),
        )
        .unwrap_err();
        let msg = serde_json::to_string(&e.1 .0).unwrap();
        assert!(msg.contains("不是目录"), "{msg}");

        // ⑤ 全新目录：放行，且**保存时就真的建出来**（等下载跑完才发现建不了就太晚了）
        let fresh = dir.path().join("projects").join("yekitrim");
        let (_, st, _) = build_ingest_patch(
            &serde_json::json!({"storage": {"projects_root": fresh.to_string_lossy()}}),
        )
        .unwrap();
        assert_eq!(
            st["projects_root"].as_str().unwrap(),
            yekitrim_core::path::display(&fresh)
        );
        assert!(fresh.is_dir(), "保存时就该把目录建出来：{}", fresh.display());
    }
    /// 落盘：全局配置里**原本没有 `[storage]` 段**时要能把它建出来，且不碰其它 section。
    ///
    /// 这条路径是本次新开的（`patch_config_file` 只在 section 不是表时插入新表，
    /// 之前所有调用方写的都是已存在的 `[ingest]`），所以要有钉子。
    #[tokio::test]
    async fn save_projects_root_creates_missing_storage_section() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(
            &path,
            "\
# 顶部注释不能丢
[ingest]
webhook_enabled = true

[detection]
vtuber_name = \"主播\"
",
        )
        .unwrap();

        let target = dir.path().join("my-projects");
        let (_, storage, _) = build_ingest_patch(
            &serde_json::json!({"storage": {"projects_root": target.to_string_lossy()}}),
        )
        .unwrap();
        write_global_config_at(&path, config_patch(serde_json::Map::new(), storage))
            .await
            .unwrap();

        let out = std::fs::read_to_string(&path).unwrap();
        assert!(out.contains("[storage]"), "该段必须被建出来：\n{out}");
        // TOML 会把路径里的反斜杠转义成 `\\`，逐字符比对太脆 —— 归一化后核对即可
        let expected = yekitrim_core::path::display(&target);
        assert!(
            out.contains("projects_root =") && out.replace("\\\\", "\\").contains(&expected),
            "写入的路径不对（期望 {expected}）：\n{out}"
        );
        // 原有内容一字不动
        for keep in ["# 顶部注释不能丢", "webhook_enabled = true", "vtuber_name = \"主播\""] {
            assert!(out.contains(keep), "丢了 {keep}：\n{out}");
        }

        // 再存一次"空 = 回默认"：只改值，不把段清空、也不动别的 section
        let (_, storage, _) =
            build_ingest_patch(&serde_json::json!({"storage": {"projects_root": ""}})).unwrap();
        write_global_config_at(&path, config_patch(serde_json::Map::new(), storage))
            .await
            .unwrap();
        let out = std::fs::read_to_string(&path).unwrap();
        assert!(out.contains("projects_root = \"\""), "回默认应写成空串：\n{out}");
        assert!(out.contains("webhook_enabled = true"), "{out}");
    }
    /// 写盘：只动 `[ingest]` / `[ingest.recorder]` 命中的键，**同表兄弟键与注释一个不能少**。
    ///
    /// 回归钉子：`patch_config_file` 曾经只认一层，`recorder` 会被整项替换成 inline table，
    /// `root`/`api_user`/`api_pass`/`watch_interval_mins` 与段内注释静默消失
    /// （文件仍是合法 TOML，不报错，下次读配置才发现）。
    #[tokio::test]
    async fn save_ingest_config_keeps_siblings_and_comments() {
        let dir = std::env::temp_dir().join(format!("yekitrim-ingest-cfg-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.toml");
        std::fs::write(
            &path,
            "\
# 顶部注释不能丢
[ingest]
# webhook 开关
webhook_enabled = true
token_hash = \"scrypt$keepme\"

[ingest.recorder]
# 录像目录
root = \"D:/rec\"
api_base = \"http://127.0.0.1:2356\"
api_user = \"admin\"
api_pass = \"oldpass\"
watch_interval_mins = 7

[detection]
vtuber_name = \"主播\"
",
        )
        .unwrap();

        let (ingest, _, _) = build_ingest_patch(&serde_json::json!({
            "allow_ips": ["127.0.0.1"],
            "recorder": {"api_base": "http://10.0.0.5:2356", "auto_pipeline": "cut"}
        }))
        .unwrap();
        write_global_config_at(&path, config_patch(ingest, serde_json::Map::new())).await.unwrap();

        let out = std::fs::read_to_string(&path).unwrap();
        assert!(out.contains("api_base = \"http://10.0.0.5:2356\""), "{out}");
        assert!(out.contains("allow_ips = [\"127.0.0.1\"]"), "{out}");
        assert!(out.contains("auto_pipeline = \"cut\""), "{out}");
        // 没提到的兄弟键原样（尤其密码：表单留空不该把它清掉）
        for keep in [
            "root = \"D:/rec\"",
            "api_user = \"admin\"",
            "api_pass = \"oldpass\"",
            "watch_interval_mins = 7",
            "token_hash = \"scrypt$keepme\"",
        ] {
            assert!(out.contains(keep), "丢了 {keep}：\n{out}");
        }
        // 注释与其它 section 原样
        for keep in ["# 顶部注释不能丢", "# webhook 开关", "# 录像目录", "vtuber_name = \"主播\""] {
            assert!(out.contains(keep), "丢了 {keep}：\n{out}");
        }
        assert!(out.contains("[ingest.recorder]"), "子表应保持表头形式：\n{out}");
        assert!(!out.contains("recorder = {"), "不该退化成 inline table：\n{out}");

        // 显式给空串才清掉密码
        let (ingest, _, _) = build_ingest_patch(&serde_json::json!({"recorder": {"api_pass": ""}})).unwrap();
        write_global_config_at(&path, config_patch(ingest, serde_json::Map::new())).await.unwrap();
        let out = std::fs::read_to_string(&path).unwrap();
        assert!(out.contains("api_pass = \"\""), "显式清空应生效：\n{out}");
        assert!(out.contains("api_user = \"admin\""), "清密码不该连用户名一起动：\n{out}");

        let _ = std::fs::remove_dir_all(&dir);
    }
    /// 读接口**绝不**回 `token_hash` 或 `api_pass` 明文（表单要的是 `api_pass_set` 布尔）。
    #[test]
    fn config_view_never_leaks_secrets() {
        let text = serde_json::to_string(&config_view_json()).unwrap();
        assert!(!text.contains("token_hash"), "不该回 token_hash：{text}");
        let v = config_view_json();
        assert!(v["recorder"].get("api_pass").is_none(), "不该回 api_pass：{text}");
        // 该有的都在
        for k in ["webhook_enabled", "token_configured", "allow_ips", "webhook_path"] {
            assert!(v.get(k).is_some(), "缺字段 {k}");
        }
        for k in ["root", "api_base", "api_user", "api_pass_set", "auto_pipeline", "watch_interval_mins"] {
            assert!(v["recorder"].get(k).is_some(), "缺字段 recorder.{k}");
        }
        assert!(v["recorder"]["api_pass_set"].is_boolean());
        // B站下载分组（P4）：表单要的字段一个都不能少，否则设置页会显示成空
        for k in [
            "engine",
            "ytdlp_path",
            "max_quality",
            "rate_limit_kbps",
            "max_concurrent_downloads",
            "cookie_file",
            "idle_timeout_secs",
        ] {
            assert!(v["download"].get(k).is_some(), "缺字段 download.{k}");
        }
    }
    /// 生成的 token 能通过它自己的哈希校验（webhook 那条链路的唯一凭据）。
    #[test]
    fn generated_token_matches_its_hash() {
        let token = yekitrim_service::ingest::generate_token();
        assert!(token_shape_ok(&token), "生成器产物必须过形状检查：{token}");
        let hash = yekitrim_service::auth::hash_password(&token).unwrap();
        assert!(yekitrim_service::auth::verify_password(&hash, &token).unwrap_or(false));
        assert!(!yekitrim_service::auth::verify_password(&hash, "yekitrim_wrong").unwrap_or(false));
        assert!(hash.starts_with("scrypt$"), "配置里只该存哈希：{hash}");
    }
    /// 审计只记键名（绝不把口令值写进 audit 表）。
    #[test]
    fn audit_keys_are_names_only() {
        let (ingest, _, _) = build_ingest_patch(&serde_json::json!({
            "webhook_enabled": true,
            "recorder": {"api_pass": "supersecret", "api_base": "http://x:1"}
        }))
        .unwrap();
        let keys = audit_keys("ingest", &ingest);
        let s = keys.join(", ");
        assert!(s.contains("ingest.recorder.api_pass") && s.contains("ingest.recorder.api_base"), "{s}");
        assert!(s.contains("ingest.webhook_enabled"), "{s}");
        assert!(!s.contains("supersecret"), "审计不得带值：{s}");

        // `[storage]` 走同一个函数，但必须带自己的 section 前缀（否则审计里看不出改的是哪一段）
        let st = audit_keys("storage", &serde_json::json!({"projects_root": "F:/x"}).as_object().unwrap().clone());
        assert_eq!(st, vec!["storage.projects_root".to_string()]);
    }
}
