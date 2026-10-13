#!/usr/bin/env node
// ============================================================================
// 生成 `scripts/anim-attribution.mjs` —— 39 档的**参数归属表**
// ============================================================================
//
// 用法：`node scripts/gen-anim-attribution.mjs [--check]`
//   * 不带参：重新生成并写盘（改了 `ANIM_PRESETS` 之后跑它）；
//   * `--check`：**只比对不写盘**（判据模式）—— 表与源码不一致就 exit 1。
//
// # 这张表是干什么的
//
// 「样式」与「动效」拆成两个类型之后，必须知道**每一档的每一个参数**该算哪一边。
// 表就是那个唯一权威 —— 界面分组、转译器分路、判据，全都读它。
//
// # ⚠️ 判据（两条，别改）
//
// * **形态** = 决定"字**静态**长什么样"（**与帧无关**）。
//   `scaleSpread` 属形态 —— 它住在 `perChar` 里，但四个端点全相等
//   ⇒ 是一条**静止的**序号阶梯（"只有最后一个字大"）。
// * **动效** = 画面**随时间变**。两个来源：
//   ① 整块时间函数（`shake*` / `sway*` / `scaleIn*`）；
//   ② 逐字 spec 的**端点不相等** ⇒ `char_transform_at` 的第 4 参（帧）真的起作用。
//
// ⚠️⚠️ **别用 `x in [array]` 判断数组里有没有这个键** —— 那查的是**索引**。
//    本脚本第一版就是这么写的，于是 `arc_up` 被误判成"纯形态"
//    （`'rotationFrom' in ['rotationFrom','rotationTo']` === `false`）。
//    症状是表**看起来全对**、只是少了一半的 `both`。⇒ 用 `Set.has`。
import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const OUT = path.join(ROOT, 'scripts', 'anim-attribution.mjs')
const ENGINE = 'file://' + path.join(ROOT, 'webui', 'src', 'polish', 'engine.js').replace(/\\/g, '/')

/** 只决定"静态长什么样"的键（与帧无关）。 */
const FORM_KEYS = new Set([
  'tiltDeg', 'squashY', 'vertical', 'fontFamily', 'fontWeight', 'safeWidthRatio',
  // `scaleSpread` 是**按序号的乘数**，四个端点全相等 ⇒ 静止阶梯，属形态。
  'scaleSpread',
  'spreadXEm', 'spreadYEm', 'spreadXFrom', 'spreadXTo', 'spreadYFrom', 'spreadYTo',
  'scaleFrom', 'scaleTo', 'rotationFrom', 'rotationTo', 'offsetYFrom', 'offsetYTo',
  'reverse',
])
/** 整块时间函数（让画面随时间变）。 */
const MOTION_KEYS = new Set([
  'shakePx', 'shakeHz', 'swayDeg', 'swayMs',
  'scaleInFrom', 'scaleInPeak', 'scaleInMs',
])
/** 逐字 spec 里"端点对"——两边都在 ⇒ 那个 spec 会随时间变。 */
const TIME_PAIRS = [
  ['scaleFrom', 'scaleTo'],
  ['rotationFrom', 'rotationTo'],
  ['offsetXFrom', 'offsetXTo'],
  ['offsetYFrom', 'offsetYTo'],
  ['spreadXFrom', 'spreadXTo'],
  ['spreadYFrom', 'spreadYTo'],
]

/** 算出 39 档的归属。**纯函数** —— 判据与生成走同一条路。 */
export async function buildAttribution() {
  const E = await import(ENGINE)
  const out = {}
  for (const [name, v] of Object.entries(E.ANIM_PRESETS)) {
    const top = Object.keys(v).filter((k) => k !== 'perChar')
    const pcArr = v.perChar ? Object.keys(v.perChar) : []
    const pc = new Set(pcArr)          // ⚠️ Set，不是数组（见文件头那条）
    const all = new Set([...top, ...pcArr])
    const form = [...all].filter((k) => FORM_KEYS.has(k)).sort()
    const motion = [...all].filter((k) => MOTION_KEYS.has(k)).sort()
    const pcTime = TIME_PAIRS.filter(([a, b]) => pc.has(a) && pc.has(b)).map(([a, b]) => a + '↔' + b)
    const moves = motion.length > 0 || pcTime.length > 0
    const hasForm = form.length > 0
    const kind = moves && hasForm ? 'both' : moves ? 'motion' : hasForm ? 'form' : 'none'
    out[name] = { kind, form, pcTime }
  }
  return out
}

/** 把归属表渲染成模块源码。 */
function render(attr) {
  const names = Object.keys(attr).sort()
  const body = names.map((n) => {
    const a = attr[n]
    const f = a.form.map((x) => "'" + x + "'").join(', ')
    const t = a.pcTime.map((x) => "'" + x + "'").join(', ')
    return "  '" + n + "': { kind: '" + a.kind + "', form: [" + f + "], pcTime: [" + t + "] },"
  }).join('\n')
  const tally = names.reduce((m, n) => ((m[attr[n].kind] = (m[attr[n].kind] || 0) + 1), m), {})
  return `// ⚠️ **本文件是生成的** —— 别手改，改 \`ANIM_PRESETS\` 后跑
//    \`node scripts/gen-anim-attribution.mjs\` 重新生成。
//    判据模式：\`node scripts/gen-anim-attribution.mjs --check\`（不一致就红）。
//
// 39 档的**参数归属表**：每一档的每个参数属于「形态」还是「动效」。
//
// # 判据（别改这两条）
//   * **形态** = 决定"字静态长什么样"（**与帧无关**）。\`scaleSpread\` 属形态 ——
//     它住在 \`perChar\` 里，但四个端点全相等 ⇒ 是一条**静止的**序号阶梯。
//   * **动效** = 画面**随时间变**。两个来源：① 整块时间函数（shake/sway/scaleIn*）；
//     ② 逐字 spec 的**端点不相等** ⇒ \`char_transform_at\` 的第 4 参（帧）真的起作用。
//
// # 统计（39 档）
//   form ${tally.form || 0} · motion ${tally.motion || 0} · both ${tally.both || 0} · none ${tally.none || 0}
//
// ⚠️ \`none\` 只有一档：\`fade_only\` —— 它动的是 **opacity**，不在这两条路上
//    （**不是"没实现"**，见 \`docs/design/39-tiers-split-style-vs-effect.md\`）。
export const ANIM_ATTRIBUTION = {
${body}
};

/** 这一档在「样式」轴上有东西吗。 */
export function hasFormPart(name) {
  const a = ANIM_ATTRIBUTION[name];
  return !!a && (a.kind === 'form' || a.kind === 'both');
}

/** 这一档在「动效」轴上有东西吗。 */
export function hasMotionPart(name) {
  const a = ANIM_ATTRIBUTION[name];
  return !!a && (a.kind === 'motion' || a.kind === 'both');
}
`
}

// ⚠️ **只在直接跑本文件时才生成/校验** —— 被 `check-anim-attribution.mjs`
//    import 时**什么都不做**（否则那条判据会顺带把文件重写一遍，
//    看起来"通过"了，其实是它自己刚把表刷成最新的 —— 判据空转）。
const isMain = process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)
if (!isMain) {
  // 被 import：只导出 `buildAttribution`，不产生任何副作用。
} else {
const attr = await buildAttribution()
const names = Object.keys(attr)
const tally = names.reduce((m, n) => ((m[attr[n].kind] = (m[attr[n].kind] || 0) + 1), m), {})
const text = render(attr)

if (process.argv.includes('--check')) {
  const cur = fs.existsSync(OUT) ? fs.readFileSync(OUT, 'utf8') : ''
  if (cur !== text) {
    console.log('✗ scripts/anim-attribution.mjs 与 ANIM_PRESETS 不一致 ——')
    console.log('  改过预设表就要重新生成：node scripts/gen-anim-attribution.mjs')
    process.exit(1)
  }
  console.log(`✓ 归属表是最新的：${names.length} 档（form ${tally.form || 0} · motion ${tally.motion || 0} · ` +
    `both ${tally.both || 0} · none ${tally.none || 0}）`)
  process.exit(0)
}

fs.writeFileSync(OUT, text)
console.log(`已重新生成 scripts/anim-attribution.mjs：${names.length} 档（` +
  `form ${tally.form || 0} · motion ${tally.motion || 0} · both ${tally.both || 0} · none ${tally.none || 0}）`)
}
