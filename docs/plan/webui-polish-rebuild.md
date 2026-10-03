# 重做 webui 精修页面（process 隐式化 + 以 preview 为基准重写）

> 日期：2026-09-05 · 状态：已批准，实施中
> 进度：后端三项改动已完成并通过 `cargo check`/`clippy`（无新警告）；前端 `webui/src/polish/api.js` 已创建，engine.js / audio.js / 三个页面重写进行中。

## 设计原则
- **process 隐式**：服务端在 LLM job 内自动链式执行 process；TimelineEditor 保存后前端自动补一个 process job；`cmd_render`/预览本就自动 process。前端不再出现任何 process 按钮。
- **显式动作只有两个**：「LLM 生成」和「渲染导出」（外加未初始化时的「初始化」）。
- **视觉与逻辑基准**：以 `crates/yekitrim-polish/templates/index.html`（GSAP 时间线 preview）为基准，忠实移植到 Vue 自绘预览（用户已确认保留 Vue 自绘方案，不用 iframe 内嵌）；保留九宫格布局、字幕编辑、封面定制三个编辑面板；`/polish` 列表页精简为入口+状态。
- **消灭石山**：特效数学收敛到纯函数模块，API/job 逻辑收敛到一个共享模块，两页面不再复制代码。

## 一、后端改动（crates/yekitrim-cli/src/server/api.rs，yekitrim-polish 小改）

1. **隐式 process 服务端链式**（`api.rs` polish_spawn）：`"llm"` 分支改为同一个 job 内先 `cmd_llm` 再 `cmd_process`，用 `jobs.log()` 输出阶段消息（"LLM 生成中…" / "自动执行 process 后处理…"），任一步失败即 fail。前端 `setTimeout(1500)` 猜测链彻底删除。
2. **GET /api/polish/project 增强**：`?preview=1` 时响应附加预览所需计算字段——`_assets.sticker_map`（复用 `render/server.rs` 的 `build_sticker_map`）、`vertical`/`vertical_position`（`get_video_size` 竖屏检测）、`loudness_curve`（`measure_loudness_curve`，缓存到 `data/loudness.json`，按源视频文件名失效）。一次请求满足 PolishPreview 全部数据需求。
3. **渲染进度桥接**：`cmd_render`（`render/pipeline.rs`）加可选进度回调参数 `Option<&RenderProgress>`（`(阶段名, current, total)`；阶段：process/prepare/chunks/concat/audio/sfx/mux/cover），api.rs 的 render job 闭包经 mpsc channel 桥接到 JobManager（chunks → progress 事件，其余 → log）；`main.rs` CLI 调用传 `None`。解决渲染几分钟零反馈问题。
4. polish job 的 message 附上 dir（目录名），便于跨页识别。其余端点（/api/polish/preview、process、cover、cover-* 等）一律不动，CLI/独立预览继续可用。

## 二、前端新增 `webui/src/polish/` 模块

- **`api.js`**：polish 全部 API 调用（project/subtitles/init/llm/render/version…）+ `watchJob(id)`（EventSource 订阅 `/api/jobs/:id/events`，异常回退轮询 `/api/jobs/:id`；订阅时先拉一次状态避免错过已结束任务）+ 九宫格常量（与 Rust `grid_rect` 命名对齐：`bot-left`/`bot-right`）。唯一数据出口，两页面共用。
- **`engine.js`**：从 index.html **Canvas 渲染器部分**（即真实出片路径 renderFrame）忠实移植的纯函数特效引擎 `(t, idx) → 覆盖层状态`：
  - GSAP 兼容 easing 表（pow2Out/pow2In/sineOut/backOut(3) 等）；
  - TRANSIENT_CAP 时长钳制表（与 `process/fx.rs` 一致）；
  - 运镜 `cameraAt`（zoom 近/中 = `style.camera.zoom_close/zoom_mid` 覆盖，默认 1.8/1.5，origin 取 `liver.origin_grid` 九宫格中心）；
  - 全部瞬态 FX：shake/flash/blur/zoom_bounce/vignette/hue_shift/stutter/split/color_shift/noise/pulse；
  - overlay（color/gradient；love 与出片路径一致被忽略）；
  - 贴纸四阶段（popIn back.out(3) / floatUp / hold / fadeOut，280×280，竖屏时 x 按 vertical_position 覆盖/左右交替）；
  - 字幕入场出场（fade 0.35/0.18 + yOff）+ 事件联动脉冲（scale 1.25 / y −20）；
  - 弹幕滚动（`data/danmaku.json` 的 `y`/`travel_duration`，canvas measureText 预测量宽度，y 映射上半屏 540）；
  - `buildIndex(config, subtitles, danmaku, duration)` 一次性预处理（含 segments 截断、音效事件表、playtest 标签表）。
- **`audio.js`**：SFX 播放（移植 sfxPlay/curveAdjust/scheduleSound：Audio 元素 + 30ms fade-in、重叠 clone、`cfg.sfx.volume` × 响度曲线 target−8dB、seek 重触发窗口 0.12s、主音量 localStorage 记忆）。补齐当前 Vue 预览完全没声音的问题。

## 三、前端重写

- **`PolishPreview.vue`**（500 行 → 目标约 250 行）：瘦播放器——props 改为父级下发数据（`config/subtitles/danmaku/metaDuration`，父组件只做一次请求）；舞台 16:9 + 1920×1080 虚拟 overlay 缩放；图层 z 序（bg 竖屏模糊背景 < corner-glow < flash < overlay[贴纸/弹幕/字幕] < intro-black）；rAF 读 `video.currentTime` 作为主时钟驱动 engine + 音效；播放/seek/音量/全屏控制 + 事件 ticker（playtest）。修复：`stickerStateMap` 在 computed 里改非响应式 Map 的副作用（错误回退改用事件回调 + Map 替换）、九宫格命名 bug、贴纸资源改用 sticker_map（不再盲猜扩展名）、显式覆盖全局 `video{max-height:170px}`。
- **`PolishView.vue`**：重组为——无 `?dir` 时显示可精修片段选择器；未初始化卡 / 左列（新 PolishPreview + TimelineEditor）/ 右栏（动作卡：LLM 生成 + 渲染导出，内联 job 进度条与日志；状态卡：processed/已渲染/封面 chips + 原版/消音版本切换；九宫格布局；字幕编辑；产物卡：cover.jpg + 渲染成片 `.video-lg` 预览/下载）。CoverDesigner 保留底部。`window.prompt`/`confirm` 换成应用内 modal（`.modal-backdrop/.modal`），渲染路径留空默认用 polish.toml 的 `project.output`。TimelineEditor 保存 segments 后自动隐式触发 process job（完成后刷新预览）。job 成功后重载 project + 预览。
- **`PolishProjectsView.vue`**：精简为入口+状态——卡片（标题/状态 chips/版本）+「打开精修」（未初始化则显示「初始化」）；openEditor 改为通过 `/api/project/list` + 路径启发式（`/output/clips/` 前缀 → 项目目录 `yekitrim.toml`）定位归属项目，**不再** POST `/api/project/switch` 改全局状态；订阅 `/api/stream`，收到 completed/failed 事件自动刷新列表。
- **`ProjectLayout.vue`**：项目 tab 栏加「精修」入口（router.js 路由已存在无需改动）。

## 四、删除/清理
- PolishPreview 中 500 行特效复刻逻辑（由 engine.js + audio.js 取代）。
- 两视图重复的 runAction/render、llm→process 的 setTimeout hack、process 按钮。
- 九宫格命名 bug（`bottom-left` → `bot-left`）、产物视频被全局 CSS 压到 170px 的问题（用 `.video-lg`）。

## 五、保留不动
`TimelineEditor.vue`、`CoverDesigner.vue`、后端其余全部端点、webui 其他页面、整体架构（Vue3 + 现有 axum server）。

## 验证
1. `cargo build` 0 warnings（含 yekitrim-cli、yekitrim-polish）——已通过 `cargo check` + `clippy`（无新警告）。
2. `cd webui && npm run build` 通过。
3. `cargo run -- server` 冒烟：/polish 列表加载 → 编辑器加载、预览播放（字幕高亮/贴纸/运镜/弹幕/音效）→ LLM job 进度与自动 process → 渲染进度 → 产物显示。

## 关键参考文件
| 文件 | 用途 |
|------|------|
| `crates/yekitrim-polish/templates/index.html` | 特效唯一基准（1004-1695 行 Canvas 纯数学版 = 出片行为；344-1002 行 GSAP DOM 版） |
| `crates/yekitrim-polish/src/render/pipeline.rs` | cmd_render 阶段顺序 + 响度混音逻辑 |
| `crates/yekitrim-polish/src/render/server.rs` | build_sticker_map / 竖屏检测 / loudness 注入 |
| `crates/yekitrim-cli/src/server/api.rs` | polish 端点 + polish_spawn + JobManager 桥接 |
| `crates/yekitrim-cli/src/server/jobs.rs` | JobEvent 种类 / progress 事件 |
| `crates/yekitrim-polish/src/project/config.rs:283` | 九宫格权威命名 grid_rect |
