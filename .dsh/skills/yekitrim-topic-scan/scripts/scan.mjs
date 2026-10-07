#!/usr/bin/env node
/**
 * yekitrim-topic-scan / scan.mjs
 *
 * 扫「当前运行的 YekiTrim 服务」上所有项目的切片，按关键词找出可剪的素材，
 * 并把「没搜到」拆成三种完全不同的情况。
 *
 * 用法：
 *   node scan.mjs --kw "维阿,virtuareal,vr"
 *   node scan.mjs --discover                 # 不给词：题材套件 + 数据侧热度榜
 *   node scan.mjs --kw "恋爱" --project 5,10 # 只扫指定项目
 *   node scan.mjs --kw "恋爱" --top 40 --json
 *
 * 开关：
 *   --kw <a,b,c>   关键词，逗号分隔（给了就只用这些）
 *   --discover     不给词模式
 *   --project <ids> 只扫这些项目 id
 *   --top <n>      热度榜条数（默认 30）
 *   --out <dir>    报告目录（默认 <仓库>/.workbuddy/topic-scan）
 *   --json         额外落一份机器可读 JSON
 *   --server <url> 服务地址（默认 http://127.0.0.1:26320）
 *   --quiet        只写文件，不往 stdout 打报告
 *
 * ⚠️ 本脚本**只读**：不写项目数据、不改 events.json、不触发重新检测。
 *    唯一会写的是报告目录。
 */

import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { compileKeywords, topicSuiteEntries } from './keywords.mjs'

const HERE = path.dirname(fileURLToPath(import.meta.url))
// scripts/ -> yekitrim-topic-scan/ -> skills/ -> .dsh/ -> <仓库根>
const REPO = path.resolve(HERE, '..', '..', '..', '..')

// ─────────────────────────────────────────── CLI

function parseArgs(argv) {
  const a = { kw: null, discover: false, project: null, top: 30, out: null, json: false, server: 'http://127.0.0.1:26320', quiet: false }
  for (let i = 0; i < argv.length; i++) {
    const t = argv[i]
    const next = () => argv[++i]
    if (t === '--kw') a.kw = next()
    else if (t === '--discover') a.discover = true
    else if (t === '--project') a.project = String(next()).split(',').map(s => Number(s.trim())).filter(Number.isFinite)
    else if (t === '--top') a.top = Math.max(1, Number(next()) || 30)
    else if (t === '--out') a.out = next()
    else if (t === '--json') a.json = true
    else if (t === '--server') a.server = next().replace(/\/+$/, '')
    else if (t === '--quiet') a.quiet = true
    else if (t === '-h' || t === '--help') {
      const src = fs.readFileSync(fileURLToPath(import.meta.url), 'utf8')
      const body = src.split('/**')[1].split('*/')[0]
      console.log(body.split('\n').filter(l => l.startsWith(' *')).map(l => l.replace(/^ \* ?/, '').replace(/^\/ /, '')).join('\n'))
      process.exit(0)
    }
    else { console.error(`未知参数：${t}（--help 看用法）`); process.exit(2) }
  }
  if (!a.kw && !a.discover) a.discover = true
  return a
}

// ─────────────────────────────────────────── HTTP

async function getJson(server, url) {
  const r = await fetch(server + url, { headers: { accept: 'application/json' } })
  const text = await r.text()
  if (!r.ok) throw new Error(`HTTP ${r.status} ${url} :: ${text.slice(0, 200)}`)
  try { return JSON.parse(text) } catch { throw new Error(`不是 JSON：${url} :: ${text.slice(0, 120)}`) }
}

// ─────────────────────────────────────────── 判据（照抄 webui/src/utils.js，不许自己发明）

export function clipNameOf(rank) {
  return 'clip-' + String(Math.max(0, Number(rank) || 0)).padStart(2, '0')
}

/** 这个片段目录**是不是这条事件**剪出来的。与 webui/src/utils.js::clipMatchesEvent 逐行一致。 */
export function clipMatchesEvent(clip, ev) {
  if (!clip || !ev) return false
  if (ev.clip_dir) return String(clip.name || '') === String(ev.clip_dir)
  if (String(clip.name || '') !== clipNameOf(ev.rank)) return false
  const meta = clip.meta || {}
  const oldTitle = String(meta.title || '').trim()
  const sameTitle = !!oldTitle && oldTitle === String(ev.label || '').trim()
  const os = Number(meta.original_start)
  const oe = Number(meta.original_end)
  const s = Number(ev.start_sec)
  const e = Number(ev.end_sec)
  const hasRange = Number.isFinite(os) && Number.isFinite(oe) && Number.isFinite(s) && Number.isFinite(e)
  const overlap = hasRange
    ? Math.max(0, Math.min(oe, e) - Math.max(os, s)) / Math.max(1e-6, Math.max(oe, e) - Math.min(os, s))
    : 1
  if (sameTitle || overlap >= 0.5) return true
  if (!oldTitle && !hasRange) return true
  return false
}

/** 未剪切事件 = 没有任何"属于自己的"片段的事件。 */
export function uncutEventsOf(events, clips) {
  const list = (clips || []).filter(c => c && c.video_exists !== false)
  return (events || []).filter(ev => !list.some(c => clipMatchesEvent(c, ev)))
}

// ─────────────────────────────────────────── 弹幕 / SRT 解析

/** 录播姬 XML：<d p="秒,模式,...">文本</d> */
function parseDanmakuXml(text) {
  const out = []
  const re = /<d\s+p="([^"]*)"[^>]*>([\s\S]*?)<\/d>/g
  let m
  while ((m = re.exec(text))) {
    const t = parseFloat(String(m[1]).split(',')[0] || '0')
    if (Number.isFinite(t)) out.push({ t, text: String(m[2]).trim() })
  }
  return out
}

/** ASS：Dialogue: 层,起,止,Style,...,,文本（第 10 段起是文本，要剥掉 {\...} 覆盖码） */
function parseDanmakuAss(text) {
  const out = []
  for (const line of text.split(/\r?\n/)) {
    if (!line.startsWith('Dialogue:')) continue
    const parts = line.slice('Dialogue:'.length).split(',')
    if (parts.length < 10) continue
    const hms = parts[1].trim().split(':')
    const t = hms.length === 3 ? (+hms[0]) * 3600 + (+hms[1]) * 60 + parseFloat(hms[2]) : 0
    const body = parts.slice(9).join(',').replace(/\{[^}]*\}/g, '').trim()
    if (Number.isFinite(t) && body) out.push({ t, text: body })
  }
  return out
}

/** SRT：取每条的时间起点 + 正文 */
function parseSrt(text) {
  const out = []
  for (const block of text.replace(/\r/g, '').split(/\n\n+/)) {
    const ls = block.split('\n')
    const ti = ls.findIndex(l => l.includes('-->'))
    if (ti < 0) continue
    const m = ls[ti].match(/(\d+):(\d+):(\d+)[,.](\d+)/)
    const t = m ? +m[1] * 3600 + +m[2] * 60 + +m[3] + +m[4] / 1000 : 0
    const body = ls.slice(ti + 1).join(' ').trim()
    if (body) out.push({ t, text: body })
  }
  return out
}

/** 读一个项目的弹幕文件（项目根下的 .xml/.ass）。读不到就返回空 + 原因。 */
function readDanmaku(rootDir) {
  const items = []
  const files = []
  if (!rootDir || !fs.existsSync(rootDir)) return { items, files, note: 'root_dir 不可读（服务在别的机器？）' }
  let names = []
  try { names = fs.readdirSync(rootDir).filter(f => /\.(xml|ass)$/i.test(f)) } catch (e) { return { items, files, note: `列举失败：${e.message}` } }
  for (const f of names) {
    try {
      const raw = fs.readFileSync(path.join(rootDir, f), 'utf8')
      const parsed = f.toLowerCase().endsWith('.xml') ? parseDanmakuXml(raw) : parseDanmakuAss(raw)
      items.push(...parsed)
      files.push({ name: f, count: parsed.length })
    } catch (e) { files.push({ name: f, count: 0, error: e.message }) }
  }
  // ASS 与 XML 常是同一份内容：按 (t,text) 去重
  const seen = new Set()
  const uniq = items.filter(i => { const k = i.t + '\u0000' + i.text; if (seen.has(k)) return false; seen.add(k); return true })
  return { items: uniq, files, note: null }
}

/** 读 output/ 下的字幕文件（srt）。订正稿能救回原稿漏的词 —— 男友粉就是这例。 */
function readSubtitleFiles(rootDir) {
  const out = { raw: [], corrected: [], note: null }
  if (!rootDir) return out
  const dir = path.join(rootDir, 'output')
  const tryRead = (name) => {
    try { const p = path.join(dir, name); return fs.existsSync(p) ? parseSrt(fs.readFileSync(p, 'utf8')) : null } catch { return null }
  }
  const raw = tryRead('video.srt')
  const corr = tryRead('video_llm.srt')
  if (raw) out.raw = raw
  if (corr) out.corrected = corr
  if (!raw && !corr) out.note = 'output/ 下没有 srt（可能只有转写）'
  return out
}

// ─────────────────────────────────────────── 工具

const clk = (s) => {
  s = Math.max(0, Math.round(Number(s) || 0))
  return [s / 3600 | 0, (s % 3600) / 60 | 0, s % 60].map(x => String(x).padStart(2, '0')).join(':')
}

/** 命中词周围的上下文（压平空白）。 */
function context(text, kwList, width = 60) {
  const flat = String(text).replace(/\s+/g, ' ')
  for (const k of kwList) {
    const i = flat.toLowerCase().indexOf(String(k).toLowerCase())
    if (i < 0) continue
    const s = Math.max(0, i - width)
    const e = Math.min(flat.length, i + String(k).length + width)
    return (s > 0 ? '…' : '') + flat.slice(s, e) + (e < flat.length ? '…' : '')
  }
  const s = flat.slice(0, width * 2)
  return s + (flat.length > width * 2 ? '…' : '')
}

// ─────────────────────────────────────────── 主流程

async function main() {
  const args = parseArgs(process.argv.slice(2))

  const health = await getJson(args.server, '/api/health').catch(e => { throw new Error(`服务不可达：${args.server}（${e.message}）`) })
  const projectsAll = (await getJson(args.server, '/api/projects')).projects || []
  const projects = args.project ? projectsAll.filter(p => args.project.includes(Number(p.id))) : projectsAll

  const wanted = args.kw ? args.kw.split(',').map(s => s.trim()).filter(Boolean) : topicSuiteEntries().map(e => e.kw)
  const matchers = compileKeywords(wanted)
  const broadWarnings = matchers.filter(m => m.broad).map(m => m.kw)
  // 词 → 分组（只有发现模式有；关键词模式全是"自定义"）
  const groupOf = new Map()
  if (!args.kw) for (const e of topicSuiteEntries()) groupOf.set(e.kw, e.group)
  const groupOfHit = (kw) => groupOf.get(kw) || '自定义'

  const perProject = []
  for (const p of projects) {
    const id = p.id
    let events = [], clips = [], transcript = []
    try { events = (await getJson(args.server, `/api/project-data/events?project=${id}`)).events || [] } catch (e) { events = [] }
    try {
      const c = await getJson(args.server, `/api/project-data/clips?project=${id}`)
      clips = Array.isArray(c) ? c : (c.clips || [])   // ⚠️ clips 返回的是**裸数组**
    } catch { clips = [] }
    try { transcript = (await getJson(args.server, `/api/project-data/transcript?project=${id}`)).segments || [] } catch { transcript = [] }
    let rootDir = null
    try { rootDir = (await getJson(args.server, `/api/project-data/config?project=${id}`)).root_dir || null } catch { /* 忽略 */ }

    const uncut = uncutEventsOf(events, clips)
    const dm = readDanmaku(rootDir)
    const subs = readSubtitleFiles(rootDir)

    perProject.push({ p, events, clips, transcript, uncut, dm, subs, rootDir })
  }

  // ── 匹配：每条命中都归位到一个事件（按**区间重叠**，不是"起点落在区间内"）
  const hits = []       // { p, kind: 'uncut'|'cut'|'orphan', ev?, t, source, kw, text }
  for (const rec of perProject) {
    const { p, events, transcript, dm, subs } = rec
    const sources = [
      ['字幕', transcript.map(s => ({ t: s.start, text: s.text, end: s.end })), true],
      ['订正稿', subs.corrected.map(s => ({ t: s.t, text: s.text, end: null })), true],
      ['弹幕', dm.items.map(s => ({ t: s.t, text: s.text, end: null })), false],
    ]
    for (const [sourceName, items] of sources) {
      for (const it of items) {
        const matched = matchers.filter(m => m.test(String(it.text)))
        if (!matched.length) continue
        const owner = events.find(e => it.t >= e.start_sec && it.t < e.end_sec)
        const kind = !owner ? 'orphan' : (rec.uncut.includes(owner) ? 'uncut' : 'cut')
        hits.push({
          projectId: p.id, projectName: p.name, kind,
          rank: owner ? owner.rank : null,
          label: owner ? owner.label : null,
          pop: owner ? (owner.popularity || 0) : 0,
          evStart: owner ? owner.start_sec : null,
          evEnd: owner ? owner.end_sec : null,
          dmCount: owner ? (owner.danmaku_count || 0) : 0,
          subCount: owner ? (owner.subtitle_count || 0) : 0,
          type: owner ? owner.event_type : null,
          t: it.t, source: sourceName,
          kws: matched.map(m => m.kw),
          text: it.text,
        })
      }
    }
  }

  // ── 缺口分析：没有任何事件覆盖的时间段（>60s）
  const gaps = []
  for (const rec of perProject) {
    const { p, events, transcript } = rec
    if (!transcript.length) continue
    const sorted = events.slice().sort((a, b) => a.start_sec - b.start_sec)
    const end = Math.max(...transcript.map(s => s.end))
    let cur = 0
    const list = []
    for (const e of sorted) {
      if (e.start_sec - cur > 60) list.push([cur, e.start_sec])
      cur = Math.max(cur, e.end_sec)
    }
    if (end - cur > 60) list.push([cur, end])
    for (const [a, b] of list) {
      const segs = transcript.filter(s => s.end > a && s.start < b)
      gaps.push({
        projectId: p.id, projectName: p.name, start: a, end: b, secs: b - a,
        segCount: segs.length,
        excerpt: segs.map(s => s.text).join(' ').replace(/\s+/g, ' ').slice(0, 200),
      })
    }
  }

  // ── 热度榜：未剪切里弹幕+字幕密度最高的
  const heat = []
  for (const rec of perProject) {
    for (const ev of rec.uncut) {
      heat.push({
        projectId: rec.p.id, projectName: rec.p.name, rank: ev.rank,
        label: ev.label, summary: ev.summary, pop: ev.popularity || 0,
        dm: ev.danmaku_count || 0, sub: ev.subtitle_count || 0,
        start: ev.start_sec, end: ev.end_sec, type: ev.event_type,
      })
    }
  }
  heat.sort((a, b) => b.pop - a.pop)

  // ── 渲染报告
  const stats = {
    server: args.server, version: health.version, localNoAuth: !!health.local_no_auth,
    projects: projects.length, events: perProject.reduce((n, r) => n + r.events.length, 0),
    uncut: perProject.reduce((n, r) => n + r.uncut.length, 0),
    cut: perProject.reduce((n, r) => n + (r.events.length - r.uncut.length), 0),
    contactsWithDanmaku: perProject.filter(r => r.dm.items.length > 0).length,
    keywords: matchers.map(m => m.kw),
    mode: args.kw ? '关键词模式' : '发现模式',
  }

  const md = renderReport({ args, stats, perProject, hits, gaps, heat, groupOfHit })
  const at = new Date()
  const stamp = `${at.getFullYear()}${String(at.getMonth() + 1).padStart(2, '0')}${String(at.getDate()).padStart(2, '0')}-${String(at.getHours()).padStart(2, '0')}${String(at.getMinutes()).padStart(2, '0')}${String(at.getSeconds()).padStart(2, '0')}`
  const outDir = args.out ? path.resolve(args.out) : path.join(REPO, '.workbuddy', 'topic-scan')
  fs.mkdirSync(outDir, { recursive: true })
  const mdPath = path.join(outDir, `scan-${stamp}.md`)
  fs.writeFileSync(mdPath, md)

  let jsonPath = null
  if (args.json) {
    jsonPath = path.join(outDir, `scan-${stamp}.json`)
    fs.writeFileSync(jsonPath, JSON.stringify({ stats, hits, gaps, heat: heat.slice(0, args.top), projects: perProject.map(r => ({ id: r.p.id, name: r.p.name, events: r.events.length, uncut: r.uncut.length, danmaku: r.dm.items.length, danmakuNote: r.dm.note, subtitleNote: r.subs.note })) }, null, 2))
  }

  if (!args.quiet) process.stdout.write(renderConsole({ stats, hits, gaps, heat, args }))
  console.log(`\n报告：${mdPath}` + (jsonPath ? `\nJSON：${jsonPath}` : ''))
}

// ─────────────────────────────────────────── 渲染

function renderConsole({ stats, hits, gaps, heat, args }) {
  const L = []
  L.push(`\n════ 扫描结果 ════`)
  L.push(`服务 ${stats.server}  v${stats.version}${stats.localNoAuth ? '（本机免登录）' : ''}`)
  L.push(`模式：${stats.mode}  关键词：${stats.keywords.join(' / ')}`)
  L.push(`范围：${stats.projects} 个项目 · ${stats.events} 事件 · 未剪 ${stats.uncut} · 已剪 ${stats.cut}`)
  L.push(`其中 ${stats.contactsWithDanmaku} 个项目有弹幕`)

  const by = (k) => hits.filter(h => h.kind === k)
  L.push(`\n命中：未剪切 ${by('uncut').length} · 已剪切 ${by('cut').length} · ⚠️无归属 ${by('orphan').length}`)
  L.push(`⚠️ 缺口：${gaps.length} 段 >60s，共 ${(gaps.reduce((n, g) => n + g.secs, 0) / 60).toFixed(1)} 分钟未被任何事件覆盖`)

  const topUncut = by('uncut').slice().sort((a, b) => b.pop - a.pop).slice(0, 12)
  if (topUncut.length) {
    L.push(`\n── 未剪切命中 Top ${topUncut.length} ──`)
    // 同一个事件会被多个词 / 多个来源各命中一次：控制台按事件去重，明细在报告里
    const seenEv = new Set()
    for (const h of topUncut) {
      const key = `${h.projectId}#${h.rank}`
      if (seenEv.has(key)) continue
      seenEv.add(key)
      L.push(`  p${h.projectId} #${String(h.rank).padStart(3)} ${clk(h.evStart)}–${clk(h.evEnd)} 热${h.pop.toFixed(2)}「${h.kws.join('/')}」`)
      L.push(`      ${h.label}`)
    }
  }
  if (gaps.length) {
    L.push(`\n── 最大缺口 ──`)
    for (const g of gaps.slice().sort((a, b) => b.secs - a.secs).slice(0, 5)) {
      L.push(`  p${g.projectId} ${clk(g.start)}–${clk(g.end)} ${(g.secs / 60).toFixed(1)}min (${g.segCount} 段字幕)`)
    }
  }
  return L.join('\n') + '\n'
}

function renderReport({ args, stats, perProject, hits, gaps, heat, groupOfHit }) {
  const by = (k) => hits.filter(h => h.kind === k)
  const L = []
  const wb = (pid, step = 'slices') => `http://127.0.0.1:26320/#/p/${pid}/${step}`

  L.push(`# 切片话题扫描报告`)
  L.push(``)
  L.push(`> 生成时间：${new Date().toLocaleString('zh-CN')}`)
  L.push(`> 数据源：\`${stats.server}\` · YekiTrim v${stats.version}${stats.localNoAuth ? '（本机免登录）' : ''}`)
  L.push(`> 模式：**${stats.mode}** · 关键词：\`${stats.keywords.join('`, `')}\``)
  L.push(``)

  L.push(`## 一、范围`)
  L.push(``)
  L.push(`| 项 | 值 |`)
  L.push(`|---|---|`)
  L.push(`| 项目 | ${stats.projects} |`)
  L.push(`| 事件总数 | ${stats.events} |`)
  L.push(`| **未剪切** | **${stats.uncut}** |`)
  L.push(`| 已剪切 | ${stats.cut} |`)
  L.push(`| 有弹幕的项目 | ${stats.contactsWithDanmaku} / ${stats.projects} |`)
  L.push(``)

  // 项目明细（含数据可用性告警）
  L.push(`### 项目明细与数据可用性`)
  L.push(``)
  L.push(`| 项目 | id | 事件 | 未剪 | 弹幕 | 备注 |`)
  L.push(`|---|---|---|---|---|---|`)
  for (const r of perProject) {
    const notes = []
    if (!r.uncut.length && r.events.length) notes.push('已全切')
    if (!r.transcript.length) notes.push('⚠️ 无转写')
    if (r.dm.note) notes.push(`⚠️ 弹幕：${r.dm.note}`)
    else if (!r.dm.items.length) notes.push('⚠️ 无弹幕')
    if (r.subs.note) notes.push(`⚠️ ${r.subs.note}`)
    L.push(`| ${r.p.name} | ${r.p.id} | ${r.events.length} | ${r.uncut.length} | ${r.dm.items.length} | ${notes.join('；') || '—'} |`)
  }
  L.push(``)

  // ── 三种「没有」
  L.push(`## 二、⚠️ 三种「没有」—— 别把后两种当成没有`)
  L.push(``)
  L.push(`| 判定 | 依据 | 本次结果 |`)
  L.push(`|---|---|---|`)
  L.push(`| ❌ 真没有 | 字幕 + 订正稿 + 弹幕全文都搜不到 | ${hits.length === 0 ? '**是**' : '否'} |`)
  L.push(`| ⚠️ 有但漏检 | 命中落在**事件缺口**里 | ${by('orphan').length} 处（另有 ${gaps.length} 段缺口本身可能是漏检） |`)
  L.push(`| ⚠️ 有但碎了 | 命中段**无归属**（跨边界 / 短于 30s 被吞） | 见「无归属命中」 |`)
  L.push(``)

  const orphan = by('orphan')
  if (orphan.length) {
    L.push(`### ⚠️ 无归属命中（不在任何事件里）`)
    L.push(``)
    L.push(`这些句子的词命中了，但它**不属于任何事件** —— 要么被切在事件边界的缝里，要么被后处理的「最短 30s 吞并」吃掉了。`)
    L.push(`**它们目前剪不出来**，要么先合并相邻事件、要么手建事件。`)
    L.push(``)
    for (const h of orphan) {
      L.push(`- **${h.projectName}** \`${clk(h.t)}\` [${h.source}]「${h.kws.join('/')}」`)
      L.push(`  - ${h.text.replace(/\s+/g, ' ')}`)
    }
    L.push(``)
  }

  // ── 命中
  L.push(`## 三、命中明细`)
  L.push(``)

  // 发现模式先给「分组概览」：词表很宽，逐条列出来没人看得完，先看哪一组值得追
  if (stats.mode === '发现模式') {
    const groups = new Map()
    for (const h of hits) {
      const g = groupOfHit(h.kws[0])
      if (!groups.has(g)) groups.set(g, { uncut: [], cut: [], orphan: [], projs: new Set() })
      const e = groups.get(g)
      e[h.kind].push(h)
      e.projs.add(h.projectId)
    }
    L.push(`### 3.0 分组概览（发现模式：先看哪一组值得追）`)
    L.push(``)
    L.push(`| 题材组 | 未剪切命中 | 涉及项目 | 已剪切 | 无归属 | 最高热度 |`)
    L.push(`|---|---|---|---|---|---|`)
    for (const [g, e] of [...groups.entries()].sort((a, b) => b[1].uncut.length - a[1].uncut.length)) {
      const top = e.uncut.reduce((m, h) => Math.max(m, h.pop), 0)
      L.push(`| ${g} | ${e.uncut.length} | ${e.projs.size} | ${e.cut.length} | ${e.orphan.length} | ${top ? top.toFixed(2) : '—'} |`)
    }
    L.push(``)
  }

  const uncut = by('uncut').slice().sort((a, b) => b.pop - a.pop)
  const cut = by('cut')

  L.push(`### 3.1 未剪切（可剪）—— ${uncut.length} 处`)
  L.push(``)
  if (!uncut.length) {
    L.push(`> 未剪切里没有命中。${hits.length ? '（可能已经被剪过了，见 3.2）' : ''}`)
    if (stats.mode === '发现模式') L.push(`> 这是**发现模式**：下面是热度榜，看看数据里本来就热的是什么。`)
  }
  // 按事件聚合，一个事件一行
  const grouped = new Map()
  for (const h of uncut) {
    const k = `${h.projectId}#${h.rank}`
    if (!grouped.has(k)) grouped.set(k, { ...h, items: [] })
    grouped.get(k).items.push(h)
  }
  // 发现模式：词表宽，逐条列 600+ 行没人看。每个题材组只出前 5 条，其余靠 3.0 概览 + 第五节热度榜。
  const rows = [...grouped.values()].sort((a, b) => b.pop - a.pop)
  const limited = stats.mode === '发现模式'
  if (limited && rows.length > 30) {
    L.push(`> 命中太多（${rows.length} 个事件），下面**每个题材组只列前 5 个**；`)
    L.push(`> 完整清单看 \`--json\`，或者用 \`--kw\` 缩小到一个话题再看。`)
    L.push(``)
  }
  const perGroupShown = new Map()
  for (const g of rows) {
    const grp = groupOfHit(g.kws[0])
    if (limited) {
      const n = perGroupShown.get(grp) || 0
      if (n >= 5) continue
      perGroupShown.set(grp, n + 1)
    }
    L.push(`- **${g.projectName}** #${g.rank} · \`${clk(g.evStart)}–${clk(g.evEnd)}\` · ${g.type} · 热度 **${g.pop.toFixed(2)}** · 弹幕 ${g.dmCount} / 字幕 ${g.subCount}`)
    L.push(`  - 标题：${g.label}`)
    L.push(`  - 命中：${[...new Set(g.items.map(i => i.source + '「' + i.kws.join('/') + '」'))].join('、')}`)
    for (const it of g.items.slice(0, 6)) {
      L.push(`    - \`${clk(it.t)}\` [${it.source}] ${it.text.replace(/\s+/g, ' ')}`)
    }
    if (g.items.length > 6) L.push(`    - …（共 ${g.items.length} 处）`)
    L.push(`  - 打开：${wb(g.projectId)}`)
  }
  L.push(``)

  L.push(`### 3.2 已剪切对照 —— ${cut.length} 处`)
  L.push(``)
  if (!cut.length) L.push(`> 已剪切的切片里没有命中 —— 这个话题**一条都还没剪出来**。`)
  else {
    for (const g of cut.slice().sort((a, b) => b.pop - a.pop).slice(0, 40)) {
      L.push(`- **${g.projectName}** #${g.rank} \`${clk(g.evStart)}–${clk(g.evEnd)}\` 热${g.pop.toFixed(2)} [${g.source}] → ${g.label}`)
    }
  }
  L.push(``)

  // ── 缺口
  L.push(`## 四、缺口分析（事件未覆盖的时间段）`)
  L.push(``)
  L.push(`这些时间段**一个字都没进过检测** —— 里面即使有你找的东西，也**不可能**出现在切片列表里。`)
  L.push(``)
  if (!gaps.length) L.push(`> 没有 >60s 的缺口，检测覆盖完整。`)
  else {
    L.push(`| 项目 | 区间 | 时长 | 字幕段 | 内容摘录 |`)
    L.push(`|---|---|---|---|---|`)
    for (const g of gaps.slice().sort((a, b) => b.secs - a.secs).slice(0, 30)) {
      L.push(`| ${g.projectName} | \`${clk(g.start)}–${clk(g.end)}\` | ${(g.secs / 60).toFixed(1)} min | ${g.segCount} | ${g.excerpt.slice(0, 80)}… |`)
    }
    L.push(``)
    L.push(`> ⚠️ 修法：在预处理页对**对应项目**重新检测。注意**重新检测会按热度整体重排 rank**，`)
    L.push(`> 已有片段与事件的对应可能错位（会被标成「独立」或「序号被占用」），需要人工核对。`)
  }
  L.push(``)

  // ── 热度榜
  L.push(`## 五、未剪切热度榜 Top ${args.top}`)
  L.push(``)
  L.push(`弹幕/字幕密度算出来的热度 —— 蹭流量该先看这个。每个项目内部按 popularity 排序。`)
  L.push(``)
  L.push(`| # | 项目 | rank | 区间 | 热度 | 弹幕 | 字幕 | 标题 |`)
  L.push(`|---|---|---|---|---|---|---|---|`)
  heat.slice(0, args.top).forEach((h, i) => {
    L.push(`| ${i + 1} | ${h.projectName} | ${h.rank} | \`${clk(h.start)}–${clk(h.end)}\` | ${h.pop.toFixed(2)} | ${h.dm} | ${h.sub} | ${h.label} |`)
  })
  L.push(``)

  // ── 建议
  L.push(`## 六、建议`)
  L.push(``)
  if (uncut.length) {
    L.push(`可剪命中（按热度）：`)
    for (const g of rows.slice(0, 10)) {
      L.push(`- **${g.projectName} #${g.rank}**（热 ${g.pop.toFixed(2)}）${g.label} → ${wb(g.projectId)}`)
    }
  } else {
    L.push(`- 关键词在未剪切里没有命中。先看**第五节热度榜**，那才是这批录播里真实的热点。`)
  }
  if (orphan.length) L.push(`- ⚠️ 有 ${orphan.length} 处命中**不在任何事件里**：先合并相邻事件或手建事件，否则剪不到。`)
  if (gaps.length) L.push(`- ⚠️ 有 ${gaps.length} 段缺口（共 ${(gaps.reduce((n, g) => n + g.secs, 0) / 60).toFixed(1)} 分钟）没进过检测：需要重新检测才能覆盖。`)
  const noDanmaku = perProject.filter(r => !r.dm.items.length)
  if (noDanmaku.length) L.push(`- 有 ${noDanmaku.length} 个项目**没有弹幕**：热度只能靠字幕密度，粉丝向内容会系统性排不高。`)
  L.push(``)
  L.push(`---`)
  L.push(``)
  L.push(`*本报告只读生成，未修改任何项目数据。剪切请在界面自行操作。*`)

  return L.join('\n') + '\n'
}

main().catch(e => {
  console.error(`\n扫描失败：${e.message}`)
  if (process.env.VT_DEBUG) console.error(e)
  process.exit(1)
})
