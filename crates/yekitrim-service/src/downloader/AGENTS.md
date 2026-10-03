# downloader — 下载器端口与 yt-dlp 适配器（**不 spawn、不认识 B站**）

> 范围：`crates/yekitrim-service/src/downloader`/** ｜ 上游：`yekitrim-server::download_worker`（唯一的执行方）｜ 验证：`cargo test -p yekitrim-service downloader`

## 职责

B站链接通道（P4）的**引擎侧**：引擎无关的端口（trait + 纯数据类型 "要跑什么命令"）、yt-dlp 适配器
（argv 构造 / stdout 哨兵分流 / 错误分类）、以及 yt-dlp 的**供应链**（版本 + sha256 钉死的注册表与查找/下载）。
本目录只产出**数据**，执行在 `yekitrim-server/src/download_worker.rs`。
设计依据与实测记录：`docs/plan/p4-implementation-plan.md`、`docs/plan/p4-prerequisite-verification.md`、
`docs/plan/p4-bilibili-downloader-selection.md`。

## 入口

- **端口**（`mod.rs`）：`DownloaderKind::{YtDlp, External(PathBuf)}` + `parse` / `label`；
  `DownloadRequest { url, out_dir, out_template, max_height, rate_limit_kbps, cookie_file, playlist_item }`
  （`new(url, out_dir)` 给默认值）；`trait Downloader { name, resolve_spec, download_spec, parse_stream_line,
  parse_resolve, classify }`；`for_kind(kind, program)`（**唯一的装配点**）；`check_playlist_item`。
- **命令即数据**：`CommandSpec { program, args, env_remove }` + `new/arg/args/display_line`；
  `PROXY_ENV_VARS`（8 个大小写代理变量）。`StreamEvent::{Progress, FinalPath, Meta}`；
  `Downloaded` / `ResolveInfo` / `ResolvedPage`（`pages` 空 = 单P）。
- **错误 → 人话**：`DownloadErrorKind`（`UnsupportedUrl` / `NotFound` / `NeedLogin` / `MemberOnly` /
  `ExtractorBroken` / `FormatUnavailable` / `RateLimited` / `ToolMissing` / `DiskFull` / `Other`）+
  `DownloadErrorKind::{as_str, user_message}` 与 `DownloadError::{new, user_facing}`。
  这个字符串会直接落进 `ingest_sessions.error` 并显示在界面上，**不能是原始 stderr**。
- **yt-dlp 适配器**（`ytdlp.rs`）：`YtDlp::{new, format_selector}`；哨兵常量
  `SENTINEL_PROGRESS`(`YEKITRIM_PROG:`) / `SENTINEL_PATH`(`YEKITRIM_PATH:`) / `SENTINEL_META`(`YEKITRIM_META:`)；
  `relative_input_path(project_dir, final_path)`（真实产物路径 → 写 `yekitrim.toml` 的项目相对名）。
- **供应链**（`registry.rs`）：`pin()` / `VERSION` / `LICENSE` / `managed_dir()`（`<程序目录>/.yekitrim/bin/`）/
  `managed_path()` / `YtDlpSource` / `YtDlpInstall { path, source, verified }` / `verify_sha256` /
  `find_ytdlp(cfg)` / `ready(cfg)` / `download_managed()` / `ensure_ytdlp(cfg)`。

## 边界

- ⚠️ **本目录不许开子进程**：判据是 `process::Command` / `Command::new` / `.spawn(`（门禁
  `downloader 不许开子进程`）。真正的 `tokio::process::Command` 只在 `yekitrim-server/download_worker.rs`。
  收益：命令构造与输出解析**完全离线可测**，不需要真跑一个下载器。
- ⚠️ **本目录不许知道 B站**：判据是 `bvid` / `cid` / `bilibili.com` / `comment.bilibili`（门禁
  `downloader 不许知道 B站`）。这里只出现 "URL"、"清晰度上限"、"cookie 文件"这类通用概念 ——
  引擎是通用端口，沾上 B站就只能适配一家，加第二个引擎要改这个目录。
- 两条都已是脚本门禁：`python3 scripts/check_module_boundaries.py`（**改了本目录就顺手跑**——
  判据写在文档里会腐坏，写成脚本才会）。
- **加第二个引擎怎么做**：加一个 `Downloader` 实现 + 在 `for_kind` 里挂上即可，`server` 侧一行不改。
  逃生舱（`<绝对路径>`）走的就是这条：它是**同一个适配器**指向另一个二进制。
  `DownloaderKind::parse` 对不认识的名字（`"bbdown"` 之类）**一律当路径**，会以"可执行文件不存在"
  在启动期暴露，而不是被静默当成 yt-dlp 用错参数。

### yt-dlp 命令构造（三条实测出来的硬约束，改参数前先读）

- ⚠️ **哨兵前缀必须在 `WHEN` 段之后**：`--print "after_move:YEKITRIM_PATH:%(filepath)s"`。
  写成 `"YEKITRIM_PATH:after_move:filepath"` 会输出 **`NA`**（`YEKITRIM_PATH` 被当成模板名）。
- ⚠️ **非 TTY 下一行进度都没有**，必须显式 `--progress`；而且 `--progress-template`
  与 `--print` 的输出**都落在 stdout**，只能靠前缀分流（不能假设"最后一行是路径"）。
- ⚠️ **格式串必须带兜底**：`-f "b[height<=360]/b"` 在"该视频没有这个档位"时 yt-dlp 直接 exit 1
  （`Requested format is not available`），**不会自动回退**。兜底串由 `YtDlp::format_selector` 给出：
  `bv*[height<=N]+ba/b[height<=N]/bv*+ba/b`（不限高时 `bv*+ba/b`）。
- **`-S vcodec:h264` 必须带**：默认会选到 HEVC，WebUI 播不了。
- cookie 必须是 **Netscape 格式**（JSON 会被静默忽略 —— 表现为"配了 cookie 还是低清"）；
  文件不存在直接 `bail`。
- ⚠️ `--no-playlist` 等价于"只下第 1 个"，与 `--playlist-items N` 同时给会打架 → **二选一**：
  指定了第 N P 就用 `--playlist-items`，否则 `--no-playlist`（防一个多P 链接把整季都下下来）。
- 输出模板用**绝对路径**而不是 `cd`：worker 不切换进程工作目录（多任务并发时那是共享状态）。
- ⚠️ **产物路径不许猜**：标题里的 `/ \ : * ? " < >` 会被消毒，只能信 `--print after_move:filepath`。

### 输出解析与错误分类

- `parse_stream_line` 只认三个哨兵前缀，其余行返回 `None`；进度百分比解析不出来就**跳过这一行**
  （不 panic、不报假进度）。
- `parse_resolve` 吃 `-J` 的 JSON：多P 看 `_type == "playlist"` 或 `entries` 存在；
  ⚠️ `entries` 里可能有 `null`（yt-dlp 对失败的条目就是塞 null），必须过滤掉；
  分P 名优先 `part` / `chapter` 再 `title`；字段可能回 `NA`。
- `classify` **只看带 `ERROR:` 的行**：yt-dlp 的 WARNING 里也会出现 `HTTP Error 404` 这类字样
  （例如封面 404 但它照样把视频下完了），扫全文会把"成功的下载"判成失败。
  规则顺序有讲究（"更具体"的在前）：`ExtractorBroken` 要排在 `NotFound` 之前，否则认错了会误导用户去重试。
  **第一行 ERROR 命中哪条就认哪条**（后面的 ERROR 往往是次生错误）；一条都没命中 → `Other`（保留原文便于上报）。
  每条关键字都来自实测（`p4-prerequisite-verification.md` T7）。

### 供应链（`registry.rs`）

- ⚠️ 托管副本的 marker **就是 sha256 本身** —— 半下载的 exe / 被替换过的 exe 都不算"就绪"。
  **别照抄 `yekitrim-core/download.rs::ensure_one_model` 的 marker 判定**（那是"某个文件存在"）。
- ⚠️ **换版本时必须三件事一起做**：改 `VERSION`、改这里每个平台的 `sha256` 与 `size`、
  并**重新实测**（下下来跑一次 `--version`）。只改版本号会让校验失败 → 下载通道整个不可用。
- ⚠️ **只有 Windows** 那份 `yt-dlp.exe` 是实测校验过的（下载后与官方 `SHA2-256SUMS` 逐字节一致），
  所以只有它带 sha256。其它平台**不允许自动下载** —— 那些平台上 `ensure_ytdlp` 会明确报错，
  让管理员用 `[ingest] ytdlp_path` 指定自备的那份（或装到 PATH）。这是刻意的：**只钉我们真正验证过的东西。**
- 查找顺序（`find_ytdlp`，纯查找不下载）：**`[ingest] ytdlp_path` → 托管目录（必须哈希匹配）→ PATH**。
  配置指定**不校验**哈希（否则管理员换新版就被拦下了）；托管目录里的那份如果哈希不对，
  `find_in_path` 也可能命中它 —— 要排除掉，避免把一份损坏的托管副本当成"PATH 上的 yt-dlp"用。
- ⚠️ **`find_ytdlp` 会做 sha256 校验（要读完整份 17.8MB 的 exe）**，所以 `api::health` 里
  **只查一次，别在 `json!` 里连着调两遍**（health 是前端反复轮询的接口；注册表侧另有 `(path, len, mtime)`
  缓存兜底，两次调用仍多余）。
- `ready(cfg)` 只回布尔 + 来源标签，**不回路径**（与 ffmpeg 那些能力位一个口径）。
- `download_managed` 失败时**不留半成品**：`.part` 会被删掉（半下载的 exe 被当成可用才是最难查的那类故障）；
  sha256 不匹配时提示"可能是网络中间人、镜像被替换或 release 已变更 —— 请勿使用"。
- ⚠️ `ensure_ytdlp` 最后一步失败**就报错，不静默降级** —— 下载通道宁可明确失败，也不要"用了半份二进制"；
  托管目录里有一份但哈希不对 → 先删掉再下。

> 改动只允许落在本目录；跨模块改动先读对方卡，并跑两条门禁。

## 验证

```bash
cargo test -p yekitrim-service downloader
```

适配层的单测**全部离线可跑**：不打真实网络、不 spawn 进程 —— argv 构造、哨兵分流（含"`YEKITRIM_PATH` 写在
`WHEN` 之前会得到 `NA`"）、`classify` 的实测关键字、注册表校验。改了本目录必须再跑
`python3 scripts/check_module_boundaries.py`（两条路径级禁则）。
