export async function api(path, opts = {}) {
  const r = await fetch(path, {
    headers: { 'Content-Type': 'application/json' },
    ...opts
  })
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
  if (!r.ok || (j && j.ok === false)) {
    throw new Error((j && j.error) || r.statusText)
  }
  return j
}
