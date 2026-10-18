# 精修系统重构方案（可执行规格 · 2026-10-19）

> 一句话：**创作归 LLM，process 只做资源热加载与应用**。本文件按批次给出改动点、判据、验收命令，
> 可以逐条勾选执行；每批结束都能独立回滚。
> 关联：`docs/plan/polish-deep-audit-plan.md`（体检与证据）、`docs/plan/rhythm-richness-plan.md`（节奏手法的词表来源）。

---

## 0. 总览

| 批次 | 目标 | 时长 | 能否独立上线 |
|---|---|---|---|
| **A 止血** | 停止 process 创作 + 幂等探针 + 预览埋点 | 0.5–1 天 | ✅ |
| **B 深检** | 审计器 / 像素探针 / ownership 断言 / LLM 契约 | 2–3 天 | ✅（只报告） |
| **C 去创作化** | 配额与派生逻辑迁出 process | 3–5 天 | ✅ |
| **D 编排轮** | LLM 第三段：全片节奏与结构 | 2–3 天 | ✅（可关） |

依赖：A → B → C → D；**D 依赖 `yekitrim-llm` 抽离完成**（当前那侧 19 条边界门禁已含 llm_is_a_leaf）。

---

## 1. 架构契约（硬边界）

```
LLM（三段）       process（两层）                render
分段 → 逐段事件             ① 资源热加载             ↓
     → 编排轮(全片)          ② 应用（写回/几何/clamp）  出片
```

**不许**：
- process 产生"创作性事件"（节奏、回位镜头、配额降级、删事件）；
- process 改 LLM 给的时间（只允许 clamp 到合法区间并**记一条 audit**）；
- LLM 做资源解析（贴纸 keyword→文件、SFX tag→文件仍归 process）。

**允许**（process 的应用层）：
写回 `origin/to_origin/x/y/source`、贴纸落点几何、文字排版、`end` 缺省补齐、音效规则解析、
资产下载、片头 `fade_in` 补种（判据 `intro_fade_seeded`）、EDL 意图落实（`materialize_edl_intents`）。

---

## 2. 数据契约

### 2.1 `Event.origin`（新增，唯一 schema 改动）

```rust
// render-core/src/types.rs —— 每个 Event 变体加一个字段
#[serde(default, skip_serializing_if = "Option::is_none")]
pub source: Option<String>,   // "llm:seg" | "llm:arrange" | "user" | "process"
```

- 老工程无此键 ⇒ `None` ⇒ 行为不变（R4）；
- 编辑器新建事件写 `"user"`；process 派生（片头 fade_in）写 `"process"`；
- **重跑清理按 `source` 而不是账本**：`retain(|e| e.source != Some("llm:arrange"))`。

### 2.2 ownership 表（谁写哪些字段）

| 字段 | 拥有者 | process 行为 |
|---|---|---|
| `time/end`（相机、特效、元素） | LLM / user | 只 clamp（`end ≤ seg.end`、|end−time| ≥ 0.3） |
| `zoom/anchor/anchor_to/cut/tilt/speed` | LLM / user | 只写回 `origin/to_origin`（**manual 不覆盖**） |
| `x/y`（贴纸） | process（几何求解） | 写；`manual_pos` 存在则不写 |
| `source`（文字排版结果） | process | 写 |
| `name/source`（贴纸解析） | process | 写 |
| `sfx/sfx_volume` | process（规则引擎） | 写 |
| `manual` | user | **只读** |

### 2.3 幂等定义

> 同一份输入（`polish.toml` + `data/` + 资源）连跑两次 `process`，
> 产出的 `polish.toml` **字段级 diff = 0**（`data/` 里除报告类文件外不新增/不改变）。

---

## 3. 批次 A：止血

### A1 节奏冻结（✅ 已完成）
- `RhythmConfig::profile_name` 空串 ⇒ `calm`；process 不再生成派生事件。
- 判据：`process::rhythm::tests::default_profile_is_frozen_to_calm`。

### A2 幂等探针（新增）
- 文件：`crates/yekitrim-polish/src/process/mod.rs` 的 `#[cfg(test)] mod tests`。
- 用例：`diagnose_process_idempotent`（`#[tokio::test]`，不 ignored，跑快）：
  临时工程 × 5 矩阵（默认 / 有 mute / 有 EDL / 有 manual 事件 / 有 rhythm 账本）→ 跑两遍 →
  读两次 `polish.toml` **反序列化成 `ProjectFile` 逐字段比较**（不要比字符串，格式会抖）。
- 判据：两次相等；不等时打印第一处差异字段路径。
- 附带：`diagnose_process_three_runs`（第 2、3 次相同）。

### A3 预览诊断埋点
- 文件：`webui/src/components/PolishPreview.vue`。
- 新增 `window.__dhampirDiag`，每帧更新：
  `{ t, lastPresentAt, activeSourceReadyState, busy, maskOn, frameNull, contextLost, canvasGen }`。
- `contextLost` 来自 `dhampirCanvas.addEventListener('webglcontextlost')`；
  `canvasGen` = canvas 元素身份计数（重挂即 +1，**用来抓 B1**）。
- 仅当 URL 带 `?diag=1` 时渲染右下角 HUD（4 行：present 延迟 / readyState / mask / canvasGen）。

### A4 canvas 不重挂断言
- `webui/scripts/check-polish-page-loads.mjs` 扩展：记录 `document.querySelector('#dhampir-canvas')` 引用，
  在切片段、任务完成、暂停/播放、拖时间轴之后比对 `__dhampirDiag.canvasGen`，必须恒为 1。
- 判据：`canvas 被重挂（gen > 1）⇒ 失败`，并打印触发动作。

---

## 4. 批次 B：深检（只报告，不阻断）

### B1 `process::audit`（新模块，纯函数）
```rust
pub struct Finding { pub kind: &'static str, pub at: f64, pub detail: String }
pub fn audit(cfg: &ProjectFile) -> Vec<Finding>;
```
检查项（对应 D3–D7）：
1. **相机重叠**：任意两条 `Camera` 的有效区间相交（前一条的 end 与后一条的 time 重叠 > 0.05s）；
2. **同刻多状态**：同一时刻存在 ≥2 条相机（按 time 排序后相邻同刻）；
3. **瞬时效过密**：相邻瞬时特效间隔 < 0.8s（`MIN_VISUAL_GAP_SECS`）；
4. **镜头总量**：`camera 数 / 片长 × 60 > 12`；
5. **元素越界**：`end > seg.end` 或 `time < seg.start`；
6. **跨静音元素**：元素窗口与 `mute_ranges` 相交（预警，不阻断）。

输出 `data/process-report.json`（`{ findings: [...], counts: {...} }`），**只在有 findings 时写**（R4）。

### B2 预览像素探针
`webui/scripts/check-polish-page-loads.mjs` 扩 5 场景（headless Chrome，无则 SKIP）：
选片段 / 拖时间轴 / 跳屏蔽 / resize 窗口 / 全屏。
每 100ms 读 `canvas.getContext('2d')`？—— 底座是 WebGL，改用 `page.screenshot()` 取平均亮度（阈值 > 2/255）。
判据：**连续黑帧 < 200ms**；输出每场景「黑帧数 / 最长黑帧 ms」。

### B3 ownership 断言（debug only）
`#[cfg(debug_assertions)]`：每次 `cmd_process` 结束前对比"运行前的 manual 事件字段快照"与运行后，
任何 `manual` 事件的 `time/end/origin/x/y` 变化 ⇒ `panic!`（测试里直接红）。

### B4 LLM 契约测试
`crates/yekitrim-polish/src/llm/segment.rs` 的 prompt ↔ `events.rs` 的 parser：
对 prompt 里出现的**每个字段/取值**构造一条事件并解析，断言不丢字段（effect / color / anchor / cut / tilt /
speed / end / text / pos / kind）。已有 5 条，补齐到覆盖全量。

---

## 5. 批次 C：规则去创作化

### C1 守卫去向（按此表改）

| 守卫 | 改动 |
|---|---|
| `cap_events` | **改为 `verify_budget`**：不删，超限写 audit + 日志；P3 后由编排轮保证 |
| `cap_pip_shots` | 同上（不降级，只报告） |
| `cap_dutch_angle` | 保留 `tilt` 的 ±6° clamp（合法性），**删除"全片 2 条"的降级**，改 audit |
| `cap_text_elements` | 删除（候补定位写进 prompt + audit 预警） |
| `cap_jump_cuts` | 改为 audit；`cut` 合法性（必须在接缝/句末）保留 |
| `align_stickers_to_subtitles` | **降级**：只 clamp 到 `[seg.start, seg.end]`，不吸附时间 |
| `fit_element_span` | **降级**：只保证 `end−time ≥ 0.3` 与 `≤ 8s`，不吞字幕 |
| `cap_handheld_sway` | 保留（能力限制） |
| `materialize_edl_intents` | 保留（应用） |
| `rhythm::plan/apply` | **删除**；保留 `clear` 与手法词表（`Preset` 枚举 + `label`）给 prompt 用 |

### C2 顺序依赖修正
- `cap_dutch_angle` 与 `rhythm` 的先后不再重要（rhythm 已删）；把"合法 clamp"类统一提到 **Step 1 末尾**，
  "应用"类（锚点/落点/排版）留在原位。

### C3 `resolve_camera_anchors` / `layout_text_elements` 尊重 manual
- `resolve_camera_anchors`：`Event::Camera { manual: true, .. }` ⇒ **跳过 origin 覆盖**（只解析 `to_origin`？也不解析）。
- `layout_text_elements`：文字事件若带 `manual`（或 `manual_pos`）⇒ 跳过排版写回。
- 判据：`manual_event_fields_survive_process`（覆盖 time/end/origin/x/y 五项）。

### C4 稳定抽样（过渡期）
若 P3 前仍需 `cap_events`：把"均匀索引"换成 `(kind, time_ms, hash) 升序取前 N`，保证加一条贴纸不换一批。

---

## 6. 批次 D：LLM 编排轮

### D1 接口
```rust
// crates/yekitrim-polish/src/llm/arrange.rs（新）
pub struct ArrangeInput {
    pub duration: f64,
    pub segments: Vec<SegSummary>,      // id, start, end, role, fx, 事件计数
    pub punches: Vec<(f64, f64)>,       // 爆点
    pub existing: Vec<EventSummary>,    // 已生成事件的摘要（kind/time/end/zoom）
    pub quotas: Quotas,                 // 见 D2
}
pub struct ArrangeOutput {
    pub structure: Vec<StructureMark>,  // hook / climax / calm / outro（时间点）
    pub events: Vec<Event>,             // 直接产事件，source = "llm:arrange"
    pub notes: Vec<String>,
}
```

### D2 配额表（写进 prompt + parser 双保险）
| 手法 | 整片上限 | 最小间隔 |
|---|---|---|
| 呼吸（推近→回位） | 1 | — |
| 硬切 / 重击 | `ceil(duration/20)` | 8s |
| 慢推 | 3 | 6s |
| 甩视线 | 2 | 10s |
| 歪镜 | 2（且累计 ≤2） | 15s |
| 画中画（远） | 1 | — |
| 任意相机事件 | 12/min | 2.5s |
| 瞬时特效 | 4/min | 0.8s |

### D3 失败与降级
- 编排轮失败 / JSON 不合法 / 超配额 ⇒ **丢弃本趟编排**，只保留逐段事件；任务日志一行：
  `⚠ 编排轮未生效（原因）：本片按逐段事件出片`。
- 开关：`[polish.llm] arrange = true`（默认 true）、`arrange_model` 可选单独模型。
- checkpoint：`id = arrange_{duration}_{punch count}@fingerprint`。

### D4 接线
- `cmd_llm`：分段 → 逐段事件 → **arrange** → 写盘（一次 `update_polish_toml`）。
- **删除** `llm/mod.rs` 的 Step 4b/4d 派生逻辑（节奏/配额降级/回位镜头），只保留"落盘前 clamp"。
- `process` 不再需要 `[rhythm]` 表：**删配置段**（老工程残留键由 `deny_unknown_fields` 前置的
  `upgrade_toml_value` 忽略/或加一条迁移删除）。

### D5 判据
- `arrange_emits_only_declared_presets`（任何 preset 都必须来自词表）；
- `arrange_respects_quotas`（构造超配额输出 ⇒ 被裁）；
- `arrange_failure_keeps_segment_events`；
- `arrange_is_idempotent`（同输入两次同输出，或直接跳过缓存）。

---

## 7. 预览黑屏修复细则（B1–B5）

| # | 修法 | 验收 |
|---|---|---|
| B1 | **不重挂**：`#dhampir-canvas` 用 `v-show` 而不是 `v-if`；换片段只换绑源，不重建 DOM | A4 断言 canvasGen 恒 1 |
| B2 | **播放中禁止改 `canvas.width`**：resize 走 `ResizeObserver` → 若 `playing` 则先标记 `pendingResize`，暂停时应用；或固定 backing store 尺寸 + CSS 缩放 | 像素探针 resize 场景 0 黑帧 |
| B3 | **遮罩只在拍到有效帧时启用**：`snapshotStageFreeze` 采样画布中心 3×3，平均亮度 < 2/255 ⇒ 返回 false（不启用遮罩），记录 `maskSkipped` | HUD 显示 maskSkipped 次数；黑帧探针 |
| B4 | **换台前等就绪**：A/B 预滚要求目标 `readyState ≥ 2`（已有）+ 换台时若 busy，保留旧源继续呈现到新源首帧（不撤元素） | 快速拖动 10 次无黑帧 |
| B5 | **null 帧兜底绘制**：`dhampirFrameAt(t) === null` 时用最近的上一帧渲染（`lastGoodSourceTime`），并同时触发 `dhampirJumpTarget`；连续 null > 300ms ⇒ 强制定位到最近可播点 | 播放头落在屏蔽段内不黑 |

---

## 8. 验收矩阵

| # | 命令 / 动作 | 通过标准 |
|---|---|---|
| V1 | `cargo test -p yekitrim-polish --lib` | 全绿 |
| V2 | `cargo test -p yekitrim-polish --lib diagnose_process_idempotent` | 两跑 diff = 0 |
| V3 | 连跑 3 次 `polish process`（真实片段） | `git diff polish.toml` 为空 |
| V4 | `cargo check --workspace --all-targets` | 0 warnings |
| V5 | `node webui/scripts/check-preview-ab.mjs` | 全绿 |
| V6 | `node scripts/check-polish-page-loads.mjs` | 5 场景黑帧 < 200ms、canvasGen = 1 |
| V7 | `python3 scripts/check_module_map.py && python3 scripts/check_module_boundaries.py` | 全绿 |
| V8 | `data/process-report.json` | 相机重叠 = 0、瞬时效 ≤ 4/min、镜头 ≤ 12/min |

---

## 9. 迁移与回滚

- **A**：纯新增（探针/断言），回滚 = 删文件。
- **B**：只报告；回滚 = 不跑。
- **C**：按条目独立；每条都保留旧函数到 D 完成后再删（`#[deprecated]` 标记）。
- **D**：`[polish.llm] arrange = false` 立即回到"只有逐段事件"；`[rhythm]` 表在 D 完成后删。
- **老工程兼容**：`Event.source` 缺省 `None`；`[rhythm]` 残留键在 D 后写一条 `upgrade_toml_value` 清理。

---

## 10. 任务清单（按依赖排序，可直接勾选）

- [x] A1 节奏冻结（默认 calm）+ 判据
- [ ] A2 `diagnose_process_idempotent` + 三跑收敛
- [ ] A3 `__dhampirDiag` 埋点 + HUD
- [ ] A4 canvasGen 断言（脚本）
- [ ] B1 `process::audit` + `data/process-report.json`
- [ ] B2 像素探针 5 场景
- [ ] B3 debug ownership 断言
- [ ] B4 LLM prompt↔parser 契约补全
- [ ] C1 `cap_*` 改校验/降级（按表）
- [ ] C2 Step 顺序整理（clamp 前置）
- [ ] C3 manual 字段不被覆盖（5 项）
- [ ] C4 稳定抽样（若保留）
- [ ] C5 删 `rhythm::plan/apply`，留词表
- [ ] D1 `llm/arrange.rs` 接口 + schema
- [ ] D2 配额表 + prompt
- [ ] D3 parser 校验 + 降级
- [ ] D4 接线 cmd_llm + 删 Step 4b/4d 派生
- [ ] D5 编排轮判据 5 条
- [ ] E  删 `[rhythm]` 配置段 + 迁移清理

---

## 11. 风险

| 风险 | 缓解 |
|---|---|
| 编排轮 token 成本 | 只喂摘要；单片一次；失败不重试（降级） |
| LLM 不听话（超配额） | parser 裁剪 + audit 报告，不静默 |
| 去掉 process 配额后片子变乱 | C 批次先"只报告"，攒一轮真实片段数据再删 |
| 预览改动引入新黑屏 | A3 埋点先行；每次改动跑 V6 |
| `Event.source` 触及 R4 | 缺省 `None`、不写键；roundtrip 测试 |
