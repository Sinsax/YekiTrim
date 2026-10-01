// 断言：时间轴「拖拽松手」必须触发后处理（emit('done')）。
//
// 背景：2026-10-01 用户报「在时间轴编辑器修改事件时后处理没有正常运作」。
// 根因不是底座迁移（process 后端实测正常），而是 TimelineEditor 的拖拽 onUp
// 只 touch()（落盘）、从不 commitEdit()（emit done）—— 而拖拽是改事件时间最常见的方式。
//
// 这个脚本对**源码**与**构建产物**各查一遍：只查源码会被"改了没重建"骗过去。
import fs from 'node:fs'

const SRC = 'webui/src/components/TimelineEditor.vue'
const src = fs.readFileSync(SRC, 'utf8')

let pass = 0, fail = 0
function check(label, cond, detail) {
  if (cond) { pass++; console.log('  ✓ ' + label) }
  else { fail++; console.log('  ✗ ' + label + (detail ? '  ← ' + detail : '')) }
}

/** 去掉注释（判据要落在真代码上：解释里就写着反面例子）。 */
function decomment(t) {
  return t.replace(/\/\*[\s\S]*?\*\//g, '').split('\n')
    .map((l) => l.replace(/(^|[^:])\/\/.*$/, '$1')).join('\n')
}

const code = decomment(src)

console.log('── 源码：TimelineEditor.vue ──')

// ① 拖拽松手必须 commitEdit
const onUpBlock = code.match(/const onUp = \(evt\) => \{[\s\S]*?\n  \}/)
check('onUp 存在', !!onUpBlock)
check('onUp 里调用了 commitEdit', !!onUpBlock && /commitEdit\(/.test(onUpBlock[0]),
  onUpBlock ? 'onUp 内没有 commitEdit → 拖完不跑后处理' : '')

// ② 只在真的动过时才通知（纯点击不该触发后处理）
check('有 dragMoved 标记', /let dragMoved = false/.test(code))
check('move 里按位移置 dragMoved', /dragMoved = true/.test(code))
check('拖拽开始时重置 dragMoved', /drag = \{ move: apply \}\s*\n\s*dragMoved = false/.test(code))
check('onUp 按 dragMoved 门控 commitEdit', /if \(moved\) commitEdit\(/.test(code))

// ③ 三个拖拽模式都要标记位移
const modes = ['move(evt)', "'resize-l'(evt)", "'resize-r'(evt)"]
/** 取某个拖拽模式的**整个函数体**：从 `xxx(evt) {` 到它的收尾 `},`（缩进 4 空格）。
 *  ⚠️ 以前用固定字符窗口（260），`move` 最长、`markDirty` 落在窗口外 ⇒ 判据假红。 */
function bodyOf(sig) {
  const i = code.indexOf(sig)
  if (i < 0) return ''
  const end = code.indexOf('\n    },', i)
  return end < 0 ? code.slice(i, i + 800) : code.slice(i, end)
}

for (const m of modes) {
  check(`${m} 里标记了 dragMoved`, /dragMoved = true/.test(bodyOf(m)))
}

// ④ 三个拖拽模式都调用 markDirty（改动要落盘）
for (const m of modes) {
  check(`${m} 里 markDirty`, /markDirty\(dk\)/.test(bodyOf(m)))
}

// ⑤ 面板 change 仍然算「编辑完成」
check('面板 @change 走 onPanelCommit', /@change="onPanelCommit"/.test(src))
check('onPanelCommit 调 commitEdit', /function onPanelCommit\(\)[\s\S]{0,200}?commitEdit\(/.test(code))

// ⑥ commitEdit 的既有出口一个都不能少
for (const [label, re] of [
  ['取消选中', /commitEdit\('取消选中'\)/],
  ['离开页面', /commitEdit\('离开页面'\)/],
  ['切换片段', /commitEdit\('切换片段'/],
]) check(`commitEdit 出口保留：${label}`, re.test(code))

console.log('\n── 构建产物：dist ──')
const idx = fs.readFileSync('webui/dist/index.html', 'utf8')
const chunk = (idx.match(/assets\/(index-[A-Za-z0-9_-]+\.js)/) || [])[1]
check('dist/index.html 能定位入口', !!chunk)
const files = fs.readdirSync('webui/dist/assets').filter((f) => /^PolishView-.*\.js$/.test(f))
check('PolishView chunk 存在', files.length === 1, files.join(','))
if (files.length === 1) {
  const dist = fs.readFileSync('webui/dist/assets/' + files[0], 'utf8')
  // 缩小后：拖拽松手那段应有 commitEdit 的调用点，且总数比"仅三个出口"多
  const n = (dist.match(/拖拽结束|拖拽結束/g) || []).length
  check('产物里带「拖拽结束」这个 reason', n > 0,
    'dist 是旧的 → 改了源码没重建 dist')
}

console.log(`\n${fail === 0 ? '✅ 全部通过' : '❌ 有失败'}：${pass} 项通过 / ${fail} 项失败`)
process.exit(fail === 0 ? 0 : 1)
