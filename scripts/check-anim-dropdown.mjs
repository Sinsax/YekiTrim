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
//   ② 直接把 `ANIM_KINDS` 抽出来数一遍档数与轴归属；
//   ③/④ 断言三栏接线 + 被否决的那套方案不许回来；
//   ⑤ 把 `kindsOnAxis` / `kindLabelOn` 的**真函数体逐字抠出来真跑**
//      （三栏档集合 · "空"排最后 · 只属一栏不加后缀 · **排版栏的"空"独一份、
//   ⑥ 反向断言 `AXIS_SUFFIX` **不再存在**；
//   ⑦ 把 `setKindOnAxis` / `setVariantOnAxis` 抠出来**真跑**（只动自己那一轴）；
//   ⑧ 把 `kindOnAxis` 抠出来**真跑**（认不出的档名不许被静默归成"无"）。
//
// ============================================================================
// 2026-10-14：**两轴 39 档 → 三轴 9 档**收敛，本判据跟着重写
// ============================================================================
//
// | 轴（内部名） | 显示名 | 事件字段 | 档位 |
// |---|---|---|---|
// | `style` | 排版 | `style_anim` | `tilt` · `vtext` · `arc`（"默认" = 空档名）|
// | `transition` | 过渡 | `transition_anim` | `pop` · `fade` |
// | `effect` | 动效 | `effect_anim` | `shake` · `perchar_big` · `perchar_small` · `heartbeat` |
//
// 两条一起看才成立的后果：
//   * 面板从**两栏**变**三栏**（「样式」+「动效」→「排版」/「过渡」/「动效」）；
//   * **`AXIS_SUFFIX` 已删** —— 它是上一版给 `axis:'both'` 的档打的补丁
//     （同一个档名在两栏里显示成「砸入 · 形态」/「砸入 · 动效」）。收敛后
//     **每条档位只属一条轴** ⇒ 两栏不可能重名 ⇒ 后缀失去意义。
//     判据 ⑥ 显式断言它**不再存在**（理由见那边）。
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

/** 三条轴（顺序即面板上的顺序）。 */
const AXES = ['style', 'transition', 'effect']

// ---- 从源码里"扒真源"的三个工具 -------------------------------------------
//
// ⚠️⚠️ 判据的纪律：**别重写一份实现**。下面这些只做"定位与搬运"，
//    任何业务语义（归属 / 标签 / 过滤 / 落盘值）都只在**源码文本**或
//    **源码里那个真函数**里有一份。
//    （上一版的 `shown` 是判据自己复刻的 `kindLabelOn` ⇒ 两处变异都存活，
//      判据绿得毫无意义。这一版把它抠出来**真跑**。）

/** 按**字符配对**取一段（`[]` / `{}` 各一个版本）。
 *  ⚠️ 别用正则猜结尾 —— 函数体/数组项里嵌套的括号会让它早停/晚停，**静默**抽错。 */
function span(marker, open, close) {
  const i = src.indexOf(marker)
  if (i < 0) return ''
  const b = src.indexOf(open, i)
  if (b < 0) return ''
  let d = 0
  let started = false
  let j = b
  for (; j < src.length; j++) {
    if (src[j] === open) { d++; started = true } else if (src[j] === close) {
      d--
      if (started && d === 0) { j++; break }
    }
  }
  return src.slice(b, j)
}

/** 位置**最靠前**的那个断言用的函数体（抠不到 ⇒ `''`）。
 *  ⚠️ 全部用 `[^)]*?` / `[\s\S]*?\n\}` 这种**紧跟形状**的锚点，
 *    抠不到就是判据失败（见各段的 `ok(...)`），**不许退化成"跳过"。*/
const grab = () => ({
  animKindsLit: span('const ANIM_KINDS = [', '[', ']'),
  kindsOnAxis: /const kindsOnAxis = \(axis\) =>[\s\S]*?\.sort\([^\n]*\)/.exec(src)?.[0] ?? '',
  kindLabelOn: /const kindLabelOn = \(axis, k\) => \{[\s\S]*?\n\}/.exec(src)?.[0] ?? '',
  kindOnAxis: /const kindOnAxis = \(cur\) => \{[\s\S]*?\n\}/.exec(src)?.[0] ?? '',
  axisFields: /const AXIS_FIELDS = \{[^}]*\}/.exec(src)?.[0] ?? '',
  axisField: /const axisField = \(axis\) =>[^\n]*/.exec(src)?.[0] ?? '',
  setKindOnAxis: /function setKindOnAxis\(axis, kind\) \{[\s\S]*?\n\}/.exec(src)?.[0] ?? '',
  setVariantOnAxis: /function setVariantOnAxis\(axis, value\) \{[\s\S]*?\n\}/.exec(src)?.[0] ?? '',
})
const G = grab()

/** 把 `ANIM_KINDS` 的字面量**原样**放进一个 `new Function` 里跑出来的真表。
 *  ⚠️ 是**执行**源码里那张表，不是"另写一份" —— 加一档、改一个 label，
 *    这里跟着变，判据不需要同步改。
 *  ⚠️ 抠不到就返回 `[]`，但**调用方必须把它当失败**（见 ② 的 `ok`）。*/
function realKinds() {
  if (!G.animKindsLit) return []
  try {
    return new Function(`return ${G.animKindsLit}`)()
  } catch {
    return []
  }
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
// ⚠️ **2026-10-14：下拉从两轴（样式 × 动效）收敛成三轴（排版 × 过渡 × 动效）**。
//    模板引用的标识符跟着变。这几个**每一个都必须真的在绑定里** ——
//    模板引用了未绑定的名字时 Vue **不报错、渲染成空白**，构建照样成功
//    （这正是本脚本存在的理由）。
for (const id of ['ANIM_KINDS', 'ANIM_KNOWN', 'kindsOnAxis',
                  'styleKind', 'transitionKind', 'effectKind',
                  'styleVariants', 'transitionVariants', 'effectVariants',
                  'setKindOnAxis', 'setVariantOnAxis',
                  // ⚠️ 排版角度滑杆（2026-10-14 新增）——模板引用了它才显示得出来。
                  'showAngle', 'LAYOUT_ANGLE_MIN', 'LAYOUT_ANGLE_MAX',
                  'layoutAngle', 'layoutAngleSet', 'setLayoutAngle', 'resetLayoutAngle']) {
  ok(bindings.has(id),
    `① \`${id}\` 不在 \`<script setup>\` 的绑定里 —— 模板引用它会渲染成空白**且不报错**`)
}
// ⚠️⚠️ **反向**：`AXIS_SUFFIX` **不许**再被绑定（2026-10-14 三轴收敛时删掉）。
//    绑定层与源码文本层（⑥）两处都钉 —— 前者管"有没有人用它"，后者管"它在不在"。
ok(!bindings.has('AXIS_SUFFIX'),
  '① `AXIS_SUFFIX` **又回到绑定里了** —— 它是"档可以属于两条轴"那个旧模型的补丁')
// ⚠️ **反向**：这两个名字**不许**再被绑定（加回来就是把那行小字加回来）。
for (const id of ['styleHint', 'effectHint', 'hintOnAxis']) {
  ok(!bindings.has(id),
    `① \`${id}\` **又回到绑定里了** —— 那行"每个种类下面的小字"已被用户明确要求删掉`)
}

// ---- ② 档数、种类与轴归属（执行源码里那张真表）----
//
// ⚠️ 权威表是 `ANIM_KINDS`（两级）。`ANIM_GROUPS` 是它 `.map()` 出来的兼容视图，
//    **这里不能扒它** —— 它里面一个 `value: '…'` 字面量都没有（是算出来的），
//    扒它会得到 0 档 ⇒ "档数不对"全线假红，而真因是判据扒错了地方。
//
// ⚠️⚠️ **档数不许写死历史数字（39 / 14）** —— 那是收敛**前**的形态，
//    留在判据里会让"收敛到 9 档"这件正确的事**报红**，而报红的信息是错的。
//    ⇒ 从 `ANIM_KINDS` 里**数**（`realKinds()` 真跑源码），并断言契约数字：
//      **8 个 kind / 9 个非空档**。
//
//    8 个 kind = 1 个 `kind:'none'`（三栏都进）+ 3 个排版（tilt / arc / vtext）
//    + 1 个过渡（pop/fade 两支）+ 3 个动效（shake / perchar 两支 / heartbeat）。
//    9 个非空档 = tilt · arc · vtext · pop · fade · shake · perchar_big ·
//                 perchar_small · heartbeat。
const WANT_KINDS = 8
const WANT_TIERS = 9
const KINDS = realKinds()
ok(KINDS.length > 0,
  '② 抠不到 `const ANIM_KINDS = [` 的真表 —— 档数 / 轴归属全都无从判起（判据的前提没了）')
const kindNames = KINDS.map((k) => k.kind)
const allValues = KINDS.flatMap((k) => (k.variants ?? []).map((v) => v.value))
const nonEmpty = allValues.filter((v) => v !== '')
ok(kindNames.length === WANT_KINDS,
  `② 种类数应当是 ${WANT_KINDS}（1 个"空" + 排版 3 + 过渡 1 + 动效 3），实得 ` +
  `${kindNames.length}：${JSON.stringify(kindNames)}`)
ok(new Set(kindNames).size === kindNames.length,
  `② 种类里有重复：${JSON.stringify(kindNames.filter((k, i) => kindNames.indexOf(k) !== i))}`)
ok(allValues.includes(''), '② 缺"空"那一档（空串）—— 用户就没法取消/回到默认了')
ok(nonEmpty.length === WANT_TIERS,
  `② 非空档应当 ${WANT_TIERS} 档（三轴收敛后的契约），实得 ${nonEmpty.length}：` +
  JSON.stringify(nonEmpty))
ok(new Set(nonEmpty).size === nonEmpty.length, '② 有重复档名')

// ⚠️ 逐轴计数：**每条轴都得有档**，且 `both` 不许回来。
{
  const axCount = new Map()
  for (const k of KINDS) axCount.set(k.axis, (axCount.get(k.axis) ?? 0) + 1)
  for (const want of AXES) {
    ok((axCount.get(want) ?? 0) > 0,
      `② 没有一条档位属于 \`axis:'${want}'\` —— 那一栏整个是空的（实得 ${JSON.stringify([...axCount])}）`)
  }
  // ⚠️⚠️ `both` **不许**回来（2026-10-14 收敛时删掉的那条轴）。
  //    它一旦回来，`kindsOnAxis` 又要把同一档列进两栏 ⇒ 用户报的
  //    「样式和动效内容过于重复」当场复发。⑥ 在 `AXIS_SUFFIX` 侧再钉一次。
  ok((axCount.get('both') ?? 0) === 0,
    '② 又出现了 `axis: "both"` —— 三轴收敛后每条档位**只属一条轴**；' +
    '`both` 是上一版"档同时属于两条轴"的模型（它让两栏重名）')
}
// ⚠️ `ANIM_KINDS` 里**不许**再留 `angle` 以外的新轴名 —— 轴名是闭集。
{
  const bad = KINDS.filter((k) => ![...AXES, 'none'].includes(k.axis))
  ok(bad.length === 0,
    `② 有档位的 \`axis\` 不是三条轴之一（${[...AXES, 'none'].join(' / ')}）：` +
    JSON.stringify(bad.map((k) => `${k.kind}:${k.axis}`)))
}

// ---- ③ 模板真的用了这些标识符（而不只是定义了）----
//
// ⚠️ **两级只属于排版轴**（2026-10-15 改）。
//
//    排版轴：一个"种类"下拉（倾斜/斜扫/竖排）+ 一个"细分"下拉 —— 两级**各有信息**。
//    过渡轴：**只有一个下拉**，选项就是 `pop` / `fade` 两档本身。
//    ⚠️⚠️ 它原先**照抄**了排版那套两级 ⇒ 种类框只能把自己重复一遍显示成「过渡」、
//       细分框显示「弹出」，面板上就是「过渡 / 弹出」两个框 —— 用户原话
//       「过渡：过渡：弹出，不觉得很蠢吗」「过渡只留弹出和淡入淡出」。
//       **别再给过渡轴补回种类框/细分框**。
//    以前那条 `v-for="g in ANIM_GROUPS"` + `<optgroup>` 的断言**已经作废**
//    （那正是用户说"还是这个"的形态）。
const tpl = parsed.descriptor.template?.content ?? ''
// ⚠️⚠️ **先剥 HTML 注释再匹配** —— 本仓踩过这个假阳性（`check_module_boundaries.py`
//    的上一版手写 grep 判据因此产出 8 条假阳性，等于没用）。
//    这里尤其要紧：**"我把 `<optgroup>` 删掉了"这句话本身必须写在注释里**
//    （否则下一个人会把它加回来），而只匹配文本的判据会**把说明当成违规**。
//    ⇒ 断言只对**真标记**生效。`(?<!--)` 不做可变长回看，所以直接整体剥掉。
const tplMarkup = tpl.replace(/<!--[\s\S]*?-->/g, '')
// ⚠️ **排版轴是两级**（种类 + 细分）；过渡轴**只有一级**（直接列两档）。
//    过渡轴那条两级结构已于 2026-10-15 删除 —— 症状见上面那段注释。
for (const ax of ['style', 'effect']) {
  ok(new RegExp(`v-for="k in kindsOnAxis\\('${ax}'\\)"`).test(tplMarkup),
    `③ 「${ax}」那条轴没有种类下拉（\`v-for="k in kindsOnAxis('${ax}')"\`）—— 那一栏是空的`)
}
for (const v of ['styleVariants', 'effectVariants']) {
  ok(new RegExp(`v-for="a in ${v}"`).test(tplMarkup), `③ \`${v}\` 那条细分下拉是空的`)
}
// ⚠️⚠️ **过渡轴：一个框、两个档**（2026-10-15）。三条一起断言才拦得住"又拆回两级"：
//    ① 不许再有 `kindsOnAxis('transition')` 的种类框；
//    ② 不许再有 `transitionVariants` 的细分框；
//    ③ 必须有一个直接列出两档的下拉（`TRANSITION_OPTIONS`）。
{
  ok(!new RegExp(`v-for="k in kindsOnAxis\\('transition'\\)"`).test(tplMarkup),
    '③ 过渡轴**又出现了种类下拉**（`kindsOnAxis(\'transition\')`）—— 面板上会变成' +
    '「过渡 / 弹出」两个框（用户报过不止一次）')
  ok(!/v-for="a in transitionVariants"/.test(tplMarkup),
    '③ 过渡轴**又出现了细分下拉**（`transitionVariants`）—— 同上，两级只属于排版轴')
  ok(/v-for="a in TRANSITION_OPTIONS"/.test(tplMarkup),
    '③ 过渡轴没有那个"直接列两档"的下拉（`v-for="a in TRANSITION_OPTIONS"`）—— 那一栏是空的')
}
ok(/ANIM_KNOWN\.includes\(/.test(tplMarkup), '③ 模板没有用 `ANIM_KNOWN` 判断未知档')
// ⚠️ 三栏**各**要有一条"认不出档名就落到未知支"的判据 —— 少一栏，那一栏的
//    错档名就会被静默显示成"默认"（用户以为没选错）。
for (const ax of AXES) {
  ok(new RegExp(`${ax}Kind === ''`).test(tplMarkup),
    `③ 「${ax}」栏：认不出的档名没有落到"未知"那一支（\`${ax}Kind === ''\`）`)
}
// ⚠️⚠️ **三轴必须绑到三个不同的字段** —— 绑同一个就是"互斥"（回退掉的那版错法）。
//    `transition_anim` 是这一版**新独立**出来的一轴。
for (const f of ['style_anim', 'transition_anim', 'effect_anim']) {
  ok(new RegExp(`selItem\\.${f}`).test(tplMarkup),
    `③ 模板没有绑到 \`selItem.${f}\` —— 那条轴没有自己的字段`)
}
// ⚠️ **`AXIS_FIELDS` 那张映射表也要真的存在**（两个 setter 都经它取字段名）——
//    它是"三轴各写各的字段"的唯一一份表，删了等于三轴回到共用一个字段。
ok(/const AXIS_FIELDS = \{[^}]*style:[^}]*transition:[^}]*effect:/.test(src),
  '③ 源码里没有 `AXIS_FIELDS` 三轴映射表（或它不再同时含 style/transition/effect）')
// ⚠️ **只对真标记断言**（`tplMarkup`）—— 见上面"先剥注释"那段。
ok(!/<optgroup/.test(tplMarkup),
  '③ 又出现了 `<optgroup>` —— 那是"四十项平铺进一个下拉"的旧形态，用户报过"还是这个"')
// ⚠️ 值匹配不上任何一项时必须有兜底 option（否则 `<select>` 会显示第一项 ——
//    用户以为自己没选错）。**三栏各一条**。
//
// ⚠️⚠️ 判据走 `tplMarkup`（**已剥 HTML 注释**），但那只剥掉模板注释 ——
//    `<script setup>` 里的 `//` 注释**剥不掉**，于是"注释里提到这个词"
//    也会被数进来。2026-10-15 实测踩到：我在 `TRANSITION_OPTIONS` 的文档注释里
//    写了一次「未知档」，计数就从 3 变成 4，判据**照样绿** —— 而过渡轴那一栏
//    其实已经不再用这个文案了。**"判据因为注释而满足"等于判据没生效。**
//    ⇒ 这里只数**真的 `<option ...>值里有它`** 的那些行。
{
  const optionLines = (tplMarkup.match(/<option[^>]*>[\s\S]*?(?=<\/option>)/g) ?? [])
  const unknownCount = optionLines.filter((l) => /未知档/.test(l)).length
  ok(unknownCount >= 2,
    `③ 带"未知档"的兜底 option 只有 ${unknownCount} 条 —— 排版/动效两栏各要一条` +
    '（否则某栏的值匹配不上时会静默显示成第一项）')
  // ⚠️ 过渡轴那一条兜底**不许**写「未知档」：空串是**正常状态**（没设过），
  //    不是坏数据。写成"未知档"会让用户以为工程坏了。
  ok(!/未知档（\{\{ selItem\.transition_anim/.test(tplMarkup),
    '③ 过渡轴的兜底 option 又写成"未知档"了 —— 空值应当显示成"没设过"，不是坏数据')
}

// ---- ④ ⚠️ **面板上只许有"排版 / 过渡 / 动效"这三栏**（用户口径 2026-10-14）----
//
// ⚠️⚠️ 这一段的由来：用户**两次**指出面板上还挂着**早已被否决的那套四档样式**
//    （hook/chapter/comment/count，`selItem.style`）。
//    第二次他说得很直白：「都说过几次这个样式是被抛弃的，
//    用的是当前这个样式(形态)+动效的双槽位，你还要留个之前否定的方案在上面。」
//    ⇒ 删掉它，并且**把"不许加回来"变成机器判据** —— 光删一次挡不住下一个人。
//
// ⚠️ 为什么必须断言"**没有**"而不只是"有三栏"：
//    第一版就是**只断言了有**，所以旧的 `样式` 下拉一直躺在上面、判据全绿。
{
  // ⚠️ **标签必须正好是这三个词** —— 用户口径：排版就叫「排版」、过渡就叫「过渡」、
  //    动效就叫「动效」（不要「样式（形态）」「动效（随时间）」那种括号注解）。
  const labels = [...tplMarkup.matchAll(/<label>([^<]*)<\/label>/g)].map((m) => m[1])
  for (const want of ['排版', '过渡', '动效']) {
    ok(labels.includes(want), `④ 面板上没有「${want}」这个标签`)
  }
  // ⚠️ 收敛**前**那个「样式」栏的标签**不许**回来。`axis:'style'` 是**内部名**，
  //    用户看到的那一栏叫「排版」—— 两个名字同时出现在界面上正是"到底哪个是哪个"
  //    这类困惑的来源。
  ok(!labels.includes('样式'),
    '④ 面板上又出现了「样式」这个标签 —— 三轴收敛后那条轴（内部名 `axis:\'style\'`）' +
    '显示成「排版」')
  // ⚠️ 带括号注解的旧写法不许回来。
  for (const bad of ['样式（形态）', '动效（随时间）']) {
    ok(!tplMarkup.includes(bad),
      `④ 又出现了「${bad}」—— 用户要求标签就是「排版」/「过渡」/「动效」，不带括号注解`)
  }
  // ⚠️⚠️ **被否决的四档样式下拉不许回来**：它绑的是 `selItem.style`。
  //    判据盯**绑定**（`selItem.style` 的赋值），不是盯文案 —— 改个标签就绕过去了。
  ok(!/selItem\.style\s*=/.test(tplMarkup) && !/TEXT_STYLES/.test(tplMarkup),
    '④ 被否决的四档样式下拉**又回到面板上了**（`selItem.style` / `TEXT_STYLES`）\n' +
    '     ⚠️ 用户口径：那套四档早已被否决，现在只有「排版 × 过渡 × 动效」这三栏。')
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
  // ⚠️ **排版角度滑杆的接线**（2026-10-14 新增）：**只在 `tilt` / `arc` 显示**。
  //    区间与转译器（权威）同值由 `scripts/check-anim-layout-angle.mjs` 逐值钉着，
  //    这里只管"模板真的把它接上了、而且真的门控了"。
  ok(/LAYOUT_ANGLE_MIN/.test(tplMarkup) && /LAYOUT_ANGLE_MAX/.test(tplMarkup),
    '④ 排版角度滑杆没有用 `LAYOUT_ANGLE_MIN/MAX` —— 区间写死了就会与转译器（权威）漂开')
  ok(/v-if="showAngle"/.test(tplMarkup),
    '④ 角度滑杆没有用 `showAngle` 门控 —— 竖排 / 默认那两档也会显示它，' +
    '用户会以为调得动（调了不生效，而界面看着是生效的）')
}

// ---- ⑤ ⚠️⚠️ **三轴**：哪一栏有哪些档 + 三栏的"空"显示名各不相同 ----
//
// # 这一段的由来（上一版），**理由没变、对象变了**
//
// 用户报「当前前端预览的设置内，**样式和动效内容过于重复**？是不是之前去重后
// 有些内容没有清理」。实测查清了两件事：
//  ① 上一版 **39 档零重复归属、面板一个档都没丢** ⇒ 用户的猜测（去重残留）**不成立**；
//  ② 但归属表里有 **11 档 `axis:'both'`** —— 它们**真的两条轴都给参数**，
//     所以在两栏里都列，而两栏显示**同一个名字**，读起来像"重复"。
//     ⇒（上一版的做法）给它们加轴后缀。
//
// ⚠️⚠️ **2026-10-14 收敛后这条判据换了对象**：
//   * `axis:'both'` 与 `AXIS_SUFFIX` 都已删除 ⇒ "两栏重名"在**结构上不可能**；
//   * 但**新的**同名风险落在"空"那一项上：三栏**都要有**这个选项（少一栏用户就
//     清不掉 / 回不到默认），而它**三栏的语义不同**（状态 / 状态 / 选择）⇒
//     显示名**必须各不相同**。
//     ⚠️ 症状是具体的：三栏都写同一个词时，用户在**动效栏**读到一个泛泛的
//     「默认」，会以为**根本没有动效这回事**（它看着像一个"无需设置"的说明，
//     而不是一个"我不加动效"的选择）—— 这正是这一条要防的。
{
  // ⚠️⚠️ **必须跑源码里那个真的 `kindsOnAxis` / `kindLabelOn`，不许在判据里复刻一份**。
  //
  //    第一版就是复刻的（`const shown = (axis,k) => ...`）—— 实测**两处变异都存活**：
  //    改源码的 `kindLabelOn`（去掉轴后缀 / 给只属一栏的也加后缀），判据照样全绿，
  //    因为它跑的是**自己那份副本**。这正是本仓反复踩的"判据测到了函数、
  //    没测到实现"那一类 —— 判据绿得毫无意义。
  //
  //    ⇒ 把真定义**逐字从源码里抠出来、用 `new Function` 真跑**，
  //      这样改源码就一定被观测到。
  //
  //    ⚠️⚠️ **抠取本身是一条判据**：抠不到就是**失败**，而且**不留任何
  //      "默认实现"**（上一版留了一句 `shown = () => { throw ... }` 当兜底 ——
  //      那是一颗静默的雷：函数形状一变，它抛的错**不是断言失败**，
  //      而是让整个脚本崩掉、红得看不懂）。
  ok(G.kindsOnAxis.length > 0,
    '⑤ 源码里抠不到 `kindsOnAxis` 的函数体 —— 过滤那段的正则跟不上新形状了')
  ok(G.kindLabelOn.length > 0,
    '⑤ 源码里抠不到 `kindLabelOn` 的函数体 —— 抠取正则跟不上新形状了')

  if (G.kindsOnAxis && G.kindLabelOn && KINDS.length) {
    // ⚠️ 只喂源码里那张真表与那两个真函数，不引入任何应用模块（零依赖口径）。
    //    `realKinds()` 已经把 `ANIM_KINDS` **执行**过了 ⇒ 这里是原样回灌。
    const mk = (ret) => new Function(
      `const ANIM_KINDS = ${JSON.stringify(KINDS)}\n${G.kindsOnAxis}\n${G.kindLabelOn}\nreturn ${ret}`,
    )()
    const kindsOnAxis = mk('kindsOnAxis')
    const shown = mk('kindLabelOn')
    ok(typeof kindsOnAxis === 'function' && typeof shown === 'function',
      '⑤ 抠出来的 `kindsOnAxis` / `kindLabelOn` 不是函数 —— 判据会空转')

    // ⑤a **每栏的档集合**：真跑出来的 kind 列表必须**正好**是该轴的档 + `none`。
    //     ⚠️ 这就是"三栏各有哪些档"的判据本体：多一个（把别的轴的档漏进来）
    //        或少一个（那一档用户选不到）都当场红。
    for (const ax of AXES) {
      const want = [...KINDS.filter((k) => k.axis === ax).map((k) => k.kind), 'none'].sort()
      let got = []
      try {
        got = kindsOnAxis(ax).map((k) => k.kind)
      } catch (e) {
        ok(false, `⑤ \`kindsOnAxis('${ax}')\` 跑不起来：${String(e.message).slice(0, 120)}`)
        continue
      }
      ok(JSON.stringify([...got].sort()) === JSON.stringify(want),
        `⑤ 「${ax}」栏的档集合不对：真跑得 ${JSON.stringify(got)}，` +
        `应当是 ${JSON.stringify(want)}（该轴的档 + 末尾的"空"）`)
    }
    // ⑤b **"空"那一项三栏都进、且排最后**。
    //     ⚠️ 只有"进三栏"不够：排第一会被当成一个正常的档位读（用户就是这么读的）。
    for (const ax of AXES) {
      const list = kindsOnAxis(ax).map((k) => k.kind)
      ok(list.includes('none'),
        `⑤ 「${ax}」栏少了"空"那一项 —— 用户选错了就再也清不掉 / 回不到默认（只能手改 toml）`)
      ok(list[list.length - 1] === 'none',
        `⑤ 「${ax}」栏的"空"必须排在**最后**，实得第 ${list.indexOf('none') + 1}/${list.length} 位：` +
        JSON.stringify(list))
    }
    // ⑤c **只属一栏的档：显示成自己的 `label`**（不加任何后缀）。
    //     ⚠️ 这是"后缀已删"的**正向**表述 —— ⑥ 反向断言 `AXIS_SUFFIX` 不存在。
    //        两条一起才有说服力：「算出来的标签就是 label」∧「没有打后缀的机器」。
    for (const k of KINDS.filter((x) => x.axis !== 'none')) {
      ok(shown(k.axis, k) === k.label,
        `⑤ 档「${k.label}」在「${k.axis}」栏被显示成「${shown(k.axis, k)}」—— 它只属这一栏，` +
        '不会歧义，加后缀是噪音（那是 `both` 时代的补丁，已随 `AXIS_SUFFIX` 一起删）')
    }
    // ⑤d ⚠️⚠️ **新增（2026-10-14）：三栏的"空"显示名必须两两不同，且各是那个词**。
    //
    //    为什么要单独一条：`none` 那一项**三栏都出现**，而它三栏的**语义不同** ——
    //      排版栏 =「默认」（回到没有任何排版变换 —— 它是一个**状态**）
    //      过渡栏 =「不设」（过渡轴于 2026-10-15 收敛成单框，「默认弹出」那个叫法已删 ——
    //                "弹出为默认"改由 `defaultEvent` 落盘 `transition_anim='pop'` 表达）
    //      动效栏 =「不设」（不额外加动效 —— 一个有意为之的**选择**）
    //
    //    ⚠️⚠️ **过渡栏与动效栏现在同名（都是「不设」）**，所以下面那条"三栏不许重名"
    //       的断言**必须相应放宽成"排版栏那一档必须独一份"**（见 ⑤d-2）——
    //       两栏同名在旧口径下是 bug（那时过渡栏的"空"是另一个词），
    //       而现在它们是**同一种语义**："我不额外指定这一栏的效果"。
    //
    //    ⚠️ 三栏都写同一个词的症状：用户在动效栏读到「默认」，会把整栏读成
    //       "这里没什么可设的"（**以为没有动效这回事**）；在排版栏读到「不设」
    //       又会以为"有一个排版叫不作排版"。**不报错、构建也过**，
    //       只有真看界面才知道 ⇒ 必须机器钉。
    {
      const noneItem = KINDS.find((k) => k.kind === 'none')
      ok(!!noneItem, '⑤ `ANIM_KINDS` 里找不到 `kind: \'none\'` 那一项（判据的前提没了）')
      if (noneItem) {
        // ⚠️⚠️ **2026-10-15：过渡栏的"空"= 动效栏的"空"=「不设」**（用户把过渡轴收敛成
        //    单框之后，「默认弹出」这个叫法失去出口）。所以再要求"三栏两两不同"就是
        //    在要求一个**假**的区别 —— 改成：**排版栏那一档独一份**，其余两栏同名且为「不设」。
        const texts = AXES.map((ax) => shown(ax, noneItem))
        const byAx = Object.fromEntries(AXES.map((ax, i) => [ax, texts[i]]))
        ok(byAx.style !== byAx.transition && byAx.style !== byAx.effect,
          '⑤ 「排版」栏的"空"（默认）**与另外两栏重名了**：' +
          AXES.map((ax, i) => `${ax}="${texts[i]}"`).join(' / ') +
          ' —— "回到出厂排版"与"不额外加效果"是两回事，混用会让用户读错')
        ok(byAx.transition === byAx.effect,
          '⑤ 过渡栏与动效栏的"空"应当同名（都叫「不设」），实得 ' +
          `transition="${byAx.transition}" / effect="${byAx.effect}" —— ` +
          '过渡轴已收敛成单框（2026-10-15），它俩现在是同一种语义')
        // ⚠️ 逐个钉**具体词**："两栏都改成空"也满足上面的判据，
        //    但那样两栏都成了没话说的占位词，用户照样读不出差别。
        const want = { style: '默认', transition: '不设', effect: '不设' }
        for (const ax of AXES) {
          const got = shown(ax, noneItem)
          ok(got === want[ax],
            `⑤ 「${ax}」栏的"空"应当显示成「${want[ax]}」，实得 ${JSON.stringify(got)}`)
        }
        // ⚠️ 三个显示名**不许**落在"没有动效/排版这回事"那种词上 ——
        //    上一版就是把「不动效」写在样式栏里，用户当场指出它
        //    "像被否决的那套方案又回来了"。
        ok(!texts.some((t) => t.includes('不动效')),
          '⑤ 又出现了「不动效」—— 它现在叫「不设」，而且只在动效栏')
      }
    }
    // ⑤e **三栏的显示标签集合两两不相交**（这就是"看起来重复"的可检形式）。
    //
    //     ⚠️ 收敛后这**不可能**因为"两栏都有的档"而违反（没有 `both` 了）——
    //        能违反的只剩**"空"那一项重名**与"两条轴上有两档真重名"。
    //
    //    ⚠️⚠️ **2026-10-15："空"那一项要从这条里豁免**。
    //       过渡轴收敛成单框之后，「不设」在**过渡栏与动效栏**是有意同名的
    //       （同一种语义："我不额外指定这一栏"）⇒ 它**该重名**，
    //       详情与"该不该"的判据都在 ⑤d（那条现在钉的是"排版栏独一份"）。
    //       这里只拦**真的档位**重名 —— 那仍是数据事故。
    {
      const sets = AXES.map((ax) => ({
        ax,
        set: new Set(kindsOnAxis(ax)
          .filter((k) => k.kind !== 'none')   // "空"的显示名归 ⑤d 管
          .map((k) => shown(ax, k))),
      }))
      for (let i = 0; i < sets.length; i++) {
        for (let j = i + 1; j < sets.length; j++) {
          const overlap = [...sets[i].set].filter((l) => sets[j].set.has(l))
          ok(overlap.length === 0,
            `⑤ 「${sets[i].ax}」栏与「${sets[j].ax}」栏有同名档：${overlap.join('、')} —— ` +
            '用户读到的就是"这两栏重复了"')
        }
      }
    }
    // ⑤f **源码侧的接线判据**：模板必须真的把 axis 传进去。
    //     只判"算出来的标签不同"是不够的 —— 模板里万一调的是不带 axis 的旧签名，
    //     算得再对也不生效（本仓反复踩过"测到函数、没测到调用方"）。
    //
    //    ⚠️ **只查 `style` / `effect` 两栏**（2026-10-15）：过渡轴已收敛成单框，
    //       它那个 `<select>` 直接列 `TRANSITION_OPTIONS`、**不再调 `kindLabelOn`**
    //       （它的选项标签是那份常量自带的 `label`）。要求它必须调 = 强迫把
    //       两级结构加回来。
    for (const ax of ['style', 'effect']) {
      ok(new RegExp(`kindLabelOn\\('${ax}', k\\)`).test(tplMarkup),
        `⑤ 模板里「${ax}」那一栏没有把 axis 传给 kindLabelOn（显示会退回不带轴的名字）`)
    }
    // ⚠️ 反向：过渡轴**不许**再用 `kindLabelOn` 出选项（用回来 = 两级结构回来了）。
    ok(!new RegExp(`kindLabelOn\\('transition', k\\)`).test(tplMarkup),
      '⑤ 过渡轴又用 `kindLabelOn(\'transition\', k)` 出选项了 —— 那是种类下拉的形态，' +
      '它应当在 `TRANSITION_OPTIONS` 上直接 `v-for`（用户报过「过渡 / 弹出」两个框）')
    // ⑤g **"判据自己空转"的兜底**：档数为 0 时上面那些 `for` 一条都不跑。
    ok(KINDS.every((k) => typeof k.label === 'string' && k.label.length > 0),
      '⑤ 有档位没扒到 `label` —— 显示名对拍会拿 `undefined` 去比，看着像"全过"')
  }
}

// ---- ⑥ ⚠️⚠️ **`AXIS_SUFFIX` 必须不再存在**（2026-10-14 三轴收敛）----
//
// # 为什么单独一条、而且是**反向**的
//
// `AXIS_SUFFIX` 是上一版给 `axis:'both'` 的档打的补丁：同一个档名在两栏里显示成
// 「砸入 · 形态」/「砸入 · 动效」，让用户看出是"同一个档的两个半边"。
//
// 收敛后**没有任何档属于两条轴** ⇒ 两条栏不可能出现同一个名字 ⇒ 后缀失去意义。
//
// ⚠️⚠️ **但"没用了"不是删它的主要理由 —— 留着它是**有害**的**：
//    它一旦躺在源码里，下一个人会以为"档可以属于两轴"，于是把 `both` 加回来、
//    把同一档重新列进两栏 —— 而那正是用户报的「样式和动效内容过于重复」的根源。
//    ⇒ 它必须**不存在**，而且是**机器可检的**不存在。
//
// ⚠️ 判据只认**定义/使用**，不认**注释里的那句"已删"** ——
//    源码里那段说明（"⚠️⚠️ `AXIS_SUFFIX` 已删"）本身**必须**留着
//    （否则下一个人会把它加回来），所以这里要先剥注释。
//    这与 `check_module_boundaries.py` 的"先剥注释再匹配"是同一条纪律。
{
  const noComments = src
    .replace(/\/\*[\s\S]*?\*\//g, '')
    // `(?<!:)` 那个写法在 JS 里是**可变长回看**（不支持）⇒ 用捕获组代替，
    // 为的是不把 `https://…` 里的 `//` 当注释起点。
    .replace(/(^|[^:])\/\/[^\n]*/g, '$1')
  ok(!/AXIS_SUFFIX/.test(noComments),
    '⑥ `AXIS_SUFFIX` **又回到源码里了**（非注释处）—— 它是"档可以属于两条轴"那个\n' +
    '     旧模型的补丁。三轴收敛后每条档位只属一条轴；它一旦存在，下一个人就会\n' +
    '     以为"档可以属于两轴" ⇒ 把 `both` 加回来 ⇒ 两栏重名，\n' +
    '     而那正是用户报的「样式和动效内容过于重复」的根源。')
  // ⚠️ **注释里那句"已删"必须留着** —— 这是反向判据的说明书。
  //    没有它，下一个人看到 `AXIS_SUFFIX` 不存在只会以为是漏了，然后加回来。
  ok(/AXIS_SUFFIX/.test(src),
    '⑥ 连注释里那句"`AXIS_SUFFIX` 已删（三轴收敛）"都不见了 —— 反向判据必须留下\n' +
    '     说明书，否则下一个人会把它当成"漏了"再加回来')
}

// ---- ⑦ ⚠️ **两个 setter 只动自己那一轴**（"可叠加"与"互斥"的分界线）----
//
// # 这一条为什么必须**真跑**
//
// 用户口径：「两个设置，有各自的细分，**是可叠加的**」（现在是三轴）。
// 历史（两次都是真事故，症状都不会自己报错）：
//   * **写同一个字段**（三级互斥那一版）⇒ 用户选样式把动效**清掉**；
//   * 修法"顺手写另一轴" ⇒ 也是错的：用户设好的另一轴被"点一下"偷走。
//
// ⚠️⚠️ **不许读源码文本判**：文本判的形态是"函数体里不许出现 `effect` 字样" ——
//    但 `axisField('effect')` 里那个 `'effect'` 是**字符串**，它本来就会出现在
//    **正确**的实现里（`AXIS_FIELDS` 那张表本身就是三个轴名）⇒ 文本判要么假红、
//    要么松到根本抓不住。⇒ 抠出来**执行**，用"整份 selItem 快照"比对。
{
  for (const [key, label] of [
    ['axisFields', '`AXIS_FIELDS` 三轴映射表'],
    ['axisField', '`axisField`'],
    ['kindsOnAxis', '`kindsOnAxis`'],
    ['setKindOnAxis', '`setKindOnAxis` 的函数体'],
    ['setVariantOnAxis', '`setVariantOnAxis` 的函数体'],
  ]) {
    ok(G[key].length > 0, `⑦ 抠不到 ${label}（抠取正则跟不上新形状了）`)
  }
  if (G.axisFields && G.axisField && G.kindsOnAxis && G.setKindOnAxis && G.setVariantOnAxis) {
    // ⚠️ 真跑，不重写：定义区**逐字**搬进去，只有 `selItem` 是个桩（它住在组件里）。
    //    探针函数把 `selItem` 一起暴露出来 —— **不改被测源码**。
    const body = [
      `const ANIM_KINDS = ${JSON.stringify(KINDS)}`,
      G.axisFields,
      G.axisField,
      G.kindsOnAxis,
      'let selItem = { value: {} }',
      G.setKindOnAxis,
      G.setVariantOnAxis,
      'return { kindsOnAxis, setKindOnAxis, setVariantOnAxis, axisField, selItem }',
    ].join('\n')
    let probe = null
    try {
      probe = new Function(body)()
    } catch (e) {
      ok(false, `⑦ 抠出来的两个 setter **跑不起来**：${String(e.message).slice(0, 160)}\n` +
        '     ⚠️ 多半是 `TimelineEditor.vue` 的接线形态变了，本判据的抽取锚点要跟着改。')
    }
    if (probe) {
      // ⚠️ 预置：**另一轴先放一个有值的档**（用户"设好了"的状态）——
      //    只测空对象抓不住"顺手覆盖另一轴"（覆盖成 `null` 也算覆盖）。
      const fresh = () => {
        const p = new Function(body)()
        for (const o of AXES) p.selItem.value[p.axisField(o)] = 'PRESET'
        return p
      }
      // ⑦a 切**种类**：三条轴 × 该轴全部 kind（**真函数**给出的用例集，
      //     不是判据手抄的一份 —— 手抄的会静默漏测新增的档）。
      for (const ax of AXES) {
        const others = AXES.filter((a) => a !== ax)
        let ownKinds = []
        try {
          ownKinds = probe.kindsOnAxis(ax).map((k) => k.kind)
        } catch (e) {
          ok(false, `⑦ \`kindsOnAxis('${ax}')\` 跑不起来：${String(e.message).slice(0, 120)}`)
          continue
        }
        for (const kind of ownKinds) {
          const p = fresh()
          try {
            p.setKindOnAxis(ax, kind)
          } catch (e) {
            ok(false, `⑦ \`setKindOnAxis('${ax}', '${kind}')\` 抛了：${String(e.message).slice(0, 120)}`)
            continue
          }
          for (const o of others) {
            const key = probe.axisField(o)
            ok(p.selItem.value[key] === 'PRESET',
              `⑦ \`setKindOnAxis('${ax}', '${kind}')\` 动了**另一轴**的 \`${key}\` ` +
              `（"PRESET" → ${JSON.stringify(p.selItem.value[key])}）—— ` +
              '用户设好的另一轴被"点一下"偷走。\n' +
              '     ⚠️ 这就是"可叠加"与"互斥"的分界线：三轴各写各的字段。')
          }
          // ⚠️ 也不许写**第三个**字段（例如顺手写了个时间戳 / 缓存）。
          const touched = Object.keys(p.selItem.value)
            .filter((k) => p.selItem.value[k] !== 'PRESET')
          ok(touched.every((k) => k === probe.axisField(ax)),
            `⑦ \`setKindOnAxis('${ax}', '${kind}')\` 写了别的字段：` +
            `${JSON.stringify(touched)} —— 只许写 \`${probe.axisField(ax)}\``)
        }
      }
      // ⑦b 切**细分**：同样只动自己那一轴。
      for (const ax of AXES) {
        const others = AXES.filter((a) => a !== ax)
        const p = fresh()
        p.setVariantOnAxis(ax, 'X')
        for (const o of others) {
          const key = probe.axisField(o)
          ok(p.selItem.value[key] === 'PRESET',
            `⑦ \`setVariantOnAxis('${ax}', 'X')\` 动了**另一轴**的 \`${key}\` ` +
            `（"PRESET" → ${JSON.stringify(p.selItem.value[key])}）`)
        }
        ok(p.selItem.value[probe.axisField(ax)] === 'X',
          `⑦ \`setVariantOnAxis('${ax}', 'X')\` 没写自己那一轴（\`${probe.axisField(ax)}\`）—— ` +
          '用户改细分会看起来没生效')
      }
      // ⑦c **空串必须落成 `null`**（`''` 是一个合法的**坏值**，`null` 才是"键不存在"）。
      //     ⚠️ 与 `check-anim-presets-parity.mjs` ⑥ 同源但**不重复**：那边读文本判
      //        "写了那个 `|| null`"，这里**真跑**判"落盘值真的是 `null`"。
      for (const ax of AXES) {
        const p = fresh()
        p.setKindOnAxis(ax, 'none')
        const key = probe.axisField(ax)
        ok(p.selItem.value[key] === null,
          `⑦ 在「${ax}」栏选"空"那一项应当落成 \`null\`（键不存在），实得 ` +
          `${JSON.stringify(p.selItem.value[key])} —— 空串是一个合法的**坏值**`)
      }
    }
  }
}

// ---- ⑧ ⚠️ **认不出的档名不能被静默归成"无"** ----
//
// 症状（用户可见、且**不报错**）：老工程里带了一个已经删掉的档名
// （比如上一版的 `tilt_std`），面板会把它**显示成"默认"** —— 用户以为"我没设过"，
// 改一下别的字段就把那个错档名**永久抹掉**了。而报告里那行 CLAMP 正告诉他
// 这一条不会动。
// ⇒ 认不出就必须落进"未知档"那一支（③ 已钉模板有那三条 option），
//   而这一支的前提是 `kindOnAxis` 真的返回 `''` 而不是 `'none'`。
{
  ok(G.kindOnAxis.length > 0, '⑧ 抠不到 `kindOnAxis` 的函数体（抠取正则跟不上新形状了）')
  if (G.kindOnAxis && KINDS.length) {
    // ⚠️ 只喂它依赖的那张 `ANIM_KIND_OF` —— 从源码的**真表**算出来（不另抄档名）。
    const fn = new Function(
      `const ANIM_KINDS = ${JSON.stringify(KINDS)}\n` +
      'const ANIM_KIND_OF = new Map(ANIM_KINDS.flatMap((k) => k.variants.map((v) => [v.value, k.kind])))\n' +
      `${G.kindOnAxis}\nreturn kindOnAxis`,
    )()
    // ⚠️ 空值（"没设过"）⇒ `'none'`（正常态，**不是**"未知"）。
    ok(fn('') === 'none' && fn(null) === 'none' && fn(undefined) === 'none',
      `⑧ 空值应当落成 \`'none'\`，实得 ${JSON.stringify([fn(''), fn(null), fn(undefined)])}`)
    // ⚠️ 认得出的档名 ⇒ 它那个 kind（**用真表的档名**，不手抄）。
    for (const name of nonEmpty) {
      const k = fn(name)
      ok(k !== '' && k !== 'none',
        `⑧ 认得出的档名 \`${name}\` 被判成了 ${JSON.stringify(k)} —— 面板会把它显示错`)
    }
    // ⚠️⚠️ **本段的重点**：认不出的档名 ⇒ `''`（走"未知档"那一支），
    //     **不许**被静默归成 `'none'`（那会让用户以为"我没设过"）。
    //     ⚠️ 用例里刻意放**上一版真实存在过的档名**（`tilt_std` / `slam_lg` …）——
    //        老工程里真会躺着的就是这些。
    for (const junk of ['tilt_std', 'slam_lg', 'fade_only', 'arc_up', 'vtext_bold',
                        'shrink_in_std', 'pop_perdoc', '不存在的档名', 'shake ']) {
      ok(fn(junk) === '',
        `⑧ 认不出的档名 \`${junk}\` 被判成了 ${JSON.stringify(fn(junk))} —— ` +
        '**不许静默归成"无"**（否则用户改一下别的字段就把这个错档名永久抹掉了）')
    }
  }
}

// ---- 结果 ----
if (fails.length === 0) {
  console.log(
    `✓ 三轴动效下拉真的接上了（${checks} 项：SFC 编译绑定 + 三轴 ${WANT_KINDS} 种类 / ` +
    `${WANT_TIERS} 档 + 排版栏"空"独一份（过渡/动效同为「不设」）+ 过渡轴单框 + 模板接线 + 真函数行为）`,
  )
  process.exit(0)
}
console.log(`✗ 动效下拉有问题（${fails.length} / ${checks}）：`)
for (const f of fails) console.log('    ' + f)
process.exit(1)
