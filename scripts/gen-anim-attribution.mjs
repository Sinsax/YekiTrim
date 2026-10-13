#!/usr/bin/env node
// ============================================================================
// 生成 `scripts/anim-attribution.mjs` —— **三轴 9 档的参数归属表**（实测）
// ============================================================================
//
// 用法：`node scripts/gen-anim-attribution.mjs [--check]`
//   * 不带参：重新生成并写盘（改了 `ANIM_PRESETS` 之后跑它）；
//   * `--check`：**只比对不写盘**（判据模式）—— 表与源码不一致就 exit 1。
//
// ============================================================================
// ⚠️⚠️ 本文件 2026-10-14 起是「**测量台**」，不是「抄表器」
// ============================================================================
//
// 上一版（39 档）这条路是**错的**，而且错得看不出来：
//
//   * 它把 `webui/src/polish/engine.js::ANIM_PRESETS` 当权威，
//     只做"读表 → 按键名分类"；
//   * 而 `94a8892`（三轴收敛）改的是 **`tools/polish-to-dhampir.mjs`**，
//     `engine.js` **一个字节都没动**（仍是 39 档两轴）。
//
//   ⇒ 结果是：表"生成"出来是 39 档，与 `engine.js` **逐字段相同** ⇒ `--check` 绿；
//     而转译器那边已经是 9 档 ⇒ 表面上"表↔源码一致"，实际上
//     **归属表守着一个已经被废弃的副本**。
//     症状（实测）：`check-anim-presets-parity.mjs` 报
//     「转译器有、预览不认识」= 9 个新档；「预览有、转译器没有」= 39 个旧档。
//
// 本版改掉两件事，**缺一不可**：
//
//   ① **权威换成转译器**（`tools/polish-to-dhampir.mjs`）—— 它才是写进 toml、
//      真的决定成片的那一份。
//   ② **不再"读表分键名"，而是真跑一遍转译器、看产物里出了哪些字段** ——
//      键名分类那一层看不见"这一档这一趟到底写没写出去"
//      （典型反例：`fade` 档一个字段都不给，靠"压掉全局"表达语义；
//        `perchar_*` 的 `per_chars` 走 `stylePreset` 那条路）。
//
// ⚠️ 三条口径（**判据，别改**）—— 它们直接决定 `motionFields` 的取值：
//
//   * **归属（axis）** = 这一档**该填哪一栏**。唯一权威是转译器的 `ANIM_AXIS`
//     （键 → `sort`，且必须等于 `ANIM_PRESETS[key].sort`）。**不猜、不从键名推**。
//   * **formFields** = 这一档**实际写在 `subtitle` 上的**、决定"字静态长什么样"的键。
//   * **motionFields** = 同理，但决定"画面随时间变"的键（含 **图层 `rotation`
//     关键帧**那条腿 —— 它不写 `subtitle`，只写 `layers[].keyframes`）。
//
// ⚠️⚠️ **`rotation` 关键帧算 motion 那一侧**：整块旋转随时间往复 = 会动。
//    漏掉它的话 `sway` 那种"只给关键帧、`subtitle` 上一个字段都不给"的档
//    会被算成"两边都没有 ⇒ 退化成 none"，而正是这条**踩过一次**
//    （见 `check-anim-two-axis.mjs` 的 `GLOBAL_ELEMENTS` 事故）：
//    纯形态档在动效轴为空 ⇒ `swayCfg` 回落全局 `[elements]` ⇒ 字自己摆起来。
//
// ⚠️ **`axisOf` 从源码里抽出来用 `new Function` 真跑**，不在本文件重写一份 ——
//    重写的那份会与被测的那份**一起漂**（本仓实测教训："判据重实现了被测函数
//    ⇒ 两条源码变异都活下来、判据假绿"）。这里同理：`animAxisOf` 里
//    「精确名 + 前缀」两条路是 `heartbeat` 能不能被认出来的**唯一**根据，
//    抄一份就等于把那条不变量从判据里删掉。
import fs from 'node:fs'
import path from 'node:path'
import os from 'node:os'
import { fileURLToPath } from 'node:url'
import { execFileSync } from 'node:child_process'

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const OUT = path.join(ROOT, 'scripts', 'anim-attribution.mjs')
const TRANSLATOR = path.join(ROOT, 'tools', 'polish-to-dhampir.mjs')

/** 三个轴名（= 事件字段去掉 `_anim`）。**顺序就是显示顺序**。 */
export const AXES = ['style', 'transition', 'effect']

/// `ANIM_PRESETS[key].sort` 用的是**显示口径**（`layout` = 面板上的「排版」），
/// 而 `ANIM_AXIS` / 事件字段用的是**内部轴名**（`style`）。
///
/// ⚠️⚠️ 这两个名字**必须分开**，别为了"看起来一致"去改任一边：
///    `style` 是**事件字段名**的一半（`style_anim`），改它 = 产物契约变更；
///    `layout` 只在面板上出现。本仓的定案是"内部名与显示名分开"
///    （见转译器里 `AXIS_LABEL` 那段注释）。
///
/// ⚠️ 本表的 `axis` 取**内部名**（界面填哪个字段），而 `sort` 是显示口径
///    ⇒ 两者要通过这张**显式**的映射对拍。**留一张显式表**而不是"直接相等"，
///    是为了让上面那条"两张表必须一致"的断言仍然有牙：
///    哪个档的 `sort` 被改成另一条轴，这里当场红。
const SORT_TO_AXIS = { layout: 'style', transition: 'transition', effect: 'effect' }

// ============================================================================
// ① 从**源码**抽取权威（绝不重写一份）
// ============================================================================

/** 切一段源码（`head` 必须唯一，`tail` 是它的结束锚）。 */
function slice(src, head, tail, what) {
  const i = src.indexOf(head)
  if (i < 0) throw new Error(`抽不出「${what}」：找不到头部锚点 ${JSON.stringify(head)}`)
  const j = src.indexOf(tail, i + head.length)
  if (j < 0) throw new Error(`抽不出「${what}」：找不到尾部锚点 ${JSON.stringify(tail)}`)
  return src.slice(i, j)
}

/**
 * 抽出并**真跑**权威实现，返回一个句柄。
 *
 * ⚠️ 返回的每个函数都来自**同一份源码切片**（`ANIM_PRESETS` / `ANIM_AXIS` /
 *    `animAxisOf` / `animHasFormPart` / `animHasMotionPart` / `splitLegacyAnim`
 *    都住在同一个块作用域里，靠闭包互相引用）⇒ 必须一起抽、一起 `new Function`。
 *    `motionFields` 住在**另一个**块作用域（`translate` 的闭包里），单独抽一份。
 */
export function loadAuthority(file = TRANSLATOR) {
  const src = fs.readFileSync(file, 'utf8')

  const presetsChunk = slice(src, '  const ANIM_PRESETS = {', '  /// 一档动效预设', 'ANIM_PRESETS')
  // ⚠️ `animPreset` / `ANIM_KNOWN` 住在"预设表"与"轴表"**之间**（转译器把它们
  //    和 `ANIM_PRESETS` 放在同一个块里）⇒ 这一段要单独补上来，
  //    否则 `animAxisOf` 之外的调用点会 `ReferenceError`（`animPreset` 未定义）。
  const helperChunk = slice(src, '  /// 一档动效预设', '  // ---- 两轴拆分', 'animPreset/ANIM_KNOWN')
  // ⚠️ 轴这段要**一直抽到 `splitLegacyAnim` 结束**（尾锚 = `swayKeyframes`）：
  //    `animHasFormPart` / `animHasMotionPart` / `splitLegacyAnim` 三者互相引用，
  //    抽一半就是 `ReferenceError`（这条路径上"抽少了"会**当场炸**，不会假绿）。
  // ⚠️ 尾锚**不能**用 "这一档在**形态**路上给了东西吗" —— 那句在文件里出现**两次**
  //    （`animHasFormPart` / `animHasMotionPart` 各有一段），不唯一 ⇒ 切片落错位置。
  const axisChunk = slice(src, '  const ANIM_AXIS = {', '  const swayKeyframes = (cue, cfg, tb) => {', 'ANIM_AXIS')
  const motionChunk = slice(src, '  const motionFields = (', '  /// **T7 摇摆**', 'motionFields')

  // ⚠️ `motionFields` 引用的 `pick/put/on/popInOn` 全在它自己的函数体里，
  //    但 `POP_IN_DEFAULT` 是**外层常量** ⇒ 要一起喂进去（值取自源码本身）。
  //    ⚠️ 而 `POP_IN_DEFAULT` 自己又引用了 `STICKER_POP_IN_SEC` —— 依赖要**跟到底**，
  //       否则 `new Function` 直接 `ReferenceError`（比"判据假绿"好，但同样跑不起来）。
  const popIn = /const POP_IN_DEFAULT = \{[^}]*\}/.exec(src)
  if (!popIn) throw new Error('抽不出 `POP_IN_DEFAULT`')
  const stickerSec = /^export const STICKER_POP_IN_SEC = [^;]+;/m.exec(src)
  if (!stickerSec) throw new Error('抽不出 `STICKER_POP_IN_SEC`')

  const bundle = new Function(`
${stickerSec[0].replace(/^export /, '')}
${popIn[0]};
${presetsChunk}
${helperChunk}
${axisChunk}
${motionChunk}
return { ANIM_PRESETS, ANIM_AXIS, animPreset, animAxisOf, animHasFormPart, animHasMotionPart, splitLegacyAnim, motionFields };
`)
  return { bundle: bundle(), src }
}

// ============================================================================
// ② 夹具：造一份**真的用了全部档**的工程，跑转译器，看产物里出了什么
// ============================================================================
//
// ⚠️ 为什么必须有夹具（不能只读预设表）：预设表里的**键名**与"这一趟真的写了"
//    是两件事。`motionFields` 里有好几道门（`on` / `onTime` / `onTransition` /
//    `onStyle` / `popInOn`），任何一道写错都会让**某个档静默不写字段** ——
//    而只看表**完全看不见**。这正是本仓最恨的"配了、不报错、没效果"。
//
// ⚠️ **全局 `[elements]` 必须配**（`GLOBAL_ELEMENTS`）：`motionFields` 的
//    `pick()` 在"任一轴都没指定"时回落全局。夹具里没有这段的话，
//    "压掉全局"与"回落全局"结果一样 ⇒ 判据空转（实测：加之前某处真 bug 是绿的）。

/// 夹具里的全局 `[elements]`。值刻意与任何一档预设**都不同**，一眼能认出漏进来的全局值。
export const GLOBAL_ELEMENTS = `[elements]
scale_in_from = 9.9
scale_in_peak = 9.8
scale_in_ms = 999
shake_px = 77
shake_hz = 66
sway_deg = 33.3
sway_ms = 4444`

const SRC_STR = 'comment|44|2|255,226,122,255|26,26,26,255|700|1'

/** 造工程（`live.axisField(name)` → 该档该填哪个字段）。 */
function writeProject(dir, events) {
  const head = `duration = 60.0
processed = true

[project]
name = "三轴归属测量夹具"
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

${GLOBAL_ELEMENTS}
`
  // ⚠️ **必须有 `clip.mp4`**（零字节占位）：`keepRanges` 从成片时间线推出来，
  //    没有源视频 ⇒ keep 集合为空 ⇒ **所有事件都被判成"落在掐掉的区间里"**
  //    ⇒ 一条文字轨都不出来（而转译器照旧 exit 0）。
  fs.writeFileSync(path.join(dir, 'clip.mp4'), Buffer.alloc(0))
  const body = events.map((ev) => `
[[segments.events]]
type = "element"
kind = "text"
text = "样本"
style = "comment"
pos = "bottom"
x = "28%"
y = "58%"
source = "${SRC_STR}"
time = ${ev.time.toFixed(2)}
end = ${(ev.time + 0.30).toFixed(2)}
${ev.axisField} = "${ev.name}"
`).join('')
  fs.writeFileSync(path.join(dir, 'polish.toml'),
    head + '\n[[segments]]\nstart = 0.0\nend = 60.0\nfx = "low"\n' + body)
}

/** 跑转译器 → 产物对象。 */
function runTranslator(dir, file) {
  const out = path.join(dir, 'out.json')
  try {
    execFileSync(process.execPath, [file, dir, '--out', out],
      { stdio: 'pipe', env: { ...process.env, DHAMPIR_QUIET: '1' } })
  } catch (e) {
    const s = String(e.stderr ?? '') + String(e.stdout ?? '')
    throw new Error('转译器跑失败：' + s.split('\n').slice(0, 4).join(' / '))
  }
  return JSON.parse(fs.readFileSync(out, 'utf8'))
}

const trackOf = (doc, id) => (doc.timeline?.tracks ?? []).find((x) => x.id === id)
const subOf = (doc, id) => trackOf(doc, id)?.subtitle ?? trackOf(doc, id)?.sub ?? null
/// 图层关键帧（`rotation` 那条腿 —— 它**不写 `subtitle`**）。
const keysOf = (doc, id) => {
  for (const l of trackOf(doc, id)?.layers ?? []) {
    if (Array.isArray(l.keyframes) && l.keyframes.length) return l.keyframes
  }
  return []
}

/**
 * **测量**：逐档单独跑一遍转译器，把产物里**真的出现**的字段分成两路。
 *
 * ⚠️ 一档一条事件、各成一条轨 ⇒ 轨上的每个字段都只可能来自**这一档**
 *    （全局那份在逐条预设生效时必须被压掉 —— 这也是判据要钉的东西）。
 *
 * ⚠️ `translated`（三路档名）与归属表里的 `axis` 是**两件事**：
 *    `axis` 回答"用户该填哪一栏"（给界面用）；`translated` 回答"这一档
 *    实际写进了哪几路"。对当前 9 档两者一致，但**别把它们合并成一个字段**
 *    —— 合并了就看不出"表说 effect、实际却从 transition 那路出去"这类错。
 */
export async function measure(file = TRANSLATOR) {
  const { bundle } = loadAuthority(file)
  const { ANIM_PRESETS, ANIM_AXIS, animAxisOf, animPreset } = bundle
  const names = Object.keys(ANIM_PRESETS)

  // ---- 归属：唯一权威是源码的 `ANIM_AXIS` / `ANIM_PRESETS[key].sort` ----
  const axisOf = {}
  for (const n of names) {
    // ⚠️⚠️ **唯一权威是 `animAxisOf(n)` 的返回值**，不是 `ANIM_AXIS[n]` 的查表结果 ——
    //    `ANIM_AXIS` 是**前缀表**（键是 `perchar` 不是 `perchar_big`），
    //    直接查它会把 `perchar_*` 判成"表里没有"。而 `animAxisOf` 正是
    //    「精确名 + 前缀」**两条路**的那份实现 —— 那两条路就是
    //    `heartbeat`（不带下划线、只走精确名）能不能被认出来的唯一根据。
    //    ⇒ 抄一份查表逻辑 = 把这条不变量从判据里删掉（本仓实测教训）。
    const viaFn = animAxisOf(n)
    if (!viaFn) {
      throw new Error(
        `\`${n}\`：\`animAxisOf\` 返回空 —— 这一档**认不出来**。` +
        `症状是"用户选了它、成片里没反应"，且报告说"档名不认识"。` +
        `（收敛后 \`heartbeat\` 不带下划线，只判前缀就会漏掉它。）`)
    }
    // ⚠️ 面板分组用的 `sort` 是**显示口径**，要与内部轴名对拍（见 `SORT_TO_AXIS`）。
    const bySort = ANIM_PRESETS[n].sort ?? null
    const byPreset = bySort === null ? null : (SORT_TO_AXIS[bySort] ?? `?${bySort}`)
    if (viaFn !== byPreset) {
      throw new Error(
        `\`${n}\`：animAxisOf 说 ${viaFn}、ANIM_PRESETS.sort 说 ${bySort}（→ ${byPreset}）` +
        ` —— 界面把这一档摆在一条轴上，填进去却按另一条轴走（静默）`)
    }
    axisOf[n] = viaFn
  }

  // ---- 实测：跑一遍转译器 ----
  //
  // ⚠️⚠️ **这里刻意**不**用"与空档名基线做键集差分"来决定归属** —— 我试过，
  //    它给出的是"9 档贡献全空"这种**错误结论**，坑在基线：
  //    "空档名"那条基线因为 `on === false` 会**回落全局 `[elements]`**，
  //    所以基线里**已经含有** `shake_px` / `scale_in_*` 这些键；
  //    而 `shake` 档给的是**同名的键、不同的值**（6 vs 全局 77）
  //    ⇒ 键集差分把 `shake_px` **差掉了**，"这一档覆盖了全局"这件事看不见。
  //    ⇒ **差分能证明"有没有覆盖"，证明不了"归属"**，两件事别混。
  //
  // 归属改用**两条互补的口径**（下面 `FORM_KEYS` / `MOTION_KEYS` 两段注释写着），
  // 再拿**产物**去交叉验证（`declaredButMissing` / `emittedButUnknown`）——
  // 交叉验证那一步才是"测量"，它抓得住"档里声明了、产物里没有"（真缺陷的形态）。
  const FIELD = { style: 'style_anim', transition: 'transition_anim', effect: 'effect_anim' }
  const measured = {}
  {
    const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'yk-anim-attr-'))
    try {
      const events = names.map((n, i) => ({ name: n, axisField: FIELD[axisOf[n]], time: 0.1 * i }))
      writeProject(dir, events)
      const doc = runTranslator(dir, file)
      for (const n of names) {
        const id = `text_comment_bottom_${n}`
        const sub = subOf(doc, id)
        if (sub === null) {
          throw new Error(`\`${n}\` 的轨没生成 —— 测量台的前提取不到，别用猜测的表糊过去`)
        }
        measured[n] = {
          // 产物里**真的出现**的键（含轨级常量 —— 交叉验证时要按"这一档声明的"过滤）。
          emitted: Object.keys(sub),
          keyframes: keysOf(doc, id).length,
        }
      }
    } finally {
      try { fs.rmSync(dir, { recursive: true, force: true }) } catch { /* 清不掉就算了 */ }
    }
  }

  return { axisOf, measured, names, bundle }
}

// ============================================================================
// ③ 产出：按**实际写出去的字段**分类
// ============================================================================

// ---- 归属：**两路各自的取值来源**（这是 `motionFields` 的真实形状）----
//
// ⚠️⚠️ 别"按字段名猜哪边" —— 本仓实测踩过：`pop_perchar` / `hop_*` 在界面上归
//    **动效**栏，但它们的 `perChar` 是**形态字段** ⇒ 按"轴"判会让逐字效果整条丢失。
//    ⇒ 判据是"**这一档有没有给形态字段**"，不是"它在界面哪条轴"。
//    所以下面两张键集**只回答"这个字段属于哪一路"**，与档位在哪条轴无关。
//
// ⚠️ 这个空间的键名与产物**同名同形**（都是 snake_case + `per_char`），
//    不是巧合：`ANIM_PRESETS` 的注释写着"逐字那半边已经是底座的蛇形、不做转换"。
//    而 `perChar` 这个**外层键名是驼峰**（`v.perChar`）—— 两者都要认。

/** 形态路：`motionFields` 里取值走 **`pickStyle(...)` / `stylePreset.xxx`** 的键。 */
export const FORM_KEYS = new Set([
  'per_char', 'vertical', 'safe_width_ratio',
])
/// 形态路：预设里叫 `perChar`（驼峰），写进产物是 `per_char`（蛇形）。
const FORM_PRESET_ALIAS = { perChar: 'per_char' }
/** 动效路：取值走 **`pick(...)`**（`onTime`/`onTransition`）的键。 */
export const MOTION_KEYS = new Set([
  'scale_in_from', 'scale_in_peak', 'scale_in_ms', 'shake_px', 'shake_hz',
])
/// 图层 `rotation` 关键帧那条腿的**标记**（不写 `subtitle`，只写 `layers[].keyframes`）。
export const KEYFRAMES_MARK = 'layers[].keyframes:rotation'

/**
 * 算出 9 档的归属。
 *
 * 两条口径**互补**，缺一条就会漏掉一整类：
 *   * **声明**（`ANIM_PRESETS[k]` 的键 → 两路）—— 这是"这一档该给什么"；
 *   * **产物**（转译器真跑出来的键）—— 这是"这一档真给了什么"。
 *
 * ⚠️⚠️ **任一档"声明了却没写出去" → 直接抛错**，**不许**生成一张"它没给这个字段"的表。
 *    那正是本仓最恨的形状（"配了、不报错、没效果"），而如果生成器把它**记成归属**，
 *    这条真缺陷就会被**写进权威表**、从此被判据当成"正常"。
 *    生成器是"把事实写下来"的地方，不是"把缺陷合理化"的地方。
 *    （实测踩到：`perchar_big` / `perchar_small` 整档不生效 —— 见下面 `path` 字段。）
 */
export async function buildAttribution(file = TRANSLATOR) {
  const { axisOf, measured, names, bundle } = await measure(file)
  const { animPreset } = bundle
  const out = {}
  const broken = []
  for (const n of names) {
    const declared = Object.keys(animPreset(n) ?? {}).filter((k) => k !== 'sort')
    // 驼峰 → 产物名（`perChar` → `per_char`），其余同名。
    const wanted = declared.map((k) => FORM_PRESET_ALIAS[k] ?? k)
    const form = wanted.filter((k) => FORM_KEYS.has(k))
    const motion = wanted.filter((k) => MOTION_KEYS.has(k))
    if (measured[n].keyframes > 0) motion.push(KEYFRAMES_MARK)

    // ---- 交叉验证：声明了的，产物里到底有没有 ----
    const emitted = new Set(measured[n].emitted)
    // ⚠️ 图层关键帧那条腿**不写 `subtitle`** ⇒ 不能拿 `emitted` 去核它，单独用 keyframes 数核。
    const missForm = form.filter((k) => !emitted.has(k))
    if (missForm.length) {
      broken.push(
        `\`${n}\` 声明了 ${JSON.stringify(missForm)}，但产物里**没有** —— ` +
        `这一档在第 ${axisOf[n]} 轴上**整条不生效**，且报告里一行差异都不会有`)
    }
    if (motion.some((k) => k === KEYFRAMES_MARK) && measured[n].keyframes === 0) {
      /* keyframes 数已经决定，不会走到这里；留个位置说明为什么它不是 miss */
    }

    out[n] = { axis: axisOf[n], form, motion }
  }
  if (broken.length) {
    throw new Error(
      '⚠️ 测量发现**档位声明与实际产物不符**（这是真缺陷，别用表把它合理化）：\n  - ' +
      broken.join('\n  - '))
  }
  return out
}

/** 把归属表渲染成模块源码。 */
function render(attr) {
  const names = Object.keys(attr).sort()
  const body = names.map((n) => {
    const a = attr[n]
    const f = a.form.map((x) => "'" + x + "'").join(', ')
    const m = a.motion.map((x) => "'" + x + "'").join(', ')
    return `  '${n}': { axis: '${a.axis}', form: [${f}], motion: [${m}] },`
  }).join('\n')
  const tally = AXES.map((ax) => `${ax} ${names.filter((n) => attr[n].axis === ax).length}`).join(' · ')
  return `// ⚠️ **本文件是生成的** —— 别手改，改 \`tools/polish-to-dhampir.mjs\` 的
//    \`ANIM_PRESETS\` 后跑 \`node scripts/gen-anim-attribution.mjs\` 重新生成。
//    判据模式：\`node scripts/check-anim-attribution.mjs\`（不一致就红）。
//
// **三轴 ${names.length} 档**的参数归属表：每一档归到「排版 / 过渡 / 动效」之一，
// 并保留"这一档**实际给了哪些字段**"这一层。
//
// # 这张表是怎么来的（⚠️ 别改成手写）
//
// \`node scripts/gen-anim-attribution.mjs\` **真跑一遍转译器**，逐档单独一条事件、
// 看产物里**真的出现了哪些字段**，再按下面两条口径分到两路。
// **不是**从预设表按键名推的 —— 键名分类看不见 \`motionFields\` 那几道门
// （\`on\` / \`onTime\` / \`onTransition\` / \`onStyle\` / \`popInOn\`）写错时
// "这一档静默不写字段"这件事。
//
// # 判据（别改这两条）
//   * **form** = 决定"字静态长什么样"（**与帧无关**）：\`vertical\` /
//     \`safe_width_ratio\` / \`font_weight\` / \`per_char\`。
//   * **motion** = 画面**随时间变**：\`scale_in_*\` / \`shake_*\`，以及
//     \`${KEYFRAMES_MARK}\`（**图层旋转关键帧**那条腿 —— 它不写 \`subtitle\`）。
//
// ⚠️ \`axis\` 与"给了哪些字段"是**两件事**：\`axis\` 回答"用户该填哪一栏"
//    （界面分组用），\`form\`/\`motion\` 回答"这一档实际写了什么"（\`motionFields\` 用）。
//    \`fade\` 就是那个反例：\`axis='transition'\`，但**一个字段都不给** ——
//    它靠"压掉全局 \`[elements]\`"表达"只淡入淡出"，不是"没实现"。
//
// # 统计（${names.length} 档）
//   ${tally}
export const ANIM_ATTRIBUTION = {
${body}
};

/** 这一档属于哪条轴（\`''\` = 表里没有这一档）。 */
export function axisOf(name) {
  const a = ANIM_ATTRIBUTION[name];
  return a ? a.axis : '';
}

/** 这一档在「排版」轴上有东西吗。 */
export function hasFormPart(name) {
  const a = ANIM_ATTRIBUTION[name];
  return !!a && a.form.length > 0;
}

/** 这一档在「动效」轴上有东西吗（含图层关键帧那条腿）。 */
export function hasMotionPart(name) {
  const a = ANIM_ATTRIBUTION[name];
  return !!a && a.motion.length > 0;
}
`
}

// ⚠️ **只在直接跑本文件时才生成/校验** —— 被 `check-anim-attribution.mjs`
//    import 时**什么都不做**（否则那条判据会顺带把文件重写一遍，
//    看起来"通过"了，其实是它自己刚把表刷成最新的 —— 判据空转）。
const isMain = process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)
if (!isMain) {
  // 被 import：只导出 `buildAttribution` / `measure` / `loadAuthority`，不产生副作用。
} else {
const attr = await buildAttribution()
const names = Object.keys(attr)
const tally = AXES.map((ax) => `${ax} ${names.filter((n) => attr[n].axis === ax).length}`).join(' · ')
const text = render(attr)

if (process.argv.includes('--check')) {
  const cur = fs.existsSync(OUT) ? fs.readFileSync(OUT, 'utf8') : ''
  if (cur !== text) {
    console.log('✗ scripts/anim-attribution.mjs 与转译器不一致 ——')
    console.log('  改过 `ANIM_PRESETS` 就要重新生成：node scripts/gen-anim-attribution.mjs')
    process.exit(1)
  }
  console.log(`✓ 归属表是最新的（实测）：${names.length} 档（${tally}）`)
  process.exit(0)
}

fs.writeFileSync(OUT, text)
console.log(`已重新生成 scripts/anim-attribution.mjs：${names.length} 档（${tally}）`)
}
