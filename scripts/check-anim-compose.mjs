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

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const VUE = path.join(ROOT, 'webui', 'src', 'components', 'TimelineEditor.vue')
const src = fs.readFileSync(VUE, 'utf8')

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
  check(s.length === 11, `样式栏种类数应为 11（不动效 + 倾斜/砸入/弹出/逐字×4/竖排…），实得 ${s.length}：${s.join(',')}`)
  check(e.length === 9, `动效栏种类数应为 9（不动效 + 抖动/摇摆/砸入/收缩/弹出/逐字抖散·弧形·错峰），实得 ${e.length}：${e.join(',')}`)
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

if (fails.length) {
  console.log(`✗ 「样式 × 动效」可叠加性有问题（${fails.length} / ${checks}）：`)
  for (const f of fails) console.log('    · ' + f)
  process.exit(1)
}
console.log(`✓ 「样式 × 动效」真的可叠加（${checks} 项：` +
  `两栏共存 · 反序不丢 · both 同步两轴 · 不偷另一轴 · 清空落 null）`)
