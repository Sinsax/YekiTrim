//! `api::polish::assets`：素材（贴纸 / 视频库 / 音效 / emoji / 贴纸帧与预热）。

use std::sync::Arc;

use axum::extract::{Extension, State};
use axum::http::StatusCode;
use axum::Json;

use crate::model::AppState;
// 贴纸两接口的路径判据：**唯一一份**在 crate::scope（中间件与 handler 两层共用）
use crate::scope::is_safe_project_relative;

use super::*;


/// 可用的贴图来源：内置 emoji 码点池 + 全局贴纸库（exe 同级 stickers/）的包名与 tags。
/// LLM 生成时按 sticker_source 过滤可用的贴纸池。
pub(crate) async fn polish_sticker_sources(
    Extension(user): Extension<crate::auth::CurrentUser>,
) -> ApiResult {
    let owner = crate::auth::owner_of(&user);
    use yekitrim_polish::assets::sticker_index::{load_package_meta, load_sticker_index};
    let root = yekitrim_core::owner::asset_dir(owner, yekitrim_core::owner::AssetKind::Stickers);
    let index = load_sticker_index(&root);
    let mut out = vec![serde_json::json!({
        "name": "emoji",
        "builtin": true,
        "description": "内置 emoji 码点池（按情绪匹配）",
        "tags": [],
    })];
    for name in &index.packages {
        let meta = load_package_meta(&root, name);
        let (desc, tags) = match meta {
            Some(m) => (m.description, m.tags.iter().map(|t| t.tag.clone()).collect::<Vec<_>>()),
            None => (String::new(), Vec::new()),
        };
        out.push(serde_json::json!({
            "name": name,
            "builtin": false,
            "description": desc,
            "tags": tags,
        }));
    }
    ok(serde_json::json!({ "sources": out }))
}


/// 贴纸库全量（**只读**，非 admin 也能看）：编辑器里贴纸选择器（**包 → 分类 → 贴纸**三段行递进）用。
///
/// 以磁盘为准（`scan_packages`），每个包给齐 `tags[].files`（`<包名>.toml` 的反向索引）
/// 与全部文件名 —— 未构建的包也给文件，只是没有 tag（前端把它的分类那一级收成一行「全部（不限）」）。
/// ⚠️ 分类计数与「包内全部图」都由这份响应派生（`tags[].files.length` / `files.length`），
/// 别再加"分类数"之类的字段 —— 两处数字迟早对不上。
pub(crate) async fn polish_sticker_library(
    Extension(user): Extension<crate::auth::CurrentUser>,
) -> ApiResult {
    let owner = crate::auth::owner_of(&user);
    use yekitrim_polish::assets::sticker_index::{
        list_package_files, load_package_meta, load_sticker_index, scan_packages,
    };
    let root = yekitrim_core::owner::asset_dir(owner, yekitrim_core::owner::AssetKind::Stickers);
    let index = load_sticker_index(&root);
    let packs: Vec<serde_json::Value> = scan_packages(&root)
        .into_iter()
        .map(|pkg| {
            let meta = load_package_meta(&root, &pkg.name);
            let files = list_package_files(&root.join(&pkg.name));
            let tags: Vec<serde_json::Value> = meta
                .as_ref()
                .map(|m| {
                    m.tags
                        .iter()
                        .map(|t| serde_json::json!({ "tag": t.tag, "files": t.files }))
                        .collect()
                })
                .unwrap_or_default();
            serde_json::json!({
                "name": pkg.name,
                "built": meta.is_some(),
                "registered": index.packages.iter().any(|n| n == &pkg.name),
                "description": meta.as_ref().map(|m| m.description.clone()).unwrap_or_default(),
                "tags": tags,
                "files": files,
            })
        })
        .collect();
    ok(serde_json::json!({ "ok": true, "packs": packs }))
}

/// **编辑器表情面板**：内建 emoji 目录按 tag 分组（`GET /api/polish/emoji-catalog`）。
///
/// 目录是**编译期内嵌**的（`assets/emoji_catalog.json`，881 条），不联网、不读用户库 ——
/// 所以这条接口没有 `Owner` 维度，也不需要 `dir`。
///
/// ⚠️ 与 `polish_sticker_sources` 的分工：那条只回"有哪些**来源**"（含 `emoji` 这个名字），
/// 回不了"emoji 里有哪些**具体表情**"；编辑器「贴纸轨」的 keyword 选到 `emoji` 之后，
/// 面板要的就是这一份。以前没有它 —— 面板只给一个"填码点"的输入框，
/// 用户得自己知道 `1f62d` 是 😭（用户报「添加表情后没有正确显示」的那个入口）。
pub(crate) async fn polish_emoji_catalog() -> ApiResult {
    use yekitrim_polish::assets::emoji_search::EmojiCatalog;
    // ⚠️ 走**内嵌**目录那条（`from_embedded` 经由 `fetch` 的第一分支），不建 HTTP 客户端：
    // 本 crate 没有 `reqwest` 依赖，而且这条接口本来就该离线可用（面板打开就要有表情）。
    // 目录读不出来不算致命 —— 回空目录，面板显示"没有可选表情"，
    // 而不是让整个编辑器报错。
    let groups = match EmojiCatalog::fetch_embedded() {
        Ok(c) => c.grouped(),
        Err(e) => {
            tracing::warn!("emoji 目录加载失败: {e}");
            Vec::new()
        }
    };
    let groups: Vec<serde_json::Value> = groups
        .into_iter()
        .map(|(tag, codepoints)| serde_json::json!({ "tag": tag, "codepoints": codepoints }))
        .collect();
    ok(serde_json::json!({ "ok": true, "groups": groups }))
}


/// 视频素材库（**只读**，非 admin 也能看）：项目设置里「片头/片尾」挑素材用。
///
/// 与 `polish_sticker_library` 同一条口径：**以磁盘为准**（`scan_video_library`），
/// 手工把文件丢进 `<数据目录>/resources/videos/` 也会出现在这里。
///
/// ⚠️ 视频库**不再挂在贴纸根下面**（2026-10-09）：它有自己的根，判据是
/// `path::video_dir_readable()`（带存量回落）。别写回 `sticker_dir().join("videos")`
/// —— 那样新布局下这里会**永远返回空列表**，而界面只会显示"库是空的"、不报错。
///
/// 每个素材带上**时长**（ffprobe）—— 界面要靠它把"出点 = 0 表示播到结尾"显示成人话，
/// 而且用户设的出点必须能被夹到这个长度（不然越界读素材会让底座报 `source_range_exceeded`）。
/// 探不出时长的照样列出来（`duration: null`），界面置灰并写明原因 —— 不隐藏，
/// 否则用户会觉得"我明明放进去了却没有"。
pub(crate) async fn polish_video_library(
    Extension(user): Extension<crate::auth::CurrentUser>,
) -> ApiResult {
    let owner = crate::auth::owner_of(&user);
    // ⚠️ **读两层**（用户库优先、全局补缺）：公用视频素材放在 `global/` 里。
    // 显示的 `root` 仍是**用户库** —— 那是"你放文件放这儿"，全局库是只读公用素材。
    let roots = yekitrim_core::owner::asset_dirs(
        owner,
        yekitrim_core::owner::AssetKind::Videos,
    );
    let lib_dir = roots[0].to_string_lossy().to_string();
    let items: Vec<serde_json::Value> =
        blocking(move || Ok(yekitrim_polish::assets::sticker_index::scan_video_library_roots(&roots)))
            .await?
            .into_iter()
            .map(|a| {
                let path = a.path.to_string_lossy().to_string();
                // ⚠️ **一次 ffprobe 拿全**（不是"每个字段起一个进程"）：素材库可能有几十个视频。
                // 右栏要显示分辨率/编码/帧率/码率，全在这里探好 —— 前端拿到就能画，
                // 不必为点开一个视频再发一次请求（那会让点选有可见延迟）。
                let p = yekitrim_polish::render::probe_video(&path);
                serde_json::json!({
                    "name": a.name,
                    "file": a.file_name(),
                    "duration": p.duration,
                    "width": p.width,
                    "height": p.height,
                    "codec": p.codec,
                    "fps": p.fps,
                    "bitrate": p.bitrate,
                    "has_audio": p.has_audio,
                })
            })
            .collect();
    // `root` 给界面显示"把文件放哪" —— 用户看不到目录就只能猜。
    ok(serde_json::json!({ "ok": true, "root": lib_dir, "videos": items }))
}


/// 音效目录（只读）：编辑器里音效事件/特效配声的列表选择。
///
/// 数据来自编译进二进制的 `assets/sfx_catalog.json`；全局 `[sfx] blacklist`
/// 屏蔽掉的整族不列出来（选了也不会被解析）。
pub(crate) async fn polish_sfx_catalog() -> ApiResult {
    use yekitrim_polish::assets::sfx_catalog;
    let blacklist = yekitrim_core::config::load_config_at(std::path::Path::new("."))
        .map(|c| c.sfx.blacklist)
        .unwrap_or_default();
    let entries: Vec<serde_json::Value> = sfx_catalog::all()
        .iter()
        .filter(|e| !sfx_catalog::is_blacklisted(&e.id, &blacklist))
        .map(|e| {
            serde_json::json!({
                "id": e.id,
                "label": e.label,
                "cat": e.cat,
                "tags": e.tags,
            })
        })
        .collect();
    ok(serde_json::json!({ "ok": true, "entries": entries }))
}


/// **素材库统一取文件**接口（`GET /api/polish/asset?kind=&file=`）—— 三个类型一条路。
///
/// 为什么要有它：素材库改成"文件夹管理器"之后，**三种素材都要能预览**
/// （视频要能真播、贴纸看大图、字体用样例字）—— 而三条老的取文件路各有各的形状：
/// 贴纸走 `?pack=&name=`、字体根本没有"取文件"（只有"取字库元数据"）、视频没有。
/// 与其让前端按类型分支拼三种 URL，不如**在服务端收敛成一条**：
/// 前端永远只说"哪个类型、哪个文件"。
///
/// ⚠️ **`kind` 只用来选根目录，不参与拼路径** —— 路径一律由服务端从
/// [`kind_root`] 现取，`file` 只允许**单个文件名**（`is_safe_component` 挡
/// 分隔符与 `..`），所以拿不到库外的文件。**别再接受"子目录"或"相对路径"**：
/// 那等于把 canonicalize 那层兜底变成唯一防线。
///
/// ⚠️ 贴纸仍然走老路（`/polish/sticker-file?pack=&name=`）：贴纸是**两级**
/// （包 / 包内文件），这里刻意**不收编**它 —— 收编就得让 `file` 认 `/`，
/// 正好把上面那条安全性质破掉。
pub(crate) async fn polish_asset_file(
    Extension(user): Extension<crate::auth::CurrentUser>,
    axum::extract::Query(q): axum::extract::Query<std::collections::HashMap<String, String>>,
    headers: axum::http::HeaderMap,
) -> axum::response::Response {
    use yekitrim_polish::assets::sticker_index::is_safe_component;
    let owner = crate::auth::owner_of(&user);
    let kind = q.get("kind").map(String::as_str).unwrap_or("");
    let file = q.get("file").map(String::as_str).unwrap_or("");
    let Some(root) = asset_kind_root(owner, kind) else {
        return crate::media::json_error(StatusCode::BAD_REQUEST, "未知的素材类型");
    };
    if !is_safe_component(file) {
        return crate::media::json_error(StatusCode::BAD_REQUEST, "文件名不合法");
    }
    let path = root.join(file);
    // 同步 canonicalize 不能留在 async worker 上（`api/mod.rs` 的纪律）——
    // 这条是**播放热路径**：视频预览会打出一连串 Range 请求。
    let (real, root_real) = tokio::task::spawn_blocking({
        let (p2, r2) = (path.clone(), root.clone());
        move || {
            (
                p2.canonicalize().unwrap_or_else(|_| p2.clone()),
                r2.canonicalize().unwrap_or_else(|_| r2.clone()),
            )
        }
    })
    .await
    .unwrap_or_else(|_| (path.clone(), root.clone()));
    if !real.starts_with(&root_real) {
        return crate::media::json_error(StatusCode::NOT_FOUND, "文件不在素材库内");
    }
    // ⚠️ mime 按**扩展名**给，别一律 octet-stream：浏览器靠它决定"当视频播还是当文件下载"。
    // `mime_guess` 对 mp4/webm/ttf/otf/png/jpg 都认，认不出的才落到 octet-stream。
    let mime = mime_guess::from_path(&path).first_or_octet_stream().to_string();
    crate::media::serve_file_from_headers(&path, &mime, &headers).await
}


/// `kind` → 该 owner 该类型的库根目录（**唯一映射表**）。
///
/// ⚠️ 一律走 `yekitrim_core::owner`，**别在这里拼 `resources/...`** ——
/// 库位置的口径只有那一份（单人 = 程序目录、多用户 = 各自的树），
/// 这里写死就等于是把另一套口径抄了第二遍。
fn asset_kind_root(
    owner: yekitrim_core::owner::Owner,
    kind: &str,
) -> Option<std::path::PathBuf> {
    use yekitrim_core::owner::{asset_dir, AssetKind};
    match kind {
        "video" => Some(asset_dir(owner, AssetKind::Videos)),
        "font" => Some(asset_dir(owner, AssetKind::Fonts)),
        _ => None,
    }
}


/// **视频素材的缩略图**（`GET /api/polish/video-thumb?file=`）—— 素材库网格用。
///
/// 抽**第一秒**那一帧（不是第 0 帧：很多录播文件开头是黑场或渐入，截出来一片纯黑，
/// 用户会以为"这素材是坏的"）。缩到 320px 宽，JPEG q=4 —— 网格里那格才 72px 高。
///
/// ⚠️ **结果按 `<库>/.thumbs/<文件>@<mtime>.jpg` 缓存**：抽帧要起一次 ffmpeg（几十到几百毫秒），
/// 而网格一进页面就会把**每个**视频都拉一遍。不做缓存的话，一个 50 个素材的库每次打开
/// 都白跑 50 次 ffmpeg。文件名里带 **mtime** ⇒ 用户换了同名文件自然失效，不用清缓存。
///
/// ⚠️ 抽不出来（ffmpeg 不在 PATH / 容器怪异 / 文件损坏）**不是错误**：回 404，
/// 前端退成图标 + 文件名（见 `MediaLibrary.vue`）。**别回 500** —— 那会在界面上报一条
/// "读取失败"，而实际只是"这张没缩略图、别的都好"。
pub(crate) async fn polish_video_thumb(
    Extension(user): Extension<crate::auth::CurrentUser>,
    axum::extract::Query(q): axum::extract::Query<std::collections::HashMap<String, String>>,
    headers: axum::http::HeaderMap,
) -> axum::response::Response {
    let owner = crate::auth::owner_of(&user);
    use yekitrim_polish::assets::sticker_index::is_safe_component;
    let file = q.get("file").map(String::as_str).unwrap_or("").to_string();
    if !is_safe_component(&file) {
        return crate::media::json_error(StatusCode::BAD_REQUEST, "文件名不合法");
    }
    let root = yekitrim_core::owner::asset_dir(owner, yekitrim_core::owner::AssetKind::Videos);
    let src = root.join(&file);
    if !src.is_file() {
        return crate::media::json_error(StatusCode::NOT_FOUND, "素材不存在");
    }
    // 抽帧是同步阻塞（起 ffmpeg），整段丢 blocking 池 —— 否则网格一进页面就把 worker 占满。
    // ⚠️ `file` 必须是 `String`（owned）：闭包要求 `'static`，借用 `q` 活不过它。
    let thumb = tokio::task::spawn_blocking(move || make_video_thumb(&root, &src, &file))
        .await
        .ok()
        .flatten();
    let Some(thumb) = thumb else {
        // 抽不出来 = "没有缩略图"，不是"接口坏了"（前端按 404 退成图标）
        return crate::media::json_error(StatusCode::NOT_FOUND, "无法生成缩略图");
    };
    crate::media::serve_file_from_headers(&thumb, "image/jpeg", &headers).await
}


/// 抽并缓存一帧。返回 `None` = 抽不出来（调用方回 404）。
///
/// 缓存在 `<库>/.thumbs/` 下 —— ⚠️ 以 `.` 开头是有意的：`scan_video_library` 只认**文件**，
/// 而 `.thumbs` 是目录，本来就不会被列成素材；名字带点则是为了让人一眼看出"这是程序生成的"。
fn make_video_thumb(root: &std::path::Path, src: &std::path::Path, file: &str) -> Option<std::path::PathBuf> {
    let mtime = std::fs::metadata(src).ok()?.modified().ok()?;
    let stamp = mtime
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let dir = root.join(".thumbs");
    let out = dir.join(format!("{file}@{stamp}.jpg"));
    if out.is_file() {
        return Some(out); // 缓存命中
    }
    std::fs::create_dir_all(&dir).ok()?;
    // `-ss 1` 放在 `-i` **之前** = 快速定位（跳过去而不是解码过去）；
    // `-frames:v 1` 只取一帧；`-vf scale=320:-2` 保宽 320、高按偶数对齐（JPEG 要求偶数）。
    // ⚠️ `-update 1` 不能省：输出名不带 `%03d` 时 ffmpeg 会警告"Use a pattern … or the
    // -update option"，虽然当前版本仍会写出文件，但那是**靠默认行为兜的**。
    // 实测（`op.mp4`，3s）：不加这行有 [image2] 警告，加了干净 —— 别等哪个版本把它变成硬错误。
    let st = yekitrim_core::process::cmd("ffmpeg")
        .args([
            "-y",
            "-ss",
            "1",
            "-i",
            &src.to_string_lossy(),
            "-frames:v",
            "1",
            "-vf",
            "scale=320:-2",
            "-q:v",
            "4",
            "-update",
            "1",
            &out.to_string_lossy(),
        ])
        .output()
        .ok()?;
    if !st.status.success() || !out.is_file() {
        // 短于 1 秒的素材（`-ss 1` 越过结尾）：退到第 0 帧再试一次。
        // ⚠️ 别直接放弃 —— 片头素材常常就一两秒，那正是最需要缩略图的一类。
        std::fs::remove_file(&out).ok();
        let st2 = yekitrim_core::process::cmd("ffmpeg")
            .args([
                "-y",
                "-i",
                &src.to_string_lossy(),
                "-frames:v",
                "1",
                "-vf",
                "scale=320:-2",
                "-q:v",
                "4",
                "-update",
                "1",
                &out.to_string_lossy(),
            ])
            .output()
            .ok()?;
        if !st2.status.success() || !out.is_file() {
            return None;
        }
    }
    Some(out)
}


/// 贴纸库里的单张图（编辑器缩略图直连 `<img>`）；只读 + 路径安全校验，与 admin 版同款。
pub(crate) async fn polish_sticker_file(
    Extension(user): Extension<crate::auth::CurrentUser>,
    axum::extract::Query(q): axum::extract::Query<std::collections::HashMap<String, String>>,
    headers: axum::http::HeaderMap,
) -> axum::response::Response {
    let owner = crate::auth::owner_of(&user);
    use yekitrim_polish::assets::sticker_index::{is_safe_component, sanitize_pack_name};
    let root = yekitrim_core::owner::asset_dir(owner, yekitrim_core::owner::AssetKind::Stickers);
    let pack = q.get("pack").map(String::as_str).unwrap_or("");
    let name = q.get("name").map(String::as_str).unwrap_or("");
    let Some(pack) = sanitize_pack_name(pack) else {
        return crate::media::json_error(StatusCode::BAD_REQUEST, "包名不合法");
    };
    if !is_safe_component(name) {
        return crate::media::json_error(StatusCode::BAD_REQUEST, "文件名不合法");
    }
    let path = root.join(&pack).join(name);
    // ⚠️ 同步 canonicalize 不能留在 async worker 上（api/mod.rs:44-51 纪律 ✓）——
    // 这条是**播放热路径**：一进 Dhampir 就连打贴纸 PNG，而工程常在慢盘上 ⇒
    // worker 被占住 ⇒ 同时刻的 engine.js 响应产不出来（用户实测的黑屏 ✗）。
    let (real, root_real) = {
        let p2 = path.clone();
        let r2 = root.clone();
        match tokio::task::spawn_blocking(move || {
            let a = p2.canonicalize().unwrap_or_else(|_| p2.clone());
            let b = r2.canonicalize().unwrap_or_else(|_| r2.clone());
            (a, b)
        })
        .await
        {
            Ok(v) => v,
            Err(_) => (path.clone(), root.clone()),
        }
    };
    if !real.starts_with(&root_real) {
        return crate::media::json_error(StatusCode::NOT_FOUND, "文件不在贴纸库内");
    }
    let mime = mime_guess::from_path(&path).first_or_octet_stream().to_string();
    crate::media::serve_file_from_headers(&path, &mime, &headers).await
}


/// 贴纸逐帧 PNG 的**进程内缓存**。
///
/// # 为什么必须有（这是踩出来的）
///
/// 解一段 GIF 要跑满它的**全部帧** —— `哭_1.gif` 66 帧、数百毫秒；而预览在 33fps 下
/// 会**反复**要同一张贴纸的不同帧。不缓存的话每个请求都重解整段，
/// 请求立刻堆起来把服务端 CPU 打满 —— 现象是**播放一走到贴纸就卡死**，
/// 而且一个请求都回不来（工具条上是 `就绪 0/4 · 解码 0 · 已交 0`）。
///
/// 键带上 **mtime + 文件长度**：素材被换掉时自动失效，不需要谁记得清缓存。
/// 上限 64 条（贴纸数量本来就少，换素材会不断产生新键，得有个头）。
fn sticker_frames_cached(
    file: &std::path::Path,
) -> Result<std::sync::Arc<Vec<(Vec<u8>, u16)>>, super::AppError> {
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex, OnceLock};
    type Key = (std::path::PathBuf, u64, u64);
    // **按字节上限的 LRU** ✓ —— 以前是"条目数 > 64 就 `clear()`"✗：
    // 那不是淘汰是清零，贴纸一多/换素材就整表作废 ⇒ 同一个 GIF 被反复整段解码（一次数百 ms ✗）
    // ⇒ 现象正是"播放一走到贴纸就卡死"（实测 `解码 49` ✗）。
    const CAP_BYTES: usize = 512 * 1024 * 1024;   // 512MB 硬上限
    struct Entry {
        used: u64,            // 最近一次使用序号（LRU ✓）
        bytes: usize,
        frames: Arc<Vec<(Vec<u8>, u16)>>,
    }
    static CACHE: OnceLock<Mutex<(HashMap<Key, Entry>, u64, usize)>> = OnceLock::new();

    let meta = std::fs::metadata(file)
        .map_err(|e| err(StatusCode::NOT_FOUND, format!("贴纸读不到: {e}")))?;
    let mtime = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    let key = (file.to_path_buf(), mtime, meta.len());

    let cache = CACHE.get_or_init(|| Mutex::new((HashMap::new(), 0, 0)));
    let mut guard = cache.lock().expect("贴纸缓存锁中毒");
    guard.1 += 1;
    let now = guard.1;
    if let Some(hit) = guard.0.get_mut(&key) {
        hit.used = now;                       // 命中即刷新活跃度 ✓
        let frames = hit.frames.clone();
        drop(guard);
        return Ok(frames);
    }
    drop(guard);
    // 解码放在锁**外面**：同一张图被两个请求同时要时，宁可多解一次，
    // 也不要让慢解码堵住整张表的读（那又是另一种"卡死"）。
    let frames = yekitrim_polish::render::gif::decode_sticker_frames_png(file)
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("贴纸解码失败: {e}")))?;
    let bytes: usize = frames.iter().map(|(b, _)| b.len()).sum();
    let arc = Arc::new(frames);
    let mut guard = cache.lock().expect("贴纸缓存锁中毒");
    guard.1 += 1;
    guard.2 += bytes;
    let used = guard.1;
    guard.0.insert(key, Entry { used, bytes, frames: arc.clone() });
    // **超上限时只逐出最久未用的，直到回到上限以内** ✓（绝不再清零 ✗）
    while guard.2 > CAP_BYTES && guard.0.len() > 1 {
        let victim = guard
            .0
            .iter()
            .min_by_key(|(_, e)| e.used)
            .map(|(k, _)| k.clone());
        match victim {
            Some(k) => {
                if let Some(e) = guard.0.remove(&k) {
                    guard.2 = guard.2.saturating_sub(e.bytes);
                }
            }
            None => break,
        }
    }
    Ok(arc)
}


/// `POST /polish/sticker-frame` —— 贴纸的**第 k 帧**，PNG 以 base64 装在 JSON 里。
///
/// # ⚠️ 现状（2026-10-02 起）：这条路**只服务出片腿**，不再是预览的贴纸帧源
///
/// 底座 wasm 到了 **host_api v6**，动图（GIF / 动画 WebP）由**引擎自己解码、自己按帧号取帧**
/// （`dhampir_asset_load_animation`）—— 所以浏览器预览那条腿已经**不再调这一条**：
/// 它现在把动图原文件整段交给引擎（一次 fetch + 一次 load），播放期零请求。
/// 本仓前端**只有静态图**（`kind === 'image'`）仍会打这里。
///
/// **接口本身保留**：本仓服务端的出片腿（未切底座时）仍以它为准，
/// 那是"预览与成片贴纸逐帧一致"的过渡期保证。删它会连出片一起废掉。
///
/// # 为什么当初"谁来解多帧图"必须由服务端回答
///
/// 浏览器这一侧**两条路都实测过、都不可用**：
///
///   * `<img>` 的动画时钟会被节流 —— 不可见、被 canvas 遮住、甚至 1×1 盖在最上层
///     （不被遮挡）都不推进帧；宿主 `drawImage` 拿到的永远是同一张；
///   * `ImageDecoder`（WebCodecs）在目标浏览器里**根本不存在**
///     （`typeof ImageDecoder !== 'function'` → `decoder=false`、帧数 0）。
///
/// 而 Dhampir 的设计是"序列帧由宿主交进来"（`AssetKind::ImageSequence` 只是个校验标记，
/// 那边没有任何 GIF 解码器）。所以这一环只能由服务端补上 —— 而且**必须与出片同源**：
/// 这里走的就是出片用的 `decode_sticker_frames_png`，于是**预览与成片的贴纸逐帧一致**。
///
/// # 为什么逐帧，而不是一次给整段
///
/// 整段拼成一张横向 PNG 实测是 **19 MB**（500×500 × 66 帧），而且宽度 33000px
/// 超过浏览器单图上限（16384）。预览一次只要一帧，所以按帧取。
///
/// 帧号越界**报错**而不是夹到范围内：夹住会让"贴纸停在最后一帧"看起来像素材放完了。
pub(crate) async fn polish_sticker_frame(
    State(_state): State<Arc<AppState>>,
    Json(body): Json<serde_json::Value>,
) -> ApiResult {
    let dir = body
        .get("dir")
        .and_then(|v| v.as_str())
        .ok_or_else(|| err(StatusCode::BAD_REQUEST, "缺 dir"))?;
    // 取成**自有 String**：下面要 move 进 blocking 闭包，借用 `body` 活不了那么久。
    let asset: String = body
        .get("asset")
        .and_then(|v| v.as_str())
        .ok_or_else(|| err(StatusCode::BAD_REQUEST, "缺 asset"))?
        .to_string();
    let frame = body.get("frame").and_then(|v| v.as_u64()).unwrap_or(0);
    let path = validated_project_dir(dir)?;
    // 越界防护：asset 必须是**工程内相对路径** —— 判据在 `crate::scope::is_safe_project_relative`
    // （与 `polish_sticker_prewarm`、scope 中间件共用同一份，不许各写一遍；这是 handler 侧的第二道）。
    if !is_safe_project_relative(&asset) {
        return Err(err(StatusCode::BAD_REQUEST, "asset 必须是工程内的相对路径"));
    }
    let file = path.join(&asset);
    // 解码要跑满 66 帧（数百 ms）→ 整段下 blocking 池，别占 worker。
    let (png, count) = blocking(move || {
        if !file.exists() {
            return Err(err(
                StatusCode::NOT_FOUND,
                format!("贴纸不存在：{asset}"),
            ));
        }
        // **走缓存**：同一个文件只解一次（见 `sticker_frames_cached` 的说明 ——
        // 不缓存的话，播放一走到贴纸就会把服务端打满、页面卡死）。
        let frames = sticker_frames_cached(&file)?;
        let count = frames.len();
        let idx = frame as usize;
        if count == 0 {
            return Err(err(StatusCode::INTERNAL_SERVER_ERROR, "贴纸一帧都没有"));
        }
        if idx >= count {
            return Err(err(
                StatusCode::BAD_REQUEST,
                format!("帧号 {idx} 越界（共 {count} 帧）"),
            ));
        }
        let (png, _delay) = frames[idx].clone();
        // 尺寸不回填：页面拿到 PNG 之后 `createImageBitmap` 自己就有宽高，
        // 在这儿再解一次 PNG 只是白花时间（而且 `image` 也不是 server 的依赖）。
        Ok((png, count))
    })
    .await?;
    use base64::Engine as _;
    let b64 = base64::engine::general_purpose::STANDARD.encode(&png);
    Ok(Json(serde_json::json!({
        "ok": true,
        "count": count,
        "frame": frame,
        "png_base64": b64,
    })))
}


/// 判据本体在 [`crate::scope::is_safe_project_relative`]（**全仓只此一份**）：
/// 中间件（`enforce_scope`，第一道）与这里的两个 handler（第二道）共用。
/// 当年"两份实现、判据还漏了根相对"就是 `/etc/passwd` 那条洞的成因 —— 别再把判据抄回来。
///
/// 把前端交上来的贴纸路径数组过滤成**工程内的相对路径**（判据见上）。
///
/// 写成函数而不是内联闭包，是为了让"越界值被挡掉"有单测钉住 —— 内联在 handler 里的 filter
/// 没有单测能碰它（这正是那条洞一直没被发现的原因）。
fn safe_sticker_assets(assets: Vec<String>, base: &std::path::Path) -> Vec<std::path::PathBuf> {
    assets
        .into_iter()
        .filter(|a| crate::scope::is_safe_project_relative(a))
        .map(|a| base.join(a))
        .collect()
}


/// `POST /polish/sticker-prewarm` —— 把若干贴纸的帧表**先解码进内存缓存**，不返回任何帧。
///
/// # 为什么需要它
///
/// `/polish/sticker-frame` 是"播到哪一帧才取哪一帧"，而**第一次**取某张贴纸时服务端要
/// **整段解码**那张 GIF（实测 `clip-04` 的 4 张贴纸冷解 **20.2s**、单张 3~9s，debug 构建）。
/// 页面那头 `Promise.race([…, 30ms])` 超时就**这一帧不交**，于是表现是
/// **"播放走到贴纸就有一段时间什么都不显示，然后突然出现"**（用户报的"贴纸有延迟"）。
///
/// 所以让客户端**在页面加载时**就把这件事做掉：解码挪到"用户还没播到那里"的时候，
/// 播放真正走到贴纸时全部命中缓存（实测命中 **10~14ms**）。
///
/// # 语义
///
/// * `assets` 是工程内相对路径数组（与 `/polish/sticker-frame` 的 `asset` 同一条纪律）。
/// * **尽力而为**：单张贴纸解不出来只跳过它，**不让整个请求失败** —— 它是预热，不是业务操作；
///   真有问题会在用户播到那帧时由 `sticker-frame` 照常报错。
/// * 返回 `{ok, decoded, failed, skipped}`，前端只用来打日志。
/// * 走 `blocking` 池：解码是纯 CPU 的同步活，占满一个核数秒，绝不能留在 async worker 上
///   （见 `crates/yekitrim-server/src/api/AGENTS.md` 的同步 IO 纪律）。
pub(crate) async fn polish_sticker_prewarm(
    State(_state): State<Arc<AppState>>,
    Json(body): Json<serde_json::Value>,
) -> ApiResult {
    let dir = body
        .get("dir")
        .and_then(|v| v.as_str())
        .ok_or_else(|| err(StatusCode::BAD_REQUEST, "缺 dir"))?;
    let assets: Vec<String> = body
        .get("assets")
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    let path = validated_project_dir(dir)?;
    let files = safe_sticker_assets(assets, &path);
    let (decoded, failed) = blocking(move || {
        let mut ok_count = 0usize;
        let mut fail_count = 0usize;
        for f in &files {
            if !f.exists() {
                fail_count += 1;
                continue;
            }
            // 命中缓存就直接算成功（`sticker_frames_cached` 自己判），不做任何多余解码。
            match sticker_frames_cached(f) {
                Ok(_) => ok_count += 1,
                Err((_status, body)) => {
                    tracing::warn!("贴纸预热失败（{}）：{}", f.display(), body.0);
                    fail_count += 1;
                }
            }
        }
        Ok((ok_count, fail_count))
    })
    .await?;
    Ok(Json(serde_json::json!({
        "ok": true,
        "decoded": decoded,
        "failed": failed,
    })))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::test_support::*;
    use axum::body::Body;

    // ⚠️ 这些用例是**按原字节搬来**的（含原本在文件顶层、缩进为 0 的那批）：
    //    统一重排缩进会动到多行字符串字面量，判据会因此悄悄变。


    /// 贴纸预热接口的**路径面**：`..` / 绝对 / 盘符 / 反斜杠 / **根相对**必须被挡掉。
    ///
    /// 根相对那两条（`/etc/passwd`、`\windows\x`）是**这条单测逼出来的加固**：
    /// 原判据只看 `Path::is_absolute()`，而它在 Windows 上对 `/etc/passwd` 返回 **false**，
    /// 且 `Path::join` 对"有根无前缀"的路径会**替换基准**（实测 `C:\proj` + `/etc/passwd`
    /// → `C:/etc/passwd`，已经出了工程）。判据已换成与平台无关的逐字符判定。
    #[test]
    fn 贴纸预热把越界路径全挡掉() {
        let base = std::path::Path::new(r"C:\proj");
        let out = safe_sticker_assets(
            vec![
                "assets/stickers/问号.gif".into(),  // 正常：工程内相对路径
                "".into(),                            // 空
                "../secret.gif".into(),               // 上跳
                r"..\secret.gif".into(),             // 反斜杠 + 上跳
                "C:/windows/x.gif".into(),           // 盘符
                "/etc/passwd".into(),                 // 根相对（Windows 上 is_absolute()==false）
                r"\windows\x.gif".into(),            // 根相对（反斜杠）
                ".hidden.gif".into(),                 // 隐藏文件
                "assets/../../escape.gif".into(),     // 夹在中间的 ..
            ],
            base,
        );
        assert_eq!(out.len(), 1, "只有第一个该活下来：{out:?}");
        assert_eq!(out[0], base.join("assets/stickers/问号.gif"));
    }


    /// 同一句判据在 `sticker-frame`（单值）与 `sticker-prewarm`（数组）上必须一致 ——
    /// 各写一份就是"改一边漏另一边"，`/etc/passwd` 那条洞就是这么留下来的。
    #[test]
    fn 两个贴纸接口共用同一条路径判据() {
        for bad in ["", "/etc/passwd", r"\windows\x", "C:/x.gif", "../x.gif", ".x"] {
            assert!(
                !is_safe_project_relative(bad),
                "`{bad}` 本该被判为越界"
            );
            assert!(
                safe_sticker_assets(vec![bad.into()], std::path::Path::new("/p")).is_empty(),
                "`{bad}` 本该被数组版判据挡掉"
            );
        }
        assert!(is_safe_project_relative("assets/stickers/问号.gif"));
    }


    /// 读 `/polish/sticker-frame` 的响应体：base64 的 PNG 一帧约 **390 KB**，
    /// 而 `test_support::body_json` 的 256 KB 上限装不下（`LengthLimitError`）——
    /// 这里按 8 MB 读，只让预热这一条测试用。
    async fn sticker_frame_json(res: axum::response::Response) -> serde_json::Value {
        let bytes = axum::body::to_bytes(res.into_body(), 8 * 1024 * 1024)
            .await
            .expect("贴纸帧响应体读不出来");
        serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null)
    }


    /// **预热真的省时间**：拿一张真 GIF 走完整条链，对比"直接要第 0 帧"与
    /// "先预热、再要第 0 帧"。判据是**预热之后的取帧明显更快**（同一进程内的服务端缓存）。
    ///
    /// 为什么不 Mock 解码：这条要证的正是"冷解码 3~9 秒"这个用户可见症状，
    /// 拿假 GIF 量不出那个数。样本是 `assets/` 里的真贴纸。
    #[tokio::test]
    async fn 贴纸预热让随后的取帧快得多() {
        // 找一张真的贴纸 GIF（贴纸库或某个真工程的产物）。
        let gif = [
            yekitrim_core::owner::sticker_dir_readable(yekitrim_core::owner::Owner::from_env()),
            std::path::PathBuf::from(r"E:\media\.yekitrim-cover-e2e\clip14\proj\assets\stickers"),
        ]
        .into_iter()
        .map(|d| d.join("思考(认真地).gif"))
        .find(|p| p.exists());
        let Some(gif) = gif else {
            eprintln!("跳过：本机没有可用的贴纸 GIF 样本");
            return;
        };
        let t = test_app().await;
        let clip = t.root.join("output/clips/prewarm-timing");
        let dir = clip.join("assets/stickers");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(clip.join("polish.toml"), yekitrim_polish::project::ProjectFile::example().to_toml_commented().unwrap())
            .unwrap();
        let rel = "assets/stickers/思考(认真地).gif";
        std::fs::copy(&gif, dir.join("思考(认真地).gif")).unwrap();
        let d = clip.to_string_lossy().to_string();

        // ① **冷**：直接要第 0 帧（此时进程内缓存必没有它）。
        let req = |uri: &str, body: serde_json::Value| {
            let (t, uri, body) = (&t, uri.to_string(), body.to_string());
            async move {
                t.call(
                    t.req("POST", &uri)
                        .header(axum::http::header::CONTENT_TYPE, "application/json")
                        .body(Body::from(body))
                        .unwrap(),
                )
                .await
            }
        };
        let t0 = std::time::Instant::now();
        let cold = req("/polish/sticker-frame", serde_json::json!({ "dir": d, "asset": rel, "frame": 0 }))
            .await;
        let cold_ms = t0.elapsed().as_millis();
        assert_eq!(cold.status(), StatusCode::OK, "冷取帧应当成功");
        let cold_v = sticker_frame_json(cold).await;
        assert!(cold_v["png_base64"].as_str().is_some_and(|s| !s.is_empty()));

        // ② 换一张还没进缓存的（同一张 GIF 复制一份改内容 → key 变 ⇒ 一定走解码）。
        //    先预热它，再立刻取帧 —— 后者命中的正是预热放进去的那份。
        let hot_name = "assets/stickers/hot.gif";
        std::fs::copy(&gif, clip.join(hot_name)).unwrap();
        let mut bytes = std::fs::read(&gif).unwrap();
        bytes.extend_from_slice(b"prewarm-probe");      // 改大小 ⇒ 缓存 key 变，必然重新解
        std::fs::write(clip.join(hot_name), &bytes).unwrap();

        let t1 = std::time::Instant::now();
        let pre = req(
            "/polish/sticker-prewarm",
            serde_json::json!({ "dir": d, "assets": [hot_name] }),
        )
        .await;
        let pre_ms = t1.elapsed().as_millis();
        assert_eq!(pre.status(), StatusCode::OK);
        assert_eq!(body_json(pre).await["decoded"].as_u64(), Some(1));

        let t2 = std::time::Instant::now();
        let warm = req("/polish/sticker-frame", serde_json::json!({ "dir": d, "asset": hot_name, "frame": 0 }))
            .await;
        let warm_ms = t2.elapsed().as_millis();
        assert_eq!(warm.status(), StatusCode::OK, "预热后取帧应当成功");
        let warm_v = sticker_frame_json(warm).await;
        assert!(
            warm_v["png_base64"].as_str().is_some_and(|s| !s.is_empty()),
            "预热之后仍应拿到同一张 PNG"
        );

        eprintln!("贴纸取帧：冷 {cold_ms}ms → 预热 {pre_ms}ms → 预热后取帧 {warm_ms}ms");
        // 预热本身就是那次冷解码，所以它取帧那一下必须快得多。
        assert!(
            warm_ms * 4 < cold_ms.max(1),
            "预热后应比冷解码快一个量级：冷 {cold_ms}ms vs 预热后 {warm_ms}ms"
        );
    }


    /// 贴纸预热接口：**不存在的贴纸不让整个请求失败**（它是预热，不是业务操作）。
    ///
    /// 回 200 + `decoded: 0`，前端才好 fire-and-forget。
    #[tokio::test]
    async fn 贴纸预热对不存在的文件也回成功() {
        let t = test_app().await;
        let clip = t.root.join("output/clips/prewarm-missing");
        std::fs::create_dir_all(&clip).unwrap();
        let cfg = yekitrim_polish::project::ProjectFile::example();
        std::fs::write(clip.join("polish.toml"), cfg.to_toml_commented().unwrap()).unwrap();

        let body = serde_json::json!({
            "dir": clip.to_string_lossy(),
            "assets": ["assets/stickers/不存在.gif", "../../etc/passwd"],
        })
        .to_string();
        let res = t
            .call(
                t.req("POST", "/polish/sticker-prewarm")
                    .header(axum::http::header::CONTENT_TYPE, "application/json")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await;
        // 越界那条被挡掉之后只剩一个不存在的文件，但**整体仍应成功**。
        assert_eq!(res.status(), StatusCode::OK, "预热是尽力而为，不该把请求判失败");
        let v = body_json(res).await;
        assert_eq!(v["ok"], true);
        assert_eq!(v["decoded"].as_u64(), Some(0));
        assert_eq!(v["failed"].as_u64(), Some(1), "只有那一个文件被真的尝试过：{v}");
    }

}