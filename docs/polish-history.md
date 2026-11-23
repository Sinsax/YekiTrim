# `crates/yekitrim-polish` 卡外迁正文（cover 通道 / 图层栈 / 预览代理 / 出片几何 / 探针 / 模板事故）

> 配套模块卡：`crates/yekitrim-polish/AGENTS.md`（DSH 在触到 `crates/yekitrim-polish/` 时自动注入）。
> 这份文档**不占指令预算**，是那张卡的外迁正文：卡里只留"改代码前必须知道、否则会写错"的规矩句与 ⚠️ 行；
> 事故复盘、实测数字、历史成因、公式推导与"为什么当初那样写"的叙述全部搬到这里。
> 判断标准：**改代码前必须知道、否则会写错的留卡；想追根问底时才看的进这里。**
> 相邻文档：`docs/plan/takumi-cover-capability.md`（Takumi 能力面 / 四引擎差异 / §15 emoji 证据链）、
> `docs/plan/cover-dhampir-tradeoff.md`（封面为什么不接底座）、`docs/plan/polish-vertical-canvas.md`（竖屏与真像素探针）、
> `docs/file-layout.md`（资产布局）、`crates/yekitrim-polish/templates/AGENTS.md`（模板侧漂移对拍）。

分节：

- §1 emoji 分类的作者入口（程序资产，没有 CLI 面）
- §2 预览代理（`_proxy_720.mp4`）的实测数字与四条能力
- §3 封面渲染通道：chrome 删除、切 takumi 的依据与四引擎差异
- §4 图层栈：默认栈零注入的由来、SHA 基线与三条 stacking context 规矩
- §5 封面出片几何：headless 视口 1904×929 的老 bug
- §6 封面 emoji 的两条路与证据链
- §7 资产布局回落与两条真像素探针的 harness 口径
- §8 长任务写 `polish.toml` 的覆写事故与"锁内重新 load"的由来
- §9 `templates/index.html` 的事故复盘（`ReferenceError` / DOM 与 canvas 分叉）
- §10 默认对齐 `left` 的迁移与兜底分支的陷阱

---

## §1 emoji 分类的作者入口（程序资产，没有 CLI 面）

2026-10-17 用户口径：「分类作为内建，交给程序处理，不给用户重建和更改的权限」。所以 `cmd_emoji_build(...)`
只剩**作者入口**，没有 clap 子命令、也不该再加：

```bash
VT_EMOJI_BUILD=1 cargo test -p yekitrim-polish --lib emoji_build_dev_tool -- --ignored --nocapture
```

- 它用 LLM 把内嵌的 **881 条**重做六维分类（三轮补漏 **40 → 20 → 10** + 三层键兜底）；
- **写回仓库里的** `assets/emoji_catalog.json`（编译期输入 ⇒ 跑完要 `cargo build`）；
- 写盘前留 `.bak`；默认**增量**（已有 `dim` 的跳过），`VT_EMOJI_ALL=1` 才全量重做。

## §2 预览代理（`_proxy_720.mp4`）的实测数字与四条能力

`polish init` 生成 `<片段>/_proxy_720.mp4`：720p / GOP10 / CRF20、**保持源帧率**、音轨 `copy`、幂等、
失败不阻断 init。它**只给预览那条腿读**（`dhampir::prepare_project` 的 `video` 参数），**出片永远用源**。

实测（同一片段，随机定位）：**165ms → 18ms**；体积 **0.52× 源**（全 I 帧那档是 2.2–4.2×，
容量在这里是硬约束 —— 别为了 18→7ms 把磁盘吃光）。

四条能力都由 `tests/preview_proxy.rs` 钉着：720p / 保持帧率 / 关键帧密度 / 幂等+过期回落；
`VTEDIT_NO_PROXY=1` 可整体关掉。

## §3 封面渲染通道：chrome 删除、切 takumi 的依据与四引擎差异

**时间线**：

1. 封面长期走 **headless Chrome 截图**那条腿（`chrome`），2026-10-01 用户拍板后**整条删除**；
2. 现在**只剩一条通道**：**`takumi`**（静态渲染，`src/render/takumi.rs`）；`cover-takumi` feature
   **默认关**，但 CLI 的 `default` 带上它；
3. `normalize_cover_renderer` 对 `"chrome"` 给的是**单独的**「这条腿已删除」错误（**不是**折进"认不出"：
   那样用户只会去猜拼写）；取值口径仍只在 `cover.rs`：显式参数 > `VTEDIT_COVER_RENDERER` > **takumi**，
   认不出的值**当场报错、不静默回退**；
4. 静态通道**能不能用**由 `cover::cover_static_capability()` 给出（编了 feature + 字体库非空），
   经 `/api/health.cover_static` 给前端置灰 + 写原因用。

**切 takumi 的依据**：字体三处来源收敛后重跑封面门禁（clip-14），非文字区 / 整图均**在阈值内**、
同库同字体对比（数字见 `docs/plan/takumi-cover-capability.md`）。对拍基线已**冻结**
（`scripts/fixtures/cover-golden/`，取自删除前最后一次 chrome 渲染）：

```bash
node scripts/check-cover-takumi.mjs --golden scripts/fixtures/cover-golden
```

**不再需要 chrome**。`build_cover_page_static` 是静态那条的构建入口（= 原替换链 + 填 `__EXPORT_CSS__`）。

**当年那条"零回归由构造保证"的说法已随 chrome 腿作废**：它原本是"动画把元素提升成合成层、
挂上「关动画」规则会换光栅化路径 ⇒ 存量 SHA 变"。它描述的是一条**已经不存在的路径**，别再照着它推。
留在卡里的只有结论：**模板里 `:root{}` 那个变量块必须留**（静态渲染器不跑 JS，而变量的唯一来源曾是页面
脚本的 `setProperty`；缺了它所有 `var(--…)` 声明整条被丢 —— 标题没字号没描边、背景没模糊，实测 38.44/255）。

**四个引擎差异与残差**（`@font-face` 不支持 / 无资源解析器要内联 / 复合选择器 `#id.class` 不匹配 /
忽略 `paint-order`；标题字形 ~15/255 是栅格化固有差，**适配层修不了**）的验收口径见
`docs/plan/takumi-cover-capability.md`。

**怎么用**（当前口径，卡里也留着）：CLI `yekitrim polish cover <目录> --renderer takumi`（不传 = 默认链）；
WebUI 封面设计器点「生成封面」直接出图 —— **通道弹窗已删**（只剩一个按钮的弹窗不再是"选择"），
前端不传 `renderer`。

## §4 图层栈：默认栈零注入的由来、SHA 基线与三条 stacking context 规矩

**沿革**：封面从"三层写死"变成**真图层栈**（`[cover] layers`，2026-09 起，顺序 + 可见性都落盘）。
`resolve_cover_stack(&CoverConfig)` 是**唯一推导**：id = `bg` / `frame`（关键帧层 `_cover_frame.jpg`）/
`filter`（渐变+暗角，一层两块）/ `title` / `fx:<CoverEffect::id>`；`plan_cover_layers()` 把它折成每块的
`z-index`，`cover_layer_css()` + `cover_fx_html()` 负责注入。**三层各有各的图源，谁也不许覆盖谁**。

**为什么默认栈一个 z-index 都不注入**（`is_default_stack`）：默认栈时**一个 z-index 都不注入**
（只把模板里那条 `#fx-extra{z-index:1}` 补回来）、特效还是一个 div —— 给五块都写上 z-index 会换合成路径，
缩放中的帧层重采样差 **~9/255**、**SHA-256 会变**，而"存量逐字节零回归"是本仓的硬判据。
判据：`.workbuddy/cover-layer/` 三样本改前改后 **SHA-256 相同**。

⚠️ **基线量于旧 chrome 通道**；Takumi（静态光栅化）**没有这条差异** —— 这条历史留着是为了解释
"为什么默认栈一个 z-index 都不注入"这个**设计**，不是为了证明现在还有那种漂移。

**三条 stacking context / 替换链规矩的由来**：

- **自定义栈时 `#fx-extra` 必须退回 `z-index:auto`**：它一带 z-index 就是个 stacking context，
  所有特效的 z-index 会被锁死在同一层（拖动排序看着生效、出片不动，**而且不报错**）。
- **`overlay` 特效额外加 `isolation:isolate`**（只加它、`blur` 加不得）：改造前 `#fx-extra` 是 stacking
  context，`mix-blend-mode:overlay` **没有背景可混**（实测"绿底叠 #ff3366"是橄榄绿）；拆开后它会真混合 ⇒
  **老封面会变色**。想要真混合走 P2 的「每层混合模式」（显式选择才有观感预期）。
- **`COVER_HTML` 只允许在 `build_cover_page` 里替换一次**：以前预览（`build_cover_html`）与出片
  （`cmd_cover` 内联那段）各写一份，漏改一边就是"预览有层序、出片没有"。`check-cover-stage.mjs` ⑦
  钉住"只有一条替换链"。

## §5 封面出片几何：headless 视口 1904×929 的老 bug

`cmd_cover` 的出片几何 = 模板里 `html.export` 让 `#page` 不缩放、左上角对齐视口原点，
同时 `Emulation.setDeviceMetricsOverride(1920×1080)` 把视口**撑满画布**，抓图用 `clip: (0,0,1920,1080)`。

**为什么不能退回 `clip: None`**：headless 实测视口 **1904×929** —— 页面被居中、成片带黑边（老 bug）。
`#toolbar` 在 `html.export` 下 `display:none`，否则连按钮一起抓进成片。

## §6 封面 emoji 的两条路与证据链

封面上的 emoji 有**两条互不相干的路**，混起来就出一堆怪症状：

1. **特效层 emoji**（`[[cover.effects]]` 的 `kind = "emoji"`）：`text` 最多 `COVER_EMOJI_MAX_CHARS`(12) 个字符
   （带 ZWJ/肤色修饰符的一个 emoji 就要 7~8 个码点），`x/y` 是**字符中心**，`amount` 当大小用
   （`COVER_EMOJI_MIN_PX`(48) ~ `COVER_EMOJI_MAX_PX`(288)px）；公式只在 `cover.rs::cover_fx_html` 里一份，
   前端 `fxStyle` 是镜像。字体栈要带 `Segoe UI Emoji / Apple Color Emoji / Noto Color Emoji`，
   否则出片是豆腐块。
2. **标题文本里的 emoji**（`title_lines` 里的**联排字符**，clip-14 真实配置 `直接飞出去流鼻血🩸`、
   `effects = []`）：用户报的「前后端 emoji 字体不一致」+「渲染的 emoji 没有描边」**都在它身上** ——
   特效层那套**一行都不落在它身上**。

收口 = `cover.rs::wrap_emoji_runs` + `emoji_run_span`：把 emoji 段包成 `<span class="cov-emoji">`，
族名用**库 id**（浏览器 FontFace/`@font-face` 与 Takumi 的 id 别名都按 id），描边环用 4 向 `drop-shadow`
（别加对角：链式累积会让外缘≈2.4k）（位图字形在 Takumi 里没有描边路径）+ **必须 `display:inline-block`**。
前端 `CoverDesigner.vue::wrapEmojiRuns` 是镜像，**改一边要改另一边**。
📖 证据链见 **`docs/plan/takumi-cover-capability.md` §15**。

## §7 资产布局回落与两条真像素探针的 harness 口径

**资产布局回落**（`tests/asset_layout_fallback.rs`）：素材库收拢到 `resources/{stickers,videos}` 后，
**存量安装靠"读两个位置"才不坏** —— 老位置仍列得出来、新位一出现就以它为准、视频库不再嵌套在贴纸库下
（均有变异自证）。布局表见 `docs/file-layout.md`。

**两条真像素探针**（`tests/portrait_*.rs`，`#[ignore]`，2026-10-02 修好并真跑过）：
`portrait_stage_fit`（竖素材进横画布必须 contain + 运镜被吃掉）与 `portrait_backdrop`（挪开处必须是模糊
衬底、不是黑）。**harness 的三条口径与被修掉的坑**：

1. **抽帧要抽成片**（不是抽中间帧/预览帧）；
2. **变异锚点要落在语义位置**（否则变异不生效、判据空转）；
3. **衬底判据按开关决定"断"或"明说跳过"**（不许静默算过）。

正文见 `docs/plan/polish-vertical-canvas.md` §十。

**画质为什么不给「质量档/分辨率/码率」配置**（E7）：由底座 + ffmpeg 编码默认决定（合流 `-c:v copy`）；
伪旋钮只会让人怀疑没生效，真要画质控制是新功能。正文见 `project/config.rs` 的 `[project]` 节注释。

## §8 长任务写 `polish.toml` 的覆写事故与"锁内重新 load"的由来

`project::update_polish_toml(dir, |cfg| …)` 在锁内重新 load + 只覆盖自己负责的字段：
cmd_llm 只写 segments/filter、cmd_process 只写 segments/processed、cmd_cover 只写 cover.*。

**事故形状**：以前是"开头 load、结尾整份写回" —— LLM 跑两分钟期间用户存的选框/封面/屏蔽会被结尾那次
覆写**静默吞掉**。这正是"LLM 剪辑时不能调区域"的根因（用户报的现象是"剪辑过程中设置不生效"）。

## §9 `templates/index.html` 的事故复盘（`ReferenceError` / DOM 与 canvas 分叉）

**事故一（`ReferenceError` → 整块 chunk 失败）**：A5 折行改造留下一句未声明的 `fs` ⇒
**所有带字幕的片段都导不出来、无字幕的正常**。出片逐帧路径里任何 `ReferenceError` 都是
"整块 chunk 失败 → 整个 render job 失败"。判据在 `node webui/scripts/check-stage-backdrop.mjs` 的 ③b 段
（走**去注释后的源码** —— 注释里出现同样的字样会把判据自己顶翻）。

**事故二（DOM 层与 canvas 层静默分叉）**：`templates/index.html` 是出片路径的唯一实现，canvas `renderFrame`
**不读 DOM**。只把 `_canvas.stage`（画面摆放）与 `CFG.lanes`（六条轨道的显示开关）消费在 DOM 上的话，
成片与预览会**静默分叉**：竖素材进横画布 → 成片被拉成宽屏、竖素材在成片里照旧被 zoom 拖走
（用户 2026-09-21 报的原文），预览里全程正常、不报错。两条消费点：

- `stageDrawRect`（`axis/position/scale/padding` → 画面矩形；`axis=fill` + 无 padding + 源比例 == 画布比例
  必须仍**精确等于整块画布** = 横屏存量零回归）；
- `getActiveCamera` 开头的 `!CFG.lanes.camera` 早退（`zoom=1 / handheld=0`，与 `engine.js::cameraAt` 同口径）。

⚠️ 注：`yekitrim-render-core::evaluate_frame` 是**已删除**的 wgpu 路径留下的平行实现，只归零 handheld、
且早已没有生产消费者 —— 别拿它对口径。

## §10 默认对齐 `left` 的迁移与兜底分支的陷阱

2026-09 起默认对齐由 `center` 改成 `left`（`default_cover_text_align()`）：默认 `text_x = "35%"`，
老工程里显式写过的 `text_align` 一律照旧（字段存在就不走默认）。

**陷阱**：`cover.rs::cover_text_align` 的 `_ => ("center","center")` 兜底**不能顺手改** —— 那个 `_`
同时兜着合法值 `"center"` 本身，改了会把显式居中的老封面一起翻成左对齐。

**导入背景图片的老 bug**：`/api/polish/cover-background` 写的是 `_cover_bg.jpg`（同时把 `bg_mode` 设成
`image`）。早期写回 `_cover_frame.jpg` —— 那是"背景替换关键帧"的老 bug（用户报过「又把关键帧那一层给我删了」）。
`cover.image` 也**不要**在导入时清空（它是关键帧的来源之一）。

**出片不再删 `_cover_frame.jpg`**：`cmd_cover` 末尾那句 `remove_file` 已删 —— 它就是关键帧层的图源，
删了等于①用户导出的那一帧白导、②下次出片又按 `frame_pos` 重新截、同一份封面每次换脸。

## §11 入口清单（`cover.rs` / `parse/`，逐字外迁）

> 卡里只留"哪块归谁、哪些是唯一入口"；改代码时照这份核对具体函数名与常量。

### 11.1 `cover.rs`

- `cmd_cover_template` / `cmd_preview_cover` / `cmd_cover(project_path, existing_video)`；
- `build_cover_page`（**HTML 替换链只此一份**，预览与出片都走它）/ `build_cover_html`（预览用的薄包装）；
- 图层栈一族：`resolve_cover_stack` / `plan_cover_layers` / `cover_layer_css` / `is_default_stack`；
- `cover_bg_vars` / `cover_frame_vars` / `sanitize_hex_color` / `cover_blur_css` / `cover_bg_mode`；
- 常量 `COVER_FRAME_IMG = "_cover_frame.jpg"` / `COVER_BG_IMG = "_cover_bg.jpg"` /
  `COVER_CAPTURE_W|H = 1920|1080` / `COVER_EMOJI_{DEFAULT,MAX_CHARS,MIN_PX,MAX_PX}`。

### 11.2 `parse/`

- `srt::{parse, parse_file, Subtitle}`；
- `text::{correct_text, apply_highlights, detect_hook, assign_danmaku_tracks}`（后者的在屏窗口规矩在卡里，别删）；
- `charlog.rs`：**字级时间戳搬运**（E4b，`asr/timeline.json` → `data/char-timeline.json`；入口 `commands.rs::sync_char_timeline`）。

### 11.3 `commands.rs` 的快照索引

- `cmd_llm(project_path, full)` 见 `commands.rs:200`；`cmd_preview(project_path, mode)` 见 `commands.rs:296`。
- 这两个行号是给"读快照的 agent"用的定位锚点，改函数体后要一起更新。

---
