# polish 精修模块：面向长期扩展的架构方案

> 立档：2026-10。目标：**让"加装任意一类新功能"都只改一处**，
> 且漏改时门禁**点名**而不是静默漂移。
>
> 配套证据：`docs/plan/polish-decoupling-assessment.md`（体检清单）、
> `docs/plan/polish-extensibility-options.md`（四方案对比）、
> `docs/dhampir/p9-downstream-requirements.md`（底座给本仓的需求收表）、
> `docs/dhampir/polish-translation.md`（转译实测 963 行）。
>
> **本文件只做架构设计，不含改动。**

---

## 0. 结论：长期架构已经存在，而且底座已经建好了

这是本次调查**最重要的发现**，它把问题从"我们要设计一个新架构"
变成"**我们这边还没跟上底座已经建好的架构**"。

### 0.1 分工早已正式定义

`docs/dhampir/README.md` 的边界一句话：

> **YekiTrim 决定"画什么"，Dhampir 决定"怎么画出来，且预览与成片画得一样"。**

### 0.2 底座已经走完"去枚举化"，并留下对本仓的正式设计

`../Dhampir/plan/effects-modularization.md` 进度表：**S1–S6c 全部 ✅ 完成**
（注册表守卫 / 转场去枚举化 / `EffectSpec` 补 space+pipeline / 渲染派发认管线不认名字）。

`../Dhampir/plan/p9-polish-parity-design.md` §3 的四条设计原则，
是**直接从本仓的坑里倒推出来的**：

> 1. **特效 = 数据行，不是枚举变体。** 每行 `{kind, params{...}, window, space, blend}`。
>    加一个特效 = 注册表加一条 + 加一个管线 + 加一份 WGSL；**渲染主路径一行不改**。
> 2. **参数是 map 不是 struct。**
> 3. **时间窗只有两种**：`persistent` / `transient`。参照实现靠 `Event::default_end()`
>    一个 15 分支的 match 表达 —— **那也是一张要泛用的表**。
> 4. **两端同一份求值**：窗口函数是纯 `t` 的函数、无累积状态。

同一份文件 §1 对本仓的**直接批评**：

> 参照实现那边是 `Event` 枚举 15 个变体 + `EffectUniform` 一个**定长 struct**，
> 加一个特效要同时改枚举、改 enum 的 4 个方法、改 uniform 结构、改着色器。
> **那正是要摆脱的形状。**

**⇒ 本仓要做的不是"设计"，是"对齐一个已有且被验证过的架构"。**

---

## 1. 实测：为什么现在的形状不可长期扩展

### 1.1 三套机制，而非一套

底座用**三套并行机制**表达画面，本仓 12 个效果**分布其上**：

| 机制 | 底座形态 | 本仓哪些效果走它 |
|---|---|---|
| **① Effect（注册表）** | `{kind, params:BTreeMap, window, opacity}` | `flash` `vignette` `noise` `overlay` `blur` `hue_shift` |
| **② Keyframe transform** | `Keyframe{ target: "scale"/"x"/"effect.0.radius" }` | `camera` `shake` `pulse` |
| **③ Layer / 素材引用** | `SourceRef{asset_id, source_in, source_rate}` | `sticker` `element` |
| （④ 音轨） | 不在底座（本仓混音） | `sfx` |

**这是关键**：任何"把 Event 改成 params map"的方案，**只能覆盖机制①的 6 个效果**。
机制②③ 需要各自的泛用化，而它们恰恰是**最大的两个 case**（`sticker` 288 行 / `element` 242 行）。

### 1.2 枚举形状的量化代价

`Event` enum 12 个变体的字段解析结果：

**通用字段**（本该是公共头）：
```
time 12/12 · end 12/12 · sfx 9/12 · sfx_volume 9/12 · anchor 9/12
```

**字段集完全相同的变体组**（★ 纯冗余）：
```
[Flash, HueShift]               → time,color,end,anchor,sfx,sfx_volume
[Blur, Vignette, Noise, Pulse]  → time,end,anchor,sfx,sfx_volume
```

`Blur/Vignette/Noise/Pulse` 是**四个字段一模一样的 enum 变体**。底座那边是
`params` 为空的一行 `EffectSpec`。

### 1.3 类型分派点

`Event::` 全仓 **296 处**；按类型字符串分派的文件 **17 个**。
对照底座：`spec_of()` 一处线性查找。

---

## 2. 长期架构：四层收敛

设计原则：**每一层都把"N 处手抄"变成"1 处声明 + 派生"**。

```
┌───────────────────────────────────────────────────────────────┐
│ L0  spec/effects.json  —— 唯一的"能力清单"（已存在，需扩展）    │
│     每个效果：id / label / lane / kind / params / window /      │
│                走哪套机制 / 各端锚点                            │
└───────────────────────┬───────────────────────────────────────┘
                        │ 派生 + 门禁点名
        ┌───────────────┼───────────────┬───────────────┐
        ▼               ▼               ▼               ▼
┌──────────────┐ ┌──────────────┐ ┌──────────────┐ ┌──────────────┐
│ L1 类型模型   │ │ L2 转译器     │ │ L3 预览      │ │ L4 编辑器     │
│ 公共头 +      │ │ 声明式映射    │ │ 只消费转译产物│ │ 由 params    │
│ kind + params│ │ （非手写case）│ │ （已部分达成）│ │ 描述生成控件  │
└──────────────┘ └──────────────┘ └──────────────┘ └──────────────┘
```

### L0：能力清单（扩展点唯一落点）

`spec/effects.json` 已经是权威（12 效果 / 155 个消费点受门禁守护）。
**需要扩展**的是它现在只描述"事件类型"，要升级成描述**一套机制**：

```jsonc
{
  "id": "shake",
  "lane": "fx",
  "mechanism": "keyframe",       // ← 新增：走哪套底座机制
  "params": [{ "name":"intensity", "min":0, "max":100, "default":14, "ui":"intensity" }],
  "window": { "default_span": 0.3, "cap": 0.5 },
  "anchors": { "dhampir": "...", "canvas": "...", "preview": "..." }
}
```

**收益**：`params` 变成结构化的 ⇒ L1/L2/L4 全部可从它派生。
**这一步是所有后续的前置。**

### L1：类型模型 —— 公共头 + kind + params

```rust
struct Event {
    // 通用头（实测 12/12 与 9/12 的公共字段）
    time: f64, end: Option<f64>, anchor: Option<String>,
    sfx: Option<String>, sfx_volume: Option<f64>, manual: bool,
    // 类型与私有参数
    kind: String,
    #[serde(default)] params: BTreeMap<String, Value>,
}
```

**可行性的关键实测**：`Event` 现在用 **`#[serde(tag = "type")]`（内部标签）** ⇒
磁盘上**已经是** `{"type":"shake","intensity":14,...}` 的扁平形状 ——
与目标形状**天然同构**。迁移是**搬迁**，不是重写。

**收益**：`296` 处 `Event::` 分派里纯机械的那批消失；
`Blur/Vignette/Noise/Pulse` 四个变体塌成一行。

### L2：转译器 —— 从手写 case 到声明式映射

现在是 12 个手写 `case`。改成一个**映射表**：

```js
const MECHANISM = {
  effect:   (ev) => ({ effects: [{ kind: mapKind(ev), params: mapParams(ev) }] }),
  keyframe: (ev) => ({ keyframes: buildKeyframes(ev) }),
  layer:    (ev) => ({ layers: buildLayer(ev) }),
}
```

**收益**：加同类效果 = L0 加一条 + 可能需要一个参数映射函数，**不再改 switch**。

⚠️ **注意**：转译器是**出片唯一通道**，且有 348 项曲线守卫 + 52/59 变异自证。
本改造必须保持**逐字节输出不变**（可用现有守卫验证）。

### L3：预览 —— 让 wasm 成为主腿

**已实测的重要事实：wasm 腿现在已经消费转译产物，与出片同源。**

| 端 | 消费什么 |
|---|---|
| 出片（底座后端） | 转译器产出的工程 JSON |
| **wasm 预览** | **同一份**工程 JSON（`POST /polish/dhampir-project` → `open(project)`） |
| DOM 预览 | 本仓 `Event` 模型（`engine.js` 自绘） |

⇒ **wasm 端不需要"对齐"，它本来就在同一条链上。**

真正的问题：`dhampirMode = ref(false)` —— **默认是 DOM 自绘，底座是可选开关**。
这与根卡的"出片默认就是底座"不一致。

**长期方向**：让 Dhampir 成为默认预览腿，DOM 降级为纯兜底（不再要求逐像素一致）。
⇒ `engine.js`（2571 行）从"必须逐像素对齐的手工移植"降为"近似兜底"。

### L4：编辑器 —— 控件由 params 描述生成

现状：`TimelineEditor.vue` 的属性面板是**手写分支**（实测 `selItem.type === ...` 共 6 处），
且这是**零门禁覆盖**的区域。

改成：遍历 L0 的 `params`（含 `ui` 类型：滑块/颜色/下拉）**自动生成控件**。
`uiGroup('intensity')` / `uiGroup('color')` 已经是这个思路的雏形 —— 把它推到底。

---

## 3. 分阶段路线（长期，可独立回退）

每一阶段都**独立可用**，且**不改上一阶段的语义**。

| 阶段 | 做什么 | 扩展性收益 | 风险 | 前置 |
|---|---|---|---|---|
| **P0** | L0 扩展：`params` 结构化 + `mechanism` 字段 + 字段级门禁 | 加字段从 7 处**零门禁** → 有门禁 | 低 | 无 |
| **P1** | L4 控件生成（编辑器不再手写分支） | 加参数 → **0 处**（自动出控件） | 低 | P0 |
| **P2** | L1 合并机制①的 6 个效果（含 4 个同形状变体） | 加同类特效 **11 → 2 处** | 中 | P0/P1 |
| **P3** | L2 转译器声明式化 | 同上，且**不再改 switch** | 中高 | P2 |
| **P4** | L3 预览主腿切换（wasm 转正） | 消灭"手工移植"整类耦合 | 中高 | 产品决策 |
| **P5** | L1 泛化机制②③（keyframe 型 / 图层型） | 新运镜 / 新元素 **11 → 2 处** | 高 | P2 成功验证 |

**关键里程碑判据**：P2 完成后，加一个"同类新特效"应只需
**改 `spec/effects.json` 两处**（能力条目 + 锚点）。这是可量化的验收标准。

---

## 4. 三条不可退让的约束（底座已验证的做法）

底座的 `effects-modularization.md` §0 列了三条"不许破坏"的既有约束，本仓同样适用：

1. **两端一致**：同一份工程在 wasm 与 native 下结构一致、可比。
2. **逐字节稳定**：序列化必须逐字节可复现（`BTreeMap` 不是 `HashMap`）；
   撤销/重做逐字节往返。**本仓 R4 红线同此**。
3. **不静默降级**：做不到的事要**报错**，不许按正常画过去。
   转译器的 `default:` 分支已经做到了这一点（区分"词汇表里有但转译器漏接"与"完全不认识"）。

**⚠️ 迁移安全网缺口**：本仓目前**没有专门的 R4 round-trip 金样门禁**
（`check-anim-docs-snapshot.mjs` 只覆盖动效文档生成物）。
P2/P3 之前**必须先补**这条 —— 否则数据模型迁移没有回归判据。

---

## 5. 待你拍板

1. **路线**：按 P0→P1→P2 逐步走（推荐，每步独立可验证），还是先做一个跨度更大的验证？
2. **P4 的产品决策**：预览要不要与出片**逐像素一致**？
   - 要 → DOM 腿必须永远维护，L3 收益归零，只能靠对拍门禁兜
   - 不要 → Dhampir 转正为主腿，DOM 降为近似兜底，**这是收益最大的一步**
3. **近期想加的功能**（2–3 个，越具体越好）：
   我用真实需求测算"今天几处、改完几处"，作为方案收益的**验收标准**。

---

## 6. 证据等级

| 等级 | 内容 |
|---|---|
| **一手实测** | 行数、门禁运行（全绿）、`Event` 变体字段解析、12 个效果→底座机制映射、`EffectSpec`/`EffectPipeline`/`Keyframe.target` 源码、`serde(tag)` 方式、跨仓对拍 3 条实跑、`dhampirMode` 默认值、`docs/dhampir/` 2421 行文档 |
| **对方仓文档** | `../Dhampir/plan/effects-modularization.md`（S1–S6c 完成）、`p9-polish-parity-design.md`（四条原则 + 对本仓形状的批评）、`docs/api-surface.md` 边界承诺 |
| **本仓文档** | 根 `AGENTS.md`、`effect-pipeline-unification.md`、`s4-*.md`、`polish-enrichment-proposal.md`、`docs/dhampir/*` |
