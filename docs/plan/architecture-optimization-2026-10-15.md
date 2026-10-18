# YekiTrim 整体架构优化建议

> 状态：**建议书，未动一行代码**
> 日期：2026-10-15 ｜ 基线：`eba5c10`
> **范围：排除 `crates/yekitrim-polish`**（该 crate 正在独立重构，本轮不参与提议）。

---

## 0. 先纠正上一轮的结论

上一轮我说"整体架构没有问题"——**那句话我没有资格说**。当时我数过门禁覆盖：

`check_module_boundaries.py` 的 17 条规则指向的目录只有 `core/src`（整体）、`polish/src`、
`service/src`（含 3 个 P4 子目录）、`server/src`、`cli/src`、`render-core/src` 与 4 个前端目录。
**而 "某个 crate 整体" 这条匹配的是"层间不许反向引用"**，不是 crate 内部设计。

具体地，这些目录在门禁里**一次都没被提到**：

| 目录 | 门禁引用数 |
|---|---:|
| `core/src/detection`（1622 行） | 0 |
| `core/src/cutting`（1167 行） | 0 |
| `polish/src/process`（4165 行） | 0 |
| `polish/src/project`（3191 行） | 0 |

**所以「17 条规则 0 违规」证明的是"crate 之间没有反向依赖"，不是"结构合理"。** 这两件事差得很远，
我上一轮把它们混为一谈了。

本轮的结论基于**重新取证**：我读了 core 的 detection/cutting、service 的 lock/workspace、
server 的 scope/api/worker，以及构建与门禁体系本身。并按你的要求**完全排除 polish**。

---

## 1. 结论：三个真正的结构问题

排查后，**排除 polish 之后**，本仓的结构问题收敛为 **3 条工程级 + 6 条 core 模块级**。
工程级三条有一个共同特征：**都不是"代码写坏了"，而是"某件重要的事只存在于人的约定里，没有机器承接"。**

| # | 问题 | 性质 | 影响 |
|---|---|---|---|
| **S1** | **53 个门禁脚本、40 条分散在卡片里的验证命令，却没有统一的执行入口，也没有 CI** | 工程化缺口 | 1236 个测试与全部守卫**依赖人记得跑** |
| **S2** | **前端零单元测试，且与后端无类型化契约**（27 个 .js + 43 个 .vue，0 个测试文件） | 覆盖缺口 | 116 条路由的请求/响应形状靠人肉对齐 |
| **S3** | **`cover-takumi` feature 只在 CLI 侧传播** | 构建图缺口 | server 语境下封面整条不参与编译（详见 §3） |
| **C1~C6** | **core 的检测/剪切/配置结构问题** | 模块级 | 见「核心业务模块」小节，**C1+C2 与 C4 最值得先动** |
| **W1** | 🔴 **`ingest.rs` 是两条 worker 边界的合法绕过通道，门禁抓不到** | **门禁盲区** | 禁令想防的事**已经发生了**，见「service/server」小节 |
| **W2~W5** | `api/polish.rs` 混装 8 领域 / `ingest_worker` 混装 / 路径判据三份 / 锁键不可互推 | 模块级 | 见同节 |

> ⚠️ **我的判断在本轮被推翻过两次**：第一次说"架构没问题"（当时没读过 core 的检测/剪切）；
> 补审 core 冒出 6 项；再补审 service/server 又冒出 5 项，**其中 W1 是本次复盘最重要的发现**。
> 这些**全部不在 polish 范围内，是独立成立的**。

### 附：核心业务模块的结构问题（core）

上一轮我**没有审计过 `core` 的检测/剪切**，却写了"架构没问题"。补审后确认 **6 项真实的结构问题**，
按严重度列出。⚠️ 注意：这些**不在 polish 范围内**，是独立成立的。

| # | 问题 | 证据 | 性质 |
|---|---|---|---|
| **C1** | **Stage 3 越界干 Stage 4 的事** | `detection/mod.rs:296,302` 调 `cutting::clip_index::{assign_clip_dirs, refresh_index}` | 职责越界 |
| **C2** | **core 内唯一横边 `detection → cutting`** | `crate::cutting` 在 core 内被引 **2 次，全部来自 detection**；cutting 不反向引用 | 边界纠缠 |
| **C3** | **transcript 同一次管线 1 次内存 + 3 次落盘，下游三处各写一遍三级回退** | `asr/mod.rs:363/370/372/387` 落盘 3 份；`core_pipeline.rs:288`、`cutting/mod.rs:855`、`workspace.rs:80` 各实现一遍回退 | 数据流绕路 |
| **C4** | **两个隐藏全局，漏初始化即静默降级** | `owner.rs:199 LOCAL_ADMIN_UID`（漏了 → 静默用 uid=1）、`llm/usage.rs:70 DEFAULT_SINK`（漏了 → **静默丢全部 token 记账**） | 隐藏耦合 |
| **C5** | **`config/mod.rs:467 merge()` ~130 行手写逐字段 if** | "非默认值才覆盖"的判据硬编码在每个 if 里，与 `Default` impl 是**两份必须手工同步的定义**；而 `patch_config_file:969` 已用 toml_edit 递归深合并 | 抽象错位 |
| **C6** | **`detection/mod.rs run()` 单函数 283 行横跨 5 类职责** | 弹幕密度 / LLM 主趟 / 第二趟 / 事件后处理评分 / 写 cutting 的地盘，无子模块边界 | 职责混乱 |

**C1+C2 是本轮最值得先动的一条**（ROI 最高）：把 `assign_clip_dirs` + `refresh_index` 从
`EventDetector::run` 摘出去、移到 Stage 4 或 `core_pipeline` 的 cut 装配处。收益有三：
① 消除 core 内**唯一的横边**；② 让「谁命名片段目录」有**唯一归属**——
现在这条规则住在 cutting、被 detection 调用、**前端还抄了第三份**
（`utils.js:40 clipMatchesEvent`，`cutting/mod.rs:822` 的注释自己承认"判据与前端 utils.js 一致"）；
③ 那个调用是**可选副作用**（失败只 warn），本就说明它不属于检测职责。

**C4 最危险**：`DEFAULT_SINK` 漏装 = **用户用量全部不记账且不报错**。C4 的修法是
把两者从"进程全局"改为**显式参数**，或至少在 core 入口 assert 已初始化 —— 让"漏初始化"
从静默降级变成启动期错误。

⚠️ **但 C3/C5/C6 我建议排在后面**：
- C3 的"三处三级回退"看起来像重复，但三处**基准不同**（`core_pipeline` 认 `-t` 覆盖、
  `cutting` 认 `ctx.transcript`、`workspace` 认配置相对路径）——**可能是三种不同的解析契约**，
  合并前必须先确认哪一个是权威，否则会把有意的差异抹平（这正是我在上一轮计划里写的
  "发现某处本来就该不同 → 立即停止该分支"）。另外 SRT 兜底**有损**（丢 `confidence`/`language`）
  是**真 bug 候选**，值得单独确认。
- C5 的 merge 手写白名单确实脆，但它**现在是对的**（只是容易漏改）。加字段时的五步协议在
  `core/config/AGENTS.md` 里已写明。改成反射/生成是净收益但**非紧急**。
- C6 的 283 行 `run()` 是真实的可读性债，但它**当前职责顺序是有硬约束的**
  （`detection/AGENTS.md` 明确写了后处理顺序）——拆函数可以，**拆顺序不行**。

### 附：service / server 的结构问题（W 系列）

| # | 问题 | 证据 | 性质 |
|---|---|---|---|
| **W1** | 🔴 **`ingest.rs` 是两条 worker 边界的合法绕过通道，门禁抓不到** | `ingest.rs` 引 `crate::ingest_worker::` **9 次** + `crate::download_worker::` **4 次** | **门禁盲区** |
| **W2** | `api/polish.rs` 2471 行混装 ≥8 个领域 | 生产段 font 提及 85、cover 106；文件头自述只有 5 件事 | 职责混乱 |
| **W3** | `ingest_worker.rs` 把「HTTP 用的扫描缓存」「入库状态机」「文件搬运」压在一起 | `RECORDINGS_CACHE`(`:326-353`) 的消费者是 **HTTP 接口** `api/rooms.rs:68` | 职责混乱 |
| **W4** | server 重复实现 service 的目录解析；**同类路径判据存在 3 份** | `scope.rs:252` / `sticker_index.rs:294` / `clip_index.rs:358` | 抽象错位 |
| **W5** | 同一资源两种不可互推的锁键 | `scope_key` 的 `dir:{规范化}` vs `filelock` 的 `<dir>/polish.toml` | 状态表达 |

#### ⚠️ W1 值得单独说明：**这是本轮最重要的发现**

本仓的边界门禁有一条规则 —— `download_worker 不许引用 ingest_worker`（`check_module_boundaries.py:90-97`），
理由是 `pipeline_boot.rs:7-11` 写得很清楚：*"录播姬那套待分配池/空闲窗口语义会顺着调用链渗进 B站通道"*。

**但 `ingest.rs` 是第三方，两条边都能走。** 我实测确认：它引 `ingest_worker` 9 次、`download_worker` 4 次，
于是**录播姬语义与 B站语义在同一个 1601 行文件里汇合**（`handle_event:161` 与 `bilibili_submit:1398` 同居）。
规则字面上没被违反（两个 worker 确实没有互相 import），**禁令想防的东西却已经发生了**。

这解释了为什么它和 S1 是**同一个病**：**门禁只写在"最窄的那处"**，而绕过它的路径没有被覆盖。
`pipeline_boot.rs` 的上提是对的（那条边断了），但同一条理由**没有推广到 `ingest.rs`**。

**建议**：把规则从"worker 之间不许引"升级为**"worker 的语义不许在路由层汇合"** ——
即按领域拆 `ingest.rs`（录播姬 webhook / B站提交 分家），或至少加一条**成对判据**
（`PAIRED_RULES` 已支持这类跨行文件级约束）。

---

### 附：S1 的证据

```
根 scripts/ 门禁脚本 : 43
webui/scripts/ 门禁   : 10
合计                  : 53 个检查脚本
各模块卡里记载的不同验证命令 : 40 条
仓库根 CI 配置        : 无（无 .github / 无根 package.json / 无 gate runner）
cargo 测试函数        : 1236 个
```

根 `AGENTS.md` 的「验证」节只列了两条结构门禁，其余 38 条散在 36 张卡里，**靠"读卡的人自己记得"**。
本仓的规矩写得极好（每条守卫的 doc 都写了它防的真实事故），但**没有一个地方能让一次改动跑完全部相关的守卫**。

⚠️ 注意 `.workbuddy/` 是 gitignore 的，`gates.mjs` 在其中**并不存在**（实测）—— 也就是说
现在连"私人的总闸"都没有。

---

## 2. S1：把门禁从"约定"变成"可执行"

**这是本轮最高优先级的建议**，因为它**保护的是已经投入的 1236 个测试与 53 个守卫**——
这些是本仓最大的资产，而它们目前没有自动执行的下游。

### 建议 2.1 —— 建一个统一入口 `cargo xtask gate`

本仓已有 `xtask`（`cargo xtask scan|package`），且其卡片明确写着它**刻意只依赖 std + sha2 + zip**、
不许调 python/PowerShell。所以**不要**把门禁塞进 xtask。

改用**根目录一个零依赖的 runner**（node，因为 `scripts/` 本来就混用 python3 与 node）：
`scripts/gate.mjs`，职责只有"按改动面选该跑哪些，并如实报出跳过了什么"。

⚠️ **关键设计约束（从本仓已有的教训来）**：
- `yekitrim-compare.mjs` 的头部记着一条事故 —— `--all-fixtures` 的参数白名单**漏了 `--golden`**，
  于是"静默换回活参照、只换口径不报错、数字还都在阈值内 ✗"。**新 runner 必须沿用同一条纪律**：
  任何"没跑"都要**显式打印**，不许静默跳过。
- 守卫的**自检（`--self-test` / `--mutate`）必须一起跑**。`check-effect-registry.mjs` 的卡片记着：
  "它曾经烂了很久（断言写死 15 个类型、而词汇表早已 11 个）却没人发现，原因就是**主检查进了 gates、自检没进**"。
  这条正好说明**当前"靠人记得"的模式已经出过事**。

### 建议 2.2 —— 补上 CI

本仓**没有任何 CI**。哪怕先只做一件事：**在 push/PR 上跑 `cargo test --workspace` + 两条结构门禁**，
就能拦住绝大部分回归。这是投入产出比最高的一项。

⚠️ 前置：本仓有 `vendor/sherpa-onnx-sys`（会按需下载预编译库并 sha256 校验）与 `webui/dist`
（编译期输入、已入库），CI 需要先解决这两处的离线可用性，否则第一次跑就会红。
建议先在**本机**把 `gate.mjs` 跑通、再谈 CI（与 §5 的排期一致）。

---

## 3. S3：`cover-takumi` feature 的构建图缺口

**已复验**（路径依赖不带 feature 是 Cargo 的确定语义，不依赖 grep）：

- `polish/src/render/takumi.rs` 有 `#![cfg(feature = "cover-takumi")]`，`render/mod.rs` 同门控；
- `polish/Cargo.toml` 是 `default = []`；
- **只有 `cli/Cargo.toml` 有 `default = ["cover-takumi"]`**；
- `server/Cargo.toml` 与 `service/Cargo.toml` 对 polish 都是**无 features 的路径依赖**，
  且 `server/src` 的 `[features]` 块是**空的**。

**后果**：`render/takumi.rs`（1051 行，**外加** `tests/takumi_font_id.rs`）在
`cargo test -p yekitrim-server` / `cargo check --workspace --all-targets` 下**整份不参与编译**，
而它正是 WebUI「生成封面」的后端。于是 `/api/health` 的 `cover_static.compiled`
（`api/mod.rs:236, 271-275`）在 server 语境**恒为 false**。

**这为什么是结构问题而不是小 bug**：它让"封面能力"这个事实**同时有两个真相** ——
CLI 构建里为真、workspace 构建里为假，而**两者都不会报错**。

**建议**（三选一，需你拍板；本轮不实施）：
1. 把 `cover-takumi` 提升为 polish 的默认 —— 但 `cli/Cargo.toml` 记着**不这么做的理由**：
   它会拉长构建、Windows 上带出 `libwebp-sys` 的 `LNK4098` 警告、**发行产物许可面要重核**。
2. 让 server 显式转发该 feature —— 保留"库构建精简"的好处，同时让 server 语境能编到它。
3. 明确"封面只经 CLI 可用"，并把 health 位改成如实反映。

⚠️ 无论选哪条，**都要加一条判据守住它**（否则下次还会漂）。`cover.rs` 里那处
`#[cfg_attr(not(feature=...), allow(dead_code))]` 补丁也是这件事的症状。

---

## 4. S2：前端契约

前端 **43 个 .vue + 27 个 .js，0 个单元测试文件**。它有 10 个 `webui/scripts/check-*.mjs`
（合计 299 KB，主要是像素/布局对拍），但**这些守的是"画得对不对"，不是"数据接得对不对"**。

与此同时后端有 **116 条路由**，而 `webui/src/api/` 是**纯 .js**（无 TS、无运行时校验），
请求/响应形状靠两侧人肉对齐。

**但这里我要给出一个反对"上 TypeScript"的建议**，理由是本仓已有两处**成功的**替代实践：

- `api/client.js` 是**唯一请求入口**（实测全仓只有 7 处 `fetch(`，其中只有 `client.js:85` 打 `/api`），
  统一留痕、统一错误语义 —— 这是**收敛点**，比类型更能防"散着写"。
- `scripts/check-dhampir-wiring.mjs` 已在钉"**隔着边界的名字**"（前端调的 HTTP 路径 ↔ `routes.rs` 注册的路由）。
  这正是前端契约里最容易断、且**编译期不会报错**的那一类。

**建议走第三条路**：把 `route_manifest.py` 从"只 dump 不比较"升级为**真的比对**。
它现在的卡片明确写着 *"exit code **始终 0** —— 比较是调用方的事"*，而**没有调用方**。
即：**已有的能力被写好了却没接上**（与 S1 同源：缺执行者，不缺工具）。

具体：让 `webui/src/api/*.js` 里出现的 `/api/...` 字面量集合，与
`routes.rs` 抽取的路由集合做双向差集 —— 前端调了不存在的路由 = 红；后端注册了前端从不调的 = 提示。
⚠️ 这条判据要允许白名单（SSE、静态资源、`/api/admin/*` 等），且**白名单本身要有注释写明理由**。

---

## 5. 明确判定为健康、建议不动的部分

避免"为了重构而重构"：

**① 分层与依赖方向本身是干净的。** 六 crate 单向依赖成立，`service → server` 反向边为 0
（这是 `service` 能被抽出来的全部理由）。`server` 直接 `use yekitrim_core::` 79 处**不是违规**——
`core` 是共享底座，硬规矩只禁 `service → server`。**这条保持。**

**② 三套配置 schema 是合理的，不是重复。** `ProjectConfig`/`Config`(core, 117 字段)、
`ProjectFile`(polish, 132 字段)、`ElementsConfig` 分属不同生命周期（管线配置 / 精修工程 / 元素轨），
且 `config_version` + `ConfigKind` 迁移表（`version.rs`：`== / < / >` 三分支）已经把它们管住了。
**这是本仓设计得最好的部分之一，别合并。**

**③ 协调机制是分层的，不是重叠的。** 实测三个锁各管一段，语义清晰：
`filelock.rs`（同路径读改写串行）、`instancelock.rs`（跨进程单实例）、
`scope_key.rs`（任务资源键互斥，且有"全切不带 `clip:*`"这类**刻意设计的并行度**）。
**保持。**

**④ 路径三防线是有意的纵深防御**（`scope.rs` / `api/config.rs` / `tenant::ensure_within`），
`ensure_within` 有 10 处调用点。根 `AGENTS.md` 明确写了"三处缺一不可"。**这是安全设计，不是冗余。**

**⑤ `docs/` 与仓库散件** —— 上一轮已单独成文（见 `architecture-refactor-plan-2026-10-15.md` §4.3/§4.4），
此处不重复。

---

## 6. 建议的排期

| 顺序 | 事项 | 理由 | 风险 |
|---|---|---|---|
| 1 | **W1 拆 `ingest.rs` + 补一条成对判据** | 门禁**盲区**，且禁令想防的事已经发生 | 中（按领域拆文件） |
| 2 | **§3 `cover-takumi` 定性** | 唯一"功能可能实际不对"的项 | 需设计决策 |
| 3 | **C1+C2 `assign_clip_dirs` 挪出 detection** | 消除 core 内唯一横边 + 让目录命名有唯一归属 | 低（纯搬迁） |
| 4 | **C4 两个隐藏全局改显式** | `DEFAULT_SINK` 漏装 = **静默丢全部记账** | 低-中 |
| 4 | **§2.1 `scripts/gate.mjs`** | 保护已有 1236 测试 + 53 守卫的投入 | 低（只增脚本） |
| 5 | **§4 `route_manifest.py` 接上比较** | 已有能力没接上，补齐即收益 | 低 |
| 6 | **§2.2 CI** | 让上面各条自动执行 | 中（需解决离线依赖） |
| 7 | C3 / C5 / C6 | **需先确认是否真有差异**，见上方说明 | 中-高 |
| 8 | §5 各项 | 维持现状 | — |

**排期原则**：第 2、3 项之所以排在工程化之前，是因为它们是**"确定的、小范围的结构修正"**，
且 C4 涉及**静默的数据丢失**；而工程化改造（gate/CI）虽价值更高，但形状需要先讨论。

**⚠️ 前置约束**：当前工作区不干净（12 个文件已改，含 `tools/polish-to-dhampir.mjs` 与两个前端组件），
且 polish 正在独立重构。**建议先落定当前改动**，再启动上述任何一条。

---

## 附：本轮的方法与未覆盖面

**本轮实际取证的范围**：`core` 的 `detection`/`cutting`/`config`/pipeline/全局状态、
`service` 的 lock/workspace/db、`server` 的 scope/api/worker/ingest、
构建体系（`build.rs` / feature 传播）、门禁体系统计、前端结构与测试覆盖。

**本轮新增覆盖**：`core` 全模块（含 detection/cutting/config/pipeline/全局状态）、
`service` 与 `server` 全模块（含 `ingest`/`jobs`/`scope`/`api`/`auth`/`workers`）。

**仍未覆盖**（如需要可继续）：
- `core/asr` 的**算法内部**（约 2800 行：`local.rs`/`align.rs`/`vad.rs`/`correct.rs`）——
  只审了它与 detection/cutting 的**接口**（C3 的数据流），没读算法本身
- `core/detection/llm.rs` 与 `scattered.rs` 的内部设计（只看了函数清单与两趟的差异点）
- `service/db.rs`(1628) 的 schema 与迁移策略
- `server/auth.rs`(869) 的会话模型细节

**三条诚实的提醒**：

1. **我给出的"健康"判定（§5），依据强度不高于我给"问题"判定的强度。**
   后续若要动 §5 里任何一项（尤其配置 schema 与路径三防线），应当**先单独取证**，而不是引用本文结论。
2. **本轮 core 的 6 项是我第一轮完全漏掉的**，说明"某个 crate 没被审过"这件事，
   在结论里必须显式标注，而不是默认它没问题。这正是本文件 §0 那段存在的理由。
3. **C3/C5/C6 我刻意没有直接建议"合并/重写"**，因为它们**可能是三种有意的差异**。
   对这类问题，取证成本高于修改成本 —— 先确认，再动手。
