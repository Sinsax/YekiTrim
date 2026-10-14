#!/usr/bin/env node
// **逐字 span 的不变式：绝不产出 `NaN`／`Infinity` 的 `transform`**（2026-10-14）。
//
// # 为什么要有这一条
//
// `perCharSpans` 的数学里有一项是
//     `scale: mix(spec.scaleFrom, spec.scaleTo) * (1 + spec.scaleSpread * rank)`
// 只要 `spec` 少了那几个键，`mix(undefined, undefined)` 就是 **`NaN`** ⇒
// 整条 `transform` 变成 `rotate(-14deg) scale(NaN)`。
//
// ⚠️⚠️ **浏览器对非法 `transform` 是「整条丢弃」，不是「忽略那一项」**：
//    症状是"逐字档配了、预览里字**完全不动**"，而成片那边 Rust 有
//    `#[serde(default = "one")]` 兜底、**正常在动** ⇒ 两边分叉，**且不报错**。
//
// # 为什么单靠"生产路径没事"不够
//
// 生产路径（`animPerChar` → `{...PER_CHAR_DEFAULT, ...p.perChar}`）**当前**不会踩到，
// 但那是**调用约定**，不是 `perCharSpans` 自己的契约。它 `export` 着，
// 任何新消费者直接喂预设里的那个 `perChar`（**我的探针就是这么踩到的**）就会中招。
// ⇒ 把"不许产出 NaN"钉成**函数自己的不变式**。
//
// # 它怎么判
//
// 不重写算法，而是**喂各种残缺的 `spec`**（缺键 / 脏值 / 空对象）进真函数，
// 断言每一条产出的 `transform` 里**不含 `NaN` / `Infinity` / `undefined`**。
// 另有一组**正例**（完整 spec）确保它不是"什么都不输出"就绿了 ——
// ⚠️ 一条把输出清空的实现能骗过"不含 NaN"，所以正例是必需的。
//
// 用法：node scripts/check-perchar-spans-finite.mjs
//
// 退出码非 0 即失败。

import { readFileSync } from 'node:fs'
import { join, dirname } from 'node:path'
import { fileURLToPath, pathToFileURL } from 'node:url'

const HERE = dirname(fileURLToPath(import.meta.url))
const ENGINE = join(HERE, '..', 'webui', 'src', 'polish', 'engine.js')

let pass = 0
let fail = 0
const failures = []
function ok(cond, what) {
  if (cond) { pass += 1 } else { fail += 1; failures.push(what) }
}

const mod = await import(pathToFileURL(ENGINE).href)
const { perCharSpans, animPerChar, animPreset, PER_CHAR_DEFAULT } = mod

// ⚠️ 探针要真的**驱动应用模块**，不是读源码文本 —— 这条不变式是**运行时**属性。
ok(typeof perCharSpans === 'function', 'engine.js 要 export perCharSpans');

/** 一条 `transform` 是否"干净"（不含 NaN/Infinity/undefined）。 */
const dirty = (s) => /NaN|Infinity|undefined|null/.test(String(s))

const TEXT = '一段弧线测试文字'
const BASE = { perCharFrame: 9, perCharFps: 30, fontPx: 80 }

function transformsOf(spec, label) {
  const sp = perCharSpans(TEXT, { perChar: spec, ...BASE })
  if (sp === null) return { sp: null, trs: [] }
  return { sp, trs: sp.map((c) => (c.style && c.style.transform) || '') }
}

// ---------------------------------------------------------------------------
// ① 反例：各种**残缺**的 spec 都不许产出脏 transform
// ---------------------------------------------------------------------------
const 残缺 = [
  ['预设里那个 raw `perChar`（arc）', animPreset('arc').perChar],
  ['预设里那个 raw `perChar`（perchar_big）', animPreset('perchar_big').perChar],
  ['预设里那个 raw `perChar`（perchar_small）', animPreset('perchar_small').perChar],
  ['空对象', {}],
  ['只给帧数（其它键全缺）', { perCharFrames: 8 }],
  ['键名写错（snake_case 喂给 camelCase 消费者）',
    { per_char_frames: 8, rotation_to: -14, stagger_frames: 3 }],
  ['脏值：字符串', { perCharFrames: 8, rotationTo: 'x', scaleFrom: 'y', scaleTo: 'z' }],
]
for (const [label, spec] of 残缺) {
  const { trs } = transformsOf(spec, label)
  const bad = trs.filter(dirty)
  ok(bad.length === 0,
    `残缺 spec「${label}」产出了脏 transform：${bad.slice(0, 2).join(' | ') || '（无，但形状不对）'}`)
}

// ---------------------------------------------------------------------------
// ② 正例：完整 spec **必须真的动** —— 否则"什么都不输出"也能骗过 ①
//
//    ⚠️ 这一组是这条判据的**灵魂**：没有它，一个 `return null` 的实现全绿。
// ---------------------------------------------------------------------------
{
  const arc = animPerChar('arc')
  const { sp, trs } = transformsOf(arc, 'arc(完整)')
  ok(sp !== null, '完整 spec 必须返回 spans（不能是 null）')
  ok(trs.some((t) => /rotate\(/.test(t)), 'arc 必须产出 rotate(...)')
  const degs = trs.map((t) => {
    const m = String(t).match(/rotate\((-?[\d.eE+-]+)deg\)/)
    return m ? Number(m[1]) : null
  }).filter((v) => v !== null)
  ok(degs.length >= 2, `arc 至少有 2 个字带 rotate（实得 ${degs.length}）`)
  // 弧线 = **逐字不同**的角度（全同一个角度那是"整块旋转"，不是弧）
  ok(new Set(degs.map((d) => d.toFixed(3))).size >= 2,
    `arc 的角度要**逐字不同**（实得 ${[...new Set(degs)].join(',')}）`)
}
{
  // `perchar_big` 靠 `scaleSpread` 放大 —— 必须真的出现 scale(...)
  const pc = animPerChar('perchar_big')
  const { trs } = transformsOf(pc, 'perchar_big(完整)')
  ok(trs.some((t) => /scale\(/.test(t)), 'perchar_big 必须产出 scale(...)')
  ok(!trs.some(dirty), 'perchar_big 的 transform 不许脏')
}
{
  // 完整 spec 经过 `animPerChar` 后**键要齐** —— 这是"生产路径不会 NaN"的根据。
  const arc = animPerChar('arc')
  for (const k of ['scaleFrom', 'scaleTo', 'scaleSpread', 'rotationFrom', 'rotationTo']) {
    ok(Number.isFinite(arc[k]), `animPerChar('arc').${k} 要是有限数（实得 ${arc[k]}）`)
  }
  ok(Number.isFinite(PER_CHAR_DEFAULT.scaleFrom) && PER_CHAR_DEFAULT.scaleFrom === 1,
    'PER_CHAR_DEFAULT.scaleFrom 必须是 1（0 会让字缩成不可见，而 0 是合法值 ⇒ 不报错）')
}

// ---------------------------------------------------------------------------
// ③ 变异自证：把 ① 的守卫拆掉，必须**当场红**
//
//    ⚠️ 做法：拿真源码，把 `okNum` 那道过滤还原成"直接 push"，
//       再用**同一组残缺 spec** 跑一遍 —— 必须至少有一条变脏。
//       证明的是"这道过滤真的在起作用"，而不是"恰好没有 NaN"。
// ---------------------------------------------------------------------------
{
  const src = readFileSync(ENGINE, 'utf8')
  ok(/Number\.isFinite\(v\)/.test(src) || /const okNum =/.test(src),
    '变异自证的前提：源码里要有那道有限性过滤（`okNum`）')
  // 还原成不作过滤的形态
  const mutated = src
    .replace("if (okNum(t.rotationDeg) && t.rotationDeg) parts.push('rotate(' + t.rotationDeg + 'deg)')",
      "if (t.rotationDeg) parts.push('rotate(' + t.rotationDeg + 'deg)')")
    .replace("if (okNum(t.scale) && t.scale !== 1) parts.push('scale(' + t.scale + ')')",
      "if (t.scale !== 1) parts.push('scale(' + t.scale + ')')")
  ok(mutated !== src, '变异自证：替换真的命中了（没命中说明锚点过时了，这条自证自己无效）')
  // ⚠ 变异体**必须写在 engine.js 旁边**：它 `import`
  //   同目录的 `canvas.js` 等，放到 `.workbuddy/` 会 ERR_MODULE_NOT_FOUND。
  const tmp = join(HERE, '..', 'webui', 'src', 'polish', 'engine.mut-perchar-finite.mjs')
  const { writeFileSync, mkdirSync, rmSync } = await import('node:fs')
  mkdirSync(dirname(tmp), { recursive: true })
  writeFileSync(tmp, mutated, 'utf8')
  try {
    const m = await import(pathToFileURL(tmp).href + '?v=' + Date.now())
    const raw = animPreset('arc').perChar
    const sp = m.perCharSpans(TEXT, { perChar: raw, ...BASE })
    const trs = (sp || []).map((c) => (c.style && c.style.transform) || '')
    ok(trs.some(dirty),
      '变异自证：拆掉有限性过滤后，raw `perChar` **必须**产出脏 transform（否则这道过滤是空转的）')
  } finally {
    rmSync(tmp, { force: true })
  }
}

// ---------------------------------------------------------------------------

console.log(`逐字 span 有限性：${pass} 项通过，${fail} 项失败`)
if (fail) {
  console.log()
  for (const f of failures) console.log('  ✗ ' + f)
  process.exit(1)
}
console.log('✓ 逐字 duration/rotation/scale 不会产出 NaN —— 非法 transform 会被浏览器整条丢弃')
