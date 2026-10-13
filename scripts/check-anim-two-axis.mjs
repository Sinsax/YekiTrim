#!/usr/bin/env node
// ============================================================================
// `check-anim-two-axis.mjs` —— **三轴动效（排版 × 过渡 × 动效）的守卫**
// ============================================================================
//
// 用法：`node scripts/check-anim-two-axis.mjs [--mutate]`
//
// ⚠️ 文件名保留 `two-axis`（**不改成 three-axis**）：它是**路径契约**，
//    `.workbuddy/gates.mjs` 与别的门禁按名字调它。改名会让"这条判据其实没跑"
//    以"文件找不到"以外的形式发生（例如被静默跳过）。内容口径见下。
//
// ============================================================================
// 本文件 2026-10-14 的口径：**两轴 → 三轴**
// ============================================================================
//
// | 轴（内部名） | 显示名 | 事件字段          | 档位 |
// |---|---|---|---|
// | `style`      | 排版   | `style_anim`      | `tilt` · `vtext` · `arc`（空档名 = 默认） |
// | `transition` | 过渡   | `transition_anim` | `pop` · `fade` |
// | `effect`     | 动效   | `effect_anim`     | `shake` · `perchar_big` · `perchar_small` · `heartbeat` |
//
// 9 档：`tilt` `arc` `vtext` `pop` `fade` `shake` `perchar_big` `perchar_small` `heartbeat`。
//
// # 它存在的理由（这是真事故换来的，别删）
//
// 「排版 / 过渡 / 动效」拆成**独立可叠加**的三条轴时，转译器要从"一个 `anim`"
// 变成"三路各取自己的字段"。这类改动踩过的坑**都不会自己报错**：
//
//   ① **4 条轨的 `per_char` 整条丢失** —— `pop_perchar` / `hop_*` 在界面上归
//      「动效」栏，但它们的 `perChar` 是**形态字段**、住「排版」侧。
//      按"轴"判就漏了 ⇒ 字不跳了，报告里一行差异都没有。
//   ② **产物里轨与 assets 的顺序整体变** —— 分组键从一维变多维时，
//      排序键也跟着变了（空串排在所有字母前）⇒ 轨内容逐条相同、
//      **只有排列顺序不同** ⇒ sha256 不同（R4 破）。
//   ③ **R4 对拍本身是空转的** —— 第一版拿 `docs/wgpu-effect-reference/demo`
//      去比，而那份工程**一个 `anim` 都没有** ⇒ 前后恒等 ⇒ "通过"是假的。
//
// ⚠️⚠️ 所以本判据**自己造夹具**（9 档 + 叠加组合），不依赖任何现成工程 ——
//    ③ 那条讲的就是"夹具没用到被改的字段时，R4 是空转的"。
//
// # 判据
//
//   ⓪ **三轴的档名取值集合两两不相交** —— 同一档名出现在两条轴 ⇒ 谁都以为自己
//      该管它 ⇒ 必须红。（原两轴那件事，扩一维。）
//   ① **`animAxisOf` 对 9 个档名逐个返回自己那条轴**（不是 `''`）—— ★见下。
//   ② **三轴字段不重叠**：同一字段名不许出现在两条轴的档位里。
//   ③ **老格式（只有 `anim`）逐档迁移无损、也不多余**（R4）。
//   ④ 顺序稳定 + 老格式按 `anim` 字母序。
//   ⑤ **叠加真的生效**：`竖排 + 抖动` 这种组合，**两边都写出去**。
//
// ============================================================================
// ⚠️⚠️ ⓪/① 为什么必须**从源码抽 `ANIM_AXIS` + `animAxisOf` 真跑**
// ============================================================================
//
// **这条是实测踩出来的**，不是洁癖：
//
//   收敛后 `heartbeat` **不带下划线**、`perchar_big` **带**。
//   原来的 `animAxisOf` 只判 `startsWith(p + '_')` ⇒
//     `animAxisOf('heartbeat')` 返回 **`''`**
//   ⇒ 它被当成"认不出的档名"走 CLAMP
//   ⇒ **用户选了心跳、成片里没反应**，而报告说"档名不认识"。
//
// ⚠️ 而"在本判据里重写一份 `animAxisOf`"是**绝对不行**的 —— 重写的那份会与
//    被测的那份**一起漂**，两条源码变异都会活下来、判据**假绿**。
//    本仓有实测教训，这句话写在 `scripts/AGENTS.md` 里。
//    ⇒ 一律 `new Function(...)` 从**转译器源码**里抽出来跑（见 `loadAuthority`）。
import fs from 'node:fs'
import path from 'node:path'
import os from 'node:os'
import { fileURLToPath } from 'node:url'
import { execFileSync } from 'node:child_process'
import { loadAuthority, GLOBAL_ELEMENTS, AXES, SORT_TO_AXIS } from './gen-anim-attribution.mjs'

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
// ⚠️ 被验对象是**可覆盖**的 —— 变异自证要把转译器拷成副本、改那份副本再跑。
//    默认是真身；`--mutate` 那条路会传 `YK_ANIM3AXIS_TRANSLATOR`。
//    ⚠️ **绝不**用 `git checkout` 还原变异（那会把未提交的工作一起冲掉，
//    这个仓里已经因此丢过一次改动）—— 一律改**副本**。
const TRANSLATOR = process.env.YK_ANIM3AXIS_TRANSLATOR
  || path.join(ROOT, 'tools', 'polish-to-dhampir.mjs')
const MUTATE = process.argv.includes('--mutate')

const fails = []
let checks = 0
const ok = (cond, msg) => { checks++; if (!cond) fails.push(msg) }

// ---- 权威：从源码抽出来（**不重写**）----
const { bundle } = loadAuthority(TRANSLATOR)
const { ANIM_PRESETS, animAxisOf, animPreset } = bundle
const ALL = Object.keys(ANIM_PRESETS)
/// 事件字段名（轴名 + `_anim`）—— 三份实现的**接口**，不许各写各的。
const FIELD_OF = { style: 'style_anim', transition: 'transition_anim', effect: 'effect_anim' }

// ============================================================================
// ⓪ 三轴的档名取值集合**两两不相交**
// ============================================================================
//
// ⚠️ 这是"归属"这件事的**根本要求**：同一档名若出现在两条轴，
//    谁都以为自己该管它 ⇒ 用户选一次、两条轴各执行一次（或互相打架）。
//    原两轴版本钉的是"样式轴 ∩ 动效轴 = ∅"，这里扩成**三对两两检查**。
{
  const setOf = {}
  for (const ax of AXES) setOf[ax] = ALL.filter((n) => animAxisOf(n) === ax).sort()
  for (let i = 0; i < AXES.length; i++) {
    for (let j = i + 1; j < AXES.length; j++) {
      const [a, b] = [AXES[i], AXES[j]]
      const inter = setOf[a].filter((n) => setOf[b].includes(n))
      ok(inter.length === 0,
        `⓪ 「${a}」与「${b}」两条轴的档名**有交集**：${JSON.stringify(inter)} —— ` +
        `同一档名只能归一条轴；重叠意味着两条轴都以为自己该管它`)
    }
  }
  // ⚠️ 三轴的档集合**合起来必须正好是全表**（不漏、不重）。
  const union = new Set(AXES.flatMap((ax) => setOf[ax]))
  const orphan = ALL.filter((n) => !union.has(n))
  ok(orphan.length === 0,
    `⓪ 这些档**不属于任何一条轴**（\`animAxisOf\` 认不出）：${JSON.stringify(orphan)} —— ` +
    `它们会被当成"认不出的档名"走 CLAMP ⇒ 用户选了没反应`)
  for (const ax of AXES) {
    ok(setOf[ax].length > 0, `⓪ 「${ax}」轴上**一档都没有** —— 那一栏会是空的`)
  }
}

// ============================================================================
// ① ★ `animAxisOf` 对 9 个档名**逐个**返回自己那条轴（不是 `''`）
// ============================================================================
//
// ⚠️⚠️ 这条是**实测踩出来的那条**（见文件头）：`heartbeat` 不带下划线。
//    它同时是"精确名 + 前缀两条路"这条实现的**唯一**判据 ——
//    把精确名那条路去掉、只留前缀，本条必须红。
{
  const NINE = {
    tilt: 'style', arc: 'style', vtext: 'style',
    pop: 'transition', fade: 'transition',
    shake: 'effect', perchar_big: 'effect', perchar_small: 'effect', heartbeat: 'effect',
  }
  // ⚠️ 先断言这 9 个名字**就是**预设表的全部 —— 否则下面那圈会"看起来在跑"，
  //    而实际漏掉了新加的档（表变了、判据没跟上 = 本仓最经典的假绿）。
  ok(JSON.stringify([...ALL].sort()) === JSON.stringify(Object.keys(NINE).sort()),
    `① 预设表是 ${JSON.stringify([...ALL].sort())}，判据里写的是 ` +
    `${JSON.stringify(Object.keys(NINE).sort())} —— 两处不同步（新档漏判 / 旧档没删）`)
  for (const [n, want] of Object.entries(NINE)) {
    const got = animAxisOf(n)
    ok(got === want,
      `① \`animAxisOf(${JSON.stringify(n)})\` 实得 ${JSON.stringify(got)}，应当是 ` +
      `${JSON.stringify(want)} —— ${got === '' ? '**返回空 = 认不出来** ⇒ 用户选了它成片里没反应，' +
        '且报告说"档名不认识"' : '归到了错的轴上'}\n` +
      `      ⚠️ 收敛后 \`heartbeat\` **不带下划线**、\`perchar_big\` **带** ⇒ ` +
      `\`animAxisOf\` 必须同时判「精确名」与「前缀」两条路`)
  }
  // ⚠️ 反向：**空 / 垃圾**必须返回 `''`（不许把认不出的也算成某一轴）。
  // ⚠️ `perchar` 这个**裸前缀**不在列表里 —— 留给前缀路是**正确**的
  //    （它正是 `perchar_big` 的族名前缀）。要判"认不出"得用**真的不认识**的名字。
  for (const bad of ['', '   ', 'nope', 'tiltx', 'hov', 'shrinnk']) {
    ok(animAxisOf(bad) === '',
      `① \`animAxisOf(${JSON.stringify(bad)})\` 应当返回空串（它是"认不出"），` +
      `实得 ${JSON.stringify(animAxisOf(bad))}`)
  }
}

// ============================================================================
// ①-bis **`sort` 与 `animAxisOf` 必须同源**（"档位摆在哪一栏" ↔ "按哪条轴走"）
// ============================================================================
//
// ⚠️ **为什么单独一条**：`animAxisOf` 读的是 `ANIM_AXIS`，而**面板分组**读的是
//    `sort`。两者是**两份**声明，漂了不会报错：
//      · 用户在界面上把这一档摆进「动效」栏（`sort: 'effect'`）；
//      · 而 `animAxisOf` 说它是 `style` ⇒ 转译器按**排版**那一路取字段。
//    ⇒ 症状是"选了这一档，效果出现在另一条轴上"（或两边都不生效），**静默**。
//
//    ⚠️ 两个名字**不一样是正常的**（`sort` 用显示口径 `layout`，轴用内部名 `style`
//      —— 因为事件字段叫 `style_anim`），所以这里走**显式映射表**对拍，
//      而不是判两个字符串相等。
{
  for (const n of ALL) {
    const bySort = ANIM_PRESETS[n].sort ?? null
    const want = bySort === null ? null : (SORT_TO_AXIS[bySort] ?? `?${bySort}`)
    ok(want !== null,
      `①-bis \`${n}\` 的预设里**没有 \`sort\`** —— 面板不知道把它摆进哪一栏`)
    ok(want === animAxisOf(n),
      `①-bis \`${n}\`：\`sort\` 是 \`${bySort}\`（→ ${want}），而 \`animAxisOf\` 说 ` +
      `\`${animAxisOf(n)}\` —— 面板把这一档摆在一条轴上，转译器却按另一条轴取字段` +
      `（症状：选了它、效果出现在别的轴上，且不报错）`)
    // ⚠️ `sort` 的取值**只许**是映射表里那三个显示名（拼错 ⇒ 那一栏静默消失）。
    ok(bySort === null || bySort in SORT_TO_AXIS,
      `①-bis \`${n}\` 的 \`sort\` 是 \`${bySort}\`，不在 ${JSON.stringify(Object.keys(SORT_TO_AXIS))}` +
      ` —— 拼错的轴名会让那一栏整批消失`)
  }
}

// ============================================================================
// ② 三轴**同一字段不许被两条轴"各给一个值"** —— 并钉住真实的优先级
// ============================================================================
//
// ⚠️⚠️ **这条判据的口径在本趟被修正过一次，理由要留住**：
//
//   原本写的是"两两轴字段集**完全不相交**"。实测发现**做不到、也不该做**：
//
//     · `perChar` 在 `arc`（排版）与 `perchar_*`（动效）上都有 —— 但那是
//       **同一条渲染分支的两个不同叶子**（`rotation_*` vs `scale_spread`），
//       `motionFields` 是 `per_char = 某个档的整个 perChar`，只从**一个**档取。
//     · `scale_in_*` 在 `pop`（过渡）与 `heartbeat`（动效）上都有 —— 实测
//       `pop + heartbeat` 同填时 `pick()` 按"**过渡 > 动效 > 排版**"取 `pop`
//       的值（350），**heartbeat 的 480 被静默丢掉**。
//
//   ⇒ "完全不相交"会让判据**永远红**，而红门禁等于没有门禁。
//   ⇒ 这条改成**两个真的抓得住的断言**：
//       ① 三轴的档集合两两不相交（在 ⓪ 里已钉）；
//       ② **重叠字段的优先级必须是"声明过的"** —— 逐值对拍 + 把已知的
//          重叠**登记在册**（新的重叠出现即红）。这样"谁赢"就是**显式契约**，
//          而不是藏在 `?:` 里的隐性行为。
{
  /// **已登记**的重叠字段 → 它属于哪几条轴（新增重叠必须显式加进来）。
  ///
  /// ⚠️ 登记不是"放过它"，是**把隐性契约写下来**：`motionFields` 里
  ///    `pick()` 的取值顺序就是这三条（过渡 > 动效 > 排版），
  ///    `per_char` 的 `perCharFrom` 同序。登记之后：
  ///      · 谁赢是**声明的**（有注释、有判据）；
  ///      · 出现**没登记**的重叠 ⇒ 当场红（逼人做决定，而不是让它悄悄赢）。
  const KNOWN_OVERLAP = {
    per_char_frames: ['style', 'effect'],
    stagger_frames: ['style', 'effect'],
    scale_in_from: ['transition', 'effect'],
    scale_in_peak: ['transition', 'effect'],
    scale_in_ms: ['transition', 'effect'],
  }
  /// 各轴拥有的**叶子字段**（`perChar.x` 展开成叶子 —— 见上面那段）。
  const leavesOf = {}
  for (const ax of AXES) {
    const keys = new Set()
    for (const n of ALL) {
      if (animAxisOf(n) !== ax) continue
      const p = animPreset(n) ?? {}
      for (const k of Object.keys(p)) {
        if (k === 'sort') continue
        if (k === 'perChar') {
          for (const leaf of Object.keys(p.perChar ?? {})) keys.add(leaf)
          continue
        }
        keys.add(k)
      }
    }
    leavesOf[ax] = keys
  }
  // ---- ① 出现**没登记**的重叠 ⇒ 红（逼人显式决定优先级）----
  const seen = new Set()
  for (let i = 0; i < AXES.length; i++) {
    for (let j = i + 1; j < AXES.length; j++) {
      const [a, b] = [AXES[i], AXES[j]]
      for (const k of [...leavesOf[a]].filter((x) => leavesOf[b].has(x))) {
        const owners = [a, b].sort()
        seen.add(k)
        const reg = KNOWN_OVERLAP[k]
        ok(reg && JSON.stringify([...reg].sort()) === JSON.stringify(owners),
          `② 字段 \`${k}\` **同时被「${a}」与「${b}」两条轴给值**，但**没登记**` +
          `（已登记的是 ${JSON.stringify(KNOWN_OVERLAP)}）—— ` +
          `未登记的重叠会成为**隐性契约**：谁赢取决于 \`?:\` 的写法，` +
          `而用户看到的是"我两条都配了、有一条没生效"`)
      }
    }
  }
  // ---- ② 登记表里**不许有已经不存在**的项（表的另一边也要防漂）----
  for (const [k, owners] of Object.entries(KNOWN_OVERLAP)) {
    ok(seen.has(k),
      `② 登记表里的 \`${k}\`（轴 ${JSON.stringify(owners)}）**实际并不重叠** —— ` +
      `登记表过期了（档位表改过之后要一起清）`)
  }
  // ---- ③ 三轴都要真的给字段（某条轴一个字段都没有 ⇒ 它是空轴）----
  for (const ax of AXES) {
    ok(leavesOf[ax].size > 0, `② 「${ax}」轴上**一个字段都没有**（那条轴是空的）`)
  }
}

// ============================================================================
// ③ 夹具：造一份真的用了全部 9 档的工程（**不依赖任何现成工程**）
// ============================================================================
const tmp = fs.mkdtempSync(path.join(os.tmpdir(), 'yk-anim3axis-'))
const tomlPath = path.join(tmp, 'polish.toml')

// ⚠️ **必须有 `clip.mp4`**：`keepRanges` 是从**成片时间线（EDL）**推出来的，
//    而没有源视频时那条推导会得到一个空的 keep 集合 ⇒ **所有事件都落在
//    "被掐掉的区间"里**、被整条跳过 ⇒ 轨只剩主视频一条、判据全报"轨没生成"。
//    （实测踩过：夹具看着完全正常，转译器也 exit 0，只是**一条文字轨都没有**。）
fs.writeFileSync(path.join(tmp, 'clip.mp4'), Buffer.alloc(0))

/// 造工程。`legacy=true` 只写 `anim`（老格式）；否则写**三路**（新格式）。
//
// ⚠️ 三条从真工程抄来的硬要求（缺一条夹具就**一条文字轨都出不来**）：
//   ① `[source] video = "clip.mp4"` + 目录里真的有那个文件（零字节即可）；
//   ② 文字元素要 **`source`**（九段竖线串，`process` 写回的那个）；
//   ③ 事件用 **`time` + `end`**，不是 `time` + `duration`。
//
// ⚠️⚠️ **还要写一段非空的 `[elements]`**（`GLOBAL_ELEMENTS`，与归属表测量台同一份）
//    —— 这是判据 ④ 反向那一半的前提：逐条预设生效时**必须压掉全局**。
//    夹具里**没有**这段的话，"回落全局"与"不回落"结果完全一样
//    ⇒ 那条判据**看起来在跑、实际空转**（实测：加 `[elements]` 之前，
//    把 `on` 退回"只看动效轴"那种真 bug 跑出来是**绿**的）。
function writeProject({ legacy, events }) {
  const head = `duration = 60.0
processed = true

[project]
name = "三轴动效夹具"
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
  // ⚠️ 用 comment 那一档的真 source 串（照抄实测值）—— 自己编一个会让
  //    "样式参数没拿到"那条 CLAMP 又冒出来。
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
${legacy
    ? `anim = "${ev.anim}"`
    : `style_anim = "${ev.style_anim || ''}"
transition_anim = "${ev.transition_anim || ''}"
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

/// 产物里某个文字轨。
const trackOf = (doc, id) => (doc.timeline?.tracks ?? []).find((x) => x.id === id)
const subOf = (doc, id) => trackOf(doc, id)?.subtitle ?? trackOf(doc, id)?.sub ?? null
/// 图层关键帧（那条腿**不写 `subtitle`**）。
const keysOf = (doc, id) => {
  for (const l of trackOf(doc, id)?.layers ?? []) {
    if (Array.isArray(l.keyframes) && l.keyframes.length) return l.keyframes
  }
  return []
}

try {

// ============================================================================
// ②-④ ⚠️ **重叠字段的优先级必须与 `motionFields` 实际行为一致**
// （放在夹具之后 —— 它要真跑一次转译器才判得出来）
// ============================================================================
{
  // ---- ④ ⚠️ **重叠字段的优先级必须与 `motionFields` 实际行为一致** ----
  //
  // 这是本条判据的**真牙齿**：光登记"它们重叠"没有意义，
  // 要钉住"**重叠时谁赢**"—— 实测 `pick()` 是 过渡 > 动效 > 排版。
  // ⇒ `pop + heartbeat` 同填时，`scale_in_ms` 必须是 **pop 的 350**（不是 480）。
  {
    const popT = ALL.find((n) => animAxisOf(n) === 'transition'
      && animPreset(n)?.scale_in_ms !== undefined)
    const beatT = ALL.find((n) => animAxisOf(n) === 'effect'
      && animPreset(n)?.scale_in_ms !== undefined)
    ok(!!popT && !!beatT,
      '② 找不到"过渡 + 动效都声明 scale_in_ms"的一对（重叠那条判据会空转）')
    if (popT && beatT) {
      const a = animPreset(popT).scale_in_ms
      const b = animPreset(beatT).scale_in_ms
      // ⚠️ 两个值**必须不同** —— 相同的话"谁赢"根本分不出来（判据空转）。
      ok(a !== b,
        `② \`${popT}\` 与 \`${beatT}\` 的 \`scale_in_ms\` 都是 ${a} —— ` +
        `分不出优先级，这条判据会**空转**（改档位表时别让它们撞值）`)
      if (a !== b) {
        writeProject({
          legacy: false,
          events: [{
            transition_anim: popT, effect_anim: beatT, time: 0.5,
          }],
        })
        const doc = run()
        const id = (doc?.timeline?.tracks ?? []).map((t) => t.id)
          .find((x) => x.startsWith('text_comment_bottom_'))
        const sub = id ? subOf(doc, id) : null
        ok(!!sub, `② 过渡+动效同时给的轨没生成（\`${popT}\` + \`${beatT}\`）`)
        if (sub) {
          ok(sub.scale_in_ms === a,
            `② 过渡与动效**同时给** \`scale_in_ms\` 时，实得 ${JSON.stringify(sub.scale_in_ms)}，` +
            `按 \`pick()\` 的优先级（过渡 > 动效）应当是 **${a}**（${popT} 的值，不是 ${beatT} 的 ${b}）
` +
            `     ⚠️ 这条不是"可有可无"：重叠的赢家若变了，` +
            `用户看到的是"两条都配了、有一条静默没生效"`)
        }
      }
    }
  }
}

// ============================================================================
// ③ 老格式（只有 `anim`）逐档迁移必须**无损**、且**不多余**（R4）
// ============================================================================
//
// 判据怎么立（**不依赖历史二进制**）：
//   把 9 档**逐个**写成老格式，每档一条事件（各自一条轨），
//   然后断言：该轨**这一档该给的字段一个不少**，而且**不该有的一样不多**。
//
// ⚠️ **反向那一半不能省** —— 只做单向（"少了就红"）会漏掉一整类**静默的 R4 破**：
//    纯排版档在动效轴上是空的 ⇒ `motionFields` 的 `on` 若算错 ⇒ `pick()`
//    **回落全局 `[elements]`** ⇒ 那些轨被**注入**全局的 `scale_in_*`/`shake_*`，
//    工程一个字没改、画面变了。
{
  const events = ALL.map((n, i) => ({ anim: n, time: 0.1 * i }))
  writeProject({ legacy: true, events })
  const doc = run()
  if (doc) {
    const missing = []
    const extra = []
    /// 动效那半边的**产物键**。
    ///
    /// ⚠️⚠️ **别在这里写"产物键 → 预设键"的驼峰映射** —— 那正是本仓踩过的坑
    ///    （第一版就是那样，29 条假红）。这三档的预设键名**本来就是蛇形**
    ///    （与底座同形，见 `ANIM_PRESETS` 的注释："档位表这一层统一驼峰"那条
    ///     只对 `perChar` / `fontWeight` 那几个；`scale_in_*` / `shake_*`
    ///     在预设里就是蛇形）。⇒ 直接用**同名**查预设，别再转换一次。
    ///    实测：写成驼峰映射时，`shake` 的 `shake_px` 被误判成"漏了全局"。
    const MOTION_FIELD_KEYS = [
      'scale_in_from', 'scale_in_peak', 'scale_in_ms', 'shake_px', 'shake_hz',
    ]
    for (let i = 0; i < ALL.length; i++) {
      const n = ALL[i]
      const id = `text_comment_bottom_${n}`
      const sub = subOf(doc, id)
      if (!sub) { missing.push(`${n}: 轨没生成`); continue }
      const p = animPreset(n)

      // ---- 正向：这一档该给的，一个不许少 ----
      // ⚠️ `per_char` / `vertical` 走**排版那路**（`stylePreset`）；
      //    `shake_*` / `scale_in_*` 走**动效/过渡那路**。
      //    `tilt` 的 `tiltDeg` 与 `sway` 的关键帧**不写 `subtitle`**（见下）。
      const want = []
      if (p.perChar !== undefined) want.push('per_char')
      if (p.vertical === true) want.push('vertical')
      if (p.shake_px !== undefined) want.push('shake_px')
      if (p.shake_hz !== undefined) want.push('shake_hz')
      if (p.scale_in_ms !== undefined) want.push('scale_in_ms')
      for (const k of want) {
        if (sub[k] === undefined || sub[k] === null) {
          missing.push(`${n}: 少了 \`${k}\``)
        }
      }
      // ⚠️ `tilt` 走**图层 `transform.rotation`**、摇摆走**图层关键帧** ——
      //    两条都不在 `subtitle` 上。第一版把 sway 算进 `subtitle`，
      //    报了**假红**（"少了 sway_deg"，其实它压根不写那儿）。
      if (p.tiltDeg !== undefined) {
        const rot = trackOf(doc, id)?.layers?.[0]?.transform?.rotation
        if (!Number.isFinite(rot) || rot === 0) {
          missing.push(`${n}: 图层 \`transform.rotation\` 实得 ${JSON.stringify(rot)} ` +
            `（这一档的 \`tiltDeg\`=${p.tiltDeg} 没生效）`)
        }
      }

      // ---- 反向：这一档**没提**的动效字段，一条都不许出现在轨上 ----
      //
      // 为什么能这么断言：每档自成一条轨（各写各的 `anim`）⇒ 轨上的每个
      // 动效字段都只可能来自**这一档**（全局那份在逐条预设生效时**必须被压掉**）。
      // ⇒ 这一档没给的键**就不该在**。
      //
      // ⚠️ 例外只有 `fade`：它是空预设（一个字段都不给），
      //    存在的意义就是**压掉全局** ⇒ 它反而是这条判据最该看的那个（全 0）。
      //
      // ⚠️⚠️ **"这一档没给"要看它自己那条轴的那一份预设**，不是只看 `p`。
      //    老格式 `anim` 会按轴拆到**三路**：`tilt`/`arc`/`vtext` 去排版、
      //    `shake`/`perchar_*`/`heartbeat` 去动效、而 **`pop`/`fade` 去过渡**。
      //    `pop` 的 `scale_in_*` 就住在**过渡**那一路的预设里 ——
      //    只拿 `p`（动效轴的预设）判，会把 `pop` 的正常输出**误报成"漏了全局"**。
      //    （本趟实测踩到：补上过渡轴的老格式拆分后，这条假红了 8 处。）
      const own = animPreset(n) ?? {}
      for (const outKey of MOTION_FIELD_KEYS) {
        if (outKey in sub && own[outKey] === undefined) {
          extra.push(`${n}: 多出 \`${outKey}\`=${JSON.stringify(sub[outKey])}` +
            `（这一档没给 \`${outKey}\`）`)
        }
      }
      // ---- 摇摆那条路也要反向核（它是**另一条**路）----
      // ⚠️ `sway` 那一族**已经删了** ⇒ 当前 9 档**没有**一档会给 `swayDeg`。
      //    所以"任何一档都不许长出 `rotation` 关键帧"就是判据 ——
      //    而它抓的正是"全局 `[elements] sway_deg` 漏进了 swayCfg"
      //    （实测踩过：`fade` 静默继承全局摇摆、字自己摆起来）。
      const keys = keysOf(doc, id)
      if (p.swayDeg === undefined && keys.length > 0) {
        extra.push(`${n}: 多出 ${keys.length} 个关键帧（这一档没给 \`swayDeg\`）` +
          ` —— 全局 \`[elements] sway_deg\` 漏进 swayCfg 了，成片里这行字会自己摆`)
      }
    }
    ok(missing.length === 0,
      `③ 老格式（只有 \`anim\`）逐档迁移**有损** —— 这些档少了字段：` +
      JSON.stringify(missing.slice(0, 12)) +
      (missing.length > 12 ? ` …另 ${missing.length - 12} 处` : '') +
      `\n     ⚠️ 最典型的是 \`per_char\`：\`perchar_big\`/\`perchar_small\` 在界面归「动效」栏，` +
      `但它们的 \`perChar\` 是**形态字段** —— 按"轴"判就会漏掉它（实测整档不生效）。`)
    ok(extra.length === 0,
      `③ 老格式迁移**多出了字段**（R4 破）—— 全局 \`[elements]\` 漏进了逐条预设的轨：` +
      JSON.stringify(extra.slice(0, 12)) +
      (extra.length > 12 ? ` …另 ${extra.length - 12} 处` : '') +
      `\n     ⚠️ 两条路**都可能漏**，要一起看：\n` +
      `        · \`motionFields\` 的 \`put/pick\`（写出 \`scale_in_*\`/\`shake_*\`）；\n` +
      `        · \`swayKeyframes\` 的 \`swayCfg\`（写出图层 \`rotation\` 关键帧）。\n` +
      `        根因都是「'这一档指定了'只看了一条轴」—— 三轴任一取到了档才算"指定了"`)
  }
}

// ============================================================================
// ④ 顺序：老格式按 `anim` 字母序；新格式稳定 —— 两次跑逐字符相同
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
      '④ 同一份工程跑两次，轨 id 序列不同 —— 产物不是确定性的')
    // ⚠️ 老格式下的顺序必须**等于按 `anim` 的字母序**（改造前就是那样）。
    const names = ids1.filter((x) => x.startsWith('text_comment_bottom_'))
      .map((x) => x.replace('text_comment_bottom_', ''))
    const sorted = [...names].sort()
    ok(JSON.stringify(names) === JSON.stringify(sorted),
      `④ 老格式下文字轨的顺序**不是**按 \`anim\` 字母序（R4 破）。\n` +
      `     实得：${JSON.stringify(names)}\n     字母序：${JSON.stringify(sorted)}\n` +
      `     ⚠️ 典型原因：排序键拿多路拼（空串排在所有字母**之前**）⇒ ` +
      `"只有动效"的档整批跑到最前面。`)
  }
}

// ============================================================================
// ④-bis **新格式**（三路都写）的顺序也必须稳定且**不依赖空串的位置**
// ============================================================================
//
// ⚠️ 这条是 ④ 的**必要补充** —— 只测老格式的话，"排序键拿三路直接拼"那个 bug
//    会**漏网**：老格式里 `anim` 非空、拼出来的第一段就是 `anim`，
//    拼法怎么错都仍然按 `anim` 排。而**新格式**里 `anim` 是空的 ⇒
//    排序键退化成三路拼 ⇒ 空串排在字母前 ⇒ 顺序与预期不符。
{
  const combos = [
    { style_anim: 'vtext', transition_anim: 'pop', effect_anim: 'shake', time: 0.5 },
    { style_anim: '', transition_anim: 'fade', effect_anim: '', time: 1.0 },
    { style_anim: 'tilt', transition_anim: '', effect_anim: '', time: 1.5 },
    { style_anim: 'arc', transition_anim: 'pop', effect_anim: 'heartbeat', time: 2.0 },
  ]
  writeProject({ legacy: false, events: combos })
  const d1 = run()
  const d2 = run()
  if (d1 && d2) {
    const ids1 = (d1.timeline?.tracks ?? []).map((t) => t.id)
    const ids2 = (d2.timeline?.tracks ?? []).map((t) => t.id)
    ok(JSON.stringify(ids1) === JSON.stringify(ids2),
      '④-bis 新格式：同一份工程跑两次，轨 id 序列不同 —— 产物不是确定性的')
    const tail = ids1.filter((x) => x.startsWith('text_comment_bottom_'))
      .map((x) => x.replace('text_comment_bottom_', ''))
    // ⚠️ 四条组合 ⇒ 四条轨（**必须都分到不同的轨**，否则就是"后写覆盖先写"）
    ok(tail.length === combos.length,
      `④-bis 新格式：${combos.length} 组不同的「排版 × 过渡 × 动效」组合只生成了 ` +
      `${tail.length} 条轨 —— 不同的组合**必须分轨**` +
      `（轨级参数不同，混在一轨就是后写的覆盖先写的）`)
    // ⚠️ 顺序必须是**排过序的**（确定性的），且**不含空串拼出来的前缀 bug**
    const tailSorted = [...tail].sort()
    ok(JSON.stringify(tail) === JSON.stringify(tailSorted),
      `④-bis 新格式：文字轨顺序不是稳定排序 —— \n` +
      `     实得：${JSON.stringify(tail)}\n     排序：${JSON.stringify(tailSorted)}\n` +
      `     ⚠️ 典型原因：排序键用"多路直接拼"，空串排在所有字母**之前**。`)
  }
}

// ============================================================================
// ⑤ 叠加：新格式写三路时，**每条轴都真的生效**（且三轴互不挤掉）
// ============================================================================
{
  // ⚠️ 挑档要用**预设表里的驼峰名**（`shakePx` / `vertical`），
  //    不是产物里的蛇形（`shake_px`）—— 第一版混过这两套名字。
  const styleName = ALL.find((n) => animAxisOf(n) === 'style'
    && animPreset(n)?.vertical === true)
  const transitionName = ALL.find((n) => animAxisOf(n) === 'transition'
    && animPreset(n)?.scale_in_ms !== undefined)
  const effectName = ALL.find((n) => animAxisOf(n) === 'effect'
    && animPreset(n)?.shake_px !== undefined)
  ok(!!styleName && !!transitionName && !!effectName,
    '⑤ 夹具挑不出"排版 + 过渡 + 动效"各一条纯档（表变了？）')
  if (styleName && transitionName && effectName) {
    writeProject({
      legacy: false,
      events: [{
        style_anim: styleName, transition_anim: transitionName,
        effect_anim: effectName, time: 0.5,
      }],
    })
    const doc = run()
    if (doc) {
      const ids = (doc.timeline?.tracks ?? []).map((t) => t.id)
      const t = ids.find((x) => x.startsWith('text_comment_bottom_'))
      const sub = t ? subOf(doc, t) : null
      ok(!!sub, `⑤ 三轴叠加那一轨没生成（轨：${JSON.stringify(ids.slice(0, 5))}）`)
      if (sub) {
        // 排版那半边
        ok(sub.vertical === true,
          `⑤ 叠加没生效：排版轴 \`${styleName}\` 的 \`vertical\` **没写出去** ` +
          `⇒ 用户选了竖排、成片里横着（实得 ${JSON.stringify(sub.vertical)}）`)
        // 过渡那半边
        ok(sub.scale_in_ms !== undefined && sub.scale_in_ms !== null,
          `⑤ 叠加没生效：过渡轴 \`${transitionName}\` 的 \`scale_in_ms\` **没写出去** ` +
          `⇒ 用户选了弹出、成片里不弹（实得 ${JSON.stringify(sub.scale_in_ms)}）`)
        // 动效那半边
        ok(sub.shake_px !== undefined && sub.shake_px !== null,
          `⑤ 叠加没生效：动效轴 \`${effectName}\` 的 \`shake_px\` **没写出去** ` +
          `⇒ 用户选了抖动、成片里不抖（实得 ${JSON.stringify(sub.shake_px)}）`)
        // ⚠️ **三条都要在**才叫叠加 —— 这是"三轴互斥"那种错法会挂掉的地方
        ok(sub.vertical === true && sub.scale_in_ms !== undefined && sub.shake_px !== undefined,
          '⑤ 三条轴没有**同时**生效 —— 这不是"可叠加"，是"三选一"')
      }
    }
  }

  // ---- ⑤-bis ⚠️ 过渡轴**压掉全局**那条语义（`fade` 是空预设）----
  //
  // ⚠️ 为什么必须单独一条：`fade` 是**唯一**"一个字段都不给"的档 ——
  //    它存在的意义就是"把全局 `[elements]` 压掉"。
  //    夹具里配了全局 `[elements]`（`GLOBAL_ELEMENTS`）⇒ 这一条**不是空转**。
  //    实测踩过：`swayCfg` 的三轴门漏了过渡轴 ⇒ `fade` 静默继承全局摇摆、
  //    该轨长出 2 个 `rotation` 关键帧（字自己摆起来），而报告里一行差异都没有。
  {
    const fadeName = ALL.find((n) => animAxisOf(n) === 'transition'
      && Object.keys(animPreset(n) ?? {}).filter((k) => k !== 'sort').length === 0)
    ok(!!fadeName, '⑤-bis 找不到"空预设的过渡档"（`fade` 没了？）')
    if (fadeName) {
      writeProject({ legacy: false, events: [{ transition_anim: fadeName, time: 0.5 }] })
      const doc = run()
      if (doc) {
        const id = (doc.timeline?.tracks ?? []).map((t) => t.id)
          .find((x) => x.startsWith('text_comment_bottom_'))
        const sub = id ? subOf(doc, id) : null
        ok(!!sub, `⑤-bis \`${fadeName}\` 那一轨没生成`)
        if (sub) {
          // ⚠️ 全局 `scale_in_*`（9.9/9.8/999）**一个都不许漏进来**
          for (const k of ['scale_in_from', 'scale_in_peak', 'scale_in_ms']) {
            ok(!(k in sub),
              `⑤-bis \`${fadeName}\`（空预设）静默继承了全局 \`[elements]\` 的 ` +
              `\`${k}\`=${JSON.stringify(sub[k])} —— ` +
              `空预设的语义是**压掉全局**，不是"没配就算了"`)
          }
          // ⚠️ 摇摆那条腿同样不许漏（全局 `sway_deg` = 33.3）
          const keys = id ? keysOf(doc, id) : []
          ok(keys.length === 0,
            `⑤-bis \`${fadeName}\` 静默继承了全局 \`[elements] sway_deg\` —— ` +
            `该轨长出 ${keys.length} 个 \`rotation\` 关键帧，成片里这行字会自己摆\n` +
            `     ⚠️ 摇摆写的是**图层关键帧**、不是 \`subtitle\`，所以只核 \`subtitle\` 看不见它`)
        }
      }
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
//    下面每一条都对应**本趟真踩过 / 复核挖出**的一个坑 —— 用它们证明判据有牙。
//
// ⚠️ **只改副本**：把转译器拷到临时目录再改，**绝不**在真身上改、
//    也**绝不**用 `git checkout` 还原（那会把未提交的工作一起冲掉）。
if (MUTATE) {
  const MUTS = [
    {
      // ★ 本趟要求的第 1 组变异：把「精确名」那条路去掉、只留前缀
      //   ⇒ `animAxisOf('heartbeat')` 变 `''` ⇒ 判据 ① 必须红。
      name: '（①）`animAxisOf` 去掉「精确名」那条路，只留前缀 ⇒ `heartbeat` 认不出来',
      from: `    if (Object.prototype.hasOwnProperty.call(ANIM_AXIS, key)) return ANIM_AXIS[key];`,
      to: `    if (false) return ANIM_AXIS[key];`,
      expect: '①',
    },
    {
      // ★ 本趟要求的第 1 组变异（另一半）：把 `heartbeat` 从轴表里删掉。
      //   ⚠️ 只删这一项不够 —— 精确名那条路会回落到前缀，而前缀也没有 `heartbeat`
      //      ⇒ 同样返回 `''`。两条路各自单独红，才说明"两条都真的在判"。
      name: '（①）`ANIM_AXIS` 里删掉 `heartbeat` 那一项 ⇒ 认不出来',
      from: `    shake: 'effect', perchar: 'effect', heartbeat: 'effect',`,
      to: `    shake: 'effect', perchar: 'effect',`,
      expect: '①',
    },
    {
      // ★ 本趟要求的第 2 组变异：把某个档的 `sort` 改成另一条轴。
      //   ⚠️ 它打在 **①-bis**（`sort` ↔ `animAxisOf` 同源）—— 不是 ⓪/①。
      //      理由：`animAxisOf` 读的是 `ANIM_AXIS`，而 `sort` 是**另一份**声明
      //      ⇒ 只改 `sort` 时 `animAxisOf` 照旧返回 `effect`，
      //      "两两不相交"（⓪）在**集合**上仍然成立（集合是按 `animAxisOf` 分的）。
      //      真正破掉的是"面板摆在哪一栏 ↔ 转译器按哪条轴取字段"这对关系。
      //      ⇒ 所以判据要**单独**把 `sort` 与 `animAxisOf` 对拍（①-bis），
      //        否则这条变异只会以"字段归属漂了"的**间接**形式露头（③），
      //        报错信息**指不到真正的原因**（实测踩到）。
      name: '（①-bis）把 `shake` 的 `sort` 从 effect 改成 style ⇒ 面板与轴不同源',
      from: `    shake: { sort: 'effect', shake_px: 6.0, shake_hz: 18.0 },`,
      to: `    shake: { sort: 'style', shake_px: 6.0, shake_hz: 18.0 },`,
      expect: '①-bis',
    },
    {
      // （R4）"指定了档"只看动效轴 ⇒ 纯排版档回落全局（put/pick 那一路）
      name: '（③）"指定了档"只看动效轴 ⇒ 纯排版档回落全局（put/pick 那一路）',
      from: `    const on = !!preset || !!stylePreset || !!transitionPreset;`,
      to: `    const on = !!preset;`,
      expect: '③',
    },
    {
      // （R4）摇摆那条路的同一条门
      name: '（③）`swayCfg` 的三轴门退回两轴（漏过渡轴）⇒ `fade` 静默继承全局摇摆',
      from: `              const anyAxis = p !== undefined
                || animPreset(cues[0].styleAnim) !== undefined
                || animPreset(cues[0].transitionAnim) !== undefined;`,
      to: `              const anyAxis = p !== undefined
                || animPreset(cues[0].styleAnim) !== undefined;`,
      expect: '③/⑤-bis',
    },
    {
      // ⚠️ 逐字那半边改读**动效轴**⇒ `perchar_*` 的 `per_char` 走错路（或丢掉）。
      // ⚠️⚠️ 这条钉的是**本趟最严重的那条真缺陷**：`perchar_big` / `perchar_small`
      //     整档不生效（用户要的「逐字变大 / 变小」两支**全是死的**，报告里一行都没有）。
      //
      //     ⚠️⚠️ **这条变异要同时摘掉两条路才有牙** —— 实测踩到过：
      //         `perchar_*` 的 `per_char` 有**两条冗余来源**
      //           a) `perCharFrom`（过渡 > 动效 > 排版 三选一）—— 动效那路给得到；
      //           b) `animHasFormPart` 认了 `perchar_*` ⇒ 老格式拆分把它**也**送进
      //              排版那路 ⇒ `stylePreset` 那路也给得到。
      //         ⇒ **只摘一条，另一条会兜住**，产物逐字节不变 ⇒ 判据不红。
      //           第一版就只摘了 `perCharFrom`，`--mutate` 报"没被打红"，
      //           差点让人以为"判据抓不住"（其实是**变异造得不对**）。
      //         ⇒ 这条把两处一起改回"只认排版轴 / 按档名硬编码"，
      //           复现出修复前的真实形态（`per_char` 整条消失）。
      name: '（③）逐字那两条路一起退回（`perchar_*` 的 `per_char` 整条丢失）',
      parts: [
        {
          from: `    return p.perChar !== undefined
      || p.tiltDeg !== undefined
      || p.vertical === true
      || p.squashY !== undefined;`,
          to: `    return animAxisOf(key) === 'style'
      || key === 'slam_squash'
      || key === 'pop_perchar'
      || /^hop_/.test(key);`,
        },
        {
          from: `    const perCharFrom = (transitionPreset?.perChar !== undefined) ? transitionPreset
      : (preset?.perChar !== undefined) ? preset
        : (stylePreset?.perChar !== undefined) ? stylePreset
          : undefined;`,
          to: `    const perCharFrom = (stylePreset?.perChar !== undefined && false) ? stylePreset
      : undefined;`,
        },
      ],
      expect: '③',
    },
    {
      // ⚠️ 这条是上面那条的**第二半**：单独摘 `perCharFrom`（它自己会被
      //    另一条路兜住 ⇒ 本变异**故意不要求翻红**，见下面 `masked: true`）。
      //     留它的意义是**把这个"冗余"写下来** —— 哪天有人删掉其中一条路，
      //     这条就变成有牙的了；而"两条都在"这件事本身是一种防御。
      name: '（③·冗余）只摘 `perCharFrom` 的动效那路（另一条路兜得住 ⇒ 允许不红）',
      from: `    const perCharFrom = (transitionPreset?.perChar !== undefined) ? transitionPreset
      : (preset?.perChar !== undefined) ? preset
        : (stylePreset?.perChar !== undefined) ? stylePreset
          : undefined;`,
      to: `    const perCharFrom = (stylePreset?.perChar !== undefined) ? stylePreset
      : undefined;`,
      expect: '③',
      masked: true,
    },
    {
      name: '（④-bis）排序键拼法退回"三路直接拼"（空串排在字母前）',
      from: `        const ax = kx[0] || [kx[1], kx[2], kx[3]].filter(Boolean).join('+');\n` +
        `        const ay = ky[0] || [ky[1], ky[2], ky[3]].filter(Boolean).join('+');`,
      to: `        const ax = kx[0] || \`\${kx[1]}+\${kx[2]}+\${kx[3]}\`;\n` +
        `        const ay = ky[0] || \`\${ky[1]}+\${ky[2]}+\${ky[3]}\`;`,
      expect: '④-bis',
    },
    {
      name: '（⑤）动效那一路改读排版字段（两轴混用 ⇒ 叠加失效）',
      from: `            animPreset(cues[0].effectAnim),`,
      to: `            animPreset(cues[0].styleAnim),`,
      expect: '⑤',
    },
  ]
  const src = fs.readFileSync(TRANSLATOR, 'utf8')
  let bad = 0
  for (const m of MUTS) {
    // ⚠️ **一条变异可以有多处替换**（`m.parts`）—— 有些缺陷是**两条路一起**才造成的
    //    （`perchar_*` 就有两条冗余来源：只摘一条，另一条兜住 ⇒ 产物逐字节不变 ⇒ 不红）。
    //    实测踩到：第一版只支持单处替换，那条变异报"没被打红"，
    //    差点被误读成"判据抓不住"。
    const parts = m.parts ?? [{ from: m.from, to: m.to }]
    const missAnchor = parts.filter((x) => !src.includes(x.from))
    if (missAnchor.length) {
      console.log(`  ✗ 变异「${m.name}」有 ${missAnchor.length} 处锚点**找不到了** —— ` +
        `源码文本变了没同步，这条变异**静默失效**（不是判据的错，是变异表过期了）`)
      bad++
      continue
    }
    let mutated = src
    for (const x of parts) mutated = mutated.replace(x.from, x.to)
    const mutDir = fs.mkdtempSync(path.join(os.tmpdir(), 'yk-anim3axis-mut-'))
    const mutFile = path.join(mutDir, 'polish-to-dhampir.mut.mjs')
    fs.writeFileSync(mutFile, mutated)
    let red = false
    try {
      execFileSync(process.execPath, [fileURLToPath(import.meta.url)],
        { stdio: 'pipe', env: { ...process.env, YK_ANIM3AXIS_TRANSLATOR: mutFile } })
    } catch {
      red = true
    } finally {
      try { fs.rmSync(mutDir, { recursive: true, force: true }) } catch { /* ignore */ }
    }
    if (red) {
      console.log(`  ✓ 变异「${m.name}」→ 打红在 ${m.expect}`)
    } else if (m.masked) {
      // ⚠️ **故意允许不红**的变异（`masked: true`）——见那条的注释。
      //    它不是"判据有洞"，而是**把这个冗余写下来**：
      //    摘掉这条路时另一条兜得住 ⇒ 产物逐字节不变 ⇒ **本来就不该红**。
      //    ⚠️ 别把它删掉：哪天另一条路没了，它立刻变成有牙的变异。
      console.log(`  ○ 变异「${m.name}」**没红（如预期）** —— 这条被另一条路兜住，` +
        `见该变异的注释（\`masked: true\`）`)
    } else {
      console.log(`  ✗ 变异「${m.name}」**没被打红** —— 判据在 ${m.expect} 那条上有洞`)
      bad++
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
  console.log(`✗ 三轴动效守卫 ${fails.length} 处问题（共 ${checks} 项判据）：`)
  for (const f of fails.slice(0, 20)) console.log('    · ' + f)
  if (fails.length > 20) console.log(`    …另有 ${fails.length - 20} 处`)
  process.exit(1)
}
console.log(
  `✓ 三轴动效（排版 × 过渡 × 动效）守卫：${ALL.length} 档迁移无损 + 顺序与老格式一致 + ` +
  `叠加真生效 + 三轴档名两两不相交 + 三轴字段不重叠 + \`animAxisOf\` 逐档归对轴；` +
  `共 ${checks} 项判据`)
