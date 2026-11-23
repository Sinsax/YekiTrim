#!/usr/bin/env node
// **9 档动效（三轴）：预览侧「真的会动吗」逐档判据**（2026-10-13，2026-10-14 三轴收敛后重写）。
//
// # 为什么必须有一条这样的判据
//
// 9 档在**三处**各有实现，而它们各自"看起来都对"：
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
//   * **静态的那些档**（`fade` 等）：形状**一致**（不许假装动了）。
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
import { fileURLToPath, pathToFileURL } from 'node:url'

const HERE = path.dirname(fileURLToPath(import.meta.url))
const ROOT = path.join(HERE, '..')
const ENGINE = path.join(ROOT, 'webui', 'src', 'polish', 'engine.js')
const PREVIEW = path.join(ROOT, 'webui', 'src', 'components', 'PolishPreview.vue')

// ⚠️ **`--mutate-sub` 也算变异**（第一版只认 `--mutate` ⇒ 第二个变异根本没生效，
//    而脚本照旧打印"✓ 全绿" —— 看起来像"判据抓不住字幕那条腿"，
//    其实是**变异压根没跑**。这种"自证自己失效"的形态最难发现）。
const MUTATE = process.argv.includes('--mutate') || process.argv.includes('--mutate-sub')
// ⚠️⚠️ **三轴的两个变异台**（2026-10-14 补）。它们守的是"两条轴**同时**生效"这件事 ——
//    而**单轴**判据对它们**全绿**（实测：忽略过渡实参之后 ⑤-quater 之外 78 项一个不红）。
//    这正是本仓最贵的假绿形态：**判据自己没覆盖到，它就替你签了字**。
//      * `--mutate-trans`：`animMotion` 的**第三实参（过渡）被忽略** ⇒ 用户在「过渡」栏
//        选「弹出」，预览里不弹（成片里弹）；
//      * `--mutate-eat`：两轴合并退回 `...(pEff ?? pTrans ?? {})` —— **一张轴吃掉另一张**
//        ⇒ 界面上两栏都选着、两栏单独看也都对，只有**组合**才露馅。
const MUTATE_TRANS = process.argv.includes('--mutate-trans')
const MUTATE_EAT = process.argv.includes('--mutate-eat')

// ---- 变异：把生产的逐字那条路摘掉（判据必须翻红）----
//
// ⚠️ 改的是**生产文件**，跑完必须还原（`try/finally`）。
// ⚠️ 变异体要"看起来合理"：把 「perCharSpans」 直接 「return null」，
//    正是"忘了接线"那种真实事故的形态（不是把函数删掉 —— 那会语法错，
//    判据红得没有意义）。
// ⚠️⚠️ **变异写的是"副本"，绝不改生产文件**（2026-10-13 踩过一次）：
//    第一版直接改 `engine.js` + 在 `finally` 里用 `git checkout` 还原 ——
//    而跑的时候那份文件**还没提交** ⇒ checkout 把它**整份退回 HEAD**，
//    本次改动的逐字实现**全没了**。改成"改副本、import 副本"之后，
//    生产文件**从头到尾一个字节都不会被碰到**（连还原都不需要）。
let mutated = false
let importUrl = new URL('../webui/src/polish/engine.js', import.meta.url).href
if (MUTATE || MUTATE_TRANS || MUTATE_EAT) {
  const orig = fs.readFileSync(ENGINE, 'utf8')
  // ⚠️ **"锚点必须命中恰好一次"**（少了这条，没匹配上的变异体会原样跑绿、
  //    看着像"判据抓不住" —— 本仓栽过）。
  const once = (src, needle, repl, label) => {
    const parts = src.split(needle)
    if (parts.length !== 2) {
      console.log(`✗ 变异失败：锚点「${label}」在 engine.js 里命中 ${parts.length - 1} 次（应当 1 次）`)
      process.exit(1)
    }
    return parts[0] + repl + parts[1]
  }
  if (MUTATE_EAT || MUTATE_TRANS) {
    // ⚠️ **两个三轴变异台**（理由见文件头）：
    //    ① `--mutate-eat`：两轴合并写回**互相覆盖**的旧形态（后摊的那档整个盖掉前档）；
    //    ② `--mutate-trans`：第三实参（过渡轴）**被忽略** ⇒ 过渡档整条看不见、静默回落全局。
    const patched = MUTATE_EAT
      ? once(orig, '...(pTrans ?? {}), ...(pEff ?? {})', '...(pEff ?? pTrans ?? {})',
          'animMotion 两轴合并')
      : once(orig, 'const pTrans = animPreset(transitionName)', 'const pTrans = undefined',
          'animMotion 过渡实参')
    const copyPath = path.join(ROOT, 'webui', 'src', 'polish', '__mutant_engine.mjs')
    fs.writeFileSync(copyPath, patched)
    mutated = true
    importUrl = new URL('../webui/src/polish/__mutant_engine.mjs', import.meta.url).href
    console.log(MUTATE_EAT
      ? '【变异·吃掉】`animMotion` 把两条轴合回 `...(pEff ?? pTrans ?? {})`（后摊的那档整个盖掉前档）'
      : '【变异·过渡】已把 `animMotion` 的第三实参（过渡轴）忽略掉（`pTrans` 恒 `undefined`）')
  } else {
    const marker = 'export function perCharSpans('
    const i = orig.indexOf(marker)
    if (i < 0) {
      console.log('✗ 变异失败：engine.js 里找不到 `perCharSpans`（判据的前提没了）')
      process.exit(1)
    }
    // ⚠️ 锚点必须取**函数签名之后**的那个 `{`，不能取 `indexOf('{', i)` ——
    //    函数上面那段 JSDoc 里就有一个 `{`（`@returns {null | Array<...>}`），
    //    取到它会把 「return null」 插进**注释块里**，于是副本整个语法错
    //    （实测：`Unexpected token 'null'`，而且看起来像"变异体本身写错了"）。
    const sigStart = orig.indexOf('(', i)
    const sigEnd = orig.indexOf(')', sigStart)
    const bodyStart = orig.indexOf('{', sigEnd)
    // ⚠️ **两处变异**，覆盖**两条腿**（少了任一条就测不到对应那个分支）：
    //    ① `--mutate`：把 「perCharSpans」 直接返回 null ⇒ 元素与字幕的逐字**一起**断；
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
  // ⚠️ **权威是转译器那份**（它才写进底座工程）⇒ 预览这份必须与它**同集合**。
  //
  // ⚠️⚠️ 转译器的 `ANIM_PRESETS` 是**函数内局部常量**（不是 `export`），
  //    所以这里**从源码里抽键名**，而不是 `import` 它 —— 与 `check-anim-two-axis.mjs`
  //    抽 `ANIM_AXIS` / `animAxisOf` 同一个手法。
  //    ⚠️ **不许在本判据里再手抄一份档名表**：抄的那份会与被测的那份一起漂，
  //       而"两边一起漂"是本仓最贵的假绿。
  const transSrc = fs.readFileSync(
    path.join(HERE, '..', 'tools', 'polish-to-dhampir.mjs'), 'utf8')
  // 定位 `const ANIM_PRESETS = {` 到它闭合的那一行，再取**顶层**键。
  const presetIdx = transSrc.indexOf('const ANIM_PRESETS = {')
  ok(presetIdx >= 0, '① 抽不到转译器的 `ANIM_PRESETS`（源码形状变了？）')
  const presetBody = transSrc.slice(presetIdx, transSrc.indexOf('\n  };', presetIdx))
  // ⚠️ 顶层键 = **恰好 4 个空格**缩进的 `名字: {` 或 `名字: { ... },`
  //    （档内字段是 6 空格缩进，不会误命中；这条区分必须留着）。
  const transNames = [...presetBody.matchAll(/^ {4}([a-z][a-z0-9_]*): \{/gm)]
    .map((m) => m[1]).sort()
  // ⚠️ **档数是契约（三轴收敛后是 9）**，但这里从**转译器**那份权威数出来再比，
  //    不写死数字 —— 写死的话，改了档位表判据自己不知道。
  //    ⚠️ 两侧必须**同集合** —— 预览少一档的症状是“用户选它、预览里不动”。
  ok(transNames.length === allNames.length,
    `① 预览与转译器的档数不同：转译器 ${transNames.length} 档、预览 ${allNames.length} 档\n` +
    `     ⚠️ **档数是契约**（三轴收敛后 9 档），但这里**不写死数字** ——\n` +
    `        写死之后"加一档"必须同时改判据，而那种改动的形状与"放宽判据求绿"一模一样。`)
  ok(JSON.stringify(allNames) === JSON.stringify(transNames),
    `① 预览与转译器的档集合不同。\n` +
    `      预览多的：${JSON.stringify(allNames.filter((n) => !transNames.includes(n)))}\n` +
    `      预览少的：${JSON.stringify(transNames.filter((n) => !allNames.includes(n)))}\n` +
    `      （多了 = 没删干净；少了 = 用户选它、预览里不动）`)

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
  // ⚠️ 静态档是**少数但确实存在**（`fade` 只淡出、`tilt` 只是姿态、
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

  // ⚠⚠ **整块那条腿的谓词只许写一份**（2026-10-14）。
  //    它曾经在 ② 与 ③ 两处各写一遍 —— 我给心跳加上 `heartbeatAmp` 后，
  //    **上面那处好了、下面那处立刻不同步**（报“③ 两条腿都无变化”）。
  //    本仓“两份实现必然漂移”的老病 —— 直接抽成函数。
  const blockLegMoves = (mo) => !!(mo && ((mo.scaleInMs > 0) || (mo.shakePx > 0) ||
    (mo.swayDeg > 0) || (mo.heartbeatAmp > 1)))
  const moving = []
  const staticOnes = []
  const shapesByTier = {}
  for (const name of allNames) {
    const shapes = charShapes(name)
    shapesByTier[name] = shapes
    // ⚠️ "逐字会动" = 逐字那条路**真的产出**、且**帧之间形状确实不同**。
    const perCharMoves = !!shapes && new Set(shapes.filter(Boolean)).size > 1
    const mo = E.animMotion(null, name)
    const movesBlock = blockLegMoves(mo)
    if (movesBlock || perCharMoves) moving.push(name)
    else staticOnes.push(name)
  }
  // ⚠️ **静态的那一批逐个点名**，而且**每一档都有理由**（不是"判据放宽"）：
  //    * `tilt` —— 改的是**静态姿态**（`rotation`），本来就不是动效；
  //    * `vtext` —— 改的是**排版方向**（底座 `SubtitleStyle::vertical`）；
  //    * `fade` —— "淡入淡出"，动的是**不透明度**（不在形状指纹里，也不给任何时间字段）；
  //    * `perchar_big` / `perchar_small` —— 用 `scaleSpread`（**按序号**的字号乘数），
  //      端点**全相等** ⇒ 是一条**静止的字号阶梯**。
  //      ⚠️ 这正是它们**该有的样子**（engine.js 里写着："用 `scale_to` 是**整行**一起
  //      变小、字与字之间没有差别 ⇒ 不是'逐字'"）⇒ 与帧无关是**设计**，不是漏了时间维度。
  //    ⇒ 判据钉的是**这一批恰好是这些**：多一档说明有档退化了、少一档说明登记过时了。
  // ⚠⚠ **2026-10-14 三轴收敛后重列**（9 档里哪些“本来就不动”）。
  //
  //  只能是下面这三类，且**各自都是设计**：
  //    · `vtext` —— 改的是**排版方向**（底座 `SubtitleStyle::vertical`），不是时间函数。
  //    · `fade`  —— **一个字段都不给**（它靠“取到了没”压掉全局弹入）。
  //                 ⚠️ 它有**动效语义**但没有形状变化 ⇒ 这里算“静态”是对的。
  //    · `tilt`  —— 改的是**静态 `rotation`**（不随时间走）。
  //  ⚠️ `heartbeat` / `pop` / `shake` / `arc` 都必须**会动** ——
  //     它们不在这张表里，少一档就是退化。
  //     ⚠ `heartbeat` 2026-10-14 起是**真双峰**（`heartbeatScaleAt`）——
  //       它之前是单峰占位，又因为不走 `scaleInMs` 而**被误判成静态**。
  //  ⚠️ 三轴收敛后**又多了一类“本来就不动”**：
  //    · `perchar_big` / `perchar_small` —— 它们是**静态的字号阶梯**
  //      （`scaleSpread` 是“按字号的乘数”，**不随时间走**）。
  //      ⚠️ 这正是它们**该有的样子** —— 与原来 `size_*` 那一族同理：
  //         “逐字变化”里的“逐字”指的是**字与字之间不同**，而不是“随时间变”。
  //         实测：同一帧里 4 个字的 transform **各不相同**（4/4），而帧与帧之间相同。
  //      ⚠️ 别把它们当“退化”去“修” —— 下面 ④ 会另测
  //         “同一帧内字与字形状不同”，那才是逐字真正的判据。
  // ⚠️⚠️ **2026-10-30 更正**：`perchar_big` / `perchar_small` **不再是静态档**。
  //    它们是 `perchar` 的**老名字别名**（转译器与 engine 的三份参数逐值相同），
  //    而 `perchar` 自 2026-10-18 起是「入场逐字变大、出场逐字变小」的**过渡动画**
  //    （`scaleFrom → scaleTo` + `perCharOutFrames: 18`）—— 不再是 `scaleSpread` 那种
  //    「按序号排开的静态字号阶梯」。这一行原先还是旧模型，所以判据一直在报
  //    「静态的那一批与登记不符」，而真因是**登记过时**（当时的字幕侧还用不了出场窗口，
  //    见下面 ⑤-ter 那个 `ReferenceError`），不是哪一档退化了。
  //    ⇒ 去掉这两个名字 = 要求它们**必须动**（判据变严，不是放宽）。
  const STATIC_BY_DESIGN = ['fade', 'tilt', 'vtext'].sort()
  const gotStatic = [...staticOnes].sort()
  ok(JSON.stringify(gotStatic) === JSON.stringify(STATIC_BY_DESIGN),
    `② "静态"的那一批与登记不符。
      实得 ${JSON.stringify(gotStatic)}
` +
    `      应当 ${JSON.stringify(STATIC_BY_DESIGN)}
` +
    `      （多出来的 = 某一档退化成不动了；少了的 = 登记过时或判据漏了）`)
  ok(moving.length === allNames.length - STATIC_BY_DESIGN.length,
    `② "会动"的档应当 ${allNames.length - STATIC_BY_DESIGN.length} 档，实得 ${moving.length}`)

  // ---- ③ 逐档求值：**会动的档，不同帧形状真的不同** ----
  //
  // 形状指纹 = 每个字自己的 `transform` 串。帧之间**只要有一个字不同**就算动了。
  // ⚠️ 用**帧**不是秒（`charTransformAt` 的第四参是帧，底座同口径）。
  // ⚠️ 取 0 / 1 / 4 / 12 / 60 这几帧：覆盖"还没开始 / 进行中 / 早就完了"。
  for (const name of allNames) {
    const shapes = shapesByTier[name]
    if (moving.includes(name)) {
      const mo = E.animMotion(null, name)
      const blockMoves = blockLegMoves(mo)
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

  // ---- ③-bis ★ **心跳必须是真双峰**（2026-10-14，用户：
  //      「动效需要增加一个心跳，扑通扑通的心跳感」）----
  //
  // ⚠⚠ **为什么要单独钉**：上面 ②/③ 只问"它动不动"——
  //    而**一次普通弹入也是"动"**。用户要的是"扑通**扑通**"，
  //    形状上就是**两个峰** —— 只钉"会动"的话，拿单峰占位照样绿
  //    （而那正是本档之前的样子）。
  //
  // ⚠ 判据跑的是**导出的真函数**（不在判据里重写一份曲线）。
  {
    const PEAKS = [0, 180, 300, 480]
    const AMP = 1.14
    const CYC = 1000
    const at = (ms) => E.heartbeatScaleAt(ms, PEAKS, AMP, CYC)
    // ① 周期内**局部极大值恰好两个** —— 这就是"扑通扑通"的定义。
    const maxima = []
    for (let ms = 1; ms < CYC - 1; ms += 1) {
      if (at(ms) > at(ms - 1) + 1e-9 && at(ms) >= at(ms + 1)) maxima.push(ms)
    }
    ok(maxima.length === 2,
      `③-bis 心跳在一个周期内应当有**恰好两个**峰（扑通·扑通），实得 ${maxima.length} 个` +
      `（位置 ${JSON.stringify(maxima)}）—— 单峰 = 退化成普通弹入；>2 = 像抖动`)
    // ② 第二峰**必须比第一峰弱**（两个等高看着像抖动）。
    if (maxima.length === 2) {
      const p1 = at(maxima[0])
      const p2 = at(maxima[1])
      ok(p2 < p1 - 1e-6,
        `③-bis 心跳的**第二峰应当比第一峰弱**，实得 ${p1.toFixed(4)} / ${p2.toFixed(4)}` +
        `（等高看着像"抖动"不像心跳）`)
    }
    // ③ 两峰之外**必须回到线基 1.0**（那 520ms 就是"扑通……扑通"的留白）。
    ok(at(700) === 1 && at(900) === 1,
      `③-bis 两峰之外应回到线基 1.0（停顿），实得 ${at(700)} / ${at(900)}`)
    // ④ 纯函数：同一 `ms` **永远同一个值**（无累积状态 ⇒ seek 安全）。
    const once = at(270)
    ok(at(270) === once && at(270) === once,
      '③-bis 心跳曲线必须是**纯函数**（同一 ms 永远同一值）—— 带状态的曲线一 seek 就与成片分叉')
    // ⑤ 周期性：`cycleMs` 之后**重现**（"循环"是需求里的一半）。
    ok(Math.abs(at(30) - at(CYC + 30)) < 1e-9,
      `③-bis 心跳应当**按 ${CYC}ms 循环**，实得 ${at(30)} vs ${at(CYC + 30)}`)
    // ⑥ 垃圾输入回基线（不报错、不跳）。
    ok(E.heartbeatScaleAt(-5, PEAKS, AMP, CYC) === 1 &&
       E.heartbeatScaleAt(0, null, AMP, CYC) === 1 &&
       E.heartbeatScaleAt(0, [0, 180], 1, CYC) === 1 &&
       E.heartbeatScaleAt(0, PEAKS, AMP, 0) === 1,
      '③-bis 心跳对垃圾输入（负数 / peaks 缺失 / amp<=1 / cycle<=0）应返**基线 1**')

    // ⑦ ⚠ **不能拿单峰曲线冒充**：`backOut3` 数学上只有**一个**极值点。
    //    这条是"为什么必须新写一个函数"的**可执行证据**，
    //    而不是注释里一句话。
    const backOut3 = (p) => { const u = p - 1; return u * u * (4 * u + 3) + 1 }
    let boMaxima = 0
    for (let i = 1; i < 999; i += 1) {
      const a = backOut3((i - 1) / 1000)
      const b = backOut3(i / 1000)
      const c = backOut3((i + 1) / 1000)
      if (b > a + 1e-12 && b >= c) boMaxima += 1
    }
    ok(boMaxima === 1,
      `③-bis 前提动了：\`backOut3\` 应当只有**一个**极值点（实得 ${boMaxima}）` +
      `—— 如果它真的变成双峰了，心跳就不需要自己的函数了`)

    // ⑧ ⚠⚠ 最关键的一条：**真跑 `getElements`**，看它确实走了心跳那一支。
    //    上面那些都只证明"函数对"，这条证明"调用点接上了"。
    const idx = E.buildIndex({
      config: {
        canvas: { width: 1920, height: 1080, fps: 30 },
        elements: {},
        segments: [{
          start: 0, end: 30, fx: 'low',
          events: [{
            type: 'element', kind: 'text', text: '扑通', time: 1, end: 20,
            style: 'comment', pos: 'bottom', x: '40%', y: '70%',
            source: 'comment|44|2|255,226,122,255|26,26,26,255|700|1',
            effect_anim: 'heartbeat',
          }],
        }],
      },
      subtitles: [], danmaku: [], duration: 30, fps: 30,
    })
    const scales = []
    for (let t = 2.0; t < 3.0; t += 0.03) {
      const e = E.getElements(idx, t)[0]
      scales.push(e ? e.scale : null)
    }
    const vals = scales.filter((v) => v != null)
    let realMax = 0
    for (let i = 1; i < vals.length - 1; i += 1) {
      if (vals[i] > vals[i - 1] + 1e-6 && vals[i] >= vals[i + 1]) realMax += 1
    }
    ok(vals.length > 20 && realMax >= 2,
      `③-bis **真跑 \`getElements\`** 时心跳应当真的跳两下（实得 ${realMax} 个峰，` +
      `采到 ${vals.length} 个样本）—— 函数对了但没接上也是白算：${JSON.stringify(vals.map((v) => Number(v.toFixed(3))))}`)
    // ⑨ ⚠ 反面：**没选心跳的档不许跳**（平平的 1.0）。
    const idx2 = E.buildIndex({
      config: {
        canvas: { width: 1920, height: 1080, fps: 30 },
        elements: {},
        segments: [{
          start: 0, end: 30, fx: 'low',
          events: [{
            type: 'element', kind: 'text', text: '扑通', time: 1, end: 20,
            style: 'comment', pos: 'bottom', x: '40%', y: '70%',
            source: 'comment|44|2|255,226,122,255|26,26,26,255|700|1',
          }],
        }],
      },
      subtitles: [], danmaku: [], duration: 30, fps: 30,
    }) 
    const flat = []
    for (let t = 2.0; t < 3.0; t += 0.03) {
      const e = E.getElements(idx2, t)[0]
      if (e) flat.push(e.scale)
    }
    ok(flat.length > 20 && flat.every((v) => Math.abs(v - 1) < 1e-9),
      `③-bis 没选动效档的元素**不许跳**（应恒为 1.0）：${JSON.stringify(flat.slice(0, 8))}`)
  }

  // ---- ④ 逐字**确实逐字**：同一帧里各个字**形状不同** ----
  //
  // ⚠️ 这是"整块套在父 div 上"那个退化的唯一机器判据：
  //    若把逐字变换算成"整行一个值"，那么同一帧里所有字的 transform **完全相同**，
  //    而上面 ③ 可能仍然是绿的（帧之间不同不代表字之间不同）。
  //    实测这就是最容易犯的错（"19 档全部看不出差别"正是这个形态）。
  {
    // ⚠️ **2026-10-14 三轴收敛后：带逐字 spec 的只剩三档** ——
    //    `arc`（排版轴，`rotation_to`）+ `perchar_big` / `perchar_small`（动效轴，`scale_spread`）。
    //    原来那 19 档（size/scatter/shrink/grow/hop…）**已整族删除**。
    //    ⚠️ 不写死：从预设表里取“有 `perChar` 的那几档”。
    const samples = allNames.filter((n) => !!E.animPerChar(n))
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
    // ⚠️ **不写死档数**：从预设表里数"带 perChar 的档"有几档，就该真跑几档。
    //    三轴收敛后是 3 档（`arc` 的 rotation + `perchar_big`/`perchar_small` 的 scale_spread）。
    ok(checked === samples.length && checked > 0,
      `④ 逐字档只真跑了 ${checked} 档（预设表里有 ${samples.length} 档带 \。` +
      `少跑 = 有档查不到；一档都没有 = 逐字那条路整条断了）`)
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
      { start: 1, end: 4, text: '弧形', style: { name: 'comment' }, pos: 'bottom', style_anim: 'arc' }
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
      ok(arc.styleAnimName === 'arc', `⑤ 有 \`style_anim\` 的字幕没带出 \`styleAnimName\`：实得 ${JSON.stringify(arc.styleAnimName)}`)
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

  // ---- ⑤-ter ⚠️ 字幕 cue 的 `perCharSpanFrames`（2026-10-30 的 `ReferenceError` 事故）----
  //
  // 症状：`subAnimFields` 里照抄元素那条路写成 `evDuration(c, 0)` —— 而 `c` 在那个作用域
  // **根本不存在**（元素那条路才有一份带 time/end 的对象）。于是**任何带任一档动效的字幕**
  // 在 `subsAt` 里当场抛 `ReferenceError: c is not defined` ⇒ 整层字幕消失、
  // 精修页预览整个挂掉（静态判据全绿：esbuild/Vue 都不为未定义变量报错）。
  //
  // 判据：有 `end` / 没 `end`（老工程或 LLM 漏写）都**不许抛**，且帧数 = **这条 cue 自己的
  // 时长 × fps**（不是 0、也不是拿元素的 time 算）。缺 `end` 时按 0.1s 兜底（= evDuration 的下限）。
  {
    const mk = (sub) => {
      const idx = E.buildIndex({ config: {}, subtitles: [sub], danmaku: [], duration: 30, fps: 30 })
      return E.subsAt(idx, 2)[0]
    }
    const withEnd = mk({ start: 1, end: 5, text: '逐字', style: { name: 'comment' }, pos: 'bottom', transition_anim: 'perchar' })
    ok(!!withEnd, '⑤-ter 带动效的字幕在 `subsAt` 里抛了（`perCharSpanFrames` 用了不存在的标识符？）')
    if (withEnd) {
      ok(Math.abs(withEnd.perCharSpanFrames - 4 * 30) < 1e-6,
        '⑤-ter 字幕的 perCharSpanFrames 应当 = (end-start)×fps = 120，实得 ' + withEnd.perCharSpanFrames)
      ok(Number(withEnd.perCharOutFrames) > 0, '⑤-ter perchar 的出场窗口没带出来（perCharOutFrames=0）')
    }
    const noEnd = mk({ start: 1, text: '缺end', style: { name: 'comment' }, pos: 'bottom', effect_anim: 'perchar_big' })
    ok(!!noEnd, '⑤-ter 缺 end 的字幕在 `subsAt` 里抛了')
    if (noEnd) {
      ok(Math.abs(noEnd.perCharSpanFrames - 0.1 * 30) < 1e-6,
        '⑤-ter 缺 end 时应当按 0.1s 兜底（= 3 帧），实得 ' + noEnd.perCharSpanFrames)
    }
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
    // ⚠️ **字幕逐字那条路必须真的接上**（`spans` + 「perCharSpans」）。
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
    const both = mkSub({ style_anim: 'vtext', effect_anim: 'shake' })
    ok(!!both, '⑤-bis 字幕行没取到')
    if (both) {
      ok(both.styleAnimName === 'vtext',
        '⑤-bis 字幕**没拿到** `style_anim` —— 形态那半边丢了：' +
        `实得 ${JSON.stringify(both.styleAnimName)}\n` +
        '     ⚠️ 成片里字幕会竖排、预览里横着（两边不一致）')
      ok(both.effectAnimName === 'shake',
        '⑤-bis 字幕**没拿到** `effect_anim` —— 动效那半边丢了：' +
        `实得 ${JSON.stringify(both.effectAnimName)}\n` +
        '     ⚠️ 用户选了抖动、成片里抖、预览里不抖 —— 就是用户报的「预览没有实现效果」')
      // ⚠️ 核**实际参数**而不只是档名（档名带出来了、参数没接上一样没效果）。
      ok(both.textMotion?.shakePx > 0,
        '⑤-bis 字幕的 `textMotion.shakePx` 是 0 ⇒ 预览里不会抖：' +
        `${JSON.stringify(both.textMotion?.shakePx)}`)
    }

    // ② 字幕的**逐字**走样式轴（`hop_*` / `pop_perchar` / `scatter_*`）
    const perchar = mkSub({ effect_anim: 'perchar_big' })
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

  // ---- ⑤-quater ⚠️★ **两条轴必须能"叠加"**（不是互相吃掉）----
  //
  // ⚠️⚠️ 这条是**变异自证逼出来的**：把 `animMotion` 的第三实参（过渡）忽略掉之后，
  //    上面所有判据**照样全绿**（75 项）—— 因为**没有一条**在问"过渡 + 动效**同时**给的时候
  //    两样都在不在"。这正是本仓最贵的那类假绿：**判据自己没覆盖到，它就替你签了字。**
  //
  // ⚠️ 症状（实测）：`{...base, ...off, ...pTrans, ...pEff}` 这种写法下，
  //    后摊的那一档把前一档字段整个盖掉 ⇒ 用户选「抖动 + 弹出」时**一半参数归零**，
  //    而**界面上两栏都亮着**、报告里也不报错。
  //
  // ⚠️⚠️ **两个变异台各自守哪一半（实测口径，别写反）**：
  //    * `--mutate-eat`（`...(pEff ?? pTrans ?? {})`）：后摊的 `pEff` 盖掉 `pTrans`
  //      ⇒ **`scaleInMs` 归零**（`shakePx` 恰好**不掉** —— 它本来就来自后摊那一档）。
  //    * `--mutate-trans`（`pTrans` 恒 `undefined`）：过渡档整条消失 ⇒ 同样 **`scaleInMs` 归零**。
  //    ⇒ 两条轴**各自的字段必须都出现在同一份 `textMotion` 里**，判据才守得住。
  //
  // ⚠️ 判据盯**实际参数**（不是档名）：两条轴各自的字段必须都出现在同一份 `textMotion` 里。
  {
    const combo = E.animMotion(null, 'shake', 'pop')
    ok(combo && combo.shakePx > 0,
      '⑤-quater 动效轴被过渡轴**吃掉了**：`shake` + `pop` 时 `shakePx` = ' +
      `${JSON.stringify(combo?.shakePx)}（应当 > 0）\n` +
      '     ⚠️ 用户选「抖动 + 弹出」只拿到一半，而两栏在界面上都亮着 —— 不报错。')
    ok(combo && combo.scaleInMs > 0 && combo.scaleInFrom < 1,
      '⑤-quater 过渡轴被动效轴**吃掉了**：`shake` + `pop` 时弹入参数 = ' +
      `${JSON.stringify([combo?.scaleInFrom, combo?.scaleInMs])}（应当与 pop 单独时一样有弹入）\n` +
      '     ⚠️ 这一条同时是 `--mutate-trans`（过渡实参被忽略）与 `--mutate-eat`\n' +
      '        （后摊的那档盖掉前档）两个变异台的翻面点。')
    // ⚠️ **反向对照**：过渡轴单独给时必须与"两条轴一起给"里的过渡那部分**逐值相同** ——
    //    防"为了叠加而让某一轴泄漏"（叠加不许改变单轴的语义）。
    const popOnly = E.animMotion(null, undefined, 'pop')
    ok(popOnly && popOnly.scaleInMs === combo?.scaleInMs &&
       popOnly.scaleInFrom === combo?.scaleInFrom,
      '⑤-quater 叠加动效轴之后**过渡轴的参数变了**（两轴互相污染）：\n' +
      `     pop 单独：${JSON.stringify([popOnly?.scaleInFrom, popOnly?.scaleInMs])}\n` +
      `     pop+shake：${JSON.stringify([combo?.scaleInFrom, combo?.scaleInMs])}`)
    // ⚠️ 反向：**单轴**时必须与"另一轴为空"逐值相同 —— 防"为了叠加而让某一轴泄漏"。
    ok(JSON.stringify(E.animMotion(null, 'shake', undefined)) ===
       JSON.stringify(E.animMotion(null, 'shake', '')),
      '⑤-quater 空过渡轴时动效结果变了（过渡轴对空值处理不一致）')
    // ⚠️ `fade` 一个字段都不给，但**必须能压掉全局**（这条是 `fade` 的语义本体）。
    const faded = E.animMotion(
      { elements: { scale_in_from: 0.2, scale_in_ms: 999, shake_px: 77 } }, 'fade', 'fade')
    ok(faded && faded.scaleInMs === 0 && faded.shakePx === 0,
      '⑤-quater `fade` 没有压掉全局 `[elements]`：实得 ' +
      `${JSON.stringify([faded?.scaleInMs, faded?.shakePx])}（应当都是 0）\n` +
      '     ⚠️ 用户要的是"只淡入淡出"，压不掉的话会变成"弹入 + 抖动 + 淡入淡出"。')
  }

  // ---- ⑤-quinquies ⚠️★ **三条轴同时生效、互不覆盖**（2026-10-14 三轴）----
  //
  // ⚠️⚠️ 上面那条 ⑤-quater 只走了 `animMotion` **一个函数**（两轴）。三轴化之后
  //    用户能做的是**三条同时**：「竖排（排版）+ 弹出（过渡）+ 抖动（动效）」。
  //    这一条才是那个能力在**事件 → 索引 → 求值 → 画面**整条链上的判据。
  //
  // ⚠️ 为什么非要有它（四条都不会自己报错）：
  //   · **过渡轴被忽略**（`--mutate-trans`）⇒ 用户在「过渡」栏选「弹出」，
  //     预览里不弹而**成片里弹** —— ⑤-quater 能抓到，但它只证明"函数里对了"，
  //     证明不了 `buildIndex` 那个调用点真的把第三路**接上了**；
  //   · 三条轴里**任意一条读错档名**（例如排版那路读 `effectAnimName`）⇒
  //     组合里**只有一部分**生效；
  //   · **两轴互相吃掉**（`--mutate-eat`）⇒ 后摊的那档把前档字段整个盖掉，
  //     **界面上三栏都选着、三栏单独看也都对**，只有"三条同时"才露馅。
  {
    const mkAll = (ev) => E.buildIndex({
      config: {
        canvas: { width: 1920, height: 1080, fps: 30 }, elements: {},
        segments: [{ start: 0, end: 10, fx: 'low', events: [{
          type: 'element', kind: 'text', text: '竖排且抖', time: 1, end: 4,
          style: 'comment', pos: 'bottom', x: '40%', y: '70%',
          source: 'comment|44|2|255,226,122,255|26,26,26,255|700|1', ...ev
        }] }]
      },
      subtitles: [], danmaku: [], duration: 10, fps: 30
    })
    const firstEl = (idx) => (idx.element || [])[0]

    const all3 = firstEl(mkAll({ style_anim: 'vtext', transition_anim: 'pop', effect_anim: 'shake' }))
    ok(!!all3, '⑤-quinquies 三条轴组合的文字元素没进索引')
    if (all3) {
      ok(all3.styleAnimName === 'vtext' && all3.transitionAnimName === 'pop'
        && all3.effectAnimName === 'shake',
        '⑤-quinquies 组合档没把三条轴**分别**带到预览：' +
        JSON.stringify([all3.styleAnimName, all3.transitionAnimName, all3.effectAnimName]))
      // ⚠️ **核实际参数**而不是档名：档名带出来了、参数没接上同样没效果。
      ok(E.animPreset(all3.styleAnimName)?.vertical === true,
        '⑤-quinquies **排版**轴参数没接上（`vtext` 应当是竖排）：' +
        JSON.stringify(E.animPreset(all3.styleAnimName)?.vertical))
      ok(all3.textMotion?.scaleInMs > 0,
        '⑤-quinquies **过渡**轴参数没接上（`pop` 应当有 `scaleInMs`）：' +
        JSON.stringify(all3.textMotion?.scaleInMs))
      ok(all3.textMotion?.shakePx > 0,
        '⑤-quinquies **动效**轴参数没接上（`shake` 应当有 `shakePx`）：' +
        JSON.stringify(all3.textMotion?.shakePx))
    }

    // ⚠️ **反序**：三条轴**不许互相清掉**（写序不同结果必须一致）。
    const rev3 = firstEl(mkAll({ effect_anim: 'shake', transition_anim: 'pop', style_anim: 'vtext' }))
    ok(rev3?.styleAnimName === 'vtext' && rev3?.transitionAnimName === 'pop'
      && rev3?.effectAnimName === 'shake',
      '⑤-quinquies 反序写三条轴结果不同（几条轴互相清掉了？）：' +
      JSON.stringify([rev3?.styleAnimName, rev3?.transitionAnimName, rev3?.effectAnimName]))

    // ⚠️★ **`shake` + `pop` 必须同时生效**（"两轴互相吃掉"的**正面判据**）。
    //    ⚠️ 只写这两条轴（不带排版），把"过渡 vs 动效"单独拎出来看 ——
    //    这正是用户能做出的那个最小组合。
    const combo2 = firstEl(mkAll({ transition_anim: 'pop', effect_anim: 'shake' }))
    ok(combo2?.textMotion?.shakePx > 0,
      '⑤-quinquies **`shake` + `pop` 同时选时 `shakePx` 归零了**（动效被过渡吃掉）：' +
      `${JSON.stringify(combo2?.textMotion?.shakePx)}\n` +
      '     ⚠️ 界面上两栏都选着、两栏单独看也都对，只有这一条**组合**判据能发现。')
    ok(combo2?.textMotion?.scaleInMs > 0,
      '⑤-quinquies **`shake` + `pop` 同时选时 `scaleInMs` 归零了**（过渡被动效吃掉）：' +
      `${JSON.stringify(combo2?.textMotion?.scaleInMs)}\n` +
      '     ⚠️ 这正是 `--mutate-eat` / `--mutate-trans` 两个变异台的翻面点。')
    // ⚠️ 还要真的**走到求值**上：参数在、`getElements` 却没算进画面，一样白搭。
    //    ⚠️ 取 1.05s（弹入窗口 350ms **之内**）—— 窗口外 `motionScale` 恒 1，
    //    那时"有没有弹"根本看不出来（假绿）。
    const comboEl = E.getElements(mkAll({ transition_anim: 'pop', effect_anim: 'shake' }), 1.05)[0]
    ok(!!comboEl, '⑤-quinquies 组合档的 `getElements` 没求出元素')
    if (comboEl) {
      ok(comboEl.shakeX !== 0 || comboEl.shakeY !== 0,
        '⑤-quinquies 组合档**算出来的画面里没有抖动**（`shakeX`/`shakeY` 恒 0）：' +
        `shakeX=${JSON.stringify(comboEl.shakeX)} shakeY=${JSON.stringify(comboEl.shakeY)}`)
      ok(comboEl.scale !== 1,
        '⑤-quinquies 组合档**算出来的画面里没有弹入**（弹入窗口内 `scale` 应当 ≠ 1）：' +
        `scale=${JSON.stringify(comboEl.scale)}`)
    }

    // ⚠️ **只写排版轴** ⇒ 另外两条必须为空（不许串到别的档上去）。
    const onlyStyle = firstEl(mkAll({ style_anim: 'vtext' }))
    ok(onlyStyle?.effectAnimName === '' && onlyStyle?.transitionAnimName === '',
      '⑤-quinquies 只写 `style_anim` 时另外两条轴应当为空：' +
      JSON.stringify([onlyStyle?.transitionAnimName, onlyStyle?.effectAnimName]))
    ok(!(onlyStyle?.textMotion?.shakePx > 0) && !(onlyStyle?.textMotion?.scaleInMs > 0),
      '⑤-quinquies 只写 `style_anim` 却弹/抖起来了（别的轴串到排版档了）：' +
      JSON.stringify([onlyStyle?.textMotion?.shakePx, onlyStyle?.textMotion?.scaleInMs]))

    // ⚠️ **只写过渡轴** ⇒ `pop` 自己必须真的弹出（不静默回落全局）。
    //    这是"忽略第三实参"那个变异在**调用点**上的正面判据。
    const onlyTrans = firstEl(mkAll({ transition_anim: 'pop' }))
    ok(onlyTrans?.textMotion?.scaleInMs > 0,
      '⑤-quinquies 只写 `transition_anim="pop"` 时 `scaleInMs` 是 0 ——' +
      '**过渡轴在预览里不生效**：' + JSON.stringify(onlyTrans?.textMotion?.scaleInMs) +
      '（`pop` 应当给出 350，成片里也会弹）')
  }

  // ---- ⑤-ter ⚠️ **形态那半边的"实际参数"必须真的被消费** ----
  //
  // ⚠️⚠️ 这一条也是**补出来的**，理由与 ⑤-bis 同源、但位置更深一层：
  //    ⑤-bis 只保证 `subAnimFields` 把 `styleAnimName` **带出来**了；
  //    而组件第一版**只拿它做 `textTiltTransform`（倾斜/压扁）** ——
  //    `tilt` 只是 9 档里的**一档**。`vtext` 给的是
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

  // ---- ⑤-sexies ⚠️★ **元素那条路也要排版轴与逐字**（2026-10-14）----
  //
  // ⚠️⚠️ 与 ⑤-ter 是**同一个病、不同的路**：⑤-ter 盯的是**字幕**那条
  //    （`subAnimFields` + `PolishPreview.vue`），而**元素**那条
  //    （「buildIndex」 的元素分支 + `getElements`）是**另一份实现**。
  //    本仓第 ④ 类老病就是"**两条路径，只修了一条**" —— 实测这里又犯了一次：
  //      · `vtext` 的 `vertical` / `fontWeight`：**只有** `capRowsWrapped`（字幕专用）读，
  //        元素那条 `texts` computed 里没有任何 `formStyle`
  //        ⇒ 同一个「竖排」档：**字幕竖着、文字元素横着**，两边都不报错；
  //      · `arc` 与 `perchar_*`：`getElements` 的 push 里
  //        `perCharFrame: local * idx.fps` 早就在了，但 **`perCharFps` 没人给**
  //        ⇒ 「perCharSpans」 第一道门 「perCharFps > 0」 不过 ⇒ **直接 「return null」**
  //        ⇒ 元素上逐字**一个 span 都不产出**（用户选「弧形 / 逐字变化」字一动不动，成片里动）；
  //      · 「entry.perChar」 也**只试排版轴** ⇒ 用户选「逐字变大」（动效轴）
  //        在元素上同样一条不动。
  //    ⇒ 元素上原本**只有 `tilt` 真的生效**（`tiltDeg` 恰好是 `textTiltTransform` 的消费者）。
  //
  // ⚠️ 判据要**走到求值**（`getElements` + 「perCharSpans」），不能只查源码文本 ——
  //    "字段给了但没人读"和"读了但值是 null"是两种不同的坏法，只有求值能同时盖住。
  {
    const mkEl = (ev) => E.buildIndex({
      config: {
        canvas: { width: 1920, height: 1080, fps: 30 }, elements: {},
        segments: [{ start: 0, end: 10, fx: 'low', events: [{
          type: 'element', kind: 'text', text: '弧形 ABC', time: 1, end: 4,
          style: 'comment', pos: 'bottom', x: '40%', y: '70%',
          source: 'comment|44|2|255,226,122,255|26,26,26,255|700|1', ...ev
        }] }]
      },
      subtitles: [], danmaku: [], duration: 10, fps: 30
    })
    const elAt = (ev, t) => E.getElements(mkEl(ev), t)[0]
    const spansAt = (ev, t) => {
      const e = elAt(ev, t)
      if (!e) return { e: null, sp: null }
      return {
        e,
        sp: E.perCharSpans(e.text, {
          perChar: e.perChar, perCharFrame: e.perCharFrame,
          perCharFps: e.perCharFps, fontPx: e.style?.fontPx
        })
      }
    }

    // ① **排版轴**：`vtext` 的竖排必须**读得到**（元素自己那一档）。
    const vt = elAt({ style_anim: 'vtext' }, 2.0)
    ok(!!vt, '⑤-sexies 「竖排」的文字元素没进索引')
    if (vt) {
      ok(E.animPreset(vt.styleAnimName)?.vertical === true,
        '⑤-sexies 元素这条路的 `styleAnimName` 没带出竖排档：' +
        JSON.stringify(vt.styleAnimName))
    }

    // ② ★ **逐字必须真的产出 span**（元素这条路）。
    //    ⚠️ `arc` 是排版轴、`perchar_big` 是动效轴 —— **两条都要试**：
    //       只试一条的话，另一个"只读排版轴"的写法照样绿。
    for (const [label, ev] of [
      ['`arc`（排版轴的逐字）', { style_anim: 'arc' }],
      ['`perchar_big`（动效轴的逐字）', { effect_anim: 'perchar_big' }],
      ['`perchar_small`（动效轴的逐字）', { effect_anim: 'perchar_small' }]
    ]) {
      const { e, sp } = spansAt(ev, 1.5)
      ok(!!e, `⑤-sexies ${label} 的文字元素没进索引`)
      if (!e) continue
      ok(!!e.perChar,
        `⑤-sexies ${label} 在**元素**这条路上 「perChar」 是空的 —— ` +
        `用户选了它、预览里字一动不动（成片里动）
` +
        `     ⚠️ 多半是 「entry.perChar」 **只试了排版轴**，而动效轴的档没被认到。`)
      ok(!!sp && sp.length > 1,
        `⑤-sexies ${label} 的 「perCharSpans」 是 null／整串 —— **元素上逐字没生效**
` +
        "     ⚠️ 第一道门是 perCharFps > 0，拿不到就 return null：" +
        `perCharFps=${JSON.stringify(e.perCharFps)}`)
      // ⚠️ 同帧内**各字变换必须不同** —— 全一样 = "整行平移"退化成假逐字。
      if (sp && sp.length > 1) {
        // ⚠⚠ 取样时刻必须**在错峰窗口内**：实测 `arc`（`per_char_frames=8`、
        //    `stagger_frames=3`、6 个字）在 **frame 0 全是起点、frame 30 全是终点**
        //    —— **两端都是均匀的**，只有中间（frame 15）才看得到错峰。
        //    ⚠ 拿端点当样本 = **假红**（我第一版就是拿 t=1.0 去量的，它是 frame 0）。
        const uniq = new Set(sp.map((x) => x.style?.transform || '').filter(Boolean))
        ok(uniq.size > 1,
          `⑤-sexies ${label} 在元素上**每个字的变换都一样**（退化成整行平移）：` +
          `${JSON.stringify([...uniq])}`)
      }
    }

    // ③ **不给逐字的档必须还是整串**（别为了绿把所有人都逐字化）。
    const shakeSp = spansAt({ effect_anim: 'shake' }, 2.0)
    ok(!shakeSp.sp,
      '⑤-sexies 「抖动」不该有逐字 span（它是纯动效，没有 `perChar`）：' +
      `${JSON.stringify(shakeSp.sp && shakeSp.sp.length)}`)

    // ④ ★ **逐字必须随时间走**（不是"生成了 span 就算数"）。
    //    ⚠️⚠️ 这一条是实测逼出来的：我第一版在 `buildIndex` 里写死
    //       `perCharFrame = 0`，结果**六个字全是同一个 `rotate(-14deg)`** ——
    //       span 产出了、看着像生效，其实**从头到尾一模一样**
    //       （`charTransformAt` 的 `(localFrame - start)/dur` 里 frame 不走 ⇒ 停在终点值）。
    //       这正是"比不画更坏"的那一类：静默的"配了不动"。
    const arcA = spansAt({ style_anim: 'arc' }, 1.3)
    const arcB = spansAt({ style_anim: 'arc' }, 1.9)
    ok(!!(arcA.sp && arcB.sp),
      '⑤-sexies arc 在元素上没产出 span（④ 没法判）')
    if (arcA.sp && arcB.sp) {
      const a = JSON.stringify(arcA.sp.map((x) => x.style))
      const b = JSON.stringify(arcB.sp.map((x) => x.style))
      ok(a !== b,
        '⑦-sexies arc 在元素上**两个时刻的画面完全一样** —— ' +
        '逐字**配了不动**（perCharFrame 大概率被写死成常量）' + '\n' +
        '     ⚠️ span 产出了所以 ② 会绿，只有这条能发现。')
    }
  }

  // ---- ⑤-bis ⚠★ **元素的 `pos`（上/中/下）必须在预览里真的被消费**（2026-10-14）----
  //
  // ⚠⚠ 用户报的第一个问题是「元素的文字位置不对」，其中 **Ⅰb** 是：
  //    `engine.js` 早就算出了 `entry.textPos`、也写进了输出，
  //    而 **组件从头到尾没用过它** ⇒ 「上/中/下」三档在预览里
  //    **完全同形**：用户在面板上换档位、预览**纹丝不动**。
  //    这不是"位置略偏"，是**这一维整个没接上**。
  //
  // ⚠ 判据跑的是**从 `.vue` 里抽出来的真函数**（不复刻一份
  //    —— 复刻的那份会与被测的那份一起漂）。
  {
    const pvSrc = fs.readFileSync(PREVIEW, 'utf8')
    const m = pvSrc.match(/const ELEMENT_EDGE_MARGIN_PCT = 6[\s\S]*?function textPosTopPct\(e\) \{[\s\S]*?\n\}/)
    ok(!!m, '⑤-bis 从 `PolishPreview.vue` 抽不到 `textPosTopPct`（源码形状变了？）')
    if (m) {
      let fn = null
      try {
        fn = new Function(m[0] + '; return textPosTopPct;')()
      } catch (err) {
        ok(false, `⑤-bis 抽出来跑不起来：${err.message}`)
      }
      if (fn) {
        const top = fn({ pos: 'top' })
        const cen = fn({ pos: 'center' })
        const bot = fn({ pos: 'bottom' })
        // ① 三档必须**互不相同** —— 不同就等于没接。
        ok(new Set([top, cen, bot]).size === 3,
          '⑤-bis 「上/中/下」三档在预览里**算出同一个位置** —— ' +
          `用户换档位画面纹丝不动：${JSON.stringify([top, cen, bot])}`)
        // ② 中档必须**真的在中间**（不是"比上档低一点"）。
        ok(top < cen && cen < bot,
          '⑤-bis 三档的**顺序不对**（应当 top < center < bottom）：' +
          `${JSON.stringify({ top, cen, bot })}`)
        ok(cen === 50, `⑤-bis 中档应当是 50%（垂直居中），实得 ${cen}`)
        // ③ ⚠ **没有 `pos` 的旧路径必须返 `null`**（R4：旧路径一个字节都不动）。
        ok(fn({ pos: '' }) === null && fn({}) === null && fn({ pos: undefined }) === null,
          '⑤-bis 没有 `pos` 的元素**不得**被接管（应该返 `null` → 沿用旧 `yPct`）：' +
          `${JSON.stringify([fn({ pos: '' }), fn({}), fn({ pos: undefined })])}`)
        // ④ 认不出的档名也必须返 `null`（别静默归到某一档）。
        ok(fn({ pos: 'middle' }) === null && fn({ pos: 'TOP' }) === 6,
          `⑤-bis 档名处理不对：认不出的应返 \`null\`、大小写应当归一化：` +
          `${JSON.stringify([fn({ pos: 'middle' }), fn({ pos: 'TOP' })])}`)
      }
    }
    // ⑤ 组件必须**真的把它用上**（算出来不用 = 白算）。
    //    ⚠ 剥注释后再查——上面"这里曾经没接过 pos"那句话
    //      本身就含 `pos` 字样，不剥会把说明当成实现。
    const pvCode = pvSrc.replace(/<!--[\s\S]*?-->/g, '').replace(/\/\*[\s\S]*?\*\//g, '')
      .replace(/^\s*\/\/.*$/gm, '')
    ok(/topPct/.test(pvCode),
      '⑤-bis 组件**没有消费** `topPct` —— `pos` 算出来不用 = 白算（用户仍然看不到换档）')
    ok(/tx\.topPct/.test(pvCode),
      '⑤-bis 模板上**没用** `tx.topPct` —— 只在 script 里算、模板不绑，画面不会变')
  }

  // ---- ⑦ ⚠️ **老格式 `anim` ↔ 新格式三轴等价**（R4 的预览侧，2026-10-14 三轴版）----
  //
  // ⚠️⚠️ **老格式那条路仍然存在**：`splitAnimTwoAxis` 返回**三路**
  //    `{style, effect, transition}`，而 `engine.js` 的**两个调用点**
  //    （「buildIndex」 的元素分支 + `subAnimFields` 的字幕分支）都接了过渡轴。
  //
  // ⚠️ **`pop` / `fade` 在老格式下走的是 `transition` 那一路** ——
  //    只吐两路时它们**三路全空** ⇒ 轨 id 退化成 `text_<style>_<pos>`
  //    （`fade` 与 `pop` 撞同一个 id、后面那条被吃掉）且 `on === false`
  //    ⇒ **静默回落全局**（用户选「弹出」拿到"全局弹入 + 全局抖动 + 全局摇摆"）。
  //    这组对拍正好把那个修复**锁住**。
  //
  // ⚠️ 为什么必须有它（三条都不会自己报错）：
  //   · 预览只读老 `anim` ⇒ 新界面写的字段它**看不见** ⇒ 预览完全没效果、
  //     成片有（这也正是 ③④⑤ 全绿时发现不了的那一类）；
  //   · 轴混用（动效那路读 `styleAnim`）⇒ 组合里**只有一半**生效；
  //   · 排版那路读 `effectAnim` ⇒ 竖排丢失、只剩抖。
  //
  // ⚠️ **用全部档名覆盖**（不只是挑几个）—— 挑着测会漏掉整整一条轴。
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
    const both = first(mk({ style_anim: 'vtext', effect_anim: 'shake' }))
    ok(!!both, '⑦ 组合档的文字元素没进索引')
    if (both) {
      ok(both.styleAnimName === 'vtext',
        `⑦ 组合档没把 \`style_anim\` 带到预览：实得 ${JSON.stringify(both.styleAnimName)}`)
      ok(both.effectAnimName === 'shake',
        `⑦ 组合档没把 \`effect_anim\` 带到预览：实得 ${JSON.stringify(both.effectAnimName)}`)
      // ⚠️ 核的是**实际参数**而不是档名：档名带出来了、参数没接上同样没效果。
      const st = E.animPreset(both.styleAnimName)
      ok(st?.vertical === true,
        `⑦ 组合档的**排版**参数没接上（\`vtext\` 应当是竖排）：实得 ${JSON.stringify(st?.vertical)}`)
      ok(both.textMotion?.shakePx > 0,
        `⑦ 组合档的**动效**参数没接上（\`shake\` 应当有 \`shakePx\`）：` +
        `实得 ${JSON.stringify(both.textMotion?.shakePx)}`)
    }

    // ② 反序也要成立（两轴不许互相清掉）
    const rev = first(mk({ effect_anim: 'shake', style_anim: 'vtext' }))
    ok(rev?.styleAnimName === 'vtext' && rev?.effectAnimName === 'shake',
      '⑦ 反序写两个字段结果不同（两轴互相清掉了？）：' +
      JSON.stringify([rev?.styleAnimName, rev?.effectAnimName]))

    // ③ 只写样式轴 ⇒ 动效那半边**必须为空**（不许串到别的档上去）
    const onlyStyle = first(mk({ style_anim: 'vtext' }))
    ok(onlyStyle?.effectAnimName === '',
      `⑦ 只写 \`style_anim\` 时动效轴应当为空：实得 ${JSON.stringify(onlyStyle?.effectAnimName)}`)
    ok(!(onlyStyle?.textMotion?.shakePx > 0),
      `⑦ 只写 \`style_anim\` 却抖起来了（动效轴串到样式档了）：` +
      `${JSON.stringify(onlyStyle?.textMotion?.shakePx)}`)

    // ④ 老格式 `anim`（单档）在预览里必须**与等价的新格式一致**（R4 的预览侧）
    //
    // ⚠️⚠️ **2026-10-14 三轴收敛后，等价的新格式是"三路"**（原来只拼两路）。
    //    收敛把 `pop` / `fade` 的轴改成了 `transition` —— 只拼 `style`/`effect`
    //    的话这两档会拼成 `{style_anim:'', effect_anim:'fade'}`（**把档名塞进错的轴**），
    //    于是"新格式"那条腿与老格式不等价 ⇒ 判据红，而**红得对**：
    //    它当场指出"过渡档没有落到过渡轴上"。
    //    ⚠️ 用**9 个档全覆盖**（不只是挑几个）—— 挑着测会漏掉整整一条轴。
    for (const n of allNames) {
      const legacy = first(mk({ anim: n }))
      const sp = E.splitAnimTwoAxis(n)
      // 按**转译器同一套归属**拼出等价的新格式（splitAnimTwoAxis 就是那份归属）。
      const viaNew = first(mk({
        style_anim: sp.style,
        effect_anim: sp.effect,
        transition_anim: sp.transition
      }))
      ok(legacy?.styleAnimName === viaNew?.styleAnimName &&
        legacy?.effectAnimName === viaNew?.effectAnimName &&
        legacy?.animName === viaNew?.animName &&
        JSON.stringify(legacy?.textMotion) === JSON.stringify(viaNew?.textMotion),
        `⑦ 老格式 \`anim="${n}"\` 与等价的新格式在预览里**不一致**\n` +
        `     老：${JSON.stringify([legacy?.styleAnimName, legacy?.effectAnimName, legacy?.animName, legacy?.textMotion])}\n` +
        `     新：${JSON.stringify([viaNew?.styleAnimName, viaNew?.effectAnimName, viaNew?.animName, viaNew?.textMotion])}\n` +
        `     ⚠️ 这一条同时守着"过渡档必须落到过渡轴"（\`splitAnimTwoAxis\` 的第三路）`)
      // ⚠️ 反向：**认不出的档名**在老格式下必须**三路全空**（不许猜成某一轴）。
      const bad = first(mk({ anim: n + '_nope' }))
      ok(!bad || (!bad.styleAnimName && !bad.effectAnimName && !bad.animName),
        `⑦ 认不出的老档名 \`anim="${n}_nope"\` 被**猜**进了某条轴：` +
        `${JSON.stringify([bad?.styleAnimName, bad?.effectAnimName, bad?.animName])}`)
    }
    // ⚠️★ **过渡轴必须真的落到过渡轴上**（不是塞进动效轴）—— 单独钉一条，
    //    因为"塞进错的轴"在预览里**也能动**，只是动的理由不对、且成片不认。
    {
      const sp = E.splitAnimTwoAxis('pop')
      ok(sp.transition === 'pop' && sp.effect === '' && sp.style === '',
        `⑦ \`pop\` 的归属错了：应当三路里**只有 transition**，实得 ${JSON.stringify(sp)}\n` +
        `     ⚠️ 修前它是三路全空 ⇒ 轨 id 退化、静默回落全局（成片侧同一个坑）`)
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
// ⚠️⚠️ **变异台的结论必须机器可判**（2026-11-24 修）：上一版在 `--mutate` 下照样走
//    「有问题 ⇒ exit 1」那条路 ⇒ 闸门把它记成**失败**，而它报的 17 处**正是想要的**
//    （摘掉逐字那条路之后判据翻红）。两边口径对不上，这条自证于是长期是红的。
//    现在按 `check-noise-parity.mjs` 的 `expectRed` 那套：**变异后必须翻红 = 通过**；
//    **变异后还全绿 = 判据空转 ⇒ 红**。
//    ⚠️ 基线（未变异）必须绿 —— 那由 gate 里**不含** `--mutate` 的那一次负责；两条合起来才是自证。
const ANY_MUTATE = MUTATE || MUTATE_TRANS || MUTATE_EAT
if (ANY_MUTATE) {
  if (!mutated) {
    console.log('✗ 变异**没打上**（副本没生成）—— 这次自证是空转的，别放过')
    process.exit(1)
  }
  if (fails.length === 0) {
    console.log(`✗ 变异**没有翻面**：改坏实现之后 ${checks} 项判据照样全绿 ⇒ 它们是空转的`)
    process.exit(1)
  }
  console.log(`✓ 变异自证：改坏实现后有 ${fails.length} 处判据翻红（共 ${checks} 项）—— 判据不是空转`)
  for (const f of fails) console.log('    · 翻红：' + f.split('\n')[0])
  process.exit(0)
}
if (fails.length === 0) {
  console.log(`✓ 三轴 9 档预览会动（${checks} 项：逐档求值 + 逐字不退化成整行 + 字幕那条分支`
    + ` + 接线 + 三条轴同时生效）`)
  process.exit(0)
}
console.log(`✗ 三轴 9 档预览有 ${fails.length} 处问题（共 ${checks} 项判据）：`)
for (const f of fails) console.log('    · ' + f)
process.exit(1)
