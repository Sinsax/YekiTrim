# yekitrim-core/src/cutting — 切片（GPU 编码 + 片段目录/索引 + 静音检测）

> 范围：`crates/yekitrim-core/src/cutting`/** ｜ 上游：`pipeline::PipelineRunner`（Stage 4）、`yekitrim-service::core_pipeline`、`yekitrim-polish`（读 `clip.json` / 复用目录）｜ 验证：`cargo test -p yekitrim-core --lib cutting`

## 职责

按 `events.json` 逐条裁出成片：ffmpeg 编码（GPU 自动降级）、目录与索引维护、字幕/弹幕裁剪、
`publish_config.json` 与 `clips_order.txt`。`clip_index.rs` 管 `clips/index.json` 与 `clip.json`，
`silence.rs` 提供静音区间检测（「去静音」用）。

## 入口

- `cutting::Cutter`（Stage 4）。
- `cutting::clip_dir_name_for_event(event)`：事件的目标目录名（`clip_dir` → 旧契约 `clip-{rank:02}`）。
  **pub** 是给上层的「剪切收尾」用的（`yekitrim_service::init_polish_for_events` 按同一条规则找刚剪出来的
  目录补精修工程）—— 服务层别再抄一份命名规则。
- `cutting::clip_index`：`ClipIndex` / `ClipEntry`（`new` / `from_event`）、`load_index` / `save_index` /
  `reconcile_index` / `refresh_index` / `upsert_clip` / `remove_clip`、`index_path`、
  `clip_meta_matches`（标题相同或原片区间 IoU ≥ 0.5）、`make_clip_dir_name` / `assign_clip_dirs` /
  `allocate_one_clip_dir`、`is_safe_component`、`ts_secs`。
- `cutting::silence`：`detect_silence_ranges`、`silence_gaps_from_segments`、`detect_silence`、
  `detect_silence_from_srt`。
- 产物：`output/clips/<clip_dir>/clip.mp4` + `clip.srt` + `clip.dm.srt`（有弹幕时）+ `clip.json`；
  汇总 `output/clips/index.json`、`output/clips/clips_order.txt`、`output/publish_config.json`。

## 边界

- GPU 加速编码 (NVENC/QSV/AMF **两级探测** + 降级 libx264)
  ⚠️ **探测必须"真编一次"，不能只查 `ffmpeg -encoders`**（2026-10-05 修）：
  列表里有 `h264_nvenc` 只说明**这份 ffmpeg 带它编译**，不说明**本机有硬件**。
  只查列表 ⇒ 无卡机器（装了全功能 ffmpeg 的 **Windows 同样**）选中 nvenc ⇒
  每条剪切 `Conversion failed!` ⇒ `所有剪切任务均失败`。
  `encoder_config()` 现在对每个候选**真编 3 帧 testsrc 到 `-f null`** 验证后才采用；
  `libx264` **不参与探测**、无条件兜底（探测失败连兜底都没有 = Stage 4 整个不可用）。
  另：硬件编码器**选中时可用、跑到一半失效**（驱动崩溃/显存耗尽）由
  `cut_segment` 的 `libx264` 重试兜住 —— 这才是"自动降级"的完整含义。
  守门测试 `encoder_config_never_returns_an_unusable_hardware_encoder`（变异验证过）。
- `ffmpeg -ss {start} -i {input} -t {duration} -c:v {auto_encoder} -preset {auto_preset} {qparam} 23 -pix_fmt yuv420p -c:a aac -movflags +faststart {output}`
- ⚠️ **字幕/弹幕取用一律按"重叠"，不许用"完全包含"**（2026-10-05 修）：
  `generate_clip_subtitle` 原判据 `s.start >= start && s.end <= end` 会因
  **转写全精度 f64 vs 事件毫秒精度**的误差把边界字幕判掉（实测差 5.7e-8），
  然后 `filtered.is_empty()` **静默返回 Ok** ⇒ 整条片段没有 `clip.srt`、无任何日志。
  现为 `s.start < end + 1ms && s.end > start - 1ms`，与弹幕的
  `e.start < end && e.end > start` 语义一致。回归钉子 5 条（含变异验证）。
- **多段片段**（`Event.ranges`，分散事件合并的产物；空 = 单段 = 老行为）：
  `cut_event` 逐段剪到 `<clip_dir>/.parts/part-NN.ts`（**一次选定编码器** —— 逐段各自探测会拼出
  两种编码的流）→ concat demuxer `-c copy`；失败回退 `-filter_complex concat` 重编码，再失败换 libx264；
  `.parts/` 成功失败都清掉。`clip.srt`/`clip.dm.srt` 按 `range_offsets` **逐段平移**
  （第 2 段接在第 1 段之后；段间空隙的内容**不进成片**）；`clip.json` 多段时写 `ranges`、
  `duration` 用**内容时长**（成片文件就是那个长度，跨度不算）。
  判据 = `cutting::multi_range_tests`（4 条单测）+ 真剪一次
  `VT_CUT_SRC=<mp4> cargo test -p yekitrim-core --lib diagnose_multi_range_cut -- --ignored --nocapture`
  （验时长 ≈ 各段之和 + 音视频轨都在 + `.parts/` 清干净）。
- 命名: `output/clips/<clip_dir>/clip.mp4`。`clip_dir` 来自 `events.json` 的 `clip_dir`：
  检测时先按标题/区间匹配旧片段（复用旧目录、保住精修产物），匹配不上再用标题 slug 生成
  唯一目录名；旧 events.json 没有 `clip_dir` 时回退 `clip-{rank:02}`。目录名不再等于 rank，
  `rank` 只表示热度名次。
- `output/clips/index.json` = clips 目录索引（目录 → title/rank/popularity/时间），可从磁盘
  `clip.json` 重建；Cutter/检测/删除后刷新，`data_clips_impl` 排序优先看 `clip.json` 的 rank。
- `clip.json` = 剪出时冻结的元数据（`title` / **`summary`** / `event_type` / `popularity` / `rank` /
  `original_start` / `original_end` / `duration`）—— 切片页拿它当"这条片段自己的身份"，
  重新检测换掉 events.json 后照样能显示（见 WebUI 一节「切片与事件的对应关系」）
- **不覆盖别人的片段**：目标目录已有 `clip.json` 且与本次事件标题不同、区间 IoU < 0.5 →
  该条剪切报错跳过（`foreign_clip_conflict`，含单测），不让目录名变化撞掉已剪好的成片
- ⚠️ **转写文件的解析顺序必须与 service 的 `Workspace::transcript_path` 一致**（`load_transcript`）：
  `ctx.transcript`（内存里已填的）→ `transcript_file`（相对**工程根**）→ `output/transcript.json` →
  `output/video.srt`。只认最后一项会让「ASR 产物在 `output/transcript.json`、工程没有 transcript_file、
  也没有 video.srt」的工程剪出**没有 `clip.srt`** 的片段（真实 cut-all 实测：clip-01 只有 clip.dm.srt）；
  相对基准错了则会静默退回旧稿。判据：`cut_falls_back_to_output_transcript_json_and_emits_clip_srt`
  与 `external_json_transcript_file_still_wins_over_output_fallback`。
- 字幕: 从原始 SRT 裁剪时间范围 + 偏移 → `<clip_dir>/clip.srt`（弹幕 → `<clip_dir>/clip.dm.srt`，元数据 → `clip.json`）
- 输出: `output/publish_config.json` + `output/clips/clips_order.txt` (按弹幕热度降序)
  ⚠️ **增量剪切必须合并旧索引**（`PublishConfig::merge_from_disk`），不能只写本批：单条重切/一键全切
  是**逐事件**跑 Cutter 的，只写本批会把其它片段从发布索引与播放顺序里洗掉（2026-10-30 实测：全切后
  `publish_config.json` 只剩最后一条）。判据 `publish_index_merges_incremental_cuts_and_drops_missing_files`。
- **弹幕取用统一走 `danmaku::load_danmaku_auto`**（`cutting/mod.rs` 已改）：以前单独用 `parse_ass`，
  录播姬 XML 弹幕会只有热力图、没有 `clip.dm.srt`。
- **`silence.rs::detect_silence_ranges` 先按时间排序 + 合并重叠区间**再取相邻间隔
  （`silence_gaps_from_segments`，含单测）—— **乱序/重叠的字幕按文件顺序硬算会造出几十秒假静音**。
  净空判定/映射成屏蔽轨道的部分在 `yekitrim-polish/src/project/silence.rs`（`SILENCE_PAD_SECS` /
  `DEFAULT_MIN_GAP_SECS = 3.5s`），别在 core 这边重复实现。
- 目录/文件名拼接一律过 `clip_index::is_safe_component`（`/api/media` 与删除片段那条线也认它）。

> 改动只允许落在本目录；跨模块改动先读对方卡，并跑两条门禁。

## 验证

```bash
cargo test -p yekitrim-core --lib cutting
```

`mod::tests`（含 `foreign_clip_conflict_*` 三条）、`clip_index::tests`（目录命名/复用/索引重建）、
`silence::tests`（排序+合并后的间隔）都要绿；动了 `clip.json` 字段或目录命名规则必须全绿。
