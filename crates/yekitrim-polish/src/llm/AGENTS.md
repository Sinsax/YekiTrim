# llm/ — 精修侧 LLM 链路（分段、逐段事件、关键词、逐段状态）

> 范围：`crates/yekitrim-polish/src/llm`/** ｜ 上游：`commands::{cmd_llm, cmd_llm_segment}`、`yekitrim-server::api::polish`（`POST /api/polish/llm`、`/api/polish/llm-segment`）｜ 验证：`cargo test -p yekitrim-polish --lib llm`

## 职责

精修工程的 LLM 三步：`segment.rs` 全局分段（一窗一段，产出 `Segment`）+ 提示词；`events.rs` Step2 **逐段**事件生成
（走 `yekitrim_core::llm::agent::Agent` + checkpoint 续传 + 贴纸批量精挑）；`keywords.rs` 弹幕过滤词与高亮词；
`status.rs` 逐段生成状态快照（编辑器据此给"没产出事件的段"上警示色）；`mod.rs` 串起整条流水线
（含 `regenerate_segment_events` 单段重做）。事件字段是 `time` + `end`。

## 入口

- `mod.rs`：`run_llm_pipeline(llm_cfg, subtitles, danmaku, event_type, sticker_source, sticker_budget, sfx_budget, sfx_blacklist, checkpoint_dir)` →
  `(Vec<Segment>, 弹幕过滤词, 高亮词, Vec<SegmentGenStatus>, Option<Vec<EdlClip>>)`；`regenerate_segment_events(llm_cfg, subtitles, sticker_source, sfx_blacklist, event_type, segments, index, project)`。
- `segment`：`generate_segments(...)`、`build_segmentation_system_prompt(sticker_source)`、`build_segmentation_user_prompt(...)`、
  `build_events_user_prompt(segment, subtitles, candidate_text)`。
- `events`：`EventGenTask::{new, with_project, build_windows, apply_outputs}`、`run_event_agent(...)`、
  `StickerChoiceContext` / `choose_sticker_files_batch(...)`。
  ⚠️ Step2b 有**两档**（`mod.rs::run_sticker_choice_step`）：本地包按 `(包名, tag)` 分组；
  **emoji 那一档**按 keyword 是**维度查询**（`emotion:joy,usage:punchline`）识别、候选 = `search_best`，
  候选标签形如 `1f602.gif 😂`（模型认得出，而 `Path::file_stem()` 取回的正好是码点 ⇒ 写进 `name`）。
  只认维度查询 ⇒ 老工程的自由词走原路，不打乱存量行为。
- `keywords`：`extract_keywords(client, subtitle_texts, danmaku_texts) -> (danmaku_filter, highlight_words)`、`build_prompt`。
- `status`：`STATUS_FILE = "polish_events.status.json"`、`SegmentGenStatus` / `LlmRunStatus`、
  `remap_to_segments(...)`、`status_path` / `save` / `load` / `upsert_segment` / `clear`。

## 边界

- **统一客户端**：一律走 `yekitrim_core::llm::LlmClient::from_config(llm_cfg)`，不要自己 `reqwest`。
  **新增链路记得挂 `.with_usage_context(...)`**：`label`（链路名）+ `project`（项目/切片目录）——
  精修侧现有五处挂点（四个文件）：`精修分段` / `精修关键词`（`llm/mod.rs`）、贴纸批量精挑（`events.rs`）、
  `贴纸包标签` / `贴纸扫描标签`（`commands.rs`）。Agent 系任务走 `AgentTask::usage_context()`。
  **不挂 context，统计里就是空 label —— 别让用量静默消失。**
- **并发只有一个旋钮**：`.yekitrim/llm.toml` 的 `[llm] max_concurrent`（默认 3），事件检测 / 精修事件生成 /
  字幕订正共用，解析统一走 `concurrency_for(link)`（`[llm.links.<link>.run] max_concurrent`，留空跟随全局）。精修**没有自己的并发配置项**，
  `Agent::new(llm_cfg).with_max_concurrent(llm_cfg.concurrency(None))`。别再在代码里写死 `3`。
- ⚠️ **精修的 `max_tokens` / `temperature` / `thinking` 走 `[llm.overrides.polish]`**：
  `run_llm_pipeline` / `regenerate_segment_events` / `run_event_agent` / `choose_sticker_files_batch`
  各在入口 `llm_cfg.for_link(LlmLink::Polish)` 一次（幂等）。**写死的 `.with_thinking(Some("disabled"))`
  已全部删掉** —— 关思考现在是 `LlmLink::Polish::default_thinking()` 给的链路默认，
  想开就 `[llm.overrides.polish] thinking = "enabled"`（拆散在调用点就等于这个旋钮不存在）。
  ⚠️ 同一改动让 `精修关键词` / 贴纸精挑从"跟随 `extra_params.thinking`（模板里是 enabled）"变成
  **默认关思考**（分段/逐段事件本来就是关的）—— 这是有意的统一。
- ⚠️ **checkpoint 按提示词指纹失效**：窗口 id 折入 `AgentTask::prompt_fingerprint()`（在
  `agent.rs::run_windows` 里改写），否则「字幕没变但 prompt 改了」会命中旧窗口、拿回上一版结果。
  改了 prompt / 贴纸来源 / 音效屏蔽后重跑，日志会显示「实际跳过 0 个窗口（内容匹配）」。
  精修侧的窗口 id 还**必须带时间段**（`seg_{i}_{start}-{end}@fingerprint`）：命中条件只有 `id && items`，
  而重新分段后「新的 seg_1」与「旧的 seg_1」的 ±3s 字幕窗口**可能一字不差** → 会把别的时间段的事件套过来
  （"续传之后显示不正常"）。带区间后重新分段必然失配。
- **空结果 / 失败一律不覆盖**（`events::apply_outputs`）：`output == None`（429/超时/解析失败）
  与 `Some([])`（模型返回空）都**跳过硬**，保留原有事件 —— 空结果是"这一次没生成"，
  不是"这一段应该有 0 条事件"。回归钉子：`events.rs::tests`（空/失败/正常三条）。
  旧行为是用 LLM 输出**无条件覆盖** `seg.events`，一次限流就把整段已有事件抹成 0。
- **逐段状态快照** `<project>/.yekitrim/polish_events.status.json`：`cmd_llm` 落盘、
  `polish_project_get?preview=1` 注入 `_llm_status`、`TimelineEditor` 给**没产出事件的段**上琥珀色
  （成功 = 当前颜色，用户要求），分段属性面板显示失败原因 + 「只重做本段事件」。
  ⚠ **落盘前必须 `remap_to_segments`**：`cmd_llm` 拿到窗口结果之后还会砍掉超出视频时长的段，
  窗口下标与最终段下标已经不是一回事（不对齐就会给错段标色）；
  成败**不看最终事件数** —— 空事件段会被 process 补上默认运镜，用 `events.is_empty()` 当判据
  会让失败永远显示成成功。（屏蔽区间**不再**改段的构成 —— 只做标记，见 `project/AGENTS.md`。）
- **入口语义**：`POST /api/polish/llm {full}` 的 `full` **默认 true = 全部重做**（先清 checkpoint；
  编辑器按钮再带一次二次确认，因为它会重写当前全部段与事件，含手工加的）；
  `POST /api/polish/llm-segment {index}` = **只重做这一段**（不重跑分段、不走 checkpoint、
  失败保留原事件并让任务失败可见），跑完照常自动 `process`。
  CLI `yekitrim polish llm` 仍走 `full=false`（长片重跑贵，续传留给命令行）。
  新增任务 kind `llm_segment` 的档位见 `JobClass::for_polish`（与 `llm` 同档 Medium）。
  `regenerate_segment_events` 的三条有意差异：不重跑 Step1 分段、不走 checkpoint、只换该段事件；
  仍跑 Step2b（贴纸精挑）与段级对齐。
- **前缀缓存**：`DEFAULT_PROMPT` 里 `{segments}` 必须留在**最后**，每场固定的弹幕爆发块走
  `{hot_windows}` 排在它之前（公共前缀 86% → 98%）。provider 按最长公共前缀命中上下文缓存，
  把变化部分放中间等于把后面的固定内容全浪费掉。老模板（用户 config.toml 里的副本）没有
  `{hot_windows}`，`build_prompt` 会把它拼进 `{segments}` 的替换值里——位置等价、不丢内容。
  `response_format = { type = "json_object" }` 放在 `[llm] extra_params` 里。
- **意图一次收走**（第 5 批，Step 4c）：`materialize_edl_intents` 在**落盘前**先跑一遍
  （与 Step 4b 同一条理由：编辑器里看到的就已经合规 —— 配额已裁、`anchor` 已收走）。
  ⚠️ 它**清掉** `anchor = "cold_open"` / `"recall"` 这两个值，别的 anchor 一律不碰；
  清掉之后落盘的段表里就没有意图了 ⇒ `cmd_process` 不会再落实一次（幂等靠这一条，见 `process/AGENTS.md`）。
- **落盘前的守卫与 `process` 成对**：`llm/mod.rs` Step 4/4b 会补默认运镜（`zoom="中", anchor="liver"`）、
  跑 `cap_pip_shots` / `align_pip_shots` / `close_pip_shots` / `align_stickers_to_subtitles(…, mute_ranges)` /
  `cap_text_elements(…, mute_ranges)` —— 与 `process/mod.rs` Step 1/1c 是**两个重复实现，要一起改**；
  让编辑器里看到的就已经合规。
- ⚠️ **屏蔽段里的事件不生成、不精挑**（2026-10-19 用户报「LLM 剪辑没过滤屏蔽段」）：
  `EventGenTask` 带 `mute_ranges`，`parse_output` 与 `load_windows` 用
  `project::event_starts_muted` 把**起点落在屏蔽区**的事件剔掉（日志「✂ 屏蔽区间内不生成事件：剔除 N 条」）；
  `run_sticker_choice_step` 跳过屏蔽段里的贴纸。⚠️ **只剔「这一趟生成 / 待精挑」的候选，不动已有事件**
  —— 屏蔽只做标记（`project/AGENTS.md` 第一条）。别以为「字幕已经按屏蔽过滤过」就够了：
  被滤掉的只是字幕，**事件本身还在**，照样会参与落点与预算。
- ⚠️ **文字元素 = 候补**（`segment.rs::build_events_system_prompt`，2026-10-19 用户口径）：填的是
  **关键词 / 短词组（2~8 字）**，不是整句；**不要与贴纸同时出现**（同一段时间二选一）；
  只在「这一段没有合适的贴纸/特效」或「其它效果已经用得比较密」时才写，**同一段最多 1 条**。
  `cap_text_elements` 在落盘前再兜一次（与 `process` 成对）。
- **节奏调度在落盘前也跑一遍**（§26，Step 4d）：与 `process` Step 1h 成对（`rhythm::plan/apply`，
  账本 `[rhythm] auto_events` 先删后建 ⇒ 幂等）；`calm` 档只清账本。`cmd_llm` 拿到 6 元组
  （末项 `Option<Vec<RhythmAuto>>`），`None` = 本趟没动节奏 ⇒ 不写 `[rhythm]`。
- **覆盖层提示词示例**：LLM 提示词里的示例长期只写 `{"overlay_type":"gradient","color":"#ff0000"}`，
  而四处求值器都只认 `colors` —— 实测 4 个项目 **8/8 覆盖层全都没画出来**。所以提示词示例现在给
  `colors` 数组 + 纯色两个写法（`llm/segment.rs`，另有单测锁住），且写了 `colors`/`color` 却漏掉
  `overlay_type` 时也会被推断成 overlay；老工程靠**读时兜底**，不用重跑 LLM。`fade_in`/`fade_out`（黑场淡入/淡出）**不进示例**：提示词里只有
  一句「系统只在初次后处理时在片头补、不要重复写」，片头那份见 `process/AGENTS.md` Step 3c（用户可手动改/删）。
- ⚠️ **prompt ↔ parser 契约有专门判据**（`llm/contract_tests.rs`，规格 §4 B4）：提示词里**每个字段/取值**
  各有一条「模型照写 → 过 `EventGenTask::parse_output` → 字段逐值还在」的用例，每条都注明了提示词出处；
  `needles` 还会断言那些字面量**仍在提示词里**。**改提示词的字段/取值、或动 `normalize_segments_json`
  的推断规则，必须同时改那张表**（否则判据与提示词脱节而全绿）。变异自证：删 `colors`/`tilt` 推断分支、
  删 `overlay_type` 补种、删 `parse_output` 里的 normalize 调用，四处都已实测变红。
- ⚠️ **类型推断必须认两种形状**（2026-10-19 实测）：`[{"events":[…]}]`（Step1 分段）**与平铺事件数组**
  （Step2 那条路 —— `parse_output` 拿到的就是它）。只认前者的话推断**一次都不会跑**：模型漏写 `type`
  ⇒ 整窗 `missing field type` ⇒ 那一段一条事件都没有，而日志只有一句解析错误。回归钉子：
  `contract_tests::every_documented_field_and_value_survives_the_parser` / `segment_shaped_payload_also_infers_types`。
- **老 checkpoint 必须过时间迁移**：里面的事件还是 `duration` 字段 → `EventGenTask::load_windows` 会先
  走 `crate::project::upgrade_event_times` 再解析，别绕过它。
- ⚠️ **冷开场/回忆回溯的意图写在 `anchor` 上**（`segment.rs::build_events_system_prompt` 那一段）：
  `anchor = "cold_open"` = 爆点提前播一次、`"recall"` = 爆点后面重提。**不是景别**，`process` 会收走它。
  配额在 `process` 侧（整片冷开场 1 条、回忆回溯 2 条）—— LLM 只是表达意图。
- ⚠️ **改内建 prompt 要同步用户配置**：`.yekitrim/config.toml` / `.yekitrim/llm.toml` 里的是
  `config init` 填进去的**副本**，改代码默认值对它无效；**每个程序目录各有一份**（debug / release 互不相干）。
- 用了贴纸包来源时 `run_llm_pipeline` 才拼 `local_sticker_candidates_text`（**只列已构建的包**，
  见 `assets/` 卡）；`has_local` 为假就整段不注入 —— 贴纸来源改了要重跑，别指望旧 checkpoint。

> 改动只允许落在本目录；跨模块改动先读对方卡，并跑两条门禁。

## 验证

```bash
cargo test -p yekitrim-polish --lib llm
```

含 `events.rs::tests`（空/失败/正常三条不覆盖回归）与 `status` 的 remap 断言。改了 prompt 或
守卫调用点后，必须在真实工程上重跑一次 `yekitrim polish llm`，确认日志里
「实际跳过 0 个窗口（内容匹配）」（说明指纹失效生效、没有拿回上一版结果）。
