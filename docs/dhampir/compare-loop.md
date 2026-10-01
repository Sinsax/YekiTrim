# V-Trim → Dhampir：对比回路与边界

> **视角（迁移后加）**：本文 2026-09-30 从 Dhampir 仓的 `docs/vtrim-integration.md` 迁来
> （在那边它是"Dhampir 这一侧的部分"）。原文里的「本仓」指 **Dhampir**；现在这份文件住在
> **V-Trim 仓**，所以凡是要区分两个仓的地方都改成了显式的 **Dhampir** / **V-Trim**。
> **读数、表格、命令的参数与结论一字未改。**

> **转译接口的正文**（形状 / 快速开始 / 契约要点 / 行为对照表 / 三级记录 / 已知差异 / 怎么扩）
> 在本仓 `docs/vtrim-integration.md`（一 ~ 八节）。
> 转译器本体 `tools/polish-to-dhampir.mjs`、它的守卫 `scripts/check-vtrim-translator.mjs` 也在本仓。

---

## 转译器住在哪、怎么用

**转译器就在本仓**：`tools/polish-to-dhampir.mjs`（零依赖 ESM，一个文件）。
`scripts/vtrim-compare.mjs` 找它的顺序是：

| 优先级 | 来源 |
|---|---|
| 1 | `--translator <路径>` |
| 2 | 环境变量 `VTRIM_TRANSLATOR` |
| 3 | `<本仓>/tools/polish-to-dhampir.mjs` |

三个都找不到就**报错并列出找过哪些地方**（不静默退成"没有转译器"）。

**只想转译、不跑对比**时直接跑它（它自带 CLI）：

```bash
node tools/polish-to-dhampir.mjs "<片段目录>" --out target/t.json
```

它用起来仍然是零依赖的 —— 转译器**不 import Dhampir 的任何东西**，
Dhampir 也**不 import 它**。两个方向都没有代码依赖，只有"约定"。

---

## 九、对比回路：`scripts/vtrim-compare.mjs`

一条命令跑完"改 toml -> 两边渲 -> 按窗口出表 -> 恢复工程"：

```bash
node scripts/vtrim-compare.mjs --clip "<片段目录>" --events target/fx-events.json --out target/vtrim-run
```

`--events` 是一个 JSON 数组，描述**要插进 `polish.toml` 的测试事件**（**源时间**）：

```json
[
  { "type": "blur",    "time": 66.2 },
  { "type": "stutter", "time": 68.2, "intensity": 30 },
  { "type": "shake",   "time": 70.2, "intensity": 20 }
]
```

不传 `--events` 就是"只比现有工程"（那是**基线**，用来确认两边本来就一致）。

输出长这样：

```text
  基线（窗口外 6 处）：像素差 7.02   高频比 0.98   位移 0px

  特效            输出窗口         中位差    邻居   高频比  最差点  位移  判定
  blur@73        67.70-68.50      1.20    3.22   0.96    157    0   ✓
  stutter@75     69.70-69.82      4.13    4.56   0.94      5    0   ✓
  shake@77       71.70-71.90      3.30    5.72   0.98      5    0   ✓
  flash@79       73.70-74.14      7.26    6.32   0.94     11    0   ✓
  noise@80.6     75.30-76.10      6.04    5.83   0.96      7    0   ✓
```

（`blur` 那一行的 `最差点 157` 就是"切换差一帧"那个点 —— 它摆在那里，
而**中位差 1.20 才是这条窗口的真实读数**。）

### 它把**四个**坑堵死了

这四条都是**我在手工搭这条回路时真踩过的**，而且**每一次都产出了"看起来有据"的假结论**：

| 坑 | 症状 | 脚本怎么防 |
|---|---|---|
| **甲：源时间当输出时间** | `mute_ranges` 有 17.49s 位移，按源时间去 `-ss` 量的是**不相干的时刻** —— 第一批结论整个是反的 | **先算映射再取样**，并把映射表打在最前面 |
| **乙：产物提前清理** | 追一个差异，回头要复核时基准已被自己删掉 | 运行目录**默认保留**，`--clean` 才删；报告里写明"结论要复核时还得看它" |
| **丙 a：两宿主的帧率不同（渲染范围）** | `--to` 用参照的帧数（2907@30fps），而 Dhampir 是 60fps -> **只渲了一半** | `--to` 按**Dhampir 自己的时间基**算，并跟工程帧数取小 |
| **丙 b：两宿主的帧率不同（取样时刻）** | 同一条时间在两个片子上落在**不同的帧**上。对短效果是致命的：stutter 的台阶只有 0.06 秒，半帧错位就把整条量歪 -> 量出 `18.24`（邻居 `5.42`），而按参照帧对齐再取，**每个点都是 `(0,0)`、整帧差 2.4~4.8** | 取样点取**参照帧的边界**（`f / refFps`）。⚠️ 取**中点**反而错：`-ss` 输出"PTS >= t 的第一帧"，中点会落到下一帧 |
| **丁：平均数被一个边界点主导** | 一个窗口只取 5~8 个点，只要有一个落在**镜头切换那一帧**上，它的差就是几十上百。实测 blur 窗口 8 个点是 `1.13 1.14 1.13 1.18 1.18 **157.35** 2.75 2.69` -> **平均 21.08，中位 1.18** | 判据用**中位数**；另把**最差点**单列一列（157 摆在那里一眼看得出是边界点） |

另外整个流程包在 `try/finally` 里：**无论中途哪一步崩，`polish.toml` 都还原**
（已实测：跑完与跑前**逐字节相同**）。

自检：`node scripts/vtrim-compare.mjs --self-test`
—— 19 项，按**逐值**钉住上面甲/乙两个坑（映射要对多段 `mute_ranges` 逐点精确；
多个事件落到不同 segment 时行号不能串）。4 个变异里 3 个打红
（第 4 个是**无操作变异**，两个正则对那份输入行为相同 —— 不是自检的漏洞）。

### 依赖

`ffmpeg` / `ffprobe` 在 PATH 上、**Dhampir 的可执行**
（`--dhampir` 或环境变量 `VTEDIT_DHAMPIR`，默认找同层 `Dhampir/target/{debug,release}/`）、
**本仓的 `vtrim.exe`**（`--vtrim` 或 `VTRIM_EXE`，默认找 `.workbuddy/perf/bin/`，
再退 `target/{release,debug}/`，最后 PATH）、
以及**转译器**（`--translator` 或环境变量 `VTRIM_TRANSLATOR`，默认 `tools/polish-to-dhampir.mjs`）。

**两边路径都可配**是有意的：底座（Dhampir）是**另一个仓库**，只调二进制、不链库，
两个仓各自演化；契约见 `docs/dhampir/README.md`。

---

## 十、这条接口的边界

**转译器只读不写 V-Trim 的东西**：它读 `polish.toml` / `clip.srt` / `clip.dm.srt`，
把旁写文件放在**输出目录**里。

**它不承诺兼容**：它不是 `dhampir-core` 底座的一部分，形状可以随 V-Trim 演化而改。
底座承诺兼容的只有 `docs/api-surface.md` 里"底座 API"那一栏。

**它不做媒体探测**：`frame_count` 留空由 `dhampir probe` 回填 ——
转译要能在一台没有 ffmpeg 的机器上跑完并给出清单。
