// 文字「倾斜 + 纵向压缩」预览↔成片契约的漂移检查（T4）。
//
// 这条规矩只有**一个**判据：**CSS transform 的乘法顺序**必须与 Dhampir 出片侧逐值一致。
//
// 权威定义（两边都写死了，改任一边都要同步这里）：
//
//   ① Dhampir 文字/CPU 侧（**出片真正给字做仿射的就是它**）
//      `crates/dhampir-worker/src/text_transform.rs::blit_affine`
//      正向（它自己的四角循环展开，逐字）：
//          dx = (px − c_src_x) · sx ;  dy = (py − c_src_y) · sy
//          ox = cos·dx − sin·dy + c_out_x
//          oy = sin·dx + cos·dy + c_out_y
//      ⇒ 矩阵 **R(rot) · S(sx, sy)**（先缩放、后旋转、绕中心）
//   ② Dhampir 契约/GPU 侧 `crates/dhampir-core/src/render/compose.rs::inverse_affine`
//      正向  p_out = R(rot) · ((p_src − c_src) ⊙ scale) + c_out + offset
//      逆向  p_src = R(−rot) · (p_out − c_out − offset) ⊘ scale + c_src
//      （`blit_affine` 是它把标量 scale 推广成 (sx, sy) 的逐式推广，那条推广由
//        Dhampir 自己的单测 `各向同性时与_gpu_侧逐值相同` 钉住）
//   ③ 预览 `webui/src/polish/engine.js::textTiltTransform`
//
// ⚠️⚠️ **为什么这条必须上机器判据**：
// CSS 的 transform 是**右到左**作用的（`transform: A B` ⇒ 矩阵 `A·B`），
// 所以要得到出片的 `R·S`，必须写 **`rotate(...) scale(...)`**（rotate 在前）。
// 写成 `scale(...) rotate(...)` 得到的是 `S·R` —— 斜着的字会被沿**旋转后的轴**拉伸。
//
// 而它**极难靠肉眼发现**，因为两种顺序在两个退化情形下**完全相等**：
//   · `sy == 1`（不压扁）或 `tilt == 0`（不倾斜）⇒ `R·S == S·R`
// 本仓的四档样式里 `chapter`/`comment` 恰好就是 `tilt=0, squash=1` ——
// **默认那两档永远试不出来**，只有 `hook`(tilt=6) / `count`(tilt=−4, 但有压缩时) 才露。
//
// 运行： node webui/scripts/check-text-tilt.mjs
//        node webui/scripts/check-text-tilt.mjs --mutate   （变异自证：必须条条打红）
//
// 为什么不 import engine.js：webui/package.json 没有 `"type": "module"`，Node 会把
// engine.js 当 CJS。所以这里从源码文本里抠函数来跑（与 check-caption-wrap.mjs /
// check-overlay.mjs / check-sticker-anchor.mjs 同一套配对括号提取）。

import { readFileSync } from 'node:fs'
import { fileURLToPath } from 'node:url'
import { dirname, join } from 'node:path'

const HERE = dirname(fileURLToPath(import.meta.url))
const ENGINE = join(HERE, '..', 'src', 'polish', 'engine.js')

// ---------- 从源码文本里抠 `textTiltTransform` ----------
function extractFn(src, name) {
  const at = src.indexOf('function ' + name + '(')
  if (at < 0) throw new Error('找不到函数 ' + name + '（改名了就要同步本判据）')
  // ⚠️ **不能取第一个 `{`** —— 参数表是解构（`{ yOff = 0, … }`），那个 `}` 会提前收尾
  //    （第一版就是这么崩的：抠出来的源码缺了函数体后半段）。先跳过配对好的 `(...)`。
  let i = src.indexOf('(', at)
  let pd = 0
  for (; i < src.length; i++) {
    if (src[i] === '(') pd++
    else if (src[i] === ')') { pd--; if (pd === 0) { i++; break } }
  }
  i = src.indexOf('{', i)
  let depth = 0
  for (let j = i; j < src.length; j++) {
    if (src[j] === '{') depth++
    else if (src[j] === '}') { depth--; if (depth === 0) return src.slice(at, j + 1) }
  }
  throw new Error('函数 ' + name + ' 的花括号不配对')
}

/** 把抠出来的源码变成一个可调函数。 */
function loadTransform(mutate) {
  let src = readFileSync(ENGINE, 'utf8')
  if (mutate) src = mutate(src)
  const code = extractFn(src, 'textTiltTransform')
  // eslint-disable-next-line no-new-func
  return new Function(code + '; return textTiltTransform;')()
}

// ---------- CSS transform 串 → 3x3 矩阵（只认本判据会出现的那几个函数）----------
const DEG = Math.PI / 180
const mul = (A, B) => {
  const C = new Array(9).fill(0)
  for (let r = 0; r < 3; r++) for (let c = 0; c < 3; c++) {
    let s = 0
    for (let k = 0; k < 3; k++) s += A[r * 3 + k] * B[k * 3 + c]
    C[r * 3 + c] = s
  }
  return C
}
const IDENT = [1, 0, 0, 0, 1, 0, 0, 0, 1]

/** 按 CSS 语义把串折成矩阵：**最右的先作用** ⇒ 矩阵 = 从左往右乘。 */
function cssMatrix(str) {
  const parts = String(str).match(/[a-zA-Z]+\([^)]*\)/g) || []
  let M = IDENT
  for (const p of parts) {
    const m = p.match(/^([a-zA-Z]+)\((.*)\)$/)
    const name = m[1]
    const args = m[2].split(',').map((x) => parseFloat(x))
    if (args.some((x) => !Number.isFinite(x))) throw new Error('参数不是数：' + p)
    let A
    if (name === 'rotate') {
      const r = args[0] * DEG, c = Math.cos(r), s = Math.sin(r)
      A = [c, -s, 0, s, c, 0, 0, 0, 1]
    } else if (name === 'scale') {
      const sx = args[0], sy = args.length > 1 ? args[1] : args[0]
      A = [sx, 0, 0, 0, sy, 0, 0, 0, 1]
    } else if (name === 'scaleY') {
      A = [1, 0, 0, 0, args[0], 0, 0, 0, 1]
    } else if (name === 'translate' || name === 'translateY' || name === 'translateX') {
      const tx = name === 'translateY' ? 0 : args[0]
      const ty = name === 'translateY' ? args[0] : (args[1] ?? 0)
      A = [1, 0, tx, 0, 1, ty, 0, 0, 1]
    } else {
      throw new Error('本判据不认识 ' + name + '() —— 新增了变换就要在这里补上，'
        + '否则那条变换会被静默忽略（判据空转）')
    }
    M = mul(M, A)
  }
  return M
}

/** Dhampir 出片侧的正向线性矩阵（`blit_affine` 四角循环的展开，逐字）。
 *
 *  ⚠️ 两个 scale 的口径来自契约（`overlay.rs::TextItem` 的字段注释，原文）：
 *      `scale`   = 这一条被整体缩了多少（动画缩放）
 *      `scale_y` = **乘在 `scale` 之上**（最终纵向倍数 = `scale × scale_y`）
 *    ⇒ 所以传进 `blit_affine` 的是 `scale_x = scale`、`scale_y = scale × squashY`。
 *  这一对换算是本判据里**最容易想当然**的地方：写成 `blit_affine(scale, squashY)`
 *  就是把"压缩"当成了绝对倍数、把动画缩放丢了（实测在 `scale=1.2` 上差 0.183）。*/
function dhampirLinear(tiltDeg, scale, squashY) {
  const r = tiltDeg * DEG, cos = Math.cos(r), sin = Math.sin(r)
  const sx = scale
  const sy = scale * squashY // ← 契约：scale_y 乘在 scale 之上
  // ox = cos·((px−c)·sx) − sin·((py−c)·sy) ; oy = sin·((px−c)·sx) + cos·((py−c)·sy)
  // ⇒ [[cos·sx, −sin·sy], [sin·sx, cos·sy]]
  return [cos * sx, -sin * sy, 0, sin * sx, cos * sy, 0, 0, 0, 1]
}

const close = (a, b, tol) => Math.abs(a - b) <= tol
/** 只比**线性部分**（下标 0,1,3,4）—— 平移是"绕哪个中心/落点在哪"的事，与顺序无关。 */
function linDiff(A, B) {
  return Math.max(...[0, 1, 3, 4].map((i) => Math.abs(A[i] - B[i])))
}

// ---------- 用例 ----------
// ⚠️ 必须含**非退化**用例：`sy==1` 或 `tilt==0` 时两种顺序相等，判据会空转。
const CASES = [
  { tilt: 6, scale: 1, squash: 0.88, tag: 'hook：倾斜 + 压缩（规格值）' },
  { tilt: -4, scale: 1, squash: 0.92, tag: 'count：负角 + 压缩（非退化）' },
  { tilt: 6, scale: 1.2, squash: 0.92, tag: '倾斜 + 缩放 + 压缩三者同时' },
  { tilt: 7, scale: 1, squash: 0.85, tag: '值域上限倾斜 + 下限压缩' },
  { tilt: -7, scale: 0.7, squash: 0.9, tag: '负上限角 + 缩放下限' },
]

let pass = 0, fail = 0
const failures = []
function check(name, ok, detail) {
  if (ok) pass++
  else { fail++; failures.push(name + (detail ? ' —— ' + detail : '')) }
}

function runSuite(mutate, { verbose = true } = {}) {
  const T = loadTransform(mutate)
  let suitePass = 0, suiteFail = 0
  const rec = (ok) => { if (ok) suitePass++; else suiteFail++ }

  // ① 顺序：逐用例与 Dhampir 线性矩阵逐值一致
  for (const c of CASES) {
    const str = T({ yOff: 0, shakeX: 0, shakeY: 0, scale: c.scale, rot: 0,
                    style: { tiltDeg: c.tilt, squashY: c.squash } })
    const got = cssMatrix(str)
    const want = dhampirLinear(c.tilt, c.scale, c.squash)
    const d = linDiff(got, want)
    if (verbose && suiteFail === 0) console.log(`  ① ${c.tag}\n     ${str}\n     线性差 = ${d.toExponential(2)}`)
    rec(d < 1e-9)
  }

  // ② 串里 `rotate` 必须在 `scale` **前面**（右到左 ⇒ rotate 后作用 ⇒ 矩阵 R·S）
  {
    const str = T({ scale: 1, rot: 0, style: { tiltDeg: 6, squashY: 0.88 } })
    const iR = str.indexOf('rotate(')
    const iS = str.indexOf('scale(')
    rec(iR >= 0 && iS >= 0 && iR < iS)
  }

  // ③ 倾斜必须**与进场动画旋转相加**（两个角都在，绕同一中心）
  {
    const str = T({ scale: 1, rot: -15, style: { tiltDeg: 6, squashY: 1 } })
    const m = str.match(/rotate\(([-\d.]+)deg\)/)
    rec(!!m && close(parseFloat(m[1]), -9, 1e-9))
  }

  // ④ 纵向压缩只乘在**纵向**（横向不许被它影响）
  {
    const a = cssMatrix(T({ scale: 1, rot: 0, style: { tiltDeg: 0, squashY: 1 } }))
    const b = cssMatrix(T({ scale: 1, rot: 0, style: { tiltDeg: 0, squashY: 0.85 } }))
    rec(close(a[0], b[0], 1e-12) && close(b[4], 0.85, 1e-12))
  }

  // ⑤ R4：缺省（老工程的 7 段 source）必须是**恒等**，不歪不压
  {
    const str = T({ scale: 1, rot: 0, style: {} })
    const m = cssMatrix(str)
    const d = Math.max(...[0, 1, 3, 4].map((i) => Math.abs(m[i] - IDENT[i])))
    const hasTilt = /rotate\(0(\.0+)?deg\)/.test(str) || /rotate\(0deg\)/.test(str)
    rec(d < 1e-12 && hasTilt)
  }

  // ⑥ 位移旋钮不许丢（浮起 / 抖动）
  {
    const str = T({ yOff: -18, shakeX: 3, shakeY: -2, scale: 1, rot: 0, style: {} })
    rec(/translateY\(-18px\)/.test(str) && /translate\(3px,-2px\)/.test(str))
  }

  // ⑦ 退化情形两种顺序相等 —— **这是本条判据必须"非退化"的理由**，写出来当文档
  //
  //   `squashY == 1`（不压）或 `tilt == 0`（不歪）时 `R·S == S·R` ⇒ 顺序写错也照样绿。
  //   本仓四档里 `chapter`/`comment` 恰好就是 `tilt=0, squash=1` —— **默认那两档永远试不出来**。
  {
    const a = cssMatrix(T({ scale: 1, rot: 0, style: { tiltDeg: 0, squashY: 0.88 } }))
    const b = cssMatrix(T({ scale: 1, rot: 0, style: { tiltDeg: 6, squashY: 1 } }))
    // 这两个退化用例即使把顺序写反也仍然与出片一致（所以它们**不能**单独当判据）
    rec(close(linDiff(a, dhampirLinear(0, 1, 0.88)), 0, 1e-12))
    rec(close(linDiff(b, dhampirLinear(6, 1, 1)), 0, 1e-12))
    // 而**非退化**的那条一旦反序必然红（下面这条就是"判据不是空转"的自证）
    const c = cssMatrix(T({ scale: 1, rot: 0, style: { tiltDeg: 6, squashY: 0.88 } }))
    const flipped = mul(cssMatrix('scale(1, 0.88)'), cssMatrix('rotate(6deg)'))
    rec(Math.max(...[0, 1, 3, 4].map((i) => Math.abs(flipped[i] - dhampirLinear(6, 1, 0.88)[i]))) > 1e-3)
  }

  return { suitePass, suiteFail }
}

// ---------- 变异自证 ----------
const MUTATIONS = [
  { name: '顺序反了：scale 写到 rotate 前面（矩阵 S·R）',
    f: (s) => s.replace(
      "' rotate(' + deg + 'deg)' +\n    ' scale(' + scale + ', ' + scale * squashY + ')'",
      "' scale(' + scale + ', ' + scale * squashY + ')' +\n    ' rotate(' + deg + 'deg)'") },
  { name: '倾斜没加进去：只写进场旋转 rot',
    f: (s) => s.replace("const deg = tiltDeg + rot", "const deg = rot") },
  { name: '压缩乘到横向去了：scale(scale*squashY, scale)',
    f: (s) => s.replace("' scale(' + scale + ', ' + scale * squashY + ')'",
                        "' scale(' + scale * squashY + ', ' + scale + ')'") },
  { name: '压缩没乘动画缩放：scale(scale, squashY)',
    f: (s) => s.replace("' scale(' + scale + ', ' + scale * squashY + ')'",
                        "' scale(' + scale + ', ' + squashY + ')'") },
  { name: '缺省不再是恒等：squashY 缺省兜 0.5',
    f: (s) => s.replace('Number.isFinite(style.squashY) ? style.squashY : 1',
                        'Number.isFinite(style.squashY) ? style.squashY : 0.5') },
  { name: '位移旋钮丢了：translateY 写死 0',
    f: (s) => s.replace("'translateY(' + yOff + 'px)'", "'translateY(0px)'") },
]

// ---------- 跑 ----------
const isMutate = process.argv.includes('--mutate')

console.log('文字倾斜/压缩 预览↔成片 契约（T4）')
console.log('参照：Dhampir `text_transform.rs::blit_affine` 的正向 = R(rot)·S(sx,sy)')
console.log('')

if (!isMutate) {
  const r = runSuite((s) => s, { verbose: true })
  pass = r.suitePass; fail = r.suiteFail
  console.log(`\n①–⑦ 共 ${pass + fail} 项，通过 ${pass}，失败 ${fail}`)
  if (fail) { console.log('\n失败明细：'); for (const f of failures) console.log('  ✘ ' + f) }
  else console.log('✓ 预览的 CSS 矩阵与出片侧逐值一致（顺序为 rotate → scale）')

  // 变异自证必须**条条打红**（"一条不会红的用例不算用例"）
  console.log('\n--- 变异自证（每条都必须打红）---')
  let allRed = true
  for (const m of MUTATIONS) {
    let red = false
    try {
      const r2 = runSuite(m.f, { verbose: false })
      red = r2.suiteFail > 0
    } catch (e) {
      red = true // 抠不出函数 / 参数非法也算"打红"（变异体已不可用）
    }
    if (!red) allRed = false
    console.log(`  ${red ? '✓' : '✘'} ${m.name}`)
  }
  console.log(allRed
    ? `\n✓ 全部 ${MUTATIONS.length} 处变异都打红了（判据不是空转的）`
    : `\n✘ 有变异**没打红** —— 那条用例是假的`)
  process.exit(fail === 0 && allRed ? 0 : 1)
} else {
  let allRed = true
  for (const m of MUTATIONS) {
    let red = false
    try { red = runSuite(m.f, { verbose: false }).suiteFail > 0 } catch { red = true }
    if (!red) allRed = false
    console.log(`  ${red ? '✓ 打红' : '✘ 没红'}  ${m.name}`)
  }
  console.log(allRed ? `\n全部 ${MUTATIONS.length} 处变异都打红了。` : '\n有变异没红。')
  process.exit(allRed ? 0 : 1)
}
