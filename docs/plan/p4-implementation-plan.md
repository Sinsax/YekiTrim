# P4「B站链接通道」落地计划

> 依据：**`p4-prerequisite-verification.md`（2026-09-19 实测）** + `p4-bilibili-downloader-selection.md`（选型）。
> 本计划的所有命令与参数都有实测支撑；实测没覆盖的已在 §6 标为不确定项，**不要当成已验证**。

## 0. 相对原设计的修订（全部由实测驱动）

| # | 原设计 | 改为 | 依据 |
|---|---|---|---|
| 1 | `cid` 从 `yt-dlp -J` 取 | **从 B站 view API 取**（裸 GET，免 WBI 免 cookie，一次给全部分P 的 cid + P名 + 时长 + 弹幕数） | 实测报告 T3：普通 `-J` 里 `cid` 出现 0 次 |
| 2 | 担心 `--sub-langs danmaku` 不落盘，故自取 | 自取**保留**，但理由改为「职责分离 + 可独立重试 + resolve 阶段可预览」 | T5：落盘实测正常，原理由不成立 |
| 3 | 弹幕解析「零改动复用」 | **必须改** `parse_bililive_xml`（真实 XML 是 9 段、`[7]` 是 dmid） | T9：3600/3600 条 y 全塌成 1500 |
| 4 | 只要求 `--print after_move:filepath` | 另需 `-S vcodec:h264`、格式串兜底、`--progress`+哨兵前缀、cookie 须 Netscape | T6 的 4 条约束 |
| 5 | 错误分类关键字（`Sign in to confirm` 等） | 按实测关键字重写，并新增 `ExtractorBroken` | T7 |
| 6 | 直接 `reqwest::get` 拉弹幕 | **手工解裸 deflate**（`flate2`，已在依赖树），不给 reqwest 加 feature | T10 / T4.2 |
| 7 | 工具探测沿用 `command_available_blocking` | **先修该函数**（写死 `-version`，yt-dlp 用 `--version`） | T2 |

## 1. 范围

**做**：粘贴 B站链接 → 预览卡 → 下载视频 + 弹幕 → 建项目 → 自动管线。
**不做**：番剧/课程（`bangumi`/`cheese`）、UP 主空间批量、收藏夹批量、字幕（CC）下载。
这些 URL 形式 yt-dlp 认，但本项目没有对应的信息架构（一轮一批项目会撑爆磁盘配额），
遇到时按 `UnsupportedUrl` 明确拒绝并给出文案，**不要静默失败**。

## 2. 隔离与模块化（先定接缝，再写代码）

**总原则：端口—适配器。跨进程、跨来源、跨 crate 的边界一律不"穿线"。**
§2.1 是三条硬边界、§2.2 是共享下沉原则，落地时按 §2.4 的 grep 判据自查，**不过关就是嵌合**。

### 2.1 依赖方向（谁可以依赖谁）

```
                    api/ingest.rs        api/projects.rs
                          │                    │
                          └─────────┬──────────┘
                                    ↓
                          download_worker.rs        ← 只做编排，不含引擎知识
                    ┌───────────────┼────────────────┐
                    ↓               ↓                ↓
            Downloader(端口)   bilibili/        ingest::capacity
             trait + DTO       B站专属知识       ingest::landing
                    ↑                            （纯逻辑，两来源共享）
        适配器：yt-dlp │ bbdown │ <绝对路径>
                    │
            CommandSpec（纯数据）
                    │
        ────────────┼──── 进程边界 ────────────────────────
                    ↓
        yt-dlp.exe 子进程（server 侧唯一 spawn 点）
```

三条硬边界：

1. **P4 的适配器不碰进程**（⚠️ 这是**路径级**规矩，不是整个 crate 的）：
   `vtrim-service/src/downloader/` 与 `bilibili/` 里的代码只产出 `CommandSpec`
   （程序 + 参数 + 要清的 env）与"解析一行 stdout"的结果，**不出现 `use std::process` / `use tokio::process`**。
   执行只在 `vtrim-server/download_worker.rs`。
   → 收益：命令构造与输出解析**完全离线可测**（§5 的快照测试靠这条）。
   > 别把这条误读成"service 不许开进程"：`disk.rs:40` 调 `df`、`instancelock.rs:72/85` 调 `tasklist`/`kill`
   > 都是既有的、合理的（进程自省类操作）。本规矩只要求**新增的下载器适配层**别把执行混进来。
2. **引擎不知道 B站，B站不知道引擎**：`downloader/` 只认"URL + 通用下载选项"；
   `bilibili/` 只认 bvid / cid / 弹幕。**二者只在一处装配**（`download_worker`）。
   加第二个引擎 = 加一个适配器，**server 侧一行不改**（逃生舱 `<绝对路径>` 同理）。
3. **两个 worker 不许互相 import**：`download_worker` **禁止** `use crate::ingest_worker`。
   要共享的东西**下沉**，不是互相调用（见 2.2）。

### 2.2 共享部分下沉，而不是两个 worker 互相调用

`check_capacity` / 项目落地 / `spawn_pipeline` 现在都在 `ingest_worker.rs` 里。
若 B站 worker 直接调它，立刻变成"改 B站功能可能碰坏录播姬"，而且录播姬那套
`is_finalized` / 待分配池的语义会顺着调用链渗进来。所以按"共性下沉、个性保留"拆：

| 要共享的 | 现状 | 去向 | 依据 |
|---|---|---|---|
| 容量预检（配额 / 总上限 / 磁盘水位） | `ingest_worker::check_capacity` | **下沉 `vtrim-service/src/ingest/capacity.rs`**（纯函数，离线可测） | 它是"还能不能再收一个任务"的通用判断，与录播姬无关 |
| 项目落地（`new_project_dir` → `write_project_config` → `create_project` → `refresh_bytes`） | 散在 `ingest_worker` | **下沉 `vtrim-service/src/ingest/landing.rs`** | 两个来源的产物同构（§8 的既定目标），落地四步本就该一致 |
| 起管线 | `ingest_worker` 私有 | **上提 `vtrim-server/src/pipeline_boot.rs`** | 需要 server 的 state/jobs，放 service 不合适；独立模块两边都能用 |
| **不共享**：`is_finalized` 空闲窗口、待分配池、`move_file` / `concat_fragments` | 录播姬专属 | **留在 `ingest_worker` 不动** | B站是"进程退出即完结"。抄过来既是无用复杂度，也会把两套语义缠死 |

`ingest_worker` 自己改为调用下沉后的 `ingest::capacity` / `ingest::landing` / `pipeline_boot` ——
**纯搬迁、行为不变**，用现有单测兜底。

### 2.3 新增/改动文件清单

严格遵守现有单向依赖（`vtrim-service` 绝不 `use vtrim_server`）：

```
vtrim-service/src/                     ← 纯逻辑，无 axum
  net/deflate.rs        （新） 裸 deflate 解压（flate2，zlib 兜底）
  bilibili/api.rs       （新） view API 客户端：BvInfo{ pages[], title, uploader, stat }
  bilibili/danmaku.rs   （新） cid → XML（浏览器 UA + 解压）→ 落 danmaku.xml
  downloader/mod.rs     （新） **Downloader trait（端口）** + CommandSpec / Resolved 等 DTO
  downloader/ytdlp.rs   （新） yt-dlp 适配器：argv 构造 + 哨兵解析 + 错误分类（**不 spawn**）
  downloader/registry.rs（新） 版本 + sha256 注册表 + ensure_ytdlp()
  ingest/capacity.rs    （新） 容量预检（**从 ingest_worker 下沉**，纯函数）
  ingest/landing.rs     （新） 项目落地四步（**从 ingest_worker 下沉**）
  assets.rs             （改） preload_policy("bilibili") → Preload::Danmaku

vtrim-core/src/danmaku/mod.rs          （改） parse_bililive_xml 的 y 取值 + 固件
vtrim-server/src/
  tool_probe.rs         （新） 外部命令探测 + 30s 缓存（**从 api/mod.rs 搬出**，worker 侧不再 use api）
  api/mod.rs            （改） 改用 crate::tool_probe；版本参数不再写死
  api/routes.rs         （改） 挂两个新端点
  api/ingest.rs         （改） resolve / bilibili 下载入口
  api/projects.rs       （改） 放行 "bilibili"
  pipeline_boot.rs      （新） 起管线（**从 ingest_worker 上提**，两个 worker 共用）
  download_worker.rs    （新） 下载槽 worker（**不 import ingest_worker**）
  ingest_worker.rs      （改） 改调下沉后的 capacity / landing / pipeline_boot（行为不变）
```

### 2.4 边界验收判据（做到就是没嵌合）

**判据已落成可执行脚本**：`python scripts/check_module_boundaries.py`（`-v` 列出通过项）。
违规非零退出，可以直接挂进 CI 或本地回归。

> ⚠️ **别退回手写 grep**。这里踩过一次：第一版判据写成"在这些目录里 grep `std::process`"，
> 一跑出 **8 条命中、全是假阳性** —— 7 条是解释规则本身的注释（"本目录不许出现 `yt-dlp`"），
> 1 条是 `std::process::id()`（只取 PID 给临时目录命名，没创建任何进程）。
> 一个天天报 8 条噪音的检查，结局就是被无视。
> 所以脚本里有两条**必须保留**的处理：**先剥注释再匹配**（`(?<!:)//` 以免切坏 `https://`）、
> 以及只认 `Command` / `.spawn(` 而不认笼统的 `std::process`。
>
> 反过来说明这个门禁**真的会红**才算数：往 `downloader/ytdlp.rs` 里塞一行
> `std::process::Command::new("x")` 验证过 → `exit 1` 并点名到行。
> 顺带它当场抓到一处真违规（`net/deflate.rs` 的测试固件里写了 `chat.bilibili.com`）——
> **修的是固件，不是规则**：解压测试本来就不该需要 B站形状的载荷。

| 检查项 | 期望 |
|---|---|
| `downloader/` / `bilibili/` 里开子进程（`Command::new` / `.spawn(`） | **0 命中**（执行只在 `download_worker.rs`；⚠️ 路径级——crate 别处有合法的 `df`/`tasklist`/`kill`，`::id()` 也不算） |
| `crates/vtrim-service` 里出现 `use vtrim_server` | 0（既有硬规矩，别破） |
| `downloader/` 里出现 `bvid` / `cid` / `bilibili.com` | 0 命中（引擎不认识 B站） |
| `bilibili/` 里出现 `yt-dlp` / `VTRIM_PROG` / `VTRIM_PATH` / `spawn` | 0 命中（B站模块不认识引擎） |
| `net/` 里出现 `bvid` / `cid` / `bilibili.com` | 0 命中（通用解压工具） |
| `download_worker.rs` 里出现 `ingest_worker` / `is_finalized` / `move_file(` | 0 命中 |
| `vtrim-core` 的**逻辑**里出现 `bvid` / `cid` / `view-interface` | 0（注释里说明来源可以 —— core 只认识"弹幕 XML"） |
| 适配器单测 | **全部离线**（录制的 stdout / XML fixture），不打外部网络 |
| 新增配置项 | 只允许 `[ingest] downloader` 与 `[ingest] ytdlp_path`；引擎私有常量（哨兵前缀、超时、浏览器 UA）**不进配置**，避免组合爆炸 |

### 2.5 yt-dlp 的调用形态：按任务短命 sidecar 子进程

**不是嵌入（链接进本进程），也不是常驻下载服务。**

| 形态 | 判定 | 理由 |
|---|---|---|
| 嵌入进程内 | ❌ | `yt-dlp` 是纯 Python，**没有可链接的 C ABI**；`yt-dlp.exe` 本身就是 PyInstaller 自解压包。要"嵌入"只能把 CPython 解释器当运行时依赖，同时丢掉它唯一的护城河——**上游持续更新的提取器**。Rust 侧的第三方重写（`rusty_ytdl` 之类）等于自己养提取器，B站改一次就得自己跟 |
| 常驻下载服务 | ❌ | `yt-dlp` 没有 server 模式。套 HTTP 壳 = 多一个 Python 运行时 + 端口 + 健康检查 + 生命周期管理；而项目现有的外部工具（ffmpeg / ffprobe）**全部是 per-task spawn**，没有常驻先例 |
| **按任务 spawn 一次，跑完即退** | ✅ | 与 `cutting/mod.rs:238` 的 ffmpeg 调用同形态。最大好处：**任务边界 = 进程生命周期**，不需要像录播姬那样为空闲判定额外写状态机（对应 S4 里"删掉 `is_finalized`"） |

**跨进程的边界只有一处**，模块分工按此切开：

- `vtrim-service/src/downloader/ytdlp.rs` —— **纯函数**：构造 argv、按哨兵前缀分流 stdout、错误分类。**不 spawn**。这样 §5 的"命令构造快照测试"可以完全离线跑。
- `vtrim-server/src/download_worker.rs` —— 真正 `tokio::process::Command` spawn + 逐行读 stdout 的地方，照 `ingest_worker.rs:639` 既有写法。
- `vtrim-service/src/bilibili/danmaku.rs` —— **进程内** `reqwest` + `flate2`，不开子进程。

⚠️ 子进程语义自带的三条硬约束（参数对应 §4.1）：

1. **代理必须清掉**：`Command` 上 `.env_remove("HTTP_PROXY"/"HTTPS_PROXY"/"ALL_PROXY")`（含小写）。
   实测本机代理会把 yt-dlp 的出网打挂；**不要靠"继承用户 shell"**。
2. **非 TTY 没有进度**：必须显式 `--progress`，否则一行都不输出。
   且 `--progress-template` 与 `--print` 的输出**都在 stdout**，靠哨兵前缀按行分流。
3. **超时与中断必须自己实现**：已验证 `ingest_worker.rs` 与 `api/mod.rs::spawn_job` **都没有子进程超时或 kill**
   （`spawn_job` 只兜 async 任务体的 panic）。一个挂死的 yt-dlp 会永久占住下载槽。
   `download_worker` 要自建「无进度超时 → kill → 标记 failed」，**这一条没有可抄的先例**。

## 3. 分阶段实现

### S0 先修既有缺陷（**独立可合，不依赖 P4**）

这**三条**是实测发现的现存缺陷，与 P4 是否做无关：

1. **版本探测参数化**
   `command_available_blocking(cmd, version_arg)`；ffmpeg 传 `-version`，yt-dlp 传 `--version`。
   顺带把探测整体搬到叶子模块 `tool_probe.rs`（worker 侧要用它，不该反向依赖 web 层 `api`）。
   判据：对不存在的命令返回 `None`；`yt-dlp --version` 返回 `2026.08.19`。
2. **`assets.rs:383` 预载策略**
   `"bilibili" => Preload::Danmaku`（现为 `Subtitle`）。同步改它的文档注释（现在写着「仍然是 P4 未实现」）。
3. **`danmaku/mod.rs` 解析器 y 取值**（见 S3，可并入 S3，但建议先做——它是纯函数改动）

⚠️ **原第 4 条「放行 `"bilibili"`」已移出 S0，挪到 S5。**
理由：`projects_create` 里 `"bilibili"` 一旦从 `Err` 分支放行，落到的是 `create_local_project`
（本机文件那一套：root/dir/input 路径囚笼），而 B站的下载落地要到 S1–S4 才存在。
放在 S0 会让用户能建出**"没有视频的 B站项目"**，等于主动制造坏状态。
S0 的判据是"独立可合、且每一条自身完备"，这条不满足 —— 它必须和下载通道一起上。

### S1 下载器适配层 + 工具发现

1. `downloader/registry.rs`：照 `vtrim-core/src/download.rs::MODEL_REGISTRY` 写
   ```
   ("yt-dlp", "yt-dlp.exe", "2026.08.19",
    "66674953fe251b89f4d08c5f0e35e0728679bd67ab3d7d05c0562af101dd3e7a", 17840399)
   ```
   `ensure_ytdlp()` 的查找顺序与 `asr::resolve_model_dir` 同构：配置路径 → `<程序目录>/.vtrim/bin/` → PATH。
   下载后**必须校验 sha256**，不匹配就删掉并报错（别用半下载的文件）。
   ⚠️ 别照抄 `ensure_one_model` 里的 `marker` 判定——exe 的 marker 就是"文件存在且 sha256 对"。
2. `downloader/mod.rs`：`enum DownloaderKind { YtDlp, Bbdown(PathBuf) }`，
   `downloader = "yt-dlp" | "bbdown" | "<绝对路径>"`（逃生舱语义见选型文档 §0）。
3. ⚠️ **拉起子进程时必须清掉 `HTTP_PROXY`/`HTTPS_PROXY`/`ALL_PROXY`**（含小写）。
   构建 `Command` 时 `.env_remove(...)`，不要靠"继承用户 shell"。

### S2 B站 API 客户端（`vtrim-service/src/bilibili/`）

1. **`api.rs` — view 端点**
   ```
   GET https://api.bilibili.com/x/web-interface/view?bvid={bv}
   Header: User-Agent: <浏览器 UA>      ← 必须有；不需要 Referer/Cookie/签名
   ```
   返回 `data.cid`（单P）、`data.pages[].cid/part/duration`（多P）、`data.title`、`data.owner.name`、
   `data.duration`、`data.stat.danmaku/view`、`data.pic`（封面）。
   ⚠️ `code != 0` 要映射成**具体错误**（`-403` 权限不足、`-404` 不存在），别当成解析失败。
2. **`net/deflate.rs` — 解压**
   ```rust
   // 先按裸 deflate 解（B站实际行为），失败再试 zlib 包装
   flate2::read::DeflateDecoder::new(bytes)   // wbits = -15
   ```
   `flate2` 已在 `Cargo.lock`（1.1.9，经 png/ureq/zip 引入），加为直接依赖不引入新传递依赖。
   **不要**改成给 reqwest 加 `deflate` feature —— 它按规范走 zlib 包装，而 B站发的是裸 deflate。
3. **`danmaku.rs` — 弹幕落盘**
   ```
   GET https://comment.bilibili.com/{cid}.xml
   Header: User-Agent: <浏览器 UA>, Referer: https://www.bilibili.com/
   ```
   - UA 非浏览器 → **412**（必须给浏览器 UA；不要用 `reqwest/x.y` 或 `python-urllib`）
   - 响应**恒为 `Content-Encoding: deflate` 且忽略 `identity`** → 必解压
   - 落 `<项目根>/danmaku.xml`，然后 `write_project_config(..., Some(Path::new("danmaku.xml")), ...)`
   - **解压后必须校验** `content.contains("<d p=")`，否则报错而不是静默传空给解析器

### S3 解析器修复（`vtrim-core/src/danmaku/mod.rs`）

1. **y 取值**：`parts[7]` 在主站语义下是 dmid，不是行号。改法二选一（推荐 a）：
   - **(a) 不再从 `[7]` 取 y**，一律走 `default_y_for_mode(mode)`。
     理由：录播姬那边 `[7]` 恒为 0 → `.filter(>0.0)` 已经把它挡掉了，
     所以**删掉这条分支对录播姬路径是行为等价的**，只修掉主站的错误值。
   - (b) 加合理性区间：`0.0 < v <= 800.0` 才采用（dmid 是 1e16 量级，必然被挡）。
2. **文档注释**：`/// 结构：<i><d p="时间,模式,字号,颜色,时间戳,弹幕池,用户,行号">` 要改成
   「`[7]` 是数据库 ID（均分/主站）或固定 0（录播姬）；**不作为 y 使用**」，并补一句 9 段兼容。
3. **测试固件换成两套真实样本**（现有固件既不是主站格式也不是录播姬格式）：
   - 主站：9 段、`[7]` 为 17–19 位 dmid、`[8]` ∈ 4..10（从 `.workbuddy/p4-verify/dm_122300483.xml` 截取）
   - 录播姬：8 段、`[7]` 恒为 `0`（按 rec.danmuji.org 的官方字段说明构造）
   - 断言：两者都能解析出正确的 `start`/`mode`/`color`，且 **`y` 不塌成同一个值**
4. ⚠️ 顺手确认 `assign_danmaku_tracks`（`vtrim-polish/src/parse/text.rs:64`）与
   `process/placement.rs` 的调用顺序（实测报告「仍未验证 #2」）。它只写不读，预计能掩盖 y 的影响，
   但 `placement.rs` 里 `y = (d.y/1920)*540` 的弹幕带计算依赖 y —— **确认后再决定要不要额外兜底**。

### S4 会话账本与下载 worker

1. **加 `kind` 列**（复用 `ingest_sessions`，不新建表）：
   ```sql
   ALTER TABLE ingest_sessions ADD COLUMN kind TEXT NOT NULL DEFAULT 'recorder';
   ```
   `session_id` 约定 `bili-{bvid}-p{n}`；`room_id` **保持 NULL**（塞 bvid 会误命中 `channel_map` 的按房间号查规则）。
   需要迁移逻辑（`db.rs` 现有建表是 `CREATE TABLE IF NOT EXISTS`，加列要单独判 `PRAGMA table_info`）。
2. **`download_worker.rs`**：结构照 `ingest_worker.rs`，但**去掉用不上的复杂度**：
   - ❌ 不要 `is_finalized` 的空闲窗口判定 → **进程退出即完结**
   - ❌ 不要 `unassigned` 待分配池 → 直接归当前登录用户
   - ❌ 不要 `move_file` / `concat_fragments` → 下载器直接写进托管目录，混流由 yt-dlp 自己做
   - ✅ 保留 `check_capacity`（配额/总上限/磁盘水位）→ 建议把它从 `ingest_worker.rs` 抽到共享位置
   - ✅ 保留 `spawn_pipeline`（自动管线）→ 同样建议抽出复用
   - 状态机：`pending → downloading → danmaku → done|failed`
3. 落位：`projects::new_project_dir(uid, &format!("bili-{bvid}"))` → `write_project_config` →
   `db.create_project(..., "bilibili", ...)` → `projects::refresh_bytes`。

### S5 API 端点

| 端点 | 说明 |
|---|---|
| `POST /api/ingest/resolve` | 薄壳、**不落库**。入参 `{"url": "..."}` → 出参 `{kind, title, uploader, duration, cover, pages:[{index, cid, part, duration, danmaku_count}], formats:[{height, vcodec, filesize_approx}], estimated_bytes}`。分P 在此展开供用户勾选 |
| `POST /api/projects {source:{kind:"bilibili", bv, pages:[...]}}` | 放行后逐 P 建会话并入下载槽 |

⚠️ `resolve` 是**用户可反复点**的端点（换链接、改清晰度）→ 必须走 `probe_cache` 那类短 TTL 缓存，
别每次打两个外部接口。

### S6 WebUI（新建向导「B站链接」分支）

1. 粘贴框 → 点「解析」→ 预览卡（封面/标题/UP主/时长/总占用）
2. 多P：列表 + 全选/单选（**默认不勾全选**，见选型文档 §6.5 与 P5 容量冲突）
3. 清晰度下拉（默认 `1080p`）；非 admin **隐藏** `cookie_file` 项
4. 提交后可看进度（SSE 复用 `/api/stream`）
5. 工具未就绪 → 引导卡（一键下载 yt-dlp，显示 sha256 与版本）

### S7 文档同步

`AGENTS.md:523`、`AGENTS.md:663-665`（bilibili 改「弹幕优先」）、`ROADMAP.md:9`、
设计稿 §4.5 默认值、`THIRD-PARTY-NOTICES.md`（加 yt-dlp，Unlicense + 说明 `xtask scan` 覆盖不到）。

## 4. 契约清单（全部来自实测）

### 4.1 下载命令（最终形态）

```bash
yt-dlp \
  --cookies <netscape.txt>              # 可选；Netscape 格式，JSON 会静默失效
  -f "bv*[height<=1080]+ba/b[height<=1080]/bv*+ba/b"   # 必须带兜底，否则可能直接 exit 1
  -S "vcodec:h264"                      # 避开默认选到的 HEVC（WebUI 预览播不了）
  --merge-output-format mp4 \
  --limit-rate <k>K                     # 可选
  --newline --progress \
  --progress-template "VTRIM_PROG:%(progress._percent_str)s|%(progress._speed_str)s|%(progress._eta_str)s" \
  --print "VTRIM_PATH:after_move:filepath" \
  --print "VTRIM_META:%(resolution)s|%(vcodec)s|%(acodec)s" \
  --no-warnings --no-playlist \
  -o "<项目目录>/source.%(ext)s" \
  --playlist-items <N>                  # 多P 时指定
  "<url>"
```

⚠️ `--progress-template` 与 `--print` **都写 stdout**（实测），所以两者都要哨兵前缀，
按行前缀分流；**不要**依赖"最后一行是路径"。

⚠️ 不加 `--progress` 时非 TTY 下**一行进度都没有**。

### 4.2 错误分类（按实测关键字重写）

| 分类 | 实测关键字 | 用户文案方向 |
|---|---|---|
| `UnsupportedUrl` | `Unsupported URL` | 这个链接不支持 |
| `NotFound` | `HTTP Error 404` | 视频不存在或已删除 |
| `NeedLogin` | `only available for registered users` | 需要登录（配 cookie_file） |
| `MemberOnly` | `Premium` / `大会员` / VIP | 需要大会员 |
| `ExtractorBroken` | **`Unable to extract initial state`** | yt-dlp 暂时不支持该视频（legacy），**重试无用** |
| `FormatUnavailable` | **`Requested format is not available`** | 清晰度不可用（内部错误，说明兜底串没写对） |
| `RateLimited` | `HTTP Error 412` / `429` | 被风控，稍后重试 |
| `ToolMissing` | spawn `NotFound` / `Access is denied` | 下载器未就绪 |
| `DiskFull` | `No space left on device` / `os error 112` | 磁盘空间不足 |

错误要落库（`ingest_sessions.state=failed` + `error`），文案直接给用户看。

### 4.3 sha256 钉死（已实测）

```
yt-dlp 2026.08.19  yt-dlp.exe  17,840,399B
66674953fe251b89f4d08c5f0e35e0728679bd67ab3d7d05c0562af101dd3e7a
官方 SHA2-256SUMS 与实算一致 ✅
```

## 5. 测试与回归钉子

| 层 | 钉子 |
|---|---|
| `net::deflate` | 真实抓的 `dm_122300483.xml`（145,077B 裸 deflate）能解出 361,725B；zlib 包装也能解；垃圾字节报错不 panic |
| `bilibili::api` | `code != 0` 映射；`pages[]` 多P 解析；**离线 fixture**（不要联网测试） |
| `danmaku::parse_bililive_xml` | **两套真实固件**（主站 9 段 + 录播姬 8 段）；断言 `y` 不全相等；断言 start/mode/color 正确 |
| `downloader::ytdlp` | 命令构造快照（防参数回退）；stdout 哨兵分流（进度行与路径行混排时的解析） |
| `downloader::registry` | sha256 不匹配时删文件并报错；`--version` 探测（防 `-version` 回归） |
| `assets::preload_policy` | `"bilibili" => Danmaku`（防回退成 Subtitle） |
| `ingest_worker::check_capacity` | 已存在，P4 复用即可 |
| 端到端 | 用 `.workbuddy/p4-verify/` 里的真实样本跑一遍：resolve → 下载 480p → 弹幕 → 建项目（**测试里不要真的打 B站**，用录制好的 stdout/XML fixture） |

## 6. 风险与不确定项

| 风险 | 状态 | 对策 |
|---|---|---|
| **池量级弹幕 ≠ 全量**，两路径 popularity 口径不同 | 实测确认差异存在，**影响程度未验证** | 在 resolve 预览卡同时显示 `stat.danmaku` 与 XML 实际条数；超量时提示"收录为弹幕池量级"。必要时对超量视频引导用户用录播姬 |
| `Unable to extract initial state`（legacy 视频） | 已确认为上游已知问题（issue #14793），**对录播场景影响小** | 独立错误类，文案明确"重试无用"；保留 BBDown `<绝对路径>` 逃生舱 |
| 长录播（GB 级）下载稳定性 / 断流 | **未验证**（只测了 19MB） | S1 之后先补一次长录播实测再放量 |
| `assign_danmaku_tracks` 与 `placement` 顺序 | **未验证** | S3 第 4 条里确认后再决定是否额外兜底 |
| 分P 全展开撞 P5 容量 | 设计冲突已知 | WebUI 默认不勾全选，让用户选 P |
| B站改接口（UA/deflate/字段） | 长期风险 | 解压做双风味兜底；解析失败必须报错不静默；错误类可观测 |
| **子进程挂死占住下载槽** | **现有代码无先例**：`ingest_worker` 与 `spawn_job` 都没有子进程超时或 kill | `download_worker` 自建「无进度超时 → kill → failed」；顺带评估 ingest 侧是否同缺 |

## 7. 验收清单

图例：✅ = 已按判据跑过；🧪 = 单测覆盖（离线）；⏳ = 需要端到端（真机 + 真实网络）才能确认，
**没跑就不勾**。

- [x] ✅ S0：版本探测参数化（`tool_probe` 单测用 python 复刻 `-version` 的坑；yt-dlp 本体在 T0 实测过 `--version` 可用）
- [x] 🧪 S2：解析真实 BV → 预览卡字段（`extract_bvid` 单测覆盖 8 种真实 URL 形态；view API 的字段映射在 T3 实测过）
- [x] ✅ S2：多P（23P）解析出全部 23 个 cid，且与 `yt-dlp --list-subs` 抠出的 cid 一致（**T3 实测**）
- [x] 🧪 S3：主站 9 段 XML 解析后 `y` **不全相等**；录播姬 8 段行为不变（两套真实方言固件）
- [x] ✅ S4：产物路径只认 `VTRIM_PATH:`（哨兵位置在 `after_move:` **之后**；T8 实测 `--print` 回报绝对路径且 fs 命中）
- [x] ✅ S4：产物是 **H.264**（T8 实测 `-S vcodec:h264` 生效，拿到 avc1 1920x1080；不带时默认选到 HEVC）
- [x] 🧪 S4：`danmaku.xml` 落盘 + 能被现有解析器读出（`save_danmaku` / `parse_session_ref` 单测；真实 XML 在 T5/T6 验过）
- [x] 🧪 S4：失败时 `state=failed` 且 `error` 是人话（`DownloadErrorKind → user_message` 单测，关键字取自 T7 实测的 stderr）
- [x] ⏳ S4：子进程挂死能被超时 kill 并落 `failed` —— **逻辑已实现**（`run_download` 的 idle 超时 + `kill_on_drop`），
      但"真挂死一次"没造出来测过；这是唯一没有实测支撑的兜底路径
- [x] ⏳ S4：`/api/health` 报 downloader 就绪位（`downloader_ready` 已加；**不报路径**，只回版本/来源/是否过哈希）
- [x] ✅ S7：`AGENTS.md` / `ROADMAP.md` / 设计稿 / `THIRD-PARTY-NOTICES.md` 全部同步
- [x] ✅ §2.4 边界门禁：`python scripts/check_module_boundaries.py` → **6 条规则 / 13 个文件 / 0 违规**；
      且**反向验证过它会红**（注入一行 `std::process::Command::new("x")` → `exit 1` 并点名到行）
- [x] ✅ 全仓 `cargo check --workspace --all-targets` 零 warning；单测全绿
      （core 131 / polish 137 / render-core 22 / server 128 / service 113，**531 passed / 0 failed**）
      + `webui/dist` 重建通过

## 8. 落地进度

### S0–S7 ✅ 全部完成（2026-09-19，**未提交**）

| 阶段 | 落点 |
|---|---|
| S0 | 三条既有缺陷：`tool_probe.rs`（版本参数化 + 从 `api` 搬到叶子模块）、`assets.rs` 预载策略、`danmaku/mod.rs` 解析器 y + 换真实固件 |
| S1 | `vtrim-service/src/downloader/{mod,ytdlp,registry}.rs` —— `Downloader` 端口 + yt-dlp 适配器 + sha256 注册表 |
| S2 | `vtrim-service/src/{net/deflate.rs, bilibili/{mod,api,danmaku}.rs}` —— view API、弹幕旁路、双风味解压 |
| S3 | 调用点修复：`vtrim-polish/src/commands.rs` 原来用**不认 XML** 的 `parse_srt_file`（4 个真实 XML 样本全产出 0 条），改走 `load_danmaku_auto`；`parse_srt_file` 加"拿到 XML 就吵"的护栏 |
| S4 | `ingest_sessions` v8 加 `kind`/`source_ref`；下沉 `ingest/{capacity,landing}.rs`；上提 `pipeline_boot.rs`；新增 `download_worker.rs` |
| S5 | `POST /api/ingest/resolve` + `POST /api/ingest/bilibili` + `POST /api/admin/downloader/{install,status}`；routes 挂载；`ingest_ready` 拆出 `downloader_ready` |
| S6 | `WorkbenchView.vue` 的 B站分支（向导）+ `DownloaderConfig.vue`（设置页卡）+ `api/ingest.js` + `utils.js::fmtClock` |
| S7 | `AGENTS.md`（新增「B站链接通道」一节 + 核心文件索引 + 开发命令校正）、`ROADMAP.md`、设计稿 §4.5 加"已被实现取代"横幅、`THIRD-PARTY-NOTICES.md` 加 yt-dlp 行 |
| 额外 | `scripts/check_module_boundaries.py`：把 §2.4 的隔离判据从"文档里的一段 grep"变成**可执行门禁** |

**过程中发现并修掉的额外缺陷（原计划没有）**

1. **精修侧静默丢弹幕**：`vtrim-polish/src/commands.rs` 的 `cmd_init → cmd_parse` 用
   `vtrim_core::danmaku::parse_srt_file` 读弹幕，而它**只认 SRT 时间轴**——
   录播姬与 B站写的都是 **XML**。用 4 个真实 XML 样本实测：**全部产出 0 条**。
   也就是说**录播姬通路上的精修弹幕此前一直是空的，且不报错**（`Err` 分支只 `tracing::info`）。
   改走 `load_danmaku_auto`（按扩展名分派），并给 `parse_srt_file` 加护栏：
   内容里出现 `<d p=` 就返回 `Err`，让这类误用**用错就吵**而不是静默。
2. **`.workbuddy/p4-verify/` 里的测试固件是手编的**：`danmaku/mod.rs` 那个标着"官方样例结构"
   的 8 段固件是我自己编的（既非主站 9 段、也非录播姬 8 段），所以那个测试**从来没跑过真实数据**。
   已换成两套真实方言。
3. **`/api/health` 一次响应里把 17.8MB 的 exe 哈希两遍**：`find_ytdlp` 内部会走
   `verify_sha256`（全量读文件），而 health 是前端**反复轮询**的接口，且原来在 `json!` 里
   连着调了两次（`ready` 与 `verified` 各一次）；`download_worker::tick` 每 10s 也要调一次。
   而 health 上方三行的注释刚写着"去掉两次同步 SQLite 查询后它**不再碰磁盘**" —— 直接抵消。
   → 两处修：`managed_copy_is_trusted` 加 **`(路径, 长度, mtime)` 键控缓存**（文件被替换时键必变，
   所以重装/换版本不会读到旧结论；用 `OnceLock<bool>` 那种一次性缓存是错的），
   health 里**只查一次**并复用。钉子 `trust_cache_key_follows_content_changes`
   专门钉"同长度但内容已改也必须换键"（即"别把 mtime 从键里去掉"）。
   这类"高频事件触发昂贵初始化"的排查套路见 skill `expensive-rebuild-storm-triage`。

**验证（实测，非推断）**

- `cargo check --workspace --all-targets` → **0 error / 0 warning**
- `cargo test --workspace --lib` → **531 passed / 0 failed**
  （core 131 / polish 137 / render-core 22 / server 128 / service 113）
- `python scripts/check_module_boundaries.py` → **6 条规则 / 13 个文件 / 0 违规**
- `webui`：`node webui/node_modules/vite/bin/vite.js build` → `✓ built in 5.04s`
  （⚠️ 这个 shell 起不了 `npm run`（找不到 bash，exit 127），必须直接调 vite）

**自纠错**

1. **「service 不许出现 `std::process`」是错的** —— `disk.rs` 调 `df`、`instancelock.rs` 调
   `tasklist`/`kill` 都是既有的合法用法。已收紧成**路径级**规矩，见 §2.1 / §2.4。
2. **S0 原第 4 条「放行 `"bilibili"`」已挪去 S5** —— 放行后落到的是 `create_local_project`，
   会建出"没有视频的 B站项目"。
3. **`check_quality` 的字段名**：它是提交端点（字段 `quality`）与设置页表单（`download.max_quality`）
   共用的校验，却把「未知清晰度」写成终态文案 → 表单侧报错不带字段名，用户不知道哪个框错了。
   改成返回**原因**、字段前缀由调用方补。这条是被我自己的回归钉子
   （`patch_download_keys_are_flat_not_nested`）抓出来的。
4. **计划里 `--print "VTRIM_PATH:after_move:filepath"` 是错的** → 会打出 `NA`。
   哨兵必须在 `WHEN:` **之后**：`after_move:VTRIM_PATH:%(filepath)s`（T8 实测）。

### 还没做的（放量前必须补）

- [ ] **长录播（GB 级）下载稳定性**：只测过 19MB 的短片。这是唯一的"大"不确定项。
- [ ] **真挂死一次**，验证 idle 超时 + kill 真的能把槽位释放（现在只有代码路径，没有实测）。
- [ ] **弹幕池量级对 popularity 的实际影响**没量化（上限 ≈ `maxlimit×1.2`，不是全量）。
- [ ] `assign_danmaku_tracks`（只写 y）与 `placement.rs`（读 y 算弹幕带）的调用顺序已确认
      **轨道分配在后、覆盖在前**，所以 S3 的 y 修复不影响 placement —— 但这是静态阅读的结论，
      **没有跑过真实精修出片**。
