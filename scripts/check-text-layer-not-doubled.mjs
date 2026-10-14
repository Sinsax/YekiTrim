#!/usr/bin/env node
// **文字元素不许被同一时刻画两遍**（2026-10-14）。
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
// ③ `DHAMPIR_NO_DRAW` 与**实测的底座能力**一致：
//    心跳/抖动在集合里（底座不画）、逐字/竖排/倾斜/过渡**不在**（底座画）。
// ④ **反例**：模板里那条 `v-if="tx.byDom"` 必须在（去掉它就是"两层都画"回来了）。
// ⑤ DOM 模式下必须**全部**由 DOM 画（那时底座根本没画面）。
//
// 用法：node scripts/check-text-layer-not-doubled.mjs   （退出码非 0 即失败）

import { readFileSync } from 'node:fs'
import { join, dirname } from 'node:path'
import { fileURLToPath } from 'node:url'

const HERE = dirname(fileURLToPath(import.meta.url))
const ROOT = join(HERE, '..')
const PREVIEW = join(ROOT, 'webui', 'src', 'components', 'PolishPreview.vue')

let pass = 0
let fail = 0
const failures = []
function ok(cond, what) {
  if (cond) { pass += 1 } else { fail += 1; failures.push(what) }
}

const src = readFileSync(PREVIEW, 'utf8')

// ---------------------------------------------------------------------------
// ① 图层栈（②那条判据的**前提**）
// ---------------------------------------------------------------------------
const zi = (sel) => {
  // 取该选择器那条规则里的 z-index（同名的会有多条，取第一条命中的）
  const re = new RegExp(`\\${sel}\\s*\\{[^}]*?z-index:\\s*(\\d+)`, 's')
  const m = src.match(re)
  return m ? Number(m[1]) : null
}
const zCanvas = zi('.dhampir-canvas')
const zOverlay = zi('.overlay-space')
ok(zCanvas !== null, '① 找得到 `.dhampir-canvas` 的 z-index（结构变了就要改本判据）')
ok(zOverlay !== null, '① 找得到 `.overlay-space` 的 z-index')
ok(zCanvas !== null && zOverlay !== null && zCanvas > zOverlay,
  `① 底座画布必须在 DOM 覆盖层**之上**（这是设计：贴纸/选框要跟着 wasm 画面走）——`
  + ` 实得 canvas=${zCanvas} overlay=${zOverlay}`)

// ---------------------------------------------------------------------------
// ② 按档位门控
// ---------------------------------------------------------------------------
ok(/v-if="tx\.byDom"/.test(src),
  '② 文字元素必须按 `tx.byDom` 门控 —— 去掉它 = 同一个元素被画两遍'
  + '（而底座那份在 z-index 40、盖住本层那份）')

ok(/byDom:\s*!dhampirMode\.value\s*\|\|\s*DHAMPIR_NO_DRAW\.has\(/.test(src),
  '② `byDom` 的式子必须是「DOM 模式一律画 / Dhampir 模式下只有 DHAMPIR_NO_DRAW 里的才画」')

// ---------------------------------------------------------------------------
// ③ 集合内容与**实测的底座能力**一致
// ---------------------------------------------------------------------------
const setMatch = src.match(/const DHAMPIR_NO_DRAW = new Set\(\[([^\]]*)\]\)/)
ok(!!setMatch, '③ 找得到 `DHAMPIR_NO_DRAW` 的定义')
const entries = setMatch
  ? setMatch[1].split(',').map((s) => s.trim().replace(/^['"]|['"]$/g, '')).filter(Boolean)
  : []

// 底座**不画**的档（实测：`grep -rn heartbeat` 遍 Dhampir `crates/` ⇒ 0 命中）
for (const must of ['heartbeat', 'shake']) {
  ok(entries.includes(must),
    `③ 「${must}」必须在 DHAMPIR_NO_DRAW 里 —— **底座不画它**，`
    + '本层是它唯一的可见实现（移出去 ⇒ 这一档在 Dhampir 模式下彻底看不见）')
}

// 底座**画**的档（实测：`layout_text_elements` / `text_perchar_anim.rs` / ASS `Outline`）
for (const mustNot of ['perchar_big', 'perchar_small', 'vtext', 'tilt', 'arc', 'pop', 'fade']) {
  ok(!entries.includes(mustNot),
    `③ 「${mustNot}」**不许**在 DHAMPIR_NO_DRAW 里 —— **底座会画它**，`
    + '留在集合里会让两层各画一份 ⇒ **错位双层字**（比"看不见"更难查）')
}

// ---------------------------------------------------------------------------
// ④ 反例：DOM 模式必须全部由 DOM 画
// ---------------------------------------------------------------------------
{
  const byDom = (dhampirMode, eff) => !dhampirMode || entries.includes(eff || '')
  ok(byDom(false, 'perchar_big') === true,
    '⑤ DOM 模式下（`dhampirMode` 假）**所有档都要由本层画** —— 那时底座根本没有画面')
  ok(byDom(true, 'perchar_big') === false,
    '⑤ Dhampir 模式下、底座画的档（`perchar_big`）**本层不许画**')
  ok(byDom(true, 'heartbeat') === true,
    '⑤ Dhampir 模式下、底座不画的档（`heartbeat`）**本层必须画**')
  ok(byDom(true, '') === false,
    '⑤ Dhampir 模式下、没有动效档的元素（纯位置/描边）**本层不许画**')
}

// ---------------------------------------------------------------------------

console.log(`文字层不许画两遍：${pass} 项通过，${fail} 项失败`)
if (fail) {
  console.log()
  for (const f of failures) console.log('  ✗ ' + f)
  process.exit(1)
}
console.log('✓ 底座画布在 DOM 层之上（设计）；文字元素按档位门控，底座不画的档才由本层画')
