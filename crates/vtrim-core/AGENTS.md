# vtrim-core — 管线编排 + ASR / 事件检测 / 剪切

> 范围：`crates/vtrim-core`/** ｜ 上游：`vtrim-polish` / `vtrim-service` / `vtrim-server` / `vtrim-cli` 都依赖我 ｜ 验证：`cargo check -p vtrim-core`

## 职责

整条切片的领域内核：把「录播视频 + 弹幕文件」变成「按事件分割的片段 + 字幕 + 事件清单」。
四个 Stage 的实现在这里（AudioExtractor → Transcriber → EventDetector → Cutter），编排器
`PipelineRunner` 也在这里；`vtrim-service::core_pipeline` 是 CLI 与 `vtrim-server` 共用的装配层，
它按 `-i`/`-o`/`-t`/`-e` 覆盖参数后驱动本 crate。此外还兜住工具模块：`types` / `path` / `srt` /
`fonts` / `download` / `progress` / `filelock` / `tenant`（没有专卡的都看这里）。

`lib.rs` 的模块表（**就是全表，新增模块必须在这里登记**）：
`types` / `config` / `fonts` / `path` / `srt` / `pipeline` / `audio` / `asr` / `llm` / `download` /
`detection` / `danmaku` / `cutting` / `progress` / `filelock` / `tenant`。

## 入口

- `types::PipelineContext`（`ctx.input_path` / `output_dir` / `config` / `audio_path` /
  `video_info` / `danmaku_path`）、`types::Event`、`types::EventStore`、`types::Transcript`、
  `types::Timestamp`（`to_seconds` / `from_seconds` / `to_srt_format`）、`types::VideoInfo`。
- `pipeline::{Stage, STAGE_AUDIO, STAGE_TRANSCRIBE, STAGE_DETECT, STAGE_CUT}`（`pipeline/stage.rs`）。
- `pipeline::PipelineRunner`（`pipeline/runner.rs`）：`new(config)` / `add_stage(Box<dyn Stage>)` /
  `set_progress(Arc<dyn ProgressReporter>)` / `run()` / `run_all()` / `run_until(max_stage)`。
- 四个 Stage：`audio::AudioExtractor`（1）、`asr::Transcriber`（2）、`detection::EventDetector`（3）、
  `cutting::Cutter`（4）。
- `config::{load_config, load_config_at, load_global_config, load_llm_config, patch_config_file,
  write_project_config, save_transcript_path_in, normalize_project_paths_in}`；
  `config::types::Config` / `ProjectConfig::current_stage(&output_dir)`。
- `path::{vtrim_home, sticker_dir, font_dir, display}`；`fonts::{dir, ensure_dir, scan, scan_at,
  resolve_file, resolve_file_at, family_of, is_font_file_name, is_safe_id}`。
- `progress::ProgressReporter`（trait）+ `NoopProgress`；`filelock::{with_path_lock, lock_path,
  with_path_lock_sync}`；`tenant::{jail_join, jail_existing, ensure_within}`。
- `download::{model_names, model_install, download_all, download_vad_model, ensure_models}`。

## 边界

⚠️ **core 不许认识上层**（门禁：`python3 scripts/check_module_boundaries.py`，扫 `crates/vtrim-core/src`）：
源码里**不得出现** `vtrim_polish` / `vtrim_service` / `vtrim_server` / `vtrim_cli`——
分层是单向的 core → polish → service → server → cli，底层一旦反向引用，两层就粘回去。
需要上层能力时由调用方**注入**（sink / 回调 / 参数），不是在 core 里 `use` 上来。

⚠️ **`wgpu-render` feature 的使用点全在 `vtrim-server`**，core 不参与这条转发链。
⚠️ core 只 `emit` LLM 用量记录，**写库由 CLI/server 侧装的 sink 负责**（见 `src/llm/AGENTS.md`）。

- **Stage 1（`audio/`，没有专卡，规矩在这里）**：输入视频 → ffmpeg 16kHz mono WAV + ffprobe
  `VideoInfo`；命令是 `ffmpeg -y -vn -acodec pcm_s16le -ar 16000 -ac 1` → `output/video.wav`，
  并把 `video_info` / `audio_path` 写回 `PipelineContext`。改名或改路径会连带炸
  `status.rs` / `current_stage()` 的推导（见下表）。
- **Stage 编号**：AudioExtractor(1) → Transcriber(2) → EventDetector(3) → Cutter(4)，连续无跳过。
- `current_stage` 不再持久化到 `vtrim.toml`，改为从输出文件存在性自动推导
  （`config/types.rs::current_stage`）：

  | 文件存在 | 推导阶段 |
  |---------|---------|
  | `output/video.wav` 或 `audio_file` 非空 | Stage 1 |
  | `output/transcript.json` 或 `video.srt` 或 `transcript_file` 非空 | Stage 2 |
  | `output/events.json` 或 `events_file` 非空 | Stage 3 |
  | `output/clips/clips_order.txt` 存在 | Stage 4 |

- **状态机**：`stage > current + 1` → 拒绝（`detect`(3) 无转写文件 → ❌）；`stage <= current` → 可重跑；
  完成后 stage 自动推进；`process` 无视文件检查、全部重跑；stage 无跳过（缺失前置文件则拒绝）。
- **弹幕路径解析在 `pipeline/runner.rs::resolve_danmaku`**（`inject_config` 调它）：相对路径按
  **工程根（`output_dir` 的父目录）→ 视频父目录**的顺序试，两个基准都试一遍，否则
  「视频在 `videos/`、弹幕在工程根」的布局会静默丢掉弹幕。
- **字体库**：`vtrim_core::fonts` + `path::font_dir()`，程序级固定在 `<程序目录>/resources/fonts/`
  （与 `.vtrim/`、`stickers/` 同级；`config init` 与 `server` 启动时都会 `ensure_dir`）。
  往里丢 `.ttf`/`.otf`/`.ttc` 就会出现在精修页两个下拉里。**id = 文件名去扩展名**，
  `family` 只是展示名（手写最小 `name` 表解析，按 `(路径, mtime, 体积)` 缓存，别每次请求重读几十 MB）；
  **空串 = 系统字体栈**（`LXGW WenKai → Noto Sans SC → Microsoft YaHei`），老工程就是这一档。
  ⚠️ **别改成"把字体拷进项目"**：出片页由渲染服务器（ServeDir 根 = 项目目录）端给 headless Chrome，
  所以库里字体必须有一条自己的路由 `/vt-fonts/:name`（在 `vtrim-polish`，见其卡）。
  回归钉子：`fonts::tests`（扫描/解析/越权）。发行包**不带任何字体**（授权原因，见 README「字体」）。
- `filelock` 的两种锁别混用：异步 `with_path_lock` / `lock_path` 给 handler，
  **同步** `with_path_lock_sync` 只给同步上下文的 read-modify-write（core 管线的 `edit_toml_at` 用）
  —— ⚠️ 只在**微秒级**的文件读改写里用，别拿它包 ffmpeg / 网络请求；它**非重入**。
- `tenant` 是多租户路径囚笼：接口只接受项目内**相对路径**，由服务端自己拼绝对路径
  （`jail_join` 写入 / `jail_existing` 读 / `ensure_within` 校验外部传入的完整路径）。
- 前置条件：`ffmpeg` + `ffprobe` 在 PATH 中（Stage 1 与 Cutter 都要）。

> 改动只允许落在本目录；跨模块改动先读对方卡，并跑两条门禁。

## 验证

```bash
cargo check -p vtrim-core
```

`cargo check --workspace --all-targets` 改任何 crate 后必跑（0 warnings）；
跨模块边界另跑 `python3 scripts/check_module_boundaries.py`。
