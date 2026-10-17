// fonts.js — 字体库里的字体在**浏览器预览**里的加载。
//
// 出片（headless Chrome 加载 templates/*.html）那条路由 `render/server.rs` 注入 @font-face；
// 而 WebUI 自己的预览是同一个浏览器、同一个页面里的 DOM/canvas —— 它只认**系统已装字体**
// 或者**页面里注册过的 FontFace**。所以选了库里的字体，必须在这里把字节拉回来注册一次，
// 否则预览看着还是系统字体、出片却是库里的字体（"预览和成片不一样"这类问题的翻版）。
//
// 字节走 `/api/polish/font`（后端候选链已包含程序级字体库），与出片侧注入的 @font-face 取的是同一份。
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

/**
 * **这串值是不是库里的一支**（按 id / 内部族名 / 显示名，忽略大小写）。
 *
 * ⚠️ 为什么必须认三种名字（2026-10-16 用户报的「Noto Sans SC（不在字体库）」）：
 * `[style.subtitle].font` / `[style.cover].font` 里那串值可能是**库 id**（界面里选出来的、
 * 老工程存的），也可能是**族名**（Rust `default_font()` 写下的 `Noto Sans SC`）。只按 id 比 ⇒
 * 明明装着的字体被标成「（不在字体库）」—— 而它一直是能用的那支。
 * Rust 侧现在按同一条口径解析（`yekitrim_core::fonts::find_entry_roots`），三处下拉共用这一份判据。
 */
export function fontInLibrary(fonts, value) {
  const want = String(value ?? '').trim().toLowerCase()
  if (!want) return false
  return (fonts || []).some((f) =>
    [f.id, f.family, f.display_name].some((n) => String(n ?? '').trim().toLowerCase() === want))
}

/** 值 → 库里那一条（认 id / 族名 / 显示名）；库里没有 ⇒ null。 */
export function findFontInLibrary(fonts, value) {
  const want = String(value ?? '').trim().toLowerCase()
  if (!want) return null
  return (fonts || []).find((f) =>
    [f.id, f.family, f.display_name].some((n) => String(n ?? '').trim().toLowerCase() === want)) || null
}

/**
 * **别名命中时下拉要显示的那行字**（库里那支的展示标签；库里没有 ⇒ 空串）。
 *
 * ⚠️ 为什么需要它（2026-10-16）：下拉的 `value` 是**库 id**，而配置里那串可能是**族名**
 * （`default_font()` / `cmd_init` 写的都是族名）。修好「认族名」之后，若不再补一条
 * `value === 当前值` 的选项，`<select>` 里**没有任何选项匹配当前值** ⇒ 浏览器显示第一项，
 * 用户看到的是「选中的字体莫名其妙跳到了别的字体」（比原来的「不在字体库」更难懂）。
 * 这里**保留用户原来那串值**（不改配置），只是按库里那支给它一个正确的标签。
 */
export function fontLabelFor(fonts, value) {
  const e = findFontInLibrary(fonts, value)
  if (!e) return ''
  const base = e.family && e.family !== e.id ? `${e.family}（${e.id}）` : e.id
  return e.is_default ? `${base}[默认]` : base
}

/**
 * **库里有没有这支字体的粗体字面**（判据：`<族名> Bold`）。
 *
 * ⚠️ 与 Rust 侧**同一条命名规则**（`render/dhampir.rs::find_font` 就在按
 * `format!("{family} Bold")` 找粗体那一支）—— 两边不一致的症状是「界面说没有粗体、底座其实找得到」
 * （或反过来）：字幕/元素写着 700/600 的字重，成片却退回常规体，而**没有一处报错**。
 * 判据钉在 `scripts/check-text-style.mjs`（⑥f）。
 *
 * ⚠️ 匹配用**三种名字**（id / 内部族名 / 显示名，忽略大小写）：与 `fontInLibrary` 同一套口径 ——
 * 库里的粗体那支可能叫 `NotoSansSC-Bold` 而族名写着 `Noto Sans SC Bold`。
 * ⚠️ 条目**自己就是粗体**（族名以 Bold 结尾）时返回 true（"你这支有没有粗体"对粗体本身没有意义）。
 */
export function hasBoldFace(fonts, entry) {
  const fam = String((entry && (entry.family || entry.id)) || '').trim()
  if (!fam) return false
  if (/bold$/i.test(fam)) return true
  const want = `${fam.toLowerCase()} bold`
  return (fonts || []).some((f) =>
    [f.id, f.family, f.display_name].some((n) => String(n ?? '').trim().toLowerCase() === want))
}

/**
 * **配置里那串值 → 页面真正要用的族名**（库内按 `family` 取，库外原样返回）。
 *
 * ⚠️⚠️ 这是"换字体不生效"的根因那一处（2026-10-16 实测）：
 *  * `[style.subtitle].font` 里存的往往是**库 id**（`NotoSansSC-Regular`），而
 *    页面与底座的 font-family 只认**内部族名**（`Noto Sans SC`）；
 *  * 预览注册 FontFace 用的是**转译产物**里的 `font_family`（= 真族名，宿主的
 *    `fontFamilyFor` 换算过），DOM 那条腿却把**原值**直接写进 CSS ⇒
 *    两者对不上 ⇒ 浏览器**静默回退系统字体**（字号/位置/颜色全对，只有字形不对）。
 *  * 与 Rust 侧同一套口径：`render/dhampir.rs` 读宿主给的 id→族名映射、
 *    转译器 `fontFamilyFor` 用它写 `font_family`。判据 `check-text-style.mjs` ⑥h。
 *
 * ⚠️ **库外原样返回**（老工程的 `font = "LXGW WenKai"` 走系统字体栈，不该被改写）。
 */
export function fontFamilyFor(fonts, value) {
  const raw = String(value ?? '').trim()
  if (!raw) return ''
  const e = findFontInLibrary(fonts, raw)
  return (e && (e.family || e.id)) || raw
}
