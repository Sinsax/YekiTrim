# P9 下游需求来源：V-Trim polish 的功能全表与分工边界

> **这份文件是什么**：从 **Dhampir 仓**的 `plan/p9-polish-parity-design.md` 里**整节迁出**的三节
> —— §2（下游功能全表）、§7（什么不进底座）、§9（接口与分工），外加 §10 里两条**只关乎下游**的边界。
> 迁移日 2026-09-30（Dhampir 当时 HEAD `8e98abf`）；**内容逐字未改**，只在标题与来源注上做了标注。
>
> **它不是什么**：不是底座设计。泛用化的设计原则、渲染侧做法、落地记录仍在 Dhampir 仓那份
> `plan/p9-polish-parity-design.md` 里，那边的对应小节保留了编号并指向本文件。
>
> **为什么要分开**：底座仓只放"底座自己的性质"。下游方言（字段名、默认值、业务布局）
> 属于本仓，翻译与解释都在这一侧（见 `docs/dhampir/README.md`）。

---

## 2. V-Trim polish 功能全表（从源码收齐）

来源：`vtrim-render-core/src/types.rs`（`Event` 15 变体 + `StyleRoot`/`Layout`）、
`vtrim-polish/src/project/config.rs`（`ProjectFile` 15 个 section）、
`templates/index.html`（14 个 handler）、`render/native/mod.rs`（FramePipeline）。

### 2.1 视觉事件（`Event`，15 个变体）

| # | V-Trim 变体 | 语义 | 起止 | Dhampir 现状（迁移当时） |
|---|---|---|---|---|
| 1 | `Camera` | 运镜：`中/近/远` 三档 + 聚焦点 `origin` + `anchor→anchor_to` 移动 + `cut` 硬切 + `handheld` 手持摇摆 | **持续**（跟到下一条） | ⚠️ 地基有（`Transform`+`Keyframe`），缺"keyframe 驱动 transform" |
| 2 | `Sticker` | 贴纸：入/浮/出三段缓动 + `manual` 时间锁 | 持续 | ⚠️ `AssetKind::Image` 有（静态），**GIF 动图无** |
| 3 | `Shake` | 抖动 | 瞬时 ~0.3s | ❌ 无 |
| 4 | `Flash` | 闪白（可带色） | 瞬时 0.25s | ❌ 无 |
| 5 | `Blur` | 模糊脉冲 `blur(8px)→0` | 瞬时 0.3s | ✅ **有** `gaussian_blur` |
| 6 | `ZoomBounce` | 缩放弹跳 1.10 | 瞬时 0.4s | ❌ 无（`scale` 机制有） |
| 7 | `Vignette` | 暗角 | 瞬时 0.4s | ❌ 无（V-Trim 自己也缺画） |
| 8 | `HueShift` | 色相叠加 | 瞬时 0.35s | ⚠️ 有 `hue`，但语义不同（旋转 vs 叠色） |
| 9 | `Stutter` | 卡顿（时间轴跳帧） | 瞬时 0.25s | ❌ 无（**时间轴层**，非像素） |
| 10 | `Split` | 分屏：`scale/skew_x/x` | 瞬时 0.25s | ❌ 无 |
| 11 | `Noise` | 噪声 | 瞬时 0.3s | ❌ 无 |
| 12 | `Pulse` | 脉冲 | 瞬时 0.4s | ❌ 无 |
| 13 | `ColorShift` | 色调偏移（sepia+saturate） | 瞬时 0.35s | ⚠️ 有 `saturation`，**无 sepia** |
| 14 | `Overlay` | 覆盖层：纯色/渐变（linear/radial）+ 4 stop | 持续 1.5s | ⚠️ 有渐变着色器，**不在注册表** |
| 15 | `Sfx` | 纯音效（无视觉） | 按文件长度 | ❌ 无（音频只拼轨） |

> **落地结果**（Dhampir P9 T8–T13 收口时补记）：这 15 条里缺的 8 个像素类特效
> （shake / flash / vignette / noise / overlay / zoom_bounce / pulse / split）
> 全部落进了 `ColorMask` 与 `Warp` 两条管线；`Stutter` 明确不进特效管线（它是时间轴行为）。

### 2.2 样式 / 布局 / 画布

| 组 | 字段（实测） | Dhampir 现状（迁移当时） |
|---|---|---|
| `SubtitleStyle` | font / size / color / stroke / highlight_color / entrance / position | ⚠️ 部分（`font_ratio`/`color`/`outline`/`max_lines`；**无 stroke/highlight/entrance**） |
| `DanmakuStyle` | font_size / color / stroke / highlight_color / speed / area / scroll_direction | ⚠️ 部分（`lanes`/`duration_ms`/`font_ratio`；**无 highlight/direction**） |
| `CameraStyle` | `zoom_mid=1.3` / `zoom_close=1.5` / `zoom_wide=0.55`（**只有这一张表**） | ❌ 无（但 `scale` 表达得了） |
| `Layout` | liver / danmaku_area / sticker_area / reaction_zone / game_area，每个 = 九宫格 `grid` 或自由选框 `top_left`+`bottom_right` | ❌ 无（**这是 V-Trim 特有业务**） |
| `CanvasConfig` + `StageConfig` | 方向 → 宽高；主画面位置；背景 `none/black/color/blur`；`blur_px`/`blur_img_scale`/`scale`/`padding` | ✅ `RenderHints{width,height,format}` 够 |
| `CoverConfig` | 20+ 字段：帧位置 / 标题断行 / 字体 / 描边 / 渐变方向 / 暗角 / 阴影 / `CoverEffect[]` | ✅ **`dhampir frame` 就是截帧** + 上述特效齐了即可 |
| `MuteRange` / `silence_ranges` | 屏蔽区间（去静音） | ⚪ **不属底座**（业务对时间轴的计算，见 §7） |
| `SfxConfig` | volume / blacklist / reactions_enabled | ❌ 无 |
| `FilterConfig` | danmaku/highlight 关键词 | ⚪ **不属底座**（LLM 业务） |
| `StickerConfig` + `sticker_source` | 落点策略 + 来源 | ⚠️ 落点 = `Transform`，够 |

### 2.3 复合结构（V-Trim 有、Dhampir 要能表达）

- **同帧多事件叠加**：一条时间轴上 blur + shake + flash + vignette 同时生效 ——
  所以特效**不能挤进一个定长 uniform**。
- **调整图层**：Dhampir 已有（`Layer::is_adjustment`，`layer.rs:198`）。
- **运镜是持续状态机**：跟到下一条运镜，不是一次性 —— 这正是 `Keyframe` 的形态。

---

## 7. 边界：什么**不**进底座

用户已定：`polish.toml ↔ 工程文件` 的**转译交给 V-Trim 那边**。据此，以下**明确不做**：

| 不进底座 | 为什么 | 谁做 |
|---|---|---|
| `polish.toml`（TOML）解析 | 那是 V-Trim 的格式；底座只有一种工程文件（JSON） | V-Trim：`polish.toml → dhampir ProjectDoc` 转译器 |
| `Layout` 九宫格 / 自由选框 | **V-Trim 特有的直播布局概念**，不是通用能力。转译时落成 `Transform` | V-Trim 转译器 |
| `FilterConfig` 关键词 | LLM 业务 | V-Trim |
| `MuteRange` / 去静音 | 对时间轴的计算，与渲染无关 | V-Trim |
| `sticker_source` 来源策略 | 素材挑选业务 | V-Trim |
| `CoverConfig` 的 20 个字段 | 封面是"截帧 + 上述特效"的组合；**字段是 V-Trim 的排版预设** | V-Trim 转译成一次 `frame` 调用 + 特效条目 |
| `CameraStyle` 三档倍率 | 转译成 `scale` keyframe | V-Trim |

**判断准则**：**凡是"这个直播间怎么排版"的，归 V-Trim；凡是"像素怎么算出来"的，归底座。**

---

## 9. 与底座（Dhampir）的接口（一次性说清）

```
V-Trim 侧（下游）                       Dhampir（底座）
───────────────                       ──────────────
polish.toml ──┐
              ├─→ 转译器 ──→ ProjectDoc(JSON) ──→ dhampir（预览 wasm / 出片 native）
clip.json  ───┘                 ▲
                                └── V-Trim 负责：布局→Transform、运镜→keyframes、
                                    封面→frame+effects、去静音→时间轴裁剪、
                                    关键词→已求值的事件列表
```

**分工一句话**：**V-Trim 决定"画什么"，底座决定"怎么画出来，且两端画得一样"。**

---

## 10. 附：两条只关乎下游的边界（原文迁出）

- **V-Trim 侧的转译器未设计**（当时只定了边界与分工）；现在转译器在本仓
  `tools/polish-to-dhampir.mjs`，守卫是 `scripts/check-vtrim-translator.mjs`。
- **§2 的表是从 V-Trim 源码读出来的**，但**没有跑过 V-Trim 的任何一条链路**
  （当时那个会话 shell 不可用）——"V-Trim 有 14 个 handler"来自 `templates/index.html`
  的 grep，不是运行结果。
