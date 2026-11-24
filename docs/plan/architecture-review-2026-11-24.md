# YekiTrim 架构评估（2026-11-24）

> 对象：当前工作树（`version = 0.5.0`，851 commits，2026-06-28 起）
> 方法：**只读代码 + 脚本核对**。本机沙箱没有 Rust 工具链，所以**没有编译、没有跑测试** ——
> 本文所有"在/不在"的判断基于源码与注释，行为结论未经运行验证。
> 与前作的关系：`architecture-optimization-2026-10-15.md` / `architecture-refactor-plan-2026-10-15.md`
> 的条目逐条回查，本文只写**仍然开放**的，并把新发现单独标出，不重复其论证。

---

## 0. 结论

1. **四件事已经做得比同类项目好，不该再动**：crate 分层、任务队列、路径安全、构建/产物一致性（§1）。
2. **真正欠的账只有两条**，其余都是它们的下游症状：
   - **有守卫、无执行者**：1598 个 `#[test]` + 82 个检查脚本，没有 CI、没有统一 runner。
   - **重复真相**：同一语义有 3~4 份实现，11 个 parity 脚本是这份"税"。
3. 有一批**当天可做**的快赢，其中 3 条已经在伤仓库（§4）。

---

## 1. 判定为健康、建议不动

避免"为重构而重构"，这些我逐条看过，是设计不是冗余：

| 项 | 证据 |
|---|---|
| **crate 分层** | `check_module_boundaries.py` 19 条 / 665 文件 / 0 违规；`service → server` 反向边 0；server 的 `use yekitrim_core::` 只有 4 处（core 是共享底座，不是违规） |
| **任务队列** | 资源键互斥 + 三档闸门 + 优先级 + 取消 + panic 兜底 + 心跳/停滞判据（`jobs.rs` 1.1k 行）；`spawn_job` 的**双兜底**（panic 与线程起不来）都写了注释说明踩过的坑 |
| **路径三防线** | `EXTRA_PATH_FIELDS`（2 个字段）+ `PROJECT_PATH_KEYS`（9 条）+ `ensure_within`；`/api/health` 已不吐任何路径（B6 已落地） |
| **构建/产物一致性** | release 内嵌 dist、debug 读盘；`webui/dist/.built-from` 指纹与 `vite.config.js`/`build.rs` 交叉校验（我重算了一遍，`4981d8496ef373d0` 与 dist 记录一致） |
| **数据库迁移** | `PRAGMA user_version` + 迁移前 `VACUUM INTO` 备份 + 失败降级拷贝，且失败**留痕**（不是 `let _ =`） |
| **SSE 终态** | `pipeline.rs` 的 `unfold` 写法明确写了为什么不能用 `take_while`/`scan`（B1 已落地） |
| **预览端口** | `PreviewPorts` LRU(16) + `shutdown_port` 真关监听（E6 已落地） |

**不要动的判断**（与 10-15 文一致，且我认为现在更成立）：三套配置 schema 分属不同生命周期；
`filelock` / `instancelock` / `scope_key` 三个锁各管一段；路径三防线是有意纵深，不是重复。

---

## 2. 主结构问题

### 2.1 有守卫、无执行者（10-15 的 S1 + S2，**仍开放**）

这是全仓最大的一笔结构性欠账，也是唯一"投入产出比确定为正"的一条。

证据：

```
scripts/ 检查脚本   : 51 × .mjs + 14 × .py = 65
webui/scripts/      : 17 × .mjs
合计                : 82 个
仓库根 CI 配置      : 无（无 .github / 无根 package.json / 无 Jenkinsfile）
统一 runner         : 无（scripts/gate.mjs 不存在；crates/xtask 只有 scan|package）
cargo 测试          : ≈1598 个 #[test]/#[tokio::test]（57 个 #[ignore]）
```

`scripts/route_manifest.py` 的文档开头就写着 **"Exit code is always 0 — comparing is the caller's job"**，
而全仓**没有任何调用方** —— 一个已经写好的能力，缺的只是"接上"。这与 S1 是同一种病：
**工具不缺，缺执行者。**

后果不是"可能出问题"，而是已经出过：`check-effect-registry.mjs` 的卡里记着
"主检查进了 gates、自检没进，烂了很久没人发现"。现在 82 个脚本里还有一批是"缺样本就 SKIP + exit 0"
（10-15 的 E4 指的就是这类），它们会在 CI 里安静地绿。

**建议（顺序不能反）**：
1. `scripts/gate.mjs`：按改动面选脚本，**任何"没跑"都要显式打印**（沿用 `yekitrim-compare.mjs`
   那条"漏了 `--golden` 静默换参照"的教训），守卫的 `--self-test` / `--mutate` 一起跑。
2. 把 `route_manifest.py` 接成"前端 `api/*.js` 的 `/api/...` 字面量 ↔ `routes.rs` 路由表"双向差集。
3. 最后才谈 CI。前置是 `vendor/sherpa-onnx-sys`（会下载并 sha256 校验）与 `webui/dist` 的离线可用性 ——
   不解决这两处，第一次 CI 必红。

### 2.2 重复真相（10-15 的"主线"，**仍开放，且是现在最大的维护成本**）

同一份特效语义存在于**四处**：

| 实现 | 规模 | 角色 |
|---|---|---|
| `crates/yekitrim-polish/templates/index.html` | 3,229 行 / 153 KB | 旧出片/预览模板（canvas `renderFrame`） |
| `webui/src/polish/engine.js` | 2,897 行 / 165 KB | 前端 DOM 近似预览（"忠实移植"） |
| `tools/polish-to-dhampir.mjs` | **7,945 行** | 出片唯一真实路径（转译成底座工程） |
| `yekitrim-render-core/src/timeline.rs` + `PolishPreview.vue` | — | 时间线数学 / 面板 |

代码注释自己写着"**四处实现必须同改**"（`timeline.rs:721`、`engine.js:2081`），全仓
"同源 / 同口径 / 逐式一致 / parity" 出现 326 次（Rust 178 + 前端 148）。51 个根 `check-*.mjs` 里
11 个是专职 parity（161 KB）。**这些脚本不是解法，是税**。

10-15 文把这条定为主线，但没有给出可执行的收敛路径。现在情况变了：
**出片已统一到 Dhampir 一条腿**（`backend.rs` 的 `RenderBackendKind` 只剩 `Dhampir`），
于是三条预览腿里：

- Dhampir（浏览器 wasm）—— 新主路；
- `engine.js` DOM 近似 —— 无 WebGPU 时的降级；
- `RenderServer` + `index.html` —— CLI `polish preview` 的兜底 + 服务端 `/polish/preview`、`cover-preview`。

**建议拍板一件事**：DOM 近似腿是否允许"语义接近、像素不追究"。
若允许（我建议允许），就能把 10 个像素级 parity 判据从"近似预览 vs 出片"缩到
"转译器 vs timeline 数学"，`index.html` 只保留封面与旧工程兼容 —— 维护面立刻少一半。
若不允许，就要承认"三套渲染必须永远逐像素对齐"，那 82 个脚本只会继续增加。

---

## 3. 次级结构问题

### 3.1 `api/` 是"伪拆分"（10-15 的 W2，**更大了**）

```
api/polish.rs        2,787 产线行 / 39 个 handler   （字体 9、贴纸 6、封面 8、预览 4、字幕 4、工程 19…）
api/project_data.rs  1,922 产线行 / 31 个 handler
```

两个文件**互相 glob 引用**：

```rust
// polish.rs:21
use super::{pipeline::*, project_data::*, render::*};
// project_data.rs:21
use super::{config::*, pipeline::*, polish::*, projects::*};
```

Rust 允许模块环，所以编译不报错、门禁也抓不到 —— 但这正是"按领域一文件一模块"失效的地方：
`polish.rs` 里的 `validated_project_dir` 来自 `project_data.rs`，`project_data.rs` 又要用
`polish.rs` 的辅助函数。新加一个 handler 时，"该放哪个文件"已经没有答案。

**建议**：按路由组把 `polish.rs` 至少切成 `polish/{project,files,assets,preview,cover}.rs`，
并**禁掉 glob 重导出**（改成显式 `use`）—— 显式之后环会立刻变成编译错误，拆分边界就有了判据。

⚠️ 这条会撞上一道**物理约束**：`api/` 现在**没有任何子目录**（一个目录一张卡），
而现存最坏链 `root + server + api` = 57,322 B，离 57,344 B 只剩 **22 B**。
一旦建 `src/api/<domain>/`，就按规矩要新开卡，而最坏链随即可超预算 ⇒ 门禁红。
所以**拆 api 之前必须先做 §4.3 的卡瘦身**，否则顺序做反会被门禁卡住。这一点值得写进计划本身。

### 3.2 HTTP 契约无类型（10-15 的 S2，**仍开放**）

```
注册路由           : 127 条（.route(）
handler 取 Json<Value> : 46 个
请求体 Deserialize 结构 : 7 个
前端类型/测试       : 0（36 个 .vue 全 JS，无 tsconfig、无 vitest）
```

请求体几乎全是裸 `serde_json::Value`，字段名拼错只在运行时变成 `null`/400。前端 38.7k 行
（59 `.vue` + 37 `.js`；`PolishPreview.vue` 4,337 / `TimelineEditor.vue` 3,924 / `PolishView.vue` 3,779）
没有任何单测，而这三个文件恰好是 churn 最高的（最近 400 个提交里分别动过 27 / 42 / 25 次）。

按你们自己的缺陷分类（`webui-typescript-assessment.md` §3）：A 类"类型/形状"28 条、
B 类"生命周期"31 条，并列最大。TS 只治 A 类的一部分，**所以顺序应该是"先给请求体定类型 + 加契约测试"，
不是先迁 TS**。最小动作：
- 每个 handler 的请求体换成 `#[derive(Deserialize)] struct`（46 处，可分批）；
- 加一条"路由表 × 未登录 401"的**枚举式**测试（见 §4），新路由自动被覆盖。

### 3.3 进程级隐式状态（10-15 的 C4，**仍在**）

```rust
// crates/yekitrim-llm/src/usage.rs:70
static DEFAULT_SINK: OnceLock<Mutex<Option<Arc<dyn UsageSink>>>> = OnceLock::new();
// crates/yekitrim-core/src/owner.rs:199
static LOCAL_ADMIN_UID: AtomicI64 = AtomicI64::new(1);
```

两处的失败模式都是**静默降级**：漏装 sink = 所有 token 不记账且不报错；
漏设 uid = 所有人落到 `users/1/`。当前调用点齐全（`service/usage.rs:41`、`server/lib.rs:184`、
`cli/main.rs:217`），所以现在**没有 bug**；但这属于"下一次加入口就会踩"的结构。
最低成本的做法不是改成显式参数（改动面大），而是在 server 启动与 CLI 入口各加一条
`debug_assert` / 启动日志："usage sink 已装 / local admin uid = N" —— 让漏装从静默变成可见。

同类的还有 `std::env::set_var("VTEDIT_RENDERER"/"VTEDIT_STRICT")`（`cli/main.rs:538`）：
进程级环境变量当参数传，出片侧在 `render/pipeline.rs:372/456` 读。现在只有一个后端所以无害；
一旦并发两种渲染配置，就是"两个任务互相改对方的参数"。edition 2024 的 crate 里
`set_var` 已是 unsafe —— 版本升级时会先在这里炸。

### 3.4 同步 IO 纪律有 3 处漏网（AGENTS 的"第一条"）

根卡把"同步 IO 绝不能留在 async 线程上"列为全局不变式，`api::blocking` 也确实被用得不错
（polish.rs / config.rs 的 handler 里 0 处裸 `std::fs`）。但按函数逐个扫，还有 3 个 async 函数
直接做同步文件操作、没有进 blocking 池：

| 函数 | 操作 | 位置 |
|---|---|---|
| `clip_delete_impl` | `remove_dir_all` 整个片段目录（含成片）+ 读写两个索引文件 | `api/project_data.rs:1206-1260` |
| `projects_source_delete` | 递归 `read_dir` 目录累计大小 | `api/projects.rs:396` |
| `create_local_project` | `create_dir_all` + `read_dir` 扫描候选 | `api/projects.rs:173` |

三处都在慢盘/网络盘上会**直接占住一个 tokio worker** —— 正是这条纪律想防的场景
（根卡里写着"一次 `exists()` 就要几百毫秒到几秒"）。`clip_delete_impl` 尤其明显：
它删的是**整个片段目录**，而片段目录里是成片 mp4。

**成本**：每处一个 `blocking(move || …)` 包一下，机械改动。

### 3.5 单连接 SQLite + 无保留期

- `Db { conn: Mutex<Connection> }`（`db.rs:223`），**全局一把锁**，没有 `busy_timeout`。
  当前所有 DB 操作都很短，够用；但 `projects_list` 里 `owner_name(db, p.owner_id)` 在
  **每个项目上跑一次 `list_users()` 全表扫描**（`api/projects.rs:86` → `db.rs:521`）——
  典型 N+1。10 个项目就是 10 次全表扫描，且都持同一把锁。
- **保留期缺失**（运维面，24/7 部署会慢慢长）：
  - `purge_expired_sessions` 只在**启动时**调一次（`auth.rs:209`），而它自己的注释写的是
    "启动 + 每日一次的后台任务用" —— **那个每日任务不存在**；
  - `audit` 表无清理；`llm_usage` 表无清理；
  - `<data_dir>/logs/yekitrim-YYYYMMDD.log` 每天一份，**永不删除**
    （`render_proc::prune_logs` 只匹配 `render-*.log`，保留 20 份）。

---

## 4. 当天可做的快赢

按"会不会真伤到仓库"排序：

1. **`crates/yekitrim-server/%SystemDrive%/`** —— 932 KB，2026-10-02 创建的 Windows
   字体缓存 `.db`（`cversions.2.db` 等）。这是某次带未展开 `%SystemDrive%` 的路径操作留下的，
   不在 git 里（被 `*.db` 忽略），但会被任何 zip/备份带上，也会让 `cargo`/IDE 的目录遍历莫名其妙。
   **删掉。**
2. **`.gitignore` 不挡 `video/`** —— `video/` 有 **2,693 个未跟踪文件 / 85 MB**，
   `git status -uall` 已经全部列出来。`.gitignore` 只有 `*.mp4`，`.jpg` 没挡。
   你们 2026-10-11 就出过一次"一次 `git add -A` 卷进 144 个中间帧（含队友半成品）"的事故
   （写进了 `.gitignore` 注释）。**补一条 `/video/`（或把要留的素材挪进受控目录）。**
3. **AGENTS 指令链预算只剩 6 字节**：门禁输出
   `最坏链 57338/57344 B（root + webui + webui-src + webui-components）`。
   也就是说**下一句新规矩**加进这四张卡里的任何一张，门禁就红。
   现在就该把 `webui/src/components/AGENTS.md`（22.3 KB，25 条 ⚠️）里已经落成代码的历史叙述
   挪去 `docs/`，给后面留出余量 —— 否则下一次改动会被迫先做这件事，而且是在赶时间的时候。
   （另一条链 `root + server + api` 是 57,322 B，只剩 22 B —— 见 §3.1。）
4. **`purge_expired` 的注释与实现对齐**：要么补每日清理任务（顺手把 audit / llm_usage / 按天日志
   的保留期一起做），要么改注释。
5. **路由 401 覆盖改成枚举式**：现在 `routes.rs:913` 手写了 33 条路径，而注册了 127 条。
   新加一条路由忘了挂 `protected`，**没有任何测试会红**。改成"从 `routes.rs` 抽路由表，
   逐条断言：要么在白名单，要么未登录必 401" —— 一次性覆盖，且以后自动生效。
   （`route_manifest.py` 已经有抽取器，直接复用。）
6. **构建/依赖卫生**：
   - `webui/` 同时有 `package-lock.json` 与 `pnpm-lock.yaml`，`package.json` 无 `packageManager`
     —— 两份锁会漂，选一份；
   - workspace 无 `rust-version`、无 `clippy.toml`，edition 6×2021 + 2×2024；
     `cargo check` 的"0 warnings"是约定不是门禁（无 `#![deny(warnings)]`、无 CI）。
     建议 workspace 统一 edition 并写 `rust-version`，否则每次升工具链都是一次盲盒。
7. **§3.4 那 3 处裸同步 IO** —— 机械改动，包一层 `blocking` 即可。

---

## 5. 10-15 那两份文档的逐条回查

| 原编号 | 内容 | 本轮状态 |
|---|---|---|
| S1 | 门禁无执行入口 / 无 CI | ❌ 仍开放（§2.1） |
| S2 | 前端零测试 + 无契约 | ❌ 仍开放（§3.2） |
| S3 | `cover-takumi` feature 只在 CLI 侧传播 | ❌ 仍开放（`server` 无 feature 转发，单 crate 测试看不到 `takumi.rs`） |
| W1 | `ingest.rs` 汇合两条 worker 语义 | ❌ 仍开放（`ingest.rs` 3,235 行，引 `ingest_worker` 11 次 / `download_worker` 4 次） |
| W2 | `api/polish/` 混装 | ❌ 仍开放且更大（§3.1） |
| W4 | 路径判据 3 份 | ❌ 仍在（`scope.rs` / `polish.rs` / `core/tenant.rs`） |
| C1+C2 | `detection → cutting` 横边 | ❌ 仍在（`detection/mod.rs:294,300`） |
| C4 | 两个隐藏全局 | ❌ 仍在（§3.3） |
| C5 | `merge()` 手写 130 行 | ❌ 仍在（`config/mod.rs:649`） |
| E4 | 门禁空转（缺样本 SKIP + exit 0） | ⚠️ 本轮未复核 |
| B1 / E6 / B6 | SSE 终态 / 预览 LRU / health 路径 | ✅ 已落地（本轮看到实现） |
| B7 | 产物增长无配额 | 🟡 部分（`api/mod.rs:142` 的 `quota_gate` 已在 4 处调用，但只拦"建任务前"，不拦产物本身） |

> ⚠️ 这份表只代表"我这一轮读到了什么"。`engineering-review-2026-11-14-addendum.md` 里声称已修的项，
> 我没有逐条复核。

---

## 6. 建议排期

| 顺序 | 事项 | 理由 | 风险 |
|---|---|---|---|
| 0 | §4 的 3 条伤仓库项（`%SystemDrive%` / `video/` / 预算） | 半小时级，且不做会继续失血 | 无 |
| 1 | `scripts/gate.mjs` + `route_manifest` 接上 | 保护已有 1598 测试 + 82 脚本的投入 | 低 |
| 2 | 路由 401 枚举测试 + 请求体改 `Deserialize` 结构 | 一次性消掉最大缺陷类别的一部分 | 低 |
| 3 | 拍板"DOM 近似预览是否追究像素" | 决定这 3 套渲染是继续并行还是收敛 | 需要产品口径 |
| 4 | 拆 `api/polish/` + 禁 glob 重导出 | 让"该放哪"重新有答案 | 中（纯搬运，**且要先做 §4.3 卡瘦身**，见 §3.1 的预算约束） |
| 5 | 拆 `ingest.rs`（录播姬 / B站分家）+ 补成对判据 | 门禁盲区，语义已在汇合 | 中 |
| 6 | 保留期 + 每日清理任务 | 24/7 部署的慢性病 | 低 |
| 7 | `cover-takumi` 归属 / 统一 edition / 锁文件 | 卫生 | 低 |

**排期原则**：第 1、2 项是"一次投入、长期自动生效"；第 3 项必须由你拍板，我不该替你定；
第 4、5 项是纯搬运但工作量大，建议在"当前功能不再大改"的窗口做。

---

## 7. 局限（如实）

- **没有编译、没有跑测试、没有起服务**：沙箱无 Rust 工具链（`cargo` 不存在）。所有"已落地"是读码判断。
- 没有复核 `engineering-review-2026-11-14-addendum.md` 的"已修"清单；§5 只标了我本轮看到的。
- `core/asr` 的算法内部（约 2,800 行）、`detection/llm.rs` 的两趟细节、`polish/process` 的 Step
  顺序语义，本轮**没有逐行审**。10-15 文也如实标过这几处未覆盖。
- 前端只看了结构与关键路径（`client.js` 的去重/超时、`workbench.js` 的单飞、`PolishPreview` 的
  两条腿），没有逐组件审。
