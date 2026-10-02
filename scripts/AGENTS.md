# scripts — 门禁与工具脚本（结构/边界检查、路由对拍、出片报告、贴纸音效目录、ASR 原型资产）

> 范围：`scripts`/** ｜ 上游：改代码的人与 CI（两条 `check_*.py` 是改动的必跑项）｜ 验证：`python3 scripts/check_module_map.py && python3 scripts/check_module_boundaries.py`

## 职责

仓库的**机器判据与大件生成工具**。规矩写成散文会腐坏（一开始没人违反，后来某次"就这一次"就破防了，而且不会报错、
测试也不会红），所以这里把两类硬边界变成能跑的检查；另外放了贴纸/音效目录的抓取打标脚本与 ASR 对齐原型的资产。
本目录**不是产品代码**，但 `check_module_map.py` / `check_module_boundaries.py` 是**每次改动都要跑的**。

## 入口

### 门禁（改动必跑）

- **`check-dhampir-wiring.mjs`** —— **Dhampir 预览接线**（模块 `dhampir-preview`：WebUI 的 Dhampir 预览接线，
  HTTP 路径 / wasm 导出名 / 引擎方法）。守三处**隔着边界**的名字：前端调的 HTTP 路径 ←→ `routes.rs` 注册的路由；
  前端引用的 `/api` 前缀 ←→ 生产里 `nest("/api", …)` 那一层；`engine.js` 调的 wasm 导出 ←→ `timeline_host.rs`
  里真的 `pub fn`。**三处都不会在编译期报错，而错了的症状都是"点了没反应"** —— 我自己在这条缝里死过一次
  （handler 返回的 URL 少了 `/api`，服务端 146 条用例全绿、只有真浏览器会 404）。

- **`check_module_boundaries.py`** —— 边界门禁：15 条规则（14 条行级 + 1 条**成对判据**；P4 隔离 + 分层方向 + 前端边界），违规**非零退出**。
  覆盖：`downloader` 不许开子进程 / 不许知道 B站；`bilibili` 不许知道下载器；`net/` 不许知道 B站；
  `download_worker` 不许引用 `ingest_worker` / 不许有录播姬语义；`core` / `render-core` / `polish` 不许认识上层；
  `service` 不许认识 `server`；`server` 不许认识 `cli`；`cli` 只用 server 的入口；
  `components` 不许 import `views`；作用域查询串只在 `webui/src/api/` 里拼。
  **加规则的方法：往 `RULES` 表加一条 `(规则名, 相对路径列表, 正则, 为什么)`**（`why` 会在报错时打出来）。
  ⚠️ **"用了 X 就必须同时有 Y"这类文件级约束加进 `PAIRED_RULES`**（`(规则名, 路径, trigger, need, why)`）：
  行级正则表达不了跨行成对关系（`Agent::new(` 与 `with_cancel(` 本来就在不同行）。现有那条是
  「批量 LLM 循环必须交出取消探针」。判据刻意做粗（文件级）—— 目的是别漏掉新的批量循环，不是精确到语句；
  自证方法：临时放一个只含 `Agent::new(` 的文件进 `crates/vtrim-polish/src/llm/`，门禁应当 exit 1。
  用法：`python3 scripts/check_module_boundaries.py [-v]`。
- **`check_module_map.py`** —— 模块卡门禁：读 `docs/module-map.json`（模块 id → 目录 → 卡 → verify），
  校验**覆盖率 / 预算 / 四节骨架 / 索引表一致 / ⚠️ 行数下限**，违规非零退出。
  `coverage_roots`（`crates/*/src`、`webui/src`、`scripts` …）下**每个源文件都必须落在某张卡的目录内**，
  否则报"无主文件"；**任何没登记进注册表的 `AGENTS.md` 也算失败**（卡片会漂移）。
  用法：`python3 scripts/check_module_map.py [-v|--json]`。
- **`check-vtrim-translator.mjs`** —— 转译器守卫（**node，不是 python**），三段：
  **339 项曲线 + 13 项颜色 + 40 项映射**。
  曲线那 339 项钉 `backOut3` 的过冲峰值 1.25、`shakeOffsetAt` 四级阶梯**每档的边界两侧**、
  `handheldSway` 的 2×4 正弦与"线性于 amp"、`stickerStateAt` 的四段相位与那条**够不到的**淡出、
  弹幕在屏窗口的 `danmakuExitMs` 与 Rust 那份 `clamp_display_windows` 的常数对拍；
  颜色那 13 项钉 `hexColor` 四种写法 + 垃圾输入必须 `null`；
  **映射那 40 项**（2026-09-30 M2 加、2026-10-01 D5 补到 27：多了 8 项 `overlayBaseAngle` 角度换算）
  拿一份**合成工程**过一遍 `translate`，钉三类
  "报告里一行、成片里没有"的旧伤：`overlay_type="color"/"fade_in"` 两档、只有 `name` 没有
  `sticker_tag` 的贴纸、以及 `style.camera` 没配时的**按档位默认倍率**（远 0.55 / 近 1.5 / 中 1.3）。
  `--mutate` 再把 **20 处变异**逐一真改源码文本、加载变异体跑**同一份**检查，要求**全部打红**；
  每处变异先断言"替换命中一次"（少了这一条，没匹配上的变异体会原样跑绿、看着像"守卫抓不住"）。
  ⚠️ **新加的检查要抽成函数并在 `mutate` 里一起调** —— 只写在顶层的检查变异台根本不会跑，
  于是那条变异"没被打红"（颜色那条第一版就是这么漏的）。
  用法：`node scripts/check-vtrim-translator.mjs [--mutate]`。改了 `tools/` 必须跑它。
- **`check-effect-registry.mjs`** —— **效果词汇表门禁**（2026-10-01，见 `docs/plan/effect-pipeline-unification.md`）：
  权威是 **`spec/effects.json`**（**11** 个事件类型 × 每通道实现锚点 + 数值 + 生命周期）。两向判据：
  **登记的锚点必须真的存在**、**那些文件里不许有没人认领的实现**（`handlers.*` / `get*` / `case` / `fx.rs` 两张表），
  再对上前端四张表（`FX_TYPES`/`TYPE_LABELS`/`DEFAULT_SPAN`/`SFX_CAPABLE`）、LLM 提示词与参考文档，
  并钉"时长上限 / 默认跨度"三处同源 + **不变量「每个效果都必须有 `anchors.dhampir_case`」**（S4 条件③）。
  **82 项** + `--self-test` **8 项**（多一个 / 少一个 / **少了底座锚点**都必须**点名**报红）。
  ⚠️ 2026-10-01 起**自检本身也进了 `gates`** —— 它曾经烂了很久（断言写死"15 个类型"，而词汇表早已 11 个）
  却没人发现，原因就是"主检查进了 gates、自检没进"。
  **加删效果先改词汇表，再跑到它绿** —— 它替你说出"漏改哪一处"。
- **`check-preview-engine.mjs`** —— **精修预览引擎的运行期守卫**（2026-10-01）：
  `import` 前端那份 `webui/src/polish/engine.js`（纯 ESM、零依赖 ⇒ node 里能直接跑），
  用**一个事件都没有**的最小工程建索引 —— 那正是 2026-10-01 那次事故的触发条件
  （桶是按需创建的，没有该类型事件时 `idx.vignette` 就是 `undefined`），然后逐个跑预览会调的
  那一族求值器 + 断言 `videoFxAt` 的返回字段齐、`ui` 是数组、**桶清单与 `templates/index.html` 同集合**。
  19 项；`--self-test` 故意删桶，要求那两族**必须抛**（判据不许空转）。
  **它存在的理由**：这类崩溃的症状是"预览整块消失、服务端日志全是 200"，静态读源码抓不住。
  用法：`node scripts/check-preview-engine.mjs [--self-test]`（已进 `.workbuddy/gates.mjs`）。
- **`check-preview-audio.mjs`** —— **精修预览的「出声」契约**（2026-10-02）：
  `55f2f16` 让 **Dhampir 默认开启**，而声音那条链上三处仍按"它是可选模式、预览不带声音"写 ⇒
  **预览完全没有声音**（画面完全正常，所以极难归因）。三处 = ① `startDhampir` 建源时
  `v.muted = true`（那批 `<video>` 同时是引擎的帧来源）② `tick()` 的 Dhampir 分支在
  `player.sync(...)` **之前就 return**（"让出重活"让的是**渲染**，音效是**出声**，两件事被混成一件）
  ③ `applyAudio()` 只写 `videoRef.value`（Dhampir 那批源完全打空 ⇒ "音量控制不好使"）。
  ④ 还有一条是修①时**由行为测试实测抓出来的**：`applyAudio` 只遍历 `dhampirVideos` 不够 ——
  换绑中间态里旧元素会**掉出**那个数组却**仍在播** ⇒ **两路同源同时在轨、声音翻倍**；
  判据必须覆盖 `dhampirSourceVideos` ∪ `dhampirTwins`。
  **20 项 + 10 处变异自证**；已进 `.workbuddy/gates.mjs`。
  用法：`node scripts/check-preview-audio.mjs`（改了 `PolishPreview.vue` 的播放/音量面就跑它）。
- **`check-dhampir-drop-list.mjs`** —— **转译报告清单的门禁**（M2，2026-09-30）：把演示工程的
  **DROP/CLAMP 清单钉成基线**（基线里每条都写着"为什么它还在"）。清单变一条就红：
  多了 = 某次改动把一档转丢了；少了 = 修好了（改基线）或**报告被削弱**（更要查）。
  零依赖、不要 GPU ⇒ 进常规门禁。`--self-test` 自证"多一条 / 少一条都必须红"。
  ⚠️ 2026-10-01（B5 文字阴影接入）后基线是 **1 DROP（只剩 `cover`）/ 0 CLAMP** —— 少了的那条
  是**真修好了**（转译器按底座契约写阴影四键），不是报告被削弱。
  用法：`node scripts/check-dhampir-drop-list.mjs [--self-test]`（改了转译器/报告措辞就看它）。
- **`vtrim-compare.mjs`** —— **两条出片通道的逐步对拍**（node，零依赖 ESM；本模块里最大的一件）：
  一条命令跑完整条回路（**取固定样本 → 插测试事件 → 两边出片 → 先算映射再抽帧 → 逐窗口度量 → 判据**），
  对每个窗口量三样：**像素差**、**高频比**（拉普拉斯能量，把"糊"和"锯齿"分开）、
  **最佳纵向位移**（非 0 基本就是"位置不对"而不是"效果不对"）。
  ⚠️ 三个踩过的坑都写在它头部/注释里：① **按源时间取帧**是错的（`mute_ranges` 会掐掉中间几段）——
  它先算映射再取帧；② 结论写完就清产物 ⇒ 复核时基准没了 —— 不传 `--clean` 就都留着；
  ③ 用户工程**必须恢复**（整条流程包在 `try/finally`）。
  **2026-09-30（M1）加的四件**：
  - **固定样本** `--fixture demo|portrait|real`（`--all-fixtures` 依次跑）：一律**先复制到运行目录**，
    绝不动样本本体（这条工具会改 `polish.toml` 并往片段目录写产物）；`media` 用硬链接（真实片段几百 MB）。
    缺样本：点名要它 = 报错并说清要准备什么，`--all-fixtures` = 跳过并打印。
    ⚠️ 样本里注入 `mute_ranges` 必须写成 `[[mute_ranges]]` **数组表**（转译器的 TOML 解析器不支持行内表）；
    改 `orientation` 用 `setTomlKey`（**原工程里常常没有那个键**，只做 replace 会静默不命中 —— 竖屏样本
    跑出横屏数字就是这么来的）。
  - **弹幕带屏蔽**（默认开，`--no-mask-danmaku` 关）：底座那套"头像 + 用户名"弹幕**按设计**与参照不同，
    拿它进阈值 = 每次都被"按设计的差异"判红。不屏蔽的数也照实打印（`pixelFull`）。
  - **阈值断言**（`THRESHOLDS` 在头部，标定依据写在旁边；`--baseline-pixel-max <n>` 覆盖）：
    时长差 ≤ 0.2s（**这条是拿"剪两次"那次事故换来的**）+ 基线中位像素差 + 基线位移 + 逐窗口"高于邻居"。
    超了 `exit 1` 且**报告照留**。
  - **自检扩到 28 项**（`--self-test`）：映射 / 插事件 / 屏蔽带 4 项 / 阈值判定 5 项
    （含"超阈值必须报"的三条 —— 判据写错的表现是"永远绿"，比报错难发现）。
  用法：`node scripts/vtrim-compare.mjs --clip <片段目录|--fixture demo> [--events <json>]
  [--out 目录] [--vtrim <exe>] [--dhampir <exe>] [--translator <路径>] [--clean] [--self-test]`；
  本机跑它的实操包装在 `.workbuddy/run-parity.mjs`（它只管"带 numpy 的 python 在哪、exe 在哪"；
  `demo|portrait|real|all`，`metric <样本>` 用 node 读报告）。
  - **黄金帧模式**（2026-10-01，S4 条件④）：`--capture-golden <目录>` 跑常规回路（用**兜底腿**当参照）
    并把 5 个基线采样帧冻结成小图（`<目录>/<样本>/ref-<t>.jpg` + `golden.json`，约 0.7 MB/样本）；
    `--golden <目录>` 之后**完全不渲染兜底腿** —— 参照只读冻结帧，只做**基线对照**（不许配 `--events`，
    也不做 ±1 帧时间对齐：冻的就是那一帧）。**为什么要它**：这条工具的参照原本就是兜底腿自己
    （`ref.mp4` = `vtrim render --renderer chrome` 的产物），删掉兜底腿会同时失去参照；冻结之后
    删了也能验回归。实测（`demo`）：判据全过、基线中位 **15.07**（与冻结点一致，JPEG 只把均值抬了 0.33）、
    27.8s（比带兜底腿的 ~90s 快），日志里不再有"V-Trim（chrome 后端…）"。
    ⚠️ **2026-10-01 起 `--capture-golden` 与"活参照"两条路都已作废**（它们都要跑那条已删除的兜底腿）：
    现在**只有 `--golden`**，脚本对活参照那条路**当场报错并指路 `--golden`**（不静默换口径）。
    ⚠️ **两套基线数字别混**（同一份代码、两种参照口径）：**活参照**（现渲染兜底腿，基线采样点开 ±1 帧
    时间对齐）`demo` 15.07 / `portrait` 9.87 / `real` 9.77；**冻结帧**（JPEG q2 + 不做时间对齐）`demo` 15.07 /
    `portrait` **9.90** / `real` **11.80**。差在 `real` 最明显（它的底噪本来就只有 ~9，去掉 ±1 帧自由度就涨到 ~11.8），
    两者都远在阈值 20 之下；**对比历史数字前先确认跑的是哪种模式**。
    ⚠️ **2026-10-01 B5（字幕阴影）之后重冻过一次**：canvas 也补了阴影（此前只有 DOM 预览有）⇒ 旧冻结帧与新
    底座不再同代。改**出片表现**（模板 canvas）就必须重冻 + 重跑三样本；重冻后三个数是 15.07 / 9.90 / 11.80。
    ⚠️ 样本是**刚复制**的，`polish init`/`process` 会失败（缺 `data/subtitles.json`）——
    **照着出片那条路的宽容度**：`cmd_render` 里 process 也是尽力而为（失败只 warn），别写成硬失败。
  用法补充：`[--capture-golden <目录>] [--golden <目录>]`；冻结产物随仓交付在
  `docs/golden/dhampir-parity/<样本>/`（`docs/` 不在模块卡的覆盖率范围内，无需登记）。
  ⚠️ **`--all-fixtures` 是靠"再起一个自己"实现的，参数走白名单**：当初 `--golden` 不在名单里
  ⇒ `all --golden` 会**静默换回活参照**（只换口径、不报错，数字还都在阈值内 ✗）。现在两个黄金帧参数
  都在名单里；**往白名单里加参数时，注释里那句"漏了会静默换口径"要一起看懂**。
  **已标定的三个样本**（2026-09-30，判据全过）：`demo` 中位 15.07 / `portrait` 9.87 / `real`（133s 长片）
  9.77，位移全 0、时长差全 0.000 ⇒ 阈值 `baselinePixel 20` / `maxDy 4` / `durationDeltaSec 0.2`。
  配套：台账 `docs/plan/dhampir-parity-register.md`（M1 的实测数字、口径理由与"下一轮做什么"）。
- **`route_manifest.py`** —— 把服务端的 axum 路由表 dump 成稳定、可 diff 的清单。
  用途是**重构对拍**：`server/` 拆 crate 时的验收线是"`(method, path)` 集合前后一致"，
  而 `cargo test` 只能证明 handler 还过、证明不了**没有路由搬走**。它是**源代码级抽取器**（`axum::Router`
  运行时不暴露路由表，另一个办法是写一个重建 router 的测试，那得住在被搬动的 crate 里）。
  **exit code 始终 0** —— 比较是调用方的事（`diff` 两次输出）。**它抓不到** handler 体、中间件顺序与
  `.route_layer(...)` 的挂载语义（那些靠 router 单测）。用法：`python3 scripts/route_manifest.py [--root .] [--out manifest.txt]`。
- **`render_phase_report.py`** —— 出片日志的**阶段报告**（只读）：把 `render-*.log` 拆成
  「墙钟 / 分块渲染 / 抽帧 / 编码 / 固定开销」与「发帧 / 启动 / 页面循环 / 回读」两张表
  （配套文档 `docs/plan/polish-render-speed.md`）。默认目录 `<仓库>/target/debug/.vtrim/cloud/logs`。零依赖、只读。
  用法：`python3 scripts/render_phase_report.py [日志目录]`。

### 贴纸 / 音效目录生成（一次性工程）

- `build_komori_catalog.py` —— 爬小森平（効果音ラボ）全站音效目录 → `experiments/komori_full_catalog.json`
  （`--download` 会顺带下全部音频，**谨慎，~200MB**）。
- `build_komori_cn.py` —— 用中文版页面重建 catalog（中文标题 + `sound_os2` URL）→ `experiments/komori_cn_full.json`；
  缺失中文版的页面（`knocking01`/`pc01`）用日文版条目兜底。
- `fetch_komori_cn.py` —— 爬中文版页面补中文释义 → `experiments/komori_labeled_catalog.json`。
- `tag_komori_catalog.py` → `experiments/komori_tagged_catalog.json`（策展 tag）；`subtag_komori_catalog.py` →
  `experiments/komori_subtagged_catalog.json`（7 大类 → 细分 tag）；`subtag_komori_cn.py` →
  `experiments/komori_cn_tagged.json`。
- `translate_sfx_zh.py` —— 给无官方中文的条目**用 LLM 批量生成中文释义**（输入 `experiments/komori_raw.json`，
  输出 `experiments/komori_raw_labeled.json`）；它读的 LLM 配置是 **`target/debug/.vtrim/llm.toml`**。
- `rename_emotelab_stickers.py` —— EmoteLab 贴纸**批量改名**：① 先把匹配的原件复制到 `<src>/backup/`
  （已存在则跳过、不覆盖）② 原地改名（去 `ysyk_` 前缀与时间戳、内容名空格转 `_`）③ 目标名已存在则追加 `_2`/`_3`。
  用法：`python3 scripts/rename_emotelab_stickers.py [--src <目录>] [--dry-run] [--no-backup]`。

### `asrtest/`（原型资产，不是产品代码）

- `align_srt.py` —— "qwen3 文本 + sensevoice 时间轴"双模型对齐的 Python 原型（LCS 锚点 + 锚点间局部 DP + SRT 断句）。
  跑它需要本地模型 `~/.vtrim/models/{qwen3-asr,sensevoice}/`。
- `results.json` —— 5 模型 × 7 音频的 CER/RTF 回归基准；`rust_validation.md` —— Rust 实施后的**历史验证记录**
  （68.8s 片段，锚点覆盖率 74.8%）；`README.md` 描述资产来源与使用方式。
  ⚠️ `rust_validation.md` 里那份 `[asr]` 配置是**当时**的写法（例如 `timestamp_alignment`），
  **现在配置里已经没有这个键了** —— 只当历史参考，别照抄。

## 边界

- ⚠️ **跑法用 `python3`**：本机（以及不少 Windows 环境）`python` **不在 PATH**（文档里的旧写法
  `python scripts/...` 跑不起来）。
- ⚠️ **本目录不只有 python**：`check-vtrim-translator.mjs` 是 **node**（零依赖 ESM）。
  它的"被测对象"是 `tools/polish-to-dhampir.mjs`，两者是一起搬过来的 ——
  **改了转译器就要跑它**（`node scripts/check-vtrim-translator.mjs [--mutate]`）；
  两条 python 门禁**看不到**那几条曲线，别以为跑完它们就没事了。
- ⚠️ **为什么不把它们放进 `crates/xtask`**：`xtask` **刻意不依赖 python / PowerShell**（整个 crate 只用
  std + `sha2` + `zip`，为的就是把"先跑个包装脚本"那一层去掉）。所以仓库任务**不许往 xtask 里塞脚本调用**，
  门禁脚本留在本目录、用 `python3` 直接跑。
- ⚠️ **门禁不可绕过**：违规了就改代码，或改规则并写清理由（规则变了要在同一个改动里说明）。
  "这次先跳过"正是这两个脚本存在的理由。
- ⚠️ **`check_module_boundaries.py` 必须保留两条处理**：① **先剥注释再匹配**
  （规则说明自己就会写下"本目录不许出现 `yt-dlp`"，不剥注释这些文档行会全部命中 —— 上一版手写的 grep 判据
  就因此产出 **8 条假阳性**，等于没用；`(?<!:)//` 是为了不把 `https://…` 里的 `//` 当注释起点）；
  ② **`std::process::id()` 不算"开子进程"**（它只取 PID 给临时目录命名，`downloader/ytdlp.rs`、
  `bilibili/danmaku.rs` 都用了，没有任何进程被创建）—— 规则认的是 `Command` / `.spawn(`。
  分层规则用 `(?<!["\w])vtrim_server::`（`logbus.rs` 有一段测试专门断言"三个 crate 前缀都要剥掉"，
  里面写着字符串字面量 `"vtrim_server::"` —— 那是数据不是依赖，裸匹配会误报）。
- ⚠️ **加规则前先确认现状是通过的**：**两条刻意没加的**是「`views/` 里不许出现 `/api/` 字面量或 `fetch(`」与
  「`views/` 里不许用 `scopeQ(`」—— 现状大量不满足，加了就是红门禁，**别盲加**；真要收紧得先改代码
  （那是行为改动，不属于"结构标注"这一批）。
- ⚠️ `check_module_map.py` 的几条判据要点（改它之前先懂为什么不这么写会更松）：
  - **覆盖率排除 root 模块**：root 的 `dir` 是 `.`，它"覆盖一切"；算作所有者的话任何没开卡的目录都会被它兜住，
    覆盖率形同虚设。所以只有 `kind != "root"` 的模块算所有者。
  - **预算按"最坏链"算，不是按单文件**：DSH 一次注入的是"根 + 沿路径每一级卡"，总预算 65536 B；
    超预算时 DSH **先整份丢掉较宽的文件** —— 根文件超标会让全局规矩（连模块索引）全部消失，这正是要拆卡的原因。
    所以既卡单文件，也卡链。
  - **⚠️ 下限**：迁移前的根 `AGENTS.md` 有 **117 行**含 ⚠️ 的规则（踩坑换来的），拆卡时**只许多、不许少** ——
    这条防的是"清理文档时顺手抹掉警告"。
  - **索引表用显式标记**：根卡里只有 `<!-- module-index:begin -->` 与 `<!-- module-index:end -->` 之间的表格
    参与校验，免得把状态机表、防线表误当模块行。
  - `coverage_roots` 里某个模式**匹配不到任何目录**也算失败（多半是路径写错了）；
    反向覆盖也失败（有 `AGENTS.md` 没登记注册表、或注册表登记了卡但文件不存在）。
- ⚠️ `route_manifest.py` 的 exit code **始终 0**；它只做结构守卫，不证明行为等价。
- ⚠️ `render_phase_report.py` 的进度回写**每 30 帧 / 1.5s 一次**，所以"发帧+启动/页面循环/回读"三段的切点是
  "**最近的一次回写**"、不是精确边界（页面循环那一段误差 ≤ 1.5s，摊到每帧 < 0.1ms）。
  日志是 **GBK/UTF-8 混合**（Windows 中文），解码失败要退 GBK。
- ⚠️ 小森平（効果音ラボ）站方要求：**免费可商用，但禁止直接转载音效 URL** —— 下载到本地缓存使用；
  抓取**节流 1s/页，请勿加速**。素材出处与条款见 `THIRD-PARTY-NOTICES.md` 第五节。
- 本目录多数脚本是**一次性工具**（爬取 / 打标 / 改名 / 原型），不是 CI 链路的一部分；往这里加新脚本不产生
  任何运行时行为，但也**不要**让产品代码去 import 它们。

> 改动只允许落在本目录；跨模块改动先读对方卡，并跑两条门禁。

## 验证

```bash
python3 scripts/check_module_map.py && python3 scripts/check_module_boundaries.py
node scripts/check-dhampir-wiring.mjs
# 封面通道对拍（冻结参照帧 ↔ Takumi）：--self-test 自证判据；缺工程/字体/带 feature 的二进制时 SKIP
# ⚠️ 2026-10-01 起 chrome 那条通道已删除，只能用 --golden（活的 chrome 对拍已无意义）
node scripts/check-cover-takumi.mjs --golden scripts/fixtures/cover-golden --project <精修工程目录>
```

两条都要求 **0 违规**：前者看模块结构（覆盖率 / 预算 / 骨架 / 索引一致 / ⚠️ 下限），
后者看边界（15 条：分层方向 + P4 隔离 + 前端 + 成对判据）。加/改规则后**一定重跑两条**——
判据写错的表现是"看起来全过"，比报错更难发现；`-v` 会同时列出通过项。
