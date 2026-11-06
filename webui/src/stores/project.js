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

/**
 * 侧栏那枚小徽标里的**来源类型**（9px 的药丸，只放得下类型）。
 *
 * ⚠ 本地来源**只回「本机」**，不要拼路径：服务端的 `source.label` 是
 * `本机 · <父目录>/<目录名>`（`api.rs`，非租户根）/ `本机文件 · <相对路径>`，
 * 而项目名本身就是那个目录名 —— 拼进去等于把名称显示两遍，还把徽标撑长。
 * 完整标签走 `sourceLabel(p)`，挂在徽标的 `title` 上，悬停仍能看到路径。
 */
export function sourceBadge(p) {
  if (p?.source?.kind === 'recorder' && p?.room) return `录制 ${p.room}`
  if (p?.source?.kind === 'bilibili') return 'B站'
  if (p?.source?.kind === 'local') return '本机'
  // 通用 webhook（biliLive-tools / 任意录制软件）：label 是「Webhook · platform · 房间 N」，
  // 整串挂进 9px 的药丸会把侧栏项目名挤成省略号 —— 徽标只放类型，完整标签在 title。
  if (p?.source?.kind === 'webhook') return 'Webhook'
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

/** 服务器本机文件浏览（仅 admin；管理员可传任意绝对路径，非管理员只能是租户根内相对路径）。
 *  `enter` 是"进入下一级"的子目录名，由服务端拼路径（前端拼会踩 `\\?\` 前缀的坑）。 */
export async function listTenantFiles(dir = '', enter = '') {
  return api('/api/fs/list', { method: 'POST', body: JSON.stringify({ dir, enter }) })
}

/** 用服务器本机文件建项目（仅 admin，永不接受上传）。已有 yekitrim.toml 时只登记不覆盖。 */
/**
 * 用「服务器本机文件」建项目 / 添加已有工程。
 *
 * ⚠️ `overwrite` 默认 **false**：目录里已有 `yekitrim.toml` 时后端**只登记/打开、不重写**
 * （保住用户手改过的工程设置）。只有"目录已有工程、用户又手动挑了另一个视频"那条路
 * —— 界面弹二次确认、用户点了确认 —— 才传 true 让它重建。
 */
export async function createLocalProject({ dir, input = '', danmaku = '', output_dir = '', overwrite = false }) {
  return api('/api/projects', {
    method: 'POST',
    body: JSON.stringify({ source: { kind: 'local', dir, input, danmaku, output_dir, overwrite } })
  })
}
