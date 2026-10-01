//! `api::routes`：路由表与中间件装配。
//!
//! **这一份是唯一的路由权威**：`scripts/route_manifest.py` 抽的就是它，
//! 拆分前后必须逐条相同（评审 §7）。

use std::sync::Arc;

use axum::{
    Router,
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{delete, get, post, put},
    Json,
};

use crate::model::AppState;
use super::*;
use super::{auth::*, config::*, invite::*, logs::*, pipeline::*, polish::*,
            project_data::*, projects::*, rooms::*};

pub fn router(state: Arc<AppState>) -> Router {
    // 白名单（§10.1 step-2.1）：登录端点与 /health 之外，**所有**接口都要会话。
    let public = Router::new()
        .route("/health", get(health))
        .route("/auth/login", post(auth_login))
        // 自助注册：邀请码是凭证，注册开关是总闸；两者都由服务端校验（§3.5 之外）
        .route("/auth/register", post(auth_register))
        // 录播姬 webhook：自带 token 鉴权（录播姬只能填 URL，无法加自定义头），
        // 因此它**不能**挂在 require_user 下（§4.1）。
        .route("/ingest/bililive/:token", post(crate::ingest::webhook));

    // 平台级资源（P2 起归入 /api/admin/*，仅 admin）：配置 / 模型 / doctor / LLM 测试
    // 旧路径作为过渡别名挂在**同一个 require_admin 层**下（否则普通账号能读到 LLM key）
    let admin = Router::new()
        .route("/admin/doctor", get(doctor))
        .route("/admin/config", get(config_get))
        .route("/admin/config/global", put(config_global_put))
        .route("/admin/config/llm", put(config_llm_put))
        .route("/admin/llm/test", post(llm_test))
        .route("/admin/models", get(models_get))
        .route("/admin/models/download", post(models_download))
        // 采集/录播姬：`GET` 读概览、`PUT` 保存配置（表单）、`POST .../token` 生成/关闭 webhook token
        .route(
            "/admin/ingest",
            get(crate::ingest::config_view).put(crate::ingest::put_config),
        )
        .route("/admin/ingest/token", post(crate::ingest::rotate_token))
        .route("/admin/ingest/assign", post(crate::ingest::assign))
        .route("/admin/ingest/scan", post(crate::ingest::scan_now))
        // 内置下载器（yt-dlp）安装：全局资产（下到 <程序目录>/.vtrim/bin/），仅 admin
        .route("/admin/downloader/install", post(crate::ingest::downloader_install))
        .route("/admin/users", get(admin_users))
        .route("/admin/recorder/test", post(admin_recorder_test))
        .route("/admin/invites", get(admin_invites).post(admin_invite_create))
        .route("/admin/invites/registration", put(admin_registration_put))
        .route("/admin/invites/:id", axum::routing::delete(admin_invite_revoke))
        // 运行日志（服务端 tracing → 环形缓冲 / SSE；「运行日志」面板用，仅 admin）
        .route("/admin/logs", get(admin_logs))
        .route("/admin/logs/events", get(admin_logs_stream))
        // LLM 用量（每次调用的 token / 缓存命中 / 耗时；设置页「LLM 用量」卡，仅 admin）
        .route("/admin/usage", get(admin_usage))
        // 表情包（本地贴纸库）：全局资产，只有管理员能导入 / 构建 / 删除
        .route("/admin/stickers", get(crate::stickers::packs_list))
        .route("/admin/stickers/import", post(crate::stickers::pack_import))
        // 上传 zip 建新包（WebUI 的「添加表情包」）。请求体是压缩包原始字节 ——
        // 单独给这一条放开体积上限（axum 默认 2 MB），其它接口维持不变。
        .route(
            "/admin/stickers/upload",
            post(crate::stickers::pack_upload)
                .layer(axum::extract::DefaultBodyLimit::max(
                    crate::stickers::UPLOAD_MAX_BYTES,
                )),
        )
        .route(
            "/admin/stickers/:pack",
            get(crate::stickers::pack_detail).delete(crate::stickers::pack_delete),
        )
        .route("/admin/stickers/:pack/build", post(crate::stickers::pack_build))
        .route("/admin/stickers/:pack/files/:name", get(crate::stickers::pack_file))
        .route("/doctor", get(doctor))
        .route("/config", get(config_get))
        .route("/config/global", put(config_global_put))
        .route("/config/llm", put(config_llm_put))
        .route("/llm/test", post(llm_test))
        .route("/models", get(models_get))
        .route("/models/download", post(models_download))
        // 字体库**写**操作：库是**程序级全局资产**（`<exe 同级>/resources/fonts/`），
        // 与贴纸库同规矩 —— **读**谁都能读（`GET /api/polish/fonts`），**写只给 admin**。
        // 导入走"服务器上已有的文件路径"（`/api/fs/list` 也仅 admin）—— 这条仍在，
        // 用于"服务器上已经躺着那支字体"的场景。
        .route("/polish/fonts/import", post(polish_fonts_import))
        // **拖拽上传**（2026-10-02 用户要求"导入字体改为拖拽字体文件"）：请求体是字体的
        // 原始字节（`application/octet-stream`，不是 multipart —— 同贴纸上传那条做法）。
        // ⚠️ 必须单独给这一条放开体积上限：axum 默认只收 2 MB，而字体动辄 10~30 MB
        // （413 就是这层丢了）。上限常量与前端预检是同一个数。
        .route(
            "/polish/fonts/upload",
            post(polish_fonts_upload).layer(axum::extract::DefaultBodyLimit::max(
                FONT_UPLOAD_MAX_BYTES,
            )),
        )
        // 设为默认（唯一默认；**不动任何工程的字体配置**）
        .route("/polish/fonts/default", post(polish_fonts_default))
        // 下载**自带字体**（清单见 `vtrim_core::fonts::BUILTIN_FONTS`；幂等，已装好的跳过）。
        // ⚠️ 会联网（约 38 MB），所以是 POST 而不是 GET 的副作用；**仅 admin**。
        .route("/polish/fonts/builtin", post(polish_fonts_builtin))
        // 删除（`?id=`，与 `/polish/font?name=` 同形）；**程序自带的返回 403**
        .route("/polish/fonts", delete(polish_fonts_delete))
        // ⚠️ 改密强制门**也必须盖住 admin 路由**：初始密码会同时写进日志与
        // `initial-admin-password.txt`，持它的会话若能调 `/api/config`，就等于直接拿到
        // 明文 LLM API key（`config_get_impl` 会回 `llm_raw`）并能铸造邀请码 ——
        // 那正是 `require_password_changed` 存在的理由。
        // ⚠️ 这一行必须写在 `require_admin` **之前**：axum 里"后加的 layer 在外层"，
        // 写在前面 `require_admin` 才是外层、才会先跑并注入 `CurrentUser`；
        // 反过来的话改密门读不到用户，按 `unwrap_or(false)` fail-open —— 等于没挂。
        .route_layer(axum::middleware::from_fn(crate::auth::require_password_changed))
        .route_layer(axum::middleware::from_fn_with_state(
            state.clone(),
            crate::auth::require_admin,
        ));

    // polish 系列：额外加"目录必须落在本人项目内"的守卫（路径面归零，§5.2）
    let polish = Router::new()
        .route("/polish/init", post(polish_init))
        .route("/polish/llm", post(polish_llm))
        .route("/polish/llm-segment", post(polish_llm_segment))
        .route("/polish/process", post(polish_process))
        .route("/polish/render", post(polish_render))
        .route("/polish/cover", post(polish_cover))
        .route("/polish/preview", post(polish_preview))
    // Dhampir **浏览器端**预览：渲染跑在页面（wasm + WebGPU），后端只发资源与工程 JSON。
    // 与上面那条 DOM 预览**并列**（按钮切换保留）：一个看 CSS/DOM 复刻、一个看真底座。
    .route("/polish/dhampir-project", post(polish_dhampir_project))
    // 底座**出片**通道的就绪位（M3）：二进制/转译器/字体/node/版本契约各查一次文件，
    // 让"底座不可用"在点渲染之前就可见（前端挂在渲染弹窗上，`vtrim doctor` 用同一份判断）。
    .route("/polish/dhampir-status", post(polish_dhampir_status))
    .route("/polish/dhampir-wasm/*path", get(polish_dhampir_wasm))
        .route("/polish/cover-preview", post(polish_cover_preview))
        .route("/polish/cover-background", post(polish_cover_background))
        .route("/polish/cover-frame", post(polish_cover_frame))
        .route("/polish/project", get(polish_project_get).put(polish_project_put))
        .route("/polish/subtitles", get(polish_subtitles_get).put(polish_subtitles_put))
        // 贴纸的**第 k 帧**（浏览器侧没有可用的多帧图解码，见那个函数的说明）
        .route("/polish/sticker-frame", post(polish_sticker_frame))
        // 贴纸帧表**预热**：页面加载时就把"播到才解"的整段解码挪走（见那个函数的说明）
        .route("/polish/sticker-prewarm", post(polish_sticker_prewarm))
        .route("/polish/sticker-sources", get(polish_sticker_sources))
        // 贴纸库只读浏览（编辑器里 keyword→贴纸 的两级选择）；写操作仍只在 admin
        .route("/polish/sticker-library", get(polish_sticker_library))
        .route("/polish/sticker-file", get(polish_sticker_file))
        // 音效目录（编辑器里音效事件的列表选择）
        .route("/polish/sfx-catalog", get(polish_sfx_catalog))
        // ⚠️ 这里曾经有 `POST /polish/render-frame`（wgpu 单帧真渲染预览）—— 那条 GPU 链路已删除
        // （GPU 那侧交给独立的 Dhampir 底座）。别把它加回来。
        .route("/polish/font", get(polish_font))
        // 字体库只读浏览（exe 同级 resources/fonts/）：字幕/封面字体的下拉数据源。
        // ⚠️ 写入（导入/设默认/删除）**不在这里**，在 admin 层（见上面那三条）：
        // 库是程序级全局资产，一张普通账号的会话不该能改所有人的字体
        // （读走 `GET /api/polish/fonts`，写走 admin 层的 `/api/polish/fonts/*`）
        .route("/polish/fonts", get(polish_fonts))
        .route("/polish/select-version", post(polish_select_version))
        .route("/polish/silence", post(polish_silence))
        .route("/polish/cover-title", post(polish_cover_title))
        .route("/polish/cover-config", post(polish_cover_config))
        .route_layer(axum::middleware::from_fn_with_state(
            state.clone(),
            crate::scope::enforce_scope,
        ));

    let protected = Router::new()
        .route("/me", get(me))
        .route("/auth/logout", post(auth_logout))
        .route("/auth/password", put(auth_password))
        .route("/project/list", get(project_list))
        .route("/projects", get(projects_list).post(projects_create))
        .route("/projects/:id", axum::routing::delete(projects_delete))
        .route("/projects/:id/source", axum::routing::delete(projects_source_delete))
        .route("/project/delete", post(project_delete))
        .route("/project/init", post(project_init))
        .route("/project/open", post(project_open))
        .route("/project-data/status", get(project_data_status))
        .route("/project-data/events", get(project_data_events).post(project_data_events_create).put(project_data_events_put))
        .route("/project-data/clips", get(project_data_clips))
        .route("/project-data/clips/:rank", axum::routing::delete(project_data_clip_delete))
        .route("/project-data/clips/:rank/thumb", get(project_data_clip_thumb))
        .route("/project-data/events/cut-all", post(project_data_event_cut_all))
        .route("/project-data/events/merge-splits", post(project_data_events_merge_splits))
        .route("/project-data/events/:rank/cut", post(project_data_event_cut))
        .route("/project-data/clips/:rank/silence", post(project_data_clip_silence))
        .route("/project-data/transcript", get(project_data_transcript).put(project_data_transcript_put))
        .route("/project-data/correct", post(project_data_correct))
        .route("/project-data/pipeline", post(project_data_pipeline))
        .route("/project-data/config", get(project_data_config).put(project_data_config_put))
        // 素材（弹幕 / 字幕）：预处理页随时查看与切换
        .route("/project-data/assets", get(project_data_assets).post(project_data_assets_put))
        .route("/project-data/polish", get(project_data_polish))
        // 直播间自助登记（用户只填房间号/链接，服务端去录播姬加房间）
        .route("/rooms", get(rooms_list).post(rooms_add))
        .route("/rooms/:room_id", axum::routing::delete(rooms_delete))
        // 该房间在录像目录里的文件（展开房间行时用）+ 单个文件导入
        .route("/rooms/:room_id/recordings", get(crate::ingest::room_recordings))
        .route(
            "/rooms/:room_id/recordings/import",
            post(crate::ingest::import_recording),
        )
        .route("/fs/list", post(fs_list))
        .route("/ingest/records", get(crate::ingest::records))
        .route("/ingest/retry", post(crate::ingest::retry))
        // B站链接通道（P4）：解析（不落库）/ 提交下载 / 下载器就绪位
        .route("/ingest/resolve", post(crate::ingest::bilibili_resolve))
        .route("/ingest/bilibili", post(crate::ingest::bilibili_submit))
        .route("/ingest/downloader", get(crate::ingest::downloader_status))
        .route("/channel-map", get(crate::ingest::channel_map_get).put(crate::ingest::channel_map_put))
        .route("/stream", get(global_stream))
        .route("/jobs", get(jobs_list))
        .route("/jobs/:id", get(job_get))
        .route("/jobs/:id/events", get(job_events))
        .route("/jobs/:id/cancel", post(job_cancel))
        .route("/media/*path", get(media_get))
        .merge(polish)
        // ⚠️ 两层的顺序**不能反**：`require_user` 必须在外层（先跑并写入 `CurrentUser`），
        // 改密强制门在内层才读得到用户。axum 里"后加的 layer 在外层"，所以这两行看起来是倒着写的。
        .route_layer(axum::middleware::from_fn(crate::auth::require_password_changed))
        .route_layer(axum::middleware::from_fn_with_state(
            state.clone(),
            crate::auth::require_user,
        ));

    public
        .merge(admin)
        .merge(protected)
        // 未匹配的 /api/* 必须是 JSON 404：外层 fallback 是 SPA HTML，
        // 若让它兜住 API 请求，前端会把 200 HTML 解析成 null 并静默失败。
        .fallback(api_not_found)
        // 访问日志挂最外层：**每次前端交互都留痕**（含上面这个 JSON 404）。
        // 挂在 api::router 里而不是 `nest("/api", …)` 外面，是为了让路由单测也走它。
        .layer(axum::middleware::from_fn(crate::access_log::access_log))
        // 限流挂**最外层的更外层**（在访问日志之外）：
        // 1. 要覆盖未认证入口（登录/注册/入库 webhook），挂进 require_user 就漏了；
        // 2. 被限流的请求不该再各写一行访问日志 —— 否则刷接口的人顺手把日志也打满。
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            crate::ratelimit::limit_requests,
        ))
        .with_state(state)
}

pub(super) async fn api_not_found(req: axum::extract::Request) -> Response {
    let method = req.method().clone();
    // 这个 fallback 挂在 `nest("/api", …)` 里，`uri().path()` 已经去掉了 `/api` 前缀；
    // 用 OriginalUri 还原成用户实际请求的路径，否则报错文案会指向不存在的 URL。
    let path = req
        .extensions()
        .get::<axum::extract::OriginalUri>()
        .map(|u| u.0.path().to_string())
        .unwrap_or_else(|| req.uri().path().to_string());
    (
        StatusCode::NOT_FOUND,
        Json(serde_json::json!({
            "ok": false,
            "error": format!("未知接口: {method} {path}"),
        })),
    )
        .into_response()
}







#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::test_support::*;


    use axum::body::Body;
    use axum::http::Request as HttpRequest;
/// 贴纸包上传这条路的**路由层**：只有 admin 能调、body 上限确实被放开、垃圾字节是 400。
///
/// 三件事缺任何一条，前端看到都是"接口不存在或返回格式错误"：
/// ① 非 admin 必须 403（贴纸库是全局资产）；
/// ② `DefaultBodyLimit` 必须挂在**这一条**路由上 —— axum 默认只收 2 MB，
///    用一个 3 MiB 的体试探：被 413 掉就说明那层丢了（改回默认值/漏挂 layer 都会挂在这条）；
/// ③ 内容不是 zip 时给 400 与原因（不是 500，也不能当成成功）。
/// 正常解压的路径在 `vtrim-polish::assets::sticker_zip` 的 13 条单测里（它们用临时目录，
/// 不碰真实的贴纸库）；这里刻意只喂垃圾字节，免得往 `<exe_dir>/stickers/` 写真文件。
#[tokio::test]
async fn sticker_upload_route_is_admin_only_and_raises_body_limit() {
    let t = test_app().await;

    // ① 普通用户：403
    let hash = vtrim_service::auth::hash_password("pass-12345678").unwrap();
    t.auth.db().create_user("plain_upload", &hash, "user", false, 0).unwrap();
    let b = t.auth.login("plain_upload", "pass-12345678", Some("10.9.9.9"), None).unwrap();
    let res = t
        .call(
            t.req_as(&b.session_id, &b.csrf, "POST", "/admin/stickers/upload?name=zz_probe")
                .body(Body::from(b"not a zip".to_vec()))
                .unwrap(),
        )
        .await;
    assert_eq!(res.status(), StatusCode::FORBIDDEN, "非 admin 不能让上传落进全局贴纸库");

    // ③ 垃圾字节 → 400 且带原因
    let res = t
        .call(
            t.req("POST", "/admin/stickers/upload?name=zz_probe")
                .body(Body::from(b"definitely not a zip".to_vec()))
                .unwrap(),
        )
        .await;
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);
    let v = body_json(res).await;
    assert!(
        v["error"].as_str().unwrap_or("").contains("不是有效的 zip"),
        "{v}"
    );

    // ② 3 MiB（> axum 默认 2 MB）：必须走到 handler（400），不能是 413
    let big = vec![0u8; 3 * 1024 * 1024];
    let res = t
        .call(
            t.req("POST", "/admin/stickers/upload?name=zz_probe")
                .body(Body::from(big))
                .unwrap(),
        )
        .await;
    assert_eq!(
        res.status(),
        StatusCode::BAD_REQUEST,
        "上传路由必须放开 axum 的 2 MB 默认上限（413 = DefaultBodyLimit 那层丢了）"
    );

    // 包名不合法 / 缺名字：在解压之前就挡住
    for uri in ["/admin/stickers/upload?name=..%2Fx", "/admin/stickers/upload"] {
        let res = t
            .call(t.req("POST", uri).body(Body::from(vec![1u8, 2, 3])).unwrap())
            .await;
        assert_eq!(res.status(), StatusCode::BAD_REQUEST, "{uri}");
    }
    // 真实贴纸库里不该留下任何东西（三条失败路径都只能在暂存目录里动手，且必须清掉）
    let root = vtrim_core::path::sticker_dir();
    if root.is_dir() {
        let leftovers: Vec<String> = std::fs::read_dir(&root)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().to_string())
            .filter(|n| n.starts_with("zz_probe") || n.starts_with(".upload-"))
            .collect();
        assert!(leftovers.is_empty(), "上传失败不该留残留: {leftovers:?}");
    }
}

/// 字体库的三条**写**路由（P1-e）：只有 admin 能调、缺参数是 400。
///
/// 与贴纸那条同规矩：字体库是**程序级全局资产**（`<程序目录>/resources/fonts/`），
/// 非 admin 能动它就是"普通用户往服务器写文件"。这里刻意只走**失败路径**，
/// 不往真实字体库里写东西（成功路径用临时目录，在 `vtrim-core::fonts` 的 11 条单测里）。
#[tokio::test]
async fn font_write_routes_are_admin_only() {
    let t = test_app().await;

    // 普通用户：三条全 403（读是敞开的，写是 admin 的）
    let hash = vtrim_service::auth::hash_password("pass-12345678").unwrap();
    t.auth.db().create_user("plain_font", &hash, "user", false, 0).unwrap();
    let b = t
        .auth
        .login("plain_font", "pass-12345678", Some("10.9.9.8"), None)
        .unwrap();
    for (method, uri) in [
        ("POST", "/polish/fonts/import"),
        ("POST", "/polish/fonts/default"),
        // 安装自带字体**必须**仅 admin：它会往服务器磁盘写几十 MB
        // （字体已内嵌、**不联网**，但仍不能让普通用户触发"任意用户写大文件"）
        ("POST", "/polish/fonts/builtin"),
        // 上传（拖拽导入）：同样仅 admin —— 它是"任意用户往服务器写大文件"的另一个口子
        ("POST", "/polish/fonts/upload?name=x.ttf"),
        ("DELETE", "/polish/fonts?id=whatever"),
    ] {
        let res = t
            .call(
                t.req_as(&b.session_id, &b.csrf, method, uri)
                    .header("content-type", "application/json")
                    .body(Body::from(b"{}".to_vec()))
                    .unwrap(),
            )
            .await;
        assert_eq!(res.status(), StatusCode::FORBIDDEN, "{method} {uri} 必须仅 admin");
    }

    // admin + 缺参数：400（不是 500，也不能当成功）
    for (method, uri, body) in [
        ("POST", "/polish/fonts/import", "{}"),
        ("POST", "/polish/fonts/import", r#"{"dir":"F:\\x"}"#),
        ("POST", "/polish/fonts/default", "{}"),
        ("DELETE", "/polish/fonts", "{}"),
        // 上传缺 name：400（拿不到文件名就没法定 id，不能静默用一个随机名）
        ("POST", "/polish/fonts/upload", "{}"),
    ] {
        let res = t
            .call(
                t.req(method, uri)
                    .header("content-type", "application/json")
                    .body(Body::from(body.to_string().into_bytes()))
                    .unwrap(),
            )
            .await;
        assert_eq!(res.status(), StatusCode::BAD_REQUEST, "{method} {uri} {body}");
    }

    // 导入一个**不存在**的路径：400 且带原因（服务端读不到就说清楚，不是静默成功）
    let res = t
        .call(
            t.req("POST", "/polish/fonts/import")
                .header("content-type", "application/json")
                .body(Body::from(
                    br#"{"path":"F:\\__no_such_font_zz.ttf"}"#.to_vec(),
                ))
                .unwrap(),
        )
        .await;
    assert_eq!(res.status(), StatusCode::BAD_REQUEST, "不存在的字体文件应当是 400");
}

/// 未注册的 /api/* 必须返回 JSON 404：外层 fallback 是 SPA HTML，
/// 若让它兜住 API 请求，前端会把 200 HTML 解析成 null 并静默失败。
#[tokio::test]
async fn unknown_api_path_returns_json_404() {
    let t = test_app().await;
    let res = t
        .call(HttpRequest::builder().uri("/__nope").body(Body::empty()).unwrap())
        .await;
    assert_eq!(res.status(), StatusCode::NOT_FOUND);
    let ct = res
        .headers()
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    assert!(ct.contains("application/json"), "content-type = {ct}");
    let bytes = axum::body::to_bytes(res.into_body(), 64 * 1024).await.unwrap();
    let text = String::from_utf8_lossy(&bytes);
    assert!(text.contains("未知接口"), "body = {text}");
}

/// 前端实际调用的核心路由必须可达（避免再次出现"handler 写了但没注册"）。
#[tokio::test]
async fn core_routes_are_reachable() {
    let t = test_app().await;
    for (method, uri) in [("GET", "/health"), ("GET", "/jobs"), ("GET", "/config"), ("GET", "/models")] {
        let res = t.call(t.req(method, uri).body(Body::empty()).unwrap()).await;
        assert_eq!(res.status(), StatusCode::OK, "{method} {uri} 应为 200");
    }
}

/// 除白名单外，**未登录一律 401**（含 loopback：没有免登录分支，§3.5.1）。
#[tokio::test]
async fn unauthenticated_requests_are_401_except_whitelist() {
    let t = test_app().await;
    // 白名单：/health 必须公开（前端靠它判断服务是否活着）
    let res = t
        .call(HttpRequest::builder().uri("/health").body(Body::empty()).unwrap())
        .await;
    assert_eq!(res.status(), StatusCode::OK);
    let health = body_json(res).await;
    assert_eq!(health["listening"], "loopback");
    assert!(health.get("workspace").is_none(), "health 不得暴露任何路径: {health}");
    // 未认证接口不得泄露账号信息：账号数、是否仍用初始密码都只能用登录态或本地 doctor 看
    assert!(health.get("accounts").is_none(), "health 不得暴露账号数量: {health}");
    assert!(
        health.get("must_change_password").is_none(),
        "health 不得暴露「是否仍用初始密码」（等于给攻击者指路）: {health}"
    );

    for (method, uri) in [
        ("GET", "/me"),
        ("GET", "/jobs"),
        ("GET", "/config"),
        ("GET", "/models"),
        ("GET", "/project/list"),
        ("GET", "/project-data/status?config=vtrim.toml"),
        ("POST", "/polish/init"),
        // ---- Dhampir 预览（B1/B2/产物转发）----
        //
        // 这三条**必须**在这张表里：
        // * 前两条会**起进程**（调 Dhampir 二进制），未登录就能触发等于给了一个
        //   "匿名让人帮你渲片子"的入口；
        // * 第三条**把文件内容读出来**给请求方 —— 它要匿名可读，就等于片段目录里的
        //   产物对全网开放。前端那边是用 `fetch`(带 cookie) 取、再转 blob 的，
        //   所以它**不需要**在白名单里。
        ("POST", "/polish/dhampir-project"),
        ("GET", "/polish/dhampir-wasm/engine.js"),
    ] {
        let res = t
            .call(
                HttpRequest::builder()
                    .method(method)
                    .uri(uri)
                    .header(axum::http::header::CONTENT_TYPE, "application/json")
                    .body(Body::from("{}"))
                    .unwrap(),
            )
            .await;
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED, "{method} {uri} 未登录必须 401");
    }

    // 未知路径仍走 JSON 404（不因为没登录就变成 401/HTML）
    let res = t
        .call(HttpRequest::builder().uri("/does-not-exist").body(Body::empty()).unwrap())
        .await;
    assert_eq!(res.status(), StatusCode::NOT_FOUND);
}

/// 写请求缺 CSRF / CSRF 不匹配一律 403（cookie 会话 + 自定义头双提交）；
/// 带上正确 CSRF 后**还要**通过项目作用域（未登记的路径一律 404）。
#[tokio::test]
async fn write_requests_require_matching_csrf() {
    let t = test_app().await;
    let body = Body::from(r#"{"raw":""}"#);
    let res = t
        .call(
            HttpRequest::builder()
                .method("PUT")
                .uri("/project-data/config?project=1")
                .header(axum::http::header::COOKIE, t.cookie())
                .header(axum::http::header::CONTENT_TYPE, "application/json")
                .body(body)
                .unwrap(),
        )
        .await;
    assert_eq!(res.status(), StatusCode::FORBIDDEN, "缺 CSRF 头必须 403");

    let res = t
        .call(
            HttpRequest::builder()
                .method("PUT")
                .uri("/project-data/config?project=1")
                .header(axum::http::header::COOKIE, t.cookie())
                .header(vtrim_service::auth::CSRF_HEADER, "not-the-token")
                .header(axum::http::header::CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"raw":""}"#))
                .unwrap(),
        )
        .await;
    assert_eq!(res.status(), StatusCode::FORBIDDEN, "CSRF 不匹配必须 403");

    // 带上正确 CSRF 后才会走到业务校验（空 raw → 400）
    let res = t
        .call(
            t.req("PUT", "/project-data/config?project=1")
                .header(axum::http::header::CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"raw":""}"#))
                .unwrap(),
        )
        .await;
    assert_eq!(res.status(), StatusCode::BAD_REQUEST, "空内容必须被拒绝");
}

/// `awaiting_input` 链路（含 `/jobs/:id/decision`）已整体移除：
/// 该端点现在必须走 JSON 404 兜底，而不是把任务状态改成 running 再返回 200。
///
/// 注：`/models/download` 已去掉 `Json` 提取器（空 body 不再 400），
/// 但这里不直接调它——它会真的启动 ~1.5GB 模型下载。
#[tokio::test]
async fn removed_decision_endpoint_falls_back_to_json_404() {
    let t = test_app().await;
    let res = t
        .call(
            HttpRequest::builder()
                .method("POST")
                .uri("/jobs/does-not-exist/decision")
                .header(axum::http::header::CONTENT_TYPE, "application/json")
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await;
    assert_eq!(res.status(), StatusCode::NOT_FOUND);
    let bytes = axum::body::to_bytes(res.into_body(), 64 * 1024).await.unwrap();
    assert!(String::from_utf8_lossy(&bytes).contains("未知接口"));
}
}
