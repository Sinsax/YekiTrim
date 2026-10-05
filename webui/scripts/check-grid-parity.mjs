#!/usr/bin/env node
// 守「九宫格」的四份实现**互相一致**：`GRID_RECTS`（engine.js）· `grid_rect`（Rust）·
// 模板手抄的中心表 · 转译器的 `1/6`·`5/6`。
//
// # 为什么需要它
//
// 卡里那句「`GRID_RECTS` 须与 Rust `grid_rect` 对齐」**一直只有人读、没有机器判据** ——
// 而同一个物理点在**四处各写了一遍**，其中模板那张是**手抄的**（历史上就它先漂）。
// 2026-10-05 实测：矩形 9/9 逐值相同；**格子中心最大差 0.01%**（≈0.2px@1920，无害）
// —— 但"没有判据"就是下次改一半的入口：改一处、另外三处照旧，症状是
// 「预览里选中的区域与出片里锚住的位置差一点点」，而**不报错**。
//
// ⚠️ 这条断言的是**四份口径互相一致**，不是"某一份对了"。
// 容差 0.02%：0.01% 那点差是"取整 vs 不取整"造成的（Rust `{:.2}` / engine 不取整 /
// 模板手抄两位小数），不构成缺陷；真漂移（改错一格、抄错一位）会远超它。
//
// 运行：node webui/scripts/check-grid-parity.mjs

import { readFile } from 'node:fs/promises'
import { fileURLToPath } from 'node:url'
import { dirname, join } from 'node:path'

const here = dirname(fileURLToPath(import.meta.url))
const root = join(here, '..', '..')

const NAMES = ['top-left', 'top', 'top-right', 'mid-left', 'center', 'mid-right', 'bot-left', 'bottom', 'bot-right']

let checks = 0
let failed = 0
function check(label, actual, expected) {
  checks += 1
  const ok = JSON.stringify(actual) === JSON.stringify(expected)
  if (!ok) failed += 1
  console.log(`  ${ok ? 'ok  ' : 'FAIL'} ${label}${ok ? '' : `\n         实得: ${JSON.stringify(actual)}\n         期望: ${JSON.stringify(expected)}`}`)
}

/** 一行里的全部数字（`33.33` / `100` / `0`）。**够用就好**：这几张表里没有别的数字。 */
const nums = (line) => (line.match(/[\d.]+/g) || []).map(Number)

/** 取 `src` 里从 `head` 开始、到下一个完整 `}` 行结束的那一段（表都在这种块里）。 */
function blockOf(src, head) {
  const i = src.indexOf(head)
  if (i < 0) return ''
  const j = src.indexOf('\n}', i)
  return src.slice(i, j < 0 ? src.length : j)
}

const engineSrc = await readFile(join(root, 'webui', 'src', 'polish', 'engine.js'), 'utf8')
const typesSrc = await readFile(join(root, 'crates', 'yekitrim-render-core', 'src', 'types.rs'), 'utf8')
const html = await readFile(join(root, 'crates', 'yekitrim-polish', 'templates', 'index.html'), 'utf8')
const transSrc = await readFile(join(root, 'tools', 'polish-to-dhampir.mjs'), 'utf8')

/** engine.js 的 `GRID_RECTS`：`'top-left': [0, 0, 33.33, 33.33],` */
function rectsFromEngine(src) {
  const out = {}
  for (const line of blockOf(src, 'export const GRID_RECTS').split('\n')) {
    const k = (line.match(/'([a-z-]+)'\s*:/) || [])[1]
    const v = nums(line)
    if (k && v.length === 4) out[k] = v
  }
  return out
}

/** Rust 的 `grid_rect`：`"top-left" => (p(0.0, 0.0), p(33.33, 33.33)),` */
function rectsFromRust(src) {
  const out = {}
  for (const line of blockOf(src, 'pub fn grid_rect').split('\n')) {
    const k = (line.match(/"([a-z-]+)"\s*=>/) || [])[1]
    const v = nums(line)
    if (k && v.length === 4) out[k] = v
  }
  return out
}

/** 模板手抄的中心表：`'top-left':[16.67,16.67],'top':[50,16.67],`（**一行多项**，分隔是 `],`） */
function centersFromTemplate(src) {
  const out = {}
  const block = blockOf(src, 'function gridOrigin')
  for (const piece of block.split('],')) {
    const k = (piece.match(/'([a-z-]+)':\[/) || [])[1]
    const v = nums(piece.slice(piece.indexOf(':[')))
    if (k && v.length >= 2) out[k] = v.slice(0, 2)
  }
  return out
}

/** 转译器的 `gridCenters`：`'top-left': [1 / 6, 1 / 6], top: [1 / 2, 1 / 6],` —— **键有些没引号、一行多项** */
function centersFromTranslator(src) {
  const out = {}
  const frac = (s) => {
    const p = s.split('/').map((x) => Number(x.trim()))
    return p.length === 2 && p.every(Number.isFinite) ? (p[0] / p[1]) * 100 : NaN
  }
  const block = blockOf(src, 'const gridCenters = {')
  for (const m of block.matchAll(/['"]?([a-z-]+)['"]?\s*:\s*\[([^\]]+)\]/g)) {
    const v = m[2].split(',').map((s) => frac(s.trim()))
    if (v.length === 2 && v.every(Number.isFinite)) out[m[1]] = v
  }
  return out
}

const engine = rectsFromEngine(engineSrc)
const rust = rectsFromRust(typesSrc)
const tpl = centersFromTemplate(html)
const trans = centersFromTranslator(transSrc)

check('[grid] 四张表都解析到 9 格（解析器坏了要立刻说，别静默放行）',
  [Object.keys(rust).length, Object.keys(engine).length, Object.keys(tpl).length, Object.keys(trans).length],
  [9, 9, 9, 9])

// ① 矩形：Rust ↔ engine 逐值一致（卡里那句"须对齐"的机器判据）
const rectDiff = NAMES.filter((n) => !rust[n] || !engine[n] || rust[n].some((x, i) => Math.abs(x - engine[n][i]) > 0.001))
check('[grid] GRID_RECTS 与 Rust grid_rect 逐值一致', rectDiff, [])

// ② 中心：Rust 现算 / engine 现算 / 模板手抄 / 转译器分数，四份互差 ≤0.02%
const centerBad = []
for (const n of NAMES) {
  const r = rust[n]
  if (!r) { centerBad.push(`${n}:rust 缺`); continue }
  const ref = [(r[0] + r[2]) / 2, (r[1] + r[3]) / 2]
  const cands = { engine: engine[n] && [(engine[n][0] + engine[n][2]) / 2, (engine[n][1] + engine[n][3]) / 2], tpl: tpl[n], trans: trans[n] }
  for (const [who, v] of Object.entries(cands)) {
    if (!v) { centerBad.push(`${n}:${who} 缺`); continue }
    if (v.some((x, i) => Math.abs(x - ref[i]) > 0.02)) centerBad.push(`${n}:${who} 偏 ${(v[0] - ref[0]).toFixed(3)}/${(v[1] - ref[1]).toFixed(3)}`)
  }
}
check('[grid] 格子中心四份实现互差 ≤0.02%（模板那张手抄表是历史漂移入口）', centerBad, [])

// 自证有效：把 engine 的 center 那格改坏（**纯字符串替换**，这格里有方括号，
// 走 `new RegExp` 还要再套一层转义 —— 2026-10-05 就是在这上面连翻三次）
const FROM = "'center': [33.33, 33.33, 66.67, 66.67]"
const TO = "'center': [30, 33.33, 66.67, 66.67]"
if (engineSrc.indexOf(FROM) < 0) {
  failed += 1
  checks += 1
  console.log('  FAIL [grid·变异] 源码里定位不到变异点（这项检查是空转的）')
} else {
  const mutated = engineSrc.replace(FROM, TO)
  const diff2 = NAMES.filter((n) => {
    const a = rust[n], b = rectsFromEngine(mutated)[n]
    return !a || !b || a.some((x, i) => Math.abs(x - b[i]) > 0.001)
  })
  check('[grid·变异] 改坏一格后「矩形逐值一致」会翻面（本脚本抓得住这类回归）', diff2, ['center'])
}

console.log(failed ? `\n${failed} 项失败（共 ${checks} 项）` : `\n全部通过（${checks} 项）`)
process.exit(failed ? 1 : 0)
