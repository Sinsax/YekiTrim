// 项目列表与租户文件（M2/M3 起的项目托管接口）。
//
// - `workbench.js` 持有列表状态；
// - 这里保留"写操作 + 展示工具"（删除项目/删源/建项目/浏览租户文件/单位格式化）。
import { api } from '../api/client'

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

export function sourceBadge(p) {
  if (p?.source?.kind === 'recorder' && p?.room) return `录制 ${p.room}`
  if (p?.source?.kind === 'bilibili') return 'B站'
  return sourceLabel(p)
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
  return api('/api/projects/' + encodeURIComponent(pid), { method: 'DELETE' })
}

export async function deleteProjectSource(pid) {
  return api('/api/projects/' + encodeURIComponent(pid) + '/source', { method: 'DELETE' })
}

/** 服务器本机文件浏览（仅 admin；囚笼在允许的根内，路径一律相对所选根）。 */
export async function listTenantFiles(path = '', root = '') {
  return api('/api/fs/list', { method: 'POST', body: JSON.stringify({ path, root }) })
}

/** 用服务器本机文件建项目（仅 admin，永不接受上传）。已有 vtrim.toml 时只登记不覆盖。 */
export async function createLocalProject({ root = '', dir, input = '', danmaku = '', output_dir = '' }) {
  return api('/api/projects', {
    method: 'POST',
    body: JSON.stringify({ source: { kind: 'local', root, dir, input, danmaku, output_dir } })
  })
}
