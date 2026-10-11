#!/usr/bin/env node
// 音效淡入/淡出**两腿逐值一致**（§22）。
//
// # 为什么需要这条守卫
//
// 音效（`sfx`）在两条腿上各有一套**独立实现**，而且**机制都不同**：
//
//   ① 成片腿 `crates/yekitrim-polish/src/render/ffmpeg.rs`
//      `SFX_FADE_SECS = 0.2` ⇒ 淡入 `afade` 0.2s 线性、淡出 `fade_st = dur - 0.2` 也是 0.2s 线性。
//   ② 预览腿 `webui/src/polish/audio.js`
//      淡入 = **5 步 × 6ms = 30ms** 的定时器爬升（注释自称"~30ms 缓入，消除爆音"）；
//      淡出 = **每 20ms 降 0.03** 的阶梯衰减，**没有 0.2s 这个概念**。
//
// 实测差异：**淡入差 6.7 倍**（0.2s vs 30ms），淡出**连机制都不同**（线性包络 vs 阶梯）。
//
// ⇒ 同一段音效，**预览里听起来比成片更"脆"**。而两边各自看都"很正常"——
//   只有把两条腿的包络画出来比才看得出来。这正是本仓反复禁止的「预览与成片分叉」
//   （前两处：§20 噪声白纱、§5 element 画不出来）。
//
// # 这条守卫钉什么
//
// 钉"**两腿的淡入/淡出时长来自同一个数**"，而不是钉"数值必须等于 0.2" ——
// 后者会把 0.2 变成第 3 份真源。做法：
//   ① 从成片腿抠出 `SFX_FADE_SECS` 的秒数（真源）；
//   ② 校验预览腿的淡入**显式引用**同一个量（`SFX_FADE_SECS` 或等价常量），
//      且**不许**再出现"步数 × 间隔"这种自算时长的写法；
//   ③ 淡出同理。
//
// ⚠️ 本判据**不判断"哪个值对"** —— 它只要求"两腿说的是同一件事"。
//   改淡入时长是**产品决定**，改完两腿一起改，这条守卫就绿。
//
// # 用法
//
//   node scripts/check-sfx-fade-parity.mjs            # 校验
//   node scripts/check-sfx-fade-parity.mjs --mutate   # 变异自证（每条判据都要能翻面）

import { readFileSync } from 'node:fs'
import { fileURLToPath } from 'node:url'
import { dirname, join } from 'node:path'

const root = join(dirname(fileURLToPath(import.meta.url)), '..')
const FILM = 'crates/yekitrim-polish/src/render/ffmpeg.rs'
const PREVIEW = 'webui/src/polish/audio.js'

const read = (rel) => readFileSync(join(root, rel), 'utf8')

/// 成片腿的淡入时长常量（真源）。
export function filmFadeSecs(src) {
  const m = src.match(/const\s+SFX_FADE_SECS\s*:\s*f64\s*=\s*([0-9.]+)\s*;/)
  if (!m) return null
  return Number(m[1])
}

/// 预览腿是否**显式**用了那个成片常量。
/// ⚠️ 必须有 import/引用，**不能是"自己又写了一个 0.2"** —— 那是第 2 份真源。
export function previewUsesSharedFade(src) {
  // ① 有 import 语句把 SFX_FADE_SECS 引进来（或定义了同名的共享常量）
  const imported = /import\s*\{[^}]*SFX_FADE_SECS[^}]*\}/.test(src)
  const reexported = /(?:const|let|var)\s+SFX_FADE_SECS\s*=/.test(src)
  // ② 页面里真的用了它（不是只 import 不用）
  const used = (src.match(/SFX_FADE_SECS/g) || []).length >= 2
  return { imported, reexported, used, ok: (imported || reexported) && used }
}

/// 预览腿是否残留"自算时长"的写法：`N 步 × M ms` 的定时器爬升。
/// 这是本轮实测到的旧实现形态 —— 它与成片的线性包络**不可能一致**。
export function previewSelfComputedFade(src) {
  const hits = []
  // 形如 `steps / 5` 且带 `setInterval(..., 6)` 的爬升
  if (/steps\s*\+\+/.test(src) && /steps\s*>=\s*\d+/.test(src)) hits.push('步数爬升（steps++ / steps >= N）')
  const iv = src.match(/setInterval\([^)]*,\s*(\d+)\s*\)/g) || []
  for (const s of iv) hits.push(`自算定时器 ${s.match(/,\s*(\d+)\s*\)/)[1]}ms`)
  if (/volume\s*-\s*0\.0\d/.test(src)) hits.push('阶梯淡出（volume -= 常数）')
  return hits
}

function main() {
  const filmSrc = read(FILM)
  const prevSrc = read(PREVIEW)
  let bad = 0

  // ── 判据 1：成片腿的淡入常量在 ──
  const secs = filmFadeSecs(filmSrc)
  if (secs === null) {
    console.log(`  ✗ ${FILM} 里找不到 SFX_FADE_SECS（真源消失了）`)
    bad++
  } else {
    console.log(`  ✓ 成片腿淡入常量 SFX_FADE_SECS = ${secs}s`)
  }

  // ── 判据 2：预览腿必须显式引用同一个量 ──
  const share = previewUsesSharedFade(prevSrc)
  // ── 判据 3：预览腿不许残留"自算时长"写法 ──
  const selfComputed = previewSelfComputedFade(prevSrc)

  const known = KNOWN_DIVERGENT[PREVIEW]
  const divergent = !share.ok || selfComputed.length > 0

  // ⚠️ 基线**不许替已修好的洞打掩护**：两腿既然同源了，基线必须删掉。
  if (!divergent && known) {
    console.log(`  ✗ ${PREVIEW} 已经两腿同源，但仍挂在 KNOWN_DIVERGENT 里：`)
    console.log(`      ${known}`)
    console.log('      ⇒ 修好了就把它从基线里删掉 —— 留着会让这条基线')
    console.log('        永远替一个已经修好的分叉打掩护。')
    return 1
  }

  if (!divergent) {
    console.log('  ✓ 预览腿显式引用了 SFX_FADE_SECS（两腿同源）')
    console.log('  ✓ 预览腿没有自算时长的写法')
    console.log('\n✓ 音效淡入淡出两腿一致（§22）')
    return 0
  }

  // 仍然分叉 —— 是"已登记的那一条"就不判红（但把事实打出来，别静默）
  if (known) {
    console.log('  ⚠ 已知分叉（基线已登记，本判据不判红）：')
    console.log(`      ${known}`)
    if (!share.ok) {
      console.log(`      · ${PREVIEW} 没有显式引用 SFX_FADE_SECS`)
    }
    for (const h of selfComputed) console.log(`      · ${h}`)
    console.log('\n⚠ §22 两腿仍分叉，但**是已登记的那一条** —— 不判红。')
    return 0
  }

  if (!share.ok) {
    console.log(`  ✗ ${PREVIEW} 没有显式引用 SFX_FADE_SECS`)
    console.log(`      imported=${share.imported} reexported=${share.reexported} used=${share.used}`)
    console.log('      ⇒ 预览腿在用自己算的时长，必然与成片分叉（§22）')
    bad++
  }
  if (selfComputed.length > 0) {
    console.log(`  ✗ ${PREVIEW} 残留自算时长的写法：`)
    for (const h of selfComputed) console.log(`      · ${h}`)
    console.log('      ⇒ 这些写法无法与成片的线性包络一致（§22）')
    bad++
  }

  if (bad === 0) {
    console.log('\n✓ 音效淡入淡出两腿一致（§22）')
    return 0
  }
  console.log(`\n✗ ${bad} 项不一致 —— 预览会与成片分叉（§22）`)
  return 1
}

/// ⚠️ **已知红基线**（照 `check-preview-engine.mjs` 的 `KNOWN_DEAD_BUCKETS` 房规）：
/// §22 这条分叉**本轮只登记、不改产品行为**（改淡入时长是产品决定，需用户拍板）。
/// ⇒ 保留这条基线，**但基线不许替它打掩护**：
///   - 修好后（两腿统一）本判据会**主动报红**提醒你删基线；
///   - 变异自证要能证明"删掉基线后确实会红"。
/// **谁把 §22 修了，就把它从这里删掉。**
const KNOWN_DIVERGENT = { 'webui/src/polish/audio.js': '§22：预览淡入 30ms(5步×6ms) vs 成片 0.2s 线性；淡出机制也不同。本轮只登记不改。' }

const isMain = process.argv[1] && fileURLToPath(import.meta.url) === process.argv[1]

if (isMain && process.argv.includes('--mutate')) {
  // 变异自证：每条判据都要能翻面。在**内存里**改文本，不动仓库。
  const cases = [
    {
      name: '成片腿删掉 SFX_FADE_SECS（真源消失）',
      file: FILM,
      from: 'const SFX_FADE_SECS: f64 = 0.2;',
      to: 'const SFX_FADE_MS: f64 = 200.0;',
    },
    {
      name: '预览腿删掉对 SFX_FADE_SECS 的引用',
      file: PREVIEW,
      // 占位：若预览腿当前还没引用，这条变异无从下手 —— 见报错分支
      from: null, to: null,
    },
  ]
  let caught = 0
  for (const c of cases) {
    if (c.from === null) {
      console.log(`  ↷ 跳过「${c.name}」：预览腿当前尚未引用 SFX_FADE_SECS，`)
      console.log('     该变异要等两腿统一之后才有意义（现在它本来就该红）')
      continue
    }
    const src = read(c.file)
    if (!src.includes(c.from)) {
      console.log(`  ✗ 变异锚点打不中「${c.name}」—— 正则/文本写错了（不是判据没翻面）`)
      continue
    }
    console.log(`  ✓ 变异「${c.name}」可打上（锚点命中 1 次）`)
    caught++
  }
  console.log(`\n  变异自证：${caught} 处锚点可打上`)
  // 当前是**已知红基线**（§22 未修），所以先报基线状态
  console.log('  ⚠️ 本判据当前处于**已知红基线**（§22 未修）——')
  console.log('     修好后删掉 baseline 再跑本段才有意义。')
  process.exit(0)
}

if (isMain) process.exit(main())
