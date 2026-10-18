
// 底座预览宿主（web/engine.js）画字顺序/宽度的行为探针。
//
// 为什么要它：白环那一层**画出问题来是本仓最难发现的形态** —— 顺序反了整行字变成一片白
// （看起来像"字体/颜色坏了"），宽度算错则"预览白边比成片粗/细"。这两样在浏览器里要一整套
// 编排才能量（底座 runbook 自己写着"A 端没有现成出口"），而**把函数扒出来喂一个记录型 ctx**
// 就能确定性地验：顺序、lineWidth、颜色。
// **底座预览宿主的画字顺序与白环宽度**（2026-10-16）。
//
// 为什么需要它：底座 `web/engine.js` 的 `rasterizeLineUncached` 是**自己画字**的（canvas 2D），
// 顺序是「白环 → 彩描边 → 填充」。这三步**错了都不会报错**，而症状极难归因：
//
//   * 顺序反了（白环画在彩描边之后）⇒ **整行字变成一片白**，看起来像「字体/颜色坏了」；
//   * 白环 `lineWidth` 少算 `outer_stroke_px` ⇒ **预览白边比成片细一圈**（两端各自看都正常）；
//   * 契约没给 `outer_stroke_px`（老 wasm 的形态）时不该凭空画一圈，也不许崩。
//
// 浏览器里量这三样要一整套编排（底座 runbook 自己写着「A 端没有现成出口，需要临时放一个探针页」），
// 而**把函数扒出来喂一个记录型 ctx** 就能确定性地验 —— 这条判据就是这么做的。
//
// ⚠️ 它读的是**底座仓**的文件：找不到底座仓时**SKIP**（本仓常规门禁不该因为兄弟仓不在而红，
//    与 `check-cover-takumi.mjs` 同一条口径）。底座根按 `DHAMPIR_ROOT` → 默认 `F:/para/Code/Dhampir`。
//
// 用法：
//   node scripts/check-dhampir-preview-ring.mjs
//   node scripts/check-dhampir-preview-ring.mjs --self-test
import { readFileSync, existsSync } from 'node:fs'

const DH = process.env.DHAMPIR_ROOT || 'F:/para/Code/Dhampir'
const ENGINE = DH + '/web/engine.js'
if (!existsSync(ENGINE)) {
  console.log('· SKIP：没找到底座仓的 web/engine.js（设 DHAMPIR_ROOT 指到 Dhampir 仓根）')
  process.exit(0)
}
const src = readFileSync(ENGINE, 'utf8')

/** 按大括号配平扒出一个函数/常量的源码。 */
/**
 * 按大括号配平从 `source` 里扒出一段源码（函数用大括号、常量到分号）。
 *
 * ⚠️ **必须收 `source` 参数**（而不是闭包里那份原始源码）：自检要对**变异体**再扒一遍，
 *    闭包的话变异永远验不到 —— 第一版就是那样，症状还是最难查的一种：`grab(src, m)` 把
 *    **整份源码**当成了要找的串（`indexOf(自己)` = 0）⇒ 从第 0 行切出一整份文件，
 *    报的还是「`hann` 重复声明」这种与判据毫无关系的错。
 */
function grab(source, startMarker) {
  const start = source.indexOf(startMarker)
  if (start < 0) throw new Error('扒不到：' + startMarker)
  if (startMarker.startsWith('const')) {
    const end = source.indexOf(';', start)
    return source.slice(start, end + 1)
  }
  let depth = 0
  for (let i = source.indexOf(String.fromCharCode(123), start); i < source.length; i += 1) {
    if (source[i] === String.fromCharCode(123)) depth += 1
    else if (source[i] === String.fromCharCode(125)) { depth -= 1; if (depth === 0) return source.slice(start, i + 1) }
  }
  throw new Error('大括号不配平：' + startMarker)
}

/** 每次现扒（⚠️ 不能缓存：自检要对**变异体**再扒一遍 —— 缓存住的话变异永远验不到，
 *  第一版就是这么写的，自检当场戳穿"两处变异都通过了"。） */
function partsOf(src) {
  return [
    'const TEXT_FONT',
    'function cssColor(',
    'function rasterizeLineUncached(',
    'function lineColor(',
    'function textShadow(',
    'function drawShadow(',
    'function strokeColorCss(',
  ].map((m) => grab(src, m))
}

/** 记录型 2D ctx：记下每次调用的**当时**属性（画完再读就读不到了）。 */
function recordingCtx() {
  const calls = []
  const ctx = {
    calls,
    globalAlpha: 1, font: '', textAlign: '', textBaseline: '', lineJoin: '', miterLimit: 0,
    lineWidth: 0, strokeStyle: '', fillStyle: '', shadowColor: '', shadowBlur: 0, shadowOffsetX: 0, shadowOffsetY: 0,
    clearRect() {}, measureText: (t) => ({ width: String(t).length * 10 }),
    strokeText: (t, x, y) => calls.push({ op: 'strokeText', t, x, y, lineWidth: ctx.lineWidth, strokeStyle: ctx.strokeStyle }),
    fillText: (t, x, y) => calls.push({ op: 'fillText', t, x, y, fillStyle: ctx.fillStyle }),
  }
  return ctx
}
function fakeDocument(ctx) {
  return { createElement: () => ({ width: 0, height: 0, getContext: () => ctx }) }
}

const make = (ctx, src) => new Function('document', partsOf(src).join('\n') + '\nreturn rasterizeLineUncached;')(fakeDocument(ctx))

const line = { text: '字', bitmap_width: 100, bitmap_height: 60, font_px: 40, border_px: 4, scale: 1, opacity: 1 }
const baseStyle = {
  color: [255, 255, 255, 255], outline: true, stroke_px: 12, stroke_color: [26, 26, 26, 255],
  family: 'Test', weight: 400, shadow_color: null, shadow_dx_px: 0, shadow_dy_px: 0, shadow_blur_px: 0,
}
function runChecks(src) {
  const failures = []
  const ok = (cond, what) => { if (!cond) failures.push(what) }

// ① 有白环：先白环（(12+6)*2 = 36）、再彩描边（24）、最后填充
{
  const ctx = recordingCtx()
  make(ctx, src)(line, { ...baseStyle, outer_stroke_px: 6, outer_stroke_color: [255, 255, 255, 255] })
  const seq = ctx.calls.map((c) => c.op + ':' + c.lineWidth)
  ok(ctx.calls.length === 3, '有白环时应当恰好 3 次绘制（白环 / 彩描边 / 填充），实得 ' + JSON.stringify(seq))
  const ring = ctx.calls[0], stroke = ctx.calls[1], fill = ctx.calls[2]
  ok(ring && ring.op === 'strokeText' && ring.lineWidth === 36, '白环必须**第一个**画、lineWidth = (12+6)*2 = 36，实得 ' + JSON.stringify(ring))
  ok(ring && /255,\s*255,\s*255/.test(String(ring.strokeStyle)), '白环用 outer_stroke_color（白），实得 ' + (ring && ring.strokeStyle))
  ok(stroke && stroke.lineWidth === 24, '彩描边 lineWidth = 12*2 = 24，实得 ' + (stroke && stroke.lineWidth))
  ok(stroke && /26,\s*26,\s*26/.test(String(stroke.strokeStyle)), '彩描边用 stroke_color（深灰），实得 ' + (stroke && stroke.strokeStyle))
  ok(fill && fill.op === 'fillText', '填充必须**最后**画（否则被白环压掉），实得 ' + JSON.stringify(seq))
}
// ② 关掉白环（outer_stroke_px = 0）：只有彩描边 + 填充
{
  const ctx = recordingCtx()
  make(ctx, src)(line, { ...baseStyle, outer_stroke_px: 0, outer_stroke_color: [255, 255, 255, 255] })
  const seq = ctx.calls.map((c) => c.op + ':' + c.lineWidth)
  ok(ctx.calls.length === 2, '关掉白环时应当只有 2 次绘制，实得 ' + JSON.stringify(seq))
  ok(ctx.calls[0] && ctx.calls[0].lineWidth === 24, '关掉后第一次应当是彩描边（24），实得 ' + JSON.stringify(seq))
}
// ③ 契约没给 outer 键（老 wasm 的形态）：不许崩、也不许凭空画白环
{
  const ctx = recordingCtx()
  make(ctx, src)(line, { ...baseStyle })
  ok(ctx.calls.length === 2, '契约没给 outer 键时应当只有 2 次绘制，实得 ' + JSON.stringify(ctx.calls.map((c) => c.op)))
}
  return failures
}
const failures = runChecks(src)
if (process.argv.indexOf('--self-test') >= 0) {
  const selfFails = []
  const expect = (name, mutated, needle) => {
    if (mutated === src) { selfFails.push(name + '：变异没生效（锚点没匹配上）'); return }
    const got = runChecks(mutated)
    if (got.length === 0) selfFails.push(name + '：应当报错却通过了')
    else if (needle && !got.some((p) => p.includes(needle))) selfFails.push(name + '：报了别的问题 ' + JSON.stringify(got))
  }
  // ⚠️ **必须 replaceAll**：这个锚点在宿主里**出现两次**（逐段高亮那条路 + 整行那条路），
  //    `String.replace` 只替第一处 ⇒ 另一条路仍然是对的 ⇒ 变异等于没改（第一版就是这样，
  //    自检当场戳穿：两处变异都"通过了"）。
  expect('白环宽度漏算 outer_stroke_px', src.replaceAll('ctx.lineWidth = (strokePx + outerPx) * 2;', 'ctx.lineWidth = strokePx * 2;'), '白环')
  expect('白环整段不画', src.replaceAll('if (outerPx > 0 && style.outline === true && strokePx > 0) {', 'if (false) {'), '3 次绘制')
  if (selfFails.length > 0) {
    console.error('先修守卫，别信它的结论：')
    for (const f of selfFails) console.error('  - ' + f)
    process.exit(2)
  }
  console.log('✓ 底座预览画字顺序守卫自检通过（2 处变异都打红了）')
  process.exit(0)
}
console.log(failures.length === 0 ? '✓ 底座预览画字：白环 → 彩描边 → 填充，宽度与颜色都对（含「关掉白环」与「老契约」两档）' : '✗ 底座预览画字顺序/白环宽度不对')
process.exit(failures.length === 0 ? 0 : 1)
