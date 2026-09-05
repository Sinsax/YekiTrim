import { ref, computed } from 'vue'
import { api } from '../api/client'

const jobs = ref([])
let started = false
let timer = null
let es = null

async function load() {
  try { jobs.value = await api('/api/jobs') } catch { jobs.value = [] }
}

// 模块级单例：App 挂载后全应用共享一份轮询 + SSE，任何视图重复调用不会叠加定时器
export function useJobs() {
  if (!started) {
    started = true
    load()
    timer = setInterval(load, 3000)
    try {
      es = new EventSource('/api/stream')
      es.onmessage = () => load()
    } catch { /* SSE 不可用时靠定时轮询兜底 */ }
  }
  const hasRunning = computed(() =>
    jobs.value.some(j => j.status === 'running' || j.status === 'queued' || j.status === 'waiting_resume')
  )
  return { jobs, reload: load, hasRunning }
}
