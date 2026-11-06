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
// 事件↔片段对应关系**只有一份判据**（utils.js::clipMatchesEvent = Rust cutting::foreign_clip_conflict）。
// ⚠️ 方向只能是 stores → utils（utils 不许 import stores）。
import { uncutEventsOf as uncutEventsBetween } from '../utils'

const LS_LAST = 'yekitrim.wb.last'
const LS_EXPANDED = 'yekitrim.wb.expanded'

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

/** 正在飞的那次请求。**单飞（single-flight）**：这个接口对每个项目都要读
 *  `yekitrim.toml` + 扫一遍产物目录，项目在网络盘上时一次要几百毫秒到几秒；
 *  而任务结束、页面挂载、各视图初始化都会同时来一发 —— 不去重就会成串地打
 *  （实测服务器日志里 1 秒内 52 次），把服务端的线程全占住。
 *  并发的调用共享同一个请求，完成后再放行下一次。 */
let inflight = null

export function loadProjects() {
  if (inflight) return inflight
  wb.loading = true
  wb.error = ''
  inflight = api('/api/projects')
    .then((d) => {
      wb.projects = Array.isArray(d?.projects) ? d.projects : []
      return wb.projects
    })
    .catch((e) => {
      wb.error = e?.message || '读取项目列表失败'
      // ⚠️ **保留上一份快照**：读失败 ≠ 没有项目。清空会让侧栏与首页一起退化成
      // 「还没有项目：去首页「新增项目」」的假空态（首次就读失败时本来就是 []）。
      return wb.projects
    })
    .finally(() => {
      wb.loading = false
      inflight = null
    })
  return inflight
}

export function projectById(pid) {
  if (pid == null) return null
  return wb.projects.find((p) => String(p.id) === String(pid)) || null
}

/**
 * 侧栏项目排序：**最新加入的在最上面**（用户口径 2026-10-06：「更改左侧项目列表，
 * 默认新的在上面 / 最新加入的项目在上面」）。
 *
 * ⚠️ 服务端 `GET /api/projects` 是 `ORDER BY id`（**老的在前面**，`db.rs::list_projects`），
 * 这里必须自己翻过来 —— 别去改服务端：那条顺序还喂着别处（工作台卡片、CLI 列表），
 * 用户要的是**侧栏**这一处。
 * 排序键：`created_at` 降序（"最新加入"），同一秒（或老项目没有这个字段）时用 `id` 降序兜底 ——
 * 两个键都是"越新越大"，方向一致，不会出现顺序抖动。
 * ⚠️ 侧栏那枚**短标识**（`projectLabels`，`SideNav.vue`）刻意按 `id` 升序算 —— 列表顺序怎么变，
 * 每个项目的标识都不许跟着变（否则"项目 3"会变成另一个项目）。
 */
export function sortProjectsNewestFirst(list) {
  return [...(list || [])].sort((a, b) => {
    const ca = Number(a?.created_at) || 0
    const cb = Number(b?.created_at) || 0
    if (ca !== cb) return cb - ca
    return (Number(b?.id) || 0) - (Number(a?.id) || 0)
  })
}

/** 侧栏过滤后的项目（名称/房间前缀匹配；**最新加入的在前**）。 */
export const filteredProjects = computed(() => {
  const kw = wb.filter.trim().toLowerCase()
  const list = kw
    ? wb.projects.filter(
        (p) =>
          String(p.name || '').toLowerCase().includes(kw) ||
          String(p.room || '').toLowerCase().includes(kw)
      )
    : wb.projects
  return sortProjectsNewestFirst(list)
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

/** 并行的 status+events+clips+polish（合并原先分散在三个视图里的重复聚合）。
 *
 *  在飞时返回同一个 promise（单飞），但要记 `stale`：任务结束时 `invalidate()` 撞上
 *  一个**任务前**发出的请求，那个响应会把旧数据写回缓存 —— 没有这个标记，
 *  页面就永远停在任务前的状态（网络盘上这几个请求要跑好几秒，撞上的概率不低）。 */
export async function loadProjectData(pid, { force = false } = {}) {
  const c = cacheOf(pid)
  if (c.loading) {
    if (force) c.stale = true
    return c
  }
  if (!force && c.status) return c
  c.loading = true
  c.stale = false
  c.error = ''
  // 四条各自记录失败原因；**失败的那一条不覆盖已有数据**（读失败 ≠ 空态）。
  // 以前 events/clips/polish 都 .catch(() => []) —— 读失败被静默写成空数组，
  // 侧栏「已切 N · 待切 M」与首页切片总览会一起显示成 0，看起来像产物丢了。
  let failed = ''
  const noteFail = (what, e) => { failed = failed || (what + '：' + (e?.message || '读取失败')) }
  const [status, events, clips, polish] = await Promise.all([
    projectData.status(pid, '读取项目状态').catch((e) => { noteFail('状态', e); return null }),
    projectData.events(pid, '读取事件列表').catch((e) => { noteFail('事件', e); return null }),
    projectData.clips(pid, '读取切片列表').catch((e) => { noteFail('片段', e); return null }),
    projectData.polishMap(pid, '读取精修状态').catch((e) => { noteFail('精修', e); return null })
  ])
  if (status) c.status = status
  if (events) c.events = events.events || []
  if (clips) c.clips = Array.isArray(clips) ? clips : []
  if (polish) {
    const m = {}
    polish.forEach((p) => { m[p.dir] = p })
    c.polishMap = m
  }
  c.error = failed
  c.loading = false
  // 期间被 invalidate 过 → 这一份已经过期，立刻再拉一轮（避免"刷新了但界面还是旧的"）
  if (c.stale) {
    c.stale = false
    return loadProjectData(pid, { force: true })
  }
  return c
}

export function invalidate(pid, ...keys) {
  const c = cacheOf(pid)
  // 正在飞的那一轮取的是**失效前**的数据：标记一下，让它落地后自己再拉一轮
  if (c.loading) c.stale = true
  const list = keys.length ? keys : ['status', 'events', 'clips', 'polishMap']
  for (const k of list) {
    if (k === 'polishMap') c.polishMap = {}
    else if (k === 'clips') c.clips = []
    else if (k === 'events') c.events = []
    else if (k === 'status') c.status = null
  }
}

/**
 * 未剪切事件 = 缓存里**没有"属于自己的"片段**的事件。
 *
 * ⚠️ 判据**只有一份**：`utils.js::uncutEventsOf(events, clips)`（= 服务端
 * `cutting::foreign_clip_conflict` 的前端口径，认 `ev.clip_dir` / 标题 / 原片区间 IoU）。
 * 这里以前自己按 `clip-<rank>` 拼目录名猜归属 —— 只要目录名不是 rank（新的稳定 `clip_dir`）
 * 或重新检测后序号错位，侧栏「待切 N」就会把**已经剪完**的事件继续算成未切。
 * 回归断言（node，demo 数据）：clip_dir 与 rank 不同的片段必须判为已剪；
 * 委托 utils 版后两条路径（侧栏 / 切片页）逐条一致。
 */
export function uncutEventsOf(pid) {
  const c = cacheOf(pid)
  return uncutEventsBetween(c.events || [], c.clips || [])
}

/** 已出片（`rendered`）的片段数 —— 喂给侧栏/阶段徽标的「精修 N/M」。
 *  只认 `rendered`（视频），不含只有封面的：那个比例问的是"这条片段渲完了没"。
 *  注意它与卡片上那枚「已导出」chip 的口径**不同**（chip 是 `rendered || cover`）。 */
export function polishedCountOf(pid) {
  const c = cacheOf(pid)
  return Object.values(c.polishMap || {}).filter((p) => p.rendered).length
}

/**
 * 移除项目登记（**不动磁盘上的任何文件**），然后刷新列表。
 *
 * 名字叫 remove 而不是 delete：服务端只把登记从列表里去掉，目录里一个文件都不动。
 */
export async function removeProject(pid) {
  await api('/api/projects/' + encodeURIComponent(pid), { method: 'DELETE' })
  forgetProject(pid)
  await loadProjects()
}

export async function removeProjectSource(pid) {
  const r = await api('/api/projects/' + encodeURIComponent(pid) + '/source', { method: 'DELETE' })
  await loadProjects()
  return r
}

/**
 * 丢掉某个项目的**前端本地痕迹**（项目已被移除时调，见 `removeProject`）。
 *
 * 服务器那边只删了一行登记，而前端有三处按 pid 存的东西会永远留着：
 * ① `wb.cache[pid]` —— per-pid 数据缓存（`loadProjectData` 命中非空 `status` 就直接返回，
 *    是个不会自己过期的快照）；② `yekitrim.wb.expanded`；③ `yekitrim.wb.last`（`/` 的回落目标）。
 * ③ 目前靠 `lastPath()` 里的 `projectById` 守卫兜住才没炸，但那只是"看不见的错"，
 * 本地存储里一直躺着指向已移除项目的 pid。精修页还有一份 `yekitrim.polish.last.<pid>`。
 *
 * ⚠️ 项目 id 是 `INTEGER PRIMARY KEY AUTOINCREMENT`，SQLite 保证不会复用，
 * 所以这些残留不会"串到别的项目上"—— 清掉是为了不留垃圾，不是为了防串号。
 */
export function forgetProject(pid) {
  if (pid == null || pid === '') return
  const key = String(pid)
  delete wb.cache[key]
  const i = wb.expanded.indexOf(key)
  if (i >= 0) {
    wb.expanded.splice(i, 1)
    localStorage.setItem(LS_EXPANDED, JSON.stringify(wb.expanded))
  }
  if (String(wb.last?.pid || '') === key) {
    wb.last = { pid: '', step: wb.last?.step || 'prepare' }
    localStorage.setItem(LS_LAST, JSON.stringify(wb.last))
  }
  try { localStorage.removeItem('yekitrim.polish.last.' + key) } catch (e) { /* 隐私模式下会抛，忽略 */ }
}
