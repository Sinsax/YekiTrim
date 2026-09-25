# vtrim-render-core — GPU 绘制原语（被 polish / wasm 用）

> 范围：`crates/vtrim-render-core`/** ｜ 上游：`vtrim-polish`（原生渲染链）、`crates/vtrim-wasm`（浏览器内真渲染）｜ 验证：`cargo check -p vtrim-render-core`

## 职责

纯绘制/几何原语 + 时间线求值：`wgpu` 后端（`draw` / `effects` / `render` / `composite` / `sprite` /
`gpu`）、CPU 参考实现（`timeline` / `easing` / `legacy` / `types` / `gif` / `shaders`）。
它**不认识业务**：不读 `polish.toml`、不做 LLM、不碰网络。

`wgpu` 是**可选 feature**：`wgpu = ["dep:wgpu", "dep:bytemuck", "dep:swash"]`。
`draw/effects/render/composite/sprite/gpu` 全在 `#[cfg(feature = "wgpu")]` 门后；
`timeline.rs` 与 `easing.rs` **不在** feature 门后，默认构建也会编译。

## 入口

- `types.rs`：
  - `CameraStyle::factor` —— **运镜倍率只有这一张表**（三档 `1.3 / 1.5 / 0.55`）：模板 canvas、
    模板 GSAP、`webui/src/polish/engine.js`、`timeline.rs`（wgpu，未启用）四处都走它。改倍率改这里。
  - `handheld_sway` —— 手持摇摆的**固定参数表**（横纵位移各两路低频正弦）：Rust ↔ JS 的一致性由
    `timeline.rs::handheld_sway_matches_js_reference` 的采样点钉住。
  - 安全区读取优先级 `free_rect()`（自由选框 → grid → origin）与 `grid_rect`（必须与
    `templates/index.html` 的 `liverOrigin` 对齐）。
- `timeline.rs`：`overlay_stops`（**覆盖层契约的全文就在它的文档注释里** —— 一条规则、四处实现）、
  `get_overlay_state`（覆盖层求值：`color`/`gradient` 走 0.4s 进出包络，`fade_in`/`fade_out` 是**整窗
  线性斜坡** —— 分支必须排在包络之前，叠了起点不透明度会乘成 0、黑场打不开；内建片头黑场
  `intro_black` 已整条退役，片头那份由 `process` 只在初次后处理补 `fade_in` 事件）、
  `upgrade_event_times`（**在反序列化前**把 `end = time + duration` 换算好）、
  `handheld_sway_matches_js_reference`（Rust↔JS 采样点对拍）。
- `legacy.rs`：老工程字段兼容（与 `upgrade_event_times` 同一条"读时兜底"路线）。
- `easing.rs`：缓动函数；`gif.rs` / `shaders.rs`：素材与 shader 资源。

## 边界

- ⚠️ **wgpu 真渲染：暂不使用，且明确不在考虑范围内（用户决定，2026-09）。**
  默认构建不含它：`vtrim-polish` 是 `default = ["chrome-render"]`，`wgpu-render` 要显式开；
  `vtrim-cli` / `vtrim-server` 只做 feature **转发**，使用点在 `vtrim-server`
  （`api/` ×3、`model.rs` ×5），前端按 `/api/health.wgpu_render` 隐藏入口。
  涉及文件：`vtrim-polish/src/render/native/`、本 crate 的 `draw/effects/render/composite/sprite/gpu`、
  `vtrim-server/src/api/` 的单帧预览接口。**漏掉转发不报错**，只会让 `wgpu_render` 报 false、
  GPU 后端静默不编译进去 —— 动构建特性要实测这一条。
- ⚠️ **`timeline.rs` 不在 feature 门后，但它的唯一非测试消费者就是 wgpu**（原生走
  `render/native/mod.rs`，浏览器走 `vtrim-wasm` —— 它启用了 `vtrim-render-core/wgpu`，
  而 wasm 产物在默认构建里被 `wgpu_render` 能力位隐藏，所以默认构建下整条路都不可达）。
  默认构建里 `timeline.rs` 照常编译、跑单测，**但不代表任何出片行为**。
- ⚠️ **别拿 `timeline.rs` 当预览的参照**：那套求值与 `webui/src/polish/engine.js` 长得像，
  但**只有 `engine.js` 代表预览**，出片的权威是 `crates/vtrim-polish/templates/index.html`。
  运镜的聚焦/位移/画中画/手持摇摆为了不留半截实现，`timeline.rs` 里也写了（并配了单测），
  **但以后不必再重复这件事**（手持摇摆例外一半：`draw.rs` 的 wgpu 消费侧仍写死 `handheld: 0.0`，
  只求值、不落地，与整条 wgpu 链的"未启用"一致）。
- `cargo test -p vtrim-render-core --lib` **只跑 `timeline.rs` + `easing.rs` 的单测**；
  `draw.rs` 等要 `--features wgpu` 才编译。**别把它当成成片行为的回归网。**
- **`cargo check --workspace --all-targets` 是通过的**（2026-09-20 实测：exit 0、0 error、0 warning）。
  本 crate **没有 `examples/` 目录**，所以旧文档里那条"`examples/` 的
  `text_render`/`text_visual`/`text_layout`/`text_dump` 引用 `vtrim_render_core::composite`（只在 `wgpu`
  feature 下存在）→ 默认构建下 `--all-targets` 必然报 4 个错"**已经失效**（那是历史说明，实测推翻）。
  真要动 `draw` / `effects` / `composite` / `sprite` / `gpu` 时才需要 `--features wgpu`。
- > 改动只允许落在本目录；跨模块改动先读对方卡，并跑两条门禁。

## 验证

```bash
cargo check -p vtrim-render-core
cargo test -p vtrim-render-core --lib        # timeline + easing 的单测（不是出片行为的回归网）
cargo test -p vtrim-render-core --features wgpu   # 要动 draw/effects/composite 时才跑
```

`easing.rs` / `timeline.rs` 的改动会影响 `webui/src/polish/engine.js` 与出片模板里对应的移植副本 ——
改完同时跑 `node webui/scripts/check-handheld.mjs` 与 `node webui/scripts/check-overlay.mjs`。
