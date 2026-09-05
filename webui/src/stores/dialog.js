import { reactive } from 'vue'

// 全局确认/输入对话框（沿袭 toast 的 reactive 单例模式）
// confirm()  → Promise<boolean>
// prompt()   → Promise<string|null>
export const dialogState = reactive({
  show: false,
  mode: 'confirm', // confirm | prompt
  title: '',
  message: '',
  placeholder: '',
  value: '',
  danger: false,
  resolve: null
})

export function confirmDialog(message, { title = '确认操作', danger = false } = {}) {
  return new Promise((resolve) => {
    Object.assign(dialogState, { show: true, mode: 'confirm', title, message, placeholder: '', value: '', danger, resolve })
  })
}

export function promptDialog(message, { title = '输入', placeholder = '', value = '', danger = false } = {}) {
  return new Promise((resolve) => {
    Object.assign(dialogState, { show: true, mode: 'prompt', title, message, placeholder, value, danger, resolve })
  })
}

export function settleDialog(ok) {
  const s = dialogState
  if (!s.show) return
  s.show = false
  const resolve = s.resolve
  s.resolve = null
  if (!resolve) return
  resolve(ok ? (s.mode === 'prompt' ? s.value : true) : (s.mode === 'prompt' ? null : false))
}
