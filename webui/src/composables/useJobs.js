import { ref, computed, watch } from 'vue'
import { authState, isAuthed } from '../stores/auth'
import { jobsApi, snapshotStale } from '../api/jobs'
import { addClient } from '../stores/log'

const jobs = ref([])
/** 最后一次**成功**拉到任务列表的时刻（ms，0 = 从没成功过）。 */
const lastLoadedAt = ref(0)
/** 最近一次读取失败的原因（成功后清空）。`JobList` 用它区分"读失败"与"真的没有任务"。 */
const lastError = ref('')
/** 快照是否已过期到不可信 —— 各视图据此把 `busyOn` 的门控降级成放行（见 `SNAPSHOT_TTL_MS`）。 */
const stale = ref(true)
let started = false
let timer = null
let es = null
const prevStatus = new Map()
const doneWatchers = []

/**
 * 注册"任务刚结束"回调（失效矩阵：`stores/workbench.js` 用它刷新项目数据）。
 *
 * **返回退订函数**：视图会因切换项目/片段反复挂载（`App.vue` 的 `router-view :key=pid`），
 * 不退订就会攒下一串陈旧回调 —— 任务一结束它们同时执行各自那一整套 project-data 请求，
 * 几十次挂载后就是上百并发（服务器日志里见过 1 秒 52 次连发的同款风暴）。
 * 注册方必须在 `onUnmounted` 里调用返回的函数。
 */
export function onJobDone(fn) {
  doneWatchers.push(fn)
  return () => {
    const i = doneWatchers.indexOf(fn)
    if (i >= 0) doneWatchers.splice(i, 1)
  }
}

/** 在飞请求合并：并发调用共享同一个请求（3s 轮询 + SSE 合帧 + 各视图 reload 都走它）。 */
let inflight = null
/** SSE 合帧定时器：事件流按事件推（进度/日志约 1.5s 一条，全切每片一条），
 *  每条都去拉一次列表会把 `/api/jobs` 打成洪水。 */
let coalesceTimer = null

/** 任务列表里刚出现的终态 → 通知失效矩阵。 */
function detectDone() {
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

async function load() {
  if (!isAuthed()) {
    jobs.value = []
    lastLoadedAt.value = 0
    lastError.value = ''
    stale.value = true
    return
  }
  if (inflight) return inflight
  inflight = (async () => {
    try {
      const list = await jobsApi.list()
      jobs.value = Array.isArray(list) ? list : []
      lastLoadedAt.value = Date.now()
      lastError.value = ''
      detectDone()
    } catch (e) {
      // **读不到 ≠ 没有任务**：保留上一份快照，把失败明说出来。
      // 以前这里写 `jobs.value = []`，于是"读取失败"和"真的没有任务"在界面上长得一模一样
      // （抽屉里都是「暂无任务」），既是误报也没有任何重试入口。
      // 保留快照的代价由 `stale` 兜住：过期后门控自动降级为放行，不会锁死按钮。
      lastError.value = e.message
      addClient('warn', `读取任务列表失败：${e.message}`, 'jobs')
    } finally {
      inflight = null
      stale.value = snapshotStale(lastLoadedAt.value)
    }
    return jobs.value
  })()
  return inflight
}

/** 事件流触发的那条路：合并成 250ms 一次（轮询仍在兜底）。 */
function loadSoon() {
  if (coalesceTimer) return
  coalesceTimer = setTimeout(() => { coalesceTimer = null; load() }, 250)
}

function start() {
  if (started || !isAuthed()) return
  started = true
  load()
  timer = setInterval(load, 3000)
  try {
    es = new EventSource('/api/stream')
    es.onmessage = () => loadSoon()
    // 断线由 EventSource 自己重连（3 秒轮询也一直在兜底）：只留一条 debug 痕迹
    es.onerror = () => addClient('debug', '任务事件流断开，正在重连（期间靠轮询）', 'jobs')
  } catch (e) {
    addClient('debug', `任务事件流不可用（${e.message}），改用轮询`, 'jobs')
  }
}

function stop() {
  if (timer) { clearInterval(timer); timer = null }
  if (es) { es.close(); es = null }
  started = false
  jobs.value = []
  prevStatus.clear()
  lastLoadedAt.value = 0
  lastError.value = ''
  stale.value = true
}

// 未登录时不轮询也不开 SSE：登录页/会话过期后不该继续打 401 接口
watch(() => authState.user?.username || '', (name) => { name ? start() : stop() })

// 模块级单例：App 挂载后全应用共享一份轮询 + SSE，任何视图重复调用不会叠加定时器
export function useJobs() {
  start()
  const hasRunning = computed(() =>
    jobs.value.some((j) => j.status === 'running' || j.status === 'queued')
  )
  /** 是否已经成功读到过一份快照（区分"读取中"与"真的没有任务"）。 */
  const loaded = computed(() => lastLoadedAt.value > 0)
  return { jobs, reload: load, hasRunning, lastError, stale, loaded, lastLoadedAt }
}

