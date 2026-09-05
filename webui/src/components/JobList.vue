<template>
  <div>
    <div v-if="!jobs.length" class="empty small">暂无任务</div>
    <div v-for="j in jobs" :key="j.id" class="job-row">
      <div class="row spread">
        <b>{{ j.kind }}</b>
        <span class="chip" :class="statusClass(j.status)">{{ j.status }}</span>
      </div>
      <div class="small muted">
        {{ j.message }}{{ j.total > 0 ? ' · ' + Math.round(j.current * 100 / j.total) + '%' : '' }}
      </div>
      <div v-if="j.status === 'running' && j.total > 0" class="progress mt-6">
        <div :style="{ width: Math.round(j.current * 100 / j.total) + '%' }"></div>
      </div>
      <div v-if="actions && isActive(j)" class="row mt-6">
        <button class="mini-btn danger" @click="cancel(j)">取消</button>
        <template v-if="j.status === 'waiting_resume'">
          <button class="mini-btn" @click="decide(j, true)">继续</button>
          <button class="mini-btn" @click="decide(j, false)">重来</button>
        </template>
      </div>
    </div>
  </div>
</template>

<script setup>
import { api } from '../api/client'
import { toast } from '../stores/toast'
import { useJobs } from '../composables/useJobs'
import { statusClass } from '../utils'

defineProps({ actions: { type: Boolean, default: false } })

const { jobs, reload } = useJobs()

function isActive(j) {
  return j.status === 'running' || j.status === 'queued' || j.status === 'waiting_resume'
}

async function cancel(j) {
  try {
    await api('/api/jobs/' + j.id + '/cancel', { method: 'POST' })
    toast('已请求取消')
    reload()
  } catch (e) { toast(e.message, 'err') }
}

async function decide(j, decision) {
  try {
    await api('/api/jobs/' + j.id + '/decision', { method: 'POST', body: JSON.stringify({ decision }) })
    toast(decision ? '继续任务' : '已选择重来')
    reload()
  } catch (e) { toast(e.message, 'err') }
}
</script>
