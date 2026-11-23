# render/ — 出片链路（底座 Dhampir 整段出片 + 本仓音轨/合流）与预览 HTTP server

> 范围：`crates/yekitrim-polish/src/render`/** ｜ 上游：`commands::cmd_preview` / `cover::cmd_cover` / `yekitrim-server::api`（渲染任务）｜ 验证：`cargo test -p yekitrim-polish --lib render`

## 职责

把精修工程变成成片：`pipeline.rs::cmd_render` 把 `polish.toml` 转译成底座工程（`dhampir.rs` → `tools/polish-to-dhampir.mjs`）→ 调 `dhampir.exe render` 整段出**画面** → ffmpeg 编混 SFX、合流、按屏蔽区间重编码 → 统计。`server.rs` 是**预览 HTTP server**（ServeDir 根 = 项目目录，`/vt-fonts/:name` 给库内字体一条自己的路由，`@font-face` 注入模板）；`ffmpeg.rs` 管编码器探测与全部 ffmpeg 调用；`frame.rs`/`gif.rs`/`sound.rs`/`backend.rs`/`context.rs` 是底层件。
⚠️ **`takumi.rs` 是封面那条的后端**（**静态封面渲染**：不跑浏览器、不跑 JS/时间）—— `cover-takumi` feature **默认关**（yekitrim-cli 的 `default` 打开）；它只做 `from_html` + `StyleSheet::parse` + 字体注册 + **四处引擎差异适配**，产物是 JPEG q85。判定、残差与复跑命令见 `docs/plan/takumi-cover-capability.md`。
⚠️⚠️ **出片只剩一条腿（2026-10-01 用户拍板）**：`dhampir`（独立仓的 `dhampir.exe render`，见 `render/dhampir.rs`、`RenderBackendKind`）。**本仓那条 headless Chrome 逐帧腿、`native/` 与逐帧接缝已整条删除**；`RenderBackendKind::parse("chrome")` 现在返回 `None`，`cmd_render` 会**明确报错**（**不静默改走底座**：那会让人以为"我在用 chrome 出片"）。`wgpu` / `native` 仍只作 `dhampir` 的**别名**（老配置兼容）。删除清单与"真要接第二个逐帧后端"的提示见 `docs/polish-render.md` §1。
  ⚠️ 封面那条对 `chrome` 给的是**单独的**「这条腿已删除」错误，**不是**折进"认不出"（那样用户只会去猜拼写）。
底座是"整段一起出"，走 `cmd_render` 里的**另一条分支**，产物落在同一个 `_temp_video.mp4` 上，后面的音轨链路一行不改（**画面归底座、声音归本仓** `--no-audio`）。

> 历史与实测数据见 **`docs/polish-render.md`**（§1~§9：只剩一条腿的删除清单 / 分阶段实测 / 缓存实测 / 字体注入 bug / Takumi 字体口径 / 色档与折行事故 / `runner.rs` 删除 / 子进程两起事故 / 前端留痕纪律）。

## 入口

- `pipeline`：`cmd_render(project_path)`、`resolve_paths(project_path, config) -> ResolvedPaths`、`inject_assets(project_path, config_json)`、`prepare_sticker_data(project_path, config)`、`unique_output_path(path)`、`get_video_size` / `get_video_duration`、`measure_loudness_curve(_cached)` / `measure_segment_loudness` / `measure_audio_loudness`、`spec_effects_path`、`print_final_stats`、`cleanup`。
- `server`：`RenderServer::{new, build_html, start_with, start, start_cover}`、`start_file_server(project_path, injected_html)`、`build_sticker_map(project_path)`、`build_cover_html`、`font_face_css` / `font_face_css_in` / `font_family_with_stack` / **`fallback_stack`（动态兜底栈，唯一权威）** / `SYSTEM_FONT_STACK`（空库回落态常量）。
- `ffmpeg`：`detect_encoder() -> EncoderInfo`、`concat`、`extract_audio`、`apply_limiter`、`mix_sfx(..., &[SfxEvent])`、`cut_ranges(input, ranges, output)`、`mux(video, audio, output)`。
- `dhampir`：`render_video_only(project_path, clip_dir, project_font, strict)`、`prepare_project(_reporting)`、`readiness(font, strict)`、`find_translator()`（`VTEDIT_TRANSLATOR` 可覆盖 —— 手动探针靠它做变异）。
- `frame` / `gif` / `sound`：`extract_chunk_frames`、`split_jpeg_frames`；**`decode_sticker_frames_png`（贴纸图统一入口：GIF 多帧、PNG/JPEG 单帧）**；`collect_sound_events(config)`（**cmd_render 的 SFX 收口**：底座腿不带页面，事件由 Rust 直接收，响度对齐后 `Ffmpeg::mix_sfx` 混入 —— "画面归底座、声音归本仓"；缺文件会打 warn 并跳过该条，不再静默）/ `sound_file_exists`（暂无消费者，判据与 collect 同源）。
  ⚠️ **`runner.rs` 已删除**（2026-09）：它自带一份 `ResolvedPaths` + `RenderRunner`，全仓库零消费者。要加回"提帧 runner"先确认有第二条链路需要它（见 `docs/polish-render.md` §7）。

## 边界

**这是一条子进程链路，出过两次"任务永远运行中、日志里一行都没有"的事故，改之前先读完这节。**

- **输出必须落文件，不能用管道**：`api::run_polish_child`（`yekitrim-server`）把子进程 stdout/stderr 重定向到 `<data_dir>/logs/render-<job_id>.log`，再用 `follow_render_output` 边跑边读。
  ⚠ 绝对不要改回 `Command::output()`/管道：ffmpeg 与底座拉起的孙进程会继承 stdout 句柄 —— 只要还有一个活着，管道就永远不 EOF（实测子进程早已退出、任务却卡了半小时，错误全被闷在缓冲区里，见 §8）。
- ⚠️ **底座那条腿（`dhampir`，默认通道）的实测规矩** —— 帧数从转译产物里读（不许拿上层 `duration × 30` 估算）、stdout 落文件按 offset tail（不许用管道）、`cmd_render` 开头必须把工程目录绝对化、找二进制时"指错"与"不在"两种失败含义不同、屏蔽区间**只能剪一次**、字体栈顺序 + 粗体两件都不能漏、就绪位与严格模式只此一份。**七条的正文与实测数字见 `docs/plan/render-dhampir-rules.md`。**
- **分类规则**：`frames A..B: NN%` / `chunk#N: ...%` → `jobs.progress()`（按**最小值**聚合：最慢的分块决定整体）；`-- Xxx --`、`FINISHED:`、`Cover saved:`、`✂ 已导出去除`、`⏱` 等里程碑 → `jobs.log(info)`；其余行 → `debug`（只挂在任务卡上，不刷日志）。
  ⚠️ **例外**：底座带 `--no-audio` 时对自己工程音轨发的那句「本次出片是无声的」是**误导**（声音由本仓 ffmpeg 混入，成片有声），日志层把它整行换成准确说法（`api/render.rs::is_base_no_audio_notice`，有单测钉住原文不进日志）。
- **取消 = 杀进程树**：子进程是另一个进程，看不到进程内标志。`render_proc::kill_tree(pid)` 在取消时 `taskkill /F /T`（含 ffmpeg 与底座拉起的孙进程），父进程再清 `_chunk_*` / `_temp_*` / **`_dhampir_project*`**（转译草稿的 `.json` 与 `.assets/` 目录 —— 进程树被杀时 `cleanup_project` 跑不到；`api/render.rs::cleanup_render_leftovers` 兜底）。
- ⚠️ **转译草稿 = `<工程文件名>.assets/` 子目录 + 收尾整目录删**（2026-10-18，用户报「渲染导出时中间文件过多」）：文字元素的 ASS 文件名带落点/动效/颜色 ⇒ 每改一次就是**新名字**，单个删收不干净（实测一个片段 42 个）。目录推导只有一处（`dhampir.rs::side_assets_dir`，与 `tools/polish-to-dhampir.mjs::sideDir` **逐字同一条**）；`render_video_only` 的**失败路径**也必须走到 `cleanup_project`（用闭包收口，别再加早退的 `?`）。存量安装的旧文件由 `prepare_project_reporting` 里的 `sweep_legacy_text_assets` 扫掉（判据很窄：`dhampir-text_*.ass`，别放宽成"删所有 `.ass`"）。
- **启动清扫孤儿**：每次渲染往 `<data_dir>/render-procs/<job_id>.pid` 登记 pid，`server::run` 启动时 `render_proc::sweep_orphans` 结束上一次遗留的渲染（**先核对命令行**再杀，pid 会被系统回收）。服务器重启带不走子进程（实测 6 个孙进程空转两小时，见 §8）。
- **卡死守卫**：父进程按"子进程 60s 无输出 → warn"看着底座；底座进度在 `_temp_dhampir_progress.ndjson`。别再退回**无超时轮询**（"长时间无反应 + 日志空白"的来源）。
- **分阶段打点**：`cmd_render` 每段一条 `⏱ 阶段 {:.1}s`，`render_chunk_sync` 把每块的**抽帧 / 渲染 / 编码**分开报（`chunk#N: done N frames（抽帧 …s / 渲染 …s / 编码 …s）`）。出片慢先看这几条，别再猜；clip-35 的四段实测数字见 §2。
- ⚠️ **BGM（§11）**：**只做引入、不做卡点**（无节拍网格 ⇒ 不适用项）。默认关。那块必须与音效 `if !sfx_data.is_empty()` **并列**（写里面 ⇒ 没音效的工程完全不跑）。闪避表达式的**逗号要转义 `\,`**。两条坑都只有量响度才发现，详见 `render/bgm.rs`。
- ⚠️ **封面不搭渲染的便车**（2026-10-02 用户拍板"视频导出就只导出"）：`cmd_render` 尾部原来串的 `cover::cmd_cover(...)` 已删 —— 渲染任务只出成片；封面走自己的入口（CLI `yekitrim polish cover` / `POST /api/polish/cover` 独立任务，关键帧回落 `resolved.video`）。WebUI 的 render 任务完成钩子不再重挂封面设计器（`coverKey` 只归封面任务）。`Cover saved:` 里程碑与 `latest_numbered_file("cover")` 产物统计**保持不动** —— 封面子进程照样打它。
- **带磁盘缓存的预计算**（都在 `<工程>/data/`，按「文件名 + mtime + 大小」失效）：
  - `loudness.json`（`render::measure_loudness_curve_cached`，出片页与混音共用一份）—— 预览服务的 HTML 是**每个请求**重建的，不缓存等于每刷一次页面烧一遍整段 ebur128。⚠️ 量它那条 ebur128 **必须带 `-vn`**（只要音频，曲线逐点一致）—— 剪切收尾会给每个新片段预热这份缓存（`yekitrim-service::core_pipeline::warm_loudness_cache`，只补缺失），热路径上一次剪切跑 N 遍。
  - `sticker_frames.json`（`prepare_sticker_data` 的解码结果）—— 贴纸多一个就多一份解码，别把缓存去掉。⚠️ 贴纸图不止 GIF（`is_image_path` 一份判据管 `gif/png/jpg/jpeg/webp`）：写死 `<name>.gif` 会让**PNG 贴纸在成片里一张都没有**（预览照常显示，静默消失）。现在找不到图/解不出帧都打 warn。钉子：`static_image_sticker_still_gets_frames`。
  - 实测数字（16.1s → 0.1s / 40.4s → 1.67s / 产物 95MB）与诊断命令见 §3。
- **屏蔽区间导出走硬件编码**：`Ffmpeg::cut_ranges` 是整段重编码，用 `detect_encoder()` 的结果。别写回硬编码。
  ⚠️ **`detect_encoder()` 必须真编一次才算数**（`cutting::encoder_works_with`）：只看 `ffmpeg -encoders` 会选中打不开的编码器（2026-11-22：无 GPU 机器选 nvenc ⇒ `Cannot load libcuda.so.1` ⇒ 屏蔽区间没剪掉 ⇒ 报成「成片长度与时间线不一致」）。`VTEDIT_ENCODER=cpu|vaapi` 可显式指定（VAAPI 三件套：`-vaapi_device` 在 `-i` 前、`format=nv12,hwupload`、`-qp` 且不写 `-pix_fmt`）；默认 `auto` = 只探 NVENC 再回落 libx264。表在 `docs/deploy.md` B-2。
- **模板里的工具函数必须放在脚本顶层**：`templates/index.html` 里 canvas 出片那一段是独立的 IIFE，看不到 `init()` 内部的东西（`evDur` 曾因此让每次出片都抛 `evDur is not defined`，而 GSAP 预览路径完全正常）。新增给两条路径共用的函数时，放顶层并留注释。**模板改完必须真跑一次出片验证**（预览看着正常 ≠ 出片正常）。
  ⚠️ **改了 `templates/index.html` 必须重建二进制**：它经 `include_str!` 进 `render/server.rs::INDEX_HTML`，运行中的 `yekitrim server` 端出的仍是旧页面。
- **字体库的渲染侧**：`/vt-fonts/:name` 是给**浏览器里那两个模板**（DOM/GSAP 预览 + 封面模板）用的 —— 底座那条腿**不走这个 server**，它的字体走 `dhampir.rs` 的 `--font-dir` / `--font-bold-file`。库里字体必须有**自己的一条路由**（只认库目录下的单段文件名）；**两条建 router 的路都要挂**（`start_file_server` + `start_with`），漏一条就是"预览有、出片没有"。
- **浏览器只认系统已装字体**，所以选了库内字体就**必须注入 `@font-face`**：`server::font_face_css(...)` 拼出来的片段进两个模板的 `<!-- __FONT_FACE__ -->` 占位（`index.html` 由 `build_html` 注入字幕字体、`cover.html` 由 `cover.rs` 的 `cover_font_css` 注入封面字体，**两条 replace 链都要改**）。**声明的字族名 = 配置里存的那串 id**，所以 `index.html` 里 `.cap{font-family:"<style.subtitle.font>",...}` **一行都不用改**；`cover.html` 的字体栈改成了 `__FONT_FAMILY__` 占位（空串时替换回系统栈）。库外/解析不到的值**什么都不注入** —— 注入了就是一条 404 的 @font-face，老工程的 `font = "LXGW WenKai"` 行为必须保持原样。
  ⚠️ **别改成"把字体拷进项目"**（贴纸那套 `copy_sticker_from_local` 是另一回事）。
  ⚠️ **解析必须走 `resolve_all`**（两种布局：v2 目录 `<slug>/font.ttf` + 老平铺文件）：曾用 `resolve_file_at`（只认平铺）⇒ v2 布局下**一条都不注入**（"选了库里的字体、出片没换"，预览另有 `FontFace` 看着还对）；钉子 `injects_font_face_for_dir_layout_too`，见 §4。
  ⚠️ **声明名保持 id、别改成内部族名**：模板与 `webui/src/polish/fonts.js` 都按 **id** 请求。
- **Takumi（`takumi.rs`）的字体口径 = 每支注册两次**：① 内嵌族名（原生）② **库 id 别名**（`FontResource::override_info(FontOverride{family_name})`）。Takumi **只按内嵌族名建族**，id ≠ 内嵌族名时会**静默回退**内置 Geist ⇒ 中文标题变豆腐块。别名由 `font_aliases(path)` 算：文件名是约定的 `font.<ext>` ⇒ **读同目录 `FONT.toml` 的 `id`**（没有元数据 / `id` 为空才回落"目录名"）；否则 = 文件名去扩展名。
  ⚠️ 目录名现在是 ASCII slug，**不能拿它当别名**（否则封面认不出用户选的那支，静默换字体）；字节用 `FontSource::from_shared` 共享，不翻倍内存。
  ⚠️ `cover_font_files()` **必须走 `fonts::scan_all`**：曾只手写"列顶层文件"⇒ v2 布局零字体，而零字体那条路是**明确报错**的。
  ⚠️ **顺序 = 默认字体第一、其余按 id 稳定序**（`cover_font_files_in`）：Takumi 对认不出的族名回退到"注册序第一支"，裸 `out.sort()` 会让「设为默认」对封面完全无效。真渲染验收见 `tests/takumi_font_id.rs` 与 `docs/plan/font-library.md` §13。
  ⚠️⚠️ **标题必须关掉 Takumi 的合成加粗**（`TAKUMI_TITLE_LAYER_CSS` 末尾 `#title{font-weight:400}`）：模板写 `900`，而 **Chrome 对库里那支单字面字体不合成加粗**，**Takumi 会**（`draw_text_embolden`）⇒ 填充外撑 `e/2`、盖掉描边内缘。
  ⚠️ **Takumi 认 `font-family`**（纠正旧结论）。⚠️ 标题里的 **emoji**（`title_lines` 联排、非特效层）：族名走库 id + 4 向 `drop-shadow` 环（**别加对角**）+ 必须 `display:inline-block`。证据链与 A-B 数字：`docs/plan/takumi-cover-capability.md` §15，细节见 §5。
- **模板里两条"四处/两份实现"的契约**（改 `templates/index.html` 或 `webui/src/polish/engine.js` 前必读）：
  - **覆盖层色档**：`overlay_type == "gradient"` 的有效色档规则见 `yekitrim-render-core/src/timeline.rs::overlay_stops`；四处实现为 `timeline.rs`、`templates/index.html`（**出片权威路径**）、`engine.js`、`PolishPreview.vue`。另两档 `fade_in` / `fade_out`（黑场淡入/淡出，2026-09 新增，内建 `intro-black` 随之退役）**不铺色档**：整窗就是过渡、**不叠 0.4s 包络** —— 分支必须排在包络之前，否则起点不透明度被乘成 0、「开头渐入没生效」。片头那份由 `process` **只在初次后处理**补 `fade_in` 事件。
    ⚠ 求值器**必须保证返回 ≥2 档且无空串**（`n == 1` 是 `0/0 = NaN` 抛 `IndexSizeError`、空串色档抛 `SyntaxError`，两者都会**打断整个 chunk 的渲染**）；⚠ 兜底出来的第二档必须是**同色零透明**（`overlay_fade_stop` / `overlayFadeStop`），**不能用 CSS 的 `transparent`** —— canvas 渐变在**非预乘**空间插值，写 `transparent` 会让浏览器预览与出片观感不一致。对拍：`node webui/scripts/check-overlay.mjs`。
  - **字幕限宽折行（A5）**：出片权威 `templates/index.html::wrapCaptionParts` 与编辑器 `webui/src/polish/engine.js::wrapCaptionParts` 必须逐字一致；上限 3 行（`MAX_LINES`）、超 3 行按 `MIN_RATIO=0.7` 缩字号；安全宽 = 画布宽 × 0.875（竖屏 945px）。
    ⚠️ 曾经 `append` 先读 `cur.length` 再判溢出，而溢出分支的 `pushLine()` 会把 `cur` 置空 ⇒ **任何一次折行都抛 `TypeError: can't access property "length", cur is null`**（"竖屏一播到长字幕就崩"，已修）。对拍：`node webui/scripts/check-caption-wrap.mjs`。
    ⚠ 两处 DOM/GSAP 预览的渐变 CSS **结尾那个 `)` 不能省** —— 少了它整条 `background` 非法被浏览器丢弃（"面板里设了色档、预览里什么都没有"）。径向语义 = **中心该色、边缘淡出**；不透明度一律用**数值判定**（`typeof ev.opacity === 'number'`），**别写 `ev.opacity || 1`**：0 是合法值，falsy 判定会把它变成全不透明。
- **前端留痕纪律（别再让信息"静默消失"）**：`api()`（`webui/src/api/client.js`）是唯一请求入口，失败统一 `console.warn` + 写日志面板（≥500 记 error，其余 warn），成功只写 `debug`；**读取类兜底用 `apiOr(path, fallback, label)`**，不要写裸 `api(...).catch(() => [])`（那会把"读取失败"伪装成"没有数据"）；非 API 的失败走 `stores/log.js::addClient(...)`，高频可预期失败记 `debug`；不做 toast→日志桥。包装器与 label 的清单见 `docs/polish-render.md` §9。
- ⚠️ **出片路径 = 底座整段**（逐帧接缝已删，见上）；`templates/index.html` 的 canvas `renderFrame` 是**惰性参照**（对拍黄金帧的冻结源、转译器注释的行号锚点），不再被任何生产路径执行 ——
  ⚠ 它曾硬编码 `1.8/1.5` 且完全不读 `[style.camera]`，别再把倍率写回去（倍率只有一张表 `CameraStyle::factor`）。

> 改动只允许落在本目录；跨模块改动先读对方卡，并跑两条门禁。

## 验证

```bash
cargo test -p yekitrim-polish --lib render
```

含 `render::server::tests`（`@font-face` 注入/不注入、模板占位符还在）。改模板或 ffmpeg 调用后必须再真跑一次出片：`node webui/scripts/check-overlay.mjs` 与 `node webui/scripts/check-caption-wrap.mjs` 是对拍门禁（预览看着正常 ≠ 出片正常）。
