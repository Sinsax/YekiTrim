# process/ — cmd_process 后处理主流程（字幕 / 段处理 / 预算 / 音效 / 特效 / 贴纸落点）

> 范围：`crates/vtrim-polish/src/process`/** ｜ 上游：`vtrim-cli`（`polish process`）、`vtrim-server::api::polish`、`render::cmd_render`（渲染前隐式跑一次）、`llm/mod.rs` Step 4b（落盘前先裁一遍）｜ 验证：`cargo test -p vtrim-polish --lib process`

## 职责

`process` 是"LLM 写完事件之后、出片之前"的全部确定性后处理：无 LLM、幂等，跑一次是秒级。
`mod.rs` 的 `cmd_process` 按固定 Step 序执行 —— Step 0 **之前**先有一道**弹幕在屏窗口归一化**
（`normalize_danmaku_windows`：把 `data/danmaku.json` 的 `end` 抬到"至少够它滚完"，
`polish.toml` 已存在的工程没有别的入口能重跑 parse，老工程靠这一步自愈），
随后 Step 0 归一化事件时间、0a 运镜倍率迁移、1/1b/1c/1d
补默认运镜 + 预算抽稀 + 画中画三件事 + 手持摇摆守卫、2/2b1/2b/2c/2d 贴纸来源与落点与运镜锚点、
3/3b 音效规则引擎与补位、3c 片头 fade_in 补位、4 下载贴纸与 SFX。
`sfx.rs` = 音效规则引擎 + 解析/屏蔽；`fx.rs` = 缺 `end` 补默认 + 瞬时特效跨度 clamp + 画中画/手持/贴纸对齐守卫；
`download.rs` = 贴纸 keyword→文件解析 + 资产下载命令；`placement.rs` = 贴纸落点（纯几何，含单测）。

## 入口

- `process::cmd_process(project_path)`；`content_budget(subtitle_count, duration, seg_count) -> (sticker, sfx)`；
  `cap_events(segments, is_target, budget) -> usize`（均匀抽稀，返回删除数）。
- `sfx`：`match_sfx_rule(text) -> Option<&'static str>`（规则表按顺序匹配、首个命中生效）；
  `event_sfx_ref` / `event_has_sfx_ref` / `event_has_sound` / `find_uncovered_sfx_moments` /
  `subtitle_context(subtitles, t)`；`resolve_event_sfx(event, blacklist, used)` /
  `resolve_sfx_value` / `resolve_sfx_tag`（tag → 具体文件名，`used` 去重）；`load_sfx_blacklist(config)`
  （全局 `.vtrim/config.toml [sfx] blacklist` ∪ 项目 `polish.toml [sfx] blacklist`）。
- `fx`：`transient_duration_cap(event_type)` / `transient_default_duration(event_type)` /
  `normalize_event_ends(segments)`；`cap_pip_shots` / `align_pip_shots(segments, subtitles)` /
  `close_pip_shots` / `cap_handheld_sway` / `align_stickers_to_subtitles(segments, subtitles)`；
  常量 `MAX_PIP_SHOTS = 1` / `MIN_PIP_SECS = 1.5` / `MIN_RETURN_SECS = 0.5` /
  `STICKER_SNAP_MAX_SECS = 2.0` / `LEGACY_ZOOM_{MID,CLOSE,WIDE}`。
- `download`：`sanitize_sticker_name(keyword)`、`cmd_download(project_path)`、`download_keyword_assets(...)`、
  `ensure_assets(project_path)`（`api/polish.rs` 的预览/出片前都会调）；`try_resolve_sticker` 是 `pub(super)` 实现。
- `placement`（`mod placement;` **私有**，外部一律走 `cmd_process`）：
  `Frame`（`sticker_scale()` = `min(W,H)/1080`）、`Rect`、`Blocked`、`ObstacleKind`、`CameraShot`、
  `Pick`；`pick_sticker_position(blocked, allow, seed, f) -> Pick`、`sticker_box(x, y, f)`、
  `camera_states_for(...)`、`transition_secs(label, prev_label, cut)`、`camera_expand`、`parse_origin`、`parse_px`、
  `caption_band(f, font_size, stroke_px)`、`danmaku_band(f, font_size, stroke_px, ys)`、`main_frame_rect(...)`；
  常量 `FRAME_W|H = 1920|1080` / `STICKER_SIZE = 280.0` / `STICKER_MARGIN = 32.0` /
  `TRANSITION_SECS = 0.6` / `TRANSIENT_ZOOM = 1.10`。

## 边界

- ⚠️ **弹幕窗口归一化只在"真的变了"时写回**（`normalize_danmaku_windows`）：首页「切片总览」按片段目录里
  最新文件的 mtime 排序（`vtrim_service::project_data::last_touched_secs`），无条件重写
  `data/danmaku.json` 会把老片段一直顶到最前面。归一化本身是幂等的，**但比较必须带容差** ——
  `serde_json` 读 f64 会差 1 ulp（见 `vtrim_core::danmaku::WINDOW_EPS_SECS`），写严格相等就变成
  "每次 process 都重写一次"。回归：`clamp_display_windows_is_idempotent_across_json_roundtrip`。
- **Step 顺序是硬约束**：2b1（贴纸对齐字幕）**必须排在 2b（落点）之前** —— 落点是按贴纸的时间窗口
  算相机/字幕/其它贴纸障碍，时间挪了就得重算；2d（锚点解析）**必须排在 2b 与 2c 之后** ——
  `anchor="sticker"` 要读最终位置。1c 的配额守卫必须排在 Step 1 补默认运镜之后。
- **片头淡入是普通 overlay 事件，只在初次后处理补一次**（Step 3c：`mod.rs::ensure_intro_fade_in` +
  `INTRO_FADE_SECS = 0.5`，2026-09-22 起内建 `intro-black` 整条退役）：给**时间最早**且起点 < 0.5s 的段补一条
  `fade_in` 覆盖事件（`#000000` / `opacity 1` / `end = min(start+0.5, seg.end)`）；段跨度 < 0.2s、或**整片**
  已有与该窗口重叠的覆盖事件则**不补**（按时间挑段 —— 段之间时间会重叠，不能拿文件顺序当片头）。
  **判据是 `polish.toml` 的 `intro_fade_seeded`**：判定过片头（补了、或已有覆盖事件盖着）就置位落盘，
  往后时间轴上改/拖/删全归用户 —— 删掉再跑 process **不许**补回来（用户报过"删了又回来了"）；
  段表空 / 最早段起点 ≥ 0.5s / 段太短**不**置位（那时片头还不存在，下次仍可补）。逃生门：删掉
  polish.toml 那一行。⚠️ **别退回"看窗口有没有覆盖事件"** —— 那等于用户每删一次就再补一次。**不打 `manual`**。
  ⚠️ 必须排在 **Step 3 音效规则之后**：规则引擎会扫覆盖事件，排前面这条纯视觉黑场每跑一次都会多报一句
  「特效无匹配、移除音效」，字幕恰好命中规则时还会被挂上音效。
- ⚠️ **手工放的事件不受后处理改写**（用户报过「调整贴纸的结束时间没有成功」）：事件带
  `manual: bool`（`Sticker` / `Camera` 两个变体有，serde `skip_serializing_if = "is_false"`，老工程零迁移）
  时，`align_stickers_to_subtitles` / Step 2c 尾帧延长 / `align_pip_shots` 三处**全部跳过**
  （`Event::is_time_manual()`），**`cap_events` 的数量抽稀也跳过**。没有这个标记的话，用户改完 1 秒后
  自动后处理就把值吸回去了 —— 看上去就是"改了没生效"。
  回归：`cargo test -p vtrim-polish --lib cap_events_never_removes_manual_events`；
  真实工程：`VT_POLISH_DIR=<片段目录> cargo test -p vtrim-polish --lib diagnose_manual_events_survive_process -- --ignored --nocapture`（**会写回该工程的 polish.toml**，只在临时工程上用）。
- **落点规则**（`placement.rs`，Step 2b）：帧坐标固定 1920×1080，贴纸固定 280×280，`ev.x/ev.y` 是**盒子左上角**
  百分比（模板 DOM/canvas 与 `engine.js` 都按左上角消费，模板再钳制到 `[32,1608]×[32,768]`）。
  障碍 = ①人物 `layout.liver`、②弹幕区域 `layout.danmaku_area`（都在**源帧**坐标里框选，出片时视频整体被
  运镜缩放 → 先按该贴纸可见窗口的相机状态投到输出帧，zoom 取模板 Canvas 路径的 1.5/1.8 × 瞬时特效预留 1.10，
  窗口内的缩放过渡两端都算）、③字幕带（Canvas `cy=960` 与 DOM `bottom:80px` 两条路径取并集）、
  ④真实弹幕带（只有 `data/danmaku.json` 非空时才是障碍，按模板 `y=(d.y/1920)*540` 映射）、
  ⑤同刻已落位的其他贴纸（互不相压）。
  候选 = 16px 网格 + 各障碍边缘的「临界位」；先取 clearance 最高的一档（×0.9），在该档里按「盒心到帧心距离」
  排序取前 8 个，再用 seed（`fract(idx*3.7 + seg.start*1.3)`）伪随机挑一个 —— 即**安全区中心而不是边缘**，
  且逐条有变化。帧内实在没有安全位时退化为「重叠代价最小」并在日志里告警；最小余量 < 20px 时也会提示
  「选框贴得太近」。
- **`layout.sticker_area` 已降级**：`grid` 完全不参与落点（会打日志说明）；手写的自由选框只作**偏好**
  （里面有安全位就优先用，没有就自动放弃）。
  只读诊断：`VT_PROBE_ROOT=<media 目录> cargo test -p vtrim-polish --lib diagnose_sticker_placement -- --ignored --nocapture`
- **尺寸基准 = 画布短边**，不是宽：`Frame::sticker_scale()` = `min(W,H)/1080`（`placement.rs`），
  `sticker_size()/margin()/float_up()` 都乘它。**竖屏也是 280px**（短边 1080 → SK=1）——
  别改成 `W/1920`（那会按宽缩小、竖屏贴纸变小）。字幕/弹幕字号仍**不随画布缩放**，别顺手"统一"。
- **两个锚点基准禁止"统一"**：事件 `ev.x/ev.y`（auto/placement 产出）是**盒子左上角**百分比；
  `[sticker] position`（manual = 用户点选的固定落点）是**盒子中心**百分比。manual 的钳制顺序 =
  **先钳进安全区、再叠加上浮量**（`y_off`），emoji 回退 span 必须自带 `left/top`（否则居中到舞台）。
- **画中画配额：一个切片只有一条**（用户要求）：`cap_pip_shots` 按时间序只保留**最早**的一条「远」，
  其余降级成「中」（**只改 zoom**，anchor/origin 留着）。调用点两处成对：`cmd_process` Step 1c 与
  LLM 落盘前（`llm/mod.rs` Step 4b）。`align_pip_shots` 把起止吸到字幕边界、结果夹在所属分段内、
  不足 `MIN_PIP_SECS`(1.5s) 往后借下一条字幕；`close_pip_shots` 在画中画终点补一条**回镜头**事件。
  ⚠️ **前后镜头按时间算，不按文件顺序**：段之间时间上会重叠（LLM 按窗口生成 + 去静音切段），
  求值器也是先按时间排序再走的。
- **`cap_handheld_sway` 把非近景上的 `handheld` 剥掉**（只剥不补、幂等、日志报条数），排在画中画收工
  **之后** —— 收工补出来的回镜头承接了前一条的 `handheld`，一起吃这道守卫。
  **别再退回"靠 prompt/界面拦"**：LLM 会写错景别、用户也会改景别，守卫是唯一兜底。
- **预算**：`content_budget` 密度 = 每 5 条字幕/12s 一个贴纸、每 8 条字幕/15s 一个 sfx；
  `seg_count = Some(n)` 时上限 `max(1, n-1)`（至少留一个 segment 无特效），`None`（LLM prompt）时省略。
  `cap_events` 是均匀分布保留，**手工事件不参与抽稀**（额度只约束 LLM 生成量）。
- **运镜（camera）**：`process` 给每个 ≥1s 且没有运镜的段补 `zoom="中", anchor="liver"`（整段）——
  所以运镜是**全程开着**的，不是点缀。改默认档位/默认锚点就在这一处
  （`process/mod.rs` Step 1 与 `llm/mod.rs` Step 4 是两个重复实现，要一起改）。Step 2d 的
  `resolve_camera_anchors` 把 `anchor`/`anchor_to` 解析成 `origin`/`to_origin`（**每次运行都覆盖**），
  渲染侧只认这些结果坐标。
  - **三档倍率的当前值：中 1.3 / 近 1.5 / 画中画 0.55**（用户定：中 1.3、近 1.5）。
    **放大档位不存在「推出框露背景」的风险**：运镜是绕不动点做 `scale`，`s ≥ 1` 时画面四周
    一律向外扩张，任何倍率都盖满整帧，露底只发生在画中画（`zoom_wide < 1`，刻意留黑边
    `(1-factor)/2×1920 = 432px`）。所以再调大是安全的，别再拿「怕露出背景底色」当理由往下压。
  - **旧内建默认按值迁移**：`process/mod.rs::migrate_camera_zoom` 在 Step 0a 把
    中 `1.15 → 1.3`、近 `1.3 → 1.5`、远 `0.7 → 0.55`（**只认这几个旧默认值**，手改过的不动，
    日志逐条报）。三处兜底默认值（`default_zoom_mid/close/wide()`、模板 `camFactor`、
    `engine.js` `camFactor`）必须一起改。
  - **画中画的判据是 `|zoom-1| > 0.001`，不是 `zoom > 1.001`**（`templates/index.html` 与
    `PolishPreview.vue` 两处）。写成后者会把「远」档整档吞掉，画面完全不动。
  - **远景进出是硬切，不是缓动**（用户要求）：过渡的一端只要是「远」，「zoom」与不动点就在
    事件起点**一次性到位**。贴纸避让侧走 `placement::transition_secs()` → `CameraShot.transition = 0`
    —— 别写回常量 0.6，否则会按 0.6s 过渡凭空多算一个早已结束的旧状态当障碍。
  - **硬切 `cut`（切镜）= 同一条规则放开给任意景别**：`Event::Camera` 的 `cut: bool`
    （serde `skip_serializing_if = "is_false"`，老配置零迁移）。`cut = true` 时这一条**整档瞬间到位**。
    想「切过去再推」就写两条事件（切 + 平滑），别在求值器里搞两段式。
  - **位移用 zoom 的同一段窗口与曲线**（`min((end-time)*0.5, 0.6)`、`pow2Out`），不新增旋钮；
    不写 `anchor_to` 时两端相等，退化成「原地聚焦」。「主播→弹幕→回主播」= 两条事件，不是一条。
  - **老工程迁移**：`origin = "50% 50%"` 且无锚点的事件补成 `anchor="liver"`（有日志计条数）——
    这个值从来不是用户能设的，所以当成「没指定」。
  - 只读诊断：`VT_POLISH_DIR=<项目目录> cargo test -p vtrim-polish --lib diagnose_camera_anchors -- --ignored --nocapture`
    （`VT_CAM_JSON=1` 多打一行 `CAM_JSON={...}`：处理后运镜轨道，可喂给 `templates/index.html` 的
    `getActiveCamera` 与 `webui/src/polish/engine.js` 的 `cameraAt` 对拍）。
- **手工屏蔽区间内的字幕不参与后续预算/位置计算**：`cmd_process` 读 `data/subtitles.json` 后先按
  `config.mute_ranges` 过滤（`MuteRange::overlaps`），后续 position/budget 全基于清理后的字幕。
  ⚠️ **但屏蔽区间不再切段、也不再删事件**（用户要求：屏蔽只做标记）：段表与事件原样留着，
  出片合流后由 `Ffmpeg::cut_ranges` 剪掉、预览跳过。以前这里还会
  `split_segments_by_mute_ranges` 把屏蔽区内的事件从 `polish.toml` 删掉（而 process 会被预览/出片
  隐式触发）—— 已删除，**别再加回来**。回归：`cargo test -p vtrim-polish --lib mute_range_keeps_events`
  （`process::tests`，临时工程真跑 `cmd_process`，把删事件钉红）。
- `resolve_event_sfx` 未命中规则的事件按"宁缺毋滥"清空音效；`Step 3b` 对"字幕命中规则但 LLM 没放事件"
  的时刻在预算内补建 sfx 事件。屏蔽列表条目的语义见 `assets/sfx_catalog.rs::is_blacklisted`。
- 真实工程诊断：`VT_POLISH_DIR=<项目目录> cargo test -p vtrim-polish --lib diagnose_pip_and_silence -- --ignored --nocapture`
  （画中画配额/对齐/收工 + 去静音阈值 + 贴纸对齐；含 2.0/3.0/3.5/5.0s 阈值对照）。

> 改动只允许落在本目录；跨模块改动先读对方卡，并跑两条门禁。

## 验证

```bash
cargo test -p vtrim-polish --lib process
```

改 `placement.rs` 的几何、`fx.rs` 的守卫或 `cap_events` 抽稀后必跑；再补一次
`cargo check -p vtrim-polish`（0 warnings）。
