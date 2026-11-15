# YekiTrim Web UI 设计文档

> 版本: v0.2（设计稿·修订） · 日期: 2026-09-03 · 状态: 待评审
> 范围: 全流程管理台（项目 / 模型 / 管线监控 / 事件与切片 / 配置），polish 精修仅保留跳转入口。
> 技术: 零构建 vanilla JS + axum 0.7，代码位于 `crates/yekitrim-cli/src/server/`。
> **设计标准: [`docs/web-ui-mockups/sakura-cream.html`](web-ui-mockups/sakura-cream.html)（樱花奶油 · 浅色）** —— 全站视觉、布局、组件语言以该样稿为唯一基准，第 4 章 Design Tokens 全量取自其 `:root` 定义。

---

## 修订说明（v0.1 → v0.2）

- **主题反转**：正式 UI 由「暗色（与 polish 编辑器一致）」改为**纯浅色樱花奶油**，以 sakura-cream.html 为设计标准（原 4.4 节「浅色→暗色映射」作废，暗色主题移出范围）。
- **信息架构变更**：左侧导航 → **顶部 pill 导航**（总览 / 项目 / 模型 / 精修 / 设置），运行监控降级为 topbar 铃铛唤起的全局抽屉。
- **架构对齐现状**：子命令 `yekitrim server`、代码位于 `crates/yekitrim-cli/src/server/`、默认端口 26320（v0.1 规划的 `yekitrim serve` + 新 crate `yekitrim-web` + 端口 3927 作废，M1 已按 spec 落地）。
- **保持不变**：功能范围、多项目工作区扫描模型、API 契约、状态机、checkpoint 决策机制、17 事件类型色相。

### 与 spec 的差异与对齐

spec（`docs/superpowers/specs/2026-09-03-yekitrim-webui-design.md`，已批准、M1 进行中）与本设计的差异点：

| # | 差异点 | 本设计（v0.2） | spec 现状 | 对齐方向 |
|---|--------|----------------|-----------|----------|
| 1 | 项目模型 | **工作区扫描多项目**（P2 卡片网格 + `#/projects/:id` 详情） | 单活动 Workspace + `POST /api/project/switch` | spec 需后续修订：补工作区扫描 → 多项目端点（M1 范围，见 11 章） |
| 2 | 代码位置 / 命令 / 端口 | `yekitrim server` · `crates/yekitrim-cli/src/server/` · 26320（跟随已落地实现） | 同 | 已一致 |
| 3 | 视觉主题 | 浅色 sakura-cream（本文档第 4 章为视觉权威） | spec 未定义视觉 | 一致（spec 无冲突） |
| 4 | polish 深度 | UI 层仅跳转入口；API 层支持 process/render/cover job | spec 含完整 polish API | 已落地 API 按 spec 保留，UI 呈现按本文档 |

---

## 1. 背景与目标

### 1.1 现状

YekiTrim 目前只有 CLI（`yekitrim`）。用户完成一轮录播切片时，需要在进程间来回切换：查看 `events.json`、`publish_config.json`、翻 `output/clips/` 目录、运行 `yekitrim status`、手动重跑某个阶段。polish 编辑器（`yekitrim polish preview`）提供了浏览器内的精修体验，但只针对**单个 clip 的编辑**，不覆盖管线管理和多项目工作流。

### 1.2 目标

1. **管理台**: 一个浏览器页面管理全部项目（工作区扫描）、全部模型、全部管线任务。
2. **可视化进度**: 管线运行时按阶段显示进度（ASR 块数、剪切条数）、实时日志、checkpoint 断点续传决策，从"黑盒 CLI 输出"变为可跟踪的任务队列。
3. **结果审阅**: 事件表格、切片卡片、海报帧、字幕预览、视频播放，全部在浏览器内完成。
4. **配置零手写**: `yekitrim.toml` / `.yekitrim/config.toml` / `.yekitrim/llm.toml` 表单化，不再手写 TOML。
5. **零构建**: 纯 vanilla JS（ES modules + hash router），静态资产编译期内嵌。

### 1.3 非目标

- ❌ 不做 polish 精修的编辑器化（时间轴/特效编辑）——只提供跳转 `yekitrim polish preview` 的入口。
- ❌ 不做多用户 / 鉴权——服务绑定 `127.0.0.1` 本机使用。
- ❌ 不做云端 / 远程渲染。
- ❌ 不做事件手工拖拽时间轴编辑（本期只做 enabled 开关、重新检测；时间轴编辑留给 polish）。
- ❌ 不做暗色主题——sakura-cream 浅色为唯一主题（暗色未来可选，不在本期）。

### 1.4 已确认约束

- 范围: 全流程管理台，polish 精修只留跳转入口。
- 交付: 本设计文档（页面清单 + 每页 ASCII 线框 + 组件交互说明 + API 契约）。
- 技术: 零构建 vanilla JS + axum，代码位于 `crates/yekitrim-cli/src/server/`。
- 主题: **浅色樱花奶油（sakura-cream 标准）**，中文界面。
- 设计标准: `docs/web-ui-mockups/sakura-cream.html`（token 行号引用见第 4 章来源列）。

---

## 2. 技术决策与理由

| # | 决策 | 理由 | 替代方案（未选） |
|---|------|------|------------------|
| D1 | `yekitrim server` 子命令，代码位于 `crates/yekitrim-cli/src/server/` | M1 已按 spec 落地（`service/` + `server/` 模块已存在），CLI 与 HTTP 共用 Service 层避免逻辑双份 | 独立 crate `yekitrim-web`（v0.1 规划，作废——CLI binary 内模块够用，避免 workspace 膨胀） |
| D2 | axum 0.7（与 polish 版本一致） | 复用同一 toolchain 与 `tower-http ServeDir`；保持全仓 dep 版本单一 | axum 0.8（polish 未升级，不引入第二个 axum 版本） |
| D3 | 静态资产编译期内嵌（`include_str!`） | `yekitrim_home()` 依赖 exe 父目录定位，内嵌最稳，无需运行时找资源路径 | 磁盘资源目录（exe 位置决定路径，易坏） |
| D4 | 工作区根目录扫描 = 项目发现 | 单项目场景自然退化（workspace = 项目父目录）；多项目零注册表 | 显式项目注册文件（多一个状态源，用户维护负担） |
| D5 | 项目运行 = 进程内 JobManager（每项目互斥 + tokio::spawn），进度经 `ProgressReporter` 实现 → tokio broadcast → SSE | 复用进程内状态、可取消；`ProgressReporter` 是 core 已定义的同步 trait，桥接成本最低 | 子进程方式（stdout 解析、进程管理、跨平台信号，成本高） |
| D6 | 阶段状态完全复用 `ProjectConfig::current_stage()` 文件存在性推导 | 与 CLI 状态机 100% 一致，杜绝"UI 认为完成但 CLI 拒绝"的分叉 | UI 自维护阶段状态（必然漂移） |
| D7 | checkpoint 续跑决策由 UI 给出 → 注入 `PipelineContext.detect_resume: Option<bool>` | `detect_resume` 字段 core 已存在（`types/mod.rs:163`），无缝对接 | UI 只显示"有 checkpoint"（丢失决策能力） |
| D8 | 媒体路由 `/api/media/*path`（safe_join 白名单：工作区根 / 工程输出目录 / 输入视频父目录） | `<video>` 直连文件无需转码；与已落地 `media.rs` 一致，防止 `../` 逃逸 | 无限制静态挂载（暴露工作区外文件） |
| D9 | 17 种事件类型 → 色卡 + 图标映射表（沿用 polish `ed-block.type-*` 色相），**浅色渲染 = 类型色 α12% 底 + 同色文字徽章** | 保留跨工具识别一致；浅色底上实底高饱和色会压住文字，soft 底 + 深字保持可读 | 沿用 polish 暗色实底色卡（浅色下对比失衡） |
| D10 | 前端 hash router（`#/projects/:id/events`） | 零构建、刷新不 404、支持后退；组件库以 `window.UI.*` 导出 | history API（需要服务端回退到 index） |

**关键代码对接点**（实现时直接引用）:

- `ProgressReporter` trait — `crates/yekitrim-core/src/progress/mod.rs:1-6`（4 个同步方法: `stage_started` / `stage_progress` / `stage_completed` / `error`）
- `PipelineContext.detect_resume` — `crates/yekitrim-core/src/types/mod.rs:163`（`Some(true)`=继续，`Some(false)`=重来，`None`=自动）
- `ProjectConfig::current_stage()` — `crates/yekitrim-core/src/config/types.rs:35-68`
- Runner 构造模式 — `crates/yekitrim-cli/src/main.rs:278-282`（`PipelineRunner::new(cfg)` + `set_progress` + `add_stage` + `run`）
- `process` 全流程编排 — `crates/yekitrim-cli/src/main.rs:289-454`（extract → transcribe → llm_correct → detect → cut，web 端 Job 需镜像）
- Stage 常量 — `crates/yekitrim-core/src/pipeline/stage.rs:5-8`（`STAGE_AUDIO=1` / `STAGE_TRANSCRIBE=2` / `STAGE_DETECT=3` / `STAGE_CUT=4`）

---

## 3. 信息架构与导航

```
YekiTrim WebUI（顶部 pill 导航 · topbar 常驻 · 浅色樱花奶油）
├ 总览      #/overview                   P1
├ 项目      #/projects                   P2（多项目卡片网格）
│            └ 项目详情 #/projects/:id    P3（核心页 · 5 Tab）
│               ├ /overview      概览
│               ├ /events        事件管理
│               ├ /clips         片段库
│               ├ /transcript    字幕转录
│               └ /settings      项目设置
├ 模型      #/models                     P6
├ 精修      #/polish                     P7
├ 设置      #/settings                   P5
└ 🔔 任务   铃铛唤起全局抽屉              P4（#/jobs 全屏兜底路由）
```

- 导航层级不超过 2 层（topbar pill → 页面 / 页面 → Tab）。
- topbar 5 个 pill 与 sakura-cream 样稿一致：**总览 / 项目 / 模型 / 精修 / 设置**；激活项 = 粉色渐变胶囊。
- P4 运行监控**不是导航项**：由 topbar 右侧 🔔 铃铛唤起全局抽屉（红点角标 = 运行中任务数），同时保留 `#/jobs` 独立路由全屏查看。
- P3 项目详情为核心页，5 个 Tab 之间路由直达: `#/projects/:id/overview | events | clips | transcript | settings`。

---

## 4. 视觉规范（Design Tokens）

> 全站唯一主题 = **浅色樱花奶油**。下列 token 全量取自 [`sakura-cream.html`](web-ui-mockups/sakura-cream.html) 的 `:root` 定义（来源列 = 样稿行号），实现时直接复制为 `app.css` 的 CSS 变量，**不得另行取色**。

### 4.1 色板

**背景 / 表面**

| Token | 值 | 用途 | 来源 |
|-------|-----|------|------|
| `--bg` | `#FFF7F3` | 页面底（奶油白偏暖粉） | sakura:14 |
| `--bg-soft` | `#FFEFE7` | 次底（pill hover 底、chip n 底、hover 行底） | sakura:14 |
| `--card` | `#FFFFFF` | 卡片 / 胶囊导航容器 / 按钮底 | sakura:14 |
| `--line` | `#F2E1D9` | 描边（卡片、输入框、topbar 下边框） | sakura:15 |
| `--line-soft` | `#FAEEE7` | 浅描边 / 虚线分隔（卡内行分隔、steps 轨道） | sakura:15 |

**文字**

| Token | 值 | 用途 | 来源 |
|-------|-----|------|------|
| `--ink` | `#4A3F45` | 主文字（暖深灰） | sakura:16 |
| `--ink-dim` | `#9C8B92` | 次级文字 | sakura:16 |
| `--ink-faint` | `#C9B8BE` | 最浅文字（路径、小注、占位） | sakura:16 |
| （品牌名） | `#E86FA5` | topbar 品牌名专用色 | sakura:40 |

**主色（樱花粉）**

| Token | 值 | 用途 | 来源 |
|-------|-----|------|------|
| `--main` | `#F682B8` | 主色（激活 pill、now 步进点、主按钮渐变起端） | sakura:17 |
| `--main-deep` | `#EE6FA6` | 主色深（渐变末端、mini-btn hover 字色、丝带字色） | sakura:17 |
| `--main-soft` | `#FDE3EC` | 主色柔和底（chip p、丝带底、sticker pink 底） | sakura:17 |
| `--main-glow` | `#FFD3E1` | 主色发光（✦ 装饰、steps 完成连线、"新建卡"虚线框） | sakura:18 |

**点缀色系**（各含 base + soft 双值，来源 sakura:18-20）

| Token | 值 | Token | 值 | 用途 |
|-------|-----|-------|-----|------|
| `--blue` | `#7FA8F0` | `--blue-soft` | `#E8EEFC` | 信息 / 辅强调（chip b、sticker blue） |
| `--mint` | `#59C0AC` | `--mint-soft` | `#E2F6F1` | 清新点缀（sticker mint） |
| `--amber` | `#F2B23E` | `--amber-soft` | `#FCF0D9` | 运行中 / 等待（chip 下载中） |
| `--green` | `#4FBF9C` | `--green-soft` | `#DEF2EA` | 完成 / 成功（steps done、chip g） |
| `--red` | `#E8727E` | `--red-soft` | `#FBE3E5` | 错误 / 危险（错误 toast、🔔 红点） |

**阴影**

| Token | 值 | 用途 | 来源 |
|-------|-----|------|------|
| `--shadow` | `0 8px 26px rgba(238,111,166,.10), 0 2px 6px rgba(90,60,70,.04)` | 卡片常态（粉调双层） | sakura:22 |
| `--shadow-hover` | `0 12px 34px rgba(238,111,166,.18), 0 3px 8px rgba(90,60,70,.05)` | 卡片 hover / 按钮 hover | sakura:23 |

**chip 色值**（sakura:96-98，base 底 + 加深同系文字）: `chip.g` 底 `--green-soft` / 字 `#2FA07E` · `chip.p` 底 `--main-soft` / 字 `#D8669B` · `chip.b` 底 `--blue-soft` / 字 `#5E7FD8` · `chip.n` 底 `--bg-soft` / 字 `--ink-dim`。

### 4.2 字体与字号

| Token | 值 | 用途 |
|-------|-----|------|
| `--font` | `'Noto Sans SC','Microsoft YaHei',sans-serif` | 全站正文（sakura:24；与 polish 同一字体栈口径体栈） |
| `--mono` | `'Consolas','Courier New',monospace` | 文件路径、时间码、数据、日志台（sakura:25） |

字号体系（sakura 实测值）: 页标题 26px/700 · 品牌名 17px/700 · 卡片标题 16px/700 · 正文/按钮 13px · 次级 12px · chip/路径 11px · 小注 10-12px。

### 4.3 圆角 / 间距 / 纹理

- 圆角: `--r: 16px`（卡片、抽屉、模态、toast）、`--r-s: 10px`（内部小块、输入框）；**按钮、chip、pill、头像一律 `999px` 全圆胶囊**（sakura:21, 42-47, 61, 96）。
- 间距基数 4px: `4 / 8 / 12 / 20 / 28`；卡片 padding `22px 22px 18px`（sakura:71）；网格 gap 20px（sakura:70）；主区 `max-width 1180px` + padding `30px 28px 90px`（sakura:56）。
- 全局纹理: `body::after` 固定定位的 26px 网格粉色圆点（`radial-gradient(rgba(246,130,184,.10) 1.5px, transparent 1.6px)`，sakura:30-32），`pointer-events:none`、`z-index:0`，内容区 `z-index:1`。
- 贴纸语言: 图标块 52px 方块、16px 圆角、`rotate(-5deg)` 微旋转、2px dashed 彩色描边 + soft 底（pink/blue/mint/amber 四色，sakura:79-84）。

### 4.4 状态色语义

| 语义 | Token | 场景 |
|------|-------|------|
| 完成 / 成功 | `--green` / `--green-soft` | steps done 对勾、chip g、模型已安装 |
| 当前 / 强调 | `--main` / `--main-soft` | 激活 pill、now 步进点（带 4px 粉色光环）、主按钮 |
| 运行中 / 等待 | `--amber` / `--amber-soft` | Job running、模型下载中 |
| 错误 / 危险 | `--red` / `--red-soft` | Job failed、错误 toast、🔔 红点角标 |
| 信息 / 中性 | `--blue` / `--blue-soft` · `--bg-soft` / `--ink-dim` | chip b（弹幕/热度）、chip n |

**事件类型徽章（浅色渲染规则）**: 17 种类型的色值沿用 polish 编辑器 `ed-block.type-*` 色相（保持跨工具识别一致，色相**不翻转**），浅色呈现为: 徽章底 = 类型色 12% 透明度，文字 = 同类型色；对浅色底对比不足的类型（如 `#0891b2` 起类）文字加深 15%。类型 → 色值映射表见 6.3.2。

> **暗色主题已移出范围**：v0.1 的「浅色→暗色映射」约定作废，本文档不再提供暗色 tokens。polish 编辑器（独立工具）保持自身暗色主题，与本 WebUI 互不影响。

### 4.5 交互效果

- 卡片 hover: `translateY(-3px)` + 阴影 `--shadow` → `--shadow-hover`，`transition: all .18s`（sakura:73）。
- 按钮 hover: `translateY(-1px)` + 阴影加深；`btn-primary` 带 `rgba(238,111,166,.40)` 粉色发光阴影（sakura:63-66）。
- mini-btn hover: 描边变 `--main`、字变 `--main-deep`、底变 `--main-soft`（sakura:101-103）。
- pill hover: 底变 `--bg-soft`；激活 pill: 粉渐变 + 白字 + 发光阴影（sakura:45-47）。
- 新建卡 hover: 虚线转实线、边框变 `--main`、底变 `--bg`（sakura:107）。
- topbar: `position:sticky` + `rgba(255,247,243,.85)` + `backdrop-filter:blur(10px)` 毛玻璃吸顶（sakura:35-36）。
- 无入场动画（保持样稿静态克制的调性）；唯一例外: 进度条 `width` transition `.2s`。

---

## 5. 全局布局线框

```
┌──────────────────────────────────────────────────────────────────┐
│ topbar (sticky · 毛玻璃 blur(10px) · 下边框 --line)               │
│ ┌───────┐  YekiTrim     [ 总览 | 项目 | 模型 | 精修 | 设置 ]   🔔 P │
│ │ V logo│  切片管理台        (白色胶囊容器 · 激活项粉渐变)        │
│ └ 38px  ┘  (#E86FA5)                                            │
├──────────────────────────────────────────────────────────────────┤
│ .wrap (max-width 1180px · 居中 · padding 30px 28px 90px)         │
│  ┌ page-head ─────────────────────────────────────────────────┐ │
│  │ 项目 (12)                              [🔄 扫描] [＋ 新建] │ │
│  │ 工作区: D:\media (mono · --ink-faint)                      │ │
│  └─────────────────────────────────────────────────────────────┘ │
│  💡 warn-line: 提示文案 (12px · --ink-faint)                      │
│  ┌─────────────────────────────────────────────────────────────┐ │
│  │                     <router-view>                            │ │
│  │                 （P1–P7 各页内容 · z-index:1）                │ │
│  └─────────────────────────────────────────────────────────────┘ │
└──────────────────────────────────────────────────────────────────┘
```

布局要点:

- **topbar**: 品牌（38px 圆形粉渐变 logo `linear-gradient(135deg,#F682B8,#FBB9D4)` + 白字，右侧「YekiTrim 切片管理台」品牌名 `#E86FA5`）· 居中 pill 导航容器（白底 999px 胶囊 + 激活项粉渐变胶囊）· 右侧 🔔 铃铛（白色圆钮 + 红点角标 = 运行中任务数，点击唤起 P4 抽屉）+ 蓝色渐变圆形头像（装饰）。
- **主区 `.wrap`**: 全站共享 `page-head` 模式（h1 26px + sub 行内 mono 工作区路径 + 操作区 `btn`/`btn-primary` 胶囊按钮）+ `warn-line` 提示行。
- **toast 系统**: 右上滑入，`999px` 胶囊 + 白底 + 描边变体（ok=`--green` / err=`--red`），3.5s 自动消失。
- **响应式**: 卡片网格 3 列 → ≤900px 2 列 → ≤600px 1 列（`repeat(auto-fill, minmax(300px,1fr))` 亦可）；窄屏下 pill 导航容器横向滚动。

---

## 6. P1–P7 逐页详设

> 每页「API」标注为该页全部交互所需接口（契约见第 8 章）。线框内 `[B]`=按钮，`[C]`=卡片，`[T]`=Tab，`[I]`=输入框。未特别说明的视觉一律按第 4/5 章 sakura tokens。

### P1 总览 `#/overview`

```
┌───────────────────────────────────────────────────────────────┐
│ 总览                             [🔄 刷新] [＋ 新建项目 btn主]  │
├───────────────────────────────────────────────────────────────┤
│ ┌─[C] 环境健康 (白底 · 16px 圆角 · --shadow) ────────────────┐ │
│ │ [chip g ffmpeg ✓] [chip g ffprobe ✓] [chip g sensevoice ✓] │ │
│ │ [chip n firered-zh 缺失] [chip g qwen3-asr ✓]               │ │
│ │ [chip n LLM 未配置] [chip g Chrome ✓] [chip g Polish ✓]     │ │
│ │  └ 点击 chip → 跳 #/models 或 #/settings-LLM                │ │
│ └─────────────────────────────────────────────────────────────┘ │
│ ┌─[C pcard 迷你] ────────────┐ ┌─[C pcard 迷你] ─────────────┐ │
│ │ [sticker 📺] 3d 录播        │ │ [sticker 🎮] new_game_0921  │ │
│ │ media/3d/直播回放.mp4 (mono)│ │ media/newgame/… (mono)      │ │
│ │ [steps 迷你 ✓✓✓●]          │ │ [steps 迷你 ✓●○ ○]          │ │
│ │ [chip g Stage4] [chip p 9片]│ │ [chip n Stage2 转写中]       │ │
│ │ ──────── dashed ────────── │ │ ──────── dashed ──────────  │ │
│ │ 2h14m33s · 1080p            │ │ 1h22m08s · 4.4GB             │ │
│ │ [👁 mini打开] [▶ mini运行▾]│ │ [👁 mini打开] [▶ mini继续]   │ │
│ └────────────────────────────┘ └─────────────────────────────┘ │
│ (最多 4 张，按 yekitrim.toml mtime 排序；点击卡片 → P3)            │
│ ■ 快速操作: [btn 新建项目] [btn 下载缺失模型] [btn 修正全部转写] │
└───────────────────────────────────────────────────────────────┘
```

**组件**: `HealthCard`（环境健康卡，含状态 chip 组）、`ProjectMiniCard`（迷你 pcard + Steps 迷你 + chips）、`QuickActions`。

**交互**（与 v0.1 一致）:

- 环境卡来自 `GET /api/health`（ffmpeg/ffprobe 版本探测、三模型 marker 检查、LLM 连通探测、Chrome/Polish 可用性）。点击 chip 跳 P6 / P5-LLM。
- 最近项目按 `yekitrim.toml` 最后修改时间排序；`Steps` 显示 `current_stage()` 推导阶段，点击进入 P3。
- 快捷操作: 「下载缺失模型」批量 POST 模型下载（复用 P6）；「修正全部转写」= 对每个 stage≥2 的项目 POST correct job（确认框）。
- 「运行 ▾」下拉: 单阶段 / 全流程（对应 CLI `extract|transcribe|detect|cut|process`）。

**API**: `health` · `projects` · `models` · `jobs/start`

---

### P2 项目列表 + 新建向导 `#/projects` —— **设计基准页**

> 本页即 sakura-cream.html 样稿本身。实现时**直接以样稿 `sakura-cream.html:70-110` 的 `.pcard` 规范为模板**，只将示例数据替换为 `GET /api/projects` 的真实数据。

```
┌───────────────────────────────────────────────────────────────┐
│ 项目 (12)                        [🔄 扫描工作区] [＋ 新建项目]  │
│ 工作区: D:\media                                              │
│ 💡 删除项目只移除 yekitrim.toml，不会触碰媒体文件                   │
├───────────────────────────────────────────────────────────────┤
│ ┌─[C pcard]───────────┐ ┌─[C pcard]──────────┐ ┌─[C pcard]──┐ │
│ │      ┌已完成┐ 丝带  │ │ [sticker 🎮]        │ │ [sticker🍜]│ │
│ │ [sticker 📺]        │ │ new_game_0921      │ │ mukbang    │ │
│ │ 3d 录播             │ │ media/newgame/…    │ │ media/…    │ │
│ │ media/3d/直播回放.mp4│ │ steps ✓●○○         │ │ steps ○○○○ │ │
│ │ steps ✓✓✓●          │ │ [chip n 转写中]    │ │ [chip n 待]│ │
│ │ [chip g 已剪切]     │ │ [chip b 弹幕✅]    │ │ [chip p GB]│ │
│ │ [chip p 9片段]      │ │ ──── dashed ────── │ │ ── dashed ─│ │
│ │ [chip b 热度0.72]   │ │ 1h22m · 4.4GB      │ │ 3h05m · GB │ │
│ │ [chip n qwen3-asr]  │ │ [👁][▶ 继续]       │ │ [👁][▶ 运行]│ │
│ │ ──── dashed ─────── │ └────────────────────┘ └────────────┘ │
│ │ 2h14m33s · 1080p    │                                       │
│ │ [👁 打开][▶ 运行▾]  │   …（示例 5 张 + 更多）                 │
│ └─────────────────────┘                                       │
│ ┌─[C pcard.new 虚线卡]────────────────────────────────────────┐│
│ │              (＋) 52px 粉色圆钮                               ││
│ │              新建项目 · 选择视频·自动发现弹幕                 ││
│ └─────────────────────────────────────────────────────────────┘│
└────────────────────────────────────────────────────────────────┘
```

**项目卡（`.pcard`）规范** —— 全部来自样稿:

- 结构（自上而下）: 贴纸图标 → h3 项目名 → mono `path` → `steps` 4 阶段 → `chips` 状态行 → dashed 分隔底栏（时长/体积 + mini-btn 组）。
- **贴纸**: 52px 方块、16px 圆角、`rotate(-5deg)`；四色（pink/blue/mint/amber）按项目名哈希稳定分配，emoji 图标按项目内容（📺🎮🍜🎤🌸…）。
- **丝带**（`.ribbon`）: 仅 stage=4 显示「已完成」；`rotate(38deg)` + 右出血 + `--main-soft` 底 `--main-deep` 字。
- **steps**: done = 绿底对勾（`--green-soft`/`--green`）、now = 粉色圆点 + 4px `--main-soft` 光环、wait = 白底虚线描边圆；连接线 6px 高圆角条，完成段 `--main-glow`。
- **chips**: `[chip g Stage N 状态]` `[chip p 片段数]` `[chip b 弹幕 ✅/❌ 或 热度 0.72]` `[chip n ASR 模型]`。
- **底栏**: 左侧 `2h14m33s · 1080p`（11px `--ink-faint`），右侧 mini-btn「👁 打开」「▶ 运行/继续/启动 ▾」（▾ 展开 = 单阶段菜单）。
- **新建卡**（`.pcard.new`）: `border: 2px dashed var(--main-glow)` + 居中 52px 粉色圆形 ＋ 号 + 引导文案；hover 虚线转实线。

**新建向导流程**（功能与 v0.1 一致，视觉 = 模态卡）:

```
步骤 1: [FileBrowserModal] 选择视频
        ├ 导航: 工作区根 / 同级目录 / 上级 / 收藏
        ├ 列表: [📁 目录] [🎬 media/3d/直播回放.mp4]
        └ 搜索框过滤: *.mp4|*.flv|*.mkv  ✓自动过滤
步骤 2: 自动发现
        ├ 同名 .ass / .xml / .srt? → [✅ 弹幕: 直播回放.ass] (可改)
        ├ 输出目录: ./output (可改)
        └ [初始化] → POST /api/projects → 跳项目详情
```

**交互**:

- 项目卡片: 阶段 badges + 片段数 + 弹幕/字幕/LLM 状态（文件存在性推导）；「重跑」→ 确认框选阶段；「删除」仅删 `yekitrim.toml`（**明确提示不会删除媒体文件**）。
- 工作区路径变化 → 自动重扫（`GET /api/projects`）。
- 空态: `EmptyState`（虚线卡风）+ [新建项目] + [设置工作区]。

**API**: `projects` · `projects/create`（即第 8.2 节 `POST /api/projects`）· `fs/list` · `settings/workspace`

---

### P3 项目详情（核心页） `#/projects/:id`

页面 Header 常驻: 贴纸 + 项目名 h1 + mono 视频路径 + **大号 `Steps` 4 阶段** + [btn 运行 ▸] [btn-primary 全流程]。

胶囊式 Tab 栏（同 pill 导航语言）: `[概览][事件][片段库][字幕转录][项目设置]`。

#### Tab 1 概览 `(#overview)`

```
┌─ [T] [概览][事件][片段库][字幕转录][项目设置] ──────────────────┐
│ ┌─[C] 视频信息 ─────────┐ ┌─[C] 产物文件清单 ────────────────┐ │
│ │ 视频: (mono 路径)      │ │ ① audio: video.wav   [chip g ✓] │ │
│ │ 时长 02:14:33 · 1920×1080│ ② transcript: video.srt [chip g ✓]│ │
│ │ h264/aac               │ │ ③ events: events.json [chip g ✓]│ │
│ └────────────────────────┘ │ ④ clips: 9 个/32:41  [chip g ✓] │ │
│ ┌─[C] 4 阶段步进器 (大号) ─┴─────────────────────────────────┐ │
│ │ ┌阶段卡① 提取音频┐  ┌阶段卡② 转写┐ ┌阶段卡③ 检测┐ ┌阶段卡④ 剪切┐│ │
│ │ │ sticker 低模     │ │ [chip g 完成]│ │ [chip g 完成]│ │ [chip g完成]│ │
│ │ │ 产物 16kHz ✓     │ │ [重跑 mini]  │ │ [重跑 mini] │ │ [重跑 mini]│ │
│ │ └─────────────────┘ └──────────────┘ └─────────────┘ └───────────┘│
│ │ 每个阶段卡: 名称/状态 chip/产物文件/时间戳/[重跑][dry-run]         │
│ └────────────────────────────────────────────────────────────┘ │
│ ┌─[C] ASR 元数据 ───────┐ ┌─[C] 最近任务 ────────────────────┐ │
│ │ model: qwen3-asr       │ │ [chip g Transcriber 12:01:33 ✓] │ │
│ │ anchors: 120 · 覆盖率  │ │ [chip p Detect 12:02:01 ⚠ 失败] │ │
│ │ max_interp_span: 0     │ └─────────────────────────────────┘ │
│ │ [重新转写(改模型) btn] │                                       │
│ └────────────────────────┘                                      │
└────────────────────────────────────────────────────────────────┘
```

**交互**（与 v0.1 一致）:

- Steps（大号）: 点击某阶段 → 改配置并重跑；每阶段卡 tooltip 显示等价 CLI（如 `yekitrim extract -i …`）。
- 「dry-run」: 触发 `run_until(max)` 预览 → 监控抽屉显示将执行的阶段与产物预估。
- 产物文件清单: 点开 → 侧边预览（SRT/JSON 原文，JSON 格式化）。
- ASR 元数据来自 `transcript.json` 的 `asr_metadata`，无字段显示「未记录」。

**API**: `projects/:id` · `projects/:id/transcript` · `jobs/start` · `jobs/:id/dry-run`

#### Tab 2 事件管理 `(#events)`

```
┌─ [I 🔍 搜索] [类型 ▾] [☑ 仅启用] [🔁 重新检测] [📤 导出 publish_config] ┐
│ ┌─[C 白底圆角卡 · 内表格] ────────────────────────────────────────────┐ │
│ │ # │ 事件标题        │ 类型徽章        │ 时段(mono)│ 热度    │ ⚙ │ │
│ │ ── dashed ─────────┼─────────────────┼───────────┼─────────┼───│ │
│ │ 1 │ 🎮 开局 20 杀   │ [gameplay 浅色] │ 00:01:33  │ ▓▓▓ 0.72│ ☁ │ │
│ │ 2 │ 💬 弹幕问答     │ [interaction]   │ 00:12:10  │ ▓▓ 0.61 │ ☁ │ │
│ │ …（行 hover: --bg-soft；点击行 → 详情抽屉）                          │ │
│ └────────────────────────────────────────────────────────────────────┘ │
│ 详情抽屉 (右侧 480px · 16px 圆角 · --shadow):                           │
│ ┌──────────────────────────────────────────────────────────────────┐   │
│ │ 🎮 开局 20 杀              [编辑标题] [☑ 启用] [🎞 预览切片]      │   │
│ │ 时段 00:01:33 ~ 00:03:12 (1m39s)                                  │   │
│ │ popularity 0.72 · confidence 1.0 · source LLM                     │   │
│ │ 摘要: …, 前 30 秒连续击杀后进入发育期…                            │   │
│ │ 类型: [gameplay ▾ 可改] (重新检测时保持手工修改)                   │   │
│ └──────────────────────────────────────────────────────────────────┘   │
└───────────────────────────────────────────────────────────────────────┘
```

**17 种事件类型 → 色卡 + 图标映射表**（色值沿用 polish `ed-block.type-*`，浅色渲染见 4.4）:

| 类型 | 色值 | 图标 | 类型 | 色值 | 图标 |
|------|------|------|------|------|------|
| `gameplay` | `#2563eb` | 🎮 | `music` | `#0891b2` | 🎵 |
| `interaction` | `#7c3aed` | 💬 | `food` | `#d97706` | 🍜 |
| `qna` | `#059669` | ❓ | `rant` | `#dc2626` | 😡 |
| `reaction` | `#db2777` | ⚡ | `break` | `#4d7c0f` | 🚻 |
| `storytelling` | `#9333ea` | 📖 | `outro` | `#525252` | 👋 |
| `chat` | `#0d9488` | 🗨 | `eating` | `#ea580c` | 😋 |
| `irl` | `#c026d3` | 📷 | `ranking` | `#ca8a04` | 🏆 |
| `discussion` | `#6d28d9` | 🗣 | `intro` | `#3b82f6` | ⭐ |
| `other` | `#64748b` | 🎭 | | | |

**交互**（与 v0.1 一致）:

- 类型徽章可改（下拉），修改即 `PUT /projects/:id/events`（同时写 `titles` 映射，见 8.3 说明）。
- 「重新检测」: 若存在 `events.checkpoint.json` → 弹出 checkpoint 决策对话框（见 P4）；否则直接 POST job。
- 摘要展开 → 抽屉内完整摘要 + metadata。
- 「🎞 预览切片」→ 跳 Tab 3 并定位该 clip。
- 表格底栏: 分页/总数；已启用 N / 总 M。

**API**: `projects/:id/events` GET+PUT · `jobs/start`（detect）· `projects/:id/export`

#### Tab 3 片段库 `(#clips)`

```
┌─ [I 🔍 搜索] [排序: 热度▾/时长/时间] [全部|已发布] ───────────────┐
│ ┌─[C ClipCard pcard 风]────────────────────────────────────────┐ │
│ │ ┌────────────────────────┐  🎮 开局 20 杀                    │ │
│ │ │    [🎞 海报帧]          │  (事件类型浅色徽章)                │ │
│ │ │    (白边圆角 · 抽帧)    │  时长 1m39s · [热度 ▓▓▓ 0.72]     │ │
│ │ └────────────────────────┘  file: clip-01/clip.mp4 (mono)    │ │
│ │ ──────── dashed ───────────────────────────────────────────  │ │
│ │ [▶ mini预览] [🧹 mini静音压缩] [↑↓ mini发布顺序]               │ │
│ └──────────────────────────────────────────────────────────────┘ │
│ …（卡片网格 3 列；点击卡片 → 详情抽屉 560px）                       │
│ ┌─ [C] 详情抽屉 ──────────────────────────────────────────────┐  │
│ │ title / event_type / rank / popularity / original_start…    │  │
│ │ [▶ 视频播放器 <video>]  [🎬 clip.srt 预览]  [📄 clip.json]   │  │
│ └─────────────────────────────────────────────────────────────┘  │
│ 底栏: [全选] [发布顺序持久化 → publish_config.json] · 共 9 个      │
└──────────────────────────────────────────────────────────────────┘
```

**组件**: `ClipCard`（海报帧 = ffmpeg 抽帧缓存 `_thumb.jpg`）、`ClipDetailDrawer`、`SilenceCompressAction`、`OrderUpDown`。

**交互**（与 v0.1 一致）:

- 海报帧: 首次打开调 `GET /projects/:id/clips/:rank/thumb`（服务端 ffmpeg 抽帧缓存到 clip 目录 `_thumb.jpg`）。
- 「静音压缩」= `yekitrim silence` 单文件等价: `POST /projects/:id/clips/:rank/silence`（job）→ 产出 `clip.silent.mp4` 同目录，完成后卡片标注「已压缩 · 体积 -32%」。
- 发布顺序 ↑↓ = 修改 `clips_order.txt` + `publish_config.json` 顺序（PUT `projects/:id/order`）。
- 「▶ 预览」: 抽屉内 `<video>` 播放 `/api/media/…`（8.11）。
- 抽屉内字幕/弹幕预览: 拉 `clip.srt` / `clip.dm.srt` 文本渲染为时间轴列表（点击跳转视频时间）。

**API**: `projects/:id/clips` · `projects/:id/clips/:rank` · `projects/:id/clips/:rank/thumb` · `projects/:id/clips/:rank/silence` · `projects/:id/order` PUT

#### Tab 4 字幕转录 `(#transcript)`

```
┌─ [I 🔍 过滤] [🕐 时间轴: 全览] [🔁 重新转写] [🤖 LLM 订正] [重置] ┐
│ ‹── 时间轴渲染: 每段 = --main 粉色圆角条 (宽=时长, 悬停 tooltip) ──› │
│ ┌─[C] ────────────────────────────────────────────────────────┐ │
│ │ 00:00:01,500 │ (confidence 0.93) │ 大家好, 欢迎来到今天的直播… │ │
│ │ 00:00:04,800 │ (0.91)             │ 今天玩一个新出的游戏      │ │
│ │ …（行 hover: --bg-soft）                                     │ │
│ ├────────────────────────────────────────────────────────────┤ │
│ │ 🤖 LLM 订正状态: [chip g 已完成]  │ 订正文件: video_llm.srt   │ │
│ │ 原字: "我们私奔" → 订正: "我们开演" │ [查看 diff] [从 checkpoint │ │
│ │                                   │  继续 (若有)]            │ │
│ └────────────────────────────────────────────────────────────┘ │
│ 底部: [保存 SRT] [导出 transcript.json] · 共 342 段 · 02:14:33   │
└─────────────────────────────────────────────────────────────────┘
```

**交互**（与 v0.1 一致）:

- 时间轴 bar: 悬停 tooltip 显示文本；点击跳转视频预览（复用 `VideoPlayer`）。
- 「🤖 LLM 订正」→ job（等价 `correct_transcript_with_checkpoint`）；有 `correct.checkpoint.json` 且当前 SRT 匹配 → 决策对话框（同 P4）。
- 「重置为默认」= 从 `video.srt` 恢复（LLM 订正文件是副本，不删除原始）。
- diff 视图: 原始 vs 订正对照（side-by-side，高亮差异）。

**API**: `projects/:id/transcript` · `jobs/start`（correct 带 checkpoint）· `projects/:id/transcript/reset` · `projects/:id/transcript/export`

#### Tab 5 项目设置 `(#settings)`

```
┌─ [C] yekitrim.toml 表单化 (保存后写文件, 保留注释) ─────────────────┐
│ [I 📄 输入视频] [I 📄 弹幕] [I 📄 转写] [I 📄 音频] [I 📄 事件]  │
│ ──── 手动路径输入 (带 FileBrowser 下拉) ──────────────────────── │
│ [detection] danmaku_window_secs: [I 10.0]                        │
│ [cutting]   output_dir: [I ./output] · overwrite: [☐] ·         │
│             silence_noise_db: [I -30.0]                          │
│ ─────────────────────────────────────────────────────────────── │
│ » 错误提示: input_file 为空 (--red 文字, 保存前校验) ──────────── │
│ [保存 btn主] [重置为默认] [打开 yekitrim.toml 原文]                  │
├──────────────────────────────────────────────────────────────────┤
│ 注意: [asr] 等全局配置在「设置」页 P5 编辑（此处只显示项目级字段） │
└──────────────────────────────────────────────────────────────────┘
```

**交互**（与 v0.1 一致）: 字段与 `ProjectConfig` 一一对应（`input_file`/`danmaku_file`/`transcript_file`/`audio_file`/`events_file` 均可为空）。保存 = PUT `projects/:id/config`；「重置」= 恢复内建默认。保存后前端立即刷新 `GET /projects/:id`（阶段推导可能变化）。

**API**: `projects/:id/config` GET+PUT

---

### P4 运行监控（全局抽屉 / 覆盖层） `#/jobs`

> 唤起: topbar 🔔（红点角标 = 运行中任务数）；抽屉右侧滑入 560px；`#/jobs` 全屏兜底路由。

```
┌─ 任务抽屉 (右侧滑入 560px · 白底 · 16px 圆角 · --shadow) ────────┐
│ ⚙ 运行监控 · 2 运行中 · 0 排队 · 1 失败     [清空] [I ⌕ 过滤]    │
│ ┌─ Job 卡片 (白底圆角卡) ──────────────────────────────────────┐ │
│ │ [chip p 3d录播] 全流程 process · 开始 12:01:30 (mono) · 4m12s│ │
│ │ [✕ 取消 mini]                                                │ │
│ │ [steps ①✓ ②● ③○ ④○]  ▓▓▓▓▓▓▓░░░ 62% (75/120 块) (粉渐变 6px)│ │
│ │ │ 日志台 (高 180px · --bg-soft 奶油底 · mono 12px · 自动滚底):│ │
│ │ │  12:03:11 INFO  [Transcriber] chunk 45/120 done (ink-dim) │ │
│ │ │  12:03:12 WARN  qwen3-asr 0.6B · chunk 46/120 (--amber)   │ │
│ │ └─ [⏸ 暂停][✕ 取消][🗎 复制日志] mini-btn 组                 │ │
│ └─────────────────────────────────────────────────────────────┘ │
│ ┌─ Job: 检测 (重新检测) ───────────────────────────────────────┐ │
│ │ [chip p 3d录播] 检测 · 等待输入 (events.checkpoint.json 存在) │ │
│ │ ⚠ 发现断点续传: 上次运行到事件 #7/12                          │ │
│ │ [▶ 继续上次] [🔄 从头重来] [✕ 取消任务]                        │ │
│ └─────────────────────────────────────────────────────────────┘ │
└─────────────────────────────────────────────────────────────────┘
```

**组件**: `JobCard`、`Steps`（4 阶段并联）、`LogConsole`（奶油底 + 级别着色 + 虚拟滚动 ≤5000 行）、`CheckpointDialog`、`CancelButton`。

**Job 生命周期**（服务端 JobManager，与 v0.1 一致）:

```
queued → running(逐 stage 推进) → completed
                         ↘ failed(错误附加在日志帽)
                         ↘ cancelled(用户/程序退出)
running 中若 stage 触发 checkpoint → waiting_resume → (UI 决策) → running / restarted
```

- 每项目**互斥**: 同项目同时只允许 1 个 job；新请求入队或拒绝（默认入队，用户可"插队"）。
- 取消: `Stage::run` 不接收取消信号（core 现状），因此取消形态为 **丢弃整个 tokio 任务**（任务按项目互斥，安全；后续重跑按文件存在性幂等推进）。
- checkpoint 决策: `detect` 与 `correct` 会写 `*.checkpoint.json`；job 启动时若存在 → 状态 `waiting_resume`，暂停在队列头等待 UI 决策（POST `/jobs/:id/resume`）。
- 日志: `tracing` 全局 Layer/Subscriber 桥接 → 每 job 一个 `mpsc::UnboundedSender`；同时保留文件日志 `.yekitrim/logs/web-YYYYMMDD.log`。
- 断线处理: SSE 断开 job 不取消（服务端继续）；重连后 `GET /api/jobs?since=` 增量补拉。

**API**: `jobs` GET · `jobs/start` POST · `jobs/:id/cancel` POST · `jobs/:id/resume` POST · `stream` SSE

---

### P5 全局设置 `#/settings`

```
┌─ [T] [ASR] [检测] [剪切] [SFX] [LLM] [工作区] (胶囊式) ─────────┤
│ ■ ASR ──────────────────────────────────────────────────────── │
│ [模型 ▾ sensevoice|firered-zh|qwen3-asr]  [num_workers 0=自动]  │
│ [chunk_duration 60.0] [chunk_overlap 10.0] [segment_gap 1.0]   │
│ [☑ llm_correct]                                                 │
│ ────────────────────────────────────────────────────────────── │
│ ■ LLM 连接 ─────────────────────────────────────────────────── │
│ base_url [I https://api.openai.com/v1]  api_key [I •••• 显示▾] │
│ model [I gpt-4o]  temperature [I 0.1]  max_tokens [I 65536]    │
│ extra_params [I JSON 编辑框, 校验] { "seed": 42 }               │
│ [保存] [测试连接 → [chip g ✓ 0.9s] / [chip p ✗ 超时]]           │
├────────────────────────────────────────────────────────────────┤
│ ■ 工作区 ───────────────────────────────────────────────────── │
│ 根目录: [I D:\media]  [更改 ▸ FileBrowser]    │
│ 扫描结果: [chip b 12 项目] · 示例: media/3d/yekitrim.toml (mono)   │
│ [立即重扫]                                                       │
└────────────────────────────────────────────────────────────────┘
```

**规则**（与 v0.1 一致）: 合并显示 `.yekitrim/config.toml`（全局）+ `.yekitrim/llm.toml`；保存分两写。`extra_params` 为 JSON 文本编辑保存前 parse 校验。路径字段均可用 FileBrowser。表单均以白底圆角卡分组（`FormField`: label `--ink-dim` + 白底圆角输入 + mono 数值字段）。

**API**: `settings/global` GET+PUT · `settings/llm` GET+PUT · `settings/workspace` GET+PUT · `llm/test` POST · `fs/list`

---

### P6 模型管理 `#/models`

```
┌─ 模型 (自动从 .yekitrim/models/ 扫描 + registry 对照) ─────────────┐
│ ┌─[C pcard] sensevoice ──────────────────────────────────────┐ │
│ │ [sticker 粉 🔉] sensevoice  [chip g ✅ 已安装]              │ │
│ │ 165 MB · 模型: sense-voice-int8-2025-09-09 (mono)           │ │
│ │ 文件: tokens.txt ✓ model.int8.onnx ✓ lexicon.txt ✓ …(chips)│ │
│ │ 说明: [SenseVoice 快] 中文单语转写 · 无 token 级时间戳       │ │
│ │ ──── dashed ─────────────────────────────────────────────  │ │
│ │ [升级下载 mini] [重新验证 mini]                               │ │
│ └─────────────────────────────────────────────────────────────┘ │
│ ┌─[C pcard] firered-zh ──────────────────────────────────────┐ │
│ │ [sticker 蓝 📀] firered-zh  [chip n ⏳ 下载中 42%]          │ │
│ │ 218/520 MB · 82 KB/s (断点: 上次 218 MB 已存) (mono)       │ │
│ │ ▓▓▓▓▓▓▓░░░░░░░░░░░ (6px 粉渐变进度条)                      │ │
│ │ ──── dashed ─────────────────────────────────────────────  │ │
│ │ [⏸ 暂停 mini] (暂停=中止进程, 保留 .part)                    │ │
│ └─────────────────────────────────────────────────────────────┘ │
│ ┌─[C pcard] qwen3-asr ───────────────────────────────────────┐ │
│ │ [sticker 薄荷 🎙] qwen3-asr  [chip n ❌ 未安装]             │ │
│ │ 879 MB · 最准最慢 (0.6B int8 中文, 有 token 时间戳)          │ │
│ │ ──── dashed ─────────────────────────────────────────────  │ │
│ │ [⬇ 下载 btn主]                                               │ │
│ └─────────────────────────────────────────────────────────────┘ │
│ 说明: 下载断点续传 — 中断后 .part 文件保留, 再次启动从 0 续传;    │
│       完成后自动解压并删除压缩包. 建议: 转写前升级到 qwen3-asr.   │
└─────────────────────────────────────────────────────────────────┘
```

**交互**（与 v0.1 一致）: 卡片 = `GET /api/models`（registry 对照，marker 检测）；下载 job = `POST /api/models/:name/download` 走 JobManager；进度经 SSE `model.progress`。三模型体积/说明来自 registry 常量（sensevoice ~165MB / firered-zh ~520MB / qwen3-asr ~879MB）。「暂停」= 取消 job（保留 `.part`，可续）。

**API**: `models` GET · `models/:name/download` POST · `jobs/:id/cancel`

---

### P7 精修工作台 `#/polish`

```
┌─ 精修工作台 (仅跳转; 编辑在现有 polish 编辑器) ──────────────────│
│ [扫描工作区] · 规则: 含 polish.toml 的目录 = 精修项目             │
│ ┌─[C pcard] clip-01 (3d录播) ────────────────────────────────┐ │
│ │ [sticker 琥珀 🎬] clip-01                                   │ │
│ │ ▶ 素材: clip.mp4 · clip.srt · clip.dm.srt · clip.json (mono)│ │
│ │ 状态: [chip n 未处理] [chip g 已渲染] [chip g 封面 ✔]        │ │
│ │ 视频: vtuber-clip.mp4 ✔ · cover.jpg ✔ · 已精修段 6          │ │
│ │ ──── dashed ─────────────────────────────────────────────  │ │
│ │ [⚙ 处理 mini] [▶ 渲染 mini] [🖼 封面 mini]                  │ │
│ │ [👁 预览 mini] [🎬 打开编辑器 mini主]                        │ │
│ └─────────────────────────────────────────────────────────────┘ │
│ 注: 「处理/渲染」为长任务 → P4 监控; 「预览/编辑器」跳转现有        │
│     yekitrim polish preview (新端口打开)                             │
└─────────────────────────────────────────────────────────────────┘
```

**组件**: `PolishCard`（pcard 风）、`StatusChip`（processed/rendered/cover 三态）。

**交互**（与 v0.1 一致）: 扫描 = 递归查找 `polish.toml`（限工作区）。状态推导: `processed` 字段 + 输出文件存在性（`project.output` 存在 → 已渲染；`cover.jpg` 存在 → 封面完成）。操作按钮映射 polish 命令（`cmd_process` / `cmd_render` / `cmd_cover` 的 web 封装，以 job 运行）。「打开编辑器」→ 调用 `yekitrim polish preview` 启动逻辑（内部 server 新端口）并新开 tab。

**API**: `polish/projects` GET · `polish/:id/process|render|cover` POST · `polish/:id/preview` POST（返回 URL）

---

## 7. 组件库清单

### 7.1 sakura 基础组件（样稿直接落地）

| 组件 | 用途 | 主要页面 | 关键 props |
|------|------|---------|------------|
| `PillNav` | topbar 胶囊导航（白底容器 + 粉渐变激活项） | 全局 | `items[]`, `active` |
| `PCard` | 白底 16px 圆角卡 + 粉调阴影（hover 上浮 3px） | 全局卡片基底 | `tone?` |
| `Sticker` | 贴纸图标块（52px · rotate(-5°) · 2px dashed · 四色） | P1/P2/P3/P6/P7 | `emoji`, `color: pink\|blue\|mint\|amber` |
| `Ribbon` | 丝带角标（rotate(38°) · 右出血） | P2 | `label`（如「已完成」） |
| `Steps` | 4 阶段步进（mini/大两档；done 绿勾 / now 粉光环 / wait 虚线） | P1/P2/P3/P4 | `stage`, `size` |
| `Chip` | 胶囊标签（g/p/b/n 四色） | 全局 | `tone: g\|p\|b\|n`, `label` |
| `Btn` / `BtnPrimary` / `MiniBtn` | 胶囊按钮三档（白底描边 / 粉渐变主按钮 / 6px 13px 小按钮） | 全局 | `variant` |
| `NewCard` | 虚线新建卡（hover 虚线转实线） | P2 | `hint` |
| `DotGrid` | 全局圆点纹理（body::after 26px 网格） | 全局 | — |

### 7.2 数据型组件（浅色适配）

| 组件 | 用途 | 主要页面 | 关键 props |
|------|------|---------|------------|
| `StatusBadge` | 状态徽章（chip 映射: ok→g / running→amber / fail→red / warn→p / unknown→n） | 全局 | `status`, `label` |
| `EventTypeBadge` | 17 类型浅色徽章（α12% 底 + 同色字，图标） | P3 | `event_type` |
| `PopularityBar` | 热度条 0~1（6px 圆角 · 粉渐变 / soft 底） | P1, P3 | `value` |
| `LogConsole` | 日志台（奶油底 `--bg-soft` · mono · 级别着色 · 虚拟滚动 ≤5000 行） | P4, P3-抽屉 | `lines[]`, `autoScroll` |
| `FileBrowserModal` | 服务端目录选择（模态卡 + 搜索过滤视频扩展名） | P2, P5 | `path`, `onSelect` |
| `CheckpointDialog` | 断点决策（继续/重来，模态卡） | P4, P3-事件/字幕 | `checkpointInfo` |
| `ClipCard` / `ClipDetailDrawer` | pcard 风卡片 + 视频播放抽屉 560px | P3-片段 | `clip`, `thumbUrl` |
| `VideoPlayer` | 封装 `<video>`（时间码跳转；视频区保持黑底） | P3 | `src`, `onSeek` |
| `JobCard` | 任务状态卡（Steps + 进度条 + 日志台） | P4, P1 | `job` |
| `Toast` / `ConfirmDialog` | 胶囊 toast / 确认模态（16px 圆角 + 粉调阴影 + 遮罩） | 全局 | `type`, `message` |
| `FormField` | label + 白底圆角输入 + 校验（mono 数值字段） | P3-设置, P5 | `label`, `value`, `type` |
| `Tabs` | 页内胶囊 Tab（同 pill 语言） | P3, P5 | `tabs[]`, `active` |
| `Table` | 卡内表格（1px dashed `--line-soft` 行分隔 · 行 hover `--bg-soft`） | P3-事件 | `rows[]` |
| `EmptyState` | 空态（虚线卡风） | P2, P3 | `icon`, `hint` |

组件以 `window.UI.*` 导出（ES module 见第 10 章）。

---

## 8. REST + SSE API 契约

> 根前缀 `/api`。JSON 字段名与 core 实际序列化**完全一致**。统一响应:

```json
{ "ok": true, "data": { … }, "error": null }
{ "ok": false, "data": null, "error": "错误信息" }
```

标准错误码: `400` 参数错误 · `404` 资源不存在 · `409` 状态冲突（项目已被任务占用）· `500` 内部错误。

### 8.1 健康 / 环境

```
GET /api/health
→ {
  "ffmpeg": { "ok": true, "version": "6.1", "path": "…" },
  "ffprobe": { "ok": true, "version": "6.1" },
  "models": { "sensevoice": "ok", "firered-zh": "missing", "qwen3-asr": "ok" },
  "llm": { "ok": false, "error": "api_key 为空", "model": "…" },
  "chrome": { "ok": true },
  "polish": { "ok": true },
  "yekitrim_home": "F:\\…\\target\\debug\\.yekitrim"
}
```

### 8.2 项目

```
GET /api/projects            → 项目列表（工作区扫描，含推导 stage）
GET /api/projects/:id        → 单项目详情
POST /api/projects           → 新建（init: video_path, danmaku_path?, output_dir?）
```

> **注（对齐现状）**: 当前落地实现为单工程端点 `POST /api/project/switch` + `GET /api/project/status`（spec M1）。向本契约的多项目端点迁移属于 M1 修订范围（见 11 章）；迁移前 UI 的「项目」页按单工程模式渲染（`project/status` 数据填入单张 pcard）。

`ProjectSummary`（列表项）:

```json
{
  "id": "3d",
  "dir": "media/3d",
  "path": "media/3d/yekitrim.toml",
  "name": "3d 录播",
  "stage": 4,
  "stage_names": ["", "AudioExtractor", "Transcriber", "EventDetector", "Cutter"],
  "input_file": "media/3d/直播回放.mp4",
  "mtime": "2026-09-02T12:00:00Z",
  "output_dir": "output",
  "status": { "audio": true, "transcript": true, "srt": true, "events": true,
              "clips": true, "clip_videos": 9, "danmaku": true, "llm_srt": true },
  "polish": { "dir": "clips/clip-01", "processed": true }
}
```

`ProjectDetail`（详情）:

```json
{
  "id": "3d", "dir": "media/3d",
  "stage": 4,
  "video_info": { "duration_secs": 8073.2, "width": 1920, "height": 1080,
                  "video_codec": "h264", "audio_codec": "aac", "format": "mov,mp4,m4a,3gp,3g2,mj2" },
  "outputs": {
    "audio": { "path": "output/video.wav", "size": 258723057, "mtime": "…" },
    "transcript": { "path": "output/transcript.json", "size": 12345, "mtime": "…" },
    "srt": { "path": "output/video.srt", "size": 8901, "mtime": "…" },
    "events": { "path": "output/events.json", "size": 23456, "mtime": "…" },
    "clips_order": { "path": "output/clips/clips_order.txt", "size": 456, "mtime": "…" },
    "publish_config": { "path": "output/publish_config.json", "size": 5678, "mtime": "…" }
  },
  "asr_metadata": { "anchors": 120, "anchor_coverage": 1.0, "max_interp_span": 0,
                    "fallback_reason": null, "model": "qwen3-asr" },
  "asr_metadata_present": true,   // transcript.json 是否含 asr_metadata（未记录时前端显示「未记录」）
  "clip_count": 9
}
```

> `video_info` 字段与 `core VideoInfo`（`types/mod.rs:42-50`）一致（ffprobe 结果内存态，无文件时 `video_info: null`——服务端按需重新 ffprobe 或读取缓存）。`outputs` 各项 `path/size/mtime` 为该阶段产物文件的实际元数据（无文件时 `size: null, mtime: null`），与 `yekitrim status` 展示一致。

### 8.3 事件

```
GET /api/projects/:id/events        → EventStore
PUT /api/projects/:id/events        { "events": [ {…}, … ] }   // 更新 enabled/label/event_type
POST /api/projects/:id/events/redetect  → job（带 checkpoint 决策）
GET /api/projects/:id/export        → 下载 publish_config.json
```

`EventStore` 结构（即 `events.json` 原文，字段来自 `core types/mod.rs:128-151`）:

```json
{
  "titles": { "开局 20 杀": true },
  "events": [
    {
      "start": { "h": 0, "m": 1, "s": 33, "ms": 300 },
      "end":   { "h": 0, "m": 3, "s": 12, "ms": 0 },
      "label": "开局 20 杀",
      "summary": "…",
      "event_type": "gameplay",
      "source": "LLM",
      "confidence": 1.0,
      "popularity": 0.72,
      "rank": 1,
      "enabled": true
    }
  ]
}
```

> `titles` 是 label→enabled 旧映射（core 消费端仅读 `events[].enabled`）。UI **只编辑 `events[].enabled`**，保存时同步维护 `titles` 键值保持一致。

### 8.4 片段

```
GET  /api/projects/:id/clips                 → [ClipSummary]
GET  /api/projects/:id/clips/:rank           → ClipDetail
GET  /api/projects/:id/clips/:rank/thumb     → jpg（抽帧/缓存）
POST /api/projects/:id/clips/:rank/silence   → job（静音压缩）
PUT  /api/projects/:id/order                 { "order": ["clip-01","clip-03",…] }
```

`ClipSummary`（目录扫描 `output/clips/clip-NN/`，结构对齐 `publish_config.ClipEntry` + `clip.json`）:

```json
{
  "rank": 1,
  "dir": "clip-01",
  "file": "clips/clip-01/clip.mp4",
  "url": "/api/media/…",
  "subtitle": "clips/clip-01/clip.srt",
  "danmaku": "clips/clip-01/clip.dm.srt",
  "title": "开局 20 杀",
  "summary": "…",
  "event_type": "gameplay",
  "popularity": 0.72,
  "start": 93.3, "end": 192.0, "duration": 98.7,
  "original_start": 93.3, "original_end": 192.0,
  "has_thumb": false, "silenced": false
}
```

### 8.5 转录

```
GET  /api/projects/:id/transcript          → { "transcript": Transcript, "srt": "全文", "llm_modified": true }
POST /api/projects/:id/transcript/reset    → 恢复 video.srt
GET  /api/projects/:id/transcript/export   → transcript.json 下载
POST /api/projects/:id/correct             → job（LLM 订正，带 checkpoint 决策）
```

`Transcript`（`core types/mod.rs:52-81` 原文）:

```json
{
  "segments": [ { "start": 1.5, "end": 4.8, "text": "大家好", "confidence": 0.93 } ],
  "language": "zh",
  "asr_metadata": { "anchors": 120, "anchor_coverage": 1.0, "max_interp_span": 0,
                    "fallback_reason": null, "model": "qwen3-asr" }
}
```

### 8.6 配置

```
GET /api/projects/:id/config      → ProjectConfig（yekitrim.toml 现值）
PUT /api/projects/:id/config      { projectConfig } → 写 yekitrim.toml（toml_edit 保留注释）
GET /api/settings/global          → Config（.yekitrim/config.toml）
PUT /api/settings/global          { config }
GET /api/settings/llm             → LlmConfig（.yekitrim/llm.toml；api_key 打码返回，留空=不修改）
PUT /api/settings/llm             { llmConfig }
GET /api/settings/workspace       → { "root": "F:\\…\\media" }
PUT /api/settings/workspace       { "root": "…" }
POST /api/llm/test                → { "ok": true, "latency_ms": 890, "model": "…" }
GET  /api/fs/list?path=…          → { "path": "…", "parents": ["F:\\","…"], "dirs": [{"name","path"}], "files": ["…mp4"] }
```

`ProjectConfig`（`core config/types.rs:19-31` 原文）:

```json
{ "input_file": "media/3d/直播回放.mp4", "danmaku_file": "media/3d/直播回放.ass",
  "transcript_file": "output/video.srt", "audio_file": "output/video.wav",
  "events_file": "output/events.json" }
```

`Config` 各段（`[asr] [detection] [cutting] [sfx]`，与 `core config/types.rs` 一致）:

```json
{
  "asr": { "model": "sensevoice", "num_workers": 0, "chunk_duration": 60.0,
           "chunk_overlap": 10.0, "segment_gap": 1.0, "llm_correct": null },
  "detection": { "danmaku_window_secs": 10.0, "vtuber_name": "",
                 "llm_score_weight": 0.3,
                 "system_prompt": null, "prompt_template": null },
  "cutting": { "output_dir": "./output", "overwrite": null, "silence_noise_db": -30.0 },
  "sfx": { "blacklist": ["tag:special"] }
}
```

> `llm_correct`/`overwrite` 为 Option 字段（可空）；`system_prompt`/`prompt_template` 为空字符串表示使用内建默认（串行化时免发 null）。

### 8.7 模型

```
GET  /api/models  → [ModelInfo]
POST /api/models/:name/download  → job
```

`ModelInfo`（registry `download.rs:13-32` 对照 marker）:

```json
{
  "name": "sensevoice", "dir": "sensevoice",
  "archive": "sherpa-onnx-sense-voice-zh-en-ja-ko-yue-int8-2025-09-09.tar.bz2",
  "marker": "tokens.txt", "installed": true,
  "size_bytes": 173015040, "size_label": "165 MB",
  "files": { "tokens.txt": true, "model.int8.onnx": true, "lexicon.txt": true, "replace.fst": true },
  "partial_bytes": 0,
  "note": "快 · 无 token 级时间戳"
}
```

### 8.8 任务

```
GET  /api/jobs          → [Job]
POST /api/jobs          { "project_id": "3d", "stage": 4 }                 // 单阶段 1..4
                        { "project_id": "3d", "stage": 99, "run_all": true } // 全流程
                        { "project_id": "3d", "operation": "correct" }
                        { "project_id": "3d", "operation": "polish.process" } // polish
POST /api/jobs/:id/cancel
POST /api/jobs/:id/resume   { "resume": true | false }                     // checkpoint 决策
GET  /api/jobs/:id/logs?cursor=…  → 日志增量
```

`Job`:

```json
{
  "id": "job-20260903-120133-a1b2",
  "project_id": "3d", "project_name": "3d 录播",
  "kind": "pipeline", "stage": 3, "run_all": false, "operation": "detect",
  "status": "running",
  "started_at": "2026-09-03T12:01:33Z", "finished_at": null,
  "stages": [
    { "num": 1, "name": "AudioExtractor", "status": "done" },
    { "num": 2, "name": "Transcriber", "status": "running", "current": 75, "total": 120 },
    { "num": 3, "name": "EventDetector", "status": "pending" },
    { "num": 4, "name": "Cutter", "status": "pending" }
  ],
  "checkpoint": { "kind": "detect", "file": "output/events.checkpoint.json", "done": 7, "total": 12 },
  "error": null
}
```

`status` 枚举: `queued | running | waiting_resume | completed | failed | cancelled`。

### 8.9 Polish

```
GET  /api/polish/projects             → [PolishProject]
POST /api/polish/:id/process|render|cover  → job
POST /api/polish/:id/preview          → { "url": "http://127.0.0.1:51234/" }
```

`PolishProject`:

```json
{
  "id": "clip-01", "dir": "media/3d/clips/clip-01",
  "name": "clip-01", "title": "开局 20 杀",
  "video": "clip.mp4", "subtitles": "clip.srt", "danmaku": "clip.dm.srt",
  "output": "vtuber-clip.mp4",
  "processed": true, "rendered": true, "cover": true,
  "segments": 6,
  "mtime": "2026-09-03T10:00:00Z"
}
```

> `rendered` / `cover` 由文件存在性推导（`project.output` 文件、`cover.jpg`），与 `cmd_preview` 的检查逻辑一致。

### 8.10 SSE `/api/stream`

`Content-Type: text/event-stream`；每事件 `event: <type>` + `data: <JSON>`；`retry: 3000`。

| event | data | 触发 |
|-------|------|------|
| `job.started` | Job | 任务入队 |
| `job.stage_started` | `{job_id, stage}` | 阶段开始 |
| `job.stage_progress` | `{job_id, stage, current, total}` | ASR/剪切进度 |
| `job.log` | `{job_id, ts, level, msg}` | 实时日志 |
| `job.waiting_resume` | `{job_id, checkpoint}` | 断点决策等待 |
| `job.completed` | Job | 完成 |
| `job.failed` | `{job_id, error}` | 失败 |
| `job.cancelled` | `{job_id}` | 取消 |
| `model.progress` | `{name, current, total}` | 模型下载 |
| `projects.changed` | `{}` | 项目文件变化（触发列表重扫） |

客户端 `EventSource` 订阅；断线重连后以 `GET /api/jobs` 全量快照兜底（或 2s 轮询降级）。

### 8.11 静态与媒体

```
GET /static/*           → 内嵌资产 (app.js / app.css / icons)
GET /api/media/*path    → 受限媒体（safe_join 白名单: 工作区根 / 工程输出目录 / 输入视频父目录，
                          <video> 播放 clips/clip-NN/clip.mp4, 文本资源如 clip.srt 直接 GET）
```

> 安全注意: `/api/media/*` 以 safe_join 白名单为边界（已落地 `media.rs`），不会暴露工作区外文件；`../` 逃逸返回 403。polish 预览仍走其内建随机端口 server（无冲突）。

---

## 9. 数据模型与项目状态机

### 9.1 状态机（与 CLI 完全一致，数据源 = 文件系统）

`current_stage()` 推导（`config/types.rs:35-68` 行为）:

```
Stage 0 ──①提取──▶ Stage 1 ──②转写──▶ Stage 2 ──③检测──▶ Stage 3 ──④剪切──▶ Stage 4
   │               video.wav            transcript.json       events.json          clips_order.txt
   │               (或 audio_file)      /video.srt            (或 events_file)     (存在 = 完成)
   └── 任意阶段可重跑 (stage ≤ current+1 合法; 跳过前置阶段则拒绝)
```

- UI 永不自行持久化阶段；每页渲染前 `GET /projects` 拿最新 `stage`。
- 单阶段命令重跑合法条件: `current + 1 >= 目标阶段`（等价 CLI `check_stage`）。
- `process`（run_all）无视文件检查全部重跑 —— UI「全流程运行」按钮明确提示会重跑全部阶段。

### 9.2 任务互斥与并发

- **每项目同一时刻最多 1 个 pipeline/polish job**（JobManager per-project `Mutex<Option<JobHandle>>`）；冲突 → `409` 或自动入队（UI 选择「排队」）。
- 模型下载 job 与项目 job 互不阻塞（不同锁）。
- 取消: 丢弃任务 + 释放项目锁；再运行从文件存在性幂等继续（core 无取消传播，取舍见 6.4 / 11 风险）。

### 9.3 checkpoint 文件

| 文件 | 产生方 | 内容 | UI 决策 |
|------|--------|------|---------|
| `output/events.checkpoint.json` + `.partial.json` | EventDetector | 已完成事件数 / BTreeMap<idx, raw> | 「继续上次 / 重来」 |
| `output/correct.checkpoint.json` + `.partial.json` | LLM correct | 已订正分段索引 | 同上 |

决策 → `POST /jobs/:id/resume {resume: bool}` → 注入 `detect_resume`（core 原字段 `types/mod.rs:163`）或 correct 侧参数。

---

## 10. 目录结构（对齐现状）

```
crates/yekitrim-cli/src/
├── main.rs                       # Command::Server(args) → server::run()
├── cli.rs                        # Server(ServerArgs): --host 127.0.0.1 --port 26320 --open --config
├── service/                      # Service 层（CLI 与 HTTP 共用，避免逻辑双份）
│   ├── mod.rs
│   └── workspace.rs              # Workspace { config_file, root_dir } + load/resolve/current_stage
└── server/                       # axum HTTP 层
    ├── mod.rs                    # ServerArgs + run()（Router + TcpListener + open_browser）
    ├── api.rs                    # REST handlers（/api/*，按域拆分文件待重构）
    ├── jobs.rs                   # JobManager + JobEvent broadcast + 日志环形缓冲
    ├── media.rs                  # safe_join 受限媒体路由
    ├── model.rs                  # AppState（workspace + jobs）
    ├── web.rs                    # include_str! 内嵌 + webui_fallback
    └── webui/                    # 编译期内嵌前端（零构建）
        ├── index.html            # 单页外壳（topbar + #view）
        ├── app.js                # ES module: router + api client + ui 组件 + pages
        └── app.css               # sakura design tokens（第 4 章） + 组件样式
```

**前端结构**（零构建）:

```
webui/app.js  (单一 ES module, 依赖注入顺序)
├── ui/            — 组件库 (第 7 章), 导出 window.UI.* (模板字符串 + 事件委托)
├── api.js         — fetch 封装 (统一 ok/error 解包) + EventSource 管理器
├── router.js      — hash 路由 (表驱动: pattern → 页面渲染函数)
├── store.js       — 极简全局状态 (projectCache, jobList, toast 队列)
└── pages/         — 7 页渲染函数 (P1..P7), 每个 = (el, params) => void
```

- 依赖: 无 build tool；无框架；仅原生 `fetch` / `EventSource` / ES module。`index.html` 内 `<script type="module" src="/static/app.js">`。
- 样式: `app.css` 定义 tokens（第 4 章 CSS 变量）；组件类名形如 `.ui-card .ui-btn .ui-chip`。
- 图标: emoji + 内联 SVG 字符串常量（事件类型映射 D9 用 emoji + 色卡，无需引入图标库）。
- 字体: 与 polish 相同字体栈（系统回退，不打包字体文件）。

---

## 11. 实施里程碑建议

| 里程碑 | 内容 | 验收 |
|--------|------|------|
| **M1 骨架+只读（修订）** | `yekitrim server`（已落地）+ 内嵌资产（已落地）+ **工作区扫描 → 多项目端点 `GET /api/projects`（新增）** + **主题切换为 sakura-cream 浅色** + P1/P2 只读渲染（P2 = sakura 项目卡网格） | `yekitrim server` 打开 `#/projects` 看到本机真实项目的樱花奶油卡片 |
| **M2 任务+监控** | JobManager 收敛（见下「已知问题」修正）+ ProgressReporter→broadcast 桥 + SSE 前端接入 + P4 抽屉（铃铛唤起）+ P3 概览/事件 + checkpoint 决策 | 全流程 job 可跑、可取消、断点决策（kill 后继续）；`yekitrim status` 与 UI 阶段一致 |
| **M3 结果审阅** | P3 片段库（缩略图/抽屉/静音压缩/顺序）+ 字幕 Tab（diff/导出）+ polish 扫描 P7（process/render/cover 触发） | 切片在 UI 内预览播放、静音压缩连锁可用 |
| **M4 配置闭环** | P3-设置 + P5 全局设置表单（含 LLM 测试、extra_params 校验）+ P6 模型管理 + polish 跳转 preview | `yekitrim config show` 与表单一致；模型下载断点续传可验证 |

**已知问题（现状骨架，归入 M1/M2 修正）**:

1. 单工程模式: 现状为 `POST /api/project/switch` + `GET /api/project/status`，与 8.2 多项目契约不符 → M1 补工作区扫描（`service/` 层已有 `Workspace::load` 基础）。
2. JobManager 双实例脱节: `api.rs` 使用 `state_job_manager()` 静态实例，API handler 创建的 job 在 `AppState.jobs` 中，状态不互通 → M2 收敛为单一实例。
3. `WebProgress` 空壳: `ProgressReporter` 实现未转发真实进度 → M2 实装 stage/progress 回调 → broadcast。
4. 前端未接 SSE: 现状为 4s 轮询 → M2 接入 `EventSource`（保留轮询兜底）。
5. 构建阻断: `jobs.rs` 使用 `chrono::Local::now()` 但 chrono 未入依赖 → 补依赖或改 std 时间。

**风险与缓解**:

1. **cancel 粒度粗**（core 无取消传播）→ M2 先实现「取消=丢弃任务+锁释放+文件状态兜底」，后续再考虑 `CancellationToken` 渗透 stage。
2. **JS 无框架**，7 页组件量大 → 严格按组件库拆分，页面只写布局；状态全部走 `store.js` 单一来源。
3. **SSE 兼容性**（本地 Chrome/Edge 均支持）→ 兜底降级为 2s 轮询 `GET /api/jobs`。

---

## 附录 A: 样稿与截图

设计标准稿: `docs/web-ui-mockups/sakura-cream.html`（P2 项目列表 · 浅色樱花奶油）。

```bash
# 重新生成 1440×900 截图（需本机 Chrome；在仓库根目录执行）:
"chrome.exe" --headless=new --disable-gpu --screenshot="sakura-cream.png" --window-size=1440,900 "file:///$(pwd)/docs/web-ui-mockups/sakura-cream.html"
```

其余样稿 `mint-soda.html`（数据表格布局）/ `sky-glow.html`（仪表盘布局）为**布局参考**，配色与组件不采用。

## 附录 C: 设置页保存的**合并语义**（清空 ≠ 不发送）

`/api/config/global` 与 `/api/config/llm` 的表单保存走的是 `patch_config_file` ——
它是**递归深合并**（`{"llm": {"model": …}}` 只会动提到的键，`[ingest.recorder]` 这类子表也递归）。

由此有一条**很容易踩的规约**：

> **省略一个键 = 保持原值**；想清空/删除，必须**显式发 `null`**（`merge_into_table` 把 null 当 `remove`）。

⚠️ 反过来过是有代价的：早期 `null` 会落到 `json_to_toml_value` 的 Null 分支被写成空串，
于是 `max_tokens = ""` 下次读配置**直接解析失败**（整份 LLM 配置被打回默认值）。
所以"删键"这条路是刻意用 null 表达的，别再发明第三种写法。

### 实例：`extra_params` 清不掉（2026-10 真机报的）

`LLM_CONFIG_TOML` 模板里那行是**活动行**：

```toml
extra_params = { seed = 42, thinking = { type = "enabled" }, response_format = { type = "json_object" } }
```

用户把设置页那个 JSON 输入框清空后保存 —— 前端当时是
`if (非空) patch.extra_params = JSON.parse(...)`，**空就不发这个键**。
合并语义下"不发" = "保持原值" ⇒ 模板那行原样留着，页面重新加载又把"预设内容"读回来，
看上去就是**"每次保存都被写回预设"**。

修法：空分支显式 `llm.extra_params = null`（与下面 `overrides` 取消勾选用的是同一机制）。
判据（真机验过）：清空 → 前端 PUT body 里 `extra_params: null` → `llm.toml` 里那行**消失**。

⚠️ 模板注释本来就写着"（设置页把输入框清空也会删掉它）" —— 也就是说这是**实现没跟上文档**，
不是"设计就是这样"。新增任何可选字段时照这条自检一遍：**它能不能被清空？**

## 附录 B: 与 CLI 命令映射速查

| UI 操作 | 等价 CLI |
|---------|----------|
| 新建项目 | `yekitrim init <video> [--danmaku]` |
| 全流程运行 | `yekitrim process` |
| 单阶段重跑（①–④） | `yekitrim extract/transcribe/detect/cut` |
| dry-run | `yekitrim process --dry-run` |
| 重新检测 | `yekitrim detect`（checkpoint 决策同 CLI 交互） |
| LLM 订正 | `yekitrim correct` |
| 静音压缩 | `yekitrim silence <file>` |
| 模型下载 | `yekitrim download` |
| polish 处理/渲染/封面 | `yekitrim polish process/render/cover` |
| polish 预览/编辑器 | `yekitrim polish preview` |
| 全局配置 | `yekitrim config show / config init` |
| 阶段推导 | `yekitrim status` |
### 附录 D 项目级「主播名」（2026-10；预处理页的输入框 2026-11-15 已删）

全局 `[detection] vtuber_name` 只是**默认值**；每个工程可以自己写一份，写在工程的
`yekitrim.toml` 里（同样是 `[detection] vtuber_name`）。⚠️ **预处理页不再放这个输入框**
（用户口径 2026-11-15：主播名只用在最终切片标题上，识别页摆它没有意义）—— 要改就手改
`yekitrim.toml`，或改全局配置；精修页那条「主播名」是**单条标题的草稿**，不写回配置。

机制不需要新的配置字段 —— `load_config_at()` 本来就是
`默认值 → 合并 .yekitrim/config.toml → 合并工程的 yekitrim.toml`，而工程文件**本来就能写 `[detection]`**。
所以只差把它暴露到界面：

| 读 | `GET /api/project-data/config` 的 `config`（合并后生效值）+ `project_config`（工程自己那份） |
| 写 | `PUT /api/project-data/config` 的 `{patch:{detection:{vtuber_name}}}`（保留注释与其他 section） |

⚠️ **清空 = 跟随全局，不是"设成空名字"**：`merge()` 对 `vtuber_name` 的**空串不覆盖**
（`config/mod.rs` 的 `if !overlay.detection.vtuber_name.is_empty()`），所以写空串就恢复继承。
手改工程文件时照这个来 —— 没有"启用覆盖"开关这回事。

（历史：界面曾据此在预处理页放一个输入框，靠 `GET /api/project-data/config` 的 `project_config`
那一份区分"本工程写了"与"继承来的"；输入框 2026-11-15 删除，接口与字段都还在。）

### 附录 E 本地模式的三处口径 · 首页检查项（含运行状况，2026-10；2026-11-15 运行状况并入检查项）

**一、`localNoAuth` 只有一个来源：`stores/health.js`**

`GET /api/health.local_no_auth` 决定三处界面口径，三处**必须同源**，否则一定会漂：

| # | 位置 | 行为 |
|---|---|---|
| ① | 侧栏用户块 | 收起「退出登录」（本地模式没有会话可退） |
| ② | 设置页标签条 | 过滤掉 `SETTINGS_SECTIONS` 里 `localHidden` 的子页（现在是 `account`） |
| ③ | 路由守卫 | 把 `/settings/account` 挡回 `/settings/config`（**直接输网址也要挡**） |

- **读失败当 `false`**（照旧显示那些入口）：别默认 true，一次请求抖动就会把入口藏掉。
  结果**缓存整个会话** —— 这个位在服务端进程生命周期内固定，而换服务端必然伴随重新加载页面。
- ⚠️ 标签条要在**首次渲染前**就拿到它，否则先闪 5 个标签再缩成 4 个。做法：用同模块的
  `localNoAuthKnown()` **同步初始化** —— 路由守卫进这一页之前已经 `await` 过同一个 promise，
  所以那时它一定是已解析的。
- ⚠️ 之前 `App.vue` 与 `SettingsView` **各拉了一次** `/api/health`：那不只是重复请求，还漏掉了
  第③处（守卫没得读），并且两处"读失败怎么办"随时会写成不一致。现在只有 `health.js` 一份。

**二、账号页与模型管理分离**

`ASR 模型`是**全机共享的程序级资产**（跟"谁在登录"无关），住在「账号与模型」页里会让人以为
它是按用户分的；而且本地模式要把整页藏掉，模型管理跟着一起消失就更不对。所以：

- 「账号」子页现在**只剩注册与邀请码**，`localHidden: true`（本地模式整页不显示）；
- 模型管理搬去**首页检查项的「字幕模型」行**（2026-11-15 起原「运行状况」卡整卡并入检查项）。

**三、首页检查项（原「运行状况」doctor 已并入，2026-11-15 用户口径）**

不再有独立的「运行状况」卡：ffmpeg / ffprobe / 出片底座的实测 chips 进检查项的**「运行环境」行**，
模型清单与下载（`ModelList`）进**「字幕模型」行**（缺模型时内嵌展开，齐了收成一句「已安装」）。
全就绪时整卡收成一行「开始使用 · 环境就绪」。改这一版的原因（用户实测）：旧运行状况卡报
「1 项待处理」，展开却只有 ffmpeg/ffprobe/模型三样全绿 —— **出片底座（`dhampir_ready`）在卡里
没有任何展示**，待处理项成了悬案。并入后逐项可见，缺哪项当场指路。

数据来自三个接口，**一起发、一起刷**：

| 接口 | 给什么 | 落在哪 |
|---|---|---|
| `GET /api/health` | `ffmpeg` / `ffprobe` / `dhampir_ready`（环境位，现场实测） | 检查项「运行环境」行的 chips |
| `GET /api/doctor` | `models_dir`、各模型 `installed` + 实际路径 | 「字幕模型」行（装没装的权威判据） |
| `GET /api/models` | 富列表（`size_label` / `note` / `source_label`），`ModelList` 靠它渲染 | 「字幕模型」行内嵌清单 |

⚠️ 只刷一个会出现"点了下载、按钮没变"的错位。

- ⚠️ **读失败照实显示、不静默留空**（`doctorError` 传进检查项，「字幕模型」行显示原句 + 重新检查，
  且**不许拿旧 doctor 快照冒充现状** —— `doctorUsable` 口径）。实测：还在用初始密码时服务端对这两个
  接口都回 **403**（`仍在使用初始密码，请先修改密码`），如实显示比变成一个空列表有用得多。
- ⚠️ `GET /api/models` / `GET /api/doctor` **谁都能读**，但**下载是 admin-only**
  （`models_download` 回 403「仅管理员可下载平台模型」）。`ModelList` 挂在**所有人都能看的首页**
  检查项里，所以下载按钮必须按 `isAdmin()` 门控；非管理员只看到「已安装 / 未安装」。

### 附录 F 从 `webui/src` 卡搬出来的细节（2026-10 卡减肥）

> 卡里只留"不许做什么"，这里放**为什么**与实测数字。卡在两条指令链里（`+views` / `+components`），
> 它的体积直接决定那两条链的余量 —— 一度只剩 **5 B / 76 B**，所以推理性内容一律放这里。

**一、换项目必须让视图重挂（`router-view :key`）**

四个视图都在 `setup`/`onMounted` 里抓一次 `pid`。同一步骤下换项目时**只有 `:pid` 变**，
vue-router 会**复用组件实例**、`setup` 不再执行 —— 用户当年的实测线索是
「**跨步骤切换正常、同步骤不正常**」。最危险的是预处理页：界面显示旧数据，
而 `saveTranscript(pid.value, …)` 用的是**新** pid ⇒ **把 A 的字幕写进 B**。

key 必须用 **pid** 而不是 `fullPath`：后者会让精修页换片段也整块重挂，丢掉原地切换。

**二、素材清单（弹幕/字幕）的细节**

- ASR 产物**不可删**（它就是那个模式用掉的转写名额），唯一例外是**文件已不存在**
  （`removable = !is_asr || !exists`）。
- 「删除」**只从选择里移除**（清 `*_file` + 摘清单），**不删磁盘文件**，走 `confirmDialog` 二次确认。
- 「切换」= 置为生效值，切完整页 `load()`。
- 条目 `exists=false` 时**照实报「文件不存在」**（置灰 + 删除线），不要静默飘到 `output/` 兜底上
  —— 否则用户以为在用这份，其实用的是另一份。
- 清单读失败（`assets` 为 null）时**不要**碰 `assetDirty`；`assetDirty` 只在**检测真的跑了**之后才清。
- **改完不自动跑任务**（改输入 ≠ 要重跑）。

**三、切片 ↔ 事件的对应关系**

`rank` 是检测按热度排的，**重新检测会整体重排** —— 只看目录名会让新事件"继承"老片段的标题。
判据 `utils.js::clipMatchesEvent`（= Rust `cutting::foreign_clip_conflict`）：
**目录名相同 且（标题相同 或 原片区间 IoU ≥ 0.5）**。

- 已剪卡片**先认自己的 `clip.json`**，只有认得出事件时才跟着事件走；认不出挂 **「独立」** chip。
- 未剪切事件 = 没有任何"属于自己的"片段的事件；序号被别人的旧片段占着**也算未剪切**
  （挂 **「序号被占用」** chip）。

**四、首页「切片总览」的排序**

全部项目的切片摊平成**一份总排序**（未导出 / 已导出两个可折叠区块），按各片段
**「最后一次改动」从新到旧**：`clips[].mtime` ← 服务端 `project_data.rs::last_touched_secs`
= 片段目录里最新的文件时间（**含 `data/` 一层**）；同秒回落**项目 id 降序 + rank 升序**。
效果：刚剪完 / 刚精修过的浮到最前。点卡片 = 去该片段的精修页，没初始化就自动 `polish init`
（`initPolish(pid, dir)` 传**绝对目录**，scope 守卫按它反查归属）。

**五、转写与素材槽（`PrepareView`）**

产物落盘时 `assets::bind_asr_artifact` 把它登记进素材清单**并置为生效字幕**，那个 ASR 模式的
"槽"就用掉了（`GET …/assets` 的 `slots` 与后端 `assets::slot_bound` **同口径**，前端别自己推）。

- ⚠️ **两格槽都占满时才置灰**（`canPickAsr = slotFree` + 任务运行中）—— 单格占满仍可切到另一个
  模式去跑；写成"有产物就灰"会让用户以为另一个模式也不能用了。
- 模式值读 **merged** 配置（`GET /api/project-data/config` 的 `config.asr.mode`，读失败回退
  `hybrid`），写走 `PUT /api/project-data/config {patch:{asr:{mode}}}`。
- **状态回显**：任务名 + 状态 chip + 进度条已经把状态说清，别再挂「任务完成」/「运行中…」；
  状态栏要**常驻**（`jobStatusText` 的 `if (!job.id) return '空闲'` 就是为它加的）。
  唯一例外：**失败原因必须显示**，不许静默吞错。

**六、表情包区块的布局数字**

卡片墙 `.stk-grid` = `minmax(112px,1fr)`；点卡片开 `AppModal wide` 大图预览，
`.stk-preview` 给 `max-height:60vh`；`.pack-files` **框内滚动、上限 420px**。
