#!/usr/bin/env node
// ============================================================================
// `check-anim-attribution.mjs` —— **三轴归属表守卫**（10 档：排版 / 过渡 / 动效）
// ============================================================================
//
// 用法：`node scripts/check-anim-attribution.mjs [--mutate]`
//   `--mutate` = 变异自证：把表里的 `shake` 标成排版轴，② 必须当场红（红了才 exit 0）。
//
// # 它存在的理由
//
// 「排版 / 过渡 / 动效」拆成三个类型之后，**每一档归到哪条轴、给了哪些字段**
// 必须有个唯一权威。这个权威就是 `scripts/anim-attribution.mjs`（**实测生成**），
// 而它有四个会腐坏的地方：
//
//   ① 改了 `ANIM_PRESETS` 却没重新生成表（表悄悄过期）；
//   ② 表的**归属**与源码的 `ANIM_AXIS` / `animAxisOf` 不符；
//   ③ 表的**字段那层**与产物不符（档里声明了、产物里没有 ⇒ 那一档静默不生效）；
//   ④ 分类数变了（有档退化成两边都没有，或从"纯排版"变成"也在动"）。
//
// ============================================================================
// ⚠️⚠️ 本判据的权威来源（2026-10-14 换过，**别再换回去**）
// ============================================================================
//
// 上一版（39 档）读的是 **`webui/src/polish/engine.js::ANIM_PRESETS`**。
// 那条路有个**会骗人**的形状，实测撞到过：
//
//   `94a8892` 把**转译器**收敛成 9 档，而 `engine.js` **一个字节没动**（仍是 39 档）
//   ⇒ 上一版判据拿**旧表对旧表**、自己跟自己一致 ⇒ **全绿**；
//   而生产上"用户选 9 个新档，预览里一个都不动"。
//
//   ⇒ **判据不能拿一份可能过期的副本当权威。** 权威是**转译器**
//     （`tools/polish-to-dhampir.mjs`，真正写进 toml、决定成片的那一份），
//     而"三份实现是否同源"由 `check-anim-presets-parity.mjs` 负责，
//     不是本判据的活。
//
// ⚠️ **`animAxisOf` 从源码抽出来真跑**，不在本判据里重写一份 ——
//    重写的那份会与被测的那份**一起漂**（本仓实测教训：
//    "判据重实现了被测函数 ⇒ 两条源码变异都活下来、判据假绿"）。
import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import {
  buildAttribution, measure, loadAuthority, AXES,
  FORM_KEYS, MOTION_KEYS, KEYFRAMES_MARK, TRANSFORM_MARK,
} from './gen-anim-attribution.mjs'

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const TABLE = path.join(ROOT, 'scripts', 'anim-attribution.mjs')

const MUTATE = process.argv.includes('--mutate')

/// 预设键 → **产物键**的改名表（本仓"档位表这一层统一驼峰、写进底座转蛇形"那条纪律）。
///
/// ⚠️⚠️ **这里只登记真的会改名的**，别把整张预设表抄一遍 ——
///    抄一遍就变成"第二份会漂的实现"，而这张表**正是为了对拍**才存在的。
///    实测漏掉 `fontWeight` → `font_weight` 时，判据把**完全正常**的 `vtext` 档
///    报成"静默不生效"（假红），假红与假绿一样会让人把判据关掉。
const PRESET_TO_OUTPUT = {
  perChar: 'per_char',
  fontWeight: 'font_weight',
}

/// **控制参数**：预设里声明了，但**本来就不该**写进产物（它们决定别的字段的值）。
///
/// ⚠️ 别把它们删掉求绿 —— 它们是"这一档到底靠什么生效"的一部分，
///    删了就失去了"这两档的幅度从哪来"的可见性。
const CONTROL_ONLY = new Set(['arcToDefault'])

/// ⚠️ 本判据**逐条实名**：`ok(cond, msg)` 每条都记数，最后打印项数。
const fails = []
let checks = 0
// ⚠️⚠️ **预检：现场重测能不能跑起来**（2026-10-30）。
//
// `buildAttribution()` 会真跑转译器、按轨 id 找每条档位的轨。**生成器的轨 id 锚点过期**时
// （2026-10-15 起 id 含落点 `_x..y..`、2026-10-30 起含颜色 `_c..`），
// 它会抛「轨没生成」—— 而那个堆栈看起来像产品坏了，实际是**判据夹具烂了**。
// ⇒ 这里先跑一次、失败就**只报这一条并指路**（别让 100 项断言去猜）。
try {
  await buildAttribution()
} catch (err) {
  console.log('✗ 归属表**现场重测跑不起来**（测量台前置失效，不是产品代码的问题）：')
  console.log('    ' + String(err.message).split('\n')[0])
  console.log('  ⇒ 多半是 `scripts/gen-anim-attribution.mjs::measure` 按旧轨 id 找轨：')
  console.log('     它找 `text_comment_bottom_<档名>`，而现网 id 是')
  console.log('     `text_comment_bottom_x<..>y<..>_<档名>_c<..>`（落点 2026-10-15、颜色 2026-10-30 进 id）。')
  console.log('     ⇒ 先修 measure 的轨 id 解析（按 animTag 后缀匹配），再 `node scripts/gen-anim-attribution.mjs`。')
  process.exit(1)
}
const ok = (cond, msg) => { checks++; if (!cond) fails.push(msg) }

// ---- 变异：把表的某个档改到**另一条轴**上（判据必须当场红）----
//
// ⚠️⚠️ **改副本，绝不碰生产文件**（本仓 `check-preview-anim.mjs` 用 `git checkout`
//    还原变异时把**未提交的实现整份冲掉**过一次）。这里只写 `__mutant_attrib.mjs`。
let mutated = false
let importUrl = TABLE
if (MUTATE) {
  const orig = fs.readFileSync(TABLE, 'utf8')
  // 把 `shake` 从动效挪到排版 —— 一个**看起来合理**的错法（"抖动"确实也算排版？）
  const patched = orig.replace("'shake': { axis: 'effect'", "'shake': { axis: 'style'")
  if (patched === orig) {
    console.log('✗ 变异失败：表里找不到 `shake` 的 effect 标记（判据的前提没了）')
    process.exit(1)
  }
  const copy = path.join(ROOT, 'scripts', '__mutant_attrib.mjs')
  fs.writeFileSync(copy, patched)
  mutated = true
  importUrl = copy
  console.log('【变异】把 `shake` 标成排版轴（只动物副本）')
}

try {
  const T = await import('file://' + importUrl.replace(/\\/g, '/'))
  const ATTR = T.ANIM_ATTRIBUTION
  // ⚠️ `live` = **现场重测**（真跑转译器）⇒ 表过期 / 字段那层漂了都会被抓到。
  const live = await buildAttribution()
  const { bundle } = loadAuthority()
  const { ANIM_PRESETS, animAxisOf } = bundle
  const names = Object.keys(ANIM_PRESETS)
  const tnames = Object.keys(ATTR).sort()

  // ---- ① 覆盖：表与预设表**同集合**（数目 + 逐档两个方向）----
  ok(tnames.length === names.length,
    `① 表里 ${tnames.length} 档，预设表 ${names.length} 档 —— 数目不等`)
  const miss = names.filter((n) => !(n in ATTR)).sort()
  const extra = tnames.filter((n) => !(n in ANIM_PRESETS)).sort()
  ok(miss.length === 0, `① 这些档表里没有：${JSON.stringify(miss)}`)
  ok(extra.length === 0, `① 表里这些档预设表没有：${JSON.stringify(extra)}`)

  // ---- ②⚠️ 归属：表 ↔ `animAxisOf` ↔ 预设表 `sort`（三处同源）----
  //
  // ⚠️⚠️ **`animAxisOf` 必须逐个档真跑** —— 这不是重复劳动：
  //    收敛后 `heartbeat` **不带下划线**，`animAxisOf` 若只判前缀就返回 `''`
  //    ⇒ 它会被当成"认不出的档名"走 CLAMP ⇒ **用户选了心跳没反应**。
  //    表说 effect、`animAxisOf` 说 `''` ⇒ 本条当场红。
  for (const n of names) {
    if (!(n in ATTR)) continue
    const viaFn = animAxisOf(n)
    ok(viaFn !== '',
      `② \`${n}\`：\`animAxisOf\` 返回空 —— 这一档**认不出来**` +
      `（症状：选了它成片里没反应，报告说"档名不认识"）`)
    ok(ATTR[n].axis === viaFn,
      `② \`${n}\` 表说 \`${ATTR[n].axis}\`，\`animAxisOf\` 说 \`${viaFn}\` —— 表与源码不同源`)
    // ⚠️ `axis` 必须是三条轴之一（拼错轴名的静默后果：那条轴整栏空掉）。
    ok(AXES.includes(ATTR[n].axis),
      `② \`${n}\` 的 \`axis\` 是 \`${ATTR[n].axis}\`，不在 ${JSON.stringify(AXES)}`
      + ` —— 拼错轴名会让那一栏整批消失`)
  }

  // ---- ③⚠️ 表必须与**现场重测**一致（防"表过期"）----
  for (const n of names) {
    if (!(n in ATTR)) continue
    const a = ATTR[n], b = live[n]
    ok(JSON.stringify(a.form) === JSON.stringify(b.form),
      `③ \`${n}\` 的形态字段不一致：表 ${JSON.stringify(a.form)} vs 现场 ${JSON.stringify(b.form)}`
      + ` —— 前者是手工改过的 / 表过期了`)
    ok(JSON.stringify(a.motion) === JSON.stringify(b.motion),
      `③ \`${n}\` 的动效字段不一致：表 ${JSON.stringify(a.motion)} vs 现场 ${JSON.stringify(b.motion)}`)
  }

  // ---- ④⚠️ 核心：**声明了就必须写出去**（"配了、不报错、没效果"那一类）----
  //
  // ⚠️⚠️ 这一条是**本文件存在的第二理由**，而且是实测踩出来的：
  //     `perchar_big` / `perchar_small` 曾经**整档不生效** ——
  //     `motionFields` 的逐字那段只读 `stylePreset`，而这两档的轴是 `effect`
  //     ⇒ 轨生成了、`per_char` **一个字段都不写**、**报告里一行差异都没有**。
  //     症状正是本仓最恨的形状：用户选了「逐字变大」，成片里字一动不动。
  //
  //     ⇒ 判据是"**预设里声明的字段，产物里必须真的出现**"。
  //       （`buildAttribution` 内部已经会为此抛错；这里再独立断言一次，
  //        是为了让"表"与"产物"两边都被看见，而不是只信一个来源。）
  const { measured } = await measure()
  for (const n of names) {
    const declared = Object.keys(ANIM_PRESETS[n] ?? {}).filter((k) => k !== 'sort')
    const emitted = new Set(measured[n].emitted)
    for (const k of declared) {
      // ⚠️ **控制参数**（不是"写出去的字段"）——它们只决定**别的字段的值**：
      //    · `arcToDefault` = `arc` 档的缺省幅度，用户填角度时覆盖 `per_char.rotation_to`；
      //    · `heartbeatPeaksMs` / `heartbeatAmp` / `heartbeatCycleMs` = 心跳的**节奏参数**，
      //      底座尚无对应原语（转译器里如实标成 NOTE，见 `heartbeatNotImplemented`）。
      //    ⇒ 它们**本来就不该出现在产物里**，别把它们判成"静默不生效"。
      if (CONTROL_ONLY.has(k) || /^heartbeat/.test(k)) continue
      // 预设键 → 产物键（**只有两处改名**，两个方向的"锚点"都写在这里）。
      const out = PRESET_TO_OUTPUT[k] ?? k
      // ⚠️ 走**其他腿**的键不在这里核（它们本来就不写 `subtitle`）：
      //    · `tiltDeg` / `squashY` → 图层 `transform`（由 `buildAttribution` 按值核）。
      if (k === 'tiltDeg' || k === 'squashY') continue
      ok(emitted.has(out),
        `④ \`${n}\` 预设里声明了 \`${k}\`，但产物 \`subtitle\` 里**没有** \`${out}\` —— ` +
        `这一档**静默不生效**（轨会生成、报告里一行差异都没有）`)
    }
  }

  // ---- ⑤ 三轴的**档集合两两不相交**（"同一档名只归一条轴"）----
  //
  // ⚠️ 这是"归属"这件事的**根本要求**：同一档名若出现在两条轴，
  //    谁都以为自己该管它 ⇒ 用户选一次、两条轴各执行一次（或互相打架）。
  const sets = {}
  for (const ax of AXES) sets[ax] = names.filter((n) => live[n].axis === ax).sort()
  for (let i = 0; i < AXES.length; i++) {
    for (let j = i + 1; j < AXES.length; j++) {
      const [a, b] = [AXES[i], AXES[j]]
      const inter = sets[a].filter((n) => sets[b].includes(n))
      ok(inter.length === 0,
        `⑤ \`${a}\` 与 \`${b}\` 两条轴的档名**有交集**：${JSON.stringify(inter)} —— ` +
        `同一档名只能归一条轴，否则谁能管它就成了隐性契约`)
    }
  }
  for (const ax of AXES) {
    ok(sets[ax].length > 0, `⑤ \`${ax}\` 轴上**一档都没有** —— 那一栏会是空的`)
  }

  // ---- ⑥ 分类数：少了说明有档退化 ----
  const tally = {}
  for (const n of names) tally[live[n].axis] = (tally[live[n].axis] || 0) + 1
  // ⚠️ **逐轴点名**（不是只看总数）—— 总数对得上、而档跑错了轴，是这条要防的。
  // ⚠️ **2026-10-18 口径**：逐字三档（`perchar` + 两个老别名）从动效轴挪到**过渡**轴
  //    （用户口径「将逐字变化归类为过渡」）⇒ 过渡 5 档 / 动效 2 档。
  //    旧值 `{transition:2, effect:4}` 是 9 档两轴时代的，表一重新生成它就会红。
  const EXPECT = { style: 3, transition: 5, effect: 2 }
  for (const ax of AXES) {
    ok(tally[ax] === EXPECT[ax],
      `⑥ \`${ax}\` 轴应当 ${EXPECT[ax]} 档，实得 ${tally[ax] ?? 0}` +
      `（表：${JSON.stringify(sets[ax])}）—— 档位表变了就要重新理解这条期望`)
  }
  // ⚠️ **`fade` 必须在、且它"一个字段都不给"** —— 它是"压掉全局 `[elements]`"
  //    这条语义的**唯一**入口（空预设）。少了它，"只淡入淡出"就没法表达。
  ok(live.fade && live.fade.form.length === 0 && live.fade.motion.length === 0,
    `⑥ \`fade\` 应当是一个字段都不给的**空预设**（靠"压掉全局"表达"只淡入淡出"），` +
    `实得 form=${JSON.stringify(live.fade?.form)} motion=${JSON.stringify(live.fade?.motion)}`)

  // ---- ⑦ "给了字段"的档必须真的分到两路里（字段那层不许退化成全空）----
  //
  // ⚠️ 防的是"表看起来全对、只是 form/motion 两列**整列是空的**"——
  //    那正是我第一版用键集差分时的形态（差分把同名键差掉了 ⇒ 9 档全空）。
  const withFields = names.filter(
    (n) => live[n].form.length > 0 || live[n].motion.length > 0)
  ok(withFields.length >= 5,
    `⑦ 10 档里只有 ${withFields.length} 档"给了字段" —— 太少，` +
    `多半是字段那层退化了（\`form\`/\`motion\` 两列整列空掉的样子与"判据全绿"一模一样）`)
  // ⚠️ 三条腿**都要有档**在用（否则某一腿整条没人走 ⇒ 判据对它是空的）。
  ok(names.some((n) => live[n].form.includes(TRANSFORM_MARK)),
    `⑦ 没有任何档走"图层 \`transform\`"那条腿 —— \`tilt\`（排版轴唯一的形态量）会整条看不见`)
  ok(names.some((n) => live[n].form.some((k) => FORM_KEYS.has(k))),
    `⑦ 没有任何档给"subtitle 上的形态键"（\`per_char\` / \`vertical\`）`)
  ok(names.some((n) => live[n].motion.some((k) => MOTION_KEYS.has(k))),
    `⑦ 没有任何档给"subtitle 上的动效键"（\`shake_*\` / \`scale_in_*\`）`)

  // ---- ⑧ 模块导出的两个谓词必须与表一致 ----
  for (const n of names) {
    ok(T.hasFormPart(n) === (ATTR[n].form.length > 0),
      `⑧ hasFormPart('${n}') 与表不一致`)
    ok(T.hasMotionPart(n) === (ATTR[n].motion.length > 0),
      `⑧ hasMotionPart('${n}') 与表不一致`)
    ok(T.axisOf(n) === ATTR[n].axis, `⑧ axisOf('${n}') 与表不一致`)
  }
  // ⚠️ 认不出的档名必须返回 `''` / `false`（不是抛错、不是 `undefined`）。
  for (const bad of ['', 'slam_lg', 'nope']) {
    ok(T.axisOf(bad) === '', `⑧ axisOf(${JSON.stringify(bad)}) 应当返回空串（旧档名不许认）`)
    ok(T.hasFormPart(bad) === false, `⑧ hasFormPart(${JSON.stringify(bad)}) 应当 false`)
    ok(T.hasMotionPart(bad) === false, `⑧ hasMotionPart(${JSON.stringify(bad)}) 应当 false`)
  }

  // ---- ⑨ 旧档名**一个都不许在表里**（不做别名迁移：用户明确"没有存量工程"）----
  const OLD = ['tilt_std', 'tilt_sm', 'tilt_rev', 'slam_lg', 'slam_std', 'slam_squash',
    'sway_std', 'sway_lg', 'sway_slow', 'pop_std', 'pop_soft', 'pop_perchar',
    'fade_only', 'arc_up', 'arc_down', 'arc_wave', 'vtext_thin', 'vtext_bold',
    'vtext_shrink', 'shrink_in_std', 'shrink_in_fast', 'scatter_sm', 'scatter_lg',
    'scatter_shake', 'hop_std', 'hop_fast', 'hop_big', 'grow_asc', 'grow_burst',
    'grow_first', 'size_last', 'size_first', 'size_mid', 'shrink_desc',
    'shrink_drop', 'shrink_tail', 'shake_std', 'shake_slow', 'shake_lg']
  const stale = OLD.filter((n) => n in ATTR)
  ok(stale.length === 0,
    `⑨ 表里还有已被删除的旧档名：${JSON.stringify(stale)} —— ` +
    `三轴收敛后旧档名**不存在**，不做别名迁移（用户：「当前没有存量工程」）`)
  // ⚠️⚠️ **但 `animAxisOf` 认得出旧档名是"对的"，别把它判成错的。**
  //
  //    `animAxisOf` 的前缀那条路**就是为迁移留的**（`tilt_std` → 前缀 `tilt` → `style`），
  //    转译器靠它把老工程的 `anim` 拆到两路。判据要钉的是：
  //      · **预设表里不许有**旧档名（`animPreset('tilt_std')` 必须是 `undefined`）——
  //        那才是"旧档位真的删了"；
  //      · 而"轴能认出来"只说明它是**哪个族的旧名字**，不等于它还是个合法档位。
  const stillInTable = OLD.filter((n) => n in ANIM_PRESETS || bundle.animPreset(n) !== undefined)
  ok(stillInTable.length === 0,
    `⑨ 预设表里仍然有这些已被删除的旧档名：${JSON.stringify(stillInTable)} —— ` +
    `三轴收敛后它们**不存在的**，` + `不做别名迁移（用户：「当前没有存量工程」）`)
  // ⚠️ 旧档名的轴**只许**落在它原来那条轴上（前缀路不许把它们指到别的轴去）。
  const misAxis = OLD
    .map((n) => [n, animAxisOf(n)])
    .filter(([, ax]) => ax !== '' && !AXES.includes(ax))
  ok(misAxis.length === 0,
    `⑨ 这些旧档名被指到了不存在的轴：${JSON.stringify(misAxis)}`)
} finally {
  if (mutated) {
    const copy = path.join(ROOT, 'scripts', '__mutant_attrib.mjs')
    try { fs.unlinkSync(copy) } catch { /* 本来就没有就算了 */ }
    if (fs.existsSync(copy)) {
      console.log('✗ 副本没删掉：' + copy)
      process.exit(1)
    }
    console.log('【变异】已删除副本（生产文件从未被改动）')
  }
}

// ⚠️ **`--mutate` 的语义是"自证"**：把表里的 `shake` 标成排版轴后，
//    ②（表 ↔ `animAxisOf`）**必须**当场红。红了才是判据有牙 —— 这里把"红"翻译成 exit 0，
//    否则 CI 跑 `--mutate` 会看到"红 = 失败"而误以为门禁坏了。
//    ⚠️ 如果变异之后**没红**（fails 为空 / 没点到 shake），那是判据空转 ⇒ exit 1。
if (MUTATE) {
  const hit = fails.some((f) => f.includes('shake'))
  if (fails.length > 0 && hit) {
    console.log(`✓ 变异自证：把 \`shake\` 标成排版轴 ⇒ ② 当场红（共 ${checks} 项判据）`)
    process.exit(0)
  }
  console.log('✗ 变异自证失败：改坏归属表之后判据**没在 ② 上翻红** —— 判据有空转的地方')
  if (fails.length) for (const f of fails.slice(0, 10)) console.log('    · ' + f)
  process.exit(1)
}

if (fails.length) {
  console.log(`✗ 归属表 ${fails.length} 处问题（共 ${checks} 项判据）：`)
  for (const f of fails.slice(0, 25)) console.log('    · ' + f)
  if (fails.length > 25) console.log(`    …另有 ${fails.length - 25} 处`)
  process.exit(1)
}
console.log(
  '✓ 归属表守卫：10 档 ── 排版 3 · 过渡 5 · 动效 2' +
  '（表↔`animAxisOf` 一致、表↔**实测产物**一致、声明了就必须写出去、' +
  '三轴档集合两两不相交）；共 ' + checks + ' 项判据')
