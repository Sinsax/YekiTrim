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

console.log(failed ? `\n${failed} 项失败` : '\n全部通过')
process.exit(failed ? 1 : 0)
