# 底座能力请求：逐字「角度错峰」（`rotation_spread`）

> 2026-10-14 · YekiTrim → Dhampir 底座
> 状态：**待底座实施**。本仓**不动** `DHAMPIR_HOST_API`、**不自造**第二套字段。
> 同类先例：`docs/plan/dhampir-heartbeat-request.md`（心跳双峰）。

## 一、要什么（一句话）

`PerCharSpec` 加一个**乘 `rank` 的角度项**，让"第 i 个字转 i 度"这件事可表达。

```
rotation_deg = mix(rotation_from, rotation_to) + rotation_spread × rank
                                 ↑ 现在到这里就没了
```

`rank` 用**现成的那个**（`reverse ? count-1-index : index`，与 `spread_x_em` /
`scale_spread` 共用 —— 各算一份会让 `reverse` 下方向不一致，那份文档自己写了这条）。

## 二、为什么（**实测**，不是"这样更好看"）

### 症状：`arc` 档跟 `tilt` 档**逐字同形**

用户口径：「**排版和逐字变化没有在预览上正常显示**」。

实测 `arc` 档在**稳态**（`t = 3.0`，入场早已结束）每个字的角度：

| 字 | 一 | 段 | 弧 | 线 | 测 | 试 | 文 | 字 |
|---|---|---|---|---|---|---|---|---|
| **现状 `arc`** | -14° | -14° | -14° | -14° | -14° | -14° | -14° | -14° |
| **真弧该是** | -14° | -10° | -6° | -2° | +2° | +6° | +10° | +14° |

⇒ 现状**每一字都是同一个角度** ⇒ 那是"整行统一倾斜 14°"，
**与 `tilt` 档（signed 单角度）在观感上分不出来**。

⚠ 注意这不是"预览漏了"：底座 `text_perchar_anim.rs:345` 是
`rotation_deg: mix(spec.rotation_from, spec.rotation_to)` —— **同样没有 `rank` 项**。
**两边一致地做不出弧**，所以这是一条**能力缺口**，不是漂移。

### 为什么现有原语凑不出来

现有能做形状的只有位移那一对：

```
offset_x_em = mix(spread_x_from, spread_x_to) × spread_x_em × rank
offset_y_em = mix(spread_y_from, spread_y_to) × spread_y_em × rank
```

它是 **`from → to` 的线性混合 × rank** ⇒ 逐字位移**只能单调**（一条斜线 / 一个扇形）。
而弧要的是"**两端下、中间上**"（抛物线）—— 单调函数做不到。

⇒ 用位移硬凑只能得到**斜排**，不是弧；要有弧必须让**角度**随序号变化。

## 三、接口（两个写法，推荐 A）

### A. 加一个标量（**推荐**）

```rust
/// 按序号错峰的角度增量（度）。第 i 个字在 `mix(from,to)` 之上再 `+ spread × rank`。
/// `0.0` = 不做（**与现有行为逐值相同 ⇒ 存量工程零回归**）。
#[serde(default)]
pub rotation_spread: f32,
```

`arc` 档据此可以写成：

```toml
rotation_from = -14.0
rotation_to   = -14.0
rotation_spread = 4.0        # 8 个字 → -14..+14
per_char_frames = 8
stagger_frames  = 3
```

⚠️ **推荐 A 的理由**：字段数 +1、语义与 `scale_spread` **完全同形**
（同一份文档已经论证过"为什么是乘法/为什么共用 rank"），实现是**一行加法**。

### B. 加一对端点（不推荐）

`rotation_spread_from/to` + `× rank`。表达力与 A 等价（`spread × rank` 里那个
`spread` 是常数），但多一个字段、且会让人以为"角度也能两端各自缓动"
（实际不会，缓动只作用在 `easing(t)` 那一层）。

## 四、验收判据（7 条）

1. **零回归**：`rotation_spread` 缺省 `0.0` ⇒ 逐帧、逐字、逐字段与今天**完全一致**。
2. **单调性**：`reverse = true` 时，第 i 个字的角度**恰好镜像**（判据要真验第三条）。
3. **`count <= 1`**：`rank` 项为 0（一个字没有"第几个"可言）—— 与
   `spread_x_em` / `scale_spread` 同一纪律。
4. **稳态**：`per_char_frames` 走完后，各字角度**保持彼此不同**（不退化成同一角度）。
   ⚠️ 这条是本次请求的**核心**：判据必须**在稳态采样**，不能在入场窗口里采
   （入场时各字本来就在不同进度上，会**误判成"已经有弧了"**）。
5. **与 `rotation_from/to` 可叠加**：两者同时配时是**相加**，不是互相覆盖。
6. **`arc` 档的实测**：8 个字、`spread = 4`、`from = to = -14` ⇒
   稳态各字 **-14 / -10 / -6 / -2 / +2 / +6 / +10 / +14**。
7. **`tilt` 与 `arc` 必须可分**：像素判据下两档不能是同图
   （参考 `check-39-tiers-pixels.mjs` ③ —— 心跳那档刚因为"峰值逐位相同"被它抓过一次）。

## 五、本仓侧**不做**什么（等底座落地）

- **不改** `ANIM_PRESETS.arc` 的**语义**（现在它是个"入场扫过"，稳态退化成倾斜）；
- **不动** `DHAMPIR_HOST_API_VERSION`（照 `dhampir-heartbeat-request.md` 的纪律）；
- **不**在预览里自造一套只有预览认的 `rotationSpread`
  —— 那正是"预览里有、成片里没有"的标准配方。

> ⚠️ 在底座落地之前，`arc` 档的**如实描述**是：
> 「**入场时逐字扫过**，稳态是一行统一倾斜」。
> 这句话要写进用户文档，别写成"弧形排版"。
