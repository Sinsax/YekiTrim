#!/usr/bin/env node
// **39 档动效：预览侧「真的会动吗」逐档判据**（2026-10-13，P4 补）。
//
// # 为什么必须有一条这样的判据
//
// 39 档在**三处**各有实现，而它们各自"看起来都对"：
//   ① 转译器 `tools/polish-to-dhampir.mjs::ANIM_PRESETS`（权威，写进 toml）；
//   ② 界面 `TimelineEditor.vue`（给用户选）；
//   ③ 预览 `webui/src/polish/engine.js`（**给用户看**）。
//
// `check-anim-presets-parity.mjs` 只对拍**档名集合**（"这一档存不存在"）。
// 它**证明不了"这一档在预览里真的会动"** —— 而这正是用户报的
// 「播放没有效果」：档名在、下拉能选、成片会动，**预览纹丝不动**。
//
// 实测就是这么一回事：`charTransformAt` **导出了、全仓没有一处 import**
// ⇒ C 组逐字变换（15 档）+ D 组逐字错峰（4 档）**共 19 档在预览里完全不动**，
// 而任何"档名对不对"的判据都是全绿的。
//
// ⇒ 本条判据**真调引擎函数**，逐档在若干帧上求值，断言：
//   * **会动的那些档**：不同帧之间的形状**确实不同**（不是"渲染出来了"就算）；
//   * **静态的那些档**（`fade_only` 等）：形状**一致**（不许假装动了）。
//
// # 一条判据红/绿都不算数，必须**变异自证**
//
// ⚠️⚠️ 本仓本月已经栽过**四次**"判据绿了但没测到东西"（其中两次就在这次改动里：
//    颜色测试抄了一遍合成代码、描边测试两层都全不透明 ⇒ 反序也绿）。
//    ⇒ 本脚本 `--mutate` 把生产实现**真的改坏**（摘掉逐字那条路），
//      **必须**看到判据翻红 —— 否则这条判据等于没有。
//
// 零依赖、不需要起服务（与 `webui/scripts/check-*.mjs` 同一套做法）。
import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const HERE = path.dirname(fileURLToPath(import.meta.url))
const ROOT = path.join(HERE, '..')
const ENGINE = path.join(ROOT, 'webui', 'src', 'polish', 'engine.js')
const PREVIEW = path.join(ROOT, 'webui', 'src', 'components', 'PolishPreview.vue')

// ⚠️ **`--mutate-sub` 也算变异**（第一版只认 `--mutate` ⇒ 第二个变异根本没生效，
//    而脚本照旧打印"✓ 全绿" —— 看起来像"判据抓不住字幕那条腿"，
//    其实是**变异压根没跑**。这种"自证自己失效"的形态最难发现）。
const MUTATE = process.argv.includes('--mutate') || process.argv.includes('--mutate-sub')

// ---- 变异：把生产的逐字那条路摘掉（判据必须翻红）----
//
// ⚠️ 改的是**生产文件**，跑完必须还原（`try/finally`）。
// ⚠️ 变异体要"看起来合理"：把 `perCharSpans` 直接 `return null`，
//    正是"忘了接线"那种真实事故的形态（不是把函数删掉 —— 那会语法错，
//    判据红得没有意义）。
// ⚠️⚠️ **变异写的是"副本"，绝不改生产文件**（2026-10-13 踩过一次）：
//    第一版直接改 `engine.js` + 在 `finally` 里用 `git checkout` 还原 ——
//    而跑的时候那份文件**还没提交** ⇒ checkout 把它**整份退回 HEAD**，
//    本次改动的逐字实现**全没了**。改成"改副本、import 副本"之后，
//    生产文件**从头到尾一个字节都不会被碰到**（连还原都不需要）。
let mutated = false
let importUrl = new URL('../webui/src/polish/engine.js', import.meta.url).href
if (MUTATE) {
  const orig = fs.readFileSync(ENGINE, 'utf8')
  const marker = 'export function perCharSpans('
  const i = orig.indexOf(marker)
  if (i < 0) {
    console.log('✗ 变异失败：engine.js 里找不到 `perCharSpans`（判据的前提没了）')
    process.exit(1)
  }
  // ⚠️ 锚点必须取**函数签名之后**的那个 `{`，不能取 `indexOf('{', i)` ——
  //    函数上面那段 JSDoc 里就有一个 `{`（`@returns {null | Array<...>}`），
  //    取到它会把 `return null` 插进**注释块里**，于是副本整个语法错
  //    （实测：`Unexpected token 'null'`，而且看起来像"变异体本身写错了"）。
  const sigStart = orig.indexOf('(', i)
  const sigEnd = orig.indexOf(')', sigStart)
  const bodyStart = orig.indexOf('{', sigEnd)
  // ⚠️ **两处变异**，覆盖**两条腿**（少了任一条就测不到对应那个分支）：
  //    ① `--mutate`：把 `perCharSpans` 直接返回 null ⇒ 元素与字幕的逐字**一起**断；
  //    ② `--mutate-sub`：把 `subAnimFields` 直接返回空 ⇒ **只**断字幕那条腿，
  //       而 ③④ 仍然全绿 —— 这正是"只补了元素、漏了字幕"那个真实事故的形态。
  //    ⇒ **⑤ 是唯一能抓到 ② 的判据**（它专查 `subsAt` 那条分支）。
  const WHICH_SUB = process.argv.includes('--mutate-sub')
  let patched
  if (WHICH_SUB) {
    const j = orig.indexOf('function subAnimFields(')
    if (j < 0) {
      console.log('✗ 变异失败：engine.js 里找不到 `subAnimFields`')
      process.exit(1)
    }
    const s2 = orig.indexOf('(', j)
    const e2 = orig.indexOf(')', s2)
    const b2 = orig.indexOf('{', e2)
    patched = orig.slice(0, b2 + 1) + '\n  return {} // <-- 变异体（只断字幕那条腿）\n' + orig.slice(b2 + 1)
  } else {
    patched = orig.slice(0, bodyStart + 1) + '\n  return null // <-- 变异体\n' + orig.slice(bodyStart + 1)
  }
  // ⚠️ 副本放在 `webui/src/polish/` 下（**换个文件名**），这样副本里的
  //    `./canvas.js` 那些相对 import 照样解析得到。
  const copyPath = path.join(ROOT, 'webui', 'src', 'polish', '__mutant_engine.mjs')
  fs.writeFileSync(copyPath, patched)
  mutated = true
  importUrl = new URL('../webui/src/polish/__mutant_engine.mjs', import.meta.url).href
  console.log(WHICH_SUB
    ? '【变异·字幕】已把 `subAnimFields` 改成直接 return {}（只断字幕那条腿，只动物副本）'
    : '【变异·逐字】已把 `perCharSpans` 改成直接 return null（逐字那条路被摘掉，只动物副本）')
}

const fails = []
let checks = 0
const ok = (cond, what) => {
  checks++
  if (!cond) fails.push(what)
}

try {
  const E = await import(importUrl)

  // ---- ① 前提：那几个函数真的导出（判据自己不许空转）----
  for (const fn of ['perCharSpans', 'charTransformAt', 'animPerChar', 'animMotion',
                    'subsAt', 'buildIndex', 'textTiltTransform', 'ANIM_PRESETS']) {
    ok(E[fn] !== undefined, `① 引擎没有导出 \`${fn}\` —— 判据的前提没了`)
  }

  const allNames = Object.keys(E.ANIM_PRESETS || {}).sort()
  ok(allNames.length === 39, `① 引擎的 \`ANIM_PRESETS\` 应当 39 档，实得 ${allNames.length}`)

  // ---- ② 逐档分类：这一档**在预览里**该不该动 ----
  //
  // 判据的口径：一档"会动" = 要么**随时间变化**的逐字 spec，要么整块时间函数
  // （`animMotion` 摊出来的弹入/抖动/摇摆）。
  //
  // ⚠️⚠️ **`perCharFrames > 0` 本身不等于"会动"**（第一版就是这么归类的，
  //    结果报了一片假红，而**真因是判据的模型错了、不是代码错**）：
  //    「大小字」那几档（`size_last` / `size_first` / `size_mid`）靠
  //    `scaleSpread`（**按序号的字号乘数**）—— 四个端点**全都相等**
  //    ⇒ 逐字结果**与帧无关**，是一条**静止的字号阶梯**。
  //    那正是它在成片里的样子（"只有最后一个字大"），**不是 bug**。
  //    ⇒ 唯一可靠的判定：**真在几个帧上求一遍、看形状变不变**。
  // ⚠️ 静态档是**少数但确实存在**（`fade_only` 只淡出、`tilt_*` 只是姿态、
  //    大小字那条阶梯）。把它们算成"该动"会让判据**永远红**
  //    （那时人就会去放宽判据 —— 这是最坏的结局）。
  const FRAMES = [0, 1, 4, 12, 60]
  const SAMPLE = '逐字动效样本'

  /** 这一档的**逐字**腿在若干帧上的形状指纹（不逐字时 `null`）。 */
  function charShapes(name) {
    const per = E.animPerChar(name)
    if (!per) return null
    return FRAMES.map((f) => {
      const spans = E.perCharSpans(SAMPLE, {
        perChar: per, perCharFrame: f, perCharFps: 30, fontPx: 72
      })
      return spans ? spans.map((s) => s.style.transform || '-').join('|') : null
    })
  }

  const moving = []
  const staticOnes = []
  const shapesByTier = {}
  for (const name of allNames) {
    const shapes = charShapes(name)
    shapesByTier[name] = shapes
    // ⚠️ "逐字会动" = 逐字那条路**真的产出**、且**帧之间形状确实不同**。
    const perCharMoves = !!shapes && new Set(shapes.filter(Boolean)).size > 1
    const mo = E.animMotion(null, name)
    const movesBlock = !!(mo && ((mo.scaleInMs > 0) || (mo.shakePx > 0) || (mo.swayDeg > 0)))
    if (movesBlock || perCharMoves) moving.push(name)
    else staticOnes.push(name)
  }
  // ⚠️ **静态的那一批是登记在案的 16 档**，而且**每一档都有理由**（不是"判据放宽"）：
  //    * `tilt_*` 3 档 —— 改的是**静态姿态**（`rotation`），本来就不是动效；
  //    * `fade_only` —— "只淡出"，动的是**不透明度**（不在形状指纹里）；
  //    * 大小字 / 变小 / 放大 **9 档** —— 用 `scaleSpread`（**按序号**的字号乘数），
  //      四个端点**全相等** ⇒ 是一条**静止的字号阶梯**。
  //      ⚠️ 这正是它们**该有的样子**（engine.js 里写着："用 `scale_to` 是**整行**一起
  //      变小、字与字之间没有差别 ⇒ 不是'逐字'"）⇒ 与帧无关是**设计**，不是漏了时间维度。
  //    * `vtext_*` 3 档 —— 改的是**排版方向**（底座 `SubtitleStyle::vertical`）。
  //    ⇒ 判据钉的是**这一批恰好是这些**：多一档说明有档退化了、少一档说明登记过时了。
  const STATIC_BY_DESIGN = [
    'fade_only', 'tilt_rev', 'tilt_sm', 'tilt_std',
    'grow_asc', 'grow_burst', 'grow_first',
    'shrink_desc', 'shrink_drop', 'shrink_tail',
    'size_first', 'size_last', 'size_mid',
    'vtext_bold', 'vtext_shrink', 'vtext_thin'
  ].sort()
  const gotStatic = [...staticOnes].sort()
  ok(JSON.stringify(gotStatic) === JSON.stringify(STATIC_BY_DESIGN),
    `② "静态"的那一批与登记不符。
      实得 ${JSON.stringify(gotStatic)}
` +
    `      应当 ${JSON.stringify(STATIC_BY_DESIGN)}
` +
    `      （多出来的 = 某一档退化成不动了；少了的 = 登记过时或判据漏了）`)
  ok(moving.length === 39 - STATIC_BY_DESIGN.length,
    `② "会动"的档应当 ${39 - STATIC_BY_DESIGN.length} 档，实得 ${moving.length}`)

  // ---- ③ 逐档求值：**会动的档，不同帧形状真的不同** ----
  //
  // 形状指纹 = 每个字自己的 `transform` 串。帧之间**只要有一个字不同**就算动了。
  // ⚠️ 用**帧**不是秒（`charTransformAt` 的第四参是帧，底座同口径）。
  // ⚠️ 取 0 / 1 / 4 / 12 / 60 这几帧：覆盖"还没开始 / 进行中 / 早就完了"。
  for (const name of allNames) {
    const shapes = shapesByTier[name]
    if (moving.includes(name)) {
      const mo = E.animMotion(null, name)
      const blockMoves = !!(mo && ((mo.scaleInMs > 0) || (mo.shakePx > 0) || (mo.swayDeg > 0)))
      const perCharMoves = !!shapes && new Set(shapes.filter(Boolean)).size > 1
      ok(perCharMoves || blockMoves,
        `③ \`${name}\` 被归成"会动"，但逐字与整块**两条腿都无变化** ` +
        `（逐字指纹: ${JSON.stringify(shapes)}）`)
    }
    // ⚠️ 反向：**有逐字 spec、且 spec 自己声明要随时间走**，却每帧同一形状 ⇒
    //    正是"忘了接线 / `scale(NaN)` 让整条 transform 被浏览器丢弃"的症状。
    //    ⚠️ 判据是"**声明的**是时间型"（`perCharFrames > 0` **且**端点不相等），
    //       不是"`perCharFrames > 0`"—— 大小字那三档帧数 14、但端点是 0（见 ②）。
    const per = E.animPerChar(name)
    if (per && per.perCharFrames > 0) {
      const varies = (a, b) => Math.abs(a - b) > 1e-6
      const timeVarying = varies(per.scaleFrom, per.scaleTo) || varies(per.rotationFrom, per.rotationTo) ||
        varies(per.offsetXFrom, per.offsetXTo) || varies(per.offsetYFrom, per.offsetYTo) ||
        varies(per.spreadXFrom, per.spreadXTo) || varies(per.spreadYFrom, per.spreadYTo)
      if (timeVarying) {
        const anyNonNull = !!shapes && shapes.some((s) => s !== null)
        const perCharMoves = !!shapes && new Set(shapes.filter(Boolean)).size > 1
        ok(anyNonNull,
          `③ \`${name}\` 声明了时间型逐字 spec，但逐字那条路**一个 span 都没产出** —— 没接上`)
        ok(perCharMoves,
          `③ \`${name}\` 是时间型逐字档，但**每一帧形状都一样**（应当随时间变）`)
      }
    }
  }

  // ---- ④ 逐字**确实逐字**：同一帧里各个字**形状不同** ----
  //
  // ⚠️ 这是"整块套在父 div 上"那个退化的唯一机器判据：
  //    若把逐字变换算成"整行一个值"，那么同一帧里所有字的 transform **完全相同**，
  //    而上面 ③ 可能仍然是绿的（帧之间不同不代表字之间不同）。
  //    实测这就是最容易犯的错（"19 档全部看不出差别"正是这个形态）。
  {
    const samples = ['arc_up', 'arc_wave', 'arc_down', 'scatter_sm', 'scatter_lg',
                     'size_last', 'size_first', 'size_mid', 'shrink_desc', 'grow_asc',
                     'shrink_tail', 'grow_burst', 'scatter_shake', 'shrink_drop',
                     'grow_first', 'hop_std', 'hop_fast', 'hop_big', 'pop_perchar']
    let checked = 0
    const flat = []
    for (const name of samples) {
      const per = E.animPerChar(name)
      if (!per) continue
      // 取"动画进行到一半"的那一帧（错峰最明显的时候）。
      const f = per.perCharFrames * 0.5
      const spans = E.perCharSpans(SAMPLE, { perChar: per, perCharFrame: f, perCharFps: 30, fontPx: 72 })
      if (!spans) { flat.push(`${name}(null)`); continue }
      const uniq = new Set(spans.map((s) => s.style.transform || '-'))
      checked++
      if (uniq.size < 2) flat.push(name)
    }
    ok(checked >= 19,
      `④ 逐字档只真跑了 ${checked} 档（应当 ≥19：C 组 15 + D 组 4）`)
    ok(flat.length === 0,
      `④ 这些档**同一帧里每个字的变换完全相同** —— 逐字退化成整行了：${JSON.stringify(flat)}`)
  }

  // ---- ⑤ 字幕那条路：`subsAt` 真的带出动画字段 ----
  //
  // ⚠️ 字幕是**另一条分支**（`subsAt`），与文字元素（`getElements`）不是一个函数。
  //    只补了元素那条、漏了字幕那条，症状是"文字元素会动、字幕不动"
  //    —— 而上面 ③④ 全是绿的。
  {
    const subs = [
      { start: 1, end: 4, text: '普通', style: { name: 'comment' }, pos: 'bottom' },
      { start: 1, end: 4, text: '弧形', style: { name: 'comment' }, pos: 'bottom', anim: 'arc_up' }
    ]
    const idx = E.buildIndex({ config: {}, subtitles: subs, danmaku: [], duration: 10, fps: 30 })
    // ⚠️ **取"动画进行中"的时刻**（不是已经走完的）：`arc_up` 是
    //    `perCharFrames: 8` + `staggerFrames: 3` ⇒ 两个字的动画在第 ~11 帧就走完了，
    //    之后再取景**两个字本来就该都在终值上**（那不是 bug，是"动画结束了"）。
    //    第一版在 1.5s（= 第 15 帧）取 ⇒ 判据假红。
    //    ⇒ 取 1.1s（= 第 3 帧：第一个字刚起步、第二个字还没轮到）。
    const rows = E.subsAt(idx, 1.1)
    ok(rows.length === 2, `⑤ \`subsAt\` 应当返回 2 条，实得 ${rows.length}`)

    const plain = rows.find((r) => (r.fullText || '') === '普通')
    const arc = rows.find((r) => (r.fullText || '') === '弧形')
    ok(!!plain && !!arc, '⑤ 找不到那两条字幕（`fullText` 没带出来？）')

    // ⚠️ **R4：没 `anim` 的那条，键集与改造前逐字相同**。
    if (plain) {
      const keys = Object.keys(plain).sort().join(',')
      ok(keys === 'fullText,opacity,parts,scale,yOff',
        `⑤ 没动效档的字幕**多出了键**（老工程产物形状变了）：实得 ${keys}`)
    }
    // 有 `anim` 的那条必须带出三样。
    if (arc) {
      ok(arc.animName === 'arc_up', `⑤ 有 \`anim\` 的字幕没带出 \`animName\`：实得 ${JSON.stringify(arc.animName)}`)
      ok(!!arc.perChar, '⑤ 有**逐字**档的字幕没带出 `perChar` —— 字幕的逐字那条路断了')
      ok(arc.perCharFrame > 0, `⑤ 字幕的 \`perCharFrame\` 没有随时刻推进：实得 ${arc.perCharFrame}`)
      ok(arc.perCharFps === 30, `⑤ 字幕的 \`perCharFps\` 不对：实得 ${arc.perCharFps}`)
      // ⚠️ 逐字必须**真的逐字**（同上 ④ 的理由）。
      const spans = E.perCharSpans(arc.fullText, {
        perChar: arc.perChar, perCharFrame: arc.perCharFrame,
        perCharFps: arc.perCharFps, fontPx: 72
      })
      ok(!!spans && new Set(spans.map((s) => s.style.transform || '-')).size >= 2,
        '⑤ 字幕的逐字变换在**同一帧里每个字相同** —— 退化成整行了')
    }

    // ⚠️ 认不出的档名 ⇒ 不许带出 `perChar`（成片也不认这个档，两边要一致）。
    const bad = E.buildIndex({ config: {}, subtitles: [
      { start: 1, end: 4, text: '坏档', style: { name: 'comment' }, pos: 'bottom', anim: 'bogus_xyz' }
    ], danmaku: [], duration: 10, fps: 30 })
    const badRow = E.subsAt(bad, 1.5)[0]
    ok(badRow && badRow.perChar === undefined,
      '⑤ 认不出的档名**不该**带出 `perChar`（成片也不认它，两边必须一致）')
  }

  // ---- ⑥ 接线：预览组件真的消费了这两个字段 ----
  //
  // ⚠️ "函数写好、没人调"是本仓的头号事故形态（这次 `charTransformAt`
  //    导出了整整一段时间没人 import，就是它）。⇒ 静态查一遍消费点。
  {
    const pv = fs.readFileSync(PREVIEW, 'utf8')
    // 只对**真标记**断言（剥注释），理由与 `check-anim-dropdown.mjs` 同：
    // "我把逐字加上去了"这句话本身必须写在注释里。
    const pvMarkup = pv.replace(/<!--[\s\S]*?-->/g, '')
    ok(/perCharSpans\s*\(/.test(pv), '⑥ 预览组件没有调用 `perCharSpans` —— 逐字没接上')
    ok(/from '\.\.\/polish\/engine'/.test(pv) && /perCharSpans/.test(pv.split('\n').find((l) => l.includes("from '../polish/engine'")) || ''),
      '⑥ 预览组件没有把 `perCharSpans` import 进来')
    ok(/v-for="\(ch, ci\) in sp\.chars"/.test(pvMarkup) || /v-for="\(c, ci\) in tx\.spans"/.test(pvMarkup),
      '⑥ 模板没有**逐字**拆 `<span>` —— 整块套在父元素上等于没做逐字')
    ok(/textTiltTransform\(/.test(pvMarkup), '⑥ 字幕/元素没有走 `textTiltTransform` —— 整块动效接不上')
    // ⚠️ 字幕的整块变换必须**与元素同一条式子**（不许另写一份）。
    // ⚠️⚠️ 2026-10-13 两级化：这里必须取**样式轴**（`styleAnimName`）——
    //    `tilt_*` / `slam_squash` 的静态倾斜是**形态**那半边，取 `animName`
    //    （或动效轴）会让"只写了 `effect_anim`"的字幕拿不到倾斜。
    ok(/animName:\s*c\.styleAnimName/.test(pvMarkup),
      '⑥ 字幕那条没有把**样式轴**（`c.styleAnimName`）交给 `textTiltTransform` ——\n' +
      '     字幕的静态形态（倾斜 / 压扁）不生效。⚠️ 别退回 `c.animName`：\n' +
      '     两轴化之后它只代表"两轴里随便哪一个"，形态那半会丢。')
    // ⚠️ **字幕逐字那条路必须真的接上**（`spans` + `perCharSpans`）。
    ok(/perCharSpans\s*\(\s*c\.fullText/.test(pvMarkup) || /c\.perChar/.test(pvMarkup),
      '⑥ 字幕没有消费 `perChar` —— 19 档逐字在**字幕**上不会动\n' +
      '     ⚠️ `subAnimFields` 带出来了，但组件不读 = 白带（本仓的经典空转）')
  }

  // ---- ⑤-bis ⚠️ **字幕那条路也要两轴**（2026-10-13，用户报「前端预览没有实现效果」）----
  //
  // ⚠️⚠️ 这一条是**补出来的**，因为真出过事：两级化当天，
  //    **元素那条路改了、字幕这条路漏了** —— `subAnimFields` 仍然只读老 `anim`。
  //    后果：用户给**字幕**选「竖排 + 抖动」（写两个新字段）⇒ 预览**完全没效果**、
  //    而成片有。用户的反馈就是「前端预览没有实现效果」。
  //
  // ⚠️ 为什么上面的 ③④⑤ 全都发现不了：
  //    · ③④ 走的是**元素**那条路（`buildIndex` 的 element 分支）；
  //    · ⑤ 只喂了老 `anim`，**从没喂过 `style_anim` / `effect_anim`**。
  //    ⇒ 同一个"两个消费者只改了一个"的空转，本仓犯过多次（判据绿 != 功能对）。
  {
    const mkSub = (extra) => {
      const idx = E.buildIndex({
        config: { canvas: { width: 1920, height: 1080, fps: 30 }, elements: {}, segments: [] },
        subtitles: [{ start: 1, end: 9, text: '竖排且抖', style: { name: 'comment' },
                      pos: 'bottom', ...extra }],
        danmaku: [], duration: 10, fps: 30
      })
      return E.subsAt(idx, 1.5)[0]
    }

    // ① 两栏都写 ⇒ 字幕也要拿到**两样**（形态 + 动效）
    const both = mkSub({ style_anim: 'vtext_bold', effect_anim: 'shake_std' })
    ok(!!both, '⑤-bis 字幕行没取到')
    if (both) {
      ok(both.styleAnimName === 'vtext_bold',
        '⑤-bis 字幕**没拿到** `style_anim` —— 形态那半边丢了：' +
        `实得 ${JSON.stringify(both.styleAnimName)}\n` +
        '     ⚠️ 成片里字幕会竖排、预览里横着（两边不一致）')
      ok(both.effectAnimName === 'shake_std',
        '⑤-bis 字幕**没拿到** `effect_anim` —— 动效那半边丢了：' +
        `实得 ${JSON.stringify(both.effectAnimName)}\n` +
        '     ⚠️ 用户选了抖动、成片里抖、预览里不抖 —— 就是用户报的「预览没有实现效果」')
      // ⚠️ 核**实际参数**而不只是档名（档名带出来了、参数没接上一样没效果）。
      ok(both.textMotion?.shakePx > 0,
        '⑤-bis 字幕的 `textMotion.shakePx` 是 0 ⇒ 预览里不会抖：' +
        `${JSON.stringify(both.textMotion?.shakePx)}`)
    }

    // ② 字幕的**逐字**走样式轴（`hop_*` / `pop_perchar` / `scatter_*`）
    const perchar = mkSub({ anim: 'hop_std' })
    ok(!!perchar?.perChar,
      '⑤-bis 字幕的逐字 spec 没带出来 —— 19 档逐字在字幕上不会动')
    ok(typeof perchar?.perCharFrame === 'number',
      '⑤-bis 字幕的 `perCharFrame` 不是数：' + JSON.stringify(perchar?.perCharFrame))

    // ③ **R4：没有任何档时，一个键都不加**（老工程形状逐字节不变）
    const blank = mkSub({})
    const leaked = Object.keys(blank).filter(
      (k) => !['parts', 'fullText', 'opacity', 'yOff', 'scale'].includes(k))
    ok(leaked.length === 0,
      '⑤-bis 没有动效档的字幕**多出了键**（老工程产物形状变了）：' + JSON.stringify(leaked))
  }

  // ---- ⑤-ter ⚠️ **形态那半边的"实际参数"必须真的被消费** ----
  //
  // ⚠️⚠️ 这一条也是**补出来的**，理由与 ⑤-bis 同源、但位置更深一层：
  //    ⑤-bis 只保证 `subAnimFields` 把 `styleAnimName` **带出来**了；
  //    而组件第一版**只拿它做 `textTiltTransform`（倾斜/压扁）** ——
  //    `tilt_*` 只是 39 档里的**一族**。`vtext_*` 三档给的是
  //    `vertical` / `fontFamily` / `fontWeight` / `safeWidthRatio`，
  //    **一个都没被读过** ⇒ 用户选「竖排」，预览里**横着**、成片里竖排。
  //    （这正是"档名带出来了 ≠ 参数接上了"，本仓 ⑤-bis 那一类空转的第二层。）
  //
  // ⚠️ 判据必须盯**组件真读没读**，不能只盯 engine 有没有给 —— 给了没人读 = 白给。
  {
    const pv = fs.readFileSync(PREVIEW, 'utf8')
    // 只对**真标记/真代码**断言（剥注释）：上面"这里曾漏读 vertical"那句话
    // 本身就会含 `vertical` 字样，不剥会把说明当成实现。
    const pvCode = pv.replace(/<!--[\s\S]*?-->/g, '').replace(/\/\*[\s\S]*?\*\//g, '')
      .replace(/^\s*\/\/.*$/gm, '')
    ok(/animPreset\s*\(/.test(pvCode),
      '⑤-ter 预览组件没有调用 `animPreset` —— 形态档的**实际参数**（vertical/字体/安全宽）读不到\n' +
      '     ⚠️ 只把档名交给 `textTiltTransform` 只能覆盖 `tilt_*` 一族')
    ok(/\.vertical\b/.test(pvCode),
      '⑤-ter 预览**没有读 `vertical`** —— 选「竖排」时预览里会横着（成片里是竖的）')
    ok(/writingMode/.test(pvCode),
      '⑤-ter 没有把 `vertical` 落成 `writing-mode` —— 读出来不用 = 白读')
    ok(/safeWidthRatio/.test(pvCode),
      '⑤-ter 折行**没有跟这一档的安全宽**（`vtext_*` 是 0.86）——\n' +
      '     断行位置会与成片不一致（`check-caption-wrap.mjs` 钉的那类漂移）')
    ok(/formStyle/.test(pvCode),
      '⑤-ter 算出来的 `formStyle` 没有在模板上套用 —— 算出来不渲染 = 白算')
  }

  // ---- ⑦ ⚠️ **两轴组合**：`style_anim` + `effect_anim` 同时生效（2026-10-13）----
  //
  // 判据 ③④⑤⑥ 都是**单档**的（老 `anim`，或只顾到接线）。两级化之后用户能做的是
  // **组合**：「竖排 + 抖动」。这一条是那个能力在预览侧的**唯一**判据。
  //
  // ⚠️ 为什么必须有它（三条都不会自己报错）：
  //   · 预览只读老 `anim` ⇒ 新界面写的两个字段它**看不见** ⇒ 预览完全没效果、
  //     成片有（这也正是 ③④⑤ 全绿时发现不了的那一类）；
  //   · 两轴混用（动效那路读 `styleAnim`）⇒ 组合里**只有一半**生效；
  //   · 形态那路读 `effectAnim` ⇒ 竖排丢失、只剩抖。
  {
    const mk = (ev) => E.buildIndex({
      config: {
        canvas: { width: 1920, height: 1080, fps: 30 }, elements: {},
        segments: [{ start: 0, end: 10, fx: 'low', events: [{
          type: 'element', kind: 'text', text: '竖排且抖', time: 1, end: 4,
          style: 'comment', pos: 'bottom',
          source: 'comment|44|2|255,226,122,255|26,26,26,255|700|1', ...ev
        }] }]
      },
      subtitles: [], danmaku: [], duration: 10, fps: 30
    })
    const first = (idx) => (idx.element || [])[0]

    // ① 两栏都写 ⇒ 两样参数都拿到（用户要的那个组合）
    const both = first(mk({ style_anim: 'vtext_bold', effect_anim: 'shake_std' }))
    ok(!!both, '⑦ 组合档的文字元素没进索引')
    if (both) {
      ok(both.styleAnimName === 'vtext_bold',
        `⑦ 组合档没把 \`style_anim\` 带到预览：实得 ${JSON.stringify(both.styleAnimName)}`)
      ok(both.effectAnimName === 'shake_std',
        `⑦ 组合档没把 \`effect_anim\` 带到预览：实得 ${JSON.stringify(both.effectAnimName)}`)
      // ⚠️ 核的是**实际参数**而不是档名：档名带出来了、参数没接上同样没效果。
      const st = E.animPreset(both.styleAnimName)
      ok(st?.vertical === true,
        `⑦ 组合档的**形态**参数没接上（\`vtext_bold\` 应当是竖排）：实得 ${JSON.stringify(st?.vertical)}`)
      ok(both.textMotion?.shakePx > 0,
        `⑦ 组合档的**动效**参数没接上（\`shake_std\` 应当有 \`shakePx\`）：` +
        `实得 ${JSON.stringify(both.textMotion?.shakePx)}`)
    }

    // ② 反序也要成立（两轴不许互相清掉）
    const rev = first(mk({ effect_anim: 'shake_std', style_anim: 'vtext_bold' }))
    ok(rev?.styleAnimName === 'vtext_bold' && rev?.effectAnimName === 'shake_std',
      '⑦ 反序写两个字段结果不同（两轴互相清掉了？）：' +
      JSON.stringify([rev?.styleAnimName, rev?.effectAnimName]))

    // ③ 只写样式轴 ⇒ 动效那半边**必须为空**（不许串到别的档上去）
    const onlyStyle = first(mk({ style_anim: 'vtext_bold' }))
    ok(onlyStyle?.effectAnimName === '',
      `⑦ 只写 \`style_anim\` 时动效轴应当为空：实得 ${JSON.stringify(onlyStyle?.effectAnimName)}`)
    ok(!(onlyStyle?.textMotion?.shakePx > 0),
      `⑦ 只写 \`style_anim\` 却抖起来了（动效轴串到样式档了）：` +
      `${JSON.stringify(onlyStyle?.textMotion?.shakePx)}`)

    // ④ 老格式 `anim`（单档）在预览里必须**与等价的新格式一致**（R4 的预览侧）
    for (const n of ['vtext_bold', 'shake_std', 'scatter_shake', 'fade_only']) {
      const legacy = first(mk({ anim: n }))
      const sp = E.splitAnimTwoAxis(n)
      const viaNew = first(mk(sp.style
        ? { style_anim: n, effect_anim: sp.effect }
        : { style_anim: '', effect_anim: n }))
      ok(legacy?.animName === viaNew?.animName &&
        legacy?.effectAnimName === viaNew?.effectAnimName &&
        JSON.stringify(legacy?.textMotion) === JSON.stringify(viaNew?.textMotion),
        `⑦ 老格式 \`anim="${n}"\` 与等价的新格式在预览里**不一致**\n` +
        `     老：${JSON.stringify([legacy?.animName, legacy?.effectAnimName, legacy?.textMotion])}\n` +
        `     新：${JSON.stringify([viaNew?.animName, viaNew?.effectAnimName, viaNew?.textMotion])}`)
    }
  }
} finally {
  // ⚠️ 只删**副本**。生产文件从头到尾没被碰过 ⇒ 不需要"还原"
  //    （这正是本脚本第一版那个"变异把未提交的实现冲掉"事故的修法）。
  if (mutated) {
    const copyPath = path.join(ROOT, 'webui', 'src', 'polish', '__mutant_engine.mjs')
    try { fs.unlinkSync(copyPath) } catch { /* 本来就没有就算了 */ }
    if (fs.existsSync(copyPath)) {
      console.log('✗ 副本没删掉：' + copyPath + '（请手动清理）')
      process.exit(1)
    }
    console.log('【变异】已删除副本（生产文件从未被改动）')
  }
}

// ---- 结果 ----
if (fails.length === 0) {
  console.log(`✓ 39 档预览会动（${checks} 项：逐档求值 + 逐字不退化成整行 + 字幕那条分支 + 接线）`)
  process.exit(0)
}
console.log(`✗ 39 档预览有 ${fails.length} 处问题（共 ${checks} 项判据）：`)
for (const f of fails) console.log('    · ' + f)
process.exit(1)
