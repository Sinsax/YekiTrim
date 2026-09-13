export async function api(path, opts = {}) {
  // silent401：调用方自己处理未登录（启动时的 GET /api/me、登录请求），
  // 不让统一 401 分支去跳登录页，否则会绕圈。
  const { silent401 = false, ...init } = opts
  const method = (init.method || 'GET').toUpperCase()
  const headers = { 'Content-Type': 'application/json', ...(init.headers || {}) }

  // CSRF：cookie 会话 + 自定义头双提交（服务端只对写请求校验）
  if (!['GET', 'HEAD', 'OPTIONS'].includes(method)) {
    const token = readCookie('vtrim_csrf')
    if (token) headers['X-CSRF-Token'] = token
  }

  const r = await fetch(path, { credentials: 'same-origin', ...init, headers })
  const ct = (r.headers.get('content-type') || '').toLowerCase()
  let j = null
  if (ct.includes('application/json')) {
    try { j = await r.json() } catch (e) {}
  } else {
    // 后端只服务 webui/dist：缺失的 /api 路由会命中 SPA 回退并返回 200 HTML，
    // 必须显式报错，否则调用方会拿到 null 当成成功。
    const body = await r.text().catch(() => '')
    if (r.ok && !body.trim()) return null // 空响应体（如 204）
    throw new Error('接口不存在或返回格式错误: ' + path + ' (' + r.status + ')')
  }

  if (r.status === 401 && !silent401) {
    notifyUnauthorized()
  }
  if (!r.ok || (j && j.ok === false)) {
    const e = new Error((j && j.error) || r.statusText)
    e.status = r.status
    throw e
  }
  return j
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
  if (cur.startsWith('/login')) return
  window.location.hash = '#/login?redirect=' + encodeURIComponent(cur)
}
