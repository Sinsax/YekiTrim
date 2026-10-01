// 封面双通道对拍门禁（chrome ↔ takumi）。
//
// 为什么要"分区"判据：实测（docs/plan/takumi-cover-capability.md §10–§11）
//   · **非文字区**（背景/帧层/渐变/暗角/扫描线/滤镜/图层栈）：**≤1.6/255**（逐像素级）；
//   · **标题所在band**：19~49/255 —— 那是 **Takumi 与 Chrome/Skia 的字形栅格化固有差**，
//     适配层改不了（双层标题试过，19.57 → 19.93 没改善）。
// 拿全图均值一刀切只会让这道门永远红或永远松，所以按区判、阈值各自标定。
//
// 用法：
//   node scripts/check-cover-takumi.mjs [--project <精修工程目录>] [--exe <vtrim.exe>]
//        [--work <临时目录>] [--font <字体文件>] [--require] [--self-test]
//   · 缺工程/缺二进制（没编 cover-takumi）/缺字体 → **SKIP** 并打印要准备什么（`--require` 时算失败）。
//   · 阈值与标定依据写在 THRESHOLDS 旁；超阈值 **exit 1**（报告照留）。
//   · `--self-test` 自证判据本身（不再渲染）：全 0 必须过、大差必须红、分区必须独立生效。
import { spawnSync } from 'node:child_process'
import fs from 'node:fs'
import path from 'node:path'

const args = process.argv.slice(2)
const has = (n) => args.includes('--' + n)
const opt = (n, d) => { const i = args.indexOf('--' + n); return i >= 0 && args[i + 1] ? args[i + 1] : d }

const W = 1920, H = 1080
const NON_TEXT_BAND = 0.2 // 上/下各 20% 视作"非文字区"（本门禁的 fixture 把标题放在中间）
// 阈值：非文字区严（实测 1.6，留 2.5× 余量）；整图按实测基线（19.9）留一成余量。
const THRESHOLDS = { nonTextMean: 4, nonTextOver32: 1.0, fullMean: 22, fullOver32: 13 }

/** 判据本体（纯函数，`--self-test` 直接喂假 stats）。 */
export function judge(stats, th = THRESHOLDS) {
  const bad = []
  if (stats.nonTextMean > th.nonTextMean) bad.push(`非文字区均值 ${stats.nonTextMean.toFixed(2)} > ${th.nonTextMean}`)
  if (stats.nonTextOver32 > th.nonTextOver32) bad.push(`非文字区 >32 ${stats.nonTextOver32.toFixed(2)}% > ${th.nonTextOver32}%`)
  if (stats.fullMean > th.fullMean) bad.push(`整图均值 ${stats.fullMean.toFixed(2)} > ${th.fullMean}`)
  if (stats.fullOver32 > th.fullOver32) bad.push(`整图 >32 ${stats.fullOver32.toFixed(2)}% > ${th.fullOver32}%`)
  return { pass: bad.length === 0, bad }
}

/** 从工程算出**标题矩形**（行区间），判据取矩形之外 —— 比"上下 20%"诚实。 */
function textRows(projectDir) {
  try {
    const toml = fs.readFileSync(path.join(projectDir, 'polish.toml'), 'utf8')
    const L = toml.split(/\r?\n/)
    let sec = ''
    const get = (k) => {
      let out = null
      for (const line of L) {
        const m = line.match(/^\s*\[([^\]]+)\]/)
        if (m) { sec = m[1]; continue }
        if (sec !== 'cover') continue
        const kv = line.match(new RegExp('^\\s*' + k + '\\s*=\\s*(.+)$'))
        if (kv) out = kv[1].trim().replace(/^["']|["']$/g, '')
      }
      return out
    }
    const fsPx = parseFloat(get('font_size') || '120')
    const lh = parseFloat(get('line_height') || '1.7')
    const ty = parseFloat((get('text_y') || '52%').replace('%', ''))
    // 标题行数：数 title_lines 数组里的元素（每行一个字符串）
    let lines = 1
    const arr = toml.match(/title_lines\s*=\s*\[([\s\S]*?)\]/)
    if (arr) lines = (arr[1].match(/"/g) || []).length / 2 || 1
    const h = Math.round(fsPx * lh * lines)
    const top = Math.round((ty / 100) * 1080 - h / 2)
    const bot = top + h
    return { topEnd: Math.max(0, top), botStart: Math.min(1080, bot), note: `标题矩形 y[${top},${bot}]（${lines} 行 × ${fsPx} × ${lh}）` }
  } catch (e) {
    return null
  }
}

/** 从两张 raw gray（1920×1080）算分区统计 —— 纯内存，不反复调 ffmpeg。 */
export function statsFrom(a, b, rows = null) {
  const band = rows ? rows.topEnd : Math.floor(H * NON_TEXT_BAND)
  const botStart = rows ? rows.botStart : H - Math.floor(H * NON_TEXT_BAND)
  let s = 0, n = 0, o32 = 0
  let sn = 0, nn = 0, on32 = 0
  for (let y = 0; y < H; y++) {
    const inBand = y < band || y >= botStart
    for (let x = 0; x < W; x++) {
      const d = Math.abs(a[y * W + x] - b[y * W + x])
      s += d; n++; if (d > 32) o32++
      if (inBand) { sn += d; nn++; if (d > 32) on32++ }
    }
  }
  return {
    fullMean: s / n, fullOver32: o32 * 100 / n,
    nonTextMean: sn / nn, nonTextOver32: on32 * 100 / nn,
  }
}

function selfTest() {
  const zero = Buffer.alloc(W * H, 0)
  const cases = [
    ['两张全同 → 必须过', zero, zero, true],
    ['全黑 vs 全白 → 必须红', zero, Buffer.alloc(W * H, 255), false],
  ]
  let bad = 0
  for (const [name, a, b, want] of cases) {
    const r = judge(statsFrom(a, b))
    const ok = r.pass === want
    if (!ok) bad++
    console.log(`  ${ok ? 'ok  ' : 'FAIL'} ${name}（pass=${r.pass}）`)
  }
  // 分区必须独立生效：只有中间带差 → 非文字区过关、整图红
  const mid = Buffer.alloc(W * H, 0)
  const band = Math.floor(H * NON_TEXT_BAND)
  for (let y = band; y < H - band; y++) for (let x = 0; x < W; x++) mid[y * W + x] = 120
  const s = statsFrom(zero, mid)
  const onlyMid = s.nonTextMean < THRESHOLDS.nonTextMean && s.fullMean > THRESHOLDS.fullMean
  console.log(`  ${onlyMid ? 'ok  ' : 'FAIL'} 分区独立：只有中间带差时非文字区过关（${s.nonTextMean.toFixed(2)}）、整图判红（${s.fullMean.toFixed(2)}）`)
  if (!onlyMid) bad++
  console.log(bad ? `\n❌ 自检失败 ${bad} 项` : '\n✅ 自检通过（3 项）')
  process.exit(bad ? 1 : 0)
}

if (has('self-test')) selfTest()

const EXE = opt('exe', 'F:\\para\\Code\\V-Trim\\target\\cl\\debug\\vtrim.exe')
const FONT = opt('font', 'C:\\Windows\\Fonts\\msyh.ttc')
const WORK = opt('work', path.join(process.env.TEMP || '.', 'vtrim-cover-gate'))
const PROJECT = opt('project', process.env.VTEDIT_COVER_FIXTURE || '')
const skip = (why) => {
  console.log(`SKIP：${why}`)
  console.log('  · 需要：带 `cover-takumi` feature 的 vtrim.exe（cargo build -p vtrim-cli --features vtrim-polish/cover-takumi）')
  console.log('  · 需要：一个真精修工程（--project 或 VTEDIT_COVER_FIXTURE 指向含 polish.toml 的目录）')
  console.log('  · 需要：字体库里至少一个字体（Takumi 不读系统字体）与 ffmpeg 在 PATH')
  process.exit(has('require') ? 1 : 0)
}

if (!fs.existsSync(EXE)) skip(`找不到 vtrim.exe: ${EXE}`)
if (!PROJECT || !fs.existsSync(path.join(PROJECT, 'polish.toml'))) skip(`找不到精修工程: ${PROJECT || '(未指定)'}`)
const fontDir = path.join(path.dirname(EXE), 'resources', 'fonts')
if (!fs.existsSync(fontDir) || fs.readdirSync(fontDir).length === 0) skip(`字体库是空的: ${fontDir}`)
const ff = spawnSync('ffmpeg', ['-version'], { stdio: 'ignore' })
if (ff.status !== 0) skip('PATH 里没有 ffmpeg')

// 硬链接副本（绝不动样本本体）；同卷才行，跨卷退化成复制
fs.rmSync(WORK, { recursive: true, force: true })
const linkTree = (src, dst) => {
  fs.mkdirSync(dst, { recursive: true })
  for (const e of fs.readdirSync(src, { withFileTypes: true })) {
    const s = path.join(src, e.name), d = path.join(dst, e.name)
    if (e.isDirectory()) linkTree(s, d)
    else if (e.isFile()) { try { fs.linkSync(s, d) } catch { fs.copyFileSync(s, d) } }
  }
}
const run = (dir, tag, env) => {
  for (const f of fs.readdirSync(dir)) if (/^cover(_\d+)?\.jpg$/.test(f)) fs.rmSync(path.join(dir, f))
  const log = fs.openSync(path.join(WORK, tag + '.log'), 'w') // fd，不用管道
  const t = Date.now()
  const r = spawnSync(EXE, ['polish', 'cover', dir], { stdio: ['ignore', log, log], env: { ...process.env, ...env } })
  fs.closeSync(log)
  const cover = path.join(dir, 'cover.jpg')
  if (r.status !== 0 || !fs.existsSync(cover)) {
    console.log(`❌ ${tag} 出封面失败（exit ${r.status}）：` + fs.readFileSync(path.join(WORK, tag + '.log'), 'utf8').split('\n').slice(-6).join('\n'))
    process.exit(1)
  }
  return { ms: Date.now() - t, cover, size: fs.statSync(cover).size }
}
const gray = (img, out) => {
  const r = spawnSync('ffmpeg', ['-y', '-loglevel', 'error', '-i', img, '-f', 'rawvideo', '-pix_fmt', 'gray', out], { stdio: 'ignore' })
  if (r.status !== 0) { console.log('❌ ffmpeg 读图失败 ' + img); process.exit(1) }
  return fs.readFileSync(out)
}

const A = path.join(WORK, 'chrome'), B = path.join(WORK, 'takumi')
linkTree(PROJECT, A); linkTree(PROJECT, B)
const a = run(A, 'chrome', {})
const b = run(B, 'takumi', { VTEDIT_COVER_RENDERER: 'takumi' })
const rows = textRows(PROJECT)
const s = statsFrom(gray(a.cover, path.join(WORK, 'a.gray')), gray(b.cover, path.join(WORK, 'b.gray')), rows)
if (rows) console.log('判据口径：' + rows.note + ' —— 非文字区取矩形之外')
else console.log('判据口径：读不到 polish.toml，退回"上下 20%"')

console.log(`工程 ${PROJECT}`)
console.log(`chrome ${a.ms}ms / ${a.size}B    takumi ${b.ms}ms / ${b.size}B`)
console.log(`非文字区(上下 ${NON_TEXT_BAND * 100}%)：均值 ${s.nonTextMean.toFixed(2)}  >32 ${s.nonTextOver32.toFixed(2)}%`)
console.log(`整图：                  均值 ${s.fullMean.toFixed(2)}  >32 ${s.fullOver32.toFixed(2)}%`)
const r = judge(s)
if (!r.pass) { for (const x of r.bad) console.log('  ❌ ' + x); console.log(`\n❌ 超阈值（非文字区 ≤${THRESHOLDS.nonTextMean}、整图 ≤${THRESHOLDS.fullMean}）`); process.exit(1) }
console.log('\n✅ 在阈值内')
