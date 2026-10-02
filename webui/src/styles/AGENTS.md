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
  - `webui/src/styles/tokens.css` = 色板/圆角/阴影/字体变量（**不得另取色**）；
    `webui/src/styles/app.css` = 两份样稿组件类的**并集**（`.shell/.side/.proj/.snode/.foot-item/`
    `.user-chip/.card/.card-blk/.btn/.mini-btn/.chip/.state-badge/.src-badge/.tb/.tabs/.wiz-src/`
    `.player/.lane/.grid9/.meter/.login-*/.empty/...`）。
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
