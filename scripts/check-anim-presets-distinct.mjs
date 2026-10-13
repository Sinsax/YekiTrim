#!/usr/bin/env node
// **档位之间必须真的不同**（2026-10-13）。
//
// # 为什么需要这条
//
// 实测抓到的真缺陷：`vtext_thin`（面板写「细楷」）与 `vtext_bold`（面板写「粗体」）
// 在 `ANIM_PRESETS` 里**逐字节相同**（两个都只有 `vertical` + `safe_width_ratio`）。
// 症状：用户在面板上选「粗体」，成片与「细楷」**一模一样** ——
// **全链路零报错、零可见差异**，而分类下拉里那两档看起来是两个功能。
//
// ⚠️ 这类缺陷**任何"字段在不在 / 链路通不通"的判据都抓不到**：
//    `vertical` 确实发出去了、`safe_width_ratio` 确实生效了、
//    轨道 id 确实不同（`text_hook_center_vtext_thin` vs `_vtext_bold`）——
//    每一条判据都是绿的，只有**把两档的产物对拍**才看得见。
//    ⇒ 判据必须是"**同一 kind 的两个档不许产出同一份参数**"。
//
// ⚠️ 判据**从源码里读真源**（`ANIM_PRESETS`），不写第二份档位表 ——
//    否则"两处一起漂"照样绿（与 `check-element-color.mjs` 同一手法）。
import fs from 'node:fs'

const TRANS = 'tools/polish-to-dhampir.mjs'
const EDITOR = 'webui/src/components/TimelineEditor.vue'

const fails = []
function check(name, ok, detail = '') {
  if (ok) console.log(`  ok   ${name}`)
  else {
    console.log(`  FAIL ${name}${detail ? `\n         ${detail}` : ''}`)
    fails.push(name)
  }
}

const src = fs.readFileSync(TRANS, 'utf8')

// 抠出 `const ANIM_PRESETS = { … };` 那一块。
const start = src.indexOf('const ANIM_PRESETS = {')
check('找得到 ANIM_PRESETS 真源', start >= 0,
  'polish-to-dhampir.mjs 里找不到 `const ANIM_PRESETS = {`')
const lines = src.slice(start).split('\n')
let endIdx = -1
for (let i = 1; i < lines.length; i++) {
  if (lines[i].replace(/\s+$/, '') === '  };') { endIdx = i; break }
}
check('找得到 ANIM_PRESETS 的收尾', endIdx > 0, '没找到缩进 2 空格的 `};`')
const block = lines.slice(0, endIdx < 0 ? lines.length : endIdx + 1).join('\n')

// 每个档：`    name: { … },`（顶层一项，值里可能有嵌套对象 —— 用括号配平取）。
//
// ⚠️ **必须在整块上做括号配平、不能逐个正则**：值里可以有嵌套对象
//    （`perChar: { … }`），而行内正则会在内层的 `}` 处提前收尾 ——
//    第一版就是这么写的，于是 39 档只解析出 18 个（嵌套的那批全丢了）。
//    ⇒ 先扫出所有**顶层**的 `    name: {` 起点，再各自向后配平。
const presets = {}
{
  const starts = []
  let depth = 0
  for (let i = 0; i < block.length; i++) {
    const c = block[i]
    if (c === '{') {
      depth++
      if (depth === 2) {
        // 进入**某项的值**：往回找这一项的 `name:`。
        const head = block.slice(Math.max(0, i - 60), i)
        const m = /(\w+):\s*$/.exec(head)
        if (m) starts.push([m[1], i + 1])
      }
    } else if (c === '}') {
      depth--
      if (depth === 1 && starts.length) {
        const [name, from] = starts[starts.length - 1]
        if (!(name in presets)) presets[name] = normalize(block.slice(from, i))
        starts.pop()
      }
    }
  }
}
function normalize(body) {
  // 去掉注释行、压掉空白 ⇒ 只比"真的会进配置的东西"。
  return body
    .split('\n')
    .map((l) => l.replace(/\/\/.*$/, '').trim())
    .filter(Boolean)
    .join(' ')
    .replace(/\s+/g, ' ')
}
check('解析出 ≥ 30 个档位', Object.keys(presets).length >= 30,
  `只解析出 ${Object.keys(presets).length} 个：${Object.keys(presets).join(', ')}`)

// ---- ① 同一 kind 内不许有两档参数完全相同 ----
//
// ⚠️ 只比**同一 kind**（`vtext_*` / `tilt_*` / …）：跨 kind 相同的两档
//    是**有意**的（比如 `fade_only` 与某档都只要一个字段），而且它们在
//    面板上分属两个不同的下拉，用户不会误以为"选了 A 得到 B"。
const groups = {}
for (const k of Object.keys(presets)) {
  const kind = k.replace(/_[^_]+$/, '')
  ;(groups[kind] ||= []).push(k)
}
const dupes = []
for (const [kind, ks] of Object.entries(groups)) {
  for (let i = 0; i < ks.length; i++) {
    for (let j = i + 1; j < ks.length; j++) {
      if (presets[ks[i]] === presets[ks[j]]) dupes.push([kind, ks[i], ks[j]])
    }
  }
}
check('同一类别内没有两档参数逐字节相同（有 = 选 A 得到 B）',
  dupes.length === 0,
  dupes.map(([kind, a, b]) =>
    `[${kind}] \`${a}\` 与 \`${b}\` 的参数完全相同 —— 面板上是两个功能、产物一模一样`).join('\n         '))

// ---- ② 具体钉住 `vtext_thin` / `vtext_bold`（实测抓到的那一对）----
//
// ⚠️ 上面 ① 是通用判据，这条是**回归钉子**：它明确说出"这两档必须靠
//    `fontWeight` 区分"，否则将来有人"化简"掉那个字段时，
//    ① 仍会红但报的是"两档相同"，读的人未必知道该补什么。
check('`vtext_thin` 与 `vtext_bold` 靠 `fontWeight` 区分',
  /fontWeight/.test(presets.vtext_thin || '') && /fontWeight/.test(presets.vtext_bold || ''),
  '`vtext_thin`（面板「细楷」）与 `vtext_bold`（「粗体」）必须各带一个 `fontWeight` —— ' +
  '没有它两档产物逐字节相同（实测抓到的真缺陷：选「粗体」与「细楷」一模一样）')
check('`vtext_thin` 的字重**小于** `vtext_bold`',
  Number(/fontWeight:\s*(\d+)/.exec(presets.vtext_thin || '')?.[1]) <
  Number(/fontWeight:\s*(\d+)/.exec(presets.vtext_bold || '')?.[1]),
  `thin=${/fontWeight:\s*(\d+)/.exec(presets.vtext_thin || '')?.[1]} ` +
  `bold=${/fontWeight:\s*(\d+)/.exec(presets.vtext_bold || '')?.[1]} —— ` +
  '写反了的话「细楷」比「粗体」还粗（而两档仍然"不同"，通用判据抓不到）')

// ---- ③ `fontWeight` 必须真的被写进产物 ----
//
// ⚠️ 档位表里写了、`motionFields` 没读 ⇒ 与前一个缺陷**同形**
//    （"字段在、链路没接"）。这一条是那个字段的接线判据。
check('`fontWeight` 真的被写进产物（`out.font_weight`）',
  /if \(stylePreset && stylePreset\.fontWeight !== undefined\) \{\s*\n\s*out\.font_weight = stylePreset\.fontWeight;/.test(src),
  '档位表里有 `fontWeight` 但 `motionFields` 没读它 —— 写了等于没写（实测同形缺陷已犯过多次）')

// ---- ④ 面板上这两档必须在**同一个下拉**里（否则"选 A 得到 B"不成立）----
const editor = fs.readFileSync(EDITOR, 'utf8')
const menu = /variants: \[([\s\S]*?)\]/.exec(
  editor.slice(editor.indexOf("kind: 'vtext'")) || '')
const menuNames = menu ? [...menu[1].matchAll(/value: '(\w+)'/g)].map((m) => m[1]) : []
check('面板 vtext 下拉与真源同集合',
  menuNames.length > 0 && menuNames.every((n) => n in presets) && menuNames.length === (groups.vtext || []).length,
  `面板 = {${menuNames.join(', ')}}，真源 = {${(groups.vtext || []).join(', ')}}`)

console.log(`\n${fails.length ? `${fails.length} 条不通过` : '全部通过'}`)
process.exit(fails.length ? 1 : 0)
