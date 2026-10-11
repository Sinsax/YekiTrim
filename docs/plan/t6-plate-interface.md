# T6 底衬（plate）底座侧接口说明 —— 交付版

> 面向 **task-17（本仓接线）**。这里是底座**实际交付**的接口与口径，
> 与 `dhampir-elements-request.md` 的"需求"一节**不完全一样** ——
> 差异集中在一处（`plate_pad_px` 的形状），下面用 ⚠️ 标出来。
>
> 底座仓：`F:\para\Code\Dhampir`。提交：`ee8424b` → `4f7534b` → `74800d4`
> → `47fbd8a` → `103a6ef` → `cf4c500`。

## 1. JSON 字段（写在哪、叫什么）

字段加在**字幕轨道的 `subtitle` 段**里（`SubtitleStyle`），和
`font_ratio` / `color` / `outline` 那一批同一个位置：

```jsonc
{
  "tracks": [{
    "id": "sub",
    "kind": "subtitle",
    "layers": [ /* … */ ],
    "subtitle": {
      "font_ratio": 0.055,
      "color": [255, 255, 255, 255],
      "outline": false,

      // ↓↓↓ T6 新加的两个键 ↓↓↓
      "plate_color": [0, 0, 0, 140],
      "plate_pad_px": 12.0
    }
  }]
}
```

| 键 | 类型 | 默认值 | 含义 |
|---|---|---|---|
| `plate_color` | `[u8;4]` \| `null` | **`null`（不铺底衬）** | 底衬颜色，**RGBA**，alpha 生效 |
| `plate_pad_px` | `number` \| `null` | **`null` ⇒ 按 12.0 算** | 底衬比文字**行盒**四周外扩多少，单位**目标像素** |

⚠️ **两个键都是"不写 ⇔ 与今天逐字节相同"**（`skip_serializing_if`）。
`plate_color = null` 时底座**一个像素都不多画**，且 `plate_pad_px`
**读都不会被读到** —— 所以只写 `plate_pad_px` 是**没有任何效果**的。

## 2. ⚠️ 与需求文档的一处差异：`plate_pad_px` 是 `Option<f32>`

需求文档（`dhampir-elements-request.md`）里写的是：

```rust
#[serde(default = "default_plate_pad")]
pub plate_pad_px: f32,       // 裸 f32 默认 12.0
```

**实际交付改成了 `Option<f32>`**：

```rust
#[serde(default, skip_serializing_if = "Option::is_none")]
pub plate_pad_px: Option<f32>,   // None ⇒ 12.0
```

**为什么**：裸 `f32` + `skip_serializing_if = "等于默认值"` 会让
**用户显式写的 `12.0` 被静默吃掉** —— 读回来仍是 12.0、渲染逐像素一样，
所以**永远不会有症状**，但显式意图没了。本仓的口径是"显式写了却读不回来"
属于最恨的那类静默。用 `Option` 才分得开"没写"与"写了默认值"。

**对 task-17 的影响：没有。** JSON 层面两种形状**完全一样**
（`12.0` / 不写 / `null` 都照常工作）；只是**不要**把它当成"裸 number 字段"
去写一个 `skip` 守卫。

## 3. 口径（接线时必须一致的三条）

### 3.1 `plate_pad_px` 是**目标像素**，不是文档像素

⚠️ 与 `rise_in_px` / `transform.x/y` 那套**文档像素**不是一个坐标系。

参照值 `12` 是"离字多远"的观感量（浏览器 `padding: 12px`）。
本仓接线时**直接照抄 12 即可** —— 底座会在**已经栅格化好的目标帧**上铺块，
所以 640×360 预览与 1920×1080 成片里，底衬外扩的都是 12 个**成片像素**。

> 若哪天要做"预览与成片落同一比例"，得先改底座这一侧的口径
> （把 `pad` 改成比例）。**现在不做** —— 本条是如实记录，不是待办。

### 3.2 底衬围的是**行盒**，不是字形外框

底座拿的是布局算出来的**行盒**（`NormalizedRect`：`x/y/width/height`），
四边各 `round()` 到整数像素后再加 `pad`。所以底衬是**矩形**，
不是"贴着字形轮廓"的形状。

接线侧如果要"更贴字"的效果，那属于**另做一个特性**，不在 T6 里。

### 3.3 底衬跟着这一行的不透明度淡入淡出

`plate_color[3]` 会**乘上**这一帧的 `opacity`。所以**不要在 `plate_color`
的 alpha 里再乘一次淡入系数** —— 那会淡两次（`0.5 × 0.5 = 0.25`）。

## 4. 参照数值

```jsonc
"plate_color": [0, 0, 0, 140],   // 黑 55%（参照实现的口径）
"plate_pad_px": 12.0
```

## 5. 已经验到的 / 没验到的（如实）

**底座侧验证过的**（`cargo test`，数字都在提交说明里）：

| 判据 | 状态 |
|---|---|
| `plate_color = null` ⇒ 一个像素不多画 | ✅ 数像素，且**变异自证**（改成"默认铺黑"⇒ 报"多画了 8256 个像素"） |
| 有值时外扩一圈出现 `plate_color` | ✅ 数像素 + 矩形面积对得上 |
| **层序**（铺在字之前，字没被盖） | ✅ 单测 + **变异自证**（铺到字之后 ⇒ 红） |
| `plate_pad_px` 真的改变外扩量 | ✅ **变异自证**（写死成 0 ⇒ 红） |
| `plate_color = null` ⇒ **JSON 逐字节相同、一个键都不多** | ✅ 变异自证 ×2 |
| 端到端（真 ffmpeg + GPU） | ⚠️ **本机跑不了** —— 沙箱写不了 `%TEMP%`，用例**如实跳过**（仓库既有的 `text_shadow` 端到端用例同因全红） |

⚠️ **端到端那一半没验过**，接线时请把它当成"还没被真机确认过"。
单测用的是假栅格化器；真链路上还隔着 `evaluate_overlay` 的透传与
`paint_lines` 的调用点 —— 那两处都写了、编译过、单测覆盖了调用方，
但**没有真起过 ffmpeg 出图**。

## 6. 接线要做的事（task-17）

本仓侧只需在 `tools/polish-to-dhampir.mjs` 的 `textTracks` 那段**多写两个字段**，
其余链路（`process` 排版、四档样式、守卫）都已就位。接线时注意：

1. 字段名就是 `plate_color` / `plate_pad_px`，位置在 `subtitle` 段里；
2. **不写**（默认）与 `null` 等价，都能让老工程逐字节不变；
3. `plate_pad_px` 写 number（底座侧收 `Option`，但 JSON 上就是个 number）；
4. 底衬的淡入淡出**不用本仓管**（跟着 `opacity` 走）。
