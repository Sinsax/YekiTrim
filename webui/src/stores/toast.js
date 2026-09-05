import { reactive } from 'vue'

export const toastState = reactive({
  items: []
})

let seed = 0

export function toast(message, type = 'ok') {
  const id = ++seed
  toastState.items.push({ id, message, type })
  setTimeout(() => {
    const i = toastState.items.findIndex(x => x.id === id)
    if (i >= 0) toastState.items.splice(i, 1)
  }, 3200)
}

export function useToast() {
  return { toast }
}
