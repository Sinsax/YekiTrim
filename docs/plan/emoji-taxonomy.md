# emoji 分类体系重做（多维交叉）与选取精度

> 状态：**方案，待拍板**（2026-10-17）。上游：用户口径「当前 emoji 包的类型有几十个，这根本不对，
> 应该进行多种交叉分类让 emoji 的选取变得精准」。

## 0. 结论

现在那 85 个 tag **不是分类**，是「7 个粗类 + 78 个某个表情的别名词」混在同一层：

| 事实（实测 `assets/emoji_catalog.json`） | 值 |
|---|---|
| 每条 emoji 平均挂几个 tag | **1.15** |
| 只挂 1 个 tag 的 emoji | **751 / 881** |
| 85 个 tag 里只覆盖 1 个表情的 | **32**（`alien` `bow` `cheer` `clap` `clown` `crown` `robot` `weapon`…） |
| 覆盖规模分布 | 1 条=32 · 2–3=17 · 4–6=19 · 7–15=6 · 16–40=4 · 40+=7 |

⇒ 它**筛不动**（每个"维度"上只有几条，用户点进去常常只看到 1 个表情），也**选不准**（见 §1 第 2 条）。

## 1. 现在 LLM 选贴纸是怎么算的（三处，逐条有出处）

1. **LLM 侧 —— 它被要求写什么**：`llm/segment.rs::sticker_lang_desc`
   —— emoji 来源下整段提示词只有一句：`(英文，如 "laugh" "cry" "shock" "game")`。
   **LLM 不知道词表**。`assets/sticker_index.rs::local_sticker_candidates_text` 只列
   **磁盘上已构建的包**（`ysyk  可用 tags: 笑(12) 生气(3)`），emoji 是内建来源、**不在其中**。
2. **解析侧 —— 谁接这个词**：`process/download.rs::try_resolve_sticker("emoji")`
   → `EmojiCatalog::search(kw, 8)`（`emoji_search.rs:236`）：
   - ① `index` 精确查（⚠️ `index` = **tag ∪ 官方短名**，所以 `game` 可能命中短名 `video-game`）；
   - ② 否则**子串 ⊇ 模糊**：凡 `tag.contains(kw)` 的组里的**全部** cp 都入选，
     **不评分、按目录顺序**取前 8；
   - ③ 否则 `FALLBACK_MAP` 硬编码兜底；
   - 最后取**第一个没被用过的**（`used_emojis` 去重是唯一让相邻贴纸不重样的机制）。
   ⇒ 关键词偏一点就选到无关表情；命中谁**由目录顺序决定**，同一关键词反复出现会连着取相邻条目。
   ⇒ 性能也不对：模糊档是 `881 × 85` 次 `contains`（每次贴纸事件都跑），而它本该是一次哈希命中。
3. **精挑侧 —— 有没有复核**：`llm/mod.rs::run_sticker_choice_step`（Step2b）
   只收 **`sticker_tag` 非空 + `load_package_meta(包名)` 命中的磁盘包**，
   再按 `(包名, tag)` 分组让 LLM 在候选里挑。
   ⇒ emoji 的 `keyword` 是英文词、不是包名，`sticker_tag` 也基本为空
   ⇒ **emoji 永远拿不到这一步精挑**（本地包有）。

**一句话**：精度差不是"标签不够"，而是 **LLM 在猜词 + 我们用子串去接 + 全程没有一步看候选**。

## 2. 目标模型：六维交叉，每维闭集

维度**必须正交**、每维取值**闭集**（LLM 只能选、不许造词），这样才能交叉查询与统计覆盖。

| 维度 | 取值数（约） | 取值示例 | 谁消费 |
|---|---|---|---|
| `emotion` 情绪 | ~25 | joy / sadness / anger / fear / surprise / love / calm / confusion / boredom / pride / awkward / gratitude | 选：情绪对齐；面板：主分组 |
| `intensity` 强度 | 1–3 | 1 轻轻 / 2 明显 / 3 爆点 | 选：爆点配 3、铺垫配 1 |
| `action` 动作 | ~20 | laugh / cry / clap / wave / bow / salute / shrug / point / dance / sleep / eat / think / phone | 选：与字幕动作对齐；面板：次分组 |
| `object` 题材 | ~20 | food / animal / vehicle / tech / sport / money / time / weather / body | 选：与话题对齐 |
| `usage` 用途（多选） | ~10 | reaction / punchline / emphasis / transition / cutaway / agreement / denial / meme | 选：**这一维最贴"剪辑"本身** |
| `tone` 极性 | 3 | positive / negative / neutral | 选：反讽/否定时不至于全选笑脸 |

**兼容做法**：`tag` 字段**保留**，改为**派生**（= 各维度取值的并集）。
⇒ `grouped()`、`EmojiCatalog::index`、编辑器面板、`/api/polish/emoji-catalog` **零迁移**；
新维度只加字段 `dim`（Rust 侧 `#[serde(default)]`）。

数据形状：

```json
{
  "name": "emoji_u1f602", "codepoint": "1f602", "url": "…/512.gif",
  "shortcodes": [":joy:"],
  "dim": { "emotion": ["joy"], "intensity": 3, "action": ["laugh"],
           "object": [], "usage": ["punchline", "reaction"], "tone": ["positive"] },
  "tag": ["joy", "laugh", "punchline", "reaction", "positive", "emotion", "happy"]
}
```

## 3. 选择链路的三处改动（与现有结构一一对应）

| # | 改哪儿 | 怎么改 | 为什么 |
|---|---|---|---|
| a | `llm/segment.rs` 的 emoji 分支 + 新增 `emoji_candidates_text()` | 把 §2 的**闭集词表**注入 Step2 的 system prompt（形状照抄 `local_sticker_candidates_text`）；要求 LLM 写 `sticker_query = "emotion:joy,usage:punchline,intensity:3"`（紧凑串，省 token） | **最大的一根杠杆**：现在它瞎猜词、我们靠子串接 |
| b | `EmojiCatalog::search` | 新增 `search_dims(query, max)`：按维度**打分排序**（各维权重 + intensity 距离）；旧 `contains ⊇` 保留为**兜底**（老工程/自由词仍能命中） | 从"目录顺序取前 8"变成"按贴合度排序"；顺带把 O(881×85) 的模糊扫描变成哈希 + 一次排序 |
| c | `llm/mod.rs::run_sticker_choice_step` | 分组键从 `(包名, tag)` 扩一档：**来源是 emoji 的事件单独成组**，候选 = `search_dims` 的 top-N（附表情字符 + 短名 + 维度词），让 LLM 选一个；prompt 与现有那条同形 | emoji 现在**完全没有复核**；本地包有，这不公平也不准 |

⚠️ 三处都**不动 `polish.toml` 的字段形状**（`keyword` / `sticker_tag` 仍是字符串）
⇒ 预览、出片、`tools/polish-to-dhampir.mjs` **零改动**。

## 4. 落地步骤（按可验证性排序）

1. **词表常量**（Rust 一份真源，prompt 与查询共用）：`assets/emoji_taxonomy.rs` 或
   `emoji_search.rs` 内的 `pub const DIM_VOCAB`。⚠️ 词表**不许散在 prompt 字符串里** ——
   改一处必须同时改查询，散开就必然漂。
2. **构建工具（开发期）**：`scripts/build-emoji-tags.mjs` ——
   读 `assets/emoji_catalog.json` → 分批（~30 条/次）喂 LLM → 产出 `dim` → **派生 `tag`** →
   原地写回（**先备份 + 写前校验**：每条都过词表合法性、每个维度都有非空覆盖）。
   ⚠️ 它是 `include_str!` 的**编译期输入**（`emoji_search.rs:182`）⇒ 跑完要 `cargo build`。
3. **Rust 解析 + 查询**：`CatalogEntry` 加 `dim`；`index`/`grouped()` 继续走 `tag`（零回归）；
   新增 `search_dims`。
4. **prompt 注入** + **精挑那一档**（§3a / §3c）。
5. **判据**（每条都要有变异自证）：
   - ① **覆盖**：每个维度的每个取值都 ≥1 条；
   - ② **闭合**：prompt 里 LLM 能写的每个取值，`search_dims` 至少要命中 1 条
     （否则又是"写了但选不出"）；
   - ③ **分布**：单维取值数落在预算内（目标：主维度每组 10–60 条，不再是"32 个标签各 1 条"）；
   - ④ **回放**：拿现网 13 个工程的真实 `keyword` 过一遍新旧两条路，逐条对比命中数/是否仍是"目录顺序前 8"。

## 5. 成本与风险

- **一次性成本**：881 条 × 约 30 次调用（batch 30）。跑完只落一个 JSON 的 diff，可 review。
- **词表是约定**：一改就要重跑 + 同步 prompt ⇒ 所以词表必须**代码里一份真源**（§4.1）。
- **`tag` 必须保留（派生）**：删了它，面板分组、`index`、`/api/polish/emoji-catalog` 一起漂。
- **回滚**：工具写回前备份 `emoji_catalog.json`（`.bak`），git 也留一份 —— 不满意就 `git checkout` 回来。

## 实施记录（2026-10-17）

### 已完成

| 项 | 落点 |
|---|---|
| 词表真源 | `assets/emoji_taxonomy.json` + `src/assets/emoji_taxonomy.rs`（六维 / 85 个取值 / 每维权重与多选标记） |
| 分类工具（开发期） | `polish emoji build`（`commands.rs::cmd_emoji_build`）—— 三轮补漏（40→20→10）+ 三层键兜底 + 备份 + 校验；手跑入口 `emoji_build_dev_tool`（`#[ignore]`） |
| 数据 | 881/881 全分类；零覆盖取值 **48 → 1**；`tag` 降级为派生字段（各维并集 ∪ 旧 tag） |
| 查询 | `EmojiCatalog::{search_dims, search_best, dim_of, has_dims}`；`process/download.rs` 的 emoji 分支改走 `search_best` |
| 面板 | `grouped()` 按主维度（emotion，单选）分区，没分到组的进 `uncategorized`（一个都不丢） |
| 提示词 | 选了 emoji 来源时注入词表 + 教 `emotion:joy,usage:punchline` 这种查询写法 |
| 内建包协议 | `sticker_index::{BUILTIN_EMOJI_PACK, is_builtin_pack, is_known_pack_source, builtin_meta}`；`load_package_meta` 磁盘没有就回落内建；候选文本列内建包（按维度报张数）；`llm/mod.rs::has_local` 改用 `is_known_pack_source` |

### 两处**有意的偏差**（比方案更保守）

1. **`is_package_source` 保持纯磁盘判据**，另开 `is_known_pack_source`（磁盘 ∪ 内建）。
   理由：`project/validate.rs::fill_default_local_packages` 拿 `is_package_source` 判"已经有包了、
   别再自动补"——让内建包算进去，`sources = ["emoji"]` 的工程就**不再自动补入已 build 的包**，
   那是静默的行为变更。
2. **`scan_packages` 不动**（仍以磁盘为准），内建包由 `local_sticker_candidates_text` 单独追加。
   理由：`scan_packages` 的消费方还包含 `try_resolve_sticker` 的包名分支与素材库列表，
   塞一个"文件是逻辑路径、磁盘上并不存在"的包进去，会让"列表以磁盘为准"这条老口径失真。

### 待做

- **emoji 接上 Step2b 精挑**（现在 `run_sticker_choice_step` 按 `(包名, tag)` 收集，而 emoji 事件的
  `keyword` 是维度查询、`sticker_tag` 为空 ⇒ 收不到）。候选 = `search_best(keyword, N)`，
  需要给 LLM **看得懂**的候选形态（`1f602 😂` 这种），返回值再取出码点。
- `process/download.rs` 的 `search_best` 接线**未提交**（与作者的存量 WIP 同一文件，hunk 拆分被 git 判 corrupt）。
- 真实工程 `keyword` 回放对比（方案 §4.5 的第 4 条判据）。

### 真实工程回放（2026-10-17，目标 §6.4）

数据源：dev 实例的账本 `target/debug/.yekitrim/cloud/yekitrim.db`（**不是** `target/debug/yekitrim.db`，后者是空的）
→ 13 个工程 → 58 份 `polish.toml` → **133 条** sticker keyword，只有 **3 个不同取值**：

| 取值 | 条数角色 | 新链路怎么接 |
|---|---|---|
| `laugh` | 词表内自由词（action.laugh） | `search_best` → `guess_dims` → 分维打分 |
| `shock` | **词表外**的旧词 | `search_best` 落到**旧匹配兜底**；而旧 tag 被并进派生 `tag` ⇒ 仍然命中 ✓ |
| `ysyk` | 包名（不是 emoji） | 包名分支，未动 |

⇒ **零维度查询**（这些工程都是改提示词之前跑的，符合预期：新的查询串只会在重跑 LLM 之后出现）。
⇒ 回放的实际价值是证明**存量 keyword 不会因为改分类而解析不出来**（那条兜底与"旧 tag 并进派生 tag"两件事一起兜住的）。

### 已知偏差（照实记）

- **分布**：一度不达标（`emotion.neutral = 374/881` = 42%）。处理过程照实记：
  ① 把 `emotion` 标成 **optional**（`optional: true` ⇒ prompt 里写"真的没有就留空 []"）并重跑全量分类（23 次调用）——
     **结果 neutral 反而涨到 430**：模型把 `neutral` 当成"没有情绪"的表达方式，那是它的合理读法；
  ② 所以真正该改的是**面板**，不是数据：`grouped()` 改成**两级分组** —— 情绪只有 `neutral`（或没有情绪）的，
     改用 `object` 的第一个取值当组名（旗子归 flag、房子归 building、工具归 tool…）。
  ③ 结果：**39 组 / 881 条**，最大 `calm` **105（12%）**，最小 1–2 —— 判据收紧到"最大组 ≤ 1/4"并带变异自证。
  （`emotion.neutral` 仍然大，但那是**数据事实**、也是选取时不会主动写的值；面板已经不受它拖累。）
- **零覆盖取值 0 个**（这一轮全补上了，连 `action.salute` 都有 1 个）。
- ⚠️ **本仓要求 LF**：用 python 重写 `emoji_taxonomy.json` 时默认换行翻译会写成 CRLF，
  被 `check_module_boundaries.py` 当场抓住 —— 写 JSON 要 `newline='\n'`。
- **零覆盖取值 1 个**：`action.salute`（🫡 `1fae1` 在目录里，模型没给）。判据允许 ≤5%。
- `process/download.rs` 的 `search_best` 接线**未提交**（与作者的存量 WIP 同文件，hunk 拆分被 git 判 corrupt）。