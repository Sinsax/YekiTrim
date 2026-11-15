# 任务进度 v2 —— 全任务审计 + 三层修法（2026-11-15）

> 触发：**「查找事件 · 运行中 · 阶段 1/1 · 已 3 分钟没有新进展 · 可能卡住」** —— 检测正在正常跑。
> 本轮先做**全任务审计**（不只补 detection），再定修法。

## 一、审计口径

误报的充要条件：任务 `running` 且 `updated_secs` 静默 > `IDLE_WARN_SECS`(120s)。
而 `updated_secs` 由 `Job::push`（progress / log / message / stage_* 全都会 push）刷新 ——
所以**只有"全程一个事件都不发"的长任务才会误报**。下面按这个口径逐类核对。

## 二、全任务覆盖表（逐个核对代码，2026-11-15）

| 任务 kind | 长耗时来源 | 事件面 | 结论 |
|---|---|---|---|
| `stage_extract` 提取音频 | ffmpeg 转码 | 秒级 `stage_progress`（P1） | ✅ |
| `stage_transcribe` 识别声音 | ASR 分钟级 | 批次/分段 `stage_progress` | ✅ |
| **`stage_detect` 查找事件** | **LLM 多窗口 3~10 分钟** | **零事件**（仅取消探针） | ❌ **本次误报** |
| `stage_cut` 剪出片段 | ffmpeg 逐条 | 批次 `stage_progress` + 逐条 `note` | ✅ |
| `stage_process` / `pipeline` 一键处理 | 四阶段串联 | 随各阶段；收尾 i/N（P2） | ✅（含 detect 漏） |
| `ingest_*` 采集后自动处理 | 同管线 | 走 `run_stages`，同阶段进度 | ✅（含 detect 漏） |
| `cut_all` / `cut_one` | 逐事件 ffmpeg | `jobs.progress`（project_data） | ✅ |
| `correct` 字幕订正 | LLM 窗口 | 窗口 `stage_progress`（P2） | ✅ |
| `polish_llm` / `llm_segment` | LLM 窗口 | 窗口 `stage_progress`（P2） | ✅ |
| `polish_process` 出片后处理 | 素材下载 | 里程碑 `note`（P3） | ✅ |
| `polish_render` 导出成片 | 底座/ffmpeg | 分块百分比 | ✅ |
| `polish_cover` 生成封面 | 子进程 | `run_polish_child` 跟随日志 | ✅ |
| **`polish_init` 出片准备** | **预览代理 720p 转码（长视频分钟级）** | **零事件**（`cmd_init` 只写 tracing） | ❌ **第二个漏洞** |
| `silence` 去静音 | 同步接口，非任务 | — | — |
| `download_models` | 网络 | `JobDownloadProgress` 字节进度 | ✅ |
| `download_bilibili` | 网络 | `jobs.progress` 百分比 | ✅ |
| `publish` 投稿 | 逐条上传 | `jobs.progress(i+1,total)` | ✅ |
| `stickers` 表情包构建 | LLM 批次 | 共用 `progress_bridge` | ✅ |

**两个漏洞**：`stage_detect`（分钟级、最高频）与 `polish_init`（分钟级、新片段必跑）。
**一个系统性缺口**：没有任何心跳兜底 —— 今天补了这两处，明天新增第三个仍然会误报。

## 三、修法（三层，缺一不可）

### L1 · 两个漏网任务接真进度
1. **查找事件**：`detection/llm.rs::call_with_progress` 增加 `progress: Option<&Arc<dyn ProgressReporter>>`；
   现有转发器**已经在收 `AgentProgress{completed_windows,total_windows}`**，只是只转发了 `output` ——
   顺手折成 `stage_progress(完成窗, 总窗)`（数据在手边，不需要新通道）。`EventDetector::run` 传入
   `ctx.progress`（Main 与第二遍 pass 两处调用）。
   效果：`阶段 1/1 · 查找事件 · 窗口 3/7 · 43%`。
2. **出片准备 `polish_init`**：`cmd_init` 在 `ensure_preview_proxy`（`commands.rs:109`）前后发
   `note("生成预览代理…")`；server 的 `init` 分支补一条开场 `jobs.log`。
   ⚠️ 不改 `cmd_init` 的签名去穿 reporter（会波及 CLI 与既有调用点）—— 两条 log + L2 心跳足够不误报。

### L2 · 静默心跳兜底（结构性，覆盖所有任务含未来新增）
`JobManager::tick()`（已有 5s 定时器）对每个 `running` 任务：
- 静默 ≥ `HEARTBEAT_SECS`(30s) ⇒ push 心跳，刷新 `updated_secs`；
  文案 = `运行中 · 已 Xs`（有阶段/进度则带上：`阶段 1/1 · 查找事件 · 窗口 3/7`）。
- **心跳不能掩盖真卡**：同处记录 `(stage, current, total)` 的**上次变化时刻**；三者连续
  `STALL_SECS`(300s) 不变 ⇒ 心跳文案改 `⚠ 进度已 5 分钟未变（窗口 3/7）—— 可能卡在外部进程`，
  并置 `JobSummary.stalled = true`。
- **`total == 0` 不判卡**（该阶段无可量化进度，没有可比对的值就不该质疑它），只报"运行中"。
- 单测（含变异自证）：静默 31s ⇒ 心跳刷新；stage/current/total 305s 不变 ⇒ stalled；
  `total=0` 静默 10 分钟 ⇒ **不**置 stalled（去掉该分支即红）。

### L3 · 前端判据跟着换
`jobLine` / `stuckNote`：卡住提示改读后端 `stalled`；普通静默只显示弱提示「运行中 · 已 Xs」。
任务抽屉 / 识别页流程卡 / 侧栏徽标共用（`jobLine` 唯一实现不变）。

## 四、验收

1. 单测：detection 窗口折算 3 条 + 心跳/stalled 4 条（变异自证）。
2. 真机：长视频跑「查找事件」全程无「可能卡住」；进度条按窗口推进。
   新片段「出片准备」不再静默。
3. `cargo check --workspace --all-targets` 0 warnings；npm build + 全部判据脚本 + 两条仓库门禁全绿。

## 五、分批

- **批次 1**：L1（detection 进度 + polish_init 两条 log）+ 单测。
- **批次 2**：L2（心跳 + stalled + Summary 字段）+ 单测（变异自证）。
- **批次 3**：L3（前端判据）+ 构建 + 部署验证。
