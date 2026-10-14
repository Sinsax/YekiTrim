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

// 剥注释（`//` 与 `/* */`）—— 本文件里解释性注释大量提到这些符号，
// 不剥的话判据会被**它自己让人写下的说明**顶翻。⚠️ `(?<!:)` 避开 `https://`。
const stripComments = (s) => s
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
// ①b **两枚眼睛管同一个开关** —— 只有贴纸轨许有眼睛
// ---------------------------------------------------------------------------
//
//  `文字` 与 `贴纸` 在磁盘上是**同一条轨**（都 `lane: 'sticker'`）。
//  两条都画眼睛 = 两枚按钮管同一个 `[stage.lanes].sticker` 开关 ——
//  点哪一枚另一枚都跟着变，用户看到的是"我只关了文字，贴纸怎么也没了"。
//  ⇒ 眼睛留在贴纸轨，文字轨放**不可点的占位**（`sharedEye`）。
{
  ok(byId.text && byId.text.sharedEye === true,
    '①b 文字轨必须标 `sharedEye: true`（它的眼睛与贴纸轨是同一个开关，'
    + '不能画成第二枚可点的眼睛）')
  ok(byId.sticker && !byId.sticker.sharedEye,
    '①b 贴纸轨**不许**标 `sharedEye` —— 眼睛留在它身上（开关名就是 `sticker`）')
  ok(/v-if="lane\.lane && !lane\.sharedEye"/.test(code),
    '①b 眼睛的 `v-if` 必须排除 `sharedEye` 那条（否则两枚眼睛管同一个开关）')
  ok(/v-else-if="lane\.sharedEye"/.test(code),
    '①b `sharedEye` 那条要有自己的占位分支（让用户知道为什么这里没有眼睛）')
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

console.log(`编辑器「文字 / 贴纸」分家：${pass} 项通过，${fail} 项失败`)
if (fail) {
  console.log()
  for (const f of failures) console.log('  ✗ ' + f)
  process.exit(1)
}
console.log('✓ 界面上两条泳道、磁盘上仍同一条轨；`:key` 与 `emit` 各用各的键；认不出的 kind 照实报')
