# yekitrim-server/src/api — HTTP 领域模块（路由权威 + 共享底座）

> 范围：`crates/yekitrim-server/src/api`/** ｜ 上游：`crate::api::router`（`lib.rs` 与路由单测都走它）｜ 验证：`cargo test -p yekitrim-server`

## 职责

`/api/*` 的全部 handler、路由表与中间件装配都在这里，按领域一个文件一个模块。本目录只做 HTTP 层：
解析请求 → 调 `yekitrim-service` 的领域逻辑 / 起任务 → 回 JSON。同步 IO、任务体、路径归属这三件最容易出事的事
都有统一的落点（`blocking` / `spawn_job` / `scope.rs`），**别在 handler 里自己开一条路**。

## 入口

- `api::router(state: Arc<AppState>) -> Router` —— **唯一入口**（`mod.rs` 里 `pub use routes::router`；路由表本身在
  `routes.rs`，`lib.rs` 与路由单测都走它）。
- 共享底座（`mod.rs`，只放被两个以上子模块用到的东西）：`AppError` / `ApiResult` / `err(status, msg)` /
  `ok(v)` / `blocking(f)` / `block_on_here(fut)` / `spawn_job(jobs, job_id, body)` /
  `health`（`GET /api/health`）。
- 领域模块（每个只服务一个领域，括号内是代表端点）：

| 模块 | 职责 / 代表端点 |
|---|---|
| `routes` | **唯一的路由权威**：`scripts/route_manifest.py` 抽的就是它，拆分前后必须逐条相同 |
| `auth` | 会话 / 注册 / 改密 / doctor / 账号列表：`POST /auth/login`、`GET /me`、`PUT /auth/password`、`GET /admin/users` |
| `invite` | 邀请码（admin）：`GET/POST /admin/invites`、`DELETE /admin/invites/:id`、`PUT /admin/invites/registration`；注册语义（三重条件 / 失败一律 403 / IP 独立限流 / 明文落库）见 `crates/yekitrim-service/AGENTS.md` |
| `logs` | 运行日志与 LLM 用量（**仅 admin**）：`GET /admin/logs`、`GET /admin/logs/events`(SSE)、`GET /admin/usage` |
| `rooms` | 用户自助登记直播间：`GET/POST /rooms`、`DELETE /rooms/:room_id` |
| `config` | 程序配置 / ASR 模型 / LLM 测试：`GET /config`、`PUT /config/global`、`PUT /config/llm`、`GET /models`；路径面防线② 也在这 |
| `projects` | 项目托管：`GET/POST /projects`、`DELETE /projects/:id`、`DELETE /projects/:id/source`、`GET /project/list` |
| `project_data` | 项目数据面：`/project-data/{status,events,clips,transcript,pipeline,correct,config,assets,polish}`、`clips/:rank`(删除)、`events/:rank/cut`、`events/cut-all`、`fs/list`、`media/*path`；`assets` 的三种形态见下「素材清单端点」 |
| `pipeline` | 管线任务体 / 队列查询 / SSE：`run_pipeline_job`、`jobs_list`、`job_get`、`job_events`、`job_cancel`、`global_stream` |
| `render` | 渲染/封面出片子进程与进度解析：`run_polish_child(data_dir, jobs, job_id, sub, dir, extra)`（`extra` = 透传给子命令的额外参数，现在只有 `render` 用：`["--renderer", …]`）、`follow_render_output`、`RENDER_MILESTONES`（含底座那条腿的 `-- Dhampir …` 行）；里程碑 → progress/`jobs.log` 的分类规则与 `loudness.json`/`sticker_frames.json` 缓存失效口径见 `crates/yekitrim-polish/src/render/AGENTS.md` |
| `polish` | 精修编辑器后端：`/polish/{init,llm,llm-segment,process,render,cover,preview,cover-config,project,subtitles,silence,fonts,sticker-library}`。⚠️ `/polish/render` 的 `body.renderer` 是**白名单**（**只有 `dhampir`**；认不出 → **400**，不静默当默认 —— `chrome` 也落这条，那条腿已删除），不给就不带 `--renderer` —— 默认通道只有 `cli.rs` 一处。⚠️ `/polish/dhampir-status`（M3）= 底座**出片**通道就绪位（六项文件定位，不渲染）；`/api/health` 的 `dhampir_ready` 是它的**布尔投影**（health 无鉴权 ⇒ 只回布尔、不回路径） |
| `core` | 管线执行入口与进度桥：`run_stages`、`progress_bridge` |
| `test_support` | `TestApp`（整张路由表的接口级夹具），**只在测试编译** |

## 边界

- ⚠️ **素材库的读写都是用户层（2026-10-09 起；此前写限 admin），且真按 `Owner` 分库（2026-10-10）**：
  三个库（视频/贴纸/字体）在路由上归成一组
  **`let assets = Router::new()`**，merge 进 `protected`（⇒ **未登录仍 401**：放开的是"哪个用户"，
  不是"要不要登录"）。
  ⚠️ **这一组刻意不挂 `enforce_scope`**：库**不属于任何工程**（单人 = `<数据目录>/resources/*`，
  多用户 = `<数据目录>/users/<uid>/resources/*`，位置一律由 `yekitrim_core::owner` 算）；
  挂进去的后果实测过 —— `fonts/import` 那条带 `{dir}` 的请求会被 scope 中间件按"这个目录在不在你的
  项目里"判掉回 **404**，而它要的其实是"服务器上某个目录里躺着一支字体"。
  ⚠️ **同一路径别在两个 Router 里各挂一次**：axum merge 时 panic（`Overlapping method route`），
  **不是**后者覆盖前者（实测漏删一份 `/polish/video-library` = 52 个单测全红）。
  读是 `GET /api/polish/fonts`（`fonts::scan_all`，多回 `display_name/builtin/is_default`/`flat`
  /`fallback_stack`）；写是 `POST /polish/fonts/{import,upload,default,builtin}` 与
  `DELETE /polish/fonts?id=`（`builtin=true` → **403「程序自带字体不可删除」**，判断在 handler 里）。
  贴纸端点 `/polish/stickers[...]` **2026-10-09 从 `/admin/stickers/*` 挪来，老路径已删除**。
- **素材库取文件（`GET /polish/asset?kind=video|font&file=`）与 `GET /polish/video-thumb?file=`**：
  ⚠️ 前者的**安全性质是"只认单个文件名"**（`is_safe_component` 挡分隔符与 `..`，根目录由
  `asset_kind_root` 按 kind 现取）—— 一旦 `file` 认 `/`，普通账号就能拿
  `?kind=font&file=../.yekitrim/llm.toml` 读走 API key。有单测钉在**穿越尝试**上（不是"正常文件名
  能取到"），去掉那道判断立刻红。⚠️ 贴纸**不收编**进来（两级：包/包内文件，收编就得让 `file` 认 `/`）。
  后者抽第 1 秒那帧（<1s 退到第 0 帧）、按 `<库>/.thumbs/<名>@<mtime>.jpg` 缓存；
  ⚠️ **抽不出来回 404 不是 500** —— 没装 ffmpeg / 容器怪异 / 文件损坏都只是"这张没缩略图"，
  回 500 会让界面报读取失败，而其它素材全都好好的。
  ⚠️ **写路径（导入/上传/装自带）必须走 `fonts::ensure_dir_for(owner)`**，**`owner` 不能省**：
  这三个接口在**用户路由**上，而 `fonts::ensure_dir()` 是单人库入口 —— 2026-10-10 实测其后果是
  "传了却看不见"（字节落进别人的库）。改后实测 carol 上传 → `users/2/resources/fonts/`，
  admin 列表里没有它。`set_default`/`remove` 同样按 `asset_dir(owner, Fonts)`。
  ⚠️ 这三条**有意不挂 `enforce_scope`**（见本卡「素材库」那条）；导入用"服务器上的路径"
  而非上传，理由同 `/api/fs/list`。
- **`POST /api/polish/cover`** 的 body 多了可选 **`renderer`**（现在**只有 `takumi`** —— `chrome` 那条截图腿 2026-10-01 整条删除，传它由 `cover.rs` **明确报错**）：服务端**只透传**成子进程的 `--renderer`，取值口径与白名单只在 `crates/yekitrim-polish/src/cover.rs`（**只此一份**，不在这一层抄第二份），认不出的值由那里**报错**。**能不能用**（编了 feature + 字体库非空）回在 `/api/health.cover_static`（`{compiled, fonts, fonts_dir, ready}`）—— 前端据此把「生成封面」置灰并在卡片里写原因（通道弹窗已删）；health 是公开接口，只回布尔 + 字体库路径（本机配置问题，不是账号/工程信息）。

- ⚠️ **`/polish/dhampir-wasm/*` 的响应压缩与 ETag/304**（2026-10-02 首屏提速，只在这条路由上开）：
  那份 `pkg/dhampir_wasm_bg.wasm` 是 **7.66 MB**，以前原样发出 ⇒ 每次进精修页都搬 7.66 MB。
  现在走 `media::serve_file_cond`：实测 **8,036,021 → 1,492,985 字节（18%）**，配合 ETag 让
  「文件没变」的刷新直接 304（几乎零字节）。三条**硬规矩**：
  ① **只压文本类**（`media::compressible_mime` 白名单：wasm / js / css / html / json）；
  ② **有 `Range` 头一律不压** —— 压缩后与「原始文件第 start..end 字节」不再一一对应，
     对 206 做压缩会直接破坏播放；`/api/media/*`（视频）仍走老签名 `serve_file`，行为不变；
  ③ **ETag 取 `len+mtime`**（`media::file_etag`），**不是**「永不变」的常量 —— 这条路由的既有
     意图是「Dhampir 重建 wasm 后刷新即新」，靠 mtime 自然失效；写成 max-age 长缓存会破坏它。
  `Cache-Control: no-cache` 是「用前先校验」，不是「不许缓存」。
- **`routes` 是唯一的路由权威**；加接口 = 在 `routes.rs` 挂一条 + 在对应领域模块写 handler。
  ⚠️ **本地免登录（`--local-no-auth`）/ `POST /api/shutdown` 控制通道**：正文（两道闸、三道闸、
  互斥与开机守卫、哨兵会话、`local_no_auth` 布尔、令牌头名与托盘各写一份字面量）见 `docs/deploy.md`。
  判据：`local_no_auth_guards_reject_foreign_host_and_cross_site` /
  `shutdown_token_gate_is_exact_and_empty_safe` / `shutdown_endpoint_is_404_without_token_env` /
  `unauthenticated_requests_are_401_except_whitelist`（免登录只允许 loopback、与 `--allow-remote` 互斥）。
  ⚠️ `GET /api/project-data/config` 同时回 `config`（**合并后**生效值）与 `project_config`
  （**项目文件自己**那份）：预处理页靠后者区分"本工程写了"与"跟随全局"
  （`merge` 对空串不覆盖 ⇒ 清空该键即恢复跟随全局）。只回合并值就分不出这两者。

  ⚠️ 两层的顺序**不能反**：`require_user` 必须在外层（先跑并写入 `CurrentUser`），改密强制门在内层才读得到用户。
  axum 里"后加的 layer 在外层"，所以 `routes.rs` 里那两行**看起来是倒着写的**（有注释，别"顺手改正"）。
- ⚠️ 子模块里的 `super::` 是 `api`、**不是 crate 根**；`api/auth.rs` 与 `crate::auth` 同名不同物，
  新增代码请写全 `crate::auth`。子模块对外的项一律 `pub(super)`（只有 `api` 及其后代可见），
  被两个以上模块用到的 helper 放 `api/mod.rs`；只服务一个领域的 helper 跟着那个模块走。
- 未注册的 `/api/*` 返回 **404 + JSON**（`{"ok":false,"error":"未知接口: …"}`），不会回退成 SPA HTML
  （否则前端会把 200 HTML 解析成 `null` 静默失败）。
- `/api/fs/list` **仅 admin**，可以浏览服务器任意目录（`{dir}` 绝对路径；也兼容 `{root, path}` 相对写法）。
  这是"服务器上已有文件"这条 admin-only 来源，**永不接受上传**；`[storage] local_roots` 只是前端下拉的
  **快捷入口**，不再是硬限制。**进入下一级用 `{dir, enter:"子目录名"}`，路径由服务端拼**：前端自己拼
  `dir + '/' + name` 会把 canonicalize 的 `\\?\F:\a` 拼成 `\\?\F:\a/b`（Windows ERROR_INVALID_NAME 123）。
  因此所有出 JSON 的路径都过 `yekitrim_core::path::display()`（剥掉 `\\?\`/`\\?\UNC\`），前端拿到的永远是
  `F:\a\b` 这种可回传、可拼 URL 的形式。
  响应里的 **`volumes`**（2026-10）= 可浏览的盘符/卷候选，给“服务器文件”浏览器铺在目录树顶部。
  ⚠️ 加它的原因：目录树只能靠 `parent` 往上展开，**到盘根就断了**（盘根的 parent 是空），
  用户**换不了盘**；而配置根默认只有租户目录，管理员真要去的 `F:/media/...` 不在它下面。
  ⚠️ `volumes` 只是**展示候选、不构成访问限制**（本接口本来就允许 admin 浏览任意绝对路径）。
  ⚠️ 探测要跟目录列举一起放 `spawn_blocking`（每个候选一次 `is_dir()`，是阻塞 IO）。
  ⚠️ 前端**别把 `roots` 当树根**：服务端会把**当前目录**追加进 `roots`（本意是给下拉框显示“当前位置”），
  拿它当根的话根会随用户走到哪而变。
- `GET /api/admin/users` 只回展示字段（id / username / role / disabled / quota_bytes / created_at），
  **不含口令哈希**，供"待分配池指派给谁"下拉使用。
- **邀请码与注册**（`POST /auth/register`）：三重条件 = `[auth] registration_enabled` 开着 + 邀请码有效 +
  未过期；失败**一律 403**（不区分"码不对/码用过"，防被拿来枚举），限流按 IP（key `register|<ip>`，
  与登录**分开**）；账本 v5 的邀请码**明文落库、可重复查看**（与 `yekitrim ingest token`"明文只见一次"
  是**两条相反的规矩**）。口径正文与消费实现都在 `crates/yekitrim-service/AGENTS.md`，本卡不重复搬运。
- **素材清单端点 `GET/POST /api/project-data/assets`**：`POST` 三种形态 ——
  `{kind, path}` 与 `{kind, dir, name}`（写前先解析一遍，解析失败一律不落盘；`{dir, name}` 这条
  **就是为了不让前端拼路径**）、`{kind, path: ""}` 只清生效值（**清单不动**，候选还在）、
  `{kind, action: "remove", path}` 摘清单那条并回 `removed: true`（清单里没登记但它正是生效值 →
  当"停止使用"放行；ASR 产物在文件还在时 → 400）。⚠️ `slots`（`{hybrid, single}`）**由服务端算**
  （遍历 `yekitrim_core::config::ASR_MODES` 调 `slot_bound`，前端**不自己从 `items` 推**）、
  ⚠️ **清单写入必须单次 `patch_config_file`**（生效值 + 整份清单走同一个 patch）—— 这两条以及
  清单落地形态 `[project.materials]` 的正文在 `webui/src/views/AGENTS.md`，本卡不重复搬运。
- `/api/polish/*` 的 `dir` 必须是非空且存在的目录；`polish.toml` / `events.json` / `yekitrim.toml` / 全局配置的读-改-写
  走 `yekitrim_core::filelock::with_path_lock` 串行化；**写 `polish.toml` 一律走路径锁**（编辑器保存、去静音、封面配置、
  `cmd_parse` 全在改同一份文件 —— 无锁的 read-modify-write 会静默丢改动）。
- ⚠️ **渲染 / 封面任务的收尾日志只报状态、不带路径**（2026-09-21 用户要求）：成功行就是
  `成片已写出` / `封面已写出`。子进程写出的是 `\\?\E:\media\…\vtuber-clip_1.mp4` 这种 verbatim 长路径，
  塞进任务日志会撑满任务框那一行（那里只有一行高度）；实际文件名在精修页「产物」区
  （`rendered_file` / `cover_file`，含 `_1`/`_2` 版本）与切片卡片上都能看到，日志不必重复。
  `subprocess_note` 仍然要跑 —— 它是「子进程真的写出了文件」的证据，不是装饰。
- ⚠️ **剪切收尾要补精修工程 + 预热响度曲线**（用户口径：剪切这个行为本身包含精修 init）：单条剪切与一键全切共用的
  `run_single_cut_event` 在 `Cutter` 跑完后调 `yekitrim_service::init_polish_for_events(...)` —— 它顺手把
  `<片段>/data/loudness.json` 也量好（只补缺失，精修页 `preview=1` 那步因此只是读文件，正文在
  `crates/yekitrim-service/AGENTS.md`）；本模块的 `ensure_polish_project`（去静音接口的隐式初始化）只是同一个
  `init_polish_for_clip` 的 HTTP 包装。
  「剪切完成 ⇒ `polish_ready`」是切片页「精修」按钮不再撞 `polish.toml not found` 的前提，
  **别在 handler 里另开一条 init（或预热）路径**（两份实现必然漂移）。
- `/api/health` 是**未认证**接口：不含账号数量、不含"是否仍用初始密码"（那等于告诉远端"这台机器还在用初始密码"），
  也不含任何路径；要看这些走 `/api/admin/users`、`/api/me` 或 `yekitrim doctor`。`GET /api/admin/usage` 与
  `/api/admin/logs` **仅 admin**：记录含**其他用户的工程路径**与任务输出（两者同一个理由）。

### 同步 IO 纪律（剪切时"整个服务器连不上"的根因，2026-09）

- ⚠️ **同步 IO 绝不能留在 async 线程上（剪切时"整个服务器连不上"的根因，2026-09）**：
  项目目录经常在网络盘上（RaiDrive/WebDAV 挂载的录播目录），剪切/渲染把盘占满时，
  一次 `exists()`、一次读 `yekitrim.toml` 就要几百毫秒到几秒。这类调用留在 tokio worker 上做，
  十几个并发（前端一次页面加载就会打出一串）就能把 worker 全占住 —— runtime 连 accept
  都排不上，浏览器表现就是"服务器无法建立连接"，而日志里一行都没有（请求根本没处理完）。
  实测样本：`GET /api/projects` 有 2288ms 的记录，日志里 1 秒内 52 次连发。
  - **handler 里的同步文件访问**一律走 `api::blocking(f)`（`spawn_blocking`，512 线程的
    blocking 池），worker 留给 accept 与轻量接口。已覆盖：`projects_list` / `project_list` /
    `project_data_{status,events,clips,polish,transcript,config}` / `clips/:rank/thumb` /
    `clips/:rank`(删除) / `clips/:rank/silence` / `events/:rank/cut` / `events/cut-all` /
    `polish_project_get` / `polish_subtitles_get`。**新增读项目目录的接口照着做**。
  - 需要「async 判定 + 同步 IO」混在一起的实现（如删除片段要先问 `busy_with`）用
    `blocking(move || block_on_here(xxx_impl(...)))` —— 整段搬到 blocking 线程，自带一个
    current-thread runtime。
  - **管线任务体**一律用 `api::spawn_job(jobs, job_id, body)`（专用线程 + current-thread
    runtime）。管线（转写/检测/剪切/精修 process）是整段同步代码，跑在 worker 上就是白占一个到
    它结束（实测 59s 的剪切占 37s，转写是分钟级）。`cut_all` 内部的逐片段剪切走 `spawn_blocking`
    （3 路真并行），进度在 await 侧统一汇报。**入库自动管线 / `models_download` /
    `stickers::spawn_build` 也走 `spawn_job`**。
  - `spawn_job` 带 **panic 兜底**（`catch_unwind` + 替任务调 `fail()`）：任务体的
    `complete/fail` 在它自己末尾，中间 panic 会让 Job 永远 `running`、资源键永不释放 ——
    同项目/同片段的后续任务会全部排队饿死。新增任务入口时把 `(jobs, job_id)` 传进去。
    ⚠️ **两条兜底缺一不可**：panic（`catch_unwind`）与**线程都起不来**（`spawn.await` 之前就 `Err`）时，
    也必须替任务 `fail` —— 只 `tracing::error!` 就完了的话，Job 停在 Running 谁也查不出来。
    注意 **`[profile.release]` 不再设 `panic = "abort"`**（2026-09 改）：panic 那条兜底在 release 里
    **真的会跑**，所以新增任务入口必须把 `(jobs, job_id)` 交给 `spawn_job` —— 否则 panic 会让 Job
    永远停在 Running。别把 abort 加回去：单进程服务下那是"一个人搞挂所有人"。
  - 其余同类修复（一并照此维护）：`serve_file` 的元数据、`/api/media` 的归属校验 + `jail_existing`
    （播放会打出一串 Range 请求）、`polish_silence`/`polish_project_put`/`config_*_put`
    （`with_path_lock` 的闭包跑在当前线程，改成 `lock_path().await` + `blocking(...)`）、
    `polish_cover_frame`/`preview` 的截帧 ffmpeg、入库的扫描与跨盘搬运、`POST /api/admin/ingest/scan`。
  - **前端取数要单飞**：`stores/workbench.js::loadProjects()`（`inflight` 去重）与
    `composables/useJobs.js::load()`（在飞合并 + SSE 250ms 合帧）。视图注册 `onJobDone(fn)`
    **必须用返回的退订函数在 `onUnmounted` 里退订** —— 不退订会随反复挂载攒出一串陈旧回调，
    任务结束时同时各打一整套 project-data 请求（就是那次 1 秒 52 连发的放大源）。
    `loadProjectData` 的 `stale` 标记也别删：任务结束撞上在飞请求时，旧响应会盖掉新数据。

### 路径面三防线（多租户的头号风险，改动前先读）

**服务端能读写的文件范围 = 请求里出现的每一个路径字段，而不是"项目目录"这个概念。**
曾经有三处漏了归属校验，普通登录用户就能把服务器上任意文件（`<数据目录>/.yekitrim/llm.toml`
里的 **LLM API key**、账号账本 `cloud/yekitrim.db` 里的明文邀请码与全部真实路径）复制/读进
自己的项目，再用 `/api/media` 取回。三道防线，缺一不可：

| 防线 | 位置 | 规矩 |
|---|---|---|
| ① polish body 字段收口 | `yekitrim-server/src/scope.rs::enforce_scope` | body 里每个"会被 handler 拿去开文件"的字段都要收口：**相对项目根**的进 `EXTRA_PATH_FIELDS` 名单（`source` / `cover.image`），`dir` / `output_file` 单独处理，贴纸的 `asset` / `assets`（**相对 `dir`**，基准不同进不了那个循环）走 `is_safe_project_relative` + `bad_sticker_path_in` 单独收口。**新增此类字段必须同步收口**，漏一个就是一条任意文件读取。 |
| ② 项目配置路径键 | `yekitrim-server/src/api/config.rs::PROJECT_PATH_KEYS` + `check_project_paths` | `PUT /api/project-data/config` 的 `raw` 与 `patch` 两条分支都要过这张表（`input_file` 等 5 个 + `cutting.output_dir` + **`[ingest]` 的 `ytdlp_path`/`cookie_file`/`recorder.root`**）：非 admin 必须能 `jail_join` 进自己项目。`section` 支持点号路径（`ingest.recorder` 是两级表）。**素材清单也在这条线上**：同一函数末尾的 `check_material_paths` 逐条校验 `[project.materials].{transcript,danmaku}[].path`（2026-09 预处理页的清单，漏了就成逃逸口）。`Workspace::resolve` 对**绝对路径原样放行**，所以校验只能在这里做。admin 豁免（用「服务器文件」建项目时 `input_file` 本来就是项目外的路径）。 |
| ③ 消费侧纵深防御 | `yekitrim-polish/src/cover.rs` | 渲染封面时对 `cover.image` 再做一次 `tenant::ensure_within`。handler 层的校验只覆盖"走 API 的那些入口"，别删这道。 |

⚠️ 相关的两个既有约定也别松：`every /api/media` 走 `jail_existing`；`is_safe_component`
挡路径拼接（删除片段/贴纸文件名）。

⚠️ **`Path::is_absolute()` 在 Windows 上挡不住根相对路径**（2026-10-01 给 `/polish/sticker-frame`
加预热接口时发现；那条洞**早就在 `sticker-frame` 里**，只是没人从那儿穿过）：
`Path::new("/etc/passwd").is_absolute()` 在 Windows 上返回 **false**（无盘符/无前缀），
而 `Path::join` 对"有根无前缀"的路径**会替换掉基准** —— 实测
`Path::new(r"C:\proj").join("/etc/passwd")` → `C:/etc/passwd`，**已经出了工程目录**。
所以判"工程内相对路径"必须**逐字符**来：空 / 绝对 / **以 `/` 或 `\` 开头** / 含 `:` / 含 `..` / 含 `\` 全挡。
现成的**唯一一份**在 `scope.rs::is_safe_project_relative`（`pub(crate)`）：scope 中间件（第一道，
`bad_sticker_path_in` 收口 `asset`/`assets`）与 `api/polish.rs` 两个 handler（第二道兜底）共用，
**不许各写一遍**。无论用什么办法，**都要有一条单测**钉住
`/etc/passwd` 与 `\windows\x` —— 这两条正是当年漏掉、而我这次是靠写测试才发现的原因。

> 改动只允许落在本目录；跨模块改动先读对方卡，并跑两条门禁。

## 验证

```bash
cargo test -p yekitrim-server
```

接口级单测（`test_support::TestApp` 建的是整张路由表）：JobManager / Range / 路由 404 / 配置 patch。
加接口后至少跑这一条；动了归属校验再补一条"越权被 403/404"的用例。
