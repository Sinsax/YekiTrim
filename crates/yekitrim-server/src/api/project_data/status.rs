//! `api::project_data::status`：项目状态、事件/片段列表入口、弹幕与搜索用文本、程序/工程配置。

use std::path::PathBuf;

use axum::Json;
use axum::response::IntoResponse;
use yekitrim_service::Workspace;

use crate::scope::Scope;

// 跨子模块 / 外部域：显式点名（不许 glob）
use axum::extract::Extension;
use super::events::data_events_impl;

use crate::api::config::config_yekitrim_get_impl;
use crate::api::config::config_yekitrim_put_impl;
use crate::api::polish::polish_projects_impl;
use crate::api::projects::project_status_impl;

use super::*;

// ---- 项目数据（P2：作用域 = 本人项目 id，`?config=` 仅作过渡期反查） ----
//
// `Scope` 提取器从 `?project=<id>`（或过渡期的 `?config=<已登记本人项目>`）解析出项目与
// `Workspace`；**未登记的路径一律 404**，所以"任意路径"这条入口已经关闭。

// 以下都是"读一个项目的产物目录"的接口：目录可能在网络盘上，解析与统计一律下 blocking 池
pub(crate) async fn project_data_status(scope: Scope) -> ApiResult {
    let ws = scope.ws;
    blocking(move || project_status_impl(&ws)).await
}

pub(crate) async fn project_data_events(
    scope: Scope,
    headers: axum::http::HeaderMap,
) -> axum::response::Response {
    let ws = scope.ws;
    let inm = headers
        .get(axum::http::header::IF_NONE_MATCH)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    // 同 clips：events.json 只在检测 / 剪切 / 编辑后变化 ⇒ ETag/304（2026-11-25）
    match blocking(move || data_events_impl(&ws)).await {
        Ok(Json(v)) => crate::api::etag_json(inm.as_deref(), v),
        Err(e) => e.into_response(),
    }
}

pub(crate) async fn project_data_polish(scope: Scope) -> ApiResult {
    let ws = scope.ws;
    blocking(move || polish_projects_impl(&ws)).await
}

/// `GET /api/project-data/config?project=<id>`：**某个工程**的配置。
///
/// 与 `GET /api/config`（纯程序配置）分开是本目录的既有一刀：那道接口编辑的是
/// `.yekitrim/config.toml` / `llm.toml`，这道回的是**这个工程的** `yekitrim.toml` 原文 + 合并后的生效配置。
/// 项目设置页的源码编辑器读 `yekitrim_raw`（`ProjectSettingsView.vue`），预处理页读 `config.asr.mode`。
pub(crate) async fn project_data_config(scope: Scope) -> ApiResult {
    let ws = scope.ws;
    blocking(move || config_yekitrim_get_impl(&ws)).await
}

pub(crate) async fn project_data_config_put(
    scope: Scope,
    Extension(user): Extension<crate::auth::CurrentUser>,
    Json(body): Json<serde_json::Value>,
) -> ApiResult {
    let ws = scope.ws;
    let is_admin = user.is_admin();
    // 读改写加锁：设置页可能同时提交源文件与其它字段
    let path = ws.config_file.clone();
    let _guard = yekitrim_core::filelock::lock_path(&path).await;
    blocking(move || config_yekitrim_put_impl(&ws, &body, is_admin)).await
}

/// 读取项目转写文件（`transcript_file` / `output/transcript.json` / `output/video.srt`）
/// 中所有字幕段的起始秒，升序。
pub(super) fn transcript_starts(ws: &Workspace) -> Vec<f64> {
    let Ok(cfg) = ws.load_config() else { return Vec::new() };
    let Some(p) = ws.transcript_path(&cfg) else { return Vec::new() };
    let Ok(raw) = std::fs::read_to_string(&p) else { return Vec::new() };
    let mut starts: Vec<f64> = if p.extension().and_then(|e| e.to_str()) == Some("json") {
        serde_json::from_str::<serde_json::Value>(&raw).ok()
            .and_then(|v| v.get("segments").and_then(|s| s.as_array()).map(|arr| {
                arr.iter().filter_map(|s| s.get("start").and_then(|x| x.as_f64())).collect::<Vec<f64>>()
            }))
            .unwrap_or_default()
    } else {
        yekitrim_core::srt::parse_srt(&raw).map(|t| t.segments.iter().map(|s| s.start).collect()).unwrap_or_default()
    };
    starts.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    starts
}

/// 读取项目弹幕文件（XML / ASS / SRT）中所有弹幕的起始秒，升序。
pub(super) fn danmaku_starts(ws: &Workspace) -> Vec<f64> {
    let Ok(cfg) = ws.load_config() else { return Vec::new() };
    let Some(p) = ws.danmaku_path(&cfg) else { return Vec::new() };
    let Ok(entries) = yekitrim_core::danmaku::load_danmaku_auto(&p) else { return Vec::new() };
    let mut starts: Vec<f64> = entries.iter().map(|e| e.start).collect();
    starts.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    starts
}

/// 读取项目弹幕文件里的**弹幕正文**（升序，去掉空行）—— 首页切片总览的"搜弹幕"用。
///
/// ⚠️ 复用 `danmaku_starts` 那条**同一条解析路径**（`Workspace::danmaku_path` +
/// `load_danmaku_auto`，ASS / 录播姬 XML / SRT 都认）：换一条自己拼路径的读法就会
/// 重现"弹幕文件解析出来了、但正文读不到"那类静默失败。
///
/// ⚠️ 只回**正文**、不回时间：搜索是按关键词找片段，时间轴信息用不上；带上时间只会
/// 让载荷凭空翻几倍（实测单项目弹幕文件 4~72 KB）。
pub(super) fn danmaku_texts(ws: &Workspace) -> Vec<String> {
    let Ok(cfg) = ws.load_config() else { return Vec::new() };
    let Some(p) = ws.danmaku_path(&cfg) else { return Vec::new() };
    let Ok(entries) = yekitrim_core::danmaku::load_danmaku_auto(&p) else { return Vec::new() };
    let mut rows: Vec<(f64, String)> = entries
        .into_iter()
        .filter(|e| !e.text.trim().is_empty())
        .map(|e| (e.start, e.text))
        .collect();
    rows.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
    rows.into_iter().map(|(_, t)| t).collect()
}

/// 弹幕正文（首页切片总览「搜弹幕」按需拉；`{lines: [...]}`）。
///
/// 404 当空态处理由前端负责 —— 这里弹幕文件没配就回空数组，不报错（跟"没配弹幕"是一回事）。
pub(crate) async fn project_data_danmaku(scope: Scope) -> ApiResult {
    let ws = scope.ws;
    blocking(move || ok(serde_json::json!({ "lines": danmaku_texts(&ws) }))).await
}

/// **搜索用文本**（首页切片总览的搜索框按需拉）：一次回**字幕 + 弹幕**两桶。
///
/// 一次回两桶而不是两个接口：勾上"字幕+弹幕"就是 12 个项目各一发，两桶分开就是 24 发 ——
/// 而且两边的失效时机完全一样（同一个任务结束刷新）。
///
/// ⚠️ **字幕桶要含 LLM 订正稿**（`output/video_llm.srt`，存在才读）：
/// 订正稿能救回原稿听错的词（实测"男友粉"那句只有订正稿完整，见 `.dsh/skills/yekitrim-topic-scan`）。
/// 两个都塞进同一个桶（前端不区分来源，只按文本匹配）；不做去重 —— 同一条话原稿/订正稿
/// 各存一份是正常的，去重反而要按时间对齐、徒增复杂度。
///
/// ⚠️ 只回**文本**、不回时间：搜索是按关键词找片段，时间轴信息用不上（带上会让载荷翻几倍）。
pub(super) fn search_texts(ws: &Workspace) -> serde_json::Value {
    let subs = subtitles_for_search(ws);
    ok_json_texts(subs, danmaku_texts(ws))
}

/// 两桶拼成 `{subs: [...], dm: [...]}`。
fn ok_json_texts(subs: Vec<String>, dm: Vec<String>) -> serde_json::Value {
    serde_json::json!({ "subs": subs, "dm": dm })
}

/// 字幕桶 = **生效转写稿** + **LLM 订正稿**（两者可能是同一个文件，那时只算一份）。
///
/// ⚠️ **生效转写稿未必是 JSON**：`transcript_file` 实测有三种取值 ——
/// `output/transcript.json`（最常见）、`output/video.srt`（订正/导出后的 SRT）、
/// `output/video_llm.srt`（**订正稿直接被登记成生效稿**）。只按 JSON 解会把 p2/p3 判成
/// "这个项目没有字幕"（实测 1018 / 2002 段全丢）—— 那是比"搜不到"更糟的静默错误。
/// 所以按**扩展名**分派解析。
///
/// ⚠️ 订正稿与生效稿**是同一个文件时不重复加**：同一条话存两份会让"命中 N 条"翻倍。
fn subtitles_for_search(ws: &Workspace) -> Vec<String> {
    let mut out = Vec::new();
    let mut seen: Option<PathBuf> = None;
    if let Ok(cfg) = ws.load_config() {
        if let Some(p) = ws.transcript_path(&cfg) {
            out.extend(read_subtitle_file(&p));
            seen = Some(p);
        }
    }
    // 订正稿：只有它**不是**刚才那份生效稿时才补（否则同内容算两遍）
    let llm = ws.output_dir().join("video_llm.srt");
    if seen.as_deref() != Some(llm.as_path()) {
        out.extend(read_subtitle_file(&llm));
    }
    out
}

/// 读一份字幕文件（`.json` 走 `segments[].text`，其余按 SRT 正文剥）。
///
/// 认不出的扩展名**按 SRT 试一遍**：字幕文件本质上只有这两种形态，而"读不到就当没有"
/// 正是这条路上最容易出的静默错误。
fn read_subtitle_file(path: &std::path::Path) -> Vec<String> {
    let Ok(raw) = std::fs::read_to_string(path) else { return Vec::new() };
    let is_json = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.eq_ignore_ascii_case("json"))
        .unwrap_or(false);
    if is_json {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&raw) {
            return v
                .get("segments")
                .and_then(|s| s.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|s| s.get("text").and_then(|t| t.as_str()))
                        .map(|t| t.trim().to_string())
                        .filter(|t| !t.is_empty())
                        .collect()
                })
                .unwrap_or_default();
        }
        return Vec::new();
    }
    srt_body_lines(&raw)
}

/// 从 SRT 正文里剥出可读文本：去掉序号行与时间行，丢掉只含标记的行。
///
/// ⚠️ 与前端 `utils.js::dmPlain` 那条是**两回事**（那条摘 ASS 覆写标签）：SRT 只需要
/// 跳过 `1` / `00:00:01,000 --> 00:00:03,000` 这两类行，别把正文里的数字一起误杀。
/// ⚠️ **先剥 BOM**：实测这几个 `video_llm.srt` 开头都是 `\u{feff}1`，
/// 不剥的话第一行的序号解析不成数字 → 那个 `1` 会被当成正文进搜索池。
fn srt_body_lines(raw: &str) -> Vec<String> {
    let raw = raw.trim_start_matches('\u{feff}');
    let mut out = Vec::new();
    for line in raw.lines() {
        let t = line.trim().trim_start_matches('\u{feff}').trim();
        if t.is_empty() || t.contains("-->") || t.parse::<u32>().is_ok() {
            continue;
        }
        out.push(t.to_string());
    }
    out
}

/// 搜索文本（`{subs: [...], dm: [...]}`）。
pub(crate) async fn project_data_search_text(scope: Scope) -> ApiResult {
    let ws = scope.ws;
    blocking(move || ok(search_texts(&ws))).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::test_support::*;

    use axum::body::Body;

    /// **空态不是错误**（2026-11-15 用户实测）：只跑过「查找事件」、还没剪片段的项目，
    /// `/project-data/clips` 曾经回 404「暂无 clips」—— 而切片页把四个接口放在
    /// `Promise.all` 里，一个 404 让整页 catch，**连已经读到的 events 一起丢掉**，
    /// 界面显示「切片数据加载失败：暂无 clips」，用户以为检测白跑了（67 条事件其实在）。
    ///
    /// 契约：没有产物 = 空集合（200），不是资源不存在（404）。
    /// ⚠️ 写接口（PUT events / merge-splits / cut-all）保持 404 —— 那些是"操作不存在的产物"。
    #[tokio::test]
    async fn empty_project_returns_empty_collections_not_404() {
        let t = test_app().await;
        let root = t._dir.path().join("empty-proj");
        std::fs::create_dir_all(root.join("output")).unwrap();
        std::fs::write(root.join("big.mp4"), vec![0u8; 16]).unwrap();
        let cfg_path = root.join("yekitrim.toml");
        std::fs::write(&cfg_path, "input_file = \"big.mp4\"\n").unwrap();
        let pid = t
            .auth
            .db()
            .create_project(1, "empty-proj", None, "local", Some("空工程"), &root, &cfg_path, None)
            .unwrap();

        let res = t
            .call(t.req("GET", &format!("/project-data/clips?project={pid}")).body(Body::empty()).unwrap())
            .await;
        assert_eq!(res.status(), StatusCode::OK, "没有片段目录 ⇒ 200 + 空数组，不是 404");
        let body = body_json(res).await;
        assert_eq!(body.as_array().map(|a| a.len()), Some(0), "必须是空数组：{body}");

        let res = t
            .call(t.req("GET", &format!("/project-data/events?project={pid}")).body(Body::empty()).unwrap())
            .await;
        assert_eq!(res.status(), StatusCode::OK, "没有 events.json ⇒ 200 + 空集合，不是 404");
        let body = body_json(res).await;
        assert_eq!(
            body.get("events").and_then(|e| e.as_array()).map(|a| a.len()),
            Some(0),
            "events 必须是空数组：{body}"
        );
    }

/// 路径面归零：未登记的路径（含任意绝对路径）一律 404，只有本人项目 id 可用。
#[tokio::test]
async fn path_face_is_closed_for_project_data() {
    let t = test_app().await;
    // 未登记的 config 路径
    let res = t
        .call(t.req("GET", "/project-data/status?config=C%3A%5CWindows%5Cyekitrim.toml").body(Body::empty()).unwrap())
        .await;
    assert_eq!(res.status(), StatusCode::NOT_FOUND, "未登记路径必须 404");
    // 他人的项目 id
    let res = t
        .call(t.req("GET", "/project-data/status?project=999").body(Body::empty()).unwrap())
        .await;
    assert_eq!(res.status(), StatusCode::NOT_FOUND, "不存在/他人的项目 id 必须 404");
    // 本项目 id → 200
    let res = t
        .call(t.req("GET", "/project-data/status?project=1").body(Body::empty()).unwrap())
        .await;
    assert_eq!(res.status(), StatusCode::OK, "本人项目应可用: {:?}", res.status());
}

/// 全局工程状态删掉之后，`/api/config` 必须是**纯程序配置**：不再回项目相关字段。
///
/// 这几个字段（`config_file` / `root_dir` / `yekitrim_raw` / `config`）曾是"当前工程"的投影，
/// 前端**一个都没消费**（项目设置页读的是 `/api/project-data/config` 的 `yekitrim_raw`）。
/// 留着它们等于在项目无关的接口上继续暴露"当前工程"这个概念。
#[tokio::test]
async fn global_config_endpoint_no_longer_exposes_a_project() {
    let t = test_app().await;
    let res = t.call(t.req("GET", "/config").body(Body::empty()).unwrap()).await;
    assert_eq!(res.status(), StatusCode::OK);
    let v = body_json(res).await;
    for gone in ["config_file", "root_dir", "yekitrim_raw", "config"] {
        assert!(v.get(gone).is_none(), "/api/config 不该再有 {gone}: {v}");
    }
    // 程序配置那几样必须还在（设置页靠它们）
    for kept in ["global_raw", "llm_raw", "global_config", "llm_config"] {
        assert!(v.get(kept).is_some(), "/api/config 少了 {kept}: {v}");
    }
}

/// 项目级的配置读取必须仍然给全"这个工程"的四样（项目设置页 / 预处理页靠它）。
#[tokio::test]
async fn project_scoped_config_still_returns_the_project() {
    let t = test_app().await;
    let res = t.call(t.req("GET", "/project-data/config?project=1").body(Body::empty()).unwrap()).await;
    assert_eq!(res.status(), StatusCode::OK);
    let v = body_json(res).await;
    for kept in ["config_file", "root_dir", "yekitrim_raw", "config"] {
        assert!(v.get(kept).is_some(), "项目配置少了 {kept}: {v}");
    }
}

/// **SRT 正文抽取**：序号行与时间行不许进搜索池，正文里的数字不许被误杀。
///
/// 现场：仓里那几个 `video_llm.srt` 开头都是 **BOM + `1`**（实测 `video_llm.srt` 的首字节是
/// `EF BB BF`）—— 不剥 BOM 的话 `"\u{feff}1".parse::<u32>()` 失败，那个序号 `1` 会被当成
/// 正文塞进搜索池（搜 "1" 会命中每一个项目，纯噪声）。
#[test]
fn srt_body_skips_index_and_time_lines() {
    let raw = "\u{feff}1\n00:00:22,500 --> 00:00:24,990\nI took a ride, and when nobody\n\n2\n00:00:25,020 --> 00:00:27,780\n第 3 次见面说了 42 句\n";
    let got = srt_body_lines(raw);
    assert_eq!(
        got,
        vec![
            "I took a ride, and when nobody".to_string(),
            "第 3 次见面说了 42 句".to_string()
        ],
        "序号行/时间行要丢掉、BOM 要剥掉、正文里的数字要留住"
    );
    assert!(!got.iter().any(|l| l == "1" || l == "2"), "序号不许漏进正文");
}

/// 只有时间行/序号的 SRT ⇒ 空（别回一堆空串）。
#[test]
fn srt_body_of_markup_only_is_empty() {
    assert!(srt_body_lines("1\n00:00:01,000 --> 00:00:02,000\n\n").is_empty());
    assert!(srt_body_lines("").is_empty());
}

/// **生效转写稿可能是 SRT**（真 bug 的回归）：`transcript_file` 实测取值有
/// `output/transcript.json` / `output/video.srt` / `output/video_llm.srt` 三种。
/// 只按 JSON 解会把"生效稿是 SRT"的项目整份判成没有字幕 ——
/// 实测项目 2（1018 段）与项目 3（2002 段）就这么静默丢了，而项目 1 的生效稿正是 `video_llm.srt`。
#[test]
fn subtitle_reading_handles_srt_as_effective_transcript() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    std::fs::create_dir_all(root.join("output")).unwrap();
    let srt = root.join("output/video.srt");
    std::fs::write(
        &srt,
        "\u{feff}1\n00:00:01,000 --> 00:00:02,000\n第一句\n\n2\n00:00:03,000 --> 00:00:04,000\n第二句\n",
    )
    .unwrap();
    let got = read_subtitle_file(&srt);
    assert_eq!(got, vec!["第一句".to_string(), "第二句".to_string()], "SRT 生效稿要读得出来");

    // JSON 那份照旧
    let js = root.join("output/transcript.json");
    std::fs::write(&js, r#"{"segments":[{"text":"甲"},{"text":"乙"}]}"#).unwrap();
    assert_eq!(read_subtitle_file(&js), vec!["甲".to_string(), "乙".to_string()]);

    // 读不到 = 空，不 panic
    assert!(read_subtitle_file(&root.join("output/nope.json")).is_empty());
}

/// 订正稿与生效稿**是同一个文件时不重复收**（否则"命中 N 条"会翻倍）。
#[test]
fn subtitles_do_not_double_count_corrected_draft() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    std::fs::create_dir_all(root.join("output")).unwrap();
    let ws = yekitrim_service::Workspace {
        config_file: root.join(yekitrim_core::path::PROJECT_CONFIG_FILE),
        root_dir: root.to_path_buf(),
    };
    // 生效稿 = 订正稿（真实情形：项目 1 的 `transcript_file` 就是 `output/video_llm.srt`）
    std::fs::write(
        root.join("yekitrim.toml"),
        "[project]\ninput_file = \"a.mp4\"\ntranscript_file = \"output/video_llm.srt\"\n",
    )
    .unwrap();
    std::fs::write(
        root.join("output/video_llm.srt"),
        "1\n00:00:01,000 --> 00:00:02,000\n只此一句\n",
    )
    .unwrap();
    let got = subtitles_for_search(&ws);
    assert_eq!(got, vec!["只此一句".to_string()], "同一个文件只算一次");
}

/// 弹幕正文桶：**走同一条解析路径**，且 ASS 覆写标签原样保留
/// （摘标签是前端 `utils.js::dmPlain` 的活 —— 解析层保留标签是给渲染用的，别在这一层摘）。
#[tokio::test]
async fn danmaku_texts_keeps_raw_ass_text() {
    let t = test_app().await;
    let pid = project_ids(&t).await[0];
    // 弹幕放**视频同目录**、用 `danmu.ass` 这个名字：`Workspace::danmaku_path` 配置没写时
    // 会回落找同目录的 `danmu.ass` / `danmaku.ass`，比手改配置更贴近真实工程。
    let dm = t.root.join("danmu.ass");
    std::fs::write(
        &dm,
        "[Script Info]\n[Events]\nFormat: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\n\
         Dialogue: 0,0:00:01.00,0:00:05.00,Default,,0,0,0,,{\\move(1970,0,-50,0)}呜呜呜\n\
         Dialogue: 0,0:00:06.00,0:00:09.00,Default,,0,0,0,,怎么没人说话\n",
    )
    .unwrap();
    // ⚠️ `ProjectConfig` 是从 **`[project]` 表**反序列化的：fixture 里那行**顶层**
    //    `input_file = "a.mp4"` 读不出来（`load_config` 走 `[project]`）—— 而"配置没写就找同目录
    //    `danmu.ass`"这条回落**要 input_file** 才走得到。这里补一份带 `[project]` 的配置。
    std::fs::write(t.root.join("yekitrim.toml"), "[project]\ninput_file = \"a.mp4\"\n").unwrap();

    let res = t
        .call(t.req("GET", &format!("/project-data/search-text?project={pid}")).body(Body::empty()).unwrap())
        .await;
    assert_eq!(res.status(), StatusCode::OK, "search-text 应当可用");
    let v = body_json(res).await;
    let dm_lines: Vec<String> = v["dm"]
        .as_array()
        .expect("要回 dm 桶")
        .iter()
        .map(|x| x.as_str().unwrap_or("").to_string())
        .collect();
    assert_eq!(dm_lines.len(), 2, "两条弹幕都要在：{dm_lines:?}");
    assert!(
        dm_lines[0].contains("{\\move(1970,0,-50,0)}"),
        "解析层要**原样保留** ASS 覆写标签（摘标签是前端的事）：{dm_lines:?}"
    );
    assert!(dm_lines.iter().any(|l| l.contains("怎么没人说话")));
    // 没配弹幕文件的项目：**回空桶**、不报错（"没配弹幕"与"弹幕是空的"对搜索是一回事）
    assert!(v["subs"].as_array().is_some(), "subs 桶也必须存在（可能是空的）");
}

}