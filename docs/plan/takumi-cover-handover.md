# Takumi 封面渲染 —— 交接与未完成清单（迁移到 Linux 用）

> 上游结论：`docs/plan/takumi-cover-capability.md`（§0–§14，逐轮证据与复跑命令）。
> 本文只写**交接要用的**：现状、代码落点、Linux 上会变的地方、未完成项、复跑命令。
> 状态：验收线**已达成**（见 §1），本清单里的项都**未做**（会逐条标）。

---

## 1. 现状（在 Windows 上实测通过，数字可复跑）

固定样本 `clip-14`（`[2026-09-14 15-21-56-774][谁想和我甜旮旯]/output/clips/clip-14`），
同一份工程状态、chrome 与 takumi 各出一张 `cover.jpg`：

```
chrome 3142ms / 263185B    takumi 4038ms / 323655B
非文字区：均值 1.17   >32 0.58%     ✅（判据 ≤4 / ≤1%）
整图：    均值 3.50                ✅（判据 ≤6；起点 34.94）
```

逐效果扫描（`sweep-effects.mjs`，以 clip-14 为基座，每变体双通道各一张）：

| 变体 | 整图 | >32 | 非文字区 | >32 |
|---|---|---|---|---|
| base | 2.00 | 1.54% | 0.72 | 0.48% |
| **bg=blur** | **4.98** | 2.61% | 3.95 | **1.00%**（贴边，见 §4-a） |
| bg=image | 3.28 | 2.55% | 1.26 | 0.81% |
| fx=全部六种 | 2.64 | 1.61% | 0.68 | 0.19% |
| fx=glow/blur/vignette/overlay/scan/emoji | 2.24/2.18/1.98/1.80/2.00/2.78 | ≤2.08% | ≤0.88 | ≤0.51% |
| layers=隐藏关键帧层 | 2.00 | 1.54% | 0.72 | 0.48% |

门禁：`check-cover-takumi.mjs --project clip-14` **exit 0** · `check_module_map.py` 35 模块/7 项 ·
`check_module_boundaries.py` 15 条/243 文件/0 · `cargo check`（默认与 `cover-takumi`）**0 warning** ·
桥测试 **7 passed**（含钉住 §5 三个自伤 bug 的那条）。

---

## 2. 代码落点（改哪儿、看哪儿）

| 位置 | 内容 |
|---|---|
| `crates/vtrim-polish/src/render/takumi.rs` | **薄桥 + 全部引擎适配**：`render_cover_png`（唯一入口）、`split_title_layers`（两层同盒 + 可见行高度）、`hoist_frame_transform`、`inline_css_vars`、`ensure_emoji_fallback`、`flatten_compound_selectors`、`adapt_css_for_takumi`（删 `@font-face` / 删 `paint-order`）、`extract_style_blocks`（先剥 HTML 注释）、遥测（HTML/CSS/产物三个短 hash）+ 测试 |
| `crates/vtrim-polish/src/cover.rs` | `build_cover_page_static`（静态那条的构建入口，填 `__EXPORT_CSS__`）、`resolve_cover_renderer`（显式 > `VTEDIT_COVER_RENDERER` > chrome，认不出**报错**）、`cover_font_files`（读 `<exe目录>/resources/fonts`）、`cover_static_capability`（能力判据，只此一份）、`COVER_EXPORT_CSS` |
| `crates/vtrim-polish/templates/cover.html` | 两个静态占位：`:root{…}` 变量块、`__EXPORT_CSS__`（对 Chrome 逐字节中性） |
| `crates/vtrim-polish/Cargo.toml` | `cover-takumi = ["dep:takumi"]`，**默认关**；`takumi 2.14`（`features = ["from-html"]`） |
| `crates/vtrim-cli` | `polish cover --renderer chrome\|takumi` |
| `crates/vtrim-server/src/api/polish.rs` | `POST /api/polish/cover { renderer }`（只透传）；`/api/health.cover_static = {compiled, fonts, fonts_dir, ready}` |
| `webui/src/components/CoverDesigner.vue` | 「生成封面」弹窗选通道；不 ready 时置灰 + 写明原因 |
| `.cargo/config.toml` | `cargo cover` 别名（见 §6） |
| `scripts/check-cover-takumi.mjs` | 双通道门禁；`textRows()` 从 `polish.toml` 算标题矩形（口径修正） |
| `THIRD-PARTY-NOTICES.md` | 「可选 feature `cover-takumi`（**默认关**）」一节（takumi 四件 MIT/Apache、ICU4X **Unicode-3.0**、libwebp-sys BSD-3 + `LNK4098` 说明） |

实验台（**全 node、零依赖**，都不入库的产品代码）：

- `.workbuddy/cover-e2e/*.mjs` —— `run.mjs`（双通道）· `sweep-effects.mjs`（逐效果）·
  `fix-height.mjs` / `fix-balance2.mjs` / `fix-hl-fill.mjs`（三个自伤 bug 的修复+复跑）·
  `probe-*.mjs`（内联语句 / CSS 链路 / transform / 取消描边 / 复刻 / 隔离）· `isolate-renderer.mjs` · `perturb.mjs` · `dump-clip14.mjs`
- `.workbuddy/takumi-spike/` —— 独立 cargo 小工程（`spike`）：`spike.exe <html> <out前缀> <字体…>`；
  另有 `make_html.py` / `make_assets.py` / `sheet.py`（**python**，早期产物）

---

## 3. Linux 迁移：会变的地方（**先看这节**）

1. **字体**：`cover_font_files()` 读 `<exe目录>/resources/fonts`，Takumi **不读系统字体**。
   Windows 上我放的是 `msyh.ttc` / `SimHeiTest.ttf` / `seguiemj.ttf`；Linux 上要换成
   **Noto Sans CJK**（中文）+ **Noto Color Emoji**（彩色 emoji）。实验台脚本里凡出现 `C:\Windows\Fonts\...`
   的都要改（`run.mjs` 的默认 `--font`、各 `probe-*.mjs`、`title-single.mjs`、`calibrate-stroke.mjs`）。
2. **路径**：实验台脚本硬编码了 `E:\media\...`（样本）与 `F:\para\Code\V-Trim\...`（仓库/二进制）。
   Linux 上要么改成参数，要么改环境变量。**样本本身**（clip-14）在 Windows 的 E 盘 —— 需要在 Linux 侧准备等价样本
   （一个带 `polish.toml` + 视频 + 关键帧的精修工程）。
3. **`cargo cover` 别名**：会读 `.cargo/config.toml`，但里面的说明文字提到 Windows 专属警告，无功能影响。用法一样：`cargo cover -- <exe 参数…>`（`--` 必须由你输入，别名里不能再带一个）。
4. **`LNK4098` 不存在**：那是 MSVC 链接器 + libwebp-sys 静态 CRT 的警告；Linux 上不会出现，
   所以 §4-b 里"默认开 feature 会多出警告"这条**只对 Windows**成立，Linux 侧只剩构建时长与许可面两条。
5. **行尾**：`crates/vtrim-polish/src/*.rs` 在 Windows 上是 **CRLF**，Linux 检出通常是 LF；
   我写的补丁脚本都按 `\r\n` 判过（会自适应），但**手工 diff 时注意别整文件重排**。
6. **工具链**：仓库门禁是 `python3 scripts/check_*.py`（文档明确要求 `python3`）；node 脚本零依赖；
   `cargo` 需 ≥ 1.91（Takumi 的 MSRV；Windows 上是 1.97）。`cargo xtask scan` 扫 **release 产物**。
7. **`target/cl`** 是我在 Windows 上为了避开"服务占用 exe"另开的 `CARGO_TARGET_DIR`，**不是仓库约定**；
   Linux 上直接用默认 `target/` 即可。

---

## 4. 未完成项（按优先级，逐条标状态）

- **a. `bg=blur` 的贴边项（4.98 / 1.00%）—— 未做。**
  clip-14 的帧层 `object-fit:contain + scale(1.386)` **盖满画布**，背景层根本没有露出的地方，
  所以 `bg=blur/image` 与 base 数值相同是**符合预期**的，不是一个有效样本。
  要压真实差距：准备一个**帧层不铺满**（竖素材或带留边）的工程，再跑 `sweep-effects.mjs`。
- **b. 是否把 `cover-takumi` 设为默认 —— 待你拍板。** 两种粒度：
  ① 只影响 CLI：`crates/vtrim-cli/Cargo.toml` 加 `cover-takumi = ["vtrim-polish/cover-takumi"]` + `default = ["cover-takumi"]`
  （你的 `cargo run --bin vtrim server` 直接带上，而**库**构建仍精简）；
  ② 全局：`crates/vtrim-polish/Cargo.toml` 的 `default` 里加 `cover-takumi`。
  **选了要配套三件**：改 `THIRD-PARTY-NOTICES` 那节的"默认关"措辞 → 在新 release 产物上重跑 `cargo xtask scan` →
  把"默认构建会多一条 `LNK4098`（仅 Windows）"记进文档。**都未做。**
- **c. 门禁的自包含 fixture —— 未做。** `scripts/check-cover-takumi.mjs` 现在必须 `--project <真工程>`；
  缺样本时它 **SKIP 并打印要准备什么**（`--require` 可把 SKIP 变失败）。要"零外部依赖跑起来"得让脚本自己
  用 ffmpeg 造一个小工程（3s 片段 + `polish.toml`）。
- **d. 完整出片链路未验 —— 未做。** 只验了 `polish cover`；`cmd_render` 末尾也会调 `cmd_cover`。
  若哪天把 takumi 设为默认，**必须跑一次真出片**（现在默认仍是 chrome，无影响）。
- **e. ICU4X 中文断行 —— 未验。** 实测会打 `ICU4X data error: No segmentation model for complex script:
  Chinese/Japanese`（警告级）。封面标题是 `nowrap` + 手动换行所以没暴露；**长文本自动折行**没测过。
- **f. 性能 —— 未解。** 这份工程 takumi **反而慢**（4038ms vs 3142ms）：主因是**图片内联 base64**
  （`from_html` 不吃相对路径，而 `RenderOptions.images` 是 `pub(crate)`，外部塞不进去）。
  唯一没试过的提速路径 = 向上游提 issue 或小 fork 暴露那个字段。
- **g. emoji 配色 —— 未单独量化。** 单点里彩色 emoji 能画（😆，与 Chrome 逐像素一致）；
  真工程里的 🩸 曾出现黑块，现在 3.50 的总读数里它的贡献没有被单独测出来（低优先）。
- **h. `webui/dist` 与卡的同步 —— 视 §4-b 的决定而定**（本轮未改 `webui/src`）。

---

## 5. 已知踩坑索引（每条都有判据，别再犯）

| 坑 | 判据 / 现象 |
|---|---|
| `split_title_layers` 正则 `([\s\S]*?)</div></div>` 把**自己的闭合也吃进去** | dump 里数 `<div>` vs `</div>`（曾 11 vs 9）⇒ HTML 不配平、两层关系全乱 |
| `.hl` 透明色被 `replace` 循环加了 4 遍 | 数 `style="color:transparent" style=`（曾 4）⇒ 非法 HTML |
| wrap 高度把**结尾 `<br>`** 算成一行 | 1020 vs Chrome 的 816 ⇒ 整块下移 102px（**整图 27 → 3.5**） |
| 模板的 `.hl` **自带 `-webkit-text-stroke`** | 双层里两层都要各自处理；漏一层 = 只有高亮词是实心黑块 |
| Takumi 忽略 `paint-order` | 要"填充在上"只能**双层**（描边层在下 + 填充层在上） |
| `text-stroke`（无前缀）**是空操作** | 宽度 6→24 输出完全一样 ⇒ 别拿它当描边（我曾误当"修正"） |
| Takumi **不匹配** `#id.class` 复合选择器 | 最小单点：Chrome A−B=104 vs Takumi 2 ⇒ 适配层要扁平化（背景层可见性就靠它） |
| `from_html` 丢 `<style>` 块 | CSS 必须自己抽出来走 `StyleSheet::parse`；**先剥 HTML 注释**（模板注释里提到了 `<style>`） |
| 变量只在 JS 里 `setProperty` | 静态渲染器不跑 JS ⇒ 要在模板里补 `:root{}`（对 Chrome 逐字节中性） |
| `<img>` 上的 `transform` | 搬给父 `#frame`（实测 img 上不生效） |
| 隔离实验的两个前提 | ① 先剥 HTML 里的原始 `<style>`（含 `@font-face` 会让渲染器报错）② **两侧同一套字体**，否则标题天然差十几个点 |
| 实验台脚本自身的坑 | 正则 strip 会把 `polish.toml` 弄坏（表现：**所有变体同一个数**）；工程里 `effects` 可能是 `[cover]` 下的**内联数组**，只剥 `[[cover.effects]]` 表头会踩 `duplicate key` |

---

## 6. 复跑命令（Linux 版；路径按需替换）

```bash
# 带静态通道（别名在 .cargo/config.toml；-- 之后才是 exe 参数）
cargo cover -- server
cargo cover -- polish cover <工程目录> --renderer takumi
# 等价的长写法（不用别名时）
cargo run --bin vtrim --features vtrim-polish/cover-takumi -- server

# 双通道对拍 / 逐效果扫描（全 node、零依赖；脚本里的样本与二进制路径需要按 Linux 改）
node .workbuddy/cover-e2e/run.mjs    --src <工程> --work <临时目录> --exe <vtrim>
node .workbuddy/cover-e2e/sweep-effects.mjs      # 依赖 run 同样的路径常量
# 官方门禁
node scripts/check-cover-takumi.mjs --project <工程目录>      # 缺样本/字体/带 feature 的二进制时 SKIP
python3 scripts/check_module_map.py && python3 scripts/check_module_boundaries.py
cargo check --workspace --all-targets            # 0 warnings（注意：开 cover-takumi 时 Windows 会多 LNK4098）

# 诊断开关（排查"输入变了但输出没变"这类问题时极有用）
VTEDIT_TAKUMI_DUMP=<目录> VTEDIT_COVER_RENDERER=takumi cargo cover -- polish cover <工程>
#   → 落盘 takumi-input.html / takumi-final.css，并在日志里给 HTML/CSS/产物三个短 hash
VTEDIT_TAKUMI_CSS_PROBE=1 …                      # 往适配后的 CSS 注入一条必然可见的规则（验 CSS 链路）
```

---

## 7. 交接边界（哪些是本会话验证过的）

- **有数字的**：§1 全部读数、§5 每条坑的判据、四条门禁的绿、桥测试 7 passed、`cargo cover` 别名展开正确。
- **没做 / 没验的**：§4 全部（a–h）。特别地：**我只在 Windows 上跑过**，Linux 上第一次跑请预期
  字体路径、样本路径、行尾这三类问题（都在 §3 点名了）。
- **共享工作区注意**：本会话期间还有别的改动在同一仓库里进行（`vtrim-render-core` 的 `entrance` 下线、
  出片 chrome 通道冻结 S4、效果词汇表 `spec/effects.json` + `check-effect-registry.mjs`、dhampir 字体粗体修复等）——
  那些**不是本会话改的**；我改动的文件清单见 §2，冲突时以仓库现状为准。
