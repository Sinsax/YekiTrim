# webui — Vue3 工作台前端（构建产物就是 `yekitrim server` 的唯一界面）

> 范围：`webui`/** ｜ 上游：`yekitrim-server`（服务 `webui/dist`）｜ 验证：`npm --prefix webui run build`

## 职责

`yekitrim server` 服务的界面**就是 `webui/dist`**（Vue3 构建产物）：**release 把它编在 exe 里**、
debug 读盘、`--webui-dir`/`YEKITRIM_WEBUI_DIR` 可覆盖（规则与判据在 `crates/yekitrim-server/src/web.rs`）。
⚠️ **不是**"内嵌一份手写单文件页面"那套 —— 那份长期与 API 漂移、已删除；内嵌的是**构建产物本身**。
因此 `dist` 是**编译期输入**（缺失或比 `webui/src` 旧会让编译失败），不是运行时依赖。
`src/` 是工作台前端（壳 / 路由 / 视图 / stores / composables），`scripts/` 是 14 个静态判据脚本
（既锁"同一套算法有两份实现"的漂移，也锁布局不变式）。

## 入口

- 构建：`cd webui && npm install && npm run build`（`build` = `vite build`，`outDir: dist` + `emptyOutDir`）。
  起不了 npm 脚本时直接调 vite：`node webui/node_modules/vite/bin/vite.js build`（**在 `webui/` 下跑**）。
- `webui/scripts/` **十四个**静态判据脚本（零依赖、不 import 应用模块、不需要起服务）：
  - `check-overlay.mjs` — 覆盖层色档：`templates/index.html` ↔ `src/polish/engine.js` 逐例对拍。
  - `check-caption-wrap.mjs` — 竖屏折行：`wrapCaptionParts` 两份实现。
  - `check-sticker-anchor.mjs` — 贴纸锚点/尺寸（事件左上角百分比 vs `[sticker] position` 中心百分比）。
  - `check-cover-stage.mjs` — 封面舞台（`.pane` 不许复用/`COVER_HTML` 只替换一次等）。
  - `check-handheld.mjs` — 手持摇摆：波形逐值对拍 + 起停状态机 + 竖素材进非竖画布归零。
  - `../scripts/check-camera-zoom-parity.mjs` — **运镜档位表五处逐值一致**（§18-C）：三档默认倍率
    （`render-core` / 模板 `camFactor` / `engine.js` / 转译器）+ 后端夹取区间 + 前端滑杆同值。
    任一路漏改的症状是"预览与成片用不同倍率"，两边各自看都正常。**8 处变异自证**。
  - `check-stage-backdrop.mjs` — 画面构成/衬底：四种模式 + 图层序 + 预览 DOM 层 + 两轴摆位。
  - `check-preview-ab.mjs` — 跳屏蔽区间的 A/B 预滚（换台顺序、Dhampir 闸门）。
  - `check-grid-parity.mjs` — **九宫格四方对拍**：`GRID_RECTS` 与 Rust `grid_rect` **逐值**一致、
    **格子中心**四份实现互差 ≤0.02%（Rust / engine / 模板**手抄表** / 转译器 `1/6`）。改一处不改其余
    三处的症状是「预览选中的区域与出片差一点点」且**不报错**。带两条变异自证。
  - `check-layout-invariants.mjs` — **布局不变式**：`.llm-cols` 两栏等宽 · 半宽列滑块跨满整行 ·
    `extra_params` `rows ≥ 10` · `ModelList` 下载按 admin 门控 · 模板标识符必须有定义（Vue 对未定义
    变量**不报错**）。⚠️ 每条带**变异自证**。㉛ **控件契约**：面板字段由 `fields[]` 派生。
  - `check-build-equivalent.mjs` — **无子进程的构建等效校验**：编译每个 `.vue` + 校验 import
    （esbuild 服务起不来时的替代验证）。
  - `check-text-tilt.mjs` — **文字倾斜/压缩 预览↔成片**（T4）：`engine.js::textTiltTransform`
    必须与 Dhampir `text_transform.rs::blit_affine` 的 **`R(rot)·S(sx,sy)`** 同序（CSS 从右往左）。
    ⚠️ **唯一**内容是"乘法顺序"；⚠️ 别拿退化用例试（`tilt=0,squash=1` 两序相等）。6 处变异自证。
  - `check-emoji-decode.mjs` — **表情解码**：目录 881 条**每条**都要被 `safeEmoji` 解出字符。
  - `check-heat-curve.mjs` — **热度曲线**（标尺带）：口径 + `punch.rs` 逐条对拍 + 装配不变式，7 处变异自证。
  - `../scripts/check-preview-audio.mjs` — 预览**出声**契约（住 `scripts/`，仓里**没有** `gates.mjs`
    这个聚合脚本 —— 逐个跑即可）。
- 子目录细则看各自卡：`src/AGENTS.md`、`src/api/AGENTS.md`、`src/styles/AGENTS.md`、`src/polish/AGENTS.md`。

- **`scripts/check-polish-page-loads.mjs`** —— **页面能不能真的挂起来**（headless Chrome；没 Chrome/服务 ⇒ SKIP）：
  精修页 + 工位页素材库三页签；抓渲染期 `ReferenceError`（用了没 import 的标识符 ⇒ Vue 卸整块子树）。

## 边界

- ⚠️ **`webui/dist` 已入库**（`webui/.gitignore` 不再忽略它）：改了 `webui/src` 必须重建并一起提交，
  否则 `yekitrim server` 端出旧界面。
- ⚠️ **改了 `crates/yekitrim-polish/templates/index.html` 必须重建二进制**：它经 `include_str!` 进
  `yekitrim-polish`（`render/server.rs::INDEX_HTML`）——「改完没反应」十有八九是没重建/没重启。
- 改样式同样要重建 `webui/dist`（本目录只写前端；样式纪律见 `src/styles/AGENTS.md`）。
- ⚠️ **别在任何探针/判据里写死 `data-v-<hash>`**：scope id = hash(路径 + 源码)，改一句模板文案就轮换该组件
  全部选择器 id、连带 `dist` 里 CSS 文件名的 hash —— 那**不等于样式改了**；比对产物先归一化 `data-v-\w{8}`。
- 契约脚本的漂移判据都写在脚本自己的头部注释里，**改脚本前先读它**（它们是门禁，不是工具）。

> 改动只允许落在本目录；跨模块改动先读对方卡，并跑两条门禁。

## 验证

```bash
npm --prefix webui run build            # 构建门禁（缺它 dist 就是旧的，编译还会失败）
for f in webui/scripts/check-*.mjs scripts/check-preview-audio.mjs; do node "$f" || break; done
```

`npm run build` 成功 = `webui/dist/` 重新生成（`emptyOutDir` 会清空旧 hash 名）；**dist 入库，
必须和 `src/` 的改动一起提交**。十四个 node 脚本（清单见「入口」）各自打印项数与结果，退出码非 0 即失败；
它们零依赖、不 import 应用模块。