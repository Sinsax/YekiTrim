# yekitrim-service — CLI 与 WebUI 共用的领域逻辑（账号账本 / 项目 / 录播姬 / 用量）

> 范围：`crates/yekitrim-service`/** ｜ 上游：`yekitrim-server`（HTTP 宿主）、`yekitrim-cli`（入口装配）｜ 验证：`cargo test -p yekitrim-service`

## 职责

独立 crate（原 `yekitrim-cli/src/service/`）：把 **CLI 与 WebUI 共用**的领域逻辑收在一处，避免两套流程漂移。
分层是**单向**的：`yekitrim-core` / `yekitrim-polish` → **本 crate** → `yekitrim-server` → `yekitrim-cli`。

覆盖面：账号账本与口令 KDF（`db` / `auth` / `user` / `invite`）、项目托管路径（`projects`）、
采集与入库的共享件（`ingest`，见 `src/ingest/AGENTS.md`）、B站通道（`bilibili` / `downloader` / `net`，各有卡）、
LLM 用量账本（`usage`）、运行日志总线（`logbus`）、单实例锁（`instancelock`）、
安全与磁盘摘要（`doctor` / `disk`）、工程快照与管线编排
（`workspace` / `core_pipeline` / `status` / `correct` / `silence` / `init_config` / `recorder`）。

`src/lib.rs` 是模块表的权威：**改 pub 面之前先看它**。

## 入口

- **模块表**（`src/lib.rs`）：私有 `core_pipeline` / `correct` / `silence` / `status` / `init_config` / `workspace`；
  显式 `pub` 的 `assets` / `auth` / `bilibili` / `db` / `disk` / `doctor` / `downloader` / `ingest` /
  `instancelock` / `invite` / `logbus` / `net` / `projects` / `recorder` / `usage` / `user`；
  私有模块再用 `pub use` 转发（`core_pipeline::*` / `correct::*` / `silence::*` / `status::*` /
  `init_config::*` / `workspace::*` / `doctor::*` / `user::*`）。
- **工程与管线**：`Workspace::{load, load_config, resolve, rel, output_dir, transcript_path, danmaku_path, current_stage}`；
  `core_pipeline::{run_stages, run_stages_with, run_stages_with_ext, init_polish_for_clip, init_polish_for_events}`
  （按 `-i`/`-o`/`-t`/`-e` 覆盖参数装配，CLI 与 `yekitrim-server` 共用同一份）；
  `status::project_snapshot`；`correct::run_correct`；`silence::run_silence`。
- **产物落盘（唯一通道）**：`assets::bind_asr_artifact` / `bind_derived_material`；
  读侧 `assets::{AssetKind, asset_status, list_items, scan_candidates, set_asset, bind_material, remove_material,
  slot_bound, preload_policy}`。
- **账本**：`db::{Db, SCHEMA_VERSION, ledger_dir, db_path, now_secs}`；`user::cmd_*`；`invite::cmd_*`；
  保留期清理：`Db::purge_old_audit(days)` / `Db::purge_old_llm_usage(days)`（只删**超过保留期**的行，
  `0` = 关闭清理 = 一条都不删；由 `yekitrim-server` 的每日维护调用，本 crate 不自己起定时器）；
  `auth::{hash_password, verify_password, validate_password, generate_initial_password, new_token,
  new_invite_code, MIN_PASSWORD_LEN, SESSION_TTL_SECS, COOKIE_SID, COOKIE_CSRF, CSRF_HEADER}`。
- **用量**：`usage::{DbSink, install, try_install_cli_sink, summarize, cmd_usage}`。
- **诊断**：`doctor::{DEFAULT_PORT, DEFAULT_URL, cmd_doctor, probe_health}`；`disk::{space_bytes, writable, human_bytes}`；
  `instancelock::{InstanceLock, lock_path, read_lock, process_alive, LockInfo}`。
- **日志**：`logbus::{bus, LogBus, LogBusLayer, enable_file_log, file_log_path, prune_old_logs, snapshot, subscribe}`。
  `prune_old_logs(dir, days)` 是**按天日志的保留期清理**（判据 = **文件名里的日期**，保留最近
  `days` 天含今天，`0` = 关闭）；`render-*.log` **不归它管**（`render_proc::prune_logs` 按份数留 20 份）。
- **采集**：`ingest::{cmd_token, cmd_status, generate_token}`；`recorder::{RecorderBackend, RecorderClient,
  BiliLiveToolsClient, RecorderRoom, room_id_from_dir_name, room_id_from_rel_path,
  RootStatus, RootProbe, probe_recorder_root, classify_root_shape, is_network_root}`
  （后五个是**录像目录校验的唯一判据**：跨机时用户自己映射盘，YekiTrim 只校验「能不能读/写」；
  判「Windows 上填了对方的 `/rec`」「盘符映射」「只读共享」都在这里，别在别处再写 `is_dir()`）。（`RecorderBackend` 按
  `[ingest.recorder].kind` 选后端：录播姬 `/api/room` + Basic；biliLive-tools `/recorder/list` +
  `Authorization: <passkey>`）。
- **配置**：`init_config::{init_project, init_global_config, ensure_global_config, validate_toml, show_config,
  download_models}`。⚠️ 程序配置只有一个位置 = `config::global_config_path()`（`users/<uid>/`，
  2026-11-13 收敛；顶层那两份已退役）—— 别在别处拼 `data_dir()/users/<uid>/config.toml`。CLI 侧 `yekitrim user *` / `yekitrim usage` / `yekitrim doctor` / `yekitrim ingest *` 的实现都在本 crate。

## 边界

- ⚠️ **不变式：本 crate 绝不能 `use yekitrim_server::*`。** 这是它能被抽出来的全部理由 ——
  一旦出现反向边，两层就又粘回去，`yekitrim-server` 也没法再独立。
- ⚠️ 本 crate **不依赖 axum**（`recorder.rs` 的测试除外，那是 dev-dependency）：
  它是纯应用层，HTTP 与否由 `yekitrim-server` 决定。
- `yekitrim-server` 只依赖 `yekitrim-service`，不碰 `yekitrim_cli`；CLI 侧要用服务端能力，只能经
  `yekitrim-server` 暴露的 `run()` / `ServerArgs` 进入。以上三条都已是机器门禁（`python3 scripts/check_module_boundaries.py` 的
  `service 不许认识 server` / `server 不许认识 cli` / `cli 只用 server 的入口`）。
- ⚠️ **默认端口（26320）的唯一一份定义**在 `doctor.rs::DEFAULT_PORT`：`ServerArgs.port`（clap 默认值）、
  `DEFAULT_URL`（`yekitrim doctor --url` 默认 + `yekitrim ingest token` 打印的 webhook 地址）都引用它。
  **别在任何地方再写端口字面量**（前端 webhook 地址用 `location.origin`，不用管）。
  26320 落在跨平台安全带 15001–32767 内，躲开 Windows 保留段（老默认 7788 掉在 7635–7834 里 →
  `10013 WSAEACCES` 而 `netstat` 里一个监听者都没有）与 Linux 动态段。
  ⚠️ 两个候选段**都会动态重排**，`mod.rs::first_bindable_port` 绑失败时实测试绑再报；
  `doctor` 有单测锁住 `DEFAULT_URL` 与 `DEFAULT_PORT` 一致（改一个忘另一个 = `yekitrim doctor` 探测不到自己）。
- ⚠️ **产物落盘 = 一次绑定**：登记清单与生效值这件事**只有一条路** ——
  `assets::bind_asr_artifact`（管线经 `core_pipeline::save_transcript_after_asr` 调它；LLM 订正那份走
  `bind_derived_material`，订正稿是**普通条目**、不占 ASR 槽）。绑不上（产物没落盘等）才退回裸写
  `transcript_file`。**别再在别处手写 `transcript_file`** —— 漏登记 = 那个模式的槽明明占了却看不见，
  用户就少一次转写机会。
- ⚠️ **剪切收尾 = 补精修工程 + 预热响度曲线**（用户口径：剪切这个行为本身包含精修 init）：`run_stage("cut")` 与
  `run_process` 的 Stage 4 之后都调 `core_pipeline::init_polish_for_events`，`yekitrim-server` 的单条剪切 /
  一键全切（`run_single_cut_event`）走同一条路。实现只此一份 `init_polish_for_clip`：
  **有 `polish.toml` 或没有 `clip.mp4` 就返回 `Ok(false)`**（`cmd_init` 会 `create_dir_all`，不能拿错目录建工程）、
  失败只 warn（剪切已经成功，进精修时还有 `ensure_polish_project` 兜底）。别再退回"等前端进页面才 init"
  —— 那样慢盘上会先撞 `polish.toml not found` 的竞态。
  同一条收尾里的 `warm_loudness_cache` 把 `clip.mp4` 的整段响度曲线量进 `<片段>/data/loudness.json`
  （精修页 `preview=1` 带的 `loudness_curve` 就是它）：**只补缺失** —— key（文件名+mtime+字节数）命中
  就零成本跳过、**一个字节都不动**，旧格式（无 `key`，永远 miss）与没缓存的才当场补一次；
  重剪后 mtime 变了自然按新片段重算。⚠️ 它挂在**共用的**收尾路径上（CLI `cut`/`process` 一样预热），
  **别再开第二条只服务 webui 的路径**；失败不影响剪切结果。
- ⚠️ **邀请码明文直接落库**（用户要求：管理员要能重复查看、复制再发 —— 与 `yekitrim ingest token`"只见一次"
  的规矩**不同**）：库里没有 selector/哈希那一套，注册校验就是一次明文查表。
  **别再退回"只存哈希"**，那会让管理员看不到码。**一个码只能绑一个账号**：消费走
  `Db::register_with_invite`（同一把锁里 `UPDATE ... WHERE used_at IS NULL AND revoked=0 AND 未过期` 再 INSERT），
  失败不会留下半个账号、也不会烧掉码。注册接口失败一律 **403**（不区分"码不对/码用过"，防止被拿来枚举），
  限流按 IP（key `register|<ip>`，与登录分开），5 次锁 15 分钟。**生成时自动打开注册开关**。
- ⚠️ 账号账本（`db` / `auth` / `user` / `invite`）**同时被 `yekitrim-server`（HTTP 会话、启动预检）与 `user` CLI
  使用，因此显式 pub，不再层层转发**；改动时两边一起想。
- ⚠️ **`logbus` 放在 service 而不是 server**：`LogBusLayer` 是 CLI 唯一需要 server 侧东西的地方
  （`main.rs` 的 subscriber 上挂它）。它若留在 `server/`，CLI 就会反向 `use` 服务端，
  `service → server` 的单向依赖就断了。它本身不碰 HTTP，只依赖 `db`。
  纪律：日志层自身**绝不 panic、绝不阻塞调用方** —— 任何 IO 失败静默忽略。
- ⚠️ **路径级禁则（不许开子进程 / 不许认识对方）只适用 `downloader/` 与 `bilibili/`**：
  `yekitrim-service` 整体不适用 —— `disk.rs` 调 `df`、`instancelock.rs` 调 `tasklist`/`kill`
  都是既有且合理的。
- `instancelock` 判定规则：拿到独占句柄 + 记录的 pid 仍存活 + 未 `--force` → 拒绝启动；
  拿到句柄 + pid 已不存在 → 崩溃残留，自动接管并 warn；**拿不到独占句柄 → 已有实例正在运行 →
  拒绝启动（`--force` 也不接管，因为对方真的在跑）**。
- `doctor` 是**只读诊断**：**绝不创建**账号库、不改任何配置；库里没有账号就如实说"尚未初始化"。
  建议区里"仍在用初始密码"这类判断直接读本机账本，不走未认证的 `/api/health`。
- ⚠️ **账本迁移前先备份**（`migrate()`：`current > 0` 时 `VACUUM INTO` 拿一致快照，失败退回文件拷贝），
  当前 `SCHEMA_VERSION = 8`：v7 = `llm_usage`（每次调用一行），v8 = `ingest_sessions` 加 `kind` / `source_ref`。
- ⚠️ **`patch_config_file` 支持两层以上（递归深合并）**，别再退回"一层 section → key"（会把整张子表静默丢掉）。
  细节与回归钉子在 `crates/yekitrim-core/src/config/AGENTS.md`。
- 归属由 SQLite `projects` 表决定（`projects::{projects_root, tenant_root, resolve_target_dir, new_project_dir,
  refresh_bytes, refresh_all_bytes, find_owned, sanitize_segment, parse_project_id, summary_json}`）；
  「移除项目」的语义与 `persist::forget_project` 那条坑见 `crates/yekitrim-server/AGENTS.md`。

### 转写 / 弹幕的解析基准（"选了不生效"与"静默丢改动"的根因，改读取方之前先读）

- ⚠️ **转写解析只有一个入口**：`Workspace::transcript_path()`（`transcript_file` → `output/transcript.json`
  → `output/video.srt`）与 `Workspace::danmaku_path()`（`danmaku_file` → 视频同目录 `danmu.ass`/
  `danmaku.ass`）。**一律相对工程根**。以前 `data_transcript_impl` 用裸 `PathBuf::from(...).exists()`
  （相对**进程 CWD**）而 `transcript_starts` 用 `ws.resolve`，靠 `output/` 兜底才没暴露 ——
  指向外部字幕时就是"选了不生效"。新增读取方走这两个方法，别再各写一份。
- ⚠️ **detect 阶段必须认 `transcript_file`**（`core_pipeline::run_stage("detect")` 的取字幕顺序：
  `-t` 覆盖 → `transcript_path` → 报错）。只认 `output/` 的话"选了外部字幕后重新检测"读到的还是
  旧的那份。`run_process` 的 recovery 分支与它共用同一个方法。
- ⚠️ **字幕保存写回"实际读到的那一份"**（`data_transcript_put_impl`）：目标是 `transcript_path`，
  再同步**已存在**的 `output/transcript.json` / `output/video.srt`。只写 `output/` 的话，
  选了外部字幕再编辑就是**静默丢失**（下次读的还是原文件）。`output/` 两份都不存在、
  又没有 `transcript_file` 时才 404。
- ⚠️ **`yekitrim-core` 里两个基准问题一并修了**：`ProjectConfig::current_stage` 与
  `runner::inject_config` 的 `*_file` 存在性/相对解析都改成先按 `output_dir.parent()`（= 工程根）
  —— 前者让"只上传字幕、从不跑 ASR"的工程能被判成 stage 2（否则重跑/门槛判断全跟着错），
  后者让"视频在 `videos/` 子目录、弹幕在工程根"这种布局不再静默丢掉弹幕（仍有视频父目录兜底）。
  `Path::new("").join(x) == x`，所以 CLI 的 `./output` 相对口径行为不变。
  两条都有回归测试（`current_stage_tests` / `runner::tests::danmaku_in_project_root_resolves_when_video_is_nested`）。
- ⚠️ **按来源类型预载**（`assets::preload_policy`）：`recorder` → 弹幕（入库时 `find_danmaku` 已搬入并写进
  `yekitrim.toml`）；`local`（服务器文件）→ **不预载**，只列候选让用户点 —— 为此 `create_local_project` 改调
  `write_project_config_exact`（不做"同目录同名 .ass 自动发现"，那条是 `yekitrim init` 的语义，两条路要分开）；
  `bilibili` → **弹幕**（价值就在弹幕，`popularity` 排序靠它）。
  ⚠️ `Preload::Subtitle` 这个变体**当前没有任何来源映射到它** —— "B站预载字幕"是错的（前几版文档这么写过），
  别改回去。
- 顺带修掉：LLM 订正取弹幕用的是 ASS-only 的 `load_danmaku`，选中的 `.xml`/`.srt` 弹幕等于没有 ——
  改走 `load_danmaku_auto` + 共享的 `ws.danmaku_path`。

### LLM 用量记账（token / 缓存命中 / 耗时）

- **采集点只有一个**：`LlmClient::send`（`yekitrim-core/src/llm/mod.rs`）。成功与**最终失败**都记一条
  `UsageRecord`（`llm/usage.rs`），重试算同一次调用（`attempts` 记实际尝试次数，**不按尝试拆行**）。
  要加字段就改这一处 + `usage.rs`，**别去 9 个调用点各写一遍**。
- **归因**：`label`（链路名）+ `project`（项目/切片目录）。Agent 系任务走 `AgentTask::usage_context()`
  （默认取 `name()`）；非 Agent 的构造点（精修分段/关键词、贴纸挑选、贴纸包标签、封面文案、LLM 测试）
  显式 `.with_usage_context(...)`。**新增链路记得挂 context**，否则统计里是空 label。
- **落地**：core 只 `emit`，写库由 CLI/server 侧装的 sink 负责 —— `usage::DbSink`（进程级默认 sink，
  `usage::install`；`LlmClient::with_usage_sink` 是测试用的私有注入）。安装点两处：`server::run`
  （`AuthService::open` 之后）与 CLI 的 `process/detect/correct/polish` 分支（`usage::try_install_cli_sink`）。
  写库失败只 warn，**绝不影响调用结果**。
- **表**：账本 v7 `llm_usage`。`cached_tokens` 认三种写法：DeepSeek `prompt_cache_hit_tokens`、
  OpenAI `prompt_tokens_details.cached_tokens`、缺省 0 ——**前缀缓存省了多少，看这一列**。
  `latency_ms` 是从第一次尝试到拿到结果的总墙钟（含退避等待）。
- **读出来**：`GET /api/admin/usage?days=N`（**仅 admin**，记录含他人项目路径，与 `/api/admin/logs` 同一个理由）
  → 总量 + 按天/链路/项目/模型分组；CLI `yekitrim usage [--days N]`。聚合全走 SQL `GROUP BY`，不在 Rust 里扫表。
  **缓存率 = Σcached ÷ Σprompt**（只算输入侧、先求和再相除，不是每次调用比率的平均；分母不掺 completion
  —— 缓存只作用于输入）。`prompt_tokens == 0` 显示 `—`（老中转没报 usage / 失败行 token 全 0，不是"没命中"）。
  实现只此一份：`usage::cache_rate`（CLI 侧，私有），前端 `LlmUsage.vue` 的 `cacheRate` 是镜像。
- 每次调用还会打一行结构化日志 `LLM 调用 label=… prompt_tokens=… cached_tokens=… latency_ms=…`
  （logbus 渲染成 `k=v` 进设置页「运行日志」与文件日志；失败是 `LLM 调用失败` + status）。

> 改动只允许落在本目录；跨模块改动先读对方卡，并跑两条门禁。

## 验证

```bash
cargo test -p yekitrim-service
```

领域逻辑单测：db / 项目 / 容量 / 下载器适配层 / B站接口。改了分层方向或 P4 隔离规矩，再跑
`python3 scripts/check_module_boundaries.py`。
