#!/usr/bin/env node
// 文字元素的**颜色预设**：四方表一致性 + 端到端取值（2026-10-13）。
//
// # 为什么需要这条门禁
//
// 颜色这条链是"**一个事实、四处实现**"：
//   ① `placement.rs::ElementColor`（真源：名字 ↔ RGBA）
//   ② `llm/segment.rs` 的提示词（LLM 照它写）
//   ③ `tools/polish-to-dhampir.mjs`（读 `process` 写回的第 10 段）
//   ④ `webui/src/polish/engine.js`（预览同上）
//
// 漏改任何一处的症状都是本仓最恨的那种 —— **配了、不报错、没效果**：
//   * 提示词多一档 ⇒ LLM 照着写、`process` 报"认不出"并静默退回默认色；
//   * 提示词少一档 ⇒ LLM 永远不写它 ⇒ **产出恒为 0**（§18-A 那个形状）；
//   * ③④ 的段数判据没跟上 ⇒ 10 段的串**整条退回兜底样式**（字号/颜色全变，报告只有一行 note）。
//
// ⚠️ 判据**从 Rust 源码里读出真源**再逐个比对，**不在本文件里写第二份颜色表** ——
//    否则"四方一起漂"照样绿（判据空转）。这与 `check-text-style.mjs` ⑦ 同一手法。
import fs from 'node:fs'
import { execFileSync } from 'node:child_process'

const RUST = 'crates/yekitrim-polish/src/process/placement.rs'
const PROMPT = 'crates/yekitrim-polish/src/llm/segment.rs'
const TRANS = 'tools/polish-to-dhampir.mjs'
const ENGINE = 'webui/src/polish/engine.js'

const fails = []
function check(name, ok, detail = '') {
  if (ok) console.log(`  ok   ${name}`)
  else {
    console.log(`  FAIL ${name}${detail ? `\n         ${detail}` : ''}`)
    fails.push(name)
  }
}

const rust = fs.readFileSync(RUST, 'utf8')

// ① 从 Rust 的 `fn name()` 里读出**档名清单**（顺序即展示顺序）。
const nameBody = /pub const fn name\(self\) -> &'static str \{([\s\S]*?)\n    \}/.exec(rust)
check('Rust 侧找得到 ElementColor::name()', nameBody !== null,
  'placement.rs 里找不到 `pub const fn name(self) -> &\'static str`')
const names = nameBody
  ? [...nameBody[1].matchAll(/=>\s*"([a-z]+)"/g)].map((m) => m[1])
  : []
check('Rust 侧解析出 7 档颜色', names.length === 7, `实得 {${names.join(', ')}}`)

// ② 从 Rust 的 `fn rgba()` 里读出每一档的 RGBA。
const rgbaBody = /pub const fn rgba\(self\) -> \[u8; 4\] \{([\s\S]*?)\n    \}/.exec(rust)
check('Rust 侧找得到 ElementColor::rgba()', rgbaBody !== null)
const rgbaMap = {}
if (rgbaBody) {
  for (const m of rgbaBody[1].matchAll(/Self::(\w+)\s*=>\s*\[(\d+),\s*(\d+),\s*(\d+),\s*(\d+)\]/g)) {
    rgbaMap[m[1].toLowerCase()] = `${m[2]},${m[3]},${m[4]},${m[5]}`
  }
}
check('Rust 侧 7 档都有 RGBA', Object.keys(rgbaMap).length === 7,
  `实得 ${Object.keys(rgbaMap).length} 档：${JSON.stringify(rgbaMap)}`)

// ③ **`ALL` 必须与 `name()` 同集合**（手工维护两张清单，加一档只改一处 ⇒
//    那一档能进提示词、却永远解析不出来）。
const allBody = /pub const ALL: \[Self; 7\] = \[([\s\S]*?)\n    \];/.exec(rust)
const allNames = allBody
  ? [...allBody[1].matchAll(/Self::(\w+)/g)].map((m) => m[1].toLowerCase())
  : []
check('Rust 的 ALL 与 name() 同集合（顺序也一致）',
  allNames.length === names.length && allNames.every((n, i) => n === names[i]),
  `ALL = {${allNames.join(', ')}}，name() = {${names.join(', ')}}`)

// ④ `parse` 必须认得出**每一档**（与 ALL 同集合）。
const parseBody = /pub fn parse\(raw: &str\) -> Option<Self> \{([\s\S]*?)\n    \}/.exec(rust)
const parseNames = parseBody
  ? [...parseBody[1].matchAll(/"([a-z]+)"\s*=>\s*Some\(Self::(\w+)\)/g)].map((m) => m[1])
  : []
check('Rust 的 parse 认得出 ALL 里每一档',
  names.every((n) => parseNames.includes(n)),
  `parse 认 {${parseNames.join(', ')}}，ALL 有 {${names.join(', ')}} —— ` +
  '差的那几档 LLM 照着提示词写也会被判「认不出」并静默退回默认色')

// ⑤ 提示词里必须**教全**这七档（少一档 ⇒ LLM 产出恒为 0）。
const prompt = fs.readFileSync(PROMPT, 'utf8')
const promptBlock = prompt
  .split('\n')
  .slice(prompt.split('\n').findIndex((l) => l.trimStart().startsWith('color ')))
  .slice(1)
  .slice(0, prompt.split('\n').slice(prompt.split('\n').findIndex((l) => l.trimStart().startsWith('color ')) + 1)
    .findIndex((l) => l.trimStart().startsWith('⚠')))
const promptNames = [...promptBlock.join('\n').matchAll(/^\s*\* `([a-z]+)`/gm)].map((m) => m[1])
check('提示词教全了七档颜色（缺一档 ⇒ LLM 永远写不出它）',
  names.every((n) => promptNames.includes(n)) && promptNames.length === names.length,
  `提示词教 {${promptNames.join(', ')}}，真源有 {${names.join(', ')}}`)

// ⑥ 转译器与预览的**段数判据**必须收得下新的第 10 段。
//
// ⚠️ 这条是"三方同值"里最容易被漏的一面：`process` 写了 10 段，
//    而消费侧只认 {7, 9} ⇒ **整条退回兜底样式**（字号/颜色/歪压全变，
//    报告里只有一行 note，而两边单测各自都过）。
const trans = fs.readFileSync(TRANS, 'utf8')
// ⚠️ **必须定位到 `parseTextStyle` 函数体内**再找那条判据 ——
//    裸匹配 `parts.length !== N` 会先撞上文件里别处的同名判据
//    （实测撞到第 2 段那一条，于是门禁报"缺 10"而真因是我的正则范围太宽）。
//    同一条教训见 `check-text-style.mjs` ④ 的注释。
const transFnStart = trans.indexOf('function parseTextStyle(')
const transFnEnd = transFnStart < 0 ? -1 : trans.indexOf('\n  }', transFnStart)
const transFn = transFnStart < 0 ? '' : trans.slice(transFnStart, transFnEnd < 0 ? trans.length : transFnEnd)
const transAccept = /if \(parts\.length !== ([\d]+(?: && parts\.length !== [\d]+)*)\)/.exec(transFn)
check('转译器接受 10 段（颜色那一段）',
  transAccept !== null && transAccept[1].includes('10'),
  transAccept === null
    ? 'parseTextStyle 里找不到段数判据'
    : `parseTextStyle 接受 {${transAccept[1].replace(/parts\.length !== /g, '')}}，缺 10 ⇒ 新工程整条退回兜底样式`)

// ⚠️ tilt/squash 的取值判据必须是 `>= 9`，**不是 `=== 9`** ——
//    写成相等时 10 段的串会**静默丢掉 tilt/squash**（字不歪不压、报告零差异）。
check('转译器的 tilt/squash 取值判据是 `>= 9`（不是 `=== 9`）',
  /parts\.length >= 9 \? Number\.parseFloat\(parts\[7\]\)/.test(trans),
  '转译器里 `parts.length === 9 ? … parts[7]` —— 10 段串会静默丢掉 tilt/squash')

const engine = fs.readFileSync(ENGINE, 'utf8')
check('预览 engine 接受 10 段（下界判据已含）',
  /if \(p\.length < 7\) return null/.test(engine),
  'engine.js 的 parseTextSource 段数判据变了 —— 确认它收得下 10 段')

// ⑦ **端到端取值**：把 7 档各过一个真实 `process` 写回，验第 10 段与 RGBA。
//
// ⚠️ 只看"字段在不在"是不够的（本仓栽过：颜色写进去了、但用的是档位默认色）——
//    这里必须**逐档比对 RGBA**。Rust 侧单测已经覆盖了这条，
//    但那是"Rust 内部自洽"；这里验的是**跨语言**：JS 读出来的与我们期望的一致。
check('四处（Rust/提示词/转译器/预览）都提到了 colorName/p[9]',
  trans.includes('colorName: parts.length >= 10') && engine.includes("String(p[9] ?? '')"),
  '转译器或预览没有带出第 10 段 —— 产物里就查不出"这条字选的是哪一档"')

// ⑧ **面板下拉**也必须与真源同集合。
//
// ⚠️ 漏这里的症状最隐蔽：LLM 写的颜色能出片（走 `process`），
//    而**用户自己在面板上选不到**（下拉里没有那一档）——
//    或者反过来：面板多列一档、后端 `parse` 不认 ⇒ 选了等于没选、且只有一行 warn。
const editor = fs.readFileSync('webui/src/components/TimelineEditor.vue', 'utf8')
const menu = /const ELEMENT_COLORS = \[([\s\S]*?)\n\]/.exec(editor)
const menuNames = menu ? [...menu[1].matchAll(/value:\s*'([a-z]+)'/g)].map((m) => m[1]) : []
check('面板颜色下拉与真源同集合（且顺序一致）',
  menuNames.length === names.length && menuNames.every((n, i) => n === names[i]),
  menu === null
    ? 'TimelineEditor.vue 里找不到 `ELEMENT_COLORS` —— 面板没有颜色入口'
    : `面板 = {${menuNames.join(', ')}}，真源 = {${names.join(', ')}}`)
check('面板的颜色下拉渲染出来了（模板里有 v-for + 一个空值项）',
  /v-for="c in ELEMENT_COLORS"/.test(editor) && /<option value="">默认色<\/option>/.test(editor),
  '模板里没有颜色下拉（Vue 对未定义标识符**不报错**，会静默渲染成空白）')

// ⑨ `RGBA` 只许有一份真源：前端**不许**自己写颜色表（它只写名字）。
//
// ⚠️ 前端再存一份 RGBA ⇒ `black` 那条"描边成对翻白"必然漏掉
//    （预览糊成一团、成片正常，两边都不报错）。
const editorHasRgba = /\b(24,\s*24,\s*24|255,\s*128,\s*192)\b/.test(editor)
check('面板不自己存 RGBA（只写颜色名）', !editorHasRgba,
  'TimelineEditor.vue 里出现了颜色 RGBA 字面量 —— 前端不该参与颜色换算（第二份真源）')

// ⑩ **文字轨的文件名是三处同源的一份契约**（2026-10-13）。
//
// ⚠️ 这条来自一个**真缺陷**：服务端原先只读两个**写死的名字**在 `--out` 旁边找，
//    而文字元素的 ASS 写在**片段根目录**（`dhampir-<轨 id>.ass`）——
//    一条都没读过 ⇒ **Dhampir 预览里文字元素一个字都不显示**，
//    而成片正常（`dhampir.exe` 按绝对 `asset.uri` 自己找文件）、报告全绿。
//
// 三方必须对得上同一个串：
//   ① 转译器**写**文件用的 `dhampir-${id}.ass`
//   ② doc 里 `assets[].uri` 的 basename（服务端按它建表）
//   ③ 前端按 `uri` basename 查表
//
// ⚠️ 2026-10-18（「渲染导出时中间文件过多」）之后，文件**不再写片段根目录**，而是落
//    `<工程文件名去扩展名>.assets/`（与 `dhampir.rs::cleanup_project` 同一推导）。
//    所以这里多钉一条：**两条推导必须成对存在** —— 只钉 basename 的话，把 `sideDir`
//    改回片段根目录这条变异不会红（成片照样对，只是又开始堆垃圾）。
const SIDE_DIR = /const sideDir = join\(dirname\(docPath\),/.exec(trans)
check('转译器给文字轨 ASS 一个**独占子目录**（`<工程文件名>.assets/`，与 Rust 清理同一推导）',
  SIDE_DIR !== null,
  '转译器里找不到 `const sideDir = join(dirname(docPath), …)` —— 排他目录没了，' +
  '文字轨 ASS 会退回片段根目录/与并发转译共用目录（前者=文件越积越多，后者=互相删）')
const TRANS_ASS = /const assPath = join\(sideDir, `dhampir-\$\{id\}\.ass`\)/.exec(trans)
check('转译器把文字轨写成 `dhampir-<轨 id>.ass`（前缀契约）', TRANS_ASS !== null,
  '转译器里找不到 `join(sideDir, \\`dhampir-${id}.ass\\`)` —— 改了这个形状，' +
  '服务端/前端按 basename 建表与查表就会对不上（预览里那类文字静默消失）')

const PREVIEW = 'webui/src/components/PolishPreview.vue'
const preview = fs.readFileSync(PREVIEW, 'utf8')
check('前端按 `uri` 的 basename 查表（与服务端建表口径一致）',
  /String\(a\.uri \|\| ''\)\.split\(\/\[\\\\\/\]\/\)\.pop\(\)/.test(preview),
  'PolishPreview.vue 不再按 basename 查表 —— 服务端那份 map 的键也必须跟着改，' +
  '否则两处口径分叉（症状：字幕/文字全都喂不进去，而两边各自看都"对"）')

const POLISH_RS = 'crates/yekitrim-server/src/api/polish.rs'
const polishRs = fs.readFileSync(POLISH_RS, 'utf8')
// ⚠️⚠️ **必须只看 `read_side_subtitles` 的函数体**。
//
//    `.rsplit(['/', '\\'])` 在文件里出现多次（**`mod tests` 里也有** ——
//    新加的用例为了造夹具会写同样的表达式）。裸查全文的话，
//    "把实现里的 basename 取值改掉"这条变异**打不红**：测试那几处替它兜住了。
//    **实测踩到过**：MC3 变异打不改三次都不红，原因就是这个。
//    （与上面 `trans`/`engine` 那几处"正则误命中无关处"是同一类坑。）
const sideSubsBody = (() => {
  const start = polishRs.indexOf('fn read_side_subtitles(')
  if (start < 0) return ''
  const rest = polishRs.slice(start)
  const end = rest.indexOf('\n}\n')
  return end < 0 ? rest : rest.slice(0, end)
})()
// ⚠️ 锚点**逐字节**对齐：源码里是 `.rsplit(['/', '\\'])`。
//    反斜杠层数写错**不报错**、只让判据永远绿，所以下面拆成两半：
//    **先断言函数体真的取到了**，再拿它判断 —— 不让"锚点写错"伪装成"实现是对的"。
const BASENAME_ANCHOR = `.rsplit(['/', '${'\\'.repeat(2)}'])`
check('取到了 `read_side_subtitles` 的函数体（判据的作用域没落空）',
  sideSubsBody.length > 0,
  '找不到 `fn read_side_subtitles(` 或它的函数体 —— 判据会在空串上恒真/恒假，' +
  '那比没有判据更糟（它会让人以为这里被守着）')
check('服务端在 `read_side_subtitles` 里按 `uri` 的 basename 建表（与前端同口径）',
  sideSubsBody.includes(BASENAME_ANCHOR),
  `函数体里找不到 \`${BASENAME_ANCHOR}\` —— 与前端 \`split(/[\\\\/]/).pop()\` 分叉，` +
  '查表全部落空（预览里没有字幕，且不报错）。' +
  '⚠️ 只查全文是不够的：`mod tests` 里也有同样的表达式，会替实现兜住')

console.log(`\n${fails.length ? `${fails.length} 条不通过` : '全部通过'}`)
process.exit(fails.length ? 1 : 0)
