// 项目数据 API —— **作用域解析的唯一出口**。
//
// 所有 `/api/project-data/*` 调用都在这里拼作用域（`api/scope.js` 的 `scopeQ(pid)`），
// 视图与组件只传不透明的 `pid`，永不自己拼 `project=` / `config=`。
import { api } from './client'
import { scopeQ } from './scope'

export const projectData = {
  status: (pid) => api(`/api/project-data/status?${scopeQ(pid)}`),
  events: (pid) => api(`/api/project-data/events?${scopeQ(pid)}`),
  clips: (pid) => api(`/api/project-data/clips?${scopeQ(pid)}`),
  polishMap: (pid) => api(`/api/project-data/polish?${scopeQ(pid)}`),
  config: (pid) => api(`/api/project-data/config?${scopeQ(pid)}`),
  transcript: (pid) => api(`/api/project-data/transcript?${scopeQ(pid)}`),

  /** 切片缩略图 URL（`<img :src>`/`<video :src>` 直接用，cookie 自动带上） */
  thumbUrl: (pid, rank) => `/api/project-data/clips/${encodeURIComponent(rank)}/thumb?${scopeQ(pid)}`,

  correct: (pid) => api(`/api/project-data/correct?${scopeQ(pid)}`, { method: 'POST' }),

  /** LS 修正后的项目配置（patch：`{section:{key:value}}`） */
  patchConfig: (pid, patch) =>
    api(`/api/project-data/config?${scopeQ(pid)}`, {
      method: 'PUT',
      body: JSON.stringify({ patch })
    }),
  putConfigRaw: (pid, raw) =>
    api(`/api/project-data/config?${scopeQ(pid)}`, {
      method: 'PUT',
      body: JSON.stringify({ raw })
    }),

  saveTranscript: (pid, segments, language = 'zh') =>
    api(`/api/project-data/transcript?${scopeQ(pid)}`, {
      method: 'PUT',
      body: JSON.stringify({ segments, language })
    }),

  createEvent: (pid, event) =>
    api(`/api/project-data/events?${scopeQ(pid)}`, {
      method: 'POST',
      body: JSON.stringify(event)
    }),
  saveEvents: (pid, events) =>
    api(`/api/project-data/events?${scopeQ(pid)}`, {
      method: 'PUT',
      body: JSON.stringify({ events })
    }),

  /** 跑管线：`{stage}` 或 `{stages,dry_run}` */
  pipeline: (pid, body) =>
    api(`/api/project-data/pipeline?${scopeQ(pid)}`, {
      method: 'POST',
      body: JSON.stringify(body)
    }),

  cutOne: (pid, rank) =>
    api(`/api/project-data/events/${encodeURIComponent(rank)}/cut?${scopeQ(pid)}`, { method: 'POST' }),
  cutAll: (pid, body = {}) =>
    api(`/api/project-data/events/cut-all?${scopeQ(pid)}`, {
      method: 'POST',
      body: JSON.stringify(body)
    }),
  silenceClip: (pid, rank) =>
    api(`/api/project-data/clips/${encodeURIComponent(rank)}/silence?${scopeQ(pid)}`, { method: 'POST' })
}
