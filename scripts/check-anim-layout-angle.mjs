#!/usr/bin/env node
// 排版角度区间**三处逐值一致**（2026-10-14，三轴收敛引入 `layout_angle_deg` 之后）。
//
// # 为什么需要这条守卫
//
// 排版轴里 `tilt`（倾斜）与 `arc`（弧形/扇形）的**角度是可调的**，而它被
// **3 条路径**各自读一遍：
//
//   ① `tools/polish-to-dhampir.mjs` 的 `LAYOUT_ANGLE_MIN/MAX`（**权威**：夹取就在这里）
//   ② `webui/src/components/TimelineEditor.vue` 的同名常量（滑杆的 `min`/`max`）
//   ③ `webui/src/polish/engine.js` 的同名常量（预览求值器）
//
// ⚠️⚠️ **不一致的症状是双向的，且两边都不报错**：
//   * 前端区间比后端**宽**（前端 -45..45 / 后端 -30..30）⇒ 用户能拉到 45，
//     面板显示 45、滑杆停在 45，而**成片被夹到 30** —— 他看到的和自己设的不一样，
//     并且转译器只在**报告**里写一行 CLAMP（用户不看报告）。
//   * 前端比后端**窄** ⇒ 用户根本调不到后端允许的值（功能被静默削掉一半）。
//
// 这正是 `check-camera-zoom-parity.mjs` 那一类（"N 处各读一遍、各看都正常"），
// 所以照它的先例做：**逐值对拍 + 变异自证**。
//
// ⚠️⚠️ 还有一个**只有本条能抓**的洞：`webui/src/components/TimelineEditor.vue`
//    的注释里写着"两端同值由 `scripts/check-anim-layout-angle.mjs` 钉着" ——
//    而**那个文件曾经不存在**（悬空引用：注释承诺了一条判据，判据没有）。
//    ⇒ 注释里承诺的判据名，必须真的存在（本条自己就钉这一条）。
//
// # 用法
//
//   node scripts/check-anim-layout-angle.mjs            # 校验
//   node scripts/check-anim-layout-angle.mjs --mutate   # 变异自证（每条判据都要能翻面）

import { readFileSync, existsSync } from 'node:fs'
import { fileURLToPath } from 'node:url'
import { dirname, join } from 'node:path'

const here = dirname(fileURLToPath(import.meta.url))
const root = join(here, '..')
const read = (rel) => readFileSync(join(root, rel), 'utf8')

const TRANSLATOR = 'tools/polish-to-dhampir.mjs'
const PANEL = 'webui/src/components/TimelineEditor.vue'
const ENGINE = 'webui/src/polish/engine.js'

let failures = 0
let checks = 0
const check = (ok, detail) => {
  checks += 1
  if (!ok) {
    failures += 1
    console.log(`  FAIL ${detail}`)
  }
}

/** 从任意源码里抽一个「名字 → 数值」的常量声明（`const X = -30` / `X = -30`）。 */
function grabConst(src, name) {
  // ⚠️ 允许 `export const` / `const` / `.vue` 里的裸 `const`。
  //    ⚠️ 必须有**词边界**：`LAYOUT_ANGLE_MIN` 会命中 `LAYOUT_ANGLE_MIN_ALT` 之类
  //       （本仓真的有过这种"前缀更长"的坑），所以 `\b` 与行尾断言都要有。
  const m = src.match(new RegExp(`(?:export\\s+)?const\\s+${name}\\s*=\\s*(-?[0-9.]+)\\s*;?\\s*$`, 'm'))
  return m ? Number(m[1]) : null
}

// ── 抽取三处 ────────────────────────────────────────────────────────
const transSrc = read(TRANSLATOR)
const panelSrc = read(PANEL)
const engineSrc = read(ENGINE)

const places = [
  ['转译器（权威）', TRANSLATOR, transSrc],
  ['面板滑杆', PANEL, panelSrc],
  ['预览求值器', ENGINE, engineSrc]
]

const got = {}
for (const [label, rel, src] of places) {
  const min = grabConst(src, 'LAYOUT_ANGLE_MIN')
  const max = grabConst(src, 'LAYOUT_ANGLE_MAX')
  got[label] = { min, max }
  // ① 三处都必须**真的有**这对常量（少一处 = 那条路径没有区间概念）。
  check(min !== null, `${label}（\`${rel}\`）里抽不到 \`LAYOUT_ANGLE_MIN\` —— 那一路没有角度区间`)
  check(max !== null, `${label}（\`${rel}\`）里抽不到 \`LAYOUT_ANGLE_MAX\` —— 那一路没有角度区间`)
}

// ② 三处**逐值一致**。
const withMin = Object.entries(got).filter(([, v]) => v.min !== null)
const minVals = [...new Set(withMin.map(([, v]) => v.min))]
check(minVals.length === 1,
  `\`LAYOUT_ANGLE_MIN\` 三处不同值：${JSON.stringify(got)}\n` +
  `     ⚠️ 前端比后端宽 ⇒ 用户能拉到区间外的值、成片被夹回来，而他看到的和自己设的不一样；\n` +
  `        前端比后端窄 ⇒ 后端允许的值用户根本调不到。两种都不报错。`)
const withMax = Object.entries(got).filter(([, v]) => v.max !== null)
const maxVals = [...new Set(withMax.map(([, v]) => v.max))]
check(maxVals.length === 1,
  `\`LAYOUT_ANGLE_MAX\` 三处不同值：${JSON.stringify(got)}\n` +
  `     ⚠️ 症状同上（双向、且都不报错）。`)

// ③ 区间必须**跨 0 且对称**（倾斜/弧形的角度是有方向的：正负都合法）。
if (withMin.length && withMax.length) {
  const lo = minVals[0]
  const hi = maxVals[0]
  check(lo < 0 && hi > 0,
    `角度区间 [${lo}, ${hi}] 没有跨 0 —— 倾斜/弧形的角度是有方向的（正负都要能调）`)
  check(Math.abs(lo) === Math.abs(hi),
    `角度区间 [${lo}, ${hi}] 不对称 —— 一个方向比另一个方向能调得更多（本仓口径是对称的）`)
  // ⚠️ `0` 必须是**合法值**（"我就要正的"）：区间必须**闭**含 0。
  check(lo <= 0 && hi >= 0, `角度区间 [${lo}, ${hi}] 不含 0 —— 但 0 是合法角度（用户可能就要正的）`)
}

// ④ ★ **注释里承诺的判据必须真的存在**（本条的由来）。
//
//    ⚠️⚠️ `TimelineEditor.vue` 的注释写着"两端同值由 `scripts/check-anim-layout-angle.mjs`
//       钉着"，而**那个文件曾经不存在** —— 注释许诺了一条判据、判据不在，
//       下一个人读到会以为这里有人守。⇒ 凡是提到"由某判据钉着"的地方，
//       那个判据文件必须在盘上。
{
  const promised = [...panelSrc.matchAll(/scripts\/(check-[a-z0-9-]+\.mjs)/g)].map((m) => m[1])
  const uniq = [...new Set(promised)]
  check(uniq.length > 0,
    `\`${PANEL}\` 的注释里没有提到任何 \`scripts/check-*.mjs\` —— ` +
    `角度区间的"谁来守"这件事在面板上不可见（本判据的存在意义就是它）`)
  for (const name of uniq) {
    check(existsSync(join(root, 'scripts', name)),
      `\`${PANEL}\` 的注释承诺"由 \`scripts/${name}\` 钉着"，但**那个文件不存在**\n` +
      `     ⚠️ 悬空引用：注释替你说"有人守"，而没有任何机器判据 —— ` +
      `两端哪天漂了不会有任何东西红。`)
  }
  // ⚠️ 本判据自己也要被点名（否则它可以被改名/删掉而注释还指着旧名）。
  check(uniq.includes('check-anim-layout-angle.mjs'),
    `\`${PANEL}\` 的注释没有点名本判据（\`check-anim-layout-angle.mjs\`）—— ` +
    `角度区间的守卫从注释里看不出是谁`)
}

// ⑤ 转译器必须**真的夹取**（常量存在 ≠ 被用上）。
{
  check(/LAYOUT_ANGLE_MAX/.test(transSrc) && /Math\.max\(LAYOUT_ANGLE_MIN/.test(transSrc),
    `转译器定义了 \`LAYOUT_ANGLE_MIN/MAX\` 但**没用它们夹取** —— ` +
    `用户能从面板拖到区间外的值，而成片不夹（或夹在别处）\n` +
    `     ⚠️ "常量在、链路没接"是本仓反复犯的那一类（per_char 就是这么丢的）`)
}

// ⑥ 夹取必须**报出来**（用户不看报告，但 CI 与排障看）。
{
  check(/\[\$\{LAYOUT_ANGLE_MIN\}, \$\{LAYOUT_ANGLE_MAX\}\]/.test(transSrc) ||
    /CLAMP/.test(transSrc.slice(transSrc.indexOf('layoutAngle'))),
    `转译器夹了角度却**没在报告里点名** —— 用户看到的角度与自己设的不一样，而没有任何提示\n` +
    `     ⚠️ 报告字符串是接口产物（见 \`tools/AGENTS.md\`），夹取必须留痕`)
}

// ── 结果 ────────────────────────────────────────────────────────────
if (failures) {
  console.log(`\n✗ 排版角度区间：${failures} 条不通过（共 ${checks} 项）`)
  process.exit(1)
}
const lo = minVals[0]
const hi = maxVals[0]
console.log(`✓ 排版角度区间三处同值（${checks} 项：转译器 / 面板滑杆 / 预览求值器 ` +
  `都 = [${lo}, ${hi}]；对称跨 0；转译器真夹取且留痕；注释承诺的判据都在盘上）`)

// ── 变异自证 ────────────────────────────────────────────────────────
// ⚠️ 每条判据都要能翻面 —— **不红的判据不算判据**。
if (process.argv.includes('--mutate')) {
  const MUTATIONS = [
    {
      name: '面板滑杆区间放宽到 ±45 ⇒ 用户能拉到成片不认的值',
      file: PANEL,
      from: 'const LAYOUT_ANGLE_MAX = 30',
      to: 'const LAYOUT_ANGLE_MAX = 45'
    },
    {
      name: '预览求值器区间收窄 ⇒ 预览与成片在边界上不一致',
      file: ENGINE,
      from: 'export const LAYOUT_ANGLE_MIN = -30',
      to: 'export const LAYOUT_ANGLE_MIN = -20'
    },
    {
      name: '转译器不再夹取（常量还在、链路断开）',
      file: TRANSLATOR,
      from: 'LAYOUT_ANGLE_MAX, Math.max(LAYOUT_ANGLE_MIN, Number(rawAngle)),',
      to: 'Number(rawAngle),'
    },
    {
      name: '区间不对称（只许往一个方向调）',
      file: TRANSLATOR,
      from: 'export const LAYOUT_ANGLE_MAX = 30;',
      to: 'export const LAYOUT_ANGLE_MAX = 20;'
    },
    {
      name: '注释指向一个不存在的判据（悬空引用回归）',
      file: PANEL,
      // ⚠️ 三处提及**一起**改名 —— 悬空引用的真实形态就是"注释全指着同一个不存在的名字"，
      //    只改一处不算复现（那样还有两处指着真判据）。
      from: 'check-anim-layout-angle.mjs',
      to: 'check-nope-does-not-exist.mjs',
      all: true
    }
  ]
  const { writeFileSync, copyFileSync, unlinkSync } = await import('node:fs')
  let bad = 0
  const { spawnSync } = await import('node:child_process')
  for (const m of MUTATIONS) {
    const p = join(root, m.file)
    const bak = `${p}.mutbak`
    const src = readFileSync(p, 'utf8')
    const hits = src.split(m.from).length - 1
    if (hits === 0 || (!m.all && hits !== 1)) {
      console.log(`  ✗ 变异台自己错了：锚点在 ${m.file} 里命中 ${hits} 次` +
        `${m.all ? '（至少要 1 次）' : '（必须是 1）'}`)
      bad += 1
      continue
    }
    copyFileSync(p, bak)
    writeFileSync(p, m.all ? src.split(m.from).join(m.to) : src.replace(m.from, m.to))
    const r = spawnSync(process.execPath, [fileURLToPath(import.meta.url)], { encoding: 'utf8' })
    copyFileSync(bak, p)
    unlinkSync(bak)
    if (r.status === 0) {
      console.log(`  ✗ 变异没被打红：${m.name}`)
      bad += 1
    } else {
      console.log(`  ✓ ${m.name}`)
    }
  }
  if (bad) {
    console.log(`\n${bad} 处变异没被打红 —— 这份守卫有洞。`)
    process.exit(1)
  }
  console.log(`✓ 变异自证：${MUTATIONS.length} 处改动每一处都让判据翻面`)
}
