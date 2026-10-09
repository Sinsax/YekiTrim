# yekitrim-server — axum HTTP API + WebUI 宿主 + 任务队列

> 范围：`crates/yekitrim-server`/** ｜ 上游：`yekitrim-cli`（只能经 `run()` / `ServerArgs` 进入）｜ 验证：`cargo test -p yekitrim-server`

## 职责

`yekitrim server` 的落点：axum HTTP API（`/api/*`）+ WebUI 宿主（**release 内嵌 `webui/dist`**，见下）+ 任务队列，并经 SSE 推送任务日志与
进度。领域逻辑一律取自 `yekitrim-service`；本 crate 只做 HTTP 装配、任务调度与文件系统边界，handler 全在 `src/api/`。

## 入口

- `src/lib.rs`：`pub async fn run(args: ServerArgs)` —— 唯一启动入口（`yekitrim server` 与 `polish preview` 回退都走它）；
  `run()` 现在只是薄壳 = `run_with(args, ctrl_c(), |_| {})`；**`run_with(args, shutdown, on_listening)`** 是可控版
  （托盘用它：`shutdown` 一到就优雅关停、`on_listening` 回**实际**监听地址 —— 端口被占时服务端会自己换一个）。
  `request_shutdown()` 是**进程级**关停触发器（HTTP 控制端点用，见 api 卡 `/api/shutdown`）：
  `run_with` 进来把它挂上、退出摘掉（存的是本次运行那个 `Notify`，不是 `OnceLock` —— 否则
  上一次留下的 permit 会把下一次**立刻**关停）。`run_with` 把"调用方给的 future"与它 **select 成一个**。
  ⚠️ **优雅关停必须有时限**（`SHUTDOWN_GRACE = 3s`）：`axum` 的 `with_graceful_shutdown` 会等**所有连接**收尾，
  而 WebUI 有一条**长期挂着**的 SSE（`/api/stream` 任务事件流）—— 页面一开着就必然有，
  于是"退出/Ctrl+C"会**永远等不到返回**（实测：托盘点退出后图标还在、日志里连一条关停记录都没有）。
  现在信号之后再给 3 秒，到点停止监听（日志一条 WARN），锁照样正常释放。**别把这段时限去掉。**
  `ServerArgs { host, port, open, config, webui_dir, allow_remote, trust_proxy, data_dir, admin_password_file, force, open_path }`；
  `first_bindable_port` / `bind_conflict_hint`（绑失败**实测试绑**再报；`10013`=系统拒绝，`10048`=真被占用）。
- `AppState`（`src/model.rs`）：共享句柄（`AuthService` / `JobManager` / 日志总线）。
  ⚠️ **里面没有"当前工程"** —— 见下面那条不变式。
- `jobs.rs`：`JobManager`（`create` / `wait_start` / `cancel` / `complete` / `fail` / `busy_with` / `spawn_housekeeping`）、
  `JobSpec`、`JobHandle`、`JobSummary`、`JobEvent`、`JobClass`、`JobPriority`。
- 其余模块见「边界」。`docs/job-queue.md` = 任务队列说明书，**改档位或上限要一起改**。

## 边界

- **依赖方向（第一条）**：**只依赖 `yekitrim-service`**，绝不反向；**不碰 `yekitrim_cli`**。两条都已是机器门禁
  `scripts/check_module_boundaries.py`（见根卡）。
- ⚠️ **没有"全局当前工程"这回事**（2026-10-03 删除）：`AppState` 里曾有一份 `workspace` /
  `workspace_root`，是**单工程时代**的遗留。P2 起归属由 DB 的 `projects` 表决定，每个请求经
  `scope.rs::Scope` 提取器按 `?project=<id>` **现场解析**出自己那份 `Workspace`。
  那份全局状态于是成了一条**没人查的旁路**（删它时全仓只剩两个消费者，且都已改掉）：
  ① 过渡期 `/api/media/<绝对路径>` —— **已删**，媒体路径现在**必须**带项目 id；
  ② `/api/project/list` 回显的 `workspace_root` —— **已删**（那个接口自己就标着 deprecated）。
  **要工程就接一个 `Scope`**；要"项目无关"的值就直接读 `path::yekitrim_home()` /
  `load_global_config()`。`server.toml` 里**只留记忆**（`last_config_file`，供启动恢复上次工程），
  **不再留状态** —— 记忆允许过期，状态不允许。
  ⚠️ 同一次还把 `/api/config` 与 `/api/project-data/config` 拆开：前者是**纯程序配置**
  （`config.toml` / `llm.toml`，只回 `global_raw` / `llm_raw` / `global_config` / `llm_config`），
  后者才是"这个工程"的（`yekitrim_raw` / `config` / `config_file` / `root_dir`）。
  两者曾共用一份实现，于是项目相关字段被挂在项目无关的接口上。
- ⚠️ **`wgpu-render` feature 已整条删除**：`api/` ×3 与 `model.rs` ×5 那些 `#[cfg(feature = "wgpu-render")]`
  分支（单帧真渲染预览 `POST /api/polish/render-frame`、常驻后端槽、`/api/health` 的 `wgpu_render` 能力位）
  连同 `yekitrim-polish/wgpu-render`、`yekitrim-render-core/wgpu`、`yekitrim-wasm` 一起没了 ——
  GPU 那侧交给独立的 Dhampir 底座。所以本 crate **没有任何 feature**。
  **别再把它加回来**：加了不报错，只会让前端以为还有 GPU 出片入口。
- ⚠️ **WebUI 两个来源，按构建形态自动选**（2026-10-03）：**release 内嵌**（`build.rs` 把
  `webui/dist` 用 `include_bytes!` 编进 exe）· **debug 读盘** · `--webui-dir`/`YEKITRIM_WEBUI_DIR`
  两种形态都覆盖。规则 = `web.rs::choose()`（纯函数，四种组合都有判据）。
  ⚠️ **release 即便磁盘上有 dist 也用内嵌那份** —— 否则在开发机上打 release 会"内嵌一份、跑的是
  仓库里那份"，**测的与发的分家**（这是本次改动的全部意义）。⚠️ `dist` 缺失或比 `webui/src` 旧 ⇒
  **编译失败**（原先只在 `cargo xtask package` 里查）。
  ⚠️ **磁盘那条路的候选次序仍是契约**：`exe同级 → exe上两级 → 构建期常量 → cwd`，
  `current_exe()` 那两条必须在编译期常量前 —— 否则换台机器/换 cwd 会静默端出**开发机源码树里**
  那份（判据见 `web.rs::tests`）。别把本 crate 挪深，否则构建期那条指到 `crates/`。
- `src/web.rs` 的 503 只在**显式覆盖指到空目录**时出现；启动日志打一行 `静态资源：内嵌（N）`。
  ⚠️ 那行**不许以 `WebUI: http://` 开头** —— 托盘靠那个前缀抓地址（跨进程契约）。

### 任务队列（`src/jobs.rs` / `src/scope_key.rs`）

- 三档**按本地资源占用分，不按时长分**：`Light` 不占闸门 / `Medium`(3) / `Heavy`(2)，上限集中在 `JobClass::cap()`。
  `create` 返回 `JobHandle { id, queued, .. }`：本档名额没满 **且** 资源键与**在跑**任务不相交 → 直接开跑，否则进
  `Queued` 按 `(优先级, 创建顺序)` 放行，**冲突不再 409**；优先级只在同档内插队，不抢占在跑的。
- ⚠️ **档位豁免的只是闸门名额，"写同一批文件"的物理冲突照旧**：轻活撞上在跑任务的资源键
  仍然排队（有单测）。所以调度器里名额满 / 资源冲突都是 `continue` 而不是 `break` ——
  重活排满不该挡住后面的轻活/略重；同档 FIFO 由"名额满则本档后续全不满足条件"自然保持。
- 资源键（**相等匹配**）：`p{pid}:out`（管线/剪切/订正/入库：events.json 与 `output/*`）、`p{pid}:idx`（所有 cut
  共写 publish_config.json + clips_order.txt）、`p{pid}:clip:{clip-07}`（单条重切 + 该目录上的精修）、`models`
  （下载模型 vs 转写 `ensure_models`）、`stickers`（表情包构建）、`dir:{规范化绝对路径}`（精修目录兜底键）。
  **全切刻意不带 `clip:*`**：它只处理没有 `clip.mp4` 的目录，与精修并行是安全的（用户拍板）；单条重切与同片段的
  精修必须互斥。
- 每个创建点（8 处，清单见 `docs/job-queue.md`）都必须先 `handle.wait_start()` 再干活 —— 排队期间被取消时它返回
  `Err`，**直接收工**。
- **任务一定要有终态**（否则**永久**占着资源键与档位名额，同项目后续任务无声排队）：① 任务体 panic / 线程起不来 →
  `api::spawn_job` 的兜底替它 `fail`（`[profile.release]` **不再** `panic = "abort"`，所以 panic 那条兜底真的会跑；
  `api::tests::spawn_job_marks_job_failed_on_panic` 钉住它。**别再把 abort 加回去** —— 那等于一个人 panic 带走所有人）；
  ② **外部进程要有超时**：`cutting::cut_segment` 的 ffmpeg 走 `run_ffmpeg(cmd, timeout, what)`（按片长给
  `max(3×时长, 180s)`）。新增外部进程调用照这个来，别再写裸 `.output()`。
- **调度器自愈 tick**：调度是事件驱动的，只要有人永远不返回就**没有任何事件**再触发它。`jobs::spawn_housekeeping`
  （`server::run` 里起）每 `SCHED_TICK`(5s) 重跑一次，排队超 `BLOCKED_WARN`(120s) 打 warn **点名是谁挡住谁、因为什么**
  （`blocked_report`）—— 这就是"为什么我的任务不动了"的答案，别删。
- **淘汰只吃"已终态且不再占资源"的**（`Job::evictable`）：单看 `is_active()` 会漏掉"已取消但协程还没退出"这类；
  淘汰它会让它之后的 `complete`/`fail` 找不到 Job，**没人再触发调度器**。取消 = `cancel()` 置状态位 + 丢开跑信号；
  **已取消但协程还没退出的任务仍然占着资源**（`worker_done`）。
- `MAX_CREATE_PER_MINUTE` / `MAX_ACTIVE_PER_OWNER` / `MAX_ACTIVE_TOTAL`：这套队列里**唯一的"拒绝"路径**
  （闸门与资源键只排队）。超限 = 任务记失败 + 丢掉开跑信号（`wait_start()` 报 Err）。**别给轻活加闸门名额**来替代它
  —— 那会把编辑器每次改动都要重跑的 `polish_process` 一起挡住。
- 同步接口（没法排队）用 `jobs.busy_with(&keys)` → 409 + 文案说明谁在占用：删片段 / merge-splits（events.json）/
  改屏蔽轨道 / 预览的隐式 process。
- **长任务写 polish.toml 一律走 `project::update_polish_toml(dir, |cfg| …)`**（锁内重新 load + 只覆盖自己负责的字段）。
  **不许再"开头 load、结尾整份写回"**：那样 LLM 跑两分钟期间用户存的选框/封面/屏蔽会被结尾那次覆写静默吞掉。

### 渲染子进程

- **输出必须落文件，不能用管道**：`api::run_polish_child` 把子进程 stdout/stderr 重定向到
  `<data_dir>/logs/render-<job_id>.log`，再用 `follow_render_output` 边跑边读。
  ⚠ 绝对不要改回 `Command::output()`/管道：ffmpeg 与底座拉起的孙进程是**孙进程**，它们会继承 stdout
  句柄 —— 只要还有一个活着，管道就永远不 EOF，`output()` 一直等下去。
- **取消 = 杀进程树**（`render_proc::kill_tree(pid)` → `taskkill /F /T`，含 ffmpeg 与底座拉起的孙进程）。**启动清扫孤儿**：
  每次渲染往 `<data_dir>/render-procs/<job_id>.pid` 登记 pid，`server::run` 启动时 `render_proc::sweep_orphans`
  结束遗留渲染（**先核对命令行**再杀，pid 会被系统回收）。

### 归属校验与作用域

- `src/scope.rs`：`/api/polish/*` 的**路径面收口**（`enforce_scope` 的 `EXTRA_PATH_FIELDS` +
  `resolve_file_in_project`）—— 细则见 `src/api/AGENTS.md`「路径面三防线」。
- ⚠️ 相关的两个既有约定也别松：`every /api/media` 走 `jail_existing`；`is_safe_component`
  挡路径拼接（删除片段/贴纸文件名）。
- 任务带 `owner_id`：`/api/jobs*`、cancel、`/api/stream` 全部按归属过滤，他人任务 404。
- ⚠️ **「移除项目」不碰任何文件**（用户拍板）：`yekitrim.toml`、媒体、`output/` 产物全留着，重新导入同一目录即可再绑
  （`create_local_project` 见到已有 `yekitrim.toml` **只登记、不覆盖**），新登记拿**新 id**。两个入口共用
  `api::unregister_project`（`DELETE /api/projects/:id` 与过渡别名 `POST /project/delete`）。
- ⚠️ **`persist::forget_project` 必须同时清 `last_config_file`**，少了这一步"移除"会被静默撤销：`lib.rs` 每次启动
  都把"当前工程"写回 `server.toml` 的 `projects[]`，而 `import_legacy_projects` 会把 `projects[]` 里**"配置文件还在、
  DB 没行"**的路径重新登记到 admin 名下 —— 注销不删文件，所以那个配置一定还在。

### 采集入库（录播姬 / B站）

- ⚠️ **会话状态机只有 `closing` 会被入库**（`ingest_worker::is_finalized()` 只认它）：**指派/导入必须把待分配会话
  推到 `closing`**（`Db::assign_ingest_session`）；`'finalized'` = **项目已建好**，别当"已指派"用；扫描
  （`register_scan_session`）每两轮重走一遍，**不能把已有归属的会话打回 `unassigned`**（改状态前先看 `owner_id`）。
- **房间号必须从路径里解析**（`room_id_from_dir_name` / `room_id_from_rel_path`），**不能拿整个目录名当房间号**：
  录播姬默认目录名是 `<房间号>-<主播名>`，而 `channel_map` 存的是 `26320007`。
- `GET /api/rooms/:id/recordings` 是**只读**；`POST /api/rooms/:id/recordings/import {rel}` 导入**单个**文件，
  两道校验别删：文件必须**属于该房间**（`room_id_from_rel_path(rel) == room_id`）、路径过 `resolve_recorder_path`。
- ⚠️ **不要为 B站另开"下载暂存目录"**：那等于把 P4 专门去掉的"搬入"又加回来。下载器直接写进托管项目目录
  `<projects_root>/users/<uid>/<bvid>/<YYYYMMDD-HHMMSS>-<pid>/`；要换地方就改 `[storage] projects_root`
  （只影响**新建项目**；改完**立刻生效**），保存时 `check_projects_root` 会真 `create_dir_all` + 写探针。
- **两个 worker 并列**：`ingest_worker.rs`（60s 兜底扫描 + 入库）与 `download_worker.rs`（下载槽 + **唯一 spawn
  子进程**处 + 无输出超时 kill + 僵尸回收）**不许互相 import**；共用部分走 `yekitrim_service::ingest::{capacity,landing}`
  或 `yekitrim_server::pipeline_boot`。⚠️ 这条以及「两个 worker 不许互调」**已经是有脚本的门禁**：
  `scripts/check_module_boundaries.py`。
- B站会话与录播姬**共用 `ingest_sessions`**（靠 `kind` 列区分）：⚠️ **别拿 `room_id` 塞 bvid** —— 会被 `channel_map`
  按房间号查的规则命中。⚠️ `projects.room` 也**不能**塞 bvid。命令构造/哨兵/弹幕旁路见
  `crates/yekitrim-service/src/{downloader,bilibili}/AGENTS.md`。
- **采集配置端点（`src/ingest.rs` 在 crate 根，三个都挂在 `require_admin` 层下）**：
  - `GET /api/admin/ingest` → `config_view_json()`：**只回 `api_pass_set` 布尔，绝不回 `api_pass` 明文或
    `token_hash`**（表单回填不需要它们；要看原文走 `GET /api/config` 的 `global_raw`，那是 admin 专用、
    也是源码编辑器 round-trip 的前提）。
  - `PUT /api/admin/ingest` → 校验在纯函数 `build_ingest_patch`（可单测、不碰真实全局配置）。语义：
    **键缺席 = 不改**、给空串/空数组 = 清空（`api_pass` 缺席即"不改密码"）。`allow_ips` 只收 IP 字面量
    （运行时是**精确字符串相等**，放行 `192.168.1.0/24` 只会让来源被静默拦掉）；`api_base` 非空必须带
    scheme；`root` 不可达**不拒绝**、只回 `warnings`（网络盘可能只是暂时没挂上）。审计只记**键名**
    （值里可能有口令）。
  - `POST /api/admin/ingest/token {action}` → `generate` 明文**只在这一次响应里**回，`disable` 只关开关
    并保留 token。WebUI 侧表单（`components/RecorderConfig.vue`）见 `webui/src/components/AGENTS.md`。

### 贴纸 / 认证 / 限流

- `src/stickers.rs`：路由在**用户层**（**不是** admin-only —— 贴纸库按 `Owner` 分，见 `src/stickers.rs` 头部；
  单人 = `<数据>/resources/stickers/`，多用户 = `<数据>/users/<uid>/resources/stickers/`）；`spawn_build` 走 JobManager（占 `stickers` 键，取消在
  **批次边界**生效）；64 MiB `DefaultBodyLimit` **只给上传那一条**放开（axum 默认 2 MB）；`.../:pack/files/:name` 与
  `/api/media/*` 共用 `media::serve_file` 的 Range 实现；包名过 `sanitize_pack_name`、文件名过 `is_safe_component`。
- `src/media.rs::serve_file` 是**唯一**的文件出口（`/api/media/*`、贴纸文件、Dhampir wasm 资源）。
  ⚠️ **压缩与 ETag 只在 `serve_file_cond` 上开、只给 `/polish/dhampir-wasm/*` 用**（2026-10-02 首屏提速，
  7.66 MB wasm → 1.49 MB）：视频走 `Range`，压缩后字节与「原始文件第 start..end 字节」不再一一对应，
  对 206 压缩会破坏播放 ⇒ **有 `Range` 头一律退回 identity**，且白名单里只有文本类（`compressible_mime`）。
  别把压缩挪进 `serve_file` 让所有调用点都吃到。细节见 `src/api/AGENTS.md`。
  ⚠️ 前端 `webui/src/api/stickers.js::UPLOAD_MAX_BYTES` 与 Rust 的是同一个数。
- `src/auth.rs`：`require_user` / `require_admin` / `require_password_changed`（**服务端强制**）/ `resolve_client_ip` /
  `trust_proxy` / `tls_terminated`；登录失败计数表（`auth.rs::rate_fail`）必须按 `MAX_LOGIN_KEYS`(4096) 清理。
  `--trust-proxy` / 端口 26320 / `--allow-remote` 的全局规矩见根卡。
  注册接口（`POST /api/auth/register`）的三重条件、失败一律 **403**（不泄露是哪一条不过）、
  `register|<ip>` 独立限流、账本 v5 邀请码**明文落库可重复查看** —— 正文在
  `crates/yekitrim-service/AGENTS.md`，本卡不重复搬运。
- `src/ratelimit.rs`：全站按客户端 IP 的通用额度 + 入库 webhook 专用额度；挂在 `api::router`**最外层、访问日志之外**。
  `ingest.rs::token_shape_ok` 的形状检查在 scrypt **之前**快速否掉垃圾请求；形状不过**不写 audit**。
- `src/access_log.rs`：最外层 `from_fn`，**每次前端交互都留痕**（≥400 时额外读响应体取 `error` 文案）。**别退回
  "只打成功"或"只打错误"**（`err(...)` 与内联 `{"ok":false,...}` 从不自己打日志，前端又有大量 `.catch(() => [])`）。
  **不记录请求体**；必须 `redact_path` 掉 webhook 的 token 段（它是**路径段**）。高频接口成功路径降级为 `debug`：
  `/api/media/*`、`/api/health`、`/api/stream`、`GET /api/jobs`、`/api/admin/logs/events`。

**多用户服务进度**：P1～P4 均 ✅ 已落地（设计稿 `docs/superpowers/specs/2026-09-12-multi-user-server-design.md`）；
**P5 容量与运维 ⏳ 待做**。

> 改动只允许落在本目录；跨模块改动先读对方卡，并跑两条门禁。

## 验证

```bash
cargo test -p yekitrim-server
```

HTTP 侧单测：JobManager / Range / 路由 404 / 配置 patch。
