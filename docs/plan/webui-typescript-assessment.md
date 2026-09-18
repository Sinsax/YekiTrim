# 前端迁 TypeScript 评估

> 评估日期：2026-09-16 · 结论基于当时仓库状态的实测与逐条清点（行数用 `wc -l`、缺口用 `git ls-files` / `ls` 核对）
> **本文档只做评估，不含任何代码改动。** 用户决定：暂不动代码；测试/CI 暂不建，将来只考虑 TS/类型。

## 0. 结论

1. **换 TS 会更好维护，但收益的大头不在 TS 本身**，而在两件事：**类型有单一来源**（从 Rust serde 结构生成）、**检查能自动跑**（CI）。
2. **JS 对运行时与性能零影响** —— TS 编译后就是 JS，打包体积、执行速度都不变。JS 影响的是**改坏了多久才发现**。
3. **别指望迁完 TS 就"稳了"**：本项目最危险的那个 bug（路由参数复用把 A 项目的字幕写进 B）是**生命周期**问题，TS 抓不到；26 条渲染/CSS 类也抓不到。类型能覆盖的是"A 类"（形状/命名/参数），约占总缺陷的 1/4。
4. 这个仓库真正的短板不只是缺类型，是**测试与 CI 两条自动防线都没有**。TS 值得上，但请按 §6 的顺序上。

## 1. 规模清点（决定成本）

| 组成 | 数量 | 行数 |
|---|---|---|
| `webui/src/**/*.js`（手写，已排除 `src/wasm/` 的生成物） | 22 个文件 | **2,727** |
| `webui/src/**/*.vue`（其中 `<script setup>` 约占 6,500 行） | 36 个文件 | **10,989** |
| `crates/vtrim-polish/templates/index.html` 内联 JS（两段 `<script>`） | — | **≈2,315**（全文 2,562） |
| **真实迁移目标** | | **≈9,000 行** |

单文件热点（`<script setup>` 行数）：

| 文件 | 全文 | script 块 |
|---|---|---|
| `webui/src/components/TimelineEditor.vue` | 1,610 | **1,245** |
| `webui/src/components/ProjectClipsSection.vue` | 1,261 | 948 |
| `webui/src/components/CoverDesigner.vue` | 1,271 | 947 |
| `webui/src/views/PolishView.vue` | 1,062 | 752 |
| `webui/src/components/PolishPreview.vue` | 765 | 631 |
| `webui/src/polish/engine.js` | 643 | — |

## 2. 基础设施：当前是"零"

- **无 `tsconfig.json` / `jsconfig.json`**（实测不存在）。
- devDeps **没有 `typescript` / `vue-tsc`**（`package.json` 只有 `vue`、`vue-router`、`@vitejs/plugin-vue`、`vite`）。
- **无 ESLint / Prettier 配置**。
- **无测试、无 CI**：没有 `*.test.js` / `*.spec.js` / vitest / jest 配置，**没有 `.github/` 目录**。
- **类型标注近乎为零**：`webui/src` 全文只有 **`@param` ×1、`@returns` ×1**（都在 `api/client.js`），`@typedef` / `@type` / `@ts-check` 均为 0。
- **无运行时 schema 校验**（没有 zod / io-ts / ajv 之类）：`api()` 只做 content-type 嗅探 + `JSON.parse` try/catch + `ok === false` 判断，之后所有 payload 都当裸对象用。
- **权威形状只在 Rust 侧**（`crates/vtrim-render-core/src/types.rs`、`crates/vtrim-polish/...` 的 serde 结构），前端没有镜像、也没有生成器 —— 前端改错字段名在编译期没有任何提示。
- ⚠️ **唯一的漂移防线没有纳入版本控制**：`git ls-files webui/scripts/` **返回空**，即 `check-overlay.mjs`（覆盖层色档契约的对拍脚本）**未被 git 跟踪**，也没有挂进任何 npm script（`scripts` 只有 `dev`/`build`/`preview`/`build:wasm`）或 CI。它随时可能悄悄消失。

## 3. 历史缺陷分类（判断"TS 值不值"的关键）

方法：把 `AGENTS.md` 的 WebUI 章节（大量"⚠️ 曾经踩过"注记）、`docs/plan/*.md`、以及 `git log` 里 `fix(webui)` / `fix(polish)` 的提交正文逐条归类，一条记录算一次。

| 类别 | 条数 | TS 能否拦住 |
|---|---|---|
| **A 类型 / 形状** —— 字段名错、字符串当数字、falsy 判定把合法 0 当空、配置 key 改名 | **28** | ✅ 多数能 |
| **B 事件 / 状态 / 生命周期** —— 重复点击、忘退订、陈旧响应盖新、路由复用、挂载时机 | **31** | ❌ 基本不能 |
| **C 渲染 / CSS / 布局** —— 优先级、flex 收缩、零尺寸、`<template>`、容器查询单位、坐标系 | **26** | ❌ 不能 |
| **D 跨实现漂移** —— 同一条规则 3~4 处副本各自演化 | **18** | ⚠️ 只能减轻 |

A 与 B 并列最大；若把 2026-09 那批"读失败被当空数据 / 静默刷新 / 陈旧 gating"（同一根因、被记成约 8 条）合并看待，A 就是最大的一类。

各类的代表性例子：

**A（类型能拦）**

- `webui/src/utils.js` / `PolishView.loadClips`：按字符串排序 → `clip-100` 排在 `clip-02` 前。
- `webui/src/api/jobs.js` 的 `KIND_ZH` 缺 `stage_process` → 抽屉显示「预处理 · process」。
- `ProjectClipsSection.vue` 的 `readSort()` 回落写死 `key:'rank'` → 排序栏一个都不高亮、还按不存在的键静默排序。
- 同文件 `FILTERS` 的 key 从 `unpolished` 改名 `polish` → localStorage 里的旧偏好静默指向另一批片段。
- `webui/src/polish/audio.js`：`parseFloat(x) || 1` —— 音量 0 是合法值，被判成"空"于是变回满音量。
- `webui/src/api/projectData.js` 的 `cutAll` 调用多包了一层 `{method, body}` → 服务端静默丢弃 `ranks`，把**全部**切了。
- `jobClose?.()` —— `watchJob()` 返回的是 `{close()}` 对象，当函数调用 → 三个视图永久卡"运行中"。

**B（类型拦不住）**

- 同一 route record 下只变 `:pid` → 组件被复用，`onMounted` 不再跑，**A 项目的字幕被写进 B**（本项目最危险的数据完整性问题）。
- `onJobDone` 注册后不退订 → 反复挂载攒出陈旧回调，任务结束同时打一整套请求（"1 秒 52 次连发"）。
- 读失败被当成空数据 → 用户拖动一次就 `PUT segments: []`，把磁盘上的真实数据盖掉。

**C（类型拦不住）**

- `.src-badge` 被压缩到 18px 而「本机」需要 30px → 只显示半个字。
- 波形画在不透明的字幕块下面（z-index 1 vs 2）→ 说话时段整段被盖。
- 裸 `<template>` 被渲染成真元素 → 整块界面看不见，但 `document.querySelector` 照样查得到。

**D（只能减轻）**

- 覆盖层色档规则有 3 个真实实现（`timeline.rs` / `templates/index.html` / `polish/engine.js`）+ `PolishPreview.vue` 作为消费方。
- 运镜倍率表 3 处（`types.rs::CameraStyle::factor` / 模板 `camFactor` / `engine.js::camFactor`）。
- `transparent` vs 同色零透明：canvas 在非预乘空间插值，导致**同一个源帧下浏览器预览与出片观感不一致**。

## 4. "该长什么样的类型"（A 类逐条映射）

这是迁 TS 最实际的一份清单 —— 每条都是本项目踩过的，且**都是构建期能报错的**：

| 现场 | 该有的类型 | 效果 |
|---|---|---|
| `KIND_ZH` 缺 `stage_process` | `Record<JobKind, string>` + 穷尽 `JobKind` 联合 | 缺键 = 编译错误 |
| `readSort()` 写死 `'rank'` | `type SortKey = typeof SORT_OPTIONS[number]['key']` | 非成员值 = 报错；回落用 `SORT_OPTIONS[0].key` 则恒合法 |
| `FILTERS` 改名 `unpolished`→`polish` | `type FilterKey = 'cut'\|'uncut'\|'polish'\|'all'` 同时用于选项表与持久化偏好 | 改名而未迁移 = 读取处报错 |
| `watchJob()` 被当函数调用 | 返回类型 `{ close(): void }` | `jobClose?.()` = 编译错误 |
| `routeStep` 缺 route / `goPolish` 用旧路径 | `RouteName` 联合 + `Record<RouteName, Step>` | 缺项/陈旧字面量 = 编译错误 |
| `project.output_dir` 在响应里不存在 | API 响应接口的**必填**字段 | 字段缺失 = 编译错误 |
| `cutAll` 多包一层 body | `cutAll(pid: string, body: { ranks: number[] })` | 传错形状 = 编译错误 |
| 字段从 `duration` 改成 `end` | 只暴露 `end: number` 的 `Event` 类型 | 读 `.duration` = 编译错误 |
| `danmaku_count` / `rendered_file` 缺失 | 同上（必填字段） | 静默退化的那一类被挡住 |

**必须诚实标注的两条限制**：

1. **`parseFloat(x) || 1` 这类"0 是合法值"的坑，光加 `number` 标注抓不到** —— 得配 lint（禁止用 `||` 做默认值 / `no-unnecessary-condition` 之类）。TS 不是银弹。
2. **D 类漂移只是减轻**：类型能保证"两边形状一致"，但保证不了"两边**行为**一致"（比如 `overlayStops` 的空白串过滤规则）。那件事的真正防线是 `check-overlay.mjs` 这类**对拍**，而不是类型。

## 5. TS 能拦住的六个用户可见事故

按"值不值得迁"排序，这些都是生产/用户可见的：

| 事故 | 后果 | 能拦住的类型 |
|---|---|---|
| `evDur is not defined`（在 `templates/index.html`） | **每次渲染都抛**，子进程死掉、任务挂 30 分钟零日志 | 作用域检查：`Cannot find name 'evDur'` |
| `project.output_dir` 不存在 | init / 去静音 / 渲染 / 预览**全都作用在错目录** | 响应接口必填字段 |
| `cutAll` 双层 body | "一键全切"**静默切了全部**（破坏性误操作） | 函数签名 |
| `watchJob()` 返回对象当函数用 | 三个视图永久"运行中" | 返回类型 |
| 同步接口当成任务 | 点一次"去静音"三个按钮永久锁死 | 返回类型（无 `id` 的同步调用） |
| 路由路径是旧版 | "点精修跳回首页" | `RouteName` 联合 + 类型化路由 |

⚠️ **注意第一条**：`evDur` 这个最有说服力的例子**在 `templates/index.html` 里** —— 而那是 Rust `include_str!` 的模板，**搬不进 webui 的 TS 工程**。也就是说"TS 能拦住的事故"里最严重的一条，恰恰在 TS 够不到的地方（详见 §6 第 ② 步）。

## 6. 建议路径（按性价比，四步）

1. **从 Rust serde 生成 TS 类型** —— 最高杠杆。权威形状本来就只在 Rust 侧，生成 `.d.ts` 一次覆盖"字段缺失/改名/类型不符"这一整类，而且是 A 类里最贵的那些（`project.output_dir`、`rendered_file`、`Event.end`）。
2. **把 `templates/index.html` 的 ≈2,315 行内联 JS 抽成独立文件** —— 它是**成片权威路径**，也是 `evDur` 事故的所在，还带有与 `engine.js` 手工同步的义务（`polish/README.md` 明写"先改模板、再移植过来"）。抽出来即可被 lint / 单测覆盖；**不抽的话 TS 永远覆盖不到这块**。构建时内联回去即可（模板仍可保持 `include_str!`）。
3. **`check-overlay.mjs` 纳入 git + npm script** —— 一条命令的成本，把"唯一的漂移防线"从"靠人记得跑"变成"能自动跑"。
4. **TS 按"纯逻辑优先"分阶段迁** —— `utils.js`、`api/*`、`stores/*`、`polish/engine.js`、`composables/useJobs.js` 先走（A 类重灾区、无 DOM、好验），那几个 1,000+ 行的 SFC 放最后（它们承载的 bug 多是 B/C 类，TS 帮不上大头，迁移成本却最高）。

配套（做第 4 步时一并）：`vue-tsc --noEmit` 进 CI；`|| 1` / `|| 0.3` 这类 falsy 默认值的 lint 规则。

## 7. 明确的限制与风险

- **TS 覆盖不到**：`templates/index.html` 的内联 JS（除非先做第 ② 步）；`webui/src/wasm/` 是 gitignored 的生成物（已带 `.d.ts`），迁 tsconfig 时要排除在 `include` 外。
- **迁移本身的成本集中在巨型 SFC**：1,245 行的 `<script setup>` + 模板里大量隐式 `any`，不宜一次性大改；按文件提交、`allowJs` 并存渐进推进。
- **没有 CI 的话收益打折**：`vue-tsc` 只在有人记得跑的时候才跑 —— 这与"靠人记得跑 `check-overlay.mjs`"是同一类失效模式。
- **TS 拦不住的大头依然存在**：B 类（31 条，含最危险的路由复用）与 C 类（26 条）。所以真正的稳定性来自于**测试 + CI**，类型只是其中一层。

## 8. 本次决定（记录）

- 用户决定：**暂不动代码**（不迁 TS、不改 `webui/`、不装依赖）。
- **测试 / CI 暂不建**；将来若要动，方向是 **TS / 类型**，不是先补测试基建。
- 本文档是评估的全部产出；仓库里**没有**因此产生任何功能性改动。
