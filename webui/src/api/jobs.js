// 任务中心 API + 展示映射（中文类型名/状态名集中在这里，任务抽屉与各页共用）。
import { api } from './client'

export const KIND_ZH = {
  pipeline: '流水线',
  stage_extract: '音频提取',
  stage_transcribe: '语音转文字',
  stage_detect: '事件检测',
  stage_cut: '剪切',
  cut_all: '一键全切',
  cut_one: '单事件剪切',
  correct: 'LLM 订正',
  silence: '去静音',
  download_models: '下载模型',
  polish_init: '精修初始化',
  polish_llm: 'LLM 剪辑',
  polish_process: '精修后处理',
  polish_render: '渲染导出',
  polish_cover: '封面生成',
  ingest_pipeline: '入库后自动处理',
  ingest_none: '入库',
  ingest_transcribe: '入库后转写',
  ingest_detect: '入库后检测',
  ingest_cut: '入库后切片'
}

export const STATUS_ZH = {
  queued: '排队中',
  running: '运行中',
  succeeded: '完成',
  failed: '失败',
  cancelled: '已取消'
}

export function kindText(kind) {
  if (KIND_ZH[kind]) return KIND_ZH[kind]
  if (kind && kind.startsWith('stage_')) return `预处理 · ${kind.slice(6)}`
  if (kind && kind.startsWith('polish_')) return `精修 · ${kind.slice(7)}`
  return kind || '任务'
}

export function statusText(status) {
  return STATUS_ZH[status] || status
}

export function statusClass(status) {
  switch (status) {
    case 'running': return 'p'
    case 'queued': return 'a'
    case 'succeeded': return 'g'
    case 'failed': return 'r'
    case 'cancelled': return 'n'
    default: return 'n'
  }
}

export function isActive(job) {
  return job && (job.status === 'running' || job.status === 'queued')
}

export const jobsApi = {
  list: (all = false) => api('/api/jobs' + (all ? '?all=1' : '')),
  detail: (id) => api('/api/jobs/' + encodeURIComponent(id)),
  cancel: (id) => api('/api/jobs/' + encodeURIComponent(id) + '/cancel', { method: 'POST' })
}

const JOB_DONE = new Set(['succeeded', 'failed', 'cancelled'])

/** 订阅任务：SSE 优先（服务端先回放历史），异常回退轮询 `/api/jobs/:id`。 */
export function watchJob(id, { onEvent, onSummary, onDone } = {}) {
  let closed = false
  let es = null
  let pollTimer = null

  function finish(summary) {
    if (closed) return
    closed = true
    cleanup()
    if (onDone) onDone(summary)
  }

  async function pollOnce() {
    if (closed) return
    try {
      const j = await jobsApi.detail(id)
      if (onSummary) onSummary(j)
      if (JOB_DONE.has(j.status)) finish(j)
    } catch (e) { /* 下次轮询重试 */ }
  }

  function startPolling() {
    if (pollTimer || closed) return
    pollTimer = setInterval(pollOnce, 2000)
    pollOnce()
  }

  pollOnce()
  try {
    es = new EventSource('/api/jobs/' + encodeURIComponent(id) + '/events')
    es.onopen = () => { pollOnce() }
    es.onmessage = (ev) => {
      let e = null
      try { e = JSON.parse(ev.data) } catch (err) { return }
      if (onEvent) onEvent(e)
      if (e.type === 'completed') finish({ id, status: 'succeeded', message: '完成' })
      if (e.type === 'failed') finish({ id, status: 'failed', message: e.error || '失败' })
      if (e.type === 'cancelled') finish({ id, status: 'cancelled', message: '已取消' })
    }
    es.onerror = () => {
      if (es) { es.close(); es = null }
      startPolling()
    }
  } catch (e) {
    startPolling()
  }

  function cleanup() {
    if (es) { es.close(); es = null }
    if (pollTimer) { clearInterval(pollTimer); pollTimer = null }
  }

  return { close: () => { closed = true; cleanup() } }
}
