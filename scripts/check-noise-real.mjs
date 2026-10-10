#!/usr/bin/env node
// §20 守卫：`noise` 必须走**真黑白噪点**，不许退回"白色 flash / 白纱"。
//
// 为什么值得一条独立门禁（2026-10-11）：
// 这个特效**名字叫 noise、画的一直是白纱**，而且**两处实现各自为政** ——
// 转译器发白色 `flash`、出片模板铺 `#ffffff`、只有 DOM 预览有真噪声（另一张 SVG）。
// 三者**都不报错**，画面"看起来有东西在闪"，所以这件事在成片里**看不出来**。
// 用户拍板改真噪声之后，任何一处退回白纱都会让"预览 ≠ 成片"重新出现。
import fs from 'node:fs'

const read = (p) => fs.readFileSync(p, 'utf8')
let fail = 0
const check = (name, ok, detail) => {
  if (ok) console.log(`  ok   ${name}`)
  else { console.log(`  FAIL ${name}${detail ? `\n         ${detail}` : ''}`); fail += 1 }
}

// ① 转译器：`case 'noise'` 里必须发 `kind: 'noise'`，且不许出现白色 flash
const trans = read('tools/polish-to-dhampir.mjs')
const caseStart = trans.indexOf("case 'noise': {")
const caseEnd = trans.indexOf("case 'blur': {", caseStart)
const caseBody = trans.slice(caseStart, caseEnd)
check(
  '转译器把 noise 发成底座 noise（不是白色 flash）',
  /kind:\s*'noise'/.test(caseBody),
  'case \'noise\' 里找不到 kind: \'noise\' —— 那是退回白纱了',
)
check(
  '转译器的 noise 分支不再出现白色 flash',
  !/kind:\s*'flash'[\s\S]{0,200}r:\s*1\s*,\s*g:\s*1\s*,\s*b:\s*1/.test(caseBody),
  'noise 分支里又出现了 r:1,g:1,b:1 的白色 flash',
)
check(
  '转译器带固定 seed（噪声必须是帧号的函数，不能用随机）',
  /seed:\s*\d+/.test(caseBody),
  'noise 分支里没有 seed —— 用随机会让预览与成片逐帧对不上',
)

// ② 出片模板：canvas 路径不许把 noise 画成纯白铺满
const tpl = read('crates/yekitrim-polish/templates/index.html')
const noiseDraw = tpl.slice(
  tpl.indexOf('// ---- noise overlay'),
  tpl.indexOf('// ---- flash ----'),
)
// ⚠️ **先把注释剥掉再判** —— 那段注释里为了说明历史**故意引用了** `fillStyle='#ffffff'`，
//    不剥的话这条门禁会被自己的说明文字打红（我第一版就是这样）。
const stripLine = new RegExp('(^|[^:])//[^\\n]*', 'g')
const stripComments = (src) => src.replace(/\/\*[\s\S]*?\*\//g, '').replace(stripLine, '$1')
const noiseCode = stripComments(noiseDraw)
check(
  '出片模板不再用 #ffffff 铺满表达 noise',
  !/fillStyle\s*=\s*'#ffffff'/.test(noiseCode),
  'canvas 的 noise 分支又变回"纯白薄纱"了',
)
check(
  '出片模板用的是灰度噪声瓷砖',
  /noiseTile\(/.test(noiseDraw),
  'canvas 的 noise 分支没有用 noiseTile —— 那不是真噪声',
)
check(
  '噪声瓷砖三通道同值（灰度，不是彩色噪点）',
  // ⚠️ 必须是**空白容忍**的：写成一段带固定缩进的字面量会因为换行/空格而假绿
  // （我第一版就是这样 —— 把 `img.data[i+1] = g` 改成 `g ^ 32` 之后它照样通过）。
  /img\.data\[i\]\s*=\s*g\s*;\s*img\.data\[i\s*\+\s*1\]\s*=\s*g\s*;\s*img\.data\[i\s*\+\s*2\]\s*=\s*g\s*;/.test(tpl),
  'noiseTile 的三通道不是同一个值 —— 那会画出彩色噪点',
)
check(
  '噪声用整数哈希（不是 Math.random —— 同一帧必须可复现）',
  /function noiseHash\(/.test(tpl) && !/noiseTile[\s\S]{0,600}Math\.random/.test(tpl),
  'noiseTile 里用了 Math.random，同一帧两次渲染会不同',
)

// ③ 词表里的 dhampir 映射必须指向真噪声
const spec = JSON.parse(read('spec/effects.json'))
const noise = (spec.effects || []).find((e) => e.id === 'noise')
check(
  '词表的 dhampir 映射指向 ColorMask::noise',
  !!noise && /ColorMask::noise/.test(noise.dhampir || ''),
  `词表里写的是 ${noise?.dhampir}`,
)

console.log(fail ? `\n${fail} 条不通过` : '\n全部通过')
process.exit(fail ? 1 : 0)
