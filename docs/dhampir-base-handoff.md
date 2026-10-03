# Dhampir 底座：待修问题交接单（可直接在 Dhampir 仓独立处理）

> **这份文档是自包含的**：不需要 YekiTrim 仓、不需要它的工程，第 1～2 节的复现都能在 Dhampir 仓里做完。
> 来源：YekiTrim 侧在"精修效果链路统一 + 类型收敛"（2026-10-01）期间整理；YekiTrim 默认出片通道就是本底座。
> 每条都给了 **症状 / 影响 / 最小复现 / 已排除项 / 可疑点（带 file:line）/ 验收判据**。
> 行号按本次整理时读到的代码，若已变动请按函数名搜。

## 结论摘要

| # | 项 | 本次核对后的状态 |
|---|---|---|
| ~~**D1**~~ | ~~`overlay` 渐变色罩**不出图**~~ → ✅ **2026-10-01 撤回**：底座复现不出来，我们复核也是正常出图；**原复现 JSON 无效**（缺 `timeline.schema`；`end` 超素材帧数）。见 §1 | **已关闭** |
| **D2** | `--from` 单独给出时**注释与代码不一致** | ❌ 仍不一致（一行可修；⚠️ 修之前先看 §2 的"注意"） |
| **D3** | `DanmakuSpec.duration_ms` 注释与实现相反 | ✅ **看起来已自行修正**（见 §3 的行号，只需确认） |
| **B1** | `stage` fit 的 **contain + 背景层** | ✅ **contain 摆位：YekiTrim 侧早已实现**（`stageDrawRect` 的 `axis=x/y` + `position` + `padding` + `scale`，判据与参照同源）。**背景层**：配方已实现（同源 cover 层 + `gaussian_blur` + 调整层）⇒ 现在**挡在 §0b**（我们发了那两层，`render` 收下但**没画出来**） |
| ~~**B2**~~ | ~~放大重采样偏软~~ → ✅ **撤回（2026-10-01）**：底座给的可复算判据是"1 像素棋盘放大 1.7778× ⇒ **100% 是插值中间值**"（与 ffmpeg `bilinear` 一致；最近邻是 0%）⇒ 底座走的就是双线性，**"软 2.5 倍"不成立**。⚠️ 顺带一条方法论：**"高频能量比"是内容相关量**（testsrc2 2.59× vs 棋盘 0.003×），底座已把它降级为**只打印的诊断量** | **已撤回** |
| **B3** | 高亮词**段间水平推进量**不同源 | ⏳ 未确认（残差几像素，中英混排时可见） |
| **B4** | 弹幕 `\pos` 静态 cue 仍走轨道级 travel | ✅ **已接（2026-10-01）**：按底座的现成配方，`withMoveTravel` 现在也给静态 cue 追加**零位移** `\move(0,0,0,0,0,travel)`（底座只读 `t2−t1`、坐标忽略）。⚠️ **防御性**：我们这条链目前根本不出 `\pos`（全仓 grep 为 0），这条是给外部工具写的 SRT 兜底；守卫钉了 5 项 + 1 处变异 | **已关闭** |
| **B5 ✅ 已实现并已接** | 字幕**文字阴影**（原描述：契约里没有字段） | ⏳ 待定：要不要扩契约由底座决定；不扩我们就一直如实报 DROP |

---

## 0. ~~新发现：分段路径的中间纹理逐帧累积 ⇒ 常驻第二层长片必 OOM~~ → ⚪ **2026-10-01 复测：在 `c7f29b9` 上复现不出来**

> **复测读数（2026-10-01，同一份工程改成竖屏 + `bg_mode=blur`，doc 里确有 `bg1/bg2` 两条背景层与 `gaussian_blur(radius=16)`）**：
> **200 / 1100 / 1500 帧全部渲完，没有 OOM** ⇒ 下面那份旧读数**在新版本上复现不出来**。
> 所以这条**不再当作阻塞**（YekiTrim 侧仍把开关关着，但理由变成"还没做产品路径验收"）。
> 如果你们想顺手收口，仍值得看一眼中间纹理有没有复用/释放；**不急**。
>
> —— 以下是**旧版本**的读数（留档）：**这是"竖屏模糊底"（B1）当时唯一挡着我们的东西。** 配方在 YekiTrim 侧已经实现并验过
> （`bg1`：同源层按 cover 缩放 + `gaussian_blur`；`bg2`：紧跟的调整层做 `contrast(0.5)+brightness(-0.25)`
> 表达 CSS 的 `brightness(0.5)`；主画面层不变），**但一发就撞下面这条**：
>
> - **现象**：长片渲染 `wgpu error: Out of Memory`（YekiTrim 侧表现为 `*rawvideo* muxer: Invalid argument`
>   —— 那是本仓 panic 之后管道断掉的下游症状，不是参照的问题）。
> - **实测阈值**（同一份工程、同一个范围、`--from 0`）：
>   1 个常驻"有素材图层"（原工程）→ 全片 **5812 帧通过**；
>   **注入一个常驻第二层** → 60/200/600/1100 帧通过、**1500 帧 OOM**（去掉模糊**也一样** ⇒ 不是模糊的问题）；
>   只去掉调整层（留模糊层）→ **仍 OOM**。
> - **可疑点**：单层走**非分段路径**（不分配中间纹理）；一旦有第二层就走 `render_segmented`
>   （`timeline.rs:696`，每个 stage 都 `alloc_texture`），而那批中间纹理**每帧都在涨**
>   （每帧 ~16–33 MB ⇒ 1500 帧 25–50 GB）。**请查中间纹理有没有复用/释放。**
> - **还一条与泄漏无关的**：`gaussian_blur.radius` 上限 **16**（= σ8），而参照的模糊底要 **σ28**；
>   `blur_radius()` 取"多个模糊里的最大值、不叠加"⇒ 叠几趟凑不出来（YekiTrim 侧按上限发并如实记 CLAMP）。
> - **恢复条件**：修好泄漏之后，YekiTrim 把 `BACKDROP_ENABLED` 打开即可（代码与注释都留着，一行开关）。

## 0b. **新发现（2026-10-01）**：背景层被**收下但没画出来**（`render` 静默忽略）

> **这是"竖屏模糊底"（B1）现在唯一挡着的东西**（§0 那条 OOM 已在 `c7f29b9` 上复现不出来）。
>
> **我们的 doc 是对的**（可复算）：一个竖画布 + 横素材 + `bg_mode=blur` 的工程，转译产物里有
> `bg1`（同源层按 cover 缩放 + `gaussian_blur(radius=16)`）与 `bg2`（紧跟的调整层）两条轨；
> 绘制矩形 `0.0,656.3 1080.0x607.5`（`fit 0.5625`）⇒ **上边带 y∈[0,656) 就是底的地盘**。
>
> **现象**：同一个工程渲两次 —— 一次带这两条背景轨、一次把它们整条删掉 —— **成片逐像素相同**
> （采样：上边带 y≈3% 处两次都是 `rgb(243,243,243)`）。若底没画，那里应当是**纯黑**
> （`clearRect` 之后的透明区落进 JPEG 就是黑，这是你们自己文档里写的）。
> ⇒ 结论：`render` **接受**了这两条轨（不报错、不 warn、帧数也对），但**没有把底画出来**。
>
> **请查**：第二条 `kind = "video"` 的轨（`bg1`）在不参与"主画面"合成时，是不是被当成空层跳过了？
> 或者 `SeparableBlur` 那个 stage 的输出有没有真的进合成？（我们把 `brightness/saturation` 拆到调整层，
> 正是照你们 `effect_would_be_ignored` 的校验写的。）
>
> **另**：即便画出来，`gaussian_blur.radius` 上限 16（σ8）而参照要 σ28 ⇒ YekiTrim 侧会如实记 CLAMP。
> YekiTrim 侧的开关（`BACKDROP_ENABLED`）**保持关闭**，等这条查清再开。

## 1. D1（重要）`overlay` 渐变色罩完全不出图 —— ✅ **2026-10-01 已撤回**

> **本条撤回**：底座在 `c7f29b9` 复现不出来；我们按同样形状重跑也复现不出来 ——
> **原复现 JSON 有两处硬错**：① 缺 `timeline.schema`（serde: missing field，根本没进渲染）；
> ② `end: 720` 配 `frame_count: 480` 的资产描述 ⇒ `source_range_exceeded`。
>
> **我们自己的复核**（`.workbuddy/p2-d1-verify.mjs`，素材 `demo/clip.mp4` = 720 帧 / 60fps / 1920×1080）：
> 窗口内（第 300 帧）"有 overlay"vs"去掉 overlay 轨"⇒ 平均差 **39.33** / 最大 115 / 变化字节 6,156,708；
> 窗口外（第 60 帧）⇒ **逐字节相同**；`angle=45` ⇒ 左上恰好 `rgb(255,0,0)`=color_a、右下恰好 `rgb(0,0,255)`=color_b。
> 底座独立判据 `node scripts/handoff-probe.mjs` → **21 / 21**。
>
> 下面内容保留作**历史**（含那段需要修正的 JSON）。

### 症状与影响

任何 `{"kind":"overlay", …}` 的调整层在出片里**整层消失**（片头渐入、色罩、氛围层都属于这一类）。
不报错、不崩溃、退出码 0 —— YekiTrim 的默认出片通道就是本底座 ⇒ 用户看到的是"这块效果没了"。
**连带影响**：YekiTrim 侧 D5 新做的 `overlay.angle`（渐变方向换算）在底座出片里**无法目视验证**。

### 最小复现（自包含，不依赖 YekiTrim）

**第 1 步**：把下面这段存成 `/tmp/ov.doc.json`（`clip.mp4` 换成你们自己的测试片段：1920×1080、timebase 60/1）。
结构 = 一条视频层 + 一条**调整层**（无 `source`、只有 `overlay` 特效，窗口帧 30..126）：

```json
{
  "project_schema": 1,
  "generator": { "app": "dhampir", "version": "0.0.1" },
  "assets": [
    { "id": "clip.mp4", "kind": "video", "name": "test", "uri": "clip.mp4",
      "timebase": { "num": 60, "den": 1 }, "width": 1920, "height": 1080 }
  ],
  "timeline": {
    "schema": 3,   // ⚠️ 第一版漏了这一条 ⇒ serde: missing field（复制这段时别漏） "tracks": [
    { "id": "v1", "kind": "video", "layers": [
      { "id": "main-0", "start": 0, "end": 720,
        "source": { "asset_id": "clip.mp4", "source_in": 0 }, "effects": [] } ] },
    { "id": "ov-0", "kind": "video", "layers": [
      { "id": "overlay-0", "start": 30, "end": 126,
        "effects": [ { "kind": "overlay", "params": {
          "amount": 0.16,
          "r": 0.0784, "g": 0.0392, "b": 0.1569,
          "r2": 0.1569, "g2": 0.0784, "b2": 0.2353,
          "shape": 1, "angle": 0 } } ] } ] }
  ] },
  "view": {}, "render_hints": {}, "extensions": {}
}
```

**第 2 步**：出那一帧（60fps 时间基 ⇒ 帧 60 = 1.0s，落在窗口 30..126 内）

```bash
dhampir frame --project /tmp/ov.doc.json --frame 60 --out /tmp/f60/ --asset-root /tmp
```

**第 3 步**：期望 vs 实际

```text
期望：画面叠一层 #140A28(≈0.078/0.039/0.157) → #28143C(0.157/0.078/0.235) 的线性渐变，
      强度 = amount × Effect.opacity 默认 1.0 = 0.16，应该**明显可见**
实际：**与"把 ov-0 那条轨删掉"逐像素相同**（没有任何色罩）
```

### 已核对过（说明不是"调用方传错字段"）

| 项 | 事实 |
|---|---|
| 效果名与参数 | `kind:"overlay"` 与 `amount/r/g/b/r2/g2/b2/shape/angle` **就是底座自己映射代码读的那几个字段**（§「可疑点」第 1 条给了行号），也与它自己的单测形状一致（同文件单测用 `effect("overlay", &[("amount",1.0),("angle",180.0),("shape",1.0)])`） |
| 图层形态 | 调整层：**无 `source`**、只有 `effects`，挂在视频层之后 |
| 窗口 | `start=30, end=126`（覆盖被取的那一帧）✓ |
| 强度 | `Effect.opacity` 缺省 1.0 ⇒ `weight = 0.16 > 0` ✓ |
| 最小化 | 上面的 JSON **已经把"逐帧包络不生效"这个可能性排除了**（这条层没有 `keyframes`，强度只来自 `amount`） |
| 是否只有 overlay 一支 | ⚠️ 未测：`flash` / `vignette` / `noise` 走的是同一族调整层 —— 如果它们**也不出**，那是同一条根因；如果它们**出**，说明只差 overlay 这一支。**建议复现时顺手把 `kind` 换成 `flash` 各出一条对比**（这一步能直接二分掉一半范围） |

### 可疑点（本次只读代码得到的线索，未验证）

1. **调整层的判定链**：`Layer::is_adjustment()` = `source.is_none() && !effects.is_empty()`
   （`crates/dhampir-timeline/src/layer.rs:235`）⇒ `compose.rs:329` 传给渲染层 ⇒
   `plan_steps()` 用它决定产 `Step::Adjust`（`crates/dhampir-core/src/render/timeline.rs:1415`，第 1420 行 `if layer.is_adjustment`）。
   **先确认这条链在你的复现里到底走没走到 `Step::Adjust`**（打一行日志/单测即可）。
2. **没有源=被 `continue` 掉**：`compose_layers` 里
   `let Some((view, size)) = resolver.texture_for(&layer.source, …) else { continue };`
   （同文件 ~1005 行）—— 如果那条调整层被判成 `Draw` 而不是 `Adjust`，它就会**静默消失**在这里。
3. **`Step::Adjust` 分支自己的前置条件**：`Step::Adjust { … } => { let Some(from) = current else { continue }; … }`
   （同文件 771–772 行）—— 如果这一帧上 `current` 还是 `None`（前面没有 Draw 过东西），这一层会被跳过。
4. **空间（space）筛选**：`apply_stage` 里 `color_mask_params(batch, space.target, frame)`
   （同文件 902 行）；`overlay` 的特效登记里 `space` 是 **Document**（`crates/dhampir-core/src/effects.rs` 的 `OVERLAY` 定义），
   而 `space.target` 是按 pass 传进来的 —— 确认 **Document 那一趟有没有跑到这条层**。
   （`vignette` 也是 Document，所以第 1 条那个"换个 kind 对比"同样能验证这一支。）

### 验收判据（不需要 YekiTrim）

```bash
# 出同一帧两次：带 overlay 层 / 把它删掉，然后比"平均绝对差"
ffmpeg -v info -i /tmp/f60/with.png -i /tmp/f60/without.png \
  -lavfi "[0][1]psnr" -f null -          # psnr 的摘要行是 info 级，别用 -v error
# 期望：不是 inf（两帧不同）；且差异集中在 overlay 的窗口帧 30..126 内，
#       窗口外（例如帧 10）两次出图应当**逐字节相同**
```

**请不要这样修**：不要在 YekiTrim 的转译器里"用别的效果凑一个色罩" —— 那会掩盖问题，
底座修好后还会变成第二份实现。转译侧现在已经把它**正确发出**（报告里没有 DROP）。

---

## 2. D2（小）`--from` 单独给出时，注释与代码不一致

**位置**：`crates/dhampir-worker/src/bin/dhampir.rs::frame_range`（~1170 行）。

注释（1186–1192 行）写的是：

```text
* 只给 `--from 10`：“从第 10 帧起，到工程结尾”；
* 只给 `--to 5`：“从第 0 帧到第 5 帧”。
```

而代码（1193–1194 行）是：

```rust
let from = args.from.unwrap_or(0);
let to = args.to.unwrap_or(from);   // ← 只给 --from 10 时 to = 10 ⇒ 只出第 10 帧
```

**影响**：信注释的调用方会拿到"一帧"而不是"到结尾"，**退出码还是 0**（静默出错）。
（YekiTrim 两头都给 `--from 0 --to N-1`，所以没踩到。）

**⚠️ 修之前注意**：`frame_range(args)` 只拿得到 `args`，而 `cmd_frame` 是**先调它、后 load 工程**
⇒ 想让 `to` 缺省成"工程结尾"，要么把工程加载提前、要么把返回值改成 `Option<Frame>` 由调用点补。
**另一条同样合法的路是改注释**（让文档与实现一致）—— 两条选一条，别只改一半。

---

## 3. D3（小）`DanmakuSpec.duration_ms` 的模块头注释

**位置**：`crates/dhampir-timeline/src/danmaku.rs`（模块头 ~15 行、实现 ~128/142/151/229 行）。

**本次核对**：文件里已经有多处"这条先前写错了…"的更正注释，
实现是 `let travel_ms = cue.travel_ms.unwrap_or(spec.duration_ms);`（151 行）⇒
**"素材里的 `End` 不参与、`\move`/`duration_ms` 说了算"这个口径与实现看起来一致**。
⇒ 判定为**已自行修正**，只需确认模块头那句是否也同步过；若仍觉得矛盾，请以 151 行实现为事实。

---

## 4. B 组：量化出来的不足（这几条来自 YekiTrim 侧的历史读数，需底座侧自测确认）

> 这几条**没有**像 D1 那样的最小复现，因为它们是"观感/精度"类差异。
> 每条给了：现象、YekiTrim 侧读数、底座侧建议的自测方法。

| # | 现象 | YekiTrim 侧读数（历史） | 底座侧建议自测 |
|---|---|---|---|
| **B1** | `stage` fit 的 **contain（摆位）+ 背景层（模糊底）** 没做 | 竖屏画布：只改 `orientation` 一行，基线像素差 **87.98**、位移 −24px；YekiTrim 转译器补了 cover 的 `f` 之后 → **44.20** / **+2px** | 用 1920×1080 源导出 1080×1920：主画面是否被 **cover 裁切铺满**（无黑边）；底座侧是否已有 `stage`/背景层概念（本次**没查**，只是记成"没做"） |
| **B2** | 放大**重采样偏软** | 手写双线性放大 1.7778x，比浏览器 `drawImage` **软 2.5 倍**（同一帧逐像素比） | 造一张高频细节图，放大 1.7778 倍后比高频能量（拉普拉斯方差）或直接与浏览器 `drawImage` 比 |
| **B3** | 高亮词**段间水平推进量**不同源 | 残差：段间推进量两端不同 —— 浏览器 `measureText` 与画字同引擎（准），`drawtext` 拿不到别段宽（用布局逻辑字宽，中英混排差几像素） | 造一条含 `<span class="hl">` 的 srt，量"高亮段之后那一段"的起始 x |
| **B4** | 弹幕 **`\pos` 静态 cue** 仍走轨道级 travel | 参照是**逐条** travel；底座在 cue 有 `\move` 时已能逐条（6 参数 `\move` 那条路已通），**静态 cue 没有 `\move` 可读** ⇒ 回退轨道级平均 | 造一条只有 `\pos` 没有 `\move` 的 cue，看它在屏时长是"逐条"还是"轨道级平均" |
| **B5 ✅ 已实现并已接** | 字幕**文字阴影**（原描述：契约里没有字段） | YekiTrim 侧对应的是 DROP 之一（`style.subtitle` 的**文字阴影**；入场动画 `cycle-5` 那一项已于 2026-10-01 从 YekiTrim 的 schema 移除 ⇒ 不再是差异） | 纯契约问题：要不要给字幕轨加阴影字段由底座定；不扩我们就一直如实报 DROP（**不是 bug**） |

---

## 5. 修好之后 YekiTrim 侧会做什么（联动约定）

1. ~~**D1 修好后**~~ → **D1 已撤回，这一步已做过**：先跑 YekiTrim 的对拍三样本复测（`demo` / `portrait` / `real`）——
   overlay 色罩会真的出现 ⇒ **基线数字必然变化，那是期望的变化**，我们会重新标定；
   然后划掉台账里的 D1，并把 `docs/yekitrim-integration.md` §4.8 的对应行改成"已接"。
2. **D1 修好后顺带能验的**：YekiTrim 的 `overlay.angle`（D5）—— 我们已按底座约定
   `base = css − 90`（CSS 0° = 向上、顺时针；底座 `dir=(cos,sin)` 0° = 向右），缺省 `135°`（= 参照的 `to bottom right`）。
   请顺手确认一条：**`angle=45` 时渐变应当是"从左上到右下"**（我们按这个换算）。
3. **D2 修好后**：YekiTrim **不改代码**（我们两头都给参数），只会核对一下文档措辞。
4. ~~**B1/B2 若修**~~ → **B2 已撤回**（见上表：底座走的就是双线性，"软 2.5 倍"不成立；判据可复算）、**B1 归我们**（三步配方：同源 cover 层 → 调整层 gaussian_blur → 主层，见 §7.4）。

> ⚠️ **YekiTrim 侧为什么盯这条**：D1 未修之前，"删掉第二条渲染链路"会**卡死** ——
> 详见 `docs/plan/s4-second-chain-removal-decision.md` —— **该硬前提已于 2026-10-01 作废**（D1 撤回），
> 现在那条决策只剩**产品取舍**（"没装底座就没预览"）要拍板。

## 6. 溯源（只想追证据时才看；不需要读 YekiTrim 仓）

- 本交接单的原始版：YekiTrim 仓 `docs/dhampir-base-issues.md`（D1–D3 的初版复现）；
- 状态表：YekiTrim 仓 `docs/plan/dhampir-parity-register.md` §四；
- B 组的读数出处：YekiTrim 仓 `docs/yekitrim-integration.md` §六（第 3/4/5 条）与 §4.8；
- D1 的最小 doc 原始文件（含 `keyframes` 包络那一版）：YekiTrim 仓 `.workbuddy/dhampir-unify-out/mini-overlay.doc.json`。
