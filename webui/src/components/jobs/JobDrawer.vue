<template>
  <Teleport to="body">
    <div v-if="show" class="drawer-mask" @click.self="close">
      <div class="drawer">
        <div class="row spread mb-8">
          <b>运行任务</b>
          <span class="row">
            <button class="mini-btn" @click="expandAll = !expandAll">{{ expandAll ? '收起历史' : '查看全部' }}</button>
            <button class="mini-btn" @click="close">关闭</button>
          </span>
        </div>
        <JobList :limit="expandAll ? 50 : 6" actions @changed="reload" />
      </div>
    </div>
  </Teleport>
</template>

<script setup>
import { ref } from 'vue'
import JobList from '../JobList.vue'
import { useJobs } from '../../composables/useJobs'

defineProps({ show: { type: Boolean, default: false } })
const emit = defineEmits(['close'])

const expandAll = ref(false)
const { reload } = useJobs()

function close() {
  emit('close')
}
</script>

<style scoped>
.drawer-mask { position:fixed; inset:0; background:rgba(90,60,70,.18); z-index:60; }
.drawer { position:fixed; top:0; right:0; bottom:0; width:min(92vw,380px); background:var(--card); border-left:1px solid var(--line); box-shadow:var(--shadow-hover); padding:16px; overflow:auto; z-index:61; }
</style>
