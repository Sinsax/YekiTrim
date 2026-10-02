# vtrim-polish — GSAP 精修引擎（从 vtedit 合并）

> 范围：`crates/vtrim-polish`/** ｜ 上游：`vtrim-cli`（`vtrim polish *`）、`vtrim-server`（`api/polish.rs`、`api/render.rs`、`stickers.rs`）、`vtrim-service`（`silence.rs`）—— 三者都直接调本 crate ｜ 验证：`cargo check -p vtrim-polish`

## 职责

从 `events.json` 出发生成片段/字幕/贴纸/音效/特效，输出渲染工程与成片。整条链：
`polish init`（初始化 + 解析字幕弹幕）→ `polish llm`（分段 + 逐段事件）→ `polish process`（后处理 + 下载资源 + 计算位置）
→ `polish render`（底座 Dhampir 整段出画面 + 本仓 ffmpeg 混音/合流）→ `polish cover`（封面）。工程状态只在项目目录的
`polish.toml` + `data/` 里，不碰数据库。

子模块各有专卡，先读它们再动手：`process/`（后处理主流程 / 音效 / 特效 / 落点）、`render/`（出片链路 / 预览服务器）、
`project/`（配置类型 / 去静音 / 校验）、`assets/`（贴纸库 / 音效目录 / 下载）、`llm/`（分段与事件生成）。
本卡覆盖**没有专卡的三块** —— `commands.rs`、`cover.rs`、`parse/` —— 以及跨子模块的硬规矩。

## 入口

`crates/vtrim-polish/src/commands.rs`（`vtrim polish *` 的实现，CLI 与 WebUI 共用同一份）：
- `cmd_init(path)` / `cmd_parse(project_path)`：初始化工程；SRT + 文本解析。
- `cmd_llm(project_path, full)`（快照索引写作 `commands.rs:200`；`full` 默认 true = 全部重做）与
  `cmd_llm_segment(project_path, index)`（只重做这一段事件）。
- `cmd_preview(project_path, mode)`（快照索引写作 `commands.rs:296`；自动先跑 process，再起双端口预览）。
- `cmd_doctor(_)` / `cmd_test(_)`。
- 贴纸库：`cmd_stickers_list()` / `cmd_stickers_test(keywords)` / `cmd_stickers_build(package)` /
  `build_package_tags(...)`（WebUI「LLM 压缩构建」的同一份实现）/ `cmd_stickers_scan(llm)`。

`crates/vtrim-polish/src/cover.rs`：`cmd_cover_template` / `cmd_preview_cover` / `cmd_cover(project_path, existing_video)` /
`build_cover_page`（**HTML 替换链只此一份**，预览与出片都走它）/ `build_cover_html`（预览用的薄包装）/
`resolve_cover_stack` / `plan_cover_layers` / `cover_layer_css` / `is_default_stack`（图层栈，见边界）/
`cover_bg_vars` / `cover_frame_vars` / `sanitize_hex_color` / `cover_blur_css` / `cover_bg_mode`；
常量 `COVER_FRAME_IMG = "_cover_frame.jpg"` / `COVER_BG_IMG = "_cover_bg.jpg"` /
`COVER_CAPTURE_W|H = 1920|1080` / `COVER_EMOJI_{DEFAULT,MAX_CHARS,MIN_PX,MAX_PX}`。

`crates/vtrim-polish/src/parse/`：`srt::{parse, parse_file, Subtitle}`；
`text::{correct_text, apply_highlights, detect_hook, assign_danmaku_tracks}` ——
⚠️ `assign_danmaku_tracks` 分完泳道后**必须把在屏窗口抬到"至少够它滚完"**
（`vtrim_core::danmaku::clamp_display_windows`）：渲染侧的渐隐钉在窗口末尾、滚动由 `travel` 决定，
不抬就是"弹幕没到左边缘就消失"（录播姬 XML 的 8 秒窗口对中文弹幕普遍不够）。明细见 core/`danmaku` 卡。

`process::cmd_process`（process/ 卡）、`render::{cmd_render, render_chunk_sync}`（render/ 卡）。

## 边界

- **依赖方向**：本 crate 只依赖 `vtrim-core` + `vtrim-render-core`（`Cargo.toml` 是硬门禁）。
  polish 不许认识 `vtrim_service` / `vtrim_server` / `vtrim_cli` —— 反向依赖会直接编译失败。
- **长任务写 `polish.toml` 一律走 `project::update_polish_toml(dir, |cfg| …)`**（锁内重新
  load + 只覆盖自己负责的字段）。cmd_llm 只写 segments/filter、cmd_process 只写 segments/processed、
  cmd_cover 只写 cover.*。**不许再"开头 load、结尾整份写回"**：那样 LLM 跑两分钟期间用户存的
  选框/封面/屏蔽会被结尾那次覆写静默吞掉（这正是"LLM 剪辑时不能调区域"的根因）。
- ⚠️ **改了 `templates/index.html` 必须重建二进制**：它经 `include_str!` 进 `vtrim-polish`
  （`render/server.rs::INDEX_HTML`），运行中的 `vtrim server` 端出的仍是旧页面 ——
  「改完没反应」十有八九是没重建/没重启，不是代码没生效。
- ⚠️ **出片逐帧路径里任何 `ReferenceError` 都是"整块 chunk 失败 → 整个 render job 失败"**
  （实测：A5 折行改造留下的一句 `ctx.font = '700 ' + fs + ' ' + ff`，
  `fs` 在 `renderFrame` 作用域里没声明 → 日志 `Chunk: Render error: fs is not defined`，
  表现是**所有带字幕的片段都导不出来、无字幕的正常**）。改了 `renderFrame` 附近后至少跑一次
  带字幕的出片；`node webui/scripts/check-stage-backdrop.mjs` 的 ③b 段钉住这类裸变量
  （判据走**去注释后的源码** —— 注释里出现同样的字样会把判据自己顶翻）。
- ⚠️ **`templates/index.html` 是出片路径的唯一实现，DOM/CSS 那层不算数**：canvas `renderFrame`
  **不读 DOM**。只把 `_canvas.stage`（画面摆放）与 `CFG.vertical`（运镜门控）消费在 DOM 上的话，
  成片与预览会**静默分叉**：竖素材进横画布 → 成片被拉成宽屏、竖素材在成片里照旧被 zoom 拖走
  （用户 2026-09-21 报的原文），预览里全程正常、不报错。两条消费点：`stageDrawRect`
  （`axis/position/scale/padding` → 画面矩形；`axis=fill` + 无 padding + 源比例 == 画布比例 必须仍
  **精确等于整块画布** = 横屏存量零回归）与 `getActiveCamera` 开头的 `CFG.vertical` 早退
  （`zoom=1 / handheld=0`，与 `engine.js::cameraAt` 同口径；⚠️ `vtrim-render-core::evaluate_frame`
  是**已删除**的 wgpu 路径留下的平行实现，只归零 handheld、且早已没有生产消费者 —— 别拿它对口径）。
  细节见 `templates/AGENTS.md`、`src/project/AGENTS.md`。
- ⚠️ **消费侧纵深防御**：渲染封面时对 `cover.image` 再做一次 `tenant::ensure_within`
  （`vtrim-polish/src/cover.rs`）。handler 层的校验只覆盖"走 API 的那些入口"，别删这道。
- 公式与净化都在 `cover.rs` 且**只此一份**：`cover_blur_css(scale)`（基准 28px、
  放大 `1+px/150` 把模糊边缘推出画面）、`sanitize_hex_color`（**颜色直接进 HTML/CSS，
  必须净化**，非法值退 `#000000`）、`cover_bg_mode`（只有 `blur` 走模糊，其余一律 `color`，
  老配置里的 `frame` 也当纯色）。前端 `CoverDesigner.vue` 的 `coverBlurCss` 是镜像，
  改一边要改另一边。
- ⚠️ **导入背景图片写的是 `_cover_bg.jpg`**（`/api/polish/cover-background` 同时把 `bg_mode` 设成
  `image`）。**别再写回 `_cover_frame.jpg`** —— 那是"背景替换关键帧"的老 bug（用户报过
  「又把关键帧那一层给我删了」）。`cover.image` 也**不要**在导入时清空（它是关键帧的来源之一）。
- ⚠️ **出片不再删 `_cover_frame.jpg`**（`cmd_cover` 末尾那句 `remove_file` 已删）：它就是关键帧层的
  图源，删了等于①用户导出的那一帧白导、②下次出片又按 `frame_pos` 重新截、同一份封面每次换脸。
- **图层栈（`[cover] layers`，2026-09 起）**：封面从"三层写死"变成**真图层栈**（顺序 + 可见性都落盘）。
  `resolve_cover_stack(&CoverConfig)` 是**唯一推导**：id = `bg` / `frame`（关键帧层 `_cover_frame.jpg`）/
  `filter`（渐变+暗角，一层两块）/ `title` / `fx:<CoverEffect::id>`；`plan_cover_layers()` 把它折成每块的
  `z-index`，`cover_layer_css()` + `cover_fx_html()` 负责注入。**三层各有各的图源，谁也不许覆盖谁**。
  - ⚠️ **老工程零迁移靠"默认栈走改造前的渲染形式"**（`is_default_stack`）：默认栈时**一个 z-index 都不注入**
    （只把模板里那条 `#fx-extra{z-index:1}` 补回来）、特效还是一个 div —— 实测给五块都写上 z-index 之后
    浏览器会换一条合成路径，缩放中的帧层重采样差 ~9/255，**SHA-256 会变**，而"存量逐字节零回归"是本仓的硬判据。
    判据：`.workbuddy/cover-layer/` 里三个样本（含六种特效全上的 rich 工程）改前改后 **SHA-256 相同**。
    ⚠️ 那条基线是**旧 chrome 截图通道**上量的；现在出片走 Takumi（静态光栅化，没有这条合成路径差异），
    所以这一项对**当前**封面出片不再适用 —— 它留着是为了解释"为什么默认栈一个 z-index 都不注入"这个**设计**。
  - ⚠️ **自定义栈时 `#fx-extra` 必须退回 `z-index:auto`**：它一带 z-index 就是个 stacking context，
    所有特效的 z-index 会被锁死在同一层（拖动排序看着生效、出片不动，**而且不报错**）。
  - ⚠️ **`overlay` 特效额外加 `isolation:isolate`**（只加它、`blur` 加不得）：改造前 `#fx-extra` 是 stacking
    context，`mix-blend-mode:overlay` **没有背景可混**（实测"绿底叠 #ff3366"是橄榄绿）；拆开后它会真混合 →
    **老封面会变色**。想要真混合走 P2 的「每层混合模式」（显式选择才有观感预期）。
  - ⚠️ **`COVER_HTML` 只允许在 `build_cover_page` 里替换一次**：以前预览（`build_cover_html`）与出片
    （`cmd_cover` 内联那段）各写一份，漏改一边就是"预览有层序、出片没有"。新增模板变量只加这一处；
    `check-cover-stage.mjs` ⑦ 钉住"只有一条替换链"。
- ⚠️ **`text` 是用户输入，进模板前必须 `escape_html`**（不转义＝往封面 HTML 里注模板，有单测）
  —— `[[cover.effects]]` 的 `kind = "emoji"` 那一支。
- ⚠️ **默认对齐是 `left`**（`default_cover_text_align()`，2026-09 由 `center` 改来）：默认 `text_x = "35%"`，
  老工程里显式写过的 `text_align` 一律照旧（字段存在就不走默认）。
  改这个默认值**不要顺手改 `cover.rs::cover_text_align` 的 `_ => ("center","center")` 兜底** ——
  那个 `_` 同时兜着合法值 `"center"` 本身，改了会把显式居中的老封面一起翻成左对齐。
- **导出产物同名不覆盖：加 `_1` / `_2`**（用户要求"如果已经存在导出文件，就加 _n，类似封面的操作"）：
  `render::unique_output_path`（`render/pipeline.rs`，**成片与封面共用**，有单测）在目标已存在时
  追加 `_1` / `_2`…（`Ffmpeg::mux` 带 `-y`，同名即覆盖 —— 别去掉这层后缀）。
- **出片几何**（`cmd_cover`）：模板里 `html.export` 让 `#page` 不缩放、左上角对齐视口原点，
  同时 `Emulation.setDeviceMetricsOverride(1920×1080)` 把视口**撑满画布**，抓图用
  `clip: (0,0,1920,1080)`。**别再退回 `clip: None`**：headless 实测视口只有 1904×929，
  16:9 页面被居中 → 成片变 1904×929 且左右两条黑边（老 bug，已修）；`#toolbar`（右下角 ↻ 预览按钮）
  在 `html.export` 下 `display:none`，否则会被连按钮一起抓进成片。
- ⚠️ **封面不接 Dhampir**（2026-09-30 探索结论；三种"接入"的含义、逐条利弊与"什么时候才该重开"见
  **`docs/plan/cover-dhampir-tradeoff.md`**）：封面是"静态单帧 + HTML/CSS 排版"，而**设计器 = 同一份
  HTML/CSS** 是它唯一的不变式 —— 换渲染器就是第三份实现。且 Dhampir 今天缺的正好是封面全在用的
  （字距/行高/文字阴影/描边宽度与颜色/混合模式/backdrop-filter/emoji/暗角/扫描线/σ>8 模糊），
  速度上也没有收益（实测一张封面 ≈3.0–3.3s，Dhampir 单帧冷启动就要 1.5–3.2s）。
- **封面只剩一条渲染通道**（2026-10-01 用户拍板）：**`takumi`**（静态渲染，`src/render/takumi.rs`，
  `cover-takumi` feature **默认关**但 CLI 的 `default` 带上它）。**`chrome` 那条 headless Chrome 截图通道
  已整条删除** —— `normalize_cover_renderer` 对 `"chrome"` 给的是**单独的**「这条腿已删除」错误
  （**不是**折进"认不出"：那样用户只会去猜拼写）。切到 takumi 的依据 = 字体三处来源收敛后重跑封面门禁
  （clip-14 真工程）非文字区 **1.17/255**、整图 **11.44/255**，均在阈值内 —— 且是**同库同字体**的对比。
  对拍基线已**冻结**（`scripts/fixtures/cover-golden/`，取自删除前最后一次 chrome 渲染）：
  `node scripts/check-cover-takumi.mjs --golden scripts/fixtures/cover-golden`，**不再需要 chrome**。
  - `build_cover_page_static` 是静态那条的构建入口（= 原替换链 + 填 `__EXPORT_CSS__`）。
  - ⚠️ 当年记录下来的那条"零回归由构造保证"（动画把元素提升成合成层、挂上"关动画"规则会换光栅化路径
    ⇒ 存量 SHA 变）**随 chrome 那条腿一起作废** —— 它描述的是一条已经不存在的路径，别再照着它推。
  - ⚠️ 模板里 `:root{}` 那个变量块**必须留**：静态渲染器不跑 JS，而变量的唯一来源曾是页面脚本的
    `setProperty` —— 缺了它所有 `var(--…)` 声明**整条被丢**（标题没字号没描边、背景没模糊，实测 38.44/255）。
  - ⚠️ **字体**：Takumi 不读系统字体，靠 `cover_font_files()` 把字体库全量注册；一个字体都没有时**明确报错**
    （静默出豆腐块是最坏形态）。`VTEDIT_COVER_RENDERER` 认不出的值也**当场报错、不静默回退**。
  - 验收口径、四个引擎差异（`@font-face` 不支持 / 无资源解析器要内联 / 复合选择器 `#id.class` 不匹配 /
    忽略 `paint-order`）与残差（标题字形 ~15/255 是栅格化固有差，**适配层修不了**）见
    **`docs/plan/takumi-cover-capability.md`**。
  - **怎么用**：CLI `vtrim polish cover <目录> --renderer takumi`（不传 = 默认链）；WebUI 封面设计器点
    「生成封面」直接出图 —— **通道弹窗已删**（只剩一个按钮的弹窗不再是"选择"），前端不传 `renderer`。
    取值口径仍只在 `cover.rs`（显式参数 > `VTEDIT_COVER_RENDERER` > **takumi**，认不出就报错，
    `chrome` 给"已删除"的专门错误）。静态通道**能不能用**由 `cover::cover_static_capability()` 给出
    （编了 feature + 字体库非空），经 `/api/health.cover_static` 给前端置灰 + 写原因用。
- 封面特效 emoji：`text` 最多 `COVER_EMOJI_MAX_CHARS`(12) 个字符（带 ZWJ/肤色修饰符的一个 emoji
  就要 7~8 个码点），`x/y` 是**字符中心**，`amount` 当大小用（`COVER_EMOJI_MIN_PX`(48) ~
  `COVER_EMOJI_MAX_PX`(288)px，公式只在 `cover.rs::cover_fx_html` 里一份，前端 `fxStyle` 是镜像）。
  字体栈要带 `Segoe UI Emoji / Apple Color Emoji / Noto Color Emoji`，否则出片是豆腐块。
  ⚠️ **标题文本里的 emoji 是另一条路**（用户报的「前后端 emoji 字体不一致」+「渲染的 emoji 没有描边」都
  在它身上）：它是 `title_lines` 里的**联排字符**（clip-14 真实配置 `直接飞出去流鼻血🩸`、`effects = []`），
  特效层那套**一行都不落在它身上**。收口 = `cover.rs::wrap_emoji_runs` + `emoji_run_span`：把 emoji 段包成
  `<span class="cov-emoji">`，族名用**库 id**（浏览器 FontFace/`@font-face` 与 Takumi 的 id 别名都按 id），
  描边环用 4 向 `drop-shadow`（别加对角：链式累积会让外缘≈2.4k）（位图字形在 Takumi 里没有描边路径）+ **必须 `display:inline-block`**。
  前端 `CoverDesigner.vue::wrapEmojiRuns` 是镜像，**改一边要改另一边**；📖 证据链见
  **`docs/plan/takumi-cover-capability.md` §15**。

- ⚠️ **两条真像素探针**（`tests/portrait_*.rs`，`#[ignore]`，2026-10-02 修好并真跑过）：
  `portrait_stage_fit`（竖素材进横画布必须 contain + 运镜被吃掉）与 `portrait_backdrop`
  （挪开处必须是模糊衬底、不是黑）。**harness 的三条口径与被修掉的坑**（抽帧要抽成片、变异锚点要落在
  语义位置、衬底判据按开关决定断或明说跳过）见 `docs/plan/polish-vertical-canvas.md` §十。

> 改动只允许落在本目录；跨模块改动先读对方卡，并跑两条门禁。

## 验证

```bash
cargo check -p vtrim-polish
```

0 warnings。动了 `templates/index.html` / `cover.html` 之后必须再真跑一次出片：
预览看着正常 ≠ 出片正常。模板侧的漂移对拍与真实像素探针（`check-*.mjs` 五条 +
`--test portrait_backdrop` / `--test portrait_stage_fit`）命令见 `templates/AGENTS.md` 的「验证」。
