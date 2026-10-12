# 规格：`elements` 元素系统（文字 + 贴纸）

> 立档：2026-10。**汇总本项在本次会话中的全部决定与后续修正** ——
> 它原本只在 [`polish-enrichment-proposal.md`](polish-enrichment-proposal.md) §2.2，
> 本文把它**独立成可实施规格**，并补上会话后段调研带来的**两处重要变化**。
>
> 上位文档：[`polish-upgrade-master-plan.md`](polish-upgrade-master-plan.md) **§5**（总表第 5 项，★★★，批 3）。
> 状态：✅ **已拍板**（两轮：命名与合并 → 后段补上底座字段前提与动效机制）。

---

## 0. 一句话

**给"往画面上放东西"这件事建立一个统一零件 `elements`，带两个子类型 `sticker`（图）与 `text`（字）。**

**为什么这是总表里 ★★★ 的一项**：今天本仓**没有"文字"这一类**。
所有"想在画面上写一句话"的需求（钩子标题、章节卡、吐槽、计数）只能退化成**贴纸图**（不达意）。

---

## 1. 已拍板的决定（会话中逐条确认）

| # | 决定 | 说明 |
|---|---|---|
| **1** | **顶层叫 `elements`（元素）** | ⚠️ **不可叫 `overlay`**（该 id 已被"氛围色罩"效果占用，两个同名让门禁和人都分不清）；**也不叫"贴图"**（那词把文字排除了） |
| **2** | **两个子类型：`sticker` / `text`** | `sticker` = 现有能力；`text` = 新增 |
| **3** | **合并而不是新开类型** | 共享落点求解器、共享素材链、用户视角是一类东西 |
| **4** | **轨道：沿用 lane key `sticker` 不改**，只把**显示标签**改成「元素」 | ⇒ **老工程零迁移**（改 key 会破坏存量） |
| **5** | **归"视觉特效"轨还是新轨？** | **不做新轨** —— 沿用现有 lane，避免 §4.3 那 9 个触点的连锁改动 |

---

## 2. 结构

```toml
[[elements]]
kind  = "text"           # 或 "sticker"
time  = 3.0
end   = 5.0

# --- kind = "text" ---
text  = "这就是传说中的…"
style = "hook"           # hook | chapter | comment | count
pos   = "top"            # top | center | bottom —— ⚠️ 见下方更正

# --- kind = "sticker" ---
keyword     = "ysyk"
sticker_tag = "害怕"
x = "auto"
y = "auto"
```

### ⚠️ `pos` 的更正（2026-10-12 实测）：**只有预览生效，成片一律贴底**

`pos` 在 `process` 里是**真被消费**的（`placement.rs::TextAnchor::parse` 按它选一条纵向带，
`pick_box_position` 在这条带里挑不重叠的落点，再写回 `x`/`y` 百分比）。
**预览**读那对 `x`/`y`，所以三档在预览里看得出差别。

**但成片侧没接通**：转译器把文字元素转成 ASS 时对齐码**硬编码 `Alignment 2`**（底部居中），
`pos` 既没进对齐码、也没进底座任何字段 ⇒ **三条档位在成片里长得一样**。

★ **根因不是"忘了接线"，而是底座没有可用的纵向锚**：

* `bottom_margin` 身兼两职 —— 底座 `text_layout::layout` 里
  `max_width_px = sequence_width × (1 − 2 × margin)`（左右安全宽）**与**
  `block_top = 1 − margin − …`（纵向位置）读的是**同一个值**。
  而安全宽 0.875 要求它 = **0.0625**；`top` 档要抬到画面上方却需要它 ≈ **0.6**，
  抬上去宽度就是负数 ⇒ **一个字都排不出来**（转译器注释记着这次真实事故：
  误填 0.5 时"每一条文字元素都静默消失"，报告里**一行差异都没有**）。
* ASS 的 `{\an8}` 会被底座 `parse_ass` **当覆盖标签丢掉**；`force_style` 只拼五个键。

⇒ **要修必须底座加一个与 `bottom_margin` 解耦的纵向锚字段**，
需求与建议接口见 [`dhampir-elements-request.md`](dhampir-elements-request.md) 的「T7 纵向锚」。
**在那之前**本仓不假装它生效：设置界面与规格都照实标注。

### `text` 的四个 `style` 预设

| `style` | 用途 | 观感 |
|---|---|---|
| `hook` | 钩子标题 | 大号、高对比、强描边 |
| `chapter` | 章节卡 | 中号、带底、居中 |
| `comment` | 吐槽 | 小号、带底色块 |
| `count` | 计数/积累（"第 3 次"） | 中号、可递增 |

> ⚠️ **2026-10-10 更正（核实于 `polish-upgrade-consolidated.md` §3）**：下文 §3 依据
> `p9-downstream-requirements.md` 得出的"底座缺 `stroke`/`highlight_color`/`entrance`"**已过期**：
> - `SubtitleStyle.stroke` 是 **`String`**（`"12px #403c3b"`）、`highlight_color` **存在**且
>   **转译器/预览/封面三条腿都在用**（`types.rs:503–522`）；
> - `entrance` 是**有意从契约下线**（`project/legacy.rs:19–38`，有读时吞并单测），
>   **不是"底座没做"** ⇒ E4 不能表述为"等底座补 `entrance`"，只能"本仓逐帧近似"或"重新提需求+发明规格"。
> - ⇒ **§9 的 E4 里只有 T5（真卡拉OK）确实被阻塞**（缺**词级时间戳**），
>   **T1/T2/T3/T4/T6 全部本仓自足、可立即实施**。

---

## 3. ⚠️ **新增变化①：底座有三个字段缺失，是本项的前置**

> 这是**会话后段的调研**查出来的（`editing-techniques-catalog.md` §0.2），
> **原方案里没有** —— 它改变了本项的**实施前提**。

`p9-downstream-requirements.md` §2.2 明确记录：

| 层 | 字段 | 底座现状 | 解锁什么 |
|---|---|---|---|
| `SubtitleStyle` | `stroke` | ⚠️ **只有 `outline: bool`**（见下） | 描边 |
| `SubtitleStyle` | **`highlight_color`** | ❌ **无** | **卡拉OK / 强调** |
| `SubtitleStyle` | **`entrance`** | ❌ **无** | **打字机 / 弹跳入场** |

### 3.1 ⚠️ 一个精确的更正（我核过转译器）

调研说"`stroke` 为无"，但实际是 **`outline: bool`**（**布尔，不是宽度**）。
转译器注释（`polish-to-dhampir.mjs:3613`）写得很清楚：

> `-webkit-text-stroke: 12px #403c3b` **→ 底座只有 `outline: bool`**

而它**已经把 CSS 的 12px 折算成 FFmpeg 的 `borderw`**（注意：**CSS 描边居中、
FFmpeg 全在外侧，所以要折半** —— 注释记着实测"直接填 12 会让描边粗一倍"）。

⇒ **所以"白字黑边"是有的，但"描边按字号比例（5–8%）可调"做不到** ——
对静态字幕够用，**对要做"大字/综艺花字"的元素就不够**。

### 3.2 ⇒ **本项的正确实施顺序被改变**

| 原计划 | ✅ 修正后 |
|---|---|
| 直接做 `elements`（本仓自足） | **先向底座提 `highlight_color` + `entrance` 两个字段**（+ `stroke` 从 bool 升级成可调宽度） |

**理由**：**光有文字层而"不会动、不能强调"，只解决一半**（见 §4）。

⚠️ **但两者可以并行**：
- **`elements.text` 的基础版（静态文字）不依赖它们** ⇒ **可以先做**；
- **动效版（打字机/弹跳/卡拉OK）依赖底座那三个字段** ⇒ **排在后面**。

---

## 4. ⚠️ **新增变化②：文字动效是"本项最值钱的部分"**

> 同样来自会话后段调研：**VTuber 切片的视觉语法本质是"字幕即设计"** ——
> **92% 移动用户静音观看、80% 说字幕让他们看到最后**
> （Verizon Media/Publicis 2019，经 [KIRARI](https://www.kirari.io/blog/vtuber-clip-subtitle-guide) 引述；⚠️ **二手引述，原始报告未核到**）。

⇒ **文字不只是"能放上去"，更要"能动"**。以下是**与 `elements` 直接相关的动效**：

| # | 动效 | 表达 | 依赖 | 可自动化 |
|---|---|---|---|---|
| **T1** | **弹入**（pop-in） | 字带弹性放大出现（0.7→1.15→1.0） | **`entrance`**（或本仓逐帧重算） | ✅ 全自动 |
| **T2** | **打字机**（typewriter） | 字逐个出现 | **`entrance`** 或逐帧改可见字符数 | ✅ 全自动 |
| **T3** | **强调放大** | 关键词突然变大再回落 | `highlight_color` / 逐帧 | ✅ 已有 `.hl` 命中词 |
| **T4** | **文字抖动** | 字跟画面一起抖 | 复用 `shake` 的逐帧函数 | ✅ 全自动 |
| **T5** | **逐字高亮**（Karaoke） | 当前词逐字变色 | **`highlight_color`** + 词级时间戳 | ⚠️ 需词级（或用整行拆条近似） |
| **T6** | **字幕底衬**（plate） | 字幕背后半透明色块 | `overlay`（现有） | ✅ 全自动 |

### 4.0b ⚠️⚠️ **更正（2026-10-11）：T1/T3/T4/T6 并不"本仓自足"**

上面 §3 那句"全部本仓自足"**说的是"不必等 `entrance`/`highlight_color` 两个字段"**，
**不等于"底座已经有逐帧能力"**。拿底座的真实渲染路径逐项对过之后（保守决策 #125–#128）：

| 动效 | 需要什么 | 底座字幕轨现状 | 结论 |
|---|---|---|---|
| **T2 打字机** | 逐帧改"显示前几个字" | 本仓把 cue 按帧拆开即可 | ✅ **已实现**（`render/elements.rs` + 转译器拆 cue） |
| T1 弹入 | `font_ratio` 逐帧缩放 | **没有这个通道** | ❌ 需底座 |
| T3 强调放大 | 换色 + 放大 | 换色 ✅；**放大没有通道** | ⚠️ 半截需底座 |
| T4 抖动 | **持续**逐帧位移 | `rise_in_px` 是**一次性**入场位移 | ❌ 需底座 |
| T6 底衬 | 文字背后的色块 | `force_style` **只拼五个键**，没有 `BorderStyle`/`BackColour` | ❌ 需底座 |

⚠️ **T6 最容易误判**：ASS 确实支持 `BorderStyle=3`，但底座那条路是
`subtitles=…:force_style=…`，而 `force_style` **根本不传这个键** ⇒
在 `.ass` 里写它会被盖掉。且底座**有意**把填充与描边都画成 `WHITE_INK`
（覆盖度要靠"全是白墨"才恢复得出来），底衬要的是**另一种墨**，与那条设计正面冲突。

⇒ 四项的接口形状、数值与验收判据见 **`docs/plan/dhampir-elements-request.md`**。

> ### ⚠️ 上表的"结论"一列**已过期**（2026-10-12 核，实施 §5 E4a 时复核）
>
> 底座**后来真做了**那三个通道 —— `Dhampir/crates/dhampir-timeline/src/text_motion.rs`
> **576 行**，T1/T4 的原语都在里面：
> `back_out3`（弹入曲线）· `text_shake_px`（持续抖动的逐帧位移）·
> `text_envelope_scaled` · `scale_in_params` / `shake_params`（从 `SubtitleStyle` 读参数）。
> 所以 **T1 弹入 / T4 抖动 / T6 底衬 / T2 打字机 四项在本仓都已接通**：
> 转译器写字段（`motionFields`，`undefined` 一律不写 ⇒ 存量产物逐字节不变），
> 预览侧是 `engine.js::getElements` 里那段"与出片侧同一条式子"。
>
> ⚠️ 保留上表是为了记住**它当时是对的**（"别以为本仓自足"这个判断救了工期），
> 而不是让后来的人照着它去重新提一遍需求。**复核口径：`text_motion.rs` 是否存在、
> 是否有真实现**（不是只看函数名 —— 那也是"代码在 ≠ 生效"）。
>
> ⚠️ 本次复核新发现的缺口**不在动效**，而在**纵向档位**：
> `pos`（top/center/bottom）**只有预览认、成片不认** ——
> 见 §2 的 `pos` 行与 `dhampir-elements-request.md` 的「T7 纵向锚」。

### 4.1 ⚠️ **一条硬约束（必须写进实现）**

> **文字动效必须逐帧计算，不许用 CSS transition / 状态机。**

否则 **`seek` 到第 100 帧与"播到第 100 帧"长得不一样** ⇒ **预览与导出分叉**。
这与本仓既有纪律同源（`handheld_sway` 注释："**纯 `t` 的函数，无累积状态**"）。

### 4.2 📐 文字的参数基准（可直接当默认值，出自 [KIRARI](https://www.kirari.io/blog/vtuber-clip-subtitle-guide)）

| 项 | 横屏 | 竖屏 |
|---|---|---|
| 字号 | 48–64 px | 56–72 px |
| 描边粗度 | **字号的 5–8%** | 同 |
| 每行字数 | 15–18 字 | 同 |
| 最多行数 | 2 行 | 同 |
| 位置 | 下 1/3 | 同 |
| 配色 | **白字黑边**（万能解）；强调用**主播应援色** | 同 |

⚠️ **"描边粗度"这一行正好卡在 §3 的 `stroke` 缺口上** ——
**参数有了，但底座现在只有 bool** ⇒ **这是提需求的具体依据（带数值）**。

---

## 5. 复用与工作量（原方案，仍然有效）

### 5.1 白拿的部分

| 需求 | 已有实现 |
|---|---|
| 折行 / 安全宽 / 字号自适应 | `wrapCaptionParts`、`CAPTION_SAFE_WIDTH_RATIO`(0.875)、`CAPTION_MAX_LINES`(3)、`CAPTION_MIN_FONT_RATIO`(0.7) |
| 与字幕带错开 | `placement.rs::caption_band()` |
| 排版观感（断行/描边/阴影） | 封面设计器那套参数，`style` 预设可从它抄 |
| 素材链（仅 `sticker`） | `keyword → 文件` 解析、下载、缓存都已就绪；`text` **不需要素材** |

### 5.2 ⚠️ 主要工作量（**不是白拿**）

`placement.rs` 现在按 **`280 × SK` 固定盒**算障碍；
**文字是变宽高**（字号 + 折行） ⇒ **落点求解器必须支持动态盒**。
不做这个的后果：**"大字压住主播脸"**。

---

## 6. 底座表达（**不需要底座加新原语**）

| 子类型 | 底座表达 | 需改吗 |
|---|---|---|
| `sticker` | `AssetKind::ImageSequence` + `Transform` | ❌ **一个字不用改** |
| `text` | **文字链本来就有**（`AssetKind::Subtitle` / `TextOverlay`，字幕与弹幕走的就是它） | ❌ **不需要新原语** |

⇒ `text` = **一条自动生成的、样式独立的字幕层**（独立轨 + 自己的 `SubtitleStyle`）。

⚠️ **注意区分**：§3 缺的是 **`SubtitleStyle` 的三个字段**（描边宽度/高亮/入场），
**不是"文字这个能力"**。**静态文字今天就能做**。

---

## 7. 词汇表登记（`spec/effects.json`）

| 锚点 | 值 |
|---|---|
| `canvas` | `handlers.elements`（参照侧照常实现，供对拍） |
| `dom_preview` | `getElements` |
| `dhampir_case` | `elements`（内部按 `kind` 分派） |
| `post_span` | `null`（**持续类**，不被瞬时 clamp） |
| `lane` | **沿用现有 `sticker`**（只改显示标签为「元素」） |

---

## 8. 验收清单

```bash
# 1. 词汇表四锚点齐
node scripts/check-effect-registry.mjs

# 2. 类型真源 + 后处理
cargo check --workspace --all-targets          # 0 warnings
cargo test -p yekitrim-polish --lib project process llm

# 3. 转译器 + 变异自证
node scripts/check-yekitrim-translator.mjs --mutate

# 4. DROP/CLAMP 基线（多一条少一条都红）
node scripts/check-dhampir-drop-list.mjs

# 5. ⚠️ 改了 templates/index.html 必须重建二进制（include_str!）
cargo build -p yekitrim-cli

# 6. 真出片一次 + 对帧验证（别只看平均值）
node scripts/yekitrim-compare.mjs --clip <片段目录> --yekitrim <yekitrim.exe> --out .workbuddy/…

# 7. 结构门禁
python3 scripts/check_module_map.py && python3 scripts/check_module_boundaries.py
```

### 额外判据（本项特有）

- ✅ **落点**：文字**不压主播脸**、**不与字幕带重叠**（动态盒生效）；
- ✅ **逐帧幂等**：`seek` 到任意帧与顺序播放到该帧**结果一致**（§4.1）；
- ✅ **老工程零迁移**：没写 `elements` 的工程逐字节不变（R4）；
- ✅ **预览 = 成片**（改模板后必须真出一次片确认）。

---

## 9. 实施顺序（修正后）

| 步 | 内容 | 依赖 | 能立刻做吗 |
|---|---|---|---|
| **E1** | **`elements` 骨架**（`kind` 分派、词汇表四锚点、lane 沿用） | 无 | ✅ |
| **E2** | **`elements.sticker` 迁进来**（现有贴纸能力，验证骨架不破坏存量） | E1 | ✅ |
| **E3** | **`elements.text` 静态版**（`style` 预设、落点动态盒、折行复用） | E1 | ✅ **不依赖底座** |
| **E4** | **文字动效**（弹入/打字机/强调/抖动/Karaoke） | ⚠️ **底座 `highlight_color` + `entrance` + `stroke` 宽度** | ⏳ 等底座 / 或本仓逐帧近似 |
| **E5** | **LLM 提示词**：教 LLM 用 `elements.text`（含四个 `style`） | E3 | ✅ |

> 💡 **建议：E1–E3 + E5 现在就做**（**完全本仓自足**，且解决了"画面上能放字"这个最缺的能力）；
> **E4 分两批** —— 能逐帧近似的（弹入/打字机/抖动）可以做，
> **真卡拉OK 等高亮相关的等底座字段**。

---

## 10. 与其他项的关系

| 关联项 | 关系 |
|---|---|
| **总方案 §5（本项）** | 批 3 |
| **总方案 §25 底座文字层三字段** | ⚠️ **本项 E4 的前置**（提需求） |
| **`emotion-narrative-engine` 形态 C 吐槽役** | **重度依赖 `elements.text`**（且已定合规红线：**只放大主播的话，不替主播说话**） |
| **`editing-decision-layer` 的 `role`** | `hook`/`chapter` 的配方**要用 `elements.text`** 表达大字/章节卡 |
| **`editing-techniques-catalog` B1–B7** | 文字动效全表（本项 §4 是它的子集 + 实施锚点） |

---

## 附：证据等级

| 等级 | 内容 |
|---|---|
| **仓库内部（已核实）** | 转译器 `:3613` 明记 **"底座只有 `outline: bool`"**，且已把 CSS `12px` 折算成 FFmpeg `borderw`（含"折半"的实测理由）；`p9-downstream-requirements.md` §2.2 记 `SubtitleStyle` **无 `highlight_color`/`entrance`**；`placement.rs` 的 `280` 固定盒与 `caption_band()`；`wrapCaptionParts` 的四个常量（0.875/3/0.7）；`overlay` id 已被 fx 效果占用 |
| **外部（文案基准）** | 字号/描边 5–8%/每行 15–18 字/白字黑边 —— [KIRARI](https://www.kirari.io/blog/vtuber-clip-subtitle-guide) |
| ⚠️ **二手引述** | **92% 静音观看 / 80% 字幕看到最后** —— 经 KIRARI 引述，**原始报告未能核到** |
| ⚠️ **待验证** | 底座 `entrance`/`highlight_color` 若交付，**是否支持"逐帧驱动"**（卡拉OK 要求按进度插值，不是一次性入场动画） |
