#!/usr/bin/env node
// **39 档动效：界面清单 ↔ 转译器清单 逐档对拍**（2026-10-13，第 3 步 A+B 组）。
//
// # 为什么必须有这条判据
//
// 39 档的名字现在住在**两个地方**：
//
//   ① `tools/polish-to-dhampir.mjs::ANIM_PRESETS` —— **权威**（底座字段的映射表）；
//   ② `webui/src/components/TimelineEditor.vue::ANIM_GROUPS` —— 界面那份（给用户选）。
//
// 两份**必须同集合**。缺一边的症状是本仓最恨的那一类：
//
//   * 界面有、转译器没有 ⇒ 用户选了下拉、**成片里什么都不发生**，
//     报告里只有一行 CLAMP（中文档名，用户很可能没看）；
//   * 转译器有、界面没有 ⇒ 这一档**用户根本选不到**（"39 档"实际只有 N 档），
//     而任何"档数对不对"的人工核对都会数到 39 —— 因为转译器那边确实是 39。
//
// ⇒ 逐档对拍 + 变异自证。
//
// # 怎么扒（零依赖，不 import 应用模块）
//
// `TimelineEditor.vue` 是 `.vue`、`polish-to-dhampir.mjs` 是 ESM。
// 前者只能读文本（正则取 `value: '...'`），后者**可以直接 import**。
// ⚠️ 不从 `.vue` 里 import —— 那要 Vue 编译器，而本目录的判据都是零依赖的。
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

// ---- ① 转译器那份（权威）----
//
// `ANIM_PRESETS` 是 `translate()` 内部的闭包常量 ⇒ **import 不到**。
// 与 `check-sway-parity.mjs` 同一个做法：正则从源码里取键名。
// ⚠️ 取的是**对象字面量的键**（`  key: {`），不是"出现过 anim_ 的地方" ——
//    后者会把注释里提到的档名也算进来（本文档自己就提了好几个）。
const transSrc = fs.readFileSync(TRANS, 'utf8')

const presetsStart = transSrc.indexOf('const ANIM_PRESETS = {')
ok(presetsStart > 0, '① 转译器里找不到 `ANIM_PRESETS` —— 39 档的权威表没了')

let transKeys = []
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
  const body = close > open ? transSrc.slice(open + 1, close) : ''
  // 只取**顶层**键。
  //
  // ⚠️ 别写死缩进宽度：`ANIM_PRESETS` 住在 `translate()` 里 ⇒ 键是**4 个空格**
  //    （第一版写了 `^ {2}` ⇒ 一个档名都取不到，于是"两侧不一致"全线报红，
  //     而**真因是判据自己的正则**，不是代码 —— 这种假红最费时间）。
  //    ⇒ 用"本行缩进 + 紧跟 `name:` + 本行以 `{` 开头结尾"三重约束：
  //      `fade_only: {},` 与 `slam_squash: { … },` 都覆盖，而
  //      `group: '...'` 那种（值带引号）与注释行都不会被误取。
  transKeys = [...body.matchAll(/^\s*([a-z][a-z0-9_]*):\s*\{/gm)].map((m) => m[1])
}

ok(transKeys.length > 0, `① 从 ANIM_PRESETS 里一个档名都没取到（实得 ${transKeys.length}）`)
// ⚠️ 挡住"正则退化"：键名都该是 snake_case 且**不含重复**。
ok(new Set(transKeys).size === transKeys.length,
  `① 转译器清单里有重复档名：${JSON.stringify(transKeys.filter((k, i) => transKeys.indexOf(k) !== i))}`)
ok(transKeys.every((k) => /^[a-z][a-z0-9_]*$/.test(k)),
  `① 有档名不像合法标识符：${JSON.stringify(transKeys.filter((k) => !/^[a-z][a-z0-9_]*$/.test(k)))}`)

// ---- ② 界面那份 ----
//
// ⚠️ **2026-10-13：界面从"一个下拉 + optgroup"改成两级（种类 → 细分）**，
//    权威表也从扁平的 `ANIM_GROUPS` 换成了 `ANIM_KINDS`
//    （`ANIM_GROUPS` 现在只是它 `.map()` 出来的兼容视图，**不再是源**）。
//    ⇒ 这里改成扒 `ANIM_KINDS`。
//
// ⚠️⚠️ 扒的仍然是**字面量里的 `{ value: '...' }`**（每个 variant 都写成那个形状）。
//    改这一条时想清楚：**扒不到 ⇒ 两侧都是空 ⇒ ③ 全绿**，
//    而"界面一个档都没有"这种事故会**静默通过**。所以下面 ② 与 ④ 有档数下限。
const editorSrc = fs.readFileSync(EDITOR, 'utf8')
const kindsStart = editorSrc.indexOf('const ANIM_KINDS = [')
ok(kindsStart > 0, '② 编辑器里找不到 `ANIM_KINDS` —— 界面那份没了（那就只能手改 toml）')

let uiKeys = []
let uiKindCount = 0
if (kindsStart > 0) {
  // 取到 `ANIM_KINDS` 定义结束（下一个顶层 `const ` 之前）。
  const rest = editorSrc.slice(kindsStart)
  const nextTop = rest.indexOf('\nconst ', 1)
  const block = nextTop > 0 ? rest.slice(0, nextTop) : rest
  uiKeys = [...block.matchAll(/\{\s*value:\s*'([a-z0-9_]*)'/g)].map((m) => m[1])
  // 种类数：顶层 `kind: '...'` 出现几次。
  // ⚠️ 缩进是 **4 个空格**（对象在数组里，数组项又缩了一层）。
  //    写死 2 会让它一个都取不到 ⇒ ④b 全线假红（而真因是判据自己的正则）。
  uiKindCount = [...block.matchAll(/^\s+kind:\s*'([a-z_]+)',\s*$/gm)].length
}

// ⚠️ 界面那份**含空串**（= 不动效）—— 对拍时要去掉，但**空串本身也要存在**。
ok(uiKeys.includes(''), '② 界面清单里没有"不动效"那一档（空串）—— 用户就没法取消动效了')
const uiNonEmpty = uiKeys.filter((v) => v !== '')
ok(uiNonEmpty.length > 0, `② 从 ANIM_GROUPS 里一个非空档名都没取到（实得 ${uiNonEmpty.length}）`)
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
//    这正是"判据空转"的经典形态。A+B 组是 **17 档**（A 3 + B 14）。
ok(transKeys.length >= 17,
  `④ 转译器的档数少于 A+B 组的 17 档（实得 ${transKeys.length}）—— ` +
  '要么档被删了，要么正则取漏了')
ok(uiNonEmpty.length >= 17,
  `④ 界面的档数少于 A+B 组的 17 档（实得 ${uiNonEmpty.length}）`)

// ---- ④b 两级结构：**拆的过程不许丢档**（2026-10-13）----
//
// ⚠️ 界面从"扁平 40 项"改成"种类 → 细分"两级。拆的过程有两种静默丢档的形态，
//    它们都**不会**被 ③ 抓到（③ 只看转译器 ↔ 界面的集合差）：
//      ① 某个 variant 被漏写（那一档用户**选不到**了）；
//      ② 两个 kind 里写了**同一个** variant（重复）。
//    ③ 能抓到 ① 吗？能 —— 只要 uiKeys 的合计少了一个，③ 就报 onlyTrans。
//    但 **② 重复**抓不到，而且"两级并集 == 扁平表"这件事**没有任何判据**
//    （扁平表已经被删了，没有对照物）。⇒ 这里把**结构不变式**钉死：
//      * 种类数 ≥ 2（不然两级是假的）；
//      * 每个种类至少 1 个细分（空种类 = 用户选进去什么都没有）；
//      * 除"不动效"外，每个种类至少 2 个细分（只有一项的"两级"是假两级）；
//      * 全局档名不重复（跨种类也算）。
{
  ok(uiKindCount >= 2,
    `④b 种类数应当 ≥ 2（实得 ${uiKindCount}）—— 两级结构没生效`)
  // 逐 kind 数它的 variant（按源码块切）。
  const kindBlocks = editorSrc.slice(editorSrc.indexOf('const ANIM_KINDS = ['))
  const cut = kindBlocks.indexOf('\nconst ')
  const body = cut > 0 ? kindBlocks.slice(0, cut) : kindBlocks
  // ⚠️ 种类块的 `{` 是 **2 空格**缩进、而里面的 `kind:` 是 **4 空格**
  //    （数组项一层 + 对象体一层）。两个数不一样，别写成同一个。
  const parts = body.split(/^ {2}\{\s*$/m).slice(1)
  ok(parts.length === uiKindCount,
    `④b 切出来的种类块数（${parts.length}）与读到的 kind 数（${uiKindCount}）对不上 —— 判据自己的切法坏了`)
  const emptyKinds = []
  const singleKinds = []
  for (const p of parts) {
    const k = (p.match(/kind:\s*'([a-z_]+)'/) || [])[1] || '(取不到)'
    const n = [...p.matchAll(/\{\s*value:\s*'([a-z0-9_]*)'/g)].length
    if (n === 0) emptyKinds.push(k)
    else if (n === 1 && k !== 'none') singleKinds.push(k)
  }
  ok(emptyKinds.length === 0,
    `④b 这些种类一个细分都没有（选进去空空如也）：${JSON.stringify(emptyKinds)}`)
  ok(singleKinds.length === 0,
    `④b 这些种类只有 1 个细分 —— 那是**假两级**（"种类"与"细分"是同一个选择）：${JSON.stringify(singleKinds)}`)
}

// ---- ⑤ 档名要"看起来成套"（挡住把一整组写成同一个前缀的笔误）----
{
  const families = ['tilt', 'shake', 'sway', 'slam', 'shrink_in', 'pop']
  const missing = families.filter((f) => !transKeys.some((k) => k.startsWith(f)))
  ok(missing.length === 0,
    `⑤ 这些族在转译器清单里一个档都没有：${JSON.stringify(missing)}`)
  // ⚠️ `fade_only` 不跟任何前缀 —— 单独钉一下它还在（它是"只淡出"，
  //    也是"空预设压掉全局弹入"那条语义的唯一入口）。
  ok(transKeys.includes('fade_only') && uiNonEmpty.includes('fade_only'),
    '⑤ `fade_only`（只淡出）必须在两侧都在 —— 它是"空预设"那条语义的唯一入口')
}

// ---- ⑥ 接线：下拉真的接到事件字段上了吗 ----
//
// ⚠️ 光有清单不算接通 —— 本仓踩过多次"函数写好、没人调"。
//
// ⚠️ **2026-10-13：两级界面之后，接线形态变了**（一个下拉 → 种类 + 细分两个）。
//    断言跟着改，但**下面那条"空串必须落成 `null`"一个字都不许放宽**
//    —— 它守的是 R4（写空串会让老工程的 toml 多一个键、产物变）。
// ① 两级控件必须**都**接到事件字段上。
ok(/setAnimKind\(/.test(editorSrc),
  '⑥ 种类控件没有接线（`setAnimKind` 没人调）—— 用户选了种类写不进事件')
ok(/setAnimVariant\(/.test(editorSrc),
  '⑥ 细分控件没有接线（`setAnimVariant` 没人调）—— 用户选了细分写不进事件')
ok(/<select[^>]*:value="animKind"[^>]*@change="setAnimKind\(\$event\.target\.value\)"/.test(editorSrc),
  '⑥ 种类下拉没有绑到 `animKind` / `setAnimKind($event.target.value)`')
ok(/<select[^>]*:value="selItem\.anim \|\| ''"[^>]*@change="setAnimVariant\(\$event\.target\.value\)"/.test(editorSrc),
  '⑥ 细分下拉没有绑到 `selItem.anim` / `setAnimVariant($event.target.value)`')
// ② **两个**写回点都必须落 `null`。
{
  const writes = [...editorSrc.matchAll(/selItem\.value\.anim = ([^\n]+)/g)].map((m) => m[1].trim())
  ok(writes.length === 2,
    `⑥ 写回 \`selItem.anim\` 的地方应当**恰好 2 处**（种类 / 细分），实得 ${writes.length}：${JSON.stringify(writes)}`)
  const bad = writes.filter((w) => !/\|\|\s*null$/.test(w))
  ok(bad.length === 0,
    `⑥ 有写回没有落成 \`null\`（空串会写进 toml，老工程产物变了）：${JSON.stringify(bad)}`)
}
// ③ 认不出的档名仍要**看得见**（不许被悄悄归成"不动效"）。
ok(/ANIM_KNOWN\.includes\(selItem\.anim\)/.test(editorSrc),
  '⑥ 认不出的档名没有被显式显示出来 —— 用户会看到"不动效"而以为自己没选错')
ok(/animKind === ''/.test(editorSrc),
  '⑥ 认不出的档名没有让**种类**控件落到"未知"那一支 —— 两级界面下用户看不到它是哪一类')
// 转译器那侧：`anim` 必须真的被读进来（不是只定义了预设表）。
ok(/String\(event\.anim \?\? ''\)/.test(transSrc),
  '⑥ 转译器没读 `event.anim` —— 预设表定义了却没人用')
ok(/animPreset\(cues\[0\]\.anim\)/.test(transSrc),
  '⑥ 预设没被摊进文字轨（`motionFields` 拿不到它）')
ok(/ANIM_KNOWN\.join/.test(transSrc),
  '⑥ 认不出的档名没在报告里列出已知档 —— 用户不知道该改成什么')

// ---- ⑦ 预览那份（第三份实现）----
//
// ⚠️ **39 档现在有三份实现**：转译器（权威）/ 界面下拉 / **预览 `engine.js`**。
//    前两份由 ①–⑥ 对拍；预览那份**以前根本没有** ——
//    症状是"用户在时间轴上选了摇摆，成片里摆、**预览里纹丝不动**"。
//    ⇒ 这里对拍它的档名集合（值是否同值由 `check-sway-parity.mjs` 与
//      转译器守卫各自钉住；本条只管"档名一个不多一个不少"）。
{
  // `engine.js` 是纯 ESM、零依赖 ⇒ **可以直接 import**（比扒文本可靠）。
  const engine = await import(new URL('../webui/src/polish/engine.js', import.meta.url).href)
  const presets = engine.ANIM_PRESETS
  ok(presets && typeof presets === 'object',
    '⑦ 预览侧没有导出 `ANIM_PRESETS` —— 预览就无从知道每条元素选了哪一档')

  if (presets) {
    const engKeys = Object.keys(presets).sort()
    const t = new Set(transKeys)
    const e = new Set(engKeys)
    const onlyT = [...t].filter((k) => !e.has(k)).sort()
    const onlyE = [...e].filter((k) => !t.has(k)).sort()
    ok(onlyT.length === 0,
      `⑦ 转译器有、**预览不认识**的档（选了它预览里不会动）：${JSON.stringify(onlyT)}`)
    ok(onlyE.length === 0,
      `⑦ 预览有、转译器没有的档：${JSON.stringify(onlyE)}`)

    // ⚠️ 空预设那一档必须**在**预览表里 —— 它是"压掉全局弹入"的唯一入口。
    ok(Object.prototype.hasOwnProperty.call(presets, 'fade_only'),
      '⑦ 预览表里缺 `fade_only`（它就是"只淡出"那一档）')
    // ⚠️ 而且它必须是**真·空**（有几个键就不是"只淡出"了）。
    ok(Object.keys(presets.fade_only ?? {}).length === 0,
      `⑦ \`fade_only\` 必须是空预设，实得 ${JSON.stringify(presets.fade_only)}`)

    // ⚠️ `animPreset` 的"认不出"必须返回 `undefined`（不是空对象）——
    //    返回空对象会让"手滑写错档名"变成**静默的只淡出**（看着像生效了）。
    ok(typeof engine.animPreset === 'function', '⑦ 预览没导出 `animPreset`')
    if (typeof engine.animPreset === 'function') {
      ok(engine.animPreset('sway_stdd') === undefined,
        '⑦ 认不出的档名必须返回 `undefined`（返回 `{}` 会变成静默的「只淡出」）')
      ok(engine.animPreset('') === undefined && engine.animPreset(null) === undefined,
        '⑦ 空档名必须返回 `undefined`（= 没写 anim ⇒ 回落全局）')
      ok(engine.animPreset('SWAY_LG') !== undefined,
        '⑦ 档名应当大小写不敏感（用户在 toml 里手写时容易大写）')
    }
  }
}

// ---- 结果 ----
if (fails.length === 0) {
  console.log(
    `✓ 39 档动效清单两侧一致（${checks} 项判据 / 转译器 ${transKeys.length} 档 · ` +
    `界面 ${uiNonEmpty.length} 档 + 不动效）`,
  )
  process.exit(0)
}
console.log(`✗ 39 档动效清单对拍失败（${fails.length} / ${checks}）：`)
for (const f of fails.slice(0, 20)) console.log('    ' + f)
process.exit(1)
