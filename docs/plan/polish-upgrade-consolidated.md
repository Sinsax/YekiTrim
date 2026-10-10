# 精修系统升级 · 施工计划书（整合版 · 状态可核对）

> 立档：2026-10-10。**本文是精修升级的唯一施工入口。**
> 它把 12 份设计文档整合成一份**带真实状态的施工计划**，并**更正了 3 处与代码不符的旧结论**。
>
> | 关系 | 文档 |
> |---|---|
> | **本文** | 施工计划书（整合 + 状态核实 + 批次） |
> | 上游决策记录 | [`polish-upgrade-master-plan.md`](polish-upgrade-master-plan.md)（679 行，25 项总表 + 10 处修正） |
> | 专题规格 | [`elements-spec.md`](elements-spec.md) · [`camera-and-noise-spec.md`](camera-and-noise-spec.md) · [`editing-decision-layer.md`](editing-decision-layer.md) · [`emotion-narrative-engine.md`](emotion-narrative-engine.md) · [`polish-enrichment-proposal.md`](polish-enrichment-proposal.md) · [`conversational-editing-and-music.md`](conversational-editing-and-music.md) |
> | 调研目录 | [`editing-techniques-catalog.md`](editing-techniques-catalog.md) · [`camera-vfx-editing-catalogue.md`](camera-vfx-editing-catalogue.md)（806 行） |
>
> ⚠️ **上游文档的 `✅ 已定 / 已拍板` 一律表示"方案定了"，不表示"代码做了"。**
> 本表用**独立的状态列**表达真实进度。这个区分就是本文存在的理由 ——
> 上游文档通篇 ✅，读者（包括我）会误读成"已实现"。

---

## 0. 一句话结论

**25 项改动，已实现 0 项，已定案 25 项。**
地基（批 1）四项**全部可立即开工、无阻塞**；唯一被自己前置卡住的是 **§18 特写**。

---

## 1. 状态分级（本文的判据）

| 标记 | 含义 | 判据 |
|---|---|---|
| ✅ **已实现** | 代码在，且**有消费者** | grep 有命中**且**接线到渲染/转译/预览任一条腿 |
| 🟡 **部分** | 一半在（如字段有、消费者无；或只读不写） | 同上，但只覆盖部分路径 |
| 📐 **待实施** | 已定案、有规格、无阻塞 | 规格完整、依赖已满足 |
| ⏳ **被阻塞** | 已定案，但前置未做 | 依赖项状态 ≠ ✅ |
| ⚠️ **待验证** | 有实现，但缺实测/对拍 | 规格里明写"必须先实测" |
| ❌ **不做** | 已否 | 决策记录 |

⚠️ **"grep 命中"必须人工判真伪** —— 本文已剔除 2 处假阳性：
`reason`（命中的是 LLM 重做状态，**不是**决策理由）、
`rotation`（命中的是数组轮转，**不是**荷兰角）。

---

## 2. 总表：25 项的真实状态

### 批 1 · 地基（4 项 —— **全部 📐 可立即开工，零阻塞**）

| # | 名称 | 状态 | 证据（本文核实） | 一句话 |
|---|---|---|---|---|
| **1** | 决策理由 `reason` | ✅ **已实现**（最小形态） | `ProjectFile::decisions` + `polish.toml` 的 `[[decisions]]`；process 每趟重算落盘；时间轴分段面板只读展示。**未做**规格 §5.1 的 `[editing]` 三表与审批流 | 每个自动决策说"为什么切在这" |
| **2** | 句末边界守卫 | ✅ **已实现** | `fx::align_segment_boundaries` + `is_sentence_end`；process Step 0b 与 llm Step 4b 两处成对；含兜底候选（真实字幕 0% 句末标点） | 不再把话说一半就切 |
| **3** | 剪辑质量分 | ✅ **已实现**（3/4 判据） | `process::quality::evaluate`（纯函数）+ `GET /project-data/clips` 的 `quality` + 切片卡「剪辑 N」tag。**未做**规格 §7.1 第 3 条「静默突变」（无阈值，见保守记录 #10） | 每条切片一个"剪得好不好"的数 |
| **4** | 转译报告可见 | ✅ **已实现**（`8c0770c` 后） | `dhampir::report_lines` + `render_video_only` 带报告 + `/polish/dhampir-project` 回包带 `translate_report` + 前端 `logTranslateReport` 三处播报 + CLI 落盘 `data/translate_report.json` | 失效不再静默 |

> ⚠️ **§4 是 §5 / §10 / §11 / §13 的前置。** 不做它，后面每项都可能"静默少画"而无人发现。

### 批 2 · 最便宜（6 项）

| # | 名称 | 状态 | 证据 | 备注 |
|---|---|---|---|---|
| **16** | 运镜速度 `speed` | ✅ **已实现** | `Event::Camera.speed`（`Option<String>`，`skip_serializing_if`）；`cameraTransitionFrames`（转译器）+ `cameraTransitionSecs`（预览）**同口径**；守卫 12 项 + 5 项对拍 + 4 处变异 | `snap 0.1 / fast 0.25 / slow min(1.5, dur×0.8)`；**`med` 与缺省同走老式子**（存量零回归） |
| **17** | 跳切手法 | ✅ **已实现** | `find_jump_seams` / `cap_jump_cuts`（`process/fx.rs`）；**零新字段**（规格方案 A：`cut` 落在 `mute_ranges` 边界 = 跳切）；7 条单测 + 4 处变异自证 | 两道配额：单段 ≤2 / 相邻 ≥20s；`process` Step 1e 与 `llm` Step 4b **成对**；转义：超额的 `cut` 降级为平滑（**不删事件**） |
| **18-C** | 暴露运镜档位配置（§18 的前置） | ✅ **已实现** | 后端 `camera` patch 键（夹取 中/近 `[1,3]`、画中画 `[0.05,1]`）+ 前端三根滑杆；新门禁 `check-camera-zoom-parity.mjs`（五处逐值一致，**8 处变异**） | **只暴露、默认值一个没动** ⇒ 存量零回归 |
| **18-A** | 景别「特写」档 | ⏳ **待你定系数** | 方案 A 已定（加第 4 档、`近` 保持 1.5），**系数 1.8 / 2.0 未定** | 现在可以直接拖编辑器里的近景滑杆试（1.0~3.0），定了数再做 A（要同步 5 处 + 露边实测） |
| **20** | 真黑白噪点 | ⛔ **被阻塞**（前置验证已做完） | **真机实测**：底座 `noise` 在**任何合法强度**下都画不出（`amount≤1` 加在 0..255 数据上 ⇒ 最大增量 0.5/255 < 1 个量化台阶）⇒ 输出与不挂 `noise` **逐字节相同**。自证用例已留在 Dhampir 仓（会红） | **必须先修底座量纲**，再做 ②③；三个选项见保守决策 #25 |
| **22** | 音效淡入 | ✅ **已实现** | `ffmpeg::sfx_filter_chain`（纯函数，可单测）；两端共用 `SFX_FADE_SECS`，淡入夹到素材长度 | 消除起点爆音 |
| **24** | 弹幕逐条颜色 | ❌ **不做**（原判据有误） | **不是缺口，是有意为之**：`stripColorTags` 主动摘掉 `\c`（`85d7fc0`，2026-10-01）。参照是本仓 canvas，`getActiveDms` 只回 `{text,opacity,x,y}`、渲染只用**轨道级** `dmS.color` ⇒ 发了会让成片**比参照更花**（实测 48.8s 那帧：参照白 / 底座品红） | 见下方「§24 更正」 |

### 批 3 · 判据与叙事（4 项）

| # | 名称 | 状态 | 证据 | 备注 |
|---|---|---|---|---|
| **5** | ⭐ `elements` 元素系统 | 🚧 **进行中**（E1/E3/E5 ✅） | **E1** `Event::Element`（kind 分派，`Sticker` 变体未动）；**E3** 求解器支持动态盒（`BoxSize`）；**E5** 提示词 + 类型推断 | 剩 **E2**（sticker 迁入）、**E3 接调用方**（排版盒）、**E4a** 动效 T1–T4/T6；**E4b T5 真卡拉OK 仍被阻塞**（缺词级时间戳） |
| **6** | 爆点判据多信号 | ✅ **内核+接线** | `process/punch.rs`：一份判据（字幕 0.6/弹幕 0.2/音效 0.2 + `fx` 加成）、三路信号适配层、`cmd_process` Step 1f 接线并播报；13 条判据 + 3 处变异 | 无弹幕时**明说**降级（规格 §2 规则 2）—— 实测同一工程去掉弹幕文件后判出**同样两处**爆点；⚠️ **消费侧（§7/§8）待做**（决策 #45：本趟不改事件） |
| **7** | 静默分两类 | ✅ **已实现** | `SilenceKind`/`classify_silence`/`split_silence_by_kind`（`project/silence.rs`）+ `apply_silence_with` 剔除保护段 + `service` 日志报数；5 条判据 + 2 处变异 | 复用 §6 的爆点判据（**没有第二套**）；**没爆点时一个都不保护** ⇒ 存量行为逐字节不变 |
| **8** | `role` 叙事角色 | 📐 | `role` **0 命中** | 建议先做三档 |

### 批 4 · 新形态（6 项）

| # | 名称 | 状态 | 证据 |
|---|---|---|---|
| **19** | 荷兰角 `rotation` | 📐 | `rotation` **1 处命中是数组轮转**（`process/mod.rs:163`），**不是**荷兰角；`HandheldSway.rot` 已在用、转译器已发关键帧，**但无 Event 变体** |
| **21** | 音效闪避 | 📐 | 已有"响度跟随"（视频响度 −8dB），只差粒度从**点→段** |
| **23** | 节奏约束求解器 | 📐 | `solver` **0 命中** |
| **9** | 多版本成片 | 📐 | `unique_output_path` 已支持 `_1`/`_2` |
| **10** | 情绪轨道 | 📐 | 弹幕颜色/文本/模式**三个信号全浪费** |
| **11** | BGM | 📐 | 全仓每处 "BGM" 都是"**把 BGM 从 ASR 里排除**" |

### 提需求给底座（2 项）

| # | 名称 | 状态 | 证据 |
|---|---|---|---|
| **25** | 文字层三字段 | 🟡 **部分已满足**（见 §3 更正 ①） | `entrance` **已被有意下线**，不是"缺失" |
| **14** | C 层五项 | 📐 提需求 | 方向模糊/RGB 分离/VHS/冲击帧/真移焦 |

### ❌ 不做（3 项）

独立位移（底座整帧缩放，平移必露边**实测 622px**）· 划像/遮罩转场（切片靠硬切活着）· 定格 `freeze`（用户搁置）。

---

## 3. ⚠️ 三处更正（本文核实，与上游文档不符）

### 更正 ① `SubtitleStyle` 的 `stroke` / `highlight_color` **不是缺失**

`elements-spec.md` §3 依据 `docs/dhampir/p9-downstream-requirements.md` §2.2 写道：
`stroke` **只有 `outline: bool`**、`highlight_color` **无**。**这份结论已过期。**

**实际（`crates/yekitrim-render-core/src/types.rs:503–522`）：**

```rust
pub struct SubtitleStyle {
    pub font: String,
    pub size: u32,
    pub color: String,
    pub stroke: String,          // ← 不是 bool，是字符串 "12px #403c3b"
    pub highlight_color: String, // ← 存在，默认 "#f56b41"
    pub position: String,
    pub position_pct: Option<f64>,
}
```

且 **`highlight_color` 全链路有消费者**（不是"字段有、接线无"）：

| 腿 | 位置 |
|---|---|
| 转译器 | `tools/polish-to-dhampir.mjs:4554–4559`（发 `[245,107,65,255]`） |
| DOM 预览 | `webui/src/components/PolishPreview.vue:2286`（`--hl-color`） |
| 封面 | `webui/src/components/CoverDesigner.vue:555 / 1912`（渐变底色由它推导） |

**⇒ 对 `elements` 的影响**：E4 里 **T3 强调放大**（依赖 `highlight_color`）**不需要等底座**，
现在就能做。**T5 真卡拉OK** 仍需词级时间戳（那是本仓侧的数据缺口，不是底座字段缺口）。

### 更正 ② `entrance` 是**被有意下线**，不是"底座没做"

`elements-spec.md` §3 与 §9-E4 把 `entrance` 当作"等底座交付的字段"。**实际相反**：

`crates/yekitrim-polish/src/project/legacy.rs:19–38` 明确记录它**已从契约里移除**：

> `entrance` 是 vtedit 时代的"字幕入场动画"，默认值 `"cycle-5"`，
> `polish init` 模板还替每个新工程写 `entrance = "fade-up"` —— 但**全仓没有任何读者**。
> 要真做就得**发明**一套动画规格 —— 没有参照，所以选择**从契约里移除**。

现状：`strip_retired_keys()` 在**读时吞掉**它（因为 `deny_unknown_fields`），并有单测
`retired_entrance_key_is_swallowed` 钉住。

**⇒ 对 `elements` 的影响（重要）**：E4 的动效**不能表述为"等底座补 `entrance`"**。
真实路径是二选一：
1. **本仓逐帧近似**（`elements-spec.md` §4.1 已定硬约束：**必须逐帧算，不许 CSS transition**）
   —— 弹入 / 打字机 / 抖动 / 强调**都能走这条**，且**不等任何人**；
2. 若要底座侧的原生入场动画，那是**新提需求**（要连规格一起发明），**不是"补一个已有字段"**。

**⇒ `elements` 的实施顺序因此更正**：原规格把 E4 整块挂在"等底座"上；实际上
**E4 里只有 T5（真卡拉OK）确实要等**，**T1/T2/T3/T4/T6 全部本仓自足**。

### 更正 ③ §17 第 4 问（`offset_secs` 固定还是每场调）**已经有答案了**

`polish-upgrade-master-plan.md` §17 把它列为待拍板。**实际它早已是用户可调控件**：

| 层 | 位置 |
|---|---|
| 类型 | `types.rs:622`，默认 `-3.0` |
| 服务端白名单 | `crates/yekitrim-server/src/api/polish.rs:1805–1812`，**clamp −10.0..10.0** |
| UI | `webui/src/views/PolishView.vue:809 / 1823`（滑杆） |
| 预览 | `webui/src/polish/engine.js:377` |

**⇒ 该问已消解**：既不是"固定"，也不是"每场手工估" —— 已经是**每工程可调**。
§6 的"滞后校正"因此**降级为可选增强**，不是必做项。

### 更正 ④ §24 弹幕逐条颜色 —— **"断点"是有意焊死的，不是缺口**

上游（`editing-techniques-catalog.md` §0.4 / `camera-vfx-editing-catalogue.md` §9）把它列为
「本仓单方面就能修」的接线缺口，原话是「**底座的逐条弹幕颜色能力是好的，断点在转译器**
（不生成 `\c` 覆盖）」+「修法很小」。**两句都不成立。**

核实（2026-10-10，三条独立证据）：

| # | 证据 | 结论 |
|---|---|---|
| 1 | `getActiveDms` 的出口是 `out.push({ text, opacity, x, y })`（`templates/index.html:1565`）；渲染只用**轨道级** `dmS.color`（同文件 `2125`） | 参照（本仓 canvas）**根本不消费**逐条颜色 |
| 2 | `tools/polish-to-dhampir.mjs:4698` 的 `stripColorTags` **主动摘掉** `\c&HBBGGRR&` | 现状是**有意为之**，不是"还没做" |
| 3 | 该行为在 `85d7fc0`（**2026-10-01**）里**随"底座成为默认出片通道"一起加入**（该提交 diff 有 4 处 `stripColorTags`） | 与底座接入**同批**下的决定，不是遗漏 |

**为什么"修了"是错的**：那是一次**实测事故**的结论 ——「48.8s 那一帧 YekiTrim 是白的、底座是品红的」。
转译器早期版本**确实发过** `\c`（当时把 `danmaku.json` 的 `color` 字段当成"本仓需要逐条颜色"的证据），
发出去之后**成片比参照更花** —— 因为**参照自己不上色**。

⚠️ 顺带纠正原判据的另一半：它说「颜色已解析成 `#rrggbb`，但**零生产消费者**」——
**解析是真的**（`yekitrim-core/src/danmaku/mod.rs:93` 的 `extract_color` → `DanmakuEntry.color`），
但那个 `color` 服务的是 **ASR 纠错 / 切片**那条链，**不是** ASS 渲染那条链。
两条链共用同一个结构体，所以"字段有值"看着像"渲染该用它"，实际不是。

**⇒ §24 由 📐 改为 ❌ 不做。** 将来真要弹幕上色，那是一个**新需求**（要同时改参照渲染器
与底座两侧、让两边同口径），**不是**"把摘掉的那段加回来" —— 加回来只会复现那次 48.8s 的偏差。

---

## 4. 两个被阻塞的项（细节）

### 4.1 §18 特写 —— 卡在"先做 C"

```
方案 A（加"特写"档）        ✅ 已定（用户："3.A."）
        ↓ 被阻塞
方案 C（暴露配置实测倍率）    ❌ 未做  ← 我核实：zoom_* 在 UI/白名单里零命中
        ↓ 才能定
特写系数 = 1.8 还是 2.0？    ❓ 待定
        ↓ 还要顺带定
档位 × anchor 组合约束        ❓ 待定（特写时"聚焦弹幕"会看不到弹幕）
```

**§3.2 的 5 处同步**（漏一处就静默走 `_ => zoom_mid` ⇒ 点"特写"出来是**中景**、不报错）：

| # | 位置 | 现状 |
|---|---|---|
| 1 | `crates/yekitrim-render-core/src/types.rs:408` `factor()` | 真源 |
| 2 | `crates/yekitrim-polish/templates/index.html:606` | canvas 出片 |
| 3 | `webui/src/polish/engine.js:433` | DOM 预览镜像 |
| 4 | `tools/polish-to-dhampir.mjs:1238` | 转译 |
| 5 | `crates/yekitrim-polish/src/process/mod.rs:1002` | 内存↔模板 diff 表 |

⚠️ `factor()` 的注释自己写着这个坑：**"四个渲染/求值路径都必须走同一张表，
否则「预览看到的不是成片」——1.5/1.8 的硬编码就是这么来的"**。
另加**露边风险**（622px 实测是 `scale=1.3`；`1.8` 会更严重）+ `cap_handheld_sway` 现在**只允许 `近`**。

### 4.2 §5 `elements` —— 只有 E4 的 T5 被阻塞（更正后）

| 步 | 内容 | 能立刻做吗 |
|---|---|---|
| **E1** | 骨架（`kind` 分派、词汇表四锚点、lane 沿用 `sticker`） | ✅ |
| **E2** | `elements.sticker` 迁进来（验证骨架不破坏存量） | ✅ |
| **E3** | `elements.text` **静态版**（`style` 预设、落点动态盒、折行复用） | ✅ **不依赖底座** |
| **E4a** | 动效 **T1/T2/T3/T4/T6**（弹入/打字机/强调/抖动/底衬） | ✅ **逐帧近似即可**（更正 ②） |
| **E4b** | 动效 **T5 真卡拉OK** | ⏳ 需**词级时间戳**（本仓数据缺口） |
| **E5** | LLM 提示词：教 LLM 用 `elements.text` | ✅ |

⚠️ **主要工作量不是白拿**：`placement.rs` 按 **`280 × SK` 固定盒**算障碍，
文字是**变宽高** ⇒ 落点求解器必须支持**动态盒**。不做 ⇒ **"大字压住主播脸"**。

⚠️ 座位表：`lane` **沿用 `sticker`**（只改显示标签为「元素」）⇒ **老工程零迁移**。

---

## 5. 施工批次（按"可开工性"重排，非按重要性）

> **重排理由**：原批次把被阻塞的项混在一起。本文按**能不能立刻动手**分层。

### 批 1 · 地基（4 项，📐 全可开工）

| 顺序 | 项 | 为什么这个位置 |
|---|---|---|
| 1.1 | **§4 转译报告可见** | **前置的放大器** —— 不做它，后面每项都可能静默失败 |
| 1.2 | §2 句末边界守卫 | 独立，极低成本 |
| 1.3 | §1 决策理由 | 依赖 §4 的展示面 |
| 1.4 | §3 质量分 | 依赖 §2 的判据 |

### 批 2 · 最便宜（📐 先做无阻塞的）

| 顺序 | 项 | 阻塞 |
|---|---|---|
| 2.1 | §22 音效淡入 | 无（一行） |
| 2.2 | §24 弹幕颜色 | 无 |
| 2.3 | §16 运镜速度 | 无 |
| 2.4 | §17 跳切 | **依赖 2.3** |
| 2.5 | §20 真噪点 | 需先查底座 `noise` 参数 |
| 2.6 | **§18-C 暴露 `zoom_*` 配置** | 无 —— **先做 C 才能定特写系数** |
| 2.7 | §18-A 特写档 | **依赖 2.6 实测出系数** |

### 批 3 · 判据与叙事

| 顺序 | 项 | 说明 |
|---|---|---|
| 3.1 | §5 `elements` **E1–E3 + E4a + E5** | 本仓自足（更正 ②） |
| 3.2 | §6 爆点多信号 | §8 的前提 |
| 3.3 | §7 静默分类 | —— |
| 3.4 | §8 `role` 三档 | 依赖 3.2 |

### 批 4 · 新形态

§19 荷兰角 · §21 闪避 · §23 节奏求解器 · §9 多版本 · §10 情绪轨道 · §11 BGM。

### 提需求

§25（**只余 `entrance` 与描边宽度** —— 见更正 ①）· §14 C 层五项。

---

## 6. 红线（施工时不得违反）

| # | 红线 | 后果 |
|---|---|---|
| **R1** | 新效果 = ①词汇表一行 ②转译器一个 `case` ③**底座有原语** | 本仓**不能**再靠"自己多画一笔"落地 |
| **R2** | "直播间怎么排版"归本仓；"像素怎么算"归底座 | 大字/章节卡是本仓；真噪声是底座 |
| **R3** | 增删效果只改 `spec/effects.json`，跑到门禁绿 | 漏改会被点名 |
| **R4** | **存量零回归**（横屏逐字节） | 新字段一律 `serde(default, skip_serializing_if)` |

**当前词汇表 11 个效果**：`camera` · `sticker` · `overlay` · `sfx` · `shake` · `flash` · `blur` · `vignette` · `hue_shift` · `noise` · `pulse`。

⚠️ **`elements` 不可叫 `overlay`** —— 该 id 已被"氛围色罩"占用。

---

## 7. 四条贯穿全局的技术事实（很多判断由此而来）

| # | 事实 | 后果 |
|---|---|---|
| 1 | 底座**整帧缩放**，平移必露边（**实测 622px**） | pan/tilt/dolly **全部退化为 `reframe`** |
| 2 | **变焦 ≠ 移动** | dolly zoom **数学上不可能** ⇒ 不提需求 |
| 3 | **音轨全归本仓**（`--no-audio`） | J/L-cut、闪避、淡入、BGM **都不需底座改** |
| 4 | **无 BGM，全是直播原声** | 卡点**前提缺失** ⇒ §11 正确表述是"先引入 BGM" |

> 📌 **教训（最便宜的一类）**：**提需求之前，先核"是不是本仓自己没接"。**
> 三例：`zoom_bounce`（转译器里**只在注释**，发出 0 次）、`saturation`（只用于模糊背景层）、
> `rotation`（关键帧已发、**无 Event 变体**）。
> **本文的更正 ① 正是这条教训的第四次命中** —— `highlight_color` 一直被当成"底座缺失"，
> 其实本仓早就有、早就在用。

---

## 8. 本文档的维护规矩

1. **状态列只认代码证据**，不认上游文档的 ✅。改状态必须附**文件:行**或 **grep 结论**。
2. **发现上游文档与代码不符 ⇒ 改本文的"更正"章节**，不直接改上游（上游是决策记录，要保真）。
3. 每完成一项：把状态改成 ✅ 并补证据；**若实测推翻了某条判断，写进 §3**。
4. ⚠️ **`grep` 命中必须人工判真伪**（本文已剔除 `reason` / `rotation` 两处假阳性）。

### 本次核实的命令（可复现）

```bash
cd <repo>
# 批量查各关键词在 polish 里的命中文件数
for k in reason boundary smoothness 句末 elements role silence bgm rotation dutch solver; do
  printf "%-12s %s\n" "$k" "$(grep -rl "$k" crates/yekitrim-polish/src/ | wc -l)"
done
# 假阳性判真伪
grep -rn "reason" crates/yekitrim-polish/src/ | head
grep -rn "rotation" crates/yekitrim-polish/src/ | head
# 两处更正
sed -n '503,522p' crates/yekitrim-render-core/src/types.rs   # stroke/highlight_color 存在
sed -n '19,38p'   crates/yekitrim-polish/src/project/legacy.rs # entrance 有意下线
# 特写阻塞项：zoom_* 是否暴露
grep -rn "zoom_close\|zoom_mid\|zoom_wide" webui/src/ crates/yekitrim-server/src/api/polish.rs
```
