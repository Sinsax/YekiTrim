// 精修 API 唯一出口：所有 polish 端点调用 + 后台任务订阅。
// 页面（PolishView / PolishProjectsView）只从这里拿数据，不重复实现。
import { api } from '../api/client'

// 九宫格格子名 — 必须与 Rust grid_rect() 的命名一致（bot-left / bot-right）
export const GRID_CELLS = [
  'top-left', 'top', 'top-right',
  'mid-left', 'center', 'mid-right',
  'bot-left', 'bottom', 'bot-right',
]

export const REGION_LABELS = {
  liver: '人物',
  danmaku_area: '弹幕区域',
  sticker_area: '贴纸区域',
  reaction_zone: '反应区',
  game_area: '游戏区',
}

export const fetchProjects = () => api('/api/polish/projects')

// 可用贴图来源：内置 emoji 码点池 + 全局贴纸库包（组名/tags），供 sticker_source 配置挑选
export const fetchStickerSources = () => api('/api/polish/sticker-sources')

// withPreview: 附加预览计算字段（sticker_map / vertical / loudness_curve），首访会触发响度测量
export const fetchProject = (dir, withPreview = false) =>
  api('/api/polish/project?dir=' + encodeURIComponent(dir) + (withPreview ? '&preview=1' : ''))

export const fetchSubtitles = (dir) =>
  api('/api/polish/subtitles?dir=' + encodeURIComponent(dir))

export function saveSubtitles(dir, segments) {
  return api('/api/polish/subtitles', { method: 'PUT', body: JSON.stringify({ dir, segments }) })
}

// patch: { segments?, layout? } 局部更新 polish.toml
export function saveProjectPatch(dir, patch) {
  return api('/api/polish/project', { method: 'PUT', body: JSON.stringify({ dir, ...patch }) })
}

export const selectVersion = (dir, version) =>
  api('/api/polish/select-version', { method: 'POST', body: JSON.stringify({ dir, version }) })

async function startJob(url, dir) {
  return api(url, { method: 'POST', body: JSON.stringify({ dir }) })
}

// 以下均返回 { ok, id }，用 watchJob 订阅进度
export const initPolish = (dir) => startJob('/api/polish/init', dir)
// LLM 生成：服务端在同一任务内自动执行 process 后处理（process 无需显式触发）
export const runLlm = (dir) => startJob('/api/polish/llm', dir)
// 隐式 process：时间轴保存后等内部触发用
export const runProcess = (dir) => startJob('/api/polish/process', dir)
export function runRender(dir, outputFile) {
  return api('/api/polish/render', { method: 'POST', body: JSON.stringify({ dir, output_file: outputFile || '' }) })
}

const JOB_DONE = new Set(['succeeded', 'failed', 'cancelled'])

// 订阅任务：SSE 优先，异常回退轮询 /api/jobs/:id。
// 回调: onEvent(JobEvent) / onSummary(JobSummary) / onDone(JobSummary)
// 返回 { close() }；无论何时都会先拉一次当前状态，避免订阅前已结束的任务收不到事件。
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
      const j = await api('/api/jobs/' + id)
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
    es = new EventSource('/api/jobs/' + id + '/events')
    es.onmessage = (ev) => {
      let e = null
      try { e = JSON.parse(ev.data) } catch (err) { return }
      if (onEvent) onEvent(e)
      if (e.type === 'completed') finish({ id, status: 'succeeded', message: '完成' })
      if (e.type === 'failed') finish({ id, status: 'failed', message: e.error || '失败' })
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
