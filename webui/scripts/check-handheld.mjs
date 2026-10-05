// 手持摇摆（`Event::Camera::handheld`）的漂移对拍。
//
// 同一条摇摆算法有三份实现（同一张固定参数表）：
//   1. `crates/yekitrim-render-core/src/types.rs::handheld_sway`（Rust 权威 + 状态机幅度）
//   2. `crates/yekitrim-polish/templates/index.html::handheldSway`（Chrome 出片权威 + DOM 预览）
//   3. `webui/src/polish/engine.js::handheldSway`（WebUI 预览，经 PolishPreview.vue）
// 谁漂了，表现就是「WebUI 预览 / 模板预览 / 成片 三者摇摆不一致」——
// 而且摇摆量很小（±9.5px / ±0.5°），对拍是唯一能在肉眼发现前抓住它的手段。
//
// Rust↔(2)(3) 由 `timeline.rs::handheld_sway_matches_js_reference` 用**硬编码期望值**钉住，
// 本脚本把同一组期望值也钉住 JS 两份，并额外对拍**起停状态机**（幅度 0~1 的曲线）：
// 状态机在模板 `getActiveCamera` 与 engine `cameraAt` 里各有一份循环，
// 口径（同一段 `min(dur*0.5, 0.6)` 窗口 + hardCut 瞬变 + 跟到下一条运镜）必须一致。
//
// 运行： node webui/scripts/check-handheld.mjs
//
// 为什么从源码文本抠函数、不 import：webui/package.json 没有 `"type": "module"`，
// Node 会把 engine.js 当 CJS（engine.js 有 `import ... from './canvas.js'`，data-URL 动态
// import 也不行）。同一套做法见 check-caption-wrap.mjs / check-overlay.mjs。

import { readFile } from 'node:fs/promises'
import { fileURLToPath } from 'node:url'
import { dirname, join } from 'node:path'

const here = dirname(fileURLToPath(import.meta.url))
const root = join(here, '..', '..')

// ── 从源码里抠出函数（用配对括号取整段）────────────────────────────────────
function extractFunction(src, name) {
  const start = src.indexOf(`function ${name}(`)
  if (start < 0) throw new Error(`找不到 function ${name}(`)
  let i = src.indexOf('{', start)
  let depth = 0
  for (; i < src.length; i++) {
    const ch = src[i]
    if (ch === '/' && src[i + 1] === '/') {
      const nl = src.indexOf('\n', i)
      if (nl < 0) break
      i = nl
      continue
    }
    if (ch === '/' && src[i + 1] === '*') {
      const e = src.indexOf('*/', i + 2)
      i = e < 0 ? src.length : e + 1
      continue
    }
    if (ch === "'" || ch === '"' || ch === '`') {
      const q = ch
      i++
      while (i < src.length && src[i] !== q) {
        if (src[i] === '\\') i++
        i++
      }
      continue
    }
    if (ch === '{') depth++
    else if (ch === '}') {
      depth--
      if (depth === 0) return src.slice(start, i + 1)
    }
  }
  throw new Error(`function ${name} 的括号没配平`)
}

const htmlPath = join(root, 'crates', 'yekitrim-polish', 'templates', 'index.html')
const html = await readFile(htmlPath, 'utf8')
const enginePath = join(root, 'webui', 'src', 'polish', 'engine.js')
const engineSrc = await readFile(enginePath, 'utf8')

// engine.js 的波形用模块级 `HANDHELD_TAU`（模板那份把 TAU 写在函数体内），抠函数时补上这个前提
const TAU_DECL = 'const HANDHELD_TAU = Math.PI * 2;\n'
const swayOf = (src) =>
  new Function(`${TAU_DECL}\n${extractFunction(src, 'handheldSway')}\n return handheldSway`)()

const tplSway = swayOf(html)
const engSway = swayOf(engineSrc)

// ── 断言脚手架 ────────────────────────────────────────────────────────────
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

const near = (a, b, tol = 1e-9) => Math.abs(a - b) < tol
const round = (x, d = 6) => Math.round(x * 10 ** d) / 10 ** d

// ── 1. Rust 参考点（与 timeline.rs::handheld_sway_matches_js_reference 同一组期望值）──
// amp=1 全幅，四点覆盖两个正弦分量的相位；amp=0 必须完全静止（与 t 无关）
const RUST_REF = [
  [0.0, [2.9749944313574055, 5.537746181460179, 0.16289376221300536, 1.0062384515204302]],
  [1.5, [-6.534804875504133, 1.9985076676346243, -0.35893342684390406, 0.9986913740508892]],
  [4.25, [-2.966335968174989, -1.8592597407226052, -0.10090716715184872, 1.0030767917992185]],
  [9.9, [-5.095502881135531, -1.5021441809987914, -0.28712740606583997, 1.003292193865482]],
]

console.log('与 Rust 参考点逐值对拍（timeline.rs 里硬编码的那四点）')
for (const [label, fn] of [['template', tplSway], ['engine  ', engSway]]) {
  check(`[${label}] amp=0 → 完全静止`, { dx: fn(3.7, 0).dx, dy: fn(3.7, 0).dy, rot: fn(3.7, 0).rot, scale: fn(3.7, 0).scale }, { dx: 0, dy: 0, rot: 0, scale: 1 })
  for (const [t, want] of RUST_REF) {
    const got = fn(t, 1.0)
    const ok = near(got.dx, want[0]) && near(got.dy, want[1]) && near(got.rot, want[2]) && near(got.scale, want[3])
    check(`[${label}] t=${t} 四项`, ok, true)
    if (!ok) console.log(`         dx=${got.dx} dy=${got.dy} rot=${got.rot} scale=${got.scale}`)
  }
}

// ── 2. 两份 JS 逐值对拍（含半幅：过渡窗口里会出现中间幅度）──────────────────
console.log('\n两份 JS 逐值对拍（振幅 0/0.25/1 × 采样点）')
for (const amp of [0, 0.25, 1]) {
  for (const t of [0, 0.37, 1.5, 3.25, 7.9, 12.5, 44.4, 121.6]) {
    const a = tplSway(t, amp)
    const b = engSway(t, amp)
    const same = ['dx', 'dy', 'rot', 'scale'].every((k) => near(a[k], b[k]))
    check(`[对拍] t=${t} amp=${amp}`, same, true)
    if (!same) console.log(`         template=${JSON.stringify(a)}\n         engine  =${JSON.stringify(b)}`)
  }
}

// ── 3. 起停状态机对拍（模板 getActiveCamera vs engine cameraAt）──────────────
// 两份循环的口径：td = min(dur*0.5, 0.6)；hardCut = cut 或任一端「远」→ 瞬变；
// 幅度是 0/1 档但**跟到下一条运镜**（持续状态，不是一次性特效）。
// 依赖的自由变量（E / lerp / camFactor / regionOrigin / CFG / CW / CH / idx）由 stub 提供，
// 两份**同一套 stub** —— 这样比出来的差异只可能来自被抠出来的循环本身。
const STUB = `
  var E = {
    pow2_out: function(t) { return 1 - (1 - t) * (1 - t); },
    pow2Out: function(t) { return 1 - (1 - t) * (1 - t); }
  };
  function lerp(a, b, t) { return a + (b - a) * t; }
  function camFactor(label) { if (label === '近') return 1.5; if (label === '远') return 0.55; return 1.3; }
  function regionOrigin() { return '65% 50%'; }
  function gridOrigin() { return '65% 50%'; }
  var CW = 1920, CH = 1080;
  var EVENT_IDX = { layout: {} };
  var CFG = { vertical: false, style: { camera: {} } };
`
const tplCam = new Function(`${STUB}\n${extractFunction(html, 'getActiveCamera')}\n return getActiveCamera`)()
const engCam = new Function(`${STUB}\n${extractFunction(engineSrc, 'cameraAt')}\n return cameraAt`)()

// 事件序列：近景开摇摆 → 近景续 → 硬切收 → 远景（硬切，另一端瞬变）→ 中景收
const CAMS = [
  { time: 0.0, duration: 4.0, ev: { zoom: '近', handheld: true } },
  { time: 6.0, duration: 4.0, ev: { zoom: '近', handheld: true } },
  { time: 10.0, duration: 0.4, ev: { zoom: '中', handheld: false, cut: true } },
  { time: 13.0, duration: 3.0, ev: { zoom: '远', handheld: true } },
  { time: 17.0, duration: 3.0, ev: { zoom: '中', handheld: false } },
]
const engIdx = { vertical: false, camera: CAMS, camStyle: {}, cv: { W: 1920, H: 1080 }, layout: {} }

console.log('\n起停状态机对拍（幅度曲线 + 景别，逐采样点）')
for (const t of [0, 0.3, 0.6, 2, 5.9, 6.0, 6.3, 6.6, 9.9, 10.0, 10.2, 12.9, 13.0, 13.5, 16.9, 17.0, 17.3, 18.0, 25.0, 40.0]) {
  const a = tplCam(t, CAMS)
  const b = engCam(engIdx, t)
  check(
    `[对拍] t=${t} 幅度/景别`,
    { handheld: round(a.handheld), zoom: round(a.zoom), ox: round(a.ox, 3), oy: round(a.oy, 3) },
    { handheld: round(b.handheld), zoom: round(b.zoom), ox: round(b.ox, 3), oy: round(b.oy, 3) }
  )
}
// 竖素材必须**整条**归零（模板 getActiveCamera 的口径；engine cameraAt 另有自己的门）。
// 用户报的「竖屏素材没禁运镜、横屏导出也还在缩放」= 模板曾只归零 handheld、zoom 照跑：
// 这一条就是那个门，摘掉它 zoom 立刻回到 1.5。
{
  const verticalSrc = STUB.replace('{ vertical: false,', '{ vertical: true,')
  const verticalTpl = new Function(
    `${verticalSrc}\n${extractFunction(html, 'getActiveCamera')}\n return getActiveCamera`
  )()
  const tv = verticalTpl(2.0, CAMS)
  check('[template] 竖素材 → 幅度归零', tv.handheld, 0)
  check('[template] 竖素材 → 景别也归零（zoom=1，横画布导出同样禁）', tv.zoom, 1)
  const idxV = { ...engIdx, vertical: true }
  const ev = engCam(idxV, 2.0)
  check('[engine  ] 竖素材 → 幅度归零', ev.handheld, 0)
  check('[对拍] 竖素材 → 模板与 engine 的 zoom / 不动点逐值一致',
    { zoom: round(tv.zoom), ox: round(tv.ox, 3), oy: round(tv.oy, 3) },
    { zoom: round(ev.zoom), ox: round(ev.ox, 3), oy: round(ev.oy, 3) })

  // 自证有效：把模板的竖素材门摘掉 → 上面两条必须翻面（CAMS 在 t=2.0 是「近」= 1.5、摇摆开着）
  const noGate = html.replace('if (CFG.vertical) return { zoom: 1.0,', 'if (false) return { zoom: 1.0,')
  check('[变异] 竖素材门确实被摘掉（源码被改了）', noGate !== html, true)
  if (noGate !== html) {
    const tplNoGate = new Function(
      `${verticalSrc}\n${extractFunction(noGate, 'getActiveCamera')}\n return getActiveCamera`
    )()
    check('[变异] 摘掉竖素材门 → zoom 判据翻面（回到 1.5）', tplNoGate(2.0, CAMS).zoom, 1.5)
    check('[变异] 摘掉竖素材门 → 幅度判据翻面（回到 1）', tplNoGate(2.0, CAMS).handheld, 1)
  }
}

// ── 3c. 瞬时特效强度：三份实现的「缺省 / 0」口径必须一致 ──────────────────────
// `intensity` 是**像素**位移，`0` 是合法值（= 不抖）。曾经两份预览都写 `ev.intensity || 14`
// —— falsy 判定把 0 变成 14px ⇒ **预览抖、出片（转译器只认 number）不抖**；而 LLM 提示词的
// 唯一示例还写着 `0.5`（亚像素 ⇒ 两边都看不见，任务报告却照写 `shake_events: 1`）。
// 本段钉住：两份 JS 里取默认的那条表达式**逐字一致**、都是数值判定、缺省都是 14；
// 出片那份（转译器）同样只认 number。亚像素示例由 Rust 单测
// `llm::segment::tests::shake_prompt_example_is_pixel_scale` 钉住。
console.log('\n瞬时特效强度：两份 JS + 转译器（出片）同口径')
const INTENSITY_DEFAULT = 14
const WANT_GUARD = `typeof sh.ev.intensity==='number'?sh.ev.intensity:${INTENSITY_DEFAULT}`
const normGuard = (s) =>
  s.replace(/\s*===\s*/g, '===').replace(/\s*\?\s*/g, '?').replace(/\s*:\s*/g, ':').trim()
const guardOf = (src, label) => {
  const body = extractFunction(src, 'getShake')
  const m = body.match(/off\s*=\s*([^\n;]+)/)
  check(`${label}：能在 getShake 里定位到 off 的取值`, Boolean(m), true)
  return m ? normGuard(m[1]) : ''
}
const tplGuard = guardOf(html, 'index.html')
const engGuard = guardOf(engineSrc, 'engine.js')
check('两份 JS 的强度取值表达式逐字一致', tplGuard === engGuard, true)
check(`表达式 = 数值判定 + 缺省 ${INTENSITY_DEFAULT}`, tplGuard, WANT_GUARD)
check('表达式里没有 falsy 判定 `||`', /\|\|/.test(tplGuard), false)
const translatorSrc = await readFile(join(root, 'tools', 'polish-to-dhampir.mjs'), 'utf8')
check(
  '转译器（出片）：intensity 只认 number、缺省 14',
  /typeof event\.intensity === 'number' \? event\.intensity : 14/.test(translatorSrc),
  true
)
// 自证有效：把模板那份改回 `|| 14` → 上面两条必须翻面
const guardPat = /var off = typeof sh\.ev\.intensity==='number' \? sh\.ev\.intensity : 14;/
check('能在模板源码里定位到那条取值表达式（变异前提）', guardPat.test(html), true)
if (guardPat.test(html)) {
  const mutated = html.replace(guardPat, 'var off = sh.ev.intensity || 14;')
  check('[变异] 变异确实改到了模板源码', mutated !== html, true)
  const mutatedGuard = guardOf(mutated, 'index.html(perturbed)')
  check('[变异] 改回 `|| 14` → 同口径判据翻面', mutatedGuard === WANT_GUARD, false)
  check('[变异] 改回 `|| 14` → falsy 判据翻面', /\|\|/.test(mutatedGuard), true)
}

// ── 4. 自证有效：改一个常数必须被抓到（否则本脚本是空转的）───────────────────
console.log('\n自证有效：把模板里 6.5 改成 6.6，对拍必须失败')
const PERTURB = /6\.5\*Math\.sin\(TAU\*0\.47\*t\)/
check('能在模板源码里定位到 6.5 这个常数', PERTURB.test(html), true)
if (PERTURB.test(html)) {
  const broken = html.replace(PERTURB, '6.6*Math.sin(TAU*0.47*t)')
  const brokenSway = swayOf(broken, 'index.html(perturbed)')
  const same = [0, 1.5, 4.25].every((t) => near(brokenSway(t, 1).dx, engSway(t, 1).dx))
  check('被改的模板副本与 engine 不再一致', same, false)
}

console.log(failed ? `\n${failed} 项失败` : '\n全部通过')
process.exit(failed ? 1 : 0)
