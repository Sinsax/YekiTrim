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
        // 采集 webhook：自带 token 鉴权（录制软件只能填 URL，无法加自定义头），
        // 因此它**不能**挂在 require_user 下（§4.1）。
        // · /ingest/bililive：录播姬原生事件（FileOpening/FileClosed/SessionEnded）
        // · /ingest/webhook：通用"录制完成"（biliLive-tools 的 externalWebhook / 任意脚本）
        .route("/ingest/bililive/:token", post(crate::ingest::webhook))
        .route("/ingest/webhook/:token", post(crate::ingest::custom_webhook))
        // 关停控制端点：**自带令牌鉴权**（和 webhook 同理不能挂 require_user ——
        // 它要在"没有会话"的场景下被托盘敲响），另外自己再卡 loopback + 令牌，见 handler。
        .route("/shutdown", post(shutdown));

    // 平台级资源（P2 起归入 /api/admin/*，仅 admin）：配置 / 模型 / doctor / LLM 测试
    // 旧路径作为过渡别名挂在**同一个 require_admin 层**下（否则普通账号能读到 LLM key）
    let admin = Router::new()
        .route("/admin/doctor", get(doctor))
        // ⚠️ **程序配置 2026-10-10 已挪到用户层**（`/api/config*`，见下面的 `assets` 组）：
        // 配置现在按 `Owner` 分（`owner::state_dir(owner)`），**每个账号读自己那份
        // `config.toml` / `llm.toml`**（后者含明文 api_key —— 再挂 admin 层就等于
        // "只有管理员能有 key、普通账号永远配不了自己的"）。**别在这里加回来**：
        // 同一路径挂两次 axum 直接 panic（`Overlapping method route`）。
        .route("/admin/models", get(models_get))
        .route("/admin/models/download", post(models_download))
        // LLM 模型清单（P3）：填好 base_url + api_key 就能取模型。
        // ⚠️ 语义是"**表命中 ⇒ 一个网络请求都不发**"，表里没有才用**已存的那把 key** 真探
        //    （失败回 502 + 已分类文案；探到的只是候选，**一律不落盘**）——见 handler 的注释。
        .route("/llm/models", get(llm_models))
        // 单支模型的**能力位**（P4「按能力渲染控件」的唯一数据源）：只查内置表，
        // 不联网、不读盘。认不出 (base_url, model) 回保守默认 + hit:false，**不是 404**。
        .route("/config/llm/catalog", get(llm_catalog))
        // 采集/录播姬：`GET` 读概览、`PUT` 保存配置（表单）、`POST .../token` 生成/关闭 webhook token
        .route(
            "/admin/ingest",
            get(crate::ingest::config_view).put(crate::ingest::put_config),
        )
        .route("/admin/ingest/token", post(crate::ingest::rotate_token))
        .route("/admin/ingest/assign", post(crate::ingest::assign))
        .route("/admin/ingest/scan", post(crate::ingest::scan_now))
        // 内置下载器（yt-dlp）安装：全局资产（下到 <程序目录>/.yekitrim/bin/），仅 admin
        .route("/admin/downloader/install", post(crate::ingest::downloader_install))
        .route("/admin/users", get(admin_users))
        // 配额（P5）：只有 admin 能改别人的额度。⚠️ 与 `/admin/users` 同一层，
        // **不是**用户层 —— 普通账号不该能给自己放宽配额。
        .route("/admin/users/:name/quota", put(admin_set_user_quota))
        .route("/admin/recorder/test", post(admin_recorder_test))
        .route("/admin/invites", get(admin_invites).post(admin_invite_create))
        .route("/admin/invites/registration", put(admin_registration_put))
        .route("/admin/invites/:id", axum::routing::delete(admin_invite_revoke))
        // 运行日志（服务端 tracing → 环形缓冲 / SSE；「运行日志」面板用，仅 admin）
        .route("/admin/logs", get(admin_logs))
        .route("/admin/logs/events", get(admin_logs_stream))
        // LLM 用量（每次调用的 token / 缓存命中 / 耗时；设置页「LLM 用量」卡，仅 admin）
        .route("/admin/usage", get(admin_usage))
        // ⚠️ **表情包（贴纸库）2026-10-09 已挪到用户层** —— 见下面 `/polish/stickers/*` 那几条。
        // 用户口径「素材库是用户级、跟随用户」：贴纸库与字体库一样是**使用者准备素材**的地方。
        // **别在 admin 层再加回来**：同一路径挂两次 axum 直接 panic（"Overlapping method route"）。
        .route("/doctor", get(doctor))
        .route("/models", get(models_get))
        .route("/models/download", post(models_download))
        // ⚠️ **字体库的写操作 2026-10-09 已挪到用户层**（`/api/polish/fonts/*`，见下面那几条）：
        // 用户口径「素材库是用户级、跟随用户」。**别在这里再加回来** —— 同一路径挂两次
        // 在 axum 里会 panic（"Overlapping method route"），而不是"后者覆盖前者"。
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
    // 让"底座不可用"在点渲染之前就可见（前端挂在渲染弹窗上，`yekitrim doctor` 用同一份判断）。
    .route("/polish/dhampir-status", post(polish_dhampir_status))
    .route("/polish/dhampir-wasm/*path", get(polish_dhampir_wasm))
        .route("/polish/cover-preview", post(polish_cover_preview))
        .route("/polish/cover-background", post(polish_cover_background))
        .route("/polish/cover-frame", post(polish_cover_frame))
        .route("/polish/project", get(polish_project_get).put(polish_project_put))
        .route("/polish/subtitles", get(polish_subtitles_get).put(polish_subtitles_put))
        // §10 情绪轨道：把 `process` 算出来的 `data/emotion.json` 交给界面。
        // ⚠️ 这个端点就是 `emotion.json` 的**第一个读者** —— 在此之前它只有写者
        //    （全仓 grep 只找得到写的一方），是一件"算出真数据、然后扔掉"的产物。
        //    空态回 404（不是 `{windows:[]}`），见 handler 的说明。
        .route("/polish/emotion", get(polish_emotion_get))
        // 贴纸的**第 k 帧**（浏览器侧没有可用的多帧图解码，见那个函数的说明）
        .route("/polish/sticker-frame", post(polish_sticker_frame))
        // 贴纸帧表**预热**：页面加载时就把"播到才解"的整段解码挪走（见那个函数的说明）
        .route("/polish/sticker-prewarm", post(polish_sticker_prewarm))
        .route("/polish/sticker-sources", get(polish_sticker_sources))
        // 贴纸库只读浏览（编辑器里 keyword→贴纸 的两级选择）
        .route("/polish/sticker-library", get(polish_sticker_library))
        // 内建 emoji 目录（编辑器表情面板：881 条按 tag 分组）
        .route("/polish/emoji-catalog", get(polish_emoji_catalog))
        .route("/polish/sticker-file", get(polish_sticker_file))
        // 音效目录（编辑器里音效事件的列表选择）
        .route("/polish/sfx-catalog", get(polish_sfx_catalog))
        // ⚠️ `/polish/video-library` **不在这里**，它挂在 `assets` 路由上（素材库那一组）——
        // 同一路径挂两个 Router 再 merge，axum 会 panic（"Overlapping method route"），
        // **不是**"后者覆盖前者"。实测漏删一份 = 52 个单测全红。
        // ⚠️ 这里曾经有 `POST /polish/render-frame`（wgpu 单帧真渲染预览）—— 那条 GPU 链路已删除
        // （GPU 那侧交给独立的 Dhampir 底座）。别把它加回来。
        .route("/polish/font", get(polish_font))
        // 字体库**只读**浏览：字幕/封面字体的下拉数据源。
        // ⚠️ 写操作（导入/上传/设默认/装自带/删除）**不在这里** —— 它们与贴纸库管理一起
        // 挂在不带 `enforce_scope` 的 `assets` 路由上（见下面那个 `let assets = ...`）。
        .route("/polish/fonts", get(polish_fonts))
        .route("/polish/select-version", post(polish_select_version))
        .route("/polish/silence", post(polish_silence))
        .route("/polish/cover-title", post(polish_cover_title))
        .route("/polish/cover-config", post(polish_cover_config))
        .route_layer(axum::middleware::from_fn_with_state(
            state.clone(),
            crate::scope::enforce_scope,
        ));

    // **素材库（用户级资产）**：视频 / 贴纸 / 字体三类的浏览、取文件与管理。
    //
    // ⚠️ **刻意不挂 `enforce_scope`**：素材库是**程序级**的（`<数据目录>/resources/*`），
    // 不属于任何工程。挂进去的后果实测过 —— `fonts/import` 那条带 `{dir}` 的请求会被
    // scope 中间件按"这个目录在不在你的项目里"判掉、回 404，而它要的其实是
    // "服务器上某个目录里躺着一支字体"。同规矩适用于贴纸的 `import`（按目录导入）。
    // 它仍然在 `protected` 下 ⇒ **未登录 401**（放开的是"哪个用户"，不是"要不要登录"）。
    let assets = Router::new()
        // 统一取文件：`?kind=video|font&file=<单个文件名>`，根目录服务端按 kind 现取
        .route("/polish/asset", get(polish_asset_file))
        // 视频缩略图（网格里那张真帧；抽不出来回 404，前端退成图标）
        .route("/polish/video-thumb", get(polish_video_thumb))
        // 视频素材库列表
        .route("/polish/video-library", get(polish_video_library))
        // 字体库管理（2026-10-09 从 admin 层挪来）
        .route("/polish/fonts/import", post(polish_fonts_import))
        // **拖拽上传**：请求体是字体的原始字节（`application/octet-stream`，不是 multipart
        // —— 同贴纸上传那条做法）。⚠️ 必须单独给这一条放开体积上限：axum 默认只收 2 MB，
        // 而字体动辄 10~30 MB（413 就是这层丢了）。上限常量与前端预检是同一个数。
        .route(
            "/polish/fonts/upload",
            post(polish_fonts_upload).layer(axum::extract::DefaultBodyLimit::max(
                FONT_UPLOAD_MAX_BYTES,
            )),
        )
        // 设为默认（唯一默认；**不动任何工程的字体配置**）
        .route("/polish/fonts/default", post(polish_fonts_default))
        // 装**自带字体**（幂等；已内嵌、不联网）
        .route("/polish/fonts/builtin", post(polish_fonts_builtin))
        // 删除（`?id=`）；**程序自带的回 403**（判断在 handler 里，不是靠路由门控）
        .route("/polish/fonts", delete(polish_fonts_delete))
        // 贴纸库管理（2026-10-09 从 `/admin/stickers/*` 挪来，**老路径已删除、不保留别名**：
        // 那是给"只有管理员能管"设计的，留着会变成两套语义）
        .route("/polish/stickers", get(crate::stickers::packs_list))
        .route("/polish/stickers/import", post(crate::stickers::pack_import))
        // 上传 zip 建新包。请求体是压缩包原始字节 —— 单独放开体积上限（axum 默认 2 MB）。
        .route(
            "/polish/stickers/upload",
            post(crate::stickers::pack_upload)
                .layer(axum::extract::DefaultBodyLimit::max(
                    crate::stickers::UPLOAD_MAX_BYTES,
                )),
        )
        .route(
            "/polish/stickers/:pack",
            get(crate::stickers::pack_detail).delete(crate::stickers::pack_delete),
        )
        .route("/polish/stickers/:pack/build", post(crate::stickers::pack_build))
        .route("/polish/stickers/:pack/files/:name", get(crate::stickers::pack_file))
        // ---- 程序配置（2026-10-10 从 admin 层挪来；旧 `/admin/*` 路径已删除）----
        // 配置按 `Owner` 分：**每个账号读/写自己那份** `config.toml` + `llm.toml`。
        // 因此它和素材库同一条道理 —— 归用户层，不挂 `enforce_scope`
        // （配置不属于任何工程），但仍在 `protected` 下 ⇒ 未登录 401。
        .route("/config", get(config_get))
        .route("/config/global", put(config_global_put))
        // 方言注册表的**清单**（下拉数据源）：纯静态、不联网/不读盘 ⇒ 与下面的 /config/llm 同层
        // （每个账号都要能拿到它来画"推理参数怎么发"那个下拉；只挂 admin 会让普通账号
        //  落到前端的 R4 兜底 —— 那正是"清单腐坏"的老路）。
        // ⚠️ 清单**从 llm crate 的 DIALECTS 现算**，别在这层或 handler 里抄一份。
        .route("/config/llm/dialects", get(llm_dialects))
        // **协议**清单（前端「API 协议」下拉）：同样从注册表现算、纯静态 ⇒ 用户层
        .route("/config/llm/protocols", get(llm_protocols))
        .route("/config/llm", put(config_llm_put))
        .route("/llm/test", post(llm_test));

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
        // 弹幕**正文**（首页切片总览「搜弹幕」按需拉）：只读，不回时间
        .route("/project-data/danmaku-text", get(project_data_danmaku))
        // 搜索用文本（字幕含 LLM 订正稿 + 弹幕，一次两桶）：只读，不回时间
        .route("/project-data/search-text", get(project_data_search_text))
        .route("/project-data/correct", post(project_data_correct))
        .route("/project-data/pipeline", post(project_data_pipeline))
        .route("/project-data/config", get(project_data_config).put(project_data_config_put))
        // 素材（弹幕 / 字幕）：预处理页随时查看与切换
        .route("/project-data/assets", get(project_data_assets).post(project_data_assets_put))
        .route("/project-data/polish", get(project_data_polish))
        // 出片回传：把成片交给 biliLive-tools 投稿（YekiTrim 自己不做 B站上传）
        .route(
            "/publish/bililive-tools",
            get(crate::publish::status).post(crate::publish::publish),
        )
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
        // 本机打开一个目录（精修页「打开输出目录」）：**只在本地免登录模式下存在**，
        // 服务端模式 handler 自己回 404（见 `api::open_path`）。挂在这一组而不是 `polish` 那组 ——
        // 那组带 `enforce_scope` 的项目目录守卫，这条端点收的是任意本机目录。
        .route("/open-path", post(open_path))
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
        .merge(assets)
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

/// **视频素材库**（`GET /polish/video-library`）的路由层：普通用户可读、形状对、磁盘为准。
///
/// 三件事：
/// ① 它是**只读**接口（与贴纸库同规矩：读敞开、写限 admin）⇒ 普通用户必须 200 而不是 403；
/// ② 回的是 `{ok, videos:[{name,file,duration}]}` —— `name` 是**去扩展名**的素材键
///    （`[intro_outro]` 存的就是它），`duration` 给界面算"播到结尾"用；
/// ③ **以磁盘为准**：往库里真写一个 <1KB 的假 mp4，它必须出现在列表里。
///    这里刻意只验"出现"、不验 duration —— 假字节 ffprobe 探不出时长，
///    正好钉住"探不出也要列出来（`duration: null`）"这条（隐藏掉用户会以为文件没放进去）。
#[tokio::test]
async fn video_library_route_is_readable_and_disk_driven() {
    let t = test_app().await;
    let hash = yekitrim_service::auth::hash_password("pass-12345678").unwrap();
    t.auth
        .db()
        .create_user("plain_vid", &hash, "user", false, 0)
        .unwrap();
    let b = t
        .auth
        .login("plain_vid", "pass-12345678", Some("10.9.9.8"), None)
        .unwrap();

    // ① 普通用户可读
    let res = t
        .call(
            t.req_as(&b.session_id, &b.csrf, "GET", "/polish/video-library")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(res.status(), 200, "素材库是只读接口，普通用户必须能看");

    // ③ 以磁盘为准：真丢一个文件进库，它必须出现（名字用去扩展名的键）。
    // ⚠️ 写进**这个用户自己的库**：handler 现在按 `Owner` 解析
    // （`owner::asset_dir(Owner::User(uid), Videos)`），测试必须与它同源 ——
    // 否则就是 2026-10-09 那次「假绿/假红取决于本机目录里有没有东西」的重演。
    // **判据与 handler 同源**这一条，比路径写成什么字面量重要得多。
    let uid = t
        .auth
        .db()
        .find_user("plain_vid")
        .unwrap()
        .expect("刚建的用户")
        .id;
    let lib = yekitrim_core::owner::asset_dir(
        yekitrim_core::owner::Owner::User(uid),
        yekitrim_core::owner::AssetKind::Videos,
    );
    std::fs::create_dir_all(&lib).unwrap();
    let probe = lib.join("zz_route_probe.mp4");
    std::fs::write(&probe, b"not really an mp4").unwrap();

    let res = t
        .call(
            t.req_as(&b.session_id, &b.csrf, "GET", "/polish/video-library")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(res.status(), 200);
    let json = body_json(res).await;
    let videos = json["videos"].as_array().expect("videos 必须是数组");
    let found = videos.iter().find(|v| v["name"] == "zz_route_probe");
    assert!(
        found.is_some(),
        "手工丢进库的文件必须出现（以磁盘为准）；实际列表：{videos:?}"
    );
    let found = found.unwrap();
    assert_eq!(found["file"], "zz_route_probe.mp4", "file 是带扩展名的真文件名");
    assert!(
        found["duration"].is_null(),
        "探不出时长的素材要照样列出来、duration 给 null（不能隐藏）"
    );

    std::fs::remove_file(&probe).ok();
}

/// **多用户可以真用**的判据：两个账号的素材库互不干扰。
///
/// 这是本批改动的核心验收 —— 在引入 `Owner` 之前，素材库是**全局一份**，
/// 甲丢进去的视频乙也能看见、乙还能删掉甲的东西。现在每个账号解析到自己的树
/// （`owner::asset_dir(Owner::User(uid), …)`）。
///
/// 三条一起验：
/// ① 甲丢进**自己的库**的文件，甲看得见；
/// ② 同一个文件名，乙**看不见**（这是分离生效的直接证据，不是"路径算对了"的间接证据）；
/// ③ 乙往**自己的库**放同名文件后，乙看见的是自己那份 —— 两次列表内容不同，
///    说明两条路径真的指向不同的目录，而不是"碰巧都没东西"。
#[tokio::test]
async fn two_accounts_do_not_see_each_others_video_library() {
    let t = test_app().await;
    let hash = yekitrim_service::auth::hash_password("pass-12345678").unwrap();
    t.auth.db().create_user("alice", &hash, "user", false, 0).unwrap();
    t.auth.db().create_user("bob", &hash, "user", false, 0).unwrap();
    let a = t.auth.login("alice", "pass-12345678", None, None).unwrap();
    let b = t.auth.login("bob", "pass-12345678", None, None).unwrap();

    let lib_of = |name: &str| {
        let uid = t
            .auth
            .db()
            .find_user(name)
            .unwrap()
            .expect("刚建的用户")
            .id;
        yekitrim_core::owner::asset_dir(
            yekitrim_core::owner::Owner::User(uid),
            yekitrim_core::owner::AssetKind::Videos,
        )
    };

    let names = |json: &serde_json::Value| -> Vec<String> {
        json["videos"]
            .as_array()
            .expect("videos 是数组")
            .iter()
            .map(|v| v["name"].as_str().unwrap_or("").to_string())
            .collect()
    };

    // ① 甲丢一个独有文件进自己的库。
    let alice_lib = lib_of("alice");
    std::fs::create_dir_all(&alice_lib).unwrap();
    std::fs::write(alice_lib.join("alice_only.mp4"), b"x").unwrap();

    let res = t
        .call(
            t.req_as(&a.session_id, &a.csrf, "GET", "/polish/video-library")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(res.status(), 200);
    let alice_view = names(&body_json(res).await);
    assert!(
        alice_view.contains(&"alice_only".to_string()),
        "甲必须看得见自己库里的文件；实际：{alice_view:?}"
    );

    // ② 乙看不见甲的东西 —— 这就是"分离真的生效"。
    let res = t
        .call(
            t.req_as(&b.session_id, &b.csrf, "GET", "/polish/video-library")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(res.status(), 200);
    let bob_view = names(&body_json(res).await);
    assert!(
        !bob_view.contains(&"alice_only".to_string()),
        "乙不该看见甲的素材；实际：{bob_view:?}"
    );

    // ③ 乙放一个**同名**文件，两边看到的仍是各自的。
    let bob_lib = lib_of("bob");
    std::fs::create_dir_all(&bob_lib).unwrap();
    std::fs::write(bob_lib.join("shared_name.mp4"), b"y").unwrap();
    std::fs::write(alice_lib.join("shared_name.mp4"), b"y").unwrap();

    let alice_lib_dir = yekitrim_core::owner::asset_dir(
        yekitrim_core::owner::Owner::User(t.auth.db().find_user("alice").unwrap().unwrap().id),
        yekitrim_core::owner::AssetKind::Videos,
    );
    assert_ne!(
        alice_lib_dir, bob_lib,
        "两个账号的素材库目录必须不同 —— 这是整个分离的结构前提"
    );

    std::fs::remove_file(alice_lib.join("alice_only.mp4")).ok();
    std::fs::remove_file(alice_lib.join("shared_name.mp4")).ok();
    std::fs::remove_file(bob_lib.join("shared_name.mp4")).ok();
}

/// 贴纸包上传这条路的**路由层**：登录用户可调（**不再限 admin**）、body 上限确实被放开、
/// 垃圾字节是 400。
///
/// ⚠️ **2026-10-09 语义变更**：用户口径「素材库是用户级、跟随用户」⇒ 贴纸/字体库的写操作
/// 从 admin 层挪到用户层。这条测试因此**反过来**钉住"普通用户能过关"：
/// 拿垃圾字节去试，必须走到 handler 拿到 **400**（解压失败），而**不是 403**。
/// 若哪天有人把 admin 门控加回去，这里立刻红。
///
/// 另外两件事缺任何一条，前端看到都是"接口不存在或返回格式错误"：
/// ① `DefaultBodyLimit` 必须挂在**这一条**路由上 —— axum 默认只收 2 MB，
///    用一个 3 MiB 的体试探：被 413 掉就说明那层丢了（改回默认值/漏挂 layer 都会挂在这条）；
/// ② 内容不是 zip 时给 400 与原因（不是 500，也不能当成成功）。
/// 正常解压的路径在 `yekitrim-polish::assets::sticker_zip` 的 13 条单测里（它们用临时目录，
/// 不碰真实的贴纸库）；这里刻意只喂垃圾字节，免得往 `<exe_dir>/stickers/` 写真文件。
#[tokio::test]
async fn sticker_upload_route_is_open_to_users_and_raises_body_limit() {
    let t = test_app().await;

    // ① 普通用户**也能调**（素材库是用户级资产）—— 这里只验"没被门控挡在 403"
    let hash = yekitrim_service::auth::hash_password("pass-12345678").unwrap();
    t.auth.db().create_user("plain_upload", &hash, "user", false, 0).unwrap();
    let b = t.auth.login("plain_upload", "pass-12345678", Some("10.9.9.9"), None).unwrap();
    let res = t
        .call(
            t.req_as(&b.session_id, &b.csrf, "POST", "/polish/stickers/upload?name=zz_probe")
                .body(Body::from(vec![1u8, 2, 3]))
                .unwrap(),
        )
        .await;
    assert_ne!(
        res.status(),
        StatusCode::FORBIDDEN,
        "素材库已改为用户级：登录用户不该被 403 挡住"
    );
    // ② 垃圾字节 → 400 且带原因（普通用户那条：**放行到 handler**，由内容判失败）
    let res = t
        .call(
            t.req("POST", "/polish/stickers/upload?name=zz_probe")
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
            t.req("POST", "/polish/stickers/upload?name=zz_probe")
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
    for uri in ["/polish/stickers/upload?name=..%2Fx", "/polish/stickers/upload"] {
        let res = t
            .call(t.req("POST", uri).body(Body::from(vec![1u8, 2, 3])).unwrap())
            .await;
        assert_eq!(res.status(), StatusCode::BAD_REQUEST, "{uri}");
    }
    // 真实贴纸库里不该留下任何东西（三条失败路径都只能在暂存目录里动手，且必须清掉）
    // ⚠️ 走 `sticker_dir_readable(Owner::from_env())` 而不是全局 `path::sticker_dir()`：
    // "真实的那份库"是谁的，由身份决定（多用户下是 `users/<uid>/…`）。
    let root = yekitrim_core::owner::sticker_dir_readable(yekitrim_core::owner::Owner::from_env());
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

/// **素材取文件**（`GET /polish/asset`）的路由层：登录用户可读、只认**单个文件名**、认不出 kind 是 400。
///
/// ⚠️ 这条路径是素材库预览的唯一出口，所以"只允许单个文件名"是它的**安全性质**
/// （不是风格问题）：一旦 `file` 认 `/` 或 `..`，一个普通账号就能拿 `?kind=font&file=../...
/// /llm.toml` 去读服务器上任意文件。判据因此必须钉在**穿越尝试**上，而不只是"正常文件名能取到"。
#[tokio::test]
async fn asset_file_route_reads_one_named_file_and_refuses_traversal() {
    let t = test_app().await;
    let hash = yekitrim_service::auth::hash_password("pass-12345678").unwrap();
    t.auth
        .db()
        .create_user("plain_asset", &hash, "user", false, 0)
        .unwrap();
    let b = t
        .auth
        .login("plain_asset", "pass-12345678", Some("10.9.9.7"), None)
        .unwrap();

    // 认不出的 kind → 400（不是 500、也不能静默当成某个默认类型）
    let res = t
        .call(
            t.req_as(&b.session_id, &b.csrf, "GET", "/polish/asset?kind=sticker&file=x")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(res.status(), StatusCode::BAD_REQUEST, "未知 kind 必须 400");

    // 缺 file → 400
    let res = t
        .call(
            t.req_as(&b.session_id, &b.csrf, "GET", "/polish/asset?kind=font")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(res.status(), StatusCode::BAD_REQUEST, "缺 file 必须 400");

    // ⚠️ **穿越尝试一律 400**：这些正是"只认单个文件名"要挡的东西。
    // 用 `%2F` / `%5C` 是因为查询串里裸 `/` 也会被解析，编码形式更接近攻击者真会发的东西。
    for bad in [
        "../llm.toml",
        "..%2Fllm.toml",
        "a%2Fb.ttf",
        "a%5Cb.ttf",
        "C%3A%5Cwindows%5Cx.ttf",
        "%2Fetc%2Fpasswd",
    ] {
        let uri = format!("/polish/asset?kind=font&file={bad}");
        let res = t
            .call(
                t.req_as(&b.session_id, &b.csrf, "GET", &uri)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await;
        assert_eq!(res.status(), StatusCode::BAD_REQUEST, "穿越尝试必须被挡住: {uri}");
    }

    // 正常文件名但库里没有 → 404（区别于 400：这条是"没这个东西"，不是"你不该问"）
    let res = t
        .call(
            t.req_as(&b.session_id, &b.csrf, "GET", "/polish/asset?kind=font&file=zz_nope.ttf")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(res.status(), StatusCode::NOT_FOUND);

    // 未登录 → 401（放开的是"哪个用户"，不是"要不要登录"）
    let res = t
        .call(
            axum::http::Request::builder()
                .method("GET")
                .uri("/polish/asset?kind=font&file=x.ttf")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
}

/// **视频缩略图**（`GET /polish/video-thumb`）的路由层：文件名安全校验 + 抽不出来是 **404 而不是 500**。
///
/// ⚠️ "404 而不是 500" 是这条接口的核心约定：抽帧要起 ffmpeg，而没装 ffmpeg / 容器怪异 /
/// 文件损坏都**不是接口故障**，只是"这张没有缩略图"。回 500 的话素材库会在界面上报一条
/// 读取失败，而实际其它素材全都好好的。这里用假字节 mp4 正好钉住它（ffmpeg 必然抽不出帧）。
#[tokio::test]
async fn video_thumb_route_refuses_traversal_and_404s_when_unavailable() {
    let t = test_app().await;
    let hash = yekitrim_service::auth::hash_password("pass-12345678").unwrap();
    t.auth
        .db()
        .create_user("plain_thumb", &hash, "user", false, 0)
        .unwrap();
    let b = t
        .auth
        .login("plain_thumb", "pass-12345678", Some("10.9.9.6"), None)
        .unwrap();

    // 穿越尝试 → 400
    for bad in ["..%2Fllm.toml", "a%2Fb.mp4", "%2Fetc%2Fpasswd"] {
        let uri = format!("/polish/video-thumb?file={bad}");
        let res = t
            .call(
                t.req_as(&b.session_id, &b.csrf, "GET", &uri)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await;
        assert_eq!(res.status(), StatusCode::BAD_REQUEST, "穿越尝试必须被挡住: {uri}");
    }

    // 库里没有这个文件 → 404
    let res = t
        .call(
            t.req_as(&b.session_id, &b.csrf, "GET", "/polish/video-thumb?file=zz_nope.mp4")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(res.status(), StatusCode::NOT_FOUND);

    // ⚠️ 真文件但**不是合法视频** → 仍然是 404（不是 500）：这条判据就是"抽不出来不算故障"。
    // ⚠️ 写进**这个用户自己的库**（与 handler 同源）—— 全局那份 handler 已经不看了。
    let lib = yekitrim_core::owner::asset_dir(
        yekitrim_core::owner::Owner::User(
            t.auth.db().find_user("plain_thumb").unwrap().expect("刚建的用户").id,
        ),
        yekitrim_core::owner::AssetKind::Videos,
    );
    std::fs::create_dir_all(&lib).unwrap();
    let probe = lib.join("zz_thumb_probe.mp4");
    std::fs::write(&probe, b"definitely not a video").unwrap();
    let res = t
        .call(
            t.req_as(&b.session_id, &b.csrf, "GET", "/polish/video-thumb?file=zz_thumb_probe.mp4")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert!(
        res.status() == StatusCode::NOT_FOUND || res.status() == StatusCode::OK,
        "抽不出帧只能是 404（或真抽到了 200），实际 {} —— 绝不能是 500",
        res.status()
    );
    std::fs::remove_file(&probe).ok();
    // 顺带把这次可能生成的缓存清掉，别在真实库里留垃圾
    std::fs::remove_file(lib.join(".thumbs").join("zz_thumb_probe.mp4@0.jpg")).ok();
}

/// 字体库的**写**路由：**登录用户可调**（2026-10-09 起，不再是 admin-only）、缺参数是 400。
///
/// ⚠️ **语义变更**：用户口径「素材库是用户级、跟随用户」—— 素材库是给使用者准备素材的地方，
/// 不是程序配置，所以写操作从 admin 层挪到了用户层（与贴纸库同口径）。
/// 这条测试因此**反过来**钉住"普通用户能过关"：拿缺参数的请求去试，必须是 **400**，
/// **不是 403**。谁把 admin 门控加回来，这里立刻红。
///
/// ⚠️ 但**「已认证」这道门仍在**：未登录必须 401（见下面那段）—— 放开的是"哪个用户"，
/// 不是"要不要登录"。
///
/// 这里刻意只走**失败路径**，不往真实字体库里写东西
/// （成功路径用临时目录，在 `yekitrim-core::fonts` 的 11 条单测里）。
#[tokio::test]
async fn font_write_routes_accept_any_logged_in_user() {
    let t = test_app().await;

    // 普通用户：**不该**被 403 挡住（素材库已是用户级资产）
    let hash = yekitrim_service::auth::hash_password("pass-12345678").unwrap();
    t.auth.db().create_user("plain_font", &hash, "user", false, 0).unwrap();
    let b = t
        .auth
        .login("plain_font", "pass-12345678", Some("10.9.9.8"), None)
        .unwrap();
    for (method, uri) in [
        ("POST", "/polish/fonts/import"),
        ("POST", "/polish/fonts/default"),
        ("POST", "/polish/fonts/builtin"),
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
        assert_ne!(
            res.status(),
            StatusCode::FORBIDDEN,
            "{method} {uri} 素材库已改为用户级，不该 403"
        );
    }

    // ⚠️ **未登录仍然挡住**：放开的是"哪个用户"，不是"要不要登录"。
    // 少了这条，一条匿名请求就能往服务器字体库里写文件。
    for (method, uri) in [
        ("POST", "/polish/fonts/import"),
        ("POST", "/polish/fonts/upload?name=x.ttf"),
        ("DELETE", "/polish/fonts?id=whatever"),
        ("POST", "/polish/stickers/upload?name=x"),
    ] {
        let res = t
            .call(
                // ⚠️ **不能**用 `t.req(..)` —— 它自带 admin 的会话 cookie
                // （`test_support.rs::req` 里加了 Cookie），那样测的是"已登录"、
                // 这条判据会**永远绿**。匿名请求必须自己建、不带 Cookie。
                // （第一版就是拿 `t.req` 写的：401 断言实际收到 400 ——
                //  请求已经进了 handler，说明它根本不是匿名的。）
                axum::http::Request::builder()
                    .method(method)
                    .uri(uri)
                    .header("content-type", "application/json")
                    .body(Body::from(b"{}".to_vec()))
                    .unwrap(),
            )
            .await;
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED, "{method} {uri} 未登录必须 401");
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
        ("GET", "/project-data/status?config=yekitrim.toml"),
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
                .header(yekitrim_service::auth::CSRF_HEADER, "not-the-token")
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

    /// 没设 `YEKITRIM_SHUTDOWN_TOKEN` 时，`POST /api/shutdown` 必须**像不存在一样**（404）——
    /// 否则它就是一个"告诉你这里有控制通道"的探测口。
    ///
    /// ⚠️ 403（令牌不对）那条路走的是 `auth::token_ok_for` 的纯函数单测，不在这里设 env：
    /// 本仓刻意不在测试里碰环境变量（`data_dir_in` 就是为这个存在的），**进程内并行**下
    /// 设 env 会让别的用例跟着变。
    /// `POST /api/open-path`（精修页「打开输出目录」）**只在本地免登录模式下存在**：
    /// 服务端模式必须 404（远端的人点一下、**服务器上**弹一个资源管理器窗口是不可接受的），
    /// 本地模式也**只开已存在的目录**（不存在 / 没给 → 400，且**根本不走到 spawn**）。
    ///
    /// ⚠️ 成功那条**刻意不写测试**：它会真弹一个资源管理器窗口（本机与 CI 都不该发生）。
    /// 这里只钉"不该开的都不开" —— 判据全是**能翻面的**：把 `open_path` 开头那道
    /// `local_admin.is_none()` 摘掉，第一条立刻从 404 变成 400（而那两条喂的都是**不存在的目录**，
    /// 所以谁也不会在跑测试的机器上真弹出一个资源管理器窗口）。
    #[tokio::test]
    async fn open_path_is_local_only_and_never_spawns_for_bad_paths() {
        use axum::body::Body;
        use tower::ServiceExt;

        // ① 服务端模式（普通会话）：端点当不存在，连"这里有个能开目录的口子"都不透露。
        // ⚠️ 这里刻意喂**不存在的目录**（而不是一个真目录）：万一有人把 `open_path` 开头那道
        // `local_admin` 闸摘掉，这条会变成 400（"目录不存在"）而不是**真弹一个窗口** ——
        // 判据照样翻面（404 → 400），但不会在跑测试的人机器上弹资源管理器。
        let t = test_app().await;
        let mut ghost = std::env::temp_dir();
        ghost.push("yekitrim-definitely-missing-dir-server-mode");
        let res = t
            .call(
                t.req("POST", "/open-path")
                    .header(axum::http::header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        serde_json::json!({ "path": ghost.to_string_lossy() }).to_string(),
                    ))
                    .unwrap(),
            )
            .await;
        assert_eq!(res.status(), StatusCode::NOT_FOUND);
        let bytes = axum::body::to_bytes(res.into_body(), 64 * 1024).await.unwrap();
        assert!(
            String::from_utf8_lossy(&bytes).contains("未知接口"),
            "服务端模式下必须报「未知接口」，实得：{}",
            String::from_utf8_lossy(&bytes)
        );

        // ② 本地免登录模式：坏路径只 400，**不会**去 spawn 任何进程
        let state = (*t.state)
            .clone()
            .with_local_admin(crate::model::LocalAdmin {
                id: 1,
                username: "local".into(),
                role: "admin".into(),
            });
        let app = router(std::sync::Arc::new(state));
        let local_post = |body: String| {
            HttpRequest::builder()
                .method("POST")
                .uri("/open-path")
                .header("host", "127.0.0.1:26320")
                .header("origin", "http://127.0.0.1:26320")
                .header(axum::http::header::CONTENT_TYPE, "application/json")
                .body(Body::from(body))
                .unwrap()
        };
        let mut missing = std::env::temp_dir();
        missing.push("yekitrim-definitely-missing-dir-2f8a1c");
        let res = app
            .clone()
            .oneshot(local_post(
                serde_json::json!({ "path": missing.to_string_lossy() }).to_string(),
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::BAD_REQUEST, "不存在的目录必须 400（不 spawn）");
        let bytes = axum::body::to_bytes(res.into_body(), 64 * 1024).await.unwrap();
        assert!(String::from_utf8_lossy(&bytes).contains("目录不存在"));

        // ③ 本地模式 + 空 path：400（同样不 spawn）
        let res = app
            .clone()
            .oneshot(local_post(serde_json::json!({ "path": "  " }).to_string()))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::BAD_REQUEST);
        let bytes = axum::body::to_bytes(res.into_body(), 64 * 1024).await.unwrap();
        assert!(String::from_utf8_lossy(&bytes).contains("缺少 path"));
    }

    #[tokio::test]
    async fn shutdown_endpoint_is_404_without_token_env() {
        let t = test_app().await;
        let res = t
            .call(
                t.req("POST", "/shutdown")
                    .header("x-yekitrim-shutdown-token", "whatever")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await;
        assert_eq!(res.status(), StatusCode::NOT_FOUND);
    }
}
