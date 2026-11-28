#!/usr/bin/env node
// **精修页能不能真的挂起来**（2026-10-16）。
//
// # 为什么必须有这条判据
//
// webui/scripts/check-layout-invariants.mjs 那一族是**静态**判据（读源码/正则），而有一类错误
// 它们**结构上看不见**：组件 setup() 里**立即求值**的代码碰到 TDZ（比如
// watch([a, b], …, { immediate: true }) 的依赖数组里放了**本文件后面才声明**的 b）
// ⇒ 组件根本挂不起来：
//
//     ReferenceError: Cannot access X before initialization
//
// 症状是**整页只剩外壳、内容一片空白**（用户报「精修页面还是无法打开」）。构建绿、cargo 绿、
// 静态门禁全绿 —— 实测就是这么漏出去的（那一次是 watch 被挪到了 danmakuFont 之前）。
// 唯一的抓法是**把真页面开起来**：这条判据用 headless Chrome + CDP 打开精修页，要求
// ① 挂载点 .props-host 出现；② **一个未捕获异常都没有**；③ 控制台没有 error 级日志。
//
// # 2026-11-13 起只有精修一种模式（快速出片已删）：`.props-host` 一直存在
//
//   **2026-11-13 起出片页只有精修一种模式**（用户口径「不留快速切片」）：
//   默认 URL 直接就是编辑器，断言 `.props-host` 挂起来；快速模式的那些断言已删除。
//   ⚠️ A4/B2 那一段要拖 `.ruler-track`（时间轴只在 pro 子树里），所以它也一律走 pro URL。
//
// # 跑法与 SKIP
//
//   node webui/scripts/check-polish-page-loads.mjs                 # 打 127.0.0.1:26320
//   VT_BASE=http://127.0.0.1:26320 node webui/scripts/check-polish-page-loads.mjs
//   node webui/scripts/check-polish-page-loads.mjs --self-test    # 反自证：故意指坏路由，判据必须红
//
// ⚠️ **没有 Chrome / 没有在跑的服务时 SKIP**（exit 0 并打印原因）：本仓常规门禁不该因为
//    「这台机器没开界面」而红（与 check-cover-takumi.mjs 同一条口径）。
// ⚠️ **只认 error**：页面里的 404（比如老工程引用了已下线的字体名）是**预期**的 —— 应用自己
//    记成 warning 并回退，不该让这条判据红。
//
// # A4 / B2（2026-10-19 加，规格 docs/plan/polish-refactor-spec-2026-10-19.md §3 A4 / §4 B2）
//
//   · **A4 canvas 不重挂**：切片段 / 暂停播放 / 拖时间轴之后比 `window.__dhampirDiag.canvasGen`
//     —— **必须恒为 1**；同时比 `#dhampir-canvas` 的**元素身份**（计数器坏了也跑不掉）。
//     「任务完成」那一步**没测**（要跑任务 = 会改工程数据），打印里明着 SKIP。
//   · **B2 像素探针**：选片段 / 拖时间轴 / 跳屏蔽 / resize / 全屏 五个场景各采样 ~2s（约 60ms 一次），
//     用 CDP 截图 + **页内解码**量平均亮度（底座是 WebGL，`getContext('2d')` 读不回来）。
//     判据：平均亮度 <= 2/255 记一帧黑、**连续黑帧 < 200ms**（离散采样 ⇒ 报"最长黑帧 ms"时带上采样间隔）。
//     ⚠️ 本机现有片段没有非空 `mute_ranges` ⇒「跳屏蔽」**明着 SKIP**，用「远距离跳」近似代替（打印写明）。
//   · 两处**变异自证**：① 故意重挂一次（换路由再回来）⇒ canvasGen 必须能读到 > 1；
//     ② 给舞台盖一层纯黑 ⇒ 亮度判据必须读到黑。两条都翻不了面就是判据空转。
//     ⚠️ ① 会触发应用**既有**的自救刷新（那条闸自己打 error）—— 这一段收到的 error 会从失败计数里
//       摘掉，但打印里点名是几条（不许悄悄吞）。

import { spawn } from 'node:child_process'
import { existsSync, mkdtempSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join, resolve, dirname } from 'node:path'
import { fileURLToPath } from 'node:url'

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), '..', '..')
const BASE = process.env.VT_BASE || 'http://127.0.0.1:26320'
const SELF_TEST = process.argv.includes('--self-test')
const PORT = 9396
const CHROME_CANDIDATES = [
  process.env.DHAMPIR_CHROME || '',
  'C:/Program Files/Google/Chrome/Application/chrome.exe',
  'C:/Program Files (x86)/Google/Chrome/Application/chrome.exe',
  '/usr/bin/google-chrome',
  '/usr/bin/chromium',
  '/usr/bin/chromium-browser',
  '/snap/bin/chromium',
].filter(Boolean)
const sleep = (ms) => new Promise((r) => setTimeout(r, ms))

/** 宿主自己 catch 掉、只留一条 warning 的**致命**症状（见下面那段注释）。 */
const FATAL_WARN = /做不出位图|subtitle_raster_failed|danmaku_raster_failed/

const chromePath = CHROME_CANDIDATES.find((p) => existsSync(p))
if (!chromePath) {
  console.log('· SKIP：没找到 Chrome（想跑就设 DHAMPIR_CHROME=<chrome 路径>）')
  process.exit(0)
}

async function pickTarget() {
  if (SELF_TEST) {
    // 反自证：**故意**指一个不存在的工程 ⇒ 挂载点不该出现 ⇒ 判据必须红。
    const hash = '#/p/999999/polish/__nope__'
    return { url: BASE + '/' + hash, label: '（自证）不存在的工程', hash, pid: 999999, clip: '__nope__', clips: [] }
  }
  let projects
  try {
    projects = (await fetch(BASE + '/api/projects').then((r) => r.json())).projects || []
  } catch (e) {
    console.log('· SKIP：' + BASE + ' 上没有在跑的服务（' + (e.message || e) + '）')
    process.exit(0)
  }
  for (const p of projects) {
    const clips = await fetch(BASE + '/api/project-data/clips?project=' + p.id).then((r) => r.json())
    const list = Array.isArray(clips) ? clips : (clips.clips || [])
    if (list.length > 0) {
      // A4/B2 要用到 pid / 片段名（切片段 = 换 hash，**同一个页面**里换路由 —— B1 的现场）
      const names = list.map((c) => c.name)
      const hash = '#/p/' + p.id + '/polish/' + encodeURIComponent(names[0])
      return {
        url: BASE + '/' + hash,
        hash,
        pid: p.id,
        clip: names[0],
        clips: names,
        label: '工程 ' + p.id + '（' + p.name + '）/ ' + names[0] + '（共 ' + names.length + ' 个片段）',
      }
    }
  }
  console.log('· SKIP：没有任何工程带片段 —— 没有可以打开的精修页')
  process.exit(0)
}

const target = await pickTarget()
console.log('目标: ' + target.label)

/** 出片页现在**默认就是精修编辑器**（没有 quick/pro 之分），直接用原 URL。 */
const proUrl = target.url

/** 挂载失败/快速模式错位这类问题在**两条**路径上都会收集，所以先立账本。 */
const problems = []

const profile = mkdtempSync(join(tmpdir(), 'cdp-polish-page-'))
const chrome = spawn(chromePath, [
  '--headless=new', '--remote-debugging-port=' + PORT, '--user-data-dir=' + profile,
  '--no-first-run', '--no-sandbox', '--window-size=1700,1400', 'about:blank',
], { stdio: 'ignore' })

let page
for (let i = 0; i < 80; i++) {
  try {
    const list = await (await fetch('http://127.0.0.1:' + PORT + '/json/list')).json()
    page = list.find((t) => t.type === 'page')
    if (page) break
  } catch { /* 还没起来 */ }
  await sleep(250)
}
if (!page) {
  console.error('✗ Chrome 起来了但拿不到调试目标 —— 环境问题（不是页面问题）')
  process.exit(2)
}

const ws = new WebSocket(page.webSocketDebuggerUrl)
await new Promise((r) => { ws.onopen = r })
let id = 0
const pending = new Map()
const exceptions = []
const consoleErrors = []
ws.onmessage = (e) => {
  const m = JSON.parse(e.data)
  if (m.id && pending.has(m.id)) { pending.get(m.id)(m); pending.delete(m.id); return }
  if (m.method === 'Runtime.exceptionThrown') {
    const d = m.params.exceptionDetails
    exceptions.push((d.exception?.description || d.text || '').split('\n').slice(0, 3).join(' | ').slice(0, 400))
  }
  if (m.method === 'Runtime.consoleAPICalled' && m.params.type === 'error') {
    consoleErrors.push(m.params.args.map((a) => a.value ?? a.description ?? a.type).join(' ').slice(0, 300))
  }
  // ⚠️ **warning 里也藏着致命的**（2026-10-16 加）：宿主把"这一条字做不出位图"**catch 在自己内部**
  //    只记一条 `console.warn` ⇒ 页面**没有未捕获异常**、挂载点也在，而**满屏的字一个都不显示**。
  //    实测就是这么漏出去的（我改宿主 JS 时把 `lineStyle` 插错位置 ⇒ 每条字都抛 ⇒ 字幕+文字层全没）。
  //    ⇒ 这一类必须按"红"处理，不能只盯 error。
  if (m.method === 'Runtime.consoleAPICalled' && m.params.type === 'warning') {
    const line = m.params.args.map((a) => a.value ?? a.description ?? a.type).join(' ').slice(0, 300)
    if (FATAL_WARN.test(line)) consoleErrors.push(line)
  }
}
const send = (method, params = {}) => new Promise((r) => {
  const i = ++id
  pending.set(i, r)
  ws.send(JSON.stringify({ id: i, method, params }))
})
const ev = async (expr) => (await send('Runtime.evaluate', { expression: expr, returnByValue: true, awaitPromise: true })).result?.result?.value

/** 轮询一个页内表达式，直到它为真（A4/B2 段落用；原来那几处是内联循环）。 */
async function waitFor(expr, ms = 15000) {
  const t0 = Date.now()
  while (Date.now() - t0 < ms) {
    if (await ev(expr)) return true
    await sleep(250)
  }
  return false
}

await send('Runtime.enable')
await send('Page.enable')

// ── 断言 `.props-host` 挂起来了（这页真正"能打开"的判据）。
await send('Page.navigate', { url: proUrl })

let mounted = false
for (let i = 0; i < 60; i++) {
  if (await ev("!!document.querySelector('.props-host')")) { mounted = true; break }
  await sleep(500)
}
await sleep(1500)

const shot = await send('Page.captureScreenshot', { format: 'png' })
const out = join(ROOT, '.workbuddy', 'check-polish-page-loads.png')
writeFileSync(out, Buffer.from(shot.result.data, 'base64'))

// ---------------------------------------------------------------------------
// **工位页 + 素材库四个页签**（2026-10-18 加；2026-11-27 补「音效」）
//
// 抓的是一类**静态门禁全都看不见**的错：`<script setup>` 里用了**没 import** 的标识符，
// 而且落在 `computed` / render 路径上 ⇒ 渲染期 `ReferenceError` ⇒ **Vue 卸掉整块子树**
// （用户报的「素材库点击字体时直接消失」就是这么来的：`hasBoldFace` 用了没引）。
// esbuild 不为未定义全局变量报错；`check-layout-invariants.mjs` 那条「模板标识符必须有定义」
// 只扫**模板**，那行在 `<script setup>` 里。
// ---------------------------------------------------------------------------
{
  await send('Page.navigate', { url: BASE + '/#/workbench' })
  let libOk = false
  for (let i = 0; i < 40; i++) {
    if (await ev("!!document.querySelector('.ml-nav')")) { libOk = true; break }
    await sleep(500)
  }
  if (!libOk) {
    problems.push('工位页的素材库没挂起来（等 20s 没等到 .ml-nav）')
  } else {
    // ⚠️ 四个页签都要点（2026-11-27 加「音效」）：这一条抓的正是 `<script setup>` 里
    //    未 import 的标识符（渲染期 ReferenceError ⇒ Vue 卸掉整块子树），新页签最容易带进来。
    for (const sheet of ['视频', '贴纸', '音效', '字体']) {
      const mark = exceptions.length + consoleErrors.length
      const clicked = await ev(
        "(function(){var b=Array.from(document.querySelectorAll('.ml-tab')).find(function(x){return x.innerText.indexOf('" + sheet + "')>=0});if(!b)return false;b.click();return true})()",
      )
      if (!clicked) { problems.push('素材库没有「' + sheet + '」页签'); continue }
      await sleep(1800)
      const alive = await ev(
        "(function(){return {nav:!!document.querySelector('.ml-nav'),grid:!!document.querySelector('.ml-grid'),empty:!!document.querySelector('.ml-main p.empty'),cells:document.querySelectorAll('.ml-cell').length}})()",
      )
      // 空库（本机没有视频/贴纸/字体）时渲染的是 `.empty` 而不是 `.ml-grid` —— 那**不是**"整块没了"。
      // 只有 nav 也没有、或既没 grid 也没 empty 才算组件被卸掉。
      if (!alive || !alive.nav || !(alive.grid || alive.empty)) {
        problems.push('切到「' + sheet + '」后素材库整块没了（nav=' + (alive && alive.nav) + ' grid=' + (alive && alive.grid) + ' cells=' + (alive && alive.cells) + '）')
      }
      const fresh = exceptions.slice(mark).concat(consoleErrors.slice(mark))
      if (fresh.length) problems.push('切到「' + sheet + '」时抛错：' + fresh.slice(0, 2).join(' | '))
    }
    const p2 = await send('Page.captureScreenshot', { format: 'png' })
    writeFileSync(join(ROOT, '.workbuddy', 'check-media-library-tabs.png'), Buffer.from(p2.result.data, 'base64'))
    console.log('· 素材库四个页签都点过（截图：.workbuddy/check-media-library-tabs.png）')
  }
}

// ---------------------------------------------------------------------------
// A4 canvas 不重挂 + B2 像素探针（2026-10-19，规格 §3 A4 / §4 B2）
//
// 为什么这两条必须在**真页面**里量：#dhampir-canvas 一旦被换掉，底座就再也画不到新块上
// （一页只 attach 一次、**没有 detach**）—— 症状是「整块黑、控制台里什么都没有」。
// 静态判据看不见它（模板里 v-show 还在就算过），像素也读不回来（WebGL 的 surface 只有
// RENDER_ATTACHMENT）⇒ 只能靠 CDP 截图 + 页内解码量平均亮度。
//
// ⚠️ 判据口径：**平均亮度 <= 2/255 记一帧黑**、**连续黑帧 < 200ms**（采样是离散的，
// 所以「最长黑帧」= 首末黑样本之间 + 一个采样间隔中位数；打印里带上采样间隔，免得读成精确值）。
// ⚠️ Dhampir 没起来（没有 WebGPU / 引擎加载失败）⇒ **SKIP 并打印原因**，不许静默算过。
// ---------------------------------------------------------------------------
const BLACK_THRESHOLD = 2      // 平均亮度 <= 2/255 ⇒ 这一帧是黑的
const BLACK_BUDGET_MS = 200    // 连续黑帧的上限
const diagRows = []

/** 在页里立一个「当前这块 canvas」的锚：元素身份 + canvasGen **一起**看（计数器坏了也跑不掉）。 */
async function a4Arm() {
  return ev("(function(){window.__a4={el:document.querySelector('#dhampir-canvas'),gen:(window.__dhampirDiag||{}).canvasGen};return !!window.__a4.el})()")
}
async function a4Read() {
  return ev("(function(){var d=window.__dhampirDiag||{};var now=document.querySelector('#dhampir-canvas');var a=window.__a4||{};return {gen:(typeof d.canvasGen==='number'?d.canvasGen:-1),same:!!a.el&&a.el===now,t:Math.round((d.t||0)*100)/100,ready:d.activeSourceReadyState,busy:!!d.busy,mask:!!d.maskOn,seen:!!d.lastPresentAt}})()")
}
/** A4 的一步：动作前后比 canvasGen + 元素身份。**恒为 1** 才算过，并打印是哪一步坏的。 */
async function a4Step(label, action, settleMs) {
  await a4Arm()
  const before = await a4Read()
  if (action) await action()
  await sleep(settleMs || 1200)
  const after = await a4Read()
  const ok = !!after && after.gen === 1 && after.same === true
  if (!ok) {
    problems.push('A4 canvas 被重挂（' + label + '）：canvasGen=' + (after && after.gen) + '、元素还是同一块=' + (after && after.same))
  }
  diagRows.push('  ' + (ok ? 'ok   ' : 'FAIL ') + 'A4 ' + label + '：canvasGen=' + (after && after.gen)
    + ' · 同一块元素=' + (after && after.same) + '（前 gen=' + (before && before.gen) + '）')
  return after
}

/** 舞台在页面里的裁剪框（每次现量：resize / 全屏都会改它）。 */
async function stageClip() {
  const r = await ev("(function(){var s=document.querySelector('.stage');if(!s)return null;s.scrollIntoView({block:'center'});var b=s.getBoundingClientRect();return {x:Math.max(0,Math.round(b.left)),y:Math.max(0,Math.round(b.top)),w:Math.round(b.width),h:Math.round(b.height)}})()")
  if (!r || r.w <= 8 || r.h <= 8) return null
  return { x: r.x, y: r.y, width: r.w, height: r.h, scale: 0.25 }
}
/** 装一次页内解码器：CDP 截图 → 页里 createImageBitmap → 平均亮度（零依赖、借浏览器自己的 PNG 解码）。 */
async function installMeanBrightness() {
  await ev([
    'window.__pxMean = async function (b64) {',
    '  var bin = atob(b64); var buf = new Uint8Array(bin.length);',
    '  for (var i = 0; i < bin.length; i++) buf[i] = bin.charCodeAt(i);',
    '  var bmp = await createImageBitmap(new Blob([buf], { type: "image/png" }));',
    '  var c = window.__pxCanvas || (window.__pxCanvas = document.createElement("canvas"));',
    '  c.width = bmp.width; c.height = bmp.height;',
    '  var ctx = c.getContext("2d", { willReadFrequently: true });',
    '  ctx.drawImage(bmp, 0, 0); bmp.close();',
    '  var d = ctx.getImageData(0, 0, c.width, c.height).data; var sum = 0;',
    '  for (var j = 0; j < d.length; j += 4) sum += (d[j] + d[j + 1] + d[j + 2]) / 3;',
    '  return sum / (d.length / 4);',
    '}',
  ].join('\n'))
}
async function stageMean(clip) {
  const shot = await send('Page.captureScreenshot', { format: 'png', clip, captureBeyondViewport: false })
  const b64 = shot && shot.result && shot.result.data
  if (!b64) return -1
  // ⚠️ 页面**刷新过**（重挂那一步会触发应用的自救刷新）⇒ 页内的 __pxMean 没了，重装一次再量
  if (!(await ev("typeof window.__pxMean === 'function'"))) await installMeanBrightness()
  const mean = await ev("window.__pxMean('" + b64 + "')")
  return (typeof mean === 'number' && isFinite(mean)) ? mean : -1
}
/** 离散采样的黑帧统计：黑样本数 + 最长连续黑的**时长**（首末样本之间 + 一个采样间隔中位数）。 */
function blackStats(samples) {
  let black = 0, run = 0, best = 0, curStart = 0, bestStart = 0, bestEnd = 0, broken = 0
  for (const s of samples) {
    if (s.mean < 0) { broken += 1; run = 0; continue }   // 量不到 ≠ 不黑：单独计数
    if (s.mean <= BLACK_THRESHOLD) {
      black += 1; if (run === 0) curStart = s.at; run += 1
      if (run > best) { best = run; bestStart = curStart; bestEnd = s.at }
    } else run = 0
  }
  const gaps = samples.slice(1).map((s, i) => s.at - samples[i].at).sort((a, b) => a - b)
  const step = gaps.length ? gaps[Math.floor(gaps.length / 2)] : 100
  const longestMs = best > 0 ? Math.round(bestEnd - bestStart + step) : 0
  return { black, longestMs, run: best, step: Math.round(step), n: samples.length, broken }
}
/** B2 的一个场景：动作 → 采样 ~2s → 报「黑帧数 / 最长黑帧 ms」。 */
async function b2Scenario(name, action, ms) {
  // 起手先确保在播：静止画面上的"0 黑帧"证明不了引擎那条路（B2 要的是播放中）
  const playing = await ensurePlaying()
  if (action) await action()
  const clip = await stageClip()
  if (!clip) { diagRows.push('  SKIP B2 ' + name + '：量不到舞台（.stage 没有尺寸）'); return null }
  const t0 = Date.now()
  const samples = []
  while (Date.now() - t0 < (ms || 2000)) {
    const at = Date.now() - t0
    samples.push({ at, mean: await stageMean(clip) })
    await sleep(20)
  }
  const st = blackStats(samples)
  const ok = st.longestMs < BLACK_BUDGET_MS && st.broken === 0
  if (st.longestMs >= BLACK_BUDGET_MS) problems.push('B2 黑帧超预算（' + name + '）：最长 ' + st.longestMs + 'ms >= ' + BLACK_BUDGET_MS + 'ms')
  if (st.broken > 0) problems.push('B2 有 ' + st.broken + ' 次亮度量不到（' + name + '）—— 那一帧既不算黑也不算亮，别混过去')
  diagRows.push('  ' + (ok ? 'ok   ' : 'FAIL ') + 'B2 ' + name + '：黑帧 ' + st.black + '/' + st.n
    + ' · 起手 ' + playing
    + ' · 最长 ' + st.longestMs + 'ms（采样间隔中位数 ' + st.step + 'ms'
    + (st.broken ? ' · 量不到 ' + st.broken + ' 次' : '') + '）')
  return st
}

/** 切片段 = **同一个页面里换路由**（不是重新加载文档）—— 这正是 B1 要抓的那条路。 */
async function switchClip(name) {
  // 直接进编辑器（没有模式之分），
  //    换片段（同一个页面里换 hash）时不带上它就会掉回默认 quick，时间轴/标尺整套消失。
  const hash = '#/p/' + target.pid + '/polish/' + encodeURIComponent(name)
  return ev("(function(){location.hash='" + hash + "';return location.hash})()")
}
/** 拖时间轴标尺（pointerdown → pointermove(window) → pointerup），返回标尺在不在。 */
async function dragRuler(fromPct, toPct) {
  return ev("(function(){var tr=document.querySelector('.ruler-track');if(!tr)return 'no-ruler';" +
    "var b=tr.getBoundingClientRect();if(b.width<60)return 'narrow';var y=b.top+b.height/2;" +
    "var x1=b.left+b.width*" + fromPct + ",x2=b.left+b.width*" + toPct + ";" +
    "var mk=function(t,x){return new PointerEvent(t,{bubbles:true,cancelable:true,clientX:x,clientY:y,button:0,buttons:t==='pointerup'?0:1,pointerId:1,pointerType:'mouse',isPrimary:true})};" +
    "tr.dispatchEvent(mk('pointerdown',x1));window.dispatchEvent(mk('pointermove',x2));window.dispatchEvent(mk('pointerup',x2));" +
    "return 'ok'})()")
}
/** 预览现在是在播还是暂停（读那颗按钮的文案）。 */
async function playState() {
  return ev("(function(){var bs=document.querySelectorAll('.preview-controls .mini-btn');for(var i=0;i<bs.length;i++){var t=bs[i].innerText.trim();if(t==='暂停')return 'playing';if(t==='播放')return 'paused'}return 'unknown'})()")
}
/** 让预览处于**播放中**（B2 量的正是"播放中出黑帧"；静止画面上的 0 黑帧说明不了什么）。 */
async function ensurePlaying() {
  if (await playState() === 'paused') { await clickPlayPause(); await sleep(800) }
  return playState()
}
/** 播放 / 暂停那颗按钮（返回点到的文案）。 */
async function clickPlayPause() {
  return ev("(function(){var bs=document.querySelectorAll('.preview-controls .mini-btn');for(var i=0;i<bs.length;i++){if(/暂停|播放/.test(bs[i].innerText)){bs[i].click();return bs[i].innerText.trim()}}return 'no-btn'})()")
}

const diagUp = { ok: false, why: '' }
{
  // 回精修页（上面素材库那一段把页面导航去了工位页）。
  // ⚠️ 用 pro：A4/B2 要拖 `.ruler-track`（时间轴），而它只在精修子树里。
  await send('Page.navigate', { url: proUrl })
  const mountedAgain = await waitFor("!!document.querySelector('#dhampir-canvas')", 30000)
  const hasDiag = mountedAgain && await waitFor('!!window.__dhampirDiag', 20000)
  const attached = hasDiag && await waitFor('!!window.__dhampirAttachedCanvas', 40000)
  await sleep(1500)
  const bar = await ev("(function(){var b=document.querySelector('.dhampir-bar');return b?b.innerText.replace(/\\s+/g,' ').slice(0,150):''})()")
  diagUp.ok = !!attached
  diagUp.why = attached ? '' : ((bar || '') + (hasDiag ? '' : ' · 页面里没有 window.__dhampirDiag（A3 埋点没跑到）'))
  if (!diagUp.ok) {
    console.log('· SKIP：Dhampir 预览没起来 ⇒ A4（canvasGen）与 B2（黑帧）这两条**没测**')
    console.log('  原因：' + (diagUp.why || '(没等到 __dhampirAttachedCanvas)'))
  }
}

// 前置：引擎**真的出过帧**（lastPresentAt > 0）—— 否则「0 黑帧」测的是个没有画面的空舞台
if (diagUp.ok && !(await waitFor('(window.__dhampirDiag||{}).lastPresentAt > 0', 20000))) {
  diagUp.ok = false
  diagUp.why = 'Dhampir 引擎一直没上屏（lastPresentAt = 0）'
  console.log('· SKIP：Dhampir 引擎没出过帧 ⇒ A4（canvasGen）与 B2（黑帧）这两条**没测**')
}
if (diagUp.ok) {
  await installMeanBrightness()
  const clips = target.clips || []
  const otherClip = clips.find((c) => c !== target.clip)
  const gen0 = await a4Read()
  diagRows.push('  ·    A4 起点：canvasGen=' + (gen0 && gen0.gen) + ' · 埋点已在（present 有值=' + (gen0 && gen0.seen) + '）')

  // ① 切片段（A4 + B2 同一个动作）
  if (otherClip) {
    await a4Step('切片段 ' + otherClip, () => switchClip(otherClip), 2500)
    await b2Scenario('选片段（切到 ' + otherClip + '）', null, 2000)
  } else {
    diagRows.push('  SKIP A4/B2 切片段：这个工程只有一个片段（' + clips.join(',') + '）')
  }

  // ② 暂停 / 播放（A4）
  let playLabel = ''
  await a4Step('暂停播放', async () => {
    playLabel = await clickPlayPause()
    await sleep(800)
    playLabel += ' → ' + await clickPlayPause()
  }, 1200)
  diagRows.push('  ·    暂停/播放按钮点到了：' + playLabel + '；收尾状态 ' + await ensurePlaying())

  // ③ 拖时间轴（A4 + B2）
  const beforeDrag = await a4Read()
  await a4Step('拖时间轴', () => dragRuler(0.15, 0.75), 1800)
  const afterDrag = await a4Read()
  diagRows.push('  ·    拖时间轴读数：t ' + (beforeDrag && beforeDrag.t) + ' → ' + (afterDrag && afterDrag.t) + ' 秒')
  await b2Scenario('拖时间轴（标尺划过）', () => dragRuler(0.8, 0.2), 2000)

  // ④ 跳屏蔽：**没有屏蔽区间就明说**（不静默算过）
  diagRows.push('  SKIP B2 跳屏蔽：本机现有片段没有非空 mute_ranges（段落表全片连续）'
    + ' ⇒ 没有可跳的段；用下面那行「远距离跳」代替（口径已写明）')
  await b2Scenario('跳（近似：播放中远跳 3 次）', async () => {
    for (let i = 0; i < 3; i += 1) { await dragRuler(0.1 + i * 0.3, 0.9 - i * 0.3); await sleep(400) }
  }, 2000)

  // ⑤ resize 窗口（B2 的正主：播放中改画布尺寸）
  await b2Scenario('resize 窗口（1600→1200 播放中）', async () => {
    await clickPlayPause()             // 确保在播（B2 要的正是「播放中改尺寸」）
    await sleep(300)
    await send('Emulation.setDeviceMetricsOverride', { width: 1200, height: 900, deviceScaleFactor: 1, mobile: false })
  }, 2200)

  // ⑥ 全屏（B2）
  const fsState = await ev("(function(){var bs=document.querySelectorAll('.preview-controls .mini-btn');for(var i=0;i<bs.length;i++){if(bs[i].title==='全屏'){window.__fsBtn=bs[i];return 'found'}}return 'no-btn'})()")
  if (fsState === 'found') {
    await send('Runtime.evaluate', { expression: 'window.__fsBtn.click()', userGesture: true })
    await sleep(900)
    const inFs = await ev('!!document.fullscreenElement')
    if (inFs) {
      await b2Scenario('全屏（stage 进全屏）', null, 2000)
      await ev('document.exitFullscreen && document.exitFullscreen()')
      await sleep(600)
    } else {
      diagRows.push('  SKIP B2 全屏：headless 里 requestFullscreen 没进全屏（浏览器不给）—— 不静默算过')
    }
  } else {
    diagRows.push('  SKIP B2 全屏：没找到全屏按钮')
  }

  // ⑦ 变异自证：**故意重挂一次**（换路由再回来）—— canvasGen 必须真的能被推过 1
  {
    // ⚠️ 这一步**预期**会让页面打 error：应用发现 canvas 换了 ⇒ 走它既有的自救闸（location.reload），
    //    而"起引擎失败：正在刷新页面以重新接入 Dhampir"就是那条路自己打的。它**不是**本轮的回归 ——
    //    收尾时按**消息**把那几条摘掉（见下面 EXCUSED 那段），并点名摘了几条。
    const beforeVar = await a4Read()
    await ev("location.hash='#/workbench'")
    await sleep(900)
    // 回到精修页那一条 hash：**从 `proUrl` 取**（脚本里只有它是那段地址的真源）——
    // 这里以前写的是从未声明过的 `proHash`，于是整条判据在 ⑦ 变异自证处
    // `ReferenceError` 崩掉：前六段全绿、看起来"跑过了"，其实 ⑦ 之后一条都没验。
    await ev("(function(){location.hash='" + new URL(proUrl).hash + "';return true})()")
    let maxGen = 0
    const tv = Date.now()
    while (Date.now() - tv < 3000) {
      const g = await ev('(window.__dhampirDiag||{}).canvasGen')
      if (typeof g === 'number' && g > maxGen) maxGen = g
      await sleep(120)
    }
    const okVar = maxGen > 1
    if (!okVar) problems.push('变异自证失败：故意重挂一次之后 canvasGen 仍是 ' + maxGen + ' —— A4 判据是空转的')
    diagRows.push('  ' + (okVar ? 'ok   ' : 'FAIL ') + '变异自证（故意重挂一次）：canvasGen 读到 ' + maxGen
      + '（起点 ' + (beforeVar && beforeVar.gen) + '）⇒ 判据 gen>1 真的会翻面')
    await sleep(1200)
    const after = await ev('(window.__dhampirDiag||{}).canvasGen')
    diagRows.push('  ·    重挂后 canvasGen=' + after + '（若回到 1 = 应用按既有自救闸刷新过页面，正是文档里那条路）')
  }

  // ⑦b 变异自证（B2）：**给舞台盖一层纯黑** ⇒ 亮度判据必须读到黑（否则那条判据是空转的）
  {
    await installMeanBrightness()   // 上面那一步刷新过页面 ⇒ 页内的解码器要重装
    await ev("(function(){var s=document.querySelector('.stage');var b=s.getBoundingClientRect();var d=document.createElement('div');d.id='__black_probe';d.style.cssText='position:fixed;left:'+Math.round(b.left)+'px;top:'+Math.round(b.top)+'px;width:'+Math.round(b.width)+'px;height:'+Math.round(b.height)+'px;background:#000;z-index:99999';document.body.appendChild(d);return true})()")
    const clip = await stageClip()
    const means = []
    for (let i = 0; i < 4; i += 1) { means.push(await stageMean(clip)); await sleep(60) }
    const maxMean = Math.max.apply(null, means)
    const okB = maxMean <= BLACK_THRESHOLD
    if (!okB) problems.push('变异自证失败（B2）：盖上纯黑之后平均亮度仍有 ' + maxMean + ' —— 亮度判据是空转的')
    diagRows.push('  ' + (okB ? 'ok   ' : 'FAIL ') + '变异自证（B2 盖纯黑）：平均亮度 '
      + means.map((m) => Math.round(m * 100) / 100).join(' / ') + '（阈值 ' + BLACK_THRESHOLD + '）')
    await ev("(function(){var d=document.getElementById('__black_probe');if(d)d.remove();return true})()")
  }

  // ⑧ HUD：只有 ?diag=1 才渲染（4 行）
  await send('Page.navigate', { url: BASE + '/?diag=1' + target.hash })
  const hudOk = await waitFor("!!document.querySelector('.dhampir-hud')", 30000)
  await sleep(4000)   // HUD 是 ~5Hz 刷的，且要等引擎起帧 —— 立刻读只会读到初始占位符
  const hudText = await ev("(function(){var h=document.querySelector('.dhampir-hud');return h?h.innerText.replace(/\\s+/g,' '):''})()")
  const hudLines = await ev("document.querySelectorAll('.dhampir-hud > div').length")
  const hudPass = hudOk && hudLines === 4 && /canvasGen/.test(hudText)
  if (!hudPass) problems.push('A3 HUD 没出来或不完整（?diag=1）：lines=' + hudLines + ' text=' + hudText)
  diagRows.push('  ' + (hudPass ? 'ok   ' : 'FAIL ') + 'A3 HUD（?diag=1）：' + hudLines + ' 行 · ' + hudText)

  // 收尾：把"故意重挂"那一步**预期**触发的自救刷新错误按**消息**摘掉（只认自救那几句，
  // 不按条数盲摘 —— 第二条 error 是刷新流程里后到的，按条数摘会漏一条在报告里）。
  {
    const EXCUSED = /重新接入 Dhampir|attachReload|已经 attach 过一块 canvas/
    const dropExcused = (arr) => {
      let n = 0
      for (let i = arr.length - 1; i >= 0; i -= 1) { if (EXCUSED.test(String(arr[i]))) { arr.splice(i, 1); n += 1 } }
      return n
    }
    const dropped = dropExcused(exceptions) + dropExcused(consoleErrors)
    if (dropped > 0) diagRows.push('  ·    摘掉 ' + dropped + ' 条**预期** error（故意重挂触发的自救刷新自己打的）')
  }

  console.log('')
  console.log('== A4 canvasGen / B2 黑帧（每场景数字）==')
  for (const line of diagRows) console.log(line)
  console.log('')
}
if (!mounted) problems.push('精修页没挂起来（等 30s 没等到 .props-host）—— setup 里多半有立即求值的错误')
for (const e of exceptions.slice(0, 5)) problems.push('未捕获异常：' + e)
for (const e of consoleErrors.slice(0, 5)) problems.push('控制台 error：' + e)

if (problems.length > 0) {
  console.error('✗ 精修页没能正常打开（' + problems.length + ' 条）：')
  for (const p of problems) console.error('  - ' + p)
  console.error('  现场截图：' + out)
  process.exitCode = 1
} else {
  console.log('✓ 精修页挂起来了：.props-host 出现、0 个未捕获异常、0 条 error 日志')
  console.log('  截图：' + out)
}
chrome.kill()
