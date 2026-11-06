#!/usr/bin/env node
// WebUI 简化契约：能力入口不许静默消失。
//
// 用法：
//   node webui/scripts/check-simplify.mjs             # 默认按 --phase=P1 检查
//   node webui/scripts/check-simplify.mjs --phase=P0  # 只查清单/路由（P0 用）
//   node webui/scripts/check-simplify.mjs --mutate    # 额外跑原语自证
//
// 判据（只登记"能力入口"，不要求每个按钮都带锚点）：
//   ① 清单自洽：id/anchor 唯一、surface 合法、route 在 router.js（settings-* 由 utils.js 的 key 派生）。
//   ② 每个生效能力的 anchor 在声明文件里恰好出现 1 次（剥 HTML 注释后数）。
//   ③ advancedOnly 的 anchor 必须位于 <details> 块内；非 advanced 落在 <details> 里只告警。
//   ④ 每面 firstScreen 数 ≤ budget；能力总数 ≥ floor（棘轮：删能力要显式改清单）。
//   ⑤ webui/src 里的每个 data-cap 都必须在清单里（防"新控件没登记 ⇒ 门禁看不见"）。
//   ⑥ /models 必须 redirect 到 /workbench（模型已搬到首页运行状况）。
//
// 这个脚本零依赖、不 import 应用模块、不写盘；`--mutate` 只改内存里的字符串。
import { readFileSync, readdirSync, statSync } from 'node:fs'
import { dirname, join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), '..', '..')
const PHASES = { P0: 0, P1: 1, P2: 2, P3: 3 }
const phaseArg = (process.argv.find((a) => a.startsWith('--phase=')) || '--phase=P1').split('=')[1]
const MUTATE = process.argv.includes('--mutate')
if (!(phaseArg in PHASES)) {
  console.error('未知 --phase：' + phaseArg)
  process.exit(2)
}

let failed = 0
function check(label, ok, detail = '') {
  if (ok) {
    console.log('  ok   ' + label)
  } else {
    failed++
    console.log('  FAIL ' + label)
    if (detail) console.log('         ' + detail)
  }
}
const read = (p) => readFileSync(join(ROOT, p), 'utf8')
const stripHtmlComments = (s) => s.replace(/<!--[\s\S]*?-->/g, '')

const manifest = JSON.parse(read('docs/webui-capabilities.json'))
const router = read('webui/src/router.js')
const utils = read('webui/src/utils.js')

const caps = Array.isArray(manifest.capabilities) ? manifest.capabilities : []
const ids = new Set()
const anchors = new Set()
for (const c of caps) {
  check('能力 id 唯一：' + c.id, !ids.has(c.id))
  ids.add(c.id)
  check('anchor 唯一：' + c.anchor, !anchors.has(c.anchor))
  anchors.add(c.anchor)
  check('surface 合法：' + c.id + ' → ' + c.surface, !!manifest.surfaces[c.surface])
  check('phase 合法：' + c.id, c.phase in PHASES)
}

// 路由：settings-* 由 SETTINGS_SECTIONS 的 key 派生，其余必须是 router.js 里的字面量 name。
function routeExists(name) {
  if (!name || name === '*') return true
  const m = /^settings-(.+)$/.exec(name)
  if (m) return new RegExp(`key:\\s*'${m[1]}'`).test(utils)
  return new RegExp(`name:\\s*'${name}'`).test(router)
}
for (const [key, s] of Object.entries(manifest.surfaces || {})) {
  check(`surface ${key} 的 route 存在：${s.route}`, routeExists(s.route))
}
check('/models redirect 到 /workbench', /path:\s*'\/models',\s*redirect:\s*'\/workbench'/.test(router))

// 读声明文件（按需）
const fileCache = new Map()
function sourceOf(p) {
  if (!fileCache.has(p)) {
    try {
      fileCache.set(p, stripHtmlComments(read(p)))
    } catch {
      fileCache.set(p, null)
    }
  }
  return fileCache.get(p)
}
function escapeRe(s) {
  return s.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')
}
function countAnchor(src, anchor) {
  const re = new RegExp(escapeRe(anchor), 'g')
  return (src.match(re) || []).length
}
/** 模板里所有 <details>…</details> 的 [start,end) 区间（只扫 <script> 之前）。 */
function detailsRanges(src) {
  const end = src.indexOf('<script')
  const tpl = end >= 0 ? src.slice(0, end) : src
  const re = /<details\b|<\/details>/g
  const ranges = []
  let depth = 0
  let start = 0
  let m
  while ((m = re.exec(tpl))) {
    if (m[0].startsWith('</')) {
      depth--
      if (depth === 0) ranges.push([start, m.index])
    } else {
      if (depth === 0) start = m.index
      depth++
    }
  }
  return ranges
}
function insideDetails(src, anchor) {
  const at = src.indexOf(anchor)
  if (at < 0) return false
  return detailsRanges(src).some(([a, b]) => at >= a && at < b)
}

const active = caps.filter((c) => PHASES[c.phase] <= PHASES[phaseArg])
console.log(`\n按 phase=${phaseArg} 检查 ${active.length}/${caps.length} 条能力`)

for (const c of active) {
  const files = Array.isArray(c.files) ? c.files : []
  let hits = 0
  let advancedHit = false
  for (const f of files) {
    const src = sourceOf(f)
    if (src == null) continue
    const n = countAnchor(src, c.anchor)
    hits += n
    if (n && insideDetails(src, c.anchor)) advancedHit = true
  }
  check(`anchor 恰好 1 处：${c.id}`, hits === 1, `实得 ${hits} 处；文件 ${files.join(', ')}`)
  if (c.advancedOnly) {
    check(`advancedOnly 位于 <details>：${c.id}`, advancedHit)
  }
}

// 首屏预算 + floor（棘轮）
const bySurface = {}
for (const c of active) {
  bySurface[c.surface] ||= []
  bySurface[c.surface].push(c)
}
if (!active.length) {
  console.log('  · 本 phase 还没有生效能力（P0 只查清单/路由），跳过首屏预算与 floor')
} else {
  for (const [key, s] of Object.entries(manifest.surfaces || {})) {
    const list = bySurface[key] || []
    // 这个 phase 还没轮到该 surface（例如 P1 的 settings 是 P3）⇒ 不拿 floor 误报；
    // 到了对应 phase、清单里却一条都没生效，才是真丢能力。
    if (!list.length) continue
    const first = list.filter((c) => c.firstScreen).length
    check(`首屏预算 ${key}: ${first}/${s.first_screen_budget}`, first <= s.first_screen_budget)
    check(`能力下限 ${key}: ${list.length}/${s.floor}`, list.length >= s.floor)
  }
}

// 反向覆盖：webui/src 里出现的每个 data-cap 都必须在清单里
const known = new Set(caps.map((c) => c.anchor))
const orphan = []
function walk(dir) {
  for (const name of readdirSync(dir)) {
    const p = join(dir, name)
    const st = statSync(p)
    if (st.isDirectory()) {
      walk(p)
      continue
    }
    if (!/\.(vue|js)$/.test(name)) continue
    const src = stripHtmlComments(readFileSync(p, 'utf8'))
    // ⚠️ 只认**静态** `data-cap="..."`：`:data-cap="dataCap || null"` 是 Vue 绑定，
    // 不是锚点（NextStepBar 的 dataCap prop 就是这么把锚点挂到真实 CTA 上的）。
    const re = /(?:^|\s)data-cap=(["'])([^"']+)\1/g
    let m
    while ((m = re.exec(src))) {
      const anchor = `data-cap="${m[2]}"`
      if (!known.has(anchor)) orphan.push(`${p.replace(ROOT + '/', '')} → ${anchor}`)
    }
  }
}
walk(join(ROOT, 'webui/src'))
check('没有未登记 data-cap', orphan.length === 0, orphan.slice(0, 8).join('; '))

// 原语自证：确认判据不是空转
if (MUTATE) {
  console.log('\n--mutate 原语自证')
  const good = '<details><summary>x</summary><button data-cap="a.b"></button></details>'
  const bad = '<div><button data-cap="a.b"></button></div>'
  check('[自证] 计数命中 1', countAnchor(good, 'data-cap="a.b"') === 1)
  check('[自证] 删掉锚点 → 0', countAnchor(good.replace(' data-cap="a.b"', ''), 'data-cap="a.b"') === 0)
  check('[自证] 复制锚点 → 2', countAnchor(good + good, 'data-cap="a.b"') === 2)
  check('[自证] details 内识别为 true', insideDetails(good, 'data-cap="a.b"'))
  check('[自证] details 外识别为 false', !insideDetails(bad, 'data-cap="a.b"'))
}

console.log(failed ? `\n✗ ${failed} 条不通过` : `\n✓ check-simplify 全过（phase=${phaseArg}）`)
process.exit(failed ? 1 : 0)
