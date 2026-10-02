# webui/src — v5 工作台前端：壳 / 路由 / 四个视图 / stores

> 范围：`webui/src`/** ｜ 上游：`webui`（构建成 `webui/dist`，由 `vtrim-server` 服务）｜ 验证：`npm --prefix webui run build`

## 职责

Vue3 工作台的全部前端：壳 + 面包屑、hash 路由、首页 `#/workbench`、四个项目视图（预处理/切片/
精修/项目设置）、`stores/` 与 `composables/`。设计依据 `docs/plan/webui-workbench-redesign.md`；
构建产物入库纪律见 `webui/AGENTS.md`。

## 入口

- `router.js`：路由表 + 旧路径 redirect + 懒加载 chunk 失效自愈。
- `utils.js`：`SETTINGS_SECTIONS`（设置页标签/副标题/路由唯一来源）、`redetectWarning()`、
  `clipMatchesEvent`、`stageInfo`、`projectEntryPath`、`STEPS`。
- `stores/`：`project.js::sourceBadge/sourceLabel`、`workbench.js`（项目列表 + per-pid 缓存 +
  `invalidate`/`forgetProject`）、`ui.js`（`sideCollapsed`/`fragRows`/`toggleSide`）。
- `composables/useJobs.js`：`load()`（单飞 + SSE 合帧）/ `onJobDone(fn)` → 返回退订函数。
- 关键组件：`MaterialList.vue`（弹幕/字幕共用清单）、`side/ProjectNode.vue`、`HomeClipsOverview.vue`、
  `StickerPacks.vue`、`SliderField.vue`、`SettingsView.vue`。

## 边界

**壳与侧栏**
- 当前项目整行高亮判据 `String(p.id) === String(route.params.pid)`（与折叠轨同一个）：
  `.proj.on .proj-head` = `--main-soft` 底色 + 左侧 3px `--main` 强调条 + 名字 `--pink-text`
  （`--main-soft` #FDE3EC 与悬停底 `--bg-soft` #FFEFE7 太接近）。
- ⚠ 选择器**必须比 `.proj-head:hover` 更特指**，否则鼠标一放上去高亮就被悬停底色盖掉；
  样式写在 `ProjectNode.vue` 的 scoped 里（样稿没有这个态），别塞回 app.css。
- 来源徽标只显示类型（`sourceBadge`）：本地来源回 **`本机`**，**不要拼路径**（服务端
  `source.label` 已含父目录/目录名，而项目名本身就是那个目录名）；完整标签 `sourceLabel(p)` 挂
  徽标的 `title`；设置页「来源」仍用完整 `sourceLabel`，两处分工别混。
- ⚠ **`.src-badge` 必须是 `flex:none`**：写成可收缩时项目名一长，盒宽被挤到 **18px** 而「本机」
  需要 **30px**，`.side-list` 又 `overflow-x:hidden` → 硬裁掉大半个字。要让位就让项目名（`.nm`，
  `flex:1 1 auto` + `min-width:0`）去截断。同理 `.proj-head .del` 也钉了 `flex:none`。
  **别再改回可收缩。**
- 阶段节点只标两件事：**当前位置**（`.snode.viewing` = `current && viewing === s.key`）与
  **已完成打勾**（`.snode.done`）。⚠ **打勾只有「预处理」拿得到**（`nodeDone` 里
  `key === 'prepare' && …`，`stage >= 3`）。**切片与精修没有真正
  意义上的"完成"**，`stageInfo().done` 那两档只是瞬时状态 —— 状态交给进度 chip
  （已切 N · 待切 M / 精修 N/M）。
- ⚠ 原来的 `cur` 档（`progress === i + 1`）**已按用户要求删除**；样稿
  `docs/web-ui-mockups/ia-demos/multiuser.html` 里有 `.snode.cur`，那是**有意弃用**的 —— 别捡回来。
- ⚠ `viewing` **必须再 `&& current`**：`SideNav` 对**每个** ProjectNode 都传同一个 `viewingStep`
  （只从 `route.name` 来，不带项目），不门控时多个项目同时展开会让同名阶段一起高亮。
- 侧栏可折叠：`.side.mini`（`width:64px`）+ `stores/ui.js` 的 `ui.sideCollapsed`/`toggleSide()`、
  localStorage `vtrim.ui.sideCollapsed`；⚠ **模块初始化时同步读取**（放 computed 里读会首帧闪
  一下展开态）。折叠轨类（`.rail-*`/`.side-toggle`/`.rdot`）写在 SideNav 的 scoped 里。
- 侧栏底部**用户块只做展示**（点它不再登出），登出是块内「退出」按钮，`doLogout()` 先
  `confirmDialog` 二次确认。**全局入口只剩 首页 / 设置**（「入库」入口已删；`#/ingest` 仍可从
  首页「录播姬录制」块进）。

**首页 `#/workbench`**
- 服务器文件（**仅 admin；非 admin 整条不显示** —— 不给点不动、只写"仅管理员"的入口）：
  `sources` 按角色过滤，`source` 用 watch 保证始终落在"看得见且可用"的那一项。
- 录播姬录制：用户**只看到「配置直播间」**；「全部记录与导入规则」「扫描录制目录」与待分配池
  全在 `<template v-if="isAdmin">` 里。不可用时整块**置灰** + 输入禁用，只给一句"录播姬未接入，
  暂时不能添加"；`roomsAvailable` 初值 **`null`** = 还没问到（避免先闪一下"未接入"）。
- 表情包区块（**仅 admin；非 admin 整块不渲染**，细节在 `crates/yekitrim-polish/src/assets/AGENTS.md`）：
  卡片墙（`.stk-grid` = `minmax(112px,1fr)`）+ 点卡片 `AppModal wide` 大图预览
  （`.stk-preview` `max-height:60vh`）；⚠ **`preview` 只存 `{pack, name}`**，文件对象由
  `previewFile` 从 `detail` 现取 —— 别存快照；`.pack-files` 框内滚动、上限 **420px**；
  **上传要前端先挡 64 MiB**（`api/stickers.js::UPLOAD_MAX_BYTES`）。

**路由**
- `#/workbench`、`#/p/:pid/{prepare,slices,polish,polish/:clip,settings}`、`#/ingest`、
  `#/settings`(admin)、`#/login`；`/` 按 localStorage 回上次项目+阶段；旧路径
  `/project/:cfg/**`、`/polish`、`/jobs`、`/models`、`/projects` **全部 redirect**。
- **懒加载 chunk 失效会自动刷一次**（`router.onError` + sessionStorage 闸门）：旧 index 点**还没
  加载过的**路由 → 旧 chunk 404 → 导航中止（"点哪个按钮都不跳转"）。检测到动态 import 失败就
  `location.reload()`；刷新后仍失败则不再刷（防死循环），任意一次导航成功即清闸门。
  **这是发版/重建 dist 后的正常现象，不是死锁。**
- ⚠ **换项目必须让视图重挂**：`App.vue` 的主 `<router-view :key="route.params.pid || route.name" />`。
  四个视图都在 `setup`/`onMounted` 抓一次 pid，同一步骤下换项目只有 `:pid` 变 → vue-router 会
  **复用组件实例**、`setup` 不再执行（用户实测线索：**跨步骤切换正常、同步骤不正常**）；
  预处理页最危险 —— 显示旧数据而 `saveTranscript(pid.value, …)` 用新 pid，**把 A 的字幕写进 B**。
  key 用 **pid** 而不是 `fullPath`（后者会让精修页换片段也整块重挂，丢掉原地切换）。
- ⚠ 路由参数 `:clip` **一律用目录名（`c.name`），绝不用绝对路径（`c.dir`）**：精修页拿它去匹配
  `clips[].name`（`GET /api/project-data/clips` 的 `name` = 目录名、`dir` = 绝对路径），传绝对
  路径永远匹配不上；跳转写 `router.push({name:'project-polish-clip', params:{pid, clip}})`（别拼
  字符串路径：目录名可能带空格 / `#` / 中文）。而 `polishMap` **按绝对 dir 索引**，判
  `polish_ready` 仍用 `dir`。

**设置页**
- 5 个子页 `/settings/<key>`（`config`/`llm`/`account`/`integrations`/`monitor`）：标签 / 副标题 /
  路由**同源于 `utils.js` 的 `SETTINGS_SECTIONS`** —— **加一页只改这一处**，别在别处再抄一份。
- ⚠ `/settings` 已降级为 **redirect 到 `config`**；`meta.admin` 与标签高亮挂在**每条子路由**上，
  **别写回 `route.name === 'settings'`**（redirect 那条永远不会被落到，照旧写会让面包屑与侧栏
  高亮一起失灵）。五块内容仍在**同一个 `SettingsView.vue`**（`defineProps({section})` + `v-if` 切），
  不拆子组件；**只有 `monitor` 才挂 `LogConsole`**（SSE 的连接/断开跟着子页走）。
- 设置页数值项走 `components/SliderField.vue`：手填的值只夹 `[min, max]`、**不吸附 step**，
  **清空/非数字不写模型**；数字框在**标签行右端**（`.lbl-row`/`.rv`）；滑块宽度 = 所在格子的
  **1/3**（`.slf-range` `flex:0 0 33.333%`）；窄格由 **`@container`** 处理，**别改成媒体查询**。

**预处理页（`PrepareView`）**
- **一个按钮、两段任务**：没转写 → 起 `stage_transcribe`，转写成功后**自动接** `stage_detect`；
  已有转写 → 直接起检测。⚠ 主按钮置灰 = `busy || (!hasTranscript && !canRunAsr)`；**已有字幕时
  它永不置灰**；两段都就绪时它变成 **「重新检测」**，走
  `confirmDialog(redetectWarning())` 二次确认，**只重跑事件检测**，**不做全流程重跑**。
- ⚠ **转写按 ASR 模式各绑一次**：产物落盘时 `assets::bind_asr_artifact` 登记进素材清单并置为
  生效字幕，那个模式的槽就用掉了（`GET …/assets` 的 `slots` 与后端同口径）；
  ⚠ **两格槽都占满时才置灰**（`canPickAsr = slotFree` + 任务运行中）—— 单格占满仍可切到另一个
  模式去跑，别写成"有产物就灰"。模式值读 merged 配置（`GET /api/project-data/config` 的
  `config.asr.mode`，读失败回退 `hybrid`），写走 `PUT /api/project-data/config {patch:{asr:{mode}}}`。
- 素材清单：**弹幕与字幕的显示/交互完全一致**（共用 `MaterialList.vue`，**改一边等于改两边**）；
  `*_file` 仍是**唯一生效值**。
- ⚠ **ASR 产物不可删**（它就是那个模式用掉的转写名额），唯一例外是**文件已不存在**
  （`removable = !is_asr || !exists`）。**「删除」只从选择里移除**（清 `*_file` + 摘清单），
  **不删磁盘文件**，走 `confirmDialog` 二次确认；**「切换」= 置为生效值**，切完整页 `load()`。
  ⚠ 条目 `exists=false` 时**照实报「文件不存在」**（置灰 + 删除线），不要静默飘到 `output/`
  兜底上。⚠ 清单读失败（`assets` 为 null）时**不要**碰 `assetDirty`。**改完不自动跑任务**；
  ⚠ `assetDirty` 只在**检测真的跑了**之后才清。
- 切片与事件的对应关系：`rank` 是检测按热度排的，**重新检测会整体重排** —— 只看目录名会让新
  事件"继承"老片段的标题。判据 `utils.js::clipMatchesEvent`（= Rust
  `cutting::foreign_clip_conflict`）：目录名相同 **且（标题相同 或 原片区间 IoU ≥ 0.5）**。
  已剪卡片**先认自己的 `clip.json`**，只有认得出事件时才跟着事件走；认不出挂 **「独立」** chip。
  未剪切事件 = 没有任何"属于自己的"片段的事件，序号被别人的旧片段占着也算未剪切
  （挂 **「序号被占用」** chip）。

**缓存与单飞**
- ⚠ **前端取数要单飞**：`stores/workbench.js::loadProjects()`（`inflight` 去重）与
  `composables/useJobs.js::load()`（在飞合并 + SSE 250ms 合帧）。视图注册 `onJobDone(fn)`
  **必须用返回的退订函数在 `onUnmounted` 里退订** —— 不退订会随反复挂载攒出一串陈旧回调，
  任务结束时同时各打一整套 project-data 请求（1 秒 52 连发的放大源）。`loadProjectData` 的
  `stale` 标记也别删：任务结束撞上在飞请求时，旧响应会盖掉新数据。
- 首页「切片总览」（`HomeClipsOverview.vue`）：全部项目的切片摊平成一份总排序（未导出/已完成
  导出两个可折叠区块），**按各片段「最后一次改动」从新到旧**（`clips[].mtime`，服务端
  `project_data.rs::last_touched_secs` = 片段目录里最新的文件时间，含 `data/` 一层；同秒回落
  项目 id 降序 + rank 升序）—— 刚剪完 / 刚精修过的浮到最前。点卡片 = 去该片段的精修页，
  没初始化就自动 `polish init`
  （`initPolish(pid, dir)` 传**绝对目录**，scope 守卫按它反查归属）。
- **任务完成后的刷新必须静默**：`load({silent:true})` 不置 `loading`、不卸载编辑器（非静默会把
  编辑器换成骨架屏，预览与滚动位置全丢）。
- 会话类前端纪律：未登录 → **401**，`api/client.js` 统一跳登录并记回跳。
- ⚠ 前端布局类改动必须做**几何断言**（`getBoundingClientRect()` 宽高 > 0）：光查 DOM 节点在不在
  会漏掉"渲染出来但零尺寸 / 被 `display:none`"这类问题。
- ⚠ **用户可见文案有长度预算（用户口径：行内 ≤20 字，硬上限 40）**：超了就把细节（生效条件、
  坐标基准、切换规则）挪进 `title` 悬停可见，行内只留"是什么 / 点什么"。右栏卡片行内容宽仅
  **266px**（`.hint`=11px、`.small`=12px）⇒ 40 字已达 480px、必然折行。别在 UI 字符串里用 `**`
  当强调（会原样显示成星号 —— 弹窗上出现过）。
- ⚠ **同一件事不说三遍**：任务名 + 状态 chip + 进度条已说清状态，别再挂「任务完成」/「运行中…」
  回显（用户要求删掉的）；**状态栏常驻**、不按任务有无出现/消失（`PolishView` 空闲态 = 「任务」+
  chip `空闲`，`jobStatusText` 里 `if (!job.id) return '空闲'` 为它加的，别删）。唯一例外是
  **失败原因必须显示**，不许静默吞错。

> 改动只允许落在本目录；跨模块改动先读对方卡，并跑两条门禁。

## 验证

```bash
npm --prefix webui run build
```

前端没有单测运行器：**这条构建就是本卡的门禁**（`vite build` 会解析模板与全部 import）。
