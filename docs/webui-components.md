# webui/src/components —— 组件级规则全文（模块卡的搬迁正文）

> 配套模块卡：`webui/src/components/AGENTS.md`（DSH 在触到 `webui/src/components/` 时自动注入）。
> 这份文档**不占指令预算**，是那张卡原来的全文：卡里只留"改代码前必须知道、否则会写错"的规则句与 ⚠️ 行，
> 实测过程、历史成因、逐项枚举清单、几何/常量表、算法推导与"为什么当初那样写"的叙述全部搬到这里。
> 判断标准：**改代码前必须知道、否则会写错的留卡；想追根问底时才看的进这里。**

分节：

- ① [时间轴编辑器](#①-时间轴编辑器timelineeditorvue)：[布局与几何](#11-布局与几何202px-的来历) ·
  [响度包络线](#12-响度包络线两个独立缺陷叠加) · [时间模型与控件](#13-时间模型--开始--结束v66) ·
  [落盘与后处理](#14-落盘后处理与-manual) · [Teleport 与 harness](#15-teleport-面板与-headless-harness) ·
  [播放头 / 空分段 / 读取失败](#16-播放头空分段与读取失败)
- ② [精修预览 `PolishPreview.vue`](#②-精修预览polishpreviewvue)
- ③ [封面设计器 `CoverDesigner.vue`](#③-封面设计器coverdesignervue)：[三层与背景模式](#31-封面三层与背景模式) ·
  [关键帧手势](#32-关键帧手势与-transform-的单一来源) · [安全区线框（几何表）](#33-安全区线框几何表) ·
  [标题自动排版](#34-标题首次自动排版) · [镜像清单](#35-设计器预览--出片的镜像清单) ·
  [素材黑边根因](#36-设计器的素材左右黑边根因) · [三处同步与写入门闸](#37-三处同步与写入门闸)
- ④ [运镜 camera](#④-运镜camera)：[倍率表](#41-倍率只有一张表) · [硬切与手持摇摆](#42-硬切与手持摇摆) ·
  [画中画配额 / 对齐 / 收工](#43-画中画配额对齐收工) · [位移](#44-位移) · [默认运镜与老工程迁移](#45-默认运镜与老工程迁移)
- ⑤ [贴纸落点（Step 2b）与区域编辑](#⑤-贴纸落点step-2b与区域编辑)
- ⑥ [其他组件逐个](#⑥-其他组件逐个)

---

## ① 时间轴编辑器（`TimelineEditor.vue`）

`TimelineEditor.vue` —— 精修时间轴（7 条泳道：分段/事件/运镜/贴纸/覆盖层/特效/字幕+屏蔽，`kind` = `seg|event|sub|mute`）。
props `dir`（必填）/`pid`/`currentTime`/`playing`/`reprocessBusy`/`propsTarget`（默认 `'#tl-props-host'`）；
emits `seek`/`toggle`/`preview`/`done`/`reprocess`/`retried`/`redo-segment`。
**`preview` = 本地改动立刻镜像给页面**（不落盘、不重挂载）；**`done` = 一次编辑真的做完了**，页面据此跑后处理。

### 1.1 布局与几何（202px 的来历）

**布局必须保持紧凑**（用户要求精简；它是竖高大户，且**独占一整行**：v6.4 起横向空间 = 整个内容宽度，
别再把它塞回左列）：泳道 **26px（原 34）×7 条 + 标尺 20px = 202px**；卡片头取消 —— 标题/提示经
`<slot name="head">` 进 `.tl-head` 一行，与「去静音 / 撤销去静音」「重新后处理」按钮同排；卡片 padding
收到 12px（`.tl-card`）。改泳道高度时同步三处：`wavePath`（常量 `WAVE_H = 26` 与 SVG 的 `height` 都在这儿）、
`.block`（`top:3px; height:20px; line-height:20px; font-size:10px`）、`.lane-label`/`.lane-track` 的 26px。

为什么是 202px：7 条泳道各 26px 是用户给出的"紧凑"上限（原 34px 时整块高度挤掉首屏的预览），
标尺 20px 是刻度的最小可读高度；卡片头被取消是因为它与 `.tl-head` 里的按钮行语义重复。

### 1.2 响度包络线：两个独立缺陷叠加

**响度显示 = 按像素列取峰值的包络线，压在不透明的字幕块之上**（2026-09 重做）。**两个独立缺陷叠加**，
改之前先读懂这两条，否则很容易只修一个、看着还是没变化：

- ⚠️ **遮挡**：曲线画在**字幕泳道**上，而那条泳道的字幕块是 `laneBlocks` 里写死的不透明
  `#3b82f6`（`top:3px; height:20px`），原来波形 `z-index:1`、块 `z-index:2` —— **说话时段（最该看电平的时候）整段被盖住**，
  只剩字幕之间的空隙和块上下各 3px。现在 `.wave-layer` 是 **`z-index:4`**：压过字幕块(2)与选中块(3)、
  仍在播放头(5)之下。`pointer-events:none` 不能去掉（否则挡住拖块与选中）。
- ⚠️ **刻度**：原来是 `h = ((db+60)/60) × 22`，即窗口 [-60, 0]；而实测
  （`media/…/clip-08/data/loudness.json`，**1330 点**，0.1s 一点）语音落在 **-12 ~ -32 LUFS**、静音 -33 ~ -56 ——
  整段语音的高度只在 11~17.6px 之间抖，26px 泳道里**动态范围只有 6.6px**，看着就是一条平线。
  现在窗口是 **[-40, -8]（`WAVE_DB_TOP`/`WAVE_DB_BOTTOM`）**，同样语音占 4.5~20.1px，动态范围 12.9px
  （约两倍），且不削顶。**这两个常量就是"看不出高低差"的旋钮。**
- ⚠️ **形态**：原来是「一个 0.1s 采样点一根 rect、宽 `max(1, scale*0.02)`」——
  1px/s 时 10 根柱子挤进 1px 糊成一片，120px/s 时点距 12px 而柱宽 1px、中间全是缝，两头都看不出"位置"。
  现在是**按 `WAVE_COL_STEP`(2px) 像素列聚合、每列取最大 db**（峰值保持，也正好是编辑器要的语义：
  "这一刻最响到过多少"；取均值会把瞬时爆点抹平，而找爆点正是看这条线的目的）。一列最多一个顶点 →
  **顶点位置永远跟着缩放走**，既不糊也不出缝；放大到比采样还细时会有空列，直接跳过让折线连过去。

**双层描边**：深色 halo（`rgba(15,23,42,.55)`、3.4px）+ 亮色芯（`#fff`、1.5px）—— 这条线要同时压在不透明蓝块
**和**近白背景（`--bg` #FFF7F3）上，单色必然在一边糊掉。**改颜色前先想清楚它会落在哪两种底上。**

用户明确**不要**电平参考线/刻度，**不要**新增泳道（保持 7 条 202px），也**不要**把字幕块改半透明 ——
这三条都是当时一起评估过、被否掉的方案，别再提。

实测数据留档：`media/…/clip-08/data/loudness.json` = 1330 点 / 0.1s 一点；语音 **-12 ~ -32 LUFS**；
静音 **-33 ~ -56 LUFS**；旧窗口 [-60, 0] 下语音高度 11~17.6px（动态范围 6.6px）；
新窗口 [-40, -8] 下 4.5~20.1px（动态范围 12.9px，约两倍），且不削顶。

### 1.3 时间模型 = 开始 + 结束（v6.6）

事件字段是 `time`（开始秒）+ `end`（结束秒），**没有 `duration` 了**；
分段/字幕/屏蔽本来就是 `start`/`end`，四类块因此共用同一套控件。老工程的 `duration` 由
`vtrim-render-core/src/legacy.rs::upgrade_event_times` 在**反序列化前**换算（`end = time + duration`），
入口四处：`polish.toml` 加载、`PUT /api/polish/project` 的 segments、LLM 事件输出 + 老 checkpoint、
wasm 预览配置。三处求值器统一走各自的 `evDur()` / `evDuration()`，别再直接读 `.duration`。

**控件统一**：**体拖移动 / 左缘改开始 / 右缘改结束 / 双击删除**，字幕与屏蔽块也能拖；唯一例外是
**分段没有左缘**（缩 start 会把事件留在段外，正确处理要连带搬事件）。块对象都带 `kind`，拖拽/选中/面板都按它分发；
分段体拖连带搬段内事件（用快照算增量，别逐帧累加）。

**事件类型由轨道决定，不给跨轨道选**（v6.7）：泳道标签上的 ＋ 定类型，视觉特效轨的 ＋ 弹 11 项细分菜单
（`openFxMenu`，点别处关闭；`closeFxMenu` 里用 `closest('.fx-menu')` 放行，别让 document 上的 capture pointerdown
提前清掉菜单）；想换轨道就删掉重加，别再退回 15 种全类型下拉。

**属性面板 = 标配块 + 类型专属块**：标配永远是 `开始（秒）/ 结束（秒）` + 「设为入点 / 设为出点 / 定位 / 删除」
（拆分只给字幕/屏蔽、复制只给事件）；**「保存修改」按钮已删** —— 面板容器上 `@input/@change` 事件委托 → `touch(kind)`。

### 1.4 落盘、后处理与 `manual`

- ⚠ **字幕/屏蔽轨道必须保持按时间有序**：`splitSelected` 拆出来的后半段要
  `splice(i+1, 0, …)` 插在原块**后面**，**不能 `push` 到数组末尾** —— 追加会让列表（以及落盘的 `clip.srt` /
  `data/subtitles.json`）乱序。乱序会连带毁掉所有按顺序读它的地方：去静音按相邻间隔算静音
  （曾判出 13 段/121s 的假静音）、预览按时间二分取当前字幕。`PUT /api/polish/subtitles` 落盘前会按 `start` 排序，
  `detect_silence_from_srt` 也会先排序+合并重叠 —— 但别因此就允许编辑器产出乱序列表。
- ⚠️ **别再改回"落盘后 1.2s 自动跑"**：你还在面板里给新建的贴纸填 keyword 时它就会插进来，
  半成品被当成品处理（数量抽稀直接把它删掉）。**落盘 ≠ 编辑完成**：后处理只在 `emit('done')` 之后跑 ——
  面板控件 `change`（失焦/选定；`input` 每敲一键**不算**）/ 取消选中 / 切片段（要带**旧 dir**）/ 组件卸载
  （卸载前先 `flushSave()`）。700ms 防抖 PUT 落盘；页面侧 `onEditDone` 再 1.5s 合并防抖 →
  `startJob('process', …, {silent:true, keepPreview:true})`。忙时走 `pendingProcess`（存 **dir**），空闲补跑一次。
- **手工放的事件（`manual`）后处理一律不改写**：新建（`addEventAt`）、复制（`duplicateSelected`）与改时间都打这个标记；
  `process` 侧三处吸附（`align_stickers_to_subtitles` / Step 2c / `align_pip_shots`）跳过它，
  **`cap_events` 的数量抽稀也跳过它**。面板 chip 写明语义（`手动` = 不吸附、不抽稀 / `自动对齐` = 会被吸附），
  「跟随字幕」清标记交回自动处理。

为什么当初那样写：「落盘后 1.2s 自动跑」的旧实现是在用户还在面板里编辑时触发的 —— 新建贴纸刚写下 keyword
就被 process 当成品处理，数量抽稀直接把它删掉，用户看到的是"贴纸自己消失了"。

### 1.5 Teleport 面板与 headless harness

- **属性面板用 `<Teleport :to="teleportTo">` 投到 `PolishView` 预览卡里的 `#tl-props-host`**（选中状态与全部编辑逻辑留在组件里）。
  **⚠️ 必须门控 Teleport**：Teleport 的目标在**挂载时**用 `document.querySelector` 解析，
  而子组件挂载时整棵编辑器子树还没 `hostInsert` 进 document —— 直接 `:to="propsTarget"` 会解析到 null、
  **面板整块不渲染，只剩挂载点那块空白**（实测踩过）。正确做法：`const teleportTo = ref('')`，在 `onMounted`
  里 `document.querySelector(target)` 成功后再设值（并留一次 `nextTick` 重试 + 失败 console.warn）。
  挂载点样式：父级 `position:relative; width:232px`，面板 `position:absolute; inset:0`（页面 scoped CSS 用 `:deep()`）。
- ⚠️ **别用裸 `<template>` 包裹要无条件渲染的内容**：Vue 会把它渲染成真正的 `<template>` 元素，
  而浏览器**不渲染 template 的内容** —— 表现是整块界面凭空消失，可 `document.querySelector` 照样能查到
  （DOM 存在 ≠ 看得见，这一版就这么翻过车）。要无条件渲染就直接把内容展开。
  同理：**前端布局类改动必须做几何断言**（`getBoundingClientRect()` 宽高 > 0）。
- ⚠️ 写 headless harness 时：**`app.mount(host)` 会清空 `host` 的内容** —— 属性面板的
  Teleport 挂载点（`#tl-props-host`）要挂在 host 的**内层节点**上、组件 mount 到另一个内层节点，
  否则挂载点被清掉 → `teleportTo` 解析不到 → 面板整块不渲染（harness 实测踩过）。
  另外：选中一个块是 **pointerdown** 干的（`startPointer`），`block.click()` 不会选中，
  得派发 `pointerdown` + window 上的 `pointerup`。

### 1.6 播放头、空分段与读取失败

- **播放头跟随 = 只在播放时跟，且越界才滚、只滚刚好**：`watch(currentTime)` 第一句是
  `if (!props.playing) return` —— **不播放时跳转/拖动一个像素都不动视角**；`drag`/`rulerDragging`/手动滚动后的
  2 秒（`userScrolling`）一律不跟。**别再退回"每帧把播放头钉在 30% 处"** —— 预览是 rAF 驱动的，那样等于一帧一滚、
  画面永远在飘。`playing` 这条链：`PolishPreview` 在 play/pause/ended 里 `emit('playing', …)` →
  `PolishView.previewPlaying` → `TimelineEditor :playing`（别用"时间在涨"猜播放状态）。坐标必须用
  **标尺轨道的实际左边**（`rulerTrackRef` 的 rect 减 scroller 的 rect）+ `t×scale`（最左边是 sticky 的 96px 标签列）。
- **分段为空时时间轴照常渲染**：`.tl-no-seg` 只留一句提示，整条时间轴（含字幕/屏蔽泳道）照旧画，
  别改成 `v-if` 整块隐藏。「去静音」在时间轴头部**常显**，**别再加回 `v-if="segments.length"` 门控**。
- ⚠ **「读取失败」不许伪装成正常状态，也不许在基线不可信时写盘**（2026-09-16，一批"静默丢改动"的根因）。
  `loadData` 直接调 `api()`（不再用 `apiOr` 兜底），失败只置 `loadError`、**不动现有数组也不清脏标记**；
  `flushSave` 开头 `if (loadError.value) return`；横幅 + 「重试」在 `tl-head` 下面（**不能放进 Teleport 的面板**）。
  `/api/polish/subtitles` 的 **404 是空态、不是错误**（只有非 404 才算读失败）。`reload()` 忙时置 `reloadQueued`，
  由 `flushSave` 的 `finally` 里 `drainQueuedReload()` 补做，**不许静默丢弃**。`watch(() => props.dir)` 里
  `commitEdit` 之后必须 `resetTimelineState()` 再 `loadData()`（否则跨片段串写）。删除保留**最近一次**
  （`lastDeleted`，15 秒）给头部「撤销删除」，事件按**分段对象本身**认位；只有**分段**删除保留确认。

---

## ② 精修预览（`PolishPreview.vue`）

`PolishPreview.vue` —— 精修预览（自绘 DOM/GSAP 层 + 可选真渲染）。props `pid`/`dir`/`config`/`subtitles`/
`danmaku`/`metaDuration`/`wgpuRender`/`selectMode`/`selectRect`/`selectLabel`/`pointMode`/`point`/`pointLabel`；
emits `time`/`playing`/`update:selectRect`/`update:point`。

- **预览与出片的一致性**是它的第一职责：自绘层与出片模板 `crates/vtrim-polish/templates/index.html`
  必须同口径 —— 覆盖层色档走 `engine.js::gradientCss`、字幕折行走 `engine.js::wrapCaptionParts`
  （经 `capRowsWrapped`）、贴纸尺寸走 `polish/canvas.js::stickerScale`、运镜/手持摇摆走 `engine.js`。
  改这些函数前先读 `webui/src/polish/AGENTS.md`（漂移对拍脚本在那儿）。
- **真渲染是可选能力**：`wgpuRender`（来自 `/api/health` 的 `wgpu_render` 能力位）为 false 时整个「真渲染」入口隐藏。
  打开后 **wasm 实时模式优先**（WebGPU 可用时渲染核心直接在浏览器合成），失败/不可用则
  **回退 server 单帧** `POST /api/polish/render-frame`（裸 `fetch`，写请求必须自己带 `csrfHeaders()`）；
  播放中不发单帧请求（`scheduleRealFrame` 里 `playing` 早退）。**默认构建下整条真渲染路不可达，
  别为它做语义同步**（见根卡「wgpu 真渲染：暂不使用」）。
- **字体要注册 `FontFace`**：页面 DOM 只认注册过的字体，所以经 `polish/fonts.js::ensureFontFace` 把同一份字节
  注册一遍（字幕字体在预览里注册；封面标题字体在 `CoverDesigner` 里注册）。
- 媒体地址一律走 `utils.js::clipMediaUrl(pid, dir, 相对路径)`，别自己拼 `/api/media/...`。
- 音量/静音 = **总音量，必须同时落到 `<video>` 元素**（`applyAudio()`：滑块 = 视频 `volume` + SFX masterVol，
  静音 = 视频 `muted` + SFX `setMuted`，读写走 `polish/audio.js`；**只调 SFX 播放器就是 bug**）；
  读回音量用 `isFinite` + clamp，**不能写 `parseFloat(...) || 1`**。垂直版背景 `<video>` 始终 `muted`。
- 预览尺寸随视口收缩：`.stage` 宽度 = `min(100%, calc(clamp(170px, 100vh - 540px, 620px) * 16/9))`；
  **别再写死 `66vh`**（那样只有预览自己能进首屏）。
- ⚠ **预览看着正常 ≠ 出片正常**：出片的事实来源是 `templates/index.html` 的 canvas 路径
  （`vtrim-render-core/src/timeline.rs` 是平行实现、不是事实来源）。任何"改完没反应"先怀疑没重建/没重启。

---

## ③ 封面设计器（`CoverDesigner.vue`）

`CoverDesigner.vue` —— 封面设计器，props `dir`（必填）/`pid`，emit `changed`；只在该片段被占用时禁用「生成封面」。

### 3.1 封面三层与背景模式

- **封面三层：文字 / 关键帧 / 背景**：`#title`（文字）在最上、`#frame`（**关键帧层**：当前帧清晰图 `_cover_frame.jpg`）
  居中、`#bg`（**背景层**）在最下，再往上才是渐变/暗角/特效。**三层各有各的图源，谁也不许覆盖谁**。
  背景层三种模式（`[cover] bg_mode`）：`color` 纯色（默认黑）/ `blur` 关键帧**模糊副本**平铺（`blur_scale` 0.2~4）/
  `image` 导入的自定义图片 `_cover_bg.jpg`；各自读各自的文件（blur → `COVER_FRAME_IMG`，image → `COVER_BG_IMG`），
  `image` 模式**没有那个文件就退回 color**（别在模板里挂死图）。设计器分组按三层排：
  「标题与高亮」（文字）·「关键帧」·「背景与阴影」；「导入背景图片」按钮**在背景组里**。
- ⚠️ **导入背景图片写的是 `_cover_bg.jpg`**（`/api/polish/cover-background` 同时把 `bg_mode` 设成 `image`）。
  **别再写回 `_cover_frame.jpg`** —— 那是"背景替换关键帧"的老 bug（用户报过「又把关键帧那一层给我删了」）。
  `cover.image` 也**不要**在导入时清空（它是关键帧的来源之一）。
- ⚠️ **出片不再删 `_cover_frame.jpg`**（`cmd_cover` 末尾那句 `remove_file` 已删）：它就是关键帧层的图源，
  删了等于①用户导出的那一帧白导、②下次出片又按 `frame_pos` 重新截、同一份封面每次换脸。

### 3.2 关键帧手势与 transform 的单一来源

- **关键帧可拖动/滚轮缩放**（`.stage-gesture-layer`）：`coverOffsetX/Y`、`coverZoom` → payload `bg_x/bg_y/bg_zoom`
  （服务端存 1920 画布 px，预览按 `stageScale` 换算）→ 出片写进 `#frame img` 的 `transform`，**与背景层互不影响**；
  这几个是**活字段**。帧的 transform 只此一份：`cover.rs::cover_frame_vars`（原点取 `layout.liver` 的聚焦点，
  右半边基准 1.4 / 左半边 1.3，再乘 `bg_zoom` 夹到 0.2~3）；前端 `mediaStyle`+`GRID_CENTERS` 是镜像。
  模糊倍率同理：`cover_blur_css` ↔ `coverBlurCss`。**改一边要改另一边。**
- ⚠️ **关键帧文件可能不存在**（新工程没导出过、也没出片过）：设计器里模糊背景会先 404 ——
  图被 v-if 摘掉、看着像"没运行"。`onBgImageError()` 的补法：**只对 blur 模式**（image 模式缺图只提示"还没导入"）
  **不带 time** 调 `/api/polish/cover-frame`，服务端按 `[cover] frame_pos` 截，只补一次（`bgFrameTried` 防循环），
  再失败就在模式下面提示手动「导出关键帧」。换背景模式会重置这两个状态。

### 3.3 安全区线框（几何表）

- **安全区线框**：舞台（1920×1080）上叠 `.guide-layer`（`inset:0`、`z-index:7`、`pointer-events:none`），
  压在标题(z5)与手势层(z6)之上才看得出"标题有没有越线"。两条框：**16:9 安全区**（`.guide-169`）= `width:95%; height:95%`
  居中 → 舞台坐标 **1824×1026**；**4:3 裁切区**（`.guide-43`）= `height:100%` + `aspect-ratio:4/3` 居中 → **1440×1080**。
  ⚠️ **别写成 `inset:5%`**：CSS 的 inset 百分比是**横向按父宽、纵向按父高**解析的，
  看着像等比其实不是（这么写会得到 1728×972，连 16:9 都不是）。等比缩放只能写 width/height。
  两条框**故意互相交叠**，别为了"好看"改成嵌套。线是**白线 + 1px 深色外圈**（封面素材亮暗不定）；
  线宽与字号用**预览 px**（**不乘 `stageScale`**）。开关在舞台下方独立一行两个 chip，**各自独立**、默认都开，
  偏好落 `localStorage['vtrim.ui.coverGuides']`，**在 setup 里同步读**（放 computed / onMounted 会先画一帧展开态再切，闪一下）；
  **不写 `cover.toml`、不加后端接口**。
- ⚠️ **只画在设计器里**：`crates/vtrim-polish/templates/cover.html` 与 `cover.rs` 一个字都没改，
  所以线框**不可能**进入成片。将来若要往 CLI 那个独立预览页加，必须同时补 `html.export .guide-layer{display:none}`
  （与 `#toolbar` 同一机制），否则进成片。

几何表（舞台坐标，1920×1080）：

| 框 | 类 | CSS | 舞台坐标 |
|---|---|---|---|
| 16:9 安全区 | `.guide-169` | `width:95%; height:95%` 居中 | **1824×1026** |
| 4:3 裁切区 | `.guide-43` | `height:100%` + `aspect-ratio:4/3` 居中 | **1440×1080** |

### 3.4 标题首次自动排版

- **标题首次自动排版 = `fitTitleIntoSafeArea()` 塞进 4:3 裁切区**：触发只有两处（`loadCoverConfig` 发现磁盘上
  `title_lines` **为空**；`generateTitleByLlm` 拿到新标题），手动操作（改文字 / 拖滑杆 / 点预设 / 拖拽）**一律不触发**
  —— **别在 watch 里接它**。**已有封面一律不碰**（判据 = 磁盘上有没有 `title_lines`）；只动
  `font_size` / `text_x` / `text_y`（必要时一次 `text_width`），零后端改动。目标矩形 = 那条 `.guide-43` 线框
  （画布 `x ∈ [240, 1680]`、`y ∈ [0, 1080]`）再内缩 `12 + stroke/2` 画布 px。**量的是 `.title-text` 而不是
  `.title-wrap`**（后者是 62% 宽的容器）。顺序：先按 `min(tW/bW, tH/bH)` 比例缩字号**并迭代**（字号一变
  换行数会跳变），到下限 24 还偏高才放宽一档文字区，最后按各边溢出量平移；**告警只看最终状态**
  （`relaxed = !fits(b)`）。实在放不下会 `console.warn` + `.fit-hint` 琥珀提示，不静默。
- ⚠️ **默认对齐是 `left`**（`default_cover_text_align()`，2026-09 由 `center` 改来）：默认预设
  就叫「左文」、默认 `text_x = "35%"`，三处以前是"预设左文、配置居中"，自相矛盾。已有工程的 `polish.toml`
  里都显式写着 `text_align = "center"`，字段存在就不走默认，**不受影响**。改这个默认值**不要顺手改
  `cover.rs::cover_text_align` 的 `_ => ("center","center")` 兜底** —— 那个 `_` 同时兜着合法值 `"center"` 本身。
- **标题文本框固定 5 行高、不可拖拽缩放**（`rows="5"` + `.title-lines { line-height:1.5; resize:none }`）。
  别改回 `rows="2"`，也别加 `resize:vertical`；字段外观用设计基准 `.fld-input`，不要再加紧凑覆盖。
  出片与预览都按 `title_lines` 逐行渲染。

推导留档：左对齐 + `text_x=35%` / `text_width=62%` 时，文字左边缘落在 76.8px，**本就在 4:3 裁切区
（左边界 240px）之外**，所以上面的"按各边溢出量平移"对左对齐而言是第一次必然发生。

### 3.5 设计器预览 ⇄ 出片的镜像清单

- **封面特效 emoji（`[[cover.effects]]` kind = `emoji`）**：`text` 是贴的字符（默认 `🙂`，最多
  `COVER_EMOJI_MAX_CHARS`(12) 个字符），`x/y` 是**字符中心**（画布百分比，`translate(-50%,-50%)`），
  `amount` 当大小用（`COVER_EMOJI_MIN_PX`(48) ~ `COVER_EMOJI_MAX_PX`(288)px，公式只在 `cover.rs::cover_fx_html`
  里一份，前端 `fxStyle` 是镜像并按 `stageScale` 缩放）。
  ⚠️ **`text` 是用户输入，进模板前必须 `escape_html`**（不转义＝往封面 HTML 里注模板，有单测）。
  加特效时组件默认把 x/y 设成 `layout.liver` 的聚焦点（`liverOrigin()`）—— 那个点是帧层 transform 的**不动点**，
  所以缩放/平移帧都不会把小表情带跑。字体栈要带 `Segoe UI Emoji / Apple Color Emoji / Noto Color Emoji`。
- ⚠️ **设计器预览 ⇄ 出片的镜像清单**（用户报过"设计器和实际渲染的图片不一致"；下列每一项
  都曾经各写各的，出片是事实来源，预览照着抄）：**字体**（两边都是
  `'LXGW WenKai','Noto Sans SC','Microsoft YaHei',sans-serif`）· 文字描边色 `#2a2522`、阴影
  `0 5px 22px rgba(0,0,0,0.78)`（预览按 `stageScale` 缩放）· **渐变底色**取自
  `[style.subtitle] highlight_color`（那套 `(1-c)×{30,25,50}` + min 10/10/15）· **图层序**（自下而上）：
  背景 < 帧 < 暗角 < 渐变 < 特效 < 标题（预览 `.vignette/.gradient/.fx` 按 2/3/4 给 z-index）·
  特效公式（`blur` = `1 + amount×14` px、`overlay` = `rgba(hex→rgb, amount)` + `mix-blend-mode`；
  emoji 字号 `48+amount×240`、`drop-shadow(0 6px 14px)`，预览一律乘 `stageScale`）·
  **`liverOrigin()` 的优先序必须与 `region_origin` 一致**：自由选框中心 > `origin_grid` > `origin` > `grid[0]` > `65% 50%` ·
  `bg_zoom` 夹取范围四处统一 **0.5~3** · 没有 `title_lines` 时按标点拆 `project.name` 并砍掉 `【主播名】`（预览也要拆）。

### 3.6 设计器的"素材左右黑边"根因

- · ⚠️ **设计器的"素材左右黑边"**（用户报过"设计器里素材左右两条黑边、出片正常"）：`contain`
  让出的留白区由**元素自身背景**填充，而 `styles/app.css` 有一条**全局元素选择器** `video { … background:#000; border-radius:var(--r-sm) }`。
  `.stage-video` 是类选择器、**只覆盖自己写过的属性**，这两条会**穿透**进来，还压在 `.stage-bg`（z-index:0 < `.stage-media` 的 1）
  之上把模糊背景层整个盖住 —— 竖屏素材于是在设计器里是**两条纯黑边**。因此 `.stage-video` 的
  `background:transparent; border-radius:0` 是**必需的覆盖、不是装饰**；根因与几何都钉在
  `node webui/scripts/check-cover-stage.mjs`（含 6 处变异自证）。

### 3.7 三处同步与写入门闸

- **三处必须同步**：`CoverConfig`（`vtrim-polish/src/project/config.rs`）、`/api/polish/cover-config` 的白名单 patch
  （漏键＝"点了没反应"）、`templates/cover.html`。公式与净化只在 `cover.rs` 一份：`cover_blur_css(scale)`、
  `sanitize_hex_color`（**颜色直接进 HTML/CSS，必须净化**，非法值退 `#000000`）、`cover_bg_mode`
  （只有 `blur` 走模糊，其余一律 `color`）。`CoverDesigner` 的 `configLoaded` 是**写入门闸**（读失败时拿默认值
  写回会把磁盘配置清成默认），读失败必须出错误条 + 「重试」，保存失败用常驻 `.save-err`。

---

## ④ 运镜（camera）

**三个能力**：**聚焦**（`anchor` = liver/danmaku/sticker/center）、**位移**（再写 `anchor_to` 即从 origin 平移到
to_origin）、**画中画**（`zoom="远"`），外加**附加变种：手持摇摆**（`handheld`）。
**锚点是「意图」、坐标是「结果」**：`process` Step 2d 的 `resolve_camera_anchors` 把 `anchor`/`anchor_to`
解析成 `origin`/`to_origin`，所以改了选框/挪了贴纸重跑 process 就生效，不用手改坐标；渲染侧四条路径都只认百分比坐标。
排位在贴纸落点(2b)与时长自适应(2c)之后 —— `anchor="sticker"` 要读最终位置。

### 4.1 倍率只有一张表

- **倍率只有一张表**：`CameraStyle::factor`（`vtrim-render-core/src/types.rs`），模板 canvas / 模板 GSAP /
  `engine.js` / `timeline.rs` 四处都走它。⚠ 模板 canvas 路径曾硬编码 `1.8/1.5` 且完全不读 `[style.camera]`，而它才是出片路径
  （`vtrim-polish/Cargo.toml`：「chrome-render 为默认出片路径」）——别再把倍率写回去。
- **三档倍率的当前值：中 1.3 / 近 1.5 / 画中画 0.55**。**放大档位不存在「推出框露背景」的风险**（运镜是绕不动点
  做 `scale`，`s ≥ 1` 时四周一律向外扩张；露底只发生在画中画 `zoom_wide < 1`，刻意留黑边
  `(1-factor)/2×1920 = 432px`），别拿"怕露出背景底色"当理由往下压。**旧内建默认按值迁移**：
  `process/mod.rs::migrate_camera_zoom` 在 Step 0a 把中 `1.15 → 1.3`、近 `1.3 → 1.5`、远 `0.7 → 0.55`
  （**只认这几个旧默认值**，手改过的不动）；三处兜底默认值（`default_zoom_mid/close/wide()`、模板 `camFactor`、
  `engine.js` `camFactor`）必须一起改。

倍率表留档：

| 档位 | 倍率 | 备注 |
|---|---|---|
| 中 | **1.3** | 整段无运镜时的默认补档 |
| 近 | **1.5** | 手持摇摆只在这一档可用 |
| 画中画（远） | **0.55** | `(1-0.55)/2×1920 = 432px` 刻意黑边 |

### 4.2 硬切与手持摇摆

- **画中画的判据是 `|zoom-1| > 0.001`，不是 `zoom > 1.001`**（`templates/index.html` 与 `PolishPreview.vue` 两处）。
  写成后者会把「远」档整档吞掉，画面完全不动。
- **远景进出是硬切，不是缓动**：过渡的一端只要是「远」，「zoom」与不动点就在事件起点**一次性到位**，
  那一段跨度 (`end - time`) 不产生任何缓动、位移也不生效。三处求值器同步实现（模板 canvas 让 `p = 1.0`、
  GSAP 用 `tl.set` 取代 `tl.to`、`engine.js` 同 canvas）；贴纸避让侧走 `placement::transition_secs()` →
  `CameraShot.transition = 0` —— 别写回常量 0.6。**硬切 `cut`**= 同一条规则放开给任意景别：
  `Event::Camera` 的 `cut: bool`（serde `skip_serializing_if = "is_false"`，老配置零迁移），
  编辑器属性面板第一行「方式 = 平滑运镜 / 硬切」；想「切过去再推」就写两条事件，别在求值器里搞两段式。
  泳道块标签与模板编辑器的 `labelFor` 都加「切·」前缀。
- **手持摇摆 `handheld`**：**只在 `zoom = "近"` 可用、情绪不限、可选能力不自动补**；编辑器运镜面板的
  「手持摇摆」开关（**非近景置灰**，景别改离近景时顺带剥掉该字段）+ `labelFor` 的「摇·」前缀。
  **门槛守卫**：`process/fx.rs::cap_handheld_sway` 把非近景上的 `handheld` 剥掉（只剥不补、幂等、日志报条数），
  排在画中画收工**之后**。**别再退回"靠 prompt/界面拦"**：守卫是唯一兜底。
- ⚠ **竖屏（`vertical = true`）跟运镜一起屏蔽**：四条求值路径全部归零
  （`timeline::evaluate_frame`、模板 `getActiveCamera` 返回 `CFG.vertical ? 0 : handheld`、
  `engine.js cameraAt` 竖屏分支返回 `handheld: 0`；DOM 预览层共用 `window.__CAM_STATE` 自动跟随）。

### 4.3 画中画配额、对齐、收工

- **画中画配额：一个切片只有一条**：`process/fx.rs::cap_pip_shots` —— 按时间序只保留**最早**的一条「远」，
  其余降级成「中」（**只改 zoom**，anchor/origin 留着）。调用点两处：`cmd_process` Step 1c 与 LLM 落盘前
  （`llm/mod.rs` Step 4b）。**画中画要对齐字幕/事件边界**：`align_pip_shots(subtitles)` —— 起点吸到**包含它的
  那条字幕的开始**（落在空隙里就吸到下一条），终点吸到**那条字幕的结束**；结果夹在所属分段 `[start, end]` 内；
  对齐后不足 `MIN_PIP_SECS`(1.5s) 就往后借下一条字幕的结束。只动 `time`/`end`。**画中画到点收工**：
  `close_pip_shots` 在画中画终点补一条**回镜头**事件：档位/不动点取**时间上**前一条非画中画运镜的**落点**，
  持到下一条运镜；下一条在 `MIN_RETURN_SECS`(0.5s) 内就来则不补。补的是**真事件**不是渲染侧特例，幂等。
  回镜头本就是硬切（「远」进出硬切那条规则管）。幂等。⚠️ **前后镜头按时间算，不按文件顺序**：
  段之间时间上会重叠（LLM 按窗口生成 + 去静音切段），求值器也是先按时间排序再走的。

### 4.4 位移

- **位移用 zoom 的同一段窗口与曲线**（`min((end-time)*0.5, 0.6)`、`pow2Out`），不新增旋钮；不写 `anchor_to`
  时两端相等，退化成「原地聚焦」。「主播→弹幕→回主播」= 两条事件，不是一条。贴纸避让跟着位移走：
  `CameraShot.origin_to` + `camera_states_for` 推**两端**，且「上一状态」取上一段的**落点**。

### 4.5 默认运镜与老工程迁移

- `process` 给每个 ≥1s 且没有运镜的段补 `zoom="中", anchor="liver"`（整段）—— 所以运镜是**全程开着**的；
  改默认档位/默认锚点就在这一处（`process/mod.rs` Step 1 与 `llm/mod.rs` Step 4 是两个重复实现，要一起改）。
  **老工程迁移**：`origin = "50% 50%"` 且无锚点的事件补成 `anchor="liver"`（有日志计条数）。
  只读诊断：`VT_POLISH_DIR=<项目目录> cargo test -p vtrim-polish --lib diagnose_camera_anchors -- --ignored --nocapture`；
  配额/对齐/收工 + 去静音阈值 + 贴纸对齐实测：`diagnose_pip_and_silence`（加 `VT_CAM_JSON=1` 多打一行 `CAM_JSON={...}`）。

---

## ⑤ 贴纸落点（Step 2b）与区域编辑

- **贴纸落点推导**（`crates/vtrim-polish/src/process/placement.rs`）：帧坐标固定 1920×1080，贴纸固定 280×280，
  `ev.x/ev.y` 是**盒子左上角**百分比（模板再钳制到 `[32,1608]×[32,768]`）。障碍 = ① 人物 `layout.liver`、
  ② 弹幕区域 `layout.danmaku_area`（都在**源帧**坐标里框选，出片时视频整体被运镜缩放 → 先按该贴纸可见窗口的
  相机状态投到输出帧，zoom 取模板 Canvas 路径的 1.5/1.8 × 瞬时特效预留 1.10）、③ 字幕带（两条路径取并集）、
  ④ 真实弹幕带（只有 `data/danmaku.json` 非空时才是障碍）、⑤ 同刻已落位的其他贴纸。**落点规则**：候选 = 16px 网格 +
  各障碍边缘的「临界位」；先取 clearance 最高的一档（×0.9），在该档里按「盒心到帧心距离」排序取前 8 个，
  再用 seed 伪随机挑一个 —— 即**安全区中心而不是边缘**。没有安全位时退化为「重叠代价最小」并告警。
  **`layout.sticker_area` 已降级**：`grid` 完全不参与落点；手写的自由选框只作**偏好**。
  只读诊断：`VT_PROBE_ROOT=<media 目录> cargo test -p vtrim-polish --lib diagnose_sticker_placement -- --ignored --nocapture`。
- **贴纸也要对齐最近的字幕（Step 2b1）**：`align_stickers_to_subtitles` —— 起点吸到**最近那条字幕**的开始、
  终点吸到它的结束（离最近字幕 > `STICKER_SNAP_MAX_SECS`(2.0s) 的不动）。**必须排在 2b 落点之前**。
  ⚠️ **手工放的事件不受后处理改写**（用户报过「调整贴纸的结束时间没有成功」）：事件带
  `manual: bool`（serde `skip_serializing_if = "is_false"`，老工程零迁移）时，`align_stickers_to_subtitles` /
  Step 2c 尾帧延长 / `align_pip_shots` 三处**全部跳过**，**`cap_events` 的数量抽稀也跳过**；
  编辑器改时间/新建/复制自动 `manual = true`，面板「跟随字幕」清标记。
- **区域编辑取代九宫格 3×3 选择器**：在「生成与导出」卡的 LLM 剪辑参数块里，只有「人物」「弹幕区域」两个按钮
  （同一个再点=退出，点另一个=换区域；无区域下拉、无贴纸区域）→ `PolishPreview` 进入**选框模式**
  （视频运镜暂停、缩放/滤镜复位；盒外压暗；拖框体平移、拖 8 个角/边缩放、空白拖拽重画，坐标为 1920×1080 整帧百分比），
  读数 / 保存选框 / 清除选框都在右侧同一块内，预览卡头显示「选框模式 · 运镜已暂停」chip。
  **别把这些控件放到页头**（页头离预览很远，用户实测反馈"按钮不见了"），也别只放预览卡头让参数与动作分家。
  交互纪律：`onSelectRect` 只回写 `selRect`（`{x,y,w,h}` 整帧百分比），保存/清除才落盘；`selectRegion` 与
  贴纸「固定落点」草稿态**互斥**（切进固定落点要清 `selectRegion`），换片段 `watch(loadedClip)` 清草稿。
  保存写 `polish.toml` 的 `layout[区域].top_left/bottom_right`（`"55.00%"` 百分比字符串），清除即删除两个角
  回到原 `grid`（grid 永不被框选改写 → 旧工程零迁移）。**保存 / 清除选框后自动重跑一次 `process`**
  （`reprocessLayout()`，与时间轴保存同一条路）。这两个框是 **process 的输入**、不是渲染的输入：Step 2b 按
  `layout.liver`/`layout.danmaku_area` 反选安全区并**重写每个贴纸的 `x/y`**，Step 2d 把 `anchor` 解析成
  `origin`/`to_origin`（**每次运行都覆盖**），所以改了框不重跑 process，出片/预览还是旧焦点。
- ⚠ **别再写回"保存不触发任务 / 人物框只影响运镜聚焦中心（渲染时读取）/ 弹幕区域仅作参考"**——
  那是加锚点解析（Step 2d）之前的旧说法：人物框现在同时喂 Step 2b 与 2d，弹幕区域也既当贴纸障碍、
  又是 `anchor="danmaku"` 在那一刻没有真实弹幕时的兜底。
- 读取优先级：`free_rect()` → `region_rects()`/`region_origin()` 一律**自由选框 > grid > `origin_grid`/`origin`**；
  JS 侧同一规则在 `webui/src/polish/engine.js` 的 `regionRect/regionOrigin`（`GRID_RECTS` 表须与 Rust `grid_rect` 对齐）
  与 `templates/index.html` 的 `liverOrigin`。**固定落点的 UI 是"草稿态"**：点选只写 `draftPos`，必须按「保存落点」
  才落盘（「取消」丢弃）；切到「固定」自动进编辑态、切回「自动」丢草稿；**保存成功才退出编辑态**。
  预览侧用 `indexConfig` 只顶替 `_canvas.sticker.position`，不动 `position_mode`。
- **贴纸来源 UI = 一组可点 chip**（全局可用来源 + 已配置但当前库里没有的来源）；**亮 = 启用、灰 = 停用**，
  点击即保存 `sticker_source`。不要再加"已选列表 + 输入框 + 添加"的第二套控件。

---

## ⑥ 其他组件逐个

- `ProjectClipsSection`：切片页卡片墙（已剪片段 + 未剪切事件混排、两级筛选、热度排序）。
  **切片↔事件对应（`clipMatchesEvent`）、`:clip` 用目录名、排序回落到 `SORT_OPTIONS[0]` 已在 `webui/src/AGENTS.md`**，不重复。
  ⚠️ **删除不是任务（不发 job），刷新只能由前端自己驱动**：`ProjectClipsSection` 的数据是父页面
  （`ProjectSlicesView`）传下来的**快照**，删完不 `emit('changed')` 让父页面 `load({silent:true})` 重新取数，
  卡片就留在墙上，用户再点一次就是 404。同一条规矩适用于改事件（`saveEdit`）与新建片段（`addClip`）。
  `silent` 必须真的静默；删除中按钮禁用（`deletingDirs`）防连点；404 当「已经不在了」提示并刷新。
  **「剪切」按钮不禁用，忙时只改文案**（`cut_one` 必持 `p{pid}:out` + `p{pid}:idx`，服务端遇到占用本来就排队）：
  忙时文案变「剪切（排队）」+ title 说明谁在占；`编辑` 仍然禁用（同步接口，撞占用是 409）；`预览` 只看有没有原片地址。
  **这条是用户拍板的，别改回禁用。**
- `HomeClipsOverview`：首页切片总览（未导出/已完成两区块）；点卡片去精修页，没初始化就自动 `polish init`
  （`initPolish(pid, dir)` 传**绝对目录**，scope 守卫按它反查归属）—— 细节见 `webui/src/AGENTS.md`。
- `SubtitlePanel` / `SubtitleEditor`：字幕列表（props `segments`/`height`/`showEnd`/`editableTime`，emit `play`；
  点击时间可改 `start`/`end`）；`SubtitleEditor` 是「添加字幕 + 保存字幕」外壳，`addSub()` 后按 `start` 排序。
- `prepare/SubtitleTrack`：预处理页字幕轨 + **常驻横排**「字幕属性」条（内容 / 起止秒 / 当前设为入点·出点 /
  拆分 / 定位 / 删除），未选中时字段禁用 + 一句提示（用户要求：不要右侧竖面板）。
- `prepare/MaterialList`：弹幕/字幕共用清单，props `items/kind/busy/emptyText`、emits `pick/activate/remove`
  —— **改一边等于改两边**（ASR 产物不可删、`exists=false` 照实报，见 `webui/src/AGENTS.md`）。
- `StickerPacks`：首页表情包区块（**仅 admin**）：上传 zip / LLM 压缩构建 / 卡片墙 + 大图预览。
  库与 zip 安全细节指向 `crates/vtrim-polish/src/assets/AGENTS.md`。⚠️ **这是全站第二处有意接收用户上传的入口**
  （第一处是封面背景图 base64）；「服务器文件建项目」那条仍然**永不接受上传**。别按旧规矩把它删掉 —— 它是有意做的。
  ⚠️ 前端 `webui/src/api/stickers.js::UPLOAD_MAX_BYTES` 与 Rust `UPLOAD_MAX_BYTES`
  是同一个数（改一处要改两处），前端必须先挡。**`preview` 只存 `{pack, name}`**，文件对象由 `previewFile`
  从 `detail` 现取（别存快照）。
- `polish/StickerPicker`：贴纸两级列表（keyword 一级 → 缩略图二级）；选中写 `keyword/sticker_tag/name/source`，
  **`name` 必须是去扩展名的文件名**（后处理与出片按 stem 匹配）；数据来自**只读**接口
  `GET /api/polish/sticker-library`（写操作只在 admin 侧，别把库管理接口开到 `/polish/*`）。
- `polish/SfxPicker`：音效选择器，数据来自 `GET /api/polish/sfx-catalog`（已按全局 `[sfx] blacklist` 过滤），
  选中 emit `pick(id)`，空串 = 清空。
- `JobList` / `jobs/JobDrawer`：`JobList` 把「读取中 / 读取失败（带重试按钮）/ 真的没有任务」**分三态**显示；
  排队行用 `queueDetail` 报「档位 · 同档第 N 位」，**细节写在说明行、不塞进 chip**（chip 全应用共用）；
  `stuckNote` 报 `IDLE_WARN_SECS`(120s) 无新事件。抽屉打开时 `reload()`（轮询可能正在失败退避，抽屉是唯一诊断入口）。
- `Steps`：4 段进度点（props `stage`/`size`）。`Icon`：Lucide 风格 24×24 stroke 图标表，`name` 是内部常量。
- `SliderField`：⚠ **设置页的数值项走 `components/SliderField.vue`**（滑块 + 标签行右端可直填的数字框，16 处）。
  手填的值只夹 `[min, max]`、**不吸附 step**（step 是滑块的语义，不是配置的语义），且**清空/非数字不写模型**
  （否则退格清空那一瞬就把 60 写成 0）；数字框在**标签行右端**（`.lbl-row` / `.rv`）。
  ⚠ **滑块宽度 = 所在格子的 1/3**（`.slf-range` 的 `flex:0 0 33.333%`，以前吃满整格；`input[type=range]` 的
  `width:100%` 会被 flex-basis 盖掉，别再改回 100%）；窄格（格子 ≤220px）由 `@container` 把标签挤到独占一行，
  **别改成媒体查询**（量的是格子宽而不是视口宽）。
- `LogConsole`：运行日志面板（级别档 / 关键字 / 自动滚动 / 清空 / 连接状态），**挂载连 SSE、卸载断**
  （`stores/log.js`，只在设置页 `monitor` 子页挂）；`.log-*` 是 scoped 新类，颜色一律走 tokens。
- `AppModal` / `AppDialog` / `ToastHost`：`AppModal` = Teleport 到 body 的弹窗外壳（`show`/`title`/`wide`/`closable`）；
  `AppDialog` 走 `stores/dialog` 的 `dialogState`/`settleDialog`（`confirmDialog` 入口，`mode:'prompt'` 带输入框）；
  `ToastHost` 渲染 `stores/toast` 的 `toastState`。**别再在各页手搓弹窗/确认**。
- `ServerDirPicker`：admin 服务器目录浏览（任意绝对路径 + 快捷入口 + 上一级）。**进入下一级用 `{dir, enter:"子目录名"}`，
  路径由服务端拼** —— 前端自己拼 `dir + '/' + name` 会把 canonicalize 的 `\\?\F:\a` 拼成 `\\?\F:\a/b`（Windows
  ERROR_INVALID_NAME 123）。`mode: 'dir'` 一步选中子目录，`mode: 'file'` 用 `exts` 把不相干的文件置灰标「不适用」。
- `RecorderConfig`：设置页「录播姬（采集）」卡：开关（`webhook_enabled`）+ `root`/`api_base`/`api_user`/`api_pass`/
  `allow_ips`/`auto_pipeline` 表单 + 一键生成/轮换 token + 「测试连接」。开关关闭时**只留开关与一句说明**。
  `GET /api/admin/ingest` **只回 `api_pass_set` 布尔，绝不回 `api_pass` 明文或 `token_hash`**；
  `PUT` 语义：**键缺席 = 不改**、给空串/空数组 = 清空（`api_pass` 缺席即"不改密码"）；`allow_ips` 只收 IP 字面量
  （运行时是**精确字符串相等**，放行 `192.168.1.0/24` 只会让来源被静默拦掉）；`api_base` 非空必须带 scheme；
  `root` 不可达**不拒绝**、只回 `warnings`；审计只记**键名**。`POST /api/admin/ingest/token` 的明文**只在这一次响应里**回，
  `disable` 只关开关并保留 token。
- `DownloaderConfig`：设置页「B站下载器」卡：引擎/清晰度/限速/并发/cookie + 一键安装（显示 sha256 校验结果）。
- `InviteManager`：设置页「注册与邀请码」卡：开关 / 生成 / **列表里明文一直可见 + 一键复制** / 未使用·已使用 chip /
  绑给了谁 / 撤销；行 `flex-wrap` 保证窄窗口换行不撑破卡片。
- `ModelList`：模型管理表，显示**实际路径与来源**（`/api/models` 的 `dir`/`source_label`），
  否则会出现"界面说没下、其实在用"的错位；下载与任何会 `ensure_models` 的转写互斥（`busyOn` + `MODELS_KEY`，排队）。
- `LlmUsage`：LLM 用量卡（`GET /api/admin/usage`：总量 + 按天/链路/项目/模型分组）。
  **`cacheRate` 是后端 `usage::cache_rate` 的镜像**（= Σcached ÷ Σprompt，只算输入侧、先求和再相除；
  `prompt_tokens == 0` 显示 `—`，不是"没命中"）。
- `ingest/SessionRow`：入库会话行（房间号 / 状态徽标 / 时间 / 文件数与体积 / 自动管线 / 错误文案），
  有 `project_id` 给「打开项目」，否则 `failed` 给「重试」，其余动作走 `actions` 插槽。
- `side/SideNav` / `side/ProjectNode`：侧栏与项目节点 —— 当前项目高亮判据、`.src-badge` 与
  **`.proj-head .del` 也钉 `flex:none`**、阶段打勾口径等全在 `webui/src/AGENTS.md`，本卡不重复。

---

## 附：验证与门禁（正文）

组件没有单测运行器：**`npm --prefix webui run build` 就是本目录的门禁**（`vite build` 会解析模板与全部 import，
`<template>`/Teleport 这类模板级事故能被它挡下一部分）。动了与出片模板同口径的算法（覆盖层色档 / 字幕折行 /
贴纸锚点 / 手持摇摆 / 衬底 / 封面舞台）再加跑对应的 `node webui/scripts/check-*.mjs`（清单见
`webui/src/polish/AGENTS.md` 与 `webui/AGENTS.md`，退出码非 0 即漂移）。改布局后按几何断言
（`getBoundingClientRect()` 宽高 > 0）实测。

---

> 改动只允许落在 `webui/src/components`；跨模块改动先读对方卡，并跑两条门禁
> （`python3 scripts/check_module_map.py` 与 `python3 scripts/check_module_boundaries.py`）。
