# tools — 对底座出片的转译器（本仓方言 → Dhampir 契约）

> 范围：`tools`/** ｜ 上游：`polish.toml` / `clip.srt` / `clip.dm.srt`（**只读**）｜ 验证：`node scripts/check-yekitrim-translator.mjs`

## 职责

把本仓的 `polish.toml` 转译成 Dhampir（**底座**，wgpu 预览 + 渲染底座）的工程文件 `*.doc.json`，
供**第二条出片通道**使用。产品自己的出片是 `yekitrim polish render`（headless Chrome）；
这条路是"把本仓的精修工程喂给底座出片"，所以它**宁可报错也不猜**：转不了 / 有歧义的都进
三级清单（`DROP` / `CLAMP` / `NOTE`），打到 stdout。

- **不是产品代码**：`yekitrim` 二进制不 import 它、编译不依赖它；产品代码也不许 import 它。
- **只读本仓的工程**（`polish.toml` / `clip.srt` / `clip.dm.srt`）：**不回写 `polish.toml`**；
  旁写文件（`dhampir-subtitle.srt` / `dhampir-danmaku.ass`）放在**工程文件旁边**。
- **不探测媒体**：`frame_count` 留空，交给底座自己处理（⚠️ 2026-10-01 更正：这句原写"由底座的
  `dhampir probe --write` 回填"，但**底座 CLI 里没有 `probe --write` 这个用法**（`--write` 只被
  `edit/undo/redo/clip/sequence/batch/import` 认）—— 实测 `frame`/`render` 在 `frame_count: null`
  下照样能出片。原句留在这里当"别再抄它"的路标）——
  为的是"在一台没有 ffmpeg 的机器上也能跑完并给出清单"（贴纸图片是例外，见下）。
- **零依赖**：不引 npm 包，`node` 直接跑。

## 入口

`tools/polish-to-dhampir.mjs`（零依赖 ESM，一个文件）：

- **CLI**：`node tools/polish-to-dhampir.mjs <clip 目录> [--out 工程文件] [--keep-muted]` ——
  第一个参数是**片段目录**（里面要有 `polish.toml`），不是 toml 文件本身。
  ⚠️ `--keep-muted` 是**出片那条腿**用的（`render/dhampir.rs::render_video_only` 传它）：时间线保持
  整条，屏蔽区间交给出片尾部那道 `cut_ranges` 按**源时间**剪一次 —— 与 chrome 那条腿同一条口径。
  **预览那条腿不许用**（客户端要靠压缩时间线"落在被掐掉的段里 = 没有对应帧"跳屏蔽区间）；
  两边都剪 = 成片短一大截 + 音轨错位（实测：12s 带 3s 屏蔽，出 6.0s 而不是 9.0s）。
  `DHAMPIR_QUIET=1` 时只留开头那几行统计。
- ⭐ **逐事件 `switch` 就是「事件类型 → 效果参数」的映射表**（11 个 `case`：`camera` `shake`
  `sticker` `sfx` `overlay` `pulse` `vignette`
  `flash` `hue_shift` `noise` `blur`）。每条 `case` 的注释写着**底座用哪条原语、为什么**，
  以及"参照侧这一处本来是什么形状"。**要读懂本模块，先读这个 `switch`。**
- 顶层 `export` 的 6 个纯函数（`backOut3` / `pow2Out` / `shakeOffsetAt` / `handheldSway` /
  `stickerStateAt` / `hexColor`）：前五条是从**出片模板逐行抄来的曲线**，被
  `scripts/check-yekitrim-translator.mjs` import 来逐值测；`hexColor` 是**颜色**那条
  （四种写法，见「边界」）。`isEntryPoint` 那道门是为了"可 import 也可直接跑"。
- 规格与词表：`docs/yekitrim-integration.md`（第一~八节）—— 每条行为的参照行号、已知差异清单、
  "怎么加一个新事件类型"的五步。**改这个文件之前先读它。**
- 贴纸探图：`probeImage()` 调 `ffprobe` 取 GIF 的帧数与尺寸 —— 探不到就返回 `null` 并进 CLAMP，
  **不给默认值假装成功**（`ffprobe` 不在 PATH 上时整条工具仍能跑完）。

## 边界

- ⚠️ **权威在出片模板，不在这里**：`crates/yekitrim-polish/templates/index.html` 的 canvas 那条腿
  （`renderFrame` / `getActiveXxx`）才是"事实来源"，本文件只是它的**复刻副本**。改模板里的曲线与常量
  必须同步这里 + 守卫。**认的是出片那条腿，不是 GSAP 预览** —— `shake` 就抄错过一次：
  预览是 0.025s 连续交替（`index.html:532`），出片是固定 0.2s 的四级阶梯（`:1575`）。
- ⚠️ **改那 6 个纯函数（含 `hexColor`）/ `zoomFactor` / `translate` 或弹幕窗口的 `danmakuExitMs`
  = 必须跑 `--mutate`**：这几条曲线/解析/映射是"看着差不多、差得远"的重灾区，守卫按**逐值**钉了
  **339 项曲线 + 13 项颜色 + 40 项映射**，并要求 **22 处变异全部打红**。新增曲线/常量/解析分支/映射时
  **往守卫的 `MUTATIONS` 里加一条**（"一条不会红的用例不算用例"）。⚠️ 新加的检查**要抽成函数
  并在 `mutate` 里也调**：只写在顶层的话 `--mutate` 那条路径根本不跑它（颜色那条第一版就是这么漏的）。
  `translate` 与 `zoomFactor` 是**为了守卫才 export** 的（`isEntryPoint` 那道门保证直接跑仍是 CLI）。
- ⚠️ **报告清单本身也有门禁**：`node scripts/check-dhampir-drop-list.mjs` 把演示工程的
  **DROP/CLAMP 清单钉成基线**（每条都写着"为什么它还在"）。清单**变一条就红** ——
  多了 = 某次改动把一档转丢了；少了 = 修好了（改基线）或**报告被削弱**（更要查）。
  它零依赖、不要 GPU，所以进常规门禁；`--self-test` 自证"多一条/少一条都必须红"。
- ⚠️ **`overlay` 有四档，别只实现 gradient**（2026-09-30 M2 补的）：`color`（单色 + 0.4s 进出包络）、
  `fade_in` / `fade_out`（**整窗线性斜坡、不叠包络**，颜色缺省黑 —— 片头渐入就是这一档）、
  `gradient`（色档 + 同一包络）。参照是 `index.html:1642-1669`；底座那边四档都是同一个 ColorMask 的
  `overlay`，`shape` = Solid(0)/Linear(1)/Radial(2)、逐帧曲线走**图层不透明度关键帧**（`placeEnvelope`）。
  以前 `color` / `fade_in|fade_out` **整条 DROP** —— 也就是说"开头渐入没生效"曾经有一条是转译侧丢的。
- ⚠️ **贴纸落点有两种模式，基准不同、不许"统一"**（2026-10-02 修）：`auto` 用事件自带的
  `x`/`y`（`process` 安全区算法算好的 **盒子左上角**）；`manual` 固定到 `[sticker] position`
  那一点、**忽略事件的 `x/y`**、含义是 **盒子中心**。两份参照都写着这条（`templates/index.html::stickerAnchor`
  与 `engine.js::stickerAnchor`）。⚠️ **本条曾经整条缺失**：转译器只读 `event.x/y`，于是"设计器里把落点
  改成固定、点保存"在那条腿上**完全丢失** —— 而 DOM 预览（有 `manual` 分支）当场就对，
  **报告里一行差异都没有**，症状极具迷惑性。判据在 `check-yekitrim-translator.mjs` 的 6c-bis
  （**行为**判据：两种模式必须算出不同落点 + 各自符合自己那份基准）+ 两处变异自证。
  ⚠️ 那条"没有 x/y 就拒转"的不变量，判据必须是 `ratio()` 的结果，**不能写 `event.x !== undefined`** ——
  没跑过后处理的工程里那个键**存在**、值恰好是字符串 `"auto"`。
- ⚠️ **贴纸的两处旧伤**（2026-09-30 M2 修）：① **不许因为"没有 `sticker_tag`"就提前 return** ——
  `name` 才是 `Step2b` 选定的文件名（只有它是空的才退回去按 tag 猜）；② 找文件**要试全部图片扩展名**
  （`gif/png/jpg/jpeg/webp`，`event.name` 不带扩展名；只试 `.gif` 会让 PNG 贴纸进不了成片 —— 本仓
  渲染侧踩过同一个坑）。**落点**（`x`/`y`）是**后处理的产物**：`auto` 只出现在没跑过 `process`
  的工程里，那时**报错并告诉人先跑一次后处理**（底座没有安全区推导，这里不猜位置）。
- ⚠️ **`zoomFactor` 的兜底必须照抄模板的 `camFactor`**（`index.html:578-585`）：**按档位**取默认
  （`近 1.5` / `远 0.55` / 其余 `1.3`）、认不出的档位按**中景**、`isFinite` 也要看。
  曾经统一退回 `1.0` ⇒ **"远景"档整段不拉远**（成片里看不出来，报告里只有一行 CLAMP）。
- ⚠️ **颜色要收四种写法**（`hexColor`：`#RGB` / `#RGBA` / `#RRGGBB` / `#RRGGBBAA`，可省 `#`）：
  YekiTrim 的 `polish.toml` 里**四种都出现过**（演示工程三条 overlay 全是 8 位带不透明度）。
  以前只认 6 位，后果**不是"少了个透明度"** —— overlay 那处是 `if (!first) drop(...)`，
  **整条覆盖层事件不进成片**（成片少了整块渐变色罩，而报告里只有一行 DROP）。
  认不出的值仍然必须返回 `null`（不许猜），由调用点兜底或报 DROP。
- ⚠️ **报告里的字符串是接口产物，不是日志文案**：`DROP` / `CLAMP` / `NOTE` 的语义见
  `docs/yekitrim-integration.md` 第五节，给下游的用法是"CI 断言 `DROP == 0` / 把 `CLAMP` 逐条挂台账"。
  **别为了让流程通过而把某条差异从报告里删掉。**
- ⚠️ **已知差异照实报，别静默近似**：**2026-10-01 起 DROP 只剩 `cover` 一条**（`vignette` 已在
  D4 接上：canvas 画 + 映射写；`style.subtitle` 的文字阴影也在 B5 接上了）。谁再引入"报告里没有、
  成片里有"的差异，要写进报告而不是悄悄近似 —— 硬凑会引入看不出来的色偏。
  （⚠️ 2026-10-01 类型收敛后 **CLAMP 归零**：`split` / `stutter` / `zoom_bounce` / `color_shift`
  四个类型已下线（词汇表 15 → 11），老工程会被 `project/validate.rs::retired_effect_message`
  用人话拒绝加载。）
- ⚠️ **源视频竖时的运镜门：已实现**（2026-09-30 核对；这条以前写着"还没复刻"，是**过期文档**）。
  做法是**探一次源尺寸**（`probeSize`，只问宽高、不解码）：探到竖的就整条跳过 camera 事件
  （参照 `index.html:1413` 也是整条早退），**探不到就进 CLAMP 明说"判不了源方向、运镜可能不一致"**
  —— 不给默认值假装成功。这是"不探测媒体"那条纪律的**第二处例外**（第一处是贴纸 GIF 的帧数/尺寸）。
  改它之前先读 `sourceIsVertical` 那段注释：判据是**源素材**的方向，与画布方向无关。
- ⚠️ **它是 JS（node）而不是 Rust —— 这是 2026-09-30 的定案，不是没做**：体量 3.7k 行、其中一大半是
  为对齐参照写死的细节，Rust 化要**逐条重写 + 重建等价的守卫**（339+13+40 项 + 20 变异），
  期间还会三处同步（模板 / JS / Rust）。依赖成本很低（node 只落在出片机器，那台机器本来就要 ffmpeg/Chrome，
  而本仓的门禁链**本身就是 node**）。所以这一轮做的是**把"要 Node"变成可见、可绕**：
  `readiness(font, deep)` 查 Node **版本**（`MIN_NODE_MAJOR = 18`，`parse_node_version` 是纯函数、有单测；
  "PATH 上有 node 但跑不出 `--version`"= 商店桩/装了一半，也单独报）、`README.md` 前置条件写清、
  `polish doctor` 多一行 `Node: vX.Y.Z`、无 Node 时用 `--renderer chrome` 出片。
  **重开的判据与如果真做的路线**（A/B 逐字段对拍 + 基线不动才算过）写在
  `docs/plan/dhampir-parity-register.md` §五 M5。
- ⚠️ **竖屏画布的三道整类门 + 位置百分比（2026-10-02）**：`[canvas] orientation = "portrait"` 时，
  `PORTRAIT_SWITCH` 表里的类型（运镜 / 8 类特效 + `overlay` / `sfx`）**缺字段 = false = 整类不转**；
  表必须与 `webui/src/polish/engine.js`、`templates/index.html` **同集合**
  （`webui/scripts/check-stage-backdrop.mjs` 三方对拍）。事件本身**一个字都不动** —— 与「屏蔽只做标记」同纪律。
  位置读 `config.stage.position_pct`（0–100 插值，老枚举 `auto` → 50 居中）；`camera` 那条**不在**
  行为判据里（`probeSize` 探不到源时会走 CLAMP，断言会空转），覆盖靠「表 + 同一个 `continue`」。
- ⚠️⚠️ **画面构成的几何必须落到「0 键层」的基准 `transform` 上**（2026-10-02 修的真缺陷）：
  `SR`/`FIT`（`axis`/`position_pct`/`scale`/`padding` 的唯一几何）以前**只**由相机关键帧写进去，
  而竖素材的运镜被整条门控掉 ⇒ 那一层**一个键都没有** ⇒ 主层停在 `scale=1/offset=0`，
  底座按「源 1:1 居中」画：**竖素材进竖画布（用户最常见的组合）画面缩在中间、四周一圈黑**。
  现在 0 键层会写一组「中性相机」基准值（`cameraOffset(1.0, {0.5,0.5})` + `scale: FIT`）。
  ⚠️ **第一实参必须是 `1.0`、不是 `FIT`** —— `cameraOffset` 内部已经 `scale = zoom * FIT`
  （它自己的注释就写着「不要预乘 FIT」），传 `FIT` 会变成 `FIT²`，而**居中那一点恰好归零**、看不出来
  （`position_pct=0` 才暴露：正确是 `x = -656.25` 贴左）。有键的层逐字节不动（存量零回归）。
- ⚠️ **别让它变成产品依赖**：不 import `crates/` 里的任何东西、不往 `webui/` 里引；
  它是"边上的一次性通道"，产品要出片走 `yekitrim polish render`。
- ⚠️ **不落中间态到仓库里**：`--out` 指哪写哪，别往固定路径写缓存。

- ⚠️ **贴纸落点属于「一份规则多份实现」那一类，而本文件是第 6 份**（2026-10-05 才被
  `webui/scripts/check-sticker-anchor.mjs` 纳入）：那份契约原先只列了模板 / `engine.js` /
  `canvas.js` / `timeline.rs` / `PolishView` —— **别只看它列的清单**。实测缺口：manual
  `[sticker] position = "98% 50%"` 在 1920 宽下参照钳到 **1608**，而这里原先**不钳**、给
  **1741.6** ⇒ 预览看着在框内、成片甩到框外（101.6px）且不报错。现已按参照补上
  `M = 32*min(W,H)/1080` 的 clamp。
- ⚠️ **`shake.intensity` 是像素**（模板 `:1610`、`engine.js:416`、本文件 `:1718` 都这么用），
  但 **LLM 提示词的唯一示例写的是 `0.5`**（`crates/yekitrim-polish/src/llm/segment.rs:96`）、
  编辑器默认又是 `10` ⇒ LLM 生成的 shake 常是**亚像素**、看不见而报告仍写 `shake_events: 1`。
  要动就"提示词 + 迁移 + 三处 `|| 14`"一起动，只改换算会让存量工程更乱
  （详见 `docs/plan/polish-preview-bugs-2026-10-05.md` R1）。
- ⚠️ **密集键的右端封在窗口末尾**（`denseRanges`）：**shake 窗口压在运镜过渡上**时，窗口之后到
  下一个稀疏键那一段从"逐帧复刻 `pow2_out`"退化成**两点直线**（实测 zoom 偏 ~0.04 ≈ 3%）。
  这是全仓唯一"有 shake 就改变运镜行为"的路径；改它必跑 `--mutate` 与 drop-list 两条守卫。

> 改动只允许落在本目录；跨模块改动先读对方卡，并跑两条门禁。

## 验证

```bash
node scripts/check-yekitrim-translator.mjs            # 339 曲线 + 13 颜色 + 40 映射（含弹幕窗口逐值检查、字幕阴影四键）
node scripts/check-yekitrim-translator.mjs --mutate   # 22 处变异必须**全部打红**
node scripts/check-dhampir-drop-list.mjs           # DROP/CLAMP 清单与基线一致（+ --self-test 自证）

# 端到端烟测（对演示工程；产物落在 target/ 下，不进仓库）
node tools/polish-to-dhampir.mjs docs/wgpu-effect-reference/demo --out target/yekitrim-demo/t.json
```

改了本目录之后另跑两条结构门禁：
`python3 scripts/check_module_map.py && python3 scripts/check_module_boundaries.py`。

**沿革**：这份转译器与它的守卫原先住在 Dhampir 仓（`tools/` 与 `scripts/`），本次交接迁到本仓。
迁移时**只改了注释与报告字符串的视角**（正文里的"本仓"→"底座"，几处"值得回给 YekiTrim"改成
"本仓自己修"），并用演示工程验过**产物逐字节不变**（同一条命令、同一个输出目录下 sha256 相同）。
⚠️ 那次校验需要 `clip.srt`，而它是被 `.gitignore` 忽略的**本地文件** —— 所以这个"整份产物指纹"
只能手工复现，**不进守卫**（否则干净检出里它是空转的）。
