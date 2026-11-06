// 出片回传（biliLive-tools 投稿）。
//
// YekiTrim 只负责把成片放到 biliLive-tools 容器看得见的暂存目录、调它的投稿 API；
// B站登录/分区/标签/封面/重试全在 biliLive-tools 里。
import { api } from './client'
import { scopeQuery } from './scope'

const qs = (pid) => scopeQuery(pid)

export const publishApi = {
  /** 读本项目已提交的任务与状态（biliLive-tools `/task` 回填）。 */
  state: (pid) => api(`/api/publish/bililive-tools${qs(pid)}`, { silent401: true }),
  /** 提交若干片段（rank 或目录名）；空数组 = 全部。**只建后台任务**，返回 `{id, queued, pending}`。 */
  publish: (pid, clips, presetId) =>
    api(`/api/publish/bililive-tools${qs(pid)}`, {
      method: 'POST',
      body: JSON.stringify({ clips: clips || [], preset_id: presetId || undefined })
    })
}
