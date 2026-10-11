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
