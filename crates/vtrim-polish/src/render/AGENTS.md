# render/ — 出片链路（headless Chrome 逐帧 + ffmpeg 合成）与预览 HTTP server

> 范围：`crates/vtrim-polish/src/render`/** ｜ 上游：`commands::cmd_preview` / `cover::cmd_cover` / `vtrim-server::api`（渲染任务）｜ 验证：`cargo test -p vtrim-polish --lib render`

## 职责

把精修工程变成成片：`pipeline.rs::cmd_render` 起本地预览 server → headless Chrome 打开
`templates/index.html` 逐块（chunk）截帧/逐帧 canvas 绘制 → ffmpeg 编码、混 SFX、合流、按屏蔽区间重编码
→ 统计；`cover.rs` 走同一个 server 的另一条路由出封面。`server.rs` 同时是**预览 HTTP server**（ServeDir 根 = 项目目录，
`/vt-fonts/:name` 给库内字体一条自己的路由，`@font-face` 注入两个模板）；`chrome.rs` 管浏览器与卡死守卫；
`ffmpeg.rs` 管编码器探测与全部 ffmpeg 调用；`frame.rs`/`gif.rs`/`sound.rs`/`backend.rs`/`context.rs` 是底层件。
⚠️ **`takumi.rs` 是封面那条的第三个后端**（**静态封面渲染**：不跑浏览器、不跑 JS/时间）——
`cover-takumi` feature **默认关**，只由 `VTEDIT_COVER_RENDERER=takumi` + 该 feature 启用；它只做
`from_html` + `StyleSheet::parse` + 字体注册 + **四处引擎差异适配**，产物与 Chrome 同口径（JPEG q85）。
判定、残差与复跑命令见 **`docs/plan/takumi-cover-capability.md`**（见 polish 卡末段）。
⚠️ **出片有两条通道**：`chrome`（本仓 headless Chrome 逐帧 canvas，**兜底**）与
`dhampir`（**默认**：独立仓的 `dhampir.exe render`，见 `render/dhampir.rs`、`RenderBackendKind`）。
⚠️ **兜底通道自 2026-10-01 起是冻结的**（S4：新效果只加进 doc 那条链路，见
`docs/plan/effect-pipeline-unification.md` §3.1）：`cmd_render` 在选 `chrome` 时会拿效果词汇表的
`anchors.canvas` 当支持矩阵，工程里用到矩阵外的效果就**当场报错并点名**（以前是静默少画）；
词汇表运行时读包内/开发树那份、读不到用编译期内嵌那份，**读不出来只警告不拦**（一个坏文件不该挡出片）。
原来的 `native/`（`WgpuBackend`）与 `wgpu-render` feature 已整条删除（GPU 那侧交给底座）。
`backend.rs` 的 `FrameRenderer` 抽象**留着** —— 但底座**不走它**：底座是"整段一起出"，
硬塞进逐帧接口就得每帧起一个进程；它走 `cmd_render` 里的**另一条分支**，产物落在同一个
`_temp_video.mp4` 上，后面的音轨链路一行不改（**画面归底座、声音归本仓** `--no-audio`）。

## 入口

- `pipeline`：`cmd_render(project_path)`、`render_chunk_sync(...)`、`resolve_paths(project_path, config) -> ResolvedPaths`、
  `inject_assets(project_path, config_json)`、`prepare_sticker_data(project_path, config)`、
  `unique_output_path(path)`、`get_video_size` / `get_video_duration`、
  `measure_loudness_curve(_cached)` / `measure_segment_loudness` / `measure_audio_loudness`、
  `export_sound_data(url)`、`print_final_stats`、`cleanup`。
- `server`：`RenderServer::{new, build_html, start_with, start, start_cover}`、`start_file_server(project_path, injected_html)`、
  `build_sticker_map(project_path)`、`build_cover_html`、`font_face_css` / `font_face_css_in` /
  `font_family_with_stack` / `SYSTEM_FONT_STACK`。
- `ffmpeg`：`detect_encoder() -> EncoderInfo`、`concat`、`extract_audio`、`apply_limiter`、
  `mix_sfx(..., &[SfxEvent])`、`cut_ranges(input, ranges, output)`、`mux(video, audio, output)`。
- `chrome`：`ChromeRenderer`、`launch(size)`、`create_tab(browser, url)`、`wait_for_ready(tab)`、
  `ChromeBackend`、`DEFAULT_STALL_SECS = 180`。
- `frame`：`extract_chunk_frames`、`split_jpeg_frames`；`gif`：`decode_gif_frames_rgba` / `decode_gif_frames_png` /
  **`decode_sticker_frames_png`（贴纸图统一入口：GIF 多帧、PNG/JPEG 单帧）**；
  `sound`：`collect_sound_events(config)` / `sound_file_exists`（⚠️ `collect_sound_events` 现在没有生产
  调用者 —— 它当年是 wgpu 那条路用的"替代页面 `__SOUND_DATA`"；留着是因为它有单测、且任何非页面后端
  都要这份 SFX 列表）；`backend::create_backend`（`Chrome` 那条实现；`Dhampir` 走到这里会**明确报错** ——
  它是整段后端，不走逐帧接口）。
  ⚠️ **`runner.rs` 已删除**（2026-09）：它自带一份 `ResolvedPaths` + `RenderRunner`，全仓库零消费者
  （真正的 `ResolvedPaths`/`resolve_paths` 在 `pipeline.rs`，经 `render/mod.rs` 的 `pub use pipeline::*` 出去），
  整模块只有一个 `#[allow(dead_code)]` 撑着。要加回"提帧 runner"先确认有第二条链路需要它。

## 边界

**这是一条子进程链路，出过两次"任务永远运行中、日志里一行都没有"的事故，改之前先读完这节。**

- **输出必须落文件，不能用管道**：`api::run_polish_child`（`vtrim-server`）把子进程 stdout/stderr 重定向到
  `<data_dir>/logs/render-<job_id>.log`，再用 `follow_render_output` 边跑边读。
  ⚠ 绝对不要改回 `Command::output()`/管道：Chrome 与 ffmpeg 是**孙进程**，它们会继承 stdout
  句柄 —— 只要还有一个活着，管道就永远不 EOF，`output()` 一直等下去。实测子进程早已带着
  `Render error: evDur is not defined` 退出，任务却卡了半小时、错误全被闷在缓冲区里。
- ⚠️ **底座那条腿（`dhampir`，**默认通道**）的三条实测规矩**（`render/dhampir.rs`，2026-09-30 全链真跑过）：
  - **帧数从转译产物里读**（`sequence_frames` = 所有层 `end` 的最大值），**不许拿上层那个
    `duration × 30` 的估算**：转译器写的时间基是**素材自己的**（demo 是 `60/1`），拿 30 算 `--to`
    会让 12 秒的片子**只出 6 秒**，而音轨照旧 12 秒（合流之后更难看）。实测踩到过。
  - **底座的 stdout 落文件 + 按 offset tail，不许用管道**：它打 NDJSON（`start`/`progress`/`done`），
    要翻成 `frames A..B: NN%` 才有人认（不翻的话 WebUI 全程 0%）。它同样会拉起 ffmpeg 孙进程，
    所以照外层那条纪律：写进 `_temp_dhampir_progress.ndjson` 再 tail（带 `_temp_` 前缀，
    失败时被 `cleanup_render_leftovers` 收掉）；stderr 直接继承，底座的 warn 原样进任务日志。
  - **`cmd_render` 开头必须把工程目录绝对化**（`std::path::absolute`）：`resolve_paths` 把
    `project.output` 归一成 `<工程目录>/export.mp4`，工程目录是**相对**的时候，合流前那一步
    "不是绝对路径就再 join 一次工程目录"会把目录名**拼两遍**（`<dir>\<dir>\export.mp4`
    → `No such file or directory`）。CLI 的默认输入就是 `.`，所以这条一定会被踩到。
  - **找二进制时两种失败含义不同**：`VTEDIT_DHAMPIR_CLI` 指错 = 当场报错；`<程序目录>/dhampir/bin/`
    不在 = **继续往下找**（同层 `Dhampir/target/{release,debug}` → PATH）。混成一条 `bail!`
    会让源码树下**永远找不到底座**（`current_exe()` 总能算出那个约定路径）。
  - ⚠️ **屏蔽区间只能剪一次**：出片那条腿转译时带 `--keep-muted`（时间线**整条**保留），
    由 `cmd_render` 尾部那道 `Ffmpeg::cut_ranges` 按**源时间**剪；预览那条腿（`polish_dhampir_project`）
    相反，必须留压缩时间线（客户端要跳）。**两边都剪 = 成片短一大截 + 音轨错位**
    （实测：12s 的片子带 3s 屏蔽，修复前出 6.0s、修复后 9.0s = 与 chrome 逐毫秒一致）。
  - ⚠️ **字体：栈的顺序 + 粗体，两件都不能漏**（2026-10-01 修完"字体不对"）：
    `find_font(style.subtitle.font) -> (常规, Option<粗体>)`，逐支按
    **① 字体库（`vtrim_core::fonts::resolve_file`，id = 文件名去扩展名）
    → ② 系统字体目录（族名归一化包含 + 别名表：`Microsoft YaHei` → `msyh.ttc`）** 解析；
    要按的顺序 = 项目选的那支 + **`render::server::SYSTEM_FONT_STACK`**，而那个常量**必须与
    `webui/src/components/PolishPreview.vue` 的 `.cap` 栈同序**（它俩曾经不一致：常量把 Noto 排在雅黑前、
    预览是雅黑在前 ⇒ 同一个工程**预览是雅黑、成片是 Noto** —— 用户报的"字体不对"）。
    整个字体库照旧交给底座（`--font-dir`，它按每条轨的 `font_family` 在目录里找）。
    ⚠️ **`--font-bold-file` 必须传**：转译器给字幕写 `font_weight: 700`、弹幕 `600`（与预览的
    `.cap{font-weight:700}` / `.dm{font-weight:600}` 同口径），而底座**不合成粗体** ——
    它只在 `>= 600` 时去要这个文件，拿不到就退回常规体 ⇒ **成片笔画比预览细一圈**（底座 schema 注释里
    就写着这句）。所以 `find_font` 要配出**同一族的粗体**（`msyh.ttc` → `msyhbd.ttc`；可变字体如
    `NotoSansSC-VF.ttf` 没有独立粗体 ⇒ `None`，此时 `describe()` 与就绪位会明说"没有粗体那一支"）。
  - **就绪位与严格模式**（2026-09-30 M3）：`dhampir::readiness(font)` 是**唯一的**"这台机器能不能出片"
    判断（六项各查一次文件：二进制/转译器/字体/node/预览产物/版本契约，**不渲染、不跑 node**）——
    `/api/polish/dhampir-status`、`/api/health` 的 `dhampir_ready`、`vtrim polish doctor`、前端提示
    都读它。**别各写一份**（症状会是"医生说好、渲染却失败"）。
    `render_video_only(..., strict)` = `polish render --strict`：转译报告里 `DROP > 0` 就**拒绝出片**
    并逐条列出（报告由 `prepare_project_reporting` 带回来；`TranslateReport::parse` 只认报告那几行的形状，
    认不出就当 0，**绝不让报告把出片搞崩**）。
- **分类规则**：`frames A..B: NN%` / `chunk#N: ...%` → `jobs.progress()`（按**最小值**聚合：
  最慢的分块决定整体）；`-- Xxx --`、`FINISHED:`、`Cover saved:`、`✂ 已导出去除`、`⏱` 等里程碑
  → `jobs.log(info)`（进运行日志面板与文件）；其余行 → `debug`（只挂在任务卡上，不刷日志）。
- **取消 = 杀进程树**：子进程是另一个进程，看不到进程内标志。`render_proc::kill_tree(pid)`
  在取消时 `taskkill /F /T`（含 Chrome/ffmpeg），父进程再清 `_chunk_*` / `_temp_*`。
- **启动清扫孤儿**：每次渲染往 `<data_dir>/render-procs/<job_id>.pid` 登记 pid，
  `server::run` 启动时 `render_proc::sweep_orphans` 结束上一次遗留的渲染（**先核对命令行**
  再杀，pid 会被系统回收）。服务器重启带不走子进程 —— 实测两个渲染各留 3 个 Chrome 空转两小时，
  把后续渲染拖到根本跑不完。
- **卡死守卫**：页面侧 `capture_and_read` 记录 `__CAPTURE_PROGRESS` 的最后变化时间，
  超过 `VTEDIT_STALL_SECS`（默认 180s，`chrome::DEFAULT_STALL_SECS`）报错退出；父进程另有
  "子进程 60s 无输出 → warn"。别再退回**无超时轮询**：那是"长时间无反应 + 日志空白"的直接来源。
- **分阶段打点**：`cmd_render` 每段一条 `⏱ 阶段 {:.1}s`，`render_chunk_sync` 把每块的
  **抽帧 / 渲染 / 编码**分开报（`chunk#N: done N frames（抽帧 …s / 渲染 …s / 编码 …s）`）。
  实测 clip-35（262s 片段，debug 构建）294s 里：起服务 16.1s、贴纸 GIF 40.8s、分块 210.7s
  （每块 抽帧 ~30s + 渲染 145~171s + 编码 8s）、音效混音 9.3s、合流 2.7s、屏蔽重编码 9.2s、封面 1.1s。
  出片慢先看这几条，别再猜。
- **带磁盘缓存的预计算**（都在 `<工程>/data/`，按「文件名 + mtime + 大小」失效）：
  - `loudness.json`（`render::measure_loudness_curve_cached`，出片页与混音共用一份）——
    预览服务的 HTML 是**每个请求**重建的，不缓存等于每刷一次页面烧一遍整段 ebur128；
    实测「起服务」16.1s → 0.1s。⚠️ 量它那条 ebur128 **必须带 `-vn`**（只要音频：同一片段
    5.6s → 0.4s，曲线逐点一致）—— 剪切收尾会给每个新片段预热这份缓存
    （`vtrim-service::core_pipeline::warm_loudness_cache`，只补缺失），热路径上一次剪切跑 N 遍。
  - `sticker_frames.json`（`prepare_sticker_data` 的解码结果）——6 个贴纸解 40.4s → 命中 1.67s
    （产物 95MB，断言过冷/热输出一致）。贴纸多一个就多一份解码，别把缓存去掉。
    ⚠️ **贴纸图不止 GIF**：库里允许 `gif/png/jpg/jpeg/webp`（判据只有 `sticker_index::is_image_path`
    一份），所以按 `<name>.<扩展名>` 找图、静态图当**单帧**（`render::gif::decode_sticker_frames_png`）。
    以前写死 `<name>.gif` —— PNG 贴纸会被复制进项目、DOM 预览的 `<img>` 也显示得出来，
    **成片里却一张都没有**（静默消失）。找不到图 / 解不出帧现在都打 warn，不再是无声的。
    钉子：`cargo test -p vtrim-polish --lib static_image_sticker_still_gets_frames`。
  - 诊断：`VT_POLISH_DIR=<项目> cargo test -p vtrim-polish --lib diagnose_sticker_cache -- --ignored --nocapture`
- **屏蔽区间导出走硬件编码**：`Ffmpeg::cut_ranges` 是整段重编码，用 `detect_encoder()`
  的结果（libx264 时 17s → NVENC 9s）。别写回硬编码。
- **模板里的工具函数必须放在脚本顶层**：`templates/index.html` 里 canvas 出片那一段是
  独立的 IIFE，看不到 `init()` 内部的东西。`evDur` 曾放在 `init()` 里、被 IIFE 的
  `buildIndex()` 调用 → 每次出片都抛 `evDur is not defined`（GSAP 预览路径却完全正常，
  所以只在出片时炸）。新增给两条路径共用的函数时，放顶层并留注释。
  **模板改完必须真跑一次出片验证**（预览看着正常 ≠ 出片正常）。
  ⚠️ **改了 `templates/index.html` 必须重建二进制**：它经 `include_str!` 进
  `render/server.rs::INDEX_HTML`，运行中的 `vtrim server` 端出的仍是旧页面。
- **字体库的渲染侧**：出片页面由渲染服务器端给 headless Chrome，所以库里字体必须有**自己的一条路由**
  —— `/vt-fonts/:name`（只认库目录下的单段文件名）。**两条建 router 的路都要挂**
  （`start_file_server` 提帧/封面截图 + `start_with` 预览/CLI 出片），漏一条就是"预览有、成片没有"。
  **浏览器只认系统已装字体**，所以选了库内字体就**必须注入 `@font-face`**：`server::font_face_css(...)`
  拼出来的片段进两个模板的 `<!-- __FONT_FACE__ -->` 占位（`index.html` 由 `build_html` 注入字幕字体、
  `cover.html` 由 `cover.rs` 的 `cover_font_css` 注入封面字体，**两条 replace 链都要改**）。
  **声明的字族名 = 配置里存的那串 id**，所以 `index.html` 里
  `.cap{font-family:"<style.subtitle.font>",...}` **一行都不用改**；`cover.html` 的字体栈改成了
  `__FONT_FAMILY__` 占位（空串时替换回系统栈）。库外/解析不到的值**什么都不注入** ——
  注入了就是一条 404 的 @font-face，老工程的 `font = "LXGW WenKai"` 行为必须保持原样。
  ⚠️ **别改成"把字体拷进项目"**（贴纸那套 `copy_sticker_from_local` 是另一回事）。
- **模板里两条"四处/两份实现"的契约**（改 `templates/index.html` 或 `webui/src/polish/engine.js` 前必读）：
  - **覆盖层色档**：`overlay_type == "gradient"` 的有效色档规则见 `vtrim-render-core/src/timeline.rs::overlay_stops`；
  四处实现为 `timeline.rs`、`templates/index.html`（**出片权威路径**）、`engine.js`、`PolishPreview.vue`。
  另两档 `fade_in` / `fade_out`（黑场淡入/淡出，2026-09 新增，内建 `intro-black` 随之退役）**不铺色档**：
  整窗就是过渡（`opacity × (1 − dt/dur)`，`fade_out` 镜像）、**不叠 0.4s 包络** —— 分支必须排在包络之前，
  否则起点不透明度被乘成 0、「开头渐入没生效」。片头那份由 `process` **只在初次后处理**补 `fade_in` 事件，对拍同上。
  ⚠ 求值器**必须保证返回 ≥2 档且无空串**（`n == 1` 是 `0/0 = NaN` 抛 `IndexSizeError`、空串色档抛
  `SyntaxError`，两者都会**打断整个 chunk 的渲染**）；⚠ 兜底出来的第二档必须是**同色零透明**
  （`overlay_fade_stop` / `overlayFadeStop`），**不能用 CSS 的 `transparent`** —— canvas 渐变在
  **非预乘**空间插值，写 `transparent` 会让浏览器预览与出片观感不一致。对拍：`node webui/scripts/check-overlay.mjs`。
  - **字幕限宽折行（A5）**：出片权威 `templates/index.html::wrapCaptionParts` 与编辑器
  `webui/src/polish/engine.js::wrapCaptionParts` 必须逐字一致；上限 3 行（`MAX_LINES`）、
  超 3 行按 `MIN_RATIO=0.7` 缩字号；安全宽 = 画布宽 × 0.875（竖屏 945px）。
  ⚠️ 曾经 `append` 先读 `cur.length` 再判溢出，而溢出分支的 `pushLine()` 会把 `cur` 置空 →
  **任何一次折行都抛 `TypeError: can't access property "length", cur is null`**（"竖屏一播到长字幕就崩"，已修）。
  对拍：`node webui/scripts/check-caption-wrap.mjs`。
  ⚠ 两处 DOM/GSAP 预览的渐变 CSS **结尾那个 `)` 不能省** —— 少了它整条 `background`
  非法被浏览器丢弃，表现正是"面板里设了色档、预览里什么都没有"（`engine.js::gradientCss` 一份，
  模板里 `overlayGradientCss` 一份）。径向语义 = **中心该色、边缘淡出**（canvas 是
  `createRadialGradient(960,540,0,…,960)`）；不透明度一律用**数值判定**（`typeof ev.opacity === 'number'`），
  **别写 `ev.opacity || 1`**：0 是合法值，falsy 判定会把它变成全不透明。
- **前端留痕纪律（别再让信息"静默消失"）**：
  - `api()`（`webui/src/api/client.js`）是唯一请求入口，失败统一 `console.warn` +
    写日志面板（≥500 记 error，其余 warn），成功只写 `debug`。
    可选 `{ label: '读取切片列表' }` 给失败配上业务上下文。
  - **读取类兜底用 `apiOr(path, fallback, label)`**，不要写裸 `api(...).catch(() => [])`：
    后者把"读取失败"伪装成"没有数据"，页面与日志里都看不出异常。包装器
    （`projectData.*` / `fetchSubtitles` / `fetchProject` / `fetchStickerSources`）
    都收一个 `label` 参数，视图侧传中文说明。
  - 非 API 的失败（裸 fetch、SSE、解析）用 `stores/log.js` 的 `addClient(level, text, source)`；
    高频可预期失败（弹幕文件缺失、字体回退、任务轮询重试）记 `debug`，不要刷 warn。
  - 不做 toast→日志桥（err toast 本来就可见，桥接只会和 `api()` 的记录重复）。
- `render_chunk_sync` 的编码失败按 `chunk#{idx} 编码失败（ffmpeg 退出码 …）` 报错。
  模板 canvas 是**唯一**的出片路径（原 `native/` wgpu 那条链已删除，见上）——
  ⚠ 模板 canvas 路径曾硬编码 `1.8/1.5` 且完全不读 `[style.camera]`，而它才是出片路径，
  别再把倍率写回去（倍率只有一张表 `CameraStyle::factor`）。

> 改动只允许落在本目录；跨模块改动先读对方卡，并跑两条门禁。

## 验证

```bash
cargo test -p vtrim-polish --lib render
```

含 `render::server::tests`（`@font-face` 注入/不注入、模板占位符还在）。改模板或 ffmpeg 调用后
必须再真跑一次出片：`node webui/scripts/check-overlay.mjs` 与
`node webui/scripts/check-caption-wrap.mjs` 是对拍门禁（预览看着正常 ≠ 出片正常）。
