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
  /**
   * 保存采集/录播姬配置（`PUT /api/admin/ingest`）。
   * 语义：键**缺席** = 保持原值；给空串/空数组 = 清空。`api_pass` 缺席即"不改密码"。
   */
  saveConfig: (patch) => api('/api/admin/ingest', { method: 'PUT', body: JSON.stringify(patch) }),
  /** 生成/关闭 webhook token（`generate` 时**只有这一次**响应里有明文 token） */
  rotateToken: (action = 'generate') =>
    api('/api/admin/ingest/token', { method: 'POST', body: JSON.stringify({ action }) }),
  scanNow: () => api('/api/admin/ingest/scan', { method: 'POST' }),

  // ---- B站链接通道（P4）----

  /**
   * 解析一个 B站链接（`POST /api/ingest/resolve`）。
   *
   * **薄壳、不落库** —— 用户每改一次输入就点一下，不会攒出没人要的会话。
   * 返回 `{ bvid, title, uploader, cover, duration_secs, pages:[{index,cid,part,duration_secs,estimate_bytes}],
   *          estimate_bytes, quality, downloader_ready, danmaku_total, view_total }`。
   */
  resolveBilibili: (url) =>
    api('/api/ingest/resolve', { method: 'POST', body: JSON.stringify({ url }) }),

  /**
   * 提交下载（`POST /api/ingest/bilibili`）。**每个 P 一个会话/一个项目**。
   *
   * `pages` 不传 = 只下第 1 P（**默认不全选**：23P 全下会直接撑爆配额）。
   * 返回 `{ accepted:[{session_id,page,title,cid,duration_secs,created}], skipped:[{...,reason,project_id}] }`。
   */
  submitBilibili: (url, pages, auto_pipeline) =>
    api('/api/ingest/bilibili', {
      method: 'POST',
      body: JSON.stringify({ url, pages, ...(auto_pipeline ? { auto_pipeline } : {}) })
    }),

  /** 下载器就绪位（任何登录用户可看；响应里**不含路径**）。 */
  downloaderStatus: () => api('/api/ingest/downloader', { timeout: 10000 }),

  /** 下载内置 yt-dlp（仅 admin；会校验 sha256）。 */
  installDownloader: () => api('/api/admin/downloader/install', { method: 'POST' })
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
