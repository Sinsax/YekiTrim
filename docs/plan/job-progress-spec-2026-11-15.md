# 任务进度统一口径 —— 范本与范围（2026-11-15）

> 起因：用户实测「一键处理」任务卡在「运行中 / 准备中」，2 分钟后弹「已 2 分钟没有新进展 ——
> 可能卡在外部进程或网络盘上」。本文先给**范本**（显示与事件的统一契约）与**范围**（全部任务
> 种类的现状与目标），确认后再按 P1 动手。

## 0. 现状病根（三条，都有代码定位）

1. **阶段事件被同步阻塞饿死**（「准备中」悬案的直接原因）
   `api/core.rs::WebProgress` 把落库动作 `Handle::spawn` 到**当前** runtime；而管线体
   （`spawn_job` 的专用线程 + current-thread runtime）里 `audio/extract.rs` 用的是
   **同步** `ffmpeg .output()` —— 提取音频的整段几分钟里 runtime 不让出，
   spawn 出去的更新任务永远排不上队。于是：
   message 停在创建时的「准备中」、`updated_at` 冻结 → 120s 后卡住提示**误报**。
2. **文案三套口径**：创建「准备中」、阶段开始「{英文类名} 运行中」（`AudioExtractor 运行中`）、
   前端 `KIND_ZH` 又一套中文 —— 同一件事三种说法。
3. **UI 不显示阶段**：`JobSummary.stage` / `stage_number` 字段一直在，任务行从不渲染它；
   复合任务（一键处理 = 4 阶段）看不出"走到第几步"。

## 1. 范本 —— 一行任务的统一骨架（JobList / 任务抽屉 / 侧栏徽标共用同一份数据）

```
┌──────────────────────────────────────────────┐
│ 一键处理                          [运行中]     │   ← kindText(job.kind) + statusText
│ 阶段 1/4 · 提取音频 · 已处理 00:37 / 03:12     │   ← 进行时行（有阶段才显示）
│ ▓▓▓░░░░░░░░░░░░░░░░░░░░ 19%                  │   ← total>0 时才显示
│ （>120s 无事件才出现）已 2 分钟没有新进展…      │   ← 兜底（节奏修好后只在真卡住时响）
└──────────────────────────────────────────────┘
```

**进行时行由前端一个纯函数拼（`api/jobs.js::jobLine(job)`，唯一实现，别抄三份）：**

- 有阶段 → `阶段 {i}/{N} · {stage_zh}`（无 stage_total 只显示 `· {stage_zh}`）；
  `i` = `stage_index`，`N` = `stage_total`，`stage_zh` 用**服务端映射好的中文名**（前端不再自己映射）；
- `message` 非空且 ≠ 阶段名 → 追加 ` · {message}`（message 语义收紧 = **阶段内一句话明细**）；
- `total > 0` → 追加 ` · {current*100/total}%`，进度条同口径。

**事件契约（服务端，全部任务的硬规矩）：**

| 规矩 | 内容 |
|---|---|
| 阶段开始 | 落库 stage 事件（`stage / stage_index / stage_total / stage_zh`），≤1s 生效 |
| 阶段内节奏 | **≥每 5s 或每变化 ≥1%** 至少一个事件（progress 或 message；节流在任务体/桥侧做，models 下载已有先例） |
| 无数值进度阶段 | **≥每 30s 一条 message 心跳**（如「等待模型响应…」「窗口 3/7 进行中」） |
| 百分比语义 | `current/total` = **当前阶段内**的进度，**不跨阶段折算**（折算权重会撒谎；复合感由 i/N 表达） |
| 中文映射唯一 | core 只发稳定英文键（`AudioExtractor`…），服务端 `jobs.rs::STAGE_ZH` 一张表映射 —— 门禁钉住"stage_num 认识的键必须有中文" |
| Summary 增量 | `stage_zh / stage_index / stage_total`（可选字段，向后兼容）；`stage` 语义收紧 = 稳定英文键 |

**结构修（P1 核心）：进度桥改走通道。**
`WebProgress` 不再 spawn 到"当前"runtime，而是把事件推 `mpsc` 通道，由**主 runtime**上的
一个落库任务消费 —— 管线线程阻塞多久都不影响事件生效。这一处改动所有任务受益，
也是「同步 IO 不留在 async 线程」纪律在任务事件面上的对偶（事件面同理不留在管线线程）。

## 2. 范围 —— 全部任务种类 × 现状 × 目标

| 任务（KIND_ZH） | 现状 | 目标 | 批次 |
|---|---|---|---|
| **一键处理**（stage_process） | 消息停在「准备中」；阶段事件被饿死（本次事故） | 复合显示 阶段 i/4 + 阶段内 % | **P1 ★** |
| **提取音频**（stage_extract） | **零进度**（ffmpeg 同步阻塞，连 stage_started 都落不了库） | 通道版桥 + ffmpeg `time=` → 秒进度（对 VideoInfo 时长） | **P1 ★** |
| 处理流程（pipeline） | 同 stage_* 组合 | 随 stage_* 一并修 | P1 |
| 识别声音（stage_transcribe） | 有批次/分段进度 ✓ | 阶段中文名 + 节奏核对 | P1 |
| 找亮点（stage_detect） | 有窗口进度 ✓（progress_tx） | 同上 | P1 |
| 剪出片段（stage_cut） | 有批次进度 ✓ | 同上 | P1 |
| 采集族（ingest_none/transcribe/detect/cut/pipeline，后台） | **无进度桥**（静默跑） | 接同一座通道版桥 | P2 |
| 剪出全部 / 剪出片段（cut_all / cut_one） | 批次进度 ✓ | 中文名核对 | P2 |
| 字幕订正（correct） | 仅文案；窗口进度被 `without_internal_progress` 关掉 | 窗口 i/N | P2 |
| 智能剪辑 / 重做本段（polish_llm / llm_segment） | call_with_progress 有窗口事件 | 窗口 i/N 核对 | P2 |
| 去静音（silence） | 无数值进度 | 阶段文案 + 心跳 | P3 |
| 出片后处理（polish_process） | 仅文案 | 步骤文案 + 心跳 | P3 |
| 投稿（publish） | 待核 | 上传/条目进度 | P3 |
| 表情包构建（stickers） | 走同一座 progress_bridge ✓ | 随桥升级 | P3 |
| 下载模型（download_models） | 字节进度 ✓（已节流） | 仅核对文案 | — |
| B站下载（download_bilibili） | 百分比 ✓ | 仅核对文案 | — |
| 导出成片（polish_render） | 分块 % ✓（慢块聚合） | 仅核对文案 | — |
| 出片准备 / 生成封面（polish_init / cover） | 短任务、仅文案 | 文案即可 | — |

## 3. 验收判据

1. 桥改造：管线体阻塞 ≥5s 期间，阶段事件落库时延 <1s（单测模拟阻塞 stage，事件时间戳早于阻塞结束）。
2. **正常路径下 >120s 无事件 = 真卡住**：提取音频按秒推进、LLM 阶段 30s 心跳 —— stuckNote 从
   "经常误报"变成"真卡才响"。
3. 一键处理全程显示「阶段 1/4 → 4/4 · 中文 · %」；英文类名零漏出
   （门禁：`STAGE_ZH` 必须覆盖 `stage_num` 认识的全部键）。
4. `jobLine()` 唯一实现（JobList / 抽屉 / 侧栏徽标共用）；前端布局类改动做几何断言。

## 4. 实现切分

- **P1（本次事故）**：① 桥走通道到主 runtime ② 提取音频秒级进度（ffmpeg `-progress` 或 stderr
  `time=` 解析，分母 = ffprobe 时长；注意保持 `process::cmd` 纪律与既有 stderr 落日志）③
  服务端 STAGE_ZH + Summary 增量字段 ④ 前端 `jobLine()` + 行渲染。
- **P2**：采集族接桥；cut_all/cut_one/correct/polish_llm 的文案与窗口口径核对。
- **P3**：silence/publish/polish_process/stickers 的明细与心跳。
