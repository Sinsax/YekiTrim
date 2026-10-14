#!/usr/bin/env node
// **描边「对外宽度」预览 ↔ 成片 必须相等**（2026-10-14，问题 ③）。
//
// # 背景
//
// 用户口径：「**元素的文字层需要厚描边**」。查证时发现两边"宽"的**定义不同**：
//
//   * 预览 `PolishPreview.vue`：`-webkit-text-stroke: Npx`
//     —— CSS 的描边是**居中**的：**一半在外、一半在内** ⇒ 对外 `N/2`。
//   * 成片（libass）：`Outline = M` —— **单侧、全在外** ⇒ 对外 `M`。
//
// 而成片的 `M` 来自 `tools/polish-to-dhampir.mjs::strokePxToAss(px) = px / 2`。
//
// ⇒ **对外宽度两边恰好相等**（都是 `strokePx/2`）。这条不是巧合，
//   是 `strokePxToAss` 的文档注释里写死的那条契约：
//   同一份契约路径早就写着 `stroke_ratio: st.strokePx / 2 / height`。
//
// ⚠️ 会漂的是**向内那半边**：预览向内吃 `strokePx/2`，成片不吃。
//   那是"预览比成片糊一点"的来源 —— 已经由 `paint-order:stroke fill`
//   **大幅缓解**（描边画在字形之下，字面颜色盖回去），
//   但**不是零**。判据把它**量出来并写在这里**，而不是假装不存在。
//
// # 它判什么
//
// ① 两个口径的**对外宽度必须相等** —— 覆盖全部四个内建档
//    （`hook` 6 / `chapter` 5 / `comment` 2 / `count` 4）+ 几个边界值。
// ② `paint-order:stroke fill` **必须还在**（它一掉，预览的观感会明显变糊）。
// ③ 预览那份 `strokePx` 确实绑到了 `-webkit-text-stroke`。
// ④ **向内那半边的差**如实打印（**不是**判红项 —— 它是已知且已缓解的差异）。
//
// 用法：node scripts/check-stroke-outward-parity.mjs  （退出码非 0 即失败）

import { readFileSync } from 'node:fs'
import { join, dirname } from 'node:path'
import { fileURLToPath } from 'node:url'

const HERE = dirname(fileURLToPath(import.meta.url))
const ROOT = join(HERE, '..')
const TRANSLATOR = join(ROOT, 'tools', 'polish-to-dhampir.mjs')
const PREVIEW = join(ROOT, 'webui', 'src', 'components', 'PolishPreview.vue')

let pass = 0
let fail = 0
const failures = []
function ok(cond, what) {
  if (cond) { pass += 1 } else { fail += 1; failures.push(what) }
}

// ---------------------------------------------------------------------------
// ① 从**真源码**里抽 `strokePxToAss` —— 不重写第三份实现
//
//    ⚠️ 先去注释：那段函数的注释里**自己就写着** `stroke_ratio: st.strokePx / 2`
//       这种样字，不去注释会抽到注释里的东西。
// ---------------------------------------------------------------------------
const tsrc = readFileSync(TRANSLATOR, 'utf8')
const noComment = tsrc
  .replace(/\/\*[\s\S]*?\*\//g, '')
  .replace(/(^|[^:])\/\/[^\n]*/g, '$1')

const fnMatch = noComment.match(/function strokePxToAss\(strokePx,\s*fontPx\)\s*\{([\s\S]{0,400}?)\n\}/)
ok(!!fnMatch, '自证：从转译器源码里抽不到 `strokePxToAss`（结构变了，抽取方式要跟着改）')
const fnBody = fnMatch ? fnMatch[1] : ''
ok(/return\s+v\s*\/\s*2/.test(fnBody), '`strokePxToAss` 必须 `return v / 2`（那条 /2 是契约）')

// ⚠️ 真跑一遍那个函数（用 `new Function` 而不是照抄它的算式）——
//    照抄算式就成了"第三份实现"，那条判据会在别人改源码时**照样绿**。
const strokePxToAss = new Function('strokePx', 'fontPx', fnBody.replace(/^/gm, '  '))

// ---------------------------------------------------------------------------
// ② 预览侧的绑定：`WebkitTextStrokeWidth: <strokePx> + 'px'`
// ---------------------------------------------------------------------------
const psrc = readFileSync(PREVIEW, 'utf8')
ok(/WebkitTextStrokeWidth:\s*tx\.style\.strokePx\s*\+\s*'px'/.test(psrc),
  '预览必须把 `strokePx` **原样**绑到 `-webkit-text-stroke-width`（不加倍、不折半）')
ok(/paint-order:\s*stroke\s+fill/.test(psrc),
  '`paint-order: stroke fill` 必须还在 —— 它一掉，预览的描边会明显地往字腔里糊')

// ---------------------------------------------------------------------------
// ③ 对外宽度对拍
// ---------------------------------------------------------------------------
const TIERS = [
  ['hook', 6], ['chapter', 5], ['comment', 2], ['count', 4],
  // 边界：0 与负数走 `undefined`（调用点落回老值 2），这里只验正数那一支
  ['边界 1px', 1], ['边界 0.5px', 0.5], ['很粗 20px', 20],
]

console.log('=== 对外宽度对拍（问题 ③）===')
console.log()
console.log('  档位        strokePx | 预览对外 = px/2 | 成片 Outline | 成片对外 | 相等')
console.log('  -----------|----------|----------------|-------------|---------|-----')
for (const [name, px] of TIERS) {
  const previewOut = px / 2                     // CSS 居中 ⇒ 一半在外
  const ass = strokePxToAss(px, 80)             // 真函数
  const filmOut = ass                           // libass Outline 全在外
  const eq = Math.abs(previewOut - filmOut) < 1e-9
  ok(eq, `${name}: 对外宽度不等（预览 ${previewOut} vs 成片 ${filmOut}）`)
  console.log(`  ${name.padEnd(11)}| ${String(px).padStart(8)} | ${String(previewOut).padStart(14)} | ${String(ass).padStart(11)} | ${String(filmOut).padStart(7)} | ${eq ? '✓' : '✗'}`)
}

// ⚠️ 非有限 / 非正 ⇒ `undefined`（调用点落回老值），**不许**变成 NaN 混进 ASS
for (const bad of [0, -1, NaN, Infinity, undefined, null]) {
  const r = strokePxToAss(bad, 80)
  ok(r === undefined, `脏值 ${String(bad)} 必须返 undefined（实得 ${String(r)}）`)
}

// ---------------------------------------------------------------------------
// ④ 已知差异：如实打印**向内那半边**（不是判红项）
// ---------------------------------------------------------------------------
console.log()
console.log('=== 已知差异（**如实记录**，不是判红项）===')
console.log()
console.log('  预览的向内半边 = strokePx/2；成片**没有**向内半边。')
console.log('  ⇒ 字腔（counter）窄的字，预览会比成片糊一点点。')
console.log('  缓解：`paint-order: stroke fill` —— 描边画在字形**之下**，字面颜色盖回去。')
console.log('  ⚠ 缓解不是消除。这是**当前口径下的固有差**，')
console.log('    要彻底消掉得让预览也走"全在外"（CSS 没有这个能力，需要 canvas/SVG 描边）。')
console.log('  实测四档向内量：' + TIERS.slice(0, 4).map(([n, p]) => `${n}=${p / 2}px`).join(' · '))

// ---------------------------------------------------------------------------

console.log()
console.log(`描边对外宽度对拍：${pass} 项通过，${fail} 项失败`)
if (fail) {
  console.log()
  for (const f of failures) console.log('  ✗ ' + f)
  process.exit(1)
}
console.log('✓ 预览与成片的**对外**描边宽度逐档相等（四个内建档 + 边界值）')
