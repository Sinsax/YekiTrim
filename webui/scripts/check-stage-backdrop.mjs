// 竖屏「模糊衬底」（背景层）的契约与回归钉子 —— 出片 canvas 路径 + 预览 DOM 路径。
//
// 用户报的现象（2026-09-20）：**精修竖屏时，震动等特效会把主画面挪开，露出来的是纯黑**，
// 需要「在视频底下垫一层当前帧的模糊」。三个叠加成因，缺一条都还是黑：
//   ① `resolve_stage` 曾经在 `axis=fill` 时**强制 `bg=none`**，把用户选的"模糊"整个吞掉
//      —— 竖画布铺满（源比例 == 画布比例）时恰好落到这一支，于是衬底根本没被要求画。
//      现在门控是**画布方向**：`axis=="fill" && !portrait` 才 none（横画布铺满保持零回归）。
//      （Rust 侧钉子在 `project::config::tests::portrait_canvas_fill_still_paints_backdrop`）
//   ② 出片是 canvas `renderFrame` **逐帧绘制**，DOM 的 `#bg-video-wrap` **完全不参与导出**：
//      `clearRect` 之后的透明区落进 JPEG 就是纯黑。只把 `stage.bg` 消费在 DOM 上，表现就是
//      「预览里有模糊底、成片被 shake 挪开露纯黑」—— 不报错、不崩溃，只有逐帧像素能发现。
//      本脚本钉住出片侧的消费面与行为（真实像素实证见 `crates/yekitrim-polish/tests/portrait_backdrop.rs`）。
//   ③ 视频层（主画面）在出片路径里**被硬铺满整块画布**（`ctx.drawImage(videoBitmap, 0, 0, CW, CH)`），
//      `_canvas.stage` 只被背景层消费 —— 源比例 ≠ 画布比例时整帧被拉伸。
//      用户报的现象（2026-09-21）：**竖素材进横画布，导出里被拉成宽屏**（要的是"竖素材 + 模糊关键帧"）。
//      修法：出片侧自己按 `_canvas.stage` 算摆放矩形（`stageDrawRect`，与 CSS `#video-wrap` / `#fit-box`
//      逐值同义），再用 CTM `translate + scale` 把整帧送进矩形 —— `drawImage` 的字面量保持不变
//      （③ 的顺序钉子与 ⑤ 的变异都用它当锚点，别顺手改成 dest 矩形参数）。
//
// 运行： node webui/scripts/check-stage-backdrop.mjs
//
// 零依赖、不 import 模块（`webui/package.json` 没有 `"type":"module"`），
// 从源码文本里抠函数（配对括号），用**假 ctx** 记录绘制调用 —— 不需要浏览器。

import { readFile } from 'node:fs/promises'
import { fileURLToPath } from 'node:url'
import { dirname, join } from 'node:path'

const here = dirname(fileURLToPath(import.meta.url))
const root = join(here, '..', '..')

const read = async (...parts) => readFile(join(root, ...parts), 'utf8')
const html = await read('crates', 'yekitrim-polish', 'templates', 'index.html')
const previewVue = await read('webui', 'src', 'components', 'PolishPreview.vue')
const engineJs = await read('webui', 'src', 'polish', 'engine.js')
const translator = await read('tools', 'polish-to-dhampir.mjs')

/** 去掉 `//` 行注释与块注释（字符串字面量原样保留）。
 *  ⚠️ **源码断言必须先过它**：注释里出现的 `fs` / `drawStageBackdrop(...)` 会让判据假阳性 ——
 *  实测踩过：`renderFrame` 里那句"别再写 `ctx.font='700 '+fs+…`"的注释
 *  把「不许出现裸 fs」这条判据自己顶翻了。 */
function stripComments(src) {
  let out = ''
  for (let i = 0; i < src.length; i++) {
    const ch = src[i]
    const nxt = src[i + 1]
    if (ch === '"' || ch === "'" || ch === '`') {
      const q = ch
      out += ch
      i++
      while (i < src.length && src[i] !== q) {
        if (src[i] === '\\') { out += src[i]; i++ }
        out += src[i]
        i++
      }
      out += src[i] ?? ''
      continue
    }
    if (ch === '/' && nxt === '/') {
      const nl = src.indexOf('\n', i)
      i = nl < 0 ? src.length : nl - 1
      continue
    }
    if (ch === '/' && nxt === '*') {
      const e = src.indexOf('*/', i + 2)
      i = e < 0 ? src.length : e + 1
      continue
    }
    out += ch
  }
  return out
}
/** 去掉注释后的模板源码 —— 所有"源码里有没有 / 谁在前"的判据都该用它 */
const code = stripComments(html)

// ── 从模板里抠顶层函数（配对括号，跳过注释与字符串；与 check-caption-wrap.mjs 同一套）──
function extractFunction(src, name) {
  const start = src.indexOf(`function ${name}(`)
  if (start < 0) throw new Error(`index.html 里找不到 function ${name}(`)
  let i = src.indexOf('{', start)
  let depth = 0
  for (; i < src.length; i++) {
    const ch = src[i]
    if (ch === '/' && src[i + 1] === '/') {
      const nl = src.indexOf('\n', i)
      if (nl < 0) break
      i = nl
      continue
    }
    if (ch === '/' && src[i + 1] === '*') {
      const e = src.indexOf('*/', i + 2)
      i = e < 0 ? src.length : e + 1
      continue
    }
    if (ch === "'" || ch === '"' || ch === '`') {
      const q = ch
      i++
      while (i < src.length && src[i] !== q) {
        if (src[i] === '\\') i++
        i++
      }
      continue
    }
    if (ch === '{') depth++
    else if (ch === '}') {
      depth--
      if (depth === 0) return src.slice(start, i + 1)
    }
  }
  throw new Error(`function ${name} 的括号没配平`)
}

/** 把 `stageCoverRect` + `drawStageBackdrop` 装进一个作用域，`CW/CH/STAGE/STAGE_BG` 由调用方给 */
function loadBackdrop(src) {
  return new Function(
    'CW', 'CH', 'STAGE', 'STAGE_BG',
    `${extractFunction(src, 'stageCoverRect')}
     ${extractFunction(src, 'drawStageBackdrop')}
     return { stageCoverRect, drawStageBackdrop }`
  )
}

const tpl = loadBackdrop(html)

/** 把 `stageDrawRect` 装进一个作用域（全部走参数，不依赖 CW/CH/STAGE 全局，便于直接喂几何用例） */
function loadRect(src) {
  return new Function(`${extractFunction(src, 'stageDrawRect')}\n return stageDrawRect`)()
}

/** **模板 ↔ 转译器**：`stageDrawRect` 是同一套几何的**两份实现**（出片 canvas 与底座转译）。
 *  只改一边 = 预览/成片对得上、底座那条腿对不上（或反过来），而且**不报错**。
 *  转译器那份是闭包（读模块级 const），所以从源码文本抽出来、喂同一组输入逐值对拍。
 *  ⚠️ 用 `indexOf` 切片而不是正则：那段函数体里自己带 `{`，正则非贪婪会切错地方。 */
function stageParityDiff(transSrc, tplSrc) {
  const src = String(transSrc)
  const at = src.indexOf('const stageDrawRect = (bw, bh, cw, ch) => {')
  if (at < 0) return { error: '转译器里找不到 `stageDrawRect` 闭包' }
  const stop = src.indexOf('\n  };', at)
  if (stop < 0) return { error: '转译器里 `stageDrawRect` 的收尾 `};` 找不到' }
  const body = src.slice(at, stop + 6)
  const rect = loadRect(tplSrc)
  const makeT = new Function(
    'axis', 'stagePctX', 'stagePctY', 'stagePadding', 'stageScale',
    `${body}\nreturn stageDrawRect;`,
  )
  const cases = [
    ['x', 0, 0, 1, 0], ['x', 25, 50, 1, 0], ['x', 75, 50, 1, 0], ['x', 100, 50, 1, 0],
    ['x', 0, 0, 2, 0], ['x', 50, 50, 2, 0], ['x', 100, 100, 2, 0],
    ['x', 50, 50, 1.6, 60], ['y', 50, 25, 1, 0], ['y', 50, 100, 2, 40],
    ['fill', 50, 50, 1, 0], ['fill', 25, 75, 1.5, 0],
  ]
  const geoms = [
    [720, 1280, 1920, 1080], [1280, 720, 1080, 1920],
    [1440, 1080, 1920, 1080], [1080, 1920, 1080, 1920],
  ]
  let diff = 0
  for (const [axis, px, py, scale, pad] of cases) {
    for (const [bw, bh, cw, ch] of geoms) {
      const st = { axis, position_x_pct: px, position_y_pct: py, scale, padding: pad }
      const a = rect(bw, bh, cw, ch, st)
      const b = makeT(axis, px, py, pad, scale)(bw, bh, cw, ch)
      for (const k of ['x', 'y', 'w', 'h']) if (Math.abs(a[k] - b[k]) > 1e-9) diff += 1
    }
  }
  return { diff }
}

// ── 假 2D 上下文：记录每个绘制调用（含当时的 filter / fillStyle） ─────────────
function fakeCtx() {
  const calls = []
  const ctx = {
    calls,
    _filter: 'none',
    fillStyle: '',
    save() { calls.push({ op: 'save' }) },
    restore() { calls.push({ op: 'restore' }) },
    translate(x, y) { calls.push({ op: 'translate', x, y }) },
    scale(sx, sy) { calls.push({ op: 'scale', sx, sy }) },
    fillRect(x, y, w, h) { calls.push({ op: 'fillRect', x, y, w, h, fillStyle: ctx.fillStyle, filter: ctx._filter }) },
    drawImage(img, x, y, w, h) { calls.push({ op: 'drawImage', x, y, w, h, filter: ctx._filter }) },
  }
  Object.defineProperty(ctx, 'filter', {
    get() { return ctx._filter },
    set(v) { ctx._filter = v; calls.push({ op: 'filter', v }) },
  })
  return ctx
}

const CW = 1080
const CH = 1920
const BMP = { width: 720, height: 1280 } // 9:16 竖素材：与画布同比例
/** 源比画布更宽（16:9 素材进竖画布）：cover 会裁掉左右 */
const BMP_WIDE = { width: 1280, height: 720 }

function run(html_, stageBg, stage = {}, bmp = BMP) {
  const CW_ = stage.CW || CW
  const CH_ = stage.CH || CH
  const fn = loadBackdrop(html_)
  const api = fn(CW_, CH_, stage, stageBg)
  const ctx = fakeCtx()
  api.drawStageBackdrop(ctx, bmp)
  return { ctx, api, CW: CW_, CH: CH_ }
}

// ── 断言脚手架 ────────────────────────────────────────────────────────────
let failed = 0
/** 深比较（数组/对象按值）：`check` 里经常拿数组当"一组事实"用，`===` 会永远不等 */
const same = (a, b) => JSON.stringify(a) === JSON.stringify(b)
function check(label, ok, want = true, hint = '') {
  if (same(ok, want)) console.log(`  ok   ${label}`)
  else {
    failed++
    console.log(`  FAIL ${label}`)
    console.log(`         实得 ${JSON.stringify(ok)}，期望 ${JSON.stringify(want)}${hint ? '：' + hint : ''}`)
  }
}
const draws = (ctx) => ctx.calls.filter((c) => c.op === 'drawImage' || c.op === 'fillRect')
const approx = (a, b, eps = 0.5) => Math.abs(a - b) <= eps

/** `object-fit:cover` 的语义：等比放大到**两个方向都盖住**画布（x/y ≤ 0、右/下 ≥ 画布） */
function coversCanvas(r, cw, ch) {
  return r.x <= 0.5 && r.y <= 0.5 && r.x + r.w >= cw - 0.5 && r.y + r.h >= ch - 0.5
}

// ── ① 几何：cover 铺满（`Math.max` 而不是 `Math.min`） ───────────────────────
console.log('① stageCoverRect = cover 铺满（两个方向都必须盖住画布）')
{
  for (const [label, bmp] of [['竖素材 720×1280 → 竖画布', BMP], ['横素材 1280×720 → 竖画布', BMP_WIDE]]) {
    const r = tpl(CW, CH, {}, 'blur').stageCoverRect(bmp.width, bmp.height, CW, CH)
    check(`${label} cover 盖满画布`, coversCanvas(r, CW, CH), true, `rect=${JSON.stringify(r)}`)
    // 等比：宽高比与源一致（不拉伸）
    check(`${label} 保持源比例`, approx(r.w / r.h, bmp.width / bmp.height, 0.01), true)
  }
}

// ── ② 行为：四种 bg 各自画什么 ──────────────────────────────────────────────
console.log('\n② drawStageBackdrop 的四种模式')
{
  // none：一个绘制调用都不该有（横画布铺满 = 改造前的零回归路径）
  const none = run(html, 'none')
  check('bg=none → 不画任何东西（透明 = JPEG 黑，与改造前逐字节一致）', draws(none.ctx).length, 0)

  // black：整幅铺纯黑
  const black = run(html, 'black')
  check('bg=black → 一次整幅 fillRect', draws(black.ctx).length, 1)
  check('bg=black 的 fillStyle 是 #000000', black.ctx.calls[0]?.fillStyle, '#000000')
  check('bg=black 铺满整幅', [black.ctx.calls[0]?.x, black.ctx.calls[0]?.y, black.ctx.calls[0]?.w, black.ctx.calls[0]?.h], [0, 0, CW, CH])

  // color：整幅铺配置色
  const color = run(html, 'color', { bg_color: '#123456' })
  check('bg=color → 整幅 fillRect 用 bg_color', [draws(color.ctx).length, color.ctx.calls[0]?.fillStyle], [1, '#123456'])

  // blur：当前帧按 cover 铺满 + 高斯模糊 + 压暗去饱和 + 绕中心放大
  const blur = run(html, 'blur', { blur_px: 28, blur_img_scale: 1.187 })
  const d = draws(blur.ctx)
  check('bg=blur → 一次 drawImage（画当前帧）', d.length, 1)
  check('bg=blur 的 filter 带 blur(28px) 与压暗去饱和',
    /blur\(28px\)/.test(d[0]?.filter || '') && /brightness\(0\.5\)/.test(d[0]?.filter || '') && /saturate\(1\.15\)/.test(d[0]?.filter || ''), true,
    `filter=${d[0]?.filter}`)
  // 放大是把模糊溢出的边缘推出画布：放 1.187 倍后仍然要盖满
  const bs = 1.187
  const eff = {
    x: CW / 2 + bs * (d[0].x - CW / 2),
    y: CH / 2 + bs * (d[0].y - CH / 2),
    w: bs * d[0].w,
    h: bs * d[0].h,
  }
  check('bg=blur 放大 blur_img_scale 后仍盖满画布', coversCanvas(eff, CW, CH), true, `eff=${JSON.stringify(eff)}`)
  // 模糊半径来自注入的 blur_px（写死 28 会让「模糊强度」滑杆失效）
  const blur40 = run(html, 'blur', { blur_px: 40, blur_img_scale: 1.187 })
  check('bg=blur 的半径跟随注入的 blur_px', /blur\(40px\)/.test(draws(blur40.ctx)[0]?.filter || ''), true)
  // save/restore 必须配对：filter 泄漏出去会让**视频层也永久糊掉**
  const s = blur.ctx.calls.filter((c) => c.op === 'save').length
  const r = blur.ctx.calls.filter((c) => c.op === 'restore').length
  check('bg=blur 的 save/restore 配对（filter 不许泄漏到视频层）', [s, r, s === r && s > 0], [1, 1, true])
  check('bg=blur 的 filter 在 restore 之后没有残留吗（由配对保证）', blur.ctx.calls.at(-1)?.op, 'restore')
}

// ── ③ 出片顺序：衬底必须先于视频层绘制 ──────────────────────────────────────
console.log('\n③ 出片 canvas 路径的图层顺序')
{
  // ⚠️ 用去注释后的源码：注释里提到过这两个调用点，拿原始文本比顺序会被注释带偏
  const backdrop = code.indexOf('drawStageBackdrop(ctx, videoBitmap)')
  const video = code.indexOf('ctx.drawImage(videoBitmap, 0, 0, CW, CH)')
  check('renderFrame 里能找到衬底绘制点', backdrop > 0, true)
  check('renderFrame 里能找到视频层绘制点', video > 0, true)
  check('衬底画在视频层之前（画反了铺满时整块看不到底色）', backdrop > 0 && video > 0 && backdrop < video, true)
  check('renderFrame 每帧先 clearRect 再画衬底',
    code.indexOf('ctx.clearRect(0, 0, CW, CH)') > 0 && code.indexOf('ctx.clearRect(0, 0, CW, CH)') < backdrop, true)
  check('出片路径的 STAGE 读的是注入的 _canvas.stage',
    /var STAGE=\(CFG\._canvas&&CFG\._canvas\.stage\)\|\|\{\};/.test(code), true)
  check('STAGE_BG 默认 none（没注入契约时 = 改造前行为）',
    code.includes("var STAGE_BG=STAGE.bg||'none';"), true)
}

// ── ③b 出片逐帧路径：不许引用作用域里没声明的变量（实测踩过，代价是整块 chunk 失败）──
console.log('\n③b renderFrame 里的裸变量（ReferenceError = 整块 chunk 失败）')
{
  const rf = extractFunction(code, 'renderFrame')
  check('renderFrame 里接到了字幕折行（wrapCaptionParts）', /wrapCaptionParts\(/.test(rf), true)
  // ⚠️ A5 折行改造把字号挪进逐条字幕（`baseSize` / `size`），却留下 `ctx.font='700 '+fs+' '+ff`：
  // `fs` 只在另外两个函数里各自 `var` 过，在 `renderFrame` 里是**未声明标识符** →
  // 只要这一帧有字幕就 `ReferenceError: fs is not defined`，
  // 出片日志是 `Chunk: Render error: fs is not defined`，表现是**所有带字幕的片段都导不出来**。
  check('renderFrame 里没有裸 `fs`（历史 ReferenceError：带字幕必崩）',
    /['"`]\s*700\s*['"`]\s*\+\s*fs\s*\+/.test(rf), false, '字号要写 baseSize / size，见 A5 折行')
  check('renderFrame 的基准字号来自 subS.size（不是外部变量）', /baseSize\s*=\s*subS\.size/.test(rf), true)
  // 变异自证：把这行塞回去，判据必须翻面
  const withFs = code.replace(
    'caps.forEach(function(cap) {',
    "ctx.font = '700 ' + fs + ' ' + ff;\n      caps.forEach(function(cap) {"
  )
  check('[变异] 塞回 `ctx.font=\'700 \'+fs+\' \'+ff` → 判据翻面', withFs !== code, true)
  check('[变异] 塞回后该判据确实翻面',
    /['"`]\s*700\s*['"`]\s*\+\s*fs\s*\+/.test(extractFunction(withFs, 'renderFrame')), true)
}

// ── ③c 视频层摆位：源比例 ≠ 画布比例时等比缩进（不许拉伸）───────────────────
// 与 CSS 的对应关系（`applyStage` 写入的那几条规则）：
//   `#video-wrap` 的 padding = `--stage-pad`；`#fit-box` 的 axis-*/stage-* = 布局盒；object-fit = cover/contain
// 这里的判据全部是**几何**判据：等比（w/h == 源比例）+ 铺满的那条边贴齐 + position 三档 + padding / scale。
console.log('\n③c 出片视频层的摆放矩形（stageDrawRect，两轴 xy）')
{
  const rect = loadRect(html)
  const SW = 720, SH = 1280            // 9:16 竖素材
  const WIDE_W = 1280, WIDE_H = 720    // 16:9 横素材
  /** 与源比例一致 = 1（不拉伸）；≈1.78 就是被拉宽、≈0.32 就是被压扁 */
  const ratio = (r, sw, sh) => r.w / r.h / (sw / sh)
  /** 只给某一轴的位置，另一轴保持居中（两轴独立 ⇒ 交叉轴不该动） */
  const at = (axis, px, py, scale, pad, bw, bh, cw, ch) =>
    rect(bw, bh, cw, ch, { axis, position_x_pct: px, position_y_pct: py, scale, padding: pad })

  // ① 存量零回归：铺满（axis=fill、无 padding、源比例 == 画布比例）必须精确是整块画布
  check('axis=fill + 同比例 → 精确 (0,0,CW,CH)（存量横画布逐像素零回归）',
    rect(1920, 1080, 1920, 1080, { axis: 'fill' }), { x: 0, y: 0, w: 1920, h: 1080 })

  // ② 竖素材进横画布（axis=x）：按高度铺满 + 等比 + **X 五档**，Y 无余量 ⇒ 取什么都不动。
  //    `position` 那个枚举已退场（基线恒居中），几何权威是两轴百分比。
  for (const [pct, wantX] of [[0, 0], [25, 328.125], [50, 656.25], [75, 984.375], [100, 1312.5]]) {
    const r = at('x', pct, 50, 1, 0, SW, SH, 1920, 1080)
    check(`竖素材进横画布 axis=x X=${pct}：等比（720×1280 → 607.5×1080）`, approx(ratio(r, SW, SH), 1, 1e-9), true, JSON.stringify(r))
    check(`竖素材进横画布 axis=x X=${pct}：高度铺满、水平摆位 x=${wantX}`,
      approx(r.h, 1080, 1e-6) && approx(r.x, wantX, 1e-6) && approx(r.y, 0, 1e-6), true, JSON.stringify(r))
    // 两轴独立：Y 给 0 / 100 都不动（该轴余量为 0 —— 界面据此把那条滑杆置灰）
    const y0 = at('x', pct, 0, 1, 0, SW, SH, 1920, 1080)
    const y100 = at('x', pct, 100, 1, 0, SW, SH, 1920, 1080)
    check(`竖素材进横画布 axis=x X=${pct}：Y 无余量（0 与 100 同值、不越位）`,
      JSON.stringify(y0) === JSON.stringify(y100) && JSON.stringify(y0) === JSON.stringify(r), true,
      `y0=${JSON.stringify(y0)} y100=${JSON.stringify(y100)}`)
  }

  // ③ 横素材进竖画布（axis=y）：按宽度铺满 + **Y 五档**（X 无余量）
  for (const [pct, wantY] of [[0, 0], [25, 328.125], [50, 656.25], [75, 984.375], [100, 1312.5]]) {
    const r = at('y', 50, pct, 1, 0, WIDE_W, WIDE_H, 1080, 1920)
    check(`横素材进竖画布 axis=y Y=${pct}：等比（1280×720 → 1080×607.5）`, approx(ratio(r, WIDE_W, WIDE_H), 1, 1e-9), true, JSON.stringify(r))
    check(`横素材进竖画布 axis=y Y=${pct}：宽度铺满、垂直摆位 y=${wantY}`,
      approx(r.w, 1080, 1e-6) && approx(r.y, wantY, 1e-6) && approx(r.x, 0, 1e-6), true, JSON.stringify(r))
  }

  // ④ fill 分支 = cover（4:3 源进 16:9 画布：裁切而不是拉伸）
  const cover = rect(1440, 1080, 1920, 1080, { axis: 'fill' })
  check('axis=fill + 4:3 源 → 等比裁切（不是拉伸）', approx(ratio(cover, 1440, 1080), 1, 1e-9), true, JSON.stringify(cover))
  check('axis=fill 的裁切仍然盖满画布（与 stageCoverRect 同一语义）', coversCanvas(cover, 1920, 1080), true, JSON.stringify(cover))

  // ⑤ padding：只改布局盒（内容框 = 画布 − 四周 padding），不裁剪
  const padded = at('x', 0, 50, 1, 100, SW, SH, 1920, 1080)
  check('padding=100 时内容框 = (100,100,1720,880) 内的等比摆放',
    approx(padded.x, 100, 1e-6) && approx(padded.y, 100, 1e-6) && approx(padded.h, 880, 1e-6) && approx(padded.w, 495, 1e-6),
    true, JSON.stringify(padded))

  // ⑥ `scale` 绕**布局盒中心**放大、**再**按余量插值（与旧顺序在 scale=1 或 pct=50 时逐值相同）。
  const scaled = at('x', 50, 50, 1.25, 0, SW, SH, 1920, 1080)
  check('scale=1.25 且两轴居中：中心不动、宽高 ×1.25',
    approx(scaled.w, 759.375, 1e-6) && approx(scaled.h, 1350, 1e-6)
    && approx(scaled.x + scaled.w / 2, 960, 1e-6) && approx(scaled.y + scaled.h / 2, 540, 1e-6),
    true, JSON.stringify(scaled))
  // 旧顺序（先插值再绕插值后的盒心缩放）在 X=0 给 -303.75：画面被推出盒左、右边 1009px 永远是空的。
  // 新顺序的余量 = 盒 − **缩放后**画面 = 1920 − 1215 = 705 ⇒ X=0 恰好贴左、100 恰好贴右。
  const p0 = at('x', 0, 0, 2, 0, SW, SH, 1920, 1080)
  const p100 = at('x', 100, 100, 2, 0, SW, SH, 1920, 1080)
  check('scale=2 且 X=0：贴左 x=0（不是旧顺序的 -303.75）', approx(p0.x, 0, 1e-9), true, JSON.stringify(p0))
  check('scale=2 且 X=100：贴右 x=705（余量 1920−1215）', approx(p100.x, 705, 1e-9), true, JSON.stringify(p100))
  check('scale=2 时 Y 变成**裁切取景**：Y=0 露顶 / Y=100 露底',
    approx(p0.y, 0, 1e-9) && approx(p100.y, -1080, 1e-9), true, `${p0.y} / ${p100.y}`)

  // ⑦ 源尺寸拿不到（0×0）时退回整块画布：不许 NaN 画出空白帧
  check('源尺寸缺失 → 退回整块画布（无 NaN）',
    rect(0, 0, 1920, 1080, { axis: 'x' }), { x: 0, y: 0, w: 1920, h: 1080 })

  // ⑩ **与转译器那份逐值对拍**（12 组参数 × 4 组尺寸 × 4 个数）—— 只改一边就是静默分叉。
  {
    const par = stageParityDiff(translator, html)
    check('模板 ↔ 转译器的 `stageDrawRect` 逐值一致（同一套输入两份实现）',
      par.error || par.diff, 0, par.error || '')
  }
}

// ── ④ 预览 DOM 路径：模糊层必须在主画面之下并被 .on 打开 ─────────────────────
console.log('\n④ 预览 DOM 路径（模板 + WebUI 编辑器）')
{
  const bgWrap = html.indexOf('#bg-video-wrap {')
  const videoWrap = html.indexOf('#video-wrap {')
  const bgZ = Number(/z-index:(\d+)/.exec(html.slice(bgWrap, html.indexOf('}', bgWrap)))?.[1] ?? NaN)
  const videoZ = Number(/z-index:(\d+)/.exec(html.slice(videoWrap, html.indexOf('}', videoWrap)))?.[1] ?? NaN)
  check('#bg-video-wrap 的 z-index 小于 #video-wrap（衬底在视频之下）', bgZ < videoZ, true, `${bgZ} < ${videoZ}`)
  check('模板按 STAGE.bg 打开模糊层（只有 blur 才 on）',
    /if\(bgMode==='blur'\)/.test(html) && html.includes("getElementById('bg-video-wrap').classList.add('on')"), true)
  check('模板的模糊半径/放大倍率来自注入契约（不是写死 28px）',
    html.includes("player.style.setProperty('--bg-blur',(STAGE.blur_px||28)+'px')")
    && html.includes("player.style.setProperty('--bg-img-scale',String(STAGE.blur_img_scale||1.187))"), true)

  const bgEl = previewVue.indexOf('class="bg-video"')
  const wrapEl = previewVue.indexOf('class="video-wrap"')
  check('PolishPreview：模糊层元素排在 .video-wrap 之前（同 z-index 下先画的在下层）', bgEl > 0 && wrapEl > 0 && bgEl < wrapEl, true)
  check('PolishPreview：bgOn 只在 bg=blur 时开模糊层',
    /const bgOn = computed\(\(\) => stageFit\.value\.bg === 'blur'\)/.test(previewVue), true)
  check('PolishPreview：拿不到 _canvas 时退回旧竖屏规则（bg=blur）',
    /props\.config\?\._canvas\?\.stage/.test(previewVue) && previewVue.includes("bg: 'blur'"), true)
  check('PolishPreview：--bg-blur 接的是契约里的 blur_px',
    /'--bg-blur': \(s\.blur_px \|\| 28\) \+ 'px'/.test(previewVue), true)

  // ⚠️ **连续位置（`position_pct`）的 DOM 修正**：CSS 的 `stage-*` 只有三档，两处 DOM 层都必须是
  //    「实测余量 × (pct/100 − class 基线)」这同一条式子 —— 少一处就是「Preview 里 25 档不动、成片动了」
  //    这种不报错的漂移（与 ③c 的 canvas 公式是同一条线性插值的 DOM 版）。
  const vueShift = new RegExp('function measureStageShift[^]*?(px / 100 - 0\\.5)[^]*?(py / 100 - 0\\.5)')
  check('PolishPreview：两轴 DOM 修正在（实测余量 × (pct/100 − 0.5)）', vueShift.test(previewVue), true)
  check('PolishPreview：.fit-box 挂上了修正后的 transform',
    previewVue.includes('ref="fitBoxRef" :style="fitBoxStyle"'), true)
  const tplShift = new RegExp('function stageShiftPx[^]*?(px / 100 - 0\\.5)[^]*?(py / 100 - 0\\.5)')
  check('模板 DOM 层：同一条两轴修正公式（与 WebUI 预览同口径）', tplShift.test(html), true)
  // ⚠️ **这条判据必须限定作用对象**（2026-10 修）：原来写的是裸 `/s\.position_pct/`，不分是哪个 `s`。
  //    而 `PolishPreview.vue` 里现在有**两个** `s`：
  //      · `stageStyle` 的 `s`（= `[stage]`）→ 老单值 `position_pct` **已作废、不许再读**（这才是本判据的本意）
  //      · `capLayerStyle` 的 `s`（= `[style.subtitle]`）→ **字幕**的新字段 `position_pct`，**语义完全无关**
  //    裸匹配会被后者误伤 ⇒ 判据**恒红**，而且红的正是"本仓正在新增的功能"，
  //    真回归反倒被这条常红判据淹没（狼来了）。所以拆成两条：位置那半条照旧，禁用那半条**只扫 stageStyle 的函数体**。
  check('PolishPreview：两轴位置都读契约的 position_x/y_pct',
    /typeof s\.position_x_pct === 'number'/.test(previewVue)
    && /typeof s\.position_y_pct === 'number'/.test(previewVue), true)
  {
    // 取 `stageStyle` 那个 computed 的函数体，只在这里面禁老单值。
    const m = previewVue.match(/const stageStyle = computed\(\(\) => \{[\s\S]*?\n\}\)/)
    check('PolishPreview：stageStyle 不再读老单值 position_pct（字幕的同名字段不算）',
      !!m && !/s\.position_pct/.test(m[0]), true)
  }
}

// ── ⑤ 自证有效：把源码改回修复前，上面的判据必须翻面 ────────────────────────
console.log('\n⑤ 自证有效：变异后判据必须翻面（否则本脚本是空转的）')
{
  // 变异 1：cover 的 Math.max → Math.min（= 变成 contain，边角就填不满了）
  // ⚠️ 必须用**比例不匹配**的素材：同比例时 max 与 min 恰好相等，变异体看不出来（空转）
  const minMath = html.replace('var s=Math.max(dw/sw,dh/sh),w=sw*s,h=sh*s;', 'var s=Math.min(dw/sw,dh/sh),w=sw*s,h=sh*s;')
  check('[变异] stageCoverRect 的替换确实命中了源码', minMath !== html, true)
  const rMin = loadBackdrop(minMath)(CW, CH, {}, 'blur').stageCoverRect(BMP_WIDE.width, BMP_WIDE.height, CW, CH)
  check('[变异] stageCoverRect 改成 contain → 横素材在竖画布上盖不满', coversCanvas(rMin, CW, CH), false,
    `rect=${JSON.stringify(rMin)}（min 会算出 1080×607.5，下方留白就是露出黑色/衬底的地方）`)

  // 变异 2：删掉 `if(STAGE_BG==='none')return;` → 该不画的也要画
  const noGuard = html.replace("if(STAGE_BG==='none')return;\r\n", '').replace("if(STAGE_BG==='none')return;\n", '')
  check('[变异] 去掉 none 守卫（源码确实被改了）', noGuard !== html, true)
  check('[变异] 去掉 none 守卫 → bg=none 也画东西', draws(run(noGuard, 'none').ctx).length > 0, true)

  // 变异 3：背景层画到视频层之后（顺序钉子该翻面）
  const swapped = html.replace('drawStageBackdrop(ctx, videoBitmap);', '/* moved */')
    .replace('ctx.drawImage(videoBitmap, 0, 0, CW, CH);', 'ctx.drawImage(videoBitmap, 0, 0, CW, CH);\n    drawStageBackdrop(ctx, videoBitmap);')
  check('[变异] 背景层挪到视频层之后 → 顺序判据翻面',
    swapped.indexOf('drawStageBackdrop(ctx, videoBitmap)') > swapped.indexOf('ctx.drawImage(videoBitmap, 0, 0, CW, CH)'), true)

  // 变异 4：模糊半径写死 28（滑杆失效）
  const hardBlur = html.replace("ctx.filter='blur('+(STAGE.blur_px||0)+'px)", "ctx.filter='blur(28px)")
  check('[变异] 模糊半径写死 28 → 「跟随 blur_px」判据翻面',
    /blur\(40px\)/.test(draws(run(hardBlur, 'blur', { blur_px: 40, blur_img_scale: 1.187 }).ctx)[0]?.filter || ''), false)

  // 变异 5：去掉 save/restore（filter 泄漏到视频层）
  const noSave = html.replace("ctx.save();\r\n    ctx.filter=", 'ctx.filter=').replace("ctx.save();\n    ctx.filter=", 'ctx.filter=')
  check('[变异] 去掉衬底的 save/restore（源码确实被改了）', noSave !== html, true)
  if (noSave !== html) {
    const c = run(noSave, 'blur', { blur_px: 28, blur_img_scale: 1.187 }).ctx
    const nSave = c.calls.filter((x) => x.op === 'save').length
    const nRestore = c.calls.filter((x) => x.op === 'restore').length
    check('[变异] 没有 save/restore 时配对判据翻面', nSave === nRestore && nSave > 0, false)
  }

  // 变异 6：axis=x 分支改回「拉伸」（= 用户报的现象）—— 等比判据必须翻面
  const stretch = html.replace('h=bhgt; w=bw*h/bh;', 'w=bwid; h=bhgt;')
  check('[变异] 拉伸替换确实命中源码', stretch !== html, true)
  if (stretch !== html) {
    const r = loadRect(stretch)(720, 1280, 1920, 1080, { axis: 'x', position: 'left', position_pct: 0 })
    check('[变异] 改回拉伸 → 等比判据翻面（w/h 不再等于 9:16）',
      approx(r.w / r.h / (720 / 1280), 1, 1e-9), false, JSON.stringify(r))
  }

  // 变异 8：X 轴插值改回**硬居中**（2026-10-05 两轴 xy 的核心：拖了没反应 + 预览与成片一起分叉）
  const centeredX = html.replace('x:bx+(bwid-w)*px, y:by+(bhgt-h)*py', 'x:bx+(bwid-w)/2, y:by+(bhgt-h)*py')
  check('[变异] X 轴硬居中的替换确实命中了源码', centeredX !== html, true)
  if (centeredX !== html) {
    const r = loadRect(centeredX)(720, 1280, 1920, 1080, { axis: 'x', position_x_pct: 0, position_y_pct: 50 })
    check('[变异] X 轴改回硬居中 → X=0 不再贴左（判据翻面）', approx(r.x, 0, 1e-6), false, JSON.stringify(r))
  }

  // 变异 9：**只改转译器那一份** —— 跨实现对拍必须翻面。S2/S3 是一对实现：只改一边 =
  //    预览/成片对得上、底座那条腿对不上，而且不报错。
  const transMut = translator.replace('x = bx + (bwid - w) * (stagePctX / 100);', 'x = bx + (bwid - w) / 2;')
  check('[变异] 转译器 X 轴插值的替换确实命中了源码', transMut !== translator, true)
  if (transMut !== translator) {
    const par = stageParityDiff(transMut, html)
    // 变异后**必须不相等**（种进 20 个数）：期望 `true`，不是 0
    check('[变异] 转译器改回硬居中 → 跨实现对拍翻面', par.diff > 0, true, par.error || '差异 0 就是没抓住')
  }
  // 变异 7：删掉 CTM 摆位（矩形算了不用 = 视频层还是硬铺满）
  const noCtm = html.replace('ctx.translate(sr.x, sr.y); ctx.scale(sr.w / CW, sr.h / CH);', '')
  check('[变异] 去掉 CTM 摆位（源码确实被改了）', noCtm !== html, true)
  check('[变异] 去掉 CTM 摆位 → 消费面判据翻面',
    stripComments(noCtm).indexOf('ctx.translate(sr.x, sr.y)') >= 0, false)
}

console.log(failed ? `\n${failed} 项失败` : '\n全部通过')
process.exit(failed ? 1 : 0)
