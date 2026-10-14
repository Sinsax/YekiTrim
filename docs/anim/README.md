# 元素文字效果类别 · HTML 预览

`text-effects-preview.html` —— **9 档 × 三轴**的可视化对照，单文件、双击即开。

> ⚠⚠ **2026-10-14（三轴收敛）：原来是 39 档。**
> 收敛后档位只剩 9 档，且多了**过渡**这第三条轴（原来只有样式/动效两轴）。
> 本文件、`anim-kinds.json`、页面分组已全部跟到新形状。

## 它凭什么说"和实际一致"

页面的求值**不是参考实现**，是 **Dhampir 权威 Rust 的逐值移植**：

| 能力 | 权威来源 |
|---|---|
| 逐字（当前 3 档） | `crates/dhampir-timeline/src/text_perchar_anim.rs` · `char_transform_at` |
| 抖动 | `crates/dhampir-timeline/src/text_motion.rs` · `text_shake_px` |
| 弹入 | 同上 · `back_out3`（S=3.0） |
| 缓动 `back_out` | `crates/dhampir-timeline/src/easing.rs` · `EasingForm::apply`（S=1.70158） |
| 摇摆 | `tools/polish-to-dhampir.mjs` · `swayKeyframes`（展开成 `rotation` 关键帧） |
| 9 档 × 三轴的参数 | `webui/src/polish/engine.js` · `ANIM_PRESETS`（**权威**）|

⚠️ **对拍过**（当时的旧档名；收敛后对应 `perchar_*` / `pop` / `shake`）：
逐字位移、弹入曲线、抖动的 `(dx,dy)` 三点与 Rust 输出**逐值相同**
（`cargo run --example` 实测，见当时的提交说明）。**数学本身没变**，只是档名收敛了。

## 页面的三个开关

- **暂停 / 播放** —— 定格看某一帧；
- **慢速 ×0.35** —— 弹入只有 160~500ms，常速看不清；
- **循环 / 播完停** —— 「播完停」能看见"演完之后不再动"那一段（**终止值**，
  不是回弹到恒等 —— 这是底座明确的行为，见 `char_transform_at` 的边界 ③）。

另外支持 `?t=毫秒` 强制定格（截图与逐值核对用）：
`text-effects-preview.html?t=300`

## 三轴归属

每张卡的右上角标写着它该填哪一栏：

- <span>只填「排版」栏</span> —— `style_anim`（`tilt` / `arc` / `vtext`）
- <span>只填「过渡」栏</span> —— `transition_anim`（`pop` / `fade`）
- <span>只填「动效」栏</span> —— `effect_anim`（`shake` / `perchar_big` /
  `perchar_small` / `heartbeat`）

### ⚠️ `arc` **不是**真弧（2026-10-14 核实）

界面上 `arc` 那一档的标签是 **「斜扫（非真弧）」**，不是「弧形」。实测稳态
（`t = 3.0`，入场早已结束）各字角度：

| 字 | 一 | 段 | 弧 | 线 | 测 | 试 | 文 | 字 |
|---|---|---|---|---|---|---|---|---|
| **现状 `arc`** | -14° | -14° | -14° | -14° | -14° | -14° | -14° | -14° |
| **真弧该是** | -14° | -10° | -6° | -2° | +2° | +6° | +10° | +14° |

⇒ 现状**每个字都是同一个角度** = "整行统一倾斜 14°"，
**与 `tilt` 档在观感上分不出来**；它实际做到的只是**入场时逐字扫过**。

⚠️ 这是**原语缺口、不是漂移**：底座 `text_perchar_anim.rs` 的
`rotation_deg = mix(rotation_from, rotation_to)`**没有 `rank` 项**，
而位移那两项（`spread_*`）是**线性混合 × rank** ⇒ 逐字位移只能单调（斜排），
做不出抛物线的弧。**两边一致地做不出**。
需求与 7 条验收判据见 `docs/plan/dhampir-rotation-spread-request.md`。

⚠⚠ **一档只属一条轴** —— 收敛后**已经没有 `both`**，
三条轴的档名集合**两两不相交**（判据 `scripts/check-anim-attribution.mjs`）。
⇒ 下拉里只会看到**本轴自己**的档，不会出现同一个名字在两栏里。

⚠️ 页面上每条演示的是**这一档单独作用**。实际是两轴叠加的
（如「竖排 + 抖动」），叠加规则见 `webui/src/polish/AGENTS.md`。

## 重新生成

```bash
node docs/anim/gen-kinds.mjs                                    # 先：从真源码抽快照
node docs/anim/gen-preview.mjs docs/anim/text-effects-preview.html  # 后：生成页面
```

改了 `ANIM_KINDS`（**新增/删除档位、改轴归属**）或 `ANIM_PRESETS`（调参数）
之后要重跑**两个**。

> ⚠⚠ **为什么现在有 `gen-kinds.mjs`：这份快照原来是手抄的。**
> 三轴收敛把 39 档收到 9 档、把 `both` 整类消灭，
> **而快照没人改** ⇒ 页面一直画着已经不存在的 39 档，
> 而且**没有任何判据会红**（它只是个文档）。
> 同一次检查还发现：页面上“逐字那一路”**一直是空跑的**
> （快照里的 `preset` 从来就没有 `perChar`）—— 现在从 `ANIM_PRESETS` 读，
> 生成时会打印 `逐字路径: 3 | 整块路径: 6`，**一眼能看出它没空跑**。

⚠ 两个脚本的路径都**相对脚本自己**（不是 CWD）——
原来是 `'./anim-kinds.json'`，从仓库根跑就 `ENOENT` ⇒ **没人跑得动的脚本**
自然也没人用它重生成。

## 预览字号：只有**一个**来源

字号定义在 `:root` 的一条 CSS 变量上：

```css
:root{ --txt-fs:23px; --stage-h:132px }
.txt{ font-size:var(--txt-fs) }
```

脚本**不写第二份字面量**，而是量这条变量：

```js
const FONT_PX = parseFloat(getComputedStyle(document.documentElement)
  .getPropertyValue('--txt-fs')) || 23;
```

⚠️ 为什么较真这一件事：逐字的 `offset_*_em` 是 **em 单位**，要乘字号才是像素。
所以字号一旦有**两份**，改了一处忘另一处 ⇒ 逐字位移按错的字号换算，
**字散得过大或过小，而页面上不报任何错** —— 看着只像"这档参数就这样"。
（这正是本仓反复栽的那类"配了、不报错、效果不对"。）

页面顶部有「预览字号」滑杆：它改的就是**那一条变量**，
所以样式与逐字换算**同时**跟着变，不可能不同步。
舞台高度也走 `--stage-h`，随字号放大，**竖排那三张不会被裁**。
