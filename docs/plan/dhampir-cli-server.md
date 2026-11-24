# Dhampir 接入：CLI 与 WebUI server 实现计划

> 配套：`docs/plan/dhampir-integration.md`（接口面与分期；D1–D4 已定）。
> 前置事实（已核对代码）：**server 不直接调底座** —— `api::render::run_polish_child` 起的是
> **自己的二进制** `yekitrim polish render <dir>`（stdout/stderr 落 `<data_dir>/logs/render-<job_id>.log`，
> `follow_render_output` 边跑边读，取消时 `render_proc::kill_tree` 杀整棵树）。
> 所以"加一条通道"= ①CLI 多一个取值 ②server 把它透传下去 ③里程碑表认它的输出。
>
> **D2 的约束贯穿全文**：chrome 是主线，底座是**实验通道**（UI 上要标明），但接口面按"**远期替代**"设计 ——
> 进度/取消/产物/报告四件事一次做全，别等替代时返工。

## 一、CLI

### 1.1 改动点

| 文件 | 改动 |
|---|---|
| `crates/yekitrim-cli/src/cli.rs` | `PolishAction::Render { input, renderer }` 的 `--renderer` 帮助文案改成 `chrome\|dhampir`（**保持 `String`**：老命令里的 `wgpu` 要能**带 warn 回退**，`ValueEnum` 会直接判非法）；新增 `PolishAction::Translate { input }`；`--strict` 加到 `render`（D4：只有它会拒绝 `DROP > 0`） |
| `crates/yekitrim-cli/src/main.rs` | `set_var("VTEDIT_RENDERER", …)` 保留（`cmd_render` 从 env 读），但**校验集中在 `pipeline.rs` 一处**（`wgpu`/`native` → warn + 回退 chrome），别写第二份 |
| `crates/yekitrim-polish/src/render/backend.rs` | 新增 `RenderChannel { Chrome, Dhampir }`（**与 `FrameRenderer` 分开**：底座不属于"给帧出帧"） |
| `crates/yekitrim-polish/src/render/dhampir.rs`（新） | `run_dhampir(project_path, opts) -> Result<PathBuf>` —— **唯一调用点**，内部见 1.2 |
| `crates/yekitrim-polish/src/render/pipeline.rs` | `cmd_render` 按通道分支（底座走 `run_dhampir`，不进 chunk 循环） |
| `crates/yekitrim-polish/src/commands.rs` | `cmd_doctor` 输出加 `[dhampir]` 块（1.3）；新增 `cmd_translate` |

### 1.2 `run_dhampir` 内部（顺序不可换；**D1 换实现时只动这里**）

```text
1. 转译     ← 现在: spawn `node tools/polish-to-dhampir.mjs <dir> --out <项目>/data/dhampir/project.doc.json`
            ← 远期(P4, D1): 换成 **Rust 原生实现**（直接吃已加载的 ProjectFile，省掉手写 TOML 那整块）；
               `tools/…mjs` **不删**（底座侧测试要用 + Rust 的 parity oracle）；调用点与其余步骤一个字不改
2. probe    dhampir probe --project <doc.json>          # 校验 schema/素材；回填帧数
3. 字体     style.subtitle.font → yekitrim_core::fonts::resolve_file → --font-file <真实路径>
            拿不到 → 退系统字体 + warn（底座没字体会判失败，别让它炸在深处）
4. render   dhampir render --project <doc.json> --asset-root <项目目录> --from/--to <本仓时间基算>
5. 产物     unique_output_path（同名 _1/_2）+ 登记成"渲染产物" + **记下通道**（D2）
```

- **D3**：产物与旁写文件（`dhampir-subtitle.srt` / `dhampir-danmaku.ass` / `report.json`）
  全在 `<项目>/data/dhampir/` —— 项目内相对路径，**不新增请求体路径字段 → 不动路径面三防线**。
- **D5（竖屏）不在这里处理**：源视频方向由**上层配置**给（`polish.toml` 的 `[source] width/height`，
  `polish process` 那次已有的 ffprobe 顺手写入），**转译器读配置、按它过滤运镜**。
  `run_dhampir` 只负责把片段目录交给转译器。⚠️ 老工程缺这个字段时转译器进 NOTE 且**不过滤** ——
  CLI 可以在那一步补一句提示（"跑一次 `yekitrim polish process` 补上源尺寸"），**别自己探媒体**。
- **JS 保留 ⇒ 需要常驻 parity 守卫**：`scripts/check-dhampir-parity.mjs`（同一输入下 Rust 产物与 JS 产物
  **逐字节相同**）。P4 之后这条必须绿，否则"两边都留"就变成了漂移源。
- 缺 Node / 缺底座 / 版本不符：**三种错分开报**，每条给可操作出路（装 Node、指 `--dhampir`、或改用 `--renderer chrome`）。
  ⚠️ **不许静默回退 chrome**：用户显式点了底座通道。

### 1.3 `yekitrim polish doctor` 加一块（不新增命令）

```
[dhampir]                      （实验通道；默认出片仍是 chrome）
  node      : 未找到  ← 转译器现在需要 Node 18+；装上它，或继续用 --renderer chrome
  exe       : F:\...\dhampir.exe (0.0.1)
  schema    : 1  ✓ 与转译器一致
  转译器    : tools/polish-to-dhampir.mjs ✓
```
- 探测顺序见 `dhampir-integration.md` §3.2；**版本/schema 不符在这里就报**。
- 只在 `polish doctor` 里加一块，**不动它现有的输出结构**（那是给用户看的摘要）。

### 1.4 CLI 验收

```bash
cargo run -- polish doctor                       # 4 行状态，缺什么一目了然
cargo run -- polish translate <片段目录>          # 只转译 + 打报告（把"底座没装"与"转译有差异"分开）
node scripts/check-yekitrim-translator.mjs --mutate  # 转译器本身没退化
cargo run -- polish render <片段目录> --renderer dhampir
# 要求：
#  ① 同一条命令跑两次，产物 sha256 相同（可复现）
#  ② 把 --dhampir 指到不存在的路径 → 非零退出 + 可操作文案，**不许回退 chrome**
#  ③ 任务日志里 DROP/CLAMP 逐条可见（info 级）
#  ④ 加 --strict 后 DROP>0 才拒绝（默认继续）
#  ⑤ 竖屏配置化（P1.5）：横素材工程（含 demo）产物**逐字节不变**；
#     竖素材工程转译报告里能看到"已过滤 N 条 camera"+ 一条 NOTE；
#     ⚠️ 顺带核一遍 shake/flash/blur 等瞬时特效**没被一起过滤掉**
```

## 二、WebUI server

### 2.1 改动点

| 文件 | 改动 |
|---|---|
| `crates/yekitrim-server/src/api/polish/` | `polish_render` 读 `body.renderer`（白名单 `chrome`/`dhampir`，缺省 `chrome`；非法 → **400**），透传给 `run_polish_child` |
| `crates/yekitrim-server/src/api/render.rs` | `run_polish_child(..., sub, dir, extra: &[String])`：拼 `["--renderer", r]`；`RENDER_MILESTONES` 加底座那几行（2.2） |
| `crates/yekitrim-server/src/api/mod.rs` | `/api/health` 加 `"dhampir": {"available", "exe", "version", "schema_ok"}` —— **这是"就绪位"，不是构建能力位**（与已删除的 `wgpu_render` 不同：那个是编译开关，这个是"装没装/版本对不对"） |
| `crates/yekitrim-server/src/api/polish/`（只读端点） | `GET /api/polish/dhampir` → 同一块（精修页打开时拉一次，不必每次 health） |
| `webui/src/api/polish.js` | `runRender(pid, dir, renderer)`；`fetchDhampirStatus()` |
| `webui/src/views/PolishView.vue` | 渲染入口加通道选择（**含"实验"标注**），按就绪位置灰 |
| 产物卡 | 显示"这条产物是哪条通道出的"（D2 要求；否则用户分不清 `xxx.mp4` 与 `_1`） |

### 2.2 进度与日志（决定"能不能用"）

- **复用现有任务 kind `render`**：通道只是参数，资源键/档位/取消语义全沿用（不新增 kind）。
- ⚠️ **底座输出要按 chrome 通道的同形里程碑打**（否则 parser 不认，进度会"卡在 0% 直到结束"）：
  ```text
  -- Translate --                    # 转译开始（顺带 DROP/CLAMP 计数）
  -- Probe --                        # 校验
  -- Render --                       # 出片开始
  frames A..B: NN%                   # 与 chrome 同形的分块进度 → jobs.progress()
  chunk#N: ...%                      # 有分块时
  ✂ DROP 9 / CLAMP 3 / NOTE 16       # 三级清单摘要（info 级）
  FINISHED: <产物路径>                # 终态
  ```
  **这条契约留给本仓**：在 `run_dhampir` 里把底座的输出**翻译**成上面的形状，别要求底座为 YekiTrim 定制输出。
- 完整清单落 `<项目>/data/dhampir/report.json`（机器可读，D4 的"当构建产物接进流程"就靠它）。
- 失败文案带 `RenderOutput::tail` 最近 30 行 + **三种前置缺失分开报**（缺 node / 缺底座 / schema 不符）。

### 2.3 WebUI 入口（注意右栏宽度预算）

⚠️ `webui/src/views/AGENTS.md` 钉着「区域与落点那一行**四颗按钮必须同行**（行内容宽 266px）」——
**通道选择不许塞进那一行**。按序优先：
1. **渲染按钮的下拉/长按菜单**：`渲染（chrome）` / `渲染（实验·底座）`；
2. 产物卡上的"出片设置"一行（通道 + 就绪状态）；
3. 设置页给"默认通道"（**本轮不做**：默认永远是 chrome，见 D2）。

- 就绪位 false → **不显示**底座那一项；显式选过但当时不可用 → 置灰 + `title` 写明缺什么。
- ⚠️ **不显示 ≠ 静默回退**：选了底座就一定走底座、失败就失败。

### 2.4 路径面与安全（照三防线核一遍）

- 请求体**只加一个枚举字段 `renderer`** → **不需要动** `scope.rs::EXTRA_PATH_FIELDS` 与
  `api/config.rs::PROJECT_PATH_KEYS`。（若要加"底座 exe 路径"这类**路径**字段，**必须**同步三防线。）
- 转译产物固定在 `<项目>/data/dhampir/`；清理/删除片段的逻辑**不要**顺手删它（它是这份工程的接口产物，
  要删就跟 `polish.toml` 一起删）。

### 2.5 server 验收

```bash
cargo test -p yekitrim-polish --lib render     # run_dhampir 单测（用假底座脚本注入，不依赖真底座）
cargo test -p yekitrim-server                  # 白名单：非法 renderer → 400（不是静默 chrome）
npm --prefix webui run build && node webui/scripts/check-overlay.mjs
python3 scripts/check_module_map.py && python3 scripts/check_module_boundaries.py
```
逐条要看的：① 非法 `renderer` → **400**；② 取消后 `tasklist | findstr dhampir node` **无残留**
（`kill_tree` 要覆盖到孙进程 —— 底座还会拉起它自己的 GPU 进程）；③ 任务日志有 DROP/CLAMP 逐条；
④ 进度条真的在动；⑤ 产物卡标出了通道。

## 三、落地顺序（每步独立验收；D2 允许把它排在主线功能之后）

1. **P1-a**：`RenderChannel` + `run_dhampir`（只做转译 + probe，render 先返回"未实现"）+ 单测。
   → `cargo test -p yekitrim-polish --lib render` 绿；缺 Node 的文案可操作。
2. **P1-b**：接 `dhampir render` + 产物登记（带通道）+ `polish translate` + `doctor` 的 `[dhampir]` 块 + `--strict`。
   → §1.4 四条。
3. **P2-a**：`run_polish_child` 透传 `--renderer` + 里程碑 + 输出翻译层 + `report.json`。
   → 任务日志/进度/取消三条。
4. **P2-b**：health + `GET /api/polish/dhampir` + 前端入口（§2.3）+ 产物卡通道标记。
   → `npm run build` + 两条门禁 + 真跑一次出片。
5. **P1.5**（竖屏配置化，主体在转译器侧）：`polish process` 把源尺寸写进 `polish.toml`、转译器按它过滤运镜；
   CLI/server 侧只加"缺字段时提示跑一次 process"。
   → 横素材（含 demo）产物**逐字节不变**。
6. **P3**：对拍基线（口径见 `dhampir-integration.md` P3）。
7. **P4**：D1 第 2 步（去 Node）= Rust 原生转译 + `scripts/check-dhampir-parity.mjs` **常驻绿**
   （Rust 产物 vs JS 产物逐字节；JS 那份保留不删）。
   → 产品在没装 Node 的机器上能出片。

## 四、已定 vs 待做（原开放问题）

| 项 | 状态 |
|---|---|
| **D1 去 Node** | 已定方向（**原生 Rust 推荐**，`.mjs` → 可内嵌 `.js` 为备选）；**JS 工具脚本保留**（底座侧测试 + Rust 的 parity oracle）。本计划第 1 步仍按"spawn node"写（底座当前只是测试通道），去 Node 是 **P4**，门槛见 `dhampir-integration.md` §3.1 |
| **D2 通道定位** | 已定：chrome 主线、底座实验通道、远期替代。影响见各节 |
| **D3 产物位置** | 已定：`<项目>/data/dhampir/` |
| **D4 `DROP>0`** | 已定：默认警告继续，`--strict` 才拒（CLI 暴露；server 暂不暴露，CI 走 CLI） |
| **D5 竖屏** | 已定：**不探媒体**，由上层配置给（`polish.toml` 的 `[source] width/height`，`polish process` 写入）+ 转译器据此**只过滤运镜**。属 **P1.5**，本计划的 CLI/server 侧只需在"缺字段"时给提示 |
| 底座进度行的归属 | 建议：**本仓翻译它的输出**（契约留本仓）——待底座侧确认一次它的输出形状即可 |
| 通道选择放哪 | 建议：先做"渲染按钮下拉"，设置页默认值等有人抱怨再说 |

## 五、远期"替代 chrome"的切换清单（D2 的目标，先写下来）

替代不是改一个默认值，要同时满足：

1. **P3 对拍基线稳定**：≥3 个真实工程、两条通道逐帧比在阈值内（且报告能指出差异属于哪个事件）。
2. **§五差异清零**（或逐条有明确"接受"记录）：尤其**字幕折行/缩字**；「源视频竖」那条已在 **P1.5**
   变成"配置驱动 + 只过滤运镜"，替代前只需确认竖素材工程在两条通道上**观感一致**。
3. **P4 完成**：产品在**没装 Node** 的机器上能出片，且 `scripts/check-dhampir-parity.mjs`（Rust vs JS 逐字节）
   **常驻绿**。
4. **四件事验收过**：进度（不是"0% 直到结束"）、取消（无残留进程）、产物（带通道 + 不覆盖）、
   报告（DROP/CLAMP 可见 + `report.json`）。
5. **默认值切换**：`RenderChannel` 默认从 `Chrome` 改成 `Dhampir`，**chrome 退成兜底**
   （底座不可用时明确提示，而不是静默换路）；同时更新 `doctor`、文档、以及"产物来自哪条通道"的展示。
6. **回归网**：把 P3 的对拍 + parity 守卫一起跑成 CI 门禁（红就是退化），否则替代之后没有基线可比。

---

## 六、2026-09-30：**默认通道已切到底座**（用户拍板，先于 §五 的部分前置）

用户直接要求"将 dhampir 的渲染导出接入、切换为默认"。已落地的部分：

| 项 | 状态 |
|---|---|
| CLI | `polish render` 的 `--renderer` 默认 `dhampir`；`wgpu`/`native` 视作 dhampir；**认不出的值在 `pipeline.rs` 一处报错**（不静默换路）。`--renderer chrome` 是逃生口，报错文案里会写出来 |
| 服务端 | `polish_render` 读 `body.renderer`（白名单 `dhampir`/`chrome`，认不出 → **400**；不给就不带 `--renderer`，沿用 CLI 默认）；`run_polish_child(..., extra)` 透传；`RENDER_MILESTONES` 认底座的 `-- Dhampir …` |
| 前端 | 渲染导出弹窗里加"出片通道"下拉（默认项 = 跟随默认；`chrome` = 兜底），`runRender(..., renderer)` |
| 进度 | 底座的 NDJSON 被翻译成 `frames A..B: NN%`（落文件 + tail，**不用管道**）⇒ WebUI 进度条真的会动 |
| 文档 | 根卡、`polish-render`、`cli`、`server-api`、`webui-api`、`webui-views` 六张卡同步 |

**真跑过一次端到端**（demo 工程，12s / 1080p60，`VTEDIT_DHAMPIR` 指向同层源码树）：

```
Renderer: dhampir  →  转译 0.2s  →  底座出画面 11.3s（frames 0..719 进度 0→100%）
→ 合流 0.2s → 封面 3.4s → FINISHED: export.mp4 (7.4MB, 总计 15s)
产物：1920×1080 / 60fps / 720 帧 / duration 12.000000，含音轨 ✓
```

**这趟真跑抓出三个只有真跑才会暴露的 bug（都已修）**：

1. **`find_binary` 在源码树下永远找不到底座**：`<程序目录>/dhampir/bin/<exe>`（发布包形状）
   与 `VTEDIT_DHAMPIR_CLI`（显式指定）混用同一条 `bail!` —— 而 `current_exe()` 总能算出前者，
   于是它一 bail，后面的回退链（同层 `Dhampir/target/{release,debug}` → PATH）一次都没跑到。
   现在两者分开：**显式指错才报错，约定位置不在就继续往下找**。
2. **`--to` 按 30fps 估算 → 只出一半**：`cmd_render` 里 `fps` 写死 30，而转译器写的时间基是
   **素材自己的**（demo 是 60/1）—— 12 秒的片子只出 360 帧，音轨照旧 12 秒。
   现在帧数由 `dhampir::sequence_frames(&translated)` 从**转译产物**里读（所有层 `end` 的最大值）。
3. **相对工程目录 → 成片路径被拼两遍**：`resolve_paths` 会把 `project.output` 归一成
   `<工程目录>/export.mp4`，而合流前那一步"不是绝对路径就再 join 一次工程目录"于是写出
   `<dir>\<dir>\export.mp4`（`No such file or directory`）。CLI 的默认输入就是 `.`，所以这条
   一定会被踩到；现在 `cmd_render` 开头 `std::path::absolute` 一次。

**还没做**（§五 里的其余前置，逐条仍是切换后要补的）：

- **P3 对拍基线**：≥3 个真实工程、两条通道逐帧比在阈值内 —— 目前只有 demo 这一份跑通了链路，
  **没有做过像素比对**。底座不是逐字节复刻参照，所以"默认已切"目前是**用户拍板 + 链路可用**，
  不是"差异已清零"。
- **P4 去 Node**：转译器仍是 `node tools/polish-to-dhampir.mjs`，所以出片机器**必须有 Node 18+**。
- **就绪位**：`/api/health` 与 `GET /api/polish/dhampir` 还没加；现在底座不可用是**渲染时报错**
  （文案可操作：设 `VTEDIT_DHAMPIR*` 或改 `--renderer chrome`），前端无法提前置灰那一项。
- `polish doctor` 的 `[dhampir]` 状态块、`--strict`（DROP>0 拒绝）也还没做。

