// 工作台全局状态：项目列表 + 侧栏（展开/过滤）+ 每项目数据缓存。
//
// 设计（对齐 docs/plan/webui-workbench-redesign.md §4）：
// - 列表来自 `GET /api/projects`（只含本人项目）；
// - 侧栏节点的精确计数（待切/精修进度）来自 `useProjectStore(pid)` 的缓存，
//   没加载过的项目只按 `stage/has_clips` 粗粒度显示，不为了徽标打一堆请求；
// - 任务完成后由 `composables/useJobs.js` 的失效矩阵触发 invalidate。
import { computed, reactive } from 'vue'
import { api } from '../api/client'
import { projectData } from '../api/projectData'

const LS_LAST = 'vtrim.wb.last'
const LS_EXPANDED = 'vtrim.wb.expanded'

function readLast() {
  try {
    const raw = JSON.parse(localStorage.getItem(LS_LAST) || 'null')
    return raw && typeof raw === 'object' ? raw : { pid: null, step: null }
  } catch {
    return { pid: null, step: null }
  }
}

function readExpanded() {
  try {
    const raw = JSON.parse(localStorage.getItem(LS_EXPANDED) || '[]')
    return Array.isArray(raw) ? raw.map(String) : []
  } catch {
    return []
  }
}

export const wb = reactive({
  projects: [],
  loading: false,
  error: '',
  filter: '',
  expanded: readExpanded(),
  last: readLast(),
  /** pid → per-project 数据缓存（status/events/clips/polishMap） */
  cache: {}
})

export function loadProjects() {
  wb.loading = true
  wb.error = ''
  return api('/api/projects')
    .then((d) => {
      wb.projects = Array.isArray(d?.projects) ? d.projects : []
      return wb.projects
    })
    .catch((e) => {
      wb.error = e?.message || '读取项目列表失败'
      wb.projects = []
      return []
    })
    .finally(() => {
      wb.loading = false
    })
}

export function projectById(pid) {
  if (pid == null) return null
  return wb.projects.find((p) => String(p.id) === String(pid)) || null
}

/** 侧栏过滤后的项目（名称/房间前缀匹配）。 */
export const filteredProjects = computed(() => {
  const kw = wb.filter.trim().toLowerCase()
  if (!kw) return wb.projects
  return wb.projects.filter(
    (p) =>
      String(p.name || '').toLowerCase().includes(kw) ||
      String(p.room || '').toLowerCase().includes(kw)
  )
})

export function isExpanded(pid) {
  return wb.expanded.includes(String(pid))
}

export function toggleExpand(pid) {
  const key = String(pid)
  const i = wb.expanded.indexOf(key)
  if (i >= 0) wb.expanded.splice(i, 1)
  else wb.expanded.push(key)
  localStorage.setItem(LS_EXPANDED, JSON.stringify(wb.expanded))
}

export function rememberLast(pid, step) {
  if (pid) wb.last = { pid: String(pid), step: step || wb.last.step || 'prepare' }
  else if (step) wb.last = { ...wb.last, step }
  localStorage.setItem(LS_LAST, JSON.stringify(wb.last))
}

/** `/` 的 redirect 目标。 */
export function lastPath() {
  if (wb.last?.pid && projectById(wb.last.pid)) {
    return `/p/${encodeURIComponent(wb.last.pid)}/${wb.last.step || 'prepare'}`
  }
  return '/workbench'
}

// ---- per-project 数据缓存 ----

export function cacheOf(pid) {
  const key = String(pid)
  if (!wb.cache[key]) {
    wb.cache[key] = { status: null, events: [], clips: [], polishMap: {}, loading: false, error: '' }
  }
  return wb.cache[key]
}

/** 并行的 status+events+clips+polish（合并原先分散在三个视图里的重复聚合）。 */
export async function loadProjectData(pid, { force = false } = {}) {
  const c = cacheOf(pid)
  if (c.loading) return c
  if (!force && c.status) return c
  c.loading = true
  c.error = ''
  const [status, events, clips, polish] = await Promise.all([
    projectData.status(pid).catch((e) => { c.error = e?.message || ''; return null }),
    projectData.events(pid).catch(() => ({ events: [] })),
    projectData.clips(pid).catch(() => []),
    projectData.polishMap(pid).catch(() => [])
  ])
  c.status = status
  c.events = (events && events.events) || []
  c.clips = Array.isArray(clips) ? clips : []
  const m = {}
  ;(polish || []).forEach((p) => { m[p.dir] = p })
  c.polishMap = m
  c.loading = false
  return c
}

export function invalidate(pid, ...keys) {
  const c = cacheOf(pid)
  const list = keys.length ? keys : ['status', 'events', 'clips', 'polishMap']
  for (const k of list) {
    if (k === 'polishMap') c.polishMap = {}
    else if (k === 'clips') c.clips = []
    else if (k === 'events') c.events = []
    else if (k === 'status') c.status = null
  }
}

/** 未剪切事件（events 里没有对应 clip.mp4 的）。 */
export function uncutEventsOf(pid) {
  const c = cacheOf(pid)
  const clipNames = new Set((c.clips || []).map((x) => x.name))
  return (c.events || []).filter((ev) => !clipNames.has('clip-' + String(ev.rank).padStart(2, '0')))
}

/** 已精修（已渲染）片段数。 */
export function polishedCountOf(pid) {
  const c = cacheOf(pid)
  return Object.values(c.polishMap || {}).filter((p) => p.rendered).length
}

export async function removeProject(pid) {
  await api('/api/projects/' + encodeURIComponent(pid), { method: 'DELETE' })
  await loadProjects()
}

export async function removeProjectSource(pid) {
  const r = await api('/api/projects/' + encodeURIComponent(pid) + '/source', { method: 'DELETE' })
  await loadProjects()
  return r
}
