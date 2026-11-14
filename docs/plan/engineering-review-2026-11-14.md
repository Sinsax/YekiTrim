# YekiTrim 全工程 Review（2026-11-14，无人值守）

> **对象**：当前工作树（review 开始时 137 项未提交改动，主要是「WebUI 门槛简化 P2–P4」+ 识别页/首页改版）。
> **范围**：后端功能、前端交互、UI 合理性、整条工作流（ASR → 手动字幕 → 切片 → 出片/预览/封面/导出）、
> LLM 配置、录播接入、下载功能、设置页。
> **方法**：5 个方向并行 review（backend / pipeline / polish / frontend / ui）+ Lead 交叉验证；每域都跑
> 单测、门禁、真实 HTTP 请求或 headless Chrome 两档视口走查。原始报告与截图在本机
> `.workbuddy/review/{backend,pipeline,polish,frontend,ui}.md` 与 `.workbuddy/review/shots/`（均未入库）。
> **口径**：只修「明显问题」（崩溃 / 数据丢失 / 安全 / 明确功能 bug / 契约矛盾）；产品取舍、文案口味、
> 算法参数、布局大改一律记录，交用户拍板（见第 5 节）。

## 0. 结论摘要

- **核心链路可用**：`cargo check` 0 warning、全 workspace 单测全绿、18 条前端静态判据 + 两条结构门禁全过；
  首页 → 添加录像 → 识别 → 挑片段 → 出片 → 导出弹窗 → 封面设计器的主流程实测可点通，全站 0 未捕获异常。
- **本轮挖出 P0 ×1、P1 ×13、P2 约 41 条、P3 约 29 条**（各域合计、按主题去重；数字见各域报告）。
- **已修 P0 ×1 + P1 ×12 + 2 条低风险 P2**；剩 1 条 P1 是产品取舍（`[cutting] overwrite` 死字段），未动。
- 没有发现「整条链路不可用 / 需要停机修」级别的问题；剩余风险集中在**多用户面未收口**、**批量请求/限流**、
  **几条算法与兼容取舍**、**若干部署期安全硬化** —— 都已列成可逐条拍板的决定项。

## 1. 验证基线（review 时）

| 检查 | 结果 |
|---|---|
| `cargo check --workspace --all-targets`（桩库 `SHERPA_ONNX_LIB_DIR`） | ✅ 0 warning |
| `cargo test --workspace --no-fail-fast`（真库） | ✅ 全绿：core 292 / llm 76 / cli 3 / server 250 / service 138 / polish 595 / render-core 41（review 时；修复后计数见 §3） |
| `python3 scripts/check_module_map.py` | ✅ 35 模块 / 7 项检查 |
| `python3 scripts/check_module_boundaries.py` | ✅ 19 条 / 656 文件 / 0 违规 |
| `npm --prefix webui run build` | ✅ 与仓库 `webui/dist` **逐字节一致**（`src_fnv1a64` 指纹相同） |
| 18 条 `webui/scripts/check-*.mjs` + `check-preview-audio` + `camera-zoom-parity` | ✅ 全过（2 条按设计 SKIP） |
| 根 `scripts/check-*.mjs` 全量 | 已知红 5 条（`39-tiers`、`39-tiers-pixels`、`dhampir-wiring`、`perchar-stagger`、`text-motion-parity`）+ `render-visible` 用法提示 + `drop-list` SKIP；与 `docs/linux.md` §4 一致，非本轮新增 |
| 真实服务 | `http://127.0.0.1:26321`（release，data-dir `.workbuddy/yekitrim-lan`）：认证、Range、路径穿越、配置、任务队列、SSE、素材、录播/下载接口均实测；详见 backend 报告 §0 |

## 2. 已修复（本轮）

### 2.1 P0（数据丢失）

| # | 问题 | 修复 | 文件 | 验证 |
|---|---|---|---|---|
| **P0-1** | `yekitrim init` 对已存在的 `yekitrim.toml` **无条件整份覆盖**：素材清单（ASR 槽位）、`transcript_file`、`output_dir` 全被模板值换掉，rc=0、无备份、无提示 | 已存在即拒绝并提示 `--force`；新增 `init --force`，覆盖前备份 `yekitrim.toml.bak-<时间>` | `service/init_config.rs`、`cli/{cli,main}.rs` | 二跑 rc=1、文件 md5 不变、无备份；`--force` 备份为原文后重建；新增 service 单测 2 条 |

### 2.2 P1

| # | 领域 | 问题 | 修复 | 文件 |
|---|---|---|---|---|
| **P1-1** | 后端 | **超配额任务被改写成「完成」**：`complete()` 把 `create()` 拒绝分支的 `Failed` 覆盖为 `Succeeded` 并清空 error；9 个 `wait_start()` 失败分支必踩 ⇒ 用户看到 200+「完成」，实际没跑 | `complete()` 只允许 `Queued` / `Running` → `Succeeded`；其余只置 `worker_done` 并调度。补回归 `quota_rejected_job_is_not_overwritten_by_complete` | `server/src/jobs.rs` |
| **P1-2** | 后端 | **任何登录用户可抢走别人的直播间归属**：`PUT /api/channel-map` 的 upsert 直接改 `user_id`；原主 GET 消失、`DELETE` 回 200 却删不动 | `replace_channel_map` 同事务查冲突，命中别人已登记 ⇒ 整批不生效 + **409**；未登记仍可自助登记，admin 可接管；删 0 行 → 404。补 DB + HTTP 两条回归 | `service/db.rs`、`server/ingest.rs`、`server/api/rooms.rs` |
| **P1-3** | 管线 | `events.json` 缺 `titles` 字段 ⇒ `cut/process` 整条失败，报错只有「解析事件 JSON 失败」（仓库自带 demo 即中招） | `EventStore.titles` 加 `#[serde(default)]`；`load_events` 同时报两种形状的解析错误 | `core/types/mod.rs`、`core/detection/mod.rs` |
| **P1-4** | 管线 | 新建工程从**已退役的顶层** `config.toml` 拷配置，忽略账号树权威那份 ⇒ 内建默认值反盖用户全局设置（全局 `single` → 新工程 `hybrid`，转写慢约 5×） | 配置来源改 `global_config_path()`（缺失/语法坏才回退内建模板） | `core/config/mod.rs` |
| **P1-5** | 管线 | LLM 订正**断点续传结构性失效**：checkpoint 路径只在文件已存在时才传 ⇒ 永不写盘、永不续传 | 新增 `prepare_correction_state`：`resume` 只决定删不删旧断点，路径恒传 `Some`；`core_pipeline` / `service/correct` 两处同改。补假 LLM 端到端用例（首跑写断点、二跑 0 请求） | `service/{core_pipeline,correct}.rs` |
| **P1-6** | 管线 | CLI `yekitrim correct` 用 ASS-only 解析器读弹幕 ⇒ 录播姬 XML/SRT **静默 0 条**，订正 prompt 的弹幕上下文整段消失 | 改走 `Workspace::danmaku_path` + `load_danmaku_auto`；补 XML fixture 用例 | `service/correct.rs` |
| **P1-7** | 管线 | 新建工程 `config_version` 落进 `[project]` 表（根层没有版本键）⇒ 工程配置每次读都按 v0 跑迁移、「版本更高不动」保护永不触发 | 写侧解析 doc → 从 `[project]` 摘掉 → `ensure_version_line` 摆第一行；`write_project_config_exact` 同路。补根层断言 + 变异自证 | `core/config/mod.rs` |
| **P1-8** | 出片 | **预览页对真实工程丢掉画布契约与六轨开关**：`build_html` 裸 `toml::from_str::<ProjectFile>` 被首行 `config_version` 顶成解析失败 ⇒ 竖画布按 1920×1080 预览、关掉的轨道照画，且不报错 | 改走 `project::load_project`（strip 版本键 + legacy + portrait lanes）。补回归（带版本行的 `polish.toml` ⇒ 注入 `_canvas/lanes/vertical`） | `polish/src/render/server.rs` |
| **P1-9** | 出片 | **自定义输出名后产物区全丢**：产物扫描写死 `vtuber-clip`，精修页允许改名 ⇒ 成片写出来了但「产物」空、切片卡片认为未出片 | 先按 `[project] output` 的 file_stem 找，再回落默认名；补单测（`my-cut_2.mp4` 命中、默认名仍命中） | `server/src/api/polish.rs` |
| **P1-10** | 前端 | 识别页**字幕文本编辑 500ms 防抖窗口内切页丢改动**；`saveSubs` 无重入保护（慢盘旧请求后到会盖新稿） | 卸载前 flush 待落盘改动；`saveInflight/saveAgain` 串行化 + 尾随重存；`ownPid` 快照防 A 的字幕写进 B。CDP e2e 验证（改字 100ms 内切路由 ⇒ 恰好 1 次 PUT 且 body 含新文本） | `webui/src/views/PrepareView.vue` |
| **P1-11** | 前端 | 识别页**字幕读失败后仍可编辑并整份覆盖磁盘转写稿**（读失败时 `segments=[]`，点一次「＋」就 PUT 一条） | `writeBlocked` 写闸（`notFound` 或 `loadError`）：`onTrackSave/saveSubs` 拦、`SubtitleTrack` 新增 `disabled`（只挡写不挡看）；404 仍按空态放行。CDP 验证读失败态 PUT=0 | `webui/src/views/PrepareView.vue`、`components/prepare/SubtitleTrack.vue` |
| **P1-12** | UI | 1280×800 精修预览控制行**溢出卡片 96px**：时间折两行、Dhampir 越过卡片右缘、全屏按钮被裁不可点 | 控制行**无条件** `flex-wrap` + 时间 `nowrap/min-width:0`（不用媒体查询：按断点写会在 1401–1422 留「断点失效但还放不下」的缝）；1680 实测仍单行、不回退 | `webui/src/views/PolishView.vue` |
| **P1-13** | 管线 | `[cutting] overwrite` 是死字段：设置页勾选框、模板、merge 都在，**全仓零读取方** | **未修**（产品取舍：真实现 / 摘 UI / 彻底删，见 §5 D1） | — |

### 2.3 顺带修的 2 条低风险 P2

- **登录页空表单点「登录」无任何反馈** → 现在显示「请输入用户名和密码/用户名/密码」并聚焦第一个空框（`LoginView.vue`）。
- **`check-simplify.mjs` 只认 `--phase=P4` 写法**：`--phase P4`（空格）会静默回落 P1 并打印「全过」；
  现在两种写法都认、任何未识别 `--` 参数 exit 2。（注：`--phase=P4` 本身原本就是 exit 2，
  Lead 最初测到的 `EXIT=0` 是 shell `| tail` 吃掉了退出码，已在报告中更正。）

## 3. 修复后全量验证

| 检查 | 结果 |
|---|---|
| `cargo check --workspace --all-targets` | ✅ exit 0（0 warning） |
| `cargo test --workspace --no-fail-fast` | ✅ **exit 0，0 failed**：core 297 / llm 76 / polish 610 / server 253 / service 143 / render-core 41 / cli 3，另有集成/文档 target 全 ok（19+16+3+2+1 ignored 是真模型/真像素/真端点诊断） |
| `python3 scripts/check_module_map.py` | ✅ 35 模块 / 7 项检查 |
| `python3 scripts/check_module_boundaries.py` | ✅ 19 条 / 656 文件 / 0 违规 |
| `npm --prefix webui run build` | ✅ exit 0；dist 指纹 `src_fnv1a64 = afbdeb9107d89249`（与重算 src 一致） |
| 18 条 `webui/scripts/check-*.mjs` + `check-preview-audio` + `camera-zoom-parity` | ✅ 全 exit 0 |
| 1280×800 精修页几何复测 | ✅ 控制行 `scrollWidth == clientWidth`（修前溢出 96px）；全屏按钮落在卡片内且 `elementFromPoint` 命中；1680 无回退 |

修复后已知残留（**环境/既有红，不是本轮引入**）：`scripts/check-dhampir-wiring.mjs`（本机底座 host_api 6 vs 本仓期望 7）、
`39-tiers` / `perchar-stagger` / `text-motion-parity` / `render-visible` 用法 / `drop-list` SKIP —— 与 `docs/linux.md` §4 一致。

```bash
SHERPA_ONNX_LIB_DIR=$PWD/.workbuddy/sherpa-stub cargo check --workspace --all-targets
SHERPA_ONNX_LIB_DIR=/home/toki/project/sherpa-onnx-prebuilt-no-tts/build/cmake-build/install/lib \
  cargo test --workspace --no-fail-fast
python3 scripts/check_module_map.py
python3 scripts/check_module_boundaries.py
npm --prefix webui run build
for f in webui/scripts/check-*.mjs scripts/check-preview-audio.mjs; do node "$f" || exit 1; done
```

## 4. 分域 review（未修的 P2/P3）

> 每条都带 `文件:行` 与复现，完整证据见各域原始报告。这里只留**可拍板的一行结论**。

### 4.1 后端（server + service）

**已核对正常**（避免重复怀疑）：路径三防线三处齐备（`scope.rs` EXTRA_PATH_FIELDS + `config.rs` PROJECT_PATH_KEYS 9 条 +
`cover.rs` ensure_within）；`/api/media` 越界与 `..` 编码实测 404；Range 200/206/416/后缀/多区间正确；CSRF +
HttpOnly + SameSite；未登录 401；未知 `/api/*` JSON 404；`/api/config` 按 Owner 抹 key；任务队列三档闸门/资源键/
取消/panic 兜底/自愈 tick 均有单测。

| # | P2 | 位置 / 现象 |
|---|---|---|
| B1 | `/api/jobs/:id/events` **终态事件永不投递**（`take_while` 吃掉最后一条） | `api/pipeline.rs:239`；前端 `jobs.js` 的 completed/failed/cancelled 分支是死代码，全靠轮询兜 |
| B2 | `/api/recorder/{objects,recordings}` **缺归属过滤**：普通用户能看全平台录像目录绝对路径 + 项目 id | `api/rooms.rs:283/397`；与 `/api/rooms/:id/recordings` 的 owned_room 不一致 |
| B3 | `PUT /api/config/global` **只校验 TOML 语法**、且写入非原子：类型写错 ⇒ 整份配置静默回退默认值 | `api/config.rs:359`；`llm.toml` 那条已有 staged+反序列化+rename，config 没有 |
| B4 | 项目 `missing` **几乎恒 false**：目录被移走也不报；`set_project_missing(false)` 零调用方 | `service/projects.rs:319`、`api/projects.rs:85` |
| B5 | `POST /api/polish/fonts/import` 在**用户层**却收服务器任意路径（只限扩展名） | `api/polish.rs:1542`；注释还写「仅 admin」 |
| B6 | `/api/health` 公开回 `cover_static.fonts_dir` 绝对路径，与卡里「不含任何路径」矛盾 | `api/mod.rs:328` |
| B7 | 配额只拦入库/下载入口，**出片/转写产物增长不设上限** | `service/ingest/capacity.rs:52`；`api/projects.rs:178` 明写手工建项目无闸门 |
| B8 | LLM `base_url` 无出网限制：任意登录用户可让服务器盲探内网/本机端口（有争议） | `api/config.rs:998`、用户层 `PUT /config/llm` |

### 4.2 管线 / ASR / LLM / CLI

**已核对正常**：取消传播（ASR 批次/检测窗口/订正/剪切/下载都有探针）；配置五步（除 overwrite 外）；
ffmpeg 参数（`-ss` 在 `-i` 前 + 重编码）；LLM 重试/退避/计量/extra_params 顺序；`bind_asr_artifact` 唯一写入口。

| # | P2 | 位置 / 现象 |
|---|---|---|
| P1 | `[asr] model="qwen3-asr"` **只影响下载清单**，运行时仍固定 SenseVoice；hybrid 下还会漏下 sensevoice | `core/download.rs:1128/1193`、`asr/local.rs:144` |
| P2 | `publish_config.json` 会把不存在的 `clip.dm.srt` 写进发布清单（实测 clip-03） | `cutting/mod.rs:157/995`；下游按清单取字幕会 404 |
| P3 | 弹幕只有个位数时密度被归一成 1.0 ⇒ **全部窗口都算「爆发时刻」**并注入 prompt | `danmaku/mod.rs:379`、`detection/mod.rs:660` |
| P4 | 缺 SenseVoice 模型时的报错不可行动（没有「跑 yekitrim download」指引） | `asr/local.rs:108-133` |
| P5 | 剪切**部分失败仍算成功**：1 条 ffmpeg 失败只有一条 error 日志，发布清单静默少条目 | `cutting/mod.rs:196-204` |
| P6 | 两个工程并发转写**共享同一个 qwen3 recognizer**（配置串味）并被全局串行化 | `asr/mod.rs:28-57` |
| P7 | `status` 的「锚点覆盖率」是常量：single 恒 0%、hybrid 恒 100% | `asr/local.rs:409`、`asr/mod.rs:326` |
| P8 | 多段片段的 `publish_config.duration` 用**跨度**而非成片时长 | `cutting/mod.rs:1010` |
| P9 | `clips/index.json` 收录空目录；两条事件写同一个 `clip_dir` 会并发写同一份 `clip.mp4` | `cutting/clip_index.rs:155`、`cutting/mod.rs:110/172` |

### 4.3 出片 / 特效 / 预览 / 封面 / 导出

**已核对正常**：转译器守卫 + 109/109 变异自证；`cargo test -p yekitrim-polish` 596 passed；EDL 不变量；
webui 7 条出片相关静态判据；真出片 2s fixture 成功（dhampir）。

| # | P2 | 位置 / 现象 |
|---|---|---|
| E1 | 源素材**没有音轨**时整条出片失败，错误是一整段 ffmpeg 原文；`has_audio_stream` 现成但没接 | `render/pipeline.rs:598`、`render/ffmpeg.rs:1131` |
| E2 | `Ffmpeg::*` 全部裸 `.output()` **无超时**；挂住只 warn 不失败，永久占 Heavy 槽 | `render/ffmpeg.rs` 11 处 |
| E3 | 出片失败路径残留 `_temp_*`（cleanup 只收 2 个文件；早退 `?` 无人清） | `render/pipeline.rs:1280` 等 |
| E4 | 三条「门禁看着在跑、其实空转」：`check-dhampir-drop-list` 缺样本 SKIP + exit 0；`takumi_font_id` 默认 0 tests；底座版本契约无下界（`MIN_HOST_API` 未落地） | `scripts/check-dhampir-drop-list.mjs`、`tests/takumi_font_id.rs`、`dhampir.rs:753` |
| E5 | `cover.image` 非空时每次出封面都覆盖 `_cover_frame.jpg`（与「关键帧保护」方向相反，疑似老工程兼容） | `cover.rs:1161` |
| E6 | 预览服务端口表按 `dir\|kind` 常驻、无淘汰（每个看过的片段一个监听） | `model.rs:35`、`api/polish.rs:960` |
| E7 | 没有「质量档/分辨率/码率」配置项（画质由底座 + ffmpeg 默认决定） | `project/config.rs`、`ffmpeg.rs:1178` |

P3：`start_file_server` 的 `axum::serve().await.unwrap()`；`ProjectClipsSection.render` 死代码且默认绝对路径必 400；
`polish.js` 注释默认值与实际不符；`readiness` 文档口径过时；无 segments 时封面截帧恒按 0.1s。

### 4.4 前端逻辑 / 手动字幕 / 设置 / 录播 / 下载

**已核对正常**：构建可复现（dist 与 src 指纹一致）；路由守卫/回跳、401 跳登录、懒加载 chunk 失效自愈；
设置页保存/回读/权限位；精修页 `loadError` 写入门闸；`PolishPreview` 卸载路径完整。

| # | P2 | 位置 / 现象 |
|---|---|---|
| F1 | 登录/刷新后把**整个任务历史**当成「刚刚结束」重放一遍：N 条终态任务 ⇒ N 轮缓存失效 + 4N 个请求，侧栏统计闪 0 | `composables/useJobs.js:43`、`jobs.rs:922` |
| F2 | B站下载进度轮询**可能永不停止**：只要有一条 session 不在最新 50 条里就永远 pending | `WorkbenchView.vue:1128-1157` |
| F3 | 设置页切子页**静默丢弃未保存表单**（无 dirty / 无 route leave 确认） | `App.vue:79`、`SettingsView.vue` |
| F4 | 一级文案与术语表不一致：「入库 / 切片 / 精修」仍出现在页面标题、面包屑与按钮上 | `IngestView.vue:3`、`App.vue:36`、`CommandPalette.vue:100`、`PolishView.vue:41-43` 等 |
| F5 | `ProjectClipsSection` 有一条被明令禁止的「读失败兜空」死分支（当前 props 路径不可达） | `ProjectClipsSection.vue:1303-1334` |
| F6 | 精修页把字幕**读失败**当成「没有字幕」，且无条件刷新 `persistedSubs`（会把热重载判据反着用坏） | `PolishView.vue:1437-1443` |
| F7 | `/api/health` 有 3 处绕过唯一缓存（注册页 / 入库页 / 封面设计器） | `RegisterView.vue:74`、`IngestView.vue:230`、`CoverDesigner.vue:381` |
| F8 | 小问题：`SubtitlePanel` 下标 key + 提交后重排；`HomeClipsOverview` 先转义再套 `<mark>`；`TimelineEditor` 跨目录 import `spec/effects.json`；`IngestView`「待分配」对非 admin 可见；`ProjectSettingsView.stageText` 只映射 0–4；`IngestView.retry` 死代码；多用户「指派给…」只有自己；admin 首页首屏 19 次 `fs/list` | 见 frontend 报告 §3 |

### 4.5 UI / 端到端

**已核对正常**：全站 0 未捕获异常、0 横向滚动；主流程可点通；任务抽屉/⌘K/删除确认/Esc/焦点进入弹层正常；
空态均为真实空态。1280 与 1680 两档截图 70+ 张。

| # | P2 | 位置 / 现象 |
|---|---|---|
| U1 | 精修页单次冷加载 **42 个 `/api` 请求**（含重复 2–3 份），叠加全局 600/min 限流会自撞：封面自动保存报 429「保存封面设置失败」 | `PolishView`、`CoverDesigner.vue:1641/1677`、`ratelimit.rs:47` |
| U2 | 首页首屏 **19 次 `POST /api/fs/list`**（用户根本没点添加录像） | `ServerDirPicker/ServerFileBrowser`、`WorkbenchView` 默认 source=local |
| U3 | 切片筛选条出现**两个「全部」**（一级/二级同款按钮，无可见分隔） | `ProjectClipsSection.vue:33/1756` |
| U4 | 切片卡右上斜角标签被 `overflow:hidden` 裁掉一角 | `ProjectClipsSection.vue:76/171` |
| U5 | 识别页字幕时间轴块内文字被硬裁，无省略号 | `PrepareView` 的 `.sub-tracks .block` |
| U6 | 首页「最近片段」卡片固定 140×123，1680 下整面墙只占 1/3 宽 | `HomeClipsOverview.vue:649` |
| U7 | 封面设计器 1280 下「文字」属性列被压到 202px（字体下拉截断、阴影开关落到滚动区外） | `CoverDesigner.vue:95` |
| U8 | 设置页无 dirty 保护（同 F3） | — |
| U9 | 登录空表单无反馈（**已修**） | `LoginView.vue` |
| U10 | 1280 精修页左「属性」列固定 232px，舞台只剩 ~339px（视频很小）；本轮只修了控制行溢出，**是否在无选中块时折叠属性列**是布局取舍 | `PolishView.vue` 的 `.props-host` |
| U11 | 1024 宽（未过 1000px 单列断点）仍有 3px 溢出（极窄档，既有） | `PolishView.vue` |

P3：`?mode=pro/quick` 已无对应 UI（文档/验收口径需更新）；精修页固定 2 个 404 + 2 条 WARNING 噪音；
项目设置页「直接编辑 yekitrim.toml高级」粘连；系统页用量 7 块在 6 列网格落单；侧栏搜索框无可见焦点环；深色主题未测。

## 5. 待你拍板（按建议优先级）

> 「建议」列是我的推荐；标 **推荐先做** 的是我认为投入产出比最高的。

| ID | 议题 | 选项 / 建议 | 影响面 |
|---|---|---|---|
| **D1** | `[cutting] overwrite` 死字段 | ①真实现（`false` 时跳过已存在且事件匹配的片段）②只摘 UI/模板 ③彻底删（走配置五步）—— **推荐 ①**，它是"重剪覆盖用户改过的片段"最常见抱怨 | 配置、剪切、设置页 |
| **D2** | 多用户面收口（P5） | ①`/api/recorder/*` 加归属过滤（admin 通吃）②设置/素材库入口对普通账号开放（后端已按 Owner 分，UI 仍 admin-only）③「指派给…」列全部账号 —— **推荐先做 ①+②** | 多租户安全/可用性 |
| **D3** | 安全硬化（部署公网前） | ①`fonts/import` 限本人项目/素材根或 admin ②`/api/health` 删 `fonts_dir` ③LLM `base_url` 禁止 loopback/私网开关（默认单人保留）—— **推荐 ①② 先做，③ 做成配置开关** | 公网/多用户 |
| **D4** | 配置写入健壮性 | `PUT /api/config/global` 改为「临时文件 → 真反序列化验 → rename」，坏值 400 且原文件不动；顺带项目 `missing` 补 `root_dir.is_dir()` 判据 | 数据安全 |
| **D5** | SSE 终态事件 | 补发 `completed/failed/cancelled` 后结束流；前端已有分支立刻生效，轮询退居兜底 | 任务体验 |
| **D6** | 批量请求与限流 | `PolishView` 的 project/dhampir/subtitles/fonts 收单飞；`CoverDesigner` 429 退避重试 + 文案区分；首页 `fs/list` 懒展开 —— **推荐先做**，它已实测制造 429 | 性能/稳定性 |
| **D7** | UI 细节包 | 两个「全部」、ribbon 裁切、时间轴省略号、卡片墙宽度、封面 1280 属性列、设置 dirty 保护 —— **建议成包做**（都不改数据模型） | 观感/可用性 |
| **D8** | 导出健壮性 | 无音轨素材：跳过音轨链或补静音轨 + 人话错误；ffmpeg 全部加超时；失败路径清 `_temp_*`；质量档（若产品需要） | 出片成功率 |
| **D9** | 发布清单正确性 | `clip.dm.srt` 逐条判存在；多段 `duration` 用内容时长；部分剪切失败是否算任务失败（产品） | 发布链路 |
| **D10** | ASR 算法/模型 | `[asr] model` 收进死字段或真实现；qwen3 recognizer 按 `(dir,max_tokens,threads)` 缓存/重建；少弹幕热窗口加绝对下限 | ASR 正确性/性能 |
| **D11** | 门禁真实化 | `drop-list` 缺样本 exit≠0（或 `--require-sample`）；`takumi_font_id` 纳入显式命令/CI；底座版本做 `MIN_HOST_API` 下界 | 质量门禁 |
| **D12** | 术语与文档 | 「入库→采集 / 切片→挑片段 / 精修→出片」是否统一；`docs/plan/webui-simplification-2026-11.md` 的 quick/pro 与 5 标签已加「现状更正」节，是否回写正文 | 文案/文档 |
| **D13** | 封面/预览遗留 | `cover.image` 覆盖关键帧（疑似老工程兼容，建议只在文件缺失时复制）；预览端口 LRU；`DROP` 报告是否升级为可见提示 | 封面/资源 |
| **D14** | 配额与容量 | 出片/转写产物是否纳入配额预检；`clips/index.json` 空目录过滤；同 `clip_dir` 并发写检测 | 多用户容量 |

## 6. 建议下一步（按顺序）

1. 你拍板 D1–D3（产品/安全面），我再开对应修复分支；D2/D3 是多用户上公网的前置。
2. D4/D5/D6 是低风险、可直接做的健壮性包（配置原子写、SSE 终态、单飞/退避）。
3. D7 做一轮 UI 细节包（含 1280 两处拥挤），配 70+ 张截图回归。
4. D8/D9/D10 需要你定产品口径（无音轨是否允许出片、部分失败是否算失败、`overwrite` 语义）。
5. 最后把本轮已修 + 后续修复一起重建 `webui/dist` 与 release 产物；本轮的 release 二进制未重建（只改了源码/dist）。
   ⚠️ 因此 `http://127.0.0.1:26321` 上：**前端修复已生效**（该实例用 `--webui-dir` 读盘），
   **后端修复要重新构建/重启才能看到**（jobs/channel-map/init 等）。

## 7. 覆盖与限制（如实）

- **未跑**：4K/长素材真出片；三端像素对拍（本机底座 host_api 6 vs 本仓期望 7，属既有已知红）；
  Takumi 封面真渲染（`cover-takumi` 未编）；portrait 真像素探针；非 admin 角色、深色主题、移动端窄屏；
  真实长任务（转写/检测/剪切/渲染/封面生成）未在 review 中触发（只读/短 fixture）。
- **识别页主按钮真实状态机**未取得 e2e 证据：demo 工程 stage=4，按钮按设计不渲染；mock 脚本收口时被打断。
- **「⋯ 更多 → 重新检测」二次确认**未拿到（点击器未命中菜单文本），对应两张截图是无效证据，不要引用。
- UI review 的副作用：设置页「保存配置」点过 1 次（值未变，`/api/config` 前后一致）；
  封面设计器挂载时的自动排版**可能**写过一次 `polish.toml`（测试 data-dir，非用户真实数据）。
- 本轮的修复全部**未 commit**；工作树同时含 review 之前就存在的 137 项未提交改动，提交时请按模块拆分。

---

### 附：原始证据

- `.workbuddy/review/backend.md`（20 KB，P1×2 + P2×8 + 已核对正常面）
- `.workbuddy/review/pipeline.md`（29 KB，P0×1 + P1×6 + P2×9 + P3×6）
- `.workbuddy/review/polish.md`（17 KB，P1×2 + P2×7 + P3×5）
- `.workbuddy/review/frontend.md`（20 KB，P1×2 + P2×8 + P3×9）
- `.workbuddy/review/ui.md`（19 KB，P1×1 + P2×9 + P3×6 + 页面×视口总表）
- `.workbuddy/review/shots/`（70+ 张 1680/1280 截图）
