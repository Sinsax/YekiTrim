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

  /**
   * 输入素材（弹幕 / 字幕）的**清单**：每类的生效值 + 状态 + 项目内候选 + ASR 槽位。
   *
   * `slots`（`{hybrid: bool, single: bool}`）= 每个 ASR 模式的产物是否已经出过 ——
   * 前端**不在本地推**（判槽还要看盘上的分模式产物），置灰转写入口一律读它。
   */
  assets: (pid, label) => api(`/api/project-data/assets?${scopeQ(pid)}`, { label }),
  /**
   * 选择一份素材 = 置为**生效值** + 登记进清单（方便一键切回来）。`body` 三选一：
   * - `{path: 'dm.xml'}` 清单里的一条 / 项目内相对（或 admin 的绝对路径）；
   * - `{path: ''}` 清掉生效值（**清单不动**：候选还在，随时能切回来）；
   * - `{dir, name}` 服务器目录里选的（**目录与文件名分开发**，前端不拼路径：服务端回传的
   *   目录可能是 `\\?\F:\a` 这种 verbatim 形式，前端拼出来在 Windows 上是非法名）。
   * 服务端写前会解析一遍，失败不落盘，成功返回条数。
   */
  setAsset: (pid, kind, body) =>
    api(`/api/project-data/assets?${scopeQ(pid)}`, {
      method: 'POST',
      body: JSON.stringify({ kind, ...body })
    }),
  /**
   * 从清单里移除一条（`{action:'remove', path}`）—— 只改选择，**不删磁盘文件**。
   * 它正好是生效值时会连带清掉 `*_file`；ASR 产物（文件还在）服务端一律拒绝（400）。
   */
  removeAsset: (pid, kind, path) =>
    api(`/api/project-data/assets?${scopeQ(pid)}`, {
      method: 'POST',
      body: JSON.stringify({ kind, action: 'remove', path })
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
