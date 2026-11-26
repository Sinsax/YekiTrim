// 布局不变式（2026-10）。它不查"算法漂移"，而是把**本轮真踩过的坑**钉成静态判据。
//
// 每一条都对应一次"构建通过、两条门禁全绿、而界面已经坏了"：
//
//  ①（L3 改写，2026-11-26 加「测试」列）**「按链路覆盖」是一张 8 列表格**
//     （链路/接口/模型/温度/输出上限/推理/测试/状态），
//     且**窄屏横滚**（`min-width:640px`）而不是把格子压扁。
//     历史：L3 之前它是 `.llm-cols` 两栏网格 —— 用户口径"各占 50%"，而两栏里并排的滑块会被压到
//     **59px**（1280px 下只剩 41px，刻度根本看不见）。现在一列一义，滑条不再进表格格子，所以
//     "两栏等宽 / 滑块跨满整行"这两条判据随结构一起退场，换成"表格列定义 + 横滚兜底"。
//  ②（L3 改写）**未覆盖的格子先显示灰字、点开才可编辑**（"点一下才变成可编辑"是用户口径原文）。
//     静态判据：LlmLinkTable 里必须真有 `isEditing(...)` 分支 + `@click="open(...)"` 入口。
//     没有它 = 一开始就是满屏输入框，"一眼看出哪条链路被改过"就不成立了。
//  ③ `.ov-cols` 必须有**单栏态**（"只看一边"）：点那个按钮要真的只剩一栏占满整行。
//  ④ `extra_params` 的 textarea `rows ≥ 10` —— 用户口径"高度翻2倍，不然根本看不清有什么内容"。
//  ⑤ 模板里用到的**关键标识符必须在 `<script setup>` 里定义过**。Vue 对未定义变量**不报错**
//     （渲染成 undefined）：`doctorProblem` 就是这么漏掉的 —— `:style="{ order: doctorProblem ? -1 : 99 }"`
//     恒取 `99`，"运行状况"永远沉底，界面上看不出任何异常。
//
//  ⑥ 区域选框的坐标系：`layout` 里的区域百分比是 **`sr`（主画面矩形）** 的百分比，而拖拽是在
//     **舞台**里进行的 —— 编辑端必须"存盘走 `stageToSrRect`、显示走 `srToStageRect`"，
//     且**点选落点不许跟着换算**（`[sticker] position` 是另一个键、另一套基准）。
//     漏掉任一半，只要 `sr ≠ 整块画布` 存下来的区域就不是用户画的那一块，而且**不报错**。
//
//  ⑦ 主画面位置的**两轴**滑杆（2026-10-05 xy 改版）：合成一条 `position_pct` 就会丢掉一整个轴；
//     而**贴满的那条轴没有余量**（`free_* == 0`）—— 不置灰 + 不写原因，用户拖了没反应只能猜。
//     判据还必须钉住「放大后位置组要出现」：整组只按 `!sameRatio` 藏起来的话，`scale>1` 的裁切取景
//     在同比工程里**根本没有入口**。
//
//  ⑧ 「哪一类渲不渲染」= 时间轴每条轨道前那枚**眼睛**（`[stage.lanes]`，2026-10-06）：
//     眼睛的亮灭**只读 Rust 注入的 `_lanes`**（默认值只住 `ProjectFile::lane_default` 一处），
//     前端自己再推一遍默认必然两边不一致、而且不报错；开关落盘必须走 `[stage.lanes]`
//     （写成 `[stage] camera = false` 这种平铺键会静默存不进去 —— PUT 是逐字段白名单）。
//
//  ⑯ 每条**内容轨**一枚眼睛，结构轨（分段/屏蔽）**没有** —— 给它补一枚"看着有作用、
//     其实什么也不管"的按钮，比没有更糟。
//
//  ⑰ 轨道名可点 = **这一条轨道的设置**，与时间轴块的属性面板**互斥**共用一列；
//     `#tl-props-host` 是 Teleport 目标，必须是**空容器**（往里面塞页面自己的子节点会被 patch 顶掉）。
//     右栏那两块（生成与导出 / 画面构成）合并成一块后不许再有页签（`genTab`）。
//
//  ⑱ 弹幕**没有独立泳道**（用户早前明确不加泳道）⇒ 它的显示开关与字体住「字幕 / 弹幕」轨的面板里；
//     而弹幕字体同时决定**丈量宽度**（滚动时长按宽度算）—— 量宽与渲染必须同一个族，
//     三处（预览 DOM / 模板 canvas / 转译器 ASS）都要读它。⚠️ `.dm` / `.cap` 必须写
//     `font-family:inherit`：`app.css` 的 `*{font-family:var(--font)}` 是**直接命中**，会压过
//     父层的内联字体栈（继承值永远输给本元素的任何声明）—— 不写就是"字体设置一点用没有"，
//     静默、无报错。
//     ⚠️ **空值的口径（2026-10 第二轮变更）**：`[style.danmaku] font` 为空 = **出厂默认文字字体**
//     （`Noto Sans SC`，= Rust `default_font()`），**不是**"跟随字幕那条字体栈"。旧口径有三个真问题：
//     ① 字幕那栏存的是**族名**、弹幕这栏存的是**库 id**，格式本就不同（拿族名当 id 用）；
//     ② 于是弹幕看起来和字幕一模一样，而用户预期是"默认那支"；
//     ③ **真出片通道（底座转译器）从不实现这一档**（填 `'Sans'`）⇒ 预览是字幕那支、
//        成片是 `Sans`，**不一致且不报错**。所以判据钉"三处都退到同一个常量"。
//
//  ⑨ 「打开输出目录」只在本机渲染（服务端模式那里也没有这个端点）。
//
//  ⑩ 面板类快捷键**不许在输入框里抢键**（2026-10-05 用户报：封面设计器里在标题文本框按 Delete，
//     一个键把**整个文本图层**删了）。判据是**行为**级：把 `utils.isEditableKeyEvent` 抽出来喂
//     假事件，并钉住四处调用点都真的过它 —— 少一处就是一类静默事故。
//
//  ⑪ 动图贴纸的「已加载」必须绑**引擎代**（2026-10-05 用户报：贴纸在 Dhampir 预览里不显示，
//     强刷新才回来）。底座 `dhampir_project_open` 里有 `clear_animations()`，而宿主那边
//     `animLoaded` 按 uri 继承、加载函数又短路"同素材不重传" ⇒ 每次热重载（每次编辑都会有一次）
//     引擎手里的动图被清掉而宿主不再交字节。判据：两项短路都带 `animGen === dhampirAnimGen`，
//     且**每个** `open()` 与 `attach()` 之后都 +1。
//
//  ⑫ 剪切按钮的文案必须按**这一条片段自己**的任务（2026-10-05 用户报：切一条，「所有都是排队」）。
//     服务端 `cut_one` 必持 `p{pid}:out`/`p{pid}:idx` ⇒ 拿项目级键当判据就会让满墙卡片一起变样。
//     判据钉三件事：走 `cutLabel`、按 `clipKey`+`cut_one` 过滤、在跑优先于排队。
//     （「一键全切」刻意**不带** clip 键 ⇒ 不会点亮整墙，这条口径与用户一致。）
//
//  ⑬ `/api/jobs` 的轮询要**空闲降频**（2026-10-05 用户要求），但**空闲间隔必须 < `SNAPSHOT_TTL_MS`**：
//     `busyOn` 的按钮门控靠快照新鲜度，间隔逼近 TTL 会让 `stale` 在边界抖动（门控时灵时不灵）。
//     钉两件事：两个档位常量都在、间隔按「有在跑的任务且页面可见」选、定时器链在 stop 里清干净。
//
//  ⑮ 屏蔽区间的跳转：`source_in` 是**素材帧号**，而工程网格可能被抬到 30fps ——宿主混用就是“经过屏蔽时声音和视频全乱”。
//
// 为什么是静态脚本而不是浏览器测试：本仓 `webui/scripts/` 的 7 个契约脚本都是**零依赖、不 import
// 应用模块、不需要起服务**的源码文本判据（见 webui/AGENTS.md 的口径）。要跑真几何断言得同时起
// 服务 + Chrome，那会把本地门禁变成"要环境才能跑"。所以这里只钉**源码级**的不变式；
// 真正的渲染几何（滑条到底多少 px）仍要靠 CDP 探针，本脚本的注释里留了实测数字。
//
// 运行：node webui/scripts/check-layout-invariants.mjs
// 退出码非 0 = 有不变式被破坏。**每条都带"变异自证"**：改坏源码后判据必须翻面（否则它只是空转）。

import { readFile, readdir, stat } from 'node:fs/promises'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'

const root = join(dirname(fileURLToPath(import.meta.url)), '..', '..')

let failed = 0
function check(label, actual, expected) {
  if (JSON.stringify(actual) === JSON.stringify(expected)) {
    console.log(`  ok   ${label}`)
  } else {
    failed++
    console.log(`  FAIL ${label}`)
    console.log(`         实得: ${JSON.stringify(actual)}`)
    console.log(`         期望: ${JSON.stringify(expected)}`)
  }
}

// 2026-11-25 整改后 server 侧那两个巨型文件都拆成了目录（api/project_data/*.rs 等）：
// 目录就按文件名顺序拼接全部 .rs —— 判据是「在这些源码里能 grep 到」。
const read = async (p) => {
  const abs = join(root, p)
  if (!(await stat(abs)).isDirectory()) return readFile(abs, 'utf8')
  const names = (await readdir(abs)).filter((n) => n.endsWith('.rs')).sort()
  const parts = await Promise.all(names.map((n) => readFile(join(abs, n), 'utf8')))
  return parts.join('\n')
}
const settings = await read('webui/src/views/SettingsView.vue')
// L3 起的五块 IA：①② 两条判据钉在链路表格组件上（它才是"点开才编辑"的实现处）
const llmLinkTable = await read('webui/src/components/llm/LlmLinkTable.vue')
const llmModelList = await read('webui/src/components/llm/LlmModelList.vue')
const overview = await read('webui/src/components/HomeClipsOverview.vue')
const workbench = await read('webui/src/views/WorkbenchView.vue')
const modelList = await read('webui/src/components/ModelList.vue')
const polishView = await read('webui/src/views/PolishView.vue')
const utilsJs = await read('webui/src/utils.js')
const coverDesigner = await read('webui/src/components/CoverDesigner.vue')
const timelineEditor = await read('webui/src/components/TimelineEditor.vue')
const prepareView = await read('webui/src/views/PrepareView.vue')
const clipsSection = await read('webui/src/components/ProjectClipsSection.vue')
const previewVue = await read('webui/src/components/PolishPreview.vue')
const jobsStore = await read('webui/src/composables/useJobs.js')
const jobsApiJs = await read('webui/src/api/jobs.js')
const engineJs = await read('webui/src/polish/engine.js')
const templateHtml = await read('crates/yekitrim-polish/templates/index.html')
// ⚠️ 出片**真路径**（底座 Dhampir 工程由它转译出来）—— ⑱ 要钉它与预览/模板同口径。
const translator = await read('tools/polish-to-dhampir.mjs')

/** 每条：label + 判据 + **怎么改坏它**（用于自证判据不是空转的）。 */
const RULES = [
  {
    // L3：链路覆盖从"两栏网格"改成表格（一列一义）。判据 = 表头各列都在 + 窄屏横滚兜底。
    // ⚠️ 2026-11-26 加「测试」列：每行要能按**自己的**接口+模型验一次（只测默认模型不够）。
    // ⚠️ 2026-11-27 接口 + 模型合并成「用哪支」一列 ⇒ 8 列变 7 列；**接口列必须消失**
    //    （负向判据：它一回来就说明又分成两个下拉选了）。
    label: '「按链路覆盖」是 7 列表格且窄屏横滚（接口与模型已合并成一列）',
    why: '两栏网格里并排的滑块会被压到 59px（1280px 下 41px）——改表格后必须给 min-width，让它横滚而不是压扁',
    hit: (s) =>
      /class="lt-tbl"/.test(s)
      && /min-width:\s*600px/.test(s)
      && /<th class="lt-c-link">链路<\/th>/.test(s)
      && /<th class="lt-c-model">用哪支<\/th>/.test(s)
      && /<th class="lt-c-reason">推理<\/th>/.test(s)
      && /<th class="lt-c-test">测试<\/th>/.test(s)
      && /<th class="lt-c-state">状态<\/th>/.test(s)
      && !/lt-c-prov/.test(s),
    break: (s) => s.replace('min-width:600px', 'min-width:0')
  },
  {
    // 2026-11-27：三个独立字段（provider / model / 链路）会被配成非法组合，
    // 而 problems() 不查"这支模型属不属于这个接口" ⇒ 把配对下沉到选项值里，界面上选不出错配。
    label: '「用哪支」一个下拉：选项值 = `接口|模型`（配对由界面保证）',
    why: '接口与模型分成两个下拉时，A 家的模型能选给 B 家（后端不校验，跑起来才 404）',
    hit: (s) =>
      /function pickModel\(/.test(s)
      && /<optgroup/.test(s)
      && /:value="p\.id \+ '\|' \+ m\.id"/.test(s),
    break: (s) => s.replace(`:value="p.id + '|' + m.id"`, ':value="m.id"')
  },
  {
    // L3：用户口径"点开才可编辑"—— 灰字态与编辑态必须是两条分支，且入口是 @click="open(...)"。
    label: '链路表格的格子先灰字、点开才编辑',
    why: '一开始就是满屏输入框 ⇒ "一眼看出哪条链路被改过"不成立（用户口径原文是"点开才可编辑"）',
    hit: (s) =>
      /function open\(/.test(s)
      && /const isEditing = /.test(s)
      && /@click="open\(/.test(s)
      && /v-if="!isEditing\(/.test(s),
    break: (s) => s.replace(/isEditing\(/g, 'editingX(')
  },
  {
    // 2026-10-06 用户口径变更：「未导出和已导出改成类似切片的那种筛选 tag，**所有切片都显示在一个区域内**」——
    // 原先那条「`.ov-cols` 必须有单栏态」测的是**两栏并排 + 只看一边**，那套已经被筛 tag 取代。
    // 现在要钉的是相反的事：**卡片墙只能有一面**，两栏结构不许回来（回来就又是两个滚动区）。
    label: '切片总览只有一面卡片墙（筛选 tag 取代了未导出/已导出两栏）',
    why: '用户要求"所有切片都显示在一个区域内"——两栏各自滚动会让"找一条片段"要在两个区里分别翻',
    hit: (s) => {
      const walls = (s.match(/class="ov-wall"/g) || []).length
      return walls === 1 && !/class="ov-cols"/.test(s) && /OV_FILTERS/.test(s)
    },
    // 变异：把两栏结构加回来（`ov-cols` 一出现就该判翻面）
    break: (s) => s.replace('<div v-if="shownClips.length"', '<div class="ov-cols"><div v-if="shownClips.length"')
  },
  {
    // 2026-10-06：两栏结构（`.ov-cols` / `.ov-sec-done` 的竖虚线）已随筛选 tag 一起删除 ——
    // 那条"单栏时撤掉竖虚线"的判据随之作废（它测的节点不再存在）。
    // 取而代之的判据：**搜索框必须真的存在且能锁到三处**（标题/字幕/弹幕）——
    // 用户口径里的"关键词可以锁到标题，字幕，弹幕"，缺一个就是没做完。
    label: '切片总览的搜索能锁标题/字幕/弹幕三处',
    why: '用户点名要"关键词可以锁到标题，字幕，弹幕"——少一处等于功能没做全',
    hit: (s) =>
      /v-model="query"/.test(s)
      && /\{ key: 'title', label: '标题'/.test(s)
      && /\{ key: 'subtitle', label: '字幕'/.test(s)
      && /\{ key: 'danmaku', label: '弹幕'/.test(s),
    break: (s) => s.replace("{ key: 'danmaku', label: '弹幕'", "{ key: 'x', label: '弹幕'")
  },
  {
    // 2026-11-26：全局 extra_params 取消 ⇒ 编辑框搬进「每个模型」的展开区（LlmModelList）。
    // 用户原口径仍是"太矮就根本看不清有什么内容"，这里钉模型级那个 JSON 框的高度。
    label: '模型级 extra_params 的编辑框 rows ≥ 6（能看清内容）',
    why: '逃逸口只按模型存在（用户口径）；JSON 正文太矮等于看不清，改回 3 行就该翻面',
    hit: (s) => {
      const m = s.match(/class="mono ml-extra"[^>]*rows="(\d+)"/s)
      return m ? Number(m[1]) >= 6 : false
    },
    break: (s) => s.replace('class="mono ml-extra" rows="6"', 'class="mono ml-extra" rows="3"')
  },
  {
    label: 'ModelList 的下载按钮按 canDownload 门控（下载是 admin-only）',
    why: '这个组件挂在**人人可见的首页**上，而 POST /api/models/download 服务端回 403（仅管理员）',
    hit: (s) => /v-if="!m\.installed && canDownload"/.test(s) && /canDownload\s*=\s*computed/.test(s),
    break: (s) => s.replace('v-if="!m.installed && canDownload"', 'v-if="!m.installed"')
  }
]

/** 关键标识符：模板里用到 + `<script setup>` 里必须定义过。 */
const CRITICAL_BINDINGS = [
  // 2026-11-15：doctorProblem/doctorIssues 随「运行状况」卡并入 SetupChecklist 删除；
  // 这张卡换成钉新的模板关键标识符（setupReady 管两块卡显隐、doctorError 要传进检查项照实报）。
  ['webui/src/views/WorkbenchView.vue', workbench, ['loadDoctor', 'doctorError', 'setupReady']],
  ['webui/src/components/HomeClipsOverview.vue', overview, ['ovFilter', 'OV_FILTERS', 'searchIn', 'query', 'shownClips']],
  ['webui/src/views/SettingsView.vue', settings, ['localNoAuth', 'visibleSections']],
  ['webui/src/components/ModelList.vue', modelList, ['canDownload']],
  // 识别页「处理流程」卡（2026-11-15 布局重构）：模板直接渲染这些标识符，
  // 漏一个 import 就是静默空白 —— statusClass 真的漏过（chip 渲染成无样式 span）。
  ['webui/src/views/PrepareView.vue', prepareView, ['statusClass', 'stageRows', 'stageSel', 'jobLine']],
  // 文字动效那组控件（2026-10-13）。⚠️ **Vue 对模板里未定义的标识符不报错**，
  // 而是渲染成空白 —— 症状是"面板里少了一行，没有任何报错"。这里逐个钉住。
  ['webui/src/views/PolishView.vue', polishView, ['swayDeg', 'swayMs', 'patchElements', 'rangeOrNull', 'numOrNull', 'pacing', 'pacingText', 'pacingTitle']]
]

console.log('\n① 布局不变式（每条都做变异自证）')
for (const r of RULES) {
  const sources = {
    label: r.label,
    hit: r.hit(settings) || r.hit(overview) || r.hit(modelList) || r.hit(llmLinkTable) || r.hit(llmModelList)
  }
  check(r.label, sources.hit, true)

  // 变异自证：改坏**命中的那份**源码，判据必须翻面
  const all = [settings, overview, workbench, modelList, llmLinkTable, llmModelList]
  const idx = all.findIndex((s) => r.hit(s))
  const broken = r.break(all[idx])
  const stillHits = broken !== all[idx] && (r.hit(broken) || (idx !== 0 && r.hit(settings)))
  check(`[变异] ${r.label} → 判据翻面`, stillHits, false)
  if (broken === all[idx]) {
    failed++
    console.log(`  FAIL [变异] ${r.label}：变异没生效（break() 没改到东西）—— 这条自证是空转的`)
  }
}

console.log('\n② 模板标识符必须在 <script setup> 里定义过（Vue 对未定义变量不报错）')
for (const [file, src, names] of CRITICAL_BINDINGS) {
  const script = src.slice(src.indexOf('<script'))
  const tpl = src.slice(0, src.indexOf('<script'))
  for (const n of names) {
    // ⚠️ **这里判的是"必须定义"，不是"必须被模板用到"**（2026-10-13 改）。
    //
    // 旧写法是 `usedInTpl && defined` —— 它把**两件事捆在一起**：
    //   ① 用了未定义的 ⇒ 真漏洞（白屏、无报错）；
    //   ② 定义了但没用 ⇒ 它也报红。
    // 而用户 2026-10-13 明确要求**保留前端接线**（只删界面），
    // 所以「定义了没用」是**预期之中**，不是缺陷。
    // ⇒ 拆开：**只钉①**（定义必须存在），并且**只在模板真用到时**才把"用了就得有定义"当成硬条件 ——
    //    那一条原本就是防白屏的，保留；只是不再把"没用到"当成失败。
    const usedInTpl = tpl.includes(n)
    const defined =
      new RegExp(`(const|let|function)\\s+${n}\\b`).test(script) ||
      new RegExp(`\\b${n}\\s*[:=]`).test(script) ||
      // ⚠️ **import 也算定义**（2026-11-15 加）：模板用的东西大多来自 import，
      // 只认 const/function 会让判据把"漏了 import"这种**真事故**判成"没定义"而误报，
      // 也会反过来放过"import 写了但名字拼错"的那一类。实测事故：statusClass
      // 在模板里渲染任务状态 chip，import 被删掉后 chip 静默变成无样式 span。
      new RegExp(`import\\s*\\{[^}]*\\b${n}\\b[^}]*\\}`).test(script)
    // ① 定义必须存在（无论用没用到）；
    check(`${file.split('/').pop()} · ${n}（必须定义过）`, defined, true)
    // ② 模板真用到了 ⇒ 就更不能定义丢了（这条与上一条同真值，但它在报错文案里点名症状）。
    check(`${file.split('/').pop()} · ${n}（模板用到就必须定义）`, !usedInTpl || defined, true)
  }
  // 变异自证：把定义改名，判据必须翻面
  const n = names[0]
  const mutated = script.replace(new RegExp(`(const|function)\\s+${n}\\b`), '$1 ' + n + 'X')
  const stillDefined = new RegExp(`(const|let|function)\\s+${n}\\b`).test(mutated)
  check(`[变异] ${file.split('/').pop()} · ${n} 改掉定义 → 判据翻面`, stillDefined, false)
}

// ── ⑥ 区域选框的坐标系：`layout` 是 **`sr`（主画面矩形）百分比**，编辑端必须做两次换算 ──
// 判据缺口 #7（`docs/plan/polish-preview-bugs-2026-10-05.md` §④）：拖拽在**舞台**里进行，
// 而 `layout.<区域>.top_left/bottom_right` 的基准是 **`sr`**（源帧经画面构成摆到画布上的那一块）
// —— 两个渲染端（模板 canvas / engine）+ 转译器 + Rust `process::placement` 都按 `sr` 读。
// 编辑端过去把舞台百分比**直接**写盘：只要 `sr ≠ 整块画布`（源比例≠画布 / `scale≠1` /
// `padding≠0`），存下来的区域与画面所指的位置就不是同一块，**而且不报错**（用户报的
// "区域与落点对不上"）。这里钉三件事：①存盘走 `stageToSrRect`；②显示走 `srToStageRect`；
// ③**点选落点仍用舞台坐标** —— `[sticker] position` 是另一个键、另一套基准（盒子中心），
// 所以 `pctAt` 里**不许**出现这两个换算（把两个换算合并就会把落点一起改错）。
{
  const preview = await read('webui/src/components/PolishPreview.vue')
  const fnBody = (name) => {
    const i = preview.indexOf(`function ${name}(`)
    if (i < 0) return ''
    let j = preview.indexOf('{', i)
    let depth = 0
    for (; j < preview.length; j++) {
      if (preview[j] === '{') depth++
      else if (preview[j] === '}') { depth--; if (!depth) return preview.slice(i, j + 1) }
    }
    return preview.slice(i)
  }
  const emitBody = fnBody('emitRect')
  check('⑥ 能定位 emitRect', emitBody.length > 0, true)
  check('⑥ 存盘走 stage→sr 换算', /stageToSrRect\(/.test(emitBody), true)
  check('⑥ 存盘不再直接吐舞台百分比', /r2\(r\.x0\)/.test(emitBody), false)
  check('⑥ 显示走 sr→stage 换算', /selNorm\(srToStageRect\(props\.selectRect\)\)/.test(preview), true)
  const pctBody = fnBody('pctAt')
  check('⑥ 点选落点仍用舞台坐标（pctAt 不参与换算）', /stageToSrRect|srToStageRect/.test(pctBody), false)
  // 变异自证：摘掉 emitRect 里那次换算 → 上面两条必须翻面
  const mutated = emitBody.replace('const sr = stageToSrRect(r)', 'const sr = r')
  check('[变异] ⑥ 变异确实摘掉了换算', mutated !== emitBody, true)
  check('[变异] ⑥ 摘掉换算后"存盘走换算"判据翻面', /stageToSrRect\(/.test(mutated), false)
}

// ── ⑦ 主画面位置 = **两轴各一条五档**（2026-10-05）：无效轴置灰 + 行内写明原因 ──
// 用户口径：「横素材 竖画布的位置操作改为 xy 操作、带吸附的五段」。两件事必须一起在：
//   ① 两轴各自一条滑杆（提交两轴键，不是合成一条 `position_pct`）；
//   ② 贴满的那条轴（`free_* == 0`）**置灰 + 写明原因** —— 否则又是一次「拖了没反应」。
// 置灰判据只许**读契约的 `free_x` / `free_y`**（Rust `resolve_stage` 算的余量）：界面自己按
// `axis`/`scale` 再推一遍就是第二份公式，改一边必漂。
{
  const twoAxes = (s) => s.includes('v-for="ax in positionAxes"')
    && s.includes('const positionAxes = computed')
    && s.includes('position_x_pct') && s.includes('position_y_pct')
  const submitsBoth = (s) => s.includes('patchStage({ [ax.key]: Number($event.target.value) })')
    && !s.includes('patchStage({ position_pct')
  const readsFree = (s) => s.includes('st.free_x') && s.includes('st.free_y')
  const disablesInert = (s) => s.includes(':disabled="stageBusy || !ready || ax.inert"')
  const hintInline = (s) => s.includes('{{ ax.hint }}') && s.includes('已贴满，放大后才能平移')
  const afterZoom = (s) => s.includes('!sameRatio.value || stageScaleNum.value !== 1')
  check('⑦ 两轴各一条滑杆（positionAxes 渲染两轴键）', twoAxes(polishView), true)
  check('⑦ 提交的是两轴键（不再写 position_pct）', submitsBoth(polishView), true)
  check('⑦ 置灰判据读契约的 free_x / free_y（不自己推 axis/scale）', readsFree(polishView), true)
  check('⑦ 无效轴置灰（`:disabled` 带上 ax.inert）', disablesInert(polishView), true)
  check('⑦ 无效轴的**行内**原因在（不是只挂 title）', hintInline(polishView), true)
  check('⑦ 放大后位置组要出现（同比工程也有裁切取景入口）', afterZoom(polishView), true)
  const MUT7 = [
    ['摘掉无效轴置灰', (s) => s.replace(':disabled="stageBusy || !ready || ax.inert"', ':disabled="stageBusy || !ready"')],
    ['摘掉行内原因（只留 title）', (s) => s.replace('<div v-if="ax.inert" class="small muted mb-4">{{ ax.hint }}</div>', '')],
    ['置灰判据改成自己算（不读契约）', (s) => s.replace('st.free_x', 'false ? 0 : 0')],
    ['退回「同比就整组隐藏」（放大后也没有入口）', (s) => s.replace('!sameRatio.value || stageScaleNum.value !== 1', '!sameRatio.value')],
    ['两轴合成回单条（位置操作退回一维）', (s) => s.replace('v-for="ax in positionAxes"', 'v-if="false"')],
  ]
  for (const [what, mutate] of MUT7) {
    const m = mutate(polishView)
    check(`[变异] ⑦ ${what} → 源码确实被改了`, m !== polishView, true)
    const ok = twoAxes(m) && submitsBoth(m) && readsFree(m) && disablesInert(m) && hintInline(m) && afterZoom(m)
    check(`[变异] ⑦ ${what} → 判据翻面`, ok, false)
  }
}

// ── ⑧ 轨道眼睛：亮灭只读注入 + 开关落盘 `[stage.lanes]`（2026-10-06）──
{
  // 亮灭只读 Rust 注入的 `_lanes`（默认值只住 Rust `ProjectFile::lane_default` 一处）
  const readsInjected = (s) =>
    /function laneEnabled\(lane\) \{[\s\S]{0,120}?props\.lanesOn\[lane\][\s\S]{0,120}?typeof v === 'boolean' \? v : true/.test(s)
  const pageReadsLanes = (s) => s.includes('const lanesOn = computed(() => cfg.value?._lanes || {})')
  const writesLanes = (s) =>
    s.includes('patchStage({ lanes: { ...(stageCfg.value.lanes || {}), [lane]: !cur } })')
  // 老的三键必须清干净（还在就是"两套开关并存"，用户改了没反应）
  const noPortraitKeys = (s) =>
    !s.includes('portrait_camera') && !s.includes('portrait_effects') && !s.includes('portrait_sfx')
  check('⑧ 眼睛的亮灭只读注入的 `_lanes`（不自己推默认）', readsInjected(timelineEditor), true)
  check('⑧ 页面读 Rust 注入的 `_lanes`', pageReadsLanes(polishView), true)
  check('⑧ 开关落盘走 `[stage.lanes]`（不是平铺键）', writesLanes(polishView), true)
  check('⑧ 老的三键（portrait_*）清干净了', noPortraitKeys(polishView + timelineEditor), true)
  const eyeOk = (m) => readsInjected(m) && pageReadsLanes(polishView) && writesLanes(polishView) && noPortraitKeys(polishView + m)
  const pageOk = (m) => readsInjected(timelineEditor) && pageReadsLanes(m) && writesLanes(m) && noPortraitKeys(m + timelineEditor)
  const writeOk = (m) => readsInjected(timelineEditor) && pageReadsLanes(polishView) && writesLanes(m) && noPortraitKeys(polishView + timelineEditor)
  const MUT8 = [
    ['眼睛恒亮（不看注入）', timelineEditor, (s) => s.replace('props.lanesOn ? props.lanesOn[lane] : undefined', 'undefined'), eyeOk],
    ['页面自己按 `stage.lanes` 推默认（不读注入）', polishView, (s) => s.replace('cfg.value?._lanes || {}', 'cfg.value?.stage?.lanes || {}'), pageOk],
    ['开关写成平铺键（PUT 白名单里没有 ⇒ 静默存不进去）', polishView, (s) => s.replace('{ lanes: { ...(stageCfg.value.lanes || {}), [lane]: !cur } }', '{ [lane]: !cur }'), writeOk],
  ]
  for (const [what, src, mutate, still] of MUT8) {
    const m = mutate(src)
    check(`[变异] ⑧ ${what} → 源码确实被改了`, m !== src, true)
    check(`[变异] ⑧ ${what} → 判据翻面`, still(m), false)
  }
}

// ── ⑯ 每条**内容轨**一枚眼睛，结构轨（分段 / 屏蔽）没有（2026-10-06）──
{
  const LANE_KEYS = ['subtitles', 'camera', 'sticker', 'effects', 'sfx']
  const laneDefs = (s) => {
    const at = s.indexOf('const LANE_DEFS = [')
    if (at < 0) return ''
    // ⚠️ 块的注释里也有 `]`（`[stage.lanes]`）—— 不能用"第一个 `]`"当结尾，
    // 否则截出来的块永远少一半，判据会**假红**（我第一版就是这么挂的）。
    const end = s.indexOf('const lanes = LANE_DEFS', at)
    return end < 0 ? s.slice(at) : s.slice(at, end)
  }
  const everyLaneHasEye = (s) => {
    const b = laneDefs(s)
    return !!b && LANE_KEYS.every((k) => b.includes("lane: '" + k + "'"))
  }
  const structLaneHasNoEye = (s) => {
    const b = laneDefs(s)
    return !!b && !/type: 'segments'[^\n]*lane:/.test(b)
  }
  check('⑯ 五条内容轨都挂了开关名（眼睛）', everyLaneHasEye(timelineEditor), true)
  check('⑯ 结构轨（分段 / 屏蔽）没有眼睛', structLaneHasNoEye(timelineEditor), true)
  const MUT16 = [
    ['给结构轨也补一枚眼睛（假装有作用）', (s) => s.replace("type: 'segments', label: '分段 / 屏蔽'", "type: 'segments', label: '分段 / 屏蔽', lane: 'segments'")],
    ['摘掉运镜轨的开关名（这条轨道没有眼睛了）', (s) => s.replace("label: '运镜', lane: 'camera',", "label: '运镜',")],
  ]
  for (const [what, mutate] of MUT16) {
    const m = mutate(timelineEditor)
    check(`[变异] ⑯ ${what} → 源码确实被改了`, m !== timelineEditor, true)
    check(`[变异] ⑯ ${what} → 判据翻面`, everyLaneHasEye(m) && structLaneHasNoEye(m), false)
  }
}

// ── ⑰ 轨道名可点 = 轨道设置；与块的属性面板互斥；右栏合并成一块（2026-10-06）──
{
  // ⚠️ 2026-10-14：元素轨拆成「文字 / 贴纸」两条之后，发出去的**不再是裸 `lane.type`**，
  //    而是 `laneTypeOf(lane)`（它**也**返回 `lane.type` —— 两条都发 `'sticker'`）。
  //    ⚠️ 为什么不能改成用界面 id（`laneKey`）：`PolishView` 的 `laneGroupEls`
  //       按**轨道名**索引，用 `laneKey` 会让点「文字」轨去查不存在的
  //       `laneGroupEls.text` ⇒ **点轨道名没反应**（静默）。
  //    本判据钉的仍是"点轨道名会发 lane-open"，放宽到接受这两种写法。
  const nameOpens = (s) =>
    /@click\.stop="emit\('lane-open', lane(TypeOf\(lane\)|\.type)\)"/.test(s)
  const itemYields = (s) => s.includes('<div v-if="!laneOpen" class="props-panel card"')
  const lanePanel = (s) => s.includes('<div class="right-col">') && /lane-group-h/.test(s)
  const hostEmpty = (s) => s.includes('<div id="tl-props-host"></div>')
  // ⚠️ 2026-10-06 第四轮：轨道设置搬到右栏常驻后，**面板不再排在 Teleport 目标之前**（反了）。
  //    这里改成钉正确的两件事：左栏那个挂载点**是空容器**、且**只有一个**（Teleport 目标唯一）。
  const hostCount = (s) => (s.match(/<div id="tl-props-host"><\/div>/g) || []).length
  const panelOk = (s) => lanePanel(s) && hostEmpty(s) && hostCount(s) === 1
  const blockSelectClosesLane = (s) =>
    /function selectBlock\(lane, block\) \{[\s\S]{0,300}?emit\('lane-open', ''\)/.test(s)
  // ⚠️ 右栏结构（2026-10-06 第四轮用户口径，**改过两轮才对齐**）：
  //   ①「轨道设置」面板**整块搬到右栏常驻**（以前住在左侧 `props-host`、点轨道名才出现）；
  //   ② 一块带页签的卡片：「项目设置」/「画面尺寸」**同一位置**切换（= 用户说的
  //      "画面尺寸退回原来的那个页面"）—— 页签另一半装着画面尺寸/缩放/主画面位置/背景；
  //   ③ 输出（LLM 剪辑 / 渲染导出）在「项目设置」页签里，**不许删**；
  //      任务栏 2026-10-18 搬到时间轴编辑器右上角（`#status` 槽），由 [jobBarInTimelineHead] 钉
  //      （用户原话：「你把项目设置全删了我是没想到的」）。
  //   ⚠️ 全在**同一个 grid 子节点** `.right-col` 里 —— `.edit-grid` 是两列（`1fr 300px`），
  //   摊成多个子节点时后面的会充当"第三列"**掉到下一行占满整宽**（本轮真踩到，实测 1045px）。
  const rightColWrapped = (s) =>
    /<div class="right-col">/.test(s)
    && /\.right-col\s*\{[^}]*display:\s*flex/.test(s)
    && /\.right-col\s*\{[^}]*flex-direction:\s*column/.test(s)
  /** 页签那一对按钮（`sideTab`）—— 两个都在，且 `sideTab` 有 ref。 */
  const hasSideTabs = (s) =>
    s.includes("sideTab === 'project'") && s.includes("sideTab === 'canvas'")
    && /const sideTab = ref\(/.test(s)
  /** 画面尺寸得**住在页签的另一半**里（不是自己独立一块卡片）。 */
  const canvasInsideTab = (s) => {
    // 锚在**页签那个** v-if 之后的 v-else（文件里别处也有 `<template v-else>`）
    const i = s.indexOf("<template v-if=\"sideTab === 'project'\">")
    if (i < 0) return false
    const j = s.indexOf('<template v-else>', i)
    if (j < 0) return false
    const k = s.indexOf('</template>', j)
    const body = s.slice(j, k)
    return body.includes('orientationOptions') && body.includes('stageScaleNum') && body.includes('positionAxes')
  }
  /** 项目设置那一半必须还在（**LLM 剪辑 + 渲染导出**）。
   *
   *  ⚠️ **2026-10-18：任务栏搬走了**（用户要求移到时间轴编辑器右上角）⇒ 这里**不再**要求
   *  `job-head` 在页签里，改由 [`jobBarInTimelineHead`] 钉它的**新位置**。
   *  这一条的价值仍在：右栏那半块是「用户说'没想到'的那件事」，不许再删。 */
  const projectInsideTab = (s) => {
    const i = s.indexOf("<template v-if=\"sideTab === 'project'\">")
    if (i < 0) return false
    // 取到页签的 v-else 为止（不是第一个 </template>：项目那半里还有内层 template）
    const j = s.indexOf('<template v-else>', i)
    const body = j < 0 ? s.slice(i) : s.slice(i, j)
    return body.includes('runLlmJob') && body.includes('openRenderModal')
  }
  /** **任务栏在时间轴编辑器右上角**（2026-10-18 用户要求从右栏搬来）。
   *
   *  三件同时成立才算：① 编辑器有 `status` 槽；② 页面把任务栏那个 `<template #status>` 传进去，
   *  且 `job-head` **落在里面**；③ 槽位在 `.tl-head` 里（不是卡片外另起一行）。 */
  const jobBarInTimelineHead = (view, editor) => {
    if (!editor.includes('<slot name="status" />')) return false
    if (!editor.includes('class="tl-head-status"')) return false
    const i = view.indexOf('<template #status>')
    if (i < 0) return false
    const j = view.indexOf('</template>', i)
    const body = view.slice(i, j)
    if (!body.includes('job-head')) return false
    // 右栏里**不许**再留一份（两份就会出现「同一个任务两处显示」）
    const rc = view.indexOf('<div class="right-col">')
    const tl = view.indexOf('        <!-- 时间轴编辑器')
    const rightCol = rc >= 0 && tl > rc ? view.slice(rc, tl) : ''
    return !rightCol.includes('job-head')
  }
  /** 轨道设置面板搬到右栏常驻：在 `.right-col` 里、且不再按 `openLane` 条件渲染。 */
  /** 右栏那块区域里的卡片数（合并后应为 1）。 */
  const rightCardCount = (s) => {
    const rc = s.indexOf('<div class="right-col">')
    const end = s.indexOf('        <!-- 时间轴编辑器')
    if (rc < 0 || end < 0) return -1
    return (s.slice(rc, end).match(/<div class="card">/g) || []).length
  }
  const lanePanelResident = (s) => {
    const rc = s.indexOf('<div class="right-col">')
    const lc = s.indexOf('lane-group-h', rc)
    const hp = s.indexOf('<div id="tl-props-host">')
    if (!(rc >= 0 && lc > rc && hp >= 0 && hp < rc)) return false
    // 合并后轨道设置**不是一个独立面板**了：它住在「项目设置」页签里、且右栏只有**一块**卡片。
    const cards = (s.slice(rc, s.indexOf('        <!-- 时间轴编辑器')).match(/<div class="card">/g) || []).length
    return cards === 1 && s.indexOf('lane-group-h', rc) < s.indexOf('<template v-else>', rc)
  }
  // ⚠️ **一个面板装全部轨道**（用户 2026-10-06 口径："点哪个轨道都是同一个面板，这样好操作"）：
  // 各段必须**同时**在面板里，`openLane` 只是"焦点"（高亮哪一段 + 往哪儿滚）。
  // 判据钉三件事：① 没有 `v-if/v-else-if="openLane === '…'"` 的链（那就是"一轨一屏"回来了）；
  // ② 段标题恰好是这几个；③ 点轨道名会滚到那一段。
  // ⚠️ 2026-10-06 第二轮：「视觉特效」整块删掉（一个控件都没有）、音效组只留音量滑块 ⇒ 5 段。
  // ⚠️ 2026-10-06 第三轮（用户：「轨道设置的仅信息展示的内容删除，例如：分段 / 屏蔽」+
  //    「将框选区域的按钮显示在同一行内」+「删除音效的音量设置」）：
  //    ①「分段 / 屏蔽」整组删（只有两个计数、零控件）②「音效」整组删（只有一条滑杆）
  //    ③ 框选弹幕区域 / 框选人物 合成**一行**（原来分居「字幕/弹幕」与「运镜」两组）
  //    ⇒ 段数 5 → **3**：字幕/弹幕 · 框选区域 · 贴纸。
  //    注意「运镜」这个标题**不再作为分组标题存在**（它的唯一控件"框选人物"并进了框选组），
  //    但点时间轴「运镜」轨道名仍要能滚到那块（ref 同时挂 camera 与 danmaku_area 两个键）。
  // 第八轮（2026-10-06）：「贴纸」与「区域」合并成**一组**「区域设置」（用户：
  // 「贴纸和区域合并，显示为区域设置」）⇒ 只剩两组，顺序 字幕/弹幕 · 区域设置。
  const LANE_GROUPS = ['字幕 / 弹幕', '区域设置', '片头 / 片尾']
  const panelBody = (s) => {
    // 面板正文 = 「项目设置」页签那一半（合并后轨道设置内容就住在里面）
    const i = s.indexOf("<template v-if=\"sideTab === 'project'\">")
    const j = s.indexOf('<template v-else>', i)
    return i < 0 || j < 0 ? '' : s.slice(i, j)
  }
  // ⚠️ 判据要**恰好相等**（顺序也钉住）：只要求"这几段都在"的话，"把删掉的那段装回来"这条
  // 变异翻不了面（多一段不影响"都在"）—— 实测踩过。
  const groupHeads = (s) => [...s.matchAll(/<div class="lane-group-h">([^<]+)<\/div>/g)].map((m) => m[1])
  const allGroupsAtOnce = (s) =>
    !s.includes('v-if="openLane ===') && !s.includes('v-else-if="openLane ===')
      && JSON.stringify(groupHeads(s)) === JSON.stringify(LANE_GROUPS)
  // **只留控件**：面板正文里不许有解释用的 `hint` 行、不许有那些"没用"的读数/说明
  const onlyControls = (s) => {
    const body = panelBody(s)
    if (!body) return false
    // ⚠️ 只看**轨道设置那一段**（到 `lane-sep` 分隔线为止）：`lane-sep` 之后是 LLM 剪辑 /
    //    任务栏，那里的 `.hint`（"该片段正在…请稍候"）是正常状态提示，不是轨道解释文字。
    // ⚠️ 只看**模板**：`<script setup>` 与 `<style>` 里这些词到处都有（注释、类名），
    //    拿整份文件判会把判据变成"永远翻不了面的空转"。
    const k = body.indexOf('lane-sep')
    const lane = (k < 0 ? body : body.slice(0, k)).split('<script')[0]
    return !lane.includes('class="hint') && !lane.includes('特效事件')
      && !lane.includes('音效事件') && !lane.includes('逐条在事件的属性面板里改')
      && !lane.includes('开关只影响渲染')
  }
  // ⚠️ **框选那两颗按钮必须同一行**（用户 2026-10-06：「将框选区域的按钮显示在同一行内」）：
  // 区域设置里三颗按钮（弹幕位置 / 人物位置 / 自动）**在同一行**。
  // ⚠️ 别用 `<div class="row">…</div>` 正则：非贪婪解遇到**内层** `</div>` 就截断了，
  //    永远跨不过这组按钮（实测 rows 里一条都不含三颗）。改用"三颗之间不许出现换行容器"。
  const selectBtnsOneRow = (s) => {
    const marks = ["toggleSelect('danmaku_area')", "toggleSelect('liver')", 'toggleStickerMode']
    const at = marks.map((m) => s.indexOf(m))
    if (at.some((i) => i < 0)) return false
    const lo = Math.min(...at)
    const hi = Math.max(...at)
    // 三颗之间只允许出现 button/span 这类行内元素，不许出现 div.row / div.col（那就是换了行/列）
    return !/<div class="(row|col)[^"]*">/.test(s.slice(lo, hi))
  }
  // ⚠️ **纯信息组不许回来**：面板里不许再出现「分段 / 屏蔽」标题，也不许有只读计数行
  const noInfoOnly = (s) => {
    const heads = groupHeads(s)
    return !heads.includes('分段 / 屏蔽') && !heads.includes('音效')
  }
  const focusScrolls = (s) =>
    s.includes('laneGroupEls[lane]') && s.includes('scrollIntoView({ block: ' + "'nearest'" + ' })')
      // ⚠️ 第八轮合并后「区域设置」同时挂 sticker / camera / danmaku_area 三个轨道名，
      //    所以 on 的表达式是**三个** || 串起来的 —— 别再钉回两个。
      && /:class="\{ on: openLane === 'sticker' \|\| openLane === 'camera' \|\| openLane === 'danmaku' \}"/.test(s)
  check('⑰ 轨道名可点开设置', nameOpens(timelineEditor), true)
  check('⑰ 块的属性面板在轨道设置打开时让位', itemYields(timelineEditor), true)
  check('⑰ 页面渲染轨道设置面板，且 Teleport 目标唯一且是空容器', panelOk(polishView), true)
  check('⑰ 选中时间轴上的块 ⇒ 收起轨道设置（两块面板不并存）', blockSelectClosesLane(timelineEditor), true)
  check('⑰ 右栏包在同一个 grid 子节点里（否则后面的会掉到下一行占满整宽）', rightColWrapped(polishView), true)
  check('⑰ 右栏有「项目设置 / 画面尺寸」两个页签', hasSideTabs(polishView), true)
  check('⑰ 画面尺寸住在页签的另一半里（同一位置切换）', canvasInsideTab(polishView), true)
  check('⑰ 项目设置那一半还在（LLM 剪辑 / 渲染导出）', projectInsideTab(polishView), true)
  check('⑰ 任务栏在时间轴编辑器右上角（2026-10-18 搬家，右栏不许再留一份）',
    jobBarInTimelineHead(polishView, timelineEditor), true)
  check('⑰ 轨道设置面板搬到右栏常驻', lanePanelResident(polishView), true)
  // ⚠️ **右栏只能有一块卡片**（用户 2026-10-06：「你分这么多区域，都把时间轴都挤走了」）：
  //    上一版把右栏拆成 2~3 块独立卡片，堆到 237 行，时间轴被挤出首屏。
  //    这条只钉"块数"，真实高度由 CDP 探针量（`.workbuddy/probe-layout-height.mjs`）。
  check('⑰ 右栏只有一块卡片（块数多了会把时间轴挤走）', rightCardCount(polishView), 1)
  check('⑰ 那块卡片里没有「项目设置页 →」跳转链接（用户说不需要）',
    /项目设置页\s*→[\s\S]{0,20}?<\/router-link>/.test(polishView), false)
  // ⚠️ 第六轮（用户 2026-10-06：「三个区域放在同一行」「把贴纸放在上面」「任务改成折叠的模式，
  //    默认折叠，只显示当前是什么任务，展开显示详情，展开时不要挤占下方控件，是浮在上层的」）
  // ⚠️ 第七轮纠偏：第六轮我读成"三组并排成三列"是**误读**，用户说「整体还是以行来排版」。
  //    所以这里反过来钉：**不许**再有 `.lane-cols` 那套并列，三组必须是行排版（上下堆叠）。
  check('⑰ 三组是**行排版**（没有并排的列容器）', /class="lane-cols"/.test(polishView), false)
  // ⚠️ 2026-10-09 由 2 → 3：加了「片头 / 片尾」（用户：「片头片尾在项目设置内可以选择是否开启，
  //    然后可以选择素材库内的视频内容」）。它是**有控件的真组**（开关 + 素材下拉 + 入出点），
  //    不是当年删掉的那种纯信息组 —— 所以 `noInfoOnly` 那条判据照样成立，别把它一起改松。
  check('⑰ 三组上下堆叠（字幕/弹幕 · 区域设置 · 片头/片尾）', (polishView.match(/<div class="lane-group"/g) || []).length, 3)
  // 用户给的行序：**贴纸来源行在按钮行上面**。
  // ⚠️ 别用 `indexOf('ss-options')` —— CSS 在 `<style>` 里也写了这个名字，会比模板更早命中。
  //    锚在**模板里的**那一处（`<div class="ss-options">`）上。
  check('⑰ 区域设置里：贴纸来源行在按钮行上面',
    polishView.indexOf('<div class="ss-options">') < polishView.indexOf('> 弹幕位置'), true)
  check('⑰ 区域设置里三颗按钮：弹幕位置 / 人物位置 / 自动（同一个 .row）',
    selectBtnsOneRow(polishView), true)
  check('⑰ 组名恰好是 字幕/弹幕 · 区域设置 · 片头/片尾',
    (polishView.match(/<div class="lane-group-h">([^<]+)<\/div>/g) || []).map((m) => m.replace(/<[^>]+>/g, '')).join('|'),
    '字幕 / 弹幕|区域设置|片头 / 片尾')
  // ---- 2026-10 用户追加两条：字幕样式两列 + 片头片尾独立页签 ----
  // ⚠️ 片头/片尾**搬进了自己的页签**（用户：「片头片尾放在一个单独的页面」），
  //    但它仍然是三组之一（下面那条 `lane-group` 计数与组名判据照旧成立）。
  // ⚠️ 提取值要用**捕获组**：早先写成整体 replace 会得到 `"|"|"`（正则把引号也吃了）。
  check('⑰ 三个页签：项目设置 / 画面尺寸 / 片头片尾',
    [...polishView.matchAll(/@click="sideTab = '(\w+)'"/g)].map((m) => m[1]).join('|'),
    'project|canvas|intro')
  // 片头片尾的**内容**必须在 `sideTab === 'intro'` 那个 `v-else-if` 分支里，
  // 也就是：分支标记之后、下一个分支（`v-else` 的画面尺寸）之前，能找到 io-row。
  const introAt = polishView.indexOf("v-else-if=\"sideTab === 'intro'\"")
  const canvasAt = polishView.indexOf('<template v-else>', introAt)
  check('⑰ 片头片尾块落在 intro 分支内（不在 project 分支里）',
    introAt > 0 && canvasAt > introAt && polishView.slice(introAt, canvasAt).includes('io-row'), true)
  // 字幕四项必须是**两列两行**（用户：「改为两行的排版」）：两个 `.ss-row-2` 容器、
  // 每个里正好两条 `.ss-row`。⚠️ 判据走**模板**：CSS 里也写了 `.ss-row-2`，只数全文件会数重。
  // ⚠️ 右端锚点**换过一次**（2026-10-12 跟随用户的按钮改名）：
  //    原锚是 `恢复默认`，而用户把按钮改成了「重置位置 / 重置大小」、那句文案整句消失 ⇒
  //    `indexOf` 返回 **-1** ⇒ `slice(i, -1)` 把整段**切到最后**（不是空！），
  //    于是这里数到的 `.ss-row` 是**整个文件**的、永远凑不出 4 条 —— 判据翻红，
  //    而红的原因是锚点烂了，不是排版坏了。**锚点必须指一个真的存在的字面量**。
  //    ⚠️ **锚点又烂了一次**（2026-10-13）：上一任锚点是
  //    「位置→默认 / 大小→默认」那两颗按钮」那句注释 —— 而这次把那段注释整块重写了，
  //    又是 `indexOf` → -1 → `slice(i, -1)` 把整段切到最后。
  //    ⇒ **锚点不该选“可能被重写的说明文字”**，选**紧跟在 `.ss-grid` 后面的另一个小节标题**（它本身就是隔断符）。
  // ⚠️ 锚点又换了一次（2026-11-14）：『显示弹幕』并进『弹幕设置』、后者自己变成开关（`.sw-toggle`），
  //    `<span class="small muted flex-none">弹幕设置</span>` 不再存在 ⇒ 旧锚点会让 ssTpl 切到 -1 之后（整段）。
  //    现在锚在**那枚开关自己的闭合标签**上：`>弹幕设置</label>`。
  const ssEnd = polishView.indexOf('>弹幕设置</label>')
  const ssTpl = polishView.slice(polishView.indexOf('<div class="ss-grid"'), ssEnd)
  check('⑰ 字幕样式块的右端锚点仍然存在（锚点烂了会让下面那条判空转/恒红）',
    ssEnd > polishView.indexOf('<div class="ss-grid"'), true)
  // 2026-11-14 口径：**第 1 行 = 字幕颜色 + 描边颜色 + 白边（3 格，`.cols-3`）**、第 2 行 = 两个数值。
  check('⑰ 字幕样式 = 两行（第 1 行两色 + 白边三格、第 2 行两个数值）',
    (ssTpl.match(/<div class="ss-row-2/g) || []).length === 2
    && /<div class="ss-row-2 cols-3">/.test(ssTpl)
    && (ssTpl.match(/<div class="ss-row"/g) || []).length === 4, true)
  // ⚠️⚠️ 这一条是**两次翻车换来的**，别改松：
  //   ① 只写 `.ss-num { flex:1 1 auto }` 没用 —— `.rv-input`（同特异性、写在文件**后面**）
  //      带着 `flex:0 0 auto; width:6ch`，源码顺序在后者胜 ⇒ 数字框被钉在 ~32px，
  //      `400` 显示不全（用户："两列的排版有问题"）。
  //   ② 同理 `.ss-row-2 .ss-lbl { width:44px }` 会被后面那条 `.ss-lbl { width:62px }` 盖掉
  //      （实测 `numBoxW:[266,266]` —— 数字框吞掉整行）。
  //   所以**两者都必须用子选择器加深特异性**，这条判据同时钉住这两处。
  check('⑰ 两列态的 `.ss-num` 用子选择器压过 `.rv-input`（只写 .ss-num 会被宽度规则盖掉）',
    /\.ss-row-2 > \.ss-row > \.ss-num \{[^}]*flex:1 1 auto[^}]*min-width:0/.test(polishView), true)
  check('⑰ 两列态的 `.ss-lbl` 用子选择器压过基础规则的 62px',
    /\.ss-row-2 > \.ss-row > \.ss-lbl \{ width:44px; \}/.test(polishView), true)
  // ⚠️ 角标在两列里放不下（文字 44px + 角标 24px + 间距 > 44px 列宽）⇒ 已删，改由 title 承载。
  check('⑰ 两列里不再挂「近似」角标（放不下会被裁掉一半）',
    /<span class="ss-lbl">描边宽度<\/span>/.test(polishView), true)
  // 页签必须在**最上面**、且那个大标题已删。
  check('⑰ 三个页签独占最上面一行（`.side-tabs` 在 ss-grid 之前）',
    polishView.indexOf('<div class="side-tabs">') < polishView.indexOf('<div class="ss-grid"'), true)
  check('⑰ 右栏那个「项目设置」大标题已删（页签名已自明）',
    /<h2 class="m-0 card-h2"><Icon name="sliders"/.test(polishView), false)
  // ---- 「重置位置 / 重置大小」必须在「字幕字体」那一行（用户口径：紧挨着它们管的控件）----
  // ⚠️ 2026-10 用户改口：「位置→默认 / 大小→默认 改成 重置位置，重置大小」——**文案与参照物都变了**：
  //    在「字幕字体」那一行（`.ss-font-row`）里的，现在是**「字幕字体」标签 + 两颗重置按钮**，
  //    而它们管的控件（字幕位置 / 字幕大小）紧跟在下面 —— 这才是"按钮贴着它复位的东西"。
  // ⚠️ 判据要求**两件事同时成立**：① 两颗按钮都落在 `.ss-font-row` 这个容器里；
  //    ② 全页各只出现一次（防止"新位置加了、老位置忘了删"—— 那会渲染出两组一模一样的按钮）。
  //    ⚠️⚠️ **红线在 2026-10-12 搬了位置**：上一版要求"两颗按钮落在 `.ss-font-row` 里"，
  //    但用户的口径是「重置位置 / 重置大小」——**按钮要贴着它复位的东西**，
  //    而它们复位的是「字幕位置」滑杆与「字幕大小」数字框（在 `.ss-grid` 里）
  //    ⇒ 按钮搬到了 `.ss-grid` **之后**。旧判据留着的结果是：
  //    代码按用户口径搬好了，判据却要求它搬回去 —— 那正是"判据在测另一个东西"。
  //    现在钉"按钮紧跟在 `.ss-grid` 之后、且在 `.ss-font-row` **之前**（字体行不再有按钮）"。
  // ⚠⚠ **口径已经反过来了**（2026-10-13，用户：「重置大小这两个按钮…放在 **字幕字体**的后方
  // 也就是**同一行**，将重置改为**符号**，精简一点」）。
  //
  // 历史：曾经叫「位置→默认 / 大小→默认」→ 2026-10-12 改名「重置位置/重置大小」并搬到 `.ss-grid` 之后
  // → 2026-10-13 又搬回字体行、只留符号。⇒ 以**当前**口径为准。
  //
  // ⚠️ 判据盯三件事（都是这次真改的）：
  //   ① 两颗按钮**在 `.ss-font-row` 里**（与「字幕字体」同一行）；
  //   ② **不再**单独占一个 `.ss-row-2`（否则就是两行、又回到旧样子）；
  //   ③ 文案是符号（↺/⟳）而非「重置位置/重置大小」。
  const resetPlacementOk = (src) => {
    const grid = src.indexOf('<div class="ss-grid"');
    const btn = src.indexOf('@click="restoreSubtitlePos"');
    if (grid < 0 || btn < 0) return false;
    const holder = src.lastIndexOf('<div', btn);
    const fontRow = src.indexOf('<div class="row ss-font-row">');
    if (fontRow < 0 || holder !== fontRow) return false;
    // ② 按钮卡在 `.ss-grid` 与 `.ss-font-row` **之间的独立块**里 ⇒ 旧样子，算错。
    if (btn > grid && btn < fontRow) return false
    return true
  }
  check('⑰ 两颗重置按钮在「字幕字体」那一行里（同一行）',
    resetPlacementOk(polishView), true);
  {
    // 变异：把按钮搬回 `.ss-grid` 之后的独立行 ⇒ 必须翻面。
    const bad = polishView.replace(
      '<div class="row ss-font-row">',
      '<div class="ss-row-2"><button @click="restoreSubtitlePos">↺</button></div>\n    <div class="row ss-font-row">',
    );
    check('⑰ [变异] 把重置按钮搬回独立行 → 判据翻面', resetPlacementOk(bad), false);
  }
  {
    // ③ 文案必须是符号：两颗按钮的可见文本里**不许**出现「重置位置」/「重置大小」。
    const tplOnly = polishView.slice(0, polishView.indexOf('<script'))
    check('⑰ 两颗按钮的可见文案是符号（不再是「重置位置/重置大小」）',
      !/>\s*重置位置\s*</.test(tplOnly) && !/>\s*重置大小\s*</.test(tplOnly), true);
    // 两枚符号必须**能区分**（不能两个一模一样）。
    check('⑰ 两颗按钮的符号不一样（能区分位置与大小）',
      /\u21ba/.test(tplOnly) && /\u27f3/.test(tplOnly), true);
  }
  check('⑰ 两颗重置按钮全页各只出现一次（新位置加了、老位置别再留一份）',
    // ⚠️ 判**模板**（`<script>` 之前）：`<script setup>` 里本来就有这两条函数定义，
    //    拿整份文件数会恒等于 2 —— 判据变成永远红的空转。只数模板里的绑定。
    (polishView.slice(0, polishView.indexOf('<script')).match(/@click="restoreSubtitlePos"/g) || []).length === 1
    && (polishView.slice(0, polishView.indexOf('<script')).match(/@click="restoreSubtitleSize"/g) || []).length === 1, true)
  // ⚠️ 旧文案「位置→默认 / 大小→默认」**不许再出现在模板里**（用户点名要改的文案）：
  //    留着就等于"改了代码没改界面"。只在注释里提旧名可以，行内文案不许有。
  //
  // ⚠️⚠️ **必须先把 HTML 注释剥掉再判**（2026-10-12 修）：这条判据原先拿整段模板正则，
  //    而上一次改动**在注释里解释了"为什么改名"**（正当地引用了旧文案）⇒ 判据翻红，
  //    红的原因是注释、不是界面 —— 那正是「判据在测另一个东西」。
  //    剥注释的义务在判据这边（注释里引用旧文案是**该做的**）。
  const tplOnly = polishView.slice(0, polishView.indexOf('<script')).replace(/<!--[\s\S]*?-->/g, '')
  check('⑰ 旧文案「位置→默认 / 大小→默认」已从模板里清掉',
    /(位置|大小)→默认/.test(tplOnly), false)
  // ⚠️ 同理，「背景音乐」那一栏**整组已删**（用户 2026-10：「将项目设置的这一栏删除，
  //    暂且用不到 bgm」）—— 只删**入口**，Rust 侧 `[bgm]` 与 `render/bgm.rs` 一个字都没动。
  //    模板里再出现 `onBgm*` / `bgm.` 就是那一栏被捡回来了。
  //    ⚠️ 同样**剥注释**（删掉的那段在注释里被逐条点名，不剥就一直红）。
  check('⑰ 「背景音乐」分组已整组删除（模板里没有 BGM 控件与处理器）',
    /onBgm|bgm\./.test(tplOnly), false)
  // ⚠️ 「显示弹幕」与**「弹幕设置」标题同一行**（2026-10 用户口径：「显示弹幕放在 *弹幕设置*
  //    同一行的后面」）。判据钉三件事：
  //      ① 标题在勾选框之前、勾选框在**弹幕字体下拉之前**（它跟标题同行，不再跟下拉同行）；
  //      ② 标题与勾选框之间**不许夹换行容器**（夹了就是掉到下一行、又变回独立一行）；
  //      ③ 勾选框真的还在（`toggleLane('danmaku')` 是本仓唯一的那枚显示开关）。
  // ⚠️ 锚点纪律（两次踩过）：`@change="onDanmakuFontChange"` 在 `<script>` 里也有定义 ⇒ 必须用
  //    剥过注释、且截到 `<script>` 之前的 `tplOnly`；`>显示弹幕<` 那种带尖括号的锚会因为
  //    "那一行是纯文本、前后是换行"而返回 -1 ⇒ 一律用**裸文案 + 唯一绑定字符串**做锚。
  // ⚠️ **2026-11-14 口径再变**（用户：「显示弹幕并入弹幕设置，用之前主播名那种开关方式」）：
  //    不再有独立的「显示弹幕」勾选框 —— **「弹幕设置」四个字本身就是开关**（`.sw-toggle`，
  //    点一下显示/隐藏，亮=显示、灰=不显示）。判据改成钉「开关长在标题自己身上」：
  //      ① 标题的前一个 `<label` 必须带 `sw-toggle`（否则就是退回普通文字/独立勾选框）；
  //      ② 那个 label 上必须挂 `toggleLane('danmaku')`（本仓唯一的那枚显示开关）；
  //      ③ 弹幕字体下拉仍在它后面。
  const dmToggleBesideHead = (s) => {
    const head = s.indexOf('>弹幕设置</label>')
    const font = s.indexOf('@change="onDanmakuFontChange"')
    if (head < 0 || font < 0) return false
    const open = s.lastIndexOf('<label', head)
    if (open < 0) return false
    const tag = s.slice(open, head)
    return /class="sw-toggle/.test(tag)
      && /@click\.prevent="toggleLane\('danmaku'\)"/.test(tag)
      && font > head
  }
  const dmToggleBesideHead_OLD = (s) => {
    const head = s.indexOf('弹幕设置')
    const lbl = s.indexOf('显示弹幕')
    const bind = s.indexOf("toggleLane('danmaku')")
    const font = s.indexOf('@change="onDanmakuFontChange"')
    if (head < 0 || lbl < 0 || bind < 0 || font < 0) return false
    // ⚠️ **块级容器的判据不能只认 `row|col`**（2026-10-13 修的腐烂锚点）：
    //    原来的写法是 `/<div class="(row|col)[^"]*">/`，只认这两种 class。
    //    而"标题被换成块级容器"的**通用形态**是**任何** `<div …>`／`<p …>`／`<section …>`
    //    —— `.small`/`.muted` 这类 class **不带 `display`**（`app.css:111-112` 只写 color/font-size）
    //    ⇒ 一个 `<div class="small muted">` **本来就是块级**，足以把勾选框挤到下一行。
    //    症状就是：**变异体真的破坏了布局，而判据不翻面** ⇒ 变异台报"❌ 没打红"，
    //    于是这条不变式**静默失效**（锚点腐烂），比"报错"难发现得多。
    //
    //    ⚠️⚠️ **必须往前看**：标题**自己**那个开标签在 `head` **之前**
    //    （`indexOf('弹幕设置')` 落在标签**内容**上），所以切片从 `head` 开始只会看到
    //    **闭合**标签 `</div>` —— 第一版就是这么写的，正则恒不命中、判据照旧不翻面。
    //    正确的窗口是"标题开标签之前一小段 + 标题与勾选框之间"。
    //    留 40 字符足以覆盖 `<div class="small muted">` 这量级的开标签。
    const from = Math.max(0, head - 40)
    const window = s.slice(from, lbl)
    const blockish = /<(div|p|section|ul|ol|h[1-6]|tr|li)\b[^>]*>/.test(window)
    return head < lbl && lbl < font && bind <= lbl + 20 && !blockish
  }
  check('⑰ 「弹幕设置」自己就是显示开关（.sw-toggle + toggleLane），字体下拉在它后面',
    dmToggleBesideHead(tplOnly), true)
  {
    // 变异：把这枚开关的点击绑定摘掉（退回"只剩文字"）⇒ 判据必须翻面。
    const bad = tplOnly.replace('@click.prevent="toggleLane(\'danmaku\')"', '')
    check('⑰ [变异] 把「弹幕设置」退回普通文字 → 判据翻面', dmToggleBesideHead(bad), false)
  }
  check('⑰ 字体行有 `min-width:0`（否则中文标签 + 两颗按钮会横向溢出）',
    /\.ss-font-row \{ display:flex; align-items:center; gap:6px; min-width:0/.test(polishView), true)
  check('⑰ 任务栏默认折叠（jobOpen 初值 false）', /const jobOpen = ref\(false\)/.test(polishView), true)
  check('⑰ 任务详情是**浮层**（不挤占下方控件）',
    /\.job-wrap \{ position:relative; \}/.test(polishView)
    && /\.job-wrap > \.row, \.job-wrap > \.job-log \{ position:absolute; top:100%/.test(polishView), true)
  check('⑰ ⚠️ 别把 absolute 写到 .job-wrap 上（整条任务栏会逃出卡片）',
    /\.job-wrap \{[^}]*position:absolute/.test(polishView), false)
  check('⑰ 三段设置**同时**在一个面板里（不是一轨一屏）', allGroupsAtOnce(polishView), true)
  check('⑰ 面板里**只留控件**（没有 hint 行 / 计数标题）', onlyControls(polishView), true)
  check('⑰ 面板里没有纯信息组（分段/屏蔽、音效都已删）', noInfoOnly(polishView), true)
  check('⑰ 点轨道名 = 焦点高亮 + 滚到那一段', focusScrolls(polishView), true)
  const MUT17 = [
    ['摘掉"轨道设置打开时让位"（两份面板并存）', timelineEditor, (s) => s.replace('<div v-if="!laneOpen" class="props-panel card"', '<div class="props-panel card"'), (m) => nameOpens(m) && itemYields(m)],
    // ⚠️ 变异要跟着**当前源码的形状**走（2026-10-14：`lane.type` → `laneTypeOf(lane)`）。
    //    锚点过时的症状是**变异静默失效**（replace 找不到东西 ⇒ 源码没变 ⇒
    //    "判据翻面"那条判据失败），而不是报"锚点找不到" —— 这里刻意两条都换了，
    //    只要今天源码里是哪种写法，变异都能真的改到东西。
    ['轨道名不可点（没有入口）', timelineEditor, (s) => s
      .replace("@click.stop=\"emit('lane-open', laneTypeOf(lane))\"", '')
      .replace("@click.stop=\"emit('lane-open', lane.type)\"", ''), (m) => nameOpens(m)],
    ['往 Teleport 目标里塞子节点', polishView, (s) => s.replace('<div id="tl-props-host"></div>', '<div id="tl-props-host"><span></span></div>'), (m) => panelOk(m)],
    ['把「画面尺寸」页签删掉（回不去调画布）', polishView, (s) => s.replace("sideTab === 'canvas'", "sideTab === 'canvasX'"), (m) => hasSideTabs(m)],
    ['画面尺寸那块搬出页签（不再同一位置切换）', polishView, (s) => {
      // ⚠️ 锚在项目页签**之后**的那个 v-else（文件里 1000 行附近还有别的 v-else）
      const p = s.indexOf("sideTab === 'project'")
      const i = s.indexOf('<template v-else>', p)
      const j = s.indexOf('</template>', i)
      return s.slice(0, i) + s.slice(i, j).replace('orientationOptions', 'canvasOptsX') + s.slice(j)
    }, (m) => canvasInsideTab(m)],
    ['把项目设置那一半删掉（用户说"没想到"的那件事）', polishView, (s) => {
      const i = s.indexOf("<template v-if=\"sideTab === 'project'\">")
      const j = s.indexOf('<template v-else>', i)
      return s.slice(0, i) + s.slice(i, j).replace('runLlmJob', 'llmJobX') + s.slice(j)
    }, (m) => projectInsideTab(m)],
    ['右栏又拆成多块卡片（把时间轴挤走）', polishView, (s) => {
      const rc = s.indexOf('<div class="right-col">')
      const at = s.indexOf('<div class="card">', rc)
      return s.slice(0, at) + '<div class="card"></div>' + s.slice(at)
    }, (m) => lanePanelResident(m)],
    ['右栏两块不包包装（第二块会掉到下一行占满整宽）', polishView, (s) => s.replace('class="right-col"', 'class="right-col-x"'), (m) => rightColWrapped(m)],
    ['选中块不收起轨道设置', timelineEditor, (s) => s.replace("if (props.laneOpen) emit('lane-open', '')", ''), (m) => blockSelectClosesLane(m)],
    ['退回"一轨一屏"（点谁换谁）', polishView, (s) => s.replace(`<div class="lane-group" :class="{ on: openLane === 'subtitles' }"`, `<div v-if="openLane === 'subtitles'" class="lane-group"`), (m) => allGroupsAtOnce(m)],
    ['摘掉一段（面板里少一条轨道的设置）', polishView, (s) => s.replace('<div class="lane-group-h">区域设置</div>', ''), (m) => allGroupsAtOnce(m)],
    ['把「分段 / 屏蔽」那个纯信息组装回来', polishView, (s) => s.replace('<div class="lane-group-h">区域设置</div>', '<div class="lane-group-h">分段 / 屏蔽</div><div class="lane-group-h">区域设置</div>'), (m) => allGroupsAtOnce(m) && noInfoOnly(m)],
    ['把「音效」音量组装回来', polishView, (s) => s.replace('<div class="lane-group-h">区域设置</div>', '<div class="lane-group-h">音效</div><div class="lane-group-h">区域设置</div>'), (m) => allGroupsAtOnce(m) && noInfoOnly(m)],
    // 变异：「自动」那颗挪进一个独立的 `.row` ⇒ 三颗不再同行
    ['把「自动」那颗挪到另一行（三颗不再同行）', polishView,
      (s) => s.replace('      <button class="mini-btn flex-1 jc-center"' + String.fromCharCode(10) + '              :class="{ selected: stickerCfg.position_mode === \'manual\' }"',
                       '      </div>' + String.fromCharCode(10) + '      <div class="row mt-4">' + String.fromCharCode(10) + '      <button class="mini-btn flex-1 jc-center"' + String.fromCharCode(10) + '              :class="{ selected: stickerCfg.position_mode === \'manual\' }"'),
      (m) => selectBtnsOneRow(m)],
    ['往面板里加回一条解释文字（hint 行）', polishView, (s) => s.replace('<div class="lane-group-h">区域设置</div>', '<div class="lane-group-h">区域设置</div><div class="hint mt-4">开关只影响渲染，不改事件</div>'), (m) => onlyControls(m)],
    ['加回"音效事件 N 条"那行读数', polishView, (s) => s.replace('<div class="lane-group-h">区域设置</div>', '<div class="lane-group-h">区域设置</div><div class="row spread small mt-4"><span class="muted">音效事件</span></div>'), (m) => onlyControls(m)],
    ['点轨道名不再滚到那一段', polishView, (s) => s.replace("if (el && typeof el.scrollIntoView === 'function') el.scrollIntoView({ block: 'nearest' })", ''), (m) => focusScrolls(m)],
  ]
  for (const [what, src, mutate, still] of MUT17) {
    const m = mutate(src)
    check(`[变异] ⑰ ${what} → 源码确实被改了`, m !== src, true)
    check(`[变异] ⑰ ${what} → 判据翻面`, still(m), false)
  }
}

// ── ⑱ 弹幕字体 / 显示开关（无独立泳道 ⇒ 住字幕轨面板；量宽与渲染同族）（2026-10-06）──
{
  const noDanmakuLane = (s) => !s.includes("lane: 'danmaku'")
  // 2026-11-14：显示开关不再是 checkbox，而是「弹幕设置」这个 `.sw-toggle` 标签（点标签切换）。
  const dmToggleInPanel = (s) => {
    const head = s.indexOf('>弹幕设置</label>')
    if (head < 0) return false
    const open = s.lastIndexOf('<label', head)
    if (open < 0) return false
    const tag = s.slice(open, head)
    return /class="sw-toggle/.test(tag) && /@click\.prevent="toggleLane\('danmaku'\)"/.test(tag)
  }
  const dmFontSaves = (s) => s.includes('saveProjectPatch(pid.value, dir.value, { danmaku_font: next })')
  // ⚠️ **2026-10 口径再变一次**（用户：「弹幕设置的字体为什么还显示跟字幕？字幕不是乐米波波体，
  //   而弹幕默认为 noto」）：空值从"跟随字幕那条字体栈"改成**出厂默认文字字体**（`Noto Sans SC`）。
  //   旧口径的两个真问题：① 字幕那栏存的是**族名**、弹幕这栏存的是**库 id**，格式本就不同；
  //   ② **真出片通道（底座转译器）从来不实现这一档**（填 `'Sans'`）⇒ 预览是字幕那支、
  //      成片是 `Sans`，**不一致且不报错**。所以判据现在钉的是"三处都退到同一个常量"。
  //   判据的意图没变（量宽必须带字体族、缓存键必须含字体族、视频与绘制同族），只换锚点。
  const DM_DEFAULT = 'Noto Sans SC';
  const measureByFamily = (s) =>
    s.includes('export function measureDanmakuWidths(dms, fontSize, fontFamily)')
    && s.includes('hit.size === size && hit.family === family')
    && s.includes("dmMeasureCtx.font = '600 ' + size + 'px ' + (family || 'sans-serif')")
    && s.includes('function dmFontFamily(cfg)')
    // 空值必须退到**出厂默认那支**（不是裸 sans-serif、也不是字幕那支）：
    && new RegExp("const fam = own \\|\\| '" + DM_DEFAULT + "'").test(s)
    // 旧的"跟随字幕"读法必须已经不在这个函数里（回归钉子）：
    && !s.includes('const sub = (cfg.style && cfg.style.subtitle && cfg.style.subtitle.font)')
  const templateMeasuresByFamily = (s) =>
    s.includes('function dmFontFamily()')
    && s.includes('cx.font = \'600 \' + fs + \'px \' + dmFontFamily();')
    // 绘制那条腿必须与量宽走**同一个**函数（否则量宽字体 ≠ 画的字体）
    && s.includes("ctx.font = '600 ' + fs_dm + ' ' + dmFontFamily();")
    && new RegExp("var fam = own \\|\\| '" + DM_DEFAULT + "'").test(s)
    && !s.includes('CFG.style.subtitle && CFG.style.subtitle.font')
  // ⚠️ **出片那条腿（底座转译器）也必须同口径** —— 它才是真出片路径，而它原先填的是 `'Sans'`
  //    （与上面两处不一致，正是"预览≠成片"的本体）。判据钉三件：常量在、两处消费都用它、
  //    那份 `'Sans'` 不再出现。
  const translatorMatches = (s) =>
    new RegExp("const DEFAULT_DANMAKU_FONT = '" + DM_DEFAULT + "'").test(s)
    && s.includes('font: dmStyle.font || DEFAULT_DANMAKU_FONT')
    && s.includes('font_family: fontFamilyFor(dmStyle.font || DEFAULT_DANMAKU_FONT)')
    && !s.includes("dmStyle.font ?? 'Sans'")
  const layersInheritFont = (s) =>
    /\.dm \{[\s\S]{0,700}?font-family:inherit;/.test(s) && /\.cap \{[\s\S]{0,400}?font-family:inherit;/.test(s)
  const measOk = (m) => measureByFamily(m) && templateMeasuresByFamily(templateHtml) && layersInheritFont(previewVue)
  check('⑱ 弹幕没有独立泳道（用户口径：不加泳道）', noDanmakuLane(timelineEditor), true)
  check('⑱ 弹幕的显示开关住字幕轨面板', dmToggleInPanel(polishView), true)
  check('⑱ 弹幕字体走 `danmaku_font` 落盘', dmFontSaves(polishView), true)
  check('⑱ 预览量宽带字体族 + 缓存键含字体族', measureByFamily(engineJs), true)
  check('⑱ 模板 canvas 量宽带字体族（与预览同口径）', templateMeasuresByFamily(templateHtml), true)
  check('⑱ 出片转译器同口径（空值 = 出厂默认，不再是 `Sans`）', translatorMatches(translator), true)
  check('⑱ 下拉首项不再叫「跟字幕」（用户点名）', /label: '默认 · Noto Sans SC'/.test(polishView), true)
  check('⑱ `.dm` / `.cap` 声明 `font-family:inherit`（否则被全局 `*` 规则盖掉）', layersInheritFont(previewVue), true)
  const MUT18 = [
    ['量宽的缓存键丢掉字体族（换字体拿旧宽度 ⇒ 没滚到边就消失）', engineJs, (s) => s.replace('hit.size === size && hit.family === family', 'hit.size === size'), measOk],
    ['预览量宽退回 sans-serif（丢掉字体族）', engineJs, (s) => s.replace(" + (family || 'sans-serif')", ''), measOk],
    ['`.dm` 的 inherit 摘掉（字体设置静默失效）', previewVue, (s) => s.replace('      font-family:inherit; }', '      }'), layersInheritFont],
    ['模板量宽退回 sans-serif（丢掉字体族）', templateHtml, (s) => s.replace(" + dmFontFamily();\n    for (var i = 0", "\n    for (var i = 0"), templateMeasuresByFamily],
    ['预览弹幕字体退回裸 sans-serif（丢掉出厂默认）', engineJs, (s) => s.replace(`const fam = own || '${DM_DEFAULT}'`, "const fam = own || 'sans-serif'"), measOk],
    ['模板弹幕字体退回裸 sans-serif（丢掉出厂默认）', templateHtml, (s) => s.replace(`var fam = own || '${DM_DEFAULT}'`, "var fam = own || 'sans-serif'"), templateMeasuresByFamily],
    ['预览又改回"跟随字幕"（预览与成片不一致的老病）', engineJs, (s) => s.replace(`const fam = own || '${DM_DEFAULT}'`, 'const fam = own || (cfg.style && cfg.style.subtitle && cfg.style.subtitle.font)'), measOk],
    ['转译器退回 `Sans`（成片与预览分叉、且不报错）', translator, (s) => s.replace('font: dmStyle.font || DEFAULT_DANMAKU_FONT', "font: dmStyle.font ?? 'Sans'"), translatorMatches],
    ['转译器改回不填默认（空值一直传到 libass）', translator, (s) => s.replace('fontFamilyFor(dmStyle.font || DEFAULT_DANMAKU_FONT)', 'fontFamilyFor(dmStyle.font)'), translatorMatches],
    ['弹幕面板里那枚显示开关摘掉', polishView, (s) => s.replace("@click.prevent=\"toggleLane('danmaku')\"", ''), dmToggleInPanel],
  ]
  for (const [what, src, mutate, still] of MUT18) {
    const m = mutate(src)
    check(`[变异] ⑱ ${what} → 源码确实被改了`, m !== src, true)
    check(`[变异] ⑱ ${what} → 判据翻面`, still(m), false)
  }
}
// ── ⑨ 「打开输出目录」= **只在本机渲染** + 交给服务端弹文件管理器（2026-10-05）──
// 用户口径：local 模式点一下**直接弹出文件夹**；server 模式**不显示这个按钮**。
// 两件事都必须钉：① `v-if="isLocal"`（服务端模式回 404，按钮露出来就是"点了没反应"）；
// ② 点击走 `openPath(dir)`（服务端 spawn 文件管理器），**不许**退回"复制路径 + toast"。
{
  const gated = (s) => s.includes('v-if="isLocal"')
  const wired = (s) => s.includes('await openPath(p)')
  const noCopy = (s) => !s.includes('copyOutDir')
  const health = (s) => s.includes("import { localNoAuthKnown } from '../stores/health'")
    && s.includes('const isLocal = localNoAuthKnown()')
  check('⑨ 按钮按 isLocal 门控（server 模式不渲染）', gated(polishView), true)
  check('⑨ 点击走 openPath（不是复制路径）', wired(polishView) && noCopy(polishView), true)
  check('⑨ isLocal 取自 health store（与路由守卫同一份缓存）', health(polishView), true)
  const MUT9 = [
    ['摘掉 isLocal 门控（server 模式也显示）', (s) => s.replace('v-if="isLocal"', 'v-if="true"')],
    ['退回"复制路径"（不弹文件夹）', (s) => s.replace('await openPath(p)', 'await navigator.clipboard.writeText(p)')],
    ['门控改成永远为真（自算 live 判定）', (s) => s.replace('const isLocal = localNoAuthKnown()', 'const isLocal = true')],
  ]
  for (const [what, mutate] of MUT9) {
    const m = mutate(polishView)
    check(`[变异] ⑨ ${what} → 源码确实被改了`, m !== polishView, true)
    check(`[变异] ⑨ ${what} → 判据翻面`, gated(m) && wired(m) && noCopy(m) && health(m), false)
  }
}

// ── ⑩ 面板快捷键不许在输入框里抢键（一个 Delete 删掉整个文本图层那件事）──
// 用户原话：「封面设计器…我删文本框里的文字，一个 delete 把整个文本图层都删除了」。
// `CoverDesigner::onPanelKey` 绑在整块 `.controls` 上，而文本框/数值框都在里面 ⇒ 打字时
// Delete 冒泡到面板 = 删图层。判据分两层：
//   ① **行为**：把 `utils.isEditableKeyEvent` 从源码抽出来，喂 6 组假事件（这一步是真跑的）；
//   ② **接线**：四处面板快捷键都真的过它（少一处就是同一类事故的复现）。
{
  const hit = utilsJs.match(/export function isEditableKeyEvent\(e\) \{[^]*?\n\}/)
  check('⑩ 能从 utils.js 抽出 isEditableKeyEvent', hit !== null, true)
  if (hit !== null) {
    const fn = new Function(`${hit[0].replace('export function', 'function')}\nreturn isEditableKeyEvent`)()
    const cases = [
      ['文本框', { target: { tagName: 'TEXTAREA' } }, true],
      ['输入框', { target: { tagName: 'INPUT' } }, true],
      ['下拉框', { target: { tagName: 'select' } }, true],
      ['富文本', { target: { tagName: 'DIV', isContentEditable: true } }, true],
      ['输入法合成中', { isComposing: true, target: { tagName: 'DIV' } }, true],
      ['普通面板/画布', { target: { tagName: 'DIV' } }, false],
      ['没有 target', { }, false],
    ]
    for (const [what, ev, want] of cases) check(`⑩ ${what} ⇒ 归控件？`, fn(ev), want)
    // 变异：把 `INPUT` 那一支删掉 ⇒ 输入框立刻不再被认成可编辑
    const noInput = hit[0].replace("tag === 'INPUT' || ", '')
    check('[变异] ⑩ 删掉 INPUT 分支的替换命中了源码', noInput !== hit[0], true)
    const fnNoInput = new Function(`${noInput.replace('export function', 'function')}\nreturn isEditableKeyEvent`)()
    check('[变异] ⑩ 输入框不再算可编辑 ⇒ 行为判据翻面', fnNoInput({ target: { tagName: 'INPUT' } }), false)
  }
  const passes = (src) => /isEditableKeyEvent\(e\)/.test(src)
  check('⑩ 封面设计器接管快捷键前先过它（bug 本体）', passes(coverDesigner), true)
  check('⑩ 时间轴编辑器的 Space 也过它', passes(timelineEditor), true)
  check('⑩ 预处理页的 Space 也过它', passes(prepareView), true)
  check('⑩ 切片页的 Space/方向键也过它', passes(clipsSection), true)
  // 变异：把守卫整行摘掉 ⇒ 判据必须翻面（两处调用点各来一次）
  const MUT10 = [
    ['封面设计器摘掉那道闸', coverDesigner],
    ['时间轴编辑器的守卫整行删掉', timelineEditor],
  ]
  for (const [what, src] of MUT10) {
    const m = src.replace('if (isEditableKeyEvent(e)) return', '/* no guard */')
    check(`[变异] ⑩ ${what} → 源码确实被改了`, m !== src, true)
    check(`[变异] ⑩ ${what} → 判据翻面`, passes(m), false)
  }
}
// ── ⑪ 动图「已加载/已失败」必须绑引擎代（否则热重载后贴纸消失，只能靠 F5 回来）──
// 事实链：底座 `dhampir_project_open` 成功即 `clear_animations()`（跨仓，见 Dhampir 的
// `crates/dhampir-wasm/src/timeline_host.rs`）⇒ 引擎手上没有动图了；而宿主 `animLoaded` 按 uri
// 继承 + `dhampirLoadAnimations` 短路 ⇒ 不再交字节 ⇒ 贴纸不画。每次编辑都会热重载一次，
// 所以这是**必然**而不是偶发。判据与变异**共用同一份** `ok11`（否则变异只翻一半）。
{
  const declRe = /let dhampirAnimGen = 0/
  const loadedSkip = /if \(st\.animLoaded && st\.animUri === a\.uri && st\.animGen === dhampirAnimGen\) continue/
  const errorSkip = /if \(st\.animError && !st\.animPendingAttach && st\.animGen === dhampirAnimGen\) continue/
  // **每个** open()/attach() 之后都要 +1：引擎在这两处都会把动图缓存清空
  const bumpsAfter = (t, needle, window) => {
    let at = t.indexOf(needle)
    let n = 0
    while (at >= 0) {
      n += 1
      if (!t.slice(at, at + window).includes('dhampirAnimGen += 1')) return -1
      at = t.indexOf(needle, at + 1)
    }
    return n
  }
  const ok11 = (t) => declRe.test(t) && loadedSkip.test(t) && errorSkip.test(t)
    && bumpsAfter(t, 'dhampirEngine.open(project)', 520) > 0
    && bumpsAfter(t, 'await dhampirEngine.attach(canvas.id)', 420) === 1
  check('⑪ 整条不变式在（声明 + 两项短路带代际 + 每个 open/attach 都 +1）', ok11(previewVue), true)
  check('⑪ 声明了引擎代 dhampirAnimGen', declRe.test(previewVue), true)
  check('⑪ 「已加载」的短路绑代际', loadedSkip.test(previewVue), true)
  check('⑪ 「已失败」的短路也绑代际（瞬时失败不该黏住）', errorSkip.test(previewVue), true)
  check('⑪ 每个 open() 之后都 +1（open 成功即清空动图）',
    bumpsAfter(previewVue, 'dhampirEngine.open(project)', 520) > 0, true)
  check('⑪ attach 之后也 +1（新引擎的动图缓存是空的）',
    bumpsAfter(previewVue, 'await dhampirEngine.attach(canvas.id)', 420), 1)
  const MUT11 = [
    ['「已加载」短路丢掉代际判据', (t) => t.replace(' && st.animGen === dhampirAnimGen) continue', ') continue')],
    ['open() 之后不再 +1', (t) => t.replace('dhampirAnimGen += 1', '/* no bump */')],
    ['引擎代声明被删', (t) => t.replace('let dhampirAnimGen = 0', 'let ANIM_GEN_REMOVED = 0')],
    ['attach 之后不再 +1', (t) => {
      const at = t.indexOf('await dhampirEngine.attach(canvas.id)')
      if (at < 0) return t
      const seg = t.slice(at, at + 420)
      return t.slice(0, at) + seg.replace('dhampirAnimGen += 1', '/* no bump */') + t.slice(at + 420)
    }],
  ]
  for (const [what, mutate] of MUT11) {
    const m = mutate(previewVue)
    check(`[变异] ⑪ ${what} → 源码确实被改了`, m !== previewVue, true)
    check(`[变异] ⑪ ${what} → 判据翻面`, ok11(m), false)
  }
}
// ── ⑫ 剪切按钮的文案按「这一条」而不是「这个项目」（用户报：切一条，所有卡片都显示排队）──
{
  const labelOk = /cutLabel\(it\.ev\)/
  const oldBad = /\{\{ cutBusy \? '排队中' : '剪切' \}\}/
  const stateOk = /function cutState\(ev\)/
  const keyOk = /clipKey\(pid\.value, dir\)/
  const kindOk = /j\.kind === 'cut_one'/
  const runningFirst = /if \(mine\.some\(\(j\) => j\.status === 'running'\)\) return 'running'/
  const wordsOk = /'running' \? '剪切中' : s === 'queued' \? '排队中' : '剪切'/
  const ok12 = (t) => labelOk.test(t) && !oldBad.test(t) && stateOk.test(t)
    && keyOk.test(t) && kindOk.test(t) && runningFirst.test(t) && wordsOk.test(t)
  check('⑫ 剪切按钮文案走每片段判据（不是项目级 cutBusy）', ok12(clipsSection), true)
  check('⑫ 每片段判据存在且按 clipKey 查', stateOk.test(clipsSection) && keyOk.test(clipsSection), true)
  const MUT12 = [
    ['退回项目级 cutBusy 当文案', (t) => t.replace("{{ cutLabel(it.ev) }}", "{{ cutBusy ? '排队中' : '剪切' }}")],
    ['去掉 cut_one 过滤（渲染/精修也算剪切中）', (t) => t.replace("j.kind === 'cut_one' && ", '')],
    ['去掉「在跑优先」那一句（在跑的反而不显示）', (t) => t.replace("  if (mine.some((j) => j.status === 'running')) return 'running'" + String.fromCharCode(10), '')],
  ]
  for (const [what, mutate] of MUT12) {
    const m = mutate(clipsSection)
    check(`[变异] ⑫ ${what} → 源码确实被改了`, m !== clipsSection, true)
    check(`[变异] ⑫ ${what} → 判据翻面`, ok12(m), false)
  }
}
// ── ⑬ `/api/jobs` 空闲降频，但空闲间隔必须 < SNAPSHOT_TTL_MS（门控靠快照新鲜度）──
{
  const num = (src, re) => {
    const m = src.match(re)
    return m ? Number(m[1]) : null
  }
  const busyMs = num(jobsStore, /const POLL_BUSY_MS = (\d+)/)
  const idleMs = num(jobsStore, /const POLL_IDLE_MS = (\d+)/)
  const ttlMs = num(jobsApiJs, /export const SNAPSHOT_TTL_MS = (\d+)/)
  check('⑬ 两个档位常量都在 useJobs 里', busyMs !== null && idleMs !== null, true)
  check('⑬ 快档就是原来的 3 秒（不偷偷改快的那一档）', busyMs, 3000)
  check('⑬ 空闲档比快档慢（真的降了）', idleMs !== null && busyMs !== null && idleMs > busyMs, true)
  check(
    '⑬ 空闲档 < 快照保鲜期（否则 busyOn 门控会一直 stale）',
    idleMs !== null && ttlMs !== null && idleMs < ttlMs,
    true,
  )
  check(
    '⑬ 间隔按「有在跑的任务 + 页面可见」来选',
    /function pollDelay\(/.test(jobsStore) && /status === 'running'/.test(jobsStore)
      && /visibilityState !== 'hidden'/.test(jobsStore),
    true,
  )
  check(
    '⑬ 定时器是 setTimeout 链（每轮重算间隔）而不是定死的 setInterval',
    /timer = setTimeout\(/.test(jobsStore) && /schedule\(\)/.test(jobsStore)
      && !/setInterval\(load/.test(jobsStore),
    true,
  )
  check(
    '⑬ 可见性监听在 stop 里摘掉（否则反复 start/stop 会攒一堆）',
    /removeEventListener\('visibilitychange'/.test(jobsStore),
    true,
  )
  const ok13 = (store, api) => {
    const b = store.match(/const POLL_BUSY_MS = (\d+)/)
    const i = store.match(/const POLL_IDLE_MS = (\d+)/)
    const t = api.match(/export const SNAPSHOT_TTL_MS = (\d+)/)
    if (!b || !i || !t) return false
    const [bv, iv, tv] = [Number(b[1]), Number(i[1]), Number(t[1])]
    return iv > bv && iv < tv
      && /function pollDelay\(/.test(store) && /timer = setTimeout\(/.test(store)
      && /removeEventListener\('visibilitychange'/.test(store)
  }
  const MUT13 = [
    ['空闲档改成 3 秒（等于快档，没降频）', (t) => t.replace('const POLL_IDLE_MS = 10000', 'const POLL_IDLE_MS = 3000')],
    ['空闲档改成 20 秒（超过快照保鲜期）', (t) => t.replace('const POLL_IDLE_MS = 10000', 'const POLL_IDLE_MS = 20000')],
    ['定时器退回定死的 setInterval', (t) => t.replace('  timer = setTimeout(async () => {', '  timer = setInterval(async () => {')],
  ]
  for (const [what, mutate] of MUT13) {
    const m = mutate(jobsStore)
    check(`[变异] ⑬ ${what} → 源码确实被改了`, m !== jobsStore, true)
    check(`[变异] ⑬ ${what} → 判据翻面`, ok13(m, jobsApiJs), false)
  }
}

// ── ⑮ 屏蔽区间的跳转必须用**素材帧率**换算源帧 + 段内换算必须**连续**（秒）──
//
// ① `source_in` 是**素材帧号**：宿主拿工程网格帧率去换算源帧 ⇒ "经过屏蔽时声音和视频全乱"。
// ② 段内换算若也按**素材帧**取整，工程帧只能落在 `round(src)×seqFps/srcFps` 的倍数上
//    —— 5fps 素材 + 30fps 网格 = **6 的倍数** ⇒ 一秒只有 5 个不同帧号 ⇒ `tick()` 里
//    `f === dhampirFrameShown` 把另外 5/6 帧全挡掉 ⇒ 读数 `5/30fps · 丢 30`，
//    而叠加层（字幕/弹幕/贴纸）也只动 5 次/秒 = **预览≠成片**（成片按 30fps 画）。
//    所以：段落**边界**按素材帧判（与 `source_in` 同单位），段内**换算走秒**。
{
  const has = (re) => re.test(previewVue)
  check('⑮ 源帧率从工程的视频素材取',
    has(/dhampirSourceFps = stb\.num > 0 && stb\.den > 0/), true)
  check('⑮ 源秒 → 源位置 用素材帧率（dhampirFrameAt）',
    has(/const srcPos = Math\.max\(0, \(seconds \|\| 0\) \* srcFps\)/), true)
  check('⑮ 段落长度要换算回素材帧再比',
    has(/const lenSrc = Math\.max\(1, Math\.round\(\(lenSeq \* srcFps\) \/ seqFps\)\)/), true)
  check('⑮ 跳转点也用素材帧率算秒（dhampirNextSourceStart）',
    has(/return next === undefined \? null : next \/ srcFps/), true)
  // ⚠️ 必须**按函数切片**判：`const src…` 这一行在 `dhampirFrameAt` 与 `dhampirNextSourceStart`
  // 里**各有一份**，全文匹配的话"源侧改用网格帧率"那条变异永远翻不了面（第二份还在）。
  // ⚠️ **花括号配平**取函数体（不是切到第一个换行+右花括号）：切片版少一个收尾花括号，
  // 拿它去 new Function(...) 直接语法错（这条行为判据第一版就是这么假红的）。
  // 拿它去 `new Function(...)` 直接语法错（这条行为判据第一版就是这么假红的）。
  const fnBody = (src, name) => {
    const i = src.indexOf('function ' + name)
    if (i < 0) return ''
    let depth = 0
    for (let k = i; k < src.length; k += 1) {
      if (src[k] === '{') depth += 1
      else if (src[k] === '}') {
        depth -= 1
        if (depth === 0) return src.slice(i, k + 1)
      }
    }
    return ''
  }
  // ⭐ **行为级**：把 `dhampirFrameAt` 抠出来，喂一份真形状的段落表（p4 那颗 5fps 素材的真实读数：
  // 网格 30 / 素材 5 / 三段 `(0,0) (485,109) (746,178)`），逐点对拍：
  //   ① **连续性**：邻帧步长只能是 0 或 1 —— 取整到素材帧时步长会变成 6（这条就是那个 bug）；
  //   ② 段落边界语义不变：10s→300、22.4s→503、40s→878、18s→null（落在被掐掉的区间里）。
  const frameAtOf = (t) => {
    const body = fnBody(t, 'dhampirFrameAt')
    if (!body) return null
    try {
      return new Function(
        'let dhampirFps={value:30},dhampirSourceFps=5;'
        + 'let dhampirSegments=[{frameStart:0,frameEnd:485,sourceIn:0},{frameStart:485,frameEnd:746,sourceIn:109},{frameStart:746,frameEnd:2787,sourceIn:178}];'
        + body + ';return dhampirFrameAt;',
      )()
    } catch (e) { return null }
  }
  const continuous = (f) => {
    if (typeof f !== 'function') return false
    let prev = -1
    for (let i = 0; i <= 600; i += 1) {
      const v = f(i / 600 * 10)          // 0~10s 扫 601 个点（含被掐掉的段落边界）
      if (v === null) { prev = -1; continue }   // null = 落在被掐掉的区间里：断开，重新起算
      if (v < prev) return false
      if (prev >= 0 && v - prev > 1) return false   // 步长 > 1 = 又把帧号钉回素材帧了
      prev = v
    }
    return true
  }
  const anchors = (f) => {
    if (typeof f !== 'function') return false
    return f(10) === 300 && f(22.4) === 503 && f(40) === 878 && f(18) === null
  }
  const frameAt = frameAtOf(previewVue)
  check('⑮ 段内换算**连续**（5fps 素材也 30 帧/秒：邻帧步长 ≤ 1）', continuous(frameAt), true)
  check('⑮ 段落边界语义不变（10s→300 / 22.4s→503 / 40s→878 / 18s→null）', anchors(frameAt), true)
  const ok14 = (t) => {
    const f = fnBody(t, 'dhampirFrameAt')
    const nextStart = fnBody(t, 'dhampirNextSourceStart')
    return /dhampirSourceFps = stb\.num > 0 && stb\.den > 0/.test(t)
      && /const srcPos = Math\.max\(0, \(seconds \|\| 0\) \* srcFps\)/.test(f)
      && /lenSeq \* srcFps\) \/ seqFps/.test(f)
      && /next \/ srcFps/.test(nextStart)
      && continuous(frameAtOf(t)) && anchors(frameAtOf(t))
  }
  const MUT14 = [
    ['源位置改用网格帧率（屏蔽后画面跳 6 倍远）', (t) => t.replace('const srcPos = Math.max(0, (seconds || 0) * srcFps)', 'const srcPos = Math.max(0, (seconds || 0) * seqFps)')],
    ['段落长度不换算回素材帧', (t) => t.replace('const lenSrc = Math.max(1, Math.round((lenSeq * srcFps) / seqFps))', 'const lenSrc = lenSeq')],
    ['跳转点用网格帧率算秒', (t) => t.replace('return next === undefined ? null : next / srcFps', 'return next === undefined ? null : next / seqFps')],
    ['段内换算退回素材帧（5fps 素材 ⇒ 工程帧只能是 6 的倍数 = 只画 5 帧/秒、`丢 30`）',
      // 复刻改造前那条式子：源位置**先取整到素材帧**，段内再按素材帧换算 —— 输出就只剩 6 的倍数
      (t) => t.replace('      const localSec = (seconds || 0) - seg.sourceIn / srcFps\n      return seg.frameStart + Math.round(localSec * seqFps)',
        '      const localSec = ((Math.round((seconds || 0) * srcFps) - seg.sourceIn) / srcFps)\n      return seg.frameStart + Math.round(localSec * seqFps)')],
  ]
  for (const [what, mutate] of MUT14) {
    const m = mutate(previewVue)
    check(`[变异] ⑮ ${what} → 源码确实被改了`, m !== previewVue, true)
    check(`[变异] ⑮ ${what} → 判据翻面`, ok14(m), false)
  }
}

// ── ⑲ 换片段**不重挂预览** + Dhampir 原地换片段（2026-10-06 用户报「切片段黑屏、只能 F5」）──
//
// 底座 `<canvas>` **一页只 attach 一次、没有 detach** ⇒ 编辑器整棵重挂（换节点）那一刻起，
// 引擎就再也画不到新 canvas 上（日志里首屏照样跑完，画面永远黑）。所以：
//   ① 切片段走**静默加载**（父级别卸载编辑器）；
//   ② `PolishPreview` 自己 `watch(props.dir)` 原地换片段（工程/源/字幕/贴纸/字体）；
//   ③ 真的撞上重挂（列表模式弹回等）时**自动刷新一次**自救（sessionStorage 挡刷新环）。
{
  // ① 父级：切片段静默
  const silentSwitch = (s) =>
    s.includes('load({ silent: hadEditor })') && s.includes('const hadEditor = ready.value')
  // ② 切片段要自己清的片段级草稿（以前靠"整棵重挂"顺手清零）
  // ⚠️ 这里**不再要求** `coverKey++`：2026-10-06 的事故就是它（重挂那一刻 `dir` 还没换过来 ⇒
  // 设计器读到旧片段、600ms 防抖把旧片段整套 `[cover]` 写进新片段）。封面现在自己
  // `watch(props.dir)` 换片段 + 写盘前核对基线，那两条由 `check-cover-stage.mjs` ⑧ 守（4 处变异）。
  const resetClipState = (s) =>
    /function resetClipUiState\(\) \{[\s\S]{0,700}?selectRegion\.value = ''/.test(s)
      && /function resetClipUiState\(\) \{[\s\S]{0,700}?pointEdit\.value = false/.test(s)
  // ③ 预览自己听 dir
  const dirWatcher = (s) =>
    /watch\(\(\) => props\.dir,[\s\S]{0,240}?switchDhampirClip\(\)/.test(s)
  // ④ 源元素 id 必须带**代际**：先建新源后拆旧源时，同 id 会让 `getElementById` 拿到旧那批
  const srcGen = (s) => s.includes("v.id = 'dhampir-src-' + gen + '-' + i")
  // ⑤ 换片段里：**先建源**（那段 await 期间旧画面照常画），最后才改 canvas 尺寸/open
  //    （先改尺寸 = 交换链被换掉 = 黑屏窗口被撑到几百毫秒；实测 1.5s 时平均亮度 0.18）
  const buildBeforeResize = (s) => {
    const at = s.indexOf('async function switchDhampirClip')
    if (at < 0) return false
    const fn = s.slice(at, at + 6000)
    const build = fn.indexOf('await buildDhampirSources(project)')
    const resize = fn.indexOf('canvas.width = size.w')
    return build >= 0 && resize > build
  }
  // ⑥ 自救：本页已经 attach 过别的 canvas ⇒ 自动刷一次（有闸）+ 重挂后再挂回来要清闸
  const attachSelfHeal = (s) =>
    s.includes('window.__dhampirAttachedCanvas = canvas')
      && s.includes('window.__dhampirAttachedCanvas && window.__dhampirAttachedCanvas !== canvas')
      && s.includes('location.reload()') && s.includes("sessionStorage.setItem(RELOAD_KEY, '1')")
      && s.includes("sessionStorage.removeItem('yekitrim.dhampir.attachReload')")
  // ⑦ 待办必须有人消费（首屏没起来 / 有帧在飞时被拦下那次，否则静默丢掉 = 画面停在上一个片段）
  const pendingDrain = (s) =>
    /if \(dhampirSwitchPending && !dhampirSwitching\) \{[\s\S]{0,160}?switchDhampirClip\(\)/.test(s)
  const ok = (s) =>
    silentSwitch(s) && resetClipState(s) && dirWatcher(s) && srcGen(s) && buildBeforeResize(s)
      && attachSelfHeal(s) && pendingDrain(s)
  check('⑲ 切片段走静默加载（父级别卸载编辑器）', silentSwitch(polishView), true)
  check('⑲ 片段级草稿自己清（框选态 + 贴纸落点草稿）', resetClipState(polishView), true)
  check('⑲ 预览自己听 `props.dir` 换片段', dirWatcher(previewVue), true)
  check('⑲ 源元素 id 带代际（先建后拆不撞 id）', srcGen(previewVue), true)
  check('⑲ 换片段：先建源、最后才改画布尺寸/open', buildBeforeResize(previewVue), true)
  check('⑲ 重挂自救：自动刷新一次（带闸）+ attach 成功后清闸', attachSelfHeal(previewVue), true)
  check('⑲ 换片段待办有人消费（tick 开头补跑）', pendingDrain(previewVue), true)
  const MUT19 = [
    ['切片段退回非静默 load（编辑器被卸载 = 黑屏）', polishView, (s) => s.replace('load({ silent: hadEditor })', 'load()'), silentSwitch],
    ['切片段不清贴纸落点草稿（会把上一个片段的坐标写进这个片段）', polishView, (s) => s.replace('  pointEdit.value = false\n  draftPos.value = { x: 50, y: 50 }', '  draftPos.value = { x: 50, y: 50 }'), resetClipState],
    ['预览不再听 dir（换片段后画面停在上一个片段）', previewVue, (s) => s.replace('  switchDhampirClip()\n})', '})\n', 1), dirWatcher],
    ['源 id 退回不带代际（引擎绑到旧元素上）', previewVue, (s) => s.replace("v.id = 'dhampir-src-' + gen + '-' + i", "v.id = 'dhampir-src-' + i"), srcGen],
    ['先改画布尺寸再建源（黑窗撑到几百毫秒）', previewVue, (s) => s.replace(
      "    const previous = [...dhampirVideos, ...[...dhampirTwins.values()].map((t) => t.el)]\n    const { bindings, created } = await buildDhampirSources(project)",
      "    const previous = [...dhampirVideos, ...[...dhampirTwins.values()].map((t) => t.el)]\n    { const c0 = dhampirCanvasRef.value; if (c0) c0.width = 2 }\n    const { bindings, created } = await buildDhampirSources(project)"), (s) => buildBeforeResize(s) && !s.includes('c0.width = 2')],
    ['去掉重挂自救（第二块 canvas 从此永远黑）', previewVue, (s) => s.replace('  window.__dhampirAttachedCanvas = canvas', ''), attachSelfHeal],
    ['去掉换片段待办的消费点', previewVue, (s) => s.replace('    if (dhampirSwitchPending && !dhampirSwitching) {\n      dhampirSwitchPending = false\n      switchDhampirClip()\n    }\n', ''), pendingDrain],
  ]
  for (const [what, src, mutate, still] of MUT19) {
    const m = mutate(src)
    check(`[变异] ⑲ ${what} → 源码确实被改了`, m !== src, true)
    check(`[变异] ⑲ ${what} → 判据翻面`, still(m), false)
  }
}

// ── ⑳ 多段切片：预处理页列表 → 服务端 → Cutter 一条链（2026-10-06 粗剪拓展）──
//
// 用户口径：「在预处理位置可以设置多个切片入点出点以列表形式，这样就可以分段但是合并输出」。
// 这条链**四段缺一不可**（缺哪段都是"界面能填、切出来还是单段"或"多段写不进去"）：
//   ① 界面：列表（加/删/每段时长/总时长）+ 标记落到选中行；**只剩一段不给删**；
//   ② 载荷：**多段才带 `ranges`**（单段与老的「标记入点/出点」逐字段同形 —— 老服务端/老路径零差异）；
//   ③ 服务端：认 `ranges`，`start/end` 拉成首段起、末段末，多段才写进事件；
//   ④ Cutter：多段 = 逐段剪 `.ts` → concat 不重编码（单段仍走老路径）。
console.log('\n⑳ 多段切片一条链（列表 → 载荷 → 服务端 → Cutter）')
{
  const prepareView = await read('webui/src/views/PrepareView.vue')
  const projectDataApi = await read('crates/yekitrim-server/src/api/project_data')
  const cutter = await read('crates/yekitrim-core/src/cutting/mod.rs')
  const typesRs = await read('crates/yekitrim-core/src/types/mod.rs')

  const hasList = (s) =>
    s.includes('const sliceRows = ref(') && s.includes('function addSliceRow(')
      && /function removeSliceRow\(i\) \{[\s\S]{0,200}?length <= 1[\s\S]{0,80}?return/.test(s)
  const payloadCompat = (s) => /\.\.\.\(ranges && ranges\.length > 1 \? \{ ranges \} : \{\}\)/.test(s)
  // ⚠️ 2026-11-25（T9 结构体化 + T18 拆分）：请求体不再走 `body.get("ranges")` —— 现在是强类型字段
  // `ranges: Vec<EventRangeBody>`，服务端仍按「多段才写」处理（`ranges.len() > 1`）。
  const serverTakesRanges = (s) =>
    /ranges:\s*Vec<EventRangeBody>/.test(s) && /ranges\.len\(\) > 1/.test(s) && /ranges\.push\(/.test(s)
  // 多段分支必须**真的可达**（只查"有这几个函数"的话，把 `if ranges.len() <= 1` 改成恒真
  // 也照样通过 —— 实测踩过）
  const cutterMulti = (s) =>
    /fn cut_event\([\s\S]{0,700}?if ranges\.len\(\) <= 1 \{/.test(s)
      && /fn cut_parts_and_concat\(/.test(s) && /fn run_cut_part\(/.test(s)
      && /fn concat_parts_copy\(/.test(s) && /"mpegts"/.test(s)
  // `cut_ranges()` 的语义：空 ⇒ 回退单段；非空 ⇒ 原样（只查签名同样会被"恒假"骗过）
  const emptyMeansSingle = (s) => {
    const i = s.indexOf('pub fn cut_ranges(&self)')
    if (i < 0) return false
    const body = s.slice(i, i + 700)
    return /if self\.ranges\.is_empty\(\) \{[\s\S]{0,200}?vec!\[EventRange::new\(/.test(body)
      && /self\.ranges\.clone\(\)/.test(body)
  }

  check('⑳ 界面有列表（加段/删段且**只剩一段不给删**）', hasList(prepareView), true,
    '删成 0 段 = 没有要切的东西，按钮该禁用而不是静默变成别的语义')
  check('⑳ 载荷：多段才带 ranges（单段与老路径同形）', payloadCompat(prepareView), true,
    '单段也塞 ranges 的话，老服务端/老脚本收到的是另一种形状')
  check('⑳ 服务端认 ranges（首段起、末段末，多段才写进事件）', serverTakesRanges(projectDataApi), true)
  check('⑳ Cutter 多段：逐段剪 ts + concat', cutterMulti(cutter), true)
  check('⑳ `cut_ranges()`：空 = 单段', emptyMeansSingle(typesRs), true)

  const MUT20 = [
    ['载荷一律带 ranges（单段线格式被改）', prepareView, (s) => s.replace('...(ranges && ranges.length > 1 ? { ranges } : {})', '...(ranges ? { ranges } : {})'), payloadCompat],
    ['删段不再挡"只剩一段"', prepareView, (s) => s.replace('  if (sliceRows.value.length <= 1) return\n', ''), hasList],
    ['服务端不再按多段处理 ranges', projectDataApi, (s) => s.replace('ranges.len() > 1', 'ranges.len() > 0'), serverTakesRanges],
    ['Cutter 不再逐段剪（多段退化成单段）', cutter, (s) => s.replace('    if ranges.len() <= 1 {', '    if true {'), cutterMulti],
    ['`cut_ranges()` 不再回退单段（空 ranges ⇒ 什么都不切）', typesRs, (s) => s.replace('        } else {\n            self.ranges.clone()\n        }', '        }'), emptyMeansSingle],
  ]
  for (const [what, src, mutate, still] of MUT20) {
    const m = mutate(src)
    check(`[变异] ⑳ ${what} → 源码确实被改了`, m !== src, true)
    check(`[变异] ⑳ ${what} → 判据翻面`, still(m), false)
  }
}

// ── ㉑ 左侧项目列表：**最新加入的在最上面**（用户口径 2026-10-06）──
//
// 服务端 `GET /api/projects` 是 `ORDER BY id`（**老的在前面**，`db.rs::list_projects`）——
// 侧栏必须自己翻过来。两条一起钉：
//   ① `filteredProjects` 走 `sortProjectsNewestFirst`（`created_at` 降序，同秒按 `id` 降序兜底）；
//   ② 侧栏那枚**短标识**（`.vue` 里的 `projectLabels`）仍旧按 `id` **升序**算 ——
//      列表顺序怎么变，每个项目的标识都不许跟着变（否则"项目 3"会换到另一个项目上）。
console.log('\n㉑ 侧栏项目列表：最新加入的在上（且短标识不跟着顺序变）')
{
  const workbenchStore = await read('webui/src/stores/workbench.js')
  const sideNav = await read('webui/src/components/side/SideNav.vue')

  const newestFirst = (s) =>
    /export function sortProjectsNewestFirst\(list\)/.test(s)
      && /const ca = Number\(a\?\.created_at\) \|\| 0/.test(s)
      && /return cb - ca/.test(s)
      && /return \(Number\(b\?\.id\) \|\| 0\) - \(Number\(a\?\.id\) \|\| 0\)/.test(s)
  const wired = (s) => /return sortProjectsNewestFirst\(list\)/.test(s)
      && /const list = kw[\s\S]{0,400}?: wb\.projects/.test(s)
  // 短标识的稳定性：`projectLabels` 里那次排序必须是 **id 升序**
  const labelsStable = (s) =>
    /const projectLabels = computed\(\(\) => \{[\s\S]{0,600}?sort\(\(a, b\) => Number\(a\.id\) - Number\(b\.id\)\)/.test(s)

  check('㉑ 侧栏排序：created_at 降序（同秒按 id 降序兜底）', newestFirst(workbenchStore), true,
    '服务端是 id 升序；不自己翻就是"老的在最上面"')
  check('㉑ `filteredProjects` 真的走了这个排序（过滤分支也要走）', wired(workbenchStore), true)
  check('㉑ 短标识仍按 id 升序（不随列表顺序变）', labelsStable(sideNav), true)

  // ⚠️ 每条变异带**自己那一条**判据（别写成把几条判据混在一个表达式里 —— 那会在
  // "变异打在另一个文件上"时假绿，实测第一版就是这么侥幸通过的）。
  const MUT21 = [
    ['排序方向反了（又变成老的在最上面）', workbenchStore, (s) => s.replace('    if (ca !== cb) return cb - ca', '    if (ca !== cb) return ca - cb'), newestFirst],
    ['过滤分支绕过排序（有过滤词时顺序又变回服务端顺序）', workbenchStore, (s) => s.replace('  return sortProjectsNewestFirst(list)', '  return list'), wired],
    ['短标识改成跟着列表顺序算（"项目 3"会换人）', sideNav, (s) => s.replace('  const list = [...(wb.projects || [])].sort((a, b) => Number(a.id) - Number(b.id))', '  const list = [...(wb.projects || [])]'), labelsStable],
  ]
  for (const [what, src, mutate, still] of MUT21) {
    const m = mutate(src)
    check(`[变异] ㉑ ${what} → 源码确实被改了`, m !== src, true)
    check(`[变异] ㉑ ${what} → 判据翻面`, still(m), false)
  }
}

// ── ㉒ 多段片段要有**特殊标记**（用户口径 2026-10-06：「多段的切片需要特殊标记」）──
//
// 四处卡片墙都要认得出多段（缺一处就是"有的列表看得出来、有的看不出来"）：
//   ① 切片页卡片墙（已剪卡片 + 未剪切事件卡片）
//   ② 精修页片段条（`FragmentList` 的卡片 + `PolishView::clipRow` 给的 `segments`）
//   ③ 首页「切片总览」（两栏卡片）
//   ④ 索引侧真把 `ranges` 带出来（`ClipEntry.ranges` + 从 `clip.json` 补）
// ⚠️ 判据只认 **> 1**（与写侧 `ranges.len() > 1` 同口径）：单段写了 `ranges` 也不算多段。
console.log('\n㉒ 多段片段的「N 段」标记（四处卡片墙 + 索引带出 ranges）')
{
  const clipsSection = await read('webui/src/components/ProjectClipsSection.vue')
  const polishView = await read('webui/src/views/PolishView.vue')
  const fragList = await read('webui/src/components/polish/FragmentList.vue')
  const homeOverview = await read('webui/src/components/HomeClipsOverview.vue')
  const clipIndex = await read('crates/yekitrim-core/src/cutting/clip_index.rs')

  // ⚠️ **逐字匹配**，不用"窗口"：两个段数助手相邻，`[\s\S]{0,300}?` 会跨进下一个函数，
  // 于是"只把第一个改成恒真"这种变异照样通过（这一版之前就是这么假绿的）。
  const cardWall = (s) =>
    /function clipRangeCount\(c\) \{\n  const n = Array\.isArray\(c\?\.meta\?\.ranges\) \? c\.meta\.ranges\.length : 0\n  return n > 1 \? n : 0\n\}/.test(s)
      && /function eventRangeCount\(ev\) \{\n  const n = Array\.isArray\(ev\?\.ranges\) \? ev\.ranges\.length : 0\n  return n > 1 \? n : 0\n\}/.test(s)
      && /v-if="clipRangeCount\(it\.view\.c\)"/.test(s)
      && /v-if="eventRangeCount\(it\.ev\)"/.test(s)
  const polishStrip = (s, f) =>
    /const segs = Array\.isArray\(c\.meta\?\.ranges\) \? c\.meta\.ranges\.length : 0/.test(s)
      && /segments: segs > 1 \? segs : 0/.test(s)
      && /v-if="r\.segments > 1"/.test(f)
  const overview = (s) =>
    /segments: \(Array\.isArray\(meta\.ranges\) \? meta\.ranges\.length : 0\) > 1 \? meta\.ranges\.length : 0/.test(s)
      && /v-if="it\.segments > 1"/.test(s)
  const indexCarries = (s) =>
    /pub ranges: Vec<crate::types::EventRange>,/.test(s)
      && /ranges: if event\.ranges\.len\(\) > 1 \{/.test(s)
      && /if parsed\.len\(\) > 1 \{\n                entry\.ranges = parsed;/.test(s)

  check('㉒ 切片页卡片墙：已剪 + 未剪两种卡片都带标记', cardWall(clipsSection), true)
  check('㉒ 精修页片段条：clipRow 给 segments + 卡片渲染', polishStrip(polishView, fragList), true)
  check('㉒ 首页切片总览：条目带 segments + 卡片渲染', overview(homeOverview), true)
  check('㉒ 索引/片段元数据真的带出 ranges（>1 才带）', indexCarries(clipIndex), true)

  const MUT22 = [
    ['判据放宽成 > 0（单段也会被打成「1 段」）', clipsSection, (s) => s.replace('  return n > 1 ? n : 0', '  return n', 1), cardWall],
    ['切片页少了一处渲染（未剪切事件看不出"将切 N 段"）', clipsSection, (s) => s.replace('v-if="eventRangeCount(it.ev)"', 'v-if="false"'), cardWall],
    ['精修片段条不再给 segments（那边看不出来）', polishView, (s) => s.replace('    segments: segs > 1 ? segs : 0,', ''), (m) => polishStrip(m, fragList)],
    ['片段卡不渲染标记', fragList, (s) => s.replace('v-if="r.segments > 1"', 'v-if="false"'), (m) => polishStrip(polishView, m)],
    ['首页总览不带 segments', homeOverview, (s) => s.replace('        segments: (Array.isArray(meta.ranges) ? meta.ranges.length : 0) > 1 ? meta.ranges.length : 0,', ''), overview],
    ['索引条目丢掉 ranges（卡片无从得知）', clipIndex, (s) => s.replace('    pub ranges: Vec<crate::types::EventRange>,\n', ''), indexCarries],
  ]
  for (const [what, src, mutate, still] of MUT22) {
    const m = mutate(src)
    check(`[变异] ㉒ ${what} → 源码确实被改了`, m !== src, true)
    check(`[变异] ㉒ ${what} → 判据翻面`, still(m), false)
  }
}

// ── ㉓ 字幕轨道的搜索（上一个/下一个）+ 滚轮缩放（用户口径 2026-10-06）──
//
// 用户原话：「预处理界面的字幕轨道增加一个搜索功能，可以进行下一个上一个的进行，方便找位置。
// 在字幕轨道用鼠标滚轮会缩小放大，当前是默认的为最大，这样好操控。」
//
// 两条各有一个**必须钉死**的点：
//   ① 搜索：命中表大小写不敏感、上下一个会循环、当前命中在轨道上真亮；
//   ② 滚轮：`@wheel` **不许是 `.passive`**（passive 监听器里 `preventDefault()` 无效，
//      浏览器照样把页面滚走 ⇒ 缩放与滚动打架），且必须 `preventDefault` + 以鼠标位置为锚
//      + `Shift+滚轮` 让路给滚动。
console.log('\n㉓ 字幕轨道：搜索（上一个/下一个）+ 滚轮缩放')
{
  const track = await read('webui/src/components/prepare/SubtitleTrack.vue')

  const searchUi = (s) =>
    /class="track-search"/.test(s)
      && /placeholder="搜索字幕内容/.test(s)
      && /@click="searchStep\(-1\)"/.test(s)
      && /@click="searchStep\(1\)"/.test(s)
      && /@keydown\.enter\.prevent="searchStep\(\$event\.shiftKey \? -1 : 1\)"/.test(s)
  const hitLogic = (s) =>
    /const hits = computed\(\(\) => \{[\s\S]{0,400}?toLowerCase\(\)/.test(s)
      && /\.toLowerCase\(\)\.includes\(q\)/.test(s)
      && /hit: hitSet\.value\.has\(i\)/.test(s)
      && /hitCur: hitPos\.value >= 0 && hits\.value\[hitPos\.value\] === i/.test(s)
      && /watch\(query, \(\) => \{ hitPos\.value = -1 \}\)/.test(s)
  const wheelZoom = (s) =>
    /@wheel="onWheel"/.test(s)
      && !/@wheel\.passive/.test(s)
      && /function onWheel\(e\) \{[\s\S]{0,700}?e\.preventDefault\(\)/.test(s)
      && /if \(e\.shiftKey\) return/.test(s)
      && /const anchorTime = \(el\.scrollLeft \+ cursorX\) \/ \(scale\.value \|\| 1\)/.test(s)
      && /Math\.max\(0, Math\.min\(scaleList\.value\.length - 1, scaleIdx\.value \+ \(e\.deltaY < 0 \? 1 : -1\)\)\)/.test(s)

  check('㉓ 搜索行在（输入框 + 上一个/下一个 + Enter/Shift+Enter）', searchUi(track), true)
  check('㉓ 命中表大小写不敏感 + 命中块高亮 + 改词清当前位置', hitLogic(track), true)
  check('㉓ 滚轮 = 缩放（非 passive + preventDefault + 鼠标锚点 + Shift 让路）', wheelZoom(track), true)

  const MUT23 = [
    ['滚轮监听改回 `.passive`（preventDefault 失效 ⇒ 缩放和滚动打架）', track,
      (s) => s.replace('@wheel="onWheel"', '@wheel.passive="onWheel"'), wheelZoom],
    ['滚轮不再 preventDefault（页面跟着滚）', track,
      (s) => s.replace('  e.preventDefault()\n', ''), wheelZoom],
    ['Shift+滚轮也让路没了（没法用滚轮滚动了）', track,
      (s) => s.replace('  if (e.shiftKey) return          // 留给浏览器横向滚\n', ''), wheelZoom],
    ['缩放不再以鼠标位置为锚（缩完跳到别处）', track,
      (s) => s.replace('const anchorTime = (el.scrollLeft + cursorX) / (scale.value || 1)', 'const anchorTime = 0'), wheelZoom],
    ['命中表不再忽略大小写（大写查询查不到）', track,
      (s) => s.replace('.toLowerCase().includes(q)', '.includes(q)'), hitLogic],
    ['命中块不再高亮（找不到位置）', track,
      (s) => s.replace('      hit: hitSet.value.has(i),', '      hit: false,'), hitLogic],
    ['「下一个」按钮摘掉（只剩键盘，鼠标党没法用）', track,
      (s) => s.replace('@click="searchStep(1)"', '@click="() => {}"'), searchUi],
  ]
  for (const [what, src, mutate, still] of MUT23) {
    const m = mutate(src)
    check(`[变异] ㉓ ${what} → 源码确实被改了`, m !== src, true)
    check(`[变异] ㉓ ${what} → 判据翻面`, still(m), false)
  }
}

// ── ㉔ 多段切片的**选段**：入点/出点只作用于选中的那段（用户口径 2026-10-06）──
//
// 用户原话：「预处理的多段切片没法回过头来处理，增加一个选择段，然后入点出点就指影响那个段，
// 默认新建段时自动转到最新段」。
//
// 实测过的病根：行里两个输入框挂了 `@click.stop` ⇒ **想改数字的那位用户怎么点都换不了段**，
// 接着按「标记入点」就落到别人身上（真页面探针修之前 ②③ 是红的）。四条一起钉：
//   ① 整行可点选（输入框也认，`@focus` 也算）；
//   ② 有个显式的选段按钮（`.seg-pick`）；
//   ③ 加段自动转到最新段；
//   ④ 界面上写得出来"现在编辑的是第几段"。
console.log('\n㉔ 多段切片：选段（入点/出点只作用于选中那段）')
{
  const prepareView = await read('webui/src/views/PrepareView.vue')

  const rowSelectable = (s) =>
    /class="slice-row" :class="\{ on: i === activeRow \}"[\s\S]{0,200}?@click="activeRow = i"/.test(s)
      && !/v-model\.number="r\.start"\s*\n\s*:title="'第 '[\s\S]{0,120}?@click\.stop/.test(s)
      && /@focus="activeRow = i"/.test(s)
  const pickButton = (s) =>
    /class="seg-pick"[\s\S]{0,400}?@click\.stop="activeRow = i"/.test(s)
      && /\.seg-pick\.on \{/.test(s)
      && /\.slice-row\.on \{ background:var\(--chip-bg\); border-left-color:/.test(s)
  const autoNewest = (s) =>
    /function addSliceRow\(\) \{[\s\S]{0,400}?activeRow\.value = sliceRows\.value\.length - 1/.test(s)
  const showsWhich = (s) =>
    /function markHint\(which\)/.test(s)
      && /编辑中 <b class="seg-cur-num">\{\{ activeRow \+ 1 \}\}<\/b>/.test(s)
      && /const row = sliceRows\.value\[activeRow\.value\] \|\| sliceRows\.value\[0\]/.test(s)

  check('㉔ 整行/输入框都能换段（输入框不许 `@click.stop`）', rowSelectable(prepareView), true)
  check('㉔ 有显式选段按钮 + 选中态一眼看得见（左侧色条）', pickButton(prepareView), true)
  check('㉔ 新建段自动转到最新段', autoNewest(prepareView), true)
  check('㉔ 界面写得出"编辑中第 N 段"、标记按钮 title 说出落点', showsWhich(prepareView), true)

  const MUT24 = [
    ['输入框又挂回 `@click.stop`（改数字后换不了段 —— 就是这个 bug）', prepareView,
      (s) => s.replace(":title=\"'第 ' + (i + 1) + ' 段入点（秒）'\" @focus=\"activeRow = i\"",
        ":title=\"'第 ' + (i + 1) + ' 段入点（秒）'\" @click.stop"), rowSelectable],
    ['去掉输入框的 `@focus` 选段', prepareView,
      (s) => s.replaceAll(' @focus="activeRow = i"', ''), rowSelectable],
    ['整行不再可点选', prepareView,
      (s) => s.replace('                 @click="activeRow = i">', '                 >'), rowSelectable],
    ['选段按钮摘掉', prepareView,
      (s) => s.replace(/<button class="seg-pick"[\s\S]{0,600}?<\/button>/, ''), pickButton],
    ['新建段不再自动转到最新段', prepareView,
      (s) => s.replace('  activeRow.value = sliceRows.value.length - 1\n', ''), autoNewest],
    ['界面上不再写"编辑中第 N 段"', prepareView,
      (s) => s.replace(/<template v-if="sliceRows\.length > 1">[\s\S]{0,120}?<\/template>/, ''), showsWhich],
  ]
  for (const [what, src, mutate, still] of MUT24) {
    const m = mutate(src)
    check(`[变异] ㉔ ${what} → 源码确实被改了`, m !== src, true)
    check(`[变异] ㉔ ${what} → 判据翻面`, still(m), false)
  }
}

// ── ㉕ 多段切片：**每段都上色** + 缩放底线 = **全长度**（用户口径 2026-10-06）──
//
// 两条：
//   ① 「多段时需要将每一段都颜色标注，而不是只有最新的才标注」⇒ 轨道上 N 段 N 条色带，
//      每段自己的颜色；列表里的段号用**同一套调色板**（一眼对得上"第 3 段在哪儿"）；
//   ② 「最大还是定为全长度吧，还是很有必要」⇒ 缩小到底那一档是**动态算的**
//      （`可用宽度 / 总时长`），固定 `0.5 px/s` 对 3 小时录音要 5400px，根本放不下一屏。
console.log('\n㉕ 多段：每段上色 + 缩到底 = 全长度')
{
  const track = await read('webui/src/components/prepare/SubtitleTrack.vue')
  const utils = await read('webui/src/utils.js')
  const prepareView = await read('webui/src/views/PrepareView.vue')

  const palette = (s) =>
    /export const SEG_COLORS = \[/.test(s) && /export function segColor\(i\)/.test(s)
      && (s.match(/line: 'rgba\(/g) || []).length >= 6
  // ⚠️ **逐字**判"每一段都画"：光查"提到 props.ranges"会被
  // `? [props.ranges[activeRange]]` 这种"只画当前那段"的变异骗过（实测踩过）。
  const everyBand = (s) =>
    /ranges: \{ type: Array, default: \(\) => \[\] \}/.test(s)
      && /activeRange: \{ type: Number, default: 0 \}/.test(s)
      && /const list = \(props\.ranges \|\| \[\]\)\.length\n\s*\? props\.ranges\n\s*: \[\{ start: props\.markIn, end: props\.markOut \}\]/.test(s)
      && /const c = segColor\(i\)/.test(s)
      && /on: i === \(Number\(props\.activeRange\) \|\| 0\),/.test(s)
      && /v-for="b in bands"/.test(s)
  const dynamicFloor = (s) =>
    /const fitScale = computed\(\(\) => \{[\s\S]{0,220}?Math\.min\(SCALES\[0\], viewWidth\(\) \/ s\)/.test(s)
      && /const scaleList = computed\(\(\) => \[fitScale\.value, \.\.\.SCALES\.filter\(\(v\) => v > fitScale\.value\)\]/.test(s)
      && /const scale = computed\(\(\) => scaleList\.value\[/.test(s)
  const wired = (s) =>
    /:ranges="sliceRows"/.test(s) && /:active-range="activeRow"/.test(s)
      && /:style="\{ borderColor: segColor\(i\)\.line/.test(s)

  check('㉕ 共享调色板（≥6 色 + `segColor(i)`）', palette(utils), true)
  check('㉕ 轨道：每段一条色带（不是只标当前那段）', everyBand(track), true)
  check('㉕ 缩到底那一档是动态算的（可用宽度 / 总时长）', dynamicFloor(track), true)
  check('㉕ 列表把整份 ranges 传下去 + 段号用同一套色', wired(prepareView), true)

  const MUT25 = [
    ['轨道只画当前那段（回到用户反对的行为）', track,
      (s) => s.replace('    ? props.ranges\n', '    ? [props.ranges[Number(props.activeRange) || 0] || { start: 0, end: 0 }]\n'), everyBand],
    ['每段用同一个颜色（分不出第几段）', track,
      (s) => s.replace('    const c = segColor(i)', '    const c = segColor(0)'), everyBand],
    ['缩到底那一档写回固定 0.5（3 小时放不下）', track,
      (s) => s.replace('  return Math.min(SCALES[0], viewWidth() / s)', '  return SCALES[0]'), dynamicFloor],
    ['列表不再把 ranges 传给轨道', prepareView,
      (s) => s.replace('            :ranges="sliceRows"\n', ''), wired],
    ['列表段号不再上色（与色带对不上）', prepareView,
      (s) => s.replace(':style="{ borderColor: segColor(i).line, color: segColor(i).line,', ':style="{'), wired],
    ['调色板缩到 2 色', utils,
      (s) => s.replace(/export const SEG_COLORS = \[[\s\S]*?\n\]\n/, 'export const SEG_COLORS = [\n  { line: \'rgba(1,1,1,.9)\', fill: \'rgba(1,1,1,.2)\' },\n]\n'), palette],
  ]
  for (const [what, src, mutate, still] of MUT25) {
    const m = mutate(src)
    check(`[变异] ㉕ ${what} → 源码确实被改了`, m !== src, true)
    check(`[变异] ㉕ ${what} → 判据翻面`, still(m), false)
  }
}

// ── ㉖ 播放区时间读数必须**同时给秒数**（用户口径 2026-10-06）──
//
// 用户原话：「播放（空格）09:36.5 需要显示秒数，方便定位」——时钟负责"读数"，
// 秒数负责"填数"（切片列表里入点/出点两个框要的就是秒）。
console.log('\n㉖ 播放区读数：时钟 + 总秒数')
{
  const prepareView = await read('webui/src/views/PrepareView.vue')
  const hasSecs = (s) =>
    /class="muted small mono cur-secs"[\s\S]{0,160}?\{\{ fmtSecs\(currentTime\) \}\}/.test(s)
      && /function fmtSecs\(t\) \{[\s\S]{0,200}?toFixed\(1\) \+ 's'/.test(s)
  check('㉖ 播放区同时显示时钟与总秒数', hasSecs(prepareView), true
    , '只有 09:36.5 的话，填进点/出点时还得自己心算')
  const MUT26 = [
    ['秒数读数摘掉', prepareView,
      (s) => s.replace(/\s*<span class="muted small mono cur-secs"[^>]*>[^<]*<\/span>/, ''), hasSecs],
    ['秒数不带单位/精度（填不进那两个框）', prepareView,
      (s) => s.replace("return (Math.max(0, Number(t) || 0)).toFixed(1) + 's'", 'return Math.max(0, Number(t) || 0)'), hasSecs],
  ]
  for (const [what, src, mutate, still] of MUT26) {
    const m = mutate(src)
    check(`[变异] ㉖ ${what} → 源码确实被改了`, m !== src, true)
    check(`[变异] ㉖ ${what} → 判据翻面`, still(m), false)
  }
}

// ── ㉗ 「LLM 剪辑」在**右栏「画面构成 / 输出」**面板里（用户口径 2026-10-06）──
//
// 用户原话：「llm剪辑放在右侧画面构成输出那个面板上」。
// 位置本身是需求的一部分，所以判据钉三件：① 轨道设置面板里**没有**它；
// ② 右栏面板里**有**；③ 排在「渲染导出」**上面**（先重剪、再出片，与工序顺序一致）。
console.log('\n㉗ LLM 剪辑在右栏「画面构成 / 输出」面板')
{
  const polishView = await read('webui/src/views/PolishView.vue')
  const lanePanel = (s) => {
    const i = s.indexOf('class="lane-panel"')
    const j = s.indexOf('项目设置 / 画面', i)
    return i < 0 || j < 0 ? '' : s.slice(i, j)
  }
  // ⚠️ 锚点 = 「项目设置」页签那一半（画面尺寸那半里没有 LLM）。
  const rightPanel = (s) => {
    const i = s.indexOf("<template v-if=\"sideTab === 'project'\">")
    const j = s.indexOf('<template v-else>', i)
    return i < 0 || j < 0 ? '' : s.slice(i, j)
  }
  const inRight = (s) => /@click="runLlmJob"/.test(rightPanel(s))
  const notInLane = (s) => !/runLlmJob/.test(lanePanel(s))
  const beforeRender = (s) => {
    const r = rightPanel(s)
    const a = r.indexOf('runLlmJob')
    const b = r.indexOf('openRenderModal')
    return a >= 0 && b > a
  }
  check('㉗ 轨道设置面板里没有 LLM 剪辑', notInLane(polishView), true)
  check('㉗ 右栏「项目设置」页签里有它（接 `runLlmJob`）', inRight(polishView), true)
  check('㉗ 且排在「渲染导出」上面（先重剪、再出片）', beforeRender(polishView), true)

  const MUT27 = [
    ['右栏那个按钮不再接 runLlmJob（点不动）', polishView,
      (s) => s.replace('@click="runLlmJob"', '@click="() => {}"'), inRight],
    // 变异：把 LLM 剪辑从「项目设置」页签里挪到**画面尺寸**那半（切错页才看得到）
    // 变异：把 LLM 那半整个挂到「画面尺寸」页签上（切错页才看得到）—— 直接把 sideTab 判据对调
    ['把 LLM 那半挂到「画面尺寸」页签上', polishView,
      (s) => s
        .replace("<template v-if=\"sideTab === 'project'\">", "<template v-if=\"sideTab === 'canvas'\">")
        .replace('<template v-else>', "<template v-else-if=\"sideTab === 'project'\">"),
      (m) => inRight(m)],
  ]
  for (const [what, src, mutate, still] of MUT27) {
    const m = mutate(src)
    check(`[变异] ㉗ ${what} → 源码确实被改了`, m !== src, true)
    check(`[变异] ㉗ ${what} → 判据翻面`, still(m), false)
  }
}

// ── ㉘ 复合片段（多段切片）的**各段源区间**必须看得见（用户口径 2026-10-06）──
//
// 背景：`clip.json` 的 `ranges` 记着"这条片段由源片的哪几段拼出来"，此前界面只显示「N 段」——
// **哪几段**只有打开 `clip.json` 才知道。用户没法判断"这次合并合不合理"：项目 12 那条 4 段片段
// 把 07:21 / 15:29 / 27:05 / 27:12 拼成一条（第一条与第二条隔了 7 分 22 秒），
// 界面上只写「4 段」，看不出跨度。
//
// 判据钉四件事：
//   ① 区间文案**只有一个出口**（`utils.js` 的 `compoundRanges` / `rangesText` / `rangesHint` /
//      `rangesGapSecs` / `fmtSpan` / `segMark`）—— 四处各写一份的话，改一处漂三处；
//   ② 切片页**两种**卡片（已剪 / 未剪切事件）都有可见的 `.range-line` 区间行（`v-for="it.ranges"`）；
//   ③ 精修片段条与首页总览（卡片太小）至少把区间放进悬停 title，且走同一个出口；
//   ④ 不再有各处手拼的区间文案 —— 尤其**不许**再用 `**` 当强调（用户可见文案里会原样显示成星号）。
console.log('\n㉘ 复合片段的「各段源区间」（可见 + 单一出口）')
{
  const utils = await read('webui/src/utils.js')
  const clipsSection = await read('webui/src/components/ProjectClipsSection.vue')
  const fragList = await read('webui/src/components/polish/FragmentList.vue')
  const homeOverview = await read('webui/src/components/HomeClipsOverview.vue')

  // ① 单一出口：每个函数都按名字 + 关键行钉住（不用 `[\s\S]{0,n}` 窗口 —— 那会跨进相邻函数，
  //    把某个函数改坏照样通过）
  const formatter = (s) =>
    s.includes('export function compoundRanges(src) {')
    && s.includes('  return out.length > 1 ? out : []')
    && s.includes('export function rangesGapSecs(src) {')
    && s.includes('export function rangesText(src) {')
    && s.includes('export function rangesHint(src) {')
    && s.includes('export function fmtSpan(sec) {')
    && s.includes('export function segMark(i) {')

  // ② 两种卡片各有一行，且渲染的是 `it.ranges`（不是只把段数塞进 title）
  const cardRow = (s) =>
    (s.match(/class="range-line"/g) || []).length === 2
    && s.includes('ranges: compoundRanges(meta)')
    && s.includes('ranges: compoundRanges(ev)')
    && s.includes('gap: rangesGapSecs(meta)')
    && s.includes('v-for="(r, i) in it.ranges"')
    && s.includes('{{ fmtSpan(r.start) }}–{{ fmtSpan(r.end) }}')
    && s.includes('{{ segMark(i) }}')

  // ③ 精修片段条（1 处）+ 首页总览（两栏各 1 处）都调同一个出口
  // ⚠️ 2026-10-06 起是 **1 处**（总览合并成一面卡片墙了；原先两栏各一处 = 2）。
  //    改回多栏时这个数要跟着改，否则会误报"少了一处"。
  const titles = (frag, ov) =>
    frag.includes(':title="rangesHint(r.ranges)"')
    && (ov.match(/:title="rangesHint\(it\.ranges\)"/g) || []).length === 1

  // ④ 手拼文案（含 `**` 强调）已经清掉：`**` 在 title/卡片上会原样显示成星号
  const noHandRolled = (...srcs) =>
    srcs.every((s) => !s.includes("' 段源区间**分段剪、合并输出**"))
      && srcs.every((s) => !/\*\*分段剪/.test(s))

  check('㉘ 区间文案只有一个出口（utils 的六个函数齐备）', formatter(utils), true)
  check('㉘ 切片页两种卡片都有可见区间行（各段起–止 + 段号）', cardRow(clipsSection), true)
  check('㉘ 精修片段条与首页总览的 title 走同一个出口', titles(fragList, homeOverview), true)
  check('㉘ 不再有手拼的区间文案（`**` 那种会原样显示的强调）',
    noHandRolled(clipsSection, fragList, homeOverview), true)
  // 「丢掉多少空隙」的外沿必须取 max(end)：段**可以互相重叠**（项目 12 那条就是
  // 1625.7–1653.8 包着 1632–1643），写成"排序后最后一段的 end"会少算 11s 且不报错 ——
  // 本轮就这么错过一次，是手算才发现的（探针里那份独立实现当时抄了同一条公式，两边一起错）。
  const gapSpan = (s) =>
    s.includes('const span = Math.max(...rs.map((r) => r.end)) - Math.min(...rs.map((r) => r.start))')
  check('㉘ 空隙外沿取 max(end) − min(start)（重叠段不许少算）', gapSpan(utils), true)

  const MUT28 = [
    ['区间判据放宽（单段也当复合片段渲染）', utils,
      (s) => s.replace('  return out.length > 1 ? out : []', '  return out'), formatter],
    ['`rangesText` 被删（行内串没人拼得出来）', utils,
      (s) => s.replace('export function rangesText(src) {', 'function rangesTextGone(src) {'), formatter],
    ['切片页已剪卡片那行不渲染区间', clipsSection,
      (s) => s.replace('class="range-line"', 'class="range-line-gone"'), cardRow],
    ['未剪切事件卡片不再给 `ranges`（看不出"将切哪几段"）', clipsSection,
      (s) => s.replace('ranges: compoundRanges(ev),', ''), cardRow],
    ['首页总览退回只给段数（区间又看不见了）', homeOverview,
      (s) => s.replace(':title="rangesHint(it.ranges)"', ':title="it.segments + \' 段\'"'), (m) => titles(fragList, m)],
    ['精修片段条退回手拼文案', fragList,
      (s) => s.replace(':title="rangesHint(r.ranges)"',
        `:title="'多段片段：由 ' + r.segments + ' 段源区间**分段剪、合并输出**'"`),
      (m) => noHandRolled(clipsSection, m, homeOverview)],
    ['空隙外沿退回"最后一段的 end"（重叠段少算）', utils,
      (s) => s.replace('const span = Math.max(...rs.map((r) => r.end)) - Math.min(...rs.map((r) => r.start))',
        'const span = rs[rs.length - 1].end - rs[0].start'), gapSpan]
  ]
  for (const [what, src, mutate, still] of MUT28) {
    const m = mutate(src)
    check(`[变异] ㉘ ${what} → 源码确实被改了`, m !== src, true)
    check(`[变异] ㉘ ${what} → 判据翻面`, still(m), false)
  }
}

// ── ㉙ 重叠的段必须**并成一段**（不然同一段内容会被剪两遍）──
//
// 现场（项目 12 手工标的那条 4 段片段）：`1625.7–1653.8` 里套着 `1632.0–1643.0` ——
// Cutter 按 `cut_ranges()` 逐段剪再拼，`clip.mp4` = 158.3s = **四段之和**（其中 11s 是重复的），
// 而 `duration` / 卡片上的「内容时长」也按各段之和多报一遍 —— 两处一起错，界面上看不出来。
// 手工标段时两行本来就可以互相压住，所以这条要**三处**都钉住：
//   ① core 的 `merge_overlapping_ranges`（唯一一份并段逻辑，`normalize_ranges` 也走它）；
//   ② 写侧 `create-event` 真的调它（不然手工那条路照旧落盘重叠）；
//   ③ 预处理页**先告诉用户**（`.seg-overlap` 警告 + 总时长按并集算，别 158s/147s 两边对不上）。
console.log('\n㉙ 重叠的段并成一段（核心 + 写侧 + 预处理页提示）')
{
  const coreTypes = await read('crates/yekitrim-core/src/types/mod.rs')
  const projectData = await read('crates/yekitrim-server/src/api/project_data')
  const prepareView = await read('webui/src/views/PrepareView.vue')

  // ⚠️ 判据只认 `next.start <= last.end`：放宽成"有内容的空隙也并"会把
  //    "分段剪、中间别的话题丢掉"的表达整条毁掉（那是 `merge_group` 的语义）。
  const coreMerge = (s) =>
    s.includes('pub fn merge_overlapping_ranges(ranges: &mut Vec<EventRange>) {')
    && s.includes('        merge_overlapping_ranges(&mut self.ranges);')
    && s.includes('Some(last) if r.start <= last.end + 1e-6 => {')
  const writeSide = (s) =>
    s.includes('yekitrim_core::types::merge_overlapping_ranges(&mut ranges);')
  const prepareUi = (s) =>
    s.includes('const sliceOverlaps = computed(() => {')
    && s.includes('if (rs[k].start <= rs[k - 1].end + 1e-6) out.push(rs[k].i + 1)')
    && s.includes('v-if="sliceOverlaps.length"')
    && s.includes('class="seg-overlap"')
    && s.includes('if (r.start <= lastEnd) {') // sliceTotal 也按并集求和

  check('㉙ core 只有一份并段逻辑（`normalize_ranges` 走它，只并相接/重叠）', coreMerge(coreTypes), true)
  check('㉙ 写侧 create-event 真的调了它（手工那条路不再落盘重叠）', writeSide(projectData), true)
  check('㉙ 预处理页会提示重叠 + 总时长按并集算', prepareUi(prepareView), true)

  const MUT29 = [
    ['并段判据放宽（把"有内容的空隙"也并了 —— 分段剪的语义没了）', coreTypes,
      (s) => s.replace('Some(last) if r.start <= last.end + 1e-6 => {',
        'Some(last) if r.start <= last.end + 60.0 => {'), coreMerge],
    ['`normalize_ranges` 不再走并段（分散合并/索引那条路又漏了）', coreTypes,
      (s) => s.replace('        merge_overlapping_ranges(&mut self.ranges);', ''), coreMerge],
    ['写侧不再并（手工标的重叠照旧剪两遍）', projectData,
      (s) => s.replace('    yekitrim_core::types::merge_overlapping_ranges(&mut ranges);', ''), writeSide],
    ['预处理页不再提示重叠', prepareView,
      (s) => s.replace('v-if="sliceOverlaps.length"', 'v-if="false"'), prepareUi],
    ['总时长退回"各段直接相加"（与剪出来的长度对不上）', prepareView,
      (s) => s.replace('    if (r.start <= lastEnd) {', '    if (false) {'), prepareUi]
  ]
  for (const [what, src, mutate, still] of MUT29) {
    const m = mutate(src)
    check(`[变异] ㉙ ${what} → 源码确实被改了`, m !== src, true)
    check(`[变异] ㉙ ${what} → 判据翻面`, still(m), false)
  }
}

// ── ㉚ 首页切片总览的搜索：**按需拉取** + 弹幕正文要摘 ASS 覆写标签 ──
//
// 用户口径（2026-10-06）：「需要一个功能，在切片总览上显示搜索框，可以关键词查找，
// 关键词可以锁到标题，字幕，弹幕。」
//
// 两条都对应真踩过的坑：
//  ① **按需**：标题在缓存里（零请求），字幕/弹幕在项目文件里（实测每项目 170~226 KB 转写 +
//     4~72 KB 弹幕）。进首页就全量拉 = 凭空搬 2~3 MB / 24 个请求。所以判据钉的是
//     "**勾了才拉**"：`ensureText` 必须在勾选回调与 data 装载之后才被调用，
//     并且**带 `wantSub`/`wantDm` 门**（否则勾一个会把另一桶也拉了）。
//  ② **摘要标签**：弹幕每一条都带 `{\move(1970,0,-50,0)}` / `{\pos(960,1040)}` 这类 ASS 覆写标签
//     （解析层有意保留，渲染要按它摆位）。不摘掉：卡片上显示标签串，且搜 `move`/`pos`
//     **命中每一条弹幕**。摘标签只能在 `utils.js::dmPlain` 一处（别在两处各写一份正则）。
console.log('\n㉚ 切片总览搜索（按需拉取 + 弹幕正文可读化）')
{
  const utils = await read('webui/src/utils.js')
  const ov = await read('webui/src/components/HomeClipsOverview.vue')

  // ① dmPlain：只此一份，且两条替换都在（`{...}` 覆写块 + `\N` 转义）
  const dmPlain = (s) =>
    /export function dmPlain\(s\)/.test(s)
    && /\.replace\(\/\\\{\[\^\}\]\*\\\}\/g, ''\)/.test(s)
    && /replace\(\/\\\\\[Nnh\]\/g/.test(s)
  // ② 总览里必须**真的用了**它（定义了但没接上 = 白搭）
  const usesDmPlain = (s) => /import \{[^}]*dmPlain[^}]*\} from '\.\.\/utils'/.test(s) && /\.map\(dmPlain\)/.test(s)
  // ③ 按需：只在 searchIn 变化 / 装载后调 ensureText，且带 want 门
  const onDemand = (s) =>
    /const wantSub = searchIn\.subtitle && b\.subs === null/.test(s)
    && /const wantDm = searchIn\.danmaku && b\.dm === null/.test(s)
    && /if \(\(!wantSub && !wantDm\) \|\| fetching\[pid\]\) continue/.test(s)
  // ④ 两桶一个接口（别退回两个接口 = 24 个请求）
  const oneEndpoint = (s) => /projectData\.searchText\(/.test(s) && !/projectData\.danmakuText\(/.test(s)

  check('㉚ `dmPlain` 只此一份（ASS 覆写块 + \\N 转义都摘）', dmPlain(utils), true)
  check('㉚ 总览真的接了 `dmPlain`（定义了不接等于白搭）', usesDmPlain(ov), true)
  check('㉚ 取字幕/弹幕走**按需 + 分桶门**（勾一个不连带拉另一个）', onDemand(ov), true)
  check('㉚ 两桶一个接口（`searchText`，不回退成两个接口）', oneEndpoint(ov), true)

  const MUT30 = [
    ['弹幕正文不摘 ASS 标签（卡片显示标签串、搜 move 命中全部）', ov,
      (s) => s.replace('.map(dmPlain)', '.map((x) => x)'), usesDmPlain],
    ['`dmPlain` 不摘覆写块（只处理 \\N）', utils,
      (s) => s.replace(".replace(/\\{[^}]*\\}/g, '')", '.replace(/(?:)/g, "")'), dmPlain],
    ['按需门退化成"勾了就全拉"（勾字幕连弹幕一起拉）', ov,
      (s) => s.replace('const wantDm = searchIn.danmaku && b.dm === null', 'const wantDm = true'), onDemand],
    ['退回两个接口（每项目两发 = 24 个请求）', ov,
      (s) => s.replace('projectData.searchText(', 'projectData.danmakuText('), oneEndpoint]
  ]
  for (const [what, src, mutate, still] of MUT30) {
    const m = mutate(src)
    check(`[变异] ㉚ ${what} → 源码确实被改了`, m !== src, true)
    check(`[变异] ㉚ ${what} → 判据翻面`, still(m), false)
  }
}

// ---------------------------------------------------------------------------
// ㉛ **控件契约**（2026-10-15，P2）—— "加字段 = 改词汇表一行"这句话的判据
//
// 背景：`autoFields` 已经能从 `spec/effects.json` 的 `fields[]` 派生控件（P1，2026-10）。
// 但那条路**没有门禁**，于是它随时可能退化成"面板里手写控件 + 词汇表字段没人接"——
// 而那正是它被做出来要解决的那个问题（"加了字段但面板没有控件"，零报错）。
//
// 五条硬规矩，**每一条都对应一种静默失败**：
//  ① 字段只许来自词汇表（面板手写 `<input>` 绑事件字段 = 绕过 `fields[]`）
//  ② 专用块漏掉的字段必须自动补（兜底块被删 ⇒ 那一整类字段消失）
//  ③ 排除清单不许吃掉词汇表里的字段（两控件写同一字段 = 写回时后者赢）
//  ④ 每个 `ui` 类型必须真的有控件（认不出 ⇒ 静默退化成文本框）
//  ⑤ 空串写 `undefined`（写 `''` ⇒ serde 读成 `Some("")` ⇒ 解析报"无法识别"）
// ---------------------------------------------------------------------------
{
  console.log('\n㉛ 控件契约（加字段 = 改词汇表一行；每条带变异自证）')

  const spec = JSON.parse(await read('spec/effects.json'))
  const vocabFields = new Set()
  for (const e of spec.effects) for (const f of (e.fields || [])) vocabFields.add(f.name)

  const autoFieldsCalls = (timelineEditor.match(/autoFields\(/g) || []).length
  check('㉛ 属性面板有 >=4 处 autoFields（专用块 + 兜底），字段由词汇表派生', autoFieldsCalls >= 4, true)

  const hasCatchAll = /autoFields\(selItem\.type\)/.test(timelineEditor)
  check('㉛ 兜底块 autoFields(selItem.type) 存在（专用块没覆盖的字段靠它露出）', hasCatchAll, true)

  const dedMatch = timelineEditor.match(/const DEDICATED_FIELDS = new Set\(\[([\s\S]*?)\]\)/)
  const ded = dedMatch ? [...dedMatch[1].matchAll(/'([a-z_0-9]+)'/g)].map((m) => m[1]) : []
  // 「结构位」字段：它们**本来就该由专用块画**（不是被排除清单顺手吃掉的普通标量）。
  // ⚠️ 新增这类字段时**只加这一处**（同时加 `TimelineEditor` 的 DEDICATED_FIELDS）——
  //    `manual_pos`（2026-10-20 起在 spec 里声明）与 `x`/`y` 同一族：逐块落点，面板有专门那个「落点」块。
  const STRUCTURAL = new Set(['kind', 'text', 'sticker_tag', 'keyword', 'name', 'source', 'x', 'y', 'manual_pos', 'color', 'sfx', 'sfx_volume', 'intensity', 'manual', 'end', 'volume'])
  const overreach = ded.filter((f) => vocabFields.has(f) && !STRUCTURAL.has(f))
  check('㉛ DEDICATED_FIELDS 没有吃掉词汇表里的标量字段', overreach, [])

  // ⚠️ `ui` 有两个消费者，缺一不可：① 专用块（`DEDICATED_FIELDS`，如 color/intensity/volume）
  //    ② `autoFields` 的 `controlKind`（switch/select/number/text）。
  //    判据是"**至少被一个认领**"——只查 controlKind 会把专用块那批误判成缺陷。
  const uiValues = new Map()
  for (const e of spec.effects) for (const f of (e.fields || [])) if (f.ui) uiValues.set(f.ui, f.name)
  const knownUi = new Set(['switch', 'select', 'number', 'text'])
  // 专用块认领的 ui（它们由各专用块自己渲染，不走 controlKind）
  const dedicatedUi = new Set(['color', 'color_list', 'slider', 'intensity'])
  const unknownUi = [...uiValues.keys()].filter((u) => !knownUi.has(u) && !dedicatedUi.has(u))
  check('㉛ 词汇表每个 ui 都被某个消费者认领（专用块或 controlKind）', unknownUi, [])

  const nullishWrite = /props\.ev\[props\.f\.name\] = v === '' \? undefined : v/.test(timelineEditor)
  check('㉛ FieldControl 空串写 undefined（不写空串 —— serde 会读成 Some(空串)）', nullishWrite, true)

  const MUT31 = [
    ['专用块不再派生（字段名换成不存在的函数）', timelineEditor,
      (s) => s.replace(/autoFields\(/g, 'legacyFields('),
      (s) => (s.match(/autoFields\(/g) || []).length >= 4],
    // ⚠️ 这里原先变异的是 `autoFields(selItem.type)` 的**调用形态**；现在兜底块多了
    //    `hasDedicatedBranch` 那道门，把"新字段还能自动冒出来吗"表达成**门被拆掉**更准 ——
    //    门一拆，有专用支的类型就会重复渲染（症状是重复，不是"新字段不出现"）。
    ['兜底块的门被拆掉（有专用支的类型会每个字段出两遍）', timelineEditor,
      (s) => s.replace('v-if="!hasDedicatedBranch(selItem.type)"', 'v-if="true"'),
      (s) => /v-if="!hasDedicatedBranch\(selItem\.type\)"/.test(s)],
    ['DEDICATED_FIELDS 吃掉标量字段 opacity', timelineEditor,
      (s) => s.replace("'color', 'intensity'", "'color', 'intensity', 'opacity'"),
      (s) => {
        const m = s.match(/const DEDICATED_FIELDS = new Set\(\[([\s\S]*?)\]\)/)
        const d = m ? [...m[1].matchAll(/'([a-z_0-9]+)'/g)].map((x) => x[1]) : []
        return d.filter((f) => vocabFields.has(f) && !STRUCTURAL.has(f)).length === 0
      }],
    ['空串写回退化成空串（serde 读成 Some(空串)）', timelineEditor,
      (s) => s.replace("v === '' ? undefined : v", 'v'),
      (s) => /props\.ev\[props\.f\.name\] = v === '' \? undefined : v/.test(s)]
  ]
  for (const [what, src, mutate, still] of MUT31) {
    const m = mutate(src)
    check(`[变异] ㉛ ${what} → 源码确实被改了`, m !== src, true)
    check(`[变异] ㉛ ${what} → 判据翻面`, still(m), false)
  }
}

// ㉜ **一个字段只许出一个控件**（2026-10-15，用户报「一长串重复」）
//
// 背景：`element` 支既有**自己的** skip 表派生的控件（专用块那几行），
// 又会被**兜底块** `autoFields(selItem.type)` 再派生一遍 —— 于是
// `style / anim / style_anim / effect_anim / transition_anim / layout_angle_deg / pos`
// **每个都出两遍**，面板上看就是"一长串重复的属性"，而且**零报错**。
//
// 根因是兜底块的**存在理由**没被写下来：它服务的是"这个类型**没有**专用支"，
// 而 `element` **有**。⇒ 判据 = 兜底块必须显式跳过 element。
//
// ⚠️ 这条一旦退化，用户看到的是"每个属性都有两遍"，而构建、两条门禁、九条前端判据**全绿**。
// ---------------------------------------------------------------------------
{
  console.log('\n㉜ 属性面板不重复渲染（每条带变异自证）')

  // ⚠️ 兜底块必须**按"有没有专用支"门控**（2026-10-15 二次修）。
  //    第一版这里钉的是字面量 `!== 'element'` —— 那条判据**只保住了 element**，
  //    而 `camera` / `overlay` 照旧重复（用户贴图就是那次）。
  //    现在钉的是**机制**：`hasDedicatedBranch` + 那张表（表与模板的差集由 ㉞ 算）。
  const catchAllGuarded = /v-if="!hasDedicatedBranch\(selItem\.type\)"/.test(timelineEditor)
  check('㉜ 兜底块按 hasDedicatedBranch 门控（有专用支的一律跳过）', catchAllGuarded, true)

  // 真·重复检测：把两处 skip 表取出来求交集，交集必须为空。
  // ⚠️ 判据是**算出来的**，不是"某个字符串还在不在"—— 改 skip 表也能被抓到。
  // skip 表现在是具名常量 `ELEMENT_DEDICATED`（漏字段就是出两遍，所以它得能被机器读）。
  const elSkipMatch = timelineEditor.match(/const ELEMENT_DEDICATED = \[([\s\S]*?)\n\]/)
  const elSkip = elSkipMatch ? new Set([...elSkipMatch[1].matchAll(/'([a-z_0-9]+)'/g)].map((m) => m[1])) : null
  const dedMatch2 = timelineEditor.match(/const DEDICATED_FIELDS = new Set\(\[([\s\S]*?)\]\)/)
  const ded2 = dedMatch2 ? new Set([...dedMatch2[1].matchAll(/'([a-z_0-9]+)'/g)].map((m) => m[1])) : null
  check('㉜ 能取到 element 的 skip 表与 DEDICATED_FIELDS', !!(elSkip && ded2), true)
  // ⚠️ 常量必须**真的被用**：写了个常量却还在模板里内联旧数组 = 声明与实现分叉。
  check('㉜ element 支用 ELEMENT_DEDICATED（不是内联数组）',
    /autoFields\(selItem\.type, ELEMENT_DEDICATED\)/.test(timelineEditor), true)
  // ⚠️⚠️ **专用块真正渲染过的那四个必须都在表里** —— 这是本轮漏掉的那四个，
  //     逐个点名钉住：漏任一个，面板上就是"那个属性出两遍"。
  for (const must of ['style_anim', 'transition_anim', 'effect_anim', 'layout_angle_deg']) {
    check(`㉜ 专用块渲染的 ${must} 已在 skip 表里（否则出两遍）`, elSkip.has(must), true)
  }

  // element 的**词表字段**里，既没被专用块吃掉、也没被 element skip 点名的 = 真正会渲染的那些。
  const specEl = JSON.parse(await read('spec/effects.json')).effects.find((e) => e.id === 'element')
  const elFields = (specEl?.fields || []).map((f) => f.name)
  // 上一支实际渲染的 = 词表字段 − DEDICATED − elementSkip。
  const renderedByFirst = elFields.filter((f) => !ded2.has(f) && !elSkip.has(f))
  // 兜底块**只有当它没跳过 element 时**才会再渲染一遍 —— 判据必须**跟着那道门走**，
  // 否则它会一直报"重复"（因为算术上两块本来就覆盖同一批字段，是那道门在挡）。
  const catchAllAppliesToElement = !catchAllGuarded
  const renderedByCatchAll = catchAllAppliesToElement ? elFields.filter((f) => !ded2.has(f)) : []
  const dup = renderedByFirst.filter((f) => renderedByCatchAll.includes(f))
  check('㉜ element 字段在两支里的重复集合为空（算出来的，不是抄的）', dup, [])
  // 反向自证：把门去掉，**这套算术必须真的报出那 7 个重复字段** ——
  // 否则这条判据只是"恒过"，抓不到任何东西。
  // ⚠️ **[自证] 这套算术不能是恒空的**：去掉兜底块那道门之后，`dup` 必须真的报出
  //    "两支都会渲染"的那批 —— 它是 `element 词表 − DEDICATED − skip 表`（本波 = style/anim/pos）。
  //    算出来是空集的话，上面那条"重复集合为空"就什么都证明不了。
  const mutDup = elFields.filter((f) => !ded2.has(f) && !elSkip.has(f))
  check('㉜ [自证] 去掉兜底块的门后，算术必须报出非空重复集（否则判据恒过）', mutDup.length > 0, true)
  // ⚠️ 再钉一层：**skip 表里不能有 element 词表之外的字段**（写错了没人发现，
  //    而它会让上面那条自证的数字对不上 —— 本轮 `keyword` 就是这样多出来的）。
  check('㉜ skip 表里没有 element 词表之外的字段（错字会让判据数字对不上）',
    [...elSkip].filter((f) => !elFields.includes(f)), [])

  // XY 位置控件（用户口径：仅贴纸与文字层，保留一对）
  const hasPosRow = /\.pos-row\s*\{/.test(timelineEditor) && /function setElementPos/.test(timelineEditor)
  // ⚠️⚠️ **兜底块对 element 的产出必须是空集**（2026-10-15）：element 支自己声明了它渲染的
  //     每一个字段（`ELEMENT_DEDICATED`），所以自动派生**一个都不该再出**。
  //     非空 ⇒ 面板上多出一个没有专用控件、也没人声明过的字段（用户看到的"冗余参数"）。
  const leaked = elFields.filter(
    (f) => f && !ded2.has(f) && !elSkip.has(f) &&
      (specEl.fields.find((x) => x.name === f) || {}).panel !== false,
  )
  check('㉜ element 的自动派生产出为空（每个字段都有归属，没有漏网的）', leaked, [])

  // ⚠️⚠️ **全类型版**（2026-10-15 用户口径：「所有所有所有所有」）：不止 element，
  //    **每一个**类型的自动派生都必须为空 —— 面板上不该出现任何"没人认领、自己冒出来"的字段。
  //    非空 = 用户看到的那串没标签、没条件、不知道干什么的裸控件。
  //    ⇒ 留一个入口字段就得先在专用块里手写它的控件（并加进那张 skip 表）。
  const allSkips = {}
  for (const m of timelineEditor.matchAll(/autoFields\(([^)]*)\)/g)) {
    const c = m[1]
    const idm = c.match(/'([a-z_0-9]+)'\s*,/)
    const arr = c.match(/\[([^\]]*)\]/)
    if (idm) allSkips[idm[1]] = new Set(arr ? [...arr[1].matchAll(/'([a-z_0-9]+)'/g)].map((x) => x[1]) : [])
    else if (/ELEMENT_DEDICATED/.test(c)) allSkips.element = elSkip
  }
  const leakEverywhere = []
  const specAll = JSON.parse(await read('spec/effects.json')).effects
  for (const e of specAll) {
    const skip = allSkips[e.id] || new Set()
    for (const f of (e.fields || [])) {
      if (f.panel === false || ded2.has(f.name) || skip.has(f.name)) continue
      leakEverywhere.push(`${e.id}.${f.name}`)
    }
  }
  check('㉜ 所有类型的自动派生产出都为空（没有裸控件冒出来）', leakEverywhere, [])
  // ⚠️ [自证] 拿掉某个类型的 skip 表，这套算术必须**真的报出残留** ——
  //    否则"全类型为空"只是一句恒真的话（比如循环写错了、字段名字段名对不上）。
  check('㉜ [自证] camera 的 skip 表非空（自证要有东西可拿掉）',
    (allSkips.camera || new Set()).size > 0, true)
  const camLeakWithoutSkip = specAll
    .find((e) => e.id === 'camera').fields
    .filter((f) => f.panel !== false && !ded2.has(f.name)).map((f) => f.name)
  check('㉜ [自证] 去掉 camera 的 skip 表后，算术必须报出残留（否则判据恒过）',
    camLeakWithoutSkip.length > 0, true)

  check('㉜ 元素位置 X/Y 控件存在（贴纸 + 文字，可手动钉住单个事件）', hasPosRow, true)
  // ⚠️ 三态：清空必须写 undefined（写 0 就变成"钉在左上角"，与"没设"混为一谈）
  const posTriState = /selItem\.value\[axis\] = undefined/.test(timelineEditor)
  check('㉜ 位置清空写 undefined 而不是 0（三态：没设 / 钉住）', posTriState, true)

  const MUT32 = [
    ['兜底块不再按专用支门控（每个字段出两遍）', timelineEditor,
      (s) => s.replace(/v-if="!hasDedicatedBranch\(selItem\.type\)"/, 'v-if="true"'),
      (s) => /v-if="!hasDedicatedBranch\(selItem\.type\)"/.test(s)],
    ['位置清空退回写 0（丢掉"没设"这一态）', timelineEditor,
      (s) => s.replace(/selItem\.value\[axis\] = undefined/g, 'selItem.value[axis] = \'0%\''),
      (s) => /selItem\.value\[axis\] = undefined/.test(s)]
  ]
  for (const [what, src, mutate, still] of MUT32) {
    const m = mutate(src)
    check(`[变异] ㉜ ${what} → 源码确实被改了`, m !== src, true)
    check(`[变异] ㉜ ${what} → 判据翻面`, still(m), false)
  }
}

// ㉝ **冗余/无效字段不许进面板**（2026-10-15 用户报「一堆冗余参数，这些应该都是后处理
//   自动处理的或者不该有的」）
//
// 背景：面板控件由 `spec/effects.json` 的 `fields[]` **自动派生**（㉛）。
// 派生是对的，但词汇表里有两类字段**不该有入口**：
//   · `anim` —— **只读迁移**字段：转译器只在三根轴都空时读它，写回是 no-op。
//     给了控件 = 用户改完"看着生效"、成片一个字都不动（**静默无效控件**）。
//   · `pos`  —— 2026-10-12 用户口径已把「纵向位置」下架，默认 `Auto`。
//     重新暴露 = 把已撤销的能力偷偷带回来。
// ⇒ 口径 = 字段上标 `panel: false`，`autoFields` 一律不给控件。
//   ⚠️ 字段本身**仍留在词汇表**（转译器读、老工程写），删掉会砸老工程 —— 藏的是**入口**，不是字段。
// ---------------------------------------------------------------------------
{
  console.log('\n㉝ 冗余字段不进面板（每条带变异自证）')

  const spec2 = JSON.parse(await read('spec/effects.json'))
  const hidden = []
  for (const e of spec2.effects) for (const f of (e.fields || [])) if (f.panel === false) hidden.push(f.name)
  // 目前已知的两个：anim（只读迁移）与 pos（已下架）。多一个少一个都要有人说话。
  // `style` 是第三类：那套四档 2026-10-13 被用户否决、下拉已删，但字段还在词汇表里
  // （字幕层仍用它）。它同样只能靠 `panel:false` 挡住 —— 否则自动派生会把它挂回来，
  // 而代码注释里明写着"别把它加回来"。
  // `camera.origin` / `camera.to_origin` 是第四类：**后端算出来的结果坐标**，
  // `resolve_camera_anchors`（Step 2d）每跑一次 process 就覆写一遍（源码里 `*origin = o`）。
  // 给控件 = 用户改完下一次后处理就被冲掉（静默无效）。真正的输入是 `anchor`/`anchor_to`
  // —— 面板上那对「聚焦 / 移到」。
  check('㉝ 已知的冗余字段恰好是 anim / pos / style / camera.origin / camera.to_origin',
    hidden.sort(),
    ['anim', 'origin', 'pos', 'style', 'to_origin'])

  // 派生器必须真的认这个标记（否则标了也白标 —— 这正是"看着做了、实际没做"的形状）。
  const honorsPanel = /f\.panel !== false/.test(timelineEditor)
  check('㉝ autoFields 认 `panel:false`（标了就要真的不渲染）', honorsPanel, true)

  // ⚠️ 反向：被藏的字段**必须仍然是合法词汇表字段**（藏入口 ≠ 删字段）。
  const allNames = new Set()
  for (const e of spec2.effects) for (const f of (e.fields || [])) allNames.add(f.name)
  check('㉝ 被藏的字段仍在词汇表里（老工程与转译器还要用）', hidden.every((h) => allNames.has(h)), true)

  const MUT33 = [
    ['派生器不再认 panel:false（冗余字段全冒出来）', timelineEditor,
      (s) => s.replace(/f\.panel !== false && /, ''),
      (s) => /f\.panel !== false/.test(s)],
    // ⚠️ 变异必须**逐字符对上真实排版**（`enum` 夹在 `ui` 与 `panel` 之间）——
    //    对不上时 `replace` 是静默 no-op，这条判据就变成"永远绿"（本轮踩过）。
    // ⚠️ 用**正则**而不是字面串：字面串对不上排版时 `replace` 是静默 no-op，
    //    判据就变成"永远绿"（本轮先踩了一次）。正则对缩进/间距不敏感。
    ['pos 被重新标成可见（把下架的能力带回来）', JSON.stringify(spec2),
      // ⚠️ `JSON.stringify` 出来的串**冒号后没有空格**（`{"name":"pos"`）——
      //    照着源文件的排版写正则必然不匹配，`replace` 静默 no-op（本轮踩过两次）。
      (s) => s.replace(/("name":"pos"[^}]*?)"panel":false,/, '$1'),
      (s) => {
        const p = JSON.parse(s)
        const el = p.effects.find((e) => e.id === 'element')
        return (el.fields.find((f) => f.name === 'pos') || {}).panel === false
      }]
  ]
  for (const [what, src, mutate, still] of MUT33) {
    const m = mutate(src)
    check(`[变异] ㉝ ${what} → 源码确实被改了`, m !== src, true)
    check(`[变异] ㉝ ${what} → 判据翻面`, still(m), false)
  }
}

// ㉞ **兜底块只给"没有专用支"的类型兜底**（2026-10-15 二次修）
//
// 背景：面板字段由 `fields[]` **自动派生**（㉛）。派生器有两条路：
//   ① **专用支** —— 手写控件（运镜/元素/覆盖层/音效/贴纸各自一段）；
//   ② **兜底块** —— `autoFields(selItem.type)`，给**还没写专用支**的类型。
// 对一个**已经有专用支**的类型再跑②，就是**每个字段出两遍**。
//
// ⚠️⚠️ 这条判据是本轮的用户报障换来的：第一版只把 `element` 挡在兜底块外
//   （我当时正在改 element），而 `camera` / `overlay` **照旧重复** —— 用户贴出运镜面板，
//   手写的六个控件下面又原样列了一遍 `zoom / anchor / anchor_to / cut / handheld / speed / tilt`。
//   **"还有谁有专用支"没问，就只修了手头那一个。**
//
// ⇒ 判据必须**从模板算**，不是照抄一张人手维护的表：
//   模板里的分支 = 事实；`DEDICATED_BRANCH_TYPES` = 声明；两者**必须相等**。
// ---------------------------------------------------------------------------
{
  console.log('\n㉞ 兜底块不重复渲染有专用支的类型（从模板算差集）')

  const branchTypes = new Set()
  for (const m of timelineEditor.matchAll(/selItem\.type === '([a-z_0-9]+)'/g)) branchTypes.add(m[1])
  check('㉞ 模板里取到的专用支类型非空（否则这条判据恒过）', branchTypes.size > 0, true)

  const declMatch = timelineEditor.match(/const DEDICATED_BRANCH_TYPES = \[([^\]]*)\]/)
  const decl = declMatch ? [...declMatch[1].matchAll(/'([a-z_0-9]+)'/g)].map((m) => m[1]) : null
  check('㉞ 找得到 `DEDICATED_BRANCH_TYPES`（结构变了就要改本判据）', !!decl, true)

  const missing = [...branchTypes].filter((t) => !(decl || []).includes(t)).sort()
  const extra = (decl || []).filter((t) => !branchTypes.has(t)).sort()
  check('㉞ 有专用支的类型都登记进白名单（漏一个 ⇒ 它每个字段出两遍）', missing, [])
  check('㉞ 白名单里没有不存在的专用支（多一个 ⇒ 那个类型失去兜底）', extra, [])

  check('㉞ 兜底块用 hasDedicatedBranch 门控（不是内联长串）',
    /v-if="!hasDedicatedBranch\(selItem\.type\)"/.test(timelineEditor), true)

  const specBr = JSON.parse(await read('spec/effects.json')).effects
  // ⚠️ `ded2` 是 ㉜ 那个块里的局部量，这里要自己取一份（跨块引用会 ReferenceError）。
  const dedRows = (timelineEditor.match(/const DEDICATED_FIELDS = new Set\(\[([\s\S]*?)\]\)/) || [])[1] || ''
  const ded2 = new Set([...dedRows.matchAll(/'([a-z_0-9]+)'/g)].map((m) => m[1]))
  check('㉞ 取得到 DEDICATED_FIELDS（否则它下面的判据空转）', ded2.size > 0, true)
  const skipByType = {}
  for (const m of timelineEditor.matchAll(/autoFields\(([^)]*)\)/g)) {
    const c = m[1]
    const idm = c.match(/'([a-z_0-9]+)'\s*,/)
    const arr = c.match(/\[([^\]]*)\]/)
    if (idm) skipByType[idm[1]] = new Set(arr ? [...arr[1].matchAll(/'([a-z_0-9]+)'/g)].map((x) => x[1]) : [])
    else if (/ELEMENT_DEDICATED/.test(c)) {
      const em = timelineEditor.match(/const ELEMENT_DEDICATED = \[([\s\S]*?)\n\]/)
      skipByType.element = new Set([...em[1].matchAll(/'([a-z_0-9]+)'/g)].map((x) => x[1]))
    }
  }
  const dupEverywhere = []
  for (const t of branchTypes) {
    const e = specBr.find((x) => x.id === t)
    if (!e) continue
    const skip = skipByType[t] || new Set()
    for (const f of (e.fields || [])) {
      if (f.panel === false || ded2.has(f.name) || skip.has(f.name)) continue
      dupEverywhere.push(t + '.' + f.name)
    }
  }
  check('㉞ 有专用支的类型，兜底块一个字段都不再多渲染', dupEverywhere, [])

  // ⚠️ [自证] 模拟的是**兜底块真的跑了**那一态 —— 那时它渲染的是
  //    `该类型所有非 DEDICATED 字段`，**与专用支的 skip 表无关**
  //    （skip 表管的是**兜底块自己**要不要跳过，不是"专用支没渲染"）。
  //    ⚠️ 第一版自证又把 skip 表减了一遍 ⇒ 减掉的正是修复所做的事，
  //       两条自证**互相抵消**、永远恒过（本轮踩到）。
  const catchAllFields = (t) => {
    const e = specBr.find((x) => x.id === t)
    if (!e) return []
    return (e.fields || [])
      .filter((f) => f.panel !== false && !ded2.has(f.name))
      .map((f) => t + '.' + f.name)
  }
  // ① 无门控（= 每条有专用支的类型都跑兜底块）：必须报出重复。
  const noGuard = [...branchTypes].flatMap((t) => catchAllFields(t))
  check('㉞ [自证] 无门控时算术必须报出重复（否则判据恒过）', noGuard.length > 0, true)

  // ② 复现第一版"只排除 element"：必须报出**非 element** 的那批。
  const onlyElem = [...branchTypes].filter((t) => t !== 'element').flatMap((t) => catchAllFields(t))
  check('㉞ [自证] 第一版“只排除 element”必须报出非 element 的重复（用户报障的复现）',
    onlyElem.length > 0, true)
  check('㉞ [自证] 那批里必须含运镜（用户贴的就是它）',
    onlyElem.filter((k) => k.startsWith('camera.')).length > 0, true)
  check('㉞ [自证] 第一版“只排除 element”必须被抓到（用户报障的复现）', onlyElem.length, 12)
}


// ㊳ **字体库页的样例字必须真的用库里那支渲染**（2026-10-16，用户报「字体列表有点紊乱」）。
//
//    卡片上的 `fontFamily` 写的是**族名**（`it.stack`），而这一页**从未注册过 FontFace**
//    ⇒ 浏览器不认识那些族名 ⇒ **每张卡片都用系统字体画样例**（三张卡看着一模一样、
//    换样例文字也没区别）。构建绿、两条门禁绿、界面上"只是看着都差不多"，没有任何报错。
//
//    ⚠️ 注册名必须与样例的 `fontFamily` **同源**（都是内部族名）：注册库 id 而 CSS 用族名
//    等于没注册 —— 同一个坑在精修预览那边也踩过（DOM 用原值、FontFace 用真族名）。
{
  const ml = await read('webui/src/components/MediaLibrary.vue')
  check('㊳ 字体库页给库里每支注册 FontFace（否则样例全是系统字体）',
    /ensureFontFace\s*\(/.test(ml), true)
  check('㊳ 注册名与样例的 fontFamily 同源（族名），不是库 id',
    /const name = f\.family \|\| f\.id/.test(ml), true)
  check('㊳ 样例仍走 it.stack（这条判据的锚点）', /fontFamily: it\.stack/.test(ml), true)
  {
    // 变异：把注册那一句删掉 ⇒ 第一条必须翻面（不然这条判据是空转的）。
    const bad = ml.replace(/await ensureFontFace\(null, '', name[^\n]*\n/, '')
    check('㊳ [变异] 删掉注册 ⇒ 判据翻面', /ensureFontFace\s*\(/.test(bad), false)
  }
}

// ㊴ **字体注册名必须与 CSS 用的名字同源**（2026-10-16，与 ㊳ 同一类）。
//
//    精修页有两套名字：`[style.*].font` 存的是**库 id**、而 `.cap`/`.dm` 的 `font-family` 用的是
//    `fontFamilyFor` 换出来的**内部族名**。注册 FontFace 时用错名字 ⇒ 面名与 CSS 对不上 ⇒
//    浏览器**静默回退系统字体**（"换字体没反应"，字号/位置/颜色全对）。
//    实测踩过两次：换字体回调里注册的是 id；首屏则**根本没人注册**（`ensureProjectFonts` 只在
//    底座模式下跑）。
{
  const pv = await read('webui/src/views/PolishView.vue')
  check('㊴ 换字体回调里注册用的是 fontFamilyFor（族名）而不是原值',
    /const faceName = fontFamilyFor\(fonts\.value, next\)/.test(pv), true)
  check('㊴ 首屏也注册当前字幕/弹幕/emoji 字体（不开底座时否则没人注册）',
    /registerPreviewFonts\(\)/.test(pv) && /watch\(\[fonts, subtitleFont, danmakuFont, fontEmoji\]/.test(pv), true)
  {
    // 变异：把注册名换回原值 ⇒ 第一条翻面。
    // ⚠️ **必须 `replaceAll`**：这行在两个回调里各出现一次（字幕 / 弹幕），`replace` 只替一处
    //    ⇒ 另一处仍然是对的 ⇒ 变异等于没改（第一版就是这样，判据当场报"实得 true 期望 false"）。
    const bad = pv.replaceAll('const faceName = fontFamilyFor(fonts.value, next)', 'const faceName = next')
    check('㊴ [变异] 注册名换回原值 ⇒ 判据翻面', /const faceName = fontFamilyFor\(fonts\.value, next\)/.test(bad), false)
  }
}

// ㊵ **量宽用的字体必须与绘制同一个族**（2026-10-16，与 ㊳/㊴ 同一类）。
//
//    两处量宽各自吃过一次亏：
//      · 字幕折行（`capFontSpec` → `wrapCaptionParts`）：用 `subStyle.font` **原值**（库 id）时
//        页面没有那个面 ⇒ canvas 按**回退字体**量 ⇒ 断行与成片不一致；
//      · 弹幕滚动时长（`buildIndex` → `measureDanmakuWidths`）：量宽走库 id、绘制走族名 ⇒
//        **量宽 ≠ 绘制** ⇒「弹幕没滚到边就消失」。
//    ⚠️ 变异锚点刻意**不含引号**（`+ (subFontFamily.value ?` 这种）：第一版拿一整段带引号的
//       源码当锚点，拼进 emitted JS 之后引号互相咬合、整个门禁**语法错误跑不起来**。
{
  const preview = await read('webui/src/components/PolishPreview.vue')
  const engineSrc = await read('webui/src/polish/engine.js')
  const capUsesMapped = preview.includes('+ (subFontFamily.value ?')
  const capHasSpec = preview.includes('const capFontSpec = computed(')
  check('㊵ 字幕折行量宽走映射后的族名（subFontFamily，不是原值）', capHasSpec && capUsesMapped, true)
  check('㊵ 弹幕量宽由调用方给映射后的族名（buildIndex 收 danmakuFontFamily）', + /danmakuFontFamily\b/.test(engineSrc) && engineSrc.includes(', dmFamily)') && /danmakuFontFamily: \(\) => fontFamilyFor\(props\.fonts, props\.config\?\.style\?\.danmaku\?\.font/.test(preview), true)
  {
    // 变异①：字幕量宽换回原值 ⇒ 第一条翻面。
    const badCap = preview.replace('+ (subFontFamily.value ?', '+ (subStyle.value.font ?')
    check('㊵ [变异] 字幕量宽换回原值 ⇒ 判据翻面', badCap.includes('+ (subFontFamily.value ?'), false)
    // 变异②：量宽改回读 cfg（库 id）⇒ 第二条翻面。
    const badDm = engineSrc.replace(', dmFamily)', ', dmFontFamily(cfg))')
    check('㊵ [变异] 弹幕量宽改回 dmFontFamily(cfg) ⇒ 判据翻面', badDm.includes(', dmFamily)'), false)
  }
}

// ㊶ **两层描边必须等宽**（2026-10-16，用户口径「第二层描边和第一层的宽度一致」）。
//
//    三处各吃过一次亏，共同形状是"第二层用了**别人**的宽度基准"：
//      · 文字元素（`.text-el`）：第二层曾复用**字幕**那份 `outerRing`（由 `[style.subtitle].stroke` 推），
//        而它的第一层是**元素自己的** `tx.style.strokePx` ⇒ 两个基准不同；
//      · 字幕行（`.cap`）：第一层是 `capStrokeW * c.fontScale`（**装不下时整行缩字号**，描边跟着缩），
//        而第二层挂在 `.captions` **容器**上、用的是**未缩放**的值 ⇒ 缩过的行两层必然不等；
//      · 底座/成片那两条腿：环的宽度由转译器按**同一个** stroke 值算出来（`stroke_ratio` 那一套），
//        本来就是同源的 —— 所以判据只需要钉住 DOM 这两处的**基准**。
//
//    `strokePx` / `stroke` 都是**全宽**（ASS 的 `Outline` 才是半宽，见 `check-text-style` ③）⇒
//    "等宽"判据就是：环的内宽参数必须与同一层的第一层内宽参数**同一个表达式**。
{
  const preview = await read('webui/src/components/PolishPreview.vue')
  check('㊶ 元素层的白环按元素自己的 strokePx 算（不许复用字幕那份 outerRing）',
    /outerRing: outerRingShadow\(subStyle\.value\.outer_stroke, e\.style\?\.strokePx/.test(preview) &&
    /\.\.\.\(tx\.outerRing \? \{ textShadow: tx\.outerRing \} : \{\}\)/.test(preview), true)
  check('㊶ 字幕的白环逐行算（用这一行的 fontScale），容器那份已去掉',
    /const ring = outerRingShadow\(subStyle\.value\.outer_stroke, capStrokeW\.value \* w\.scale\)/.test(preview) &&
    /\.\.\.\(c\.ring \? \{ textShadow: c\.ring \} : \{\}\)/.test(preview) &&
    !/outerRing\.value \? \{ textShadow: outerRing\.value \}/.test(preview), true)
  {
    // 变异①：元素层环换回字幕那份 ⇒ 第一条翻面。
    const badEl = preview.replace('outerRing: outerRingShadow(subStyle.value.outer_stroke, e.style?.strokePx', 'outerRing: outerRingShadow(subStyle.value.outer_stroke, subStyle.value.stroke')
    check('㊶ [变异] 元素层环改用字幕的 stroke ⇒ 判据翻面', /e\.style\?\.strokePx/.test(badEl.split('outerRing:')[1] || ''), false)
    // 变异②：字幕环用未缩放的 capStrokeW ⇒ 第二条翻面。
    const badRow = preview.replace('capStrokeW.value * w.scale)', 'capStrokeW.value)')
    check('㊶ [变异] 字幕环用未缩放的值 ⇒ 判据翻面', /const ring = outerRingShadow\(subStyle\.value\.outer_stroke, capStrokeW\.value \* w\.scale\)/.test(badRow), false)
  }
}
console.log(failed ? `\n${failed} 项失败` : '\n全部通过')
process.exit(failed ? 1 : 0)
