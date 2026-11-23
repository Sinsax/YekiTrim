# yekitrim-server/src/api — HTTP 领域模块（路由权威 + 共享底座）

> 范围：`crates/yekitrim-server/src/api`/** ｜ 上游：`crate::api::router`（`lib.rs` 与路由单测都走它）｜ 验证：`cargo test -p yekitrim-server`

> 历史与实测数据见 **`docs/server-api.md`**（① 路由与端点清单 / ② 素材库与字体通道 / ③ 文件出口·免登录·其它端点约定 / ④ 同步 IO 与任务体 / ⑤ 路径面三防线）。

## 职责

`/api/*` 的全部 handler、路由表与中间件装配都在这里，按领域一个文件一个模块。本目录只做 HTTP 层：
解析请求 → 调 `yekitrim-service` 的领域逻辑 / 起任务 → 回 JSON。同步 IO、任务体、路径归属这三件最容易出事的事
都有统一的落点（`blocking` / `spawn_job` / `scope.rs`），**别在 handler 里自己开一条路**。

## 入口

- `api::router(state: Arc<AppState>) -> Router` —— **唯一入口**（`mod.rs` 里 `pub use routes::router`；路由表本身在
  `routes.rs`，`lib.rs` 与路由单测都走它）。
- 共享底座（`mod.rs`，只放被两个以上子模块用到的东西）：`AppError` / `ApiResult` / `err(status, msg)` /
  `ok(v)` / `blocking(f)` / `block_on_here(fut)` / `spawn_job(jobs, job_id, body)` / `health`（`GET /api/health`）。
- 领域模块（一个领域一个文件；逐个端点清单见 `docs/server-api.md` §①）：

| 模块 | 职责 |
|---|---|
| `routes` | **唯一的路由权威**：`scripts/route_manifest.py` 抽的就是它，拆分前后必须逐条相同 |
| `auth` / `invite` / `logs` / `rooms` | 会话·注册·改密·doctor·账号列表 / 邀请码（admin）/ 日志与用量（admin）/ 直播间登记 |
| `config` | 程序配置、ASR 模型、LLM 测试；**路径面防线② 在这** |
| `projects` / `project_data` | 项目托管 / 项目数据面（`clips`·`events`·`assets`·`fs/list`·`media`） |
| `pipeline` / `core` / `render` | 管线任务体与 SSE / 执行入口与进度桥 / 出片子进程与进度解析 |
| `polish` | 精修编辑器后端（`/polish/*`）；`renderer` 白名单**只有 `dhampir`**（认不出 → **400**） |
| `test_support` | `TestApp`（整张路由表的接口级夹具），**只在测试编译** |

## 边界

- ⚠️ **素材库的读写都是用户层（2026-10-09 起），且真按 `Owner` 分库**：三个库（视频/贴纸/字体）归一组 `let assets = Router::new()` merge 进 `protected`（⇒ **未登录仍 401**）。
  ⚠️ **这一组刻意不挂 `enforce_scope`**：库**不属于任何工程**（位置由 `yekitrim_core::owner` 算）—— 挂进去 `fonts/import` 会被 scope 判成"不在你项目里"回 **404**。
  ⚠️ **同一路径别在两个 Router 里各挂一次**：axum merge 时 panic（`Overlapping method route`），**不是**后者覆盖前者。
  读 `GET /api/polish/fonts`（`fonts::scan_all`），写 `POST /polish/fonts/{import,upload,default,builtin}` 与 `DELETE /polish/fonts?id=`（`builtin=true` → **403**）。贴纸端点从 `/admin/stickers/*` 挪来，老路径已删。
- **素材库取文件（`GET /polish/asset?kind=video|font&file=`）与 `GET /polish/video-thumb?file=`**：
  ⚠️ 前者的**安全性质是"只认单个文件名"**（`is_safe_component` 挡分隔符与 `..`）—— 一旦 `file` 认 `/`，普通账号就能读走 `.yekitrim/llm.toml` 里的 API key；单测钉在**穿越尝试**上。
  ⚠️ 贴纸**不收编**进来（两级：包 / 包内文件；收编就得让 `file` 认 `/`）。
  ⚠️ **抽不出来回 404 不是 500**（没装 ffmpeg / 文件损坏都只是"这张没缩略图"）。
  ⚠️ **写路径（导入/上传/装自带）必须走 `fonts::ensure_dir_for(owner)`**，`owner` 不能省 —— 省了字节会落进别人的库（"传了却看不见"）。`set_default`/`remove` 同样按 `asset_dir(owner, Fonts)`。
  ⚠️ 这三条**有意不挂 `enforce_scope`**；导入用"服务器上的路径"而非上传（同 `/api/fs/list`）。
- **`POST /api/polish/cover`** 的 body 有可选 **`renderer`**（现在**只有 `takumi`**）：服务端**只透传**成子进程的 `--renderer`，取值口径与白名单只在 `crates/yekitrim-polish/src/cover.rs`（**只此一份**，认不出由那里**报错**）。能不能用回在 `/api/health.cover_static`（`{compiled, fonts, ready}`）。
  ⚠️ **health 公开、一律不回路径**（review B6，2026-11-14）—— 空字体的指路文案由前端给。

- ⚠️ **文件出口只有一个入口：`media::serve_file_from_headers`** —— `/api/media/*`、`/polish/{font,sticker-file,asset,video-thumb}`、`/polish/stickers/:pack/files/:name`、`/polish/dhampir-wasm/*` **一律**走它：`ETag` + `no-cache` → 304、文本类 gzip、Range；**有 `Range` 头不压不 304**。
  ⚠️ **`/api/*` 的 JSON 也 gzip**（`api/compress.rs`；**SSE 不在白名单**，超限原样放行）。
- **`routes` 是唯一的路由权威**；加接口 = 在 `routes.rs` 挂一条 + 在对应领域模块写 handler。
  ⚠️ **新增路由不用手动挂 protected —— 有一层认证兜底**（整改 T10，2026-11-24）：
  `require_authenticated_or_whitelist` 挂在 public/admin/protected **merge 之后、资源级 layer 之外**，
  规则四步 = 已有 CurrentUser ⇒ 放行 / `MatchedPath` 缺失（真没这条路由）⇒ 放行给 JSON 404 /
  命中 `PUBLIC_ROUTES` ⇒ 放行 / 其余一律走 `crate::auth::authenticate`（过了**把 CurrentUser 注进请求**再放行）。
  所以「漏配 = 匿名公开」这个洞没有了 —— 判据 `route_without_auth_layer_is_still_401` 用一条
  **故意不挂认证层**的探针路由钉住它（摘掉兜底层立刻红：`left: 200 / right: 401`）。
  ⚠️ **白名单全仓只有一份** `PUBLIC_ROUTES`（每条带理由）：只有「该端点另有独立凭证（URL token / env 令牌 /
  邀请码+开关）或前端在无会话时必须调到它」才准进；其余一律不进。
  ⚠️ 挂载位置两条硬约束：在 `access_log` **之内**（401 也要留日志、且它要读到明文 body）、
  在 `compress_json` **之外**（压缩只在 protected 里；挂反了日志读到的是 gzip 字节，判据是「受保护路由未登录的 401
  不许带 content-encoding」）。
  ⚠️ 它**不替代** `require_user` / `require_admin`（那两层仍管 CSRF 与角色），代价是非白名单请求会认证两次
  （结论相同、CSRF 不会少）；别为省这一次查询给 `require_user` 加「已有 CurrentUser 就跳过」——
  那等于让兜底层接管它的职责。
  ⚠️ **本地免登录（`--local-no-auth`）/ `POST /api/shutdown` 控制通道**：正文（两道闸、三道闸、互斥与开机守卫、哨兵会话、令牌头名与托盘各写一份字面量）见 `docs/deploy.md`；四条判据见 `docs/server-api.md` §④。
  ⚠️ `GET /api/project-data/config` 同时回 `config`（**合并后**生效值）与 `project_config`（**项目文件自己**那份）：预处理页靠后者区分"本工程写了"与"跟随全局"（`merge` 对空串不覆盖 ⇒ 清空即恢复跟随全局）。
  ⚠️ 两层的顺序**不能反**：`require_user` 必须在外层（先跑并写入 `CurrentUser`），改密强制门在内层才读得到用户。axum 里"后加的 layer 在外层"⇒ `routes.rs` 那两行**看起来是倒着写的**，别"顺手改正"。
- ⚠️ 子模块里的 `super::` 是 `api`、**不是 crate 根**（`api/auth.rs` 与 `crate::auth` 同名不同物，新增代码写全 `crate::auth`）；对外项一律 `pub(super)`，被两个以上模块用的 helper 放 `api/mod.rs`。
- 未注册的 `/api/*` 返回 **404 + JSON**，不会回退成 SPA HTML（否则前端把 200 HTML 解析成 `null` 静默失败）。
- `/api/fs/list` **仅 admin**，可浏览服务器任意目录（`{dir}` 绝对路径，也兼容 `{root, path}`）；**永不接受上传**。**进入下一级用 `{dir, enter:"子目录名"}`，路径由服务端拼**；所有出 JSON 的路径都过 `yekitrim_core::path::display()`。
  ⚠️ 探测要跟目录列举一起放 `spawn_blocking`（每个候选一次 `is_dir()`，是阻塞 IO）。
  ⚠️ 前端**别把 `roots` 当树根**：服务端会把**当前目录**追加进 `roots`（给下拉框显示"当前位置"），拿它当根会随用户走到哪而变。
- `GET /api/admin/users` 只回展示字段，**不含口令哈希**。
- **邀请码与注册**（`POST /auth/register`）：三重条件、失败**一律 403**（不区分哪条不过，防枚举）、按 IP 独立限流（`register|<ip>`）—— 正文在 `crates/yekitrim-service/AGENTS.md`，本卡不重复搬运。
- **素材清单端点 `GET/POST /api/project-data/assets`**：`POST` 三种形态（`{kind, path}` / `{kind, dir, name}` / `remove`），写前先解析、失败一律不落盘。
  ⚠️ `slots`（`{hybrid, single}`）**由服务端算**（`ASR_MODES` × `slot_bound`，前端**不自己从 `items` 推**）。
  ⚠️ **清单写入必须单次 `patch_config_file`**（生效值 + 整份清单同一个 patch）；清单形态 `[project.materials]` 的正文在 `webui/src/views/AGENTS.md`。
- ⚠️ **按账号的配置只用 `load_*_for(owner)`**（无 owner 版读顶层旧位置）；`/polish/cover-title` 踩过此坑（"API 服务和设置里的不一致"）。
- `/api/polish/*` 的 `dir` 必须是非空且存在的目录；`polish.toml` / `events.json` / `yekitrim.toml` / 全局配置的读-改-写走 `yekitrim_core::filelock::with_path_lock` 串行化。
- ⚠️ **渲染 / 封面任务的收尾日志只报状态、不带路径**：成功行就是 `成片已写出` / `封面已写出`（verbatim 长路径会撑满任务框那一行）；`subprocess_note` 仍然要跑。
- ⚠️ **剪切收尾要补精修工程 + 预热响度曲线**（剪切这个行为本身包含精修 init）：`run_single_cut_event` 跑完调 `yekitrim_service::init_polish_for_events(...)`；**别在 handler 里另开一条 init（或预热）路径**。
- ⚠️ **`/api/health` 是未认证接口**：不含账号数量、不含"是否仍用初始密码"、不含任何路径；`/api/admin/usage` 与 `/api/admin/logs` **仅 admin**（记录含其他用户的工程路径）。
  ⚠️ **`llm` 这一位按调用者的账号算**：走 `crate::auth::optional_owner` + `load_llm_config_for(owner)`，无会话才回落 `Owner::Local`（旧写法读 `load_global_config()` ⇒ **恒 false**）。判据 `auth::tests::optional_owner_reads_the_session_account`。

### 同步 IO 纪律（剪切时"整个服务器连不上"的根因）

- ⚠️ **同步 IO 绝不能留在 async 线程上**：项目目录常在网络盘上，一次 `exists()` 就可能几百毫秒，十几个并发就把 worker 全占住。
  - **handler 里的同步文件访问**一律走 `api::blocking(f)`；需要「async 判定 + 同步 IO」混用（如删除片段先问 `busy_with`）用 `blocking(move || block_on_here(xxx_impl(...)))`。**新增读项目目录的接口照着做**（已覆盖清单见 `docs/server-api.md` §④）。
  - **管线任务体**一律用 `api::spawn_job(jobs, job_id, body)`（专用线程 + current-thread runtime）；`cut_all` 内部逐片段剪切走 `spawn_blocking`。
  - `spawn_job` 带 **panic 兜底**（`catch_unwind` + 替任务调 `fail()`）：中间 panic 会让 Job 永远 `running`、资源键永不释放。
    ⚠️ **两条兜底缺一不可**：panic 与**线程都起不来**（`spawn.await` 前就 `Err`）时也**必须**替任务 `fail`。注意 `[profile.release]` **不再设 `panic = "abort"`**，panic 那条兜底在 release 里真的会跑；新增任务入口必须把 `(jobs, job_id)` 交给 `spawn_job`，**别把 abort 加回去**。
  - 其余同类修复（`serve_file` 元数据、`/api/media` 的归属校验 + `jail_existing`、`config_*_put` 的 `lock_path().await` + `blocking(...)`、截帧 ffmpeg、入库扫描）清单见 §④。

### 路径面三防线（多租户的头号风险，改动前先读）

**服务端能读写的文件范围 = 请求里出现的每一个路径字段，而不是"项目目录"这个概念。** 三道防线，缺一不可：

| 防线 | 位置 | 规矩 |
|---|---|---|
| ① polish body 字段收口 | `yekitrim-server/src/scope.rs::enforce_scope` | body 里每个"会被 handler 拿去开文件"的字段都要收口：**相对项目根**的进 `EXTRA_PATH_FIELDS`（`source` / `cover.image`），`dir` / `output_file` 单独处理，贴纸的 `asset` / `assets` 走 `is_safe_project_relative` + `bad_sticker_path_in`。**新增此类字段必须同步收口**，漏一个就是一条任意文件读取。 |
| ② 项目配置路径键 | `api/config.rs::PROJECT_PATH_KEYS` + `check_project_paths` | `PUT /api/project-data/config` 的 `raw` 与 `patch` 两条分支都要过这张表；非 admin 必须 `jail_join` 进自己项目；同一函数末尾 `check_material_paths` 校验 `[project.materials]` 清单。admin 豁免。 |
| ③ 消费侧纵深防御 | `yekitrim-polish/src/cover.rs` | 渲染封面时对 `cover.image` 再做一次 `tenant::ensure_within`。别删这道。 |

⚠️ 相关的两个既有约定也别松：`every /api/media` 走 `jail_existing`；`is_safe_component` 挡路径拼接（删除片段/贴纸文件名）。

⚠️ **`Path::is_absolute()` 在 Windows 上挡不住根相对路径**（`/etc/passwd` 返回 **false**，而 `join` 会替换掉基准 ⇒ `C:\proj` + `/etc/passwd` = `C:/etc/passwd`）：判"工程内相对路径"必须**逐字符**来（空 / 绝对 / 以 `/` 或 `\` 开头 / 含 `:` / 含 `..` / 含 `\` 全挡）。**唯一实现**是 `yekitrim_core::tenant::is_safe_relative_with`（严格度是显式参数：请求面 STRICT、`jail_*` LENIENT），`scope.rs::is_safe_project_relative` 只是**请求面那个调用点的薄委托**（整改 T19：以前是 scope 一份、tenant 一份，差异只在严格度上，那种差异要写成参数）—— **不许各写一遍**；**都要有一条单测**钉住 `/etc/passwd` 与 `\windows\x`。

> 改动只允许落在本目录；跨模块改动先读对方卡，并跑两条门禁。

## 验证

```bash
cargo test -p yekitrim-server
```

接口级单测（`test_support::TestApp` 建的是整张路由表）：JobManager / Range / 路由 404 / 配置 patch。
加接口后至少跑这一条；动了归属校验再补一条"越权被 403/404"的用例。
