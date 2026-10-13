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
check(
  '文字落点走 pick_box_position（动态盒），不是 pick_sticker_position',
  modSrc.includes('pick_box_position(&blocked, &obstacles.allow, seed, f, &b)'),
  'mod.rs 里找不到 `pick_box_position(...&b)` —— 文字可能退回成"按 280 固定盒避让"',
)
check(
  '文字落点把排版出来的盒子传进去了',
  /TextLayout::solve\([^)]*\)[\s\S]{0,240}box_size\(\)/.test(modSrc),
  '找不到 `TextLayout::solve(...)` 后面紧跟 `box_size()` —— 算出来的盒子没进求解器',
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

console.log(`\n${fails.length ? `${fails.length} 条不通过` : '全部通过'}`)
process.exit(fails.length ? 1 : 0)
