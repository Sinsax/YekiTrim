# vtrim-polish — GSAP 精修引擎（从 vtedit 合并）

> 范围：`crates/vtrim-polish`/** ｜ 上游：`vtrim-cli`（`vtrim polish *`）、`vtrim-server`（`api/polish.rs`、`api/render.rs`、`stickers.rs`）、`vtrim-service`（`silence.rs`）—— 三者都直接调本 crate ｜ 验证：`cargo check -p vtrim-polish`

## 职责

从 `events.json` 出发生成片段/字幕/贴纸/音效/特效，输出渲染工程与成片。整条链：
`polish init`（初始化 + 解析字幕弹幕）→ `polish llm`（分段 + 逐段事件）→ `polish process`（后处理 + 下载资源 + 计算位置）
→ `polish render`（headless Chrome 逐帧 + ffmpeg 合成）→ `polish cover`（封面）。工程状态只在项目目录的
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
`build_cover_html` / `cover_bg_vars` / `cover_frame_vars` / `sanitize_hex_color` / `cover_blur_css` / `cover_bg_mode`；
常量 `COVER_FRAME_IMG = "_cover_frame.jpg"` / `COVER_BG_IMG = "_cover_bg.jpg"` /
`COVER_CAPTURE_W|H = 1920|1080` / `COVER_EMOJI_{DEFAULT,MAX_CHARS,MIN_PX,MAX_PX}`。

`crates/vtrim-polish/src/parse/`：`srt::{parse, parse_file, Subtitle}`；
`text::{correct_text, apply_highlights, detect_hook, assign_danmaku_tracks}`。

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
- **封面三层：文字 / 关键帧 / 背景**（用户原话：「封面分三层：文字，关键帧，背景」）：
  `#title`（文字）在最上、`#frame`（**关键帧层**：当前帧清晰图 `_cover_frame.jpg`）居中、
  `#bg`（**背景层**）在最下，再往上才是渐变/暗角/特效。**三层各有各的图源，谁也不许覆盖谁**。
- ⚠️ **`text` 是用户输入，进模板前必须 `escape_html`**（不转义＝往封面 HTML 里注模板，有单测）
  —— `[[cover.effects]]` 的 `kind = "emoji"` 那一支。
- ⚠️ **默认对齐是 `left`**（`default_cover_text_align()`，2026-09 由 `center` 改来）：默认预设
  就叫「左文」、默认 `text_x = "35%"`，三处以前是"预设左文、配置居中"，自相矛盾。
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
- 封面特效 emoji：`text` 最多 `COVER_EMOJI_MAX_CHARS`(12) 个字符（带 ZWJ/肤色修饰符的一个 emoji
  就要 7~8 个码点），`x/y` 是**字符中心**，`amount` 当大小用（`COVER_EMOJI_MIN_PX`(48) ~
  `COVER_EMOJI_MAX_PX`(288)px，公式只在 `cover.rs::cover_fx_html` 里一份，前端 `fxStyle` 是镜像）。
  字体栈要带 `Segoe UI Emoji / Apple Color Emoji / Noto Color Emoji`，否则出片是豆腐块。

> 改动只允许落在本目录；跨模块改动先读对方卡，并跑两条门禁。

## 验证

```bash
cargo check -p vtrim-polish
```

0 warnings。动了 `templates/index.html` / `cover.html` 之后必须再真跑一次出片：
预览看着正常 ≠ 出片正常。
