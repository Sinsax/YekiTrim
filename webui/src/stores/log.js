// stores/log.js — 「运行日志」（设置页那张卡的数据源）
//
// 一条时间线上合两个来源：
// - server：服务端 tracing → `GET /api/admin/logs`（历史）+ `/api/admin/logs/events`（SSE 实时）
// - client：前端自己的请求失败/异常（`api()` / `apiOr()` 里记，source='client'）
//
// 只存内存（上限 2000 条，与服务端环形缓冲同量级）：刷新后从服务端重新拉历史，
// 不写 localStorage（日志是过程数据，不该在磁盘上留第二份）。
//
// ⚠️ **本模块不 import `api/client.js`**（client.js 反过来要 import 本模块记失败），
// 两个端点直接走裸 fetch：都是 admin-only 的 GET，不带 CSRF。

import { reactive } from 'vue'

const MAX = 2000
const LEVEL_RANK = { trace: 0, debug: 1, info: 2, warn: 3, error: 4 }

export function levelRank(level) {
  return LEVEL_RANK[String(level || '').toLowerCase()] ?? 2
}

export const logState = reactive({
  /** 合并后的条目：{ id, time, level, source, target, text } */
  records: [],
  /** SSE 是否已连上 */
  connected: false,
  /** 服务端端点是否可用（非 admin / 未登录 → false，面板给一句说明） */
  available: true,
  /** 过滤：最低级别（'all' | 'debug' | 'info' | 'warn' | 'error'） */
  levelKey: 'info',
  keyword: '',
  autoscroll: true,
})

let localSeed = 0
let lastServerSeq = 0
let es = null
/** 面板是否打开（没人看的时候不连 SSE，省一条长连接） */
let attached = false

function hms() {
  const d = new Date()
  const p = (n, w = 2) => String(n).padStart(w, '0')
  return `${p(d.getHours())}:${p(d.getMinutes())}:${p(d.getSeconds())}.${p(d.getMilliseconds(), 3)}`
}

function push(rec) {
  logState.records.push(rec)
  if (logState.records.length > MAX) {
    logState.records.splice(0, logState.records.length - MAX)
  }
}

/** 前端侧的一条（请求失败、异常、兜底读取失败…）。 */
export function addClient(level, text, target = 'ui') {
  push({ id: 'c' + (++localSeed), time: hms(), level, source: 'client', target, text: String(text) })
}

/** 服务端侧的一条（SSE / 历史拉取）。按 seq 去重：EventSource 重连会重放同一段。 */
export function addServer(rec) {
  if (!rec || typeof rec !== 'object') return
  const seq = Number(rec.seq) || 0
  if (seq && seq <= lastServerSeq) return
  if (seq) lastServerSeq = seq
  push({
    id: 's' + seq,
    time: String(rec.time || hms()),
    level: String(rec.level || 'info'),
    source: 'server',
    target: String(rec.target || ''),
    text: String(rec.text || ''),
  })
}

function openStream() {
  if (es) return
  try {
    es = new EventSource('/api/admin/logs/events?after=' + lastServerSeq)
    es.onopen = () => { logState.connected = true }
    es.onmessage = (ev) => {
      let rec = null
      try { rec = JSON.parse(ev.data) } catch { return }
      addServer(rec)
    }
    // 断线：EventSource 自带重连（重复回放由 addServer 的 seq 去重挡掉）。
    // 只把状态标成未连接，不往日志里刷屏。
    es.onerror = () => { logState.connected = false }
  } catch {
    logState.connected = false
  }
}

/** 打开面板时调用：先补历史，再挂实时流。重复调用无副作用。 */
export async function attach() {
  if (attached) return
  attached = true
  try {
    const r = await fetch('/api/admin/logs?limit=500', { credentials: 'same-origin' })
    if (r.ok) {
      const d = await r.json().catch(() => null)
      const recs = (d && d.records) || []
      recs.forEach(addServer)
      logState.available = true
    } else if (r.status === 401 || r.status === 403) {
      // 非管理员/未登录：面板里说明原因，不要一直重试
      logState.available = false
      addClient('warn', '没有读取服务端日志的权限（仅管理员）', 'log')
      return
    }
  } catch (e) {
    // 首次拉取失败不致命：下面还有 SSE；真连不上就只显示前端侧日志
    logState.available = true
  }
  openStream()
}

/** 关掉面板时调用：断开 SSE。 */
export function detach() {
  attached = false
  if (es) {
    es.close()
    es = null
  }
  logState.connected = false
}

/** 清空（只清这份本地缓冲，不动服务端环形缓冲）。 */
export function clear() {
  logState.records.splice(0, logState.records.length)
}
