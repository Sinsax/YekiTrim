# polish 精修模块：整合优化与解耦（扩展性）方案集

> 立档：2026-10。目标：**让"加装一个新功能"变便宜** —— 从"改 11 处、靠 41 条门禁查漏"
> 降到"改 1–2 处、漏了门禁点名"。
>
> 配套：`docs/plan/polish-decoupling-assessment.md`（体检报告 · 实测清单）、
> `docs/plan/effect-pipeline-unification.md`（既有蓝图 S1–S3 已完成）、
> `docs/plan/s4-second-chain-removal-decision.md`（S4 决策）、
> `docs/plan/polish-enrichment-proposal.md`（R1–R4 红线）。
>
> **本文件只做方案对比，不含改动。**

---

## 0. 一句话结论

**瓶颈不是"门禁不够"，而是"效果模型是枚举而非注册表"。**

底座 Dhampir 用 `EffectSpec` 注册表做到了"加一个特效 = 加一行声明"；
本仓用 `Event` enum（12 个变体 × 各自字段），于是一个效果要在 **4 个语言里各写一遍**。
41 条门禁是在给这个形状**打补丁**，不是在解决它。

---

## 1. 实测基线（全部已验证）

### 1.1 规模

| 四份渲染实现 | 行数 | | 前端精修面 | 行数 |
|---|---:|---|---|---:|
| `tools/polish-to-dhampir.mjs` | 7487 | | `PolishPreview.vue` | 3767 |
| `templates/index.html` | 3130 | | `TimelineEditor.vue` | 3323 |
| `webui/src/polish/engine.js` | 2571 | | `PolishView.vue` | 2770 |
| `render-core/src/timeline.rs` | 1526 | | `CoverDesigner.vue` | 2290 |
| **合计** | **14714** | | **合计** | **12150** |

### 1.2 门禁基线（当前**全绿**）

```
python3 scripts/check_module_map.py        → 35 模块 / 7 根，全部合规
python3 scripts/check_module_boundaries.py → 17 条规则 / 415 文件，0 违规
node scripts/check-effect-registry.mjs     → 155 个消费点与词汇表一致
```

门禁规模：`scripts/check-*.mjs` **41** 条 + `webui/scripts/check-*.mjs` **10** 条 + `scripts/check-*.py` 1 条。

### 1.3 生产路径现状（关键）

| 实现 | 生产状态 | 证据 |
|---|---|---|
| `polish-to-dhampir.mjs` | ✅ **出片唯一通道** | 根卡："出片默认就是底座" |
| `engine.js` | ✅ **预览默认腿** | `dhampirMode = ref(false)` —— DOM 默认，底座要手动开 |
| `index.html` | ⚠️ 仅预览服务页 + 对拍锚点 | `INDEX_HTML` 只被 `render/server.rs` 用；canvas `renderFrame` 无生产消费者 |
| `timeline.rs` | ❌ **零消费者**（死代码） | 全仓 grep 无生产引用 |

> ⚠️ **结构性矛盾**：出片走底座、预览走自绘移植 ⇒ **两边的"效果模型"根本不是同一个**。
> 这是"改一处漏三处"的最终来源。

### 1.4 跨仓对拍**真跑真绿**（纠正早前误判）

`../Dhampir` **在磁盘上**，三个跨仓守卫实测通过：

| 守卫 | 判据 |
|---|---|
| `check-text-motion-parity.mjs` | 54 项同口径 |
| `check-perchar-parity.mjs` | **560** 个对拍点（546 非零） |
| `check-sway-parity.mjs` | **68190** 个对拍点（20130 非零） |

⇒ `engine.js` 里 9 处"逐字照抄 Dhampir"的注释，**是被守住的**，不是裸奔。

---

## 2. 核心诊断：底座已有正确答案

### 2.1 底座的模型（`../Dhampir/crates/dhampir-timeline/src/schema.rs:629`）

```rust
pub struct EffectSpec {
    kind: &'static str,                            // 类型串
    params: &'static [(&'static str, f32, f32)],    // 参数名 → (min,max)
    space: EffectSpace,
    pipeline: EffectPipeline,                        // 渲染器按它派发
    window_default: Option<WindowDefault>,           // 瞬时特效默认时长
}
pub const REGISTRY: &[EffectSpec] = &[ GAUSSIAN_BLUR, FLASH, SHAKE, VIGNETTE, ... ];
```

磁盘上的工程形状：
```json
{ "kind": "flash", "params": {"amount":0.2,"r":1,"g":1,"b":1},
  "window": {...}, "opacity": 1.0 }
```

`params` 是**通用 `BTreeMap<String,f32>`**。底座 schema 自己写了理由：

> `window` 与 `opacity` 分开而不是把强度塞进 `params`：它们是**所有**特效共有的量，
> 而 `params` 是每个特效私有的。混在一起的话 `params` 的键空间会被通用名污染。

**加一个特效 = 在 `REGISTRY` 里加一行常量。**

### 2.2 本仓的模型（实测 `Event` enum 12 个变体）

**通用字段**（出现在 ≥8/12 变体里）：
```
time 12/12 · end 12/12 · sfx 9/12 · sfx_volume 9/12 · anchor 9/12
```

**字段集完全相同的变体组**（★ 病根）：
```
[Flash, HueShift]               → time,color,end,anchor,sfx,sfx_volume
[Blur, Vignette, Noise, Pulse]  → time,end,anchor,sfx,sfx_volume
```

`Blur/Vignette/Noise/Pulse` 是**四个字段一模一样的 enum 变体** ——
在 Rust 是四个类型、转译器四个 `case`、模板四个 handler、`engine.js` 四个求值器，
**为的是同一个形状换四个名字**。对照底座：这四行只是 `params` 为空的一条 `EffectSpec`。

### 2.3 成本对照

| 动作 | 本仓（enum） | 底座（注册表） |
|---|---|---|
| 加一个零参数特效 | **11 处** | **1 行** |
| 加一个参数 | enum + 所有 match 分支 | `params` 数组加一项 |
| 类型分派点 | **296 处 `Event::`** | 1 处 `spec_of()` 线性查找 |

### 2.4 ★ 迁移可行性的关键发现

`Event` 用的是 **`#[serde(tag = "type", rename_all = "snake_case")]`（内部标签）** ⇒
磁盘上**已经是** `{"type":"shake","intensity":14,...}` 的扁平形状 ——
**与"注册表 + params map"的形状天然同构**。这让迁移比预想**便宜得多**：
`type` 键已存在，私有参数只是从"变体字段"挪到"`params` 子对象"，且可 `serde(default)` 兜底。

---

## 3. 方案集

四个方案，按**改动面从小到大**排列。每个都可以独立成立、独立回退。

---

### 方案 A：低成本收口（不动数据模型）

**做什么**：只消灭"没有单一权威"的手抄点，让扩展成本从 11 处降到 **8 处**。

| # | 动作 | 落点 |
|---|---|---|
| A1 | `INDEXED_TYPES` 从词汇表派生 | `engine.js:293` |
| A2 | `LANE_OF_TYPE` 从词汇表派生（消灭四份手抄） | `engine.js:304` / `index.html:300` / `polish-to-dhampir.mjs:2704` |
| A3 | 事件**字段**纳入词汇表 + 门禁 | `spec/effects.json` 扩展 schema |
| A4 | 修两处 doc rot | `PORTRAIT_SWITCH` 幽灵、`__CAM_STATE` 说错层 |
| A5 | 删 `timeline.rs`（1526 行死代码） | `render-core` |

**收益**：加效果 11 → 8 处；加字段 7 处**零门禁** → 有门禁点名。
**风险**：**低**。不碰数据模型、不碰出片。
**代价**：**没有解决根本问题** —— 枚举形状不变，4 份实现仍在。
**已知坑**：A2 会打断 `check-preview-engine.mjs` 的变异台（它把 `engine.js` 拷到临时目录，
相对依赖要一起搬）。我实测过这条，需要同步改门禁。

---

### 方案 B：同形状变体合并（最小化的"注册表"验证）

**做什么**：把 `[Blur, Vignette, Noise, Pulse]` 这 **4 个字段完全相同的变体**
合并成一个带 `params` 的变体，作为"注册表模型"的**垂直切片试点**。

```
// 之前：4 个变体，各写一遍
Blur { time, end, anchor, sfx, sfx_volume }
Vignette { time, end, anchor, sfx, sfx_volume }
Noise { time, end, anchor, sfx, sfx_volume }
Pulse { time, end, anchor, sfx, sfx_volume }

// 之后：1 个变体 + 参数表
Fx { time, end, anchor, sfx, sfx_volume, #[serde(flatten)] params: FxParams }
```

**收益**：验证"注册表确实更便宜"；这 4 类的 4×4 处实现降到 4×1。
**风险**：**中**。要迁移 4 个类型的磁盘格式（有 `serde` 兼容手段，但需老工程回归）。
**代价**：只动了 4 个类型，其余 8 个仍是老样子 ⇒ **一段时间内两种模型并存**。
**前置**：必须先做方案 A3（字段级门禁），否则迁移漏了没人知道。

---

### 方案 C：事件模型对齐底座（根治）

**做什么**：把 `Event` enum 改成"公共头 + `kind` 字符串 + `params` map"，
与底座 `EffectSpec` 同构。**这是唯一能让"加效果 = 词汇表一行 + 转译器一个 case"的方案。**

```rust
struct Event {
    time: f64, end: Option<f64>, anchor: Option<String>,      // 通用头（12/12、9/12）
    sfx: Option<String>, sfx_volume: Option<f64>,              // 通用头
    kind: String,                                              // 类型串
    #[serde(default)] params: BTreeMap<String, Value>,         // 私有参数
    manual: bool,
}
```

**收益**：加效果 **11 处 → 2 处**；`296` 处 `Event::` 分派里纯机械的那批消失。
**风险**：**高**。但 §2.4 的发现（内部标签、形状已同构）把它从"重写"降到"搬迁"。
**代价**：12 个变体全量迁移；`types.rs` 的 49 处 `skip_serializing_if` 要重排。
**前置**：必须有 R4（老工程逐字节一致）的回归判据 —— 目前**没有专门的 round-trip 金样门禁**，
要先补。
**依赖**：方案 A3 + B 的成功经验。

---

### 方案 D：预览与出片真相合一（S4 收口）

**做什么**：让预览与出片走同一条链，消灭"手工移植"这一整类耦合。

分两个子问题：
- **D1**：让 Dhampir 成为**默认**预览腿（现在是可选开关，`dhampirMode = ref(false)`）
- **D2**：DOM 自绘腿降级为**纯兜底**（或按 S4 的 B2 口径保留但不再要求逐像素一致）

**收益**：`engine.js`（2571 行）从"必须与底座逐像素一致的手工移植"降为"兜底近似" ⇒
**跨仓对拍门禁的负担大幅下降**。
**风险**：**中高**。若 DOM 腿必须逐像素一致，那就**永远要维护两份**，本方案收益归零。
**代价**：取决于 §4 的产品决策。
**依据**：`s4-second-chain-removal-decision.md`（阻断项 B 已拍板 = B2，部分已执行）。

---

## 4. 方案对比总表

| | A 收口 | B 合并试点 | C 对齐底座 | D 真相合一 |
|---|---|---|---|---|
| **加效果触点** | 11 → 8 | 11 → 8（4 类） | **11 → 2** | 不变 |
| **加字段触点** | 7 → 有门禁 | 同 A | **7 → 2** | 不变 |
| **风险** | 低 | 中 | 高 | 中高 |
| **可独立回退** | ✅ | ✅ | ⚠️ 数据迁移 | ✅ |
| **动数据模型** | ❌ | ⚠️ 4 类 | ✅ 全部 | ❌ |
| **动出片** | ❌ | ⚠️ | ⚠️ | ✅ |
| **解决根本问题** | ❌ | 部分 | ✅ | 部分 |
| **前置依赖** | 无 | A3 | A3 + B + R4 门禁 | 产品决策 |

---

## 5. 建议路线（推荐）

**A → B → C**，D 并行评估。理由：

1. **A 立刻可做、零风险、且是后续一切的前置**（尤其 A3 字段级门禁 —— 没有它，B/C 的迁移漏改无人发现）。
2. **B 是"最小可信验证"** —— 用 4 个语义安全的同形状变体证明注册表模型真的更便宜。若 B 的收益不如预期，C 就该放弃。
3. **C 是唯一能达成你原始目标（"加装功能只改一处"）的方案**，但要等 B 的实证。
4. **D 单独走产品线** —— 它的瓶颈是决策（预览要不要逐像素）而不是技术。

**我不建议直接上 C**：它一次性迁移 12 个变体，且当前**缺 R4 的 round-trip 金样门禁**，
属于"没有安全网的高空作业"。

---

## 6. 待你拍板的三件事

1. **路线选哪个**：A / A→B→C / 直接 C / 其他组合？
2. **预览与出片要不要逐像素一致**（决定 D 的形态，也影响 A/B/C 的收益上限）？
3. **近期真正想加的 2–3 个功能是什么**？
   —— 这是最准的输入：我可以据实测算"今天要改几处、改完要几处"，用真实需求验证方案收益，
   而不是停在抽象层面。

---

## 7. 本方案的证据等级

| 等级 | 内容 |
|---|---|
| **一手实测（本文件）** | 行数、门禁运行结果、`Event` 变体字段解析、跨仓对拍 3 条实跑、`EffectSpec` 源码、`dhampirMode` 默认值、serde tag 方式 |
| **仓库文档** | 根 `AGENTS.md`、`effect-pipeline-unification.md`、`s4-*.md`、`polish-enrichment-proposal.md`、各模块卡 |
| **已纠正的早前误判** | ① 跨仓对拍"静默 skip" → 实为真跑真绿；② `window.__CAM_STATE` "不存在" → 实存在于 `index.html:1553`，但文档说错了层；③ 门禁"约 30 条" → 实为 41+10 |
