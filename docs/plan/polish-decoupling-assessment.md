# polish 精修模块：解耦与整合体检报告

> 立档：2026-10（**只做体检，未动一行代码**）。范围：`crates/yekitrim-polish` / `webui/src/polish` /
> `webui/src/components/{PolishPreview,TimelineEditor}.vue` / `webui/src/views/PolishView.vue` /
> `tools/polish-to-dhampir.mjs` / `spec/effects.json`。
> 相关：**`docs/plan/effect-pipeline-unification.md`（已有蓝图，S1–S3 已完成）**、
> `docs/plan/s4-second-chain-removal-decision.md`（S4 决策记录）、
> `docs/plan/polish-enrichment-proposal.md`（R1–R4 红线）。
>
> **本报告的用途**：把"加装一个功能要改 N 处、漏一处不报错"的**实际形状**量化出来，
> 供后续排期。**不含任何代码改动。**
>
> 方法：两轮独立代码审计（重复实现清单 / 前端数据流与状态耦合），全部结论带 `file:line`，
> 并对审计提出的三条"文档漂移"主张逐条**复核过**（见 §6）。

---

## 0. 一句话结论

**目标架构（"增删效果只改一处"）已经设计好、并且已经建成了大约 60%。**
`spec/effects.json` 是唯一权威、`check-effect-registry.mjs` 会点名漏改处、前端四张表与 LLM 提示词
**已经改成派生**（S1–S3 ✅）。**剩下的病根不在"词汇表没建"，而在 S4 未完成 ——
预览与出片仍是两条渲染链路，于是同一算法平均有 4 份实现、靠 30 个漂移门禁勉强钉住。**

---

## 1. 顶层事实：一个效果今天要"被知道"多少次

`effect-pipeline-unification.md` §1 记下的原始形状是 **15 处**。现状（本报告实测）：

| 分类 | 处数 | 门禁覆盖 |
|---|---|---|
| 已被门禁覆盖（含 3 个"**不该手改**"的派生点） | 15 | ✅ `check-effect-registry.mjs` |
| **无单一权威**（真正的剩余成本） | **4** | ⚠️ 靠对拍脚本间接兜 |

### 1.1 已经被门禁管住的（跑到绿即可，不必人肉记）

| # | 位置 | 角色 |
|---|---|---|
| 1 | `spec/effects.json` `effects[]` | **权威**，先改它 |
| 2–5 | `spec/effects.json` `anchors.{dhampir_case,canvas,dom_preview,post_span}` | 四个锚点（`dhampir_case` **不许 null**，S4 条件③） |
| 6 | `crates/yekitrim-render-core/src/types.rs` `enum Event` | 类型真源（变体） |
| 7 | 同上 `Event::default_end` | 默认跨度 |
| 8 | `crates/yekitrim-polish/src/process/fx.rs` 两张表 | 时长上限 / 默认时长 |
| 9 | `tools/polish-to-dhampir.mjs` `case '<id>'` | **加效果的唯一必改处之一** |
| 10 | `templates/index.html` `handlers.<id>` | canvas 参照（**可 null**） |
| 11 | `webui/src/polish/engine.js` `get<Camel>` | 预览求值器（**可 null**） |
| 12 | `docs/polish-events-reference.md` | 文档提一句 |
| **13** | `TimelineEditor.vue` 四张表 + 两个 UI 分组 | ⚠️ **不该手改**：全部 `Object.fromEntries(EFFECTS…)` 派生 |
| **14** | `llm/segment.rs` 提示词两份清单 | ⚠️ **不该手改**：`include_str!(".../spec/effects.json")` 派生 |
| **15** | `TimelineEditor.vue::LANE_DEFS` 的 `types:` | ⚠️ **不该手改**：`LANE_TYPES('<lane>')` 派生 |

### 1.2 没有被任何单一权威管住的四处（★ 本报告的核心）

| # | 位置 | 为什么是问题 |
|---|---|---|
| **A** | `TimelineEditor.vue` 属性面板的类型专属控件 | 手写 `<template v-else-if="selected.kind==='event'">` 分支；加字段时**门禁一个字都不管** |
| **B** | `engine.js:293` `INDEXED_TYPES` 桶清单 | 不在词汇表门禁里；2026-10-01 那次"预览整块消失"事故的**直接成因** |
| **C** | `LANE_OF_TYPE` —— **四份同集合** | `engine.js:304` · `index.html:300` · `polish-to-dhampir.mjs:2704` · Rust `Lane::of_event`（`project/config.rs:994`） |
| **D** | `templates/index.html` 的 canvas handler 本体 | 锚点存在性被门禁检查，**实现内容**不被检查 |

> ⚠️ **词汇表只管"事件类型"，不管"事件字段"。**
> 加一个新字段（如 `layout_angle_deg`）**没有任何门禁保护**，要手查 6 处：
> Rust serde `types.rs` + `legacy.rs::upgrade_event_times` + 模板 + `engine.js` + 转译器 + 面板控件 + `ProjectFile`(`config.rs`)。
> **这是"加装功能"当前最贵的一类改动。**

---

## 2. 重复实现清单（按"份数 × 保证强度"排序）

> **权威列的含义**：模块卡之间对"谁是权威"**本身就有分歧**（模板同时被称作"出片权威路径"与
> "参照实现…'改模板 = 改出片'这句话不再成立"）。代码不解决分歧处标 **争议**。

### 2.1 最危险：份数多 **且** 保证弱

| 算法 | 份数 | 位置 | 保证 | 漂移症状（原文） |
|---|---|---|---|---|
| **区域矩形 / 九宫格** | **4** | `engine.js:144` · `types.rs:1090` · `index.html`（**手抄表**） · `polish-to-dhampir.mjs`（`1/6`,`5/6`） | ⚠️ **最弱（结构上）**：`check-grid-parity.mjs:13` 自述断言的是"**四份口径互相一致**，**不是'某一份对了'**" —— **没有权威**。容差 0.02% 本身是合理的（脚本 `:14-15` 说明 0.01% 来自"取整 vs 不取整"、真漂移"会远超它"）；**弱点在于判据是相对而非绝对**：四份**一起**漂到同一个错值时报绿 | "同一个物理点在**四处各写了一遍**，其中模板那张是**手抄的**（历史上就它先漂）" |
| **贴纸锚点 + 盒尺寸** | **6** | `index.html:995` · `engine.js` · `canvas.js`(SSOT) · `timeline.rs` · `PolishView.vue` · `polish-to-dhampir.mjs` | 中（`check-sticker-anchor.mjs` 64 项覆盖全部 6 份） | "manual `98% 50%` 在 1920 宽下参照钳到 **1608**，这里原先**不钳**、给 **1741.6** ⇒ 预览看着在框内、成片甩到框外（101.6px）**且不报错**" |
| **手持摇摆** | **4** | `types.rs:758` · `index.html:622` · `engine.js:1647` · **`polish-to-dhampir.mjs:1557`** | 中 | 卡片写"改常数**三处**必须同步" —— **转译器是没被点名的那第 4 份** |
| **动效预设（39 档）** | **3** | `polish-to-dhampir.mjs::ANIM_PRESETS`（权威） · `engine.js:657` · UI 下拉 | 中（`check-anim-presets-parity.mjs`） | ⚠️ **跨命名约定漂移**（snake_case ↔ camelCase），机械 diff 显示"处处不同"、人眼 diff 不现实 |

### 2.2 份数多但保证强（**对照组：份数不决定风险**）

| 算法 | 份数 | 保证 |
|---|---|---|
| **运镜倍率**（特写1.8/近1.5/中1.3/远0.55） | **5** | ✅ **最强**：`check-camera-zoom-parity.mjs` 逐值钉五处 + **8 处变异自证** |
| **底色档 / fade 档** | 4 | ✅ `check-overlay.mjs`（77 项） |
| **字幕折行** | 2 | ✅ `check-caption-wrap.mjs`（21 项 + 4 变异） |

> 📌 **结论**：运镜倍率有 **5 份**却是最安全的，区域矩形有 **4 份**却是最危险的。
> **风险由"有没有权威 + 判据强不强"决定，不由份数决定。** 整改该优先补权威，而非无脑减份数。

### 2.3 其余（含跨仓手工移植）

| 算法 | 说明 |
|---|---|
| **字幕折行 / 覆盖层 / 阶段摆位 / 衬底 / 弹幕窗口 / 卡拉OK / 文字倾斜 / 逐字动效 / 缓动曲线** | 各 2–4 份，均有对拍门禁 |
| **缓动 `back_out3` / `pow2Out`** | ⚠️ 本仓 3 份 **+ Dhampir 外部仓 1 份**（`text_motion.rs`）。`engine.js:1397` 自述"**逐字照抄** Dhampir 的 `back_out3`" |
| **`curve.rs::channel_from` / `text_perchar_anim.rs::charTransformAt`** | ⚠️⚠️ **预览在手工复刻外部仓的 Rust 内部实现**（`engine.js:1012-1015, 1348-1355`）。**本仓无法用门禁钉住外部仓** —— 底座一改，预览静默漂移 |
| **`shake.intensity` 单位** | 4 份消费者 + **LLM 提示词写的是错的单位**（`llm/segment.rs:96` 写 `0.5`，实际是像素、默认 14） |
| **字体栈** | 权威 = `render/server.rs::fallback_stack`，4 处消费 |

---

## 3. 前端状态耦合（"改一个字段要同时想两件事"的来源）

### 3.1 三份镜像 + 一份草稿

| 变量 | 位置 | 语义 |
|---|---|---|
| `cfg` | `PolishView.vue:740` | 内存镜像（**可含未落盘改动**） |
| `persistedCfg` | `PolishView.vue:1209` | **保证已落盘**（Dhampir 热重载**只认它**） |
| `persistedSubs` | `PolishView.vue:1221` | **保证已落盘**的字幕（字幕住 `data/subtitles.json`，**不在 toml 里**） |
| `subs` / `dm` | `PolishView.vue:741-742` | 字幕 / 弹幕镜像 |

`TimelineEditor.vue` 内部还有**自己的一份**（`segments`/`subtitles`/`muteRanges`/`edlClips`，`defineExpose` `:3092`）。

### 3.2 写入路径：两层防抖，**两个信号不许合并**

```
改动 → markDirty (:2009) → 60ms 节流 → emit('preview')  ← 内存镜像，不落盘 (:1435)
     → touch() → 250ms (:1974) → flushSave() (:2112) → PUT → 成功才 emit('saved') (:2163)
     → onEditSaved (:1427) 更新 persistedCfg/persistedSubs
     → done 挂 onceSaved (:2065) → 1500ms 合并防抖 (:1465) → POST /api/polish/process
```

⚠️ **`done` 必须排在 `saved` 之后**（`:2049-2057`，2026-10-02 修过）：此前 `done` 抢在 PUT 前面，
只是被 `process` 前压的 1.5s 侥幸盖住。

⚠️ **写入门闸 `if (loadError.value) return`**（`:2118`）：读失败时**一个键都不许写** ——
否则一次读取失败 + 一次拖动 = 把磁盘上的分段/字幕清空。

### 3.3 "config 变了"有 **7 个消费者**

1. `PolishPreview` 的 `idx` computed（`:2231`，认 `props.config`）
2. `PolishPreview` 的 Dhampir 热重载 watch（`:3473`，认 `props.persistedConfig`，指纹含 **13 棵子树 + 字幕**）
3. `PolishPreview` 的 `activeStage` / 音效 `player.setEvents`（`:3453-3456`）
4. `TimelineEditor`（认 `props.dir` 重挂 + `reload()`；⚠️ **换片段刻意不走 key** `:714-721`）
5. `CoverDesigner`（认 `coverKey`）
6. `PolishView` 的 `styleWriteBlocked` 闸门（`:1819`）
7. `PolishView` 的 `onEditDone` → `process`（`:1458`）

三个重挂 key：`segKey`(LLM剪辑/初始化/去静音) · `coverKey`(仅封面) · `previewKey`(任何任务)。

### 3.4 `PolishPreview.vue` 一文件承载三层预览

| 层 | 实现 | 失效方式 |
|---|---|---|
| **DOM/GSAP 自绘** | Vue 模板 + `engine.js` 求值器 | `idx = computed(...)` `:2231` —— `props.config` 一变就重算 |
| **Dhampir wasm** | `<canvas id="dhampir-canvas">` `:9` + `dhampirEngine` `:316` | `open(project)`，工程**从磁盘转译**（`POST /polish/dhampir-project`）⇒ **引擎收不到 config 变化** |
| **A/B 换台源** | `dhampirSourceVideos` `:381` / `dhampirTwins` `:392` | 预滚 + 双元素绑源，跳屏蔽区用 |

**`persistedConfig` / `persistedSubs` 双份镜像的存在理由**：Dhampir 只认磁盘 ⇒ `props.config` 单独承担不了热重载。
这是"加一个影响出片的字段要同时想两件事（内存镜像 + 磁盘指纹）"的**结构性成本来源**。

---

## 4. 现状量化

### 4.1 文件规模（全部实测）

| 文件 | 行数 |
|---|---|
| `tools/polish-to-dhampir.mjs` | **7487** |
| `webui/src/components/PolishPreview.vue` | **3767** |
| `webui/src/components/TimelineEditor.vue` | **3323** |
| `crates/yekitrim-polish/templates/index.html` | **3130** |
| `webui/src/views/PolishView.vue` | **2770** |
| `webui/src/polish/engine.js` | **2571** |
| `webui/src/components/CoverDesigner.vue` | **2290** |
| `crates/yekitrim-render-core/src/timeline.rs` | **1526** |
| `scripts/check-effect-registry.mjs` | 595 |
| `spec/effects.json` | 336 |

**前端精修面合计 ≈ 15,054 行**（不含 `dist`）；四份渲染实现合计 ≈ **16,955 行**。

### 4.2 门禁规模：约 **30 条**契约/对拍脚本

`scripts/check-*.mjs`（约 20 条）+ `webui/scripts/check-*.mjs`（10 条）。

> 📌 **这些脚本既是安全网，也是症状**：每一条都对应一处"同一算法 N 份实现"的契约。
> 它们让漂移**能被发现**，但没有**减少**实现份数 ——
> 所以"加装功能"依然要同时改 N 处，然后跑 30 个脚本确认没漏。

---

## 5. 与既有蓝图的关系：S4 是终点，且只差一个产品取舍

`effect-pipeline-unification.md` §3 的阶段表：

| 阶段 | 内容 | 状态 |
|---|---|---|
| S1 | 词汇表 + 门禁 | ✅ |
| S2 | 打包带转译器/底座/词汇表 | ✅ |
| S3 | 前端四张表 + LLM 提示词改派生 | ✅ |
| **S4** | **收敛到一条渲染链路** | ⏳ **已定 B2 口径、部分执行** |

S4 的四个前置条件现状（`s4-second-chain-removal-decision.md` §1）：

| # | 条件 | 状态 |
|---|---|---|
| 1 | 底座随包分发 | ✅ |
| 2 | 底座不可用时有明确分诊 | ✅ |
| 3 | 没有"只能靠兜底通道"的效果 | ✅ **已程序化为不变量**（`checkDhampirCoverage`） |
| 4 | 删兜底不影响基线（黄金帧） | ✅ **已解决口径** |
| ~~0~~ | ~~底座 D1~~ | ✅ **已作废**（复现 JSON 无效） |
| **B** | **"没装底座 = 预览没兜底"的产品取舍** | ⏳ **唯一阻断项** |

> **B 已拍板 = B2**（保留 DOM 预览腿与对拍守卫，只删"已无生产用途"的部分），并按 B2 部分执行：
> Rust 逐帧接缝已整段删除（`FrameRenderer` trait / `create_backend` / `render_chunk_sync` / `render/context.rs`）。
> **`templates/index.html` 与 WebUI 预览腿保留。**

**⇒ 若要走完 S4，加效果的触点可从 5 处降到 2 处**（词汇表一行 + 转译器一个 `case`）。

---

## 6. 复核结果：审计提出的三条"文档漂移"主张

> 三条主张**逐条复核过**。两条成立、一条**被推翻**（审计只 grep 了 `webui/src`，漏了模板）。

| # | 主张 | 复核结论 |
|---|---|---|
| 1 | `PORTRAIT_SWITCH` 在代码里不存在，是三份卡都提到的幽灵 | ✅ **成立**。全仓 grep：仅命中 `webui/src/polish/AGENTS.md:109` 与 `tools/AGENTS.md:141`，**代码零处**。它已被 **`[stage.lanes]` + `LANE_OF_TYPE`** 取代（2026-10-06 改版）。**真实表是 §1.2-C 那四份** |
| 2 | `templates/index.html` 完全没有逐字/动效/文字倾斜实现 | ✅ **成立**。模板内 grep `ANIM_PRESETS`/`backOut3`/`perCharSpans`/`textTiltTransform` = **0 命中**。故动效预设是 **3 份**不是"四处实现" |
| 3 | `window.__CAM_STATE` 是不存在的幽灵机制 | ❌ **推翻**。它**真实存在**于 `templates/index.html:1553`（定义）+ `:920,946-947`（消费）。审计只查了 `webui/src` ⇒ 结论错误。**前端文档说"DOM 预览层共用 `window.__CAM_STATE`"这句仍需核对**（`PolishPreview.vue` 里未命中该符号，DOM 层实际走 `engine.js` 的 `videoFxAt`） |

> ⚠️ 第 3 条是**方法论教训**：本项目里同一机制常常**跨 `webui/` 与 `crates/*/templates/` 两侧**
> （正是本报告 §2 的主题）。**任何"全仓 grep"都必须同时覆盖这两侧**，否则会得出错误结论。

---

## 7. 整改建议（供排期，**不含具体代码改动**）

### P0 — 补权威，不删实现（风险最低、收益最直接）

| # | 动作 | 为什么优先 |
|---|---|---|
| 1 | **给"区域矩形 / 九宫格"指定权威**，把 `check-grid-parity.mjs` 从"互相一致"改成"**逐份对权威**" | 判据现在是**相对**的：四份一起漂到同一个错值时报绿。补上绝对权威后，这类"集体漂移"才会红 |
| 2 | **`LANE_OF_TYPE` 四份同集合**纳入 `check-effect-registry.mjs`（词汇表的 `lane` 字段本就是权威） | 直接消灭 §1.2-C；词汇表已有 `lane` 字段可派生 |
| 3 | **`INDEXED_TYPES` 从词汇表派生** | 它正是 2026-10-01"预览整块消失"事故的成因 |
| 4 | **给事件字段加门禁**（现在词汇表只管类型） | §1.2 末尾：加字段要手查 6 处，**这是"加装功能"最贵的一类改动** |
| 5 | 修 **`PORTRAIT_SWITCH` / `window.__CAM_STATE` 两处文档漂移** | 文档描述不存在的代码 = 下一个人会照着它改错 |

### P1 — 消除"跨仓手工移植"（最难，但风险最高）

`engine.js` 自述"逐字照抄" Dhampir 的 `curve.rs` / `text_motion.rs` / `text_perchar*.rs`。
**本仓的门禁钉不住外部仓** —— 底座一改，预览静默漂移，而 `check-text-motion-parity.mjs`
在 `../Dhampir` 缺席时**直接 skip**。

- 短期：确认 `check-*-parity.mjs` 在**底座在场**时真的被跑（现在会静默 skip）。
- 长期：这条只有 S4 能根治（预览直接用底座 wasm ⇒ 不再有"照抄"这回事）。

### P2 — 走完 S4（根治，需产品取舍已定）

B2 口径下 S4 的剩余动作见 `s4-second-chain-removal-decision.md` §4。
**本报告实测确认**：走完后加效果触点 = **2 处**。

### 明确不建议

- ❌ **不要为了减份数而"统一"贴纸的两个锚点基准**（左上角% vs 中心%）—— 那**是**功能，不是重复。
- ❌ **不要盲删 30 条对拍脚本** —— 在 S4 完成前它们是唯一的安全网；
  `s4-...md` §4 第 6 步明说"**逐条确认后**再删"。
- ❌ **不要新造第二套架构** —— S1–S3 已经建好了正确的骨架，本报告的病都在 S4 未完成这一段。

---

## 8. 环境备查

- ⚠️ **本机 `bash` 与 `pwsh` 均不可用**（Git Bash `couldn't create signal pipe, Win32 error 5`），
  本轮全部测量走 `read`/`grep`/`glob`。
- ⚠️ **`grep` 不给 `path` 参数会失败**（`rg: ./1.: 系统找不到指定的文件`）—— 必须显式传路径。
  这条同时是 §6 第 3 条误判的诱因之一。
- `.workbuddy/gates.mjs` 在本机**不存在**（已 gitignore），所以"跑一遍 gates"无法复现，
  只能逐条跑 `scripts/check-*.mjs`。

---

## 附：证据等级

| 等级 | 内容 |
|---|---|
| **一手实测** | 全部 `file:line` 引用、行数、`grep` 命中与否、`spec/effects.json` 与门禁脚本的实际读取 |
| **仓库文档（已核对正文）** | `effect-pipeline-unification.md` / `s4-second-chain-removal-decision.md` / `polish-enrichment-proposal.md` / 各 `AGENTS.md` |
| **⚠️ 已推翻** | "`window.__CAM_STATE` 不存在"（§6-3）—— 保留在此以说明方法论 |
