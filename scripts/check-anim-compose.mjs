#!/usr/bin/env node
// ============================================================================
// `check-anim-compose.mjs` —— **「样式 × 动效」真的可叠加吗**
// ============================================================================
//
// 用法：`node scripts/check-anim-compose.mjs`
//
// # 它存在的理由
//
// 用户口径：「两个设置，有各自的细分，**是可叠加的**」。这句话有三个
// **都不会自己报错**的破坏方式，每一处本趟都真踩到过：
//
//   ① **写同一个字段** —— 两轴共用一个 `anim`/`style_anim` ⇒ 用户选样式就
//      把动效**清掉**。这是"三级互斥"那一版的错法（已回退过一次）。
//   ② **`both` 的类不同步** —— `slam_squash` 的压扁来自 `scale_y`（形态侧），
//      用户从**动效栏**选它、只在**一栏**写值 ⇒ 压扁不生效，而界面看着是选中的。
//   ③ **同步得太狠** —— 另一轴已经有别的档（「竖排」）时，点一下动效栏的
//      `both` 类就把竖排**冲掉**了。这最阴：用户设好的东西被"点一下"偷走。
//
// ⚠️⚠️ **本判据不重写逻辑** —— 它把 `TimelineEditor.vue` 里的**真函数**
//   （`axisField` / `kindsOnAxis` / `setKindOnAxis` / `setVariantOnAxis`）
//   原样抽出来**执行**。自己照语义再写一遍是自证循环：我写错，判据跟着错。
//   （第一版就踩了：harness 用正则剥注释把函数体切坏了 ⇒ 报 6 个种类，
//     而真值是 11 / 9 —— **判据的工具坏掉时，看起来和"代码坏了"一模一样**。）
import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
// ⚠️ **权威归属表**（`scripts/anim-attribution.mjs`，由 `gen-anim-attribution.mjs` 生成）。
//    第 ⑦ 段拿它跟另外三份实现对拍。
import { ANIM_ATTRIBUTION } from './anim-attribution.mjs'

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const VUE = path.join(ROOT, 'webui', 'src', 'components', 'TimelineEditor.vue')
const src = fs.readFileSync(VUE, 'utf8')

// 预览那份（第四份实现）—— 它自己导出这两条，直接 import，**不重写**。
const PREVIEW = await import(
  'file://' + path.join(ROOT, 'webui', 'src', 'polish', 'engine.js').replace(/\\/g, '/')
)
const ATTRIBUTION = ANIM_ATTRIBUTION
/// 13 个效果族的**前缀**（`none` 不算族）。
/// ⚠️ `shrink_in` 必须排在 `shrink` 前面（长前缀优先，否则"逐字变小"会被"收缩"抢走）。
const FAMILIES = ['shrink_in', 'tilt', 'size', 'shrink', 'grow', 'vtext', 'arc',
  'scatter', 'shake', 'sway', 'slam', 'pop', 'hop']

const fails = []
const ok = (cond, msg) => { if (!cond) fails.push(msg) }
let checks = 0
const check = (cond, msg) => { checks++; ok(cond, msg) }

/// 按**字符配对**取一段（`[]` / `{}` 各一个版本）。
/// ⚠️ 别用正则猜结尾 —— 函数体里嵌套的括号会让它早停/晚停，**静默**抽错。
function span(marker, open, close) {
  const i = src.indexOf(marker)
  if (i < 0) throw new Error(`找不到锚点：${marker}`)
  const b = src.indexOf(open, i)
  let d = 0, started = false, j = b
  for (; j < src.length; j++) {
    if (src[j] === open) { d++; started = true } else if (src[j] === close) { d--; if (started && d === 0) { j++; break } }
  }
  return src.slice(b, j)
}

const ANIM_KINDS = eval(span('const ANIM_KINDS = [', '[', ']'))
// ⚠️ 定义区**整段**取（从 `kindsOnAxis` 到 `setVariantOnAxis` 结束）——
//    逐个函数取会被箭头函数体里的 `{}` 骗到。**原样**执行，不剥注释、不重写。
const startAt = src.indexOf('const kindsOnAxis = ')
if (startAt < 0) throw new Error('找不到 `const kindsOnAxis = `（可叠加的实现没了）')
const varBody = span('function setVariantOnAxis', '{', '}')
const varEnd = src.indexOf(varBody) + varBody.length
const defsSrc = src.slice(startAt, varEnd)

const body = [
  // 桩：定义区里那几个 `xxxKind` / `xxxHint` 是 `computed(...)`，这里不用它们。
  'const computed = (fn) => ({ get value() { return fn() } });',
  `const ANIM_KINDS = ${JSON.stringify(ANIM_KINDS)};`,
  'const ANIM_KIND_OF = new Map(ANIM_KINDS.flatMap((k) => k.variants.map((v) => [v.value, k.kind])));',
  'let selItem = { value: {} };',
  defsSrc,
  'return {',
  '  reset: () => { selItem.value = {}; },',
  '  kind: (a, k) => { setKindOnAxis(a, k); return { ...selItem.value }; },',
  '  variant: (a, v) => { setVariantOnAxis(a, v); return { ...selItem.value }; },',
  '  kindsOnAxis: (a) => kindsOnAxis(a).map((k) => k.kind),',
  // ⚠️ `labelOn` 必须把**真的那一项**（含 `label`）交给 `kindLabelOn` ——
  //    第一版图省事传了 `{ kind: k }`，缺 `label` ⇒ 动效栏恒 `undefined`，
  //    而样式栏照样返回「清除」（那条走的是常量分支）⇒ **只红一半**。
  '  labelOn: (a, k) => kindLabelOn(a, kindsOnAxis(a).find((x) => x.kind === k) ?? {}),',
  '};'
].join('\n')

let api
try {
  api = new Function(body)()
} catch (e) {
  console.log(`✗ 抽出来的真函数**跑不起来**：${String(e.message).slice(0, 200)}`)
  console.log('    ⚠️ 多半是 `TimelineEditor.vue` 的接线形态变了，本判据的抽取锚点要跟着改。')
  process.exit(1)
}

// ---- ① 两栏都非空，且**同一条轴上的种类数**是设计值 ----
{
  const s = api.kindsOnAxis('style')
  const e = api.kindsOnAxis('effect')
  // ⚠️⚠️ **样式栏 10 类 / 动效栏 9 类 —— 差的就是「不设」那一项**（用户口径 2026-10-13）。
  //    第一版把 `kind: 'none'`（当时叫「不动效」）塞进了**两栏**、**都排第一** ⇒
  //    样式栏第一项就是「不动效」，说明还写着"这一条文字不做任何动效" ——
  //    用户当场指出：一个**形状**的下拉里出现"什么都不做"，而且它**像被否决的
  //    那套方案又回来了**。
  //    ⇒ 两栏的"空"语义不同：动效栏的"空"= **不额外加动效**（有意义的一档）；
  //      样式栏的"空"= **不选形状**（缺省，不是一档效果）。
  check(s.length === 11,
    `样式栏种类数应为 11（倾斜/砸入/弹出/逐字×4/竖排… + 末尾的「清除」），实得 ${s.length}：${s.join(',')}`)
  check(e.length === 9,
    `动效栏种类数应为 9（抖动/摇摆/砸入/收缩/弹出/逐字抖散·弧形·错峰 + 末尾的「不设」），实得 ${e.length}：${e.join(',')}`)
  // ⚠️⚠️ **两栏的"空"不是同一件事：两栏都要有，但叫法必须不同**。
  //
  //    用户口径（2026-10-13）：面板上只许有「样式 × 动效」这一对槽位。
  //    第一版把 `none` 两栏都叫「不动效」⇒ 样式栏第一项是「不动效」：
  //    一个**形状**的下拉里出现"什么都不做"，而且「不动效」三个字里带个"动效"。
  //    用户当场指出它**像被否决的那套方案又回来了**。
  //    ⇒ 样式栏叫「清除」（**操作**），动效栏叫「不设」（**选择**）。
  //
  //    ⚠️ **但两栏都必须留着它**：样式栏若没有「清除」，用户**选错了就再也
  //       清不掉**（只能手改 toml）—— 这是"删掉不动效"最容易掉进去的坑。
  check(s.includes('none') && e.includes('none'),
    `两栏都必须有"空"那一项（样式=${s.includes('none')} / 动效=${e.includes('none')}）\n` +
    '     ⚠️ 样式栏少了它 ⇒ 用户选错形状后**清不掉**，只能手改 toml')
  check(api.labelOn('style', 'none') === '清除',
    `样式栏"空"那一项应当叫「清除」（**操作**），实得 ${JSON.stringify(api.labelOn('style', 'none'))}`)
  check(api.labelOn('effect', 'none') === '不设',
    `动效栏"空"那一项应当叫「不设」（**选择**），实得 ${JSON.stringify(api.labelOn('effect', 'none'))}`)
  // ⚠️ **"空"那一项不许排第一** —— 排第一会被当成一个正常的档位读（用户就是这么读的）。
  check(s[s.length - 1] === 'none',
    `样式栏的"空"必须排在**最后**，实得第 ${s.indexOf('none') + 1}/${s.length} 位：${s.join(',')}`)
  check(e[e.length - 1] === 'none',
    `动效栏的"空"必须排在**最后**，实得第 ${e.indexOf('none') + 1}/${e.length} 位：${e.join(',')}`)
  // ⚠️ `both` 的类（slam/pop/arc/scatter/hop）**两栏都必须在** —— 它们确实两边都给参数。
  for (const k of ['slam', 'pop', 'arc', 'scatter', 'hop']) {
    check(s.includes(k), `样式栏缺了 \`both\` 的类 \`${k}\``)
    check(e.includes(k), `动效栏缺了 \`both\` 的类 \`${k}\``)
  }
  // ⚠️ 纯动效的类**不该**出现在样式栏（那会让用户以为它有形态效果）。
  check(!s.includes('shake'), '样式栏不该有 `shake`（纯动效，没有形态参数）')
  check(!e.includes('tilt'), '动效栏不该有 `tilt`（纯形态，没有时间参数）')
}

// ---- ② 核心：`竖排 + 抖动` 两栏**共存** ----
{
  api.reset()
  api.kind('style', 'vtext'); api.variant('style', 'vtext_bold')
  const afterStyle = api.kind('effect', 'shake')
  check(afterStyle.style_anim === 'vtext_bold',
    `选了动效之后样式**被清掉**了：${JSON.stringify(afterStyle)} —— 这就是"互斥"那一版的错法`)
  const r = api.variant('effect', 'shake_std')
  check(r.style_anim === 'vtext_bold' && r.effect_anim === 'shake_std',
    `「竖排 + 抖动」没有共存：${JSON.stringify(r)}\n     ⚠️ 两轴共用一个字段、或互相清空，都会长这样`)
}

// ---- ③ 反序也一样（先动效后样式）----
{
  api.reset()
  api.kind('effect', 'shake'); api.variant('effect', 'shake_lg')
  api.kind('style', 'vtext'); const r = api.variant('style', 'vtext_thin')
  check(r.effect_anim === 'shake_lg' && r.style_anim === 'vtext_thin',
    `反序设置丢了东西：${JSON.stringify(r)}`)
}

// ---- ④ `both` 的类：从任一栏选，两轴都要拿到（否则"带压扁"只生效一半）----
{
  api.reset()
  api.kind('effect', 'slam')
  const a = api.variant('effect', 'slam_squash')
  check(a.effect_anim === 'slam_squash' && a.style_anim === 'slam_squash',
    `\`slam_squash\` 的压扁没同步到样式轴：${JSON.stringify(a)}\n` +
    `     ⚠️ 压扁来自 \`scale_y\`（形态侧）—— 只写动效轴 ⇒ **压扁不生效**，而界面看着是选中的`)
}

// ---- ⑤ 最阴的一条：另一轴已有别的档时，**不许**被偷走 ----
{
  api.reset()
  api.kind('style', 'vtext'); api.variant('style', 'vtext_bold')
  api.kind('effect', 'slam')
  const r = api.variant('effect', 'slam_squash')
  check(r.style_anim === 'vtext_bold',
    `把用户设好的「竖排」**偷走了**：${JSON.stringify(r)}\n` +
    `     ⚠️ 点一下动效栏的 \`both\` 类就冲掉另一轴 —— 用户的车会被偷`)
  check(r.effect_anim === 'slam_squash', `动效轴没设上：${JSON.stringify(r)}`)
}

// ---- ⑥ 空值必须落 `null`（不写 `''`）—— R4：空串会写进 toml ----
{
  api.reset()
  api.kind('style', 'vtext')            // 先设上
  const r = api.kind('style', 'none')   // 再选"不动效"
  check(r.style_anim === null,
    `清空时写成了 ${JSON.stringify(r.style_anim)} —— 应当是 \`null\`\n` +
    `     ⚠️ 空串是一个**合法的坏值**，写进 toml 会让老工程多一个键（R4 破）`)
}

// ---- ⑦ 三份实现 vs **权威归属表**：逐族/逐档对拍 ----
//
// ⚠️ 「样式 × 动效」现在有**四份实现**：转译器 / 预览 `engine.js` /
//    界面 `ANIM_KINDS.axis` / 归属表 `scripts/anim-attribution.mjs`（权威）。
//    漂了的表现是"预览能叠加、成片不能"（或反过来）而**两边各自看都正常**。
//
// ⚠️⚠️ **族内的归属可以不一样**（`slam` 里只有 `slam_squash` 是 `both`、
//    `pop` 里只有 `pop_perchar` 是 `both`）⇒ 判定族该在哪一栏，要看
//    **族内有没有任何一档**给那一侧的参数，**不是**看某一档。
//    （第一版判据写成"族内必须同归属"，于是 `slam`/`pop`/`shrink_in` 报假红 ——
//      真因是判据的口径错，不是代码错。）
{
  const A = ATTRIBUTION
  const famOf = (n) => FAMILIES.find((p) => n === p || n.startsWith(p + '_'))
  const byFam = {}
  for (const [n, e] of Object.entries(A)) {
    const f = famOf(n)
    if (!f) continue
    byFam[f] ??= new Set()
    byFam[f].add(e.kind)
  }
  const expAxis = {}
  for (const [f, s] of Object.entries(byFam)) {
    const hasF = s.has('form') || s.has('both')
    const hasM = s.has('motion') || s.has('both')
    expAxis[f] = hasF && hasM ? 'both' : hasF ? 'style' : hasM ? 'effect' : 'none'
  }
  // ⑦a 界面 `ANIM_KINDS.axis`
  for (const k of ANIM_KINDS) {
    if (k.kind === 'none') continue
    check(k.axis === expAxis[k.kind],
      `界面族 \`${k.kind}\` 的 \`axis\` 是 \`${k.axis}\`，按归属表应当是 \`${expAxis[k.kind]}\`\n` +
      `     ⚠️ 该族里有这些归属：${[...(byFam[k.kind] ?? [])].sort().join(',')} —— ` +
      `族内有 \`form\`/\`both\` ⇒ 要出现在样式栏；有 \`motion\`/\`both\` ⇒ 要出现在动效栏`)
  }
  // ⑦b 预览的两条 has*Part
  //
  // ⚠️⚠️ **`fade_only` 是唯一一条"归属表说 none、但动效路要 true"的档**，
  //    而且**这是有意的**：它一个字段都不给（动的是 opacity），但必须**传下去**
  //    —— `animMotion` 靠"取没取到这一档"决定要不要**压掉全局 `[elements]` 的弹入**。
  //    拆成空的话 `fade_only` 会静默变成"弹入 + 淡出"，而用户要的是"只淡出"。
  //    ⇒ 这里把它写成**具名例外**（不是放宽判据）：其它任何档对不上都照红。
  const MOTION_SEMANTIC_ONLY = new Set(['fade_only'])
  for (const [n, e] of Object.entries(A)) {
    const wf = e.kind === 'form' || e.kind === 'both'
    const wm = e.kind === 'motion' || e.kind === 'both' || MOTION_SEMANTIC_ONLY.has(n)
    check(PREVIEW.animHasFormPart(n) === wf,
      `预览 \`animHasFormPart('${n}')\` = ${PREVIEW.animHasFormPart(n)}，按归属表应当是 ${wf}`)
    check(PREVIEW.animHasMotionPart(n) === wm,
      `预览 \`animHasMotionPart('${n}')\` = ${PREVIEW.animHasMotionPart(n)}，按归属表应当是 ${wm}\n` +
      `     ⚠️ 唯一合法的例外是 \`fade_only\`（归属 \`none\` 但要有动效语义 —— ` +
      `它靠"取到了没"去压掉全局弹入）`)
  }
}

if (fails.length) {
  console.log(`✗ 「样式 × 动效」可叠加性有问题（${fails.length} / ${checks}）：`)
  for (const f of fails) console.log('    · ' + f)
  process.exit(1)
}
console.log(`✓ 「样式 × 动效」真的可叠加（${checks} 项：` +
  `两栏共存 · 反序不丢 · both 同步两轴 · 不偷另一轴 · 清空落 null）`)
