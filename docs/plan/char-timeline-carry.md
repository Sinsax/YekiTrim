# 字级时间戳搬运（E4b）：`asr/timeline.json` → 精修工程

> 状态：已实现（`crates/yekitrim-polish/src/parse/charlog.rs` + `commands.rs::sync_char_timeline`）
> 判据：`crates/yekitrim-polish/tests/char_timeline_carry.rs`（3 条端到端）+ `parse::charlog::tests`（15 条）

## 1. 起因：一个不成立的数据缺口记录

总表一度把 E4b「T5 真卡拉OK」记为 **"⏳ 需词级时间戳（本仓数据缺口）"**。

**那个说法不成立。** 数据从 2026-09 起就有，而且**比词级更细**（逐字）：

| 环节 | 位置 |
|---|---|
| SenseVoice 逐字时间戳 → `CharTimeline` | `yekitrim-core/src/asr/align.rs`（`chars[i]` 发生在 `times[i]`） |
| 落盘 `<output_dir>/asr/timeline.json` | `asr/artifacts.rs::timeline_record` + `write_all` |
| 由转写收尾调用 | `asr/mod.rs`（Stage 2） |

真正卡住的是另一件事：**`yekitrim-polish` 一行都不读 `asr/`**（`grep -rn asr crates/yekitrim-polish/src` = 0 命中）。
E4b 要补的就是这一段搬运。

## 2. ⚠️ 目录关系：**两个目录不是同一个**（本任务最容易做错的地方）

| 概念 | 实际路径 |
|---|---|
| `ctx.output_dir`（管线侧） | `<工程根>/output`（`yekitrim.toml [cutting] output_dir`，默认 `./output`） |
| **片段目录**（= polish 的 `project_path`） | `<工程根>/output/clips/<dir_name>` |

依据：`cutting/mod.rs` 的 `let clips_dir = ctx.output_dir.join("clips")` 与 `let clip_dir = cd.join(&dir_name)`。
所以 `片段目录 = output_dir/clips/<名>`，是**结构性**父子关系，不是巧合。
实测样本 `.workbuddy/edl-replay/output/clips/demo-clip/`（内含 `polish.toml` + `data/`）与同级的 `output/` 并存。

⚠️ 两处容易写错：

1. **`output_dir` 本身已经是 `output/`** —— ASR 产物在 `ctx.output_dir/asr/timeline.json`，
   **不要再拼一层** `output`。
2. **polish 的入口拿不到 `ctx`**（签名只有 `project_path`），而 `output_dir` 是**可配置**的 ⇒
   只能用**路径反推**。口径照抄本仓已有的 `resolve_danmaku`（`pipeline/runner.rs`）：
   **多个候选基准，第一个存在的赢。**

## 3. 候选表

`parse::charlog::char_timeline_candidates(clip_dir)`，按顺序：

| # | 候选 | 对应布局 |
|---|---|---|
| ① | `片段目录/../../asr/timeline.json` | **规范布局**：`<根>/output/clips/<名>` ⇒ `<根>/output/asr/` |
| ② | `片段目录/../asr/timeline.json` | 万一 `clips` 那层不存在（`output_dir` 直接就是父目录） |
| ③ | `片段目录/asr/timeline.json` | 手工搭的工程（把 asr 放在片段里） |

⚠️ **顺序本身有判据**（单测 `candidate_order_prefers_canonical_layout`）：三个**同时存在**时必须①赢。
顺序写错了在正常布局下**根本看不出来**（因为通常只有一个存在）—— 这正是本仓反复踩的
"跑得通、结果不对"。同一条测试还钉住候选表的字面顺序，防"边走边返回"式改写把顺序弄丢。

## 4. 缺文件必须优雅（与 `_proxy_720.mp4` 同口径）

存量工程的**绝大部分**都没有 `asr/timeline.json`（只有真跑过 Stage 2 的工程才有）。规矩：

- `load_char_timeline` **任何失败一律 `None`**（找不到 / 读不动 / 解析不了 / 长度对不上），**不报错**；
- 调用方拿到 `None` ⇒ 不阻断 `init`/`parse`、**不落任何文件**；
- ⚠️ **"不落文件"是 R4 逐字节零回归的一部分**：如果没数据也落一份空的 `data/char-timeline.json`，
  存量工程目录里就会**多出一个文件**，"删掉 asr 文件 ⇒ 行为与今天逐字节相同"这条判据当场就假了。
- 真读到数据也**只在内容变了才写** —— 首页「切片总览」按片段目录里最新文件的 mtime 排序，
  无条件重写会把老片段一直顶到最前面（与 `process::normalize_danmaku_windows` 同一条纪律）。

判据：`tests/char_timeline_carry.rs::without_timeline_behaviour_is_byte_identical` 真跑两个工程
（有 / 没有 `timeline.json`），断言 **①文件集合逐项相等**（除新增那一份）、**②共有文件内容逐字节相同**。

## 5. 字幕 ↔ 字级时间轴：为什么不能逐字相等

两条链路的文本**本来就不一样**：

- `timeline.json` 记的是 `asr::text::normalize` **之前**的原文（`artifacts.rs` 的模块头明说有意如此）；
- `clip.srt` 是 normalize **之后**的排版结果：行尾 `。.` `，、；：,;:` 被去掉、转述引号被删、
  引号前的 `：` 变 `，`（`asr/text.rs::normalize`）。

所以按 `==` 去对必然大面积为空。**算法 = 子序列贪心 + 命中率闸**：

1. 从时间轴 `cursor` 起按序走字幕的每个非空白字，在**前方 `SEARCH_AHEAD`(4) 个字以内**找相等的字
   —— 找到就跳过去（消化掉"删掉/被改写"的那些字符），找不到就**原地吃一个**（当作"替换"，
   按时间轴当前位置计时）；
2. 算**命中率** = 真正相等的字数 ÷ 字数，**低于 `MIN_HIT_RATE`(0.5) ⇒ 整条作废并回滚游标**；
3. 通过的字条把 `cursor` 留在最后吃到的位置 + 1（**单调、不回头**）。

⚠️ 三个都不能省：

- **命中率闸是必需的，不是保险丝**：没有它，任何一段文本都能靠"原地吃替身"凑出结果 ——
  于是"字幕与时间轴毫无关系"（工程拼错、字幕被人工改过）也会产出一串**看着正常但全是错的时间**。
  加了闸之后，对不上就是**没有**。
- **作废必须回滚游标**：不回滚的话，"一条对不上的字幕"会白白吃掉时间轴上一段字，
  把它后面**本来能对上**的字也顶掉（错一条连累一片）。
- **单调是硬要求**：字幕按时间有序，字也必须只往后走，否则第二条会把第一条的字再吃一遍。

## 6. 落点与数据

- `data/char-timeline.json`（常量 `CHAR_TIMELINE_REL`），与 `subtitles.json` 同级。
- 落的是 **ASR 原样的字级时间轴**（`CharTimelineLog`：`text` / `chars` / `times` / `breath_gap` /
  `breaths[{index,time,gap}]`，字段名逐字对齐 `artifacts::TimelineRecord`）。
- ⚠️ **对齐结果本身不落盘**：哪些字属于哪条字幕是**可重算的派生值**，落盘就有两个真源。
  一句话：**搬数据，不搬结论。**
- 读取结构体自带 `#[serde(deny_unknown_fields)]`：`timeline.json` 是**本仓自己的中间产物**、
  不是用户手改的配置文件，所以宁可"多一个字段就当场读不出来"（暴露上游改了形状），
  也不要静默丢掉不认识的内容。这与 `ProjectFile`（用户会手写，必须宽容）**方向相反**。

## 7. 验证

```bash
cargo test -p yekitrim-polish --lib parse::charlog
cargo test -p yekitrim-polish --test char_timeline_carry
```

变异自证（三条都验过、都翻红）：

| 变异 | 翻红的判据 |
|---|---|
| 缺文件时落一份空的 | `without_timeline_behaviour_is_byte_identical`（R4） |
| 解析失败改成 panic | `missing_or_broken_file_is_graceful` |
| 候选表顺序反转 | `candidate_order_prefers_canonical_layout` |

⚠️ **样本 `timeline.json` 是按 `asr::artifacts::TimelineRecord` 的结构造的，不是真 ASR 链路产出的**
（真跑要模型 + 音频，成本高）。字段名/类型由单测 `matches_timeline_record_shape` 钉住。
