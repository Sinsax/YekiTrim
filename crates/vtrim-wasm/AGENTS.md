# vtrim-wasm — wgpu 渲染核心的 WASM 绑定（浏览器实时真渲染预览）

> 范围：`crates/vtrim-wasm`/** ｜ 上游：`webui/src/wasm/`（前端那份拷贝，`PolishPreview.vue` 用）｜ 验证：`cargo check --manifest-path crates/vtrim-wasm/Cargo.toml --target wasm32-unknown-unknown`

## 职责

把 `vtrim-render-core` 的绘制/合成管线包成浏览器可调用的绑定：WebGPU 设备 + surface + 事件索引只初始化一次，
之后每个 `requestAnimationFrame` 把 `<video>` 的当前帧拷进 GPU，走**与桌面出片完全相同的合成管线**，
直接 present 到 canvas。JS 侧流程（`src/lib.rs` 文档头）：

1. `init_renderer(canvas, configJson, subsJson, dmsJson, …)` —— 初始化 WebGPU 设备 + surface + 事件索引（一次）；
2. `set_sticker(name, data, isGif)` —— 注入贴纸字节（可多次，同名覆盖）；
3. 每帧：`render_frame(t, videoElement)`。

浏览器要求 WebGPU（Chrome/Edge 113+ / Firefox 141+ / Safari 26+）。

## 入口

- `init_renderer(canvas, config_json, subs_json, dms_json, font_name, font_bytes, sticker_names,
  sticker_datas, sticker_is_gifs, width, height, vertical, vertical_position) -> Result<WasmRenderer, JsValue>`
  （`async`）。贴纸以**平行数组**传入：`names[i]` ↔ `datas[i]`（`Uint8Array`）↔ `is_gifs[i]`（0/1）。
- `WasmRenderer::{set_sticker(name, data, is_gif), set_subtitles(subs_json), render_frame(t, video),
  dispose(), canvas()}`。`set_subtitles` 是**热更新**：精修编辑器里改字幕不必重建渲染器。
- `src/wasm/` = wasm-bindgen 生成物（`vtrim_wasm.js` / `vtrim_wasm.d.ts` / `vtrim_wasm_bg.wasm` /
  `vtrim_wasm_bg.wasm.d.ts` / `package.json`）；`webui/src/wasm/` 是**前端实际 import 的那份拷贝**。
- crate 元数据：`[lib] crate-type = ["cdylib", "rlib"]`，`edition = "2024"`，
  依赖 `vtrim-render-core`（`features = ["wgpu"]`）+ `wgpu 27` + `wasm-bindgen` / `wasm-bindgen-futures` /
  `js-sys` / `web-sys`（`Window` / `HtmlCanvasElement` / `HtmlVideoElement`）/ `serde` / `serde_json` /
  `console_error_panic_hook`。

## 边界

- ⚠️ **本 crate 不在 workspace members 里**（独立 target：自己一份 `Cargo.toml` 里有 `[workspace]`，
  自己的 `Cargo.lock`，构建产物在 `crates/vtrim-wasm/target/wasm32-unknown-unknown/release/`）。
  所以 `cargo check`/`build` 必须带 `--manifest-path crates/vtrim-wasm/Cargo.toml`，
  根 workspace 的 `cargo check --workspace` **不会**覆盖它。
- **只依赖 `vtrim-render-core`**，不许依赖 `vtrim-core` / `vtrim-polish` / `vtrim-service` /
  `vtrim-server`：浏览器侧只跑绘制原语，业务（段/贴纸挑选/预算）在 Rust 服务端算好后以 JSON 传进来。
- ⚠️ **`src/wasm/` 是 wasm-bindgen 生成物，不许手改**（根卡列的生成物清单里就有它）。
  前端用的是 `webui/src/wasm/` 那一份**拷贝** —— 改绑定后两边都要重新生成，别只更新一处。
  （`package.json` 也是 wasm-pack 生成的模板：`type: module`、`main: vtrim_wasm.js`。）
- ⚠️ **改了 `vtrim-render-core` 的导出、或任何进 `WasmRenderer` 的 API，就要重新生成**这份产物；
  否则浏览器侧编译不上（或静默用旧版行为）。
- ⚠️ **wgpu 真渲染：暂不使用，且明确不在考虑范围内（用户决定，2026-09）。**
  改动渲染/运镜/特效语义时**不要为它做任何同步**，也不用去修它的测试。理由与边界：
  - `vtrim-polish` 默认是 `chrome-render`（出片的事实来源是 `templates/index.html` 的 canvas 路径）；
  - `vtrim-render-core/timeline.rs` 不在 feature 门后，但它的唯一非测试消费者就是 wgpu ——
    它是一份**平行实现，不是行为的事实来源**（改语义要同步 `templates/index.html` + `engine.js` +
    `PolishPreview.vue` 三处即可）；
  - 前端入口按 `/api/health.wgpu_render` 能力位隐藏，默认构建下整条路都不可达。
- `wasm-opt = false`（`[package.metadata.wasm-pack]` 的 dev 与 release 两档都关）。
- `init_renderer` 里解析 `config_json` 时**先跑 `rcore::legacy::upgrade_event_times`**（老 `duration` 字段 →
  `end`，与 `polish.toml` 同一条升级路径）—— 别把这一步删掉。
- 字体字节由 **JS 侧 fetch 后传入**（`rcore::composite::set_font_bytes(&font_name, Some(bytes))`，
  与桌面出片的字体注入走同一份实现）；贴纸字节同样由 JS 传入后在 Rust 侧解码
  （`rcore::render::decode_sticker_asset`），失败只是那个贴纸缺席、不影响整帧。
- `render_frame` 用**一个 encoder、最后只 submit 一次**（减少 wasm 实时预览的 GPU 提交开销）：
  source pass 离屏渲染 → effect pass 读 source 合成到 surface → sprite pass 直接叠加到 surface，
  **与桌面出片的 pass 顺序一致**（别在这里另发明一套顺序，否则预览与成片会分叉）。
- `dispose()` 只清贴纸缓存（降级/出错时由 JS 调用），不是完整的 surface 释放。

> 改动只允许落在本目录；跨模块改动先读对方卡，并跑两条门禁。

## 验证

```bash
# 注册表（docs/module-map.json）里登记的就是这一条 —— 两个参数都不能丢，见下
cargo check --manifest-path crates/vtrim-wasm/Cargo.toml --target wasm32-unknown-unknown
```

⚠️ **两个参数缺一不可**：必须带 `--manifest-path`（独立 workspace，根 workspace 不覆盖它），
也必须带 `--target wasm32-unknown-unknown` —— `lib.rs` 用的 `SurfaceTarget::Canvas` /
`Queue::copy_external_image_to_texture` / `CopyExternalImageSourceInfo` 都是 wgpu 在 `#[cfg(web)]`
下才有的 API，**host target 上这几处必然报 E0599**（`--target` 缺省是宿主三元组）。
这是本 crate 的既有事实，不是可以"顺手修掉"的错：它本来只服务浏览器，宿主构建不产生任何可用产物。
真出浏览器产物要走 wasm-bindgen，生成后把 `vtrim_wasm.js` / `.d.ts` / `_bg.wasm` 同步到 `webui/src/wasm/`。
前端侧的效果验证在 `PolishPreview.vue` 的「真渲染」按钮（需 `wgpu_render` 为 true）。
