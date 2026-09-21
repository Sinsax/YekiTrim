// 字幕折行（`wrapCaptionParts`）的漂移 + 回归检查。
//
// 同一条算法有两份实现：出片权威路径 `crates/vtrim-polish/templates/index.html`
// （DOM 预览与 canvas 出片都调它）与编辑器预览 `webui/src/polish/engine.js`
// （经 `PolishPreview.vue` 的 `capRowsWrapped`）。两份一旦漂移，表现就是
// 「预览与成片断行位置不一致」。
//
// ⚠️ 这里更重要的是一条**回归钉子**：`append` 曾经先读 `cur.length` 再判溢出，
// 而溢出分支里的 `pushLine()` 会把 `cur` 置空 —— 于是**任何一次折行**都直接抛
// `TypeError: can't access property "length", cur is null`。竖屏画布安全宽只有
// 1080*0.875 = 945px（横屏 1680px），一条普通长度的字幕必折行，表现是
// 「竖屏一播到长字幕（往往伴随贴纸）编辑器就崩」。
//
// 运行： node webui/scripts/check-caption-wrap.mjs
//
// 为什么不写成 `node --test` / 不 import engine.js：webui/package.json 没有
// `"type": "module"`，Node 会把 engine.js 当 CJS。check-overlay.mjs 用 data-URL
// 动态 import 绕开过这个，但那招**已经失效** —— engine.js 现在有 `import ... from
// './canvas.js'`，data-URL 没有可解析的相对基准，会直接
// `ERR_UNSUPPORTED_RESOLVE_REQUEST`。所以这里改成**从源码文本里抠函数**
// （两份都用同一套配对括号提取），不加载模块、零依赖、也不动模块类型。

import { readFile } from 'node:fs/promises'
import { fileURLToPath } from 'node:url'
import { dirname, join } from 'node:path'

const here = dirname(fileURLToPath(import.meta.url))
const root = join(here, '..', '..')

// ── 从 templates/index.html 里抠出顶层函数（用配对括号取整段）────────────────────
function extractFunction(src, name) {
  const start = src.indexOf(`function ${name}(`)
  if (start < 0) throw new Error(`index.html 里找不到 function ${name}(`)
  let i = src.indexOf('{', start)
  let depth = 0
  for (; i < src.length; i++) {
    const ch = src[i]
    if (ch === '/' && src[i + 1] === '/') {
      const nl = src.indexOf('\n', i)
      if (nl < 0) break
      i = nl
      continue
    }
    if (ch === '/' && src[i + 1] === '*') {
      const e = src.indexOf('*/', i + 2)
      i = e < 0 ? src.length : e + 1
      continue
    }
    if (ch === "'" || ch === '"' || ch === '`') {
      const q = ch
      i++
      while (i < src.length && src[i] !== q) {
        if (src[i] === '\\') i++
        i++
      }
      continue
    }
    if (ch === '{') depth++
    else if (ch === '}') {
      depth--
      if (depth === 0) return src.slice(start, i + 1)
    }
  }
  throw new Error(`function ${name} 的括号没配平`)
}

const htmlPath = join(root, 'crates', 'vtrim-polish', 'templates', 'index.html')
const html = await readFile(htmlPath, 'utf8')
const tplWrap = new Function(
  `${extractFunction(html, 'wrapCaptionParts')}
   return wrapCaptionParts`
)()

const enginePath = join(root, 'webui', 'src', 'polish', 'engine.js')
const engineSrc = await readFile(enginePath, 'utf8')
const engineWrap = new Function(
  `${extractFunction(engineSrc, 'wrapCaptionParts')}
   return wrapCaptionParts`
)()
const engine = { wrapCaptionParts: engineWrap }

// ── 断言脚手架 ────────────────────────────────────────────────────────────
let failed = 0
const eq = (a, b) => JSON.stringify(a) === JSON.stringify(b)

function check(label, actual, expected) {
  if (eq(actual, expected)) {
    console.log(`  ok   ${label}`)
  } else {
    failed++
    console.log(`  FAIL ${label}`)
    console.log(`         实得: ${JSON.stringify(actual)}`)
    console.log(`         期望: ${JSON.stringify(expected)}`)
  }
}

/** 与实现无关的假 measure：每字 72px（≈ 72px 字号的 CJK 字宽），结果可复现 */
const measure = (t) => String(t).length * 72
const parts = (text, hl = false) => [{ text, hl }]
const flatten = (r) => r.lines.map((line) => line.map((p) => p.text).join(''))

// 安全宽：竖屏画布 1080*0.875 = 945（触发折行的门槛比横屏低得多）
const PORTRAIT = 1080 * 0.875
const LANDSCAPE = 1920 * 0.875

// 长到必须折行的字幕（30 字 ≈ max_segment_chars 的上限）
const LONG = '一二三四五六七八九十一二三四五六七八九十一二三四五'

console.log('折行不崩（曾经的 TypeError：pushLine 置空 cur 后仍读 cur.length）')
for (const [label, fn] of [['template', tplWrap], ['engine  ', engine.wrapCaptionParts]]) {
  for (const [where, maxLineW] of [['竖屏', PORTRAIT], ['横屏', LANDSCAPE]]) {
    let r = null
    let err = null
    try { r = fn(parts(LONG), maxLineW, measure) } catch (e) { err = e }
    check(`[${label}] ${where}长字幕不抛异常`, err && err.message, null)
    if (!err) check(`[${label}] ${where}长字幕确实折成多行`, r.lines.length > 1, true)
  }
  // 边界：刚好等于安全宽（13 字 × 72 = 936 ≤ 945）不该折行
  let one = null
  try { one = fn(parts('一二三四五六七八九十一二三'), PORTRAIT, measure) } catch (e) { one = e }
  check(`[${label}] 刚好装得下 → 单行`, one instanceof Error ? one.message : flatten(one).length, 1)
  // 空输入不能产出空字幕行，也不能崩
  let empty = null
  try { empty = fn([], PORTRAIT, measure) } catch (e) { empty = e }
  check(`[${label}] 空 parts → 0 行`, empty instanceof Error ? empty.message : empty.lines.length, 0)
}

console.log('\n两份实现逐例对拍（断行位置必须一致）')
const cases = [
  { name: '竖屏长字幕', p: parts(LONG), w: PORTRAIT },
  { name: '横屏长字幕', p: parts(LONG), w: LANDSCAPE },
  { name: '短字幕', p: parts('短句'), w: PORTRAIT },
  { name: '高亮跨行', p: [{ text: '一二三四五六七八九十', hl: true }, { text: '甲乙丙丁戊己庚辛壬癸', hl: false }], w: PORTRAIT },
  { name: '超长（撞 0.7 缩放下限）', p: parts(LONG + LONG), w: PORTRAIT },
]
for (const c of cases) {
  const a = tplWrap(c.p.map((x) => ({ ...x })), c.w, measure)
  const b = engine.wrapCaptionParts(c.p.map((x) => ({ ...x })), c.w, measure)
  check(`[对拍] ${c.name}`, { lines: flatten(a), scale: a.scale }, { lines: flatten(b), scale: b.scale })
}

console.log('\n自证有效：把顺序改回修复前，必须抛异常（否则本脚本是空转的）')
// 若上面那些 "不抛异常" 只是因为测试数据没触发折行，这个检查会失败。
// 两条 append 前两行的**顺序**是唯一的差异：修复前先读 cur.length 再判溢出。
//
// ⚠️ 仓库里这两个文件都是 **CRLF**（engine.js 708 行、index.html 2722 行全是 CRLF），
// 所以下面的模式一律用 `\\r?\\n` 匹配 —— 写死 `\n` 会静默匹配不到，让这项检查变成空转。
// （注意是双反斜杠：单反斜杠在 JS 字符串里会先被转义成真的回车换行符，正则就废了。）
const PRE_FIX_JS = [
  ['if \\(cur && cur\\.length && curW \\+ w > maxLineW \\+ EPS\\) pushLine\\(\\)\\r?\\n\\s*if \\(!cur\\) cur = \\[\\]',
   'if (!cur) cur = []\r\n      if (cur.length && curW + w > maxLineW + EPS) pushLine()'],
  ['if\\(cur&&cur\\.length&&curW\\+w>maxLineW\\+EPS\\)pushLine\\(\\);\\r?\\n\\s*if\\(!cur\\)cur=\\[\\];',
   'if(!cur)cur=[];\r\n      if(cur.length&&curW+w>maxLineW+EPS)pushLine();'],
]
for (const [label, src] of [['template', html], ['engine  ', engineSrc]]) {
  let broken = null
  for (const [pattern, to] of PRE_FIX_JS) {
    const re = new RegExp(pattern)
    if (re.test(src)) { broken = src.replace(re, to); break }
  }
  check(`[${label}] 能在源码里定位到修复点`, broken !== null, true)
  if (broken === null) continue
  const fn = new Function(`${extractFunction(broken, 'wrapCaptionParts')}; return wrapCaptionParts`)()
  let msg = null
  try { fn(parts(LONG), PORTRAIT, measure) } catch (e) { msg = e.message }
  check(`[${label}] 修复前顺序在竖屏长字幕上抛 TypeError`, /length/.test(msg || ''), true)
}

console.log(failed ? `\n${failed} 项失败` : '\n全部通过')
process.exit(failed ? 1 : 0)
