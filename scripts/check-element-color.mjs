#!/usr/bin/env node
// **文字颜色链「已退役」判据**（2026-10-30）。
//
// # 为什么退役（证据，不是一个决定）
//
// 颜色曾经是「一个事实、四处实现」：提示词教 LLM 写 color -> process 的 ElementColor 解析
// -> 写进 Event::Element.source 第 10 段 -> 转译器/预览再读回来。但：
//   * Event::Sticker / Event::Element{kind:"sticker"} 没有 color 字段 => LLM 照提示词写的
//     color 在 serde 反序列化时被静默丢弃（配了、存不了、也不报错）；
//   * process 里 ElementColor::parse / rgba 没有任何生产调用点（只有它自己的测试）；
//   * 颜色原本服务的文字层已于 2026-10-20 整条退役（读时丢弃）。
// => 这就是本仓最恨的「配了、不报错、没效果」。口径：彻底退役。
//
// # 本判据现在还锁什么
//
// 1. 不许复活：Rust 类型、提示词教学、转译器第 10 段读取、预览按名字查表、面板颜色入口；
// 2. 老工程仍可读：Event::Element.color 字段还在，source 串仍收 7/9/10 段；
// 3. 老工程文字轨的侧挂 ASS 路径（独占子目录 / dhampir-id.ass / 前后端 basename 一致）
//    -- 与颜色无关，但本脚本曾是它唯一的门禁点，退役时不能顺手丢掉。
//
// 运行： node scripts/check-element-color.mjs
// 自证： node scripts/check-element-color.mjs --self-test
import fs from 'node:fs'

const B = String.fromCharCode(96) // 反引号（本文件里不直接写它，免得引号互咬）

const FILES = {
  placement: 'crates/yekitrim-polish/src/process/placement.rs',
  processMod: 'crates/yekitrim-polish/src/process/mod.rs',
  prompt: 'crates/yekitrim-polish/src/llm/segment.rs',
  types: 'crates/yekitrim-render-core/src/types.rs',
  translator: 'tools/polish-to-dhampir.mjs',
  engine: 'webui/src/polish/engine.js',
  editor: 'webui/src/components/TimelineEditor.vue',
  preview: 'webui/src/components/PolishPreview.vue',
  polishRs: 'crates/yekitrim-server/src/api/polish.rs',
}

const load = () => {
  const out = {}
  for (const [k, rel] of Object.entries(FILES)) out[k] = fs.readFileSync(rel, 'utf8')
  return out
}

/// 从 function 声明起、到下一个缩进两格的 } 之前的源码切片（与其它门禁同手法）。
const sliceFn = (src, marker) => {
  const start = src.indexOf(marker)
  if (start < 0) return ''
  const end = src.indexOf('\n  }', start)
  return src.slice(start, end < 0 ? src.length : end)
}

function checkAll(src) {
  const fails = []
  const warns = []
  let checks = 0
  const check = (name, ok, detail) => {
    checks += 1
    if (!ok) fails.push(name + (detail ? ' :: ' + detail : ''))
  }
  const warn = (name, detail) => warns.push(name + (detail ? ' :: ' + detail : ''))

  // (1) Rust 侧的类型必须删干净，且不许再被 re-export。
  check(
    'Rust 侧 ElementColor 类型已删除（不许复活）',
    !/pub enum ElementColor\b/.test(src.placement) && !/impl ElementColor\b/.test(src.placement),
    'placement.rs 里又出现了 ElementColor —— 它没有生产消费者，复活就是第二条死链路',
  )
  check(
    'process 不再 re-export ElementColor',
    !src.processMod.includes('pub use placement::ElementColor'),
    'process/mod.rs 又把它导出了 —— 通常意味着提示词/解析器的对拍也要跟着复活',
  )

  // (2) 提示词的 sticker 段不许再教颜色（overlay 的 color/colors 是另一回事）。
  check(
    '提示词不再教贴纸颜色（七档教学已删）',
    !src.prompt.includes('七选一') && !src.prompt.includes('color **颜色'),
    '提示词里又出现"七选一" / "color **颜色" —— Event 没有 color 字段，写了必被 serde 丢弃',
  )
  check(
    '提示词里没有色档清单残留',
    !src.prompt.includes(B + 'white' + B + ' 白') && !src.prompt.includes(B + 'pink' + B + ' 粉'),
    '色档清单又回到了提示词 —— 那是退役前那段教学',
  )

  // (3) 转译器：仍收 7/9/10 段（老工程可读），但不再读第 10 段。
  const parseFn = sliceFn(src.translator, 'function parseTextStyle(')
  check('转译器仍有 parseTextStyle（老工程 source 串的解析器）', parseFn.length > 0)
  check(
    '转译器不再产出 colorName（第 10 段只做段数兼容）',
    parseFn.length > 0 && !parseFn.includes('colorName'),
    'parseTextStyle 里又读了第 10 段 —— 颜色链已退役，读出来只会被当成第二份颜色来源',
  )
  const accepted = [...parseFn.matchAll(/parts\.length\s*!==\s*(\d+)/g)].map((m) => Number(m[1]))
  check(
    '转译器仍接受 7/9/10 段（老工程可读）',
    [7, 9, 10].every((n) => accepted.includes(n)),
    '合法段数 = {' + accepted.join(', ') + '} —— 缺的那些会让对应年代的老工程整条字退回兜底样式',
  )

  // (4) 预览：颜色只来自 source 的第 4/5 段 RGBA，不按名字查表。
  check(
    '预览的文字颜色只来自 RGBA 段（fill=p[3] / stroke=p[4]）',
    src.engine.includes('const fill = rgba(p[3])') && src.engine.includes('const stroke = rgba(p[4])'),
    'engine.js 的 parseTextSource 不再从 3/4 段取 RGBA —— 颜色会被别的东西接管',
  )
  check(
    '预览没有颜色预设表（不按名字查表）',
    !/ELEMENT_COLORS|ElementColor|COLOR_PRESETS/.test(src.engine),
    'engine.js 里出现了颜色预设表 —— 那是第二份颜色来源，black 的描边翻转规则必然漏',
  )
  if (src.engine.includes('colorName')) {
    warn(
      'webui/src/polish/engine.js 仍读第 10 段（colorName 死字段）',
      '不消费（只用 RGBA，不查表）⇒ 无行为影响；不在本任务写作用域，建议随前端清理删除',
    )
  }

  // (5) 面板：颜色入口已删（文字层面板没了）。
  check(
    '面板没有颜色下拉（v-for ELEMENT_COLORS）',
    !src.editor.includes('v-for="c in ELEMENT_COLORS"'),
    'TimelineEditor.vue 又渲染了颜色下拉 —— 用户选了也没有任何东西读它',
  )
  check(
    '面板没有「默认色」选项',
    !src.editor.includes('<option value="">默认色</option>'),
    '颜色下拉的「默认色」选项又回来了（与上一条通常成对出现）',
  )
  if (src.editor.includes('ELEMENT_COLORS') || src.editor.includes('ELEMENT_COLOR_VALUES')) {
    warn(
      'webui 仍留有死常量 ELEMENT_COLORS/ELEMENT_COLOR_VALUES',
      '不在本任务写作用域（webui/src 归 Lead 统一处理）；它不是入口、不会渲染，建议随前端清理删除',
    )
  }

  // (6) 老工程仍可读：字段必须在（删了 = 老 polish.toml 直接打不开）。
  check(
    'Event::Element.color 字段仍保留（老文件可反序列化）',
    /color:\s*Option<String>/.test(src.types),
    'render-core 的 Element 变体丢了 color 字段 —— 老工程会被拒绝加载（R4 红线）',
  )

  // (7) 老工程文字轨的侧挂 ASS：三条同源（与颜色无关，但这是本脚本原先唯一的覆盖点）。
  check(
    '转译器给文字轨 ASS 一个独占子目录（与 Rust 清理同一推导）',
    src.translator.includes('const sideDir = join(dirname(docPath),'),
    '找不到 sideDir —— 文字轨 ASS 会退回片段根目录（文件越积越多）',
  )
  const assNeedle = 'const assPath = join(sideDir, ' + B + 'dhampir-$' + '{id}.ass' + B + ')'
  check(
    '转译器把文字轨写成 dhampir-<轨 id>.ass（前缀契约）',
    src.translator.includes(assNeedle),
    '改了这个形状，服务端/前端按 basename 建表与查表就会对不上（预览里那类文字静默消失）',
  )
  {
    const BS0 = String.fromCharCode(92)
    const previewNeedle =
      'String(a.uri || ' + "''" + ').split(/[' + BS0 + BS0 + '/]/).pop()'
    check(
      '前端按 uri 的 basename 查表（与服务端建表口径一致）',
      src.preview.includes(previewNeedle),
      'PolishPreview.vue 不再按 basename 查表 —— 与服务端那份 map 的口径分叉',
    )
  }
  // 必须只看 read_side_subtitles 的函数体（mod tests 里也有同样的表达式）。
  const sideBody = (() => {
    const at = src.polishRs.indexOf('fn read_side_subtitles(')
    if (at < 0) return ''
    const rest = src.polishRs.slice(at)
    const end = rest.indexOf('\n}\n')
    return end < 0 ? rest : rest.slice(0, end)
  })()
  check(
    '取到了 read_side_subtitles 的函数体（判据作用域没落空）',
    sideBody.length > 0,
    '找不到 fn read_side_subtitles( —— 判据会在空串上恒真/恒假',
  )
  const BS = String.fromCharCode(92)
  const BASENAME = '.rsplit([' + "'" + '/' + "'" + ', ' + "'" + BS + BS + "'" + '])'
  check(
    '服务端在 read_side_subtitles 里按 uri 的 basename 建表（与前端同口径）',
    sideBody.includes(BASENAME),
    '函数体里找不到 .rsplit 的那个形状 —— 服务端/前端查表全部落空',
  )

  return { fails, warns, checks }
}

// 自证：把每条退役的链路重新注入，必须红。
function selfTest(base) {
  const clone = () => ({ ...base })
  const cases = [
    ['Rust ElementColor 复活', (s) => { s.placement += '\npub enum ElementColor { White }\n' }],
    ['提示词重新教颜色', (s) => { s.prompt += '\n  color **颜色(可选)**, 七选一\n' }],
    ['转译器重新读第 10 段', (s) => {
      s.translator = s.translator.replace(
        'function parseTextStyle(source, styleName) {',
        'function parseTextStyle(source, styleName) {\n    const colorName = parts[9];',
      )
    }],
    ['预览新增颜色预设表', (s) => { s.engine += '\nconst ELEMENT_COLORS = []\n' }],
    ['面板重新渲染颜色下拉', (s) => { s.editor += '\n<option v-for="c in ELEMENT_COLORS">x</option>\n' }],
    ['Event::Element.color 字段被删', (s) => {
      s.types = s.types.split('color: Option<String>').join('color_removed: bool')
    }],
  ]
  const out = []
  for (const [name, apply] of cases) {
    const s = clone()
    apply(s)
    const { fails } = checkAll(s)
    out.push({ name, caught: fails.length > 0 })
  }
  return out
}

const base = load()
if (process.argv.includes('--self-test')) {
  const res = selfTest(base)
  let bad = 0
  for (const r of res) {
    if (!r.caught) bad += 1
    console.log((r.caught ? '  ok   ' : '  FAIL ') + r.name + (r.caught ? ' -> 红（判据有效）' : ' -> **没红**（判据空转）'))
  }
  console.log(bad === 0
    ? '颜色链退役判据：' + res.length + ' 处「复活」全部被抓'
    : '颜色链退役判据：' + bad + '/' + res.length + ' 处复活没被抓住')
  process.exit(bad === 0 ? 0 : 1)
}

const { fails, warns, checks } = checkAll(base)
for (const w of warns) console.log('  WARN ' + w)
if (fails.length === 0) {
  console.log('颜色链已退役（' + checks + ' 项判据）：类型 / 提示词 / 第 10 段 / 预览查表 / 面板入口全部不在；老工程仍可读')
  process.exit(0)
}
console.log('颜色链退役判据失败（' + fails.length + ' / ' + checks + '）：')
for (const f of fails) console.log('    ' + f)
process.exit(1)
