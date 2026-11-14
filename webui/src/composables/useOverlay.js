// 弹层通用行为：焦点圈定 / 初始焦点 / Esc 只关最上层 / 关闭后还焦点 / 滚动锁。
//
// 为什么要有这一份：`AppModal`、`AppDialog`、`JobDrawer` 三个弹层原先各写各的 ——
// 只有 AppModal 装了 Esc，AppDialog 在**整个容器**上挂 `@keydown.enter` 直接确认
// （危险确认也能被一次回车点掉），三个都没有焦点圈定与还焦点，关闭后焦点掉回 <body>，
// 键盘用户要重新从头 Tab 一遍。抽到这里之后新增弹层只要调用一次 `useOverlay`。
//
// ⚠️ 三个不能改坏的点：
//   1. **Esc 只关最上面那层**：监听在 window 的**捕获阶段**，取栈顶；否则两层弹层叠开时
//      一次 Esc 会把底下那层也关掉，Promise 直接 settle。
//   2. **滚动锁要计数**：两层弹层叠开时，关掉上层不许把 body 提前解锁
//      （原先每个组件各写 `body.style.overflow = ''`，上层一关背景就能滚）。
//   3. **关闭后还焦点**：记的是 `document.activeElement`，元素已经从文档里消失时
//      （列表项被删掉的确认框）不能硬调 focus，否则焦点掉到 <body>。
import { nextTick, onUnmounted, watch } from 'vue'

const FOCUSABLE = [
  'a[href]',
  'button:not([disabled])',
  'input:not([disabled]):not([type="hidden"])',
  'select:not([disabled])',
  'textarea:not([disabled])',
  '[tabindex]:not([tabindex="-1"])'
].join(',')

/** 当前打开的弹层栈（后进先出：Esc/Tab 只作用于栈顶） */
const stack = []
let listening = false
let lockCount = 0
let savedOverflow = ''

function isVisible(el) {
  if (!el || el.disabled) return false
  if (el.getAttribute?.('aria-hidden') === 'true') return false
  // 几何判据：`display:none` / `visibility:hidden` 的元素不能被 Tab 到
  const rect = el.getBoundingClientRect()
  return rect.width > 0 || rect.height > 0
}

function focusables(root) {
  if (!root?.querySelectorAll) return []
  return Array.from(root.querySelectorAll(FOCUSABLE)).filter(isVisible)
}

function onKeydown(event) {
  const top = stack[stack.length - 1]
  if (!top) return

  if (event.key === 'Escape') {
    // 捕获阶段拦下：组件的冒泡监听不再重复处理
    event.preventDefault()
    event.stopPropagation()
    top.close()
    return
  }

  if (event.key !== 'Tab') return
  const nodes = focusables(top.el)
  if (!nodes.length) {
    event.preventDefault()
    top.el?.focus?.()
    return
  }
  const first = nodes[0]
  const last = nodes[nodes.length - 1]
  const active = document.activeElement
  const inside = !!top.el?.contains?.(active)
  if (event.shiftKey) {
    if (!inside || active === first) {
      event.preventDefault()
      last.focus()
    }
  } else if (!inside || active === last) {
    event.preventDefault()
    first.focus()
  }
}

function bindListener() {
  if (listening) return
  listening = true
  window.addEventListener('keydown', onKeydown, true)
}

function lockScroll() {
  if (lockCount === 0) {
    savedOverflow = document.body.style.overflow
    document.body.style.overflow = 'hidden'
  }
  lockCount += 1
}

function unlockScroll() {
  if (lockCount === 0) return
  lockCount -= 1
  if (lockCount === 0) document.body.style.overflow = savedOverflow
}

/**
 * @param {object} opts
 * @param {() => boolean} opts.isOpen  弹层是否打开（传 getter，例：`() => props.show`）
 * @param {() => void} opts.onClose    请求关闭（Esc 走这里，由调用方决定怎么关）
 * @param {{ value: HTMLElement|null }} opts.rootRef  弹层根节点 ref
 * @param {(root: HTMLElement|null) => HTMLElement|null} [opts.initialFocus] 自定义初始焦点
 * @param {boolean} [opts.restoreFocus=true] 关闭后把焦点还给打开它的元素
 */
export function useOverlay({ isOpen, onClose, rootRef, initialFocus, restoreFocus = true }) {
  let entry = null
  let lastActive = null

  async function open() {
    if (entry) return
    lastActive = document.activeElement instanceof HTMLElement ? document.activeElement : null
    entry = { el: null, close: () => onClose?.() }
    stack.push(entry)
    bindListener()
    lockScroll()
    // 等 v-if 把 DOM 渲染出来再找焦点（弹层普遍是 `v-if` 控制，setup 时节点还不存在）
    await nextTick()
    if (!entry) {
      // 打开期间被立刻关掉：`close()` 已经出过栈、解过锁，这里只补一次兜底
      return
    }
    const root = rootRef?.value || null
    entry.el = root
    if (!root) return
    const preferred = initialFocus ? initialFocus(root) : null
    const target = preferred || root
    target?.focus?.()
  }

  function close() {
    if (!entry) return
    const i = stack.indexOf(entry)
    if (i >= 0) stack.splice(i, 1)
    entry = null
    unlockScroll()
    if (restoreFocus && lastActive && document.contains(lastActive)) lastActive.focus()
    lastActive = null
  }

  watch(isOpen, (v) => {
    if (v) open()
    else close()
  }, { immediate: true })

  onUnmounted(close)

  return { close }
}
