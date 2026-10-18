# 按长期架构方案：新增特效/内容的预期工作量（预测）

> 立档：2026-10。配套：`docs/plan/polish-longterm-architecture.md`（架构）、
> `docs/plan/polish-extensibility-options.md`（方案对比）。
> **本文件回答一个问题：按这个方案走，加东西到底要改几处？**
>
> 全部数字来自**实测**（`grep` / 源码解析），不是估计。

---

## 0. 一张表看懂

| 场景 | 今天 | 方案走完之后 | 阶段 |
|---|---|---|---|
| **A. 用底座已有、本仓没接的特效** | 5 处 | **1 处** | 现在就能做 |
| **B. 加一个"简单特效"**（单参数、走 Effect 机制） | ~11 处 | **2 处** | P2 |
| **C. 加一个"新参数"给已有特效** | 7 处（**零门禁**） | **1–2 处** | P0+P1 |
| **D. 加一个新图层类型**（如新贴纸玩法） | ~11 处 | **2–3 处** | P5 |
| **E. 加一个新运镜** | ~11 处 | **2–3 处** | P5 |
| **F. 加一个新的文字动画档** | 3 处 | **1 处** | 已有类似机制 |
| **G. 加一个新的后处理步骤** | 1–2 处 | 1–2 处（本来就便宜） | — |

**关键区分**：B/C 便宜，D/E 要等 P5。**"新特效"和"新内容"是两类成本。**

---

## 1. 实测基线：`vignette` 走过的 24 个文件

我拿最简单的效果 `vignette` 做了全仓追踪（`grep -rn`）：

| 归属 | 文件数 | 说明 |
|---|---|---|
| **封面链**（不在本线） | 3 | `cover.rs` / `CoverDesigner.vue` / `cover.html` —— 封面的 vignette 是**另一个东西** |
| 门禁与文档 | ~6 | 检查脚本、参考文档 |
| 死代码 | 1 | `timeline.rs`（22 处，无消费者） |
| **polish 事件链真正要改** | **7** | 见下 |

**polish 事件链的 7 个落点**（实测每处的 `vignette` 命中数）：

| # | 文件 | 命中 | 门禁覆盖 |
|---|---|---|---|
| 1 | `spec/effects.json` | 6 | ✅ 权威 |
| 2 | `render-core/src/types.rs` | 5 | ✅ |
| 3 | `polish/src/process/fx.rs` | 3 | ✅ |
| 4 | `tools/polish-to-dhampir.mjs` | 9 | ✅ |
| 5 | `templates/index.html` | 14 | ✅ 存在性 |
| 6 | `webui/src/polish/engine.js` | 8 | ✅ 可达性 |
| 7 | `webui/src/components/TimelineEditor.vue` | **0** | ✅ **已派生** |

### ★ 一个纠正我早前判断的发现

`TimelineEditor.vue` 里 `vignette` **零命中** —— 编辑器**已经**从词汇表派生了。

而且对**新**类型它还有**通用兜底**（`TimelineEditor.vue:538-550`）：

```vue
<template v-else>
  <template v-if="INTENSITY_TYPES.includes(selItem.type)"> …强度滑块… </template>
  <template v-if="COLOR_TYPES.includes(selItem.type)"> …颜色选择… </template>
</template>
```

`INTENSITY_TYPES` / `COLOR_TYPES` 都来自 `uiGroup()`（读词汇表的 `ui` 字段）。

**⇒ 一个"单强度参数"或"单颜色参数"的新特效，编辑器控件是白拿的**，
只需在词汇表写 `"ui": ["intensity"]`。

这比我早前说的"面板控件零门禁、要手改"**乐观** —— 只有**复杂控件**（多参数组合、
自定义布局，如 `camera`/`element`）才需要手写分支。

---

## 2. 场景 A：用底座已有、本仓没接的特效 ★ 现在就能做

**实测：底座 REGISTRY 有 16 个 kind，本仓只用了 8 个。剩下 8 个是"白拿"的：**

```
brightness_multiply · hue · hue_rotate_css · pulse · saturation_css · shake · split · zoom_bounce
```

⚠️ 注意 `pulse` / `shake` 虽在底座注册表里，但本仓走的是**关键帧机制**（不是 `kind`），
所以那两条不能直接"白拿"。**真正零底座改动、直接可用的是这 6 个**：

```
brightness_multiply · hue · hue_rotate_css · saturation_css · split · zoom_bounce
```

| 步骤 | 改哪 |
|---|---|
| 1 | `spec/effects.json` 加一条（含 4 个锚点） |
| 2 | `types.rs` 加枚举变体 |
| 3 | `fx.rs` 两张表加条目 |
| 4 | 转译器加一个 `case`（**~14 行**，见下） |
| 5 | `engine.js` + `index.html` 加求值器/handler |

**这 5 处是今天就必须做的**，方案不改变它 —— 因为这是"首次接入一个底座能力"，
本仓侧没有实现过它。

**⇒ 方案的价值不在于让 A 变便宜，而在于让 A 之后的那一批（B）变便宜。**

### A 的转译 case 实测规模（`flash`，16 行）

```js
case 'flash': {
  const startFrame = remapFrame(srcStart);
  const rgb = hexColor(event.color) ?? { r: 1, g: 1, b: 1 };
  const dur = typeof event.duration === 'number' ? event.duration : 0.35;
  placeEnvelope(startFrame, 0.04 + dur,
    { kind: 'flash', params: { amount: 0.20, r: rgb.r, g: rgb.g, b: rgb.b } },
    (dt) => (dt < 0.04 ? dt / 0.04 : Math.max(0, 1 - (dt - 0.04) / dur)),
    where);
  note(where, `flash 色 …`);
  break;
}
```

**这是一个 14–16 行的重复模式**（`remapFrame` → `placeEnvelope` → `note`）。
简单效果的 case 实测：`pulse` 13 / `hue_shift` 15 / `flash` 16 / `sfx` 22 / `noise` 34 / `vignette` 46 行。

---

## 3. 场景 B：加一个简单特效（P2 之后）

**P2 完成后预期形态**：

```jsonc
// 只需改 spec/effects.json 两处：
{
  "id": "zoom_bounce",
  "label": "缩放弹跳",
  "lane": "fx",
  "mechanism": "effect",          // ← 走底座 Effect 机制
  "kind": "zoom_bounce",          // ← 直接映射，无需写 case
  "params": [
    { "name": "amount", "min": 0, "max": 1, "default": 0.3, "ui": "intensity" }
  ],
  "window": { "default_span": 0.4, "cap": 0.6 },
  "anchors": { "dhampir_case": "zoom_bounce", "canvas": "handlers.zoom_bounce", … }
}
```

**预期改动**：
1. `spec/effects.json` 加条目
2. `engine.js` + `index.html` 加求值器/handler（**预览与 canvas 是端实现，仍需写**）

**⇒ 2–3 处**（若底座 kind 已存在且预览可用通用实现，则可能降到 2 处）。

**对比今天**：`types.rs` 变体 + `fx.rs` 两张表 + 转译器 case 都**省掉了**（由 L0 派生/生成）。

**验收判据**：改完 `node scripts/check-effect-registry.mjs` 变绿即可，
**不再需要人肉记"要改几处"**。

---

## 4. 场景 C：给已有特效加一个参数（P0+P1 之后）

**今天**：7 处，**零门禁** —— 实测 `speed` 字段分布：
`types.rs` 30 / `polish-to-dhampir.mjs` 50 / `TimelineEditor.vue` 13 / `engine.js` 12 / `legacy.rs` **0**。

**P0+P1 之后**：词汇表 `params` 加一项 + 转译器映射函数加一行。
编辑器控件**自动出现**（走 `ui` 字段），门禁**点名**漏改处。

**⇒ 1–2 处，且有门禁。**

**这是"新内容"里最常见的一类**（给已有能力加旋钮），收益非常直接。

---

## 5. 场景 D/E：新图层类型 / 新运镜（P5，最贵）

这两类**不走 Effect 机制**，所以 P2 的收益覆盖不到：

| 机制 | 实测 | 走它的效果 |
|---|---|---|
| ① Effect（注册表） | ✅ P2 覆盖 | flash, vignette, noise, overlay, blur, hue_shift |
| ② Keyframe transform | ❌ P5 | **camera, shake, pulse** |
| ③ Layer / 素材引用 | ❌ P5 | **sticker（288 行 case）, element（242 行）** |

**⇒ 在 P5 之前，加"新运镜"或"新元素玩法"仍是 ~11 处。**

这是我必须诚实说明的部分：**P2 完成后只解决了"特效"这一类**，
而"内容"（元素/文字/运镜）要等 P5。

---

## 6. 一条重要的边界：什么情况下**改不动**

**底座的 `REGISTRY` 是可扩展的，但"加一个新的渲染管线"不行。**

底座的 `EffectPipeline` 只有 4 条（`SeparableBlur` / `ColorAdjust` / `ColorMask` / `Warp`）。
一个新特效若落在**这 4 条管线内**（只改参数、不改像素算法），本仓**可以只声明**；
若需要**新的像素算法**，就必须改底座（加管线 + WGSL）—— 那超出本仓范围。

底座自己也写明了这条边界（铁律 3）：*"特效是'类型 + 参数'的声明式数据，不是代码"*，
`Effect` 的 schema 描述里有一句：

> **不接受可上传的 shader** —— 那等于把两端一致性交给用户。

**⇒ 预期的"只改一处"，前提是新特效落在底座已有的 4 条管线内。**
若要新算法，走底座仓的流程（那是另一条线，不属于本方案）。

---

## 7. 预期时间线的诚实版本

| 阶段 | 完成后能"只改 1–2 处"的场景 |
|---|---|
| 现在 | A 类（用底座已有能力）—— 仍需 5 处，但是**一次性**的 |
| **P0+P1** | C 类（加参数）+ 编辑器控件自动生成 |
| **P2** | **B 类（加简单特效）** ← 主目标达成 |
| P3 | B 类不再改转译器 switch |
| P4 | 预览侧不再维护两份实现（**需产品决策**） |
| **P5** | D/E 类（新元素 / 新运镜） |

---

## 8. 需要你确认的两件事

1. **你近期要加的，偏 B（特效）还是 D/E（元素/运镜）？**
   - 偏 B → 走 P0→P1→P2，**收益快**
   - 偏 D/E → P2 帮助有限，要重新排优先级（**先做 P5 的前置会更划算**）

2. **你要加的东西，需要新的像素算法吗？**
   - 不需要（只用底座 4 条管线）→ 在本仓就能做到"改一处"
   - 需要 → 要动底座仓，**不在本方案范围内**

---

## 9. 证据等级

| 等级 | 内容 |
|---|---|
| **一手实测** | `vignette` 全仓 24 文件追踪、每个文件命中数、`flash` case 全文 16 行、各 case 行数、底座 16 个 kind 与本仓 8 个已用的差集、编辑器通用兜底分支源码、`INTENSITY_TYPES` 派生链、`EffectPipeline` 4 条枚举、`speed` 字段分布、门禁运行结果 |
| **对方仓文档** | `../Dhampir/plan/effects-modularization.md`、`p9-polish-parity-design.md` §3 四原则与铁律 3、`schema.rs::EffectSpec` |
