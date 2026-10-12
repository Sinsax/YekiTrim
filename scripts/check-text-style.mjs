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

console.log(`\n${fails.length ? `${fails.length} 条不通过` : '全部通过'}`)
process.exit(fails.length ? 1 : 0)
