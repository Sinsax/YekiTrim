# YekiTrim 架构复盘与重构计划

> 状态：**待评审**（未动一行代码）
> 日期：2026-10-15
> 基线：`eba5c10`（工作区另有未提交改动，见 §6 风险）
> 方法：两条机器门禁全绿的前提下，按「度量 → 分流 → 判据」三步取证；**每条结论都带可复核的证据行**。
> 取证来源：主编排 + 2 个只读审计（Rust 冗余 / docs 冗余）。**审计提出的修正已逐条复验并采纳**，
> 被推翻的自我判断记录在附录。

---

## 0. 结论摘要（先读这段）

**这次复盘推翻了我最初的假设。** 起初按代码行数看，`api/polish/` 4933 行、`process/mod.rs` 3981 行、
`project/config.rs` 3801 行，像是"巨型文件"问题。逐文件把 `#[cfg(test)]` 边界切出来之后，
**这个判断是错的**：全仓 76286 行生产代码 + 29426 行内联测试，巨型文件里有 30%~50% 是测试。
真正的冗余**不在文件长度上**。

复盘后确认的**真问题只有两个**，其余全部降级为次要项：

> **① 重复真相（主线）。** 同一个事实在 3~4 处独立实现（Rust / 转译器 / 预览引擎 / 面板），
> 靠 43 个门禁脚本（其中 10 个专职 parity，135 KB）把它们"钉"在一起。

这不是"有冗余内容"，而是**架构性的重复真相**（multi-source-of-truth）。43 个守卫脚本是**症状**，
不是解法 —— 它们是防止重复真相分叉的**税**。计划的主体（§3）就是把这条根因拆掉。

> **② `cover-takumi` feature 归属（审计新发现）。** `render/takumi.rs`（1051 行，WebUI 封面的实际
> 后端）在 server / service 语境下**整份不参与编译**，导致 `/api/health` 的封面能力位恒为 false。
> 这不是冗余而是**功能正确性问题**，见 §4.5 —— 价值高于全部清理项。

其余为**次要清理**（§4.1~4.4、4.6），体量与风险都低得多。

| 项 | 判定 | 证据 |
|---|---|---|
| 分层与依赖方向 | ✅ **健康**，无需重构 | 17 条边界规则 0 违规；`server → service` 单向 |
| 巨型文件 | ⚠️ **伪问题** | 26774/105712 行是内联测试；生产代码最大 4165 行 |
| 重复真相（Rust/转译器/预览/面板） | 🔴 **真问题（主线）** | 10 个 parity 守卫 + 多处"三处/四处同源"自述 |
| 文档冗余 `docs/` 22.2 MB 入库（全量 69.4 MB / 270） | 🟡 次要，可批量归档 | `docs/plan` 138 文件多为闭环自引用 |
| 死代码 / legacy | 🟡 **少量真死码**（13 个符号） | 审计逐条核实，见 §4.1；legacy **不是**死码 |
| ⚠️ `cover-takumi` feature 归属 | 🔴 **新发现的设计风险** | server/service 编不到它，见 §4.5 |

---

## 1. 度量基线（可复核）

```
生产代码 76286 行   ← 排除 #[cfg(test)] 之后
内联测试  29426 行   ← 占"源码"口径的 27.8%
```

| crate | 生产行 | 含测试行 |
|---|---:|---:|
| yekitrim-polish | 27759 | 38981 |
| yekitrim-core | 17960 | 24087 |
| yekitrim-server | 16100 | 24479 |
| yekitrim-service | 8279 | 11154 |
| yekitrim-render-core | 2703 | 3462 |
| yekitrim-cli | 1884 | 1884 |
| xtask | 1601 | 1665 |

**测试密度极高**（`polish/fx.rs` 1477 生产 / 1841 测试、`api/polish/` 2471 / 2692），
说明这个仓的测试纪律很好 —— 这一点在重构时必须**当成资产保护，不能当包袱砍**。

仓库体量：工作区 tracked 701 文件 / 53.2 MB。`docs/` 入库 251 文件 / **22.2 MB**，
但**磁盘上实际 270 文件 / 69.4 MB** —— 差额 47 MB 是 untracked 的 demo 素材（`.gitignore` 的裸
`*.mp4` / `*.srt` 规则吞掉了它们）。**统计 docs 体积时必须以 tracked 为准**，否则会高估一倍以上。

---

## 2. 健康的部分（明确不动）

**① 分层与依赖方向是干净的。** `python3 scripts/check_module_boundaries.py` → 17 条规则 / 415 文件 / **0 违规**。
`python3 scripts/check_module_map.py` → 35 模块 / 7 项全覆盖，通过。六 crate 单向依赖图成立，
`server` 里对 `yekitrim_core::` 的 79 处直接引用**不是违规** —— `core` 是共享底座，硬规矩只禁止
`service → server` 反向边，而那条边是 0。

**② 删库留下的痕迹都已带说明封存。** `render/backend.rs`（38 行）在 wgpu / Chrome 两条腿删除后
只剩 `Dhampir` 一个枚举值，看着像残留抽象；但它的 doc 明确写了"为什么保留这个枚举"，
且 `parse()` 仍接受 `wgpu`/`native` 别名（老命令兼容），`chrome`/`chome` 刻意**不**认
（避免"选了 Chrome 却跑底座"的静默换路）。`render/mod.rs` 同理。**保留。**

**③ `legacy.rs` 是活的迁移路径，不是死码。**
`render-core/legacy.rs`（事件 `duration → end`）与 `polish/project/legacy.rs`（吞掉已下线键
`entrance`，因为 `deny_unknown_fields` 会让老工程读不出来）都有真实调用者。
**保留。**

---

## 3. 主线：消除重复真相

### 3.1 问题陈述（带证据）

同一组常量 / 同一套公式在**四条路径上各写一遍**，各自"看起来都对"：

| 事实 | 实现处数 | 权威 | 守卫 |
|---|---:|---|---|
| 排版角度区间 `LAYOUT_ANGLE_MIN/MAX` | **3** | `tools/polish-to-dhampir.mjs:1496` | `check-anim-layout-angle.mjs` |
| 文字颜色预设 | **4** | `placement.rs::ElementColor` | `check-element-color.mjs` |
| 动画档位 `ANIM_PRESETS` | **3** | 转译器 | `check-preview-anim.mjs` (66 KB) |
| noise `noiseHash` 逐常数 | **3** | — | `check-noise-parity.mjs` |
| 音效淡入淡出 | **2**（机制都不同） | — | `check-sfx-fade-parity.mjs` |
| EDL `clips` 消费点 | **3** | `docs/plan/edl-retiming-spec.md` | `check-edl-invariants.mjs` |

证据行（源码自述）：
- `TimelineEditor.vue:1135` —— "⚠️⚠️ **权威在 `tools/polish-to-dhampir.mjs` 的 `LAYOUT_ANGLE_MIN/MAX`**"，紧接着 1144-1145 行**又抄了一遍** `const LAYOUT_ANGLE_MIN = -30`。
- `engine.js:768` —— "**与转译器 `LAYOUT_ANGLE_MIN/MAX` 及面板同值**"，774-775 行**第三遍**抄写。
- `check-element-color.mjs:6` —— "颜色这条链是"**一个事实、四处实现**""。
- `check-edl-invariants.mjs:308` —— "⚠️ 这不是通过 —— 三处同源目前**只有两处有判据**"。

**守卫税**：`scripts/` 下 43 个 `check-*`，1033.8 KB；其中 10 个 `*-parity.mjs` 共 135 KB，
职责**仅仅是发现重复实现之间分了叉**。转译器本体 `polish-to-dhampir.mjs` 431.9 KB，
它的守卫 `check-yekitrim-translator.mjs` **289.1 KB** —— 守卫是本体体量的 67%。

> 说明：这些守卫**写得很好**（每条 doc 都写了它防止的真实事故）。问题不在守卫质量，
> 而在**它们要防的东西本来就该不存在**。删掉重复真相，守卫可以连同它防的分叉一起消失；
> 反过来先删守卫，就是把踩过的坑重新埋回去。

### 3.2 重构方向：单一真源 + 代码生成

核心手法：**把"多处抄同一组数字"换成"一处定义、其余生成或引用"**。

**R1 — 建立唯一真源 `spec/`**
`spec/` 目录已经存在但只有 `spec/effects.json`（27.7 KB）+ 一个 0 字节的 `.probe` 残留。
把它提升为**机器可读的唯一真源**，收纳目前散落的契约：
`layout-angle.json`、`anim-presets.json`、`element-colors.json`、`sfx-fade.json`（以及既有的 `effects.json`）。

**R2 — 生成各消费侧**
- Rust 侧：build.rs 或 xtask 从 spec 生成常量模块；
- `tools/polish-to-dhampir.mjs`：`import` spec，不再内联副本；
- `webui/src/polish/engine.js` + `TimelineEditor.vue`：从 spec 生成 / 导入。

**R3 — 守卫降级**
真源落地后，`check-anim-layout-angle.mjs` 这类"逐值对拍三处"的守卫，退化为
"spec 与生成产物一致"（一次判据，而不是 N 处两两比对）。**不删守卫语义，只削掉它的分支数。**

### 3.3 分期与验收

| 阶段 | 内容 | 验收 |
|---|---|---|
| P0 | 建 `spec/` 真源 + 生成器骨架，**先只收 `LAYOUT_ANGLE`** 一对常量 | 三处不再出现字面量；`check-anim-layout-angle.mjs` 仍绿 |
| P1 | 收 `ELEMENT_COLORS`（四方表） | `check-element-color.mjs` 仍绿，且分支数下降 |
| P2 | 收 `ANIM_PRESETS`（三处，最大一件） | `check-preview-anim.mjs` 仍绿 |
| P3 | 收 `noise` 逐常数与 `sfx` 淡入淡出 | 两条 parity 守卫改判 spec ↔ 生成物 |

**硬约束**：每一期都必须**先跑绿、改完再跑绿**，且属于"两处实现 → 一处真源"的**减法**。
任何一期若发现某处**本来就该不同**（例如预览与成片的采样差半像素），**立即停止该分支**并把
差异写进 spec 作为显式字段 —— 不允许用"就近对齐"掩盖真实差异。

---

## 4. 次要清理项

### 4.1 死代码 / legacy —— 逐条核实

**A. 真死码（13 个符号，零生产消费者）** —— 独立审计取证，我已复验（递归全仓 grep，命中数 = 定义行数）：

| file:line | 符号 | 命中 | 替代 / 说明 |
|---|---|---:|---|
| `polish/src/project/config.rs:1352` | `fn is_zero` | 1 | 同名生产项在别处；此为本文件私有残骸 |
| `polish/src/project/config.rs:1347/1348` | `default_zoom` / `default_origin` | 1 / 1 | 生产项在 `render-core/types.rs:777` 等，同名不同物 |
| `service/src/db.rs:789` | `Db::touch_project` | 1 | 无调用点 |
| `service/src/db.rs:1402` | `Db::list_ingest_files` | 1 | `IngestFileRow` 仍被 `ingest_file` 用，**只删该函数** |
| `service/src/workspace.rs:127` | `Workspace::current_stage` | 1 | 权威已迁 `core::config::types::ProjectConfig::current_stage` |
| `polish/src/render/gif.rs:32` | `decode_gif_frames_rgba` + `GifFrames`(:23) | 1 | 生产只走 `decode_sticker_frames_png` |
| `polish/src/render/sound.rs:247` | `sound_file_exists` | 2 | `render/AGENTS.md:48` 已自认「暂无消费者」 |
| `server/src/persist.rs:117` | `remember_project` | 2 | 只剩定义 + 文档提及；`remember_last` 才是现役 |
| `service/src/status.rs:59` + `server/src/api/projects.rs:36` | `project_summary` ×2 | 3 | 两层过渡别名，**整条链都无调用者** |
| `polish/src/project/config.rs:401` | `#[deprecated] region_rect` | 4 | 其余 3 处 = `allow(deprecated)` 测试 + 文档；**无生产调用** |
| `polish/src/assets/emoji_search.rs:252` | `EmojiCatalog::all()` | 1 | `self.all_codepoints` 仍被 `search` 用，**只删 getter** |

**B. 陈旧 `#[allow(dead_code)]`（7 处）** —— 函数**都在生产用**，属性是噪音，会让真死码淹没：
`projects.rs:136/156`（`sanitize_segment`/`new_project_dir`，2 处生产调用）、`jobs.rs:888`（3 处）、
`db.rs:1362/821/1326`（各 3/4/2 处）、`workspace.rs:59`。**批量删属性，0 行为变化。**

**C. ⚠️ 我最初的 8 个嫌疑 —— 全部推翻**（记录在此避免重复调查）：

| 嫌疑 | 核实结果 | 证据 |
|---|---|---|
| `assets/emoji_search.rs` 5 处 `allow(dead_code)` | ❌ **活的**；其中 **4 处是 serde 字段** | `:16/:19/:28/:37` 是 JSON 解析必需字段（删了直接解析失败），**不是死码** |
| `assets/irasu_search.rs` / `assets/sfx_catalog.rs` | ❌ 活的 | `sfx_catalog::lookup` 被 `server/api/polish.rs:2220` 等 20+ 处调用 |
| `render/backend.rs` 单值枚举 | ❌ **有意保留** | doc 写明老命令兼容与"禁止静默换路" |
| `project/legacy.rs` / `render-core/legacy.rs` | ❌ **活的（不可退役）** | `validate.rs:98`（polish.toml 主加载路径）+ `server/api/polish.rs:1840`、`llm/events.rs:184/227` |
| `scripts/` 里"零引用"脚本 | ❌ **活的** | 见下 |

> ⚠️ **方法论教训（值得记进 `scripts/AGENTS.md`）**：我最初用"脚本文件名是否在文档里出现"筛出
> 10 个"零引用"门禁脚本，看上去很像死文件。逐个读文件头后发现**全部是近期活跃守卫**
> （`check-anim-layout-angle.mjs` 2026-10-14、`check-sfx-fade-parity.mjs` 2026-10-12）。
> 同理，用"模块名出现次数"筛出的 4 个"零引用" asset 模块也全是活的。
> **在这种"高注释密度 + 弱命名引用"的仓里，引用计数不能单独作为删码判据** ——
> 必须落到具体符号（如 `sfx_catalog::lookup`）上再数。
> 反过来说：**A 组那 13 条之所以可信，正是因为它们数到了"定义行本身"这个最强的形式。**

**D. 确认保留（仅测试引用，删了会丢回归网）**：`render/pipeline.rs:83/104/128` 的
`canvas_supported_effects`/`used_effect_types`/`missing_support`（S4 canvas 冻结闸门遗产）、
`jobs.rs:927` `JobManager::list`、`render/frame.rs:37/113` `extract_chunk_frames`/`split_jpeg_frames`
（真像素探针夹具底座）。**建议保留或迁进 `#[cfg(test)]`，不要直接删。**

### 4.2 内联测试搬出

10 个文件的 `#[cfg(test)]` 段超过 600 行，最大的 `api/polish/` 有 **2692 行测试**
（比它的 2471 行生产代码还多）。建议**只搬最大的 3~4 个**到同级 `tests/` 或 `#[path]` 文件：

- `server/src/api/polish.rs`（2692）、`polish/src/process/fx.rs`（1841）、
  `polish/src/process/placement.rs`（1279）、`server/src/api/project_data.rs`（1093）。

⚠️ **注意**：内联测试常访问私有函数，搬出会**逼着放宽可见性** —— 收益（可读性）可能不抵代价。
**建议先只做 `api/polish/` 一个试点**，量出真实代价再决定是否推广。
**这是纯整理，不是重构，优先级最低。**

### 4.3 `docs/` 归档（入库 22.2 MB；磁盘 69.4 MB）

**制度缺口是根因**：`docs/README.md` **不存在**，全仓无一条成文归档判据。
`docs/archived/` 只有 6 个文件（多为 2026-10-03 一次搬迁），而 `docs/plan/` 涨到 **138 个文件** ——
**执行期过程文档（计划/重验/调研/报告）完成后从未被清理**。
⚠️ 注意：`docs/archive/`（无 d）**不存在**，命名只有 `archived`。

**建议补一条判据**（进 `docs/AGENTS.md`）：*「计划文档在其描述的改动全部落地、
且有后续文档显式取代之后，移入 `docs/archived/<年-月>/`」*。

**A. 有文档内显式取代链（最硬的证据，可直接归档）** —— 约 395 KB：

| 文件 | KB | 取代判据 |
|---|---:|---|
| `plan/polish-upgrade-conservative-log.md` | 166.6 | 施工期保守记录，被 10-12 consolidated/v2 收编 |
| `plan/polish-upgrade-plan-v2.md` | 88.6 | **自身即旧版**：L3 自述"取代 consolidated"，随后又被 master-plan 与 10-14/15 系列取代 |
| `plan/polish-upgrade-consolidated.md` | 28.1 | L19 自认"§1 判据有洞"；被 v2 L3/L10 显式取代 |
| `plan/reverify-batch-{A,B,C}.md` | 74.5 | 已并入 `polish-upgrade-reverify.md` |
| `plan/polish-upgrade-master-plan.md` | 37.4 | 被 v2 L9 降格为"上游决策记录" |

**B. 调研/报告类，结论已入代码** → 可归档：ASR 五份 survey（`moss-transcribe` / `model-landscape` /
`cloud-api` / `sensevoice-replacement` / `per-mode-tradeoff`，共 56.3 KB，选型已定 SenseVoice）、
`export-pipeline-web-survey.md`、`linux-adaptation-test-report.md`、`takumi-cover-{capability,handover}.md`、
`rename-{execution-plan,verification-report}.md`、`p4-{implementation-plan,prerequisite-verification,bilibili-downloader-selection}.md`。

**C. 技术栈已与仓库矛盾（可归档）**：`docs/web-ui-design.md`（85.2 KB）L5/L14/L24 反复声明
技术栈是「零构建 vanilla JS」、代码在 `crates/yekitrim-cli/src/server/` —— **该目录不存在**
（实测 `Test-Path` = False），实际是 Vue+Vite 的 `webui/`。同代的
`superpowers/specs/2026-09-03-yekitrim-webui-design.md` 一并归档。

**D. 磁盘大件（可回收 ~47 MB）**：
- `wgpu-effect-reference/demo/chrome_export.mp4` 与 `wgpu_export.mp4` —— 各 20.9 MB，
  **sha256 实测完全相同**（`1f519ca1…44606`），是同一文件的两份拷贝。两者均 untracked，
  是已删除 wgpu 管线的对比成片，无脚本读取 → **删这两个可回收 40.9 MB**。
- `plan/t5-karaoke-frames-2026-10.png` ≡ `plan/t5-karaoke-2026-10.png`（sha256 `2443d396…`，156 KB）。
- `plan/w1-noise-{broken,fixed}-frame165-2026-10.png` 共 6.1 MB（单次排查截图，建议按日期归档而非删）。

> ⚠️ **审计纠正了我两处判断，记录在此**：
> ① 我把 `docs/` 记为 22.2 MB，那是 **tracked-only**；磁盘实为 69.4 MB，差额是 untracked 素材。
> ② 我写"`docs/wgpu-effect-reference/` 随 wgpu 删除一并退役"是**错的** —— 见下方 §4.6。

⚠️ **必须保留**（被根 AGENTS.md / 卡片直接引用为权威）：`docs/deploy.md`（7 引用）、
`docs/file-layout.md`（11 引用）、`docs/module-map.json`（6）、`docs/job-queue.md`（4）、
`docs/webui-components.md`（**13 引用，是 `components/AGENTS.md` 的指令预算溢出正文，不是旧设计稿**）、
`docs/yekitrim-integration.md`（5，被 `polish-to-dhampir.mjs` 引用）、
`docs/polish-events-reference.md`、`docs/golden/dhampir-parity/`（冻结帧基准）、
`docs/anim/`（`check-anim-docs-snapshot.mjs` 硬编码路径），
以及 `plan/` 中被卡片点名的约 12 个（`service-direction-review` / `effect-pipeline-unification` /
`font-library` / `dhampir-parity-register` / `edl-retiming-spec` / `p4-implementation-plan` /
`polish-vertical-canvas` / `fx-type-consolidation` / `takumi-cover-capability` 等）。

⚠️ **`docs/dhampir-base-handoff.md`（18.6 KB）归档前必须先改引用**：它确认被
`dhampir/base-handoff.md`（30.7 KB，底座侧已中性化回复，D1~D3 均已修复）取代
（`dhampir/README.md:30` 明写处置结果已记在新那份），**但有 6 份文档仍指向旧路径**
（`dhampir-parity-register.md:123,126,129`、`effect-pipeline-unification.md:92`、
`s4-second-chain-removal-decision.md:4,46,91`、`linux-migration-handoff.md:95`、
`rename-verification-report.md:161`）—— **先改引用再归档，否则断链**。

### 4.4 根目录散件

`ffmpeg-20261015-034830.log`（3 KB，运行残留）、`dhampir-subtitle.srt`（104 B，散落产物）、
`spec/effects.json.probe`（**0 字节**）。均为可删的临时残留 —— 属"改代码协议"里
"新增目录 = 新增卡"之外的一次性卫生问题。

### 4.5 🔴 `cover-takumi` feature 归属 —— **新发现，价值高于全部清理项**

**这不是死码，是构建图上的真实缺口。** 证据（已复验）：

- `polish/src/render/takumi.rs:30` 有 `#![cfg(feature = "cover-takumi")]`，`render/mod.rs:6-7` 同门控。
- `polish/Cargo.toml:21` 是 `default = []` —— **polish 自己默认不编**。
- **只有 CLI 编它**：`cli/Cargo.toml:66-67` `default = ["cover-takumi"]`。
- **server 与 service 都没编**：两者对 `yekitrim-polish` 都是**无 features 的路径依赖**
  （`server/Cargo.toml:20`、`service/Cargo.toml:15`），`server/src` 的 `[features]` 块是空的。

**后果**：`render/takumi.rs`（1051 行，**外加** `tests/takumi_font_id.rs`）
在 `cargo test -p yekitrim-server` / `cargo check --workspace --all-targets` 下**整份不参与编译**，
而它正是 WebUI「生成封面」的实际后端。于是 `/api/health` 的
`cover_static.compiled`（`api/mod.rs:236,271-275`）在 server 语境**恒为 false** ——
一个"能力位"在它自己的服务端永远报 false。

**判定：设计风险 > 冗余，需决策，不要当删减处理。** 备选方向（**本轮不实施**）：
① 把 `cover-takumi` 提升为 polish 的默认；② 让 server 显式转发该 feature；
③ 明确"封面只在 CLI 路径下可用"并把 health 位改成如实反映。
无论哪条，都要与 `docs/plan/takumi-cover-capability.md`（6 引用）一起评估。
附带收益：`cover.rs:114` 那处 `#[cfg_attr(not(feature=...), allow(dead_code))]` 补丁可一并删掉。

### 4.6 前端审计：**我提出的三条"重复"假设全部证伪**

记录在此，因为我最初把它们当成了候选主线（见附录 R4）：

| 我的假设 | 实测结论 | 证据 |
|---|---|---|
| `PolishPreview.vue` ↔ `engine.js` 重复实现渲染 | ❌ **是消费关系** | `PolishPreview.vue:212` **import 18 个符号** 自 `'../polish/engine'`；engine.js 导出 60+ 纯函数，无一被重写 |
| `TimelineEditor.vue` ↔ `edl.js` 重复 | ❌ **是分层** | `edl.js:19` 从 engine.js 导入；`edl.js:6-7` 注释明写"out_start 推导**只有一份**、本文件只是薄封装、**不许**再写一遍"，另有 17 项对拍门禁 |
| 封面逻辑三处（+ `PolishView.vue`） | ❌ **只有两处，且属不同层** | `PolishView.vue` 全文**无任何 cover API 调用**，只在 `:587` 挂 `<CoverDesigner>`；前端实现仅 `CoverDesigner.vue` 一处，Rust `cover.rs` 是渲染后端 —— 正常前后端分工 |

**⚠️ 两条"看着该合并、其实不能合并"的**：
- `AppDialog.vue`(33 行) 与 `AppModal.vue`(57 行)：模板结构相似，但**交互契约不同** ——
  前者是 store 驱动的 confirm/prompt（**1 处使用**），后者是 slot 通用外壳 + Esc 监听 + body 滚动锁
  （**12 处使用**）。**合并是错的。**
- `api/*.js`：`client.js` **确是唯一收口**（全仓 7 处 `fetch(`，只有 `client.js:85` 打 `/api`）。
  但有一处轻微破口：`stores/log.js:98` 直连 `/api/admin/logs?limit=500`，绕过 `client.js`。
  **可评估收编，优先级低。**

**⚠️ 真正的债（受控，不在本轮动）**：`engine.js` 是 `templates/index.html`（**2948 行**）的
**手工移植**（`webui/src/polish/README.md:4,7` 明写"改特效**先改 index.html 再同步移植到这里**"）。
这是"预览 ≠ 成片"这条主线债的**前端半边**，且已被 10 个前端门禁钉住 —— 属**已受控的架构债**。
它与 §3 是**同一个根因**（同一事实多处实现），但**不作为本轮切口**：先做 §3 的 spec 真源，
再看是否能把 index.html ↔ engine.js 也纳入同一套生成。

**⚠️ `webui/dist` 是健康的，勿动**：`.built-from` 时间戳（08:39）**晚于** src 最新文件（08:38），
31 个文件已入库。提交里"32 个 dist 文件删除"**是误读** —— 那是 vite **内容哈希滚动重命名**
（旧 hash 删、新 hash 增，同名不同 hash），**不是**陈旧重复 chunk。

---

## 5. 执行顺序建议

按"**信息增益 / 风险**"排序，而不是按体量：

**第 0 步 —— 决策项（先做，成本最低、价值最高）**
1. **§4.5 `cover-takumi` feature 归属** —— 这是本轮唯一"**功能可能实际是坏的**"的发现
   （server 语境下封面能力位恒 false）。**先定性再谈清理**，并评估是否需要一条判据守住它。
2. **§4.3 补归档判据**（`docs/AGENTS.md` 加一条，`docs/README.md` 建起来）——
   制度缺口是 `docs/plan` 涨到 138 文件的根因；补规则比一次性搬家更能防复发。

**第 1 步 —— 零风险回收（纯文件操作，不碰代码）**
3. §4.3 D：删两个 sha256 相同的 20.9 MB mp4 + 一对同名 png，**回收 ~47 MB**；
   `spec/effects.json.probe`（0 字节）与根目录两件散件一并清掉（§4.4）。
4. §4.3 A/B/C 归档（需先改 `dhampir-base-handoff.md` 的 6 处引用）。

**第 2 步 —— 低风险删码（可独立提交，先跑 `cargo check --workspace --all-targets`）**
5. §4.1 B：清 7 处陈旧 `#[allow(dead_code)]`（纯属性，0 行为变化）。
6. §4.1 A：删 13 个已确证死码符号。

**第 3 步 —— 主线重构（唯一真正动架构的一步）**
7. §3 的 **P0**：只收 `LAYOUT_ANGLE` 一对常量，验证"单一真源 + 生成"在本仓构建链
   （cargo build.rs / node / vite）上走得通。**P0 是可行性验证，不是收益项。**
8. P0 通过后再排 P1~P3。**若 P0 的摩擦大于收益，就此止步** —— 那说明 43 个守卫是当前
   架构下的合理代价，而不是需要偿还的债。

**第 4 步 —— 可选**
9. §4.2（内联测试搬出）：**最后做，或不做**。
10. §4.6 `engine.js` ↔ `templates/index.html` 手工移植：**等 §3 定型后再评估**能否并入同一套生成。

---

## 6. 风险与前置约束

- ⚠️ **工作区不干净**：`git status` 显示 12 个文件已修改（含 `tools/polish-to-dhampir.mjs`、
  `webui/src/components/PolishPreview.vue`、`webui/src/components/TimelineEditor.vue`），
  `webui/dist` 有大量删除未提交，另有 6 个未跟踪的 docs/plan 文档。
  **§3 的重构正是要动这几个文件** —— 必须先落定当前改动或切分支，否则冲突/丢失风险极高。
- ⚠️ **`webui/dist` 是编译期输入且已入库**：任何动 `webui/src` 的生成式改造，
  都必须同批 `npm --prefix webui run build` 重建 dist，并跑 `node webui/scripts/check-overlay.mjs`。
- ⚠️ **门禁不可绕过**：每期改完必须两条门禁全绿 + 对应 parity 守卫仍绿。
  `scripts/AGENTS.md:192` 写明"违规了就改代码，或改规则并写清理由"。
- ⚠️ **根 `AGENTS.md` 有 20 KB 硬预算**（当前 14.7 KB）：本计划若要登记进卡片，
  只能进 `scripts/AGENTS.md` 或新增 `docs/` 文档，**不许塞进根文件**。
- ⚠️ **本计划属跨模块改动**：按"改代码协议"第 2 条，需先读对方卡，并在提交说明里写明理由。

---

## 附：方法与自我修正记录

本次复盘共提出 **6 条初始判断，其中 5 条被证据推翻**。全部记录在此，因为这比结论本身更有价值 ——
**它划出了"凭直觉看冗余"与"凭证据定冗余"之间的距离。**

| # | 初始判断 | 核实结果 | 修正 |
|---|---|---|---|
| R1 | 巨型文件是主要问题（`api/polish/` 4933 行…） | ❌ 错。其中 2692 行是 `#[cfg(test)]` | 降级到 §4.2 最低优先级 |
| R2 | 10 个门禁脚本零引用 = 死文件 | ❌ 错。逐个读文件头，全是近期活跃守卫 | 撤销；写入方法论教训（§4.1） |
| R3 | 4 个 asset 模块零引用 | ❌ 错。grep 只数了模块名，没数导出符号 | 全部撤销 |
| R4 | 前端三处重复实现（预览/时间轴/封面） | ❌ **三条全错**，见 §4.6 | 撤销；改为记录"不能合并"的两对 |
| R5 | `docs/` 约 200 MB | ❌ 错。**入库 22.2 MB**，磁盘 69.4 MB（差额是 untracked 素材） | 基线已改（§1） |
| R6 | `wgpu-effect-reference/` 可整目录退役 | ❌ 错。`demo/` 是 3 条门禁的**活夹具** | 改为"只删两个 mp4"（§4.3 D） |
| — | **`cover-takumi` feature 归属** | ✅ **审计新发现，我已复验** | 新增 §4.5，列为本轮最高价值项 |

**方法论结论（建议记进 `scripts/AGENTS.md`）**：
在这个"高注释密度 + 弱命名引用 + 大量生成物"的仓里，**引用计数不能单独作为删码判据**。
R2/R3/R4 三条都是同一个错误 —— 用"名字出现了几次"去代理"还有没有人用"。
可用的判据有两级，且**必须先做第一级**：
1. **最强**：命中数 = **定义行数**（如 §4.1 A 那 13 条，每条都只剩自己的定义）。
2. **次强**：读被怀疑对象的**文件头/卡**（R2 靠这招推翻；`legacy.rs` 的保留理由也是这么读出来的）。

**经双重核实、可作为计划基石的项**：
- `LAYOUT_ANGLE` 三处字面量 —— `polish-to-dhampir.mjs:1496-1497` /
  `TimelineEditor.vue:1144-1145` / `engine.js:774-775`（**三处都是文件内直接可见的 `const` 字面量**）。
- parity 守卫的数量与体量 —— 43 个 `check-*` / 1033.8 KB，10 个 `*-parity.mjs` / 135 KB。
- `cover-takumi` 的 feature 传播链 —— `cli/Cargo.toml:66-67` 有、`server/Cargo.toml:20` 与
  `service/Cargo.toml:15` 无（**路径依赖不带 feature 是 Cargo 的确定语义，不依赖 grep**）。
