// 字体输入面门禁（2026-10-02，字体职责分离 + 输入层收敛）。
//
// # 它守的是什么
//
// 「用哪支字体」这件事在本仓曾经**同一份知识写了 7 遍**（3 处 Rust + 4 处前端），
// 靠注释互相提醒"要同序"。漂移的症状是「预览和成片不一样」，**而且不报错** ——
// 正是本仓反复吃亏的那一类（同 check-effect-registry / check-dhampir-wiring 的动机）。
//
// 现在唯一权威是 yekitrim_core::fonts::resolve_font_plan / emoji_font / fallback_chain。
// 这条门禁做**两向判据**：
//   ① **登记的来源必须真的存在**（防止清单腐坏：代码改了、清单没改）；
//   ② **那些文件里不许有没人认领的字体栈字面量**（防止"第 8 份写死又长出来"）。
//
// # 为什么"逐字面量白名单"而不是"扫所有 font-family"
//
// 模板里的 canvas 量宽串（ctx.font = '600 ' + fs + 'px sans-serif'）**不是字体选择**，
// 它只是量文字宽度用的度量串 —— 把它也纳入会得到一堆假阳性，门禁就没人看了
// （同 check_module_boundaries 的"先剥注释"教训）。
//
// ⚠️ **两种形态都要认**（实测：只认 CSS 那种会漏掉最关键的两处）：
//   ① CSS 形态：font-family:'A','B',sans-serif —— 模板 / 组件里的写法；
//   ② **裸常量形态**：pub const EMOJI_FONT: &str = "'A','B',sans-serif" —— Rust 侧的写法。
//   cover.rs::EMOJI_FONT 与 server.rs::SYSTEM_FONT_STACK 正是**最该被收敛的两处**，
//   而它们**不带 font-family: 前缀** —— 只写 CSS 判据的话，这条门禁会"绿着"放过它们。
//
// 用法：node scripts/check-font-inputs.mjs [--self-test]
// 退出码非 0 = 有未认领的字体栈字面量（或清单腐坏）。

import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const __dirname = path.dirname(fileURLToPath(import.meta.url))
const ROOT = path.resolve(__dirname, '..')

/** 唯一权威（改这里之前先确认 crates/yekitrim-core/src/fonts.rs 真的有这些函数）。 */
const AUTHORITY = {
  file: 'crates/yekitrim-core/src/fonts.rs',
  symbols: ['resolve_font_plan', 'emoji_font', 'fallback_chain'],
}

/**
 * 允许存在的字体栈字面量（**每一条都要写清"为什么它不是漂移"**）。
 *
 * ⚠️ 往这里加一条之前先问：它能不能改成问 resolve_font_plan？
 * 能改就该改代码，而不是加白名单 —— 白名单是给"确实不该收敛"的地方用的。
 */
const ALLOWED = [
  // ⚠️ `crates/yekitrim-core/src/fonts.rs` **不在白名单里** —— 它是唯一权威本体，但
  // `fallback_chain()` 的字面量是**逐项 vec!**（不是"引号族名 + 通用族"那种栈形状），
  // 只有 `#[cfg(test)]` 里那条零回归断言才是栈形状（测试区已被跳过）。
  // 所以它天然干净 —— **别再把它加回来**：加回来就等于给权威本体开后门。
  { file: 'crates/yekitrim-polish/src/render/server.rs', why: 'SYSTEM_FONT_STACK 常量：空库回落态的文档锚点 + 单测钉子（收敛后由 fallback_chain 供给）' },
  { file: 'crates/yekitrim-polish/src/cover.rs', why: 'EMOJI_FONT：封面 emoji 特效层（inline style，CSS 扫不到，必须就地给）' },
  { file: 'crates/yekitrim-polish/templates/index.html', why: '出片模板：canvas 出片路径的权威实现，量宽/绘制串必须自带字面量（不读 DOM、不跑 JS）' },
  { file: 'webui/src/components/PolishPreview.vue', why: '预览：旧 server 兼容用的字面量兜底（新 server 永远给非空 fallback_stack）' },
  { file: 'webui/src/components/CoverDesigner.vue', why: '设计器镜像：与 cover.rs 的特效层同源（镜像清单要求两处一起改）' },
  { file: 'webui/src/components/FontLibrary.vue', why: '字体库卡片预览栈：每支字体现场注册后的展示用（不是出片字体选择）' },
  { file: 'webui/src/styles/tokens.css', why: '全站 UI 字体 token（--font），与出片无关' },
  { file: 'crates/yekitrim-server/src/web.rs', why: '503 构建指引页内联样式，与出片无关' },
]

/** 字体栈字面量的两种形态（见文件头说明）。 */
const LITERAL_RES = [
  /font-family\s*:[^;}"']*['"][^;}]*?(?:sans-serif|serif|monospace)/,
  /['"][^'"]+['"]\s*,[^;]*?(?:sans-serif|serif|monospace)\b/,
]

/** 一行里是否有字体栈字面量。 */
export function lineHasLiteral(line) {
  return LITERAL_RES.some((re) => re.test(line))
}

/**
 * 扫一个文件里所有的字体栈字面量（返回行号 + 原文）。
 *
 * ⚠️ **测试模块要跳过**（`#[cfg(test)] mod tests` / `describe(` 之类）：那里的字体栈是
 * **夹具**（喂给被测函数的输入字符串），不是产品里的字体选择。实测踩到：给
 * `ensure_emoji_fallback` 写的两条幂等单测里带了 `font-family:'X',serif` 这样的样本，
 * 门禁把自己的夹具当成"第 8 份写死"报了红。
 * 判据取"文件里出现 `mod tests`/`#[cfg(test)]` 之后的行" —— 粗但够用（Rust 惯例是 tests 收尾）。
 */
export function scanLiterals(absPath) {
  let text
  try { text = fs.readFileSync(absPath, 'utf8') } catch { return [] }
  const out = []
  const lines = text.split(/\r?\n/)
  // 测试区的起点：第一个 `#[cfg(test)]` 或 `mod tests` 所在行
  let testFrom = Number.POSITIVE_INFINITY
  for (let i = 0; i < lines.length; i++) {
    if (/^\s*#\[cfg\(test\)\]|^\s*(pub\s+)?mod tests\b/.test(lines[i])) { testFrom = i; break }
  }
  for (let i = 0; i < lines.length; i++) {
    if (i >= testFrom) break
    const trimmed = lines[i].trim()
    if (trimmed.startsWith('//') || trimmed.startsWith('*') || trimmed.startsWith('<!--') || trimmed.startsWith('#')) continue
    if (lineHasLiteral(lines[i])) out.push({ line: i + 1, text: trimmed.slice(0, 120) })
  }
  return out
}

/** 主判据。返回 { ok, bad[] }。 */
export function judge(root = ROOT) {
  const bad = []

  const authPath = path.join(root, AUTHORITY.file)
  let authText = ''
  try { authText = fs.readFileSync(authPath, 'utf8') } catch {
    bad.push('缺唯一权威文件：' + AUTHORITY.file)
  }
  for (const sym of AUTHORITY.symbols) {
    if (authText && !new RegExp('fn\\s+' + sym + '\\b').test(authText)) {
      bad.push('唯一权威里找不到 ' + sym + '（清单腐坏：改了代码没改这里）')
    }
  }

  const allowedFiles = new Set(ALLOWED.map((a) => a.file))
  for (const a of ALLOWED) {
    const p = path.join(root, a.file)
    if (!fs.existsSync(p)) {
      bad.push('白名单里登记了不存在的文件：' + a.file + '（清单腐坏）')
      continue
    }
    if (scanLiterals(p).length === 0) {
      bad.push('白名单里的 ' + a.file + ' 已无任何字体栈字面量 —— 该从白名单删掉（清单腐坏）')
    }
  }

  // ⚠️ 本文件自身要排除：它的 --self-test 会**就地造字体栈样本**（写进临时文件再删），
  // 那些样本行本来就在源码里 ⇒ 不排除的话门禁会被自己顶红（实测踩到）。
  const SELF = 'scripts/check-font-inputs.mjs'
  const SCAN_DIRS = ['crates', 'webui/src', 'scripts']
  const SKIP_DIRS = new Set(['node_modules', 'target', 'dist', '.git', '.workbuddy'])
  const walk = (dir, acc = []) => {
    let items = []
    try { items = fs.readdirSync(dir, { withFileTypes: true }) } catch { return acc }
    for (const it of items) {
      if (SKIP_DIRS.has(it.name)) continue
      const full = path.join(dir, it.name)
      if (it.isDirectory()) walk(full, acc)
      else acc.push(full)
    }
    return acc
  }
  for (const d of SCAN_DIRS) {
    for (const f of walk(path.join(root, d))) {
      if (!/\.(rs|vue|js|mjs|html|css)$/.test(f)) continue
      const rel = path.relative(root, f).split(path.sep).join('/')
      if (allowedFiles.has(rel) || rel === SELF) continue
      for (const h of scanLiterals(f)) {
        bad.push('未认领的字体栈字面量：' + rel + ':' + h.line + ' —— ' + h.text)
      }
    }
  }

  return { ok: bad.length === 0, bad }
}

/** --self-test：判据自身不许空转。 */
function selfTest() {
  const results = []
  const tmp = path.join(ROOT, '.workbuddy', 'font-gate-selftest')
  fs.rmSync(tmp, { recursive: true, force: true })
  fs.mkdirSync(tmp, { recursive: true })

  const bare = path.join(tmp, 'bare.rs')
  fs.writeFileSync(bare, 'pub const S: &str = "\'A\',\'B\',sans-serif";\n', 'utf8')
  results.push(['裸常量形态必须被抓到', scanLiterals(bare).length === 1])

  const css = path.join(tmp, 'a.css')
  fs.writeFileSync(css, "html { font-family:'A','B',sans-serif; }\n", 'utf8')
  results.push(['CSS 形态必须被抓到', scanLiterals(css).length === 1])

  const cmt = path.join(tmp, 'comment.rs')
  fs.writeFileSync(cmt, '/// 字体栈是 \'A\',\'B\',sans-serif 这样\nfn x() {}\n', 'utf8')
  results.push(['注释里的字样不许误报', scanLiterals(cmt).length === 0])

  const measure = path.join(tmp, 'measure.js')
  fs.writeFileSync(measure, "ctx.font = '600 48px sans-serif'\n", 'utf8')
  results.push(['纯量宽串不算字体栈', scanLiterals(measure).length === 0])

  // ⑥ 测试模块里的夹具不算产品字体栈（实测踩到：自己的单测样本被当成"第 8 份写死"）
  const withTests = path.join(tmp, 'has_tests.rs')
  fs.writeFileSync(withTests, 'pub fn f() {}\n\n#[cfg(test)]\nmod tests {\n    fn t() { let s = "\'X\',serif"; }\n}\n', 'utf8')
  results.push(['测试夹具不算产品字体栈', scanLiterals(withTests).length === 0])

  // ⑦ 但测试模块**之前**的字面量照旧要抓（别把整个文件放过）
  const beforeTests = path.join(tmp, 'before_tests.rs')
  fs.writeFileSync(beforeTests, 'pub const S: &str = "\'A\',\'B\',sans-serif";\n#[cfg(test)]\nmod tests {}\n', 'utf8')
  results.push(['测试模块之前的字面量照旧要抓', scanLiterals(beforeTests).length === 1])

  results.push(['缺权威文件必须报红', !judge(path.join(tmp, 'nope')).ok])

  fs.rmSync(tmp, { recursive: true, force: true })
  let badN = 0
  for (const [name, ok] of results) {
    console.log((ok ? '✓ ' : '✗ ') + name)
    if (!ok) badN += 1
  }
  console.log(badN === 0 ? '自检 ' + results.length + ' 项全过' : '自检有 ' + badN + ' 项没过')
  process.exit(badN === 0 ? 0 : 1)
}

if (process.argv.includes('--self-test')) selfTest()

const { ok, bad } = judge()
if (ok) {
  console.log('字体输入面全绿（白名单 ' + ALLOWED.length + ' 处，唯一权威 ' + AUTHORITY.symbols.length + ' 个符号）')
  process.exit(0)
}
console.log('字体输入面有未认领的字体栈字面量（或清单腐坏）：')
for (const b of bad) console.log('  ✗ ' + b)
console.log('\n修法：能改成问 resolve_font_plan 的就改代码；确实不该收敛的，加进 ALLOWED 并写清 why。')
process.exit(1)
