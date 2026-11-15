// 任务中心 API + 展示映射（中文类型名/状态名集中在这里，任务抽屉与各页共用）。
import { api } from './client'
import { addClient } from '../stores/log'

export const KIND_ZH = {
  pipeline: '处理流程',
  stage_extract: '提取音频',
  stage_transcribe: '识别声音',
  stage_detect: '查找事件',
  stage_cut: '剪出片段',
  stage_process: '一键处理',
  cut_all: '剪出全部片段',
  cut_one: '剪出片段',
  correct: '字幕订正',
  silence: '去静音',
  download_models: '下载模型',
  polish_init: '出片准备',
  polish_llm: '智能剪辑',
  polish_llm_segment: '重做本段',
  polish_process: '出片后处理',
  polish_render: '导出成片',
  polish_cover: '生成封面',
  ingest_pipeline: '采集后自动处理',
  publish: '投稿',
  ingest_none: '采集',
  ingest_transcribe: '采集后识别',
  ingest_detect: '采集后查找事件',
  ingest_cut: '采集后剪片段'
}

export const STATUS_ZH = {
  queued: '排队中',
  running: '运行中',
  succeeded: '完成',
  failed: '失败',
  cancelled: '已取消',
  lost: '记录已失效'
}

/**
 * 任务档位（后端 `server/jobs.rs::JobClass`）—— 按**本地资源占用**分，不按时长分。
 *
 * - `light` 轻活：不占并发闸门，资源不冲突就直接开跑（精修初始化/后处理、封面生成）；
 * - `medium` 略重：最多 `class_cap` 个并行，满了排队（单条剪切、LLM 检测/订正/剪辑、下载）；
 * - `heavy` 重活：名额更小，满了排队（转写、批量剪切、渲染、入库自动管线）。
 *
 * 档位豁免的只是**闸门名额**：撞上在跑任务的资源键时，轻活照样排队。
 */
export const CLASS_ZH = {
  light: '轻活',
  medium: '略重',
  heavy: '重活'
}

export function classText(cls) {
  return CLASS_ZH[cls] || ''
}

/**
 * 提交后拿到 `queued: true` 时的统一提示。
 *
 * 不再写"前面那条任务结束后自动开始"——排队有两种原因（本档闸门名额满了 / 资源键撞上在跑的活），
 * 具体哪一种、排在第几位，任务抽屉里的排队说明行会写明（见 `queueDetail`）。
 */
export const QUEUED_HINT = '已排队：轮到时自动开始（任务抽屉里能看到档位与排队位次）'

/** 档位说明（任务行状态 chip 的 title）：档位 + 并行上限 + 来源。 */
export function classTitle(job) {
  if (!job) return ''
  const cls = classText(job.class)
  if (!cls) return ''
  const cap = job.class_cap == null ? '不占并发闸门' : `并行上限 ${job.class_cap}`
  const src = job.priority === 'background' ? '系统自动任务，排在用户任务之后' : '用户点按的任务'
  return `${cls} · ${cap} · ${src}`
}

/**
 * 排队中的任务那一行说明：档位 + 并行上限 + **同档**名次 + 来源。
 *
 * 排队有两种原因（本档名额满了 / 资源键撞上在跑的活），这里只说清"这条属于哪一档、排第几"，
 * **不猜原因** —— 名次是同档队列里的位置，卡在资源冲突上也照算。
 *
 * 写在任务行已有的那行说明里（`.small.muted`，本来就换行）而不是塞进状态 chip：
 * chip 全应用共用，塞长文案会把窄抽屉里的行挤变形。
 */
export function queueDetail(job) {
  if (!job) return ''
  const cls = classText(job.class)
  if (!cls) return '排队中：轮到时自动开始'
  const cap = job.class_cap == null ? '不占并发闸门' : `并行上限 ${job.class_cap}`
  const pos = job.queue_position > 0 ? ` · 同档第 ${job.queue_position} 位` : ''
  const src = job.priority === 'background' ? ' · 系统自动任务' : ''
  return `${cls}任务（${cap}）${pos}${src} · 轮到时自动开始`
}

/**
 * 进行时行的统一拼装（docs/plan/job-progress-spec-2026-11-15.md §1 范本）——
 * JobList 的运行/失败行与任务抽屉（整块复用 JobList）共用这一个实现，别抄第二份。
 *
 * 段间用 ` · ` 连接，从左到右：
 * - 阶段段（只有服务端给了 `stage_zh` 才有）：`stage_index` 与 `stage_total` **都在** →
 *   `阶段 {i}/{N} · {stage_zh}`；任一缺失 → 只 `{stage_zh}`。这三个都是服务端可选增量字段，
 *   没上之前整段缺席 —— 行为与旧版逐字节一致（只有 message，百分比同旧式追加）；
 * - `message` 非空且 ≠ `stage_zh` → 追加（语义 = 阶段内一句话明细；与阶段名相同就不重复说）；
 * - `total > 0` → 追加 `{Math.round(current*100/total)}%`，与进度条同口径、只在行尾出现一次。
 *
 * queued 行（queueDetail）、卡住提示（stuckNote）与终态的静默规则不归它管 —— JobList 另有分支。
 */
export function jobLine(job) {
  if (!job) return ''
  const parts = []
  const zh = job.stage_zh
  if (zh) {
    parts.push(
      job.stage_index != null && job.stage_total != null
        ? `阶段 ${job.stage_index}/${job.stage_total} · ${zh}`
        : zh
    )
  }
  if (job.message && job.message !== zh) parts.push(job.message)
  if (job.total > 0) parts.push(`${Math.round(job.current * 100 / job.total)}%`)
  return parts.join(' · ')
}

export function kindText(kind) {
  if (KIND_ZH[kind]) return KIND_ZH[kind]
  if (kind && kind.startsWith('stage_')) return `预处理 · ${kind.slice(6)}`
  if (kind && kind.startsWith('polish_')) return `精修 · ${kind.slice(7)}`
  return kind || '任务'
}

export function statusText(status) {
  return STATUS_ZH[status] || status
}

export function statusClass(status) {
  switch (status) {
    case 'running': return 'p'
    case 'queued': return 'a'
    case 'succeeded': return 'g'
    case 'failed': return 'r'
    case 'cancelled': return 'n'
    default: return 'n'
  }
}

export function isActive(job) {
  return job && (job.status === 'running' || job.status === 'queued')
}

/**
 * 任务作用域（资源键）—— 与后端 `server/scope_key.rs` **必须一致**。
 *
 * 后端按资源键做互斥/排队：键相交的任务不会同时跑。前端用同一套键判断
 * "我这个片段/项目现在忙不忙"，所以按钮只在自己作用域被占用时才禁用 ——
 * 别的项目在渲染、别的片段在跑 LLM，都不该挡住你（用户要求）。
 */
export function clipKey(pid, dirName) {
  return `p${pid}:clip:${String(dirName || '').toLowerCase()}`
}

export function outKey(pid) {
  return `p${pid}:out`
}

export function idxKey(pid) {
  return `p${pid}:idx`
}

export const MODELS_KEY = 'models'
export const STICKERS_KEY = 'stickers'

/** 这些资源键上现在有没有活动任务？（有则返回那条任务，供文案显示是谁在跑） */
export function busyOn(jobs, keys) {
  const want = (Array.isArray(keys) ? keys : [keys]).filter(Boolean)
  if (!want.length) return null
  return (
    (jobs || []).find(
      (j) => isActive(j) && (j.resources || []).some((r) => want.includes(r))
    ) || null
  )
}

/**
 * 任务列表快照的**保鲜期**：超过它就别再拿这份快照做门控了。
 *
 * `busyOn` 只是"提前告诉用户要等"，**不是正确性保证** —— 正确性由服务端的资源键排队兜
 * （点下去最多是进队列，不会撞车）。所以宁可放行，也不能让按钮停在一个可能早就结束、
 * 却因为任务列表刷不出来而永远灰着的状态上：一次读取故障把功能锁死，代价远大于多一次排队。
 * `useJobs` 按这个时限算 `stale`，各视图用它把 `busyOn` 的结果降级成 `null`。
 */
export const SNAPSHOT_TTL_MS = 15000

/** 快照是否已过期到不可信（`loadedAt` 为 0 = 从没成功读过 → 视为过期）。 */
export function snapshotStale(loadedAt, now = Date.now()) {
  return !loadedAt || now - loadedAt > SNAPSHOT_TTL_MS
}

/** 运行中的任务静默多久就在行上提示"可能卡住"（秒）。 */
export const IDLE_WARN_SECS = 120

/**
 * 运行中的任务已经多久没有新事件了（秒）。`0` = 刚有动静 / 不是运行中。
 *
 * `updated_at` 由后端每次 `Job::push` 更新，所以这个值就是"底下还有没有在动"。
 * 外 ffmpeg / 渲染子进程挂住时，任务状态还停在 `running`，但事件早就不来了 ——
 * 用户看到的就是"卡住"，而界面上原本没有任何线索，只能干等。
 */
export function idleSecs(job, nowMs = Date.now()) {
  if (!job || job.status !== 'running' || !job.updated_at) return 0
  return Math.max(0, Math.floor(nowMs / 1000) - job.updated_at)
}

/** 运行中但久无进展时的提示文案（否则空串）。
 *
 * ⚠️ **两档必须分开**（v2 L3，2026-11-15）：
 * - `job.stalled`（后端判定：**有可量化进度且连续 5 分钟没变**）⇒ 真卡，给完整提示 + 醒目样式；
 * - 只是"没有新事件" ⇒ 弱提示，**不再断言"可能卡住"**。
 *   依据：静默 ≠ 卡住 —— 检测阶段曾整条链路零事件，那时这条判据让每个长任务都误报一次
 *   （实测「查找事件 · 已 3 分钟没有新进展 · 可能卡住」而检测正在正常跑）。
 *   后端心跳（`HEARTBEAT_SECS`=30s）会刷新 `updated_at`，所以这里通常也触发不了。
 */
export function stuckNote(job, nowMs = Date.now()) {
  if (!job || job.status !== 'running') return ''
  if (job.stalled === true) {
    return `⚠ 进度已很久没变 —— 可能卡在外部进程或网络盘上，可看任务日志最后一行，或取消后重试`
  }
  const idle = idleSecs(job, nowMs)
  if (idle < IDLE_WARN_SECS) return ''
  return `已 ${Math.floor(idle / 60)} 分钟没有新事件 —— 任务仍在运行中（该阶段可能不上报进度）`
}

/** 卡住提示是不是"真卡"（决定样式：只对 stalled 用醒目色）。 */
export function isStalled(job) {
  return !!(job && job.status === 'running' && job.stalled === true)
}

/**
 * 任务接口的**专用超时**：比默认松上限紧得多，因为这两条是 3 秒一次的轮询。
 *
 * 一次悬挂的 `/api/jobs` 会把 `useJobs` 的去重 `inflight` 永久占住（它只由 `.finally` 清），
 * 此后所有轮询都返回同一个未决 promise —— 任务列表与全部按作用域门控的按钮一起冻住。
 * 宁可超时报错（抽屉里看得见 + 有重试），也不能无声地卡死。
 */
const JOBS_TIMEOUT_MS = 10000

export const jobsApi = {
  list: (all = false) => api('/api/jobs' + (all ? '?all=1' : ''), { timeout: JOBS_TIMEOUT_MS }),
  detail: (id) =>
    api('/api/jobs/' + encodeURIComponent(id), { timeout: JOBS_TIMEOUT_MS }),
  cancel: (id) =>
    api('/api/jobs/' + encodeURIComponent(id) + '/cancel', { method: 'POST', timeout: JOBS_TIMEOUT_MS })
}

const JOB_DONE = new Set(['succeeded', 'failed', 'cancelled'])

/** 订阅任务：SSE 优先（服务端先回放历史），异常回退轮询 `/api/jobs/:id`。 */
export function watchJob(id, { onEvent, onSummary, onDone } = {}) {
  let closed = false
  let es = null
  let pollTimer = null

  function finish(summary) {
    if (closed) return
    closed = true
    cleanup()
    if (onDone) onDone(summary)
  }

  async function pollOnce() {
    if (closed) return
    try {
      const j = await jobsApi.detail(id)
      if (onSummary) onSummary(j)
      if (JOB_DONE.has(j.status)) finish(j)
    } catch (e) {
      // 404 = 这条任务永远查不到了：任务表在内存里，服务重启会清空全部历史
      // （浏览器标签页留宿一夜后就是这种现场：每 2 秒一条 404 刷到天明）。
      // 继续轮询不会有结果，收掉订阅并把"记录已失效"交给调用方。
      if (e && e.status === 404) {
        addClient('warn', `任务 ${id} 已不存在（服务重启会清空任务历史），停止轮询`, 'jobs')
        finish({ id, status: 'lost', message: '任务记录已不存在（服务重启会清空任务历史）' })
        return
      }
      // 其余轮询失败会自己重试；只在日志面板留一条 debug（否则每 2 秒刷一条 warn）
      addClient('debug', `任务 ${id} 轮询失败：${e.message}（下次重试）`, 'jobs')
    }
  }

  function startPolling() {
    if (pollTimer || closed) return
    pollTimer = setInterval(pollOnce, 2000)
    pollOnce()
  }

  pollOnce()
  try {
    es = new EventSource('/api/jobs/' + encodeURIComponent(id) + '/events')
    es.onopen = () => { pollOnce() }
    es.onmessage = (ev) => {
      let e = null
      try {
        e = JSON.parse(ev.data)
      } catch (err) {
        addClient('warn', `任务事件格式异常（已跳过）：${String(ev.data).slice(0, 120)}`, 'jobs')
        return
      }
      if (onEvent) onEvent(e)
      if (e.type === 'completed') finish({ id, status: 'succeeded', message: '完成' })
      if (e.type === 'failed') finish({ id, status: 'failed', message: e.error || '失败' })
      if (e.type === 'cancelled') finish({ id, status: 'cancelled', message: '已取消' })
    }
    es.onerror = () => {
      if (es) { es.close(); es = null }
      startPolling()
    }
  } catch (e) {
    startPolling()
  }
  // 首轮 `pollOnce()` 可能已经判定「记录已失效」并收尾（closed=true）：
  // 收尾时 `es` 还没建出来，别把这条 SSE 留在外面自己重连。
  if (closed && es) { es.close(); es = null }

  function cleanup() {
    if (es) { es.close(); es = null }
    if (pollTimer) { clearInterval(pollTimer); pollTimer = null }
  }

  return { close: () => { closed = true; cleanup() } }
}
