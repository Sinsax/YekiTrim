# yekitrim-core/src/pipeline — Stage trait 与管线编排器

> 范围：`crates/yekitrim-core/src/pipeline`/** ｜ 上游：`yekitrim-service::core_pipeline`（CLI 与 server 共用）、四个 Stage 实现 ｜ 验证：`cargo test -p yekitrim-core --lib pipeline`

## 职责

管线骨架：`Stage` trait（所有阶段必须用它实现）+ `PipelineRunner`（顺序驱动、按文件存在性
决定"从哪开始/跑不跑"、把进度经 `ProgressReporter` 报出去）。这里不含任何阶段逻辑 ——
四个 Stage 的实现分别在 `audio/`、`asr/`、`detection/`、`cutting/`。

## 入口

- `pipeline::Stage`（`#[async_trait]`）：`name() -> &'static str` / `stage_number() -> u32` /
  `async run(&self, ctx: &mut PipelineContext) -> anyhow::Result<()>`。
- 常量 `STAGE_AUDIO = 1` / `STAGE_TRANSCRIBE = 2` / `STAGE_DETECT = 3` / `STAGE_CUT = 4`。
- `pipeline::PipelineRunner`（`runner.rs`）：
  - `PipelineRunner::new(config: Config)`、`add_stage(Box<dyn Stage>)`、`config()`、
    `set_progress(Arc<dyn ProgressReporter>)`。
  - `async run(&self, ctx)` / `async run_all(&self, ctx)` / `async run_until(&self, ctx, max_stage)`。
- `inject_config` / `resolve_danmaku`（模块内私有，`runner.rs`）：把 `Config` 与解析后的
  弹幕绝对路径注入 `PipelineContext`。

## 边界

**Stage 编号**: AudioExtractor(1) → Transcriber(2) → EventDetector(3) → Cutter(4)，连续无跳过。

所有 stage 用 `#[async_trait]` 实现 `Stage` trait (`crates/yekitrim-core/src/pipeline/stage.rs`)。

`PipelineRunner` 提供三种运行模式 (`runner.rs`):
- `run()` — 受文件存在性约束（而非 `current_stage`），单 stage 命令用
- `run_all()` — 无视文件检查从头跑到底，`process` 命令用
- `run_until(max)` — 跑到指定 stage 为止，`process --dry-run` 用

`current_stage` 不再持久化到 `yekitrim.toml`，改为从输出文件存在性自动推导：

| 文件存在 | 推导阶段 |
|---------|---------|
| `output/video.wav` 或 `audio_file` 非空 | Stage 1 |
| `output/transcript.json` 或 `video.srt` 或 `transcript_file` 非空 | Stage 2 |
| `output/events.json` 或 `events_file` 非空 | Stage 3 |
| `output/clips/clips_order.txt` 存在 | Stage 4 |

**状态机**（`config/types.rs::current_stage()` 通过文件存在性推导）:

| 规则 | 示例 |
|------|------|
| `stage > current + 1` → 拒绝 | `detect`(3) 无转写文件 → ❌ |
| `stage <= current` → 可重跑 | `transcribe`(2) 存在转写文件 → ✅ |
| 完成后 stage 自动推进 | `transcribe`(2) 成功后文件已存在 |
| `process` 无视文件检查, 全部重跑 | |
| stage 无跳过 | 缺失前置文件则拒绝 |

- **编号连续是硬要求**：`Stage::stage_number()` 与 `STAGE_*` 一一对应，跳号会让 `run_until`
  与状态机推导一起错位；新增阶段要同时改 `stage.rs` 常量、`current_stage()` 表与上表。
- **弹幕解析必须在 `resolve_danmaku` 里做**：`danmaku_file` 是相对工程目录存的
  （`yekitrim init` 用 `path_to_rel` 写），读取侧按**工程根（`output_dir` 的父目录）→ 视频父目录**
  的顺序拼绝对路径，两个基准都试一遍。
  ⚠️ 少了这一步，相对路径会被当成"相对进程 CWD"：弹幕永远找不到，热度静默退化成纯字幕密度、
  切片也不会生成 `clip.dm.srt`。反过来，**别在 stage 里再各写一份路径拼接**。
- **阶段预检（2026-11-15）**：每个 stage 开工前 runner 都跑 `disk::ensure_free` ——
  `output_dir` 所在卷的可用空间 ≥ `[cutting] min_free_gb`（`0` = 关）才派发；不达标
  **整个阶段在此停下**（`disk::FreeSpaceError`，可 downcast，一键全切的收尾据此把
  「没让剪」与「剪切失败」分开并停整批）。起因：全切写穿数据盘 → 外部磁盘看门狗删源。
  判据 `runner_aborts_stage_before_it_runs_when_disk_precheck_fails`（stage 一次都不执行）。
- **`asr/artifacts::reset` 必须先清目录**：`raw/` 按块编号命名，换过 `chunk_duration` 后残留会冒充
  本次结果 —— 管线在开跑 ASR 前调它，别绕过（细节见 `src/asr/AGENTS.md`）。
- 单 stage 命令与 `process` 走的是**同一个 runner**（`yekitrim-service::core_pipeline` 装配），
  所以"某个命令自己绕过文件检查"这类改动会同时影响 CLI 与 WebUI 两条入口。

> 改动只允许落在本目录；跨模块改动先读对方卡，并跑两条门禁。

## 验证

```bash
cargo test -p yekitrim-core --lib pipeline
```

`pipeline::tests` 钉住 runner 的三种模式与文件存在性判据；动了 `run/run_all/run_until`
或 stage 编号就必须看到它绿。