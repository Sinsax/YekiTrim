#!/usr/bin/env node
// **贴纸选择器 · 真 Chrome headless 几何判据**（2026-10-15）。
//
// # 为什么要它
//
// 「行递进 + 固定高度」这件事**只有布局引擎说了算**：整块 330 / 路径 78 / 正文 196 全是 CSS 写死的数，
// 静态读源码的判据（check-sticker-picker-flow.mjs）只能证明「写了这几个数」，
// 证明不了「切页面不跳」「正文里没横向溢出」「网格格子真是 3 列」。这一条**挂真组件、在真 Chrome 里量**。
//
// # 它怎么跑
//
// 夹具在 scripts/fixtures/sticker-picker/（真组件 + 假数据，宿主 = 真 .props-panel：232px 外框 / 内容 210px）：
//   vite 构建（用 webui 的 node_modules）→ 本地静态服务（顺带伪造 /api/polish/emoji-catalog 与
//   贴纸缩略图）→ Chrome --headless=new + CDP 打开页面 → 逐个状态**真点真量** getBoundingClientRect。
//
// 两段：
//   ① 贴纸选择器十种状态：整块 330 / 路径 78 / 正文 192 / 3 行 × 24 / 标题行不换行 / 高度零跳动。
//   ② 属性面板「标配行」：标签从 TimelineEditor.vue 抽、行 class 也从那儿抽（不抄一份）⇒
//      字幕/屏蔽那套（入点/出点/定位/拆分，4 颗）与事件/分段那套（3 颗）各量一次：4 颗 48px = 192px
//      （+3×4 间距）= 204px ⇒ **一行**（210px 内容宽）。⚠️「设为入点/设为出点」每颗 72px ⇒ 旧写法 252px
//      会折成两行；「复制」按钮已按用户口径删掉（连只被它用的 duplicateSelected 一起）。
//
// 环境：本机装了 Chrome（或 Edge）就跑，找不到浏览器**SKIP 并退出 0**（本仓判据不许因为
// 「这台没装浏览器」变红）。环境变量 CHROME_PATH 可显式指定。
//
// 用法：node scripts/check-sticker-picker-layout.mjs [--keep]
//   --keep  保留 Chrome 的临时 profile 目录（排查用）
//   截图固定写 .workbuddy/sticker-picker-headless-*.png（已 gitignore）

import { spawn } from 'node:child_process'
import { existsSync, mkdtempSync, rmSync, readFileSync, writeFileSync } from 'node:fs'
import { createServer } from 'node:http'
import { tmpdir } from 'node:os'
import { dirname, extname, join } from 'node:path'
import { fileURLToPath } from 'node:url'
import { EMOJI } from './fixtures/sticker-picker/fixture.js'

const HERE = dirname(fileURLToPath(import.meta.url))
const ROOT = join(HERE, '..')
const WEBUI = join(ROOT, 'webui')
const FIXTURE = join(HERE, 'fixtures', 'sticker-picker')
const OUT = join(WEBUI, 'node_modules', '.sticker-picker-harness')
const SHOT_DIR = join(ROOT, '.workbuddy')
const KEEP = process.argv.includes('--keep')

const sleep = (ms) => new Promise((r) => setTimeout(r, ms))
let pass = 0
let fail = 0
const failures = []
function ok(cond, what) {
  if (cond) { pass += 1; console.log('ok   ' + what) } else { fail += 1; failures.push(what); console.log('FAIL ' + what) }
}

/* ---------------- ① 找浏览器 ---------------- */
function findChrome() {
  const la = process.env.LOCALAPPDATA || ''
  const pf = process.env['ProgramFiles'] || 'C:/Program Files'
  const pf86 = process.env['ProgramFiles(x86)'] || 'C:/Program Files (x86)'
  const cands = [
    process.env.CHROME_PATH,
    join(pf, 'Google/Chrome/Application/chrome.exe'),
    join(pf86, 'Google/Chrome/Application/chrome.exe'),
    la ? join(la, 'Google/Chrome/Application/chrome.exe') : '',
    join(pf, 'Microsoft/Edge/Application/msedge.exe'),
    join(pf86, 'Microsoft/Edge/Application/msedge.exe'),
    la ? join(la, 'Microsoft/Edge/Application/msedge.exe') : '',
    '/usr/bin/google-chrome',
    '/usr/bin/chromium',
    '/usr/bin/chromium-browser',
    '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome',
  ].filter(Boolean)
  return cands.find((p) => existsSync(p)) || null
}

/* ---------------- ② 构建夹具 ---------------- */
function run(cmd, args, cwd) {
  return new Promise((res) => {
    const p = spawn(cmd, args, { cwd, stdio: 'inherit' })
    p.on('exit', (code) => res(code === 0))
  })
}

/* ---------------- ③ 静态服务（顺带伪造两条 /api） ---------------- */
const PNG_1x1 = Buffer.from('iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8DwHwAFAAH/q842iQAAAABJRU5ErkJggg==', 'base64')
function startServer() {
  const srv = createServer((req, res) => {
    const u = new URL(req.url, 'http://127.0.0.1')
    if (u.pathname === '/api/polish/emoji-catalog') {
      res.writeHead(200, { 'Content-Type': 'application/json' })
      res.end(JSON.stringify({ ok: true, groups: EMOJI }))
      return
    }
    if (u.pathname === '/api/polish/sticker-file') {
      // 故意让一张图 404：验证「图挂了」那一格靠占位图标撑住、高度不变
      if ((u.searchParams.get('name') || '').indexOf('meme_3') >= 0) { res.writeHead(404); res.end('nope'); return }
      res.writeHead(200, { 'Content-Type': 'image/png' })
      res.end(PNG_1x1)
      return
    }
    const rel = u.pathname === '/' ? '/index.html' : u.pathname
    const file = join(OUT, decodeURIComponent(rel))
    if (!file.startsWith(OUT) || !existsSync(file)) { res.writeHead(404); res.end(''); return }
    const ext = extname(file)
    const type = ext === '.js' ? 'text/javascript' : ext === '.css' ? 'text/css' : ext === '.html' ? 'text/html' : 'application/octet-stream'
    res.writeHead(200, { 'Content-Type': type })
    res.end(readFileSync(file))
  })
  return new Promise((res) => srv.listen(0, '127.0.0.1', () => res({ srv, port: srv.address().port })))
}

/* ---------------- ④ Chrome + CDP ---------------- */
function launchChrome(chrome) {
  const profile = mkdtempSync(join(tmpdir(), 'yt-headless-'))
  const child = spawn(chrome, [
    '--headless=new', '--disable-gpu', '--no-first-run', '--no-default-browser-check',
    '--disable-extensions', '--hide-scrollbars', '--mute-audio', '--window-size=300,400',
    '--user-data-dir=' + profile, '--remote-debugging-port=0', 'about:blank',
  ], { stdio: 'ignore' })
  return { child, profile }
}
async function debugPort(profile) {
  const f = join(profile, 'DevToolsActivePort')
  for (let i = 0; i < 150; i++) {
    if (existsSync(f)) {
      const t = readFileSync(f, 'utf8').split('\n')
      if (t[0] && t[1]) return Number(t[0])
    }
    await sleep(100)
  }
  return null
}
function connect(url) {
  return new Promise((res, rej) => {
    const ws = new WebSocket(url)
    const pending = new Map()
    let id = 0
    ws.addEventListener('open', () => res({
      send(method, params) {
        return new Promise((resolve) => { const i = ++id; pending.set(i, resolve); ws.send(JSON.stringify({ id: i, method, params: params || {} })) })
      },
      close() { try { ws.close() } catch (e) { /* 已关就别管 */ } },
    }))
    ws.addEventListener('error', () => rej(new Error('websocket 连不上 ' + url)))
    ws.addEventListener('message', (e) => {
      const m = JSON.parse(e.data)
      if (m.id && pending.has(m.id)) { const f = pending.get(m.id); pending.delete(m.id); f(m) }
    })
  })
}

/* ---------------- 主流程 ---------------- */
const chrome = findChrome()
if (!chrome) {
  console.log('SKIP 没找到 Chrome / Edge（可用 CHROME_PATH 指定）—— 几何判据跳过，不算失败')
  process.exit(0)
}
console.log('浏览器：' + chrome)

if (!(await run(process.execPath, [join(WEBUI, 'node_modules', 'vite', 'bin', 'vite.js'), 'build', '--config', join(FIXTURE, 'vite.config.mjs'), '--logLevel', 'warn'], WEBUI))) {
  console.log('FAIL 夹具构建失败（vite build）')
  process.exit(1)
}

const { srv, port } = await startServer()
const { child, profile } = launchChrome(chrome)
const dPort = await debugPort(profile)
if (!dPort) {
  console.log('FAIL Chrome 没起来（DevToolsActivePort 一直不出）')
  try { child.kill() } catch (e) { /* ignore */ }
  process.exit(1)
}

const targets = await (await fetch('http://127.0.0.1:' + dPort + '/json/list')).json()
const page = targets.find((t) => t.type === 'page')
const cdp = await connect(page.webSocketDebuggerUrl)
await cdp.send('Page.enable')
await cdp.send('Runtime.enable')
await cdp.send('Page.navigate', { url: 'http://127.0.0.1:' + port + '/index.html' })

const ev = async (expr) => {
  const r = await cdp.send('Runtime.evaluate', { expression: expr, returnByValue: true, awaitPromise: true })
  if (r.result && r.result.exceptionDetails) throw new Error(String(JSON.stringify(r.result.exceptionDetails)).slice(0, 300))
  return r.result ? r.result.result.value : undefined
}
let booted = false
for (let i = 0; i < 120; i++) {
  if (await ev('!!window.__H')) { booted = true; break }
  await sleep(120)
}
ok(booted, '夹具页起来了（window.__H 在）')
if (!booted) {
  try { cdp.close() } catch (e) { /* ignore */ }
  try { child.kill() } catch (e) { /* ignore */ }
  srv.close()
  process.exit(1)
}

const shot = async (name) => {
  const r = await cdp.send('Page.captureScreenshot', { format: 'png' })
  writeFileSync(join(SHOT_DIR, 'sticker-picker-headless-' + name + '.png'), Buffer.from(r.result.data, 'base64'))
}

/** 每个状态：先真点（**每步之间要等 Vue 刷一帧**，否则 reset 与 click 撞在同一次求值里），
 *  再量几何 + 抓可见文案（证明点真的推进了页面）。 */
const R = '__H.reset()'
const steps = [
  ['第1页 · 空库', ["__H.setLibError(''); __H.setLib([])", R], "__H.count('.srow') === 3 && __H.count('.body') === 1"],
  ['第1页 · 读失败', ["__H.setLibError('接口 500')"], "document.body.innerText.indexOf('贴纸库读取失败') >= 0"],
  ['第1页 · 有包', ["__H.setLibError(''); __H.setLib(__FIX.LIB)", R], "document.body.innerText.indexOf('ysyk') >= 0"],
  ['第2页 · 选分类', ["__H.click('.row-item', 0)"], "document.body.innerText.indexOf('正在选') >= 0"],
  ['第3页 · 网格', ["__H.click('.row-item', 1)"], "document.body.innerText.indexOf('害怕') >= 0"],
  ['第3页 · 已选图', ["__H.click('.cell', 2)"], "document.body.innerText.indexOf('害怕_3') >= 0"],
  ['第2页 · 未构建包', [R, "__H.click('.row-item', 2)"], "document.body.innerText.indexOf('只能手动选图') >= 0"],
  ['第3页 · 图挂了', ["__H.click('.row-item', 0)"], "__H.count('.grid3 .cell') >= 20"],
  ['第2页 · 表情分类', [R, "__H.click('.row-item', 3)"], "document.body.innerText.indexOf('笑脸') >= 0"],
  ['第3页 · 表情墙', ["__H.click('.row-item', 1)"], "__H.count('.em-cell') > 0"],
]
const heights = []
for (const [label, acts, expect] of steps) {
  for (const a of acts) { await ev(a); await sleep(140) }
  await sleep(60)
  const drove = await ev(expect)
  ok(drove, label + '：点击真的推进到了这一页')
  const m = await ev('window.__H.measure()')
  heights.push({ label, h: m.sp ? m.sp.h : -1 })
  console.log('     ' + label.padEnd(16, ' ') + ' 整块 ' + (m.sp ? m.sp.h : '-') + ' / 路径 ' + (m.stack ? m.stack.h : '-') +
    ' / 正文 ' + (m.body ? m.body.h : '-') + ' / 行 ' + m.rows.length + 'x' + (m.rows[0] ? m.rows[0].h : '-') +
    ' / 横向溢出 ' + m.overflowX + ' / 格子 ' + m.cells + (m.cellW ? 'x' + m.cellW : '') + ' / 面板宽 ' + (m.panel ? m.panel.w : '-'))
  ok(Math.abs((m.sp ? m.sp.h : 0) - 330) <= 1, label + '：整块高 = 330 ± 1（实得 ' + (m.sp ? m.sp.h : '-') + '）')
  ok(m.stack && Math.abs(m.stack.h - 78) <= 1, label + '：路径区高 = 78 ± 1（实得 ' + (m.stack ? m.stack.h : '-') + '）')
  ok(m.body && Math.abs(m.body.h - 192) <= 1, label + '：正文区高 = 192 ± 1（实得 ' + (m.body ? m.body.h : '-') + '）')
  ok(m.rows.length === 3, label + '：路径区恒 3 行（实得 ' + m.rows.length + '）')
  ok(m.rows.every((r) => Math.abs(r.h - 24) <= 0.6), label + '：每行 24（实得 ' + m.rows.map((r) => r.h).join('/') + '）')
  ok(m.overflowX <= 1, label + '：没有横向溢出（实得 ' + m.overflowX + '）')
  // ⚠️ 判据不写死标题行的 px（它随右侧按钮高度走）：真正的**不变量**是"内容装得进盒子里、
  //    且没有横向溢出" —— 换行/被裁都会让 scroll 高 > 盒高（这一条就是这么抓到 18px 盒装 22px 按钮的）。
  ok(m.secH && m.secH.sh <= m.secH.h + 1 && m.secH.sw <= m.secH.cw + 1,
    label + '：标题行**没被挤爆**（内容 ' + (m.secH ? m.secH.sh : '-') + ' ≤ 盒高 ' +
    (m.secH ? m.secH.h : '-') + '；横向 ' + (m.secH ? m.secH.sw + ' vs ' + m.secH.cw : '-') + '）')
  ok(m.panel && Math.abs(m.panel.w - 232) <= 1, label + '：宿主外框 232（.props-panel 真值；内容 210，实得 ' + (m.panel ? m.panel.w : '-') + '）')
  if (label.indexOf('第3页 · 已选图') >= 0) {
    ok(m.cells === 12, '第3页：害怕 分类 12 张（实得 ' + m.cells + '）')
    ok(m.cellW >= 62 && m.cellW <= 72, '第3页：3 列格子宽 ≈ 67（210px 内容宽 / 3，实得 ' + m.cellW + '）')
    await shot('p3')
  }
  if (label.indexOf('表情墙') >= 0) {
    ok(m.emCells > 0 && m.emW >= 22, '表情墙：8 列格子在 210px 里没被挤没（实得 ' + m.emCells + ' 格 x ' + m.emW + 'px）')
    await shot('emoji')
  }
}

const uniq = [...new Set(heights.map((x) => x.h))]
ok(uniq.length === 1, '所有状态整块高度**完全一致**（不同的高度集合 ' + JSON.stringify(uniq) + '）')

/* ---------------- ② 属性面板「标配行」：标签从真源码抽，量它是不是一行 ----------------
   用户口径：「整合为一行」+「设为」删去。`.props-panel` 只有 232px（padding 10 ⇒ 内容 210px），
   而「设为入点/设为出点」每颗 72px ⇒ 四颗 252px **折成两行**；两字标签每颗 48px ⇒ 四颗 204px 一行。
   ⚠️ 标签与行的 class 都从 TimelineEditor.vue 里**抽**（不在这里抄一份，免得改标签后判据失真）。 */
{
  const TL = readFileSync(join(ROOT, 'webui', 'src', 'components', 'TimelineEditor.vue'), 'utf8')
  // ⚠️ 别拿 `<div class="row mt-6 gap-4">` 当锚点：**成片段**那一行也是这个 class（先出现）——
  //    锚点落在「入点」按钮的 @click 上，再往前找它所属的 row。
  const anchor = TL.indexOf('@click="setInPoint"')
  const i = anchor < 0 ? -1 : TL.lastIndexOf('<div class="row', anchor)
  const spec = (() => {
    if (i < 0) return null
    const marker = '<div class="row'
    const block = TL.slice(i, TL.indexOf('</div>', anchor))
    const btns = [...block.matchAll(/<button([^>]*)>([^<]+)<\/button>/g)].map((m) => ({ attrs: m[1], text: m[2].trim() }))
    if (!btns.length) return null
    const isSplit = (b) => b.attrs.indexOf('canSplit') >= 0
    const isDup = (b) => b.attrs.indexOf("selected.kind === 'event'") >= 0
    return {
      rowClass: (marker.match(/class="([^"]+)"/) || [])[1],
      all: btns.map((b) => b.text),
      // 拆分是**唯一**带条件的（只给字幕/屏蔽）⇒ 字幕那套 4 颗、事件/分段那套 3 颗
      base: btns.filter((b) => !isSplit(b) && !isDup(b)).map((b) => b.text),
      nSplit: btns.filter(isSplit).length,
      nDup: btns.filter(isDup).length,
    }
  })()
  ok(!!spec, '② 标配行：抽得到那条 row（结构变了要改本判据）')
  if (spec) {
    ok(spec.all.indexOf('复制') < 0,
      '② 标配行里**没有「复制」**（2026-10-15 用户口径「不需要复制」；实得 ' + JSON.stringify(spec.all) + '）')
    ok(spec.nSplit === 1 && spec.nDup === 0,
      '② 标配行只给「拆分」留条件（字幕/屏蔽）⇒ 字幕那套 4 颗、其它 3 颗（实得 ' + spec.nSplit + ' / ' + spec.nDup + '）')
    await cdp.send('Page.navigate', { url: 'http://127.0.0.1:' + port + '/panel-row.html' })
    let up = false
    for (let k = 0; k < 120; k++) { if (await ev('!!window.__ROW')) { up = true; break } await sleep(120) }
    ok(up, '② 标配行夹具页起来了（同一套 headless 管道）')
    if (up) {
      for (const [name, labels, want] of [['字幕/屏蔽（带拆分）', spec.all, 4], ['事件/分段（不带）', spec.base, 3]]) {
        await ev('window.__ROW.set(' + JSON.stringify({ rowClass: spec.rowClass, labels }) + ')')
        await sleep(60)
        const m = await ev('window.__ROW.measure()')
        console.log('     标配行 ' + name + '：' + m.n + ' 颗 ' + m.btns.map((b) => b.t + '=' + b.w).join(' / ') +
          ' ⇒ 合计 ' + m.sum + 'px（行宽 ' + m.clientW + '）· 行高 ' + m.rowH + '（按钮 ' + m.btnH + '）')
        ok(m.n === want, '② ' + name + '：' + want + ' 颗按钮（实得 ' + m.n + '）')
        ok(m.rowH <= m.btnH + 1, '② ' + name + '：**一行**放得下（行高 ' + m.rowH + ' vs 按钮 ' + m.btnH + '）')
        ok(m.scrollW <= m.clientW + 1, '② ' + name + '：没横向溢出（' + m.scrollW + ' vs ' + m.clientW + '）')
        ok(m.sum <= m.clientW + 1, '② ' + name + '：按钮合计 ≤ 行宽（' + m.sum + ' ≤ ' + m.clientW + '）')
        if (name.indexOf('字幕') >= 0) await shot('panel-row')
      }
    }
  }

  // ③ 属性面板「字段密度」（2026-10-16 的两列网格）
  //
  // 判据形状：**同一份内容量两遍** —— `is-dense`（现行 app.css 的两列网格）与 `is-legacy`
  // （夹具里复刻的旧规则：标签换行 + 控件独占一行）。"现在多高"是恒真判据；量**新旧之比**
  // 才说得上"排版真的变紧了"，而且它同时钉住"别把两列改回去"。
  //
  // ⚠️ **三轴（排版/过渡/动效）+ 角度那四个标签随文字层退役**（2026-10-20，文字元素面板整块删除）。
  //    现在 `TimelineEditor.vue` 里只剩贴纸/字幕面板的「角度」，所以这里**不再要求 4 个标签**；
  //    如果哪天文字元素面板回来，`axisLabels.length >= 4` 会重新出现，届时把这条恢复成硬断言。
  //    ③b 那组通用行形状判据**不依赖文字面板**，继续跑。
  const axisLabels = (() => {
    const found = [...TL.matchAll(/<label>(排版|过渡|动效|角度)<\/label>/g)].map((m) => m[1])
    return [...new Set(found)]
  })()
  if (axisLabels.length >= 4) {
    ok(true, '③ 抽得到三轴 + 角度四个标签（实得 ' + JSON.stringify(axisLabels) + '）')
  } else {
    console.log('     · ③ 文字元素三轴面板已退役（实得标签 ' + JSON.stringify(axisLabels) + '，只需「角度」）')
  }
  await cdp.send('Page.navigate', { url: 'http://127.0.0.1:' + port + '/panel-form.html' })
  let formUp = false
  for (let k = 0; k < 120; k++) { if (await ev('!!window.__FORM')) { formUp = true; break } await sleep(120) }
  ok(formUp, '③ 字段密度夹具页起来了（同一套 headless 管道）')
  if (formUp) {
    await ev('window.__FORM.set(' + JSON.stringify(axisLabels) + ')')
    await sleep(80)
    const m = await ev('window.__FORM.measure()')
    console.log('     字段块：两列网格 ' + m.dense.h + 'px（' + m.dense.labels.length + ' 个字段）' +
      ' vs 旧布局 ' + m.legacy.h + 'px ⇒ 压矮 ' + (100 - Math.round(m.dense.h / m.legacy.h * 100)) + '%')
    console.log('     下拉最窄 ' + m.dense.minSel + 'px（旧布局 ' + m.legacy.minSel + 'px）· 横向 ' + m.dense.scrollW + '/' + m.dense.clientW)
    // ⚠️ 阈值取**实测口径**（4 字段 + 1 提示：214 vs 292 ⇒ −27%），不取「应该省一半」的想当然：
    //    两列省下的是**每个字段的标签行**（约 22px），下拉本身的高度与提示行省不掉。
    ok(m.dense.h <= m.legacy.h * 0.75, '③ 两列网格把这块压矮 ≥25%（' + m.dense.h + ' vs ' + m.legacy.h + '）')
    // 文字面板退役后夹具里可能一个 <select> 都没有（minSel=0）—— 只在三轴面板还在时要求宽度。
    if (axisLabels.length >= 4) {
      ok(m.dense.minSel >= 120, '③ 下拉最窄 ≥120px（实得 ' + m.dense.minSel + '）')
    } else {
      console.log('     · ③ 下拉宽度跳过：本夹具已无文字面板的三轴下拉（minSel=' + m.dense.minSel + '）')
    }
    ok(m.dense.scrollW <= m.dense.clientW + 1, '③ 面板没横向溢出（' + m.dense.scrollW + ' vs ' + m.dense.clientW + '）')
    ok(m.dense.labels.every((l) => l.w + 0.5 >= l.need), '③ 标签没被裁（44px 标签列：' + JSON.stringify(m.dense.labels.map((l) => l.t + '=' + l.w + '/' + l.need)) + '）')
    ok(m.dense.w <= 233, '③ 面板仍是 232px 外宽（实得 ' + m.dense.w + '）')
    console.log('     角度行右边界 ' + m.dense.angleRight + ' / 滑杆 ' + m.dense.rangeRight + '（面板右边界 ' + m.dense.panelRight + '）')
    ok(m.dense.angleRight !== null && m.dense.angleRight <= m.dense.panelRight + 1,
      '③ 角度行（滑杆）不越出面板右边界（' + m.dense.angleRight + ' vs ' + m.dense.panelRight + '）')
    console.log('     标签盒 ' + JSON.stringify(m.dense.angleLabelBox) + ' · 角度行盒 ' + JSON.stringify(m.dense.angleRowBox))
    ok(m.dense.angleInline === true, '③ 角度行与它的标签**同一行**（两列网格；实得 ' + m.dense.angleInline + '）')
    await shot('panel-form')
  }

  // ③b **面板里每种行形状都不能被网格摆坏**（两列网格是全局的；只量三轴块不够）。
  //
  // 判据是**通用不变量**，不是某个数字：不横向溢出 · 控件列里的控件仍够宽 · 标签不裁字 ·
  // textarea 拿到整行（它在 160px 的控件列里没法用）。
  if (formUp) {
    await ev('window.__FORM.shapes()')
    await sleep(80)
    const s = (await ev('window.__FORM.measure()')).dense
    console.log('     七种行形状：面板高 ' + s.h + 'px · 横向 ' + s.scrollW + '/' + s.clientW)
    ok(s.scrollW <= s.clientW + 1, '③b 七种行形状都没横向溢出（' + s.scrollW + ' vs ' + s.clientW + '）')
    ok(s.labels.every((l) => l.w + 0.5 >= l.need), '③b 标签没被裁（' + JSON.stringify(s.labels.map((l) => l.t)) + '）')
    const tight = await ev('(() => { const p = document.querySelector(\'.props-panel.is-dense\'); const out = []; for (const el of p.querySelectorAll(\'select, input[type=text], input[type=number], input[type=color]\')) out.push({ tag: el.tagName.toLowerCase(), t: el.type || \'\', w: +el.getBoundingClientRect().width.toFixed(2) }); const ta = p.querySelector(\'textarea\'); return { ctl: out, ta: ta ? +ta.getBoundingClientRect().width.toFixed(2) : null }; })()')
    const narrow = tight.ctl.filter((c) => c.w < 120)
    ok(narrow.length === 0, '③b 控件列里的控件都 ≥120px 宽（窄的：' + JSON.stringify(narrow) + '）')
    ok(tight.ta !== null && tight.ta >= 200, '③b textarea 拿到整行（实得 ' + tight.ta + 'px）')
    await shot('panel-form-shapes')
  }
}

try { cdp.close() } catch (e) { /* ignore */ }
try { child.kill() } catch (e) { /* ignore */ }
srv.close()
if (!KEEP) { try { rmSync(profile, { recursive: true, force: true }) } catch (e) { /* Windows 上偶尔还占着，忽略 */ } }

console.log()
console.log('贴纸选择器 · headless 几何：' + pass + ' 项通过，' + fail + ' 项失败（截图 .workbuddy/sticker-picker-headless-*.png）')
if (fail) {
  for (const f of failures) console.log('  x ' + f)
  process.exit(1)
}
console.log('✓ 选择器：整块 330 / 路径 78 / 正文 192 / 3 行 x 24 / 十状态零跳动；标配行：入点/出点/定位(+拆分) 4 颗 192px + 间距 204 <= 210，一行，且已无「复制」')
