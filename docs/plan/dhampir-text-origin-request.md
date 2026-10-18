# 元素文字的位置：请底座收一个**绝对盒原点**（跨仓需求书）

> 触发：用户 2026-10-15 报「**文字层的位置和描边没有正常实现**」。
> 描边那一半本仓已修（见文末）；**位置这一半必须动底座**，本文件是需求书 —— 按
> 既有的 \`dhampir-rotation-spread-request.md\` / \`dhampir-heartbeat-request.md\` 同一格式写。
> ⚠️ 本仓**不先写键**：底座不收之前写出去 = 「配了、不报错、没效果」，正是本仓最恨的形状。

---

## 一 · 实测证据（三处，都是读真源码 / 真产物，不是推测）

### 1. 面板写的 X/Y，转译器**一次都没读**

真跑（夹具 \`.workbuddy/textprobe/polish.toml\`：一条 \`type="element" kind="text"\`，
\`x = "20.00%"\` / \`y = "30.00%"\`）：

\`\`\`
node tools/polish-to-dhampir.mjs .workbuddy/textprobe --out .workbuddy/textprobe/doc.json
\`\`\`

产物 \`dhampir-project.doc.json\` 里：文字层的 \`transform\` 是 **恒等** \`{x:0,y:0,scale:1,rotation:0}\`，
而 \`20.00%\` / \`30.00%\` **一处都不出现**。
\`case 'element'\` 里读 \`event.x/event.y\` 的地方：**0 处**（那两行只在 \`case 'sticker'\` 里）。

### 2. 底座**没有**横向位置字段

\`dhampir-timeline/src/layer.rs::SubtitleStyle\`（实读全部字段）：
\`font_ratio / bottom_margin / anchor_bottom / max_lines / color / outline / stroke_ratio /
stroke_color / fade_in_ms / fade_out_ms / rise_in_px / rise_out_px / font_family / font_weight /
line_height / safe_width_ratio / shrink_min_scale / keep_all_lines / highlight_color / shadow_*\`。

* 纵向有 **\`anchor_bottom\`**（= 文字块**底边**的归一化 y，宿主就是拿它表达"上/中/下"）；
* 横向**没有任何字段** —— 文字块只能在 \`safe_width_ratio\` 里**居中**。

### 3. DOM 预览那条腿在默认模式下不画文字

\`PolishPreview.vue:2488\`：\`byDom = !dhampirMode || DHAMPIR_NO_DRAW.has(effectAnim)\`
⇒ **Dhampir 预览（用户默认）下，位置/描边这类档由底座画**，DOM 那层（它确实读 \`x\`/\`y\`）
处在 \`z-index:10\`、被 \`z-index:40\` 的底座画布盖着。

**⇒ 结论：面板的 X/Y 今天是"死控件"**（Dhampir 预览与成片都不看它），
用户拖了没有任何变化 —— 与他的复报一致。

---

## 二 · 提议契约（底座侧，最小改动）

给 \`SubtitleStyle\` 加一个可选字段：

\`\`\`rust
/// **绝对盒原点**（归一化：0..1，左上角）。\`None\` = 沿用本仓自己的锚点（现状，逐字节零回归）。
///
/// 与 \`anchor_bottom\` 的分工：那个表达"块底边贴哪"，本字段表达"**整块**摆在哪"。
/// 给了本字段 ⇒ **跳过**本仓的纵向锚点与横向居中，直接把排版出来的**盒子左上角**放到这里
/// （字号/折行/缩字仍由本仓算，绝不外移）。
#[serde(default, skip_serializing_if = "Option::is_none")]
pub origin: Option<[f32; 2]>,
\`\`\`

* **为什么是"盒左上角"而不是"行中心"**：宿主的 \`ev.x/ev.y\` 就是 \`process\`
  （\`placement::Pick\`，\`process/placement.rs:1124\` 的字段文档）算出来的**盒左上角百分比**；
  一一对应才不会又引入一次换算（本仓已经因为"两个基准"吃过两轮）。
* **\`None\` ⇒ 一个键都不写**（R4）。
* 与 \`layer.transform\` 的关系：本字段是**绝对**语义；\`transform\` 仍是"相对排版结果的位移/旋转"，
  两者可叠加（宿主现在写恒等 transform）。

---

## 三 · 本仓这一侧要做的（底座落地后，一次做完）

1. \`tools/polish-to-dhampir.mjs\` 的 \`case 'element'\`：\`event.x/y\` 解析得出来（\`ratio()\` 非 null）
   ⇒ 写 \`origin\`；解析不出来 ⇒ **不写**（现状）。
   ⚠️ **\`"auto"\` 字符串的陷阱**：没跑过后处理的工程里键**存在**、值是字符串 \`"auto"\`
   （见 \`case 'sticker'\` 那条注释），判据必须是 \`ratio()\` 的结果，不是 \`!== undefined\`。
2. 预览那条腿（\`engine.js\` / \`PolishPreview.vue\`）**不动**：它本来就按 \`x\`/\`y\` 摆；
   底座落地后两边终于同源（这也是"预览=成片"的必要条件）。
3. 判据（\`scripts/check-yekitrim-translator.mjs\`）：
   * 给了 \`x/y\` ⇒ 产物里出现 \`origin\`，且**逐值等于** \`x/100\`、\`y/100\`；
   * 没给 / \`"auto"\` ⇒ **一个键都不写**（R4）；
   * \`check-dhampir-drop-list.mjs\` 的清单**不许变**（这条不引入 DROP/CLAMP）。

---

## 四 · 验收判据（底座侧，4 条）

1. **\`None\` 零回归**：既有工程（不写 \`origin\`）出片**逐字节不变**（R4 那条老判据照跑）。
2. **给了就真在那**：\`origin = [0.2, 0.3]\` ⇒ 真出片里**文字块左上角**落在 (384, 324) ±2px
   （1920×1080；\`text_plate\` / \`text_overlay\` 两条路都要过 —— 后者是预览，前者是出片/覆盖度）。
3. **盒子仍是本仓算的**：同一 \`origin\`、把文字从 4 字换成 20 字 ⇒ 左上角**不动**、只有右下角变
   （证明"只搬不量"）。
4. **与 \`anchor_bottom\` 不同时生效**：两个都写时 \`origin\` **赢**，且报告里说清
   （避免"两个字段各说一套、谁也不生效"）。

---

## 五 · 另外两条路为什么不走（写下来省得下次再讨论）

| 路 | 为什么不 |
|---|---|
| **在前端复刻底座排版**（算盒尺寸 → 换算成文字原点） | 本仓已有的"一份规则 N 份实现"清单里已经躺着 5 份（模板 / canvas.js / timeline.rs / engine.js / 转译器）。用户报的「位置修了几轮还是不正常」**正是近似**造成的（实测三值近似在底部差 231/311/391px，随行数变）。 |
| **让 DOM 接管文字、底座别画** | 底座**没有**"不画这条字幕轨"的开关；而且那样是"预览用 A 套排版、成片用 B 套" —— 用户看得见的那一份与成片不是一份，本仓的老病。 |

---

## 六 · 描边那一半：**本仓已修**（不需要底座）

用户口径（原话，2026-10-14）：「元素文字的描边**可以参考字幕的默认值**」。

* 之前：元素描边 = \`TextStyle::base_stroke_px()\` 的四档值（6/5/2/4），而 ASS 的 \`Outline\` 只有**一半**
  ⇒ \`comment\` 档在 1080p 上约 **1px**，肉眼等于没有描边。
* 现在：\`process\` 的 \`layout_text_elements\` 收 \`[style.subtitle].stroke\` 解析出来的宽度
  （默认 \`12px\`），写进 \`source\` 串 —— **转译器与 DOM 预览读的是同一个串**
  （一处改、两条腿同时生效）。
* 实测：夹具带 \`source = "comment|80|12|…"\` ⇒ 产物 ASS
  \`Style: …,1,6,1,2,…\`（\`Outline = 6\` = 12/2 的外侧宽；之前是 1），
  底座 \`stroke_ratio = 12/2/1080 = 0.005556\`（之前 0.000926）。
* 判据：Rust 单测 \`element_stroke_follows_subtitle_stroke\`（两个方向：给了用 12、没给仍走四档）
  + \`check-text-style.mjs\` / \`check-stroke-outward-parity.mjs\`。
* ⚠️ **观感如实报**：12px 描边配 80px 字号 = 字号的 15%，**超出规格的 5–8%**，字腔会变窄 ——
  这是用户口径要的效果（\`elements-spec.md\` 的值是推导值，不是用户要的）。
* 颜色仍是元素自己的近黑 \`#1a1a1a\`（字幕是 \`#403c3b\`，肉眼同色）；要完全同色得在 Rust 侧加一个
  hex 解析（本仓现在只有 JS 的 \`parseStroke\`），**没做**，等用户点。
---

## 七 · 第二件事：外圈白描边（2026-10-15 用户口径，待实现）

> 用户原话：「给字幕和文字层描边**再增加一层白色描边**」。
> 默认口径（用户没反对就照做）：**白在外、深在内**；白边 = 内圈之外**再 4px**（1080p 基准）；颜色纯白。
> **字幕与文字元素同一个字段生效**（两者共用底座的 SubtitleStyle）。

### 契约

```rust
/// 外圈描边：在 stroke_ratio 那一圈**外面**再加一圈。None = 不画（既有工程逐字节不变）。
pub struct OuterStroke { pub ratio: f32, pub color: [u8; 4] }
// SubtitleStyle 上：
#[serde(default, skip_serializing_if = Option::is_none)]
pub outer_stroke: Option<OuterStroke>,
```

### 为什么不是「把 stroke_ratio 加大一圈」

那样画出来的是一圈**同色**的粗边；用户要的是**两圈不同色**（外白内深）。
libass 一遍只能给一个 Outline 与一个 OutlineColour ⇒ 必须**多一遍栅格化 + 环带相减**。

### 实现路线（已勘定，照着做即可）

1. crates/dhampir-timeline/src/layer.rs：加 OuterStroke 结构体（带 json-schema 的 cfg_attr 派生）
   + SubtitleStyle.outer_stroke（Option）+ Default 里给 None。
2. crates/dhampir-core/src/overlay.rs：TextStyle 加 outer_stroke_px: f32 / outer_stroke_color: [u8;4]；
   evaluate_overlay 里与 stroke_px **同一处**落地（ratio × target_h；None ⇒ 0）。
   ⚠️ TextStyle 有 13 处构造点（大多在 text_overlay.rs 的测试夹具里），编译器会逐个点出来。
3. crates/dhampir-worker/src/text_raster.rs：
   * TextRasterKey 加 outer_stroke_px: u32 / outer_stroke_color: [u8;4]（**进键**：宽度变了整张位图要重画）；
   * stroke_only_args 抽出 stroke_only_args_with(key, extra_px, …)（extra_px = 0 时输出**逐字符不变**，冻结串判据不动）；
   * rasterize_with_stroke 加第三遍：宽环 = 带 outer 的那遍覆盖度 **减去** 内圈那遍覆盖度
     （新函数 ring_of(outer, inner)：逐像素饱和相减）⇒ tint(ring, outer_color) 垫底，再 overlay 上原有的 compose_stroke_and_fill；
   * rasterize_line 的分支放宽成 key.outline || key.outer_stroke_px > 0（且要求 shadow_color 为 None）
     （outline 为假时内圈那遍跳过，环带改减 fill_cov）。
4. crates/dhampir-worker/src/text_overlay.rs：三处 TextRasterKey 构造点（570 / 744 / 1222）填两个新字段。
5. 判据：① ring_of 的饱和相减（含「内圈 ⊄ 宽环」的边界）；② 画出来的像素里白环**在**深环外面（拿小位图断言）；
   ③ None 时参数串与像素**逐字节不变**（老工程零回归）。

### ⚠️ 改完必须做的两件事（这次 ③ 就是栽在这里）

1. **重打底座产物并装回程序目录**：node scripts/package.mjs --out 程序目录/dhampir --clean-wasm --no-zip
   —— 改完源码不重打 = 用户看到的还是旧 wasm（③ 的真根因就是**程序目录里那份是 10-12 的包**，而源码 10-13 就修好了）。
2. **硬刷新浏览器**：dhampir_wasm_bg.wasm 会被浏览器强缓存，不硬刷新等于没换。
---

## 八 · 验收记录（2026-10-16，真出片 + 真像素）

探针：`docs/wgpu-effect-reference/demo/clip.mp4` 的副本 + 一条文字元素（`口`，盒左上角给 x/y）；
渲染：程序目录里的新底座 `target/debug/dhampir/bin/dhampir.exe render --from 60 --to 150`（真字体、91 帧、`lines_failed: 0`）。
测量用**颜色探针**（视频里不会出现的纯色）：填充品红 `255,0,255` · 内圈描边青 `0,255,255` · **外圈绿 `4px #00ff00`**。

| 判据 | 实测 | 结论 |
|---|---|---|
| **origin 位移**：(10%,20%) → (60%,70%) | 填充 bbox 位移 = **(960, 540)**（期望 960/540） | ✅ 像素级精确 |
| 没写 x/y 的元素 | 落点 (936,102)，与写了 origin 的 (208,286) 不同 | ✅ 两条路真的分开 |
| **外圈开关** | `outer_stroke = 4px #00ff00` ⇒ 绿像素 1678；空串 ⇒ **绿像素 0** | ✅ 开了才有 |
| **层序**（第 311 行剖面） | GGGG…CCCC…MM…CCCC…GGGG | ✅ 外圈在外、内圈描边在内、填充最里 |
| 白像素增量（白环档 A/B） | 开 6354 vs 关 4631 ⇒ **+1723** | ✅ |

复现脚本留在 `.workbuddy/px/`（`measure.py` 掩码/差分、`layer.py` 层序剖面，gitignored）。
⚠️ **预览 DOM 那条腿**用 4 向 `text-shadow`（同一个配置值、同一条「内圈之外」口径），
按本仓既有纪律**不承诺逐像素**（「字形像素允许不同，落点必须同源」）。
---

## 九 · 2026-10-16 追加：**描边本身画错了**（用户「预期效果没有正常」的真根因）

用户口径：「**设置的字体颜色 + 可设置的字体描边颜色 + 一层多出来白色描边**；
白色描边是在**原来的正常字体样式**上多加的」。

### 真根因：底座的描边不是一圈边，是**字形路径填实后的整块**

底座的描边走 libass 的 `Outline=`（`text_raster.rs::stroke_only_args`），而 `coverage_from_libass`
要求「填充与描边都画成白墨」⇒ 拿到的覆盖度是 **「字形 ∪ 描边」的并集、且字形路径被填实**
（**连字腔一起填**）。后果不是「不好看」：汉字笔画间隙只有 4–8px，两侧描边一盖就是**一块实心黑板**
（实测密度 0.921，正常字形 0.363），而 `lines_drawn` / `failed` / `issues` **全都正常**。
底座自己的文档早就把这个后果写下来了（`rasterize_with_stroke` 上面那一大段），只是没人动几何。

### 修法（底座）：`描边 = dilate(字形, N) − 字形`

* 新增 `dilate_disc`（**圆盘**膨胀，稀疏实现）+ `disc_offsets`；
* `rasterize_with_stroke` **只栅格化一遍**（字形 `Outline=0`），描边与白环都由它膨胀相减：
  `白环 = dilate(字形, 内圈+外圈) − dilate(字形, 内圈)`（先垫白环、再压内圈+填充）；
* `stroke_only_args` / `stroke_only_args_with` 改成 `#[cfg(test)]`（生产路径不再走 libass 的描边，
  **一行少起一次 ffmpeg**）；
* ⚠️⚠️ 新增两个平面转换 `coverage_plane` / `plane_to_premultiplied`：`coverage_from_libass` 给的是
  **预乘 RGBA（4 字节/像素）**，而我第一版按**1 字节/像素**遍历 ⇒ 膨胀/环带全空、只剩填充色，
  **不报任何错**（铁证：`fill_nz = 776 像素 × 4 = 3104`）。这条坑写在 `coverage_plane` 的注释里。

### 判据与实测

* `描边是字形外侧的环带而不是实心块`（纯函数：字腔不进描边带、圆盘不在 45° 外扩）；
* `真起_ffmpeg_三色探针_描边与白环都要在位图里`（`#[ignore]`，需 TMP 可写）；
* 位图实拍（`.workbuddy/px4/dump-zoom2.png`）：品红填充 / 青内圈 / **绿白环**，字腔是空的；
* 真实观感（`.workbuddy/px2/on/fix-crop.png` = 内圈 12px + 白边 4px；
  `.workbuddy/px2/wide/fix-crop.png` = 白边 12px）：白字 + 干净深描边 + 外边一层白。

### 口径（两边同源）

* `[style.subtitle] stroke = "12px #403c3b"` —— **内圈**描边（宽度+颜色），字幕与文字元素都读它；
* `[style.subtitle] outer_stroke = "4px #ffffff"` —— **外圈**白边（默认 4px = 外侧 2px），
  空串 = 不画；预览 DOM 那条腿的 4 向 `text-shadow` 也**折半**（同一条口径）。

---

## 十 · 2026-10-16 文字层动效/样式逐轴审计（真渲染 + 数像素）

审计台：`.workbuddy/anim-audit/audit.py`（16 个变体 × 真出片 × 逐帧像素统计，gitignored）。
判据两条：**静X** = 与基线第 X 帧有多少像素不同（形态类档应 >0 且随时间稳定）；
**动A-B** = 同一档内两帧之间变了多少像素（动效类档应 >0）。

| 轴 | 实测 | 结论 |
|---|---|---|
| `tilt` + `layout_angle_deg` | 静90 = 27704，墨迹 bbox 变高（249→371） | ✅ 旋转生效、静态 |
| `arc`（逐字旋转） | 静90 = 26799；60→63/63→66 有逐字相位 | ✅ |
| `vtext`（竖排） | 静90 = 47102 | ✅ 真重排 |
| `pop`（弹入） | 静60→132 收敛到 1948（**墨迹 bbox 与基线逐像素相同**，最大差 59 只在边缘） | ✅ 无残留缩放；仅抗锯齿差异 |
| `fade` | 与基线逐像素相同（设计如此：它只动 opacity） | ✅ |
| `shake` | 静90 = 14806 + **动90-96 = 22256**（持续型） | ✅ |
| `perchar_big` / `perchar_small` | 静90 = 41125 / 25271，重心横移（432.7 → 474.8 / 376.3） | ✅ 逐字按序变化 |
| `heartbeat` | 有动效（静90 = 34225 → 静132 = 1583）；报告里有那条 NOTE | ✅（按文档只有单次脉冲，底座未实现双峰） |
| 颜色 | `source` 第 4 段写 `255,128,192` ⇒ 静90 = 12135 | ✅（`color` 字段本身不直接生效：由 `process` 把它烧进 `source`，这是设计） |
| **底衬（`chapter`/`comment`）** | 修前：与基线**逐像素相同**（那两档的卡片底衬在成片里不存在） | 🔧 **本轮修**（见下） |

### 本轮修的缺陷：`backdrop` 这个 bit 没人消费

`process` 把「这档要不要卡片底衬」写进 `source` 的**第 7 段**（`TextStyle::has_backdrop()`），
转译器也解析进 `st.backdrop` —— 但**没有任何地方用它** ⇒ 四档里「带底衬」的两档
（`chapter` / `comment`）与不带的长得**一模一样**，而档位区分度正是靠它（描边 / 底衬 / 字重）。

修法：元素轨的样式里补一条（写在 `motionFields` 之后 ⇒ 逐条预设 > 全局 `[elements]`）：

```js
...(st.backdrop ? { plate_color: [0, 0, 0, 140] } : {}),   // 参照：黑 55%，外扩用底座默认 12
```

验证（视频底，`.workbuddy/anim-audit/15-底衬-视频底` vs `16-无底衬-视频底`）：
**37788 像素不同**；文字区（200..700 × 250..380）平均亮度 **97.1 → 84.7**（被压暗）✔。
⚠️ 黑底探针上看不出它（黑底衬画在黑底上）—— 这也是它一直没被发现的原因之一。
---

## 十一 · 2026-10-16：`subtitle_ink_clipped` 的实测根因与底座改法（待用户拍板）

### 现象

演示工程出片时底座自己的终态报告里带 3 条 `subtitle_ink_clipped`（本仓这一侧现在会逐条报出来，
见本轮 `judge_base_done` 的修复），而日志看起来是"成功了"。

### 实测（底座自己的位图 dump，`DHAMPIR_DUMP_RASTER`）

那一帧字幕「wgpu 原生渲染效果参考」的**文字位图**：

```
raster-…888220.png   尺寸 1920x156
  墨迹 y 65..155（上留 65 / **下留 0**）· x 561..1356（左留 561 / 右留 563）
```

⇒ **不是保守告警**：墨迹真的贴到下边界（下留 0 = 被裁）。
⇒ 但墨迹只有 **90px** 高、位图有 **156px** ⇒ **装得下，问题在"没居中"**：
   居中应是「上 33 / 下 33」，实测「上 65 / 下 0」。

### 根因（在底座侧，且与它自己的契约冲突）

* 底座把一行渲成位图走的是 **`write_subtitle_file`（单条 SRT）+ `subtitles=` + `force_style`**
  （`dhampir-worker/src/text_raster.rs`）。SRT **不带样式** ⇒ libass 用**默认样式**，
  而默认样式是 `Alignment=2`（**贴底居中**）+ 默认 `MarginV` ⇒
  **文字被画在位图底部**，于是墨迹顶到下边界。
* 而底座的绘制契约是「**位图的中心对准行盒的中心**」（`place_line`），
  这条契约**要求"栅格化时文字在位图里居中"** —— 默认样式并不满足它。
* `force_style` 目前只写 5 个键（`FontName / FontSize / PrimaryColour / Outline / OutlineColour`），
  **不含 `Alignment`** ⇒ 默认值一路带到底。

### 建议改法（底座侧，最小改动）

1. 在生成 `force_style` 的两处**生产**构造里补 `Alignment=5`（中-中）与 `MarginL/R/V=0`：
   `drawtext_args`（阴影/无描边那条，约 598 行）与 `fill_only_args`（墨迹那条，约 1265 行）；
   `stroke_only_args_with` 现在是 `#[cfg(test)]`（描边几何已改成膨胀带），不必跟。
2. 同步更新两条**冻结串**判据（`frozen_argv` 副本 + `不画阴影时参数串逐字符与冻结串相同`）
   —— 冻结对象**故意换一次**，理由写进注释（与 T2 那次"冻结对象变了"同样的处理）。
3. 文档：把那句"只写五个键"改成"六个"，并写明**为什么必须有 `Alignment=5`**：
   少了它，`place_line` 的"位图中心 = 行盒中心"不成立，长行/大字号会**贴底被裁**。

### 验收（不用肉眼看）

* 重跑演示工程 ⇒ 底座终态报告里的 `subtitle_ink_clipped` **应当消失**（它自己就是判据）；
* 再 dump 一次那张位图 ⇒ 墨迹上下留白**应当接近**（90px 墨迹 / 156px 位图 ⇒ 约 33 / 33）；
* 全库底座测试（`cargo test` in `Dhampir`）除既有那条环境失败外应保持绿。

### 本仓侧的对应关系

* 本仓**无法**在翻译层修掉它：位图的尺寸与绘制对齐都在底座（我们只给字号/行高/字体）；
* 本仓已经把"底座报告失败"这件事从"看不见"变成"逐条 warn"（本轮修复），
  所以这条 issue 以后不会再溜过去。
### 状态：**已修**（2026-10-16，同一次会话内做完并自证）

底座侧改动（`crates/dhampir-worker/src/text_raster.rs`，共 4 处）：

1. 新增常量 `CENTER_IN_BITMAP = ",Alignment=10,MarginL=0,MarginR=0,MarginV=0"`（doc 里带实测数字）；
2. `drawtext_args`（无描边那条）与 **`fill_only_args`（开了描边时的字形那一遍，生产路径）**
   各带上它；`stroke_only_args_with`（`#[cfg(test)]`，只在判据里）**故意不跟**（留作历史形状）；
3. 冻结串判据更新（第三次有意换冻结点），doc 里写明代价；
4. 复跑 `cargo test -p dhampir-worker --lib` ⇒ **158 通过 / 0 失败**。

#### ⚠️ 一个必须记住的坑：**是 `10`，不是 `5`**

第一版按 ASS 的"小键盘编号"写 `Alignment=5`（= 中-中），结果**墨迹跑到了左上角** ——
因为我们的输入是 **SRT**，libass 转出来的是 **SSA v4 旧编号**脚本：那一套里 `5` = 左上、
`10` = 中-中。三次实测（同一份工程、同一个 dump 开关）：

| 写法 | 墨迹 | 结论 |
|---|---|---|
| 不写 `Alignment` | y 65..155（**下留 0**）· x 561..1356 | 贴底、下边缘被裁（原缺陷） |
| `Alignment=5` | y 5..97 · x **0**..787 | **左上角** ⇒ 说明走的是旧编号 |
| `Alignment=10` | y **38..130**（上 38 / 下 25）· x 561..1356 | 居中 ✔ 不再贴边 |

#### 验收（两处，都不用肉眼）

* **底座自己的终态报告**：同一份工程 ⇒ `subtitle_ink_clipped` **消失**、**进程退出码 1 → 0**；
* **本仓 CLI 全流程**：演示工程出片日志里不再出现任何底座 issue，干净 `FINISHED`
  （`judge_base_done` 那条 warn 是打印它们的地方，没有出现才说明真的没有）。

#### 影响面（如实记）

这是**所有文字像素都变**的一次改动（位图内文字上移/居中），且**已换进本仓打包目录**：
`target/debug/dhampir/bin/dhampir.exe`（`VERSION` 未动 —— host API 没变）。
要出发布包时按常规跑 `node scripts/package.mjs …` 重新打包即可。
#### 顺带修好的第二条：成片字幕**比自己的模型低 32px**

同一帧（演示工程、第 90 帧）里量字幕墨迹的**垂直中心**：

| | 墨迹 y | 墨迹中心 | 契约的字幕带中心（Canvas `cy=960`） |
|---|---|---|---|
| 修前 | 947..1037 | **992.0** | 960 ⇒ **低 32px** |
| 修后 | 920..1011 | **965.5** | 960 ⇒ 差 5.5px（墨迹自身的上下不对称） |

横向一直是 958.5（画面中心 960）✔。

⚠️ 这条的含义不只是"好看"：**贴纸落点求解器与字幕带避让都是按 960 算的**
（`placement::caption_band`），而底片实际画在 992 ⇒ "贴纸压到字幕上"这一类
在成片里本来就该发生。居中修好之后，**模型与成片对上了**。
### 附：颜色预设全链路实测（2026-10-16，补 ① 唯一没验透的一条）

第 2 轮的审计里"颜色"那一档走的是**手写 `source`**（把 RGBA 直接写进第 4 段），
那段验的是"填充色能进画面"，**没有**验"面板/LLM 只写 `color` ⇒ `process` 烘焙"这一段。补上：

1. 探针：`source = ""` + `color = "pink"`（正是面板/LLM 的产物形态），`processed = false`；
2. 跑 `polish process` ⇒ 写回：
   `source = "hook|80|12|255,128,192,255|26,26,26,255|900|0|6|1|pink"`
   —— 第 4 段 **255,128,192** = `ElementColor::Pink.rgba()`，第 10 段记了档名 ✔；
3. 真出片一帧，按色分类数字形像素：
   `非黑 28314 | 粉(填充) 6319 | 白(外环) 1759 | 深色(描边) 13261`，
   粉色像素平均 **(229.7, 142.2, 184.3)**（期望填充 255,128,192，均值被抗锯齿与视频编码拉低）✔。