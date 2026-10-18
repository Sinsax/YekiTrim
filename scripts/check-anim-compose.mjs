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

// ---- ① 三栏都非空，且各自的档集合 = 该轴的档 + 末尾的“空” ----
{
  const s = api.kindsOnAxis('style')
  const t = api.kindsOnAxis('transition')
  const e = api.kindsOnAxis('effect')
  // ⚠⚠ **2026-10-14 三轴收敛后的契约**（用户口径）：
  //    排版 3 档（`tilt`/`arc`/`vtext`）、过渡 2 档（`pop`/`fade`）、动效 4 档
  //    （`shake`/`perchar_big`/`perchar_small`/`heartbeat`）。
  //    ⚠️ 不写死数字：从**归属表**数出每条轴几档再比。
  const tiersOn = (ax) => Object.entries(ATTRIBUTION)
    .filter(([, v]) => v.axis === ax).map(([k]) => k).sort()
  // ⚠️ `kindsOnAxis` 返回的是 **kind 字符串数组**；要展开成档名得回
  //    `ANIM_KINDS` 里取那一项的 `variants`（`perchar` 收着两档）。
  const kindObj = (k) => ANIM_KINDS.find((x) => x.kind === k) ?? { kind: k }
  // ⚠️ `none` 表示“不设”，**不是一档**（`animPreset('')` 返回 `undefined`）⇒ 排除。
  const expand = (kinds) => kinds.flatMap((k) => {
    if (k === 'none') return []
    const o = kindObj(k)
    return o.variants?.length ? o.variants.map((v) => v.value) : [o.kind]
  }).sort()
  for (const [ax, label, list] of [
    ['style', '排版', s], ['transition', '过渡', t], ['effect', '动效', e]
  ]) {
    const onPanel = expand(list)
    const want = tiersOn(ax)
    check(onPanel.join(',') === want.join(','),
      `${label}栏的档集合不对\n     实得 ${JSON.stringify(onPanel)}\n` +
      `     应当 ${JSON.stringify(want)}\n` +
      `     ⚠️ 多了 = 收敛没删干净；少了 = 用户选它、预览里不动`)
  }
  // ⚠⚠ **三栏的“空”都必须有**；叫法上：**排版栏独一份**（「默认」），
  //    **过渡栏与动效栏同为「不设」**。
  //
  //    ⚠️⚠️ 2026-10-15：过渡轴收敛成**单框**之后，它那一栏的“空”不再是「默认弹出」
  //       （那个叫法表达的是“缺省 = 弹出”，而缺省改由 `defaultEvent` 落盘
  //       transition_anim = pop 表达）。所以“三栏两两不同”这条**旧口径已经不成立** ——
  //       硬留着它 = 强迫给过渡轴补回两级结构（用户报过「过渡 / 弹出」两个框）。
  //    ⚠️ 但“排版栏必须独一份”仍然要钉：“回到出厂排版”与“不额外加效果”
  //       是两回事（同词会让用户把排版栏读成“这里没什么可设的”）。
  {
    const words = ['style', 'transition', 'effect'].map((ax) => api.labelOn(ax, 'none'))
    check(words.every((w) => typeof w === 'string' && w.length > 0),
      `有栏缺了“空”那一项：${JSON.stringify(words)}\n` +
      `     ⚠️ 用户选错了就**清不掉**（只能手改 toml）`)
    const [sty, tra, eff] = words
    check(sty !== tra && sty !== eff,
      `排版栏的“空”与另外两栏**重名**：${JSON.stringify(words)}\n` +
      `     ⚠️ “回到出厂排版”与“不额外加效果”是两回事，同词会让用户读错`)
    check(tra === eff,
      `过渡栏与动效栏的“空”应当同名（都叫「不设」）：${JSON.stringify(words)}\n` +
      `     ⚠️ 过渡轴已收敛成单框（2026-10-15），两者是同一种语义：“我不额外指定”`)
  }
  // ⚠️ **“空”那一项不许排第一** —— 排第一会被当成一个正常的档位读。
  for (const [label, names] of [['排版', s], ['过渡', t], ['动效', e]]) {
    check(names[names.length - 1] === 'none',
      `${label}栏的“空”必须排在**最后**，实得第 ${names.indexOf('none') + 1}/${names.length} 位：${names.join(',')}`)
  }
  // ⚠ 一条档位**只出现在一栏**（这就是用户要的“不重复”）。
  {
    const all = [...expand(s), ...expand(t), ...expand(e)]
    check(new Set(all).size === all.length,
      `有档位同时出现在两栏：${JSON.stringify(all)}\n` +
      `     ⚠️ 用户报的「样式和动效内容过于重复」就是这个`)
  }
}

// ---- ② 核心：`竖排 + 弹出 + 抖动` **三轴同时共存** ----
{
  api.reset()
  api.kind('style', 'vtext')
  const afterStyle = api.kind('effect', 'shake')
  check(afterStyle.style_anim === 'vtext',
    `选了动效之后排版**被清掉**了：${JSON.stringify(afterStyle)} ` +
    `—— 这就是“互斥”那一版的错法`)
  const r = api.variant('transition', 'pop')
  check(r.style_anim === 'vtext' && r.effect_anim === 'shake' && r.transition_anim === 'pop',
    `「竖排 + 弹出 + 抖动」没有三轴共存：${JSON.stringify(r)}\n` +
    `     ⚠️ 三轴共用一个字段、或互相清空，都会长成这样`)
}

// ---- ③ 反序也一样（先动效后排版）----
{
  api.reset()
  api.kind('effect', 'shake')
  api.variant('transition', 'fade')
  api.kind('style', 'vtext')
  const r = api.variant('style', 'vtext')
  check(r.effect_anim === 'shake' && r.transition_anim === 'fade' && r.style_anim === 'vtext',
    `反序设置丢了东西：${JSON.stringify(r)}`)
}

// ---- ④ ⚠️ **两条连f的双向性**（与转译器同源）----
//
// ⚠⚠ `perchar_big` / `perchar_small` 归**动效**轴，但它们给的是**形态**字段 `per_char`
//    —— “给不给形态字段”与“归哪条轴”**不是一回事**的唯一实例。
//    它们必须同时进**两条路**（否则用户选「逐字变大」在成片里
//    **字一动不动** —— 实测过：报告里一行差异都没有）。
{
  for (const n of ['perchar_big', 'perchar_small']) {
    check(PREVIEW.animHasFormPart(n) === true,
      `预览说 \`${n}\` **不给形态** —— 它的 \`per_char\` 会丢，成片里字一动不动`)
    check(PREVIEW.animHasMotionPart(n) === true,
      `预览说 \`${n}\` **不走动效路** —— 它归动效轴，不走的话档位丢失`)
  }
  // ⚠ 反向：`pop` / `fade` **只走过渡那一路**。
  //    ⚠⚠ 它们的 `scale_in_*` 是**过渡**语义 —— 不能因为“预设里有 `scale_in_*`”
  //       就把它们当成动效档。实测：收起来会让 `splitAnimTwoAxis('pop')`
  //       从 `{effect:''}` 变成 `{effect:'pop'}` ⇒ 与转译器**不同源**。
  for (const n of ['pop', 'fade']) {
    check(PREVIEW.animHasMotionPart(n) === false,
      `预览把 \`${n}\` 也算进了**动效路** —— 它只该走过渡那一路\n` +
      `     ⚠️ 同时进两路 ⇒ 与转译器不同源 ⇒ 老格式在预览与成片里走不同的路`)
  }
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
// ⚠️ 「排版 × 过渡 × 动效」现在有**四份实现**：转译器 / 预览 `engine.js` /
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
  // ⚠⚠ **2026-10-14、归属表现在直接带 `axis`**（不再从 `kind` 推导）。
  //
  //    原因：收敛后**每条档位只属一条轴**，而“给不给形态字段”
  //    与“归哪条轴”**是两件事**（`perchar_big` 归**动效**轴却给 `per_char`）
  //    ⇒ 旧的 `kind`（`'form'|'motion'|'both'|'none'`）**表达不了事实**了。
  //
  //    ⚠️⚠️ **面板的 `kind` 与档名不是一个东西**：
  //      面板把 `perchar_big` / `perchar_small` 收在**一个** kind（`perchar`）下的
  //      `variants` 里（用户口径「合并逐字的效果作为一个动效」）
  //      —— 所以不能拿档名直接去查 `k.kind`。
  //      这里按“每个 kind 展开成它真正负责的档名”对账。
  const kindTiers = (k) => (k.variants?.length
    ? k.variants.map((v) => v.value)
    : [k.kind])
  const byFam = {}
  for (const [n, e] of Object.entries(A)) {
    const f = famOf(n)
    if (!f) continue
    byFam[f] ??= new Set()
    byFam[f].add(e.axis)
  }
  // ⚠ 族内归属必须**唯一** —— 收敛后每条档只属一条轴。
  for (const [f, st] of Object.entries(byFam)) {
    check(st.size === 1,
      `归属表里档名\`${f}\`组内出现了**多条轴**：${[...st].sort().join(', ')}` +
      `\n     ⚠️ 收敛后每条档位只属一条轴（用户报的` +
      `「样式和动效内容过于重复」就是这个）`)
  }
  // ⑦a 界面 `ANIM_KINDS.axis`
  for (const k of ANIM_KINDS) {
    if (k.kind === 'none') continue
    const mine = kindTiers(k)
    // 每个 kind 负责的档必须都在归属表里，且**同一条轴**。
    const axes = new Set(mine.map((n) => A[n]?.axis))
    const exp = [...axes][0]
    check(axes.size === 1 && !!exp,
      `界面 kind \`${k.kind}\` 负责的档 ${JSON.stringify(mine)} ` +
      `在归属表里对不上一条轴：${JSON.stringify([...axes])}` +
      `\n     ⚠️ 症状：预览/成片按一条轴取字段，面板按另一条摆 —— ` +
      `选了它、效果出现在别的轴上，且**不报错**`)
    if (exp) {
      check(k.axis === exp,
        `界面 kind \`${k.kind}\` 的 \`axis\` 是 \`${k.axis}\`，` +
        `按归属表应当是 \`${exp}\`（它负责 ${JSON.stringify(mine)}）`)
    }
  }
  // ⚠ 反向：归属表里每一档都必须在面板上选得到。
  {
    const onPanel = new Set(ANIM_KINDS.flatMap((k) => kindTiers(k)))
    for (const n of Object.keys(A)) {
      check(onPanel.has(n),
        `归属表里的 \`${n}\` 在面板\`ANIM_KINDS\` 里选不到` +
        `\n     ⚠️ 症状：成片认得这一档、用户在面板里根本选不到它`)
    }
  }
  // ⑦b 预览的两条 has*Part
  //
  // ⚠️⚠️ **2026-10-14 三轴收敛后，这一节问的问题变了，判据必须跟着变。**
  //
  //    归属表的 `form[]` / `motion[]` 回答的是"**这一档往产物里写了哪几个字段**"。
  //    而 `animHasFormPart` / `animHasMotionPart` 回答的是**另一个问题**：
  //    "**老格式的单个 `anim` 该拆到哪几路去**"（它们是 `splitLegacyAnim` 的分路器）。
  //
  //    ⚠️ 这两个问题的答案**故意不完全重合**，别拿一个去校验另一个：
  //      · `arc` 归**排版**轴，但它的逐字 **随时间转** ⇒ **两路都要给**
  //        （只给排版的话，老工程 `anim="arc"` 的"逐字动"会静默变成"静态阶梯"）；
  //      · `perchar_big`/`perchar_small` 归**动效**轴，但它们给的是**形态**字段
  //        （`per_char`）⇒ **两路都要给**；
  //      · `pop`/`fade` 归**过渡**轴，走**第三路** ⇒ 两路**都不给**。
  //
  //    ⇒ 判据 = "**与转译器那份定义同源**"，而不是"与轴一一对应"。
  //      权威是转译器的 `animHasFormPart` / `animHasMotionPart`（它才是写进底座的那个），
  //      预览那份必须**逐档一致** —— 漂了的表现就是"老格式在预览与成片里不一样"。
  //
  //    ⚠️ 转译器那两个谓词是**函数内局部常量**，所以从源码里抽出来**真跑**
  //      （与 `check-anim-two-axis.mjs` 抽 `animAxisOf` 同一个手法），
  //      **不在本判据里复刻一份语义** —— 复刻的那份会与被测的那份一起漂。
  const transSrc2 = fs.readFileSync(path.join(ROOT, 'tools', 'polish-to-dhampir.mjs'), 'utf8')
  /** 抽出 `const NAME = (...) => { ... };` 的**整个声明**（花括号匹配）。
   *  ⚠️ 必须**花括号匹配**，不能用“找到下一个 `\n  };`” ——
   *     谓词体里的 `if (...) { ... }` 会提前碰上那个字面量。 */
  const grabFn = (name) => {
    const i = transSrc2.indexOf(`const ${name} = `)
    if (i < 0) return null
    const open = transSrc2.indexOf('{', i)
    if (open < 0) return null
    let depth = 0
    for (let k = open; k < transSrc2.length; k++) {
      const c = transSrc2[k]
      if (c === '{') depth++
      else if (c === '}') {
        depth--
        if (depth === 0) {
          const tail = transSrc2[k + 1] === ';' ? k + 2 : k + 1
          return transSrc2.slice(i, tail)
        }
      }
    }
    return null
  }
  /** 抽 `const NAME = ...;`（一行到分号；多行用花括号匹配）。 */
  const grabConst = (name) => {
    const i = transSrc2.indexOf(`const ${name} = `)
    if (i < 0) return null
    const semi = transSrc2.indexOf(';', i)
    const open = transSrc2.indexOf('{', i)
    if (open >= 0 && (semi < 0 || open < semi)) {
      let depth = 0
      for (let k = open; k < transSrc2.length; k++) {
        if (transSrc2[k] === '{') depth++
        else if (transSrc2[k] === '}') {
          depth--
          if (depth === 0) {
            const tail = transSrc2[k + 1] === ';' ? k + 2 : k + 1
            return transSrc2.slice(i, tail)
          }
        }
      }
      return null
    }
    return semi < 0 ? null : transSrc2.slice(i, semi + 1)
  }
  let transHasForm = null
  let transHasMotion = null
  try {
    const axSrc = grabFn('animAxisOf')
    const preSrc = grabFn('animPreset')
    const formSrc = grabFn('animHasFormPart')
    const moSrc = grabFn('animHasMotionPart')
    // ⚠️ `animAxisOf` 依赖 `ANIM_AXIS` 与 `ANIM_AXIS_PREFIXES`，
    //    而这两个**同样是函数内局部常量**（engine 那份也没 export）
    //    ⇒ 一并抽出来，**不在本判据里手抄一份轴表**。
    const axisSrc = grabConst('ANIM_AXIS')
    // ⚠️ 预设表也一并抽（权威是转译器那份）。
    const presetSrc = grabConst('ANIM_PRESETS')
    const pre2Src = grabConst('ANIM_AXIS_PREFIXES')
    if (!axSrc || !preSrc || !formSrc || !moSrc || !axisSrc || !pre2Src || !presetSrc) {
      check(false, '⑦b 抽不全六个声明：' +
        `${JSON.stringify({ axSrc: !!axSrc, preSrc: !!preSrc, formSrc: !!formSrc, moSrc: !!moSrc, axisSrc: !!axisSrc, pre2Src: !!pre2Src, presetSrc: !!presetSrc })}`)
    } else {
      const mk = new Function(`
        ${presetSrc}
        ${axisSrc}
        ${pre2Src}
        ${axSrc}
        ${preSrc}
        ${formSrc}
        ${moSrc}
        return { animHasFormPart, animHasMotionPart };
      `)
      const got = mk()
      transHasForm = got.animHasFormPart
      transHasMotion = got.animHasMotionPart
    }
  } catch (err) {
    check(false, `⑦b 抽出来跑不起来：${err.message}`)
  }
  check(!!transHasForm && !!transHasMotion,
    '⑦b 没能从转译器源码里抽出 `animHasFormPart` / `animHasMotionPart` 来真跑')
  if (transHasForm && transHasMotion) {
    for (const n of Object.keys(A)) {
      check(PREVIEW.animHasFormPart(n) === transHasForm(n),
        `预览 \`animHasFormPart('${n}')\` = ${PREVIEW.animHasFormPart(n)}，` +
        `转译器那份 = ${transHasForm(n)} —— **两侧不同源**\n` +
        `     ⚠️ 症状：老格式 \`anim="${n}"\` 在预览与成片里走不同的路（两边各自看都正常）`)
      check(PREVIEW.animHasMotionPart(n) === transHasMotion(n),
        `预览 \`animHasMotionPart('${n}')\` = ${PREVIEW.animHasMotionPart(n)}，` +
        `转译器那份 = ${transHasMotion(n)} —— **两侧不同源**\n` +
        `     ⚠️ 症状：老格式 \`anim="${n}"\` 的逐字动在一边有、另一边没有`)
    }
  }
}

if (fails.length) {
  console.log(`✗ 「样式 × 动效」可叠加性有问题（${fails.length} / ${checks}）：`)
  for (const f of fails) console.log('    · ' + f)
  process.exit(1)
}
console.log(`✓ 「排版 × 过渡 × 动效」真的可叠加（${checks} 项：` +
  `三栏共存 · 反序不丢 · 各自只写自己那条轴 · 清空落 null · ` +
  `预览与转译器同源）`)
