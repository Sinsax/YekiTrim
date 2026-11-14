<template>
  <Teleport to="body">
    <div v-if="show" class="drawer-mask" @click.self="close">
      <aside
        ref="rootRef"
        class="drawer"
        role="dialog"
        aria-modal="true"
        aria-labelledby="job-drawer-title"
        tabindex="-1"
      >
        <div class="row spread mb-8 drawer-head">
          <b id="job-drawer-title">运行任务</b>
          <span class="row drawer-head-actions">
            <button class="mini-btn" type="button" @click="expandAll = !expandAll">
              {{ expandAll ? '收起历史' : '查看全部' }}
            </button>
            <button ref="closeRef" class="mini-btn" type="button" @click="close">关闭</button>
          </span>
        </div>
        <JobList :limit="expandAll ? 50 : 6" actions @changed="reload" />
      </aside>
    </div>
  </Teleport>
</template>

<script setup>
import { computed, ref, watch } from 'vue'
import JobList from '../JobList.vue'
import { useJobs } from '../../composables/useJobs'
import { useOverlay } from '../../composables/useOverlay'

const props = defineProps({ show: { type: Boolean, default: false } })
const emit = defineEmits(['close'])

const expandAll = ref(false)
const rootRef = ref(null)
const closeRef = ref(null)
const { reload } = useJobs()

// 打开抽屉时立刻拉一次：轮询可能正处在失败/退避状态，抽屉是用户唯一的诊断入口，
// 不能让他盯着上一份可能已经很旧的快照（以前只在挂载时取数，抽屉一开就是死的）。
watch(() => props.show, (v) => { if (v) reload() })

function close() {
  emit('close')
}

// Esc / 焦点圈定 / 关闭后还焦点（还给顶栏那颗「任务」按钮）/ 滚动锁
useOverlay({
  isOpen: computed(() => props.show),
  onClose: close,
  rootRef,
  initialFocus: () => closeRef.value
})
</script>

<style scoped>
.drawer-mask { position:fixed; inset:0; background:var(--scrim-soft); z-index:60; }
.drawer { position:fixed; top:0; right:0; bottom:0; width:min(92vw,380px); background:var(--card);
  border-left:1px solid var(--line); box-shadow:var(--shadow-hover); padding:16px; overflow:auto; z-index:61; }
.drawer-head { position:sticky; top:-16px; z-index:1; padding:6px 0 8px; margin-top:-6px;
  background:var(--card); border-bottom:1px solid var(--line-soft); }
.drawer-head-actions { gap:6px; }
@media (max-width:520px) {
  .drawer { width:100vw; border-left:none; }
}
</style>
