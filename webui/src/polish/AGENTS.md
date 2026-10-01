# webui/src/polish — 精修共享模块：预览求值器 / 画布参数 / SFX 播放 / 字体

> 范围：`webui/src/polish`/** ｜ 上游：`views/PolishView.vue`、`components/PolishPreview.vue`、`TimelineEditor.vue`、`CoverDesigner.vue` ｜ 验证：`node webui/scripts/check-overlay.mjs`

## 职责

本目录**已有一份 `README.md`**（`api.js` / `engine.js` / `audio.js` 的三行说明 + 那条移植纪律）
—— **动手前先读它**。与 Rust 侧的对应关系：`engine.js` 是出片模板
`crates/vtrim-polish/templates/index.html` 的 Canvas 渲染器**移植**（纯函数特效状态机 + 各 `*At(idx, t)`
预览求值器），**`templates/index.html` 才是真实出片路径**（headless Chrome 渲染）。
⚠️ 曾经还有第三条 `vtrim-render-core/src/timeline.rs`（wgpu 路径）—— 那条链连同 `vtrim-wasm` 已整体删除
（GPU 那侧交给独立的 Dhampir 底座）；`timeline.rs` 还在，但只剩 CPU 参考与覆盖层契约的正文，**不是预览的参照**。
`canvas.js` 是画布参数 SSOT，
`audio.js` 是 SFX 播放器与全应用共用的音量读写，`fonts.js` 按需加载项目字体。

## 入口

- `engine.js`：`buildIndex({config, subtitles, danmaku, duration, stickerSrc})` 建索引 + 全部求值器
  （`cameraAt` / `videoFxAt` / `getFlash` / `overlayStops` / `overlayFadeStop` / `gradientCss` /
  `getOverlayState` / `subsAt` / `capMeasure` / `wrapCaptionParts` / `danmakuAt` / `stickersAt` /
  `handheldSway` / `camFactor` / `evDuration` / `gridOrigin` / `freeRect` / `regionRect` / `regionOrigin`）、
  `GRID_RECTS` / `TRANSIENT_CAP` / `STICKER_BASE` / `STICKER_MARGIN_BASE`。
- `canvas.js`：`BASE_W` / `BASE_H` / `canvasOf(cfg)` / `stickerScale` / `stickerSize` / `stickerMargin` /
  `stickerFloatUp`（**画布参数 SSOT**，别在组件里另算一份）。
- `audio.js`：`readMasterVolume` / `readMuted` / `saveMasterVolume` / `saveMuted` /
  `createSfxPlayer({resolveUrl})`（localStorage `vtrim_master_vol` / `vtrim_muted`）。
- `fonts.js`：`ensureFontFace(pid, dir, id, {onDebug})`；`api.js`：polish 端点 + `watchJob`。
- 漂移对拍脚本：`webui/scripts/check-overlay.mjs`（覆盖层色档）、`check-caption-wrap.mjs`（折行）、
  `check-sticker-anchor.mjs`（贴纸锚点/尺寸）、`check-handheld.mjs`（手持摇摆）、
  `check-stage-backdrop.mjs`（画面构成/衬底）。它们**从源码文本里扒函数**、零依赖、不 import 本模块。

## 边界

- ⚠ **改特效表现先改 `crates/vtrim-polish/templates/index.html`，再同步移植到这里**：预览与出片
  必须一致（漂移的表现就是"面板里设了、预览/成片里没有"或"预览与成片断行/落点不一致"）。
- **覆盖层（overlay）的契约 = 一条规则，四处实现**（`timeline.rs::overlay_stops` 的文档注释里有全文）：
  `overlay_type == "gradient"` 的有效色档是 —— `colors` 滤掉空串后 ≥2 档就用它 / 恰好 1 档则用它 +
  同色零透明凑两档 / 没有 `colors` 但有 `color` 则 `[color, 同色零透明]` / 都没有就不渲染；
  `"color"` 以 `color` 为准、缺失时退回 `colors[0]`。**单色一律「该色 → 透明」**（用户定的语义）。
  **`fade_in` / `fade_out`**（黑场淡入/淡出，2026-09）走另一条口：整窗就是过渡（`fade_in` 起点全遮 →
  末端全透 = `opacity × (1 − dt/dur)`，`fade_out` 镜像），**不叠 0.4s 包络**（分支排在包络之前判），
  `color` 缺省 `#000000`、`opacity` = 遮罩最大不透明度。内建片头黑场 `intro-black` 已整条退役，
  片头那份是 `process` **只在初次后处理**补的 `fade_in` 事件（可拖可删、删掉后不再补，见
  `process/AGENTS.md` Step 3c）。
  ⚠ 求值器**必须保证返回 ≥2 档且无空串**（出片路径 `n == 1` 是 `0/0 = NaN` 抛 `IndexSizeError`，
  空串色档抛 `SyntaxError`，两者都会**打断整个 chunk 的渲染**）；⚠ 兜底出来的第二档必须是
  **同色零透明**（`overlayFadeStop`），**不能用 CSS 的 `transparent`** —— canvas 渐变在非预乘空间
  插值会插出暗红，而 CSS 那边两者等价，写 `transparent` 恰好让**浏览器预览与出片观感不一致**。
  四处实现：`vtrim-render-core/src/timeline.rs`、`templates/index.html`（**出片权威路径**）、
  `engine.js`、`PolishPreview.vue`（经 engine 的 `gradientCss`）。漂移检查跑
  `node webui/scripts/check-overlay.mjs`。
- ⚠ 两处 DOM/GSAP 预览的渐变 CSS **结尾那个 `)` 不能省** —— 少了它整条 `background` 非法被浏览器
  丢弃，表现正是"面板里设了色档、预览里什么都没有"（`engine.js::gradientCss` 一份，模板里
  `overlayGradientCss` 一份）。径向语义 = **中心该色、边缘淡出**。
- ⚠ 不透明度一律用**数值判定**（`typeof ev.opacity === 'number'`），**别写 `ev.opacity || 1`**：
  0 是合法值，falsy 判定会把它变成全不透明。
- **字幕限宽折行（A5）同样是两份实现**：出片权威 `templates/index.html::wrapCaptionParts` 与
  `engine.js::wrapCaptionParts`（经 `PolishPreview.vue` 的 `capRowsWrapped`）必须**逐字一致**。
  上限 3 行（`MAX_LINES`）、超 3 行按 `MIN_RATIO=0.7` 缩字号；安全宽 = 画布宽 × 0.875（竖屏 945px）。
  ⚠ 曾经 `append` 先读 `cur.length` 再判溢出，而溢出分支的 `pushLine()` 会把 `cur` 置空 →
  **任何一次折行都抛 `TypeError: can't access property "length", cur is null`**（竖屏安全宽只有
  945px，普通长字幕必折行）。改一边就同时改另一边，并用 `node webui/scripts/check-caption-wrap.mjs`
  对拍（21 项 + 4 处变异自证）。
- ⚠ **手持摇摆 `handheld` 一份参数表 + 两个 JS 副本**（模板 `handheldSway` / `engine.js handheldSway`）：
  Rust↔JS 由 `timeline.rs::handheld_sway_matches_js_reference` 的采样点钉住，两个 JS 由
  `node webui/scripts/check-handheld.mjs` 对拍（**改常数三处必须同步**；脚本自带扰动自证）。
  波形是**纯 `t` 的函数**（无累积状态）；⚠ **竖屏（`vertical = true`）跟运镜一起屏蔽**：
  `engine.js cameraAt` 的竖屏分支返回 `handheld: 0`（四条求值路径全部归零，DOM 预览层共用
  `window.__CAM_STATE` 自动跟随）。transform 组装顺序三处一致：`translate(sway)` → `rotate(sway)` →
  `scale(zoom × sway.scale)`。
- **贴纸尺寸与落点口径（四条路径同源）**：尺寸基准 = **画布短边** `stickerScale(cv)` =
  `min(W,H)/1080`（`canvas.js`，`stickerSize/Margin/FloatUp` 都乘它）——**竖屏也是 280px**，
  ⚠ 别改成 `W/1920`（那会按宽缩小、竖屏贴纸变小）；字幕/弹幕字号仍**不随画布缩放**，别顺手"统一"。
  ⚠ **两个锚点基准禁止"统一"**：事件 `ev.x/ev.y`（auto/placement 产出）是**盒子左上角**百分比；
  `[sticker] position`（manual = 用户点选的固定落点）是**盒子中心**百分比。契约测试
  `node webui/scripts/check-sticker-anchor.mjs`（64 项 + 4 处变异自证）。
- **画面构成（`[canvas] orientation` + `[stage] position / bg_mode / scale / padding`）与"衬底"**：
  摆放与背景由 `ProjectFile::resolve_stage(src)` **唯一推导**，本模块只读注入的 `_canvas.stage`。
  ⚠ 竖屏出片的模糊衬底有两道闸缺一不可：① `resolve_stage` 的背景门控是**画布方向**、不是 `axis`
  （`axis=="fill" && !portrait` 才 `none`）；② **出片是 canvas `renderFrame` 逐帧绘制，DOM 的
  `#bg-video-wrap` 完全不参与导出** —— 只把 `stage.bg` 消费在 DOM 上，表现就是「预览里有模糊底、
  成片被 shake / 画中画 / 手持摇摆挪开露纯黑」（不报错、不崩溃，只有逐帧像素能发现）。
  钉子：`node webui/scripts/check-stage-backdrop.mjs`（34 项 + 5 处变异自证）。模糊半径/倍率一律取
  契约里的值，**别写死 28px**。
- ⚠ **区域读取优先级（`engine.js regionRect/regionOrigin`，须与 Rust 一致）**：`free_rect()` →
  `region_rects()`/`region_origin()` 一律**自由选框 > grid > `origin_grid`/`origin`**；
  `GRID_RECTS` 表须与 Rust `grid_rect` 对齐。
- ⚠ **音量 0 是合法值**：读回音量用 `isFinite` + clamp，**不能写 `parseFloat(...) || 1`**（falsy
  判定会把它变成 1）；音量/静音走**全应用共用**的 `vtrim_master_vol` / `vtrim_muted`，读写只走
  `audio.js` 导出的 `readMasterVolume/readMuted/saveMasterVolume/saveMuted`（别各写一份），
  且必须同时落到 `<video>` 元素（只调 SFX 播放器就是 bug）。
- ⚠ **出片逐帧路径里任何 `ReferenceError` 都是"整块 chunk 失败 → 整个 render job 失败"**：
  改了 `renderFrame` 附近（模板与 engine 同构）后至少跑一次带字幕的出片 —— 症状是**所有带字幕的
  片段都导不出来、无字幕的正常**。`check-stage-backdrop.mjs` 的 ③b 段钉住这类裸变量（判据走
  **去注释后的源码**，注释里出现同样字样会把判据自己顶翻）。

- ⚠️ **事故（2026-10-01）：删效果类型时"同一个手法"在三处留下连带损伤，预览整块消失**
  （用户报"视频和控件一起消失"，而服务端日志全是 200 —— 因为 Vue 遇到 render/setup 抛错会**卸载整块子树**）：
  ① `buildIndex` 的**桶清单**丢了 `vignette`/`sfx`（模板那份是全的）—— 桶是**按需创建**的，
  工程里恰好没有该类型事件时它就是 `undefined` ⇒ `for (const v of idx.vignette)` 抛
  `Symbol.iterator ... undefined`（实测那个工程有**五个**类型没有事件 ⇒ 会连炸五处）；
  ② `videoFxAt` 的**返回对象**被削成 `{shake, blur}` ⇒ 预览读 `f.camera.handheld` 抛
  `can't access property "handheld", i.camera is undefined`；
  ③ 词汇表 `overlay.ui` 写成**对象** `{}` ⇒ 编辑器 `(e.ui ?? []).includes()` 抛
  `includes is not a function`。
  **防它**：桶清单只许有一份（`INDEXED_TYPES`，初始化与排序共用；必须与 `templates/index.html` 同集合）、
  删字段前先 grep 消费者、`ui` 只许是数组 —— 三条都由 **`node scripts/check-preview-engine.mjs`**
  真跑一遍钉住（它 import 本模块、用**空工程**建索引跑那一族求值器；`--self-test` 证明删桶会抛）。
  **静态读源码抓不住这一类**，所以它在门禁链里（`.workbuddy/gates.mjs`）。

> 改动只允许落在本目录；跨模块改动先读对方卡，并跑两条门禁。

## 验证

```bash
node webui/scripts/check-overlay.mjs
node webui/scripts/check-caption-wrap.mjs
node webui/scripts/check-sticker-anchor.mjs
node webui/scripts/check-handheld.mjs
node webui/scripts/check-stage-backdrop.mjs
npm --prefix webui run build
```

前五条是漂移门禁（各打印对拍项数与变异自证，退出码非 0 即漂移）；**动 `engine.js` 就要跑全**——
它们把本模块与出片模板的每一份同口径算法钉在一起。最后一条重建 `webui/dist`（改了前端必须一起提交）。
