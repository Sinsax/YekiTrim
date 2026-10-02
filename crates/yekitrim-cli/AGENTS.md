# yekitrim-cli — 只剩入口（clap 命令 + tracing 装配），二进制名 `yekitrim`

> 范围：`crates/yekitrim-cli`/** ｜ 上游：用户与脚本（无 crate 依赖它）｜ 验证：`cargo check -p yekitrim-cli`

## 职责

命令行入口：把 clap 解析出来的命令**转发**给 `yekitrim_service` / `yekitrim_polish` / `yekitrim_server`，
并把 `tracing` 订阅者（终端 + 进程内日志总线）与进度条装上。**这里不实现任何领域逻辑** ——
`yekitrim user *` / `yekitrim usage` / `yekitrim doctor` / `yekitrim ingest *` 的实现都在 `yekitrim-service`，
精修命令在 `yekitrim-polish`，HTTP 服务端在 `yekitrim-server`。

## 入口

- **二进制**：`[[bin]] name = "yekitrim"`，`src/main.rs`（`src/cli.rs` = clap 定义，`src/progress.rs` = 进度条）。
- **命令面**（`cli.rs` 的 `Cli` / `Command`，改命令先改这里）：
  `init <video> [--danmaku <file>]` / `extract [-i] [-o]` / `process [-i] [-o] [--dry-run]` /
  `transcribe [-i]` / `detect [-t <transcript>]` / `cut [-i] [-e <events>] [-o]` /
  `silence [files...] [--duration 3.5] [--threshold]` / `download` / `status` / `usage [--days N]` /
  `doctor [--url <base>]` / `ingest token [--disable]` / `ingest status` /
  `user list|info|add|passwd|check|disable|init-admin|reset-admin-password|invite|invites|invite-revoke` /
  `config init [--path]` / `config show` / `correct [-t <transcript>]` / `server <ServerArgs>` /
  `polish init|llm|process|render|cover|preview|doctor|test` / `polish stickers list|test|scan|build`。
  `stage_name(num)`（1..4 → AudioExtractor/Transcriber/EventDetector/Cutter）在 `cli.rs`。
  所有 stage 命令的 `-i` 可选，默认从 `vtrim.toml` 的 `input_file` 读取。
  **各命令是什么**（原先记在根卡里 —— 模块内容归模块卡，这次迁 `tools/` 时为最坏指令链预算把它挪回来）：
  `init` 建 `vtrim.toml`；`extract`/`transcribe`/`detect`/`cut` = Stage 1~4
  （`detect`/`cut` 的自动发现顺序：参数 > config > `output_dir` 里的 json > `*.srt`）；
  `process` 全自动管线（`--dry-run` = 只到 Stage 3）；`silence`「去静音」**只写 `polish.toml`、不改视频**；
  `download` 把全部 ASR 模型下到 `<程序>/.vtrim/models/`；`status` 看当前进度；
  `usage` LLM 用量（token/缓存命中/耗时，账本 `llm_usage`）；`doctor` 安全与运行状况摘要；
  `ingest` 录播姬 webhook / 采集配置概览；`user invite|invites|invite-revoke` = 邀请码
  （**明文落库、可重复查看**）+ 注册开关；`config` 全局配置生成 / 显示；`correct` LLM 订正转录文本；
  `polish stickers list|test|scan` 本地贴纸（列出 / 测 keyword 匹配 / 生成清单）。
- **服务端转发**：`Command::Server(args) => yekitrim_server::run(args.clone()).await`；
  `ServerArgs` 就是 `yekitrim_server::ServerArgs`（clap 直接 derive 在它上面）。
- **tracing 装配**（`setup_tracing()`）：`registry + EnvFilter + fmt::layer + yekitrim_service::logbus::LogBusLayer::new(bus())`。
  `LogBusLayer` **必须挂上**：`yekitrim server` 的访问日志、任务日志、错误文案都靠它进设置页「运行日志」与日志文件；
  对纯 CLI 命令它只是多一份内存环形缓冲，没有副作用。全局过滤器来自 `RUST_LOG`（**默认 `info`**），
  想看高频接口的逐条访问日志用 `RUST_LOG=yekitrim=debug`。
- **进度**：`progress::IndicatifProgress`（实现 `yekitrim_core::progress::ProgressReporter`），
  在 `main` 里建一份 `Arc<dyn ProgressReporter>`，转发给 `run_stages_with[_ext]`。
- **CLI 侧用量记账**：`yekitrim_service::usage::try_install_cli_sink()` 的安装点在**四类分支**——
  `process` / `detect` / `correct` / `polish *`。

## 边界

- `polish cover` 有 **`--renderer takumi`**（**唯一一个值**；不传 = `cover.rs` 的默认链：显式参数 > `VTEDIT_COVER_RENDERER` > takumi）。⚠️ 老脚本里的 `--renderer chrome` 现在会由 `cover.rs` **明确报错**（那条 headless Chrome 截图腿 2026-10-01 整条删除）。⚠️ **封面与视频的出片通道互相独立**：视频走底座时封面仍走 Takumi（静态光栅化），两者是不同的引擎，别把它们绑一起。

- ⚠️ **CLI 只用 server 的入口**：`yekitrim_server::` 只许出现 `run` / `ServerArgs`，不许直接调 server 内部模块。
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
- `yekitrim init` 走的是**命令自带的 `--config`**（`Command::Init.config` = `vtrim.toml` 的**写出路径**，
  缺省取输入视频的同级目录），**不读**全局 `-c/--config`；其余分支才用全局 `cli.config` 当配置路径。
- `polish preview` 是**两条路**：先在输入目录向上找 `vtrim.toml` —— 找到就起 `yekitrim_server::run()`
  打开同一 WebUI 精修页（`ServerArgs { host: "127.0.0.1", port: DEFAULT_PORT, open: true, config: Some(cfg), … }`，
  路径经 `percent_encode_component` 拼 `/#/project/.../polish?dir=`）；找不到才回退旧的独立
  `yekitrim_polish::commands::cmd_preview`，保证命令仍然可用。
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

> 改动只允许落在本目录；跨模块改动先读对方卡，并跑两条门禁。

## 验证

```bash
cargo check -p yekitrim-cli
```

只做入口装配，所以没有单测、只要求编译通过（0 warnings）。改了命令面或 `ServerArgs` 的字段，
顺手跑一次 `cargo run -- --help`。
