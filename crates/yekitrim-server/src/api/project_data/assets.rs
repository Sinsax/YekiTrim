//! `api::project_data::assets`：素材清单（`[project.materials]`）与转写稿的读写。

use std::path::{Path, PathBuf};

use axum::http::StatusCode;

use axum::Json;
use yekitrim_service::Workspace;

use crate::scope::Scope;

// 跨子模块 / 外部域：显式点名（不许 glob）
use axum::extract::Extension;

use super::*;

pub(crate) async fn project_data_transcript(scope: Scope) -> ApiResult {
    let ws = scope.ws;
    blocking(move || data_transcript_impl(&ws)).await
}

pub(crate) async fn project_data_transcript_put(scope: Scope, Json(body): Json<serde_json::Value>) -> ApiResult {
    let ws = scope.ws;
    let path = ws.output_dir().join("transcript.json");
    // 锁在 await 里持有，读改写本身下 blocking 池（`with_path_lock` 的闭包是同步的，会把
    // 整段 IO 留在 worker 上）
    let _guard = yekitrim_core::filelock::lock_path(&path).await;
    blocking(move || data_transcript_put_impl(&ws, &body)).await
}

/// `GET /api/project-data/assets`：当前弹幕/字幕 + **素材清单** + 项目内的候选 + ASR 槽位。
///
/// 候选一次扫完（项目根 + 一层子目录），预处理页打开弹窗即可直接用。
/// 只 stat、不解析文件内容（3 小时录播的弹幕 XML 有几十 MB），所以这里不需要担心耗时。
///
/// `slots` 是**每个 ASR 模式是否已经出过产物**：它决定转写入口置灰。
/// 前端不去自己从 `items` 推 —— 判槽还要看盘上的分模式产物（老工程没登记过），
/// 那是服务端的口径（`assets::slot_bound`），抄一份到前端就会漂。
pub(crate) async fn project_data_assets(scope: Scope) -> ApiResult {
    let ws = scope.ws;
    let source_kind = scope.project.source_kind.clone();
    let source_label = scope.project.source_label.clone().unwrap_or_default();
    let room = scope.project.room.clone();
    blocking(move || assets_impl(&ws, &source_kind, &source_label, room.as_deref())).await
}

/// `POST /api/project-data/assets`：改一类素材的清单 / 生效值。
///
/// - `{kind, path|dir+name}` → 选它（进清单 + 置为生效值）
/// - `{kind, path: ""}` → 清除生效值（清单不动，随时能切回来）
/// - `{kind, action: "remove", path}` → 从清单里删掉这条
///
/// 写前**解析一遍**（解析失败不落盘），成功返回条数供前端提示。
pub(crate) async fn project_data_assets_put(
    scope: Scope,
    Extension(user): Extension<crate::auth::CurrentUser>,
    Json(body): Json<serde_json::Value>,
) -> ApiResult {
    let ws = scope.ws;
    let is_admin = user.is_admin();
    // 与设置页、去静音等写同一份 yekitrim.toml：读改写必须上路径锁
    let path = ws.config_file.clone();
    let _guard = yekitrim_core::filelock::lock_path(&path).await;
    blocking(move || assets_put_impl(&ws, &body, is_admin)).await
}

pub(super) fn assets_impl(
    ws: &Workspace,
    source_kind: &str,
    source_label: &str,
    room: Option<&str>,
) -> ApiResult {
    use yekitrim_service::assets::{asset_status, scan_candidates, slot_bound, AssetKind};
    let cfg = ws.load_config().map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}")))?;
    let mut candidates = serde_json::Map::new();
    for kind in [AssetKind::Danmaku, AssetKind::Transcript] {
        let list = scan_candidates(ws, &cfg, kind);
        candidates.insert(
            kind.as_str().to_string(),
            serde_json::to_value(list).unwrap_or(serde_json::Value::Array(vec![])),
        );
    }
    let mut slots = serde_json::Map::new();
    for mode in yekitrim_core::config::ASR_MODES {
        slots.insert(
            mode.to_string(),
            serde_json::Value::Bool(slot_bound(ws, &cfg, AssetKind::Transcript, mode)),
        );
    }
    ok(serde_json::json!({
        "danmaku": asset_status(ws, &cfg, AssetKind::Danmaku),
        "transcript": asset_status(ws, &cfg, AssetKind::Transcript),
        "source": { "kind": source_kind, "label": source_label, "room": room },
        "candidates": candidates,
        "slots": slots,
        "root_dir": yekitrim_core::path::display(&ws.root_dir),
    }))
}

pub(super) fn assets_put_impl(ws: &Workspace, body: &serde_json::Value, is_admin: bool) -> ApiResult {
    use yekitrim_service::assets::{asset_status, remove_material, set_asset, AssetKind};
    let kind_raw = body.get("kind").and_then(|v| v.as_str()).unwrap_or("");
    let kind = AssetKind::parse(kind_raw)
        .map_err(|e| err(StatusCode::BAD_REQUEST, format!("{e:#}")))?;
    // 两种提交形态：`{path}`（项目内相对 / admin 的绝对路径），
    // 或 `{dir, name}`（服务器目录浏览选中的文件）—— 后者**由服务端拼路径**，
    // 因为回传的目录可能带 `\\?\` verbatim 前缀，前端拼 `\\?\a/b` 在 Windows 上是非法名。
    let path = match (
        body.get("dir").and_then(|v| v.as_str()),
        body.get("name").and_then(|v| v.as_str()),
    ) {
        (Some(dir), Some(name)) if !dir.trim().is_empty() && !name.trim().is_empty() => Path::new(dir.trim())
            .join(name.trim())
            .to_string_lossy()
            .into_owned(),
        _ => body.get("path").and_then(|v| v.as_str()).unwrap_or("").to_string(),
    };
    // 删清单条目：`remove_material` 自带"ASR 产物（文件还在）不给删"的守卫
    let removing = body
        .get("action")
        .and_then(|v| v.as_str())
        .map(|a| a.eq_ignore_ascii_case("remove"))
        .unwrap_or(false);
    if removing {
        remove_material(ws, kind, &path)
            .map_err(|e| err(StatusCode::BAD_REQUEST, format!("{e:#}")))?;
        let cfg = ws.load_config().map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}")))?;
        return ok(serde_json::json!({
            "ok": true,
            "kind": kind.as_str(),
            "removed": true,
            "value": "",
            "count": 0,
            "status": asset_status(ws, &cfg, kind),
        }));
    }
    let (value, count) = set_asset(ws, kind, &path, is_admin)
        .map_err(|e| err(StatusCode::BAD_REQUEST, format!("{e:#}")))?;
    let cfg = ws.load_config().map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}")))?;
    ok(serde_json::json!({
        "ok": true,
        "kind": kind.as_str(),
        "removed": false,
        "value": value,
        "count": count,
        "status": asset_status(ws, &cfg, kind),
    }))
}

pub(super) fn data_transcript_impl(ws: &Workspace) -> ApiResult {
    let cfg = ws.load_config().map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}")))?;
    // 与检测阶段同一口径（`Workspace::transcript_path`）：`transcript_file` → output/transcript.json
    // → output/video.srt。⚠️ 别改回裸 `PathBuf::from(...)`（那是相对进程 CWD，不是工程根）。
    let path = ws.transcript_path(&cfg);
    match path {
        Some(p) => {
            let raw = std::fs::read_to_string(&p).map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("读取失败: {e}")))?;
            let ext = p.extension().and_then(|e| e.to_str()).unwrap_or("");
            if ext.eq_ignore_ascii_case("json") {
                let v: serde_json::Value = serde_json::from_str(&raw).unwrap_or(serde_json::Value::Null);
                ok(v)
            } else {
                let t = yekitrim_core::srt::parse_srt(&raw).map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("解析失败: {e}")))?;
                ok(serde_json::to_value(t).unwrap_or_default())
            }
        }
        None => Err(err(StatusCode::NOT_FOUND, "暂无转写文件")),
    }
}

/// 两个路径是否指同一个文件（canonicalize 兜住 `./` 与大小写；文件还不存在时退化成字面比较）。
pub(super) fn same_path(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
}

/// ⚠️ `pub(crate)` 而不是 `pub(super)`：`api/test_support.rs` 的夹具（只测试编译）直接调它
/// 造转写稿，那个文件在 `project_data` 之外（T18 拆分后必须显式说明，别当成疏忽）。
pub(crate) fn data_transcript_put_impl(ws: &Workspace, body: &serde_json::Value) -> ApiResult {
    let cfg = ws.load_config().map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}")))?;
    let output = ws.output_dir();
    let configured = cfg
        .project
        .transcript_file
        .as_ref()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let segments = body.get("segments").cloned().ok_or_else(|| err(StatusCode::BAD_REQUEST, "缺少 segments"))?;
    let segs: Vec<yekitrim_core::types::TranscriptSegment> = serde_json::from_value(segments).map_err(|e| err(StatusCode::BAD_REQUEST, format!("segments 解析失败: {e}")))?;
    let transcript = yekitrim_core::types::Transcript { segments: segs, language: body.get("language").and_then(|l| l.as_str()).unwrap_or("zh").to_string(), asr_metadata: None };

    // 落盘目标 = 当前**实际读到的那一份**（`transcript_file` 优先，可能是用户选的外部字幕），
    // 再同步已存在的 output/ 两份。只写 output/ 的话，「选了外部字幕后在预处理页编辑」
    // 下次读到的还是原文件 —— 编辑静默丢失。
    let mut targets: Vec<PathBuf> = Vec::new();
    if let Some(effective) = ws.transcript_path(&cfg) {
        targets.push(effective);
    }
    if targets.is_empty() {
        // 配置写了但文件不在（例如刚选的外部字幕被删了）→ 写回配置指的位置，把它建回来。
        match &configured {
            Some(raw) => targets.push(ws.resolve(raw)),
            None => return Err(err(StatusCode::NOT_FOUND, "暂无转写文件")),
        }
    }
    let json_path = output.join("transcript.json");
    let srt_path = output.join("video.srt");
    for existing in [json_path, srt_path] {
        if existing.exists() && !targets.iter().any(|t| same_path(t, &existing)) {
            targets.push(existing);
        }
    }

    let json_text = serde_json::to_string_pretty(&transcript).map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("序列化失败: {e}")))?;
    for target in &targets {
        if target.extension().map(|e| e.eq_ignore_ascii_case("json")).unwrap_or(false) {
            std::fs::write(target, &json_text).map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("写入失败 {}: {e}", target.display())))?;
        } else {
            yekitrim_core::srt::write_srt(&transcript, target).map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("写字幕失败 {}: {e}", target.display())))?;
        }
    }
    let paths: Vec<String> = targets.iter().map(|p| yekitrim_core::path::display(p)).collect();
    ok(serde_json::json!({ "ok": true, "saved": transcript.segments.len(), "paths": paths }))
}

/// 统计 SRT 文件中的字幕条数（以时间轴行 `-->` 为准）；文件缺失/编码异常时为 0。
pub(super) fn count_srt_cues(path: &Path) -> u64 {
    match std::fs::read(path) {
        Ok(bytes) => String::from_utf8_lossy(&bytes).lines().filter(|l| l.contains("-->")).count() as u64,
        Err(_) => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::test_support::*;

/// 选了**外部字幕**（`transcript_file` 指向工程内的 sub.srt）后，在预处理页保存编辑
/// 必须写回那份文件 —— 只写 output/ 的话下次读到的还是原文件，编辑静默丢失。
#[test]
fn transcript_save_writes_back_to_selected_subtitle() {
    let (ws, root) = tmp_workspace("sub-writeback");
    let sub = root.join("sub.srt");
    std::fs::write(&sub, "1\n00:00:01,000 --> 00:00:02,000\n原始\n\n").unwrap();
    yekitrim_core::config::patch_config_file(
        &ws.config_file,
        &serde_json::json!({ "project": { "transcript_file": "sub.srt" } }),
    )
    .unwrap();

    let _ = put_transcript(&ws, &["改过的"]).unwrap();
    let saved = std::fs::read_to_string(&sub).unwrap();
    assert!(saved.contains("改过的"), "外部字幕必须就地更新: {saved}");
    // 没有 output/ 产物时不应凭空多造一份
    assert!(!root.join("output/transcript.json").exists());
    let _ = std::fs::remove_dir_all(&root);
}

/// ASR 产物的工程：两份 output/ 文件都要同步写（老行为不能丢）。
#[test]
fn transcript_save_keeps_output_pair_in_sync() {
    let (ws, root) = tmp_workspace("output-pair");
    std::fs::write(root.join("output/transcript.json"), "{\"segments\":[]}").unwrap();
    std::fs::write(root.join("output/video.srt"), "1\n00:00:01,000 --> 00:00:02,000\n旧\n\n").unwrap();
    yekitrim_core::config::patch_config_file(
        &ws.config_file,
        &serde_json::json!({ "project": { "transcript_file": "output/video.srt" } }),
    )
    .unwrap();

    let _ = put_transcript(&ws, &["新的"]).unwrap();
    assert!(std::fs::read_to_string(root.join("output/video.srt")).unwrap().contains("新的"));
    assert!(std::fs::read_to_string(root.join("output/transcript.json")).unwrap().contains("新的"));
    let _ = std::fs::remove_dir_all(&root);
}

/// 一份转写都没有、也没选字幕 → 仍然 404（不给空工程凭空造字幕）。
#[test]
fn transcript_save_without_any_transcript_is_404() {
    let (ws, root) = tmp_workspace("no-transcript");
    let e = put_transcript(&ws, &["x"]).unwrap_err();
    assert_eq!(e.0, StatusCode::NOT_FOUND);
    let _ = std::fs::remove_dir_all(&root);
}

/// `GET /api/project-data/assets` 的返回必须带来源与两侧候选（前端据此决定文案与预载语义）。
#[test]
fn assets_impl_reports_source_and_candidates() {
    let (ws, root) = tmp_workspace("assets-impl");
    std::fs::write(root.join("video.mp4"), b"x").unwrap();
    std::fs::write(root.join("video.xml"), b"<i/>").unwrap();
    yekitrim_core::config::patch_config_file(
        &ws.config_file,
        &serde_json::json!({ "project": { "input_file": "video.mp4" } }),
    )
    .unwrap();
    let v = assets_impl(&ws, "recorder", "录播姬 · 房间 1", Some("1")).unwrap();
    assert_eq!(v["source"]["kind"], "recorder");
    assert_eq!(v["danmaku"]["value"], "");
    assert_eq!(v["danmaku"]["exists"], false);
    assert_eq!(v["candidates"]["danmaku"][0]["name"], "video.xml");
    assert_eq!(v["candidates"]["danmaku"][0]["same_stem"], true);
    // 两种 ASR 模式都没产物 → 转写入口不置灰
    assert_eq!(v["slots"]["hybrid"], false);
    assert_eq!(v["slots"]["single"], false);
    let _ = std::fs::remove_dir_all(&root);
}

/// 某模式的 ASR 产物一落盘，`slots` 就报该槽已占（前端据此把转写入口置灰）。
#[test]
fn assets_impl_reports_asr_slots() {
    let (ws, root) = tmp_workspace("assets-slots");
    std::fs::create_dir_all(root.join("output")).unwrap();
    std::fs::write(
        root.join("output/transcript.hybrid.json"),
        r#"{"segments":[{"start":0.0,"end":1.0,"text":"a"}],"language":"zh"}"#,
    )
    .unwrap();
    let v = assets_impl(&ws, "local", "", None).unwrap();
    assert_eq!(v["slots"]["hybrid"], true);
    assert_eq!(v["slots"]["single"], false);
    // 清单里也要能看到它（盘上有、没登记过 → 兜底列出来），且不可删
    let items = v["transcript"]["items"].as_array().expect("items");
    assert_eq!(items.len(), 1, "{items:?}");
    assert_eq!(items[0]["mode"], "hybrid");
    assert_eq!(items[0]["is_asr"], true);
    assert_eq!(items[0]["removable"], false);
    let _ = std::fs::remove_dir_all(&root);
}

/// 清单删除：用户素材可删，ASR 产物（文件还在）必须被拒。
#[test]
fn assets_put_impl_remove_protects_asr_items() {
    let (ws, root) = tmp_workspace("assets-remove");
    std::fs::create_dir_all(root.join("output")).unwrap();
    std::fs::write(root.join("dm.xml"), "<?xml version=\"1.0\"?><i><d p=\"1.5,1,25,16777215,0\">hi</d></i>").unwrap();
    std::fs::write(
        root.join("output/transcript.hybrid.json"),
        r#"{"segments":[{"start":0.0,"end":1.0,"text":"a"}],"language":"zh"}"#,
    )
    .unwrap();
    // 选中 ASR 产物 → 按槽登记（不可删）
    let v = assets_put_impl(
        &ws,
        &serde_json::json!({ "kind": "transcript", "path": "output/transcript.hybrid.json" }),
        false,
    )
    .unwrap();
    assert_eq!(v["value"], "output/transcript.hybrid.json");
    assert_eq!(v["removed"], false);

    let e = assets_put_impl(
        &ws,
        &serde_json::json!({ "kind": "transcript", "action": "remove", "path": "output/transcript.hybrid.json" }),
        false,
    )
    .unwrap_err();
    assert_eq!(e.0, StatusCode::BAD_REQUEST);

    // 用户素材：选了能删，删掉的正好是生效值 → 连带清空 `danmaku_file`
    let _ = assets_put_impl(&ws, &serde_json::json!({ "kind": "danmaku", "path": "dm.xml" }), false).unwrap();
    let v = assets_put_impl(
        &ws,
        &serde_json::json!({ "kind": "danmaku", "action": "remove", "path": "dm.xml" }),
        false,
    )
    .unwrap();
    assert_eq!(v["removed"], true);
    assert_eq!(v["value"], "");
    let cfg = ws.load_config().unwrap();
    assert_eq!(cfg.project.danmaku_file.as_deref(), Some(""));
    assert!(cfg.project.materials.danmaku.is_empty());
    // 字幕的 ASR 条目还在（删弹幕不该动字幕）
    assert_eq!(cfg.project.materials.transcript.len(), 1);
    let _ = std::fs::remove_dir_all(&root);
}

/// 选素材：写前解析（坏文件不落盘），并在返回里带上条数与状态。
#[test]
fn assets_put_impl_validates_and_reports_count() {
    let (ws, root) = tmp_workspace("assets-put");
    std::fs::write(
        root.join("dm.xml"),
        "<?xml version=\"1.0\"?><i><d p=\"1.5,1,25,16777215,0\">hi</d></i>",
    )
    .unwrap();
    let v = assets_put_impl(&ws, &serde_json::json!({ "kind": "danmaku", "path": "dm.xml" }), false)
        .unwrap();
    assert_eq!(v["count"], 1);
    assert_eq!(v["value"], "dm.xml");
    assert_eq!(v["status"]["exists"], true);

    // 坏文件（不是弹幕）→ 400 且不落盘
    std::fs::write(root.join("bad.xml"), "not danmaku at all").unwrap();
    let e = assets_put_impl(&ws, &serde_json::json!({ "kind": "danmaku", "path": "bad.xml" }), false)
        .unwrap_err();
    assert_eq!(e.0, StatusCode::BAD_REQUEST);
    let cfg = ws.load_config().unwrap();
    assert_eq!(cfg.project.danmaku_file.as_deref(), Some("dm.xml"));

    // 未知类别
    let e = assets_put_impl(&ws, &serde_json::json!({ "kind": "nope", "path": "dm.xml" }), false)
        .unwrap_err();
    assert_eq!(e.0, StatusCode::BAD_REQUEST);
    let _ = std::fs::remove_dir_all(&root);
}

/// 服务器目录浏览的提交形态：目录与文件名**分开发**，由服务端拼路径。
/// 前端拼不出来是对的 —— 服务端回传的目录可能带 `\\?\` verbatim 前缀。
#[test]
fn assets_put_impl_accepts_dir_plus_name() {
    let (ws, root) = tmp_workspace("assets-put-dirname");
    let sub = root.join("sub");
    std::fs::create_dir_all(&sub).unwrap();
    std::fs::write(sub.join("dm.xml"), "<?xml version=\"1.0\"?><i><d p=\"1.5,1,25,16777215,0\">hi</d></i>").unwrap();
    let v = assets_put_impl(
        &ws,
        &serde_json::json!({ "kind": "danmaku", "dir": root.to_string_lossy(), "name": "sub/dm.xml" }),
        true,
    )
    .unwrap();
    assert_eq!(v["count"], 1);
    // 落在工程根内 → 存相对路径（可移植）
    assert_eq!(v["value"], "sub/dm.xml");
    let _ = std::fs::remove_dir_all(&root);
}

}