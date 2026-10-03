# 精修出片的速度：现状实测 + 可选加速路线（技术探索）

> 日期：2026-09-20 · 状态：**探索，不改代码**（结论可直接作为下一轮实施依据）
> 数据来源：本机 `target/debug/.yekitrim/cloud/logs/render-*.log`（2026-09-15 ~ 09-20，**全部是 debug 构建**）
> + 代码走查（`render/pipeline.rs` / `render/chrome.rs` / `templates/index.html` / `render/native/*` / `yekitrim-render-core`）
> 机器：RTX 4070（NVENC）/ 16 逻辑核 / 素材在网络盘（RaiDrive-WebDAV）

**结论先行**

1. 现在墙钟 ≈ `帧数 × ~75ms ÷ 并行度(4)` **+ 固定开销 ~20~100s**（贴纸 GIF 解码、音效数据、拼接/合流/去静音重编码/封面）。
   聚合 **18.6~39.6 fps = 0.62~1.32× 实时**；4.4 分钟的片段要 3.7 分钟出片。
2. **最大的钱花在"搬画面"上，不在"画画面"上**：`75ms/帧` 里
   **发帧+启动 51%**、**页面逐帧合成 33%**、**回读 16%**（9 次出片 / 36,387 帧合计）——
   即 **DevTools 协议 + base64 的两趟搬运占了 67%**，浏览器真正画画只占三分之一。
   （微基准实测 debug 下 base64+serde 只有 9.5ms/帧，所以那 30~40ms 在 CDP/WebSocket + V8 解析 13MB 表达式 + GC。）
3. 编码（NVENC）只占 3~6ms/帧、抽帧 7~13ms/帧：**都不是瓶颈**。
3b. **竖屏更慢**：唯一的竖屏工程实测页面循环 **45.8 ms/帧**（横屏 21~27），多出来的 ~22ms
   最可能是每帧一次整幅 `blur(28px)` 的衬底（§1.3）——而衬底模糊降到 1/4 分辨率几乎是无风险的改动。
4. 结构性浪费：每帧被编解码 3 次（ffmpeg→JPEG(q2) → 页面解 → canvas → `toBlob` JPEG(0.6) → NVENC），
   其中 `toBlob` 那次还直接决定了成片画质上限。
5. 路线：**A0 把帧传输换成本地 HTTP 二进制（2~4 天，渲染段 2.5×）→ A1 并行度 8（零代码）→
   静态开销单趟化 → B WebCodecs/流水线 → C 原生 wgpu（质变 5~10×，但要付"第三份实现"的对拍成本）**。

---

## 一、实测基线

同一批日志里 9 次成功出片（`渲染Σ/抽帧Σ/编码Σ` 是 4 个分块**各自**的耗时之和，分块之间并行）：

| 日志 | 片段 | 帧数 | 并行 | 总墙钟 | 渲染Σ | 抽帧Σ | 编码Σ | ms/帧(渲染段) | 聚合 fps | 相对实时 |
|---|---|---|---|---|---|---|---|---|---|---|
| render-0000-154645 | 262.4s | 7874 | 4 | 222.2s | 622.8 | 104.5 | 29.0 | 79.1 | 35.4 | 1.18× |
| render-0000-154330 | 146.1s | 4382 | 4 | 110.7s | 306.1 | 42.7 | 18.5 | 69.9 | 39.6 | 1.32× |
| render-000b-164341 | 139.1s | 4174 | 4 | 121.7s | 289.9 | 42.7 | 15.0 | 69.5 | 34.3 | 1.14× |
| render-0021-085346 | 167.6s | 5027 | 4 | 208.1s | 366.5 | 48.5 | 29.5 | 72.9 | 24.2 | 0.81× |
| render-0015-075403 | 106.5s | 3197 | 4 | 128.1s | 282.1 | 38.9 | 15.7 | 88.2 | 25.0 | 0.83× |
| render-001d-002525 | 105.1s | 3153 | 3 | 131.8s | 175.6 | 22.0 | 14.4 | 74.2 | 23.9 | 0.80× |
| render-0000-005819 | 105.1s | 3153 | 4 | 169.2s | 237.5 | 28.2 | 19.8 | 75.3 | 18.6 | 0.62× |
| render-0031-170833 | 98.9s | 2967 | 4 | 99.0s | 219.1 | 29.9 | 12.5 | 73.8 | 30.0 | 1.00× |
| **render-0002-155606**（**竖屏**） | 82.0s | 2460 | 4 | 95.8s | 198.3 | 3.7 | 9.0 | 80.6 | 25.7 | 0.86× |

最后一行是**用户自己的竖屏工程**（日志里 `画布 1080x1920 · 源 Some((720, 1280)) → axis="fill" position="center" bg="blur"`）——
它的「抽帧 3.7s」不是异常：源是 720×1280（0.92 Mpx，只有 1080p 的 44%），MJPEG 抽帧当然快。
但它的**页面循环是 45.8 ms/帧**，是横屏那批（21~27）的近两倍 —— 见 §1.3。

读法：**聚合 18.6~39.6 fps，即 0.62~1.32× 实时**。4.4 分钟的片段要 3.7 分钟出片；1.7 分钟的要 2~3 分钟。

### 1.1 一次 105s 出片的完整阶段（`render-0000-005819.log`，这是最能说明问题的样本）

```
16:58:19.77  开始
16:58:20.01  ⏱ process 0.2s
16:58:20.31  ⏱ 起服务 0.1s
16:58:26.37  音效数据 6.1s                    ← 页面协议导出 sfx 时间表
16:59:18.91  贴纸 GIF 帧 52.5s                ← 4~5 个 GIF 解成 base64 PNG（debug）
16:59:18.95  Encoder: h264_nvenc
17:00:31.16  ⏱ 分块渲染 72.2s                 ← 4 块并行，每块 抽帧7 + 渲染59 + 编码5
17:00:38.98  ⏱ 拼接 7.8s                      ← 网络盘 IO
17:00:43.30  ⏱ 音效混音 4.3s
17:00:49.26  ⏱ 合流 6.0s
17:01:05.55  ✂ 去屏蔽区间：整段重编码 16.3s     ← 又一趟全片编码
17:01:08.97  ⏱ 封面 3.4s
总墙钟 169.2s = 渲染 72.2 + 固定开销 97.0
```

**固定开销（与帧数几乎无关）在这条链路上占 57%**；而 262s 的片段里它只占 ~15%。
也就是说：**改「每帧多快」对长片段有效，改「固定开销」对短片段更有效**（用户日常大多在切 1~3 分钟）。

### 1.2 `75ms/帧` 的成分拆解：**搬运比渲染贵得多**（关键结论）

`chrome.rs::capture_and_read` 每 1.5s 轮询页面的 `__CAPTURE_PROGRESS`（页面每 30 帧回写一次），
日志里就是 `frames 0..1969: 50% 12s` 这种行。于是「渲染段」可以被**日志时间戳**切成三段：

```
本块渲染段开始 ──▶ 第一次进度回写 ──▶ 最后一次进度回写 ──▶ 渲染段结束
   (启动+就绪+发帧)      (页面逐帧循环)        (回读 + 解码)
```

对 9 次出片的每个分块都算一遍（校验：三段之和 = 日志里的「渲染」耗时，误差 ≤0.1s），得到：

| 日志（片段/帧数/并行） | 渲染段 ms/帧 | ① 发帧+启动 | ② 页面循环 | ③ 回读 |
|---|---|---|---|---|
| 262.4s / 7874 / 4 | 79.1 | **36.3~44.9** | 25.3~26.8 | 9.3~24.2 |
| 139.1s / 4174 / 4 | 69.5 | **34.1~35.2** | 23.2~23.3 | 10.6~13.0 |
| 98.9s / 2967 / 4 | 73.8 | **38.8~40.9** | 21.0~23.3 | 10.8~13.1 |
| 106.5s / 3197 / 4 | 88.2 | **51.1~51.6** | 24.3~26.2 | 11.4~13.5 |
| 105.1s / 3153 / 4 | 75.3 | **41.3~41.5** | 21.9 | 11.5~12.9 |

（单位 ms/帧。全部 9 份日志 × 每个分块的明细：`python scripts/render_phase_report.py`，见下。）

**怎么算的**（可复现，不需要改代码）：本块渲染段起点 = 日志里 `Encoder:` 行的时刻 + 本块**抽帧**耗时，
渲染段终点 = `chunk#N: done` 行的时刻 − 本块**编码**耗时；
中间用 `frames A..B: N% Ns` 进度行的**首末两条**（精确到 10ms）切出三段。
`scripts/render_phase_report.py` 就是这段推导，跑一遍能重出上面两张表（含校验列）。

**读法：**

- ② 页面循环 = **21~27 ms/帧**，这一段才是真正的"浏览器在画画"（JPEG 解码 + canvas 合成 + `toBlob` + base64）；
- ① 发帧+启动 = **33~52 ms/帧**（`ChromeBackend::send_frames`：每帧 base64 → 每 50 帧拼一个 ~13MB 的
  `window.__LOAD_FRAMES(<json>)` **JS 表达式**交给 `Runtime.evaluate`）；
- ③ 回读 = **9~24 ms/帧**（`window.__GET_FRAMES` 把 base64 字符串 `JSON.stringify` 出来 → CDP → Rust 解析 → base64 解码）。

即 **「搬运」占了渲染段的大头**。把 9 次出片、36,387 帧全部合计（`scripts/render_phase_report.py` 末行）：

```
① 发帧+启动 51%  ② 页面循环 33%  ③ 回读 16%      → 搬运 = 67%
```

（竖屏那次偏心：22.9 + 45.8 + 10.7 里页面占 58%，因为衬底模糊把 ② 抬到了 45.8ms —— 见 §1.3。）

这也解释了为什么单纯优化 Rust 侧没用：微基准（`cargo test -p yekitrim-polish --test render_bench
-- --ignored plumbing`）实测 **debug 下 base64+serde 只有 9.5 ms/帧**（300 帧 × 200KB：发送 4.1 + 接收 5.4），
剩下那 30~40ms 在 **CDP/WebSocket 传输 + V8 解析 13MB 表达式 + 每块几百 MB 的 JS 字符串分配/GC** 上。
外部调研独立得到同一个结论（`docs/plan/export-pipeline-web-survey.md`：26ms 是画布、其余是 round trip）。

> 也就是说：**当前出片最大的浪费，是把画面用 base64 JPEG 通过 DevTools 协议"搬"了两趟。**
> 任何"换渲染引擎"的方案（包括原生 wgpu）都会顺带干掉这一项 —— 这是它们收益的大头，
> 而不是"canvas 画得慢"。

### 1.3 竖屏 + 模糊衬底：页面循环 **45.8 vs 23.5 ms/帧**

唯一一份竖屏工程的日志（`render-0002-155606`，画布 1080×1920、`bg="blur"`、源 720×1280）

| | 横屏那批（`bg=none`，1080p 源） | 竖屏（`bg=blur`，720×1280 源） |
|---|---|---|
| 抽帧 | 7~28 s/块 | **3.7 s/块**（源小） |
| 发帧+启动 | 37~45 ms/帧（JPEG 大） | **22.9 ms/帧**（JPEG 小 → 印证「发帧 ∝ 字节数」） |
| **页面循环** | 21~27 ms/帧 | **45.8 ms/帧** |
| 回读 | 10~13 ms/帧 | 10.7 ms/帧 |

竖屏比横屏多出来的 ~22ms/帧，最可能来自 `drawStageBackdrop`：**每帧一次整幅
`drawImage` + `ctx.filter='blur(28px) brightness(.5) saturate(1.15)'` + 绕中心放大**（1080×1920）。
`toBlob` 的输出尺寸两边一样（都是 2.07 Mpx），所以尺寸解释不了这个差；差别就是"多画了一层全画布模糊"。
（这条要坐实只需在页面里给 backdrop 单独打一次计时，见 §四-4 与 §三 P0-0/1。）

**最便宜的一刀**：把衬底模糊放到 **1/4 分辨率的离屏 canvas**（270×480）上做，再放大铺满 ——
`sigma` 已是 28px 的量级，重模糊下肉眼无差，工作量降一个数量级。这直接影响用户现在正在做的竖屏。

### 1.4 每帧被编解码三次（结构性浪费）

```
源 mp4 ──ffmpeg -vcodec mjpeg -q:v 2──▶ JPEG ①（有损）
        ──base64/CDP──▶ 页面 createImageBitmap（解码）
        ──canvas 2D 合成（drawImage / filter blur / 文字描边 / GIF 贴纸）──
        ──canvas.toBlob('image/jpeg', 0.6)──▶ JPEG ②（有损、质量 0.6）
        ──base64/CDP──▶ Rust ──▶ ffmpeg h264_nvenc ──▶ H.264 ③
```
一帧跑 **1 次解码 + 3 次编码 + 2 次 base64 + 2 次 JSON 序列化**。
JPEG ②（q0.6）还直接决定了成片的画质上限——现在**没有**从「源画面 → 成片」的干净通路。

### 1.5 其他固定开销的事实（会咬人）

- **`data/sticker_frames.json` 是 58~95MB 的 base64 PNG 帧表**（实测用户工程：58MB / 71MB / 92MB / 94MB）。
  每次出片都要：读盘（网络盘！）→ `serde_json` 解析 → 重新序列化 → `tab.evaluate` 注入页面 →
  页面再 `createImageBitmap` 全部帧。**这一串与帧数无关，但对每个片段都付一次**。
- 贴纸 GIF 解码实测 **52.5s**（debug，5 个 GIF）；缓存命中时会跳过解码，但 90MB 的读+解析还在。
- 去屏蔽区间（`cut_ranges`）是**整段重编码**（105s 片段 16.3s）；它发生在渲染之后，等于全片多走一趟。
- 拼接/混音/合流各 4~8s，主要是**网络盘 IO**（工程在网络盘上）。

---

## 二、候选方案

| # | 方案 | 预期（相对现在） | 工作量 | 风险 | 新依赖 |
|---|---|---|---|---|---|
| **A0** | **把帧传输从 CDP 换成本地 HTTP 二进制**：抽好的帧落本地临时目录（或由 Rust 侧的 axum 直接按需吐），页面 `fetch('/frame/<i>.jpg')` 取；渲染结果用 `fetch` POST 回本地端点，不再走 `__LOAD_FRAMES` / `__GET_FRAMES` 的 base64+JSON | 渲染段 **75→~30 ms/帧（2.5×）** | 2~4 天 | 低（页面换数据来源、握手协议小改；可 A/B） | 无 |
| **A1** | **并行度 4→8**（`VTEDIT_PARALLELISM`）：16 逻辑核，页面循环单线程，按块并行近乎线性 | 并行段再 **~1.7×** | **0**（先手动试） | 低；需确认 NVENC 并发会话上限 | 无 |
| **A2** | 其余同架构优化：release 基线、流式发帧、竖屏衬底模糊降到 1/4 分辨率、单趟后处理（渲染→编码→混音→去静音）、`sticker_frames.json` 改二进制/按包共享 | 固定开销（占短片段 30~57%）砍半 | 2~4 天 | 低 | 无 |
| **B** | **页面内 WebCodecs**：`VideoDecoder` 吃 H.264 帧流 + `VideoEncoder` 直接从 canvas 出 H.264，回传只有码流（体积降 20~50×）；容器用 MediaBunny 流式封装 | 在 A0 之上再把 ②页面循环 24→~15ms、并**省掉一次有损 JPEG** | 3~7 天 | 中：GOP/参数集拼接、`encodeQueueSize` 背压；**headless 上是否真给硬件编码器必须先实测**（见 §七-4） | MediaBunny（JS，可随包） |
| **C** | **原生 wgpu 合成器**（仓库已有半成品）：ffmpeg 直出 raw RGBA → GPU 合成 → 直接喂 NVENC | **5~10×**（A0 之后剩下的 ② 也没了；单进程目标 3~8ms/帧） | 2~4 周 | 中高：补 stage/竖屏/字幕折行/贴纸锚点 + **第三份实现**必须自动化对拍 | 已有（wgpu/swash） |
| **D** | **页面内流水线化**（decode/render/encode 三阶段并发，多 OffscreenCanvas/Worker） | 1.3~1.8×（在 A0 之后仍有效） | 2~4 天 | 低中 | 无 |
| **E** | **静态部分下沉到 ffmpeg**（底板/主画面 fit/blur 用 GPU 滤镜：本机 ffmpeg 有 `gblur_vulkan`/`avgblur_vulkan`/`boxblur_opencl`/`scale_cuda`/`overlay_cuda`，已实测；字幕用 libass） | 静态层几乎免费 | 1~2 周 | 中：动态特效/贴纸动画用滤镜图表达很痛苦；**libass 的逐段高亮 + 动画是难点** | libass（本机 ffmpeg 需实测 `-filters`） |
| **F** | 多机/队列 | 线性 | 大 | 运维复杂 | — |

**排序即优先级**：A0/A1/A2 是「不换引擎就能拿到 2.5~4×」的一组，B/D 是保 Chrome 的第二波，C 是质变但要付对拍成本。

方案 C 的现有基础（决定了它的工作量比想象中小）：
- `yekitrim-render-core/src/timeline.rs`（53KB）**已经把 JS 的时间轴数学整段搬过来了**（相机/震动/手持/弹幕/字幕/贴纸/overlay）；
- `composite.rs` 用 **swash** 做 CJK 文字整形 + 描边光栅化（`render_text_rgba`），`sprite.rs` 有纹理缓存（同一段文字只光栅化一次）；
- `effects.rs` / `shaders.rs` 是 blur / color_shift 等效果 shader；`render.rs` 是 source pass；
- `native/mod.rs` 已能整块渲染并给 `eval_ms` / `read_ms` 分相计时（`wgpu chunk progress: ... eval=Xms read=Yms`）；
- WebUI 的「真渲染」按钮走的就是服务端 `/api/polish/render-frame` → `WgpuBackend`（`PolishPreview.vue`），
  另有把同一套 `yekitrim-render-core` 编到 wasm 的浏览器预览路径 —— **这条路不是从零开始，也不是没人用**。

它现在的两处明显低效（也正好是最好下手的点）：
1. `extract_chunk_frames(need_rgba=true)` 用 `image` crate 在 **CPU** 上解 JPEG → 再上传 GPU（应该直接 `-f rawvideo` 让 ffmpeg 出 RGBA，省掉 JPEG 编解码）；
2. `encode_all_parallel` 把 GPU 读回的 RGBA **又编成 JPEG**（应该 `-f rawvideo` 直接喂 ffmpeg/NVENC，省掉第 3 次编码与一代画质损失）。

---

## 三、建议的路线（每步都带判定指标与「杀死条件」）

**P0 · 先量（半天，零结构改动）**
0. **先确认 headless Chrome 是不是落在软件光栅上**（半天，可能白拿一笔）：
   项目用的是 `headless_chrome`（不是 Puppeteer，所以没有"Puppeteer 偷偷塞 `--use-angle=swiftshader-webgl`"
   这个具体坑），但同类问题一样存在 —— **用 `chrome://gpu` 或显式加 `--use-angle=d3d11 --ignore-gpu-blocklist`
   跑一次，看 ② 页面循环动不动**。判定：① 若现在就是软件 GL/光栅，换到 D3D11 后 ② 明显下降 → 白赚；
   ② 若本来就是硬件，则参考下列已知坑（Chromium 有 "GPU canvas 上的 blur 滤镜非常慢" 的历史 bug）。
1. 用 **release** 重跑同一批片段，记录「分块渲染 / 固定开销」两段。判定：§1.2 里 ① 与 ③ 的绝对值掉多少
   —— 微基准显示 base64+serde 只占 9.5ms/帧（debug），所以**若 release 后 ① 仍 > 25ms/帧，
   瓶颈就是 CDP/V8 本身，A0 必须是第一刀**；若掉到 10ms 以下，则 ② 页面循环成为主要目标（B/D）。
2. **并行度**：`VTEDIT_PARALLELISM=6/8` 跑同一片段（日志 `-- Render chunks (N) --` 可确认生效）。
   判定：聚合 fps 是否近似线性上升；**RTX 4070 只有 1 个 NVENC 引擎**，SDK 13.0 应用笔记写的
   消费级上限是 **8 路并发会话**（NVIDIA 支持矩阵那份写 12，两个来源不一致）——所以 8 块正好卡在边上，
   要看有没有会话超限报错；编码本身余量巨大（Ada 1080p H.264 P5 CBR 单引擎 **291 fps**）。
3. 新基准跑一版留档：`cargo test -p yekitrim-polish --test render_bench -- --ignored --nocapture`
   （只跑渲染、不写 mp4，输出「抽帧 / 渲染 / ms每帧 / 有效 fps」）。
   `VTEDIT_RENDERER=wgpu` + `--features wgpu-render` 再跑一版**只看耗时**（画面暂不对齐），
   拿 `eval=` / `read=` 分相数字 —— 这决定 C 的天花板。
4. 判定后选路：**A0+A1**（保 Chrome，2.5~4×）或直接上 **C**。

**P1 · 打掉搬运（2~4 天，目标 2.5×，不换引擎）**
- 抽好的帧落**本地临时目录**（别落工程所在的网络盘），页面 `fetch('/frames/<i>.jpg')` 取；
- 渲染结果 `fetch('/put/<i>', {method:'POST', body:blob})` 回本地端点（二进制，不 base64、不走 CDP JSON）；
- 发帧/回读各变成一次本地 HTTP + 二进制体：按 §1.2 的基线，①③ 合计应从 ~50ms/帧 降到几 ms/帧；
- 顺带把 `send_frames` 与渲染**流水起来**（发一批渲一批），页面内存从 ~400MB/chunk 降下来；
- A/B 判定：同一个工程、同一段帧，`render_bench` 的 ms/帧 + 真实出片的 `抽帧/渲染/编码` 日志。

**P2 · 保 Chrome 的第二波（3~7 天）**
- 页面内 `VideoDecoder`/`VideoEncoder`（WebCodecs）：抽帧改成一次 H.264 码流喂页面、页面直接出码流，
  省掉一次有损 JPEG、也省掉画布→JPEG 编码；
- decode/render/encode 三阶段流水线（多个 OffscreenCanvas）；
- 竖屏 `bg=blur` 的衬底模糊降到 1/4 分辨率（**用户正在做竖屏，这条最直接**：每帧一笔全画布模糊）；
- 后处理单趟化：渲染→编码→混音→去静音一次成型，去掉 `cut_ranges` 的第二趟全片编码。

**P3 · 原生合成器（2~4 周，目标 5~10×）**
- 先补「覆盖大多数片段」的子集：`[stage]` 衬底、竖屏画布与 fit、字幕折行（`wrapCaptionParts` 的 Rust 版）、
  `manual` 贴纸锚点；
- 输入改 raw RGBA（别在 CPU 解 JPEG）、输出改 rawvideo→NVENC（别把 GPU 读回的 RGBA 又编成 JPEG）；
- **必配护栏**：逐帧 golden diff（同工程同时刻，Chrome 与 native 各出一帧逐像素比）——
  `crates/yekitrim-polish/tests/portrait_backdrop.rs` 已经是这个模式（同帧 A/B + 断言），扩成全画面即可。
  没有它，native 路径一定会重演「预览 ≠ 成片」。

**P4 · 可选**：原生渲染常驻 worker 进程池（复用 wgpu device，省掉每块一次的初始化）。

---

## 四、立即可做的低风险清单（不动架构）

1. **`VTEDIT_PARALLELISM=8`**（零代码；日志 `-- Render chunks (N) --` 确认生效，注意 NVENC 并发会话上限）；
2. 出片用 **release/打包版**（debug 的字节处理成倍放大，日志自己都提示了）；
3. **把帧传输换成本地 HTTP 二进制**（A0）——按 §1.2 的实测，这是当前最大的一笔；
4. 给 `render_chunk` 补细分计时（launch / send / page / readback），把 §1.2 的三段直接打进日志；
5. **竖屏 `bg=blur` 的衬底模糊降到 1/4 分辨率**（§1.3 实测竖屏页面循环 45.8 vs 横屏 23.5 ms/帧，最可能就花在这）；
   顺手给页面里的 backdrop / toBlob / createImageBitmap 各打一次计时，把 ② 再拆细；
6. `sticker_frames.json` 改成**按包/按 GIF 内容**共享（现在按工程目录各存一份 58~94MB），或改二进制；
7. 「去屏蔽区间」若真的要用，放到渲染**前**做时间轴映射，别在成片后再走一趟全片重编码；
8. 帧的落盘/传输一律走**本地盘**，别落在工程所在的网络盘（拼接/合流那 4~8s 多半是 WebDAV 的 IO）。

---

## 五、不建议 / 已排除

- **ffmpeg 滤镜图全量重写**（把整套特效/贴纸动画/字幕搬进 `filter_complex`）：表达能力与可维护性都差，
  只适合静态层（方案 E）；这也是为什么现有实现选了浏览器/GPU 合成。
- **`Page.captureScreenshot` 逐帧截图**：每帧一次全屏 readback + PNG，比现在的 canvas 路径更慢。
- **`MediaRecorder` + `captureStream`**：录制节奏受实时性影响、质量控制差、随机访问困难，
  要编码就该用 WebCodecs 而不是 MediaRecorder。

---

## 六、待量化（开放问题）

1. release 构建下 ①（发帧）与 ③（回读）的真实绝对值 —— 决定 A0 是不是第一刀（P0-1/2）。
2. A0 落地后，② 页面循环还是不是新的瓶颈（21~27ms/帧 里，`ctx.filter` 模糊与 `toBlob` 各占多少）。
3. headless Chrome 在 Windows 上 `VideoEncoder` 是否真的走硬件（NVENC/QSV）——只影响 B 的上限。
4. RTX 4070 + 当前驱动（610.74）的**并发 NVENC 会话上限**（决定并行度能不能到 8）。
5. `ctx.filter='blur(Npx)'` 在 headless 里是 CPU 还是 GPU 光栅（决定第 5 条清单的收益；也对竖屏观感有影响）。
6. native 路径补齐后单帧上限受限于 GPU 合成、readback 还是 NVENC。

> 1/2/4 都能用一次实验回答，不需要写生产代码；建议按 P0 的顺序做完再决定 P1/P2 的投入比例。

---

## 七、外部调研要点（含来源）

另一份**纯外部调研**在 `docs/plan/export-pipeline-web-survey.md`（15 条带 URL 的事实 + 未证实清单）。
与本文档的实测结论对齐之后，值得记在这里的是：

1. **编码侧完全没有压力**：NVIDIA 官方 Ada 数据，1080p H.264 单引擎 **P5 CBR 291 fps / P1 910 fps**
   （RTX 4090 @2415MHz）——是 30fps 需求的 ~10 倍。[NVENC 应用笔记](https://docs.nvidia.com/video-technologies/video-codec-sdk/13.0/nvenc-application-note/index.html)
   → 任何"为了编码器换架构"的理由都不成立；这条和 §1 实测（编码 3~6ms/帧）互相印证。
2. **并发编码会话**：支持矩阵写 12、SDK 13.0 应用笔记写**消费级 8 路/系统**，两处不一致 → A1 的 8 块要实测。
   [support matrix](https://developer.nvidia.com/video-encode-and-decode-gpu-support-matrix-new)
3. **Split Frame Encoding（把一帧拆到多个 NVENC）不适用**：Ada + SDK 12.0、**仅 HEVC/AV1**、且需要 ≥2 个引擎；
   4070 只有 1 个引擎。[OBS 文档](https://obsproject.com/kb/advanced-nvenc-options)
4. **WebCodecs 在 Windows 上给不给硬件编码器必须先测**（这是 B 的天花板与唯一"生死条件"）：
   有 Chromium issue 报告 `prefer-hardware` + `isConfigSupported` 在 1080p@30 以上被拒（GTX 1080/2070S/3070Ti），
   但那是多年前的报告。[issue 40249559](https://issues.chromium.org/issues/40249559)
   → 实测方法：页面里打印 `isConfigSupported` 结果，同时 `nvidia-smi` 看编码会话数；
   **软件编码 + CPU 打满 = 这条路没走通**。
5. **canvas 模糊在 GPU 路径上曾是"更慢"的那条**：Chromium 有专门 bug「GPU 加速 canvas 上的 blur 滤镜非常慢」，
   这也解释了为什么把衬底模糊降到 1/4 分辨率是稳赚的。[issue 41391683](https://issues.chromium.org/issues/41391683)
6. **本机 ffmpeg 已具备方案 E 要的 GPU 滤镜**（我在这台机器上实测 `ffmpeg -filters`）：
   `gblur_vulkan` / `avgblur_vulkan` / `boxblur_opencl` / `avgblur_opencl` / `scale_cuda` / `overlay_cuda` /
   `hwupload_cuda` / `scale_vulkan` / `overlay_vulkan` / `libplacebo`；
   hwaccel 有 `cuda / qsv / d3d11va / dxva2 / opencl / vulkan / d3d12va / amf / vaapi`；
   NVENC 有 `h264/hevc/av1`。→ 静态层（底板模糊、主画面 fit）下沉到 ffmpeg 是**现成可做**的。
   ⚠️ 但 **libplacebo 没有 blur 原语**（本机这份的 `libplacebo` 还是 `N->V` 的 source filter），
   所以底衬模糊要走 `gblur_vulkan` 而不是 libplacebo。[libplacebo](https://github.com/haasn/libplacebo)
7. **WebCodecs 出来的码流封装**：`mp4-muxer` 已弃用、继任者是 **Mediabunny**（`CanvasSource` + 流式 `StreamTarget`）
   —— 方案 B 的容器层不用自己写。[mediabunny](https://mediabunny.dev/guide/writing-media-files)
   · 已有生产实例走同一条路：headless Chromium 渲染 + WebCodecs 编码 + MediaBunny 封装，
   并把「逐帧截图 → ffmpeg stdin」明确称为 fundamentally inefficient。
   [videoflow](https://videoflow.dev/blog/zero-ffmpeg-video-pipelines-webcodecs)
8. **Rust 侧的文本**：`cosmic-text` 有 CJK/折行/逐段样式但**没有描边**，`glyphon` 是它的 wgpu 包装且没有基准数据；
   本仓库已经用 **swash** 自己做描边（`composite::render_text_rgba`），这条路是对的，别换。
   [cosmic-text](https://github.com/pop-os/cosmic-text) · [glyphon](https://github.com/grovesNL/glyphon)

**没能证实、别当成结论的**：现代 Chrome 在 Windows 上 1080p60 是否能从 WebCodecs 拿到硬件 H.264；
`--enable-accelerated-2d-canvas` 对 `ctx.filter='blur()'` 到底有没有帮助（正反都没有公开基准）；
`ctx.filter` 模糊的每帧毫秒数 / 是否随半径二次增长；NVENC 相对 libx264 的 BD-rate；
分块渲染出来的 `EncodedVideoChunk` 能不能干净拼接（参数集/GOP 连续性）。
