#!/usr/bin/env node
// 守「成片时间线（EDL）的**三处消费点同源**」：同一个 `clips` 数组派生出的
// **输出时间（out_start / out_end）必须逐值一致**。
//
// # 为什么需要它
//
// 规格 `docs/plan/edl-retiming-spec.md` §3 的第一句话是「**一个数组，三处消费**」：
//
//   ① Rust `Edl::resolve()`             —— 出片腿（音轨组装、总时长、ffmpeg 侧）
//   ② 转译器 `polish-to-dhampir.mjs`     —— 画面腿（每段一个 main layer 的 start/end）
//   ③ 前端成片轨的 out_start 推导        —— 预览腿（第 3 批 task-6 才到）
//
// 三处各推一遍的症状与 `check-grid-parity.mjs` 那条教训**一模一样**：
// 「预览里块的位置和出片里差一点点」**且不报错**。这是规格 §9 的 **R7（预览 ≠ 成片）**，
// 本仓的**头号事故类型**。所以这里全部按**逐值**钉，不写"大约""接近"这类断言。
//
// # 钉的是**派生值**，不是代码形状
//
// 反面教材是"断言某个函数被调用了"——那种断言改坏算法照样全绿。
// 这里的对拍点是：给一份具体的 `clips`（含 speed≠1），
// **各消费点算出来的 out_start/out_end 数列**必须逐值相等。
//
// # ⚠️ R8：累加必须在**帧号域**
//
// `out_start = Σ(前面每段的 (source_end - source_start) / speed)` 是**累加**：
// 在**秒域**累加，单段 0.01s 的舍入会推着后面**所有**段一起偏，段越多越明显
// （`cut_ranges` 时代不累加，这是 **EDL 特有**的新风险）。所以：
//   · 分段时长**先换算成帧再取整**，累加只在整数帧号上做；
//   · 本脚本有一条变异专门打这个：把帧域累加换成秒域累加，必须翻面。
//
// # 每个断言都有**变异自证**
//
// 少了它，断言可能只是匹配了**注释里的字样**（本仓踩过：`check-grid-parity.mjs`
// 的 `toFixed(2)` 谓词被注释顶翻，变异永远不翻面）。
// 本脚本的变异全部走**真改源码文本 → 断言替换命中 1 次 → 再断言检查翻面**三步，
// 少了第一步，"变异体"可能根本没变（`check-yekitrim-translator.mjs` 记过这个坑：
// 缩进从 4 空格变 2 空格导致替换串不匹配）。转译器那侧是**真加载变异体**跑的，
// 不是文本比对。
//
// # ⚠️ 判据走**去注释后的源码**
//
// 本脚本按源码文本抓 `buildKeepRanges` 的累加式时，**先剥注释**（`stripComments`）：
// 上面那段文档注释里就写着 `+ (end - start) / speed` 这个字面量，
// 不去注释的话"把累加改坏"根本不会让谓词翻面 —— 谓词命中的是注释。
// 凡走这条路的地方，代码里都标了 `[去注释]`。
//
// # ⚠️ 整合期须知：task-1 的工作**尚未 commit**
//
// 2026-10 这一轮里 `tools/polish-to-dhampir.mjs` 的 `buildKeepRanges`（task-1）改完
// **还没有落成 commit** —— 所以 `git diff --stat tools/polish-to-dhampir.mjs` 会显示
// 几十行插入，那**不是**本批次改的，是 task-1 的存量未提交改动。
// ⇒ 想看清"这一批到底改了什么"，**别拿 `git diff` 对着 HEAD 读**，
// 那个基线还不是一个 clean tree。
//
// # 运行
//
//     node scripts/check-edl-invariants.mjs              # 逐值对拍
//     node scripts/check-edl-invariants.mjs --mutate     # 变异自证（每个变异必须翻面）
//
// 零依赖、不 import 应用模块、不需要起服务。退出码非 0 即失败。
// ⚠️ 消费点 ① 会**真跑一次 cargo**（临时目录里的一次性探针工程，见 `runRustResolveProbe`）；
//    首次会拉依赖图，之后走本仓 `target/` 的增量。
//
// ⚠️ **有跳过项时结尾打的是 `△`，不是 `✓ 全部通过`** —— 见文件末尾那段注释。
//    有人/CI grep `✓ 全部通过` 时，不该能把"R7 尚未成立"读成"同源已验收"。

import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { execFileSync } from 'node:child_process'
import { dirname, join } from 'node:path'
import { fileURLToPath, pathToFileURL } from 'node:url'
import { tmpdir } from 'node:os'

/// 最近一次 Rust 探针的失败原因（**必须原样打出来**，否则 SKIP 没法归因）。
let probeError = null
/// 消费点 ① 真跑回来的值（`null` = 没跑起来 / 还没实现）。
/// 交给变异块做「这条对拍抓得住吗」的自证。
let rustProbeResult = null

const HERE = dirname(fileURLToPath(import.meta.url))
const ROOT = join(HERE, '..')
const TRANSLATOR = join(ROOT, 'tools', 'polish-to-dhampir.mjs')
const RUST_CONFIG = join(ROOT, 'crates', 'yekitrim-polish', 'src', 'project', 'config.rs')
const RUST_TYPES = join(ROOT, 'crates', 'yekitrim-render-core', 'src', 'types.rs')
const FRONTEND = join(ROOT, 'webui', 'src', 'polish', 'engine.js')

const MUTATE = process.argv.includes('--mutate')

// ---------------------------------------------------------------------------
// 计数与断言
// ---------------------------------------------------------------------------
let checks = 0
let failed = 0
const fails = []
/** 跳过的项（**必须显式打印**：静默放行 = 把"没判据"伪装成"判据通过"）。 */
const skips = []

function check(label, actual, expected) {
  checks += 1
  const ok = JSON.stringify(actual) === JSON.stringify(expected)
  if (!ok) {
    failed += 1
    fails.push(label)
    console.log(
      `  FAIL ${label}\n         实得: ${JSON.stringify(actual)}\n         期望: ${JSON.stringify(expected)}`,
    )
  } else {
    console.log(`  ok   ${label}`)
  }
  return ok
}

/** 跳过一条判据 —— 打印得**足够响**，绝不混进 "ok" 里。 */
function skip(label, why) {
  skips.push({ label, why })
  console.log(`  SKIP ${label}\n         ↳ ${why}`)
}

/** 逐值相等（浮点用整数帧比，见 R8；秒域只在最后格式化时才出现）。 */
const eqFrames = (a, b) => Math.round(a) === Math.round(b)

// ---------------------------------------------------------------------------
// 注释剥离
// ---------------------------------------------------------------------------
/// 剥掉 `//` 行注释与 `/* */` 块注释（**保留换行**，否则行号会错位）。
///
/// ⚠️ 只在需要"不许命中注释"的地方用 —— 见文件头「判据走去注释后的源码」。
/// 实现是够用就好：本仓这几个源文件里没有把 `//` 塞进字符串字面量的写法；
/// 但**正则字面量里出现 `//` 是有的**（`/^https?:\/\//`），所以先挡掉简单的正则场景。
function stripComments(src) {
  // 先挡 `://`（URL / 正则里的转义斜杠），别把它当注释起点。
  const shielded = src.replace(/(\\\/|:)\/\//g, '\u0000SLASH\u0000')
  const out = shielded
    .replace(/\/\*[\s\S]*?\*\//g, (m) => m.replace(/[^\n]/g, ' '))
    .replace(/(^|[^:])\/\/[^\n]*/g, (m, p1) => p1 + ' '.repeat(m.length - p1.length))
  return out.replace(/\u0000SLASH\u0000/g, '//')
}

// ---------------------------------------------------------------------------
// 参考推导（**独立实现，不是抄任何一处源码**）
// ---------------------------------------------------------------------------
/// 规格 §2.2 + §3.1 的派生规则，按**帧号域累加**写一遍。
///
/// 这是对拍的**参照真值**：两个消费点都必须与它一致。故意写得直白 ——
/// 它唯一的职责是把规格翻成代码，好让"谁漂了"一眼看出来。
///
/// ⚠️ **中间量必须落在同一格上**：先把**两端的秒各自**量化成源帧号
/// （`round(source_start*fps)` / `round(source_end*fps)`），**再**用边界差除以 speed。
/// 写成 `round((end-start)*fps/speed)` 在非帧对齐的秒上会比 Rust / 转译器差 1 帧，
/// 而那 1 帧会**顺延到后面所有段** —— 这正是本守卫要抓的那类回归。
/// 口径与 Rust `edl_clip_frames`（`types.rs`）逐式相同，注释里也写明了这个反例。
///
/// @param {Array<{source_start:number, source_end:number, speed:number}>} clips 秒域、有序
/// @param {number} fps 工程帧率（帧号域累加的分母）
function referenceResolve(clips, fps) {
  const out = []
  let cursor = 0 // **整数帧号**累加（R8）
  for (const c of clips) {
    const speed = c.speed ?? 1.0
    // 源两端各自量化成帧号（与 Rust `edl_frame_at` = `round(seconds*fps)` 同式）
    const sf = Math.round(c.source_start * fps)
    const ef = Math.round(c.source_end * fps)
    const srcFrames = ef - sf
    // 除以 speed 后再取整；累加只在整数帧号上做。
    const durFrames = Math.round(srcFrames / speed)
    if (!(durFrames >= 1)) continue // 非法段跳过（与 Rust `edl_clip_frames` 同口径）
    out.push({
      out_start: cursor,
      out_end: cursor + durFrames,
      source_start: c.source_start,
      source_end: c.source_end,
      speed,
      durFrames,
    })
    cursor += durFrames
  }
  return out
}

// ---------------------------------------------------------------------------
// 消费点 ②：转译器
// ---------------------------------------------------------------------------
/// 从 `buildKeepRanges` 的源码里抽出它的**累加式**，判断累加走的是帧域还是秒域。
///
/// ⚠️ `[去注释]` —— 必须剥注释：函数上方那段文档注释里**原样写着**
/// `+ (end - start) / speed`，不剥的话谓词命中注释，"改坏累加"永远不翻面。
function outStartAccumulatesInFrames(src) {
  const code = stripComments(src)
  const i = code.indexOf('function buildKeepRanges')
  if (i < 0) return null // 函数没了 —— 由调用方报 FAIL，别静默
  const body = code.slice(i, code.indexOf('\n}', i))
  // 帧域版：`outputCursor += range.durFrames ?? (range.end - range.start)`
  //   —— 第 2 批把 `durFrames`（可带 speed 的时长）引进来后，原来那个
  //   `outputCursor += range.end - range.start` 被**泛化**成了带回退的形式。
  //   ⚠️ 谓词仍然钉在**代码形状**上（`outputCursor +=` 后必须出现**整数帧**，不许出现 `/ speed` 或 `/ fps`），
  //   所以"把累加挪到秒域"这类回归照样翻面。
  const m = body.match(/outputCursor\s*\+=\s*([^;]+);/)
  if (!m) return null
  const expr = m[1]
  // 秒域的特征：拿时长去除以 speed / fps，或除以任何**非 1 的常数**（如 `/ 60`）。
  // ⚠️ 只认 `durFrames` 或 `end - start` 那种"整数帧直接相加"的形状。
  if (/\/\s*(speed|fps)\b/.test(expr)) return false
  if (/\/\s*[0-9]/.test(expr)) return false
  // 帧域：直接加 `durFrames` 或 `end - start`（两者都是整数帧）。
  return /durFrames|range\.end\s*-\s*range\.start/.test(expr)
}

// ---------------------------------------------------------------------------
// 对拍用例：一份**含 speed≠1、多段**的 clips
// ---------------------------------------------------------------------------
/// 这一份同时压住三件事：
///   · 多段累加（R8 漂移在段多时最明显）；
///   · `speed ≠ 1`（§4.2 第 2 条：`outStart` 累加要 `/(end-start)/speed`）；
///   · 乱序 / 重复源区间（重放与冷开场的形态）。
const CLIPS = [
  { source_start: 12.4, source_end: 18.9, speed: 1.0 },
  { source_start: 18.9, source_end: 24.4, speed: 1.0 },
  { source_start: 3.25, source_end: 6.75, speed: 0.5 },
  { source_start: 30.0, source_end: 42.0, speed: 2.0 },
  { source_start: 12.4, source_end: 18.9, speed: 1.0 }, // 重放：与第 1 段同源
]
const FPS = 60

// ---------------------------------------------------------------------------
// main
// ---------------------------------------------------------------------------
async function main() {
  console.log('check-edl-invariants：成片时间线（EDL）三处消费点**逐值**同源\n')

  const transSrc = readFileSync(TRANSLATOR, 'utf8')
  const expected = referenceResolve(CLIPS, FPS)

  console.log('参考推导（规格 §2.2 / §3.1，帧号域累加）:')
  for (const [i, r] of expected.entries()) {
    console.log(
      `  clip[${i}] 源 ${r.source_start}~${r.source_end}s x${r.speed} ⇒ ` +
        `out_start=${r.out_start} out_end=${r.out_end} (${r.durFrames} 帧)`,
    )
  }
  console.log(`  成片总帧数 = ${expected.length ? expected[expected.length - 1].out_end : 0}\n`)

  // ---- 消费点 ②：转译器的 outStart 累加域 ----
  console.log('消费点 ②：转译器 tools/polish-to-dhampir.mjs')
  const framesDomain = outStartAccumulatesInFrames(transSrc)
  check(
    '[②] buildKeepRanges 的 outStart 在**帧号域**累加（R8：秒域累加会让后面全段一起漂）',
    framesDomain,
    true,
  )

  // 真加载转译器，拿它**实际算出来**的 outStart 数列对拍（不是文本比对）
  const mod = await import(pathToFileURL(TRANSLATOR).href)
  if (typeof mod.buildKeepRanges !== 'function') {
    check('[②] 转译器导出 buildKeepRanges（导不出来 = 消费点不存在，必须报错）', typeof mod.buildKeepRanges, 'function')
  } else {
    // 把 CLIPS 当成"要保留的区间"喂进去：构造一个 totalFrames 让它原样保留。
    const totalFrames = 100000
    const got = mod.buildKeepRanges(totalFrames, [], true) // keepMuted ⇒ 整条区间
    check(
      '[②] keepMuted=true 仍退化成"整条一条保留区间"（§5：出片腿的存量语义不许变）',
      got.map((r) => [r.start, r.end, r.outStart]),
      [[0, totalFrames, 0]],
    )

    // mute 补集形态：outStart 必须**逐段**等于前面各段长度之和
    const muteFrames = [
      { start: 200, end: 300 },
      { start: 700, end: 900 },
    ]
    const sub = mod.buildKeepRanges(1200, muteFrames, false)
    const subExpected = []
    {
      let cursor = 0
      for (const [s, e] of [[0, 200], [300, 700], [900, 1200]]) {
        subExpected.push({ start: s, end: e, outStart: cursor })
        cursor += e - s
      }
    }
    check(
      '[②] 屏蔽补集下 outStart 逐段 = 前面各段长度之和（累加语义本身）',
      sub.map((r) => [r.start, r.end, r.outStart]),
      subExpected.map((r) => [r.start, r.end, r.outStart]),
    )

    // 成片总长：`durationFrames` 的口径 = 最后一段的 outStart + 自身长度
    const last = sub[sub.length - 1]
    check(
      '[②] 成片总长 = 最后一段 (outStart + 长度)（§2.2 规则 2 的口径）',
      last.outStart + (last.end - last.start),
      1200 - (100 + 200),
    )
  }

  // ---- 消费点 ③：前端成片轨（task-6，尚未落地）----
  console.log('\n消费点 ③：前端成片轨的 out_start 推导')
  let frontSrc = null
  try {
    frontSrc = readFileSync(FRONTEND, 'utf8')
  } catch {
    frontSrc = null
  }
  const frontHas = frontSrc ? /function\s+edlOutStart|edlOutStart\s*=/.test(stripComments(frontSrc)) : false
  if (!frontHas) {
    skip(
      '[③] 前端 edlOutStart 与 ②/① 逐值一致',
      frontSrc === null
        ? `前端模块不存在（${FRONTEND}）`
        : '前端还没有 edlOutStart 推导（第 3 批 task-6 才加）。' +
          '⚠️ 这不是通过 —— 三处同源目前**只有两处有判据**，撤掉这条守卫不能当成 R7 已覆盖。' +
          'task-6 请镜像 scripts/check-edl-invariants.mjs 的 referenceResolve：' +
          '同一份 CLIPS + 帧号域累加，输出 {out_start, out_end} 数列。',
    )
  } else {
    // 一旦落地，这里立刻开始真对拍（接口就位，task-6 不用再改本脚本的结构）。
    const fn = extractFrontResolve(frontSrc)
    if (fn === null) {
      skip(
        '[③] 前端 edlOutStart 与 ②/① 逐值一致',
        '前端出现了 edlOutStart 字样但**抽不出可执行的推导**（形状与约定不符）。' +
          '这必须人工看一眼 —— 别当成通过。',
      )
    } else {
      const got = fn(CLIPS, FPS)
      check(
        '[③] 前端推导的 out_start/out_end 与参考推导逐值一致',
        got.map((r) => [Math.round(r.out_start), Math.round(r.out_end)]),
        expected.map((r) => [r.out_start, r.out_end]),
      )
    }
  }

  // ---- 消费点 ①：Rust Edl::resolve（task-2，并发中）----
  console.log('\n消费点 ①：Rust Edl::resolve()（出片腿）')
  const rustState = probeRustResolve()
  if (!rustState.present) {
    skip(
      '[①] Rust Edl::resolve 的 out_start/out_end 与 ② 逐值一致',
      'Edl::resolve() 还不存在（task-2 并发中）。' +
        `已探测：types.rs Edl=${rustState.typesEdl}，config.rs 推导函数=${rustState.resolveFn ?? '无'}。` +
        '⚠️ 这不是通过 —— 今天钉住的只有消费点 ② 与(将来)③，' +
        '出片腿与预览腿的逐值同源**尚未被任何判据覆盖**。' +
        'task-2 落地后重跑本脚本，这条会自动从 SKIP 变成真对拍。',
    )
  } else {
    // 用**真跑出来的** Rust 值对拍（临时 cargo 工程，不碰仓库任何文件）。
    const rustOut = runRustResolveProbe()
    rustProbeResult = rustOut
    if (rustOut === null) {
      skip(
        '[①] Rust Edl::resolve 的 out_start/out_end 与 ② 逐值一致',
        'Edl::resolve 已存在，但**探针没能跑起来**。' +
          `原因：${probeError ?? 'cargo 输出里没有 PROBE_JSON 行'}。` +
          '⚠️ 不能当成通过 —— 修好探针再重跑，别把"跑不起来"读成"一致"。',
      )
    } else {
      check(
        `[①] Rust ${rustState.resolveFn}() 的 out_start_frame/out_end_frame 与参考推导逐值一致`,
        rustOut,
        expected.map((r) => [r.out_start, r.out_end]),
      )
      // 总帧数口径（§2.2 规则 2）：Rust 的 total_frames 也要与参照的最后一段 out_end 相同。
      const rustTotal = rustOut.length ? rustOut[rustOut.length - 1][1] : 0
      check(
        '[①] Rust total_frames 口径 = 最后一段 out_end（§2.2 规则 2，与 sequence_frames 对齐）',
        rustTotal,
        expected.length ? expected[expected.length - 1].out_end : 0,
      )
    }
  }

  // ---- 变异自证 ----
  console.log('\n变异自证（每个变异都必须让上面的判据**翻面**）:')
  await runMutations(transSrc, expected, rustState)

  // ---- 汇总 ----
  const compared = checks
  console.log('')
  if (skips.length) {
    console.log(`⚠️  ${skips.length} 项**跳过**（未覆盖 —— 别把这份输出读成"三处同源已验收"）:`)
    for (const s of skips) console.log(`    · ${s.label}`)
    console.log('')
  }
  if (failed) {
    console.log(`✗ ${failed}/${compared} 项失败，${skips.length} 项跳过`)
    return 1
  }
  if (skips.length) {
    // ⚠️ **不许打 `✓ 全部通过`**：有跳过 = R7（预览≠成片）**尚未成立**。
    // 一个 grep `✓ 全部通过` 的人/CI 不能把这份输出读成"同源已验收"。
    console.log(
      `△ ${compared} 项对拍通过，但 ${skips.length} 项**未覆盖**（R7 尚未成立）—— ` +
        '三处同源只钉住了有判据的那几处，见上面的跳过清单',
    )
    return 0
  }
  console.log(`✓ 全部通过（${compared} 项对拍）—— 三处消费点同源均有判据`)
  return 0
}

// ---------------------------------------------------------------------------
// 变异自证
// ---------------------------------------------------------------------------
/// 每个变异三步走（照 `check-yekitrim-translator.mjs` 的房规）：
///   1. 断言源码里**定位得到**变异点（命中 1 次）；
///   2. 断言变异后文本**真的不同**（否则"变异体"没变，检查当然不翻面）；
///   3. 要求检查**翻面**。
async function runMutations(transSrc, expected, rustState) {
  let allFlipped = true

  // 累加那一行的**真身**（第 2 批泛化成了 `range.durFrames ?? (range.end - range.start)`）。
  // ⚠️ 变异**不许写死旧形状** —— 写死的话第 2 批一改，变异就"定位不到"，自证变空转。
  const ACCUM_RE = /outputCursor\s*\+=\s*[^;]+;/
  const accumHits = (transSrc.match(new RegExp(ACCUM_RE.source, 'g')) || []).length
  const accumLine = accumHits === 1 ? transSrc.match(ACCUM_RE)[0] : null

  // —— 变异 1：把 outStart 累加从**帧域**换成**秒域**（R8 要防的就是这个）——
  {
    const label = '[②·变异] 累加改到秒域（R8）后「帧号域累加」这条必须翻面'
    if (accumHits !== 1) {
      check(`${label} —— 源码里定位不到变异点（命中 ${accumHits} 次，这项自证是空转的）`, accumHits, 1)
      allFlipped = false
    } else {
      // 把那一行整体换成秒域写法（保留 `??` 的回退语义，只把除数换成秒）。
      const mutated = transSrc.replace(ACCUM_RE, 'outputCursor += (range.end - range.start) / 60;')
      check(`${label} —— 变异体确实变了`, mutated !== transSrc, true)
      const stillFrames = outStartAccumulatesInFrames(mutated)
      check(label, stillFrames, false)
      if (stillFrames !== false) allFlipped = false
    }
  }

  // —— 变异 2：**往注释里塞正确字样**，证明谓词没被注释顶翻 ——
  {
    const label = '[②·变异] 只在**注释**里写累加式，谓词不许被骗'
    const injected = transSrc.replace(
      'function buildKeepRanges',
      `// 备注：${accumLine ?? 'outputCursor += range.end - range.start;'} 是旧版写法\nfunction buildKeepRanges`,
    )
    check(`${label} —— 注入确实命中`, injected !== transSrc, true)
    // 剥注释后谓词仍应看**真代码**；这条注入的注释不该改变判断结果。
    const before = outStartAccumulatesInFrames(transSrc)
    const after = outStartAccumulatesInFrames(injected)
    check(label, after, before)
    if (after !== before) allFlipped = false
  }

  // —— 变异 3：真加载变异体，证明**逐值对拍**抓得住 ——
  {
    const label = '[②·变异] 真加载一版 outStart 少加一段的转译器，「逐段累加」对拍必须翻面'
    if (accumHits !== 1) {
      check(`${label} —— 定位不到变异点`, accumHits, 1)
      allFlipped = false
    } else {
      const mutated = transSrc.replace(ACCUM_RE, 'outputCursor += 0; // 变异：忘了累加')
      const dir = mkdtempSync(join(tmpdir(), 'edl-mut-'))
      try {
        const file = join(dir, 'mutated-translator.mjs')
        writeFileSync(file, mutated, 'utf8')
        // **真加载**变异体（新 URL ⇒ 模块缓存不串），拿它算真值。
        const mutMod = await import(pathToFileURL(file).href)
        const sub = mutMod.buildKeepRanges(
          1200,
          [
            { start: 200, end: 300 },
            { start: 700, end: 900 },
          ],
          false,
        )
        const expectedSub = [{ outStart: 0 }, { outStart: 200 }, { outStart: 600 }]
        const bad = sub.map((r) => r.outStart).join(',') !== expectedSub.map((r) => r.outStart).join(',')
        check(label, bad, true)
        if (!bad) allFlipped = false
      } catch (e) {
        check(`${label} —— 变异体加载失败：${e.message}`, 'loaded', 'loaded')
        allFlipped = false
      } finally {
        rmSync(dir, { recursive: true, force: true })
      }
    }
  }

  // —— 变异 4：参照推导自己改坏（证明对拍**不是**两边一起改坏就通过）——
  {
    const label = '[参照·变异] 把 speed 换算摘掉（无视变速）后参照推导与上面的分帧对拍必须不一致'
    const refNoSpeed = (clips, fps) => {
      const out = []
      let cursor = 0
      for (const c of clips) {
        const dur = Math.round((c.source_end - c.source_start) * fps) // 漏掉 /speed
        out.push({ out_start: cursor, out_end: cursor + dur })
        cursor += dur
      }
      return out
    }
    const a = referenceResolve(CLIPS, FPS).map((r) => [r.out_start, r.out_end])
    const b = refNoSpeed(CLIPS, FPS).map((r) => [r.out_start, r.out_end])
    check(label, JSON.stringify(a) !== JSON.stringify(b), true)
    // 期望值本身也要与 CLIPS 里那两段 speed≠1 相符（钉死参照实现没写错）
    check(
      '[参照] speed=0.5 段时长翻倍、speed=2.0 段时长减半',
      [
        expected[2].durFrames,
        expected[3].durFrames,
        expected[2].durFrames === Math.round((6.75 - 3.25) * FPS * 2),
        expected[3].durFrames === Math.round((42.0 - 30.0) * FPS * 0.5),
      ],
      [Math.round((6.75 - 3.25) * FPS * 2), Math.round((42.0 - 30.0) * FPS * 0.5), true, true],
    )
  }

  // —— 变异 5：**消费点 ①（Rust）不能是空转的** ——
  //
  // 上面那条真对拍（跑 cargo）已经把真值取回来了。这里证明它**抓得住** Rust 侧的漂移：
  // 把"Rust 的返回值"换成"少 1 帧 / 秒域累加"的版本，同一份比较逻辑必须不等。
  // ⚠️ 不真去 mutate Rust 源（那是跨模块写 + 一次全量重编），而是变异**取回来的值** ——
  // 判据的比较部分与真跑那次**是同一段代码**，所以这个自证是有效的。
  if (rustProbeResult) {
    const label = '[①·变异] Rust 侧每段多 1 帧（或秒域累加）后，「与参考推导逐值一致」必须翻面'
    const good = rustProbeResult
    const expectPairs = expected.map((r) => [r.out_start, r.out_end])
    const same = (a, b) => JSON.stringify(a) === JSON.stringify(b)

    // (a) 每段多 1 帧 —— 累加漂移在 Rust 侧的形态
    const plusOne = good.map((p, i) => [p[0] + i, p[1] + i + 1])
    check(`${label}（每段 +1 帧）`, same(plusOne, expectPairs), false)

    // (b) 丢掉 speed 换算 —— 无视变速就一定分叉（本用例里有两段 speed≠1）
    const noSpeed = []
    {
      let cur = 0
      for (const c of CLIPS) {
        const dur = Math.round(c.source_end * FPS) - Math.round(c.source_start * FPS)
        noSpeed.push([cur, cur + dur])
        cur += dur
      }
    }
    check(`${label}（无视 speed）`, same(noSpeed, expectPairs), false)

    // (c) 正控：**没变异**的那份必须相等 —— 否则上面两条"翻面"可能只是比较函数坏了
    check('[①·变异·正控] 未变异的 Rust 真值必须与参考推导相等（比较函数本身没坏）', same(good, expectPairs), true)
  } else {
    check(
      '[①·变异] 消费点 ① 的真值对拍可用（探针跑得起来）',
      'probe-ran',
      rustState.present ? 'probe-ran' : 'not-implemented-yet',
    )
    if (rustState.present) allFlipped = false
  }

  if (allFlipped) {
    console.log('  → 全部变异都翻面了（本脚本不是空转的）')
  } else {
    console.log('  → ⚠️ 有变异**没翻面** —— 对应的判据抓不住这类回归')
  }
}

// ---------------------------------------------------------------------------
// 前端抽取（task-6 落地前不会走到）
// ---------------------------------------------------------------------------
/// 按**约定形状**从前端源码抽一个可执行的 out_start 推导。
///
/// 约定（发给 task-6 的接口）：前端导出一个纯函数
///     export function edlOutStart(clips, fps) -> Array<{out_start:number, out_end:number}>
/// 语义与 `referenceResolve` 完全一致：**帧号域累加**、含 speed。
/// 抽不到就返回 `null`（由调用方报 SKIP，**不当通过**）。
function extractFrontResolve(src) {
  const code = stripComments(src)
  const m = code.match(/export\s+function\s+edlOutStart\s*\(([^)]*)\)\s*\{/)
  if (!m) return null
  // 就地 eval 抽出函数体 —— 零依赖、不改仓库文件。
  try {
    const start = m.index + m[0].length - 1
    let depth = 0
    let end = -1
    for (let i = start; i < code.length; i += 1) {
      if (code[i] === '{') depth += 1
      else if (code[i] === '}') {
        depth -= 1
        if (depth === 0) {
          end = i + 1
          break
        }
      }
    }
    if (end < 0) return null
    const body = code.slice(start, end)
    // eslint-disable-next-line no-new-func
    return new Function(`return function(${m[1]}) ${body}`)()
  } catch {
    return null
  }
}

// ---------------------------------------------------------------------------
// Rust 探针
// ---------------------------------------------------------------------------
/// 探测 Rust 侧的 EDL 推导是否**已经存在**（task-2 落地前返回 present:false）。
///
/// 只做"在不在"的判断，**不当成对拍** —— 真对拍走 `runRustResolveProbe()`。
function probeRustResolve() {
  let typesEdl = false
  let resolveFn = null
  try {
    const t = stripComments(readFileSync(RUST_TYPES, 'utf8'))
    typesEdl = /\bpub\s+struct\s+Edl\b/.test(t)
  } catch {
    /* 文件还没建 */
  }
  try {
    const c = stripComments(readFileSync(RUST_CONFIG, 'utf8'))
    // ⚠️ 名字钉在**推导函数**上（`resolve_edl`），不是随便一个 `fn resolve` ——
    // 本文件里还有 `resolve_stage`，泛匹配会认错人。
    const m = c.match(/\bpub\s+fn\s+(resolve_edl|resolve)\s*\(/)
    resolveFn = m ? m[1] : null
  } catch {
    /* 文件还没建 */
  }
  return { present: Boolean(typesEdl && resolveFn), typesEdl, resolveFn }
}

/// 跑一个**真**的 Rust 探针，把 `Edl::resolve*` 的真值打回来。
///
/// 做法：在系统临时目录里生成一个一次性 cargo 工程（`path` 依赖本仓的
/// `yekitrim-polish` + `yekitrim-render-core`），跑它、读它 stdout 里的
/// `PROBE_JSON:` 行。
///
/// ⚠️ **为什么不在仓里放一个 `tests/` 用例**：本脚本的写权限只有自己那一个文件，
/// 而 `crates/yekitrim-polish/tests/` 归 `data-model` / task-2 —— 往那儿写就是
/// 跨模块改动。临时工程**一个仓库文件都不碰**，也不留下构建产物（`[workspace]`
/// 空表让它在临时目录自成 workspace，不污染本仓 `target/` 的成员图）。
///
/// 失败（cargo 不在 / 编译错 / 解析不出）一律返回 `null` → 调用方走 SKIP，
/// **绝不伪造真值**。
function runRustResolveProbe() {
  const state = probeRustResolve()
  if (!state.resolveFn) return null

  const dir = mkdtempSync(join(tmpdir(), 'edl-rust-probe-'))
  try {
    mkdirSync(join(dir, 'src'), { recursive: true })
    writeFileSync(
      join(dir, 'Cargo.toml'),
      [
        '[package]',
        'name = "edl-probe"',
        'version = "0.0.0"',
        'edition = "2021"',
        '',
        '[dependencies]',
        `yekitrim-polish = { path = ${JSON.stringify(join(ROOT, 'crates', 'yekitrim-polish'))} }`,
        `yekitrim-render-core = { path = ${JSON.stringify(join(ROOT, 'crates', 'yekitrim-render-core'))} }`,
        '',
        '[workspace]',
        '',
      ].join('\n'),
      'utf8',
    )
    // 探针源码里那三句与 CLIPS/FPS **逐值同源** —— 由本脚本生成，不手抄。
    // ⚠️ 每个数都要带小数点：JS 把 `1.0` 打印成 `1`，而 Rust 那边是 `f64` 元组 ——
    // 少了 `.0` 就是 `E0308`（探针编译不过，SKIP 变成"反正没判据"，很容易被忽略）。
    const f = (n) => (Number.isInteger(n) ? `${n}.0` : String(n))
    const clipLiterals = CLIPS.map(
      (c) => `        (${f(c.source_start)}, ${f(c.source_end)}, ${f(c.speed ?? 1.0)}),`,
    ).join('\n')
    writeFileSync(
      join(dir, 'src', 'main.rs'),
      [
        '// 由 scripts/check-edl-invariants.mjs 生成的一次性探针（临时目录，不入库）。',
        'use yekitrim_polish::project::ProjectFile;',
        'use yekitrim_render_core::types::{Edl, EdlClip};',
        '',
        'fn main() {',
        '    let clips: Vec<(f64, f64, f64)> = vec![',
        clipLiterals,
        '    ];',
        `    let fps = ${FPS}.0_f64;`,
        '    let core_clips: Vec<EdlClip> = clips',
        '        .iter()',
        '        .map(|(s, e, sp)| EdlClip {',
        '            source_start: *s,',
        '            source_end: *e,',
        '            speed: *sp,',
        '            kind: None,',
        '            label: None,',
        // ⚠️ 第 6 批给 `EdlClip` 加了三个 `Option` 字段（`source_kind` / `asset_id` /
        // `transition_in`，都是 `serde(default)`）。**Rust 的结构体字面量没有默认值**，
        // 所以这里必须逐个点名 —— 漏一个就是 `E0063`，探针**编译不过** ⇒ 本项 SKIP。
        //
        // ⚠️ 这正是本守卫最怕的失效方式：SKIP **不会**让脚本 exit 1，它只是**丢掉**消费点①
        // 的覆盖（连 `[①·变异]` 自证也一起**空转**）。所以"探针跑不起来"必须当成**失败**读，
        // 不许当成"通过"（下面那段 SKIP 文案已经这么写了，别再让探针静默过期）。
        // ⇒ **以后谁动 `EdlClip` 的字段，就要同时改这里。**
        '            source_kind: None,',
        '            asset_id: None,',
        '            transition_in: None,',
        '        })',
        '        .collect();',
        '    let cfg = ProjectFile {',
        '        edl: Edl { clips: core_clips },',
        '        ..ProjectFile::example()',
        '    };',
        '    let got = cfg.resolve_edl(3600.0, fps);',
        '    let parts: Vec<String> = got',
        '        .iter()',
        '        .map(|c| format!("[{},{}]", c.out_start_frame, c.out_end_frame))',
        '        .collect();',
        '    println!("PROBE_JSON:{{\\"clips\\":[{}],\\"total\\":{}}}",',
        '        parts.join(","), cfg.edl.total_frames(fps));',
        '}',
        '',
      ].join('\n'),
      'utf8',
    )

    const stdout = execFileSync('cargo', ['run', '--quiet'], {
      cwd: dir,
      encoding: 'utf8',
      // 编译输出很长；只留最后一段做诊断，别把整个 build log 灌进守卫输出。
      stdio: ['ignore', 'pipe', 'pipe'],
      maxBuffer: 64 * 1024 * 1024,
      // 首次编译要拉整条依赖图（本仓已编过，走增量会快很多）。
      timeout: 20 * 60 * 1000,
      env: { ...process.env, CARGO_TARGET_DIR: join(ROOT, 'target') },
    })
    const line = stdout.split('\n').find((l) => l.includes('PROBE_JSON:'))
    if (!line) return null
    const payload = JSON.parse(line.slice(line.indexOf('PROBE_JSON:') + 'PROBE_JSON:'.length))
    return payload.clips.map((c) => c.map((n) => Math.round(n)))
  } catch (e) {
    probeError = e.message.split('\n').slice(0, 3).join(' / ')
    return null
  } finally {
    rmSync(dir, { recursive: true, force: true })
  }
}

// ---------------------------------------------------------------------------
// 入口
// ---------------------------------------------------------------------------
const code = await main()
if (MUTATE) {
  // `--mutate` 与普通跑法共用同一份变异块；两者都必须 exit 0
  // （打红是在**变异体上**打红，不是让本进程失败）。
  console.log('\n（--mutate：上面的变异块已全部执行）')
}
process.exit(code)
