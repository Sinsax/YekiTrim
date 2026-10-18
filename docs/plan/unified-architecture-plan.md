# 精修系统统一架构计划（三端收敛）

> 立档：2026-10-15。上游勘察：本轮实测（门禁跑批 + 两仓源码 + host_api 契约）。
> **本文件只做计划，不含改动。**

---

## 0. 结论先说

**"一个统一中间层"在 wasm ↔ 后端这一轴已经存在，而且不需要新建** ——
它就是 `tools/polish-to-dhampir.mjs`，产物 `*.doc.json` 同时喂给两端。

**真正没统一的只有 DOM 这条腿**，而它**不是"缺一层"，是"多了一条独立实现链"**。
更要紧的是：**DOM 缺的不是中间层，是一个求值器 —— 而那个求值器底座已经提供**。

| 事实 | 证据 |
|---|---|
| 转译器 = 唯一中间层 | `dhampir.rs`：三条用法（出片/短片段/单帧）**共用同一套「转译 → 调二进制」骨架** |
| wasm 与后端同源 | 底座**一份 WGSL**，wasm 与 exe 是**同一渲染器的两种编译产物** |
| **底座已暴露逐帧求值结果** | `dhampir_project_frame` → `FrameResult{frame, layers:[LayerView], overlay}` |
| `LayerView` 已含 DOM 需要的全部标量 | `opacity: f32` + `transform: TransformView` + `effects: Vec<EffectView>` |
| `OverlayView` 已含文字/弹幕落点 | `items:[{text,rect}]` + `danmaku:[{text,rect,lane,enter,exit}]`，`rect` 已归一化 |

**⇒ 结论：不必造中间层；把 DOM 腿接到 `dhampir_project_frame` 上即可。**

---

## 1. 现状：三端 × 六类东西的矩阵

### 1.1 六类东西各按什么组织（实测）

| 类别 | 组织依据 | 真源 | 走 `effects.json`？ | 扩展方式 |
|---|---|---|---|---|
| 视觉特效（flash/blur/vignette/noise/hue_shift/overlay） | 词汇表 | `Event` 瞬时变体 | ✅ | 声明式（`dhampir_rule`，5 个已走） |
| 运镜（camera） | 词汇表 | `Event::Camera` | ✅ | 半声明式（`keyframe_camera`） |
| 手持摇摆 / shake / pulse | 词汇表 | `Event` | ✅ | 手写 case |
| 贴纸 / 文字层（sticker/element） | 词汇表 | `Event` + `StickerConfig` | ✅ | 手写 case（49/38 行） |
| **字幕** | **Rust struct** | `types.rs::SubtitleStyle` | ❌ | 各引擎各写一遍 |
| **弹幕** | **Rust struct** | `types.rs::DanmakuStyle` | ❌ | 各引擎各写一遍 |
| 音效（sfx） | 词汇表 | `Event::Sfx` | ✅ | 不过底座，本仓混音 |

**关键：只有"事件类"被词汇表化了；"持续样式类"（字幕/弹幕）是另一套体系。**

### 1.2 六条「一份规则 N 份实现」契约（实测份数）

| 契约 | 实现份数 | 现状保证 |
|---|---|---|
| 覆盖层色档 | **4**（timeline.rs / index.html / engine.js / PolishPreview.vue） | check-overlay.mjs 77 项 |
| 字幕折行 | **2**（index.html / engine.js） | check-caption-wrap.mjs 21 项 |
| 贴纸锚点 | **5**（含转译器） | check-sticker-anchor.mjs 64 项 |
| 手持摇摆 | **3** | check-handheld.mjs 77 项 |
| 运镜倍率 | **5 处** | check-camera-zoom-parity.mjs 8 变异 |
| 九宫格 | **4** | check-grid-parity.mjs |

**⇒ 真正的维护成本在这里，不在效果清单。**（清单已由 221 点门禁解决）

### 1.3 三条输出腿，各自读什么

```
polish.toml ──┬──→ 转译器 ──→ doc.json ──┬──→ wasm 预览（OpenGL/wgpu）   ✅ 同源
              │                          └──→ 后端出片（dhampir.exe）     ✅ 同源
              └──→ engine.js（手抄第 2 份）──→ DOM 预览                    ❌ 另一份实现
```

---

## 2. 目标架构

### 2.1 核心判断：不新建层，改接法

```
                    polish.toml（唯一真源）
                          │
              tools/polish-to-dhampir.mjs        ← 已存在的中间层
                          │
                    <clip>.doc.json              ← 统一产物
                          │
        ┌─────────────────┼─────────────────┐
        ▼                 ▼                 ▼
   wasm 引擎         后端 exe          ★ DOM 预览（改为消费求值结果）
   （已有）           （已有）          dhampir_project_frame → 只套 CSS
```

**DOM 腿的改造原则：DOM 里不再有任何算法，只做机械映射。**

### 2.2 四环分工（明确谁负责什么）

| 环 | 内容 | 状态 |
|---|---|---|
| **环 1 · 能力清单** | `spec/effects.json`（效果/字段/通道锚点）<br>`spec/lanes.json`（**新增**：轨道本身） | 部分已有 |
| **环 2 · 转译** | `polish-to-dhampir.mjs` → `doc.json` | ✅ 已有 |
| **环 3 · 求值** | `dhampir_project_frame`（底座提供） | ✅ 已有，**DOM 未接** |
| **环 4 · 消费** | wasm 引擎 / 后端 / DOM | DOM 待接 |

**⇒ 计划的主体是"把 DOM 接进环 3"，不是造环。**

---

## 3. 分阶段计划

### P1 · DOM 腿接入求值层（★ 最高优先，收益最大）

**做什么**：`PolishPreview.vue` 的 DOM 腿改为消费 `dhampir_project_frame` 的返回体，
不再用 `engine.js` 自己算数学。

- `LayerView.transform` + `opacity` → 直接写进 CSS `transform` / `opacity`
- `OverlayView.items[].rect`（已归一化）→ 文字层定位
- `OverlayView.danmaku[].rect`（逐帧滚动位置）→ 弹幕定位
- `EffectView` → 交给 CSS filter / 或直接不画（让 wasm 层叠在其上）

**收益**：一次改动能同时消灭 **6 条契约里的 4–5 条**（色档/折行/贴纸锚点/手持摇摆/运镜倍率 —— 这些在 DOM 侧**全部不再需要算**）。

**风险**：DOM 腿现有能力（GSAP 交互、拖拽即时反馈）需保留。
**判据**：`check-overlay.mjs` 等脚本改为 **DOM ↔ wasm 逐帧对拍**（用同一 `frame` 号）。

**⚠️ 前置勘察（必须先做）**：
1. 拖拽期间（`onTimelinePreview` 60ms 节流、未落盘）求值器读的是磁盘工程 —— 需确认能否传内存工程
2. `fetch` 单帧的延迟能否撑住 60fps 预览（若不行动画期间走 DOM 局部、静止时对齐求值器）

### P2 · 控件契约硬化为门禁（低风险、独立收益）

把 `fields[]` 升为**强制完整声明**，并加门禁。

**五条硬规矩**：

| # | 规矩 | 门禁判据 |
|---|---|---|
| 1 | 面板里每个字段都必须来自 `fields[]` | 禁止 `.vue` 里手写 `<input v-model="selItem.*">`；从源码扒行为测试 |
| 2 | 每种 `ui` 一个注册组件 | `FieldControl.vue` 分派；加控件 = 加组件，不改面板 |
| 3 | 数值项一律走 `SliderField` | 沿用（夹取不吸附、清空不写模型） |
| 4 | 字段名 = 磁盘键名 | 已踩过 `perChar`/`per_char` 蛇形驼峰不匹配 |
| 5 | `ui` / `label` / `_why` 三键必填 | 门禁断言非空 |

**现状**：`autoFields()` 已实现派生（P1 已做）；缺的是**强制 + 覆盖复杂控件**（现靠 `skip` 表避让）。

### P3 · `spec/lanes.json` + 生成器（加新轨道的终极解）

**现状成本**：加一条新轨道 = 5–6 处
```
types.rs::Event 变体       ← Rust 编译期，必须手写
fx.rs / config.rs           ← 数值与默认
LANE_DEFS (TimelineEditor)  ← 前端泳道
LANE_OF_TYPE ×4              ← 四处按类型分派
effects.json                 ← 词汇表
+ 通道实现（转译器 case / engine.js / 模板）
```

**目标**：压到 **1 JSON + 1 通道实现**
```jsonc
// spec/lanes.json（与 effects.json 同构）
{
  "id": "my_track",
  "label": "我的轨道",
  "mechanism": "layer",          // effect | keyframe | layer | audio
  "event_variant": "MyTrack",    // Rust 枚举名（唯一手写处）
  "fields": [ /* 同 effects.json 的 fields 形状 */ ],
  "channels": { "dhampir": "...", "dom_preview": "..." },
  "ui": { "color": "...", "icon": "..." }
}
```

**生成**：Rust `Event` 骨架 + `LANE_DEFS` 条目 + 控件定义（复用 `autoFields`）。

**⚠️ 只在轨道数量真增长时才做** —— 生成器本身也要维护。

### P4 · 纯数据表代码生成（可选，低风险）

运镜倍率表、九宫格坐标 → 从一个 JSON 生成 Rust + JS。
**收益中，风险低**，可随时插入。

### P5 · 字幕/弹幕纳入声明式（长期）

`SubtitleStyle` / `DanmakuStyle` 目前是**独立 Rust struct + 各引擎手写副本**。
可考虑给它们也做字段级词汇表（复用环 1 的 `fields` 机制）。
**优先级最低** —— 它们字段少（各 ~10 个）、变化慢。

---

## 4. 不要做的（明确否决）

| 不做 | 理由 |
|---|---|
| **再造"统一所有输出"的中间层** | wasm↔后端已同源；真正缺的是 DOM 接求值器 |
| **"一个统一所有效果"的抽象层** | 四套机制产物不同（Effect/Keyframe/Layer/audio），硬统一 = 词汇表原话"为了统一而统一" |
| **把字幕/弹幕塞进 `effects.json`** | 它们是**持续样式**不是**时间事件**，形状不同 |
| **让 Rust 枚举由 JSON 无生成器派生** | 编译期类型；要么上生成器，要么保留手写 |
| **删掉 DOM 腿只留 wasm** | 会丢 GSAP 交互与拖拽即时反馈（除非产品决策要丢） |

---

## 5. 落地顺序与收益

| 序 | 做什么 | 收益 | 风险 | 前置 |
|---|---|---|---|---|
| **1** | **P1 · DOM 接入 `dhampir_project_frame`** | **最高**（6 契约消灭 4–5 条） | 中高 | 两条前置勘察 |
| 2 | P2 · 控件契约门禁 | 高（新字段零成本） | 低 | 无 |
| 3 | P4 · 纯数据表代码生成 | 中 | 低 | 无 |
| 4 | P3 · lanes.json + 生成器 | 高（新轨道 1 处） | 中高 | 轨道数增长时 |
| 5 | P5 · 字幕/弹幕声明式 | 低 | 中 | 最后 |

**核心判断**：**先做 P1 的前置勘察。** 它是唯一能把"4–5 份手抄实现"物理消除的改动，
收益远超任何新抽象层；而它的可行性**只取决于两个可测的问题**（拖拽期间的内存工程、单帧延迟）。

---

## 6. 判据设计原则（贯穿全程）

1. **每条契约要有对拍脚本**，且带**变异自证**（"一条不会红的用例不算用例"）。
2. **判据要跑真东西** —— `check-preview-engine.mjs` 的教训：静态读源码抓不住"桶丢了"这类崩溃。
3. **改判据要问"它还会红吗"** —— 本仓已栽过三次"判据自己烂了"（断言写死 15 个类型等）。
4. **新加的检查要抽成函数并在 `--mutate` 里一起调** —— 只写在顶层的话变异台根本不跑它。
5. **对拍口径要写明** —— 历史两套基线数字（活参照 / 冻结帧）混过，`real` 样本 9.77 vs 11.80。


## P1 前置勘察结论（2026-10-15 实测）

### 问题 1：拖拽期间（未落盘）能否把内存工程传给求值器？

**答：不需要传 —— 求值器在浏览器里，工程已在内存中。**

`dhampirEngine.evaluate(frame)` 是 **wasm 同步调用**（`engine.js:580`：
`return JSON.parse(this.mod.dhampir_project_frame(frame))`），
工程由 `dhampir_project_open(doc)` **一次性载入 wasm 堆**，之后逐帧查询**不再碰磁盘**。

⇒ 拖拽期间的 1290ms 不是"每次查询要转译"，而是**重新转译整个工程**（`POST /polish/dhampir-project`
读磁盘 `polish.toml`）。**要区分两件事**：
- **工程变更**（改一个值）→ 需要重新 open（贵，1290ms）
- **换帧查询**（拖播放头 / 播放）→ 0.05ms，**与磁盘无关**

而 DOM 腿现在正是**每帧自己重算**（`engine.js` 的 `cameraAt`/`videoFxAt`/`subsAt`/`danmakuAt`/`stickersAt`），
恰恰是求值器该做的事。⇒ **P1 可行，且不需要解决"内存工程"问题。**

### 问题 2：单帧查询延迟能否撑住 60fps 预览？

**答：能，余量极大（实测）。**

| 导出 | 实测耗时 | 60fps 预算 | 占用 |
|---|---|---|---|
| `dhampir_project_frame`（图层 + 覆盖层） | **0.0510 ms/帧** | 16.67 ms | **0.3%** |
| `dhampir_project_text_frame`（文字专用） | **0.1012 ms/帧** | 16.67 ms | **0.6%** |

样本：`.workbuddy/yekitrim-handover/final.doc.json`（54 轨、timebase 60/1、720 帧）。
500 次查询总计 25.95ms / 50.6ms。

### 附带确认：返回体确实含 DOM 需要的全部标量

```json
layer[0] = {
  "clip_id":"main-0", "source":"clip.mp4", "source_frame":598,
  "opacity":1, "frozen_for_transition":false,
  "transform":{"x":0,"y":0,"scale":1.604717,"rotation_deg":0},
  "effects":[]
}
```

`transform{x,y,scale,rotation_deg}` + `opacity` = **正是 CSS transform/opacity 的输入**。
`overlay.items[].rect` / `overlay.danmaku[].rect`（已归一化）供文字与弹幕定位。

### 一个必须记下的坑

`dhampir_project_text_frame` 在**未 attach 预览宿主**时返回
`error: "工程预览宿主尚未初始化，先调 dhampir_project_attach"`，
且 `items` 为空 —— **不报错、静默返回空数组**。
⇒ P1 实现时**必须先 attach**，否则文字层会静默消失（正是本仓最怕的那类失败）。

### 勘察用的临时脚本（可删）
`.workbuddy/probe-eval-cost.mjs` / `probe-eval-cost2.mjs`（已 gitignore 目录）

---

## 7. 一句话

**统一中间层已经有了（转译器 → `doc.json`）；wasm 与后端天然同源（同一渲染器）。
唯一没统一的是 DOM 腿，而它需要的不是新层 —— 是接上底座已经提供的逐帧求值结果
（`dhampir_project_frame`）。先做那件事，其余按需跟进。**
### 改动前基线（2026-10-15）

全部 exit 0：

| 门禁 | 状态 |
|---|---|
| check-overlay.mjs | ✅ |
| check-caption-wrap.mjs | ✅ |
| check-sticker-anchor.mjs | ✅ |
| check-handheld.mjs | ✅ |
| check-stage-backdrop.mjs | ✅ |
| check-preview-engine.mjs | ✅ 22 项 |
| check-preview-audio.mjs | ✅ |
| check-dhampir-wiring.mjs | ✅ 97 项 |
| check_module_map.py | ✅ |
| check_module_boundaries.py | ✅ 0 违规 |
### ★ P1 范围复勘（2026-10-15）—— 结论修正

勘察通过后我量了 DOM 腿的体量，**结论需要修正**：

| 文件 | 行数 |
|---|---|
| `webui/src/components/PolishPreview.vue` | **3781** |
| `webui/src/polish/engine.js` | **2572** |
| `crates/yekitrim-polish/templates/index.html` | 3131 |

**DOM 腿不是"一份可以丢弃的手抄实现"**，它承载了三类底座**没有**的东西：

1. **文字栅格化与字形**（`textTiltTransform` / `perCharSpans` / `animPreset`）——
   底座返回的是 `rect` + `color` + `outline`，**字形像素只有浏览器里才拿得到**
   （`engine.js` 头注释原话：「两端允许不同的只有字形」）。
2. **GSAP 交互层**（拖动即时反馈、选中态、遮罩）。
3. **39 档文字动效的逐字求值**（C 组 15 档 + D 组 4 档），DOM 侧已与底座对齐过（2026-10-13/14）。

⇒ **"DOM 只是重算了一遍数学"这个判断不成立。** DOM 重算的是
**数学 + 字形 + 交互**，其中只有**数学那一半**能由 `dhampir_project_frame` 取代。

#### 修正后的 P1 定义（增量，非重写）

**只把"数学那一半"接到求值器上**，保留字形与交互：

| 现在 DOM 自己算 | 可由求值器提供 | 处理 |
|---|---|---|
| 视频层 `transform` / `opacity`（运镜+手持+特效位移） | ✅ `LayerView.transform`/`opacity` | **换源** |
| 文字/弹幕**落点** `rect` | ✅ `Overview.items[].rect` | **换源** |
| 文字**折行** | ⚠️ 底座也折（`max_lines`/安全宽） | **对拍**，暂不换 |
| 文字**字形**（倾斜/逐字/竖排） | ❌ 底座不给字形 | **保留 DOM** |
| 覆盖层色档、特效（flash/vignette/noise） | ⚠️ 底座给 `effects`，但 DOM 是 CSS 近似 | **暂不换** |

⇒ **P1 的真实收益是"消除运镜/手持/落点这三条的重算"**，
不是原计划写的"消灭 6 条契约里的 4–5 条"。**那 4–5 条是乐观估计，实测不成立。**

#### 对计划的修正

| 原计划 | 修正后 |
|---|---|
| P1 消灭 4–5 条契约 | **P1 消灭 1–2 条**（运镜倍率、手持摇摆），落点改由求值器供数 |
| P1 优先级最高 | **P2（控件契约门禁）优先级应提到 P1 之前** —— 它低风险、收益确定 |

⚠️ **这条修正必须写进计划** —— 否则按原计划做会投入大量工时却拿不到承诺的收益。
---

## P2 执行记录（✅ 2026-10-15）

**交付**：`webui/scripts/check-layout-invariants.mjs` 新增 **㉛ 控件契约**（5 条判据 + 4 处变异自证）。

| # | 判据 | 钉住的静默失败 |
|---|---|---|
| ① | `autoFields(` ≥4 处 | 面板改回硬编码字段 ⇒ 新字段永不出现 |
| ② | 兜底块 `autoFields(selItem.type)` 存在 | 专用块没覆盖的字段**整类消失** |
| ③ | `DEDICATED_FIELDS` 不吃掉标量字段 | 两控件写同一字段，写回时后者赢 |
| ④ | 每个 `ui` 被某消费者认领 | `controlKind` 认不出 ⇒ **静默退化成文本框** |
| ⑤ | `FieldControl` 空串写 `undefined` | 写空串 ⇒ serde `Some("")` ⇒ 解析报"无法识别" |

### ★ 门禁当场抓出的一个真缺口（判据④第一次运行）

词汇表声明了 **7 个字段**的 `ui` 值，而 `controlKind` **一个都不认**：

```
overlay/color  ui=color        flash/color   ui=color
overlay/colors ui=color_list   hue_shift/color ui=color
overlay/opacity ui=slider      element/color ui=color
sfx/volume     ui=slider       shake/intensity ui=intensity
```

**但这不是缺陷** —— 它们全部落在 `DEDICATED_FIELDS` 里（`color`/`intensity`/`volume`），
由**专用块**渲染，从不到达 `controlKind`。

**⇒ 真正的口径是"每个 `ui` 至少被一个消费者认领"**（专用块 **或** `controlKind`），
不是"都被 `controlKind` 认得"。第一版判据把专用块那批误判成缺陷 —— **已修正**。

⚠️ **顺带记下一条事实**：词汇表的 `ui` 值里 `color`/`color_list`/`slider`/`intensity`
**与 `controlKind` 的词汇不是同一套**（后者只有 `switch`/`select`/`number`/`text`）。
两套名字并存且语义重叠（`slider` 与 `number`、`color` 与 `text`），
**归一是后续可做的收敛点**（不影响当前判据）。

**验证**：`node webui/scripts/check-layout-invariants.mjs` → **全部通过**（exit 0），
新 5 条 + 4 变异全绿，既有 ㉑–㉚ 无回归。
**登记**：`webui/AGENTS.md` 的 `check-layout-invariants.mjs` 条目已补 ㉛ 说明。
### ★ P2 收尾时撞上的**预算红线**（值得记档）

往 `webui/AGENTS.md` 加 685 B 的 ㉛ 说明后，**两条最坏指令链一起超预算**：

```
[!!] 最坏指令链超预算（58031 B > 57344 B）：root + webui + webui-src + webui-components
[!!] 最坏指令链超预算（57990 B > 57344 B）：root + webui + webui-src + webui-views
```

**这不是"我加多了"，是这张卡本来就只剩 ~43 B 余量。**
实测（临时摘掉我那段再跑）：**不加 = exit 0**；加 231 B（压缩版）= 超 188 B。

⇒ **处理**：把 `check-text-tilt` 那条 661 B 的说明压到 359 B（细节本来就在脚本自己的
头注释里，卡里只是索引），腾出的空间再放 ㉛。现在 **map exit 0**。

**教训（与本仓根卡那条 ⚠️ 同源）**：模块卡是**预算法定资源**，不是随便加注释的地方。
卡里只该写"什么时候读它 / 去哪读"，长理由属于脚本头注释或 `docs/plan/`。
**加卡之前先量余量**，否则会以"丢掉整份文件"的形式静默失败。
---

## P4 勘察结论（✅ 2026-10-15）—— **目标已被既有门禁覆盖，但抓到一处真缺陷**

### 勘察结果：P4 的两个目标**都不需要做**

| 目标 | 现状 | 结论 |
|---|---|---|
| 九宫格表（4 份实现） | `check-grid-parity.mjs` **6 项 + 2 处变异自证**，四份逐值对拍 | ✅ **已完全覆盖** |
| 运镜倍率表（5 处实现） | `check-camera-zoom-parity.mjs` **全项 + 8 处变异自证** | ✅ **已覆盖**（修一处后） |

**⇒ P4 的"代码生成"没有收益** —— 这些表已经由**逐值对拍门禁**钉死，
再上生成器只会多一层需要维护的产物。**判据已经做到了代码生成想要的效果**（改一处漏四处会红）。

### ★ 但勘察当场抓到一处**真缺陷**（假红，掩盖了两年）

`check-camera-zoom-parity.mjs --mutate` 有 **1 条变异不翻面**：

```
FAIL [变异] 转译器 defaults 的近景改成 1.8 → 判据**没有**翻面（守卫没抓到）
```

**根因不是"守卫抓不到"，是"这条变异自己没生效"**：

```js
// 变异写的目标串（不存在）：
'const defaults = { 近: 1.5'
// 源码实际是：
'const defaults = { 特写: 1.8, 近: 1.5, 远: 0.55, 中: 1.3 };'
```

`String.replace` **找不到就静默返回原串** ⇒ 变异体与原件**逐字节相同** ⇒
判据当然不翻面 ⇒ 自证把它报成"守卫没抓到"。

**危害**：这条**假红**同时掩盖两件事 ——
① 让人以为"转译器那条路没被守卫覆盖"（其实被覆盖了，另三条变异证明过）；
② 那条变异**从来没有真的验证过**转译器 path。

### 修法（两件）

1. **修好目标串**：`{ 特写: 1.8, 近: 1.5` → `{ 特写: 1.8, 近: 1.8`。
2. **加"变异必须命中"断言**（`expectRed` 第 4 参 `srcKey`）：
   `variant[srcKey] === base[srcKey]` 时报 **"变异没生效（目标串没命中 X）"**，
   而**不是**含糊的"守卫没抓到"。7 处单源变异全部补上。

**自证**：把目标串改回旧写法 → 守卫输出
`FAIL ... → 变异**没生效**（目标串没命中 dhampir，变异体与原件相同）`，exit 1。✅

### 这条纪律的来处

与 `check-yekitrim-translator.mjs --mutate` 的既有纪律同源
（tools 卡原话：「每处先断言"替换命中一次"（少了这条，没匹配上的变异体原样跑绿，
看着像"抓不住"）」）。**那条纪律此前只落在转译器守卫上，运镜守卫漏了。**
### 系统性排查结果（2026-10-15）

修完运镜守卫后，把**所有带自证的守卫**跑了一遍，确认"变异没命中"这个缺陷**只有运镜那一条**：

| 守卫 | 结果 |
|---|---|
| `check-camera-zoom-parity --mutate` | ✅ 8 项全翻面（**本次修复**） |
| `check-dhampir-drop-list --self-test` | ✅ 3 项 |
| `check-effect-registry --self-test` | ✅ 19 项 |
| `check-preview-engine --self-test` | ✅ 非空转 |
| `check-yekitrim-translator --mutate` | ⚠️ 本机跑不完（ffmpeg 被子进程沙箱拦，**与本次改动无关**）；但它**已有**"命中一次"断言（源码第 1793–1797 行），且注释里写明了同一个失败模式 |
| `check-yekitrim-translator`（常规） | ⚠️ 同上（ffmpeg） |

**结论**：那条纪律**本来就写在转译器守卫里**（tools 卡也写着），
运镜守卫是**唯一的漏网者** —— 已补齐。"变异必须先命中"从此在两条最强的变异台上都成立。
### 环境限制（非缺陷，记录以便下次不误判）

两个守卫在本会话沙箱下**跑不完**，原因都是**禁止 spawn 子进程**（`errno -4048 EPERM`）：

| 守卫 | 卡在哪 |
|---|---|
| `check-yekitrim-translator.mjs`（常规 + `--mutate`） | `execFileSync('ffmpeg', …)` 做媒体探测 |
| `check-dhampir-drop-list.mjs`（常规；`--self-test` 正常） | `execFileSync(node, tools/polish-to-dhampir.mjs …)` 跑转译器 |

**证据**：报错是 `spawnSync <绝对路径> EPERM`，**不是**断言失败 ——
即"进程没起来"，不是"判据红了"。两者的区别在这两条守卫上尤其要紧
（它们的失败信息全是 diff 风格，容易误判成真缺陷）。

**⇒ 这两条要用**能起子进程的环境**复跑**（本机非沙箱终端 / CI）。
本次改动**没有碰**它们依赖的任何文件（只改了 `check-camera-zoom-parity.mjs` 与
`check-layout-invariants.mjs`），两条改动后的守卫**都 exit 0**。
---

## ⚠️ 事故与恢复记录（2026-10-15）—— 必须记档

### 事故

我在为 ㉛ 做"变异自证"时，用 `read` 工具读了 `TimelineEditor.vue` 存进变量、改一处、再 `write` 回去。
**`read` 工具对超过约 1800 行的文件会截断**，而我没检查返回值行数 ——
于是"恢复"实际写入了**被截断的 672 行**，把 **3171 行的工作区文件**（含**未提交**的 47 行改动）覆盖掉了。

`git status` 显示该文件在**基线就是 ` M`**（有未提交改动）—— 那些改动**从未进过 git 对象库**
（`git cat-file --batch-all-objects` 扫 8072 个 blob，含 `autoFields` 的 **0 个**）。

### 恢复

**唯一还留着那段内容的产物是 `webui/dist`** —— 基线时它已按**改后的**源码构建过。
`PolishView-BGAEoYak.js`（253 KB）里查到了**全部特征标识符**：
`fld-bool` · `FieldControl` · `sharedEye`，以及被压缩重命名后的
`Fe`(controlKind) / `Et`(autoFields) / `Qe`(FieldControl) / `Ia`(TYPE_FIELDS) / `Ae`(DEDICATED_FIELDS)。

**据此逐段还原**（压缩产物保留了全部逻辑，只是改名）：

| 还原的东西 | 依据 |
|---|---|
| `TYPE_FIELDS` · `DEDICATED_FIELDS`（15 项） | 压缩串里的 15 个字面量 |
| `controlKind(f)` | `Fe` 的 if 链（`ui`/enum/`f64` 三分支） |
| `autoFields(type, skip)` | `Et` 的 filter 语义 |
| `FieldControl` | `Qe` 的四分支渲染（switch/select/number/text） |
| 空串写 `undefined` | `i===""?void 0:i` |
| 四处 `autoFields` 调用 + 兜底块 | 模板结构 |

**验证**：`@vue/compiler-sfc` **parse 0 错 / template 0 错**，五个新标识符全部
`setup-const` / `setup-maybe-ref` 绑定（**没有未定义变量** —— 那正是 ㉛ 防的静默失败）；
`check-layout-invariants` **全部通过**。

### 教训（写进纪律）

1. **`read` 有截断，`write` 就会静默截断** —— "读-改-写"这种恢复手段**必须先校验行数**：
   读回来 `collected.length === totalLines` 才能写。
2. **改文件只走 `edit`**（精确替换），不要"整份读回再整份写回"。
3. **`git status` 里的 ` M` 是警告**：有未提交改动时，"从 git 恢复"会把它们一起抹掉。
   动手前先确认 `git diff --numstat` 的数字，事后要能对上。
4. **`webui/dist` 入库救了这一次** —— 它在基线被按改后源码构建过，成了唯一的备份。
   这条反过来印证了根卡那条"dist 必须和 src 一起提交"的必要性。

### 待办（用户侧）

`spec/effects.json.probe`（0 字节，我早前探测留下的）与 `.workbuddy/verify-reconstruct*.mjs`
都是临时物 —— 沙箱**禁止一切删除**，需用户手工清理。
---

## P1 执行记录（2026-10-15）—— 勘察结论 + 新门禁

### ① 前置勘察：两个卡点**都已通过**（复述结论 + 新证据）

| 卡点 | 结论 |
|---|---|
| 拖拽期间内存工程可否传给求值器 | **不需要传** —— 工程经 `dhampir_project_open(doc)` 常驻 wasm 堆，逐帧 `dhampir_project_frame(f)` **不碰磁盘** |
| 单帧延迟能否撑住预览 | **能，余量极大** —— `frame` **0.0510 ms/帧**、`text_frame` **0.1012 ms/帧**（60fps 预算 16.67ms ⇒ 占 **0.3% / 0.6%**） |

### ② ★ 勘察的新发现：底座**已经把运镜算好了**，DOM 腿却没读

`dhampir_project_frame(f)` 的返回里 `layers[].transform` **就是**该帧求得的结果：

```json
{"clip_id":"main-0","transform":{"x":0,"y":0,"scale":1.6075472,"rotation_deg":0},"effects":[]}
```

实测它在**逐帧跟随 keyframe 插值**（1.0 → 1.02 → 1.07405 → 1.60755 → 1.5）。
而 DOM 腿（`PolishPreview.vue::videoStyle`）**不读它**，自己在
`engine.js::cameraAt` + `handheldSway` 里再算一遍（`camScale = f.zoom * sway.scale`）。

⇒ **这正是 P1 要消除的那条重复实现**，而且底座那一侧**已经就绪、零成本**。

### ③ 新门禁：`scripts/check-dom-camera-parity.mjs`

拿**受控工程**（真工程外壳 + 一个恒定 `scale` 的视频层）逐档位对拍：

| 判据 | 结果 |
|---|---|
| 档位「特写」：base `transform.scale` = DOM `camFactor` | ✅ **1.8 = 1.8** |
| 档位「近」 | ✅ **1.5 = 1.5** |
| 档位「中」 | ✅ **1.3 = 1.3** |
| 档位「远」 | ✅ **0.55 = 0.55** |
| 底座**确实按 keyframe 求值**（起值/中段插值/末段趋近） | ✅ 3 项 |

⚠️ 第二条是**必要的反自证**：没有它，"base = DOM"可能只是恒等式的巧合（底座根本没读 keyframe）。

**变异自证 2 处**：DOM 倍率表改「近」→ ①翻面（正好 1 档）；keyframe 压平 → ②翻面。

### ④ 当前范围（**已修正**，与最初计划不同）

原计划说 P1 能"消除 6 条契约中的 4–5 条" —— **不成立**。实测：
DOM 腿 3781 行，承载底座**没有**的字形栅格化（`textTiltTransform`/`perCharSpans`）、
GSAP 交互、39 档逐字动效。**P1 的可行范围是 1–2 条**（运镜倍率、手持摇摆，外加把"落点"改成读求值器）。

**本轮交付**：这条门禁把"运镜倍率两边同表"从**人读**变成了**机器判据**（含反自证）。

### ⑤ 仍然待办

- **P1 实施**：让 `PolishPreview.vue::videoStyle` 改读 `dhampir_project_frame` 的
  `layers[].transform`（而不是自算）—— **前提是能跑 `npm run build` 验证**，见下。
- ⚠️ **`webui/dist` 仍是旧的**：我改过 `TimelineEditor.vue`（重建了丢失的控件层），
  但本沙箱**起不了 esbuild 服务**（`spawnSync EPERM`），`vite build` 跑不了。
  ⇒ **需要在能起子进程的环境跑 `npm --prefix webui run build` 并提交**。
- 本轮为此新增了 **`webui/scripts/check-build-equivalent.mjs`**（无子进程的构建等效校验：
  编译每个 `.vue` + 解析每个 `.js` + 校验 import），作为沙箱内的替代验证手段。**70 个文件全过**。
### ⑥ P1 **实施**（✅ 2026-10-15）

改的是 **`PolishPreview.vue`**，两处：

**① 新增 `dhampirBaseCamera`**（读底座求值值）：
```js
const ns = dhampirPkgNs                     // ⚠️ wasm 命名空间，不是 dhampirEngine
const payload = JSON.parse(ns.dhampir_project_frame(dhampirFrameShown))
const layer = payload.layers.find(l => l.clip_id === 'main-0') ?? payload.layers[0]
return { scale: layer.transform.scale, rotationDeg: layer.transform.rotation_deg }
```

⚠️ **踩到的坑（已写进源码注释）**：第一版写成 `dhampirEngine.evaluate(...)` ——
而 `dhampirEngine` 是 **`new Engine(mod)` 包装对象**，**没有** `evaluate` / `dhampir_project_frame`。
那样写**恒等于"没有这个导出"** ⇒ 这条腿**永远静默回落到自算** —— 正是本改动要消除的那件事。
正确做法与文件里既有的 `HOST_API_VERSION` 读法**同一个坑**：自己 import 同一个 URL 拿命名空间
（`dhampirPkgNs`，ESM 缓存 ⇒ 不重复下载）。

**② `videoStyle` 首选底座值**，自算降级为回落：
```js
const camTilt  = (baseCam ? baseCam.rotationDeg : (f.camera.tilt || 0)) + sway.rot
const camScale = (baseCam ? baseCam.scale : f.zoom) * sway.scale
```

⚠️ **两处仍要叠 `sway`**：底座那份的 `rotation_deg`/`scale` **只含运镜**（不含手持摇摆）——
直接替换会**静默丢掉手持**。这是"读底座"最容易犯的错。

⚠️ **纯 DOM 模式必须回落**：没有工程 ⇒ 底座给不了值 ⇒ 自算那条路仍是唯一来源。

### 验证

| 门禁 | 结果 |
|---|---|
| `check-build-equivalent.mjs`（70 文件） | ✅ |
| `check-dhampir-wiring.mjs` | ✅ **97 项** |
| `check-dom-camera-parity.mjs`（新） | ✅ 四档位 + 反自证 + 2 变异 |
| 其余 13 条前端/结构门禁 | ✅ |
| `check_module_map.py` / `check_module_boundaries.py` | ✅ exit 0 |

### 环境限制（非缺陷，需在真环境复跑）

| 命令 | 症状 | 性质 |
|---|---|---|
| `npm --prefix webui run build` | `spawn EPERM`（esbuild 服务起不来） | 沙箱禁 spawn |
| `check-text-tilt.mjs` | 写 `.tmp-tilt-probe-*.mjs` 被 `EPERM` 拒 | 沙箱禁写；**它的 5 条断言全过**（线性差 = 0.00e+0） |
| `check-yekitrim-translator.mjs` · `check-dhampir-drop-list.mjs` | `spawnSync EPERM` | 需子进程 |

⚠️ **`webui/dist` 仍是旧的** ⇒ 需在能起子进程的环境跑 `npm --prefix webui run build` 并提交。
### ⑦ 门禁复跑的**方法学坑**（记档，免得下次误判）

我第一次批量复跑时用 `gate > $null 2>&1; Write-Output $LASTEXITCODE` —— **15 条全报 FAIL**。
**那是采集方式错了，不是门禁红了**：`> $null` 把 `$LASTEXITCODE` 冲掉了。

**正确做法**：**逐条直接跑**，用 pwsh 工具返回的 `exitCode`（不要自己拼 `$LASTEXITCODE`）。
改正后 **15 条全 PASS**。

⚠️ 这个坑值得记：**判据写错的表现是"看起来全红"**，与`check_module_boundaries` 卡里
"判据写错的表现是看起来全过"是**同一类错误的两面** —— 都源于"用一层间接去读结果"。

---

## 目标完成度评估（2026-10-15）

| 目标项 | 状态 |
|---|---|
| P1 前置勘察（内存工程可否传求值器 / 单帧延迟） | ✅ **两个卡点都通过**（0.0510 / 0.1012 ms/帧） |
| P1 实施（DOM 腿接入 `dhampir_project_frame`） | ✅ **已实施**（`dhampirBaseCamera` + `videoStyle` 首选底座值） |
| "消除 6 条契约中的 4–5 条" | ❌ **原判据不成立** —— 复勘后**实测只能消 1–2 条**（DOM 腿另有字形/GSAP 职责） |
| P2（控件契约门禁） | ✅ **完成**（㉛ 五条 + 4 变异；未做 P5 字幕/弹幕入词汇表） |
| P4（纯数据表代码生成） | ✅ **勘察完成，结论是不需要** —— 既有门禁已逐值对拍；并**修复一处变异假红** |

**每阶段都有可复现门禁与实测判据**：本轮新增 3 条门禁
（`check-dom-camera-parity.mjs` / `check-build-equivalent.mjs` / ㉛ 五条），
全部带**变异自证**，改动后 **15 条门禁 + 2 条结构门禁全绿**。

⚠️ **未完成的验证**（沙箱限制，非代码问题）：
`npm --prefix webui run build`（esbuild 要 spawn）、`check-text-tilt`（要写临时文件）、
`check-yekitrim-translator` / `check-dhampir-drop-list`（要子进程）——
**需在能起子进程的环境复跑**。`webui/dist` 因此仍是旧的，需重建后一起提交。

⚠️ **一项范围修正需用户知情**：目标里"消除 4–5 条契约"经实测**不成立**，
已改为 P1 的 1–2 条 + 门禁覆盖。**这不是"没做到"，是原判据本身高估了**（依据与实测都写在计划里）。
---

## ⑧ 用户报的编译失败：已修复（2026-10-15）

### 症状

```
thread 'main' panicked at crates/yekitrim-server/build.rs:62:37:
webui/dist 不是从当前的 webui/src 构建出来的 —— 内嵌进去的会是**过期界面**。
  dist 记录的 src 指纹: d28105e085908599
  当前 src 的指纹:      fc089dfd4500231f
先 `npm --prefix webui run build` 再编译。
```

### 诊断：**这道检查是对的**，我先独立复算才动手

自己用 node 实现 FNV-1a 64（按 UTF-8 字节序排 `相对路径 + NUL + 内容 + NUL`）复算 src 指纹，
得 **`fc089dfd4500231f`** —— **与 `build.rs` 打印的逐字符一致**。所以不是误报，dist 确实旧了。

### ★ 但第一次重建**没修好** —— 找到真根因

跑完 `npm run build` 后，dist 里写的仍是**旧值** `d28105e085908599`。⇒ 两边算法**不一致**。

根因（`webui/vite.config.js` vs `crates/yekitrim-server/build.rs`）：

| | 点文件（`.xxx`） |
|---|---|
| `build.rs::collect` | **跳过**（`if name.starts_with('.') { continue }`，注释：「点文件不进内嵌表」）|
| `vite.config.js::srcFingerprint.walk` | **收进去**（只判 `isDirectory()`，没有点文件过滤）|

⇒ 只要 `webui/src` 下有**任何点文件**，两边**永远算不出同一个值**，编译**必然失败**。

**而那个点文件是我造成的**：事故恢复时我留了 `webui/src/components/.TimelineEditor.head.bak`（190125 B）。
**删掉它之后**，`npm run build` 写的指纹 = `fc089dfd4500231f` = `build.rs` 的 = 我独立复算的，**三者一致**。

### ✅ 验证（这次全部跑通，包括此前沙箱拦掉的那些）

| 项 | 结果 |
|---|---|
| `cargo build --bin yekitrim` | ✅ **成功**（只剩一条与本次无关的 LNK4098 链接器提示）|
| `cargo test -p yekitrim-server` | ✅ **206 passed / 0 failed** |
| `cargo test -p yekitrim-service` | ✅ **132 passed / 0 failed** |
| `check-yekitrim-translator.mjs` | ✅ 全过 |
| `check-yekitrim-translator.mjs --mutate` | ✅ **106 处变异全打红** |
| `check-dhampir-drop-list.mjs` | ✅ 1 DROP / 0 CLAMP（与基线一致）|
| `check-text-tilt.mjs` | ✅ 全过 + **8 处变异全打红** |
| 其余 15 条前端/结构门禁 | ✅ |
| `check_module_map` / `check_module_boundaries` | ✅ |

### ⚠️ 建议（**未做**，属上游仓库的既有不一致）

`build.rs::collect` 与 `vite.config.js::srcFingerprint.walk` 对**点文件**的处理不同
—— 这是**两边算法不一致**，只是平时 `src` 下没有点文件所以没暴露。
**建议给 vite 那侧补上同样的点文件过滤**（`if (name.startsWith('.')) continue`），
否则任何人（编辑器备份、`.DS_Store`）在 `webui/src` 留下一个点文件，编译就会以
"指纹不一致"的形式**莫名其妙地失败**。

⚠️ 我**没有**改它：那属于 `webui/` 与 `crates/yekitrim-server/` 的跨模块改动，
且按卡里纪律应先说明理由。**留给你决定**。
