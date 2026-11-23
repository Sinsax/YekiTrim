# `crates/yekitrim-polish/src/render` 卡外迁正文（出片腿 / 缓存实测 / 字体注入 / 模板契约）

> 配套模块卡：`crates/yekitrim-polish/src/render/AGENTS.md`（DSH 在触到本目录时自动注入）。
> 这份文档**不占指令预算**：卡里只留"改代码前必须知道、否则会写错"的规矩句与 ⚠️ 行；
> 删除清单、实测数字、历史成因与事故复盘全部搬到这里。
> 判断标准：**改代码前必须知道、否则会写错的留卡；想追根问底时才看的进这里。**
> 相邻文档：`docs/plan/render-dhampir-rules.md`（底座那条腿的七条实测规矩）、
> `docs/plan/takumi-cover-capability.md`（封面静态渲染能力面 / §15 emoji 证据链）、
> `docs/plan/font-library.md`（字体库 §13 Takumi 别名）、`docs/deploy.md` B-2（编码器）。

分节：

- §1 只剩一条出片腿：删除清单与兼容别名
- §2 分阶段打点的实测数字（clip-35）
- §3 磁盘缓存预计算的实测数字
- §4 字体注入的历史 bug（P1-e / v2 目录布局 / `resolve_all`）
- §5 Takumi 的字体口径：别名、注册序与合成加粗
- §6 覆盖层色档与字幕折行：两份实现的事故复盘
- §7 `runner.rs` 的删除
- §8 子进程链路的两起「任务永远运行中」事故
- §9 前端留痕纪律（日志面板与 `apiOr`）

---

## §1 只剩一条出片腿：删除清单与兼容别名

**2026-10-01 用户拍板**：出片只剩一条腿 —— `dhampir`（独立仓的 `dhampir.exe render`，
见 `render/dhampir.rs`、`RenderBackendKind`）。**本仓那条 headless Chrome 逐帧腿已整条删除**：

- `render/chrome.rs`；
- `RenderBackendKind::Chrome`；
- `headless_chrome` 依赖；
- 封面那条 Chrome 截图通道（`cover.rs` 里 `chrome` 分支）；
- `/api/health` 的 `chrome` 能力位；
- WebUI 的出片通道下拉与封面通道弹窗。

**全部一起没了。**`RenderBackendKind::parse("chrome")` 现在返回 `None`，`cmd_render` 会**明确报错**
（**不静默改走底座**：那会让人以为"我在用 chrome 出片"，而两版成片的差别只有对着看才发现）。
封面那条对 `chrome` 给的是**单独的**"这条腿已删除"错误，不是折进"认不出"。

**兼容别名**：`wgpu` / `native` 仍只作 `dhampir` 的**别名**（老配置/老脚本的兼容映射）。

**更早的删除**：原来的 `native/`（`WgpuBackend`）与 `wgpu-render` feature 已整条删除（GPU 那侧交给底座）；
`backend.rs` 的逐帧接缝（`FrameRenderer` trait + `create_backend` + `render_chunk_sync`）**已整段删除**
（2026-10-02，S4 阻断 B 拍 **B2** 后的收口；真要接第二个逐帧后端从 git 历史挖形状重新评估）。

**现状**：底座是"整段一起出"，走 `cmd_render` 里的**另一条分支**，产物落在同一个 `_temp_video.mp4` 上，
后面的音轨链路一行不改（**画面归底座、声音归本仓** `--no-audio`）。

## §2 分阶段打点的实测数字（clip-35）

`cmd_render` 每段一条 `⏱ 阶段 {:.1}s`，`render_chunk_sync` 把每块的**抽帧 / 渲染 / 编码**分开报
（`chunk#N: done N frames（抽帧 …s / 渲染 …s / 编码 …s）`）。出片慢先看这几条，别再猜。

实测 **clip-35**（262s 片段，debug 构建）总 **294s**：

| 阶段 | 耗时 |
|---|---|
| 起服务 | 16.1s |
| 贴纸 GIF | 40.8s |
| 分块 | 210.7s（每块 抽帧 ~30s + 渲染 145~171s + 编码 8s） |
| 音效混音 | 9.3s |
| 合流 | 2.7s |
| 屏蔽重编码 | 9.2s |

## §3 磁盘缓存预计算的实测数字

两份缓存都在 `<工程>/data/`，按「文件名 + mtime + 大小」失效：

- **`loudness.json`**（`render::measure_loudness_curve_cached`，出片页与混音共用一份）——
  预览服务的 HTML 是**每个请求**重建的，不缓存等于每刷一次页面烧一遍整段 ebur128；
  实测「起服务」**16.1s → 0.1s**。
  ⚠️ 量它那条 ebur128 **必须带 `-vn`**（只要音频：同一片段 **5.6s → 0.4s**，曲线逐点一致）——
  剪切收尾会给每个新片段预热这份缓存（`yekitrim-service::core_pipeline::warm_loudness_cache`，只补缺失），
  热路径上一次剪切跑 N 遍。
- **`sticker_frames.json`**（`prepare_sticker_data` 的解码结果）—— 6 个贴纸 **40.4s → 命中 1.67s**
  （产物 **95MB**，断言过冷/热输出一致）。贴纸多一个就多一份解码，别把缓存去掉。

**诊断**：`VT_POLISH_DIR=<项目> cargo test -p yekitrim-polish --lib diagnose_sticker_cache -- --ignored --nocapture`。

**贴纸图不止 GIF**（`is_image_path` 一份判据管 `gif/png/jpg/jpeg/webp`）：写死 `<name>.gif` 会让
**PNG 贴纸在成片里一张都没有**（预览照常显示，静默消失）。现在找不到图/解不出帧都打 warn。
钉子：`static_image_sticker_still_gets_frames`。

## §4 字体注入的历史 bug（P1-e / v2 目录布局 / `resolve_all`）

**形状**：字体解析曾用 `resolve_file_at`（只认平铺布局）⇒ v2 目录布局（`<slug>/font.ttf` + `FONT.toml`）下
**一条 `@font-face` 都不注入**，症状是"选了库里的字体、出片没换"（预览另有 `FontFace`，看着还对）。
P1-e 修的就是它，钉子 `injects_font_face_for_dir_layout_too`。

**结论（现在卡里也留着）**：解析必须走 `resolve_all`（两种布局都认）；声明名保持 **id**、别改成内部族名
（模板与 `webui/src/polish/fonts.js` 都按 id 请求）；库外/解析不到的值**什么都不注入** ——
注入了就是一条 404 的 @font-face，老工程的 `font = "LXGW WenKai"` 行为必须保持原样。

**两个 router 都要挂**（`start_file_server`（预览 + 封面）+ `start_with` 预览），漏一条就是
"预览有、出片没有"。**两条 replace 链都要改**：`index.html` 由 `build_html` 注入字幕字体、
`cover.html` 由 `cover.rs` 的 `cover_font_css` 注入封面字体。

## §5 Takumi 的字体口径：别名、注册序与合成加粗

**每支注册两次**：① 内嵌族名（原生）② **库 id 别名**（`FontResource::override_info(FontOverride{family_name})`）。
Takumi **只按内嵌族名建族**，id ≠ 内嵌族名时（id 是文件名、内嵌族名带空格那种）它会**静默回退**内置 Geist
⇒ 中文标题变豆腐块。

**别名怎么算**（`font_aliases(path)`）：文件名是约定的 `font.<ext>` ⇒ **读同目录 `FONT.toml` 的 `id`**
（没有元数据 / `id` 为空才回落"目录名"—— 老库行为不变）；否则 = 文件名去扩展名。
目录名现在是 ASCII slug，**不能拿它当别名**（否则封面认不出用户选的那支，静默换字体）；
字节用 `FontSource::from_shared` 共享，不翻倍内存。

**`cover_font_files()` 必须走 `fonts::scan_all`**：曾只手写"列顶层文件"⇒ v2 布局零字体，
而零字体那条路是**明确报错**的（"静态封面直接出不来"）。
**顺序 = 默认字体第一、其余按 id 稳定序**（`cover_font_files_in`）：Takumi 对认不出的族名回退到
"注册序第一支"，裸 `out.sort()` 的字典序运气会让「设为默认」对封面完全无效。
真渲染验收与变异自证见 `tests/takumi_font_id.rs` 与 `docs/plan/font-library.md` §13。

**合成加粗**（`TAKUMI_TITLE_LAYER_CSS` 末尾 `#title{font-weight:400}`）**必须关**：模板写 `900`，
而 **Chrome 对库里那支单字面字体不合成加粗**（受控实测 400/900 **逐像素相同**），**Takumi 会**
（`draw_text_embolden`）⇒ 填充外撑 `e/2`、盖掉描边内缘（**20px 描边实测 10px → 7px**）。

**另一条旧结论已被纠正**：**Takumi 认 `font-family`**（不是"只认内嵌族名、不吃 CSS 声明"）。
标题里的 **emoji**（`title_lines` 联排、非特效层）：族名走库 id + 4 向 `drop-shadow` 环
（**别加对角**：链式累积会翻倍）+ 必须 `display:inline-block`。
📖 证据链 / A-B 数字 / 复跑命令：**`docs/plan/takumi-cover-capability.md` §15**。

## §6 覆盖层色档与字幕折行：两份实现的事故复盘

**覆盖层色档**：`overlay_type == "gradient"` 的有效色档规则见 `yekitrim-render-core/src/timeline.rs::overlay_stops`；
四处实现为 `timeline.rs`、`templates/index.html`（**出片权威路径**）、`engine.js`、`PolishPreview.vue`。
另两档 `fade_in` / `fade_out`（黑场淡入/淡出，2026-09 新增，内建 `intro-black` 随之退役）**不铺色档**：
整窗就是过渡（`opacity × (1 − dt/dur)`，`fade_out` 镜像）、**不叠 0.4s 包络** —— 分支必须排在包络之前，
否则起点不透明度被乘成 0、「开头渐入没生效」。片头那份由 `process` **只在初次后处理**补 `fade_in` 事件。

**两条硬判据的由来**：

- 求值器**必须保证返回 ≥2 档且无空串**：`n == 1` 是 `0/0 = NaN` 抛 `IndexSizeError`、空串色档抛
  `SyntaxError`，两者都会**打断整个 chunk 的渲染**；
- 兜底出来的第二档必须是**同色零透明**（`overlay_fade_stop` / `overlayFadeStop`），**不能用 CSS 的
  `transparent`** —— canvas 渐变在**非预乘**空间插值，写 `transparent` 会让浏览器预览与出片观感不一致。

**渐变 CSS 结尾 `)` 的事故**：两处 DOM/GSAP 预览的渐变 CSS **结尾那个 `)` 不能省** —— 少了它整条
`background` 非法被浏览器丢弃，表现正是"面板里设了色档、预览里什么都没有"（`engine.js::gradientCss` 一份，
模板里 `overlayGradientCss` 一份）。径向语义 = **中心该色、边缘淡出**（canvas 是
`createRadialGradient(960,540,0,…,960)`）；不透明度一律用**数值判定**（`typeof ev.opacity === 'number'`），
**别写 `ev.opacity || 1`**：0 是合法值，falsy 判定会把它变成全不透明。

**字幕限宽折行（A5）**：出片权威 `templates/index.html::wrapCaptionParts` 与编辑器
`webui/src/polish/engine.js::wrapCaptionParts` 必须逐字一致；上限 3 行（`MAX_LINES`）、
超 3 行按 `MIN_RATIO=0.7` 缩字号；安全宽 = 画布宽 × 0.875（竖屏 945px）。

**事故**：曾经 `append` 先读 `cur.length` 再判溢出，而溢出分支的 `pushLine()` 会把 `cur` 置空 ⇒
**任何一次折行都抛 `TypeError: can't access property "length", cur is null`**
（"竖屏一播到长字幕就崩"，已修）。对拍：`node webui/scripts/check-caption-wrap.mjs`。

## §7 `runner.rs` 的删除

`runner.rs` 已删除（2026-09）：它自带一份 `ResolvedPaths` + `RenderRunner`，全仓库零消费者
（真正的在 `pipeline.rs`，经 `render/mod.rs` 的 `pub use pipeline::*` 出去），整个模块只靠一个
`#[allow(dead_code)]` 撑着。要加回"提帧 runner"先确认有第二条链路需要它。

## §8 子进程链路的两起「任务永远运行中」事故

**事故一（管道不 EOF）**：`api::run_polish_child`（`yekitrim-server`）把子进程 stdout/stderr 重定向到
`<data_dir>/logs/render-<job_id>.log`，再用 `follow_render_output` 边跑边读。改回 `Command::output()`/管道时，
ffmpeg 与底座拉起的**孙进程**会继承 stdout 句柄 —— 只要还有一个活着，管道就永远不 EOF，`output()` 一直等下去。
**实测**：子进程早已带着 `Render error: evDur is not defined` 退出，任务却卡了半小时、错误全被闷在缓冲区里。

**事故二（孤儿孙进程）**：每次渲染往 `<data_dir>/render-procs/<job_id>.pid` 登记 pid，
`server::run` 启动时 `render_proc::sweep_orphans` 结束上一次遗留的渲染（**先核对命令行**再杀，
pid 会被系统回收）。服务器重启带不走子进程 —— **实测两个渲染各留 3 个渲染孙进程空转两小时**，
把后续渲染拖到根本跑不完。

**卡死守卫**：父进程按"子进程 60s 无输出 → warn"看着底座；底座进度在 `_temp_dhampir_progress.ndjson`。
别再退回**无超时轮询**（"长时间无反应 + 日志空白"的来源）。

## §9 前端留痕纪律（日志面板与 `apiOr`）

- `api()`（`webui/src/api/client.js`）是唯一请求入口，失败统一 `console.warn` + 写日志面板
  （≥500 记 error，其余 warn），成功只写 `debug`。可选 `{ label: '读取切片列表' }` 给失败配上业务上下文。
- **读取类兜底用 `apiOr(path, fallback, label)`**，不要写裸 `api(...).catch(() => [])`：
  后者把"读取失败"伪装成"没有数据"，页面与日志里都看不出异常。包装器
  （`projectData.*` / `fetchSubtitles` / `fetchProject` / `fetchStickerSources`）都收一个 `label` 参数，
  视图侧传中文说明。
- 非 API 的失败（裸 fetch、SSE、解析）用 `stores/log.js` 的 `addClient(level, text, source)`；
  高频可预期失败（弹幕文件缺失、字体回退、任务轮询重试）记 `debug`，不要刷 warn。
- 不做 toast→日志桥（err toast 本来就可见，桥接只会和 `api()` 的记录重复）。
