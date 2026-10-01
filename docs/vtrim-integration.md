# V-Trim → Dhampir 接入指导

这份文档写给**要把本仓的 `polish.toml` 喂给 Dhampir 出片的人**。
它只讲两件事：**怎么接**、**接了之后哪里会不一样**。

> **位置**：转译器在本仓 `tools/polish-to-dhampir.mjs`（零依赖 ESM，一个文件）；
> 它的守卫是 `scripts/check-vtrim-translator.mjs`（334 项逐值检查 + 12 处变异自证）。
> 事件类型 → 效果参数的映射表就在转译器的逐事件 `switch` 里（每条 `case` 的注释）。
>
> **它不在 `vtrim` 二进制的编译/运行路径上**：产品自己出片走 `vtrim polish render`（headless Chrome）。
> 这条路是给**底座出片**用的第二条通道，所以它宁可报错也不猜。
>
> **边界**：转译器**只读**本仓的 `polish.toml` / `clip.srt` / `clip.dm.srt`，旁写文件放在工程文件旁边；
> 它**不探测媒体**（`frame_count` 留空，由 `dhampir probe` 回填）；
> 它**不承诺兼容**：形状可以随本仓演化而改 —— 底座承诺的只有**底座仓**的 `docs/api-surface.md` 里那一栏。
>
> **词表**（正文里通篇出现这两个词）：「**底座**」= Dhampir（wgpu 预览 + 渲染底座，转译器的下游）；
> 「**参照**」= **本仓自己的出片模板** `crates/vtrim-polish/templates/index.html`
> —— canvas 那条腿的 `renderFrame` / `getActiveXxx`。正文里的 `index.html:NNN` 都指这个文件
> （行号已按本仓核过）。
>
> **范围**：本文是**第一~八节**。第九节（对比回路 `scripts/vtrim-compare.mjs`）与第十节（底座侧边界声明）
> 原本在 Dhampir 仓，**2026-09-30 已迁到本仓**：`docs/dhampir/compare-loop.md`、脚本 `scripts/vtrim-compare.mjs`。

---

## 一、它是什么形状

```
polish.toml  ──[tools/polish-to-dhampir.mjs]──▶  timeline JSON  ──▶  dhampir render
（本仓方言）             转译器                    （底座契约）          （出片）
```

**为什么不让底座直接吃 `polish.toml`**：底座要同时服务浏览器预览与服务器出片，
它的契约必须窄、可校验、与"某一代编辑器的配置形状"无关。
本仓的字段名、默认值、历史包袱属于**一方方言**，翻译放在边上是唯一能同时
"接得快"和"底座不脏"的形状。

**转译器不做的事**：
- 不探测媒体（`frame_count` 留空，由 `dhampir probe` 回填）
- 不解析系统字体（`--font-file` 显式给）
- 不猜缺省（凡是没有明确对应关系的一律进 DROP/CLAMP，见第五节）

---

## 二、快速开始

```bash
# 0. 改过转译器之后先跑守卫（不需要网络、不需要 GPU、不需要 ffmpeg）
node scripts/check-vtrim-translator.mjs            # 334 项：几条曲线的逐值检查
node scripts/check-vtrim-translator.mjs --mutate   # 12 处变异必须**全部打红**（自证不是空转）

# 1. 转译。第一个参数是**片段目录**（里面要有 polish.toml），不是 toml 文件本身。
node tools/polish-to-dhampir.mjs \
  "E:/media/<项目>/output/clips/<片段名>" \
  --out out/vtrim/t.json

# 输出：
#   转译完成 -> out/vtrim/t.json
#     轨道 12 条 / 素材 10 个
#     运镜 14 条、贴纸 4 张、音效 3 条、overlay 0 条、原声 3 段、字幕 ✓、弹幕 4 条 ✓
#     丢失 3 / 降级 2 / 注意 43
#     ---- DROP (3) ----   ...（详见第五节）

# 2. 校验（**不需要 GPU、不需要 ffmpeg**）
dhampir probe --project out/vtrim/t.json

# 3. 回填素材帧数（转译器故意不探测）
dhampir probe --project out/vtrim/t.json --write   # 或按 --help

# 4. 出片
dhampir render --project out/vtrim/t.json --from 0 --to 5811 --out out/dhampir.mp4 \
  --asset-root "E:/media/<项目>/output/clips/<片段名>" \
  --font-file C:/Windows/Fonts/msyh.ttc \
  --font-bold-file C:/Windows/Fonts/msyhbd.ttc
```

**字幕/弹幕是**：转译器把本仓的 `clip.srt` / `clip.dm.srt` 旁写成
`dhampir-subtitle.srt` / `dhampir-danmaku.ass`，**写在工程文件旁边**，
再在工程里登记成 `Subtitle` 素材。别把它们删了。

**字体必须显式给**（`--font-file`）。工程有字幕轨而你没给，render 会**判失败**而不是
偷偷用一个系统字体 —— 因为"同一份工程在两台机器上挑到不同字体"是两端可比最难查的一类失败。

**报告在 stdout**（`DHAMPIR_QUIET=1` 时只留开头那几行统计）。三级清单是**构建产物**，
别只当日志看（见第五节）。

---

## 三、契约里最小要知道的几条

| | |
|---|---|
| **帧号是整数** | 时间一律是 `Frame`（整数）。转译器负责 `秒 × fps` 的取整，取整规则见第四节 |
| **帧率是有理数** | `timebase = {num, den}`，不是 f32。素材自己的时间基与序列的分开记 |
| **关键帧帧号是图层相对的** | 不是绝对帧号。最后一个关键帧必须 **`< duration`** |
| **缓动写在"到达"的那个关键帧上** | 求值取的是相邻一对里**后一个**键的 `easing` = "怎么到达 b" |
| **没有素材 + 有特效 = 调整图层** | 这是**推导**出来的，不是标志位。`shake`/`flash` 这类必须挂这种层 |
| **有素材的层只跑模糊** | 别的管线挂上去会被 `validate_timeline_v2` 报 `effect_would_be_ignored` |

---

## 四、行为对照表（逐项）

这一节是**转译器的规格说明**。左边是本仓出片模板的行为（引用行号），右边是底座里的表达。

### 4.1 运镜（camera）

| 本仓（`getActiveCamera` / `index.html:1398`） | 底座 |
|---|---|
| 状态机：`zoom=1.0` 初值；`td = min(duration*0.5, 0.6)`；过渡发生在每个镜头的**起点** | 每个事件在 `srcStart` 与 `srcStart + tdFrames` 各写一组关键帧 |
| `hardCut = cut \|\| tgt==远 \|\| zoom==远` | 硬切时 `tdFrames = 0`，起点即终点 |
| `pow2_out(t) = 1-(1-t)²` | `Easing::EaseOut` —— **逐值相同**（`schema.rs` 有用例钉着） |
| `pow2_in(t) = t²` | `Easing::EaseIn` —— 逐值相同 |
| `ox = lerp(ax, bx, p)`，`origin` 是百分比 | `cameraOffset(z, origin) = (1-z)·(origin - 中心)`，单位是文档像素 |
| **手持摇摆** `handheldSway(t, amp)`（`index.html:593`）：两条正弦 × dx/dy/rot/scale | **按帧采样**成 `x`/`y`/`scale`/`rotation` 关键帧（见 4.2） |

> ⚠️ **源视频竖时本仓不出运镜**（`index.html:1413` 的 `CFG.vertical` 早退：`zoom=1`、`handheld=0`，
> 与画布方向无关）。**转译器没有这道门** —— 它不探媒体、判不了源是否竖，
> 所以竖素材会"参照不动、转译后照动"。见第六节第 10 条。

### 4.2 三条"按帧采样"的曲线

**凡是 `t` 的纯函数，就按帧采样成关键帧，不要用手写几条关键帧去猜它。**
这是本项目反复验证过的教训（手写必漏口径）。

| 曲线 | 为什么不能手写 |
|---|---|
| **手持摇摆** | 4 个通道 × 每条 2 个正弦分量，共 6 个频率/相位参数，手写关键帧必然失真。`amp` 是状态机上的 **0~1 连续量**（与 zoom 同一套 td/缓动），不是布尔 |
| **贴纸弹入** | 用 `back_out(p, 3)`（`s=3`），而契约的 `Easing::BackOut` 是经典 1.70158 —— 峰值 1.10 vs 1.25，**装不下这个参数** |
| **贴纸四段相位** | 四段的边界必须一起复现；四段都落在**窗口内部**（窗口 = `dur`，见 §14.3） |

代价是关键帧变多（手持一段 8.5 秒 ≈ 510 帧 × 4 通道），收益是**逐帧精确**。
底座的关键帧求值本来就是 O(log n) 二分 + 线性插值，这个量级不构成负担。

### 4.3 贴纸（sticker）

| 本仓（`getActiveSticker` / `index.html:1517`） | 底座 |
|---|---|
| 窗口 `[et, et + dur]`（**不溢出到事件之外**，见 §14.3） | 图层 `[startFrame, startFrame + windowFrames + 1]`（= `dur` 那一段 + 1 帧） |
| `popIn 0.35`：`scale = 0.3+0.7·back_out(p,3)`、`rot = -15+15·bp`、`opacity = pow2_out(p)` | 逐帧采样（4.2） |
| `floatUp 0.4`：`yOff = -18·SK·p`，**线性** | 逐帧采样 |
| `hold`：`yOff = -18·SK`，区间是 `0.75 → dur - 0.3`（**右端缩短**，给淡出让位） | 逐帧采样 |
| `fadeOut 0.3`：**贴在窗口末尾** `dur - 0.3 → dur`，`opacity = 1-p`、`scale = 1-0.6p` | 逐帧采样（4.2） |
| `SK = min(CW,CH)/1080`（`index.html:305`），盒子 `S = 280·SK` | `SK` 按短边算；`fit = S/素材宽` |
| 落点 `x: left + S/2, y: top + S/2 + yOff`（中心对齐） | `transform.x/y` 加 `+box/2` 把左上角换成中心 |

> **口径（见 §14.3，两轮修完）**：窗口 = **事件时长 `dur`**，淡出**移进窗口内部**
> （`dur - 0.3 → dur`）⇒ 出点 = 最后一帧。
> - 第一轮：窗口 `dur + 0.65` 够不到淡出（要 `t >= dur + 0.75`）⇒ 末尾硬切；
> - 第二轮：窗口放宽到 `dur + 1.05` ⇒ 贴纸比事件**多显示 0.4 秒**（用户实测：出点晚了）✗。
> 现在**五处**同一条口径：参照 `getActiveSticker`（`index.html:1527`）、
> 产生端 `tools/polish-to-dhampir.mjs`（`STICKER_WINDOW_SEC = 0`）、
> `crates/vtrim-render-core/src/timeline.rs:530`、`webui/src/polish/engine.js:705`，
> 以及 **DOM/GSAP 预览腿** `handlers.sticker`（`index.html:955`：`fadeStart = et + dur - fadeOut`、
> 隐藏在 `et + dur`；它的**上浮起点** `et + 0.25` 是这条腿自己的既有偏差，刻意不动）。

### 4.4 字幕（subtitle）

| 本仓（`getActiveSubs` / `index.html:1454`） | 底座 |
|---|---|
| `fadeIn 0.35` / `fadeOut 0.18`，**`if / else if` 互斥** | `text_envelope`，同样互斥 |
| 淡入：`opacity = pow2_out(p)`，`yOff = 20·(1-opacity)` | 同上 |
| 淡出：`opacity = pow2_in(p)`，`yOff = -8·(1-p)` ← **注意乘的是 `(1-p)` 不是 `(1-opacity)`** | 同上（两段口径**故意不对称**） |
| `cy = CH - 120·CKH` | `bottom_margin = (120 - 行盒/2)/H` |
| 换行安全宽 `CW * 0.875` | `SubtitleStyle::safe_width_ratio = 0.875` |
| CSS（`index.html:747`）：`font-size:72px`、`color:#dcbda0`、`stroke:12px #403c3b`、`font-weight:700`、`line-height:1.5` | `font_ratio` / `color` / `stroke_ratio` / `stroke_color` / `font_weight` / `line_height` |

**`entrance` 曾是死配置（2026-10-01 已从 schema 移除 + 读时吞掉，见本节第 2 条）**：`"cycle-5"` / `"fade-up"` 全仓（JS、Rust、webui）
**没有一个读者** —— 只被写进 toml、只在 `crates/vtrim-render-core/src/types.rs:570` 里有定义。
动效是写死的。转译器把它记进 DROP 并说明。

### 4.5 弹幕（danmaku）

| 本仓（`getActiveDms` / `index.html:1487`） | 底座 |
|---|---|
| `if (t < d.start \|\| t >= d.end) continue` —— **在屏窗口 = 素材自己的 `[start,end)`** | `exit = frame_at_ms(cue.end_ms) - 1` |
| `travel` **只管滚动速度**，不管在屏时长 | `scroll_start = enter + fade_frames`，`travel_frames` 独立 |
| ⚠️ **`End` 短于"滚完时刻"时，渐隐会在弹幕还没到左边缘时发生**（录播姬 XML 的窗口是固定 8 秒，中文弹幕的 travel 普遍 8.5~16 秒：实测 5 字弹幕在距左边缘 337px 处渐隐、39 字节那条在屏幕中段 728px 处就没了） | 转译器把 ASS 的 `End` 抬到 `max(End, start + fadeIn + travel)`（`danmakuExitMs`；**只抬不压**）。Rust 侧同一条规则是 `vtrim_core::danmaku::clamp_display_windows`，由守卫对拍常数 |
| `travel = (1080 + utf8_字节数*32)/150` 秒 | 同上（**按字节，不是字符**） |
| `x = CW - min(progress,1)·(CW + textW)` | 同上 |
| `y = (d.y/1920)·(540·CK)` → 归一化 `d.y/3840` | `lane_top_ratio` / `lane_spacing_ratio` |
| `TRACK_YS = [300,500,…,1500]` | `lane_top_ratio = 300/3840`、`lane_spacing_ratio = 200/3840` |
| **颜色只有轨道级**（`getActiveDms` 不返回颜色，`index.html:2006` 只读轨道级 `color`） | 轨道级 `color`；素材里的 `\c` 标签转译器**剥掉** |
| CSS：`font-size:42px`、`#ffffff`、`stroke:2px #000`、`font-weight:600`、`base opacity 0.9` | 对应字段 |

⚠️ **弹幕的排布按底座当前的设计**（用户 2026-09-30 拍板）：底座把弹幕画成"头像 + 用户名"那种列表，
与参照的滚动弹幕不是一回事 —— **这条不作为差异跟进**（不列 DROP、不列修复项）。
`travel` / 泳道 / 在屏窗口这些**数值**仍按上表对齐（那是"滚动速度与在屏时长"的口径，不是排布）。

### 4.6 瞬时特效（fx）

参照的 fx 全集是 `index.html:1382` 那一行（14 种）。**转译器对每一种都有 `case`**，
落到哪条表达如下（`kind` 是底座的效果名）：

| 本仓事件 | 底座表达（`kind`） | 挂哪 |
|---|---|---|
| `camera` | Transform 关键帧（zoom + `cameraOffset`） | 视频层 |
| **`shake`** | 视频层 `x`/`y` **逐帧**：固定 0.2s 的**四级阶梯**，`intensity \|\| 14` 直接当**像素** | 视频层（字幕/弹幕/贴纸**不跟着抖**） |
| ~~`stutter`~~ | **已下线**（2026-10-01）：与 `shake` 重叠 | — |
| ~~`zoom_bounce`~~ | **已下线**（2026-10-01）：与运镜的缩放重叠 | — |
| `pulse` | 视频层 `scale` ×1.06 / ×0.97（两档），窗口写死 0.48s | 视频层 |
| `flash` | `flash`（峰值 0.20） | 调整图层 + 逐帧 `opacity` 包络 |
| `hue_shift` | `flash`（峰值 0.25、色 `#ff0040`） | 同上 |
| `noise` | `flash`（**纯白薄纱** 0.08）—— **不用底座的 `noise` 特效** | 同上 |
| `blur` | `gaussian_blur`，`radius = 2 × CSS px`（CSS 的 `blur(N)` 里 N 就是 σ，底座 σ = radius/2） | 调整图层，**按帧拆层**（半径是参数，不能靠 opacity 混） |
| `overlay` | `overlay` 效果的覆盖层图层；**渐变方向 `angle`** 两边约定不同（CSS 0° = 向上 vs 底座 0° = 向右，`dir=(cos,sin)`）⇒ 转译器按 `base = css − 90` 换算（`overlayBaseAngle`，D5 修掉了"写死 0 = 正右方"这个被掩盖的偏差） | 视频层 |
| ~~`split`~~ | **已下线**（2026-10-01 类型收敛）：斜切在底座没有原语，本仓也就不再提供这一类 | — |
| ~~`color_shift`~~ | **已下线**（2026-10-01）：能力并入 `overlay`（纯色/渐变），sepia 底座本来就没有 | — |
| `vignette` | ✅ **已接**（2026-10-01 D4）：本仓 canvas 补上暗角绘制 ⇒ 转译器映射底座 `vignette`（`amount` 0.5 + `placeEnvelope` 包络） | ColorMask::vignette |

`flash` / `hue_shift` / `noise` 都走 `placeEnvelope`：那几条包络**各有各的分段点**、
攻击段只有 2~9 帧，所以与手持/贴纸同一个办法 —— **按帧采样**成调整图层的 `opacity` 关键帧
（前提：调整图层的 `opacity` 真的被渲染器读到）。

### 4.7 音频

| 本仓 | 底座 |
|---|---|
| `mute_ranges` = **把这几段掐掉**（不是静音） | ⚠️ **两条腿两种转法**（2026-09-30 定的口径）：<br>**出片**：`--keep-muted` ⇒ 主画面**整条**、事件时间**不重映射**，屏蔽区间交给出片尾部那道 `Ffmpeg::cut_ranges` 按**源时间**剪一次（与 chrome 那条腿同一处）。<br>**预览**：掐掉那几段（主画面拆段 + 所有事件重映射）—— 客户端靠"落在被掐掉的段里 = 没有对应帧"跳过它们。<br>⚠️ 两边都剪过 = 成片短一大截（12s 带 3s 屏蔽出成 6.0s）+ 音轨错位；两边都不剪 = 屏蔽区间照常播出来。回归钉子：`scripts/check-dhampir-wiring.mjs` |
| 原声是**默认就有的底** | 必须**显式**写一条引用视频素材的音频层（第一版漏了这个，症状是成片整个没声音） |
| `sfx` 的 `vol = ev.volume \|\| CFG.sfx.volume \|\| 0.1` —— **覆盖，不是相乘** | `layer.gain` |
| `shake_enabled` / 音效驱动画面 | ⚠️ **没接**：底座没有"音效驱动画面"这条链路 |

### 4.8 颜色 / 字体 / 运镜锚点（2026-09-30 三处修正）

| 项 | 口径 |
|---|---|
| **颜色写法** | `hexColor` 收 `#RGB` / `#RGBA` / `#RRGGBB` / `#RRGGBBAA`（可省 `#`）。⚠️ 以前**只认 6 位**：`#140A28CC` 返回 `null`，而 overlay 那处是 `if (!first) drop(...)` ⇒ **整条覆盖层事件不进成片**（不是"少了个透明度"）。两个色档不透明度不同时 → CLAMP（底座整层只有一个 `amount`，取平均并报出来） |
| **覆盖层四档** | ✅ 四档全接（2026-09-30 M2 补齐后两档）：`gradient`（色档 + 0.4s 进出包络）、`color`（单色 + 同一包络）、`fade_in` / `fade_out`（**整窗线性斜坡、不叠包络**，颜色缺省黑）。参照 `index.html:1642-1669`；底座那边四档都是同一个 ColorMask 的 `overlay`（`shape` = Solid/Linear/Radial），逐帧曲线走图层不透明度关键帧（`placeEnvelope`）。（**2026-10-01 复核**：新底座 overlay **正常出图** —— 窗口内平均差 39.33、窗口外逐字节相同、`angle=45` 方向与我们一致；台账里那条"D1 底座不出色罩"已撤回） |
| **`zoomFactor` 的兜底** | ✅ 照抄模板 `camFactor`（`index.html:578-585`）：**按档位**取默认（`近 1.5` / `远 0.55` / 其余 `1.3`）、认不出的档位按**中景**、`isFinite` 也要看。以前统一退回 `1.0` ⇒ **"远景"档整段不拉远**（成片里看不出来，报告里只有一行 CLAMP） |
| **字幕字体名** | ✅ **已接**（不再是 DROP）：本仓把 `style.subtitle.font` 解成 `--font-file`，并把**整个字体库**给 `--font-dir`，由底座按每条轨的 `font_family` 在库里找。字重靠库里有没有 bold 那一支；**行高底座也有字段（`line_height`）**，不是 DROP |
| **字体族与字重（2026-10-01 修）** | 两件都按**预览**钉死：① **族**：`find_font` 走 `SYSTEM_FONT_STACK`（= `PolishPreview.vue` 的 `.cap` 栈，`'LXGW WenKai','Microsoft YaHei','Noto Sans SC',sans-serif`），逐支"字体库 → 系统字体目录"解析 —— 本机没装 LXGW 时装了 Noto，而**预览是雅黑在前**，所以成片也必须落雅黑（曾经两边顺序不同：预览雅黑、成片 Noto ✗）；② **字重**：转译器给字幕写 `font_weight: 700`、弹幕 `600`（与预览的 `.cap{font-weight:700}`/`.dm{font-weight:600}` 同口径），而底座**不合成粗体** —— 它只在 `>=600` 时去要 `--font-bold-file`，所以那条参数必须给（`msyh.ttc` → `msyhbd.ttc`）；不给就是**笔画比预览细一圈**（用户报的"字体不对"）。可变字体（`NotoSansSC-VF.ttf`）没有独立粗体文件，此时就绪位会明说"没有粗体那一支" |
| **运镜锚点兜底** | ✅ 照抄出片模板 `index.html:1413`：`origin_grid → origin → 写死 65% 50%`，且 `parseFloat` 认不出的串（`origin = "center"`）**也落 65% / 50%**。以前"算不出锚点就整条丢" ⇒ 参照在缩放、成片不动 |
| **贴纸取图** | ✅ 两处旧伤已修（2026-09-30 M2）：**不许因为"没有 `sticker_tag`"提前丢**（`name` 才是后处理选定的文件名）；找文件**试全部图片扩展名**（`gif/png/jpg/jpeg/webp`，`name` 不带扩展名）。**落点**（`x`/`y`）是后处理的产物：写 `auto` 的工程要先跑一次 `process`（报错里会这么说） |
| **竖素材运镜门** | ✅ **已实现**（本节早先写着"还没复刻"，是过期文案）：探一次源尺寸（`probeSize`），竖的就整条跳过 camera（与参照 `index.html:1413` 同口径），**探不到进 CLAMP 明说** |
| **暗角 `vignette`（2026-10-01 D4）** | ✅ **两端都修**：① 本仓 canvas 出片**补上了暗角绘制**（`getVignette` 包络 + 归一化空间的径向渐变，`transparent 55% → rgba(0,0,0,0.5×包络)`）—— 以前**只有 DOM 预览有**（`handlers.vignette`），成片里压根不画；② 转译器从 **DROP 改成映射**底座 `vignette`（`amount` 0.5 + `radius` 0.7 + `softness` 0.35，逐帧走 `placeEnvelope`）。实证：同帧 A/B 四角更暗 3.6~14.5、**画面中央 0.00 不变**；报告的 DROP 从 3 条降到 **2** |
| **覆盖层方向 `angle`（2026-10-01 D5）** | ✅ 本仓补字段 + **四处求值**（`timeline.rs` / 模板 canvas 端点 + `overlayGradientCss` / `engine.js::gradientCss` / 预览），转译器按 **`base = css − 90`** 换算（CSS 0° = 向上、顺时针；底座 `dir=(cos,sin)` 0° = 向右），缺省 = 参照的 `to bottom right` = 135°。⚠️ 以前这里**写死 0**（= 正右方）—— 一个被"D1 底座不出色罩"这条**后来被撤回的**判断掩盖的偏差，D5 一并修掉（当时没人能在真机上对着看）；守卫加了 8 项角度换算 + 1 处变异（27 项映射 / 16 处变异全部打红） |


---

## 五、三级记录：DROP / CLAMP / NOTE

转译器**每次运行都打印一份三方清单**。这是这个接口最重要的产物 ——
它把"哪里不一样"从"事后靠对帧发现"变成"转译时就知道"。

| 级别 | 含义 | 使用者该做什么 |
|---|---|---|
| **DROP** | 参照里有、底座**根本不表达** | 判断能不能接受；不能就得扩契约 |
| **CLAMP** | 有对应概念但**表达力或口径不够**，转出来的与原意不同 | 逐条看，多数是"量化后可接受" |
| **NOTE** | 信息，行为一致或已按参照复现 | 供审阅；**含参照侧的不一致** |

**按级别统计**（2026-10-01 类型收敛后，演示工程）：**DROP 2 / CLAMP 0**（NODE 随样本走动，不再钉数字）。
⚠️ 这条曾经写"DROP 3 / CLAMP 5" —— 类型收敛（四类下线）与 D4（vignette 接底座）之后必须重数，
基线钉在 `scripts/check-dhampir-drop-list.mjs`（改一条就红）。

**NOTE 里有两类要特别看**：
- `**这是本仓出片模板的不一致，值得本仓自己修**` —— 比如贴纸那条"够不到的淡出"，
  以及后来那条"窗口溢出到 `dur + 1.05`、出点晚 0.4 秒"（两轮都已修，见 §14.3）。
  **这类要收进第八节。**
- `这一条用了事件自己的 origin（…），与 layout.liver 中心（…）不同` —— 这类是**如实记录**，
  因为参照确实允许两者不同，不是错。

**给下游的用法**：把清单当**构建产物**接进流程，不要只打日志。
例如 CI 里断言 `DROP == 0`，或把 CLAMP 逐条挂到"已知差异"的台账上。

---

## 六、当前已知的差异（**未对齐，按影响排序**）

| # | 差异 | 影响 | 状态 |
|---|---|---|---|
| 1 | 字幕**折行缩字**没交给底座 | 参照装不下会缩字号（下限 0.7、最多 3 行）；转译后底座直接丢行 | **已做**：底座加 `shrink_min_scale` + `keep_all_lines`。⚠️ 光缩字不够 —— 它的闭式只保证 `总宽×scale == 可用宽×3`，而**贪心折行每行会浪费**，实测会折出 4 行，必须同时"不截断" |
| 2 | `max_lines` 默认 2（参照 3） | 长字幕的断行位置可能不同 | **已做**：转译器写 3（配合第 1 条，3 行是"缩完装得下"的结果） |
| 3 | **高亮词 `.hl`** 未实现 | `clip.srt` 有 `<span class="hl">` 时颜色不生效 | **已做**（两端）。⚠️ 本样例的 srt 里**没有** `.hl`，是**造了带标记的工程**验的：`parts` 颜色正确、像素层 R/B 比 3.6（高亮 3.8）vs 1.37（正文 1.375）。**【残差】** 段间水平推进量两端不同源：浏览器 `measureText` 与画字同引擎（准），CLI 的 `drawtext` 拿不到别段宽（用布局的逻辑字宽，中英混排差几像素） |
| 4 | 弹幕**逐条滚动时长** | 参照的 `progress = (el - fadeIn) / travel`，**travel 逐条**；底座在**轨道级**取平均 | **已做**（**没动契约**）：转译器把 `\move` 写成 **6 参数** `\move(x1,y1,x2,y2,0,travelMs)` —— ASS 原生表达"滚多久"的方式；底座读它，读不到才回退轨道级。**【残差已补（2026-10-01）】** `\pos` 静态 cue 现在也追加**零位移** `\move(0,0,0,0,0,travel)`（底座只读 `t2−t1`、坐标忽略）—— ⚠️ **防御性**：本仓素材只出 `\move`（全仓 grep `\pos(` 为 0），这条是给外部素材兜底；守卫 5 项 + 1 处变异 |
| 5 | `stage` 的 fit（cover/contain）+ 模糊底 | 参照能按 `axis`/`position`/`scale` 摆放主画面、还能铺模糊底 | **cover 已做**（`f` 乘进相机 `scale`）：竖屏画布读数 **87.98 -> 44.20**、位移 **−24 -> +2**。几何经**三种独立方法**确认正确（含 `stageDrawRect` 的权威实现）。**剩下的不是几何，是重采样**（本仓手写双线性放大 1.7778x，比浏览器 `drawImage` 软 2.5 倍）—— 那属于**跨项目**、且会同时改两端，是独立的一件事。**`contain`（摆位）已做**（2026-10-01 更正：`stageDrawRect` 实现了 `axis=x/y` + `position` + `padding` + `scale`；以前的"只做了 fill"是过期说法）；**背景层**：配方已实现、**被 `BACKDROP_ENABLED` 关着**。2026-10-01 查清了两件事：① **能跑**（带 `bg1/bg2` 渲 200/1100/1500 帧全过、产品路径 12s 成功、无 OOM）；② **底座没画** —— 用**正确的 contain 夹具**（竖画布 + 横素材 + `position=center` ⇒ `axis=y`、主画面 `y=656..1264`、上边带就是底的地盘）渲两次（带/不带那两条背景轨），**成片逐像素相同**（上边带都是 `rgb(243,243,243)`，本该是纯黑）⇒ 底座收下了轨但静默忽略。已回报交接单 §0b。⚠️ 另：`radius` 上限 16 ⇒ σ8 vs 参照 σ28 ⇒ 只能一半（将来打开也要记 CLAMP） |
| 6 | 贴纸 GIF **逐帧延迟** | `AssetKind::ImageSequence` 需要一张延迟表 | **已做（两端）**：契约加 `Asset.frame_delays_ms`（可选，不改版本号）；底座加 `source_frame_at_delays()`（**按延迟累加**落帧，没表走老路）；转译器用 `ffprobe -show_entries frame=duration_time` 探逐帧时长，**只在非匀速时写**。⚠️ 本样例四张贴纸的 delays **全等 `[30]`** -> 字段**不写**、既有行为不变；端到端是用**自己造的非匀速 GIF** 验的（`50/500/50/50/200 ms` -> 落进 `frame_delays_ms`）|
| 7 | ~~音效驱动画面~~ | ~~参照的 `shake_enabled` 会按音效强弱触发抖动~~ | **不是差异**：`shake_enabled` 在参照里**全仓只有定义与赋值、没有任何读者**（`config.rs:240/256`、`commands.rs:681`）—— 死配置。`data/loudness.json` 的响度只用于**音频跟随**与预览波形，**不驱动画面** |
| 8 | 封面（`polish cover`） | 另一条输出通道（静态图），底座不做 | 有意不做 |
| 9 | 预览侧的**事件联动脉冲** | 字幕 `scale 1.25 / y -20`，**只在预览**、不在出片 | 未做（不影响出片） |
| 10 | **源视频竖时少了 `CFG.vertical` 那道门** | 参照整条不出运镜（`zoom=1`/`handheld=0`，`index.html:1413`），转译后底座照出 | **已做**：转译器加 `probeSize()`（**只问宽高**，不带 `-count_frames`）判 `h > w`；竖素材时**整条跳过 camera 事件**（不是丢掉 —— 参照本来就不画）；探不到就 CLAMP 明说。实测：竖素材运镜 **0 条**、横素材 **14 条**不变 |

### 类型收敛之后的实况（2026-10-01，本表的补充）

- **CLAMP 归零**：`split` / `stutter` / `zoom_bounce` / `color_shift` 四类**已下线**（词汇表 15 → 11，
  `spec/effects.json` 为权威）；老工程加载时由 `project/validate.rs::retired_effect_message`
  用人话拒绝（"该工程含已下线的特效事件（…）"+ 两条路），**普通解析错误原样透传**。
- **`vignette` 不再是 DROP**：本仓 canvas 补画 + 转译器映射（见 §4.8）⇒ 报告 **2 DROP / 0 CLAMP**
  （剩下两条：`cover` 与 `style.subtitle` 的文字阴影；`entrance`/`cycle-5` 已于 2026-10-01 **从契约移除**）。
- **`overlay` 并入「视觉特效」泳道**，泳道成员由词汇表 `lane` 派生；
  渐变的**方向**参数见 §4.8。
- **对拍判据加了"位移可识别性"闸门**（`scripts/vtrim-compare.mjs`）：残差曲线平坦时不许下
  "几何差"的结论（竖屏样本上就误报过 −10px；自己逐点量过：最佳位移与 0 位移只差 0.00~0.05dB）。
  修好后三样本：`demo` 15.07 / `portrait` 9.87 / `real` 9.77，**位移全 0**、时长差全 0.000。
  ⚠️ 竖屏的历史读数（本表第 5 条）里 `−24px` / `+2px` 是**另一个口径**（那时还没这道闸门），
  不要与上面的数字直接比。

> **⚠️ 第 4 条里的数字被更正过两次 —— 两次都是"我算错了，而不是底座差"。**
>
> 1. 文档原先写"参照逐条 **20.2/16.4/16.8/14.2** 秒、底座平均 **16.9**"。
>    照源码公式 `(1080 + 字节数*32)/150` 重算是 **15.52/10.40/12.11/9.76**、均值 **11.95**。
> 2. 根因：`rawCues` 的 `cue.text` **还带着 `{\move(...)}` 标签**，
>    `Buffer.byteLength` 把标签字符也数了（39 字节 -> 61 字节）。
>    剥标签后与素材 `data/danmaku.json` 的 `travel_duration` **逐条吻合**。
>
> **教训**：那份"参照读数"**本身是错的**，而它看起来完全像"底座的公式选错了"。

### 第 5 条：**已经量到读数了**（把画布改成竖屏）

原话说"本样例不显形" —— 那是对的（源 16:9 == 横画布 -> `bg = none`）。但**只改一行配置**就能显形：

```toml
[canvas]
orientation = "portrait"      # 只改这一行，源 1920x1080 就铺不满 1080x1920
```

量出来的读数（`vtrim-compare`，无事件，纯基线）：

```text
横画布（原样）      基线 像素差 7.01    高频比 0.98    位移 0
竖画布（只改方向）  基线 像素差 87.98   高频比 0.64    位移 -24px
```

**而且真正缺的**（看帧就清楚）**是 `stage` 的 fit，不是那层模糊底**：

* 参照：源被 **cover 裁切铺满** 竖屏画布（`axis="fill"` -> `object_fit=cover`），**画面填满、无黑边**
* 本仓：源按**原始尺寸 1920x1080** 画在 1080x1920 上 -> **上下大黑边 + 内容错位**

（`bg = "blur"` 在 cover 后面根本看不见 —— 所以"模糊底"是**次要**的，**fit 才是主因**。）

**所以第 5 条的正确标题应该是「`stage` 的 fit（cover/contain/position）整个没做」**，
模糊底只是它的一个附属。要动的是**转译器的相机几何**（`cameraOffset` 现在
**假设源铺满画布** —— zoom=1 时 offset=0；比例不匹配时这个假设不成立）。

**验收用的测试台已经有了**：把 `orientation` 改成 `portrait`，`vtrim-compare` 直接给读数
（87.98 vs 7.01）。**这是"只改配置、不用重跑 `process`"就拿到读数的路子。**

#### 第 5 条的进展：**cover 那一半做了，但比例过冲**

转译器现在会算一个 **fit 因子**乘进相机 `scale`。推导（这是唯一要改的地方）：

```text
参照：p_out = z*(p_in - origin) + origin，而 p_in = f*(p_src - c_src) + c_out
代入：p_out = z*f*(p_src - c_src) + z*(c_out - origin) + origin
底座：p_out = scale*(p_src - c_src) + c_out + offset

  ->  scale  = z * f                        <- 只改这里
  ->  offset = (1 - z) * (origin - c_out)   <- 现有的 cameraOffset，**一字不动**
```

取 `f = max(cw/sw, ch/sh)`（cover）。实测（竖屏画布）：

| | 基线像素差 | 高频比 | 位移 |
|---|---|---|---|
| 改之前 | 87.98 | 0.64 | −24px |
| **改之后** | **44.20** | 0.06 | **+2** |

**方向对（位移基本归零）**，一度以为"比例过冲" —— 但**后续两个独立测量否掉了那个判断**（见下）。

**向后兼容已验**：横屏（比例一致）时 `f = 1`，关键帧值一字不变。

#### 第 5 条的**当前实测状态**：`f` 是对的，差的是**采样**

两个**互相独立**的测量都说 `f = max(cw/sw, ch/sh) = 1.7778` **是对的**：

| 测法 | 结果 |
|---|---|
| 横屏参照（1:1）与竖屏参照的**竖向剖面**比 | `f = 1.77`，四个时刻一致 |
| 竖屏参照与本仓**直接比尺度**（同 1080x1920，无需换带） | `g = 0.90~1.00` —— 特征同大 |

⚠️ 先前那句"看帧就能看出本仓放得比参照大"是**肉眼判断，不可靠** —— 被上面两个数否掉了。
**"看帧觉得不对"不能当结论**（这一条我自己踩了）。

**剩下的是采样**：

```text
t=5s 竖屏：整帧差 47.08
高频（Laplacian 均值）：参照 8.73   本仓 3.52   比 0.403
```

**本仓比参照"软" 2.5 倍** ✗。差的块集中在**中/下 1/3 的左侧**（行 720/1440/1680、列 240~720）。

#### 查到权威实现了 —— **几何第三次被确认正确**，差的是**重采样**

参照的 canvas 那条腿（`index.html:1793` 的 `stageDrawRect` + `renderFrame`）：

```js
// axis=fill -> cover：铺满短边、居中
var s = Math.max(bwid/bw, bhgt/bh);  w = bw*s;  h = bh*s;
x = bx + (bwid-w)/2;  y = by + (bhgt-h)/2;
// 本例 pad=0、scale=1 -> sr = (-1166, 0, 3413, 1920)，就是 1.7778x

// 相机是**绕 sr** 做的，不是绕画布：
var anchorX = sr.x + cam.ox / CW * sr.w;
var anchorY = sr.y + cam.oy / CH * sr.h;
```

把 `anchorY` 代进 `p_out = anchor + camScale*(p_sr - anchor)`，再与底座
`p_out = scale*(p_src - c_src) + c_out + offset` 对照（用 `s*c_src_y = 960 = c_out_y`）：

```text
scale  = camScale * s             <- 与转译器的 z * f 一致 ✓
offset = (1 - z) * (oy_px - 960)  <- 与现有 cameraOffset **逐值相同** ✓
```

**所以几何（fit 比例、锚点、位移）三处全部对** ✓ —— **不必再改几何**。

#### 那件"没查清"的事：**查清了 —— 是我推错了**

把 `stageDrawRect` 照搬进转译器之后，我把 `offset` 也换成**通式**：

```text
offset = anchor*(1-z) + z*sr + (z*FIT)*c_src - c_out
```

我一度以为它**在 cover 时与旧式 `(1-z)*(origin - c_out)` 等价**，并因为实测读数
从 44.20 变成 48.94 而**退回了旧式**。**那是错的**。两份转译器各转一次、直接 diff：

```text
764 个关键帧不同（全在 x 上），差值 105~175 像素
ox=0.4, z=0.55 时：旧式 1080*0.45*(0.4-0.5) = -48.66   （实测 -48.6648）
                    通式 3413.3*(0.4*0.45+0.275-0.5) = -153.80（实测 -153.8048）
```

**两式本来就不等价** ✗ —— 我的"证明"只验了 `origin = 0.5` 那一个点，而
**两式在恰恰那一点都归零**，于是验了个寂寞。

按 `anchorX = sr.x + cam.ox/CW*sr.w` 推，**通式才是忠实的那个** ✓，所以**用它**：

| | 横屏 | 竖屏 |
|---|---|---|
| 旧式（只 cover） | 7.01 | 44.20 |
| **通式（现在用的）** | **7.01** ✓ 不变 | **48.94** |

**48.94 不能当作反证**：竖屏那份的读数被"本仓手写双线性放大比浏览器 `drawImage`
软 2.5 倍"主导（高频比 0.06 ✗），4.7 是二阶效应。**以推导为准，不以那个读数为准。**

**教训**：拿一个"特例上相等"去证明"恒等"，是这一轮最贵的错误 ——
**换个参数就能分开的等式，不算验过。**

⚠️ **2026-10-01 更正**：下面这段"重采样软 2.5 倍"的结论**已撤回** —— 底座给了可复算的判据（1 像素棋盘放大后 100% 是插值中间值 ⇒ 双线性），并指出"高频能量比"是**内容相关量**（只作诊断）。保留以下正文作历史。

**剩下的是重采样**：本仓把源放大 1.7778x 用的是**手写双线性**（4 次 `textureLoad` + 2 次 mix），
而参照那边是浏览器的 `drawImage` —— **放大时核更宽、细节保留更多**。

这一条**不属于"两端可比"**（浏览器的 WebGPU 预览与原生 wgpu 走的是同一个着色器，两端一致），
它是**跨项目**的重采样差。要收得改采样核 —— 而那会**同时改掉两端**，是独立的一件事。

#### `contain` 与背景层：**已实现**，但端到端读数取不到

按 `drawStageBackdrop`（`index.html:1828`）复刻了背景层：

```js
var r = stageCoverRect(bmp, CW, CH);                 // cover
ctx.filter = 'blur(' + blur_px + 'px) brightness(0.5) saturate(1.15)';
```

**三件事值得记**：

1. **要拆成两层。** 底座的**有素材图层只执行 `SeparableBlur`** —— 把 `brightness`/`saturation`
   挂在同一个图层上会被整条丢掉（现在是校验错误）。所以：视频层只带 `gaussian_blur`，
   色彩用**紧跟其后的调整层**（那时画面上只有背景，碰不到主画面）。
2. **`brightness(0.5)` 可以精确表达**：底座没有"乘法亮度"，但
   `contrast(0.5)` = `(x-0.5)*0.5+0.5`，再叠加性的 `brightness(-0.25)` -> **恰好 `0.5x`** ✓。
3. **⚠️ 模糊底只能做到"一半"**：参照要 `blur(28px)`（CSS 的 N **就是 σ**），
   而底座 `gaussian_blur` 的 `radius` **上限 16**（= σ8），而且 `blur_radius()` 取的是
   **多个模糊里的最大值、不叠加** —— "多叠几趟凑 σ28"这条路走不通。
   转译器按上限发并进 CLAMP。

**端到端读数没取到**：把 `[stage] position` 设成 `top` 让 `axis` 走 contain 之后，
**参照自己的渲染就失败了**（ffmpeg `rawvideo` muxer：`Invalid argument`）。
所以 contain 这一半**实现完了但没验过** —— 要验得先查参照那条路为什么在这个配置下崩。

**第 5 条到此可以收**：fit 已实现；几何经**三种独立方法**确认正确；读数 87.98 -> 44.20。

### 浏览器端实时预览：**两端一致性实测 1.000000**

WebUI 的 Dhampir 预览走**浏览器端渲染**（Dhampir 的 wasm + WebGPU），原生 CLI 只负责出片。
在真机（无头 Chrome、`BrowserWebGpu`、1920×1080）上量到的两条读数：

| 读数 | 值 | 说明 |
|---|---|---|
| **浏览器腿每帧**（顺序播放） | **7.0 ms**（seek 0.7 / draw 6.2）→ **142.9 fps** | `--dev` 构建、合成工程、短距离 seek → **悲观下界的构建** |
| **原地重画**（不 seek） | 6.9 ms → 144.9 fps | 渲染本身的下界 |
| **两端一致性** | **SSIM 1.000000 / MAE 0.0000**（帧 0、30、89） | `check-dual-end`：浏览器逐帧导出 → worker 渲染 → `compare_project` |

**第 3 行是"两端同步"的实测依据** ✓ —— 但要说清它成立的条件：那是**合成语料工程**，
两端吃的是**同一份像素** ✓。真片段上浏览器用 `<video>` seek、原生用 ffmpeg 解码 ✓，
所以**逐像素不保证** ✓ —— 边界仍按 `plan/consistency-criteria.md`：能对齐的是**形状与图层清单** ✓。

**为什么不做服务端逐帧渲染**：实测同一段 31 帧，`frame` 的 **PNG 出口 1250 ms/帧** ✗
（h264 那条只要 ~30 ms/帧 ✓），而浏览器腿 7.0 ms/帧 ✓ —— 像素不过边界，就没有"传输"这个瓶颈 ✓。
另外冷启动一个进程要 **1.5–3.2 s** ✗、软件适配器单次上传 **244 ms** ✗，逐帧调 CLI 结构性不可能 ✓。

**浏览器端要的两件事**（后端只做这两件）✓：
`GET /polish/dhampir-wasm/*`（静态托管 wasm 包，`.wasm` 必须 `application/wasm` ✓）
`POST /polish/dhampir-project`（跑转译器，回工程 JSON **内容**）✓

**一处只有踩过才知道的坑**：转译器把字幕/弹幕写成**旁写文件**，而工程里那两条资产的
`uri` 是**绝对路径**（指向转译输出目录）✗ —— 浏览器取不到（`/api/media` 只服务项目内
相对路径 ✓），而且 `cleanup_project` 之后那个文件就没了 ✗。
**症状是最难查的那一类**：预览看起来正常、**只是没有字幕和弹幕**，还不报错 ✗。
所以 `/dhampir-project` 在 cleanup **之前**把两个旁写文件的**内容**一并带回去 ✓，
前端转成 blob URL 交给 `engine.loadSubtitles([{assetId, url, format}])` ✓（解析在 wasm 里 ✓）。

### ⚠️ 一条**底座的真 bug**（不在本仓修，但会挡住本仓的功能）

**常驻的"有素材图层"超过 1 个时，长片渲染必 OOM。**

复现（最小）：

```text
原工程（1 个常驻有素材图层）      --from 0 --to 5811   -> 通过
注入第 2 个常驻有素材图层（全时间线） 60 / 200 / 600 / 1100 帧 -> 通过
                                    1500 帧                 -> **OOM**
                                    **去掉模糊也一样**（所以不是模糊的问题）
```

- 阈值落在 1100~1500 帧之间 -> **逐帧增长**（每帧 ~16-33 MB，1500 帧就是 25-50 GB，12 GB 卡上爆）
- 症状：`thread 'main' panicked: wgpu error: Out of Memory`
  —— 在 `vtrim-compare` 里还会看到 `*rawvideo* muxer: Invalid argument`，
  那是本仓 panic 之后管道断掉的**下游症状**，**不是 V-Trim 的问题**（参照渲得好好的）
- **怀疑位置**：单层走**非分段路径**（不分配中间纹理）；一旦有第二层就走
  `render_segmented`（`timeline.rs:696`），它每个 stage 都 `alloc_texture` 中间纹理
  （`alloc_texture` 每次都 `create_texture` 新建，见 `timeline.rs:487`）——
  那批中间纹理**每帧都在涨**。`scene.rs:977-978` 也有同形状的"每次建两块"。

**影响本仓的一个具体后果**：第 5 条的**背景层**（模糊底）需要"多一个常驻图层"，
正好踩中它 —— 所以那条**暂时没发**（转译器里代码留着、开关关着，并进了 CLAMP）。

**另外**：底座的 `gaussian_blur` `radius` 上限是 **16**（= σ8），而且
`blur_radius()` 取的是**多个模糊里的最大值、不叠加** —— 参照的模糊底要 σ28，
所以即使修好泄漏，模糊底也只能做到**一半的模糊**。


| | 事件类型 |
|---|---|
| **已接（11）** | `camera` `shake` `sticker` `sfx` `overlay` `flash` `hue_shift` `noise` `blur` `pulse` |
| **已下线（2）** | `split`（底座缺 shear）、`color_shift`（底座缺 sepia）—— 2026-10-01 类型收敛删掉，**CLAMP 归零** |
| **DROP（1）** | `cover`（静态图另开一条）—— **`style.subtitle` 已接**（2026-10-01：底座 B5 给了 `shadow_color/dx/dy/blur_ratio` 四键，转译器按 `.cap` 的 `text-shadow:0 2px 12px rgba(0,0,0,0.4)` 写）—— **`vignette` 已接**（D4）、**`entrance` 已从契约移除**（Phase ③），`CLAMP` 为空 |

⚠️ **改事件覆盖时同步这张表**：它曾经在"补完 8 种特效"之后没更新、还写着"只接了 6 种"。
判据是转译器里的 `case` 个数（现在是 **11** 个：类型收敛删掉 4 类之后，别照抄本表；权威是 `spec/effects.json`）。

**2026-10-01 之后的清单口径**（演示工程实测 `丢失 2 / 降级 0`；类型收敛前是 `丢失 3 / 降级 2`，
更早是 `丢失 11`）：

- `layout.liver` 聚焦点、三条 `camera`、两条 `overlay` —— **已修**（见 §4.8），从 DROP 变 NOTE / 真转出去；
- `vignette` —— **已接**（D4：本仓 canvas 补画 + 转译器映射底座 `vignette`），不再是 DROP；
- `split` / `stutter` / `zoom_bounce` / `color_shift` —— **类型已下线**（老工程被人话报错拒绝），CLAMP **归零**；
- ✅ `[style.subtitle]` 那条**已清空**（2026-10-01：文字阴影接了底座 B5 四键；字体名 / 字重 / 行高早已接；`entrance` 已从契约移除）；
- 还没接的：贴纸 `auto` 落点的安全区推导（现在**报错并告诉人先跑一次 `process`**，不猜）、
  弹幕高亮色（`\c` 被剥）—— 逐条计划见 **`docs/plan/dhampir-parity-register.md` §五 M2**。
- ⚠️ **overlay 渐变色罩**：转译侧已正确（字段与底座自己的映射同一套），但**底座当前不出这个色罩** ——
  这是**底座侧的问题**（D1），本仓**不做近似补偿**，跟进见台账 §四。


### `shake` 的两份（**已对齐**，留作教训）

参照有**两份** `shake`，形状完全不同：

| 出处 | 用途 | 形状 |
|---|---|---|
| `index.html:532` `shake(tl,…)` | **GSAP DOM 预览** | `n = dur/0.05` 个 0.025s 的交替偏移 |
| `index.html:1575` `getShake(t, items)` | **canvas 出片** | **固定 0.2s 的四级阶梯**，与事件 duration 无关 |

**转译器一开始抄了预览那一份**（连续波形）。出片要的是阶梯：

```js
if (t >= sh.time && t < sh.time + 0.2) {
  var off = sh.ev.intensity || 14;          // 直接当**像素**用
  var dt = t - sh.time;
  if (dt < 0.04) return { x:  off,     y: -off     };
  if (dt < 0.08) return { x: -off,     y:  off     };
  if (dt < 0.12) return { x:  off/2,   y: -off/2   };
  if (dt < 0.16) return { x: -off/2,   y:  off/2   };
  return             { x: 0,        y: 0        };
}
```

现在按**逐帧**写进**视频层**的 `x`/`y`（`renderFrame` 里 `ctx.translate(shake.x, shake.y)`
在 video 那段 `ctx.save()` 之内 —— 字幕/弹幕/贴纸**不跟着抖**）。

**教训**：同一个名字在两条腿上可能是两种东西。**认的是出片那条腿。**
`scripts/check-vtrim-translator.mjs` 现在把阶梯逐档钉住了（含每一档的边界两侧），
并且把这处"抄错版本"做成了**变异自证**的一条（`--mutate` 的第 2 条）。

## 七、怎么扩：加一个新的本仓事件类型

1. 在 `tools/polish-to-dhampir.mjs` 的逐事件 `switch` 里加一个 `case`。
2. **先在本仓出片模板里找出片那条腿的实现**（`getActiveXxx` 或 `renderFrame` 里那一段，
   都在 `crates/vtrim-polish/templates/index.html`），
   不要看 GSAP 时间轴 —— 那是预览（第六节第 `shake` 条就是这么踩的）。
3. 判断它能不能用**现有契约**表达：
   - 能 → 用关键帧 / 调整图层 / 音频层表达
   - 是 `t` 的纯函数 → **按帧采样**（4.2）
   - 不能 → 在报告里 `clamp(...)` 说明差在哪，**不要静默近似**
4. **对帧验证**：出片后与参照逐帧比。**不要只看平均值** ——
   本项目有 4 个 bug 在 13 个时刻的平均差里完全看不出来（−0.08/−0.02/−0.82/0.00），
   只有把两边并排看才发现。
5. 有新的常量/口径，补一条**会被变异打红**的用例（"一条不会红的用例不算用例"）：
   纯函数提到模块顶层 `export`，然后在 `scripts/check-vtrim-translator.mjs` 里
   逐值断言 + 往 `MUTATIONS` 加一条（`--mutate` 会强制它打红）。

---

## 八、本仓自己可以修的问题（原「要给回 V-Trim 的清单」）

转译过程中发现的、**参照侧本身可以修**的问题 —— 现在这份清单的主人是**本仓自己**：

| # | 问题 | 出处 | 建议 |
|---|---|---|---|
| 1 | ~~贴纸的 `fadeOut` 相位**永远够不到**（窗口 `dur+0.65` vs 相位要 `dur+0.75`），贴纸硬切~~ **已修 ✓**；随后又修掉"窗口溢出到 `dur + 1.05` ⇒ 出点晚 0.4 秒"（见 §14.3） | `getActiveSticker`（`index.html:1527`） | 已落地：窗口 = **事件时长 `dur`**、淡出移进窗口内部（`dur - 0.3 → dur`）；四处同形判据（模板 / 转译器 / `timeline.rs` / `engine.js`）已统一 |
| 2 | 配置项 `entrance`（`"cycle-5"` / `"fade-up"`）**没有任何读者** | `crates/vtrim-render-core/src/types.rs` 的 `SubtitleStyle` | ✅ **已从契约移除**（2026-10-01 Phase ③）：schema 里的字段与默认值删掉，`project/legacy.rs::strip_retired_keys` **读时吞掉**（`deny_unknown_fields` 决定了不能直接删），`polish init` 模板也不再写它；回归 `cargo test -p vtrim-polish --lib retired_entrance_key_is_swallowed` |
| 3 | 同一个名字 `shake` 在**预览**与**出片**是两套完全不同的实现 | `index.html:532` vs `:1575` | 统一，或改名区分 |
| 4 | 字幕动效只在**预览**有"事件联动脉冲"，**出片没有** | `index.html:1134`（GSAP）vs `getActiveSubs` | 明确哪一端是想要的 |
| 5 | 出片的 `noise` **名不副实**：画的是一层纯白薄纱，不是噪声（真噪声只在 DOM 预览的 `handlers.noise`） | `getNoiseOpacity` / `renderFrame` | 改名，或做出真噪声 |
| 6 | 源视频竖的运镜门（`CFG.vertical`）转译器复刻不到 —— 它判不了源的方向 | `index.html:1413` | 要么让转译器探一次源尺寸（它已经在用 `ffprobe` 探贴纸），要么给个显式开关；见第六节第 10 条 |


---

# 能力逐项对账 + 逐层读数（2026-09 实测定稿）

> 这一节是**读数**，不是计划。判据全部来自真工程：
> 服务 `127.0.0.1:26320`，项目 2，片段 `clip-25` 与 `clip-04`
> （素材 `<media>/[2026-09-14 15-21-56-774][谁想和我甜旮旯]/output/clips/*`）。

## 1. 能力逐项：工程里**有没有** vs 预览**画没画**

| 能力 | clip-04 | clip-25 | 结论 |
|---|---|---|---|
| 视频层 | ✓ | ✓ | ✅ 60fps（真适配器）/ 24fps（回退适配器）|
| 字幕 | ✓ | ✓ | ✅ 图里可见 |
| 弹幕 | 0 | **12** | ✅ 图里可见 |
| 贴纸 `image_sequence` | **6 层 / 4 个 GIF** | 0 | ✅ 图里可见 **+ 判据 `isolated:true`** |
| 相机 / 缩放 | **17 事件** | 0 | ✅ 图里可见（帧 1933 `scale=1.51`，角色明显推近）|
| 抖动 `shake` | 1 | 0 | ✅ 与相机同类（都落在 `v1` 的 transform 关键帧上）|
| 层特效 `overlay` | **2 层** | 0 | ✅ 注册表 + `color_mask` 核心管线 + 注册表守卫 |
| 转场 `transition_in` | **无** | **无** | ⚪ **数据里不存在** —— 是"没用到"，不是"没实现" |
| 封面 `cover` | 有 | 有 | ⚪ 分离产物，不进片段渲染 |
| 音效 | 4 | 0 | ⚪ **口径内静音**（预览不带声音）|

**一条重要的澄清**：早先记的「`image_sequence` 完全没绑、那些层根本没画」是**错的** ——
贴纸一直在画（宿主侧 `<img>` + 引擎的位图优先路径都是通的）。当时看到"逐字节不变"，
是因为**引擎只在 bitmap 模式清位图**，video 模式下旧位图永不过期，于是"打断贴纸"
不会改变画面。那个 bug 已修（见下）。

## 2. 逐层读数（`text_probe`，clip-25 帧 1250）

底座**自带**逐层判定：`text_probe` 为每一行渲染一次「减去这一行」，与「全都在」相比，
**差出来的就是这一行的墨迹**。所以"逐层开关各出一个对比读数"不需要新功能。

```
engine.seek(1250)   →   engine.textProbe(1250)      # 顺序不能反，见下

ink: { bounds {x:129 y:811 w:1664 h:192}, lines_overlap: false,
       lines_pixels: 146213,  pixels: 146213 }      # 逐行之和 == 总和
lines[0]: "啊，说这个，我想到了啊，不是说我之前刚大一的时"
          font_px 72 / bitmap 1920x156 / placement.y 774
          ink { pixels: 97890,  inside: true,  on_edge: false }
lines[1]: ink { pixels: 48323,  inside: true,  on_edge: false }
```

* **每一行都有自己的墨迹像素数** → 这就是逐层读数。
* `inside: true` + `on_edge: false` → 字没被画面切。
* `lines_overlap: false` → 行与行不互相压。
* 逐行之和 == 总和 → 行之间不吞并。

**⚠️ 顺序**：`textProbe` 直接调会报「这一帧的行清单还没算过（现在存的是 None）」。
正确是 **先 `seek` 再 `textProbe`** —— `seek` 内部的 `prepareText` 会算那份清单。
**不要自己调 `text_frame`**：契约明说判定路径不要碰它（重算会作废刚提交的行位图）。

**边界**：`text_probe` 只判**字幕行**；弹幕单独计数（两者的判定规矩不同：
字幕顶到边是"被切"，弹幕每一趟进出都要过边，顶边是常态）。

## 3. 性能读数

| 项 | 读数 | 怎么量的 |
|---|---|---|
| 浏览器腿每帧（1080p 顺序播放）| **7.0 ms**（seek 0.7 / draw 6.2）→ 142.9 fps | 无头 Chrome / BrowserWebGpu |
| 两端一致性 | **SSIM 1.000000 / MAE 0.0000**（帧 0/30/89）| `check-dual-end` |
| `textMs`（2 行字幕可见）| **3.3 ms**（200 帧中位，clip-25）| `dhampir_project_text_probe` 同路的 `lastPrepareBreakdown` |
| `textMs`（无文字帧）| **0.3 ms** | 同上 |
| **文字栅格化缓存的效果** | **3.7 → 3.3 ms**（约 −11%）| 双跑：临时关缓存 vs 开着，同帧区间 |

**那条 −11% 说明**：字体渲染**不是** `textMs` 的大头；剩下的是
`upload_text_bitmaps` **每帧、每一行**无条件 `create_texture` + 重传。
**宿主一个人省不掉**（就算不交，wasm 手里还是上一帧那张位图，照样重传）。
完整改动清单、锚点、借用怎么办、判据，都在
`Dhampir/docs/plan/text-upload-cache.md`。

## 4. 两条环境坑（本地开发会遇到）

1. **本机 `node … > file` 写出来的是 UTF-16LE**（文件头 `b'\xff\xfe'`）。
   按 UTF-8 读会得到"看起来像乱码 / 解析失败"的东西 —— 我在这上面丢过好几轮。
2. **`cargo clean -p dhampir-wasm` 对 wasm 不够**：它只清 host target，
   wasm 产物在 `target/wasm32-unknown-unknown/`。依赖（`dhampir-core` /
   `dhampir-timeline`）的改动**进不去**，而 `wasm-pack build` 会报
   `Finished in 0.13s` 装作没事。要 `cargo clean --target wasm32-unknown-unknown`。


## 5. 原生出片（端到端实测，三次才全绿）

`dhampir render` 这一条腿原来的读数只到"能跑"，这一节是**三次真跑**的结果 ——
而**文件大小本身就是最干净的判据**：

| 缺什么 | 产物 | 说明 |
|---|---|---|
| 少 `--font-file` **且** 少 `--asset-root` | **4 KB** | 空画面、无字幕 |
| 补上 `--font-file`、仍缺 `--asset-root` | **16 KB** | 字幕进片了、画面还是空的 |
| **两个都给** | **527 KB** ✓ | **画面 + 字幕都在** |

全绿那次：

```text
exit=0 ✓   耗时 2.1s（30 帧 ≈ 70ms/帧，debug 构建）
1920x1080 / 60fps / nb_frames=30 ✓
文字覆盖层：字幕画了 19 行、被切 0、丢弃 0、画不出 0
           栅格化缓存 **命中 18 / 未命中 1**（约 95%）
```

**那句"缓存命中 18/1"值得单独看**：它是**原生侧**的栅格化缓存实测值 ——
我们给浏览器侧补的第 ① 步（`RASTER_CACHE`）就是同一件事。**浏览器侧以前没有这一层。**

### 两个参数是**必给**的，而且理由写在 help 里

```text
--asset-root <目录>   工程文件里 asset.uri 的相对根（**默认 target/s3**）
--font-file <文件>    工程里有字幕轨时**必须给**：本仓**不内嵌字体、也不猜系统字体**，
                      画不出来就判失败（问题码 subtitle_font_missing）
```

**"也不猜系统字体"是设计，不是缺陷** —— 宁可判失败也不猜。V-Trim 那侧两条都给
（`prepare_project(clip_dir, …)` + `find_font()`），所以真集成里本来就是对的。

### 三次失败的报告质量（这才是能自己修错的原因）

```text
第一次（cwd 里带方括号没加 -LiteralPath，PowerShell 把它当通配符）：
  asset_unavailable：ffprobe 读不了 **target/s3\clip.mp4** —— No such file or directory
  path: frame[0].source[clip.mp4]        ← 说出了**它试过哪个路径**

第二次（缺字体）：
  文字覆盖层：字幕画了 0 行 …… **画不出 19 行**   ← 说出了**每一类的行数**

每一次最后都写：
  出片报告判定为失败（问题清单或空帧不为空，或帧数对不上）
```

**它没有把"30 帧真 mp4 但字幕全丢"当成成功** —— 这是这个项目的"绝不静默"纪律。
对照本仓库里那些 `catch {}`（我在预览侧踩过）：**那才是把可诊断的错变成谜**。


## 6. 硬件适配：差距全在适配器，**没有可改的代码**

同一份代码、同一份工程，两台机器差 **2.5 倍**（60fps vs 24fps）。

### 证据链

| 读数 | 机器 A（本机） | 机器 B（宿主机） |
|---|---|---|
| `AdapterInfo.subgroup_min_size` | **32** | **4** |
| `AdapterInfo.name` / `device_type` | 空 / Other | 空 / **Other** |
| 每帧（1080p 顺序播放）| **~7 ms** → 60fps | 24fps |
| 分段（回退适配器上的工具栏）| `prep 4 / draw 1` | `prep 18 / draw 10`；`seek 0 / bitmap 9 / text 8 / set 0` |

**旁证（早前自己量过的）**：**软件适配器（WARP）上传一张 1080p 纹理 = 244 ms**。
机器 B 的 `prep 188ms / set 0` 与它同量级 —— 慢在**上传与合成的软件路径**上，
不在渲染逻辑里。

### 为什么"换成零拷贝"救不了它

引擎有一条 `video` 模式（把 `<video>` 直接塞进 GPU 纹理，省掉每帧
`createImageBitmap` 的 CPU 拷贝）。但实测在宿主机上**这条路直接 panic**：

```text
GPUQueue.copyExternalImageToTexture: 'source' member of
GPUCopyExternalImageSourceInfo could not be converted to any of:
ImageBitmap, HTMLImageElement, HTMLCanvasElement, OffscreenCanvas
```

**那张清单里没有 `<video>`** —— 不是设备怪癖，是 WebGPU 的输入类型就不含视频元素。
所以 `bitmap` 模式（每帧 `createImageBitmap(video)` + 上传）是浏览器里**唯一能走通**的路，
它那 ~9ms 是**结构性**的，不是可以优化掉的。已经做了两件事让宿主别用错：

* `Engine.setSourceMode()`（宿主可明确指定，探针本身用空 video 探不准）；
* 文档写明**不要**指定 `video` —— 第一次 panic 会把 wasm 里的 `ProjectHost`
  （一个 `RefCell`）**永久借住**，之后任何调用都 `RefCell already borrowed`。

### 结论

**硬件适配 = 让 WebGPU 拿到真适配器**（开硬件加速 / 换 Edge / 更新驱动），
**没有可改的代码**。查法：`chrome://gpu` 看 WebGPU 是不是 Hardware accelerated。


## 7. 三条记录下来的坑（都是实测踩到的）

### 7.1 **改完前端 / wasm 之后必须"强刷新"**

实测：Rust 侧接口与前端都改好、服务也重启了，页面看起来仍然**卡死 / 贴纸不动** ——
而 **Ctrl+Shift+R 强刷新一次就正常了**。普通刷新命中了 `dist/assets/*` 或
`www/pkg/*` 的**旧副本**。

**判断方法**：看控制台里加载的文件名哈希（`index-XXXX.js`）是不是**刚构建出来的那个**。
**代价**：这一条让我在"已经修好"之后又白查了一轮 —— 现象和真 bug 一模一样。

### 7.2 `clip-04` 的 `danmaku = 0` —— 那一段**本来就没有弹幕**

预览里"没有弹幕"是**对的**，不是缺口。要看弹幕请用 `clip-25`（`danmaku = 12`）。
两个片段的计数器实测（用 `tools/polish-to-dhampir.mjs` 转译后读 `extensions.vtrim.polish_toml`）：

| 片段 | `danmaku` | `dm` 轨 |
|---|---|---|
| `clip-04` | **0** | **没有** |
| `clip-25` | **12** | 有（lanes 8 / duration_ms 12391）|

### 7.3 `assetId` 与 `asset.uri` 是两样东西

```
assetId = "sticker/问号.gif"            ← 素材表的键
uri     = "assets/stickers/问号.gif"     ← **工程内相对路径**（服务端 join 的就是它）
```

给服务端传了前者 → `404 贴纸不存在：sticker/问号.gif`。
**那句话读起来像"文件丢了"**，其实是**参数给错了** —— 查这类问题时先打印一下字段真值。


## 8. 与 Dhampir 的分离：**只吃两个定版产物**

### 8.1 为什么改（现在这条路的两个坑，都实测踩过）

以前是"**指着 Dhampir 的源码树**"跑：`VTEDIT_DHAMPIR_WASM=<仓根>`，然后去读
`<根>/web/engine.js` 与 `<根>/crates/dhampir-wasm/www/pkg/`。代价有两条：

1. **V-Trim 得知道 Dhampir 的内部目录结构** —— 那边一挪目录，这边就断；
2. **它读的是活的源码树** —— "改了没生效 / 生效了又说不清是哪一版"。
   实测症状就是 **§7.1 那条：改完必须强刷新**，而且排查时连"我加载的是哪份 bundle"
   都要靠哈希去比。

### 8.2 Dhampir 出的两样东西（这就是全部契约）

```
node scripts/package.mjs [--out <目录>] [--clean-wasm] [--no-zip]

<out>/
  preview/
    engine.js                      ← web/engine.js
    pkg/dhampir_wasm.js            ← wasm-pack 输出
    pkg/dhampir_wasm_bg.wasm
  bin/
    dhampir(.exe)                  ← **release** 二进制（出片用）
  VERSION                          ← project_schema / git / built_at / platform
<out>.zip
```

`--clean-wasm` 默认**不开**，但它对应的那个坑写在脚本里：`cargo clean -p dhampir-wasm`
**清不掉 wasm 目标的依赖**，`wasm-pack build` 会报 `Finished in 0.13s` **装作没事**；
真清要 `cargo clean --target wasm32-unknown-unknown`（实测 41283 文件 / 20 GiB / 重建 29s）。

### 8.3 V-Trim 认的一个目录 + 两个变量

```
<程序目录>/                      ← vtrim(.exe) 所在（既有约定：stickers/、.vtrim/ 也在这层）
  dhampir/                       ← 把上面那个产物目录**整体拷进来**
    preview/  bin/  VERSION
  dist/  stickers/  tools/  .vtrim/ …
```

| 变量 | 默认值 | 作用 |
|---|---|---|
| `VTEDIT_DHAMPIR_PREVIEW` | `<程序目录>/dhampir/preview` | 预览包（里面 `engine.js` 与 `pkg/` **平铺**）|
| `VTEDIT_DHAMPIR_CLI` | `<程序目录>/dhampir/bin/dhampir(.exe)` | 出片 CLI |

**发布时一个环境变量都不用设** ✓。开发时把两个变量指到 Dhampir 源码树所在的形态，
**老路完整保留**（`VTEDIT_DHAMPIR_WASM` 指向仓根 → `<根>/web` + `<根>/crates/…`）。

### 8.4 三条自检：**明确报错，不许静默**

产物分支一旦命中就直接返回，不再回退源码树 —— 回退会把"**部署坏了**"伪装成
"**开发形态**"，用户看到的就变成白屏而不是一句人话。三条检查：

| 检查 | 失败时的行为 |
|---|---|
| `preview/engine.js` + `preview/pkg/dhampir_wasm.js` + `…_bg.wasm` **三件齐全** | 报错并**列出实际找过的路径**，告诉你去跑 `node scripts/package.mjs` |
| `VERSION` **存在** | 报错（"它是产物与 V-Trim 之间的契约版本，不能没有"）；存在时把 `project_schema=` 读出来 |
| 出片 CLI **在** | 报错（"设 `VTEDIT_DHAMPIR_CLI=`，或放到 `<程序目录>/dhampir/bin/`"）|

**"契约版本是否一致"的权威判定仍在 Dhampir 那一侧**（`project.rs` 里
`doc.project_schema != PROJECT_SCHEMA_VERSION` 就拒绝）—— V-Trim 只**报出读到的值**，
不在自己这边再抄一份期望值：同一条规矩不留第二处实现。

### 8.5 实测读数（2026-09-29）

```
产物齐全 ✓（exit=0）
CLI 独立可跑 ✓ —— 在**仓外**目录直接跑 `…\dhampir-artifacts\bin\dhampir.exe --help`
                   → "用法：dhampir <子命令> [选项]" ✓
预览包**零路径泄漏** ✓ —— 逐个文件搜 `Code\Dhampir`：engine.js / dhampir_wasm.js /
                          dhampir_wasm_bg.wasm / *.d.ts / package.json **都是 0 处**
VERSION:  project_schema=1  git=8e98abf  built_at=2026-09-29T17-09-43  platform=win32-x64
大小:     CLI 7.9 MB / wasm 1.1 MB / engine.js 48 KB
构建:     cargo --release 34.41s + wasm-pack 37.64s（增量时几秒）
```

**⇒ 产物的形态是对的**：CLI 能独立跑、预览包不含任何源码树路径、契约版本带在 `VERSION` 里。


## 9. wasm 预览的提速：逐项结论（每一行都有依据，或写明为什么不采纳）

### 9.1 落地并验证过的

| 项 | 做了什么 | 依据（读数） |
|---|---|---|
| **release wasm** | 服务端发的预览包改成 `wasm-pack --release`（此前一直是调试构建） | wasm = **1135594** 字节（`wasm-opt` 跑过）；与打包脚本产物同类 |
| **① 按窗口尺寸渲染** | 预览 canvas backing = CSS 尺寸 × DPR，**只缩不放**、**保持工程宽高比**；`?nofit=1` 切回满分辨率 | 驱动真页面实测 canvas：**1920×1080 → 649×365**（面积 8.7× 更少），两个片段都复现 |
| **①b 取帧按目标尺寸缩** | `createImageBitmap(video, { resizeWidth, resizeHeight })`，只在比视频小时给（绝不上采样） | 落地；**收益待工具条 `bitmap` 段确认**（`bitmap` 没变小就撤回） |
| **#4 贴纸 PNG 解码降频** | 原来**每个显示帧**都把同一张 PNG 解一遍；改成**每个 GIF 帧解一次**、画进常驻 canvas，每帧只做一次便宜的 `createImageBitmap(canvas)` | 构建通过；`解码` 涨得应比 `已交` 慢（工具条有这两个计数） |
| **⑤ 预渲染缓存** | GPU 纹理环形缓存（容量 = min(前后 N 秒, 内存硬上限 256MB，`VTEDIT_PREVIEW_CACHE_MB` 可调）；命中→整段求值/渲染/文字全跳过，只搬运一次 | 命中数直接出现在工具条的 **`命中 ? 未命中 ?`**（那是引擎本来就有的 shortCircuit 计数） |
| **pre-roll** | 底座加 `draw_impl(frame, present)`：与 `draw` 完全一样，**只是最后不 blit 到画布**；导出 `dhampir_project_preroll`；engine 加 `options.preroll`；Vue 在 **`实测 fps ≥ 50` 且本工程无贴纸**时往前算一帧 | 驱动真页面：`Uncaptured = 0`、不崩、工具条正常 |

### 9.2 ⑤ 与 pre-roll 里踩过、值得记的四个坑

1. **画布纹理不能当拷贝源/目的**：WebGPU canvas 通常只有 `RENDER_ATTACHMENT`。所以合成必须先落一张**自己拥有**的中间纹理（`RENDER_ATTACHMENT | COPY_SRC | COPY_DST | TEXTURE_BINDING`），再 blit 到画布。
2. **`TEXTURE_BINDING` 少一个就是"静默无效"**：`BlitRenderer` 把源纹理绑进 bind group，usage 少了它 WebGPU 只报
   `Uncaptured ... do not contain required usage flags TextureUsages(TEXTURE_BINDING)` —— **不抛异常、不崩、页面照常**，只是那趟搬运什么都不做。**这一条是用户贴日志才抓到的**（我的探针只测"崩不崩"，测不出这个）。
3. **底与文字的 target 必须一起换**：`render_frame_at`（底）与 `compose_overlay`（文字）都画进同一张纹理；只换一处 = 文字的混色基准不再是底 ⇒ 颜色/透明度错，而画面"看起来还挺正常"。
4. **失效判据不能用 `text_dirty`**：它是"用完即清"的，清掉之后再问"变过没有"永远得到"没变"。本项目的做法是在**两个入口**清整份缓存：`dhampir_project_open`（工程载入 = 换了一条历史）与 `invalidate_text_uploads`（字幕/弹幕/尺寸）。

### 9.3 不采纳的（写明理由）

| 项 | 为什么不做 |
|---|---|
| 每帧重复解析 `sourcesFor` | 那是个**两元素**的小 JSON（`{"source":…,"source_frame":…,"seconds":…}`），量级微秒，混在 13~18ms 里看不出来 |
| 关掉 `engine.timing` | 它**正是**工具条 `prep/draw/bitmap` 的来源 —— 关掉等于关掉唯一的仪器 |
| 跨帧流水线（"一帧在飞"改成两帧重叠） | 与底座设计冲突：一帧的顺序是 `sources_for → seek → clear_bitmaps → set_bitmap → draw`，**单条状态流水线**；两帧交错会让 `set_bitmap` 落进另一帧的 `clear_bitmaps` 之后 ⇒ 错帧，而且**看起来完全正常**。引擎里 `textToken`（"代"）机制存在的意义就是"同一时刻只有一趟 seek 合法" |
| 按层跳过"没变的层" | 本工程的层几乎全是随时间变的（视频逐帧推进、贴纸按素材时基换帧；文字位图**早已缓存**）。真正能省的只剩色块/效果类那一小撮，却要在渲染器核心加"层内容指纹" ⇒ 不成比例。**而且 ⑤ 命中时整帧都不重合成，比逐层判断更彻底** |
| 视频帧零拷贝（`copyExternalImageToTexture(<video>)`） | **结构性**：WebGPU 的那个联合类型里**没有 `HTMLVideoElement`**（实测 panic：`could not be converted to any of: ImageBitmap, HTMLImageElement, HTMLCanvasElement, OffscreenCanvas`）。`bitmap` 模式是浏览器里唯一通道 ⇒ **那台机器拿到真适配器，才是到 60 的唯一前提** |

### 9.4 复跑命令（改动后必跑）

```
node scripts/check-dual-end.mjs --frames 0,30,89     # 最差 SSIM 必须仍 **1.000000**
node scripts/run-guards.mjs                          # 18/20（红的两条是项目标注的"要真出片"）
node scripts/api-surface.mjs                         # 加了 wasm 导出后：--write + docs/host-api.md 名单
```


## 10. 提速续篇：读数驱动出来的四刀（**待用户那台裁决**）

### 10.1 用户的实测（`clip-25`，回退适配器 `subgroup_min_size = 4`）

```
#1833 / 60fps · 实测 **24fps** · prep **18ms** / draw **1ms** · seek 0 / bitmap **17** / set 1 / text 0
（命中 0 未命中 0）· bitmap · 贴纸 就绪 1/4 · 本帧 idx 1 · 解码 1 · 已交 1
```

**对照改前（早前读数）**：`prep 15~19ms（其中 bitmap 13~18ms）/ draw 8~14ms` ⇒ 结论两条：

| 项 | 判定 |
|---|---|
| **① 按窗口尺寸渲染** | ✅ **大胜**：`draw 8~14ms → 1ms`（合成面积少 8.7× 真的兑现了）|
| **①b 取帧缩放** | ✗ **无效并已撤回**：`bitmap` 仍 17ms，与改前同区间 ⇒ `resizeWidth/resizeHeight` **没有**省掉那次 1080p 回读（回读发生在缩放之前）|

### 10.2 于是问题变成"一帧的活只有 19ms，为什么 fps 还是 24"

- **`prep 18`** = prepare（含取视频帧 17ms）✓；**`draw 1`** = `onPrepared`（贴纸 fetch）+ 真正提交 ✓
  ⇒ 分段**没有漏项**（`t1` 在 prepare 之后、`onPrepared` 之前 —— 已逐行核实）
- 19ms ⇒ 本该 ~52fps ⇒ **差的 ~22ms 发生在 `engine.seek` 之外**

### 10.3 据此落的三刀 + 一个仪器（都已编译通过、`check-dual-end` 仍 1.000000）

| 落点 | 问题 | 改法 |
|---|---|---|
| `PolishPreview.vue` 模板 ref | `dhampirFrame` **既是模板依赖、又被每帧写** ⇒ **每帧整模板重渲染** ✗（模板含 canvas/工具条/贴纸/video/多个 `v-if`）| 拆开两个角色：新增**非响应式** `dhampirFrameShown` 专管"帧号没变就跳过"；模板那个 ref 挪进 4Hz 限频块 ✓（普查确认：这是模板依赖里**唯一**被每帧写的一个 ✓）|
| `PolishPreview.vue` 帧尾排帧 | 一帧画完又挂 `requestAnimationFrame` ⇒ **下一帧白等一个刷新周期（≤16.7ms）** ✗ | 帧尾改 `setTimeout(tick, 0)`（让出一次宏任务、**不等刷新**）；**tick 开头那处 rAF 必须保留**（它是循环唯一的启动者/守夜人 —— 摘掉会导致"某帧没画之后再起不来" ✗）|
| `PolishPreview.vue` 工具栏 | 分段解释不了 `seek` 之外的时间 | 新增 **`整圈`** 读数：从"决定画这一帧"到"这一帧画完"的墙钟，与 `seek/bitmap/set/text` 并列 ⇒ **`整圈 − seek` 就是"seek 之外"** ✓ |
| `web/engine.js` 取帧 | ①b 已撤回 | 回到 `createImageBitmap(video)` 不带参数 ✓ |

**自测（本机真适配器，驱动真页面）**：`整圈 281ms ≈ seek 280ms` ⇒ **seek 主导的那条路上，"seek 之外"只有 ~1ms** ✓
（⇒ 说明 **tick 自身的活不是 22ms 的来源** —— 我先前那条猜测被自己削弱 ✓；而那两刀花在**浏览器渲染阶段/等待**上，与这个 1ms 不矛盾 ✓）

### 10.4 裁决表（下一次读数照它走）

| `实测 fps` / `整圈` | 结论 | 动作 |
|---|---|---|
| fps 升到 ~40–50 ✓ | 两刀都对（**空档是真凶**）| 保留并收口 |
| fps 仍 24，`整圈 ≈ 20ms` ✗ | fps 被别处限制（rAF 起搏相位 / 浏览器帧预算）| 查起搏那条路径 |
| `整圈 ≈ 40ms` ✗ | 那 22ms 在 tick 的活里 | 按 `整圈 − seek` 差额逐项缩（贴纸 await / 段落表 / 镜像暂停）|
| 两者都没变 ✗ | 我的判断错 | **撤回那两刀**（写明理由）|


## 11. 「按窗口尺寸渲染」（①）：**病根找到了，默认已关**

### 11.1 现象与证据（同一帧、同窗口、只切这一个开关）

```
驱动真页面：同路由 #/p/2/polish/clip-25 ✓ / 同 seek(t=30s) ✓ / 同窗口 1600x1000 ✓ / 都停在预览页 ✓
  ① 开：canvas **649x365**    box 649x365    Uncaptured 0 ✓
  ① 关：canvas **1920x1080**  box 649x365    Uncaptured 0 ✓
两张画布 PNG 逐像素比：**差 61.82%** ✗（bbox = 全图）

并排图（out/cmp-fit-nofit.png）说明了一切 ✓：
  · **字幕层完全一致** ✓（"学生会什么团的…" 位置/字号/换行都一样 ✓）
  · **弹幕层完全一致** ✓
  · **视频层完全不同** ✗ —— ① 开时是**脸部特写（裁切/放大）**；
    ① 关时是**完整画面**（左侧直播间聊天栏 ✓ 右上角时钟 16:57:58 ✓ 日期 ✓ 右侧蝙蝠贴纸 ✓ 画框 ✓）
⇒ **⇒ ① 不是"同画面换分辨率"，而是"改变了取景"** ✗ —— 用户报的"比例什么的全有问题"就是它 ✓
```

**机制（读到的部分 + 未知的部分，分开写）**：文字的落点是**按目标尺寸重新排版**的（所以等比 ✓），
**视频层不是** ✗ —— 它更像按目标"铺满"而不是按顺序尺寸等比映射。
**⇒ 具体在 `RenderSpace`/层 `source_size` 哪一段，我还没读** ✗（不写猜测 ✓）；等要重开 ① 时先读那段。

### 11.2 处置：**默认关掉**（取景正确 > 帧率）

```
`PolishPreview.vue`：`forceFull` 的默认改成"满分辨率" ✓
  · 想再试：地址加 `?fit=1`，或运行时 `window.__dhampirFit = true` ✓
  · 旧的 `?nofit=1` 随之失效（默认已满分辨率 ✓）
代价：放弃 ① 的收益（用户那台 `draw` 会从 1ms 回到 ~8ms）✗
理由：目标的铁序是 **能力完整 > 能播放 > 性能**，而"取景错"属于第一类 ✗
```

### 11.3 我自己的一处过度推断（记下来防再犯）

```
第 87 轮我根据"backing == box、宽高比恒 16:9"下过结论："① 是无辜的" ✗
⇒ 那只证明了**画布几何**没问题 ✓ —— **取景/内容映射是另一件事** ✗✗
⇒ 教训：**"几何对"是必要条件，不是充分条件** ✓；
   要判"画面内容对不对"，必须**比内容**（并排图/逐像素 ✓），不能只看尺寸 ✓
```

### 11.4 自测工具（这几轮补出来的，以后这些判断我自己做 ✓）

```
out/drive-real-page.mjs 新增（都在 gitignore 的 scratch 里 ✓，产品代码未被污染 ✓）：
  · canvas / box / aspectBacking / aspectBox / object-fit / dpr   ← 几何判据 ✓
  · **png**（画布当前帧的 dataURL，直接存成 PNG 用眼睛看 ✓）
  · VT_SEEK=<秒>（**每次采样前把视频钉回该时刻并暂停** ✓ —— 否则 12 秒采样窗口里视频自己在走 ✗）
  · VT_WIN_W / VT_WIN_H（窗口尺寸 ✓，用来验证"跨尺寸" ✓）
  · VT_ORIGIN / VT_NOFIT（后者用运行时 `window.__dhampirFit` ✓ —— **不加 URL 查询串** ✓）
```


## 12. 「预览起不来」的真根因：**我在 async worker 上做了同步 FS**

### 12.1 用户的失败形态（有服务端日志为证）

```
点 Dhampir ⇒ POST /api/polish/dhampir-project → **200** ✓
⇒ 紧接着 `/api/polish/dhampir-wasm/engine.js` 的动态 import **失败** ✗
   控制台原文：`error loading dynamically imported module: …/engine.js`
   —— **没有 net:: 错误码** ✓（"连上了、拿不到"的典型形态）
⇒ 画布黑 ✗；同一时刻 `/api/jobs` **10 秒超时** ✗
⇒ **服务端日志里从来没有那条 engine.js 请求** ✗
```

**⚠️ 关键判读**：日志行是**响应写完之后**才打印的 ⇒
**"日志里没有该请求" ≠ "请求没发出"** ✓，而是**"请求被受理了，但响应迟迟产不出来"** ✓✓。
这一条曾经把我引向三个错误方向（浏览器缓存 ✗ / 扩展 ✗ / 媒体整文件读 ✗ —— 媒体端点本来就
是**流式 + 206 Range** ✓，见 `crates/vtrim-server/src/media.rs` 的 `serve_file`）。

### 12.2 根因（代码行级）

```
crates/vtrim-server/src/api/polish.rs  polish_dhampir_wasm（我加的那条转发路由）
  wasm_dirs()                    ← 内部是一串同步 `is_file()`
  file.canonicalize()            ← **同步 FS**
  root.canonicalize()            ← **同步 FS**
  real.is_file()                 ← **同步 FS**
⇒ 全部在 **async worker** 上执行 ✗

而这条纪律就写在本仓 `crates/vtrim-server/src/api/mod.rs:44-51`：
  「项目目录经常在网络盘上（RaiDrive/WebDAV）……一次 `exists()`、一次读 `vtrim.toml`
    就要几百毫秒到几秒……十几个并发就能把 runtime 的 worker 全占住 ——
    表现是**"任务在跑的时候整个服务器连新连接都建立不了"（用户实测）**。
    同步 IO 一律走 blocking 线程池，worker 留给 accept 与轻量接口。」
⇒ **同一个文件里，转译那条路（`run_blocking` ✓）本来就遵守了这条纪律，偏偏这条没有** ✗
```

### 12.3 修复

```
把 `wasm_dirs()` 与 `canonicalize()/is_file()` 全部挪进 `spawn_blocking` ✓
（`serve_file` 本来就是异步流式 ✓），**404 / 400 的语义原样保留** ✓（那两条有测试盯着）。
`cargo check -p vtrim-server` → exit=0 ✓；新二进制已在跑 ✓
```

### 12.4 顺带查出的第二个问题：**服务端一直在发旧产物**

```
`VTEDIT_DHAMPIR_PREVIEW` 指向第 44 轮打包的那份定版产物 ✗ ⇒
  实际发出的：engine.js **49636** / wasm **1130103**（第 44 轮那份 ✗）
  当前构建：  engine.js **51430** / wasm **1134603** ✓
⇒ **用户过去所有预览都跑在旧引擎 + 旧 wasm 上** ✗✓ —— 这本身就可能是一堆怪现象的来源。
（"不缓存、不复制"的设计是对的 ✓，但**产物目录得跟着重建** ✗ —— 这一条要写进部署步骤。）
```

### 12.5 `video` 直传（零拷贝源模式）的判定

| 层面 | 我这台（真适配器，Chrome 154）| 用户那台（回退适配器）| 结论 |
|---|---|---|---|
| 类型层：`copyExternalImageToTexture({source: <video>})` | **NO THROW** ✓ 无 uncaptured ✓ | **NO THROW** ✓ | 都能过 ✓ |
| 渲染层：画布出不出像素 | **纯黑 0.3%** ✓ 能画；且**合成与 bitmap 逐层一致** ✓ | **全黑** ✗ | **依适配器** ✓ |

**⇒ 判定：不可作默认** ✗；**可做"启用后先校验画布非零像素，通过才切 video，否则自动回 bitmap"的自适应开关** ✓。
（做这个之前，早先那句"WebGPU 不收 `<video>`"**是错的** ✗ —— 那次的 panic 来自更早的浏览器/版本；
但仍要记住：**类型过关 ≠ 内容可用** ✓✓，两者必须分别验证。）

**⚠️ 仪器的一处缺陷（会影响跨轮比对）**：我加的 `VT_SEEK` 只在采样循环**前**定位一次，
而驱动的采样窗口有 12 秒 —— 期间视频继续走，两次跑会差到 **10 秒**（并排图里视频自带的时钟
`16:57:48` vs `16:57:58` 就是铁证 ✓）。所以：
**"两次跑同一帧"这件事并未真正做到** ✗ ⇒ 一切跨跑的画布像素比对（包括那 30.58%）
**都作废** ✓。要做严格同帧比对，必须先让驱动**截图前一刻重新定位并暂停** ✓。


## 13. 续修：同一条纪律的三处落地（+ 一个仪器事实）

### 13.1 三处修复（都是"同步 FS 留在 async worker 上"，与 §12 同型）

| 路由 | 问题（代码行级）| 修法 | 验证 |
|---|---|---|---|
| `/api/polish/dhampir-wasm/*` | `wasm_dirs()` 里的 `is_file()` + 两次 `canonicalize()` + `is_file()` ✗ | 全挪进 `spawn_blocking` ✓ | 引擎三件 200 ✓（engine.js 51430 / wasm 1134603 ✓）|
| `/api/polish/sticker-file` | 两次 `canonicalize()` ✗（**播放一开始前端就连打贴纸 PNG** ⇒ 顺序上最先动手 ✗）| 同上 ✓ | 路由 404/正常响应 13.8ms ✓（不挂 ✓）|
| `/api/polish/font` | **`std::fs::read` 一个 19.7 MB 字体** ✗（字幕首次用到字体就打它 ✗）| 读文件挪进 `spawn_blocking` ✓（`c.clone()` 保住借用 ✓）| `?name=msyh` → **200 / 19704352 B / 44.2ms** ✓ |

三处 `cargo check -p vtrim-server` 均 **exit=0** ✓；404/400/200 语义原样保留 ✓（那两条有测试盯着）。

### 13.2 精确清单（后续按需清理；判据：**只看 async 函数体内、且排除 `#[cfg(test)]` 段**）

```
`crates/vtrim-server/src/api/*.rs`：async 处理器体内的同步 FS 共 **64 处**，
其中上下 25 行内找不到 `run_blocking/spawn_blocking/tokio::fs` 护栏的 **45 处** ✗。
**在预览页热路径上**的（优先）：`polish_project_get_impl`（`video_path.exists()`）、
`polish_subtitles_get_impl`（`srt_path.exists()` + `read_to_string`）、
`project_data.rs` 切片列表循环里的 `video_path.exists()` + `metadata`（**每个切片一次 stat** ✗）。
其余（`clip_delete / event_cut(_all) / fs_list / create_local_project / projects_source_delete /
polish_cover_* / polish_preview`）不在预览热路径上，按需清理 ✓。

⚠️ 教训：第一版扫描**没有区分"处理器函数体"与"测试代码"** ⇒ 报出 336 处 ✗，
**那不是结论**（本项目长期大量用同步 FS，服务平时是好的 ✓）。判据要精确才可用 ✓。
```

### 13.3 仪器事实：我一直只跑在**"暂停 + 每帧真 seek"**那条路上

```
同一个驱动，两种条件，读数完全不同：

  视频**暂停**（第 5~14 轮的默认注入）：
      实测 **1fps** · prep 277ms · **整圈 277 / seek 276** · bitmap 0
      ⇒ 每帧都真 seek ⇒ **这条路上永远看不到"跟随播放"的表现** ✗

  视频**真在播**（本轮加的 `VT_PLAY=1`，每秒抢回播放 ✓）：
      实测 **60fps** · **整圈 0 / seek 0 / bitmap 0 / set 0 / text 0** ✓✓
      ⇒ **跟随播放这条路在我这台真适配器上是 60fps 跑满的** ✓（与代码注释
        「浏览器腿 1080p 顺序播放 7.0 ms/帧（142.9 fps）」一致 ✓）
      ⇒ `整圈 0` + `text/set 0` 说明该帧被"**帧号没变就跳过**"接住了 ✓（第 71 轮那刀 ✓）

⇒ **⇒ 这解释了为什么我在 §12 那几轮里怎么都复现不出用户的现象** ✓：
   我的仪器一直把被测对象**钉在"每帧 seek"那个工作点上** ✗ —— 而用户的失败发生在
   **"跟随播放"**那个工作点上 ✓✓。
⇒ 以后要紧跟用户的现象，**必须用 `VT_PLAY=1`（真播放）** ✓，不能用默认的暂停注入 ✓。
```

### 13.4 三处修复 + 真播放条件下的回归（本轮实测）

```
真播放注入 playing ✓ · dhampir-wasm 已加载 ✓ · engine.js 报错 **0** ✓ · 404 **0** ✓
Uncaptured 0 / is invalid 0 ✓ · 画布 1920x1080 **纯黑 0.3%** ✓ · `check-dual-end` 最差 **SSIM 1.000000** ✓
⇒ **三处服务端改动没有破坏任何东西** ✓，且"跟随播放"这条路健康 ✓
⇒ 用户那台的 24fps（`bitmap 13~15ms` ✗）**只在适配器那一层**，与代码无关 ✓（见 §6）


### 13.5 第五批：批量清理的**过滤器**（这是这轮最值钱的经验）

```
目标：把 async 处理器体内的同步 FS 挪进 blocking 池。做了**三次**批量尝试：

  第 1 次（无过滤）      替换 25 处 ⇒ **13 个 `error[E0728]: await is only allowed inside async fn`** ✗
                        （同样的文本也出现在**同步辅助函数**里，如 `fn transcript_starts` ✗）
  第 2 次（只限 async 体内）替换 12 处 ⇒ 剩 **2 个错误** ✗
                        （那两处在 `run_blocking("…", move || { … })` 的**闭包体内** ——
                          那里**本来就是 blocking 池** ✓，再加 `.await` 反而非法 ✗）
  第 3 次（async 体内 **且** 不在已 blocking 的闭包内）替换 **5 处** ⇒ **cargo check exit=0** ✓✓

⇒ **过滤器 = 「最近的 fn 头是 async」且「从该 fn 头到此处没有 `run_blocking(` / `blocking(move` /
   `spawn_blocking(` / `block_on_here(`」** ✓
⇒ 前两次都**自动回滚**（脚本在编译失败时反向替换 ✓），`cargo check` 回到 exit=0 ✓，树始终干净 ✓。
```

**累计的五批（覆盖用户预览页的完整请求链）**：

```
① `/api/polish/dhampir-wasm/*`    引擎三件（engine.js / pkg/*.js / *.wasm）
② `/api/polish/sticker-file`      贴纸 PNG（**播放一开始前端就连打** ⇒ 顺序上最先动手）
③ `/api/polish/font`              **19.7 MB 整文件读**（实测 200 / 19704352 B / 44.2ms ✓）
④ **8 处** `path.join("polish.toml").exists()`（页面加载热路径）
⑤ **5 处** `read_to_string` / `exists()`（async 体内、且不在已 blocking 的闭包里）
⇒ 五次 `cargo check -p vtrim-server` **全部 exit=0** ✓；404/400/200 语义原样保留 ✓
```

### 13.6 媒体端点（目标 A/C 项）：**实测结案，不用改** ✓

```
`/api/media/2/output/clips/clip-25/clip.mp4`（**341,724,933 B = 326 MB** ✓ 就是用户的片段）

  无 Range             → **200** ✓ video/mp4 ✓ **Accept-Ranges: bytes** ✓  19.6ms
  Range: 0-1023        → **206** ✓ **Content-Range: bytes 0-1023/341724933** ✓  14.8ms
  Range: 1000000-      → **206** ✓ Content-Range: bytes 1000000-341724932/341724933 ✓  **0.9ms**
  Range: 99999999999-  → **416** ✓ Content-Range: bytes */341724933 ✓

⇒ 流式 + 正确 Range 全部到位 ✓ ⇒ **播放器按需取片段，不会整文件读** ✓
⇒ **"媒体整文件读"这条怀疑被实测否掉** ✗ —— 黑屏不是媒体这条路造成的 ✓
   （`serve_file` 本来就是 `Body::from_stream` ✓，见 `crates/vtrim-server/src/media.rs`）
```

### 13.7 建议的下一步：`video` 直传的**自适应开关**（未实现，设计留档）

```
现状：源模式默认 `bitmap` ✓（§9），因为 `video` 在**回退适配器上全黑** ✗。
目标：让真适配器的机器白拿 13~15ms 的零拷贝，同时不给回退适配器留黑屏。

设计（一个开关 + 一次校验，不做内容质量 proxy ✓）：
  1. 默认仍 `bitmap` ✓（保守）；
  2. 显式开启后（例如 `?src=video` 或 `window.__dhampirSrc='video'`）：
     首帧画完后**读一次画布像素**（`canvas.toDataURL()` ✓ —— 已实测 WebGPU canvas 支持 ✓，
     §13.3 的驱动就是这么取帧的 ✓）；
  3. 若**全黑**（或近全黑）⇒ **立刻回退 `bitmap` 并重画当前帧** ✓，同时把原因报出来 ✓；
  4. 校验通过才留在 `video` ✓。
⇒ 判据依据：我这台真适配器上 `video` 模式**纯黑 0.3%** ✓ 且**合成与 bitmap 逐层一致** ✓（§12.5）；
   用户那台回退适配器上**全黑** ✗ ⇒ 校验会把它挡回 bitmap ✓。
```

### 13.8 与用户那一侧的边界（如实）

```
✅ 我这边已能自证：五批修复后、**真播放条件**下 —— `dhampir-wasm 已加载` ✓ / `engine.js` 报错 **0** ✓ /
   404 **0** ✓ / Uncaptured **0** ✓ / 画布纯黑 **0.3%** ✓ / **实测 60fps** ✓ / `check-dual-end` **1.000000** ✓
🔧 仍未完成：**用户那台点一次 Dhampir 的读数** —— 这是目标完成判据里唯一还空着的一格 ✓
   （我这边两种播放条件都试过、整条请求链都清了，**无法再复现出故障** ✗ ⇒ 只能等那一格 ✓）

---

## 14. 贴纸与一切"有时段"的层的时间语义：入点/出点

> 本节是**规范条文**（解释与折算），不是新功能：不引入新字段、不改任何序列化格式。
> 贴纸 / 字幕 / 弹幕 / overlay / 音效 / 运镜 —— 凡是"有一段持续时间"的层，一律按下面这六条折算。

### 14.1 定义（逐条）

1. **入点** = 该层**第一个被显示的帧**（含）。
2. **出点** = 该层**最后一个被显示的帧**（含）。
   两个都是**闭**的：入点那一帧要画，出点那一帧也要画。
3. **时基一律工程帧号**。
   - 不用源帧、不用秒 —— 秒只出现在 `polish.toml` / `vtrim.toml` 的输入侧，以及给人看的报告里。
   - 秒 → 帧：`floor(秒 × 工程fps)`。
     用 `floor` 而不是四舍五入：`2.37s @ 24fps = 56.88` → **56**，取的是"这一秒里已经走完的整帧"。
   - `工程fps` 是**工程自己的**帧率，不是素材帧率（见第 5 条）。
4. **存储/比较一律半开区间 `[in, out+1)`**，判定式：

   ```
   in <= f < end          // end = out + 1
   ```

   - 存 `out + 1`（而不是存 `out`、到每个使用点再 `+1`）是为了让**区间运算**退化成整数比较：
     层的拼接、求交、求差都不会再出现"到底含不含端点"的分歧。
   - 给人看时打印**闭**区间 `[in, out]`；**落盘与比较只用** `[in, out+1)`。
5. **源帧 ↔ 工程帧必须显式走比值**：

   ```
   工程帧 = 源帧 × 工程fps / 素材fps          // 边界上算一次，之后全在工程帧域里算
   ```

   - **不许把源帧直接当工程帧用、也不许两者相乘**。工程 60fps 与素材 `100/3` fps（29.97）
     **不是一回事**：直接按数值使用会差 `60 / (100/3) = 1.8` 倍 —— 这是**量级错误**，不是舍入误差。
6. **`out < in` 视为不可见**：该层**不出现在任何一帧上**。
   - **不纠正**（不把 `out` 抬到 `in`）、**不夹一帧**（不硬给一帧的时长）。
   - `[in, in)` 是空区间 —— 半开区间下它天然什么都不含，`out < in` 只是它的显式写法。
   - 空层是**合法输入**（老工程、被裁掉的段、`duration = 0`）：报一条 NOTE 即可，不要报错。

### 14.2 贴纸窗口就是这套定义的一次代入

```
四段相位（**都在窗口内部**）：popIn 0.35 → floatUp 0.4 → hold → fadeOut 0.3
                             0      0.35        0.35   0.75      0.75  dur-0.3   dur-0.3  dur
总窗口  = dur                       // **不溢出**到事件之外 ✓（用户实测口径：出点 = 最后一个显示帧）
淡出起点 = dur - fadeOut = dur - 0.3 // 于是最后一帧正好落在出点

in   = floor(et × 工程fps)
存    : [in, in + round(dur × 工程fps))     // 半开区间
给人看: [in, in + round(dur × 工程fps) - 1]  // 闭区间出点
```

`dur` 装不下 `0.35 + 0.4 + 0.3` 时**优先保淡出**：前两段按可用时长等比压缩、淡出起点不得早于 0；
`dur <= 0.3` 退化为"全程淡出"。任何时长都不出现负段长 / NaN / 最后一帧不透明。

### 14.3 本次结论：淡出**移进窗口内部**、窗口**以事件时长为准** ✓

两轮都记在这里（同一个判据，一次改短、一次改长）：

| 轮次 | 原文 | 修法 | 状态 |
|---|---|---|---|
| 一 | "窗口只到 `dur + 0.65`、淡出要 `dur + 0.75` ⇒ 贴纸硬切" | `dur + 0.65` → `dur + (0.35 + 0.4 + 0.3)` = `dur + 1.05` | ✓（**已被下一轮取代**） |
| 二 | "窗口 = `dur + 1.05` ⇒ 贴纸比事件多显示 0.4 秒，出点晚了 ✗（用户实测）" | 窗口回到 **`dur`**，淡出起点从 `popIn + floatUp + dur` 改成 **`dur - fadeOut`** | ✓ 已修 |

五处**同一条口径**（窗口上界 = `dur`，淡出起点 = `dur - fadeOut`）：

| 位置 | 形状 |
|---|---|
| 产生端 `tools/polish-to-dhampir.mjs` | `STICKER_WINDOW_SEC = 0` + `stickerWindowFrames(durSec, fps)`；`stickerStateAt` 的 `fadeStart = durSec - fadeOut` |
| 参照守卫 `crates/vtrim-polish/templates/index.html` 的 `getActiveSticker()` | `if (t < et \|\| t > et + dur) continue;` + `fadeStart = dur - fadeOut` |
| CPU 参考 `crates/vtrim-render-core/src/timeline.rs`（`get_active_sticker`） | `if t < et \|\| t > et + dur {` + `fade_start = dur - fade_out` |
| WebUI 预览 `webui/src/polish/engine.js`（`stickersAt`） | `if (t < et \|\| t > et + dur) continue` + `fadeStart = dur - fadeOut` |
| DOM/GSAP 预览腿 `handlers.sticker`（`index.html:955`） | `fadeOut = min(0.3, dur)` + `fadeStart = et + dur - fadeOut` + **线性**淡出（`ease:'none'`）+ 隐藏在 `et + dur`（**上浮起点 `et + 0.25` 不动** —— 这条腿自己的既有偏差） |

由守卫钉住：`scripts/check-vtrim-translator.mjs` 第 5 节按**逐值**采样四段相位，并断言
"`dur = 1.0s` / `fps = 60` 时**最后一帧**（第 59 帧，`t = 59/60`）opacity ≤ 0.1、第 60 帧（`t = 1.0`）已不可见"；
第 6 节**读模板源码**算出窗口值（必须是 `dur`，**自证**：换成 `dur + 1.05` 必翻面）；
第 6b 节把四处源码一起钉（不许再出现 `dur` 之后追加时长，且四处的淡出起点都必须是 `dur - fadeOut`）；
第 6c 节按源码钉 **DOM/GSAP 预览腿**（`handlers.sticker` 的函数体必须**能编译**、不许再出现 `0.35 + dur`、
必须有 `dur - fadeOut`、隐藏于 `et + dur`、淡出长度必须是 `min(0.3, dur)`；后三条各带自证）。
（真变异自证做过一次：把该腿改回 `et+0.35+dur` / `fadeStart+0.3` ⇒ 守卫 exit 1 并报出该段源码。）

### 14.4 明确**不动**的东西

- `polish.toml` / `vtrim.toml` 的**字段名**、**结构**、**注释化序列化**（`to_toml_commented` 的输出）
  **一律不动**。本节只是解释与折算：秒还是秒、字段还是那些字段，帧语义只在读取/比较侧解释。
- 不新增字段、不改键名、不改注释文本 ⇒ 已有工程文件**照旧逐字节可读**，模板不回写。
