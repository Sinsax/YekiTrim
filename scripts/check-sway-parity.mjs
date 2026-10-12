#!/usr/bin/env node
// §5 T7 摇摆（P1）· **预览 ↔ 底座 逐值对拍**。
//
// ⚠️ 这条判据存在的理由（2026-10-13，本计划 P1）：
//
// 摇摆在 2026-10-13 已经**三处齐备**：转译器把 `[elements] sway_deg/sway_ms`
// 折成 `rotation` 关键帧（`swayKeyframes`）、底座 `evaluate_overlay` 按帧求值。
// **唯独预览那一份不存在** —— `engine.js::textTiltTransform` 只读静态 `tiltDeg`，
// `grep keyframe` / `grep sway` 命中数**都是 0**。
//
// 症状是本仓最恨的那一类：**配了摇摆、成片里字在摆、预览里纹丝不动，且不报任何错**。
// 用户调那两个旋钮看不到反应，只能出片才知道。
//
// ⇒ 现在预览有了 `swayDegAt`（与转译器同一个 `PATTERN` / 同一个 `quarter` 公式）。
//    **但"两份实现"正是漂移的温床** —— 所以这条判据把
//    「预览在 t 秒处的角度」与「底座读那批关键帧得到的角度」**逐值对拍**。
//
// # 对拍口径（为什么这样才是对的）
//
// 转译器产出的是**离散关键帧**（喂底座），预览要的是**连续时间上的角度**。
// 两者的等价关系：在**帧号** `f = t × fps` 处，
//   `swayDegAt(t, amp, periodMs, fps)` **必须等于**
//   `channelFrom(0, swayKeyframes(...), "rotation", f)`。
// 因为 `swayDegAt` 的取整/分段与 `swayKeyframes` 是同一套（`PATTERN=[0,1,0,-1]`、
// `quarter = max(1, round(periodMs/1000 × fps / 4))`），只有"离散 vs 连续"这一点差别。
//
// ⚠️ **非整帧时刻也照样对拍**：`swayDegAt` 在段内线性插值，
//    而 `channelFrom` 在**相邻两个关键帧之间**同样线性插值 ⇒ 两者在整段上都重合，
//    不只是整帧点。这是"预览每帧都画得对"的依据，不是"只在采样点对"。
import fs from 'node:fs'

const ENGINE = 'webui/src/polish/engine.js'
const TRANS = 'tools/polish-to-dhampir.mjs'

const fails = []
let checks = 0
function check(name, ok, why) {
  checks++
  if (!ok) fails.push(name + (why ? ' —— ' + why : ''))
}

// ---- 从源码里扒出被测函数（零依赖、不 import 应用模块；本目录惯用做法） ----
//
// ⚠️ `engine.js` 是纯 ESM、零依赖，**可以直接 import** —— 比正则扒文本可靠得多。
//    但 `webui/package.json` 没有 `"type": "module"`，故用动态 import 的 file: URL。
const engine = await import(new URL('../' + ENGINE, import.meta.url).href)
const swayDegAt = engine.swayDegAt
const channelFrom = engine.channelFrom

check(
  'engine.js 导出了 swayDegAt',
  typeof swayDegAt === 'function',
  '预览侧没有按帧求摇摆的函数 —— 那就是"配了不摆"',
)
check(
  'engine.js 导出了 channelFrom',
  typeof channelFrom === 'function',
  '预览侧没有关键帧求值 —— 认不出转译器写的 keyframes',
)

// ---- 从转译器源码里扒 `swayKeyframes`（它是内层闭包，import 不到） ----
const transSrc = fs.readFileSync(TRANS, 'utf8')
const fnStart = transSrc.indexOf('const swayKeyframes = (cue, cfg, tb) => {')
check('转译器里找得到 swayKeyframes', fnStart > 0, '关键帧的生产者没了')
// 朴素花括号配对取函数体（源码里没有字符串里带 `{}` 的干扰行）。
function bodyOf(s, from) {
  let i = s.indexOf('{', from)
  let depth = 0
  for (let j = i; j < s.length; j++) {
    if (s[j] === '{') depth++
    else if (s[j] === '}') {
      depth--
      if (depth === 0) return s.slice(i, j + 1)
    }
  }
  return ''
}
const swayBody = fnStart > 0 ? bodyOf(transSrc, fnStart) : ''
check('swayKeyframes 函数体取得出来', swayBody.length > 100, '花括号配对失败')

// 在沙箱里重建转译器那份实现（逐字用它的源码，不重写 —— 避免"判据自己写一份"）。
const swayKeyframes = swayBody
  ? new Function(
      'cue',
      'cfg',
      'tb',
      // ⚠️ 只把函数体交进去；它只依赖参数（源码里特意不读闭包 `timebase`）。
      swayBody,
    )
  : () => []

// ---- 对拍 ----
//
// 覆盖面刻意宽：角度 × 周期 × 帧率 × 时刻，四个维度都要有非平凡组合。
// ⚠️ 角度 0 / 周期 0 是"关"的分支，**也要测**（它们必须两边都返回 0 / fallback）。
const AMPS = [0, 3, 6, 8, 12, 20]
const PERIODS = [0, 300, 900, 2000, 5000]
const FPSS = [24, 25, 30, 50, 60]
const SPAN_SEC = 6.0

let compared = 0
let nonZero = 0
for (const amp of AMPS) {
  for (const periodMs of PERIODS) {
    for (const fps of FPSS) {
      const cue = { start: 0, end: SPAN_SEC * fps }
      const tb = { num: fps, den: 1 }
      const keys = swayKeyframes(cue, { sway_deg: amp, sway_ms: periodMs }, tb)
      const span = cue.end - cue.start

      // ① 整帧点（每帧都对，不只是采样点）
      for (let f = 0; f <= span; f++) {
        const t = f / fps
        const preview = swayDegAt(t, amp, periodMs, fps, span)
        // 底座口径：`channel_from(fallback=0, keys, "rotation", local_frame)`。
        // ⚠️ 没有键 ⇒ 返回 fallback（0）；而预览在"关"的分支也返回 0 ⇒ 两边一致。
        const base = keys.length
          ? channelFrom(0, keys, 'rotation', f)
          : 0
        compared++
        if (Math.abs(preview) > 1e-9) nonZero++
        if (Math.abs(preview - base) > 1e-6) {
          fails.push(
            `预览↔底座对拍不一致：amp=${amp} period=${periodMs}ms fps=${fps} ` +
              `frame=${f} 预览=${preview} 底座=${base}`,
          )
          f = span // 一组只报第一次，别刷屏
        }
      }
      // ② 帧间中点（证明"段内线性插值"两边同一条，而不只是整帧点重合）
      for (let f = 0; f < span; f++) {
        const t = (f + 0.5) / fps
        const preview = swayDegAt(t, amp, periodMs, fps, span)
        const base = keys.length ? channelFrom(0, keys, 'rotation', f + 0.5) : 0
        compared++
        if (Math.abs(preview - base) > 1e-6) {
          fails.push(
            `帧间中点不一致：amp=${amp} period=${periodMs}ms fps=${fps} ` +
              `frame=${f}+0.5 预览=${preview} 底座=${base}`,
          )
          break
        }
      }
    }
  }
}

check(`对拍样本够多（实测 ${compared} 点）`, compared > 5000, '样本太少，判据等于没跑')
// ⚠️ 如果全是 0，那"两边相等"毫无意义（两边都恒 0 也会全绿）——
//    这正是"判据空转"的经典形态，必须显式挡住。
check(`对拍点里有非零角度（实测 ${nonZero} 个）`, nonZero > 1000, '全是 0 ⇒ 判据在空转')

// ---- 逐条钉住语义（这些是"改坏了能抓住"的部分，不是对拍能覆盖的） ----

// ③ 峰值确实等于 ±amp（不是被 quarter 取整吃掉）
{
  const amp = 8
  const periodMs = 900
  const fps = 30
  const quarter = Math.max(1, Math.round((periodMs / 1000) * fps / 4))
  const atPeak = swayDegAt((quarter * 1) / fps, amp, periodMs, fps)
  const atTrough = swayDegAt((quarter * 3) / fps, amp, periodMs, fps)
  check(
    '① 四分之一周期处到达 +amp',
    Math.abs(atPeak - amp) < 1e-9,
    `期望 ${amp}，得到 ${atPeak}`,
  )
  check(
    '② 四分之三周期处到达 −amp（符号真的翻转）',
    Math.abs(atTrough - -amp) < 1e-9,
    `期望 ${-amp}，得到 ${atTrough}`,
  )
  check('③ t=0 处是 0（起手不歪）', Math.abs(swayDegAt(0, amp, periodMs, fps)) < 1e-9)
  // ④ **两个四分之一周期的整数倍**之后回到 0（不是单程）。
  //
  // ⚠️ 不能用"一个周期" `periodMs/1000` 去测：`quarter` 是**取整**的
  //    （`round(0.9*30/4) = 7` ⇒ 实际周期 28 帧 = 0.9333s，而请求的是 0.9s）。
  //    拿请求值当采样点会落在半路 —— 实测得到 −1.14°（那不是 bug，是我自己
  //    这条断言的前提错了）。要测"回到 0"就得按**实际**的 `4 × quarter` 去取点。
  const q4 = 4 * Math.max(1, Math.round((periodMs / 1000) * fps / 4))
  check(
    '④ 实际周期（4×quarter 帧）后回到 0',
    Math.abs(swayDegAt(q4 / fps, amp, periodMs, fps)) < 1e-9,
    `得到 ${swayDegAt(q4 / fps, amp, periodMs, fps)}`,
  )
}

// ⑤ 三个"关"的分支必须都返回 0（与转译器的 `return []` 逐条对应）
check('⑤ 角度 0 ⇒ 不摆', swayDegAt(1.0, 0, 900, 30) === 0)
check('⑥ 周期 0 ⇒ 不摆', swayDegAt(1.0, 8, 0, 30) === 0)
check('⑦ 周期负数 ⇒ 不摆', swayDegAt(1.0, 8, -900, 30) === 0)
check('⑧ fps 0 ⇒ 不摆（不猜帧率）', swayDegAt(1.0, 8, 900, 0) === 0)
check('⑨ 角度 NaN ⇒ 不摆', swayDegAt(1.0, NaN, 900, 30) === 0)

// ⑥ 幅度线性：角度翻倍 ⇒ 角位移翻倍（"线性于 amp"是这套波形的契约）
{
  const a = swayDegAt(0.25, 4, 900, 30)
  const b = swayDegAt(0.25, 8, 900, 30)
  check('⑩ 角度线性（8/4 = 2）', Math.abs(b - 2 * a) < 1e-9, `${a} → ${b}`)
}

// ⑦ 周期翻倍 ⇒ 到达同一角度需要的时刻翻倍（摆得更慢）
{
  const q1 = swayDegAt(0.25, 8, 900, 30)
  const slow = swayDegAt(0.5, 8, 1800, 30)
  check('⑪ 周期翻倍 ⇒ 摆得慢一半', Math.abs(q1 - slow) < 1e-6, `${q1} vs ${slow}`)
}

// ⑧ `channelFrom` 自己的四条语义（与底座 `curve.rs` 逐步对应）
{
  const keys = [
    { frame: 0, target: 'rotation', value: 0 },
    { frame: 10, target: 'rotation', value: 10 },
  ]
  check('⑫ 第一键之前取第一键', channelFrom(99, keys, 'rotation', -5) === 0)
  check('⑬ 最后一键之后取最后一键', channelFrom(99, keys, 'rotation', 99) === 10)
  check('⑭ 中间线性插值', Math.abs(channelFrom(99, keys, 'rotation', 5) - 5) < 1e-9)
  check('⑮ target 不匹配 ⇒ 回 fallback', channelFrom(99, keys, 'opacity', 5) === 99)
  check('⑯ 空数组 ⇒ 回 fallback', channelFrom(99, [], 'rotation', 5) === 99)
  // ⚠️ 契约不要求有序 —— 乱序输入必须自己排（与底座 `keys.sort_by_key` 同一条）
  const shuffled = [
    { frame: 10, target: 'rotation', value: 10 },
    { frame: 0, target: 'rotation', value: 0 },
  ]
  check('⑰ 乱序关键帧自己排', Math.abs(channelFrom(99, shuffled, 'rotation', 5) - 5) < 1e-9)
  // ⚠️ 同帧两个键 ⇒ 取后一个（防除零），与底座 `span <= 0 → t = 1`
  const same = [
    { frame: 5, target: 'rotation', value: 1 },
    { frame: 5, target: 'rotation', value: 2 },
  ]
  check('⑱ 同帧取后一个（不除零）', Number.isFinite(channelFrom(99, same, 'rotation', 5)))
}

// ---- 接线判据：预览真的把角度送上去了吗 ----
//
// ⚠️ 光有函数不算接通 —— 本仓踩过多次"函数写好、没人调"。
//    `textTiltTransform` 必须**接受并相加** `swayDeg`，`getElements` 必须**带出去**。
const engineSrc = fs.readFileSync(ENGINE, 'utf8')
// ⚠️ 别用 `\{[^}]*swayDeg`：解构默认值里有 `style = {}`，那个 `}` 会让正则提前停住
//    （第一版就是这么假红的 —— 代码是对的，判据自己写错了）。
//    改成"从头到第一个 `)` 之间含 swayDeg"，签名里没有嵌套括号。
check(
  '⑲ textTiltTransform 接受 swayDeg 参数',
  /export function textTiltTransform\(\{[^)]*swayDeg[^)]*\)/.test(engineSrc),
  'signature 里没有 swayDeg ⇒ 传进来也被丢掉',
)
check(
  '⑳ rotate 的角度里真的加上了 swayDeg',
  /const deg = tiltDeg \+ rot \+ \(Number\.isFinite\(swayDeg\)/.test(engineSrc),
  'swayDeg 没有进 rotate ⇒ 字不会摆',
)
// ⚠️ 别写"它必须是 `out.push({...})` 的**最后一个**字段" —— 第一版就是那么写的
//    （`swayDeg\s*\n\s*\}\)`），而 39 档那次在它后面又加了 `animName` / `textMotion`
//    ⇒ **判据当场假红**（代码是对的）。要钉的是"这个键在 `out.push` 那个对象里"，
//    不是"它排第几"。
const pushAt = engineSrc.indexOf('out.push({', engineSrc.indexOf('export function getElements'))
const pushObj = pushAt >= 0 ? engineSrc.slice(pushAt, pushAt + 3000) : ''
check(
  '㉑ getElements 把 swayDeg 带出去了',
  /\bswayDeg\b/.test(pushObj),
  '求值结果里没有 swayDeg ⇒ 组件读不到',
)
check(
  '㉑b getElements 把 animName 带出去了',
  /\banimName\b/.test(pushObj),
  '元素上没有 animName ⇒ textTiltTransform 查不到 tilt_* / slam_squash 那两个静态值',
)
check(
  '㉒ getElements 真的调了 swayDegAt',
  /swayDegAt\(local, mo\.swayDeg/.test(engineSrc),
  '没有调用 ⇒ 恒 0',
)
check(
  '㉓ textMotionCfg 带上了两个旋钮',
  /swayDeg: num\(e\.sway_deg, 0\)/.test(engineSrc) && /swayMs: num\(e\.sway_ms, 0\)/.test(engineSrc),
  '配置没读 sway_deg/sway_ms ⇒ 预览永远不知道该摆',
)
check(
  '㉔ buildIndex 接受 fps',
  /export function buildIndex\(\{[^}]*fps[^}]*\}\)/.test(engineSrc),
  '拿不到 fps ⇒ swayDegAt 恒返回 0',
)
check(
  '㉕ idx.fps 不猜默认值',
  /idx\.fps = Number\.isFinite\(fps\) && fps > 0 \? fps : 0/.test(engineSrc),
  '猜一个帧率会让预览与成片的 quarter 不同，且两边都不报错',
)
// ⚠️ 转译器的 PATTERN / quarter 公式必须与预览那份一致（这是"逐值对拍能过"的前提）
check(
  '㉖ 转译器用同一套 PATTERN',
  /const PATTERN = \[0, 1, 0, -1\]/.test(transSrc),
  '转译器的峰值序列变了 ⇒ 预览那份也要跟着改',
)
check(
  '㉗ 两边 quarter 公式同形',
  /Math\.max\(1, Math\.round\(\(periodMs \/ 1000\) \* fps \/ 4\)\)/.test(engineSrc),
  '预览的 quarter 公式与转译器不同形 ⇒ 摆的节奏对不上',
)

// ---- 结果 ----
if (fails.length === 0) {
  console.log(
    `✓ 摇摆预览↔底座一致（${checks} 项判据 / ${compared} 个对拍点，其中 ${nonZero} 个非零）`,
  )
  process.exit(0)
}
console.log(`✗ 摇摆对拍失败（${fails.length} / ${checks}）：`)
for (const f of fails.slice(0, 20)) console.log('    ' + f)
process.exit(1)
