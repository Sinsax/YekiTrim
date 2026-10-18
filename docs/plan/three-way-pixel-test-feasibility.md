# 三端像素级测试：可行性勘查报告

> 日期：2026-10。**结论：可行，且大部分设施已经存在。**
> 本文件只记**实测事实**与**未验项**，不含猜测。

---

## 1. ★ 最重要的发现：底座侧的「双端一致性」**已经建好并实测通过**

`Dhampir/scripts/check-dual-end.mjs` 是一条现成的编排命令，实测结果：

```
$ node scripts/check-dual-end.mjs
抽取 4 帧做双端比对：0, 30, 60, 89
[1/3] 浏览器逐帧导出 …
[2/3] worker 渲染同一份工程 …
[3/3] 比对 …
[
  {"frame": 0,  "ssim": 1.000000, "psnr_db": inf,     "mae": 0.0000},
  {"frame": 30, "ssim": 1.000000, "psnr_db": 100.506, "mae": 0.0000},
  {"frame": 60, "ssim": 1.000000, "psnr_db": inf,     "mae": 0.0000},
  {"frame": 89, "ssim": 1.000000, "psnr_db": inf,     "mae": 0.0000}
]
最差 SSIM = 1.000000
✓ 双端一致性通过
```

**它不是空转**（变异自证已做）：
- 把 native 侧 `frame-0030.png` 换成同样 640×360 的**合法 RGBA PNG、但像素不同**
  ⇒ **`ssim: 0.747646` / `mae: 119.9723`**，判定翻面 ✓
- 换回原图 ⇒ 恢复 `1.000000` ✓
- 篡改成非法 PNG ⇒ 解码报错拒收 ✓（不是"静默算成一样"）

### 1.1 它比的**是哪两端**（别搞错）

| 端 | 实现 | 入口 |
|---|---|---|
| **dhampir-web** | 浏览器 + wasm | `web/synthetic.html` → `dhampir_sample_project_render_png` |
| **dhampir-native** | Rust worker | `cargo run --example render_project` |

⇒ 这是**底座内部**的 wasm ↔ native 一致性。

### 1.2 为什么"输入一致"是**构造出来的**，不是声明出来的

`synthetic.html` 的注释说得最清楚：

> 它与 `web/app.js` 那条路的关键区别：**不吃 `<video>`**，而是调
> `dhampir_sample_project_render_png` —— 那个函数用 core 的
> `synthetic_source_rgba8 + synthetic_seed_for_source_frame` 造源图，
> 与 native 的 `render_project` 是**同一对函数**。

编排器头部还记着一个**踩过的坑**：早先用"浏览器解码真视频" vs "worker 合成源"
去比，**比的是两个不同的东西**，那个 SSIM 是**没有意义的**。所以现在两侧都走合成源。

---

## 2. ★ 另一个关键发现：`dhampir_sample_project_render_png` 是**自包含**入口

`crates/dhampir-wasm/src/timeline_host.rs:2290`：

```rust
pub async fn dhampir_sample_project_render_png(
    project_json: String, frame: i32, width: u32, height: u32,
) -> Result<Vec<u8>, JsValue>
```

源码注释（原话）：

> 刻意**不碰** thread_local 里的工程：这个入口要能被独立调用（驱动直接喂 JSON），
> 免得比对结果依赖"页面之前打开了什么"。

⇒ **喂 JSON + 帧号 + 尺寸 → 返回 PNG 字节**。
**不要 video、不要登录、不要 canvas、不要精修页。**

---

## 3. 免登录路径已实测可行

`yekitrim server --local-no-auth`（**三道闸**：只许 loopback、与 `--allow-remote` 互斥、
Host 必须 loopback 主机名、写请求必须同源）：

| 资源 | 无该开关 | 有该开关 |
|---|---|---|
| `/api/polish/dhampir-wasm/engine.js` | 401 | ✅ **200 / 75309 B / `text/javascript`** |
| `…/pkg/dhampir_wasm.js` | 401 | ✅ **200 / 111556 B / `text/javascript`** |
| `…/pkg/dhampir_wasm_bg.wasm` | 401 | ✅ **200 / 1539731 B / `application/wasm`** |

⚠️ 路径必须带 `/api` 前缀（路由挂在 `nest("/api", …)` 下）。
不带前缀会命中 **SPA 兜底**，返回 1007 B 的 HTML —— **那不是 404，是伪装成 200 的兜底**。

⚠️ `--local-no-auth` 让**同机任何进程**都能调这套 API。测试完记得关。

---

## 4. headless 能力已实测

| 项 | 结果 |
|---|---|
| headless Chrome | ✅ 可用（Chrome + Edge 都在） |
| **canvas 像素读回** | ✅ 写入 `(12,34,56)` → 读出 **`PX:12,34,56,255`** |
| **WebGPU adapter** | ✅ **`ADAPTER-OK`**（`--headless=new --enable-unsafe-webgpu`） |

★ 而且上面的双端测试**已经实际跑通了 wasm 出图** —— 那 4 张 PNG 是 wasm 真画出来的
（session 用 `dhampir_sample_project_render_png` + headless）。所以"wasm 在 headless 下
能不能出图"这个**最大的未知已经被回答：能**。

---

## 5. ⚠️ 必须纠正的两个**我先前说错**的地方

| 我先前的说法 | 实测事实 |
|---|---|
| "`render/mod.rs` 只剩 `pub mod dhampir`" | **11 个模块**。真正的证据是 `backend.rs::RenderBackendKind` 只剩 `Dhampir` 一个变体 |
| "golden 可能是底座冻的" | **不是**。`golden.json` 写着 `"frozen_from": "yekitrim polish render --renderer chrome"`，而 **Chrome 腿 2026-10-01 已整条删除** |

⇒ **`docs/golden/dhampir-parity/` 是"已删除的 DOM 腿"的历史快照**，
**不能当"底座真值"**。它只能回答"DOM 渲染有没有大改"。

---

## 6. 三端到底指哪三个（YekiTrim 口径）

| 端 | 实现 | 现状 |
|---|---|---|
| **A 本仓预览** | `webui/src/polish/engine.js` 在精修页里 | ⬜ 未做像素出口 |
| **B 底座预览** | 底座 wasm（同一份 wasm，浏览器里跑） | ✅ 双端测试已验证它出图与 native 一致 |
| **C 底座出片** | `dhampir.exe` native | ❌ **写盘被拦**（见 §7） |

★ `engine.js` 的自我定位（原话）：

> 这一层是「底座」的一部分…**框架无关**…不写业务规则 —— 校验走 Rust 的
> `dhampir_project_open`，求值走 `dhampir_project_frame`，渲染走 wasm 的时间线宿主。

⇒ **A 端不是独立实现，它是同一份 wasm 的另一个宿主**。
所以 A↔B 的差异**理论上只在"字形栅格化"**（CLI 走 ffmpeg drawtext、浏览器只有系统字体）。

---

## 7. 仍未通的一环：`dhampir.exe` 写盘

| 命令 | 写盘 | 结果 |
|---|---|---|
| `dhampir render` | mp4 | ❌ `Permission denied` |
| `dhampir frame` | PNG | ❌ `PermissionDenied (code 5)` |
| `dhampir probe` | 不写 | ✅ 正常 |

**已排除**：不是 ffmpeg（`frame` 不走它）、不是我改的代码（**未改动的 release 二进制同样失败**）、
不是路径权限（同目录 `python3` 能写）。

⇒ **浏览器侧的 wasm 路线完全不受影响**（不写盘）。受影响的只有"用 exe 出片"。

---

## 8. 结论与建议

**可行性：✅ 通。** 而且比我预估的省事得多 —— 不需要从零搭三端设施，
**底座侧的双端编排、自包含 PNG 入口、headless 驱动全都现成**。

建议的落法：
1. **底盘**用现成的 `check-dual-end.mjs`（dhampir-web ↔ dhampir-native），已实测 SSIM=1.0；
2. **A 端补一个像素出口**（engine.js 在 headless 里出帧）；
3. **登记 skill**：把"三端对应怎么测、坑在哪、阈值口径"记下来。

⚠️ 但**不要**把 golden（§5）当成三端里的"真值端"。

---

## 9. ★ 补充实测：两条腿**都能在 headless 下出图**

### 9.1 底座 wasm 腿（`web/synthetic.html`）—— ✅ 已由 §1 的双端测试证明

`check-dual-end.mjs` 出来的 4 张 PNG 就是它画的，SSIM=1.0。

### 9.2 wasm 宿主腿（`web/probe.html`）—— ✅ 实测通过

```
$ node scripts/web-check.mjs --probe
✓ W0 验收通过（工程帧能上 canvas，且由时间线驱动）
```

**它不是空转**：报告里带着**真实 PNG 长度**

```
"frames": [
  { "frame": 0,  "sources": [{"source":"a.mp4","source_frame":0,"seconds":0}],  "pngLength": 2654 },
  { "frame": 30, "sources": [3 个源各带自己的 source_frame],                    "pngLength": 9782 }
]
```

⇒ canvas 读回拿到了**非空像素**（2654 / 9782 字节的 PNG），不是黑屏。

### 9.3 ⚠️ 但要注意 `probe.html` **没走 `engine.js`**

它 `import("/pkg/dhampir_wasm.js")` **直接调 wasm**，绕过了 `engine.js`。

而 **YekiTrim 精修页的预览腿用的是 `engine.js`**（`web/ui` 那层宿主）。
两者是**同一个 wasm 的两个宿主**，但**宿主层不同**：

| | 入口 | 谁在用 |
|---|---|---|
| `probe.html` | 直接 `import` wasm | 底座自己的 W0 验收 |
| `effects-demo-wasm.html` | `loadEngine('/pkg/dhampir_wasm.js')` → `engine.seek()` | **与 YekiTrim 精修页同一条路** |

⇒ 若要覆盖 **A 端（YekiTrim 精修页那条腿）**，该参照的是
**`effects-demo-wasm.html`**（它走 `loadEngine` + `engine.seek` + `onPrepared` + `setSourceBitmap`），
**不是** `probe.html`。

### 9.4 `engine.js` 的公开接口（`loadEngine()` 返回 `Engine` 实例）

关键方法（`web/engine.js`）：

| 方法 | 用途 |
|---|---|
| `open(project)` / `validate(project)` | 装工程 |
| `attach(canvasId)` | 绑画布 |
| `bindSource(source, videoId)` / `registerVideo(source, video)` | 绑视频源 |
| `sourcesFor(frame)` | 这一帧要哪些源、各在第几秒 |
| **`async seek(frame, options)`** | seek 到某帧并画（`options.onPrepared` 是交静态位图的**唯一缝隙**） |
| `setSourceBitmap(source, bitmap)` | 交位图（**必须在 `onPrepared` 里**） |
| `resize(w, h)` | 尺寸 |
| `textFrame(frame)` / `loadSubtitles(entries)` | 字幕 |

⚠️ **`onPrepared` 那个缝隙是硬约束**（源码注释实测）：
`prepare` 的第一件事是 `clear_bitmaps()`，所以"seek 之前/之后随意交"会被下一次 prepare 清掉
—— 现象是「交与不交，画面逐字节相同」。

---

## 10. 可行性结论（最终）

| 环节 | 状态 | 证据 |
|---|---|---|
| 底座 wasm ↔ native 双端像素对拍 | ✅ **实测 SSIM=1.000000** | `check-dual-end.mjs` |
| 该对拍**能失败** | ✅ 变异自证（0.747646） | 篡改一帧 |
| 免登录取 wasm 资源 | ✅ 200 + MIME 正确 | `--local-no-auth` |
| headless canvas 像素读回 | ✅ `PX:12,34,56,255` | 自建探针 |
| headless WebGPU adapter | ✅ `ADAPTER-OK` | 自建探针 |
| wasm 出图（合成源） | ✅ pngLength 2654/9782 | `--probe` |
| **A 端（engine.js 宿主）出图** | ⬜ **未直接验**（`effects-demo-wasm.html` 是现成参照） | — |
| C 端（exe 出片） | ❌ **写盘被拦** | `Permission denied` |

**⇒ 三端测试可行**，底盘（底座双端）已实测通过；补 A 端出口即可闭环。
**C 端（exe）在当前环境不可用**，替代方案是 `dhampir_sample_project_render_png`（wasm 内出图，不写盘）。
