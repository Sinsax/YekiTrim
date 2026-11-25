# webui/src — v5 工作台前端：壳 / 路由 / 四个视图 / stores

> 范围：`webui/src`/** ｜ 上游：`webui`（构建成 `webui/dist`，由 `yekitrim-server` 服务）｜ 验证：`npm --prefix webui run build`

## 职责

Vue3 工作台的全部前端：壳 + 面包屑、hash 路由、首页 `#/workbench`、四个项目视图（预处理/切片/精修/
项目设置）、`stores/` 与 `composables/`。设计依据 `docs/plan/webui-workbench-redesign.md`；构建产物
入库纪律见 `webui/AGENTS.md`。

## 入口

- `router.js`：路由表 + 旧路径 redirect + 懒加载 chunk 失效自愈。
- `utils.js`：`SETTINGS_SECTIONS`（设置页标签/副标题/路由唯一来源）、`redetectWarning()`、
  `clipMatchesEvent`、`stageInfo`、`projectEntryPath`、`STEPS`。
- `stores/`：`project.js::sourceBadge/sourceLabel`、`workbench.js`（项目列表 + per-pid 缓存 +
  `invalidate`/`forgetProject`）、`ui.js`（`sideCollapsed`/`fragOpen`/`toggleSide`）、`theme.js`
  （**哥特浅色/暗色**，见「主题」）、`health.js`（`localNoAuth()` —— 见「本地模式」）。
- `composables/useJobs.js`：`load()`（单飞 + SSE 合帧）/ `onJobDone(fn)` → 返回退订函数。
- `composables/useOverlay.js`：弹层通用行为（初始焦点 / Tab 圈定 / **Esc 只关最上层** / 还焦点 /
  计数式滚动锁）。新增弹层直接调它，别再各写一套 keydown。
- 关键组件：`MaterialList.vue`（弹幕/字幕共用清单）、`side/ProjectNode.vue`、`HomeClipsOverview.vue`、
  `StickerPacks.vue`、`SliderField.vue`、`SettingsView.vue`。

## 边界

**简化 / 披露**
- 一级文案在 `utils.js::STEPS`（`name` 白话 / `tech` 技术名）与 `api/jobs.js::KIND_ZH`；别处不抄第二份。
- 每个能力入口带 `data-cap`，清单 `docs/webui-capabilities.json`；高级项用 `useDisclosure`/`<details>` 默认折叠，
  **隐藏 ≠ 删除**。门禁：`node webui/scripts/check-simplify.mjs --phase=P3`。
- 出片页**只有精修**（2026-11-13「不留快速切片」）：quick 那一列、`polish.quick-*` 四颗锚点、
  `ui.polishMode` / `?mode=pro` **已删**。⚠️ 判据方向相反：把 quick 加回来会翻红。
- `stores/health.js` 是 `/api/health` 的唯一缓存（`local_no_auth` 仍只认那个字段）；首页 checklist 与出片
  `dhampirStatus` 都读它，别各拉一次。
  ⚠️ `multi_user` 也只认这里：单人藏多人入口（隐藏 ≠ 删除）。

**主题**
- `stores/theme.js` 是唯一写 `<html data-theme>` 的地方，localStorage `yekitrim.ui.theme`
  （缺省 = 跟随系统）。⚠ **模块初始化时同步写 DOM**，`main.js` 在任何 import **之前**引入
  —— 放 `onMounted` 会先闪一帧亮色。入口在 `SideNav` 底部。
- ⚠ 色值一律走 tokens 两套变量（见 `styles/AGENTS.md` 双主题纪律）。

**壳与侧栏**
- 当前项目整行高亮判据 `String(p.id) === String(route.params.pid)`：
  `.proj.on .proj-head` = `--main-soft` 底色 + 左侧 3px `--main` 强调条 + 名字 `--pink-text`
  （`--main-soft` 与悬停底 `--bg-soft` 太接近）。
- ⚠ 选择器**必须比 `.proj-head:hover` 更特指**，否则悬停底色会盖掉高亮；样式写在 `ProjectNode.vue`
  的 scoped 里，别塞回 app.css。
- 来源徽标只显示类型（`sourceBadge`）：本地来源回 **`本机`**，**不要拼路径**（服务端 `source.label`
  已含父目录/目录名，而项目名本身就是那个目录名）；完整标签 `sourceLabel(p)` 挂徽标的 `title`；设置页
  「来源」仍用完整 `sourceLabel`，两处分工别混。
- ⚠ **`.src-badge` 必须是 `flex:none`**：写成可收缩时项目名一长，盒宽被挤到 **18px** 而「本机」需要
  **30px**，`.side-list` 又 `overflow-x:hidden` → 硬裁掉大半个字。要让位就让项目名（`.nm`，
  `flex:1 1 auto` + `min-width:0`）去截断。同理 `.proj-head .del` 也钉了 `flex:none`。**别再改回可收缩。**
- 阶段节点只标两件事：**当前位置**（`.snode.viewing` = `current && viewing === s.key`）与**已完成打勾**
  （`.snode.done`）。⚠ **打勾只有「预处理」拿得到**（`nodeDone` 里 `key === 'prepare' && …`，
  `stage >= 3`）。**切片与精修没有真正意义上的"完成"**，`stageInfo().done` 那两档只是瞬时状态 —— 状态
  交给进度 chip（已切 N · 待切 M / 精修 N/M）。
- ⚠ 原来的 `cur` 档（`progress === i + 1`）**已按用户要求删除**；样稿里的 `.snode.cur` 是**有意弃用**的，别捡回来。
- ⚠ `viewing` **必须再 `&& current`**：`SideNav` 对**每个** ProjectNode 都传同一个 `viewingStep`（只从
  `route.name` 来，不带项目），不门控时多个项目同时展开会让同名阶段一起高亮。
- 侧栏可折叠：`.side.mini`（`width:64px`）+ `stores/ui.js` 的 `ui.sideCollapsed`/`toggleSide()`、
  localStorage `yekitrim.ui.sideCollapsed`；⚠ **模块初始化时同步读取**（放 computed 里读会首帧闪一下
  展开态）。折叠轨类（`.rail-*`/`.side-toggle`/`.rdot`）写在 SideNav 的 scoped 里。
- 侧栏底部**用户块只做展示**（点它不再登出），登出是块内「退出」按钮，`doLogout()` 先 `confirmDialog`
  二次确认。**全局入口只剩 首页 / 设置**（「入库」入口已删；`#/ingest` 仍可从首页「录播姬录制」块进）。

**首页 `#/workbench`**
- **v7 起是单页**。顺序：检查项（v8 常驻）→ 统计条 → 继续上次 → 添加录像 → 最近片段 → 素材库。
- ⚠️ **v8：「运行状况」卡并入检查项**：待处理项行内逐项可见，页头 chip 已删。
- ⚠️ **首页不再渲染「项目」列表**：侧栏已经是项目树，再列一遍是重复信息。
- 「添加录像」= **左选来源 / 右填参数**，`.add-body` 定高；三块面板见 `docs/plan/webui-redesign-v6.md`。
  ⚠️ projects_root 留空≠不能下载（回退程序目录）。
- ⚠️ `localNoAuth`（`stores/health.js`）· 检查项口径：见 `docs/web-ui-design.md` 附录 E。
- 服务器文件（**仅 admin；非 admin 整条不显示** —— 不给点不动、只写"仅管理员"的入口）：`sources` 按
  角色过滤，`source` 用 watch 保证始终落在"看得见且可用"的那一项。
- 录播姬录制：用户**只看到「配置直播间」**；「全部记录与导入规则」「扫描录制目录」与待分配池全在
  `<template v-if="isAdmin">` 里。不可用时整块**置灰** + 输入禁用，只给一句"录播姬未接入，暂时不能
  添加"；`roomsAvailable` 初值 **`null`** = 还没问到（避免先闪一下"未接入"）。
- 表情包区块（**仅 admin；非 admin 整块不渲染**，细节在 `crates/yekitrim-polish/src/assets/AGENTS.md`）：
  ⚠ **`preview` 只存 `{pack, name}`**，文件对象由 `previewFile` 从 `detail` 现取 —— 别存快照；**上传前按
  `stickersApi.limits()` 现取上限**。布局数字见附录 F 第 6 节。

**路由**
- `#/workbench`、`#/p/:pid/{prepare,slices,polish,polish/:clip,settings}`、`#/ingest`、
  `#/settings`(admin)、`#/login`；`/` 按 localStorage 回上次项目+阶段；旧路径
  `/project/:cfg/**`、`/polish`、`/jobs`、`/models`、`/projects` **全部 redirect**。
- **懒加载 chunk 失效会自动刷一次**（`router.onError` + sessionStorage 闸门）：旧 index 点**还没加载过的**
  路由 → 旧 chunk 404 → 导航中止（"点哪个按钮都不跳转"）。检测到动态 import 失败就 `location.reload()`；
  刷新后仍失败则不再刷（防死循环），任意一次导航成功即清闸门。**这是发版/重建 dist 后的正常现象，不是
  死锁。**
- ⚠ **换项目必须让视图重挂**：`App.vue` 的主 `<router-view :key="route.params.pid || route.name" />`。四个
  视图都在 `setup` 抓一次 pid，复用了实例就不会重跑 —— 预处理页最危险（**把 A 的字幕写进 B**）。key 必须
  用 **pid**，不是 `fullPath`。来龙去脉见 `docs/web-ui-design.md` 附录 F。
- ⚠ 路由参数 `:clip` **一律用目录名（`c.name`），绝不用绝对路径（`c.dir`）**：精修页拿它去匹配
  `clips[].name`（`GET /api/project-data/clips` 的 `name` = 目录名、`dir` = 绝对路径），传绝对路径永远
  匹配不上；跳转写 `router.push({name:'project-polish-clip', params:{pid, clip}})`（别拼字符串路径：目录名
  可能带空格 / `#` / 中文）。而 `polishMap` **按绝对 dir 索引**，判 `polish_ready` 仍用 `dir`。

**设置页**
- **4 个子页**（`config`/`llm`/`integrations`/`system`）：标签/路由同源于 `SETTINGS_SECTIONS`；旧
  `account`/`monitor` 已并入 `system`（redirect）。
- ⚠ `/settings` redirect 到 `config`；`meta.admin` 与高亮挂在每条子路由上，别写回 `route.name === 'settings'`。
  四块内容同一个 `SettingsView.vue`（`v-if` 切），`LogConsole` 只在 `system` 挂。
- ⚠️ `LLM 配置`页（2026-11-26 起）分成**两个子页**（页内 tab，localStorage `yekitrim.settings.llmTab`）：
  **模型配置** = ① 当前接口（模型选择只此一处）/ ④ 模型列表 / 高级（接口管理 · extra_params）；
  **链路配置** = ② 默认参数 / 检测评分权重 / ③ 按链路覆盖表（灰字实值、点开才编辑，来自 `/api/config/llm/effective`）/ ⑤ 提示词（**自己的保存按钮**，写 llm.toml）。
  子页只影响渲染：表单状态与「保存 LLM」共用，别把两块状态拆成两套。
- 设置页数值项走 `components/SliderField.vue`：手填的值只夹 `[min, max]`、**不吸附 step**，**清空/非数字
  不写模型**；数字框在**标签行右端**（`.lbl-row`/`.rv`）；滑块宽度 = 所在格子的 **1/3**（`.slf-range`
  `flex:0 0 33.333%`）；窄格由 **`@container`** 处理，**别改成媒体查询**。

**预处理页（`PrepareView`）**
- **一个按钮、两段任务**：没转写 → 起 `stage_transcribe`，转写成功后**自动接** `stage_detect`；已有转写 →
  直接起检测。⚠ 主按钮置灰 = `busy || (!hasTranscript && !canRunAsr)`；**已有字幕时它永不置灰**；两段都
  就绪时它变成 **「重新检测」**，走 `confirmDialog(redetectWarning())` 二次确认，**只重跑事件检测**，
  **不做全流程重跑**。
- ⚠ **转写按 ASR 模式各绑一次**：落盘即占掉那个模式的槽（`assets::bind_asr_artifact`）；⚠ **两格槽都占满
  时才置灰**，别写成"有产物就灰"。模式值读**merged** 配置、写走
  `PUT /api/project-data/config {patch:{asr:{mode}}}` —— 细节见附录 F。
- 素材清单：**弹幕与字幕的显示/交互完全一致**（共用 `MaterialList.vue`，**改一边等于改两边**）；
  `*_file` 仍是**唯一生效值**。
- ⚠ **ASR 产物不可删**（`removable = !is_asr || !exists`）；**「删除」只从选择里移除，不删磁盘**；**「切换」
  = 置为生效值 + 整页 `load()`**；`exists=false` 要**照实报**（别静默飘到 `output/`）；清单读失败时**别碰**
  `assetDirty`；**改完不自动跑任务**。细节见附录 F。
- 切片与事件的对应关系：判据 `utils.js::clipMatchesEvent`（= Rust `cutting::foreign_clip_conflict`）。已剪
  卡片**先认自己的 `clip.json`**，认不出挂 **「独立」**；序号被占的挂 **「序号被占用」**。`rank` 是检测排
  的、**重新检测会整体重排** —— 只看目录名会让新事件继承老标题。细节见附录 F。

**缓存与单飞**
- ⚠ **前端取数要单飞**：`stores/workbench.js::loadProjects()`（`inflight` 去重）与
  `composables/useJobs.js::load()`（在飞合并 + SSE 250ms 合帧）。视图注册 `onJobDone(fn)`**必须用返回的
  退订函数在 `onUnmounted` 里退订** —— 不退订会随反复挂载攒出一串陈旧回调，任务结束时同时各打一整套
  project-data 请求（1 秒 52 连发的放大源）。`loadProjectData` 的 `stale` 标记也别删：任务结束撞上在飞
  请求时，旧响应会盖掉新数据。
- 首页「切片总览」（`HomeClipsOverview.vue`）：全部项目的切片混排成一份总排序，**按各片段最后一次改动从新
  到旧**（`clips[].mtime`）—— 刚剪完/刚精修过的浮到最前。点卡片 = 去精修页，没初始化就自动 `polish init`
  （`initPolish(pid, dir)` 传**绝对目录**）。排序细节见附录 F。一面墙 + 筛选 tag + 搜索（标题/字幕/弹幕）见
  `docs/webui-components.md` ⑥b。
- **任务完成后的刷新必须静默**：`load({silent:true})` 不置 `loading`、不卸载编辑器（非静默会把编辑器换成
  骨架屏，预览与滚动位置全丢）。
- 会话类前端纪律：未登录 → **401**，`api/client.js` 统一跳登录并记回跳。
- ⚠ 前端布局类改动必须做**几何断言**（`getBoundingClientRect()` 宽高 > 0）：光查 DOM 节点在不在会漏掉
  "渲染出来但零尺寸 / 被 `display:none`"这类问题。
- ⚠ **用户可见文案有长度预算（用户口径：行内 ≤20 字，硬上限 40）**：超了就把细节（生效条件、坐标基准、
  切换规则）挪进 `title` 悬停可见，行内只留"是什么 / 点什么"。右栏卡片行内容宽仅 **266px**
  （`.hint`=11px、`.small`=12px）⇒ 40 字已达 480px、必然折行。别在 UI 字符串里用 `**` 当强调（会原样显示
  成星号 —— 弹窗上出现过）。
- ⚠ **同一件事不说三遍**：状态已由任务名 + chip + 进度条说清，别再挂「任务完成」/「运行中…」回显。
  **状态栏常驻**（`jobStatusText` 的 `if (!job.id) return '空闲'` 别删）；唯一例外：**失败原因必须显示**。

> 改动只允许落在本目录；跨模块改动先读对方卡，并跑两条门禁。

## 验证

```bash
npm --prefix webui run build
```

前端没有单测运行器：**这条构建就是本卡的门禁**（`vite build` 会解析模板与全部 import）。
