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
  起不了 npm 脚本时直接调 vite：`node webui/node_modules/vite/bin/vite.js build`（**在 `webui/` 下跑**）。
- `webui/scripts/` **九个**静态判据脚本（零依赖、不 import 应用模块、不需要起服务）：
  - `check-overlay.mjs` — 覆盖层色档：`templates/index.html` ↔ `src/polish/engine.js` 逐例对拍。
  - `check-caption-wrap.mjs` — 竖屏折行：`wrapCaptionParts` 两份实现。
  - `check-sticker-anchor.mjs` — 贴纸锚点/尺寸（事件左上角百分比 vs `[sticker] position` 中心百分比）。
  - `check-cover-stage.mjs` — 封面舞台：黑边覆盖 + 恒横版 + 模糊同源 + 类名穿透 + 图层栈
    （`.pane` 不许复用、`.lyr-props` 要 `min-height`、`COVER_HTML` 只替换一次、`#fx-extra` 无 z-index）。
  - `check-handheld.mjs` — 手持摇摆：波形逐值对拍 + 起停状态机 + 竖素材进非竖画布归零。
  - `../scripts/check-camera-zoom-parity.mjs` — **运镜档位表五处逐值一致**（§18-C）：三档默认倍率
    （`render-core` / 模板 `camFactor` / `engine.js` / 转译器）+ 后端夹取区间 + 前端滑杆同值。
    任一路漏改的症状是"预览与成片用不同倍率"，两边各自看都正常。**8 处变异自证**。
  - `check-stage-backdrop.mjs` — 画面构成/衬底：四种模式 + 图层序 + 预览 DOM 层 + 两轴摆位。
  - `check-preview-ab.mjs` — 跳屏蔽区间的 A/B 预滚（换台顺序、Dhampir 闸门）。
  - `check-grid-parity.mjs` — **九宫格四方对拍**：`GRID_RECTS` 与 Rust `grid_rect` **逐值**一致、
    **格子中心**四份实现互差 ≤0.02%（Rust / engine / 模板**手抄表** / 转译器 `1/6`）。
    这条规矩过去**只有人读没有机器判据** —— 改一处不改其余三处的症状是「预览选中的区域与出片
    锚住的位置差一点点」且**不报错**。带两条变异自证。
  - `check-layout-invariants.mjs` — **布局不变式**（2026-10）：`.llm-cols` 两栏等宽 · 半宽列里滑块跨满
    整行（否则滑条压到 59px）· `.ov-cols` 单栏态 · `extra_params` `rows ≥ 10` · `ModelList` 下载按
    admin 门控 · 模板标识符必须有定义（Vue 对未定义变量**不报错**）。⚠️ 每条带**变异自证**。
  - `check-text-tilt.mjs` — **文字倾斜/压缩 预览↔成片**（T4）：`engine.js::textTiltTransform`
    拼出的 CSS transform 折成矩阵后，必须与 Dhampir `text_transform.rs::blit_affine` 的正向
    **`R(rot)·S(sx,sy)` 逐值一致**。⚠️ 这条规矩的**唯一**内容就是"乘法顺序"：
    CSS 从右往左 ⇒ 必须写 `rotate(...) scale(...)`（rotate 在左）。
    写成 `scale(...) rotate(...)` 得 `S·R`，斜着的字被沿**旋转后的轴**拉伸（实测线性差 1.25e-2）。
    ⚠️ **别拿退化用例试**：`squashY==1` 或 `tilt==0` 时两种顺序相等 —— 四档里
    `chapter`/`comment` 恰是 `tilt=0,squash=1`，**默认那两档永远试不出来**。6 处变异自证。
  - `../scripts/check-preview-audio.mjs` — 预览**出声**契约（住 `scripts/`，仓里**没有** `gates.mjs`
    这个聚合脚本 —— 逐个跑即可）。
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
npm --prefix webui run build            # 构建门禁（缺它 dist 就是旧的，编译还会失败）
for f in webui/scripts/check-*.mjs scripts/check-preview-audio.mjs; do node "$f" || break; done
```

`npm run build` 成功 = `webui/dist/` 重新生成（`emptyOutDir` 会清空旧 hash 名）；**dist 入库，
必须和 `src/` 的改动一起提交**。九个 node 脚本（清单见「入口」）各自打印对拍项数与通过情况，
退出码非 0 即失败；它们零依赖、不 import 应用模块（`webui/package.json` 没有 `"type": "module"`）。
