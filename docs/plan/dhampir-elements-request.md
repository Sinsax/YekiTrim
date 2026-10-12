# 底座需求：文字元素动效的逐帧通道（T1 / T3 / T4 / T6）

> 提出方：YekiTrim（本仓）｜ 日期：2026-10-11 ｜ 对应：§5 E4a 的四个动效
> 结论依据：保守决策 #125–#128、#134–#138（`polish-upgrade-conservative-log.md`）

## 为什么提这份需求

规格 `docs/plan/elements-spec.md` §3 的 2026-10-10 更正写着"T1/T2/T3/T4/T6 全部本仓自足"。
**那句话的本意是"不必等 `entrance`/`highlight_color` 两个字段"**，
不等于"底座已经有逐帧能力"。拿底座的真实渲染路径逐项对过之后，只有 **T2** 真的自足。

本仓已经把 T2 做完（`render/elements.rs` + 转译器拆 cue）。
剩下四项**都需要底座加通道**，逐项列在下面 —— 每项都带**数值**与**接口形状**，
以便直接开工，不必再回来问。

## 先说清楚：底座的文字渲染现状

- 文字走 `subtitles=<ass>:fontsdir=<dir>:force_style=<keys>`；
- `force_style` **只拼五个键**（`text_raster.rs` 全文只有这些）：
  `FontName` / `FontSize` / `PrimaryColour` / `Outline` / `OutlineColour`；
- ⚠️ **填充与描边都画成 `WHITE_INK`（`&H00FFFFFF`）**，颜色在 ffmpeg 之后那一步才合成
  —— 因为**覆盖度要靠"全是白墨"才恢复得出来**；
- 字幕轨的**全部**逐帧手段是 `text_envelope`：
  `fade_in_ms` / `fade_out_ms` / `rise_in_px`（入场位移，**一次性**）；
- `SubtitleStyle` **没有** `font_ratio` 关键帧；
- `text_overlay.rs::paint_one` **全程不读 `layer.transform`**（字幕那条路上它被完全忽略）。

⇒ 结论：**逐帧的缩放、持续位移、底衬，三个通道都不存在。**

---

## ✅ 已交付（2026-10-12）：文字轨的**纵向锚**（`T7`）

> **状态：底座侧已实现并验证。** 本仓（YekiTrim）已接通，见下面的「交付回执」。
> 需求原文（现象、根因、建议形状）保留在下一节，供追溯。

### 交付回执

| 项 | 结果 |
|---|---|
| 底座版本 | `dhampir-timeline` **`HOST_API_VERSION` 6 → 7**（返回体加键，按本仓口径必须升） |
| 字段 | `SubtitleStyle::anchor_bottom: f32`，缺省 `1.0`（= 贴底 = 老行为） |
| 解耦 | `safe_width_ratio > 0` 时**左右只认它**；`anchor_bottom < 1.0` 时接管纵向 |
| 双保险 | 契约层 `skip_serializing_if = "is_one_f32"` + 返回体层 `is_one_f32_view` ⇒ **老工程一个键都不多** |
| 单测 | 新增 4 条（抬升且不动折行 / 缺省走老式子 / 不压空 / 三档单调）；**变异自证**：拿掉接管条件后 2 条当场红 |
| 回归 | `cargo test --workspace` **796 passed / 1 failed**（那条是改动前就红的环境用例） |
| 本仓接通 | 转译器 `pos` → `anchor_bottom`，**按 (样式 × 档位) 两维分轨** |
| ★ 真机证据 | 成片三档文字纵向重心 **0.394 / 0.663 / 0.903**（严格单调，跨度 549px） |

### ⚠️ 换产物这一步：**顺序反了会让预览整个不可用**

契约号从 6 升到 7 之后，**旧前端对着新底座、或新前端对着旧底座，都会当场报
「底座 wasm API 版本不匹配：期望 N，实得 M」**（并退回"近似预览"）—— 这正是那个哨兵的意义。

所以本机换产物的顺序是：

1. **先**把底座产物整个目录放到 `<程序目录>/dhampir/`（本机开发形态 = `target/debug/dhampir/`）；
2. **再**改本仓的 `DHAMPIR_HOST_API`（`webui/src/components/PolishPreview.vue`）。

本机实际做的（2026-10-12）：

* 底座 `dist/dhampir-0.4.0+901bb6a/` → `target/debug/dhampir/`（旧的备份在 `target/dhampir-old-0.1.0-api6/`）；
* 本仓 `DHAMPIR_HOST_API` 6 → 7，重建 `webui/dist`，重启 `yekitrim server`。

⚠️ **换完必须重启 server**：那份产物是**启动时**解析并交给前端的，不重启就还是旧的
（症状与"改完没反应"一模一样）。

★ 实测（`target/debug/dhampir/` 换过之后、浏览器开了 WebGPU）：

| 检查 | 结果 |
|---|---|
| 页面上的「底座 wasm API 版本不匹配」 | **没有了** |
| 「（当前是近似预览）」错误条 | **没有了** |
| `<canvas id="dhampir-canvas">` | **1102×620**（换之前是 **0×0**） |
| 画布底板 | **1920×1080** |
| 预览状态行 | 「dhampir 预览 · 0/30fps · 221ms · 丢 0」 |
| `yekitrim polish doctor` | 「✓ 底座可用（出片只有它一条腿）」，`host_api=7` |

⚠️ **无头浏览器默认没有 GPU 适配器** —— 不额外开 WebGPU 时会看到
「没有可用的 GPU 适配器：NotFound」而**误以为还是版本问题**。
那句和版本无关，是 headless 的环境限制（加 `--enable-unsafe-webgpu` 才正常）。

⚠️ **实施时发现的一处需求外的事**：`anchor_bottom` 是**轨级**常量，而 `pos` 是**逐事件**的
—— 只按样式分轨会让同一样式的两个档位**共用一个常量**（后写覆盖先写）。
实测真实工程里 `hook` 同时出现 `top` 与 `center`，所以本仓改成两维分轨。
底座侧不需要为此改动：**一条轨一个锚本来就是正确的粒度**。

---

## ⚠️ 追加需求原文（2026-10-12）：文字轨的**纵向锚**（`T7`，本次实施时发现）

> 这一条不在原计划里 —— 是**做 §5 E3 的「元素落点」时顺手核出来的**。
> 它与上面三项**性质不同**：上面三项是"动效不够好"，这一项是**"用户选的东西没生效"**。

### 现象

本仓的 `elements.text` 有一个 `pos = "top" | "center" | "bottom"` 档位（`elements-spec.md` §2），
`process` 也**真的按它选落点**（`placement.rs::TextAnchor::parse` → 在对应带里挑不重叠的点 →
写回 `x`/`y` 百分比）。**预览按它摆、成片不认** ⇒ 两块分叉：

* 预览：`top` 的字在画面上方、`center` 的在中部；
* 成片：**三条全贴底**，`pos` 完全无效。

### 为什么本仓接不通（两条路都堵死）

1. **`bottom_margin` 不能拿来做纵向位置**。底座 `text_layout::layout`：

   ```rust
   let margin = style.bottom_margin.clamp(0.0, 0.5);
   let max_width_px = sequence_width * (1.0 - 2.0 * margin);   // ← 左右安全宽
   let block_top = 1.0 - margin - (keep as f32) * line_height_norm;  // ← 纵向位置
   ```

   **同一个字段身兼两职**：左右安全宽要求它 **0.0625**（`(1-0.875)/2`，转译器现在就是这么填的），
   而把 `top` 档抬到画面上方需要它 ≈ **0.6**。抬上去之后
   `max_width_px = W × (1 − 1.2) < 0` ⇒ **一个字都排不出来**。
   ⚠️ 这**不是理论风险**：转译器那段注释记着一次真实事故 ——
   `bottom_margin` 误填 `0.5` 时"每一条文字元素都静默消失"，报告里**一行差异都没有**。

2. **ASS 对齐码传不到底座**。`.ass` 里写 `{\an8}` 会被底座 `parse_ass`
   **当覆盖标签丢掉**（单测名就叫「解析_ass_跳过样式段并去掉覆盖标签」）；
   而 Style 行的 `Alignment` / `MarginV` 也不参与底座自己的 `text_layout::layout`
   （那条路只读 `SubtitleStyle`）。且 `force_style` **只拼五个键**，塞不进对齐。

### 建议接口

给 `SubtitleStyle` 加一个**与 `bottom_margin` 解耦**的纵向锚，最小形状：

```rust
pub struct SubtitleStyle {
    // ... 既有字段 ...
    /// 文字块**底边的归一化 y**（0 = 画面顶，1 = 画面底）。默认 1.0 = 现行为。
    ///
    /// ⚠️ 与 `bottom_margin` **必须分开**：那个字段今天是"左右安全宽"的输入
    ///    （见 `text_layout::layout` 那句 `1.0 - 2.0 * margin`），
    ///    再拿它当纵向位置会让宽度塌成 0（本仓踩过，文字静默全没）。
    #[serde(default = "one_f32")]
    pub anchor_bottom: f32,
}
```

**底座的 `block_top` 改为**：`anchor_bottom - (keep as f32) * line_height_norm`
（`bottom_margin` 回归它"只管左右"的本职）。

**本仓怎么用**：转译器把 `[elements.text].pos` 映射成
`anchor_bottom`（`top ≈ 0.30` / `center ≈ 0.62` / `bottom ≈ 1.0`，
数值应取 `placement.rs::TextAnchor` 那条带的中线，**由底座定口径后本仓跟随**）。

**验收**：同一条文字元素，`pos = top` / `center` / `bottom` 三份工程出片后，
黄色像素的**纵向重心**必须明显不同（单调：top < center < bottom），
且三者的**左右安全宽一致**（`bottom_margin` 不动 ⇒ 折行与字号都不该变）。

⚠️ **在底座给出这个字段之前**，本仓的选择只有：
① 维持现状（预览按档位、成片贴底，**分叉继续存在**）；
② 把预览也改成"一律贴底"（分叉消失，但等于**承认四档是假的**）。
本仓**暂取 ①**，并在设置界面把这个落差的**真实情况**标出来（不假装它生效了）。

---

## T1 弹入（pop-in）

**要什么**：字带弹性放大出现 —— `0.7 → 1.15 → 1.0`。

**为什么现在做不到**：`font_ratio` 是**轨道级常量**，没有逐帧版本。

**建议接口**（最小改动，且与既有 `text_envelope` 同源）：

```rust
pub struct SubtitleStyle {
    // ... 既有字段 ...
    /// 入场缩放的**起点**（1.0 = 不缩）。参照的弹入是 0.7。
    #[serde(default = "one_f32")]
    pub scale_in_from: f32,
    /// 入场缩放的**过冲峰值**（1.0 = 不过冲）。参照是 1.15。
    #[serde(default = "one_f32")]
    pub scale_in_peak: f32,
    /// 过冲持续多久（毫秒）。0 = 直接线性到 1.0。
    #[serde(default)]
    pub scale_in_ms: u64,
}
```

求值放在 `text_envelope` 里（它已经是逐帧的）：返回 `(opacity, dy_px, scale)`，
其中 `scale` 走 `back_out3`（底座的曲线函数，本仓的曲线表与它同源）。
⚠️ `place_line(rect, target, font_ratio)` 要收**乘过 scale 的** `font_ratio`。

**参考数值**：起点 `0.7`、峰值 `1.15`、时长 `180ms`、曲线 `back_out3`。

**T3 强调放大**用的是同一对字段（只是时机在词命中处而不是入场），
所以这一项做完，**T3 的"放大"那半截也一并解决**（换色那半截本仓已有）。

## T4 文字抖动

**要什么**：字跟画面一起抖 —— **持续的**逐帧纵向（其实要两轴）偏移。

**为什么现在做不到**：`rise_in_px` 是**一次性入场位移**
（`text_envelope` 里从 `rise_in_px` 线性收到 0），不是持续抖动。

**建议接口**：

```rust
pub struct SubtitleStyle {
    // ... 既有字段 ...
    /// 逐帧抖动幅度（文档像素）。0 = 不抖。
    #[serde(default)]
    pub shake_px: f32,
    /// 抖动频率（Hz）。参照的画面抖动是 18Hz。
    #[serde(default = "default_shake_hz")]
    pub shake_hz: f32,
    /// 相位（弧度）—— 让同一屏里的多条字**不同相**，否则会整块平移。
    #[serde(default)]
    pub shake_phase: f32,
}
```

**⚠️ 关键约束：必须是"纯 `t` 的函数，无累积状态"**
（本仓 `handheld_sway` 的注释就是这么写的）。
否则 `seek` 到第 100 帧与"播到第 100 帧"长得不一样 ⇒ 预览与导出分叉。

本仓这边已经有 `shakeOffsetAt(t, level)` 的四级阶梯表
（`tools/polish-to-dhampir.mjs`，348 项曲线判据钉着），
**建议直接复用同一套相位表**，免得画面抖与文字抖看着不同步。

## T6 字幕底衬（plate）

**要什么**：字幕背后一块半透明色块。

**为什么现在做不到**（这条最容易误判）：
1. `force_style` **没有 `BorderStyle` / `BackColour`** ⇒
   在 `.ass` 里写 `BorderStyle=3` **会被盖掉**（本仓第一版就是这么误判的）；
2. 即使把 `BorderStyle=3` 传下去，libass 的底框画的是**同一种墨**，
   而底座**有意**把填充与描边都画成 `WHITE_INK`（覆盖度要靠"全是白墨"才恢复得出来）
   ⇒ **底衬要的是另一种颜色**，与那条设计**正面冲突**。

**建议接口**（两条路，推荐 ①）：

① **在 `text_overlay.rs::paint_one` 里、画字之前先铺一块** ——
   本仓已经把文字矩形算出来了（`place_line` 的返回值），
   铺块矩形是纯几何、**不需要动 libass**，也不碰"白墨"那条设计：

```rust
pub struct SubtitleStyle {
    // ... 既有字段 ...
    /// 底衬颜色（RGBA）。`None` = 不铺。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plate_color: Option<[u8; 4]>,
    /// 底衬比文字矩形**外扩**多少（文档像素）。
    #[serde(default = "default_plate_pad")]
    pub plate_pad_px: f32,
}
```

② 或者走 libass 的 `BorderStyle=3` + `BackColour`，
   但**得先把"白墨"那条约束解开**（覆盖度怎么恢复），代价大得多，不推荐。

**参考数值**：`plate_color = [0, 0, 0, 140]`（黑 55%）、`plate_pad_px = 12`。

## 本仓自己做的部分（不必底座操心）

- **T2 打字机**：已做完 —— 拆 cue（不是 `\k`：底座会把 `{...}` 覆盖标签整块剥掉）。
- **T3 的"换色"半截**：`highlight_color` 已在用。
- 四项做完之后，本仓只需在 `tools/polish-to-dhampir.mjs` 的
  `textTracks` 那段**多写几个字段**，其余链路（`process` 排版、四档样式、守卫）都已就位。

## 验收怎么对

四项都属于"**跑得通、日志正常、结果不对**"那一类
（本仓已经栽过三次：`bottom_margin` 算成 0 可用宽、§11 的逗号转义、§9 的 EDL 覆盖）。
所以**每一项目标侧都要有一条"数像素"的判据**，不能只看"没报错"：

| 项 | 判据 |
|---|---|
| T1 | 入场那 180ms 内，文字外接框宽度**先缩后过冲再回落**（逐帧量） |
| T3 | 命中词那几帧的字高**变大**（与相邻帧比） |
| T4 | 相邻两帧文字质心的**位移非零且随时间变号**（不是单向漂移） |
| T6 | 文字外接框**外扩一圈**的区域里出现非背景色，且**色值等于 `plate_color`** |

并且四项都要求：**同一帧 `seek` 过去与顺序播放出来逐像素相同**
（规格 §4.1 那条硬约束的机器判据）。
