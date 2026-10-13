// 几何 + 行为断言：动效下拉**真的渲染出来了**（不是"模板里有就算"）。
//
// ⚠️ 为什么必须真跑一遍：**Vue 对未定义的模板标识符不报错、渲染成空白**
//    （本仓踩过，见 webui/src/AGENTS.md 与 `check-layout-invariants.mjs`）。
//    所以"我加了 `<optgroup v-for="g in ANIM_GROUPS">`"这句话，
//    在 `ANIM_GROUPS` 没进 setup 作用域时会**静默什么都不渲染**，
//    而构建照样成功、判据照样绿。
//
// 这里用 `@vue/compiler-sfc` 把组件**真编译一遍**，然后：
//   ① 断言模板里引用的每个标识符都在 `<script setup>` 的绑定里（编译诊断能查出来）；
//   ② 直接把 `ANIM_GROUPS` 抽出来数一遍档数。
import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const HERE = path.dirname(fileURLToPath(import.meta.url))
const ROOT = path.join(HERE, '..')
const FILE = path.join(ROOT, 'webui', 'src', 'components', 'TimelineEditor.vue')

const src = fs.readFileSync(FILE, 'utf8')
const fails = []
let checks = 0
const ok = (cond, what) => {
  checks++
  if (!cond) fails.push(what)
}

// ---- ① 编译一次，看有没有"未定义标识符"这类诊断 ----
//
// `@vue/compiler-sfc` 在 `webui/node_modules` 下（vite 的依赖，必然在）。
const sfcMod = await import(
  new URL('../webui/node_modules/@vue/compiler-sfc/dist/compiler-sfc.cjs.js', import.meta.url).href
)
const sfc = sfcMod.default ?? sfcMod
const parsed = sfc.parse(src, { filename: 'TimelineEditor.vue' })
ok(!parsed.errors || parsed.errors.length === 0,
  `SFC 解析有错：${JSON.stringify((parsed.errors ?? []).map(String))}`)

const script = parsed.descriptor.scriptSetup ?? parsed.descriptor.script
ok(!!script, 'TimelineEditor.vue 没有 <script setup>（判据的前提没了）')

// ⚠️ 编译 `<script setup>` 会拿到 `bindings`（模板可用的标识符全集）。
const compiled = sfc.compileScript(parsed.descriptor, { id: 'x' })
const bindings = new Set(Object.keys(compiled.bindings ?? {}))
// ⚠️ **2026-10-13：下拉改成两轴（样式 × 动效），每轴各自两级（种类 → 细分）**。
//    模板引用的标识符跟着变。这几个**每一个都必须真的在绑定里** ——
//    模板引用了未绑定的名字时 Vue **不报错、渲染成空白**，构建照样成功
//    （这正是本脚本存在的理由）。
//
// ⚠️ **`styleHint` / `effectHint` 已从这张表里去掉**（用户口径 2026-10-13）：
//    每个种类下面那行小字是**多余的**（选项标签已经说清了是什么），
//    连 `hintOnAxis` 与 `ANIM_KINDS` 里的 `hint` 字段一起删了。
//    ⇒ 判据不能只"删名字就算完"：下面 ④ 有一条**反向**断言
//      （模板里不许再出现 `styleHint`/`effectHint`）—— 否则把字段加回来、判据照绿。
for (const id of ['ANIM_KINDS', 'ANIM_KNOWN', 'kindsOnAxis',
                  'styleKind', 'effectKind', 'styleVariants', 'effectVariants',
                  'setKindOnAxis', 'setVariantOnAxis']) {
  ok(bindings.has(id),
    `① \`${id}\` 不在 \`<script setup>\` 的绑定里 —— 模板引用它会渲染成空白**且不报错**`)
}
// ⚠️ **反向**：这两个名字**不许**再被绑定（加回来就是把那行小字加回来）。
for (const id of ['styleHint', 'effectHint', 'hintOnAxis']) {
  ok(!bindings.has(id),
    `① \`${id}\` **又回到绑定里了** —— 那行"每个种类下面的小字"已被用户明确要求删掉`)
}

// ---- ② 档数与种类（直接读源码里的字面量）----
//
// ⚠️ 权威表是 `ANIM_KINDS`（两级）。`ANIM_GROUPS` 是它 `.map()` 出来的兼容视图，
//    **这里不能扒它** —— 它里面一个 `value: '…'` 字面量都没有（是算出来的），
//    扒它会得到 0 档 ⇒ "档数不对"全线假红，而真因是判据扒错了地方。
// ⚠️ 必须**切到定义结束**为止：只 `slice(indexOf(...))` 会一直读到后面的数组
//    （实测把之后另一张带 value 的表也算了进来 ⇒ **判据自己数错了**，代码是对的）。
const gStart = src.indexOf('const ANIM_KINDS = [')
ok(gStart > 0, '② 源码里找不到 `const ANIM_KINDS = [` —— 两级表没了（判据的前提没了）')
const gEnd = src.indexOf('\nconst ', gStart + 1)
const block = gStart > 0 ? src.slice(gStart, gEnd > gStart ? gEnd : undefined) : ''
const values = [...block.matchAll(/\{\s*value:\s*'([a-z0-9_]*)'/g)].map((m) => m[1])
const nonEmpty = values.filter((v) => v !== '')
// 种类：`kind: '...'`（缩进 **4 空格** —— 数组项一层 + 对象体一层；
// ⚠️ 写成 2 会让它一个都取不到 ⇒ 全线假红，而真因是判据自己的正则）。
const kinds = [...block.matchAll(/^\s+kind:\s*'([a-z_]+)',\s*$/gm)].map((m) => m[1])

// ⚠️ 计数**随施工推进而增长**，不是钉死的历史数字：
//    A+B 组 17 档 → 加 C 组 14 档 → 加 D 组 4 档 + E 组 3 档 = 39 档 / 14 族。
//    ⚠️ **2026-10-13 两级化之后**：种类数 = 14（"不动效" + 13 个效果族）。
//    改这里时**必须同时**改 `check-anim-presets-parity.mjs` 的档数 ——
//    两边对不上就说明有一处漏改了。
const WANT_KINDS = 14
const WANT_TIERS = 39
ok(kinds.length === WANT_KINDS,
  `② 种类数应当是 ${WANT_KINDS}（不动效 + 13 个效果族），实得 ${kinds.length}`)
ok(new Set(kinds).size === kinds.length,
  `② 种类里有重复：${JSON.stringify(kinds.filter((k, i) => kinds.indexOf(k) !== i))}`)
ok(values.includes(''), '② 缺"不动效"那一档（空串）')
ok(nonEmpty.length === WANT_TIERS, `② 已知档应当 ${WANT_TIERS} 档，实得 ${nonEmpty.length}`)
ok(new Set(nonEmpty).size === nonEmpty.length, '② 有重复档名')

// ---- ③ 模板真的用了这些标识符（而不只是定义了）----
//
// ⚠️ **两轴 × 两级**：样式那条轴一个"种类"下拉 + 一个"细分"下拉，
//    动效那条轴同样两个 ⇒ **模板里应当有四个 `<select>`**。
//    以前那条 `v-for="g in ANIM_GROUPS"` + `<optgroup>` 的断言**已经作废**
//    （那正是用户说"还是这个"的形态）。
const tpl = parsed.descriptor.template?.content ?? ''
// ⚠️⚠️ **先剥 HTML 注释再匹配** —— 本仓踩过这个假阳性（`check_module_boundaries.py`
//    的上一版手写 grep 判据因此产出 8 条假阳性，等于没用）。
//    这里尤其要紧：**"我把 `<optgroup>` 删掉了"这句话本身必须写在注释里**
//    （否则下一个人会把它加回来），而只匹配文本的判据会**把说明当成违规**。
//    ⇒ 断言只对**真标记**生效。`(?<!--)` 不做可变长回看，所以直接整体剥掉。
const tplMarkup = tpl.replace(/<!--[\s\S]*?-->/g, '')
// ⚠️ 两条轴**各自**要有种类下拉（`kindsOnAxis('style')` / `('effect')`）。
ok(/v-for="k in kindsOnAxis\('style'\)"/.test(tplMarkup),
  "③ 样式那条轴没有种类下拉（`v-for=\"k in kindsOnAxis('style')\"`）—— 那一栏是空的")
ok(/v-for="k in kindsOnAxis\('effect'\)"/.test(tplMarkup),
  "③ 动效那条轴没有种类下拉（`v-for=\"k in kindsOnAxis('effect')\"`）—— 那一栏是空的")
ok(/v-for="a in styleVariants"/.test(tplMarkup), '③ 样式那条轴的细分下拉是空的')
ok(/v-for="a in effectVariants"/.test(tplMarkup), '③ 动效那条轴的细分下拉是空的')
ok(/ANIM_KNOWN\.includes\(/.test(tplMarkup), '③ 模板没有用 `ANIM_KNOWN` 判断未知档')
ok(/styleKind === ''/.test(tplMarkup), '③ 样式栏：认不出的档名没有落到"未知"那一支')
ok(/effectKind === ''/.test(tplMarkup), '③ 动效栏：认不出的档名没有落到"未知"那一支')
// ⚠️⚠️ **两轴必须绑到两个不同的字段** —— 绑同一个就是"互斥"（回退掉的那版错法）。
ok(/selItem\.style_anim/.test(tplMarkup) && /selItem\.effect_anim/.test(tplMarkup),
  '③ 模板没有分别绑到 `selItem.style_anim` 与 `selItem.effect_anim` —— ' +
  '两轴共用一个字段 ⇒ 用户选样式会把动效清掉（**可叠加**就不成立了）')
// ⚠️ **只对真标记断言**（`tplMarkup`）—— 见上面"先剥注释"那段。
ok(!/<optgroup/.test(tplMarkup),
  '③ 又出现了 `<optgroup>` —— 那是"四十项平铺进一个下拉"的旧形态，用户报过"还是这个"')
// ⚠️ 未知档必须**有 option**（否则 `<select>` 的值匹配不上任何一项，
//    浏览器会显示成第一项——用户以为自己没选错）。**两轴各一条**。
{
  const unknownCount = (tplMarkup.match(/未知档/g) ?? []).length
  ok(unknownCount >= 2,
    `③ "未知档"那条 option 只有 ${unknownCount} 条 —— 两轴各要一条（否则某栏会静默显示成"无"）`)
}

// ---- ④ ⚠️ **面板上只许有"样式 × 动效"这一对槽位**（用户口径 2026-10-13）----
//
// ⚠️⚠️ 这一段的由来：用户**两次**指出面板上还挂着**早已被否决的那套四档样式**
//    （hook/chapter/comment/count，`selItem.style`）。
//    第一次我没看懂；第二次他说得更直白：「都说过几次这个样式是被抛弃的，
//    用的是当前这个样式(形态)+动效的双槽位，你还要留个之前否定的方案在上面。」
//    ⇒ 删掉它，并且**把"不许加回来"变成机器判据** —— 光删一次挡不住下一个人。
//
// ⚠️ 为什么必须断言"**没有**"而不只是"有那两个新下拉"：
//    第一版就是**只断言了有**，所以旧的 `样式` 下拉一直躺在上面、判据全绿。
{
  // ⚠️ **标签必须正好是这两个词** —— 用户明确要求：样式就叫「样式」，
  //    动效就叫「动效」（不要「样式（形态）」「动效（随时间）」那种括号注解）。
  const labels = [...tplMarkup.matchAll(/<label>([^<]*)<\/label>/g)].map((m) => m[1])
  ok(labels.includes('样式'), '④ 面板上没有「样式」这个标签')
  ok(labels.includes('动效'), '④ 面板上没有「动效」这个标签')
  // ⚠️ 带括号注解的旧写法不许回来。
  for (const bad of ['样式（形态）', '动效（随时间）']) {
    ok(!tplMarkup.includes(bad),
      `④ 又出现了「${bad}」—— 用户要求标签就是「样式」和「动效」，不带括号注解`)
  }
  // ⚠️⚠️ **被否决的四档样式下拉不许回来**：它绑的是 `selItem.style`。
  //    判据盯**绑定**（`selItem.style` 的赋值/读取），不是盯文案 ——
  //    改个标签就绕过去了。
  ok(!/selItem\.style\s*=/.test(tplMarkup) && !/TEXT_STYLES/.test(tplMarkup),
    '④ 被否决的四档样式下拉**又回到面板上了**（`selItem.style` / `TEXT_STYLES`）\n' +
    '     ⚠️ 用户口径：那套四档早已被否决，现在只有「样式 × 动效」这一对槽位。')
  // ⚠️ **每个种类下面那行小字已删**（用户说"这些文案，没有必要"）。
  ok(!/styleHint|effectHint|hintOnAxis/.test(tplMarkup),
    '④ 模板又引用了 `styleHint`/`effectHint` —— 那行小字已被用户要求删掉')
  // ⚠️ 用户同时点名的两句废话：重复的落点说明 + "两栏可同时设"。
  const dupPos = (tplMarkup.match(/位置 x\/y 由后处理算/g) ?? []).length
  ok(dupPos === 0,
    `④ 又出现了「位置 x/y 由后处理算」${dupPos} 处 —— 用户明确说这些文案没有必要`)
  ok(!tplMarkup.includes('两栏可同时设'),
    '④ 又出现了「两栏可同时设（如「竖排 + 抖动」）」—— 用户说这句没有必要')
  // ⚠️ 「不动效」这个说法只许留在**动效**那一栏，且标签已改成「不设」。
  ok(!tplMarkup.includes('不动效'),
    '④ 又出现了「不动效」—— 它现在叫「不设」，而且只在动效栏')
}

// ---- ⑤ ⚠️ **两栏里都出现的档，标签必须带轴后缀**（2026-10-14）----
//
// # 由来
//
// 用户报「当前前端预览的设置内，**样式和动效内容过于重复**？是不是之前去重后
// 有些内容没有清理」。
//
// 实测查清了两件事，必须一起看：
//  ① **39 档零重复归属、面板一个档都没丢**（`check-anim-presets-parity.mjs` 钉着）
//     ⇒ 用户的猜测（去重残留）**不成立**；
//  ② 但归属表是 `形态 15 · 动效 12 · **两者 11** · 无 1` —— 那 11 档**真的两条轴
//     都给参数** ⇒ 两栏都列是**必需的**，而两栏显示**同一个名字**，读起来像"重复"。
//
// ⇒ 判据：**出现在两栏的种类，两栏的显示标签必须不同**；只属一栏的必须原样。
//    ⚠️ 这条压的是"呈现"，`variants[].value` 与 `kind` 一个都不许动。
{
  // 从源码扒 ANIM_KINDS（与其它段同一口径：不 import 应用模块）。
  // ⚠️ 用文件顶部的 `src`（= `TimelineEditor.vue` 全文），别在这里再 readFileSync 一份。
  const block = /const ANIM_KINDS = \[([\s\S]*?)\n\]/.exec(src)?.[1] ?? ''
  ok(block.length > 0, '⑤ 扒不到 `ANIM_KINDS` —— 下面几条会全部空转（比没有判据更糟）')
  const kinds = [...block.matchAll(/kind: '([a-z_]+)',\s*\n\s*label: '([^']+)',\s*\n\s*axis: '([a-z]+)'/g)]
    .map((m) => ({ kind: m[1], label: m[2], axis: m[3] }))
  ok(kinds.length === WANT_KINDS,
    `⑤ 扒到 ${kinds.length} 个种类，应是 ${WANT_KINDS} —— 扒取正则与源码形状分叉了`)
  const axesOf = (axis) => kinds.filter((k) => k.kind === 'none' || k.axis === axis || k.axis === 'both')
  const styleKinds = axesOf('style')
  const effectKinds = axesOf('effect')
  const bothKinds = kinds.filter((k) => k.axis === 'both')
  ok(bothKinds.length > 0,
    '⑤ 一个 `axis: "both"` 的档都没有 —— 那两栏就不该有重复项，这条判据的前提不成立')
  // ⚠️⚠️ **必须跑源码里那个真的 `kindLabelOn`，不许在判据里复刻一份**。
  //
  //    第一版就是复刻的（`const shown = (axis,k) => ...`）—— 实测**两处变异都存活**：
  //    改源码的 `kindLabelOn`（去掉轴后缀 / 给只属一栏的也加后缀），判据照样全绿，
  //    因为它跑的是**自己那份副本**。这正是本仓反复踩的"判据测到了函数、
  //    没测到实现"那一类 —— 判据绿得毫无意义。
  //
  //    ⇒ 把 `AXIS_SUFFIX` 与 `kindLabelOn` **逐字从源码里抠出来、用 `new Function`
  //      真跑**。这样改源码就一定被观测到。
  const suffixSrc = /const AXIS_SUFFIX = \{[^}]*\}/.exec(src)?.[0] ?? ''
  const labelFnSrc = /const kindLabelOn = \(axis, k\) => \{[\s\S]*?\n\}/.exec(src)?.[0] ?? ''
  ok(suffixSrc.length > 0, '⑤ 源码里抠不到 `AXIS_SUFFIX` —— 下面的判据会空转')
  ok(labelFnSrc.length > 0, '⑤ 源码里抠不到 `kindLabelOn` 的函数体 —— 下面的判据会空转')
  let shown = () => {
    throw new Error('kindLabelOn 没抠出来')
  }
  if (suffixSrc && labelFnSrc) {
    // 只喂常量与那个函数，不引入任何应用模块（与其它段同口径的零依赖）。
    shown = new Function(`${suffixSrc}\n${labelFnSrc}\nreturn kindLabelOn`)()
  }
  ok(typeof shown === 'function', '⑤ 抠出来的 `kindLabelOn` 不是函数 —— 判据会空转')
  // ① 两栏都有的档：两栏显示必须**不同**（否则用户读到的就是"重复"）。
  for (const k of bothKinds) {
    const a = shown('style', k)
    const b = shown('effect', k)
    ok(a !== b,
      `⑤ 档「${k.label}」(axis=both) 在两栏显示成同一个「${a}」—— ` +
      '用户会把两栏读成"重复了一半"。必须带轴后缀（形态 / 动效）')
  }
  // ② 只属一栏的档：**不许**加后缀（那是纯噪音，它本来就不会歧义）。
  for (const k of kinds.filter((x) => x.axis === 'style' || x.axis === 'effect')) {
    const ax = k.axis
    ok(shown(ax, k) === k.label,
      `⑤ 只属「${ax}」栏的档「${k.label}」被加了后缀（显示成「${shown(ax, k)}」）—— ` +
      '只属一栏的档不会歧义，加后缀是噪音')
  }
  // ③ 两栏各自的**显示标签集合**不许有交集（这就是"看起来重复"的可检形式）。
  const styleLabels = new Set(styleKinds.map((k) => shown('style', k)))
  const effectLabels = new Set(effectKinds.map((k) => shown('effect', k)))
  const overlap = [...styleLabels].filter((l) => effectLabels.has(l))
  ok(overlap.length === 0,
    `⑤ 两栏仍有同名条目：${overlap.join('、')} —— 用户报的就是这件事`)

  // ⚠️ ④ **源码侧的接线判据**：模板必须真的把 axis 传进去。
  //    只判"算出来的标签不同"是不够的 —— 模板里万一调的是不带 axis 的旧签名，
  //    算得再对也不生效（本仓反复踩过"测到函数、没测到调用方"）。
  //    ⚠️ `AXIS_SUFFIX` / `kindLabelOn` **是否存在**已由上面的"抠出来跑"覆盖
  //      （抠不到就直接断了），这里只管**模板调用点**。
  for (const ax of ['style', 'effect']) {
    ok(new RegExp(`kindLabelOn\\('${ax}', k\\)`).test(tplMarkup),
      `⑤ 模板里「${ax}」那一栏没有把 axis 传给 kindLabelOn（显示会退回不带后缀的名字）`)
  }
}

// ---- 结果 ----
if (fails.length === 0) {
  console.log(
    `✓ 动效下拉真的接上了（${checks} 项：SFC 编译绑定 + 两级 ${WANT_KINDS} 种类 / ${WANT_TIERS} 档 + 模板接线）`,
  )
  process.exit(0)
}
console.log(`✗ 动效下拉有问题（${fails.length} / ${checks}）：`)
for (const f of fails) console.log('    ' + f)
process.exit(1)
