# webui/src/components — 精修组件层：时间轴编辑器 / 预览 / 封面设计器 / 运镜 UI
> 范围：`webui/src/components`/** ｜ 上游：`webui/src/views/*`（传 `pid`/`dir`/配置，接 `preview`/`done`/`changed`）｜ 验证：`npm --prefix webui run build`
## 职责

精修页与设置页的全部子组件。壳 / 侧栏 / 路由 / 缓存与单飞纪律见 `webui/src/AGENTS.md`，页面文件纪律见
`webui/src/views/AGENTS.md`，求值器与出片模板的同口径算法（覆盖层色档 / 字幕折行 / 贴纸锚点 / 手持摇摆 /
衬底）见 `webui/src/polish/AGENTS.md` —— 本卡只写**组件级**规矩，不重复它们。

## 入口

- `TimelineEditor.vue`（7 条泳道，`kind` = `seg|event|sub|mute`）：props `dir`(必填)/`pid`/`currentTime`/
  `playing`/`reprocessBusy`/`propsTarget`(默认 `'#tl-props-host'`)；emits `seek`/`toggle`/`preview`/`done`/
  `reprocess`/`retried`/`redo-segment`。**`preview` = 本地改动立刻镜像给页面**（不落盘、不重挂载）；
  **`done` = 一次编辑真的做完了**，页面据此跑后处理。
- `PolishPreview.vue`（自绘 DOM/GSAP 层 + 可选真渲染）：props `pid`/`dir`/`config`/`subtitles`/`danmaku`/
  `metaDuration`/`wgpuRender`/`selectMode`/`selectRect`/`selectLabel`/`pointMode`/`point`/`pointLabel`；
  emits `time`/`playing`/`update:selectRect`/`update:point`。
- `CoverDesigner.vue`：props `dir`(必填)/`pid`，emit `changed`（只在该片段被占用时禁用「生成封面」）。
- 其余组件逐个的职责与规则见「边界 · 其他组件」。

## 边界
> 组件逐个的详细清单、实测数字、几何/常量表与推导见 **`docs/webui-components.md`**，按组件分节：
> ① 时间轴编辑器（布局几何 / 响度包络线 / 时间模型与控件 / 落盘与后处理 / Teleport 与 headless harness /
> 播放头·空分段·读取失败）② 精修预览 ③ 封面设计器（三层与背景模式 / 关键帧手势 / 安全区线框几何 /
> 标题自动排版 / 镜像清单 / 素材黑边根因 / 三处同步与写入门闸）④ 运镜（倍率表 / 硬切与手持摇摆 /
> 画中画配额·对齐·收工 / 位移 / 默认与迁移）⑤ 贴纸落点与区域编辑 ⑥ 其他组件逐个。
### 时间轴编辑器
- **布局必须保持紧凑**（用户要求精简；它是竖高大户，且**独占一整行**：v6.4 起横向空间 = 整个内容宽度，
  别再把它塞回左列）：泳道 **26px（原 34）×7 条 + 标尺 20px = 202px**；卡片头取消 —— 标题/提示经 `<slot name="head">` 进 `.tl-head`，与「去静音/撤销去静音」「重新后处理」同排。
- ⚠️ **遮挡**：曲线画在**字幕泳道**上，而那条泳道的字幕块是 `laneBlocks` 里写死的不透明 `#3b82f6`
  （`top:3px; height:20px`），原来波形 `z-index:1`、块 `z-index:2` —— **说话时段（最该看电平的时候）整段被盖住**；
  现在 `.wave-layer` 是 **`z-index:4`**（压过字幕块(2)与选中块(3)、仍在播放头(5)之下），`pointer-events:none` 不能去掉。
- ⚠️ **刻度**：原来是 `h = ((db+60)/60) × 22`，即窗口 [-60, 0]；而实测
  （`media/…/clip-08/data/loudness.json`，**1330 点**，0.1s 一点）语音落在 **-12 ~ -32 LUFS**、静音 -33 ~ -56 ——
  旧窗口下动态范围只有 6.6px；现在窗口是 **[-40, -8]（`WAVE_DB_TOP`/`WAVE_DB_BOTTOM`）**，动态范围 12.9px（约两倍），且不削顶。
  **这两个常量就是"看不出高低差"的旋钮。**
- ⚠️ **形态**：原来是「一个 0.1s 采样点一根 rect、宽 `max(1, scale*0.02)`」—— 两头都看不出"位置"。
- 用户明确**不要**电平参考线/刻度，**不要**新增泳道（保持 7 条 202px），也**不要**把字幕块改半透明 ——
  这三条都是当时一起评估过、被否掉的方案，别再提。
- 事件字段是 `time` + `end`，**没有 `duration` 了**；老工程的 `duration` 由 `legacy.rs::upgrade_event_times`
  在**反序列化前**换算；三处求值器统一走各自的 `evDur()` / `evDuration()`，别再直接读 `.duration`。
- **控件统一**：**体拖移动 / 左缘改开始 / 右缘改结束 / 双击删除**，字幕与屏蔽块也能拖；唯一例外是
  想换轨道就删掉重加，别再退回 15 种全类型下拉。**属性面板 = 标配块 + 类型专属块**；**「保存修改」按钮已删**。
- ⚠ **字幕/屏蔽轨道必须保持按时间有序**：`splitSelected` 拆出来的后半段要
  `splice(i+1, 0, …)` 插在原块**后面**，**不能 `push` 到数组末尾** —— 追加会让落盘的 `clip.srt` /
  `data/subtitles.json` 乱序（去静音按相邻间隔曾判出 13 段/121s 的假静音、预览按时间二分取当前字幕）。
- ⚠️ **别再改回"落盘后 1.2s 自动跑"**：你还在面板里给新建的贴纸填 keyword 时它就会插进来，
  半成品被当成品处理（数量抽稀直接把它删掉）。**落盘 ≠ 编辑完成**：后处理只在 `emit('done')` 之后跑 ——
  控件 `change`（`input` 每敲一键**不算**）/取消选中/切片段（带**旧 dir**）/卸载前 `flushSave()`。
- **手工放的事件（`manual`）后处理一律不改写**：新建（`addEventAt`）、复制（`duplicateSelected`）与改时间都打这个标记；**三处吸附与 `cap_events` 数量抽稀全部跳过**。
- **属性面板用 `<Teleport :to="teleportTo">` 投到 `PolishView` 预览卡里的 `#tl-props-host`**（选中状态与编辑逻辑留在组件里）。
  **⚠️ 必须门控 Teleport**：Teleport 的目标在**挂载时**用 `document.querySelector` 解析，而子组件挂载时整棵子树还没 `hostInsert` 进 document → 直接 `:to="propsTarget"` 解析到 null、**面板整块不渲染**。做法：`teleportTo = ref('')`，`onMounted` 里 query 到了再设值（`nextTick` 重试 + 失败 warn）。
- ⚠️ **别用裸 `<template>` 包裹要无条件渲染的内容**：Vue 会把它渲染成真正的 `<template>` 元素，
  要无条件渲染就直接把内容展开。同理：**前端布局类改动必须做几何断言**（`getBoundingClientRect()` 宽高 > 0）。
- ⚠️ 写 headless harness 时：**`app.mount(host)` 会清空 `host` 的内容** —— 挂载点（`#tl-props-host`）要挂在 host 的**内层节点**、组件 mount 到另一个内层节点；选中块靠 **pointerdown**（`startPointer`，`block.click()` 不会选中）。
- **播放头跟随 = 只在播放时跟，且越界才滚、只滚刚好**：`watch(currentTime)` 第一句是 `if (!props.playing) return`；
  `drag`/`rulerDragging`/手动滚动后 2 秒（`userScrolling`）一律不跟。**别再退回"每帧把播放头钉在 30% 处"**。
- **分段为空时时间轴照常渲染**：「去静音」在时间轴头部**常显**，**别再加回 `v-if="segments.length"` 门控**；
- ⚠ **「读取失败」不许伪装成正常状态，也不许在基线不可信时写盘**（2026-09-16，一批"静默丢改动"的根因）。
  `flushSave` 开头 `if (loadError.value) return`；横幅 + 「重试」在 `tl-head` 下面（**不能放进 Teleport 的面板**）；
  `/api/polish/subtitles` 的 **404 是空态、不是错误**；**不许静默丢弃**；`watch(() => props.dir)` 里 `commitEdit` 之后必须 `resetTimelineState()` 再 `loadData()`
### 精修预览（`PolishPreview.vue`）
- 自绘层与出片模板 `crates/vtrim-polish/templates/index.html` **必须同口径**（覆盖层色档 `gradientCss`、
  失败则**回退 server 单帧** `POST /api/polish/render-frame`（裸 `fetch`，写请求必须自己带 `csrfHeaders()`）；
- 媒体地址一律走 `utils.js::clipMediaUrl(pid, dir, 相对路径)`，别自己拼 `/api/media/...`。
- 音量/静音 = **总音量，必须同时落到 `<video>` 元素**（**只调 SFX 播放器就是 bug**）；读回用 `isFinite` + clamp，
  **不能写 `parseFloat(...) || 1`**。**别再写死 `66vh`**。
- ⚠ **预览看着正常 ≠ 出片正常**：出片的事实来源是 `templates/index.html` 的 canvas 路径
  （`vtrim-render-core/src/timeline.rs` 是平行实现、不是事实来源）。"改完没反应"先怀疑没重建/没重启。
### 封面设计器（`CoverDesigner.vue`）
- **三层：文字 / 关键帧 / 背景**（`#title` / `#frame` = `_cover_frame.jpg` / `#bg`）：**三层各有各的图源，谁也不许覆盖谁**。
- ⚠️ **导入背景图片写的是 `_cover_bg.jpg`**（`/api/polish/cover-background` 同时把 `bg_mode` 设成 `image`）。
  **别再写回 `_cover_frame.jpg`** —— 那是"背景替换关键帧"的老 bug（用户报过「又把关键帧那一层给我删了」）；`cover.image` 导入时**不要清空**（它也是关键帧的来源之一）。
- ⚠️ **出片不再删 `_cover_frame.jpg`**（`cmd_cover` 末尾那句 `remove_file` 已删）：它就是关键帧层的图源，删了等于①用户导出的那一帧白导、②下次出片又按 `frame_pos` 重新截、同一份封面每次换脸。
- ⚠️ **关键帧文件可能不存在**（新工程没导出过、也没出片过）：模糊背景会先 404、图被 v-if 摘掉像"没运行" → `onBgImageError()` **只对 blur 模式**、**不带 time** 调 `/api/polish/cover-frame`（按 `[cover] frame_pos` 截），只补一次（`bgFrameTried`），再失败提示手动「导出关键帧」。
- **安全区线框**：**16:9 安全区**（`.guide-169`）= `width:95%; height:95%` 居中 → 舞台坐标 **1824×1026**；
  **4:3 裁切区**（`.guide-43`）= `height:100%` + `aspect-ratio:4/3` 居中 → **1440×1080**。
  ⚠️ **别写成 `inset:5%`**：CSS 的 inset 百分比是**横向按父宽、纵向按父高**解析的，看着像等比其实不是（会得到 1728×972，连 16:9 都不是）；等比只能写 width/height。
- ⚠️ **只画在设计器里**：`templates/cover.html` 与 `cover.rs` 一个字都没改，所以线框**不可能**进入成片。
  将来若要往 CLI 那个独立预览页加，必须同时补 `html.export .guide-layer{display:none}`，否则进成片。
- **标题首次自动排版 `fitTitleIntoSafeArea()`**：触发只有两处（`loadCoverConfig` 发现磁盘上 `title_lines` **为空**、
  `generateTitleByLlm` 拿到新标题），手动操作**一律不触发** —— **别在 watch 里接它**。
  **已有封面一律不碰**（判据 = 磁盘上有没有 `title_lines`）；只动 `font_size`/`text_x`/`text_y`（必要时一次 `text_width`）。
- ⚠️ **默认对齐是 `left`**（`default_cover_text_align()`，2026-09 由 `center` 改来）：已有工程的 `polish.toml` 都显式写了 `text_align`，字段存在就不走默认；**不要顺手改 `cover.rs::cover_text_align` 的 `_ => ("center","center")` 兜底** —— 那个 `_` 同时兜着合法值 `"center"` 本身，改了会把显式居中的老封面一起翻成左对齐。
- ⚠️ **`text` 是用户输入，进模板前必须 `escape_html`**（不转义＝往封面 HTML 里注模板，有单测）。
- ⚠️ **设计器预览 ⇄ 出片的镜像清单**（用户报过"设计器和实际渲染的图片不一致"；下列每一项
  emoji 字号 `48+amount×240`、`drop-shadow(0 6px 14px)`，预览一律乘 `stageScale`）·
  **`liverOrigin()` 的优先序必须与 `region_origin` 一致**：自由选框中心 > `origin_grid` > `origin` > `grid[0]` > `65% 50%` ·
- · ⚠️ **设计器的"素材左右黑边"**（用户报过"设计器里素材左右两条黑边、出片正常"）：`contain` 让出的留白由**元素自身背景**填充，而 `styles/app.css` 的全局元素选择器 `video { background:#000; border-radius:var(--r-sm) }` 会穿透 `.stage-video`（类选择器只覆盖自己写过的属性），还压在 `.stage-bg`（z-index 0 < `.stage-media` 的 1）之上把模糊背景层整个盖住。所以 `.stage-video` 的 `background:transparent; border-radius:0` 是**必需的覆盖、不是装饰**；出片模板用 `<img>` 且不引 app.css，所以**只有设计器**这样（根因与几何钉在 `node webui/scripts/check-cover-stage.mjs`）。
- **三处必须同步**：`CoverConfig`、`/api/polish/cover-config` 的白名单 patch（漏键＝"点了没反应"）、`templates/cover.html`。
  `sanitize_hex_color`（**颜色直接进 HTML/CSS，必须净化**，非法值退 `#000000`）、`cover_bg_mode`
  （只有 `blur` 走模糊，其余一律 `color`）。`configLoaded` 是**写入门闸**：读失败必须出错误条 + 「重试」。
### 运镜（camera）
- **倍率只有一张表**：`CameraStyle::factor`，四处都走它。⚠ 模板 canvas 路径曾硬编码 `1.8/1.5` 且完全不读
  `[style.camera]`，而它才是出片路径（`chrome-render` 为默认出片路径）——别再把倍率写回去。
- **三档倍率的当前值：中 1.3 / 近 1.5 / 画中画 0.55**。**放大档位不存在「推出框露背景」的风险**（运镜是绕不动点
  做 `scale`，`s ≥ 1` 时四周一律向外扩张；露底只发生在画中画 `zoom_wide < 1`，刻意留黑边 `(1-factor)/2×1920 = 432px`）。
  三处兜底默认值（`default_zoom_mid/close/wide()`、模板 `camFactor`、
  `engine.js` `camFactor`）必须一起改。
  **别再退回"靠 prompt/界面拦"**：守卫是唯一兜底。
- ⚠ **竖屏（`vertical = true`）跟运镜一起屏蔽**：四条求值路径全部归零（`timeline::evaluate_frame`、
  模板 `getActiveCamera` 返回 `CFG.vertical ? 0 : handheld`、`engine.js cameraAt` 竖屏分支返回 `handheld: 0`）。
  **画中画到点收工**：`close_pip_shots` 补一条**回镜头**真事件，档位/不动点取**时间上**前一条非画中画运镜的
  **落点**；下一条在 `MIN_RETURN_SECS`(0.5s) 内就来则不补。幂等。⚠️ **前后镜头按时间算，不按文件顺序**：段之间时间上会重叠（LLM 按窗口生成 + 去静音切段），求值器也是先按时间排序再走的。
- `process` 给每个 ≥1s 且没有运镜的段补 `zoom="中", anchor="liver"`；改默认档位/锚点就在这一处
### 贴纸落点（Step 2b）与区域编辑
- `align_stickers_to_subtitles` 起点吸到**最近那条字幕**的开始、终点吸到它的结束（> `STICKER_SNAP_MAX_SECS`(2.0s) 的不动）。
  **必须排在 2b 落点之前**。⚠️ **手工放的事件不受后处理改写**（用户报过「调整贴纸的结束时间没有成功」）：
- ⚠ **别再写回"保存不触发任务 / 人物框只影响运镜聚焦中心（渲染时读取）/ 弹幕区域仅作参考"**——那是加锚点解析（Step 2d）之前的旧说法：人物框现在同时喂 Step 2b 与 2d，弹幕区域也既当贴纸障碍、又是 `anchor="danmaku"` 在那一刻没有真实弹幕时的兜底。
- 读取优先级：`free_rect()` → `region_rects()`/`region_origin()` 一律**自由选框 > grid > `origin_grid`/`origin`**；
  **固定落点的 UI 是"草稿态"**：必须按「保存落点」才落盘；**保存成功才退出编辑态**。
### 其他组件
- `ProjectClipsSection`（切片页卡片墙）：⚠️ **删除不是任务（不发 job），刷新只能由前端自己驱动**：它拿的是父页面
  （`saveEdit`/`addClip` 同理）。`silent` 必须真的静默；**「剪切」按钮不禁用，忙时只改文案**（占用本来就排队）。
  ⚠️ 卡片的「精修」按钮（`goPolish`）：剪切收尾已经把工程建好了，正常直接跳；**只有老片段**才补 init，
  且**必须 `await waitJobDone(id)` 再跳**（发完 init 就跳会先撞 `polish.toml not found` → 精修页报「片段配置读取失败」，
  与首页 `HomeClipsOverview::openClip` 同一手感）。`polishOpening` 防连点，`jobClosers` 在 `onUnmounted` 里关订阅。
- `StickerPacks`（首页表情包区块，**仅 admin**）：⚠️ **这是全站第二处有意接收用户上传的入口**（第一处是封面背景图
  base64）；「服务器文件建项目」那条仍然**永不接受上传**，别按旧规矩把它删掉。⚠️ 前端
  `api/stickers.js::UPLOAD_MAX_BYTES` 与 Rust `UPLOAD_MAX_BYTES` 是同一个数（改一处要改两处），前端必须先挡。
- `SliderField`：⚠ **设置页的数值项走 `components/SliderField.vue`**（滑块 + 标签行右端可直填的数字框，16 处）。
  ⚠ **滑块宽度 = 所在格子的 1/3**（`input[type=range]` 的 `width:100%` 会被 flex-basis 盖掉，**别再改回 100%**）；
- `JobList`/`jobs/JobDrawer`（任务列表与按钮门控 —— "界面哑火"的来源，2026-09-16 一次补齐）：
  - ⚠ **请求必须有超时**：`api()` 默认兜一个松上限（10 min），`jobsApi` 自己传 **10s**。没有超时的
    `fetch` 一旦卡住就永不 settle，而 `useJobs` 的去重 `inflight` 只由 `.finally` 清 —— 一次卡住的
    `/api/jobs` 会把任务列表与所有按作用域门控的按钮一起冻在最后一帧。用户症状正是**"队列加载不出来 +
    剪切按钮一直灰"**，而日志里**一条 `/api/jobs` 都没有**（它在 `access_log.rs` 的 noisy 名单里只记 debug）。
    新增轮询类接口自己给短超时；noisy 接口**慢过 `SLOW_MS`(1s) 照样记 info**。
  - ⚠ **读失败 ≠ 没有任务**：`useJobs.load()` 失败时**保留上一份快照**并置 `lastError`，`JobList` 把
    「读取中 / 读取失败（带重试按钮）/ 真的没有任务」**分三态**显示。以前失败写 `jobs.value = []`，
    界面和"真的没有任务"长得一模一样（都是「暂无任务」）。
  - ⚠ **门控要能被陈旧快照兜住**：`busyOn` 只是"提前告诉用户要等"，**不是正确性保证**（正确性由服务端的
    资源键排队兜）。`useJobs.stale`（快照超过 `SNAPSHOT_TTL_MS`=15s 没刷新）为真时，各视图必须把 `busyOn`
    的结果降级成 `null` 放行 —— 否则一次读取故障就把页面按钮永久锁死。**别把门控当"不许点"**：宁可放行
    让服务端去排队。
  - ⚠ **任务结束的刷新必须 `silent`**：`ProjectSlicesView` 的 `onJobDone` 曾漏了 `silent:true` —— 非静默
    `load()` 一置 `loading`，`v-if="loading"` 就把整面卡片墙换成骨架屏并**卸载 `ProjectClipsSection`** →
    `clipsRef` 变 null → 工具条三颗按钮（`:disabled="!clipsRef"`）全灰；慢盘上这一次要 3–4 秒，而它
    **每个任务结束都跑一次**（用户报的"剪切完界面卡住、按钮变灰"）。这类刷新还要**单飞 + 尾随重刷**
    （`ProjectSlicesView.load` 的 `inflight`/`queuedReload`）。
  - ⚠ **按钮按作用域门控**：`api/jobs.js` 的 `clipKey/outKey/idxKey/MODELS_KEY/STICKERS_KEY` 必须与
    `crates/vtrim-server/src/scope_key.rs` 一致，`busyOn(jobs, keys)` 求交集 —— 只在自己片段/项目被占用时
    才禁用，「别处有任务在跑」不再让按钮变灰。**同步保存类操作（选框/封面参数/字幕/去静音）不受门控。**
  - **任务行的"卡住"提示**：`JobSummary.updated_at`（epoch 秒，`Job::push` 每次事件都更新）—— 运行中但
    `IDLE_WARN_SECS`(120s) 没有新事件，就在行上写"可能卡在外部进程或网络盘上"。
  - ⚠️ **前端订阅**：`watchJob()` 返回的是 **`{ close() }` 对象**，关订阅要 `w.close()` —— 当函数调
    （`jobClose?.()`）会抛 `jobClose is not a function`，而且它就在 `jobClose = watchJob(...)` 上一行：
    抛错后**新任务永远订阅不上，界面从此停在「运行中」**。
  - 编辑器里被推迟的后处理（时间轴自动保存 / 选框保存触发的 process）：**不进队列**，只记 `pendingProcess`
    （带片段名），该片段空闲时自动补跑一次 —— 一次编辑风暴不该塞十几条任务。
- `LogConsole`：**挂载连 SSE、卸载断**，只在设置页 `monitor` 子页挂；`.log-*` 是 scoped 新类，颜色一律走 tokens。
  `AppModal`/`AppDialog`/`ToastHost`（`stores/toast`）—— **别再在各页手搓弹窗/确认**。
- `ServerDirPicker`：**进入下一级用 `{dir, enter:"子目录名"}`，路径由服务端拼**（前端自己拼会把 canonicalize 的 `\\?\` 前缀带进去 → Windows 报 ERROR_INVALID_NAME 123）。
- `RecorderConfig`：`GET /api/admin/ingest` **只回 `api_pass_set` 布尔，绝不回 `api_pass` 明文或 `token_hash`**；
  `api_base` 非空必须带 scheme。
- 其余：`HomeClipsOverview`（自动 `polish init` 传**绝对目录**）· `SubtitlePanel`/`SubtitleEditor`（`addSub()` 后按
  `start` 排序）· `prepare/SubtitleTrack`（常驻横排属性条，不要右侧竖面板）· `prepare/MaterialList`
  （**改一边等于改两边**）· `polish/StickerPicker` + `polish/SfxPicker`（`name` 必须是**去扩展名的文件名**；库数据只读）·
> 改动只允许落在本目录；跨模块改动先读对方卡，并跑两条门禁。
## 验证
```bash
npm --prefix webui run build
```
