import { ref, computed, watch } from 'vue'
import { api } from '../api/client'
import { authState, isAuthed } from '../stores/auth'

const jobs = ref([])
let started = false
let timer = null
let es = null

async function load() {
  if (!isAuthed()) { jobs.value = []; return }
  try { jobs.value = await api('/api/jobs') } catch { jobs.value = [] }
}

function start() {
  if (started || !isAuthed()) return
  started = true
  load()
  timer = setInterval(load, 3000)
  try {
    es = new EventSource('/api/stream')
    es.onmessage = () => load()
  } catch { /* SSE 不可用时靠定时轮询兜底 */ }
}

function stop() {
  if (timer) { clearInterval(timer); timer = null }
  if (es) { es.close(); es = null }
  started = false
  jobs.value = []
}

// 未登录时不轮询也不开 SSE：登录页/会话过期后不该继续打 401 接口
// （EventSource 对 401 会自动重连，会形成持续的无效请求）
watch(() => authState.user?.username || '', (name) => { name ? start() : stop() })

// 模块级单例：App 挂载后全应用共享一份轮询 + SSE，任何视图重复调用不会叠加定时器
export function useJobs() {
  start()
  const hasRunning = computed(() =>
    jobs.value.some(j => j.status === 'running' || j.status === 'queued')
  )
  return { jobs, reload: load, hasRunning }
}
