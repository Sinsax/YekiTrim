# webui — Vue3 工作台前端（构建产物就是 `yekitrim server` 的唯一界面）

> 范围：`webui`/** ｜ 上游：`yekitrim-server`（服务 `webui/dist`）｜ 验证：`npm --prefix webui run build`

## 职责

`yekitrim server` 服务的界面**就是 `webui/dist`**（Vue3 构建产物）：**release 把它编在 exe 里**、
debug 读盘、`--webui-dir`/`YEKITRIM_WEBUI_DIR` 可覆盖（规则与判据在 `crates/yekitrim-server/src/web.rs`）。
⚠️ **不是**"内嵌一份手写单文件页面"那套 —— 那份长期与 API 漂移、已删除；内嵌的是**构建产物本身**。
因此 `dist` 是**编译期输入**（缺失或比 `webui/src` 旧会让编译失败），不是运行时依赖。
`src/` 是工作台前端（壳 / 路由 / 视图 / stores / composables），`scripts/` 是 9 个静态判据脚本
（既锁"同一套算法有两份实现"的漂移，也锁布局不变式）。

## 入口

- 构建：`cd webui && npm install && npm run build`（`build` = `vite build`，`outDir: dist` + `emptyOutDir`）。
- 起不了 npm 脚本时直接调 vite：`node webui/node_modules/vite/bin/vite.js build`（**在 `webui/` 下跑**）。
- `webui/scripts/` **九个**静态判据脚本（零依赖、不 import 应用模块、不需要起服务）：
  - `check-overlay.mjs` — 覆盖层色档：`templates/index.html` ↔ `src/polish/engine.js` 逐例对拍。
  - `check-caption-wrap.mjs` — 竖屏折行：`wrapCaptionParts` 两份实现。
  - `check-sticker-anchor.mjs` — 贴纸锚点/尺寸（事件左上角百分比 vs `[sticker] position` 中心百分比）。
  - `check-cover-stage.mjs` — 封面舞台：黑边覆盖 + 恒横版 + 模糊同源 + 类名穿透 + 图层栈
    （`.pane` 不许复用、`.lyr-props` 要 `min-height`、`COVER_HTML` 只替换一次、`#fx-extra` 无 z-index）。
  - `check-handheld.mjs` — 手持摇摆：波形逐值对拍 + 起停状态机 + 竖屏归零。
  - `check-stage-backdrop.mjs` — 画面构成/衬底：四种模式 + 图层序 + 预览 DOM 层。
  - `check-preview-ab.mjs` — 跳屏蔽区间的 A/B 预滚（换台顺序、Dhampir 闸门）。
  - `check-grid-parity.mjs` — **九宫格四方对拍**：`GRID_RECTS` 与 Rust `grid_rect` **逐值**一致、
    **格子中心**四份实现互差 ≤0.02%（Rust / engine / 模板**手抄表** / 转译器 `1/6`）。
    这条规矩过去**只有人读没有机器判据** —— 改一处不改其余三处的症状是「预览选中的区域与出片
    锚住的位置差一点点」且**不报错**。带两条变异自证。
  - `check-layout-invariants.mjs` — **布局不变式**（2026-10）：`.llm-cols` 两栏等宽 · 半宽列里滑块跨满
    整行（否则滑条压到 59px）· `.ov-cols` 单栏态 · `extra_params` `rows ≥ 10` · `ModelList` 下载按
    admin 门控 · 模板标识符必须有定义（Vue 对未定义变量**不报错**）。⚠️ 每条带**变异自证**。
  - `../scripts/check-preview-audio.mjs` — 预览**出声**契约（住 `scripts/`）。
    （卡里曾写"已进 `gates.mjs`"，但仓里**没有**这个聚合脚本 —— 逐个跑即可。）
- 子目录细则看各自卡：`src/AGENTS.md`、`src/api/AGENTS.md`、`src/styles/AGENTS.md`、`src/polish/AGENTS.md`。

## 边界

- ⚠️ **`webui/dist` 已入库**（`webui/.gitignore` 不再忽略它）：改了 `webui/src` 必须重建并一起提交，
  否则 `yekitrim server` 端出旧界面。
- ⚠️ **改了 `crates/yekitrim-polish/templates/index.html` 必须重建二进制**：它经 `include_str!` 进
  `yekitrim-polish`（`render/server.rs::INDEX_HTML`），运行中的 `yekitrim server` 端出的仍是旧页面 ——
  「改完没反应」十有八九是没重建/没重启，不是代码没生效。
- 改样式同样要重建 `webui/dist`（本目录只写前端；样式纪律见 `src/styles/AGENTS.md`）。
- ⚠️ **别在任何探针/判据里写死 `data-v-<hash>`**：生产构建的 scope id = hash(路径 + 源码)，改一句模板
  文案就会轮换该组件全部选择器 id，连带 `dist` 里 CSS 文件名的 hash 一起变 —— 那**不等于样式改了**，
  比对产物时先把 `data-v-\w{8}` 归一化，再逐字节比。
- 契约脚本的漂移判据都写在脚本自己的头部注释里，**改脚本前先读它**（它们是门禁，不是工具）。

> 改动只允许落在本目录；跨模块改动先读对方卡，并跑两条门禁。

## 验证

```bash
npm --prefix webui run build
node webui/scripts/check-overlay.mjs
node webui/scripts/check-caption-wrap.mjs
node webui/scripts/check-sticker-anchor.mjs
node webui/scripts/check-cover-stage.mjs
node webui/scripts/check-handheld.mjs
node webui/scripts/check-stage-backdrop.mjs
node webui/scripts/check-preview-ab.mjs
node webui/scripts/check-grid-parity.mjs
node webui/scripts/check-layout-invariants.mjs
node scripts/check-preview-audio.mjs
```

`npm run build` 成功 = `webui/dist/` 重新生成（`emptyOutDir` 会清空旧 hash 名）；**dist 入库，
必须和 `src/` 的改动一起提交**。九个 node 脚本各自打印对拍项数与通过情况，退出码非 0 即失败；
它们零依赖、不 import 应用模块（`webui/package.json` 没有 `"type": "module"`）。
