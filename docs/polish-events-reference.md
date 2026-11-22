# 精修事件参考（种类 / 参数 / 编辑器覆盖范围）

> 依据代码：事件定义 `crates/yekitrim-render-core/src/types.rs`（时间统一为 **`time`(开始) + `end`(结束)**，
> 老工程的 `duration` 由 `crates/yekitrim-render-core/src/legacy.rs` 在反序列化前换算，见 §2.6）；
> LLM 出参 `crates/yekitrim-polish/src/llm/{segment,events}.rs`；后处理
> `crates/yekitrim-polish/src/process/{mod,fx,sfx,download}.rs`；预览求值 `webui/src/polish/engine.js`；
> 出片求值 `crates/yekitrim-render-core/src/timeline.rs`（Chrome/GSAP 路径见 `crates/yekitrim-polish/templates/index.html`）。

## 0. 一个事件的一生（四层）

| 层 | 位置 | 干什么 |
|---|---|---|
| 定义 | `render-core/types.rs` 的 `Event`（serde tagged enum，`type` 字段） | 15 个变体 + 结构类 `Segment`/`Subtitle`/`MuteRange` |
| 生成 | `polish/llm/segment.rs` Step1（分段）+ Step2（逐段生成事件） | LLM 按 prompt 写 `type/time/end/...` |
| 后处理 | `process/mod.rs`（剥静音区间、贴纸落点/跨度、预算、音效规则、片头 fade_in）`process/fx.rs`（缺 `end` 补默认 + 瞬时跨度 clamp）`process/download.rs`（资产） | 把 LLM 的粗事件补成可出片的事件；写回 `polish.toml` |
| 消费 | 预览 `engine.js`（DOM 自绘）／出片 `timeline.rs`（wgpu 原生、wasm）／`templates/index.html`（Chrome+GSAP） | 逐帧求值成画面与声音 |

`polish.toml` 里一段视频的全部事件都挂在 `[[segments]]` + `[[segments.events]]` 下（`segments[].events[]`）。

---

## 1. 总表

| type | 类别 | 主要参数 | `end - time` | 编辑器可改 |
|---|---|---|---|---|
| `camera` | 持续 · 运镜 | `zoom`(远/中/近)、`anchor`/`anchor_to`、`origin`/`to_origin`、`cut`、`handheld` | 事件自带（秒） | 时间/起止/类型/zoom/anchor/anchor_to/cut/handheld/配音 |
| `sticker` | 持续 · 贴纸 | `keyword`、`sticker_tag`、`name`、`source`、`x`、`y`、`sfx`、`sfx_volume` | 1.5~2.5s（prompt 建议；process 可延长到后一条字幕结束，≤ +50%） | 时间/起止/keyword→贴纸两级列表/配音 |
| `element` | 持续 · 元素（文字/贴纸） | `kind`(text/sticker)、`text`、`style`(hook/chapter/comment/count)、`pos`(top/center/bottom)、`sticker_tag`、`x`、`y`、`manual` | 1.2~5s（**process 强制**，见 §2.6） | 时间/起止/kind/正文或关键词/样式/落点档位 |
| `overlay` | 持续 · 覆盖层 | `overlay_type`(color/gradient/fade_in/fade_out)、`color`、`opacity`、`colors[]`、`shape`(linear/radial)、`angle`(线性档方向；CSS 约定 0=向上，缺省=右下 135°) | 0.5~5s（prompt 建议；fade 两档整块时长＝渐变时长） | 时间/起止/类型/颜色/不透明度/配音（渐变档另有 colors/形状/方向） |
| `sfx` | 点 · 音效 | `name`(音效 id)、`volume`、`end` | 缺省 = 文件原始长度 | 时间/起止/音效列表/音量 |
| `shake` | 瞬时 · 视觉 | `intensity`(默认 14) | cap 0.5 / 默认 0.3 | 时间/起止/intensity/配音 |
| `flash` | 瞬时 · 视觉 | `color`(默认 `#ffffff`) | cap 0.3 / 默认 0.25 | 时间/起止/color/配音 |
| `blur` | 瞬时 · 视觉 | —（固定 8px） | cap 0.4 / 默认 0.3 | 时间/起止/配音 |
| `vignette` | 瞬时 · 视觉 | —（四角暗角） | cap 0.6 / 默认 0.4 | 时间/起止/配音 |
| `hue_shift` | 瞬时 · 视觉 | `color`(默认 `#ff0040`) | cap 0.5 / 默认 0.35 | 时间/起止/color/配音 |
| `noise` | 瞬时 · 视觉 | —（噪点 opacity 0.08） | cap 0.4 / 默认 0.3 | 时间/起止/配音 |
| `pulse` | 瞬时 · 视觉 | —（zoom ×1.06 → ×0.97） | cap 0.5 / 默认 0.4 | 时间/起止/配音 |

「配音」= 所有视觉事件都可挂 `sfx` 引用 + `sfx_volume`（`sfx` 事件本身用 `name`/`volume`）。
所有类型的公共块是 **开始 / 结束 / 入点 / 出点 / 定位**（删除走双击块）；改完自动保存并自动跑一次后处理。

结构类（不是 event，但在同一条时间轴上）：

| 结构 | 字段 | 编辑器 |
|---|---|---|
| `[[segments]]` | `start`/`end`/`fx`(low\|med\|high)/`note`/`events[]` | 拖 / 右缘缩放 / 面板改备注与情绪 / 面板确认后删除；**不能新增分段，也没有左缘缩放**（缩 start 会把事件留在段外） |
| `subtitles`（`data/subtitles.json`） | `start`/`end`/`text`/`html` | **拖 / 左右缘缩放** / 面板改内容 / 拆分 / 设入出点 / 删除 / ＋新增（与事件同一套控件） |
| `mute_ranges` + `silence_ranges` | `start`/`end` | 添加 / **拖 / 左右缘缩放** / 拆分 / 删除；「去静音」把检测结果**并入** `mute_ranges`（与现有屏蔽相加，重叠/相邻合成一段），只有自动加进去的那部分记进 `silence_ranges`（撤销按它相减回退） |

另外非事件的内建表现：字幕/弹幕/贴纸图层本身 —— 原来的内建片头黑场（`intro-black`）**已退役**，片头渐入改由 `process` 在**初次**后处理补的 `overlay` 事件承担（见 §2.3）。

---

## 2. 逐类说明

### 2.1 运镜 `camera`（持续）

```toml
[[segments.events]]
type = "camera"
time = 12.5
end = 16.5
zoom = "中"          # "远"(画中画) | "中" | "近"
anchor = "liver"     # **起点**：块开始时画面中心在哪 —— liver | danmaku | sticker | center（可省）
anchor_to = "danmaku"  # **终点**：块结束时画面中心在哪（同样的取值）；写了不同的值 = 块内摇过去
origin = "83.33% 50%"  # 由 anchor 解析回填；也可直接手写坐标
to_origin = "16.67% 16.67%"  # 由 anchor_to 在**块尾时刻**解析回填；与 origin 相同会被清掉
cut = true           # 可省；**运镜过渡 = 硬切**：本块的起点瞬间到位（块内位移照常）
handheld = true      # 可省；手持摇摆（附加变种）—— 只在 zoom = "近" 上有效
```

- **两种写法，别混**：写 `anchor`/`anchor_to` 是「意图」，`process`（Step 2d）解析成 `origin`/`to_origin`，
  与贴纸的 `keyword`→`name` 同一套约定；直接写 `origin` 就是字面坐标，process 不动它。
- 锚点取值：`liver`（`layout.liver` 的中心）、`danmaku`（**那一刻真的在飞的弹幕**轨道带中心，
  没有弹幕时退回 `layout.danmaku_area`）、`sticker`（时间上最近的贴纸盒子中心）、`center`（画面正中）。
  认不出的名字只打日志、保留原 `origin`，不让整条管线失败。
- `zoom` 只是**档位名**，实际倍率来自 `polish.toml [style.camera]`：
  `zoom_mid = 1.3`（中）、`zoom_close = 1.5`（近）、`zoom_wide = 0.55`（远，小于 1 = 画面缩小）。
  **四条路径共用 `CameraStyle::factor` 这一张表**（模板 canvas = 出片权威 / 模板 GSAP / `engine.js`，
  外加 wgpu 的 `timeline.rs` —— 那条未启用，见 `crates/yekitrim-render-core/AGENTS.md`（wgpu 真渲染））；
  这里曾经只有模板 canvas 硬编码 1.5/1.8，导致配置对成片无效、预览与成片不是一回事。
  **放大档位不会「推出框」**：运镜是绕不动点做 `scale`，`s ≥ 1` 时画面四周一律向外扩张
  （`overflow:hidden` 裁掉多出来的部分），任何倍率都盖满整个画面，不可能露出背景色/背景层；
  只有画中画（`zoom_wide < 1`）才是刻意留黑边。所以调大这两档只有观感问题、没有露底风险。
  老工程里写死的旧内建默认由 `cmd_process` Step 0a 迁移成新值（中 `1.15 → 1.3`、近 `1.3 → 1.5`、
  远 `0.7 → 0.55`；**只认这几个旧默认值**，手改过的一律保留，日志会报条数）。
- **位置状态机（2026-11-21 改）**：`anchor` / `anchor_to` 是**同一个概念的两个端点**（面板文案 =「起点 / 终点」）。
  一条运镜 = 一个时间块，块里的位置分两段走：
  ① **接缝「运镜过渡」** `[time, time+td]`：把镜头从**上一块落下的位置**接到本块的 `origin`；
     `td` 由 `cut`（硬切 ⇒ 0）与 `speed` 档位决定，缺省是老式子 `min((end-time)*0.5, 0.6)`。
  ② **块内位移** `[time+td, end]`：本块的 `origin` → `to_origin`，同一条 `pow2Out` 曲线。
  `anchor_to` 不写、或与 `anchor` 相同 ⇒ `process` 把 `to_origin` 清掉，这一块**整块静止**
  （用户口径：「开始和终止位置相同就默认不操作终止位置」）。
  「主播→弹幕→回主播」= 两条事件：第一条 `anchor="liver", anchor_to="danmaku"`（块内摇过去），
  第二条 `anchor="liver"`（接缝里 0.6s 自己推回来）。
  ⚠️ 位置是**连续状态**（与 `zoom` / `handheld` / `tilt` 同形）。它曾经是每块**绝对赋值**，
  `p=0` 时直接等于本块起点 ⇒ 上一块落在弹幕、下一块写「起点=主播」就是**一帧横移 928px**；
  同一个根因还让「硬切 + 终点」把起点整个吞掉。
- **画中画**（`zoom = "远"`）：画面以不动点为中心缩到 0.55，四周留黑边（各 432px @1920）
  （canvas 在 `clearRect` 后透明、导出 JPEG 里是黑；`.stage` 背景本就是 `#000`；wgpu 的 source pass 清屏为黑）。
  ⚠ 判据是 `|zoom-1| > 0.001` 而不是 `zoom > 1`——写成后者会把整档吞掉。
- **画中画到点收工**（用户要求：「镜头事件需要拉长无缝连接…本来是画中画的戏剧效果，被延长后
  感觉很有诡异」）：运镜是**状态机**，事件只写起点、档位一直保持到下一条运镜——别的档位的
  别的景别的跨度只是"多久推到位"，**画中画的 `end - time` 却是"它该待多久"**。不补就会一直挂着
  （实测 clip-35 一条 2.1s 的画中画挂成了 16.8s：下一条运镜在 16 秒外）。
  `fx::close_pip_shots` 在画中画自己的终点补一条**回镜头**的运镜事件：档位与不动点取
  **时间上**前一条非画中画运镜的**落点**（写了 `anchor_to`/`to_origin` 就是终点那一侧；
  没有前一条就照默认运镜聚焦主播），一直持到下一条运镜。补的是真事件（不是渲染侧特例），
  四条求值路径 / 时间轴编辑器 / 贴纸避让看到的完全一致；回镜头是硬切（「远」进出本就硬切）。
  下一条运镜紧接着（`< MIN_RETURN_SECS = 0.5s`）就让它接手，不补；幂等，重跑不会再补。
  - **档位取时间序而不是文件序**：段之间时间上可以重叠（LLM 按窗口生成、还可能有
    `split_segments_by_mute_ranges` 切出来的段），求值器（模板 `buildIndex` / `engine.js`）
    都是按时间排序后再走的，所以"前一条镜头"必须按时间算。
  - 想让它别收工（罕见）：在画中画终点后再补一条「远」—— 但那会被配额守卫降级成「中」，
    真要长画中画得手改 `polish.toml` 并把 `cap_pip_shots` 的配额算上。
- **画中画配额：一个切片只有一条**（用户要求）：`process/fx.rs::cap_pip_shots` 按时间序只保留
  **最早**的一条「远」，其余降级成「中」（只改档位，`anchor`/`origin` 原样留着，日志会报）。
  `process` 与 LLM 落盘后各跑一次；prompt 里也写了同样规则，但 LLM 是按段窗口生成的、
  数不出"整片几次"，所以守卫才是真正的兜底。
- **画中画对齐字幕边界**（用户要求："不能没头没尾的断开"）：`align_pip_shots` 把起点吸到包含它的
  那条字幕的**开始**、终点吸到那条字幕的**结束**（落在空隙里就取相邻字幕的边界），
  结果夹在所属分段内；对齐后不足 `MIN_PIP_SECS = 1.5s` 就往后借下一条字幕的结束。
  只动 `time`/`end`，档位/锚点不碰。
- **贴纸也对齐最近的字幕**（用户要求：「表情贴纸也需要对齐最近的字幕」）：`align_stickers_to_subtitles`
  把起点吸到**最近那条字幕**的开始、终点吸到它的结束（离最近字幕超过 `STICKER_SNAP_MAX_SECS = 2.0s`
  的保持原位 —— 纯视觉反应没有台词可依附）。排在 `process` **Step 2b（落点）之前**，
  落点按贴纸时间窗口算障碍，时间必须先定下来；2c 的尾帧自适应还能在此基础上往后接一句。
  `end - time` 是"停留时长"，入场/上浮/淡出（0.35+0.4+0.3s）另外算，所以对齐到一条 1s 的字幕也没问题。
- **远景进出是硬切，不是缓动**（用户要求）：只要过渡的一端是"远"，
  `zoom` 与不动点都在事件起点**一次性到位**，这一段跨度不产生任何缓动，位移也不生效。
  理由：缓动缩到画中画看着像「镜头在拉远」，而它要的是喜剧式的切换——切才有那个意思。
- **硬切 `cut = true`（切镜）**：把上面那条规则**放开给任意景别** —— 这一条整档瞬间到位，
  不从上一个落点推过去。想在切完之后再平滑推移，就再写一条普通运镜（两条事件）。
  三处求值器都读了它（模板 canvas / 模板 GSAP / `engine.js`），贴纸避让侧走
  `placement::transition_secs(label, prev, cut)` → `transition = 0`，
  否则会按 0.6s 过渡凭空多算一个早已结束的旧状态当障碍。
- **手持摇摆 `handheld = true`（附加变种，用户要求）**：把相机换成「手持」——低频位移
  （横 ≈±9.5px / 纵 ≈±7.4px @1920 基准，主次两路正弦不成整数比，避免钟摆感）+ 轻微倾斜
  （≈±0.48°，绕**聚焦中心**）+ 呼吸缩放（≈±0.7%），**叠加在当前景别之上**。
  参数表是唯一真源：`render-core::types::handheld_sway`（两个 JS 副本
  `templates/index.html::handheldSway` / `engine.js::handheldSway` 由
  `webui/scripts/check-handheld.mjs` 对拍，Rust↔JS 由 `timeline.rs` 的采样测试钉住）。
  波形是**纯 `t` 的函数**：跳帧 / seek / 并行求值结果一致。
  - 与 `zoom` 同一套**状态机**：它也是持续状态 —— 从这条运镜起生效、一直跟到下一条运镜
    （交回稳镜头就再写一条不带它的运镜）；幅度起停复用该事件的过渡窗口，`cut` 即瞬变。
    与 `zoom` 一样**分量在段间按时间序求值**（不是文件序）。
  - ⚠ **只有近景（`zoom = "近"`）可用**：可选能力、**不自动补**。`process` 的
    `fx::cap_handheld_sway` 把非近景上的 `handheld` 直接剔除（只剥不补、幂等；
    排在画中画收工**之后**，回镜头承接的值一起吃守卫，日志报条数）。
    LLM prompt 只写「近景专用」的口径，真正的兜底是这道守卫。
  - 编辑器：运镜面板「手持摇摆」开关（**非近景置灰**）、块标签前缀「摇·」；
    景别改离近景时面板会顺带剥掉这个字段（与守卫同语义，避免悬空开关）。
  - 参数固定：不开放 `[style.camera]` 旋钮，也没有事件级强弱。
- 过渡：`min((end-time)*0.5, 0.6)` 秒，`pow2Out` 曲线；**竖屏（`vertical = true`）完全不运镜**，
  手持摇摆跟着整条运镜一起屏蔽（幅度归零，四条求值路径都是）。
- 分段被屏蔽区间切开、或截断到视频长度时，运镜的 `end` 会被夹到段尾（`clamp_camera_end`）——
  不夹的话位移会移到段外。
- 编辑器：时间轴「运镜」泳道可拖 / 左右缘缩放；属性面板可改 `type/time/end/zoom/anchor/anchor_to/cut/handheld`；
  **`[style.camera]` 的三个倍率仍然只能手改 `polish.toml`**。
  想让运镜对准人物 → 右栏「区域编辑 → 人物」框选（写 `layout.liver.top_left/bottom_right`），
  锚点或默认运镜都会读它。
- `process` 会给每个 ≥1s 且没有运镜事件的段**补一条** `zoom = "中", anchor = "liver"`（整段）。
  这就是「运镜全程开着」的原因——不是每个情绪点推一下，而是一直停在某个档位上。
- **老工程迁移**：`origin = "50% 50%"` 且没有 `anchor`/`to_origin` 的事件，会被补成
  `anchor = "liver"`（日志会报条数）。依据是这个值从来不是用户能设的——它是 LLM prompt
  示例与两个默认生成器写死的常量，一直被当作「没指定」。不迁移的话老切片重跑仍然对着画面正中推。

### 2.6 元素 `element`（持续）

**元素 = 统一的视觉元素**（总方案 §5），一支承载两种 `kind`：

```toml
type = "element"; time = 3.0; end = 5.0; kind = "text"
text = "这就是传说中的…"   # kind=text 时是正文；kind=sticker 时是关键词（同 sticker.keyword）
style = "hook"            # hook | chapter | comment | count（kind=text 用）
pos = "top"               # top | center | bottom（kind=text 用）

type = "element"; time = 10.0; end = 12.0; kind = "sticker"
text = "ysyk"             # 关键词（贴纸包名）
sticker_tag = "害怕"
x = "auto"; y = "auto"    # 与 sticker 同口径（process 计算）
```

| `style` | 用途 |
|---|---|
| `hook` | 钩子标题（大号高对比强描边） |
| `chapter` | 章节卡（中号带底居中） |
| `comment` | 吐槽（小号带底色块） |
| `count` | 计数/积累（"第 3 次"） |

- ⚠️ **`element` 与 `sticker` 长期共存**：老工程里全是 `type = "sticker"`，
  两个变体都会被 `process` 当视觉元素看待，**不迁移**（`polish.toml` 一个字节都不动）。
- ⚠️ **`kind` 缺省**：LLM 偶尔漏写（提示词里 `kind` 与 `text` 挨着），
  缺省时留空字符串 ⇒ 按认不出的 kind 处理，**整个工程照样读得出来**。
- ⚠️ **`manual`**：语义与 `sticker.manual` 完全一致 —— 后处理不改写它的时间。
- 时长规矩与贴纸**完全相同**（最短 1.2s / 最长 5.0s / 不跨静音空隙），见 `process` 的 `fit_element_span`。
- ⚠️ **`kind = "text"` 目前不出图**（§5 E3 的排版通道未接上）：
  转译器会**在报告里明说**「文字元素还没有排版通道」，而不是静默跳过。
  `kind = "sticker"` 走贴纸路径，**与 `type = "sticker"` 逐字节等价**。
- ⚠️ **`kind = "text"` 的定位是「候补」**（2026-10-19 用户口径：文字很突出、指向性也强，滥用适得其反）：
  LLM 提示词要求它写**关键词 / 短词组（2~8 字）**而不是整句，**不与贴纸同时出现**（同一段时间二选一），
  同一段最多 1 条，只在「没有合适的贴纸/特效」或「其它效果已经用得比较密」时才写。
  后处理侧 `process::cap_text_elements` 兜底：与贴纸（`sticker` 与 `element{kind="sticker"}` 两个载体都算）
  时间重叠的文字删掉、每段最多留 1 条（留最早）、`manual` 豁免。
- ⚠️ **屏蔽区间里的事件不生成、也不参与后处理**：LLM 生成/贴纸精挑按**起点**剔掉屏蔽段里的候选，
  `process` 的预算、落点、对齐与文字守卫也跳过它们 —— **但一个已有事件都不删**（屏蔽只做标记）。

### 2.2 贴纸 `sticker`（持续）

```toml
type = "sticker"; time = 10.0; end = 12.0
keyword = "ysyk"        # LLM 写：来源包名（emoji/irasu/本地贴纸包）
sticker_tag = "害怕"    # LLM 写：语义 tag（只在生成期用，出片不再读）
name = "cry_01"         # process 解析后回填：实际文件名
source = "emoji"        # process 回填：emoji | irasu | 包名
x = "23%"; y = "71%"    # process 计算（LLM 一律写 "auto"）
sfx = "tag:コメディ"; sfx_volume = 0.4
manual = true           # 手工定过时间（编辑器里拖过/改过）→ 后处理不再吸附/延长；可省
```

- LLM 只负责 `time/end/keyword/sticker_tag`；`name/source` 由 process 按 `[sticker_source]`（emoji / irasu / 本地贴纸包）解析回填。
- **手工选择（编辑器）**：属性面板给的是**两级列表** —— 一级 = 全部 keyword（`包名 · tag`，来自 `GET /api/polish/sticker-library`），
  二级 = 该 keyword 下的贴纸缩略图（`GET /api/polish/sticker-file`）。选中即写 `keyword`/`sticker_tag`/`name`/`source`
  （`name` 存**去扩展名的文件名**）；选「自动」则清掉 `name/source`，交回后处理解析。
- **时间对齐最近的字幕**（Step 2b1，用户要求）：起点吸到最近那条字幕的开始、终点吸到它的结束
  （离最近字幕超过 2.0s 的不动）；排在 2b 落点之前，所以落点用的是对齐后的时间窗口。
  ⚠️ **`manual = true` 的贴纸跳过这条**（连同 §2.1 的画中画对齐、下面 Step 2c 的尾帧延长）——
  否则用户手工改完时间，1 秒后自动后处理就把值吸回字幕边界，看起来就是「改了没成功」。
- **落点由 process 算**（Step 2b）：按 **人物 `layout.liver` / 字幕带 / 弹幕区域 `layout.danmaku_area` / 真实弹幕带** 反选安全区，在**安全区中心**用确定性种子撒点，写回 `x/y` 百分比。`layout.sticker_area` 已降级为偏好（有安全位才优先用），`grid` 完全不参与。
- **跨度自适应**（Step 2c，排在 2b1 对齐之后）：若下一条字幕在 `end` 结束后 50% 以内结束，则把 `end` 延长到那条字幕的结束点 —— 对齐到本句之后再往后接一句（`manual = true` 的跳过）。
- 表现（预览与出片一致）：280×280 盒子，`popIn` 0.35s（`backOut(3)`，−15° 转正）→ `floatUp` 0.4s（上移 18px）→ **保持到 `end`** → `fadeOut` 0.3s（缩到 0.4）；
  也就是说 `end` 是「保持阶段」的终点，还会再淡出 0.3s（入场/浮起也另计）；安全边距 32px，贴纸位置夹在 1080p 内。
- 竖屏：`vertical_position = left` → 贴纸水平居中；`center` → 左右交替。
- 编辑器：泳道拖 / 左右缘缩放；属性面板用**两级列表**选 `keyword`/`sticker_tag`/`name`/`source`
  （一级 = 全部 keyword「包名 · tag」，二级 = 该 keyword 下的贴纸缩略图；选「自动」= 交回后处理解析）。
  改完自动保存并自动跑一次后处理，预览里立刻能看到；**改过开始/结束会自动打上 `manual = true`**
  （面板上显示「手动时间」+「跟随字幕」按钮，点它清掉标记、下一次后处理恢复按字幕吸附）；
  `x/y` 不可改（后处理每次重算）；
  贴纸池在右栏「贴纸来源」勾选；预览资源来自 `/api/polish/project?preview=1` 的 `_assets.sticker_map`。

### 2.3 覆盖层 `overlay`（持续）

```toml
type = "overlay"; time = 0.0; end = 5.0
overlay_type = "gradient"   # "color" | "gradient" | "fade_in" | "fade_out"
color = "#ff0000"           # color / fade 两档用；gradient 用 colors[]
colors = ["rgba(0,0,0,0)", "rgba(0,0,0,0.6)"]
opacity = 0.3               # 默认 1；fade 两档 = 遮罩色的最大不透明度
shape = "radial"            # "linear"(默认) | "radial"，仅 gradient
```

- 表现：进入 0.4s 淡入、结束前 0.4s 淡出；`color` 是纯色覆盖，`gradient` 是线性/径向渐变。
- `fade_in` / `fade_out`（**黑场淡入 / 淡出**，名字说的是画面）：**整窗就是过渡本身**，线性、**不叠**那
  0.4s 包络 —— `fade_in` 起点全遮 → 末端全透（`opacity × (1 − dt/dur)`），`fade_out` 镜像；`color` 缺省
  `#000000`，所以「纯黑遮罩 + `opacity = 1`」＝开头从纯黑淡开。窗口之外照常是正常画面（覆盖层没有
  「无限保持」的语义，想让画面**停在**黑场就把末端拖到片子结束）。
- 片头那 0.5s 原来是内建 `intro-black` 硬编码，已退役：现在由 `process` 在**初次**后处理给首段补一条 `fade_in`
  事件，在时间轴上可拖 / 可改时长 / 可改色 / 可删。**只补这一次** —— 删掉之后再跑 `process` **不会**补回来
  （那是你的意图，不是"默认观感没生效"）；想拿回默认片头就删掉 `polish.toml` 里的 `intro_fade_seeded` 那一行再跑。
- 编辑器：泳道拖 / 左右缘缩放 + 属性面板 —— `overlay_type` 下拉（纯色 / 渐变 / 淡入（黑场）/ 淡出（黑场））、
  颜色取色器、渐变两端色、不透明度、形状（线性 / 径向）可改（`colors` 在面板里就是两个取色器）；
  fade 两档只留颜色（整块时长＝渐变时长，拖右缘改）。

### 2.4 音效 `sfx`（点事件 + 挂在别的特效上）

两种用法：

1. **独立音效事件**：`{"type":"sfx","time":3.0,"end":4.0,"name":"<音效 id>","volume":0.5}`（`end` 可省 = 用文件原始长度）
2. **挂在任何视觉事件上**：`sticker/shake/flash/blur/vignette/hue_shift/noise/pulse/overlay` 都有 `sfx` + `sfx_volume` 两个字段（`sfx = false/null` 表示不响，`sfx = true` 会解析成 `pop-notice`）。

**可用音效目录**：`crates/yekitrim-polish/assets/sfx_catalog.json`（小森平音效库，编译期内嵌）

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

**屏蔽**：`[sfx].blacklist`（项目）与全局 `.yekitrim/config.toml [sfx].blacklist` 合并，写精确 id 屏蔽单个，写 `tag:xxx` 屏蔽整族。

**编辑器**：时间轴「音效」泳道（点事件，可拖/缩放）；属性面板的 `name` 是**音效列表**（`GET /api/polish/sfx-catalog`，
搜索 + 分类提示，不再手打 id），`volume` / `end` 都可改。

### 2.5 视觉特效（11 种瞬时，同一条「视觉特效」泳道）

| type | 表现（预览 = 出片） | 参数 | 跨度 (`end-time`) cap / 默认 |
|---|---|---|---|
| `shake` | 抖动：+off → −off → ±off/2，0.2s 内四段 | `intensity`（默认 14px） | 0.5 / 0.3 |
| `flash` | 前 0.04s 冲到 0.2 不透明度再线性淡出 | `color`（默认白） | 0.3 / 0.25 |
| `blur` | 固定 8px：0.48s 满 + 0.32s 淡出（共 0.8s 窗口） | — | 0.4 / 0.3 |
| `vignette` | 四角暗角：前 40% 保持 1.0 再淡出 | — | 0.6 / 0.4 |
| `hue_shift` | 0.15s 淡入 0.25 不透明度再 0.8s 淡出（共 0.95s 窗口） | `color`（默认 `#ff0040`） | 0.5 / 0.35 |
| `noise` | 噪点：0.08s 升到 0.08 不透明度，0.72s 淡出（共 0.8s 窗口） | — | 0.4 / 0.3 |
| `pulse` | 0.12s ×1.06 → 0.28s ×0.97 → 回正（共 0.48s） | — | 0.5 / 0.4 |

- **跨度由系统接管**：`process/fx.rs::normalize_event_ends` 把每个瞬时特效的 `end - time` clamp 到 `0.1 ~ cap`，
  缺失 `end` 的按类型补默认值 —— LLM 写多长都会被改（prompt 里也明说了）。
- 编辑器：一条「视觉特效」泳道装全部 11 种（靠块标题/颜色区分）。
  **事件类型由所在轨道决定，面板里不给跨轨道选择**：泳道标签的 `＋` 负责定类型 —— 运镜/贴纸/覆盖层/音效轨
  直接加该类型，**特效轨的 `＋` 弹 11 项细分类型菜单**（选哪个加哪个）；属性面板里特效事件只列这 11 种可换
  （切换按新类型重建字段、保留时间跨度与配音），其余事件只读展示「类型 贴纸 sticker」。
  `intensity`（shake）与 `color`（flash/hue_shift）都有输入控件。

### 2.6 其余事件字段

- `anchor`（12 类事件上都有 `anchor: Option<String>`）：**声明了但没有任何代码读它**，目前是死字段。
- `Segment.fx`（`low|med|high`）：只进 Step2 的 user prompt（「段情绪密度」），**不参与任何渲染**。
- `sticker_tag`：生成期用它选贴纸；出片不读，但**编辑器的手工选择会写它**（一级筛选就是它）。
- **时间字段（2026-09 起）**：`time`(开始) + `end`(结束) 两个绝对秒，**没有 `duration` 了**。
  老工程的 `duration` 由 `yekitrim-render-core/src/legacy.rs::upgrade_event_times` 在反序列化前换算成 `end`
  （`end = time + duration`，渲染结果 1:1 不变；下一次保存即完成迁移），跑的入口有四处：
  `polish.toml` 加载、`PUT /api/polish/project`、LLM 事件输出（含老 checkpoint）、wasm 预览配置。
  缺 `end` 的事件由 `process` Step 0 按类型补默认（`Event::default_end`），`sfx` 例外 —— 缺省 = 用文件原始长度。

---

## 3. 编辑器实现的范围（一张表看清）

**能改**（时间轴 + 右栏）：

| 能力 | 位置 | 写回 |
|---|---|---|
| 事件的**类型 / 时间 / 结束 / 跨度** | 时间轴泳道拖拽、左右缘缩放；属性面板 `type/time/end` | `PUT /api/polish/project {segments}` |
| 贴纸的 `keyword` / `sticker_tag` / `name` / `source` | 属性面板两级列表（keyword → 贴纸缩略图） | 同上 |
| 音效的 `name` / `volume` | 属性面板（音效目录列表） | 同上 |
| 任意视觉事件的 `sfx` 引用 / `sfx_volume` | 属性面板「配声」 | 同上 |
| 运镜的**运镜过渡**（平滑 / 硬切 `cut` + `speed`）、`zoom`、`anchor` / `anchor_to` | 属性面板（「运镜过渡」+「景别」+「起点/终点」） | 同上 |
| 覆盖层 `overlay_type` / `color` / `colors` / `opacity` / `shape` | 属性面板（下拉 + 取色器；fade 两档只留颜色） | 同上 |
| 瞬时特效 `intensity`（shake）、`color`（flash/hue_shift） | 属性面板 | 同上 |
| 分段 `note` / `fx` / `start` / `end`（体拖、右缘缩放、面板确认后删） | 时间轴 + 属性面板 | 同上 |
| 字幕增删改拆、设入出点、拖动 | 字幕泳道 + 属性面板 | `PUT /api/polish/subtitles` |
| 屏蔽区间增删改拆、拖动 | 屏蔽泳道 + 属性面板 | `PUT /api/polish/project {mute_ranges}` |
| 「重新后处理」按需重算（自动那次被占用跳过时的兜底） | 时间轴头部按钮 | `POST /api/polish/process` |
| 「去静音」自动屏蔽 / 撤销 | 时间轴头部按钮 | `POST /api/polish/silence` |
| 贴纸来源池 `sticker_source` | 右栏「贴纸来源」 | `PUT /api/polish/project {sticker_source}` |
| 运镜聚焦框 / 弹幕区域框 | 右栏「区域编辑」（保存后自动重跑一次后处理） | `PUT /api/polish/project {layout}` → `POST /api/polish/process` |
| LLM 剪辑 / 渲染导出 / 封面 | 右栏 + 封面设计器 | `POST /api/polish/{llm,render,cover}` |

**统一约定**：所有块（事件 / 字幕 / 屏蔽 / 分段）都是同一套控件 —— **体拖移动、左缘改开始、右缘改结束、双击删除**
（分段例外：不给左缘 —— 缩 start 会把事件留在段外，正确处理要连带搬事件）；分段的新增仍然只能靠 LLM 剪辑。
属性面板的第一块永远是 **开始 / 结束 / 入点 / 出点 / 定位**（删除走双击块），往下才是各类型自己的属性 ——
**没有「保存修改」按钮**：改动即时镜像进预览，**250ms** 防抖落盘（2026-10-05 由 700ms 收短），
预览侧重载再防抖 150ms（由 300ms 收短），落盘后再防抖跑一次后处理。

**不能改**（只能手改 `polish.toml` 再重跑后处理/渲染）：

`[style.camera] zoom_mid/zoom_close/zoom_wide`（档位名与锚点已可改，数值只能手改 `polish.toml`）、
`sticker.x/y`（每次 process 重算，面板里也不给输入框）。

**改动会自动落盘 + 自动后处理**（`PUT /api/polish/project` 之后跟一次 `POST /api/polish/process`，
防抖 1.2s、静默、有任务在跑就跳过）：后处理会按既有规则**改写**事件（贴纸吸附到最近字幕、落点重算、
运镜锚点解析、画中画配额/对齐/收工），完成后编辑器会重新拉数据把结果同步回块与面板 ——
所以你拖到的位置可能被规范化，这是预期的（渲染导出本来也会先跑一次 process）。

**区域选框（人物 / 弹幕区域）同样要重跑后处理** —— 它们不是渲染的输入，而是 process 的输入：
Step 2b 按它们反选安全区、**重写每个贴纸的 `x/y`**，Step 2d 把 `anchor` 解析成 `origin`/`to_origin`
（**每次运行都覆盖**）；渲染侧只认这些结果坐标（`templates/index.html` 的 `ce.origin || liverOrigin`
里，读 layout 的只是"事件没写 origin"时的兜底）。所以保存选框后编辑器与预览里显示的旧坐标会骗人
—— 渲染虽然内部会先跑一次 process 兜底，但界面上看起来像"没生效"。UI 里「保存选框 / 清除选框」
现在会自动起一次 `process`（与时间轴保存同一条路）；命令行手动跑 `yekitrim polish process <目录>` 同理。

---

## 4. 预览 / 出片一致性（现状与已知差异）

**已对齐**（逐条核对 `engine.js` ↔ `timeline.rs`）：

- 瞬时特效的求值窗口：shake 0.2s、blur 0.8s(0.48 满)、hue_shift 0.95s、noise 0.8s、pulse 0.48s、vignette = `(end-time)*0.4` 保持后淡出。
- 瞬时跨度上限表：`process/fx.rs` 与 `engine.js` 的 `TRANSIENT_CAP` 完全一致。
- 贴纸四阶段、字幕 fade+事件联动脉冲、弹幕滚动、运镜过渡时长与曲线。

**已知差异（都在缺字段的边界上）**：

| 项 | 预览 `engine.js` | 出片 `render-core` | 影响 |
|---|---|---|---|
| ~~`[style.camera]` 缺失时的倍率~~ | 已修：四条路径共用 `CameraStyle::factor`，缺省一律 **1.3 / 1.5 / 0.55** | 同左 | 无 |
| `flash` 缺 `end` | 0.35s | `max(end-time, 0.1)` | process 已把缺失值填 0.25，正常流程不会走到 |
| `shake.intensity` 为 `0` | 退回 14px | 直接用 0 | 只有手写 `intensity = 0` 才看得出 |
| 非 16:9 源视频 | 预览 `object-fit: contain`（留黑边） | 出片把源拉伸铺满 1920×1080 | 直播录像一般就是 16:9，不受影响 |

---

## 5. 默认值 / 上限速查

```
瞬时特效跨度 (end-time)：shake 0.5 | flash 0.3 | blur 0.4
                    noise 0.4 | pulse 0.5 | hue_shift 0.5 | vignette 0.6
                    缺失时默认：0.3 / 0.25 / 0.3 / 0.4 / 0.25 / 0.25 / 0.3 / 0.4 / 0.35 / 0.35 / 0.4
                    下限一律 0.1s
运镜倍率：[style.camera] zoom_mid 1.3（中） zoom_close 1.5（近） zoom_wide 0.55（远=画中画，四周黑边 432px）
          放大档位不会露底（s≥1 时画面只向外扩张）；老工程里写死的旧默认
          （中 1.15 / 近 1.3 / 远 0.7）由 process Step 0a 按值迁移，手改过的不动
运镜起点/终点：anchor/anchor_to 解析成 origin/to_origin（liver/danmaku/sticker/center，未知值保留 origin）；两端相同则清 to_origin
          没写 anchor 时用 origin 字面量；也没有则 region_origin(liver) → 65% 50%
运镜位移：块内 origin -> to_origin，窗口 [time+td, end]、pow2Out；td 见「运镜过渡」
          例外：另一端是"远"时整段硬切（zoom + 不动点瞬间到位，跨度不再产生缓动）
硬切 cut：事件写 cut=true（编辑器「方式 = 硬切」）→ 这一条整档瞬间到位，任意景别都行
画中画配额：cap_pip_shots —— 整片只留最早的一条，其余降级成"中"；align_pip_shots —— 起止对齐字幕边界（不足 1.5s 往后借）
画中画收工：close_pip_shots —— 画中画的 end 就是它到点收工的时刻，在原终点补一条回镜头
          补一条回镜头事件（时间上前一条非画中画运镜的落点，硬切回去），持到下一条运镜；
          下一条运镜在 0.5s 内就来则不补；幂等
默认运镜：process 给每个 ≥1s 且无运镜的段补 zoom="中" anchor="liver"（整段）
音效音量：[sfx].volume 0.1（事件 volume/sfx_volume 覆盖）
音效条目：1213 语义 id / 2118 变体 / 7 大类 / ~75 tag；中文关键词兜底规则 23 条
预算：贴纸 min(round(每5条字幕1个), ceil(时长/12))｜音效 min(round(每8条字幕1个), ceil(时长/15))
      两者上限都是 segment 数 − 1（至少 1）
贴纸盒子：280×280，安全边距 32px；popIn 0.35 → floatUp 0.4（上移18px）→ hold 到 end → fadeOut 0.3
弹幕：1920×1080 画布上半，y = y/1920*540，travel_duration 默认 8s，opacity 0.9
片头淡入：process 只在初次后处理给「时间最早且 start<0.5s」的段补一条 overlay fade_in（#000000 / opacity 1 /
          opacity 1 / 0.5s 或到段末）；判据 polish.toml 的 intro_fade_seeded；段跨度<0.2s、或该窗口已有覆盖事件则不补；可删可拖，删掉再跑不会补（想拿回就删掉那一行）
```
