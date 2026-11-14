# yekitrim-cli — 只剩入口（clap 命令 + tracing 装配），二进制名 `yekitrim`

> 范围：`crates/yekitrim-cli`/** ｜ 上游：用户与脚本（无 crate 依赖它）｜ 验证：`cargo check -p yekitrim-cli`

## 职责

命令行入口：把 clap 解析出来的命令**转发**给 `yekitrim_service` / `yekitrim_polish` / `yekitrim_server`，
并把 `tracing` 订阅者（终端 + 进程内日志总线）与进度条装上。**这里不实现任何领域逻辑** ——
`yekitrim user *` / `yekitrim usage` / `yekitrim doctor` / `yekitrim ingest *` 的实现都在 `yekitrim-service`，
精修命令在 `yekitrim-polish`，HTTP 服务端在 `yekitrim-server`。

## 入口

- **二进制**：`[[bin]] name = "yekitrim"`，`src/main.rs`（`src/cli.rs` = clap 定义，`src/progress.rs` = 进度条）。
- **两个 bin，同一个 crate**：
  · `yekitrim`（`src/main.rs`）= 控制台 CLI（**唯一**用 `tracing_setup::setup(true)` 的地方）；
  · **`yekitrim-tray`（`src/tray.rs`）= Windows 托盘「薄启动器」**（2026-10 改版）：
    双击即后台起 `yekitrim.exe server --local-no-auth --open`，托盘左键开网页、右键「实际地址（灰显）/ 打开网页 / 退出」。
    ⚠️ **它刻意不 `use` 任何 `yekitrim-*` crate**（连 `yekitrim-core::path` 都不碰）。这不是洁癖：
    `yekitrim-core` → `sherpa-onnx`（静态链接的 C++ ASR）是整条依赖图的根，引用 core 的**任何一个符号**
    都会把 ASR 一起链进来 —— 上一版就是这样：托盘 **57 MB**、和主程序一样重。现在 **0.49 MB**。
    代价是两处**刻意抄的规则**，改一处要改两处：
      ① 数据目录（`YEKITRIM_DATA_DIR` ?? exe 同级，只为放 `logs/tray.log`）—— 正文在 `docs/file-layout.md`；
      ② `CREATE_NO_WINDOW`（它用不了 `process::cmd()`，见下面门禁的例外）。
    ⚠️ **跨进程两条契约**（本文件与 `yekitrim-server` 各写一份字面量，改一处必须改两处）：
      ① 关停令牌走 **环境变量 `YEKITRIM_SHUTDOWN_TOKEN`**（不用命令行参数：本仓"秘密不进进程列表"）；
      ② 实际监听地址从子进程 **stdout 的 `WebUI: http://…`** 那行抓（端口被占时 server 会自己换端口，
         所以**不能**假定就是请求的那个）。改那行日志文案会静默打断托盘 → 表现是"打开网页"点不动。
    ⚠️ **端口不写字面量**：只有 `YEKITRIM_PORT` 设了才给子进程 `--port`，否则让 server 用自己的权威常量。
    ⚠️ **两个管道都必须持续排空**（`pump()` 起线程读到 EOF）：管道满了子进程会**卡死**。
    ⚠️ **`quit()` 收完子进程必须自己 `PostMessageW(WM_CLOSE)`** 打破消息循环 ——
      旧（同进程）设计里是 server 线程干的，现在托盘进程里没有 server 线程；"看门狗"也指望不上
      （`quit()` 已经把 child 从 `Shared` 里 `take()` 走了）。漏了的症状：**子进程已优雅退出、锁也干净，托盘进程却一直挂着**。
    ⚠️ **子进程必须放进 `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` 的 Job**（`make_kill_on_close_job`）：
      任务管理器"结束任务"是 `TerminateProcess`，父进程**没有任何机会收尾**（连 Drop 都不跑），
      只能靠系统 —— job 句柄一关系统就把 job 里的进程全杀。**句柄故意不显式关**（存 `Mutex<isize>`，
      没有 Drop）：进程退出时系统关它，那一刻才生效；正常退出时子进程早收尾了，这一刀砍空。
      ⚠️ 它拉起的孙进程（ffmpeg/底座）也在同一 job 里 —— 这正是要的（别留孤儿 ffmpeg）。
      ⚠️ 建 job / 放进程**失败只记日志、不拦启动**（外层可能已有不允许 breakaway 的 job）：
      退化成"托盘被杀后服务会留着"，而不是托拉起不来。实测：强杀托盘后子进程**一起消失**，
      残留锁在下次启动时被自动接管。
    ⚠️ **开浏览器用 `ShellExecuteW`**，别退回 `cmd /c start`：本进程无控制台，起 `cmd.exe` 会**闪黑框**。
    ⚠️ **托盘图标取 exe 自带的图标资源**（`LoadIconW(hinst, 1)`）→ 内嵌 `.ico` → exe 同级文件 → 系统默认。
      别退回"只读 exe 同级那个 .ico"：开发时 `target/release/` 下没有它，而**没图标是静默的**。
    ⚠️ `#![cfg_attr(windows, windows_subsystem = "windows")]` 是 **crate 根**属性；因为它是**独立 bin**，
      `yekitrim.exe` 那个控制台程序才没被一起变成无控制台。
    ⚠️ `yekitrim server` 与 `polish preview` **仍然要登录** —— 免登录只是托盘那条路。
    ⚠️ windows-sys **钉 0.52**（那个版本句柄是 `isize`；0.59+ 改指针包装，代码对不上）。
- **`src/tracing_setup.rs` 只给 `yekitrim` 这个 bin 用**（`setup(true)` = 终端 + 日志总线）。
  ⚠️ **托盘不再用它**：薄启动器不能链接 `yekitrim-service`（`LogBusLayer` 在那儿），
  否则 ASR 被一起拉回来。它自己往 `<数据目录>/logs/tray.log` 追加几行（`tray.rs::log`）。
- **命令面**（`cli.rs` 的 `Cli` / `Command`，改命令先改这里）：
  `init <video> [--danmaku <file>] [--force]` / `extract [-i] [-o]` / `process [-i] [-o] [--dry-run]` /
  `transcribe [-i]` / `detect [-t <transcript>]` / `cut [-i] [-e <events>] [-o]` /
  `silence [files...] [--duration 3.5] [--threshold]` / `download` / `status` / `usage [--days N]` /
  `doctor [--url <base>]` / `ingest token [--disable]` / `ingest status` /
  `user list|info|add|passwd|check|disable|init-admin|reset-admin-password|invite|invites|invite-revoke` /
  `config init [--path]` / `config show` / `config migrate [路径…]` / `correct [-t <transcript>]` / `server <ServerArgs>` /
  `polish init|llm|process|render|cover|preview|doctor|test` / `polish stickers list|test|scan|build` /
  `fonts list|install [--id]`。
  `stage_name(num)`（1..4 → AudioExtractor/Transcriber/EventDetector/Cutter）在 `cli.rs`。
  所有 stage 命令的 `-i` 可选，默认从 `yekitrim.toml` 的 `input_file` 读取。
  **各命令是什么**（原先记在根卡里 —— 模块内容归模块卡，这次迁 `tools/` 时为最坏指令链预算把它挪回来）：
  `init` 建 `yekitrim.toml`；`extract`/`transcribe`/`detect`/`cut` = Stage 1~4
  （`detect`/`cut` 的自动发现顺序：参数 > config > `output_dir` 里的 json > `*.srt`）；
  `process` 全自动管线（`--dry-run` = 只到 Stage 3。⚠️ 它是**补齐未完成的阶段**：`current_stage` 已经
  覆盖到的阶段一律"已存在，跳过"—— 改完 prompt / 模型想重跑，得用单级命令；根卡那句"无视检查全部重跑"
  与实际不符，已在本卡纠正）；`silence`「去静音」**只写 `polish.toml`、不改视频**；
  `download` 把全部 ASR 模型下到 `<程序>/.yekitrim/models/`；`status` 看当前进度；
  `config migrate` = **把已下线的键从配置文件里真清掉并写回**（工程级配置按设计不自动写回，见 core/config 卡）；
  `usage` LLM 用量（token/缓存命中/耗时，账本 `llm_usage`）；`doctor` 安全与运行状况摘要；
  `ingest` 录播姬 webhook / 采集配置概览；`user invite|invites|invite-revoke` = 邀请码
  （**明文落库、可重复查看**）+ 注册开关；`config` 全局配置生成 / 显示；`correct` LLM 订正转录文本；
  `polish stickers list|test|scan` 本地贴纸（列出 / 测 keyword 匹配 / 生成清单）。
- **服务端转发**：`Command::Server(args) => yekitrim_server::run(args.clone()).await`；
  `run()` = `run_with(args, ctrl_c(), |_| {})` —— `shutdown` 一到就**优雅关停**（限时 `SHUTDOWN_GRACE`），
  于是 Ctrl+C 不再让 `InstanceLock` 的 Drop 跑不到、下次启动少一条"残留锁"WARN；
  `on_listening(url)` 回调给的是**实际**监听地址（端口被占时服务端会自己换一个）。
  ⚠️ 门禁 `cli 只用 server 的入口` 放行的是 `yekitrim_server::{run*, ServerArgs}`；
  `ServerArgs` 就是 `yekitrim_server::ServerArgs`（clap 直接 derive 在它上面）。
  ⚠️ **托盘一个 `yekitrim_server::` 都不用了**（它只 spawn `yekitrim.exe`）—— 门禁照样过（0 命中）。
- **tracing 装配**：`tracing_setup::setup(terminal)`（`registry + EnvFilter + [fmt::layer] + LogBusLayer`）。
  `LogBusLayer` **必须挂上**：`yekitrim server` 的访问日志、任务日志、错误文案都靠它进设置页「运行日志」与日志文件；
  对纯 CLI 命令它只是多一份内存环形缓冲，没有副作用。全局过滤器来自 `RUST_LOG`（**默认 `info`**），
  想看高频接口的逐条访问日志用 `RUST_LOG=yekitrim=debug`。
- **进度**：`progress::IndicatifProgress`（实现 `yekitrim_core::progress::ProgressReporter`），
  在 `main` 里建一份 `Arc<dyn ProgressReporter>`，转发给 `run_stages_with[_ext]`。
- **CLI 侧用量记账**：`yekitrim_service::usage::try_install_cli_sink()` 的安装点在**四类分支**——
  `process` / `detect` / `correct` / `polish *`。

## 边界

- `polish cover` 有 **`--renderer takumi`**（**唯一一个值**；不传 = `cover.rs` 的默认链：显式参数 > `VTEDIT_COVER_RENDERER` > takumi）。⚠️ 老脚本里的 `--renderer chrome` 现在会由 `cover.rs` **明确报错**（那条 headless Chrome 截图腿 2026-10-01 整条删除）。⚠️ **封面与视频的出片通道互相独立**：视频走底座时封面仍走 Takumi（静态光栅化），两者是不同的引擎，别把它们绑一起。

- ⚠️ **CLI 只用 server 的入口**：`yekitrim_server::` 只许出现 `run` / `run_with` / `ServerArgs`
  （门禁正则是 `(?!(run|ServerArgs))`，`run*` 一族都放行），不许直接调 server 内部模块。
  这是机器门禁（`cli 只用 server 的入口`）：`python3 scripts/check_module_boundaries.py`。
  `yekitrim-cli` 本来就依赖 `yekitrim-core` / `yekitrim-polish`（入口装配要用），但**服务端能力**只能经
  `yekitrim_server::run()` / `ServerArgs` 进。
- ⚠️ **`wgpu-render` 转发链已删除**：那条 GPU 出片链路（`yekitrim-server` 的 `api/` ×3 与 `model.rs` ×5、
  `yekitrim-polish/wgpu-render`、`yekitrim-render-core/wgpu`）整条没了 —— GPU 那侧交给独立的 Dhampir 底座。
  所以 `[features]` 里**不再有 GPU 那条**，`/api/health` 也不再回 `wgpu_render`。
  现在只有一条**封面**转发 `cover-takumi = ["yekitrim-polish/cover-takumi"]`，且 **CLI `default` 带上它**
  （`cargo run --bin yekitrim server` 开箱可选 Takumi；库构建仍精简）。代价：Windows 默认构建会多一条
  `LNK4098` 链接器警告（libwebp-sys 静态 CRT）—— `cargo check` 不链接所以仍是 0 warning。
  **别为了"留个口子"把它加回来**（加了不报错，只会让人以为还有 GPU 出片）。
- ⚠️ **端口/默认地址一律引用 `yekitrim_service::doctor::{DEFAULT_PORT, DEFAULT_URL}`，别写字面量**：
  `Command::Doctor` 的 `#[arg(long, default_value = yekitrim_service::doctor::DEFAULT_URL)]`，
  `run_polish_preview` 回退起服务时也用 `port: yekitrim_service::doctor::DEFAULT_PORT`
  （唯一一份定义在 service 侧，见 `crates/yekitrim-service/AGENTS.md`）。
- ⚠️ **新增会走 LLM 的 CLI 分支要记得挂 `try_install_cli_sink()`**，否则用量统计里没有它
  （写库失败只 warn，**绝不影响调用结果**）。
- `yekitrim init` 的 `--config` 是 **`yekitrim.toml` 的写出路径**（缺省取输入视频的同级目录），不读全局
  `cli.config`；⚠️ 但它和全局 `-c/--config` 在 clap 里**是同一个 id**，所以 `-c` 写在**子命令之前**时
  也会被 init 采用（实测 `yekitrim -c A.toml init v.mp4` ⇒ 写到 A.toml；子命令自己的 `--config` 优先）。
  `init` **之后**的 `-c` 会被 clap 直接拒（那个位置只有 `--config`）。其余分支一律用全局
  `cli.config` 当**读**的配置路径。
- `polish preview` 是**两条路**：先在输入目录向上找 `yekitrim.toml` —— 找到就起 `yekitrim_server::run()`
  打开同一 WebUI 精修页（`ServerArgs { host: "127.0.0.1", port: DEFAULT_PORT, open: true, config: Some(cfg), … }`）；
  找不到才回退旧的独立 `yekitrim_polish::commands::cmd_preview`，保证命令仍然可用。
  ⚠️ **打开的是哪一页（代价：一条曾经静默失效的路由）**：WebUI v5 起只认数字项目 id ——
  `webui/src/router.js` 对 `/project/:cfg/:rest` 里**非数字**的 `:cfg` 一律 redirect 到 `/workbench`，
  而且**没有任何视图再读 `?dir=`**。所以旧实现拼的 `/#/project/<配置路径>/polish?dir=<片段目录>`
  永远落工作台首页（命令退出 0、页面却不是那个片段）。现在 `polish_preview_path` 拼
  `/#/p/<pid>/polish[/<clip>]`：`pid` 由 `project_id_for_config` 查账本 `projects` 表
  （**只读、不建库**），`:clip` 只取**目录名**并 percent-encode（精修页拿它匹配 `clips[].name`，
  传绝对路径永远匹配不上；vue-router 会把 `%20` 解回来）。工程还没登记进项目库 ⇒ 退回 `/#/workbench`
  并打一条 info（回去还能导入工程）。**改路由先看 `webui/src/router.js`**，别凭记忆调。
- `polish llm` 在 CLI 保留**断点续传**（`cmd_llm(input, false)`）——长片重跑很贵；编辑器那颗按钮走的是 `full=true`。
- `polish render` 用 `std::env::set_var("VTEDIT_RENDERER", renderer)` 传后端。⚠️ **默认 `dhampir`**
  （**出片只剩这一条腿**，2026-09-30 用户拍板切换）；老写法 `wgpu` / `native` 视作 dhampir，
  `chrome` 是本仓那条**已整条删除**的兜底腿 —— 现在 `RenderBackendKind::parse("chrome")` 返回 `None`、
  出片**明确报错**（不静默改走底座）
  （它们本来就是"用 GPU 出片"，而 GPU 出片现在只有底座那一条路）。**认不出的值由 `pipeline.rs`
  一处报错**（不静默换路）—— 校验别在 `cli.rs` 里再抄一份。
  ⚠️ `--strict` 走 `VTEDIT_STRICT=1`：转译报告里 `DROP > 0` 就拒绝出片（判据在
  `dhampir.rs`，CLI 只是传个开关）。`polish doctor` 里也有 `[dhampir]` 块（读
  `dhampir::readiness()` 的同一份判断）。
- `Command::Status` 自己组装输出：`yekitrim_service::Workspace::load` + `project_snapshot`（`current_stage` 由
  输出文件存在性推导），再读 `output/transcript.json` 的 `asr_metadata` 打印模型/锚点覆盖率/降级原因。
  ⚠️ 两条口径：① `current_stage` = **最后跑完的** stage（状态机 `stage > current + 1` 才拒绝）⇒
  标记走 `stage_mark`：`num <= current` ✓、`num == current + 1` →、其余 ·（以前最后跑完那一级被标成
  "正在跑"、真正的下一步是 ·，而且每行打了两遍标记）；② `Workspace::load` / `load_config_at`
  **刻意容忍配置不存在**（server 要先能算路径），但 `status` 是在回答"这个工程走到哪了" ——
  配置文件不存在时**报错并非零退出**，否则"找错目录"会被伪装成"全新工程"（脚本里
  `status && 下一步` 也跟着错）。

> 改动只允许落在本目录；跨模块改动先读对方卡，并跑两条门禁。

## 验证

```bash
export SHERPA_ONNX_LIB_DIR=/home/toki/project/sherpa-onnx-prebuilt-no-tts/build/cmake-build/install/lib
cargo check -p yekitrim-cli --all-targets   # 0 warnings
cargo test -p yekitrim-cli                  # 纯函数单测（路由拼装 / status 标记 / 项目 id 查询）
```

只做入口装配，所以绝大多数东西没有单测 —— 但**拼 URL、算标记、查账本这类纯逻辑有**
（`main.rs` 的 `tests`：`polish preview` 的目标路由、`status` 的标记口径、项目 id 查询）。
改了命令面或 `ServerArgs` 的字段，顺手跑一次 `cargo run -- --help`。
