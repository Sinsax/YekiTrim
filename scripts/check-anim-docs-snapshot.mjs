#!/usr/bin/env node
/**
 * **`docs/anim/anim-kinds.json` 必须与真源码一致**（2026-10-14）。
 *
 * # 为什么需要这条判据
 *
 * 那份快照原来是**手抄** `TimelineEditor.vue::ANIM_KINDS` 的。
 * 三轴收敛把档位从 **39 收到 9**、把 `style/effect/both` 换成
 * `style/transition/effect`，**而快照没人改** ⇒ `docs/anim/` 那份 HTML
 * 一直画着**已经不存在的 39 档**。
 *
 * ⚠️ 当时**没有任何判据会红** —— 它只是个文档，而"文档漂了"在本仓
 * 属于第 ④ 类老病（"两条路径，只修了一条"）的**静默版本**：
 * 没有报错、没有日志、只有一个人照着它做判断时才会发现问题。
 *
 * 同一次检查还发现更糟的：快照里的 `preset` **从来就没有 `perChar`**
 * ⇒ 页面上"逐字那一路"**一直是空跑的**（生成时会打印"逐字路径: 0"）。
 *
 * # 它查什么
 *
 *   ① `gen-kinds.mjs` 是**可执行**的（从真源码抽，不是手抄）；
 *   ② 现在重跑它，产物与入库的**逐字节相同**（漂了就说明没重跑）；
 *   ③ 档位集合与 `ANIM_PRESETS` 的键**一致**（少一档 = 页面少一张卡）；
 *   ④ 轴归属是三条轴之一（`style` / `transition` / `effect`），
 *      **不许再出现 `both`** —— 那个类别已经被删掉了；
 *   ⑤ ★ 逐字那一路**真的有内容**（`preset.perChar` 存在），
 *      否则页面上的逐字演示是空转的。
 */
import fs from 'node:fs'
import path from 'node:path'
import os from 'node:os'
import { execFileSync } from 'node:child_process'
import { fileURLToPath, pathToFileURL } from 'node:url'

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const SNAP = path.join(ROOT, 'docs/anim/anim-kinds.json')
const GEN = path.join(ROOT, 'docs/anim/gen-kinds.mjs')

const fails = []
let checks = 0
const ok = (name, pass, detail = '') => {
  checks += 1
  if (pass) console.log(`  ok   ${name}`)
  else {
    console.log(`  FAIL ${name}${detail ? `\n         ${detail}` : ''}`)
    fails.push(name)
  }
}

console.log('docs/anim 快照与真源码一致')
console.log('='.repeat(72))

// ---- ① 生成器存在且能跑 ----
ok('`docs/anim/gen-kinds.mjs` 存在（快照必须是生成的，不是手抄的）',
  fs.existsSync(GEN))
if (!fs.existsSync(GEN)) {
  console.log('\n✗ 没有生成器 ⇒ 快照只能靠人记得手改，必然再漂')
  process.exit(1)
}

// ---- ② 重跑生成器，逐字节比对 ----
const committed = fs.readFileSync(SNAP, 'utf8')
let regenerated = null
try {
  execFileSync(process.execPath, [GEN], { cwd: ROOT, stdio: 'pipe' })
  regenerated = fs.readFileSync(SNAP, 'utf8')
} catch (e) {
  ok('生成器能跑通', false, String(e.stderr ?? e.message).slice(0, 400))
}
if (regenerated !== null) {
  ok('重跑 `gen-kinds.mjs` 后产物与入库**逐字节相同**（漂了 = 改了源码没重生成）',
    regenerated === committed,
    regenerated === committed ? '' :
      '跑 `node docs/anim/gen-kinds.mjs` 重新生成并一起提交')
}

const snap = JSON.parse(committed)

// ---- ③ 档位集合与 ANIM_PRESETS 一致 ----
const { ANIM_PRESETS } = await import(
  pathToFileURL(path.join(ROOT, 'webui/src/polish/engine.js')).href)
const presetKeys = Object.keys(ANIM_PRESETS).sort()
const snapTiers = [...new Set(snap.map((r) => r.value).filter(Boolean))].sort()
ok('快照的档位集合 == `ANIM_PRESETS` 的键（少一档 = 页面少一张卡）',
  JSON.stringify(snapTiers) === JSON.stringify(presetKeys),
  `快照 ${JSON.stringify(snapTiers)}\n         engine ${JSON.stringify(presetKeys)}`)

// ---- ④ 轴归属合法，且不许再有 `both` ----
const AXES = new Set(['none', 'style', 'transition', 'effect'])
const badAxis = snap.filter((r) => !AXES.has(r.axis))
ok('每条快照记录的轴归属都是 `none`/`style`/`transition`/`effect` 之一',
  badAxis.length === 0,
  badAxis.length ? JSON.stringify(badAxis.map((r) => [r.value, r.axis])) : '')
ok('⚠️ 快照里**不许再出现 `both`**（那个类别已随三轴收敛删除）',
  !snap.some((r) => r.axis === 'both'))

// ---- ⑤ ★ 逐字那一路真的有内容 ----
const perCharTiers = snapTiers.filter((t) => ANIM_PRESETS[t]?.perChar)
const snapPerChar = snap.filter((r) => r.preset?.perChar).map((r) => r.value)
ok('★ 有逐字档，且**快照里带着 `perChar`**（否则页面上的逐字演示是空转的）',
  perCharTiers.length > 0 && perCharTiers.every((t) => snapPerChar.includes(t)),
  `engine 里有 perChar 的档 ${JSON.stringify(perCharTiers)}；` +
  `快照里带的 ${JSON.stringify(snapPerChar)}`)

// ---- ⑥ 修完必须把文件恢复原样 ----
if (regenerated !== null) {
  fs.writeFileSync(SNAP, committed, 'utf8')
  ok('比对后把快照恢复原样（这条判据不许有副作用）',
    fs.readFileSync(SNAP, 'utf8') === committed)
}

console.log('='.repeat(72))
if (fails.length) {
  console.error(`✗ docs/anim 快照漂了：${fails.length} 项红（共 ${checks} 项）\n`)
  for (const f of fails) console.error('  - ' + f)
  console.error('\n  ⇒ 这份快照喂着 `docs/anim/text-effects-preview.html`（人工核对档位用）。')
  console.error('     它漂了**不会报错、不会影响出片**，只会让人照着一张错的表做判断 ——')
  console.error('     2026-10-14 实测就是：页面画着 39 档，而实际只有 9 档。')
  process.exit(1)
}
console.log(`✓ docs/anim 快照与真源码一致（${checks} 项判据 · ${snapTiers.length} 档）`)
