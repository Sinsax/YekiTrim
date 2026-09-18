# 任务队列与三档并发闸门

服务端所有后台任务都过 `JobManager`（`crates/vtrim-server/src/jobs.rs`）。
这份文档解释**为什么要分三档、每一档装了什么、为什么这么装、什么时候会排队**。
代码实现只此一份，本文是它的说明书 —— 改档位/上限时请一起改这里。

---

## 1. 要解决的问题

最早的实现是"全局同一时刻只跑一个任务"，任何任务活动时新任务直接 409。后来放宽成
**资源域互斥 + 单一并发闸门**（`MAX_CONCURRENT_JOBS = 3`，FIFO）：资源键不相交、且活动任务
不到 3 个就直接开跑。

单一闸门留下了两类明显不合理：

1. **轻活陪着排队。** 3 个渲染在跑时，一个 5 秒的 `polish_init`（建目录 + 抽 fx/sfx 资源）、
   一张封面的 `polish_cover`、或者后处理 `polish_process`，全都要排在它们后面。
   用户点了按钮，界面几分钟没动静，而这几件事本身只要几秒。
2. **等网络的任务白占名额。** `stage_detect` / `correct` / `polish_llm` 的墙钟可以到几分钟，
   但它们 99% 的时间在等 LLM 返回，**本机 CPU 几乎不动**。把它们和渲染、转写算作同类，
   等于用一个"CPU 保护闸门"去限制一个不消耗 CPU 的任务 —— 保护没生效，吞吐白丢。

---

## 2. 分档依据：本地资源占用，**不是**时长

这是整个设计里唯一真正需要记住的判断标准。

| | 判据 | 反例 |
|---|---|---|
| ✅ 采用 | 这条任务会不会**长时间占住本机 CPU/GPU/磁盘** | |
| ❌ 不采用 | 这条任务墙钟要跑多久 | `stage_detect` 墙钟 5 分钟，本地几近于零 |

墙钟长 ≠ 是重活。网络等待型的任务堆多少个都不会把机器抢死，所以它们该被允许并行；
而 ffmpeg 编码、ASR 推理、headless Chrome 这类"墙钟在跑 = CPU 在烧"的活，堆起来就会互相
拖慢甚至把机器拖垮 —— 那才是闸门要管的东西。

---

## 3. 三档

| 档位 | 名额 | 超额行为 | 成员 |
|---|---|---|---|
| `Light` 轻活 | **不占闸门**（不限） | 只有资源键冲突才排队 | `polish_init`、`polish_process`、`polish_cover` |
| `Medium` 略重 | `MEDIUM_CONCURRENT` = **3** | 本档排满则排队 | `cut_one`、`stage_extract`、`stage_detect`、`correct`、`polish_llm`、`sticker_build`、`download_models` |
| `Heavy` 重活 | `HEAVY_CONCURRENT` = **2** | 本档排满则排队 | `stage_transcribe`、`stage_cut`、`stage_process`/`pipeline`、`cut_all`、`polish_render`、`ingest_*` |

两个上限是 `jobs.rs` 里的常量（与旧 `MAX_CONCURRENT_JOBS` 同一约定：**要调就改常量**，
不是配置项），但统一从 `JobClass::cap()` 取，只有一处。

> ⚠️ 表里的名字是 `JobSummary.kind`（后端机器名，前端翻成中文），**不是档位名** ——
> 每个 kind 到底做什么、从哪个按钮触发，见 **附录 A**。
> 一条动作属于哪一档与它叫什么名字是两个独立的决定。

### 3.1 轻活 `Light` —— 不占闸门

**这类任务的存在意义就是"不该让它等"**：用户每次改动都会触发的、或者点一下就该立刻看到反应的。

| 任务 | 是什么 | 为什么是轻活 |
|---|---|---|
| `polish_init` | 初始化精修工程：建 `polish.toml` + `data/` + `sfx/`，从 `clip.json` 自动认标题/时长/素材路径，探测弹幕文件，再跑一遍素材解析 | 秒级，纯本地小活；通常是一次精修的**第一步**，排在别的渲染后面等于"点了没反应" |
| `polish_process` | 分段后处理 + 资源下载 + 位置计算 | **编辑器每次改动后自动重跑**（1.5s 防抖），是交互循环的一部分；它被闸门挡住就等于编辑卡住 |
| `polish_cover` | 出一张封面（起一次 headless Chrome） | 十几秒，只出一张图；不该排在 3 个渲染后面等十几分钟 |

**敢设成不限并行的三个理由：**

1. 都是秒级到十几秒的活，堆起来也很快就散了；
2. 它们**彼此由片段资源键互斥** —— 同一个片段上的多个精修任务是串行的，闸门不用再管一遍；
3. 唯一的硬开销是每条任务一个 OS 线程（`spawn_job`），几十个线程对机器不构成压力。

⚠️ 第 3 条只在**单机自用**时成立。公网多租户下"一个账号能开多少线程"不能没有上限，但解法
**不是**给轻活加闸门名额（那会把编辑器每次改动都要重跑的 `polish_process` 一起挡住），
而是按账号给**活动任务总数**封顶 —— 见 §3.5。

⚠️ `polish_process` 是这里最容易看错的一个：它由编辑器自动触发、**调用频率最高**，
所以它必须豁免闸门。如果嫌它小看它、把它塞进略重档，用户每改一次都在赌"现在有名额吗"。

### 3.2 略重 `Medium` —— 上限 3

**装的是"要么只用一条 ffmpeg、要么全程在等网络"的任务。**

| 任务 | 是什么 | 为什么在这里 |
|---|---|---|
| `cut_one` | 单条片段剪切（一条 ffmpeg） | 有 NVENC/QSV 时是 GPU 活，没有时是一条 libx264；量级等于"一条 ffmpeg"，不到重活 |
| `stage_extract` | 抽 16kHz 单声道音频 | 一条线性解码的 ffmpeg，是一次性预处理里最轻的一步 |
| `stage_detect` | LLM 事件检测（分批窗口调用） | **吃网络不吃 CPU** —— 典型的时间长但资源轻 |
| `correct` | LLM 订正转录 | 同上 |
| `polish_llm` | LLM 生成剪辑方案（然后自动跑一次 process） | 同上；后半段 process 本来就是轻活 |
| `sticker_build` | 贴纸包 LLM 批量打标 | 同上；且持有全局 `stickers` 键，实际同时只有一个 |
| `download_models` | 下载 ASR 模型（约 1.1GB） | 纯网络 + 解包，本机只是写盘 |

**为什么是 3：** 这与旧的单一闸门值相同，所以对"网络型任务"的吞吐不降级。3 条略重任务 ×
每条最多 3 路并发 LLM 窗口 ≈ 9 个并发请求，与改造前的最坏情况一致，不会额外压中转站。

### 3.3 重活 `Heavy` —— 上限 2

**装的是"跑起来就在烧 CPU/GPU 和磁盘"的任务。**

| 任务 | 是什么 | 为什么在这里 |
|---|---|---|
| `stage_transcribe` | ASR（sherpa-onnx，CPU only） | 吃满 CPU：默认 CPU/4 路并行、每块 intra-op threads=2 |
| `stage_cut` | 按事件批量剪切 | 一次跑 N 条 ffmpeg |
| `cut_all` | 一键全切 | 同上（内部还有 3 路 ffmpeg 并发） |
| `stage_process` / `pipeline` | 全流程 / 自定义阶段组合 | 只要含转写、剪切或全流程就是重活（见 `for_pipeline`） |
| `polish_render` | 渲染导出（headless Chrome + ffmpeg 分块编码） | 全程 CPU，分钟级 |
| `ingest_*` | 入库后自动管线 | 与手动管线同一批阶段，重量级完全一样 |

**为什么是 2（比旧的 3 更严）：** 旧闸门的 3 个名额是"和所有任务一起分"的，真跑起来可能是
1 个转写 + 2 个渲染同时烧 CPU。把重活单独收到 2 是更严的保护 —— 代价是**重活吞吐确实降了**：
改造前 3 个不同片段的渲染能同时跑，现在第 3 个会排队。这是刻意的取舍（详见 §7）。

### 3.4 不在名单里的任务

容易误以为"也是个任务"、实际不是的四个，完整清单见**附录 A 末节**，这里只点出最容易踩的两个：

- **`silence`（去静音）不是任务**：前端 `KIND_ZH` 里有这个名字，但服务端它是**同步接口**
  （`/api/polish/silence` 与 `/api/project-data/clips/:rank/silence`），当场跑、不进队列，
  所以它**不受档位管辖**。这类同步接口冲突时返回 409 而不是排队（见 §7）。
- **`ingest_none` 也不会产生 job**：`Pipeline::None` 在 `spawn_pipeline` 里直接 return。

### 3.5 账号级配额（公开部署护栏）

闸门管的是"机器扛不扛得住"，管不了"同一个账号能不能把队列和线程刷爆"。下面三条在
`JobManager::create` 里生效，**是这套队列里唯一的"拒绝"路径**（闸门与资源键都只会让任务排队）：

| 常量 | 值 | 作用 |
|---|---|---|
| `MAX_CREATE_PER_MINUTE` | 60 | 单账号建任务频率。编辑器自动重跑的 `polish_process` 有 1.5s 合并防抖（上限约 40/分钟），正常人手点碰不到 |
| `MAX_ACTIVE_PER_OWNER` | 12 | 单账号**同时活动**（运行中 + 排队中）的任务数 |
| `MAX_ACTIVE_TOTAL` | 64 | 全服活动任务数合计 |

超限的任务**记为失败**并**丢掉开跑信号**：工作协程 `wait_start()` 拿到 `Err` 后直接收工
（与"排队中被取消"同一条路径，所有创建点都已经在处理）。任务本身仍留在 `/api/jobs` 列表里，
前端能看到"为什么没跑（活动任务过多 / 建任务过于频繁）"，而不是点了没反应。

> 为什么需要：`docs/job-queue.md` §4 里"前台任务都是人点出来的，频率有天然上限"这条前提，
> 在**公网多租户**下不成立 —— 脚本化的客户端可以每秒几千次。单机自用时这三条永远不会触发。

---

## 4. 优先级：前台 / 后台

`JobPriority` 只有两档，且**只在同一个档位的队列内部起作用**：

| 值 | 谁 | 行为 |
|---|---|---|
| `Foreground`（默认） | 用户点按发起的一切任务 | 正常排队 |
| `Background` | 入库自动管线 `ingest_*`、`download_models`、`sticker_build` | 同档内**排在所有前台任务后面** |

用 `.background()` 标在创建点上。作用场景：管理员一次指派几场录播，入库自动管线会在重活档
占着名额跑好几分钟；此时另一个用户点"渲染导出"，他的任务应该排在入库任务**前面**，而不是
等自动任务跑完。

**两条刻意的限制：**

1. **不抢占。** 已经在跑的任务不会被让位 —— 半路掐断一次转写等于把它白跑一遍。
   优先级只影响"还没发车的那批谁先上"。
2. **后台可能饥饿。** 如果前台任务持续不断，后台任务会一直等。因为前台任务都是人点出来的
   （频率有天然上限），这个风险可以接受；真到了会自动刷任务的规模，再考虑加"等待超时提升"。

---

## 5. 与资源键的关系：两个正交的轴

这是接手这块代码时最容易混淆的地方。**档位和资源键管的是完全不同的两件事：**

| | 管什么 | 怎么表达 | 冲突结果 |
|---|---|---|---|
| **档位** `JobClass` | 机器扛不扛得住 | 本档名额计数 | 排队（等名额） |
| **资源键** `scope_key` | 会不会写同一批文件 | 键集合求交集（`resources_conflict`，相等匹配） | 排队（等对方收工） |

**档位豁免的只是闸门名额，"写同一批文件"的物理冲突照旧。**
举例：`polish_init` 是轻活，不占闸门；但如果同一片段上正有一条 `polish_render` 在跑，
两者都持 `p{pid}:clip:clip-07`，`polish_init` **仍然要排队** —— 放开它等于让两个任务
同时改同一份 `polish.toml`。

反过来说，档位的收益只在**资源不相交**时体现：同一个片段的精修任务本来就被资源键串起来了，
分档对它们没有影响；分档真正解决的是"我的轻活排在**别的项目/别的片段**的重活后面"。

所以一条任务的最终状态是**两个轴的叠加**：名额没满 **且** 资源键与所有在跑任务不相交，
才立刻开跑。

---

## 6. 调度器怎么放行

`schedule_locked()` 在**每次任务入队**与**每次 `complete`/`fail`**（即资源释放）时跑一遍；
另外还有一个**自愈 tick**（`jobs::spawn_housekeeping`，每 `SCHED_TICK`=5s）兜底 ——
事件驱动的前提是"每个任务最终都会 complete/fail"，只要有人永远不返回就没有事件了，
tick 保证其它任务不会因此永远不动（见 §7）：

1. 扫一遍所有"还占着资源"的任务（`holds_resources()`：`Running`，或已取消但协程还没退出），
   按档位累计**已用名额**，并收集它们的资源键集合；
2. 把所有 `Queued` 任务的**下标**收集起来，按 `(优先级, 创建顺序)` 稳定排序 —— 排序的是下标数组
   而不是 `jobs` 本身（`jobs` 的插入顺序还要用于列表展示）；
3. 按这个顺序逐条判定：**本档名额满 → 跳过；资源键冲突 → 跳过；都通过 → 置 `Running` 并发开跑信号**。

两个关键细节：

- **名额满 / 资源冲突都是 `continue`，不是 `break`。** 旧实现撞到并发上限就停止扫描，
  那在单闸门下是对的（保证 FIFO）；分档之后继续扫描，才能让"重活排满"不挡住后面的轻活。
- **同档 FIFO 没有因此丢掉。** 名额满时，本档在它后面的每一条也都不满足条件 —— 天然全被跳过。
  所以同档内仍是严格的先来先跑，只有优先级能让前台越过后台。

### 排队位次

`JobSummary.queue_position` 是**同档队列里的名次**（1 起，0 = 没在排队），
按 `(优先级, 创建顺序)` 算。两点要注意：

- 它是"同档里排第几"，**不是"还要等几个人"**；
- **卡在资源冲突上的任务也照算名次**。所以别把它呈现成剩余等待数。

---

## 7. 五条"任务一定有终态"的底线

**调度是事件驱动的**：`create` / `complete` / `fail` 各跑一次 `schedule_locked`。这套设计有一个
隐含前提 —— **每个任务最终都会走到 `complete` 或 `fail`**。一旦有任务永远停在 `running`，
它会一直占着资源键与档位名额，而同项目/同片段的后续任务全部**无声地**排队（界面上只看到
一个不动了的任务，查不出是谁挡的）。

2026-09-16 排查"剪切单个视频被卡住 + 剪切按钮一直灰"时把这些缺口一次补齐：

| # | 底线 | 违反的后果 |
|---|---|---|
| 1 | 工作协程 **panic** → `spawn_job` 的 `catch_unwind` 替它 `fail` | 任务停在 `running`，资源永不释放 |
| 2 | **线程起不来** → `spawn_job` 也要 `fail`（以前只打日志） | 同上，而且日志里只有一行 error |
| 3 | **外部进程必须有超时**：`cutting::cut_segment` 的 ffmpeg 走 `run_ffmpeg(cmd, timeout, what)`，超时 `kill` | 素材在网络盘（RaiDrive/WebDAV）上，盘一抖 ffmpeg 就永远卡在 read 上 —— 无限期占着 `p{pid}:out`/`idx` |
| 4 | **调度器自愈 tick**：`jobs::spawn_housekeeping` 每 `SCHED_TICK`(5s) 重跑一次调度 | 没人再触发调度器时，排队的任务停在原地等下一次 `create` |
| 5 | **淘汰只吃"已终态且不再占资源"的**（`Job::evictable`） | 淘汰掉"已取消但协程没退出"的任务后，它的 `complete`/`fail` 会找不到 Job 直接 `return`：资源键不再被计入（两个任务可能同时写一个目录）**且**没人再触发调度器 |

⚠️ 第 1 条在 **release 构建里其实是失效的**：`Cargo.toml` 的 `[profile.release] panic = "abort"`
下 `catch_unwind` 永远不会返回 `Err`。别指望它，真正兜住的是 2/3/4。

**tick 还会点名**：有任务排队超过 `BLOCKED_WARN`(120s) 时打一条 warn，说明**谁被谁挡住、因为什么**
（`blocked_report`：先按资源冲突找，找不到才是本档名额满 —— 两者的处置完全不同）。
这条日志就是"为什么我的任务不动了"的答案。

---

## 8. 已知取舍 / 明确不做

| 取舍 | 说明 |
|---|---|
| 重活档 2 比旧闸门 3 更严 | 重活吞吐下降（3 个渲染同时跑 → 只允许 2 个）。换来的是 CPU 保护更硬。要放宽就改 `HEAVY_CONCURRENT` |
| 最坏并发是两档相加 | 2 重活 + 3 略重 + 不限轻活。但重活这一档更小，整体 CPU 压力比旧的"3 个不限种类"低 |
| 不抢占 | 见 §4 |
| 队列本身不设上限，但**活动任务数有账号级配额** | 见 §3.5：超限是"拒绝"（任务记失败 + 不开工），不是排队。这是公网多租户的护栏；单机自用碰不到 |
| 后台任务可能饥饿 | 见 §4 |
| 同步接口仍然 409 | `/api/polish/silence`、`cover-frame`、`cover-config`、字幕/配置保存这类当场执行、不进队列的接口，冲突时返回 409 + 说明谁在占用。它们没有排队能力，把闸门改细也不会改变这一点 |
| 资源键粒度未动 | `p{pid}:out` 偏粗（同项目的检测/剪切/订正全串行）、`models` 是全局键（所有转写全局串行）。这是另一个问题，改动会牵出大量文件冲突分析 |

---

## 9. 怎么改

| 想做什么 | 改哪里 |
|---|---|
| 给某个任务换档 | 管线类改 `JobClass::for_pipeline(stages)`；精修类改 `JobClass::for_polish(kind)`；其余创建点上是 `JobClass::Heavy` / `Medium` 字面量 |
| 调上限 | `jobs.rs` 的 `MEDIUM_CONCURRENT` / `HEAVY_CONCURRENT` |
| 标后台任务 | 创建点的 `JobSpec::…background()` |
| 加第四个档位 | `JobClass` 枚举 + `ALL` 数组 + `slot()`（三处必须同序，有单测钉住）+ 新常量；调度器的计数表按 `JobClass::COUNT` 自动跟着走 |

⚠️ `slot()` 是调度器那张计数表的下标，`ALL` 的顺序必须与它一致 ——
`pipeline_and_polish_kinds_are_classified` 里有一条断言盯着，别把 `ALL` 重排了却没改 `slot()`。

---

## 10. 排查：为什么我的任务在排队

`GET /api/jobs` 的每条 `JobSummary` 带 `class` / `class_cap` / `queue_position` / `priority`，
webui 任务行在排队时显示：

```
重活任务（并行上限 2）· 同档第 2 位 · 轮到时自动开始
```

排队只有**三种**可能，按这个顺序查：

1. **本档名额满了。** 看 `class_cap` 与同档正在跑的数量。
2. **资源键撞上了在跑的任务。** 看 `resources` 字段与谁在占用 —— 这条即使档位是轻活也一样。
   典型：同一片段上的另一个精修任务、同一项目的另一个管线任务（都持 `p{pid}:out`）。
3. **同档前面排着后台任务**（或反过来）—— 看 `priority`。

对应源码：`vtrim-server/src/jobs.rs`（`JobClass` / `JobPriority` / `JobSpec` / `schedule_locked`）、
`vtrim-server/src/scope_key.rs`（资源键）。单测在 `vtrim-server/src/jobs.rs::tests`。

### 10.1 界面看起来"卡住"时先分清是哪一种

排队是**后端**状态（任务确实没开跑），下面这两种是**前端**问题，症状相似但修法完全不同：

| 症状 | 大概率是 | 去哪看 |
|---|---|---|
| 任务列表整片不动、按钮一直灰 | 前端任务列表冻住了（一次卡住的请求把去重 `inflight` 占住） | 抽屉里有没有"任务列表读取失败"；服务端日志里有没有慢过 1s 的 `/api/jobs`（`access_log` 只在慢/出错时记 info） |
| 抽屉说「暂无任务」但其实有任务 | 读失败被伪装成"没有任务"（旧代码 `jobs.value = []`） | 现在会显示"读取失败 + 重试"，不再是「暂无任务」 |
| 点了「剪切」界面卡 3–4 秒、按钮闪灰 | 任务结束的刷新没走 `silent`，整面卡片墙换骨架屏并卸载了子组件 | `ProjectSlicesView` 的 `onJobDone` 是否带 `{ silent: true }` |
| 任务显示"运行中"但几十分钟没进展 | 底下有个外部进程挂住了 | 任务行会写"已 N 分钟没有新进展"；服务端日志里 `blocked_report` 的 warn 会点名谁挡住谁 |

`/api/jobs` 在 `access_log.rs` 里属于 noisy 接口（成功路径只记 debug），
**但慢过 1s 会照记 info** —— 排查"队列刷不出来"时先在那里找。

---

## 附录 A：任务类型一览

本文正文里的 `cut_one`、`stage_extract` 这些名字是 **`JobSummary.kind`** —— 后端创建任务时写死的
机器名，前端 `webui/src/api/jobs.js::KIND_ZH` 把它翻成中文给人看。它们**不是档位名**，
一条动作用哪个 kind 与它属于哪一档是两个独立的决定（见 §5）。

管线里的**阶段名只有四个**（`vtrim-core/src/pipeline/stage.rs`）：
`extract`（抽音频）、`transcribe`（转写）、`detect`（检测事件）、`cut`（剪切）；
`process` 是"这四步全跑"的别名。`stage_{阶段名}` 是单个阶段，`pipeline` 是显式给一串阶段。

### 管线类

| kind | 实际做什么 | 触发入口 | 档位 |
|---|---|---|---|
| `stage_extract` | Stage 1：ffmpeg 抽 16kHz 单声道 WAV + ffprobe 读视频信息 | 预处理页 / 管线 | 略重 |
| `stage_transcribe` | Stage 2：sherpa-onnx ASR（默认 hybrid = sensevoice 打轴 + qwen3-asr 填词），出 `video.srt` + `transcript.json` | 预处理页主按钮第一段 | 重活 |
| `stage_detect` | Stage 3：分批窗口调 LLM 从字幕里挑事件，写 `output/events.json` | 预处理页第二段 / 切片页「重新检测」 | 略重 |
| `stage_cut` | Stage 4：按 events.json 逐条剪出 `clips/clip-NN/`（clip.mp4 + 字幕 + clip.json）+ 写发布索引 | 管线带 cut | 重活 |
| `stage_process` | 「全自动处理」= extract + transcribe + detect + cut 四步全跑 | 预处理页 | 重活 |
| `pipeline` | 同一套阶段，但阶段列表由请求显式给出（默认也是四步） | `POST /api/project-data/pipeline` 不带 `stage` | 按阶段算 |

`job.kind` 里的 `stage_*` 由 `POST /api/project-data/pipeline` 的 `stage` 字段决定
（`vtrim-server/src/api/pipeline.rs` 里拼成 `format!("stage_{}", stage)`），所以前端给「预处理」按钮分两次调用时
拿到的是两个独立任务：先 `stage_transcribe`、成功后自动接 `stage_detect`。

### 入库自动管线

`ingest_*` 是上面同一批阶段的**前缀版**，由录播姬录制完成的 webhook 经 ingest worker
**自动**发起（`Pipeline::stages()`，`vtrim-server/src/ingest_worker.rs`）。前缀的语义就是"跑到哪一步为止"：

| kind | 阶段 | 档位 |
|---|---|---|
| `ingest_none` | **不建任务**（只是显示用的名字，服务端查不到这个 job） | — |
| `ingest_transcribe` | extract + transcribe | 重活 |
| `ingest_detect` | extract + transcribe + detect | 重活 |
| `ingest_cut` | extract + transcribe + detect + cut | 重活 |

全部带**后台优先级**（§4）。注意即使只是 `ingest_transcribe`，它也含转写 ——
所以 `for_pipeline` 判它是重活，这与手动管线的判定完全一致。

### 剪切类

| kind | 实际做什么 | 触发入口 | 档位 |
|---|---|---|---|
| `cut_one` | 只剪**一条**：从 `events.json` 取出那个 rank 的事件，只注册 Cutter 一个 stage 跑一遍（`run_single_cut_event`） | 切片页卡片「剪切」 | 略重 |
| `cut_all` | 一键全切：把所有**未剪切**事件都剪一遍，内部最多 3 条 ffmpeg 并发 | 切片页「一键全切（N）」 | 重活 |

### LLM / 网络类

| kind | 实际做什么 | 触发入口 | 档位 |
|---|---|---|---|
| `correct` | LLM 订正转录文本，写 `video_llm.srt` + checkpoint | 前端入口已撤，只剩 CLI `vtrim correct` | 略重 |
| `polish_llm` | 精修 LLM 生成剪辑方案（分段 / 运镜 / 贴纸 / 覆盖层…），跑完在**同一个任务内自动接一次 process 后处理** | 精修页「LLM 剪辑」 | 略重 |
| `sticker_build` | 读一个贴纸包的文件名，LLM 压出 10~20 个语义 tag，写 `<包名>.toml` 并注册进 `sticker.toml` | 首页「表情包」区块 | 略重 + 后台 |
| `download_models` | 下载全套 ASR 模型（sensevoice + qwen3-asr，约 1.1GB）到 `<程序目录>/.vtrim/models/`，sensevoice 还补 hr-files，再解包 | 设置页「模型」 | 略重 + 后台 |

### 精修类

kind = `polish_{sub}`，`sub` 就是 CLI 子命令名（`vtrim polish <sub> <dir>`）。

| kind | 实际做什么 | 档位 |
|---|---|---|
| `polish_init` | 初始化精修工程：建 `polish.toml` + `data/` + `sfx/`，从 `clip.json` 自动认标题/时长/素材路径，探测弹幕文件，最后 `cmd_parse` 解析素材。已存在则报错（不覆盖） | 轻活 |
| `polish_llm` | 见上（LLM 生成 segments，随后接 process） | 略重 |
| `polish_process` | 后处理：按当前 polish.toml 与 layout 重算每个事件的位置/锚点/时长吸附。**无 LLM、幂等**，每次都全量重算 | 轻活 |
| `polish_render` | 渲染导出：走**独立 CLI 子进程**（`vtrim polish render <dir>`，会先跑一次 process），进程里 headless Chrome 抽帧 + ffmpeg 分块编码 | 重活 |
| `polish_cover` | 出封面：同样走独立子进程（`vtrim polish cover <dir>`），headless Chrome 截 1920×1080 | 轻活 |

`render` / `cover` 之所以走子进程而不是进程内调用，是因为 headless Chrome 在 WebUI 服务进程内
启动会超时；代价是取消要按 pid 杀进程树（`render_proc.rs`）。

### 不是任务的东西

| 名字 | 实际情况 |
|---|---|
| `silence`（去静音） | **同步接口** `/api/polish/silence` 与 `/api/project-data/clips/:rank/silence`，当场跑完返回。前端 `KIND_ZH` 里有这个名字，但服务端没有对应 job |
| 封面配置 / 取当前帧 / 封面参数 patch | 同步接口（`/api/polish/cover-*`） |
| 保存字幕 / 保存 polish.toml / 保存项目配置 | 同步接口，走 `filelock` 路径锁 |
| 入库本身（文件搬运 / 拼接） | 跑在 ingest worker 的后台线程上，不是 job |

