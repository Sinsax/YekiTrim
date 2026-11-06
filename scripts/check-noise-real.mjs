#!/usr/bin/env node
// §20 守卫：`noise` 必须走**真黑白噪点**，不许退回"白色 flash / 白纱"。
//
// 为什么值得一条独立门禁（2026-10-11）：
// 这个特效**名字叫 noise、画的一直是白纱**，而且**两处实现各自为政** ——
// 转译器发白色 `flash`、出片模板铺 `#ffffff`、只有 DOM 预览有真噪声（另一张 SVG）。
// 三者**都不报错**，画面"看起来有东西在闪"，所以这件事在成片里**看不出来**。
// 用户拍板改真噪声之后，任何一处退回白纱都会让"预览 ≠ 成片"重新出现。
//
// ============================================================================
// ⚠️ 2026-10-30 口径更新：**`case 'noise'` 已删，noise 走声明式派发**
// ============================================================================
//
// 旧断言（已退役）：在 `tools/polish-to-dhampir.mjs` 里找 `case 'noise': { ... }`，
//   要求它含 `kind: 'noise'` + 固定 `seed`。
//   为什么退役：P2 把 `vignette` / `flash` / `hue_shift` / `noise` 四条 case 搬进了
//   `spec/effects.json` 的 `dhampir_rule`（转译器 5320 行那段注释写着"别加回来"），
//   现在由**声明式派发器**按 `rule.kind` + `rule.params` 生成。
//   ⇒ 旧断言在"产品已经改好"的情况下**永远红**，而且它红的时候指错了地方。
//
// 现在锁什么（两条都要，缺一条就抓不住"白纱回来了"）：
//   ① **声明**：`spec/effects.json` 的 noise 必须 `dhampir_rule.kind === 'noise'`、
//      `params.seed` 是**固定整数**、**不许**出现白色 flash（`kind:'flash'` + `r/g/b:1`）。
//   ② **派发**：转译器里必须真的按声明求值（`rule.kind` / `rule.params` / `placeEnvelope`），
//      且**不许把 `case 'noise'` 加回来**（两处都算 = "改了词汇表没生效"，报告里一行差异都没有）。
//   ③ 出片模板仍是灰度噪声瓷砖（这条没变）。
//
// 反向/变异判据（`--self-test`）：把 kind 改成 flash + 白色 / 拿掉 seed /
// 把 `case 'noise'` 加回来 / 模板用 `#ffffff` 铺满 —— 每一种都必须让判据**翻红**。
import fs from 'node:fs'

let fail = 0
let checks = 0
const check = (name, ok, detail) => {
  checks += 1
  if (ok) console.log(`  ok   ${name}`)
  else { console.log(`  FAIL ${name}${detail ? `\n         ${detail}` : ''}`); fail += 1 }
}

const stripLine = new RegExp('(^|[^:])//[^\\n]*', 'g')
const stripComments = (src) => src.replace(/\/\*[\s\S]*?\*\//g, '').replace(stripLine, '$1')

/**
 * 对**三份输入**跑全部判据（主流程传文件内容；`--self-test` 传变异体）。
 * ⚠️ 抽成函数是为了让自证跑的是**同一份判据**（只写在顶层的话变异台根本不会跑它）。
 */
function analyze({ spec, trans, tpl }) {
  const out = []
  const add = (name, ok, detail) => out.push({ name, ok, detail })

  // ---------------------------------------------------------------------
  // ① 声明（spec/effects.json）
  // ---------------------------------------------------------------------
  const noise = (spec.effects || []).find((e) => e.id === 'noise')
  const rule = noise?.dhampir_rule ?? {}
  add(
    '词表的 noise 走声明式 `kind: noise`（不是白色 flash）',
    rule.kind === 'noise',
    `dhampir_rule.kind = ${JSON.stringify(rule.kind)} —— 那是退回白纱了（P2 之后 noise 由声明驱动）`,
  )
  add(
    '词表的 noise 映射指向 ColorMask::noise',
    /ColorMask::noise/.test(noise?.dhampir || ''),
    `词表里写的是 ${noise?.dhampir}`,
  )
  // 固定 seed：噪声必须是帧号的函数（用随机会让预览与成片逐帧对不上）。
  add(
    '词表的 noise 带固定整数 seed（噪声必须是帧号的函数，不能用随机）',
    Number.isInteger(rule.params?.seed) && rule.params.seed >= 0,
    `params.seed = ${JSON.stringify(rule.params?.seed)} —— 没有固定 seed 会让同一帧两次渲染不同`,
  )
  // 白色 flash 的形态：kind:'flash' + r/g/b 全 1（旧白纱）。
  const p = rule.params ?? {}
  const whiteFlash = rule.kind === 'flash' && p.r === 1 && p.g === 1 && p.b === 1
  add(
    '词表的 noise 不许退回"白色 flash"',
    !whiteFlash,
    'dhampir_rule 是 kind:flash + r/g/b:1 —— 那就是白纱',
  )

  // ---------------------------------------------------------------------
  // ② 派发（转译器）
  // ---------------------------------------------------------------------
  const body = stripComments(trans)
  add(
    '转译器有声明式派发器（按 rule.kind + rule.params 求值）',
    /const eff = \{ kind: rule\.kind, params \};/.test(body) && /placeEnvelope\(startFrame, seconds, eff/.test(body),
    '找不到 `const eff = { kind: rule.kind, params }` / `placeEnvelope(...)` —— 声明了却没人执行，特效一个都不转',
  )
  add(
    '转译器**没有**把 `case \'noise\'` 加回来（P2 之后它住词汇表）',
    !/case 'noise':/.test(body),
    '又出现了手写 `case \'noise\'` —— 与声明式派发两处都算，改了词汇表不生效且报告里无差异',
  )

  // ---------------------------------------------------------------------
  // ③ 出片模板：canvas 路径不许把 noise 画成纯白铺满
  // ---------------------------------------------------------------------
  const noiseDraw = tpl.slice(
    tpl.indexOf('// ---- noise overlay'),
    tpl.indexOf('// ---- flash ----'),
  )
  // ⚠️ **先把注释剥掉再判** —— 那段注释里为了说明历史**故意引用了** `fillStyle='#ffffff'`，
  //    不剥的话这条门禁会被自己的说明文字打红（我第一版就是这样）。
  const noiseCode = stripComments(noiseDraw)
  add(
    '出片模板不再用 #ffffff 铺满表达 noise',
    !/fillStyle\s*=\s*'#ffffff'/.test(noiseCode),
    'canvas 的 noise 分支又变回"纯白薄纱"了',
  )
  add(
    '出片模板用的是灰度噪声瓷砖',
    /noiseTile\(/.test(noiseDraw),
    'canvas 的 noise 分支没有用 noiseTile —— 那不是真噪声',
  )
  add(
    '噪声瓷砖三通道同值（灰度，不是彩色噪点）',
    // ⚠️ 必须是**空白容忍**的：写成一段带固定缩进的字面量会因为换行/空格而假绿
    // （我第一版就是这样 —— 把 `img.data[i+1] = g` 改成 `g ^ 32` 之后它照样通过）。
    /img\.data\[i\]\s*=\s*g\s*;\s*img\.data\[i\s*\+\s*1\]\s*=\s*g\s*;\s*img\.data\[i\s*\+\s*2\]\s*=\s*g\s*;/.test(tpl),
    'noiseTile 的三通道不是同一个值 —— 那会画出彩色噪点',
  )
  add(
    '噪声用整数哈希（不是 Math.random —— 同一帧必须可复现）',
    /function noiseHash\(/.test(tpl) && !/noiseTile[\s\S]{0,600}Math\.random/.test(tpl),
    'noiseTile 里用了 Math.random，同一帧两次渲染会不同',
  )
  return out
}

const FILES = {
  spec: JSON.parse(fs.readFileSync('spec/effects.json', 'utf8')),
  trans: fs.readFileSync('tools/polish-to-dhampir.mjs', 'utf8'),
  tpl: fs.readFileSync('crates/yekitrim-polish/templates/index.html', 'utf8'),
}

// ---------------------------------------------------------------------------
// 自证：判据不许空转 —— 四种"改坏产品"的形态都必须翻红
// ---------------------------------------------------------------------------
if (process.argv.includes('--self-test')) {
  const { strict: assert } = await import('node:assert')
  const { spec, trans, tpl } = FILES
  let self = 0

  // ① kind 改成 flash + 白色 ⇒ 必须红
  const whiteSpec = structuredClone(spec)
  const whiteNoise = whiteSpec.effects.find((e) => e.id === 'noise')
  whiteNoise.dhampir_rule.kind = 'flash'
  whiteNoise.dhampir_rule.params = { ...whiteNoise.dhampir_rule.params, r: 1, g: 1, b: 1 }
  const r1 = analyze({ spec: whiteSpec, trans, tpl }).filter((x) => !x.ok)
  assert.ok(r1.some((x) => x.name.includes('kind: noise')), '自证①：kind 改成 flash 必须红')
  assert.ok(r1.some((x) => x.name.includes('白色 flash')), '自证①：白纱形态必须红')
  self += 1

  // ② 拿掉固定 seed ⇒ 必须红
  const noSeed = structuredClone(spec)
  delete noSeed.effects.find((e) => e.id === 'noise').dhampir_rule.params.seed
  const r2 = analyze({ spec: noSeed, trans, tpl }).filter((x) => !x.ok)
  assert.ok(r2.some((x) => x.name.includes('seed')), '自证②：拿掉 seed 必须红')
  self += 1

  // ③ 把 `case 'noise'` 加回来 ⇒ 必须红
  const caseBack = trans.replace("        case 'pulse': {", "        case 'noise': {\n          break;\n        }\n\n        case 'pulse': {")
  assert.notEqual(caseBack, trans, '自证③：变异必须命中')
  const r3 = analyze({ spec, trans: caseBack, tpl }).filter((x) => !x.ok)
  assert.ok(r3.some((x) => x.name.includes("case 'noise'")), '自证③：手写 case 回潮必须红')
  self += 1

  // ④ 模板退回 #ffffff 铺满 ⇒ 必须红
  const whiteTpl = tpl.replace(
    'if (nPat) { ctx.fillStyle = nPat; ctx.fillRect(0, 0, CW, CH); }',
    "if (nPat) { ctx.fillStyle = '#ffffff'; ctx.fillRect(0, 0, CW, CH); }",
  )
  assert.notEqual(whiteTpl, tpl, '自证④：变异必须命中（模板锚点变了就要改自证）')
  const r4 = analyze({ spec, trans, tpl: whiteTpl }).filter((x) => !x.ok)
  assert.ok(r4.some((x) => x.name.includes('#ffffff')), '自证④：白色铺满必须红')
  self += 1

  console.log(`✓ noise 守卫自证：${self} 项（kind 改 flash / 拿掉 seed / case 回潮 / 模板白纱，都必须红）`)
  process.exit(0)
}

for (const x of analyze(FILES)) check(x.name, x.ok, x.detail)

console.log(fail ? `\n${fail} 条不通过（共 ${checks} 项）` : `\n全部通过（共 ${checks} 项）`)
process.exit(fail ? 1 : 0)
