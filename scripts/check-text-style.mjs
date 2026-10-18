#!/usr/bin/env node
// §5 E3：守卫「文字样式只有一份真源」。
//
// ⚠️ 这条门禁存在的理由（2026-10-11）：
// 四档样式的数值（字号/描边/颜色/字重）本来会**两边各存一份** ——
// Rust 那份算盒子、JS 那份出图。分叉的症状是
// **「算出来的盒子与画出来的字对不上」**（文字溢出自家的底衬色块），
// 而**两边单测各自都过** —— 这正是本仓反复踩的那类「看起来成功、其实没验」。
//
// 判据：转译器**不许**再出现四档的**数值表**，只许出现档位**名字**
// （`hook`/`chapter`/`comment`/`count`）与解析器。
import fs from 'node:fs'

const TRANS = 'tools/polish-to-dhampir.mjs'
const RUST = 'crates/yekitrim-polish/src/process/mod.rs'

const s = fs.readFileSync(TRANS, 'utf8')
const rust = fs.readFileSync(RUST, 'utf8')
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

// ④ `source` 写回格式：Rust 侧与 JS 侧的**字段数**必须一致
//
// ⚠️ 这是最容易静默分叉的一处：Rust 加一个字段、JS 还按 6 段解析 ⇒
// `parts.length !== 6` 命中兜底 ⇒ **所有文字都退到 comment 档**，
// 而报告里只有一行「样式参数没拿到」，很容易被当成偶发。
const m = /format!\(\s*"([^"]*\|[^"]*)"\s*,\s*style_name/.exec(rust)
check(
  'Rust 侧写回的格式串找得到',
  m !== null,
  'mod.rs 里没找到那条 `format!` —— 写回路径可能被改掉了',
)
if (m) {
  const rustFields = m[1].split('|').length
  // ⚠️ **只在 `parseTextStyle` 函数体内找** —— 裸匹配 `parts.length !== N`
  // 会先撞上文件别处的同名判据（实测撞到第 304 行的 `!== 2`），
  // 于是门禁报"两侧不一致"，而真正的原因是我的正则范围太宽。
  // 判据必须**定位到那个函数**，不是"文件里某处有个数字"。
  const fnStart = s.indexOf('function parseTextStyle(')
  const fnEnd = fnStart < 0 ? -1 : s.indexOf('\n  }', fnStart)
  const body = fnStart < 0 ? '' : s.slice(fnStart, fnEnd < 0 ? s.length : fnEnd)
  // ⚠️⚠️ **JS 侧合法段数是【一个集合】，不是单个数字**（2026-10-13 修，判据腐烂）。
  //
  //    `T3` 加 `tilt`/`squash` 时用的是**向后兼容**的写法：
  //        if (parts.length !== 7 && parts.length !== 9) { /* 兜底 */ }
  //    —— 7 段（老工程）与 9 段（T3 之后）**都是合法的**。
  //
  //    而旧判据的 `/parts\.length\s*!==\s*(\d+)/` 只抓**第一个**数字（7），
  //    再拿它跟 Rust 的 9 比 ⇒ 报「Rust 写 9 段，JS 按 7 段解析 —— 永远命中兜底」。
  //    **那句结论是错的**：9 段明明在生产里正常工作（T3 的判据全绿、真机出片过）。
  //    症状是**门禁恒红**，而恒红的门禁比没有门禁更坏 ——
  //    它会训练人"这条红了是正常的"，于是真正腐烂的那天也没人看。
  //
  //    ⇒ 判据改成：**Rust 写的段数必须落在 JS 接受的集合里**。
  //      这样才能同时表达"老段数仍兼容"与"新段数已被认识"两件事；
  //      真要收紧（比如某天不再兼容 7 段）时，集合自然只剩一个数。
  const accepted = [...body.matchAll(/parts\.length\s*!==\s*(\d+)/g)].map((x) => Number(x[1]))
  const jsOk = accepted.length > 0 && accepted.includes(rustFields)
  check(
    `两侧字段数一致（Rust ${rustFields} 段 ∈ JS 接受 {${accepted.join(', ')}}）`,
    jsOk,
    accepted.length === 0
      ? '`parseTextStyle` 里找不到 `parts.length !== N` —— 它没有校验段数'
      : `Rust 写 ${rustFields} 段，而 JS 只接受 {${accepted.join(', ')}} —— ` +
        '落在集合外就**永远命中兜底**（所有文字退到 comment 档）',
  )
}

// ⑤ 认不出时必须**报出来**（R1：不许静默降级）
check(
  '样式认不出时会打一行 note（不静默降级）',
  s.includes('样式参数没拿到') || s.includes('style = "'),
  '解析失败没有对应的 note —— 降级会静默发生',
)

// ⑥ 文字落点必须走**动态盒**求解器（`pick_box_position`），不许用贴纸那条
//    `pick_sticker_position` —— 后者的盒子写死 280 方块。
//
// ⚠️ 这条是 E3 的**核心不变式**，而且**单测抓不住它**：
// 改回 `pick_sticker_position` 之后单测全绿（那函数本身是对的、只是盒子不对），
// 症状是"三行大字按一个 280 方块去避让 ⇒ 压住主播脸"，而且**不报错**。
// 实测：这个变异在我的 Rust 用例下**一条都没红**，所以必须在这里拦。
const modSrc = fs.readFileSync('crates/yekitrim-polish/src/process/mod.rs', 'utf8')
// ⚠️ 2026-10-15：调用点改成了 pick_box_position_pref（多一个"纵向偏好"实参 —— 不传 pos
//    的话三档对成片零影响，见 mod.rs 那段注释）。写死旧签名会让这条**永久变红**，
//    而一条不断红的判据等于没有判据（它会把真正报警的那几条一起埋掉）。
//    ⇒ 判据放宽成"两种都认"，仍然钉住的本意：**走动态盒求解器、不是贴纸那条**。
const dynBoxCall = modSrc.includes('pick_box_position_pref(') || modSrc.includes('pick_box_position(')
const passesBox = /pick_box_position(_pref)?\(\s*&blocked,\s*&obstacles\.allow,\s*seed,\s*f,\s*&b/.test(modSrc)
check(
  '文字落点走 pick_box_position(_pref)（动态盒），不是 pick_sticker_position',
  dynBoxCall && passesBox,
  'mod.rs 里找不到去重后的动态盒调用（pick_box_position[_pref](&blocked, &obstacles.allow, seed, f, &b)）—— 文字可能退回成固定 280 盒避让',
)
check(
  '文字落点把排版出来的盒子传进去了',
  /TextLayout::solve\([^)]*\)[\s\S]{0,240}box_size\(\)/.test(modSrc),
  '找不到 `TextLayout::solve(...)` 后面紧跟 `box_size()` —— 算出来的盒子没进求解器',
)

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
// ⑦ **字号真源只有一处：Rust 的 `ELEMENT_FONT_PX`**（2026-10-13，字号统一 80）。
//
// ⚠️ 这条补的是 ②「四档数值表只有一处」**管不到的那一半**：
// ② 管的是"转译器里不许出现第二份**表**"，但字号在 JS 侧还有**两个兜底标量**
// （转译器 `ELEMENT_FONT_PX`、预览 `engine.js::parseTextSource` 的 `num(p[1], 80)`）——
// 它们**不是表**，② 看不见它们。
//
// 症状：工程没跑过 `process`（或样式串坏了）时，那两处才会被读到 ⇒
// **同一个 `polish.toml`，跑过 process 的与没跑的画出不同字号**，
// 而两边各自都不报错、单测各自都过。本仓"两份同一个事实必然漂移"的经典形状。
//
// ⚠️ 判据必须**在 Rust 侧读出那个值**再逐个比对，**不能三处各写一个 80** ——
// 那样"三处同时漂到 99"照样绿（= 判据空转）。
const fontConst = /pub const ELEMENT_FONT_PX: f64 = ([\d.]+);/.exec(
  fs.readFileSync('crates/yekitrim-polish/src/process/placement.rs', 'utf8'),
)
check(
  'Rust 侧有 ELEMENT_FONT_PX 常量（字号唯一真源）',
  fontConst !== null,
  'placement.rs 里找不到 `pub const ELEMENT_FONT_PX: f64 = …` —— 字号又变回散落的字面量了',
)
if (fontConst) {
  // ⚠️ **按数值比、不按字符串比**：Rust 侧写作 `80.0`、JS 侧写作 `80` ——
  //    字符串比较会把一对**完全同值**的常量报成分叉（判据恒红 = 比没有还坏，
  //    它会训练人"这条红了正常"）。
  const want = Number(fontConst[1])
  // 转译器表里的四档 fontPx 必须**全等于**它。
  const transFonts = [...s.matchAll(/(?:hook|chapter|comment|count):\s*\{\s*fontPx:\s*([\d.]+)/g)].map(
    (m) => Number(m[1]),
  )
  check(
    `转译器兜底表的四档字号全等于 Rust 的 ${want}`,
    transFonts.length === 4 && transFonts.every((v) => v === want),
    `转译器兜底四档字号 = {${transFonts.join(', ')}}，Rust = ${want} —— ` +
      '没跑过 process 的工程会画出不同字号（且不报错）',
  )
  // 转译器那个独立常量（`const ELEMENT_FONT_PX = N`）也要同值。
  const transConst = /const ELEMENT_FONT_PX = ([\d.]+)/.exec(s)
  check(
    `转译器 ELEMENT_FONT_PX 与 Rust 同值（${want}）`,
    transConst !== null && Number(transConst[1]) === want,
    transConst === null
      ? '转译器里找不到 `const ELEMENT_FONT_PX = …`'
      : `转译器写 ${transConst[1]}，Rust 写 ${want}`,
  )
  // 预览 engine 的解析兜底 `num(p[1], N)` 也要同值。
  const engine = fs.readFileSync('webui/src/polish/engine.js', 'utf8')
  const engineFallback = /fontPx:\s*num\(\s*p\[1\]\s*,\s*([\d.]+)\s*\)/.exec(engine)
  check(
    `预览 engine 的 fontPx 兜底与 Rust 同值（${want}）`,
    engineFallback !== null && Number(engineFallback[1]) === want,
    engineFallback === null
      ? 'engine.js 的 parseTextSource 里找不到 `fontPx: num(p[1], N)`'
      : `engine 写 ${engineFallback[1]}，Rust 写 ${want} —— 预览与成片字号分叉`,
  )
}

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
