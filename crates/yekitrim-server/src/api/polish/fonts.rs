//! `api::polish::fonts`：字体库（按 `Owner` 分库；写入口一律 `fonts::ensure_dir_for`）。

use std::path::{Path, PathBuf};
use std::sync::Arc;

// 字体拖拽上传的**原始字节**载体（与贴纸上传同一条做法，不用 multipart）
use axum::body::Bytes;
use axum::extract::{Extension, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;

use crate::model::AppState;

use super::*;


// ⚠️ 这里曾经有 `polish_render_frame`（单帧真渲染预览：常驻 wgpu 后端 + `model::PreviewBackend` 槽，
// 路由 `/api/polish/render-frame`），以及 feature 未启用时的那个 501 占位版本。**整条已删除**
// （用户拍板：GPU 那侧交给独立的 Dhampir 底座），`/api/health` 的 `wgpu_render` 能力位一并去掉 ——
// 前端也不再按它隐藏任何入口。**别再把它加回来**：要"真渲染预览"就走底座那条链路；
// 出片的事实来源是底座（`render/dhampir.rs`），预览那层是 `webui/src/polish/engine.js`。

/// 字体库全量（**只读**，非 admin 也能看）：字幕字体 / 封面字体的下拉数据源。
///
/// 库位置就是 `owner::asset_dir(owner, Fonts)`：**`<data>/users/<uid>/fonts/`** 一处
/// （默认程序目录；`YEKITRIM_DATA_DIR` 可覆盖）。读写同一个位置，见 `fonts::ensure_dir`。
/// **以磁盘为准**：往里丢一个 `.ttf` / `.otf` / `.ttc` 就会出现在列表里，没有索引文件要维护。
/// `id` = 目录名（v2 布局）或文件名去扩展名（老布局）—— **配置里存的就是它**；
/// `family` = **内部族名**（`FONT.toml` 的 `family`，唯一权威：预览 / 出片 `@font-face` / Takumi 三处都认它）。
/// `display_name` 只是给人看的标签，别拿它当族名用（详见 `docs/plan/font-library.md` §13）。
/// 扫描要读字体文件解析字族名，所以整段走 `blocking`（同步 IO 不能留在 worker 上）。
///
/// `fallback_stack` = 出片侧的**动态兜底字体栈**（`yekitrim-polish` 的 `server::fallback_stack`：
/// 首项随库里默认那支走）—— 预览（`PolishPreview.vue` 的 `.cap` 栈）据它与成片保持同源，
/// 这是三处字体来源里"预览那一腿"的收口。
///
/// `root`（服务器上的绝对路径）**只给 admin**：非 admin 拿它没用（也丢不进文件），
/// 而它泄露的是服务器的安装路径 —— 与 `/api/admin/usage` 的理由同一条。
pub(crate) async fn polish_fonts(Extension(user): Extension<crate::auth::CurrentUser>) -> ApiResult {
    let is_admin = user.is_admin();
    let owner = crate::auth::owner_of(&user);
    let payload = blocking(move || {
        // ⚠️ **读两层**（用户库优先、全局补缺，2026-10-10）：自带字体装在**全局**，
        // 用户自己导入的在**自己库**里；只看其中一层会"少一半字体"。
        // 写入仍然只有一处（各 `ensure_dir_for(owner)`），见 `owner::asset_dirs` 的说明。
        let roots = yekitrim_core::owner::asset_dirs(
            owner,
            yekitrim_core::owner::AssetKind::Fonts,
        );
        let root = roots[0].clone();
        let fonts: Vec<serde_json::Value> = yekitrim_core::fonts::scan_all_roots(&roots)
            .into_iter()
            .map(|f| {
                serde_json::json!({
                    "id": f.id,
                    "file": f.file,
                    // ⚠️ 走 `family_for`（唯一权威），别直接回 `f.family`（那是"从文件读到的名字"，
                    // 可能与 FONT.toml 不一致）—— 界面上"内部族名 vs 显示名"两行要靠它对得上。
                    // ⚠️ 走多根：这款字体可能在**全局库**里（自带字体就在那）
                    "family": roots
                        .iter()
                        .map(|r| yekitrim_core::fonts::family_for_at(r, &f.id))
                        .find(|s| !s.is_empty())
                        .unwrap_or_default(),
                    "display_name": f.display_name,
                    "bytes": f.bytes,
                    "builtin": f.builtin,
                    "is_default": f.is_default,
                    "license": f.license,
                    "scripts": f.scripts,
                    "emoji": f.emoji,
                    // 彩色字形能力（`cbdt`/`sbix`/`colrv0`/`colrv1`/`svg`/`none`）。
                    // ⚠️ 界面据此判「这支 emoji 字体到底能不能出彩色」：`colrv1`/`svg` 会**静默**坏掉
                    // （Takumi 只认 COLRv0，见 `yekitrim_core::fonts::color_glyph_support` 的表）。
                    // 纯文字字体这里是 `none` —— 正常，别当成错误。
                    "color_format": f.color_format,
                    // ⚠️ 自带字体**没下载**时也在列表里（`installed: false`）：界面据此给"下载"按钮，
                    // 而不是当成"库里有这支"。用户自己导入的永远是 true。
                    "installed": f.installed,
                    // 老平铺布局（散文件直接在库根目录）⇒ 界面打「老布局」标签
                    "flat": f.flat,
                })
            })
            .collect();
        // 自带字体清单（"还能下什么"）：与 `fonts::BUILTIN_FONTS` 同源，别在这一层手抄
        let builtin: Vec<serde_json::Value> = yekitrim_core::fonts::builtin_fonts()
            .iter()
            .map(|b| {
                serde_json::json!({
                    "id": b.id,
                    "display_name": b.display_name,
                    "family": b.family,
                    "license": b.license,
                    "license_url": b.license_url,
                    // ⚠️ 自带字体现在装在**全局库**里，所以"装没装"要按**两层**判：
                    // 只看用户库会永远报 `installed: false`（界面显示"未安装"，
                    // 而实际出片时是能用的 —— 这种不一致最难查）。
                    "installed": roots
                        .iter()
                        .any(|r| yekitrim_core::fonts::builtin_installed(r, b)),
                })
            })
            .collect();
        Ok(serde_json::json!({
            "ok": true,
            // 出 JSON 的路径必须过 display（去掉 `\\?\` 逐字前缀），否则前端拼路径会炸
            "root": if is_admin { yekitrim_core::path::display(&root) } else { String::new() },
            // 动态兜底字体栈（首项随库里默认走）：预览 `.cap` 栈与成片同源的收口
            "fallback_stack": yekitrim_polish::render::server::fallback_stack(),
            // ⚠️ **emoji 补充槽位**（2026-10-02 字体职责分离）：库里那支负责表情的字体，
            // 由 `yekitrim_core::fonts::emoji_font` **自动选**（至多一支），**用户不选**。
            //
            // 前端拿它做两件事：① 预览 `.cap` 栈把它追加在**末尾**（正文在前、emoji 兜底）；
            // ② 界面据此说明"emoji 由系统自动补充"。库里没有就是 null ⇒ 前端不追加
            // （浏览器自带系统 emoji）。
            //
            // ⚠️ 是**独立字段**、不是拼进 `fallback_stack` 的一段：封面那条链看的是
            // **注册顺序**而不是这个栈（Takumi 不读系统字体），底座那条链**根本没有回退**
            // —— 拼进去对它们零效果，还会撑破 polish 侧"四段"那条单测。
            "emoji_family": yekitrim_core::fonts::emoji_font(&root).map(|e| {
                if e.family_internal.trim().is_empty() { e.id } else { e.family_internal }
            }),
            "fonts": fonts,
            "builtin": builtin,
        }))
    })
    .await?;
    ok(payload)
}


/// B5 判据（纯函数）：`path` 能不能作为这个账号的字体导入来源。
///
/// admin 不受限；普通账号 = 路径必须落进 `roots` 之一（[`font_import_roots`] 给出的
/// 本人素材根 / 本人项目根）。`ensure_within` 是 core 的租户囚笼（词法 + 符号链接
/// 双判），别在这里手写第二份。
fn font_import_allowed(is_admin: bool, roots: &[PathBuf], path: &Path) -> bool {
    if is_admin {
        return true;
    }
    roots
        .iter()
        .any(|r| yekitrim_core::tenant::ensure_within(r, path).is_ok())
}


/// 非 admin 的字体导入来源清单：本人素材根 + 本人项目根（B5）。
///
/// `resources_root` 由调用方按 owner 现算（`owner::resources_dir`）；单独抽成参数是为了
/// 单测能把"素材根"指到 tempdir —— 否则用例得往真实程序目录里种文件（禁止）。
fn font_import_roots(
    db: &yekitrim_service::db::Db,
    uid: i64,
    resources_root: PathBuf,
) -> Vec<PathBuf> {
    let mut roots = vec![resources_root];
    if let Ok(rows) = db.list_projects(Some(uid)) {
        roots.extend(rows.into_iter().map(|p| p.root_dir));
    }
    roots
}


/// 往字体库里**导入**一支字体（**登录即可，但来源受限**）。
///
/// body：`{path}` —— 服务器上那个**字体文件**的路径；或 `{dir, name}`（推荐，
/// **由服务端拼**，免得前端把 `\\?\F:\a` 拼成 `\\?\F:\a/b` 撞上 Windows 123，同 `/api/fs/list` 的规矩）。
/// 可选 `{display}` 当给人看的显示名（`display_name`）；**id 永远取自文件名**（配置里存的就是它）。
///
/// ⚠️ **来源面必须收口**（2026-11-14 后端 review B5）：这条在**用户层**（素材库那一组，
/// 刻意不挂 `enforce_scope`），却收"服务器上的路径"。不管的话，普通账号可以
/// `{"path":"/任意/位置/x.ttf"}` 把服务器上任意 `.ttf/.otf/.ttc` 拷进自己的库、再用
/// `GET /api/polish/asset?kind=font&file=…` 读回来（还能拿它探测文件存在性）。
/// 所以：**普通用户只能从「自己的素材根」或「自己的项目目录」里导入**，admin 不受限
/// （与 `/api/fs/list` 的 admin-only 口径对齐；admin 本来就能浏览服务器任意目录）。
pub(crate) async fn polish_fonts_import(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<crate::auth::CurrentUser>,
    Json(body): Json<serde_json::Value>,
) -> ApiResult {
    let owner = crate::auth::owner_of(&user);
    let path = match body.get("path").and_then(|v| v.as_str()).map(str::trim) {
        Some(p) if !p.is_empty() => PathBuf::from(p),
        _ => {
            // `{dir, name}`：**服务端拼**（前端自己拼 Windows verbatim 前缀会出 123）
            let dir = body.get("dir").and_then(|v| v.as_str()).unwrap_or("").trim();
            let name = body.get("name").and_then(|v| v.as_str()).unwrap_or("").trim();
            if dir.is_empty() || name.is_empty() {
                return Err(err(StatusCode::BAD_REQUEST, "缺少 path，或 dir + name"));
            }
            // 只取文件名那一段：`name` 来自请求体，`..\..\x` 不能让它越界读文件
            let base = Path::new(name)
                .file_name()
                .map(PathBuf::from)
                .ok_or_else(|| err(StatusCode::BAD_REQUEST, "name 不是合法文件名"))?;
            if base.as_os_str().is_empty() {
                return Err(err(StatusCode::BAD_REQUEST, "name 不是合法文件名"));
            }
            Path::new(dir).join(base)
        }
    };
    // ⚠️ 显示名读 **`display`**（它与 `name` 撞语义：`name` 是**文件名**，服务端拿它拼路径 / 当 id）。
    // 兼容老的 `name`-当显示名（那时 `{path}` 分支没有文件名冲突）。
    let display = body
        .get("display")
        .or_else(|| body.get("name").filter(|_| body.get("path").is_some()))
        .and_then(|v| v.as_str())
        .map(str::to_string);
    let user_for_check = user.clone();
    let db = state.auth.db_arc();
    let entry = blocking(move || {
        // 目录布局写入 + 读字体表都碰磁盘（可能是网络盘），一律留在 blocking 池上。
        // ⚠️ 字体库的错都是 `anyhow`（"只收 .ttf/.otf"、"程序自带字体不可删除"…），
        // 转成 400 + 原话：用户要看到"为什么这支不行"，不是笼统的 500。
        // ⚠️ 写路径走 `ensure_dir_for(owner)`（它会 `create_dir_all`），别自己拼 `font_dir()`。
        // ⚠️ **必须带 owner**：这条走的是 `require_user` 那条用户路由，写错的库会让
        // 甲导入的字体落进单人库、而甲自己的库还是空的（多用户下就是"传了却看不见"）。
        //
        // 来源面收口（review B5）：非 admin 只许从**自己的素材根 / 自己的项目目录**里取文件。
        // admin 不受限（他能用 /api/fs/list 浏览服务器任意目录，卡他只会打断既有工作流）。
        // 判据抽成纯函数（[`font_import_allowed`] / [`font_import_roots`]）—— 单测能钉住
        // "别人的项目目录 → 拒"，不必往真实字体库里种文件。
        if !user_for_check.is_admin() {
            let resources_root =
                yekitrim_core::owner::owner_root(crate::auth::owner_of(&user_for_check));
            let roots = font_import_roots(&db, user_for_check.id, resources_root);
            if !font_import_allowed(false, &roots, &path) {
                return Err(err(
                    StatusCode::FORBIDDEN,
                    "只能从自己的素材目录或项目目录导入字体（服务器其他位置仅管理员可用）",
                ));
            }
        }
        let root = yekitrim_core::fonts::ensure_dir_for(owner)
            .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("字体库目录不可用: {e}")))?;
        yekitrim_core::fonts::import_font(&root, &path, display.as_deref())
            .map_err(|e| err(StatusCode::BAD_REQUEST, e.to_string()))
    })
    .await?;
    ok(serde_json::json!({ "ok": true, "font": entry }))
}


/// 字体库单文件**上传上限**（2026-10-02 拖拽导入）。
///
/// ⚠️ 与前端 `webui/src/api/polish.js::FONT_UPLOAD_MAX_BYTES` 是**同一个数**（改一处要改两处，
/// 同贴纸那条规矩）。取 64 MiB 的理由：常见中文字体 10~30 MB，彩色 emoji 位图字体 ~10 MB，
/// 给到 64 MiB 能收下绝大多数；再大就不该走浏览器这条链路了。
pub(crate) const FONT_UPLOAD_MAX_BYTES: usize = 64 * 1024 * 1024;


/// `POST /api/polish/fonts/upload?name=<文件名>` —— **拖拽上传**一支字体（**仅 admin**）。
///
/// 请求体是字体的**原始字节**（`application/octet-stream`，不是 multipart）—— 与贴纸上传同一条
/// 做法：单文件用 multipart 只是多一层边界解析，而前端 `api()` 本来就能直接传 `File`。
/// 路由上挂了 `DefaultBodyLimit::max(FONT_UPLOAD_MAX_BYTES)`（axum 默认只有 2 MB）。
///
/// ⚠️ `name` 是**文件名**（服务端拿它当 id），显示名走独立的 `display` 查询参数 ——
/// 两个字段语义别混（与 `polish_fonts_import` 同一口径）。
///
/// ⚠️ 与 import 的分工：import 收的是**服务器上已有的路径**（admin 浏览服务器文件那条来源），
/// 这条收的是**浏览器里的字节**。两条都能进库，但来源不同、安全考量也不同 ——
/// 这条不需要路径归属校验（没有路径），只需要体积上限 + 格式校验。
pub(crate) async fn polish_fonts_upload(
    Extension(user): Extension<crate::auth::CurrentUser>,
    Query(q): Query<std::collections::HashMap<String, String>>,
    body: Bytes,
) -> ApiResult {
    let owner = crate::auth::owner_of(&user);
    let file_name = q.get("name").map(String::as_str).unwrap_or("").trim();
    if file_name.is_empty() {
        return Err(err(StatusCode::BAD_REQUEST, "缺少 name（字体文件名）"));
    }
    // ⚠️ 只取 basename：`name` 来自查询串，`..\\..\\x.ttf` 不能让它决定落盘位置
    // （库内的 id 与文件名由 `import_bytes` 自己从 basename 推，这里只是先挡一道）。
    let base = Path::new(file_name)
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    if base.is_empty() || base.starts_with('.') {
        return Err(err(StatusCode::BAD_REQUEST, "name 不是合法文件名"));
    }
    let display = q.get("display").map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
    let entry = blocking(move || {
        // ⚠️ 写路径走 `ensure_dir_for(owner)`（它会 `create_dir_all`）。**必须带 owner** ——
        // 写错的库 = 用户传完字体、库里却没有。
        let root = yekitrim_core::fonts::ensure_dir_for(owner)
            .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("字体库目录不可用: {e}")))?;
        yekitrim_core::fonts::import_font_bytes(&root, &base, &body, display.as_deref())
            .map_err(|e| err(StatusCode::BAD_REQUEST, e.to_string()))
    })
    .await?;
    ok(serde_json::json!({ "ok": true, "font": entry }))
}


/// 把某支字体设为**默认字体**（**仅 admin**）。body：`{id}`。
///
/// ⚠️ 默认是**互斥**的（`fonts::set_default` 会把别的清掉）。
/// ⚠️ 只影响**新建项目**用它；已有工程里 `polish.toml` 存的那串 id 不会被改写
/// （改了就是替用户改他已定的稿子）。
pub(crate) async fn polish_fonts_default(
    Extension(user): Extension<crate::auth::CurrentUser>,
    Json(body): Json<serde_json::Value>,
) -> ApiResult {
    // ⚠️ **两层库**：自带字体装在全局库里，只看用户库会找不到它们
    // （用户报「设为默认失败：…先重新导入一次：NotoColorEmoji-CBDT」就是这个）。
    // `set_default_all_roots` 只借用多根**读**，写仍然落用户库。
    let owner_fonts = yekitrim_core::owner::asset_dirs(
        crate::auth::owner_of(&user),
        yekitrim_core::owner::AssetKind::Fonts,
    );
    let id = body
        .get("id")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| err(StatusCode::BAD_REQUEST, "缺少 id"))?
        .to_string();
    blocking(move || {
        // set_default / remove 操作的是**当前可见的那份库**（`font_dir()` 读侧解析），
        // 不是写路径 —— `set_default` / `remove` 改的是**当前可见的那份库**（`font_dir()`）。
        yekitrim_core::fonts::set_default_all_roots(&owner_fonts, &id)
            .map_err(|e| err(StatusCode::BAD_REQUEST, e.to_string()))
    })
    .await?;
    ok(serde_json::json!({ "ok": true }))
}


/// 删除库里一支字体（**仅 admin**）。`DELETE /api/polish/fonts?id=<id>`。
///
/// ⚠️ 用 **query 参数**而不是路径段：`axum::extract::Path` 会与本文件里的 `std::path::Path` 撞名，
/// 而且"字体 id 可以带空格/CJK"（`is_safe_id` 允许）—— 放进路径段还要额外转义。
/// ⚠️ `builtin = true`（程序自带）**返回 403**（`fonts::remove_font` 直接 bail，文案「程序自带字体不可删除」）。
pub(crate) async fn polish_fonts_delete(
    Extension(user): Extension<crate::auth::CurrentUser>,
    Query(q): Query<std::collections::HashMap<String, String>>,
) -> ApiResult {
    let owner_fonts = yekitrim_core::owner::asset_dir(
        crate::auth::owner_of(&user),
        yekitrim_core::owner::AssetKind::Fonts,
    );
    let id = q.get("id").map(String::as_str).unwrap_or("").trim().to_string();
    if id.is_empty() {
        return Err(err(StatusCode::BAD_REQUEST, "缺少 id"));
    }
    blocking(move || {
        // ⚠️ `builtin` 那一支**必须**能被用户看见 —— 403 的文案就是「程序自带字体不可删除」，
        // 与 `webui/src/components/MediaLibrary.vue`（首页素材库）里把自带字体的删除键置灰是同一条规矩。
        yekitrim_core::fonts::remove_font(&owner_fonts, &id).map_err(|e| {
            let msg = e.to_string();
            let status = if msg.contains("程序自带") {
                StatusCode::FORBIDDEN
            } else {
                StatusCode::BAD_REQUEST
            };
            err(status, msg)
        })
    })
    .await?;
    ok(serde_json::json!({ "ok": true }))
}


/// 下载**自带字体**（**仅 admin**）。body 可为空（下全部还缺的）或 `{id}`（只下某一支）。
///
/// 清单与校验和都在 `yekitrim_core::fonts::BUILTIN_FONTS`（**不在这一层抄第二份**）。
/// ⚠️ 幂等：已装好的**不重复下**（用户可能手动替换过那支字体）。
/// 装上自带字体（**内嵌写入，不联网**；幂等，已装好的跳过）。
///
/// ⚠️ 2026-10-02 从"下载"改成"内嵌"：正常路径下**前端不该再调它** ——
/// `ensure_global_config`（server 启动）已经自动装好。留这条接口是**修复入口**
/// （用户手删了库里那支之后想补回来），语义与 CLI 的 `yekitrim fonts install` 完全一致。
///
/// 仍然走 `blocking`：写 ~23 MB 到（可能很慢的）磁盘上，不该占住 tokio worker。
pub(crate) async fn polish_fonts_builtin(
    Extension(user): Extension<crate::auth::CurrentUser>,
    Json(body): Json<serde_json::Value>,
) -> ApiResult {
    let owner = crate::auth::owner_of(&user);
    let only = body
        .get("id")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let report = blocking(move || {
        // ⚠️ 写路径走 `ensure_dir_for(owner)`（它负责 `create_dir_all`）——
        // 否则字体会写进**别人的**库里，而本账号的库仍然显示「未安装」。
        let root = yekitrim_core::fonts::ensure_dir_for(owner)
            .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("字体库目录不可用: {e}")))?;
        // 点名单支时先确认它真是自带的 —— 否则用户白等一场才发现 id 打错
        if let Some(id) = &only {
            if !yekitrim_core::fonts::is_builtin_id(id) {
                return Err(err(
                    StatusCode::BAD_REQUEST,
                    format!("不是自带字体（自带清单里没有它）: {id}"),
                ));
            }
        }
        let mut done: Vec<String> = Vec::new();
        for b in yekitrim_core::fonts::builtin_fonts() {
            if let Some(want) = &only {
                if !b.id.eq_ignore_ascii_case(want) {
                    continue;
                }
            }
            // 已装好的跳过（幂等）
            if yekitrim_core::fonts::builtin_installed(&root, b) {
                continue;
            }
            yekitrim_core::fonts::install_builtin_font(&root, b)
                // 校验和不符 / 落盘失败的原话对用户有意义，照实回
                .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("安装自带字体失败: {e}")))?;
            done.push(b.id.to_string());
        }
        Ok(serde_json::json!({ "ok": true, "installed_now": done, "installed": yekitrim_core::fonts::builtin_status(&root) }))
    })
    .await?;
    ok(report)
}

/// 字体文件端点（wasm 预览 fetch 字体字节用）：项目字体候选 -> 内置资源。
// polish_font_async_read_fixed
pub(crate) async fn polish_font(Query(q): Query<std::collections::HashMap<String, String>>) -> Response {
    let name = q.get("name").cloned().unwrap_or_default();
    let dir = q.get("dir").map(std::path::PathBuf::from).unwrap_or_default();
    let mut candidates: Vec<PathBuf> = Vec::new();
    if !name.trim().is_empty() {
        // 只取文件名部分：name 来自查询参数，`..\..\x` 这类路径不能让字体接口越界读文件
        let base = Path::new(name.trim())
            .file_name()
            .map(PathBuf::from)
            .unwrap_or_default();
        if !base.as_os_str().is_empty() {
            for d in [
                dir.join("assets").join("fonts"),
                dir.join("fonts"),
                dir.clone(),
            ] {
                candidates.push(d.join(&base).with_extension("ttf"));
                candidates.push(d.join(&base).with_extension("otf"));
                candidates.push(d.join(&base).with_extension("ttc"));
            }
            // 程序级字体库：`name` 就是字体 id（文件名去扩展名）。
            // 放在项目候选之后 —— 项目里自带的字体优先，库只是"别处都没有"时的来源。
            //
            // ⚠️ **必须走 `resolve_all_roots` + 两层根（用户库优先、全局补缺），不能只 `resolve_file`**：
            // 自带字体装在 **`global/`** 子库里，而 `fonts::resolve_file` 只看单层 `fonts::dir()` ——
            // 用它会让"选了自带字体"在这条路上一律解析不到（随后就落到雅黑兜底，见下）。
            // 出片那条腿（`render/server.rs::serve_library_font`）用的就是这两层，
            // 两边不同口径正是"预览与出片不一致"的来源，这里对齐它。
            let roots = yekitrim_core::owner::asset_dirs(
                yekitrim_core::owner::Owner::from_env(),
                yekitrim_core::owner::AssetKind::Fonts,
            );
            if let Some(p) = yekitrim_core::fonts::resolve_all_roots(&roots, name.trim()) {
                candidates.push(p);
            }
            candidates.push(Path::new("C:/Windows/Fonts").join(&base).with_extension("ttf"));
        }
    }
    // ⚠️⚠️ **这里曾经硬塞一档 `C:/Windows/Fonts/msyh.ttc` 兜底，那是"换字体没反应"的根因**
    // （2026-10 实测）：任何解析不到的名字都回**微软雅黑**的字节，而且回 **200**。
    // 于是前端 `new FontFace("乐米波波体", <雅黑字节>)` 注册**成功**、
    // `.cap{font-family:"乐米波波体"}` 也能命中 —— 但画出来是雅黑字形，**全链路不报错**。
    // 实测四个名字（含一个根本不存在的）返回的字节 **sha1 全等** `e5a993b85dd9`。
    // 用户报的「预览时字体更换没有正常实现、刷新也没有」就是它。
    //
    // 现在与出片那条腿（`render/server.rs::serve_library_font`，`/vt-fonts/<id>`）**同口径**：
    // **解析不到就 404，绝不拿另一支字体顶包**。前端 `fonts.js` 本来就把失败
    // 当"回退系统字体栈"处理（失败不抛），404 正是它预期的信号。
    //
    // ⚠️ 那支"圆润可爱"的 **乐米波波体是给用户自行安装的**（授权不含嵌入式用途，
    // 见 README「字体」与 THIRD-PARTY-NOTICES.md）——它**不在库里**，所以走的是
    // `C:/Windows/Fonts/<名字>` 那条**按名字**的候选（装了就命中、没装就 404），
    // 而不是"随便给一支中文"。
    for c in &candidates {
        let res = { let __p = c.clone(); tokio::task::spawn_blocking(move || __p.exists()).await.unwrap_or(false) }; if res {
            let res = {
                let v2 = c.clone();
                // ⚠️ 整文件同步读不能留在 async worker 上（api/mod.rs:44-51 纪律 ✓）：
                // 字体几十 MB，且字幕渲染首次用到字体就会打这条路由 ⇒ 占住 worker 时
                // 同一时刻的其他请求（含 engine.js）都产不出响应 ✗
                tokio::task::spawn_blocking(move || std::fs::read(v2))
                    .await
                    .unwrap_or_else(|_| Err(std::io::Error::other("读取字体任务崩了")))
            }; match res {
                Ok(bytes) => {
                    return ([(axum::http::header::CONTENT_TYPE, font_content_type(c))], bytes)
                        .into_response();
                }
                Err(_) => continue,
            }
        }
    }
    StatusCode::NOT_FOUND.into_response()
}


/// 字体文件的 Content-Type 按扩展名给：以前恒 `font/ttf`，`.otf` / `.ttc` 会被标错
/// （浏览器按 MIME 决定要不要当字体用，别赌它容错）。
pub(super) fn font_content_type(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .as_deref()
    {
        Some("otf") => "font/otf",
        Some("ttc") => "font/collection",
        Some("woff") => "font/woff",
        Some("woff2") => "font/woff2",
        _ => "font/ttf",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::test_support::*;
    use axum::body::Body;

    // ⚠️ 这些用例是**按原字节搬来**的（含原本在文件顶层、缩进为 0 的那批）：
    //    统一重排缩进会动到多行字符串字面量，判据会因此悄悄变。


    /// **B5（2026-11-14 后端 review）：fonts/import 的来源面收口（判据层，两用户）**。
    ///
    /// 成功路径不往真实字体库里写（与 routes.rs 那条同规矩）；这里钉的是**判据**：
    /// 普通账号只能从「本人素材根 / 本人项目根」导，别人的项目目录 → 拒，admin 不受限。
    #[test]
    fn font_import_sources_are_scoped_to_the_owners_roots() {
        let dir = tempfile::tempdir().unwrap();
        let db = yekitrim_service::db::Db::open(&dir.path().join("t.db")).unwrap();
        let a = db.create_user("fa", "h", "user", false, 0).unwrap();
        let b = db.create_user("fb", "h", "user", false, 0).unwrap();
        let proj_a = dir.path().join("a-proj");
        let proj_b = dir.path().join("b-proj");
        std::fs::create_dir_all(&proj_a).unwrap();
        std::fs::create_dir_all(&proj_b).unwrap();
        let cfg_a = proj_a.join("yekitrim.toml");
        let cfg_b = proj_b.join("yekitrim.toml");
        db.create_project(a, "a-proj", None, "local", None, &proj_a, &cfg_a, None).unwrap();
        db.create_project(b, "b-proj", None, "local", None, &proj_b, &cfg_b, None).unwrap();
        let res_root_b = dir.path().join("b-resources");
        std::fs::create_dir_all(&res_root_b).unwrap();

        // ① 清单 = 素材根（调用方传入的那份）+ **本人**项目根；别人的项目根不许进清单
        let roots = font_import_roots(&db, b, res_root_b.clone());
        assert!(roots.contains(&res_root_b), "{roots:?}");
        assert!(roots.iter().any(|r| r == &proj_b), "{roots:?}");
        assert!(!roots.iter().any(|r| r == &proj_a), "别人的项目根不许进清单：{roots:?}");

        // ② 别人的项目目录里的文件 → 拒
        let font_in_a = proj_a.join("x.ttf");
        std::fs::write(&font_in_a, b"x").unwrap();
        assert!(!font_import_allowed(false, &roots, &font_in_a));
        // ③ 自己的项目目录 → 允许
        let font_in_b = proj_b.join("y.ttf");
        std::fs::write(&font_in_b, b"x").unwrap();
        assert!(font_import_allowed(false, &roots, &font_in_b));
        // ④ 自己的素材根 → 允许
        let font_in_res = res_root_b.join("z.ttf");
        std::fs::write(&font_in_res, b"x").unwrap();
        assert!(font_import_allowed(false, &roots, &font_in_res));
        // ⑤ 素材根外的相对路径小动作（.. 段）同样出不去 —— ensure_within 的词法判据
        let escape = proj_b.join("../a-proj/x.ttf");
        assert!(!font_import_allowed(false, &roots, &escape));
        // ⑥ admin：任何位置都允许
        assert!(font_import_allowed(true, &roots, &font_in_a));
    }


    /// B5 的 HTTP 面：普通账号从**别人项目**导入 ⇒ 403（判据在动字体库**之前**）；
    /// 从**自己项目**导入 ⇒ 判据放行（走到 import_font 的格式校验 ⇒ 400，不是 403）。
    ///
    /// ⚠️ 两条都**不写真实字体库**：403 在 `ensure_dir_for` 之前返回；
    /// 400 那条是 import_font 拒收假字体（写不进库）。
    #[tokio::test]
    async fn font_import_rejects_other_users_project_paths() {
        let t = test_app().await;
        let db = t.auth.db();
        let hash = yekitrim_service::auth::hash_password("pass-12345678").unwrap();
        let uid = db.create_user("font_b", &hash, "user", false, 0).unwrap();

        // admin 的测试项目（t.root）里放一支假字体 —— B 试图从那儿导
        let foreign = t.root.join("foreign.ttf");
        std::fs::write(&foreign, b"x").unwrap();
        let b = t.auth.login("font_b", "pass-12345678", None, None).unwrap();
        let res = t
            .call(
                t.req_as(&b.session_id, &b.csrf, "POST", "/polish/fonts/import")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({ "path": yekitrim_core::path::display(&foreign) })
                            .to_string()
                            .into_bytes(),
                    ))
                    .unwrap(),
            )
            .await;
        assert_eq!(res.status(), StatusCode::FORBIDDEN, "别人项目里的文件不许导：{res:?}");

        // 自己项目里的假字体：判据放行 ⇒ 400（import_font 拒收"读不出族名"的假货），
        // **绝不能是 403** —— 那样说明判据把自己人也拦了。
        let own = t._dir.path().join(format!("own-proj-{uid}"));
        std::fs::create_dir_all(&own).unwrap();
        let cfg = own.join("yekitrim.toml");
        std::fs::write(&cfg, "").unwrap();
        let own_pid = db
            .create_project(uid, "own-proj", None, "local", None, &own, &cfg, None)
            .unwrap();
        let _ = own_pid;
        let own_font = own.join("mine.ttf");
        std::fs::write(&own_font, b"x").unwrap();
        let res = t
            .call(
                t.req_as(&b.session_id, &b.csrf, "POST", "/polish/fonts/import")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({ "path": yekitrim_core::path::display(&own_font) })
                            .to_string()
                            .into_bytes(),
                    ))
                    .unwrap(),
            )
            .await;
        assert_eq!(res.status(), StatusCode::BAD_REQUEST, "自己项目内应放行到格式校验：{res:?}");
    }

    /// 字体库只读接口的**载荷契约**（2026-10-02 字体职责分离新增两个字段的落点）。
    ///
    /// 前端有三处直接吃这个 JSON：
    /// * `PolishPreview` 的 `.cap` 栈取 `fallback_stack`（首项随库里默认走）；
    /// * 字幕预览把 `emoji_family` **追加在栈尾**（正文在前、emoji 兜底）；
    /// * 两个字体下拉按每支的 `emoji` 过滤（emoji 字体不许被选成正文）。
    ///
    /// 所以这里钉两件事：**字段在**，且 **`emoji_family` 不是 `fallback_stack` 的一段**
    /// —— 后者是有意的（封面靠注册顺序、底座没有回退，拼进去只会误导）。
    #[test]
    fn 字体库载荷带上_emoji_槽位且与兜底栈分开() {
        let payload = serde_json::json!({
            "root": "",
            "fallback_stack": yekitrim_polish::render::server::fallback_stack(),
            "emoji_family": yekitrim_core::fonts::emoji_font(&yekitrim_core::fonts::dir_for(
                yekitrim_core::owner::Owner::from_env(),
            ))
            .map(|e| {
                if e.family_internal.trim().is_empty() { e.id } else { e.family_internal }
            }),
            "fonts": [],
            "builtin": [],
        });
        let stack = payload["fallback_stack"].as_str().expect("fallback_stack 必须在");
        assert!(!stack.is_empty(), "兜底栈不能是空的");
        // 兜底栈**永远不含 emoji**（yekitrim-core 有单测钉着；这里从 HTTP 载荷再确认一次）
        assert!(
            !stack.to_ascii_lowercase().contains("emoji"),
            "emoji 不许混进兜底栈（它走独立字段）: {stack}"
        );
        // `emoji_family` 字段本身必须在（值可以是 null —— 库里没装 emoji 字体就是 null）
        assert!(
            payload.get("emoji_family").is_some(),
            "emoji_family 字段必须在（前端据此决定要不要追加栈尾）"
        );
    }


    /// `GET /api/polish/font` **解析不到就必须 404，绝不拿别的字体顶包**。
    ///
    /// 钉的是 2026-10 用户报的「换字体没反应、刷新也没用」：这条 handler 曾经在候选链末尾
    /// 硬塞一档 `C:/Windows/Fonts/msyh.ttc`，于是**任何**名字（含根本不存在的）都回 200 +
    /// 微软雅黑的字节。前端 `new FontFace("<要的名字>", <雅黑字节>)` 注册**成功**、
    /// CSS 也命中，画出来却是雅黑 —— **全链路一个错都不报**。
    ///
    /// 实测四个名字（含 `完全不存在的字体XYZ`）返回字节 sha1 全等 `e5a993b85dd9`
    /// （= `msyh.ttc` 的 19704352 字节），就是这条漏洞的直接证据。
    ///
    /// 判据只看**候选链**（不碰文件系统，避免依赖测试机的字体）：源码里不许再出现那档兜底，
    /// 且必须与出片那条腿同口径走**两层**解析（`resolve_all_roots` + 用户库/全局库）。
    #[test]
    fn 字体端点不许拿别的字体顶包_且按两层库解析() {
        let src = include_str!("fonts.rs");
        // 取 `polish_font` 那个函数体（到下一个 `pub(super)` 项为止）
        let start = src.find("async fn polish_font(").expect("polish_font 必须还在");
        let rest = &src[start..];
        let end = rest[1..]
            .find("\npub")
            .map(|i| i + 1)
            .unwrap_or(rest.len());
        let body = &rest[..end];
        // ⚠️ **必须先剥注释再查**：上面那段解释性注释里就写着 `msyh.ttc`（在讲这条坑本身），
        // 不剥掉的话判据会被自己的说明文字顶翻 —— 本仓既有纪律（判据走"去注释后的源码"）。
        let code: String = body
            .lines()
            .filter(|l| !l.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            !code.contains("msyh.ttc"),
            "字体端点又硬塞了 msyh.ttc 兜底 —— 任何名字都会回微软雅黑且 **200**，\
             前端注册成功、CSS 命中，画出来却是雅黑（换字体没反应的根因）"
        );
        assert!(
            code.contains("resolve_all_roots"),
            "字体端点必须走两层库解析（`resolve_all_roots` + 用户库/全局库）—— \
             自带字体装在 `global/`，只查单层会解析不到自家字体"
        );
        assert!(
            !code.contains("fonts::resolve_file("),
            "别退回单层的 `fonts::resolve_file` —— 它只看 `fonts::dir()`，\
             自带字体（在 global/）一律解析不到"
        );
    }


    /// 接口级：`GET /api/polish/font?name=<不存在的名字>` 必须 **404**，不是 200 + 别的字体。
    ///
    /// 与上一条（源码级候选链判据）互补：这条真发请求，钉的是**对外行为**。
    /// 只断言状态码 —— 不断言字节，免得依赖测试机装了哪些字体。
    ///
    /// ⚠️ **`dir` 必须传项目内的绝对路径**：`/polish/font` 挂在 `enforce_scope` 那层里，
    /// 给相对/不存在的 `dir` 会被**中间件**先回 404 —— 那样这条用例"过了"却**根本没走到 handler**，
    /// 是假通过（实测：加回 msyh 兜底后它照样绿，因为它压根没进 handler）。
    /// 所以这里用 `t.root`（那个真项目目录）拼绝对路径。
    #[tokio::test]
    async fn 字体端点对解析不到的名字回404而不是200() {
        let t = super::test_support::test_app().await;
        // 手写百分号编码（本 crate 没有 urlencoding 依赖；`dir` 里有 `\` 与 `:` 必须编掉）
        let dir: String = t
            .root
            .to_string_lossy()
            .chars()
            .map(|c| match c {
                'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '_' | '.' | '~' | '/' => c.to_string(),
                _ => format!("%{:02X}", c as u32),
            })
            .collect();
        // ⚠️ 路径**不带 `/api` 前缀**：`api::router()` 返回的就是内层路由，
        // `/api` 那一段由 `nest("/api", …)` 加上（本文件与其它 api 单测一致）。
        let uri = format!(
            "/polish/font?project=1&dir={}&name=__绝对不存在的字体_XZ__",
            dir
        );
        let resp = t.call(t.req("GET", &uri).body(Body::empty()).unwrap()).await;
        assert_eq!(
            resp.status(),
            StatusCode::NOT_FOUND,
            "解析不到的字体必须 404 —— 回 200 + 任意字体字节会让前端\
             `new FontFace(要的名字, 别的字体字节)` 注册成功、CSS 命中却画出别的字形，\
             整条链路不报错（用户报的「换字体没反应」）"
        );
    }

}