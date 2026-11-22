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
const EXPECTED = { 特写: 2.0, 近: 1.5, 微: 1.06, 远: 0.55, 全: 1.0, 中: 1.3 }

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
    full: grab('default_zoom_full'),
    mid: grab('default_zoom_mid'),
    closeup: grab('default_zoom_closeup'),
    close: grab('default_zoom_close'),
    micro: grab('default_zoom_micro'),
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
    micro: grab('微'),
    wide: grab('远'),
    full: grab('全'),
    mid: grab('中') ?? midFallback(),
  }
}

/** ④ 转译器：`const defaults = { 近: 1.5, 远: 0.55, 中: 1.3 }`。 */
function translatorDefaults(src) {
  const m = src.match(/const defaults = \{([^}]+)\}/)
  if (!m) return { closeup: null, close: null, wide: null, full: null, mid: null }
  const get = (label) => {
    const mm = m[1].match(new RegExp(`${label}:\\s*([0-9.]+)`))
    return mm ? Number(mm[1]) : null
  }
  return { closeup: get('特写'), close: get('近'), micro: get('微'), wide: get('远'), full: get('全'), mid: get('中') }
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
    full: grab('zoom_full'),
    wide: grab('zoom_wide'),
  }
}

/** ④③② 三处速度档位表：`CAMERA_SPEED_SECS = { snap: 0.10, fast: 0.25, slow: 1.5 }`（写法各异）。 */
function speedTableOf(src) {
  const m = /(?:export\s+)?(?:const|var)\s+CAMERA_SPEED_SECS\s*=\s*\{([^}]*)\}/.exec(src)
  if (!m) return { snap: null, fast: null, slow: null }
  const get = (k) => {
    const mm = m[1].match(new RegExp(`${k}:\\s*([0-9.]+)`))
    return mm ? Number(mm[1]) : null
  }
  return { snap: get('snap'), fast: get('fast'), slow: get('slow') }
}

/** ① placement.rs：`Some("snap") => 0.10` 这种匹配臂。 */
function rustSpeedTable(src) {
  // ⚠️ 用 indexOf + 正则**字面量**取，别拼 `new RegExp`：那串 `Some\\(...\\)` 的转义在
  //    模板字符串里过一遍就变形了（实测取出来全是 null，判据看起来在跑、其实空转）。
  const grab = (k) => {
    const at = src.indexOf('Some("' + k + '")')
    if (at < 0) return null
    const m = src.slice(at).match(/=>\s*([0-9.]+)/)
    return m ? Number(m[1]) : null
  }
  return { snap: grab('snap'), fast: grab('fast'), slow: grab('slow') }
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
    ['微', rust.micro, tpl.micro, eng.micro, trans.micro],
    ['远', rust.wide, tpl.wide, eng.wide, trans.wide],
    ['全', rust.full, tpl.full, eng.full, trans.full],
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

  console.log('\n①b 运镜**过渡时长档位表**四处逐值一致（§16 speed，2026-11-21 加强）')
  // ⚠️ 这一条以前**不存在**，而模板那份 `getActiveCamera` 当时根本没读 speed（写死
  //    `min(dur*0.5, 0.6)`）：预览与成片都按档位走、只有「事实来源」那份最旧。
  //    加进来的目的就是让"四处"变成真的四处 —— 任一路漏改当场红。
  const spTpl = speedTableOf(variant.html)
  const spEng = speedTableOf(variant.engine)
  const spTr = speedTableOf(variant.dhampir)
  const spRust = rustSpeedTable(variant.placement)
  const wantSpeed = { snap: 0.10, fast: 0.25, slow: 1.5 }
  for (const k of ['snap', 'fast', 'slow']) {
    check(
      `速度档「${k}」四处一致 = ${wantSpeed[k]}（模板 ${spTpl[k]} / engine ${spEng[k]} / 转译器 ${spTr[k]} / placement ${spRust[k]}）`,
      eq(spTpl[k], wantSpeed[k]) && eq(spEng[k], wantSpeed[k]) && eq(spTr[k], wantSpeed[k]) && eq(spRust[k], wantSpeed[k]),
      `期望全部 ${wantSpeed[k]}`,
    )
  }
  // `med` / 没写 / 认不出 ⇒ **老式子** `min(dur*0.5, 0.6)`，四处都得是它（不是常量 0.6）
  const medOf = (src) => /min\(\s*\w+\s*\*\s*0\.5\s*,\s*0\.6\s*\)/.test(src)
  check('模板：med 走 min(dur*0.5, 0.6)', medOf(variant.html), true)
  check('engine：med 走 min(dur*0.5, 0.6)', medOf(variant.engine), true)
  check('转译器：med 走 min(dur*0.5, 0.6)', medOf(variant.dhampir), true)
  check('placement：med 走 min(dur*0.5, 0.6)（不是常量 TRANSITION_SECS）', /TRANSITION_SECS\.min\(dur \* 0\.5\)/.test(variant.placement), true)
  console.log('\n①c 运镜**位置状态机**四处同形（接缝从上一块落点出发、块内走到终点）')
  // ⚠️ 这条是**源码锚点**判据（不是逐值对拍）：模板 ↔ engine 的严格对拍在
  //    `webui/scripts/check-handheld.mjs` 的「位置状态机对拍 + 块缝连续性」两段里；
  //    这里补的是**转译器**那一份 —— 它把同一条曲线写成稀疏关键帧，形状不一样、
  //    没法拿同一个函数对拍，所以只钉"接缝的起点必须是 prevOffset，不是本块 origin"
  //    这条曾在 2026-11-21 之前写错、并且**不报错**（块缝瞬跳 928px / 一帧）。
  check('模板：接缝从**当前**位置出发（ox = lerp(ox, ax, p)）', /ox = lerp\(ox, ax, p\);/.test(variant.html), true)
  check('engine：接缝从**当前**位置出发（ox = lerp(ox, startX, p)）', /ox = lerp\(ox, startX, p\)/.test(variant.engine), true)
  check('转译器（稀疏键）：接缝起点键写 prevOffset（不是本块 origin）',
    /placeKeyframe\(shot\.srcStart, 'x', prevOffset\.x, startEasing\);/.test(variant.dhampir), true)
  check('转译器（逐帧重放）：接缝从当前出发（ox = ox + (ax - ox) * p）',
    /ox = ox \+ \(ax - ox\) \* p;/.test(variant.dhampir), true)
  check('转译器（块内位移）：块尾关键帧写到 offEnd',
    /placeKeyframe\(shot\.srcEnd, 'x', offEnd\.x, 'ease_out'\);/.test(variant.dhampir), true)
  console.log('\n② 后端写入的**夹取区间**必须与语义一致（§0.6 露边 / 画中画语义）')
  // 中景与近景：下限 1.0 = 不比原始画面更小（< 1 会缩出黑边，那是画中画的语义）
  check('zoom_mid 夹到 [1.0, 3.0]', eq(srv.mid?.[0], 1.0) && eq(srv.mid?.[1], 3.0), `实得 ${JSON.stringify(srv.mid)}`)
  check('zoom_close 夹到 [1.0, 3.0]', eq(srv.close?.[0], 1.0) && eq(srv.close?.[1], 3.0), `实得 ${JSON.stringify(srv.close)}`)
  // 特写与中/近同一区间：它同样是"放大"，< 1 会缩出黑边（那是画中画的语义）。
  check('zoom_closeup 夹到 [1.0, 3.0]', eq(srv.closeup?.[0], 1.0) && eq(srv.closeup?.[1], 3.0), `实得 ${JSON.stringify(srv.closeup)}`)
  // 「全」（原画 1.0）：与中/近同区间（< 1 会缩出黑边 = 画中画的语义）
  check('zoom_full 夹到 [1.0, 3.0]', eq(srv.full?.[0], 1.0) && eq(srv.full?.[1], 3.0), `实得 ${JSON.stringify(srv.full)}`)
  // 画中画：> 1 就不是画中画了；下限取正数（0 会让画面缩成一个点）
  check('zoom_wide 夹到 [0.05, 1.0]', eq(srv.wide?.[0], 0.05) && eq(srv.wide?.[1], 1.0), `实得 ${JSON.stringify(srv.wide)}`)

  console.log('\n③ 后端夹取区间必须**包住**默认值（否则默认值一存就被夹坏）')
  for (const [key, label] of [['mid', '中'], ['closeup', '特写'], ['close', '近'], ['full', '全'], ['wide', '远']]) {
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
  check('zoom_full 前端滑杆区间 = 后端夹取区间', rangesEqual(fe.full, srv.full), `前端 ${JSON.stringify(fe.full)} / 后端 ${JSON.stringify(srv.full)}`)
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
    full: grab('zoom_full'),
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
  /** ⚠️ **变异体必须真的与原件不同** —— 目标串写错（少一段、多空格）时 `.replace` 静默不命中，
   *  变异体与原件逐字节相同 ⇒ 判据当然不翻面 ⇒ 自证把它报成"守卫没抓到"，
   *  而真相是**这条变异自己没生效**（2026-10-15 实测：转译器那条的 "近" 目标串漏了 `特写: 1.8,`）。
   *  这与 `check-yekitrim-translator.mjs --mutate` 的纪律同源：**先断言替换命中，再看判据翻面**。
   *  `srcKey` 指出被变异的那个来源键（rust/html/engine/dhampir/server/polishView）。 */
  const expectRed = (name, variant, fn, srcKey) => {
    if (srcKey && variant[srcKey] === base[srcKey]) {
      console.log(`  FAIL [变异] ${name} → 变异**没生效**（目标串没命中 ${srcKey}，变异体与原件相同）`)
      bad += 1
      return
    }
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

  expectRed('render-core 把 zoom_close 默认改成 1.8', { ...base, rust: base.rust.replace('fn default_zoom_close() -> f64 {\n    1.5', 'fn default_zoom_close() -> f64 {\n    1.8') }, run, 'rust')
  expectRed('出片模板的 `d=1.5` 改成 `d=1.8`', { ...base, html: base.html.replace("if(label==='近'){v=c.zoom_close;d=1.5;}", "if(label==='近'){v=c.zoom_close;d=1.8;}") }, run, 'html')
  expectRed('engine.js 的近景回落值改成 1.8', { ...base, engine: base.engine.replace("pick(c.zoom_close, 1.5)", "pick(c.zoom_close, 1.8)") }, run, 'engine')
  // 「微」这一档（2026-10-20）：转译器表里 1.06 → 1.10 必须翻面。
  expectRed('转译器的「微」改成 1.10', { ...base, dhampir: base.dhampir.replace('微: 1.06', '微: 1.10') }, run, 'dhampir')
  // ⚠️ 变异目标串必须**逐字**等于源码（`tools/polish-to-dhampir.mjs` 的 `zoomFactor` 里是
  //    `const defaults = { 特写: 1.8, 近: 1.5, 远: 0.55, 中: 1.3 };`）。
  //    曾经写成 `const defaults = { 近: 1.5`（**少了 `特写: 1.8,` 那一段**）⇒ `.replace` 静默不命中
  //    ⇒ 变异体与原件逐字节相同 ⇒ 判据当然不翻面，而自证把它报成"守卫没抓到"。
  //    那是**假红**：真正的守卫一直在工作（另三条变异证明过），红的是这条变异自己没生效。
  //    教训与 `check-yekitrim-translator.mjs` 的 `--mutate` 同源：**每处变异都要先断言"替换命中一次"**。
  expectRed('转译器 defaults 的近景改成 1.8', { ...base, dhampir: base.dhampir.replace('const defaults = { 特写: 2.0, 近: 1.5', 'const defaults = { 特写: 2.0, 近: 1.8') }, run, 'dhampir')
  // 「全」（2026-10-20 新增档）：把模板那一档的回落值改掉 ⇒ 五处不再一致
  expectRed('出片模板的「全」回落值改成 1.2', { ...base, html: base.html.replace("else if(label==='全'){v=c.zoom_full;d=1.0;}", "else if(label==='全'){v=c.zoom_full;d=1.2;}") }, run, 'html')
  expectRed('后端把 zoom_full 下限放到 0.5（原画档会缩出黑边）', { ...base, server: base.server.replace('("zoom_full", &mut cam.zoom_full, 1.0, 3.0)', '("zoom_full", &mut cam.zoom_full, 0.5, 3.0)') }, run, 'server')
  expectRed('后端放开 zoom_mid 下限到 0.5（中景会缩出黑边）', { ...base, server: base.server.replace('("zoom_mid", &mut cam.zoom_mid, 1.0_f64, 3.0_f64)', '("zoom_mid", &mut cam.zoom_mid, 0.5_f64, 3.0_f64)') }, run, 'server')
  expectRed('后端把 zoom_wide 上限放到 2.0（画中画不再是画中画）', { ...base, server: base.server.replace('("zoom_wide", &mut cam.zoom_wide, 0.05, 1.0)', '("zoom_wide", &mut cam.zoom_wide, 0.05, 2.0)') }, run, 'server')
  expectRed('前端滑杆上限与后端不同（3.0 → 5.0）', { ...base, polishView: base.polishView.replace("key: 'zoom_close', label: '近景', min: 1.0, max: 3.0", "key: 'zoom_close', label: '近景', min: 1.0, max: 5.0") }, run, 'polishView')
  expectRed('默认值掉出夹取区间（近景默认改 0.5）', { ...base, rust: base.rust.replace('fn default_zoom_close() -> f64 {\n    1.5', 'fn default_zoom_close() -> f64 {\n    0.5').replace('fn default_zoom_mid() -> f64 {\n    1.3', 'fn default_zoom_mid() -> f64 {\n    0.5').replace('fn default_zoom_wide() -> f64 {\n    0.55', 'fn default_zoom_wide() -> f64 {\n    0.55'), html: base.html.replace("if(label==='近'){v=c.zoom_close;d=1.5;}", "if(label==='近'){v=c.zoom_close;d=0.5;}").replace("if(label==='远'){v=c.zoom_wide;d=0.55;}", "if(label==='远'){v=c.zoom_wide;d=0.55;}").replace("else{v=c.zoom_mid;d=1.3;}", "else{v=c.zoom_mid;d=0.5;}"), engine: base.engine.replace("pick(c.zoom_close, 1.5)", "pick(c.zoom_close, 0.5)").replace("pick(c.zoom_mid, 1.3)", "pick(c.zoom_mid, 0.5)"), dhampir: base.dhampir.replace('const defaults = { 近: 1.5, 远: 0.55, 中: 1.3 }', 'const defaults = { 近: 0.5, 远: 0.55, 中: 0.5 }') }, run)

  // ── 位置状态机（2026-11-21）：接缝起点这一处必须能翻面 ──
  expectRed('模板把接缝退回绝对赋值（旧 bug：块缝瞬跳）', { ...base, html: base.html.replace('ox = lerp(ox, ax, p);', 'ox = lerp(ax, bx, p);') }, run, 'html')
  expectRed('转译器把接缝起点键写成本块 origin（同一 bug）', { ...base, dhampir: base.dhampir.replace("placeKeyframe(shot.srcStart, 'x', prevOffset.x, startEasing);", "placeKeyframe(shot.srcStart, 'x', offStart.x, startEasing);") }, run, 'dhampir')

  // ── 速度档位表（2026-11-21）：四条都要能翻面 ──
  expectRed('模板的 snap 档改成 0.2', { ...base, html: base.html.replace('snap: 0.10, fast: 0.25, slow: 1.5', 'snap: 0.20, fast: 0.25, slow: 1.5') }, run, 'html')
  expectRed('engine 的 fast 档改成 0.3', { ...base, engine: base.engine.replace('snap: 0.10, fast: 0.25, slow: 1.5', 'snap: 0.10, fast: 0.30, slow: 1.5') }, run, 'engine')
  expectRed('转译器的 slow 档改成 2.0', { ...base, dhampir: base.dhampir.replace('  slow: 1.5,', '  slow: 2.0,') }, run, 'dhampir')
  expectRed('placement 的 snap 臂改成 0.2', { ...base, placement: base.placement.replace('Some("snap") => 0.10', 'Some("snap") => 0.20') }, run, 'placement')
  expectRed('placement 的 med 退回常量 0.6（短镜头会多算旧状态）', { ...base, placement: base.placement.replace('_ => TRANSITION_SECS.min(dur * 0.5),', '_ => TRANSITION_SECS,') }, run, 'placement')

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
    // ⑤ 运镜过渡时长（§16 speed）的第四处：贴纸避让侧的 `transition_secs`
    placement: read('crates/yekitrim-polish/src/process/placement.rs'),
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