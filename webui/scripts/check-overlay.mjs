// 覆盖层（overlay）色档契约的漂移检查。
//
// 同一条规则有四份实现：Rust `vtrim-render-core/src/timeline.rs::overlay_stops`、
// 出片权威路径 `crates/vtrim-polish/templates/index.html`、预览求值器
// `webui/src/polish/engine.js`、以及经 engine 的 `PolishPreview.vue`。
// 这几份一旦漂移，表现就是"面板里设了颜色、画面上什么都没有"——正是这次要修的 bug。
//
// 运行： node webui/scripts/check-overlay.mjs
//
// 为什么不写成 `node --test`：webui/package.json 没有 "type": "module"，Node 会把
// engine.js 当 CJS，直接 import 会报 "Cannot use import statement outside a module"。
// 这里用 data-URL 动态 import 绕开（engine.js 无任何相对 import，可以整体当 ESM 求值），
// 既不动 package.json 的模块类型，也不需要任何依赖。
//
// Rust 侧那份由 `cargo test -p vtrim-render-core --lib` 覆盖，本脚本只管两份 JS。

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

const htmlPath = join(root, 'crates', 'vtrim-polish', 'templates', 'index.html')
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
const engine = await import(
  'data:text/javascript;base64,' + Buffer.from(engineSrc, 'utf8').toString('base64')
)

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

console.log(failed ? `\n${failed} 项失败` : '\n全部通过')
process.exit(failed ? 1 : 0)
