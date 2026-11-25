# webui/src/views — 页面文件自身的纪律

> 范围：`webui/src/views`/** ｜ 上游：`webui/src/router.js`｜ 验证：`npm --prefix webui run build`

## 职责

9 个页面组件（清单见下「入口」，与 `router.js` 同源）。**壳与侧栏、路由表、重挂 key、设置页四子页、缓存与单飞、`useJobs` 退订、首页** 见 `webui/src/AGENTS.md`，本卡只写页面自身的纪律。

> 历史与实测数据见 **`docs/webui-views.md`**（§1 读失败的四处同源 bug / §2 LLM 剪辑 429 事故 / §3 预处理页沿革 / §4 切片页排序与迁移 / §5 设置页与模型管理 / §6 首页 / §7 静默刷新与 `:key`）。

## 入口

- `router.js` → `#/workbench`=WorkbenchView；`#/p/:pid[/prepare]`=PrepareView；`:pid/slices`=ProjectSlicesView；`:pid/polish[/:clip]`=PolishView；`:pid/settings`=ProjectSettingsView；`#/settings/<key>`=SettingsView；`#/ingest`=IngestView；`#/login|register`=Login/RegisterView。
- 共用子组件：`ProjectClipsSection.vue` / `FragmentList.vue` / `MaterialList.vue`（`ModelList` 已并入检查项，v8）；接口层 `api/{projectData,polish,models,auth}.js` 与 `/api/health`，查询串只经 `scopeQ(pid)`。

## 边界

**简化 / 每页一个主行动**
- 一级 CTA 唯一；空态 `components/AppEmptyState.vue`、下一步 `components/NextStepBar.vue`；能力入口带 `data-cap`（清单 `docs/webui-capabilities.json`），高级项默认折叠、保留可达。
- 出片页只有精修（快速切片已删）：右栏 / 时间轴**无条件渲染**，⚠️ 别用裸 `<template>` 包（内容不进 DOM）。识别页主按钮状态机见 `docs/plan/webui-simplification-2026-11.md`。

**读失败 / 静默刷新（全页面通用）**

- ⚠ **「读取失败」不许伪装成正常状态，也不许在基线不可信时写盘**（2026-09-16"静默丢改动"根因；四处复盘见 §1）：`loadData` 失败只置 `loadError`（不动数组、不清脏标记）、`flushSave` 开头 `if (loadError.value) return`、`saveText` 里 `loadError` 优先于 `saving`/`ok`。
  - **404 是空态不是错误**（`/api/polish/subtitles`：两份都没有就是没转写）；**`reload()` 不许静默丢弃**（忙 → `reloadQueued`，`flushSave` 的 `finally` 里补做）；**换片段先清空**（`commitEdit` 后 `resetTimelineState()` 再 `loadData()`）。
  - **可见性**：`fetchProject` 失败记 `cfgError`（否则渲染成「未初始化」+ 初始化按钮）；`clipsError` / `stickerSourcesError` 保留旧列表 + 错误条，**不许退化成假空态**；`CoverDesigner` 的 `configLoaded` 是**写入门闸**。
  - ⚠️ 保存失败用与「保存中…」同处的常驻文字（`.save-err`），不是一闪就没的 toast —— 读失败/写失败都必须留下可追溯痕迹。
  - **误删要有退路**：`TimelineEditor` 保留**最近一次删除**（`lastDeleted`，15 秒）+「撤销删除」，按**分段对象本身**认位；只有**分段**保留删除确认。
- **任务完成后的刷新必须静默**：`PolishView.load({silent:true})` 不置 `loading`、不卸载编辑器（非静默会把编辑器换成骨架屏，预览与滚动位置全丢）。需重新取数的子组件各用 `:key` 重挂（`segKey` / `coverKey` / `previewKey`，见 §7）。

**精修页 `PolishView`：LLM 剪辑四条硬规矩**

「LLM 剪辑」= 全部重做（2026-09-20 用户要求 + 事故：`apply_outputs` 曾无条件覆盖 `seg.events`，一次 429 把整段事件抹成 0）。四条硬规矩（推导见 §2）：

1. **空结果 / 失败一律不覆盖**（`llm/events.rs::apply_outputs`）：`output == None`（429/超时/解析失败）与 `Some([])` 都**跳过硬**，保留原有事件（钉子 `events.rs::tests`）。
2. **checkpoint 的窗口 id 必须带时间段**（`seg_{i}_{start}-{end}@fingerprint`）：命中条件只有 `id && items`，重新分段后新旧 `seg_1` 的 ±3s 字幕窗口**可能一字不差** ⇒ 会套错事件。
3. **逐段状态快照** `.yekitrim/polish_events.status.json`（`llm/status.rs`）：`cmd_llm` 落盘、`polish_project_get?preview=1` 注入 `_llm_status`、编辑器给**没产出事件的段**上琥珀色 + 失败原因 +「只重做本段事件」。
   ⚠ **落盘前必须 `remap_to_segments`**（`cmd_llm` 之后还会砍掉超出时长的段，窗口下标 ≠ 最终段下标）。成败**不看最终事件数** —— 空事件段会被 process 补上默认运镜，用 `events.is_empty()` 判会让失败显示成成功。
4. **入口语义**：`POST /api/polish/llm {full}` 的 `full` **默认 true = 全部重做**（先清 checkpoint；编辑器按钮再带二次确认，它含手工加的全部段与事件）；`llm-segment {index}` = **只重做这一段**（不重跑分段、不走 checkpoint、失败保留原事件）。CLI 仍走 `full=false`。

**精修页 `PolishView`：右栏按钮行有宽度预算**

- 右栏定稿（一块卡、三页签、行排版、任务栏折叠、出片通道已删）见 **`docs/polish-right-column.md`**；几何/多轮口径/踩过的坑都在那里，改右栏前先读。判据 ⑰ 钉着"块数/页签/无链接"。

**预处理页 `PrepareView`（一个按钮、两段任务）**

- ⚠️ **主按钮的置灰 = `busy || (!hasTranscript && !canRunAsr)`**，"跑完就永久置灰"作废（2026-09 用户改口）：两段都就绪时它变成**「重新检测」**（`confirmDialog(redetectWarning())` 二次确认，只重跑事件检测，**不做全流程重跑**）；已有字幕时它**永不置灰**。
- ⚠️ **转写按 ASR 模式各绑一次**（旧口径"一生只跑一次"作废）：落盘即占掉那个模式的槽（`bind_asr_artifact`；`slots` 与后端 `slot_bound` 同口径）。**两格都占满才**一起置灰 —— 单格占满仍可切另一个模式。服务端 409 同口径但**只拦 HTTP**（CLI 是逃生口）。细节见 §3.2。
- ⚠️ 行内还有一颗 **「跑 ASR」**（2026-09）：**只转写、不接检测**（`chainDetect = false`），是"换模式再出一份"的入口；`canRunAsr = !slotUsed(当前模式)`，两模式都跑过时文案变「已跑过」。
- ASR 模式下拉在这页（`hybrid` / `single`，体积写 `title`）；值读 merged 配置（读失败回退 `hybrid`），写走 `PUT /api/project-data/config {patch:{asr:{mode}}}`。LLM 订正入口已从前端撤下。
- ⚠️ **清单落地形态 = `[project.materials]` 下的 `transcript` / `danmaku` 数组**（`MaterialEntry{ path, source, mode }`）：`*_file` 仍是**唯一生效值**；`source = "asr"` 才是 ASR 产物、`mode` 记模式；`source` 用 **String 不用 enum**（坏 toml 不该让整份配置读失败）。新增 `project.*` 字段记得同步补 `merge()` 与 `normalize_project_paths_in()`。
- ⚠️ **ASR 产物不可删** —— 它就是那个模式用掉的转写名额（唯一例外：文件已不存在，`removable = !is_asr || !exists`）。**「删除」只从选择里移除**（清 `*_file` + 摘清单），**不删磁盘文件**，二次确认；**「切换」= 置为生效值**（没登记过也认），切完整页 `load()`。
- ⚠️ 条目 `exists=false` 时**照实报「文件不存在」**（置灰 + 删除线、切换也灰），不要静默飘到 `output/` 兜底上 —— 用户选的这份没了必须让他知道（`api/config.rs::check_project_paths` 同口径）。
- ⚠️ 清单读失败（`assets` 为 null）时**不要**碰 `assetDirty`（`activate` 在接口成功后才设），否则"素材已切换"会凭空冒出来。
- **改完不自动跑任务**（LLM 有成本）：只把提示切成「素材已切换 —— 点按钮重新检测才生效」。
  ⚠️ `assetDirty` 只在**检测真的跑了**之后才清（`chained || kind === "stage_detect"`）：单独跑一次「跑 ASR」不消费切过的弹幕/字幕。
- ⚠️ `slots`（`{hybrid, single}`）**由服务端算**（遍历 `ASR_MODES` 调 `slot_bound`），前端**不自己从 `items` 推** —— 判槽还要看盘上没登记的老产物，抄一份就会漂。
- ⚠️ **清单写入必须单次 `patch_config_file`**（生效值 + 整份清单同一 patch）：拆两次会连撞两次内部路径锁（`with_path_lock_sync` 非重入），中间还会"值指向新文件、清单还是旧的"。
- ⚠️ 非 admin 的清单路径也要过 jail：`check_project_paths` 末尾的 `check_material_paths` 对 `[project.materials].{transcript,danmaku}[].path` 逐条 `jail_join` —— 漏一条就是逃逸口。

**切片页 `ProjectSlicesView`（+ `ProjectClipsSection`）**

- ⚠️ 两个踩过的坑：①**顺序必须按数字序号**（`clip-100` 在 `clip-02` 之后）—— 服务端与 `loadClips` 都按 `rankOfName()` 排过，别退回字符串排序；②`.frag-cards` 是 `grid-auto-flow:column`，**必须写 `justify-content:start` + `grid-auto-columns:max-content`**（缺省 stretch 会把列撑满整行）。
- ⚠️ 「产物」栏的名字别退回写死的 `renderedName`/`coverName`（它们从没被赋值，恒为 `export.mp4`/`cover.jpg`）：现在取 `clipEntry.rendered_file`/`cover_file`。
- ⚠️ **剪切完成 ⇒ 精修工程已就绪**（服务端在剪切收尾里 init，见 `crates/yekitrim-service/AGENTS.md`）：「精修」按钮正常情况下直接进页面；只有**老片段**才需要补一次 init，那条路**必须等任务跑完再跳**（发完就跳会先撞 `polish.toml not found`）。
- ⚠️ **删除不是任务（不发 job），刷新只能由前端自己驱动**：`ProjectClipsSection` 拿的是父页面的**快照**，删完要 `emit("changed")` 让父页面 `load({silent:true})` 重取（否则再点就是 404）；同规矩适用于改事件与新建片段。
- ⚠️ `readSort()` 的回落必须用 **`SORT_OPTIONS[0]`**：以前写死 `key: "rank"`，换成 popularity 之后旧偏好失配 → 排序栏一个都不高亮、还按不存在的键静默排序（实测踩过）。
- 筛选两级（v6.7）：一级 **已剪切 / 未剪切 / 全部（最后）**（`filterKey`），二级 **全部 / 精修中 / 已导出**（`polishFilter`）**只在选中「已剪切」时渲染**（`.subfilters` 内嵌分组）；挂到另两档上是错的（"点了没反应"）。
- ⚠️ 二级判据（2026-09 改口径）= **`isExported`（`rendered || cover`）**，与卡片那枚 `已导出` chip 同一套。**别再改回"进没进过精修"**：**剪切收尾本身就会 `polish init`** ⇒ `polish_ready` 恒为真，旧两档退化成"恒空 / 恒满"。
- ⚠️ localStorage 迁移（`yekitrim.slices.filter` / `.subfilter`）：`LEGACY_FILTERS` / `LEGACY_SUB_FILTERS` 把旧档映射到新口径，其中 `polished`→`exported` **有损且有意**（旧档含"进过精修但没出片"那批）；认不出回落默认档。映射表见 §4.7。
- 卡片状态只有一枚 chip **`已导出`**（`isExported`）、没导出**不挂 chip**，与筛选栏**同一判据**（不会"某档高亮、卡片挂另一个词"）。
  ⚠️ 历史上这里写着「别把它统一回已导出」——**已被用户推翻**。
- 操作条 = 重新检测 / 添加片段 / **一键全切（N）**（`cutAll`）。**「重新检测」在已有结果时必须二次确认**（`redetectWarning()`，与预处理页同一份文案）：重跑只覆盖 `output/events.json`，**`output/clips/` 一个文件都不动**。
- **「弹幕量」排序**：有 `clip.dm.srt` 就用它，**没有（老切片）就按 `clip.json` 的 `original_start/end` 从源弹幕里数**（`count_in_range`）—— 否则整列 0，排序静默退化成按 rank。

**字体库** = 素材库的字体卡片（`components/MediaLibrary.vue`）：见 `docs/webui-components.md` ⑥。

**项目设置页 / 设置页 / 模型管理 / 登录注册 / 首页**

- 项目设置 `#/p/:pid/settings`：**「删除源录播」**（真删媒体，唯一的手动释放空间手段）与 **「移除项目」**（只注销登记、零文件改动）。两个入口（设置页按钮 + 侧栏项目行的 `×`）文案与行为必须一致：**都不是 `danger` 红色**，但仍二次确认。移除后要调 `forgetProject(pid)` 清本地痕迹（per-pid 缓存 / `yekitrim.wb.*` / `yekitrim.polish.last.<pid>`）。
- ⚠️ **「移除项目」不碰任何文件**（用户拍板）：`yekitrim.toml`、媒体、`output/` 全留着，重新导入即可再绑（`create_local_project` 见到已有 `yekitrim.toml` **只登记、不覆盖**）。两个入口共用 `api::unregister_project` —— 以前是两份拷贝各删一次，别再写回两份。
- 设置页 `SettingsView.vue` 五块内容仍在**同一个文件**（`defineProps({section})` + `v-if` 切），不拆子组件。
- ⚠️ 「未保存的改动」的基线要在 `await nextTick()` 之后拍：加载期 `flush:"pre"` 的 watcher 会补数据，同步拍 = 刚进页面什么都没点就 dirty。
- ⚠️ **LLM 配置块的「按链路覆盖」是表格**（L3 起）：行 = 链路（后端 `override_links` 给），列 = 接口/模型/温度/输出上限/推理/状态；
  未覆盖的格子显示 `/api/config/llm/effective` 的**灰字实值**，**点一下才可编辑**，「跟随」按钮发 `null` ⇒ 删键。
  **别退回"空输入框表达没有值"**（发空串更糟：`max_tokens = ""` 会让整份 llm.toml 被打回默认）。
- **模型管理（别再各写一套）**：「已安装」与运行时**同源** —— `asr::local::resolve_model_dir` 是唯一真源，`download::model_install()` 在其上再校验注册表 marker（`tokens.txt` / `tokenizer/tokenizer_config.json`，**不是** `dir.exists()`）；界面显示实际路径与来源（细节 §5.4）。
- ⚠️ `/api/models` 与运行时**必须同源**（曾经只查程序目录 `dir.exists()`，qwen3-asr 在**另一处** ⇒ 界面写着"未下载"却一直在用它）；列表仍由 `download::model_names()` 给出。⚠️ 资产收回程序目录前的"用户目录优先"档**已删除** —— 现在只有两档：配置覆盖、数据目录。
- ⚠️ **`download::ensure_one_model`（转写前的自动守卫）也必须走 `model_install`**，不要再拼 `yekitrim_home()`：曾经写一处、运行时读另一处 ⇒ ①同一份模型下载两次；②**同音纠错的 `lexicon.txt` + `replace.fst` 与 `create_recognizer` 不同源 ⇒ 同音纠错从未生效**（日志一直是"未启用"）。
- **登录 / 注册页**（`LoginView` / `RegisterView`，壳外样式）：能力位只来自 `/api/health`（`registration_enabled` / `accounts` / `must_change_password`，**不含任何路径**，它无鉴权）—— `#/register` 未开放时直接说明而不是给表单。**服务端仍是最终裁判**：注册失败一律 403（不区分"码不对/码用过"）、限流按 IP（5 次锁 15 分钟）。
- 首页 `#/workbench`（`WorkbenchView`）：来源按角色过滤 + `source` watch 保证可见可用、非 admin 整条不显示、`roomsAvailable` 初值 `null`、录播姬不可用整块置灰 —— 其余见 `webui/src/AGENTS.md`「首页」；页面自身只多一条：SSE 进度须在 `onUnmounted` 退订 `onJobDone`。

> 改动只允许落在本目录；跨模块改动先读对方卡，并跑两条门禁。

## 验证

```bash
npm --prefix webui run build
```

前端没有单测运行器：**这条构建就是本卡的门禁**（`vite build` 会解析模板与全部 import）。
改完 `webui/src` 必须重建 `webui/dist` 并一起提交。
