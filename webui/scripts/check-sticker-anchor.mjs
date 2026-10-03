// 贴纸「尺寸 + 落点」契约的漂移检查。
//
// 同一条规则有多份实现，任何一份漂移都表现为「预览里点哪儿、成片里贴到别处」或
// 「竖屏贴纸小一圈」：
//   1. 出片权威路径 `crates/yekitrim-polish/templates/index.html`
//      画布倍率 `CK/CKH/SK` → `stickerAnchor` → `stickerBox` → `handlers.sticker`
//      （DOM 落点）+ `getActiveSticker`（canvas 出片取中心）
//   2. 预览求值器 `webui/src/polish/engine.js`
//      `stickerAnchor` → `clampSticker` → `stickersAt`
//   3. 画布参数 SSOT `webui/src/polish/canvas.js`（`stickerScale/Size/Margin/FloatUp`）
//   4. Rust GPU 路径 `yekitrim-render-core/src/timeline.rs::parse_sticker`（先钳中心再叠 y_off）
//      —— 那份由 `cargo test -p yekitrim-render-core --lib` 覆盖，本脚本只管前三份 JS。
//   5. 编辑入口 `webui/src/views/PolishView.vue`（落点是**草稿态**：进编辑 → 点选 → 保存/取消。
//      点选回调一旦直接落盘，"看一眼预览"就会改坏配置 —— 见 ⑧ 段）
//
// 两条口径（**不同键、不同基准，不要"统一"**）：
//   · 事件 x/y（auto，placement.rs 安全区算法产出）= 盒子**左上角**百分比
//   · `[sticker] position`（manual，用户在预览上点的）= 盒子**中心**百分比
// 尺寸口径：盒子/边距/上浮一律按**短边**缩放（`sticker_scale = min(W,H)/1080`），
// 横竖画布短边都是 1080 ⇒ 恒等于 280/32/18px，不随画布缩水。
// clamp 顺序：**先钳进安全区、再叠加上浮量**（DOM 侧上浮是 GSAP transform，作用在
// 已钳制的 CSS 位置上；GPU 侧同样"先钳再叠"）。反过来写会让贴纸能顶破安全边距。
//
// 运行： node webui/scripts/check-sticker-anchor.mjs
//
// 为什么不 import engine.js：webui/package.json 没有 `"type": "module"`，Node 会把
// engine.js 当 CJS；data-URL 动态 import 又会因为 `./canvas.js` 没有相对基准而
// ERR_UNSUPPORTED_RESOLVE_REQUEST。所以这里从源码文本里抠函数来跑
// （与 check-caption-wrap.mjs / check-overlay.mjs 同一套配对括号提取）。

import { readFile } from 'node:fs/promises'
import { fileURLToPath } from 'node:url'
import { dirname, join } from 'node:path'

const here = dirname(fileURLToPath(import.meta.url))
const root = join(here, '..', '..')

// ── 源码提取 ──────────────────────────────────────────────────────────────
/** 返回 `src[i]`（必须是 `{`）匹配的 `}` 的下标 */
function braceEnd(src, i) {
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
      if (depth === 0) return i
    }
  }
  throw new Error('括号没配平')
}

function extractFunction(src, name) {
  const start = src.indexOf(`function ${name}(`)
  if (start < 0) throw new Error(`找不到 function ${name}(`)
  return src.slice(start, braceEnd(src, src.indexOf('{', start)) + 1)
}

/** 抠出 `marker` 之后那个对象字面量（用于 `export const E = { … }`） */
function extractObjectAfter(src, marker) {
  const at = src.indexOf(marker)
  if (at < 0) throw new Error(`找不到 ${marker}`)
  const s = src.indexOf('{', at + marker.length)
  return src.slice(s, braceEnd(src, s) + 1)
}

const html = await readFile(join(root, 'crates', 'yekitrim-polish', 'templates', 'index.html'), 'utf8')
const engineSrc = await readFile(join(root, 'webui', 'src', 'polish', 'engine.js'), 'utf8')
const canvasSrc = await readFile(join(root, 'webui', 'src', 'polish', 'canvas.js'), 'utf8')
const previewSrc = await readFile(join(root, 'webui', 'src', 'components', 'PolishPreview.vue'), 'utf8')
const viewSrc = await readFile(join(root, 'webui', 'src', 'views', 'PolishView.vue'), 'utf8')

// ── 断言脚手架 ────────────────────────────────────────────────────────────
let failed = 0
const eq = (a, b) => JSON.stringify(a) === JSON.stringify(b)
const near = (a, b) => typeof a === 'number' && Math.abs(a - b) < 1e-6

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
const checkNear = (label, actual, expected) => check(label, near(actual, expected), true)
/** 抹掉浮点末位差（两份实现的运算顺序不同），便于整组比对 */
const g6 = (...xs) => xs.map((v) => (typeof v === 'number' ? Math.round(v * 1e6) / 1e6 : v))
/** 与出片无关的几何量：容差 1e-6 */
const checkGeom = (label, actual, expected) => check(label, g6(...actual), g6(...expected))

// ── 1. index.html 的画布倍率（把源码那一段原样求值，不抄副本）────────────────
const canvasBlockStart = html.indexOf('var CW=1920')
if (canvasBlockStart < 0) throw new Error('index.html 里找不到 `var CW=1920`')
const canvasBlock = html.slice(canvasBlockStart, html.indexOf('})();', canvasBlockStart) + 5)

/** 用给定的 `_canvas` 求值 index.html 的倍率块 → {CW,CH,CK,CKH,SK} */
function tplScale(canvas) {
  return new Function('CFG', `${canvasBlock}
    return { CW: CW, CH: CH, CK: CK, CKH: CKH, SK: SK }`)({ _canvas: canvas })
}
/** `stickerAnchor` + `stickerBox`（注入 index.html 自己的 CW/CH/SK） */
function tplSticker(canvas) {
  const s = tplScale(canvas)
  return new Function('CFG', 'CW', 'CH', 'SK', `
    ${extractFunction(html, 'stickerAnchor')}
    ${extractFunction(html, 'stickerBox')}
    return { stickerAnchor, stickerBox }`)({ _canvas: canvas }, s.CW, s.CH, s.SK)
}

console.log('① 画布倍率：贴纸按短边缩放（SK = min(CW,CH)/1080）')
const LAND = { width: 1920, height: 1080 }
const PORT = { width: 1080, height: 1920 }
const scL = tplScale(LAND), scP = tplScale(PORT)
check('[template] 横屏 CK/CKH/SK', [scL.CK, scL.CKH, scL.SK], [1, 1, 1])
// ★ ⑥ 的钉子：竖屏 SK 必须仍是 1。老的 `280*CK` 口径在这里是 0.5625 → 盒子只有 157.5px
check('[template] 竖屏 CK/CKH', [scP.CK, scP.CKH], [0.5625, 1.7777777777777777])
check('[template] 竖屏 SK 仍为 1（贴纸不随画布缩水）', scP.SK, 1)
check('[template] 短边倍率写法在源码里（不是 SK=CK）', /SK\s*=\s*Math\.min\(CW\s*,\s*CH\)\s*\/\s*1080/.test(canvasBlock), true)
check('[template] 已无残留的 `280*CK` 贴纸尺寸', /280\s*\*\s*CK\b/.test(html), false)
check('[template] 已无残留的 `140*CK` / `-18*CK`', /140\s*\*\s*CK\b|-18\s*\*\s*CK\b/.test(html), false)

// ── 2. canvas.js（预览侧画布 SSOT）────────────────────────────────────────
const BASE_H = Number((canvasSrc.match(/const BASE_H\s*=\s*(\d+)/) || [])[1])
const BASE_W = Number((canvasSrc.match(/const BASE_W\s*=\s*(\d+)/) || [])[1])
check('[canvas.js] BASE_W/BASE_H 仍是 1920/1080', [BASE_W, BASE_H], [1920, 1080])

const cvApi = new Function('BASE_W', 'BASE_H', `
  ${extractFunction(canvasSrc, 'canvasOf')}
  ${extractFunction(canvasSrc, 'stickerScale')}
  ${extractFunction(canvasSrc, 'stickerSize')}
  ${extractFunction(canvasSrc, 'stickerMargin')}
  ${extractFunction(canvasSrc, 'stickerFloatUp')}
  return { canvasOf, stickerScale, stickerSize, stickerMargin, stickerFloatUp }`)(BASE_W, BASE_H)

console.log('\n② 贴纸几何：横竖都是 280/32/18，非 1080 短边时按短边缩放')
const geomOf = (canvas) => {
  const cv = cvApi.canvasOf({ _canvas: canvas })
  return [cvApi.stickerSize(cv), cvApi.stickerMargin(cv), cvApi.stickerFloatUp(cv)]
}
checkGeom('[canvas.js] 横屏 1920×1080 → 280/32/18', geomOf(LAND), [280, 32, 18])
checkGeom('[canvas.js] 竖屏 1080×1920 → 280/32/18', geomOf(PORT), [280, 32, 18])
// 720×1280（用户真实素材的源尺寸）：短边 720 → 0.6667 倍
checkNear('[canvas.js] 720×1280 → 短边 720 缩放', geomOf({ width: 720, height: 1280 })[0], 280 * 720 / 1080)
checkNear('[canvas.js] 1920×1040 → 短边 1040 缩放', geomOf({ width: 1920, height: 1040 })[0], 280 * 1040 / 1080)

// ── 3. engine.js 侧 ──────────────────────────────────────────────────────
function engineStickerOf(src) {
  return new Function(
    'canvasOf', 'stickerSize', 'stickerMargin', 'stickerFloatUp', 'E', 'pctPx', `
    ${extractFunction(src, 'stickerGeom')}
    ${extractFunction(src, 'stickerAnchor')}
    ${extractFunction(src, 'clampSticker')}
    ${extractFunction(src, 'stickersAt')}
    return { stickerGeom, stickerAnchor, clampSticker, stickersAt }`
  )(
    cvApi.canvasOf, cvApi.stickerSize, cvApi.stickerMargin, cvApi.stickerFloatUp,
    new Function(`return ${extractObjectAfter(engineSrc, 'const E = ')}`)(),
    new Function(`return ${extractFunction(engineSrc, 'pctPx')}`)()
  )
}
const eng = engineStickerOf(engineSrc)

/** 造一个 engine 的 idx（`stickersAt` 只用到 canvas/cv/sticker 三处） */
function engIdx(canvas, ev, time = 0, duration = 3) {
  return {
    canvas,
    cv: cvApi.canvasOf({ _canvas: canvas }),
    sticker: [{ time, duration, ev, src: 'x.png', isEmoji: false, emoji: '' }]
  }
}

// ── 4. 对拍：manual = 盒子中心 / auto = 盒子左上角 ─────────────────────────
console.log('\n③ 落点口径：manual 给中心、auto 给左上角（两份实现逐例对拍）')

/** 出片侧最终视觉位置（clamp 之后再叠上浮量 —— GSAP 的 y 是 transform） */
function tplBox(canvas, ev, mode, pos) {
  const c = mode === 'manual' ? { ...canvas, sticker: { position_mode: 'manual', position: pos } }
    : { ...canvas, sticker: { position_mode: 'auto' } }
  const s = tplScale(c), t = tplSticker(c)
  const anc = t.stickerAnchor(ev)
  const box = t.stickerBox(anc)
  return { left: box.left, top: box.top - 18 * s.SK, size: box.size, center: anc.center }
}
/** 预览侧最终位置（`stickersAt` 的返回值，hold 相位） */
function engBox(canvas, ev, mode, pos) {
  const c = mode === 'manual' ? { ...canvas, sticker: { position_mode: 'manual', position: pos } }
    : { ...canvas, sticker: { position_mode: 'auto' } }
  const r = eng.stickersAt(engIdx(c, ev), 1.5)[0]
  return { left: r.x, top: r.y, size: r.w, center: eng.stickerAnchor(engIdx(c, ev), ev).center }
}

const ANCHOR_CASES = [
  { name: 'manual 53% 48%（用户电台工程的实际值）', mode: 'manual', pos: '53% 48%', ev: {} },
  { name: 'manual 2% 2%（左上越界 → 被钳进边距）', mode: 'manual', pos: '2% 2%', ev: {} },
  { name: 'manual 98% 98%（右下越界 → 被钳进边距）', mode: 'manual', pos: '98% 98%', ev: {} },
  { name: 'manual 单值 40%（两轴同值）', mode: 'manual', pos: '40%', ev: {} },
  { name: 'manual 空白串 → 退回 50% 50%', mode: 'manual', pos: '', ev: {} },
  { name: 'auto 事件 12% 42%（实测探针值）', mode: 'auto', pos: '', ev: { x: '12%', y: '42%' } },
  { name: 'auto 事件 90% 90%（越界 → 钳）', mode: 'auto', pos: '', ev: { x: '90%', y: '90%' } },
  { name: 'auto 事件数值型 25 30（老数据不带 %）', mode: 'auto', pos: '', ev: { x: 25, y: 30 } },
]
for (const canvas of [LAND, PORT]) {
  const tag = canvas.width > canvas.height ? '横屏' : '竖屏'
  for (const c of ANCHOR_CASES) {
    const a = tplBox(canvas, c.ev, c.mode, c.pos)
    const b = engBox(canvas, c.ev, c.mode, c.pos)
    checkGeom(`[对拍·${tag}] ${c.name}`,
      [a.left, a.top, a.size, a.center], [b.left, b.top, b.size, b.center])
  }
}

console.log('\n④ manual 必须是「中心」口径（否则用户点哪儿都不在哪儿）')
// 1920×1080、盒子 280：53% → 1017.6px 中心 ⇒ 左上角 877.6（老口径会直接落在 1017.6）
checkNear('[template] manual 53% → 左上角 = 中心 - 140', tplBox(LAND, {}, 'manual', '53% 48%').left, 0.53 * 1920 - 140)
checkNear('[engine  ] manual 53% → 左上角 = 中心 - 140', engBox(LAND, {}, 'manual', '53% 48%').left, 0.53 * 1920 - 140)
// 竖屏同理：50% → 540 中心 ⇒ 左上角 400
checkNear('[template] 竖屏 manual 50% → 左上角 400', tplBox(PORT, {}, 'manual', '50% 50%').left, 400)
checkNear('[engine  ] 竖屏 manual 50% → 左上角 400', engBox(PORT, {}, 'manual', '50% 50%').left, 400)
// 与 auto 的差别正是半个盒子 —— 这条同时钉住「两个键不同基准，别统一」
const manL = tplBox(LAND, {}, 'manual', '50% 50%').left
const autL = tplBox(LAND, { x: '50%', y: '50%' }, 'auto', '').left
checkNear('[template] manual 与 auto 在同一个 50% 上差半个盒子', manL - autL, -140)
checkNear('[engine  ] manual 与 auto 在同一个 50% 上差半个盒子',
  engBox(LAND, {}, 'manual', '50% 50%').left - engBox(LAND, { x: '50%', y: '50%' }, 'auto', '').left, -140)

console.log('\n⑤ clamp 顺序：先钳进安全区，再叠加上浮量（上浮是 transform）')
// 手动 2% 2%：中心 38.4px → 左上角 -101.6 → 钳到 32；上浮 18 之后再减 ⇒ 14
checkNear('[template] 左上越界钳到边距后仍叠加上浮', tplBox(LAND, {}, 'manual', '2% 2%').top, 32 - 18)
checkNear('[engine  ] 左上越界钳到边距后仍叠加上浮', engBox(LAND, {}, 'manual', '2% 2%').top, 32 - 18)
// 若改成"先加上浮再钳"，这一条会变成 32（顶破边距 18px）——下面自证段会验证
check('[engine  ] 源码里 clamp 只吃 left/top（没有把 yOff 塞进去）',
  /clampSticker\(g,\s*left,\s*top\)/.test(engineSrc), true)

// ── 5. 结构断言（源码级，防"改了一处忘了另一处"）──────────────────────────
console.log('\n⑥ 结构断言（源码级）')
check('[template] handlers.sticker 用 stickerBox 定位', /var box=stickerBox\(stickerAnchor\(ev\)\)/.test(html), true)
check('[template] getActiveSticker 取的是盒子中心（left + S/2）',
  /x:\s*left\s*\+\s*S\s*\/\s*2,\s*y:\s*top\s*\+\s*S\s*\/\s*2\s*\+\s*yOff/.test(html), true)
check('[template] getActiveSticker 的盒子边长用 SK', /var S = 280 \* SK;/.test(html), true)
check('[engine  ] stickersAt 在钳制之后叠加 yOff', /y:\s*pos\.y\s*\+\s*yOff/.test(engineSrc), true)
check('[engine  ] stickerGeom 走 canvas.js 的贴纸倍率',
  /s:\s*stickerSize\(cv\),\s*m:\s*stickerMargin\(cv\)/.test(engineSrc), true)
check('[engine  ] 已无 `280 * cv.k` 这类贴纸尺寸', /stickerSize\(cv\)[\s\S]{0,40}?cv\.k/.test(engineSrc), false)
// 预览的点选占位框必须同口径：以锚点为中心 + 短边倍率
check('[preview ] pointBoxStyle 用短边倍率', /const size = 280 \* stickerScale\(c\)/.test(previewSrc), true)
check('[preview ] pointBoxStyle 以锚点为中心', /px - size \/ 2/.test(previewSrc), true)
check('[preview ] 点选提示点画在框中心', /\.point-dot \{[^}]*left:50%; top:50%/.test(previewSrc), true)
check('[preview ] --sticker-s 用短边倍率', /'--sticker-s': \(280 \* stickerScale\(v\)\)/.test(previewSrc), true)

// ── 6. 自证有效 ──────────────────────────────────────────────────────────
console.log('\n⑦ 自证有效：改回旧口径后，上面的断言必须失败（否则本脚本是空转的）')
// ⚠️ 仓库里这两个文件都是 CRLF，但下面没有跨行模式，所以不需要 `\r?\n`。
function mutate(src, pattern, to, label) {
  return mutateAll(src, [[pattern, to]], label)
}
/** 依次套用多处替换；任何一处定位不到都算失败（防"空转变异"） */
function mutateAll(src, edits, label) {
  let out = src
  for (const [pattern, to] of edits) {
    const re = new RegExp(pattern)
    if (!re.test(out)) {
      failed++
      console.log(`  FAIL ${label}：源码里定位不到变异点 ${pattern}（这项检查是空转的）`)
      return null
    }
    out = out.replace(re, to)
  }
  return out
}

// (a) SK 回到「宽比」口径 → 竖屏贴纸缩成 157.5
const brokenBlock = mutate(canvasBlock, 'SK\\s*=\\s*Math\\.min\\(CW\\s*,\\s*CH\\)\\s*\\/\\s*1080', 'SK=CK', '[template·变异] SK=Math.min(CW,CH)/1080')
if (brokenBlock) {
  const s = new Function('CFG', `${brokenBlock} return { SK: SK }`)({ _canvas: PORT })
  check('[template·变异] SK=CK 时竖屏倍率变成 0.5625（本脚本能抓到）', near(s.SK, 0.5625), true)
}

// (b) 去掉 manual 的中心换算 → 落点整体偏半个盒子
const brokenEngine = mutate(engineSrc, 'if \\(anc\\.center\\) \\{ left -= g\\.s / 2; top -= g\\.s / 2 \\}', '', '[engine·变异] 去掉中心换算')
if (brokenEngine) {
  const b = engineStickerOf(brokenEngine)
  const r = b.stickersAt(engIdx({ ...LAND, sticker: { position_mode: 'manual', position: '50% 50%' } }, {}), 1.5)[0]
  check('[engine·变异] 不换中心 → 落点差半个盒子（本脚本能抓到）', near(r.x, 960), true)
}

// (c) 回到「先叠加上浮、再钳制」的旧顺序 → 贴纸能顶破安全边距（新顺序 14px，旧顺序 32px）
// 只改**调用点**：`function clampSticker(g, left, top)` 的声明也要跟着改的话连语法都过不去。
const brokenOrder = mutateAll(engineSrc, [
  ['const pos = clampSticker\\(g,\\s*left,\\s*top\\)', 'const pos = clampSticker(g, left, top + yOff)'],
  ['y:\\s*pos\\.y\\s*\\+\\s*yOff', 'y: pos.y'],
], '[engine·变异] 回到「先叠再钳」')
if (brokenOrder) {
  const b = engineStickerOf(brokenOrder)
  const r = b.stickersAt(engIdx({ ...LAND, sticker: { position_mode: 'manual', position: '2% 2%' } }, {}), 1.5)[0]
  check('[engine·变异] 旧顺序被钳回边距 32（对照新顺序的 14）', near(r.y, 32), true)
}

// ── 7. 落点编辑 = 草稿态（⑧ 的钉子）────────────────────────────────────────
console.log('\n⑧ 固定落点 = 草稿态：进入编辑 → 点选 → 保存/取消（不再"点哪儿存哪儿"）')
const pickBody = extractFunction(viewSrc, 'onPointPick')
const saveBody = extractFunction(viewSrc, 'savePointEdit')
const startBody = extractFunction(viewSrc, 'startPointEdit')
const cancelBody = extractFunction(viewSrc, 'cancelPointEdit')
const toggleSelBody = extractFunction(viewSrc, 'toggleSelect')

// 常驻点选（旧行为）的判据：pointMode 只看 position_mode
check('[view   ] pointMode 由编辑态门控（不是 manual 就常驻点选）',
  /const pointMode = computed\(\(\) => pointEdit\.value && !selectRegion\.value\)/.test(viewSrc), true)
// ★ 核心：点选回调里**不能有任何落盘**，只改草稿
check('[view   ] 点选只写草稿', /draftPos\.value\s*=/.test(pickBody), true)
check('[view   ] 点选不落盘（无 patchSticker / saveProjectPatch）',
  /patchSticker|saveProjectPatch/.test(pickBody), false)
check('[view   ] 进入编辑用已保存值初始化草稿',
  /draftPos\.value = \{ \.\.\.stickerPos\.value \}/.test(startBody), true)
check('[view   ] 保存落点写 [sticker] position', /patchSticker\(\{ position: fmtPos\(p\) \}\)/.test(saveBody), true)
check('[view   ] 保存成功才退出编辑态（失败不丢草稿）', /if \(await patchSticker\(/.test(saveBody), true)
check('[view   ] 取消丢弃草稿', /pointEdit\.value = false/.test(cancelBody), true)
// 两个"在预览上取坐标"的模式必须互斥，否则两层抢指针
check('[view   ] 进框选会丢弃点选草稿', /cancelPointEdit\(\)/.test(toggleSelBody), true)
check('[view   ] 进点选编辑会退出框选', /selectRegion\.value = ''/.test(startBody), true)
// 模板里真的挂了保存/取消两颗按钮（否则草稿态无从退出）
check('[view   ] 模板有「保存落点」按钮', /@click="savePointEdit"/.test(viewSrc), true)
check('[view   ] 模板有「取消」按钮', /@click="cancelPointEdit"/.test(viewSrc), true)
// 所见即所得：编辑态把草稿喂给预览（贴纸本身跟着动），不再显示"已保存那一版"
check('[view   ] 预览在编辑态传草稿落点', /:point="pointEdit \? draftPos : stickerPos"/.test(viewSrc), true)
check('[preview] 点选编辑态把草稿喂给索引', /if \(!props\.pointMode \|\| !props\.point \|\| !props\.config\) return props\.config/.test(previewSrc), true)
check('[preview] 只覆盖 _canvas.sticker.position（不动 position_mode）',
  /_canvas: \{ \.\.\.cv, sticker: \{ \.\.\.st, position: next \} \}/.test(previewSrc), true)

// 自证有效：把点选回调改回「点哪儿存哪儿」，上面那条必须翻面
const brokenPick = mutateAll(pickBody, [
  ['draftPos\\.value = \\{ x, y \\}', `patchSticker({ position: x + '% ' + y + '%' })`],
], '[view·变异] 点选回退成「点哪儿存哪儿」')
if (brokenPick) {
  check('[view·变异] 旧实现下「点选不落盘」这条会失败（本脚本能抓到）',
    /patchSticker|saveProjectPatch/.test(brokenPick), true)
}

console.log(failed ? `\n${failed} 项失败` : '\n全部通过')
process.exit(failed ? 1 : 0)
