import { ref, computed, watch } from 'vue'
import { authState, isAuthed } from '../stores/auth'
import { jobsApi } from '../api/jobs'

const jobs = ref([])
let started = false
let timer = null
let es = null
const prevStatus = new Map()
const doneWatchers = []

/** 注册"任务刚结束"回调（失效矩阵：`stores/workbench.js` 用它刷新项目数据）。 */
export function onJobDone(fn) {
  doneWatchers.push(fn)
}

async function load() {
  if (!isAuthed()) {
    jobs.value = []
    return
  }
  let list = []
  try {
    list = await jobsApi.list()
  } catch {
    list = []
  }
  jobs.value = Array.isArray(list) ? list : []
  for (const j of jobs.value) {
    const prev = prevStatus.get(j.id)
    if (prev !== j.status && ['succeeded', 'failed', 'cancelled'].includes(j.status)) {
      for (const fn of doneWatchers) {
        try { fn(j) } catch (e) { /* 监听方自己兜错 */ }
      }
    }
    prevStatus.set(j.id, j.status)
  }
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
  prevStatus.clear()
}

// 未登录时不轮询也不开 SSE：登录页/会话过期后不该继续打 401 接口
watch(() => authState.user?.username || '', (name) => { name ? start() : stop() })

// 模块级单例：App 挂载后全应用共享一份轮询 + SSE，任何视图重复调用不会叠加定时器
export function useJobs() {
  start()
  const hasRunning = computed(() =>
    jobs.value.some((j) => j.status === 'running' || j.status === 'queued')
  )
  return { jobs, reload: load, hasRunning }
}
