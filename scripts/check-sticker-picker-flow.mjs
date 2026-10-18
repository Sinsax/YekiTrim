#!/usr/bin/env node
// **贴纸选择器的「行递进 + 固定高度」契约**（2026-10-15 定稿）。
//
// # 背景（用户口径，三轮迭代）
//
// ① 「当前贴纸的添加不合理，应该分三段选择进行」—— 老实现把 (包 · tag) 压成一行平铺，
//    用户得在一列 ysyk · 害怕 里同时认出包与分类。
// ② 「以行形式递进的感觉…点击对应的行就回到那个页面重新选择」—— 已选的那几级各成一行。
// ③ 「固定高度」—— 切页面/切状态**不许跳**（老实现是 132px/210px 的 max-height，切一级换一个高度）。
//
// # 它判什么
//
// ① 三个高度写死：整块 330 / 路径区 78 / 正文 196（列表与网格靠滚动，不改整块高度）。
// ② 路径区**恒 3 行**（pack / tag / sticker）：没轮到的那两级写成 todo 占位行 ——
//    三页等高**就靠它**；砍掉占位会让第 1/2/3 页各差 24/27px（正是要修的东西）。
// ③ **换值才清更深的选择**：重选同一个包/同一个分类时早退（老实现是点包就无条件清 name/source）。
// ④ **点已选行（✎）只换页、不发 patch**（字段一个都不动）。
// ⑤ emoji 是第 1 页的一行，且点码点能写 source=emoji（老实现里 emoji 基本选不到）。
// ⑥ **读失败分支必须排在「空库」之前** —— 顺序反了就是用户报过的「假空态」
//    （接口挂了显示成「贴纸库为空」，人就去翻自己的贴纸目录）。
// ⑦ 筛选/手填输入必须 @input.stop：父面板有 @input="onPanelEdit"，
//    不 stop 的话**每敲一个键都当成一次编辑并落盘**。
// ⑧ TimelineEditor 两个载体都要接线（:lib-error / @retry-lib），且读失败要挂原因。
//
// 用法：node scripts/check-sticker-picker-flow.mjs   （退出码非 0 即失败）

import { readFileSync } from 'node:fs'
import { join, dirname } from 'node:path'
import { fileURLToPath } from 'node:url'

const HERE = dirname(fileURLToPath(import.meta.url))
const ROOT = join(HERE, '..')
const FILE = join(ROOT, 'webui', 'src', 'components', 'polish', 'StickerPicker.vue')
const TL = join(ROOT, 'webui', 'src', 'components', 'TimelineEditor.vue')

let pass = 0
let fail = 0
const failures = []
function ok(cond, what) {
  if (cond) { pass += 1 } else { fail += 1; failures.push(what) }
}

// 剥注释：本文件里解释「为什么」的注释本身就含这些符号（emit( / libError / 高度数字），
// 不剥就是**判据被自己的说明顶翻**。⚠️ .vue 里 HTML 注释要一起剥。
const strip = (s) => s
  .replace(/<!--[\s\S]*?-->/g, '')
  .replace(/\/\*[\s\S]*?\*\//g, '')
  .replace(/(^|[^:])\/\/[^\n]*/g, '$1')

const raw = readFileSync(FILE, 'utf8')
const src = strip(raw)

/** 取一个函数体（从 function name( 到首个顶格 }）—— 判据要读**行为**，不是「整篇里出现过什么」。 */
function body(s, start) {
  const i = s.indexOf(start)
  if (i < 0) return ''
  const j = s.indexOf('\n}', i)
  return j < 0 ? s.slice(i) : s.slice(i, j)
}
/** 某个 CSS 选择器的块里有没有写死这个高度（new RegExp 的字符串要双反斜杠 ⇒ 正则里是 \s）。 */
const hasHeight = (sel, px) =>
  new RegExp(sel.replace(/[.*+?^${}()|[\]\\]/g, '\\$&') + '\\s*\\{[^}]*height:\\s*' + px + 'px').test(raw)

// ---------------------------------------------------------------------------
// ① 三个高度写死
// ---------------------------------------------------------------------------
ok(hasHeight('.sp', 330), '① 整块高度写死 .sp{height:330px}（切页面/切状态都不许跳）')
ok(hasHeight('.stack', 78), '① 路径区写死 .stack{height:78px}（3 行 × 24 + 2 × 3）')
ok(hasHeight('.body', 192), '① 正文区写死 .body{height:192px}（列表/网格在它里面滚）')

// ---------------------------------------------------------------------------
// ② 路径区恒 3 行
// ---------------------------------------------------------------------------
{
  const lvs = src.match(/lv: '(pack|tag|sticker)'/g) || []
  ok(lvs.length === 3, '② 路径区必须恒 3 行（pack/tag/sticker），实得 ' + lvs.length + ' 条')
  ok(/v-for="r in rows"/.test(src) && /:key="r\.lv"/.test(src),
    '② 模板按 rows 渲染、:key 用 lv（别用下标：三行的身份是级别）')
  ok(/todo:\s*!r\.val\s*&&\s*!cur/.test(src),
    '② 未轮到的那两级留 todo 占位行 —— 第 1/2/3 页等高**就靠它**')
  // [自证] 砍掉占位 ⇒ 上面那条必须翻面
  const m = src.replace(/todo:\s*!r\.val\s*&&\s*!cur/, 'todo: false')
  ok(m !== src, '② [自证] 变异锚点还在（todo 的算法）')
  ok(!/todo:\s*!r\.val\s*&&\s*!cur/.test(m), '② [自证] 砍掉占位后判据翻面')
}

// ---------------------------------------------------------------------------
// ③ 换值才清更深（重选同一个值保留）
// ---------------------------------------------------------------------------
const keepSamePack = (s) => /if \(name === props\.pack\)/.test(body(s, 'function pickPack('))
const keepSameTag = (s) => /if \(next !== props\.tag\)/.test(body(s, 'function pickTag('))
ok(keepSamePack(src), '③ 重选同一个包 ⇒ 早退（更深的选择保留）；换包才清 sticker_tag/name/source')
ok(/\{ keyword: name, sticker_tag: null, name: null, source: null \}/.test(body(src, 'function pickPack(')),
  '③ 换包要清 sticker_tag/name/source（旧图不属于新包）')
ok(keepSameTag(src) && /sticker_tag: next \|\| null, name: null, source: null/.test(body(src, 'function pickTag(')),
  '③ 换分类才清 name/source；「全部（不限）」⇒ sticker_tag 清空（next 为空串时写 null）')
{
  // [自证] 去掉早退 ⇒ 判据翻面
  const m = src.replace("if (name === props.pack) { edit.value = 'tag'; return }", '')
  ok(m !== src, '③ [自证] 变异锚点还在（pickPack 的同值早退）')
  ok(!keepSamePack(m), '③ [自证] 去掉早退后判据翻面')
}

// ---------------------------------------------------------------------------
// ④ 点已选行只换页、不发 patch
// ---------------------------------------------------------------------------
const goIsPure = (s) => {
  const b = body(s, 'function go(')
  return /edit\.value = lv/.test(b) && !/emit\(/.test(b)
}
ok(goIsPure(src), '④ 点已选行（✎）只换页、不发 patch —— 字段一个都不动')
{
  // [自证] 往 go() 里塞一个 emit ⇒ 判据翻面
  const m = src.replace('function go(lv) { edit.value = lv }', "function go(lv) { emit('pick', {}); edit.value = lv }")
  ok(m !== src, '④ [自证] 变异锚点还在（go 的实现）')
  ok(!goIsPure(m), '④ [自证] 塞进 emit 后判据翻面')
}

// ---------------------------------------------------------------------------
// ⑤ emoji 是第 1 页的一行
// ---------------------------------------------------------------------------
ok(/pickPack\('emoji'\)/.test(src), '⑤ 第 1 页的包列表里要有 emoji 这一行（老实现里它基本选不到）')
ok(/source: EMOJI/.test(src) && /keyword: EMOJI/.test(src),
  '⑤ 点表情 ⇒ keyword=emoji / source=emoji（与 LLM、process 同一口径）')

// ---------------------------------------------------------------------------
// ⑥ 读失败 vs 空库：顺序不许反
// ---------------------------------------------------------------------------
{
  const iErr = src.indexOf('v-if="libError"')
  const iEmpty = src.indexOf('!packs.length')
  ok(iErr > 0 && iEmpty > iErr,
    '⑥ libError 那一支必须排在「空库」之前 —— 反了就是「接口挂了」显示成「贴纸库为空」的假空态')
}

// ---------------------------------------------------------------------------
// ⑦ 筛选/手填输入不许冒泡成面板编辑
// ---------------------------------------------------------------------------
{
  const n = (src.match(/@input\.stop/g) || []).length
  ok(n >= 3, '⑦ 三个筛选/手填输入都要 @input.stop（父面板 @input 会把每次敲键当落盘改动），实得 ' + n)
}

// ---------------------------------------------------------------------------
// ⑧ TimelineEditor 的接线（两个载体各一次）
// ---------------------------------------------------------------------------
{
  const tl = strip(readFileSync(TL, 'utf8'))
  const nErr = (tl.match(/:lib-error="stickerLibError"/g) || []).length
  const nRetry = (tl.match(/@retry-lib="loadPickers"/g) || []).length
  ok(nErr === 2, '⑧ 两个贴纸载体都要传 :lib-error，实得 ' + nErr)
  ok(nRetry === 2, '⑧ 两个贴纸载体都要能重试（@retry-lib="loadPickers"），实得 ' + nRetry)
  ok(/stickerLibError\.value = \(e && e\.message\)/.test(tl),
    '⑧ 读失败要把原因挂出去（不是静默写空数组）')
  ok(!/catch \{ stickerLib\.value = \[\] \}/.test(tl),
    '⑧ 不许退回旧的静默写法：catch { stickerLib.value = [] }（假空态）')
}

console.log('贴纸选择器「行递进 + 固定高度」：' + pass + ' 项通过，' + fail + ' 项失败')
if (fail) {
  console.log()
  for (const f of failures) console.log('  ✗ ' + f)
  process.exit(1)
}
console.log('✓ 路径区恒 3 行 / 高度写死 330·78·192 / 换值才清更深 / 点行只换页 / emoji 可选中 / 读失败不装空库')
