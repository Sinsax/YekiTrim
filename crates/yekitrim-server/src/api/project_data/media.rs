//! `api::project_data::media`：`/api/media/*path` 文件出口（ETag / Range / 归属校验）。

use std::path::PathBuf;

use axum::extract::{Path as AxumPath, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

use crate::model::AppState;
use crate::media;

// 跨子模块 / 外部域：显式点名（不许 glob）
use axum::extract::Extension;

use super::*;

pub(crate) async fn media_get(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<crate::auth::CurrentUser>,
    AxumPath(path): AxumPath<String>,
    headers: axum::http::HeaderMap,
) -> Response {
    // 只服务本人项目内的文件：
    // - 新契约 `/api/media/<project_id>/<项目内相对路径>`
    // - 过渡期 `/api/media/<绝对路径>`：必须落在自己的某个项目根内
    //
    // 归属校验 + `jail_existing`/`canonicalize` 都是对项目目录的同步 IO（可能网络盘），
    // 而播放视频会打出一连串 Range 请求 —— 整段下 blocking 池，别让播放把 worker 占满。
    let (project, rel) = media::split_project_media(&path);
    let st = state.clone();
    let resolved = blocking(move || resolve_media_path(st.auth.db(), &user, project, &rel)).await;
    let path = match resolved {
        Ok(p) => p,
        Err(e) => return e.into_response(),
    };
    let mime = match path.extension().and_then(|e| e.to_str()).map(|e| e.to_ascii_lowercase()).as_deref() {
        Some("mp4" | "m4v") => "video/mp4".to_string(),
        Some("webm") => "video/webm".to_string(),
        Some("ogv") => "video/ogg".to_string(),
        Some("mov") => "video/quicktime".to_string(),
        _ => mime_guess::from_path(&path).first_or_octet_stream().to_string(),
    };
    // **ETag/304 + 文本类 gzip**：口径与硬约束全在 `media::serve_file_cached` 的文档里，
    // 这里只负责把头交过去 —— **别再在这一层自己抠 `Range`/`If-None-Match`**：
    // 那正是「四个文件出口只有这一条补了 ETag、另外四条漏了」的成因（2026-11-21 修）。
    //
    // 历史（2026-10-02，别丢）：精修页进 Dhampir 时会把该工程引用的**全部动图**整份取一遍
    // —— 实测一组 5 张贴纸合计 **15.72 MB**（单张最大 4.95 MB）。没有校验符时浏览器每次进
    // 页面都得原样重收一遍，用户看到的是「动图加载要 6.5 秒」，而大头其实是这 15.7 MB 的搬运。
    media::serve_file_from_headers(&path, &mime, &headers).await
}

/// `/api/media/*` 的归属校验 + 路径解析（同步：DB 查询 + 项目目录上的 canonicalize/jail）。
/// 抽出来是为了整体丢进 blocking 池 —— 见 `media_get` 的调用点。
///
/// ⚠️ **过渡期那条 `/api/media/<绝对路径>` 已于 2026-10-03 删除**。它当年靠一份"全局当前工程"
/// （`AppState.workspace`）把相对路径解析成绝对路径、再用 `find_project_for_dir` 反查归属 ——
/// 那份全局状态已经删掉（见 `model.rs` 头部）。而且**前端早就明确不认这条路**：
/// `webui/src/utils.js::mediaUrl` 的注释写着"不接受绝对路径，也不做旧 `/api/media/<绝对路径>` 的兼容"，
/// 所有调用点都走 `mediaUrl` / `clipMediaUrl`。
///
/// 判据是**路径形状**而不是"全局状态在不在"：要服务媒体就必须带项目 id —— 这样归属校验
/// 永远只看 `find_owned_project` 一处，不再有第二条反查旁路。
pub(super) fn resolve_media_path(
    db: &yekitrim_service::db::Db,
    user: &crate::auth::CurrentUser,
    project: Option<i64>,
    rel: &str,
) -> Result<PathBuf, AppError> {
    // `None` = 路径第一段不是正整数 id（旧契约，或手敲的 URL）。
    // 回 404 而不是 403：与"项目不存在"同口径，不给"这个路径存在但你没权限"的区分度。
    let Some(id) = project else {
        return Err(err(
            StatusCode::NOT_FOUND,
            "媒体路径必须带项目 id（/api/media/<项目id>/<项目内相对路径>）",
        ));
    };
    let proj = match crate::scope::find_owned_project(db, user, id) {
        Ok(Some(p)) => p,
        Ok(None) => return Err(err(StatusCode::NOT_FOUND, "项目不存在或无权访问")),
        Err(e) => return Err(err(StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}"))),
    };
    yekitrim_core::tenant::jail_existing(&proj.root_dir, rel)
        .map_err(|e| err(StatusCode::NOT_FOUND, format!("文件不可用: {e:#}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::test_support::*;

    use axum::body::Body;

    /// **`/api/media/*` 必须带 ETag，并且命中 `If-None-Match` 时回 304。**
    ///
    /// # 为什么这条判据要有（2026-10-02）
    ///
    /// 精修页进 Dhampir 时会把该工程引用的**全部动图**原样取一遍 —— 实测一组 5 张贴纸
    /// 合计 **15.72 MB**（单张最大 4.95 MB）。这条路此前**没有任何校验符、也没有
    /// `Cache-Control`** ⇒ 每次进页面都得重新搬一遍这 15.7 MB。
    ///
    /// 用户当时看到的症状是「动图加载要 6.5 秒」，而那条日志里的"解码"其实把
    /// `fetch` + `arrayBuffer` 也算进去了（宿主侧计时区间打错，
    /// 见 `PolishPreview.vue` 里那段注释）—— 于是"解码很慢"这个结论是被错计时喂出来的，
    /// 真正的大头是这 15.7 MB 的搬运。
    ///
    /// ⚠️ 与 wasm 那条的差别：这里**不能开压缩**（`/api/media/*` 要服务视频的 Range，
    /// 压缩后字节不再一一对应，会破坏播放）。所以本用例只验 ETag/304 与 Range 的共存。
    #[tokio::test]
    async fn media_带_etag_且命中时回_304() {
        let t = test_app().await;
        let pid = project_ids(&t).await[0];
        let rel = "output/clips/media-etag/sticker.gif";
        let dir = t.root.join("output/clips/media-etag");
        std::fs::create_dir_all(&dir).unwrap();
        // 用纯 ASCII 文件名：URI 里带 CJK 要走百分号编码，那是另一条判据，别混进来。
        // 造一份"看起来像视频"的字节：内容无所谓，判据只看长度/mtime 算出的校验符。
        let bytes: Vec<u8> = (0..4096u32).map(|i| (i % 251) as u8).collect();
        std::fs::write(t.root.join(rel), &bytes).unwrap();

        let uri = format!("/media/{pid}/{rel}");
        let req = |extra: Vec<(&str, &str)>| {
            let mut b = t.req("GET", &uri);
            for (k, v) in extra {
                b = b.header(k, v);
            }
            b.body(Body::empty()).unwrap()
        };

        // ① 首次：200 + **必须有 ETag**（没有它浏览器就无从校验，只能整份重收）
        let res = t.call(req(vec![])).await;
        assert_eq!(res.status(), StatusCode::OK, "首次取媒体应当成功");
        let etag = res
            .headers()
            .get(axum::http::header::ETAG)
            .expect("`/api/media/*` 必须带 ETag —— 否则 15.7 MB 的贴纸每次进页面都重传")
            .to_str()
            .unwrap()
            .to_string();
        assert!(etag.starts_with('"') && etag.ends_with('"'), "ETag 要带引号：{etag}");
        assert!(
            res.headers().get(axum::http::header::CONTENT_ENCODING).is_none(),
            "媒体路由**不许压缩**（要服务 Range，压缩会破坏播放）"
        );
        let len = axum::body::to_bytes(res.into_body(), usize::MAX).await.unwrap().len();
        assert_eq!(len, bytes.len());

        // ② 回带同一个 ETag ⇒ 304 且**空体**（省下的正是那 15.7 MB）
        let res = t.call(req(vec![("if-none-match", etag.as_str())])).await;
        assert_eq!(res.status(), StatusCode::NOT_MODIFIED, "ETag 没变就该 304");
        let body = axum::body::to_bytes(res.into_body(), usize::MAX).await.unwrap();
        assert!(body.is_empty(), "304 不许带 body，实际 {} 字节", body.len());

        // ③ 文件变了（长度变）⇒ ETag 必须变，刷新不能拿到旧贴纸
        let mut bigger = bytes.clone();
        bigger.extend_from_slice(b"tail");
        std::fs::write(t.root.join(rel), &bigger).unwrap();
        let res = t.call(req(vec![("if-none-match", etag.as_str())])).await;
        assert_eq!(
            res.status(),
            StatusCode::OK,
            "文件变了却仍回 304 —— 用户刷新会拿到旧贴纸（比慢更糟）"
        );
        let res = t.call(req(vec![])).await;
        let new_etag = res
            .headers()
            .get(axum::http::header::ETAG)
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();
        assert_ne!(new_etag, etag, "长度变了 ETag 必须变");

        // ④ Range 与 ETag 共存：**206 仍然按原字节发**（ETag 不许把播放搞坏）
        let res = t.call(req(vec![("range", "bytes=0-99")])).await;
        assert_eq!(res.status(), StatusCode::PARTIAL_CONTENT, "带 Range 必须还是 206");
        let part = axum::body::to_bytes(res.into_body(), usize::MAX).await.unwrap();
        assert_eq!(part.len(), 100, "Range 的字节数不对：{}", part.len());
        assert_eq!(&part[..], &bigger[..100], "206 的字节必须与原始文件一一对应");
    }

    /// **复现真实场景的省流量**：5 张动图贴纸（合计约 15.7 MB）第二次进页面时**一字节都不传**。
    ///
    /// 上一条验的是"304 的机制对不对"，这条验的是"**省下的量级**对不对" ——
    /// 用户报的是"动图加载 6.5 秒"，而那 6.5 秒里的大头是这 15.7 MB 的搬运。
    /// 判据用**实际读出的字节数**，不用 header：只有真少传了才算数。
    #[tokio::test]
    async fn media_第二次进页面时贴纸一字节都不重传() {
        let t = test_app().await;
        let pid = project_ids(&t).await[0];
        let dir = t.root.join("output/clips/sticker-vol");
        std::fs::create_dir_all(&dir).unwrap();

        // 五个文件，尺寸取真实贴纸的量级（这里按 1/100 缩放造：够验机制，又不拖慢用例）。
        let sizes = [495, 426, 247, 217, 187];
        let mut names = Vec::new();
        for (n, size) in sizes.iter().enumerate() {
            let name = format!("sticker-{n}.gif");
            std::fs::write(dir.join(&name), vec![0xA5u8; size * 1024]).unwrap();
            names.push(name);
        }

        // 第一次：全量取，同时记下每个文件的 ETag 与字节数
        let mut total_first = 0usize;
        let mut etags = Vec::new();
        for name in &names {
            let uri = format!("/media/{pid}/output/clips/sticker-vol/{name}");
            let res = t.call(t.req("GET", &uri).body(Body::empty()).unwrap()).await;
            assert_eq!(res.status(), StatusCode::OK);
            etags.push(
                res.headers()
                    .get(axum::http::header::ETAG)
                    .expect("每个贴纸都要有 ETag")
                    .to_str()
                    .unwrap()
                    .to_string(),
            );
            total_first += axum::body::to_bytes(res.into_body(), usize::MAX).await.unwrap().len();
        }
        assert!(total_first > 1_000_000, "这组素材应当有 MB 级体量，实际 {total_first}");

        // 第二次（模拟用户再进一次精修页）：带 ETag ⇒ **全部 304、零字节**
        let mut total_second = 0usize;
        for (name, tag) in names.iter().zip(&etags) {
            let uri = format!("/media/{pid}/output/clips/sticker-vol/{name}");
            let res = t
                .call(t.req("GET", &uri).header("if-none-match", tag.as_str()).body(Body::empty()).unwrap())
                .await;
            assert_eq!(res.status(), StatusCode::NOT_MODIFIED, "{name} 没有命中 304");
            total_second += axum::body::to_bytes(res.into_body(), usize::MAX).await.unwrap().len();
        }
        assert_eq!(
            total_second, 0,
            "第二次进页面仍然搬了 {total_second} 字节 —— 贴纸应当一字节都不重传"
        );
    }

/// `/api/media/<绝对路径>` 那条**过渡期契约**必须已经关掉（2026-10-03 随全局工程状态一起删）。
///
/// 它当年是拿"全局当前工程"把相对路径解析成绝对路径、再用 `find_project_for_dir` 反查归属的 ——
/// 那条反查是**第二套归属判据**，与 `find_owned_project` 并存。现在只剩后者一条路。
/// 钉住两件事：① 不带项目 id 一律 404；② 带项目 id 时仍然只能拿自己项目内的文件。
#[tokio::test]
async fn media_requires_project_id_and_stays_inside_the_project() {
    let t = test_app().await;
    let clip = t.root.join("output").join("clips").join("clip-01");
    std::fs::create_dir_all(&clip).unwrap();
    std::fs::write(clip.join("clip.mp4"), b"fake-mp4").unwrap();

    // ① 项目内相对路径 + 合法项目 id → 能取到
    let ok = t.call(t.req("GET", "/media/1/output/clips/clip-01/clip.mp4").body(Body::empty()).unwrap()).await;
    assert_eq!(ok.status(), StatusCode::OK, "带项目 id 的项目内文件应当可取");

    // ② 不带项目 id（旧契约形状）→ 404，且**不许**被解析成"当前工程"下的文件
    for legacy in ["/media/output/clips/clip-01/clip.mp4", "/media/C:/Windows/win.ini"] {
        let res = t.call(t.req("GET", legacy).body(Body::empty()).unwrap()).await;
        assert_eq!(res.status(), StatusCode::NOT_FOUND, "旧契约必须已关闭: {legacy}");
    }

    // ③ 带项目 id 但跳出项目根 → 不放行
    let esc = t.call(t.req("GET", "/media/1/../yekitrim.toml").body(Body::empty()).unwrap()).await;
    assert_ne!(esc.status(), StatusCode::OK, "越出项目根不许放行");
}

}