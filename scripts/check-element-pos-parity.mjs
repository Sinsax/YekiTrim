#!/usr/bin/env node
// **元素落点：预览必须用 `x`/`y`（盒左上角），不许用 `pos` 三值近似**（2026-10-14）。
//
// # 为什么要有这一条（这是用户报了**四轮**的那个 bug）
//
// 用户口径：「**这个文字层的位置问题，修了几轮？当前还是不正常的位置**」。
//
// 四轮各修了一层（偏 85px → 接上 `pos` → 竖排/弧形/逐字 → 逐字读哪条轴），
// **没有一轮修到根上**。根因是：
//
//   * 真权威 = `process` 按 `TextLayout::anchor_origin(f)` 算好的**盒左上角**，
//     写进 `x`/`y`；
//   * 而预览把 `pos` 当落点、用**三值近似**（`6 / 50 / 94`）—— 它**不带**
//     `caption_clearance`（底部避让字幕带）、**不带**盒尺寸（随行数变）。
//
// 实测底部锚点的差：**单行 231px / 两行 311px / 三行 391px**。
//
// # 它判什么
//
// ① `getElements` 对缺 `x`/`y` 的条目**必须整条跳过**（不许"近似画出来"）；
// ② 有 `x`/`y` 时，输出里 `usesBoxXY === true`，且 `yPct` **逐值等于**输入；
// ③ 模板的 `top` 表达式：`usesBoxXY` 为真 ⇒ **必须取 `yPct`**；
// ④ **反例**：`pos` 三值近似**不许**盖过 `x`/`y`（把这条写成数值断言，见下）。
//
// 用法：node scripts/check-element-pos-parity.mjs  （退出码非 0 即失败）

import { readFileSync } from 'node:fs'
import { join, dirname } from 'node:path'
import { fileURLToPath, pathToFileURL } from 'node:url'

const HERE = dirname(fileURLToPath(import.meta.url))
const ROOT = join(HERE, '..')
const ENGINE = join(ROOT, 'webui', 'src', 'polish', 'engine.js')
const PREVIEW = join(ROOT, 'webui', 'src', 'components', 'PolishPreview.vue')

let pass = 0
let fail = 0
const failures = []
function ok(cond, what) {
  if (cond) { pass += 1 } else { fail += 1; failures.push(what) }
}

const { buildIndex, getElements } = await import(pathToFileURL(ENGINE).href)

// ⚠️ 元素事件必须给全 `text` / `x` / `y` / `source` —— 少一个 `getElements`
//    就返回**空数组**，而那种空结果看起来"不是 bug"，会让人查错方向。
const SRC = 'hook|80|6|255,255,255,255|26,26,26,255|700|0'
function idxOf(ev) {
  const cfg = {
    _canvas: { width: 1920, height: 1080, fps: 30, element: { fontPx: 80 } },
    segments: [{ start: 0, end: 20, fx: 'low', events: [ev] }],
  }
  return buildIndex({ config: cfg, subtitles: [], danmaku: [], duration: 20, fps: 30, stickerSrc: () => '' })
}
const base = { type: 'element', kind: 'text', text: '测试文字', source: SRC, time: 1, end: 20 }

// ---------------------------------------------------------------------------
// ① 有 x/y：`yPct` 必须**逐值**等于输入，并标 `usesBoxXY`
// ---------------------------------------------------------------------------
for (const [yIn, xIn] of [['72%', '50%'], ['9%', '12%'], ['88.5%', '50%']]) {
  const e = getElements(idxOf({ ...base, x: xIn, y: yIn, pos: 'bottom' }), 3)[0]
  ok(!!e, `有 x/y 的条目必须被画出来（y=${yIn}）`)
  if (!e) continue
  ok(e.usesBoxXY === true, `有 x/y ⇒ usesBoxXY 必须为 true（y=${yIn}）`)
  ok(e.yPct === parseFloat(yIn), `yPct 必须逐值等于输入 ${yIn}（实得 ${e.yPct}）`)
  ok(e.xPct === parseFloat(xIn), `xPct 必须逐值等于输入 ${xIn}（实得 ${e.xPct}）`)
}

// ---------------------------------------------------------------------------
// ② 缺 x/y：**整条跳过** —— 不许"用 pos 近似画出来"
//
//    ⚠️ 这条是判据的**灵魂**：三值近似之所以危险，正是因为它**画得出来**，
//       而且看着"挺像那么回事"，于是没人发现位置不对。
// ---------------------------------------------------------------------------
for (const [label, ev] of [
  ['两端都缺', { ...base, pos: 'bottom' }],
  ['只缺 x', { ...base, y: '72%', pos: 'bottom' }],
  ['只缺 y', { ...base, x: '50%', pos: 'bottom' }],
]) {
  const out = getElements(idxOf(ev), 3)
  ok(out.length === 0,
    `缺 x/y（${label}）必须**整条不画**（实得 ${out.length} 条）—— 不许用 pos 近似补一个位置出来`)
}

// ---------------------------------------------------------------------------
// ③ 模板：`usesBoxXY` 真 ⇒ `top` **必须**取 `yPct`
// ---------------------------------------------------------------------------
const psrc = readFileSync(PREVIEW, 'utf8')
ok(/top:\s*\(tx\.usesBoxXY\s*\|\|\s*tx\.topPct\s*===\s*null\s*\?\s*tx\.yPct\s*:\s*tx\.topPct\)/.test(psrc),
  '模板的 `top` 必须写成 `(tx.usesBoxXY || tx.topPct === null ? tx.yPct : tx.topPct)`')
ok(/left:\s*tx\.xPct\s*\+\s*'%'/.test(psrc), '模板的 `left` 必须用 `tx.xPct`')

// ⚠️ 把那个三元**真跑一遍**（用同一段文字抽出来的表达式不行 ⇒ 直接按语义验算）
//    这里验的是"有 x/y 的那条分支取 yPct"，与真模板同义。
const pickTop = (tx) => (tx.usesBoxXY || tx.topPct === null ? tx.yPct : tx.topPct)
ok(pickTop({ usesBoxXY: true, yPct: 72, topPct: 94 }) === 72,
  '有 x/y ⇒ top 必须取 yPct(72)，**不许**被 pos 近似的 94 盖掉')
ok(pickTop({ usesBoxXY: true, yPct: 62, topPct: null }) === 62,
  'pos 认不出（topPct=null）⇒ top 仍取 yPct')

// ---------------------------------------------------------------------------
// ④ 反例（**把"近似会错多少"写成数值**，免得有人再退回近似）
//
//    用 `anchor_origin` 的公式算一次真值，与三值近似比 —— 差必须**大**。
//    ⚠️ 判据反过来写：如果哪天有人把近似改准了（差值变小），这条也该跟着改，
//       而不是让它静默通过。
// ---------------------------------------------------------------------------
{
  const F = { w: 1920, h: 1080 }
  const MARGIN = 32
  // 字幕带下沿（与 `caption_band` 同量级：底部 80px + 字号 72 + 描边 12）
  const bandY0 = F.h - 80 - 72 - 12
  const clearance = (F.h - bandY0)
  const approxPct = 94                       // 三值近似的 bottom 档
  for (const [name, bh] of [['单行', 100], ['两行', 180], ['三行', 260]]) {
    const yReal = Math.max(MARGIN, F.h - bh - MARGIN - clearance)
    const yApprox = F.h * approxPct / 100
    const gap = Math.abs(yApprox - yReal)
    ok(gap > 100,
      `自证：三值近似在「${name}」上与真值该差 >100px（实得 ${gap.toFixed(0)}px）`
      + ' —— 差太小说明这条判据的前提变了，要重新核')
  }
}

// ---------------------------------------------------------------------------

console.log(`元素落点权威性：${pass} 项通过，${fail} 项失败`)
if (fail) {
  console.log()
  for (const f of failures) console.log('  ✗ ' + f)
  process.exit(1)
}
console.log('✓ 落点走 `x`/`y`（盒左上角）；缺 x/y 的条目**整条不画**，不用 `pos` 近似冒充')
