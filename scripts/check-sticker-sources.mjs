#!/usr/bin/env node
// **贴纸的两种事件写法，`stickersAt` 必须都求得出、且落在同一处**（2026-10-14）。
//
// # 背景（用户口径：「贴纸设置了、预览里不显示」）
//
// 贴纸有**两种写法**，取决于 `process` 跑没跑过：
//
// | 写法 | 落在哪个桶 | 谁在产 |
// |---|---|---|
// | `type = "sticker"` | `idx.sticker` | 老工程 / 手写 |
// | `type = "element"` + `kind = "sticker"` | `idx.element` | **`process` 现在产的** |
//
// ⚠️ 实测用户全部工程：**旧写法 28 个片段、新写法 1 个片段** —— 而
// `stickersAt`（模板 `.sticker-el` 的唯一数据源）**原来只读 `idx.sticker`**
// ⇒ 新写法**一条贴纸都求不出来**，而且**不报错**（桶是空的，循环转一圈就出来）。
//
// ⚠️ 同时 `getElements()` 认 `kind === 'sticker'`，但它产出
// `{src, opacity, scale, rot, yOff}` —— **没有 `x`/`y`**，而模板读 `st.x`/`st.y`；
// 且 `PolishPreview.vue` 的 `texts` **只收 `isText === true`**，把这些条目丢掉。
// ⇒ **两半各拿一半，缝里就是"贴纸不显示"。**
//
// # 它判什么
//
// ① 两种写法**都要被 `stickersAt` 求出来**（这是本判据的主项）；
// ② 求出来的**落点逐值相等**（同一个 `[sticker] manual` 落点）；
// ③ `src` 都要解析到真图（不是空串）—— "求得出事件、求不出图"是第二种静默失败；
// ④ 名字优先级链（`name` → `keyword` → `text`）三条都要过一遍；
// ⑤ **旧写法零回归**：值必须与"合流之前"逐位相同（这里把期望值写死）。
//
// 用法：node scripts/check-sticker-sources.mjs   （退出码非 0 即失败）

import { join, dirname } from 'node:path'
import { fileURLToPath, pathToFileURL } from 'node:url'

const HERE = dirname(fileURLToPath(import.meta.url))
const ROOT = join(HERE, '..')
const ENGINE = join(ROOT, 'webui', 'src', 'polish', 'engine.js')

let pass = 0
let fail = 0
const failures = []
function ok(cond, what) {
  if (cond) { pass += 1 } else { fail += 1; failures.push(what) }
}

const { buildIndex, stickersAt } = await import(pathToFileURL(ENGINE).href)

const SRC = (n) => `stickers/${n}.gif`
const T = 2   // 稳态：避开 0.35s 弹出 / 0.4s 上浮那两段
function run(ev, { mode = 'manual', pos = '49% 51%', stickerSrc = SRC } = {}) {
  const cfg = {
    _canvas: { width: 1920, height: 1080, fps: 30, sticker: { position_mode: mode, position: pos } },
    segments: [{ start: 0, end: 30, fx: 'low', events: [ev] }],
  }
  const idx = buildIndex({ config: cfg, subtitles: [], danmaku: [], duration: 30, fps: 30, stickerSrc })
  const list = stickersAt(idx, T)
  return { all: list, one: list[0] }
}

// ---------------------------------------------------------------------------
// ① 两种写法都必须求得出
// ---------------------------------------------------------------------------
const OLD_EV = { type: 'sticker', name: '思考', keyword: '思考', time: 1, end: 5, x: 49, y: 51 }
const NEW_EV = { type: 'element', kind: 'sticker', text: 'ysyk', name: 'PNGTuber_加载中',
                 sticker_tag: '加载', source: 'ysyk', time: 1, end: 5 }   // ⚠️ 真实原文没有 x/y

const ro = run(OLD_EV)
const rn = run(NEW_EV)

ok(!!ro.one, '① 旧写法 `type="sticker"` 必须求得出贴纸')
ok(!!rn.one, '① 新写法 `type="element" kind="sticker"`（`process` 产出）**也必须求得出**'
  + ' —— 这条就是用户那个 bug 的判据')

// ---------------------------------------------------------------------------
// ② 落点逐值相等（同一个 manual 落点，两种写法必须同处）
// ---------------------------------------------------------------------------
if (ro.one && rn.one) {
  ok(Math.abs(ro.one.x - rn.one.x) < 1e-9,
    `② 两种写法的 x 必须逐值相等（旧 ${ro.one.x} vs 新 ${rn.one.x}）`)
  ok(Math.abs(ro.one.y - rn.one.y) < 1e-9,
    `② 两种写法的 y 必须逐值相等（旧 ${ro.one.y} vs 新 ${rn.one.y}）`)
  ok(ro.one.w === rn.one.w, `② 边长必须相等（旧 ${ro.one.w} vs 新 ${rn.one.w}）`)
}

// ---------------------------------------------------------------------------
// ③ `src` 必须解析到真图
// ---------------------------------------------------------------------------
ok(!!ro.one && ro.one.src === 'stickers/思考.gif',
  `③ 旧写法的 src 必须解析出来（实得 ${JSON.stringify(ro.one && ro.one.src)}）`)
ok(!!rn.one && rn.one.src === 'stickers/PNGTuber_加载中.gif',
  `③ 新写法的 src 必须解析出来（实得 ${JSON.stringify(rn.one && rn.one.src)}）`
  + ' —— "求得出事件、求不出图"是第二种静默失败')

// ---------------------------------------------------------------------------
// ④ 名字优先级链：`name` → `keyword` → `text`
//
//    ⚠️ 这三条与 `buildIndex` 里那条 `const name = ev.name || ev.keyword || ev.text`
//       **必须同序**。少一项的症状是"src 空"（有位置、没图）。
// ---------------------------------------------------------------------------
for (const [label, mk, want] of [
  ['name 优先', () => ({ type: 'element', kind: 'sticker', name: 'AAA', keyword: 'BBB', text: 'CCC', time: 1, end: 5 }), 'AAA'],
  ['无 name 用 keyword', () => ({ type: 'element', kind: 'sticker', keyword: 'BBB', text: 'CCC', time: 1, end: 5 }), 'BBB'],
  ['只有 text', () => ({ type: 'element', kind: 'sticker', text: 'CCC', time: 1, end: 5 }), 'CCC'],
]) {
  const r = run(mk())
  ok(!!r.one && r.one.src === `stickers/${want}.gif`,
    `④ ${label}：src 该是 stickers/${want}.gif（实得 ${JSON.stringify(r.one && r.one.src)}）`)
}

// ---------------------------------------------------------------------------
// ⑤ 旧写法**零回归**：期望值由"合流之前"的实测钉死（R4）
//
//    ⚠️ 这几个数是改之前**真跑出来**的，不是我推的：
//       1920×1080 画布、贴纸边长 280、manual 中心 49%/51%
//       ⇒ 左上角 = 940.8 − 140 = 800.8 / 550.8 − 140 = 410.8
//       ⚠️ 但实测 y 是 392.8 ⇒ 差 18px = 稳态上浮量 `stickerFloatUp()`
//          （`stickerFloatUp` 随画布短边算）⇒ 8.5 那类是**预期**的，不是漂移。
// ---------------------------------------------------------------------------
if (ro.one) {
  ok(Math.abs(ro.one.x - 800.8) < 0.05, `⑤ 旧写法 x 零回归（期望 800.8，实得 ${ro.one.x}）`)
  ok(Math.abs(ro.one.y - 392.8) < 0.05, `⑤ 旧写法 y 零回归（期望 392.8，实得 ${ro.one.y}）`)
  ok(ro.one.w === 280, `⑤ 旧写法边长零回归（期望 280，实得 ${ro.one.w}）`)
  ok(ro.one.name === '思考', `⑤ 旧写法 name 零回归（期望 思考，实得 ${ro.one.name}）`)
}

// ---------------------------------------------------------------------------
// ⑥ manual 与 auto 必须分得开（别为了合流把 auto 那条压成 manual）
// ---------------------------------------------------------------------------
{
  const man = run(NEW_EV, { mode: 'manual', pos: '49% 51%' }).one
  const aut = run(NEW_EV, { mode: 'auto', pos: '49% 51%' }).one
  ok(!!man && !!aut, '⑥ manual / auto 两种模式都要能求出来')
  if (man && aut) {
    ok(Math.abs(man.x - aut.x) > 1,
      `⑥ manual（盒中心锚）与 auto（事件左上角锚）落点必须不同 —— 别把两套基准压成一套`
      + `（manual x=${man.x} auto x=${aut.x}）`)
  }
}

// ---------------------------------------------------------------------------
// ⑦ **反例**：窗口外不许画（贴纸有时长，`t` 出了 `[time, time+dur]` 就该没有）
// ---------------------------------------------------------------------------
{
  const r = (() => {
    const cfg = { _canvas: { width: 1920, height: 1080, fps: 30, sticker: { position_mode: 'manual', position: '49% 51%' } },
                  segments: [{ start: 0, end: 30, fx: 'low', events: [NEW_EV] }] }
    const idx = buildIndex({ config: cfg, subtitles: [], danmaku: [], duration: 30, fps: 30, stickerSrc: SRC })
    return stickersAt(idx, 20)     // NEW_EV 是 [1, 5]
  })()
  ok(r.length === 0, `⑦ 窗口外的 t 不该画出贴纸（实得 ${r.length} 条）`)
}

// ---------------------------------------------------------------------------

console.log(`贴纸事件来源合流：${pass} 项通过，${fail} 项失败`)
if (fail) {
  console.log()
  for (const f of failures) console.log('  ✗ ' + f)
  process.exit(1)
}
console.log('✓ `type="sticker"` 与 `element+kind="sticker"` 两种写法都能求值、落点同处、src 都有图')
