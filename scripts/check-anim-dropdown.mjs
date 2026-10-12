// 几何 + 行为断言：动效下拉**真的渲染出来了**（不是"模板里有就算"）。
//
// ⚠️ 为什么必须真跑一遍：**Vue 对未定义的模板标识符不报错、渲染成空白**
//    （本仓踩过，见 webui/src/AGENTS.md 与 `check-layout-invariants.mjs`）。
//    所以"我加了 `<optgroup v-for="g in ANIM_GROUPS">`"这句话，
//    在 `ANIM_GROUPS` 没进 setup 作用域时会**静默什么都不渲染**，
//    而构建照样成功、判据照样绿。
//
// 这里用 `@vue/compiler-sfc` 把组件**真编译一遍**，然后：
//   ① 断言模板里引用的每个标识符都在 `<script setup>` 的绑定里（编译诊断能查出来）；
//   ② 直接把 `ANIM_GROUPS` 抽出来数一遍档数。
import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const HERE = path.dirname(fileURLToPath(import.meta.url))
const ROOT = path.join(HERE, '..')
const FILE = path.join(ROOT, 'webui', 'src', 'components', 'TimelineEditor.vue')

const src = fs.readFileSync(FILE, 'utf8')
const fails = []
let checks = 0
const ok = (cond, what) => {
  checks++
  if (!cond) fails.push(what)
}

// ---- ① 编译一次，看有没有"未定义标识符"这类诊断 ----
//
// `@vue/compiler-sfc` 在 `webui/node_modules` 下（vite 的依赖，必然在）。
const sfcMod = await import(
  new URL('../webui/node_modules/@vue/compiler-sfc/dist/compiler-sfc.cjs.js', import.meta.url).href
)
const sfc = sfcMod.default ?? sfcMod
const parsed = sfc.parse(src, { filename: 'TimelineEditor.vue' })
ok(!parsed.errors || parsed.errors.length === 0,
  `SFC 解析有错：${JSON.stringify((parsed.errors ?? []).map(String))}`)

const script = parsed.descriptor.scriptSetup ?? parsed.descriptor.script
ok(!!script, 'TimelineEditor.vue 没有 <script setup>（判据的前提没了）')

// ⚠️ 编译 `<script setup>` 会拿到 `bindings`（模板可用的标识符全集）。
const compiled = sfc.compileScript(parsed.descriptor, { id: 'x' })
const bindings = new Set(Object.keys(compiled.bindings ?? {}))
ok(bindings.has('ANIM_GROUPS'),
  '① `ANIM_GROUPS` 不在 `<script setup>` 的绑定里 —— 模板里的 v-for 会渲染成空白**且不报错**')
ok(bindings.has('ANIM_KNOWN'),
  '① `ANIM_KNOWN` 不在绑定里 —— 那条"未知档"的 option 与提示永远不会出现')

// ---- ② 档数与分组（直接读源码里的字面量）----
// ⚠️ 必须**切到定义结束**为止：只 `slice(indexOf(...))` 会一直读到后面的数组
//    （实测把 `ANIM_GROUPS` 之后的另一张 `{value:'color'}` 表也算了进来 ⇒
//     "17 档"变成 21 —— **判据自己数错了**，代码是对的）。
// ⚠️ 必须**切到定义结束**为止：只 `slice(indexOf(...))` 会一直读到后面的数组
//    （实测把 `ANIM_GROUPS` 之后的另一张 `{value:'color'}` 表也算了进来 ⇒
//     "17 档"变成 21 —— **判据自己数错了**，代码是对的）。
const gStart = src.indexOf('const ANIM_GROUPS = [')
const gEnd = src.indexOf('\nconst ', gStart + 1)
const block = src.slice(gStart, gEnd > gStart ? gEnd : undefined)
const groups = [...block.matchAll(/group:\s*'([^']+)'/g)].map((m) => m[1])
const values = [...block.matchAll(/\{\s*value:\s*'([a-z0-9_]*)'/g)].map((m) => m[1])
const nonEmpty = values.filter((v) => v !== '')

// ⚠️ 计数**随施工推进而增长**，不是钉死的历史数字：
//    A+B 组 17 档（7 组）→ 加 C 组逐字 14 档（+5 组）= 32 档 / 12 组。
//    改这里时**必须同时**改 `check-anim-presets-parity.mjs` 的档数 ——
//    两边对不上就说明有一处漏改了。
const WANT_GROUPS = 14
const WANT_TIERS = 39
ok(groups.length === WANT_GROUPS, `② 分组数应当是 ${WANT_GROUPS}（不动 + 11 个效果族），实得 ${groups.length}`)
ok(values.includes(''), '② 缺"不动效"那一档（空串）')
ok(nonEmpty.length === WANT_TIERS, `② 已知档应当 ${WANT_TIERS} 档，实得 ${nonEmpty.length}`)
ok(new Set(nonEmpty).size === nonEmpty.length, '② 有重复档名')

// ---- ③ 模板真的用了这两个标识符（而不只是定义了）----
const tpl = parsed.descriptor.template?.content ?? ''
ok(/v-for="g in ANIM_GROUPS"/.test(tpl), '③ 模板没有 `v-for="g in ANIM_GROUPS"` —— 下拉是空的')
ok(/v-for="a in g\.items"/.test(tpl), '③ 模板没有遍历分组内的档')
ok(/ANIM_KNOWN\.includes\(/.test(tpl), '③ 模板没有用 `ANIM_KNOWN` 判断未知档')
ok(/<optgroup/.test(tpl), '③ 没有 `<optgroup>` —— 三十多档平铺成一个下拉没法用')
// ⚠️ 未知档必须**有 option**（否则 `<select>` 的值匹配不上任何一项，
//    浏览器会显示成第一项"不动效"——用户以为自己没选错）。
ok(/未知档/.test(tpl), '③ 模板里没有"未知档"那条 option')

// ---- 结果 ----
if (fails.length === 0) {
  console.log(
    `✓ 动效下拉真的接上了（${checks} 项：SFCE 编译绑定 + ${WANT_GROUPS} 组 / ${WANT_TIERS} 档 + 模板接线）`,
  )
  process.exit(0)
}
console.log(`✗ 动效下拉有问题（${fails.length} / ${checks}）：`)
for (const f of fails) console.log('    ' + f)
process.exit(1)
