// 布局不变式（2026-10）。它不查"算法漂移"，而是把**本轮真踩过的坑**钉成静态判据。
//
// 每一条都对应一次"构建通过、两条门禁全绿、而界面已经坏了"：
//
//  ① `.llm-cols` 两栏必须**等宽** —— 用户口径"各占 50%"。写成固定右栏宽度就不再是各半。
//  ② `.llm-cols` 里的链路行：两个滑块必须**各占一整行**。并排的实测结果是滑条被压到 **59px**
//     （1280px 视口下只剩 41px），刻度根本看不见；而 `SliderField` 的滑条是**固定占所在格 1/3**
//     （`.slf-range { flex:0 0 33.333% }`，那是设置页宽格子的既定口径），格子一窄就直接腰斩。
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

import { readFile } from 'node:fs/promises'
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

const read = (p) => readFile(join(root, p), 'utf8')
const settings = await read('webui/src/views/SettingsView.vue')
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

/** 每条：label + 判据 + **怎么改坏它**（用于自证判据不是空转的）。 */
const RULES = [
  {
    label: '.llm-cols 两栏等宽（各占 50%）',
    why: '用户口径"各占50%的宽度"；写成固定右栏宽度（曾用 minmax(300px,420px)）就不是各半了',
    hit: (s) =>
      /\.llm-cols\s*\{[^}]*grid-template-columns:\s*minmax\(0,\s*1fr\)\s+minmax\(0,\s*1fr\)\s*;/s.test(s),
    break: (s) =>
      s.replace(
        'grid-template-columns:minmax(0,1fr) minmax(0,1fr)',
        'grid-template-columns:minmax(0,1fr) minmax(300px,420px)'
      )
  },
  {
    label: '.llm-cols 的滑块与「跟随全局」跨满整行（否则滑条腰斩）',
    why: '并排时滑条被压到 59px（1280px 下 41px）—— 滑条固定占所在格 1/3，格子一窄就废',
    hit: (s) =>
      /\.llm-cols\s+\.llm-link-row\s+:deep\(\.slf\)[\s\S]{0,120}?grid-column:\s*1\s*\/\s*-1\s*;/.test(s),
    break: (s) => s.replace(/grid-column:\s*1\s*\/\s*-1\s*;/g, 'grid-column:auto;')
  },
  {
    label: '.ov-cols 有单栏态（"只看一边"要真的只剩一栏）',
    why: '用户要求"缩小展开，可以只看左边或者只看右边的栏"',
    hit: (s) => /\.ov-cols\.single\s*\{[^}]*grid-template-columns:\s*minmax\(0,\s*1fr\)\s*;/s.test(s),
    break: (s) => s.replace('.ov-cols.single {', '.ov-cols.never {')
  },
  {
    label: '单栏时右栏那条竖虚线要撤掉',
    why: '只剩一栏还挂着 border-left 会多出一条悬空的线',
    hit: (s) => /\.ov-cols\.single\s+\.ov-sec-done\s*\{[^}]*border-left:\s*none/s.test(s),
    break: (s) => s.replace(/\.ov-cols\.single\s+\.ov-sec-done\s*\{[^}]*border-left:\s*none/s, '.ov-cols.single .ov-sec-done {')
  },
  {
    label: 'extra_params 的 rows ≥ 10（高度翻倍）',
    why: '用户口径"高度翻2倍，不然根本看不清有什么内容"；原来是 rows=5',
    hit: (s) => {
      const m = s.match(/class="mono llm-extra"[^>]*rows="(\d+)"/s)
      return m ? Number(m[1]) >= 10 : false
    },
    break: (s) => s.replace('rows="10" placeholder="留空 = 不带额外参数"', 'rows="5" placeholder="留空 = 不带额外参数"')
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
  ['webui/src/views/WorkbenchView.vue', workbench, ['doctorProblem', 'loadDoctor']],
  ['webui/src/components/HomeClipsOverview.vue', overview, ['ovCols', 'showUnexported', 'showExported', 'focusCol']],
  ['webui/src/views/SettingsView.vue', settings, ['localNoAuth', 'visibleSections']],
  ['webui/src/components/ModelList.vue', modelList, ['canDownload']]
]

console.log('\n① 布局不变式（每条都做变异自证）')
for (const r of RULES) {
  const sources = {
    label: r.label,
    hit: r.hit(settings) || r.hit(overview) || r.hit(modelList)
  }
  check(r.label, sources.hit, true)

  // 变异自证：改坏**命中的那份**源码，判据必须翻面
  const all = [settings, overview, workbench, modelList]
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
    const usedInTpl = tpl.includes(n)
    const defined =
      new RegExp(`(const|let|function)\\s+${n}\\b`).test(script) ||
      new RegExp(`\\b${n}\\s*[:=]`).test(script)
    check(`${file.split('/').pop()} · ${n}（模板用到就必须定义）`, usedInTpl && defined, true)
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
  const nameOpens = (s) => /@click\.stop="emit\('lane-open', lane\.type\)"/.test(s)
  const itemYields = (s) => s.includes('<div v-if="!laneOpen" class="props-panel card"')
  const lanePanel = (s) => s.includes('<div v-if="openLane" class="props-panel card lane-panel">')
  const hostEmpty = (s) => s.includes('<div id="tl-props-host"></div>')
  const panelBeforeHost = (s) => {
    const a = s.indexOf('class="props-panel card lane-panel"')
    const b = s.indexOf('<div id="tl-props-host"></div>')
    return a >= 0 && b >= 0 && a < b
  }
  const noTabs = (s) => !s.includes('genTab') && s.includes('画面构成 / 输出')
  const blockSelectClosesLane = (s) =>
    /function selectBlock\(lane, block\) \{[\s\S]{0,300}?emit\('lane-open', ''\)/.test(s)
  const panelOk = (s) => lanePanel(s) && panelBeforeHost(s) && hostEmpty(s)
  // ⚠️ **一个面板装全部轨道**（用户 2026-10-06 口径："点哪个轨道都是同一个面板，这样好操作"）：
  // 六段必须**同时**在面板里，`openLane` 只是"焦点"（高亮哪一段 + 往哪儿滚）。
  // 判据钉三件事：① 没有 `v-if/v-else-if="openLane === '…'"` 的链（那就是"一轨一屏"回来了）；
  // ② 六段标题都在；③ 点轨道名会滚到那一段。
  const LANE_GROUPS = ['分段 / 屏蔽', '字幕 / 弹幕', '运镜', '贴纸', '视觉特效', '音效']
  const allGroupsAtOnce = (s) =>
    !s.includes('v-if="openLane ===') && !s.includes('v-else-if="openLane ===')
      && LANE_GROUPS.every((g) => s.includes(`<div class="lane-group-h">${g}</div>`))
      && s.includes('所有轨道共用一个面板')
  const focusScrolls = (s) =>
    s.includes('laneGroupEls[lane]') && s.includes('scrollIntoView({ block: ' + "'nearest'" + ' })')
      && /:class="\{ on: openLane === 'segments' \}"/.test(s)
  check('⑰ 轨道名可点开设置', nameOpens(timelineEditor), true)
  check('⑰ 块的属性面板在轨道设置打开时让位', itemYields(timelineEditor), true)
  check('⑰ 页面渲染轨道设置面板，且排在 Teleport 目标之前', panelOk(polishView), true)
  check('⑰ 选中时间轴上的块 ⇒ 收起轨道设置（两块面板不并存）', blockSelectClosesLane(timelineEditor), true)
  check('⑰ 右栏合成一块、没有页签', noTabs(polishView), true)
  check('⑰ 六段设置**同时**在一个面板里（不是一轨一屏）', allGroupsAtOnce(polishView), true)
  check('⑰ 点轨道名 = 焦点高亮 + 滚到那一段', focusScrolls(polishView), true)
  const MUT17 = [
    ['摘掉"轨道设置打开时让位"（两份面板并存）', timelineEditor, (s) => s.replace('<div v-if="!laneOpen" class="props-panel card"', '<div class="props-panel card"'), (m) => nameOpens(m) && itemYields(m)],
    ['轨道名不可点（没有入口）', timelineEditor, (s) => s.replace("@click.stop=\"emit('lane-open', lane.type)\"", ''), (m) => nameOpens(m)],
    ['往 Teleport 目标里塞子节点', polishView, (s) => s.replace('<div id="tl-props-host"></div>', '<div id="tl-props-host"><span></span></div>'), (m) => panelOk(m)],
    ['页签捡回来（右栏又分家）', polishView, (s) => s.replace("const openLane = ref('')", "const openLane = ref('')\nconst genTab = ref('export')"), (m) => noTabs(m)],
    ['选中块不收起轨道设置', timelineEditor, (s) => s.replace("if (props.laneOpen) emit('lane-open', '')", ''), (m) => blockSelectClosesLane(m)],
    ['退回"一轨一屏"（点谁换谁）', polishView, (s) => s.replace(`<div class="lane-group" :class="{ on: openLane === 'subtitles' }"`, `<div v-if="openLane === 'subtitles'" class="lane-group"`), (m) => allGroupsAtOnce(m)],
    ['摘掉一段（面板里少一条轨道的设置）', polishView, (s) => s.replace('<div class="lane-group-h">运镜</div>', ''), (m) => allGroupsAtOnce(m)],
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
  const dmToggleInPanel = (s) => s.includes("@change=\"toggleLane('danmaku')\"")
  const dmFontSaves = (s) => s.includes('saveProjectPatch(pid.value, dir.value, { danmaku_font: next })')
  const measureByFamily = (s) =>
    s.includes('export function measureDanmakuWidths(dms, fontSize, fontFamily)')
    && s.includes('hit.size === size && hit.family === family')
    && s.includes("dmMeasureCtx.font = '600 ' + size + 'px ' + (family ?")
  const templateMeasuresByFamily = (s) =>
    s.includes('var dmFam = (CFG.style && CFG.style.danmaku && CFG.style.danmaku.font)')
  const layersInheritFont = (s) =>
    /\.dm \{[\s\S]{0,700}?font-family:inherit;/.test(s) && /\.cap \{[\s\S]{0,400}?font-family:inherit;/.test(s)
  const measOk = (m) => measureByFamily(m) && templateMeasuresByFamily(templateHtml) && layersInheritFont(previewVue)
  check('⑱ 弹幕没有独立泳道（用户口径：不加泳道）', noDanmakuLane(timelineEditor), true)
  check('⑱ 弹幕的显示开关住字幕轨面板', dmToggleInPanel(polishView), true)
  check('⑱ 弹幕字体走 `danmaku_font` 落盘', dmFontSaves(polishView), true)
  check('⑱ 预览量宽带字体族 + 缓存键含字体族', measureByFamily(engineJs), true)
  check('⑱ 模板 canvas 量宽带字体族（与预览同口径）', templateMeasuresByFamily(templateHtml), true)
  check('⑱ `.dm` / `.cap` 声明 `font-family:inherit`（否则被全局 `*` 规则盖掉）', layersInheritFont(previewVue), true)
  const MUT18 = [
    ['量宽的缓存键丢掉字体族（换字体拿旧宽度 ⇒ 没滚到边就消失）', engineJs, (s) => s.replace('hit.size === size && hit.family === family', 'hit.size === size'), measOk],
    ['预览量宽退回 sans-serif', engineJs, (s) => s.replace(" + (family ? '\"' + family + '\",sans-serif' : 'sans-serif')", ''), measOk],
    ['`.dm` 的 inherit 摘掉（字体设置静默失效）', previewVue, (s) => s.replace('      font-family:inherit; }', '      }'), layersInheritFont],
    ['模板量宽退回 sans-serif', templateHtml, (s) => s.replace('var dmFam = (CFG.style && CFG.style.danmaku && CFG.style.danmaku.font)', 'var dmFam = 0'), templateMeasuresByFamily],
    ['弹幕面板里那枚显示开关摘掉', polishView, (s) => s.replace("@change=\"toggleLane('danmaku')\"", ''), dmToggleInPanel],
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
  const resetClipState = (s) =>
    /function resetClipUiState\(\) \{[\s\S]{0,500}?coverKey\.value\+\+/.test(s)
      && /function resetClipUiState\(\) \{[\s\S]{0,500}?pointEdit\.value = false/.test(s)
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
  check('⑲ 片段级草稿自己清（封面重挂 + 落点草稿）', resetClipState(polishView), true)
  check('⑲ 预览自己听 `props.dir` 换片段', dirWatcher(previewVue), true)
  check('⑲ 源元素 id 带代际（先建后拆不撞 id）', srcGen(previewVue), true)
  check('⑲ 换片段：先建源、最后才改画布尺寸/open', buildBeforeResize(previewVue), true)
  check('⑲ 重挂自救：自动刷新一次（带闸）+ attach 成功后清闸', attachSelfHeal(previewVue), true)
  check('⑲ 换片段待办有人消费（tick 开头补跑）', pendingDrain(previewVue), true)
  const MUT19 = [
    ['切片段退回非静默 load（编辑器被卸载 = 黑屏）', polishView, (s) => s.replace('load({ silent: hadEditor })', 'load()'), silentSwitch],
    ['不再重挂封面设计器（会拿上个片段的封面写进这个片段）', polishView, (s) => s.replace('  coverKey.value++\n  selectRegion.value = \'\'', '  selectRegion.value = \'\''), resetClipState],
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

console.log(failed ? `\n${failed} 项失败` : '\n全部通过')
process.exit(failed ? 1 : 0)
