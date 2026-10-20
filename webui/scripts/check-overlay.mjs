// 覆盖层（overlay）色档契约的漂移检查。
//
// 同一条规则有四份实现：Rust `yekitrim-render-core/src/timeline.rs::overlay_stops`、
// 出片权威路径 `crates/yekitrim-polish/templates/index.html`、预览求值器
// `webui/src/polish/engine.js`、以及经 engine 的 `PolishPreview.vue`。
// 这几份一旦漂移，表现就是"面板里设了颜色、画面上什么都没有"——正是这次要修的 bug。
//
// 运行： node webui/scripts/check-overlay.mjs
//
// 为什么不写成 `node --test` / 不 import engine.js：webui/package.json 没有
// `"type": "module"`，Node 会把 engine.js 当 CJS。这里原本用 data-URL 动态 import
// 绕开，但 engine.js 现在有 `import { canvasOf, … } from './canvas.js'`（画布 SSOT），
// data-URL 没有可解析的相对基准，会直接 ERR_UNSUPPORTED_RESOLVE_REQUEST —— 于是
// 整个脚本在跑任何检查之前就崩了（"漂移防线"静默失效）。
// 改成**从源码文本里抠函数**（与 check-caption-wrap.mjs 同一套配对括号提取），
// 不加载模块、零依赖、也不动模块类型。
//
// Rust 侧那份由 `cargo test -p yekitrim-render-core --lib` 覆盖，本脚本只管两份 JS。

import { readFile } from 'node:fs/promises'
import { fileURLToPath } from 'node:url'
import { dirname, join } from 'node:path'

const here = dirname(fileURLToPath(import.meta.url))
const root = join(here, '..', '..')

// ── 从 templates/index.html 里抠出三个函数（顶层声明，用配对括号取整段）──────────
function extractFunction(src, name) {
  const start = src.indexOf(`function ${name}(`)
  if (start < 0) throw new Error(`index.html 里找不到 function ${name}(`)
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
const tpl = new Function(
  `${extractFunction(html, 'overlayFadeStop')}
   ${extractFunction(html, 'overlayStops')}
   ${extractFunction(html, 'overlayGradientCss')}
   ${extractFunction(html, 'getOverlayState')}
   return { overlayFadeStop, overlayStops, overlayGradientCss, getOverlayState }`
)()

const enginePath = join(root, 'webui', 'src', 'polish', 'engine.js')
const engineSrc = await readFile(enginePath, 'utf8')
// engine.js 的函数是 `export function`，extractFunction 从 `function name(` 起取，
// 正好把 `export ` 前缀留在外面（`new Function` 里不能出现 export）。
const engine = new Function(
  `${extractFunction(engineSrc, 'overlayFadeStop')}
   ${extractFunction(engineSrc, 'overlayStops')}
   ${extractFunction(engineSrc, 'gradientCss')}
   ${extractFunction(engineSrc, 'getOverlayState')}
   return { overlayFadeStop, overlayStops, gradientCss, getOverlayState }`
)()

// ── 断言脚手架 ────────────────────────────────────────────────────────────
let failed = 0
const eq = (a, b) => JSON.stringify(a) === JSON.stringify(b)

function check(label, actual, expected) {
  if (eq(actual, expected)) {
    console.log(`  ok   ${label}`)
  } else {
    failed++
    console.log(`  FAIL ${label}`)
    console.log(`         实得: ${JSON.stringify(actual)}`)
    console.log(`         期望: ${JSON.stringify(expected)}`)
  }
}

// 事件统一 time=0 / end=3.0，查询 t=1.5：落在 0.4s 淡入淡出之外，opacity 原样保留
const idx = (evs) => ({ overlay: evs.map((ev) => ({ time: 0, duration: 3, ev })) })
const tplState = (evs) => tpl.getOverlayState(1.5, idx(evs).overlay)
const engState = (evs) => engine.getOverlayState(idx(evs), 1.5)

const CASES = [
  {
    name: '渐变 + 只有 color（真实工程 8/8 都是这个形态）',
    evs: [{ overlay_type: 'gradient', color: '#ff0000', opacity: 0.3 }],
    want: { type: 'gradient', colors: ['#ff0000', 'rgba(255,0,0,0)'], shape: 'linear', opacity: 0.3 },
  },
  {
    name: '渐变 + 单档 colors（不补档就是 0/0 = NaN）',
    evs: [{ overlay_type: 'gradient', colors: ['#00ff00'], opacity: 0.5 }],
    want: { type: 'gradient', colors: ['#00ff00', 'rgba(0,255,0,0)'], shape: 'linear', opacity: 0.5 },
  },
  {
    name: '渐变 + 三档（参考 demo 那种，原样保留）',
    evs: [{ overlay_type: 'gradient', colors: ['#140A28CC', '#28143CCC', '#140A2888'], opacity: 1 }],
    want: { type: 'gradient', colors: ['#140A28CC', '#28143CCC', '#140A2888'], shape: 'linear', opacity: 1 },
  },
  {
    name: '渐变 + 空白色档（空串会让 addColorStop 抛 SyntaxError）',
    evs: [{ overlay_type: 'gradient', colors: ['#ff0000', '', '  '], opacity: 0.5 }],
    want: { type: 'gradient', colors: ['#ff0000', 'rgba(255,0,0,0)'], shape: 'linear', opacity: 0.5 },
  },
  {
    name: '渐变 + 既无 colors 也无 color',
    evs: [{ overlay_type: 'gradient', opacity: 0.5 }],
    want: null,
  },
  {
    name: '渐变 + 径向形状',
    evs: [{ overlay_type: 'gradient', color: '#0033ff', shape: 'radial', opacity: 0.25 }],
    want: { type: 'gradient', colors: ['#0033ff', 'rgba(0,51,255,0)'], shape: 'radial', opacity: 0.25 },
  },
  {
    name: '纯色 + color',
    evs: [{ overlay_type: 'color', color: '#123456', opacity: 0.4 }],
    want: { type: 'color', color: '#123456', opacity: 0.4 },
  },
  {
    name: '纯色 + 只有 colors[0]（类型从渐变切到纯色不该把颜色弄丢）',
    evs: [{ overlay_type: 'color', colors: ['#00ff00', '#000000'], opacity: 0.4 }],
    want: { type: 'color', color: '#00ff00', opacity: 0.4 },
  },
  {
    name: '不透明度 0 必须保持 0（`opacity || 1` 会把它变成全不透明）',
    evs: [{ overlay_type: 'color', color: '#ffffff', opacity: 0 }],
    want: null,
  },
  {
    name: '窗口里前一条类型不认识时，要接着看下一条而不是整窗放弃',
    evs: [
      { overlay_type: 'love', color: '#ff0000', opacity: 1 },
      { overlay_type: 'gradient', color: '#0033ff', opacity: 1 },
    ],
    want: { type: 'gradient', colors: ['#0033ff', 'rgba(0,51,255,0)'], shape: 'linear', opacity: 1 },
  },
]

console.log('覆盖层求值器对拍（templates/index.html ↔ engine.js）')
for (const c of CASES) {
  check(`[template] ${c.name}`, tplState(c.evs), c.want)
  check(`[engine  ] ${c.name}`, engState(c.evs), c.want)
}

// ── 淡入 / 淡出：**整窗就是过渡本身**，不叠 0.4s 进出包络 ──────────────────────
// 语义来源：把内建片头黑场（`intro-black`，四处硬编码、没有任何手动入口）换成可手改的
// overlay 事件。窗口起点必须**全遮**（=1）；一旦被 0.4s 包络乘一次就变成 0 ——
// 黑场打不开，用户看到的正是「开头渐入没生效」。这组用例就是钉死这一条的。
const tplAt = (evs, t, dur) => tpl.getOverlayState(t, evs.map((ev) => ({ time: 0, duration: dur, ev })))
const engAt = (evs, t, dur) => engine.getOverlayState({ overlay: evs.map((ev) => ({ time: 0, duration: dur, ev })) }, t)

const BLACK = (opacity) => ({ type: 'color', color: '#000000', opacity })
const FADE_CASES = [
  // 窗长 4s 取 0/1/2/3 四个点：斜坡值 1 / 0.75 / 0.5 / 0.25 在二进制里精确，能直接比相等
  { name: '淡入 t=0 全遮（叠了包络这里会变成 0）', evs: [{ overlay_type: 'fade_in', color: '#000000' }], t: 0, dur: 4, want: BLACK(1) },
  { name: '淡入 t=1', evs: [{ overlay_type: 'fade_in', color: '#000000' }], t: 1, dur: 4, want: BLACK(0.75) },
  { name: '淡入 t=2 中点', evs: [{ overlay_type: 'fade_in', color: '#000000' }], t: 2, dur: 4, want: BLACK(0.5) },
  { name: '淡入 t=3', evs: [{ overlay_type: 'fade_in', color: '#000000' }], t: 3, dur: 4, want: BLACK(0.25) },
  { name: '淡入 t=4 窗口外 → 什么都不画', evs: [{ overlay_type: 'fade_in', color: '#000000' }], t: 4, dur: 4, want: null },
  { name: '淡出 t=0 全透 → 什么都不画', evs: [{ overlay_type: 'fade_out', color: '#000000' }], t: 0, dur: 4, want: null },
  { name: '淡出 t=1（淡入的镜像）', evs: [{ overlay_type: 'fade_out', color: '#000000' }], t: 1, dur: 4, want: BLACK(0.25) },
  { name: '淡出 t=2 中点', evs: [{ overlay_type: 'fade_out', color: '#000000' }], t: 2, dur: 4, want: BLACK(0.5) },
  { name: '淡出 t=3', evs: [{ overlay_type: 'fade_out', color: '#000000' }], t: 3, dur: 4, want: BLACK(0.75) },
  // 短窗（0.4s = 包络的长度）：包络同样不许叠 —— t=0 必须仍是全遮，t=0.2 恰好一半
  { name: '淡入 0.4s 短窗 t=0 仍全遮', evs: [{ overlay_type: 'fade_in' }], t: 0, dur: 0.4, want: BLACK(1) },
  { name: '淡入 0.4s 短窗 t=0.2', evs: [{ overlay_type: 'fade_in' }], t: 0.2, dur: 0.4, want: BLACK(0.5) },
  // opacity = 遮罩的**最大**不透明度，乘在斜坡上（0 是合法值，不该被 `|| 1` 抬成全遮）
  { name: '淡入 + opacity 0.6（t=0）', evs: [{ overlay_type: 'fade_in', color: '#101010', opacity: 0.6 }], t: 0, dur: 4,
    want: { type: 'color', color: '#101010', opacity: 0.6 } },
  { name: '淡入 + opacity 0.6（t=2 中点）', evs: [{ overlay_type: 'fade_in', color: '#101010', opacity: 0.6 }], t: 2, dur: 4,
    want: { type: 'color', color: '#101010', opacity: 0.3 } },
  { name: '淡入 + opacity 0 → 整窗不出画', evs: [{ overlay_type: 'fade_in', opacity: 0 }], t: 0, dur: 4, want: null },
  { name: '淡入 不写颜色 → 缺省黑', evs: [{ overlay_type: 'fade_in' }], t: 0, dur: 4, want: BLACK(1) },
  { name: '淡入 只有色档 → 取第一档当遮罩色', evs: [{ overlay_type: 'fade_in', colors: ['#203040', '#203040'] }], t: 0, dur: 4,
    want: { type: 'color', color: '#203040', opacity: 1 } },
  { name: '淡入 排在认不出的类型后面也照样生效', evs: [{ overlay_type: 'love', color: '#ff0000', opacity: 1 }, { overlay_type: 'fade_in' }], t: 0, dur: 4,
    want: BLACK(1) },
]

console.log('\n淡入 / 淡出（整窗斜坡，不叠 0.4s 包络）')
for (const c of FADE_CASES) {
  check(`[template] ${c.name}`, tplAt(c.evs, c.t, c.dur), c.want)
  check(`[engine  ] ${c.name}`, engAt(c.evs, c.t, c.dur), c.want)
}


// ── hue（2026-10-20 由退役的 `hue_shift` 并入覆盖层）────────────────────────
//
// 曲线必须与老 `getHueShiftFlash` **逐点相同**：峰位 R = 0.15/0.95、峰值 0.25。
// 期望值由**老公式**算（legacyHue），不用实现里的表达式 —— 否则等于自己证明自己。
const HUE_DUR = 0.95
const legacyHue = (dt) => (dt < 0.15 ? (0.25 * dt) / 0.15 : 0.25 * (1 - (dt - 0.15) / 0.8))

const hueIdxTemplate = (evs) => ({ overlay: evs.map((ev) => ({ time: 0, duration: HUE_DUR, ev })) }).overlay
const hueState = (which, evs, t) =>
  which === 'template' ? tpl.getOverlayState(t, hueIdxTemplate(evs)) : engine.getOverlayState({ overlay: hueIdxTemplate(evs) }, t)

const HUE_CASES = [0.0, 0.05, 0.15, 0.30, 0.60, 0.90].map((dt) => ({
  name: `hue t=${dt}`,
  evs: [{ overlay_type: 'hue', color: '#ff0040', end: HUE_DUR }],
  t: dt,
  want: { type: 'color', color: '#ff0040', opacity: legacyHue(dt) },
}))
HUE_CASES.push({
  name: 'hue t=0.95 右边界（开区间）→ 不画',
  evs: [{ overlay_type: 'hue', color: '#ff0040', end: HUE_DUR }],
  t: HUE_DUR,
  want: null,
})
HUE_CASES.push({
  name: 'hue 同窗两条覆盖层 ⇒ 只画第一条（overlay 的既有语义）',
  // 第一条是 color 档：t=0.2 落在 0.4s 淡入内 ⇒ opacity = 1 * 0.2/0.4 = 0.5
  evs: [
    { overlay_type: 'color', color: '#111111', end: HUE_DUR },
    { overlay_type: 'hue', color: '#ff0040', end: HUE_DUR },
  ],
  t: 0.2,
  want: { type: 'color', color: '#111111', opacity: 0.5 },
})

/** 带数值容差的比较：hue 的期望值由老公式算，浮点末位可能差 1 ulp。 */
function checkNear(label, actual, expected, eps = 1e-9) {
  const close = (a, b) => {
    if (typeof a === 'number' && typeof b === 'number') return Math.abs(a - b) <= eps
    if (a && b && typeof a === 'object' && typeof b === 'object') {
      const ka = Object.keys(a).sort(), kb = Object.keys(b).sort()
      if (ka.join() !== kb.join()) return false
      return ka.every((k) => close(a[k], b[k]))
    }
    return a === b
  }
  if (close(actual, expected)) {
    console.log(`  ok   ${label}`)
  } else {
    failed++
    console.log(`  FAIL ${label}`)
    console.log(`         实得: ${JSON.stringify(actual)}`)
    console.log(`         期望: ${JSON.stringify(expected)}`)
  }
}

console.log('\nhue（并入覆盖层；与老 hue_shift 逐点相等）')
for (const c of HUE_CASES) {
  checkNear(`[template] ${c.name}`, hueState('template', c.evs, c.t), c.want)
  checkNear(`[engine  ] ${c.name}`, hueState('engine', c.evs, c.t), c.want)
}

console.log('\n色档 CSS（DOM 预览路径）')
// 收尾 ')' 曾经漏掉，整条 background 非法被浏览器丢弃——这是"设了色档却什么都看不见"的直接原因
const LIN = 'linear-gradient(to bottom right, #ff0000 0.0%,rgba(255,0,0,0) 100.0%)'
const RAD = 'radial-gradient(circle at center, #ff0000 0.0%,rgba(255,0,0,0) 100.0%)'
check('[template] 线性 CSS 完整闭合', tpl.overlayGradientCss(['#ff0000', 'rgba(255,0,0,0)'], 'linear'), LIN)
check('[template] 径向 CSS 完整闭合', tpl.overlayGradientCss(['#ff0000', 'rgba(255,0,0,0)'], 'radial'), RAD)
check('[engine  ] 线性 CSS 完整闭合', engine.gradientCss(['#ff0000', 'rgba(255,0,0,0)'], 'linear'), LIN)
check('[engine  ] 径向 CSS 完整闭合', engine.gradientCss(['#ff0000', 'rgba(255,0,0,0)'], 'radial'), RAD)
check('[engine  ] 三档按 0/50/100% 铺开', engine.gradientCss(['#a', '#b', '#c'], 'linear'),
  'linear-gradient(to bottom right, #a 0.0%,#b 50.0%,#c 100.0%)')

console.log('\n结构断言')
check('[engine  ] 单色 → 两档且第二档是同色零透明', engine.overlayStops({ color: '#ff0000' }), ['#ff0000', 'rgba(255,0,0,0)'])
check('[template] 单色 → 两档且第二档是同色零透明', tpl.overlayStops({ color: '#ff0000' }), ['#ff0000', 'rgba(255,0,0,0)'])
check('[engine  ] 无颜色 → null', engine.overlayStops({}), null)
check('[template] 无颜色 → null', tpl.overlayStops({}), null)

// 第二档必须是**同色零透明**：canvas 在非预乘空间插值，`transparent`(=rgba(0,0,0,0))
// 会把红色渐变插成暗红（实测平均 R 184.12→178.82，而同色零透明是 →193.66）。
// CSS 那边两者等价，所以用 transparent 只会让预览与出片不一致。
console.log('\n第二档：同色零透明（不是 CSS 的 transparent）')
for (const [label, fn] of [['engine  ', engine.overlayFadeStop], ['template', tpl.overlayFadeStop]]) {
  check(`[${label}] #ff0000`, fn('#ff0000'), 'rgba(255,0,0,0)')
  check(`[${label}] #f00（缩写）`, fn('#f00'), 'rgba(255,0,0,0)')
  check(`[${label}] #12345678（带 alpha）`, fn('#12345678'), 'rgba(18,52,86,0)')
  check(`[${label}] rgba(255,50,50,0.3)`, fn('rgba(255,50,50,0.3)'), 'rgba(255,50,50,0)')
  check(`[${label}] 命名色退回 transparent`, fn('rebeccapurple'), 'transparent')
}

// LOVE 分支只有页内预览支持过（canvas/engine/Rust 都没有），已删；模板里不该再有那套飘心元素
check('[template] 已删除只有预览支持的 love 飘心分支', /lov_/.test(html), false)

// ── 内建片头黑场已退役 ────────────────────────────────────────────────────
// `intro-black` 原来是四条硬编码路径上的内建表现（DOM 黑幕 + GSAP 时间轴 / canvas 消费段 /
// engine.js 求值器 / Rust `get_intro_black`），没有任何手动入口 —— 用户要的是「能改时长、能删」，
// 所以整条退役成 overlay 的 `fade_in` / `fade_out` 两档。这里挡住"某个实现偷偷把它加回来"。
const previewSrc = await readFile(join(root, 'webui', 'src', 'components', 'PolishPreview.vue'), 'utf8')
console.log('\n内建片头黑场已退役（改成了 overlay 的 fade_in / fade_out）')
check('[template] index.html 无 intro-black（CSS/DOM/GSAP/求值器全算）', /intro[-_]?black/i.test(html), false)
check('[engine  ] engine.js 无 introBlack', /introBlack/i.test(engineSrc), false)
check('[preview ] PolishPreview.vue 无 introBlack', /introBlack/i.test(previewSrc), false)


// ── hue 变异自证（--self-test）：**真改源码 → 重抠函数 → 跑同一份 HUE_CASES** ──────────
//
// 口径与 `check-yekitrim-translator --mutate` 同源：① `replace` 前先断言**命中恰好 1 处**
// （没命中 = 变异没生效，不许静默）；② 变异体加载后**先跑一遍未变异对照**，确认对照仍绿
// （否则"翻面"可能只是加载坏了）；③ 变异体必须**至少一条翻面**。
if (process.argv.includes('--self-test')) {
  console.log('\nhue 变异自证（真改源码）')
  const near = (a, b, eps = 1e-9) => {
    if (typeof a === 'number' && typeof b === 'number') return Math.abs(a - b) <= eps
    if (a && b && typeof a === 'object' && typeof b === 'object') {
      const ka = Object.keys(a).sort(), kb = Object.keys(b).sort()
      if (ka.join() !== kb.join()) return false
      return ka.every((k) => near(a[k], b[k], eps))
    }
    return a === b
  }
  const makeTpl = (src) =>
    new Function(`${extractFunction(src, 'overlayFadeStop')}
${extractFunction(src, 'overlayStops')}
${extractFunction(src, 'overlayGradientCss')}
${extractFunction(src, 'getOverlayState')}
return { getOverlayState }`)()
  const makeEng = (src) =>
    new Function(`${extractFunction(src, 'overlayFadeStop')}
${extractFunction(src, 'overlayStops')}
${extractFunction(src, 'gradientCss')}
${extractFunction(src, 'getOverlayState')}
return { getOverlayState }`)()
  const mismatchCount = (api, kind) => {
    let bad = 0
    for (const c of HUE_CASES) {
      const items = hueIdxTemplate(c.evs)
      const got = kind === 'template' ? api.getOverlayState(c.t, items) : api.getOverlayState({ overlay: items }, c.t)
      if (!near(got, c.want)) bad++
    }
    return bad
  }
  const replaceOnce = (src, from, to, label) => {
    const parts = src.split(from)
    if (parts.length !== 2) {
      failed++
      console.log(`  FAIL [变异] ${label} → 替换命中 ${parts.length - 1} 处（必须恰好 1 处）`)
      return null
    }
    console.log(`  ok   [变异] ${label} → 替换命中 1 处`)
    return parts.join(to)
  }
  const controlTpl = makeTpl(html)
  const controlEng = makeEng(engineSrc)
  const baseTplBad = mismatchCount(controlTpl, 'template')
  const baseEngBad = mismatchCount(controlEng, 'engine')
  check('自证 · 未变异对照（模板）仍全绿', baseTplBad, 0)
  check('自证 · 未变异对照（engine）仍全绿', baseEngBad, 0)

  const mutTplSrc = replaceOnce(html, 'var HR = 0.15 / 0.95;', 'var HR = 0.20 / 0.95;', '模板峰位 0.15 → 0.20')
  if (mutTplSrc) {
    const bad = mismatchCount(makeTpl(mutTplSrc), 'template')
    check('自证 · 模板变异体必须翻面（≥1 条不等）', bad > 0, true)
  }
  const mutEngSrc = replaceOnce(engineSrc, 'const R = 0.15 / 0.95', 'const R = 0.20 / 0.95', 'engine 峰位 0.15 → 0.20')
  if (mutEngSrc) {
    const bad = mismatchCount(makeEng(mutEngSrc), 'engine')
    check('自证 · engine 变异体必须翻面（≥1 条不等）', bad > 0, true)
  }
}

console.log(failed ? `\n${failed} 项失败` : '\n全部通过')
process.exit(failed ? 1 : 0)
