# 整合方案：底座文字动效（T1/T3/T4/T6）+ 本仓接线 + E4b + §14

> 2026-10-11 用户拍板：「整合一下然后全部开展，底座的也在这里进行了」
> 即：**底座（Dhampir）也放进这一轮的作业面**，不再只提需求。

## 作业面（两个仓）

| 仓 | 路径 | 本轮角色 |
|---|---|---|
| 本仓 YekiTrim | `F:\para\Code\YekiTrim` | 转译器接线、E4b 搬运、§14、总表 |
| **底座 Dhampir** | **`F:\para\Code\Dhampir`** | **T1/T3/T4/T6 的逐帧通道** |

⚠️ **两个仓各自独立提交**，互不代提。

## 关键结构约束（决定了任务怎么切）

1. **T1 / T3 / T4 都改 `crates/dhampir-timeline/src/layer.rs` 的同一个
   `SubtitleStyle`**，而且都要在 `dhampir-core/src/overlay.rs` 的**同一个逐帧循环**里
   取值 ⇒ **三者必须串行**（同一个文件、同一个函数，并行必冲突）。
2. **T6 改的是另外两个文件**（`text_overlay.rs::paint_one` +
   `text_raster.rs` 的 `force_style`）⇒ **可以与 T1/T3/T4 并行**。
3. 本仓侧（转译器 / E4b / §14）与底座**完全不相交** ⇒ 可全程并行。

```
        底座：T1+T3+T4（串行，同一文件）
        ├────────────────────────────────────┐
        底座：T6（并行，另外两个文件）        │
        └────────────────────────────────────┘
                         ↓ 底座出接口后
        本仓：转译器接线 T1/T3/T4/T6 + 守卫
        本仓：E4b 搬运（独立）
        本仓：§14 C 层（独立，等形态拍板）
```

## 底座四项的接口设计（已勘察过落点，不是凭空写）

### 落点（逐个已核实）

| 项 | 落点 | 现状 |
|---|---|---|
| T1/T3 缩放 | `layer.rs::text_envelope`（逐帧，当前返回 `(f32,f32)`）+ `text_layout.rs::place_line(rect, target, font_ratio)`（603 行） | `font_ratio` 是**轨道级常量** |
| T4 抖动 | 同 `text_envelope` 的返回值 | 只有一次性的 `rise_in_px` |
| T6 底衬 | `text_overlay.rs::paint_one`（729 行，画字之前）+ `text_raster.rs::force_style`（590 行，只拼 5 个键） | 无底衬通道 |

### T1 弹入（`0.7 → 1.15 → 1.0`，180ms，`back_out3`）

```rust
// layer.rs · SubtitleStyle
#[serde(default = "one_f32")]  pub scale_in_from: f32,   // 默认 1.0 = 不缩
#[serde(default = "one_f32")]  pub scale_in_peak: f32,   // 默认 1.0 = 不过冲
#[serde(default)]              pub scale_in_ms: u64,     // 默认 0 = 关
```

`text_envelope` 返回值从 `(opacity, dy_px)` 变成 `(opacity, dy_px, scale)`。
⚠️ **改返回值是破坏性改动**：`overlay.rs:315` 与 `:415` 两个调用点都要改，
且 `danmaku.rs:145` 那条路也要跟着。**要么给新函数、要么一次改全** ——
推荐**改全**（只有三处，且编译器会全部指出来）。

### T3 强调放大

**不新增字段**：复用 T1 的 `scale` 通道，只是触发时机在"词命中处"而不是入场。
本仓已经在传 `highlight_color`；放大由**本仓在 cue 上给一个 scale** 即可 ——
所以底座这一侧 T1 做完，T3 只要能**逐 cue 覆盖 scale** 就够：

```rust
// layer.rs · SubtitleCue（不是 Style）
#[serde(default, skip_serializing_if = "Option::is_none")]
pub scale: Option<f32>,   // 这一条的瞬时缩放倍率，None = 1.0
```

### T4 抖动

```rust
// layer.rs · SubtitleStyle
#[serde(default)]                      pub shake_px: f32,     // 默认 0 = 不抖
#[serde(default = "default_shake_hz")] pub shake_hz: f32,     // 默认 18.0
#[serde(default)]                      pub shake_phase: f32,  // 相位，同屏多条要错开
```

⚠️ **必须是纯 `t` 的函数、无累积状态**（否则 seek 与顺序播放分叉，
这是本仓 `handheld_sway` 已经立过的规矩）。建议复用
`tools/polish-to-dhampir.mjs` 里 `shakeOffsetAt(t, level)` 的**同一套相位表**。

### T6 底衬

```rust
// layer.rs · SubtitleStyle
#[serde(default, skip_serializing_if = "Option::is_none")]
pub plate_color: Option<[u8; 4]>,
#[serde(default = "default_plate_pad")]
pub plate_pad_px: f32,   // 默认 12.0
```

⚠️ **不要走 libass 的 `BorderStyle=3`** —— `force_style` 只拼 5 个键，
传不下去；且底座**有意**把填充与描边都画成 `WHITE_INK`（覆盖度要靠"全是白墨"
才恢复得出来），底衬要的是**另一种墨**，与那条设计正面冲突。
⇒ **在 `paint_one` 里、画字之前铺一块矩形**（纯几何，不动 libass）。

## 验收（四项各自要有"数像素"的判据）

⚠️ 这一整类错误在本仓栽过三次（`bottom_margin` 算成 0 可用宽、§11 的逗号转义、
§9 的 EDL 覆盖），全是"**跑得通、日志正常、结果不对**" ⇒ **不能只看"没报错"**：

| 项 | 判据 |
|---|---|
| T1 | 入场 180ms 内文字外接框宽度**先缩后过冲再回落**（逐帧量） |
| T3 | 命中词那几帧的字高**变大**（与相邻帧比） |
| T4 | 相邻两帧文字质心位移**非零且随时间变号**（不是单向漂移） |
| T6 | 文字外接框**外扩一圈**的区域出现非背景色，且**色值等于 `plate_color`** |
| 通用 | 同一帧 `seek` 过去与顺序播放逐像素相同（规格 §4.1 硬约束） |

## 决策纪律（沿用本仓一贯口径）

- 文档没覆盖处 → **取最保守方案** + 记进 `docs/plan/polish-upgrade-conservative-log.md`（本仓侧编号 #153 起）。
- **零回归优先**：新字段一律 `#[serde(default)]`，默认值是"行为与今天逐字节相同"。
  ⚠️ 新增配置段**必须带 `deny_unknown_fields`**（本仓这一趟刚栽过两次，见 #147–#151）。
- 新增判据**必须用变异自证**，且必须覆盖**调用方那条路**（不是只测函数）。
