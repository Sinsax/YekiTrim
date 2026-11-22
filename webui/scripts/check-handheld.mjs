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
  var CFG = { vertical: false, lanes: { camera: true }, style: { camera: {} } };
  // 模板顶层那份 laneOn 的等价物（判据只有一处：Rust 注入的 CFG.lanes）
  function laneOn(lane) {
    var v = CFG.lanes ? CFG.lanes[lane] : undefined;
    return typeof v === 'boolean' ? v : true;
  }
`
// ⚠️ **两处都要 `cameraTransitionSecs`**（运镜速度 §16）：这个对拍台是把**函数体从源码里抠出来
// eval** 的，被调用的兄弟函数不会自动跟进来 ⇒ 不显式注入就是
// `ReferenceError: cameraTransitionSecs is not defined`（实测：整套门禁在这里炸）。
// ⚠️ 2026-11-21 起**模板也有这一份**（在那之前它写死 `min(dur*0.5, 0.6)`，是唯一不读 speed 的实现）。
// 所以两边**各抠各的**、不共用同一份 —— 这样这条门禁顺带把"两份速度表/两套夹取"也钉在一起。
// ⚠️ `CAMERA_SPEED_SECS` 是**变量字面量**、不是 `function` 声明 ⇒ 不能走 `extractFunction`
// （它会抛"找不到 function CAMERA_SPEED_SECS("）—— 这里按变量声明抓。
const speedTableOf = (src) => {
  const m = /(?:export\s+)?(?:const|var)\s+CAMERA_SPEED_SECS\s*=\s*\{([^}]*)\}/.exec(src)
  if (!m) throw new Error('找不到 CAMERA_SPEED_SECS 的字面量表')
  return new Function(`return {${m[1]}}`)()
}
const TPL_SPEED = speedTableOf(html)
const ENG_SPEED = speedTableOf(engineSrc)
const tplCam = new Function(
  `${STUB}\nvar CAMERA_SPEED_SECS = ${JSON.stringify(TPL_SPEED)};\n` +
    `${extractFunction(html, 'cameraTransitionSecs')}\n${extractFunction(html, 'getActiveCamera')}\n return getActiveCamera`,
)()
const engCam = new Function(
  `${STUB}\nvar CAMERA_SPEED_SECS = ${JSON.stringify(ENG_SPEED)};\n` +
    `${extractFunction(engineSrc, 'cameraTransitionSecs')}\n${extractFunction(engineSrc, 'cameraAt')}\n return cameraAt`,
)()
/** 从一个**被扰动过**的模板副本里抠 `getActiveCamera`（速度档位表 + 接缝时长一起注入）。 */
const tplCamOf = (stub, src) =>
  new Function(
    `${stub}\nvar CAMERA_SPEED_SECS = ${JSON.stringify(speedTableOf(src))};\n` +
      `${extractFunction(src, 'cameraTransitionSecs')}\n${extractFunction(src, 'getActiveCamera')}\n return getActiveCamera`,
  )()

// 事件序列：近景开摇摆 → 近景续 → 硬切收 → 远景（硬切，另一端瞬变）→ 中景收
const CAMS = [
  { time: 0.0, duration: 4.0, ev: { zoom: '近', handheld: true } },
  { time: 6.0, duration: 4.0, ev: { zoom: '近', handheld: true } },
  { time: 10.0, duration: 0.4, ev: { zoom: '中', handheld: false, cut: true } },
  { time: 13.0, duration: 3.0, ev: { zoom: '远', handheld: true } },
  { time: 17.0, duration: 3.0, ev: { zoom: '中', handheld: false } },
]
const engIdx = { vertical: false, cameraOff: false, camera: CAMS, camStyle: {}, cv: { W: 1920, H: 1080 }, layout: {} }

console.log('\n起停状态机对拍（幅度曲线 + 景别，逐采样点）')
// ── 3b. **位置状态机**（2026-11-21）：接缝过渡 + 块内位移，模板 ↔ engine 逐值对拍 ──
//
// 钉住的是一个实测事故：位置曾经是**每块绝对赋值**（`ox = lerp(本块起, 本块终, p)`），
// `p=0` 时直接等于本块起点、跟上一块落在哪儿无关 ⇒ A 块移到弹幕(16.67%) 之后接一条
// 「起点=主播」(65%) 的块，**一帧横移 928px（画面宽 48%）**、没有过渡。
// 同一根因还让「硬切 + 终点」把起点整个吞掉（p=1 直接落到终点）。
// 现在位置与 zoom 同形：从**当前**出发，接缝把它接到本块起点，块内再从起点走到终点。
const CAMS_POS = [
  { time: 0.0, duration: 4.0, ev: { zoom: '中', origin: '65% 50%', to_origin: '16.67% 16.67%' } },
  { time: 4.0, duration: 4.0, ev: { zoom: '中', origin: '65% 50%' } },
  { time: 8.0, duration: 4.0, ev: { zoom: '近', origin: '50% 50%', cut: true } },
  { time: 12.0, duration: 4.0, ev: { zoom: '中', origin: '16.67% 16.67%', to_origin: '65% 50%', speed: 'fast' } },
  { time: 16.0, duration: 4.0, ev: { zoom: '远', origin: '65% 50%' } },
]
const posIdx = { vertical: false, cameraOff: false, camera: CAMS_POS, camStyle: {}, cv: { W: 1920, H: 1080 }, layout: {} }

console.log('\n位置状态机对拍（接缝 + 块内位移，模板 vs engine，逐采样点）')
let mismatch = 0
for (let t = 0; t <= 20.0001; t += 0.05) {
  const a = tplCam(t, CAMS_POS)
  const b = engCam(posIdx, t)
  const same = Math.abs(a.ox - b.ox) < 1e-6 && Math.abs(a.oy - b.oy) < 1e-6 && Math.abs(a.zoom - b.zoom) < 1e-9
  if (!same) {
    mismatch += 1
    if (mismatch <= 3) console.log(`         t=${t.toFixed(2)} template=${a.ox.toFixed(2)},${a.oy.toFixed(2)} engine=${b.ox.toFixed(2)},${b.oy.toFixed(2)}`)
  }
}
check('[对拍] 位置/景别逐采样点一致（模板 canvas = engine 预览）', mismatch, 0)

// **连续性**：块缝处不许瞬跳。取样步长 5ms，最大单步位移必须远小于一次跳变
// （实测旧实现的跳变是 928px；这里阈 60px 仍留了 15 倍余量）。
console.log('\n块缝连续性（5ms 步长，不许出现瞬跳）')
{
  // ⚠️ **硬切块例外**：cut 的定义就是「瞬间到位」，那一步本来就该跳（本例 288px）。
  //    连续性判据只约束**平滑**的接缝与块内位移。
  const cutTimes = CAMS_POS.filter((c) => c.ev.cut === true).map((c) => c.time)
  let worst = 0, worstAt = 0, prev = null, prevT = 0
  for (let t = 0; t <= 20.0001; t += 0.005) {
    const c = tplCam(t, CAMS_POS)
    const straddlesCut = cutTimes.some((ct) => ct >= prevT - 1e-9 && ct < t + 1e-9)
    if (prev && !straddlesCut) {
      const d = Math.hypot(c.ox - prev.ox, c.oy - prev.oy)
      if (d > worst) { worst = d; worstAt = t }
    }
    prev = c
    prevT = t
  }
  check(`单步最大位移 ${worst.toFixed(1)}px @ t=${worstAt.toFixed(2)}（阈 60px；旧实现是 928px 一帧）`, worst < 60, true)
  // 起止两端必须真的落在事件写的位置上（别为了平滑把端点也糊掉）
  const c0 = tplCam(0.0, CAMS_POS)
  check('[端点] t=0 落在 65% 50%（片头起点）', Math.round(c0.ox) === 1248 && Math.round(c0.oy) === 540, true)
  const c4 = tplCam(3.999, CAMS_POS)
  check('[端点] 第一块块尾到达终点 16.67%', Math.abs(c4.ox - 320.06) < 2, true)
  const c8 = tplCam(8.0, CAMS_POS)
  check('[端点] 硬切块起点直接到位 50% 50%（旧实现会直接落到终点）', Math.round(c8.ox) === 960 && Math.round(c8.oy) === 540, true)
}

// 自证有效：把模板的位置公式退回**绝对赋值**（旧 bug），上面两条必须翻面
{
  const brokenHtml = html.replace('ox = lerp(ox, ax, p);', 'ox = lerp(ax, bx, p);')
  check('[变异前提] 能定位到接缝那一行（ox = lerp(ox, ax, p)）', brokenHtml !== html, true)
  if (brokenHtml !== html) {
    const brokenCam = tplCamOf(STUB, brokenHtml)
    let jump = 0, last = null
    for (let t = 0; t <= 20.0001; t += 0.005) {
      const c = brokenCam(t, CAMS_POS)
      if (last) jump = Math.max(jump, Math.hypot(c.ox - last.ox, c.oy - last.oy))
      last = c
    }
    check(`[变异] 退回绝对赋值 ⇒ 出现瞬跳（${jump.toFixed(0)}px）`, jump > 100, true)
  }
}
for (const t of [0, 0.3, 0.6, 2, 5.9, 6.0, 6.3, 6.6, 9.9, 10.0, 10.2, 12.9, 13.0, 13.5, 16.9, 17.0, 17.3, 18.0, 25.0, 40.0]) {
  const a = tplCam(t, CAMS)
  const b = engCam(engIdx, t)
  check(
    `[对拍] t=${t} 幅度/景别`,
    { handheld: round(a.handheld), zoom: round(a.zoom), ox: round(a.ox, 3), oy: round(a.oy, 3) },
    { handheld: round(b.handheld), zoom: round(b.zoom), ox: round(b.ox, 3), oy: round(b.oy, 3) }
  )
}
// 运镜门 = **运镜轨的眼睛**（2026-10-06；模板读 `CFG.lanes.camera`，engine 读 `idx.cameraOff`，
// 唯一判据在 Rust `ProjectFile::lane_enabled`）。用户报的「竖屏素材没禁运镜、横屏导出也还在缩放」
// 就是这条闸：模板曾只归零 handheld、zoom 照跑 —— 摘掉它 zoom 立刻回到 1.5。
// ⚠️ 门读的是**轨道开关**，不是素材方向（`CFG.vertical`）：竖素材进竖画布只要眼睛亮着就照常运镜。
//    下面那条 `verticalOnly` 用例钉的就是这件事。
{
  const gatedSrc = STUB.replace('vertical: false, lanes: { camera: true }', 'vertical: true, lanes: { camera: false }')
  const gatedTpl = tplCamOf(gatedSrc, html)
  const tv = gatedTpl(2.0, CAMS)
  check('[template] 竖素材进非竖画布 → 幅度归零', tv.handheld, 0)
  check('[template] 竖素材进非竖画布 → 景别也归零（zoom=1）', tv.zoom, 1)
  const idxV = { ...engIdx, vertical: true, cameraOff: true }
  const ev = engCam(idxV, 2.0)
  check('[engine  ] 竖素材进非竖画布 → 幅度归零', ev.handheld, 0)
  check('[对拍] 竖素材进非竖画布 → 模板与 engine 的 zoom / 不动点逐值一致',
    { zoom: round(tv.zoom), ox: round(tv.ox, 3), oy: round(tv.oy, 3) },
    { zoom: round(ev.zoom), ox: round(ev.ox, 3), oy: round(ev.oy, 3) })

  // ⚠️ **竖素材 + 眼睛亮着 ⇒ 照样有运镜**（`vertical` 一点作用都没有）——
  //    这条把「模板读的是门、不是源方向」钉死：改回 `CFG.vertical` 立刻翻面。
  const verticalOnlySrc = STUB.replace('vertical: false, lanes: { camera: true }', 'vertical: true, lanes: { camera: true }')
  const verticalOnlyTpl = tplCamOf(verticalOnlySrc, html)
  const vo = verticalOnlyTpl(2.0, CAMS)
  check('[template] 竖素材进竖画布 → 运镜照常（门是「方向不一致」，不是「源竖」）', vo.zoom, 1.5)
  check('[template] 竖素材进竖画布 → 摆动也照常', vo.handheld, 1)
  const idxVO = { ...engIdx, vertical: true, cameraOff: false }
  const evo = engCam(idxVO, 2.0)
  check('[对拍] 竖素材进竖画布 → 模板与 engine 逐值一致',
    { zoom: round(vo.zoom), ox: round(vo.ox, 3), oy: round(vo.oy, 3), handheld: round(vo.handheld) },
    { zoom: round(evo.zoom), ox: round(evo.ox, 3), oy: round(evo.oy, 3), handheld: round(evo.handheld) })

  // 自证有效：把模板的运镜门摘掉 → 上面两条必须翻面（CAMS 在 t=2.0 是「近」= 1.5、摇摆开着）
  const noGate = html.replace("if (!laneOn('camera')) return { zoom: 1.0,", 'if (false) return { zoom: 1.0,')
  check('[变异] 运镜门确实被摘掉（源码被改了）', noGate !== html, true)
  if (noGate !== html) {
    const tplNoGate = tplCamOf(gatedSrc, noGate)
    check('[变异] 摘掉运镜门 → zoom 判据翻面（回到 1.5）', tplNoGate(2.0, CAMS).zoom, 1.5)
    check('[变异] 摘掉运镜门 → 幅度判据翻面（回到 1）', tplNoGate(2.0, CAMS).handheld, 1)
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
