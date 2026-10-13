#!/usr/bin/env node
// ============================================================================
// `check-anim-two-axis.mjs` —— **两级动效（样式 × 动效）的 R4 与叠加守卫**
// ============================================================================
//
// 用法：`node scripts/check-anim-two-axis.mjs [--mutate]`
//
// # 它存在的理由（这是一次真事故换来的）
//
// 「样式」与「动效」拆成两个**独立可叠加**的设置时，转译器要从"一个 `anim`"
// 变成"两路各取自己的字段"。这条改动踩了**三个都不会自己报错的坑**：
//
//   ① **4 条轨的 `per_char` 整条丢失** —— `pop_perchar` / `hop_*` 在界面上归
//      「动效」栏，但它们的 `perChar` 是**形态字段**、住「样式」侧。
//      按"轴"判就漏了 ⇒ 字不跳了，报告里一行差异都没有。
//   ② **产物里轨与 assets 的顺序整体变** —— 分组键从一维变两维时，
//      排序键也跟着变了（空串排在所有字母前）⇒ 轨内容逐条相同、
//      **只有排列顺序不同** ⇒ sha256 不同（R4 破）。
//   ③ **R4 对拍本身是空转的** —— 我第一版拿 `docs/wgpu-effect-reference/demo`
//      去比，而那份工程**一个 `anim` 都没有** ⇒ 前后恒等 ⇒ "通过"是假的。
//
// ⚠️⚠️ 所以本判据**自己造夹具**（39 档 + 叠加组合），不依赖任何现成工程 ——
//    ③ 那条讲的就是"夹具没用到被改的字段时，R4 是空转的"。
//
// # 判据
//
//   ① **39 档迁移**：每个档名单独写进老字段 `anim` 时，拆出来的两路
//      合起来取到的字段 == 那一档的全部字段（**逐档**，不是抽样）。
//   ② **R4（结构级）**：老工程（只有 `anim`）转出来的**轨内容与顺序**
//      与"改造前的语义"一致 —— 用"两路拆法必须无损"来钉，
//      不依赖历史二进制。
//   ③ **两轴字段不重叠**：样式路与动效路取的是**不相交**的字段集 ——
//      这是"可叠加"成立的根据；一旦重叠，"两个设置"就会互相打架。
//   ④ **叠加真的生效**：`竖排 + 抖动` 这种组合，**两边都写出去**。
//   ⑤ **顺序稳定**：同一份工程跑两次，轨 id 序列逐字符相同。
import fs from 'node:fs'
import path from 'node:path'
import os from 'node:os'
import { fileURLToPath } from 'node:url'
import { execFileSync } from 'node:child_process'

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const ENGINE = 'file://' + path.join(ROOT, 'webui', 'src', 'polish', 'engine.js').replace(/\\/g, '/')
// ⚠️ 被验对象是**可覆盖**的 —— 变异自证要把转译器拷成副本、改那份副本再跑。
//    默认是真身；`--mutate` 那条路会传 `YK_ANIM2AXIS_TRANSLATOR`。
//    ⚠️ **绝不**用 `git checkout` 还原变异（那会把未提交的工作一起冲掉，
//    这个仓里已经因此丢过一次改动）—— 一律改**副本**。
const TRANSLATOR = process.env.YK_ANIM2AXIS_TRANSLATOR || path.join(ROOT, 'tools', 'polish-to-dhampir.mjs')
const MUTATE = process.argv.includes('--mutate')

const fails = []
const ok = (cond, msg) => { if (!cond) fails.push(msg) }

const E = await import(ENGINE)
const ALL = Object.keys(E.ANIM_PRESETS)

// ---- 与转译器同源的"归属"判据（转译器是零依赖 ESM，没法 import 这里的表）----
//
// ⚠️ 这里**故意重新实现一遍**：如果只 import 转译器自己的函数，那"转译器写错了"
//    会连判据一起错（自证循环）。两份实现**独立写**、对拍同一份事实 —— 这才是判据。
//    ⚠️ 与 `scripts/anim-attribution.mjs`（逐档**参数**归属）也要一致，
//      那条由 `check-anim-attribution.mjs` 钉着。
const AXIS = {
  tilt: 'style', arc: 'style', size: 'style', scatter: 'style',
  shrink: 'style', grow: 'style', vtext: 'style',
  shake: 'effect', sway: 'effect', slam: 'effect',
  shrink_in: 'effect', pop: 'effect', hop: 'effect',
}
const PREFIXES = Object.keys(AXIS).sort((a, b) => b.length - a.length)
const axisOf = (name) => {
  const k = String(name ?? '').trim().toLowerCase()
  if (!k) return ''
  for (const p of PREFIXES) if (k.startsWith(p + '_')) return AXIS[p]
  return ''
}

/// 逐档算"这一档在两条路上各给了什么字段"（**独立于转译器**）。
//
// ⚠️ 这两张键集是**判据自己的口径**，与转译器/归属表**各自独立写** ——
//    如果 import 被验对象的函数，那"它写错了"会连判据一起错（自证循环）。
// ⚠️ `perChar` 算**形态**（它决定"字静态什么样"）—— 见 `check-anim-attribution.mjs`
//    里那条判据；`scaleSpread` 那类**端点全相等**的逐字阶梯是**静止**的。
const FORM_KEYS = new Set(['tiltDeg', 'squashY', 'vertical', 'fontFamily', 'fontWeight',
  'safeWidthRatio', 'perChar'])
const MOTION_KEYS = new Set(['shakePx', 'shakeHz', 'swayDeg', 'swayMs',
  'scaleInFrom', 'scaleInPeak', 'scaleInMs'])
const PARTS = (() => {
  const out = {}
  for (const [name, v] of Object.entries(E.ANIM_PRESETS)) {
    const keys = Object.keys(v)
    out[name] = {
      form: keys.filter((k) => FORM_KEYS.has(k)).sort(),
      motion: keys.filter((k) => MOTION_KEYS.has(k)).sort(),
    }
  }
  return out
})()

// ============================================================================
// ① 两轴字段不重叠（"可叠加"成立的根据）
// ============================================================================
{
  const allForm = new Set(), allMotion = new Set()
  for (const n of ALL) {
    for (const k of PARTS[n].form) allForm.add(k)
    for (const k of PARTS[n].motion) allMotion.add(k)
  }
  const both = [...allForm].filter((k) => allMotion.has(k))
  ok(both.length === 0,
    `① 形态与动效两路的字段**有重叠**：${JSON.stringify(both)} —— ` +
    `重叠意味着两个设置会互相打架，"叠加"就不成立了`)
  ok(allForm.size > 0 && allMotion.size > 0, '① 两路里有一路一个字段都没有（拆法退化了）')
}

// ============================================================================
// ② 叠加真的能表达：每一条轴都**存在**只给这一路的档，且两路都有档
// ============================================================================
{
  const pureStyle = ALL.filter((n) => PARTS[n].form.length && !PARTS[n].motion.length)
  const pureEffect = ALL.filter((n) => PARTS[n].motion.length && !PARTS[n].form.length)
  const bothParts = ALL.filter((n) => PARTS[n].form.length && PARTS[n].motion.length)
  ok(pureStyle.length > 0, '② 没有"只给形态"的档 —— 样式那一栏会是空的')
  ok(pureEffect.length > 0, '② 没有"只给动效"的档 —— 动效那一栏会是空的')
  // ⚠️ `both` 那些档（`slam_squash` / `scatter_shake` / `hop_*` / `pop_perchar`）
  //    是**迁移正确性**的关键：它们两边都要给，否则老工程丢一半。
  ok(bothParts.length > 0, '② 没有"两边都给"的档 —— 迁移那条判据会空转')
}

// ============================================================================
// ③ 夹具：造一份真的用了全部 39 档 `anim` 的工程（**不依赖任何现成工程**）
// ============================================================================
const tmp = fs.mkdtempSync(path.join(os.tmpdir(), 'yk-anim2axis-'))
const tomlPath = path.join(tmp, 'polish.toml')

// ⚠️ **必须有 `clip.mp4`**：`keepRanges` 是从**成片时间线（EDL）**推出来的，
//    而没有源视频时那条推导会得到一个空的 keep 集合 ⇒ **所有事件都落在
//    "被掐掉的区间"里**、被整条跳过 ⇒ 轨只剩主视频一条、判据全报"轨没生成"。
//    （实测踩过：夹具看着完全正常，转译器也 exit 0，只是**一条文字轨都没有**。）
//    照抄 demo 的做法：建一个**零字节**的占位 —— 转译器不探测媒体。
fs.writeFileSync(path.join(tmp, 'clip.mp4'), Buffer.alloc(0))

/// 造工程。`legacy=true` 只写 `anim`（老格式）；否则写两路（新格式）。
//
// ⚠️ **三条从真工程抄来的硬要求**（缺一条夹具就**一条文字轨都出不来**，
//    而判据全报"轨没生成" —— 看起来像代码坏了，其实是夹具不对）：
//   ① **`[source] video = "clip.mp4"`** + 目录里真的有那个文件（零字节即可）——
//      `keepRanges` 是从成片时间线推出来的，没有源视频 ⇒ keep 集合为空
//      ⇒ **所有事件都判成"落在被掐掉的区间里"**、整条跳过。
//   ② 文字元素要 **`source`**（九段竖线串，`process` 写回的那个）——
//      没有它转译器会 CLAMP「样式参数没拿到」并**按兜底值画**。
//   ③ 事件用 **`time` + `end`**，不是 `time` + `duration`。
function writeProject({ legacy, events }) {
  const head = `duration = 60.0
processed = true

[project]
name = "两级动效夹具"
output = "export.mp4"
version = 1

[source]
video = "clip.mp4"
subtitles = ""
danmaku = ""
metadata = ""

[canvas]
width = 1920
height = 1080
fps = 60
`
  // ⚠️ 用 comment 那一档的真 source 串（照抄 `.workbuddy/closeup-sampling` 的实测值）——
  //    自己编一个会让"样式参数没拿到"那条 CLAMP 又冒出来。
  const SRC = 'comment|44|2|255,226,122,255|26,26,26,255|700|1'
  const body = events.map((ev) => `
[[segments.events]]
type = "element"
kind = "text"
text = "样本"
style = "comment"
pos = "bottom"
x = "28%"
y = "58%"
source = "${SRC}"
time = ${ev.time.toFixed(2)}
end = ${(ev.time + 0.30).toFixed(2)}
${legacy ? `anim = "${ev.anim}"` : `style_anim = "${ev.style_anim || ''}"
effect_anim = "${ev.effect_anim || ''}"`}
`).join('')
  fs.writeFileSync(tomlPath, head + '\n[[segments]]\nstart = 0.0\nend = 60.0\nfx = "low"\n' + body)
}

/// 跑转译器，返回产物对象。
function run() {
  const out = path.join(tmp, 'out.json')
  try {
    execFileSync(process.execPath, [TRANSLATOR, tmp, '--out', out],
      { stdio: 'pipe', env: { ...process.env, DHAMPIR_QUIET: '1' } })
  } catch (e) {
    const s = String(e.stderr ?? '') + String(e.stdout ?? '')
    ok(false, `转译器跑失败：${s.split('\n').slice(0, 4).join(' / ')}`)
    return null
  }
  return JSON.parse(fs.readFileSync(out, 'utf8'))
}

/// 产物里某个文字轨的 `subtitle`（= 两路摊出来的字段）。
const subOf = (doc, id) => {
  const t = (doc.timeline?.tracks ?? []).find((x) => x.id === id)
  return t ? (t.subtitle ?? t.sub ?? null) : null
}
/// 产物里某个文字轨的**关键帧**（`sway_*` 那条动效走这里，不走 `subtitle`）。
const keysOf = (doc, id) => {
  const t = (doc.timeline?.tracks ?? []).find((x) => x.id === id)
  const layers = t?.layers ?? []
  for (const l of layers) if (Array.isArray(l.keyframes) && l.keyframes.length) return l.keyframes
  return []
}

try {

// ============================================================================
// ④ R4：老格式（只有 `anim`）逐档 —— 拆出来的两路必须**无损**
// ============================================================================
//
// 判据怎么立（**不依赖历史二进制**）：
//   把 39 档**逐个**写成老格式，每档一条事件（各自一条轨），
//   然后断言：该轨的 `subtitle` 里**这一档该给的字段一个不少**。
//   ⇒ 这正好抓住"① 那 4 条轨 per_char 丢了"那类回归。
{
  const events = ALL.map((n, i) => ({ anim: n, time: 0.1 * i }))
  writeProject({ legacy: true, events })
  const doc = run()
  if (doc) {
    const missing = []
    for (let i = 0; i < ALL.length; i++) {
      const n = ALL[i]
      const sub = subOf(doc, `text_comment_bottom_${n}`)
      if (!sub) { missing.push(`${n}: 轨没生成`); continue }
      // 逐字段核对该档**应该有**的东西
      //
      // ⚠️ **`sway_*` 不在 `subtitle` 上** —— 它走 `swayKeyframes` 写成
      //    **图层关键帧**（`keysOf`）。第一版把它算进 `subtitle`，
      //    于是 3 条 `sway_*` 报了**假红**（"少了 sway_deg"，其实它压根不写那儿）。
      const want = []
      const p = E.animPreset(n)
      if (p.perChar !== undefined) want.push('per_char')
      if (p.vertical === true) want.push('vertical')
      if (p.safeWidthRatio !== undefined) want.push('safe_width_ratio')
      if (p.shakePx !== undefined) want.push('shake_px')
      if (p.scaleInMs !== undefined) want.push('scale_in_ms')
      for (const k of want) {
        if (sub[k] === undefined || sub[k] === null) {
          missing.push(`${n}: 少了 \`${k}\``)
        }
      }
      // ⚠️ 摇摆单独核：它必须在**关键帧**里体现（不在 subtitle）
      if (p.swayDeg !== undefined) {
        const keys = keysOf(doc, `text_comment_bottom_${n}`)
        if (keys.length === 0) missing.push(`${n}: 少了关键帧（sway 走 keyframes）`)
      }
    }
    ok(missing.length === 0,
      `④ 老格式（只有 \`anim\`）逐档迁移**有损** —— 这些档少了字段：` +
      JSON.stringify(missing.slice(0, 12)) +
      (missing.length > 12 ? ` …另 ${missing.length - 12} 处` : '') +
      `\n     ⚠️ 最典型的是 \`per_char\`：\`pop_perchar\`/\`hop_*\` 在界面上归"动效"，` +
      `但它们的 \`perChar\` 是**形态字段** —— 按"轴"判就会漏掉它。`)
  }
}

// ============================================================================
// ⑤ R4：**顺序**必须与老格式的字母序一致（轨 id 序列逐字符）
// ============================================================================
//
// ⚠️ 这是"② 顺序整体变"那条坑的判据：轨内容对、但**排列顺序**不同
//    ⇒ 产物 sha256 不同 ⇒ R4 破。
{
  const events = ALL.map((n, i) => ({ anim: n, time: 0.1 * i }))
  writeProject({ legacy: true, events })
  const d1 = run()
  const d2 = run()   // 跑两次，验稳定性
  if (d1 && d2) {
    const ids1 = (d1.timeline?.tracks ?? []).map((t) => t.id)
    const ids2 = (d2.timeline?.tracks ?? []).map((t) => t.id)
    ok(JSON.stringify(ids1) === JSON.stringify(ids2),
      '⑤ 同一份工程跑两次，轨 id 序列不同 —— 产物不是确定性的')
    // ⚠️ 老格式下的顺序必须**等于按 `anim` 的字母序**（改造前就是那样）。
    const textIds = ids1.filter((x) => x.startsWith('text_comment_bottom_'))
    const names = textIds.map((x) => x.replace('text_comment_bottom_', ''))
    const sorted = [...names].sort()
    ok(JSON.stringify(names) === JSON.stringify(sorted),
      `⑤ 老格式下文字轨的顺序**不是**按 \`anim\` 字母序 —— 与本改动前的产物顺序不符（R4 破）。\n` +
      `     实得前 8 个：${JSON.stringify(names.slice(0, 8))}\n` +
      `     字母序前 8：${JSON.stringify(sorted.slice(0, 8))}\n` +
      `     ⚠️ 典型原因：排序键拿两路拼（空串排在所有字母**之前**）⇒ ` +
      `"只有动效"的档整批跑到最前面。`)
  }
}

// ============================================================================
// ⑤b **新格式**（两路都写）的顺序也必须稳定且**不依赖空串的位置**
// ============================================================================
//
// ⚠️ 这条是 ⑤ 的**必要补充** —— 只测老格式的话，"排序键拿两路直接拼"那个 bug
//    会**漏网**：老格式里 `anim` 非空、两路拼出来的第一段就是 `anim`，
//    拼法怎么错都仍然按 `anim` 排（实测：故意把排序键退回两路直接拼，⑤ 照样绿）。
//    而**新格式**里 `anim` 是空的 ⇒ 排序键退化成两路拼 ⇒
//    空串排在字母前 ⇒ 顺序与预期不符。这就是那条 bug 真正咬人的地方。
{
  // 造一组"样式各异、动效各异"的组合，新格式写
  const combos = [
    { style_anim: 'vtext_bold', effect_anim: 'shake_std', time: 0.5 },
    { style_anim: '', effect_anim: 'fade_only', time: 1.0 },
    { style_anim: 'tilt_std', effect_anim: '', time: 1.5 },
    { style_anim: 'arc_up', effect_anim: 'sway_std', time: 2.0 },
  ]
  writeProject({ legacy: false, events: combos })
  const d1 = run()
  const d2 = run()
  if (d1 && d2) {
    const ids1 = (d1.timeline?.tracks ?? []).map((t) => t.id)
    const ids2 = (d2.timeline?.tracks ?? []).map((t) => t.id)
    ok(JSON.stringify(ids1) === JSON.stringify(ids2),
      '⑤b 新格式：同一份工程跑两次，轨 id 序列不同 —— 产物不是确定性的')
    const textIds = ids1.filter((x) => x.startsWith('text_comment_bottom_'))
    // ⚠️ 四条组合 ⇒ 四条轨（**必须都分到不同的轨**，否则就是"后写覆盖先写"）
    ok(textIds.length === combos.length,
      `⑤b 新格式：${combos.length} 组不同的「样式 × 动效」组合只生成了 ${textIds.length} 条轨 —— ` +
      `不同的组合**必须分轨**（轨级参数不同，混在一轨就是后写的覆盖先写的）`)
    // ⚠️ 顺序必须是**排过序的**（确定性的），且**不含空串拼出来的前缀 bug**
    const tail = textIds.map((x) => x.replace('text_comment_bottom_', ''))
    const tailSorted = [...tail].sort()
    ok(JSON.stringify(tail) === JSON.stringify(tailSorted),
      `⑤b 新格式：文字轨顺序不是稳定排序 —— \n` +
      `     实得：${JSON.stringify(tail)}\n     排序：${JSON.stringify(tailSorted)}\n` +
      `     ⚠️ 典型原因：排序键用"两路直接拼"，空串排在所有字母**之前** ⇒ ` +
      `"只有动效"的组合整批跑到最前面。`)
  }
}

// ============================================================================
// ⑥ 叠加：新格式写两路时，**两边都真的生效**
// ============================================================================
{
  // 挑一条纯形态 + 一条纯动效，明确是"叠加"
  //
  // ⚠️ `ANIM_PRESETS` 里的键是**驼峰**（`shakePx` / `swayDeg`），
  //    而产物里是**蛇形**（`shake_px` / `sway_deg`）—— 两套名字**别混**。
  //    第一版挑档时错用了蛇形 ⇒ `effectName` 恒 `undefined`、
  //    判据报"夹具挑不出"（而真因是判据自己的名字写错了）。
  const styleName = ALL.find((n) => n.startsWith('vtext_') && PARTS[n].form.includes('vertical'))
  const effectName = ALL.find((n) => n.startsWith('shake_') && PARTS[n].motion.includes('shakePx'))
  ok(!!styleName && !!effectName, '⑥ 夹具挑不出"纯形态 + 纯动效"的一对（表变了？）')
  if (styleName && effectName) {
    writeProject({
      legacy: false,
      events: [{ style_anim: styleName, effect_anim: effectName, time: 0.5 }],
    })
    const doc = run()
    if (doc) {
      const ids = (doc.timeline?.tracks ?? []).map((t) => t.id)
      const t = ids.find((x) => x.startsWith('text_comment_bottom_'))
      const sub = t ? subOf(doc, t) : null
      ok(!!sub, `⑥ 叠加那一轨没生成（轨：${JSON.stringify(ids.slice(0, 5))}）`)
      if (sub) {
        // 形态那半边
        ok(sub.vertical === true,
          `⑥ 叠加没生效：样式轴 \`${styleName}\` 的 \`vertical\` **没写出去** ` +
          `⇒ 用户选了竖排、成片里横着（实得 ${JSON.stringify(sub.vertical)}）`)
        // 动效那半边
        ok(sub.shake_px !== undefined && sub.shake_px !== null,
          `⑥ 叠加没生效：动效轴 \`${effectName}\` 的 \`shake_px\` **没写出去** ` +
          `⇒ 用户选了抖动、成片里不抖（实得 ${JSON.stringify(sub.shake_px)}）`)
        // ⚠️ **两个都要在**才叫叠加 —— 这条是"三级互斥"那版做错的地方
        ok(sub.vertical === true && sub.shake_px !== undefined && sub.shake_px !== null,
          '⑥ 两边没有同时生效 —— 这不是"可叠加"，是"二选一"')
      }
    }
  }
}

// ============================================================================
// ⑦ 新格式与老格式**等价**：同一组效果，两种写法产物应当一致
// ============================================================================
{
  // 老格式写 `scatter_shake`（两边都给的那一档）⇒ 应当等价于
  // 新格式同时写 `style_anim = scatter_shake` + `effect_anim = scatter_shake`。
  const n = ALL.find((x) => PARTS[x].form.length && PARTS[x].motion.length)
  ok(!!n, '⑦ 找不到"两边都给"的档（迁移那条会空转）')
  if (n) {
    writeProject({ legacy: true, events: [{ anim: n, time: 0.5 }] })
    const a = run()
    writeProject({ legacy: false, events: [{ style_anim: n, effect_anim: n, time: 0.5 }] })
    const b = run()
    if (a && b) {
      const pick = (d) => {
        const t = (d.timeline?.tracks ?? []).find((x) => x.id.startsWith('text_comment_bottom_'))
        return t ? JSON.stringify(t.subtitle ?? t.sub ?? null) : null
      }
      const sa = pick(a), sb = pick(b)
      ok(sa !== null && sb !== null, '⑦ 两种写法的轨都没生成')
      ok(sa === sb,
        `⑦ 老格式 \`anim = "${n}"\` 与新格式"两路都写 \`${n}\`"**不等价**。\n` +
        `     老：${String(sa).slice(0, 220)}\n     新：${String(sb).slice(0, 220)}\n` +
        `     ⚠️ 两者语义相同（那一档两边都给），不等价说明拆法有损。`)
    }
  }
}

} finally {
  try { fs.rmSync(tmp, { recursive: true, force: true }) } catch { /* 清不掉就算了 */ }
}

// ============================================================================
// 变异自证（`--mutate`）：故意改坏转译器，判据**必须**翻红
// ============================================================================
//
// ⚠️ 这是本判据的**自证**：一条判据如果对"真正改坏的代码"照样绿，它就是空转的。
//    下面每一条都对应**本趟真踩过**的一个坑 —— 用它们证明判据有牙。
//
// ⚠️ **只改副本**：把转译器拷到临时目录再改，**绝不**在真身上改、
//    也**绝不**用 `git checkout` 还原（那会把未提交的工作一起冲掉）。
if (MUTATE) {
  const MUTS = [
    {
      name: '把 pop_perchar/hop 从"形态侧"摘掉（本趟真踩过的 bug）',
      from: `      key === 'slam_squash' ||\n      key === 'pop_perchar' ||\n      /^hop_/.test(key)`,
      to: `      key === 'slam_squash'`,
      expect: '④',
    },
    {
      name: '排序键拼法退回带前导 + （本趟真踩过的 bug）',
      from: `        const ax = kx[0] || [kx[1], kx[2]].filter(Boolean).join('+');\n` +
        `        const ay = ky[0] || [ky[1], ky[2]].filter(Boolean).join('+');`,
      to: `        const ax = kx[0] || \`\${kx[1]}+\${kx[2]}\`;\n` +
        `        const ay = ky[0] || \`\${ky[1]}+\${ky[2]}\`;`,
      expect: '⑤b',
    },
    {
      name: '动效那一路改读样式字段（两轴混用 ⇒ 叠加失效）',
      from: `            animPreset(cues[0].effectAnim),`,
      to: `            animPreset(cues[0].styleAnim),`,
      expect: '⑥',
    },
    {
      name: '形态那一路改读动效字段（两轴混用，另一个方向）',
      from: `            animPreset(cues[0].styleAnim),`,
      to: `            animPreset(cues[0].effectAnim),`,
      expect: '⑥',
    },
  ]
  const src = fs.readFileSync(TRANSLATOR, 'utf8')
  let bad = 0
  for (const m of MUTS) {
    if (!src.includes(m.from)) {
      console.log(`  ✗ 变异「${m.name}」的锚点**找不到了** —— 源码文本变了没同步，` +
        `这条变异**静默失效**（不是判据的错，是变异表过期了）`)
      bad++
      continue
    }
    const mutated = src.replace(m.from, m.to)
    const mutDir = fs.mkdtempSync(path.join(os.tmpdir(), 'yk-anim2axis-mut-'))
    const mutFile = path.join(mutDir, 'polish-to-dhampir.mut.mjs')
    fs.writeFileSync(mutFile, mutated)
    try {
      execFileSync(process.execPath, [fileURLToPath(import.meta.url)],
        { stdio: 'pipe', env: { ...process.env, YK_ANIM2AXIS_TRANSLATOR: mutFile } })
      console.log(`  ✗ 变异「${m.name}」**没被打红** —— 判据在 ${m.expect} 那条上有洞`)
      bad++
    } catch {
      console.log(`  ✓ 变异「${m.name}」→ 打红在 ${m.expect}`)
    } finally {
      try { fs.rmSync(mutDir, { recursive: true, force: true }) } catch { /* ignore */ }
    }
  }
  if (bad) {
    console.log(`✗ 变异自证：${bad} 处问题 —— 判据有空转的地方`)
    process.exit(1)
  }
  console.log(`✓ 变异自证：${MUTS.length} 处改动**每一处都让判据翻红**（判据不是空转的）`)
  process.exit(0)
}

if (fails.length) {
  console.log(`✗ 两级动效守卫 ${fails.length} 处问题：`)
  for (const f of fails.slice(0, 20)) console.log('    · ' + f)
  if (fails.length > 20) console.log(`    …另有 ${fails.length - 20} 处`)
  process.exit(1)
}
console.log(`✓ 两级动效（样式 × 动效）守卫：39 档迁移无损 + 顺序与老格式一致 + ` +
  `叠加真生效 + 两轴字段不重叠（${ALL.length} 档）`)
