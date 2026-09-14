# WebUI 工作台重构设计方案（v3 · 实施规格）

> 状态：设计定稿，待实施。交互原型：`docs/web-ui-mockups/ia-demos/b-workbench-sidebar.html`。
> 原则：**现有 WebUI 功能零回退**，只重排导航与代码结构；后端逻辑本期不动。
> 本文是实施蓝图：页面规格、状态/API/组件契约、阶段任务与验收都以本文为准。

## 0. 变更记录

| 版本 | 变更 |
|---|---|
| v1 | 方案 B 定稿：侧栏工作台 + 五段流水线 |
| v2 | 转写+检测合并为预处理；预处理页契合现有逻辑；切片未剪切事件保留折叠/放大；精修功能全保留 + 片段切换带标题；删渲染导出独立页；侧栏置顶「新建项目」；预处理页改为项目总览（字幕列表移除，字幕编辑只在精修页） |
| v3 | 细化为实施规格：页面逐卡规格、侧栏节点状态推导、store/API 契约、失效矩阵、目标文件树、阶段任务分解、风险清单 |
| v5 | 对齐预览样稿 `ia-demos/multiuser.html`：预处理=视频+字幕轨道（块点击行内编辑）+切片标记；切片未剪切事件=侧收细条/卡片式放大；精修=直接进编辑器+片段列表、生成与导出（含贴纸来源）、LLM 剪辑、无状态与保存卡（自动保存）、无 wgpu 入口、九宫格（贴纸位置）、封面设计器独立区块、产物底部整行 |
| v6 | **2026-09-14（用户要求）：九宫格 3×3 选择器 → 右栏「区域编辑」**。精修页的「九宫格布局」卡取消；右栏「生成与导出」卡改成 **LLM 剪辑块在上（参数 = 区域编辑 + 贴纸来源，动作 = LLM 剪辑 / 去静音）、渲染导出在下**。区域编辑只留**两个按钮**「人物」「弹幕区域」（无区域下拉、无「贴纸区域」——贴纸落点由 `process` 计算），点进去左侧预览进入**选框模式**（运镜暂停、缩放复位，整帧上拖框体平移 / 拖 8 个角边缩放，比九宫格精确），读数与保存/清除都在同一块内。落盘仍是 `polish.toml` 的 `[layout.<区域>]`，用 `top_left`/`bottom_right`（百分比）表达自由选框，**优先于 `grid`**；`grid` 保留为清除选框后的回退（旧配置零迁移）。保存**不再触发 process**；任务完成后的刷新改为静默（不再整页闪骨架屏）。 |
| v4 | 对齐多用户服务器计划（`docs/superpowers/specs/2026-09-12-multi-user-server-design.md`，远期）：路由参数改不透明 `:pid`、作用域解析收敛 api 层单点、client.js 错误分类与 CSRF 注入点、新建项目来源抽象、侧栏全局入口列表化、渲染导出默认项目内 `export.mp4`；新增 §11 预留设计 |

## 1. 背景与定案

现有 WebUI（Vue3 + hash 路由 + Vite，`webui/`，约 6.4k 行）的问题在**逻辑**不在视觉：流水线主线无表达、命名入口混乱（预览≠预览、/polish 撞名、/jobs 空壳）、数据聚合重复两份、任务进度 UI 两份、`?config=` 样板三处、巨石组件（TimelineEditor 843 行 / PolishView 670 行 / CoverDesigner 689 行）。

**定案**：方案 B「空间常驻导航」——侧栏树 = 项目 × 三段流水线（预处理 → 切片 → 精修），任意项目×阶段一击直达。设计语言沿用 sakura-cream token，不引 UI 框架/状态管理库/TS。

**与多用户计划的关系**：多用户服务（`docs/superpowers/specs/2026-09-12-multi-user-server-design.md`）是远期目标、未排期，本方案属于它 §0.1 所说"先做完的基础功能"。但该计划 §8 已列明前端改造清单（`?config=`→`?project=`、登录/CSRF、入库动态、StorageView/AdminView、FileBrowserModal 退场），本方案逐项预留接缝，把届时改动压到「api 层单点 + 少量视图增删」——详见 §11。

## 2. 信息架构

### 2.1 布局壳与层级

```
┌──────────┬──────────────────────────────────┐
│ 侧栏 264px│  面包屑：项目名 / 阶段名            │
│          ├──────────────────────────────────┤
│ V-TRIM   │                                  │
│ ＋新建项目│        主工作区 router-view        │
│ 项目 ▾   │   （滚动，与侧栏列表独立）           │
│  ├ 预处理│                                  │
│  ├ 切片  │                                  │
│  ├ 精修  │                                  │
│ 项目 ▸   │                                  │
│──────────│                                  │
│ 🏃 任务 ●│                                  │
│ ⚙️ 设置  │                                  │
└──────────┴──────────────────────────────────┘
```

z-index 层级（统一现有散落值）：

| 层 | z-index | 说明 |
|---|---|---|
| 侧栏 | 10 | 常驻左列，独立滚动区 |
| 任务/信息抽屉 + mask | 60 / 61 | 右滑抽屉 |
| 居中弹层（放大 modal、查看全部任务、渲染导出） | 80 | mask 点击关闭 |
| Toast / Dialog | 99 | 全局最上 |

### 2.2 侧栏规格

结构（自上而下）：

1. **Brand**（点击 → `/workbench`）+ 右侧**折叠按钮**（v6.1）
2. **「＋ 新建项目」置顶入口**：~~滚动区之外常驻~~ → **2026-09-13 改版（用户要求）**：侧栏**不再**放「＋新建项目」与「当前项目」入口；建项目移到**首页的「新增项目」区块**，按来源选择切换导入方式（服务器文件＝内嵌目录浏览器，仅 admin；录播姬录制＝待处理会话指派 + 扫描；B站链接＝P4）。首页原来的项目表格区块已删除，项目导航由侧栏项目树/面包屑承担，项目级危险操作移到 `#/p/:pid/settings`。向导按**来源类型抽象**（source kind）保留这一设计意图
3. **项目树**（滚动区）：每项目一行 + 可展开的阶段节点
4. **底部固定**：全局入口为**列表化配置**（本期 `[运行任务, 全局设置]` + 环境状态点；多用户期追加 `[入库, 存储, 管理(admin)]`，侧栏结构不变，可见性由 `/api/me.role` 驱动——见 §11）

**折叠（v6.1，用户要求）**：常驻侧栏可向左折叠成 **64px 图标轨**（`.side.mini`，`transition:width .18s`），折叠态 =
品牌 V + 折叠按钮（竖向排列）+ 项目**首字圆头像**（悬停看 `名称 · 当前阶段`，点击直达该项目当前阶段，
运行中带红点）+ 底部三个图标入口（首页/入库/设置，`title` 给文字）+ 用户头像（点击登出）。
状态在 `stores/ui.js` 的 `ui.sideCollapsed` / `toggleSide()`，持久化 `vtrim.ui.sideCollapsed`，
**模块初始化时同步读取**（否则首帧会先展开再收起，闪一下）。展开态保留过滤框与完整项目树。

项目行：`名称` + `徽标`（当前阶段名；有任务运行时附百分比，如 `预处理 42%`）。行右侧 hover 出现操作：删除（走现有 confirmDialog + `POST /api/project/delete`）。

**阶段节点状态推导**（数据全部来自 project store，不新加后端字段）：

| 节点 | done | active（●，附运行 %） | 徽标文本 |
|---|---|---|---|
| 预处理 | `status.current_stage >= 3`（events.json 存在） | 有 pipeline/stage_* 任务运行中 | `预处理` |
| 切片 | `clips.length > 0 && uncutEvents.length === 0` | 有 cut/cut_all 任务运行中 | `已切 N · 待切 M` |
| 精修 | `polishMap 中已渲染数 === clips 数 > 0` | 有 polish_* 任务运行中 | `精修 R/C` |

节点点击 → `#/p/:pid/<step>`；`.cur` = 进度位（上表第一个满足非 done 的节点），`.viewing` = 当前路由位。展开状态持久化 localStorage（`vtrim.wb.expanded`）。

空态：项目树为空时显示引导卡（「＋ 新建项目」开始）。

### 2.3 流水线三段与页面映射

| 阶段 | 页面 | 现有载体 |
|---|---|---|
| `prepare` 预处理 | 视频 + 字幕轨道 + 切片标记（§2.4） | `PreviewView.vue` → `PrepareView.vue` |
| `slices` 切片 | 切片工作区（§2.5） | `ProjectSlicesView` + `ProjectClipsSection` |
| `polish` 精修 | 编辑器（片段列表切换，§2.6） | `PolishView.vue` |

渲染导出、封面、产物保留在精修页内，不设独立阶段。

### 2.4 预处理页（PrepareView · 视频轨道工作台）

布局：左列（播放器 + 字幕轨道 + 切片标记行）+ 右列（预处理动作卡）。早期"总览三卡"（流水线状态/项目文件/环境）已按预览迭代移除——项目状态由侧栏节点徽标与首页项目列表承载，页面专注"看视频、修字幕、切片段"。

**左列 · 视频与轨道**：
- **播放器**：与精修页同构（空格播放），播原视频（`/api/media`）
- **字幕轨道（仅此一条泳道）**：字幕块直接显示文本；**点击块 = 行内编辑**（回车/失焦保存，`PUT /api/project-data/transcript`）；拖拽块调整时间
- **切片标记行**：`标记入点 → 入点 → 标记出点 → 出点 → 切片`；标记后轨道显示选区高亮；「切片」= 现有"手动添加片段"的轨道化（`POST events` → 自动剪切 → 进切片页待切列表）

**右列 · 预处理动作卡**（现有功能逐项保留）：
- 子步骤状态 chips（转写 / LLM 订正 / 事件检测）：由 `status.files` + 运行中 job 推导：`待开始` / `转写中 42%` / `✓`
- ASR 模式下拉（single/hybrid）：读取 `GET /api/project-data/config`，保存走 `PUT` patch（**删除现 `saveMode()` 前端正则改 TOML**）
- 开始转写 → `POST /api/project-data/pipeline {stage:'transcribe'}`；运行中禁用并显示进度（`useJobRun`）
- LLM 订正字幕 → `POST /api/project-data/correct`；未配置时禁用 + 引导
- 开始事件检测 → `{stage:'detect'}`；完成后 chips 点亮并自动跳切片节点
- 手动添加片段按钮（表单弹层：起止/标题/类型/详情）

不显示字幕列表，也不显示流水线状态/项目文件/环境卡。

**未来可隐藏（仅预留，不实现）**：`vtrim init` 自动跑预处理后，本页入口可收起（侧栏节点按条件跳过，配置位 `[webui] auto_prepare`，默认关，本期不读取）；运行中/未完成的预处理节点必须保留（进度可见性）。本期预处理页始终显示。

### 2.5 切片页（ProjectSlicesView + ProjectClipsSection）

布局：顶操作条 + 左切片卡片墙 + 右未剪切事件面板。

**操作条**：排序（现有四键保留：热度 rank/asc、开始时间 start/asc、弹幕量 danmaku/desc、字幕量 subtitle/desc，点击切换升降序，持久化 `vtrim.slices.sort`）、重新检测（`defineExpose` 转发保留）、添加片段、一键全切。

**切片卡**：缩略图（`/project-data/clips/:rank/thumb`）+ 标题 + 时长/开始时间 + 类型 chip + 弹幕量 + 操作行：预览（媒体弹层）、渲染（promptDialog 输出路径 → `POST /api/polish/render` job）、静音（`POST :rank/silence`）、精修（→ `polish/:clip`）。

**未剪切事件面板 · 侧收 / 卡片式放大**（现有折叠/放大交互的演进）：

| 态 | 触发 | 内容 |
|---|---|---|
| 侧收细条 | 点「收起」 | 右缘竖排细条（44px，竖排文字 `未剪切事件 (M)`），点击展开；主列卡片墙自动占满宽度 |
| 面板（默认） | 点细条展开 | 标题 + 待切 chip + 收起/放大 + 前 4 条（单切）+ 「+K 更多」 |
| 卡片式放大 | 点「放大」 | 居中弹层（720px）：事件以两列卡片展示（类型/时间/标题/单切）+ 一键全切 + 关闭 |

### 2.6 精修页（PolishView · 直接进编辑器）

**入口语义**（侧栏 polish 节点无 :clip）：

| 项目状态 | 行为 |
|---|---|
| 有切片 | **直接进入编辑器**：默认定位「精修中」片段（否则第一个），右栏「片段」列表切换；未初始化片段在列表行内「初始化」（`POST /api/polish/init`） |
| 无任何切片 | 空态引导：「还没有可精修的切片」+「去切片」按钮；不报错不白屏 |
| 深链 `polish/:clip` 无效 | 回退默认片段 + toast |

（不再有独立"片段选择器"页面——切换完全由编辑器内的片段列表承担。）

**布局**（三层）：

1. **顶部（片段条）**：`FragmentList chips` 一行 chip（编号 + 标题 + 状态 chip，未初始化带行内「初始化」），点击 = 路由 `polish/:clip`；v6.2 起由卡片墙压成 chip 行（省 ~100px 竖高）；有片段时页头不再输出 `.sub`
2. **中部**：`grid cols-main edit-grid`（`align-items:start`，右栏不跟着左列拉长）
   - **左列**（上下相邻）= 播放器（PolishPreview，右上角「人物 / 弹幕区域」框选按钮）+ **TimelineEditor**（7 条泳道 26px + 标尺 20px ≈ 202px、波形、拖拽缩放、空格播放、自动保存 polish.toml；**字幕块点击行内编辑**；标题/提示经 `<slot name="head">` 与「去静音 / 撤销去静音」同排一行，没有独立卡片头）
   - **右栏** = **生成与导出**卡片，从上到下：**LLM 剪辑块**（标题「LLM 剪辑 · 下列为参数」，参数 = **区域编辑**〔人物 / 弹幕区域两键 + 读数 + 保存/清除选框〕+ **贴纸来源**，块内动作 = `[LLM 剪辑]`）→ **`[渲染导出]`**（整宽主按钮）→ 任务框
     - **去静音只在时间轴头部**（那里带状态与「撤销」），右栏不再重复放一个（用户指出重复）
   - **预览尺寸随视口收缩**（v6.2）：`.stage` 宽度 = `min(100%, calc(clamp(170px, 100vh - 600px, 620px) * 16/9))`；600px = 面包屑 66 + 内容 padding 18 + 页头 ~80 + 片段条 ~50 + 编辑区上边距 16 + 预览卡头/控制条 ~93 + 左列间距 16 + 时间轴卡 ~251。目的：最大化窗口下**预览与时间轴同屏**（用户要求），不再写死 `66vh`；1080p 最大化（视口 ~950）→ 预览舞台 ~350px 高
   - 栅格用 `grid cols-main`（`1fr 300px`），与 v5 样稿一致；不再有「九宫格布局」卡
   - 任务完成后的刷新走 `load({silent:true})`：不闪骨架屏、不卸载编辑器（否则预览视频/滚动位置重置，看起来像浏览器刷新）；需要重取磁盘数据的子组件各自 `:key` 重挂载（TimelineEditor=`segKey`、CoverDesigner=`coverKey`、PolishPreview=`previewKey`）
3. **底部**：**封面设计器**（独立区块，功能对齐现有 CoverDesigner：舞台预览 + 时间/取当前帧/导出关键帧/导入背景图片、预设（左文/居中/右文/极简）、标题与高亮（多行标题、LLM 生成标题、高亮色、文字阴影、关键词 chips）、排版滑块（对齐/字号/X/Y/描边/行距）、背景与阴影（阴影方向/暗角/背景缩放）、封面特效叠加（光晕/模糊/暗角点/颜色覆盖/扫描线）、生成封面）；最底 **产物整行**（成片 export.mp4 + 封面 cover.jpg + 打开输出目录）

**布局框选（v6）与九宫格的关系**：

| 项 | 说明 |
|---|---|
| 区域 | 只有 **人物（liver）** 与 **弹幕区域（danmaku_area）** 两个按钮（在右栏 **LLM 剪辑参数块**的「区域编辑」里，同一个再点=退出，点另一个=换区域）；**贴纸落点不在 UI 框选** —— `layout.sticker_area` 由 `process` 计算，要改就手改 polish.toml |
| 交互 | 进入框选后左侧预览叠加选框（盒外压暗）；拖框体平移，拖 8 个角/边缩放，空白处拖拽 = 重画。此模式**暂停视频运镜并复位缩放/滤镜**，选框坐标 = 1920×1080 整帧百分比；读数与「保存选框 / 清除选框」都在右侧同一块内，预览卡头显示「选框模式 · 运镜已暂停」chip |
| 落盘 | `layout[区域].top_left / bottom_right = { x = "55%", y = "15%" }`（服务端 `Layout` 已有字段，无 schema 变更） |
| 优先级 | Rust `free_rect()` → `region_rects()` / `region_origin()` 均为**自由选框 > grid > origin_grid/origin**（运镜聚焦中心随之改变）；JS 侧 `engine.js` 的 `regionOrigin/regionRect` 与 `templates/index.html` 的 `liverOrigin` 同步同一规则 |
| 回退 | 「清除选框」删掉两个角即回到原 `grid`；`grid` 字段全程不被框选改写，旧工程零迁移 |
| 生效 | **保存不触发任务**：人物框只影响运镜聚焦中心（渲染时读取）、弹幕区域仅作参考；只有贴纸落点依赖 `process`，而它已不在 UI 内（渲染导出内部会跑 process） |

**已移除**：状态与保存卡（字幕与 segments 走自动保存，无手动保存按钮）、`真渲染预览` 按钮（wgpu 功能暂歇，界面不出现；后端 `render-frame` 接口保留）、独立片段选择器页、独立"字幕编辑"列表（编辑在轨道块上）、**九宫格 3×3 选择器卡（v6 由布局框选取代）**。

**精修功能保留清单**（重构不触碰其行为）：CoverDesigner 全部能力、贴纸来源配置、字幕编辑（轨道块行内编辑，保存走 polish 字幕接口）、SFX 预览音量（audio.js）；区域布局改为自由选框（九宫格 `grid` 仍可手改 TOML 作为兜底）。

### 2.7 设置页

**全局设置**（表单+源码双模式，沿用现有 raw/patch PUT `/api/config/global|llm`）：
- ASR：model / num_workers / chunk_duration / chunk_overlap / segment_gap / llm_correct
- 检测：vtuber_name / danmaku_window_secs（system_prompt / prompt_template 仅源码模式展示）
- LLM：base_url / api_key / model / temperature / max_tokens + 「测试连接」（`/api/llm/test`）
- 模型管理（原 /models 页并入）：三模型安装状态 + 下载（job）
- 坏配置 `.broken` 备份提示保留

**项目设置**（`GET/PUT /api/project-data/config`，同双模式）：
- 表单常用项：`output_dir` / `overwrite` / `silence_noise_db` / `detection.danmaku_window_secs`；路径字段（input/danmaku/transcript）放源码模式
- 保存前 TOML 校验（后端已有）

### 2.8 任务抽屉与弹层

- 抽屉（右滑，全局单例）：任务行 = 类型中文名（pipeline/stage_*/cut/cut_all/correct/download_models/polish_* → 中文映射表统一放 `api/jobs.js`）+ 状态 chip + 进度条 + 取消；`waiting_resume` 状态显示「继续 / 重来」（`POST /jobs/:id/decision`）
- 「查看全部」→ 居中弹层：完整历史（后端保留 50 条）
- 数据源：`useJobs`（3s 轮询 + `/api/stream` SSE 触发即时刷新）不变；`watchJob`（SSE 回放 + 轮询兜底）不变

## 3. 路由

```
#/                → redirect：localStorage vtrim.wb.last 有记录 → 上次 项目+阶段；否则 /workbench
#/workbench       → WorkbenchView（项目列表：进度/下一步/预览/打开/删除源录播 + 导入引导）
#/p/:pid/prepare  → PrepareView
#/p/:pid/slices   → ProjectSlicesView
#/p/:pid/polish   → PolishView（选择器/空态）
#/p/:pid/polish/:clip → PolishView（编辑器）
#/p/:pid/settings → ProjectSettingsView
#/settings        → SettingsView（全局）
#/login /#/storage /#/admin   （仅登记，多用户期才注册路由/建文件，见 §11）
```

| 旧路径 | 去向 |
|---|---|
| `#/`（HomeView） | 上面 redirect 规则 |
| `#/project/:cfg` | `#/p/:pid`（入口守卫：stage≥3 → slices，否则 prepare；沿用 `projectEntryPath` 语义但收敛进 `utils.nextStep()`） |
| `#/project/:cfg/preview` | `#/p/:pid/prepare` |
| `#/project/:cfg/{slices,settings}` | `#/p/:pid/{slices,settings}` |
| `#/project/:cfg/{slices/:clip,polish}` | `#/p/:pid/polish(/:clip)` |
| `#/polish`、`#/jobs`、`#/models` | 删除，404 → `/workbench` |

- **`:pid` 是不透明项目标识**：本期 = encodeURIComponent(vtrim.toml 绝对路径)（兼容现服务），多用户期 = `projects` 表 id。**组件与路由永不解释其内容**，也不拼接查询串——作用域解析只在 `api/projectData.js` 的 `scopeQ(pid)` 单点发生（本期拼 `?config=`，多用户期改 `?project=`，见 §4/§11）。
- 切换项目/阶段时写 `vtrim.wb.last`（store 订阅路由统一写，值为 pid）。
- ProjectLayout 挂载时若 pid 不在 server.toml 列表：仍按 status API 加载（沿用现 fallback 逻辑），不强制注册。

## 4. 状态层

自建模块级单例 store（与 toast/dialog 同模式），**不引 Pinia**。

```js
// stores/workbench.js —— 侧栏与全局
loadProjects()            // GET /api/project/list；任务 completed 后自动重载刷徽标
pathOf(pid)               // pid → 项目记录（含 config_file 绝对路径）——唯一的 id↔路径映射点
removeProject(cfg)        // confirm → POST /api/project/delete → reload
expanded: Set<pid>        // vtrim.wb.expanded
toggleExpand(pid) / isExpanded(pid)

// stores/project.js —— per-pid 缓存（Map<pid, store>）
useProjectStore(pid) → {
  project, status, events, clips, polishMap,
  ready, loading, error,
  loadAll({ force }={}),   // 并行 status+events+clips+polish（合并现有两份重复聚合）
  invalidate(...keys),     // 'status'|'events'|'clips'|'polishMap'
  // computed：uncutEvents（events - 已切）、stageInfo（三段状态/徽标文本，供侧栏与总览共用）
}

// api/projectData.js 内部 —— 作用域解析单点（多用户期只改这里，见 §11）
scopeQ(pid) → `config=${encodeURIComponent(workbench.pathOf(pid).config_file)}`  // 本期
//          → `project=${pid}`                                                    // 多用户期
```

**失效矩阵**（`useJobs` SSE `completed` 事件驱动；进行中不刷新避免撕裂）：

| job.kind | invalidate | 受影响 UI |
|---|---|---|
| `pipeline` / `stage_extract|transcribe|detect` | status, events | 侧栏徽标、总览状态卡、切片页事件数 |
| `correct` | status | 总览 chips |
| `cut` / `cut_all` | clips, events, status | 切片卡片墙、未剪切面板、侧栏徽标 |
| `polish_init|llm|process` | polishMap | 精修状态 chips、侧栏徽标 |
| `polish_render|cover` | polishMap | 产物卡、侧栏徽标 |
| `download_models` | workbench 环境区 | 设置模型区、侧栏环境点 |

localStorage 收编（旧 key 原样兼容，不迁移）：`vtrim.wb.last`（新）、`vtrim.wb.expanded`（新）、`vtrim.slices.sort`（沿用）、音量 key（polish/audio.js 内部，不动）。

## 5. API 层

```
api/client.js       不动（SPA 回退陷阱防护已有）
api/projectData.js  新增：cfgQ(cfg) + status/events(GET|POST|PUT)/clips/thumb(:rank)/
                    transcript(GET|PUT)/correct/pipeline/config(GET|PUT)/polish
api/polish.js       polish/api.js 迁入：initPolish/runLlm/runProcess/runRender/
                    fetchProject/fetchSubtitles/saveSubtitles/saveProjectPatch/
                    selectVersion/fetchStickerSources/GRID_CELLS/REGION_LABELS/watchJob
                    —— 删除死代码 fetchProjects()；startJob 内部 scopeQ 化
                    （`dir` 参数多用户期改 {project, dir}，见 §11）
api/jobs.js         useJobs/watchJob 共用：list/detail/cancel/decision/中文类型映射
```

切片卡「预览」：改跳 `polish/:clip`（Vue 自绘预览）；`POST /api/polish/preview` 后端保留（CLI 用）。

**client.js 错误分类与注入点**（本期实现映射，行为中性；多用户期直接复用）：

| 状态 | 处理 |
|---|---|
| 401 | 跳 `#/login`（记回跳）——本期（未接入认证）不会发生；服务端 P1 起启用该分支 |
| 403 | toast 越权/权限文案 |
| 409 | 直出后端原因文案（现：任务占用；多用户期含磁盘/水位原因） |
| 429 | toast 排队已满（多用户期才会出现） |
| CSRF | 预留注入点：`api()` 统一入口处加 `X-CSRF-Token`（本期 no-op 空实现，多用户期单点启用） |

## 6. 组件拆分与目标文件树

```
webui/src/
├─ api/            client.js · projectData.js · polish.js · jobs.js
├─ stores/         workbench.js · project.js · toast.js · dialog.js
├─ composables/    useJobs.js · useJobRun.js · useTimelineGestures.js
├─ router.js       utils.js（+nextStep）
├─ App.vue         # AppShell：SideNav + router-view + JobDrawer + Toast + Dialog
├─ components/
│  ├─ side/        SideNav.vue · ProjectNode.vue
│  ├─ jobs/        JobDrawer.vue（JobList 迁入+查看全部弹层）
│  ├─ timeline/    TimelineEditor.vue（骨架）· WaveformLane.vue · SubtitleLane.vue · MuteLane.vue
│  ├─ polish/      GenerateExportCard.vue（生成与导出·含贴纸来源与布局框选）· FragmentList.vue ·
│  │               CoverDesignerSection.vue · ~~GridNine.vue~~（v6 取消，框选叠加层在 PolishPreview 内）·
│  │               ArtifactBar.vue（底部产物整行）· PolishPreview.vue · TimelineEditor 装配
│  ├─ slices/      ProjectClipsSection.vue · ClipCard.vue · UncutEventsPanel.vue
│  └─ common/      Icon · AppModal · AppDialog · ToastHost · FileBrowserModal · NewProjectModal · Steps
└─ views/          WorkbenchView.vue · PrepareView.vue · ProjectSlicesView.vue（壳）·
                   PolishView.vue · ProjectSettingsView.vue · SettingsView.vue
```

拆分纪律：**纯搬运**（模板/逻辑原样移动，props/emits 最小化），TimelineEditor 拆分前后用手测清单比对（拖拽缩放、拆分行、空格播放、自动保存）。`engine.js`/`audio.js` 与 Rust 模板的双维护关系不变（不在本次范围）。

## 7. 后端配合（本期零逻辑改动）

- `AGENTS.md` 补「WebUI」章节（`web.rs:7` 注释正引用它）：webui/ 工程、路由表、dev（vite 代理 /api→7788）/build（dist 三级候选）流程。
- 预留配置位 `[webui] auto_prepare`（默认关，本期不读取）——登记于本文 §2.4，未来 init 自动预处理用。
- 后续可选另立任务：`/project/*` 双轨收敛、错误 envelope 统一、api.rs 按域拆模块。

## 8. 实施阶段（每步可独立交付、build+手测通过再进下一步）

| # | 阶段 | 任务 | 触及文件 | 验证 |
|---|---|---|---|---|
| 1 | 布局壳+侧栏树 | App.vue → AppShell；SideNav/ProjectNode（置顶新建入口、节点推导、展开持久化）；路由重排+redirect 表；删 HomeView/PolishProjectsView/JobsView/ModelsView；JobDrawer | App.vue、router.js、side/*、jobs/*、views/-4 | ✅ 已实施（提交 `f9c44e0`） |
| 2 | store+API | workbench/project store、api/{projectData,jobs,polish}、失效矩阵接线、localStorage | stores/*、api/*、useJobs.js | ✅ 已实施（同上） |
| 3 | 预处理轨道工作台 | PreviewView→PrepareView：播放器+字幕轨道（块点击行内编辑）+切片标记行；预处理动作卡与 chips；ASR patch 保存（删 saveMode 正则） | views/PrepareView.vue、utils.js | ✅ 已实施（提交 `05b4748`；新增 `components/prepare/SubtitleTrack.vue`，后端补 `PUT /api/project-data/config {patch}`） |
| 4 | 切片页 | 接 store；UncutEventsPanel 侧收细条/卡片式放大；ClipCard 抽取；预览跳 polish/:clip | slices/*、ProjectClipsSection | ✅ 侧收/放大/排序/单切/全切已在 `ProjectClipsSection` 内实现（预览仍走媒体弹层，与 v5 样稿一致）；组件未再拆分为 ClipCard/UncutEventsPanel |
| 5 | 精修页 | 直接进编辑器+FragmentList（行内初始化）；GenerateExportCard（含贴纸来源）；CoverDesignerSection；GridNine；ArtifactBar 底部整行；空态引导 | polish/*、views/PolishView.vue | ✅ 已实施（提交 `05b4748`，仍在 PolishView 单文件内，未拆子组件） |
| 6 | TimelineEditor 拆分 | 三泳道 + useTimelineGestures 纯搬运 | timeline/* | ⏳ 未做（纯搬运重构，不影响 IA；`TimelineEditor.vue` 原样保留） |
| 7 | 设置页 | 全局/项目双模式表单；模型管理并入；useJobRun 统一任务 UI | views/Settings*、composables/useJobRun.js | ✅ 模型管理早已并入 `SettingsView`（`ModelList`）；未新增 `useJobRun`（各页内联 watchJob） |
| 8 | 文档 | AGENTS.md WebUI 章节；本文标记已实施 | AGENTS.md、本文 | ✅ 已更新 |

## 9. 验收清单

- [ ] 侧栏任意项目×阶段一击直达；刷新/深链保持原位（含 `polish/:clip`）
- [ ] 「＋ 新建项目」置顶入口任何时刻可见可用
- [ ] 直接点精修节点：有切片直接进编辑器（片段列表切换/行内初始化），无切片进空态引导，永不白屏
- [ ] 任务完成 → 侧栏徽标 + 工作区数据自动刷新（无整页重拉、无重复聚合请求）
- [ ] 预处理轨道工作台：字幕块点击行内编辑；标记入点/出点后切片进待切列表；动作卡与现有功能一致
- [ ] 切片：排序四键、未剪切事件侧收细条/卡片式放大、单切/全切、静音、渲染
- [ ] 精修：片段列表切换/行内初始化、LLM 剪辑自动链 process/渲染导出/去静音/**布局框选（v6：预览选框模式 + 保存/清除，运镜暂停）**/封面设计器/时间轴全部交互；自动保存无手动保存按钮
- [ ] 旧深链 `#/project/...` 全部 redirect 正常
- [ ] 作用域单点：除 `api/projectData.js` / `api/polish.js` 外，无任何文件拼接 `?config=` 或 `dir=`（多用户迁移面验证）
- [ ] `npm run build` 零错误；`cargo build` 0 warning

## 10. 风险与边界

| 风险 | 缓解 |
|---|---|
| TimelineEditor 拆分引入回归 | 纯搬运纪律 + 阶段 6 独立手测清单；不顺手改逻辑 |
| 单活动任务 409：任务运行中用户触发其他动作 | `useJobRun` 统一禁用 + 「查看任务」按钮跳抽屉 |
| SSE 断连 | useJobs 已有 3s 轮询兜底，不动 |
| 大转写文件渲染卡顿 | 字幕列表已移出预处理；精修时间轴沿用现状（虚拟滚动列为后续可选） |
| 多标签页同开 | 无锁；沿用现状（单用户本地工具），不处理 |
| 旧 localStorage key | `vtrim.slices.sort` 原样沿用，无迁移 |
| engine.js 双维护 | 不在本次范围，保持「先改 Rust 模板再移植」约定 |
| wgpu 真渲染 | 功能暂歇：界面无入口，后端 render-frame 接口与 feature 保留 |
| 多用户迁移（远期 P1–P5） | 已按 §11 预留：不透明 pid、scopeQ 单点、入口列表化、来源抽象、渲染输出默认收敛；本期不实现任何多用户行为 |

## 11. 多用户对齐（预留设计）

对应 `docs/superpowers/specs/2026-09-12-multi-user-server-design.md`（**远期、未排期**；本方案属于其 §0.1 "先做完的基础功能"）。原则：多用户改造只应触及「api 层 + 少量视图增删」，**信息架构与组件全部不动**——侧栏树仍是"我的项目列表"，单用户本机＝只有一个账号的部署，语义自洽。

> 多用户期的**完整前端设计**（登录/入库/B站向导/存储/管理页、M1–M5 阶段与验收）见配套文档 `docs/plan/webui-multiuser-frontend.md`；本节只登记接缝与改动面。

| 多用户计划 | 本方案已预留的接缝 | 到时改动范围 |
|---|---|---|
| P1 账号/登录/CSRF（§3） | `#/login` 预留路由；App 启动第一个请求位（`GET /api/me`：200 进工作台 / 401 跳登录；**无免登录分支**）；client.js 401 分支 + CSRF 注入点（本期 no-op） | 新增 LoginView；client.js 启用注头 |
| P2 `?config=`→`?project=`（§5.2/§8） | 路由用不透明 `:pid`；`api/projectData.js` 的 `scopeQ(pid)` 单点解析；`workbench.pathOf(pid)` 唯一映射 | 改 `scopeQ` 一处 + `loadProjects` 端点（`/api/project/list`→`/api/projects`）+ `api/polish.js` 的 `dir`→`{project,dir}` |
| P2 删除本地路径入口（§5.2） | 渲染导出默认已是项目内 `export.mp4`（本地模式才允许改）；手动添加片段本就是项目内相对量 | 删渲染导出 promptDialog 的自定义路径分支 |
| P2 FileBrowserModal 退场（§8.3） | 新建向导按 source kind 抽象（§2.2） | 来源列表加「入库记录 / B 站链接」，移除文件浏览来源 |
| P3 入库动态/房间映射（§4） | 侧栏底部全局入口为列表化配置（预留「入库」项）；WorkbenchView 预留「入库动态」区块位 | 新增 IngestView（或总览区块组件）+ channel-map 管理卡 |
| P5 StorageView / AdminView（§8.4） | 预留路由 `#/storage`、`#/admin`；侧栏入口可见性由 `/api/me.role` 驱动 | 新增两个视图 + 删除源录播按钮（项目设置/总览） |
| 任务分槽/排队 429（§6） | `useJobRun` 已统一任务 UX；client.js 429/409 文案位 | 任务行加"排队中"状态展示；文案接入 |
| `/api/health` 收敛（§5.3） | 总览环境卡按 chip 消费（ffmpeg/LLM/Chrome/模型就绪位），不依赖路径字段 | 无 |
| `/api/stream` 按 owner 过滤（§6） | 前端只是订阅者，过滤在服务端发生 | 无 |

**明确不预留**：多租户树结构（扁平"我的项目"语义不变）、每用户自带 key/模型、计费/OAuth——这些是后端侧事务，前端无对应面。
