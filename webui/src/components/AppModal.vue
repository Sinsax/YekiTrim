<template>
  <Teleport to="body">
    <div v-if="show" class="modal-backdrop" @click.self="requestClose">
      <div
        ref="rootRef"
        class="modal"
        :class="{ 'modal-lg': wide }"
        role="dialog"
        aria-modal="true"
        :aria-labelledby="titleId"
        tabindex="-1"
      >
        <div class="modal-head">
          <h2 :id="titleId">{{ title }}</h2>
          <button v-if="closable" class="modal-x" type="button" aria-label="关闭" @click="requestClose">
            <Icon name="x" :size="14" />
          </button>
        </div>
        <slot />
        <div v-if="$slots.footer" class="modal-foot row">
          <slot name="footer" />
        </div>
      </div>
    </div>
  </Teleport>
</template>

<script setup>
import { computed, ref } from 'vue'
import Icon from './Icon.vue'
import { useOverlay } from '../composables/useOverlay'

const props = defineProps({
  show: { type: Boolean, default: false },
  title: { type: String, default: '' },
  wide: { type: Boolean, default: false },
  closable: { type: Boolean, default: true }
})
const emit = defineEmits(['close'])

// 每个实例一个稳定 id：`aria-labelledby` 要指向真正的标题节点（无障碍工具据此播报"这是什么弹窗"）。
// 同页可能同时存在多个弹层，用随机后缀避免 id 撞车。
const titleId = `yt-modal-title-${Math.random().toString(36).slice(2, 9)}`

const rootRef = ref(null)
const isOpen = computed(() => props.show)

function requestClose() {
  if (props.closable) emit('close')
}

// Esc / Tab 圈定 / 还焦点 / 滚动锁 全在 useOverlay 里（见该文件头注释）。
// ⚠️ 初始焦点给**容器本身**（不是右上角关闭键）：里面有表单时要先读到标题与内容，
//    按 Tab 自然会落到第一个可交互元素。
useOverlay({ isOpen, onClose: requestClose, rootRef })
</script>

<style scoped>
.modal-head { display:flex; align-items:center; justify-content:space-between; gap:10px; margin-bottom:12px; }
.modal-head h2 { margin:0; font-size:16px; }
.modal-foot { display:flex; align-items:center; gap:8px; flex-wrap:wrap; margin-top:14px;
  padding-top:12px; border-top:1px dashed var(--line-soft); }
.modal-foot :deep(.flex-1) { min-width:0; }
.modal-x { flex:none; width:28px; height:28px; border-radius:50%; border:1px solid var(--line);
  background:var(--card); color:var(--ink-dim); display:flex; align-items:center; justify-content:center;
  cursor:pointer; transition:color .15s, border-color .15s, background-color .15s; }
.modal-x:hover { color:var(--red-deep); border-color:var(--red); background:var(--red-soft); }
</style>
