#!/usr/bin/env node
// 热度曲线的口径 + 回归门禁（`webui/src/polish/heat.js`）。
//
// 这条曲线**只在编辑器里看**（不落盘、不进出片），所以它没有"预览 vs 成片"那种对拍对象 ——
// 它真正的两个风险是别的：
//
//   ① **与母口径漂移**：按字 / 按条 / 自基准归一 / "缺席的那路不参与归一" 这四条是从
//      `crates/yekitrim-polish/src/process/punch.rs` 继承来的。那边改了（比如把字幕改成按条数），
//      这边会**静默**变成另一条曲线 —— 而两边各自看都"说得通"。所以这里**从 Rust 源码里逐条对拍**，
//      并显式记录**有意不同的两处**（窗口更细、权重反过来：弹幕为主）。
//   ② **把三种量纲写错**：字数按 UTF-16 码元（emoji 算 2）· 字量按"字数 ÷ 字幕时长"（被 ASR 碎片劫持）·
//      缺 end 的字幕整条静默丢掉。三条都各自有一个用例钉着，且都做过变异自证。
//
// 运行： node webui/scripts/check-heat-curve.mjs [--mutate]
import { readFileSync } from 'node:fs'
import { fileURLToPath, pathToFileURL } from 'node:url'
import { dirname, join } from 'node:path'

const here = dirname(fileURLToPath(import.meta.url))
const repo = join(here, '..', '..')
const HEAT_PATH = join(repo, 'webui', 'src', 'polish', 'heat.js')
const PUNCH_PATH = join(repo, 'crates', 'yekitrim-polish', 'src', 'process', 'punch.rs')

// ⚠️ Windows 上动态 import 必须走 `file://` URL（`pathToFileURL`），直接给 `F:\...`
//    会报 ERR_UNSUPPORTED_ESM_URL_SCHEME（同 check-emoji-decode.mjs）。
const heat = await import(pathToFileURL(HEAT_PATH).href)
const { heatSeries, heatBucketAt, heatCharCount, heatDisplay, HEAT_WINDOW_SECS, HEAT_W_DANMAKU, HEAT_W_SUBTITLE, HEAT_DISPLAY_GAMMA } = heat

let failed = 0
let passed = 0
const near = (a, b) => Math.abs(Number(a) - Number(b)) < 1e-9

function check(label, ok, detail) {
  if (ok) {
    passed++
    console.log(`  ok   ${label}`)
  } else {
    failed++
    console.log(`  ✗    ${label}${detail ? ' —— ' + detail : ''}`)
  }
}
function eq(label, actual, expected) {
  check(label, JSON.stringify(actual) === JSON.stringify(expected), `得到 ${JSON.stringify(actual)}，期望 ${JSON.stringify(expected)}`)
}
const nearArr2 = (actual, expected) =>
  Array.isArray(actual) && actual.length === expected.length && actual.every((v, i) => near(v, expected[i]))

function nearArr(label, actual, expected) {
  const ok = Array.isArray(actual) && actual.length === expected.length
    && actual.every((v, i) => near(v, expected[i]))
  check(label, ok, `得到 ${JSON.stringify(actual)}，期望 ${JSON.stringify(expected)}`)
}

// ── ① 与 punch.rs 的母口径对拍（漂了必须有人看一眼）─────────────────────────────
const punch = readFileSync(PUNCH_PATH, 'utf8')
const parity = [
  ['判据窗 5s（与这里的 0.5s **有意不同**：那是判据窗，这是画线的桶）',
    'pub const PUNCH_WINDOW_SECS: f64 = 5.0;'],
  ['字幕按字（不是按条）—— 空白剔除 + 码点计数',
    'sub.text.chars().filter(|c| !c.is_whitespace()).count() as f64'],
  ['分母是窗口宽度、不是说话时长（碎片劫持那条坑）',
    '.map(|(i, chars)| (i as f64 * window, chars / window))'],
  ['弹幕按条数累加',
    'let idx = (at / window).floor() as usize;'],
  ['自基准归一：除以最大值',
    'let max = raw.iter().map(|(_, v)| *v).fold(0.0, f64::max);'],
  ['母口径的权重是"字幕为主 0.6 / 弹幕为次 0.2"（**与这里反着**，理由在 heat.js 模块头 ②）',
    'const W_SUB: f64 = 0.6;'],
]
for (const [label, needle] of parity) {
  check(`punch.rs 仍然：${label}`, punch.includes(needle))
}
check('punch.rs 的弹幕权重仍是 0.2（母口径没被改成弹幕为主）', /const W_DM: f64 = 0\.2;/.test(punch))

// ── ② 常量本身（改它们就等于改整条曲线的观感，必须是显式的）────────────────────
check(`桶宽 = 0.5s（比判据窗细），实得 ${HEAT_WINDOW_SECS}`, near(HEAT_WINDOW_SECS, 0.5))
check(`弹幕权重 ${HEAT_W_DANMAKU} > 字幕权重 ${HEAT_W_SUBTITLE}（用户口径：弹幕要大一点）`,
  HEAT_W_DANMAKU > HEAT_W_SUBTITLE && near(HEAT_W_DANMAKU + HEAT_W_SUBTITLE, 1))

// ── ③ 码点计数（emoji 不能算 2）────────────────────────────────────────────────
eq("heatCharCount('笑😂') = 2（.length 会给 3）", heatCharCount('笑😂'), 2)
eq("heatCharCount('a b\\nc') = 3（空白剔除）", heatCharCount('a b\nc'), 3)
eq('heatCharCount(null/undefined) = 0', [heatCharCount(null), heatCharCount(undefined)], [0, 0])

// ── ④ 手算用例：两条路都在场，逐桶逐值 ────────────────────────────────────────
// duration 1.0s / 桶 0.5s ⇒ 2 桶；字幕 0→1.0 共 4 字（摊成 4 字/秒 × 2 桶）；
// 弹幕 0.10 ×2 条、0.60 ×1 条 ⇒ 桶0 = 2 条、桶1 = 1 条。
// 归一分（各除以本片段最大值）：字幕 [1,1]、弹幕 [1,0.5]
// ⇒ heat = 0.4×字幕 + 0.6×弹幕 = [0.4+0.6, 0.4+0.3] = [1.0, 0.7]
const a = heatSeries({
  duration: 1.0,
  subtitles: [{ start: 0, end: 1.0, text: '一二三四' }],
  danmakuStarts: [0.1, 0.15, 0.6],
})
eq('桶数 = ceil(1.0/0.5)', a.count, 2)
nearArr('字幕摊成 4 字/秒 × 2 桶', a.sub, [4, 4])
nearArr('弹幕按条归桶 [2,1]', a.dm, [2, 1])
nearArr('合成 heat = 0.4×字幕 + 0.6×弹幕', a.heat, [1.0, 0.7])
check('peak/peakAt 报的是最热的那一桶', near(a.peak, 1.0) && near(a.peakAt, 0))
check('两路都在场 ⇒ hasDanmaku / hasSubtitle 都为真', a.hasDanmaku === true && a.hasSubtitle === true)

// ── ⑤ 碎片不被劫持（punch.rs 那条坑）──────────────────────────────────────────
const frag = heatSeries({ duration: 12, subtitles: [{ start: 10.0, end: 10.05, text: '笑死我' }] })
check('0.05s / 3 字的碎片只贡献 0.3（不是 60 字/秒）', near(frag.sub[20], 0.3), `实得 ${frag.sub[20]}`)

// ── ⑥ 缺 end / 零长：字数**不许静默消失**（落在起点那一桶）──────────────────────
const point = heatSeries({ duration: 3, subtitles: [{ start: 1.2, text: '一二三四' }] })
check('缺 end 的点事件：4 字整份落在起点桶', near(point.sub[2], 4), `实得 ${point.sub[2]}`)
const zero = heatSeries({ duration: 3, subtitles: [{ start: 1.2, end: 1.2, text: '一二三四' }] })
check('零长（end === start）同上', near(zero.sub[2], 4), `实得 ${zero.sub[2]}`)

// ── ⑦ 没有弹幕 ⇒ 字幕独占权重（不是"给缺席的那路算 0 分"）───────────────────────
// duration 2s / 桶 0.5s ⇒ 4 桶；桶0 一条 4 字（摊成 4 字/秒）、桶2 一条 2 字 ⇒ 归一 [1,0,0.5,0]
const noDm = heatSeries({ duration: 2, subtitles: [{ start: 0, end: 0.5, text: '一二三四' }, { start: 1.0, end: 1.5, text: '一二' }] })
check('无弹幕 ⇒ hasDanmaku 假', noDm.hasDanmaku === false)
nearArr('无弹幕 ⇒ heat 逐值等于字幕的归一值（字幕独占 1.0 权重）', noDm.heat, [1, 0, 0.5, 0])
const dmOnly = heatSeries({ duration: 1.0, danmakuStarts: [0.1, 0.2, 0.3, 0.7] })
nearArr('无字幕 ⇒ heat 逐值等于弹幕的归一值', dmOnly.heat, [1, 1 / 3])
check('无字幕时弹幕仍是唯一判据（hasSubtitle 假）', dmOnly.hasSubtitle === false)

// ── ⑧ 自基准归一：整体等比放大不该改变形状 ────────────────────────────────────
const once = heatSeries({ duration: 2, subtitles: [{ start: 0, end: 2, text: '八个字八个字' }], danmakuStarts: [0.2, 0.3, 1.2] })
const thrice = heatSeries({ duration: 2, subtitles: [{ start: 0, end: 2, text: '八个字八个字' }], danmakuStarts: [0.2, 0.3, 1.2, 0.2, 0.3, 1.2, 0.2, 0.3, 1.2] })
nearArr('弹幕整体 ×3 ⇒ 归一后的 heat 逐值不变', thrice.heat, once.heat)

// ── ⑨ 退化输入不许抛、也不许给空序列 ──────────────────────────────────────────
let threw = null
let degen = null
try {
  degen = [
    heatSeries(), heatSeries({}), heatSeries({ duration: 0 }), heatSeries({ duration: NaN }),
    heatSeries({ duration: 5, subtitles: [{ start: NaN, text: 'x' }, { start: -3, end: -1, text: 'y' }, { text: 'z' }], danmakuStarts: [NaN, -1, 0] }),
  ]
} catch (e) { threw = e }
check('退化输入不抛（undefined / duration 0 / NaN / 负时间）', threw === null, threw && threw.message)
check('退化输入也给至少一桶（不是空数组）', degen && degen.every((s) => s.count >= 1 && s.heat.length === s.count))
const clamped = heatBucketAt(heatSeries({ duration: 1, danmakuStarts: [0] }), 99)
check('读数越界钳到末桶（不返回 null）', clamped && clamped.index === 1, JSON.stringify(clamped))
check('空序列的读数返回 null（不抛）', heatBucketAt(null, 1) === null)
const readA = heatBucketAt(a, 0.6)
check('读数取到第 2 桶，且带上两路的原始量', readA && readA.index === 1 && near(readA.heat, 0.7) && readA.dm === 1 && near(readA.sub, 4),
  JSON.stringify(readA))

// ── ⑩ 装配不变式（读 `TimelineEditor.vue` 的源码文本）──────────────────────────────
// 这几条正是**静态判据看不见、画面上也不报错**的错法：
//   · 标尺带长高的三条高度必须**成对** —— 少一条就与轨道列错位 12px，画面上只是"刻度比泳道矮一点"；
//   · 曲线层的 `pointer-events` 必须是 `none` —— 否则标尺的 pointerdown（seek / 拖播放头）被吃掉；
//   · 序列必须**只在 `loadData` 里建一次** —— 用户口径 2026-10「按 polish init 时的数据算即可，
//     不做随编辑即时重算」；把 `subtitles` 接进 computed 就是那条口径的反面（且看不出来）。
const VIEW = join(repo, 'webui', 'src', 'components', 'TimelineEditor.vue')
const viewSrc = readFileSync(VIEW, 'utf8')

function assemblyProblems(src) {
  const out = []
  // ① 三条 `.heat-on` 高度成对（36px = 20 标尺 + 16 热度带）
  for (const sel of ['.ruler-label.heat-on', '.ruler-lane.heat-on', '.ruler-track.heat-on']) {
    const re = new RegExp(sel.replace(/\./g, '\\.') + '\\s*\\{[^}]*height:36px')
    if (!re.test(src)) out.push(`${sel} 没有 height:36px`)
  }
  // ② 刻度下移的量 = 热度带高度；
  //    ⚠️ **时间标签不许再叠一次位移**：`.ruler-time` 的 containing block 就是 `.ruler-tick`
  //    （模板里 Span 在绝对定位的 Div 里）—— 刻度整体下移 16px 后，标签保持原来那个 `top:1px` 正好。
  //    再写一条 `.heat-on .ruler-time { top:13px }` = 16 + 13 + 字号 9 ≈ 38px > 36px 带高 ⇒
  //    **标签被裁掉半行**（2026-10 实测踩过：静态判据、"刻度下移 16px"的读数都是绿的，是截图放大才看见）。
  if (!/\.ruler-track\.heat-on \.ruler-tick \{[^}]*top:16px/.test(src)) out.push('刻度没有下移到 16px')
  if (/\.ruler-track\.heat-on \.ruler-time \{/.test(src)) out.push('时间标签被再叠了一次位移（containing block 就是刻度，会越出带高）')
  const num = (re) => { const m = src.match(re); return m ? Number(m[1]) : null }
  const bandH = num(/const HEAT_H = (\d+)/)
  const onH = num(/\.ruler-track\.heat-on \{ height:(\d+)px/)
  const tickTop = num(/\.ruler-track\.heat-on \.ruler-tick \{[^}]*top:(\d+)px/)
  const labTop = num(/\.ruler-time \{[^}]*top:(\d+)px/)
  const labFont = num(/\.ruler-time \{[^}]*font-size:(\d+)px/)
  if ([bandH, onH, tickTop, labTop, labFont].some((v) => v === null)) {
    out.push('抠不出标尺带的几何数字（带高 / 刻度偏移 / 标签偏移 / 字号）')
  } else if (tickTop + labTop + labFont > onH) {
    out.push(`标签越出带底：${tickTop} + ${labTop} + ${labFont} = ${tickTop + labTop + labFont} > ${onH}`)
  }
  // ③ 曲线层不吃指针
  const layer = src.match(/\.heat-layer \{[^}]*\}/)
  if (!layer || !/pointer-events:\s*none/.test(layer[0])) out.push('.heat-layer 没有 pointer-events:none')
  // ④ 热度带高度只有一份（常量 − SVG 高度）
  if (!/const HEAT_H = 16\b/.test(src)) out.push('HEAT_H 不是 16')
  if (!/:height="HEAT_H"/.test(src)) out.push('SVG 高度没有走 HEAT_H')
  // ⑤ 序列只在 loadData 里建一次（"不随编辑重算"）
  const builds = src.split('heatRaw.value = heatSeries(').length - 1
  if (builds !== 1) out.push(`heatSeries 被建了 ${builds} 次（应当只有 loadData 里那一次）`)
  // ⑥ 换片段必须清（跨片段串写：拿上一个片段的弹幕算新片段的曲线）
  const at = src.indexOf('function resetTimelineState()')
  const body = at < 0 ? '' : src.slice(at, src.indexOf('\n}', at))
  if (!/heatRaw\.value = null/.test(body)) out.push('resetTimelineState 没有清 heatRaw')
  if (!/heatDanmaku\.value = \[\]/.test(body)) out.push('resetTimelineState 没有清 heatDanmaku')
  // ⑦ 曲线颜色只许走 tokens（双主题）：写死 #hex / rgb() 就是"只在一套主题下好看"
  const cssFrom = src.indexOf('.heat-layer {')
  const cssTo = src.indexOf('.ruler-tick {')
  const heatCss = cssFrom >= 0 && cssTo > cssFrom ? src.slice(cssFrom, cssTo) : ''
  if (!heatCss) out.push('找不到热度曲线的 CSS 段（.heat-layer … .ruler-tick）')
  else if (/#[0-9a-fA-F]{3,6}\b|rgba?\(/.test(heatCss)) out.push('热度曲线的 CSS 里有颜色字面量（必须走 tokens）')
  // ⑧ **画线与读数必须是同一个映射**（否则"曲线画得高、数字读数低"= 界面在说谎）。
  //    两处都得过 `heatDisplay`：画线 `yOf(heatDisplay(v / peak))`、读数 `heatDisplay(b.heat / heatPeak.value)`。
  if (!/yOf\(heatDisplay\(/.test(src)) out.push('画线没有过 heatDisplay（曲线与读数会各说一套）')
  if (!/heatDisplay\(b\.heat \/ heatPeak\.value\)/.test(src)) out.push('读数没有过 heatDisplay（曲线与读数会各说一套）')
  return out
}

const assembly = assemblyProblems(viewSrc)
check('标尺带 / 曲线层的装配不变式（8 类）', assembly.length === 0, assembly.join('；'))

// ── ⑩b 显示对比拉伸（"起伏大一点"那把旋钮）──────────────────────────────────────
// 它**不改 heat 的数值**（上面那一整套对拍不受影响），只决定画多高 / 读多少。
check('显示对比拉伸的指数是 2（用户口径：起伏要看得见）', near(HEAT_DISPLAY_GAMMA, 2), `实得 ${HEAT_DISPLAY_GAMMA}`)
check('heatDisplay：两端不动（0→0、1→1）', near(heatDisplay(0), 0) && near(heatDisplay(1), 1),
  `实得 ${heatDisplay(0)} / ${heatDisplay(1)}`)
check('heatDisplay：中点被压到 0.25（落差拉开，但峰值仍顶格）', near(heatDisplay(0.5), 0.25), `实得 ${heatDisplay(0.5)}`)
check('heatDisplay：越界与脏输入钳在 [0,1]（1.5→1、-0.5→0、NaN/undefined→0）',
  near(heatDisplay(1.5), 1) && near(heatDisplay(-0.5), 0) && near(heatDisplay(NaN), 0) && near(heatDisplay(undefined), 0),
  [1.5, -0.5, NaN, undefined].map((v) => heatDisplay(v)).join(' / '))
check('heatDisplay：单调不降（画出来的高低顺序 = 数据的高低顺序）', (() => {
  let prev = -1
  for (let i = 0; i <= 100; i++) { const v = heatDisplay(i / 100); if (v < prev) return false; prev = v }
  return true
})())
check('显示拉伸**不动** heat 的数值（对拍那套仍是原口径）', (() => {
  const s = heatSeries({ duration: 1, subtitles: [{ start: 0, end: 1, text: '一二三四' }], danmakuStarts: [0.1, 0.6] })
  return near(s.heat[0], 1) && near(heatDisplay(s.heat[0]), 1)
})())

// ── ⑪ 变异自证：判据抓不抓得住这几种真实错法 ────────────────────────────────────
if (process.argv.includes('--mutate')) {
  const src = readFileSync(HEAT_PATH, 'utf8')
  const importMutant = async (text, tag) => {
    // ⚠️ 先证明替换**真的命中**了，否则"没红"可能只是没改到（同 check-emoji-decode.mjs）
    if (text === src) {
      console.log(`  ✗ 变异 ${tag}：源码文本没有变化，变异无效`)
      process.exit(1)
    }
    return await import('data:text/javascript;base64,' + Buffer.from(text, 'utf8').toString('base64'))
  }

  // 变异 A：把"字量按重叠摊开"退回"每个覆盖到的桶都记满字数" ⇒ 跨桶的那条字幕必须翻面。
  // ⚠️ **夹具必须跨桶**：一条恰好整桶跨度的字幕（0→0.5）在两种写法下**逐值相同**
  //    （4 字 × 0.5 / 0.5 = 4），拿它当夹具就是"变异没红、判据其实抓不住"。
  const bucketOnly = src.replace('sub[i] += (chars * ov) / win', 'sub[i] += chars')
  const mA = await importMutant(bucketOnly, 'A 按桶记满')
  // 4 字跨 0.25→1.25（桶宽 0.5）⇒ 正确 [2,4,2]；变异后 [4,4,4]
  const spreadFixture = { duration: 1.5, subtitles: [{ start: 0.25, end: 1.25, text: '一二三四' }] }
  const good = heatSeries(spreadFixture)
  const spread = mA.heatSeries(spreadFixture)
  check('变异自证 A：字量退回"每个覆盖到的桶都记满" ⇒ 跨桶字幕翻面（判据有效）',
    nearArr2(good.sub, [2, 4, 2]) && !nearArr2(spread.sub, good.sub),
    `正确 ${JSON.stringify(good.sub)} 变异后 ${JSON.stringify(spread.sub)}`)

  // 变异 B：字数退回 UTF-16 码元（`String.length`）⇒ ③ 的 emoji 用例必须翻面
  const byUnits = src.replace(
    /for \(const ch of String\(text == null \? '' : text\)\) \{\n    if \(!\/\\s\/.test\(ch\)\) n\+\+\n  \}/,
    "n = String(text == null ? '' : text).replace(/\\s/g, '').length",
  )
  const mB = await importMutant(byUnits, 'B 按码元计数')
  check('变异自证 B：按 UTF-16 码元计数 ⇒ 笑😂 变成 3（判据有效）', mB.heatCharCount('笑😂') === 3,
    `实得 ${mB.heatCharCount('笑😂')}`)

  // 变异 C：把权重反过来（回到判据窗那套）⇒ ④ 的手算用例必须翻面
  const mC = await importMutant(src.replace('export const HEAT_W_DANMAKU = 0.6', 'export const HEAT_W_DANMAKU = 0.2')
    .replace('export const HEAT_W_SUBTITLE = 0.4', 'export const HEAT_W_SUBTITLE = 0.8'), 'C 权重反转')
  const c = mC.heatSeries({ duration: 1.0, subtitles: [{ start: 0, end: 1.0, text: '一二三四' }], danmakuStarts: [0.1, 0.15, 0.6] })
  check('变异自证 C：权重改成 0.8/0.2 ⇒ 第 2 桶从 0.7 变成 0.9（判据有效）', near(c.heat[1], 0.9),
    `实得 ${c.heat[1]}`)

  // 变异 D：删掉三条 `.heat-on` 高度里的一条（最真实的错法：改了标尺忘了标签列）⇒ 装配判据必须翻面
  const dropOne = viewSrc.replace('.ruler-label.heat-on { height:36px; }', '')
  const dProblems = dropOne === viewSrc ? ['变异没命中'] : assemblyProblems(dropOne)
  check('变异自证 D：少一条 .heat-on 高度 ⇒ 装配判据翻面（判据有效）',
    dropOne !== viewSrc && dProblems.length > 0, dProblems.join('；') || '变异没命中')

  // 变异 F：把显示拉伸关掉（指数回到 1 = 直接画原始值）⇒ ⑩b 的中点判据必须翻面
  const mF = await importMutant(src.replace('export const HEAT_DISPLAY_GAMMA = 2', 'export const HEAT_DISPLAY_GAMMA = 1'), 'F 关掉对比拉伸')
  check('变异自证 F：显示拉伸指数改成 1 ⇒ 中点从 0.25 变回 0.5（判据有效）', near(mF.heatDisplay(0.5), 0.5),
    `实得 ${mF.heatDisplay(0.5)}`)

  // 变异 G：读数漏掉 heatDisplay（曲线照旧、数字变回原始值）⇒ 装配判据 ⑧ 必须翻面
  // ⚠️ 必须 replaceAll：同一句在**读数与悬停提示里各有一份**，只换第一处时 ⑧ 仍能在那另一处匹配上
  //    ⇒ "变异没红"其实只是没改到（第一次写就是这样，报了"变异没命中"）。
  const noDisplay = viewSrc.replaceAll('Math.round(heatDisplay(b.heat / heatPeak.value) * 100)',
    'Math.round((b.heat / heatPeak.value) * 100)')
  const gProblems = noDisplay === viewSrc ? ['变异没命中'] : assemblyProblems(noDisplay)
  check('变异自证 G：读数少过一边 heatDisplay ⇒ 装配判据翻面（判据有效）',
    noDisplay !== viewSrc && gProblems.length > 0, gProblems.join('；') || '变异没命中')

  // 变异 E：把 token 换成写死的颜色 ⇒ 双主题判据必须翻面
  const hardCoded = viewSrc.replace('stroke:var(--main)', 'stroke:#E86838')
  const eProblems = hardCoded === viewSrc ? ['变异没命中'] : assemblyProblems(hardCoded)
  check('变异自证 E：描边写死 #E86838 ⇒ 双主题判据翻面（判据有效）',
    hardCoded !== viewSrc && eProblems.length > 0, eProblems.join('；') || '变异没命中')
}

console.log(failed
  ? `\n✗ ${failed} 项失败（通过 ${passed} 项）`
  : `\n✓ 全部通过（${passed} 项）`)
process.exit(failed ? 1 : 0)
