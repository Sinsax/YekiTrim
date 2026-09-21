# vtrim-cli — 只剩入口（clap 命令 + tracing 装配），二进制名 `vtrim`

> 范围：`crates/vtrim-cli`/** ｜ 上游：用户与脚本（无 crate 依赖它）｜ 验证：`cargo check -p vtrim-cli`

## 职责

命令行入口：把 clap 解析出来的命令**转发**给 `vtrim_service` / `vtrim_polish` / `vtrim_server`，
并把 `tracing` 订阅者（终端 + 进程内日志总线）与进度条装上。**这里不实现任何领域逻辑** ——
`vtrim user *` / `vtrim usage` / `vtrim doctor` / `vtrim ingest *` 的实现都在 `vtrim-service`，
精修命令在 `vtrim-polish`，HTTP 服务端在 `vtrim-server`。

## 入口

- **二进制**：`[[bin]] name = "vtrim"`，`src/main.rs`（`src/cli.rs` = clap 定义，`src/progress.rs` = 进度条）。
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
- **服务端转发**：`Command::Server(args) => vtrim_server::run(args.clone()).await`；
  `ServerArgs` 就是 `vtrim_server::ServerArgs`（clap 直接 derive 在它上面）。
- **tracing 装配**（`setup_tracing()`）：`registry + EnvFilter + fmt::layer + vtrim_service::logbus::LogBusLayer::new(bus())`。
  `LogBusLayer` **必须挂上**：`vtrim server` 的访问日志、任务日志、错误文案都靠它进设置页「运行日志」与日志文件；
  对纯 CLI 命令它只是多一份内存环形缓冲，没有副作用。全局过滤器来自 `RUST_LOG`（**默认 `info`**），
  想看高频接口的逐条访问日志用 `RUST_LOG=vtrim=debug`。
- **进度**：`progress::IndicatifProgress`（实现 `vtrim_core::progress::ProgressReporter`），
  在 `main` 里建一份 `Arc<dyn ProgressReporter>`，转发给 `run_stages_with[_ext]`。
- **CLI 侧用量记账**：`vtrim_service::usage::try_install_cli_sink()` 的安装点在**四类分支**——
  `process` / `detect` / `correct` / `polish *`。

## 边界

- ⚠️ **CLI 只用 server 的入口**：`vtrim_server::` 只许出现 `run` / `ServerArgs`，不许直接调 server 内部模块。
  这是机器门禁（`cli 只用 server 的入口`）：`python3 scripts/check_module_boundaries.py`。
  `vtrim-cli` 本来就依赖 `vtrim-core` / `vtrim-polish`（入口装配要用），但**服务端能力**只能经
  `vtrim_server::run()` / `ServerArgs` 进。
- ⚠️ **`wgpu-render` 是转发，不是自己启用**：`#[cfg(feature = "wgpu-render")]` 全在 `vtrim-server`（8 处）。
  这里必须把 feature 递下去，否则 `cargo build --features wgpu-render` 照样构建成功，
  但 GPU 后端不会被编译进 server —— `/api/health` 的 `wgpu_render` 报 false、静默退回。
  声明只有一行：`wgpu-render = ["vtrim-server/wgpu-render"]`。
- ⚠️ **端口/默认地址一律引用 `vtrim_service::doctor::{DEFAULT_PORT, DEFAULT_URL}`，别写字面量**：
  `Command::Doctor` 的 `#[arg(long, default_value = vtrim_service::doctor::DEFAULT_URL)]`，
  `run_polish_preview` 回退起服务时也用 `port: vtrim_service::doctor::DEFAULT_PORT`
  （唯一一份定义在 service 侧，见 `crates/vtrim-service/AGENTS.md`）。
- ⚠️ **新增会走 LLM 的 CLI 分支要记得挂 `try_install_cli_sink()`**，否则用量统计里没有它
  （写库失败只 warn，**绝不影响调用结果**）。
- `vtrim init` 走的是**命令自带的 `--config`**（`Command::Init.config` = `vtrim.toml` 的**写出路径**，
  缺省取输入视频的同级目录），**不读**全局 `-c/--config`；其余分支才用全局 `cli.config` 当配置路径。
- `polish preview` 是**两条路**：先在输入目录向上找 `vtrim.toml` —— 找到就起 `vtrim_server::run()`
  打开同一 WebUI 精修页（`ServerArgs { host: "127.0.0.1", port: DEFAULT_PORT, open: true, config: Some(cfg), … }`，
  路径经 `percent_encode_component` 拼 `/#/project/.../polish?dir=`）；找不到才回退旧的独立
  `vtrim_polish::commands::cmd_preview`，保证命令仍然可用。
- `polish llm` 在 CLI 保留**断点续传**（`cmd_llm(input, false)`）——长片重跑很贵；编辑器那颗按钮走的是 `full=true`。
- `polish render` 用 `std::env::set_var("VTEDIT_RENDERER", renderer)` 传后端（`--renderer chrome|wgpu`，默认 `chrome`）。
- `Command::Status` 自己组装输出：`vtrim_service::Workspace::load` + `project_snapshot`（`current_stage` 由
  输出文件存在性推导），再读 `output/transcript.json` 的 `asr_metadata` 打印模型/锚点覆盖率/降级原因。

> 改动只允许落在本目录；跨模块改动先读对方卡，并跑两条门禁。

## 验证

```bash
cargo check -p vtrim-cli
```

只做入口装配，所以没有单测、只要求编译通过（0 warnings）。改了命令面或 `ServerArgs` 的字段，
顺手跑一次 `cargo run -- --help`；动了 build feature 要实测 `/api/health` 的 `wgpu_render` 那一条。
