# render/ — 出片链路（headless Chrome 逐帧 + ffmpeg 合成）与预览 HTTP server

> 范围：`crates/vtrim-polish/src/render`/** ｜ 上游：`commands::cmd_preview` / `cover::cmd_cover` / `vtrim-server::api`（渲染任务）｜ 验证：`cargo test -p vtrim-polish --lib render`

## 职责

把精修工程变成成片：`pipeline.rs::cmd_render` 起本地预览 server → headless Chrome 打开
`templates/index.html` 逐块（chunk）截帧/逐帧 canvas 绘制 → ffmpeg 编码、混 SFX、合流、按屏蔽区间重编码
→ 统计；`cover.rs` 走同一个 server 的另一条路由出封面。`server.rs` 同时是**预览 HTTP server**（ServeDir 根 = 项目目录，
`/vt-fonts/:name` 给库内字体一条自己的路由，`@font-face` 注入两个模板）；`chrome.rs` 管浏览器与卡死守卫；
`ffmpeg.rs` 管编码器探测与全部 ffmpeg 调用；`frame.rs`/`gif.rs`/`sound.rs`/`backend.rs`/`context.rs` 是底层件；
`native/`（`WgpuBackend`）在 `wgpu-render` feature 下才有，**chrome-render 是默认出片路径**。

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
- `frame`：`extract_chunk_frames`、`split_jpeg_frames`；`gif`：`decode_gif_frames_rgba` / `decode_gif_frames_png`；
  `sound`：`collect_sound_events(config)` / `sound_file_exists`；
  `backend::create_backend`；`native::WgpuBackend`（feature `wgpu-render`）。
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
    实测「起服务」16.1s → 0.1s。
  - `sticker_frames.json`（`prepare_sticker_data` 的 GIF 解码结果）——6 个贴纸解 40.4s → 命中 1.67s
    （产物 95MB，断言过冷/热输出一致）。贴纸多一个就多一份解码，别把缓存去掉。
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
- `render_chunk_sync` 的编码失败按 `chunk#{idx} 编码失败（ffmpeg 退出码 …）` 报错；
  `native/`（wgpu）那条链**未启用**，模板 canvas 是哪条出片路径的权威 ——
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
