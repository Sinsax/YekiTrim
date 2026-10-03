# 效果管线的统一（增删效果只改一处）

> 立档：2026-10-01。范围：`spec/effects.json`（新）、`scripts/check-effect-registry.mjs`（新）、
> 前端三张表、`tools/polish-to-dhampir.mjs`、兜底通道（canvas + DOM 预览）、打包（`crates/xtask`）。
> 相关：`docs/plan/fx-type-consolidation.md`（**类型收敛的定稿计划**，本方案的 P1 就是它）、
> `docs/plan/dhampir-parity-register.md`（对拍台账）、`crates/yekitrim-polish/src/render/AGENTS.md`。

## 1. 问题的形状（实测，不是感觉）

一个精修效果（比如 `blur`）在仓库里要"被知道"的地方有 **15 处代码**：

| # | 消费点 | 现在怎么表达 |
|---|---|---|
| 1 | 类型真源 `yekitrim-render-core/src/types.rs` | `enum Event` 的变体 |
| 2 | 后处理 `yekitrim-polish/src/process/fx.rs` | **两张表**：时长上限 + 默认时长 |
| 3 | LLM 提示词 `yekitrim-polish/src/llm/segment.rs` | 提示词里的类型清单（**两份**：system 与 user） |
| 4 | 出片模板 `templates/index.html` | `handlers.<id> = function(…)` |
| 5 | 预览引擎 `webui/src/polish/engine.js` | `get<Camel>(idx, t)`（5 个历史命名不规则） |
| 6-9 | 前端四张表 `TimelineEditor.vue` | `FX_TYPES` / `TYPE_LABELS` / `DEFAULT_SPAN` / `SFX_CAPABLE`（+ `INTENSITY_TYPES` / `COLOR_TYPES`） |
| 10 | 转译器 `tools/polish-to-dhampir.mjs` | `case '<id>':`（15 个） |
| 11 | 求值器 `yekitrim-render-core/src/timeline.rs` | 平行实现（无生产消费者） |
| 12 | 服务端 `yekitrim-server/src/api/polish.rs` | 6 处白名单/区域校验 |
| 13 | 音效 `yekitrim-polish/src/render/sound.rs` | 按类型挂音效 |
| 14 | 守卫（4 个脚本） | 测试事件、DROP 基线、变异表 |
| 15 | 文档（`polish-events-reference.md` 等 13 份） | 人肉清单 |

**没有一处是权威**，所以每次增删都要人肉对齐 11+ 处，**漏一处不报错**：症状是"某个通道少画一个效果"
（`split`/`color_shift` 在底座通道就是 CLAMP 空转）、"LLM 永远不生成新类型"、"前端菜单里没有它"。

## 2. 目标架构（三层）

```
        ┌─────────────────────────── 唯一权威 ───────────────────────────┐
        │  spec/effects.json：id / 中文名 / 分类 / 参数 / **每通道的实现锚点** │
        │                     / 时长上限 / 默认跨度 / 可否挂音效 / 生命周期   │
        └───────────────┬───────────────────────────────┬───────────────┘
                        │ 派生（读它，而不是抄它）        │ 门禁（对齐 + 点名）
        ┌───────────────▼───────────────┐   ┌───────────▼────────────────────────┐
        │ 生成端：LLM 提示词 / 前端菜单与  │   │ scripts/check-effect-registry.mjs  │
        │ 标签 / 默认值 / 时长上限        │   │  82 项：锚点在不在 + 有没有没人认领  │
        └───────────────┬───────────────┘   │  的实现 + 数值三处同源 + 双向集合     │
                        │                   └────────────────────────────────────┘
        ┌───────────────▼───────────────┐
        │ 编译器：config → doc（转译器）  │  ← 加效果的**唯一必改处**（+ 词汇表一行）
        └───────────────┬───────────────┘
                        │ 同一份 doc
        ┌───────────────▼───────────────┐   ┌──────────────────────────────┐
        │ 预览 = 底座 wasm 引擎（默认）  │   │ 出片 = 底座二进制（默认）      │  ← 一条链路，天然一致
        └───────────────────────────────┘   └──────────────────────────────┘
        ┌───────────────────────────────┐   ┌──────────────────────────────┐
        │ 兜底预览 = DOM/GSAP（冻结）    │   │ 兜底出片 = canvas（冻结）      │  ← 见 S4，只保老集合
        └───────────────────────────────┘   └──────────────────────────────┘
```

**为什么这才叫"根本"**：现在 15 处里有 **7 处是为了让第二条渲染链路（canvas + DOM 预览）也会画这个效果**
而存在的。把渲染链路收敛成一条（config → doc → 底座，预览与出片同用），触点从 11 降到 **2**
（词汇表一行 + 转译器一个 `case`，底座没有原语时才需要动底座）。在收敛完成之前，词汇表 + 门禁保证
"一次就知道要动哪几处、漏了会红"。

## 3. 阶段与验收

| 阶段 | 内容 | 验收 | 状态 |
|---|---|---|---|
| **S1 词汇表 + 门禁** | `spec/effects.json`（**当时 15 个类型** ⇒ 类型收敛后 11 个）+ `scripts/check-effect-registry.mjs`（**当时 113 项** ⇒ 现 82 项，随类型增删浮动；判据本身不变）+ 接进常规门禁 | 故意把某个 type 从词汇表删掉/加进去，门禁**点名**剩下哪几处没跟 | ✅ 本轮 |
| **S2 发布形态修好** | 打包带**转译器 + 底座二进制 + 预览产物 + 效果词汇表**（`<程序目录>/dhampir/…`）；`find_translator` 补 `current_exe()` 回退（`find_binary` 早有，它没有） | 打一个包，在**没有源码树**的目录里 `polish render --renderer dhampir` 真出片 + `polish doctor` 全绿 | ✅ 本轮（`find_translator` 顺序改对；`xtask package` 带 `tools/` + `spec/` + `dhampir/`；真包实测：doctor 六项全在包内、从包里出片、`--strict` 按预期拒绝）<br>✅ **2026-10-01 复验（类型收敛 / D4 / D5 之后重做）**：`cargo xtask package --smoke-test` → 用 `tar -xf` 把 zip **真解到源码树之外**的临时目录 → 包内 5 项齐全（`dhampir/bin/dhampir.exe`、`dhampir/preview/engine.js`、`dhampir/VERSION`、`tools/polish-to-dhampir.mjs`、`spec/effects.json`）→ `polish doctor` 显示**四条路径**（二进制 / 转译器 / 预览产物 / **效果词汇表**）**全在包内** → `polish render --renderer dhampir` **退出码 0 / 14.2s / 出片 `export.mp4` 6.77MB**；`--smoke-test` 另外**起了一次打包好的 exe**（端口 18100）并验 `/api/health ok` + `/` 端出打包的 WebUI。zip sha256 **`db67e3f19055051c79bee39f7d314c1a68a3fbe4285926441a2730411811d293`**（21.8MB）。脚本：`.workbuddy/s2-package-verify.mjs`（可复跑）<br>✅ **顺带修掉一个产物一致性问题**：`<zip>.sha256.txt` 以前只在手工发版时写，于是它从 2026-09-22 起就没更新过（**撒谎的校验和比没有更糟**）⇒ 现在 `xtask package` 每次自动重写它（`write_checksum_sidecar` + 无依赖的 UTC 日期）<br>✅ **词汇表也进 doctor 了**（2026-10-01 Phase ③）：`Readiness.spec` + `spec_effects_path()`，三种情形三句话（包内路径 / 开发树路径 / 「内嵌（包内没有 spec/effects.json，用编译期那份）」）—— 三种都实测过，`.workbuddy/doctor-spec-verify.mjs` 可复跑 |
| **S3 生成而非对齐** | 前端四张表 + 转译器白名单 + `--strict` 的支持矩阵**直接读** `spec/effects.json` | 删掉手工清单后门禁仍绿（门禁退化为"读没读"）；改一个默认值只改词汇表 | ✅ 本轮（前端四张表 + 两个 UI 分组 / **LLM 提示词两份清单** / 转译器报告措辞都改成派生；门禁 110 项，含"必须派生"与**四份时长同源**：词汇表 ↔ `fx.rs` 两张表 ↔ Rust `Event::default_end` ↔ 前端派生） |
| **S4 收敛到一条链路** | 兜底通道（canvas 出片 + DOM 预览）**冻结**：新效果只进 doc；用 `--renderer chrome` 渲染含未实现效果的工程时**明确报错**（不许静默少画）。当底座随包分发成为常态后**删除**第二条链路 | 删掉兜底后：增删效果的触点 = 2；对拍基线不动 | ✅ 闸门 + 预览"近似"提示 + 删除条件成文（见 §3.1）｜**类型收敛已完成 D1/D3/D4/D5**（四类下线、CLAMP 归零、overlay 并泳道、vignette 接底座、色罩补 angle） |

### 3.1 兜底通道的冻结与"删掉第二条链路"的条件（S4）

**已落地（本轮）**：

1. **冻结闸门**：`cmd_render` 选 `chrome` 时，拿效果词汇表的 `anchors.canvas` 当支持矩阵，
   工程里用到不在矩阵里的效果 ⇒ **当场报错并点名**，两条路写进报错里（改用默认通道 / 去掉该效果）。
   实现见 `render/pipeline.rs`（`used_effect_types` + `unsupported_for_canvas`，
   词汇表**运行时**读包内或开发树那份、读不到用编译期内嵌那份；读不出来时**只警告不拦**）。
   端到端实证：把包内 `anchors.canvas.flash` 置 null ⇒ `--renderer chrome` 退出码 1 并点名 `flash`；
   还原后同一条命令正常出片（`· canvas 兜底通道：工程用到的 14 种效果都在它的冻结集合里`）。
2. **预览侧告知**：Dhampir 引擎不可用时（没装底座/版本不符），预览条上写"（当前是近似预览）" ——
   DOM/GSAP 那条腿是**第二条渲染链路**，看起来像但不等价。
3. **策略**：**新效果只加进 doc 那条链路**（词汇表 + 转译器 + 底座原语）。canvas/DOM 只保冻结前的集合，
   缺口由上面那条闸门**说清楚**，不再靠人记得。

**删掉第二条渲染链路的前置条件**（都满足才动手；任一不满足就先保持冻结）：

| # | 条件 | 现状 |
|---|---|---|
| 1 | **底座随包分发**，绝大多数用户开箱就是默认通道（S2 已把 `dhampir/` 打进去） | ✅ 已具备 |
| 2 | 装底座失败时有**明确的分诊**（`polish doctor` 的 `[dhampir]` 块 + `/api/polish/dhampir-status` + 前端红字） | ✅ 已具备（M3） |
| 3 | 关掉兜底之后**没有"只能靠它出片"的存量工程**（即：没人还在用 canvas 实现而底座不实现的效果） | ✅ **已程序化为不变量**（2026-10-01）：`check-effect-registry.mjs::checkDhampirCoverage` 要求**每个效果**都有 `anchors.dhampir_case`，缺了就红并点名（自检也加了这条用例，且自检本身已接进 `.workbuddy/gates.mjs`）。实测：11 个效果全部有底座映射 |
| 4 | 对拍三样本在"只走底座"下全绿（兜底腿删掉不影响基线） | ✅ **参照已能脱离兜底腿**（2026-10-01）：对拍新增**黄金帧模式** —— `--capture-golden <目录>` 在还有兜底腿时把 5 个基线采样帧冻结成小图（`docs/golden/dhampir-parity/<样本>/`，约 0.7 MB/样本，随仓交付），`--golden <目录>` 之后**完全不渲染兜底腿**（参照只读冻结帧）。实测三样本：`demo` 基线中位 **15.07** / `portrait` **9.87** / `real` **9.77**，判据全过；日志里不再出现"YekiTrim（chrome 后端…）"，`demo` 耗时从 ~90s 降到 **27.8s**。⚠️ **口径**：黄金帧模式只做**基线对照**（5 个窗口外采样点），**不量事件窗口** —— 事件窗口的逐窗口判据仍需要兜底腿、或一个能出参照的旧二进制（要连窗口一起冻，成本与收益不成比例） |
| **0（新增，硬前提）** | **底座 D1（overlay 渐变色罩不出图）必须先修好** | ⛔ 未满足。现在 overlay / 片头渐入是**靠兜底腿画对的**；删掉兜底腿而 D1 未修 ⇒ 这些效果从"兜底画对"变成**成片里彻底没有**（本仓明确不做近似补偿）。证据与复现在 \`docs/dhampir-base-handoff.md\` §1 |

**删的步骤**（条件齐了再执行，一次一件事）：
① `cli.rs` 去掉 `--renderer`（或只留 `chrome` 报错引导）→ ② `cmd_render` 删 chrome 分支与预览 server 的"出片用途"→
③ `templates/index.html` 保留**封面**用不到的部分删净、`engine.js` 与 DOM 预览腿删净 →
④ 删 `check-*.mjs` 里只为"两份实现一致性"存在的那几条（保留 cover 那几条）→ ⑤ 更新本表与台账。

> **📌 结论书（2026-10-01 Phase ②）**：`docs/plan/s4-second-chain-removal-decision.md` —— 现状 **NO-GO**（
> 阻断项：底座 D1 + 「没装底座就没预览」的产品取舍），但**两条能程序化的前提已经全部落地**（条件③ 不变量、
> 条件④ 黄金帧）；一旦解除，删除清单（文件级 + 每步验证 + 回滚点）就在那份文件的 §4。

⚠️ **删之前还要定一个产品取舍**：现在"机器上没装底座"时，预览会退到 DOM 预览腿（条上写着"（当前是近似预览）"）。
删掉之后**预览没有任何兜底**（只有红字说底座不可用）—— 这条要用户拍板，不是技术条件。


**④ 类型收敛的铺路（本轮已做）**：老工程里含已下线类型时，加载会以 serde 的
`unknown variant \`stutter\`, expected one of …` 失败 —— 那句话对人没用。现在
`project/validate.rs` 的 `retired_effect_message()` 把它包成"该工程含**已下线的特效事件**（…）"
并给出两条路（旧版本删事件 / 直接编辑 `polish.toml`），**普通解析错误原样透传**（不误包装）。
于是 ④ 的动作变成纯机械：删类型 + 删各消费点，报错体验已经就位（`cargo test -p yekitrim-polish --lib project::`）。


## 4. 增删一个效果的新流程（S1+S3 之后）

1. 改 `spec/effects.json`：加一条（或把 `status` 改成 `retiring`）；
2. `node scripts/check-effect-registry.mjs` —— 它**点名**还剩哪几处没跟（含数值不一致）；
3. **自动跟上的**（不用动）：前端四张表与两个 UI 分组、LLM 提示词的两份清单、转译器的报告措辞、
   时长上限/默认时长的三处数字（`fx.rs` 两张表 + Rust `default_end` 由门禁钉住）；
4. **仍需手改的**：类型真源（`Event` 枚举）→ 后处理逻辑 → 预览锚点（`engine.js`）→ 模板锚点
   （`handlers.*`）→ 转译器 `case`（映射逻辑本身）→ 事件参考文档；
5. 跑常规门禁：`node .workbuddy/gates.mjs`（含本门禁与转译器守卫）；转译器改了要跑 `--mutate`。

**实测收益**（S3 本轮）：以前"改一个默认时长"要动 4 处（Rust `default_end` / `fx.rs` / 前端 `DEFAULT_SPAN` /
词汇表），现在**只动词汇表**，另外三处要么派生、要么被门禁钉死；"LLM 不生成新类型"这类漏改也消失了
（提示词是算出来的）。

**删除**也一样：把 `status` 改成 `retiring`（词汇表里保留一条"待删"记录），先按 `fx-type-consolidation.md`
的触点清单删干净，再把那条从词汇表里去掉 —— 门禁会保证"要么处处都在、要么处处都不在"。

## 5. 已知风险与"不做"的事

- **不重写求值器**：`yekitrim-render-core/src/timeline.rs` 是已删 wgpu 路径留下的平行实现、**没有生产消费者**。
  收敛的目标不是"三份实现合一"，而是**删掉没有消费者的那份**（S4 顺带做）。
- **不把底座的渲染搬进本仓**：底座是独立仓、有 GPU 出片与 wasm 预览；本仓要的是"**一份效果定义**"，
  不是"把它的实现复制过来"（那正是今天 canvas/engine.js 两份实现的老路）。
- **封面不走这条线**：封面的"设计器 = 出片 HTML/CSS 同源"是它自己的不变式（见
  `docs/plan/cover-dhampir-tradeoff.md`），词汇表只管**精修事件**。
- **S4 的前置依赖**：S2 必须先把默认通道在发布形态下做成真的可用，否则删掉兜底通道 = 没装底座的人
  完全不能出片。
