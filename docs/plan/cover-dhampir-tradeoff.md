# 封面要不要接 Dhampir：取舍探索（2026-09-30）

> **结论：现在不接。** 封面是"**一张静态图 + HTML/CSS 排版**"，Dhampir 的强项（逐帧吞吐）在这里恰好用不上；
> 而它今天缺的能力**正好是封面全都在用的那些**（字距 / 行高 / 文字阴影 / 描边宽度与颜色 / 混合模式 /
> backdrop-filter / emoji / 暗角 / 扫描线 / σ>8 的模糊）。接它 = 在另一个仓**从零做一套封面渲染器**，
> 同时把本模块唯一的存在理由（"设计器里看到的就是出片"）变成两份实现。
>
> 想买"前端↔后端完整性"，买在下面 §5 那三件上，**比接 Dhampir 便宜一个量级，而且今天就缺**。

---

## 1. 先把"接入"拆成三件事（不拆会各说各的）

| 代号 | 含义 | 今天是什么 |
|---|---|---|
| **A** | 封面**合成**换渲染器：HTML→Chrome 截图 → Dhampir 单帧 PNG | `cover.rs::cmd_cover` + `build_cover_page(export=true)` + `Page::CaptureScreenshot` |
| **B** | 封面**关键帧**（`_cover_frame.jpg`）换解码器：ffmpeg 抽帧 → Dhampir | `polish.rs:969-977`，`ffmpeg -ss T -i <video> -vframes 1 … -vcodec mjpeg -` |
| **C** | **只为验证前后端完整性**而把封面接到 Dhampir | 现在封面在 `webui/src` 里**零处** dhampir（只有 `PolishPreview.vue` 有） |

⚠️ 用户口径里的"只是截帧的话浏览器也够用"把 **B** 和 **A** 混在一起了 —— 这两件事的成本/收益完全不同，
下面分开算。另外 **A 不是"接一条已经存在的线"**：Dhampir 侧今天没有封面这条通道，
`docs/vtrim-integration.md:259` 的原话是「封面（`polish cover`）｜另一条输出通道（静态图），底座不做｜**有意不做**」，
`docs/dhampir/gap-plan.md:77` E2 同口径。

## 2. 现状（都是实测/代码，不是估算）

**链路（A）**：设计器 `CoverDesigner.vue`（纯 DOM：`<video class="stage-video">` + `<img>` + CSS，**无 canvas**）
→ `POST /api/polish/cover-config`（写 `polish.toml`，手写白名单 `polish.rs:660-737`）
→ `POST /api/polish/cover` → **子进程** `vtrim polish cover`（`polish.rs:253`、`api/render.rs:247-268`）
→ `cmd_cover`（`cover.rs:707`）→ 本地 server（`server.rs:208-233`）→ Chrome `--headless=new`
→ `Emulation 1920×1080` → 等 `window.__COVER_READY`（模板写死 1.5s）→ `CaptureScreenshot(Jpeg,q85,clip 1920×1080)`
→ `cover.jpg` / `cover_1.jpg`（不覆盖）。

**成本（真日志）**

| 项 | 值 | 出处 |
|---|---|---|
| 一次封面：`-- Cover --` → `✓ Cover saved` | **3.18s**（Chrome launch 0.08s / navigate 1.28s / 页面+截屏 1.9s） | `.workbuddy/cover-layer/layers.log:2-5` |
| 六特效 rich 工程同上 | **3.0s** | `.workbuddy/cover-layer/rich.log` |
| 出片流程内封面 | **2.7–3.4s** | `docs/plan/polish-render-speed.md:50-64` |
| 文档口径 | 「十几秒」（`docs/job-queue.md:64`）/「1.1s」（`render/AGENTS.md`） | 两个都偏，**该按实测改成 ≈3s** |
| Dhampir 单帧 PNG 出口 | **1250 ms/帧** | `docs/vtrim-integration.md:454` |
| Dhampir 冷启动一个进程 | **1.5–3.2 s** | 同上 `:456` |
| 转译（node） | **0.2–1.1s**（真日志 `POST /dhampir-project → 200 (212~1176ms)`） | `target/debug/.vtrim/cloud/logs/vtrim-20260930.log:1153,2338` |

## 3. A：封面合成换 Dhampir

**收益（真实但都可替代）**
1. 少一个运行期依赖（浏览器）——**远期**才有意义：今天 `docs/deploy.md:129` 本来就要求用户装 ffmpeg + Chrome，
   出片主路（逐帧）永远走 Chrome；只有"Dhampir 成为默认出片通道 **且** 产品决定彻底不要 Chrome"时，
   封面才是最后一个拖着浏览器的东西。
2. 真·GPU 渲染——封面是**一帧静态图**，GPU 与否在观感上不存在差别。
3. 顺带拿到 C（见 §4），但 C 有更便宜的买法。

**代价（每条都有证据）**
1. **打破本模块唯一的不变式**：设计器是 DOM/CSS 自绘、出片是**同一份 HTML/CSS** 的截图，两边同源靠
   `build_cover_page` 一条替换链（`cover.rs:612-617` 有钉子 `check-cover-stage.mjs:215-219`）+ 手工镜像常量
   （`28px / 1+px/150 / 0.2~4`、`mediaStyle`、`fxStyle`…）。换成 Dhampir 之后就是**三份实现**，
   而"设计器 = 出片"正是用户反复报障的那一类（黑边、图层序、高亮词…）。**这条单独就足以否掉 A。**
2. **Dhampir 缺的正好是封面全用的**：字重/行高/文字阴影「无」（`gap-plan.md:39` A8）、
   描边「只有 `outline: bool`」（A6，而封面是 `-webkit-text-stroke: Npx #2a2522`，0~60px 可调）、
   转译器自己每次都打一条 NOTE「字体名与字重/行高/阴影没接」（`tools/polish-to-dhampir.mjs:3356-3360`）、
   混合模式/backdrop-filter 全无（转译器一律输出 `blend:'normal'`）、暗角是 DROP、
   扫描线是封面独有效果（`cover.rs:467`）、emoji 无契约字段、模糊 `radius≤16`（=σ8）而封面底色模糊基准是
   **σ28**（`polish-to-dhampir.mjs:2791-2795` 那条 disabled 分支就是为它写的）。
3. **文字栅格化器不同已被实测记为"抹不掉"**：Chrome/Skia vs FreeType，
   「同一个字体、同一个字号、同一个位置，两种引擎画出来的字形边缘逐像素不同 …… 要抹掉它得把出片那一路
   也换成 canvas」（`gap-plan.md:437-441`）—— 封面是**文字为主**的产物，这条直接命中。
4. **没有速度收益**：Dhampir 单帧 = 冷启动 1.5–3.2s + 转译 0.2–1.1s + PNG 1250ms/帧 ≈ **3–5.5s**，
   而现在一张封面是 **3.0–3.3s**。"逐帧吞吐快"的前提是帧数多，封面只有一帧。
   （口径提醒：Dhampir 那三个数是**合成语料工程**上量的，不是封面，只作量级参考。）
5. **封面会绑上另一个仓的版本节奏**：Dhampir 产物要 `preview/`+`bin/`+`VERSION` 三件齐、
   `project_schema` 版本由**对方**裁决，`dhampir.rs` 明确**不许静默回退**（`:31-34`）——
   于是"机器上没放 Dhampir 包"会变成**生成封面直接失败**，而今天封面只需要 Chrome。
6. **要改的代码不在本仓**：A 的全部难点（文字/混合/模糊/emoji 的 GPU 实现）在 Dhampir 侧，
   本仓只能提交需求 + 转译规则（`tools/polish-to-dhampir.mjs` 要新增一整段 cover 方言）。
7. 顺带记录一条**今天就不一致的现状**：`cmd_render` 末尾**无条件**调 `cover::cmd_cover`
   （`pipeline.rs:370-374`），所以走 `--renderer dhampir` 时**画面来自 Dhampir、封面仍来自 Chrome**，
   这条混合路**没有任何测试**。

## 4. B：关键帧换 Dhampir

**没有收益，而且搞错了对象。** 今天 `_cover_frame.jpg` 是 ffmpeg 抽的一张（`polish.rs:969-977`），
它只花几百毫秒，**跟"浏览器够不够用"无关**——浏览器只在最后一步把合成好的 HTML 截成 JPG。
Dhampir 侧也没有"把某一时刻解成一张 JPEG"的出口（`dhampir render --from --to` 是帧序列/h264 那条）。

## 5. C：想验证"前端↔后端完整性"，今天就缺的三件（建议按序做）

今天**没有一条判据走完"设计器 → HTTP → 真出片"**：
前端 harness 用的是 **stub**（`stubs.js` 把 payload 塞进 `window.__saved`），
Rust 侧用的是**手写 JSON**（`polish.rs:2032` 那条单测），**两边从没对上过**。

| # | 要补什么 | 为什么它才是"完整性" | 代价 |
|---|---|---|---|
| 1 | **封面失败要能被看见**：`pipeline.rs:371-373` 现在只 `tracing::info!("⚠ Cover generation skipped: …")`（经 `render.rs:168` 落一条 warn 日志），**任务仍然成功**、产物卡上没有封面 —— 实测样本 `render-002f-231241.log:222-223`「Frame extraction failed / ⏱ 封面 0.3s」→ 成片有、封面没有 | 这是今天封面链路里最该修的一条：用户的失败形态是"没有封面，但没人告诉他" | 一处：把封面的失败并进任务的终态/产物标记（或至少升成显式 warn + 前端产物卡标"封面缺失"） |
| 2 | **白名单键 ↔ 前端 payload 键的机器对拍**：`polish.rs:660-737` 是**手写 if-let 链**（27 个键），前端 `coverConfigPayload()`（`CoverDesigner.vue:1331`）是**手写对象**（25 个键） | 今天两边一致（逐个核过：payload 的 25 个键全部有人接；服务端多出的 `frame_pos`/`image` 前端不发，属别的入口在用），但**没有判据**；新增字段漏接的失败形态是"改了没保存"，正是本仓最恨的静默错 | 一个 `webui/scripts/check-*.mjs` 风格的脚本（读两边源码比键集合），零依赖 |
| 3 | **一条真 E2E**：设计器 payload → 真 `POST /api/polish/cover-config` → 真 `polish cover` → 与基线比 SHA（或与 DOM 预览比） | 这才是"前端到后端到出片"的端到端；`.workbuddy/cover-layer/` 那套 SHA 零回归（rich `F608702D96C0D7BC`）**是手工跑的、没有脚本入口** | 一条 `#[ignore]` 的 Rust 集成测试（照 `dhampir_backend.rs` 的样子）或 `.workbuddy` 脚本 + 固定 fixture 工程 |

另外两条**顺带**（与本议题无关但同族）：`check-dhampir-wiring.mjs` 的 `engine.<方法>` 只查 7 个白名单
（`loadSubtitles` / `sourcesFor` / `setSourceBitmap` 这些真在用的都不查）；`scripts/vtrim-compare.mjs`
（Dhampir 对拍）**没进任何守卫清单、没人定期跑**（`docs/dhampir/README.md:56-57` 自己写着）。

## 6. 什么条件下重新开这个决策

三个都要满足才谈 A：

1. Dhampir 成为**默认出片通道**，且产品决定**不再要求用户装 Chrome**（那时封面才有"必须搬家"的理由）；
2. Dhampir 侧把封面要的补齐：字距 / 行高 / 文字阴影 / 描边宽度与颜色 / 混合模式 / backdrop-filter /
   emoji / 暗角 / 扫描线 / σ>8 模糊 —— 每一条都是**对方仓**的工作量；
3. 有一套"**设计器预览也走同一个渲染器**"的方案（否则 §3.1 的 parity 直接崩，等于用一个新问题换一个旧问题）。

在此之前，无论"觉得慢"还是"想验证完整性"，答案都是 **不接**，改做 §5 的三件。

## 7. 复核命令（照抄即可复现本文数字）

```bash
# 封面单张的真实耗时（debug 构建；看 "-- Cover --" 到 "✓ Cover saved" 的差）
cargo run -- polish cover "<片段目录>"          # 或看 <data_dir>/logs/render-<job_id>.log
# 出片流程内封面耗时
grep -n "⏱ 封面" <data_dir>/logs/render-*.log
# 封面零回归基线（手工，rich/横/竖三个样本）
node .workbuddy/cover-layer/make-variants.js && <逐个出片比 SHA-256>
# Dhampir 那一侧的数字出处
grep -n "PNG 出口\|冷启动\|逐帧调 CLI" docs/vtrim-integration.md
grep -n "^| A[0-9]" docs/dhampir/gap-plan.md        # 文字能力缺口表
```

> 引文状态提醒：`docs/dhampir/gap-plan.md` 的差异表**比 `docs/vtrim-integration.md` 旧** ——
> 例如"高亮词"在 gap-plan 里写"无"（A7），而 integration 记"已做（两端）"（`:254`）。
> 引用时以较新的实测为准；本文只引用了两边不冲突的条目（A6/A8/栅格化器）。

---

## 附录 A：逐项能力对账（"Dhampir 能覆盖当前封面设计器的所有效果吗？"）

**判据口径**：
- "封面要什么"= **代码生成的 CSS**（`cover.rs::cover_fx_html` / `cover_gradient` / `cover_bg_vars` / `cover_frame_vars` + `templates/cover.html`），不按 UI 文案。
- "Dhampir 有什么"= **本仓唯一的通路 = 转译器** `tools/polish-to-dhampir.mjs`（它没有映射 = 走不通），
  再加底座侧已有原语的证据（转译器注释会点名）。
- 三档判定：**✅ 有通路** / **⚠️ 部分或仅底座有原语、本仓没接** / **❌ 底座无此原语（要在对方仓做）**。

### A1 六种特效（`[[cover.effects]]`）

| # | 封面生成的 CSS | Dhampir 现状 | 判定 |
|---|---|---|---|
| 1 | **光晕** `radial-gradient(circle at x% y%, rgba(255,220,180,a) 0%, transparent 60%)` | 渐变有 linear/radial（`shape:1/2`），stop 与"中心到 60% 淡出"能否任意表达未证 | ⚠️ 待确认 |
| 2 | **模糊** `backdrop-filter:blur(1+14a px)` + `mask-image:radial-gradient(...)`（**取下层、局部**） | 只有对**源纹理整体**的 `gaussian_blur`；无 backdrop / mask 语义 | ❌ |
| 3 | **暗角点** `radial-gradient(circle at x% y%, transparent 40%, rgba(0,0,0,a) 100%)`，x/y 可调 | 转译器注释原话「**底座有 `vignette` 特效，但不替参照补画**」——成片参照不画所以丢弃 | ⚠️ 底座有原语、本仓没接；参数域（中心/起点）要对齐 |
| 4 | **颜色覆盖** `background:rgba(r,g,b,a)` + **`mix-blend-mode:overlay`** + `isolation:isolate` | overlay 只有**渐变**，且**一律 `blend:'normal'`**（`polish-to-dhampir.mjs:1137,2626,2779,…` 共 11 处） | ❌ |
| 5 | **扫描线** `repeating-linear-gradient(0deg, rgba(255,255,255,a·0.45) 0 1px, transparent 1px 4px)` | 无重复线性渐变原语（转译器里不存在）| ❌（理论上可用预先平铺的贴图层绕，但底座无 tile 语义） |
| 6 | **表情** 字符 + 彩色字体栈 + `drop-shadow` + 字号 48–288px | 工程契约**没有 emoji 字段**；native 文字走 ffmpeg/FreeType，彩色 emoji（COLRv1/CBDT）支持差 | ❌ |

### A2 滤镜层（一层两块）与图层栈

| # | 封面生成的 CSS | Dhampir 现状 | 判定 |
|---|---|---|---|
| 7 | **渐变遮罩** linear（4 方向）/ radial，色取 `[style.subtitle].highlight_color` | linear/radial 都有；角度参数底座已有（`polish-to-dhampir.mjs:1828` 注释"底座已有参数，V-Trim 配置里没写，取 0"） | ✅（4 方向 → 角度） |
| 8 | **暗角**（滤镜层那块）同 #3 | 同 #3 | ⚠️ |
| 9 | **图层栈**：任意 z 序 + `visible:false → display:none` | track/layer 顺序 + `start/end` 可见窗口（用单帧区间表达） | ✅（语义要映射） |

### A3 文字（硬骨头，封面是文字为主的产物）

| # | 封面生成的 CSS | Dhampir 现状 | 判定 |
|---|---|---|---|
| 10 | **描边** `-webkit-text-stroke:Npx #2a2522` + `paint-order:stroke fill`（N=0~60，**居中**描边） | 只有 `outline: bool`（`gap-plan.md` A6）；且 CSS 居中 vs ffmpeg 全在外侧另有"折半"口径 | ❌ |
| 11 | **文字阴影** `0 5px 22px rgba(0,0,0,.78)` | A8「字重/行高/文字阴影 **无**」 | ❌ |
| 12 | **字距** `letter-spacing:Npx`（−10~60，可为负） | 同上（`drawtext` 无 letter-spacing） | ❌ |
| 13 | **行高** `line-height` 0.8~2.6（倍数） | A8 无；`drawtext` 的 `line_spacing` 是**绝对像素**，不是倍数 | ❌（且口径不同） |
| 14 | **对齐** `text-align` left/center/right | 字幕轨的对齐能力未逐行确认 | ⚠️ 待确认 |
| 15 | **逐词高亮** `.hl{color:#hlc}`（正文里挑词上色） | 字幕/弹幕那支**已做两端**（`vtrim-integration.md:254`），但**封面标题不是字幕轨** | ⚠️ 能力在，映射要新做 |
| 16 | **自定义字体**：库内字体（`/vt-fonts` + `@font-face`）；**空 = 系统字体栈** | 底座**故意不猜系统字体**，必须显式 `--font-file` | ❌（默认行为冲突：封面默认系统栈） |
| 17 | **手动分行**：你敲的每一行就是一行、`white-space:nowrap` + `<br>` | 底座 subtitle 会**主动折行 + 缩字**（`shrink_min_scale`/`keep_all_lines`） | ❌（语义冲突：要关掉自动折行） |
| 18 | 入场动画 `fadeUp 0.6s` | 截图时已结束（`__COVER_READY` 1.5s） | ✅ 不需要覆盖 |

### A4 帧层 / 背景层 / 输出

| # | 封面生成的 CSS | Dhampir 现状 | 判定 |
|---|---|---|---|
| 19 | **帧层** `object-fit:contain` + `transform`（`bg_x/bg_y/bg_zoom` 0.5~3） | transform/scale 有；`contain` 摆位**未做**（`vtrim-integration.md:256` 只做了 cover 那一半） | ⚠️ |
| 20 | **背景纯色** `bg_mode=color` | 纯色/黑底有 | ✅ |
| 21 | **背景图片** `_cover_bg.jpg` + `object-fit:cover` | 位图/图片 asset 有 | ✅ |
| 22 | **背景模糊** `filter:blur(σ)` + `scale(1+px/150)`，**σ = 28 × blur_scale（0.2~4，最大 σ112）** | `gaussian_blur` **radius ≤ 16（= σ8）**，且 `blur_radius()` 取最大值**不叠加**（`polish-to-dhampir.mjs:2765,2794` 的注释就是它） | ❌（差 3.5×~14×） |
| 23 | 恒 1920×1080 → JPEG q85 | CLI `frame --out`（PNG，实测 **1250ms/帧**） | ✅（格式 PNG，需转码或接受） |

> **不进成片的东西不需要覆盖**：安全区线框、选中框、拖动层、`__COVER_READY` 之前的交互态 —— 它们只活在设计器里。

### A5 对账结论

- **✅ 有通路 6 项**：渐变遮罩 · 图层栈 · 纯色底 · 图片底 · 画布输出（PNG，需转码或接受）· 入场动画（本来就不需要覆盖）。
- **⚠️ 5 项**：光晕 · 暗角（**底座有原语、本仓转译器没接，本仓可做**）· 帧层 `contain` · 文字对齐 · 逐词高亮（能力在字幕那支）。
- **❌ 11 项，而且都不是"缺参数"而是"缺原语"**：局部模糊（backdrop+mask）· `mix-blend-mode` · 扫描线 · emoji · 描边宽度与颜色 · 文字阴影 · 字距 · 行高 · **系统字体栈** · **不折行语义** · σ>8 模糊。
- 也就是说：**Dhampir 能画"朴素封面"（背景 + 一行纯色字），画不了"当前设计器的封面"** ——
  当前设计器的卖点（描边/阴影/字距/六种特效/逐词高亮/emoji）**几乎全落在 ❌ 那一列**。
- 工作量的归属很清楚：⚠️ 那 5 项**本仓可做**（改转译器/映射）；❌ 那 11 项**必须在 Dhampir 仓做**（新原语 + 两端实现），
  而 native 侧的文字还走 ffmpeg/FreeType —— 与浏览器端不是一个引擎（`gap-plan.md:437-441`）。
- **最小裁决实验**（半天，先做这个再谈别的）：只画一张 **背景图 + 一行带描边和字距的标题**，两端各出一张（浏览器 wasm / `dhampir frame`），
  比 SSIM + 中位差，**专门看文字边缘**。这一张同时命中 ❌ 里的 #10/#12 和 native↔wasm 的文字引擎差异 —— 过不了就整条路不成立。

---

## 8. 不换渲染器的话，封面本身还有哪些好选择

**前提（这才是封面唯一真正的优势）**：封面两端都是 **HTML/CSS 引擎**（客户端是浏览器的 DOM，服务端是 headless Chrome）。
所以**任何增强只要落在 CSS + 模板变量上，就自动满足"两端一致"** —— 不需要 GPU、不需要第二份实现。
按这个标准排，四条路：

| # | 选择 | 解决什么 | 代价 | 建议 |
|---|---|---|---|---|
| ① | **设计器显示那份真 HTML**（接上 `coverPreview`，`polish.js:104` 现成的死接口） | 把"统一"从**约定**升级成**物理**（预览与出片共用 `build_cover_page`）；顺带设计器预览不再需要服务端 Chrome | 跨源（`127.0.0.1` 只在同机可用）/远程部署（要 `srcdoc` + 同源资源）/缓存 bust；设计器 DOM 退化成**交互层**（选框、拖动、安全区线框） | **做**（结构性收益最大） |
| ② | **像素对拍门禁**：设计器舞台截图 vs 出片 JPEG（SSIM + 中位差，口径照 `vtrim-compare.mjs`） | "像不像"从人眼变成数字；也是①的验收线 | 需要 Chrome + 真工程的环境（`.workbuddy/cover-layer/` 已有手工先例，可脚本化） | **做**（没有它①只是"感觉一致"） |
| ③ | **能力增强走纯 CSS**：每层不透明度 / 混合模式 / 新特效（P2 清单） | 设计器能表达的表现力 | 每加一条要同步四处（`CoverConfig` / 白名单 / `cover.html` / 设计器镜像）—— 但**两端天然一致**；①之后镜像那两处会自然收窄 | **按需做**（想加效果时只走这条路） |
| ④ | **性能与健壮性三小件**：a. 干掉 `cover.html:112` 那 **1.5s 白等**（占整次封面 3.18s 的 47%）· b. 出片末尾的封面失败**要可见**（`pipeline.rs:371-373` 现在只落一条 warn、任务仍成功）· c. Chrome 探测：`[chrome] path` 指 Edge（本机已有 154）+ 把 Edge 加进探测名单 + `doctor` 报一句 | a 立刻快近一倍；b 消除"成片有、封面没有"的静默；c 少装一个浏览器 | 都很小（a 在模板 + `cover.rs`；b 在 `pipeline.rs` + 产物标记；c 在 `commands.rs` / `api/mod.rs` / `docs/deploy.md`） | **做**（性价比最高） |

**明确不建议的四条**（都已论证）：换 Dhampir（见附录 A）· ffmpeg 合成封面（又一份排版引擎 + 观感变化，且不省依赖）·
用户浏览器截帧（丢可复现性 + 丢无人值守出封面）· 为了封面引入任何 GPU/第三方渲染器。
