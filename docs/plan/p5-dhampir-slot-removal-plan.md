# P5 实施方案：底座去掉定长槽位（D3 去槽位化）

> 状态：**步 1 已完成**（底座仓，未提交）；步 2–5 待做。
> 归属：改动落在 **Dhampir 仓**（与 YekiTrim **同一作者** —— 两边提交者都是 `Sinsax`，
> 且底座近期提交就是"牵动 YekiTrim 39 档判据"这类**联动改动**）。
> 上游依据：`docs/plan/polish-extensibility-master-plan.md` §3.3 / §3.4。

---

## 0. 进度

| 步 | 内容 | 状态 |
|---|---|---|
| 1 | 注册表加 `slots` 声明（**零行为变更**） | ✅ **完成**（见 §10） |
| 2 | 加判据：声明与 `timeline.rs` 那份 match 逐条对拍 | ✅ **完成**（5 条测试 + 变异自证） |
| 3 | **色罩管线切换**（四个 kind 读登记表，手写 match 已删） | ✅ **完成**（见 §11） |
| 3b | **色调整 + Warp 切换**（三处 match 全删） | ✅ **完成**（见 §12） |
| 4 | 删掉旧 match | ✅ **三处全删**（全仓 `match spec.kind` 只剩注释） |
| 5 | 发底座新版本；本仓 `check-dhampir-wiring` 复验 | ⬜ 待做 |

---

## 1. 为什么单独一份文档

P5 与前五阶段**性质不同**：

| | P0–P4 | P5 |
|---|---|---|
| 落点 | YekiTrim 仓 | **Dhampir 仓** |
| 版本线 | 本仓 | 独立（`dhampir-0.3.0`） |
| 回退 | 本仓 `git revert` | 要发底座新版本 |
| 影响面 | 本仓出片/预览 | **所有底座消费者** |

⇒ **不该由本仓单方面推改动**。这份文档的作用是把"要改什么、为什么安全、怎么验证"
写成可直接执行的形态，交给底座仓排期。

---

## 2. 现状（实测，非推测）

### 2.1 三处 `match spec.kind`

`crates/dhampir-core/src/render/timeline.rs` 有 **3 处** `match spec.kind`，
共 **15 个硬编码 kind 分支**（文档里"16 个"是把 `split` 等历史项也算进来的口径）：

| 位置 | 分支数 | kinds | 落进哪个定长结构 |
|---|---|---|---|
| `:364` | 7 | `brightness` `brightness_multiply` `contrast` `saturation` `hue` `hue_rotate_css` `saturation_css` | `ColorAdjustParams` |
| `:532` | 4 | `flash` `vignette` `noise` `overlay` | `ColorMaskParams` |
| `:645` | 4 | `shake` `zoom_bounce` `pulse` `split` | `WarpParams` |

### 2.2 定长槽位结构（`#[repr(C)]`，与 WGSL **逐字段对齐**）

| 文件 | struct | 槽位数（**实测**） |
|---|---|---|
| `render/color_adjust.rs:29` | `ColorAdjustParams` | **7**：`brightness` `contrast` `saturation` `hue` `scale` `hue_css` `saturation_css` |
| `render/color_mask.rs:37` | `ColorMaskParams` | **21**：`flash_*`(4) `vignette_*`(3) `noise_*`(2) `overlay_*`(9) `width` `height` `frame`(3) |
| `render/warp.rs:30` | `WarpParams` | **13**：`shake_*`(3) `bounce_*`(2) `pulse_*`(2) `split_*`(3) `width` `height` `seconds`(3) |

**合计 41 个定长槽位**。`ColorMaskParams` 的**前缀命名**就是"一个 kind 占一组固定槽位"的化石证据。

### 2.3 每组的 combine 规则**各不相同**

这是泛化的真正难点，**不是**"统一乘个系数"：

| kind | combine | 位置 |
|---|---|---|
| `flash` | `if weight > out.flash_amount` （严格大于） | `:547` |
| `vignette` | `if weight >= out.vignette_amount` | `:558` |
| `noise` | `if weight >= out.noise_amount` | `:570` |
| `overlay` | `over(out.overlay_amount, weight)` （over 合成） | `:595` |
| `brightness` 等 | `+=` 或 `*=`（可交换） | `:364` 段 |

⚠️ **四种规则都是刻意写的**（各自注释都写着"为什么不是另一种"）——
去槽位化**必须把这四种原样带过去**，不能"统一"。

---

## 3. 已确认的**安全前提**（这是 P5 可做且低风险的根本原因）

| 项 | 实测结论 |
|---|---|
| 要不要改 `project_schema` | ❌ **不用** —— `Effect.params` 已是自由 map（`additionalProperties:number`），工程文件形状不变 |
| 要不要改 `host_api` | 通常**不用**（导出面不变）；仅当新增导出才升 |
| 老工程 | **仍可读**（schema 中性） |
| 当前版本线 | `project_schema=1` · `host_api=6` · 产物 `dhampir-0.3.0` |
| 验证回路 | `scripts/yekitrim-compare.mjs`（已标定三样本阈值） |

⇒ **改动限于渲染器内部**：把"定长槽位 + 手写 match"换成
"**参数缓冲区 + 按注册表布局**"，对外契约定向不变。

### 3.1 ★ 关键发现：**注册表已经存在**（P5 比原估**小得多**）

原计划把 P5 描述成"参数缓冲区 + 按注册表布局"，读起来像**要先建一张注册表**。
实测：**注册表早就在了** —— `crates/dhampir-core/src/effects.rs`：

```rust
pub fn spec_of(kind: &str) -> Option<&'static EffectSpec>   // :321
pub fn kinds() -> Vec<&'static str>                          // :326
const REGISTRY: &[EffectSpec] = &[ /* 每条一个 const */ ];
```

而 `EffectSpec`（定义在 `dhampir-timeline/src/schema.rs:629`）**已经有**
`kind` / `params`（名→范围）/ `space` / **`pipeline`** / `window_default`。

⇒ 现状是**"注册表已登记 kind，但渲染时仍按定长槽位手写 match"**：

```
spec_of(kind)  →  拿到 EffectSpec（含 pipeline）
                      ↓
             但 timeline.rs 里仍有一份 match spec.kind { ... }  ← P5 要拆的就是这一层
                      ↓
                  写进定长 ColorMaskParams
```

**所以 P5 真正要做的是**：给 `EffectSpec` 补一列
「**写哪些槽位 + 用哪条 combine + 参数名→槽位**」，
然后把那 3 处 `match` 换成一次查表。**不用新建注册表、不用改 schema。**

（这与本仓 P2 的形态**同构**：那边也是"词汇表已有、但转译器仍手写 case"，
做完之后加同类效果只需改词汇表一处。跨仓两侧的**同一种病、同一种治法**。）


---

## 4. 目标形态

### 4.1 注册表驱动

**已存在**（见 §3.1），只需给 `EffectSpec` 补一列：

```
EffectSlot { name, offset, combine: Combine }

enum Combine {
    MaxStrict,        // flash:  weight >  cur      —— timeline.rs:547
    MaxInclusive,     // vignette/noise: weight >= cur  —— :558 / :570
    Over,             // overlay: 1-(1-a)(1-b) clamp    —— :595
    Add,              // brightness/hue
    Mul,              // scale/contrast/saturation
}
```

**✅ 已抄下来的确切语义**（照搬，别重写）：

```rust
// timeline.rs:517 —— over 的确切实现（含 clamp）
fn over(a: f32, b: f32) -> f32 {
    (1.0 - (1.0 - a) * (1.0 - b)).clamp(0.0, 1.0)
}

// color_mask.rs:112 —— overlay_shape 的枚举映射（round 后比 EPSILON）
pub fn from_param(value: f32) -> Self {
    let rounded = value.round();
    if (rounded - 1.0).abs() < f32::EPSILON { Self::Linear }
    else if (rounded - 2.0).abs() < f32::EPSILON { Self::Radial }
    else { Self::Solid }
}
```

⚠️ 注意 `flash` 分支里还有一条**非 combine 的副作用**：它**只在真正胜出时才写颜色**
（`if weight > out.flash_amount { out.flash_r = ...; }`）—— 泛化时不能把"写参数"和"判胜负"
合成一步，否则**输的那次也会改颜色**。

### 4.2 三处 match 变成一次查表

```rust
if let Some(reg) = registry.get(spec.kind) {
    reg.apply(&mut params, |k| param(k), weight);
}
```

### 4.3 WGSL 侧

`ColorMaskUniform` 的**字段顺序**要么保持（则偏移表就是现在这份）、
要么改成 `array<f32, N>` + 偏移常量 —— **两者都行**，
但 `#[repr(C)]` 与 WGSL 的**对齐规则**必须逐字段核对（现在两边注释都写着"必须一致"）。

---

## 5. 判据（**逐字节 / 逐像素**，不是"看着差不多"）

P5 的验收线必须是**数值等价**，理由见 §6：

1. **单元级**：`ColorMaskParams::IDENTITY` 跑一遍输出与输入**逐像素相同**（现有判据，保留）。
2. **对拍级**：`node scripts/yekitrim-compare.mjs --golden docs/golden/dhampir-parity/<样本>`
   三样本（`demo` / `portrait` / `real`）判据全过，并且**中位像素差与重冻前一致**
   （`15.07` / `9.90` / `11.80`）。
   ⚠️ 这三个数**必须一模一样** —— 去槽位化是纯重构，
   中位数变了就说明 combine 规则搬错了。
3. **本仓联动**：`node scripts/check-yekitrim-translator.mjs` + `check-dhampir-drop-list.mjs`
   在**新底座**下全绿（DROP 仍为 1 / CLAMP 仍为 0）。
4. **端到端**：本仓演示工程转译产物**逐字节不变**
   （判据 `ba9cc60e8dd42c85`，与 P2/P4 用的是同一条）。

---

## 6. ★ 为什么必须"逐像素一致"而不是"看着对"

本仓 P2/P4 的实测教训：**5 个 bug 全部通过了单元测试，只有整份产物 hash 抓得住**。

其中与 P5 同型的一条值得写在这里（P4 现场）：

> `collect.fields` 里的 `origin` 是**原始字符串**（`"liver"`），而记录里那个键放的是
> **解析后的点** `{x,y}`。`...carried` 展开让原始值把派生值**顶掉** ⇒
> `e.origin.x` 是 `undefined` ⇒ `NaN` ⇒ **806 个关键帧变成 `null`**。
> 转译器不报错、DROP/NOTE 一条没变 —— 只有产物 hash 抓得住。

P5 的槽位布局有**同一个形状的风险**：`offset` 写错一个数、combine 用错一档，
**编译通过、测试通过、报告全绿**，只是画面轻微不对。
⇒ 所以验收线必须是**像素级**，且**三样本的数必须与重冻前逐位相同**。

---

## 7. 建议的执行顺序（每步可回退）

| 步 | 动作 | 回退点 |
|---|---|---|
| 1 | 建注册表，**先不改**任何行为（原 match 仍在，注册表只被测试读） | 删文件即可 |
| 2 | 加判据：注册表登记的槽位/combine 与现有 match **逐条对拍** | 同上 |
| 3 | 逐管线切换（`color_adjust` → `color_mask` → `warp`），**每切一条跑一次三样本** | 逐条 revert |
| 4 | 删掉旧 match | 前一步的 commit |
| 5 | 发底座新版本；本仓 `check-dhampir-wiring` 复验 | 底座可钉旧版本 |

⚠️ **第 3 步不许三条一起切** —— 三条管线的槽位布局不同，
一起切出问题时**无法定位是哪一条**（这正是本仓 P2 一次删 4 条 case 时踩过的：
幸好有整份 hash，否则四个 case 里哪个错了根本不知道）。

---

## 8. 收益（对照计划 §3 的里程碑）

P5 完成后达成的判据是：

> **加任意新特效（含新像素算法）= 本仓 2 处 + 底座 1–2 处，且 wasm 与出片同时生效（一份 WGSL）。**

当前（P0–P4 完成后）已达成的是**落在既有 4 条管线内**的特效：
**本仓只改 1 处**（`spec/effects.json`），**底座 0 处**。

⇒ P5 补的是**跨出既有管线**那一档（新 WGSL 算法）。

---

## 9. 明确不做

| 项 | 原因 |
|---|---|
| 上传 shader / 热插拔 | 底座 schema 明示否决：*"那等于把两端一致性交给用户"* |
| 改 `project_schema` | 不需要（§3），动了反而破坏存量工程兼容 |
| 把 4 种 combine 统一 | 它们各自是**创作语义**（§2.3），统一会改行为 |

---

## 10. ★ 步 1 执行记录（✅ 已完成，底座仓工作副本、**未提交**）

### 10.1 改了什么

| 文件 | 改动 |
|---|---|
| `dhampir-timeline/src/schema.rs` | 新增 `EffectSlot` / `SlotCombine` 两个类型；`EffectSpec` 加一列 `slots: &'static [EffectSlot]` |
| `dhampir-core/src/effects.rs` | 16 个 `EffectSpec` 实例各补 `slots`：4 个色罩 kind **填真实槽位**，其余 12 个 `&[]` |
| `dhampir-timeline/src/layer.rs` | 2 处测试 helper 补 `slots: &[]` |

**共 3 个文件**。`EffectSpec` 初始化点全仓库 **22 处**（`grep -rn "EffectSpec {"`）：
`dhampir-core/src/effects.rs` 16（`const` 定义）+ 测试；
`dhampir-timeline/src/layer.rs` 2（测试 helper）；`dhampir-timeline/src/schema.rs` 1（测试常量）。

⚠️ **最终判据是编译器，不是数出来的数**：缺字段必然报 `E0063`，
实测 `cargo check --workspace --all-targets` 里该错误 **0 次** ⇒ 一处不漏。
（第一版用正则数"每个块里有没有 `slots`"，因正则把多行块截断而**误报 10 处** ——
所以这里记的是编译器给出的结论。）

### 10.2 为什么这一列是**数据**而不是"统一公式"

`SlotCombine` 五个变体各自对应**刻意的创作语义**，都注明了原实现行号：

| 变体 | 原实现 | 语义 |
|---|---|---|
| `MaxStrict` | `timeline.rs:547` | `flash`：严格大于者胜 |
| `MaxInclusive` | `:558` / `:570` | `vignette` / `noise`：含等号 |
| `Over` | `:595` | `overlay`：`1-(1-a)(1-b)` clamp |
| `Add` / `Mul` | `:364` 段 | 色调整：可交换 |

外加 `write_only_on_win: bool` 表达 `flash` 那条**颜色只在胜出时写**的语义
（弱的那条不许把颜色稀释成两者平均）。

### 10.3 判据（5 条，**全部带变异自证**）

| 测试 | 钉什么 |
|---|---|
| `槽位声明与色罩实现逐条一致` | 4 个 kind 的槽位名单与 `timeline.rs` 逐条对齐 |
| `色罩四个_kind_合计槽位正好覆盖结构体里那十八个` | 合计 **18** = `ColorMaskParams` 除 `width/height/frame` 外全部 |
| `不走定长槽位的_effect_必须声明空槽位` | 反向：模糊/色调整/Warp 不许声明 |
| `合成规则不许被统一成一种` | 4 种 combine 各不相同 |
| `闪白的颜色只在胜出时写` | `write_only_on_win` 的取值 |

**变异自证已实测**：删掉 `vignette_softness` 的声明 ⇒ 两条测试当场红并点名。

### 10.4 验证结果

| 项 | 结果 |
|---|---|
| `cargo check --workspace --all-targets` | **0 error / 0 warning** |
| `dhampir-core` 测试 | **264 → 269**（+5，全过） |
| `cargo test --workspace` | 除**一条既有的环境相关字体测试**外全过 |
| 那条字体测试 | ⚠️ **既有失败，与本次改动无关** —— `git stash` 后在**干净基线**上同样失败 |
| **本仓（YekiTrim）全部门禁** | 全绿（`221` / `17 条边界` / `97 项接线`） |
| **本仓演示工程产物** | **逐字节相同**（`ba9cc60e8dd42c85`）⇒ **schema 中性已实测验证** |

⚠️ **步 1 刻意不改任何行为**：渲染器仍在读老那份 `match`，`slots` 是"已声明、尚未被读"的数据。
所以"产物逐字节相同"是**预期的**，不是"没生效"。

### 10.5 下一步（步 3）的注意点

- 三条管线**逐条切**，每切一条跑一次 `scripts/yekitrim-compare.mjs --golden`，
  中位数必须仍是 **15.07 / 9.90 / 11.80**；
- 切 `color_adjust` 时注意它的 7 个槽位是**共享**的（由 `kind` 决定写哪个），
  与色罩的"每个 kind 一组独立槽位"**形状不同**；
- `flash` 的 `write_only_on_win` 与 `overlay` 的 `from_param`/`to_radians` 变换
  是两处**逐 kind 的特殊逻辑**，泛化时要保留，不能当"直接写数"。

---

## 11. ★ 步 3（色罩管线）执行记录（✅ 完成）

**手写 `match spec.kind` 已删除，改成读 `EffectSpec::slots`**（`render/timeline.rs`）：
75 行 match → 69 行登记表驱动 + 3 张映射表（`read_slot` / `write_slot` / `slot_default`）。

### 11.1 ★★ 泛化过程中**被既有测试抓出来的四个语义错误**

这一段值得完整记下来：它们是"看着对、跑起来错"的典型，**四个全都是既有测试抓到的**，
而如果没有那几条测试，它们会**静默产生错误画面**。

| # | 我写的 | 原实现 | 症状 | 抓到它的测试 |
|---|---|---|---|---|
| 1 | 参数没给就 `continue` | `unwrap_or(1.0)` 等**逐个缺省** | `flash` 的颜色变成**黑**（缺省应是**白**） | `闪白的颜色缺省是白` |
| 2 | `combine` 套到**每个**槽位 | 只有 `*_amount` 参与合成 | `overlay_angle` 被 `over(0,180)` 变成 **1**（应为 π） | `覆盖层的角度在这里就转成弧度` |
| 3 | 漏了 `.max(1e-3)` | `softness` 是**除数**，有下限 | `softness=0` 时着色器**除零** | `暗角的_softness_不会变成零` |
| 4 | 未胜出仍写 `amount` | `amount` 的赋值**也在 `if` 内部** | 弱的闪白把强的**覆盖回去**（0.8 被 0.3 盖掉） | 我新加的 `两条同_kind_…` |

⇒ **教训**：`over()` 是**不透明度**的合成函数，套到角度/枚举上没有意义；
"参数没给就用缺省"与"参数没给就跳过"是**两件事**（旧写法里每个 `unwrap_or` 都在表达前者）。

### 11.2 新加的等价性判据（6 条）

`p5_slot_tests`：`r2/g2/b2` 回落主色、显式给了就不回落、两条同 kind 只让更强的写、
顺序反过来也要对、`overlay` 两条按 over 合成（0.5+0.5→0.75 而不是 0.5）、四类逐值等价。

**合计：`dhampir-core` 264 → 275（+11）。**

### 11.3 验证

| 项 | 结果 |
|---|---|
| `cargo check --workspace --all-targets` | **0 error / 0 warning** |
| `dhampir-core` | **275 passed / 0 failed** |
| 全工作区 | 除那条**既有**环境字体测试外全过 |
| 本仓 15 条门禁 | 全绿 |
| 本仓演示工程产物 | **逐字节相同** `ba9cc60e8dd42c85` |

⚠️ 产物逐字节相同说明**工程文件层面零影响**；画面层面的等价性由那 11 条测试钉住。

### 11.4 还没做的

- `color_adjust`（7 个共享槽位，形状与色罩不同）与 `warp`（13 个）**仍是手写 match**；
- 三样本对拍（`yekitrim-compare.mjs --golden`，中位数 15.07 / 9.90 / 11.80）**尚未跑**
  —— 需要在底座产物（wasm/worker）可用时跑，是本步"真·验收"。

---

## 12. ★ 步 3b 执行记录（✅ 完成）—— 三处 match 全部消失

### 12.1 结果

| 管线 | 原形态 | 现形态 | 槽位数 |
|---|---|---|---|
| 色罩 `color_mask_params` | 75 行 `match`（4 分支） | 登记表驱动 | 18 |
| 色调整 `color_params` | 50 行 `match`（7 分支） | 登记表驱动 | 7 |
| Warp `warp_params` | 46 行 `match`（4 分支） | 登记表驱动 | 10 |
| **合计** | **171 行手写派发** | **0** | **35** |

**`grep -rn "match spec.kind"` 全仓只剩注释** —— 三条管线 15 个 kind 的字符串派发**全部消除**。

### 12.2 ★ 三条管线形状各不相同（这是不能"统一"的原因）

| | 色罩 | 色调整 | Warp |
|---|---|---|---|
| 槽位归属 | 每 kind **一组**（`flash_*`） | 7 kind **共用**一组、各写一个 | 每 kind 一组 |
| 竞争 | `MaxStrict`/`MaxInclusive`/`Over` | **无竞争**（`Add`/`Mul` 可交换） | `MaxInclusive` |
| 未胜出 | 一个槽位都不写 | 不适用 | 一个槽位都不写 |
| 缺省 | `flash` 白 / `r2` 回落主色 | 无 | 频率 12/3/1.5、偏移 0.08 |
| 变换 | `shape` 枚举、`angle` 弧度 | `hue`/`hue_css` 弧度 | `skew` 弧度 |
| 特殊 | `write_only_on_win` | — | **频率不乘 strength** |

⇒ 所以 `SlotCombine` 是**数据**、逐条登记；三张槽位映射表各写各的。

### 12.3 ★★ 泛化时被抓出来的语义错误（累计 **5 个**，全是既有/新增测试抓到）

色罩那 4 个见 §11.1；Warp 又抓到 1 个：

| # | 我写的 | 原实现 | 症状 |
|---|---|---|---|
| 5 | 频率也乘 `strength` | **只 `*_amount` 乘** | 窗口边缘频率突变（"像卡了一下"） |

**变异自证实测**：把频率也乘上 `strength` ⇒ `warp_频率不乘时间窗强度` 当场红
（`频率**不许**乘强度，实得 0.5`）。

### 12.4 里程碑实测（★ 目标达成）

把 `test_tint`（一个全新色罩特效，复用 `overlay_*` 槽位）加进 `effects.rs` 的 `REGISTRY`：

- **只改 `effects.rs` 一个文件**；
- 渲染主路径**一行没动**；
- `grep "没有在.*接上"`（那条 `debug_assert` 是"登记了却没接上"的哨兵）= **0 次**
  ⇒ 新特效**自动**走通了三条管线里的渲染路径；
- 唯一变红的是 `kinds()` 的**清单断言**（它本来就该红 —— 那是"有人加了 kind"的记录）。

⇒ **P5 的目标形态达成**：加一个落在既有管线内的特效 = 加一条 `EffectSpec` + 声明槽位。

### 12.5 验证

| 项 | 结果 |
|---|---|
| `cargo check --workspace --all-targets` | **0 error / 0 warning** |
| `dhampir-core` | **264 → 286（+22）** |
| 全工作区 | 除那条**既有**环境字体测试外全过 |
| 本仓 15 条门禁 | 全绿 |
| 本仓演示工程产物 | **逐字节相同** `ba9cc60e8dd42c85` |

### 12.6 ⚠️ 三样本对拍照**跑了但跑不通** —— 原因是环境，不是本次改动

**已尝试**（`--fixture demo --golden docs/golden/dhampir-parity`）：
整条回路走到最后一步 —— 转译 ✓、参照帧读到 ✓、`dhampir render` 起来了 ✓、
**第 1 帧写进编码器**时失败：

```
Error opening output …\.mine.mp4.video.mp4: Permission denied
第 1 帧写进编码器失败：管道正在被关闭。 (os error 232)
```

**已定位：与本次改动无关**。三条证据：

1. **换成未改动的 release 二进制**（`--dhampir …/target/release/dhampir.exe`，
   2026-10-12 构建，**早于**本次全部改动）—— **同样失败、同一行报错**；
2. **绕开 compare 脚本直接调 dhampir** —— 同样失败；
3. **换到工作区之外**（`%TEMP%`）—— 同样失败。

⇒ 是 dhampir 的**编码器 ffmpeg 子进程**在本机当前环境下开不了输出文件
（`pipeline.rs:1344` 那段：输出路径当 ffmpeg 参数传，由 ffmpeg 自己打开）。
**不是参数折叠层的问题** —— 那 22 条新单测已逐值钉住。

⚠️ 所以结论要**说清楚边界**：
- ✅ **参数折叠层**（`color_params` / `color_mask_params` / `warp_params`）
  **逐值等价**，有 22 条测试 + 变异自证；
- ❌ **端到端像素**等价**尚未验证** —— 不是"验过没问题"，是"这条回路在本机跑不通"。
  等环境可用时必须补跑，中位数须仍是 **15.07 / 9.90 / 11.80**。
