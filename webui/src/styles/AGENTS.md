# webui/src/styles — 全局样式：tokens 变量 + app.css 基准类

> 范围：`webui/src/styles`/** ｜ 上游：整个前端（由 `main.js` 引入）｜ 验证：`npm --prefix webui run build`

## 职责

唯一设计标准的落地点。`tokens.css` 是色板 / 圆角 / 阴影 / 字体变量（全应用唯一的取色处）；
`app.css` 是两份设计样稿组件类的**并集**，所有 shell 与数据型组件的基准外观都在这里。
组件 scoped 样式只写样稿里没有的东西。改完必须重建 `webui/dist`。

## 入口

- `tokens.css`：`--main` / `--main-soft` / `--pink-text` / `--bg` / `--bg-soft` / `--r-sm` 等变量。
- `app.css`：`.shell/.side/.proj/.snode/.foot-item/.user-chip/.card/.card-blk/.btn/.mini-btn/.chip/`
  `.state-badge/.src-badge/.tb/.tabs/.wiz-src/.player/.lane/.grid9/.meter/.login-*/.empty/...`
  以及 `.lbl-row`/`.rv`（设置页标签行数值位）、`.fld-input`（设计基准输入框）。
- 两份文件都由 `main.js` 全局引入，全应用只有这一份。

## 边界

- **样式纪律（前端改样式前必读）**：唯一设计标准是
  `docs/web-ui-mockups/sakura-cream.html`（配色/圆角/阴影/字体，v0.2 §4 全量取自它），
  v5 壳与数据型组件的布局数值取自交互基准 `docs/web-ui-mockups/ia-demos/multiuser.html`。
  - ⚠️ **色板 2026-10-03 二次换肤**（粉 → 砖红 → **哥特橙瞳**），特征色逐像素取自角色立绘
    （反查 RGB 距离 ≤ 3.2）：`#E86838` 橙瞳 = `--main`，`#202020`/`#F0F0E0` 裙黑白 = 亮暗两套
    `--ink`，`#F86860` 蝙蝠红瞳 = `--red`，`#D8C0A0` 金发 = `--tan`。基调「**黑白为骨 + 橙瞳点睛**」，
    **别把大面积 UI 染橙**。**样稿与 tokens 必须同源改**（样稿是"唯一设计标准"，它说谎会带偏下一个人）。
    ⚠️ `--pink-light`/`--pink-text`/`--shadow-pink` **名字保留、语义已变**（橙瞳浅调/文字/投影），
    改名牵动 27+ 处。
  - ⚠️ **双主题（2026-10-03 起）**：亮色默认（`:root`），暗色在 `:root[data-theme="dark"]` 覆盖。三条硬纪律：
    ① **两套成对补齐**（当前 43:43；`--font`/`--mono`/`--r*` 是主题无关的，只声明一次，不算数）。
       漏一个 → 暗底继承亮色浅值，**静默看不见**，构建与契约脚本都不报错。
    ② **组件里不许写颜色字面量**，骨架类也走变量（`--scrim`/`--sticky-bg`/`--shadow-*`/`--tex`/`--veil`）；
       `rgba(32,32,32,.05)` 这种**看着像绝对色、其实看主题**的最容易漏。
       ⚠️ **`rgba(255,255,255,…)` 是最阴的一种**：亮色下它压在 `#FFFFFF` 卡片上**等于没有**，
       本地看一切正常；暗色下才露出一块发白的高亮底。2026-10 一次捞到三处（对比度用 token 值算的）：
       `.empty` 空态背景 **1.20**（用户报的"空目录背景颜色不对"）· `.toast` 白字压 `--ink`
       **1.15**（`--ink` 暗色下是浅色 ⇒ 白底白字）· `.ov-init` 初始化浮层 **1.25**。
       修法分别是 `--bg-soft` / `color: var(--bg)`（`--ink` 的天然反色）/ 新增 `--veil`。
       **改完拿 token 值算一遍对比度**（≈1 就是"整块看不见"），别只看颜色"变没变"。
    ③ 主色渐变与其上文字**成对用** `--grad-main`/`--on-main`：暗色下渐变提亮、文字反成**深色**
       （深字压橙 5.02 > 白字 3.25）；写死 `#fff` 会糊掉。
    ⚠️ 功能色（封面色板、`#000` 视频底）**不随主题变**，别进 dark 块。
  - `webui/src/styles/tokens.css` = 色板/圆角/阴影/字体变量（**不得另取色**）；
    `webui/src/styles/app.css` = 两份样稿组件类的**并集**（`.shell/.side/.proj/.snode/.foot-item/`
    `.user-chip/.card/.card-blk/.btn/.mini-btn/.chip/.state-badge/.src-badge/.tb/.tabs/.wiz-src/`
    `.player/.lane/.grid9/.meter/.login-*/.empty/...`）。
  - ⚠️ **主行动按钮用 `--action-bg` / `--action-bg-deep` / `--on-action`，不用 `--grad-main` / `--on-main`**：
    白字压 `--main`(#E86838) 只有 **3.25**，而按钮文案是 13px 正文（AA 要 4.5）—— 现值最亮端 4.99。
    暗色**反向**（提亮底 + `#202020` 深字，6.71）。渐变那套只留给**图形**（头像 / 进度条 / 折叠轨）。
  - ⚠️ **Toast 的定位与堆叠只在 `app.css` 一份**（`.toast-wrap` + `.toast`）：组件里再写同名类会
    各覆盖一半属性，子元素仍是 `fixed`、脱离容器 flex 流 ⇒ 多条消息全叠在屏幕中央（实测过）。
    正文一律 `--ink`、语义只上左侧强调条（`--green-deep` 压白卡只有 3.26，读不清）。
  - **组件内不再重复定义这些基准类**（scoped 样式只写样稿里没有的东西，如行内删除按钮、
    页面专属布局）；新增组件先在样稿里找同名类，找到就照抄数值。
  - 关键数值：侧栏 268px、卡片 `22px 22px 18px`（`.card-lg`）/`16px`（`.card-blk`）、
    页标题 26px、chip `2px 10px`、进度条 6px、贴纸 52px/`rotate(-5deg)`、
    `.cols-main` = `1fr 300px`、主区 padding `18px 22px 60px`。
- ⚠ **`.src-badge` 必须是 `flex:none`**（app.css，样稿本来就是这么写的）：它曾经为了让长目录名
  标签能截断而写成 `flex:0 1 auto; min-width:0; max-width:150px` + 省略号，源标签改成只放类型
  之后这个前提没了，而它仍会被压缩 —— 实测项目名很长时盒宽被挤到 **18px** 而「本机」需要
  **30px**，`.side-list` 又是 `overflow-x:hidden`，于是硬裁掉大半个字。同理 `.proj-head .del`
  也钉了 `flex:none`。**别再改回可收缩。**（谁消费它见 `webui/src/AGENTS.md`）
- ⚠ **app.css 里的全局元素选择器会穿透组件 scoped 样式**：`video { background:#000;
  border-radius:var(--r-sm) }`（给剪辑卡片那类缩略图用的）会盖上封面设计器的 `.stage-video`，
  竖屏素材在设计器里变成**两条纯黑边**；`.stage-video` 的 `background:transparent;
  border-radius:0` 是**必需的覆盖、不是装饰**。根因与几何钉在
  `node webui/scripts/check-cover-stage.mjs`（含 6 处变异自证）。**加全局元素选择器前先想清楚
  它会落到哪些组件上**。
- 页面专属的新态（`.proj.on`、`.snode.viewing`、`.rail-*`、`.side-toggle`、`.rdot`）写在各自组件的
  scoped 里，**不要**塞回 app.css。
- **改样式后必须重建 `webui/dist`**（`webui/dist` 已入库）：不重建，`yekitrim server` 端出的还是旧界面。

> 改动只允许落在本目录；跨模块改动先读对方卡，并跑两条门禁。

## 验证

```bash
npm --prefix webui run build
node webui/scripts/check-cover-stage.mjs
```

构建成功即样式被正常打包进 `webui/dist`（本卡没有单测运行器）；`check-cover-stage.mjs` 是唯一
直接钉住 **app.css 全局元素选择器穿透** 那条坑的脚本，动 app.css 的全局选择器后要跑它。
