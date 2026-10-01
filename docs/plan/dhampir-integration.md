# Dhampir 接入计划（V-Trim → 底座出片）

> 结论一句话：**本仓不再自带 GPU 渲染**（已清理：`vtrim-render-core` 的 wgpu 管线、`polish/src/render/native/`、
> `vtrim-wasm` + `webui/src/wasm/`、`/api/health` 的 `wgpu_render`、`POST /api/polish/render-frame`）。
> 底座（Dhampir）**当前是测试通道**（默认通道仍是 chrome），**远期目标是替代 chrome**。
> 接口唯一形状：`polish.toml` → 转译 → 底座工程 JSON → 底座出片。
>
> **本轮只做本仓这一侧**：不改 Dhampir 仓、不要求那边删任何东西（见 §七）。

## 〇、已决定（D1–D4）

| # | 决定 | 落地含义 |
|---|---|---|
| **D1** | 转译器要去掉 Node 依赖：`.mjs` → **可内嵌的 `.js`**，或**原生 Rust 移植**；**但 JS 工具脚本保留** | §3.1：**产品路径**去 Node（推荐原生 Rust），**JS 那份留作规格 + 对拍 oracle + 给底座侧测视频效果的工具**；两者的"逐字节一致"从一次性门槛升级成**常驻守卫** |
| **D2** | **chrome 仍是主线**；底座**只为测试**，**远期目标是替代** | ①默认通道/产物口径以 chrome 为准；②底座在 UI 上**明确标注"实验"**；③接口面**按最终会替代来做全**（进度/取消/产物/报告四件事）；④优先级可低于主线功能 |
| **D3** | 转译产物落 `<项目>/data/dhampir/` | 项目内相对路径（**不新增请求体路径字段 → 不动路径面三防线**），可复现、可复查、跟着项目走 |
| **D4** | `DROP > 0` **默认警告并继续**，`--strict` 才拒绝 | 理由与真实清单见 §3.4 |
| **D5**（本轮新增） | **转译器不探媒体**；"源视频竖"由**上层项目配置**给出 | §五第 1 条：需要**先补一个配置字段**（`video_info` 现在不落盘），由已有那次 ffprobe 写入。竖屏**只过滤运镜**（实测竖素材工程 0 事件、口径也是"竖屏实际只有字幕/弹幕/贴纸"；模板 A3 同样只屏蔽运镜 —— 额外删特效会**制造新的分叉**）。⚠️ **判据只能是源方向**：真实样本里竖素材工程的 `[canvas] orientation` 是 landscape |

## 一、已经就位的（交接产物）

| 环节 | 位置 | 状态 |
|---|---|---|
| 转译器（V-Trim 方言 → 底座契约） | `tools/polish-to-dhampir.mjs` | ✅ 本仓所有；**保留**（底座侧测试要用） |
| 曲线守卫（266 项 + 9 变异自证） | `scripts/check-vtrim-translator.mjs` | ✅ **保留**；将来还要扩成"Rust vs JS 对拍" |
| 接口规格（行为对照 / 三级记录 / 怎么扩） | `docs/vtrim-integration.md` 第一~八节 | ✅ 行号按本仓核过 |
| 事件类型 → 效果参数映射表 | 转译器逐事件 `switch` 的内联注释 | ✅ 11 个 `case`（2026-10-01 类型收敛删掉 4 类后） |
| 底座侧：`dhampir probe` / `render` / 对比回路 | Dhampir 仓 | 那边自带，**本仓不动它** |

**已实测**：`docs/wgpu-effect-reference/demo` 跑转译器，产物 JSON 与 Dhampir 原版**逐字节相同**
（sha256 `D21F5763…`）。这份对拍就是 D1 里"Rust vs JS"守卫的原型。

## 二、目标 / 非目标

**目标**
- CLI 与 WebUI 都能选"用底座出片"，并拿到与 chrome 通道**同级的过程可观测性**（进度/日志/取消/产物登记）。
- 出片差异**在转译时就报出来**（DROP/CLAMP/NOTE 进任务日志与产物报告），不靠事后对帧发现。
- 为"替代 chrome"铺路：**去掉产品路径的 Node 依赖** + 对拍基线 + §五差异清单。

**非目标**
- 不用底座替换 chrome；不把 wgpu 搬回本仓；不让底座读 `polish.toml`。
- **不为底座侧改任何东西**（脚本、默认值、文档都按"两边都留"处理，见 §七）。

## 三、接口面：四个接入点

### 3.1 转译（谁调、在哪跑、JS 与 Rust 怎么共存）

**产品路径分两步去 Node，但 JS 那份永远不删**：

| | 现在（底座=测试通道，P1/P2） | 远期（替代 chrome 之前，P4） |
|---|---|---|
| 产品路径 | `run_dhampir` 里 spawn `node tools/polish-to-dhampir.mjs <片段目录> --out <项目>/data/dhampir/project.doc.json` | 换成 **Rust 实现**（见下），`run_dhampir` **内部**替换，CLI/server/前端一个字不改 |
| 为什么现在可以接受 | dev/CI 机器本来就有 Node（前端门禁就是 `npm run build`）；这条通道当前只在开发/验证时用 | 产品要交给用户 → 不能要求机器上有 Node |

**Rust 实现（推荐的原生移植）**：`crates/vtrim-polish/src/render/dhampir/translate.rs`。
- ⚠️ **输入直接吃已加载的 `ProjectFile`** —— 那一整块手写 TOML 解析器（~150 行、只支持 polish.toml
  用到的那撮语法）因此**根本不需要移植**，这是原生移植最大的一笔省。
  代价：Rust 的 `toml`/serde 与手写解析器的宽容度不完全一致（多行数组、尾逗号、缺键的默认值），
  这类差异**必须**由下面的对拍暴露出来。
- 内嵌引擎那条路（`.mjs` → 能被引擎 eval 的普通 `.js`，去掉 `import/export`、把 `node:fs`/`node:child_process`
  换成注入的宿主对象，用 `boa`（纯 Rust，MIT）或 `rquickjs`（带 C 依赖 → 要过 `cargo xtask scan`）执行）
  仍然可行，**但它会让"给底座侧测试用的那份 JS"与产品那份分叉**，收益不如原生移植明确；
  除非出现"必须以 JS 为唯一实现"的硬理由，否则不改这条。

**JS 那份的定位（D1 的关键）**：`tools/polish-to-dhampir.mjs` + `scripts/check-vtrim-translator.mjs`
**保留在 `tools/`/`scripts/`**，因为：
1. 它是**规格与注释的载体**（100 KB 里大半是"为什么这么映射"的历史）；
2. 它是**底座侧测视频效果的工具**（Dhampir 那边可以继续用/引用它）；
3. 它是 Rust 实现的**对拍 oracle**。

**新增常驻守卫（把一次性门槛升级成门禁）**：`scripts/check-dhampir-parity.mjs`
- 取 `demo` + ≥3 个真实工程（clip-04 / clip-08 / clip-12）；
- 同一输入、同一输出目录下，**Rust 实现的产物与 JS 版逐字节相同**（sha256 对比）；
- 任何一边改了（映射、曲线、默认值）**两边都要改**，否则这条红 —— 这就是防漂移的那道闸。
- 变异自证照 `check-vtrim-translator.mjs` 的房规：改坏 Rust 侧一处、断言它必须红。

### 3.2 底座可执行文件的发现
优先级：`--dhampir <path>` → `VTRIM_DHAMPIR_EXE` → `[polish.dhampir] exe`（`polish.toml`）→
`dhampir` / `dhampir.exe`（PATH）→ 程序同目录。
- `vtrim polish doctor` 报四件事：**node**、**底座 exe**、**底座版本**、**`project_schema` 是否与转译器一致**。
- **版本/schema 不符要拒绝出片**（宁可报错，不要出一份静默走样的片子）。

### 3.3 出片（整片，不是"一帧一帧"）
⚠️ **底座不属于 `FrameRenderer` 那个 seam**（那是"给一批帧、回一批帧"；底座自带 probe + 整片 render）。
- 新增 `crates/vtrim-polish/src/render/dhampir.rs`：`run_dhampir(project_path, opts) -> Result<PathBuf>`，
  内部 = 转译 → `probe` → `render`。**唯一调用点** —— P4 换实现时只动它内部。
- `cmd_render` 按通道分支（与 `cut_ranges` 同一种"后处理一段"的地位），**不塞进 `create_backend`**；
  `RenderBackendKind` 保持只有 `Chrome`，底座另起一个枚举。
- **子进程纪律照 `render/AGENTS.md` 抄**：stdout/stderr **落文件不落管道**（底座还会拉起自己的 GPU 侧进程）、
  超时、取消 = **杀整棵进程树**、启动清扫孤儿、进度按**里程碑行**解析。
- 产物走 `unique_output_path`（`_1`/`_2`）+ 登记成"渲染产物"，**并记下是哪条通道出的**（D2）。
- 字体：`style.subtitle.font` → `vtrim_core::fonts::resolve_file` → `--font-file <真实路径>`；
  拿不到就退系统字体 + **warn**（底座没字体会判失败，别让它炸在深处）。

### 3.4 报告进流程（D4 的解释）

**三级记录**（`docs/vtrim-integration.md` §5）：
- **DROP** = 参照里有、底座**根本不表达**（这一条会丢）；
- **CLAMP** = 有对应概念但**表达力/口径不够**，转出来的与原意不同；
- **NOTE** = 信息（行为一致，或已按参照如实复现，含**参照侧本身的不一致**）。

**为什么 `DROP > 0` 默认不拒** —— 用真实跑出来的清单说话（demo 夹具，DROP 9 条）：

| DROP 实例 | 性质 | 拒了会怎样 |
|---|---|---|
| `[cover] 封面 0 行标题 + 字体/描边/暗角 13 个参数` | **有意不做**（封面另开一条通道） | 与画面无关的项，却让整片出不来 |
| `[style.subtitle] 入场动画 "cycle-5"` | **参照侧死配置** —— V-Trim 已于 2026-10-01 把 `entrance` 从 schema 移除（读时吞掉）| 同上 |
| `[vignette] 参照出片路径里根本没有它` | **参照侧"预览有、出片没有"** | 同上 |
| `[overlay] 颜色解析不了：["#140A28CC", …]` | **该修的缺口**（8 位 hex 底座不认） | "拒"只会让人改不动片，修不修是另一件事 |
| `[layout.liver] 没有可用聚焦点 / 锚点算不出来` | **工程侧数据缺失** | 一拒连片都出不了 |

四类里没有一类适合当硬门禁。所以：
- 默认**警告并继续**；DROP/CLAMP **逐条**进任务日志（`info` 级，不许 debug）+ 落 `report.json`，
  出片卡给一行摘要 `DROP 9 / CLAMP 3 / NOTE 16`；
- `--strict`（才拒绝）只给两个场景：① **CI 基线**；② 已验证工程做**回归**。
  与 `docs/vtrim-integration.md` §5 给下游的建议同一条路。

## 四、分期与验收

| 期 | 内容 | 验收 |
|---|---|---|
| **P0 手动可用** | 文档 + 命令行走通（**不改代码**） | `docs/vtrim-integration.md` §2 的命令逐条跑通，产出一份成片 |
| **P1 CLI**（形状见 `docs/plan/dhampir-cli-server.md` §一） | `polish translate`、`polish render --renderer dhampir`、`doctor` 的 `[dhampir]` 块、`--strict` | 缺 node/底座 → **可操作**的错（不静默回退）；两次出片 sha256 相同 |
| **P1.5 竖屏配置化**（§五第 1 条） | 上层配置落"源视频方向" + 转译器按它**只过滤运镜** | 竖素材工程：报告里有"已过滤 N 条 camera" + NOTE，且**字幕/弹幕/贴纸三类输出与"没加这道门时"逐字节相同**；横素材（含 demo）**逐字节不变** |
| **P2 server + WebUI** | 渲染接口收 `renderer`（标注"实验"）、就绪位、进度/取消、产物带通道 | 进度真的在动；取消后**无残留进程**；日志能看到 DROP/CLAMP 逐条；`webui/dist` 重建入库 |
| **P3 对拍基线** | "同一工程两条通道逐帧比"跑成门禁（口径与底座侧 `vtrim-compare.mjs` 对齐） | 三列（像素差/高频比/位移）有阈值；超阈值红且能指出是哪个事件 |
| **P4 去 Node** | Rust 原生转译 + **`check-dhampir-parity.mjs` 常驻绿** | 产品在**没装 Node** 的机器上能出片；parity 守卫绿；变异自证可红 |
| **P5 替代 chrome（远期）** | 切默认通道 | P3 基线稳定 + §五差异清零 + P4 完成 + 四件事验收（清单见 CLI/server 计划 §五） |

## 五、接入前要先解决的差异（`docs/vtrim-integration.md` §6）

### 1. 源视频竖 → **由上层配置给，不由转译器探**（D5，**建议排在 P1.5**）

**现状**：出片模板 `index.html:1413` 按 `CFG.vertical`（**源视频**竖，**与画布方向无关**）整条早退
`zoom=1 / handheld=0`；转译器判不了源方向 → 竖素材会"参照不动、转译后照动"。

**核实到的事实**（决定了怎么做）：

| # | 事实 | 出处 |
|---|---|---|
| 1 | `video_info` **现在根本不落盘**：只在管线内存里（`PipelineContext.video_info`，Stage 1 的 ffprobe 填），`detect` 独立跑还得再探一次 | `vtrim-core/src/types/mod.rs:200`、`detection/mod.rs:53` |
| 2 | **`polish process` 已经在探源尺寸**（给贴纸避让"主画面"障碍用）→ **最自然的写入点**，不用新增探测 | `process/mod.rs` `get_video_size(&config.source.video)` |
| 3 | **出片页每次 `build_html` 都在 ffprobe 源视频**（只为算 `vertical` 与画面构成），而预览 HTML 是**每个请求**重建的 | `render/server.rs:257-261` |
| 4 | **实测真实工程**：`便利店打工…`（源 **720×1280 竖素材**）→ `orientation = landscape`（**横画布 + 竖素材**）且 **0 个事件**；`弟弟吃自助…`（源 1920×1080 **横素材**）→ 25 个事件（其中 `camera` **15** 条） | 逐工程数 `type = "..."` + `ffprobe` |

**由事实 4 得到的口径（这条你要的判断）**：
- **判据只能是源方向**。真实样本里竖素材的工程 `orientation` 是 **landscape** —— 谁拿 `[canvas] orientation`
  去顶替源方向，正好在这个样本上翻车（这正是 `templates/AGENTS.md` 记的"竖素材进横画布"那个事故场景）。
- **竖屏实际只有字幕 / 弹幕 / 贴纸**（你的口径；实测那条竖素材工程 **0 个事件**也印证）⇒
  转译器在竖屏下**只过滤运镜**，**不为瞬时特效/`overlay`/`sfx` 做任何特殊处理**。
  理由不只是你的口径，还有**参照行为**：模板 A3 已定"竖素材**只屏蔽运镜**，其余一律通过"
  （`docs/plan/polish-vertical-canvas.md` §A3 + 决策表第 6 条，`VIS` 一刀切分支已删）。
  转译器若额外删特效，就会**在两条通道之间制造新的分叉**。
- **验收范围因此收窄**：竖屏的 P3 对拍只看 **字幕 / 弹幕 / 贴纸** 的窗口 + 确认"运镜确已过滤"，
  不必为竖屏下的特效差异纠结。

**做法**：
1. **补配置字段**（二选一，推荐甲）：
   - **甲（推荐）**：`polish.toml` 加 `[source] width / height`，由 `polish process` 那次已有的 ffprobe
     **顺手写入**。转译器入参就是片段目录，**只读 `polish.toml`**，不必往上找 `vtrim.toml`。
     **顺带收益**（可选第二步，别绑进 P1.5 的验收）：事实 3 那次"每次 `build_html` 都 ffprobe"可以不再探
     —— 但**必须先定失效口径**（视频被换掉时怎么知道？存 `mtime`/大小，或每次 `process` 覆盖），
     否则就是"改了没反应"的新来源。
   - 乙：`vtrim.toml` 加（Stage 1 写，素材事实归管线工程）→ 转译器要**往上找两级**，
     且精修工程与管线工程可能不同步。
2. **转译器按配置过滤运镜**（在 V-Trim 侧、转译器里做，**不必复刻那道门**）：
   - 源视频竖 → **不写** `camera` 相关的 Transform 关键帧与逐帧重放（含 handheld），
     报告里 NOTE：`源视频竖：本仓出片模板整条不做运镜，已过滤 N 条 camera`。
   - ⚠️ **只手过滤运镜**（见上"口径"）。
   - **配置缺失时（老工程没过 process）**：如实进 **NOTE** 且**不做过滤**（不知道就不动），
     并给可操作提示（"跑一次 `vtrim polish process` 即可补上"）。**不猜、不探**。
   - **它是安全网**：实测竖素材工程多为 0 事件，但**手工事件**（`.workbuddy/perf/vertical/vert-port`
     就有 1 条 camera）、老工程、以及将来 LLM 生成都可能带 → 这道门必须有，成本也只有一个 `if`。
3. **回归判据**：
   - 横素材工程（含 `demo`）的产物**逐字节不变** —— 这是"只加了一条门"的证明；
   - 竖素材工程：报告里能看到"已过滤 N 条 camera"，且**字幕/弹幕/贴纸三类**的输出与"没加这道门时"**逐字节相同**。

### 2. 其余按现状接受并进报告
字幕折行缩字未接（§6 第 2 条，长字幕观感差异最大）、（`split` / `stutter` / `zoom_bounce` / `color_shift` 已于 2026-10-01 下线、CLAMP 归零）、
`max_lines` 2 vs 3、高亮词 `.hl`、弹幕逐条滚动时长（要动契约）、`bg_mode` 模糊底、GIF 逐帧延迟表、
音效驱动画面、封面（有意不做）。

## 六、不做什么（防止漂移）

- 不在本仓重新引入 wgpu；不让底座读 `polish.toml`；不让转译器探媒体（D5）。
- 不让"测试通道"变成"偷偷在用"：底座出片必须在产物与日志上**标明通道**（D2）。
- 不把 DROP/CLAMP 藏进 debug 级。
- **不删 `tools/polish-to-dhampir.mjs` 与它的守卫**（D1；它们是给底座侧测效果 + Rust 对拍 oracle）。

## 七、Dhampir 仓：本轮不动（只记一条同步风险）

原计划里"让 Dhampir 删掉重复副本 / 改 `vtrim-compare.mjs` 默认路径 / 裁 `docs/vtrim-integration.md`"
**撤回** —— 那套脚本留着，Dhampir 那边靠它测视频效果。

**代价要写下来**：两边都留 ⇒ 可能漂移（改了一边没改另一边，症状是"我测的是旧版"）。所以：
- **权威在哪**：本仓 `tools/polish-to-dhampir.mjs` 是**权威**（守卫 + `docs/vtrim-integration.md` 都在这边）；
  Dhampir 那份当作**测试用的工作副本**，改映射时**先改本仓**、再同步过去。
- 真要根治漂移，只有两条路：Dhampir 通过 `--translator ../V-Trim/tools/…` **直接引用**本仓那份
  （它的参数本来就是可配的），或**反过来**把本仓这份指过去 —— ⚠️ 两条都**需要动 Dhampir 侧**，
  超出本轮范围，等那边愿意动时再定。
