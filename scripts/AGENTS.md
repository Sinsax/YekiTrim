# scripts — 门禁与工具脚本（结构/边界检查、路由对拍、出片报告、贴纸音效目录、ASR 原型资产）

> 范围：`scripts`/** ｜ 上游：改代码的人与 CI（两条 `check_*.py` 是改动的必跑项）｜ 验证：`python3 scripts/check_module_map.py && python3 scripts/check_module_boundaries.py`

## 职责

仓库的**机器判据与大件生成工具**：规矩写成散文会腐坏（后来某次"就这一次"就破防了，不报错、测试也不红），所以把两类硬边界变成能跑的检查；另有一批一次性工具与 ASR 原型。**不是产品代码**，但 `check_module_map.py` / `check_module_boundaries.py` **每次改动都要跑**。

> 历史与实测数据见 **`docs/scripts-gates.md`**（① `gate.mjs` 八条设计口径 / ② 转译器十二段明细与 59 处变异台 / ③ 「出声」契约事故复盘 / ④ 对拍三个坑·M1 四件·黄金帧·两套基线 / ⑤ 效果词汇表 82 项 / ⑥ 预览引擎 19 项 / ⑦ DROP 基线沿革 / ⑧ 契约归一化五步与白名单 / ⑨ 其余沿革）。

## 入口

每个脚本记三件事：**守什么 / 怎么跑 / 缺样本算什么**（"缺样本：—" = 不需要样本，跑不了就是真红）。

### 门禁（改动必跑）

- **`gate.mjs`** —— **统一执行入口**（T5/T8）。**守**：一次改动该跑哪些守卫。
  - **怎么跑**：`--changed [base]`（`git diff --name-only <base>`，默认 `HEAD`；无参数 = 这条）· `--all`（脚本 + 自检/mutate + 结构门禁 + cargo；**默认不跑 cargo**，`--with-cargo` 才跑）· `--check-structure`（只跑两条 python 门禁）· `--list`（只打印会跑什么）。另有 `--files` / `--only` / `--timeout` / `--jobs` / `--logs`。
  - **自动发现**：`scripts/check-*.mjs` + `webui/scripts/check-*.mjs`（node）、`scripts/check_*.py`（python3，**下划线**）；文本里出现 `--self-test` / `--mutate` 就自动多跑一次。映射表在本文件里（每条带"为什么"）。
  - ⚠️ **任何"没跑"都要显式打印 SKIP**（唯一不可妥协的判据）：SKIP 只来自脚本自己打的 SKIP 行 / **退出码 3**（缺样本 = exit 3）/ 超时；**永不算通过**，也不受 baseline 影响。**静默跳过比不跑更糟**（`yekitrim-compare.mjs` 曾漏 `--golden` 而静默换参照，"数字还在阈值内"）。
  - ⚠️ **退出码 = 是否存在非基线失败**（0/1/2）；`gate-baseline.json` **每条必须带非空理由**，**只准登记环境性失败**（判据真红就改代码）。它不写仓库（日志只写 `/tmp`）。
  - **缺样本**：SKIP（见上）；`--all` 每项独立超时。
- **`check-clippy-budget.mjs`** —— **clippy 预算（只降不升）**（2026-11-25，整改 T20a 配套）。
  **守什么**：`cargo clippy --workspace --all-targets` 的 `clippy::*` 告警**去重后**的数量不许超过 `check-clippy-budget.json` 的预算。
  为什么不是直接 `-D warnings`：清零之前塞进去只有两种结局 —— 立刻红一片，或逼人拿 baseline 把红盖住（而 baseline 只准登记环境性失败）。
  所以先立「只降不升」，清到 0 再换成 `-D warnings` 并删掉这两个文件。
  **怎么跑**：`node scripts/check-clippy-budget.mjs`（超预算 exit 1，并打印按类别的增量与前 10 条位置）；
  `--update` 把当前数量写回预算文件（人工确认过再跑）。**去重口径**：按 `code|file:line` 去重 ——
  同一份代码会被 lib / lib-test / bin 各编一遍，cargo 会重复报同一条（实测 20+ 条重复，不去重会把预算算虚高）。
  **缺样本/缺前置**：没有 cargo 或编译失败 ⇒ exit 3（SKIP，永不算通过）。**当前预算 113 条**（从 196 清到 129，去重后 113）。
- **`check-api-contract.mjs`** —— **前后端路由契约差集**（`webui/src/api/*.js` 的 `/api` 调用 ←→ `routes.rs` 注册的 `(method, path)`）。
  - **守什么**：**前向（红）** = 前端调了、后端没注册 ⇒ exit 1（给 `文件:行` / 字面量 / 最接近的已注册路由）；**后向（只提示）** = 后端注册、前端从不调 ⇒ **不红**（CLI / 运维 / SSE / 视图直连都会用到）。
  - **怎么跑**：`node scripts/check-api-contract.mjs` · `--self-test` · `--mutate` · `-v`。它调 `route_manifest.py` 拿路由表（那个 dump 仍 exit 恒 0，本脚本是"调用方"）。
  - **口径**：扫 `/api` 字面量 —— **剥注释、剥查询串**，**插值 → `:x`**（只吞一段），`+` 拼接同一套规则续喂；**对称相交**判据（前向/后向同一个函数）。**白名单只降级、不扩大**（不许吞掉真差集）；另有**交叉验证专抓被注释掉的 `.route(`**（`route_manifest.py` 不剥注释）。五步与逐条理由见 §⑧。
  - ⚠️ **它抓不到**：请求体字段名（46 个 handler 收 `Json<serde_json::Value>`）、method（只提示，axum 回 405）、handler 体 / 中间件 / `.route_layer`（归 `cargo test -p yekitrim-server`）。**缺样本：—**。
- **`check-dhampir-wiring.mjs`** —— **Dhampir 预览接线**。**守**三处**隔着边界**的名字：前端 HTTP 路径 ←→ `routes.rs` 的路由；前端 `/api` 前缀 ←→ `nest("/api", …)`；`engine.js` 的 wasm 导出 ←→ `timeline_host.rs` 的 `pub fn`。**三处都不在编译期报错，错了的症状都是"点了没反应"**（URL 少了 `/api`：146 条用例全绿、只有真浏览器 404）。**怎么跑**：`node scripts/check-dhampir-wiring.mjs`。**缺样本：—**。
- **`check_module_boundaries.py`** —— 边界门禁：**21 条规则**（17 行级 + 3 **成对判据** + 1 **卫生判据**），违规**非零退出**。**怎么跑**：`python3 scripts/check_module_boundaries.py [-v]`。**缺样本：—**。
  ⚠️ 第三类「卫生判据」查**整棵工作区**的属性，不是"某行不许出现 X"：目前一条 —— **工作副本不许有 CRLF**（`git ls-files --eol` 出现 `w/crlf` 即违规）；为什么必须机器拦见 §⑨.1（实测连踩三轮、无任何报错）。
  覆盖：分层方向（`core`/`render-core`/`polish` 不认识上层；`service` 不认识 `server`；`server` 不认识 `cli`；`cli` 只用 server 入口）+ P4 隔离（`downloader` 不开子进程/不认识 B站；`bilibili` 不认识下载器；`net/` 不认识 B站；`download_worker` 不引用 `ingest_worker`）+ 前端（`components` 不 import `views`；查询串只在 `webui/src/api/` 拼）。**加规则**：往 `RULES` 表加 `(规则名, 路径列表, 正则, 为什么)`。
  ⚠️ **"用了 X 就必须同时有 Y"这类文件级约束加进 `PAIRED_RULES`**（`(规则名, 路径, trigger, need, why)`）：行级正则表达不了跨行成对关系（`Agent::new(` 与 `with_cancel(` 不同行）。现有那条是「批量 LLM 循环必须交出取消探针」，判据刻意做粗（文件级）；自证：放一个只含 `Agent::new(` 的文件进 `crates/yekitrim-polish/src/llm/`，应 exit 1。
  ⚠️ 同一张表的 **`need = None`** 是**禁止形态**（命中 `trigger` 即违规）——给「这个文件不许出现对方的领域词」这类**语义隔离**用：整改 T17 拿它拦 `ingest/webhook.rs` 与 `ingest/bilibili.rs` 互相串味（门禁原本只防 import 汇合，于是那个 3235 行的单文件成了**合法绕过通道**）。自证：往 `webhook.rs` 写一句含 `bvid` 的代码，应 exit 1。
- **`check_module_map.py`** —— 模块卡门禁：读 `docs/module-map.json`，校验**覆盖率 / 预算 / 四节骨架 / 索引一致 / ⚠️ 行数下限**，违规非零退出；`coverage_roots` 下**每个源文件都必须落在某张卡的目录内**，**任何没登记进注册表的 `AGENTS.md` 也算失败**。**怎么跑**：`python3 scripts/check_module_map.py [-v|--json]`。**缺样本：—**。
- **`check-llm-models.mjs`** —— 能力表门禁：产物 vs 入库源 `vendor/pi-ai-models/`。**怎么跑**：`node scripts/check-llm-models.mjs [--self-test]`。**缺样本**：缺源 ⇒ SKIP。
- **`check-yekitrim-translator.mjs`** —— 转译器守卫（**node，不是 python**），被测对象 `tools/polish-to-dhampir.mjs`。**守**十二段逐项判据：曲线 / 颜色 / 映射 / 画面构成 / 几何 / 密集键 / 字体族名 / 夹取 / 变速 / TOML 父级 / 素材 / 片头片尾（每段钉什么见 §②）。**怎么跑**：`node scripts/check-yekitrim-translator.mjs [--mutate]`（`--mutate` = 59 处变异逐一真改源码、要求全部打红）。**缺样本：—**；改了 `tools/` 必须跑它。
  ⚠️ **新加的检查要抽成函数并在 `mutate` 里一起调** —— 只写在顶层的检查变异台根本不会跑，于是那条变异"没被打红"（颜色那条第一版就是这么漏的）。
- **`check-effect-registry.mjs`** —— **效果词汇表门禁**：权威 `spec/effects.json`（**11** 个事件类型 × 每通道锚点 + 数值 + 生命周期）；两向判据（登记的锚点必须真的存在、那些文件里不许有没人认领的实现）+ 前端四张表 + LLM 提示词，钉"时长上限/默认跨度"三处同源 + 不变量「每个效果都必须有 `anchors.dhampir_case`」。**怎么跑**：`node scripts/check-effect-registry.mjs [--self-test]`（82 项 + 自检 8 项）。**缺样本：—**。**加删效果先改词汇表，再跑到它绿**。
  ⚠️ 2026-10-01 起**自检本身也进了 `gates`** —— 它曾经烂了很久（断言写死"15 个类型"，而词汇表早已 11 个）却没人发现，原因就是"主检查进了 gates、自检没进"（沿革见 §⑤）。
- **`check-preview-engine.mjs`** —— **精修预览引擎的运行期守卫**：`import` 前端 `webui/src/polish/engine.js`，用**一个事件都没有**的最小工程建索引，再跑预览会调的那族求值器 + 断言 `videoFxAt` 字段齐、`ui` 是数组、**桶清单与 `templates/index.html` 同集合**（19 项）。**怎么跑**：`node scripts/check-preview-engine.mjs [--self-test]`（自检故意删桶，那两族**必须抛**）。**缺样本：—**。**理由**：症状是"预览整块消失、服务端日志全是 200"，静态读源码抓不住（§⑥）。
- **`check-preview-audio.mjs`** —— **精修预览的「出声」契约**：总音量必须落到**所有会出声**的 `<video>`（`dhampirSourceVideos` ∪ `dhampirTwins`，不能只遍历 `dhampirVideos`），`tick()` 的 Dhampir 分支不许漏 `player.sync(...)`；34 项 + 17 处变异自证（含 2026-11-27 增补的「出声⇔正在播放」与「setPlaying 铺状态」）。**怎么跑**：`node scripts/check-preview-audio.mjs`（改了 `PolishPreview.vue` 的播放/音量面就跑它）。**缺样本：—**；事故复盘 ①②③④ 见 §③。
- **`check-dhampir-preview-ring.mjs`** —— 底座预览画字顺序 / 白环宽度（扒 `engine.js::rasterizeLineUncached` 喂"记录型 ctx"）。**怎么跑**：`node scripts/check-dhampir-preview-ring.mjs [--self-test]`。**缺样本**：底座不在 ⇒ SKIP。
- **`check-dhampir-drop-list.mjs`** —— **转译报告清单的门禁**：把演示工程的 **DROP/CLAMP 清单钉成基线**；多了 = 某次改动把一档转丢了，少了 = 修好了（改基线）或**报告被削弱**（更要查）。**怎么跑**：`node scripts/check-dhampir-drop-list.mjs [--require-sample] [--self-test]`。**缺样本**：SKIP + exit 0；`--require-sample` ⇒ exit 3。
  ⚠️ 2026-10-01（B5 文字阴影接入）后基线是 **1 DROP（只剩 `cover`）/ 0 CLAMP** —— 少的那条是**真修好了**（转译器按契约写阴影四键），不是报告被削弱。
  ⚠️ 缺样本时 SKIP + exit 0、输出以 `· SKIP` 明说"没跑"（不许长得像"通过"，E4a）；`--require-sample` ⇒ 缺样本 exit 3；`--self-test` 只测纯函数、不需要样本（放在缺样本检查之前）。
- **`yekitrim-compare.mjs`** —— **两条出片通道的逐步对拍**（本模块最大的一件）。**守**：每窗口的**像素差** / **高频比**（拉普拉斯能量，把"糊"和"锯齿"分开）/ **最佳纵向位移**（非 0 基本是"位置不对"）+ 阈值断言（`baselinePixel 20` / `maxDy 4` / `durationDeltaSec 0.2`）。**怎么跑**：`node scripts/yekitrim-compare.mjs --clip <片段目录|--fixture demo|portrait|real> [--golden <目录>] [--clean] [--self-test]`（参数全表见 §④.2/④.3）。**缺样本**：点名要它 = 报错并说清要准备什么；`--all-fixtures` = 跳过并打印。
  ⚠️ **三个踩过的坑**（详见文件头与 §④.1）：按源时间取帧是错的（`mute_ranges` 会掐掉几段）⇒ 先算映射再取帧；不传 `--clean` 才留产物（否则复核无基准）；用户工程**必须** `try/finally` 恢复。
  ⚠️ 注入 `mute_ranges` 必须写成 `[[mute_ranges]]` **数组表**（转译器 TOML 解析器不支持行内表）；改 `orientation` 必须用 `setTomlKey`（**原工程常没有该键**，replace 会静默不命中）。
  ⚠️ **`--capture-golden` 与"活参照"两条路已作废**：现在**只有 `--golden`**；对活参照**当场报错并指路**（不静默换口径）。
  ⚠️ **两套基线数字别混**（活参照 15.07/9.87/9.77 vs 冻结帧 15.07/**9.90**/**11.80** —— 表在 §④.4）：**对比历史数字前先确认跑的是哪种模式**。
  ⚠️ **B5（字幕阴影）之后重冻过一次** ⇒ 旧冻结帧与新底座不同代；改**出片表现**（模板 canvas）必须重冻 + 重跑三样本。
  ⚠️ 样本是**刚复制**的，`polish init`/`process` 会失败（缺 `data/subtitles.json`）—— 照出片那条路的宽容度，别写成硬失败。
  ⚠️ **`--all-fixtures` 参数走白名单**：当初 `--golden` 不在名单里 ⇒ `all --golden` **静默换回活参照**（数字还都在阈值内 ✗）。加参数时那句"漏了会静默换口径"要一起看懂。
- **`route_manifest.py`** —— 把 axum 路由表 dump 成稳定可 diff 的清单（重构对拍的输入；**exit code 始终 0**，比较是调用方的事；抓不到 handler 体 / 中间件顺序 / `.route_layer` 语义）。**怎么跑**：`python3 scripts/route_manifest.py [--root .] [--out manifest.txt]`。**缺样本：—**（找不到路由就是空清单）。
- **`render_phase_report.py`** —— 出片日志的**阶段报告**（只读）：`render-*.log` 拆成两张表。**怎么跑**：`python3 scripts/render_phase_report.py [日志目录]`。**缺样本**：没日志就是空表、不报错（误差来源见 §⑨.2）。
- **`check-render-visible.mjs`** —— **真渲染 + 数像素**：文字/贴纸画进画面没有（摘掉目标轨重渲，逐帧数差异像素）。**怎么跑**：`node scripts/check-render-visible.mjs`。**缺样本**（工程/二进制）⇒ SKIP。见 `docs/plan/text-layer-wasm-pixel-proof-2026-10-14.md`。
- **`check-dom-camera-parity.mjs`** —— **DOM 腿运镜 = 底座求值器**：逐档位对拍 `dhampir_project_frame().layers[].transform.scale` ↔ `engine.js::camFactor` + "底座确实按 keyframe 插值"的反自证（少了它，相等可能只是巧合）。**怎么跑**：`node scripts/check-dom-camera-parity.mjs [--mutate]`（2 处变异）。**缺样本**（底座不在）⇒ SKIP。⚠️ 整改 D-1 后**降级为容差 ±1px**（归类见 polish 卡「三条预览腿与判据边界」）。

### 一次性工程与原型资产（`asrtest/`）

- 一次性工具（贴纸/音效目录六件套 `build_komori_*.py` / `fetch_komori_cn.py` / `tag_komori_catalog.py` / `subtag_*.py`、`translate_sfx_zh.py`、`rename_emotelab_stickers.py`）与 `asrtest/` 原型资产（`align_srt.py` / `results.json` / `rust_validation.md`）：产出在 `experiments/*.json`，**运行时不需要**；逐条用法与参数见 `docs/scripts-gates.md` §⑨.3。
  ⚠️ 那份 `[asr]` 配置是**当时**的写法（如 `timestamp_alignment`），**现在配置里已无这个键** —— 只当历史参考，别照抄。

## 边界

- ⚠️ **跑法用 `python3`**：本机（以及不少 Windows 环境）`python` **不在 PATH**（旧写法跑不起来）。
- ⚠️ **本目录不只有 python**：`check-yekitrim-translator.mjs` 是 **node**（零依赖 ESM），被测对象 `tools/polish-to-dhampir.mjs` —— **改了转译器就要跑它**；两条 python 门禁看不到那几条曲线。
- ⚠️ **为什么不放进 `crates/xtask`**：`xtask` **刻意不依赖 python / PowerShell**（只用 std + `sha2` + `zip`）。所以**不许往 xtask 里塞脚本调用**。
- ⚠️ **门禁不可绕过**：违规了就改代码，或改规则并写清理由（在同一个改动里说明）。"这次先跳过"正是这两个脚本存在的理由。
- ⚠️ **`check_module_boundaries.py` 必须保留三条处理**：① **先剥注释再匹配**（规则说明自己会写下"不许出现 `yt-dlp`"，不剥注释会全部命中 —— 曾产出 **8 条假阳性**；`(?<!:)//` 不把 `https://` 当注释起点）；② **`std::process::id()` 不算"开子进程"**（只取 PID 命名临时目录）；③ 分层规则用 `(?<!["\w])yekitrim_server::`（`logbus.rs` 有测试写着该字符串字面量，那是数据不是依赖）。
- ⚠️ **加规则前先确认现状是通过的**：**两条刻意没加**的是「`views/` 里不许出现 `/api/` 字面量或 `fetch(`」与「不许用 `scopeQ(`」—— 现状大量不满足，加了就是红门禁，**别盲加**。
- ⚠️ `check_module_map.py` 的判据要点：**覆盖率排除 root 模块**（`.` 算作所有者就兜住一切）；**预算按"最坏链"算**（一次注入根 + 沿路径每级卡，超了**先整份丢掉较宽的文件**）；**索引表用显式标记**；`coverage_roots` 匹配不到目录也失败。
  - **⚠️ 下限**：迁移前根卡有 **117 行**含 ⚠️，拆卡**只许多不许少**（防"清理时顺手抹掉警告"）。
- ⚠️ `route_manifest.py` 的 exit code **始终 0**；它只做结构守卫，不证明行为等价。
- ⚠️ `render_phase_report.py` 的回写**每 30 帧 / 1.5s 一次**，切点是"**最近的一次回写**"（页面循环那段误差 ≤ 1.5s）；日志 **GBK/UTF-8 混合**，解不开退 GBK。
- ⚠️ 小森平（効果音ラボ）：**免费可商用，但禁止直接转载音效 URL** —— 下载到本地缓存；抓取**节流 1s/页**。条款见 `THIRD-PARTY-NOTICES.md` 第五节。
- 本目录多数脚本是**一次性工具**；往这里加新脚本不产生运行时行为，但也**不要**让产品代码 import 它们。

> 改动只允许落在本目录；跨模块改动先读对方卡，并跑两条门禁。

## 验证

```bash
python3 scripts/check_module_map.py && python3 scripts/check_module_boundaries.py
node scripts/gate.mjs --changed        # 按改动面自动选；--all 全量（默认不含 cargo）
node scripts/check-api-contract.mjs    # 前后端路由契约差集（前向红 / 后向只提示）
node scripts/check-dhampir-wiring.mjs
# ⚠️ chrome 那条通道已删除，封面只能用 --golden；缺工程/字体/feature 时 SKIP
node scripts/check-cover-takumi.mjs --golden scripts/fixtures/cover-golden --project <精修工程目录>
```

两条都要求 **0 违规**：前者看模块结构（覆盖率 / 预算 / 骨架 / 索引一致 / ⚠️ 下限），后者看边界（21 条）。加/改规则后**一定重跑两条** —— 判据写错的表现是"看起来全过"，比报错更难发现。
