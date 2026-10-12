# 方案 D 执行档：读 `transform.rotation` 能解锁什么

> 2026-10-12。**这是"方案 D"的施工前勘察结论 + 能力清单。**
> 起因：*"那就按照 d 进行，将能实现的样式和动态效果列出"*

---

## 〇、勘察结论：比预想的更好

**旋转的 GPU 实现已经存在、已经测试、两端同源** —— 文字只是**没走这条路**。

| 事实 | 证据 |
|---|---|
| `TransformV2.rotation` 字段存在 | `layer.rs:386`（`Layer.transform`） |
| `compose` 层**已经把 rotation 做成可动画的** | `compose.rs:350` `rotation_deg: animated(element.transform.rotation, "rotation")` |
| GPU 逆仿射**已实现**且注释详尽 | `render/compose.rs:281` `inverse_affine(...)`，含正/逆变换定义 |
| 已有单测覆盖 | `compose.rs:1141` / `:1196`（`rotation_deg == 30.0`）/ `render/compose.rs:691`（90°） |
| **文字路径完全不读 transform** | `text_overlay.rs` grep `transform\|rotation` = **0 命中** |

⇒ **不是"要发明一个新能力"，是"把已有能力接到文字上"。**

---

## 一、为什么文字漏了这条路（架构原因）

worker 出片每帧做**两件独立的事**：

```
① compose::evaluate_v2_with_assets(...)  → GPU 合成图层（走 transform / rotation / effects）
      ↓ readback 成 RGBA8 内存图
② painter.paint(&mut image, &overlay, ...) → CPU 把文字叠上去（不走 transform）
```

- ① 那侧 `inverse_affine` 已经把 `rotation_deg` 用上了
- ② 那侧 `TextOverlay { items, danmaku }` 里**根本没有 transform 字段**

⇒ **文字是在 GPU 合成之后、用 CPU 叠上去的**，所以图层的变换对它天然无效。
这不是 bug，是两条路当初的接缝。

---

## 二、⚠️ 一个必须先定的设计问题

`rotation` 在**两条不同的语义**上存在，接的时候要选一个：

| 接法 | 语义 | 影响面 | 风险 |
|---|---|---|---|
| **D1 · 轨级** | `TrackV2.subtitle` 旁边加一个 rotation | 整条字幕轨一起歪 | 小。但"倾斜 3 档"是**每档不同角度**，轨级做不到 |
| **D2 · 条级** | 每条文字（`TextItem`）带自己的 rotation | 每条字幕/每个文字元素各自的角度 | **推荐**。与 `dx_px`/`dy_px`/`color`/`font_px` 同款（都是"跟着条目走"） |

⚠️ 注意 `TextOverlay` 的注释已经立过规矩：

> ⚠️ **字幕与弹幕都没有帧级的"画法"字段了** —— 两者的轨道级样式都跟着**每一条**走。
> 理由是同一条真 bug 的**两处实例**：这里曾经有两个**共享槽**，循环里每轨整份重写、只留最后一份
> ⇒ **最后一条轨说了算**（元素轨 + 字幕轨并存时，前面那条轨的样式全失效）。

⇒ **必须走 D2（条级）**，否则会重蹈"共享槽"的覆辙。

---

## 三、D 能实现的样式（清单）

### ✅ 能实现

| 稿中档位 | 靠什么 | 说明 |
|---|---|---|
| **倾斜 · 标准** | 整块 rotate -6° | 每条的 rotation |
| **倾斜 · 小角度** | 整块 rotate -3° | 同上 |
| **倾斜 · 反方向** | 整块 rotate +5° | 同上（正负都行） |
| **砸入 · 带压扁** ⚠️ 部分 | rotate -6° **能**，`scaleY(1.18)` 压扁**不能** | `TransformV2.scale` 是**等比**，非等比要另加字段 |

### ❌ 不能实现（D 的边界）

| 档位 | 缺什么 |
|---|---|
| 弧形（3 档） | 逐字旋转 |
| 大小字（3 档） | 逐字字号 |
| 逐字抖散（3 档） | 逐字位移 + 旋转 |
| 竖排（3 档） | 排版方向 |
| 逐字变小 / 放大（6 档） | 逐字字号 |

---

## 四、D 能实现的动态效果（清单）

### ✅ 能实现

| 稿中档位 | 靠什么 | 说明 |
|---|---|---|
| **摇摆 · 标准** | rotation ±3.5° 往复 | 底座**已有** `animated(..., "rotation")` 通道 |
| **摇摆 · 大角度** | rotation ±9° | 同上 |
| **摇摆 · 慢摆** | rotation ±5°，周期拉长 | 同上 |

⚠️ **摇摆原本被列为"底座做不到"**（见 `text-styles-emotion-base-mapping.md`），
**本次勘察推翻了这个结论** —— rotation 通道早就有了，接上就能用。

### ✅ 摇摆的"往复"从哪来（已查证：能配）

`animated(element.transform.rotation, "rotation")` 走的是**关键帧动画**，所以要两个关键帧。
**已查证这条路是通的**：

| 环节 | 事实 | 位置 |
|---|---|---|
| `rotation` 是**一等公民**动画目标 | `TRANSFORM_TARGETS = ["opacity","x","y","scale","rotation"]` | `schema.rs:308` |
| 关键帧挂在 `Layer` 上 | `Keyframe { frame, target, value, easing }` | `schema.rs:270` |
| **字幕轨也有 `layers`** | `TrackV2 { kind, layers: Vec<Layer>, subtitle: Option<SubtitleStyle>, .. }` | `layer.rs:1100-1107` |
| 每个 `Layer` 都带 `transform` | `pub transform: TransformV2` | `layer.rs:386` |

⇒ **"摇摆"（rotation 往复）今天就能配出来**，不用等编排层。
⇒ 「摇摆 3 档」从"做不到"改判为 **✅ 能做**（`text-styles-emotion-base-mapping.md` 那行要修）。

### ❌ 不能实现

| 动效 | 缺什么 |
|---|---|
| 抖动（3 档） | ✅ **本来就已支持**（`shake_px`，不属 D 范围） |
| 砸入（3 档） | ✅ **本来就已支持**（`scale_in_*`） |
| 收缩 / 弹出 | ✅ **本来就已支持**（`fade_*` / `scale_in_*`） |
| 逐字跳起（3 档） | 逐字错峰 → 需要 E |

---

## 五、D 的完整收益（合并去看）

**D 做完之后，"文字能出片"的覆盖率：**

| 类别 | 原判定 | D 之后 | 净增 |
|---|---|---|---|
| 样式 | 0/21 | **3/21**（倾斜 3 档；「砸入·带压扁」只差等比外的缩放） | **+3** |
| 动效 | 16/18 | **18/18**（摇摆 3 档已查证能做，但上面 16 档里**已含**摇摆的误判） | 见下 |

⚠️ 原表把"摇摆 3 档"标成 `△ 整块近似`（降级），**实为 ✅ 完全能做**。
所以严格说：**动效能做的从 16 档变 18 档（18 档里 3 档是摇摆）**，不是"净增 3 档"。

⚠️ **这里要重新盘一次**：`text-styles-emotion-base-mapping.md` 里"摇摆 3 档"被错误地
归成"△ 整块近似"，实际它是**✓ 能做**（rotation 通道已存在）。**该文档那一行要修正。**

---

## 六、施工清单（待确认后动手）

### 6.1 改动点

| # | 文件 | 改什么 |
|---|---|---|
| 1 | `dhampir-timeline/src/layer.rs` | `SubtitleStyle` 或条级结构加 `rotation_deg: Option<f32>`（`skip_serializing_if`，`None` = 老行为） |
| 2 | `dhampir-core/src/overlay.rs` | `TextItem` 加 `rotation_deg: f32`（照 `dx_px` 的先例） |
| 3 | `dhampir-core/src/compose.rs` | 求值时把轨/条的 rotation 填进 `TextItem` |
| 4 | `dhampir-worker/src/text_overlay.rs` | `paint` 里对位图做旋转（**CPU 侧**，因为文字是 CPU 叠的） |
| 5 | YekiTrim `tools/polish-to-dhampir.mjs` | 把样式的角度写进工程 |

### 6.2 ⚠️ 关键实现选择（影响成本）

文字是 **CPU 叠图**，所以旋转要在 CPU 做。两条路：

| 做法 | 成本 | 质量 |
|---|---|---|
| **CPU 最近邻/双线性旋转整个位图** | 低 | 边缘可能略糊 |
| **把文字改成 GPU 图层**（走现成的 `inverse_affine`） | **高**（要改架构） | 最好，且与素材同一个引擎 |

⇒ **建议先用 CPU 旋转**（改动局限在 `text_overlay.rs` 一个函数），
验证效果后再决定要不要把文字挪进 GPU 图层。

### 6.3 验证要求（按既有口径）

- ✅ `Option<T>` + `skip_serializing_if` ⇒ **老工程逐字节不变**（R4）
- ✅ 真机出片对比：**同一工程、开/关 rotation**，两帧像素差必须**只在文字区域**
- ✅ 文字**不该**因为旋转而移动锚点（旋转绕中心，不是绕左上角）
- ❌ 不许写"应该可以" —— 要有出片帧 + 像素数

---

## 七、待你拍板的两个问题

1. ~~摇摆的往复要不要做？~~ **已查证：能做**（rotation 是关键帧目标，字幕轨有 layers）。
2. **非等比压缩（`scaleY`）** 要不要一起加？
   它是「砸入 · 带压扁」缺的那一半，而且**成本很低**（`TransformV2` 加一个字段）。
   加了之后，稿里 4 档样式 + 「砸入·带压扁」全部成立。

---

## 八、⚠️ 施工中修正（2026-10-12，由 data-model 实测抓出）

原 6.1 表把 C 写在 `compose.rs` —— **错**。三处实测：

| 原写法 | 实际 |
|---|---|
| 「传进 `compose.rs`」 | `grep -c TextItem compose.rs` = **0**；`TextItem` 只有 `overlay.rs:506` 一个构造点 |
| 「`rotation` 可走关键帧」 | 那是 **compose 那条路**（图层变换）；`evaluate_overlay` 里 `channel_from/​animated/​local_frame` **零命中**，**不求值关键帧** |

⇒ **C 落在 `overlay.rs::evaluate_overlay`，`compose.rs` 不动。**

### 四个连带决定

| # | 决定 | 理由 |
|---|---|---|
| 1 | **静态透传**（不求值） | 让 `evaluate_overlay` 再求一遍 = **第二份关键帧求值** ⇒ 两份实现必然漂移 |
| 2 | **`TRANSFORM_TARGETS` 不加 `scale_y`** | 按 #1 无人求值 ⇒ 加了是**死字段**，用户关键帧被静默忽略（比不加更坏） |
| 3 | **`TransformView` / `Layer` 不动** | `host_api.rs:313` 自己写着"改名属破坏性改动，不在此顺手做" |
| 4 | **不往 `TextStyle` 加同名字段** | `dx_px` 源是轨道级样式，`rotation` 源是**元素 transform**，加进去 = 第二份真相 |

### ★ 必须写进代码注释的限制

> `rotation` 走的是 `element.transform.rotation` 的**静态值**；关键帧求值在 `compose.rs` 那条路上，
> 与本字段**尚未连接**。要让文字旋转可动画，需要在求值层打通。

**理由**：不留这句，后来的人会以为"字段有了 ⇒ 动画就有了"。
**这正是本次抓到的那个坑，必须留在代码里。**

### ★ T3 必须知道的 JSON 坐标

倾斜要写在**文字图层自己的 `layers[i].transform.rotation`** ——
写在 `TrackV2.subtitle` 里**不会生效**（`evaluate_overlay` 读的是 element 的 transform）。
`scale_y` 同理：`layers[i].transform.scale_y`。
