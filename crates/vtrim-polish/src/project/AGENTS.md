# project/ — 项目配置类型（vtedit 风格）、去静音、校验与老工程迁移

> 范围：`crates/vtrim-polish/src/project`/** ｜ 上游：polish 全部子模块（`process`/`render`/`llm`/`cover`/`commands`）都读它 ｜ 验证：`cargo test -p vtrim-polish --lib project`

## 职责

`polish.toml` 的唯一真源。`config.rs` 定义 `ProjectFile`（+ `Event` / `Segment` 类型经 `vtrim-render-core`
复用）与画面构成推导；`silence.rs` 实现「去静音」= 静音区间 ↔ 屏蔽轨道；`validate.rs` 负责加载 /
原子写 / 路径锁 / 校验；`legacy.rs` 把老工程的 `duration` 迁移成 `start`+`end`。

## 入口

- `config`：`ProjectFile`（`source` / `layout` / `style` / `canvas` / `stage` / `sticker` / `sfx` / `cover` /
  `mute_ranges` / `silence_ranges` / `segments` / `processed` …）、`Event` / `Segment`、`MuteRange`
  （`normalized()` / `overlaps(start, end)`）、`CoverConfig` / `CoverLayer`（图层栈一项：`id` + `visible`；
  **顺序 = 数组顺序、从下到上**，解析在 `cover.rs::resolve_cover_stack`）/ `CoverEffect`（`id` 是栈引用的
  稳定标识；`visible` 不存在 —— 可见性**只住 `layers` 里**）/ `SfxConfig` / `FilterConfig` /
  `SourceRef` / `CanvasConfig` / `StageConfig` / `StickerConfig` / `StageFit`；
  `resolve_stage(src) -> StageFit`（画面构成的**唯一推导**）、`canvas_contract`、`canvas_size`、`is_portrait`、
  `region_rect`、`stage_bg_mode` / `stage_position` / `sticker_position_mode`、`to_toml_commented`、`example`、
  `clamp_segments_to_duration`；**没有** `split_segments_by_mute_ranges`（已删除，见边界第一条）；
  常量 `CANVAS_LANDSCAPE_WH = (1920,1080)` / `CANVAS_PORTRAIT_WH = (1080,1920)` /
  `CAPTION_SAFE_WIDTH_RATIO = 0.875` / `CAPTION_MAX_LINES = 3` / `CAPTION_MIN_FONT_RATIO = 0.7` /
  `STICKER_SAFE_MARGIN = 32.0`。
- `silence`：`apply_silence(project_path)` / `apply_silence_with(project_path, min_gap, noise_db)`、
  `clear_silence(project_path) -> usize`、`detect_ranges(project_path, allow_audio_fallback)` /
  `detect_ranges_with(...)`、`is_silent` / `mute_range_count`、`SilenceApply { ranges, added, total }`；
  纯函数（含单测）`union_ranges` / `subtract_ranges`；常量 `SILENCE_PAD_SECS = 0.8`
  （阈值 `DEFAULT_MIN_GAP_SECS = 3.5s` / `DEFAULT_NOISE_DB = -30.0` 在 `vtrim_core::cutting`）。
- `validate`：`load_project(path)` / `load_config_from(path, filename)`、`update_polish_toml(path, |cfg| …)`
  （**锁内重新 load + 只覆盖自己负责的字段**）、`write_atomic(path, contents)`、`validate_project(cfg, path) -> Vec<String>`。
- `legacy`：`upgrade_event_times`（re-export）、`upgrade_toml_value(value)`。

## 边界

- **「去静音」与当前屏蔽状态「相加」**（用户要求）：检测出的静音**并入**「屏蔽」轨道，
  重叠/相邻的合成一段（不是追加 —— 追加会在手动 10~20s 的屏蔽上再叠一块 16.5s 的红块）；
  状态文案要带上相加后的总数（`已自动屏蔽 N 段静音 · 屏蔽轨道共 M 段`），而不是只报自动那几段。
  撤销是**差集**：只减掉当初自动加进去的区间（`silence_ranges`），手动屏蔽原样留下。
  应用 = `轨道 ∪ 检测结果`，`silence_ranges` 只记**真正自动加进去的那部分**（检测结果减掉本来就有的屏蔽）；
  重叠/相邻的合成一段；检测结果两侧各留 `SILENCE_PAD_SECS`(0.8s) **气口**（贴边剪会削掉首尾音节）。
  **默认阈值 `DEFAULT_MIN_GAP_SECS = 3.5s`**（字幕间隔 3.5s 以内一律不处理，用户要求；
  实际剪掉 = 间隔 − 1.6s）。`union_ranges`/`subtract_ranges`/`pad_ranges` 是纯函数，有单测；
  `pad_ranges` 收没了（`e - s <= 0.05`）就整段丢掉。
- ⚠️ **屏蔽只做标记：不许切段、不许删事件**（用户要求）。`mute_ranges` 由**消费侧**生效 ——
  出片在合流后 `Ffmpeg::cut_ranges` 剪掉、预览跳过、字幕在 `cmd_process` 里按 `MuteRange::overlaps` 过滤。
  曾经有个 `split_segments_by_mute_ranges`：`cmd_process` / `cmd_llm` 会用它把屏蔽区里的事件
  **从 `polish.toml` 删掉**并落盘，而 `cmd_process` 会被预览/出片**隐式触发** —— 于是"随手屏蔽一段"
  等于**永久丢事件**，撤销屏蔽也回不来。**已删除，别再加回来**（回归：`cargo test -p vtrim-polish --lib
  mute_range_keeps_events`，它会在删事件时打红）。
- **静音检测先按时间排序 + 合并重叠区间再取相邻间隔**（`vtrim_core::cutting::silence::silence_gaps_from_segments`，含单测）
  —— 乱序/重叠的字幕按文件顺序硬算会造出几十秒假静音。去静音只按字幕间隔检测（列表接口
  `allow_audio_fallback=false`，避免逐条解码视频），音频只作兜底。
- ⚠️ **字幕/屏蔽轨道必须保持按时间有序**：`splitSelected` 拆出来的后半段要
  `splice(i+1, 0, …)` 插在原块**后面**，**不能 `push` 到数组末尾** —— 追加会让列表
  （以及落盘的 `clip.srt` / `data/subtitles.json`）乱序。乱序会连带毁掉所有按顺序读它的
  地方：去静音按相邻间隔算静音（曾判出 13 段/121s 的假静音，用户原话"去静音完全对不上，
  像用了别的素材"）、预览按时间二分取当前字幕。`PUT /api/polish/subtitles` 落盘前会按
  `start` 排序（老数据再存一次即自愈），`detect_silence_from_srt` 也会先排序+合并重叠 ——
  但别因此就允许编辑器产出乱序列表。
- **时间模型 = 开始 + 结束**（v6.6）：事件字段是 `time`（开始秒）+ `end`（结束秒），**没有 `duration` 了**；
  分段/字幕/屏蔽本来就是 `start`/`end`，四类块因此共用同一套控件。老工程的 `duration` 由
  `vtrim-render-core/src/legacy.rs::upgrade_event_times` 在**反序列化前**换算（`end = time + duration`，
  渲染 1:1 不变，下次保存即完成迁移），入口三处：`polish.toml` 加载（`validate.rs`）、
  `PUT /api/polish/project` 的 segments、LLM 事件输出 + 老 checkpoint（`llm/events.rs`）。
  （原来还有第四处「wasm 预览配置」—— 那条浏览器真渲染路径随「作废 wgpu」一起删掉了。）
  缺 `end` 的由 `process` Step 0 补默认（`Event::default_end`；`sfx` 例外 = 用文件原始长度）。
  三处求值器统一走各自的 `evDur()` / `evDuration()`，别再直接读 `.duration`。
- ⚠️ **已下线特效类型的人话报错**（2026-10-01，为 `docs/plan/fx-type-consolidation.md` 的类型收敛铺路）：
  `validate.rs::retired_effect_message()` 把 serde 的 `unknown variant \`stutter\`…` 包成
  "该工程含**已下线的特效事件**（…）" + 两条路（旧版本删事件 / 直接编辑 `polish.toml`），
  **普通解析错误原样透传**（判据只在 serde 明说 `unknown variant \`X\`` 时命中，`expected one of` 列表不算 ——
  裸匹配类型名必然误判）。那份名单是**历史清单**，别拿效果词汇表去校验它（那几个类型正是要消失的）。
- **画面构成（`[canvas] orientation` + `[stage] position_pct / bg_mode / scale / padding / portrait_*`）与「衬底」**：
  摆放与背景由 `ProjectFile::resolve_stage(src)` **唯一推导**，模板与编辑器只读注入的 `_canvas.stage`，
  两边不各算一份。⚠️ **2026-10-02 改版**（用户拍板）：① **位置 = `position_pct`（0–100，界面五档吸附**
  0/25/50/75/100**）**，老枚举 `position` **只读兼容、不再写出**（`auto` → 50 居中）；② **轴只看比例**
  （同比 → `fill`，否则 x/y）—— **`auto` 特例已删**，不匹配的源改成**适配留边**（老 `auto` 是裁切）；
  ③ 缺省背景 = `blur`（UI 删掉「自动」，老值按 blur 处理）；④ `padding` **字段保留、UI 不再出现**
  （与 `scale` 不同量、且 `deny_unknown_fields` 删不得）；⑤ 竖屏画布三个 `portrait_effects/camera/sfx`
  **bool、缺字段 = false = 关**（整类事件只在渲染侧跳过，**事件本身不动**）。
  ⚠️ **别拿 `StageFit.position` 算几何**（那是过渡期的兼容输出，25/75 会被压成 center）——
  消费侧一律读 `StageFit::position_pct`。
  ⚠️ **竖屏出片露黑边**（用户报「精修竖屏时特效如震动会显示出背景的黑色，要在视频底下垫一层模糊」）
  两道闸缺一不可：
  ① `resolve_stage` 的背景门控是**画布方向**、不是 `axis` —— `axis=="fill" && !portrait` 才 `none`
  （横画布铺满保持"不画" = 存量逐字节零回归），**竖画布铺满也照 `bg_mode` 铺衬底**；
  ② **出片是 canvas `renderFrame` 逐帧绘制，DOM 的 `#bg-video-wrap` 完全不参与导出** ——
  `clearRect` 之后的透明区落进 JPEG 就是纯黑。只把 `stage.bg` 消费在 DOM 上，表现就是「预览里有
  模糊底、成片被 shake / 画中画 / 手持摇摆挪开露纯黑」：**不报错、不崩溃，只有逐帧像素能发现**。
  `renderFrame` 的 `drawStageBackdrop` **必须画在视频层之前**：`blur` = 同一帧 bitmap 按 cover 铺满
  + `blur(blur_px) brightness(.5) saturate(1.15)` + 绕中心放大 `blur_img_scale`；`black`/`color` 铺色；
  `none` 一个像素都不画（透明 = 黑，与改造前同源）。模糊半径/倍率一律取契约里的值，别写死 28px。
  ③ **摆放与运镜同样必须落在 canvas 出片路径里**：`resolve_stage` 推的 `axis/position/scale/padding`
  由模板 `stageDrawRect` 算成画面矩形（`ctx.translate/scale` 送进 CTM，`drawImage` 的 dst 仍是
  `(0,0,CW,CH)` 字面量），**竖素材再额外吃掉运镜**（`getActiveCamera` 开头 `CFG.vertical` →
  `zoom=1 / handheld=0`；`CFG.vertical` 由 `render/server.rs:261` 按「源 `h > w`」注入，**与画布方向无关**）。
  只在 DOM/CSS 上消费这套推导的表现是「成片被拉成宽屏 + 竖素材在成片里照旧缩放」（用户 2026-09-21
  报的原文），预览侧全程正常、**不报错**。`axis=fill` + 无 padding + 源比例 == 画布比例 必须仍算出
  `(0,0,CW,CH)` —— 横屏存量逐字节零回归的判据（真出片对拍过 SHA-256）。
  钉子：`node webui/scripts/check-stage-backdrop.mjs`（73 项 + 8 处变异自证；③c 段钉摆放几何）、
  `node webui/scripts/check-handheld.mjs`、Rust `template_stage_hooks_match_contract` +
  `portrait_canvas_fill_still_paints_backdrop`、真实像素 A/B
  `VTEDIT_BACKDROP_DIR=<竖屏工程> VTEDIT_BACKDROP_T=<被挪开的时刻> cargo test -p vtrim-polish --test portrait_backdrop -- --ignored --nocapture`
  （A/B 判据只断在**真露底帧**上 + "至少一帧露底"；跑完看输出的「真露底 n / 总帧」，n=0 就换 `T`
  —— 竖素材的运镜已被门控吃掉，露底只能来自其它特效或 `[stage] scale` 这类结构性留边）、
  `VTEDIT_FIT_DIR=<竖素材工程> cargo test -p vtrim-polish --test portrait_stage_fit -- --ignored --nocapture`。
- **读取优先级**（`vtrim-render-core/src/types.rs`）：`free_rect()` → `region_rects()`/`region_origin()`
  一律**自由选框 > grid > origin_grid/origin**；JS 侧同一规则在 `webui/src/polish/engine.js` 的
  `regionRect/regionOrigin`（GRID_RECTS 表须与 Rust `grid_rect` 对齐）与 `templates/index.html` 的 `liverOrigin`。
  保存选框写 `polish.toml` 的 `layout[区域].top_left/bottom_right`（"55.00%" 百分比字符串），
  清除即删除两个角回到原 `grid`（grid 永不被框选改写 → 旧工程零迁移）。
- **长任务写 `polish.toml` 一律走 `project::update_polish_toml`**，不许"开头 load、结尾整份写回"：
  LLM 跑两分钟期间用户存的选框/封面/屏蔽会被结尾那次覆写静默吞掉。写盘走 `write_atomic`。
- 路径参数、`polish.toml` / 全局配置的读-改-写走 `vtrim_core::filelock::with_path_lock(_sync)` 串行化；
  同步上下文的写（`config::edit_toml_at` / `patch_config_file`）走 `with_path_lock_sync`（**非重入**的 std Mutex，
  只包微秒级读改写）—— 管线的写与设置页保存是同一份文件，无锁时后写的会把先写的键整段覆盖。

> 改动只允许落在本目录；跨模块改动先读对方卡，并跑两条门禁。

## 验证

```bash
cargo test -p vtrim-polish --lib project
```

含 `union_ranges` / `subtract_ranges` / `pad_ranges` 的纯函数单测与 `update_polish_toml_keeps_concurrent_edits`。
真实工程上的去静音阈值对照（2.0/3.0/3.5/5.0s）：
`VT_POLISH_DIR=<项目目录> cargo test -p vtrim-polish --lib diagnose_pip_and_silence -- --ignored --nocapture`
