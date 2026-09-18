# 服务方向评审：精简为单人 WebUI / 综合型服务端 / 拆分独立

> 2026-09-18。**只读勘察，未改任何代码**。本文所有行数、引用计数、`文件:行号` 均为实测，可采信不必重查。
> 勘察对象是「接下来该走哪个方向」，结论是**先拆接缝、再堆功能**。

## 0. 结论

| 候选路线 | 裁定 | 一句话理由 |
|---|---|---|
| 精简为单人 WebUI | ❌ 否决 | 单人**不是另一条代码路径**，而是多用户能力的子集；切掉外层省不到什么，反而要改写内层 |
| 综合型服务端 | ✅ 方向 | 但它已落地约 80%，**不能继续直接堆** —— `api.rs` 已 7 492 行，应先拆再堆 |
| 拆成两个独立交付物 | ❌ 否决 | 单人已是多人的子集，拆产品等于把子集复制一份，数据模型与前端都要维护两套 |

**下一步（S1–S5）**：先把 `server/` 抽成独立 crate，再把 `api.rs` 按领域分模块，**然后**才做 ROADMAP 的 P4/P5。每步都能单独交付与回滚。

**顺带发现一处文档债**：ROADMAP 把「多用户服务」列为**远期目标（暂不排期）**，但 `AGENTS.md:1716` 的进度表显示 **P1 / P2 / P3 + 邀请码自助注册已全部落地 ✅**，只剩 P4（B站链接）/ P5（容量与运维）。也就是说它不是"未来"，是**已建成 80%**。本文档不改 ROADMAP，留作独立小改动。

---

## 1. 规模清点（决定成本）

| 单元 | 行数 | 说明 |
|---|---|---|
| **crates/vtrim-cli** | **22 391** | 下面三块之和 |
| ├ 入口（`main.rs`/`cli.rs`/`progress.rs`） | 838 | clap 30+ 子命令 |
| ├ `service/` 共用应用层 | **5 924** | db 1 743、assets 569、recorder 384、projects 341、core_pipeline 335、user 321、instancelock 277、workspace 272、invite 251、usage 248、auth 168、init_config 153、disk 116、ingest 106、correct 101、silence 93、status 89 |
| └ `server/` HTTP 层 | **15 629** | **占 70%**。api.rs 7 492（含单测 2 288）、jobs 1 411、ingest 1 334、ingest_worker 1 038、auth 848、scope 668、mod 633、stickers 432、logbus 384、render_proc 231、ratelimit 210、media 196、scope_key 193、persist 165、web 158、access_log 150、model 86 |
| crates/vtrim-polish | 15 218 | 精修 + 渲染 |
| crates/vtrim-core | 12 051 | 管线 + ASR / detect / cut |
| crates/vtrim-render-core | 5 015 | GPU 绘制原语 |
| crates/vtrim-wasm | 349 | **不在 workspace members 里**（有独立 target） |
| webui/src | 16 085 | TimelineEditor 1 610、CoverDesigner 1 329、ProjectClipsSection 1 261、PolishView 1 137、PrepareView 718 |

两个直接结论：

1. **`vtrim-cli` 有 70% 是服务端**。crate 名与实际内容严重不符，"CLI 里塞着一个服务端"是当前最大的结构问题。
2. `server/` 已是 `vtrim-cli` 里最大的单一子系统，且 `api.rs` 单文件 7 492 行 —— 其中 2 288 行（30%）是单测。

---

## 2. 现状：服务流程（本次 review 的对象）

### 2.1 启动序列（`main.rs:316` → `server::run` @ `server/mod.rs:180`）

1. 解析 addr、判 loopback、`auth::set_trust_proxy`（`mod.rs:181-190`）
2. `prepare_data_dir`：data_dir 解析 + **可写探针**（`mod.rs:193-198`、`:135`）
3. `logbus::enable_file_log`（`mod.rs:201`）
4. 初始密码 `explicit_admin_password`（`mod.rs:203`、`:157`）
5. `AuthService::open` → `Db::open(<data_dir>/vtrim.db)` + `migrate`（`mod.rs:204`；`auth.rs:130`；`db.rs:207`、`:236`）
6. `usage::install`（`mod.rs:212`）→ 7. `ensure_admin_bootstrap`（`mod.rs:213`）
8. **非 loopback 的两道硬门槛**：必须 `--allow-remote` 且不得仍用初始密码（`mod.rs:230-251`）
9. `InstanceLock::acquire`（`mod.rs:254`）
10. `import_legacy_projects`（`mod.rs:262`）、`ensure_global_config`（`mod.rs:271`）
11. `Workspace::load` + persist（`mod.rs:286-320`）
12. `AppState::with_auth`（`mod.rs:322`）→ 13. 清扫遗留渲染进程（`mod.rs:326`）
14. `ingest_worker::start` + `jobs::spawn_housekeeping`（`mod.rs:332`、`:336`）
15. `api::router` + `nest("/api")` + `webui_fallback`（`mod.rs:338-341`）
16. `TcpListener::bind` + `serve(ConnectInfo)`（`mod.rs:343-408`）

### 2.2 三条链路

**A. 服务器文件建项目**（**不进队列**）
`WorkbenchView.vue:385` → `stores/project.js:61` `POST /api/projects`（路由 `api.rs:141`）→ `projects_create`（`api.rs:1399`，非 admin 403 @ `1421`）→ `create_local_project`（`api.rs:1441`）→ `db.create_project`（`db.rs:599`）

**B. 跑转写**（主力）
`PrepareView.vue:573-583` → `projectData.js:68` `POST /api/project-data/pipeline?project=<pid>`（路由 `api.rs:158`）→ `project_data_pipeline`（`api.rs:2011`，已有转写则 409 @ `2025-2033`）→ `run_pipeline_job`（`api.rs:3065`）→ `jobs.create`（`:3087`，Heavy）→ `spawn_job`（`:3095`）→ `core::run_stages`

**C. 录播姬旁路**（P3 已落地）
`POST /api/rooms/:room_id/recordings/import`（路由 `api.rs:168-171`）→ `ingest::import_recording`（`ingest.rs:521`）→ `db.assign_ingest_session` + `set_ingest_state("closing")`（`ingest.rs:598-601`）→ `ingest_worker::notify`（`ingest.rs:607`）→ worker tick / 60s 兜底扫描（`ingest_worker.rs:93-131`）→ `process_one`（`ingest_worker.rs:450`）→ `ingest_into`（`ingest_worker.rs:739`，搬文件 + `db.create_project` @ `828`）→ `spawn_pipeline`（`ingest_worker.rs:871`）→ `jobs.create(owner_id, …)`（`:887`）→ `spawn_job`（`:900`）→ `core::run_stages`（`api.rs:5095`）

> 注意 A 与 B/C 的分界：**只有真正干活的才进队列**，纯元数据操作只有一次 SQL。

### 2.3 进度回流

`WebProgress`（`api.rs:5126`）→ `jobs.progress` → broadcast；前端 SSE `/api/stream`（`api.rs:176`、`:3239`；`useJobs.js:98`）+ **3s 轮询兜底** `/api/jobs`（`useJobs.js:96`）；单任务用 `watchJob` SSE `/api/jobs/:id/events`（`jobs.js:241`、`api.rs:179`）。

**job 状态只在内存、不落库**（`jobs.rs:18-19`、`:520`）—— 重启即丢，这是有意设计（见 `docs/job-queue.md`）。

### 2.4 `api.rs` 的职责块（7 492 行，供 S4 分模块用）

| 行区间 | 块 |
|---|---|
| 34-208 | 路由 + 中间件装配 |
| 210-443 | 通用工具（err / blocking / `spawn_job` / 探测） |
| 444-667 | 认证 + doctor + admin_users |
| 668-766 | 邀请码 |
| 767-849 | 运行日志 |
| 850-1077 | 直播间 / 录播姬 |
| 1078-1322 | 配置 / 模型 / LLM |
| 1323-1843 | 项目托管 |
| 1844-3062 | 项目数据（含 `fs_list` / `media_get`） |
| 3063-3368 | Pipeline + jobs + SSE |
| 3369-3695 | 渲染子进程 |
| 3696-5084 | Polish（含封面标题纯函数 3869-4154） |
| 5087-5203 | core / 进度桥 |
| **5204-7492** | **单测（约 2 288 行）** |

### 2.5 一处双路渲染（判断"服务端该多瘦"时的关键事实）

- `vtrim-wasm` **不依赖 `vtrim-cli`/`server`**，只依赖 `vtrim-render-core`（`crates/vtrim-wasm/Cargo.toml:15-17`）
- `PolishPreview.vue:80` 的 `tryEnableWasm()`（`:549-637`）在 WebGPU 可用时**于浏览器内合成**，并支持字幕热更新（`:688-695`）
- 服务端另有等价能力：`/api/polish/render-frame`（`api.rs:122`、`:4711`）与 `polish render` 子进程（`api.rs:3597`）

→ **"预览合成"可完全不经服务端**，但**取项目数据与跑管线仍必须经过服务端**。即服务端的角色在向「数据 + 队列」收缩，这是支持"服务端应更瘦、边界应更清楚"的一条独立证据。

---

## 3. 三条候选路线与代价

### 3.1 精简为单人 WebUI —— ❌ 否决

**前提不成立**：单人不是"另一条路径"，而是多用户能力的**子集**。这是设计稿里已经写死的决策，不是我的推断：

- `docs/superpowers/specs/2026-09-12-multi-user-server-design.md` §3.5「部署即生成管理员账号：**单人/多人同一套认证逻辑**」
- 同稿 §3.5.1 专门有一节「**为什么不再有免登录**」（决策记录）
- 同稿 §3.5.3「**单人 / 多人只差这三件事**」
- `AGENTS.md:1320`、`README.md:262`：单人部署＝只有一个 admin 账号的部署，**本机也要登录**

**真正只有多人用得到的只有 4 项**：邀请码自助注册、IP 限流、用量与配额、审计留痕。而下面这些**单人也要走同一套代码**：

| 能力 | 单人为什么也要 |
|---|---|
| 登录会话 | 设计稿 §3.5.1 明确取消免登录分支 |
| 项目归属 + 路径囚笼 | `projects.owner_id` 与 `Scope` 提取器是**全部** project-data / polish 请求的访问前提（`scope.rs:67-90`）；单人只是恒等通过 |
| 录播姬入库 | P3 已落地，单人正是它的主力通道 |
| 三档任务队列 | `Job.owner_id` 必填并贯穿 summary / SSE / 取消（`jobs.rs:295`、`:898-924`） |
| WebUI 全部编辑功能 | 时间轴、封面设计、精修预览是**主界面**，不是多用户加出来的壳 |

**成本**：约 20 个文件（server ≥6、service ≥4、core 1 `tenant.rs`、前端约 8）。
**收益**：省下 4 项功能的代码量，**不获得任何新能力**。
**额外代价**：CLI 自己也依赖这套账本 —— `vtrim user` / `invite` / `ingest token` / `usage` 都是 CLI 子命令（`main.rs:266-297`、`:303-304`、`:313`），`service/usage::try_install_cli_sink` 在 CLI 路径也 install（`main.rs:169`、`:218`、`:369`、`:380`）。删掉多用户 = 同时删掉这 4 个 CLI 子命令族。

**结论**：收益/代价比差，且与已确认决策 + 已落地 P1–P3 正面冲突。

### 3.2 综合型服务端 —— ✅ 方向，但**不能直接继续堆**

事实是它已经做完了 80%：`AGENTS.md:1716-1726` 的进度表 —— P1（账号账本 + `vtrim user` CLI）、P1 step-2（HTTP 会话鉴权 + 前端 M1）、P1 step-3（`vtrim doctor` 安全摘要）、P2（项目托管与隔离）、P3（录播姬接入）**全部 ✅**；邀请码自助注册 ✅。只剩 P4（B站链接，设计中已标为"次要通道"）/ P5（容量与运维）。

**但**：`api.rs` 已达 7 492 行，装 11 个领域。P4 要新增一整条通道（前端已有 `M4 #/bilibili` 设计：`docs/plan/webui-multiuser-frontend.md` §4），P5 要加存储页与运维 —— **不加拆分会把 7 492 行推到 9 000+**，改动成本随文件大小非线性上升。

所以 3.2 的正确形态是：**先拆 §4 的接缝，再堆 P4/P5**。

### 3.3 拆成两个独立交付物 —— ❌ 否决

如果指的是"本地单人工具 + 多用户服务端各成一个产品"：单人已是多人的子集（§3.1），拆产品等于**把子集复制一份**，要维护两套数据模型（`projects/<pid>` vs `users/<uid>/projects/<pid>`）、两套前端路由与角色判断。

**但"分开"另有一层正确含义**：把 `server/` 从 `vtrim-cli` 里**拆成独立 crate**。这是纯结构重构，**无数据模型变更、无行为变更**，也不产生第二个产品 —— 见 §4/§5。本轮采纳的是这一层。

---

## 4. 接缝：为什么可以机械搬迁

### 4.1 依赖方向实测

| 方向 | 实测 |
|---|---|
| `server/` → `service/` | **重**：`service::auth` 31 次、`projects` 26、`db` 22、`recorder` 7、`invite` 4、`assets` 4、`validate_toml` 3、`ingest` 3，另有 `user`/`usage`/`run_stages`/`run_correct`/`project_summary`/`project_snapshot`/`instancelock`/`ensure_global_config`/`doctor` 各 1 |
| **`service/` → `server/`** | **0 处** ← 这是接缝干净的**决定性证据** |
| CLI → `server/` | **仅 1 处**：`main.rs:34` 用 `server::logbus::LogBusLayer`（logbus 384 行） |
| `server/` → `cli.rs` | 0 处（grep 到的只是 tracing target 字符串 `"vtrim_cli::http"`，不是代码依赖） |
| `server/` → `vtrim_core` / `vtrim_polish` | 147 / 71 次 |
| `vtrim-cli` 被谁依赖 | **无人依赖**（仅出现在 workspace members 与自己清单里） |

### 4.2 目标 crate 图

```
vtrim-core ─┐
vtrim-polish ┼─→ vtrim-service ──→ vtrim-server ──┐
            │   （现 service/）  （现 server/）    ├─→ vtrim-cli （只剩入口 838 行）
            └──────────────────────────────────────┘
```

- `logbus` 需**下沉**到 `vtrim-service`：它是 CLI 唯一渗入 `server/` 的点
- `vtrim-wasm` 走 `vtrim-render-core`，**不参与**本次拆分

### 4.3 为什么是"机械搬家"

- 依赖图是**单向 DAG**（§4.1），没有反向边 → 不需要打破循环
- `server/` 与 `service/` 全部是 `pub(crate)` 之上的普通模块，无 `include_*!` 宏、无 `rust_embed`（已 grep 确认）→ 没有编译期内嵌资源要改
- `webui/dist` 的定位靠 `CARGO_MANIFEST_DIR`，**同深度搬迁不会坏**（见 §5.1）

---

## 5. 搬迁的具体坑（逐条实测，必须处理）

### 5.1 ✅ `webui/dist` 路径 —— 同深度搬迁**不会坏**（但别嵌套更深）

`server/web.rs:20-24`：

```rust
let mut out = vec![PathBuf::from(env!("CARGO_MANIFEST_DIR"))
    .join("..").join("..").join("webui").join("dist")];
```

`CARGO_MANIFEST_DIR` 是 **crate 根**。现在 `crates/vtrim-cli` + `../..` = 仓库根 → `<root>/webui/dist` ✅。
新 crate 放 `crates/vtrim-server`：`crates/vtrim-server` + `../..` = **仓库根，同一个位置** ✅。

⚠️ 唯一失败模式：把新 crate 放得更深（如 `crates/server/vtrim-server`），`../..` 会指到 `crates/`，WebUI 静默 404。

### 5.2 ⚠️ `CARGO_PKG_VERSION` 的来源会静默改变（需版本同源）

三处：

| 位置 | 现状 | 搬迁后 |
|---|---|---|
| `server/api.rs:427` | `/api/health` 的 `"version"` 字段 | 变成 **vtrim-server 的**版本 |
| `service/doctor.rs:47` | `vtrim doctor` 打印的版本 | 变成 **vtrim-service 的**版本 |
| `xtask/src/main.rs:598-600` | 读 `crates/vtrim-cli/Cargo.toml` 的 `[package] version` 决定包名 `vtrim-v<版本>-win-x64` | **不变**（xtask 按路径读文件） |

当前全仓都是 `0.1.0`，所以今天看不出差别；一旦将来只改一个 crate 的版本，`vtrim.exe --version`（来自 api.rs）与发行包名（来自 xtask）就会不一致，而且**不会报错**。

→ **对策**：改用 workspace 继承版本，把版本钉成同源：

```toml
[workspace.package]
version = "0.1.0"
```

各 crate 写 `version.workspace = true`。这样三处永远一致。

### 5.3 ⚠️ `wgpu-render` feature 会**静默失效**（最危险的一条）

`cfg(feature = "wgpu-render")` 的 **8 处使用全在 `server/`**：`api.rs:433`、`:4710`、`:4812`；`model.rs:29`、`:33`、`:37`、`:44`、`:70`。

feature 随 crate 走。若把 `server/` 搬走而 `vtrim-cli/Cargo.toml` 仍写

```toml
wgpu-render = ["vtrim-polish/wgpu-render"]
```

那么 `cargo build --features wgpu-render` **不会报错**，但 GPU 后端不会被编译进新 crate —— 结果是 `/api/health` 的 `wgpu_render` 报 `false`、`VTEDIT_RENDERER=wgpu` 静默退回，而构建是成功的。

→ **对策**（S3 一并做）：新 crate 声明自己的 feature，`vtrim-cli` 改为转发

```toml
wgpu-render = ["vtrim-server/wgpu-render"]
```

并在验收时**显式比对** `cargo build --features wgpu-render` 后 `/api/health` 的 `wgpu_render` 为 `true`。

### 5.4 ℹ️ `service/` 对 axum 的引用**只在测试里**

`service/recorder.rs:272-311`（`#[tokio::test] fn client_talks_to_a_fake_recorder`）起了一个**假录播姬** HTTP 服务来测 `RecorderClient`。除此之外 `service/` 不碰 axum。

→ `vtrim-service` 把 `axum` + `tokio`（`macros`/`rt` 部分）放 `[dev-dependencies]` 即可，**主依赖不需要 axum**。这也是"service 是纯应用层"能成立的原因之一。

### 5.5 依赖清单**按 cargo 报错搬**，不要预先切分

要搬的候选：`axum`、`tokio`、`tokio-stream`、`rusqlite`、`scrypt`、`subtle`、`rand`、`reqwest`、`mime_guess`、`base64`、`futures-util`、`toml`。
但 `vtrim-cli/Cargo.toml` 里还有 CLI 自己用的（`clap`、`indicatif`、`tracing`、`anyhow`、`chrono`、`serde`…）。**预设清单一定会漏**（例如 `toml` 可能两边都用）→ 先按功能分组移动，让 `cargo check` 报出缺失，再逐个补。

### 5.6 `api.rs` 的单测 2 288 行

**S3 先原样搬**（整文件移动，零逻辑变更）；**S4 才拆模块**，此时单测跟着它测的函数走。理由：把"搬家"和"重构"混在一个提交里，会让 `git diff -M` 的 rename 检测失效，回滚也不再干净。

---

## 6. 实施顺序（每步可独立交付 / 回滚）

| 步 | 内容 | 规模 | 验收 |
|---|---|---|---|
| **S1** | `logbus`（384 行）从 `server/` 下沉到 `service/`，改 `main.rs:34` | 小 | `cargo check` 全绿；CLI 已无 `server::` 引用 |
| **S2** | 新增 `crates/vtrim-service`（＝现 `service/`），`vtrim-cli` 依赖它 | 5 924 行移动 | `cargo test` 全绿；`crate::service::` → `vtrim_service::` 全量替换 |
| **S3** | 新增 `crates/vtrim-server`（＝现 `server/`），依赖 `vtrim-service`；**同时做 §5.3 的 feature 转发** | 15 629 行移动 | 见 §7 全部验收项 |
| **S4** | `vtrim-server` 内把 `api.rs` 按 §2.4 的块拆成 `api/{mod,routes,auth,invite,logs,ingest,config,projects,project_data,pipeline,render,polish}.rs` | 纯搬 | 路由清单前后逐条相同 |
| **S5** | 之后才做 ROADMAP 的 P4（B站链接）/ P5（容量与运维） | —— | 按各自设计稿 |

S1 是刻意拆出来的**最小一步**：它单独就把「CLI → server」这条唯一的反向渗漏切断，代价极小，失败了也好退。

---

## 7. 验收（不可省）

- [ ] `cargo check --workspace --all-targets` **0 warning**（仓库基线就是 0，不能退化）
- [ ] `cargo test --workspace` 全绿；重点看 `api.rs` 的路由单测（`tower::oneshot`）与 §5.4 那个假录播姬测试
- [ ] **路由清单前后逐条 diff 相同**：把 `Router` 的 (method, path) 集合 dump 出来对比，这是"行为等价"最硬的证据
- [ ] CLI 子命令清单前后 diff 相同（`cli.rs` 只改了两行类型路径：`crate::service::doctor::DEFAULT_URL`
      → `vtrim_service::doctor::DEFAULT_URL`、`crate::server::ServerArgs` → `vtrim_server::ServerArgs`；
      clap 属性一字未动，所以 18 个子命令与 `--help` 输出无差异）
- [ ] `cargo build --features wgpu-render` 后 `/api/health` 的 `wgpu_render == true`（§5.3 的专项校验）
- [ ] `cargo xtask scan` 仍 PASS（产物门禁）
- [ ] `cargo xtask package --skip-build --smoke-test` 通过（`/api/health` + `/` 的 `<div id="app">`）
- [ ] `cargo tree -p vtrim-cli` 不再出现 `axum` / `rusqlite`（证明依赖确实搬走了，而不只是文件搬走了）

---

## 8. 风险与回滚

- **每步一个提交，不推送**，任一 S 步可单独 `git revert`
- **最大的风险是循环依赖**：`service/` 现在对 `server/` 是 0 引用，搬完后**必须复查一次**这条仍成立（`vtrim-service` 绝不能 `use vtrim_server`）
- 纯搬迁提交里 `git diff -M --stat` 应只显示 rename + 少量路径改写；若出现大段逻辑改动，说明"搬家"和"改逻辑"混在一起了，要拆开
- 若 S3 出现意料外耦合（例如 `server/` 用了 `cli.rs` 里的某个工具函数），**停在那一步**，把该工具下沉到 `vtrim-service` 再继续；**不要**为了搬而在 `vtrim-server` 里复制一份实现
- `api.rs` 分模块时若某块与相邻块共享私有 helper，先把它放进 `api/mod.rs`，不要跨模块 `pub` 暴露内部类型

---

## 9. 明确的非目标

- **不改数据模型**：`projects.owner_id`、`users/<uid>/…` 目录布局、`Job.owner_id`、11 张表全部保留
- **不改 API 契约、不改协议、不改前端**
- **不删任何多用户功能**
- **不顺带重构业务逻辑**（同 §5.6 的理由）
- **不拆成两个产品**（§3.3）

---

## 10. 本次决定（记录）

**2026-09-18 裁定**：

1. 方向定为 **先拆接缝、再堆功能**。依据：`service/` → `server/` 零反向引用，接缝天然存在且已验证；`api.rs` 7 492 行的规模已到"再加一整条通道就难维护"的程度。
2. **否决**「精简为单人 WebUI」：单人已是多人的子集，切不到外层只切到内层，且与设计稿 §3.5 已确认决策及已落地 P1–P3 冲突（§3.1）。
3. **否决**「拆成两个独立交付物」：等于复制子集、维护两套（§3.3）。
4. **采纳**「把 `server/` 拆成独立 crate」这一层含义的"分开独立"（§4、§6）。
5. 顺带记录一处**文档债**：ROADMAP 把多用户服务写为「远期目标（暂不排期）」，与 `AGENTS.md:1716` 进度表矛盾，需另做小改动修正措辞（本文档不改）。

---

## 11. 实施结果（2026-09-18 收尾）

S1–S4 全部落地，一步一个提交；S5（P4/P5）按设计稿另开。

| 步 | 提交 | 实测结果 |
|---|---|---|
| **S1** logbus 下沉 | `e630bd8` | CLI 已无 `server::` 引用 |
| **S2** 抽出 `vtrim-service` | `f1bc2ef` | 19 个模块 + `lib.rs` 逐文件 rename；`cargo tree -p vtrim-cli` 的直接依赖里不再有 axum |
| **S3** 抽出 `vtrim-server` | `bab861a` | 16 个模块 rename；**feature 转发**已做并通过 §5.3 专项校验（`--features wgpu-render` → `/api/health` 的 `wgpu_render == true`） |
| **S4** `api.rs` 拆模块 | `bf6535c` | 7 492 行 → 14 个文件，最大 1 718；**路由清单 110 条逐条相同** |

**§7 验收清单（逐条实测）**

- [x] `cargo check --workspace --all-targets` 0 warning
- [x] `cargo test --workspace` 全绿（466 passed / 0 failed / 15 ignored）
      （**注**：不是"拆分前后同数"——`865a4c3` 的基线是 465 passed / 487 个 `#[test]` fn，
      拆分后是 466 / 488。多出来的那 1 条是 **S1 新增**的
      `logbus::tests::short_target_strips_every_crate_prefix`（测 `vtrim_cli::` / `vtrim_server::`
      前缀都要剥掉），是净增不是丢失；测试函数总数 488 在 `bab861a` 与 `HEAD` 两次清点相同。）
- [x] 路由清单前后逐条 diff 相同：110 条 `(method, path, handler)` 完全一致 + **13 条** router
      指令的**多重集**逐项相同（`nest`/`layer`/`route_layer`/`merge`/`fallback`/`with_state` 各归各的计数。
      指令的**顺序**会变，那是预期的：清单按源文件顺序拼接，单体 `api.rs` 一份 vs 拆开后 14 份按名排序，
      顺序本来就不同 —— 判"行为等价"看的是路由集合与指令计数，不是指令的书写次序。)
- [x] CLI 子命令清单前后 diff 相同（本轮的比对口径见 §12.4 最后一条：只对**当前**二进制取了
      `--help`，旧侧是按 `cli.rs` 的 clap 属性逐条比对推出的，不是两份二进制字节级对拍）
- [x] `cargo build --features wgpu-render` 后 `/api/health` 的 `wgpu_render == true`
      （带**反向对照**：同一棵树的默认 feature 构建回 `false` —— 证明这个字段真在读 feature，
      不是恒真。feature 转发链在 `vtrim-cli/Cargo.toml`（`wgpu-render = ["vtrim-server/wgpu-render"]`）
      → `vtrim-server/Cargo.toml`（`["vtrim-polish/wgpu-render"]`）。）
- [x] `cargo xtask scan` PASS
- [x] `cargo xtask package --smoke-test` 通过：产品门禁 PASS + `/api/health ok, / served the packaged WebUI`
      ⚠️ **必须用不带 `--skip-build` 的完整打包**：`--skip-build` 会直接对 `target/release/vtrim.exe`
      跑门禁与冒烟，而拆分前的那个 release 产物（本机实测 mtime 12:28）比 S1–S4 的提交（15:30–17:11）**更早** ——
      这时"冒烟通过"证明的是**拆分前**的二进制，是个假阳性。完整打包会先重建 release（本机实测 4m40s）。
      `/` 返回 `200` + `<div id="app">` 同时验证了 §5.1 的 `CARGO_MANIFEST_DIR/../..` 深度假设：
      新 crate 同深度搬迁后 `webui/dist` 仍能被找到（路径错了会是 503 + 构建指引，不是 200）。
- [x] `cargo tree -p vtrim-cli --depth 1` 已无 axum / rusqlite
      （**注**：§7 原文写的是 `cargo tree -p vtrim-cli`，但那种写法会列出传递依赖 ——
      vtrim-cli → vtrim-server → axum 本来就该出现。口径按"直接依赖"理解。）

**与 §5 预先判断的偏差（都记在这里，避免下次又踩）**

1. §5.3 说的 feature 静默失效**确实存在**，而且修好之后立刻又暴露一处：
   `api.rs::polish_render_frame` 在 `wgpu-render` 下有一个从未被用的 import，
   旧布局里也一直在报 warning（默认 feature 不编译这段代码，所以基线 0 warning 掩盖了它）。
   搬完顺手删掉，现在两种 feature 组合都是 0 warning（`vtrim-render-core` / `vtrim-polish`
   在 `wgpu-render` 下另有 13 条历史 warning，不在本次范围）。
2. §5.6 说"单测跟着它测的函数走"——`api.rs` 的 2 288 行单测**大部分是接口级的**
   （`TestApp` 建整张路由表再发请求），不是纯函数单测。所以落地成：
   夹具单独一份 `api/test_support.rs`，用例按领域散进 12 个模块的 `#[cfg(test)] mod tests`。
   真正的纯函数单测（`sec_to_ts_value` / `parse_chunk_progress` / `prune_clip_from_index` …）
   就留在它被测的那个模块里。
3. **多了一处 §5 没点名的机械改写**：原 `api.rs` 里的 `super::` 指 crate 根，
   搬进 `api::foo` 之后 `super` 变成 `api`，所以 29 处 `crate::server::` 与 132 处
   `super::…` 都要改写成 `crate::…`。**注意 `api/auth.rs` 与 `crate::auth` 现在同名不同物。**
4. `scripts/route_manifest.py` 有两个一直没生效的地方（前一个提交只写了函数没调用）：
   `normalise_handler` 从未被调用（`super::/crate::` 前缀一直留在清单里），
   候选路径也只认 `api.rs`（拆成目录模块后直接报"找不到 api.rs"）。两处都已补齐。
5. **§5.2 的版本同源已在前置提交 `865a4c3` 做完**，本次没有再动版本。

**遗留（未做，和本次目标是两件事）**

- `access_log.rs` / `jobs.rs` 里的 tracing `target: "vtrim_cli::http"` / `"vtrim_cli::job"`
  字面量仍是旧 crate 名。它**不是代码依赖**（只是日志 target 字符串），改了会影响
  `RUST_LOG` 过滤与用户已保存的配置，所以**有意不动**；`logbus::short_target` 本来就会
  把 `vtrim_cli::` / `vtrim_server::` 前缀都剥掉，面板显示不受影响。
- `docs/web-ui-design.md` 等**设计稿/历史文档**里的 `crates/vtrim-cli/src/server/` 路径
  保持原样（那是当时的记录）。live 参考文档（`AGENTS.md` / `docs/job-queue.md` /
  `docs/deploy.md`）已全部更新到新路径。

---

## 12. 独立复验（2026-09-18，第二轮）

§11 是实施者自己的记录。这一轮是**不看结论、只认实测**地把 §7 重跑一遍，
用来甄别"哪几条是真做完的、哪几条只是写成了 `[x]`"。结论：**S1–S4 全部真实落地**，
但有 4 处**记录不准**，已就地修正（都在上面 §7/§11 的正文里）。

### 12.1 复验通过项（本次实测）

| 复验项 | 命令 / 方法 | 实测 |
|---|---|---|
| 结构 | `crates/` 目录 + `Cargo.toml` members | 6 个 crate 就位；`vtrim-cli/src/` 只剩 `main.rs`/`cli.rs`/`progress.rs` |
| 迁移完整性 | `git diff 865a4c3 HEAD -- crates/vtrim-cli/` | 39 文件、−21 620 行；新的 `api/` 是 14 个文件 |
| 0 warning | `cargo check --workspace --all-targets` | **0 条** `warning:` |
| 测试 | `cargo test --workspace` | 466 passed / 0 failed / 15 ignored，exit 0 |
| 测试未丢失 | 两次导出源码树数 `#[test]`/`#[tokio::test]` fn | `865a4c3`=487、`bab861a`=488、`HEAD`=488 |
| 路由等价 | `scripts/route_manifest.py` 对 `bf6535c^` 导出树 vs 当前树 | 两边 **110 条**路由逐条相同；13 条指令的多重集相同 |
| CLI 面 | `vtrim --help` + `cli.rs` 的 clap 属性逐条比对 | **18 个子命令**；`ServerArgs` 的 `#[arg]` 与旧 `server/mod.rs` 完全一致 |
| feature 转发 | 带/不带 `--features wgpu-render` 各起一次服务读 `/api/health` | `true` / `false`（反向对照成立） |
| WebUI 深度 | 起服务请求 `/` | `200` + `<div id="app">`（§5.1 的假设成立） |
| 产物门禁 | `cargo xtask scan` | PASS（espeak-ng 五个标记全 0） |
| 打包冒烟 | `cargo xtask package --smoke-test`（完整重建） | 门禁 PASS + `/api/health ok, / served the packaged WebUI`；产出 zip 14.9 MB |
| 依赖方向 | `cargo tree -p vtrim-cli --depth 1` | 直接依赖里**无** axum / rusqlite |
| 硬规矩 | grep `vtrim_service` | 无 `use vtrim_server` —— §8「最大的风险」复查通过 |
| 版本同源 | 起服务读 `/api/health` | `version: "0.1.0"` 正常（§5.2 的 `[workspace.package]` 生效） |

### 12.2 复验改掉的 4 处记录不准

1. **`12 条 router 指令` → 13 条**（§11）。实测 13 条，且**顺序**在拆分前后不同 ——
   顺序不同是预期的（清单按源文件拼接顺序生成），但 §11 原话"计数逐项相同"没说清"顺序会变"，
   容易被下一个人当成回归。
2. **`466 passed，拆分前后同数` → 不是同数**（§11）。拆分前（`865a4c3`）是 **465**，
   拆分后 466；多出的 1 条是 **S1 净增**的 `logbus::tests::short_target_strips_every_crate_prefix`。
   方向是"变多"不是"丢失"，但写成"同数"会掩盖"这条测试是本轮新增的"这个事实。
3. **`cli.rs 不动` → 动了 2 行**（§7）。两行都是类型路径改写（`crate::service::` → `vtrim_service::`、
   `crate::server::ServerArgs` → `vtrim_server::ServerArgs`），clap 属性一字未动，
   所以**结论（子命令清单无差异）仍然成立**，只是理由不能写成"文件没动"。
4. **`xtask package --skip-build --smoke-test` 这条口径不安全**（§7/§11）。
   `--skip-build` 复用的是 `target/release/vtrim.exe`，而拆分前那次 release 构建**早于** S1–S4 的提交；
   照 §7 原文跑，冒烟测的是**拆分前**的二进制 —— 通过也不说明拆分后的代码能打包能跑。
   本次改用完整 `cargo xtask package --smoke-test`（先重建 release）才有意义。

### 12.3 顺带修掉的文档错

- `AGENTS.md` 防线表第 ② 行把 `PROJECT_PATH_KEYS` 的位置写成 `vtrim-server/src/api::PROJECT_PATH_KEYS`
  ——`api::` 是模块路径、不是目录，按这个字符串找不到文件。实际在
  `vtrim-server/src/api/config.rs`（`pub(super) const`），已改。

### 12.4 复验**没有**覆盖的（如实列出）

- `--features wgpu-render` 下的 GPU **出片**只验了 `/api/health` 的开关字段与单帧预览接口可编译，
  **没有**实跑一次 `VTEDIT_RENDERER=wgpu` 的完整渲染出片。
- `vtrim-polish` / `vtrim-render-core` 在 `wgpu-render` 下的历史 warning 没清（§11 已声明不在本次范围）。
- §7 最后一条"CLI 子命令清单**前后** diff"里，"前"的一侧是用 `cli.rs` 的 clap 属性逐条比对推出的
  （两棵树的 `--help` 需要两份二进制才能字节级对拍，本次没为基线单独建一份 CLI）。
  属性集合相同 + 只改了类型路径 ⇒ 输出相同，这个推理是可靠的，但严格说不是"两份二进制对拍"。
