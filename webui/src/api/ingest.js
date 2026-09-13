// 入库（M3）：录播姬会话记录 / 房间映射 / 待分配指派 / 重试。
import { api } from './client'

export const ingestApi = {
  /** 我的入库记录；`scope` = mine | all | unassigned（后两者仅 admin） */
  records: (scope = 'mine', limit = 50) => {
    const q = scope === 'mine' ? '' : `?${scope}=1`
    const sep = q ? '&' : '?'
    return api(`/api/ingest/records${q}${sep}limit=${limit}`)
  },
  channelMap: () => api('/api/channel-map'),
  saveChannelMap: (rules) =>
    api('/api/channel-map', { method: 'PUT', body: JSON.stringify({ rules }) }),
  assign: (session_id, user_id, auto_pipeline) =>
    api('/api/admin/ingest/assign', {
      method: 'POST',
      body: JSON.stringify({ session_id, user_id, auto_pipeline })
    }),
  retry: (session_id) =>
    api('/api/ingest/retry', { method: 'POST', body: JSON.stringify({ session_id }) }),
  config: () => api('/api/admin/ingest'),
  scanNow: () => api('/api/admin/ingest/scan', { method: 'POST' })
}

/** 自动策略档位（与服务端 auto_pipeline 一致）。 */
export const PIPELINES = [
  { value: 'none', label: '仅入库', hint: '只把录像变项目，不耗算力' },
  { value: 'transcribe', label: '转写', hint: '入库后自动提取音频 + 转写' },
  { value: 'detect', label: '检测（推荐）', hint: '转写 + 事件检测，不自动切片' },
  { value: 'cut', label: '切片', hint: '自动切到成片，会占用磁盘' }
]

export function pipelineLabel(v) {
  return PIPELINES.find(p => p.value === v)?.label || v || '-'
}
