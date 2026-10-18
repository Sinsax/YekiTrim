# polish × 底座：包含底座改造的**统一长期架构**

> 立档：2026-10。上游：`docs/plan/polish-longterm-architecture.md`（本仓侧四层收敛）、
> `docs/plan/polish-extension-cost-forecast.md`（成本预测）。
>
> **本文件的区别**：前面几份都把底座当**固定约束**，于是"加全新能力"这条路被堵死。
> 本文件**把底座纳入改造范围**，回答"能不能做到任意新功能、甚至底座没有的功能"。
>
> **本文件只做设计，不含改动。**

---

## 0. 结论先说

**能，而且这才是真正的根本解 —— 但有一个必须先摆平的硬事实。**

| 问题 | 答案 |
|---|---|
| 加一个"底座已支持、本仓没接"的特效 | ✅ 现在就能做，本仓 5 处 |
| 加一个"底座没有、但落在已有 4 条管线内"的特效 | ✅ **改底座 4–5 处 + 本仓 2 处** |
| 加一个"需要全新像素算法"的特效 | ✅ 可行，但要**加一条管线 + 一份 WGSL** |
| 加一个新的**图层类型/文字玩法** | ✅ 本仓侧（机制③），底座已泛用 |
| 加一个新的**运镜** | ⚠️ 底座侧**是定长槽位**，要改结构 |

### 0.1 硬事实：底座**不是**完全数据驱动的

底座自己的 `plan/effects-modularization.md` §0 写着一句原话：

> 一句话：**数据可扩展、渲染不可扩展。**

我逐行核实了这句话，它**今天仍然成立**：

| 面 | 现状 | 加一个特效 |
|---|---|---|
| **数据面** | ✅ `Effect { kind: String, params: BTreeMap<String,f32>, window, opacity }` | 加一行 |
| **渲染面** | ❌ **16 个硬编码字符串** 派发到 **3 个定长 struct** | **改结构 + 改 WGSL** |

实测 `render/timeline.rs` 的 16 条字符串分支：
```
brightness brightness_multiply contrast saturation hue hue_rotate_css saturation_css
flash vignette noise overlay          ← ColorMask
shake zoom_bounce pulse split         ← Warp
```

而三个 uniform 结构是**定长槽位**（`#[repr(C)]`，裸内存布局）：

```rust
pub struct ColorMaskParams {
    flash_amount, flash_r, flash_g, flash_b,        // flash 占 4 个槽
    vignette_amount, vignette_radius, vignette_softness,
    noise_amount, noise_seed,
    overlay_amount, overlay_r, overlay_g, overlay_b, // overlay 占 4 个槽
}
```

> ⚠️ 这正是底座在 `p9-polish-parity-design.md` §1 里**批评本仓的那种形状**
> （"`EffectUniform` 一个定长 struct…那正是要摆脱的形状"）——
> **它自己也还留着一份**，只是藏在"数据面已泛用"的表象后面。

---

## 1. 关键技术事实（决定架构形态）

### 1.1 归一化是"管线的职责"

```rust
pub enum EffectPipeline { SeparableBlur, ColorAdjust, ColorMask, Warp }
```

管线的注释把设计意图写得很清楚：

> 按管线派发而不是按 kind 字符串，加特效就不必改渲染主路径 ——
> 而"忘了改"正是字符串派发最容易出的错（**S3 修的就是这个**）。

**⇒ 底座已经把"渲染主路径"这一层做对了**，残留的字符串派发只在
"kind → 定长槽位"这一步。

### 1.2 WGSL 只有一份，两端共用

`shaders/*.wgsl` 共 9 份，wasm 与 native 读**同一份文本**。
⇒ **写一次着色器，预览与出片同时生效** —— 这是底座最值钱的性质。

### 1.3 底座是独立仓，但本仓**能**构建它

- 底座是**独立 Cargo workspace**（`../Dhampir`），本仓**不依赖它任何 crate**；
- 消费方式是**产物**：`dhampir` 可执行 + wasm `pkg/`；
- `find_binary()` 的候选链含 **`Dhampir/target/{release,debug}`** ⇒
  **源码树形态下本仓直接用底座构建产物**。

**⇒ "改底座"在工程上是通的**，代价是**跨仓协同**（版本线 `project_schema` / `host_api` 要对齐）。

---

## 2. 统一架构：一张图

把本仓与底座**放在一起**看，扩展点是**一条链路**：

```
┌──────────────────────── 本仓 YekiTrim ────────────────────────┐
│ L0  spec/effects.json  ← 唯一能力清单（id/params/window/lane） │
│                              │ 派生 + 门禁                    │
│    ┌────────────┬────────────┼────────────┬────────────┐      │
│    ▼            ▼            ▼            ▼            ▼      │
│  L1 类型模型  L2 转译器   L3 预览     L4 编辑器    L5 后处理   │
│ 公共头+kind  声明式映射  wasm 主腿   控件生成     fx.rs 表   │
│  +params                                                      │
└───────────────────────────┬───────────────────────────────────┘
                            │ 转译：polish.toml → 底座工程
                            ▼
┌──────────────────────── 底座 Dhampir ─────────────────────────┐
│ D1 数据面   Effect{kind, params:BTreeMap, window, opacity} ✅  │
│ D2 管线     EffectPipeline 4 条  ← 按管线派发（S3 已做对）     │
│ D3 槽位     ⚠️ 定长 uniform struct + 字符串派发（残留）        │
│ D4 着色器   一份 WGSL，两端共用 ✅                             │
└───────────────────────────────────────────────────────────────┘
```

**扩展点的唯一落点 = L0**；其余各层由 L0 派生。底座侧的分工是：
- **D2 已有** ⇒ 新特效落在 4 条管线内时，本仓**可以做到"只改一处"**
- **D3 是瓶颈** ⇒ 新算法仍需动底座

---

## 3. 分场景：能做到什么程度

### 场景 A：底座已有、本仓没接（8 个）

`brightness_multiply · hue · hue_rotate_css · saturation_css · split · zoom_bounce`
（`pulse`/`shake` 虽在注册表，本仓走关键帧，不算）

**改动**：本仓 5 处（词汇表/类型/fx.rs/转译 case/预览求值器）。
**底座**：0 处。**⇒ 现在就能做。**

### 场景 B：新特效，落在已有管线内（★ 本仓可"只改一处"）

例如"色偏""扫描线""老电影颗粒" —— 只要能表达成
`ColorAdjust`/`ColorMask`/`Warp`/`SeparableBlur` 的**参数组合**。

> ⚠️ 但**仅在底座也认这个 kind 的前提下**：D3 的字符串派发必须加分支 +
> 定长 struct 必须加槽位（若现有槽位能复用，则 0 处）。

**最理想情况**（复用现有槽位，如新 kind 只是 `flash` 的别名）：
**⇒ 底座 1–2 处 + 本仓 2 处**（P2 完成后）。

### 场景 C：新特效，需要全新像素算法

**底座最小改动清单（实测）**：

| # | 位置 | 改动 |
|---|---|---|
| ① | `effects.rs` | `EffectSpec` 常量 + `REGISTRY` 登记（2 处） |
| ② | `render/<pipeline>.rs` | Params 结构加槽位（1 处） |
| ③ | `render/timeline.rs` | 字符串派发加分支（1 处） |
| ④ | `shaders/<pipeline>.wgsl` | **算法本体**（1 处） |
| ⑤ | 数据面新字段（若需要） | `schema.rs` + 序列化（1–2 处） |

**⇒ 底座 5–7 处 + 本仓 2 处。**
**关键优势**：底座改完，**wasm 与出片同时生效**（一份 WGSL 两端共用）——
比在本仓维护两份实现**便宜得多**。

### 场景 D：新图层类型 / 文字玩法

底座图层面**已泛用**（`Layer` 有 `effects`/`keyframes`/`transform`/`mask`/`shadow`…），
且 `Keyframe.target` 支持**寻址表达式**：

```
"opacity" / "x" / "y" / "scale" / "rotation"
"effect.<下标>.<参数名>"     ← ★ 让"驱动某条特效的参数"成为通用能力
```

**⇒ 本仓侧改（机制③），底座基本不动。**

### 场景 E：新运镜

底座运镜 = `Keyframe` 驱动 `scale`/`x`/`y`，**数据面是泛用的**。
本仓 `camera` 的 12 个字段（`zoom`/`anchor`/`to_origin`/`speed`/`tilt`…）
在转译器里展开成关键帧。

**⇒ 新增"运镜风格"（如新的缓和曲线）= 本仓侧，底座不动。**
**⇒ 新增"运镜能力"（如旋转运镜）= 需底座 `TransformV2` 支持**（现只有 `rotation/scale/scale_y/x/y`，够用）。

---

## 4. 因此：真正的最优方案

### 4.1 核心判断

**不该把底座当固定约束，也不该把"改底座"当常态。** 正确的分工是：

| 加什么 | 改哪 | 理由 |
|---|---|---|
| **参数/组合/风格** | **只改本仓**（L0 + 派生） | 底座数据面已泛用 |
| **像素算法** | **改底座**（一份 WGSL，两端生效） | 在本仓写等于维护两份 |
| **图层/文字玩法** | **只改本仓** | 底座图层面已泛用 |
| **新管线**（如几何形变、光流） | **改底座**（加 pipeline） | 那是渲染器能力 |

### 4.2 建议的两阶段架构改造

**阶段 1（本仓）：L0 成为唯一扩展点** —— 即 `polish-longterm-architecture.md` 的 P0–P3。
**阶段 2（跨仓）：把 D3 的定长槽位消除** —— 这是"任意新特效都便宜"的最后一公里。

### 4.3 D3 消除方案（底座侧，建议提给底座仓）

把"定长槽位 + 字符串派发"换成**按注册表布局的参数缓冲区**：

```rust
// 现在：每个特效占死槽位
struct ColorMaskParams { flash_amount, flash_r, flash_g, flash_b, vignette_amount, … }

// 目标：一个参数缓冲区 + 按 EffectSpec.params 的布局
struct PipelineParams { values: [f32; MAX_PARAMS], count: u32 }
```

底座 `p9-polish-parity-design.md` §3 原则 2 **已经写了这条**：
> **参数是 map 不是 struct。** 参照实现用 `EffectUniform`（定长、字段写死、
> 注释里编号复用 `misc.xyz`）。改成"一个参数缓冲区 + 按注册表布局"，**参数个数不受限**。

**⇒ 这不是新提案，是把底座自己已写下的设计原则执行到底。**
消除后：**加一个特效 = 注册表加一条 + 若需新算法则加一份 WGSL**，
本仓侧 **1–2 处**。

---

## 5. 路线图（含底座）

| 阶段 | 范围 | 收益 | 风险 |
|---|---|---|---|
| **P0** | 本仓 L0 扩展（params 结构化 + 字段门禁） | 一切的前置 | 低 |
| **P1** | 本仓 L4 控件生成 | 加参数 → 0 处手改 | 低 |
| **P2** | 本仓 L1/L2 声明式（机制①） | 加同类特效 **11 → 2 处** | 中 |
| **P3** | 本仓 L3 预览主腿（wasm 转正） | 消灭"两份实现" | 中高（需决策） |
| **P4** | 本仓机制②③泛化（运镜/图层） | 新元素/新运镜 → 2–3 处 | 高 |
| **P5** | **底座 D3 去槽位化**（跨仓） | **任意新特效 → 2 处** | 中（底座仓配合） |

**关键里程碑**：P5 完成后，**加任何落在既有管线内的特效，只需改本仓 2 处 + 底座 1–2 处**，
且 wasm/出片**同时生效**。

---

## 6. 必须说清的三条边界

1. **底座不接受可上传 shader**（schema 原话：*"那等于把两端一致性交给用户"*）。
   ⇒ "热插拔特效"这条路**被设计否决了**，新算法必须进底座仓。
2. **跨仓协同有版本线约束**：`project_schema`（工程能否读）+ `host_api`（wasm 导出面），
   启动时**两条都要校验**。改底座要同步升这两条。
3. **本仓目前缺 R4 round-trip 金样门禁** ⇒ P2/P5 之前**必须先补**，
   否则数据模型迁移没有回归判据。

---

## 7. 待你拍板

1. **要不要把 P5（底座 D3 改造）纳入计划？**
   - 纳入 → 需要与底座仓协同，能达成"任意新特效都便宜"
   - 不纳入 → 新算法永远要手工改底座结构，且本仓只能等

2. **你近期要加的功能，属于哪一类**（A/B/C/D/E）？
   我据实测算"本仓几处 + 底座几处"。

3. **P3 的产品决策**：预览要不要与出片逐像素一致？

---

## 8. 证据等级

| 等级 | 内容 |
|---|---|
| **一手实测** | `render/timeline.rs` 16 条字符串派发全文、`WarpParams`/`ColorAdjustParams`/`ColorMaskParams` 定长槽位源码、`EffectPipeline` 4 条枚举、9 份 WGSL 清单、`warp.wgsl` 槽位对应、`find_binary()` 候选链含源码树、底座 Cargo workspace 成员与依赖方向、`Keyframe.target` 寻址语法、图层 `Layer` 属性面 |
| **对方仓文档** | `plan/effects-modularization.md`（"数据可扩展、渲染不可扩展" + S1–S6c 完成）、`plan/p9-polish-parity-design.md`（§3 原则 2「参数是 map 不是 struct」+ §1 对本仓形状的批评）、`schema.rs::EffectSpec`/`EffectPipeline`、`docs/api-surface.md` 两条版本线 |
