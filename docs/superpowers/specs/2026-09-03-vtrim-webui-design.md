# V-Trim WebUI + `vtrim server` 设计

日期: 2026-09-03
状态: 已批准（2026-09-03），M1 进行中

## 1. 背景与目标

V-Trim 目前是纯 CLI 工具，流程跨多个阶段（extract → transcribe → correct(可选) → detect → cut）和多个精修步骤（polish init/llm/process/render/preview/cover）。用户希望在本地通过浏览器可视化地驱动整个流程。

目标：

- 给 CLI 增加 `vtrim server` 子命令，启动一个本机 HTTP 服务。
- 制作零构建原生 WebUI（单个 HTML/CSS/JS，嵌入 Rust 二进制），覆盖：
  - 核心切片流程：工程创建、4 个 Stage、进度/日志、断点续传、事件时间线。
  - 精修流程：clip 选择、polish init/llm/process/render/preview/cover。
- 执行方式采用**复用现有 CLI/库函数/模块**，不通过子进程二次包装。
- 先完成可运行骨架，再逐步细化 UI 与编辑能力。

## 2. 范围与取舍

| 项目 | 决策 |
|---|---|
| 覆盖流程 | 核心切片流程 + polish 精修流程 |
| 执行模型 | 同一进程内复用现有模块，通过 JobManager 调度 |
| 前端 | 零构建原生 HTML/CSS/JS，无 node 工具链 |
| 项目模型 | 第一版以“一个活动工程/工作目录”为主，支持切换工程文件路径 |
| 日志 | 通过 tracing 层转发到当前 Job，前端 SSE/轮询查看 |
| 取消 | v1 提供“请求取消”，对异步可中断步骤立即生效；对正在运行的 ffmpeg 子进程只能尽量终止/阶段边界生效（现有核心使用 blocking Command，后续可迁移 tokio::process 增强） |
| 文件选择 | 浏览器只负责展示路径输入 + 服务端受限目录浏览，不提供大文件上传 |

## 3. 总体架构

```text
┌──────────────────────────────────────────────────────────────┐
│ vtrim (crates/vtrim-cli)                                      │
│                                                              │
│  CLI 入口 (clap)                                              │
│   ├─ 原有命令 ──────────────────────────────────────────────┐ │
│   │  init/process/extract/transcribe/detect/cut/status/... │ │
│   └─ vtrim server ─────────────────────────────────────────┼ │
│      ┌───────────────────────────────────────────────────┐ │ │
│      │ Axum HTTP Server (127.0.0.1:7788)                 │ │ │
│      │  /webui (index.html) + /api + SSE                 │ │ │
│      └──────────────────────┬────────────────────────────┘ │ │
│                             │ 调用                            │ │
│      ┌──────────────────────▼────────────────────────────┐ │ │
│      │ Service Layer (service/)                          │ │ │
│      │  Workspace / ProjectSnapshot / CorePipeline       │ │ │
│      │  PolishPipeline / ConfigEdit / Doctor             │ │ │
│      └──────────────────────┬────────────────────────────┘ │ │
│                             │ 复用                            │ │
│      ┌──────────────────────▼────────────────────────────┐ │ │
│      │ vtrim-core / vtrim-polish 现有模块                 │ │ │
│      │ PipelineRunner, Stage, polish commands, render... │ │ │
│      └───────────────────────────────────────────────────┘ │ │
└──────────────────────────────────────────────────────────────┘
```

CLI 命令与 HTTP API 共用 Service Layer，避免同一逻辑存在两份。

## 4. CLI 接口变化

```text
vtrim server [--host 127.0.0.1] [--port 7788] [--open]
             [--config <vtrim.toml>] [--no-open]
```

- `--config` 复用/兼容全局 `-c` 语义，未传时自动探测 `./vtrim.toml`。
- 默认只监听 `127.0.0.1`（本机工具，避免暴露到局域网）。
- `--open` 启动成功后自动打开默认浏览器。
- `Ctrl+C` 优雅停止：停止接收新 Job，已运行 Job 发取消请求，然后退出。

## 5. 代码结构规划

```text
crates/vtrim-cli/src/
├── cli.rs                  # 增加 Server 子命令
├── main.rs                 # Server 分支 -> server::run()
├── progress.rs             # 保留现有终端进度
├── service/
│   ├── mod.rs              # Service 入口：暴露给 CLI 和 HTTP 的领域函数
│   ├── workspace.rs        # 工程目录/相对路径解析（关键重构）
│   ├── snapshot.rs         # ProjectSnapshot / Status / clip 列表
│   ├── core_pipeline.rs    # 从 main.rs 抽出的核心流程函数
│   ├── polish_pipeline.rs  # polish 流程包装 + preview 拆分
│   ├── config_edit.rs      # vtrim.toml / .vtrim/llm.toml 读取与保存
│   ├── doctor.rs           # 系统/环境检查，返回结构化 JSON
│   └── jobs.rs             # JobManager、JobState、取消、SSE 事件
├── server/
│   ├── mod.rs              # axum Router / 启动
│   ├── api.rs              # REST handlers
│   ├── media.rs            # 受限文件服务（支持视频 range）
│   ├── sse.rs              # SSE 事件流
│   └── webui/
│       ├── index.html      # 单页 UI（内联 CSS/JS 或少量静态文件）
│       └── app.js / app.css # 可选分离，仍随二进制 include_str
└── webui_assets.rs         # include_str! 静态资源注入
```

vtrim-cli 保持 binary crate；Service 模块先作为 `mod service` 放在 binary 内，配合 `cargo test --bin vtrim` 做单元测试。若后续需要集成测试，再拆 `[lib]`。

## 6. Workspace 路径语义重构（必须先做）

现状：CLI 大量依赖“当前进程 CWD = 工程目录”，例如 `config::save_audio_path()` 写死 `./vtrim.toml`、`resolve_input()` 直接使用相对路径。Server 是长驻进程，不能靠反复 `set_current_dir` 切换工程。

设计引入：

```rust
pub struct Workspace {
    pub config_file: PathBuf,   // vtrim.toml 的绝对路径
    pub root_dir: PathBuf,      // config_file 所在目录
    pub output_dir: PathBuf,    // 绝对化后的输出目录
}

impl Workspace {
    pub fn load(flag: Option<&Path>) -> Result<Self>;
    pub fn resolve(&self, value: &str) -> PathBuf; // 相对 root_dir 解析
    pub fn to_toml(&self) -> Config;
}
```

配套改动：

- `vtrim-core::config` 增加路径参数化保存函数：
  - `save_audio_path_in(&config_file, &path)`
  - `save_transcript_path_in(&config_file, &path)`
  - `save_events_path_in(&config_file, &path)`
  - 保留现有无参函数作为“当前目录快捷方式”，CLI 行为不变。
- Service Layer 所有函数签名接收 `&Workspace`，传入 `PipelineContext` 的 input/output 统一为绝对路径。
- 读取配置文件从 `Workspace.config_file` 而非 CWD 的 `vtrim.toml`。
- 全局配置 `.vtrim/config.toml`、`.vtrim/llm.toml` 仍使用 `vtrim-core::path::vtrim_home()`，与工程目录无关。

这样 `vtrim server` 可通过 `POST /api/project/switch` 在同一个长驻进程中切换不同工程，而不会污染全局 CWD。

## 7. JobManager 与可视化进度

### 7.1 Job 状态

```rust
pub enum JobStatus {
    Queued,
    Running,
    AwaitingInput, // 需要断点续传决策
    Succeeded,
    Failed(String),
    Cancelled,
}

pub struct Job {
    pub id: String,             // uuid: run_xxx
    pub kind: JobKind,          // core_pipeline / polish_llm / render / download ...
    pub status: JobStatus,
    pub stage: Option<u32>,
    pub stage_name: Option<String>,
    pub progress: Option<(u64, u64)>,
    pub message: String,
    pub error: Option<String>,
    pub created_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
    pub logs: VecDeque<LogLine>, // 滚动保留最近 N 条
}
```

### 7.2 事件流

Job 运行期间向 `tokio::sync::broadcast::Sender<JobEvent>` 发送：

```json
{ "type": "log",         "level": "INFO", "message": "..." }
{ "type": "stage_started","stage": "Transcriber", "stage_number": 2 }
{ "type": "progress",     "stage": "Transcriber", "current": 3, "total": 10 }
{ "type": "stage_completed","stage": "Transcriber" }
{ "type": "awaiting_resume", "checkpoint": "events.checkpoint.json", "question": "发现事件检测断点，是否继续？" }
{ "type": "completed",    "result": { ... } }
{ "type": "failed",       "error": "..." }
```

前端通过 `GET /api/jobs/{id}/events`（SSE）订阅；不支持 SSE 时轮询 `GET /api/jobs/{id}`。

### 7.3 日志捕获方案

现有代码通过 `tracing` 输出日志，没有逐函数回调。为不全面改写所有模块，v1 采用：

1. Server 启动时设置自定义 `tracing_subscriber::Layer`，将格式化日志同时写入内存环形缓冲并广播到全局 `LogBus`。
2. JobManager 同一时间只允许一个长任务运行；任务开始时记录 `current_job_id`，日志层把该任务期间的日志挂到当前 Job。
3. 非任务期间的 HTTP 访问日志直接进入服务端控制台，不进入 Job 历史。
4. 进度数字继续使用现有 `ProgressReporter` 的实现：新增 `WebProgressReporter`，把 `stage_started/progress/stage_completed/error` 转发为 JobEvent，并保持与 CLI `IndicatifProgress` 平行。

> 已知限制：多个长任务并发时日志会混。v1 通过“单活动 Job”规避；未来可引入 span/task-local 做精细隔离。

### 7.4 断点续传决策

CLI 中 `confirm_resume()` 需要交互输入。Service Layer 改为显式策略：

```rust
pub enum ResumePolicy {
    Ask,      // CLI 交互；API 返回 AwaitingInput
    Resume,   // 自动继续已有断点
    Restart,  // 删除断点重新开始
}
```

`POST /api/pipeline/run` 的 body 中可传 `"resume": "auto" | "restart" | "ask"`：

- `auto`：有断点就继续。
- `restart`：有断点就删除。
- `ask`（默认前端交互）：检查到断点先返回 `awaiting_resume`，前端弹窗由用户选择继续/重来，再调用 `POST /api/jobs/{id}/decision`。

涉及断点：`correct.checkpoint.json`、`events.checkpoint.json`。

### 7.5 取消

- `POST /api/jobs/{id}/cancel` -> `JobManager` 将 Job 标记 `Cancelled` 并对 `JoinHandle::abort()`。
- 对 LLM agent 等异步步骤可较快生效；对正在执行的 blocking ffmpeg 命令，abort 要等该命令返回后才能到达阶段边界。
- 后续增强：把核心里 `std::process::Command` 的 ffmpeg 调用逐步替换为 `tokio::process::Command`，支持真正的子进程终止。

## 8. REST API 设计

### 8.1 系统与配置

| Method | Path | 说明 |
|---|---|---|
| GET | `/api/health` | `{ ok, version }` |
| GET | `/api/doctor` | ffmpeg/ffprobe/Chrome/LLK/models 结构化结果 |
| GET | `/api/config` | 当前 Workspace、vtrim.toml/global/llm 的摘要与 raw |
| PUT | `/api/config/vtrim` | 保存 vtrim.toml（支持整篇 raw 或结构化 patch） |
| PUT | `/api/config/llm` | 保存 `.vtrim/llm.toml` |
| POST | `/api/models/download` | 下载全部 ASR 模型，返回 Job |

### 8.2 工程

| Method | Path | 说明 |
|---|---|---|
| POST | `/api/project/init` | body: `{ input, danmaku?, output_dir?, config_path? }` -> snapshot |
| POST | `/api/project/switch` | body: `{ config_path }`，切换 Workspace |
| GET | `/api/project/status` | 输入/输出/current_stage/各产物存在性/ASR metadata |
| GET | `/api/project/files` | 核心产物文件清单 |

### 8.3 数据可视化与编辑

| Method | Path | 说明 |
|---|---|---|
| GET | `/api/data/transcript` | transcript.json 或解析 SRT 后的 segments |
| GET | `/api/data/heatmap` | 弹幕密度窗口（用于时间线热度） |
| GET | `/api/data/events` | events.json（含 enabled 字段） |
| PUT | `/api/data/events` | 保存编辑后的事件（数组/EventStore 均可） |
| GET | `/api/data/clips` | `output/clips/*/clip.json` + 文件路径列表 |
| GET | `/api/file?path=...` | 受限文件服务，支持视频 range；仅允许工程输入/输出/clip 路径 |
| POST | `/api/fs/list` | body `{ dir }`，返回子目录/文件；仅允许常见媒体扩展名提示，服务端做目录白名单 |

### 8.4 核心流程任务

| Method | Path | 说明 |
|---|---|---|
| POST | `/api/pipeline/run` | body: `{ stages: ["extract","transcribe","detect","cut"], dry_run?, resume?, force? }`，启动 Job |
| POST | `/api/pipeline/stage` | body: `{ stage: "extract" \| "transcribe" \| "detect" \| "cut", ... }` |
| GET | `/api/jobs` | Job 列表 |
| GET | `/api/jobs/{id}` | Job 详情 |
| GET | `/api/jobs/{id}/events` | SSE 事件流 |
| POST | `/api/jobs/{id}/decision` | 对 `awaiting_resume` 作出继续/重来 |
| POST | `/api/jobs/{id}/cancel` | 请求取消 |

`stages` 数组顺序固定为 `audio -> transcribe -> detect -> cut`；`force` 用于强制重跑指定 stage（忽略文件存在性，对应 process 的“无视文件检查”）。

### 8.5 Polish 流程

| Method | Path | 说明 |
|---|---|---|
| GET | `/api/polish/projects` | 扫描 `output/clips/*/clip.json`，标注是否已有 polish.toml/processed |
| POST | `/api/polish/init` | body `{ dir }` -> 调用 `commands::cmd_init`（自动 parse） |
| POST | `/api/polish/llm` | body `{ dir }` -> Job |
| POST | `/api/polish/process` | body `{ dir }` -> Job |
| POST | `/api/polish/render` | body `{ dir, renderer? }` -> Job（render 内部会先 process） |
| POST | `/api/polish/cover` | body `{ dir, template? }` -> Job |
| POST | `/api/polish/preview` | body `{ dir, mode? }` -> 启动独立预览端口，返回 `{ video_url?, cover_url? }`，不阻塞 |
| GET | `/api/polish/project` | query `dir` -> polish.toml 转 JSON（含 segments/events） |
| PUT | `/api/polish/project` | body `{ dir, segments? }` -> 写回 polish.toml（可复用现有 `save_config_handler` 逻辑） |

## 9. WebUI 设计

单页原生应用，左侧导航，右侧内容区；暗色本地工具风格。

### 9.1 页面/视图

| 视图 | 内容 |
|---|---|
| 总览 | 当前工程、current_stage、4 个 Stage 状态卡片、关键产物文件、开始全流程按钮 |
| 核心管线 | Stepper 1-4 + “LLM 订正”可选步骤；每个 Stage 可单独运行；运行中显示进度条与实时日志；断点续传弹窗 |
| 事件时间线 | 时间轴展示：字幕段、弹幕热力、事件块；点击事件查看详情；支持启用/停用事件、编辑 label/时间并保存 |
| 素材/预览 | `<video>` 播放输入视频或某个 clip；服务端 range 支持拖动进度 |
| 精修 | 左侧 clip 列表；选中后执行 init/llm/process/render/cover；`preview` 以 iframe 嵌入现有 polish 页面；可查看/编辑 polish.toml segments |
| 设置 | vtrim.toml / .vtrim/llm.toml 编辑（文本 + 常用字段表单）；系统 Doctor；下载模型 |

### 9.2 时间线实现

- 不引入前端框架/图表库。
- 使用普通 DOM + CSS absolute 定位绘制事件块，或 `<canvas>` 绘制刻度/热度。
- 缩放：支持“全片 / 1 分钟 / 10 分钟”按钮。
- 事件块可拖拽修改起止时间（v1 先做文本编辑，拖拽放 v2）。

### 9.3 与现有 polish 预览的关系

- 现有 `commands::cmd_preview()` 会启动随机端口服务器并 `loop` 阻塞等待。
- 重构为：
  - `launch_preview(project_path, mode) -> PreviewUrls`（只启动服务器）
  - CLI `cmd_preview()` = `launch_preview()` + 打开浏览器 + 等待 Ctrl+C（保持现状）
  - Server API = `launch_preview()`，把 URL 返回前端嵌入 iframe。

## 10. 实施步骤

### M1：基础重构
- Workspace 模型 + 路径参数化 config 保存。
- 将 main.rs 中核心流程逻辑抽到 `service/core_pipeline.rs`。
- CLI 回归测试：现有 `vtrim process/status/detect/cut` 行为不变。

### M2：Server 骨架
- `vtrim server` 子命令 + axum 启动 + 静态 index.html。
- `/api/health`、`/api/doctor`、`/api/project/status`。
- 前端总览页可展示当前工程与 Stage 状态。

### M3：任务执行与进度
- JobManager、WebProgressReporter、SSE。
- `/api/pipeline/run|stage`、断点决策 API。
- 前端核心管线 Stepper + 日志 + 断点弹窗。

### M4：数据可视化与编辑
- transcript/heatmap/events API。
- 时间线 UI + 事件编辑/保存。
- 媒体 range 服务与 video 预览。

### M5：Polish 集成
- polish API + 精修视图 + preview iframe。
- polish.toml segments 可视化/编辑。

### M6：收尾
- README/AGENTS.md 更新。
- 单元测试与手动验收。
- 可选：取消增强（tokio::process）。

## 11. 测试与验收

### 单元测试
- `Workspace::resolve` 相对/绝对路径、config 路径切换。
- `ResumePolicy` 对 checkpoint 的删除/保留。
- `JobManager` 状态机：running -> succeeded/failed/cancelled/awaiting_input。
- `core_pipeline` 的 stage 选择与 dry_run 参数映射。
- 事件时间线 API 的 events.json 读改写（enabled 过滤与 `load_events` 一致）。

### 手动验收
1. 在含 `vtrim.toml` 的工程目录运行 `vtrim server --open`，浏览器看到总览。
2. 从 WebUI 执行 `extract -> transcribe -> detect -> cut`，全程能看到 stage/progress/日志。
3. 中途手动放置 `events.checkpoint.json`，WebUI 出现续传/重来弹窗，选择后可继续。
4. 事件时间线显示字幕与弹幕热力，能停用一个事件并让 `cut` 跳过。
5. 选择一个 clip 目录执行 polish init/process/render/preview，页面能嵌入预览。
6. `vtrim process` 等 CLI 原命令仍可正常使用。

## 12. 开放问题 / 后续增强

- 多个长任务并发：当前单 Job 串行；后续可升级为基于 span/task-local 的日志隔离。
- 取消精确度：迁移 ffmpeg 到 `tokio::process` 后可实现强取消。
- 文件浏览器安全模型：先仅限工程根与输出目录，后续可加“信任目录”配置。
- 配置结构化编辑：第一版以 raw TOML + 常用字段表单为主，后续可做完整 JSON Schema 表单。
- 多工程并行窗口：`vtrim server` 支持一个 Workspace，多工程可通过切换实现；若需要同屏多工程，可让用户启动多个 server 实例。
