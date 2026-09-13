<template>
  <div>
    <div v-if="!shown.length" class="empty small">暂无任务</div>
    <div v-for="j in shown" :key="j.id" class="job-row">
      <div class="row spread">
        <b>{{ kindText(j.kind) }}</b>
        <span class="chip" :class="statusClass(j.status)">{{ statusText(j.status) }}</span>
      </div>
      <div v-if="showLine(j)" class="small muted">
        {{ j.message }}{{ j.total > 0 ? ' · ' + Math.round(j.current * 100 / j.total) + '%' : '' }}
      </div>
      <div v-if="j.status === 'running' && j.total > 0" class="progress mt-6">
        <div :style="{ width: Math.round(j.current * 100 / j.total) + '%' }"></div>
      </div>
      <div v-if="actions && isActive(j)" class="row mt-6">
        <button class="mini-btn danger" @click="cancel(j)">取消</button>
      </div>
    </div>
    <div v-if="limit && (jobs.value || []).length > shown.length" class="muted small mt-8">
      仅显示最近 {{ shown.length }} 条（点「查看全部」看完整历史）
    </div>
  </div>
</template>

<script setup>
import { computed } from 'vue'
import { toast } from '../stores/toast'
import { useJobs } from '../composables/useJobs'
import { isActive, jobsApi, kindText, statusClass, statusText } from '../api/jobs'

const props = defineProps({
  actions: { type: Boolean, default: false },
  limit: { type: Number, default: 0 }
})
const emit = defineEmits(['changed'])

const { jobs, reload } = useJobs()
const shown = computed(() => (props.limit > 0 ? (jobs.value || []).slice(0, props.limit) : jobs.value || []))

function showLine(j) {
  const quiet = (j.status === 'succeeded' && j.message === '完成') || (j.status === 'cancelled' && j.message === '已取消')
  return !quiet
}

async function cancel(j) {
  try {
    await jobsApi.cancel(j.id)
    toast('已请求取消')
    reload()
    emit('changed')
  } catch (e) {
    toast(e.message, 'err')
  }
}
</script>
