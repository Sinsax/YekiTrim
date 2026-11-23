#!/usr/bin/env node
// 表情解码契约：**内建 emoji 目录里的每一条，前端都必须画得出来**。
//
// 为什么值得一条门禁：`safeEmoji` 是"码点 → 字符"的**唯一**解码点（预览与出片各有一份实现，
// 模板那份在 `templates/index.html`），而目录是**编译期内嵌的**、谁也不会在改代码时想到它。
// 两者一旦对不上，症状是"面板上有这个表情、点下去画面上什么都没有或画成别的东西"，
// **不报错、不崩溃** —— 2026-10-15 实测：目录 881 条里 **343 条（39%）** 是多码点序列
// （`203c_fe0f` = ‼️），而老实现只认**单个**十六进制串 ⇒ 那 39% 全都解不出来。
//
// 判据是**穷举**（不是抽样）：目录里每一条都要能解成一个非空字符串。
// 变异自证见 `--mutate`：把 `_` 序列处理去掉，这条必须变红。
import { readFileSync } from 'node:fs'
import { fileURLToPath } from 'node:url'
import { dirname, join } from 'node:path'
import { pathToFileURL } from 'node:url'

const here = dirname(fileURLToPath(import.meta.url))
const repo = join(here, '..', '..')
const CATALOG = join(repo, 'crates', 'yekitrim-polish', 'assets', 'emoji_catalog.json')

// ⚠️ Windows 上动态 import 必须走 `file://` URL（`pathToFileURL`），
//    直接给 `F:\\...` 会报 ERR_UNSUPPORTED_ESM_URL_SCHEME。
const { safeEmoji, isEmojiName } = await import(
  pathToFileURL(join(repo, 'webui', 'src', 'polish', 'engine.js')).href,
)

let fail = 0
const catalog = JSON.parse(readFileSync(CATALOG, 'utf8'))

// ① 穷举：每一条都要画得出来
const blank = []
const notEmoji = []
for (const e of catalog) {
  const cp = e.codepoint
  if (!safeEmoji(cp)) blank.push(cp)
  if (!isEmojiName(cp, '')) notEmoji.push(cp)
}
console.log(`目录 ${catalog.length} 条`)
if (blank.length) {
  fail++
  console.log(`  ✗ 解不出字符的 ${blank.length} 条，前 10: ${blank.slice(0, 10).join(', ')}`)
} else {
  console.log('  ✓ 全部可解码')
}
if (notEmoji.length) {
  fail++
  console.log(`  ✗ isEmojiName 不认的 ${notEmoji.length} 条，前 10: ${notEmoji.slice(0, 10).join(', ')}`)
} else {
  console.log('  ✓ 全部被判为 emoji')
}

// ② 多码点序列必须真的被支持（不是"碰巧都不为空"）
const seq = catalog.filter((e) => e.codepoint.includes('_'))
if (!seq.length) {
  fail++
  console.log('  ✗ 目录里一条多码点序列都没有？分组/目录被改写了')
} else {
  const ok = seq.filter((e) => safeEmoji(e.codepoint).length > 1)
  if (ok.length !== seq.length) {
    fail++
    console.log(`  ✗ 多码点序列 ${seq.length} 条里只有 ${ok.length} 条解成了多字符`)
  } else {
    console.log(`  ✓ 多码点序列 ${seq.length} 条全部解成多字符`)
  }
}

// ③ 越界必须挡住（`fromCodePoint` 对 > 0x10FFFF **抛 RangeError** —— 求值器抛错
//    会让 Vue 卸载整块预览子树，见 polish/AGENTS.md 那次事故）
for (const bad of ['deadbeef', 'ffffffff', '110000']) {
  let threw = false
  let out = ''
  try { out = safeEmoji(bad) } catch { threw = true }
  if (threw || out !== '') {
    fail++
    console.log(`  ✗ 越界值 ${bad} 应当返回空串（得到 ${threw ? '抛错' : JSON.stringify(out)}）`)
  }
}
console.log('  ✓ 越界值返回空串、不抛错')

// ④ 普通文本不许被当成 emoji（否则包内文件名会被画成一个巨大的字符）
for (const word of ['angry', 'celebration', 'happy']) {
  if (isEmojiName(word, '') && safeEmoji(word)) {
    fail++
    console.log(`  ✗ 普通词 ${word} 被当成了 emoji`)
  }
}
console.log('  ✓ 普通词不被当成 emoji')

// ⑤ 直接粘的 emoji 字符认得出来
for (const ch of ['😭', '‼️', '🩸']) {
  if (!isEmojiName(ch, '') || safeEmoji(ch) !== ch) {
    fail++
    console.log(`  ✗ 直接粘的字符 ${ch} 没被认出`)
  }
}
console.log('  ✓ 直接粘的 emoji 字符可用')

if (process.argv.includes('--mutate')) {
  // 变异自证：把「多码点序列」那条分支拆掉，① 必须变红。
  // ⚠️ 判据与显式断言一致 —— 先证明替换**真的命中**了，否则「没红」可能只是没改到。
  //
  // ⚠️⚠️ 命中次数**不再写死 2**（2026-11-24 修）：同一个「码点写法」正则现在有**三处**用法 ——
  //    `isEmojiName` / `safeEmoji` / `emojiGifUrl`（后者是内建 emoji 的 CDN 地址，2026-10-15 才加，
  //    只认码点写法）⇒ 写死 2 会把**多出来的那个合法用法**报成「变异目标出现 3 次，变异无效」，
  //    而**判据与代码都是对的**（那是假红）。
  //    ⇒ 这里只要求**至少命中一次**（防「目标串已经不存在 ⇒ 变异是 no-op」）；
  //    「变异到底有没有生效」由下面 `broken.length` 那条**行为**断言负责：替换之后 `safeEmoji`
  //    若仍能解多码点，那条当场红。三处一起替换是**有意的**（它们本来就是同一份解码口径）。
  const src = readFileSync(join(repo, 'webui', 'src', 'polish', 'engine.js'), 'utf8')
  const target = "/^[0-9a-f]+(_[0-9a-f]+)*$/i"
  const hits = src.split(target).length - 1
  if (hits < 1) {
    console.log(`  ✗ 变异目标一次都没命中 —— 正则写法改了？变异无效`)
    process.exit(1)
  }
  const mutated = src.replaceAll(target, '/^[0-9a-f]+$/i')
  const mod = await import('data:text/javascript;base64,' + Buffer.from(
    mutated.replace(/^import .*$/gm, ''), 'utf8').toString('base64'))
  const broken = catalog.filter((e) => e.codepoint.includes('_') && !mod.safeEmoji(e.codepoint))
  if (!broken.length) {
    console.log('  ✗ 变异后仍全部可解码 —— 这条判据抓不住这个缺陷')
    process.exit(1)
  }
  console.log(`  ✓ 变异自证：去掉多码点支持（${hits} 处同源正则）⇒ ${broken.length} 条解不出（判据有效）`)
}

console.log(fail ? `\n✗ ${fail} 项失败` : '\n✓ 全部通过')
process.exit(fail ? 1 : 0)
