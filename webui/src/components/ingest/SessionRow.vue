<template>
  <div class="event-row sess">
    <div class="flex-1">
      <div class="row">
        <b class="small">房间 {{ r.room_id || '-' }}</b>
        <span class="state-badge" :class="stateClass(r.state)">{{ r.state_label }}</span>
        <span class="muted small">{{ fmtTime(r.started_at) }}</span>
      </div>
      <div class="muted small mt-2">
        {{ r.file_count }} 个文件 · {{ fmtBytes(r.bytes) }}
        <template v-if="r.auto_pipeline && r.auto_pipeline !== 'none'"> · 自动 {{ pipelineLabel(r.auto_pipeline) }}</template>
      </div>
      <div v-if="r.error" class="small err mt-2">{{ r.error }}</div>
    </div>
    <div class="row sess-actions">
      <router-link v-if="r.project_id" class="mini-btn" :to="`/p/${r.project_id}/slices`">打开项目</router-link>
      <template v-else>
        <button v-if="r.state === 'failed'" class="mini-btn" @click="retry">重试</button>
        <slot name="actions" :r="r" />
      </template>
    </div>
  </div>
</template>

<script setup>
import { ingestApi, pipelineLabel } from '../../api/ingest'
import { toast } from '../../stores/toast'
import { fmtBytes, fmtTime, stateClass } from '../../stores/ingest'

const props = defineProps({
  r: { type: Object, required: true },
  compact: { type: Boolean, default: false }
})
const emit = defineEmits(['changed'])

async function retry() {
  try {
    await ingestApi.retry(props.r.session_id)
    toast('已重新排队')
    emit('changed')
  } catch (e) {
    toast(e.message, 'err')
  }
}
</script>

<style scoped>
/* 只补两处基准没有的：竖向对齐与紧凑态 */
.sess { align-items:flex-start; }
.sess-actions { flex:none; }
</style>
