// 项目列表（M2）：`/api/projects` 只回**本人**项目。
//
// 服务端 P2 起归属由 DB 决定，前端不再需要（也不应该）传工程路径。
import { reactive } from 'vue'
import { api } from '../api/client'

export const projectState = reactive({
  list: [],
  loading: false,
  error: ''
})

export async function loadProjects() {
  projectState.loading = true
  projectState.error = ''
  try {
    const d = await api('/api/projects')
    projectState.list = Array.isArray(d?.projects) ? d.projects : []
  } catch (e) {
    projectState.error = e?.message || '读取项目列表失败'
    projectState.list = []
  } finally {
    projectState.loading = false
  }
  return projectState.list
}

export function projectById(pid) {
  if (pid == null) return null
  return projectState.list.find(p => String(p.id) === String(pid)) || null
}

/** 来源展示文字：`录制 23058` / `B站下载` / `本机文件`。 */
export function sourceLabel(p) {
  const label = p?.source?.label
  if (label) return label
  switch (p?.source?.kind) {
    case 'recorder': return '录播姬录制'
    case 'bilibili': return 'B站下载'
    case 'local': return '本机文件'
    default: return '未知来源'
  }
}

export function humanBytes(n) {
  const v = Number(n) || 0
  if (v < 1024) return v + ' B'
  const units = ['KB', 'MB', 'GB', 'TB']
  let x = v / 1024
  let i = 0
  while (x >= 1024 && i < units.length - 1) {
    x /= 1024
    i++
  }
  return x.toFixed(1) + ' ' + units[i]
}

export async function deleteProject(pid) {
  await api('/api/projects/' + encodeURIComponent(pid), { method: 'DELETE' })
  await loadProjects()
}

export async function deleteProjectSource(pid) {
  const r = await api('/api/projects/' + encodeURIComponent(pid) + '/source', { method: 'DELETE' })
  await loadProjects()
  return r
}

/** 服务器本机文件浏览（仅 admin，囚笼在租户根内，路径一律相对）。 */
export async function listTenantFiles(path = '') {
  return api('/api/fs/list', { method: 'POST', body: JSON.stringify({ path }) })
}

/** 用服务器本机文件建项目（仅 admin）。 */
export async function createLocalProject({ dir, input = '', danmaku = '', output_dir = '' }) {
  const r = await api('/api/projects', {
    method: 'POST',
    body: JSON.stringify({ source: { kind: 'local', dir, input, danmaku, output_dir } })
  })
  await loadProjects()
  return r
}
