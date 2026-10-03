// 主题（哥特浅色 / 哥特暗色）。
//
// 设计要点：
// 1. **首帧必须已定**：`data-theme` 要挂到 <html> 上，且在 Vue 挂载前就设好 ——
//    否则暗色用户会先看到一帧亮色再翻黑（闪白）。
//    因此这里在模块初始化时**同步**读 localStorage 并立刻写 DOM。
// 2. **系统偏好作为默认**：用户没手动选过时跟随 `prefers-color-scheme`，
//    且系统切换时实时跟随（听了 matchMedia 的 change）。
// 3. 用户一旦手动选过，**系统偏好不再覆盖**（存 'light' / 'dark' 而不是 'auto'）。

import { reactive } from 'vue'

const LS_THEME = 'yekitrim.ui.theme'   // 'light' | 'dark' | 缺省 = 跟随系统

const mq = (() => {
  try {
    return window.matchMedia('(prefers-color-scheme: dark)')
  } catch {
    return null
  }
})()

/** 读用户的选择；没存过返回 null（表示"跟随系统"） */
function readTheme() {
  try {
    const v = localStorage.getItem(LS_THEME)
    return v === 'light' || v === 'dark' ? v : null
  } catch {
    return null
  }
}

/** 实际生效的主题 = 用户选择 ?? 系统偏好 ?? 亮色 */
function resolve() {
  const saved = readTheme()
  if (saved) return saved
  return mq && mq.matches ? 'dark' : 'light'
}

/** 写到 <html data-theme>；同时同步 <meta name="theme-color">（移动端地址栏） */
function apply(mode) {
  const el = document.documentElement
  el.setAttribute('data-theme', mode)
  // 让浏览器原生控件（滚动条、表单、下拉）也跟着换，否则暗色下白滚动条很扎眼
  el.style.colorScheme = mode
  const meta = document.querySelector('meta[name="theme-color"]')
  if (meta) meta.setAttribute('content', mode === 'dark' ? '#202020' : '#F8F8F0')
}

const initial = resolve()
apply(initial)   // ⚠️ 同步执行，早于 Vue 挂载 —— 这是不闪白的关键

export const ui = reactive({
  /** 当前生效主题 */
  theme: initial,
  /** 用户是否手动选过（false = 跟随系统） */
  themeIsManual: readTheme() !== null
})

/** 切到指定主题并记为用户选择 */
export function setTheme(mode) {
  const m = mode === 'dark' ? 'dark' : 'light'
  ui.theme = m
  ui.themeIsManual = true
  apply(m)
  try {
    localStorage.setItem(LS_THEME, m)
  } catch {}
}

/** 在浅/暗之间切换 */
export function toggleTheme() {
  setTheme(ui.theme === 'dark' ? 'light' : 'dark')
}

/** 交还给系统偏好（清掉手动选择） */
export function followSystemTheme() {
  ui.themeIsManual = false
  try {
    localStorage.removeItem(LS_THEME)
  } catch {}
  setTheme(resolve())
  ui.themeIsManual = false   // setTheme 会置 true，这里改回来
}

// 系统偏好变化：仅在用户没手动选过时跟随
if (mq) {
  const onChange = (e) => {
    if (ui.themeIsManual) return
    const m = e.matches ? 'dark' : 'light'
    ui.theme = m
    apply(m)
  }
  // addEventListener 在旧 Safari 上不存在，回落到 addListener
  if (mq.addEventListener) mq.addEventListener('change', onChange)
  else if (mq.addListener) mq.addListener(onChange)
}
