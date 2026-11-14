import { reactive } from 'vue'

export const toastState = reactive({
  items: []
})

/** 同屏最多几条：再多就会盖住页面顶部的操作区（错误信息本来就该去看日志/错误条） */
const MAX_TOASTS = 4
/** 默认停留时长（ms）：长错误看一眼删不掉，给足时间 */
const DEFAULT_MS = 3200

let seed = 0

/**
 * 弹一条提示。
 *
 * @param {string} message 文案
 * @param {'ok'|'err'|'warn'} [type] 语义档位（决定左侧强调条颜色，正文一律用 --ink 保证对比度）
 *
 * ⚠️ 超出 `MAX_TOASTS` 时从**最旧**的一条开始丢：反过来的话新消息永远看不见。
 */
export function toast(message, type = 'ok', ms = DEFAULT_MS) {
  const id = ++seed
  toastState.items.push({ id, message: String(message ?? ''), type })
  if (toastState.items.length > MAX_TOASTS) {
    toastState.items.splice(0, toastState.items.length - MAX_TOASTS)
  }
  setTimeout(() => {
    const i = toastState.items.findIndex((x) => x.id === id)
    if (i >= 0) toastState.items.splice(i, 1)
  }, ms)
}

export function useToast() {
  return { toast }
}
