#!/usr/bin/env node
// §5 E3 的门禁，**已随文字层退役（2026-10-20 / 2026-10-30 收尾）缩表**。
//
// # 退役了什么（为什么）
//
// 文字层整条退役：`Event::Element{kind:"text"}` 读时丢弃，写入方
// `layout_text_elements` / style_name / `TextLayout` / `pick_box_position` /
// Rust `ELEMENT_FONT_PX` / `TextStyle` **全部删除**（颜色链 2026-10-30 随之退役）。
// 所以下面三条判据**结构上不可能再通过**，留着就是恒红门禁（比没有门禁更坏）：
//   * Rust 侧 `format!(…, style_name)` 写回串 —— 写入方已删；
//   * 文字落点走 `pick_box_position(_pref)` + `TextLayout::solve` —— 函数已删；
//   * Rust `ELEMENT_FONT_PX` 作为字号真源 —— 常量已删。
//
// # 现在还锁什么（都是**活着的**契约）
//
// 1. 老工程的 `source` 串仍被转译器按位置解析 ⇒ 合法段数（7/9/10）与"没有第二份表"
//    仍是**读兼容**的硬约束；颜色**预设名**（第 10 段）已退役，解析器不许再带出
//    `colorName`；
// 2. 字幕/弹幕/外圈/白环/卡片底衬/字体族名这些**仍在生产**的跨端数值必须同源；
// 3. 文字轨的 ASS 描边（`outlinePx`）必须真的写进产物 —— 这条与"退役"无关，
//    是老工程出片唯一还在走的文字路径。
//
// ⚠️ 判据不许为了绿而删：本文件每一条都对应一次真实事故（注释里写着是哪一次）。
import fs from 'node:fs'

const TRANS = 'tools/polish-to-dhampir.mjs'

const s = fs.readFileSync(TRANS, 'utf8')
const fails = []

function check(name, ok, detail = '') {
  if (ok) console.log(`  ok   ${name}`)
  else {
    console.log(`  FAIL ${name}${detail ? `\n         ${detail}` : ''}`)
    fails.push(name)
  }
}

// ① 必须有解析器（读 process 写回的串）
check(
  '转译器有 parseTextStyle（读 process 写回的样式串）',
  s.includes('function parseTextStyle('),
  '找不到 parseTextStyle —— 样式串没人解析，四档要么没实现要么又变回硬编码',
)

// ② 不许出现第二份「四档数值表」—— 那是分叉的根源
const tables = [...s.matchAll(/hook\s*:\s*\{\s*fontPx\s*:/g)]
check(
  '四档数值表只有一处（兜底那份）',
  tables.length <= 1,
  `找到 ${tables.length} 处 \`hook: { fontPx:\` —— 多出来的那几处就是「同一组数值的第二份拷贝」`,
)

// ③ 兜底表必须**自称兜底**（否则下一个人会以为是真源，往它里面改数值）
if (tables.length === 1) {
  const idx = s.indexOf('TEXT_STYLE_FALLBACK')
  const window = s.slice(Math.max(0, idx - 1200), idx + 300)
  check(
    '兜底表附近写明了「正常路径不读这里」',
    window.includes('正常路径不读') || window.includes('兜底'),
    '兜底表旁边没有说明 —— 下一个人会把它当真源往里面改数值',
  )
}

// ④ 老工程 `source` 串的**合法段数**（只读兼容）。
//
// 写入方（`layout_text_elements`）已随文字层退役删除（2026-10-20），Rust 侧不再有
// 格式串可对拍 ⇒ 这条从"两侧字段数一致"退化成"解析器仍收得下历史三档段数"。
// 7 = T3 之前、9 = T3 之后、10 = 颜色预设那一代（2026-10-13 ~ 退役）。
// ⚠️ 只认一档的话，老工程整条字会**退回兜底样式**（字号/颜色全变、报告只有一行 note）。
const parseFnStart = s.indexOf('function parseTextStyle(')
const parseFnEnd = parseFnStart < 0 ? -1 : s.indexOf('\n  }', parseFnStart)
const body = parseFnStart < 0 ? '' : s.slice(parseFnStart, parseFnEnd < 0 ? s.length : parseFnEnd)
const accepted = [...body.matchAll(/parts\.length\s*!==\s*(\d+)/g)].map((x) => Number(x[1]))
check(
  'parseTextStyle 仍接受 7/9/10 段（老工程 source 串可读）',
  [7, 9, 10].every((n) => accepted.includes(n)),
  accepted.length === 0
    ? '`parseTextStyle` 里找不到 `parts.length !== N` —— 它没有校验段数'
    : `合法段数 = {${accepted.join(', ')}}，缺 {${[7, 9, 10].filter((n) => !accepted.includes(n)).join(', ')}} —— 那些老工程会整条退回兜底样式`,
)
// 颜色**预设名**（第 10 段）已退役：解析器只认段数、不许再把名字带进对象。
check(
  'parseTextStyle 不再产出 colorName（颜色链已退役）',
  !body.includes('colorName'),
  'parseTextStyle 里又出现了 `colorName` —— 第 10 段只允许用于"段数兼容"，读出来就是第二条死链路',
)

// ⑤ 认不出时必须**报出来**（R1：不许静默降级）
check(
  '样式认不出时会打一行 note（不静默降级）',
  s.includes('样式参数没拿到') || s.includes('style = "'),
  '解析失败没有对应的 note —— 降级会静默发生',
)

// ⑥ **文字落点/排版那套判据已退役**（2026-10-20 文字层删除）。
//
// 原来这里钉两件事：① 落点走 `pick_box_position(_pref)` 动态盒（不是贴纸的固定 280 盒）；
// ② `TextLayout::solve` 的结果真的进了求解器。两个函数都随文字层一起删了，
// 判据结构上不可能通过（恒红门禁 = 比没有更坏）。
// 仍活着的那一半（贴纸落点必须逐像素不变）在 Rust 单测
// `placement::tests::sticker_path_is_byte_identical_after_dynamic_box_refactor` 里。

// ⑥c **卡片底衬的色值两处同源**（2026-10-16）：`chapter`/`comment` 的底衬由 `process` 写回
//     `source` 第 7 段（backdrop），**成片那条腿**（转译器）与 **DOM 预览**（engine.js）各读一次；
//     两处字面量不一致 = 「预览有底衬、成片没有」（或反过来），而两边各自看都正常。
const engineSrc = fs.readFileSync('webui/src/polish/engine.js', 'utf8')
const plateInTranslator = /plate_color:\s*\[0,\s*0,\s*0,\s*140\]/.test(s)
const plateInEngine = /PLATE_CARD_COLOR\s*=\s*\[0,\s*0,\s*0,\s*140\]/.test(engineSrc)
check(
  '卡片底衬的色值两处同值（黑 55%）',
  plateInTranslator && plateInEngine,
  '转译器与 engine.js 里的卡片底衬色必须是同一对值（plate_color: [0,0,0,140]）—— ' +
    '只改一处 = 预览与成片分叉，而两边各自看都正常',
)

// ⑥d **字幕样式的两处默认值**（2026-10-16）：工程**没写**这两个键时，
//     Rust（`default_sub_stroke()` / `default_sub_outer_stroke()`）与转译器的兜底
//     （`parseStroke(subStyle.stroke, 12, ...)` / `DEFAULT_OUTER_STROKE`）各给一份 ——
//     两处不一致的症状是「没写这个键的工程：预览一套、成片另一套」，而各自看都正常。
const subStyleSrc = fs.readFileSync('crates/yekitrim-render-core/src/types.rs', 'utf8')
const rustStroke = /fn default_sub_stroke\(\) -> String \{\s*"([^"]+)"/.exec(subStyleSrc)
const rustOuter = /fn default_sub_outer_stroke\(\) -> String \{\s*"([^"]+)"/.exec(subStyleSrc)
check(
  '字幕描边默认值两处同源（12px #403c3b）',
  !!rustStroke && rustStroke[1] === '12px #403c3b' && /parseStroke\(subStyle\.stroke, 12, '#403c3b'\)/.test(s),
  'Rust default_sub_stroke() 与转译器 parseStroke 的兜底必须同值（实得 Rust ' + (rustStroke && rustStroke[1]) + '）',
)
check(
  '外圈描边默认值两处同源（auto #ffffff = 跟随第一层）',
  !!rustOuter && rustOuter[1] === 'auto #ffffff' && /DEFAULT_OUTER_STROKE = 'auto #ffffff'/.test(s),
  'Rust default_sub_outer_stroke() 与转译器 DEFAULT_OUTER_STROKE 必须同值（实得 Rust ' + (rustOuter && rustOuter[1]) + '）',
)
// ⑥e **弹幕的两个默认值两处同源**（2026-10-16）：工程**没写** `font_size` / `stroke` 时，
//     **成片那条腿**（转译器 `cuesToAss` 的 `fontSize = 42` / `dmSizePx` 的 `?? 42`）与
//     **DOM 预览**（`engine.js` 的 `fontSize || 42` / `dmFontSize` 的 `|| 42`）各给一份 ——
//     漂了的症状是"预览与成片的弹幕字号/描边不一样"，而两边各自看都正常（老工程最容易命中：
//     它们大多没写这两个键）。
const engineSrcDm = fs.readFileSync('webui/src/polish/engine.js', 'utf8')
const dm42 = (engineSrcDm.match(/\|\| 42\b/g) || []).length
check(
  '弹幕字号默认值两处同源（42）',
  /fontSize = 42/.test(s) && /dmSizePx[^\n]*: 42\b/.test(s) && dm42 >= 2,
  '转译器（cuesToAss 默认参数 42 / dmSizePx 的 ?? 42）与 engine.js（|| 42 ×2 处）必须同值：' +
    '实得 engine 侧 ' + dm42 + ' 处 42',
)
// ⑥f **粗体字面的命名规则两处同源**（2026-10-16）：Rust `find_font` 在库里按
//     `format!("{family} Bold")` 找粗体那一支；前端（字体库的「无粗体」提示）必须用同一条后缀，
//     两边不一致 = 界面与底座对"这支有没有粗体"的判断相反 —— 而两边各自看都正常。
const dhampirSrc = fs.readFileSync('crates/yekitrim-polish/src/render/dhampir.rs', 'utf8')
const fontJsSrc = fs.readFileSync('webui/src/polish/fonts.js', 'utf8')
const rustBoldSuffix = dhampirSrc.indexOf(String.raw`{family} Bold`) >= 0
const jsBoldSuffix = fontJsSrc.indexOf('${fam.toLowerCase()} bold') >= 0
check(
  '粗体字面的命名规则两处同源（族名 + " Bold"）',
  rustBoldSuffix && jsBoldSuffix,
  'Rust find_font 按 `<族名> Bold` 找粗体、前端 hasBoldFace 必须同后缀（实得 Rust ' +
    rustBoldSuffix + ' / JS ' + jsBoldSuffix + '）',
)
// ⑥h 需要 `findFontInLibrary`（同一个文件里的邻居函数）—— 同样按括号配平扒出来，不抄第二份。
const findInLibrary = (() => {
  const s2 = fs.readFileSync('webui/src/polish/fonts.js', 'utf8')
  const start2 = s2.indexOf('function findFontInLibrary')
  if (start2 < 0) return null
  let depth = 0
  for (let i = s2.indexOf(String.fromCharCode(123), start2); i < s2.length; i += 1) {
    if (s2[i] === String.fromCharCode(123)) depth += 1
    else if (s2[i] === String.fromCharCode(125)) { depth -= 1; if (depth === 0) return new Function('return (' + s2.slice(start2, i + 1) + ')')() }
  }
  return null
})()
// ⑥h **换字体的族名换算两处同源**（2026-10-16，用户报「更改字体时没有正常切换字体」）：
//     预览注册 FontFace / 底座 canvas / 出片用的都是转译产物里的 `font_family`
//     （= 宿主的 id→族名映射算出来的**内部族名**），而 DOM 的 CSS 用的是 `[style.*].font`
//     **原值**（往往是库 id）。两者在 id≠族名时（自带 `NotoSansSC-Regular` / 族名
//     `Noto Sans SC` 就是）对不上 ⇒ 浏览器**静默回退系统字体** ⇒「换字体没反应」。
//     ⚠️ 这里**跑真函数**：从 `fonts.js` 扒出 `fontFamilyFor` 喂一条合成清单。
const fontSrc = fs.readFileSync('webui/src/polish/fonts.js', 'utf8')
const fnStart = fontSrc.indexOf('function fontFamilyFor')
let fnEnd = -1
if (fnStart >= 0) {
  let depth = 0
  for (let i = fontSrc.indexOf(String.fromCharCode(123), fnStart); i < fontSrc.length; i += 1) {
    if (fontSrc[i] === String.fromCharCode(123)) depth += 1
    else if (fontSrc[i] === String.fromCharCode(125)) { depth -= 1; if (depth === 0) { fnEnd = i + 1; break } }
  }
}
const familyFor = fnEnd > 0
  ? new Function('findFontInLibrary', 'return (' + fontSrc.slice(fnStart, fnEnd) + ')')(findInLibrary)
  : null
check(
  '换字体的族名换算：库内 id → 内部族名（DOM 与底座/成片同源）',
  !!familyFor && familyFor([{ id: 'NotoSansSC-Regular', family: 'Noto Sans SC' }], 'NotoSansSC-Regular') === 'Noto Sans SC' &&
    familyFor([{ id: 'NotoSansSC-Regular', family: 'Noto Sans SC' }], 'LXGW WenKai') === 'LXGW WenKai' &&
    familyFor([{ id: 'A', family: 'B' }], '') === '',
  'DOM 预览必须把 `[style.*].font`（库 id）换成内部族名，库外原样 —— 否则注册的 FontFace 与 CSS 对不上，' +
    '浏览器静默回退系统字体（症状：换字体没反应），实得 ' + (familyFor ? familyFor([{ id: 'NotoSansSC-Regular', family: 'Noto Sans SC' }], 'NotoSansSC-Regular') : '<扒不到函数>'),
)
// ⑥g **白环厚度系数两处同源**（2026-10-16）：`auto` 那一档的「粗细跟随第一层」由一个系数决定 ——
//     成片/底座预览走转译器的 `OUTER_RING_FOLLOW`，DOM 预览走 `PolishPreview.vue` 里那份镜像。
//     漂了的症状是「预览白边比成片粗/细」，而两边各自看都正常（本仓的老形状）。
const previewSrc = fs.readFileSync('webui/src/components/PolishPreview.vue', 'utf8')
const translatorFollow = /export const OUTER_RING_FOLLOW = ([\d.]+)/.exec(s)
const domFollow = /const OUTER_RING_FOLLOW = ([\d.]+)/.exec(previewSrc)
check(
  '白环厚度系数两处同源（OUTER_RING_FOLLOW）',
  !!translatorFollow && !!domFollow && translatorFollow[1] === domFollow[1],
  '转译器与 PolishPreview.vue 的 OUTER_RING_FOLLOW 必须同值（实得 转译器 ' +
    (translatorFollow && translatorFollow[1]) + ' / DOM ' + (domFollow && domFollow[1]) + '）',
)
// ⑦ **字号兜底的两份 JS 实现必须同值**（Rust 真源已随文字层退役删除）。
//
// 现在只剩两处会在"工程没跑过 process / 样式串坏了"时被读到：
//   转译器 `TEXT_STYLE_FALLBACK` 的四档 fontPx（+ 独立常量 `ELEMENT_FONT_PX`），
//   预览 `engine.js::parseTextSource` 的 `num(p[1], N)` 兜底。
// ⚠️ **互相之间比**（不写死 80）：只跟一个字面量比的话，"四处同时漂到 99"照样绿。
const transFonts = [...s.matchAll(/(?:hook|chapter|comment|count):\s*\{\s*fontPx:\s*([\d.]+)/g)].map((m) => Number(m[1]))
const transConst = /const ELEMENT_FONT_PX = ([\d.]+)/.exec(s)
const engine = fs.readFileSync('webui/src/polish/engine.js', 'utf8')
const engineFallback = /fontPx:\s*num\(\s*p\[1\]\s*,\s*([\d.]+)\s*\)/.exec(engine)
const fallbacks = [
  ...transFonts,
  transConst ? Number(transConst[1]) : NaN,
  engineFallback ? Number(engineFallback[1]) : NaN,
]
check(
  '字号兜底四处同值（转译器四档 + 独立常量 + 预览解析默认）',
  transFonts.length === 4 && fallbacks.every((v) => Number.isFinite(v) && v === fallbacks[0]),
  '兜底字号不一致：{转译器表=' + transFonts.join(',') +
    '，转译器常量=' + (transConst ? transConst[1] : '缺') +
    '，预览=' + (engineFallback ? engineFallback[1] : '缺') +
    '} —— 没跑过 process 的老工程预览/成片字号会分叉',
)

// ⑧ ⚠⚠ **描边宽必须真的进 ASS**（2026-10-14，用户报的问题 ③）。
//
// ①~⑦ 全部绿、而用户仍然看到「四档描边一样粗」—— 因为那些判据只钉了
// **`strokePx` 这个值对不对**（`TEXT_STYLE_FALLBACK` 与 Rust 同值），
// **没有任何一条钉”它有没有被用上”**。
// 实测：`cuesToAss` 的 ASS `Outline` 写死成字幕默认的 `2`，四个档位全出 `Outline=2`
// —— 而本文件的 ①~⑦ **一条都不红**。
//
// ⇒ 判据必须落到**产物那一侧**：元素调用点要把 `st.strokePx` 换算后传进去。
{
  // ① 元素调用点必须传 `outlinePx`（且来自 `st.strokePx`）。
  check(
    '元素 ASS 必须把 `st.strokePx` 传进 `cuesToAss`（否则四档描边一样粗）',
    /outlinePx: strokePxToAss\(\s*st\.strokePx\b/.test(s),
    '找不到 `outlinePx: strokePxToAss(st.strokePx, …)` —— ' +
    '`strokePx` 算出来了却没人读，ASS 会用写死的默认值',
  )
  // ② `cuesToAss` 必须真的把它写进 `Outline` 那一列。
  check(
    '`cuesToAss` 必须把 `outlinePx` 写进 ASS 的 Outline 列',
    /\$\{outlinePx/.test(s),
    'ASS 的 `Style:` 行里看不到 `${outlinePx` —— 传进来了但没写出去',
  )
  // ③ 换算必须是 `/2`（外侧宽 → ASS 单侧宽），与契约路径 `stroke_ratio` **同一口径**。
  check(
    '`strokePxToAss` 的换算与 `stroke_ratio` 同口径（都是 `/2`）',
    /return v \/ 2\b/.test(s) && /stroke_ratio:\s*st\.strokePx \/ 2 \/ /.test(s),
    '两处换算不一致 —— 同一次编辑里选的描边，预览与成片粗细会不一样',
  )
  // ④ ⚠ 反面：不许再出现写死的 `Outline` 数字（`1,2,1,` 那一段）。
  check(
    'ASS 的 Outline/Shadow 不许写死成字面量',
    !/'0,0,0,0,100,100,0,0,1,2,1,2,10,10,10,1'/.test(s),
    '还是写死的 `…,1,2,1,2,10,10,10,1` —— 那是“文字元素描边取字幕默认值”的原样',
  )
}

console.log(`\n${fails.length ? `${fails.length} 条不通过` : '全部通过'}`)
process.exit(fails.length ? 1 : 0)
