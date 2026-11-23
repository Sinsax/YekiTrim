# `scripts/` 卡外迁正文（门禁口径、事故复盘、实测数字与逐项清单）

> 配套模块卡：`scripts/AGENTS.md`（DSH 在触到 `scripts/` 时自动注入）。
> 这份文档**不占指令预算**：卡里只留每个脚本的**三件事**（守什么 / 怎么跑 / 缺样本算什么）与 ⚠️ 行；
> 事故复盘、实测数字、逐项清单、变异台细节与"为什么当初那样写"的叙述全部搬到这里。
> 判断标准：**改代码/加规则前必须知道、否则会写错的留卡；想追根问底时才看的进这里。**
> 相邻文档：`docs/plan/remediation-plan-2026-11-24.md`（T5/T8 的原始需求）、
> `docs/plan/dhampir-parity-register.md`（对拍台账）、`docs/plan/effect-pipeline-unification.md`（效果词汇表）、
> `docs/plan/polish-render-speed.md`（出片阶段报告）、`docs/plan/font-library.md`、`THIRD-PARTY-NOTICES.md`。

分节：

- ① `gate.mjs`：8 条设计口径、SKIP 的三处来源、baseline 规矩
- ② `check-yekitrim-translator.mjs`：四段明细与 59 处变异台
- ③ `check-preview-audio.mjs`：「出声」契约的事故复盘（①②③④）
- ④ `yekitrim-compare.mjs`：三个坑、M1 四件、黄金帧模式、两套基线数字、自检 28 项
- ⑤ `check-effect-registry.mjs`：82 项 + 自检 8 项 + 「自检烂了很久」的教训
- ⑥ `check-preview-engine.mjs`：19 项与 2026-10-01 那次事故的触发条件
- ⑦ `check-dhampir-drop-list.mjs`：DROP/CLAMP 基线的沿革与 SKIP/exit 3 约定
- ⑧ `check-api-contract.mjs`：归一化五步、白名单语义、"它抓不到什么"
- ⑨ 其余沿革：CRLF 卫生判据、出片阶段报告的误差、`asrtest/`、小森平条款

---

## ① `gate.mjs`：8 条设计口径、SKIP 的三处来源、baseline 规矩

它只做一件事：**把"这次改动该跑哪些守卫"变成机器判断，并把"没跑"显式印出来**。
记住下面 8 条，改它的时候不要绕开任何一条（每条都在文件头写了"为什么"）：

1. **脚本自动发现，不写死清单**：`scripts/check-*.mjs`（node）、`webui/scripts/check-*.mjs`（node）、
   `scripts/check_*.py`（python3，注意是**下划线**）。新增一个 `check-*.mjs` 就自动进门禁 —— 手写清单一定会漂。
2. **守卫自检也自动发现**：读脚本文本，出现 `--self-test` 就多跑一次 `--self-test`，出现 `--mutate` 就多跑一次
   `--mutate`。为什么：`check-effect-registry` 的自检曾经烂了很久（断言写死"15 个类型"而词汇表早已 11 个），
   原因只有一句 —— "主检查进了 gates、自检没进"。
3. **任何"没跑"都必须显式打印成 SKIP**（唯一不可妥协的判据）。SKIP 只来自三处：
   ① 脚本自己打印的 SKIP 行（**原样带走脚本自己说的"要准备什么"**）② 退出码 3（本仓约定：缺样本 = exit 3）
   ③ 超时（`--timeout`，默认 300s）。**SKIP 永不算通过，也不受 baseline 影响。**
   教训：`yekitrim-compare.mjs` 曾因参数白名单漏了 `--golden` 而**静默换参照**，"数字还都在阈值内"。
   静默跳过比不跑更糟。
4. **`--changed [base]`（默认 base=HEAD）**：`git diff --name-only <base>` → 按文件里的 `MAPPING` 表选脚本。
   映射表**就写在本文件里**（不另开配置文件），每条都写了"为什么"。只改前端时**不跑 Rust 测试**，
   并且会显式打印 `SKIP cargo test -p yekitrim-server（未触及 crates/）`。
5. **`--all`**：全量（脚本 + 自检/mutate + 结构门禁 + cargo）；**`--all` 默认不跑 cargo**，要跑加 `--with-cargo`
   （`--no-cargo` 强制不带）。每项独立超时，超时按 SKIP 显式打印（不是"通过"）。
6. **`--check-structure`**：把 `python3 scripts/check_module_map.py` 与 `python3 scripts/check_module_boundaries.py`
   包成同一个子动作（T8）。可以和 `--changed` 叠加。
7. **汇总表**：通过 N / 跳过 M / 失败 K（基线 X），每行带脚本名 + 耗时。
   **退出码 = 是否存在非基线失败**（0 = 干净，1 = 有红，2 = 用法/配置错）。
8. **baseline = `scripts/gate-baseline.json`**，形如 `{ "scripts/check-xxx.mjs": "本机底座 host_api 低一版（已知红）" }`。
   **每条都必须带非空理由字符串**（空理由的 baseline 就是第二个"静默跳过"）；
   **只准登记环境性 / 本机不具备的失败** —— 判据真红不许登记，真红就改代码。

其它三条用法：`--list`（只打印"会跑什么 / 为什么"，不执行）、`--files <p1,p2,…>`（手工给改动面，测映射用）、
`--only <子串>` / `--jobs <n>` / `--logs <目录>`。
⚠️ 它**不往仓库里写任何东西**（失败日志只写 `/tmp/yekitrim-gate-logs`）：跑完 `git status` 必须和跑之前一样。
⚠️ **零依赖**：只用 node 内置模块。

## ② `check-yekitrim-translator.mjs`：四段明细与 59 处变异台

**四段与逐项数字**（348 曲线 + 13 颜色 + 44 映射 + 6 画面构成 + 25 几何 + 4 密集键 + 9 字体族名 + 2 夹取
+ 8 变速 + 9 TOML 父级 + 13 素材 + 10 片头/片尾）：

- **曲线项**钉过冲峰值、抖动四级阶梯的**每档边界两侧**、手持 2×4 正弦、贴纸四段相位、
  弹幕在屏窗口（与 Rust `clamp_display_windows` 对拍）；
- **颜色 13 项**钉 `hexColor` 四种写法 + 垃圾输入必须 `null`；**字体族名 9 项**钉「库内 id→真族名、库外原样」；
- **几何**抽 `stageDrawRect` 闭包与模板逐值对拍；
- **映射**拿合成工程过 `translate`，钉三类"报告里一行、成片里没有"的旧伤
  （overlay 两档 / 只有 `name` 的贴纸 / 缺 `style.camera` 的默认倍率）。

**变异台**：`--mutate` 把 **59 处变异**逐一真改源码、加载变异体跑**同一份**检查，要求**全部打红**；
每处先断言"替换命中一次"（少了这条，没匹配上的变异体原样跑绿，看着像"抓不住"）。
**新加的检查要抽成函数并在 `mutate` 里一起调** —— 只写在顶层的检查变异台根本不会跑，于是那条变异"没被打红"
（颜色那条第一版就是这么漏的）。

## ③ `check-preview-audio.mjs`：「出声」契约的事故复盘（①②③④）

`55f2f16` 让 **Dhampir 默认开启**，而声音那条链上三处仍按"它是可选模式、预览不带声音"写 ⇒
**预览完全没有声音**（画面完全正常，所以极难归因）。三处 + 一条实测抓出来的第四条：

1. `startDhampir` 建源时 `v.muted = true`（那批 `<video>` 同时是引擎的帧来源）；
2. `tick()` 的 Dhampir 分支在 `player.sync(...)` **之前就 return**（"让出重活"让的是**渲染**，
   音效是**出声**，两件事被混成一件）；
3. `applyAudio()` 只写 `videoRef.value`（Dhampir 那批源完全打空 ⇒ "音量控制不好使"）；
4. 还有一条是修 ① 时**由行为测试实测抓出来的**：`applyAudio` 只遍历 `dhampirVideos` 不够 ——
   换绑中间态里旧元素会**掉出**那个数组却**仍在播** ⇒ **两路同源同时在轨、声音翻倍**；
   判据必须覆盖 `dhampirSourceVideos` ∪ `dhampirTwins`。

**20 项 + 10 处变异自证**。

## ④ `yekitrim-compare.mjs`：三个坑、M1 四件、黄金帧模式、两套基线数字、自检 28 项

一条命令跑完整条回路（**取固定样本 → 插测试事件 → 两边出片 → 先算映射再抽帧 → 逐窗口度量 → 判据**），
对每个窗口量三样：**像素差**、**高频比**（拉普拉斯能量，把"糊"和"锯齿"分开）、
**最佳纵向位移**（非 0 基本就是"位置不对"而不是"效果不对"）。

### ④.1 三个踩过的坑（也写在它头部/注释里）

① **按源时间取帧**是错的（`mute_ranges` 会掐掉中间几段）—— 它先算映射再取帧；
② 结论写完就清产物 ⇒ 复核时基准没了 —— 不传 `--clean` 就都留着；③ 用户工程**必须恢复**
（整条流程包在 `try/finally`）。

### ④.2 2026-09-30（M1）加的四件

- **固定样本** `--fixture demo|portrait|real`（`--all-fixtures` 依次跑）：一律**先复制到运行目录**，
  绝不动样本本体（这条工具会改 `polish.toml` 并往片段目录写产物）；`media` 用硬链接（真实片段几百 MB）。
  缺样本：点名要它 = 报错并说清要准备什么，`--all-fixtures` = 跳过并打印。
  ⚠️ 样本里注入 `mute_ranges` 必须写成 `[[mute_ranges]]` **数组表**（转译器的 TOML 解析器不支持行内表）；
  改 `orientation` 用 `setTomlKey`（**原工程里常常没有那个键**，只做 replace 会静默不命中 —— 竖屏样本跑出
  横屏数字就是这么来的）。
- **弹幕带屏蔽**（默认开，`--no-mask-danmaku` 关）：底座那套"头像 + 用户名"弹幕**按设计**与参照不同，
  拿它进阈值 = 每次都被"按设计的差异"判红。不屏蔽的数也照实打印（`pixelFull`）。
- **阈值断言**（`THRESHOLDS` 在头部，标定依据写在旁边；`--baseline-pixel-max <n>` 覆盖）：
  时长差 ≤ 0.2s（**这条是拿"剪两次"那次事故换来的**）+ 基线中位像素差 + 基线位移 + 逐窗口"高于邻居"。
  超了 `exit 1` 且**报告照留**。
- **自检扩到 28 项**（`--self-test`）：映射 / 插事件 / 屏蔽带 4 项 / 阈值判定 5 项
  （含"超阈值必须报"的三条 —— 判据写错的表现是"永远绿"，比报错难发现）。

### ④.3 黄金帧模式（2026-10-01，S4 条件④）

`--capture-golden <目录>` 跑常规回路（用**兜底腿**当参照）并把 5 个基线采样帧冻结成小图
（`<目录>/<样本>/ref-<t>.jpg` + `golden.json`，约 0.7 MB/样本）；`--golden <目录>` 之后**完全不渲染兜底腿** ——
参照只读冻结帧，只做**基线对照**（不许配 `--events`，也不做 ±1 帧时间对齐：冻的就是那一帧）。
为什么要它：参照原本就是兜底腿自己（已删），冻结后删了也能验回归。

⚠️ **2026-10-01 起 `--capture-golden` 与"活参照"两条路都已作废**（都要跑已删除的兜底腿）：现在**只有 `--golden`**，
脚本对活参照**当场报错并指路 `--golden`**（不静默换口径）。
冻结产物随仓交付在 `docs/golden/dhampir-parity/<样本>/`（`docs/` 不在模块卡的覆盖率范围内，无需登记）。

### ④.4 两套基线数字别混（同一份代码、两种参照口径）

| 口径 | demo | portrait | real |
|---|---|---|---|
| **活参照**（现渲染兜底腿，基线采样点开 ±1 帧时间对齐） | 15.07 | 9.87 | 9.77 |
| **冻结帧**（JPEG q2 + 不做时间对齐） | 15.07 | **9.90** | **11.80** |

差在 `real` 最明显（它的底噪本来就只有 ~9，去掉 ±1 帧自由度就涨到 ~11.8），两者都远在阈值 20 之下；
**对比历史数字前先确认跑的是哪种模式**。
⚠️ **2026-10-01 B5（字幕阴影）之后重冻过一次**：canvas 也补了阴影（此前只有 DOM 预览有）⇒ 旧冻结帧与新底座
不再同代。改**出片表现**（模板 canvas）就必须重冻 + 重跑三样本；重冻后三个数是 15.07 / 9.90 / 11.80。
⚠️ 样本是**刚复制**的，`polish init`/`process` 会失败（缺 `data/subtitles.json`）——
**照着出片那条路的宽容度**：`cmd_render` 里 process 也是尽力而为（失败只 warn），别写成硬失败。

**已标定的三个样本**（2026-09-30，判据全过）：`demo` 中位 15.07 / `portrait` 9.87 / `real`（133s 长片）9.77，
位移全 0、时长差全 0.000 ⇒ 阈值 `baselinePixel 20` / `maxDy 4` / `durationDeltaSec 0.2`。
配套台账 `docs/plan/dhampir-parity-register.md`（M1 的实测数字、口径理由与"下一轮做什么"）。

### ④.5 `--all-fixtures` 的参数白名单

`--all-fixtures` 是靠"再起一个自己"实现的，参数走白名单：当初 `--golden` 不在名单里 ⇒ `all --golden` 会
**静默换回活参照**（只换口径、不报错，数字还都在阈值内 ✗）。现在两个黄金帧参数都在名单里；
**往白名单里加参数时，注释里那句"漏了会静默换口径"要一起看懂**。

### ④.6 本机实操包装

`.workbuddy/run-parity.mjs`（已 gitignore）只管"带 numpy 的 python 在哪、exe 在哪"：
`demo|portrait|real|all`，`metric <样本>` 用 node 读报告。

## ⑤ `check-effect-registry.mjs`：82 项 + 自检 8 项 + 「自检烂了很久」的教训

权威是 **`spec/effects.json`**（**11** 个事件类型 × 每通道实现锚点 + 数值 + 生命周期）。两向判据：
**登记的锚点必须真的存在**、**那些文件里不许有没人认领的实现**
（`handlers.*` / `get*` / `case` / `fx.rs` 两张表），再对上前端四张表
（`FX_TYPES` / `TYPE_LABELS` / `DEFAULT_SPAN` / `SFX_CAPABLE`）、LLM 提示词与参考文档，
并钉"时长上限 / 默认跨度"三处同源 + **不变量「每个效果都必须有 `anchors.dhampir_case`」**（S4 条件③）。

**82 项** + `--self-test` **8 项**（多一个 / 少一个 / **少了底座锚点**都必须**点名**报红）。
⚠️ 2026-10-01 起**自检本身也进了 `gates`** —— 它曾经烂了很久（断言写死"15 个类型"而词汇表早已 11 个）却没人发现，
原因就是"主检查进了 gates、自检没进"。**加删效果先改词汇表，再跑到它绿** —— 它替你说出"漏改哪一处"。
背景见 `docs/plan/effect-pipeline-unification.md`。

## ⑥ `check-preview-engine.mjs`：19 项与 2026-10-01 那次事故的触发条件

`import` 前端那份 `webui/src/polish/engine.js`（纯 ESM、零依赖 ⇒ node 里能直接跑），
用**一个事件都没有**的最小工程建索引 —— 那正是 2026-10-01 那次事故的触发条件
（桶是按需创建的，没有该类型事件时 `idx.vignette` 就是 `undefined`），然后逐个跑预览会调的
那一族求值器 + 断言 `videoFxAt` 的返回字段齐、`ui` 是数组、**桶清单与 `templates/index.html` 同集合**。
**19 项**；`--self-test` 故意删桶，要求那两族**必须抛**（判据不许空转）。
**它存在的理由**：这类崩溃的症状是"预览整块消失、服务端日志全是 200"，静态读源码抓不住。

## ⑦ `check-dhampir-drop-list.mjs`：DROP/CLAMP 基线的沿革与 SKIP/exit 3 约定

把演示工程的 **DROP/CLAMP 清单钉成基线**（基线里每条都写着"为什么它还在"）。清单变一条就红：
多了 = 某次改动把一档转丢了；少了 = 修好了（改基线）或**报告被削弱**（更要查）。零依赖、不要 GPU ⇒ 进常规门禁。

⚠️ 2026-10-01（B5 文字阴影接入）后基线是 **1 DROP（只剩 `cover`）/ 0 CLAMP** —— 少的那条是**真修好了**
（转译器按契约写阴影四键），不是报告被削弱。

**缺样本的约定（E4a）**：缺样本时 SKIP + exit 0、输出以 `· SKIP` 明说"没跑"（不许长得像"通过"）；
CI 加 `--require-sample` ⇒ 缺样本 exit 3。`--self-test` 只测纯函数、不需要样本（放在缺样本检查之前）。

## ⑧ `check-api-contract.mjs`：归一化五步、白名单语义、它抓不到什么

**前向（红）**：前端调了、后端没注册 ⇒ exit 1，逐条给出 `文件:行` / 字面量 / 最接近的已注册路由。
**后向（只提示）**：后端注册了、前端从不调 ⇒ **不红**。CLI / 运维 / SSE / 视图直连都会用到；
红它就是逼下一个人往白名单里塞东西（那才是把判据废掉的第一步）。

### ⑧.1 归一化五步（唯一有技术含量的部分，别退化成前缀匹配）

1. **先剥查询串**：`/api/polish/project?${qs(…)}` → `/api/polish/project`（`?` / `#` 之后都不属于路径；
   模板串插值常在 `?` 后面，一起被剥掉）；
2. **插值段 → `:x`**：`/api/admin/users/${encodeURIComponent(u)}/quota` → `/api/admin/users/:x/quota`，
   与路由里的 `:name` 段同形；
3. **`:x` 只吞一段**（不跨 `/`）：`/api/jobs/:x` 与 `/api/jobs/:id/cancel` **不相交**；
4. **粘连插值 = 查询串 / 后缀，不是段**：`/api/ingest/records${q}${sep}limit=${limit}`（`q` 是 `''` 或 `'?all=1'`，
   前面没有 `/`）→ `/api/ingest/records`。判据是"插值前面那个字符是不是 `/`"；
5. **`+` 拼接按同一套规则续喂**：`'/api/jobs/' + encodeURIComponent(id) + '/cancel'` → `/api/jobs/:x/cancel`
   （只从以 `/api` 开头的那个字面量起头，链尾的 `'/cancel'` 不算新路径）。

比对用**对称相交**判据（`pathsIntersect`）：static 必须逐段相等，`:x` / `:param` 各自只吞一段，`*path` 吞剩余
（至少一段）。**前向与后向用同一个函数** —— 两个方向问的都是"这两条路径有没有公共的具体路径"，各写一份必然漂移。

### ⑧.2 白名单：只降级、不扩大

每条都在 `WHITELIST` 表里写理由。语义是**只在"这条前端路径没命中任何已注册路由"时把红降成提示**，
**不许**吞掉真正的差集 —— 判据是段级相交、不是前缀，所以 `/api/healthz` 不会被 `/api/health` 吞掉（自证 ⑦ 钉住）。
加条目之前先问一句「它真的调不到吗」：能靠补一条 `api/` 里的封装解决的，就别加白名单 ——
它自己就是那条"门禁不可绕过"的滑坡。

### ⑧.3 交叉验证：被注释掉的 `.route(`

`route_manifest.py` **不剥注释**，所以一条被注释掉的 `.route(...)` 在清单里照样"注册过"。
这个脚本另有一道交叉验证：把 `routes.rs` 里的 `.route(` 出现次数与清单条数对账，**注释掉的也要点数**，
对不上就点名 —— 否则"注释掉一条路由"能同时骗过差集与清单。

### ⑧.4 它抓不到什么（写在这里，免得下次有人以为它证明了更多）

- **字段名**：46 个 handler 收 `Json<serde_json::Value>`，请求体字段写错这条看不见（那是 T9 的事）；
- **method 只做提示**：前端把 method 写在 opts 对象里（`{ method: 'PUT' }`），还有一批"只造 URL、不自己发请求"的
  helper（`paths.status(pid)`）—— method 推定不可靠，所以 method 不符只打提示、**不参与红**
  （axum 会回 405，那是另一档更明显的错误）；
- **handler 体 / 中间件 / `.route_layer` 语义** —— 归 `cargo test -p yekitrim-server`。

## ⑨ 其余沿革：CRLF 卫生判据、出片阶段报告的误差、`asrtest/`、小森平条款

### ⑨.1 为什么"工作副本不许有 CRLF"必须机器拦

`.gitattributes` 的 `eol=lf` **只在检出时生效、不会重写工作区已有文件**，而任何做换行翻译的写入方式
（Python `write_text` 默认就会）都能把工作副本变成 CRLF —— 那会让**变异自证型契约脚本静默失效**
（`.replace` 的锚点含换行，CRLF 下匹配不上 ⇒ 变异不生效 ⇒ 判据不翻面，脚本照旧"通过"），
同时 `webui/dist` 的 chunk hash 整体漂移。**实测连踩三轮、无任何报错。**

### ⑨.2 `render_phase_report.py` 的误差来自"最近一次回写"

进度回写**每 30 帧 / 1.5s 一次**，所以"发帧 + 启动/页面循环/回读"三段的切点是"**最近的一次回写**"、不是精确边界
（页面循环那一段误差 ≤ 1.5s，摊到每帧 < 0.1ms）。日志是 **GBK/UTF-8 混合**（Windows 中文），解码失败要退 GBK。
明细见 `docs/plan/polish-render-speed.md`。

### ⑨.3 `asrtest/` 的历史资产

`results.json` 是 5 模型 × 7 音频的 CER/RTF 回归基准；`rust_validation.md` 是 Rust 实施后的**历史记录**
（68.8s 片段，锚点覆盖 74.8%）；`README.md` 描述资产来源与用法。
⚠️ 那份 `[asr]` 配置是**当时**的写法（如 `timestamp_alignment`），**现在配置里已无这个键** —— 只当历史参考，别照抄。

### ⑨.4 小森平（効果音ラボ）条款全文

站方要求：**免费可商用，但禁止直接转载音效 URL** —— 下载到本地缓存使用；抓取**节流 1s/页，请勿加速**。
素材出处与条款见 `THIRD-PARTY-NOTICES.md` 第五节。
