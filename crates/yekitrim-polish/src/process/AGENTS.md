# process/ — cmd_process 后处理主流程（字幕 / 段处理 / 预算 / 音效 / 特效 / 贴纸落点）

> 范围：`crates/yekitrim-polish/src/process`/** ｜ 上游：`yekitrim-cli`（`polish process`）、`yekitrim-server::api::polish`、`render::cmd_render`（渲染前隐式跑一次）、`llm/mod.rs` Step 4b（落盘前先裁一遍）｜ 验证：`cargo test -p yekitrim-polish --lib process`

## 职责

`process` 是"LLM 写完事件之后、出片之前"的全部确定性后处理：无 LLM、幂等，跑一次是秒级。
`mod.rs` 的 `cmd_process` 按固定 Step 序执行 —— Step 0 **之前**先有一道**弹幕在屏窗口归一化**
（`normalize_danmaku_windows`：把 `data/danmaku.json` 的 `end` 抬到"至少够它滚完"，
`polish.toml` 已存在的工程没有别的入口能重跑 parse，老工程靠这一步自愈），
随后 Step 0 归一化事件时间、0a 运镜倍率迁移、1/1b/1c/1d
补默认运镜 + 预算抽稀 + 画中画三件事 + 手持摇摆守卫、2/2b1/2b/2c/2d 贴纸来源与落点与运镜锚点、
3/3b 音效规则引擎与补位、3c 片头 fade_in 补位、4 下载贴纸与 SFX、7 预览代理保鲜。
`sfx.rs` = 音效规则引擎 + 解析/屏蔽；`fx.rs` = 缺 `end` 补默认 + 瞬时特效跨度 clamp + 画中画/手持/贴纸对齐守卫；
`download.rs` = 贴纸 keyword→文件解析 + 资产下载命令；`placement.rs` = 贴纸落点（纯几何，含单测）。

## 入口

- `process::cmd_process(project_path)`；`content_budget(subtitle_count, duration, seg_count) -> (sticker, sfx)`；
  `cap_events(segments, is_target, budget) -> usize`（均匀抽稀，返回删除数）。
- `audit`（规格 §4 B1）：`Finding { kind, at, detail }` / `audit(cfg) -> Vec<Finding>`（**六项检查**，纯函数）/ `write_report`。
  ⚠️ `data/process-report.json` **只在有 findings 时写**（R4：干净工程不许多一个文件）。
- `sfx`：`match_sfx_rule(text) -> Option<&'static str>`（规则表按顺序匹配、首个命中生效）；
  `event_sfx_ref` / `event_has_sfx_ref` / `event_has_sound` / `find_uncovered_sfx_moments` /
  `subtitle_context(subtitles, t)`；`resolve_event_sfx(event, blacklist, used)` /
  `resolve_sfx_value` / `resolve_sfx_tag`（tag → 具体文件名，`used` 去重）；`load_sfx_blacklist(config)`
  （全局 `.yekitrim/config.toml [sfx] blacklist` ∪ 项目 `polish.toml [sfx] blacklist`）。
- `fx`：`transient_duration_cap(event_type)` / `transient_default_duration(event_type)` /
  `normalize_event_ends(segments)`；`cap_pip_shots` / `align_pip_shots(segments, subtitles)` /
  `close_pip_shots` / `cap_handheld_sway` / `align_stickers_to_subtitles(segments, subtitles, mute_ranges)` /
  `cap_text_elements(segments, mute_ranges)`（文字元素**候补**守卫）/ `fit_element_span(start, end, subs, mute_ranges)`（元素时长规矩，私有）；
  常量 `MAX_PIP_SHOTS = 1` / `MIN_PIP_SECS = 1.5` / `MIN_RETURN_SECS = 0.5` /
  `STICKER_SNAP_MAX_SECS = 2.0` / `ELEMENT_MIN_SECS = 1.2` / `ELEMENT_MAX_SECS = 5.0` /
  `ELEMENT_MAX_GAP_SECS = 0.5` / `LEGACY_ZOOM_{MID,CLOSE,WIDE}`；
  **第 5 批**：`materialize_edl_intents(segments, &mut edl_clips, total_duration, mute_ranges) -> MaterializedEdl`
  （冷开场/回忆回溯意图 → `edl.clips` + 配额；**空 EDL 先把 `mute_ranges` 补集种成正片时间线**）；常量 `MAX_COLD_OPEN = 1` / `MAX_RECALL = 2` /
  `MIN_COLD_OPEN_SECS = 1.0` / `INTENT_COLD_OPEN = "cold_open"` / `INTENT_RECALL = "recall"` /
  `EDL_KIND_COLD_OPEN` / `EDL_KIND_RECALL`；`mod.rs::writeback_edl(materialized, clips)`。
- `rhythm`（§26，**已冻结**）：`params` / `plan` / `apply` / `clear` / `metrics` / `report_json`；默认档 `calm`。
- `download`：`sanitize_sticker_name(keyword)`、`cmd_download(project_path)`、`download_keyword_assets(...)`、
  `ensure_assets(project_path)`（`api/polish.rs` 的预览/出片前都会调）；`try_resolve_sticker` 是 `pub(super)` 实现。
- `placement`（`mod placement;` **私有**，外部一律走 `cmd_process`）：
  `Frame`（`sticker_scale()` = `min(W,H)/1080`）、`Rect`、`Blocked`、`ObstacleKind`、`CameraShot`、
  `Pick`；`pick_sticker_position(blocked, allow, seed, f) -> Pick`、`sticker_box(x, y, f)`、
  `camera_states_for(...)`、`transition_secs(label, prev_label, cut)`、`camera_expand`、`parse_origin_in_stage`、`parse_px`、
  `caption_band(f, font_size, stroke_px, position_pct)`、`danmaku_band(f, font_size, stroke_px, ys)`、`main_frame_rect(...)`、
  `stage_rect(stage, src, canvas)`（主画面矩形 `sr`，唯一真源是 `project::stage_draw_rect`）、
  `layout_region_rects(region, sr)` / `stage_region_rect(tl, br, sr)`（`layout.<区域>` 的百分比是 **`sr`** 的百分比）；
  常量 `FRAME_W|H = 1920|1080` / `STICKER_SIZE = 280.0` / `STICKER_MARGIN = 32.0` /
  `TRANSITION_SECS = 0.6` / `TRANSIENT_ZOOM = 1.10`。

## 边界

- ⚠️ **弹幕窗口归一化只在"真的变了"时写回**（`normalize_danmaku_windows`）：首页「切片总览」按最新文件
  mtime 排序，无条件重写 `data/danmaku.json` 会把老片段一直顶到最前面。**比较必须带容差** ——
  `serde_json` 读 f64 差 1 ulp（`WINDOW_EPS_SECS`），写严格相等就变成"每次 process 都重写一次"。
  回归：`clamp_display_windows_is_idempotent_across_json_roundtrip`。
- **Step 顺序是硬约束**：2b1（贴纸对齐字幕）**必须排在 2b（落点）之前** —— 落点是按贴纸的时间窗口
  算相机/字幕/其它贴纸障碍，时间挪了就得重算；2d（锚点解析）**必须排在 2b 与 2c 之后** ——
  `anchor="sticker"` 要读最终位置。1c 的配额守卫必须排在 Step 1 补默认运镜之后。
- **片头淡入是普通 overlay 事件，只在初次后处理补一次**（Step 3c：`mod.rs::ensure_intro_fade_in` +
  `INTRO_FADE_SECS = 0.5`，2026-09-22 起内建 `intro-black` 整条退役）：给**时间最早**且起点 < 0.5s 的段补一条
  `fade_in` 覆盖事件（`#000000` / `opacity 1` / `end = min(start+0.5, seg.end)`）；段跨度 < 0.2s、或**整片**
  已有与该窗口重叠的覆盖事件则**不补**（按时间挑段 —— 段之间会重叠）。
  **判据是 `polish.toml` 的 `intro_fade_seeded`**：判定过片头（补了、或已有覆盖事件盖着）就置位落盘，
  往后时间轴上改/拖/删全归用户 —— 删掉再跑 process **不许**补回来（用户报过"删了又回来了"）；
  段表空 / 最早段起点 ≥ 0.5s / 段太短**不**置位（那时片头还不存在，下次仍可补）。
  ⚠️ **别退回"看窗口有没有覆盖事件"** —— 那等于用户每删一次就再补一次。**不打 `manual`**。
  ⚠️ 必须排在 **Step 3 音效规则之后**：规则引擎会扫覆盖事件，排前面这条纯视觉黑场每跑一次都会多报一句
  「特效无匹配、移除音效」，字幕恰好命中规则时还会被挂上音效。
- ⚠️ **手工放的事件不受后处理改写**（用户报过「调整贴纸的结束时间没有成功」）：事件带
  `manual: bool`（`Sticker` / `Camera` 两个变体有，serde `skip_serializing_if = "is_false"`，老工程零迁移）
  时，`align_stickers_to_subtitles` / Step 2c 尾帧延长 / `align_pip_shots` 三处**全部跳过**
  （`Event::is_time_manual()`），**`cap_events` 的数量抽稀也跳过**。没有这个标记的话，用户改完 1 秒后
  自动后处理就把值吸回去了 —— 看上去就是"改了没生效"。
  回归：`cargo test -p yekitrim-polish --lib cap_events_never_removes_manual_events`；
  真实工程：`VT_POLISH_DIR=<片段目录> cargo test -p yekitrim-polish --lib diagnose_manual_events_survive_process -- --ignored --nocapture`（**会写回该工程的 polish.toml**，只在临时工程上用）。
- ⚠️ **ownership（规格 §2.2）与幂等是两条地基判据**（2026-10-19）：`manual` 的 `time/end`、
  `manual_pos` 归用户 —— C3 让 `layout_text_elements` 不写 manual 文字的 x/y（**照写 `source`**，否则丢样式）；
  B3 在 debug 下由 `cmd_process` 用 `ownership_violations` **assert**。
  ⚠️ **相机的 `origin` 不是所有权字段**（2026-11-21）：它是派生坐标（`panel: false`）——当所有权字段 ⇒ 跳过 manual ⇒ **新建的运镜选起点不生效**。
  A2 的 `diagnose_process_idempotent`（5 矩阵 × 两趟，**字段级** diff = 0，比 `toml::Value`
  树不比文本）+ `diagnose_process_three_runs`（收敛）—— process 会被预览/出片**隐式触发**，改任何一步先跑它。
- **元素（贴纸/文字）的时长规矩 —— 最短 / 最长 / 不跨静音空隙**（2026-10-11 用户口径：碎片要撑够、
  长句要封顶，否则时间轴上碎一块长一块"很奇怪"）。实现在 `fx.rs::fit_element_span`，
  由 `align_stickers_to_subtitles` 在**吸完字幕之后**调一次。
  **只动元素的 `time`/`end`，一个字节都不碰字幕**（用户明确：「不对字幕处理，只对其他元素处理」——
  字幕截短会吃字，元素是贴图/贴字，截短不丢内容）。四条规矩：
  ① **起点不动**（元素挂哪句话是 LLM 的意图，不因为时长不够就往前挪）；
  ② **不足 `ELEMENT_MIN_SECS`(1.2s) 就往后吞**字幕，直到「累计够长 **且** 落在某条字幕的结尾上」；
  ③ **不跨静音空隙**（`ELEMENT_MAX_GAP_SECS` 0.5s）：下一段字幕起点离当前 end 超过它就**停下**。
  ⚠️ 没有这道闸会一头扎进静音段（实测真实碎片链 `…→22.67` 跳到 `23.20`，等于在没人说话的画面上悬 3.88s）；
  ④ **超过 `ELEMENT_MAX_SECS`(5.0s) 硬截**。
  ⚠️ 旧的「sub 短于 0.2s 就跳过」守卫**已删**：它把**最该被治的碎片**（ASR 把一口气重复的音节
  切成 `21.92→21.97→22.02` 那种）全部放过。
  回归：`element_extends_past_min_duration_and_lands_on_subtitle_end`（用**真实碎片链**）/
  `element_is_capped_at_max_duration`（16.30s 那条真实样本）/
  `element_accepts_short_span_when_no_more_subtitles` / `element_fit_is_idempotent`
  （process 每次预览/出片都被隐式触发，必须收敛）。三条规矩分别做过变异自证。
- ⚠️⚠️ **贴纸有两个载体，凡"按贴纸处理"处必须同时认**（2026-10-15 用户报「加表情后不显示」）：
  `Event::Sticker`（老，**LLM/老工程**）与 `Event::Element{kind:"sticker"}`（**编辑器「贴纸轨」点＋新建的**，
  `addElementAt` 有意一律产 `element`）。字段名不同：关键词 `keyword` vs **`text`**、`x`/`y`
  `String` vs **`Option<String>`**。**读走** `Event::is_sticker()/sticker_keyword()/sticker_name()/
  `sticker_source()/sticker_span()`（`render-core/src/types.rs` 一处），**写走** `set_sticker_resolved()/
  `set_sticker_position()`。⚠️ **别在每个消费点各写一遍 `match`** —— 这次正是那样漏的：Step 2、2b、
  `cap_events`、2c、`sticker_obstacles` **五处全只认老载体** ⇒ 手动贴纸拿不到落点/`name` 永不解析/
  不互斥/逃过上限，**而 LLM 放的老载体一切正常**（两边各自看都正常，故极难归因）。⚠️ `sticker_span()`
  坐标是 `Option`：落点是**写**，不能 `continue` 掉"还没落点"的条目；障碍表相反。
  回归 `element_sticker_goes_through_the_same_pipeline_as_legacy_sticker`（跑**真** `cmd_process`
  钉"消费点有没有用访问器"；变异自证：退回只认老载体 ⇒ 当场红）。
- **落点规则**（`placement.rs`，Step 2b）：帧坐标固定 1920×1080，贴纸固定 280×280，`ev.x/ev.y` 是**盒子左上角**
  百分比（模板 DOM/canvas 与 `engine.js` 都按左上角消费，模板再钳制到 `[32,1608]×[32,768]`）。
  障碍 = ①人物 `layout.liver`、②弹幕区域 `layout.danmaku_area`（都在**源帧**坐标里框选，出片时视频整体被
  运镜缩放 → 先按该贴纸可见窗口的相机状态投到输出帧，zoom 取模板 Canvas 路径的 1.5/1.8 × 瞬时特效预留 1.10，
  窗口内的缩放过渡两端都算）、③字幕带（Canvas `cy=960` 与 DOM `bottom:80px` 两条路径取并集；
  ⚠️ **2026-10-15 起还读 `[style.subtitle].position_pct`** —— 此前只有出片模板与转译器读它，
  落点求解器读的是写死的历史带位，于是"用户把字幕拖到中部 ⇒ 文字压上字幕"。
  三处必须同口径；`None`（老工程没写过）走历史公式 = 零回归）、
  ④真实弹幕带（只有 `data/danmaku.json` 非空时才是障碍，按模板 `y=(d.y/1920)*540` 映射）、
  ⑤同刻已落位的其他贴纸（互不相压）。
  候选 = 16px 网格 + 各障碍边缘的「临界位」；先取 clearance 最高的一档（×0.9），在该档里按「盒心到帧心距离」
  排序取前 8 个，再用 seed（`fract(idx*3.7 + seg.start*1.3)`）伪随机挑一个 —— 即**安全区中心而不是边缘**，
  且逐条有变化。帧内实在没有安全位时退化为「重叠代价最小」并在日志里告警；最小余量 < 20px 时也会提示
  「选框贴得太近」。
- **`layout.sticker_area` 已降级**：`grid` 完全不参与落点（会打日志说明）；手写的自由选框只作**偏好**
  （里面有安全位就优先用，没有就自动放弃）。
  只读诊断：`VT_PROBE_ROOT=<media 目录> cargo test -p yekitrim-polish --lib diagnose_sticker_placement -- --ignored --nocapture`
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
  ⚠️ **前后镜头按时间算、不按文件顺序**（段之间会重叠）。
- **`cap_handheld_sway` 把非近景上的 `handheld` 剥掉**（只剥不补、幂等、日志报条数），排在画中画收工
  **之后** —— 收工补出来的回镜头承接了前一条的 `handheld`，一起吃这道守卫。
  **别再退回"靠 prompt/界面拦"**：LLM 会写错景别、用户也会改景别，守卫是唯一兜底。
- **预算**：`content_budget` 密度 = 每 5 条字幕/12s 一个贴纸、每 8 条字幕/15s 一个 sfx；
  `seg_count = Some(n)` 时上限 `max(1, n-1)`（至少留一个 segment 无特效），`None`（LLM prompt）时省略。
  `cap_events` 均匀保留，**手工事件不参与抽稀**（额度只约束 LLM 生成量）。
- **运镜（camera）**：`process` 给每个 ≥1s 且没有运镜的段补 `zoom="中", anchor="liver"`（整段）——
  所以运镜是**全程开着**的，不是点缀。改默认档位/默认锚点就在这一处
  （`process/mod.rs` Step 1 与 `llm/mod.rs` Step 4 两处同改）。Step 2d 的
  `resolve_camera_anchors` 把 `anchor`/`anchor_to`（面板「起点 / 终点」，**同一概念的两个端点**）解析成
  `origin`/`to_origin`（**每次都覆盖，manual 也覆盖**）；终点按**块尾**解析，**与起点相同 ⇒ 清 `to_origin`**。
  - **三档倍率的当前值：中 1.3 / 近 1.5 / 画中画 0.55**（用户定：中 1.3、近 1.5）。
    **放大档位不会露底**：绕不动点 scale，`s ≥ 1` 时四周外扩；露底只发生在画中画（刻意留黑边）。
    再调大是安全的，别再拿「怕露出背景底色」当理由往下压。
  - **旧内建默认按值迁移**：`process/mod.rs::migrate_camera_zoom` 在 Step 0a 把
    中 `1.15 → 1.3`、近 `1.3 → 1.5`、远 `0.7 → 0.55`（**只认这几个旧默认值**，手改过的不动，
    日志逐条报）。三处兜底默认值（`default_zoom_mid/close/wide()`、模板 `camFactor`、
    `engine.js` `camFactor`）必须一起改。
  - **画中画的判据是 `|zoom-1| > 0.001`，不是 `zoom > 1.001`**（`templates/index.html` 与
    `PolishPreview.vue` 两处）。写成后者会把「远」档整档吞掉，画面完全不动。
  - **远景进出是硬切，不是缓动**（用户要求）：过渡的一端只要是「远」，「zoom」与不动点就在
    事件起点**一次性到位**。贴纸避让侧走 `placement::transition_secs()` → `CameraShot.transition = 0`
    —— 别写回常量 0.6，否则会按 0.6s 过渡凭空多算一个早已结束的旧状态当障碍。
    ⚠️ `med`/没写 `speed` ⇒ **老式子 `min(dur*0.5, 0.6)`**（以前是常量 0.6）；**硬切 `cut` 同理**（只接缝到位、块内位移照走）。
  - **位置是连续状态**（同 `zoom`）：接缝接**上一块落点** → 本块 `origin`，块内再走到 `to_origin`；
    没写 `anchor_to` ⇒ 整块静止。⚠️ **别退回「每块绝对赋值」**（块缝瞬跳 928px/一帧；四份实现同改）。
  - **老工程迁移**：`origin = "50% 50%"` 且无锚点的事件补成 `anchor="liver"`（有日志计条数）——
    这个值从来不是用户能设的，所以当成「没指定」。
  - 只读诊断：`VT_POLISH_DIR=<项目目录> cargo test -p yekitrim-polish --lib diagnose_camera_anchors -- --ignored --nocapture`
    （`VT_CAM_JSON=1` 多打一行 `CAM_JSON={...}`，可喂给模板 `getActiveCamera` 与 `engine.js cameraAt` 对拍）。
- **手工屏蔽区间内的字幕不参与后续预算/位置计算**：`cmd_process` 读 `data/subtitles.json` 后先按
  `config.mute_ranges` 过滤（`MuteRange::overlaps`），后续 position/budget 全基于清理后的字幕。
  ⚠️ **但屏蔽区间不再切段、也不再删事件**（用户要求：屏蔽只做标记）：段表与事件原样留着，
  出片合流后由 `Ffmpeg::cut_ranges` 剪掉、预览跳过。以前这里还会用 `split_segments_by_mute_ranges`
  把屏蔽区事件删掉（process 会被预览/出片隐式触发）—— 已删除，**别再加回来**。
  回归：`mute_range_keeps_events`（真跑 `cmd_process`，把删事件钉红）。
- ⚠️ **屏蔽区间只跳过、不删事件**（用户 2026-10-19 报「LLM 剪辑和后处理没过滤屏蔽段」）：
  判据 `project::event_starts_muted`（左闭右开、只看**起点**；`Camera`/`Overlay` 例外）。
  用在 `cap_events` 额度、2b/2b2 落点、2c 延长、音效规则（屏蔽段音效要留着）、文字守卫、LLM 生成与精挑。
- **节奏已冻结**（§26）：默认 `calm`，process 不再生成派生事件 —— 节奏改由 LLM 的编排轮产出，
  process 只做资源热加载与应用（账本仅供迁移期清理）。
- **文字元素是候补**（用户 2026-10-19：文字突出、指向性强，滥用适得其反）：`cap_text_elements`（Step 1d2b）
  兜底 —— 与贴纸（两个载体都算）重叠就删、每段 1 条（留最早）、`manual` 豁免、屏蔽段的不占名额。
  prompt 同步：写**关键词 2~8 字**、不与贴纸同段、只在没合适贴纸/其它效果已密时用。
- **冷开场 / 回忆回溯：LLM 表达意图、`process` 落实成 `edl.clips`**（第 5 批，Step 5）。
  LLM 在**运镜事件**上写 `anchor = "cold_open"` / `"recall"` 表示"这段值得在成片里再放一次"
  （**不是**景别 —— 见 `llm/segment.rs` 的提示词段）。落实 = 往 `edl.clips` 里加一段**与它源区间
  完全相同**的 clip（冷开场插**数组最前**、回忆回溯**追加在末尾**），复用第 2 批（重放）已经验过的
  多 layer + 分段音轨那套，**本批没有新原语**。
  ⚠️ **配额与合法性只能在这里兜**，与 `cap_pip_shots` 同一条理由：**LLM 是按段窗口生成的，数不出
  "整片几次"**。冷开场整片 [`MAX_COLD_OPEN`]=1（按时间序留**最早**那条）、回忆回溯 [`MAX_RECALL`]=2；
  落选的只**清意图**、不加段（日志报「太密」）。
  ⚠️ **幂等靠"意图是一次性触发条件"**：落实时把两个认得的 `anchor` **就地清掉**（`clear_anchor`，
  别的 anchor 一律不碰），所以第二趟没有任何候补。**别改成"看 EDL 里有没有这一段"去判重** ——
  用户删掉重放段之后再跑 process 就会把它复活（`intro_fade_seeded` 那条教训一模一样的形状）。
  带 `manual` 的运镜不参与（用户自己摆的事件不被后处理改写）。
  ⚠️ **触发条件 = 老工程零回归的判据本身**：`edl.clips` 非空 ⇒ `mute_ranges` **整个被忽略**
  （`resolve_edl`、规格 §2.3）⇒ 落实前必须先把**补集**种进空 EDL（`edl_clips_from_mute_complement`），
  否则正片消失、屏蔽段全回来；所以只在**真的落实了**时才写
  （`writeback_edl(edl_intent_done, clips)`），没意图的工程**一个字节都不碰** `cfg.edl`。
  ⚠️ **这一条是两层挡的、两层各有各的观察点**：把 `writeback_edl` 改成无条件 `Some` 之后，
  端到端那条 `process_without_edl_intent_never_writes_edl` **照样绿** —— 空 `Edl` 会被
  `ProjectFile::edl` 的 `skip_serializing_if = "Edl::is_empty"` 静默吃掉，症状完全相同
  ⇒ 写回门必须有**纯谓词**判据（`process_writeback_gate_only_touches_edl_when_materialized`）。
  回归：`cold_open_only_one_per_clip`（3 候选 → 只剩 1）/ `cold_open_materialize_is_idempotent` /
  `cold_open_on_empty_edl_seeds_the_mute_complement` / `non_empty_edl_is_never_reseeded` /
  `intent_only_edl_from_old_version_is_rebuilt` /
  `no_intent_keeps_edl_untouched` / `manual_camera_intent_is_ignored` /
  `materialize_skips_impossible_spans` / `recall_is_capped`。
  ⚠️ 改内建 prompt 对**已有配置无效**（`.yekitrim/config.toml` 里是 `config init` 填的副本）。
- `resolve_event_sfx` 未命中规则的事件按"宁缺毋滥"清空音效；`Step 3b` 对"字幕命中规则但 LLM 没放事件"
  的时刻在预算内补建 sfx 事件。屏蔽列表条目的语义见 `assets/sfx_catalog.rs::is_blacklisted`。
- 真实工程诊断：`VT_POLISH_DIR=<项目目录> cargo test -p yekitrim-polish --lib diagnose_pip_and_silence -- --ignored --nocapture`
  （画中画配额/对齐/收工 + 去静音阈值 + 贴纸对齐；含 2.0/3.0/3.5/5.0s 阈值对照）。

> 改动只允许落在本目录；跨模块改动先读对方卡，并跑两条门禁。

## 验证

```bash
cargo test -p yekitrim-polish --lib process
```

改 `placement.rs` 的几何、`fx.rs` 的守卫或 `cap_events` 抽稀后必跑；再补一次
`cargo check -p yekitrim-polish`（0 warnings）。
