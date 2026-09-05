<template>
  <div>
    <div class="page-head"><div><h1>运行监控</h1></div></div>
    <div class="card">
      <h2>运行任务</h2>
      <div v-if="!jobs.length" class="muted">暂无任务</div>
      <div v-for="j in jobs" :key="j.id" style="padding:7px 0;border-bottom:1px dashed var(--line-soft)">
        <div class="row spread"><b>{{ j.kind }}</b><span class="chip" :class="statusClass(j.status)">{{ j.status }}</span></div>
        <div class="small muted">{{ j.message }} {{ j.total ? Math.round(j.current * 100 / j.total) + '%' : '' }}</div>
      </div>
    </div>
  </div>
</template>

<script setup>
import { ref, onMounted } from 'vue'
import { api } from '../api/client'

const jobs = ref([])

function statusClass(s) {
  return s === 'failed' ? 'r' : s === 'running' ? 'b' : 'g'
}

async function load() {
  try { jobs.value = await api('/api/jobs') } catch (e) { jobs.value = [] }
}

onMounted(load)
setInterval(load, 3000)
</script>
