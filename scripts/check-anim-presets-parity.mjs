#!/usr/bin/env node
// **三轴 9 档动效：界面清单 ↔ 转译器清单 逐档对拍**（2026-10-14，39 档 → 9 档收敛后重写）。
//
// # 为什么必须有这条判据
//
// 档名现在住在**两个地方**：
//
//   ① `tools/polish-to-dhampir.mjs::ANIM_PRESETS` —— **权威**（底座字段的映射表，
//      每个键带 `sort` 声明它属于哪条轴）；
//   ② `webui/src/components/TimelineEditor.vue::ANIM_KINDS` —— 界面那份（给用户选，
//      每个 kind 带 `axis` 声明它属于哪条轴）。
//
// 两份**必须同集合**。缺一边的症状是本仓最恨的那一类：
//
//   * 界面有、转译器没有 ⇒ 用户选了下拉、**成片里什么都不发生**，
//     报告里只有一行 CLAMP（中文档名，用户很可能没看）；
//   * 转译器有、界面没有 ⇒ 这一档**用户根本选不到**（"9 档"实际只有 N 档），
//     而任何"档数对不对"的人工核对都会数到 9 —— 因为转译器那边确实是 9。
//
// ⚠️ 三轴收敛**不改变**上面这条理由：档名换了、轴从两变三，缺口两侧的症状一模一样
//    （选了没反应 / 根本选不到）。所以判据照旧，只是清单与轴归属跟着换。
//
// ⇒ 逐档对拍（③）+ 档数下限（④）+ **轴归属一致（⑤）** + 变异自证。
//
// # 三条轴（用户口径，2026-10-14）
//
//   | 轴（内部名） | 显示名 | 事件字段 | 档位 |
//   |---|---|---|---|
//   | `style` | 排版 | `style_anim` | `tilt` · `vtext` · `arc`（"默认" = 空档名）|
//   | `transition` | 过渡 | `transition_anim` | `pop` · `fade` |
//   | `effect` | 动效 | `effect_anim` | `shake` · `perchar_big` · `perchar_small` · `heartbeat` |
//
//   9 个档名：tilt arc vtext pop fade shake perchar_big perchar_small heartbeat。
//
// # 怎么扒（零依赖，不 import 应用模块）
//
// `TimelineEditor.vue` 是 `.vue`、`polish-to-dhampir.mjs` 是 ESM。
// 前者只能读文本（正则取 `value: '...'`），后者**可以直接 import**。
// ⚠️ 不从 `.vue` 里 import —— 那要 Vue 编译器，而本目录的判据都是零依赖的。
//
// ⚠️ **旧档名一个都不许在本文件里出现**（`tilt_std` / `slam_lg` / `sway_std` /
//    `fade_only` / `arc_up` / `vtext_bold` / `shrink_in_std` / `pop_perdoc` …
//    全部不存在了）。用户明确「当前没有存量工程还在开发中不要考虑」⇒ **不做别名迁移**，
//    判据也不该要求它。本文件只认那 9 个档名与那三条轴。
import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const HERE = path.dirname(fileURLToPath(import.meta.url))
const ROOT = path.join(HERE, '..')

const TRANS = path.join(ROOT, 'tools', 'polish-to-dhampir.mjs')
const EDITOR = path.join(ROOT, 'webui', 'src', 'components', 'TimelineEditor.vue')

const fails = []
let checks = 0
const ok = (cond, what) => {
  checks++
  if (!cond) fails.push(what)
}

// ---- 轴归属契约（两条实现的轴名不同，靠这张表对应）----
//
// 转译器 `sort` 用 `layout`，界面 `axis` 用 `style` —— **不是笔误**：
// 事件字段是 `style_anim`，界面显示名是「排版」。`layout`⟷`style` 是那份对应关系。
const AXIS_MAP = { layout: 'style', transition: 'transition', effect: 'effect' }
const AXIS_KEYS = Object.keys(AXIS_MAP)

// ---- ① 转译器那份（权威）----
//
// `ANIM_PRESETS` 是 `translate()` 内部的闭包常量 ⇒ **import 不到**。
// 正则从源码里取键名。
// ⚠️ 取的是**对象字面量的键**（`  key: {`），不是"出现过 anim_ 的地方" ——
//    后者会把注释里提到的档名也算进来（本文档自己就提了好几个）。
const transSrc = fs.readFileSync(TRANS, 'utf8')

const presetsStart = transSrc.indexOf('const ANIM_PRESETS = {')
ok(presetsStart > 0, '① 转译器里找不到 `ANIM_PRESETS` —— 三轴 9 档的权威表没了')

let transKeys = []
let transBody = ''
if (presetsStart > 0) {
  // 花括号配对取出整个字面量。
  const open = transSrc.indexOf('{', presetsStart)
  let depth = 0
  let close = -1
  for (let i = open; i < transSrc.length; i++) {
    if (transSrc[i] === '{') depth++
    else if (transSrc[i] === '}') {
      depth--
      if (depth === 0) {
        close = i
        break
      }
    }
  }
  ok(close > open, '① `ANIM_PRESETS` 的花括号没配对（源码被改坏了？）')
  transBody = close > open ? transSrc.slice(open + 1, close) : ''
  // 只取**顶层**键。
  //
  // ⚠️ 别写死缩进宽度：`ANIM_PRESETS` 住在 `translate()` 里 ⇒ 键是**4 个空格**
  //    （第一版写了 `^ {2}` ⇒ 一个档名都取不到，于是"两侧不一致"全线报红，
  //     而**真因是判据自己的正则**，不是代码 —— 这种假红最费时间）。
  //    ⇒ 用"本行缩进 + 紧跟 `name:` + 本行以 `{` 开头结尾"三重约束：
  //      `fade: { sort: 'transition' },` 与 `arc: { …多行… },` 都覆盖，而
  //      `sort: 'effect',` 那种（值带引号）与注释行都不会被误取。
  transKeys = [...transBody.matchAll(/^\s*([a-z][a-z0-9_]*):\s*\{/gm)].map((m) => m[1])
}

ok(transKeys.length > 0, `① 从 ANIM_PRESETS 里一个档名都没取到（实得 ${transKeys.length}）`)
// ⚠️ 挡住"正则退化"：键名都该是 snake_case 且**不含重复**。
ok(new Set(transKeys).size === transKeys.length,
  `① 转译器清单里有重复档名：${JSON.stringify(transKeys.filter((k, i) => transKeys.indexOf(k) !== i))}`)
ok(transKeys.every((k) => /^[a-z][a-z0-9_]*$/.test(k)),
  `① 有档名不像合法标识符：${JSON.stringify(transKeys.filter((k) => !/^[a-z][a-z0-9_]*$/.test(k)))}`)

// ---- ①b 每个键的 `sort`（= 轴归属，**这就是权威**）----
//
// ⚠️ 用**档内逐块**取值，不用"从键到下一个键"的贪婪扫描：
//    `arc` / `perchar_*` / `heartbeat` 的 `sort` 在**下一行**（多行对象），
//    只匹配 `{ ... }` 同一行会漏掉 4 个（实测 9 取 5）⇒ ⑤ 会拿 `undefined` 去比。
//    ⇒ 先按顶层键切块，再在块内找**第一处** `sort:`。
// ⚠️ 块内那处 `sort:` **不一定在行首**：单行档写成 `tilt: { sort: 'layout', … },`
//    （`sort` 跟在 `{ ` 后面）。正则**不许锚 `^`**，否则单行档全被判"没有 sort"
//    （实测：tilt / vtext / pop / fade / shake 五档假红，而真因是判据自己的正则）。
const transSort = new Map()
if (transBody) {
  const keyRe = /^\s*([a-z][a-z0-9_]*):\s*\{/gm
  const marks = [...transBody.matchAll(keyRe)].map((m) => ({ name: m[1], at: m.index }))
  for (let i = 0; i < marks.length; i++) {
    const from = marks[i].at
    const to = i + 1 < marks.length ? marks[i + 1].at : transBody.length
    const piece = transBody.slice(from, to)
    const m = piece.match(/\bsort:\s*'([a-z]+)'/)
    if (m) transSort.set(marks[i].name, m[1])
  }
}
const missingSort = transKeys.filter((k) => !transSort.has(k))
ok(missingSort.length === 0,
  `①b 这些档在 ANIM_PRESETS 里没有 \`sort\`（轴归属缺失，⑤ 无从对拍）：${JSON.stringify(missingSort)}`)
const badSort = [...transSort.entries()].filter(([, v]) => !AXIS_KEYS.includes(v))
ok(badSort.length === 0,
  `①b 这些档的 \`sort\` 不是三条轴之一（${AXIS_KEYS.join(' / ')}）：${JSON.stringify(badSort)}`)

// ---- ② 界面那份 ----
//
// ⚠️ 扒的是**字面量里的 `{ value: '...' }`**（每个 variant 都写成那个形状）。
//    改这一条时想清楚：**扒不到 ⇒ 两侧都是空 ⇒ ③ 全绿**，
//    而"界面一个档都没有"这种事故会**静默通过**。所以下面 ② 与 ④ 有档数下限。
const editorSrc = fs.readFileSync(EDITOR, 'utf8')
const kindsStart = editorSrc.indexOf('const ANIM_KINDS = [')
ok(kindsStart > 0, '② 编辑器里找不到 `ANIM_KINDS` —— 界面那份没了（那就只能手改 toml）')

let uiKeys = []
let kindBlocks = []
if (kindsStart > 0) {
  // 取到 `ANIM_KINDS` 定义结束（下一个顶层 `const ` 之前）。
  const rest = editorSrc.slice(kindsStart)
  const nextTop = rest.indexOf('\nconst ', 1)
  const block = nextTop > 0 ? rest.slice(0, nextTop) : rest
  uiKeys = [...block.matchAll(/\{\s*value:\s*'([a-z0-9_]*)'/g)].map((m) => m[1])
  // 按"只有一个 `{` 的行"切出每个 kind 块。
  // ⚠️ 别写死缩进宽度（本仓踩过：写死 2 个空格 ⇒ 一个块都切不出来 ⇒ 假红）。
  //    `^\s*\{\s*$` 对 2/4 空格都成立，且不会切到 `variants: [{ value… }]` 那种行内对象。
  kindBlocks = block.split(/^\s*\{\s*$/m).slice(1).map((p) => ({
    kind: (p.match(/kind:\s*'([a-z_]+)'/) || [])[1] || '',
    axis: (p.match(/axis:\s*'([a-z_]+)'/) || [])[1] || '',
    values: [...p.matchAll(/\{\s*value:\s*'([a-z0-9_]*)'/g)].map((m) => m[1]),
  }))
}

// ⚠️ 界面那份**含空串**（= 不动效 / 默认）—— 对拍时要去掉，但**空串本身也要存在**
//    （少了它用户就没法取消动效 / 回不到「默认」）。
ok(uiKeys.includes(''), '② 界面清单里没有"默认"那一档（空串）—— 用户就没法取消动效了')
const uiNonEmpty = uiKeys.filter((v) => v !== '')
ok(uiNonEmpty.length > 0, `② 从 ANIM_KINDS 里一个非空档名都没取到（实得 ${uiNonEmpty.length}）`)
ok(new Set(uiNonEmpty).size === uiNonEmpty.length,
  `② 界面清单里有重复档名：${JSON.stringify(uiNonEmpty.filter((k, i) => uiNonEmpty.indexOf(k) !== i))}`)

// ---- ③ 逐档对拍 ----
{
  const t = new Set(transKeys)
  const u = new Set(uiNonEmpty)

  const onlyTrans = [...t].filter((k) => !u.has(k)).sort()
  const onlyUi = [...u].filter((k) => !t.has(k)).sort()

  ok(onlyTrans.length === 0,
    `③ 转译器有、界面**选不到**的档（用户根本用不上这一档）：${JSON.stringify(onlyTrans)}`)
  ok(onlyUi.length === 0,
    `③ 界面有、转译器**不认识**的档（用户选了什么都不会发生，而报告只有一行 CLAMP）：` +
    JSON.stringify(onlyUi))
  ok(t.size === u.size,
    `③ 两侧档数不等：转译器 ${t.size} / 界面 ${u.size}`)
}

// ---- ④ 档数下限（挡住"判据自己空转"）----
//
// ⚠️ 如果两边都被清空了，③ 的"两个 onlyXxx 都是空数组"会**全绿** ——
//    这正是"判据空转"的经典形态。权威档数是 **10**（2026-10-18 起；此前 9）：
//    排版 3（tilt / vtext / arc）+ **过渡 5**（pop / fade / `perchar` + 两个**老名字别名**
//    `perchar_big` / `perchar_small`）+ 动效 2（shake / heartbeat）。
//
//    ⚠️ 为什么过渡变成了 5：用户口径（2026-10-18）「**将逐字变化归类为过渡**，并固定为
//    入场逐字变大、出场逐字变小，不用手动设置」—— 逐字从动效轴挪到过渡轴，**只留一个入口**
//    （`perchar`）；两个老名字仍**留在表里**（老工程的 `effect_anim` 写着它们，
//    删了就是"认不出的档 ⇒ 界面显示不设、一保存就丢"），它们**不进下拉**（`legacy: true`）。
// ⚠️ 用 **=== 10**、不是 `>= 10`：档数是**契约**，
//    多出来的档同样是缺口（用户清单之外的档谁也选不到 / 或界面多出一条没人认）。
const EXPECTED_TIERS = 10
ok(transKeys.length === EXPECTED_TIERS,
  `④ 转译器的档数应为 ${EXPECTED_TIERS} 档（实得 ${transKeys.length}）—— ` +
  '要么档被删/多加了，要么正则取漏了')
ok(uiNonEmpty.length === EXPECTED_TIERS,
  `④ 界面的档数应为 ${EXPECTED_TIERS} 档（实得 ${uiNonEmpty.length}）—— ` +
  '要么档被删/多加了，要么正则取漏了')

// ---- ⑤ **三轴归属一致**（2026-10-14 新增）----
//
// ⚠️ ③ 只保证"档名两边都有"，**不保证"它在两边属于同一条轴"**。
//    这是**两次不同的事故**，而症状不同：
//      * 界面上归 `style`（排版栏）、转译器按 `effect` 给字段 ⇒ 用户从「排版」栏
//        选了它，成片里走的是**动效**那条路 ⇒ 观感与栏目标题对不上；
//        更糟的是它会被写进 `style_anim`，而转译器的**跨轴误用检查**
//        （`animAxisOf(name) !== want` 那条）会报"填错栏" ——
//        **用户是按界面选的，却被告知填错了**。
//      * 反向（界面 effect / 转译器 layout）同理。
//    ⇒ 逐档对拍 `sort` 与 `axis` 的对应关系（`layout`⟷`style` / `transition`⟷
//      `transition` / `effect`⟷`effect`，见 `AXIS_MAP`）。
const uiAxisOf = new Map()      // 档名 → 界面轴的**内部名**（未映射）
for (const b of kindBlocks) {
  for (const v of b.values) if (v !== '') uiAxisOf.set(v, b.axis)
}
{
  const mismatched = []
  const unreadable = []
  for (const name of [...new Set([...transKeys, ...uiNonEmpty])].sort()) {
    const raw = transSort.get(name)
    // 转译器侧的 `sort` 直接就是内部轴名（`layout`/`transition`/`effect`）；
    // 界面侧写的是 `style`/`transition`/`effect` ⇒ 要过 `AXIS_MAP` 反过来对。
    const want = AXIS_MAP[raw]           // 转译器声称的**界面**内部轴名
    const got = uiAxisOf.get(name)       // 界面实际归的轴
    if (!want || !got) { unreadable.push([name, raw ?? '(无 sort)', got ?? '(界面无 axis)']); continue }
    if (want !== got) mismatched.push(`${name}: 转译器 sort='${raw}'(→${want}) vs 界面 axis='${got}'`)
  }
  ok(unreadable.length === 0,
    `⑤ 这些档有一侧的轴归属读不到（轴对拍会空转）：${JSON.stringify(unreadable)}`)
  ok(mismatched.length === 0,
    `⑤ 三轴归属不一致（用户从哪一栏选、成片走哪条轴，两边说的不是一件事）：` +
    JSON.stringify(mismatched))

  // ⚠️ 三条轴**每条都得有档** —— 少一条的症状是"那一栏整个空着"，
  //    而 ③④⑤ 在"某轴 0 档"时全绿（它只比对存在的档）。
  const axesSeen = new Set([...transSort.values()].map((v) => AXIS_MAP[v]).filter(Boolean))
  const missingAxes = Object.values(AXIS_MAP).filter((a) => !axesSeen.has(a))
  ok(missingAxes.length === 0,
    `⑤ 这些轴在转译器的档位表里**一个档都没有**（那一栏整个空着）：${JSON.stringify(missingAxes)}`)
}

// ---- ⑥ ⚠⚠ **`perChar` 的键名**：两份表一个蛇形、一个驼峰 ----
//
// ⚠⚠⚠ 这是上面那类漂移的**第二层**，而且更隐蔽：
//
//   * 转译器 `ANIM_PRESETS[k].perChar` = **蛇形**（`rotation_from` / `per_char_frames`…）
//     —— 它直接走底座字段；
//   * 预览 `engine.js::ANIM_PRESETS[k].perChar` = **驼峰**（`rotationFrom` / `perCharFrames`…）
//     —— 它直接喂 `charTransformAt`。
//
//   ⚠ 两份都叫 `perChar`、都是"同一张表"，**中间没有任何转换**：
//     转译器那份不经 `readPerChar`（那是读 toml 那条路的），
//     预览那份也不经（`animPerChar` 直接 `{…默认, …p.perChar}` 展开）。
//
//   ⇒ **今天恰好自洽**（两边都是手抄的、值也对得上），但**改一处就静默漂**：
//     在转译器那边加一个字段（比如 `rotation_from_x`），预览这边拿不到就默默用默认值
//     —— 成片里转了、预览里没转，两边各自看都正常。
//
//   ⚠ 判据只能钉**键名集合**（不钉值：值本来就该各自定义）。
//     两边的 `perChar` 必须覆盖**同一组语义字段**。
{
  // ⚠️⚠️ **必须逐档对**，不能只比"键的并集" —— 实测：
  //    只比并集时，把 `perchar_small` 的 `stagger_frames` 删掉判据**照样绿**
  //    （`perchar_big` 里还有，并集不变）⇒ 漏掉的正是"**某一档**漂了"。
  //    这与 `check-anim-presets-distinct` 那条"两个预设产物相同"是同一类教训：
  //    **整体对得上 ≠ 逐项对得上**。
  // ⚠ 预览那份源码（驼峰）。
  const uiSrc = fs.readFileSync(path.join(ROOT, 'webui/src/polish/engine.js'), 'utf8')

  /** 按**档**切开预设表，再在每档里找 `perChar: { … }`。
   *
   *  ⚠⚠ 不能用一条跨档的非贪心匹配去找 `perChar` ——
   *     它会把“**下一档的名字**”当成这一档的名字
   *     （实测抽出 `["perchar_small","tags","vtext"]`）。
   *     ⇒ 先按顶格缩进切档，再在档内找。
   */
  const perTierPerChar = (src, indent) => {
    const out = new Map()
    const head = new RegExp('^' + indent + '([a-z][a-z0-9_]*):\\s*\\{', 'gm')
    const marks = []
    for (const m of src.matchAll(head)) marks.push({ name: m[1], at: m.index })
    for (let i = 0; i < marks.length; i++) {
      const body = src.slice(marks[i].at, i + 1 < marks.length ? marks[i + 1].at : src.length)
      const pc = body.match(/perChar:\s*\{([^}]*)\}/)
      if (!pc) continue
      const keys = new Set()
      for (const k of pc[1].matchAll(/([a-zA-Z_][a-zA-Z0-9_]*)\s*:/g)) keys.add(k[1])
      out.set(marks[i].name, keys)
    }
    return out
  }
  // ⚠️⚠️ **只在档位表各自的范围内切档**（2026-10-18 修）：
  //    以前喂的是**整份源码**，于是别的表里那些同样缩进的条目也会被当成"档" ——
  //    实测转译器侧抽出过一个 `count`（那是 `TEXT_STYLE_FALLBACK.count`，**不是动效档**），
  //    而它的"档体"一路切到下一个同缩进条目、**里面恰好有 `perChar:`** ⇒
  //    ⑥ 报"预览侧少了 count"，指向一个根本不存在的档。
  //    （这个脚本自己的注释就写着这种切法脆弱 —— 现在把范围收窄到表内。）
  const uiStart = uiSrc.indexOf('export const ANIM_PRESETS = {')
  const uiClose = uiStart > 0 ? uiSrc.indexOf('\n}', uiStart) : -1
  const uiBody = uiStart > 0 && uiClose > uiStart ? uiSrc.slice(uiStart, uiClose) : uiSrc
  const transPerTier = perTierPerChar(transBody, '    ')
  const uiPerTier = perTierPerChar(uiBody, '  ')
  const snakeToCamel = (s) => s.replace(/_([a-z0-9])/g, (_, c) => c.toUpperCase())

  ok(transPerTier.size > 0 && uiPerTier.size > 0,
    '⑥ 两边至少有一边抽不到 `perChar`（源码形状变了？） —— ' +
    `转译器 ${transPerTier.size} 档 / 预览 ${uiPerTier.size} 档`)

  // ① 有 `perChar` 的档集合必须一致。
  const tk = [...transPerTier.keys()].sort()
  const uk = [...uiPerTier.keys()].sort()
  ok(JSON.stringify(tk) === JSON.stringify(uk),
    `⑥ 带 \`perChar\` 的档不一致：转译器 ${JSON.stringify(tk)} / 预览 ${JSON.stringify(uk)}`)

  // ② **逐档**字段集合一致（蛇形→驼峰后比）。
  for (const tier of tk) {
    if (!uiPerTier.has(tier)) continue
    const want = new Set([...transPerTier.get(tier)].map(snakeToCamel))
    const got = uiPerTier.get(tier)
    const missing = [...want].filter((k) => !got.has(k))
    const extra = [...got].filter((k) => !want.has(k))
    ok(missing.length === 0,
      `⑥ 档 \`${tier}\` 的 \`perChar\` 在预览里**少了**：${JSON.stringify(missing)}
` +
      `     ⚠ 成片里有这个参数、预览里没有（默默用默认值）——两边各自看都正常`)
    ok(extra.length === 0,
      `⑥ 档 \`${tier}\` 的 \`perChar\` 在预览里**多了**：${JSON.stringify(extra)}
` +
      `     ⚠ 这个参数在成片里**没人读**（预览自己演了一套）`)
  }
}

// ---- 结果 ----
if (fails.length === 0) {
  console.log(
    `✓ 三轴 9 档动效清单两侧一致（${checks} 项判据 / 转译器 ${transKeys.length} 档 · ` +
    `界面 ${uiNonEmpty.length} 档 + 默认）`,
  )
  console.log(
    `  轴归属：排版(style) ${transKeys.filter((k) => transSort.get(k) === 'layout').length} · ` +
    `过渡(transition) ${transKeys.filter((k) => transSort.get(k) === 'transition').length} · ` +
    `动效(effect) ${transKeys.filter((k) => transSort.get(k) === 'effect').length}`,
  )
  process.exit(0)
}
console.log(`✗ 三轴 9 档动效清单对拍失败（${fails.length} / ${checks}）：`)
for (const f of fails.slice(0, 20)) console.log('    ' + f)
process.exit(1)
