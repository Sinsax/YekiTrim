# 元素文字效果类别 · HTML 预览

`text-effects-preview.html` —— 39 档文字动效的**可视化对照**，单文件、双击即开。

## 它凭什么说"和实际一致"

页面的求值**不是参考实现**，是 **Dhampir 权威 Rust 的逐值移植**：

| 能力 | 权威来源 |
|---|---|
| 逐字（C/D 组 20 档） | `crates/dhampir-timeline/src/text_perchar_anim.rs` · `char_transform_at` |
| 抖动 | `crates/dhampir-timeline/src/text_motion.rs` · `text_shake_px` |
| 弹入 | 同上 · `back_out3`（S=3.0） |
| 缓动 `back_out` | `crates/dhampir-timeline/src/easing.rs` · `EasingForm::apply`（S=1.70158） |
| 摇摆 | `tools/polish-to-dhampir.mjs` · `swayKeyframes`（展开成 `rotation` 关键帧） |
| 39 档参数 | `webui/src/polish/engine.js` · `ANIM_PRESETS` |

⚠️ **对拍过**：`hop_std` 逐字位移、`slam_squash` 弹入曲线、`shake_std` 的
`(dx,dy)` 三点与 Rust 输出**逐值相同**（`cargo run --example` 实测，见提交说明）。

## 页面的三个开关

- **暂停 / 播放** —— 定格看某一帧；
- **慢速 ×0.35** —— 弹入只有 160~500ms，常速看不清；
- **循环 / 播完停** —— 「播完停」能看见"演完之后不再动"那一段（**终止值**，
  不是回弹到恒等 —— 这是底座明确的行为，见 `char_transform_at` 的边界 ③）。

另外支持 `?t=毫秒` 强制定格（截图与逐值核对用）：
`text-effects-preview.html?t=300`

## 两轴归属

每张卡的右上角标写着它该填哪一栏：

- <span>只填「样式」栏</span> —— `style_anim`
- <span>只填「动效」栏</span> —— `effect_anim`
- <span>两栏都要填</span> —— `both` 的 5 个种类（`slam` / `pop` / `arc` /
  `scatter` / `hop`），**只填一栏只生效一半**。

⚠️ 页面上每条演示的是**这一档单独作用**。实际是两轴叠加的
（如「竖排 + 抖动」），叠加规则见 `webui/src/polish/AGENTS.md`。

## 重新生成

```bash
node docs/anim/gen-preview.mjs ./docs/anim/text-effects-preview.html
```

改了 `ANIM_PRESETS`（新增档位 / 调参数）之后要重新生成 ——
`anim-kinds.json` 是从 `engine.js` 导出的快照。

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
