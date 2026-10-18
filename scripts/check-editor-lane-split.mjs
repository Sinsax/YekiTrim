#!/usr/bin/env node
// **前端编辑器的「文字 / 贴纸」必须分成两条泳道**（2026-10-14）。
//
// # 背景（用户口径：「前端编辑器也要分离」）
//
// 磁盘上 `element` 与 `sticker` 是**同一条轨**（`Lane::Sticker` ⇒ `"sticker"`，
// 四处 `LANE_OF_TYPE` 镜像都这么映射）—— 那个**不能改**（改了就要求数据迁移，
// 而老工程里全是 `type:"sticker"`）。
//
// 但**界面上**它们该是两条轨：载荷、属性面板、能选的东西完全不同
// （文字有排版/描边/逐字，贴纸只有图和落点）。
// 混在一条轨上的代价是"＋"必须弹「文字 / 贴纸」二选一菜单 ——
// 而用户本来就是在**某一条轨**上点的加号，那个选择是多余的。
//
// # 它判什么（每一条都做变异自证）
//
// ① 泳道表里**有两条**（`kinds` 分别是文字 / 贴纸），但 `lane` **都是 `'sticker'`**
//    —— 界面分家、磁盘不分家。⚠️ 这条是本次改动的**定义**。
// ② `laneKey` 与 `laneTypeOf` **是两个不同的东西**：
//    `laneKey`（界面 id）用于 `v-for :key` 与选中判等；`laneTypeOf`（轨道名）用于 `emit`。
//    ⚠️ 混用的两种症状都是**静默**的：
//      * `emit` 发 `laneKey` ⇒ `PolishView` 查不到 `laneGroupEls.text` ⇒ **点轨道名没反应**；
//      * `:key` 用 `laneTypeOf` ⇒ 两条泳道**共用同一个 key** ⇒ Vue 复用错节点。
// ③ 模板里 `:key` **不许**用 `laneTypeOf`、`emit('lane-open')` **不许**用 `laneKey`。
// ④ 两个 ＋ 各自**直接**建对应 kind 的元素（不再有"二选一菜单"）。
// ⑤ ⚠️ **反例**：`elementKindOf` 认不出时必须是 `'unknown'`，**不许默认成 `'text'`**
//    （默认成文字 = 把坏数据伪装成正常元素）。
// ⑥ 界面上**不许再出现「元素」这个类别名** —— 含 ASCII 的**磁盘载体名 `element`**
//    （用户原话：「因为元素(原文字+贴纸)已分离，请查找元素相关文案进行修改」）。
//
// 用法：node scripts/check-editor-lane-split.mjs   （退出码非 0 即失败）

import { readFileSync } from 'node:fs'
import { join, dirname } from 'node:path'
import { fileURLToPath } from 'node:url'

const HERE = dirname(fileURLToPath(import.meta.url))
const ROOT = join(HERE, '..')
const TL = join(ROOT, 'webui', 'src', 'components', 'TimelineEditor.vue')

let pass = 0
let fail = 0
const failures = []
function ok(cond, what) {
  if (cond) { pass += 1 } else { fail += 1; failures.push(what) }
}

const src = readFileSync(TL, 'utf8')

// 剥注释 —— 本文件里解释性注释大量提到这些符号，
// 不剥的话判据会被**它自己让人写下的说明**顶翻。⚠️ `(?<!:)` 避开 `https://`。
//
// ⚠️⚠️ **`.vue` 里必须连 HTML 注释（`<!-- -->`）一起剥**（2026-10-15 踩到）：
//    只剥 `//` 与 `/* */` 时，写在 `<template>` 里的警告（"别再加回 `xxx`"）
//    **本身就是一段代码**，于是"`xxx` 不许出现"的判据被**它自己的说明**顶翻 ——
//    假红，而且修法看起来像"把警告删掉"（删掉解释 = 后人不知道为什么）。
//    ⇒ 剥注释不是洁癖，是让判据读的是**行为**而不是**关于行为的散文**。
const stripComments = (s) => s
  .replace(/<!--[\s\S]*?-->/g, '')
  .replace(/\/\*[\s\S]*?\*\//g, '')
  .replace(/(^|[^:])\/\/[^\n]*/g, '$1')
const code = stripComments(src)

// ---------------------------------------------------------------------------
// ① 泳道表：两条，但 `lane` 都是 `'sticker'`
// ---------------------------------------------------------------------------
const laneDefs = (() => {
  const i = src.indexOf('const LANE_DEFS = [')
  if (i < 0) return null
  let d = 0
  for (let j = src.indexOf('[', i); j < src.length; j++) {
    if (src[j] === '[') d++
    else if (src[j] === ']') { d--; if (d === 0) return src.slice(i, j + 1) }
  }
  return null
})()
ok(!!laneDefs, '① 找得到 `LANE_DEFS`（结构变了就要改本判据）')

// ⚠️ 按**整条对象**切（泳道定义常跨两行：`id`/`kinds` 在一行、`lane`/`label` 在下一行）。
//    按行切的话 `lane` 会取到 `undefined`，判据报的是"lane 必须是 sticker、实得 undefined"——
//    那是**我自己解析错**，不是源码错。按 `{ id:` 起、下一个 `{ id:` 或 `]` 止来切。
const byId = {}
{
  const chunks = laneDefs ? laneDefs.split(/\{\s*(?=id:)/).slice(1) : []
  for (const chunk of chunks) {
    const body = chunk.split(/\n\s*\{/)[0]
    const id = (body.match(/\bid:\s*'([^']+)'/) || [])[1]
    const lane = (body.match(/\blane:\s*'([^']+)'/) || [])[1]
    const kindsRaw = (body.match(/\bkinds:\s*\[([^\]]*)\]/) || [])[1]
    if (id) {
      byId[id] = {
        lane,
        sharedEye: /\bsharedEye:\s*true/.test(body),
        kinds: kindsRaw === undefined
          ? null
          : kindsRaw.split(',').map((x) => x.trim().replace(/^'|'$/g, '')),
      }
    }
  }
}

ok(!!byId.text, '① 有「文字」轨（`id: \'text\'`）')
ok(!!byId.sticker, '① 有「贴纸」轨（`id: \'sticker\'`）')

// ⚠️ 两条的 `lane` 必须都是 `'sticker'` —— 界面分家 / 磁盘不分家
for (const id of ['text', 'sticker']) {
  if (byId[id]) {
    ok(byId[id].lane === 'sticker',
      `① 「${id}」轨的 \`lane\` 必须是 \`'sticker'\`（磁盘上与贴纸同轨，不做数据迁移）；实得 ${JSON.stringify(byId[id].lane)}`)
  }
}

// ⚠️ `kinds` 必须把两种分开，且**必须重叠为空**
if (byId.text && byId.sticker) {
  ok(byId.text.kinds && byId.text.kinds.includes('text'),
    '① 文字轨的 `kinds` 必须含 `text`')
  ok(byId.text.kinds && byId.text.kinds.includes(''),
    '①⚠️ 文字轨的 `kinds` **必须含空串** —— `kind` 缺失/为空是最常见的存量形状'
    + '（Rust 侧 `#[serde(default)]` 兜空串、面板新建的也是它），漏了它那些元素会在**两条轨上都看不见**')
  ok(byId.sticker.kinds && byId.sticker.kinds.includes('sticker') && !byId.sticker.kinds.includes('text'),
    '① 贴纸轨的 `kinds` 必须只含 `sticker`（不许含 `text` —— 否则两条轨重叠、块画两遍）')
  const inter = (byId.text.kinds || []).filter((k) => (byId.sticker.kinds || []).includes(k))
  ok(inter.length === 0, `① 两条轨的 \`kinds\` **不许重叠**（实得重叠 ${JSON.stringify(inter)}）`)
}

// ---------------------------------------------------------------------------
// ①b **每条内容轨都要有一枚能点的眼睛**（2026-10-15 用户口径，**逆转了旧设计**）
// ---------------------------------------------------------------------------
//
//  ⚠️⚠️ **旧设计（已作废）**：`文字` 与 `贴纸` 磁盘上同一条轨（都 `lane: 'sticker'`）
//  ⇒ 两条都画眼睛 = 两枚按钮管同一个 `[stage.lanes].sticker` 开关，
//  点哪一枚另一枚都跟着变 —— 当时判断这是缺陷，于是**文字轨放不可点的占位**。
//
//  ⚠️ **那个判断站不住**（用户报的正是它）：`laneEnabled` 的键**就是轨道名**
//  （两条都是 `'sticker'`）⇒ 共用开关**恰恰意味着**两枚眼睛读数一致、谁点都生效 ——
//  那正是用户要的（"文字不渲染"与"贴纸不渲染"本来就是同一件事，
//  落盘也只有一个 `[stage.lanes].sticker`）。占位符带来的**真实**体验是
//  "文字轨前面有个坏掉的符号、还点不动"。
//  ⇒ 现在两条都走**同一条**可点分支；`sharedEye` 字段与占位支**都已删除**。
{
  // 占位支不许回来（字段或模板任一残留都算）。
  ok(!/sharedEye/.test(code),
    '①b `sharedEye` 占位机制**不许回来** —— 文字轨要一枚**真的能点**的眼睛'
    + '（用户报过"前面那个符号不对、还点不动"）')
  // 眼睛分支只按 `lane.lane` 门控（不是"排除某条"）。
  ok(/v-if="lane\.lane"/.test(code),
    '①b 眼睛的 `v-if` 只许按 `lane.lane` 门控（每条有轨道名的轨都画眼睛）')
  // 两条泳道的 `lane` 都是 `sticker` ⇒ 它们读到的是同一个开关（这是**对的**）。
  ok(byId.text && byId.text.lane === 'sticker' && byId.sticker && byId.sticker.lane === 'sticker',
    '①b 文字与贴纸的 `lane` 都必须是 `sticker`（同一条磁盘轨 ⇒ 同一个眼睛开关）')
}

// ---------------------------------------------------------------------------
// ② `laneKey` 与 `laneTypeOf` 是两个不同的东西，且定义得对
// ---------------------------------------------------------------------------
ok(/const laneKey = \(lane\) => lane\.id \|\| lane\.type/.test(code),
  '② `laneKey` 必须是「界面 id 优先」：`lane.id || lane.type`')
ok(/const laneTypeOf = \(lane\) => lane\.type/.test(code),
  '② `laneTypeOf` 必须是「轨道名」：`lane.type`')

// ---------------------------------------------------------------------------
// ③ 用途不许混（两条都是**静默**故障）
// ---------------------------------------------------------------------------
{
  // `:key` 不许用 laneTypeOf（两条轨会共用同一个 key）
  const keyLines = code.split('\n').filter((l) => /:key=/.test(l) && /lane/.test(l))
  const badKey = keyLines.filter((l) => /laneTypeOf/.test(l))
  ok(badKey.length === 0,
    `③ \`v-for\` 的 \`:key\` **不许**用 \`laneTypeOf\`（两条轨会共用 key ⇒ Vue 复用错节点）；`
    + `实得 ${badKey.length} 处：${badKey.map((l) => l.trim()).join(' | ')}`)
  ok(keyLines.some((l) => /laneKey/.test(l)),
    '③ `v-for` 的 `:key` 必须用 `laneKey`（两条轨要不同的 key）')

  // `emit('lane-open')` 不许用 laneKey
  const emitLines = code.split('\n').filter((l) => /lane-open/.test(l) && /emit\(/.test(l))
  const badEmit = emitLines.filter((l) => /laneKey/.test(l))
  ok(badEmit.length === 0,
    '③⚠️ `emit(\'lane-open\')` **不许**发 `laneKey` —— `PolishView::laneGroupEls` 按'
    + '**轨道名**索引，发界面 id 会查不到 ⇒ **点轨道名没反应**（静默）')
  ok(emitLines.some((l) => /laneTypeOf/.test(l)),
    '③ `emit(\'lane-open\')` 必须发 `laneTypeOf`（= 轨道名）')
}

// ---------------------------------------------------------------------------
// ④ 两个 ＋ 各自直接建对应 kind（不再有"二选一菜单"）
// ---------------------------------------------------------------------------
ok(/if \(laneType === 'text'\) \{ addElementAt\('text'\); return \}/.test(code),
  '④ 文字轨的 ＋ 必须直接 `addElementAt(\'text\')`')
ok(/if \(laneType === 'sticker'\) \{ addElementAt\('sticker'\); return \}/.test(code),
  '④ 贴纸轨的 ＋ 必须直接 `addElementAt(\'sticker\')`')
ok(!/elementMenu|openElementMenu/.test(code),
  '④⚠️ 那个「文字 / 贴纸」二选一菜单**不许回来** —— 轨道已经分开，再问一遍是把已知的事重问'
  + '（删掉的症状是"在文字轨点＋却被问要不要贴纸"）')

// ---------------------------------------------------------------------------
// ⑤ ⚠️ 反例：认不出的 kind 不许默认成 `text`
// ---------------------------------------------------------------------------
{
  const i = code.indexOf('function elementKindOf(')
  ok(i >= 0, '⑤ 找得到 `elementKindOf`（结构变了就要改本判据）')
  if (i >= 0) {
    const body = code.slice(i, code.indexOf('\n}', i))
    ok(/'unknown'/.test(body), '⑤ `elementKindOf` 必须有 `\'unknown\'` 返回值')
    ok(!/return\s+'text'\s*$/.test(body.trim()) && /k === 'text'/.test(body),
      '⑤ `elementKindOf` 的 `text` 必须是**判出来的**（`k === \'text\'`），'
      + '不是兜底 —— 兜底会把认不出的 kind **伪装成文字**')
  }
  // 面板里三个分支各认各的（不许用 v-else 兜住 unknown）
  ok(/elementKindOf\(selItem\) === 'unknown'/.test(code),
    '⑤ 面板必须有 `unknown` 那一支（照实显示，让用户看见坏数据）')
  ok(/elementKindOf\(selItem\) === 'text'/.test(code) && /elementKindOf\(selItem\) === 'sticker'/.test(code),
    '⑤ 文字支与贴纸支必须**各自显式认领**（`=== \'text\'` / `=== \'sticker\'`），'
    + '不许用 `v-else` 兜 —— 那会把 `unknown` 当成贴纸')
}

// ---------------------------------------------------------------------------


// ---------------------------------------------------------------------------
// ⑥ ⚠️⚠️ **界面上不许再出现「元素」这个类别名**（2026-10-15 用户口径）
//
// 用户原话：「**因为元素(原文字+贴纸)已分离，请查找元素相关文案进行修改**」。
//
// 轨道分家之后，"元素"作为一个**类别**在界面上已经不存在了 —— 用户只看得到
// 「文字」与「贴纸」两类。而属性面板顶部那行「类型」此前显示的是词汇表的
// `element.label` = **「元素（文字 / 贴纸）」** ⇒ 把已经分开的两样东西**又并回一个名字**
// 摆在用户面前（他刚要求分开的就是这个）。
//
// ⚠️ 判据只查**用户看得见的文案**：模板插值（`{{ }}`）与 `label`/`title`/placeholder 的值。
//    注释不查 —— 注释里说"元素轨"是在解释代码概念，改了反而失真。
// ⚠️ 词表 `spec/effects.json` 的 `element.label` **不改**：它喂的是 `docs/` 的生成物
//    （`polish-events-reference.md` 等），属于**数据面**的名字；这里管的是**界面**口径。
//
// ⚠️⚠️ **ASCII 的 `element` 同样是「元素相关文案」**（2026-10-15 收尾）：上一批只查了「元素」
//    二字，而属性面板「类型」行那枚回显写的是 `element · {{ kind }}` —— 判据漏了，
//    用户照旧看见 **「element · sticker」/「element · text」**（他报的就是这两个字符串）。
//    ②b 补上这条：它查 ASCII `element`，与 ② 的 `/元素/` **正交**。
// ---------------------------------------------------------------------------
{
  // ① 「类型」行：element 必须按真实 kind 显示文字 / 贴纸。
  //    ⚠️⚠️ **判据要取"那一行"，不是"全文某处出现过这个模式"** —— 本轮自证抓到：
  //       把 `TYPE_LABELS[...]` 拼在那个三元表达式**前面**（`… || (elementKindOf(…) === 'text' ? …`）
  //       时，模式照样命中、判据照样绿，而用户看到的是「元素（文字 / 贴纸）」。
  //       症状正是**判据被"模式存在"满足、而不是被"行为正确"满足**。
  // ⚠️ 那个 `<b>` **跨好几行**（三元表达式换行写），所以要**从 `<b>` 拼到 `</b>`** ——
  //    只看单行的话 `'贴纸'` 在下一行、判据会假红。
  const bStart = code.indexOf('<b>{{ elementKindOf')
  const elRow = bStart >= 0 ? code.slice(bStart, code.indexOf('</b>', bStart)) : ''
  ok(!!elRow, '⑥ 找得到「类型」行里那个按 kind 显示的元素行（结构变了要改本判据）')
  ok(!!elRow && !/TYPE_LABELS/.test(elRow),
    '⑥ 「类型」行对 element **不许**引 `TYPE_LABELS`（那句是给文档/数据面的合并名；'
    + '拼进来用户就会看到「元素（文字 / 贴纸）」）')
  ok(!!elRow && /'文字'/.test(elRow) && /'贴纸'/.test(elRow),
    '⑥ 「类型」行对 element 必须按真实 kind 显示「文字」/「贴纸」')

  // ② **模板里的可见文案**不许含「元素」二字。
  //    ⚠️ 取**剥过注释的**模板段：本文件里解释"为什么不能用元素这个词"的注释
  //       本身就大量含「元素」，不剥就是**判据被自己的说明顶翻**。
  const tplEnd = src.indexOf('<script')
  const tpl = stripComments(tplEnd > 0 ? src.slice(0, tplEnd) : src)
  const visible = []
  // ⚠️ 插值里取**字符串字面量**，不是整条表达式：`{{ elementKindOf(selItem) === 'text' ? '文字' }}`
  //    里的 `elementKindOf` 是**代码**、不是给用户看的字 —— 整条拿去测会**假红**。
  for (const m of tpl.matchAll(/\{\{([^}]*)\}\}/g)) {
    for (const s of m[1].matchAll(/['"]([^'"]*)['"]/g)) visible.push(s[1])
  }
  for (const m of tpl.matchAll(/<label>([^<]*)<\/label>/g)) visible.push(m[1])
  for (const m of tpl.matchAll(/placeholder="([^"]*)"/g)) visible.push(m[1])
  for (const m of tpl.matchAll(/title="([^"]*)"/g)) visible.push(m[1])
  const badVisible = visible.filter((s) => /元素/.test(s))
  ok(badVisible.length === 0,
    `⑥ 模板里用户看得见的文案不许出现「元素」（已分离，用户只看得到文字 / 贴纸）；`
    + `实得 ${badVisible.length} 处：${badVisible.map((s) => s.trim().slice(0, 40)).join(' | ')}`)

  // ②b ⚠️⚠️ **「类型」行的回显里不许再出现 `element` 这个载体名**（2026-10-15 收尾）
  //
  // 上一批只改了 `<b>`（按 kind 显示「文字」/「贴纸」），紧跟其后那枚 mono 回显
  // 还写着 `element · {{ selItem.kind }}` —— 用户看到的正是 **「element · sticker」/
  // 「element · text」**：刚要求拆开的两样东西**又被一个合并名捆在一起**摆回他面前。
  //
  // ⚠️ `element` 是**磁盘上的载体名**（两条泳道共用 `type: "element"`，Rust `Lane::of_event`
  //    也把它映到 `Lane::Sticker`），属于**数据面** —— 界面上一个都不该露。
  // ⚠️ 它是 ASCII 词，**不在上面 ② 的 `/元素/` 判据里** ⇒ 单独判一条；
  //    不补的话这类回显可以一路漏下去（上一批就是这么漏的）。
  // ⚠️ 只查**用户看得见的文案**：插值里的字符串字面量 + 去掉标签/插值之后的**静态文本节点**
  //    （`element · ` 正是后者）。`elementKindOf(...)` 是**代码**、不是文案，不许因此假红。
  const typeRoBlock = (s) => {
    const i = s.indexOf('class="type-ro"')
    if (i < 0) return ''
    const j = s.indexOf('</div>', i)
    return j < 0 ? s.slice(i) : s.slice(i, j)
  }
  const visibleOf = (s) => {
    const out = []
    for (const m of s.matchAll(/\{\{([^}]*)\}\}/g)) {
      for (const t of m[1].matchAll(/['"]([^'"]*)['"]/g)) out.push(t[1])
    }
    out.push(...s.replace(/\{\{[\s\S]*?\}\}/g, ' ').replace(/<[^>]*>/g, ' ').split(/\s+/))
    return out.filter(Boolean)
  }
  const roBlock = typeRoBlock(tpl)
  ok(!!roBlock, '⑥ 找得到「类型」行那个 `type-ro` 块（结构变了要改本判据）')
  const roTexts = roBlock ? visibleOf(roBlock) : []
  const badRo = roTexts.filter((s) => /element/i.test(s))
  ok(badRo.length === 0,
    '⑥⚠️ 「类型」行的**可见文案**不许出现磁盘载体名 `element`（用户看到的会是'
    + '「element · sticker」/「element · text」，等于把刚分开的两样又捆成一个名字）；'
    + `实得 ${badRo.length} 处：${badRo.map((s) => s.trim().slice(0, 40)).join(' | ')}`)
  ok(roTexts.length > 0, '⑥ [自证] 「类型」行取文案的算术必须抽得到内容（否则这条判据恒过）')
  // [自证] 把 `element · ` 前缀塞回去，上面那条必须翻面
  // （⚠️ 锚点写错时 `replace` 是**静默 no-op**，那时"自证"变成恒过判据）
  const mutRo = tpl.replace('class="muted small mono">{{ selItem.kind',
    'class="muted small mono">element · {{ selItem.kind')
  ok(mutRo !== tpl, '⑥ [自证] 变异真的改到了「类型」行（锚点还在吗）')
  const mutRoTexts = mutRo !== tpl ? visibleOf(typeRoBlock(mutRo)) : []
  ok(mutRoTexts.some((s) => /element/i.test(s)),
    '⑥ [自证] 把 `element · ` 塞回「类型」行后判据必须翻面（否则它抓不到任何东西）')

  // ③ [自证] 取文案的算术必须真的抽得到东西 —— 抽不到就是恒真判据。
  ok(visible.length > 0, '⑥ [自证] 取文案的算术必须抽得到内容（否则这条判据恒过）')
  // ④ [自证] 塞一句带「元素」的可见 label，上面那条必须翻面。
  //    ⚠️ 锚点用模板里**确实存在**的 label（写错锚点时 `replace` 是静默 no-op，
  //       自证就变成"恒过" —— 本轮先用了 `<label>位置</label>`，而它早已改成 `pos-lbl`）。
  const mutatedTpl = tpl.replace(/<label>字幕内容<\/label>/, '<label>元素内容</label>')
  ok(mutatedTpl !== tpl, '⑥ [自证] 变异真的改到了模板（锚点还在吗）')
  const mutVisible = [...mutatedTpl.matchAll(/<label>([^<]*)<\/label>/g)].map((m) => m[1])
  ok(mutVisible.some((s) => /元素/.test(s)),
    '⑥ [自证] 塞一句「元素位置」后判据必须翻面（否则它抓不到任何东西）')
}
console.log(`编辑器「文字 / 贴纸」分家：${pass} 项通过，${fail} 项失败`)
if (fail) {
  console.log()
  for (const f of failures) console.log('  ✗ ' + f)
  process.exit(1)
}
console.log('✓ 界面上两条泳道、磁盘上仍同一条轨；`:key` 与 `emit` 各用各的键；认不出的 kind 照实报')
