# webui/src/views — 页面文件自身的纪律

> 范围：`webui/src/views`/** ｜ 上游：`webui/src/router.js`｜ 验证：`npm --prefix webui run build`

## 职责

9 个页面组件（清单见下「入口」，与 `router.js` 同源）。**壳与侧栏、路由表、换项目重挂 key、`:clip` 用目录名、
设置页五子页、缓存与单飞、`useJobs` 退订、首页新增项目 / 录播姬 / B站** 见 `webui/src/AGENTS.md`，
本卡不重复；这里只写页面文件自身的纪律（读失败、静默刷新、置灰判据、文案口径）。

## 入口

- `router.js` → `#/workbench`=`WorkbenchView`；`#/p/:pid` 与 `:pid/prepare`=`PrepareView`；
  `:pid/slices`=`ProjectSlicesView`；`:pid/polish[/:clip]`=`PolishView`；`:pid/settings`=`ProjectSettingsView`；
  `#/settings/<key>`=`SettingsView`；`#/ingest`=`IngestView`；`#/login`/`#/register`=`LoginView`/`RegisterView`。
- 共用子组件：`components/ProjectClipsSection.vue`（props 模式下 `load()` 只照抄父页面快照）、
  `components/FragmentList.vue`、`MaterialList.vue`、`ModelList.vue`（`/models` → `/settings/account`）。
- 接口：`api/projectData.js` / `polish.js` / `models.js` / `auth.js` / `/api/health`；查询串只经 `scopeQ(pid)`。

## 边界

**读失败 / 静默刷新（全页面通用）**

- ⚠ **「读取失败」不许伪装成正常状态，也不许在基线不可信时写盘**（2026-09-16，一批"静默丢改动"的根因）。
  同一类 bug 在精修链路上出现过四处，这几条一起守：
  - **读失败 → 可见 + 暂停写入**：`TimelineEditor::loadData` 直接调 `api()`（**不再用 `apiOr` 兜底**），
    失败只置 `loadError`、**不动现有数组也不清脏标记**；`flushSave` 开头 `if (loadError.value) return`。
    没有这道闸，读失败会被当成"空时间轴"，用户拖一下就把 `segments: []` PUT 回去 —— 磁盘上的分段/字幕
    就这么没了。横幅 + 「重试」在 `tl-head` 下面（**不能放进 Teleport 的属性面板**：挂载点解析不到
    就整块不渲染）。`saveText` 里 `loadError` 必须优先于 `saving`/`ok` —— 这时写着"已保存"是骗人的。
  - **`/api/polish/subtitles` 的 404 是空态、不是错误**：`clip.srt` 与 `data/subtitles.json` 都不存在时
    服务端就返回 404「暂无字幕文件」，没转写的片段本来就是这样。**只有非 404 才算读失败** ——
    否则每个没有字幕的片段都会挂一条"读取失败，已暂停保存"，还把保存一起锁掉。
  - **`reload()` 不许静默丢弃**：老写法 `if (drag || saving || saveTimer) return` 会被 250ms 防抖窗口
    挡掉 —— 编辑器留着后处理**之前**的值，紧接的 `flushSave` 又把旧值 PUT 回去，把刚算出的贴纸落点/锚点
    整个顶掉且没人重试。现在：忙 → 置 `reloadQueued`，`flushSave` 的 `finally` 里 `drainQueuedReload()`
    补做；真要跑时若还有没落盘的改动，**先落盘再重载**；期间再收到请求就再跑一轮。
  - **换片段先清空**：`watch(() => props.dir)` 里 `commitEdit` 之后必须 `resetTimelineState()` 再 `loadData()`。
    不清空的话，新片段读取失败时数组还留着**上一个片段**的分段/字幕，用户一次编辑就把它们写进新片段目录
    （跨片段串写）。清空 + `loadError` 拦写，两道一起才安全。
  - 配套的**可见性**：`PolishView` 页面级读取（`fetchProject`）失败要记 `cfgError` —— 否则 `ready=false`
    会渲染成「未初始化精修项目」+ 一颗「初始化」按钮，用户会把**已存在**的工程当成没初始化再点一次；
    但**要带 `!ready` 门控**，已经有数据时的静默刷新失败只加一条横幅、绝不卸载编辑器（见上一条）。
    切片列表（`clipsError`）与贴纸来源（`stickerSourcesError`）同理：失败保留上一次的列表 + 错误条，
    **不能退化成"还没有可精修的切片"/"贴纸库为空"**（假空态让人去查库而不是查服务）。
  - `CoverDesigner` 的 `configLoaded` 是**写入门闸**（读失败时拿默认值写回会把磁盘配置清成默认）：
    读失败必须出错误条 + 「重试」；保存失败用与「保存中…」同处的常驻文字（`.save-err`），不是一闪就没的 toast。
  - **误删要有退路**：删除是"双击即生效 → 250ms 落盘 → 随后自动后处理"，点完就不可逆 ——
    `TimelineEditor` 保留**最近一次删除**（`lastDeleted`，15 秒）给头部一枚「撤销删除」，
    事件按**分段对象本身**认位（后处理会整体改写 segments，下标会漂）；只有**分段**保留删除确认。
- **任务完成后的刷新必须静默**：`PolishView.load({silent:true})` 不置 `loading`、不卸载编辑器
  （非静默会把编辑器换成骨架屏，预览与滚动位置全丢）。需重新取数的子组件各用 `:key` 重挂：
  `TimelineEditor`=`segKey`（LLM 剪辑/初始化/去静音）、`CoverDesigner`=`coverKey`（封面完成 ——
  渲染与封面已解耦，render 钩子不碰它）、`PolishPreview`=`previewKey`（任何任务完成）。

**精修页 `PolishView`：LLM 剪辑四条硬规矩**

「LLM 剪辑」= 全部重做（2026-09-20 用户要求 + 一次真实事故：`apply_outputs` 曾无条件覆盖 `seg.events`，
一次 429 就把整段事件抹成 0）。四条硬规矩：

1. **空结果 / 失败一律不覆盖**（`llm/events.rs::apply_outputs`）：`output == None`（429/超时/解析失败）
   与 `Some([])`（模型返回空）都**跳过硬**，保留原有事件。回归钉子：`events.rs::tests`（空/失败/正常三条）。
2. **checkpoint 的窗口 id 必须带时间段**（`seg_{i}_{start}-{end}@fingerprint`）：命中条件只有
   `id && items`，而重新分段后「新的 seg_1」与「旧的 seg_1」的 ±3s 字幕窗口**可能一字不差**
   → 会把别的时间段的事件套过来（"续传之后显示不正常"）。带区间后重新分段必然失配。
3. **逐段状态快照** `<project>/.yekitrim/polish_events.status.json`（`llm/status.rs`）：`cmd_llm` 落盘、
   `polish_project_get?preview=1` 注入 `_llm_status`、`TimelineEditor` 给**没产出事件的段**上琥珀色，
   分段面板显示失败原因 + 「只重做本段事件」。
   ⚠ **落盘前必须 `remap_to_segments`**（`cmd_llm` 之后还会砍掉超出时长的段，窗口下标 ≠ 最终段下标）。
   成败**不看最终事件数** —— 空事件段会被 process 补上默认运镜，用 `events.is_empty()` 判会让失败显示成成功。
4. **入口语义**：`POST /api/polish/llm {full}` 的 `full` **默认 true = 全部重做**（先清 checkpoint；
   编辑器按钮再带一次二次确认，因为它会重写当前全部段与事件，含手工加的）；
   `POST /api/polish/llm-segment {index}` = **只重做这一段**（不重跑分段、不走 checkpoint、
   失败保留原事件并让任务失败可见），跑完照常自动 `process`。
   CLI `yekitrim polish llm` 仍走 `full=false`（长片重跑贵，续传留给命令行）。

**精修页 `PolishView`：右栏按钮行有宽度预算**

- ⚠️ **右栏 = 两块常驻卡片**（2026-10-06 第三轮用户口径：「把轨道设置放在右侧常驻，改为项目设置
  和 llm 剪辑导出之类的合并，画面尺寸退回原来的那个页面」）：①**画面尺寸**（`[canvas]` 横屏/竖屏 +
  缩放 / 主画面位置 / 背景）自己一块；②**项目设置 / 输出**（LLM 剪辑 + 渲染导出 + 一跳设置页链接）。
  **两块都常驻、没有页签**（页签第一轮就删了）。⚠️ **两块必须包在同一个 grid 子节点 `.right-col` 里** ——
  `.edit-grid` 是两列（`1fr 300px`），直接摊成两个子节点时第二块充当"第三列"**掉到下一行占满整宽**
  （实测 1045px，本轮真踩到）。判据 ⑰。字幕/弹幕字体与偏移 · 贴纸来源 · 框选落点仍在**轨道设置**。
- ⚠️ **轨道设置只留控件**（第三轮：「仅信息展示的内容删除，例如：分段 / 屏蔽」）：「分段 / 屏蔽」
  （两个计数、零控件）与「音效」（一条全局音量滑杆，事件自己的 `volume`/`sfx_volume` 优先 ⇒
  改了看不出效果；要改走 `polish.toml` 的 `[sfx] volume`）**整组删**。现在只剩三段：
  字幕 / 弹幕 · 框选区域 · 贴纸。⚠️ **两颗框选按钮（弹幕区域 / 人物）必须同一行**（它们原分居
  「字幕/弹幕」与「运镜」两组）；判据 ⑰ 用 `getBoundingClientRect().top` 相等钉。
- **出片通道下拉已删除**（2026-10-01）：出片只剩底座一条腿，兜底腿已整条删除 —— 留一个只有一个
  选项的下拉只会让人以为"还有得选、只是我没选对"。`runRender` 不再收 `renderer` 参数。

**预处理页 `PrepareView`（一个按钮、两段任务）**

- ⚠️ **主按钮的置灰 = `busy || (!hasTranscript && !canRunAsr)`**，"跑完就永久置灰"作废（2026-09
  用户改口）：两段都就绪时它变成**「重新检测」**，点击走 `confirmDialog(redetectWarning())`
  二次确认，只重跑事件检测（切了弹幕/字幕后要生效就是点它）。**不做全流程重跑**。
  已有字幕时它**永不置灰**（检测随时能重跑）；只有"一个字幕都没有、而当前模式的 ASR 槽已经占了"才灰
  （按钮本要点的那次转写后端也会 409），此时引导用户去清单里选字幕或换模式。
- ⚠️ **转写按 ASR 模式各绑一次**（2026-09 用户要求"ASR 输出的字幕要标识模式、输出一次就在
  yekitrim.toml 里绑定一次、只能两次、用尽后转写入口置灰"；旧口径"转写一生只跑一次、跑过就再没有
  转写入口"作废）：产物落盘时 `assets::bind_asr_artifact` 把它**登记进素材清单**并置为生效字幕，
  那个模式的槽就算用掉了（`GET …/assets` 的 `slots`，与后端 `assets::slot_bound` 同口径）。
  **两格都占满才**一起置灰（`canPickAsr = slotFree` + 任务运行中）—— 单格占满仍可切到另一个模式去跑，
  那正是"两种模式各出一次"的正常路径，别写成"有产物就灰"；要再出一份就换另一个模式。
  服务端同口径在 `project_data_pipeline` 里守着（`stage == "transcribe"` 且 `slot_bound(模式)` → **409**），
  但**只拦 HTTP 这一侧**：CLI `yekitrim transcribe` 不受影响，是转写崩在中途时的逃生口。
- ⚠️ 行内还有一颗 **「跑 ASR」**（2026-09）：**只转写、不接检测**（`chainDetect = false`），
  是"换模式再出一份"的入口；`canRunAsr = !slotUsed(当前模式)`，两模式都跑过时文案变「已跑过」。
  跑完产物进字幕列表并自动成为生效值，要不要接着检测由用户点主按钮决定。
- ASR 模式下拉在这页（`hybrid · 更准（双模型）` / `single · 更快（单模型）`，大小放 `title`：165 MB / 879 MB）；
  值读 merged 配置（`GET /api/project-data/config` 的 `config.asr.mode`，读失败回退 `hybrid`），
  写走 `PUT /api/project-data/config {patch:{asr:{mode}}}`。LLM 订正入口已从前端撤下。
- ⚠️ **清单落地形态 = `[project.materials]` 下的 `transcript` / `danmaku` 数组**
  （`MaterialEntry{ path, source, mode }`，2026-09）。`*_file` 仍是**唯一生效值**，下游读取口径
  一个都没动（`transcript_path` / `danmaku_path`）。`source = "asr"` 才是 ASR 链路产物、
  `mode` 记它属于哪个模式；`source` 用 **String 不用 enum** —— 手写坏的 toml 不该让整份配置
  `load_config_at` 报错。新增 `project.*` 字段记得同步补 `merge()`（非空才覆盖）与
  `normalize_project_paths_in()`（条目路径也要相对化）。
- ⚠️ **ASR 产物不可删** —— 它就是那个模式用掉的转写名额；唯一例外是**文件已不存在**
  （`removable = !is_asr || !exists`，文件没了就只剩个死条目）。**「删除」只从选择里移除**
  （清 `*_file` + 摘清单，正好是生效值才连带清），**不删磁盘文件**，走 `confirmDialog` 二次确认。
  **「切换」= 置为生效值**（清单里没登记过也认，选中即补登记），切完整页 `load()`（字幕轨读生效值）。
- ⚠️ 条目 `exists=false` 时**照实报「文件不存在」**（置灰 + 删除线、切换按钮也灰），
  不要静默飘到 `output/` 兜底上 —— 管线确实会退到 `output/`，但用户选的这份没了必须让他知道
  （`api/config.rs::check_project_paths` 同一条口径）。
- ⚠️ 清单读失败（`assets` 为 null）时**不要**碰 `assetDirty`（`activate` 在接口成功后才设），
  否则一句"素材已切换"会凭空冒出来。
- **改完不自动跑任务**：只把提示切成「素材已切换 —— 点下面的按钮重新检测才会生效」（LLM 有成本，
  **别改成保存即起任务**）。⚠️ `assetDirty` 只在**检测真的跑了**之后才清（`chained || kind === 'stage_detect'`）：
  单独跑一次「跑 ASR」不消费切过的弹幕/字幕，那句提示要留着。
- ⚠️ `slots`（`{hybrid, single}`）**由服务端算**（遍历 `yekitrim_core::config::ASR_MODES` 调
  `slot_bound`），前端**不自己从 `items` 推** —— 判槽还要看盘上那些没登记的老产物，抄一份就会漂。
- ⚠️ **清单写入必须单次 `patch_config_file`**（生效值 + 整份清单走同一个 patch，
  见 `assets.rs::write_state` 的注释）：拆两次会连撞两次它内部的路径锁
  （`filelock::with_path_lock_sync`，**非重入**的 std Mutex），中间那一小段还会出现
  "值指向新文件、清单还是旧的"的错位。
- ⚠️ 非 admin 的清单路径也要过 jail：`check_project_paths`（`api/config.rs`）末尾的
  `check_material_paths` 对 `[project.materials].{transcript,danmaku}[].path` 逐条 `jail_join`
  —— 别的写 `[project]` 的口子都校验了，这里漏一条就是逃逸口。

**切片页 `ProjectSlicesView`（+ `ProjectClipsSection`）**

- ⚠️ 两个踩过的坑：①**顺序必须按数字序号**（`clip-100` 在 `clip-02` 之后）——
  服务端 `data_clips_impl` 与 `PolishView.loadClips` 都按 `rankOfName()` 排过，别退回字符串排序
  （会得到 clip-01, clip-02, clip-100, …, clip-11，卡片序号也跟着错）；②`.frag-cards`
  是 `grid-auto-flow:column`，**必须写 `justify-content:start` + `grid-auto-columns:max-content`** ——
  缺省 `normal`(=stretch) 会把 auto 宽的列撑满整行，只有两张卡时它们被推到两端。
- ⚠️ 「产物」栏的名字原来写死（`renderedName`/`coverName` 从没被赋值，恒为 `export.mp4`/`cover.jpg`），
  用户以为被覆盖了 —— 现在取 `clipEntry.rendered_file`/`cover_file`，别退回写死的名字。
- ⚠️ **剪切完成 ⇒ 精修工程已就绪**（服务端在剪切收尾里 init，见 `crates/yekitrim-service/AGENTS.md`）：
  卡片的「精修」按钮正常情况下直接进页面；只有**老片段**（这次改动前剪出来的）才需要补一次 init，
  那条路**必须等任务跑完再跳**（发完就跳会先撞 `polish.toml not found` → 精修页显示「片段配置读取失败」）。
- ⚠️ **删除不是任务（不发 job），刷新只能由前端自己驱动**：`ProjectClipsSection` 的数据是父页面
  传下来的**快照**，删完不 `emit('changed')` 让父页面 `load({silent:true})` 重新取数，用户再点就是
  404「clip-XX 不存在」（报过「删除会报错找不到内容」）。同规矩适用于改事件与新建片段。
  `silent` 必须真静默（否则整面墙换骨架屏）；删除中禁用防连点；404 当"已经不在了"提示并刷新。
- ⚠️ `readSort()` 的回落必须用 **`SORT_OPTIONS[0]`**：以前写死 `key:'rank'`，换成 popularity 之后
  旧偏好失配 → 排序栏一个都不高亮、还按不存在的键静默排序（实测踩过）。
- 筛选两级（v6.7）：一级 **已剪切 / 未剪切 / 全部（最后）**（`filterKey`），二级 **全部 / 精修中 / 已导出**
  （`polishFilter`）**只在选中「已剪切」时渲染**，紧跟「已剪切」之后（`.subfilters` 内嵌分组）；
  挂到「未剪切 / 全部」上是错的（那两档下会"点了没反应"）。
- ⚠️ 二级判据（2026-09 改口径）= **`isExported`（`rendered || cover`）**，与卡片那枚 `已导出` chip
  **同一套**；「精修中」= `!isExported`。**别再改回"进没进过精修"**（`isInPolish` =
  `polish_ready || rendered || cover`）：**剪切收尾本身就会 `polish init`**，每个已剪片段都
  `polish_ready` → 那个判据恒为真，旧两档退化成"恒空 / 恒满"。
  默认（`applyDefaultFilter`）：有已剪切则选「已剪切」，否则「未剪切」，**不写盘**；二级默认 `SUB_FILTERS[0]`。
  读数分母 = 一级档位涉及的片段数（`poolTotal`）；空态文案随二级变（`emptyText`）。
- ⚠️ localStorage 迁移（存 key：`yekitrim.slices.filter` / `.subfilter`）：`LEGACY_FILTERS` 把上一版
  `polish` → 已剪切 + 已导出；二级旧值走 `LEGACY_SUB_FILTERS`（`unpolished`→`polishing`、
  `polished`→`exported`）。后一对**有损且有意**（旧 `polished` 含"进过精修但没出片"那批，
  新口径只有"已导出"能表达；旧两档已退化，继续认只会让用户看到筛不出东西的按钮）。认不出回落默认档。
- 卡片状态只有一枚 chip **`已导出`**（`isExported`）、没导出**不挂 chip**，与筛选栏**同一判据**（不会"某档高亮、
  卡片挂另一个词"）。⚠️ 历史上这里写着「别把它统一回已导出」——**已被用户推翻**。
- 操作条 = 重新检测 / 添加片段 / **一键全切（N）**（`ProjectSlicesView` 调 `cutAll`）。
  **「重新检测」在已有检测结果时必须二次确认**（`utils.js::redetectWarning()`，与预处理页共用同一份文案）：
  重跑只整份覆盖 `output/events.json`，**`output/clips/` 一个文件都不动**；事件按热度重新编号后目录名
  `clip-NN`（= rank）会与新事件对不上 —— 文案要提醒"需要时删掉重切"，别写成"会删片段"。
- **「弹幕量」排序**：有 `clip.dm.srt` 就用它，**没有（老切片）就按 `clip.json` 的 `original_start/end`
  从源弹幕里数**（`count_in_range`）—— 否则整列 0，排序静默退化成按 rank。

**字体库（`components/FontLibrary.vue`，首页内嵌区块，仅 admin，无单独页面）**：见
`docs/webui-components.md` ⑥ 与 `docs/plan/font-library.md` §12/§21。

**项目设置页 / 设置页 / 模型管理 / 登录注册 / 首页**

- 项目设置 `#/p/:pid/settings`：**「删除源录播」**（真删媒体，唯一的手动释放空间手段）与
  **「移除项目」**（只注销登记、零文件改动）。两个入口——设置页按钮与侧栏项目行的 `×`
  （`ProjectNode` → `SideNav` → `App.vue::removeProject`）——文案与行为必须一致：
  **都不再是 `danger` 红色**（移除已不是破坏性操作），但仍然二次确认（误点一下项目就从列表里没了）。
  移除后前端要调 `stores/workbench.js::forgetProject(pid)` 清本地痕迹（per-pid 缓存 / `yekitrim.wb.expanded`
  / `yekitrim.wb.last` / `yekitrim.polish.last.<pid>`）—— id 是 `AUTOINCREMENT` 不复用，但会永远躺在本地存储里。
- ⚠️ **「移除项目」不碰任何文件**（用户拍板）：`yekitrim.toml`、媒体、`output/` 产物全留着，
  重新导入同一个目录即可再绑回来（`create_local_project` 见到已有 `yekitrim.toml`
  **只登记、不覆盖**，所以手改过的工程设置不会丢；新登记会拿到**新 id**，
  项目目录内的产物都不含服务器侧 id，换 id 安全）。
  两个入口共用 `api::unregister_project`（`DELETE /api/projects/:id` 与过渡别名
  `POST /project/delete`）—— 以前是两份拷贝、各删一次 `yekitrim.toml`，别再写回两份。
- 设置页 `SettingsView.vue` 五块内容仍在**同一个文件**（`defineProps({section})` + `v-if` 切），
  不拆子组件；`account` = 注册与邀请码 + **模型管理**（`components/ModelList.vue`）。
- ⚠️ **LLM 配置块的「按链路覆盖」= 一行一条链路：勾选框 + 两个 `SliderField` + thinking 下拉**。
  "跟随全局"由**勾选框**表达（不勾 = 发 `null` ⇒ 服务端 `patch_config_file` 删键），**别用空输入框** ——
  SliderField 的清空会拒绝写模型，"没有值"它表达不出来；发空串更糟（`max_tokens = ""` 会让整份
  llm.toml 被打回默认）。行由 `llm_config.overrides` 的键自动生成，后端加链路前端不用改。
  几何：`max-content + minmax(0,300px)×2 + max-content` + `justify-content:start`（1fr 会把滑块拉到格子
  中央、一行散开），实测 1600/1280/1000/860 四档都无横向溢出、控件同行；窄屏 900px 媒体查询拆成一列。
- **模型管理（别再各写一套）**：模型管理页的「已安装」与运行时**同源** ——
  `yekitrim_core::asr::local::resolve_model_dir`（配置 `local_model_dir` → `<数据目录>/.yekitrim/models`，
  默认即程序目录）是唯一真源，`download::model_install()` 在其之上再校验注册表
  marker（`tokens.txt` / `tokenizer/tokenizer_config.json`，**不是** `dir.exists()`）。
  接口 `/api/models` 回 `{name, installed, dir, source, source_label}`，界面显示实际路径与来源。
- ⚠️ 曾经 `/api/models` 只查程序目录 `dir.exists()`，而 qwen3-asr 放在**另一处** →
  界面显示"未下载"却一直在用它（用户报过）；列表也改成由注册表 `download::model_names()` 给出，
  别再手写模型数组。`yekitrim download` / `POST /api/models/download` 装到 `resolve_model_dir` 的落点。
  ⚠️ 2026-10-02 资产收回程序目录之前还有"用户目录优先"那一档，**已删除** —— 现在只有两档：
  配置覆盖、数据目录。
- ⚠️ **`download::ensure_one_model`（转写前的自动守卫）也必须走 `model_install`**，
  不要再拼 `yekitrim_home()`：曾经它写一处、而运行时读的是**另一处**，后果有两个 ——
  ① 同一份模型被下载两次（其中一份永远不会被读）；
  ② **同音纠错的 `lexicon.txt` + `replace.fst` 下到一处，而 `create_recognizer` 读的是另一处那份
  → 同音纠错从未生效**（日志一直是"未启用"）。
- **登录 / 注册页**（`LoginView` / `RegisterView`，壳外样式）：能力位只来自 `/api/health`
  （`registration_enabled` / `accounts` / `must_change_password`，**不含任何路径**，它无鉴权）——
  `#/register` 在 `registration_enabled` 为 false 时直接显示"未开放"而不是给表单，
  登录页只在开关打开时露出「没有账号？用邀请码注册」入口。**服务端仍是最终裁判**：
  注册失败一律 403（不区分"码不对/码用过"）、限流按 IP（5 次锁 15 分钟），前端不自己判"码有效"。
- 首页 `#/workbench`（`WorkbenchView`）：来源按角色过滤 + `source` watch 保证可见可用、非 admin 整条不显示、
  `roomsAvailable` 初值 `null` 避免先闪一下"未接入"、录播姬不可用时整块置灰且**不给配置指引**、
  B站链接分支（粘贴 → 解析预览 → 勾分P → 下载 → 自动建项目）—— 原文见 `webui/src/AGENTS.md`「首页」；
  页面自身只多一条：B站/录播姬进度走 SSE，`onUnmounted` 必须退订 `onJobDone`。

> 改动只允许落在本目录；跨模块改动先读对方卡，并跑两条门禁。

## 验证

```bash
npm --prefix webui run build
```

前端没有单测运行器：**这条构建就是本卡的门禁**（`vite build` 会解析模板与全部 import）。
改完 `webui/src` 必须重建 `webui/dist` 并一起提交。
