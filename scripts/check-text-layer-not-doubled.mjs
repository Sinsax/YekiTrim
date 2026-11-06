#!/usr/bin/env node
// **文字元素不许被同一时刻画两遍**（2026-10-14；2026-10-18 / 2026-10-20 两次口径更新）。
//
// # 背景（用户口径：「**你这个位置没有一次有变化**」）
//
// 前四轮我都在改**算法**、跑纯函数自证"算对了"，而用户看到的一点变化都没有。
// 查清的原因是**图层栈**：
//
// | 层 | 选择器 | z-index | 谁在画 |
// |---|---|---|---|
// | 底座画面（**含它自己那份文字**） | `.dhampir-canvas` | **40** | wasm / Dhampir |
// | DOM 覆盖层（字幕/弹幕/贴纸/**文字元素**） | `.overlay-space` | **10** | `engine.js` + Vue |
//
// 文字元素**两边都画**（转译器把 `kind === "text"` 落成 ASS 资产 ⇒ 底座自己会画）
// ⇒ **40 > 10** ⇒ 用户看到的**永远是底座那份**，我改的**永远是 DOM 那份**。
//
// # 它判什么
//
// ① `.dhampir-canvas` 的 `z-index` **必须高于** `.overlay-space` —— 这条是**设计**，
//    不许"顺手对齐"（它保证贴纸/选框跟着 wasm 画面走）；但要**显式钉住**，
//    因为它是下面那条判据的**前提**。
// ② 文字元素**必须按档位门控**（`byDom`）：`DHAMPIR_NO_DRAW` 里的档才由 DOM 画。
// ③ `DHAMPIR_NO_DRAW` 与**实测的底座能力**一致。
// ④ **反例**：模板里那条 `v-if="tx.byDom"` 必须在（去掉它就是"两层都画"回来了）。
// ⑤ DOM 模式下必须**全部**由 DOM 画（那时底座根本没画面）。
//
// ============================================================================
// ⚠️ ③ 的口径变过两次 —— 旧断言与理由必须留住（别再按旧表改回去）
// ============================================================================
//
//   * **2026-10-14 旧断言**：`heartbeat` **和** `shake` 都在 `DHAMPIR_NO_DRAW` 里
//     （「底座不画它」）。当时查底座 `crates/` 的确 0 命中。
//     ⚠️ 这条**已退役**：2026-10-18 底座加了周期缩放原语
//     （`scale_curve` + `scale_cycle_ms`）⇒ **heartbeat 底座真的画了**。
//     再把 heartbeat 留在本层 ⇒ 底座一层 + DOM 一层、相位一错开就是"抖上加抖"
//     （症状比"看不见"更难查）。产品侧 `PolishPreview.vue` 同一天把它摘掉了。
//   * **现在锁的口径**：集合**恰好等于** `['shake']` —— `shake` 是"本层的动效档"
//     （底座那条腿没有它的位置），其余档（含新的 `perchar` 与老的 `perchar_big`/`perchar_small`）
//     一律由底座画、本层**不许**再画。
//     用**集合相等**而不是"包含某项"：两个方向都要拦（摘掉 shake ⇒ 双层字；
//     加回 heartbeat ⇒ 双层字）。
//   * **反向/变异判据**（`--self-test`）：把 heartbeat 加回集合、把 byDom 的式子改掉、
//     把 z-index 反过来 —— 每一种都必须让本判据**翻红**。判据写错的表现是"永远绿"。
//
// 用法：node scripts/check-text-layer-not-doubled.mjs [--self-test]   （退出码非 0 即失败）

import { readFileSync } from 'node:fs'
import { join, dirname } from 'node:path'
import { fileURLToPath } from 'node:url'

const HERE = dirname(fileURLToPath(import.meta.url))
const ROOT = join(HERE, '..')
const PREVIEW = join(ROOT, 'webui', 'src', 'components', 'PolishPreview.vue')

/// ③ 的唯一权威口径（改它 = 先确认底座能力真的变了，见文件头）。
const EXPECTED_NO_DRAW = ['shake']

/**
 * 对**一份源码文本**跑全部判据（主流程传文件内容；`--self-test` 传变异体）。
 * ⚠️ 抽成函数是为了让自证跑的是**同一份判据** —— 只写在顶层的话变异台根本不会跑它。
 */
function analyze(src) {
  const pass = []
  const failures = []

  // -------------------------------------------------------------------------
  // ① 图层栈（②那条判据的**前提**）
  // -------------------------------------------------------------------------
  const zi = (sel) => {
    // 取该选择器那条规则里的 z-index（同名的会有多条，取第一条命中的）
    const re = new RegExp(`\\${sel}\\s*\\{[^}]*?z-index:\\s*(\\d+)`, 's')
    const m = src.match(re)
    return m ? Number(m[1]) : null
  }
  const zCanvas = zi('.dhampir-canvas')
  const zOverlay = zi('.overlay-space')
  const c1 = [
    zCanvas !== null && '① 找得到 `.dhampir-canvas` 的 z-index（结构变了就要改本判据）',
    zOverlay !== null && '① 找得到 `.overlay-space` 的 z-index',
    zCanvas !== null && zOverlay !== null && zCanvas > zOverlay &&
      `① 底座画布必须在 DOM 覆盖层**之上**（这是设计：贴纸/选框要跟着 wasm 画面走）—— 实得 canvas=${zCanvas} overlay=${zOverlay}`,
  ].filter(Boolean)
  pass.push(...c1)
  if (c1.length < 3) failures.push('① 图层栈：z-index 关系被改（canvas 必须 > overlay）')

  // -------------------------------------------------------------------------
  // ② 按档位门控
  // -------------------------------------------------------------------------
  const byDomOk = /v-if="tx\.byDom"/.test(src)
  const formulaOk = /byDom:\s*!dhampirMode\.value\s*\|\|\s*DHAMPIR_NO_DRAW\.has\(/.test(src)
  if (byDomOk) pass.push('② 文字元素按 `tx.byDom` 门控')
  else failures.push('② 文字元素必须按 `tx.byDom` 门控 —— 去掉它 = 同一个元素被画两遍（而底座那份在 z-index 40、盖住本层那份）')
  if (formulaOk) pass.push('② `byDom` 的式子 = DOM 模式一律画 / Dhampir 模式下只有 DHAMPIR_NO_DRAW 里的才画')
  else failures.push('② `byDom` 的式子必须是「DOM 模式一律画 / Dhampir 模式下只有 DHAMPIR_NO_DRAW 里的才画」')

  // -------------------------------------------------------------------------
  // ③ 集合内容与**实测的底座能力**一致（集合相等，两个方向都拦）
  // -------------------------------------------------------------------------
  const setMatch = src.match(/const DHAMPIR_NO_DRAW = new Set\(\[([^\]]*)\]\)/)
  const entries = setMatch
    ? setMatch[1].split(',').map((s) => s.trim().replace(/^['"]|['"]$/g, '')).filter(Boolean)
    : []
  if (!setMatch) {
    failures.push('③ 找得到 `DHAMPIR_NO_DRAW` 的定义（结构变了就要改本判据）')
  } else {
    const want = [...EXPECTED_NO_DRAW].sort()
    const got = [...entries].sort()
    if (JSON.stringify(got) === JSON.stringify(want)) {
      pass.push(`③ DHAMPIR_NO_DRAW 恰好等于 ${JSON.stringify(want)}（底座不画的档才由本层画）`)
    } else {
      const extra = got.filter((x) => !want.includes(x))
      const missing = want.filter((x) => !got.includes(x))
      failures.push(
        `③ DHAMPIR_NO_DRAW 实得 ${JSON.stringify(got)}，应当是 ${JSON.stringify(want)}` +
        (extra.length ? ` —— 多了 ${JSON.stringify(extra)}：底座已经会画它，留在集合里 = **两层各画一份**（错位双层字）` : '') +
        (missing.length ? ` —— 少了 ${JSON.stringify(missing)}：底座不画它，移出去 = 这一档在 Dhampir 模式下**彻底看不见**` : ''),
      )
    }
    // 底座**画**的档：显式点名（集合相等已覆盖，这两条让"为什么"可读）。
    const baseDrawn = ['heartbeat', 'perchar', 'perchar_big', 'perchar_small', 'vtext', 'tilt', 'arc', 'pop', 'fade']
    for (const mustNot of baseDrawn) {
      if (!entries.includes(mustNot)) pass.push(`③ 「${mustNot}」不在 DHAMPIR_NO_DRAW（底座会画它，本层不画）`)
      else failures.push(`③ 「${mustNot}」**不许**在 DHAMPIR_NO_DRAW 里 —— 底座会画它（2026-10-18 起 heartbeat 也归这条）`)
    }
  }

  // -------------------------------------------------------------------------
  // ④/⑤ 反例：DOM 模式必须全部由 DOM 画；Dhampir 模式下按集合门控
  // -------------------------------------------------------------------------
  const byDom = (dhampirMode, eff) => !dhampirMode || entries.includes(eff || '')
  const cases = [
    [byDom(false, 'perchar_big') === true, '⑤ DOM 模式下（`dhampirMode` 假）**所有档都要由本层画** —— 那时底座根本没有画面'],
    [byDom(true, 'perchar_big') === false, '⑤ Dhampir 模式下、底座画的档（`perchar_big`）**本层不许画**'],
    [byDom(true, 'perchar') === false, '⑤ Dhampir 模式下、底座画的档（`perchar`）**本层不许画**'],
    [byDom(true, 'shake') === true, '⑤ Dhampir 模式下、底座不画的档（`shake`）**本层必须画**'],
    [byDom(true, 'heartbeat') === false, '⑤ Dhampir 模式下、底座**会画**的档（`heartbeat`，2026-10-18 起有 scale_curve）本层不许画'],
    [byDom(true, '') === false, '⑤ Dhampir 模式下、没有动效档的元素（纯位置/描边）**本层不许画**'],
  ]
  for (const [okc, msg] of cases) { if (okc) pass.push(msg); else failures.push(msg) }

  return { pass, failures }
}

// ---------------------------------------------------------------------------
// 自证：判据不许空转 —— 三种"改坏产品"的形态都必须翻红
// ---------------------------------------------------------------------------
if (process.argv.includes('--self-test')) {
  const { strict: assert } = await import('node:assert')
  const src = readFileSync(PREVIEW, 'utf8')
  let self = 0
  // ① 把 heartbeat 加回集合（2026-10-18 之前的口径）⇒ ③ 必须红
  const addBack = src.replace('const DHAMPIR_NO_DRAW = new Set([', "const DHAMPIR_NO_DRAW = new Set(['heartbeat', ")
  assert.notEqual(addBack, src, '自证①：变异必须命中')
  assert.ok(analyze(addBack).failures.some((f) => f.includes('heartbeat')), '自证①：heartbeat 加回集合必须红')
  self += 1
  // ② 把 byDom 的式子改成"永远由 DOM 画" ⇒ ② 必须红
  const badFormula = src.replace(
    'byDom: !dhampirMode.value || DHAMPIR_NO_DRAW.has(',
    'byDom: true || DHAMPIR_NO_DRAW.has(',
  )
  assert.notEqual(badFormula, src, '自证②：变异必须命中')
  assert.ok(analyze(badFormula).failures.some((f) => f.includes('byDom')), '自证②：门控式子坏掉必须红')
  self += 1
  // ③ 把两层 z-index 反过来 ⇒ ① 必须红
  const zCanvas = src.match(/\.dhampir-canvas\s*\{[^}]*?z-index:\s*(\d+)/s)?.[1]
  const zOverlay = src.match(/\.overlay-space\s*\{[^}]*?z-index:\s*(\d+)/s)?.[1]
  assert.ok(zCanvas && zOverlay, '自证③：抽不到两个 z-index')
  const swapped = src
    .replace(new RegExp(`(\\.dhampir-canvas\\s*\\{[^}]*?z-index:\\s*)${zCanvas}`, 's'), `$1${zOverlay}`)
    .replace(new RegExp(`(\\.overlay-space\\s*\\{[^}]*?z-index:\\s*)${zOverlay}`, 's'), `$1${zCanvas}`)
  assert.notEqual(swapped, src, '自证③：变异必须命中')
  assert.ok(analyze(swapped).failures.some((f) => f.includes('①')), '自证③：z-index 反转必须红')
  self += 1
  console.log(`✓ 文字层双层守卫自证：${self} 项（heartbeat 加回 / byDom 式子坏 / z-index 反转，都必须红）`)
  process.exit(0)
}

const src = readFileSync(PREVIEW, 'utf8')
const { pass, failures } = analyze(src)
console.log(`文字层不许画两遍：${pass.length} 项通过，${failures.length} 项失败`)
if (failures.length) {
  console.log()
  for (const f of failures) console.log('  ✗ ' + f)
  process.exit(1)
}
console.log('✓ 底座画布在 DOM 层之上（设计）；文字元素按档位门控，底座不画的档（shake）才由本层画')
