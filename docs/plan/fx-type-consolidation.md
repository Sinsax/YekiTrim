# 特效类型收敛与色罩合并（overlay）+ vignette 修复

> 状态：**计划已定稿**；**D1 已完成第一批（2026-10-01）**：`split` / `color_shift` 两个类型已从
> 类型真源 / 后处理 / 求值器 / 模板 canvas / `engine.js` / 前端派生表 / 转译器 / 词汇表 / 文档里删净，
> 样本工程同步摘掉这两条事件（`CLAMP` 归零：3 DROP / 0 CLAMP）。
> **D1 全部完成（2026-10-01 第二批）**：`zoom_bounce` / `stutter` 也已下线（四类全删，词汇表 15 → 11，CLAMP 归零）。
> **D3 完成**（overlay 并入「视觉特效」泳道，泳道成员按词汇表 `lane` 派生）；
> **D4 完成**（vignette 两端都修：canvas 出片补画暗角 + 转译器映射底座 `vignette` ⇒ `DROP` 从 3 条降到 **2**）。
> **D5 完成**（2026-10-01）：`overlay` 补 `angle` —— 字段（`Event::Overlay.angle`，serde 默认省略）+ **四处求值**（`timeline.rs` / 模板 canvas + `overlayGradientCss` / `engine.js::gradientCss` / `PolishPreview.vue`）+ 转译器换算（`overlayBaseAngle`：CSS 0°=向上 vs 底座 0°=向右 ⇒ `base = css − 90`；缺省 = 参照的 `to bottom right` = 135°）。
> 顺带修掉一个**被"底座不出色罩"掩盖的偏差**：转译器以前把方向**写死 0**（= 正右方），而参照是"右下"。
> **④ 类型收敛至此全部完成（D1/D3/D4/D5）**；D6（不做径向圆心）按原计划不做。
> 范围：类型真源 / 后处理 / 出片模板 / WebUI / 转译器 / 守卫 / 文档
> 相关：`docs/vtrim-integration.md`、`crates/vtrim-polish/src/render/AGENTS.md`、`tools/AGENTS.md`、`webui/src/polish/AGENTS.md`、
> **`docs/plan/effect-pipeline-unification.md`**（词汇表 + 门禁：加删效果先改 `spec/effects.json`）

## 1. 已确认的决策

| # | 决策 | 说明 |
|---|---|---|
| D1 | **删除** `split` / `stutter` / `zoom_bounce` / `color_shift` 四个事件类型 | 前两类在底座没有对应原语（split 无 shear、color_shift 无 sepia），后两类与 shake / 运镜重叠 |
| D2 | **不做旧工程兼容** | 老 `polish.toml` 含这些事件时**直接读不出来**（serde 硬失败）；不做读时丢弃/迁移 |
| D3 | **合并方向 = 读法 A，统一类型名保留 `overlay`** | 删掉 `color_shift` 这个"色罩"角色，它的能力并入 `overlay`；同时**删掉「覆盖层」泳道**（-1 条泳道），`overlay` 并入「视觉特效」泳道 |
| D4 | **`vignette` 走方案 A** | 本仓 canvas 出片补画 + 转译器从 DROP 改为映射底座 `vignette`（两端都修） |
| D5 | **色罩新增方向参数 `angle`** | 底座已支持 `overlay.shape`(0/1/2) + `overlay.angle`(0–360°)，本仓只需补字段与四处求值 |
| D6 | **不做**径向圆心/半径 | 两端都是固定居中；要偏心光晕需扩底座契约，是独立一件事 |

**唯一未决项**：无（径向圆心默认不做）。

## 2. 目标形态

| 维度 | 现在 | 改后 |
|---|---|---|
| fx 瞬时类型 | 11 种 | **7 种**：`shake` `flash` `blur` `vignette` `hue_shift` `noise` `pulse` |
| 视觉色罩 | `overlay`（独立泳道）+ `color_shift`（sepia，出片本就没接） | **一个 `overlay`**：纯色 / 渐变（**可设方向**）/ 淡入 / 淡出 |
| 泳道数 | 7（含「覆盖层」） | **6** —— `overlay` 并入「视觉特效」 |
| 删除类型 | — | `split` `stutter` `zoom_bounce` `color_shift` |
| `vignette` | 两个预览有、canvas 出片没有；底座腿 DROP | canvas 补画 + 转译器映射底座 `vignette` |

底座（Dhampir）本次**不需要改动**：色罩方向用现成的 `shape`/`angle`，暗角用现成的 `vignette`。

## 3. P1 类型收敛（无兼容版）

### 3.1 触点清单

| 层 | 位置 | 动作 |
|---|---|---|
| 类型真源 | `crates/vtrim-render-core/src/types.rs:184-236` | 删 4 个变体；连带 `start()`/`end()`/`set_end()`/`default_end()` 分支与 `:404` 那条 `match` |
| 后处理 | `crates/vtrim-polish/src/process/fx.rs:11-62` | `transient_duration_cap` / `transient_default_duration` / `event_type_name` 各删 4 条 |
| CPU 参考 | `crates/vtrim-render-core/src/timeline.rs:900-1020` | 删 `get_stutter_video`/`get_split_video`/`get_zoom_bounce`/`get_color_shift` 与 `FrameState`/索引桶字段（本文件无生产消费者） |
| 出片模板 | `crates/vtrim-polish/templates/index.html` | 删 4 类 `handlers.*` 与 canvas 求值分支；`LANE_DEFS.types`（`:2172`）删 4 项 |
| 前端 | `webui/src/polish/engine.js:27/201/236/339-354/497`、`webui/src/components/TimelineEditor.vue:509-555/1208`、`webui/src/utils.js:89`、`webui/src/components/PolishPreview.vue:1295` | 时长表、索引桶、`FX_TYPES`、`TYPE_LABELS`、`DEFAULT_SPAN`、`INTENSITY_TYPES`、`COLOR_TYPES`、`SFX_CAPABLE`、泳道配色、DOM transform 分支 |
| 转译器 | `tools/polish-to-dhampir.mjs` | 删 4 个 `case`（`split`/`color_shift` 本是 CLAMP 空转） |
| LLM 提示词 | `crates/vtrim-polish/src/llm/segment.rs:92/108` | 两份类型清单 + 示例 + 锁 prompt 的单测 |
| 守卫 | `scripts/check-vtrim-translator.mjs`（含 `MUTATIONS`）、`scripts/check-dhampir-drop-list.mjs`（DROP/CLAMP 基线）、`webui/scripts/check-handheld.mjs`、`check-stage-backdrop.mjs` | 跟着改；DROP 基线变动需在 PR 里说明原因 |
| 文档 | `docs/polish-events-reference.md`、`docs/vtrim-integration.md` §4.6/§5/§6、`tools/AGENTS.md`（"15 个 case"）、`webui/src/polish/AGENTS.md`（漂移清单） | 逐条同步 |

### 3.2 失败语义（无兼容 ≠ 难查）

加载链是 `crates/vtrim-polish/src/project/validate.rs:69-71`：`toml::from_str` → `upgrade_toml_value` → `serde_json::from_value::<ProjectFile>`；未知 `type` 会报 `unknown variant \`stutter\`, expected one of ...`。

- 在这个 `Err` 外**包一层人话**，例如：「该工程含已下线的特效事件（split / stutter / zoom_bounce / color_shift）—— 本版本不兼容这类工程；请用旧版本打开并删除该事件，或手工编辑 `polish.toml`」，并保留原始 serde 文本。
- **不做**"跳过该条继续读"。

### 3.3 LLM checkpoint 按"缓存失效"处理

`crates/vtrim-polish/src/llm/events.rs:224-228` 的 `load_windows`：解析失败 → `warn!` 一行 + 当作**无 checkpoint**（重算该窗口），不要让一份过期缓存把整趟 LLM 跑挂。这不是兼容，是缓存失效语义。`polish_events.status.json` 只存状态/计数，确认即可。

### 3.4 其余入口

- `PUT /api/polish/project` 收到已删类型 → 同一条 serde 错误 → 400（前端类型表已删，正常不会产出）。
- `duration → end` 的读时升级（`upgrade_event_times`）**保留**，与本次改动无关。

### 3.5 用户可见面

- 文档 + 发布说明写明「**本版本不兼容含这四类事件的旧工程**」，给两条自救路径：用旧版本删除该事件 / 手工删 `polish.toml` 里那几段。
- 可选、低价值：`ProjectFile.version` 从 `1` 抬到 `2` 作标记（`crates/vtrim-polish/src/project/config.rs:127/151`；该字段现在**只存不校验**），除非要做版本提示，否则不动。

**验证**：`cargo test -p vtrim-polish --lib project` / `--lib process` / `--lib llm`；新增**负向测试**（含 `stutter` 的 toml → 报错且文案点名该类型）；`node scripts/check-vtrim-translator.mjs --mutate`。

## 4. P2 `overlay` 并入「视觉特效」泳道（-1 条泳道）

- 类型不动，只是不再单独成轨。
- `webui/src/components/TimelineEditor.vue`: `FX_TYPES`（`:509`）追加 `'overlay'`（共 8 项）；删 `LANE_DEFS` 里那条泳道（`:518`）；面板仍按事件 `type` 分支（`:205`），`:1179` 那条"覆盖层不算瞬时"的提示继续有效。
- `crates/vtrim-polish/templates/index.html:2168-2176`：`fx.types` 追加 `'overlay'`、删 overlay 那行；`DEFAULT_DUR.overlay = 1.5` 与 `Event::default_end = 1.5` 已有，不动。
- **必须钉住时长语义**：`transient_duration_cap("overlay")` 走 `_ => INFINITY`（`process/fx.rs:24`），新增单测 `overlay_not_clamped_to_transient_cap`。

**验证**：`npm --prefix webui run build`；时间轴上手工加一条 overlay，跨度拖到 5s 不被夹。

## 5. P3 色罩方向 `angle`（"有方向的渐变"）

**底座现成能力**（不用改底座）：`overlay.shape` = 0 纯色 / 1 线性 / 2 径向，`overlay.angle` = 0–360°（`dhampir-core/src/effects.rs:150-164`）；着色器 `dir=(cos θ, sin θ)`、`t = clamp(dot(uv, dir) + 0.5, 0, 1)`、径向 `length(uv)*2`（`src/shaders/color_mask.wgsl:126-143`）。

1. `Event::Overlay` 加 `angle: Option<f64>`（`types.rs:239-253`，`serde(default, skip_serializing_if)`），缺省按 **45°** = 现在 canvas 的 `createLinearGradient(0,0,CW,CH)` 对角线 ⇒ 现有观感零变化。
2. 五处求值器同步：模板 canvas（`index.html:1952` 改为按角度算两端点）、模板 CSS、`engine.js`、`PolishPreview.vue`、`timeline.rs::get_overlay_state`（参考实现）。
3. **角度换算只许一处定义**：引擎 θ=45° ≡ canvas 对角线 ≡ CSS `135deg` ⇒ **CSS 角 = θ + 90**；写进 `webui/scripts/check-overlay.mjs` 对拍 + 两条变异。
4. 转译器：`angle: event.angle ?? 45`，删掉"底座有角度、V-Trim 没写"那行 NOTE。
5. UI：覆盖层属性面板加「方向」滑块（仅 `shape=linear` 显示），新建默认 45；**LLM 提示词不收录该字段**。

**验证**：`node webui/scripts/check-overlay.mjs` 全绿（含新用例与变异）。

## 6. P4 `vignette` 修复（方案 A）

1. `templates/index.html::renderFrame` 补画暗角（现在只有 GSAP/DOM 的 `handlers.vignette`，`:1016-1021`）。语义照现有唯一口径：`radial-gradient(ellipse at center, transparent 55%, rgba(0,0,0,0.5) 100%)` + 包络「窗口前 40% 保持、之后线性降到 0」（`render-core/src/timeline.rs:880-897`，已有单测 `vignette_matches_js_hold_and_fade`）；用 `createRadialGradient` + 黑 0.5 alpha 实现，**不引入新字段**。
2. **叠放次序定死**：先确认 DOM `#flash-overlay` 相对字幕/贴纸的层级，canvas 按同一层级画。
3. **新增漂移守卫** `webui/scripts/check-vignette.mjs`（照 `check-handheld.mjs`：从源码扒 `getVignette` 逐值对拍 + 变异自证），对拍模板 canvas ↔ `engine.js`（↔ Rust 参考）。
4. 转译器 `tools/polish-to-dhampir.mjs:1947-1957`：DROP → 用 `placeEnvelope` 发一条**调整图层**挂底座 `vignette`（`strength`/`radius`/`softness`，`src/shaders/color_mask.wgsl:111-120`）。
5. **形状不同源 ⇒ 定标 + 如实标 CLAMP**：起草 `amount=0.5 / radius≈0.78 / softness≈0.22`，用逐帧像素 A/B（同工程、有/无该事件）验证；`docs/vtrim-integration.md` §4.6 表格与 DROP 计数同步。

**验证**：新守卫 + `cargo test -p vtrim-polish --lib render` + `cargo build -p vtrim-cli` + **真跑一次带字幕的出片**（模板经 `include_str!`，必须重建二进制）。

## 7. 验证矩阵

| 批次 | 出口判据 |
|---|---|
| P1 | `cargo test -p vtrim-polish --lib {project,process,llm}`；负向测试：含已删类型的 toml **报错且文案点名类型**；`node scripts/check-vtrim-translator.mjs --mutate` |
| P2 | `npm --prefix webui run build` + 5s overlay 不被夹（含新单测） |
| P3 | `node webui/scripts/check-overlay.mjs`（新用例 + 变异）全绿 |
| P4 | `node webui/scripts/check-vignette.mjs` 绿 + 真出片一次 + 逐帧 A/B |
| 收尾 | `python3 scripts/check_module_map.py && python3 scripts/check_module_boundaries.py`；`node scripts/check-dhampir-drop-list.mjs`（基线变动须说明）；`webui/dist` 与 `src` 一起提交 |

## 8. 风险与回滚

| 风险 | 对策 |
|---|---|
| 用户升级后**打不开**含已删类型的老工程 | 已接受（D2）；靠 §3.2 的明确文案 + 文档给自救路径 |
| 删类型把长片 LLM 续传炸掉 | §3.3：checkpoint 解析失败 = 缓存失效重算 |
| 用户 `.vtrim/config.toml` 的 prompt 副本仍生成已删类型 | 不再有读时兜底 → LLM 输出会落盘失败；发布说明里必须给"重建提示词副本"的一步 |
| fx 泳道里 overlay 被瞬时 clamp | P2 的单测钉住 |
| 五处求值器漂移（改漏不报错） | P3/P4 各配漂移脚本 + 变异自证 |
| 改了模板没重建 → "改完没反应" | 收尾强制 `cargo build -p vtrim-cli` + 真出片一次 |

**回滚**：P1–P4 各自独立成 commit，可单独回退。

## 附录 A：预览与出片为什么会"效果不一致"（排查清单）

同一个 core，但不是同一条流水线 —— 只有**求值**共用 `dhampir_core::compose`（`dhampir-wasm/src/timeline_host.rs:8`）。三条轴：

| 轴 | 预览（WebUI） | 出片 |
|---|---|---|
| 产物/版本 | 浏览器腿：`dhampir_wasm` + `web/engine.js`，`wasm_dirs()` 找（优先打包产物 `<程序目录>/dhampir/preview/`，否则源码树 pkg） | 原生腿：`dhampir.exe render`，`find_binary()` 找（`VTEDIT_DHAMPIR_CLI` → 打包 bin → 同层 target → PATH） |
| 输入工程 | `prepare_project(..., keep_muted=false)`：**压缩时间线**（客户端靠"那段没有帧"跳屏蔽） | `render_video_only`：`keep_muted=true`：**整条**，屏蔽交尾部 `cut_ranges` 按源时间剪一次 |
| 像素来源 | 浏览器 `<video>` seek + 每帧 `createImageBitmap`；`followPlayback=true` **不帧精确**；画布 = 窗口尺寸 | native ffmpeg 顺序解码、帧精确；目标 = 工程渲染尺寸 |

另外：字幕/弹幕的**栅格化在浏览器由 JS 做**（`engine.js` 的 `rasterizeLine` → `set_text_bitmap`，字体走 `@font-face`），CLI 用自己的 `--font-file`/`--font-dir`；贴纸在浏览器也由宿主交位图。底座自己的口径：真片段上两端"能对齐的是**形状与图层清单**"，逐像素只在两端吃同一份像素（合成语料）时才成立 —— 见 `docs/vtrim-integration.md:439-455`。

**排查顺序**：① 两条腿都是 dhampir 吗（`renderer` 白名单里有 chrome 兜底）② `dhampir.exe` 与 wasm 包的 mtime / `VERSION.project_schema` 是否同批（不同批先重建 wasm：`cargo clean --target wasm32-unknown-unknown` + `wasm-pack build`）③ 该效果落在转译报告的哪一档（**报告目前在服务端被丢弃** —— 见 P0 记录，需接出来）④ 剩下的"形状一致、像素不一致"属于已声明边界。

## 附录 B：与本次改动相邻、但不在范围内的项

- **转译报告在服务端被丢弃**：`crates/vtrim-polish/src/render/dhampir.rs::prepare_project` 用 `cmd.output()`，成功时 stdout（DROP/CLAMP/NOTE）直接丢掉 —— 预览与出片都看不到"哪些特效没进去"。建议单开一条：把报告随 `/polish/dhampir-project` 回包 + 写任务日志（`render::readiness`/`strict` 已有 `prepare_project_reporting` 的先例可照抄）。
- **打包时的版本对齐**：`cargo xtask package` 应把 `dhampir.exe` 与 wasm 预览包**一起出**并写同一个 `VERSION`，否则两条腿按各自的构建时间漂移（"shake 刷新后才正常"就是这一类）。
- **径向圆心/半径**（D6 明确不做）。
