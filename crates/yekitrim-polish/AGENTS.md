# yekitrim-polish — GSAP 精修引擎（从 vtedit 合并）

> 范围：`crates/yekitrim-polish`/** ｜ 上游：`yekitrim-cli`（`yekitrim polish *`）、`yekitrim-server`（`api/polish.rs`、`api/render.rs`、`stickers.rs`）、`yekitrim-service`（`silence.rs`）—— 三者都直接调本 crate ｜ 验证：`cargo check -p yekitrim-polish`

## 职责

从 `events.json` 出发生成片段/字幕/贴纸/音效/特效，输出渲染工程与成片。整条链：`polish init`（+ **预览代理**）→ `polish llm`（分段 + 逐段事件）→ `polish process`（后处理 + 资源 + 落点）→ `polish render`（底座出画面 + 本仓 ffmpeg 混音/合流）→ `polish cover`（封面）。工程状态只在 `<工程>/polish.toml` + `data/`，不碰数据库。

子模块各有专卡，先读它们再动手：`process/`、`render/`、`project/`、`assets/`、`llm/`。本卡覆盖**没有专卡的三块** —— `commands.rs`、`cover.rs`、`parse/` —— 以及跨子模块的硬规矩。

> 历史与实测数据见 **`docs/polish-history.md`**（§1~§10：emoji 作者入口 / 预览代理实测 / cover 通道演进 / 图层栈基线 / 封面出片几何 / emoji 证据链 / 探针 harness / `polish.toml` 覆写事故 / 模板事故复盘 / 对齐迁移；§11：`cover.rs` / `parse/` 的函数与常量清单；§12：三条预览腿 / 11+2 降级清单的划法 / 为什么不再加像素判据）。

## 入口

`crates/yekitrim-polish/src/commands.rs`（`yekitrim polish *` 的实现，CLI 与 WebUI 共用同一份）：
- `cmd_init(path)` / `cmd_parse(project_path)`：初始化工程；SRT + 文本解析。
- `cmd_llm(project_path, full)`（快照索引 `commands.rs:200`；`full` 默认 true = 全部重做）与 `cmd_llm_segment(project_path, index)`（只重做这一段事件）。
- `cmd_preview(project_path, mode)`（快照索引 `commands.rs:296`；自动先跑 process，再起双端口预览）。`cmd_doctor(_)` / `cmd_test(_)`。
- 贴纸库：`cmd_stickers_list/test_build/scan/set` 一族（`cmd_stickers_build` 内含 WebUI「LLM 压缩构建」的 `build_package_tags`，同一份实现）。
  ⚠️ **emoji 分类是程序资产，没有 CLI 面**（2026-10-17 用户口径：「分类作为内建，交给程序处理，不给用户重建和更改的权限」）：`cmd_emoji_build(...)` 只剩**作者入口**（命令与 881 条六维重做的细节见 §1）：
  `VT_EMOJI_BUILD=1 cargo test -p yekitrim-polish --lib emoji_build_dev_tool -- --ignored --nocapture`。
  它**写回仓库里的 `assets/emoji_catalog.json`**（编译期输入 ⇒ 跑完要 `cargo build`）；写盘前留 `.bak`；默认**增量**（`VT_EMOJI_ALL=1` 才全量重做）。**别再给它加 clap 子命令。**

`crates/yekitrim-polish/src/cover.rs`：入口 `cmd_cover_template` / `cmd_preview_cover` / `cmd_cover(...)`；**HTML 替换链只此一份**（`build_cover_page`，预览与出片都走它）+ 图层栈一族；函数与常量清单见 `docs/polish-history.md` §11.1。

`crates/yekitrim-polish/src/parse/`：`srt::*` + `text::*`。⚠️ `assign_danmaku_tracks` 分完泳道后**必须把在屏窗口抬到"至少够它滚完"**（`clamp_display_windows`）：渐隐钉在窗口末尾、滚动由 `travel` 决定，不抬就是"弹幕没到左边缘就消失"（录播姬 XML 的 8 秒窗口对中文弹幕普遍不够）。明细见 core/`danmaku` 卡、§11.2。

`parse/charlog.rs`：**字级时间戳搬运**（E4b，`asr/timeline.json` → `data/char-timeline.json`，入口 `sync_char_timeline`）。⚠️ **`output_dir` ≠ 片段目录**（父子，polish 拿不到 ctx）⇒ 路径反推；**缺文件优雅**（不报错、不阻断、**不落文件**=R4）。见 `docs/plan/char-timeline-carry.md`。

`process::cmd_process`（process/ 卡）、`render::{cmd_render, render_chunk_sync}`（render/ 卡）。

- **预览代理**（`render/proxy.rs`）：`polish init` 生成 `<片段>/_proxy_720.mp4`（720p / GOP10 CRF20 / **保持源帧率** / 音轨 `copy` / 幂等 / 失败不阻断 init），**只给预览腿读**，**出片永远用源**；**容量是硬约束**（实测 165ms→18ms、0.52× 源 —— 别再为快几毫秒吃光磁盘，数字与四条钉子见 §2）。`VTEDIT_NO_PROXY=1` 可整体关掉。

## 边界

### 三条预览腿与判据边界（决策 D-1；推导、13 条清单的划法与"为什么不再加像素判据"见 `docs/polish-history.md` §12）

1. **真值方向 = 转译器 + 时间线数学**：`tools/polish-to-dhampir.mjs` + `yekitrim-render-core/src/timeline.rs`；`engine.js` 那条 DOM 腿是**无 WebGPU 时的降级近似**，允许视觉细节差、**不要求与成片逐像素一致**（Dhampir wasm 才是主预览腿）。
2. **13 条 parity 判据里 11 条继续精确对拍**（逐条清单见 §12.2），**只有 2 条降级**：`check-dom-camera-parity` → 容差 **±1px**；`check-noise-parity` → 只验"开关/强度值透传"，不验噪点图案。
3. ⚠️ **新增 parity 判据前先在这里登记它属于哪一类**；"近似预览 vs 出片"的**像素**判据不再新增 —— 要验像素走底座那条（`scripts/check-render-visible.mjs` 或 `.workbuddy/` 的真像素探针）；真实收益在**删掉第三套渲染实现本身**（T14），不在放宽判据。

- **依赖方向**：本 crate 只依赖 `yekitrim-core` + `yekitrim-render-core`（`Cargo.toml` 是硬门禁）。
  polish 不许认识 `yekitrim_service` / `yekitrim_server` / `yekitrim_cli` —— 反向依赖会直接编译失败。
- **长任务写 `polish.toml` 一律走 `project::update_polish_toml(dir, |cfg| …)`**（锁内重新 load + 只覆盖自己负责的字段：cmd_llm 写 segments/filter、cmd_process 写 segments/processed、cmd_cover 只写 cover.*）。**不许再"开头 load、结尾整份写回"** —— 那样 LLM 跑两分钟期间用户存的选框/封面/屏蔽会被结尾那次覆写静默吞掉（覆写事故复盘见 §8）。
- ⚠️ **改了 `templates/index.html` 必须重建二进制**：它经 `include_str!` 进 `yekitrim-polish`（`render/server.rs::INDEX_HTML`），运行中的 `yekitrim server` 端出的仍是旧页面 ——「改完没反应」十有八九是没重建/没重启。
- ⚠️ **出片逐帧路径里任何 `ReferenceError` 都是"整块 chunk 失败 → 整个 render job 失败"**（实测：A5 折行改造留下一句未声明的 `fs` ⇒ **所有带字幕的片段都导不出来、无字幕的正常**；复盘见 §9）。改了 `renderFrame` 附近后至少跑一次带字幕的出片；`node webui/scripts/check-stage-backdrop.mjs` 的 ③b 段钉住这类裸变量（判据走**去注释后的源码**：注释里同样的字样会把判据自己顶翻）。
- ⚠️ **校正（2026-11-24，T13）：`templates/index.html` 的 canvas `renderFrame` 不是出片路径** —— 真实出片走转译器 → 底座（`pipeline.rs` 对模板的两处提及都是**注释**）；全仓只有两处 `include_str!(index.html)`（`render/server.rs` 预览服务、`project/config.rs` 的「模板必须消费 `_canvas`」契约测试）。旧文"出片路径的唯一实现"是 wgpu 时代口径，见 §9 / `docs/polish-history.md` §12.4。
  ⚠️ **但模板腿的两条消费点仍然必须对**（它会端出给人看的预览）：canvas `renderFrame` **不读 DOM**，只把 `_canvas.stage`（画面摆放）与 `CFG.lanes`（六条轨道开关）消费在 DOM 上就会**静默分叉**（竖素材进横画布 → 成片被拉宽、竖素材照旧被 zoom 拖走，用户 2026-09-21 的原文；预览全程正常）。消费点：`stageDrawRect`（`axis=fill` + 无 padding + 源比例 == 画布比例 仍**精确等于整块画布**）与 `getActiveCamera` 开头的 `!CFG.lanes.camera` 早退（与 `engine.js::cameraAt` 同口径）。
  ⚠️ `yekitrim-render-core::evaluate_frame` 是**已删除**的 wgpu 路径留下的平行实现，只归零 handheld、且早已没有生产消费者 —— 别拿它对口径。细节见 `templates/AGENTS.md`、`src/project/AGENTS.md`。
- ⚠️ **消费侧纵深防御**：渲染封面时对 `cover.image` 再做一次 `tenant::ensure_within`
  （`yekitrim-polish/src/cover.rs`）。handler 层的校验只覆盖"走 API 的那些入口"，别删这道。
- 公式与净化都在 `cover.rs` 且**只此一份**：`cover_blur_css(scale)`（基准 28px、`1+px/150` 推出画面）、`sanitize_hex_color`（**颜色直接进 HTML/CSS，必须净化**，非法值退 `#000000`）、`cover_bg_mode`（只有 `blur` 走模糊，其余 `color`）。前端 `CoverDesigner.vue` 的 `coverBlurCss` 是镜像，改一边要改另一边。
- ⚠️ **导入背景图片写的是 `_cover_bg.jpg`**（`/api/polish/cover-background` 同时把 `bg_mode` 设成 `image`）。**别再写回 `_cover_frame.jpg`** —— 那是"背景替换关键帧"的老 bug（用户报过「又把关键帧那一层给我删了」）。`cover.image` 也**不要**在导入时清空（它是关键帧的来源之一）。
- ⚠️ **出片不再删 `_cover_frame.jpg`**（`cmd_cover` 末尾那句 `remove_file` 已删）：它就是关键帧层的图源，删了等于①用户导出的那一帧白导、②下次出片又按 `frame_pos` 重新截、同一份封面每次换脸。
- **图层栈（`[cover] layers`，2026-09 起）**：封面是真图层栈（顺序 + 可见性都落盘）。`resolve_cover_stack(&CoverConfig)` 是**唯一推导**（id = `bg`/`frame`/`filter`/`title`/`fx:<CoverEffect::id>`），`plan_cover_layers()` 折成每块 `z-index`，`cover_layer_css()` + `cover_fx_html()` 注入；**三层各有各的图源，谁也不许覆盖谁**。
  - ⚠️ **老工程零迁移靠"默认栈走改造前的渲染形式"**（`is_default_stack`）：默认栈时**一个 z-index 都不注入**（只把模板里那条 `#fx-extra{z-index:1}` 补回来）、特效还是一个 div —— 给五块都写上 z-index 会换合成路径，缩放中的帧层重采样差 ~9/255、**SHA-256 会变**，而"存量逐字节零回归"是本仓的硬判据（判据 `.workbuddy/cover-layer/` 三样本改前改后 **SHA-256 相同**）。
    ⚠️ 基线量于**旧 chrome 通道**；Takumi（静态光栅化）没有这条差异 —— 它留着是为了解释"为什么默认栈一个 z-index 都不注入"这个**设计**（见 §4）。
  - ⚠️ **自定义栈时 `#fx-extra` 必须退回 `z-index:auto`**：它一带 z-index 就是个 stacking context，所有特效的 z-index 会被锁死在同一层（拖动排序看着生效、出片不动，**而且不报错**）。
  - ⚠️ **`overlay` 特效额外加 `isolation:isolate`**（只加它、`blur` 加不得）：改造前 `#fx-extra` 是 stacking context，`mix-blend-mode:overlay` **没有背景可混**（实测"绿底叠 #ff3366"是橄榄绿）；拆开后它会真混合 ⇒ **老封面会变色**。想要真混合走 P2 的「每层混合模式」（显式选择才有观感预期）。
  - ⚠️ **`COVER_HTML` 只允许在 `build_cover_page` 里替换一次**：以前预览（`build_cover_html`）与出片（`cmd_cover` 内联那段）各写一份，漏改一边就是"预览有层序、出片没有"。新增模板变量只加这一处；`check-cover-stage.mjs` ⑦ 钉住"只有一条替换链"。
- ⚠️ **`text` 是用户输入，进模板前必须 `escape_html`**（不转义＝往封面 HTML 里注模板，有单测）
  —— `[[cover.effects]]` 的 `kind = "emoji"` 那一支。
- ⚠️ **默认对齐是 `left`**（`default_cover_text_align()`，2026-09 由 `center` 改来）：默认 `text_x = "35%"`，老工程显式写过的 `text_align` 照旧。改默认值**不要顺手改 `cover.rs::cover_text_align` 的 `_ => ("center","center")` 兜底** —— 那个 `_` 同时兜着合法值 `"center"`，改了会把显式居中的老封面一起翻成左对齐（§10）。
- **导出产物同名不覆盖：加 `_1` / `_2`**（用户要求）：`render::unique_output_path`（`render/pipeline.rs`，**成片与封面共用**，有单测）在目标已存在时追加 `_1` / `_2`…（`Ffmpeg::mux` 带 `-y`，同名即覆盖 —— 别去掉这层后缀）。
- **出片几何**（`cmd_cover`）：模板里 `html.export` 让 `#page` 不缩放、左上角对齐视口原点，同时 `Emulation.setDeviceMetricsOverride(1920×1080)` 把视口**撑满画布**，抓图 `clip: (0,0,1920,1080)`。**别再退回 `clip: None`**（headless 视口 1904×929 ⇒ 居中 + 黑边，见 §5）；`#toolbar` 在 `html.export` 下 `display:none`。
- ⚠️ **封面不接 Dhampir**：封面是"静态单帧 + HTML/CSS 排版"，而**设计器 = 同一份 HTML/CSS** 是它唯一的不变式 —— 换渲染器就是第三份实现；且 Dhampir 缺的正好是封面全在用的那些（字距/行高/阴影/描边/混合/backdrop-filter/emoji/暗角/扫描线/σ>8 模糊）。利弊见 **`docs/plan/cover-dhampir-tradeoff.md`**。
- **封面只剩一条渲染通道**（2026-10-01 用户拍板）：**`takumi`**（静态渲染，`src/render/takumi.rs`；`cover-takumi` feature **默认关**但 CLI 的 `default` 带上它）。**`chrome` 那条截图通道已整条删除** —— `normalize_cover_renderer` 对 `"chrome"` 给**单独的**「这条腿已删除」错误（**不是**折进"认不出"）。对拍基线已**冻结**（`scripts/fixtures/cover-golden/`）：`node scripts/check-cover-takumi.mjs --golden scripts/fixtures/cover-golden`，**不再需要 chrome**；`build_cover_page_static` 是静态那条的构建入口（= 原替换链 + `__EXPORT_CSS__`）。依据、四引擎差异与残差见 §3 / `docs/plan/takumi-cover-capability.md`。
  - ⚠️ 当年那条"零回归由构造保证"的说法**随 chrome 那条腿一起作废** —— 它描述的是一条已经不存在的路径，别再照着它推。
  - ⚠️ 模板里 `:root{}` 那个变量块**必须留**：静态渲染器不跑 JS，而变量的唯一来源曾是页面脚本的 `setProperty` —— 缺了它所有 `var(--…)` 声明**整条被丢**（标题没字号没描边、背景没模糊，实测 38.44/255）。
  - ⚠️ **字体**：Takumi 不读系统字体，靠 `cover_font_files()` 把字体库全量注册；一个字体都没有时**明确报错**（静默出豆腐块是最坏形态）。`VTEDIT_COVER_RENDERER` 认不出的值也**当场报错、不静默回退**。
  - **怎么用**：CLI `yekitrim polish cover <目录> --renderer takumi`（不传 = 默认链）；WebUI 点「生成封面」直接出图（**通道弹窗已删**，前端不传 `renderer`）。取值口径只在 `cover.rs`（显式参数 > `VTEDIT_COVER_RENDERER` > **takumi**；静态通道**能不能用**由 `cover_static_capability()` 给出，经 `/api/health.cover_static` 供前端置灰 + 写原因）。
- 封面特效 emoji：`text` 上限 `COVER_EMOJI_MAX_CHARS`(12)、`x/y` 是**字符中心**、`amount` 当大小（48~288px；公式只在 `cover_fx_html` 一份，前端 `fxStyle` 是镜像）；字体栈要带 `Segoe UI Emoji / Apple Color Emoji / Noto Color Emoji`。
  ⚠️ **标题文本里的 emoji 是另一条路**（用户报的「前后端 emoji 字体不一致」+「渲染的 emoji 没有描边」都在它身上）：它是 `title_lines` 里的**联排字符**，特效层那套**一行都不落在它身上**。收口 = `cover.rs::wrap_emoji_runs` + `emoji_run_span`（包 `<span class="cov-emoji">`、族名用**库 id**、4 向 `drop-shadow`、**必须 `display:inline-block`**）。前端 `CoverDesigner.vue::wrapEmojiRuns` 是镜像，**改一边要改另一边**；两条路的细节与证据链见 §6 / `docs/plan/takumi-cover-capability.md` §15。

- ⚠️ **资产布局回落**（`tests/asset_layout_fallback.rs`）：素材库收拢到 `resources/{stickers,videos}` 后，**存量安装靠"读两个位置"才不坏**（老位置仍列得出来、新位一出现就以它为准、视频库不再嵌套在贴纸库下；均有变异自证）。见 `docs/file-layout.md`。
- ⚠️ **两条真像素探针**（`tests/portrait_*.rs`，`#[ignore]`）：`portrait_stage_fit`（竖素材进横画布必须 contain + 运镜被吃掉）与 `portrait_backdrop`（挪开处必须是模糊衬底、不是黑）。**harness 的三条口径**（抽帧要抽成片、变异锚点落在语义位置、衬底判据按开关决定断或明说跳过）见 §7 / `docs/plan/polish-vertical-canvas.md` §十。

- ⚠️ **画质刻意不提供「质量档/分辨率/码率」配置**（E7）：由底座 + ffmpeg 编码默认决定（合流 `-c:v copy`）；伪旋钮只会让人怀疑没生效。正文见 `project/config.rs` 的 `[project]` 注释。

> 改动只允许落在本目录；跨模块改动先读对方卡，并跑两条门禁。

## 验证

```bash
cargo check -p yekitrim-polish
# takumi 字体验收：tests/takumi_font_id.rs 被 #[cfg(feature = "cover-takumi")] 门控、默认跑 0 条（E4b）
cargo test -p yekitrim-polish --features cover-takumi
```

0 warnings。动了 `templates/index.html` / `cover.html` 之后必须再真跑一次出片：预览看着正常 ≠ 出片正常。
模板侧漂移对拍与真像素探针（`check-*.mjs` 五条 + `--test portrait_*`）命令见 `templates/AGENTS.md` 的「验证」。
