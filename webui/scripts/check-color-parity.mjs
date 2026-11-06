#!/usr/bin/env node
// **配色的「跨端逐值相等」判据**（2026-10-20 task-40；2026-10-30 缩表）。
//
// # 为什么需要它（check-overlay.mjs 管不了这件事）
//
// check-overlay.mjs 盯的是**色档算法**（overlayStops / overlayFadeStop / gradientCss /
// getOverlayState 四份实现同口径），**不比较任何强调色值** —— 实测：把 index.html 里的
// #dcbda0 改成 #d0b8a0，它**照样绿**。于是「改配色只改一半」在过去**没有任何护栏**：
// 面板/预览一个色、成片另一个色，两边各自看都正常。
//
// # 退役了什么（2026-10-30 颜色链收尾）
//
// 原来的 ① 文字元素颜色预设（placement.rs 的 ElementColor::rgba）与 ② 转译器
// TEXT_STYLE_FALLBACK.comment.color 的对拍**整条退役**：
// * ElementColor 没有生产消费者（LLM 写、存盘、出片不看），提示词教学也已删除；
// * 文字层 2026-10-20 退役，TextStyle / ELEMENT_FONT_PX 随之删除
//   ⇒ ② 的"Rust 真源 ↔ 转译器兜底"那一半已经不存在。
// 留着这两条只会恒红（比没有门禁更坏），所以删掉——但**没有放宽任何活着的判据**。
//
// # 现在还锁什么
//
//   ③ 字幕高亮默认：render-core/src/types.rs 的 default_highlight_color()
//      ↔ templates/index.html 的两处兜底 ↔ commands.rs 里 polish init 模板写出的值
//      （封面 hl_color 直接读 ③，所以改 ③ 两处一起变）
//   ④ 主题强调色：tokens.css 深色 --red/--amber ↔ 样稿 sakura-cream.html 的图例
//      （卡里写着「特征色取自立绘、样稿与 tokens 必须同源改」）
//
// ⚠️ **真源是 ③ 与 ④**：判据从它们**取值**再要求其它处相等（不是把 A 套色号写死在这里）
//   ⇒ 整套换色只要同源改，本脚本**不用动**；只改一处 ⇒ 当场红。
//
// 运行： node webui/scripts/check-color-parity.mjs
// 自证： node webui/scripts/check-color-parity.mjs --mutate —— 逐条真改内存里的源码，
//        每一处变异都**必须**让对应对拍红；全红才算判据不是空转。
import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const HERE = path.dirname(fileURLToPath(import.meta.url))
// ⚠️ 本脚本住 webui/scripts/，而被对拍的文件在**仓库根**下 ⇒ 往上两级。
const ROOT = path.join(HERE, '..', '..')

const FILES = {
  types: 'crates/yekitrim-render-core/src/types.rs',
  template: 'crates/yekitrim-polish/templates/index.html',
  commands: 'crates/yekitrim-polish/src/commands.rs',
  tokens: 'webui/src/styles/tokens.css',
  mockup: 'docs/web-ui-mockups/sakura-cream.html',
}

const load = () => {
  const out = {}
  for (const [k, rel] of Object.entries(FILES)) out[k] = fs.readFileSync(path.join(ROOT, rel), 'utf8')
  return out
}

// ---- 解析器（都做成「取不到 ⇒ undefined」，由判据自己报红）--------------------

// ⚠️ 元素颜色预设 / 转译器 comment 色的解析器已随颜色链退役（见文件头）。

/// `default_highlight_color()` 返回的那个 hex。
function parseHighlightDefault(src) {
  const m = /fn default_highlight_color\(\)\s*->\s*String\s*\{\s*(?:\/\/[^\n]*\n\s*)*"(#[0-9A-Fa-f]{6})"\s*\.into\(\)/.exec(src)
  return m ? m[1].toUpperCase() : undefined
}

/// `index.html` 里两处高亮兜底 hex（顺序：CSS 拼串、canvas 取值）。
function parseTemplateHighlights(src) {
  const out = []
  // ⚠️ 必须**限定 `subStyle.`**：`subS.highlight_color` 那行也含 `highlight_color||'…'`，
  //    不限定就会被上面这条再算一遍（实测 3 条而不是 2 条）。
  for (const m of src.matchAll(/subStyle\.highlight_color\s*\|\|\s*'(#[0-9A-Fa-f]{6})'/g)) out.push(m[1].toUpperCase())
  for (const m of src.matchAll(/subS\.highlight_color\s*\|\|\s*'(#[0-9A-Fa-f]{6})'/g)) out.push(m[1].toUpperCase())
  return out
}

/// `commands.rs` 里 `polish init` 模板写出的 highlight_color。
function parseCommandsHighlight(src) {
  const m = /highlight_color\s*=\s*"(#[0-9A-Fa-f]{6})"/.exec(src)
  return m ? m[1].toUpperCase() : undefined
}

/// tokens.css **暗色块**里的 `--red` / `--amber`。
function parseTokensDark(src) {
  // ⚠️ **lastIndexOf**：文件头那段注释里也写着同一个选择器，indexOf 会从注释处切，
  //    于是取到的是**亮色**块的 --red/--amber（实测就是这么错的）。
  const at = src.lastIndexOf(':root[data-theme="dark"]')
  if (at < 0) return {}
  const block = src.slice(at)
  const grab = (name) => {
    const m = new RegExp('--' + name + ':\\s*(#[0-9A-Fa-f]{6})').exec(block)
    return m ? m[1].toUpperCase() : undefined
  }
  return { red: grab('red'), amber: grab('amber') }
}

/// 样稿图例里那两行 `#XXXXXX … → 语义红` / `… → 警示/提醒`。
function parseMockupLegend(src) {
  // ⚠️ 那一行有**多个 hex**（裙暖白 / 语义红 …），必须只认**紧挨标签**的那个：
  //    `[^\n#]{0,20}` 不许跨过下一个 `#`。
  const red = /(#[0-9A-Fa-f]{6})\s+[^\n#]{0,20}?语义红/.exec(src)
  const amber = /(#[0-9A-Fa-f]{6})\s+[^\n#]{0,20}?(?:警示|提醒)/.exec(src)
  return { red: red ? red[1].toUpperCase() : undefined, amber: amber ? amber[1].toUpperCase() : undefined }
}

// ---- 判据 -------------------------------------------------------------------
function checkAll(src) {
  const fails = []
  let checks = 0
  const ok = (cond, msg) => { checks++; if (!cond) fails.push(msg) }

  // ①② 文字元素颜色预设：**已整条退役**（见文件头），这里不再对拍。

  // ③ 字幕高亮默认 == index.html 两处兜底 == commands.rs 的 init 模板
  const hl = parseHighlightDefault(src.types)
  const tpl = parseTemplateHighlights(src.template)
  const cmd = parseCommandsHighlight(src.commands)
  ok(!!hl && tpl.length === 2 && !!cmd,
    '③ 高亮默认色取不齐（types ' + hl + ' / template ' + JSON.stringify(tpl) + ' / commands ' + cmd + '）')
  ok(tpl.every((c) => c === hl),
    '③ 字幕高亮：render-core 默认 ' + hl + ' vs index.html 兜底 ' + JSON.stringify(tpl) +
    ' —— 预览/成片会一个色')
  ok(cmd === hl,
    '③ 字幕高亮：render-core 默认 ' + hl + ' vs commands.rs 模板 ' + cmd +
    ' —— 新工程写出去的还是旧色')

  // ④ tokens 深色强调色 == 样稿图例（卡里：特征色取自立绘、样稿与 tokens 同源改）
  const tok = parseTokensDark(src.tokens)
  const mock = parseMockupLegend(src.mockup)
  ok(!!tok.red && !!tok.amber && !!mock.red && !!mock.amber,
    '④ 取不齐 tokens 深色 --red/--amber 或样稿图例（' + JSON.stringify(tok) + ' / ' + JSON.stringify(mock) + '）')
  ok(tok.red === mock.red,
    '④ 深色 --red：tokens ' + tok.red + ' vs 样稿图例 ' + mock.red + ' —— 样稿是唯一设计标准，必须同源改')
  ok(tok.amber === mock.amber,
    '④ 深色 --amber：tokens ' + tok.amber + ' vs 样稿图例 ' + mock.amber)

  return { fails, checks, hl, tpl, cmd, tok, mock }
}

// ---- 自证：逐处真改一个分量，必须红 -----------------------------------------
function mutate(base) {
  const cases = []
  const clone = () => ({ ...base })
  const add = (name, key, from, to, expectIdx) => cases.push({ name, key, from, to, expectIdx })
  // ⚠️ 从**当前源码的实体**取旧串，避免写死（换色号后自证仍然有效）
  // ⚠️ 元素颜色那三条变异（ElementColor / TextStyle::Comment / 转译器 comment）已随
  //    颜色链退役删除 —— 它们连"被对拍的那一侧"都不存在了，留着是空转。
  const tplOld = parseTemplateHighlights(base.template)[0]
  const mockOld = parseMockupLegend(base.mockup).red
  add('index.html 高亮兜底换值', 'template', "'" + tplOld + "'", "'" + (tplOld === '#000000' ? '#010101' : '#000000') + "'", 2)
  add('样稿图例换值', 'mockup', mockOld, mockOld === '#000000' ? '#010101' : '#000000', 4)
  const out = []
  for (const c of cases) {
    const src = clone()
    if (!c.from || !src[c.key].includes(c.from)) { out.push({ ...c, hit: false, red: false }); continue }
    src[c.key] = src[c.key].replace(c.from, c.to)
    const { fails } = checkAll(src)
    out.push({ ...c, hit: true, red: fails.length > 0, fails })
  }
  return out
}

// ---- main -------------------------------------------------------------------
const base = load()
if (process.argv.includes('--mutate')) {
  const res = mutate(base)
  let bad = 0
  for (const r of res) {
    const good = r.hit && r.red
    if (!good) bad++
    console.log((good ? '  ✓ ' : '  ✗ ') + r.name + (r.hit ? (r.red ? ' -> 红（判据有效）' : ' -> **没红**（判据空转）') : ' -> 变异串没命中（自证本身坏了）'))
  }
  console.log(bad === 0
    ? '✓ 配色四处对拍：' + res.length + ' 处变异全部打红（判据不是空转的）'
    : '✗ 配色四处对拍：' + bad + '/' + res.length + ' 处变异没被抓住')
  process.exit(bad === 0 ? 0 : 1)
}

const { fails, checks, el, hl, tok } = checkAll(base)
if (fails.length === 0) {
  console.log('✓ 配色跨端逐值相等（' + checks + ' 项判据）：高亮 ' + hl +
    ' · 深色 red ' + tok.red + ' / amber ' + tok.amber)
  process.exit(0)
}
console.log('✗ 配色对拍失败（' + fails.length + ' / ' + checks + '）：')
for (const f of fails.slice(0, 20)) console.log('    ' + f)
process.exit(1)