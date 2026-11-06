// 页面内的"展开/收起"状态：首帧同步读 localStorage，避免先展开再收起闪一下。
//
// 用法：
//   const { open, toggle } = useDisclosure('prepare.materials', false)
//
// key 会被拼成 `yekitrim.ui.<key>` 存盘；只认 '1' / '0'，其它值回落 defaultOpen
// （读盘抛错也一样回落 —— 隐私模式下 localStorage 可能直接不可用）。
import { ref } from 'vue'

export function useDisclosure(key, defaultOpen = false) {
  const storageKey = `yekitrim.ui.${key}`
  let initial = !!defaultOpen
  try {
    const raw = localStorage.getItem(storageKey)
    if (raw === '1') initial = true
    else if (raw === '0') initial = false
  } catch {}

  const open = ref(initial)

  function set(next) {
    open.value = !!next
    try { localStorage.setItem(storageKey, open.value ? '1' : '0') } catch {}
  }

  function toggle() {
    set(!open.value)
  }

  return { open, set, toggle, storageKey }
}
