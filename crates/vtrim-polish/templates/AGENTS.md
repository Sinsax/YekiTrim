# 出片模板（`templates/index.html`）— 真实渲染路径

> 范围：`crates/vtrim-polish/templates`/** ｜ 上游：`vtrim-polish::render`（`include_str!` 进二进制）、`webui/src/polish/engine.js`（浏览器预览的移植副本）｜ 验证：`node webui/scripts/check-overlay.mjs`

## 职责

`index.html` 是**出片的权威实现**：canvas 逐帧 `renderFrame` 绘制 + GSAP 预览，经
`include_str!` 编进 `vtrim-polish`（`render/server.rs::INDEX_HTML`），预览服务与出片共用同一份。

它是"事实来源"，不是"预览页面"：`webui/src/polish/engine.js` 是把这里的纯函数**逐字移植**到
前端的结果，两处必须一致 —— 漂移的表现就是「预览与成片不一样」，而且**不报错**。

## 入口

- `INDEX_HTML`（`crates/vtrim-polish/src/render/server.rs`）—— 模板进二进制的唯一入口。
- 与 `engine.js` 成对存在的顶层函数（改动必须成对）：
  - 覆盖层求值 `getOverlayState`（四档：`color` / `gradient` / `fade_in` / `fade_out`；**fade 两档 = 整窗
    线性斜坡、不叠 0.4s 包络**，所以分支必须排在包络之前）与色档助手（`overlay_fade_stop` / `overlayFadeStop`）；
  - `wrapCaptionParts`（字幕限宽折行，上限 `MAX_LINES = 3`、`MIN_RATIO = 0.7`、安全宽 = 画布宽 × 0.875）；
  - `getActiveCamera` / `cameraAt`（运镜；两句函数开头都有**竖素材门**：`CFG.vertical` → 整条返回
    `zoom=1 / handheld=0`）、`liverOrigin`（立绘原点，须与 Rust `grid_rect` 对齐）；
  - `stageDrawRect`（视频层摆放矩形，**出片专用、`engine.js` 没有对应实现**）与
    `drawStageBackdrop`（画面衬底：`blur` / `black` / `color` / `none` 四种模式）。

## 边界

- ⚠️ **改了 `templates/index.html` 必须重建二进制**：它经 `include_str!` 进 `vtrim-polish`，
  运行中的 `vtrim server` 端出的仍是旧页面 —— 「改完没反应」十有八九是没重建/没重启，不是代码没生效。
- ⚠️ **`cover.html` 同样经 `include_str!` 进 `cover.rs::COVER_HTML`**（改它也要重建二进制）。
  它有两个**给静态渲染通道**（Takumi，见 polish 卡末段）准备的占位：
  ① `:root{--fs:__FONT_SIZE__;…}` 那个**静态变量块** —— 变量的唯一来源曾是页面脚本的 `setProperty`，
  静态渲染器不跑 JS；② `__EXPORT_CSS__`（只填给静态那条，= 关掉入场动画）。
  ⚠️ 两条**都必须留**：`__EXPORT_CSS__` 空串、`--fs` 同值时，对当年那条 chrome 截图通道是逐字节中性的
  （那条通道**已删除**，这条中性性不再可测）；**删任何一个都会静默削弱静态通道**。
- ⚠️ **模板里的工具函数必须放在脚本顶层**：canvas 出片那一段是独立的 IIFE，看不到 `init()` 内部的东西。
  `evDur` 曾放在 `init()` 里、被 IIFE 的 `buildIndex()` 调用 → 每次出片都抛 `evDur is not defined`
  （GSAP 预览路径却完全正常，所以**只在出片时炸**）。新增给两条路径共用的函数时，放顶层并留注释。
  ⚠️ **2026-10-01 又踩了一次同款**：顶层新增的 `getVignette` 调了 `init()` 里的 `transientDur`
  → `Chunk: Render error: transientDur is not defined`，整块 chunk 全废。**顶层函数要用 `init()` 里的东西，
  就把那几行自包含地算一遍**（`getVignette` 现在自己算 `evDuration` + cap）。这正是"改完模板必须真跑一次出片"
  的第二条证据：静态检查没抓住，是对拍/真出片抓住的。
- ⚠️ **出片逐帧路径里任何 `ReferenceError` 都是"整块 chunk 失败 → 整个 render job 失败"**
  （实测：A5 折行改造留下的一句 `ctx.font = '700 ' + fs + ' ' + ff`，`fs` 在 `renderFrame` 作用域里
  没声明 → 日志 `Chunk: Render error: fs is not defined`，表现是**所有带字幕的片段都导不出来、
  无字幕的正常**）。改了 `renderFrame` 附近后**至少跑一次带字幕的出片**。
- ⚠️ **单色覆盖层一律「该色 → 同色零透明」**，**不能用 CSS 的 `transparent`**：canvas 渐变在
  **非预乘**空间插值，`#ff0000 → transparent` 中间会插出 `rgba(127,0,0,·)` 那种暗红 ——
  实测同一条红色覆盖层（opacity 0.3、同一个源帧、同一时刻）画面平均 R「184.12 → **178.82**」
  （红了反而变暗）；换成 `rgba(255,0,0,0)` 才是「184.12 → **193.66**」的干净淡出。
  而 CSS 那边两者**完全等价**（按预乘插值），所以写 `transparent` 恰好让**预览与出片不一致**。
- ⚠️ **片头黑场不再是内建表现**（`intro-black` 2026-09-22 整条退役）：模板里已无 `#intro-black` /
  `getIntroBlack()` / 那段 GSAP；片头那份是 `process::ensure_intro_fade_in` **只在初次后处理**补的一条
  `fade_in` 覆盖事件（`#000000` + `opacity 1`；判据 = `polish.toml` 的 `intro_fade_seeded`，
  用户删掉之后不再补）。**别往回加内建黑幕** —— 它没有手动入口、用户改不了，这正是退役的原因。
- ⚠️ **色档求值器必须保证返回 ≥2 档且无空串**：出片路径按 `addColorStop(i/(n-1))` 铺档，
  `n == 1` 是 `0/0 = NaN` 抛 `IndexSizeError`、空串色档抛 `SyntaxError`，
  两者都会**打断整个 chunk 的渲染**。四处实现（`vtrim-render-core/src/timeline.rs`、
  本模板、`engine.js`、`PolishPreview.vue`）以本模板为准。
- ⚠️ **`drawStageBackdrop` 必须画在视频层之前**：出片是 canvas 逐帧绘制，DOM 的 `#bg-video-wrap`
  **完全不参与导出** —— `clearRect` 之后的透明区落进 JPEG 就是纯黑。只把 `stage.bg` 消费在 DOM 上，
  表现就是「预览里有模糊底、成片被 shake / 画中画 / 手持摇摆挪开露纯黑」：**不报错、不崩溃，
  只有逐帧像素能发现**。模糊半径/倍率一律取契约里的值，**别写死 28px**；`none` 一个像素都不画。
- ⚠️ **视频层必须在出片路径里自己算摆放矩形**（`stageDrawRect`）：出片走 canvas、**不读 DOM** ——
  DOM 那层（`#video-wrap` / `#fit-box` / `.axis-*` / `.stage-*`）只服务预览。只在 CSS 上消费
  `_canvas.stage` 的话，`ctx.drawImage(videoBitmap, 0, 0, CW, CH)` 就是**硬铺满整块画布**：
  竖素材进横画布 → 成片"被拉成宽屏"（用户 2026-09-21 报的原文），预览里却完全正常、**不报错**。
  `axis`/`position`/`scale`/`padding` 逐值对齐 CSS 那套（`scale` 绕**内容框盒心**、`padding` 只改
  布局盒不裁剪）；`axis=fill` + 无 padding + 源比例 == 画布比例 时**必须精确算出 `(0,0,CW,CH)`**
  —— 那是"存量逐字节零回归"的判据。摆位经 CTM 送入矩形（`ctx.translate(sr.x,sr.y)` +
  `ctx.scale(sr.w/CW, sr.h/CH)`），`drawImage` 的 dst **保持字面量 `(0,0,CW,CH)`**：
  顺序钉子与两处变异都锚在它上面，别顺手改成 dest 矩形参数。
- ⚠️ **竖素材屏蔽运镜与画布方向无关**：判据是**源视频**竖（注入的 `CFG.vertical`，`render/server.rs`
  按 `h > w` 给），不是画布方向。只在循环末尾归零 `handheld` 不够 —— `zoom` 照旧把画面拖走，
  而预览侧（`engine.js::cameraAt:310`）早就整条返回了：**预览不动、成片在缩放**，同样不报错。
  另注：`vtrim-render-core::evaluate_frame` 只归零 handheld —— 那是**已删除**的 wgpu 路径留下的
  平行实现，现在没有生产消费者；两边口径差异记在这里，改之前先确认动的是哪条路径。
- ⚠️ **字幕折行同样是两份实现**（本模板 `wrapCaptionParts` ↔ `engine.js::wrapCaptionParts`），
  必须逐字一致。曾经 `append` 先读 `cur.length` 再判溢出，而溢出分支的 `pushLine()` 会把 `cur` 置空
  → **任何一次折行都抛 `TypeError: can't access property "length", cur is null`**
  （竖屏安全宽只有 945px，普通长字幕必折行 → "竖屏一播到长字幕就崩"）。改一边就同时改另一边。
- ⚠️ **预览看着正常 ≠ 出片正常**：模板改完必须**真跑一次出片**验证。
- > 改动只允许落在本目录；跨模块改动先读对方卡，并跑两条门禁。

## 验证

```bash
node webui/scripts/check-overlay.mjs        # 覆盖层色档 + fade 两档契约：本模板 ↔ engine.js 逐例对拍（77 项）
node webui/scripts/check-caption-wrap.mjs   # 折行契约（21 项 + 4 处变异自证）
node webui/scripts/check-handheld.mjs       # 手持摇摆 + 运镜起停状态机（63 项，含竖素材 zoom/handheld 归零）
node webui/scripts/check-sticker-anchor.mjs # 贴纸锚点（64 项）
node webui/scripts/check-stage-backdrop.mjs # 衬底 + 视频层摆位（73 项 + 8 处变异自证；③b 裸变量 / ③c 几何）

# 改了模板后必须重建二进制，并真跑一次带字幕的出片
cargo build -p vtrim-cli
```

Rust 侧的钉子：`template_stage_hooks_match_contract`、`portrait_canvas_fill_still_paints_backdrop`
（`cargo test -p vtrim-polish`）；真实像素 A/B 见 `VTEDIT_BACKDROP_DIR=<竖屏工程>` 那条诊断，
以及竖素材摆放 + 运镜门的整链实证
`VTEDIT_FIT_DIR=<竖素材工程> cargo test -p vtrim-polish --test portrait_stage_fit -- --ignored --nocapture`
（真模板 + 三段变异，逐帧读像素，见文件头）。
