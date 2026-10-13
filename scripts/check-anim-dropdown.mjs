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
// ⚠️ **2026-10-13：下拉改成两级（种类 → 细分）**，模板引用的标识符跟着变。
//    这几个**每一个都必须真的在绑定里** —— 模板引用了未绑定的名字时
//    Vue **不报错、渲染成空白**，而构建照样成功（这正是本脚本存在的理由）。
for (const id of ['ANIM_KINDS', 'ANIM_KNOWN', 'animKind', 'animVariants',
                  'animKindHint', 'setAnimKind', 'setAnimVariant']) {
  ok(bindings.has(id),
    `① \`${id}\` 不在 \`<script setup>\` 的绑定里 —— 模板引用它会渲染成空白**且不报错**`)
}

// ---- ② 档数与种类（直接读源码里的字面量）----
//
// ⚠️ 权威表是 `ANIM_KINDS`（两级）。`ANIM_GROUPS` 是它 `.map()` 出来的兼容视图，
//    **这里不能扒它** —— 它里面一个 `value: '…'` 字面量都没有（是算出来的），
//    扒它会得到 0 档 ⇒ "档数不对"全线假红，而真因是判据扒错了地方。
// ⚠️ 必须**切到定义结束**为止：只 `slice(indexOf(...))` 会一直读到后面的数组
//    （实测把之后另一张带 value 的表也算了进来 ⇒ **判据自己数错了**，代码是对的）。
const gStart = src.indexOf('const ANIM_KINDS = [')
ok(gStart > 0, '② 源码里找不到 `const ANIM_KINDS = [` —— 两级表没了（判据的前提没了）')
const gEnd = src.indexOf('\nconst ', gStart + 1)
const block = gStart > 0 ? src.slice(gStart, gEnd > gStart ? gEnd : undefined) : ''
const values = [...block.matchAll(/\{\s*value:\s*'([a-z0-9_]*)'/g)].map((m) => m[1])
const nonEmpty = values.filter((v) => v !== '')
// 种类：`kind: '...'`（缩进 **4 空格** —— 数组项一层 + 对象体一层；
// ⚠️ 写成 2 会让它一个都取不到 ⇒ 全线假红，而真因是判据自己的正则）。
const kinds = [...block.matchAll(/^\s+kind:\s*'([a-z_]+)',\s*$/gm)].map((m) => m[1])

// ⚠️ 计数**随施工推进而增长**，不是钉死的历史数字：
//    A+B 组 17 档 → 加 C 组 14 档 → 加 D 组 4 档 + E 组 3 档 = 39 档 / 14 族。
//    ⚠️ **2026-10-13 两级化之后**：种类数 = 14（"不动效" + 13 个效果族）。
//    改这里时**必须同时**改 `check-anim-presets-parity.mjs` 的档数 ——
//    两边对不上就说明有一处漏改了。
const WANT_KINDS = 14
const WANT_TIERS = 39
ok(kinds.length === WANT_KINDS,
  `② 种类数应当是 ${WANT_KINDS}（不动效 + 13 个效果族），实得 ${kinds.length}`)
ok(new Set(kinds).size === kinds.length,
  `② 种类里有重复：${JSON.stringify(kinds.filter((k, i) => kinds.indexOf(k) !== i))}`)
ok(values.includes(''), '② 缺"不动效"那一档（空串）')
ok(nonEmpty.length === WANT_TIERS, `② 已知档应当 ${WANT_TIERS} 档，实得 ${nonEmpty.length}`)
ok(new Set(nonEmpty).size === nonEmpty.length, '② 有重复档名')

// ---- ③ 模板真的用了这些标识符（而不只是定义了）----
//
// ⚠️ **两级**：种类一个 `<select>`、细分另一个。以前那条
//    `v-for="g in ANIM_GROUPS"` + `<optgroup>` 的断言**已经作废**
//    （那正是用户说"还是这个"的形态）—— 换成断言两级真的各有一个控件。
const tpl = parsed.descriptor.template?.content ?? ''
// ⚠️⚠️ **先剥 HTML 注释再匹配** —— 本仓踩过这个假阳性（`check_module_boundaries.py`
//    的上一版手写 grep 判据因此产出 8 条假阳性，等于没用）。
//    这里尤其要紧：**"我把 `<optgroup>` 删掉了"这句话本身必须写在注释里**
//    （否则下一个人会把它加回来），而只匹配文本的判据会**把说明当成违规**。
//    ⇒ 断言只对**真标记**生效。`(?<!--)` 不做可变长回看，所以直接整体剥掉。
const tplMarkup = tpl.replace(/<!--[\s\S]*?-->/g, '')
ok(/v-for="k in ANIM_KINDS"/.test(tplMarkup), '③ 模板没有 `v-for="k in ANIM_KINDS"` —— 种类下拉是空的')
ok(/v-for="a in animVariants"/.test(tplMarkup), '③ 模板没有 `v-for="a in animVariants"` —— 细分下拉是空的')
ok(/ANIM_KNOWN\.includes\(/.test(tplMarkup), '③ 模板没有用 `ANIM_KNOWN` 判断未知档')
ok(/animKind === ''/.test(tplMarkup), '③ 认不出的档名没有让种类控件落到"未知"那一支')
// ⚠️ **只对真标记断言**（`tplMarkup`）—— 见上面"先剥注释"那段。
ok(!/<optgroup/.test(tplMarkup),
  '③ 又出现了 `<optgroup>` —— 那是"四十项平铺进一个下拉"的旧形态，用户报过"还是这个"')
// ⚠️ 未知档必须**有 option**（否则 `<select>` 的值匹配不上任何一项，
//    浏览器会显示成第一项"不动效"——用户以为自己没选错）。
ok(/未知档/.test(tplMarkup), '③ 模板里没有"未知档"那条 option')

// ---- 结果 ----
if (fails.length === 0) {
  console.log(
    `✓ 动效下拉真的接上了（${checks} 项：SFC 编译绑定 + 两级 ${WANT_KINDS} 种类 / ${WANT_TIERS} 档 + 模板接线）`,
  )
  process.exit(0)
}
console.log(`✗ 动效下拉有问题（${fails.length} / ${checks}）：`)
for (const f of fails) console.log('    ' + f)
process.exit(1)
