# Dhampir 出片通道 · 问题台账与长线目标

> 立档：2026-09-30（用户拍板：默认通道已切到底座，接下来按台账逐项修）。
> 配套：`docs/plan/dhampir-cli-server.md`（接口与阶段计划）、`docs/vtrim-integration.md`（转译器的规格与已知差异）、
> `crates/vtrim-polish/src/render/AGENTS.md`（出片链路的硬规矩）。


### ✅ 已定性并修好判据：`portrait` 的"基线位移 −10px"**不是几何差**（2026-10-01）

**结论**：那是**判据在"目标函数平坦"时仍然下了结论** —— 竖屏样本的两条腿差异主要来自**色调**
（它的参照产物是**更早的历史渲染**：没有当前样本的字幕、整体偏紫），于是"最佳纵向位移"落在噪声里。
三个样本复测（判据修好后）：

| 样本 | 基线中位像素差 | 位移（只用可识别的点） | 判定 |
|---|---|---|---|
| `demo` | 15.07 | 0px | ✓ |
| `real` | 9.77 | 0px | ✓ |
| `portrait` | **9.87** | **0px**（5 点里 2 点判"不可识别"被排除） | ✓ |

**怎么排除"几何差"的**（都做了实测）：

1. **摆放几何那几个关键函数本次改动一行没碰** —— `git diff` 在 `templates/index.html` 里
   对 `stageDrawRect` / `resolve_stage` / `sr.x|sr.y` / `axis` **零命中**。
2. **残差曲线是平的** —— 自己逐采样点量过（ffmpeg `psnr` 扫 ±20px）：最佳位移与 0 位移的 PSNR
   只差 **0.00 ~ 0.05 dB** ⇒ 那个"位移"不可识别（不是"差 10px"，是"没有信息"）。
3. **同代码下 demo / real 的 dy 都是 0** —— 而 `demo` 里既有 vignette 又有线性渐变 overlay
   ⇒ D4/D5 那两处改动不可能制造竖屏专属的平移。
4. 判定性实验：往被摘掉的 `color_shift@4.8` 窗注入 dummy 事件让采样点挪开，位移**仍是 −12px**
   ⇒ 也不是"参照里留着已下线效果"造成的。

**判据的修法**（`scripts/vtrim-compare.mjs`）：

- `best_dy` 现在**连 dy=0 的残差一起回传**（`residual0`）；新增纯函数 `identifiableDy(residual0, residualBest)`：
  要求**相对改善 ≥ 1% 且绝对改善 ≥ 0.02**，够不上就判 **dy = 0**（原始值留在 `dyRaw`）。
- 组级位移**只统计可识别的点**（一个都没有就判 0），报告里带 `dyN` = 参与点数。
- 抑制发生时**打印**：`⚠ 位移不可识别（残差曲线平坦）⇒ 判 0 的有 N 处：那不是几何差…`
- 自检加了 5 条（含"过度抑制"的反例：改善 60% 必须仍算识别、绝对改善 0.01 必须不算）。

⚠️ **这条改动的理由**：在噪声上宣布"几何差"比漏报更糟 —— 它会让人去查一个不存在的摆放 bug
（本轮就差点这么干）。**口径变松的地方都配了"照实打印"**，不是把数据藏起来。

## 一、怎么判定"是谁的问题"（每次都走这三步，别拍脑袋）

1. **看转译器报告**：`node tools/polish-to-dhampir.mjs <片段> --out x.json --keep-muted`
   → `DROP` = 我们没转（本仓）；`CLAMP` = 转了个近似（本仓要写清近似了什么）。
2. **报告说转了、成片却没有** → **绕开 V-Trim**，直接让底座渲染同一份 doc：

   ```bash
   <Dhampir>/target/release/dhampir.exe frame --project x.json --frame <N> --out out/ \
     --asset-root <片段目录> --font-file <字体> --font-dir <字体库>
   ```

   仍然不对 ⇒ **底座侧**（这时最小复现就是证据，见 §四）；对了 ⇒ 我们驱动/参数的问题。
3. **参照永远是 `crates/vtrim-polish/templates/index.html` 的 canvas 出片路径** ——
   不是 DOM/GSAP 预览，也不是底座的预览。

## 二、已修（本仓，2026-09-30，全部有实测数字）

| # | 症状 | 根因 | 证据 |
|---|---|---|---|
| V1 | 出片比 chrome **时长短**、画面/音轨错位 | 转译器先掐掉 `mute_ranges`，`cmd_render` 尾部**又剪一遍** | 12s 带 3s 屏蔽：修前 **6.016s/359 帧** → 修后 **9.002667s/538 帧**，与 chrome 的 9.002667s 逐毫秒一致 |
| V2 | 只出一半长度 | `cmd_render` 里 `fps` **写死 30**，工程时间基是素材的 60 | `frames 0..359` → `0..719` |
| V3 | **字体/字体样式不对** | `find_font()` 写死 `msyh.ttc`，工程选的字体没进底座 | 日志 `font=…\resources\fonts\SimHeiTest.ttf`；`--font-dir` 一并交给底座 |
| V4 | 源码树下**找不到底座** | "约定位置不存在"被当成硬错误，回退链一次都没跑到 | `dhampir=<找不到：…\dhampir\bin\dhampir.exe>` → 现在找到同层 `Dhampir/target/release` |
| V5 | 合流报 `No such file`（成片路径拼两遍） | 相对工程目录 + `resolve_paths` 已归一成 `<dir>/export.mp4` | CLI 默认输入就是 `.`，必踩；`cmd_render` 开头 `std::path::absolute` |
| V6 | 成片**少了整块渐变色罩** | 转译器只认 6 位色，`#140A28CC` 解析失败 ⇒ `drop` 掉整条 overlay | 报告 `丢失 11` → **6**；overlay 层 0 → **1** |
| V7 | 构图/取景与 chrome 不一致 | camera 算不出锚点就整条丢；参照模板有写死的 `65% 50%` 兜底（`origin = "center"` 经 `parseFloat` 也落这里） | 运镜 **0 → 3 条**；t=1.0s/5.0s 抽帧构图一致 |
| V8 | WebUI 出片进度条全程不动 | 底座的 NDJSON 没翻成上游认的 `frames A..B: NN%` | 实测 0→20→42→61→80→100%，每 ~1.5s 一条 |
| V9 | （承上）默认通道接线 | `--renderer` 默认、服务端白名单透传、前端下拉、不可用时报错而不是静默换路 | `cargo test -p vtrim-server` 143 passed（含"通道认不出 → 400"） |

## 三、按设计不修（写在这里，免得以后被当 bug 反复查）

| 项 | 口径 |
|---|---|
| **弹幕排布**（底座那套：头像 + 用户名列表） | **用户 2026-09-30 拍板：按当前设计**。不再往 chrome 的滚动弹幕上对 |
| 成片帧率 60（chrome 那条腿 30） | 底座按**素材时间基**出片，时长一致；60 更贴源 |
| `vignette` 不进成片 | **参照自己的出片路径里就没有它**（只有 DOM 预览有）——不替参照补画 |
| 画面归底座、音轨归本仓（`--no-audio`） | 通道分工，两版成片的音轨都出自本仓 |
| 字体必须显式给文件 | 底座不猜系统字体；本仓负责解析出文件（V3 已接） |
| 封面不接底座 | 已决策，见 `docs/plan/cover-dhampir-tradeoff.md` |


### 两套基线数字（**同一份代码、两种参照口径**，2026-10-01）

| 样本 | **活参照**（现渲染兜底腿；基线采样点开 ±1 帧时间对齐） | **冻结帧**（黄金帧：JPEG q2、不做时间对齐、**完全不渲染兜底腿**） |
|---|---|---|
| `demo` | 15.07 | **15.07** |
| `portrait` | 9.87 | **9.90** |
| `real`（133s） | 9.77 | **11.80** |

- 位移全 0、时长差全 0.000，两种口径**都远在阈值 20 之下**。
- ⚠️ **2026-10-01 重冻过一次**：B5 给 canvas 补了字幕阴影（此前只有 DOM 预览有）⇒ 旧冻结帧（改前）
  与新底座（已带阴影）不再同代。重冻后三样本 15.07 / 9.90 / 11.80，与旧值只差 0.03~0.04 ——
  正是"两条腿都多了同一层阴影"应有的样子（不是回归）。**改 canvas 的表现就必须重冻 + 重跑三样本。**
- `real` 差得最明显（+1.99）：它本来底噪就只有 ~9，去掉 ±1 帧对齐那点自由度就涨到 11.76 —— **不是回归**。
- **对比历史数字前先确认跑的是哪种口径**：命令里带 `--golden` 就是冻结帧那一列。
- ⚠️ **坑（已修）**：`--all-fixtures` 是"再起一个自己"，只转**白名单**里的参数；`--golden` 当初不在名单里
  ⇒ `all --golden` 会**静默换回活参照**（portrait 报 9.87 而不是 9.84，两边都在阈值内 ⇒ 不报红，只会让人拿错基线比）。
  现在 `golden` / `captureGolden` 都在白名单里（`scripts/vtrim-compare.mjs`）。

## 四、底座侧（本仓只交接 + 跟进，不在本仓改）

| # | 问题 | 证据（最小复现） |
|---|---|---|
| ~~D1~~ | ~~**overlay 渐变色罩不生效**~~ → ✅ **2026-10-01 已撤回**（复核见下"§四 D1 撤回"）| 缩到"只有视频层 + 一条 overlay 层"的工程，用底座自己的 `frame` 出第 60 帧（窗口 30–126 之内）**仍无色罩**：`.workbuddy/dhampir-unify-out/mini-overlay.doc.json` + 上面的命令。发出去的字段正是底座映射代码读的那套（`timeline.rs:362` 读 `amount/r/g/b/r2/g2/b2`），`Effect.opacity` 缺省 1.0，层在视频轨之后、窗口覆盖该帧 |
| D2 | `--from` 单独给时**文档与代码不一致** | 注释说"到工程结尾"，代码 `to = args.to.unwrap_or(from)` = 只出一帧（`dhampir.rs::frame_range`）。我们两头都给所以没踩到。✅ **2026-10-01 底座已改实现**（`frame_spec` + 纯函数 `resolve_frames`：只给一头就按工程补全；互斥用法报用法错），并加了第 33 条 CLI 判据 + 变异验证（旧行为下 32/33 红）|
| D3 | `DanmakuSpec.duration_ms` 模块头注释与 `layout` 实现相反 | ✅ **2026-10-01 底座已按实现补全注释**（逐条 `travel_ms` 优先 → 回退轨道级 `duration_ms`；素材 `End` 不参与；静态 `\pos` 也走回退）。原复核笔记：看起来已在底座侧自行修正 —— `crates/dhampir-timeline/src/danmaku.rs` 里有多处"这条先前写错了"的更正注释，实现是 `cue.travel_ms.unwrap_or(spec.duration_ms)`（151 行）⇒ 只需确认模块头那句是否同步 |
| **D4** | **底座自己发现**的一致性问题：`overlay.shape = 0`（纯色）取的是 `r2/g2/b2` 而不是 `r/g/b` | ✅ **底座已改实现**（改成取 `color_a`，并补了"故意 `r2≠r`"的真机判据）；对**我们零影响**（`solidEffect` 写的本来就是 `r2: col.r`）。⚠️ 底座提醒：将来若在 `shape=0` 时写 `r2≠r`，画面会与从前不同 ⇒ **本仓补了不变量**：`scripts/check-vtrim-translator.mjs` 的映射段断言纯色档 `r2==r && g2==g && b2==b`（+1 处变异）| — |

> **交接材料已成文（M4，2026-09-30）**：三条的"自包含最小复现 + 证据 + 影响 + 可能方向 + 跟进约定"
> 全在 **`docs/dhampir-base-issues.md`**（用仓库自带的演示工程就能复现，不依赖本机那些 `.workbuddy` 产物）。
> 跟状态就在这张表里改；对方修好后按那份文档末尾的"跟进约定"走（先用 M1 对拍复测再重标定基线）。
>
> ✅ **D1 已撤回（2026-10-01）**：底座 `c7f29b9` 复现不出来，我们复核也是"正常出图"
> （`.workbuddy/p2-d1-verify.mjs`：窗口内平均差 39.33 / 窗口外逐字节相同 / angle=45 左上=color_a 右下=color_b）。
> 原复现 JSON 无效（缺 `timeline.schema`；`end` 超素材帧数）⇒ 台账里那条"底座不出色罩"的说法**作废**，
> `docs/vtrim-integration.md` §4.8 与 S4 决策书的硬前提 A 都已同步改过。
>
> ⚠️ 下面那段"**D1 修好后的**跟进项"保留作历史（① 复测 / ② 重标定 / ③④⑤ 文档已在本次做完）：
> ① 先用 `node .workbuddy/run-parity.mjs demo` 复测
> （overlay 窗口的像素差应**下降**）；② 因为色罩从"兜底腿画对、底座不画"变成"两条腿都画"，**基线会变**
> ⇒ 按本台账的约定**重标定三样本**（活参照与黄金帧两套数字都要重取：`--capture-golden` 重冻一次）；
> ③ 划掉本节 D1 与 `docs/dhampir-base-handoff.md` §1；④ 把 `docs/vtrim-integration.md` §4.8 的 overlay 行
> 从"底座当前不出这个色罩（D1）"改成"已接并可见"；⑤ **之后**才谈 S4 删除（`s4-second-chain-removal-decision.md`）。
>
> ⚠️ **2026-10-01 新增自包含交接单**：`docs/dhampir-base-handoff.md`（D1 的最小 doc 内联成 JSON，
> 不依赖本仓工程；B 组五条"量化不足"也一并整理进去了）—— 要单开会话修底座时**直接用它**。
>
> ⚠️ **2026-10-01 新增自包含交接单**：`docs/dhampir-base-handoff.md`（D1 的最小 doc 内联成 JSON，
> 不依赖本仓工程；B 组五条"量化不足"也一并整理进去了）—— 要单开会话修底座时**直接用它**。

## 五、待修（本仓，长线目标分解）

### M1 对拍基线（**先做这个**：没有它，后面每一步"像不像"都是嘴上说）

- **工具已经有了**：`scripts/vtrim-compare.mjs`（零依赖 node，2026-09-30 随转译器一起迁进本仓）。
  它一条命令跑完整条回路：**取固定样本（复制到运行目录，绝不动样本本体）** → 插测试事件 → 两边出片 →
  **先算源时间→输出时间映射再抽帧**（`mute_ranges` 会让"按源时间取帧"量到完全不相干的时刻）→
  每个窗口量三样：**像素差**（与窗口外基线比）、**高频比**（分"糊"与"锯齿"）、
  **最佳纵向位移**（非 0 多半是"位置不对"）。
  **判据**（超了 `exit 1`）：时长差 ≤ `durationDeltaSec` + 基线中位像素差 / 基线位移 / 逐窗口"高于邻居"。
  屏蔽、阈值、样本与自证都在下面「第二次实测」里。
- 用法（本仓跑，别用底座仓的路径）：

  ```bash
  node scripts/vtrim-compare.mjs --clip <片段目录> \
    --vtrim <vtrim.exe> --out .workbuddy/vtrim-compare-<样本>
  ```

  ⚠️ 它最后一步用 `python3`（要 **numpy**）算度量。本机 `python3` 是商店桩、`python` 不在 PATH ——
  用 `VTRIM_PYTHON=<真解释器>` 指定（DSH 里就是 `load_workspace_dependencies` 给的那个
  `…\dependencies\python\python.exe`）。
- M1 的三个交付（按顺序）：
  1. **产物名不再写死**（✅ 2026-09-30 已修）：原来硬编码 `vtuber-clip_1.mp4`，演示工程
     （`output = "export.mp4"`）一跑就失败；现在从 `polish.toml` 的 `output` 读，并接受
     "有旧成片 ⇒ `_1`" 两种名字。
  2. **python 不写死**（✅ 2026-09-30 已修）：原来 `run('python', …)`，本机没有 ⇒ 两边都出完片、
     最后一步报 `Command failed: python …`（看着像渲染失败）。现在按
     `VTRIM_PYTHON` → `python3` → `python` 依次试，全不行就把"要带 numpy 的 python"说清楚。
  3. **阈值断言 + 固定样本**（⏳）：把"像素差 / 高频比 / 位移"的判定写进脚本（阈值常量放头部 + 写清怎么定的），
     超了 `exit 1`；固定样本 ≥3 个（`docs/wgpu-effect-reference/demo` 最快、
     `.workbuddy/perf/realproj/output/clips/clip-08` 长片、一个**竖屏 + 带屏蔽区间**的），
     缺样本就明确跳过并说清要准备什么。
- 验收：一条命令给出"每个样本 × 每个窗口"的表 + 红/绿，并且在 CI/手动两条路上都能跑
  （与 `render_bench.rs` 同一套约定：**默认测试里不跑** —— 它要底座二进制、ffmpeg、Chrome、GPU、numpy）。

#### 第二次实测（2026-09-30 本轮，M1 三件套落地之后）

命令：`node .workbuddy/run-parity.mjs demo|portrait|real|all`（node 包装在 `.workbuddy/`，把
"带 numpy 的 python 在哪 / `vtrim.exe` 在哪 / 参数怎么拼"收在一处；`metric <样本>` 用 node 读报告）。

| 样本 | 参照 vs 本仓 | 基线（**中位**像素差） | 逐点（窗口外 5 处） | 位移 | 判据 |
|---|---|---|---|---|---|
| `demo`（12s 横屏，带 2 段屏蔽） | 268 帧/8.933s @30 vs 536 帧/8.933s @60 | **15.07**（平均 32.42 / 最差 77.61） | 3.0 / 6.5 / 15.1 / 60.0 / 77.6 | 0px | ✓ 全过 |
| `portrait`（同源，1080×1920） | 同上 | **9.87**（平均 35.02 / 最差 114.84） | 3.6 / 6.3 / 9.9 / 40.5 / 114.8 | 0px | ✓ 全过 |
| `real`（`clip-08`，133.333s 长片） | 4000 帧/133.333s @30 vs 8000 帧/133.333s @60 | **9.77**（平均 9.16 / 最差 10.53） | 5.9 / 9.1 / 9.8 / 10.5 / 10.5 | 0px | ✓ 全过 |

**这三条数怎么读**（本轮查出来的）：

1. **判据用中位数，不用平均**：演示工程是**快切特效片**，采样点一旦落在硬切上，两条腿的 PTS
   网格差一帧就足以让整帧差几十（实测 60.0 / 77.6 / 114.8），而那既不是口径差也不是效果差
   （同一组 `dy` 反而是 0）。平均会被它们主导（32.42）——与窗口判据用中位数是同一个理由。
   分布**照实打全**（mean/median/worst/逐点都在报告里），不是"把大数藏起来"。
   **`real` 长片给出了干净的那一版**：5.9~10.5 挤在一起（无剪切帧）、高频比 0.98（锐度一致）、
   `dy=0` —— 这才是两条腿的真实底噪（与底座仓当年记的 7.02 同一量级）。
2. **内容对齐后真实底噪 ~6**（诊断实测：同一时刻 6.19、参照往前一帧 **5.90**、往前两帧 9.73）。
   所以基线采样点**避开屏蔽区间的剪点**（±0.4s），并给基线**只给基线**开 ±1 帧的时间对齐 ——
   特效窗口不许开：那里量的就是"同一时刻像不像"，给它自由度会把"特效慢了一帧"放过去。
3. **弹幕带屏蔽不是那个 30 的原因**：`demo` 屏蔽前后中位 29.31 → 15.07 的变化来自"采样点不再落在剪点上"；
   `real` 屏蔽后反而略高（8.04 → 9.77，屏蔽掉的区域本来恰好比较像）。
   两个数都照实打印 —— 屏蔽是判据口径，不是"看不见"的许可。

**阈值 M1 定稿**（`scripts/vtrim-compare.mjs` 头部 `THRESHOLDS`，标定依据写在常量旁边）：
`baselinePixel 20`（中位口径；三样本 9.8~15.1，留 ~1.3–2 倍余量）、`maxDy 4`（实测全是 0）、
`durationDeltaSec 0.2`（实测 0.000 —— **这条是拿"剪两次"那次事故换来的**）、窗口判据 `邻居 × 1.5 + 2`。
`--baseline-pixel-max <n>` 可覆盖 —— **它就是"让断言红"的开关**，自证：
`node .workbuddy/run-parity.mjs demo -- --baseline-pixel-max 5` ⇒ 判据报错 + `exit 1`、报告仍保留 ✓。
另有 `--self-test`（28 项：映射 / 插事件 / 屏蔽带 4 项 / 阈值判定 5 项，含"超阈值必须报"三条）。

#### ✅ M1 完成（2026-09-30）

验收对照：
- **≥3 个真实工程** → `demo`（12s）/`portrait`（1080×1920）/`real`（133s 真实长片），全部跑通、判据全过；
- **两条通道逐帧差异阈值** → 三条硬判据 + 窗口判据（上面「阈值 M1 定稿」）；
- **可重复脚本** → `node scripts/vtrim-compare.mjs --fixture <名字|--all-fixtures>`（一条命令给出表 + 红/绿）；
  样本一律复制到运行目录，绝不动样本本体；缺样本：点名 = 报错并说清要准备什么、`--all-fixtures` = 跳过；
- **CI 门禁** → 与 `render_bench.rs` 同规（要底座二进制 / ffmpeg / Chrome / GPU / numpy ⇒ **默认测试里不跑**），
  本机实操入口是 `.workbuddy/run-parity.mjs`（`demo|portrait|real|all`、`metric <样本>` 用 node 读报告）；
  仓库侧另有**常跑**的三条 node 守卫（接线 38 / 转译器 334+13 / 对拍自检 28）—— 它们不需要 GPU。

**下一轮（M2）**：清 `DROP/CLAMP` 清单 —— 先做转译器报告里剩下那几条
（`overlay_type = "fade_in" / "color"`、贴纸 `auto` 落点、弹幕高亮 `\c`、竖素材运镜门），
每条要么修、要么把"为什么不修"写进报告与 `docs/vtrim-integration.md`。

### M2 报告清单清零（**进行中：2026-09-30 第一轮**）

现状（三个样本，`--keep-muted`）：`demo` **丢失 3 / 降级 2**（修之前 5 / 3）、`real` **丢失 2 / 降级 1**
（修之前同）—— 剩下的每一条都有明确归属：

| 清单项 | 结论 |
|---|---|
| `overlay_type = "color"` | ✅ **已修**：映射成 `OverlayShape::Solid(0)` + 0.4s 进出包络（参照 `index.html:1662-1666`）。以前整条 DROP |
| `overlay_type = "fade_in" / "fade_out"` | ✅ **已修**：整窗线性斜坡、**不叠** 0.4s 包络（参照 `:1652-1658`）。**片头渐入就是这一档** —— 以前"开头渐入没生效"有一条是转译侧丢的 |
| `gradient` 的包络 | ✅ **已修**：改走 `placeEnvelope`（以前整窗恒定 `amount`，头尾硬起硬落） |
| 贴纸"没有 `sticker_tag`" | ✅ **已修**：`name` 才是后处理选定的文件名（旧写法**提前 return** ⇒ 选好的贴纸不进成片）。顺带修"只试 `.gif`"（PNG 贴纸会找不到） |
| 贴纸落点 `auto` | ✅ **改成可操作**：落点是 `process` 的产物 ⇒ 报错里直接说"先跑一次 `vtrim polish process`"，**不猜位置**（底座没有安全区推导） |
| `zoom="远"` 找不到倍率 | ✅ **已修**：兜底照抄模板 `camFactor`（**按档位**：远 0.55 / 近 1.5 / 其余 1.3、认不出按中景、`isFinite`）。以前统一退 1.0 ⇒ **远景档整段不拉远** |
| 竖素材运镜门 | ✅ **本来就是实现的**（探源尺寸；探不到进 CLAMP）—— 卡与本文档那句"还没复刻"是**过期文案**，已改 |
| `overlay_events` 元数据 | ✅ 顺手修：以前数的是一个**已经没人写入的数组**（永远 0）；现在改成诚实的计数器 |
| `vignette` | ⛔ **不修**：参照自己的出片路径里就没有它（只有 DOM 预览有）——不替参照补画 |
| `cover` | ⛔ **不修**：静态图产出，本仓自己另开一条（`docs/plan/cover-dhampir-tradeoff.md`） |
| `[style.subtitle]` 字重/行高/文字阴影 | ⛔ **不修**：底座契约没有对应字段（字重只靠字体库里有没有 bold 支）；`cycle-5` 入场动画（`entrance` 字段）**已从契约移除**（2026-10-01：schema 删字段 + `legacy.rs` 读时吞掉） |
| `color_shift`（sepia） | ✅ **已随类型下线消失**（2026-10-01 D1）：能力并入 `overlay`（纯色/渐变），sepia 底座本来就没有 |
| `split`（shear） | ✅ **已随类型下线消失**（2026-10-01 D1）：底座没有 shear，本仓也就不再提供这一类 |

**本轮加的守卫**（都在常规门禁里，不要 GPU）：
- `scripts/check-vtrim-translator.mjs` 的第三段 **27 项映射检查**（含 `overlayBaseAngle` 的 8 项角度换算）（合成工程过一遍 `translate`：
  四档覆盖层的图层/形状/包络、只有 `name` 的贴纸、`zoomFactor` 六个取值）+ 两处新变异（15/15 打红）；
- **`scripts/check-dhampir-drop-list.mjs`（新）**：把演示工程的 DROP/CLAMP 清单钉成**基线**，
  每条都写着"为什么它还在"；清单**多一条/少一条都红**，`--self-test` 自证这两个方向都会红。

### 2026-10-01 追加：**"字体不对"到底哪不对**（用户实测 clip-18 后报的）

用户口径：**"字体不对，其他的正确。"** 逐层查下来是**两件事叠在一起**，都不是"选错了字体文件"这么简单：

| 层 | 事实 | 结论 |
|---|---|---|
| 预览（`webui/src/components/PolishPreview.vue`） | `.cap{font-weight:**700**; -webkit-text-stroke:12px}`、`.dm{font-weight:**600**}`；族名 = `"<项目字体/缺省 LXGW WenKai>","Microsoft YaHei","Noto Sans SC",sans-serif` | 预览的字幕是**雅黑 Bold**（本机没装 LXGW、雅黑排在 Noto 前） |
| 转译产物 | 字幕轨 `font_weight: 700`、弹幕轨 `600` —— **本来就写对了** | 契约里字重没问题 |
| 底座 | **不合成粗体**：`font_weight >= 600` 时它去要 `--font-bold-file`，拿不到就退回 `--font-file`（`msyh.ttc` = **常规体**）；底座 schema 的注释原话："本仓先前完全不设字重，于是笔画比它细一圈" | 本仓**从来没传过 `--font-bold-file`** ⇒ 成片比预览细一圈 ✗ |
| 族名顺序 | `SYSTEM_FONT_STACK` 原本是 `LXGW → Noto → YaHei`，而**预览那处是 `LXGW → YaHei → Noto`** —— 同一份工程，预览落雅黑、成片落 Noto（两者都是"装了的"） | 两处栈**顺序不一致**；顺序决定"项目选的那支没装时谁赢" |

**修法**（都在 `render/` 与转译器口径之内）：

1. `SYSTEM_FONT_STACK` 改成与预览同序（`'LXGW WenKai','Microsoft YaHei','Noto Sans SC',sans-serif`）
   —— chrome 那条腿（index.html/cover.html）与底座那条腿读**同一份常量**，三处一起对齐预览；
2. `find_font` 返回 `(常规, Option<粗体>)`：按栈逐支"字体库 → 系统字体目录（族名归一化 + 别名表
   `Microsoft YaHei → msyh.ttc`）"解析，同族再配对粗体（`msyhbd.ttc`）；`run_render` 传 `--font-bold-file`；
3. 就绪位/医生把**有没有粗体那一支**露出来（`font_bold`）——它就是"细一圈"的直接原因。

**实测证据**（同一份工程、同一帧，只差 `--font-bold-file`）：`.workbuddy/font-compare/bold-ab.png`
（上 = 不给粗体，下 = `msyhbd.ttc`）——下图的笔画明显更粗 = 预览的 700 口径 ✓。
演示样本对拍**复跑仍全绿**（基线中位 15.07 未动、位移 0、时长差 0.000）。

⚠️ 顺带记两条**教训**：
- **"同样的字体名"不等于"同样的字重"**：这一轮的差异全在字重上，靠"看字体文件名"永远查不出来 ——
  要比的是 `font_weight` 这条契约值，以及宿主**有没有能力实现它**（底座只能换文件，不能变轴）。
- **同一个"字体栈"在仓库里有两份**（`server.rs::SYSTEM_FONT_STACK` 与 `PolishPreview.vue:1333`），
  顺序一漂，表现就是"预览一个字体、成片另一个字体"且**不报错**。改任一处先与另一处对顺序。

**M2 剩下**：`real` 样本重新跑一次确认清单没反弹（本轮只跑了 `demo`）；以及把 `docs/vtrim-integration.md`
的第六节"已知差异"表与这里的清单对齐（本轮已改 §4.8 与 §5 的口径，§6 的逐条还没逐行核）。

### M3 就绪位与医生（**2026-09-30 本轮完成**）

| 交付 | 落点 | 验收（都实测过） |
|---|---|---|
| **唯一一份就绪判断** | `dhampir::readiness(font)` → `Readiness { ready, binary, preview, translator, font, contract_version, problems[] }` | 单元测试：`VTEDIT_DHAMPIR_CLI` 指到不存在的文件 ⇒ `ready=false` + `problems` 点名"出片二进制"（**六项各查一次文件，不渲染、不跑 node**） |
| **HTTP 就绪位** | `POST /api/polish/dhampir-status`（已登录） | 接口测试：200 + `ready` 布尔 + `problems` 数组；不 ready 时**至少一条原因**（否则前端说不出为什么） |
| **health 能力位** | `/api/health` 的 `dhampir_ready`（**只回布尔、不回路径** —— health 无鉴权） | 与上一条同源（同一函数），不需要新增测试 |
| **`polish doctor` 的 `[dhampir]` 块** | `commands.rs::cmd_doctor` | 实测输出：`✓ 二进制 / ✓ 转译器 / ✓ 字体 / ✓ 预览产物 / ✓ 底座可用（…--renderer chrome 可退回本仓）` |
| **前端提示** | 渲染弹窗打开时调就绪位：不可用 ⇒ 一行红字说明原因（路径挂 `title`），可用 ⇒ 一行"底座就绪 ✓" | `npm --prefix webui run build` 通过；接线门禁 40 项（前端 3 条 dhampir 路径 ↔ 路由） |
| **`--strict`** | `polish render --strict` → `VTEDIT_STRICT=1` → `render_video_only(..., strict)`：转译报告 `DROP > 0` **拒绝出片**并逐条列出 | 实测（演示样本，3 条 DROP）：转译后、**渲染前**退出码 1，报错里逐条列出 DROP + "不加 `--strict` 就会照常出片"；`TranslateReport::parse` 有单元测试 |

⚠️ 一处**刻意的取舍**：`/api/health` 只回布尔（不查 node、不回路径）—— 它是未认证接口且被前端反复轮询；
"六项各查一次文件"的完整判断走已登录的 `/polish/dhampir-status`。

### M4 底座问题交接与跟进

- 把 §四 三条 + 最小复现整理成一页发出去；本仓台账记录状态（对方修没修、我们何时能去掉 CLAMP）。

### M5 P4 去 Node（**2026-09-30 本轮定案：现在不做 Rust 化，把"要 Node"变成可见、可绕的两件事**）

**现状**：`tools/polish-to-dhampir.mjs` **3,679 行 / 136 KB**，零依赖 ESM；它把 `polish.toml`
转译成底座工程。它是**本仓模板的复刻副本**（曲线逐值抄、每条事件的注释写着"参照第几行、底座用哪条原语"），
等价性由守卫钉住：**334 项曲线 + 13 项颜色 + 27 项映射 + 16 处变异自证**。

**本轮做的**（"明确记录不做的理由"那一支，外加把依赖变得可见）：

| 交付 | 内容 |
|---|---|
| 就绪位查 **Node 版本** | `readiness(font, deep)`：`deep` 时才跑一次 `node --version`（**已登录**接口与 `polish doctor` 用它；`/api/health` 那条被轮询、无鉴权 ⇒ 只走"PATH 里有没有"的便宜判断）。`MIN_NODE_MAJOR = 18`；`parse_node_version` 是**纯函数**（真实环境没法安全假装 v16，所以抽出来测） |
| 三类问题说清 | ① PATH 上没有 node；② **有 node 但 `node --version` 跑不通**（商店桩 / 装了一半 —— 我自己在 `python3` 上踩过同款）；③ 版本 < 18。三条都带"改用 `--renderer chrome`"的逃生口 |
| 文档 | `README.md` 前置条件补 **Node.js 18+（默认出片通道需要；`--renderer chrome` 可绕）**；`polish doctor` 的 `[dhampir]` 块多一行 `Node: vX.Y.Z`；前端渲染弹窗的 `title` 也带上 Node 版本 |

**为什么现在不做 Rust 化**（成本/风险对比，不是"懒得做"）：

1. **体量**：3.7k 行里一大半是**为对齐参照写死的细节**（每条事件映射的注释就是"参照哪一行、底座用哪条原语、
   为什么这么近似"）。Rust 化 = 逐条重写 + **重新建立等价性证据**，不是"翻译一遍"。
2. **守卫要重建**：334+13+27 项与 16 处变异全是针对现实现的；Rust 版需要**等价守卫**（否则"看着像"的回归会溜过去）
   —— 这本身就是半个项目，而且期间会有**两份实现并存**（模板改了要同步三处：模板、JS、Rust）。
3. **依赖成本很低**：它**不是产品代码**（不 import `crates/`、不进二进制），node 只落在"出片机器"这一侧；
   而那台机器本来就要 **ffmpeg + ffprobe**（README 前置条件）与（chrome 那条腿要）**Chrome** —— 多一个 Node 安装
   不是质变。反过来，本仓的对拍/守卫**本来就是 node**（`vtrim-compare.mjs` / `check-*.mjs`），
   所以"CI 里没有 Node"这条理由不成立。
4. **收益有限**：省掉一个安装 vs 一次大重写 + 永久两份实现风险。

**什么时候该重开**（判据，任一成立就重新评估）：

- 出片要**打包分发给"不会装 Node"的用户**（分发形态变了，安装 Node 从"开发者一次"变成"每个用户一次"）；
- 转译器的维护成本**超过重写**：例如模板改动频繁，每次都要三处同步（模板 / JS / 守卫）；
- 需要把转译塞进**无 Node 的环境**（当前的守卫链其实都要 Node，所以今天不成立）。

**如果真做，路线与验收**（先写在这里，省得下次重新想）：

1. 新模块 `vtrim-polish/src/render/translate/` + `vtrim polish translate <dir> --out <doc>` 子命令；
2. **A/B 对拍**：同一份 `polish.toml` 两条实现各出一份 doc，**逐字段比对**（结构与数值；先允许浮点误差），
   跑遍 M1 的三个样本 + 合成工程；
3. 验收线：所有样本 doc **结构等价** + M1 对拍基线**不动**（像素差不变）；
4. 只有在 1–3 全绿之后才切默认实现，并**同期把守卫换成 Rust 版**（否则守卫就只保护旧实现了）。

## 六、每次改动后的必跑项

```bash
cargo check --workspace --all-targets          # 0 warnings
cargo test -p vtrim-polish --lib               # 198+
cargo test -p vtrim-server                     # 143+
node scripts/check-vtrim-translator.mjs --mutate   # 334 曲线 + 13 颜色；13 处变异全红
node scripts/check-dhampir-wiring.mjs          # 接线（含"出片腿带 --keep-muted / 预览腿不带"）
node webui/scripts/check-preview-ab.mjs        # 预览 A/B（跳屏蔽区间）
python3 scripts/check_module_map.py && python3 scripts/check_module_boundaries.py
```

改动 `tools/polish-to-dhampir.mjs` 之后**必须**再真跑一次出片（预览看着正常 ≠ 出片正常）。
