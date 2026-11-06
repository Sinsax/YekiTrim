# webui/src/components — 精修组件层：时间轴编辑器 / 预览 / 封面设计器 / 运镜 UI
> 范围：`webui/src/components`/** ｜ 上游：`webui/src/views/*`｜ 验证：`npm --prefix webui run build`
## 职责

精修页与设置页的全部子组件。壳/侧栏/路由/缓存纪律见 `webui/src/AGENTS.md`，页面纪律见
`webui/src/views/AGENTS.md`，出片模板同口径算法见 `webui/src/polish/AGENTS.md`。

## 入口

- `TimelineEditor.vue`（7 条泳道，`kind` = `seg|event|sub|mute`）：props 见文件头（`propsTarget` 默认
  `'#tl-props-host'`）；emits `seek`/`toggle`/`preview`/`saved`/`done`/`reprocess` 一族。`saved` = **这批改动已落盘**（Dhampir 那条腿据此重载），与
  `preview`（本地镜像、**不落盘**）**别互相替代**；`done` 保证在它之后。
- `PolishPreview.vue`（自绘 DOM/GSAP 层，**只有这一层**）：props 见文件头（`config`/`subtitles`/`danmaku`/
  `fonts`/`fontFallback`/`selectRect`/`point` 等）；emits `time`/`playing`/`update:selectRect`/`update:point`。
  字体栈纪律见下节。
- `CoverDesigner.vue`：props `dir`(必填)/`pid`，emit `changed`（「生成封面」按 `polish_cover` 门控）。
- 其余组件逐个的职责与规则见「边界 · 其他组件」。

## 边界

**简化基础件**
- 页面级空态/下一步/首启检查：`AppEmptyState.vue` / `NextStepBar.vue` / `SetupChecklist.vue`。
- LLM 快配：`components/llm/LlmQuickSetup.vue` + `presets.js`（只给 base_url/模型提示，不是能力表）；
  详细多接口编辑仍在 `LlmProviders.vue` 的「高级配置」里。
- 新控件带 `data-cap`；高级项用 `useDisclosure`/`<details>` 默认折叠，隐藏 ≠ 删除。

> 组件逐个的详细清单、实测数字、几何/常量表与推导见 **`docs/webui-components.md`**，按组件分节：
> ① 时间轴编辑器 ② 精修预览 ③ 封面设计器 ④ 运镜 ⑤ 贴纸落点与区域编辑 ⑥ 其他组件逐个。
> 各节内部自带子目录（布局几何 / 安全区几何 / 两种「隐身」根因等）。
### 时间轴编辑器
- **布局必须保持紧凑**（它**独占一整行** = 整个内容宽度）：泳道 **26px ×7 条 + 标尺 20px = 202px**（标尺带里
  叠了条 16px 的**热度曲线**：弹幕按条 + 字幕按字量，口径 `polish/heat.js`，开着时 218px —— 几何、四条
  装配纪律与"眼睛不进 `[stage.lanes]`"见 ①.8）；标题/提示经 `<slot name="head">` 进 `.tl-head`（见 ①）。
- ⚠️ **遮挡**：曲线画在**字幕泳道**上，而那条泳道的字幕块不透明（`laneBlocks`）—— 波形 `.wave-layer` 必须
  是 **`z-index:4`**（压过字幕块与选中块、在播放头之下），否则**说话时段整段被盖住**；`pointer-events:none`
  不能去掉（①.2）。
- ⚠️ **刻度**：窗口 **[-40, -8]**（`WAVE_DB_TOP`/`WAVE_DB_BOTTOM`）—— 这两个常量即"看不出高低差"的旋钮
  （改回 [-60,0] 动态范围只剩 6.6px）；曲线判据 `node webui/scripts/check-heat-curve.mjs`。
- 用户明确**不要**电平参考线/刻度、**不要**新增泳道（保持 7 条 202px）、**不要**把字幕块改半透明。
- 事件字段是 `time` + `end`，**没有 `duration` 了**；老工程的 `duration` 由 `legacy.rs::upgrade_event_times`
  在**反序列化前**换算；三处求值器统一走各自的 `evDur()` / `evDuration()`，别再直接读 `.duration`。
- **控件统一**：**体拖移动 / 左缘改开始 / 右缘改结束 / 双击删除**，字幕与屏蔽块也能拖；唯一例外是想换轨道就
  删掉重加，别再退回 15 种全类型下拉。**属性面板 = 标配块 + 类型专属块**；**「保存修改」按钮已删**。
- ⚠ **字幕/屏蔽轨道必须保持按时间有序**：`splitSelected` 拆出的后半段要 `splice(i+1, 0, …)` 插在原块**后面**，
  **不能 `push` 到数组末尾** —— 追加让落盘的 `clip.srt` / `data/subtitles.json` 乱序（去静音曾判出
  13 段/121s 的假静音、预览按时间二分取当前字幕）。
- ⚠️ **别再改回"落盘后 1.2s 自动跑"**（你还在填 keyword 时它就会插进来，半成品被当成品处理）。**落盘 ≠ 编辑
  完成**：后处理只在 `done` 之后跑（`done` 本身等落盘成功）——控件 `change`（`input` 每敲一键**不算**）/取消
  选中/切片段（带**旧 dir**）/卸载前 `flushSave()`。
- **手工放的事件（`manual`）后处理一律不改写**：新建（`addEventAt`）与改时间都打这个标记；
  **吸附与 `cap_events` 抽稀全跳过**。
- **属性面板用 `<Teleport :to="teleportTo">` 投到 `PolishView` 预览卡里的 `#tl-props-host`**（选中/编辑逻辑留在
  组件里）。**⚠️ 必须门控 Teleport**：目标是**挂载时**解析的，而那时子树还没进 document ⇒ 直接
  `:to="propsTarget"` 会解析到 null、**面板整块不渲染**（①.5）。
- ⚠️ **别用裸 `<template>` 包裹要无条件渲染的内容**：Vue 会将其渲染成真正的 `<template>` 元素，要无条件渲染就
  直接把内容展开。同理：**前端布局类改动必须做几何断言**（`getBoundingClientRect()` 宽高 > 0）。
- ⚠️ 写 headless harness 时：**`app.mount(host)` 会清空 `host` 的内容**、选中块只能靠 **pointerdown**
  （`block.click()` 不选中）—— 见 `docs/webui-components.md` ①.5。
- **播放头跟随 = 只在播放时跟，且越界才滚、只滚刚好**：`watch(currentTime)` 第一句是 `if (!props.playing) return`；
  `drag`/`rulerDragging`/手动滚动后 2 秒（`userScrolling`）一律不跟。**别再退回"每帧把播放头钉在 30% 处"**。
- **分段为空时时间轴照常渲染**：「去静音」在时间轴头部**常显**，**别再加回 `v-if="segments.length"` 门控**；
- ⚠ **「读取失败」不许伪装成正常状态，也不许在基线不可信时写盘**（2026-09-16，一批"静默丢改动"的根因）。
  `flushSave` 开头 `if (loadError.value) return`；横幅 + 「重试」在 `tl-head` 下面（**不能放进 Teleport 的面板**）；
  `/api/polish/subtitles` 的 **404 是空态、不是错误**；**不许静默丢弃**；`watch(() => props.dir)` 里 `commitEdit`
  之后必须 `resetTimelineState()` 再 `loadData()`

### 精修预览（`PolishPreview.vue`）
- ⚠️ **跳手动屏蔽区间 = A/B 预滚（首选）+ 定格遮罩（兜底），不许退回"直接 seek"**：`videoRef` 是 **computed**
  （换台靠 `activeStage`；Dhampir 那条腿靠 `engine.bindSource` 换绑 + 源没就绪不画），改回模板 ref、删掉备用源/
  那道闸门 = A/B **静默失效**（②.1/②.2）。
- 自绘层与出片模板 `crates/yekitrim-polish/templates/index.html` **必须同口径**（覆盖层色档 `gradientCss` 等纯函数
  从 `polish/engine.js` 来）。⚠️ **只有自绘这一层**：两条真渲染通道（wasm 实时 + `render-frame` 单帧回退）随
  「作废 wgpu」删掉了 —— **别再接回来**，要真渲染走 Dhampir 底座。
- 媒体地址一律走 `utils.js::clipMediaUrl(pid, dir, 相对路径)`，别自己拼 `/api/media/...`。
- 音量/静音 = **总音量，必须落到所有会出声的 `<video>` 元素**（**只调 SFX 播放器即 bug**）；读回用
  `isFinite` + clamp，**不能写 `parseFloat(...) || 1`**。**别再写死 `66vh`**（②.6）。⚠️ **Dhampir 模式下"台上那块"不是
  唯一在响的元素**：`applyAudio()` 必须覆盖 `dhampirSourceVideos` ∪ `dhampirTwins`（**不能只遍历
  `dhampirVideos`** —— 换绑中间态里旧元素会掉出数组却仍在播 ⇒ **声音翻倍**）；备用那批一直 `muted`（判据
  `dhampirIsTwin`）；**`tick()` 的 Dhampir 分支不许漏 `player.sync(...)`**（"让出重活"让的是**渲染**，音效是
  **出声**）；出声时纠偏速率要收窄。详见 **`docs/webui-components.md` ②.6**；判据 =
  `node scripts/check-preview-audio.mjs`。
- ⚠️ **字体栈顺序 / 字重必须与出片侧同源**（`.cap` ↔ 服务端 `fallback_stack`：页面透传
  `fetchFonts().fallback_stack`，**不许前端写死字体名** —— 写死则「设为默认」对预览永远无效；另有
  `--font-bold-file`）：漂了 = 预览雅黑、成片 Noto。
- ⚠ 动图（GIF/动画 WebP）由底座自己解码（v6）：四条硬规矩与调用顺序陷阱见 **`docs/webui-components.md` ②.4**。
- ⚠ **预览看着正常 ≠ 出片正常**：出片的事实来源是 `templates/index.html` 的 canvas 路径
  （`yekitrim-render-core/src/timeline.rs` 是平行实现、不是事实来源）。"改完没反应"先怀疑没重建/没重启。
- ⚠️ **Dhampir 加载期间不许播**（门帘 + 播放键置灰 + `togglePlay` 三道闸；热重载不挂）：见
  `docs/webui-components.md` §2.5。
### 封面设计器（`CoverDesigner.vue`）
- 细则（图层栈/镜像清单/四处同步/写入门闸/安全区）见 **`docs/webui-components.md` §③**；
  改它之前先读那张表，别只改前端镜像（出片侧 `cover.rs` 与 `templates/cover.html` 必须同步）。
  `configLoaded` 是写入门闸：读失败出错误条 + 重试，别拿默认值写回。
### 运镜（camera）
- **倍率只有一张表**：`CameraStyle::factor`，四处都走它。⚠ 模板 canvas 路径曾硬编码 `1.8/1.5`、完全不读
  `[style.camera]`，而它才是出片路径 —— 别写回去。
- **三档倍率当前值：中 1.3 / 近 1.5 / 画中画 0.55**。**放大档没有「推出框露背景」的风险**（绕不动点 `scale`，
  `s ≥ 1` 四周一律向外；露底只在画中画 `zoom_wide < 1`，刻意留黑边 `(1-factor)/2×1920 = 432px`）。三处兜底
  默认值（`default_zoom_*()`、模板与 `engine.js` 的 `camFactor`）一起改。
- ⚠ **运镜门 = 运镜轨的眼睛**（`[stage].lanes.camera`，取代 `portrait_no_camera` 硬门）：模板 `getActiveCamera`
  与 `engine.js cameraAt` 整条返回 `zoom=1 / handheld=0`。六轨眼睛/轨道设置见 `docs/webui-components.md` ①.7。
  **画中画到点收工**：`close_pip_shots` 补一条**回镜头**真事件，档位/不动点取**时间上**前一条非画中画运镜的
  **落点**；下一条在 `MIN_RETURN_SECS`(0.5s) 内就来则不补。幂等。⚠️ **前后镜头按时间算、不按文件顺序**。
- `process` 给每个 ≥1s 且没有运镜的段补 `zoom="中", anchor="liver"`；改默认档位/锚点就在这一处
### 贴纸落点（Step 2b）与区域编辑
- `align_stickers_to_subtitles` 起点吸到**最近那条字幕**的开始、终点吸到它的结束（>
  `STICKER_SNAP_MAX_SECS`(2.0s) 的不动）。**必须排在 2b 落点之前**。⚠️ **手工放的事件不受后处理改写**（用户报过
  「调整贴纸的结束时间没有成功」）：
- ⚠ **人物框同时喂 Step 2b 与 2d；弹幕区域既是贴纸障碍，也是 `anchor="danmaku"` 没真实弹幕时的兜底** —— 别
  退回"人物框只影响运镜中心 / 弹幕区域仅作参考"那套加 Step 2d 之前的旧说法。
- 读取优先级：`free_rect()` → `region_rects()`/`region_origin()` 一律**自由选框 > grid > `origin_grid`/`origin`**；
  **固定落点的 UI 是"草稿态"**：必须按「保存落点」才落盘；**保存成功才退出编辑态**。
### 其他组件
- `ProjectClipsSection`（切片页卡片墙）：⚠️ **删除不是任务（不发 job），刷新只能由前端自己驱动**：它拿的是父页面
  （`saveEdit`/`addClip` 同理）。`silent` 必须真的静默；**「剪切」按钮不禁用，忙时只改文案**（占用本来就排队）。
  ⚠️ 卡片的「精修」按钮（`goPolish`）：剪切收尾已把工程建好了，正常直接跳；**只有老片段**才补 init，且
  **必须 `await waitJobDone(id)` 再跳**（发完就跳会先撞 `polish.toml not found`；细节见 ⑥）。`polishOpening` 防
  连点，`jobClosers` 在 `onUnmounted` 里关订阅。
- `MediaLibrary`（首页素材库，**文件夹管理器**布局，2026-10-09 重写；`FontLibrary`/`StickerPacks`
  **已删除**、能力全并入它 —— 改素材库只改这一处）：左=类型导航、右=该类型网格；三种素材折成同一
  形状（`items` 的映射），**一个网格画三种** —— ⚠️ 别再退回"三块各自实现"。规则与坑见
  `docs/webui-components.md`「素材库」：**视频没有上传**（几 GB，拖拽直接 return）· 缩略图抽不出是
  **404 不是 500** · 删除一律 `confirmDialog` · 前端上传上限预检必须留着（超限回 413 纯文本，
  `client.js` 只认 JSON，会误报成"接口不存在"）。
- `SliderField`：⚠ **设置页的数值项走 `components/SliderField.vue`**（滑块 + 行尾可直填数字框，16 处）。
  ⚠ **滑块宽度 = 所在格子的 1/3**（`input[type=range]` 的 `width:100%` 会被 flex-basis 盖掉，**别再改回 100%**）；
- `JobList`/`jobs/JobDrawer`（任务列表与按钮门控 —— "界面哑火"的来源，2026-09-16 一次补齐）：
  - ⚠ **请求必须有超时**：`jobsApi` 自己传 **10s**（`api()` 默认 10 min）—— 没超时的 `fetch` 卡住就永不 settle，
    而 `useJobs` 的去重 `inflight` 只由 `.finally` 清，一次卡住的 `/api/jobs` 会把任务列表与所有按作用域门控的
    按钮一起冻住（症状：队列加载不出来 + 剪切按钮一直灰）。新增轮询接口自己给短超时；noisy 接口**慢过
    `SLOW_MS`(1s) 照样记 info**。
  - ⚠ **读失败 ≠ 没有任务**：`useJobs.load()` 失败时**保留上一份快照**并置 `lastError`，`JobList` 把「读取中 /
    读取失败（带重试按钮）/ 真的没有任务」**分三态**显示（以前失败写 `jobs.value = []`，界面和"真的没有任务"
    长得一样）。
  - ⚠ **门控要能被陈旧快照兜住**：`busyOn` 只是"提前告诉用户要等"，**不是正确性保证**。`useJobs.stale`
    （快照超过 `SNAPSHOT_TTL_MS`=15s）为真时各视图必须把 `busyOn` 降级成 `null` 放行 —— 否则一次读取故障就把
    按钮永久锁死。**别把门控当"不许点"**：宁可放行让服务端去排队。
  - ⚠ **任务结束的刷新必须 `silent`**：`ProjectSlicesView.onJobDone` 曾漏了 `silent:true` —— 非静默 `load()`
    一置 `loading`，`v-if="loading"` 就把卡片墙换成骨架屏并**卸载 `ProjectClipsSection`** → `clipsRef` 变 null
    → 工具条三颗按钮全灰（慢盘 3–4 秒，而它每个任务结束都跑）。这类刷新还要**单飞 + 尾随重刷**。
  - ⚠ **按钮按作用域门控**：`api/jobs.js` 的 `clipKey/outKey/idxKey/MODELS_KEY/STICKERS_KEY` 必须与
    `crates/yekitrim-server/src/scope_key.rs` 一致，`busyOn(jobs, keys)` 求交集 —— 只在自己片段/项目被占用时才
    禁用，「别处有任务在跑」不再让按钮变灰。**同步保存类操作（选框/封面参数/字幕/去静音）不受门控。**
  - **任务行的"卡住"提示**：`JobSummary.updated_at`（epoch 秒，`Job::push` 每次事件都更新）—— 运行中但
    `IDLE_WARN_SECS`(120s) 没有新事件，就在行上写"可能卡在外部进程或网络盘上"。
  - ⚠️ **前端订阅**：`watchJob()` 返回的是 **`{ close() }` 对象**，关订阅要 `w.close()` —— 当函数调
    （`jobClose?.()`）会抛 `jobClose is not a function`，而且它就在 `jobClose = watchJob(...)` 上一行：抛错后
    **新任务永远订阅不上，界面从此停在「运行中」**。
  - 编辑器里被推迟的后处理（时间轴自动保存 / 选框保存触发的 process）：**不进队列**，只记 `pendingProcess`
    （带片段名），该片段空闲时自动补跑一次 —— 一次编辑风暴不该塞十几条任务。
- `ModelList`：⚠️ 下载 **admin-only**（服务端 403），而它挂在**人人可见的首页**「运行状况」里 —— 按钮必须按
  `isAdmin()` 门控。
- `LogConsole`：**挂载连 SSE、卸载断**，只在设置页 `monitor` 挂；颜色一律走 tokens。品牌标
  `src/assets/logo.png`（favicon 在 `public/`），别改回内联 SVG。`AppModal`/`AppDialog`/`ToastHost`
  （`stores/toast`）—— **别再手搓弹窗/确认**。
- `ServerDirPicker`：进下一级用 `{dir, enter}`、**路径由服务端拼**。⚠ `update:dir`（onMounted 也发）要**清已选
  文件**；外部改 `startDir` 要跟着 browse；`update:listing` 供判可用性。⚠ **`projectFile` prop 默认须空
  （opt-in）**：传了才有绿框与「工程配置」标签。弹出用 **`ServerFileBrowser`**（双栏；细节见 ⑥）。
- `RecorderConfig`：`GET /api/admin/ingest` **绝不回 `api_pass` 明文/token_hash**，只回 `api_pass_set` 布尔；
  `api_base` 非空要带 scheme。
- 其余：`HomeClipsOverview`（自动 `polish init` 传**绝对目录**）· `SubtitlePanel`/`SubtitleEditor`（`addSub()`
  后按 `start` 排序）· `prepare/SubtitleTrack`（常驻横排属性条，不要右侧竖面板）· `prepare/MaterialList`
  （**改一边等于改两边**）· `polish/StickerPicker`/`SfxPicker`（`name` = **去扩展名的文件名**）。
- ⚠️ **预览的 `<canvas id="dhampir-canvas">` 一被换掉就整块黑**：底座 attach 一页只接一次、**没有 detach** ⇒
  任何重挂都让新 canvas 从没被 `present` 过（F5 才好）。**任务完成与切片段都不许重挂** —— 见
  `docs/webui-components.md` ②.7。

> 改动只允许落在本目录；跨模块改动先读对方卡，并跑两条门禁。
## 验证
```bash
npm --prefix webui run build
```
