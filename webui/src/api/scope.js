// 项目作用域（M2）：前端只有这一处拼 `project=<id>` 查询串。
//
// 服务端契约（P2）：所有 `/api/project-data/*` 与 `/api/polish/*` 都要求
// `project=<id>`（或 `{project, dir}`），**不再接受任意路径**。
import { reactive } from 'vue'

export const scopeState = reactive({
  /** 当前项目 id（来自路由 `:pid`） */
  pid: null,
  /** 当前项目行（来自 `/api/projects`，含 name/source/bytes） */
  project: null
})

export function setScope(pid, project = null) {
  scopeState.pid = pid == null || pid === '' ? null : String(pid)
  scopeState.project = project
}

/** 查询串片段：`project=12`（不含前导 `?`）。 */
export function scopeQ(pid = scopeState.pid) {
  return 'project=' + encodeURIComponent(pid == null ? '' : String(pid))
}

/** 带前导 `?` 的版本，便于 `url + scopeQuery()` 拼接。 */
export function scopeQuery(pid = scopeState.pid) {
  return '?' + scopeQ(pid)
}
