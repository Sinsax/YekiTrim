// api/client.js — 唯一的请求入口。
//
// **所有请求都在这里留痕**（`console` + 设置页「运行日志」）：
// 以前只有调用方的 toast，静默 `.catch(() => [])` 之后失败就彻底消失了。
// - 失败（网络错误 / 非 2xx / `ok:false`）→ `console.warn` + 日志面板（warn/error）
// - 成功 → 只写 `console.debug`，避免刷屏（逐条成功日志看面板的「调试」档）
// - 高频只读接口（/api/jobs 轮询、/api/media、/api/health）成功时连面板都不写，
//   否则 3 秒一次的轮询会把 2000 条缓冲冲干净。

import { addClient } from '../stores/log'

/** 成功路径不写日志面板的高频接口（失败照常记）。 */
const QUIET_OK = [/^\/api\/media\//, /^\/api\/health\b/, /^\/api\/jobs\b/, /^\/api\/stream\b/, /^\/api\/admin\/logs\b/]

function nowMs() {
  return typeof performance !== 'undefined' && performance.now ? performance.now() : Date.now()
}

/**
 * 记一条成功（仅 debug）。
 * @param {string} label 调用方的业务说明（可选），如「读取片段列表」
 */
function noteOk(method, path, status, t0, label = '') {
  const ms = Math.round(nowMs() - t0)
  const text = `${label ? label + '：' : ''}${method} ${path} → ${status} (${ms}ms)`
  console.debug('[api]', text)
  if (!QUIET_OK.some((re) => re.test(path))) addClient('debug', text, 'api')
}

/**
 * 记一条失败（控制台 + 日志面板），返回可直接 `throw` 的错误对象。
 * @returns {Error} 带 `status`（网络层失败为 0）
 */
function noteFailure(method, path, status, t0, detail, label = '') {
  const ms = Math.round(nowMs() - t0)
  const text = `${label ? label + '：' : ''}${method} ${path} → ${status || '网络错误'} (${ms}ms) · ${detail}`
  console.warn('[api]', text)
  addClient(status >= 500 ? 'error' : 'warn', text, 'api')
  const e = new Error(detail)
  e.status = status
  return e
}

/**
 * 默认请求超时（ms）。
 *
 * **请求必须一定会 settle。** 没有超时的 `fetch` 在连接卡住时永远不 resolve，
 * 调用方如果靠 `.finally` 做收尾就会被永久冻住 —— `useJobs` 就是（它的 `inflight`
 * 去重只由 `.finally` 清），一次卡住的 `/api/jobs` 会让任务列表与所有按作用域门控的
 * 按钮一起停在最后一帧，用户看到的是"队列加载不出来 + 剪切按钮一直灰"。
 *
 * 只能兜一个**很松**的上限：这里有一批同步长接口 —— `/api/polish/silence` 要对**整个视频**
 * 跑 ffmpeg silencedetect（3 小时素材 + 网络盘）、`cover-title` 要等 LLM、`ingest/scan`
 * 要遍历录制目录 —— 卡紧了就是把它们从"慢"改成"失败"。真正的时限由调用方按接口自己给：
 * 高频轮询传短的（`jobsApi` 传 10s，那条路径才是冻住的源头），确实需要更久的传 `{ timeout: 0 }`。
 */
const DEFAULT_TIMEOUT_MS = 10 * 60_000

export async function api(path, opts = {}) {
  // silent401：调用方自己处理未登录（启动时的 GET /api/me、登录请求），
  // 不让统一 401 分支去跳登录页，否则会绕圈。
  // label：业务说明（「读取片段列表」），进日志更容易对上"刚才点的是什么"。
  // timeout：毫秒；0 = 不设上限（见 DEFAULT_TIMEOUT_MS 的说明）。
  const { silent401 = false, label = '', timeout = DEFAULT_TIMEOUT_MS, ...init } = opts
  const method = (init.method || 'GET').toUpperCase()
  const headers = { 'Content-Type': 'application/json', ...(init.headers || {}) }

  // CSRF：cookie 会话 + 自定义头双提交（服务端只对写请求校验）
  if (!['GET', 'HEAD', 'OPTIONS'].includes(method)) {
    const token = readCookie('vtrim_csrf')
    if (token) headers['X-CSRF-Token'] = token
  }

  const t0 = nowMs()
  // 调用方自带 signal 时不接管（它自己的取消语义优先）
  const ctrl = timeout > 0 && !init.signal ? new AbortController() : null
  let timer = null
  if (ctrl) {
    init.signal = ctrl.signal
    timer = setTimeout(() => ctrl.abort(), timeout)
  }
  try {
    let r
    try {
      r = await fetch(path, { credentials: 'same-origin', ...init, headers })
    } catch (e) {
      // 超时会被 abort 中断，走到这里的是 DOMException（各浏览器消息不一致），
      // 所以用 signal 状态判定，别去猜 message
      const timedOut = !!ctrl && ctrl.signal.aborted
      throw noteFailure(
        method, path, 0, t0,
        timedOut ? `请求超时（${timeout}ms）` : `网络错误: ${(e && e.message) || e}`,
        label,
      )
    }

    const ct = (r.headers.get('content-type') || '').toLowerCase()
    let j = null
    if (ct.includes('application/json')) {
      const text = await r.text().catch(() => '')
      if (!text.trim()) {
        // 空响应体（如 204）：成功就返回 null；失败仍要按状态码报错
        if (r.ok) return null
        throw noteFailure(method, path, r.status, t0, r.statusText || '请求失败', label)
      }
      try {
        j = JSON.parse(text)
      } catch (e) {
        // 以前这里静默留 null，调用方把"烂响应"当成成功；现在明确报错并留痕
        throw noteFailure(method, path, r.status, t0, `响应不是合法 JSON：${text.trim().slice(0, 120)}`, label)
      }
    } else {
      // 后端只服务 webui/dist：缺失的 /api 路由会命中 SPA 回退并返回 200 HTML，
      // 必须显式报错，否则调用方会拿到 null 当成成功。
      const body = await r.text().catch(() => '')
      if (r.ok && !body.trim()) return null // 空响应体（如 204）
      throw noteFailure(
        method, path, r.status, t0,
        '接口不存在或返回格式错误: ' + path + ' (' + r.status + ')',
        label,
      )
    }

    if (r.status === 401 && !silent401) {
      notifyUnauthorized()
    }
    if (!r.ok || (j && j.ok === false)) {
      throw noteFailure(method, path, r.status, t0, (j && j.error) || r.statusText || '请求失败', label)
    }
    noteOk(method, path, r.status, t0, label)
    return j
  } finally {
    // 计时器活到函数真正结束（含响应体读取）：连接卡在半途时 `r.text()` 同样会挂住
    if (timer) clearTimeout(timer)
  }
}

/**
 * 带兜底值的请求：失败**留痕**后返回 `fallback`。
 *
 * 专治 `api(...).catch(() => [])`：那种写法把"读取失败"伪装成"没有数据"，
 * 页面上看不出任何异常，日志里也什么都不剩。用这个 + 一句 `label`（是哪个功能在读），
 * 失败就能在控制台与「运行日志」里查到（留痕由 `api()` 那一次完成，这里只兜值）。
 */
export async function apiOr(path, fallback, label) {
  try {
    return await api(path, { label })
  } catch (e) {
    return fallback
  }
}

export function readCookie(name) {
  const m = document.cookie.match(new RegExp('(?:^|;\\s*)' + name + '=([^;]*)'))
  return m ? decodeURIComponent(m[1]) : ''
}

/** 写请求需要带的 CSRF 头（裸 `fetch` 调用的场景，如渲染单帧的 PNG 响应）。 */
export function csrfHeaders() {
  const token = readCookie('vtrim_csrf')
  return token ? { 'X-CSRF-Token': token } : {}
}

// ---- 401 统一处理（由 main.js 注册，避免 client ↔ store 循环依赖） ----
let unauthorizedHandler = null

export function setUnauthorizedHandler(fn) {
  unauthorizedHandler = fn
}

function notifyUnauthorized() {
  if (unauthorizedHandler) unauthorizedHandler()
}

/** 当前 hash 路由（不含 `#`），用于登录回跳。 */
export function currentRoute() {
  return window.location.hash.replace(/^#/, '') || '/'
}

/** 跳到登录页并记住回跳（已在登录页则不动）。 */
export function redirectToLogin() {
  const cur = currentRoute()
  if (cur.startsWith('/login') || cur.startsWith('/register')) return
  window.location.hash = '#/login?redirect=' + encodeURIComponent(cur)
}
