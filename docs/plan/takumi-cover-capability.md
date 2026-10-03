# Takumi 能不能画我们的封面：能力判定（2026-09-30 起）

> 实验台：`.workbuddy/takumi-spike/`（crate `spike` 两个 bin：`spike`=整张封面、`points`=单点判定）
> 版本：`takumi 2.14.0`（MIT OR Apache-2.0，MSRV 1.91 / edition 2024）+ `takumi-core 0.25` / `takumi-html 0.3.4` / `takumi-raster 0.5.1`
> 复跑：`python make_assets.py` → `cargo build --release --bin points` → `points.exe ../assets.txt ../points2 msyh.ttc seguiemj.ttf arial.ttf` → `python sheet.py`

## 0. 一句话

**CSS 文本路径是通的**（id/class/后代选择器、四种渐变、`filter:blur`、`backdrop-filter`+遮罩、`text-shadow`、
字距/行距/对齐、`object-fit`、`opacity` 全部生效），所以上一轮整图不符**不是"CSS 没落地"**；
已确认的硬差异只有两处（下面 §2），其中一处（`paint-order` 无效 + 两种 stroke 前缀语义不同）
正好命中封面标题最核心的构造。

## 1. 单点判定表（`.workbuddy/takumi-spike/points-sheet.png`）

| # | 封面构造 | 结论 | 证据 |
|---|---|---|---|
| 01 | id 选择器 + `color`/`font-size`/`font-weight` | ✅ | 黄字 40px 粗体 |
| 02 | **class 选择器**（`.hl` 逐词高亮） | ✅ | 「游戏」黄、其余白 |
| 03 | **后代选择器**（`#title .t`） | ✅ | 绿字 |
| 04 | `radial-gradient`（暗角/光晕） | ✅ | 径向压暗 |
| 05 | `repeating-linear-gradient`（扫描线） | ✅ | 条纹 |
| 06 | `linear-gradient`（滤镜层） | ✅ | 左粉右透明 |
| 07 | `filter: blur(4px)`（背景模糊） | ✅ | 棋盘糊化 |
| 08 | **`backdrop-filter: blur(6px)`**（模糊特效） | ✅ | 只有那一块被糊 |
| 09 | `mix-blend-mode: overlay` + `isolation` | ⚠️ 画了，但**观感待定**（见 §3） | 半透明粉叠棕底 → 橙红 |
| 10 | **文字描边 + `paint-order`** | ❌ **见 §2** | 糊块 |
| 11 | `text-shadow: 0 6px 18px` | ✅ | 阴影可见 |
| 12 | `letter-spacing` + `line-height` + `text-align` | ✅ | 字距与居中 |
| 13 | `object-fit: contain` | ✅ | 竖图居中、左右留边 |
| 14 | `opacity: .45` | ✅ | 半透明块 |

## 2. 已确认的硬差异：文字描边（封面标题的核心）

五个变体并排（`.workbuddy/takumi-spike/stroke-compare.png`）：

| 写法 | 结果 |
|---|---|
| `-webkit-text-stroke:8px` + `paint-order:stroke fill` | ❌ 糊块（填充被描边盖住） |
| `-webkit-text-stroke:8px`（去掉 `paint-order`） | ❌ **与上一条逐像素同貌** ⇒ **`paint-order` 不生效** |
| `-webkit-text-stroke:2px` + `paint-order` | ✅ 正常（细描边看不出来） |
| `color:transparent` + `3px` 描边 | ✅ 只有描边，说明描边本身能画 |
| **`text-stroke:8px`（无 `-webkit-` 前缀）** | ✅ **正常**：奶油填充 + 深色描边 |

**推论**：两个前缀的**宽度语义不同**（`-webkit-text-stroke` 表现得像"整宽向内"、`text-stroke` 像"居中"），
而封面模板用的是 `-webkit-text-stroke: var(--sk)`（0~60px 可调）。

**可选的解法（待定标）**：
1. 改用 `text-stroke`（需与 Chrome 逐像素对拍定标宽度）；
2. 或把宽度按语义折半；
3. 或绕开 `paint-order`：**把标题画两遍**（底层 = `color:transparent` + 描边，上层 = 填充），
   纯 CSS 可实现、不改模板语义、两份实现共享同一份 HTML。

## 3. 还没验的（下一批单点）

1. **`mix-blend-mode: overlay` 的观感**：单点 09 出了橙红，但**没有与 Chrome 同例对比**，
   说不清是"overlay 正确"还是"退化成普通 alpha"。→ 补一张 Chrome 同例。
2. **多层绝对定位 + z-index 叠加**：上一轮整图里"暗角/扫描线/颜色覆盖"三层没出现，
   而单点里它们都работ。怀疑点：`inset:0` 的空 div 在**大容器**里的尺寸、或层级被上层的大描边盖住。→ 补一个
   "三层 `inset:0` + z-index 1/2/3"的单点。
3. **中文度量/断行**：整图那轮报 `ICU4X data error: No segmentation model for complex script: Chinese/Japanese`。
   单点里中文能画、能对齐，但**断行/度量是否可靠**要专门验（封面标题是 `nowrap` + 手动 `<br>`）。
4. **`width:max-content`**：样式表解析阶段直接报错（`Unexpected token: max-content`），
   而参考表把它列为合法值、封面模板正好用了它。→ 补等价写法（`display:inline-block` 等）。
5. **`text-align:center` + `width:auto` 与 Chrome 的度量差**（影响 `translate(-50%)` 的落点）。

## 4. 性能（release 实测，供接入时决策）

| 路线 | 单张 | 占用 |
|---|---|---|
| Chrome（现状） | 3.18s（含我们自己的 1.5s `setTimeout`） | 11 进程 / 峰值 520MB |
| **Takumi（release）** | `from_html` 1.0ms + `render` **249ms**；端到端 887ms（含读 23MB 字体 + 两次编码写盘） | **单进程 / 峰值 54MB**；exe 9.3MB |
| 客户端 foreignObject | 140ms（草稿语义，服务端 0） | 服务端 0 |

> 单张 400×200 的单点图：1–4ms。字体上下文复用 + CJK 子集是把 887ms 压到 ~250ms 的两个手段。


---

## 5. 第二轮：整图分段定位 + 适配规则（已验证）

### 5.1 分段定位（同一份 HTML，两侧 1920×1080 各出一张）

| 段 | 含义 | 平均差 | >8 | 结论 |
|---|---|---|---|---|
| v1 | 图片层（`object-fit:cover/contain` + `filter:blur(6px)` + `scale(1.04)`） | **0.01** | 0.00% | ✅ 逐像素一致 |
| v2a | + `linear-gradient` + 两个 `radial-gradient` | **0.49** | 0.00% | ✅ |
| **v2b** | **+ `repeating-linear-gradient`（扫描线）** | **12.57** | **50.00%** | ❌ **就是它** |
| v2 | 四层齐（= v2a + v2b） | 13.85 | 50.00% | ❌ |
| v3 | + `backdrop-filter`+mask、`mix-blend-mode:overlay` | 11.24 | 40.82% | （扫描线的差） |
| v4 | 全量（+标题/表情） | 13.27 | 41.08% | （扫描线 + 文字 AA） |

**修掉一处（扫描线方向）后的整图**：

| | 改前 | 改后 |
|---|---|---|
| v2（图片 + 四渐变） | 13.85 | **0.52**（>8 0.03%） |
| v4（全量） | 13.27 | **3.72**（>8 3.12%、>32 2.36%、>64 1.57%） |

残差 3.72 全部落在**标题与表情边缘**（描边/字形抗锯齿）—— 即"两个栅格化器不同"那一档。

### 5.2 扫描线标定（六种写法 vs Chrome，400×200 单点）

| 写法 | 平均差 | 判定 |
|---|---|---|
| `repeating-linear-gradient(0deg, …)`（**我们模板现在用的**） | 30.50（50% >32） | ❌ |
| `repeating-linear-gradient(180deg, …)` | **0.04** | ✅ |
| `repeating-linear-gradient(to bottom, …)` | **0.04** | ✅ |
| `0.5px 1.5px / 1.5px 4.5px`（停靠点整体偏移半像素） | **0.00** | ✅✅（更准，但语义上是"魔法偏移"） |
| `0deg` + `background-position:0 0.5px` | 30.50 | ❌ 无效 |
| `linear-gradient` + `background-size:100% 4px` 平铺 | 15.25 | ❌ |

⇒ **Takumi 的 `0deg` 方向语义与 CSS 相反**（等价于 Chrome 的 `180deg`）。

### 5.3 适配规则（三条，均已被整图数字验证）

1. **`repeating-linear-gradient` 的角度写反**：模板里的 `0deg` 要写成 `180deg`（或 `to bottom`）。
   —— 单这一条就把整图从 13.85 拉到 0.52。
2. **绝不写 `paint-order`**：写了它描边会盖住填充（8px/56px 就是一片黑）。删掉即可；
   `-webkit-text-stroke:Npx C` 本身 Takumi 认得对（0.47/255 ≙ Chrome）。
3. **`width:max-content` 在样式表解析阶段直接报错**（`Unexpected token: max-content, expected a value of <length>`），
   而封面模板用了它 → 换 `display:inline-block` + `white-space:nowrap`（等价，差 2.72 属文字 AA 档）。

另有一条**接入前提**（不是规则）：`from_html` 会丢掉 `<style>` 块 ⇒ CSS 必须自己抠出来走
`StyleSheet::parse` + `RenderOptions::stylesheet`（本轮实验台就是这么做的，全程可用）。

### 5.4 复跑

```bash
cd .workbuddy/takumi-spike
python make_layers.py                      # 生成 cover_v1/v2/v2a/v2b/v3/v4.html
spike/target/release/spike.exe spike/cover_v4.html fix-v4 msyh.ttc seguiemj.ttf arial.ttf
# Chrome 侧：同一份 html --headless --screenshot --window-size=1920,1080
python fixdiff.py                          # 逐段给平均差 / >8 / >32 / >64
python make_assets.py && points.exe ..\assets.txt ..\points5 <字体...>   # 26 个单点
```

> 字体：实验台用系统 `msyh.ttc`（中文）+ `seguiemj.ttf`（彩色 emoji）+ `arial.ttf`。
> 接入本仓时要改成"库内字体 + 显式注册"（Takumi 不读系统字体），并**复用同一个 `Fonts` 上下文**共享解码缓存。
>
> ⚠️⚠️ **2026-10-02 补的限定：实验台那支 `seguiemj.ttf` 实测是 `COLR version 0`，**
> **而 Takumi 的彩色字形解析写死只问 COLRv0**（`takumi-core-0.25.0/src/resources/glyph.rs:440-443`
> 的 `ColorGlyphFormat::ColrV0`；顺序是「位图 strikes → COLRv0 → 普通 `glyf` 描边」）。
> 所以本节所有"
> **"emoji 逐像素级通过"的结论，只对 COLRv0（或 CBDT/sbix 位图）字体成立** ——
> 对 **COLRv1** 与 **SVG-in-font** 都**不成立**，而它们 Takumi 一个都不支持。
> 实测（`.workbuddy/emoji-cover-test/`，三组对照留了产物）：库里放一支 COLRv1 的 Noto Color Emoji 时，
> 封面上的 emoji **整片消失**（与"一个 emoji 字体都没有"的产物**逐字节相同**），
> 且**不报错、界面照样显示已装**。⇒ 自带 emoji 字体已换成 CBDT 版（见 §11.3 与 font-library.md）。


---

## 6. 第三轮：拿**真实模板产物**验证（`rich-new.html`）

产物：`.workbuddy/cover-layer/rich-new.html`（零回归对拍时从预览服务抓下来的真封面 HTML，
3 个 `<style>` 块 / 2 处 `paint-order` / 2 处 `repeating-linear-gradient(0deg)` / 图片走 `_cover_frame.jpg`）。
适配脚本 `adapt_rich.py`（内联图片 + 出片模式 + 三条规则）；两侧同一份文件：
Takumi `render 241.5ms`，Chrome 截图 970 KB。

**当前结果：平均差 38.43（>8 75.12%、>32 31.20%）**，肉眼定位：**Takumi 侧标题整块不可见**。

### 6.1 新发现的第 4 条适配规则：入场动画（主因）

真模板的标题 CSS：

```css
#title { …; -webkit-text-stroke:var(--sk) #2a2522; text-shadow:var(--text-shadow);
         opacity:0; transform:translateY(24px); animation:fadeUp 0.6s 0.3s ease both; }
```

- **Chrome 侧**：截图前 `__COVER_READY` 等了 1.5s，动画（0.3s 延迟 + 0.6s）早跑完 → 标题可见；
- **Takumi 侧**：它渲染的是**静态帧**，默认动画时间 = 0 → `both` 填充模式让元素停在 **`opacity:0`** → 标题不可见。

⇒ 接入时必须二选一：
1. **给 Takumi 设动画时间**（`RenderOptions` 里有"全局动画时间(ms)"这一项）到动画结束之后（≈ 1000ms）；
2. 或者**在出片模式下把入场动画关掉**（`html.export` 那条分支里 `animation:none; opacity:1`）——
   这条更稳：出片本来就只截一帧，让模板在出片模式下**不依赖时间**才是正解。

### 6.2 抽 `<style>` 的坑（决定接入形态）

真模板里有一句注释**提到了 `<style>` 三个字**，naive 抽取会把注释当成 CSS →
`StyleSheetParseError { kind: DanglingCombinator }`（本仓模板第 68 行那句注释就是）。

⇒ **接入的正解不是"从 HTML 里抠 CSS"，而是让 `build_cover_page` 直接回传它注入的 CSS**
（模板 CSS + `cover_layer_css` 两个来源）。实验台里我打了补丁（先剥 HTML 注释）只是为了让验证能继续。

### 6.3 还没定的两项（下一轮）

1. `#bg` 的模糊口径：真模板的背景是 `bg_mode=blur` 那条（28px 基准 × `blur_scale` + `scale(1+px/150)`），
   而分数段的 v1 用的是我手写的 6px（那一段是 0.01 逐像素一致）。**28px 档还没量**。
2. 真模板标题里的 `var(--sk)` / `var(--fs)` 等变量是否被 Takumi 解析到位（需要把动画那条先解决再量）。


---

## 7. 第四轮：真实封面跑通（**38.44 → 3.03**）——五条适配规则

### 7.1 主因：模板把布局变量交给**运行时 JS**

真模板（`rich-new.html` / `templates/cover.html`）里有 19~20 个变量**不是**写在 CSS 里，而是页面 JS 注入：

```js
document.documentElement.style.setProperty('--fs','120px');  ('--sk','20px');  ('--bg-blur','39px');
('--bg-blur-scale','1.261');  ('--grad-bg','linear-gradient(…)');  ('--vignette-op','0.12');
('--tw','62%');  ('--text-x','35%'); ('--text-y','52%'); ('--align-items',…); ('--text-shadow',…); ('--hlc','#ffcc00')
```

CSS 里 **20 处 `var(--…)`** 全靠它们。**Takumi 不执行 JS** ⇒ 变量全未定义 ⇒ 依赖它们的声明**整条被丢**：
标题没字号/没描边、背景没模糊没亮度、渐变底与暗角丢失、标题盒位置也不对。

⇒ **正解：`build_cover_page` 在服务端把变量展开成 `:root{…}`**（与现在的 `__FONT_FAMILY__` 一样是字符串替换，
两条通道都受益；对 Chrome 那条是**等价**的 —— 变量值一模一样，所以不会动现有 SHA 基线）。

### 7.2 五条适配规则（全部实测验证）

| # | 规则 | 依据 | 改前 → 改后 |
|---|---|---|---|
| 1 | **变量服务端展开**（19 个 `setProperty` → `:root{}`） | Takumi 不跑 JS | 38.44 → 主因 |
| 2 | **出片模式关入场动画**（`animation:none` / `opacity:1` / `transform:none`，**三条独立规则**） | 静态帧默认动画时间 0，`both` 会停在 `opacity:0`；⚠️ 不可解析的值会**丢掉整条声明块**，所以不能合在一块写 | — |
| 3 | **不写 `paint-order`**（2 处） | 写了描边会盖住填充（8px/56px 就是黑块） | 18.24 → 0.47 |
| 4 | **`repeating-linear-gradient` 角度反向**（2 处 `0deg` → `180deg`） | Takumi 的角度语义与 CSS 相反 | 12.57 → 0.04 |
| 5 | **CSS 由 `build_cover_page` 回传**，别从 HTML 里抠 | 模板注释里**提到了 `<style>`**，naive 抽取会把它当样式表（`DanglingCombinator`） | — |

### 7.3 最终数字（真实封面 `rich-new.html`，1920×1080，两侧同一份 HTML）

| 阶段 | 平均差 | >8 | >32 | >64 |
|---|---|---|---|---|
| 只做 3/4/5（语法适配） | 38.44 | 75.12% | 31.21% | 24.55% |
| **+ 规则 1、2（变量展开 + 关动画）** | **3.03** | **2.77%** | 2.07% | 1.62% |

残差 3.03 全部落在**标题与表情的字形边缘**（右图 diff×4 只有字形描边和表情圆环），
即"两个栅格化器不同"那一档 —— 与手写复刻的 3.72 同档。**其余区域逐像素一致。**

对照：客户端 foreignObject 截图那条是 0.43/255（同为浏览器渲染）；Dhampir 那条（附录 A）23 项里 ❌ 11 项。

### 7.4 顺带澄清一个早先的误判

单点 `07_blur_img`（`filter:blur(4px)` 直接作用在**流内**的棋盘 img 上）差 40.77，
而封面里 `#bg img{filter:blur(…)}` 在 `overflow:hidden` 的盒子里差 **0.01**。
⇒ 那 40.77 是**画布边界处的模糊取样**差（Chrome 取盒外透明、Takumi 取夹取），
在封面里因为 `overflow:hidden` 被裁掉，**与产品路径无关**。**`filter:blur` 口径本身是一致的。**

---

## 8. 第五轮：E2E（真工程走完整条产品链路）

工程 `E:\media\[...]\output\clips\clip-25`（341MB 真片段）；驱动 `.workbuddy/cover-e2e/run.mjs`
（node：递归硬链接副本、ffmpeg 出差异面、超阈值 exit 1）。

| 轮次 | chrome | takumi | 对拍 |
|---|---|---|---|
| 1 | 3744ms / 290KB | **失败** `AtRuleInvalid("font-face")` | — |
| 2 | 3744ms / 290KB | 1503ms / 77KB | **146.44/255**（图片层整块缺失） |
| 3 | 3641ms / 290KB | 3454ms / 261KB | 19.57/255 · >32 11.00% |

E2E 逐轮抓出**四个单点测试看不到的真问题**：

1. **`@font-face` 不支持** —— 项目选了库内字体时模板会注入它，Takumi 的样式表解析器直接报
   `AtRuleInvalid("font-face")`。适配层要**整块删掉**（它靠 `Fonts::register` 注册字体）。
2. **图片必须内联成 data URI** —— Takumi 没有资源解析器，`src="_cover_frame.jpg"` 一张都拿不到
   （`RenderOptions.images` 是 `pub(crate)`，外部塞不进去）。补上后 146.44 → 19.57。
3. **产物必须 JPEG q85** —— 桥最早写的是 PNG 却命名为 `cover.jpg`（格式与扩展名不符）。
4. ⚠️ **`paint-order` 被完全忽略**（**这是现在剩下的全部差异**）：Takumi 永远把描边画在填充之上，
   而模板给 Chrome 写的是 `paint-order:stroke fill`（填充在上）。**20px 描边 / 120px 字** →
   takumi 侧标题糊成黑块、平均差 19.57/255、>32 占 11%。
   单点测试抓不到它：那里是 **8px 细描边**且参照侧也没有 `paint-order`（两边同样糊 → 0.47）。
   候选解法：适配层让标题**画两遍**（底层 `color:transparent` + 描边、上层纯填充）——
   单点 `21_dual_stroke` 实测 10.46，偏高，下一轮要标定（间距/AA）。

---

## 9. 第六轮：复合选择器（已确认的引擎缺陷）

最小单点 `.workbuddy/cover-e2e/selector-probe.mjs`（200×200，只测这一件事）：

| 变体 | Takumi 均值 | Chrome 均值 |
|---|---|---|
| A `#a.k img`（带 class） | 2.45 | 127.11 |
| B 无 class（对照） | 0.44 | 23.00 |
| **A−B** | **2.01 ≈ 没出现** | **104.11 = 出现了** |

⇒ **Takumi 2.14 不匹配 `#id.class` 复合选择器**。真封面的背景层可见性正是靠它
（`#bg img{display:none}` / `#bg.bg-blur img{display:block}`）⇒ **`bg_mode=blur|image` 的工程
背景层整块不画**。已修进适配层：`flatten_compound_selectors(css, html)`（只扁平化 HTML 里
真出现的 class），配钉子。

⚠️ **同时更正一处误用**：`blur-probe.mjs` 里"改 `--bg-blur` 七张输出逐字节相同（25.24）"
是我**人为强制** `class="bg-blur"` 造出来的场景，**不是产品证据** —— 它只证明了
"背景层没显示时模糊半径当然不改变输出"。这条数据不可当结论。

## 10. 第七轮：把 19.57 拆开（几何 + 区域分解）

`.workbuddy/cover-e2e/geometry-probe.mjs`（ffmpeg 各读一次 raw gray，之后所有位移在 node 内存里算）：

**位移搜索**（dx/dy ∈ [−12,12]，步 2）：最小值就在 **(dx,dy)=(0,0) → 19.57**，任何平移都更差
⇒ **没有几何差**（`--bg-transform` 的 translate/scale、`object-fit` 的几何都对上了）。

**区域分解**（4×4 网格平均差，每格 480×270）：

```
y0    0.8   0.6   0.9   0.8
y1   19.5  43.2  38.2  46.6
y2   21.3  40.9  47.4  49.0
y3    0.7   0.8   0.8   1.6
```

⇒ **背景 / 帧层 / 渐变 / 滤镜 / 图层栈：全部 ≤1.6/255（基本逐像素）**。
**19.57 全部落在标题所在的那两行**（`text_y=52%`，两行 120px 字）。

⚠️ **更正第 7 轮的读法**：那一轮"完全去掉描边只从 19.57 → 15.73"被我说成"描边不是主因"——
口径错了。区域分解显示**非标题区就是 ~1/255**，所以那 15.73 也是标题的（去掉描边等于把
"描边画错"换成"没有描边"，两边都不对，差值只有 4）。正确的结论是：
**整张封面只有标题这一块的渲染与 Chrome 不同**，其余逐像素级一致。

⇒ 收口只剩一件事：**标题**（描边画序 + 字形栅格化）。候选：适配层把标题**画两遍**
（底层 `color:transparent` + 描边、上层纯填充）。做完后整图应落到 ~1–3/255 档。

---

## 11. 第八轮：把残差定到"字形栅格化"，不是画序

### 11.1 先补遥测，再谈修复

前一轮的双层标题改完数字几乎不动，而**分不清是"没命中"还是"命中了但没用"** —— 这条路上每一步
（正则抽取、选择器匹配、属性覆盖）都是"静默不生效"的高危形态。补了遥测（`render/takumi.rs`）：

```
封面(takumi) 适配：标题双层=true 内联图片=2 张 HTML=725883B CSS=5378B
```

外加 `VTEDIT_TAKUMI_DUMP=<dir>` 会把**交给 Takumi 的那份 HTML/CSS 落盘**（复核用）。
落盘核对：`vt-outer` 1 次、`title-wrap` 3 次（DOM 2 + CSS 1）、`src="data:` 2 张 ⇒ **确实生效了**。

### 11.2 于是结论变了

| 配置 | 平均差 | 说明 |
|---|---|---|
| 现状（单层 + 描边） | 19.57 | |
| **双层（描边在下 + 填充在上）** | **19.93** | **没有改善** ⇒ 残差不是画序 |
| 完全不要描边（纯填充） | **15.73** | 光字形本身就值 ~15.7 |
| 非标题区（区域分解） | **≤1.6** | 背景/帧层/渐变/滤镜逐像素级 |

⇒ **残差的绝大部分是"字形栅格化本身"**（同一支 SimHei，Takumi 与 Chrome/Skia 的 hinting/AA 不同；
标题 120px、占画面中部两行，所以摊到全图仍有 ~15/255）。
**这部分不是适配层能修的** —— 它是两个栅格化器的固有差异，与仓库里既有的
"Chrome/Skia vs ffmpeg/FreeType 光栅化底噪 5~13"是同一类账。

### 11.3 对"能不能接"的最终口径

- **能**：版面几何（位移搜索最优在 (0,0)）、背景三模式、帧层 contain、渐变/暗角/扫描线、
  `filter:blur`、`backdrop-filter`+mask、`mix-blend-mode`+isolation、图层栈顺序与隐藏、
  字距/行高/对齐/逐词高亮、emoji、JPEG 产物 —— 实测**逐像素级**（≤1.6/255）。
  ⚠️ **emoji 那条有前提**：见 §11 开头的字体限定 —— 只对 **COLRv0 / CBDT / sbix** 字体成立；
  **COLRv1 会整片消失**（2026-10-02 实测，自带那支已从 COLRv1 换成 CBDT）。
- **不能**：标题字形的**逐像素**一致（~15/255 的固有差）。
- ⇒ 所以"接 Takumi"的验收线应当是**分区的**：非文字区按严格阈值，文字区另定（或按 SSIM/结构相似度），
  而不是拿全图均值一刀切。

---

## 12. 收口：门禁落地 + 许可 + 卡（2026-10-01）

### 12.1 对拍门禁：`scripts/check-cover-takumi.mjs`（node，零依赖）

- 拿一份真精修工程（`--project` 或 `VTEDIT_COVER_FIXTURE`），**硬链接**副本跑两条通道 → 分区判据；
- **分区阈值**（标定依据写在脚本头部）：
  ```
  THRESHOLDS = { nonTextMean: 4, nonTextOver32: 1.0,   // 实测 0.89 / 0.13%，留余量
                 fullMean: 22,    fullOver32: 13 }      // 实测 19.93 / 11.70%（标题字形），留一成余量
  ```
- `--self-test` 自证判据本身（3 项：全同必须过、全黑 vs 全白必须红、**分区独立** ——
  只有中间带差时非文字区过关而整图判红）；
- 缺工程 / 缺带 feature 的二进制 / 字体库为空 / 没 ffmpeg → **SKIP 并打印要准备什么**（`--require` 时算失败）。

实测（真工程 clip-25）：`chrome 3391ms / 290197B` vs `takumi 4388ms / 261016B`；
**非文字区 0.89 / >32 0.13%**（⇒ 除标题外全部逐像素级）、整图 19.93 / 11.70% ⇒ **在阈值内**。

### 12.2 许可

- `cargo xtask scan` **PASS**（它扫的是**默认** release 产物；`cover-takumi` 默认关 ⇒ 不在里面）；
- `THIRD-PARTY-NOTICES.md` 新增「可选 feature `cover-takumi`（默认关）」一节：takumi 四件
  **MIT OR Apache-2.0**、parley/harfrust 排版栈、**ICU4X = Unicode-3.0**（宽松但不在 MIT/Apache 之列 ⇒
  需保留声明）、libwebp-sys **BSD-3-Clause**（并记下它带来的 `LNK4098` 静态 CRT 链接器警告）。
  **⚠️ 若把该 feature 打进发行包，必须重跑 scan 并复核该节。**

### 12.3 三张卡已补

`crates/yekitrim-polish/AGENTS.md`（两条通道 / 静态构建入口 / 变量块 / 字体 / 口径）、
`src/render/AGENTS.md`（`takumi.rs` 是封面第三条后端 + feature 门）、
`templates/AGENTS.md`（`cover.html` 的两个静态占位：`:root{}` 与 `__EXPORT_CSS__`）。

### 12.4 怎么用

```bash
cargo build -p yekitrim-cli --features yekitrim-polish/cover-takumi   # 默认不编
VTEDIT_COVER_RENDERER=takumi yekitrim polish cover <工程目录>       # 默认仍是 chrome（兜底）
yekitrim polish cover <工程目录> --renderer takumi                # 显式参数优先（WebUI 弹窗走的就是这条）
node scripts/check-cover-takumi.mjs --project <工程目录>          # 双通道对拍
```

### 12.5 最终口径

- **可换**：除"标题字形逐像素一致"外，封面的一切（几何 / 背景三模式 / 帧层 / 渐变 / 暗角 / 扫描线 /
  滤镜 / 混合 / 图层栈 / 字距行高对齐 / 逐词高亮 / emoji）都已验证到**逐像素级**；
  ⚠️ **emoji 那条的字体格式前提见 §11**（只支持 COLRv0 / CBDT / sbix，**COLRv1 会整片消失**）；
  但**速度上别期待收益**：这一份工程里 takumi 反而更慢（4.4s vs 3.4s，图片内联吃掉了优势）。
- **不可换**：标题字形（~15/255 固有差 = 两个栅格化器之差）。
- ⇒ 建议落点：**默认仍是 Chrome**（现状），静态通道作为**显式可选**；用它出片时按 §12.1 的分区阈值验收。


### 12.6 WebUI 入口与能力位（2026-10-01）

- **封面设计器点「生成封面」→ 弹窗选渲染通道**：Chrome（默认，附 11 进程/520MB 的代价）/ Takumi
  （单进程/约 54MB、不需要装浏览器）；取消 = 什么都不做。前端只透传 `POST /api/polish/cover {renderer}`。
- **能力位**：`/api/health.cover_static = { compiled, fonts, fonts_dir, ready }`。`ready = 编了
  `cover-takumi` feature 且字体库非空`；不 ready 时弹窗里那个按钮**置灰并写明原因**（去哪个目录放字体、
  或者重编的命令），而不是点了才在任务日志里失败。
- 判据只有一处：`yekitrim-polish::cover::cover_static_capability()`（服务端与 CLI 都读它）。
---

## 13. 第九轮起：标题专章（真工程 clip-14）

### 13.1 我自己的 bug：发出的 HTML **不闭合**

`split_title_layers` 第一版**复制整个 `#title-wrap`**，但替换串少一个 `</div>` ⇒
**填充层的 wrap 被嵌进描边层的 wrap**，解析器兜底后 `color:transparent` 还继承给填充层。
判据（可复跑）：dump 出来的 `takumi-input.html` 里 `<div` 12 个 vs `</div>` **11** 个。
**症状极具迷惑性**：HTML/CSS hash 都变了、产物 hash 一像素不动（因为两层关系已经乱掉）。

修法：**不复制 wrap**，改成同一个 wrap 内两个 `#title`（描边层 + 填充层）。

### 13.2 四个方案的实测（同一份工程状态、同一二进制）

| 方案 | 产物 | 整图差 | >32 |
|---|---|---|---|
| 起点（HTML 不闭合） | 237404B | 34.94 | 28.45% |
| **双层 + 修好闭合** | 225335B | **21.21** | 16.27% |
| 双层 + 填充层内联绝对定位 | 231739B | 25.13 ❌ | 17.86% |
| **单层 + 无前缀 `text-stroke`** | 278647B | **21.13** | 19.47% |

⇒ 描边**顺序**问题已解决（两种写法都能表达"填充在上"）；剩下 ~21/255 **不是** paint-order。
依据：实验台 `10e_text_stroke_prop`（`text-stroke:8px`）观感正常，而 `10_stroke`（`-webkit-text-stroke:8px`）糊成黑块。

### 13.3 已证伪的一整类猜测（都有最小单点）

内联 `style` 有效 · CSS 追加链路有效（注入 `#title-wrap{background:#00ffff}` → 产物 hash 变）·
`img`/`div` 上的 `transform` 有效（含百分比 origin + `object-fit:contain`，差 0.85/255）·
逗号带空格无碍 · 取消描边的三种内联写法都有效 · 双层结构在**干净文档**下有效（`probe-replicate.mjs` 对照图）。

复跑：`fix-split.mjs`（结构+E2E）· `fix-textstroke.mjs`（本版在位）· `probe-css.mjs` · `perturb.mjs` · `probe-transform.mjs` · `probe-stroke-cancel.mjs` · `probe-replicate.mjs`。

---

## 14. 收口（第 9–19 轮）：三个自伤 bug、一次口径修正、逐效果达标

### 14.1 我自己发的 HTML 里踩的三个坑（都有判据，别再犯）

| # | Bug | 判据（可复跑） | 后果 |
|---|---|---|---|
| ① | `split_title_layers` 的正则 `([\s\S]*?)</div></div>` 把**自己的闭合也吃进去**了 | 数 dump 里 `<div` 与 `</div>`：曾 11 vs 9 | 两层关系全乱；"改 HTML 却一像素不动" |
| ② | `.hl` 高亮词的透明色被 `replace` 循环加了 **4 遍**（非幂等） | 数 `style="color:transparent" style=`：曾 4 | 非法 HTML（属性重复） |
| ③ | wrap 的 `height` 把**结尾那个 `<br>` 也当成一行** | 高度 1020 vs Chrome 的 816 | 整块下移 102px ⇒ **整图 27 → 3.5** |

另有一条**真缺陷**（不是自伤）：模板的 `.hl` 规则**自带 `-webkit-text-stroke`** ⇒ 双层里**两层都要各自处理**（描边层 `color:transparent`、填充层 `-webkit-text-stroke:0`），漏一层就是"只有高亮词是实心黑块"。

### 14.2 判据口径修正（门禁）

原"非文字区 = 上下各 20%"会把**标题第一行**圈进去（于是那 1.26% 被算成背景差）。现在由 `polish.toml` 的
`text_y / font_size / line_height / title_lines` 算**标题矩形**，判据取矩形之外；读不到配置时回退 20% 并打印说明
（`scripts/check-cover-takumi.mjs` 里的 `textRows()`）。

### 14.3 逐效果扫描（`sweep-effects.mjs`，以 clip-14 为基座，每变体双通道各出一张）

| 变体 | 整图 | >32 | 非文字区 | >32 |
|---|---|---|---|---|
| base | 2.00 | 1.54% | 0.72 | 0.48% |
| bg=blur | **4.98** | 2.61% | 3.95 | 1.00%（边缘） |
| bg=image | 3.28 | 2.55% | 1.26 | 0.81% |
| **fx=全部六种** | 2.64 | 1.61% | 0.68 | 0.19% |
| fx=glow / blur / vignette / overlay / scan / emoji | 2.24 / 2.18 / 1.98 / 1.80 / 2.00 / 2.78 | ≤2.08% | ≤0.88 | ≤0.51% |
| layers=隐藏关键帧层 | 2.00 | 1.54% | 0.72 | 0.48% |

⚠️ 扫描脚本自己的两个坑（已修，注释在位）：正则 strip 会把 TOML 弄坏（表现：**所有变体同一个数**）；
工程里 `effects` 可能写成 `[cover]` 下的**内联数组**（只剥 `[[cover.effects]]` 表头会踩 `duplicate key`）。

### 14.4 最终绿测（clip-14，同一份工程状态）

```
chrome 3142ms / 263185B    takumi 4038ms / 323655B
非文字区：均值 1.17   >32 0.58%     ✅（≤4 / ≤1%）
整图：    均值 3.50                ✅（目标 ≤6；起点 34.94）
✅ 在阈值内  exit = 0
```

配套：`check_module_map.py` 35/7 ✅ · `check_module_boundaries.py` 15 条/243 文件/0 ✅ ·
`cargo check`（默认与 `cover-takumi`）0 warning ✅ · 桥测试 **7 passed**（含钉住 14.1 三个 bug 的那条）。

复跑：`node .workbuddy/cover-e2e/sweep-effects.mjs` · `node scripts/check-cover-takumi.mjs --project <clip-14>` ·
`node .workbuddy/cover-e2e/fix-height.mjs`（③ 那条修复 + 裁剪图）· `fix-balance2.mjs` / `fix-hl-fill.mjs`（①②与 `.hl`）。

## 15. 标题描边不准 / emoji 字体不一致 / emoji 没描边（2026-10-02 三个问题的收口）

三个问题都是**同一条根**：用户的 emoji 在 **`title_lines` 的联排文本里**（clip-14 的真实配置就是
`直接飞出去流鼻血🩸`、`effects = []`），**不在** `[[cover.effects]]` 特效层 —— 所以特效层那套
（`cover_fx_html` 的描边环、`#fx-extra` 的权威字族层）一行都不落在它身上。上一轮据「特效层」推的
修法因此**全部无效**（用户复报「三个问题都没解决」）。教训：**先确认用户的东西在哪条路上，再动手。**

### 15.1 「中文描边不够准确」= Takumi 的**合成加粗**，不是 join

受控实验（同一字体文件 乐米波波体、同 200px 字号、同 20px 描边、无阴影、平底）：

| | 描边外扩 | 填充 bbox | 说明 |
|---|---|---|---|
| Chrome `font-weight:400` | 9px | 154×180 | 浏览器基准 |
| Chrome `font-weight:900` | **9px** | **154×180** | **逐像素与 400 相同** ⇒ Chrome 对这支单字面字体**不合成加粗** |
| Takumi `font-weight:400` | 10px | 166×192 | （166/154 的差是当时两边**用了不同字体**的乌龙，不是引擎差） |
| Takumi `font-weight:900` | **7px** | **172×198** | Takumi **合成加粗**：填充向外撑 6px、吃掉描边 3px |

机制：`takumi-core/resources/glyph.rs::synthesis_embolden_strength(font_size)` = `font_size × skia_fake_bold_factor`
（≥36px 时 = `1/32`；200px ⇒ e = 6.25px），由 `draw_text_embolden` 用 `Stroke::new(e)` 把**填充**撑开 `e/2`；
而 `draw_text_stroke` 是按**未加粗**的原始轮廓描的 ⇒ 填充盖掉描边内缘。线性验证：stroke 20/40/60 ⇒
外扩 7/17/27 = `w/2 − 3`（**恒定少 3px**，与宽度无关 —— 正是 `e/2`）。

修法：`TAKUMI_TITLE_LAYER_CSS` 末尾加 `#title{font-weight:400}`（同特异度按文档序压过模板那条
`#title{…font-weight:900…}`）。库里**每支字体只有一个字面**（`font.ttf`），Takumi 永远拿不到真粗体 ⇒
合成加粗必然是「预览与出片不一致」的来源。真工程 A/B（clip-14、同一份 HTML、Chrome vs Takumi）：
第 4 行区域「暗/亮像素比」**修正前 0.24 vs 0.41 → 修正后 0.48 vs 0.51**。

> `stroke-linejoin:round`（先前那轮加的）**仍然要留**：Miter 在 CJK 尖笔画上会出尖刺，那条是真问题，
> 只是**不是**「描边不准」的主因。

### 15.2 「前后端 emoji 字体不一致」= 浏览器根本没拿到库里那支

- 出片侧：Takumi 注册全库（emoji 收尾），🩸 落在 **NotoColorEmoji-CBDT**（彩色 CBDT 位图）。
- 浏览器侧：标题栈 = `乐米波波体` + fallback —— **一个 emoji 族都没有**；本机又没装 `Noto Color Emoji`
  ⇒ 落到系统 **Segoe UI Emoji**。**两支完全不同的字体**（字形/配色都不同）。
  ⚠️ 而且 `/api/polish/font?name=` 与 `@font-face` 都**按 id 解析**，用**内部族名**（`Noto Color Emoji`）
  注册 FontFace / 请求字节会两头落空（服务器兜底回 `msyh.ttc`）—— 先前那轮就是这么写的，所以是空操作。

修法（两边都用**库 id**）：

1. `cover.rs::cover_font_css` 把 emoji 那支也注进 `@font-face`（`font_face_css(&[title_id, emoji_id])`）；
2. 标题里的 emoji 段（`wrap_emoji_runs` / `emoji_run_span`）写 `font-family:'<库 id>'`；
3. 设计器 `loadFonts` 按 **id** 注册 FontFace（选哪支以服务端 `emoji_family` 为准），`wrapEmojiRuns` 是镜像。

**Takumi 认族名**（纠正 §13 时代的旧结论）：受控实测 —— 注册四支字体后把某行的族名写成库 id，
**连数字都换成该字体的字形**；旧结论「族名无用、只由注册序决定」是把「**认不出的**族名会落进
『已注册字体按注册序』的兜底桶」当成了「族名不起作用」。⇒ 两条控制点都有效：注册序决定「认不出的族名」落哪支，
族名决定「指定了族名」时用哪支。

### 15.3 「渲染的 emoji 没有描边」= 位图字形没有描边路径，只能用环

`takumi-raster/text_drawing.rs`：描边只走 `ResolvedGlyph::Outline`；CBDT/COLR **位图**分支没有描边代码 ⇒
`-webkit-text-stroke` 对 emoji **物理无效**（Chrome 侧同样不描彩色 emoji，所以预览里也没有 —— 用户要的是
「和标题一样的描边」）。修法：**4 向**（只发正上/下/左/右）`drop-shadow(±k 0 0 #2a2522)` 环，`k = stroke/4`（= 文字描边外扩量的**一半** ——
首版用 `stroke/2`，用户 2026-10-02 反馈「emoji 的描边太厚了，削弱一半」。特效层那条 emoji 环
（`cover_fx_html`）同口径：`size × 0.06 → size × 0.03`）。

⚠️ **只许 4 向，别加对角**（2026-10-02 实测纠正）：`filter: drop-shadow` 是**链式**的 —— 每条阴影吃的是
「已叠加的整层」（takumi-raster `apply_drop_shadow_filter` 的 `composite_pixel_under`；浏览器同规范）。
8 向时对角那两条继续沿 -x/-y 推已叠加的层 ⇒ 最外缘 = `k + 2k·0.707 ≈ 2.4k`（真封面实测：`k=5` 时暗带
**13px**、`k=10` 时 **25px** —— 用户「还是太厚」的由来）。4 向每条方向只被推一次 ⇒ 外缘**精确等于 k**。

⚠️ **环宽不留 1px 下限**（原写法 `.max(1.0)`）：描边拖到 0 时环必须一起消失，否则「文字没描边、emoji 却挂一圈」
——用户口径「描边无法更改」的另一半原因。修后实测（clip-14 真工程）：描边 0 → 环 0px、20 → 5px、40 → 10px。

两个实测钉子（都在 `probe_title_emoji_variants` 里，`--ignored`）：

- **必须 `display:inline-block`**：Takumi 对**行内** span 的 `filter` **不生效**（逐像素与无 filter 版相同）；
- **inline-block + filter 不改变 emoji 的尺寸/位置**：红滴掩膜在「无环 / 行内环 / inline-block 环」三版里
  都是 `w=50 h=74`、同一个左上角 ⇒ 环是**加法**，不会把 emoji 画大或挪位。

### 15.4 复跑命令（真实工程 + 真实浏览器基准）

```bash
# ① 出片侧（真实工程、真实字体库、真 1920×1080）
VTEDIT_COVER_PROBE_DIR=<目录> cargo test -p yekitrim-polish --lib --features cover-takumi \
  probe_real_cover_pixels -- --ignored --nocapture        # 可加 VTEDIT_COVER_PROBE_PROJECT=<clip 目录>
# ② 浏览器基准：把 ① 落下的 cover.html 里的 @font-face 指到文件路径，再用 headless Chrome 截图
chrome --headless=new --disable-gpu --window-size=1920,1080 --screenshot=chrome.png \
  --allow-file-access-from-files file:///<目录>/page.html
# ③ 引擎能力（族名定向 / 环要不要 inline-block）
VTEDIT_PROBE_FONT_ROOT=<字体库> cargo test -p yekitrim-polish --lib --features cover-takumi \
  probe_title_emoji_variants -- --ignored --nocapture
```
