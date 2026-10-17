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
    return { url: BASE + '/#/p/999999/polish/__nope__', label: '（自证）不存在的工程' }
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
      return {
        url: BASE + '/#/p/' + p.id + '/polish/' + encodeURIComponent(list[0].name),
        label: '工程 ' + p.id + '（' + p.name + '）/ ' + list[0].name,
      }
    }
  }
  console.log('· SKIP：没有任何工程带片段 —— 没有可以打开的精修页')
  process.exit(0)
}

const target = await pickTarget()
console.log('目标: ' + target.label)

const profile = mkdtempSync(join(tmpdir(), 'cdp-polish-page-'))
const chrome = spawn(chromePath, [
  '--headless=new', '--remote-debugging-port=' + PORT, '--user-data-dir=' + profile,
  '--no-first-run', '--window-size=1700,1400', 'about:blank',
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

await send('Runtime.enable')
await send('Page.enable')
await send('Page.navigate', { url: target.url })

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
// **工位页 + 素材库三个页签**（2026-10-18 加）
//
// 抓的是一类**静态门禁全都看不见**的错：`<script setup>` 里用了**没 import** 的标识符，
// 而且落在 `computed` / render 路径上 ⇒ 渲染期 `ReferenceError` ⇒ **Vue 卸掉整块子树**
// （用户报的「素材库点击字体时直接消失」就是这么来的：`hasBoldFace` 用了没引）。
// esbuild 不为未定义全局变量报错；`check-layout-invariants.mjs` 那条「模板标识符必须有定义」
// 只扫**模板**，那行在 `<script setup>` 里。
// ---------------------------------------------------------------------------
const problems = []
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
    for (const sheet of ['视频', '贴纸', '字体']) {
      const mark = exceptions.length + consoleErrors.length
      const clicked = await ev(
        "(function(){var b=Array.from(document.querySelectorAll('.ml-tab')).find(function(x){return x.innerText.indexOf('" + sheet + "')>=0});if(!b)return false;b.click();return true})()",
      )
      if (!clicked) { problems.push('素材库没有「' + sheet + '」页签'); continue }
      await sleep(1800)
      const alive = await ev(
        "(function(){return {nav:!!document.querySelector('.ml-nav'),grid:!!document.querySelector('.ml-grid'),cells:document.querySelectorAll('.ml-cell').length}})()",
      )
      if (!alive || !alive.nav || !alive.grid) {
        problems.push('切到「' + sheet + '」后素材库整块没了（nav=' + (alive && alive.nav) + ' grid=' + (alive && alive.grid) + ' cells=' + (alive && alive.cells) + '）')
      }
      const fresh = exceptions.slice(mark).concat(consoleErrors.slice(mark))
      if (fresh.length) problems.push('切到「' + sheet + '」时抛错：' + fresh.slice(0, 2).join(' | '))
    }
    const p2 = await send('Page.captureScreenshot', { format: 'png' })
    writeFileSync(join(ROOT, '.workbuddy', 'check-media-library-tabs.png'), Buffer.from(p2.result.data, 'base64'))
    console.log('· 素材库三个页签都点过（截图：.workbuddy/check-media-library-tabs.png）')
  }
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
