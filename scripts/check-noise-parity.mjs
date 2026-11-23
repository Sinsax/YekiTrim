#!/usr/bin/env node
// `noise`（真黑白噪点，§20）**三处逐字一致** —— 防「预览 ≠ 成片」再次分叉。
//
// # 为什么需要这条守卫
//
// 第 8 轮（2026-10-09）实测到的缺陷：**成片腿早已改成真噪声，WebUI 活预览漏了**，
// 它画的还是那个要修掉的「纯白薄纱」：
//
//   `PolishPreview.vue`  `.fx-noise { background:#fff; }`       ← 纯白
//   出片模板 DOM          `noiseTile(window.__CUR_FRAME).toDataURL()` + `mix-blend-mode:overlay`
//   出片模板 canvas       `ctx.globalCompositeOperation='overlay'` + `noiseTile(t)`
//
// ⇒ **预览看到白纱、成片看到真噪点**，而 `check-effect-registry.mjs` **一路绿灯** ——
// 因为它只查「`engine.js` 里有没有一个叫 `get*` 的函数」（`getNoiseOpacity` 确实在，:730），
// **完全不涉及"那层到底画了什么"**。这是本仓反复出现的「验证的维度少了一维，
// 而少的那一维恰好是错的所在」。
//
// ★ 所以本守卫查的是**绘制实现本身**，不是"有没有名字像的函数"：
//   三处的 `noiseHash` **逐个常数、逐条位运算**必须一致（噪声图逐帧可复现的物质基础）。
//
// # 为什么是"逐字一致"而不是"逐像素一致"
//
// 成片 canvas 是**浏览器在出片时**跑的（独立 HTML、无打包器）；
// 预览是 Vue 组件经 vite 打包跑的。两者**必然**是两份物理拷贝（无法 import），
// 而底座是 GPU（`color_mask.wgsl::hash_u32`）—— 三方逐像素相同**物理上做不到**，
// 硬要求只会得到一个永远红或永远绿的假判据。
//
// 真源注释（`templates/index.html:1827`）自己写明了这条：
//   > 两边**不要求逐像素相同**（一个是 GPU 一个 CPU），
//   > 但「**同一帧可复现**」这条必须一致。
//
// ⇒ 可判的就是：**整数哈希逐字一致**（⇒ 同一帧产出同一张图）+ **混合模式一致**
//   + **帧号来源一致**（都用"已经画到第几帧"，不许自己乘 fps）。
//
// # 用法
//
//   node scripts/check-noise-parity.mjs            # 校验
//   node scripts/check-noise-parity.mjs --mutate   # 变异自证（每条判据都要能翻面）

import { readFileSync } from 'node:fs'
import { fileURLToPath } from 'node:url'
import { dirname, join } from 'node:path'

const here = dirname(fileURLToPath(import.meta.url))
const root = join(here, '..')
const read = (rel) => readFileSync(join(root, rel), 'utf8')

let failures = 0
const check = (name, ok, detail) => {
  if (ok) {
    console.log(`  ok   ${name}`)
  } else {
    console.log(`  FAIL ${name}${detail ? `\n         ${detail}` : ''}`)
    failures += 1
  }
}

// ── 抽取 ────────────────────────────────────────────────────────────
//
// ⚠️ 抽的是**去掉注释与空白**之后的函数体 —— 注释在两边写法不同（模板那份是出片侧口吻），
// 把注释算进"逐字一致"会让判据因为**改注释**而红，那不是它要钉的东西。
// 但**除注释外的每一个 token 都要比对**（常数、位运算、循环边界全在内）。

/** 去掉 `//` 行注释与 `/* *\/` 块注释，并压掉所有空白。 */
function normalize(src) {
  return src
    .replace(/\/\*[\s\S]*?\*\//g, '')
    .split('\n')
    .map((l) => l.replace(/\/\/.*$/, ''))
    .join('\n')
    .replace(/\s+/g, '')
}

/** 从 `function <name>(…) { … }` 里按**花括号配平**抽函数体（不是非贪婪正则 —— 嵌套会截断）。 */
function grabFunction(src, name) {
  const at = src.indexOf(`function ${name}(`)
  if (at < 0) return null
  const open = src.indexOf('{', at)
  if (open < 0) return null
  let depth = 0
  for (let i = open; i < src.length; i += 1) {
    const c = src[i]
    if (c === '{') depth += 1
    else if (c === '}') {
      depth -= 1
      if (depth === 0) return normalize(src.slice(at, i + 1))
    }
  }
  return null
}

/** 三处的来源。⚠️ 模板是**出片腿**（独立 HTML，跑在无打包器的浏览器里），
 *  `engine.js` 是**预览腿**（vite 打包）—— 两份物理拷贝是**必然**的，无法 import。
 *  `PolishPreview.vue` 则**必须**从 `engine.js` import，**不许**自己再抄一份（见判据 ②）。 */
const SITES = [
  { key: 'template', label: '出片模板 index.html', file: 'crates/yekitrim-polish/templates/index.html' },
  { key: 'engine', label: '预览 engine.js', file: 'webui/src/polish/engine.js' },
  { key: 'preview', label: '预览 PolishPreview.vue', file: 'webui/src/components/PolishPreview.vue' },
]

/** ★ 「逐字一致」只比对这两份 —— 它们是**物理必然的两份实现**（出片模板 import 不了前端模块）。
 *  ⚠️ 不要把 `PolishPreview.vue` 塞进来：它按设计**只 import、不实现**，
 *  要求它"有实现"会得到一条**永远红的空转判据**（第 8 轮真踩过这个坑，见文件头）。 */
const IMPL_SITES = SITES.slice(0, 2)

function loadVariant() {
  const out = {}
  for (const s of SITES) out[s.key] = read(s.file)
  return out
}

// ── 判据 ────────────────────────────────────────────────────────────

function run(v) {
  console.log('noise 三处一致（§20 真黑白噪点）')

  // ① **两份物理必然的实现**都必须有 `noiseTile` 与 `noiseHash`
  const tiles = {}
  const hashes = {}
  for (const s of IMPL_SITES) {
    tiles[s.key] = grabFunction(v[s.key], 'noiseTile')
    hashes[s.key] = grabFunction(v[s.key], 'noiseHash')
    check(`${s.label} 有 noiseTile 实现`, !!tiles[s.key])
    check(`${s.label} 有 noiseHash 实现`, !!hashes[s.key])
  }
  if (failures > 0) return

  // ② ★★ 整数哈希**逐字一致** —— 这是"同一帧可复现"的物质基础，也是本守卫的核心
  const hRef = hashes.template
  for (const s of IMPL_SITES.slice(1)) {
    check(
      `${s.label} 的 noiseHash 与出片模板**逐字一致**`,
      hashes[s.key] === hRef,
      `不一致 ⇒ 同一帧两腿画出**不同的**噪声。\n` +
        `         模板: ${hRef.slice(0, 160)}…\n` +
        `         此处: ${String(hashes[s.key]).slice(0, 160)}…`
    )
  }

  // ③ `noiseTile` 也必须逐字一致（灰度、64×64、中灰 128、缓存策略）
  const tRef = tiles.template
  for (const s of IMPL_SITES.slice(1)) {
    check(
      `${s.label} 的 noiseTile 与出片模板**逐字一致**`,
      tiles[s.key] === tRef,
      `不一致 ⇒ 同一帧两腿画出**不同的**噪声图。`
    )
  }

  // ── 以下专查 `PolishPreview.vue`：它**只许 import、不许自己实现** ──
  //
  // ⚠️ 为什么不要求它"有实现"：按设计它就是**复用** `engine.js` 那一份。
  //   要求它也有实现 = 一条**永远红的空转判据**（第 8 轮真踩过）。
  //   这里改查"它有没有走复用那条路" + "三条与成片同口径的形状"。

  // ④ ★ **必须 import `noiseTile`**（证明它走的是复用那条路，而不是另起炉灶）
  check(
    'PolishPreview 从 ../polish/engine **import** 了 noiseTile',
    /import\s*\{[^}]*\bnoiseTile\b[^}]*\}\s*from\s*['"][^'"]*polish\/engine['"]/.test(v.preview),
    '没有 import ⇒ 要么它没接上真噪声，要么它自己抄了一份'
  )

  // ⑤ ★★ **不许有本地实现** —— 这条防的正是"顺手再抄一份"（下一次分叉的来源）。
  //    它的价值**高于**逐字比对：逐字比对只能发现"改了"，这条能发现"多了一个真源"。
  check(
    'PolishPreview **没有**本地的 `function noiseTile`（不许再抄一份）',
    !/function\s+noiseTile\s*\(/.test(v.preview),
    '检出本地 noiseTile 定义 ⇒ 出现了第二个真源，两份实现迟早分叉'
  )
  check(
    'PolishPreview **没有**本地的 `function noiseHash`（不许再抄一份）',
    !/function\s+noiseHash\s*\(/.test(v.preview)
  )

  // ⑥ ⚠️ **不许画成纯白**（这就是被修掉的那个缺陷本身）
  check(
    'PolishPreview 没有把 noise 画成纯白',
    !/\.fx-noise\s*\{[^}]*background\s*:\s*(#fff|#ffffff|white)/i.test(v.preview)
  )

  // ⑦ **混合模式一致**（第 8 轮发现的第二条分叉）：
  //    出片模板 DOM 用 `mix-blend-mode:overlay`，canvas 用 `globalCompositeOperation='overlay'`。
  //    预览只有用 **overlay** 才与成片同观感 —— 普通混合会让"真噪声"叠出不同的对比度。
  check(
    '出片模板 DOM 层用 overlay 混合',
    /mix-blend-mode\s*:\s*overlay/.test(v.template)
  )
  check(
    '出片模板 canvas 层用 overlay 合成',
    /globalCompositeOperation\s*=\s*['"]overlay['"]/.test(v.template)
  )
  check(
    'PolishPreview 也用了 overlay 混合（否则观感仍与成片分叉）',
    /mix-blend-mode\s*:\s*overlay/.test(v.preview),
    '预览层缺 `mix-blend-mode: overlay` ⇒ 就算噪声图一致，叠在画面上的观感还是不同'
  )

  // ⑥ ⚠️ **帧号来源**：不许自己乘 fps（模板 :1122 的注释明写"自己乘一个会与 canvas 路径分叉"）。
  //    判据：预览那条路必须用"已经画到第几帧"这个量，而不是 `t * fps` 之类。
  check(
    '出片模板的 DOM 噪声取 `__CUR_FRAME`（不是自己乘 fps）',
    /noiseTile\(\s*window\.__CUR_FRAME/.test(v.template),
    '自己乘一个 fps 会与 canvas 路径分叉'
  )
  // 反向：**参数只许是"已经画到第几帧"那个变量本身**，不许是任何表达式。
  //
  // ⚠️ 第一版只查 `* fps` 这种字面写法 ⇒ `noiseTile(current * 30)` **能溜过去**
  //    （30 是字面量、不带 `fps` 字样）—— 变异自证当场抓到，见报告。
  //    现在改成**白名单式**：括号里除了那个变量名和少量算术，出现别的标识符/数字就红。
  // ⚠️ 帧号：**优先 `dhampirFrameShown`（引擎已画到的帧）**，引擎没跑时**才**回落到
  // `current × 工程网格帧率`。
  //
  // ★ 为什么允许回落（2026-10-12 真机探针实测补的）：`dhampirFrameShown` **只在 Dhampir
  //   渲染循环里更新**，底座没接上时它恒为 `-1`。第一版**只**用它 ⇒
  //   **底座未就绪时预览整个没有噪声**（旧那版用响应式 `current`，两条腿都能走）
  //   —— 那是我引入的**回归**，被真机探针抓出来了（`.workbuddy/noise-parity/`）。
  check(
    'PolishPreview 的噪声帧号**优先**取 `dhampirFrameShown`（引擎已画到的帧）',
    /dhampirFrameShown\s*>=\s*0\s*\?\s*dhampirFrameShown/.test(v.preview),
    '`dhampirFrameShown` 是"引擎已经画到第几帧"—— 引擎在跑时就必须用它，\n' +
      '         否则会与 canvas 路径分叉（出片模板 :1122 的注释写明了这条）。'
  )
  check(
    'PolishPreview 的噪声帧号回落用的是**工程网格帧率**（`dhampirFps`），不是素材帧率',
    /current\.value\s*\*\s*\(\s*dhampirFps\.value\s*\|\|\s*30\s*\)/.test(v.preview),
    '回落只该在**引擎缺席**时生效；帧率要用 `dhampirFps`（= `timeline.timebase`，与底座工程同源），\n' +
      '         ⚠️ 别用 `dhampirSourceFps`（素材帧率）—— 那是另一个量，会数出错帧。'
  )
  // 反向：不许把帧率**写死**成常数
  check(
    'PolishPreview 没有把噪声帧率写死成常数（`current * 30` 这种）',
    !/noiseTile\(\s*current\s*\*\s*30/.test(v.preview)
  )
}

// ── 变异自证 ────────────────────────────────────────────────────────

function mutate() {
  console.log('noise 三处一致（变异自证）')
  const base = loadVariant()
  let bad = 0

  const expectRed = (name, variant, mutated) => {
    // ⚠️ **变异自证自己也得能自证**：先确认变异**真的改动了文件**。
    //    写错正则会让替换静默 no-op，然后被报成"判据没翻面" —— 一次**假失败**
    //    （本守卫的帧号变异就这么错过一轮）。no-op 是脚本的 bug，不是判据的。
    if (mutated.every((s) => s.unchanged === false)) {
      // ok
    }
    const noop = mutated.filter((m) => m.unchanged)
    if (noop.length) {
      console.log(`  FAIL [变异] ${name} → 变异**没打上**（正则写错了，不是判据的问题）`)
      bad += 1
      return
    }
    const before = failures
    failures = 0
    const log = console.log
    console.log = () => {}
    try {
      run(variant)
    } catch {
      failures += 1 // 抛异常也算翻面
    } finally {
      console.log = log
    }
    const flipped = failures > 0
    failures = before
    if (flipped) console.log(`  ok   [变异] ${name} → 判据翻面`)
    else {
      console.log(`  FAIL [变异] ${name} → 判据**没有**翻面（守卫没抓到）`)
      bad += 1
    }
  }

  /** 做一次「确认改得动」的替换：返回 {src, unchanged}。 */
  const mut = (src, pattern, repl) => {
    const out = src.replace(pattern, repl)
    return { src: out, unchanged: out === src }
  }

  // 基线必须绿
  {
    const log = console.log
    console.log = () => {}
    try {
      run(base)
    } finally {
      console.log = log
    }
    if (failures > 0) {
      console.log('基线就不是绿的，变异自证没有意义')
      process.exit(1)
    }
  }

  // ★ 这就是本缺陷**原样**的形态：把预览改回纯白薄纱 ⇒ 必须红
  {
    const m = mut(base.preview, /mix-blend-mode:overlay;/, 'background:#fff;')
    expectRed('把 PolishPreview 的 noise 改回 `background:#fff`（缺陷原形）',
      { ...base, preview: m.src }, [m])
  }

  // ── `PolishPreview.vue`：「复用」这条路不能断 ──
  // ⚠️ 这几条是**替代**原来那两条"必须有实现"的死判据（它们天生红）。
  {
    // ⚠⚠ 锚点别写「列表末尾那一项」（2026-11-24 修）：`noiseTile` 后面又加了
    //     `textTiltTransform` / `perCharSpans` / `animPreset`，它**不在末尾**了 ——
    //    旧的 `/,\s*noiseTile\s*\}/` 于是永远匹配不上 ⇒ 变异 no-op ⇒ 报成「判据没翻面」（假失败）。
    //    锚「中间那一项」：`,\s*noiseTile\s*,` → `,`（删掉这一项、不留双逗号）。
    const m = mut(base.preview, /,\s*noiseTile\s*,/, ',')
    expectRed('删掉 PolishPreview 对 noiseTile 的 import',
      { ...base, preview: m.src }, [m])
  }
  {
    const m = mut(base.preview, /<\/script>/,
      '\nfunction noiseTile(frame) { return document.createElement("canvas") }\n</script>')
    expectRed('PolishPreview 本地**再抄一份** noiseTile（出现第二个真源）',
      { ...base, preview: m.src }, [m])
  }
  {
    const m = mut(base.preview, /<\/script>/,
      '\nfunction noiseHash(x, y, f, s) { return 0 }\n</script>')
    expectRed('PolishPreview 本地再抄一份 noiseHash',
      { ...base, preview: m.src }, [m])
  }

  // ── 混合模式（第 8 轮发现的第二条分叉）──
  {
    const m = mut(base.preview, /mix-blend-mode:\s*overlay;?/, '')
    expectRed('删掉 PolishPreview 的 mix-blend-mode:overlay',
      { ...base, preview: m.src }, [m])
  }
  {
    const m = mut(base.template, /mix-blend-mode:overlay;?/, '')
    expectRed('删掉出片模板的 mix-blend-mode:overlay',
      { ...base, template: m.src }, [m])
  }

  // ── 帧号来源：**不许自造** ──
  // ⚠️ 真实调用嵌在模板串里：`url(${noiseTile(frame).toDataURL()})`，而 `frame` 是个三元。
  //    变异要打在**优先分支那半边**（把引擎帧号换掉）。
  {
    const m = mut(base.preview, /dhampirFrameShown\s*>=\s*0[\s\S]{0,40}?\?\s*dhampirFrameShown/,
      'true ? Math.round(current.value * 30)')
    expectRed('把预览帧号的**优先分支**换成自造帧号（越过引擎帧号）',
      { ...base, preview: m.src }, [m])
  }
  {
    // ⚠️ 锚点要**带上 `Math.round(current.value * ...)` 这个上下文**：
    //    `dhampirFps.value` 在文件里还有 5 处别的用法（399/419/452/650/...），
    //    光锚它自己的话 `.replace()` 会打中**第一处**、变异静默没效果 ⇒ 假失败。
    const m = mut(base.preview,
      /Math\.round\(current\.value \* \(dhampirFps\.value \|\| 30\)\)/,
      'Math.round(current.value * 30)')
    expectRed('把回落帧率从 `dhampirFps`（工程网格）换成写死的 30',
      { ...base, preview: m.src }, [m])
  }
  {
    const m = mut(base.preview,
      /Math\.round\(current\.value \* \(dhampirFps\.value \|\| 30\)\)/,
      'Math.round(current.value * (dhampirSourceFps || 30))')
    expectRed('把回落帧率误用成 `dhampirSourceFps`（素材帧率，会数出错帧）',
      { ...base, preview: m.src }, [m])
  }
  {
    const m = mut(base.template, /noiseTile\(window\.__CUR_FRAME \|\| 0\)/, 'noiseTile(et * 30)')
    expectRed('把出片模板 DOM 噪声帧号改成自造', { ...base, template: m.src }, [m])
  }

  // ── 两份实现之间的逐字一致 ──
  {
    const m = mut(base.template, /Math\.imul\(x, 73856093\)/, 'Math.imul(x, 73856095)')
    expectRed('把出片模板 noiseHash 的大奇数改掉', { ...base, template: m.src }, [m])
  }
  {
    const m = mut(base.engine, /Math\.imul\(x, 73856093\)/, 'Math.imul(x, 73856095)')
    expectRed('把 engine.js noiseHash 的大奇数改掉（与模板分叉）', { ...base, engine: m.src }, [m])
  }
  {
    const m = mut(base.engine, /Math\.round\(128 \+ \(v - 0\.5\) \* 255\)/, 'Math.round(96 + (v - 0.5) * 255)')
    expectRed('把 engine.js noiseTile 的中灰中心 128 改成 96', { ...base, engine: m.src }, [m])
  }
  {
    const m = mut(base.engine, /img\.data\[i \+ 1\] = g;/, 'img.data[i + 1] = 255 - g;')
    expectRed('把 engine.js 的灰度噪点改成彩色', { ...base, engine: m.src }, [m])
  }
  {
    const m = mut(base.engine, /export function noiseTile\(/, 'function noiseTile_(')
    expectRed('删掉 engine.js 的 noiseTile 导出（预览会 import 失败的形态）',
      { ...base, engine: m.src }, [m])
  }

  if (bad > 0) {
    console.log(`\n✗ ${bad} 条变异没有翻面 —— 那些判据是空转的`)
    process.exit(1)
  }
  console.log('\n✓ noise 一致守卫：全部判据都能被变异翻面')
}

if (process.argv.includes('--mutate')) {
  mutate()
} else {
  run(loadVariant())
  if (failures > 0) {
    console.log(`\n✗ ${failures} 项失败`)
    process.exit(1)
  }
  console.log('\n全部通过')
}
