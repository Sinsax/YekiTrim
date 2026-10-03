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
- ③ [封面设计器 `CoverDesigner.vue`](#③-封面设计器coverdesignervue)：[三层与背景模式](#31-封面三层与背景模式) · · [右栏左右排布](#310-右栏内部左设置-右列表左右排布--宽度减半2026-10)
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
`yekitrim-render-core/src/legacy.rs::upgrade_event_times` 在**反序列化前**换算（`end = time + duration`），
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

- **预览与出片的一致性**是它的第一职责：自绘层与出片模板 `crates/yekitrim-polish/templates/index.html`
  必须同口径 —— 覆盖层色档走 `engine.js::gradientCss`、字幕折行走 `engine.js::wrapCaptionParts`
  （经 `capRowsWrapped`）、贴纸尺寸走 `polish/canvas.js::stickerScale`、运镜/手持摇摆走 `engine.js`。
  改这些函数前先读 `webui/src/polish/AGENTS.md`（漂移对拍脚本在那儿）。
- ⚠️ **真渲染那条路已整条删除**（wasm 实时模式 + `POST /api/polish/render-frame` 单帧回退、
  `/api/health` 的 `wgpu_render` 能力位 —— 随「作废 wgpu」一起没了，GPU 那侧交给独立的 Dhampir 底座）。
  预览**只有自绘 DOM/GSAP 这一层**；`PolishPreview.vue` 也不再收 `wgpuRender` prop。**别再照本节旧写法加回来。**
- **字体要注册 `FontFace`**：页面 DOM 只认注册过的字体，所以经 `polish/fonts.js::ensureFontFace` 把同一份字节
  注册一遍（字幕字体在预览里注册；封面标题字体在 `CoverDesigner` 里注册）。
- 媒体地址一律走 `utils.js::clipMediaUrl(pid, dir, 相对路径)`，别自己拼 `/api/media/...`。
- 音量/静音 = **总音量，必须同时落到 `<video>` 元素**（`applyAudio()`：滑块 = 视频 `volume` + SFX masterVol，
  静音 = 视频 `muted` + SFX `setMuted`，读写走 `polish/audio.js`；**只调 SFX 播放器就是 bug**）；
  读回音量用 `isFinite` + clamp，**不能写 `parseFloat(...) || 1`**。垂直版背景 `<video>` 始终 `muted`。
- 预览尺寸随视口收缩：`.stage` 宽度 = `min(100%, calc(clamp(170px, 100vh - 540px, 620px) * 16/9))`；
  **别再写死 `66vh`**（那样只有预览自己能进首屏）。
- ⚠ **预览看着正常 ≠ 出片正常**：出片的事实来源是 `templates/index.html` 的 canvas 路径
  （`yekitrim-render-core/src/timeline.rs` 是平行实现、不是事实来源）。任何"改完没反应"先怀疑没重建/没重启。

### 2.1 跳手动屏蔽区间为什么要有"定格遮罩"（2026-09）

用户报的现象：**播到手动屏蔽区间时黑屏一下 + 极短卡顿，然后继续**。

机制：预览跳过 `mute_ranges` 的办法是给 `<video>` 改 `currentTime`（`tick()` 里那一跳）。
观测台 `.workbuddy/mute-skip/run.mjs`（CDP 录**合成器输出** + 页内逐帧轨迹；软件解码 / GPU 解码 ×
headless / 真窗口 **四种组合**；样本是单关键帧长 GOP 的 720p，配一个"远段延迟 600ms 才发"的
静态服务来模拟"seek 目标未缓冲"）实测：

| 观测项 | 实测值 |
|---|---|
| seek 期间元素 `readyState` | 掉到 **1**（HAVE_METADATA）→ **整段没有可呈现的帧** |
| seek 时长 | 缓冲好的 190ms；未缓冲的远跳 **707ms** |
| 四种组合里的黑帧 | **0 张** —— 本机合成器会**保持上一帧**（这四种组合都拍到了"画面停住"而不是变黑） |

所以：**黑屏那一档取决于解码路径/平台**（元素没有帧时，露出来的是它自己的黑底
`app.css` 全局 `video{background:#000}`），本机复现不出来、但**不能因此认为它不存在**；
而**与平台无关、必然发生**的是**错位** —— 画面停住等解码（最长 700ms），而改造前
`current`/字幕/弹幕/时间轴是**立刻**跳到剪切后的：那段时间里字已经换了、画面还是剪切前那一张，
看起来正是"卡一下"。

做法（`PolishPreview.vue`）：

* `beginSkipFreeze()`：**改 `currentTime` 之前**把"此刻屏幕上那块画面"快照进 `.freeze-frame`
  画布（`.stage` 内 `inset:0`）。几何一律照元素的 `getBoundingClientRect()` 走 —— `axis/position/
  scale/padding` 与运镜那套 transform 已经由 CSS 应用过一次，照包围盒画即可（手持摇摆那零点几度的
  rotate 不复现，一帧的误差看不出来）。Dhampir 模式直接照抄那块 canvas（它本来就铺满舞台）。
* `armFreezeRelease(wantFrame)`：等**新帧真的上屏**才撤 —— 首选 `requestVideoFrameCallback`（回调时新帧已交给
  合成器），没有它的浏览器退到 `seeked` + 两帧（`seeked` 只说明解码好了、还没上屏，直接撤会露出黑的那一帧）。
  ⚠️ **两条腿不一样**（2026-09-30）：DOM 模式等 `videoRef` 那一帧就够了（画面就是它自己）；
  **Dhampir 模式还要等"引擎已经画到目标帧"**（`freezeTargetDrawn`：`dhampirFrameShown ≥ dhampirFrameAt(target)`）
  —— 那块 canvas 是引擎用**它自己那套 `<video>` 源**画的，与 DOM 时钟**不是同一个解码器**：
  只等后者时遮罩会在源就绪前撤掉，引擎先画的那一张没有像素（画布露黑）—— 症状就是"跳屏蔽区还是黑一下"。
  DOM 模式传 `wantFrame = null`，判据恒真，行为与改造前逐字节相同。
* `endSkipFreeze()`：撤遮罩的那一帧**才推时钟**（`current`/`emit('time')`）—— 定格期间 `tick()`/`onSeeked()`
  都不推，否则就是"字先动、画后到"。
* 层级别弄反：**DOM 模式 `z-index:5`**（只冻视频像素，DOM 覆盖层 z-index 10 留在上面一起定格），
  **Dhampir 模式 60**（画面是那块 canvas，z-index 40，遮罩必须盖在它上面）。
  `.fx` 那五层（flash / color / hue / vignette / noise）是 **`z-index:6`** —— 快照里没有它们（遮罩只拍
  视频像素），不抬到 6 的话定格那 200~700ms 里它们会**整块消失再回来**，又是一次眨眼。
* 常量：`SKIP_FREEZE_MAX_MS=700`（防"永久定格"）、`SKIP_FREEZE_MIN_MS=30`（rvfc 可能在 seek 生效前回调）、
  `SKIP_LOOKAHEAD=0.03`（提前一帧认定要进区间，剪辑点才落在区间起点上 —— 否则区间里那一帧会先被显示）。
* **观测台的实测数字**（`.workbuddy/mute-skip/run.mjs`，2026-09-30 复跑：headless 与真窗口 GPU 解码各一趟）：
  seek 窗口 **340~370ms**（skip→seeked），窗口内亮度 119.1~119.3 —— **元素保持上一帧、0 张黑帧**；
  `readyState` 掉到 1、`seeking=1` 的 ~50 帧里，fix 档遮罩**全程都在**（baseline 0 帧）。
  也就是说：黑底这一档在**本机**复现不出来（平台/解码路径相关），而与平台无关的是**那 340ms 的定格** ——
  遮罩把它从"黑一下"变成"画面停一下再硬切"，**要去掉这 340ms 只能把 seek 藏到切割之前**（预滚双缓冲）。
* 用户机器上 GPU 解码路径是否真会露黑底仍未复现（本机四种组合都没有），处置仍是这一层遮罩。

### 2.2 A/B 预滚：把"停一下"也消掉（2026-09-30）

用户复报"**整块纯黑 + 画面停住**"。定位到是 **Dhampir 那条腿**（组件挂载 300ms 后默认进 Dhampir）：

* **黑从哪来**：`followPlayback = true` 时引擎**不 await seek**（`webui/web/engine.js`：
  `const following = this.followPlayback === true && video.paused !== true; if (following !== true) await this.seekVideo(...)`
  —— Dhampir 仓的 `web/engine.js:962`）。跳屏蔽区间时宿主把源 `<video>` 一把改 `currentTime`
  （`syncDhampirTime`），源进入 `seeking`、`readyState` 掉到 1 —— 而引擎这一帧照画：
  视频层没有像素，合成出来就是整块黑。
* **为什么遮罩救不了它**：那块 canvas 是 WebGPU 表面，**读不回来**（surface 只有 `RENDER_ATTACHMENT`，
  没有 `COPY_SRC` —— Dhampir 仓 `crates/dhampir-wasm/src/timeline_host.rs:399` 自己写着这件事），
  所以 `drawImage(canvas)` 拿不到像素、定格遮罩是**透明**的，黑画布照旧露出来。

做法（`PolishPreview.vue`）：

* **宿主闸门**：`dhampirSourcesReady(f)` —— 这一帧要用的源只要有一个在 `seeking` / `readyState < 2`，
  这一帧**不交给引擎**（`dhampirHoldSince` ≤ `DHAMPIR_SOURCE_WAIT_MS=700` 兜底，超了照画）。
  不画，画布会**保留上一帧**（交换链的既有行为）⇒ 从"整块黑"变成"停一下"。
* **A/B（真正消掉那一下）**：每个源多备一块同 `src` 的 `<video>`（`dhampirTwins`，1px、暂停、静音、
  `preload=metadata`），进屏蔽区前 `PREROLL_SECS=0.8` 把**下一段起点**在它上面 seek 好
  （`dhampirPrerollTarget` 往前看：`dhampirFrameAt(t + PREROLL_SECS) === null` = 马上要切）；
  到点 `dhampirSwapSources()` 用 **`engine.bindSource(assetId, twinId)` 换绑** + 让备用的播起来、
  旧的暂停并接下一次预滚（角色互换）⇒ 引擎那一帧**本来就有像素**，零等待。
  ⚠️ 只换"这一趟武装过的源"（`dhampirArmedIds`）：工程里不活跃的第二路素材没有目标位置，
  把它算进"全都就绪"会让换绑永远等不齐（A/B 静默失效）。
* **DOM 那条腿同样 A/B**：`.fit-box` 里放两块 `<video>`（`stage0Ref`/`stage1Ref`），
  台下那块是 `.source-video-standby`（1px、仍在渲染树里 ⇒ 照旧解码 —— `display:none` 会停解码）。
  **`videoRef` 改成了 `computed`**（`activeStage` 选中哪块就是哪块）：组件里几十处读 `videoRef.value`
  的地方一个字都没改，换台就是翻一个数。到点 `tryStageSwap()` 换台 ⇒ 零等待、连遮罩都不用上；
  没备好（刚拖过进度条、区间太近）才回落到定格遮罩那条老路。
* 两腿的兜底都在：只要预滚没赶上，行为与改造前一致（DOM 定格遮罩 / Dhampir 闸门 + 定格遮罩）。
* 守卫：`node webui/scripts/check-preview-ab.mjs`（17 项 + 3 处变异自证）—— 守"`videoRef` 必须是
  computed、换台先翻 `activeStage` 再 `pause`、Dhampir 必有那道闸门与备用源换绑"这三件**错了也不报错**
  的事。

还没验到的一档：**真机上的观感**（A/B 命中时那一下是不是真的"无缝"）。观测台只覆盖了 DOM 那条腿的
seek 行为，Dhampir 那条腿要真浏览器 + WebGPU 才跑得起来 —— 请在精修页里把预览切到 Dhampir
（默认就是）播到一段屏蔽区间上看。

### 2.3 预览的字体是"一套口径"，出片必须照抄（2026-10-01，"字体不对"）

用户实测报"字体不对，其他的正确"。查下来**不是选错字体文件**，而是预览与出片在**两件**事上不同口径：

| 项 | 预览（本组件） | 出片（底座那条腿） | 结论 |
|---|---|---|---|
| 族名顺序 | `.cap` 的 `fontFamily = "<项目字体/缺省 LXGW WenKai>","Microsoft YaHei","Noto Sans SC",sans-serif` | 读 `render/server.rs::SYSTEM_FONT_STACK`，而它原本是 `LXGW → **Noto** → YaHei` | 顺序**相反** ⇒ 没装项目字体的工程：**预览雅黑、成片 Noto**（两者都装了），且不报错 |
| 字重 | `.cap{font-weight:700}`、`.dm{font-weight:600}`（+ `-webkit-text-stroke`） | 转译器写了 `font_weight: 700/600`，但底座**不合成粗体**：它只在 `>=600` 时去要 `--font-bold-file`，而这条参数**从来没传过** | 成片退回常规体 ⇒ **笔画比预览细一圈** |

修法与判据：

* `SYSTEM_FONT_STACK` 改成与 `.cap` **同序**（`'LXGW WenKai','Microsoft YaHei','Noto Sans SC',sans-serif`）
  —— 它同时喂 index.html / cover.html 与底座那条腿，一处改三处对齐；
* 底座侧 `find_font` 返回 `(常规, Option<粗体>)`（`msyh.ttc` ↔ `msyhbd.ttc`）并传 `--font-bold-file`；
  就绪位/`polish doctor` 把"有没有粗体那一支"露出来（没有就是细一圈的直接原因）。
* 本组件**不许改 `.cap` 的顺序/权重而不改 `SYSTEM_FONT_STACK`**（反之亦然）。
* 实测证据与完整推导在 `docs/plan/dhampir-parity-register.md`「2026-10-01 追加」一节
  （A/B 对照图 `.workbuddy/font-compare/bold-ab.png`：同一帧，给不给粗体文件）。


### 2.4 动图贴纸：由底座自己解码（host_api v6，2026-10-02）

GIF / 动画 WebP 贴纸**不再逐帧吐位图**：宿主把整段字节交给引擎（`dhampirLoadAnimations()`），
之后波放期**零请求、零宿主操作**。改造前是每换一帧就 `POST /polish/sticker-frame`
取一张 base64 PNG（~390KB/帧）—— 整包实测 **46.2 MiB → 8.4 MiB**（原文件 8.4 MiB，4.1× 膨胀消失）。

#### 四条硬规矩

1. **顺序必须 `open()` → `attach()` → load** —— 底座把**纹理上传挂在 `attach` 上**：
   host 还没接上时，它**只记账时间真值**、不上传纹理，并返回
   `{parsed:true, ok:false, error:"还没 attach 上 canvas：动图现在只能先记账"}`。
   也就是说**字节白传了、贴纸一张都不出**。⚠️ 它是个**正常的** `ok:false` 返回体，
   没异常、没堆栈 —— 两端都不报错，症状只是"动图不动"，与"格式不支持"长得一模一样。
   ⚠ `info`（`frame_count` / `frame_delays_ms` / …）**只在 `ok:true` 时给出**，所以 attach 之前也拿不到延迟表。
   ⚠️ `open()` 会 `clear_animations()`（换工程 = 换一批素材），所以 load 也不能排在 `open` 之前。
   钉子：`scripts/check-dhampir-wiring.mjs` 断言 `startDhampir` 里三者的**下标顺序**。
2. **动图绝不能再调 `setSourceBitmap`** —— 底座把动图缓存排在 `texture_for` **最前**，
   宿主再交位图就是两边抢 —— 症状是贴纸**永远钉在宿主给的那一帧**。
   帧循环里的判据就是 `if (st.animLoaded) continue`。
3. **静态图仍走老路**（宿主逐帧位图 + `sticker-prewarm` 预热）：底座的动图这条路
   **只收动图**，单帧图它明确拒收（`AnimError::NotAnimation`）。所以两条路并存，**不能互相替代**。
4. **那两条导出不在引擎包装对象上** —— `loadEngine()` 返回的是 `new Engine(mod)`，
   底座 `web/engine.js` 还没包它们。要拿**模块命名空间**（本仓存为 `dhampirPkgNs`，`stopDhampir` 里清）：
   ESM 缓存保证与 `loadEngine` 内部同一份，不重复下载。

#### 时间真值归一

**解码器读出的 `frame_delays_ms` 是事实**（存进 `st.delays`），工程 JSON 里那份退为诊断。
底座实测：clip-18 四张动图全是均匀 30 ms，**与 YekiTrim 探测的那份逐值相同**。
⚠ 不一致时必须报出来——否则「映射用一张表、像素用另一张表」会把漂移请回来
（症状是动图越播越偏）。**当前只做了"存下来"，冲突警告待补。**

⚠ **不要自己算帧号**：`source_frame` 是**求值层直出的最终值**（`layer.rs::source_frame_at_delays`，
逐帧延迟累加 + 循环取余 + `source_in` 为圈入口）。`seconds` 只是给 `<video>` seek 用的驱动字段，
**不能拿它去换算任何东西**（底座 §8.2-1 明确更正过提案里的"从秒映射"写法）。

#### 失败形态

加载失败**不改路由**：没解开的资产照旧走宿主位图那条路（也就是"这一层画不出来"），
而不是把工程搞坏。⚠ 必须在**加载期**就接住：渲染期没有 issue 通道 ——
`resolver.texture_for` 返回 `None` 的层是被**静默跳过**的（底座 `render/timeline.rs`），指望 `draw` 报错是等不到的。

#### ⚠ 排查第一课：报"版本不匹配"先量 wasm 字节数

预览报 `底座 wasm API 版本不匹配：期望 6，实得 5` 时，**先别怀疑源码** ——
本仓 `wasm_dirs()` 里 `preview_dir()` 是**定版产物优先**（`<程序目录>/dhampir/preview`），
那一份会把底座源码树里**当前**的 wasm 整个遮住。实测撞过：`target/debug/dhampir/preview/pkg/`
躺着旧的 **1.18 MB（v5）**，而底座源码树里是 **8.04 MB（v6）**，症状就是这句版本不匹配。
刷新：把 `../Dhampir/crates/dhampir-wasm/www/pkg/*` 覆盖过去（打包时 `xtask` 会做这件事）。
**判据是字节数**：两边量一下就知道是不是过期产物，比读代码快得多。

⚠ **"还没 attach" 不是失败，是时机不对**：必须与"真解不开"分开（判据 = 错误原文里的 `attach`），
混在一起会让动图**永远不重试**。本仓做法：标 `st.animPendingAttach` 并 `continue`，attach 之后那次调用会重试。

#### 参考

- 计划与执行记录：`docs/plan/dhampir-gif-landing-plan.md`（§1.3b 调用顺序契约 / §7 执行记录）
- 底座方案与实测：`../Dhampir/plan/dhampir-gif-native-decode-proposal.md`（§8 复核 / §10 执行）

### 2.5 Dhampir 加载门帘：加载期间不许播（2026-10-02）

**症状**：进 Dhampir（含刷新后自动进）时引擎还没起来，用户按播放能播 —— 看到的是**缺效果**
（贴纸 / 动图 / 字幕还没喂进去）甚至空白的画面；**加载失败**时更容易被当成「这一版就是这样」，
而它其实只是未就绪 / 近似状态。

**根因**：`toggleDhampir()` 在 `await startDhampir()` **之前**就把 `dhampirMode` 翻成真，而三件事
都是异步的：① `loadEngine` + `open/attach/resize/bindSource` ② `dhampirLoadAnimations`（贴纸 GIF/WebP）
③ `dhampirApplySubtitles`。也就是「模式已开」≠「效果已就绪」，而播放按钮只看前者。

**做法（三道闸，缺一条都漏）**：

| 闸 | 位置 | 作用 |
|---|---|---|
| `dhampirLoading` 门帘 | 模板 `.dhampir-loading` | 说清在等什么；`v-if` 必须带 **`&& dhampirMode`** —— 加载卡住时用户仍能再点一次「Dhampir」退出模式，门帘要跟着消失 |
| 播放按钮置灰 | `:disabled="dhampirLoading"` | 一开始就看得见「现在不能播」 |
| `togglePlay` 首行 `if (dhampirLoading.value) return` | 函数体 | `togglePlay` 经 `defineExpose` 给父级（PolishView 快捷键 / 时间轴 `toggle`）—— **只禁按钮挡不住它们** |

**只在首次加载挂，热重载（`reloadDhampirProject`）不挂**：热重载时引擎已经起来、画面仍是上一版
（失败也只 warn 并保留上一份工程），挂门帘会变成「每次编辑闪一下」（它是 300ms 防抖后自动跑的）。

**门帘样式三条**：`.dhampir-loading` = `position:absolute; inset:0; z-index:70`（压过定格遮罩 60）；
`pointer-events:none`（它只负责告知，挡了框选 / 点选那两层得不偿失）；**颜色写死**（`rgba(0,0,0,.62)` + `#fff`）
—— 它盖在视频上，必须与主题无关，与 `.stage{background:#000}` / `.dhampir-canvas{background:#000}` 同一条口径。
---

### ②.6 预览的**声音**（2026-10-02 修：Dhampir 默认开启之后"完全没声音"）

**症状**：精修页一进来就**一点声音都没有**（说话声、音乐、音效全无），而**画面完全正常**、
控制台不报错、服务端日志全 200 —— 唯一的异常是**音量/静音控件看着是接线的、拖了没反应**。

**根因**：`55f2f16` 同时做了两件事 —— **新增 Dhampir 模式** + **让它默认开启**
（`onMounted` 里 300ms 后自动 `toggleDhampir()`）。而声音那条链上有三处仍按"Dhampir 是**可选**模式、
与 DOM 预览并列、预览不带声音"来写，于是三条一起断：

| # | 位置 | 原来的写法 | 为什么是错的 |
|---|---|---|---|
| ① | `startDhampir` 建源 | `for (const v of dhampirVideos) { v.muted = true; … }` | 那批 `<video>` **同时是引擎的帧来源与声源**（`dhampirSourceVideos` 与 `dhampirVideos` 是**同一批元素**）⇒ 说话声/音乐全没了 |
| ② | `tick()` 的 Dhampir 分支 | 在 `player.sync(...)` **之前** `return` | 那里"让出重活"让的是 DOM/CSS **渲染**（字幕/弹幕/贴纸被 canvas 盖住，继续跑只是白烧 CPU），而 **SFX 是出声** —— 两件事被混成一件 |
| ③ | `applyAudio()` | 只写 `videoRef.value` | `videoRef` 是**时钟/DOM 预览那一路**；Dhampir 那批源是**另一批**元素 ⇒ 控件对真正在响的那路完全打空 |

**修法**（四条，缺一条都会留下一个"听不见"或"听着不对"的口子）：

1. **源按用户声明的状态出声**：`startDhampir` 里不再写死 `muted`，只调 `dhampirPreservePitch(v)` +
   `v.play()`，末尾 `applyAudio()` 铺一次。⚠️ 底座的**画面不看 `muted`**（跟播条件只有 `paused`，
   它只取"正在显示的那一帧"）⇒ 出声不影响画面；⚠️ 出片那条腿**根本不经过页面**（音轨由本仓 ffmpeg
   经 `render/sound.rs` 混入）⇒ 这里怎么改都动不到成片。
2. **Dhampir 分支补 `player.sync(idx.value.soundEvents, v.currentTime)`**：位置在"时钟照发"那段
   （与**最后**一个 `emit('time')` 挨着）。⚠️ 该分支本来就有两处**合法**的提前 `return`
   （A/B 换绑命中、定格遮罩兜底）—— 它们只是"这一帧先不画/先跳"，不是"以后都不出声"，
   所以判据不能写成"sync 排在第一个 return 之前"。
3. **`applyAudio()` 覆盖 `dhampirSourceVideos` ∪ `dhampirTwins`**（经 `dhampirAllSourceVideos()` + Set 去重），
   **不能只遍历 `dhampirVideos`**。
   ⚠️ 这一条是**由行为测试实测抓出来的**，不是读代码想出来的：`dhampirSwapSources` 换绑时把旧元素
   **换出** `dhampirVideos`（`dhampirVideos[i] = tw.el`）同时把它记成新备用，那一瞬间它**不在**数组里
   **却仍在播**（`pause()` 之后下一帧镜像循环又按时钟把它 play 起来）⇒ **两路同源同时在轨、声音翻倍**。
   ⚠️ **备用那批必须永远 `muted`**（判据 = `dhampirIsTwin(dv)` 的**身份**比较，**不是下标**）——
   换台瞬间两套源指向同一段素材，不压住就叠成两声。
4. **出声时纠偏速率收窄**（`dhampirAlignRateRange()`）：`dhampirAlignSources` 的第二档靠
   `playbackRate ±2%`，而**速率连着音高** ⇒ 出声时把幅度收到 ±0.5%（约 1/12 个半音）。
   **两道防线**：① 给源装 `preservesPitch`（含 `moz-`/`webkit-` 前缀，装在"建源"与"换绑"**两条路**上 ——
   换绑那条漏了，换过去那一路就会带着上一轮预滚留下的旧速率跑调）；② 源**都**拿到该属性时才允许用满 ±2%
   （`dhampirSourcesPitchLocked()`，有一个没有就整批收窄 —— 宁可漂一点，也不要一路跑调）。
   ⚠️ 无声时仍用满 ±2%（那时源只是"帧的来源"，追得越紧画面越跟手；硬 seek 实测 307ms/帧）。

**回归判据** = `node scripts/check-preview-audio.mjs`（**20 项 + 10 处变异自证**，已进 `.workbuddy/gates.mjs`）：
① 源不写死静音 ② `startDhampir` 末尾调 `applyAudio` ③ Dhampir 分支真的调了 `player.sync`
④ `applyAudio` 走并集而非只遍历台上那批 ⑤ 并集真的含两批 ⑥ 备用恒静音 ⑦ 收窄区间 + 三前缀
⑧ 纠偏**真的调用**那个区间（函数写了不调用 = 空转）⑨ 建源与换绑**都**装音高保持。
⚠️ 还另有一份**行为测试**证明过它（临时脚本，未入库）：12 项覆盖"音量落到所有在响的元素 / 静音按钮 /
音量 0 是合法值 / **换绑后旧元素立刻静音** / 收窄三态 / 三前缀"——④ 就是它抓出来的。

## ③ 封面设计器（`CoverDesigner.vue`）

`CoverDesigner.vue` —— 封面设计器，props `dir`（必填）/`pid`，emit `changed`；只在该片段被占用时禁用「生成封面」。

### 3.1 真图层栈（2026-09 起）与背景模式

- **面板布局**（用户要求把设置栏与图层栏**换位**）：左预览 / 右面板 = **上：属性面板 · 下：图层组列表 ·
  最底：工具条（`+ 新建图层` + 用法提示）**。整列高度由 `syncControlsBound()` 限到与左侧预览等高，
  三块各自滚动。
  ⚠️ 那个高度必须是**确定高度**（给 `height`，不是 `max-height`）：`height:auto` 的 flex 列没有"剩余空间"
  可分，属性面板会长成内容高度、**图层列表跟着上下跳**（用户报过："固定图层组位置在列表的下 50%"）。
  定高之后 `.lyr-list` 的 `flex:0 0 50%` 才有百分比基准 → 列表**钉在面板下半部**，换选中层（属性面板
  长短变化）它的位置一动不动；窄屏（≤900px）单列时解除定高、列表退回内容高度（上限 260px）。
- **列宽 = `minmax(0,1fr) minmax(440px, 40%)`**（2026-09 用户要求："封面设计区小一点、右侧图层编辑和图层设置
  宽一点"；改前是定宽 `372px`）。40% 只在宽屏生效、`440px` 是下限（图层行的六个格与文字面板"两列一行"要放得下）。
  实测（外层内容宽 1146px，harness 读 `gridTemplateColumns`）：**755/376 → 672/458**（预览 −11%、面板 +22%）；
  内容宽 902px 时是 446/440。
  ⚠️ 两个方向是**联动**的：预览高度 = 宽度 × 9/16，而面板高度跟着预览 → **把右栏再改宽 = 面板变矮**、
  文字那一屏要滚的地方更多。改这一条要连着看下面那条下限。
- ⚠️ 面板高度是"等于预览高度"**外加一个下限** `CONTROLS_MIN_H`(380px)。面板里能压缩的只有属性面板
  （`min-height:120px`），`.lyr-list` 是 `flex:0 0 50%`、工具条 `flex:none` —— 再挤就是
  `.controls{overflow:hidden}` 把**最底下那条工具条裁掉**（实测内容宽 936px：面板 288px、工具条被裁 46px，
  "新建图层"整个看不见）。⚠️ **`.lyr-list` 的 `flex-shrink` 必须是 0**：改成 `0 1 50%` 会让它被按 flex-basis
  比例收缩（属性面板的 basis 是**内容高度**，基准之和早就溢出）—— 422px 的面板里列表只剩 120px，而且
  **换选中层时列表位置跟着跳**（实测 334 → 243），正是这条规则要防的那件事。
- **顺序与可见性都落盘**（不再是 v1 那种"只读投影"）：`[cover] layers = [{id, visible}, …]`，
  id = `bg` / `frame` / `filter`（渐变+暗角，一层两块）/ `title` / `fx:<CoverEffect::id>`。
  出片由 `cover.rs::resolve_cover_stack` + `plan_cover_layers` 折成每块的 `z-index`；
  前端 `resolveStack()` / `blockZ` / `layerVisible` 是**镜像**（连容错规则一起抄：缺项按默认序的相对位置
  补齐、未知 id 忽略、重复只认第一次、没列出的特效仍然渲染）。**两边必须一起改。**
- **图层行**：`[拖柄 ⠿] [眼睛] [缩略] [名字] [锁定/辅助线徽标] [垃圾桶]`。
  - **眼睛 = 非破坏性隐藏**（`visible:false` → 出片 `display:none`）：属性一个字节不动，行留着（淡化）。
    两条安全区走 `localStorage['yekitrim.ui.coverGuides']`（不在 `layers` 里），交互一样、含义不同。
  - **删除 = 从栈里摘掉**：固定层（背景/滤镜/文字）属性全留，`+ 新建图层` 加回来就恢复；
    **特效/表情例外** —— 它的"属性"就是 `[[cover.effects]]` 那一条，删除连它一起删。
  - **视频预览 `locked`**：可隐藏，**不给删也不给拖**（它就是封面那一帧）。
  - ⚠️ **别退回"用属性字段模拟开关"**：v1 那套存续判据（`title_lines=['']` / 清渐变 / 纯黑背景）已废，
    层存在性只看 `layers`。
- **拖动排序**：行上 HTML5 DnD（`draggable` + `dragstart/dragover/drop`），落点画 2px 主色指示线
  （`drop-before/after`），`Esc`/无效落点复原；**列表显示序 = 栈的倒序**，换算集中在 `moveLayerTo()`。
  快捷键绑在 `.controls` 上（**别挂 window**，精修页还有别的全局键）：`Ctrl/Cmd+[ ]` 上下移一层、
  加 `Shift` 置顶/置底、`Delete` 删选中层。
- **画布拖动 = 跟着"选中的那一层"走**（用户口径："选中哪个图层就可以手动拖动"）：
  `dragTarget` 是唯一判据 —— 视频预览 → `bg_x/bg_y`（滚轮 `bg_zoom`）· 文字 → `text_x/text_y` ·
  特效/表情 → `x/y`（滚轮 `amount`）· 滤镜/背景/安全区 → **不可拖**（光标 `not-allowed`，`title` 里说明）。
  `Shift` 约束单轴；拖动过程只改本地 ref，落盘仍走 600ms 防抖。
- **封面三层：文字 / 关键帧 / 背景**：`#title`（文字）、`#frame`（**关键帧层**：当前帧清晰图 `_cover_frame.jpg`）、
  `#bg`（**背景层**）—— **三层各有各的图源，谁也不许覆盖谁**（层序可以拖，图源不会互换）。
  背景层三种模式（`[cover] bg_mode`）：`color` 纯色（默认黑）/ `blur` 关键帧**模糊副本**平铺（`blur_scale` 0.2~4）/
  `image` 导入的自定义图片 `_cover_bg.jpg`；各自读各自的文件（blur → `COVER_FRAME_IMG`，image → `COVER_BG_IMG`），
  `image` 模式**没有那个文件就退回 color**（别在模板里挂死图）。
- ⚠️ **导入背景图片写的是 `_cover_bg.jpg`**（`/api/polish/cover-background` 同时把 `bg_mode` 设成 `image`）。
  **别再写回 `_cover_frame.jpg`** —— 那是"背景替换关键帧"的老 bug（用户报过「又把关键帧那一层给我删了」）。
  `cover.image` 也**不要**在导入时清空（它是关键帧的来源之一）。
- ⚠️ **出片不再删 `_cover_frame.jpg`**（`cmd_cover` 末尾那句 `remove_file` 已删）：它就是关键帧层的图源，
  删了等于①用户导出的那一帧白导、②下次出片又按 `frame_pos` 重新截、同一份封面每次换脸。
- ⚠️ **帧层（2026-10 三轮口径，最终版）**：帧层 = **舞台上常驻的播放器**（`.stage-video-live`），
  用户直接在大画面里播放/拖进度条选帧；`_cover_frame.jpg` 的 `<img>` 退为**兜底**（视频取不到时才显示）。
  **播放器与"当前选中哪个层"无关** —— 曾经按选中层门控过一轮，结果是两个真 bug：选中别的层就没法
  拖进度条、刚选好的帧一切层就跳回静态图（用户当场报回来）。**别再把播放器藏起来或门控。**
  ⚠️ 当年那条"帧层不许有 video"的契约是**错的**：出图链路是 ffmpeg 按 `frame_pos × 时长` 截帧 →
  写 `_cover_frame.jpg` → 注入 `cover.html` → 出图程序渲染，**前端从来不是帧源**，它只负责让用户
  **选时间点**。那次"设计器第 0 秒 / 成片 frame_pos"的真根因是**打开时没把播放头对到 frame_pos**，
  守门人是 `seekToFramePos()`（门禁里对应"打开设计器会按 frame_pos × 时长 seek"那条），不是"禁止播放"。同工程实测（8s 片、`frame_pos=0.4`）：帧层若按旧行为取 0 秒与成片差 **11.09/255**，
  改成同一帧后 **0.00/255**。配套三条：① 打开时 `framePos ← frame_pos` 并 `seekToFramePos()` 对上进度条；
  ② 「导出关键帧」把时间点**写回 `frame_pos`**（`at / duration`，图与点成对）；
  ③ 「生成封面」**无条件**刷新关键帧 —— 原判据 `if (time > 0)` 会让「没拖过进度条」的用户选的那一帧
  永远进不了产物（0 秒是合法取值，与「音量 0 是合法值」同一条教训）。

### 3.2 新建图层：类型选择器

底部工具条的 `+ 新建图层` 弹一个小面板（**向上弹**），**按类型分两组**（判据是"能不能再叠一层"）：

| 组 | 类型 | 落在哪 | 说明 |
|---|---|---|---|
| 可叠加（任意多个） | 表情 · 光晕 · 模糊 · 暗角点 · 颜色覆盖 · 扫描线 | `[[cover.effects]]` 各一条（带稳定 `id`） | 表情与特效是同一条数据的两个 `kind`；表情默认贴主播（`liverOrigin()`，帧层变换的不动点），新层**追加到栈顶** |
| 单层（每种一层） | 背景 · 滤镜 · 文字 | `[cover] bg_*` / `gradient_dir+vignette` / `title_*` | 已经在栈里 → 标「已有」；**不置灰**，点它 = 选中那一层（被隐藏时顺带打开） |

- **视频预览与两条安全区不在菜单里**：前者是基础层（`locked`），后者只是辅助线 —— 菜单底下一行写明。
- 这九项就是"当前设计器里能编辑的全部类型"，`+ 新建图层` 是它们唯一的入口（别再退回一排 `<details>` 分组）。
- ⚠️ **特效 `id` 用短随机串**（不是自增序号）：序号会"这次会话删过一条、下次会话又从 1 数起"撞车，
  撞了之后栈里两条同 `id` 只解析出一条（= 图层凭空少一个）。老数据的空 id 在**读回时补一个**再落盘。

### 3.3 出片侧的注入与"零回归"快路径

- `cover_layer_css(plan, legacy)`：**自定义栈**时给固定的五块写 `z-index` + 隐藏层 `display:none`，
  并把 `#fx-extra` 退回 `z-index:auto`；**默认栈**（含全部可见）时只补回模板原来那条
  `#fx-extra{z-index:1}`、**一个逐层 z-index 都不注入**。
- ⚠️ **为什么要有这条快路径**：实测给五块都写上 z-index 之后，Chrome 会换一条合成路径 ——
  缩放中的帧层重采样差 ~9/255，肉眼看不出，但 **SHA-256 会变**，而"存量逐字节零回归"是本仓
  真出片对拍过的硬判据。判据：`.workbuddy/cover-layer/` 三个样本（横 / 竖 / 六种特效全上的 rich 工程）
  改前改后 **SHA-256 相同**（rich：`F608702D96C0D7BC`）。
- ⚠️ **`overlay` 特效额外带 `isolation:isolate`**（只它，`blur` 加不得）：改造前 `#fx-extra` 是 stacking
  context，`mix-blend-mode:overlay` **没有背景可混**（实测"绿底叠 #ff3366"是橄榄绿）；拆开后会真混合 →
  **老封面会变色**。设计器的 `.fx-slot-iso` 是镜像（**同构，不能省**）。
- 自定义栈的验收（端到端）：把 `title` 排到栈底 + `filter` 隐藏 + 表情隐藏 → 成片里标题被盖住、
  没有渐变/暗角、没有表情（`.workbuddy/cover-layer/after/LAYERS-cover.jpg`）。

### 3.4 画布拖动（跟着选中层）与帧层 transform 的单一来源

- **画布上的拖动改哪一层，只看"当前选中哪一层"**（用户口径）——`dragTarget` 一个判据：
  视频预览 → `bg_x/bg_y`（画布 px，按 `stageScale` 换算）· 文字 → `text_x/text_y`（百分比，中心点）·
  特效/表情 → `x/y`（百分比）· 滤镜/背景/安全区 → 没有位置可拖（光标 `not-allowed` + `title` 说明）。
  `Shift` 约束单轴（锁定开始拖动那一刻的主方向）；拖动过程只改本地 ref，松手让 600ms 防抖保存落盘。
- **帧层（视频预览）的 transform 只此一份**：`cover.rs::cover_frame_vars`（原点取 `layout.liver` 的聚焦点，
  右半边基准 1.4 / 左半边 1.3，再乘 `bg_zoom` 夹到 0.5~3）；前端 `mediaStyle` + `GRID_CENTERS` 是镜像。
  模糊倍率同理：`cover_blur_css` ↔ `coverBlurCss`。**改一边要改另一边。**
- ⚠️ **关键帧文件可能不存在**（新工程没导出过、也没出片过）：设计器里模糊背景会先 404 ——
  图被 v-if 摘掉、看着像"没运行"。`onBgImageError()` 的补法：**只对 blur 模式**（image 模式缺图只提示"还没导入"）
  **不带 time** 调 `/api/polish/cover-frame`，服务端按 `[cover] frame_pos` 截，只补一次（`bgFrameTried` 防循环），
  再失败就在模式下面提示手动「导出关键帧」。换背景模式会重置这两个状态。

### 3.5 安全区线框（几何表）

- **安全区线框**：舞台（1920×1080）上叠 `.guide-layer`（`inset:0`、`z-index:950`、`pointer-events:none`），
  压在手势层（`900`）与所有内容层之上才看得出"标题有没有越线"。两条框：**16:9 安全区**（`.guide-169`）= `width:95%; height:95%`
  居中 → 舞台坐标 **1824×1026**；**4:3 裁切区**（`.guide-43`）= `height:100%` + `aspect-ratio:4/3` 居中 → **1440×1080**。
  ⚠️ **别写成 `inset:5%`**：CSS 的 inset 百分比是**横向按父宽、纵向按父高**解析的，
  看着像等比其实不是（这么写会得到 1728×972，连 16:9 都不是）。等比缩放只能写 width/height。
  两条框**故意互相交叠**，别为了"好看"改成嵌套。线是**白线 + 1px 深色外圈**（封面素材亮暗不定）；
  线宽与字号用**预览 px**（**不乘 `stageScale`**）。显隐 = **图层列表里那两行上的眼睛**（各自独立、默认都开，
  舞台下**不再放 chip**：同一件事不说两遍），偏好落 `localStorage['yekitrim.ui.coverGuides']`，
  **在 setup 里同步读**（放 computed / onMounted 会先画一帧展开态再切，闪一下）；**不写 `polish.toml`、不加后端接口**。
- ⚠️ **只画在设计器里**：`crates/yekitrim-polish/templates/cover.html` 与 `cover.rs` 一个字都没改，
  所以线框**不可能**进入成片。将来若要往 CLI 那个独立预览页加，必须同时补 `html.export .guide-layer{display:none}`
  （与 `#toolbar` 同一机制），否则进成片。

几何表（舞台坐标，1920×1080）：

| 框 | 类 | CSS | 舞台坐标 |
|---|---|---|---|
| 16:9 安全区 | `.guide-169` | `width:95%; height:95%` 居中 | **1824×1026** |
| 4:3 裁切区 | `.guide-43` | `height:100%` + `aspect-ratio:4/3` 居中 | **1440×1080** |

### 3.6 标题首次自动排版

- **标题首次自动排版 = `fitTitleIntoSafeArea()` 塞进 4:3 裁切区**：触发只有两处（`loadCoverConfig` 发现磁盘上
  `title_lines` **为空**；`generateTitleByLlm` 拿到新标题），手动操作（改文字 / 拖滑杆 / 拖拽）**一律不触发**
  —— **别在 watch 里接它**。**已有封面一律不碰**（判据 = 磁盘上有没有 `title_lines`）；只动
  `font_size` / `text_x` / `text_y`（必要时一次 `text_width`），零后端改动。目标矩形 = 那条 `.guide-43` 线框
  （画布 `x ∈ [240, 1680]`、`y ∈ [0, 1080]`）再内缩 `12 + stroke/2` 画布 px。**量的是 `.title-text` 而不是
  `.title-wrap`**（后者是 62% 宽的容器）。顺序：先按 `min(tW/bW, tH/bH)` 比例缩字号**并迭代**（字号一变
  换行数会跳变），到下限 24 还偏高才放宽一档文字区，最后按各边溢出量平移；**告警只看最终状态**
  （`relaxed = !fits(b)`）。实在放不下会 `console.warn` + `.fit-hint` 琥珀提示，不静默。
- ⚠️ **默认对齐是 `left`**（`default_cover_text_align()`，2026-09 由 `center` 改来）：默认 `text_x = "35%"`。
  已有工程的 `polish.toml` 里都显式写着 `text_align`（含 `"center"`），字段存在就不走默认，**不受影响**。
  改这个默认值**不要顺手改 `cover.rs::cover_text_align` 的 `_ => ("center","center")` 兜底** ——
  那个 `_` 同时兜着合法值 `"center"` 本身。
  （设计器里那 4 个排版预设 —— 左文/居中/右文/极简 —— 已按用户要求删掉；CLI 侧 `polish cover --template` 仍在。）
- **标题文本框固定 5 行高、不可拖拽缩放**（`rows="5"` + `.title-lines { line-height:1.5; resize:none }`）。
  别改回 `rows="2"`，也别加 `resize:vertical`；字段外观用设计基准 `.fld-input`，不要再加紧凑覆盖。
  出片与预览都按 `title_lines` 逐行渲染。

推导留档：左对齐 + `text_x=35%` / `text_width=62%` 时，文字左边缘落在 76.8px，**本就在 4:3 裁切区
（左边界 240px）之外**，所以上面的"按各边溢出量平移"对左对齐而言是第一次必然发生。

### 3.7 设计器预览 ⇄ 出片的镜像清单

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
  由 `[cover] layers` 决定 —— 设计器与出片**都从栈算 z-index**（前端 `blockZ`、Rust `plan_cover_layers`），
  **CSS 里不许再写内容层的 z-index**（`check-cover-stage.mjs` ②/⑦ 钉住）·
  特效公式（`blur` = `1 + amount×14` px、`overlay` = `rgba(hex→rgb, amount)` + `mix-blend-mode`（外面套
  `isolation:isolate` 的槽位，见 3.3）；emoji 字号 `48+amount×240`、`drop-shadow(0 6px 14px)`，
  预览一律乘 `stageScale`）·
  **`liverOrigin()` 的优先序必须与 `region_origin` 一致**：自由选框中心 > `origin_grid` > `origin` > `grid[0]` > `65% 50%` ·
  `bg_zoom` 夹取范围四处统一 **0.5~3** · 没有 `title_lines` 时按标点拆 `project.name` 并砍掉 `【主播名】`（预览也要拆）。
- ⚠️ **高亮关键词也是"两份实现"的一处**（2026-09 用户报「高亮词没有实际使用上」）：出片认
  `cover.highlight_keywords`，**为空时回退 `[filter] highlight`**（`cover.rs::cover_highlight_words`，
  也就是「设置 · 关键词过滤」里那串）—— 设计器必须照实显示**回退后真正生效的那串**并写明来源
  （`hlFromFilter` / `hlSourceHint`），用户一增删就写进封面自己的那份。以前只读 `cover.*`，
  于是"面板里空的、标题却在高亮"。
  ⚠️ **另一处更隐蔽的同款（2026-10 用户报「关键词字体被改了、高亮色没显示」）**：高亮那段 HTML 是
  `highlightTitleLine()` 拼出来后经 **`v-html`** 注入 `.title-text` 的，而 **Vue 的 scoped 编译不会给
  `v-html` 内容打 `data-v-xxx`** —— 规则写成 `.title-text .hl` 会编译成 `.title-text .hl[data-v-xxx]`，
  **永远匹配不上**（实测 dist CSS 就是这个形态）。症状极具迷惑性：**字体照旧会变**（`fontFamily` 在父级
  `.title-text` 上、span 继承），只有**高亮色**静默退成继承来的 `#f0e6d3`。**必须写 `.title-text :deep(.hl)`**
  （编译成 `.title-text[data-v-xxx] .hl`，属性落在父级、`.hl` 不受限）。出片侧没这问题：`cover.rs` 把
  同一串 HTML 注入 `cover.html` 的**全局** `<style>`，没有 scope。钉子在 `check-cover-stage.mjs` ⑥b。
  ⚠️ **高亮字号（2026-10 用户要求「给高亮区域增加一个字号设置，输出和前端要统一」）**：只作用于
  高亮词那一段，**不动字体、也不动整块标题的字号** —— 就是 `cover.hl_font_scale`（默认 `1.0` = 与标题
  同号，老工程零迁移）。两端读**同一个 CSS 变量** `--hl-fs`：设计器由 `titleStyle` 写进行内 style，
  出片模板从 `__HL_FS__` 填进 `:root{}`（**静态渲染器不跑 JS，变量只能从那儿来** —— 缺了它整条
  `var(--hl-fs)` 声明会被丢掉，同 `:root` 里其它变量那条教训）。改成写死 px/em 就会出现"预览调了、
  成片没变"。钉子在 `check-cover-stage.mjs` ⑥b（含两处变异自证），Rust 侧是
  `cover::tests::hl_font_scale_reaches_both_builders` 与 `cover_config_persists_…` 的夹取往返。
- **文字面板分三区**（位置大小 / 排版 / 高亮 tag）：小标题走 `.lyr-sec`（小字 + 一条发丝线）。
  ⚠️ 它**不能**叫 `pane-sec`：`check-cover-stage.mjs` ⑥ 的 `\bpane\b` 钉子会把它判成"复用了全局
  `.pane{display:none}`"（2026-09 就这么红了一次）。
  「位置大小」那一行 = **左文本框 / 右侧控件列各占 50%**（用户要求；以前右列钉死 `168px`），
  两侧都写 `minmax(0,1fr)` —— `1fr` 的自动下限是 `auto`，文本框的固有宽度会把轨道顶出去。
- **文字面板的数值是"滑块 + 行尾可填数字"**（`.rv-input`）：滑块有 step 栅格（`line_height` 0.05、
  `font_size` 2…），手填才落得到栅格之外；手填值只在 `[min, max]` 上夹、**不吸附 step**，清空/非数字还原、不写模型。
  没用 `SliderField.vue`（设置页那 16 处的组件）：它 `@container (max-width:220px)` 会把标签挤到第二行，
  而这里是**一行两列**的紧凑栅格（面板 458px 时每格 ≈210px；改列宽前右栏只有 372px、每格 ~170px）。

### 3.8 设计器里的两种「隐身」根因（都不报错）

两种都是"整块东西看不见，但控制台一个字都没有"，2026-09 图层化改造时被 headless harness 抓到：

- ⚠️ **设计器的"素材左右黑边"**（用户报过"设计器里素材左右两条黑边、出片正常"）：`contain`
  让出的留白区由**元素自身背景**填充，而 `styles/app.css` 有一条**全局元素选择器** `video { … background:#000; border-radius:var(--r-sm) }`。
  `.stage-video` 是类选择器、**只覆盖自己写过的属性**，这两条会**穿透**进来，还压在 `.stage-bg`（z-index:0 < `.stage-media` 的 1）
  之上把模糊背景层整个盖住 —— 竖屏素材于是在设计器里是**两条纯黑边**。因此 `.stage-video` 的
  `background:transparent; border-radius:0` 是**必需的覆盖、不是装饰**；出片模板用 `<img>` 且不引 app.css，所以**只有设计器**这样。
- ⚠️ **属性面板的类名不能叫 `.pane`**：`app.css` 里有一条**全局** `.pane { display:none }`（设置页那几个 tab 面板
  靠 `.on` 才显示）。新面板如果也叫这个类名，就**整块 display:none** —— 页面不报错、只是没有属性面板。
  现名 `.lyr-props`。同一个机制、同一条教训：**给组件起类名之前先看 app.css 有没有同名规则**。
- ⚠️ **高亮词的字体会被全局通配符抢走**（2026-10 用户报「高亮词和标题字体不一样，后端没问题、前端有问题」）：
  `styles/app.css` + `styles/tokens.css` **各有一条** `* { … font-family:var(--font) }`。通配符**直接命中**
  `v-html` 注入的 `<span class="hl">` —— 直接命中的声明压过**继承**来的值，于是库字体在 span 上被换回全站
  `--font`：**标题是选的库字体、高亮词是系统字体**。出片侧没这问题（`cover.html` 的 `*` 只设
  `margin/padding/box-sizing`，**不设 font-family**），所以只有设计器会漂 —— 正是"后端没问题"的原因。
  修法 = 高亮规则里显式写 `font-family:inherit`。**别只查 `.hl` 有没有自己的 `font-family`**（它确实没有，
  继承看起来天经地义）—— 这类 bug 的判据是"有没有**更靠近** span 的规则命中它"；本机 headless 实测 A/B：
  没有 `inherit` 时 `getComputedStyle(.hl).fontFamily` 退成 `"Noto Sans SC", …`，加上就是库字体。
- ⚠️ **两栏的 `min-height` 必须是 0**（2026-10 起）：整列高度是 JS（`syncControlsBound`）按左侧预览高度设的，
  两栏要能**各自滚**。flex/grid 子项默认 `min-height:auto`（= 内容高度）⇒ **永远出不了滚动条**，而是把两栏
  连同外层一起撑高、溢出被 `.controls{overflow:hidden}` 裁掉（表现：最底下那排控件点不到，且**不报错**）。
  （旧结构是上下排列 + `min-height:120px` 下限，防的是"属性面板被压成 0 高"；左右排布后那个理由不再适用。）
- 钉子都在 `node webui/scripts/check-cover-stage.mjs` 的 **⑥ / ⑥b / ⑥c**（`usesClass` + `paneCanScroll` +
  两栏并排/减半/字体 inherit，各含变异自证）。真挂载组件量 `getBoundingClientRect()` / `getComputedStyle()`
  的做法见「附：验证与门禁」的 harness 段。

### 3.9 四处同步与写入门闸

- **四处必须同步**（漏一处的表现都是"点了没反应 / 两边不一样"）：
  ① `CoverConfig`（`yekitrim-polish/src/project/config.rs`，含 `layers` / `CoverLayer` / `CoverEffect.id`）；
  ② `/api/polish/cover-config` 的白名单 patch（`layers` 整数组替换、`effects[].id`）；
  ③ `templates/cover.html`（`__LAYER_CSS__` 占位 + `#fx-extra` 不许有 z-index）；
  ④ **设计器的镜像**（`resolveStack` 的容错、`blockZ` 的编号、`layerStyle()` 的行内 z-index/display）。
  公式与净化只在 `cover.rs` 一份：`cover_blur_css(scale)`、`sanitize_hex_color`
  （**颜色直接进 HTML/CSS，必须净化**，非法值退 `#000000`）、`cover_bg_mode`（只有 `blur` 走模糊，其余一律 `color`）。
- **写入门闸**：`CoverDesigner` 的 `configLoaded`（读失败时拿默认值写回会把磁盘配置清成默认），
  读失败必须出错误条 + 「重试」，保存失败用面板头那一行常驻的 `.save-line.err`（**不再用 toast**：600ms 防抖会刷屏）。
- **服务端钉子**：`crates/yekitrim-server/src/api/polish.rs` 的
  `cover_config_persists_layer_stack_and_effect_ids`（空 id 丢掉、`visible` 落盘、未知 id 只在解析侧忽略）。

### 3.10 右栏内部：左「设置」/ 右「列表」左右排布 + 宽度减半（2026-10）

用户原文：「图层列表和图层设置改为左右排布，而不是上下结构，减少宽度为原来的一半来换取更多显示范围」。

- **结构**：`.controls`（面板）> `.lyr-head`（标题行）· `.lyr-body`（两栏）· `.lyr-bar`（底部工具条）。
  `.lyr-body` 是 **两列 grid**：**左 = `.lyr-props`（设置）/ 右 = `.lyr-col` > `.lyr-list`（列表）**。
  工具条在两栏**之外**、横跨整个面板宽度 —— 别把它塞进某一栏。
- **宽度**：`.designer-grid` 的右栏 `minmax(440px, 40%)` → **`minmax(220px, 20%)`**（减半）。
  左栏是 `1fr`，右栏一窄预览就自动变大 —— 这就是"换取更多显示范围"。
- **为什么用 grid 而不是两个 flex 子项**：flex 子项的 `flex-basis` 会被**内容高度**顶开（文字层那一屏属性
  约 800px），两栏就不按比例了；grid 的 `grid-template-columns` 只看轨道定义，与内容无关。
  两栏都写 `min-width:0`，否则长的设置项会把 grid 撑破、右栏被挤出面板。
- **实测几何**（本机 headless 真挂载组件 + 真 dist CSS，外层内容宽 1146px）：

  | | 改前 | 改后 |
  |---|---|---|
  | 右栏总宽 | 458px | **229px** |
  | 两栏位置 | 上下叠 | 设置 x=937 / 列表 x=1055，**同一 y** |
  | 每栏宽 | — | 各 111px（等宽） |
  | 左侧预览 | 672px | **901px** |

- ⚠️ 量几何必须给测试页的元素补上 `data-v-<scopeId>` —— 本仓的布局规则全是 scoped 的，
  光挂 dist CSS 一个都不命中（实测会得到"两栏上下叠、各 1146px 宽"的假象）。
  scope id 生产构建会随源码变（见 `webui/AGENTS.md`），**别在探针里写死**。
- 钉子在 `check-cover-stage.mjs` **⑥c**（两栏并排 / 设置在前 / 宽度减半 / `min-width:0`，含两处变异自证）。
- **窄屏退路**：`@media (max-width:900px)` 下 `.lyr-body` 退回单列，两栏各给 `max-height:260px`
  （两栏并排在这个宽度下每栏只剩百来像素，没法用）。

---

## ④ 运镜（camera）

**三个能力**：**聚焦**（`anchor` = liver/danmaku/sticker/center）、**位移**（再写 `anchor_to` 即从 origin 平移到
to_origin）、**画中画**（`zoom="远"`），外加**附加变种：手持摇摆**（`handheld`）。
**锚点是「意图」、坐标是「结果」**：`process` Step 2d 的 `resolve_camera_anchors` 把 `anchor`/`anchor_to`
解析成 `origin`/`to_origin`，所以改了选框/挪了贴纸重跑 process 就生效，不用手改坐标；渲染侧四条路径都只认百分比坐标。
排位在贴纸落点(2b)与时长自适应(2c)之后 —— `anchor="sticker"` 要读最终位置。

### 4.1 倍率只有一张表

- **倍率只有一张表**：`CameraStyle::factor`（`yekitrim-render-core/src/types.rs`），模板 canvas / 模板 GSAP /
  `engine.js` / `timeline.rs` 四处都走它。⚠ 模板 canvas 路径曾硬编码 `1.8/1.5` 且完全不读 `[style.camera]`，而它才是出片路径
  （`yekitrim-polish/Cargo.toml`：「chrome-render 为默认出片路径」）——别再把倍率写回去。
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
  只读诊断：`VT_POLISH_DIR=<项目目录> cargo test -p yekitrim-polish --lib diagnose_camera_anchors -- --ignored --nocapture`；
  配额/对齐/收工 + 去静音阈值 + 贴纸对齐实测：`diagnose_pip_and_silence`（加 `VT_CAM_JSON=1` 多打一行 `CAM_JSON={...}`）。

---

## ⑤ 贴纸落点（Step 2b）与区域编辑

- **贴纸落点推导**（`crates/yekitrim-polish/src/process/placement.rs`）：帧坐标固定 1920×1080，贴纸固定 280×280，
  `ev.x/ev.y` 是**盒子左上角**百分比（模板再钳制到 `[32,1608]×[32,768]`）。障碍 = ① 人物 `layout.liver`、
  ② 弹幕区域 `layout.danmaku_area`（都在**源帧**坐标里框选，出片时视频整体被运镜缩放 → 先按该贴纸可见窗口的
  相机状态投到输出帧，zoom 取模板 Canvas 路径的 1.5/1.8 × 瞬时特效预留 1.10）、③ 字幕带（两条路径取并集）、
  ④ 真实弹幕带（只有 `data/danmaku.json` 非空时才是障碍）、⑤ 同刻已落位的其他贴纸。**落点规则**：候选 = 16px 网格 +
  各障碍边缘的「临界位」；先取 clearance 最高的一档（×0.9），在该档里按「盒心到帧心距离」排序取前 8 个，
  再用 seed 伪随机挑一个 —— 即**安全区中心而不是边缘**。没有安全位时退化为「重叠代价最小」并告警。
  **`layout.sticker_area` 已降级**：`grid` 完全不参与落点；手写的自由选框只作**偏好**。
  只读诊断：`VT_PROBE_ROOT=<media 目录> cargo test -p yekitrim-polish --lib diagnose_sticker_placement -- --ignored --nocapture`。
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
  库与 zip 安全细节指向 `crates/yekitrim-polish/src/assets/AGENTS.md`。⚠️ **这是全站第二处有意接收用户上传的入口**
  （第一处是封面背景图 base64）；「服务器文件建项目」那条仍然**永不接受上传**。别按旧规矩把它删掉 —— 它是有意做的。
  ⚠️ 前端 `webui/src/api/stickers.js::UPLOAD_MAX_BYTES` 与 Rust `UPLOAD_MAX_BYTES`
  是同一个数（改一处要改两处），前端必须先挡。**`preview` 只存 `{pack, name}`**，文件对象由 `previewFile`
  从 `detail` 现取（别存快照）。
- `FontLibrary`：首页字体库区块（**仅 admin**；2026-10-02 起管理就内嵌在这一块，**没有单独的 `#/fonts` 页**）。
  导入（`ServerDirPicker mode="file" :exts="['ttf','otf']"`，**路径由服务端拼**，与「表情包 → 添加」同一份）/
  下载自带字体（幂等，清单在服务端）/ 删除（自带的那支按钮置灰 + 服务端 403）/ 设为默认（`is_default`
  与字幕/封面下拉、状态行三处共用）全在这一块；「测试文本」实时喂所有卡片试渲染（空 = 各卡用「永 Aa 123」）。
  预览用**现场注册的出片同源字节**（`/api/polish/font?name=<id>` 建 `FontFace`），浏览器认不出的格式
  卡片上挂「预览不可用」，**不许静默回退到系统字体假装换好了** —— 详见 `docs/plan/font-library.md` §12/§21。
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

**布局类改动的实测办法（封面设计器，2026-09）**：本仓没装 jsdom/vitest，但本机有 Chrome —— 临时搭一个
headless harness 就能真挂载组件、真量几何，`.workbuddy/`（已 gitignore）里做过一轮，抓到了 3.8 那两条"隐身"：

1. 用 `@vue/compiler-sfc` 把 `CoverDesigner.vue` 的 `<script setup>` 抠出来（剥掉 `import`，把
   `ref/computed/…` 与那几个 api 替身挂到 `window`），模板用 `vue.global.js`（全量包，带运行时编译器）。
2. 页面里 `app.component('Icon', …)` + `app.config.globalProperties.fmtTime = …`（运行时编译的模板不认 setup
   之外的 import），`link` 上 `app.css` 与组件的 scoped CSS，宿主宽度按真实卡片宽度给。
3. 断言用 `chrome --headless=new --window-size=1600,1000 --virtual-time-budget=20000 --dump-dom` 读回来：
   行数/文字、点击后的显隐、payload（替身 `coverConfig` 收 `__saved`）、以及
   `getBoundingClientRect()` + `getComputedStyle().display/minHeight` + `elementFromPoint`（弹层是否被盖住）。
   ⚠️ 两个坑：脚本在 `<head>` 里同步执行时 `document.body` 还是 null（整段要等 `DOMContentLoaded`）；
   `<script setup>` 的 setup 会给自己返回的对象打**不可枚举且不可配置**的 `__isScriptSetup`，Vue 见到它就不再让
   `_ctx.x` 解析 setup 绑定 —— 浅拷一份再交出去。

---

> 改动只允许落在 `webui/src/components`；跨模块改动先读对方卡，并跑两条门禁
> （`python3 scripts/check_module_map.py` 与 `python3 scripts/check_module_boundaries.py`）。
