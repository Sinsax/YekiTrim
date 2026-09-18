// fonts.js — 字体库里的字体在**浏览器预览**里的加载。
//
// 出片（headless Chrome 加载 templates/*.html）那条路由 `render/server.rs` 注入 @font-face；
// 而 WebUI 自己的预览是同一个浏览器、同一个页面里的 DOM/canvas —— 它只认**系统已装字体**
// 或者**页面里注册过的 FontFace**。所以选了库里的字体，必须在这里把字节拉回来注册一次，
// 否则预览看着还是系统字体、出片却是库里的字体（"预览和成片不一样"这类问题的翻版）。
//
// 字节走 `/api/polish/font`（后端候选链已包含程序级字体库），与 wasm 渲染器取的是同一份。
import { fontUrl } from '../api/polish'

// 按 `id|dir` 缓存 Promise：同一个字体在多个组件里注册也只会拉一次
const loaded = new Map()

/**
 * 注册一条 FontFace（幂等）。**失败不抛** —— 字体缺失只该退回系统字体栈，
 * 不该把预览整块搞挂。
 * @returns {Promise<boolean>} 是否可用
 */
export function ensureFontFace(pid, dir, id, { onDebug } = {}) {
  const name = String(id || '').trim()
  if (!name) return Promise.resolve(false)
  const key = `${name}|${dir || ''}`
  const cached = loaded.get(key)
  if (cached) return cached

  const job = (async () => {
    try {
      // 已经注册过就不再重复加。
      // ⚠️ **别用 `document.fonts.check('12px "x"')` 判**：对"没有任何 @font-face 匹配"的字族名
      // 它按规范返回 **true**（视为可回退），于是第一次永远走不到注册那一步 —— 预览静默用系统字体
      // （实测：选了库里的字体，出片换了、预览没换）。要在 FontFaceSet 里按名字找已加载的那条。
      if (typeof document !== 'undefined' && document.fonts) {
        for (const f of document.fonts) {
          if (f.family.replace(/^"|"$/g, '') === name && f.status === 'loaded') return true
        }
      }
      const face = new FontFace(name, `url(${fontUrl(pid, dir, name)})`)
      await face.load()
      document.fonts.add(face)
      return true
    } catch (e) {
      onDebug?.(`字体「${name}」加载失败，回退系统字体：${e.message}`)
      // 失败的也留在缓存里：同一次会话里不必为同一个缺失字体反复发请求
      return false
    }
  })()
  loaded.set(key, job)
  return job
}
