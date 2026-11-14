# S4 收口结论书：**是否删掉第二条渲染链路**（canvas 出片 + DOM 预览腿）

> 立档：2026-10-01（目标 `goal-33caab05` Phase ②）。上游依据：`docs/plan/effect-pipeline-unification.md` §3.1
> （冻结闸门与前置条件表）、`docs/dhampir-base-handoff.md`（底座侧 D1 的复现与影响）。
> **本文件只做决策记录**：删代码要等**条件齐备 + 用户拍板**，两者缺一不删。

## 0. 结论

**NO-GO（现在不删）** —— 两个阻断项：

| # | 阻断项 | 性质 |
|---|---|---|
| ~~A~~ | ~~底座 D1（`overlay` 渐变色罩不出图）~~ → ✅ **2026-10-01 已排除，本条作废**：底座复现不出来，我们用自己的判据复核也是"正常出图"（`.workbuddy/p2-d1-verify.mjs`：窗口内平均差 39.33 / 窗口外逐字节相同 / `angle=45` 左上=color_a、右下=color_b）。原复现 JSON 无效（缺 `timeline.schema`；`end` 超素材帧数）| **已作废** |
| **B** | 「机器上没装底座 ⇒ 预览没有任何兜底」这个产品取舍需要用户拍板 | **产品决策**（不是技术条件）|

> ✅ **2026-10-02 更新：B 已拍板 = B2，并按 B2 口径部分执行** ——
> **保留 DOM 预览腿与对拍守卫**（预览在底座不可用时仍有兜底），只删"已无生产用途"的部分：
> ① **Rust 逐帧接缝整段删除**：`FrameRenderer` trait / `create_backend` / `render_chunk_sync` /
> `render/context.rs`（唯一消费者就是接缝）；`frame.rs` 保留（`extract_chunk_frames` 被测试夹具用）；
> ② `templates/index.html` 与 WebUI 预览腿**保留** —— 模板里的 canvas `renderFrame` 已是**惰性参照**
> （黄金帧冻结后没有任何生产路径执行它），但守卫从它身上扒函数、转译器注释拿它当行号锚点，
> 删它要单独一轮清理，本轮不动；③ §4 清单的第 1-3 步早已随 chrome 删除做完，第 4-6 步按 B2 作废，
> 第 7 步（文档扫尾）已按 B2 口径完成。

反过来，**技术上能程序化的两条前置条件本轮已经全部落地**（条件③ 变成不变量、条件④ 参照不再依赖兜底腿），
一旦 A/B 解除，删除本身是**机械操作**，清单见 §4。

## 1. 前置条件逐条现状（含证据）

| # | 条件 | 状态 | 证据 |
|---|---|---|---|
| 1 | 底座**随包分发**，开箱即默认通道 | ✅ | `cargo xtask package` 打进去 `dhampir/bin` + `preview/` + `VERSION`；源码树外解包实测 `polish doctor` 四个路径全在包内 |
| 2 | 底座不可用时有**明确分诊** | ✅ | `polish doctor` 的 `[dhampir]` 块（7 项）+ `/api/polish/dhampir-status` + 前端红字；不可用时出片会失败并报出原因 |
| 3 | **没有"只能靠兜底通道出片"的效果** | ✅ **已变成不变量** | `scripts/check-effect-registry.mjs::checkDhampirCoverage`：每个效果都必须有 `anchors.dhampir_case`，缺了就红并点名（自检 8 项含这条）。实测 11 个效果全有底座映射 |
| 4 | **删掉兜底腿不影响基线**（对照不依赖兜底腿） | ✅ **已解决口径问题** | 对拍新增**黄金帧模式**：`--capture-golden` 在还有兜底腿时冻结 5 个基线采样帧（`docs/golden/dhampir-parity/<样本>/`，共 2.61 MB），`--golden` 之后**完全不渲染兜底腿**。三样本实测：`demo` 15.07 / `portrait` 9.84 / `real` 11.76，判据全过；日志里不再出现"YekiTrim（chrome 后端…）"。⚠️ **口径**：只覆盖**基线**（5 个窗口外采样点），**事件窗口**的逐窗口判据仍需要兜底腿或一个能出参照的旧二进制 |
| ~~**0（硬前提）**~~ | ~~**底座 D1 先修好**~~ | ✅ **2026-10-01 已作废**（D1 撤回：原复现 JSON 无效，底座复核正常出图）| ~~见 §2~~（§2 保留作历史）|

## 2. 硬前提 A：D1 未修就删，会**从"画对了"变成"彻底没有"**

**现在是这样的**：`overlay`（纯色/渐变/`fade_in`/`fade_out`）在**底座那条腿上一整层不出图**（D1），
而兜底腿（canvas）**画得对**。也就是说：**今天用户看到的色罩与片头渐入，全靠在兜底腿上**。

**删掉兜底腿之后**：默认通道只剩底座 ⇒ 这些效果**在成片里彻底消失**（本仓明确**不做近似补偿** ——
拿 `flash` 之类凑色罩会掩盖问题，底座修好后还会变成第二份实现）。

**D1 的最小复现、已核对项与 4 条可疑点**都在 `docs/dhampir-base-handoff.md` §1（自包含：内联了一份
最小 doc JSON，不需要本仓的工程）。**A 的解除判据**：底座上用那份最小 doc 出帧，色罩**可见**
（与"删掉 overlay 轨"的帧不同），且 YekiTrim 侧跑 `node .workbuddy/run-parity.mjs demo` 时
overlay 窗口的像素差**下降**（基线会变 —— 那是**期望的**变化，按 `dhampir-parity-register.md` 的约定重标定）。

## 3. 产品取舍 B：没装底座时，预览怎么办

现状：底座不可用时，预览会退到 **DOM 预览腿**（页面上那行"近似预览 · <原因>"的提示就是它），
出片则**直接失败**并说明原因（不会静默降级）。三个选项：

| 选项 | 代价 | 我的建议 |
|---|---|---|
| **B1 接受"没底座就没预览"** | 装不上底座的机器上，精修页只剩红字；用户必须先把底座装上 | ✅ **推荐**（与"统一到一条链路"的目标一致；底座已随包分发，源码树开发者本来就该有底座）|
| **B2 只删出片那条腿，保留 DOM 预览腿** | 少删一半代码；预览仍与成片**可能不同**（正是这轮要消灭的"两份实现"） | 备选。若你更在意"没底座也能看着调"，选它 |
| **B3 给预览做"静态首帧"兜底**（新功能） | 又一份实现，收益低（只能看一帧） | ❌ 不建议 |

**只要你说一句"按 B1 办"（或 B2），B 就解锁**；A 仍要等底座。

## 4. 若 GO：删除清单（文件级，按顺序，每步可独立验证）

> 前提：A、B 都已解除。**一次一件事**，每步跑完该步的验证再进下一步。

| 步 | 改什么 | 验证 |
|---|---|---|
| 1 | `cli.rs`：`polish render` 去掉 `--renderer`（或只保留一个"已停用"的报错引导，指向 doctor） | `cargo test -p yekitrim-cli`；`yekitrim polish render <样本> --renderer chrome` 必须报**明确**的"该通道已删除"而不是"未知参数" |
| 2 | `render/pipeline.rs`：删 chrome 分支；`render/backend.rs`：删 `RenderBackendKind::Chrome` + `chrome.rs`；`render/mod.rs` 收敛出去 | `cargo check --workspace --all-targets` 0 warnings；`.workbuddy/run-parity.mjs demo --golden …` 仍全过（这一步之后对拍只能走黄金帧 ✓）|
| 3 | **用户可见的逃生提示**（别漏）：`commands.rs` 的 doctor 尾句"也可以先用 `--renderer chrome` 出片"、`dhampir.rs` 的 3 处"装 Node，或改用 `--renderer chrome`" | 全文 grep `--renderer chrome` 只剩本决策文件里的历史说明 |
| 4 | `templates/index.html`：整份删除（它的**唯一**消费者是 `server.rs:322` 的预览/出片页面）；`server.rs` 的 `INDEX_HTML` 与 `inject_config_tag` 一并清；`templates/cover.html` **必须保留**（封面另开一条，见 `cover.rs`） | `cargo test -p yekitrim-polish --lib render`；`yekitrim polish cover` 仍出图 |
| 5 | `webui/src/polish/engine.js` 与 DOM 预览腿（`PolishPreview.vue` 里那层自绘）+ `webui/src/api/polish.js` 的无用端点 | `npm --prefix webui run build`；Dhampir 预览仍能播（这是唯一剩下的预览）|
| 6 | 只为"两份实现一致性"存在的守卫：`check-caption-wrap` / `check-overlay` / `check-handheld` / `check-sticker-anchor` / `check-stage-backdrop` **逐条确认后**再删；**保留** `check-cover-stage`（封面）与 `check-preview-ab`（守的是**保留下来的** Dhampir 预览的 A/B 预滚）| `node .workbuddy/gates.mjs`（gates 里逐条列着这些脚本，删要同步改）|
| 7 | 文档一次性改完：`effect-pipeline-unification.md` §3/§4 的"两条通道"叙述、`render/AGENTS.md` 的"出片有两条通道"、`webui/src/polish/AGENTS.md`、根 `AGENTS.md` 的那段、`yekitrim-integration.md` | `python3 scripts/check_module_map.py && python3 scripts/check_module_boundaries.py` |

**回滚点**：每一步一个 commit ✓；第 4、5 步（模板/前端）是"删代码"里最不可逆的，
建议在它们之前先打一个 tag ✓。

## 5. 什么时候该回来重看这份结论

1. **底座 D1 修好**（A 解除）⇒ 立刻能进第 1 步；
2. **用户对 B 拍板**；
3. 新的"只能靠兜底通道"的效果被加进来 —— **不用等重看**：条件③ 那条不变量会**当场打红** ✓；
4. 未来若真要**自研渲染器**（不依赖底座）：那时"第二条链路"会以新形态回来，这份结论的前提（底座是唯一目标形态）就要重估。

## 6. 与交接材料的关系

- **底座侧**：只有 D1 挡住我们（D2/D3 与 YekiTrim 无关，D3 已自行修正）；材料与跟进见
  `docs/dhampir-base-handoff.md`（可直接贴给对方会话）与 `dhampir-parity-register.md` §四。
- **本仓侧**：本文件 + `effect-pipeline-unification.md` §3.1 是删除的**全部依据**；
  本轮把条件③/④ 程序化之后，这两份文档里剩下的待办只有 A 与 B。
