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

- GPU 加速编码 (NVENC/QSV/AMF 自动检测, 降级 libx264)
- `ffmpeg -ss {start} -i {input} -t {duration} -c:v {auto_encoder} -preset {auto_preset} {qparam} 23 -pix_fmt yuv420p -c:a aac -movflags +faststart {output}`
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
- 字幕: 从原始 SRT 裁剪时间范围 + 偏移 → `<clip_dir>/clip.srt`（弹幕 → `<clip_dir>/clip.dm.srt`，元数据 → `clip.json`）
- 输出: `output/publish_config.json` + `output/clips/clips_order.txt` (按弹幕热度降序)
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
