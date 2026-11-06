#!/usr/bin/env node
// **文字元素面板的「排版 / 过渡 / 动效」三轴下拉：退役守卫**（2026-10-20 文字层退役后重写）。
//
// ============================================================================
// ⚠️⚠️ 旧版本（2026-10-14 三轴收敛）判的是什么，为什么整段退役
// ============================================================================
//
// 旧版用 `@vue/compiler-sfc` 把 `TimelineEditor.vue` 真编译一遍，然后断言：
//   ③ 三栏下拉真的渲染出来了（`v-for="g in ANIM_GROUPS"` / `styleVariants` /
//      `transitionVariants` / `effectVariants` / `v-for="a in TRANSITION_OPTIONS"`），
//      且认不出的档名落到「未知」兜底、三个字段各自绑定 `selItem.style_anim` /
//      `transition_anim` / `effect_anim`；
//   ④ 面板上的「排版」「过渡」「动效」三个标签 + 角度滑杆（`LAYOUT_ANGLE_MIN/MAX` /
//      `showAngle` 门控）+ 两处 `kindLabelOn(axis)` 传轴。
//
// **为什么退役**：2026-10-20 用户口径「算了，删除文字层吧」⇒ 文字层整条退役
// （读时丢弃 + 计数），编辑器里的**文字支整块删掉**（含三轴下拉与角度滑杆）。
// 那些断言在"产品按新口径删干净了"之后**永久红**，而且红的时候指错了地方 ——
// 会让人以为"下拉没渲染"是 bug，其实是它该被删。
//
// ============================================================================
// 现在锁什么（**退役守卫**：删掉的东西不许悄悄回来）
// ============================================================================
//
//   ① `TimelineEditor.vue` 仍有 `<script setup>` 且能取到 `<template>`。
//      这条是通用兜底：**Vue 对模板里未绑定的标识符不报错、渲染成空白**
//      （本仓踩过，见 `webui/src/AGENTS.md`），所以模板一旦引用不存在的名字，
//      构建成功、界面空白。
//   ② **文字动效面板不许回来**：模板（剥注释后）里不许出现
//      `selItem.style_anim` / `selItem.transition_anim` / `selItem.effect_anim` /
//      `selItem.layout_angle_deg` 的绑定，也不许出现 `ANIM_GROUPS` /
//      `styleVariants` / `transitionVariants` / `effectVariants` / `showAngle` /
//      `TRANSITION_OPTIONS` / `LAYOUT_ANGLE_MIN` / `LAYOUT_ANGLE_MAX` 的模板引用。
//      为什么锁这个：这些是**文字元素**的逐条动效字段；文字层已退役（后端读时丢弃），
//      面板若把它们接回来，用户能配、成片里没有，**且不报错** —— 本仓最恨的形状。
//      要重新支持文字元素，得先恢复后端整条链并重建它自己的判据（不是把这段删掉）。
//   ③ `ANIM_KINDS` 等声明**可以**留在 `<script setup>` 里（退役清理不归本判据管），
//      但**不许有模板消费者**（②已经覆盖）。判据只认"用户看得见的接线"。
//
// # 反向/变异判据（`--self-test`）
//
//   把 `<label>过渡</label>` + `selItem.transition_anim` 塞回模板、把
//   `v-for="g in ANIM_GROUPS"` 塞回模板 —— 两种回潮都必须让 ② **翻红**。
//   判据写错的表现是"永远绿"，比报错难发现。
//
// 用法：`node scripts/check-anim-dropdown.mjs [--self-test]`（退出码非 0 即失败）

import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const HERE = path.dirname(fileURLToPath(import.meta.url))
const ROOT = path.join(HERE, '..')
const FILE = path.join(ROOT, 'webui', 'src', 'components', 'TimelineEditor.vue')

/// 剥注释：**HTML 注释也要剥** —— 退役说明本身就写着这些字段名，
/// 不剥的话判据会被它自己的告警顶翻（本仓 `check-editor-lane-split` 踩过同类假阳性）。
const stripComments = (s) => s
  .replace(/<!--[\s\S]*?-->/g, '')
  .replace(/\/\*[\s\S]*?\*\//g, '')
  .replace(/(^|[^:])\/\/[^\n]*/g, '$1')

/// 取出 `<template>...</template>` 段（只看用户看得见的模板；`<style>` / `<script>` 不算）。
function templateOf(src) {
  const start = src.indexOf('<template>')
  const end = src.lastIndexOf('</template>')
  if (start < 0 || end < 0 || end <= start) return ''
  return src.slice(start + '<template>'.length, end)
}

/// 模板里禁用的**三轴动效接线**（任何一个出现 = 文字动效面板回潮）。
const FORBIDDEN = [
  // 三条轴的逐条字段绑定（用户一改就落盘）。
  'selItem.style_anim',
  'selItem.transition_anim',
  'selItem.effect_anim',
  'selItem.layout_angle_deg',
  // 三栏下拉/兜底/角度滑杆的模板消费者。
  'ANIM_GROUPS',
  'styleVariants',
  'transitionVariants',
  'effectVariants',
  'TRANSITION_OPTIONS',
  'showAngle',
  'LAYOUT_ANGLE_MIN',
  'LAYOUT_ANGLE_MAX',
]

/**
 * 对**一份源码文本**跑全部判据（主流程传真文件；`--self-test` 传变异体）。
 * ⚠️ 抽成函数是为了让自证跑的是**同一份判据**。
 */
function analyze(src) {
  const pass = []
  const failures = []

  // ① 有 <script setup> 且取得到 <template>（通用兜底：模板引用未绑定标识符会静默空白）
  if (/<script\s+setup[^>]*>/.test(src)) pass.push('① 有 `<script setup>`')
  else failures.push('① 找不到 `<script setup>` —— 判据的前提没了')

  const tpl = templateOf(src)
  if (!tpl) {
    failures.push('① 取不到 `<template>` 段（结构变了就要改本判据）')
    return { pass, failures }
  }
  pass.push('① 取得到 `<template>`')

  // ② 文字动效面板不许回来（模板里一个禁用标识符都不许出现）
  const code = stripComments(tpl)
  const hits = FORBIDDEN.filter((k) => code.includes(k))
  if (hits.length === 0) {
    pass.push('② 文字动效面板没有模板消费者（检查了 ' + FORBIDDEN.length + ' 个禁用标识符）')
  } else {
    failures.push(
      '② 文字动效面板回潮：模板里出现了 ' + JSON.stringify(hits) + ' —— ' +
      '文字层已退役（后端读时丢弃），这些控件接回来只会是"用户能配、成片里没有、且不报错"。' +
      '要重新支持文字元素，先恢复后端整条链并重建它自己的判据。',
    )
  }
  return { pass, failures }
}

const src = fs.readFileSync(FILE, 'utf8')

// ---------------------------------------------------------------------------
// 自证：判据不许空转 —— 两种"把面板接回来"的形态都必须翻红
// ---------------------------------------------------------------------------
if (process.argv.includes('--self-test')) {
  const { strict: assert } = await import('node:assert')
  let self = 0

  // ① 把「过渡」标签 + 字段绑定塞回元素面板 ⇒ 必须红
  const withSelect = src.replace(
    '<label class="pos-lbl">位置</label>',
    '<label>过渡</label><select :value="selItem.transition_anim"></select><label class="pos-lbl">位置</label>',
  )
  assert.notEqual(withSelect, src, '自证①：变异必须命中（模板锚点变了就要改自证）')
  assert.ok(analyze(withSelect).failures.some((f) => f.includes('transition_anim')),
    '自证①：把过渡下拉塞回去必须红并点名字段')
  self += 1

  // ② 把三栏下拉的 v-for 塞回模板 ⇒ 必须红
  const withGroups = src.replace(
    '<label class="pos-lbl">位置</label>',
    '<label>动效</label><select v-for="g in ANIM_GROUPS"></select><label class="pos-lbl">位置</label>',
  )
  assert.notEqual(withGroups, src, '自证②：变异必须命中')
  assert.ok(analyze(withGroups).failures.some((f) => f.includes('ANIM_GROUPS')),
    '自证②：把 ANIM_GROUPS 下拉塞回去必须红并点名')
  self += 1

  console.log('✓ 文字动效下拉退役守卫自证：' + self + ' 项（字段绑定回潮 / 下拉组回潮，都必须红）')
  process.exit(0)
}

const { pass, failures } = analyze(src)
console.log('文字动效面板退役守卫：' + pass.length + ' 项通过，' + failures.length + ' 项失败')
if (failures.length) {
  console.log()
  for (const f of failures) console.log('  ✗ ' + f)
  process.exit(1)
}
console.log('✓ 文字层已退役：模板里没有三轴动效面板的接线（ANIM_KINDS 允许留作死代码，但不许接回模板）')
