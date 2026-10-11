# 重验 A：批 1-2 八项三列核实（代码 · 后端接线 · 前端）

> 立档：2026-10。方法：task-20。**只读审计，未改动任何代码。**
>
> 上游：`polish-upgrade-consolidated.md` 的 25 项总表。那张表只用**单列 ✅**，判据是
> 「grep 有命中 **且** 接线到渲染/转译/预览任一条腿」。
> ⚠️ **这个判据掩盖了真实状态** —— 已实测出反例：`§18-A 特写景别` 被标 ✅，
> 但 LLM 提示词给 LLM 的 zoom 选项只有 `"远"/"中"/"近"`，**没有「特写」**
> ⇒ **有消费者、没有生产者** ⇒ 产品里永远看不见。
>
> 本文按**三列**重验：
>
> | 列 | 含义 | 判据 |
> |---|---|---|
> | **① 代码** | 实现存在 | 文件:行号 + 函数名 |
> | **② 后端接线** | **生产者 → 消费者**全程走通 | 指名生产者是谁、消费者是谁；**特别查"有没有生产者"** |
> | **③ 前端** | WebUI 有控件/展示 **且** 预览支持 | `grep -rn` 命中数 + 人工判真伪 |
>
> ⚠️ **两条纪律**（继承自总表）：
> ① `grep 命中` **必须人工判真伪** —— 本文逐条列了剔除的假阳性；
> ② **"教了 LLM" ≠ "会产出"** —— 提示词有示例 ≠ 该值一定从 LLM 出来，反之亦然。两者分开写。

---

## 0. 结论速览

| § | 名称 | ① 代码 | ② 后端接线 | ③ 前端 | 断在哪 |
|---|---|---|---|---|---|
| §1 | 决策理由 `reason` | ✅ | ✅ | ✅ | 无断点（最小形态；规格 §5.1 的审批流未做，**已声明**） |
| §2 | 句末边界守卫 | ✅ | ✅ | ✅（经 §1 面板） | 无断点 |
| §3 | 剪辑质量分 | ✅ | ✅ | ✅ 控件 / ⚠️ 预览 | 预览不涉及（质量分是切片级徽标，非时间轴效果）—— 见 §3 说明 |
| §4 | 转译报告可见 | ✅ | ✅ | ✅ | 无断点 |
| §16 | 运镜速度 `speed` | ✅ | ✅ | ✅ | 无断点（LLM 有示例；前端有下拉） |
| §17 | 跳切手法 | ✅ | ✅ | ⚠️ | **前端只有文字说明，没有专门的跳切可视化/开关** |
| §20 | 真黑白噪点 | ✅ | ⚠️ | ❌ | **WebUI 活预览仍是纯白薄纱**（`background:#fff`），与成片分叉 |
| §22 | 音效淡入 | ✅ | ✅ | ⚠️ | 预览有淡入但**时长不同**（~30ms vs 0.2s），属"同类不同参数" |

**净结果：8 项里 5 项三列全绿（§1/§2/§4/§16 + §3 带说明），1 项前端缺（§17），
1 项前端分叉（§20，**本次最重要的发现**），1 项参数不一致（§22）。**

---

## §1 决策理由 `reason`

### ① 代码 ✅
- **类型**：`crates/yekitrim-polish/src/project/config.rs:167` `struct Decision`
  （`kind` 稳定串 + `at`/`start`/`end` + `text` 人话 + `detail`）；
- **落盘字段**：`config.rs:140` `pub decisions: Vec<Decision>`
  （`#[serde(default, skip_serializing_if = "Vec::is_empty")]` ⇒ 空则不写键，老工程零回归）；
- **类别常量**：`crates/yekitrim-polish/src/process/fx.rs:746` `pub mod decision_kind`
  —— **7 个**：`boundary_sentence_end` / `boundary_already_clean` / `boundary_kept_as_is` /
  `camera_default_added` / `pip_demoted` / `element_span_fitted` / `intro_fade_seeded`；
- **构造器**：`config.rs` 的 `Decision::at` / `Decision::span` / `Decision::whole`；
- **去重**：`process/fx.rs:789` `dedup_decisions`（按 `(kind, at)` 量化到毫秒）。

### ② 后端接线 ✅ —— **生产者存在，且不止一个**

⚠️ 这一项**特意先查"有没有生产者"**（「特写」那个坑的反面教材）。

| 生产者 | 位置 | 产出 |
|---|---|---|
| 边界挪动 | `process/mod.rs:124` `decisions.extend(boundary_moves.iter().map(boundary_move_decision))` | `boundary_sentence_end` |
| 画中画降级 | `process/mod.rs:179` `Decision::whole(decision_kind::PIP_DEMOTED, …)` | `pip_demoted` |
| 片头淡入补种 | `process/mod.rs:861` `Decision::whole(decision_kind::INTRO_FADE_SEEDED, …)` | `intro_fade_seeded` |

- **写回**：`process/mod.rs:870` `config.decisions = decisions;` ⇒ 随 `update_polish_toml` 落盘；
- **顺延规则**：`process/mod.rs:118-125` 的 `CARRY_OVER`（只接边界类 + 片头类；
  次数型如画中画降级**不顺延**，因为"上趟降了几条"已过时）；
- **消费者**：`crates/yekitrim-server/src/api/polish.rs:3112` 的测试
  `polish_project_http_returns_decisions` —— **这条是 HTTP 级判据**，注释写明了理由：
  「`serde_json::to_value(&cfg)` 里**自动**带着 `decisions`」⇒ 必须钉 HTTP 回包，
  否则"理由面板会永远空"而 Rust 侧测试全绿。

### ③ 前端 ✅
```
grep -rn "decisions" webui/src/   -> 4 命中
```
| 文件:行 | 真伪 |
|---|---|
| `components/TimelineEditor.vue:611` `const decisions = ref([])` | ✅ 真 |
| `components/TimelineEditor.vue:976-988` `segDecisions`（按段过滤，无 `at` 挂首段） | ✅ 真 |
| `components/TimelineEditor.vue:2246` `decisions.value = … cfg?.decisions : []` | ✅ 真 |
| `components/TimelineEditor.vue:218` 模板 `v-if="segDecisions.length"` → `why-row` | ✅ 真 |

```
grep -rn "reason" webui/src/   -> 14 命中
```
⚠️ **人工判真伪 —— 13 个是假阳性**（总表已点名这个坑，本文复核确认）：
- `TimelineEditor.vue:203` / `:1030` —— **LLM 重做失败状态**（`gen.reason`），不是决策理由；
- `TimelineEditor.vue:1246` `commitEdit(reason, …)` / `:1249` `payload.reason` —— 提交说明；
- `PolishView.vue:1280/1298/1312/1367` —— `processPendingReason`（后处理排队原因）；
- `api/ingest.js:48`、`WorkbenchView.vue:347/476/483` —— 入库跳过原因 / 录播姬配置。

⇒ **`reason` 这个搜索词在本项上是错的**（决策理由的字段名是 `decisions`）。
这条正是总表 §1 那句「`reason` 命中的是 LLM 重做状态」的复现。

### 结论
**① ✅ ② ✅ ③ ✅** —— 不断在任何一处。⚠️ 但**范围**要写清：
规格 §5.1 的 `[editing]` 三表与审批流**未做**（`config.rs:145-149` 的注释**主动声明了**这一点：
"不是规格 §5.1 那套…先落最小形态"）。这是**有意的降级**，不是缺口。

---

## §2 句末边界守卫

### ① 代码 ✅
- **判据**：`crates/yekitrim-polish/src/process/fx.rs:534` `pub fn is_sentence_end(text: &str) -> bool`；
- **主逻辑**：`fx.rs:596` `align_segment_boundaries_detailed(...) -> Vec<BoundaryMove>`
  （薄封装 `fx.rs:585` `align_segment_boundaries` 只回条数）；
- **兜底候选**：`fx.rs:648` `sentence_end_aligned` / `fx.rs:701` 那条 `.filter(|s| is_sentence_end(&s.text))`
  —— 真实字幕 **0% 句末标点**时的退路（退到"最近一条字幕的结束处"）。

### ② 后端接线 ✅ —— **两处成对**（这是本项的关键）
| 消费者 | 位置 | 时机 |
|---|---|---|
| `process` Step 0b | `process/mod.rs:95` | **每趟后处理都收敛** |
| `llm` Step 4b | `llm/mod.rs:192` | **刚生成就对齐** |

代码注释（`process/mod.rs:91-93`）明写了为什么必须两处：
「与 `llm/mod.rs` Step 4b 是**配对的两处**（老工程只有 process 这条腿能重跑）——
同 `cap_pip_shots` 那条先例：LLM 那边管"刚生成就对齐"，这里管"每次后处理都收敛"」。
⇒ **生产者 = 两处调用点；消费者 = `config.segments[]` 的边界**（被就地改写）。

- **副作用接线**：挪动明细经 `boundary_move_decision`（`fx.rs:766`）变成 §1 的决策理由。

### ③ 前端 ✅（经 §1 的面板，无独立控件）
```
grep -rni "句末|sentence" webui/src/   -> 1 命中
```
- `components/TimelineEditor.vue:224` `:class="d.kind === 'boundary_sentence_end' ? 'b' : 'o'"`
  —— ✅ 真：**面板按 `kind` 给边界类决策单独着色**。

⚠️ 本项**没有独立的用户控件**，这是**架构上正确的**：句末守卫是一条**自动**收敛规则
（用户不该手动触发"把切点挪到句末"）。它的**可观测面**就是 §1 的理由面板。
⇒ 不构成"前端缺失"，但要在总表里注明"前端可见性依附于 §1"。

### 结论
**① ✅ ② ✅ ③ ✅**（经 §1 面板）—— 无断点。

---

## §3 剪辑质量分

### ① 代码 ✅
- `crates/yekitrim-polish/src/process/quality.rs`（**纯函数模块**）：
  - `quality.rs:169` `pub fn evaluate(segments, subtitles, duration) -> Quality`；
  - `quality.rs:89` `struct Quality`，方法 `score()`(:138) / `smoothness()`(:115) /
    `interruptions_per_minute()`(:124) / `by_kind()`(:144) / `interruption_count()`(:105)；
  - `quality.rs:81` `struct Interruption`（逐条扣分项）；
  - `quality.rs:269` `pub fn is_visual(ev)`。
- **评分口径**：量的是「可感知的**打断**次数」，**不评价内容好不好看**。

### ② 后端接线 ✅
- **生产者**：`quality::evaluate`（纯函数，被 API 直接调）；
- **消费者**：`crates/yekitrim-server/src/api/project_data.rs:696` `fn clip_quality(dir, duration_secs)`
  → `:708` `quality::evaluate(&cfg.segments, &subtitles, duration)`
  → `:1024` 挂进片段视图 → `:1040` `"quality": quality` 出 HTTP；
- ⚠️ **关键设计（防"假的高分"）**：`:1755` 那条测试
  「没精修过的片段 quality 必须是 null —— 回满分就是「假的高分」」
  —— 后端对"没跑过精修的片段"回 **`null`** 而不是 100。

### ③ 前端 ✅ 控件 / ⚠️ 预览
```
grep -rn "quality" webui/src/   -> 12 命中
```
| 文件:行 | 真伪 |
|---|---|
| `components/ProjectClipsSection.vue:96-97` `qualityTagClass` / `qualityHint` 徽标 + tooltip | ✅ 真 |
| `components/ProjectClipsSection.vue:586-596` `clipQuality()` 判 `Number.isFinite` | ✅ 真 |
| `components/ProjectClipsSection.vue:610` `qualityHint()` 逐条列扣分项（最多 8 条） | ✅ 真 |

⚠️ **假阳性 3 组**（人工剔除）：
- `components/DownloaderConfig.vue:101/213/234/302` —— **下载的清晰度上限** `max_quality`；
- `views/WorkbenchView.vue:291` —— B站 `biliInfo.quality`（清晰度）；
- `api/ingest.js:39` —— 入库统计里的 `quality`。

⚠️ **预览（`webui/src/polish/`）命中 0**：
```
grep -rn "quality" webui/src/polish/   -> 0 命中
```
**判定：不构成分叉。** 理由：质量分是**切片级（clip-level）徽标**，
它描述的是"这条切片剪得好不好"，**不是时间轴上的一个可渲染效果** ——
预览渲染的是片段内部的画面，两者不在同一层级。
「预览 vs 成片分叉」这条禁令管的是**同一可视化对象的两份实现**（如 §20 的噪点、§16 的速度），
**不包括**"后端算好的只读徽标"。

### 结论
**① ✅ ② ✅ ③ ✅（控件）/ ⚠️ 预览命中 0 —— 但这是层级问题，不是分叉。**
⚠️ 规格 §7.1 第 3 条「静默突变」**未做**（无阈值，见保守记录 #10）—— 已声明，8 项里 §3 的落地度是 3/4。

---

## §4 转译报告可见

### ① 代码 ✅
- **报告生成**：`crates/yekitrim-polish/src/render/dhampir.rs:1409`
  `pub fn report_lines(report: &TranslateReport) -> Vec<(&'static str, String)>`；
- ⚠️ **关键语义（防"静默的零"）**：`report_lines` **一条都不许静默** ——
  `DROP > 0` 打 `warn`，**`= 0` 也打一行 `info`**（注释：*"本次零 DROP"本身是好消息且可核对，
  静默的零与"没跑报告"分不出来*）。逐条明细走 `debug`（防止淹掉日志面板）。

### ② 后端接线 ✅ —— **三处播报 + 一处落盘**
| 消费者 | 位置 |
|---|---|
| 出片腿日志 | `crates/yekitrim-polish/src/render/pipeline.rs:680` `for (level,msg) in report_lines(...)` |
| **落盘** | `pipeline.rs:690` `<工程>/data/translate_report.json`（日志会被滚动清掉，报告要事后可查） |
| 预览腿 HTTP | `crates/yekitrim-server/src/api/polish.rs:3983`（转译时拿报告）→ `:4009` 播报 → `:4016` 回包 |
| CLI 侧 | `pipeline.rs:690` 写文件（CLI 与 server 共用） |

- **HTTP 级判据**：`polish.rs:2687` 断言 `v.get("translate_report").is_some()`
  —— 原文「回包必须带 translate_report（§4：DROP/CLAMP 不许静默消失）」，
  并逐项钉 `drop_count` / `clamp_count` 是数字。

### ③ 前端 ✅
```
grep -rn "translate_report" webui/src/   -> 1 命中
```
- `api/polish.js:25` `export function logTranslateReport(res, source)` → `:26` 读 `res.translate_report` ✅；
- **三处播报**（`components/PolishPreview.vue`）：`:641` / `:1175` / `:1301`
  —— 对应 Dhampir 预览的三条加载路径，都调 `logTranslateReport(res, 'dhampir')`；
- `:151` import。

⇒ **预览腿是主消费面**，这正是本项想要的位置（"失效不再静默"发生在用户看得见的预览上）。

### 结论
**① ✅ ② ✅ ③ ✅** —— 无断点。这是本批**接线最完整**的一项。

---

## §16 运镜速度 `speed`

### ① 代码 ✅
- **字段**：`crates/yekitrim-render-core/src/types.rs:185` `speed: Option<String>`
  （`Event::Camera`，⚠️ `Option<String>` 而非枚举 —— 认不出的值走缺省分支；
  注释 `:170` 写明「缺省必须走原式（`med` 的算法一个字节不改）」、
  `:174` 「与 `cut` 互斥：同时写以 `cut` 为准」）；
- **档位表**：`tools/polish-to-dhampir.mjs:1331` 注释「运镜速度档位表」
  + `:1355` `export function cameraTransitionFrames({speed, hardCut, durationSec, fps})`。

### ② 后端接线 ✅ —— **生产者（LLM）+ 两个消费者同口径**
- **生产者（LLM 提示词）✅ 有**：
  `crates/yekitrim-polish/src/llm/segment.rs:87`
  `想控制**推进快慢**加 "speed":"snap"/"fast"/"med"/"slow"`
  + `:91` **带完整示例**：`例: {"type":"camera","time":30.0,"end":34.0,"zoom":"近","anchor":"liver","speed":"slow"}`
  + `:90` 明确说「speed 与 cut 别一起写」。
  ⇒ **这一项和「特写」相反：提示词里四种档位齐全、且有示例。** ✅
- **回填**：`llm/events.rs:425` / `llm/mod.rs:162` 两处 `speed: None`（老路径不写 ⇒ 走缺省分支）；
- **消费者 ①（出片腿）**：`tools/polish-to-dhampir.mjs:4219`
  `cameraTransitionFrames({ speed: shot.speed, hardCut, durationSec, fps })`
  → `:3170` 注释确认「缺省（没写）= 老式子」；
- **消费者 ②（预览腿）**：`webui/src/polish/engine.js:42` `export function cameraTransitionSecs(ev, dur, hardCut)`
  → 被 `engine.js:516` `const td = cameraTransitionSecs(c.ev, c.duration, hardCut)` 消费
  → 经 `PolishPreview.vue:152` import → `:2178` 一族的求值。

⚠️ **两条链的公式逐式对齐**（本文人工比对，非 grep）：
| 档位 | 转译器（帧）| engine.js（秒）|
|---|---|---|
| `hardCut` | `0` | `0` |
| `snap`/`fast` | 固定 `0.1`/`0.25` | 固定 `0.1`/`0.25` |
| `slow` | `min(1.5, dur×0.8)` | `min(1.5, dur×0.8)` |
| `med`/缺省/认不出 | `min(dur×0.5, 0.6)` | `min(dur×0.5, 0.6)` |

`engine.js:33-34` 的注释也主动声明了两者只在"同一档位对应同一段时间"层面同源
（转译器多一步 `max(1, round(secs×fps))`，因为那边要帧）。

### ③ 前端 ✅
```
grep -rn "speed" webui/src/   -> 40 命中
grep -rn "cameraTransitionSecs" webui/src/   -> 3 命中
```
- **真控件**：`components/TimelineEditor.vue:271-273`
  ```html
  <option value="snap">极快（0.1s，像怼上去）</option>
  <option value="fast">较快（0.25s）</option>
  <option value="slow">缓慢（1.5s）</option>
  ```
  + `:889` 注释：「`med` 是**默认档、等于"没写过"**，所以选它要**删字段**而不是写 `speed="med"`」
  —— ✅ 真：**下拉存在，且默认档的落盘语义做对了**（删字段而非写值 ⇒ 存量零回归）。
- 预览侧由 `engine.js:42` 支撑（见上）。

### 结论
**① ✅ ② ✅（生产者 ✅ + 两消费者同口径）③ ✅** —— **无断点**。
⚠️ 唯一"未验"的是**端到端真跑**（未验：本次只读审计，没有真出片对比 `snap` 与 `slow` 的过渡帧数）。

---

## §17 跳切手法

### ① 代码 ✅
- **找缝**：`crates/yekitrim-polish/src/process/fx.rs:1272`
  `pub fn find_jump_seams(segments: &[Segment], mute_ranges: &[MuteRange]) -> Vec<JumpCut>`；
- **配额**：`fx.rs:1319` `pub fn cap_jump_cuts(...)`
  → `:1324` 内部调 `find_jump_seams`；
- **零新字段**（规格方案 A）：`cut` 落在 `mute_ranges` 边界 = 跳切；
- **两道配额**：单段 ≤2 / 相邻 ≥20s；**转义**：超额的 `cut` 降级为平滑（**不删事件**）。

### ② 后端接线 ✅ —— **两处成对**（同 §2 的模式）
| 消费者 | 位置 |
|---|---|
| `process` Step 1e | `crates/yekitrim-polish/src/process/mod.rs:309` `cap_jump_cuts(&mut config.segments, &config.mute_ranges, &subtitles)` |
| `llm` Step 4b | `crates/yekitrim-polish/src/llm/mod.rs:199` `let jumps = crate::process::cap_jump_cuts(&mut segments, mute_ranges, subtitles)` |

- **生产者 = 两处调用点**；**消费者 = `config.segments[].cut`**（就地改写）；
- 单测：`fx.rs:2932/2939/2948/2952/2966/2969/2989/3001` —— 覆盖"找缝 / 不找 / 降级 / 幂等"。

### ③ 前端 ⚠️ —— **有说明，无专门控件**
```
grep -rni "跳切" webui/src/   -> 2 命中
grep -rni "jump" webui/src/   -> 0 命中
```
| 文件:行 | 真伪 |
|---|---|
| `components/TimelineEditor.vue:148` `<p class="mini-hint" title="块与块首尾相接：源时间上不连续的地方，在成片里就是一次跳切">块首尾相接 = 跳切</p>` | ✅ 真（**文字说明**） |
| `components/TimelineEditor.vue:472` 注释「成片通常比源**短**（跳切/屏蔽）或**长**（重放/慢放）」 | ✅ 真（注释） |

⚠️ **判定 = ⚠️（不是 ✅，也不是 ❌）**：
- ✅ 好的方面：跳切**没有专用 UI 是设计使然** —— 它是 `cut` 事件落在屏蔽边界上的**副作用**，
  用户在「屏蔽」轨道上看到的就是它（`TimelineEditor.vue:424` 有"添加屏蔽区间"按钮，
  `:31-32` 显示"屏蔽轨道共 N 段"）。**没有第二套"跳切"概念需要用户学。**
- ❌ 缺的方面：**用户看不出"这一段是跳切、那一段不是"** ——
  时间轴上没有视觉区分，也没有"跳切有几处"的计数展示。
  §1 的 `decision_kind` 里**也没有** `jump_cut` 这个类别（7 个常量里没有它）⇒
  **跳切决策不进理由面板**。

### 结论
**① ✅ ② ✅ ③ ⚠️** —— 断在**前端可观测性**：
跳切在数据层完整、但**既没有独立可视化、也不进 §1 的理由面板**，
用户只能从"块首尾相接"这句提示自己推。
⚠️ 这也解释了为什么 §17 在总表里只有单列 ✅ 却让人"体感什么都没做"。

---

## §20 真黑白噪点 —— **本文最重要的发现**

### ① 代码 ✅（两侧都在）
**本仓转译器**：
- `tools/polish-to-dhampir.mjs:3867` `case 'noise':` → `:3888`
  `{ kind: 'noise', params: { amount: 0.08, seed: 7 } }`；
- `:3869` 注释：「§20：`noise` 发**底座的 `noise` 原语**（真黑白噪点），不再发白色 `flash`」；
- `:3653` / `:2434` 包络（`placeEnvelope`，0.08s 升 / 0.72s 落）。

**底座（`F:\para\Code\Dhampir`）**：
- `crates/dhampir-core/src/shaders/color_mask.wgsl:135`
  `const NOISE_FULL_SCALE: f32 = 255.0;` → `:137`
  `rgb = rgb + vec3<f32>((n - 0.5) * NOISE_FULL_SCALE * mask.noise_amount);`
- **量纲修复已落地**（`rgb` 来自 `textureLoad` 是 **0–255**，而 `amount` 契约是 **0–1**）；
- **镜像测试**：`crates/dhampir-worker/tests/timeline.rs:824-880`
  `const NOISE_FULL_SCALE: f32 = 255.0;` + `hash_u32` / `noise_at` 逐行移植
  （注释：*"用普通 `*` 在 debug 下会 panic、release 下静默变成别的数 —— 两种都是在测一个跟着色器不一样的东西"*）。

### ② 后端接线 ⚠️ —— **成片腿 ✅，但"五处同步"其实只同步了四处**
| 腿 | 位置 | 画法 |
|---|---|---|
| 转译器 → 底座 | `polish-to-dhampir.mjs:3888` | 真噪声（`amount 0.08 + seed 7`） |
| 出片模板 **canvas** | `templates/index.html:2050-2066` | ✅ **真噪声**（`noiseTile(t)` + `overlay`） |
| 出片模板 **DOM** | `templates/index.html:1115-1127` `handlers.noise` | ✅ **真噪声**（同一个 `noiseTile`） |
| **WebUI 活预览** | `webui/src/components/PolishPreview.vue:3322` | ❌ **纯白薄纱** `background:#fff` |

模板自己的注释（`index.html:2053-2055`）写着这个坑的历史：
> 「这里以前画的是一层纯白薄纱…而**真噪声一直都在 DOM 预览的 `handlers.noise` 里**…
>  用户拍板「noise 需要改成真黑白噪点」⇒ **两处一起**换成真噪声。」

⇒ **"两处"指模板内部的两处（canvas + DOM），漏了 WebUI 自己的 `PolishPreview.vue`。**

### ③ 前端 ❌ —— **预览是分叉的**
```
grep -rni "噪点" webui/src/   -> 0 命中
grep -rn "noise" webui/src/   -> 9 命中
```
| 文件:行 | 判定 |
|---|---|
| `components/PolishPreview.vue:37` `<div class="fx fx-noise" :style="{ opacity: noiseOp }"></div>` | ⚠️ 结构在 |
| `components/PolishPreview.vue:2178` `const noiseOp = computed(() => getNoiseOpacity(idx.value, current.value))` | ⚠️ 包络对 |
| **`components/PolishPreview.vue:3322` `.fx-noise { background:#fff; }`** | ❌ **纯白薄纱** |
| `polish/engine.js:730` `getNoiseOpacity` | ✅ 包络值对（0.08 峰值，与模板 `getNoiseOpacity` 同式） |

**实证分叉**（三条独立证据）：
1. **预览侧零个 `noiseTile`**：
   `grep -c 'noiseTile' webui/src/components/PolishPreview.vue` → **0**
   （而模板 DOM 腿 `index.html:1124` 与 canvas 腿 `:2064` 都用它）；
2. **预览侧零个 `noiseHash`**：`grep -rn "noiseTile|noiseHash" webui/src/polish/engine.js` → **0**；
3. **CSS 是白的**：`PolishPreview.vue:3322` `.fx-noise { background:#fff; }`
   —— 一个纯白 div + `opacity: noiseOp` 的叠加 = **就是"白纱"**，与 §20 要修掉的那个东西**逐字相同**。

⚠️ **而 `spec/effects.json` 的 `noise` 条目声明 `"dom_preview": true`** ——
登记表说 DOM 预览支持，实际预览画的是白纱。**登记表本身也没抓住这个分叉。**

### 结论
**① ✅ ② ⚠️ ③ ❌**
**断在哪**：`noise` 的**成片腿（转译器 + 模板 canvas + 模板 DOM）三处已同步成真黑白噪点**，
但 **WebUI 的活预览 `webui/src/components/PolishPreview.vue:3322` 仍是纯白薄纱** ——
正是本仓明令禁止的「预览 ≠ 成片」，且**不报错**。
症状：用户在预览里看到"蒙了一层白"，出片里是"颗粒噪点"。
⚠️ 未验：本次只读审计，未真出片对比同一帧的预览截图与成片抽帧（**建议列为复验项**）。

---

## §22 音效淡入

### ① 代码 ✅
- `crates/yekitrim-polish/src/render/ffmpeg.rs:60` `const SFX_FADE_SECS: f64 = 0.2;`；
- `ffmpeg.rs:89` `fn sfx_filter_chain(...)`（**纯函数、可单测**）：
  - `:96` `let fade_d = SFX_FADE_SECS.min(dur);` —— **淡入夹到素材长度**；
  - `:97` `let fade_st = (dur - SFX_FADE_SECS).max(0.0);` —— 淡出起点；
- `:73` 注释：「极短事件（`dur < 2 × SFX_FADE_SECS`）两头窗口会**重叠**：允许（等于整段都在淡）」；
- **消费点**：`ffmpeg.rs:373` `filter_parts.push(sfx_filter_chain(...))`。

### ② 后端接线 ✅
- **生产者**：`sfx_filter_chain` 生成的 ffmpeg `afade=t=in:st=0:d={}` 链；
- **消费者**：ffmpeg filter graph（`:373`）→ 出片音轨；
- **单测 + 变异自证**（`ffmpeg.rs:1302` / `:1321`）：
  - `:1302` 「变异自证：把 `afade=t=in:st=0:d={}` 整段删掉 ⇒ 本条翻红」；
  - `:1321` 「变异自证：把 `SFX_FADE_SECS.min(dur)` 改成 `SFX_FADE_SECS` ⇒ 本条翻红」；
  - `:1354` `empty_envelope_is_byte_identical_to_the_old_chain`（与 §21 空包络零回归对拍）。

### ③ 前端 ⚠️ —— **预览有淡入，但时长不同**
```
grep -rni "SFX_FADE" webui/src/   -> 0 命中
grep -rni "sfx.*fade|fade.*sfx" webui/src/   -> 0 命中
```
⚠️ 上表 0 命中**不等于预览没有淡入** —— 它换了个说法。真实现：
- `webui/src/polish/audio.js:99` `a.volume = 0` → `:103-106`
  ```js
  const fade = setInterval(() => {
    steps++
    a.volume = Math.min(target, target * steps / 5)
    if (steps >= 5 || a.ended) clearInterval(fade)
  }, 6)   // ~30ms 缓入，消除爆音
  ```
- `:113-119` 结尾还有一段 `-0.03/20ms` 的淡出 + `pause()`。

⚠️ **人工判真伪**：`grep -rni "淡入" webui/src/` 的命中**全是覆盖层的 `fade_in`**
（`TimelineEditor.vue:818/822/824/898/924/973`）—— **与音效无关**，是假阳性。

**两腿对比**：

| | 出片腿 | 预览腿 |
|---|---|---|
| 实现 | `afade=t=in:st=0:d=0.2`（ffmpeg） | `setInterval` 6ms × 5 步线性 |
| **淡入时长** | **0.2s**（`SFX_FADE_SECS`） | **~30ms** |
| 目的 | 消除起点爆音 | 消除起点爆音（注释原文） |

⇒ **同目的、同类手法，但时长差 ~6.7 倍。** 因"消除爆音"这个目的两者都能达到，
**判定为 ⚠️ 而非 ❌** —— 但这构成**参数级分叉**：
用户在预览里听到的音效起音比成片**硬**得多。
（严格说这不是 §22 引入的 —— 预览的 `audio.js` 是从模板 `sfxPlay` 移植的**独立实现**，
从未与 `SFX_FADE_SECS` 对过表。）

### 结论
**① ✅ ② ✅ ③ ⚠️**
**断在哪**：出片腿 ✅（0.2s，有单测+变异自证）；预览腿**有淡入但常数不同**
（`audio.js:103-106` ~30ms vs `ffmpeg.rs:60` 0.2s）——
**没有一条判据钉住"两腿淡入时长一致"**，所以下次改任一边都不会报错。

---

## 附：本文的核实命令（可复现）

```bash
# 命中数（⚠️ 每一条都要人工判真伪，见各节）
grep -rn "decisions"      webui/src/    # 4   —— 真
grep -rn "reason"         webui/src/    # 14  —— 13 个假阳性（LLM 状态/提交说明/排队原因）
grep -rn "quality"        webui/src/    # 12  —— 3 组假阳性（下载清晰度/B站清晰度）
grep -rn "quality"        webui/src/polish/   # 0
grep -rn "translate_report" webui/src/  # 1   —— 真
grep -rn "cameraTransitionSecs" webui/src/    # 3   —— 真
grep -rn "speed"          webui/src/    # 40
grep -rni "跳切"          webui/src/    # 2   —— 真（文字说明）
grep -rni "jump"          webui/src/    # 0
grep -rni "噪点"          webui/src/    # 0
grep -rn "noise"          webui/src/    # 9
grep -c  "noiseTile"      webui/src/components/PolishPreview.vue   # 0  ← 分叉实证
grep -rn "noiseTile\|noiseHash" webui/src/polish/engine.js         # 0  ← 分叉实证
grep -rni "SFX_FADE"      webui/src/    # 0
grep -rni "sfx.*fade\|fade.*sfx" webui/src/    # 0（真实现在 audio.js:99-106）

# 底座（独立仓）
grep -rn "NOISE_FULL_SCALE" F:/para/Code/Dhampir/crates/   # wgsl:135/137 + tests:824
```

⚠️ **本文未跑任何真出片 / 真渲染**（task-20 是只读审计）。§16 / §20 / §22 的
"两腿一致"结论**全部由源码逐式比对得出，不是实测** —— 需要实测的三条见各节标注。

## 附：给总表的三条修订建议

1. **§20 的 ✅ 应降为 🟡（部分）**：成片腿已修，**预览腿仍是白纱**
   （`PolishPreview.vue:3322`）⇒ 属"预览 ≠ 成片"。
   且 `spec/effects.json` 的 `noise.dom_preview: true` 与事实不符，**要一并改**。
2. **§17 的 ✅ 应标注"前端仅文字说明"**：数据层完整、无独立可视化、不进 §1 理由面板
   （`decision_kind` 的 7 个常量里没有 `jump_cut`）。
3. **§22 建议加一条两腿对拍判据**：`audio.js` 的淡入常数与 `SFX_FADE_SECS` 目前**无人守**。
