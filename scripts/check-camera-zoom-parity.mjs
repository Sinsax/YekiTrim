#!/usr/bin/env node
// 运镜档位表**五处逐值一致**（总方案 §18-C / 规格 `camera-and-noise-spec.md` §3.2）。
//
// # 为什么需要这条守卫
//
// 档位表（档位名 → 缩放系数）被 **5 条路径**各自读一遍：
//
//   ① `crates/yekitrim-render-core/src/types.rs` `default_zoom_*()` + `CameraStyle::factor()`（真源）
//   ② `crates/yekitrim-polish/templates/index.html` `camFactor()`（canvas 参照 / 出片模板）
//   ③ `webui/src/polish/engine.js` `camFactor()`（DOM 预览镜像）
//   ④ `tools/polish-to-dhampir.mjs` `zoomFactor()`（转译到底座）
//   ⑤ `crates/yekitrim-server/src/api/polish.rs` 的 `camera` 分支（**写入**这一份配置）
//
// 规格原话：*"都必须走同一张表，否则「预览看到的不是成片」—— 1.5/1.8 的硬编码就是这么来的。"*
//
// 拆开的症状特别难查：**默认值**（`[style.camera]` 没写时用的那三个数）如果在某一路漏改，
// 那么"没改过运镜强度的工程"里，预览与成片会用**不同的倍率** —— 而两边各自看都"很正常"，
// 只有把两张图叠起来量才能发现。本条把五处的默认值钉在一起。
//
// ⚠️ 与 `check-handheld.mjs` 不同的分工：那条钉**手持摇摆**（另一组常数），
// 本条钉**档位倍率**。两条都改时别只跑一条。
//
// # 用法
//
//   node scripts/check-camera-zoom-parity.mjs            # 校验
//   node scripts/check-camera-zoom-parity.mjs --mutate   # 变异自证（每条判据都要能翻面）

import { readFileSync } from 'node:fs'
import { fileURLToPath } from 'node:url'
import { dirname, join } from 'node:path'

const here = dirname(fileURLToPath(import.meta.url))
const root = join(here, '..')

const read = (rel) => readFileSync(join(root, rel), 'utf8')

/** 五处默认值必须同时等于这三个数 —— 改档位就是改这里 + 五处（有变异钉着）。 */
const EXPECTED = { 特写: 1.8, 近: 1.5, 远: 0.55, 中: 1.3 }

let failures = 0
const check = (name, ok, detail) => {
  if (ok) {
    console.log(`  ok   ${name}`)
  } else {
    console.log(`  FAIL ${name}${detail ? `\n         ${detail}` : ''}`)
    failures += 1
  }
}

// ── 抽取各处的默认值 ────────────────────────────────────────────────

/** ① Rust：`fn default_zoom_close() -> f64 { 1.5 }` 这种。 */
function rustDefaults(src) {
  const grab = (fn) => {
    const m = src.match(new RegExp(`fn ${fn}\\(\\) -> f64 \\{\\s*([0-9.]+)\\s*\\}`))
    return m ? Number(m[1]) : null
  }
  return {
    mid: grab('default_zoom_mid'),
    closeup: grab('default_zoom_closeup'),
    close: grab('default_zoom_close'),
    wide: grab('default_zoom_wide'),
  }
}

/** ②③ 两处 `camFactor` 的同形写法：`if(label==='近'){...;d=1.5;}`。 */
function jsCamFactorDefaults(src) {
  const grab = (label) => {
    // 允许 `if(label==='近'){v=c.zoom_close;d=1.5;}` 与 `return pick(c.zoom_close, 1.5)`
    const m =
      src.match(new RegExp(`'${label}'\\s*\\)\\s*\\{[^}]*?d=([0-9.]+)`)) ||
      src.match(new RegExp(`'${label}'\\s*\\)\\s*return pick\\([^,]+,\\s*([0-9.]+)\\)`))
    return m ? Number(m[1]) : null
  }
  // 「中」是**兜底档**、没有 `label==='中'` 那种判断（`else{...d=1.3}` / 最后的 `return pick(...)`）。
  // ⚠️ 认不出的档位也走这一支，所以这个数**必须**与 render-core 的 `_ => self.zoom_mid` 同值。
  const midFallback = () => {
    const m =
      src.match(/else\s*\{[^}]*?d=([0-9.]+)/) ||
      src.match(/return pick\(c\.zoom_mid,\s*([0-9.]+)\)/)
    return m ? Number(m[1]) : null
  }
  return {
    closeup: grab('特写'),
    close: grab('近'),
    wide: grab('远'),
    mid: grab('中') ?? midFallback(),
  }
}

/** ④ 转译器：`const defaults = { 近: 1.5, 远: 0.55, 中: 1.3 }`。 */
function translatorDefaults(src) {
  const m = src.match(/const defaults = \{([^}]+)\}/)
  if (!m) return { closeup: null, close: null, wide: null, mid: null }
  const get = (label) => {
    const mm = m[1].match(new RegExp(`${label}:\\s*([0-9.]+)`))
    return mm ? Number(mm[1]) : null
  }
  return { closeup: get('特写'), close: get('近'), wide: get('远'), mid: get('中') }
}

/** ⑤ 后端：`("zoom_wide", &mut cam.zoom_wide, 0.05, 1.0)` 的夹取区间。 */
function serverClamps(src) {
  const grab = (key) => {
    // 允许 `1.0` / `1.0_f64` / `0.05` 三种写法（`zoom_mid` 那条带 `_f64` 后缀，
    // 因为同级的 `zoom_wide` 字面量足以让类型推断失败 —— 别把后缀当错字删掉）。
    const m = src.match(new RegExp(`\\("${key}",\\s*&mut cam\\.${key},\\s*([0-9.]+)(?:_f64)?,\\s*([0-9.]+)(?:_f64)?\\)`))
    return m ? [Number(m[1]), Number(m[2])] : null
  }
  return {
    mid: grab('zoom_mid'),
    closeup: grab('zoom_closeup'),
    close: grab('zoom_close'),
    wide: grab('zoom_wide'),
  }
}

function run(variant) {
  const rust = rustDefaults(variant.rust)
  const tpl = jsCamFactorDefaults(variant.html)
  const eng = jsCamFactorDefaults(variant.engine)
  const trans = translatorDefaults(variant.dhampir)
  const srv = serverClamps(variant.server)

  const eq = (a, b) => a !== null && b !== null && Math.abs(a - b) < 1e-9

  console.log('\n① 五处**默认值**逐值一致（这是"预览 = 成片"的前提）')
  const all = [
    ['特写', rust.closeup, tpl.closeup, eng.closeup, trans.closeup],
    ['近', rust.close, tpl.close, eng.close, trans.close],
    ['远', rust.wide, tpl.wide, eng.wide, trans.wide],
    ['中', rust.mid, tpl.mid, eng.mid, trans.mid],
  ]
  for (const [label, a, b, c, d] of all) {
    const want = EXPECTED[label]
    check(
      `档位「${label}」五处一致 = ${want}（render-core ${a} / 模板 ${b} / engine ${c} / 转译器 ${d}）`,
      eq(a, want) && eq(b, want) && eq(c, want) && eq(d, want),
      `期望全部 ${want}，实得 [${a}, ${b}, ${c}, ${d}]`,
    )
  }

  console.log('\n② 后端写入的**夹取区间**必须与语义一致（§0.6 露边 / 画中画语义）')
  // 中景与近景：下限 1.0 = 不比原始画面更小（< 1 会缩出黑边，那是画中画的语义）
  check('zoom_mid 夹到 [1.0, 3.0]', eq(srv.mid?.[0], 1.0) && eq(srv.mid?.[1], 3.0), `实得 ${JSON.stringify(srv.mid)}`)
  check('zoom_close 夹到 [1.0, 3.0]', eq(srv.close?.[0], 1.0) && eq(srv.close?.[1], 3.0), `实得 ${JSON.stringify(srv.close)}`)
  // 特写与中/近同一区间：它同样是"放大"，< 1 会缩出黑边（那是画中画的语义）。
  check('zoom_closeup 夹到 [1.0, 3.0]', eq(srv.closeup?.[0], 1.0) && eq(srv.closeup?.[1], 3.0), `实得 ${JSON.stringify(srv.closeup)}`)
  // 画中画：> 1 就不是画中画了；下限取正数（0 会让画面缩成一个点）
  check('zoom_wide 夹到 [0.05, 1.0]', eq(srv.wide?.[0], 0.05) && eq(srv.wide?.[1], 1.0), `实得 ${JSON.stringify(srv.wide)}`)

  console.log('\n③ 后端夹取区间必须**包住**默认值（否则默认值一存就被夹坏）')
  for (const [key, label] of [['mid', '中'], ['closeup', '特写'], ['close', '近'], ['wide', '远']]) {
    const c = srv[key]
    const d = EXPECTED[label]
    check(
      `默认值 ${d}（档位「${label}」）落在夹取区间 ${JSON.stringify(c)} 内`,
      !!c && d >= c[0] && d <= c[1],
      `区间 ${JSON.stringify(c)} 夹不住默认值 ${d}`,
    )
  }

  console.log('\n④ 前端滑杆的 min/max 与后端夹取区间**同值**')
  // 两边不一致的症状：滑块拖到 3.0、保存后弹回 2.5（后端夹了、前端没夹），
  // 或反过来前端夹了、后端存了 5.0。所以这两组数必须是同一批。
  const fe = frontendRanges(variant.polishView)
  check('zoom_close 前端滑杆区间 = 后端夹取区间', rangesEqual(fe.close, srv.close), `前端 ${JSON.stringify(fe.close)} / 后端 ${JSON.stringify(srv.close)}`)
  check('zoom_closeup 前端滑杆区间 = 后端夹取区间', rangesEqual(fe.closeup, srv.closeup), `前端 ${JSON.stringify(fe.closeup)} / 后端 ${JSON.stringify(srv.closeup)}`)
  check('zoom_mid 前端滑杆区间 = 后端夹取区间', rangesEqual(fe.mid, srv.mid), `前端 ${JSON.stringify(fe.mid)} / 后端 ${JSON.stringify(srv.mid)}`)
  check('zoom_wide 前端滑杆区间 = 后端夹取区间', rangesEqual(fe.wide, srv.wide), `前端 ${JSON.stringify(fe.wide)} / 后端 ${JSON.stringify(srv.wide)}`)
}

function frontendRanges(src) {
  const grab = (key) => {
    const m = src.match(new RegExp(`key: '${key}',\\s*label: '[^']*',\\s*min: ([0-9.]+),\\s*max: ([0-9.]+)`))
    return m ? [Number(m[1]), Number(m[2])] : null
  }
  return {
    closeup: grab('zoom_closeup'),
    close: grab('zoom_close'),
    mid: grab('zoom_mid'),
    wide: grab('zoom_wide'),
  }
}

const rangesEqual = (a, b) =>
  !!a && !!b && Math.abs(a[0] - b[0]) < 1e-9 && Math.abs(a[1] - b[1]) < 1e-9

// ── 变异自证 ────────────────────────────────────────────────────────
//
// ⚠️ **每条判据都要有一条变异能翻面**（本仓纪律：守卫不能是空转的）。
// 反过来说：如果某条判据怎么改都不翻面，那它**不是判据**、只是打印了一行字。

function mutate() {
  const base = loadVariant()
  let bad = 0
  const expectRed = (name, variant, fn) => {
    const before = failures
    failures = 0
    try {
      fn(variant)
    } catch {
      /* 解析失败也算抓到 —— 但那不是"判据翻面"，所以下面按 failures 判 */
    }
    const flipped = failures > 0
    failures = before
    if (flipped) {
      console.log(`  ok   [变异] ${name} → 判据翻面`)
    } else {
      console.log(`  FAIL [变异] ${name} → 判据**没有**翻面（守卫没抓到）`)
      bad += 1
    }
  }
  const quiet = (fn) => {
    const log = console.log
    console.log = () => {}
    try {
      fn()
    } finally {
      console.log = log
    }
  }

  // 先确认基线是绿的
  quiet(() => run(base))
  if (failures > 0) {
    console.log('基线就不是绿的，变异自证没有意义')
    process.exit(1)
  }

  expectRed('render-core 把 zoom_close 默认改成 1.8', { ...base, rust: base.rust.replace('fn default_zoom_close() -> f64 {\n    1.5', 'fn default_zoom_close() -> f64 {\n    1.8') }, run)
  expectRed('出片模板的 `d=1.5` 改成 `d=1.8`', { ...base, html: base.html.replace("if(label==='近'){v=c.zoom_close;d=1.5;}", "if(label==='近'){v=c.zoom_close;d=1.8;}") }, run)
  expectRed('engine.js 的近景回落值改成 1.8', { ...base, engine: base.engine.replace("pick(c.zoom_close, 1.5)", "pick(c.zoom_close, 1.8)") }, run)
  expectRed('转译器 defaults 的近景改成 1.8', { ...base, dhampir: base.dhampir.replace('const defaults = { 近: 1.5', 'const defaults = { 近: 1.8') }, run)
  expectRed('后端放开 zoom_mid 下限到 0.5（中景会缩出黑边）', { ...base, server: base.server.replace('("zoom_mid", &mut cam.zoom_mid, 1.0_f64, 3.0_f64)', '("zoom_mid", &mut cam.zoom_mid, 0.5_f64, 3.0_f64)') }, run)
  expectRed('后端把 zoom_wide 上限放到 2.0（画中画不再是画中画）', { ...base, server: base.server.replace('("zoom_wide", &mut cam.zoom_wide, 0.05, 1.0)', '("zoom_wide", &mut cam.zoom_wide, 0.05, 2.0)') }, run)
  expectRed('前端滑杆上限与后端不同（3.0 → 5.0）', { ...base, polishView: base.polishView.replace("key: 'zoom_close', label: '近景', min: 1.0, max: 3.0", "key: 'zoom_close', label: '近景', min: 1.0, max: 5.0") }, run)
  expectRed('默认值掉出夹取区间（近景默认改 0.5）', { ...base, rust: base.rust.replace('fn default_zoom_close() -> f64 {\n    1.5', 'fn default_zoom_close() -> f64 {\n    0.5').replace('fn default_zoom_mid() -> f64 {\n    1.3', 'fn default_zoom_mid() -> f64 {\n    0.5').replace('fn default_zoom_wide() -> f64 {\n    0.55', 'fn default_zoom_wide() -> f64 {\n    0.55'), html: base.html.replace("if(label==='近'){v=c.zoom_close;d=1.5;}", "if(label==='近'){v=c.zoom_close;d=0.5;}").replace("if(label==='远'){v=c.zoom_wide;d=0.55;}", "if(label==='远'){v=c.zoom_wide;d=0.55;}").replace("else{v=c.zoom_mid;d=1.3;}", "else{v=c.zoom_mid;d=0.5;}"), engine: base.engine.replace("pick(c.zoom_close, 1.5)", "pick(c.zoom_close, 0.5)").replace("pick(c.zoom_mid, 1.3)", "pick(c.zoom_mid, 0.5)"), dhampir: base.dhampir.replace('const defaults = { 近: 1.5, 远: 0.55, 中: 1.3 }', 'const defaults = { 近: 0.5, 远: 0.55, 中: 0.5 }') }, run)

  if (bad > 0) {
    console.log(`\n✗ ${bad} 条变异没有翻面 —— 那些判据是空转的`)
    process.exit(1)
  }
  console.log('\n✓ 运镜档位表守卫：全部判据都能被变异翻面')
}

function loadVariant() {
  return {
    rust: read('crates/yekitrim-render-core/src/types.rs'),
    html: read('crates/yekitrim-polish/templates/index.html'),
    engine: read('webui/src/polish/engine.js'),
    dhampir: read('tools/polish-to-dhampir.mjs'),
    server: read('crates/yekitrim-server/src/api/polish.rs'),
    polishView: read('webui/src/views/PolishView.vue'),
  }
}

if (process.argv.includes('--mutate')) {
  console.log('运镜档位表守卫（变异自证）')
  mutate()
} else {
  console.log('运镜档位表守卫（五处逐值一致）')
  run(loadVariant())
  if (failures > 0) {
    console.log(`\n✗ ${failures} 项失败`)
    process.exit(1)
  }
  console.log('\n全部通过')
}
