<template>
  <Teleport to="body">
    <div v-if="show" class="modal-backdrop" @click.self="requestClose">
      <div class="modal" :class="{ 'modal-lg': wide }">
        <div class="modal-head">
          <h2>{{ title }}</h2>
          <button v-if="closable" class="modal-x" aria-label="关闭" @click="requestClose">
            <Icon name="x" :size="14" />
          </button>
        </div>
        <slot />
        <div v-if="$slots.footer" class="row mt-12">
          <slot name="footer" />
        </div>
      </div>
    </div>
  </Teleport>
</template>

<script setup>
import { watch, onUnmounted } from 'vue'
import Icon from './Icon.vue'

const props = defineProps({
  show: { type: Boolean, default: false },
  title: { type: String, default: '' },
  wide: { type: Boolean, default: false },
  closable: { type: Boolean, default: true }
})
const emit = defineEmits(['close'])

function requestClose() {
  if (props.closable) emit('close')
}

function onKey(e) {
  if (e.key === 'Escape') requestClose()
}

// ⚠️ 必须 `immediate`：调用方普遍写成 `<AppModal v-if="open" :show="true">`，组件**出生时
// `show` 已经是 true**，非 immediate 的 watcher 永远不会触发 ⇒ Esc 监听没装上、`body` 滚动也没锁，
// 用户会发现"Esc 关不掉弹窗、背后的页面跟着滚"。`immediate` 让首次挂载也走一遍装配。
watch(() => props.show, (v) => {
  if (v) {
    window.addEventListener('keydown', onKey)
    document.body.style.overflow = 'hidden'
  } else {
    window.removeEventListener('keydown', onKey)
    document.body.style.overflow = ''
  }
}, { immediate: true })

onUnmounted(() => {
  window.removeEventListener('keydown', onKey)
  document.body.style.overflow = ''
})
</script>

<style scoped>
.modal-head { display:flex; align-items:center; justify-content:space-between; gap:10px; margin-bottom:12px; }
.modal-head h2 { margin:0; font-size:16px; }
.modal-x { flex:none; width:28px; height:28px; border-radius:50%; border:1px solid var(--line); background:var(--card); color:var(--ink-dim); display:flex; align-items:center; justify-content:center; cursor:pointer; transition:all .15s; }
.modal-x:hover { color:var(--red-deep); border-color:var(--red); background:var(--red-soft); }
</style>
