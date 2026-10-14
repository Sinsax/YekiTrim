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

import { readFileSync } from 'node:fs'
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

const { buildIndex, stickersAt, getElements } = await import(pathToFileURL(ENGINE).href)

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
// ⑧ **贴纸与文字必须分家**（2026-10-14，用户口径：「分离贴纸和文字层」）
//
//    ⚠️ 这一组是本次改动**存在的理由**，不是附带的整洁度检查：
//      分家之前，贴纸同时挂在 `idx.element` 与 `idx.sticker` 两个桶里，
//      **两个求值器互相知道对方** —— 动文字层就会碰到贴纸
//      （用户报的"贴纸不显示"正是这条缝的产物）。
// ---------------------------------------------------------------------------
{
  // 一个工程里**两种写法各来一条贴纸 + 一条文字**
  const MIX = [
    { type: 'element', kind: 'sticker', text: 'ysyk', name: 'PNG', time: 1, end: 5 },
    { type: 'element', kind: 'text', text: '你好', x: '38%', y: '47%', pos: 'bottom',
      source: 'comment|80|2|255,128,192,255|26,26,26,255|700|1|0|1|pink', time: 6, end: 10 },
    { type: 'sticker', name: '思考', keyword: '思考', time: 1, end: 5, x: 49, y: 51 },
  ]
  const cfg = {
    _canvas: { width: 1920, height: 1080, fps: 30, sticker: { position_mode: 'manual', position: '49% 51%' } },
    segments: [{ start: 0, end: 30, fx: 'low', events: MIX }],
  }
  const idx = buildIndex({ config: cfg, subtitles: [], danmaku: [], duration: 30, fps: 30, stickerSrc: SRC })

  // ⑧-1 `idx.element` 里**一条贴纸都不许有**（分家的正面判据）
  const strayStickers = (idx.element || []).filter(
    (e) => String(e.ev?.kind ?? '').trim().toLowerCase() === 'sticker')
  ok(strayStickers.length === 0,
    `⑧ \`idx.element\` 里不许再有贴纸（实得 ${strayStickers.length} 条）——`
    + ' 贴纸必须在**索引期**就归一进 `idx.sticker`，那是"分离"的定义')
  ok((idx.element || []).length === 1,
    `⑧ \`idx.element\` 该只剩那 1 条文字（实得 ${(idx.element || []).length} 条）`)

  // ⑧-2 `idx.sticker` 要**两种写法都在**（别为了分家把新写法弄丢）
  ok((idx.sticker || []).length === 2,
    `⑧ \`idx.sticker\` 该有 2 条（老写法 + 归一来的新写法），实得 ${(idx.sticker || []).length} 条`)

  // ⑧-3 **两个求值器各读各的**（互不越界）
  const st = stickersAt(idx, 3)
  const tx = getElements(idx, 8)
  ok(st.length === 2 && tx.length === 1,
    `⑧ 两个求值器各读各的桶（贴纸 ${st.length} 条该=2、文字 ${tx.length} 条该=1）`)
  ok(tx.every((e) => e.isText === true),
    '⑧ 文字求值器的产物**必须全是文字**（贴纸形状一条都不许混进来）')
  ok(st.every((s) => typeof s.x === 'number' && typeof s.y === 'number'),
    '⑧ 贴纸求值器的产物**必须都带 x/y**（模板 `.sticker-el` 读的就是这两个）')

  // ⑧-4 ⚠️⚠️ **源码级判据：`stickersAt` 只许读 `idx.sticker`**
  //
  //    ⚠️ 上面三条是**行为**判据。它们**抓不住"重新耦合"** —— 实测过：
  //      把 `stickersAt` 改回 `[...idx.sticker, ...idx.element]`（即恢复读两个桶），
  //      **23 项一条都不红**。原因很实在：分家已经把 `idx.element` 洗干净了，
  //      多读那个桶**贡献 0 条**，结果逐值相同 ⇒ 行为判据看不出差别。
  //
  //    ⇒ 要抓住"耦合回来"，只能**直接查源码读了哪个桶**。
  //      （这是"行为判据抓不住结构倒退"的典型一例，写在这里免得下一个人删掉它。）
  {
    const engSrc = readFileSync(ENGINE, 'utf8')
    // ⚠️ 用 `indexOf` + 花括号配平，**不用正则**：正则里的 `\n` / `\s`
    //    经 shell heredoc 会被吃掉（这个坑我自己踩过好几次，症状是
    //    `SyntaxError: Invalid regular expression` 或"锚点悄悄不命中"）。
    const start = engSrc.indexOf('export function stickersAt(idx, t) {')
    let body = null
    if (start >= 0) {
      let depth = 0
      for (let i = engSrc.indexOf('{', start); i < engSrc.length; i++) {
        if (engSrc[i] === '{') depth++
        else if (engSrc[i] === '}') { depth--; if (depth === 0) { body = engSrc.slice(start, i + 1); break } }
      }
    }
    ok(!!body, '⑧ 抽不到 `stickersAt` 的函数体（结构变了就要改本判据）')
    if (body) {
      ok(body.includes('idx.sticker'),
        '⑧ `stickersAt` 必须读 `idx.sticker`')
      ok(!body.includes('idx.element'),
        '⑧⚠️ `stickersAt` **不许读 `idx.element`** —— 那会把"贴纸/文字"重新耦合起来。'
        + '行为判据抓不到这一条（分家后那个桶贡献 0 条），所以这里查源码。')
    }

    // ⑧-5 镜像：**`getElements` 不许读 `idx.sticker`**（反方向也要钉住）
    const gStart = engSrc.indexOf('export function getElements(idx, t) {')
    let gBody = null
    if (gStart >= 0) {
      let d = 0
      for (let i = engSrc.indexOf('{', gStart); i < engSrc.length; i++) {
        if (engSrc[i] === '{') d++
        else if (engSrc[i] === '}') { d--; if (d === 0) { gBody = engSrc.slice(gStart, i + 1); break } }
      }
    }
    ok(!!gBody, '⑧ 抽不到 `getElements` 的函数体（结构变了就要改本判据）')
    if (gBody) {
      // ⚠️⚠️ **先剥注释再判**：`getElements` 里那段解释"为什么把贴纸求值删掉"的注释
      //     本身就会写出 `kind === 'sticker'` 与 `idx.sticker` 这些字样 ——
      //     不剥注释的话，判据会被**它自己让人写下的说明**顶翻
      //     （本仓 `check_module_boundaries.py` 头一条处理就是这个：它上一版手写 grep
      //     产出过 8 条假阳性，等于没用）。
      //     ⚠️ `(?<!:)` 是为了不把 `https://…` 里的 `//` 当注释起点。
      const code = gBody
        .replace(/\/\*[\s\S]*?\*\//g, '')
        .replace(/(^|[^:])\/\/[^\n]*/g, '$1')
      ok(!code.includes('idx.sticker'),
        '⑧⚠️ `getElements` **不许读 `idx.sticker`** —— 反方向的耦合同样要挡')
      // ⚠️ 精确到 **`sticker` 这个字面量**，不是"出现过 `kind`"：
      //    `getElements` **合法地**读 `kind`（判 `kind === 'text'` 是它唯一的入口），
      //    要挡的是**贴纸那条**（`kind === 'sticker'` 的求值已删）。
      //    写成"不许出现 kind"会把合法代码一起判红 —— 那是假阳性，比不判还坏。
      const mentionsStickerAsKind = /kind\s*===\s*['"]sticker['"]/.test(code)
        || /kind\s*!==\s*['"]sticker['"]/.test(code)
      ok(!mentionsStickerAsKind,
        '⑧⚠️ `getElements` 里**不许再判 `kind === "sticker"`** —— '
        + '那段求值已删（它产出没有 x/y 的形状、被消费者整批丢掉），'
        + '加回来 = 把"动文字层碰到贴纸"这个病复发。'
        + '（⚠️ 判的是"有没有拿 sticker 当 kind 比"，不是"有没有出现 kind" ——'
        + ' 后者会把 `kind === "text"` 这个合法入口一起判红）')
    }
  }
}

// ---------------------------------------------------------------------------

console.log(`贴纸来源与「贴纸/文字分离」：${pass} 项通过，${fail} 项失败`)
if (fail) {
  console.log()
  for (const f of failures) console.log('  ✗ ' + f)
  process.exit(1)
}
console.log('✓ `type="sticker"` 与 `element+kind="sticker"` 两种写法都能求值、落点同处、src 都有图')
