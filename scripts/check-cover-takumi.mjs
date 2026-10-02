// 封面渲染门禁（**默认：takumi ↔ 冻结参照帧**）。
//
// ⚠️ 2026-10-01 起：Chrome 链路正在被删除，参照改为**冻结参照帧**
// （`scripts/fixtures/cover-golden/`，pixel 来自删除前最后一次 chrome 渲染）。
// 不带参数时仍是"活的 chrome ↔ takumi"对拍（chrome 还在的过渡期才有意义）。
//
// 为什么要"分区"判据：实测（docs/plan/takumi-cover-capability.md §10–§11）
//   · **非文字区**（背景/帧层/渐变/暗角/扫描线/滤镜/图层栈）：**≤1.6/255**（逐像素级）；
//   · **标题所在band**：19~49/255 —— 那是 **Takumi 与 Chrome/Skia 的字形栅格化固有差**，
//     适配层改不了（双层标题试过，19.57 → 19.93 没改善）。
// 拿全图均值一刀切只会让这道门永远红或永远松，所以按区判、阈值各自标定。
//
// 用法：
//   node scripts/check-cover-takumi.mjs [--project <精修工程目录>] [--exe <yekitrim.exe>]
//        [--work <临时目录>] [--font <字体文件>] [--require] [--self-test]
//        [--capture-golden <目录>] [--golden <目录>]
//   · 缺工程/缺二进制（没编 cover-takumi）/缺字体 → **SKIP** 并打印要准备什么（`--require` 时算失败）。
//   · 阈值与标定依据写在 THRESHOLDS 旁；超阈值 **exit 1**（报告照留）。
//   · `--self-test` 自证判据本身（不再渲染）：全 0 必须过、大差必须红、分区必须独立生效。
//   · `--mutate` **变异自证**：拿冻结参照帧人为造偏差，证明判据真的会红（也真的会绿）。秒级。
//
// **冻结参照帧**（2026-10-01，Chrome 链路删除前；做法抄 `scripts/yekitrim-compare.mjs` 的先例）：
//   · `--capture-golden <目录>`：跑一次 chrome 通道，把产物**冻结**成参照帧 + `golden.json`。
//   · `--golden <目录>`：**完全不渲染 chrome**（它即将被删），只把 takumi 的产物与冻结帧对拍。
//   ⚠️ 为什么必须"先冻结再删"：这道门禁原本的参照是**活的 chrome 渲染**。删掉 chrome 之后
//   参照就没了 —— 而"已经渲染过的那一版就是当时的可信参照"，冻结下来即可（q=2 JPEG 肉眼无损，
//   但带一点编码噪声 ⇒ 黄金帧模式的基线会比现在略高，阈值余量足够）。
//   ⚠️ 冻结的是 **PNG 源图转的 JPEG**（不直接存 chrome 的 `cover.jpg`）：chrome 那份本身就是
//   JPEG，再转一次等于**二次编码**，噪声会叠；从它出封面时的 PNG 中转一次更干净。
import { spawnSync } from 'node:child_process'
import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const __dirname = path.dirname(fileURLToPath(import.meta.url))

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

// **变异自证**（`--mutate`）：证明这道门禁**真的会红**，不是永远绿的摆设。
// 做法：拿冻结参照帧，人为地把它和"正确的 takumi 产物"错开（整体位移/提亮），
// 判据必须变红；再确认原样的那一对是绿的。**不渲染**，纯内存 —— 秒级。
if (has('mutate')) {
  const refPath = path.join(__dirname, 'fixtures', 'cover-golden', 'ref-cover.jpg')
  if (!fs.existsSync(refPath)) { console.log(`❌ 找不到参照帧: ${refPath}`); process.exit(2) }
  const tmp = path.join(process.env.TEMP || '.', 'vtrim-cover-mutate')
  fs.mkdirSync(tmp, { recursive: true })
  // ⚠️ 不能调 `gray()`：它定义在下面（`const` 的 TDZ），`--mutate` 在模块顶层跑到这儿就炸。
  // 自己用 ffmpeg 转一次 rawvideo —— 几行代码换掉一个"顺序耦合"，值。
  const baseGray = (() => {
    const out = path.join(tmp, 'base.gray')
    const r = spawnSync('ffmpeg', ['-y', '-loglevel', 'error', '-i', refPath, '-f', 'rawvideo', '-pix_fmt', 'gray', out], { stdio: 'ignore' })
    if (r.status !== 0) { console.log('❌ ffmpeg 读参照帧失败'); process.exit(2) }
    return fs.readFileSync(out)
  })()
  const rows = JSON.parse(fs.readFileSync(path.join(__dirname, 'fixtures', 'cover-golden', 'golden.json'), 'utf8')).rows
  const okStats = statsFrom(baseGray, baseGray, rows)
  const shifted = Buffer.from(baseGray)
  for (let y = 0; y < H; y++) for (let x = 0; x < W; x++) {
    // 整个非文字区提亮 60（远超"非文字区 ≤4"的阈值）
    const inBand = y < rows.topEnd || y >= rows.botStart
    if (inBand) shifted[y * W + x] = Math.min(255, shifted[y * W + x] + 60)
  }
  const badStats = statsFrom(baseGray, shifted, rows)
  const okPass = judge(okStats).pass
  const badPass = judge(badStats).pass
  console.log(`  原样（自己比自己）→ pass=${okPass}（应为 true）`)
  console.log(`  非文字区提亮 60   → pass=${badPass}（应为 false）；非文字区均值 ${badStats.nonTextMean.toFixed(2)}`)
  const good = okPass && !badPass
  console.log(good ? '\n✅ 变异自证通过：判据能红也能绿'
    : '\n❌ 变异自证失败：判据区分不出好坏')
  process.exit(good ? 0 : 1)
}

const EXE = opt('exe', 'F:\\para\\Code\\V-Trim\\target\\cl\\debug\\yekitrim.exe')
const FONT = opt('font', 'C:\\Windows\\Fonts\\msyh.ttc')
const WORK = opt('work', path.join(process.env.TEMP || '.', 'vtrim-cover-gate'))
const PROJECT = opt('project', process.env.VTEDIT_COVER_FIXTURE || '')
const skip = (why) => {
  console.log(`SKIP：${why}`)
  console.log('  · 需要：带 `cover-takumi` feature 的 yekitrim.exe（cargo build -p yekitrim-cli --features yekitrim-polish/cover-takumi）')
  console.log('  · 需要：一个真精修工程（--project 或 VTEDIT_COVER_FIXTURE 指向含 polish.toml 的目录）')
  console.log('  · 需要：字体库里至少一个字体（Takumi 不读系统字体）与 ffmpeg 在 PATH')
  process.exit(has('require') ? 1 : 0)
}

if (!fs.existsSync(EXE)) skip(`找不到 yekitrim.exe: ${EXE}`)
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
// ⚠️ **两条腿都必须显式给 `VTEDIT_COVER_RENDERER`**，一个都不能靠"默认值"。
// 2026-10-01 踩过：默认通道由 chrome 改成 takumi 之后，chrome 那条传的是 `{}`（吃默认）
// ⇒ `chrome` 腿其实又渲染了一遍 takumi ⇒ 冻结出来的"参照帧"是 takumi 的，
// 而门禁显示 0.32/255（自比自当然几乎为 0，看着像"完美通过"，实为假绿）。
// 判据：PSNR(ref, chrome)=20.7dB vs PSNR(ref, takumi)=50.2dB。
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

const CAPTURE = opt('capture-golden', '')
// ⚠️ `--golden` 不给目录时用**入库的参照帧**（`scripts/fixtures/cover-golden/`）——
// 那正是"chrome 删掉之后还能跑"的全部意义所在。给目录 = 换一套参照（比如新工程）。
const GOLDEN = has('golden') ? (opt('golden', '') || path.join(__dirname, 'fixtures', 'cover-golden')) : ''
if (CAPTURE && GOLDEN) { console.log('❌ --capture-golden 与 --golden 只能用一种'); process.exit(2) }

const rows = textRows(PROJECT)
if (rows) console.log('判据口径：' + rows.note + ' —— 非文字区取矩形之外')
else console.log('判据口径：读不到 polish.toml，退回"上下 20%"')
console.log(`工程 ${PROJECT}`)

const B = path.join(WORK, 'takumi')
linkTree(PROJECT, B)
const b = run(B, 'takumi', { VTEDIT_COVER_RENDERER: 'takumi' })
const bGray = gray(b.cover, path.join(WORK, 'b.gray'))

// ---------------------------------------------------------------------------
// **冻结参照帧**（--capture-golden）：跑一次 chrome，把产物存成参照帧 + golden.json。
// ⚠️ 必须在 chrome 还被删之前做 —— 删完之后这道门禁的参照就没了。
// ---------------------------------------------------------------------------
if (CAPTURE) {
  const A = path.join(WORK, 'chrome')
  linkTree(PROJECT, A)
  const a = run(A, 'chrome', { VTEDIT_COVER_RENDERER: 'chrome' })
  // 经 PNG 中转一次再存 JPEG（chrome 那份本身就是 q=85 JPEG，直接再存等于二次编码叠噪声）
  const srcPng = path.join(WORK, 'ref-src.png')
  let rr = spawnSync('ffmpeg', ['-v', 'error', '-y', '-i', a.cover, srcPng], { stdio: 'ignore' })
  if (rr.status !== 0) { console.log('❌ 转 PNG 失败（冻结不出干净的参照帧）'); process.exit(1) }
  fs.mkdirSync(CAPTURE, { recursive: true })
  const dst = path.join(CAPTURE, 'ref-cover.jpg')
  rr = spawnSync('ffmpeg', ['-v', 'error', '-y', '-i', srcPng, '-q:v', '2', dst], { stdio: 'ignore' })
  if (rr.status !== 0) { console.log('❌ 写参照帧失败: ' + dst); process.exit(1) }
  fs.writeFileSync(path.join(CAPTURE, 'golden.json'), JSON.stringify({
    fixture: path.basename(PROJECT),
    captured_at: new Date().toISOString(),
    frozen_from: 'vtrim polish cover --renderer chrome（Chrome 链路删除前最后一次渲染）',
    width: W, height: H,
    ref_file: path.basename(dst),
    rows: rows || null,
    note: '参照帧是 chrome 通道的产物（q=2 JPEG）。--golden 模式拿它当基线，不再渲染 chrome。',
  }, null, 2) + '\n', 'utf8')
  console.log(`\n✔ 已冻结参照帧 -> ${dst}（chrome ${a.ms}ms / ${a.size}B）`)
  console.log('  之后用 `--golden <该目录>` 复跑，**不再需要 chrome**。')
  process.exit(0)
}

// ---------------------------------------------------------------------------
// **对冻结帧**（--golden）：完全不渲染 chrome —— 它已被删除。
// ---------------------------------------------------------------------------
if (GOLDEN) {
  const ref = path.join(GOLDEN, 'ref-cover.jpg')
  if (!fs.existsSync(ref)) { console.log(`❌ 找不到参照帧: ${ref}（先用 --capture-golden 冻结）`); process.exit(2) }
  const meta = (() => { try { return JSON.parse(fs.readFileSync(path.join(GOLDEN, 'golden.json'), 'utf8')) } catch { return null } })()
  const refRows = (meta && meta.rows) || rows
  const sg = statsFrom(gray(ref, path.join(WORK, 'ref.gray')), bGray, refRows)
  console.log(`参照 ${ref}${meta && meta.captured_at ? '（冻结于 ' + meta.captured_at + '）' : ''}`)
  console.log(`takumi ${b.ms}ms / ${b.size}B`)
  console.log(`非文字区(上下 ${NON_TEXT_BAND * 100}%)：均值 ${sg.nonTextMean.toFixed(2)}  >32 ${sg.nonTextOver32.toFixed(2)}%`)
  console.log(`整图：                  均值 ${sg.fullMean.toFixed(2)}  >32 ${sg.fullOver32.toFixed(2)}%`)
  const rg = judge(sg)
  if (!rg.pass) { for (const x of rg.bad) console.log('  ❌ ' + x); console.log(`\n❌ 超阈值（非文字区 ≤${THRESHOLDS.nonTextMean}、整图 ≤${THRESHOLDS.fullMean}）`); process.exit(1) }
  console.log('\n✅ 在阈值内（与冻结参照帧比）')
  process.exit(0)
}

// 默认：活的双通道对拍（chrome 还在时才有意义；chrome 删除后请用 --golden）
const A = path.join(WORK, 'chrome')
linkTree(PROJECT, A)
const a = run(A, 'chrome', { VTEDIT_COVER_RENDERER: 'chrome' })
const s = statsFrom(gray(a.cover, path.join(WORK, 'a.gray')), bGray, rows)
console.log(`chrome ${a.ms}ms / ${a.size}B    takumi ${b.ms}ms / ${b.size}B`)
console.log(`非文字区(上下 ${NON_TEXT_BAND * 100}%)：均值 ${s.nonTextMean.toFixed(2)}  >32 ${s.nonTextOver32.toFixed(2)}%`)
console.log(`整图：                  均值 ${s.fullMean.toFixed(2)}  >32 ${s.fullOver32.toFixed(2)}%`)
const r = judge(s)
if (!r.pass) { for (const x of r.bad) console.log('  ❌ ' + x); console.log(`\n❌ 超阈值（非文字区 ≤${THRESHOLDS.nonTextMean}、整图 ≤${THRESHOLDS.fullMean}）`); process.exit(1) }
console.log('\n✅ 在阈值内')
