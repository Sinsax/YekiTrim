# assets/ — 贴纸库、音效目录与资产下载

> 范围：`crates/yekitrim-polish/src/assets`/** ｜ 上游：`process`（贴纸解析/落点/音效）、`llm`（贴纸候选与 tag 构建）、`commands::build_package_tags`、`yekitrim-server::api`（上传/列表/预览）｜ 验证：`cargo test -p yekitrim-polish --lib assets`

## 职责

贴纸与音效两类素材的**库 + 索引 + 导入 + 下载**。`sticker_index.rs` 是贴纸库的核心（包扫描、tag 索引、
包内选文件、本地候选文本、包名校验）；`sticker_zip.rs` 是上传 zip → 新包；`sfx_catalog.rs` 是内建音效目录
（tag 族、屏蔽、候选取样、模糊搜索）；`emoji_search.rs` / `irasu_search.rs` 是 emoji 与 いらすとや 的检索与下载；
`downloader.rs` 是统一的 `AssetDownloader`（贴纸 + SFX 落盘）。

## 入口

- `sticker_index`：`scan_packages(root) -> Vec<StickerPackage>`、`pick_package_file(packages, name, seed)`、
  `local_sticker_candidates_text(root)`、`is_package_source(root, source)`、`load_sticker_index` / `save_sticker_index`、
  `register_package` / `unregister_package` / `remove_package`、`import_package(root, name, src) -> ImportReport`、
  `list_package_files`、`load_package_meta` / `build_package_meta_from_tags` / `package_meta_to_toml` / `tag_files`、
  `file_tags_map`、**`is_image_file_name(name)` / `is_image_path(p)`（唯一扩展名判据）**、
  `is_safe_component(s)`、`sanitize_pack_name(raw) -> Option<String>`、`StickerIndex` / `StickerPackage` / `MatchResult`。
- `sticker_zip`：`import_package_from_zip(root, name, bytes)` / `..._with(root, name, bytes, &ZipLimits)`、
  `ZipLimits { max_entries, max_file_bytes, max_total_bytes }`（默认 2000 / 32 MiB / 256 MiB）、`ZipImportReport`。
- `sfx_catalog`：`all()` / `lookup(id)` / `lookup_variant(file_stem)` / `lookup_label(label)`、
  `is_blacklisted(id, blacklist)`、`search_by_tags` / `candidates_for_tags(target_tags, used, limit, blacklist)` /
  `candidates_text(...)`、`fuzzy_search(_excluding)`、`categories_text()` / `CATEGORY_ORDER` / `all_tags()`、
  `variant_url(entry, variant)`、`SfxEntry`。
- `emoji_search`：`EmojiCatalog`（`search`）、`download_images` / `download_images_by_codepoints`、`collect_keywords(segments)`。
- `irasu_search`：`download_images` / `resolve_image_urls` / `collect_keywords(segments)`、`PLACEHOLDER_PNG`。
- `downloader`：`AssetDownloader::{new, download_stickers, …}`、`DownloadReport { sticker_downloaded, sticker_existing, sfx_downloaded, sfx_existing, errors }`、`collect_sfx_ids(segments)`。

## 边界

- 库固定在 `yekitrim_core::path::sticker_dir()` = **`<程序目录>/stickers/`**（不是项目里那份拷贝）：
  一个子文件夹 = 一个包，`<包名>.toml` = LLM 压出来的 tag 索引，`sticker.toml` = 已构建包的注册表。
  **debug 与 release 实例各有一份库，互不相干**（改包别只改一个）。
- ⚠️ **项目要用的是"复制进项目的那一份"**：`AssetDownloader::download_stickers` 把事件**选中的那个文件**
  复制到 `<项目>/assets/stickers/`，而**出片与预览只读项目目录**（`prepare_sticker_data` 解帧、
  `build_sticker_map` 给 DOM）。这是"单个 polish 项目能独立复现"的唯一保证 —— 库改名/换图/换机器之后
  老项目照样出得来。
  两条硬规矩：① 判据取**事件自己的 `source`**（`process` Step 2 落过盘的那个），`sticker_source`
  只给"没有 `source` 的老工程"兜底轮询（以前拿 `sticker_source` 非空当门控，用户清空来源列表后
  **一个文件都不复制**，项目就悄悄不自包含了）；② **复制失败必须报出来**（`report.errors` +
  `process` 的逐条 warn）—— "项目里没有这张图"以前是一条看不见的失败，日志一个字都没有，
  出片时贴纸凭空消失。
- 列表**以磁盘为准**（`scan_packages`）：手工把文件夹丢进 `stickers/` 也会出现。
  「未构建」= 没有 `<包名>.toml`：图能看，但 LLM 生成事件时读不到这个包
  （`local_sticker_candidates_text` 只列已构建的，`fill_default_local_packages`
  也只认 `sticker.toml` 里注册过的）。
- **上传 zip 的处理**（`yekitrim_polish::assets::sticker_zip`，与 `import_package` 同处）：
  **只取条目 basename**（`../../x.gif`、`/abs/x.gif`、`..\x.gif` 因此都只会落到
  `stickers/<包名>/x.gif`，zip-slip 不需要额外的 containment 校验）；跳过目录条目、
  `__MACOSX/` 与 `.` 开头的条目（苹果伴随文件叫 `._x.gif`，**它有 .gif 后缀**，只按扩展名过滤会收进库里）、
  非图片（判据是 `sticker_index::is_image_file_name`，**唯一一份**，别在各处再抄一遍扩展名清单）；
  子目录**摊平**，摊平后重名**只留第一个**、其余计入 `skipped`。
  **先解到暂存目录 `stickers/.upload-<pid>-<rand>/`，成功后 `fs::rename` 成 `<包名>/`**
  —— 任何失败路径都 `remove_dir_all(暂存)`，所以**不存在半个包**；暂存名以 `.` 开头，
  而 `scan_packages` 只认非 `.` 开头的目录、`sanitize_pack_name` 也拒绝 `.` 开头，
  所以它既不出现在列表里也访问不到。
  闸门：请求体 64 MiB（路由上的 `DefaultBodyLimit`，只给这一条放开，axum 默认才 2 MB）+
  `ZipLimits` 的条目数 2000 / 单文件 32 MiB / 解压总量 256 MiB；单文件那层按**实际写入字节数**
  判（`take(limit+1)` 后比对），所以中央目录里谎报尺寸没用。
  ⚠️ **这是全站第二处有意接收用户上传的入口**（第一处是封面背景图 base64）；
  「服务器文件建项目」那条仍然**永不接受上传**。别按旧规矩（"全站没有 multipart、不接受上传"）
  把这条删掉 —— 它是有意做的，闸门在上面。
  ⚠️ 前端 `webui/src/api/stickers.js::UPLOAD_MAX_BYTES` 与 Rust `yekitrim-server/src/stickers.rs::UPLOAD_MAX_BYTES`
  是同一个数（改一处要改两处），前端必须先挡：axum 超限回的是 **413 + 纯文本**，
  而 `client.js` 只认 JSON，会把"文件太大"报成"接口不存在或返回格式错误"。
  上传**没有进度条**（`fetch` 拿不到上传进度），只显示「上传中…」。
  zip 依赖钉在 `zip = "2.4.2"`（`default-features = false, features = ["deflate"]`）：
  该版本连同全部传递依赖都在本机 cargo 缓存里，**4.6.1 的 deflate 走 `flate2/zlib-rs`，那份没缓存**。
- 目录导入**接口保留但没有 UI**：`POST /api/admin/stickers/import {dir,name,build}`
  （`import_package`：把服务器上一个目录里的图片**复制**进 `stickers/<包名>/`，来源目录原样留着）
  —— 留给「服务器上已经有一堆图」时用 curl 导入。两条入口的共同规矩：目标已存在一律拒绝、
  绝不合并；一张图都没进来（复制失败 / 压缩包里没有图片）就报 400 且不留目录。
- 「LLM 压缩构建」= `yekitrim_polish::commands::build_package_tags`（CLI `polish stickers build`
  的同一份实现）：读文件名产出 10~20 个语义 tag → 写 `<包名>.toml` → 注册进 `sticker.toml`。
  走 JobManager（kind `sticker_build`，占 `stickers` 资源键：另一个构建在跑时**排队**），
  进度经 `api::core::progress_bridge` 上报，取消在**批次边界**生效。
  上传时勾了「上传后立刻构建」就顺带起它；**导入成功与构建任务是两件事** ——
  构建起不来只回一句 `job_error`，上传照样算成功。
- `sfx_catalog` 的屏蔽语义：精确 id（如 `fear-1`）屏蔽单个音效；`tag:xxx`（如 `tag:surprise`）
  屏蔽整个标签族。项目级 `polish.toml [sfx] blacklist` 与全局 `.yekitrim/config.toml [sfx] blacklist`
  由 `process::load_sfx_blacklist` 求并集。
- 只读接口（非 admin 可读）：`GET /api/polish/sticker-library`（磁盘为准）与
  `GET /api/polish/sticker-file`（缩略图）；音效列表来自 `GET /api/polish/sfx-catalog`。
  写操作仍然只在 admin 侧，别把库管理接口开到 `/polish/*`。
- 管理接口（全部 admin）：`GET /api/admin/stickers`（列表）/ `GET .../:pack`（包内文件 + 每个文件的
  tags，展开时才拉）/ `POST .../upload?name=&build=`（**上传 zip**）/ `POST .../import {dir,name,build}` /
  `POST .../:pack/build` / `DELETE .../:pack`（目录 + 清单 + 索引项一起删）/
  `GET .../:pack/files/:name`（预览图，与 `/api/media/*` 共用 `media::serve_file` 的 Range 实现）。
  包名过 `sanitize_pack_name`、文件名过 `is_safe_component`，再 canonicalize 要求落在库内。
- `is_image_file_name` 是**唯一**的图片扩展名判据 —— 上传、目录导入、包扫描、预览四处都走它，
  别在任何地方再抄一份扩展名清单。
- ⚠️ `is_safe_component` 挡路径拼接（删除片段/贴纸文件名）是**既有约定，别松**；
  与之配套的 `every /api/media` 走 `jail_existing` 同样别松。
- 贴纸 `name` 是**去扩展名的文件名**（与 `copy_sticker_from_local` 的匹配一致）；
  `emoji_search` / `irasu_search` 的 `collect_keywords` 决定"哪些词值得去找图"。

> 改动只允许落在本目录；跨模块改动先读对方卡，并跑两条门禁。

## 验证

```bash
cargo test -p yekitrim-polish --lib assets
```

含 zip 闸门 / zip-slip / 暂存目录 / 摊平重名 / 非图片过滤的回归。动了贴纸库后
CLI 侧再手验一次：`yekitrim polish stickers list` 与 `yekitrim polish stickers test <kw...>`。
