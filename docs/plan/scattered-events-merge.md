# 分散事件合并 + 多段切片（粗剪拓展）

> 立项：2026-10-06（用户口径见下）｜ 归属：`yekitrim-core`（detection / cutting）+ `webui`（预处理页）
> 关联卡：[core-detection](../..//crates/yekitrim-core/src/detection/AGENTS.md)、[core-cutting](../..//crates/yekitrim-core/src/cutting/AGENTS.md)、[webui-views](../..//webui/src/views/AGENTS.md)

## 0. 用户要什么（原话拆解）

> 「增加一个分散事件的功能，合并某一时间段和另一时间段的片段，在事件检测结束后按照顺序合并一些话题一样的片段，
> 可能需要下调话题的阈值，当前值为普通事件，低阈值取出的极小片段可能为分散片段，也可能被抛弃，因为太短了而且也组不了。
> 这样可能要增加一个新的切片功能，在预处理位置可以设置多个切片入点出点以列表形式，这样就可以分段但是合并输出。算粗剪功能的拓展。」

拆成四件：

1. **检测第二趟**（低标准）：每个窗口除"符合规定的事件"外，**多输出一路**低标准片段（时长 / 字幕量 / 弹幕量三档下调）。
2. **检测后合并**：所有窗口跑完后，把这些低于标准的片段**按顺序、按同话题**合并；**仍旧不达标就丢弃**。
3. **多段切片**：合并出来的一条事件可能由**多段源区间**组成 ⇒ 粗剪要能"分段剪、合并输出"。
4. **预处理页列表 UI**：入点/出点以**列表**形式多组，一次建一条多段片段。

口径澄清（用户 2026-10-06 追问后）：

- **段间空隙**：合并时相邻两小块之间没被选中的时间。**无字幕的空隙可以保留**（拼成一条连续区间，交给后处理
  「去静音」→ 屏蔽区间，出片侧 `Ffmpeg::cut_ranges` 剪掉）；**有字幕/有内容的空隙才丢掉**（这才需要多段）。
  ⇒ 多段模型只在少数片段上用到，代价大降。

## 1. 现状事实（勘察结论，带位置）

| 事实 | 位置 |
|---|---|
| 一个片段 = **一条连续源区间** `Event{start,end}`；Cutter 一次 `-ss/-t` | `types/mod.rs:149`、`cutting/mod.rs:297` |
| `clip.json` 只存 `original_start/end`（并集语义天然兼容） | `cutting/clip_index.rs:62` |
| core **没有** concat 工具（`cutting/mod.rs:495` 那句是悬空注释） | 待新增 |
| "丢中间、拼两头"本仓已跑通，但只在**出片侧**（屏蔽区间） | `polish/render/ffmpeg.rs:181` |
| 硬标准：`MIN_EVENT_SECS = 30.0` + `enforce_min_duration` 吞并相邻 | `detection/mod.rs:22`、`:691` |
| 跨窗口去重：重叠比 ≥50% 留更长；截断后 <30s 丢 | `detection/llm.rs:938` |
| 弹幕热点：10s 窗密度 ≥0.5、top 20 → 注入 prompt | `detection/mod.rs:526` |
| 热度：`base=(0.6·峰值+0.4·均值 + 字幕密度)/2`，`popularity=(1-w)·base+w·LLM分`，w 默认 0.3 | `detection/mod.rs:385` |
| **没有**"字幕条数"硬门限（只在 prompt 里软写） | — |
| 预处理页现在只有**一组**入点/出点 | `webui/src/views/PrepareView.vue:163` |

## 2. 标准对照表（改动前 → 改动后）

| 维度 | 普通事件（不动） | **分散片段（新增第二趟）** |
|---|---|---|
| 时长 | 硬 ≥30s；prompt 0.5~3 分钟 | 单块 **8s~30s** 也收 |
| 字幕量 | prompt：低密度长段并走 | 块内 **≥2 条** 或 字幕密度 **≥0.15** |
| 弹幕量 | 热点阈值 ≥0.5 | **≥0.15** 也算信号（无弹幕素材不看） |
| 切分前提 | 明显话题转换 | 能起一个独立标题就收（宁可多报） |

合并（跨全部窗口之后）：同 `topic` 键 + 时间相邻、间隔 ≤ **45s**；
无字幕空隙 ⇒ 连续区间；有内容空隙 ⇒ `ranges` 丢掉；
合并后总时长 **<20s** 或 字幕 **<3 条** ⇒ **丢弃**；单条合并后 **≤240s**（超了按最热块截断）；
最后与普通事件做一次重叠合并（重叠 ≥50% 并入普通事件、**保留普通事件标题**）。

配置（默认关）：

```toml
[detection.scattered]
enabled = false
min_secs = 8.0          # 单块时长下限
min_subs = 2            # 单块字幕条数下限
dm_floor = 0.15         # 弹幕密度下限（0 = 不看弹幕）
merge_gap_secs = 45.0   # 同话题相邻块的合并间隔
merge_min_secs = 20.0   # 合并后仍不足 ⇒ 丢弃
merge_min_subs = 3      # 合并后字幕条数下限
max_merge_secs = 240.0  # 合并上限
topic_key = true        # 要求 LLM 输出 topic 键
```

## 3. 数据模型

- `Event.ranges: Vec<EventRange>`（`#[serde(default, skip_serializing_if = "Vec::is_empty")]`）
  —— **空 = 单段 = `start..end`** ⇒ 老 `events.json` / `clip.json` 全部照旧可读。
- `Event.topic: Option<String>`（第二趟的归组键；`skip_serializing_if` 空）。
- 不变式（单测钉住）：`ranges` 非空时 `start == ranges[0].start`、`end == ranges.last().end`。
- `clip.json` 增 `ranges`；`original_start/end` 仍写**并集**（`clip_meta_matches` 的 IoU 照旧）。

## 4. 分期与验收

| 期 | 内容 | 验收 |
|---|---|---|
| **P0** | `Event.ranges` / `topic` + `[detection.scattered]` 配置（五步：字段/merge/Default/模板/文档） | `cargo test -p yekitrim-core --lib`；老 events.json 逐字节读回单段 |
| **P1** | 第二趟检测（`SCATTER_PROMPT` + 独立 checkpoint，**不经过 30s 吞并**）+ `merge_scattered` 纯函数 + 并入普通事件 + rank 重排 | 纯函数单测（同 topic 合并 / 超间隔不并 / 太短丢 / 上限截断 / 无字幕空隙保留 / 有内容空隙出 `ranges`）+ **变异自证** + 真素材端到端（`events.json` 打印合并统计） |
| **P2** | Cutter 多段：逐段剪 `part-NN.ts`（同 encoder 参数）→ concat demuxer `-c copy` → `clip.mp4`；失败回退 `-filter_complex concat` 重编码；`clip.srt`/`clip.dm.srt` 平移拼接；`clip.json.ranges` | 单测（段数/总时长/偏移）+ 真素材实测（2 段拼 1 条，逐段对拍字幕时间不漂） |
| **P3** | 预处理页入点/出点**列表** UI（加/删段、总时长、段数）+ POST `ranges` | CDP 探针（载荷含 `ranges`、几何断言）+ `check-layout-invariants.mjs` 加条目与**变异** |
| **P4** | 展示（事件表/切片卡 `（N 段）` 徽标）+ 设置页 scattered 数值 + 卡片/文档 | 全量门禁 |

**每期结束都跑**：`cargo check --workspace --all-targets`（0 警告）· 相关 `cargo test` ·
`python3 scripts/check_module_map.py` · `python3 scripts/check_module_boundaries.py` ·
前端改动加 `npm --prefix webui run build` + `node webui/scripts/check-*.mjs`。

## 5. 风险与取舍

- **事件数暴涨**：第二趟只发一次调用（复用同批窗口），并有 `max_merge_secs` + "合并后仍不达标就丢" 兜底。
- **30s 吞并会吃小片段**：两条通道必须**分开**（否则症状是"阈值降了、结果没变"）。
- **多段剪切**：N 段 = N 次编码 + 1 次 copy 拼接；编码参数必须一致，不一致回退整条重编码（慢但正确）。
- **字幕漂移**是这类拼接最典型的坑 ⇒ 判据必须逐段对拍 `clip.srt` 时间。
- **不做**：不把段间空隙当屏蔽区间（那是出片侧语义，会让粗剪产物仍含中间段）。
- **兼容**：老 `events.json` / `clip.json` / `publish_config.json` 一律按单段读；汇总产物不变（一条 clip 一项）。

---

## 6. P1 实测（2026-10-06，项目 4「电台小耶」，真 LLM 真数据）

命令（诊断测试，不写用户工程 —— 断点落临时目录）：

```bash
# 第二趟单独跑（120 段节选）：
YEKITRIM_DATA_DIR=target/debug VT_PROJECT_DIR='E:\media\[…][电台小耶]' VT_SCATTER_MAX_SEGS=120 \
  cargo test -p yekitrim-core --lib diagnose_scattered_pass -- --ignored --nocapture
# 端到端跑 Stage 3（临时工程 + 打开 scattered，再看落盘的 events.json）：
YEKITRIM_DATA_DIR=target/debug VT_PROJECT_DIR='…' [VT_SCATTER_FROM=1420] VT_SCATTER_MAX_SEGS=90 \
  cargo test -p yekitrim-core --lib diagnose_scattered_stage -- --ignored --nocapture
```

**第二趟本身工作正常**（82.6~636.6s 节选，用时 5.8s）：

| 指标 | 数 |
|---|---|
| LLM 报出的候选块 | 27 |
| 太短（<8s）丢掉 | 11 |
| 归组 | 8 组 → 留下 **5 条**（其中 5 条由多块合并、**1 条带多段 `ranges`**）|
| 合并后内容不足 20s 丢掉 | 3 组 |

**但端到端跑完整 Stage 3 时，分散那趟一条都没进最终 `events.json`** —— 原因不是 bug，是
**主检测的覆盖已经把它们全盖住了**：项目 4 的 70 条主事件覆盖 **8932s / 10944s = 81.6%**，
中位时长 108s；而第二趟报出的块（100.5-212.9、223.9-285.2、387.7-434.7…）**全都落在主事件内部**
⇒ `absorb_scattered` 按"重叠 ≥50% 即丢弃"全部丢掉。

**结论（这条改变了后续设计）**：当前"只在空隙里补"的口径，在这个真实项目上等于**没开**。
用户真正要的是「同一个话题分散在两处 ⇒ 合成一条」——真实数据里确实存在这种情况：
`451.4-529.2`（落花一样飘落）与 `552.7-758.0`（轻声唱被说痛耻）是**两条独立的主事件、同一个"唱歌"话题**。
⇒ 下一步（**P1b**）：
- 用第二趟的 `topic` 键去**标注主事件**（一个主事件被哪个 topic 的块覆盖最多 ⇒ 算它属于这个 topic）；
- **同一个 topic 的 ≥2 条主事件** ⇒ 合成**一条多段事件**（ranges = 各主事件自己的区间，中间那段
  有内容的部分丢掉），标题取**最热的那条主事件**的（主事件的标题是权威，不许被小块改写）；
- 被合并掉的主事件从 `events.json` 移除 ⇒ **必须先有 P2 多段剪切**（否则合并出来的片段切不出来），
  而且这会**改动已有片段拓扑**（两条片段并成一条）⇒ 要有日志/开关，且默认只在
  `scattered.enabled = true` 时生效。

---

## 7. P2 多段剪切（已落地，2026-10-06）

- `cut_event(video, ranges, output)`：**单段走老路径**（`cut_segment`，行为不变）；多段
  「逐段剪 `.ts`（**一次选定编码器**）→ concat demuxer `-c copy`」，失败回退
  `-filter_complex concat` 重编码，再失败整体换 `libx264`；`.parts/` 成功失败都清掉。
- 字幕/弹幕：`range_offsets` = `[0, d0, d0+d1, …]`，逐段裁剪 + 平移（第 2 段接在第 1 段之后），
  段间空隙的内容**不进成片**；单段就是"减一个常数"，老的 `generate_clip_subtitle/danmaku`
  改成薄包装（只给单测用），口径只留这一条。
- `clip.json`：多段时写 `ranges`，`duration` = **内容时长**（成片文件长度，不是跨度）。
- 判据：`cutting::multi_range_tests` 4 条单测（段起点累加 / 字幕平移 + 空隙排除 / 弹幕同口径 /
  meta 的 duration 与 ranges）+ 真剪一次 `diagnose_multi_range_cut`：
  `[0.5-2.0] + [3.0-4.5]` → 成片 **3.06s**（期望 3.0）、轨道 `h264,video | aac,audio`、
  `.parts/` 清干净、用时 1.2s。

---

## 8. P3 预处理页「入点/出点列表」（已落地，2026-10-06）

- `PrepareView.vue` 的「切片」卡：**列表**（每行一组入点/出点 + 该段时长 + 删），底部「＋ 一段」、
  「标记入点/出点」（落到**选中行**）、段数与总时长。**只剩一段不给删**（删成 0 段 = 没有要切的东西）。
- 载荷：**多段才带 `ranges`**（单段与老的「标记入点/出点」逐字段同形）；服务端
  `POST /api/project-data/events` 认 `ranges`，`start/end` 拉成首段起、末段末，多段才写进事件。
- 模态框（更多设置）在多段时把「开始/结束」置灰 + `title` 说明（起止由列表决定）。
- 判据：`check-layout-invariants.mjs` **⑳**（5 项 + **5 处变异**：载荷一律带 ranges / 删段不挡最后一段 /
  服务端不解析 / Cutter 退化成单段 / `cut_ranges()` 不回退单段）+ 真页面 CDP
  `node .workbuddy/probe-slice-list.mjs`（11 项：加段/删段禁用、每段时长与总时长、
  **多段载荷带 ranges**、**单段载荷不带 ranges**）。探针把 `fetch` 换成拦截器 ⇒ **不写用户数据**。

---

## 9. P4 多段片段的「N 段」标记（已落地，2026-10-06）

用户口径：「多段的切片需要特殊标记」。四处卡片墙都要认得出，判据只认 **> 1**
（与写侧 `Event::ranges.len() > 1` 同一条口径；单段写了 `ranges` 也不算多段）：

| 位置 | 标记 | 判据 |
|---|---|---|
| 切片页卡片墙（已剪卡片） | `N 段`（`tag r`，`title` 说明"分段剪、合并输出"） | `clip.json` 的 `ranges` |
| 切片页卡片墙（未剪切事件） | `将切 N 段` | `events.json` 的 `ranges` |
| 精修页片段条（`FragmentList`） | `N 段`（chip，贴在状态 chip 右边） | `clip.json` 的 `ranges`（`PolishView::clipRow` 给 `segments`） |
| 首页「切片总览」 | `N 段`（叠在时长下方） | `clip.json` 的 `ranges` |

索引侧：`ClipEntry` 加 `ranges`（`#[serde(default, skip_serializing_if = "Vec::is_empty")]`）——
`from_event` 只在多段时带；老索引条目从 `clip.json` 的 `ranges` 补（`merge_meta_into_entry`，
退化段滤掉、≤1 段不算）。**单段序列化出来没有 `ranges` 键**（老索引/老前端不受影响）。

判据：
- `check-layout-invariants.mjs` **㉒**（4 项 + **6 处变异**：判据放宽成 `>0` / 少一处渲染 /
  精修片段条不给 segments / 片段卡不渲染 / 首页不带 segments / 索引丢掉 `ranges`）。
  ⚠️ 两个段数助手相邻，第一版用 `[\s\S]{0,300}?` "窗口"匹配 ⇒ 变异只改第一个也照样通过（假绿），
  已改成**逐字匹配**。
- 核心单测 `cutting::clip_index::multi_range_index_tests`（2 条）：多段进索引 / 单段不写 `ranges` 键 /
  从 `clip.json` 补 ranges / 退化段滤掉。
- 行为级 `.workbuddy/check-range-badge.mjs`（**不需要起服务**）：从 `.vue` 源码**抠出**
  `clipRangeCount` / `eventRangeCount` 喂 9 组数据（缺字段 / 空数组 / 1 条 / 2 条 / 3 条 / 脏数据）。
- 真页面 `.workbuddy/probe-range-badge.mjs`：接口 vs DOM 对拍（多段的必须有标记、单段的不许有）。
