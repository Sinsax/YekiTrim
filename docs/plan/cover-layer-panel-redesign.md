# 封面设计器：图层栈重设计（PS 口径）

> 状态：**已实现（P0 + P1，2026-09）** ｜ 范围：`crates/vtrim-polish`（`cover.rs` / `project/config.rs` /
> `templates/cover.html`）、`crates/vtrim-server/src/api/polish.rs`、`webui/src/components/CoverDesigner.vue`
>
> **实现结果与本文档的三处偏差**（以代码与模块卡为准，本文档保留决策过程）：
> 1. **`CoverEffect` 只加了 `id`，没加 `visible`** —— 可见性**只住 `[[cover.layers]]` 一项**，
>    免得"同一件事两个字段"最后打架。
> 2. **加了"默认栈走改造前渲染形式"的快路径**（`cover.rs::is_default_stack`）：默认栈时一个逐层
>    z-index 都不注入。实测给五块都写 z-index 会让 Chrome 换一条合成路径（缩放帧重采样差 ~9/255），
>    SHA-256 就变了 —— 而"存量逐字节零回归"是本仓真出片对拍过的硬判据。三个样本（横/竖/六特效 rich）
>    改前改后 SHA-256 **相同**。
> 3. **`overlay` 特效额外带 `isolation:isolate`**：改造前 `#fx-extra` 是 stacking context，
>    `mix-blend-mode:overlay` **根本没混上**（绿底叠 `#ff3366` 是橄榄绿）；拆开后它会真混合 →
>    老封面变色。这次保持原观感，真混合留给 P2 的「每层混合模式」。
>
> 一句话：v1 的图层面板只是**同一份配置的另一种读法**（顺序由出片模板写死，隐藏靠清空字段）；
> 这次变成了**真正的图层栈** —— 顺序、可见性都落进 `polish.toml`，出片按栈注入 z-index，
> 交互按 Photoshop 的图层面板来。

---

## 1. 目标与口径

**要做的四件事**（用户口径）：

1. **面板换位**：图层设置栏在上、图层栏（列表）在下。
2. **图层可拖动排序 + 可隐藏**：列表里拖动改顺序；眼睛控制可见性（非破坏性，数据都留着）。
3. **移动逻辑改成"选中哪层就在画布上拖哪层"**（不再永远拖视频预览那一帧）。
4. **删掉文字的 4 个预设**（左文 / 居中 / 右文 / 极简），只留基础功能。

**不变式（贯穿全篇，违反即返工）**：

- **设计器预览 ⇄ 出片逐像素同口径**：出片是 headless Chrome 对同一份 HTML 截图，
  所以"预览里看到的层序"必须就是"出片里画的层序"。顺序**只有一处推导**（见 §3.2），两端各自镜像并互相对拍。
- **画布恒 1920×1080 横版**、`object-fit:contain`、背景/帧两份图源各不覆盖（v1 的规矩全部保留）。
- **老工程零迁移**：`polish.toml` 缺新字段 = 走默认栈，成片**逐像素不变**（判据见 §4.3）。
- **不做任何"只影响预览"的层序** —— 那是本模块历史上最贵的一类 bug（`.stage-video` 黑边、设计器与出片不一致）。

---

## 2. 现状与差距

| | v1（已实现） | 这次的目标 |
|---|---|---|
| 层序 | 出片模板写死（`#bg` < `#frame` < `#vignette` < `#overlay` < `#fx-extra` < `#title`），列表只是**倒着排的投影** | 栈落盘（`[[cover.layers]]`），出片按栈注入 z-index |
| 调序 | 只有特效/表情之间能 ▲▼（它们同属 `#fx-extra`，数组顺序 = 叠放顺序） | **任意两层都能拖**：文字可以拖到滤镜下面、emoji 可以拖到文字上面 |
| 隐藏 | 单层三种靠"清空自己的字段"（`title_lines=['']` / `gradient_dir='none'` / 纯黑背景），刷新后等同于删除 | `visible = false`：**存在但不画**，数据一个字节不动 |
| 删除 | 清字段 + 会话内 stash（刷新即丢） | 从栈里摘掉（属性字段全留）→ 可撤销式删除，刷新也在 |
| 画布拖动 | 永远拖视频预览那一帧（`bg_x/bg_y`） | **选中哪层拖哪层**（见 §5.6） |
| 预设 | 4 个 chip（同时改文字位置 + 滤镜，语义混乱） | 删掉；位置/字号/描边各归各的滑块 |
| 面板 | 头 → 图层列表 → 属性面板 | 头 → **属性面板** → 图层列表（按用户要求换位） |

---

## 3. 数据模型：图层栈（新字段）

### 3.1 字段草案

```toml
[cover]
# 图层栈：**从下到上**（栈底在前）。缺项/未知 id 按默认序补齐；老工程没有这一段 = 走默认栈。
# id: bg | frame | filter | title | fx:<effect.id>
[[cover.layers]]
id = "bg"
visible = true          # 隐藏 = false：这一层还在（数据全留），只是不画

[[cover.layers]]
id = "filter"
visible = false

# 特效/表情：各自成为**独立图层**（顺序由 cover.layers 决定），所以需要一个稳定 id
[[cover.effects]]
id = "k3f9"             # 新增：稳定 id（老数据为空 → 见 §3.2 的兜底）
kind = "emoji"
amount = 0.5
x = 65.0
y = 50.0
text = "😆"
```

- **属性字段一个都不动**：`title_*` / `bg_*` / `gradient_dir` / `vignette` / `effects[]` 全部照旧。
  新字段只描述"**哪些层在、按什么顺序、可见不可见**"。
- **可见性只在 `layers` 一份**（原计划想在 `CoverEffect` 上再加 `visible`，实现时去掉了：
  同一件事两个字段迟早打架）。
- **`filter` 是一层、内部两个 DOM 块**（`#overlay` 渐变 + `#vignette` 暗角，永远相邻）。
  理由：v1 就把它们当一层编辑（`gradient_dir + vignette` 在同一个属性面板里），拆成两层对用户没有收益，
  却要在栈里维护两条必须相邻的项。**（开放问题 1：要不要拆成两层？）**
- **安全区两条不进栈**：它们是设计器辅助线（`localStorage['vtrim.ui.coverGuides']`），不进出片，也不在 `layers` 里。
- **`opacity` / `blend`（PS 面板顶部那两行）**：字段上很便宜（`[[cover.layers]] opacity = 1.0` +
  `blend = "normal"`，出片就是给该层 CSS 加 `opacity` / `mix-blend-mode`，两端都是 Chrome DOM，天然同口径），
  但属于**本期可选**（见 §7 的 P2 与开放问题 2）。

### 3.2 唯一推导：`resolve_cover_stack()`

```rust
// crates/vtrim-polish/src/project/config.rs（或 cover.rs，与 cover_bg_vars 同级）
pub struct ResolvedLayer { pub id: String, pub block: LayerBlock, pub visible: bool, pub effect: Option<usize> }
pub enum LayerBlock { Bg, Frame, Gradient, Vignette, Effect, Title }

/// 默认栈（== 今天模板写死的那一序）：bg < frame < vignette < overlay < 每个特效 < title
pub fn default_cover_stack(cc: &CoverConfig) -> Vec<ResolvedLayer>
/// 配置真有 layers 时：按它排，缺的层按"默认序里的相对位置"补进来，未知 id 忽略
pub fn resolve_cover_stack(cc: &CoverConfig) -> Vec<ResolvedLayer>
```

容错规则（**零迁移的关键**，每一条都要有单测）：

| 情形 | 处理 |
|---|---|
| `layers` 为空/字段缺失（老工程） | 返回 `default_cover_stack()`：`bg, frame, vignette, overlay, fx[0..n], title` —— 与今天逐像素一致 |
| `layers` 有内容但**少**了某些 id | 缺的按默认序的相对位置插入（"补齐"而不是"丢掉"）—— 否则手改 toml 删一行会让某一层凭空消失 |
| `layers` 里有**未知 id** | 忽略（前向兼容；别报错挡住出片） |
| `effects[].id` 为空（老数据） | 按数组序依次插在默认的 fx 槽（vignette/overlay 之后、title 之前） |
| `layers` 同时列了老特效的**空 id** | 不匹配任何项 → 走上面那条兜底 |
| 同一 id 出现两次 | 保留第一次，后面忽略 |

> 设计器打开时**不做**任何"补 id / 物化栈"的写盘（避免开一次页面就重写所有工程）；
> 只有当用户真的拖了/点了眼睛，才把**完整栈**写进 `polish.toml`（幂等）。

### 3.3 语义替换（v1 的"存续判据"作废）

有了栈之后，**"在不在"和"画不画"分开了**，这正是 PS 的语义：

| 动作 | 数据 | 出片 | 能否撤销 |
|---|---|---|---|
| **隐藏**（点眼睛） | `visible = false`，属性全留 | 该层 `display:none` | 再点一下即恢复，刷新也在 |
| **删除**（垃圾桶） | 从 `layers` 摘掉该项，属性**仍然全留** | 该层不渲染 | 从「+ 新建图层」再加回来，内容和位置都还在 |
| **清空内容**（如标题清空、特效删除） | 属性字段归零 | 层在，但画出来是空的 | 数据没了，靠重打 |

- 于是 v1 那套「文字层存在 ⇔ `title_lines` 有非空行 / 滤镜层 ⇔ 渐变非 none / 背景层 ⇔ 非纯黑」
  **全部作废**：层存在性只看 `layers`。`title_lines` 空 + `title` 在栈里 = 走 `project.name` 回退（与今天一致）。
- 好处：`CoverDesigner.vue` 里的 `textStash/filterStash/bgStash` 和三个 `watch(字段→把层变出来)` 都能删掉
  —— 现在那套是"用属性字段模拟开关"的补丁，栈一落地就没存在理由了。
- 属性为默认值但层在栈里：**照画**（比如 `bg_mode=color + 纯黑` 的背景层 = 一层纯黑，与"没有背景层"在
  黑底画布上看不出差别，但它是**真的存在**——可以随时改成别的色，也可以拖到标题上面去）。

---

## 4. 出片侧实现（`cover.rs` / `cover.html`）

### 4.1 注入方式：一份 `cover_layer_css()` + 每个特效自带 z-index

```rust
/// 栈 → 注入样式（一行 CSS 规则块）：层级用整数，默认栈的编号 == 今天的实际叠放序
/// #bg{z-index:1} #frame{z-index:2} #vignette{z-index:3} #overlay{z-index:4}
/// #title-wrap{z-index:9}  #fx-extra{z-index:auto}   ← 关键：不能是 stacking context
fn cover_layer_css(stack: &[ResolvedLayer]) -> String
```

- **`#fx-extra` 必须去掉 `z-index:1`**（`templates/cover.html:71` 内联那条）：
  它现在 `position:absolute + z-index:1` = 一个 stacking context，里面所有特效被**锁死在同一层**，
  单靠外层的 z-index 拖不开。去掉之后子元素自己带 `z-index` 就参与 `#page` 的叠放，
  **每个特效才能成为独立图层**。`pointer-events:none` / `inset:0` 原样保留。
- 每个特效 div 的 `z-index` 由 `cover_fx_html(cc, stack)` 写进它自己的内联样式
  （它现在就是拼内联样式，加一段 `z-index:N;` 即可）；`visible=false` 的特效写 `display:none`。
- 五个固定块的 z-index / display 走 `cover_layer_css()` 的规则块，**不用改模板的 DOM 结构**
  （`#bg/#frame/#vignette/#overlay/#title-wrap` 的 id 都不动，只多一份 style 注入）。
- 注入点：模板 head 里已经有 `<!-- __FONT_FACE__ -->` 这一类占位注入，再加一处
  `__LAYER_CSS__`（或直接 `.replace("</style>", &format!("{layer_css}</style>"))`，二选一，实现时定）。

### 4.2 顺手消重：两个构建器合流

`cover.rs` 现在有**两份**几乎相同的变量替换：

- `build_cover_html()`（`cover.rs:386`）→ 预览服务器用（`__EXPORT__=false`）
- `cmd_cover()` 内联那一段（`cover.rs:596` 附近）→ **出片用**（`__EXPORT__=true`）

新注入如果只加在其中一处，就是"预览有层序、出片没有"的经典分叉。所以本次一并把替换抽成一个
`apply_cover_vars(html, cc, …, export: bool) -> String`，两处都调它（**这是本次唯一的"顺手改"，理由是它直接防止新功能分叉**）。

### 4.3 零回归判据（必须真跑）

1. **Rust 单测（表驱动）**：`resolve_cover_stack()` 的 9 种容错情形 + `default_cover_stack()`
   的**块序 == 旧硬编码序**（`bg, frame, vignette, overlay, fx*, title`）。
2. **真出片对拍**：改前 / 改后各出一次封面（不设新字段 = 默认栈），**SHA-256 相同**。
   候选样本（`.workbuddy/perf/` 下都已带 `_cover_frame.jpg` + `clip.mp4`，可直接跑）：
   `fadecheck/seed/proj`、`vertical/vert-land`、`renderproj/clip-01` —— 建议前两个各跑一次
   （横屏 + 竖屏素材，竖屏能顺带验证 `#frame` 的 `contain` 与背景层）。
   命令：`cargo run -p vtrim-cli -- polish cover <工程目录>`（改前先在 `.workbuddy/` 留一份成片）。
   这是"老工程逐像素零回归"的唯一硬证据，结论写进 `docs/webui-components.md` ③。
   顺便看一眼 `blur` 特效（`kind="blur"` 用 `backdrop-filter`）：它的"背景采样范围"按规范由 backdrop root
   决定、不受 z-index 影响，所以拆不拆 stacking context 都不该变 —— 但只信像素对拍，别信推理。
3. 再跑一遍"显式指定一个乱序栈"的出片，人工看一眼层序确实按栈来（并留一张成片在 `.workbuddy/`）。

---

## 5. 设计器：PS 感面板

### 5.1 布局（按用户要求换位）

```
┌ 封面设计器 ─────────────────────────────────────────────── [生成封面] ┐
│ ┌──────────────────────────────┐ ┌────────────────────────────────┐ │
│ │                              │ │ 图层            [+ 新建图层]   │ │
│ │      舞台（画布预览）         │ │ ┌────────────────────────────┐ │ │
│ │   ·选中层可直接拖动            │ │ │ ◧ 属性面板（选谁改谁）      │ │ │
│ │   ·Shift 约束单轴             │ │ │ ◧ 位置 / 大小 / 参数 …      │ │ │
│ │   ·滚轮 = 该层的量（见 §5.6） │ │ │                            │ │ │
│ │                              │ │ └────────────────────────────┘ │ │
│ │                              │ │ ┌────────────────────────────┐ │ │
│ │                              │ │ │ 安全区                      │ │ │
│ ├──────────────────────────────┤ │ │  👁 ▢ 16:9 安全区   辅助线   │ │ │
│ │ ▶ ─────●────────  3.2 / 60.0 │ │ │  👁 ▢ 4:3 裁切区    辅助线   │ │ │
│ └──────────────────────────────┘ │ │ │ 封面图层        上 = 更前   │ │ │
│                                  │ │ │  👁 文 文字                 │ │ │
│                                  │ │ │  ⠿ 👁 ✦ 光晕        🗑      │ │ │
│                                  │ │ │  ⠿ 👁 🙂 表情        🗑      │ │ │
│                                  │ │ │  👁 滤 滤镜          🗑      │ │ │
│                                  │ │ │  👁 视 视频预览     🔒       │ │ │
│                                  │ │ │  👁 背 背景          🗑      │ │ │
│                                  │ │ └────────────────────────────┘ │ │
│                                  │ │ [+ 新建] [⧉ 复制] [🗑 删除]    │ │
│                                  │ └────────────────────────────────┘ │
└───────────────────────────────────────────────────────────────────────┘
```

- 面板头一行：`图层` + `+ 新建图层` + 保存状态（`保存中…` / `保存失败`，v1 就有）。
- **属性面板在上、列表在下**（用户要求）。两块各自滚动：属性面板 `flex:1`（`min-height` 不能省，
  v1 被这条坑过 —— 会被 flex 收缩压成 0 高），列表 `max-height` 定高滚动。
- 底部工具条（PS 的图层面板底部那一排）：新建 / 复制 / 删除。键盘党不看它也能干活（见 §5.8）。

### 5.2 行结构（PS 的五格 + 我们的徽标）

```
[⠿] [👁] [缩略] 名称 .................. [徽标] [🗑]
 拖柄  眼   20px  双击可重命名          锁定/辅助线
```

- **拖柄**：只在可拖的行上出现（这次**所有内容层都能拖**；安全区两行没有拖柄，它们不在栈里）。
  鼠标移上去整行也可拖（PS 的手感），拖柄是"看得见的抓手" + `Hover` 提示。
- **眼**：`👁 / 🚫`（内联 SVG，v1 已有）。**点击只切可见性，不选中该层**（`@click.stop`），
  与 PS 一致；`Alt + 点击` = **只看这一层**（solo，P2 可选）。
- **缩略**：v1 的字符缩略（`文/滤/视/背/光/表`）保留；**emoji 层直接显示那个字符**，
  有图源的两层（视频预览/背景）后续可以换成真的 20px 小图（P2，`_cover_frame.jpg` / `_cover_bg.jpg`）。
- **名称**：默认取类型名；**双击进入重命名**（`[[cover.layers]] name = "…"`，空 = 默认名）。
  ⚠️ 重命名要 `@keyup.enter` 提交、`Esc` 取消、失焦提交，且**不能**把点击冒泡成"选中"（P1 或 P2，开放问题 3）。
- **徽标**：`锁定`（视频预览：不可删）、`辅助线`（两条安全区）。隐藏的层：名字与缩略一起降到 `--ink-faint`
  并加删除线感（PS 里眼睛关了整行变淡），**行还在、还能点**。
- 选中态沿用 v1：`--main-soft` 底 + 左侧 3px 主色条。

### 5.3 拖动排序（这次是真排序）

- **实现**：HTML5 DnD（`draggable` + `dragstart/dragover/drop`）。理由：原生有"拖起来"的视觉反馈、
  不用自己算命中；v1 的列表已经是 `overflow-y:auto` 的容器，`dragover` 到边缘自动滚动用 `scrollTop` 手动补即可。
- **插入指示线**：在目标行的上/下半区插一条 2px 主色横线（`dragover` 时算 `offsetY < h/2`），
  松手时按指示线的位置 `splice`。拖到列表两端之外 = 栈顶/栈底。
- **落点粒度**：内容层之间任意位置都能落（**因为现在顺序是真的**）；安全区分区与它的两条行
  **不接受**内容层（`dragover` 不 preventDefault → 显示"禁止"光标），因为安全区不在栈里。
- **拖动中**：被拖的行半透明（`opacity:.5`）；`Esc` 取消（`dragend` 里判断 `dropEffect === 'none'` 复原）；
  点一下不拖 = 只是选中（不写盘）。
- **写盘**：松手后把**完整栈**（`layers = [...]`）交给既有的 600ms 防抖保存 —— 顺序变化走同一条
  `/api/polish/cover-config` patch（白名单要加 `layers`）。
- **可选强化（P2）**：拖动中左侧预览实时按新序重排（`z-index` 直接跟着算，不落盘）；
  多选拖动（成组移动）。

### 5.4 眼睛（非破坏性隐藏）

- 写 `visible`（栈项 / 特效项各一份）；出片与预览两侧都渲染成 `display:none`。
- **安全区两行**仍走 `localStorage`（它们不是出片图层）—— 交互一样，含义不同，行上已有「辅助线」徽标区分。
- **视频预览也能隐藏**了（v1 做不到：没有字段）。隐藏后画布只剩背景层 + 其它层 —— 语义干净。
- Alt+点击 = solo（只看这一层）：实现上是"临时把其它层的 visible 当 false 渲染、不写盘"，
  松手/再点恢复。P2。

### 5.5 画布内拖动（"移动逻辑"）

**规则：改哪个字段，只看当前选中哪一层。** 光标也要跟着变（`grab` / `not-allowed`）。

| 选中的层 | 拖动改 | 滚轮 = | 备注 |
|---|---|---|---|
| 视频预览 | `bg_x` / `bg_y`（画布 px，按 `stageScale` 换算，与 v1 同一套） | `bg_zoom`（0.5~3，v1 行为） | 原点仍是 `liverOrigin()`（自由选框 > grid > origin > 65% 50%），**不动** |
| 文字 | `text_x` / `text_y`（百分比，中心点） | 不改（防误触；字号走滑块） | 拖到边缘夹在 0~100 |
| 特效 / 表情 | `x` / `y`（百分比，中心） | `amount`（表情=大小，其它=强度，0~1） | 拖动时 x/y 取整到 1%（v1 属性面板也是整数） |
| 滤镜 / 背景 | **不可拖**（`not-allowed` 光标 + 画布上浮一句「这一层没有位置」） | 不改 | —— |
| 安全区 | **不可拖** | 不改 | 它们的尺寸是几何常量（1824×1026 / 1440×1080） |

- 交互细节：
  - 用 `pointerdown/move/up`（v1 已有这套，且处理了"切片段时 window 监听要摘掉"）；`touch-action:none`。
  - **`Shift` 约束单轴**（PS 手感）：锁定开始拖动那一刻的主方向。
  - **拖动中不落盘**，松手才让现有防抖保存写盘（与 v1 一致：拖动过程只改本地 ref）。
  - **不发任务**：改位置是同步保存，不碰 `polish process`（与 v1 的选框保存同一纪律）。
  - 拖动中的读数：舞台角上浮一枚小 chip（`X 46% · Y 52%` / `+18px, -6px`），松手消失。
- **P2 强化：命中即拖**（真正的 PS 手感）——点画布上的**任意对象**直接开始拖它，不必先选中：
  从栈顶往下做命中测试（emoji 用字号算出的方框、文字用 `Range` 量出的文本框、帧层兜底），
  跳过隐藏层；点空白 = 拖**视频预览**（今天的默认行为）。本期先做"选中哪层拖哪层"（用户原话），
  命中测试放 P2 —— 它要处理"点空白算谁的"这种歧义，值一次单独的评审。

### 5.6 属性面板（随选中层）

- 结构不变（v1）：视频预览（时间 / 取当前帧 / 导出关键帧 / 缩放 / 偏移）、背景（纯色 / 当前帧模糊 /
  自定义图片）、滤镜（渐变 / 暗角）、文字（标题 / 高亮色 / 阴影 / 关键词 / 对齐 / 位置 / 字号 / 描边 / 字体 / 行距 / 字距）、
  特效（各 `kind` 的参数）、安全区（只给显隐）。
- **文字面板删掉 4 个预设 chip**（用户要求）：位置/对齐/字号/描边各归各的滑块，不再有一键换排版。
  ⚠️ 连带要改的文档：`webui/src/components/AGENTS.md` 里"默认预设左文"那两条理由、以及
  `docs/webui-components.md` 3.6 的推导段（默认 `text_align=left` / `text_x=35%` 保持不变，只是不再有预设入口）。
- 面板顶部（属性区之上）预留 PS 的"不透明度 / 混合模式"两行位置 —— **本期不填**（见 P2）。

### 5.7 键盘（PS 味的快捷键）

| 键 | 作用 |
|---|---|
| `Ctrl/Cmd + ]` / `[` | 选中层上移 / 下移一层（等价于拖一格） |
| `Shift + Ctrl/Cmd + ]` / `[` | 置顶 / 置底 |
| `Delete` / `Backspace` | 删除选中层（视频预览除外） |
| `Alt + 点击眼睛` | 只看这一层（solo，P2） |
| `Esc` | 取消拖动 |

⚠️ 画布已占用 `wheel`（缩放）与 `pointerdown`（拖动），快捷键要绑在**面板容器**上（`tabindex="0"` + `@keydown`），
别挂 `window` —— 精修页有别的全局键（时间轴）。

---

## 6. 诚实边界：PS 里有的、这里不做

| PS 功能 | 这里 | 为什么 |
|---|---|---|
| 图层组 / 嵌套文件夹 | ❌ | 数据模型是平栈；嵌套只增加复杂度 |
| 蒙版 / 调整层 / 通道 | ❌ | 出片侧没有任何对应实现 |
| 图层样式（描边/发光…） | ❌（已有 5 种"特效"独立成层） | 与 PS 的图层样式不是一回事，避免同名混淆 |
| 不透明度 / 混合模式 | ⏳ P2 | 字段+注入都很便宜，但会让"封面为什么糊了"变难解释 |
| 锁定（不可编辑） | ⏳ P2 | 纯编辑期概念，建议只存 localStorage（不进出片配置） |
| 多选 / 成组拖动 | ⏳ P2 | 先保证单选链路稳 |
| 命中即拖 | ⏳ P2 | 见 §5.5 |

---

## 7. 分阶段落地

### P0 —— 数据与出片（其余全依赖它）

| 文件 | 改动 |
|---|---|
| `crates/vtrim-polish/src/project/config.rs` | `CoverLayer{id,visible}` + `CoverConfig.layers`；`CoverEffect{id,visible}`；`Default` 补项；`to_toml_commented` 的 `[cover]` 注释表加 `layers` |
| 同上 / `cover.rs` | `default_cover_stack()` / `resolve_cover_stack()`（纯函数 + 表驱动单测） |
| `crates/vtrim-polish/templates/cover.html` | `#fx-extra` 去掉内联 `z-index:1`；加 `__LAYER_CSS__` 占位 |
| `crates/vtrim-polish/src/cover.rs` | `cover_layer_css()`；`cover_fx_html()` 加 `z-index/display`；抽 `apply_cover_vars()` 并让两个构建器都走它 |
| `crates/vtrim-server/src/api/polish.rs` | `cover-config` 白名单：`layers`（整数组替换）、`effects[].id/visible` |
| `crates/vtrim-polish/AGENTS.md`、`src/project/AGENTS.md` | "封面三层"那段改写成"图层栈" |

验证：`cargo test -p vtrim-polish`（新单测）、`cargo test -p vtrim-server`（patch 用例）、
**真跑一次封面出片对拍 SHA-256**（§4.3）。

### P1 —— 设计器（本次用户要的四件事）

| 文件 | 改动 |
|---|---|
| `webui/src/components/CoverDesigner.vue` | 面板换位（属性在上/列表在下）；行结构（拖柄/眼/缩略/名/徽标/删）；拖动排序；眼睛写 `visible`；画布按选中层拖动；删预设；删 v1 的 stash 与三个 `watch(字段→层)`；payload 加 `layers` |
| `webui/scripts/check-cover-stage.mjs` | `bgUnderFrame` 那条（读组件 CSS 的 z-index）**必须改写**成「默认栈里 bg 在 frame 之下」+「内容层的 z-index 不再硬编码在 CSS 里」，带变异自证 |
| `webui/src/components/AGENTS.md`、`docs/webui-components.md` ③、`webui/AGENTS.md` | 同步（含删掉预设那处） |

验证：`npm --prefix webui run build` + 6 个 `check-*.mjs` + headless harness（拖序/眼睛/画布拖动改的字段/
payload 的 `layers`/几何），再对拍一次出片（设计器看到的序 == 成片的序）。

### P2 —— PS 感加分（按开放问题拍板）

不透明度 + 混合模式（`opacity` / `blend`）· 图层重命名（`name`）· 锁定（localStorage）·
复制图层（特效可复制，单层三种禁用）· 多选/成组拖动 · 命中即拖 · Alt 点眼 solo · 缩略图换成真图。

---

## 8. 风险与钉子

1. **两份构建器分叉**（§4.2）：不清重就必然出"预览有层序、出片没有"。→ P0 必做，且
   `check-cover-stage.mjs` 加一条"两个构建器都调 `apply_cover_vars`"的文本钉子。
2. **`#fx-extra` 的 stacking context**：忘了去掉 `z-index:1` → 特效永远锁在一层里，
   拖动排序看着生效、出片却不动（**不报错**）。→ 脚本钉子 + 变异自证。
3. **`check-cover-stage.mjs` 的既有钉子**：`bgUnderFrame` 现在读 `.stage-bg` / `.stage-media` 的 CSS z-index；
   改成动态 z-index 后这条必然失效，**必须**改写成"默认栈顺序"，不能一删了之。
4. **老工程零回归**：只能靠真出片 SHA-256 对拍（§4.3）。单测覆盖不了 `cover.rs` 的字符串拼接。
5. **手改 toml 的容错**：`layers` 少一项不能导致整层消失（§3.2 的"补齐"规则 + 单测）。
6. **面板高度**：属性面板在上之后，`min-height` 与列表的 `max-height` 都要重新标定（v1 被 flex 收缩压成 0 高）。
7. **拖动排序与滚动容器**：列表 `overflow-y:auto`，`dragover` 到边缘要手动滚动，否则长列表拖不到顶/底。
8. **画布拖动与既有手势**：`pointerdown` 现在同时服务"拖动"和"选中"，要明确：
   点中画布 = 只拖**已选中**的层；换层靠列表点击（P2 的命中即拖另开一条路）。

---

## 9. 开放问题（要拍板）

1. **滤镜拆不拆**：保持"渐变+暗角 = 一层"（推荐），还是拆成两层各自排序/隐藏？
2. **不透明度 / 混合模式**：进 P1 还是 P2（或不做）？
3. **图层重命名**：要不要（`name` 字段，成本很低）？
4. **CLI 预设**：设计器删掉 4 个预设后，`vtrim polish cover --template <t>`（`cover.rs::cmd_cover_template`）
   是否也一起删？（现在它只剩 CLI 一个入口）
5. **P2 的取舍**：多选 / 锁定 / 复制 / 命中即拖，哪些进本期？

---

## 附：实现时要注意的既有约定（照抄，别重新发明）

- 长任务写 `polish.toml` 一律 `project::update_polish_toml`（锁内重载 + 只覆盖自己负责的字段）；
  设计器保存走 `/api/polish/cover-config`（已有路径锁）。**配置改动的四步**（字段 + merge 条件 +
  手动 `Default` + 注释/模板）在 `crates/vtrim-core/src/config/AGENTS.md`，polish 侧对应"字段 + serde default +
  `Default` + `to_toml_commented` 注释 + server 白名单"。
- 改了 `templates/cover.html` **必须重建二进制并真跑一次出片**（`include_str!` 进 `vtrim-polish`）。
- 前端布局类改动**必须做几何断言**（`getBoundingClientRect()` 宽高 > 0）——
  本仓没有 jsdom/vitest，可行做法是 `.workbuddy/` 下的 headless harness（真挂载组件 + Chrome `--dump-dom`）。
- 类别名先查 `styles/app.css`：它是**全局**的（`.pane{display:none}` 那次踩过），
  设计器里用过的类名不能与它撞。
