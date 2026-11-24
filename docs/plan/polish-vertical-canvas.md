# 精修：画面构成（位置/背景）增强 + 竖屏 9:16 画布编辑与导出

> 日期：2026-09-19（末次更新 **2026-09-21**）· 状态：**P0–P3 已落地并验证，P4 部分落地**（阶段表见第六节）
> 本轮变更：竖素材出片的**画面摆放 + 运镜门控**修复 —— 用户报告「横屏导出时竖屏素材被拉伸成宽屏，
> 而不是竖屏素材 + 模糊的关键帧」「竖屏素材没有禁用运镜，横屏竖屏导出都一样」（见文末 2026-09-21 节）
> 范围：`yekitrim-polish` + `yekitrim-server` + `webui`（**仅 Chrome 出片路径**，wgpu 不修）

## 需求（两部分）

- **A. 横画布 × 竖素材的操作丰富**：位置左/中/右、背景黑/当前帧模糊、特效解禁
- **B. 竖屏 9:16 画布的编辑器 + 导出兼容**

前置结论：**竖屏不是从零做，是"已有半成品"** —— 源视频竖屏时已有「模糊背景 + 主画面左/中摆放」，
但它 ① 运行时自动检测、**不落盘不可编辑**（且检测逻辑重复两份）；② 输出画布**仍是 16:9**；
③ 竖屏分支会**关掉几乎全部视觉特效**。

---

## 一、现状核查（先对齐事实）

### 1.1 已存在的东西

| 能力 | 位置 | 说明 |
|---|---|---|
| 竖屏素材检测 | `render/server.rs:243-254`（预览服务）<br>`api/polish.rs:1007-1024`（API） | ffprobe 判 `h > w` → 注入 `vertical=true` + `vertical_position`。**同一逻辑重复两份**，不落盘 |
| 竖屏布局 CSS | `templates/index.html:31-35` | `#video-wrap.portrait.left / .center` + `object-fit:contain`。**只有左/中，无 right** |
| 模糊背景 | `templates/index.html:26-28` | `#bg-video { object-fit:cover; filter:blur(28px) brightness(0.5) saturate(1.15); transform:scale(1.15) }`，靠 `.on` 开关。**只有模糊，不能选黑底** |
| 背景配置先例 | `project/config.rs:303-321`、`cover.rs:98-160` | `[cover] bg_mode = color\|blur\|image` + `bg_color` + `blur_scale`，含净化 + 单测 + PUT 白名单 + 落盘测试。**正片侧镜像这套命名，不另发明** |
| WebUI 竖屏预览分支 | `PolishPreview.vue:5,129,725-727` | 与模板同构的 `.portrait.left/.center` |

### 1.2 关键缺口：输出画布尺寸没有单一真相源（SSOT）

1920×1080 散落 8 处：

| 位置 | 用途 | 影响出片 |
|---|---|---|
| `templates/index.html:1524-1525` `CANVAS.width/height` | **Chrome 路径真正的画布**（默认出片路径） | ✅ 决定像素尺寸 |
| `index.html:20-23` `#player { aspect-ratio:16/9 }` | 预览舞台比例 | 否 |
| `index.html:45,46,50,53,54` | `#overlay` 1920×1080、`#captions` 1680/left 120/bottom 80、`#danmaku` 1920×540、`#emoji-layer`、`.sticker-img` 280×280 | ✅ 绝对 px |
| `index.html:352-353` `pctX/pctY` | 百分比→像素换算基准 | ✅ |
| `index.html:773` | 贴纸 clamp `1920-280-32` / `1080-280-32` | ✅ |
| `render/context.rs:34-35` | `RenderContext.width/height` | ❌ **只服务 wgpu**；`pipeline.rs:394-401` 构造 ctx 时从不赋值 → 出片路径上是死字段 |
| `process/placement.rs:28-29` | `FRAME_W/FRAME_H`（贴纸落点网格 + 安全区算法） | ✅ process 阶段 |
| `render/chrome.rs:27,42` | `--window-size=1920,1080` | 间接（页面布局/视口） |
| `cover.rs` + `templates/cover.html:18,94` | 封面截图尺寸 | ✅ **恒 1920×1080 横版**（2026-09-20 用户要求改回固定横版：平台封面就是横的；素材比例靠 `#frame img` 的 `object-fit:contain` 保持，画布尺寸不动） |
| `PolishPreview.vue:719-721,737,742,744,653` + `engine.js:140-148` | 编辑器第三套：舞台比例、`.overlay-space` 1920×1080、`.danmaku-layer` 1920×540、`overlayScale = stageW/1920` | 否（编辑器预览） |

### 1.3 ⚠️ 修正：特效作用域实测结论（推翻初版计划）

初版计划判断「运镜与特效需要按作用域三分类拆分，`blur` 要改成只作用主画面层」。
**逐条读 handler 后不成立** —— 现有实现的作用对象本来就是对的：

| 事件 | 实际作用对象（实测） | 竖屏布局下 |
|---|---|---|
| `camera`（运镜） | `#source-video`，`transformOrigin` 取横屏九宫格百分比（`:715-734`） | ❌ **唯一真正错乱**：origin 按横屏九宫格算，作用在窄竖视频上会指向视频外 |
| `shake` | `#video-wrap`（`shake()` `:447-456`） | ✅ 只抖主视频容器；背景 `#bg-video-wrap` 是兄弟节点不动 |
| `blur` | `#video-wrap` `filter:blur()`（`:793-800`） | ✅ **只糊主视频**，背景是同级兄弟 `#bg-video-wrap`，不受波及 |
| `color_shift` | `#video-wrap` `filter:sepia`（`:849-854`） | ✅ 同上 |
| `zoom_bounce` / `pulse` / `stutter` / `split` | `#source-video`（`:802-807` / `:868-878` / `:823-834` / `:836-847`） | ✅ 作用在视频元素本身；竖屏 `curZoom` 固定 1.0（`:714`），缩放基准正常 |
| `flash` / `vignette` / `hue_shift` | `#flash-overlay`（全屏，`:458-463` / `:809-821`） | ✅ 刻意全屏 |
| `noise` | 全屏 div（`:856-866`） | ✅ 刻意全屏 |
| `overlay` | `#color-overlay`（全屏，`:880-896`） | ✅ 刻意全屏 |

**结论**：`index.html:911-921` 的 `VIS` 表 + 一刀切 `return` 属于**过度屏蔽**，
删掉整块即可（handler 内部自己会调 `fxSfx`），**不需要按作用域重构**。
只需保留 `:714,718` 的运镜屏蔽。**工作量比初版估计小一个量级。**

### 1.4 其他必须知道的坑

1. **保存白名单静默丢字段**：`api/polish.rs:1260-1289` `polish_project_put_locked` 是逐字段
   `if let Some(...)` 白名单（现只收 `segments` / `mute_ranges` / `layout` / `sticker_source` / `subtitle_font`）。
   不在其中的字段**保存时不报错、但存不进去**。
2. **`deny_unknown_fields` + `flatten extra`**：`ProjectFile`（`config.rs:6,62-63`）与 `Layout`
   （`types.rs:624,641-642`）都有。新字段必须**显式声明 + `#[serde(default)]`**，否则落进 `extra` 被静默忽略。
3. **`vertical_position` 现在漂在 `Layout.extra` 里**，不是 typed 字段。
4. **编码层不用改尺寸**：`pipeline.rs:415-432` 的 ffmpeg 只有 `-framerate` / `-c:v` / `-pix_fmt yuv420p`，
   **无 `-s` / `-vf scale`**；输出分辨率 = 上游后端返回的 JPEG 尺寸；`concat` / `mux` 是 `-c copy`。
   → **改画布只需改"上游每帧产出多大"**。fps=30 硬编码在 `pipeline.rs:88`。
5. **预览 ≠ 成片是两套独立实现**：`webui/src/polish/engine.js` ↔ `templates/index.html`，必须同步。
6. **`.source-video` 横屏用 `object-fit:cover`（`:30`）**：画布比例 ≠ 素材比例时会裁切。
   竖素材分支已切 `contain`，**横素材进竖画布必须同样切 `contain` + 背景补边**。
7. **wgpu 路径是明确缺口**：`RenderContext.vertical` 恒 false、`SOURCE_VERT`（`shaders.rs:55-73`）
   是硬编码全屏 NDC quad + uv 0..1 铺满、**无 dst-rect / 比例适配** → 竖素材会被拉伸变形；
   `composite.rs:370-380` 的 CPU 合成已清空。按 AGENTS.md（wgpu 不在考虑范围）**本次不修**。

---

## 二、设计原则

1. **先立 SSOT，再谈竖屏**：新增配置节，Rust 侧 `CanvasSize::resolve(&cfg)` 与 `StageFit::resolve(&cfg)`
   **只此一处**算尺寸与摆放；模板/前端读注入进 cfg 的 `_canvas`，不再各自硬编码。
2. **向后兼容 = 自动检测仍是默认**：`position = "auto"` / `bg_mode = "auto"` 精确等于今天的行为
   （比例不匹配 → blur + left）。**存量项目零迁移、出片逐像素不变**（回归测试钉住）。
3. **比例常量替代绝对像素**：横向尺寸用 `k = W/1920`，纵向位置用 `kh = H/1080`。
   **横画布下 k = kh = 1，零回归**；竖画布 1080×1920 下 `k = 0.5625 / kh = 1.7778`。
4. **方位是对称的**：横画布用 left/center/right，竖画布用 top/center/bottom，同一个 `fit()` 实现。
5. **预览 = 成片**：`engine.js` 与 `index.html` 任何改动必须同步，加常量一致性测试。
6. **风险倒序实施**：先做横画布下的画面构成（低风险、立刻有用），再做画布方向参数化（动常量面最大）。

---

## 三、配置契约（`polish.toml`）

```toml
[canvas]
# 输出画布方向：landscape(1920x1080) | portrait(1080x1920)
# 只给这一个开关，宽高由此推导 —— 不做分辨率档位、不做自定义宽高（已定）
orientation = "landscape"

[stage]
# 画面构成：主画面比例 ≠ 画布比例时如何摆、让出来的边补什么。
#   auto  = 保持现状：不匹配时按 blur 处理 + 主画面靠左（存量项目默认，出片不变）
#   black = 背景纯黑
#   color = 背景纯色（bg_color）
#   blur  = 背景为当前帧 cover 铺满 + 高斯模糊
# ⚠️ 背景**不是只在留边时才用得上**：竖画布即使主画面正好铺满也按这个模式铺一层底色
#    （震动 / 画中画 / 手持摇摆会把主画面挪开，没有底色就露透明 → JPEG 纯黑）。
#    横画布铺满时保持"无背景"，存量横画布项目逐帧零回归。
bg_mode = "auto"
bg_color = "#000000"     # bg_mode = color 时使用（复用 cover::sanitize_hex_color 净化）
blur_scale = 1.0         # bg_mode = blur 的模糊倍率，1.0 = 基准 28px（镜像 [cover].blur_scale）
# 主画面位置：auto | left | center | right（横画布）/ top | center | bottom（竖画布）
position = "auto"
# 主画面缩放：1.0 = contain（完整可见不裁切）
scale = 1.0
# 主画面额外留白（px，横画布基准，按 k 缩放）
padding = 0

[sticker]
# 落点模式：auto(默认，按安全区算法自动避让、逐条变化) | manual(固定落点，电台/竖屏常用)
position_mode = "auto"
# 手动固定落点（position_mode = manual 时生效）；百分比，相对输出画布，"x% y%"
position = "50% 50%"
```

**说明**
- `sticker_source`（现有根级 `Vec<String>`）**保持不动**，避免历史迁移；`[sticker]` 只管落点。
- 值域走**净化函数**（照抄 `cover.rs:98` 的 `cover_bg_mode` 模式）：非法值回落默认 + `tracing::warn`。
- **`vertical_position` 提升为 `[stage] position` 的正式 typed 字段**，并保留一次性兼容读取
  （读到旧的 `layout.vertical_position` 就映射过来），否则老项目位置回默认。

**Rust 落点**
- `project/config.rs`：`ProjectFile` 加 `#[serde(default)] pub canvas: CanvasConfig` /
  `pub stage: StageConfig` / `pub sticker: StickerConfig`；同步手写 `Default`、`to_toml_commented()`（`:413`）注释、`example()`（`:630`）。
- `api/polish.rs:1260-1289`：新增三个 `if let Some(...)` 分支，**照抄 `:616-627` 的 cover 背景写法**，
  并补一条镜像 `:1662-1713` 的落盘测试（值真写进 toml + 非法值被净化 + 兄弟键不丢）。

---

## 四、Part A：横画布 × 竖素材的操作丰富

### A1 位置：左 / 中 / 右

- **模板**：`index.html:33-34` 补 `#video-wrap.portrait.right { justify-content:flex-end; }`；
  接线处 `:583-593` 与 `:917` 从 `layout.vertical_position` 改读 `_stage.position`。
- **WebUI**：`PolishPreview.vue:725-727` 同步补 `.portrait.right`；`:129` `portraitClass` 改读 `stage.position`。
- **UI**：右列 `PolishView.vue:121-197` 的 `.gen-block` 分组模式，新增「画面构成」分组：位置三态按钮 ◧ ◫ ◨。

### A2 背景：黑 / 当前帧模糊（+ 纯色）

- **模板**：`#bg-video-wrap` 现在只靠 `.on` 开关 + CSS 固定 `blur(28px)`。改为：
  - `black` → 关掉 `#bg-video-wrap`，`#player` 底色设 `#000`，主画面 contain 后自然黑边；
  - `blur` → 开 `#bg-video-wrap`，**模糊半径由 `blur_scale` 驱动**（写成 CSS 变量 `--bg-blur`，由 cfg 注入）；
  - `color` → 关 bg 视频，`#player` 背景设为 `bg_color`。
- ⚠️ **上面这三条只讲 DOM 预览层，出片路径必须另画一遍**（2026-09-20 用户报"震动露出黑边"）：
  导出走 canvas `renderFrame`（`?capture=1` + `__CAPTURE_CHUNK`），`clearRect` 之后的
  **透明区在 JPEG 里就是纯黑**，DOM 的 `#bg-video-wrap` 根本不参与出片 ——
  所以 `renderFrame` 里要有自己的背景层：`black`/`color` 铺纯色，`blur` 用**同一帧 bitmap**
  按 cover 铺满 + `blur(px) brightness(0.5) saturate(1.15)` + 绕中心放大 `blur_img_scale`。
  钉子：`templates/index.html` 的 `drawStageBackdrop` / `stageCoverRect`，
  契约测试 `template_stage_hooks_match_contract` 断言这些消费点存在且**画在视频层之前**
  （只消费在 DOM 上正是"预览有底色、成片纯黑"的成因）。
- **WebUI**：`PolishPreview.vue:4` 已有 `bgRef`，复用同一套 + 同 CSS 变量。
- **模糊实现走 CSS `filter: blur()`**（与封面 `cover_blur_css` 同思路）：**不需要 GPU**，
  绕开 `wgpu-render` feature 门控。
- ⚠️ **铺满（`axis=fill`）也要背景层**：竖画布 × 同比素材时 `resolve_stage` 曾经强制
  `bg="none"`，把用户选的"模糊"整个吞掉。现在的门控是**画布方向**而不是 `axis`：
  `axis=="fill" && !portrait` → `none`，其余一律按 `bg_mode`。
  `bg_mode=auto` 的语义不变（留边 = 当前帧模糊）。

### A3 特效：只屏蔽运镜，其余一律通过（已定）

- **删除 `index.html:911-921` 整块**（`VIS` 表 + `return` + 贴纸强制覆盖）。
  handler 内部本来就会调 `fxSfx`，删掉后音效不受影响。
- **保留 `index.html:714` 与 `:718`**（竖屏 `curZoom=1.0` + 跳过运镜事件）。
- 依据见 §1.3 实测表：其余特效的作用对象本来就是正确的，无需改作用域。
- **`engine.js` 同步**：删掉对应的 `VIS` 屏蔽分支（`engine.js` 里同名逻辑）。
- 📌 **出片（canvas）侧的运镜门在 `getActiveCamera` 函数开头**（`CFG.vertical` → 整条返回
  `zoom=1 / handheld=0`，与 `engine.js::cameraAt:310` 逐值同口径）。`.portrait` 布局在 P1 删除后，
  这套门控一度只在 DOM 侧完整 —— 循环末尾归零 `handheld` 挡不住 `zoom`，成片仍在缩放（见文末 2026-09-21）。
- 📌 若日后想让「竖画布 + 竖素材」（主画面正好铺满画布）也能运镜，
  只需把屏蔽判据从「源素材竖屏」改成「主画面未铺满画布」——**一行判据**，本计划按已定决策不做。

### A4 🆕 贴纸落点：auto / manual（本次新增需求）

**语义**
- `auto`（默认）= **现状**：`process/placement.rs` 的安全区算法逐条算落点，写进 `segments` 的 `x/y`（百分比），渲染时读它。
- `manual` = **固定一点**：所有贴纸落在 `[sticker] position` 指定的一点（电台、竖屏常用）。

**渲染实现**
- 模板与 `engine.js` 在算贴纸位置时分支：
  - `manual` → **忽略 events 里的 x/y**，直接用 `_sticker.position`；
  - `auto` → 现状（读 events 的 x/y）。
- `manual` 下**仍保留**入场/floatUp 动画（`index.html:776-780`）与出框 clamp（`:773`，边界按 k 参数化）；
  "固定"指落点固定，不是取消动画。

**为什么 manual 走"渲染时覆盖"而不是"process 时改写 events"**
- 用户在编辑器里拖点/改数字 → **立即生效，无需重跑 process**，预览即时反馈。
- 因此 **`process` 的落点计算照常执行、不跳过**：这样 `manual → auto` 切回来也**无需重跑 process**
  （events 里一直有 auto 落点）。代价是 manual 时白算一次（落点只是 process 里很小的一步），换来"切模式零成本、行为可预测"。

**顺带修掉一个真实缺陷**：`auto` 模式目前**不知道主画面在哪**。
`placement.rs` 的障碍来自 `layout` 区域（liver / danmaku_area / sticker_area）+ 字幕带，
而"主画面在画布内的实际矩形"是运行时 `fit()` 出来的，不在其中 →
**横画布 + 竖素材 + auto 时贴纸可能被放到主视频身上**。
→ 把 `StageFit` 解析出的主画面 dst-rect 作为 `placement` 的**额外障碍**传入，auto 才是真的"自动避让主画面"。

**竖屏原有的贴纸强制覆盖删除**：`index.html:916-920`（left → 强制 50%，否则 15%/85% 交替）
与 `yekitrim-render-core/src/timeline.rs:541-548` 同款逻辑一并删除 —— 该行为由 `manual` 模式正式替代，
`auto` 则走安全区算法（含上面的主画面避让）。

**UI**（`PolishView.vue:156-176` 的「贴纸来源」分组内）
- 落点：`自动` / `手动` 二选一；选手动时出现 `x%` / `y%` 数字输入 + 九宫格快选。
- 可选增强（后续）：预览舞台上直接拖拽放置。**P2 阶段先不做**。

### A5 字幕：限宽 + 多行（本次新增，已定）

> 由 Part B 竖画布驱动，但折行逻辑是横竖共用的**同一份**（横画布下等价于今天，见下方"零回归"）。

**现状（实测）—— 这不只是竖屏问题，今天就已经是个缺陷**

- **出片路径没有折行能力**：`index.html:1635-1676` 的 canvas 字幕是**单行连排** ——
  `totalW = Σ measureText(part)`、`sx = cx - totalW/2`、逐 part `fillText`，
  **没有任何宽度上限或折行**。唯一天花板是 ASR 侧 `[asr] max_segment_chars = 30`
  （且只在"无标点/无气口"时兜底切分）。
  → 1920 宽下 30 字 × 72px ≈ 2160px（再加 12px 描边）**已经越界**；竖画布 1080 宽下 14 字就贴边。
- **预览路径却会自动折行**：DOM 的 `.cap{width:100%}` 装在 `#captions{width:1680px}` 里
  （`index.html:46-47`），浏览器自己折行；`PolishPreview.vue:744-745` 同构。
  → **两条路径今天就在折行行为上不一致**：预览折行、成片不折。
  竖画布把这个差异放大成"预览正常、成片字幕跑出屏幕"。
  `engine.js:522-549 subsAt()` 只产出 `parts / opacity / yOff / scale`，**折行从来没实现过**。
- 另一处既有不一致（顺手一起定）：`.cap` 在模板里是 `position:absolute`（多条 active 字幕**叠在同一位置**），
  在 `PolishPreview.vue:745` 里是 `position:relative`（多条**按流堆叠**）。多条同屏时预览与成片已经不同。

**规格**

| 项 | 取值 | 理由 |
|---|---|---|
| 安全宽 | `maxLineW = W * 0.875`，居中（左右各留 `W * 0.0625`） | 1920 下 = `1680 / 120`，与今天 `.captions{left:120;width:1680}` **逐像素一致 → 横屏零回归**；1080 下 = 945 / 67.5 |
| 折行 | 段内贪心；断点优先级 标点（，。、；：！？…）→ 空格 → 任意字 | 中文无空格，标点优先；高亮词允许被拆 |
| 数据结构 | 折行后重算为 `lines: [[{text, hl}]]`，`hl` 跨行保持 | 现在 `parts` 是平的，折行必须重排 |
| 行数 | 上限 **3 行**；超过则按比例缩字号（下限 `0.7 ×` 配置字号）；仍超则继续增行 | 竖屏主画面在中间，字幕向上生长要有天花板 |
| 堆叠方向 | **底行基线 = 今天单行的位置（不变）**，多出的行**向上**堆叠，行距 `fontSize × 1.5` | 单行时输出与今天逐像素一致 |
| 多条字幕 | 统一为**按行堆叠**（两个路径都改），不再一条叠一条 | 顺手修掉上面的既有不一致 |
| 字号 | **不做 k 缩放**，沿用 `[style.subtitle] size`（默认 72）绝对值 | 1080 宽 / 72px 是竖屏常规字号；按 k 会掉到 40px 偏小 |
| 配置 | **不新增用户配置项**，常量随 `_canvas` 注入（`caps = {maxW, maxLines, marginX}`） | 本次没有调参需求；将来要调再提升为字段 |

**实现要点**

1. 抽一个纯函数 `wrapCaptionParts(parts, maxLineW, measure)`（`measure` 由调用方注入 `ctx.measureText`），
   放 `templates/index.html` 的 `renderFrame` 上方（与 `ensureCanvas` 同层）；
   `renderFrame:1635-1676` 改为逐行绘制。
2. **DOM 预览（`index.html` + `PolishPreview.vue`）必须用同一个函数算断点**，再手动按行渲染，
   **不要交给浏览器折行** —— 否则预览与成片断点不同（CJK + 标点 + 高亮切分的组合下几乎必然不同）。
   预览侧测量用离屏 canvas（`engine.js:115-130` 的 `dmMeasureCtx` 是现成先例）。
3. 描边要算进 `maxLineW`：canvas `lineWidth` 沿路径中心扩散（两侧各出 `sw/2`），
   DOM 的 `-webkit-text-stroke` **不占布局宽度** —— 这是两条路径唯一的宽度差异，**以 canvas 侧为准**。
4. 验收钉子：短字幕（`totalW ≤ maxLineW`）必须**返回 1 行且几何与今天逐像素一致**。

### A6 弹幕：完全不动（已定）

- **不折行、不限宽**（`.dm{white-space:nowrap}` 保持）、**不新增配置**、不改语义。
- 只跟随 §B2 的统一参数化：宽度跟随画布（全宽）+ 高度 `540*k` + y 映射 `(d.y/W)*(540*k)`
  （文档既有条目；横画布下逐像素不变）。
- 备查（本次不改）：竖画布下这个带占画布高度的比例会从 50% 降到约 16%（`540*k / 1920`）。
  若将来希望竖屏弹幕占满上半屏，只改这一处（`540*k` → `540*kh`）。

---

## 五、Part B：竖屏 9:16 画布的编辑 + 导出

### B1 Rust 侧尺寸贯通

1. `project/config.rs`：`CanvasConfig::resolve() -> (u32, u32)`（唯一推导点，`landscape → 1920×1080` / `portrait → 1080×1920`）。
2. `render/server.rs:235-300` `build_html`：注入 `cfg["_canvas"] = {"width","height","k","kh","stage":{...},"sticker":{...}}`；
   **同时把重复的竖屏检测收敛到这一处**（`api/polish.rs:1007-1024` 改读同一份），补 `tracing` 日志。
3. `render/context.rs:9-43`：`width/height` 改为从 cfg 填；`pipeline.rs:394-401` 真赋值（消掉"字段恒默认"的谎）。
4. `render/chrome.rs:27,42`：`--window-size` 与 `window_size()` 跟随画布。
5. `process/placement.rs:28-29`：`FRAME_W/FRAME_H` 从常量改为**参数化**
   （调用点 `process/mod.rs:313,559,588,761,782,1322,1436` 需拿到画布尺寸），并接收 A4 的主画面障碍矩形。
   ⚠️ 落点是**在 process 阶段算好写进 segments 的**：改画布方向后**旧项目的落点需重跑 process 才更新**
   → 编辑器补提示/入口（"画布已变更，建议重新后处理"）。
6. `pipeline.rs:88` fps=30 与 `ffmpeg.rs:19-39` 编码器探测**不动**（§1.4 第 4 条）。

### B2 模板参数化（`templates/index.html`）

| 常量 | 现状 | 改法 | 基准 |
|---|---|---|---|
| `CANVAS.width/height` | `:1524-1525` 写死 | 从 `CFG._canvas` 读 | — |
| `#player-wrap` / `#player` `aspect-ratio` | `:22-23` `16/9` | 由 `_canvas` 注入 | — |
| `#overlay` | `:45` `1920×1080` | `W×H` | — |
| `#captions` | `:46` `width:1680 left:120 bottom:80` | `W*0.875 / W*0.0625 / 80*kh`（= A5 的 `maxLineW`，同一个常量） | 宽/高 |
| `.cap` | `:47` `width:100%` + 浏览器自动折行 | 宽度不变，但**改由 A5 的折行函数分行渲染**（不再交给浏览器） | — |
| `#danmaku` | `:50` `1920×540` | `W × 540*k`（A6：语义与行为都不动） | 宽 |
| `#emoji-layer` | `:53` `1920×1080` | `W×H` | — |
| `.sticker-img` | `:54` `280×280` | `280*k`（建议 CSS 变量 `--sticker-size`） | 宽 |
| `pctX` / `pctY` | `:352-353` `pctPx(v,1920/1080)` | `pctPx(v, W/H)` | — |
| 贴纸 clamp | `:773` `1920-280-32` | `W - sticker - 32*k` | 宽 |
| 弹幕 y 映射 | `:1272` `(d.y/1920)*540` | `(d.y/W)*(540*k)` | 宽 |
| radial / linear 圆心 | `:1614-1615` `960,540` / `1920,1080` | `W/2,H/2` / `W,H` | — |
| camera 映射 | `:1227` `ox*1920/100` | `ox*W/100` | — |
| `#bg-video` 模糊 | `:28` `blur(28px)` | CSS 变量 `--bg-blur`（由 `blur_scale` 推） | — |

> 把 `W/H/k/kh` 一次性注入 `_canvas`，模板只做代入。**验收条件：grep 不到裸 `1920` / `1080` 的字面量坐标。**

### B3 WebUI 编辑器参数化

- `PolishPreview.vue:719-721` 舞台 `aspect-ratio` 由画布方向驱动；竖屏同时收窄 `max-width`
  （否则 9:16 在宽屏上会顶高撞顶）。
- `:737` `.overlay-space` `1920×1080` → `W×H`；
  `:653` `overlayScale = stageW/1920` → **`min(stageW/W, stageH/H)`**（只除宽度在竖屏下会算错缩放）。
- `:742` `.danmaku-layer`、`:744` `.captions` 同步参数化。
- `:600-601,613-614` wasm 真渲染路径的 1920×1080 —— **wgpu 缺口，不改**，加注释标注。
- `engine.js:140-148` 贴纸常量与 `clampSticker` 边界参数化；`:583-585` 弹幕 y；`:627-640` 贴纸 pct→px；
  A3 的 `VIS` 屏蔽分支删除；A4 的 `manual` 分支新增。
- **唯一换算入口**：新建 `webui/src/polish/canvas.js` 导出 `{W,H,k,kh,fit}`，由 `cfg._canvas` 构建，
  `engine.js` / `PolishPreview.vue` 都从这里取，杜绝第三套常量。

### B4 导出 + 封面

- **导出不需要新参数**：分辨率由 `[canvas]` 决定，保存项目时落盘。
  导出弹窗（`PolishView.vue:261-273`）显示只读摘要："将导出 1080×1920（9:16）"。
- **封面恒横版 1920×1080**（2026-09-20 用户要求，推翻 2026-09-19 的"跟随画布方向"）：
  平台封面就是横的，画布尺寸与项目方向解耦 —— `COVER_CAPTURE_W/H` 常量保留，
  `cover.html` 的 `__COVER_W__/__COVER_H__` 仍由 `cover.rs` 注入（值恒 1920/1080），
  `cmd_cover` 的 Chrome 窗口 / CDP clip 同值。
  **素材原比例**由 `#frame img` 的 `object-fit:contain` 保证（竖屏素材左右留边、露出背景层），
  不是靠改画布尺寸。设计器（`CoverDesigner.vue`）舞台 / stageScale 基准 / 安全区全按横版。
  钉子测试 `cover_html_page_size_is_always_landscape`（三种 orientation 都必须是 1920×1080）。
- ⚠️ **设计器的 `.stage-video` 必须显式 `background:transparent; border-radius:0`**（2026-09-20 用户报
  "设计器里素材左右两条黑边、出片正常"）：`contain` 让出的留白区由**元素自身背景**填充，而
  `webui/src/styles/app.css` 有一条**全局元素选择器** `video { … background:#000; border-radius:var(--r-sm) }`
  （给剪辑卡片缩略图用的）。`.stage-video` 是类选择器、只覆盖自己写过的属性 → 黑底**穿透**进来，
  且 `.stage-media`（z1）压住 `.stage-bg`（z0）把模糊背景层整个盖掉。出片模板用 `<img>` 且不引
  app.css，所以只有设计器有黑边 —— **不是 contain 的问题，也不是出片几何的问题，别回头去改它们**。
  契约测试 `node webui/scripts/check-cover-stage.mjs`（27 项 + 6 处变异自证）；真实产物 A/B
  （`target/tmp-cov9`：v2-prefix 留白 100% 纯黑 → v3-fixed 0% 纯黑，素材区逐像素一致）见交付说明。
- 输出文件名/去重（`pipeline.rs:28-52` `unique_output_path`）不动。

### B5 竖画布 × 横素材：对称支持（已定）

横素材 contain 进 9:16 画布 → 上下留白，位置变成 **上/中/下** + 同样的 `bg_mode` 三选。
`[stage] position` 已含 `top/center/bottom`，`fit(源比例, 画布, position)` 一处实现覆盖两个方向。
⚠️ 必须同时把 `object-fit:cover` 切成 `contain`（§1.4 第 6 条），否则会裁掉画面而不是留白。

### B6 入口：复用同一编辑器（已定）

不做独立竖屏模式。在编辑器加「画布方向」开关（放 `PolishView.vue` 右栏 `.gen-block` 里，
与「画面构成」同组或相邻），复用全部面板（时间轴、封面、贴纸、字幕）。

---

## 六、分阶段实施

| 阶段 | 内容 | 风险 | 状态 |
|---|---|---|---|
| **P0** | 配置契约 + SSOT：`CanvasConfig` / `StageConfig` / `StickerConfig` / `resolve()` / 注入 `_canvas` / PUT 白名单 / 净化 + 单测 / `vertical_position` 提升 + 兼容读取 | 低（**不改任何现有行为**） | **✅ 已完成** |
| **P1** | Part A 画面构成：位置 left/center/right + 背景 black/blur/color + `scale` + 「画面构成」UI 分组 | 低 | **✅ 已完成** |
| **P2** | A3 特效解禁（删 `VIS` 块，两处同步）+ A4 贴纸 auto/manual（含 placement 主画面障碍）+ 删竖屏贴纸强制覆盖 | 中 | **✅ 已完成** |
| **P3** | Part B 画布参数化：模板 + chrome + placement + 前端舞台 + `canvas.js` + **A5 字幕限宽折行（canvas 与两处 DOM 预览共用 `wrapCaptionParts`）** + 画布方向开关进 UI | 高（常量面最大，靠 grep 验收） | **✅ 已完成** |
| **P4** | Part B 收尾：竖画布×横素材对称适配 + 导出摘要 + 画布变更提示重跑 process | 中 | **部分**（2026-09-21：canvas 出片路径的**视频层摆位 + 运镜门控**已落地并逐帧实证，见文末；导出摘要 / 画布变更提示仍未做。~~P2 遗留：`main_frame_rect` 坐标系守卫~~ 已在 P3 放开；~~封面"固定 1920×1080"提示~~ 封面恒横版，无需提示） |
| **P5** | 一致性测试 + 冒烟矩阵 | — | 部分（模板消费面钉子已进 `canvas_stage_tests`；**竖屏"衬底"另有 Node 契约 + 真实像素 A/B**，见下） |

### 已落地清单（P0 + P1，仅记录事实）

**P0 — 契约与单一真相源（不改任何现有行为）**
- `project/config.rs`：新增 `CanvasConfig` / `StageConfig` / `StickerConfig` / `StageFit` +
  `ProjectFile::resolve_stage()` / `canvas_contract()`；`example()` 与 `to_toml_commented()`（含段头注释）同步；
  `vertical_position` 的兼容读取就位（显式 `[stage] position` 优先于老字段）。
- `render/server.rs::build_html`：注入 `cfg._canvas`（画布尺寸 / 摆放 / 字幕安全宽 / 贴纸落点）。
  **第二份竖屏检测已删除** —— `api/polish/` 的预览注入改读同一个 `canvas_contract()`。
- `api/polish/`：PUT 白名单加 `canvas` / `stage` / `sticker` 三个分支（漏了就是"保存不报错但值没了"）。
- 14 条新单测，全绿。含四类钉子：老工程（无新表）解析 = 改造前行为；
  横画布字幕安全宽 = `1680 / 120`；竖素材 + 全 auto = `axis=x / position=left / bg=blur`；
  **模板消费面与契约字段、class、CSS 变量逐一对齐**（这类错不编译报错，只会"改了没反应"）。

**P1 — 横画布下的位置 + 背景**
- `templates/index.html`：新增 `#fit-box`（主画面按比例就位），`#video-wrap` 由
  `axis-fill|axis-x|axis-y` + `stage-left|center|right|top|bottom` 驱动，
  背景 `none/black/color/blur` 由 `_canvas.stage.bg` 驱动（DOM 层 + **出片 canvas 层两处**，
  见 §A2 的 ⚠️）。**老的 `.portrait` 布局整体删除**。
- `PolishPreview.vue`：同一套规则 + `bgOn` 门控背景层（拿不到 `_canvas` 时退回旧竖屏规则）。
- `PolishView.vue`：新增「画面构成」卡（位置 / 背景 / 纯色 / 模糊强度 / 缩放 / 留白 +
  一段解释性提示：为什么有时调了没效果）。

### 与计划的偏差（有意为之，已落地）

1. **背景模糊层的放大倍数 1.15 → 1.187**：改为复用 `cover::cover_blur_css`（1.0 = 28px，
   放大 = `1 + blur/150`），与封面同一套换算。模糊层上看不出差别，
   但**竖素材存量项目不再逐像素一致**；横素材 / 全 auto 的项目仍是逐像素一致。
2. **`scale` 挂在 `#fit-box` 而不是 `#source-video`**：`#source-video` 的 transform 归 GSAP 运镜所有，
   挂同一元素会互相覆盖。副作用是主画面缩放与 `shake` / `blur`（打在 `#video-wrap`）解耦。
3. **画布方向开关**：`[canvas] orientation` 已能存能读。UI 开关**已进**「生成与导出 → 画面构成」
   页签（用户要求）。P3 落地后切方向即全链路生效：重取配置拿到新 `_canvas` 后，
   预览舞台 / 出片窗口 / 模板与引擎几何全部跟随（原「参数化在下一阶段生效」的提示已删）。

### P2 已落地清单
**A3 特效解禁（只屏蔽运镜）**

- **模板**：删掉事件调度处的 `VIS` 表 + `return`（原 `index.html:956-966`）。各 handler 内部本来
  就会调 `fxSfx(ev)`，所以音效不受影响。保留 `camEvents` 段的 `CFG.vertical` 两处（竖屏不做运镜）。
- **`engine.js`**：它的一刀切不在表里而在**逐 getter 上**（`idx.vertical ? null : ...`），
  逐条去掉 `stutter` / `split` / `zoom_bounce` / `pulse` / `shake` / `blur` / `colorShift` 的竖屏门控，
  只留 `cameraAt` 顶部的运镜门控。
- ⚠️ **连带恢复了「字幕联动」**：原 `return` 把后面的"当前字幕缩放 + 上移强调"也一起跳过了，
  现在竖屏也会跑。这符合决策 6（"其余一律通过"），但属于**行为变化**，需目视确认。

**A4 贴纸落点 auto / manual**

- **渲染侧分支（两条路径各一份 `stickerAnchor`，同构）**：
  - `templates/index.html`：新增 `stickerAnchor(ev)`，`handlers.sticker` 改用它算左上角。
    ⚠️ 这里只改**一处**就够 —— 出片 canvas 路径的 `getActiveSticker` 回读的是 DOM 的
    `style.left/top`（`:1369-1370`），所以 DOM 与 canvas 天然同源。
  - `engine.js`：新增 `stickerAnchor(idx, ev)`，`stickersAt` 改用它。
  - `manual` = 忽略事件 x/y，固定到 `_canvas.sticker.position`；`auto` = 沿用事件 x/y。
    两种模式都保留入场动画与出框 clamp（"固定"指落点固定，不是取消动画）。
  - 顺带把「事件缺 x/y」从 `NaN`（写进 CSS 就成了 `left: NaNpx`）改成回落 50%。
- **删掉竖屏强制覆盖（三处）**：模板的 `ev.x='50%'` / 15%-85% 交替、`engine.js` 的
  `entry.xOverride`（连带 `sideToggle`、`idx.verticalPosition` 一起变死代码，已删）、
  `yekitrim-render-core/src/timeline.rs` 的 `if vertical { ... }`。
- **UI**：`PolishView.vue` 贴纸来源分组下新增「贴纸落点」——`自动避让`/`固定落点` 二选一，
  手动时出现九宫格快选（`.grid9`）+ X/Y 数字输入，值即 `"x% y%"`。拖拽放置仍列为后续增强。

**A4b 主画面障碍（`placement.rs`）**

- 新增 `ObstacleKind::MainFrame`（权重 2.5，略低于 `Liver` 的 3.0：非要压的话，
  压在主画面的非人物区域比压住主播好）。
- 新增 `main_frame_rect()`：横画布×竖素材（`axis=x`）时视频只占一侧一条，
  原先贴纸会被放到主视频身上；把它计入障碍后 `auto` 才算真的自动避让。
- **三条守卫，缺一条就会反向变差**（都是实测踩出来的，不是预防性编码）：
  1. **`axis = fill` 一律返回 `None`** —— 整帧都是画面，拿它当障碍等于"处处不安全"，
     会把所有贴纸推进 relaxed 兜底。改造前的横素材项目全是 fill → **逐像素零回归**。
  2. **边带至少要 `STICKER_SIZE + STICKER_MARGIN`**（= 311px）才计入 ——
     主画面几乎铺满但没完全铺满时（`scale=3` 只剩 48px 侧边、4:3 源显式居中只剩 240px 边），
     加上障碍会让**所有**候选都相交，比不加还差。
  3. **坐标系守卫**：`canvas != 1920×1080` 一律返回 `None`。本模块的候选网格 / 字幕带 /
     弹幕带全是 1920×1080 硬编码，竖画布下算出来的矩形与网格不同坐标系，
     钳制（`clamp_to_frame` 用的是 `FRAME_H=1080`，会把竖画布 1920 高截断）与
     "还放得下吗"的判断都无意义。**P3 参数化 `FRAME_W/FRAME_H` 时一并放开。**
     几何部分已拆到 `main_frame_geometry()` 并单测覆盖，P4 接竖画布时有现成的正确性依据。
- 源视频尺寸走 `ffprobe`（`render::get_video_size`），取不到就退化成不计入（不失败）。
  日志里障碍摘要改成先拼串再打印 —— 塞进 `format!` 时可选的那一项会让占位符数量对不上。

### P3 已落地清单（画布参数化 + A5 折行）

**P3-A 渲染侧（Chrome 路径）**

- `render/context.rs`：`RenderContext` 尺寸改由 `config.canvas_size()` 推导（原来硬编码 1920×1080）。
- `render/chrome.rs`：`ChromeRenderer::launch(size)` 接收画布尺寸，`--window-size` / `window_size`
  动态构建；`ChromeBackend` 持有 `canvas` 并在 `prepare()` 时从 ctx 同步。
  `pipeline.rs::export_sound_data` 与 `cover.rs` 仍固定 1920×1080（声音数据非视觉、封面刻意不动）。

**P3-B placement.rs 坐标系参数化（含放开 P2 遗留守卫）**

- 新增 `pub struct Frame { w, h }`：`k() = w/1920`、`kh() = h/1080`、`sticker_size() = 280*k`、
  `margin() = 32*k`、`float_up() = 18*k`。`FRAME_W/H/STICKER_SIZE/...` 保留为**基准常量**。
- 全部几何签名改收 `f: Frame`：`Rect::from_points / gap_to_frame_edge / clamp_to_frame`、
  `parse_origin / caption_band / danmaku_band / main_frame_rect / Pick::x_pct / y_pct`、
  `sticker_box / probe / center_dist / central_pick / pick_sticker_position`。
  调用方 `process/mod.rs` 的 `StickerObstacles` 增加 `frame` 字段并全线穿参。
- **P2 的坐标系守卫（canvas≠1920×1080 返回 None）已删除**（`main_frame_rect` 内部自建 Frame）。
- 新增竖画布钉子测试：`portrait_canvas_main_frame_is_now_considered`、
  `sticker_avoids_main_frame_on_portrait_canvas`。

**P3-A 模板参数化（`templates/index.html`）**

- 顶部新增 SSOT 全局：`CW/CH/CK/CKH`（由 `CFG._canvas` 推导；横屏 k=kh=1 → 逐像素不变）。
- DOM 层尺寸由 JS 代入：`#overlay`、`#danmaku`（高 = 540*CK）、`#emoji-layer`、
  `#captions`（bottom 80*kh / left 120*k / 宽 CW−240*k）、`.sticker-img`（280*k，注入 style 覆盖）。
- 预览舞台比例跟随画布：`#player` aspectRatio + `#player-wrap` 宽度公式（capture/record 全屏模式不受影响）。
- 出片 canvas（`ensureCanvas` 的 `_polish_cc`）宽高 = CW×CH —— 导出帧尺寸即画布尺寸。
- 几何换算全部参数化：`r()` 缩放、`pctX/pctY`、弹幕 y（`(d.y/1920)*540*CK`）与滚动 x、
  贴纸 clamp / emoji 回退尺寸 / 上浮 −18*CK、相机 `ox*CW/100, oy*CH/100`、
  `renderFrame` 的 clear/draw/fill、径向渐变圆心、字幕基线 `cy = CH − 120*CKH`。
- grep 验收：模板内裸 1920/1080 坐标字面量仅剩 CSS 基准值（被 JS 覆盖）、SSOT 常量、
  `d.y/1920` 数据刻度（两处，与弹幕数据坐标系绑定，属预期）。

**P3-C WebUI 参数化**

- 新增 `webui/src/polish/canvas.js`：`canvasOf()`（`_canvas` → `{W,H,k,kh}`）+
  `stickerSize / stickerMargin / stickerFloatUp`，WebUI 侧唯一来源。
- `engine.js`：`idx.cv` 由 `canvasOf(idx.canvas)` 推导；`cameraAt` 百分比→像素、`danmakuAt`、
  `stickersAt`（clamp / 盒子尺寸 / 上浮）全部走 `idx.cv`；原 `19.2 / 10.8` 与 `STICKER_W/H/MARGIN` 常量删除。
- `PolishPreview.vue`：舞台 `--stage-ar / --stage-ratio`、虚拟 overlay `--ov-w/--ov-h`、
  弹幕带 `--dm-h`、字幕带 `--cap-bottom/--cap-left/--cap-w`、贴纸 `--sticker-s/--sticker-emoji`
  全部由 `cv` 注入 CSS 变量；`overlayScale = stageW / W`；wasm 画布尺寸与 `init_renderer` 入参跟随。
- `PolishView.vue`：画面尺寸开关提示更新（切方向即全链路生效，原「下一阶段生效」提示删除）。

**P3-D A5 字幕限宽折行**

- 新函数 `wrapCaptionParts(parts, maxLineW, measure)`（模板与 `engine.js` 各一份，同构同算法；
  模板为出片权威路径）：逐字符贪心（中文无空格）、高亮分段可跨行断开再按 hl 合并、
  安全宽 `maxLineW`（调用方传 `画布宽*0.875`，与 Rust `caption_band` 的 0.0625/0.9375 一致）、
  最多 3 行、装不下按 `maxLineW*3/总宽` 缩字号（下限 0.7）。返回 `{ lines, scale }`。
- 三处接入：① 模板 `renderFrame` canvas 出片（多行居中绘制、行距 1.5em、描边宽同步缩、
  保留事件联动 yOff）；② 模板 DOM 预览（`.cap-line` 块级 nowrap 行，禁浏览器自动换行，
  scale<1 时 inline 字号）；③ `PolishPreview.vue`（`capRowsWrapped` computed，行内 nowrap，
  字号/描边宽按 fontScale 缩）。`.cap` 水平 padding 44px → 0（行宽即盒宽，nowrap 溢出才对称）。

**P3 验证记录**：`cargo check --workspace --all-targets` 0 错误；
`cargo test -p yekitrim-polish -p yekitrim-render-core --lib` 162+22 全绿；
`cargo test -p yekitrim-server --lib` 132 全绿；模板 3 段内联 JS `node --check` 通过；vite build 通过。

---

## 七、明确不做 / 非目标

- **wgpu / native 路径不修**（`RenderContext.vertical` 死字段、`SOURCE_VERT` 无 dst-rect、
  `render-frame` 接口 `polish.rs:1034` 在竖屏下仍是拉伸画面）。`--features wgpu-render` 关闭时该接口返回 501，
  不影响默认路径。
- ~~**封面不跟随画布方向**~~（2026-09-20 用户要求改回**恒横版 1920×1080**，见 B4）。
- **字幕/弹幕不做避让重排**（已定）：弹幕**完全不动**（不折行 / 不限宽 / 不加配置，见 A6）；
  字幕只做**限宽折行**（A5：`W*0.875` + 最多 3 行），不做"避开主画面"的位置重排。
- 不做分辨率档位、不做自定义宽高（只给画布方向）。
- 不改 fps（30）与编码器探测策略、不改 `concat`/`mux` 的 `-c copy`。
- 不动 `yekitrim-core` 切片管线与 `publish_config.json`。
- 不做"竖素材下运镜放开"（若将来要，改判据一行，见 A3）。

---

## 八、风险与坑

1. **PUT 白名单静默丢弃**（`polish.rs:1260`）：最易踩。新字段必须显式收 + **必须有落盘断言测试**。
2. **`deny_unknown_fields` + `flatten extra`**：新字段必须显式声明 + `#[serde(default)]`。
3. **预览 ≠ 成片两套实现**：`engine.js` ↔ `index.html` 必须同步；A3/A4 两处都是双改。
4. **竖屏检测有两份**（`server.rs:243` / `polish.rs:1007`）：必须收敛，否则改了画布方向有一边不跟随。
5. **`process` 落点是持久化的**：改画布后旧项目 segments 的贴纸坐标需重跑 process；
   `placement.rs` 的 `STICKER_MARGIN` / `FLOAT_UP` / `GRID_STEP`（`:31-38`）也是横屏基准。
6. **`.source-video` 的 `object-fit:cover`**：比例不一致时必须切 `contain`，否则裁画面。
7. **`#captions` 1680px 在 1080 宽画布上溢出**：必须随 k 缩放（"沿用"≠"不缩放"）。
8. **`layout.vertical_position` 兼容**：提升字段时保留一次性映射，否则老项目位置回默认。
9. **`chrome.rs` 的 `--window-size`** 对 canvas 无强制力但影响页面布局/视口，改了画布方向要一起改。
10. **A4 的 manual 点与 `yOff = -18`**：floatUp 上浮量会让固定点轻微偏移，clamp 时要算进去
    （`placement.rs:35` 已有 `FLOAT_UP`）——落地时确认渲染侧也算了。
11. **字幕只按 k 缩放是不够的**（A5）：第 7 条说的溢出只是表象 —— 出片路径（canvas `renderFrame`）
    **根本没有折行能力**，必须补 `wrapCaptionParts`，而且**预览与成片要共用同一套断点**。
    今天"预览自动折行、成片不折"已是既有不一致，竖画布会把它放大成"预览正常、成片文字跑出屏幕"。
12. **折行在两条路径上的测量差异**：canvas 的 `lineWidth` 描边**占宽**、DOM 的 `-webkit-text-stroke` **不占宽**；
    另外 `.cap` 在模板里是 `absolute`（多条 active 字幕叠在同一处）、在 `PolishPreview.vue` 里是 `relative`（按流堆叠）。
    统一**以 canvas 侧语义为准**（A5 规格表）。
13. **背景层是"两套渲染路径"的字段**（2026-09-20 踩过）：DOM 的 `#bg-video-wrap` 只服务预览/`yekitrim polish preview`，
    **出片（`?capture=1` + `__CAPTURE_CHUNK` → `renderFrame`）是纯 canvas**。
    只把 `stage.bg` 消费在 DOM 上，表现就是"预览里有模糊底、成片被 shake 挪开露纯黑"——
    这类错**不报错、不崩溃**，只有逐帧像素能发现。新增任何"底色/衬底"类字段都要问一句：
    **出片路径谁画？**（钉子在 §九.7 与 `template_stage_hooks_match_contract` 的
    "先于视频层绘制"断言。）
    **摆放与运镜同理**：`_canvas.stage` 与 `CFG.vertical` 只消费在 DOM/CSS 上 = 成片与预览静默分叉
    —— 2026-09-21 的「竖素材被拉成宽屏 + 运镜没禁」就是这一条的两次（见 §九.8 与文末）。
14. **预览模糊半径 ≠ 出片模糊半径（已知偏差，未修）**：`blur_px` 是**画布 px**（出片直接用它），
    DOM 预览把它当**布局 px**（`.stage` / `#player` 被 CSS 缩到视口大小），
    所以预览看起来比成片更糊（差一个 `显示宽 / 画布宽` 的系数）。要收敛就在预览侧乘 `overlayScale`，
    属于"看起来更准"而非"哪里错了"，改前先确认用户口径。

---

## 九、验证

1. `cargo check` / `clippy`（0 新警告）+ `cargo test`（`yekitrim-polish`、`yekitrim-server`）。
2. 新增单测：
   - `CanvasConfig::resolve()` 两个方向；
   - `[canvas]`/`[stage]`/`[sticker]` 缺键时 `Default`（= 现状行为）+ 非法值净化；
   - PUT 落盘断言（镜像 `polish.rs:1662-1713`：值真写进 toml、兄弟键与段内注释不丢）；
   - `vertical_position` → `stage.position` 的兼容映射；
   - **模板/前端常量一致性**（grep 断言不再出现裸 `1920`/`1080` 坐标）；
   - **字幕折行快照**（A5）：给定 `parts` + `maxLineW` 断言断点数与断点位置
     （覆盖：高亮跨行、无空格长串、标点优先、`hl` 不丢）；**短字幕必须 1 行且几何与今天一致**。
   - 存量回归：`position="auto"` + `bg_mode="auto"` 解析出的 fit 与今天逐字段一致。
3. `cd webui && npm run build`。
4. 冒烟矩阵：`{横画布, 竖画布} × {横素材, 竖素材} × {black, blur} × {左/中/右 或 上/中/下} × {贴纸 auto, manual} × {字幕 短句, 30 字超宽}`。
   字幕专项：最长 30 字（无标点兜底切分）在横/竖画布下都**不出屏**（≤3 行）。
5. 产物校验：`ffprobe` 断言 1920×1080 / 1080×1920；封面断言**仍为 1920×1080**（刻意不跟随）。
6. 回归重点：
   - **存量横画布项目出片应与改造前逐帧一致**（`auto` + `auto`）——
     背景层的门控是 `axis=="fill" && !portrait`，横画布铺满仍然是 `bg=none`（不画 = 透明 = JPEG 黑，
     与改造前逐字节同源）；
   - **折行的零回归钉子**：短字幕（`totalW ≤ maxLineW`）必须是单行，行位置与字号与改造前逐像素一致（A5）；
   - 竖素材项目的视觉特效从"全禁用"变为可用（**行为变化，需目视确认**，尤其 `shake`/`blur`/`split`/`stutter`）。
7. 竖画布"底色"专项（2026-09-20 用户报告）：
   - 单测：`portrait_canvas_fill_still_paints_backdrop`（竖画布同比 + 显式位置 → `axis=fill` 且
     `bg=blur/black/color` 都成立；横画布对照仍是 `none`）；
   - 模板契约：`template_stage_hooks_match_contract` 断言出片 canvas 路径有背景层消费面
     （`drawStageBackdrop` / `stageCoverRect` / `STAGE_BG`）且**先于视频层绘制**；
   - 像素实证：真实项目出片后在 `shake` 帧上取四角像素，断言**不是纯黑**（`blur` 底色）。
8. 竖素材"被拉成宽屏 + 运镜没禁"专项（2026-09-21 用户报告，见文末）：
   - 模板契约：`check-stage-backdrop.mjs` 的 ③c 段 —— **独立复刻** `stageDrawRect` 求期望矩形，
     覆盖 `fill`/`x`/`y` 三支 + `position` 三档 + `padding` + `scale`，其中
     **「`axis=fill` + 无 padding + 比例相同 → 必须精确等于整块画布」是存量零回归的判据**；
     外加「改回拉伸」「去掉 CTM 摆位」两处变异自证。`check-handheld.mjs` 钉竖素材
     `zoom=1 / handheld=0`（与 `engine.js::cameraAt` 同口径）。
   - 像素实证：`VTEDIT_FIT_DIR=<竖素材工程> cargo test -p yekitrim-polish --test portrait_stage_fit -- --ignored --nocapture`
     —— 真模板 + **三段变异**（硬铺满 / `if(false)` 去运镜门 / `STAGE_BG='none'`），逐帧读白描边 bbox
     （= 内容框）、亮绿块 bbox 与占比、框外亮度与纯黑占比、逐帧占比稳定性。
   - 真出片 A/B（release 二进制，各 3 个探针工程）：**横素材×横画布 前后 SHA-256 完全相同**
     （`815F331C…6ED1`，12,530,137 B）—— 逐字节零回归；竖素材两向的绿块占比
     52.0% → 7.92%（横画布）/ 52.6% → 24.91%（竖画布，= 1.34² 等比）。

---

## 十、已定稿的决策

| # | 问题 | 结论 |
|---|---|---|
| 1 | 分辨率档位 | **只给 1080×1920**（横 1920×1080 / 竖 1080×1920）。`[canvas]` 只留 `orientation`，宽高推导 |
| 2 | 竖画布 × 横素材 | **对称支持**位置（上/中/下）+ 背景（black/blur/color） |
| 3 | 竖画布下字幕 | **限宽 + 多行**：安全宽 `W*0.875`（横屏 = 今天 `1680/120`，**零回归**），超宽折行、上限 3 行、超 3 行按比例缩字号；多条按行堆叠；不做避让重排 |
| 3b | 竖画布下弹幕 | **完全不动**：不折行 / 不限宽 / 不新增配置；只跟随 §B2 的统一参数化 |
| 4 | 入口形态 | **复用**同一编辑器，加「画布方向」开关；不做独立竖屏模式 |
| 5 | 封面 | **不跟随**画布方向，恒 1920×1080（UI 上明示，避免误判为 bug） |
| 6 | 竖素材下的特效 | **只屏蔽运镜（camera）**，其余一律通过（实测确认其余作用对象本来就正确，见 §1.3） |
| 7 | 贴纸落点（新增） | 加 `auto`（现状，安全区算法）/ `manual`（固定一点，电台/竖屏常用）；渲染时分支，process 照常算 |
| 8 | 竖画布下字幕字号 | **不缩放**（沿用 `[style.subtitle] size` 绝对值，默认 72px）；溢出**靠折行**解决，不靠缩字号 |
| 9 | 贴纸尺寸基准（2026-09-20 用户要求） | **画布短边**：`sticker_scale = min(W,H)/1080`（`placement.rs` `Frame::sticker_scale()`）。竖屏也是 280px；**不要**改成 `W/1920` 或"按画布等比缩小"（用户口径是"正常大小"=短边基准）。四条路径同源：`placement.rs` / `index.html` 的 `SK` / `canvas.js::stickerScale` / `PolishPreview.vue`。字幕、弹幕字号**不**跟着缩放 |
| 10 | 固定落点（manual）的锚点 | `[sticker] position` = **盒子中心**百分比（点选就是"把贴纸中心放这"）；事件 `ev.x/ev.y`（auto/placement）仍是**盒子左上角**。**两者不统一**。钳制顺序 = **先钳进安全区 → 再叠加上浮量** |
| 11 | 固定落点的交互（2026-09-20 用户要求） | **草稿态**：进「固定」= 进编辑态，点预览只写草稿，必须「保存落点」才落盘（「取消」丢弃；保存**成功**才退出编辑态）；切回「自动」丢草稿；与框选互斥；切片段清草稿。**不再是**常驻点选态、也不再只有数字输入 + 九宫格 |
| 12 | 设计器素材左右黑边（2026-09-20 用户报告） | **不是 contain、不是出片几何问题**：`app.css` 的全局 `video { background:#000; border-radius:var(--r-sm) }` 穿透到 `.stage-video`，填满 contain 让出的留白并盖住 `.stage-bg`。修法 = `.stage-video` 显式 `background:transparent; border-radius:0`（**必需覆盖**，见 §B4），钉子 `check-cover-stage.mjs` |
| 13 | 竖屏出片露黑边（2026-09-20 用户报告：震动等特效露出黑色） | 两个叠加成因：① `resolve_stage` 在 `axis=fill` 时**强制 `bg=none`**，把用户选的"模糊"吞掉 → 门控改为**画布方向**（`axis=="fill" && !portrait`），横画布铺满保持 `none`（零回归）；② 出片是 canvas `renderFrame` 逐帧绘制，**DOM 的 `#bg-video-wrap` 完全不参与导出**，`clearRect` 之后的透明区落进 JPEG 就是纯黑 → 在 `renderFrame` 里补 canvas 侧背景层（`black`/`color` 铺色；`blur` 用同一帧 bitmap 按 cover 铺满 + 模糊 + 放大 `blur_img_scale`），并断言它**画在视频层之前**。`bg_mode=auto` 语义不变；`bg=none` 保持"不画"（透明 = 黑，与改造前逐字节一致） |
| 14 | 竖素材出片被拉伸 + 运镜没禁（2026-09-21 用户报告） | **同一类成因的第三次**（出片走 canvas、不读 DOM），两条修法：① 视频层摆位搬进 canvas 路径 —— 新增顶层 `stageDrawRect(bw,bh,cw,ch,st)` 求画面矩形，`axis/position/scale/padding` 逐值对齐 CSS（`scale` 绕内容框盒心、`padding` 只改布局盒不裁剪），经 CTM 送入矩形，`drawImage` 的 dst **保持 `(0,0,CW,CH)` 字面量**（既有顺序钉子与两处变异锚在它上面）；`axis=fill` + 无 padding + 比例相同 → 精确 `(0,0,CW,CH)` = 横屏存量零回归。② 运镜门提到 `getActiveCamera` 开头（`CFG.vertical` 整条返回 `zoom=1/handheld=0`），**判据是源视频竖、与画布方向无关**；`yekitrim-render-core::evaluate_frame` 只归零 handheld（wgpu 路径，不在 Chrome 出片链路上），**不跟着改**。`contain` + 模糊关键帧当底的观感由 `bg_mode` 决定，本修法不动 `resolve_stage` |

**剩余开放项**（不阻塞 P0–P2，可实施中再定）
- ~~`manual` 点在编辑器里的交互形态：先数字输入 + 九宫格快选，**拖拽放置列为后续增强**。~~
  → 2026-09-20 定稿为**预览点选 + 草稿态**（决策表 11）；数字输入仍保留，九宫格快选已删。
- 竖素材下 `camera` 之外是否还有需要裁掉的个别特效：P2 目视确认后再定（当前按"一律通过"实现）。
- 封面设计器剩余项：GPU 路径（`yekitrim-render-core/src/timeline.rs`）**仍未接 `manual` 中心锚点**
  （需改函数签名 + 重生成 wasm 胶水）；WebUI 里的素材留白问题已修（决策表 12）。

---

## 关键参考文件

| 文件 | 用途 |
|---|---|
| `crates/yekitrim-polish/templates/index.html` | **Chrome 出片唯一真相**。`:26-35` 竖屏背景/位置 CSS、`:45-54` 图层绝对尺寸、`:352-353` `pctX/pctY`、`:447-463` `shake()/flash()`、`:583-593` 竖屏接线、`:707-734` `curZoom`+运镜、`:714,718` 竖屏禁运镜、**`:759-781` 贴纸 handler（`:773` clamp，`:776-780` 动画）**、`:793-896` 各特效 handler（作用对象见 §1.3）、**`:911-921` `VIS` 一刀切 + 贴纸强制覆盖（待删）**、`:1518-1531` canvas 尺寸、`:1541-1717` 逐帧绘制、**`:1635-1676` 字幕绘制（单行连排、无任何折行，A5 待改）**、`:1668-1676` `cap.yOffset`、`:618-629` 字幕样式注入、`:46-48` `#captions`/`.cap` CSS |
| `crates/yekitrim-polish/src/project/config.rs` | `ProjectFile`（`:5-64`）、`[cover] bg_mode/bg_color/blur_scale` 先例（`:303-321`）、`to_toml_commented`（`:413`）、`example`（`:630`） |
| `crates/yekitrim-polish/src/render/server.rs` | `build_html` 注入 cfg（`:235-300`）、竖屏检测（`:243-254`）、`save_config_handler` |
| `crates/yekitrim-server/src/api/polish/` | PUT 白名单（`:1260-1289`）、cover 背景白名单先例（`:616-627`）、落盘测试先例（`:1662-1713`）、竖屏注入（`:1007-1024`）、`render-frame`（`:1034`） |
| `crates/yekitrim-polish/src/render/context.rs` / `pipeline.rs` | `RenderContext`（`:9-43`）、ctx 构造（`:394-401`）、fps（`:88`）、ffmpeg 编码（`:415-432`） |
| `crates/yekitrim-polish/src/process/placement.rs` | 贴纸落点算法 + `FRAME_W/H`（`:28-29`）、`STICKER_SIZE`/`MARGIN`/`FLOAT_UP`/`GRID_STEP`（`:31-38`） |
| `crates/yekitrim-render-core/src/timeline.rs` | 竖屏贴纸覆盖/左右交替（`:488-565`，待删） |
| `crates/yekitrim-polish/src/cover.rs` + `templates/cover.html` | 封面尺寸（`__COVER_W__/__COVER_H__` 注入点、`cover.html:20,96`）—— **恒横版 1920×1080**（2026-09-20 起，不跟随画布方向；常量 `COVER_CAPTURE_W/H` 仍在） |
| `webui/src/components/PolishPreview.vue` | 舞台比例（`:719-721`）、overlay-space（`:737`）、danmaku-layer（`:742`）、`captions`/`.cap`（`:744-746`，A5 折行同步点，注意这里是 `position:relative` 与模板不同）、`overlayScale`（`:653`）、竖屏 class（`:129`）、`bgRef`（`:4`） |
| `webui/src/polish/engine.js` | 贴纸常量/clamp（`:140-148`）、弹幕 y（`:583-585`）、贴纸 pct→px（`:627-640`）、`subsAt` 只产出 `parts` 且**无折行**（`:522-549`，A5 要挂断行计算）、离屏测量先例（`:115-130`）、`VIS` 屏蔽分支（待删） |
| `webui/src/views/PolishView.vue` | `.gen-block` 分组挂载点（`:121-197`，贴纸来源 `:156-176`）、导出弹窗与 `doRender`（`:261-273, 1020-1025`） |
| `docs/plan/webui-polish-rebuild.md` | 同风格先例；`:15` 记录了 `vertical`/`vertical_position` 的原始注入设计 |


### 衬底（决策 13）的验证记录 —— 2026-09-20

用户口径：**精修竖屏时，震动一类特效会把主画面挪开、露出背景的黑色，需要在视频底下垫一层模糊**
（= 决策 13 的"竖屏出片露黑边"）。

**实现面**（当时已落地在 `templates/index.html` + `project/config.rs`，两道闸）：

1. `ProjectFile::resolve_stage`：`bg` 的门控是**画布方向**（`axis=="fill" && !portrait` → `none`），
   竖画布铺满也照 `bg_mode` 给衬底；横画布铺满保持不画（存量逐字节零回归）。
2. 出片 canvas 路径：`renderFrame` 里 `drawStageBackdrop(ctx, videoBitmap)` **画在视频层之前**
   （DOM 的 `#bg-video-wrap` 完全不参与导出，只改 DOM 就是"预览有底、成片纯黑"）。

**验证（本轮补齐，原计划 §九.7 的"像素实证"）**：

| 项 | 命令 | 结果 |
|---|---|---|
| 契约 + 变异自证 | `node webui/scripts/check-stage-backdrop.mjs` | 40 项全过；6 处变异（cover→contain、去 `none` 守卫、图层反序、模糊半径写死、去 save/restore、塞回裸 `fs`）**全部翻面** |
| 真实像素 A/B | `VTEDIT_BACKDROP_DIR=<竖屏工程> VTEDIT_BACKDROP_T=<被挪开的时刻> cargo test -p yekitrim-polish --test portrait_backdrop -- --ignored --nocapture` | 同帧同角、A/B 各 30 帧：**真模板最暗角 73.9**（模糊衬底）/ 变异体（`STAGE_BG` 变异常量成 `none`）**最暗角 0.0**；逐帧差最小 67.8、平均 72.2。探针工程每帧都带字幕 + 弹幕 + 一串瞬时特效（见下条 `fs` 崩溃） |
| 工程形态 | — | 画布 1080×1920（portrait）× 源 720×1280、`axis=x / position=center / bg=blur / 28px×1.187`（= 用户那个竖屏工程） |

> 注：上表是**当轮**计数。脚本此后有扩充（③b 裸变量 / ③c 视频层摆位 + 更多变异），
> 最新计数与新一轮实证见文末「2026-09-21」。

**踩坑记录（写测试时）**：探针里 Chrome 抓帧是同步的，而页面要访问的 HTTP server 跑在同一个 tokio
runtime 上 —— 直接在 `await` 里同步跑会把 runtime 线程占死、server 不回包，报一个与根因无关的
`The event waited for never came`。必须 `spawn_blocking`（生产路径同理，见 AGENTS.md 的
"同步 IO 绝不能留在 async 线程上"）。

⚠️ **改了 `templates/index.html` 必须重建二进制再重启 `yekitrim server`**：模板经 `include_str!`
进 `yekitrim-polish`（`render/server.rs::INDEX_HTML`），运行中的服务端出的仍是旧页面。
本轮用户报"还是黑的"就属于这种情况（当时跑的是 13:04 的构建，模板 14:22 才加上的衬底消费点）。

### 出片崩溃：`fs is not defined`（2026-09-20 用户日志）

用户贴出的失败日志：

```
ERROR yekitrim_cli::job: 任务失败: render 进程退出码: Some(1)
Error: Chunk: Render error: fs is not defined   job=0000-150630 kind=polish_render
```

**根因**：A5 字幕折行改造把字号挪进了逐条字幕（`baseSize` / `wrapped.scale`），
但 `renderFrame` 的字幕块里**留下了一句没被删掉的** `ctx.font = '700 ' + fs + ' ' + ff;`
—— `fs` 只在另外两个函数（全屏切换 / 弹幕测量）里各自 `var` 过，在这个作用域里是**未声明标识符**，
于是**只要这一帧有字幕就抛 `ReferenceError`**。出片是"一块失败 = 整个 job 失败"，
所以表现是**所有带字幕的片段都导不出来**（无字幕的片段反而不受影响 —— 这正是它能潜伏下来的原因）。

**修法**：删掉那句（基准字号 / 缩放后的字号在 forEach 里设，就在量宽之前）；
`ctx.textBaseline` / `lineJoin` / `cx` / `cy` 保持原位不动。
钉子：`node webui/scripts/check-stage-backdrop.mjs` 的 ③b 段（`renderFrame` 里不许出现裸 `fs`
+ 变异自证），以及像素探针 `crates/yekitrim-polish/tests/portrait_backdrop.rs` ——
它的探针工程现在**每一帧都有字幕**（含一条 30 字必折行的）+ 两条弹幕 + 一串瞬时特效，
所以这一段 30 帧全部渲染成功本身就是逐帧绘制路径的冒烟测试。

### UI 调整（用户要求，2026-09-19）

- 「画面构成」卡并入「生成与导出」卡，卡片头部标签页切换（导出 / 画面构成），
  任务状态框两个页签下都可见。
- 背景「纯黑」并入「纯色」：bg=none 与 black 渲染结果相同（露出 #player 黑底），按钮组只剩
  自动 / 纯色（默认 #000000）/ 模糊；存量 bg_mode=black 在界面显示为 纯色 + #000000，
  渲染侧不动。
- 画面尺寸（横屏/竖屏）开关进「画面构成」页签，写 [canvas] orientation（patchCanvas 与
  patchStage 同一提交链）；位置选项方向随画布联动（左中右 ↔ 上中下）。预览/出片尺寸
  参数化仍属 P3，UI 提示已写明。

### 竖素材出片的摆放 + 运镜（决策 14）—— 2026-09-21

用户原话：

> 对于竖屏的导出没有正常实现，具体为横屏时的效果显示为竖屏素材被拉伸成宽屏，而不是竖屏素材+模糊的
> 关键帧。而且对于竖屏素材没有禁用运镜，即使是横屏导出还是竖屏导出

**根因：两条都在同一条路径上** —— 出片走 canvas `renderFrame` 逐帧绘制、**不读 DOM**，而画布契约
（`_canvas.stage` 的摆放）与竖素材标记（`CFG.vertical` 的运镜门控）此前**只被 DOM/CSS 消费**：

1. 视频层是 `ctx.drawImage(videoBitmap, 0, 0, CW, CH)` —— **硬铺满整块画布**。竖素材进横画布
   （`axis=x`、720×1280 → 1920×1080）被横向拉成宽屏；预览侧 CSS 走 `contain`，所以**预览完全正常**。
2. `getActiveCamera` 只在循环末尾 `handheld: CFG.vertical ? 0.0 : handheld` —— **`zoom` 没被吃掉**：
   预览侧 `engine.js::cameraAt:310` 早就整条返回了，于是"预览不动、成片在缩放"。

两条都**不报错、不崩溃**（与决策 13 的衬底同一类），只有逐帧像素能发现。

**实现**（`crates/yekitrim-polish/templates/index.html`，CRLF 保持；模板内仍不许出现子串 `portrait`）：

| 改动 | 内容 |
|---|---|
| 新增顶层 `stageDrawRect(bw,bh,cw,ch,st)` | 画面矩形 = `axis/position/scale/padding` → 画布矩形。`axis=x` 按高度等比 + `position` 左/中/右，`axis=y` 按宽度等比 + 上/中/下（等比溢出时按 contain 截住 = CSS `max-width/height:100%`），`fill` = cover 居中；`padding` 只改布局盒、`scale` 绕**盒心** |
| `renderFrame` 视频层 | `var sr = stageDrawRect(...)`；运镜不动点按 `sr` 映射（`sr.x + cam.ox/CW*sr.w`）；在 `drawImage` 之前 `ctx.translate(sr.x, sr.y); ctx.scale(sr.w/CW, sr.h/CH);` —— dst **仍是 `(0,0,CW,CH)` 字面量** |
| `getActiveCamera` 开头 | `if (CFG.vertical) return { zoom: 1.0, ox:…, oy:…, handheld: 0.0 }`（与 `engine.js::cameraAt` 同口径）；循环末尾那句 `CFG.vertical ? 0.0 : handheld` 随之删掉 |

`resolve_stage` / `canvas_contract` / `engine.js` / `PolishPreview.vue` **一行未动** —— 缺陷只在
"出片这一步没消费契约"。

**验证**（三套探针工程：竖素材×横画布 / 竖素材×竖画布 / 横素材×横画布。自造 720×1280 探针源 =
整帧品红 + 最外圈 6px 白描边 + 正中央 360×640 亮绿块，均带三条字幕 + 一条整段 `camera zoom=近`；
release 二进制的 `before` / `after` 各跑一遍）：

| 项 | 结果 |
|---|---|
| 存量零回归 | 横素材×横画布 **前后 SHA-256 完全相同**（`815F331C…6ED1`，12,530,137 B） |
| 竖素材 × 横画布 | 修复前：绿块 **52.0%** + 白描边被推出画布（拉伸 + 运镜）；修复后：内容框 `x0 y0 608×1080`、绿块 **7.92%**（= 内容框面积 × 25%）、框外是模糊衬底、逐帧全稳 |
| 竖素材 × 竖画布 | 修复前 **52.6%**；修复后内容框铺满 1080×1920、绿块 **24.91%**（= 1.34² 等比） |
| 字幕出片 | 成片在下半屏取字幕色像素：修复后 `t=1.5 / 4.5 / 7.0` = 25856 / 35925 / 33963（`t=0.2` 无字幕时 = 1）—— 折行 + 绘制没被摆位改动打破 |
| 契约钉子 | `check-stage-backdrop.mjs` **73 项**（新增 ③c 段几何 + 「改回拉伸」「去掉 CTM 摆位」两处变异）/ `check-handheld.mjs` **63 项**（竖素材 `zoom` + `handheld` 归零 + 变异自证）/ `check-overlay` · `check-caption-wrap` · `check-sticker-anchor` 全过 |
| Rust 钉子 | `project::config::template_stage_hooks_match_contract` 新增三条消费面 needle + 两条顺序断言（摆放矩形在衬底之后、`drawImage` 之前）；`cargo test -p yekitrim-polish --lib` 179 全绿，`cargo check --workspace --all-targets` 0 warning |
| 像素探针（新） | `VTEDIT_FIT_DIR=<竖素材工程> cargo test -p yekitrim-polish --test portrait_stage_fit -- --ignored --nocapture`：真模板 18 帧全过（内容框 608×1080、纵横比 0.5630、绿块 7.87%、框外亮度 74.5 / 纯黑 0.00%），**三段变异全部翻面** —— 硬铺满 → 内容框 1920×1080 + 绿块 24.90%；`if(false)` 去运镜门 → 绿块 16.76%；`STAGE_BG='none'` → 框外纯黑 96.0% |

⚠️ 「硬铺满」变异体在真模板的运镜门加持下会把画面整个推出画布（白描边不可见），所以判据写成
「内容框变宽 **或** 白描边消失」+ 绿块 > 1.5× 期望 —— 这正是改造前那版二进制的实测形态
（`无白像素 + 绿块 52%`）。探针的期望矩形是**独立复刻**（Rust 侧再写一遍 `stageDrawRect`），
不读模板函数，避免自证。

#### 连带发现：既有的 `portrait_backdrop` 探针原来靠"被修掉的缺陷"在露底

同一轮把决策 13 的像素探针 `crates/yekitrim-polish/tests/portrait_backdrop.rs` 重跑，**发现它几乎哑了**：
30 帧里只剩 1 帧（`t=1.13s`）真让出衬底，其余帧四角都是主画面自己盖住的 —— A/B 比的是同一份像素，
差值天然 ≈ 0，判据 `AB_DELTA_MIN_LUMA=40` 于是在 `t=1.00s`（差 36.7）报红。

根因：探针工程的"画中画"当初是用 `[[segments.events]] type="camera" zoom="远"`（`zoom_wide=0.55`）
做的 —— 而**竖素材的运镜正是本轮被门控吃掉的那条**。2026-09-20 那轮"30 帧全部露底（逐帧差最小 67.8、
平均 72.2）"是**靠这个 bug 撑出来的**：运镜把画面缩成 0.55，四周才露出衬底。

处置（两处，**阈值一个没动**）：

| 面 | 改法 |
|---|---|
| 判据 | `AB_DELTA_MIN_LUMA` 仍 40，但只断在**真露底帧**上（变异体最暗角 ≤ 12，新常数 `EXPOSED_BACKDROP_MAX_LUMA`），另加一条「窗口里至少有一帧真露底」—— 否则报"探针在空转，把 `T` 挪到主画面真被推出画布的时刻"。四角被画面盖住的帧没有判据意义 |
| 探针工程 | 画中画改由 `[stage] scale = 0.55` 做（**结构性**露底、与运镜无关）；camera 事件留着跑那条被门控的代码路径 |

阈值 12 与判据 ① 是配套的：① 要求真模板四角 > 20，而四角被盖住的帧里 A/B 是同一份像素 → 变异体
那角也 ≳ 20（> 12）→ 被跳过；反过来只有真缺衬底的角才会掉到 12 以下。`noise` 这类全屏覆盖层把缺衬底
的角抬到 13~21 的帧同样被跳过 —— 它们不提供证据，也不需要。

重跑（新工程）：30 帧全过、**20/30 帧真露底**、这些帧差值最小 **71.1**；把 10 个被覆盖层抬亮的帧也算
进来则逐帧差最小 **67.8**、平均 **72.2** —— 与 2026-09-20 那轮记录**逐项一致**（`scale=0.55` 与
`zoom_wide=0.55` 是同一个取景），说明现在这份数字不再依赖任何被修掉的缺陷。收窄后的判据先在旧工程上
实跑过一次（1/30 帧露底 → 通过），两条路径都有实测。

⚠️ 过程纪律：验证用 `target/release` + 沙箱副本（用户 dev 实例占着 `target/debug`），
探针素材/工程/脚本一律在 `.workbuddy/perf/vertical/`（已 gitignore），**不碰线上数据目录**。

---

## 十、2026-10-02：画面构成改版 + 两条真像素探针复跑（本轮）

### 10.1 改了什么（口径）

「画面构成」按用户 2026-10-02 的拍板重做：**位置 = `position_pct`（0–100，界面五档吸附**
0/25/50/75/100**，默认 50 居中**，老枚举 `position` 只读兼容、`auto` 掉档为居中）、**轴只看比例**
（同比 → `fill`，否则 x/y ⇒ 不匹配的源从「裁切铺满」改成**适配留边**）、**背景删「自动」、缺省 = 模糊**、
**`padding` 字段保留但 UI 不再出现**、**竖屏画布三个开关 `portrait_{effects,camera,sfx}`（缺字段 = false = 关，
按整类事件屏蔽、事件本身不动）**。`resolve_stage` 仍是唯一推导，三处消费侧同批：
`templates/index.html::stageDrawRect`、`PolishPreview.vue` 的 `.fit-box` 修正、`tools/polish-to-dhampir.mjs`；
`process/placement.rs::main_frame_geometry`（贴纸避让）也跟着换到 `position_pct`。

### 10.2 复跑探针挖出来的**真缺陷**（转译器，已修）

`SR`/`FIT`（画面构成的唯一几何）**以前只由相机关键帧写进去**。而竖素材的运镜被整条门控掉
（`sourceIsVertical`）⇒ 那一层**一个关键帧都没有** ⇒ 主层停在 `scale=1 / offset=0`，
底座按「源 1:1 居中」画：**竖素材进竖画布（用户最常见的组合，源 720×1280 → 画布 1080×1920，同比！）
画面缩在中间、四周一圈黑**。实测：转译产物的 `transform = {x:0,y:0,scale:1}` 且 `keyframes = 0`。

修法：**0 键层写一组「中性相机」基准值** —— `cameraOffset(1.0, {x:0.5,y:0.5})` + `scale: FIT`
（只对 0 键的层做，有键的层逐字节不动 = 存量零回归）。修后实测：竖画布同比 → `{x:0,y:0,scale:1.5}`
（720×1280 × 1.5 = 1080×1920 正好铺满）；横画布 + 竖素材 → `{x:0,y:0,scale:0.84375}`（contain 居中，
内容框 607.5×1080 @ x=656.25）；`position_pct=0` → `x=-656.25` 贴左（位置真的跟着走了）。

⚠️ **修的时候自己踩了一次 `FIT²`**：`cameraOffset(zoom, origin)` 内部已经是 `scale = zoom * FIT`，
第一实参传 `FIT` 就等于把 `FIT` 预乘两次 —— 而**居中那一点恰好归零**（它自己的注释也记着这个坑），
所以只看中心看不出来；是 `position_pct=0` 的实测把它逼出来的（正确 `-656.25`）。

### 10.3 探针 harness 的两处过期（都修了）

1. **抽帧抽错了源**：`tests/common/mod.rs::render_range` 抽的是**源素材**，于是探针里那句
   「成片帧尺寸必须等于画布尺寸」**必然失败**（探针源 720×1280 vs 画布 1920×1080）。
   chrome 逐帧腿删掉之后没人再跑过这两条探针，所以一直没暴露。现在抽**成片** `_temp_video.mp4`。
2. **变异锚点落在语义位置之外**：B②「去掉竖素材运镜门」用 `replacen(..., 1)` 改的是转译器里
   **第一次**出现的 `if (sourceIsVertical === true) {` —— 那是**报告用的 NOTE**，运镜门在后面一处。
   于是变异体与真转译器**逐帧相同**、判据静默空转（实测 `运镜 0 条`）。改成
   `replace_first_after(code, "case 'camera': {", …)`。
3. **变异形态要在当前后端可表达**：B① 原来是「`w = bwid` 硬铺满」，等价于旧参照的
   `drawImage(…,0,0,CW,CH)`；但**底座的 `transform.scale` 是单值（等比）**，做不出各轴不等比的硬拉 ——
   那个变异会把画面整体推出画布（`offset.x=-1600`），绿块与描边一起消失。现在改成 **cover**
   （`FIT = max(width/srcW, height/srcH)`，= 本次改版之前的 `auto` 语义，底座可表达），判据只断几何。

### 10.4 复跑结果（2026-10-02，本机）

探针工程（自造，`.workbuddy/probe/stagefit/`，已 gitignore）：720×1280 品红 + 外圈 6px 白描边 +
中央 360×640 亮绿块，8s；`[canvas] orientation="landscape"` + `[stage] bg_mode="blur"` + 一条整段运镜。
（⚠️ 工程必须写顶层 `duration`，否则转译出的层 `end=0`、底座算不出帧数；`[source] subtitles` 是必填键。）

- `--test portrait_stage_fit`：**全部通过** —— 内容框 `x656 y0 608×1080`、纵横比 0.5630（源 0.5625）、
  绿块 7.89%（期望 7.91%）且逐帧恒定；三段变异全部翻面（几何失效 1264×1080 / 去运镜门 17.73% ≈ 2.25× /
  不画衬底 框外纯黑 99.5%）。
- `--test portrait_backdrop`：**全部通过** —— A（衬底开）四角亮度 86.0、B（衬底关）四角 0.0，
  **30/30 帧真露底**、A/B 差 86.0。

⚠️ 衬底那两条判据在 `portrait_stage_fit` 里**只在 `BACKDROP_ENABLED` 打开时才断**：转译器里它默认
`false`（底座侧长片 OOM 的已知限制，补的就是黑边），关着时**明说跳过**；衬底本身由 backdrop 探针专门量
（它自己把开关打开再 A/B）。

