# Linux 迁移：未完成项 + 交接内容（照着这页调用即可）

> 立档：2026-10-01（Windows 侧最后一轮）。**读法**：先看 §1 建立环境 → 跑 §2 的五组命令对照读数（应当与 §7 的表一致）
> → 按 §3 的未完成项往下做；§4 是**要发给底座会话**的内容；§5 是**要用户拍板**的两件事；§6 是 Linux 上会咬人的平台差异。

## 1. 环境前置（Linux）

```bash
# ① 两个仓并排（本仓的工具按"同层目录"找底座：../Dhampir）
#    V-Trim/ 与 Dhampir/ 必须是兄弟目录
# ② 底座：构建 + 打包它自己的产物（我们只跑它的脚本，不改它源码）
cd ../Dhampir && cargo build --release && node scripts/package.mjs      # 产物默认落在 dist/dhampir-<版本>+<sha>/
#    需要 wasm-pack（wasm32-unknown-unknown target）与 node —— 它会自己报缺什么
# ③ 本仓
cd ../V-Trim
cargo build -p vtrim-cli            # 二进制名在 Linux 上是 target/debug/vtrim（**没有 .exe**）
python3 scripts/check_module_map.py  # 本仓一律 python3（`python` 常常不在 PATH）
node --version && ffmpeg -version | head -1
# ④ 字体：底座**不内嵌字体**，出片/取帧要显式给 `--font-file`
#    Windows 上我们用 C:/Windows/Fonts/msyh.ttc；Linux 上换成任一中文字体（例如
#    /usr/share/fonts/**/NotoSansCJK*.ttc）。产品路径由我们的 Rust 解析（`polish doctor` 会打印它选中的那份）。
```

> ⚠️ `webui/dist/` **已入库**，Linux 上不必重建就能跑服务；改了 `webui/src` 才需要 `npm --prefix webui run build`。
> ⚠️ `.workbuddy/` 是 **gitignore** 的 —— 我这一路用的临时脚本（含聚合门禁 `gates.mjs`）**不会跟过去**。
> 门禁本身都在 `scripts/` 里（会跟过去），聚合命令见 §8。

## 2. 五组命令，重新建立状态（期望读数在 §7）

```bash
# ① 聚合门禁（替代 Windows 上的 .workbuddy/gates.mjs —— 它没入库，这是等价清单）
python3 scripts/check_module_map.py && python3 scripts/check_module_boundaries.py
node scripts/check-dhampir-wiring.mjs          # 期望 48 项全绿（含跨仓 HOST_API_VERSION == 5）
node scripts/check-vtrim-translator.mjs        # 期望 339 曲线 + 13 颜色 + 40 映射
node scripts/check-vtrim-translator.mjs --mutate   # 期望 20 处变异**全部打红**
node scripts/check-effect-registry.mjs && node scripts/check-effect-registry.mjs --self-test   # 82 + 8
node scripts/check-preview-engine.mjs && node scripts/check-preview-engine.mjs --self-test     # 19 + 自证
node scripts/check-dhampir-drop-list.mjs       # 期望 1 DROP / 0 CLAMP
node scripts/vtrim-compare.mjs --self-test     # 期望 24 项 + 屏蔽带/阈值自证

# ② 三样本对拍（**黄金帧模式不需要 Chrome** —— 参照是随仓交付的冻结帧）
node scripts/vtrim-compare.mjs --fixture demo     --golden docs/golden/dhampir-parity
node scripts/vtrim-compare.mjs --fixture portrait --golden docs/golden/dhampir-parity
node scripts/vtrim-compare.mjs --fixture real     --golden docs/golden/dhampir-parity   # 本机才有这个样本，缺了会跳过
#    期望：判据全过，基线中位 15.07 / 9.90 / 11.80（位移 0、时长差 0.000）
#    ⚠️ 别用 `--all-fixtures --golden`（历史坑已修，但单跑更好定位）
#    ⚠️ `--fixture` 需要一个真实片段目录或仓库自带样本；`real` 是本机本地目录，Linux 上大概会跳过

# ③ 测试
cargo test -p vtrim-polish --lib    # 期望 212 passed
cargo test -p vtrim-render-core --lib
cargo test -p vtrim-server
cargo check --workspace --all-targets   # 期望 0 warnings

# ④ 产品路径出片（底座那条腿）：拿一个真实片段目录
target/debug/vtrim polish render <clip 目录> --renderer dhampir
#    ⚠️ 底座 CLI 的 `render` 若要自己混音轨，会去找它自己的 target/s3 媒体 ⇒ **必须由产品路径调用**
#       （我们的 Rust 只让底座出画面、音轨归自己）。直调底座时记得加 `--no-audio`。

# ⑤ 预览（可选）：起服务后页面上按"底座预览"；dev 树需要底座预览产物
node tools/polish-to-dhampir.mjs <clip 目录> --out /tmp/p.doc.json    # 转译器单独跑（零依赖）
```

## 3. 未完成项（只有 2 条，且都在我们边界之外）

### 3.1【底座侧挡着】背景层（竖屏模糊底）没被画出来 —— 交接单 **§0b**

- **我们侧已完成**：`tools/polish-to-dhampir.mjs` 里那三步配方（`bg1` 同源 cover 层 + `gaussian_blur`、
  `bg2` 调整层做 `contrast(0.5)+brightness(-0.25)` 表达 CSS 的 `brightness(0.5)`）**已实现**，开关 `BACKDROP_ENABLED` 目前 **false**。
- **实测（可复算）**：竖画布 + 横素材 + `[stage] position="center"` 的工程 ⇒ 转译产物的绘制矩形是
  `0.0,656.3 1080.0x607.5`（`axis=y / fit 0.5625`）、`bg=blur`；带那两层与**整条删掉**各渲一次 ⇒ **成片逐像素相同**
  （本该是纯黑的上边带两次都是 `rgb(243,243,243)`）⇒ **底座收下了轨但静默没画**。
- **完成判据**：底座把底画出来（上边带变成模糊画面而不是黑）+ 产品路径出片里肉眼可见。
- **修好之后我们要做的（一行 + 一轮）**：
  1. `tools/polish-to-dhampir.mjs` 把 `const BACKDROP_ENABLED = false;` 改 `true`；
  2. 重跑三样本对拍（`--golden`）并**重冻**黄金帧（`--capture-golden`，这条**需要 Chrome**）；
  3. 如实记一条 **CLAMP**：底座 `gaussian_blur.radius` 上限 **16（σ8）**，参照模糊底要 **σ28** ⇒ 只能做到一半。
- **别踩**：`radius` 不叠加（`blur_radius()` 取多模糊里的最大值），"多叠几趟凑 σ28"行不通。

### 3.2【有过一次误判，已撤回，勿重做】"常驻第二层长片必 OOM" —— 交接单 **§0**

- 旧 Windows 版本上实测：注入一个常驻第二层 ⇒ 1100 帧过、**1500 帧 OOM**（每帧 16–33MB 累积）。
- **2026-10-01 在 `c7f29b9` 上新底座复测：200 / 1100 / 1500 帧全部渲完、无 OOM** ⇒ 这条**已降级为"请顺手看一眼中间纹理有没有复用，但不阻塞"**。
- **别把 §0 当成 §0b 的原因**：§0b 是"画不出来"，与 OOM 无关。

### 3.3 已知残差（**不修**，底座文档已写明）

- **B3**：高亮词段间水平推进量不同源（残差几像素，中英混排可见）。
- **弹幕阴影**：底座契约里没有弹幕的 `shadow_*`（只有字幕）；我们 canvas 也没画 ⇒ 只有 DOM 预览有那层 `.dm` 阴影。
- **弹幕逐条颜色 `\c`**：底座**能**显示了，但**我们故意不发** —— 参照（本仓 canvas）只读轨道色，
  发了会让成片比参照更花（历史上真踩过：48.8s 那帧 V-Trim 白 / 底座品红）。理由写在 `tools/polish-to-dhampir.mjs` 的 `stripColorTags` 上方，**别顺手打开**。

## 4. 交接给底座会话（照发，材料都在这两个文件里）

1. **主件**：`docs/dhampir-base-handoff.md` —— **整份可直接贴**（它是自包含的：每条都有最小复现或可复算读数）。
   本轮新增/变化的重点是：
   - **§0b（新，最要紧）**：背景层被收下但没画 —— 含我们的 doc 形状（`bg1/bg2` + `radius=16`）、绘制矩形、
     上下边带像素读数、以及"我们按你们 `effect_would_be_ignored` 的校验把色彩拆到调整层"的说明。
   - **§0（已降级）**：OOM 在新版本上复现不出来 ⇒ 只请顺手确认。
   - **§1 D1（已由我们撤回）**：原复现 JSON 是我们的错（缺 `timeline.schema`；`end` 超素材帧数）；
     底座在 `c7f29b9` 上是正常的（我们独立复算：窗口内差 39.33、窗口外逐字节相同、`angle=45` 左上=color_a/右下=color_b）。
   - **B1**：`contain` 摆位**我们侧早已实现**（别在底座加概念）；只差背景层（见 §0b）。
   - **B2**：已按他们的可复算判据撤回（1 像素棋盘放大 ⇒ 100% 中间值 ⇒ 是双线性）。
   - **B4**：我们按他们的配方补了（静态 cue 发零位移 `\move(…,0,travel)`；我们素材其实不出 `\pos`，属防御）。
   - **D 组**：D1 撤回 / D2·D3·D4 记"已处置"（D4 我们侧还加了不变量：纯色档 `r2==r && g2==g && b2==b`）。
2. **版本面**（对齐口径，勿改口径）：`HOST_API_VERSION` 现在是 **5**（wasm 返回体加了 4 个阴影键 ⇒ 形状变更）；
   `LAYER_SCHEMA_VERSION` 4 / `PROJECT_SCHEMA_VERSION` 1 不变。我们**已把它做成门禁**（见 §2 ①，跨仓对拍，底座升版我们没跟就会红）。

## 5. 需要用户拍板的两件事

1. **S4：要不要删掉第二条渲染链路**（canvas 出片 + DOM 预览腿）—— 结论书在
   `docs/plan/s4-second-chain-removal-decision.md`。
   **现状**：原先的硬前提 A（底座 D1 色罩不出图）**已于 2026-10-01 作废**（复核：底座出图正常），
   所以**只剩产品取舍 B**："机器上没装底座 ⇒ 预览没有任何兜底" ⇒ 选 **B1**（接受"没底座就没预览"）
   还是 **B2**（只删出片那条腿、保留 DOM 预览腿）。**拍板后**才动 §4 那份删除清单（7 步，文件级，含每步验证与回滚点）。
2. **B5 成片观感**（已落地，请看一一眼）：字幕现在带一层**柔和阴影**（此前**只有预览有**、成片没有 ⇒ 两条腿不一致）。
   很轻（描边本来就厚）。想调就改**一对常量**：`templates/index.html` 的 `shadowBlur = 12` / `rgba(0,0,0,0.4)`
   ↔ 转译器的 `12 / height` / `[0,0,0,0.4×255=102]`（守卫把这两处钉在一起，**改一处就会红**）。

## 6. Linux 平台差异清单（会被咬的地方）

| 项 | Windows（现状） | Linux 上要改成 |
|---|---|---|
| 二进制名 | `target/debug/vtrim.exe` | `target/debug/vtrim`（**没有扩展名**）—— 我留在 `.workbuddy/` 里的临时脚本都硬编码了 `.exe`（那些脚本不入库，直接丢掉/改掉即可） |
| python | `.workbuddy/gates.mjs` 里写了 `C:/Users/para/.dsh/.../python.exe` | 用 `python3`（§2 ① 的清单就是等价物） |
| 字体 | 脚本里写死 `C:/Windows/Fonts/msyh.ttc` | 换 Noto/思源等中文字体路径；**产品路径不用管**（我们的 Rust 自己解析，`polish doctor` 打印它选的那份） |
| Chrome | `check-*.mjs` 里按 `where chrome` 找 | 找 `google-chrome` / `chromium`（代码里已有 Linux 分支；**黄金帧模式不需要 Chrome** ✓） |
| 底座产物目录 | 我们曾只找 `out/`，而底座默认出 `dist/dhampir-*` | **已修**（`crates/xtask/src/main.rs::newest_dhampir_dist` 先取 `dist/` 最新那份）；里面还留了一个 `F:/para/Code/Dhampir` 的兜底常量，Linux 上无用但无害 |
| 硬链接/拷贝 | 我在 `cmd /c mklink /H` 上栽过（PowerShell 的 `-LiteralPath` 不展开通配符也栽过） | Linux 上用 `cp --reflink=auto` 或直接 `cp -r`；**别用 `-LiteralPath '…\*'` 那种写法** |
| 换行 | 仓库里有 CRLF 文件 | 我的经验：**多行字符串锚点会失配**（要按行号 splice 或单行锚点）—— 你们那边若无此问题，忽略 |

## 7. 已落地内容与实测读数（口径对照，别重做）

| 项 | 读数 / 判据 |
|---|---|
| 底座版本 | `Dhampir c7f29b9`；产物 `dist/dhampir-1+c7f29b9/`；`bin/dhampir.exe` sha256 `4b5ee233acccf7b740dde31875eea25e3e72a57ff1659c55286d3ef19268eda3`（Windows 构建）；`HOST_API_VERSION = 5` |
| P1 版本断言 | 前端 `DHAMPIR_HOST_API = 5` + 加载 wasm 后运行时断言；门禁 40 → **48** 项（含跨仓对拍） |
| P2 D1 | **撤回**：窗口内差 39.33 / 窗口外逐字节相同 / `angle=45` 左上 `rgb(255,0,0)`=color_a、右下 `rgb(0,0,255)`=color_b；底座 `handoff-probe.mjs` 21/21 |
| P3 B5 阴影 | 转译器四键 `[0,0,0,102] / 0 / 2 / 12÷1080`；canvas 补画；守卫 29→**40** 映射、17→**20** 变异；**DROP 2 → 1**；底座腿 RGBA 差 10,769 字节（y=912..1012）；chrome 腿 A/B 348 像素（y=773..1036） |
| 三样本（黄金帧口径） | demo **15.07** / portrait **9.90** / real **11.80**，位移 0、时长差 0.000（冻结帧已按 B5 重冻一次） |
| 活参照口径（历史值） | demo 15.07 / portrait 9.87 / real 9.77 —— **两套别混**（阈值 `baselinePixel 20` / `maxDy 4` / `durationDeltaSec 0.2`） |
| 测试 | polish **212** / render-core 30 / server 144（Windows 上最后一次读数）；`cargo check --workspace --all-targets` 0 warnings |
| 发布形态（早前一轮） | `cargo xtask package --smoke-test` ⇒ 包内 5 项齐、doctor 四条路径在包内、`render --renderer dhampir` 出片；zip sha256 `db67e3f19055051c79bee39f7d314c1a68a3fbe4285926441a2730411811d293`（**Windows 构建**，Linux 上要重打） |

## 8. 门禁命令清单（Linux 上直接用）

```bash
python3 scripts/check_module_map.py                 # 结构：覆盖率/预算/骨架/索引一致/⚠️ 下限
python3 scripts/check_module_boundaries.py          # 边界：15 条（分层 + P4 隔离 + 前端 + 成对判据）
node scripts/check-dhampir-wiring.mjs               # 接线（含跨仓 HOST_API_VERSION）
node scripts/check-vtrim-translator.mjs [--mutate]  # 转译器曲线/颜色/映射 + 变异自证
node scripts/check-effect-registry.mjs [--self-test]# 效果词汇表 + 自检
node scripts/check-preview-engine.mjs  [--self-test]# 预览引擎（真跑一遍，抓"预览整块消失"那类）
node scripts/check-dhampir-drop-list.mjs [--self-test]  # DROP/CLAMP 基线
node scripts/vtrim-compare.mjs --self-test          # 对拍工具自证
node webui/scripts/check-overlay.mjs                # 出片模板 ↔ engine.js 的漂移对拍（另有 4 个 check-*.mjs）
cargo check --workspace --all-targets && cargo test -p vtrim-polish --lib -p vtrim-render-core --lib -p vtrim-server
```

> ⚠️ Windows 上我用的聚合脚本 `.workbuddy/gates.mjs` **不入库**；上面这份清单就是它的全部内容（外加两条 python 门禁）。
> 建议：迁移后在 Linux 上把它落成 `scripts/gates.sh`（或直接照上面手敲），省得每次拼十条命令。

## 9. 还没提交的东西（迁移前先处理掉）

- 工作树**未提交**，且跨了多个目标（效果链路统一收尾、类型收敛、S4 决策书、B5/B4、P6 立档、Linux 这份）。
  建议按主题分几个 commit：① 底座新版本对齐 + 版本断言 + xtask 产物发现；② D1 撤回 + S4 决策书更新；
  ③ B5 文字阴影（转译器 + canvas + 守卫 + 基线重冻）；④ B4 防御 + P5 台账；⑤ P6 与 §0b 立档；⑥ 本文件。
- `webui/dist/`（已重建）、`crates/vtrim-polish/templates/index.html`（加了阴影）都要一起进。
- `.workbuddy/` 与 `target/`、`release/` 都不入库（Linux 上自然没有）。
