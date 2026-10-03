# yekitrim-render-core — 事件/时间线**类型**与时间线数学（不含 GPU）

> 范围：`crates/yekitrim-render-core`/** ｜ 上游：`crates/yekitrim-polish`（`Event` / `Segment` / `CameraStyle` 就是它的字段类型）｜ 验证：`cargo check -p yekitrim-render-core`

## 职责

`polish.toml` 的**类型真源** + 时间线数学：`types.rs`（`Event` / `Segment` / `CameraStyle` / `Point` …，
被 `yekitrim-polish` 直接 `pub use` 成自己的字段类型）、`legacy.rs`（老工程 `duration` → `end` 的读时升级）、
`timeline.rs` + `easing.rs`（CPU 参考求值与缓动）。
本 crate **不认识业务**：不读 `polish.toml`、不做 LLM、不碰网络，**也不含任何 GPU 代码**。

> ⚠️ **这里曾经有一整套 wgpu 合成管线**（`draw` / `effects` / `render` / `composite` / `sprite` / `gpu` /
> `shaders` / `gif`，由 `wgpu` feature 门控）、一个浏览器真渲染预览 `yekitrim-wasm`（独立 target）、
> 以及桌面 `yekitrim-polish/src/render/native/` 的 `WgpuBackend`。**整条已删除**（用户拍板：
> GPU 出片与本仓无关，那侧交给独立的 **Dhampir 底座**，见根卡「本仓不再自带任何 GPU 渲染」）。
> 依赖也一并收干净（`wgpu` / `bytemuck` / `swash` / `image` / `futures` 都没了，只剩 `serde` /
> `serde_json` / `tracing`）。**别再往这里加回任何 GPU 管线、feature 或可选依赖。**

## 入口

- `types.rs`：
  - `CameraStyle::factor` —— **运镜倍率只有这一张表**（三档 `1.3 / 1.5 / 0.55`）：模板 canvas、
    模板 GSAP、`webui/src/polish/engine.js` 三处都走它。改倍率改这里。
  - `Event`（`Sticker` / `Camera` / `Sfx` / `Overlay` + 九种瞬时特效）、`Segment`、`Point`；
    事件字段是 `time` + `end`（**没有 `duration`**）；`manual` / `cut` 这些标记的语义见 `polish` 各卡。
  - 安全区读取优先级 `free_rect()`（自由选框 → grid → origin）与 `grid_rect`（必须与
    `templates/index.html` 的 `liverOrigin` 对齐）。
  - `handheld_sway` 的**固定参数表**（横纵位移各两路低频正弦）：Rust ↔ JS 的一致性由
    `timeline.rs::handheld_sway_matches_js_reference` 的采样点钉住。
- `timeline.rs`：`overlay_stops`（**覆盖层契约的全文就在它的文档注释里** —— 一条规则、三处实现：
  本文件、`templates/index.html`（**出片权威路径**）、`engine.js`）、`get_overlay_state`（覆盖层求值：
  `color`/`gradient` 走 0.4s 进出包络；`fade_in`/`fade_out` 是**整窗线性斜坡**，分支必须排在包络之前）、
  `upgrade_event_times`（**在反序列化前**把 `end = time + duration` 换算好）。
- `legacy.rs`：老工程字段兼容（与 `upgrade_event_times` 同一条"读时兜底"路线）。
- `easing.rs`：缓动函数。

## 边界

- ⚠️ **`timeline.rs` 现在没有生产消费者**（wgpu 那条路删掉之后，只剩它自己的单测与
  `yekitrim-polish/src/process/mod.rs` 里对 `overlay_stops` 的断言在用）。留着它有两个理由：
  ① `overlay_stops` 的文档注释是**覆盖层契约的正文**（另两处实现按它对齐）；
  ② 它是 CPU 参考实现，改语义时用来对拍。
  **它不是行为的事实来源** —— 事实来源永远是 `templates/index.html` 的 canvas 路径。
  改渲染/运镜/特效语义时**不要为它做同步**（要同步的只有 `templates/index.html` + `engine.js`
  + `PolishPreview.vue` 三处）。
- ⚠️ **别拿 `timeline.rs` 当预览的参照**：那套求值与 `webui/src/polish/engine.js` 长得像，
  但**只有 `engine.js` 代表预览**，出片权威是 `crates/yekitrim-polish/templates/index.html`。
  运镜的聚焦/位移/画中画/手持摇摆在这里也写了一份（配了单测），**但以后不必再重复这件事**。
- `cargo test -p yekitrim-render-core --lib` **只跑 `timeline.rs` + `easing.rs` 的单测** ——
  **别把它当成成片行为的回归网**（成片行为的守卫在 `cargo test -p yekitrim-polish` 与
  `webui/scripts/check-*.mjs` 那几条漂移对拍上）。
- 本 crate 的依赖只有 `serde` / `serde_json` / `tracing`。要加依赖先问一句"这是不是业务"——
  业务属于 `yekitrim-polish`。

> 改动只允许落在本目录；跨模块改动先读对方卡，并跑两条门禁。

## 验证

```bash
cargo check -p yekitrim-render-core
cargo test -p yekitrim-render-core --lib        # timeline + easing 的单测（不是出片行为的回归网）
```

`types.rs` 的字段改动会直接影响 `polish.toml` 的读写（**序列化格式就是文件格式**），
改完必须跑 `cargo test -p yekitrim-polish --lib project`；`easing.rs` / `timeline.rs` 的改动会让
`engine.js` 与出片模板里对应的移植副本漂移 —— 改完同时跑 `node webui/scripts/check-handheld.mjs`
与 `node webui/scripts/check-overlay.mjs`。
