# webui/src/components — 精修组件层：时间轴编辑器 / 预览 / 封面设计器 / 运镜 UI
> 范围：`webui/src/components`/** ｜ 上游：`webui/src/views/*`｜ 验证：`npm --prefix webui run build`
## 职责

精修页与设置页的全部子组件。壳/侧栏/路由/缓存纪律见 `webui/src/AGENTS.md`，页面纪律见
`webui/src/views/AGENTS.md`，模板同口径算法见 `webui/src/polish/AGENTS.md`。

> 历史与实测数据见 **`docs/webui-components.md`**（①~⑧：布局几何 / 事故复盘 / 逐项枚举清单 / 推导过程都在那份里）。

## 入口

- `TimelineEditor.vue`（7 条泳道，`kind` = `seg|event|sub|mute`）：props/emits 见文件头。`saved` = **这批改动已落盘**（Dhampir 腿据此重载），与 `preview`（**不落盘**）**别互相替代**；`done` 在它之后。
- `PolishPreview.vue`（自绘 DOM/GSAP 层，**只有这一层**）：props/emits 见文件头。
- `CoverDesigner.vue`：props `dir`(必填)/`pid`，emit `changed`。
- 其余组件逐个的职责见「边界 · 其他组件」。

## 边界

**简化基础件**
- 页面级空态/下一步/首启检查：`AppEmptyState.vue` / `NextStepBar.vue` / `SetupChecklist.vue`。
- `SetupChecklist` = 首页环境与模型的**唯一**状态卡（v8 并入原「运行状况」卡）：「运行环境」行 = ffmpeg/ffprobe/**出片底座** chips（`/api/health`，缺项指路 → `dhampir/bin/`）+「字幕模型」行内嵌 `ModelList`（下载仅 admin）；三源齐刷，`doctorError` 照实报（读失败 ≠ 缺失）。详见 §⑧.2。
  ⚠️ **待处理项必须行里逐项可见** —— 旧运行状况卡"报 1 项待处理、展开却全绿"的病根就是底座缺失没有展示位（用户实测）；页头汇总 chip 随之删除（同一件事不说三遍）。口径见 `docs/web-ui-design.md` 附录 E。
- LLM 组件（2026-11-27 起 **左右列表**：左 `LlmProviderList` 接口清单 / 右 = 设置卡 + 模型清单）：
  `LlmEndpointCard`（协议/地址/密钥/默认模型/获取模型/测试/设为当前/删除；**右栏顶部常驻卡**，不是弹窗）·
  `LlmModelList`（**只有 id + 显示名**；「获取模型」的候选在表格上方，id 是 `<datalist>` 输入框 ⇒ 能选也能手打）·
  `LlmLinkTable`（**5 列**：用哪支/温度/输出上限/推理；数字 = 滑块 + 数字框；无测试列、无状态列）·
  `LlmModelCombobox`（连接卡的默认模型用）。
  ⚠️ **别把探针 caps 写进 `catalog`**（形状不同，文件合法但反序列化才炸 ⇒ 整份 llm.toml 被拒收）；判据 ㊷。
  ⚠️ **已删、别捡回来**：模型行的能力手填（输入模态/温度/输出字段/推理写法/档位表）、每模型 `extra_params`
  （连 `extra.js` 一起删）、链路页「默认参数」块与 `LlmParamFields.vue`。参数全归链路。
  ⚠️ **推理没有「跟随」档**（缺省 `off`）；链路表「用哪支」仍是一个分组下拉（值 = `接口|模型`）。
- 新控件带 `data-cap`；高级项用 `useDisclosure`/`<details>` 默认折叠，隐藏 ≠ 删除。

### 共享原语（`components/ui/`，v6）

**无状态**：只吃 props、发 events，不 import stores、不发请求；新页面**优先用它们**，别再造第 N 套（九个原语清单见 §⑧.1）。

⚠️ `AppEmptyState` 保留（9 个页面在用），新代码用 `UiEmpty`；两者接口同名，可逐页替换。

### 时间轴编辑器
- **布局必须保持紧凑**（**独占一整行**）：泳道 **26px ×7 + 标尺 20px = 202px**（叠 16px 热度曲线时 218px）—— 用户明确**不要**新增泳道/改宽、**不要**电平参考线、**不要**把字幕块改半透明。几何见 ①.8。
- ⚠️ **遮挡**：曲线画在**字幕泳道**上、而那条泳道的字幕块不透明 —— 波形 `.wave-layer` 必须 **`z-index:4`**、`pointer-events:none` 不能去掉（①.2）。
- ⚠️ **刻度**：窗口 **[-40, -8]**（`WAVE_DB_TOP`/`WAVE_DB_BOTTOM`），别改回 [-60,0]；判据 `node webui/scripts/check-heat-curve.mjs`。
- 事件字段是 `time` + `end`（**没有 `duration`**）；老工程由 `legacy.rs::upgrade_event_times` 反序列化前换算；求值器统一走 `evDur()`/`evDuration()`。
- **控件统一**：**体拖移动 / 左缘改开始 / 右缘改结束 / 双击删除**；换轨道就删掉重加。**属性面板 = 标配块 + 类型专属块**；**「保存修改」按钮已删**。
- ⚠ **字幕/屏蔽轨道必须保持按时间有序**：`splitSelected` 的后半段要 `splice(i+1, 0, …)` 插在原块**后面**，**不能 `push` 到末尾**（会让落盘的 `clip.srt` 乱序）。
- ⚠️ **别再改回"落盘后 1.2s 自动跑"**：**落盘 ≠ 编辑完成**，后处理只在 `done` 之后跑（`done` 等落盘成功）；`input` 每敲一键**不算**，切片段带**旧 dir**。
- **手工放的事件（`manual`）后处理一律不改写**：**吸附与 `cap_events` 抽稀全跳过**。
- **属性面板用 `<Teleport :to="teleportTo">` 投到 `PolishView` 预览卡里的 `#tl-props-host`**。
  ⚠️ **必须门控 Teleport**：目标是**挂载时**解析的，那时子树还没进 document ⇒ `:to="propsTarget"` 会解析到 null、**面板整块不渲染**（①.5）。
- ⚠️ **别用裸 `<template>` 包裹要无条件渲染的内容**；**前端布局类改动必须做几何断言**（`getBoundingClientRect()` 宽高 > 0）。
- ⚠️ 写 headless harness 时：**`app.mount(host)` 会清空 `host`**、选中块只能靠 **pointerdown**（`block.click()` 不选中）—— 见 ①.5。
- **播放头跟随 = 只在播放时跟、越界才滚**：`watch(currentTime)` 第一句 `if (!props.playing) return`。**别退回"每帧钉在 30% 处"**。
- **分段为空时时间轴照常渲染**：「去静音」**常显**，**别加回 `v-if="segments.length"` 门控**。
- ⚠ **「读取失败」不许伪装成正常状态，也不许在基线不可信时写盘**：`flushSave` 开头 `if (loadError.value) return`；横幅 +「重试」在 `tl-head` 下（**不能放进 Teleport 面板**）；`/api/polish/subtitles` 的 **404 是空态**；切 `props.dir` 时 `commitEdit` 之后必须 `resetTimelineState()` 再 `loadData()`。

### 精修预览（`PolishPreview.vue`）
- ⚠️ **跳手动屏蔽区间 = A/B 预滚（首选）+ 定格遮罩（兜底），不许退回"直接 seek"**：`videoRef` 是 **computed**（Dhampir 腿靠 `engine.bindSource` 换绑 + 源没就绪不画），改回模板 ref / 删备用源 = A/B **静默失效**（②.1/②.2）。
- 自绘层与出片模板 `crates/yekitrim-polish/templates/index.html` **必须同口径**（纯函数从 `polish/engine.js` 来）。
  ⚠️ **只有自绘这一层**：两条真渲染通道（wasm 实时 + `render-frame` 单帧回退）随「作废 wgpu」删掉了 —— **别再接回来**，要真渲染走 Dhampir 底座。
- 媒体地址一律走 `utils.js::clipMediaUrl(pid, dir, 相对路径)`，别自己拼 `/api/media/...`。
- ⚠️ **预览视频源优先 `_preview_video`**（`_proxy_720.mp4`）；拿不到回落 `source.video`。
- 音量/静音 = **总音量，必须落到所有会出声的 `<video>` 元素**；读回 `isFinite` + clamp，**不能写 `parseFloat(...) || 1`**；**别写死 `66vh`**。
  ⚠️ **Dhampir 模式下"台上那块"不是唯一在响的元素**：`applyAudio()` 必须覆盖 `dhampirSourceVideos` ∪ `dhampirTwins`（**不能只遍历 `dhampirVideos`** ⇒ 旧元素仍在播 = **声音翻倍**）；备用那批一直 `muted`；**`tick()` 的 Dhampir 分支不许漏 `player.sync(...)`**。详见 ②.6；判据 `node scripts/check-preview-audio.mjs`。
- ⚠️ **字体栈顺序 / 字重必须与出片侧同源**（`.cap` ↔ 服务端 `fallback_stack`：透传 `fetchFonts().fallback_stack`，**不许前端写死字体名**，否则「设为默认」对预览永远无效）：漂了 = 预览雅黑、成片 Noto。
- ⚠ 动图（GIF/动画 WebP）由底座自己解码（v6）：四条硬规矩与调用顺序陷阱见 **②.4**。
- ⚠ **预览看着正常 ≠ 出片正常**：出片的事实来源是 `templates/index.html` 的 canvas 路径（`yekitrim-render-core/src/timeline.rs` 是平行实现）。"改完没反应"先怀疑没重建/没重启。
- ⚠️ **Dhampir 加载期间不许播**（门帘 + 播放键置灰 + `togglePlay` 三道闸；热重载不挂）：见 §2.5。

### 封面设计器（`CoverDesigner.vue`）
- 细则（图层栈/镜像清单/四处同步/写入门闸/安全区）见 **§③**；别只改前端镜像（`cover.rs` 与 `templates/cover.html` 必须同步）。`configLoaded` 是写入门闸（读失败出错误条 + 重试，别拿默认值写回）。

### 运镜（camera）
- **倍率只有一张表**：`CameraStyle::factor`，四处都走它。⚠ 模板 canvas 路径曾硬编码 `1.8/1.5`、完全不读 `[style.camera]`，而它才是出片路径 —— 别写回去。
- **三档倍率当前值：中 1.3 / 近 1.5 / 画中画 0.55**；三处兜底默认值（`default_zoom_*()`、模板与 `engine.js::camFactor`）一起改；放大档不会"推出框露背景"，露底只在画中画 `zoom_wide < 1`（推导见 §④）。
- ⚠ **运镜门 = 运镜轨的眼睛**（`[stage].lanes.camera`，取代 `portrait_no_camera` 硬门）：模板 `getActiveCamera` 与 `engine.js cameraAt` 整条返回 `zoom=1 / handheld=0`。六轨眼睛见 ①.7。
  **画中画到点收工**：`close_pip_shots` 补一条**回镜头**真事件，档位/不动点取**时间上**前一条非画中画运镜的**落点**；下一条在 `MIN_RETURN_SECS`(0.5s) 内来则不补。幂等。⚠️ **前后镜头按时间算、不按文件顺序**。
- `process` 给每个 ≥1s 且没有运镜的段补 `zoom="中", anchor="liver"`；改默认档位/锚点就在这一处。
- ⚠️ **位置面板是「起点 / 终点」+「运镜过渡」**：两个下拉是**同一件事的两个端点**（`anchor`/`anchor_to`），文案**不许**退回「聚焦 / 移到」；**终点默认「不移动（同起点）」**（相同则清 `to_origin`、整块静止）；「**运镜过渡**」= 接缝（`cut`+`speed`），管「上一块落点 → 本块起点」。
- ⚠️ **位置是连续状态**（与 `zoom`/`handheld`/`tilt` 同形；四份实现：模板 / `engine.js` / 转译器 / Rust）：以前每块**绝对赋值** ⇒ 块缝瞬跳（实测 928px/帧）；判据 `webui/scripts/check-handheld.mjs` 两段。

### 贴纸落点（Step 2b）与区域编辑
- `align_stickers_to_subtitles` 起点吸到**最近那条字幕**的开始、终点吸到它的结束（> `STICKER_SNAP_MAX_SECS`(2.0s) 的不动）；**必须排在 2b 落点之前**。
  ⚠️ **手工放的事件不受后处理改写**（用户报过「调整贴纸的结束时间没有成功」）：`manual` 标记下吸附与抽稀全跳过。
- ⚠ **人物框同时喂 Step 2b 与 2d；弹幕区域既是贴纸障碍，也是 `anchor="danmaku"` 没真实弹幕时的兜底** —— 别退回"弹幕区域仅作参考"那套旧说法。
- 读取优先级：`free_rect()` → `region_rects()`/`region_origin()` 一律**自由选框 > grid > `origin_grid`/`origin`**；**固定落点 UI 是"草稿态"**：按「保存落点」才落盘，成功才退出编辑态。

### 其他组件
- `ProjectClipsSection`（切片页卡片墙）：⚠️ **删除不是任务（不发 job），刷新只能由前端自己驱动**。`silent` 必须真的静默；**「剪切」按钮不禁用，忙时只改文案**。
  ⚠️ 「精修」按钮（`goPolish`）：**只有老片段**才补 init，且**必须 `await waitJobDone(id)` 再跳**（否则撞 `polish.toml not found`）；`polishOpening` 防连点，`jobClosers` 在 `onUnmounted` 里关。
- `MediaLibrary`（首页素材库，**文件夹管理器**布局；`FontLibrary`/`StickerPacks` 已并入它）：左类型导航 / 右网格，**一个网格画三种** —— ⚠️ 别再退回"三块各自实现"。规则与坑见 ⑥。
- `SliderField`：⚠ **设置页的数值项走 `components/SliderField.vue`**（滑块 + 行尾可直填数字框）。
  ⚠ **滑块宽度 = 所在格子的 1/3**（`input[type=range]` 的 `width:100%` 会被 flex-basis 盖掉，**别改回 100%**）。
- `JobList`/`jobs/JobDrawer`（任务列表与按钮门控 —— "界面哑火"的来源）：
  - ⚠ **请求必须有超时**：`jobsApi` 自己传 **10s**（`api()` 默认 10 min）—— 卡住的 `fetch` 永不 settle，会连同按作用域门控的按钮一起冻住。新增轮询接口同样给短超时。
  - ⚠ **读失败 ≠ 没有任务**：`useJobs.load()` 失败时**保留上一份快照**并置 `lastError`，`JobList` 分「读取中 / 读取失败（带重试）/ 真的没有任务」**三态**显示。
  - ⚠ **门控要能被陈旧快照兜住**：`useJobs.stale`（超 `SNAPSHOT_TTL_MS`=15s）为真时必须把 `busyOn` 降级成 `null` 放行。**别把门控当"不许点"**。
  - ⚠ **任务结束的刷新必须 `silent`**：非静默 `load()` 一置 `loading`，`v-if="loading"` 就卸载卡片墙 → 工具条按钮全灰。这类刷新还要**单飞 + 尾随重刷**。
  - ⚠ **按钮按作用域门控**：`api/jobs.js` 的 `clipKey/outKey/idxKey/MODELS_KEY/STICKERS_KEY` 必须与 `crates/yekitrim-server/src/scope_key.rs` 一致，`busyOn(jobs, keys)` 求交集；同步保存类操作不受门控。
  - ⚠️ **前端订阅**：`watchJob()` 返回 **`{ close() }`**，关订阅要 `w.close()` —— 当函数调会抛错、**新任务永远订阅不上**。
  - 任务行「卡住」提示与「被推迟的后处理」（`pendingProcess`）的口径见 §⑧.6。
- `ModelList`：⚠️ 下载 **admin-only**（服务端 403），而它挂在**人人可见的首页**检查项里 —— 按钮必须按 `isAdmin()` 门控。
- `LogConsole`：**挂载连 SSE、卸载断**，只在设置页 `monitor` 挂；颜色走 tokens。`AppModal`/`AppDialog`/`ToastHost` —— **别再手搓弹窗**。
- `ServerDirPicker`：进下一级用 `{dir, enter}`、**路径由服务端拼**。
  ⚠ `update:dir`（onMounted 也发）要**清已选文件**；外部改 `startDir` 要跟着 browse。
  ⚠ **`projectFile` prop 默认须空（opt-in）**，弹出用 `ServerFileBrowser`（细节 ⑥）。
- `RecorderConfig`：`GET /api/admin/ingest` **绝不回 `api_pass` 明文/token_hash**，只回 `api_pass_set`；`api_base` 非空要带 scheme。⚠ 卡内只有**录制软件**与**自动入库**两块 —— **没有「两样都收」的模式切换**；敏感项在「高级」里折叠。
- `AppModal` / `AppDialog` / `JobDrawer`：焦点圈定 / Esc / 滚动锁统一走 `composables/useOverlay.js`。
  ⚠️ **AppDialog 不再整块挂 `@keydown.enter`**：默认焦点在**取消**，只有 prompt 的输入框允许 Enter 提交。
- 其余：`HomeClipsOverview`（自动 `polish init` 传**绝对目录**）· `SubtitleEditor`（`addSub()` 后按 `start` 排序）· `prepare/SubtitleTrack`（横排属性条）· `MaterialList`（**改一边等于改两边**）· `StickerPicker`/`SfxPicker`（`name` = **去扩展名的文件名**）。
- ⚠️ **预览的 `<canvas id="dhampir-canvas">` 一被换掉就整块黑**：底座 attach 一页只接一次、**没有 detach** ⇒ **任务完成与切片段都不许重挂** —— 见 ②.7。

> 改动只允许落在本目录；跨模块改动先读对方卡，并跑两条门禁。
## 验证
```bash
npm --prefix webui run build
```
