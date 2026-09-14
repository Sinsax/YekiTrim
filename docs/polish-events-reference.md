# 精修事件参考（种类 / 参数 / 编辑器覆盖范围）

> 依据代码（提交 `de5a12c`）：事件定义 `crates/vtrim-render-core/src/types.rs`；
> LLM 出参 `crates/vtrim-polish/src/llm/{segment,events}.rs`；后处理
> `crates/vtrim-polish/src/process/{mod,fx,sfx,download}.rs`；预览求值 `webui/src/polish/engine.js`；
> 出片求值 `crates/vtrim-render-core/src/timeline.rs`（Chrome/GSAP 路径见 `crates/vtrim-polish/templates/index.html`）。

## 0. 一个事件的一生（四层）

| 层 | 位置 | 干什么 |
|---|---|---|
| 定义 | `render-core/types.rs` 的 `Event`（serde tagged enum，`type` 字段） | 15 个变体 + 结构类 `Segment`/`Subtitle`/`MuteRange` |
| 生成 | `polish/llm/segment.rs` Step1（分段）+ Step2（逐段生成事件） | LLM 按 prompt 写 `type/time/duration/...` |
| 后处理 | `process/mod.rs`（剥静音区间、贴纸落点/时长、预算、音效规则）`process/fx.rs`（瞬时 duration clamp）`process/download.rs`（资产） | 把 LLM 的粗事件补成可出片的事件；写回 `polish.toml` |
| 消费 | 预览 `engine.js`（DOM 自绘）／出片 `timeline.rs`（wgpu 原生、wasm）／`templates/index.html`（Chrome+GSAP） | 逐帧求值成画面与声音 |

`polish.toml` 里一段视频的全部事件都挂在 `[[segments]]` + `[[segments.events]]` 下（`segments[].events[]`）。

---

## 1. 总表

| type | 类别 | 主要参数 | 时长 | 编辑器可改 |
|---|---|---|---|---|
| `camera` | 持续 · 运镜 | `zoom`(中/近)、`origin`(x% y%) | 事件自带（秒） | 时间/时长（**zoom、origin 不能改**） |
| `sticker` | 持续 · 贴纸 | `keyword`、`sticker_tag`、`name`、`source`、`x`、`y`、`sfx`、`sfx_volume` | 1.5~2.5s（prompt 建议；process 可延长到后一条字幕结束，≤ +50%） | 时间/时长/keyword/name |
| `overlay` | 持续 · 覆盖层 | `overlay_type`(color/gradient)、`color`、`opacity`、`colors[]`、`shape`(linear/radial) | 0.5~5s（prompt 建议） | 时间/时长（**其余不能改**） |
| `sfx` | 点 · 音效 | `name`(音效 id)、`volume`、`duration` | 默认整段音频 | 时间/时长/name（**volume 不能改**） |
| `shake` | 瞬时 · 视觉 | `intensity`(默认 14) | cap 0.5 / 默认 0.3 | 时间/时长（**intensity 不能改**） |
| `flash` | 瞬时 · 视觉 | `color`(默认 `#ffffff`) | cap 0.3 / 默认 0.25 | 时间/时长 |
| `blur` | 瞬时 · 视觉 | —（固定 8px） | cap 0.4 / 默认 0.3 | 时间/时长 |
| `zoom_bounce` | 瞬时 · 视觉 | —（当前 zoom ×1.10） | cap 0.6 / 默认 0.4 | 时间/时长 |
| `vignette` | 瞬时 · 视觉 | —（四角暗角） | cap 0.6 / 默认 0.4 | 时间/时长 |
| `hue_shift` | 瞬时 · 视觉 | `color`(默认 `#ff0040`) | cap 0.5 / 默认 0.35 | 时间/时长 |
| `stutter` | 瞬时 · 视觉 | `intensity`(默认 12) | cap 0.35 / 默认 0.25 | 时间/时长 |
| `split` | 瞬时 · 视觉 | `intensity`(默认 30) | cap 0.35 / 默认 0.25 | 时间/时长 |
| `noise` | 瞬时 · 视觉 | —（噪点 opacity 0.08） | cap 0.4 / 默认 0.3 | 时间/时长 |
| `pulse` | 瞬时 · 视觉 | —（zoom ×1.06 → ×0.97） | cap 0.5 / 默认 0.4 | 时间/时长 |
| `color_shift` | 瞬时 · 视觉 | —（sepia+saturate） | cap 0.5 / 默认 0.35 | 时间/时长 |

结构类（不是 event，但在同一条时间轴上）：

| 结构 | 字段 | 编辑器 |
|---|---|---|
| `[[segments]]` | `start`/`end`/`fx`(low\|med\|high)/`note`/`events[]` | 拖 / 缩放 / 双击删 / 备注 / 起止；**不能新增分段** |
| `subtitles`（`data/subtitles.json`） | `start`/`end`/`text`/`html` | 拖 / 行内编辑 / 拆分 / 设入出点 / 删除 / ＋新增 |
| `mute_ranges` + `silence_ranges` | `start`/`end` | 添加 / 拖 / 拆分 / 删除；「去静音」只写 `silence_ranges` 那部分（撤销只回退它） |

另外两个非事件的内建表现：片头 0.5s 黑场淡出（`intro-black`）、字幕/弹幕/贴纸图层本身。

---

## 2. 逐类说明

### 2.1 运镜 `camera`（持续）

```toml
[[segments.events]]
type = "camera"
time = 12.5
duration = 4.0
zoom = "中"          # "中" | "近"
origin = "50% 50%"   # 聚焦中心，"x% y%"
```

- `zoom` 只是**档位名**，实际倍率来自 `polish.toml [style.camera]`：`zoom_mid = 1.15`（中）、`zoom_close = 1.3`（近）。
- 聚焦中心优先级：**事件 `origin` > `layout.liver` 的自由选框中心 > `origin_grid` > `origin` > `grid[0]` 中心 > `65% 50%`**。
- 过渡：`min(duration*0.5, 0.6)` 秒，`pow2Out` 曲线；**竖屏（`vertical = true`）完全不运镜**。
- 编辑器：时间轴「运镜」泳道可拖/改时长；属性面板能改 `type/time/duration`；
  `zoom` 档位与倍率、`origin` **没有 UI**；想让运镜对准人物 → 右栏「区域编辑 → 人物」框选（写 `layout.liver.top_left/bottom_right`）。

### 2.2 贴纸 `sticker`（持续）

```toml
type = "sticker"; time = 10.0; duration = 2.0
keyword = "ysyk"        # LLM 写：来源包名（emoji/irasu/本地贴纸包）
sticker_tag = "害怕"    # LLM 写：语义 tag（只在生成期用，出片不再读）
name = "cry_01"         # process 解析后回填：实际文件名
source = "emoji"        # process 回填：emoji | irasu | 包名
x = "23%"; y = "71%"    # process 计算（LLM 一律写 "auto"）
sfx = "tag:コメディ"; sfx_volume = 0.4
```

- LLM 只负责 `time/duration/keyword/sticker_tag`；`name/source` 由 process 按 `[sticker_source]`（emoji / irasu / 本地贴纸包）解析回填。
- **落点由 process 算**：从 `layout.sticker_area` 的矩形里（自由选框优先，否则每个九宫格格子各算一个矩形）用确定性种子随机落点，写回 `x/y` 百分比。
- **时长自适应**：若下一条字幕在 `duration` 结束后 50% 以内结束，则延长到那条字幕的 `end`。
- 表现（预览与出片一致）：280×280 盒子，`popIn` 0.35s（`backOut(3)`，−15° 转正）→ `floatUp` 0.4s（上移 18px）→ 保持 `duration` → `fadeOut` 0.3s（缩到 0.4）；安全边距 32px，贴纸位置夹在 1080p 内。
- 竖屏：`vertical_position = left` → 贴纸水平居中；`center` → 左右交替。
- 编辑器：泳道拖/缩放；属性面板可改 `keyword`/`name`（改 `keyword` 不会自动重新解析，**要重跑后处理**）；
  `x/y/source` 不可改（后处理每次重算）；贴纸池在右栏「贴纸来源」勾选；预览资源来自 `/api/polish/project?preview=1` 的 `_assets.sticker_map`。

### 2.3 覆盖层 `overlay`（持续）

```toml
type = "overlay"; time = 0.0; duration = 5.0
overlay_type = "gradient"   # "color" | "gradient"
color = "#ff0000"           # color 用；gradient 用 colors[]
colors = ["rgba(0,0,0,0)", "rgba(0,0,0,0.6)"]
opacity = 0.3               # 默认 1
shape = "radial"            # "linear"(默认) | "radial"，仅 gradient
```

- 表现：进入 0.4s 淡入、结束前 0.4s 淡出；`color` 是纯色覆盖，`gradient` 是线性/径向渐变。
- 编辑器：泳道 + 属性面板（`type/time/duration`）；**`overlay_type`/`color`/`colors`/`opacity`/`shape` 只能手改 `polish.toml`**。

### 2.4 音效 `sfx`（点事件 + 挂在别的特效上）

两种用法：

1. **独立音效事件**：`{"type":"sfx","time":3.0,"name":"<音效 id>","volume":0.5,"duration":1.0}`
2. **挂在任何视觉事件上**：`sticker/shake/flash/blur/zoom_bounce/vignette/hue_shift/stutter/split/noise/pulse/color_shift/overlay` 都有 `sfx` + `sfx_volume` 两个字段（`sfx = false/null` 表示不响，`sfx = true` 会解析成 `pop-notice`）。

**可用音效目录**：`crates/vtrim-polish/assets/sfx_catalog.json`（小森平音效库，编译期内嵌）

| 项 | 数量 |
|---|---|
| 语义 id（LLM 能看到、能选的名字） | **1213** |
| 变体音频文件（同一 id 的多个素材，出片随机取一） | **2118** |
| 大分类 | 7：① 主播情绪反应 117 · ② 游戏战斗 156 · ③ 演出节目 119 · ④ 生活日常 308 · ⑤ 饮食水声 162 · ⑥ 环境氛围 235 · ⑦ 交通出行 116 |
| 策展 tag | ~75（`破坏倒塌`/`PC`/`电话`/`雨雷`/`魔法`/`猫狗`/`爆笑搞笑`/`爆炸`/`恐怖氛围`/`铁路轨道` …） |

**怎么选**：LLM 从「可用音效候选」（按本段方向 tag 预筛 + 去掉 `blacklist` + 去掉已用，限量）里选具体 id，或写 `tag:xxx` 交给系统匹配；候选外的一律无效。
另有 23 条**中文关键词 → tag 兜底规则**（`process/sfx.rs::SFX_RULES`，如 `炸/爆炸/地震 → tag:爆発`、`笑/哈哈/破防 → tag:コメディ`、`敲门/关门 → tag:ドア`、`下雨/打雷 → tag:雨`），并按顺序取首个命中。
**自动补位（Step 3）**：字幕命中规则但该时刻没有带声音的事件 → 在字幕起点 +0.2s 补一个 `sfx`（与已有声音/计划点 2s 内不重复，数量受预算限制）。

**预算**（`process/mod.rs::content_budget`）：

| 项 | 公式 | 上限 |
|---|---|---|
| 贴纸 | `min(round(字幕数 / 5), ceil(时长 / 12))` | `segment 数 − 1`（至少 1） |
| 音效 | `min(round(字幕数 / 8), ceil(时长 / 15))` | 同上 |

超出预算时按「均匀分布保留」抽稀（不是从头截断）。

**音量**：事件级 `volume` / `sfx_volume` > `polish.toml [sfx].volume`（默认 **0.1**）。
预览由 `webui/src/polish/audio.js` 播放 `sfx/<name>.mp3`（主音量可调、静音按钮）；出片由 `render/ffmpeg.rs` 混音。

**屏蔽**：`[sfx].blacklist`（项目）与全局 `.vtrim/config.toml [sfx].blacklist` 合并，写精确 id 屏蔽单个，写 `tag:xxx` 屏蔽整族。

**编辑器**：时间轴「音效」泳道（点事件）；属性面板可改 `name/time/duration`；**`volume` 没有 UI**。

### 2.5 视觉特效（11 种瞬时，同一条「视觉特效」泳道）

| type | 表现（预览 = 出片） | 参数 | duration cap / 默认 |
|---|---|---|---|
| `shake` | 抖动：+off → −off → ±off/2，0.2s 内四段 | `intensity`（默认 14px） | 0.5 / 0.3 |
| `flash` | 前 0.04s 冲到 0.2 不透明度再线性淡出 | `color`（默认白） | 0.3 / 0.25 |
| `blur` | 固定 8px：0.48s 满 + 0.32s 淡出（共 0.8s 窗口） | — | 0.4 / 0.3 |
| `zoom_bounce` | 前 0.08s 当前 zoom ×1.10 | — | 0.6 / 0.4 |
| `vignette` | 四角暗角：前 40% 保持 1.0 再淡出 | — | 0.6 / 0.4 |
| `hue_shift` | 0.15s 淡入 0.25 不透明度再 0.8s 淡出（共 0.95s 窗口） | `color`（默认 `#ff0040`） | 0.5 / 0.35 |
| `stutter` | 0.12s 内两段反向位移 | `intensity`（默认 12px） | 0.35 / 0.25 |
| `split` | 0.12s：缩到 0.95 + 斜切 + 水平位移 | `intensity`（默认 30） | 0.35 / 0.25 |
| `noise` | 噪点：0.08s 升到 0.08 不透明度，0.72s 淡出（共 0.8s 窗口） | — | 0.4 / 0.3 |
| `pulse` | 0.12s ×1.06 → 0.28s ×0.97 → 回正（共 0.48s） | — | 0.5 / 0.4 |
| `color_shift` | 1.0s：sepia 0.6→0 + saturate 1.4→1.0 | — | 0.5 / 0.35 |

- **`duration` 由系统接管**：`process/fx.rs::normalize_fx_durations` 把每个瞬时特效 clamp 到 `0.1 ~ cap`，缺失补默认值 —— LLM 写多长都会被改（prompt 里也明说了）。
- 编辑器：一条「视觉特效」泳道装全部 11 种（靠块标题/颜色区分）；属性面板改 `type/time/duration`；
  **`intensity` / `color` 没有 UI**；新增走泳道标签上的 `＋`（先弹窗选类型，再填时间）。

### 2.6 其余事件字段

- `anchor`（12 类事件上都有 `anchor: Option<String>`）：**声明了但没有任何代码读它**，目前是死字段。
- `Segment.fx`（`low|med|high`）：只进 Step2 的 user prompt（「段情绪密度」），**不参与任何渲染**。
- `sticker_tag`：只在生成期用于选贴纸，出片不再读。

---

## 3. 编辑器实现的范围（一张表看清）

**能改**（时间轴 + 右栏）：

| 能力 | 位置 | 写回 |
|---|---|---|
| 事件的**类型 / 时间 / 时长** | 时间轴泳道拖拽、右缘缩放；属性面板 `type/time/duration` | `PUT /api/polish/project {segments}` |
| 贴纸的 `keyword` / `name` | 属性面板 | 同上 |
| 音效的 `name` | 属性面板 | 同上 |
| 分段 `note` / `start` / `end`（拖、缩放、双击删） | 时间轴 + 属性面板 | 同上 |
| 字幕增删改拆、设入出点 | 字幕泳道 + 属性面板 | `PUT /api/polish/subtitles` |
| 屏蔽区间增删改拆 | 屏蔽泳道 + 属性面板 | `PUT /api/polish/project {mute_ranges}` |
| 「去静音」自动屏蔽 / 撤销 | 时间轴头部按钮 | `POST /api/polish/silence` |
| 贴纸来源池 `sticker_source` | 右栏「贴纸来源」 | `PUT /api/polish/project {sticker_source}` |
| 运镜聚焦框 / 弹幕区域框 | 右栏「区域编辑」 | `PUT /api/polish/project {layout}` |
| LLM 剪辑 / 渲染导出 / 封面 | 右栏 + 封面设计器 | `POST /api/polish/{llm,render,cover}` |

**不能改**（只能手改 `polish.toml` 再重跑后处理/渲染）：

`camera.zoom`（档位名）、`camera.origin`、`[style.camera] zoom_mid/zoom_close`、
`sticker.x/y/source`（每次 process 重算）、
`overlay.overlay_type/color/colors/opacity/shape`、
`sfx.volume` / 任意事件的 `sfx_volume`、
11 种瞬时特效的 `intensity` / `color`、
`segment.fx`、`anchor`。

**"改了但不生效"的坑**：改贴纸 `keyword` 后 `name/source` 仍是旧值 → 要重跑「后处理」（`POST /api/polish/process`，UI 上由「渲染导出」内部自动跑）；改 `layout`（区域编辑）不需要重跑。

---

## 4. 预览 / 出片一致性（现状与已知差异）

**已对齐**（逐条核对 `engine.js` ↔ `timeline.rs`）：

- 瞬时特效的求值窗口：shake 0.2s、blur 0.8s(0.48 满)、hue_shift 0.95s、noise 0.8s、color_shift 1.0s、stutter/split 0.12s、zoom_bounce 0.30s、pulse 0.48s、vignette = `duration*0.4` 保持后淡出。
- 瞬时 duration 上限表：`process/fx.rs` 与 `engine.js` 的 `TRANSIENT_CAP` 完全一致。
- 贴纸四阶段、字幕 fade+事件联动脉冲、弹幕滚动、运镜过渡时长与曲线。

**已知差异（都在缺字段的边界上）**：

| 项 | 预览 `engine.js` | 出片 `render-core` | 影响 |
|---|---|---|---|
| `[style.camera]` 整段缺失时的倍率 | `zoom_close \|\| 1.8`、`zoom_mid \|\| 1.5` | `CameraStyle::default()` = **1.3 / 1.15** | 只在 `[style.camera]` 缺失时才不同（正常 polish.toml 一直有这一段） |
| `flash` 缺 `duration` | 0.35s | `max(duration, 0.1)` | process 已把缺失值填 0.25，正常流程不会走到 |
| `shake.intensity` 为 `0` | 退回 14px | 直接用 0 | 只有手写 `intensity = 0` 才看得出 |
| 非 16:9 源视频 | 预览 `object-fit: contain`（留黑边） | 出片把源拉伸铺满 1920×1080 | 直播录像一般就是 16:9，不受影响 |

---

## 5. 默认值 / 上限速查

```
瞬时特效 duration：shake 0.5 | flash 0.3 | blur 0.4 | zoom_bounce 0.6 | stutter 0.35
                    split 0.35 | noise 0.4 | pulse 0.5 | hue_shift 0.5 | color_shift 0.5 | vignette 0.6
                    缺失时默认：0.3 / 0.25 / 0.3 / 0.4 / 0.25 / 0.25 / 0.3 / 0.4 / 0.35 / 0.35 / 0.4
                    下限一律 0.1s
运镜倍率：[style.camera] zoom_mid 1.15（中） zoom_close 1.3（近）
运镜聚焦：事件 origin > liver 自由选框中心 > origin_grid > origin > grid[0] > 65% 50%
音效音量：[sfx].volume 0.1（事件 volume/sfx_volume 覆盖）
音效条目：1213 语义 id / 2118 变体 / 7 大类 / ~75 tag；中文关键词兜底规则 23 条
预算：贴纸 min(round(每5条字幕1个), ceil(时长/12))｜音效 min(round(每8条字幕1个), ceil(时长/15))
      两者上限都是 segment 数 − 1（至少 1）
贴纸盒子：280×280，安全边距 32px；popIn 0.35 → floatUp 0.4（上移18px）→ hold duration → fadeOut 0.3
弹幕：1920×1080 画布上半，y = y/1920*540，travel_duration 默认 8s，opacity 0.9
片头：0.5s 黑场淡出（内建，不是事件）
```
