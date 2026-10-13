#!/usr/bin/env node
// ============================================================================
// `check-anim-attribution.mjs` —— **归属表守卫**（39 档：形态 / 动效 / 两者 / 都没有）
// ============================================================================
//
// 用法：`node scripts/check-anim-attribution.mjs [--mutate]`
//
// # 它存在的理由
//
// 「样式」与「动效」拆成两个类型之后，**每一档的每一个参数属于哪一边**必须有个
// 唯一权威。这个权威就是 `scripts/anim-attribution.mjs`，而它有三个会腐坏的地方：
//
//   ① 改了 `ANIM_PRESETS` 却没重新生成表（表悄悄过期）；
//   ② 表的**静态判定**与**真实渲染行为**不符（表说"形态"、实际它在动，或反之）；
//   ③ 分类数变了（有档退化成两边都没有，或从"纯形态"变成"也在动"）。
//
// ⚠️ 第 ② 条是**最容易出的错**，也是本判据的核心：
//    我第一版生成脚本用了 `x in [array]`（查的是**索引**不是值），
//    于是 `arc_up` 被判成"纯形态"，而它其实**逐字随时间旋转**
//    —— 表看起来全对、只是少了一半的 `both`。**静态读表抓不住这种错**，
//    必须**真调引擎、比较帧间形状**。
import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { buildAttribution } from './gen-anim-attribution.mjs'

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const ENGINE = 'file://' + path.join(ROOT, 'webui', 'src', 'polish', 'engine.js').replace(/\\/g, '/')
const TABLE = path.join(ROOT, 'scripts', 'anim-attribution.mjs')

const MUTATE = process.argv.includes('--mutate')

const fails = []
const ok = (cond, msg) => { if (!cond) fails.push(msg) }

const E = await import(ENGINE)

// ---- 变异：把"逐字时间"那一条判据摘掉（表必须当场红） ----
let mutated = false
let importUrl = TABLE
if (MUTATE) {
  const orig = fs.readFileSync(TABLE, 'utf8')
  // 把若干净形态档的 kind 改成 'form'（模拟"漏判了逐字时间"）
  const patched = orig.replace(/'arc_up': \{ kind: 'both'/, "'arc_up': { kind: 'form'")
  if (patched === orig) {
    console.log('✗ 变异失败：表里找不到 `arc_up` 的 both 标记（判据的前提没了）')
    process.exit(1)
  }
  const copy = path.join(ROOT, 'scripts', '__mutant_attrib.mjs')
  fs.writeFileSync(copy, patched)
  mutated = true
  importUrl = copy
  console.log('【变异】把 `arc_up` 标成纯形态（漏判逐字时间），只动物副本')
}

try {
  const T = await import('file://' + importUrl.replace(/\\/g, '/'))
  const ATTR = T.ANIM_ATTRIBUTION
  const live = await buildAttribution()
  const names = Object.keys(E.ANIM_PRESETS)
  const tnames = Object.keys(ATTR).sort()

  // ---- ① 覆盖：表必须与预设表**同集合** ----
  ok(tnames.length === names.length,
    `① 表里 ${tnames.length} 档，预设表 ${names.length} 档 —— 数目不等`)
  const miss = names.filter((n) => !(n in ATTR)).sort()
  const extra = tnames.filter((n) => !(n in E.ANIM_PRESETS)).sort()
  ok(miss.length === 0, `① 这些档表里没有：${JSON.stringify(miss)}`)
  ok(extra.length === 0, `① 表里这些档预设表没有：${JSON.stringify(extra)}`)

  // ---- ② 表必须与**现场重算**一致（防"表过期"）----
  for (const n of names) {
    if (!(n in ATTR)) continue
    const a = ATTR[n], b = live[n]
    ok(a.kind === b.kind, `② \`${n}\` 表说 ${a.kind}，现场算是 ${b.kind}（表过期了）`)
    ok(JSON.stringify(a.form) === JSON.stringify(b.form),
      `② \`${n}\` 的形态键不一致：表 ${JSON.stringify(a.form)} vs 现场 ${JSON.stringify(b.form)}`)
    ok(JSON.stringify(a.pcTime) === JSON.stringify(b.pcTime),
      `② \`${n}\` 的逐字时间端不一致：表 ${JSON.stringify(a.pcTime)} vs 现场 ${JSON.stringify(b.pcTime)}`)
  }

  // ---- ③⚠️ **核心**：归属必须与**真实渲染行为**一致 ----
  //
  // "会动" = 逐字形状**帧间不同**，或整块时间函数非恒等。
  // "有形态" = 同一帧里**各字形状不同**（逐字阶梯），或整块静态量非恒等（tilt/squash/vertical）。
  const FRAMES = [0, 2, 5, 12, 40]
  const SAMPLE = '逐字样本'
  for (const n of names) {
    const a = ATTR[n]
    const per = E.animPerChar(n)
    const shapes = FRAMES.map((f) => {
      const sp = E.perCharSpans(SAMPLE, { perChar: per, perCharFrame: f, perCharFps: 30, fontPx: 72 })
      return sp ? sp.map((s) => s.style.transform || '-').join('|') : null
    })
    const perCharMoves = !!shapes[0] && new Set(shapes.filter(Boolean)).size > 1
    const mo = E.animMotion(null, n)
    const blockMoves = !!(mo && (mo.scaleInMs > 0 || mo.shakePx > 0 || mo.swayDeg > 0))
    const actuallyMoves = perCharMoves || blockMoves
    const saysMoves = a.kind === 'motion' || a.kind === 'both'
    ok(actuallyMoves === saysMoves,
      `③ \`${n}\` 表说 \`${a.kind}\`（会动=${saysMoves}），但实测会动=${actuallyMoves}` +
      `（逐字动=${perCharMoves} 整块动=${blockMoves}）—— 归属与真实行为不符`)
  }

  // ---- ④ 分类数：少了说明有档退化，变了说明要重新理解 ----
  const tally = names.reduce((m, n) => ((m[live[n].kind] = (m[live[n].kind] || 0) + 1), m), {})
  ok(tally.form === 15, `④ 纯形态档应当 15，实得 ${tally.form}`)
  ok(tally.motion === 12, `④ 纯动效档应当 12，实得 ${tally.motion}`)
  ok(tally.both === 11, `④ 两边都有的档应当 11，实得 ${tally.both}`)
  ok(tally.none === 1, `④ 两边都没有的档应当 1（只有 fade_only），实得 ${tally.none}`)
  // ⚠️ `none` 那一档必须是 fade_only —— 它动的是 opacity，**不是没实现**。
  const nones = names.filter((n) => live[n].kind === 'none').sort()
  ok(JSON.stringify(nones) === JSON.stringify(['fade_only']),
    `④ "两边都没有"的档应当只有 fade_only，实得 ${JSON.stringify(nones)}`)

  // ---- ⑤ 三个轴的档集合（界面分组要用）----
  const formSide = names.filter((n) => live[n].kind === 'form' || live[n].kind === 'both').sort()
  const motionSide = names.filter((n) => live[n].kind === 'motion' || live[n].kind === 'both').sort()
  ok(formSide.length === 26, `⑤ 样式轴应当 26 档（form 15 + both 11），实得 ${formSide.length}`)
  ok(motionSide.length === 23, `⑤ 动效轴应当 23 档（motion 12 + both 11），实得 ${motionSide.length}`)
  // ⚠️ 两轴**必须有交集** —— 交集就是"能叠加"这件事的证据。空交集说明拆法退化成互斥。
  const inter = formSide.filter((n) => motionSide.includes(n))
  ok(inter.length === 11,
    `⑤ 两轴的**交集**应当 11 档（both），实得 ${inter.length} —— ` +
    `交集为空说明"样式与动效能叠加"这条丢了`)

  // ---- ⑥ 模块导出的两个谓词必须与表一致 ----
  for (const n of names) {
    ok(T.hasFormPart(n) === (ATTR[n].kind === 'form' || ATTR[n].kind === 'both'),
      `⑥ hasFormPart('${n}') 与表不符`)
    ok(T.hasMotionPart(n) === (ATTR[n].kind === 'motion' || ATTR[n].kind === 'both'),
      `⑥ hasMotionPart('${n}') 与表不符`)
  }
} finally {
  if (mutated) {
    const copy = path.join(ROOT, 'scripts', '__mutant_attrib.mjs')
    try { fs.unlinkSync(copy) } catch { /* 本来就没有就算了 */ }
    if (fs.existsSync(copy)) {
      console.log('✗ 副本没删掉：' + copy)
      process.exit(1)
    }
    console.log('【变异】已删除副本')
  }
}

if (fails.length) {
  console.log(`✗ 归属表 ${fails.length} 处问题：`)
  for (const f of fails.slice(0, 25)) console.log('    · ' + f)
  if (fails.length > 25) console.log(`    …另有 ${fails.length - 25} 处`)
  process.exit(1)
}
console.log('✓ 归属表守卫：39 档（形态 15 · 动效 12 · 两者 11 · 无 1）—— ' +
  '表↔源码一致、表↔**实测渲染**一致、两轴交集 11 档')
