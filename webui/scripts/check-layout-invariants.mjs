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
//  ⑧ 竖画布那三道整类门只挂在 `title` 上等于没说：提示必须在**行内**（用户不会去悬停）。
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

// ── ⑧ 竖画布三道整类门的说明必须在**行内**（只挂 title = 用户看不到）──
{
  const inlineHint = (s) => s.includes('竖画布默认不渲染运镜 / 特效 / 音效这三类')
  check('⑧ 竖屏三开关的行内说明在（不是只挂 title）', inlineHint(polishView), true)
  const m = polishView.replace('竖画布默认不渲染运镜 / 特效 / 音效这三类', '竖屏启用')
  check('[变异] ⑧ 摘掉行内说明 → 源码确实被改了', m !== polishView, true)
  check('[变异] ⑧ 摘掉行内说明 → 判据翻面', inlineHint(m), false)
}
console.log(failed ? `\n${failed} 项失败` : '\n全部通过')
process.exit(failed ? 1 : 0)
