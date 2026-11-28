//! `api::polish::assets`：素材（贴纸 / 视频库 / 音效 / emoji / 贴纸帧与预热）。

use std::sync::Arc;

use axum::extract::{Extension, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
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


/// 音效目录（只读）：编辑器里音效事件/特效配声的列表选择，以及素材库「音效」页。
///
/// 数据来自编译进二进制的 `assets/sfx_catalog.json`；全局 `[sfx] blacklist`
/// 屏蔽掉的整族默认不列出来（选了也不会被解析）。
///
/// # 查询参数
///
/// - `include_blacklisted=1` —— 把它们也列出来（**素材库「音效」页要用**：
///   那一页的职责之一就是"看见并取消屏蔽"，默认过滤掉等于没法取消）。
///   条目上带 `blacklisted` 位，前端自己决定怎么标。
///
/// # 返回值
///
/// `entries[]` 每组带 `variants`（**扁平化后的变体键**，下载时再按 `variant_source`
/// 换回原变体名）、`blacklisted`（该条是否被屏蔽：精确 id 或整族 tag 命中）；
/// 顶层带 `blacklist`（当前 owner 的全局音效屏蔽列表）与 `tags`（全部 tag + 组数，
/// 素材库左栏分组用）。
///
/// ⚠️ **黑名单按 `Owner` 读**（`load_config_for`）。以前这里是
/// `load_config_at(Path::new("."))` —— 那是**进程默认 owner**，多用户下等于
/// "每个账号看到的是管理员那份黑名单"。音效页要能把屏蔽写回自己的配置，
/// 读侧就必须先落到同一个 owner 上，否则"我屏蔽了、刷新又回来"。
pub(crate) async fn polish_sfx_catalog(
    Extension(user): Extension<crate::auth::CurrentUser>,
    Query(q): Query<std::collections::HashMap<String, String>>,
) -> ApiResult {
    use yekitrim_polish::assets::sfx_catalog;
    let owner = crate::auth::owner_of(&user);
    let include_blacklisted = q.get("include_blacklisted").map(String::as_str) == Some("1");
    // 同步 IO（读 config.toml）⇒ blocking 池（本目录纪律）
    let blacklist = blocking(move || {
        Ok(yekitrim_core::config::load_config_for(owner)
            .sfx
            .blacklist)
    })
    .await?;
    let mut entries: Vec<serde_json::Value> = sfx_catalog::all()
        .iter()
        .filter(|e| include_blacklisted || !sfx_catalog::is_blacklisted(&e.id, &blacklist))
        .map(|e| {
            let blocked = sfx_catalog::is_blacklisted(&e.id, &blacklist);
            serde_json::json!({
                "id": e.id,
                "label": e.label,
                "cat": e.cat,
                // 素材库音效页按**包**分组（一包 = 贴纸那样的一行卡片）。
                // ⚠️ `pack` 走 `sfx_catalog::pack_of` —— **唯一**的兜底规则在那里
                //    （缺 subcat 的 10 条噪音类落到「其他」），别在前端再写一份
                //    `subcat || '其他'`。
                // **三层细分**（左侧是库，层级全在右侧）：
                //   pack    = 哪个库（预设一律「小森平」）
                //   group   = 一级细分（大类，7 个）
                //   subgroup= 二级细分（`subcat`，145 个）
                // ⚠️ 兜底各只在 Rust 侧一份（`sfx_catalog::pack_of/group_of/subgroup_of`）。
                "pack": sfx_catalog::pack_of(e),
                "group": sfx_catalog::group_of(e),
                "subgroup": sfx_catalog::subgroup_of(e),
                "subcat": e.subcat,
                "tags": e.tags,
                // 变体键（不带扩展名、已扁平化）—— 试听与前端列变体用。
                // ⚠️ 必须带 `.as_str()`：`variant_stem` 吃 `&str`，传 `&String` 是类型错。
                "variants": e.variants.iter().map(|v| sfx_catalog::variant_stem(v.as_str())).collect::<Vec<_>>(),
                "blacklisted": blocked,
                "built": true,
            })
        })
        .collect();
    // **用户音效包**（`<data>/users/<uid>/sfx/<包名>/`）：与预设包排在同一个包列表里。
    // ⚠️ 它们是**运行期资产**（不是内嵌的）⇒ 每次读目录都现扫，不缓存。
    let user_packs = user_pack_list(owner);
    // ⚠️ **用户包的条目必须并进 `entries`**（2026-11-28 修）：它和预设包在素材库里是
    //    同一种东西（网格里的一个组），而前端只读 `entries`。以前这里把它们只放在
    //    `user_packs` 里 ⇒ 左侧卡片显示"N 组"、点进去网格空白，用户以为包导坏了。
    //    编辑器的手动配声（`SfxPicker`）读的也是 `entries`，同一条修复覆盖两处。
    for p in &user_packs {
        entries.extend(user_pack_entries(p, &blacklist, include_blacklisted));
    }
    // 左侧「音效库」列表：**预设只有一条**（小森平），用户导入的各一条。
    // ⚠️ 别把大类塞进来 —— 左侧是"库的位置"，占满了新库没地方放（用户口径）。
    // ⚠️ 预设的 `count` 用**预设条目数**，不是 `entries.len()` —— `entries` 现在
    //    还并进了用户包条目，拿它会让预设卡片显示"1213 + 用户包总数"
    //    （而且被屏蔽的条目会被过滤掉，数量还会随屏蔽状态漂）。
    let preset_count = sfx_catalog::all().len();
    let mut packs: Vec<serde_json::Value> = vec![serde_json::json!({
        "id": sfx_catalog::PRESET_PACK_NAME,
        "name": sfx_catalog::PRESET_PACK_NAME,
        "kind": "preset",
        "count": preset_count,
        "deletable": false,
        "built": true,
    })];
    for p in &user_packs {
        packs.push(serde_json::json!({
            "id": p.name,
            "name": p.name,
            "kind": "user",
            "count": p.entries.len(),
            "groups": p.groups(),
            "deletable": true,
            // 「未构建」= 没有 `pack.toml`：能试听、能手选，但 **LLM 生成事件时读不到**。
            "built": p.built,
            "built_by": p.built_by,
        }));
    }
    ok(serde_json::json!({
        "ok": true,
        "entries": entries,
        "blacklist": blacklist,
        "packs": packs,
        "tags": sfx_catalog::all_tags(),
    }))
}


/// **单条音效试听**（`GET /api/polish/sfx-preview?id=&variant=`）—— 素材库「音效」页的播放源。
///
/// # 为什么不把 `taira-komori.net` 的 URL 直接发给浏览器
///
/// 源站（小森平/効果音ラボ）声明音效免费可商用，但**要求不要直接转载音效 URL**
/// （见 `THIRD-PARTY-NOTICES.md`）。所以浏览器只跟本服务说话：这里按需抓一次字节、
/// 落进 `<data>/cache/sfx/<变体键>.mp3`，之后所有试听都走本地文件（Range 由
/// [`crate::media::serve_file_from_headers`] 处理，`<audio>` 的拖动进度条能用）。
///
/// # 失败语义
///
/// 抓不到（源站 404、断网、超时）回 **404 + JSON**，前端标"试听不可用"。
/// **不回 500**：那会把一条"这个音效暂时听不到"渲染成"接口坏了"。
///
/// # 参数
///
/// - `id`：组 id（`explosion`）；给了 `variant` 就用指定变体，否则取该组第一个。
/// - `variant`：**变体键**（`variant_stem` 之后的形态，例如 `Crowds-Amb_Terrasse_Resto`）。
///   也接受原始变体名（`Crowds/Amb_Terrasse_Resto.mp3`）—— 前端只发键，但老链接要能用。
pub(crate) async fn polish_sfx_preview(
    Extension(user): Extension<crate::auth::CurrentUser>,
    Query(q): Query<std::collections::HashMap<String, String>>,
    headers: axum::http::HeaderMap,
) -> Response {
    use yekitrim_polish::assets::{downloader, sfx_catalog};
    let id = q.get("id").map(String::as_str).unwrap_or("");

    // **用户包**的 id 形如 `<包名>/<变体键>`（见 `user_pack_list` 的 `id` 字段）——
    // 它们不在内嵌目录里，所以要在 `lookup` 之前分流。
    // ⚠️ 分流判据是"包名目录真的存在"（`sanitize_pack_name` 已挡掉分隔符/`..`），
    //    不是"id 里带 `/`" —— 后者的兜底会随 id 格式变化而失效。
    if let Some((pack, stem)) = id.split_once('/') {
        if let Some(resp) = user_pack_preview(&user, pack, stem, &headers).await {
            return resp;
        }
    }

    let Some(entry) = sfx_catalog::lookup(id) else {
        return crate::media::json_error(StatusCode::NOT_FOUND, "没有这个音效");
    };
    // 变体键：没给就用该组第一个
    let stem = match q.get("variant").map(String::as_str).filter(|v| !v.is_empty()) {
        Some(v) => sfx_catalog::variant_stem(v),
        None => entry
            .variants
            .first()
            .map(|v| sfx_catalog::variant_stem(v.as_str()))
            .unwrap_or_default(),
    };
    if stem.is_empty() {
        return crate::media::json_error(StatusCode::NOT_FOUND, "这个音效没有可试听的变体");
    }
    // ⚠️ 键必须真的属于这一组：`variant` 是用户输入，直接拿去拼路径等于开了个任意文件名口子
    let Some((_raw, url)) = sfx_catalog::variant_source(id, &stem) else {
        return crate::media::json_error(StatusCode::BAD_REQUEST, "variant 不属于这个音效");
    };
    // ⚠️ 再挡一道 `is_safe_component`：`stem` 来自用户输入，虽然已经过 `variant_source`
    //    的"必须真在 catalog 里"校验，但它是**文件名**，两道判据比一道稳。
    if !yekitrim_polish::assets::sticker_index::is_safe_component(&stem) {
        return crate::media::json_error(StatusCode::BAD_REQUEST, "变体名不合法");
    }
    // 缓存根在 blocking 池里建（`create_dir_all` 是同步 IO；目录可能落在移动盘/网络盘上）
    let cache_dir = yekitrim_core::path::cache_dir().join("sfx");
    if let Err((status, body)) = blocking({
        let dir = cache_dir.clone();
        move || {
            std::fs::create_dir_all(&dir).map_err(|e| {
                err(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("缓存目录创建失败: {e}"),
                )
            })
        }
    })
    .await
    {
        return (status, body).into_response();
    }
    let path = cache_dir.join(format!("{stem}.mp3"));
    if !path.is_file() {
        // ⚠️ 先写临时文件再 rename：两个浏览器标签同时点同一首时，
        // 后来的那个不能读到半个文件（Range 会对不上长度）。
        // 临时名带**进程号 + 纳秒**：两个并发请求各写各的，最后谁的 rename 先到就用谁的
        // （同一份源字节，内容一致；重名 `.part` 会互相截断）。
        let uniq = format!(
            "{}-{:x}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        );
        let tmp = cache_dir.join(format!(".{stem}.{uniq}.part"));
        match downloader::fetch_sfx_bytes(&url).await {
            Ok(bytes) => {
                if let Err(e) = tokio::fs::write(&tmp, &bytes).await {
                    let _ = tokio::fs::remove_file(&tmp).await;
                    return crate::media::json_error(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        &format!("缓存落盘失败: {e}"),
                    );
                }
                // Windows 上 rename 到**已存在**的目标会失败（另一个并发请求先落地了）——
                // 那不是错误：目标已经是同一份字节，删掉自己的临时文件即可。
                if let Err(e) = tokio::fs::rename(&tmp, &path).await {
                    let _ = tokio::fs::remove_file(&tmp).await;
                    if !path.is_file() {
                        return crate::media::json_error(
                            StatusCode::INTERNAL_SERVER_ERROR,
                            &format!("缓存落盘失败: {e}"),
                        );
                    }
                }
            }
            Err(_) => {
                let _ = tokio::fs::remove_file(&tmp).await;
                // 源站没有 / 断网 / 超时 —— 对用户来说都是"这一条暂时听不到"
                return crate::media::json_error(
                    StatusCode::NOT_FOUND,
                    "试听源抓取失败（源站不可达或该文件缺失）",
                );
            }
        }
    }
    crate::media::serve_file_from_headers(&path, "audio/mpeg", &headers).await
}


/// **用户包的试听**：直接从 `<data>/users/<uid>/sfx/<包名>/` 读字节，不走源站、不缓存。
///
/// 返回 `None` 表示"这个 id 不是用户包的形态"（由调用方继续走内嵌目录那条路）；
/// `Some(resp)` 是**最终响应**（成功 200、或者 4xx —— 都不要再落到 `lookup`）。
///
/// ⚠️ 与预设包那条的关键区别：预设包要**抓源站并缓存**（站方要求不转载 URL），
/// 用户包的音频就在本地磁盘上，直接 `serve_file_from_headers` 即可（Range 天然支持）。
/// ⚠️ `pack` 与 `stem` 都是用户输入：`sanitize_pack_name` / `is_safe_component` 两道挡，
/// 拼出来的路径还必须真在包目录里（`canonicalize` 比对，防符号链接逃逸）。
async fn user_pack_preview(
    user: &crate::auth::CurrentUser,
    pack: &str,
    stem: &str,
    headers: &axum::http::HeaderMap,
) -> Option<Response> {
    use yekitrim_polish::assets::{sfx_packs, sticker_index};
    let owner = crate::auth::owner_of(user);
    let Some(pack) = sticker_index::sanitize_pack_name(pack) else {
        return None;
    };
    let Some(stem) = sticker_index::sanitize_pack_name(stem) else {
        return Some(crate::media::json_error(
            StatusCode::BAD_REQUEST,
            "变体名不合法",
        ));
    };
    let dir = sfx_pack_root(owner).join(&pack);
    if !dir.is_dir() {
        return None; // 不是用户包 → 交给内嵌目录那条路（它会回 404）
    }
    // 变体键 → 磁盘文件名：`scan_pack` 是**唯一**的键来源（它自己套了 `stem_of`）
    let entries = blocking(move || Ok(sfx_packs::scan_pack(&dir)))
        .await
        .unwrap_or_default();
    let Some(entry) = entries.into_iter().find(|e| e.stem == stem) else {
        return Some(crate::media::json_error(
            StatusCode::NOT_FOUND,
            "这个音效包里没有这一条",
        ));
    };
    let file = sfx_pack_root(owner).join(&pack).join(&entry.file_name);
    // 纵深防御：解析后必须仍在包目录内（防软链接/竞态）
    let root = sfx_pack_root(owner).join(&pack);
    let (Ok(real), Ok(real_root)) = (file.canonicalize(), root.canonicalize()) else {
        return Some(crate::media::json_error(
            StatusCode::NOT_FOUND,
            "音频文件不在了",
        ));
    };
    if !real.starts_with(&real_root) {
        return Some(crate::media::json_error(
            StatusCode::BAD_REQUEST,
            "路径不合法",
        ));
    }
    let mime = match real.extension().and_then(|e| e.to_str()).unwrap_or("") {
        "mp3" => "audio/mpeg",
        "wav" => "audio/wav",
        "ogg" => "audio/ogg",
        "m4a" | "aac" => "audio/mp4",
        "flac" => "audio/flac",
        _ => "application/octet-stream",
    };
    Some(crate::media::serve_file_from_headers(&real, mime, headers).await)
}

/// 用户音效包根目录（`<data>/users/<uid>/sfx/`）。
fn sfx_pack_root(owner: yekitrim_core::owner::Owner) -> std::path::PathBuf {
    yekitrim_core::owner::asset_dir(owner, yekitrim_core::owner::AssetKind::Sfx)
}

/// 扫出来的一个用户音效包（内部形态；对外的 JSON 由调用方组装）。
///
/// ⚠️ 抽成结构体是为了**一份扫描喂两个消费者**：左侧 `packs` 卡片与中间 `entries` 网格
/// 都必须来自同一次 `scan_pack`，否则两边的组数会对不上。
struct UserPack {
    name: String,
    entries: Vec<yekitrim_polish::assets::sfx_packs::PackEntry>,
    /// 有没有 `pack.toml`（未构建 = LLM 生成事件时读不到这个包）
    built: bool,
    /// 清单是谁写的（`author` / `yekitrim`）；没有清单 = `None`
    built_by: Option<String>,
}

impl UserPack {
    fn groups(&self) -> Vec<String> {
        let mut g: Vec<String> = self.entries.iter().map(|e| e.group.clone()).collect();
        g.sort();
        g.dedup();
        g
    }
}

/// 列当前 owner 的**用户音效包**。
///
/// 同步 IO ⇒ 只在 blocking 池里调（[`polish_sfx_catalog`] 与导入/删除 handler 都是这么做的）。
/// ⚠️ 用户包是**运行期资产**，每次现扫、不缓存 —— 导入/删除之后必须立刻反映到目录里。
fn user_pack_list(owner: yekitrim_core::owner::Owner) -> Vec<UserPack> {
    use yekitrim_polish::assets::sfx_packs;
    let root = sfx_pack_root(owner);
    let Ok(rd) = std::fs::read_dir(&root) else {
        return Vec::new();
    };
    let mut dirs: Vec<std::path::PathBuf> = rd
        .flatten()
        .filter(|e| e.path().is_dir())
        .map(|e| e.path())
        // 暂存目录（`.upload-*`）不算包
        .filter(|d| {
            d.file_name()
                .and_then(|n| n.to_str())
                .map(|n| !n.starts_with('.'))
                .unwrap_or(false)
        })
        .collect();
    dirs.sort();
    dirs.into_iter()
        .filter_map(|dir| {
            let name = dir.file_name()?.to_str()?.to_string();
            let entries = sfx_packs::scan_pack(&dir);
            if entries.is_empty() {
                return None; // 空目录（比如导入到一半崩了）不出现在列表里
            }
            let manifest = sfx_packs::load_manifest(&dir);
            Some(UserPack {
                name,
                built: manifest.is_some(),
                built_by: manifest.as_ref().map(|m| {
                    if m.pack.built_by.is_empty() {
                        sfx_packs::BUILT_BY_AUTHOR.to_string()
                    } else {
                        m.pack.built_by.clone()
                    }
                }),
                entries,
            })
        })
        .collect()
}

/// 用户包条目是否被屏蔽：精确 id + `tag:xxx`（按**清单里的 tag**）。
///
/// ⚠️ `sfx_catalog::is_blacklisted` 只认内嵌目录（它内部 `lookup(id)`），用户包的 id
/// 在那里查不到 ⇒ `tag:xxx` 对用户包**恒不命中**。这条补上那一半。
/// 两半的语义必须一致：精确 id 屏蔽单条、`tag:xxx` 屏蔽整族。
fn user_pack_blacklisted(
    id: &str,
    tags: &[String],
    blacklist: &[String],
) -> bool {
    blacklist.iter().any(|b| match b.strip_prefix("tag:") {
        Some(tag) => tags.iter().any(|t| t == tag),
        None => b == id,
    })
}

/// 用户包条目的全局 id（`<包名>/<变体键>`）。
///
/// ⚠️ 与 `sfx_packs::entry_id` **同一份实现**：那个函数在 polish crate 里，
/// 这里直接转调，**不要再 `format!` 一遍** —— 两处各拼一次就是"两个 id 空间"的起点。
fn user_pack_entry_id(pack: &str, stem: &str) -> String {
    yekitrim_polish::assets::sfx_packs::entry_id(pack, stem)
}

/// 把一个用户包铺成**与预设条目同形状**的 JSON 列表。
///
/// ⚠️ 抽出来是为了能被单测直接钉住 —— 这段曾经漏掉整条（用户包只进了
/// `user_packs`、没进 `entries`），症状是"左侧卡片写着 N 组、点进去网格空白"，
/// 只有靠 `entries` 的输出才测得出来。
///
/// 形状约定（与预设条目的键**逐个对齐**，前端只有一条渲染路径）：
/// - `pack` = 包名（左侧卡片就按它过滤 `sfxPackEntries`）
/// - `group` = 空串（用户包只有一级细分 ⇒ 前端跳过一级 chips）
/// - `subgroup` = 包内分组（清单 / zip 子目录 / 「其他」）
/// - `variants` = `[stem]`（用户包一条就是一个变体）
/// - `built` = 有没有清单
fn user_pack_entries(pack: &UserPack, blacklist: &[String], include_blacklisted: bool) -> Vec<serde_json::Value> {
    let mut out = Vec::new();
    for e in &pack.entries {
        let id = user_pack_entry_id(&pack.name, &e.stem);
        let blocked = user_pack_blacklisted(&id, &e.tags, blacklist);
        if blocked && !include_blacklisted {
            continue;
        }
        out.push(serde_json::json!({
            "id": id,
            "label": e.label,
            "cat": "",
            "pack": pack.name,
            "group": "",
            "subgroup": e.group,
            "subcat": serde_json::Value::Null,
            "tags": e.tags,
            "variants": [e.stem],
            "blacklisted": blocked,
            "built": pack.built,
        }));
    }
    out
}

/// **导入用户音效包**（`POST /api/polish/sfx-packs/upload?name=<包名>`）。
///
/// 请求体就是 zip 的原始字节（与贴纸上传同一条口径，不是 multipart）。
/// zip 里的**子目录名 = 包内细分组**；文件一律拍平到包根（理由见 `sfx_packs` 模块头：
/// 出片侧按 `sfx/<stem>.mp3` 找文件，子目录会让它只 warn 一句「文件缺失」）。
pub(crate) async fn polish_sfx_pack_upload(
    Extension(user): Extension<crate::auth::CurrentUser>,
    Query(q): Query<std::collections::HashMap<String, String>>,
    body: axum::body::Bytes,
) -> ApiResult {
    use yekitrim_polish::assets::{sfx_packs, sticker_zip::ZipLimits};
    let owner = crate::auth::owner_of(&user);
    let name = q.get("name").map(String::as_str).unwrap_or("").trim().to_string();
    if name.is_empty() {
        return Err(err(StatusCode::BAD_REQUEST, "缺少包名 name"));
    }
    if body.is_empty() {
        return Err(err(StatusCode::BAD_REQUEST, "上传内容为空"));
    }
    let root = sfx_pack_root(owner);
    let pack = name.clone();
    // 同步解压 ⇒ blocking 池（可能有上百 MB）
    let report = blocking(move || {
        sfx_packs::import_pack_from_zip(&root, &pack, &body, &ZipLimits::default())
            .map_err(|e| err(StatusCode::BAD_REQUEST, format!("{e:#}")))
    })
    .await?;
    // `transcode_failed` 照实回给前端：那一批会保留原格式，出片按真实扩展名找
    // （不是错误，但用户该知道"有几个没转成 mp3"）。
    ok(serde_json::json!({
        "ok": true,
        "name": name,
        "copied": report.copied,
        "skipped": report.skipped,
        "transcode_failed": report.transcode_failed,
    }))
}

/// **LLM 构建用户音效包清单**（`POST /api/polish/sfx-packs/:name/build`）。
///
/// 与贴纸的 `POST /polish/stickers/:pack/build` 对应：读包内文件名 → 让 LLM 挑 tag →
/// 写 `<包名>/pack.toml`。**写的是清单，不动音频**。
///
/// # 与贴纸构建的关键差异
///
/// 贴纸 tag 是 LLM **自由发明**的；音效 tag 是**封闭词表**（内建 84 个）——
/// 词表外的会被 `filter_known_tags` 丢弃。实现与理由在
/// `yekitrim_polish::commands::build_sfx_pack_for`。
///
/// # 任务档位
///
/// `JobClass::Medium`（整批 LLM 调用，与 `sticker_build` 同档）+ 资源键
/// [`crate::scope_key::SFX_PACKS`]（与贴纸库分开：两个库互不相干，
/// 共用一个键会让贴纸构建挡住音效构建）。
pub(crate) async fn polish_sfx_pack_build(
    State(state): State<std::sync::Arc<crate::model::AppState>>,
    Extension(user): Extension<crate::auth::CurrentUser>,
    axum::extract::Path(name): axum::extract::Path<String>,
) -> ApiResult {
    use yekitrim_polish::assets::sticker_index::sanitize_pack_name;
    let name = sanitize_pack_name(&name)
        .ok_or_else(|| err(StatusCode::BAD_REQUEST, "包名不合法"))?;
    let owner = crate::auth::owner_of(&user);
    // 包必须真实存在（读目录是同步 IO ⇒ blocking 池）
    let dir = sfx_pack_root(owner).join(&name);
    let exists = blocking(move || Ok(dir.is_dir())).await?;
    if !exists {
        return Err(err(StatusCode::NOT_FOUND, format!("音效包不存在: {name}")));
    }
    let handle = state
        .jobs
        .create(
            user.id,
            crate::jobs::JobSpec::new(
                "sfx_pack_build",
                format!("用 LLM 构建「{name}」的音效标签"),
                vec![crate::scope_key::SFX_PACKS.to_string()],
                crate::jobs::JobClass::Medium,
            )
            .background(),
        )
        .await;
    let id = handle.id.clone();
    let job_id = id.clone();
    let jobs = state.jobs.clone();
    let cancel = jobs.cancel_token(&job_id).await;
    // 无管线阶段框架：expected 空（经 stage_started("音效包标签") 自报阶段）
    let progress = crate::api::core::progress_bridge(job_id.clone(), jobs.clone(), cancel, Vec::new());
    let pack = name.clone();
    crate::api::spawn_job(jobs.clone(), job_id.clone(), move || async move {
        if handle.wait_start().await.is_err() {
            jobs.complete(&job_id).await;
            return;
        }
        // ⚠️ **必须传 `data_owner`**（理由同 `stickers::spawn_build`）：任务跑在 server
        //    进程里，`Owner::from_env()` 拿到的是 server 自己的环境，多用户下会翻错库。
        match yekitrim_polish::commands::build_sfx_pack_for(owner, &pack, progress.as_ref()).await {
            Ok(r) => {
                jobs.log(
                    &job_id,
                    "info",
                    format!(
                        "清单已写出：{} 条音效，{} 个 tag{}",
                        r.files,
                        r.tags,
                        if r.dropped_tags > 0 {
                            format!("（丢弃 {} 个词表外 tag）", r.dropped_tags)
                        } else {
                            String::new()
                        }
                    ),
                )
                .await;
                jobs.complete(&job_id).await;
            }
            Err(e) => {
                jobs.log(&job_id, "error", format!("{e:#}")).await;
                jobs.fail(&job_id, format!("{e:#}")).await;
            }
        }
    });
    ok(serde_json::json!({ "ok": true, "id": id }))
}

/// **删除用户音效包**（`DELETE /api/polish/sfx-packs/:name`）。
/// ⚠️ 只删**用户包**：预设包不在磁盘上，`lookup` 走的是内嵌目录，
/// 这里对 `①`-去序号后的预设名会先被 `is_reserved` 那层挡掉（见 `sfx_packs`）。
pub(crate) async fn polish_sfx_pack_delete(
    Extension(user): Extension<crate::auth::CurrentUser>,
    axum::extract::Path(name): axum::extract::Path<String>,
) -> ApiResult {
    use yekitrim_polish::assets::sticker_index::sanitize_pack_name;
    let owner = crate::auth::owner_of(&user);
    let Some(name) = sanitize_pack_name(&name) else {
        return Err(err(StatusCode::BAD_REQUEST, "包名不合法"));
    };
    if yekitrim_polish::assets::sfx_catalog::CATEGORY_ORDER
        .iter()
        .any(|c| yekitrim_polish::assets::sfx_packs::category_bare_name(c) == name)
    {
        return Err(err(StatusCode::BAD_REQUEST, "内建预设包不能删除"));
    }
    let dir = sfx_pack_root(owner).join(&name);
    let present = blocking(move || {
        let ok = dir.is_dir();
        if ok {
            std::fs::remove_dir_all(&dir)
                .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("删除失败: {e}")))?;
        }
        Ok(ok)
    })
    .await?;
    if !present {
        return Err(err(StatusCode::NOT_FOUND, format!("没有音效包「{name}」")));
    }
    ok(serde_json::json!({ "ok": true, "name": name }))
}

/// **音效屏蔽**（`PUT /api/polish/sfx-blacklist`）—— 素材库「音效」页的"不看这一类"。
///
/// # 语义
///
/// body：`{"blacklist": ["tag:恐怖氛围", "white_noise", ...]}` —— **整份覆盖写**
/// 当前 owner `config.toml` 的 `[sfx] blacklist`。值是精确 id 或 `tag:xxx`（整族），
/// 与 `sfx_catalog::is_blacklisted` 同一套语义（唯一一份判据在那里）。
///
/// # 为什么是覆盖而不是增删单条
///
/// 这一页天然是"整张表打勾"的形状（取消勾选 = 从列表里去掉），覆盖写让前端只维护
/// 一个数组、不担心两条并发 patch 合并出中间态。代价是**前端必须先拿到当前那份**
/// （`GET /polish/sfx-catalog` 顶层 `blacklist` 已回）再改 —— 别在没读到的情况下盲写。
///
/// # 写法
///
/// 走 `patch_config_file`（toml_edit 合并语义、保留注释）：`[sfx] blacklist = [...]`
/// 一项数组值。**不要**用"读整份 Config 再写回"那条路 —— 那会按结构体重新序列化，
/// 把用户 config.toml 里的注释与其他手写键抹掉。
pub(crate) async fn polish_sfx_blacklist_put(
    Extension(user): Extension<crate::auth::CurrentUser>,
    Json(body): Json<serde_json::Value>,
) -> ApiResult {
    let owner = crate::auth::owner_of(&user);
    // ⚠️ 同步 IO（扫用户包目录）⇒ 先整体搬进 blocking 池算一遍「合法的用户包 id 集合」。
    let user_ids_owner = owner;
    let user_pack_ids: std::collections::HashSet<String> = blocking(move || {
        Ok(user_pack_list(user_ids_owner)
            .iter()
            .flat_map(|p| {
                p.entries
                    .iter()
                    .map(|e| user_pack_entry_id(&p.name, &e.stem))
                    .collect::<Vec<_>>()
            })
            .collect::<std::collections::HashSet<String>>())
    })
    .await?;
    let Some(list) = body.get("blacklist").and_then(|v| v.as_array()) else {
        return Err((StatusCode::BAD_REQUEST, Json(serde_json::json!({
            "ok": false, "error": "缺少 blacklist 数组"
        }))));
    };
    let mut cleaned: Vec<String> = Vec::new();
    for v in list {
        let Some(s) = v.as_str() else {
            return Err((StatusCode::BAD_REQUEST, Json(serde_json::json!({
                "ok": false, "error": "blacklist 只能全是字符串"
            }))));
        };
        let s = s.trim();
        if s.is_empty() {
            continue;
        }
        // 形状校验：`tag:xxx` / catalog id / 用户包 id（`<包名>/<变体键>`）；
        // 其余一律 400（别把路径/任意串写进配置）
        if let Some(tag) = s.strip_prefix("tag:") {
            if !yekitrim_polish::assets::sfx_catalog::all_tags()
                .iter()
                .any(|(t, _)| *t == tag)
            {
                return Err((StatusCode::BAD_REQUEST, Json(serde_json::json!({
                    "ok": false, "error": format!("未知的音效 tag: {tag}")
                }))));
            }
        } else if yekitrim_polish::assets::sfx_catalog::lookup(s).is_none()
            && !user_pack_ids.contains(s)
        {
            // ⚠️ 用户包条目也要认（`<包名>/<变体键>`）：`lookup` 只查内嵌目录，
            //    不补这一条的话素材库里对导入包点「屏蔽这一组」**必失败**
            //    （400「未知的音效 id」），而界面上的按钮看起来一切正常。
            return Err((StatusCode::BAD_REQUEST, Json(serde_json::json!({
                "ok": false, "error": format!("未知的音效 id: {s}")
            }))));
        }
        if !cleaned.contains(&s.to_string()) {
            cleaned.push(s.to_string());
        }
    }
    let path = yekitrim_core::owner::config_path(owner);
    let patch = serde_json::json!({ "sfx": { "blacklist": cleaned } });    let _guard = yekitrim_core::filelock::lock_path(&path).await;
    blocking(move || {
        yekitrim_core::config::patch_config_file(&path, &patch)
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({
                "ok": false, "error": format!("音效屏蔽保存失败: {e:#}")
            }))))
    })
    .await?;
    ok(serde_json::json!({ "ok": true }))
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

    /// 用户包的屏蔽判据：精确 id + `tag:xxx`（按**清单里的 tag**）。
    ///
    /// ⚠️ 这条补的是 `sfx_catalog::is_blacklisted` 的另一半：那个函数内部 `lookup(id)`
    /// 只查内嵌目录，用户包的 id 在那里查不到 ⇒ `tag:xxx` 对用户包**恒不命中**。
    /// 两半的语义必须一致（精确 id 屏蔽单条、`tag:xxx` 屏蔽整族）。
    #[test]
    fn 用户包屏蔽判据认精确id与tag两档() {
        let tags = vec!["敲门推门".to_string(), "打击格斗".to_string()];
        // 精确 id
        assert!(user_pack_blacklisted("我的包/敲门", &tags, &["我的包/敲门".into()]));
        // tag 整族
        assert!(user_pack_blacklisted("我的包/敲门", &tags, &["tag:敲门推门".into()]));
        // 不相干的不许误伤
        assert!(!user_pack_blacklisted("我的包/敲门", &tags, &["tag:雨雷".into()]));
        assert!(!user_pack_blacklisted("我的包/敲门", &tags, &["我的包/下雨".into()]));
        // 没有清单（`tags` 空）⇒ 只有精确 id 那条能命中
        assert!(!user_pack_blacklisted("我的包/敲门", &[], &["tag:敲门推门".into()]));
        assert!(user_pack_blacklisted("我的包/敲门", &[], &["我的包/敲门".into()]));
        // 空黑名单恒 false
        assert!(!user_pack_blacklisted("我的包/敲门", &tags, &[]));
    }

    /// 用户包的条目**必须**铺成与预设同形状，且 `pack` 必须是包名。
    ///
    /// ⚠️ 这条钉的是那个真 bug：用户包曾经只进 `user_packs`、没进 `entries` ——
    /// 左侧卡片（读 `packs[].count`）写着"N 组"，点进去网格（读 `entries` 按
    /// `pack === 包名` 过滤）**一条都没有**。前端只读 `entries`，所以只有在这里
    /// 断言形状才拦得住回归。
    #[test]
    fn 用户包条目铺成与预设同形状且pack是包名() {
        use yekitrim_polish::assets::sfx_packs::PackEntry;
        let pack = UserPack {
            name: "我的包".into(),
            built: true,
            built_by: Some("yekitrim".into()),
            entries: vec![
                PackEntry {
                    stem: "敲门".into(),
                    label: "敲铁门".into(),
                    group: "门窗".into(),
                    file_name: "敲门.mp3".into(),
                    tags: vec!["敲门推门".into()],
                    gain_db: None,
                },
                PackEntry {
                    stem: "下雨".into(),
                    label: "下雨".into(),
                    group: "天气".into(),
                    file_name: "下雨.mp3".into(),
                    tags: vec![],
                    gain_db: None,
                },
            ],
        };
        let out = user_pack_entries(&pack, &[], false);
        assert_eq!(out.len(), 2, "两条都要铺出来：{out:?}");

        let first = &out[0];
        // ⚠️ `pack` 是左侧卡片的过滤键：写成空串 = 前端网格永远空
        assert_eq!(first["pack"], "我的包");
        assert_eq!(first["id"], "我的包/敲门", "id 必须走 entry_id 那一份实现");
        assert_eq!(first["label"], "敲铁门");
        assert_eq!(first["group"], "", "用户包没有大类这一层");
        assert_eq!(first["subgroup"], "门窗");
        assert_eq!(first["variants"], serde_json::json!(["敲门"]));
        assert_eq!(first["tags"], serde_json::json!(["敲门推门"]));
        assert_eq!(first["built"], true);
        assert_eq!(first["blacklisted"], false);
        // 键集合必须与预设条目**逐个对齐**（前端只有一条渲染路径）
        for k in ["id", "label", "cat", "pack", "group", "subgroup", "tags", "variants", "blacklisted", "built"] {
            assert!(first.get(k).is_some(), "缺字段 {k}：{first}");
        }
    }

    /// 被屏蔽的用户包条目：默认过滤掉；`include_blacklisted` 时出现且带位。
    #[test]
    fn 用户包条目按屏蔽位过滤且带blacklisted标记() {
        use yekitrim_polish::assets::sfx_packs::PackEntry;
        let pack = UserPack {
            name: "包".into(),
            built: false,
            built_by: None,
            entries: vec![PackEntry {
                stem: "a".into(),
                label: "A".into(),
                group: "其他".into(),
                file_name: "a.mp3".into(),
                tags: vec!["敲门推门".into()],
                gain_db: None,
            }],
        };
        let blocked = vec!["tag:敲门推门".to_string()];
        assert!(user_pack_entries(&pack, &blocked, false).is_empty(), "默认应过滤掉");
        let with = user_pack_entries(&pack, &blocked, true);
        assert_eq!(with.len(), 1);
        assert_eq!(with[0]["blacklisted"], true);
        assert_eq!(with[0]["built"], false, "没清单 ⇒ 未构建");
    }

}