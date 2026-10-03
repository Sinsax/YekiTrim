# webui/src/components — 精修组件层：时间轴编辑器 / 预览 / 封面设计器 / 运镜 UI
> 范围：`webui/src/components`/** ｜ 上游：`webui/src/views/*`｜ 验证：`npm --prefix webui run build`
## 职责

精修页与设置页的全部子组件。壳/侧栏/路由/缓存纪律见 `webui/src/AGENTS.md`，页面纪律见
`webui/src/views/AGENTS.md`，与出片模板的同口径算法见 `webui/src/polish/AGENTS.md`
—— 本卡只写**组件级**规矩，不重复它们。

## 入口

- `TimelineEditor.vue`（7 条泳道，`kind` = `seg|event|sub|mute`）：props `dir`(必填)/`pid`/`currentTime`/
  `playing`/`reprocessBusy`/`propsTarget`(默认 `'#tl-props-host'`)；emits `seek`/`toggle`/`preview`/`saved`/`done`/
  `reprocess`/`retried`/`redo-segment`。`saved` = **这批改动已落盘**（Dhampir 那条腿据此重载），
  与 `preview`（本地镜像、**不落盘**）**别互相替代**；`done` 保证在它之后。
- `PolishPreview.vue`（自绘 DOM/GSAP 层，**只有这一层**）：props `pid`/`dir`/`config`/`subtitles`/`danmaku`/
  `metaDuration`/`selectMode`/`selectRect`/`selectLabel`/`pointMode`/`point`/`pointLabel`/`fontFallback`；
  emits `time`/`playing`/`update:selectRect`/`update:point`。字体栈纪律见下节。
- `CoverDesigner.vue`：props `dir`(必填)/`pid`，emit `changed`（只在该片段被占用时禁用「生成封面」）。
  **图层面板**（左预览 / 右「左：属性 · 右：图层列表 · 底：工具条」，PS 口径）：见「边界 · 封面设计器」与
  `docs/webui-components.md` ③。
- 其余组件逐个的职责与规则见「边界 · 其他组件」。

## 边界
> 组件逐个的详细清单、实测数字、几何/常量表与推导见 **`docs/webui-components.md`**，按组件分节：
> ① 时间轴编辑器 ② 精修预览 ③ 封面设计器 ④ 运镜 ⑤ 贴纸落点与区域编辑 ⑥ 其他组件逐个。
> 各节内部自带子目录（布局几何 / 安全区几何 / 两种「隐身」根因等）。
### 时间轴编辑器
- **布局必须保持紧凑**（它**独占一整行** = 整个内容宽度）：泳道 **26px ×7 条 + 标尺 20px = 202px**；
  标题/提示经 `<slot name="head">` 进 `.tl-head`（见 `docs/webui-components.md` ①）。
- ⚠️ **遮挡**：曲线画在**字幕泳道**上，而那条泳道的字幕块是不透明的（`laneBlocks`）——
  波形 `.wave-layer` 必须是 **`z-index:4`**（压过字幕块与选中块、在播放头之下），否则**说话时段整段被盖住**；
  `pointer-events:none` 不能去掉。
- ⚠️ **刻度**：窗口是 **[-40, -8]**（`WAVE_DB_TOP`/`WAVE_DB_BOTTOM`）。
  **这两个常量就是"看不出高低差"的旋钮**（改回 [-60,0] 动态范围只剩 6.6px）。
- 用户明确**不要**电平参考线/刻度、**不要**新增泳道（保持 7 条 202px）、**不要**把字幕块改半透明。
- 事件字段是 `time` + `end`，**没有 `duration` 了**；老工程的 `duration` 由 `legacy.rs::upgrade_event_times`
  在**反序列化前**换算；三处求值器统一走各自的 `evDur()` / `evDuration()`，别再直接读 `.duration`。
- **控件统一**：**体拖移动 / 左缘改开始 / 右缘改结束 / 双击删除**，字幕与屏蔽块也能拖；唯一例外是
  想换轨道就删掉重加，别再退回 15 种全类型下拉。**属性面板 = 标配块 + 类型专属块**；**「保存修改」按钮已删**。
- ⚠ **字幕/屏蔽轨道必须保持按时间有序**：`splitSelected` 拆出来的后半段要
  `splice(i+1, 0, …)` 插在原块**后面**，**不能 `push` 到数组末尾** —— 追加会让落盘的 `clip.srt` /
  `data/subtitles.json` 乱序（去静音按相邻间隔曾判出 13 段/121s 的假静音、预览按时间二分取当前字幕）。
- ⚠️ **别再改回"落盘后 1.2s 自动跑"**（你还在填 keyword 时它就会插进来，半成品被当成品处理）。
  **落盘 ≠ 编辑完成**：后处理只在 `done` 之后跑（`done` 本身等落盘成功）——控件 `change`（`input` 每敲一键**不算**）/
  取消选中/切片段（带**旧 dir**）/卸载前 `flushSave()`。
- **手工放的事件（`manual`）后处理一律不改写**：新建（`addEventAt`）、复制（`duplicateSelected`）与改时间都打这个标记；**吸附与 `cap_events` 抽稀全跳过**。
- **属性面板用 `<Teleport :to="teleportTo">` 投到 `PolishView` 预览卡里的 `#tl-props-host`**（选中/编辑逻辑留在组件里）。
  **⚠️ 必须门控 Teleport**：目标是**挂载时**解析的，而那时子树还没进 document ⇒ 直接 `:to="propsTarget"` 会
  解析到 null、**面板整块不渲染**（做法与踩坑：`docs/webui-components.md` ①.5）。
- ⚠️ **别用裸 `<template>` 包裹要无条件渲染的内容**：Vue 会把它渲染成真正的 `<template>` 元素，
  要无条件渲染就直接把内容展开。同理：**前端布局类改动必须做几何断言**（`getBoundingClientRect()` 宽高 > 0）。
- ⚠️ 写 headless harness 时：**`app.mount(host)` 会清空 `host` 的内容**、选中块只能靠 **pointerdown**
  （`block.click()` 不选中）—— 布局与做法见 `docs/webui-components.md` ①.5。
- **播放头跟随 = 只在播放时跟，且越界才滚、只滚刚好**：`watch(currentTime)` 第一句是 `if (!props.playing) return`；
  `drag`/`rulerDragging`/手动滚动后 2 秒（`userScrolling`）一律不跟。**别再退回"每帧把播放头钉在 30% 处"**。
- **分段为空时时间轴照常渲染**：「去静音」在时间轴头部**常显**，**别再加回 `v-if="segments.length"` 门控**；
- ⚠ **「读取失败」不许伪装成正常状态，也不许在基线不可信时写盘**（2026-09-16，一批"静默丢改动"的根因）。
  `flushSave` 开头 `if (loadError.value) return`；横幅 + 「重试」在 `tl-head` 下面（**不能放进 Teleport 的面板**）；
  `/api/polish/subtitles` 的 **404 是空态、不是错误**；**不许静默丢弃**；`watch(() => props.dir)` 里 `commitEdit` 之后必须 `resetTimelineState()` 再 `loadData()`
### 精修预览（`PolishPreview.vue`）
- ⚠️ **跳手动屏蔽区间 = A/B 预滚（首选）+ 定格遮罩（兜底），不许退回"直接 seek"**：`videoRef` 是
  **computed**（换台靠 `activeStage`；Dhampir 那条腿靠 `engine.bindSource` 换绑 + 源没就绪不画），
  改回模板 ref、删掉备用源/那道闸门 = A/B **静默失效**。
- 自绘层与出片模板 `crates/yekitrim-polish/templates/index.html` **必须同口径**（覆盖层色档 `gradientCss`
  等纯函数从 `polish/engine.js` 来）；
  ⚠️ **只有自绘这一层**：两条真渲染通道（wasm 实时 + `render-frame` 单帧回退）随「作废 wgpu」删掉了 —— **别再接回来**，要真渲染走 Dhampir 底座。
- 媒体地址一律走 `utils.js::clipMediaUrl(pid, dir, 相对路径)`，别自己拼 `/api/media/...`。
- 音量/静音 = **总音量，必须同时落到所有会出声的 `<video>` 元素**（**只调 SFX 播放器就是 bug**）；读回用 `isFinite` + clamp，
  **不能写 `parseFloat(...) || 1`**。**别再写死 `66vh`**。
  ⚠️ **Dhampir 模式下"台上那块"不是唯一在响的元素**：`applyAudio()` 必须覆盖
  `dhampirSourceVideos` ∪ `dhampirTwins`（**不能只遍历 `dhampirVideos`** —— 换绑中间态里旧元素会掉出
  数组却仍在播 ⇒ **声音翻倍**）；备用那批永远 `muted`（判据 `dhampirIsTwin`）；**`tick()` 的 Dhampir
  分支不许漏 `player.sync(...)`**（"让出重活"让的是**渲染**，音效是**出声**）；出声时纠偏速率要收窄。
  详见 **`docs/webui-components.md` ②.6**；判据 = `node scripts/check-preview-audio.mjs`。
- ⚠️ **字体栈顺序 / 字重必须与出片侧同源**（`.cap` ↔ 服务端 `fallback_stack`：页面透传
  `fetchFonts().fallback_stack`，**不许前端写死字体名** —— 写死则「设为默认」对预览永远无效；
  另有 `--font-bold-file`）：漂了 = 预览雅黑、成片 Noto。
- ⚠ 动图（GIF/动画 WebP）由底座自己解码（v6）：四条硬规矩与调用顺序陷阱见 **`docs/webui-components.md` ②.4**。
- ⚠ **预览看着正常 ≠ 出片正常**：出片的事实来源是 `templates/index.html` 的 canvas 路径
  （`yekitrim-render-core/src/timeline.rs` 是平行实现、不是事实来源）。"改完没反应"先怀疑没重建/没重启。
- ⚠️ **Dhampir 加载期间不许播**（门帘 + 播放键置灰 + `togglePlay` 三道闸；热重载不挂）：见
  `docs/webui-components.md` §2.5。
### 封面设计器（`CoverDesigner.vue`）
- **真图层栈（PS 口径，2026-09）**：面板 = 左预览 / 右「**左：属性 · 右：图层列表** · 底：工具条」。
  ⚠️ 2026-10 用户要求：**列表与设置改左右排布**，右栏宽度**减半**（→ `minmax(220px,20%)`，省下的给预览）。
  两栏走 **grid**（`.lyr-body` 两列），别退回 flex 子项（`flex-basis` 会被内容顶开、不等宽）。
  ⚠️ 面板**定高**（`syncControlsBound` + `CONTROLS_MIN_H` 380px 下限，否则裁掉底部工具条）；
  两栏各自滚、`min-height` 必须是 **0**（`auto` ⇒ 永不出滚动条）。详见 ③.8。
  列表**上 = 更靠前** = `[cover] layers` 的**倒序**。顺序与可见性都落盘，出片由 `resolve_cover_stack`/
  `plan_cover_layers` 注 z-index；前端 `resolveStack`/`blockZ`/`layerVisible`/`layerStyle` 是**镜像** —— **两边一起改**。
- **图层行** `[拖柄 ⠿][眼睛][缩略][名字][徽标][垃圾桶]`：**眼睛 = 非破坏性隐藏**（`visible:false` → `display:none`，
  行留着淡化；安全区那两行走 localStorage）· **删除 = 从栈里摘掉**（固定层属性全留、加回来即恢复；
  **特效/表情连 `[[cover.effects]]` 那条一起删**）· **视频预览 `locked`**：可隐藏、不给删/拖。
  ⚠️ 别退回"用属性字段模拟开关"（v1 那套存续判据已废）。
- **拖动排序**：行上 HTML5 DnD + 2px 插入指示线（显示序 = 栈倒序，换算集中在 `moveLayerTo`）；快捷键绑
  `.controls`（**别挂 window**）：`Ctrl/Cmd+[ ]` 上下移、加 `Shift` 置顶底、`Delete` 删层。
- **画布拖动跟选中层走**（`dragTarget` 是唯一判据）：视频预览→`bg_x/bg_y`（滚轮 `bg_zoom`）· 文字→`text_x/text_y` ·
  特效/表情→`x/y`（滚轮 `amount`）· 滤镜/背景/安全区→**不可拖**（`not-allowed`）。`Shift` 约束单轴。
- **文字面板**：左文本框 / 右控件列**各占 50%**（LLM 生成标题 · X · Y · 字体 · 字号），下面两列一行
  （对齐·行距 / 描边·字距）→ 高亮色·文字阴影 → 高亮关键词。**每个数值都是「滑块 + 行尾可填数字」**
  （`.rv-input`：手填可越 step 栅格；夹 `[min,max]`、**不吸附 step**、清空/非数字不写；
  不用 `SliderField.vue` —— `@container` 会把标签挤到第二行）。
- ⚠️ **高亮词要显示"出片真正在用的那串"**：`cover.highlight_keywords` 为空时出片**回退** `[filter] highlight`
  （`cover.rs::cover_highlight_words`）—— 面板照实显示它并写明来源（`hlSourceHint`），
  用户一改就固化到封面（以前只读 `cover.*` → "面板空、标题却高亮"）。
  ⚠️ 高亮串走 `v-html`，CSS **必须 `:deep()`**（否则匹配不上 → 高亮色失效；⑥b）。
  ⚠️ **高亮字号 `hl_font_scale` 同读 `--hl-fs`**（③.7；⑥b）。
- **新建图层**（底部工具条，菜单向上弹）：可叠加六种（表情·光晕·模糊·暗角点·颜色覆盖·扫描线，落
  `[[cover.effects]]`、带稳定随机 `id`）+ 单层三种（背景·滤镜·文字；已有→标「已有」但**可点**＝选中/打开）。
  视频预览与安全区不在菜单里。**文字预设已删**（`presets`/`applyPreset`/`.preset-row`）；
  ⚠️ CLI 侧 `polish cover --template` 还在。
- ⚠️ **内容层的 z-index 只在行内给**（`layerStyle()`）：CSS 里不许再写 `.stage-bg/.stage-media/.vignette-layer/
  .gradient-layer/.title-wrap` 的 z-index；设计器专属两层固定在上面（900 / 950）。
- ⚠️ **特效要有槽位结构**：`.fx-slot`（承载 z-index）> `.fx-item`，与出片 HTML 同构；`overlay` 的槽位必须带
  `isolation:isolate`（改造前它没真混合，省了老封面变色），`blur` 的槽位**不能**加；`.fx-layer` 也不许有 z-index。
- **三层各有各的图源，谁也不许覆盖谁**（文字 / 关键帧 `_cover_frame.jpg` / 背景 `_cover_bg.jpg`）。
- ⚠️ **属性面板类名不能叫 `.pane`**（`app.css` 有全局 `.pane{display:none}`，改回去整块隐身且**不报错**）；
  ⚠️ **高亮词必须 `font-family:inherit`**（全局 `*{font-family:var(--font)}` 直接命中 `v-html` 的 `span.hl`，
  直接命中压过继承 ⇒ 只有前端会漂；③.7）。钉子 = `check-cover-stage.mjs` ⑥/⑥b/⑥c。
  删掉选中的层后要自动落到文字/栈顶，别让面板空着。
- ⚠️ **导入背景图片写 `_cover_bg.jpg`**（同时置 `bg_mode=image`），**别再写回 `_cover_frame.jpg`**；
  `cover.image` 导入时不要清空；⚠️ **出片不再删 `_cover_frame.jpg`**（它就是关键帧层的图源）。
- ⚠️ **关键帧文件可能不存在**：模糊背景先 404 → `onBgImageError()` **只对 blur**、**不带 time** 调
  `/api/polish/cover-frame`，只补一次（`bgFrameTried`），再失败提示手动「导出关键帧」。
- **安全区线框**：`.guide-169` = `width:95%; height:95%`（1824×1026）· `.guide-43` = `height:100%` +
  `aspect-ratio:4/3`（1440×1080）；⚠️ **别写 `inset:5%`**（百分比横竖不同基准，会得到 1728×972）；
  显隐 = 列表里那两行的眼睛，偏好 `localStorage['yekitrim.ui.coverGuides']`；⚠️ **只画在设计器**。
- **标题首次自动排版 `fitTitleIntoSafeArea()`**：只在两处触发（磁盘 `title_lines` 为空；LLM 拿到新标题），
  手动操作一律不触发（**别在 watch 里接**）；已有封面不碰（判据 = 磁盘上有无 `title_lines`），只动字号/位置。
- ⚠️ **默认对齐 `left`**、`text_x = "35%"`：老工程字段存在就不走默认；**别改 `cover.rs::cover_text_align` 的
  `_ => ("center","center")` 兜底**（那个 `_` 也兜着合法值 `"center"`）。⚠️ `text` 进模板前必须 `escape_html`。
- ⚠️ **镜像清单**（用户报过"设计器与成片不一致"）：字体栈 · 描边 `#2a2522` · 阴影 `0 5px 22px rgba(0,0,0,.78)`
  （×`stageScale`）· 渐变底色取自 `[style.subtitle].highlight_color` · **层序/z-index 走同一个栈** ·
  特效公式（`blur`=`1+amount×14`、`overlay`=`rgba(hex,amount)`+blend、emoji=`48+amount×240`）·
  emoji **描边环**（4 向 `drop-shadow`、**别加对角**；宽 = 标题联排 `描边/4`、特效层 `size×0.03`；描边 0 ⇒ 无环）·
  `liverOrigin()` 优先序 = `region_origin`（自由选框中心 > `origin_grid` > `origin` > `grid[0]` > `65% 50%`）·
  `bg_zoom` 0.5~3 · **高亮字号 `--hl-fs`**。
- ⚠️ **取帧播放器常驻舞台**（`.stage-video-live`；`_cover_frame.jpg` 的 img 只兜底）：**不许按选中层门控**
  （曾这么干 → 切层没法拖进度条、选好的帧跳回去）；前端不是帧源（后端 ffmpeg 截帧），同帧靠 `seekToFramePos`（钉子 ②）。
- **四处必须同步**：`CoverConfig`（含 `layers`）/ `cover-config` 白名单 / `templates/cover.html` / **设计器镜像**；
  `sanitize_hex_color`、`cover_bg_mode` 只在 `cover.rs` 一份。`configLoaded` 是**写入门闸**：读失败出错误条 + 「重试」。
### 运镜（camera）
- **倍率只有一张表**：`CameraStyle::factor`，四处都走它。⚠ 模板 canvas 路径曾硬编码 `1.8/1.5` 且完全不读
  `[style.camera]`，而它才是出片路径（当年 `chrome-render` 是默认路径，那条腿已删除）——别写回去。
- **三档倍率的当前值：中 1.3 / 近 1.5 / 画中画 0.55**。**放大档位不存在「推出框露背景」的风险**（绕不动点
  做 `scale`，`s ≥ 1` 四周一律向外；露底只在画中画 `zoom_wide < 1`，刻意留黑边 `(1-factor)/2×1920 = 432px`）。
  三处兜底默认值（`default_zoom_mid/close/wide()`、模板与 `engine.js` 的 `camFactor`）一起改。
  **别再退回"靠 prompt/界面拦"**：守卫是唯一兜底。
- ⚠ **竖屏（`vertical = true`）跟运镜一起屏蔽**：四条求值路径全部归零（`timeline::evaluate_frame`、
  模板 `getActiveCamera` 返回 `CFG.vertical ? 0 : handheld`、`engine.js cameraAt` 竖屏分支返回 `handheld: 0`）。
  **画中画到点收工**：`close_pip_shots` 补一条**回镜头**真事件，档位/不动点取**时间上**前一条非画中画运镜的
  **落点**；下一条在 `MIN_RETURN_SECS`(0.5s) 内就来则不补。幂等。⚠️ **前后镜头按时间算，不按文件顺序**（段之间时间上会重叠，求值器也先按时间排序再走）。
- `process` 给每个 ≥1s 且没有运镜的段补 `zoom="中", anchor="liver"`；改默认档位/锚点就在这一处
### 贴纸落点（Step 2b）与区域编辑
- `align_stickers_to_subtitles` 起点吸到**最近那条字幕**的开始、终点吸到它的结束（> `STICKER_SNAP_MAX_SECS`(2.0s) 的不动）。
  **必须排在 2b 落点之前**。⚠️ **手工放的事件不受后处理改写**（用户报过「调整贴纸的结束时间没有成功」）：
- ⚠ **人物框同时喂 Step 2b 与 2d；弹幕区域既是贴纸障碍，也是 `anchor="danmaku"` 没真实弹幕时的兜底**
  —— 别退回"人物框只影响运镜中心 / 弹幕区域仅作参考"那套加 Step 2d 之前的旧说法。
- 读取优先级：`free_rect()` → `region_rects()`/`region_origin()` 一律**自由选框 > grid > `origin_grid`/`origin`**；
  **固定落点的 UI 是"草稿态"**：必须按「保存落点」才落盘；**保存成功才退出编辑态**。
### 其他组件
- `ProjectClipsSection`（切片页卡片墙）：⚠️ **删除不是任务（不发 job），刷新只能由前端自己驱动**：它拿的是父页面
  （`saveEdit`/`addClip` 同理）。`silent` 必须真的静默；**「剪切」按钮不禁用，忙时只改文案**（占用本来就排队）。
  ⚠️ 卡片的「精修」按钮（`goPolish`）：剪切收尾已经把工程建好了，正常直接跳；**只有老片段**才补 init，
  且**必须 `await waitJobDone(id)` 再跳**（发完就跳会先撞 `polish.toml not found`；细节见 ⑥）。`polishOpening` 防连点，`jobClosers` 在 `onUnmounted` 里关订阅。
- `StickerPacks`（首页表情包区块，**仅 admin**）：⚠️ **全站第二处刻意接收上传的入口**（第一处=封面背景图）；「服务器文件建项目」那条仍**永不接受上传**，别按旧规矩删掉。
  `UPLOAD_MAX_BYTES` 前后端同数（改一处要改两处），前端先挡。
- `SliderField`：⚠ **设置页的数值项走 `components/SliderField.vue`**（滑块 + 标签行右端可直填的数字框，16 处）。
  ⚠ **滑块宽度 = 所在格子的 1/3**（`input[type=range]` 的 `width:100%` 会被 flex-basis 盖掉，**别再改回 100%**）；
- `JobList`/`jobs/JobDrawer`（任务列表与按钮门控 —— "界面哑火"的来源，2026-09-16 一次补齐）：
  - ⚠ **请求必须有超时**：`api()` 默认兜 10 min，`jobsApi` 自己传 **10s** —— 没有超时的
    `fetch` 卡住就永不 settle，而 `useJobs` 的去重 `inflight` 只由 `.finally` 清，一次卡住的
    `/api/jobs` 会把任务列表与所有按作用域门控的按钮一起冻住（症状：**"队列加载不出来 +
    剪切按钮一直灰"**，日志里一条 `/api/jobs` 都没有 —— noisy 名单只记 debug）。
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
    `crates/yekitrim-server/src/scope_key.rs` 一致，`busyOn(jobs, keys)` 求交集 —— 只在自己片段/项目被占用时
    才禁用，「别处有任务在跑」不再让按钮变灰。**同步保存类操作（选框/封面参数/字幕/去静音）不受门控。**
  - **任务行的"卡住"提示**：`JobSummary.updated_at`（epoch 秒，`Job::push` 每次事件都更新）—— 运行中但
    `IDLE_WARN_SECS`(120s) 没有新事件，就在行上写"可能卡在外部进程或网络盘上"。
  - ⚠️ **前端订阅**：`watchJob()` 返回的是 **`{ close() }` 对象**，关订阅要 `w.close()` —— 当函数调
    （`jobClose?.()`）会抛 `jobClose is not a function`，而且它就在 `jobClose = watchJob(...)` 上一行：
    抛错后**新任务永远订阅不上，界面从此停在「运行中」**。
  - 编辑器里被推迟的后处理（时间轴自动保存 / 选框保存触发的 process）：**不进队列**，只记 `pendingProcess`
    （带片段名），该片段空闲时自动补跑一次 —— 一次编辑风暴不该塞十几条任务。
- `LogConsole`：**挂载连 SSE、卸载断**，只在设置页 `monitor` 挂；颜色一律走 tokens。
  `AppModal`/`AppDialog`/`ToastHost`（`stores/toast`）—— **别再手搓弹窗/确认**。
- `ServerDirPicker`：进下一级用 `{dir, enter}`、**路径由服务端拼**。⚠ 收到 `update:dir`（onMounted 也发）要**清已选文件**；外部改 `startDir` 要跟着 browse（否则弹出选定后外层框不动）；`update:listing` 供判有无可用视频。首页内联用它，弹出用 **`ServerFileBrowser`**（双栏树+列表，草稿态不写回；树顶铺 `volumes` 盘符）。
- `RecorderConfig`：`GET /api/admin/ingest` **绝不回 `api_pass` 明文/token_hash**，只回 `api_pass_set` 布尔；
  `api_base` 非空要带 scheme。
- 其余：`HomeClipsOverview`（自动 `polish init` 传**绝对目录**）· `SubtitlePanel`/`SubtitleEditor`（`addSub()` 后按
  `start` 排序）· `prepare/SubtitleTrack`（常驻横排属性条，不要右侧竖面板）· `prepare/MaterialList`
  （**改一边等于改两边**）· `polish/StickerPicker` + `polish/SfxPicker`（`name` 必须是**去扩展名的文件名**；库数据只读）·
> 改动只允许落在本目录；跨模块改动先读对方卡，并跑两条门禁。
## 验证
```bash
npm --prefix webui run build
```
