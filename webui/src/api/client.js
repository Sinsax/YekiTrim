export async function api(path, opts = {}) {
  const r = await fetch(path, {
    headers: { 'Content-Type': 'application/json' },
    ...opts
  })
  let j = null
  try { j = await r.json() } catch (e) {}
  if (!r.ok || (j && j.ok === false)) {
    throw new Error((j && j.error) || r.statusText)
  }
  return j
}
