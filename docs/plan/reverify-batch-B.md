# 重验 B：§5 `elements` 元素系统（三列核实）

> **只读审计**，未改任何代码。仓：`F:\para\Code\YekiTrim`。
> 起因：`polish-upgrade-consolidated.md` 的 25 项总表**只用了单列 ✅**
> （判据 = "代码在 + 有消费者"），**这个判据掩盖了"有消费者、没有生产者"**
> （实例：`§18-A 特写` 标 ✅，但 `llm/segment.rs` 的 zoom 选项里根本没有「特写」）。
>
> ⇒ 本次按**三列**重核：**① 代码 ② 后端接线（生产者→消费者全程） ③ 前端（webui UI + 预览）**。

---

## 0. 先回答用户的三个原话问题

用户原话（**这是本次验收标准**）：

> 「**没有看到新轨道，贴纸也没有和文字层合并，也就是没有实现文字层的功能**」

### 0.1 「没有看到新轨道」 —— ✅ **用户是对的，这是设计如此，但有个真问题**

**"不新增轨道"是计划明确写的**，不是漏做：

| 证据 | 内容 |
|---|---|
| `docs/plan/elements-spec.md:28` | 「**轨道：沿用 lane key `sticker` 不改**，只把**显示标签**改成「元素」」⇒ **老工程零迁移** |
| `docs/plan/elements-spec.md:214` | 「`lane` \| **沿用现有 `sticker`**（只改显示标签为「元素」）」 |
| `docs/plan/polish-upgrade-consolidated.md:239` | 座位表同样声明 |

**后端确实没新增轨道**（✅ 与计划一致）：

```
crates/yekitrim-polish/src/project/config.rs:1002
    "element" => Some(Lane::Sticker),        ← 复用 sticker 轨
```

**⚠️ 但"只改显示标签为「元素」"这半句 —— 只有注释，没有代码：**

```
crates/yekitrim-polish/src/project/config.rs:998
    // 元素（§5）沿用 sticker 轨（规格 §7 座位表：只改显示标签为「元素」）
```
这是**注释**。**整条链路上没有任何地方把那枚显示标签真的改成「元素」。**

**用户实际看到的是「贴纸」**：

```
webui/src/components/TimelineEditor.vue:758
    { type: 'sticker', label: '贴纸', lane: 'sticker', ... }
                                     ^^^^^^ 轨头显示的就是这两个字
```

> **⇒ 用户说的"没有看到新轨道"字面为真**：轨头仍写「贴纸」。
> **但这不等于功能没实现** —— `element` 事件**会**落进这条轨（见 0.3 的 `LANE_TYPES`）。
> **真正的问题（比"忘改标签"重得多）**：**这条轨现在混装两种完全不同的东西**
> —— 贴纸图（`kind=sticker`）与文字（`kind=text`），**但轨头只说"贴纸"**。
> 用户看到文字出现在一条叫「贴纸」的轨上，合理解读就是"贴纸和文字层没分开/没实现"。

**⚠️ 另有一处口径分裂**：`spec/effects.json` 里 `element` 的 `label` **是**
「元素（文字 / 贴纸）」：

```json
{ "id": "element", "label": "元素（文字 / 贴纸）", "lane": "sticker", ... }
```
它经 `TimelineEditor.vue:739` 进 `TYPE_LABELS`，**用于"块/事件"的显示名**；
而**轨头**（`LANE_DEFS`，`:758`）用的是**手写的**「贴纸」。
⇒ 同一屏上可能出现"轨头写贴纸、里面的块写元素" —— **两处标签不是一个来源**。

### 0.2 「贴纸也没有和文字层合并」 —— ⚠️ **"合并两层"从来没在计划里**

**明确答复：没有。计划从头到尾是"`kind=sticker` 复用贴纸路径 + `kind=text` 另走排版"，
是"**一个事件类型、两个子类型**"，不是"把两条轨合并成一条"。**

| 证据 | 内容 |
|---|---|
| `docs/plan/elements-spec.md:1` | 「`elements` 元素系统（**文字 + 贴纸**）」—— 两个子类型 |
| `docs/plan/polish-enrichment-proposal.md:63` | 「元素系统：`elements`（顶层零件），**两个子类型 `sticker` / `text`**」 |
| `config.rs:1002` | `element` → `Lane::Sticker`（**复用轨**，非"合并两层"） |

⚠️ **但"用户为什么会这么以为"是有道理的**：既然 `element` 与 `sticker` **共用一条轨**、
**共用一个眼睛开关**（`LANE_OF_TYPE` 里两者都 → `sticker`）、轨头还叫「贴纸」——
**从用户角度看，这两者本来就"已经在一起"了，只是看起来没做**。
⇒ 这不是"用户预期与计划不符"，而是**计划的措辞（"只改显示标签"）没有落地，
导致共用一条轨这件事对用户不可见**。

### 0.3 「没有实现文字层的功能」 —— ⚠️ **后端实现了；把它显示出来/预览出来没实现**

**`kind="text"` 的后端全链路是通的**（详见 §E3）：

```
process  Step 1d3  layout_text_elements   crates/yekitrim-polish/src/process/mod.rs:1684
                   → 写回 event.source      mod.rs:1732
转译器             case 'element'          tools/polish-to-dhampir.mjs:3552
                   → 每条文元素建独立 ASS 资产 + 单层字幕轨
底座               text_* 字幕轨 → 真出图（T6 已由 `7401173` 全链路复核过）
```

**但"用户看得见"的那两处都不通**：
- **前端 UI**：`grep -rln "element" webui/src/views/ webui/src/components/` → **0 命中**
  （**没有任何**元素事件的编辑控件/展示）
- **预览**：`engine.js:890` **明确 `continue` 掉 `kind !== "sticker"`**（见 §E3 第③列）

⇒ **用户说"没实现文字层的功能"，从"我能看见、我能操作、我能预览"的角度是准确的。**

### 0.4 ★ 关于「LLM 会不会真的产出 `kind="text"`」 —— **未验，且已有的"实测"证据不成立**

总表 `polish-upgrade-consolidated.md:72` 写：

> **E5** 提示词 + 类型推断（`d349445`，**实测 LLM 产出 `kind="text"`**）

**我核了这条"实测"，它没有可追溯的现场：**
1. `git show d349445` 的提交说明里**没有任何"实测/真跑 LLM"的记述** —— 它只讲了提示词与推断规则。
2. 该提交的测试（`segment.rs:522` 起）验的是**类型推断**：
   给一个**已经带 `kind` 的 JSON**，断言它被归成 `element`。
   **这不等于"LLM 自己会产出 `kind`"** —— 前者是解析侧兜底，后者是提示词侧的产出率。
3. `polish-upgrade-conservative-log.md:281`（条目 199）确实出现过
   `type="element" kind="text"` 的工程 —— 但那是**手写的**：
   「在本仓**造**正规工程（`[[segments.events]] type="element" kind="text"`）」
   ⇒ 那是**人写进去**的，不是 LLM 产的。

**⚠️ 而提示词侧的表述确实偏软**（Lead 的怀疑成立，我逐行核过）：

```
crates/yekitrim-polish/src/llm/segment.rs:113
  ⚠️ 整段最多 1~2 条。文字元素很抢眼, 写多了整片都是大字 → 反而没有重点
```

- 这是**上限**（"最多"），**不是下限**。
- `element` 与 `camera`/`sticker`/`视觉特效`/`overlay` 并列在**同一张平铺的可选清单**里
  （`segment.rs:80-118`），**没有一处说它是必须的**。
- **对比证据（说明作者会写下限）**：同一份提示词里 `punch` 写的是
  `segment.rs:363`「**整片至少要有一段**」；`camera` 的切换配额写的是
  `segment.rs:135`「移过去之后**必须**移回来」。
  ⇒ **"必须/至少"这套词在这份提示词里是存在的、用得是很熟的 —— 唯独没用在 `element` 上。**

> **⇒ 结论：`kind="text"` 属于「有消费者、**生产者是纯可选项且无下限**」——
> 与「特写」的坑**同类但更隐蔽**：**
> 「特写」是提示词里**根本没有**这个词；
> `kind="text"` 是提示词里**有**、但**只被允许、不被要求**。
> **两者在产品里的结果完全一样：LLM 完全可以整片一条都不产出，而所有门禁/判据全绿。**
>
> **⚠️ 我方未验的部分（如实说明）**：我没有跑真实 LLM 去测产出率
> （需要真实网络 + 额度，且本轮为只读审计）。
> ⇒ **"LLM 实际产出率是多少"这条我标「未验」**，不写"应该可以"。
> 但上面 3 条足以推翻总表里那句"**实测** LLM 产出 `kind="text"`"的**口气**
> —— 它最多是"提示词教了、推断兜住了"，**不是**"测过它会产"。

---

## 1. 逐子项三列核实

**图例**：✅ 通 ｜ ⚠️ 部分/有前提 ｜ ❌ 不通 ｜ 🚫 未验

### E1 —— 骨架（`Event::Element`、kind 分派、词汇表、lane 沿用）

| 列 | 结论 | 证据 |
|---|---|---|
| ① 代码 | ✅ | `project/config.rs:1002` `"element" => Some(Lane::Sticker)`（`Lane::of_event`）；`Event::Element` 变体 + `#[serde(default)]` kind（`d349445`）；词汇表 `spec/effects.json` 有 `element` 条目（`lane: sticker`） |
| ② 后端接线 | ✅ | 三份镜像同集合：Rust `config.rs:1002` ／ `webui/src/polish/engine.js:308` ／ `tools/polish-to-dhampir.mjs:2575`（各有一条守卫钉着：`check-yekitrim-translator.mjs` / `check-preview-engine.mjs`） |
| ③ 前端 | ⚠️ | **事件能落进轨**（`TimelineEditor.vue:750` `LANE_TYPES('sticker')` 会 filter 出 `element`，因为它 `lane==='sticker'`）；**但轨头标签仍是「贴纸」**（`:758`）⇒ 与计划"改成「元素」"**不符** |

**断在哪**：代码与接线都通；**断在"显示标签"这半句只有注释没有实现**（`config.rs:998` 是注释）。

### E2 —— `kind=sticker` 复用贴纸路径

| 列 | 结论 | 证据 |
|---|---|---|
| ① 代码 | ✅ | 翻译器在进 `switch` **之前**把 `element+kind=sticker` 归一化成 `type='sticker'`（`polish-to-dhampir.mjs:3075`）⇒ 并入既有 `case 'sticker'` |
| ② 后端接线 | ✅ | "零复制"成立：不新增求值路径。预览侧 `engine.js:876-884` 同口径复用 `stickersAt` |
| ③ 前端 | ✅ | 预览走贴纸那条已验通道；弹出/上浮/淡出与模板四份逐值对齐（`engine.js` 注释声明） |

**断在哪**：**没断**。这是 §5 里做得最实的一块。
⚠️ 但注意：**"复用"≠"合并"** —— 见 0.2，用户理解的"合并两层"从来没在计划里。

### E3 —— `elements.text` 静态版（四档 style / 落点盒 / 折行）

| 列 | 结论 | 证据 |
|---|---|---|
| ① 代码 | ✅ | 排版盒：`process/mod.rs:1684 layout_text_elements`；四档样式：`render/elements.rs` + `polish-to-dhampir.mjs:3552 parseTextStyle`；折行与落点写在 `event.source` |
| ② 后端接线 | ✅ | `process` Step 1d3 算出盒子 → 写回 `event.source`（`mod.rs:1732`）→ 转译器 `case 'element'` 建**独立 ASS 资产 + 单层字幕轨** → 底座出图。T6 已由 `7401173` 走完整 `process → 转译 → 出片` 复核过（`conservative-log.md:281`） |
| ③ 前端 | ❌ | **两处都不通**：① **UI 无控件**（`grep -rln "element" webui/src/views/ webui/src/components/` = **0**）；② **预览明确跳过**：`webui/src/polish/engine.js:890` `if (kind !== 'sticker') continue`，且 `:884` 注释自认「`kind === "text"` **返回空**」 |

**断在哪**：**断在前端**。后端能出片，**但用户在 UI 上既没有入口、预览里也看不到**。
⚠️⚠️ **这是"预览/成片分叉"**（本仓明令禁止）：
成片里 `kind=text` 画得出字，预览里**一片空白**。
⚠️ **附带发现**：`getElements()`（`engine.js:886`）**是死代码** ——
`grep -rn "getElements" webui/src/` **只有它自己的定义那一行**，**没有任何调用方**。
⇒ 就算它支持 text，预览也不会调它。

### E4a —— 动效 T1 弹入 / T2 打字机 / T3 强调 / T4 抖动 / T6 底衬

| 子项 | ① 代码 | ② 后端接线 | ③ 前端（预览） |
|---|---|---|---|
| **T1 弹入** | ✅ `render/elements.rs:115 scale_in_from` | ✅ `config.rs:2149` 解析 → 转译器 `polish-to-dhampir.mjs:2774 put('scale_in_from', …)` → 底座 `903ed85` 通道 | ❌ 预览无对应实现 |
| **T2 打字机** | ✅ `render/elements.rs:311 split_typewriter` | ✅ 转译器读 `[elements] typewriter`（`:5544`） | ❌ **`grep -rn "typewriter" webui/src/` = 0 命中**（Lead 给的线索**核实为真**） |
| **T3 强调** | ⚠️ 半截：换色 ✅ / 放大 **故意留空** | ⚠️ 无触发源（`render/elements.rs:21` 注释自认「本仓不接」） | ❌ 无 |
| **T4 抖动** | ✅ `shake_px/shake_hz/shake_phase` | ✅ `config.rs:2152` + 转译器 `:2778` | ❌ 预览无对应 |
| **T6 底衬** | ✅ `plate_color` | ✅ `config.rs:2154` + 转译器 `:2784`；**全链路真机复核过**（`7401173`，红像素 12011~20279） | ❌ 预览无对应 |

**断在哪**：**T1/T4/T6 断在前端**（后端已通）；**T2 前端为 0**；**T3 断在触发源**（有意留空）。
⚠️ **总表把 E4a 标成 `(T1·T2·T4·T6)` 完成 —— 按三列看，
这四项在"③ 前端"这一列全是 ❌，**而 `spec/effects.json` 里 `element` 的
`canvas: true, dom_preview: true` **宣称预览支持** —— **这两个 flag 与实际不符**。

### E4b —— 字级时间戳搬运

| 列 | 结论 | 证据 |
|---|---|---|
| ① 代码 | ✅ | `crates/yekitrim-polish/src/parse/charlog.rs`（15 单测 + 3 端到端，`conservative-log` 记载） |
| ② 后端接线 | ✅ | 消费侧 `process/mod.rs:1309 load_char_timeline(project_path)` |
| ③ 前端 | 🚫 | **未验**：本轮未核预览是否消费字级时间戳（不在 Lead 给的清单里，且时间有限）—— 如实标未验 |

### E5 —— LLM 提示词

| 列 | 结论 | 证据 |
|---|---|---|
| ① 代码 | ✅ | `llm/segment.rs:99-113` 教了 `element` + 四档 style；推断规则 `:296-307` |
| ② 后端接线 | ✅ | 推断排 `keyword` **之前**（`:294` 注释 + 用例）；`Element.kind` 加 `#[serde(default)]`（漏写不炸整份工程） |
| ③ 前端 | ⚠️ | LLM 产出的事件**会**进轨（同 E1），但**用户看不到它是什么**（轨头「贴纸」+ 无 UI） |

**断在哪**：⚠️ **"生产者是纯可选项且无下限"** —— 详见 §0.4。
**这一列是本项最大的风险**：所有判据全绿，而 LLM 可以一条都不产。

### T5 —— 真卡拉OK

| 列 | 结论 | 证据 |
|---|---|---|
| ① 代码 | ⚠️ | 配置项在（`render/elements.rs:199 karaoke`），但 `conservative-log`/总表自认：底座 `ass_text_to_plain` 只抽 `color`/`\move`，**真 `\k` 物理不通**；可行落法是逐字切 cue（复用 T2 通道） |
| ② 后端接线 | ❌ | **总表自认"真渲染仍未做"**；转译器侧有 `elements.karaoke` 的判据与报错（`polish-to-dhampir.mjs:5007-5074`），但**没有真实渲染通道** |
| ③ 前端 | ❌ | 预览无对应（同 E4a） |

**★ 但 `8d5a3f6` 那条已合并**：转译器切 SRT 轨的落法在仓里
（`render/elements.rs:170` 注释：「`karaoke`（T5） \| **SRT 字幕轨** \| 字级时间戳」）。
⇒ **代码在、通道在，但"真卡拉OK效果"未验/未完成** —— 与总表"仍未做"一致。

---

## 2. §25 文字层三字段

**先定位**：`polish-upgrade-consolidated.md:92` 的 §25「文字层三字段」= **`stroke` / `highlight_color` / `entrance`**
（三字段的展开在 §3 更正 ① 与更正 ②）。

**本轮逐字段独立复核（不引用计划的转述）**：

| 字段 | ① 代码 | ② 后端接线 | ③ 前端 | 断在哪 |
|---|---|---|---|---|
| **`stroke`** | ✅ `crates/yekitrim-render-core/src/types.rs:686 / :777` **是 `String`**（`"12px #403c3b"`），**不是 `bool`** —— 计划"更正 ①"成立 | ✅ 转译器 `polish-to-dhampir.mjs:5116 parseStroke(subStyle.stroke, 12, '#403c3b')` → 拆成底座的 `stroke_ratio`/`stroke_color` | ✅ DOM 预览 `webui/src/components/PolishPreview.vue:2297 WebkitTextStroke: s.stroke`、`:2204 capStrokeW` | **没断** |
| **`highlight_color`** | ✅ `types.rs` 同段存在，默认 `#f56b41` | ✅ 转译器发 `[245,107,65,255]` | ✅ 预览 `PolishPreview.vue:2298 --hl-color`；封面 `CoverDesigner.vue:1912 / :1264` | **没断** |
| **`entrance`** | ⏳ **已被有意移除**（不是"缺失"）—— `project/legacy.rs:36 sub.remove("entrance")`，`:82` 有单测 `retired_entrance_key_is_swallowed` 钉住"读时吞掉" | ❌ 无消费者（**全仓没有任何读者**，这是移除的理由本身） | ❌ 无 | **有意不做**（要做得先发明一套动画规格，没有参照） |

**⇒ §25 结论**：三字段里 **`stroke` 与 `highlight_color` 三列全通**（✅）；
**`entrance` 是被有意下线**（计划"更正 ②"成立，我复核为真）——
**它不是"等底座补"，而是"本仓主动移除"**。

⚠️ **一处容易看错的地方（我核了）**：`polish-to-dhampir.mjs:5814` 有一句
「`highlight_color` **本仓渲染器也不消费**」—— **那句是讲 `style.danmaku`（弹幕）的**，
不是讲字幕。**字幕的 `highlight_color` 三列都是通的**（上表）。
⇒ **别把这条 drop 说明误读成"字幕高亮没接"**。

---

## 3. 汇总表（三列）

| 子项 | ① 代码 | ② 后端接线 | ③ 前端 | 断在哪 |
|---|---|---|---|---|
| **E1** 骨架 | ✅ | ✅ | ⚠️ | 轨头标签仍「贴纸」（计划说改成「元素」，只有注释） |
| **E2** sticker 复用 | ✅ | ✅ | ✅ | 没断（但"复用"≠"合并两层"） |
| **E3** text 静态版 | ✅ | ✅ | ❌ | **前端无 UI + 预览明确跳过 text**（预览/成片分叉） |
| **E4a T1** 弹入 | ✅ | ✅ | ❌ | 预览无对应 |
| **E4a T2** 打字机 | ✅ | ✅ | ❌ | `typewriter` 在 `webui/src` = 0 命中 |
| **E4a T3** 强调 | ⚠️ | ⚠️ | ❌ | 无触发源（有意留空） |
| **E4a T4** 抖动 | ✅ | ✅ | ❌ | 预览无对应 |
| **E4a T6** 底衬 | ✅ | ✅ | ❌ | 预览无对应（成片已真机复核） |
| **E4b** 字级搬运 | ✅ | ✅ | 🚫 | 本轮未验 |
| **E5** 提示词 | ✅ | ✅ | ⚠️ | **生产者是纯可选项、无下限** ⇒ LLM 可一条不产 |
| **T5** 真卡拉OK | ⚠️ | ❌ | ❌ | 真渲染未做（总表自认） |
| **§25** `stroke` | ✅ | ✅ | ✅ | 没断（是 `String` 不是 `bool`） |
| **§25** `highlight_color` | ✅ | ✅ | ✅ | 没断（字幕那条；别与弹幕的 drop 说明混淆） |
| **§25** `entrance` | ⏳ | ❌ | ❌ | **有意下线**，不是缺失（`legacy.rs:36`） |

---

## 4. 三条我认为该单独立项的问题（只报告，未改）

1. **★ 预览/成片分叉（最重）**：`engine.js:890` 明确 `continue` 掉 `kind=text`，
   而成片画得出。`spec/effects.json` 里 `element` 却写着 `dom_preview: true`。
   **本仓明令禁止预览/成片分叉** ⇒ 这是**既存违规**，且**没有门禁在管**
   （`check-preview-engine.mjs:49` 只比对 `INDEXED_TYPES` 的**成员集合**，
   查的是"有没有登记"，**不是"登记了有没有真的画"**）。

2. **★ `getElements()` 是死代码**：`engine.js:886` 定义了但**全仓无调用方**
   （`grep -rn "getElements" webui/src/` 只命中定义行）。
   ⇒ 就算补上 text 支持，**预览也不会调它** —— 修的时候别只改这个函数。

3. **⚠️ `element` 的产出无下限**：见 §0.4。与「特写」同类，
   **区别是"特写"是词不存在、`kind=text` 是词存在但只被允许**。
   ⇒ 建议把这条列进"有消费者、没生产者"的清单里一并整治。

---

## 5. 未验清单（如实列出，不写"应该可以"）

| 项 | 为什么未验 |
|---|---|
| **LLM 真实产出率**（会不会真的产 `kind="text"`） | 需真实 LLM + 网络 + 额度；本轮为只读审计 |
| **E4b 的"③ 前端"列** | 不在本轮给定清单内，时间有限，未核 |
| **T5 `8d5a3f6` 的实际渲染效果** | 只核到"代码在、通道在"，**未跑真机出片**验证卡拉OK观感 |

---

## 6. 方法说明（可复核）

- 所有 `文件:行号` 均为**本轮实测**（`read` / `grep` / `git show`），非引用计划文档的转述。
- 「三列」判据的定义：**① 代码在**（有实现 + 有函数名）、
  **② 后端接线**（生产者写到哪、消费者读到哪，**全程**走通）、
  **③ 前端**（WebUI 控件 + **预览**，两处都要）。
- ⚠️ **本次没有改任何代码**；发现的缺口按上面的形式回报。
