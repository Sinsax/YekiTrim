#!/usr/bin/env node
/**
 * 把 `webui/src/components/TimelineEditor.vue` 的 `ANIM_KINDS` **导出成快照**
 * `docs/anim/anim-kinds.json`（供 `gen-preview.mjs` 生成 HTML 对照页）。
 *
 * # ⚠️⚠️ 为什么必须有一个**生成器**，而不是"记得手动更新"
 *
 * 这个文件原来是**手抄**的 39 档快照。三轴收敛（2026-10-14）把档位从 39 收到 9、
 * 轴的集合也从 `style/effect/both/none` 变成 `style/transition/effect/none`，
 * **但快照没人改** ⇒ `docs/anim/` 那份 HTML 一直画着**已经不存在的 39 档**，
 * 而且没有任何判据会红（它只是个文档）。
 *
 * 这正是本仓第 ① 类老病「**值在那儿、第二份没跟上**」的文档版本。
 * ⇒ 快照不再手抄：**从真源码里抽**，改了 `ANIM_KINDS` 就重跑这个脚本。
 *
 * # 用法
 *
 *     node docs/anim/gen-kinds.mjs
 *
 * ⚠️ 抽取方式是**括号配平**（不是正则）：`ANIM_KINDS` 里既有嵌套对象、
 *    又有注释里的 `[` / `]`，正则跨块匹配会吃到别的东西（本仓踩过一次，
 *    `check-39-tiers.mjs` 的注释里记着）。
 */
import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath, pathToFileURL } from 'node:url'

const HERE = path.dirname(fileURLToPath(import.meta.url))
const ROOT = path.resolve(HERE, '..', '..')
const SRC = path.join(ROOT, 'webui/src/components/TimelineEditor.vue')
const OUT = path.join(HERE, 'anim-kinds.json')

// ⚠⚠ `ANIM_KINDS` **不带 `preset`**（它只管"下拉里有什么"）——
//    档内参数的权威是 `engine.js::ANIM_PRESETS`。
//    原来快照里的 `preset` 是**手抄**的（而且早就漂了：逐字那三档
//    的 `perChar` 根本不在里面 ⇒ 页面上逐字那一路**一直是空跑的**）。
//    ⇒ 从权威表**读**过来，不再手抄。
const PRESETS_MOD = await import(pathToFileURL(
  path.join(ROOT, 'webui/src/polish/engine.js')).href)
const ANIM_PRESETS = PRESETS_MOD.ANIM_PRESETS

const src = fs.readFileSync(SRC, 'utf8')
const decl = 'const ANIM_KINDS = ['
const at = src.indexOf(decl)
if (at < 0) {
  console.error(`✗ 在 ${SRC} 里找不到 \`${decl}\``)
  process.exit(1)
}

// ---- 括号配平（跳过字符串与注释）-----------------------------------------
const open = at + decl.length - 1
let depth = 0
let end = -1
let quote = null
let line = false
let block = false
for (let i = open; i < src.length; i += 1) {
  const c = src[i]
  const n = src[i + 1]
  if (line) { if (c === '\n') line = false; continue }
  if (block) { if (c === '*' && n === '/') { block = false; i += 1 } continue }
  if (quote) {
    if (c === '\\') { i += 1; continue }
    if (c === quote) quote = null
    continue
  }
  if (c === '/' && n === '/') { line = true; i += 1; continue }
  if (c === '/' && n === '*') { block = true; i += 1; continue }
  if (c === "'" || c === '"' || c === '`') { quote = c; continue }
  if (c === '[') depth += 1
  else if (c === ']') { depth -= 1; if (depth === 0) { end = i + 1; break } }
}
if (end < 0) {
  console.error('✗ ANIM_KINDS 的 `[` 没有配平的 `]` —— 源码结构变了，抽取方式要跟着改')
  process.exit(1)
}

// ---- 求值（不 eval 源码：用 Function 只为把它变成数据，内容仍是纯字面量）----
let KINDS
try {
  // eslint-disable-next-line no-new-func
  KINDS = new Function(`${src.slice(at, end)}; return ANIM_KINDS`)()
} catch (e) {
  console.error('✗ ANIM_KINDS 不再是纯字面量（含函数调用或变量引用）⇒ 无法导出快照')
  console.error(`  ${e.message}`)
  process.exit(1)
}

// ---- 摊平成快照（保持老格式：一条 variant 一条记录）-------------------------
const rows = []
for (const k of KINDS) {
  for (const v of k.variants) {
    rows.push({
      kind: k.kind,
      kindLabel: k.label,
      // ⚠️ `axis: 'none'` 那一项是**"不指定"**，不是一条轴 —— 原样带出去，
      //    由 `gen-preview.mjs` 决定怎么显示（三栏都有的意思）。
      axis: k.axis,
      value: v.value,
      label: v.label,
      // ⚠ 从 **ANIM_PRESETS**（权威）取，不是从 `ANIM_KINDS`（后者根本不带这个字段）。
      preset: ANIM_PRESETS[v.value] ?? {},
    })
  }
}

fs.writeFileSync(OUT, JSON.stringify(rows, null, 1) + '\n', 'utf8')

const tiers = rows.map((r) => r.value).filter(Boolean)
console.log(`✓ ${path.relative(ROOT, OUT)}：${rows.length} 条（${tiers.length} 档 + "不指定"）`)
for (const k of KINDS) {
  console.log(`   ${k.axis.padEnd(11)} ${k.kind.padEnd(12)} ${k.variants.map((v) => v.value || '(空)').join(' / ')}`)
}
