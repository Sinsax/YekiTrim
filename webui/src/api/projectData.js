// 项目数据 API —— **作用域解析的唯一出口**。
//
// 所有 `/api/project-data/*` 调用都在这里拼作用域（`api/scope.js` 的 `scopeQ(pid)`），
// 视图与组件只传不透明的 `pid`，永不自己拼 `project=` / `config=`。
import { api } from './client'
import { scopeQ } from './scope'

export const projectData = {
  // 读取类都收一个 `label`（业务说明，如「读取事件列表」）：失败时它会进
  // 控制台与设置页「运行日志」，否则 `.catch(() => [])` 一兜就什么都查不到了。
  status: (pid, label) => api(`/api/project-data/status?${scopeQ(pid)}`, { label }),
  events: (pid, label) => api(`/api/project-data/events?${scopeQ(pid)}`, { label }),
  clips: (pid, label) => api(`/api/project-data/clips?${scopeQ(pid)}`, { label }),
  polishMap: (pid, label) => api(`/api/project-data/polish?${scopeQ(pid)}`, { label }),
  config: (pid, label) => api(`/api/project-data/config?${scopeQ(pid)}`, { label }),
  transcript: (pid, label) => api(`/api/project-data/transcript?${scopeQ(pid)}`, { label }),

  /** 输入素材（弹幕 / 字幕）：当前值 + 状态 + 项目内的候选。预处理页用它渲染两行素材。 */
  assets: (pid, label) => api(`/api/project-data/assets?${scopeQ(pid)}`, { label }),
  /**
   * 选择/清除一类素材。`body` 传三选一：
   * - `{path: ''}` 清除；`{path: 'dm.xml'}` 项目内相对（或 admin 的绝对路径）；
   * - `{dir, name}` 服务器目录里选的（**目录与文件名分开发**，前端不拼路径：服务端回传的
   *   目录可能是 `\\?\F:\a` 这种 verbatim 形式，前端拼出来在 Windows 上是非法名）。
   * 服务端写前会解析一遍，失败不落盘，成功返回条数。
   */
  setAsset: (pid, kind, body) =>
    api(`/api/project-data/assets?${scopeQ(pid)}`, {
      method: 'POST',
      body: JSON.stringify({ kind, ...body })
    }),

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
    api(`/api/project-data/clips/${encodeURIComponent(rank)}/silence?${scopeQ(pid)}`, { method: 'POST' }),
  /** 删除一个已剪切片段（整目录，含精修产物）—— 破坏性，调用方必须先二次确认。
   *  `ref` 是**目录名**（卡片墙列的 `c.name`，服务端两种都收：数字 rank 或目录名）。 */
  deleteClip: (pid, ref) =>
    api(`/api/project-data/clips/${encodeURIComponent(ref)}?${scopeQ(pid)}`, { method: 'DELETE' }),
}
