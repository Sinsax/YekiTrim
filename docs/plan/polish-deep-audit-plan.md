# 精修系统深度体检与修复计划（2026-10-19）

> 范围：**只覆盖精修**（`crates/yekitrim-polish` + 精修页前端），不含预处理/切片。
> 四条线：① LLM 精修链路 ② 后处理循环（幂等）③ 后处理规则（ownership 与分布）④ 底座预览加载变黑。
> 现状：节奏调度已上线但**默认开着**，是当前"跑一次变一次 / 镜头重叠 / 抖动滥用"的主要来源。

---

## 0. 结论（一句话）

精修现在缺的不是功能，是**三张契约**：
- **幂等契约**：`process` 必须"同输入 → 同输出"，今天不是（规划跑在清理之前，自己产出的抖动会改变下一趟的格点）；
- **ownership 契约**：每个 Step 到底"拥有"哪些字段没有声明，于是 `resolve_camera_anchors` 每趟覆盖 `origin`、`layout_text_elements` 每趟重排文字、`cap_text_elements` 每趟删文字；
- **画面存活契约**：预览没有"最后一次成功上屏是什么时候"的概念，黑屏只能靠肉眼发现。

修完这三张契约，节奏、抖动、重叠这些问题才有地方**被拦住**，而不是靠调参数。

---

## 1. 体检证据（代码级）

### 1.1 底座预览「时常变黑」——五条已知机制

| # | 机制 | 证据 | 触发条件 |
|---|---|---|---|
| B1 | **canvas 被重挂 ⇒ 永久黑**（底座 attach 只接一次、没有 detach） | `webui/src/components/AGENTS.md` ②.7 明写；换片段/任务完成**不许重挂** | 组件卸载重挂、F5 前任何 `v-if` 切换 |
| B2 | **改 `canvas.width` 会换掉交换链**，旧画面立刻黑（实测亮度 219 → 0.18） | `PolishPreview.vue:1376-1378` 只把"改尺寸"挪到最后，**没禁止播放中改** | 窗口 resize / 全屏切换 / 舞台布局变化 |
| B3 | **定格遮罩可能拍到黑帧**：`snapshotStageFreeze` 直接 `drawImage(dhampirCanvas)` | `PolishPreview.vue:3022-3055`；失败时退到"底色 + 视频元素"，此时正在 seek ⇒ **底色（黑）**；遮罩最长 700ms（`SKIP_FREEZE_MAX_MS`） | 进屏蔽区、连续屏蔽、刚 resize 完 |
| B4 | **源就绪闸门不够**：A/B 换台/预滚要求 `readyState ≥ 2`，但换台中间态与 `dhampirBusy` 撞上时会走"撤掉重建"，重建期间画面无人绘制 | `PolishPreview.vue:1382-1397` | 快速拖动、连续换片段、跳屏蔽 |
| B5 | **帧映射为 null**：`dhampirFrameAt(t) === null`（落在被裁掉的源区间）时没有兜底绘制 | 同文件 `dhampirFrameAt` / `dhampirJumpTarget` | 播放头停在屏蔽段内、EDL 有空洞 |

> 结论：黑屏不是单一 bug，是 **B1 永久 / B2–B5 瞬时** 两类。B1 只能靠"不重挂"保证；B2–B5 需要**探针**才能定位到底是哪条。

### 1.2 后处理「每跑一次都变」——四条根因

| # | 根因 | 证据 |
|---|---|---|
| C1 | **先规划、后清理**：`plan` 看得到上一趟自己生成的 shake/flash（`blocked_spans` 把它们当障碍）⇒ 格点挪位 ⇒ 下一趟再挪 | `process/mod.rs:481-482`；`rhythm.rs:261` |
| C2 | **已有运镜不算占位**：`blocked_spans` 只把 `manual` 运镜当障碍 ⇒ 派生运镜与 LLM 运镜在时间上叠着 | `rhythm.rs:261-280` |
| C3 | **账本字段不全**：没记 `speed` / `anchor_to`，而后续步骤会改 `tilt`（`cap_dutch_angle`）、`origin`（2d）⇒ 认不出 ⇒ 不删 ⇒ 叠加 | `RhythmAuto`（`project/config.rs`）；`rhythm.rs::matches_auto` |
| C4 | **无瞬时特效总量上限**：轻档每一下都抖，空档补位保证 ≤5s 一个 ⇒ 60s 里 12+ 个 shake | `rhythm.rs:788`；`cap_events` 只覆盖贴纸/音效 |

### 1.3 规则互相踩（ownership 未声明）

| # | 现象 | 证据 |
|---|---|---|
| R1 | **运镜 `origin` 每趟被覆盖**，且**不检查 `manual`** —— 用户手改的落点会丢 | `process/mod.rs:2149-2188` |
| R2 | **文字元素每趟重排**（`layout_text_elements` 直接写回 `source`），没有"用户拖过就别动"的判据（贴纸有 `manual_pos`，文字没有对应豁免） | `process/mod.rs:2020-2080` |
| R3 | **顺序风险**：`cap_dutch_angle`（Step 1d2）排在节奏（1h）**之前** ⇒ 本趟加的歪镜绕过它的 ±6°/全片 2 条配额 | `mod.rs:307` vs `:468` |
| R4 | **抽样不稳定**：`cap_events` 用"均匀索引"保留，加一条贴纸就换一批被留下的 | `mod.rs::cap_events` |

### 1.4 LLM 链路耦合

| # | 现象 | 证据 |
|---|---|---|
| L1 | **节奏在 LLM 落盘前也跑**（Step 4d）⇒ 派生事件混进 LLM 产物、账本被两处写 | `llm/mod.rs:277-305` |
| L2 | `full=true` 重跑**会重写全部事件（含用户手放）** —— 已有二次确认，但 API 默认值仍是 `true` | 卡片「入口语义」 |
| L3 | prompt 是"代码默认 + 用户配置副本"，改代码对老配置无效（历史上多次踩） | 根卡「改内建 prompt 要同步用户配置」 |

---

## 2. 修复计划（四阶段）

### P0 冻结与止血（0.5–1 天，先让系统回到可观察状态）

1. **节奏默认降为 `calm`**（`RhythmConfig::profile_name` 空串 → calm；`cmd_init` 新建工程才写 `balanced`）。理由：当前问题全部由它引入，先停掉再逐个修。
2. **先清后算**：新增 `rhythm::rebuild(segments, params, punches, mutes, old_ledger)` = 清账本 → plan → 插；两处调用点（process 1h、llm 4d）改成一行。
3. **窗口级冲突检测**：插入前比较 `[start,end]`（不是时刻），与任何已有/已计划事件相交就跳过该重音；强制 `0.8s ≤ 窗口 ≤ 4s`。
4. **抖动预算**：`Punch` 受 `punch_max`（默认 2/clip）+ 与任何瞬时特效 ≥1.2s 不重叠；新增 `transient_max_per_min = 4`。
5. **预览黑屏埋点**：`window.__dhampirDiag = { lastPresentAt, readyState, maskOn, contextLost, frameNull }`，并在页面右下角加一个 `?diag=1` 才显示的 HUD；先做到"黑一下就知道是哪条"。

### P1 深检基建（2–3 天，先造尺子再改规则）

6. **幂等探针**：`cargo test -p yekitrim-polish --lib diagnose_process_idempotent`（ignored）——临时工程跑两遍 `cmd_process`，对 `polish.toml` 做**字段级 diff**，要求 0 差异；矩阵：默认 / 有屏蔽 / 有 EDL / 有 manual 事件 / rhythm 各一例。
7. **重叠审计器** `process::audit`（纯函数）：运镜时间重叠、同刻多个运镜状态、瞬时特效 <0.8s 相邻、贴纸/文字互压、元素跨静音；输出 `data/process-report.json` + 单测。
8. **预览像素探针**：扩 `scripts/check-polish-page-loads.mjs`（headless Chrome，已有）：选片段 → 播放 3s → 每 100ms 采样 canvas 平均亮度 → 断言**连续黑帧 < 200ms**；场景：切片段 / 拖时间轴 / 跳屏蔽 / 窗口 resize / 全屏。
9. **LLM 契约测试**：prompt↔parser 全字段（effect/color/anchor/cut/tilt/speed/end），空输出、限流、单段重做、`full` 是否动 manual、checkpoint 指纹。

### P2 规则重写（3–5 天）

10. **ownership 表**：每个 Step 声明 `owns: [字段]` / `respects: [manual 等]`，debug 构建里断言（R1/R2 直接按表修）。
11. **`cap_events` 稳定抽样**：用 `(事件类型, time 量化, 哈希)` 排序代替索引均匀，加一条不改变已保留集合。
12. **节奏重做成"事件驱动"**：只在**爆点 + 长段**给手法，取消全片网格补位（治"刻意"）；保留 8 个手法但每次只挑 2–3 个。
13. **叠化/转场**（可选）：只在话题转折处。

### P3 LLM 分层（2–3 天，配合你正在做的整合/分离）

14. **LLM 只产出"内容事件 + 意图"**：删 `llm` Step 4b/4d 的派生逻辑，派生事件（节奏/回位/收工镜头）**只由 process 产生**——顺带消掉"两个重复实现要一起改"这类历史坑。
15. **prompt 版本化**：`config init` 写入的副本带版本号，代码默认变更时给出"可一键合并"的提示，而不是静默无效。
16. **`full` 语义拆分**：`full=true` 只重做 LLM 自己的事件；manual 事件永远保留（现在靠二次确认，改成硬保证）。

---

## 3. 深度检查清单（Deep Check）

| # | 检查项 | 判据 | 工具 |
|---|---|---|---|
| D1 | process 幂等 | 两跑 diff = 0 | `diagnose_process_idempotent` |
| D2 | 三跑收敛 | 第 2、3 次输出相同 | 同上（跑三次） |
| D3 | 运镜重叠 | 任意两条状态区间不相交（同一时刻只有一条有效） | `process::audit` |
| D4 | 瞬时密度 | ≤ `transient_max_per_min` | audit + rhythm 报告 |
| D5 | 镜头总量 | 60s 片段 ≤ 12 条运镜 | audit |
| D6 | 手改不被覆盖 | `manual` 事件的 time/end/origin/x/y 逐值不变 | 单测（已有 `cap_events_never_removes_manual_events` 扩展） |
| D7 | 屏蔽只标不删 | 事件数前后一致 | `mute_range_keeps_events` |
| D8 | 预览无黑帧 | 5 场景 × 连续黑帧 < 200ms | `check-polish-page-loads.mjs` |
| D9 | canvas 不重挂 | 同一页面生命周期内 `id=dhampir-canvas` 元素引用不变 | 页面探针断言 |
| D10 | LLM 契约 | prompt 教的字段 parser 全认 | `llm` 单测 |
| D11 | 空/失败不覆盖 | 空输出保留原事件 | `events.rs::tests` |
| D12 | 账本自愈 | 被后续步骤改过字段的派生事件能被正确清理 | 单测 |

---

## 4. 验收（每个阶段结束时跑）

```bash
cargo test -p yekitrim-polish --lib            # 全量
cargo test -p yekitrim-polish --lib diagnose_process_idempotent -- --ignored --nocapture
cargo check --workspace --all-targets          # 0 warnings
node webui/scripts/check-preview-ab.mjs        # 跳屏蔽
node scripts/check-polish-page-loads.mjs       # 预览黑帧（无 Chrome 则 SKIP）
python3 scripts/check_module_map.py && python3 scripts/check_module_boundaries.py
```

**真实片段验收**：同一片段连跑 3 次 `polish process`，`git diff` 对 `polish.toml` 应为空；播放 60s 视频记录黑帧数与最长黑帧。

---

## 5. 回滚与开关

- 节奏：`[rhythm] profile = "calm"` 即回到今天之前的行为（P0 之后默认就是它）。
- 审计器：P1 只报告不阻断（写 `data/process-report.json`），P2 才把"重叠 > 0"变成失败。
- 预览探针：无 Chrome / 无服务时 SKIP（不伪装通过）。

---

## 6. 需要拍板

1. **节奏先下线（calm）还是保留但改成事件驱动？** 我建议 P0 先 calm，P2 再上事件驱动版。
2. **黑屏优先修哪条？** 我建议先做 P0-5 埋点 + D9（canvas 不重挂断言），因为 B1 是"永久黑"、其余是"闪一下"。
3. **P3 的"LLM 只产意图"会不会和你正在做的 LLM 整合冲突？** 如果冲突，P3 只做 `full` 语义与 prompt 版本化，不动链路结构。

---

## 7. 拍板与架构 v2（2026-10-19）

### 三条决定

1. **节奏 = 事件驱动**：取消「全片网格补位」，只在爆点与长段给手法。
2. **预览黑屏按推荐顺序**：先埋点 + canvas 不重挂断言（D9），再上像素探针。
3. **创作全部归 LLM**：事件、意图、整体剪辑/安排都由 LLM 产出；**可能再加一轮「编排轮」**；
   **process 只做资源热加载与应用**（下载/解析贴纸与音效、写回落点/排版/锚点、clamp）。

### 由此产生的硬边界

- **process 不再产生创作性事件**：`process::rhythm` 的 `plan/apply` 迁到 LLM 编排轮，process 侧只留
  `clear`（清理历史派生事件）与手法**词表**（给 prompt 与 parser 校验用）。
- **`[rhythm] auto_events` 账本随派生事件一起退役**：所有事件都带来源（`llm:seg` / `llm:arrange` / `manual`），
  重跑时按来源清理即可，不再需要第二份账本。
- **配额从 process 前移到 LLM**：LLM 编排轮看得到全片，配额由它在生成时遵守；process 只**校验并在超限时报错**，
  不再偷偷抽稀/降级。

### 守卫的去向（P2 的核心表）

| 守卫/步骤 | 今天 | 新归属 |
|---|---|---|
| `cap_events`（贴纸/sfx 总量） | process 抽稀 | **LLM 编排轮**（生成时遵守）＋ process 只校验 |
| `cap_pip_shots` | process 降级 | LLM 编排轮 |
| `cap_dutch_angle` | process 剔除 | LLM 编排轮（±6° 仍由类型/clamp 兜底） |
| `cap_text_elements` | process 删文字 | LLM 编排轮（prompt 已写「候补」） |
| `cap_jump_cuts` | process 降级 | LLM 编排轮 |
| `align_stickers_to_subtitles` | process 吸附时间 | **降级为 clamp**：只保证不越界，不改 LLM 给的时间 |
| `fit_element_span` | process 吞字幕延长 | 同上：只保护最短显示，不改时间 |
| `cap_handheld_sway` | process 剔除 | **保留**（能力/几何限制，不是创作） |
| `materialize_edl_intents` | process | **保留**（把意图落成 EDL = 应用） |
| `resolve_camera_anchors` / `layout_text_elements` / `placement` | process 每趟重算 | **保留**，但必须遵守 ownership（`manual` 不覆盖） |
| 片头 `fade_in` / 音效规则 | process | 保留（应用） |
| `rhythm::plan/apply` | process | **迁出**：LLM 编排轮 |

### 编排轮（新增，LLM 第三段）设计要点

- **一镜到底、单片一次**：输入 = 段表摘要 + 爆点表 + 字幕/弹幕摘要 + 既有事件摘要（不塞全文，控 token）；
  输出 = 全片结构（hook / body / climax / 收尾）+ 手法表（哪一段用哪个手法、强度、时间）+ 转场；
- **输出即事件**：直接出 `camera/shake/flash/overlay` 等事件，带 `origin = "llm:arrange"`；
- **配额在这里兜**：每分钟运镜数、瞬时效数、画中画/歪镜/呼吸的整片次数，全部由 prompt + parser 双保险；
- **失败降级**：编排轮失败 ⇒ 只保留逐段事件（片子平淡但不崩），并在任务日志里明说；
- **可关**：`[polish.llm] arrange = true|false`（默认开，失败自动降级）。

### 实施顺序（按新架构调整）

| 阶段 | 内容 | 影响面 |
|---|---|---|
| **P0（现在）** | 节奏冻结（默认 `calm`，已落地）· 幂等探针 · 黑屏埋点 + D9 | 只动 process / webui |
| **P1** | 审计器（重叠/密度）· ownership 表 · 预览像素探针 | process + webui |
| **P2** | 守卫去创作化（按上表迁移/降级）· `Event.origin` 来源标记 · 幂等矩阵全绿 | process + render-core |
| **P3** | LLM 编排轮（新第三段）· process 只留资源与应用 · 删 `rhythm::plan/apply` | polish/llm + yekitrim-llm |

⚠️ P3 依赖你正在做的 `yekitrim-llm` 抽 crate（当前 `check_module_map.py` 还有 4 条红：新 crate 未登记卡/
`core-llm` 卡随目录搬走）。在它落定之前，P0–P2 不碰 `crates/yekitrim-llm`。
