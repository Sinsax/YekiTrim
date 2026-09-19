// 通用前端工具：媒体路径 / 哈希贴纸 / 事件类型 / 时间格式化

/**
 * 「重新检测」的二次确认文案（切片页「重新检测」与预处理页「开始事件检测」共用）。
 *
 * 事实依据（`service/core_pipeline.rs` 的 `"detect"` 分支 + `EventDetector::run`）：
 * 重跑只写 `output/events.json`（整份覆盖，手动改过的标题/时间也一起没），
 * **不碰** `output/clips/` —— 已剪片段仍在，但 rank 是按热度重排的，目录名 `clip-NN`
 * 与新事件的对应关系会错位，所以要提醒"需要时删掉重切"。
 */
export function redetectWarning(count = 0) {
  const n = Number(count) || 0
  const head = n
    ? `将重新跑一次 LLM 事件检测，覆盖当前的 ${n} 个事件：标题、摘要、时间、类型全部重算（手动改过的也一样）。`
    : '将重新跑一次 LLM 事件检测，覆盖上一次的检测结果。'
  return head
    + '\n已经剪好的片段不会被覆盖或删除（clips 目录原样保留），但事件会重新编号，'
    + '旧片段和新事件的对应关系可能对不上 —— 需要时把旧片段删掉重切。'
}

export const EVENT_TYPES = [
  'gameplay', 'interaction', 'qna', 'reaction', 'storytelling',
  'music', 'food', 'rant', 'break', 'outro', 'chat', 'eating',
  'irl', 'ranking', 'discussion', 'intro', 'other'
]

/** 旧契约：事件 rank → 片段目录名（`clip-{rank:02}`）。新片段请用 `ev.clip_dir`。 */
export function clipNameOf(rank) {
  const n = Math.max(0, Number(rank) || 0)
  return 'clip-' + String(n).padStart(2, '0')
}

/**
 * 这个片段目录**是不是这条事件**剪出来的。
 *
 * 新事件有稳定的 `clip_dir`（检测时按标题/区间分配或复用），直接精确匹配即可；
 * 旧 events.json 没有 `clip_dir` 时保留旧契约：目录名 = `clip-{rank}` 且
 * **标题相同 或 原片区间基本重合（IoU ≥ 0.5）**。
 */
export function clipMatchesEvent(clip, ev) {
  if (!clip || !ev) return false
  if (ev.clip_dir) return String(clip.name || '') === String(ev.clip_dir)
  if (String(clip.name || '') !== clipNameOf(ev.rank)) return false
  const meta = clip.meta || {}
  const oldTitle = String(meta.title || '').trim()
  const sameTitle = !!oldTitle && oldTitle === String(ev.label || '').trim()
  const os = Number(meta.original_start)
  const oe = Number(meta.original_end)
  const s = Number(ev.start_sec)
  const e = Number(ev.end_sec)
  const hasRange = Number.isFinite(os) && Number.isFinite(oe) && Number.isFinite(s) && Number.isFinite(e)
  const overlap = hasRange
    ? Math.max(0, Math.min(oe, e) - Math.max(os, s)) / Math.max(1e-6, Math.max(oe, e) - Math.min(os, s))
    : 1
  if (sameTitle || overlap >= 0.5) return true
  if (!oldTitle && !hasRange) return true // 老 clip.json 什么都没有 → 只能按目录名认
  return false
}

/**
 * 未剪切事件 = 没有任何"属于自己的"片段的事件。
 * 序号被**别的**片段占着（重新检测后的常见情况）也算未剪切 —— 那目录不是它剪出来的。
 */
export function uncutEventsOf(events, clips) {
  const list = (clips || []).filter(c => c && c.video_exists !== false)
  return (events || []).filter(ev => !list.some(c => clipMatchesEvent(c, ev)))
}


export const TYPE_ZH = {
  gameplay: '游戏', interaction: '互动', qna: '问答', reaction: '反应',
  storytelling: '故事', music: '音乐', food: '美食', rant: '吐槽',
  break: '休息', outro: '结尾', chat: '聊天', eating: '吃东西',
  irl: '日常', ranking: '排行', discussion: '讨论', intro: '开场', other: '其他'
}

export const TYPE_COLORS = {
  gameplay: '#2563eb', interaction: '#7c3aed', qna: '#059669',
  reaction: '#db2777', storytelling: '#9333ea', music: '#0891b2',
  food: '#d97706', rant: '#dc2626', break: '#4d7c0f', outro: '#525252',
  chat: '#0d9488', eating: '#ea580c', irl: '#c026d3', ranking: '#ca8a04',
  discussion: '#6d28d9', intro: '#3b82f6', other: '#64748b'
}

// 时间轴泳道/事件块着色（TimelineEditor 专用色板，与 TYPE_COLORS 同一色族）
export const TIMELINE_COLORS = {
  segments: '#525252', camera: '#2563eb', sticker: '#7c3aed', shake: '#dc2626',
  flash: '#d97706', blur: '#0891b2', zoom_bounce: '#059669', vignette: '#4d7c0f',
  hue_shift: '#db2777', stutter: '#9333ea', split: '#ea580c', noise: '#64748b',
  pulse: '#0d9488', color_shift: '#c026d3', overlay: '#ca8a04', sfx: '#6d28d9'
}

export const STICKER_COLORS = ['pink', 'blue', 'mint', 'amber']
export const STICKER_ICONS = ['📺', '🎮', '🍜', '🎤', '🌸', '🎬']

// 任务状态 → chip 颜色 class（全局唯一定义）
export function statusClass(s) {
  return s === 'failed' ? 'r'
    : s === 'running' || s === 'queued' ? 'b'
    : 'g'
}

export function hashIndex(str, mod) {
  let h = 0
  for (const c of String(str || '')) h = (h * 31 + c.charCodeAt(0)) >>> 0
  return h % mod
}

export function stickerColor(name) {
  return STICKER_COLORS[hashIndex(name, STICKER_COLORS.length)]
}

export function stickerIcon(name) {
  return STICKER_ICONS[hashIndex(name, STICKER_ICONS.length)]
}

export function fmtClock(sec) {
  const n = Number(sec)
  if (!Number.isFinite(n)) return String(sec ?? '')
  const h = Math.floor(n / 3600)
  const m = Math.floor((n % 3600) / 60)
  const s = Math.floor(n % 60)
  return (h ? h + ':' + String(m).padStart(2, '0') : m) + ':' + String(s).padStart(2, '0')
}

export function fmtTime(sec) {
  const n = Number(sec)
  if (!Number.isFinite(n)) return '00:00'
  const m = Math.floor(n / 60)
  const s = Math.floor(n % 60)
  return String(m).padStart(2, '0') + ':' + String(s).padStart(2, '0')
}

// 事件检测完成（events.json 已生成）的 stage：>= 此值说明预处理结束，可直接进切片页
export const STAGE_EVENTS_READY = 3
// 切片阶段跑完（output/clips/clips_order.txt 存在）的 stage
export const STAGE_CLIPS_DONE = 4

// 三段流水线（与侧栏/面包屑共用）
export const STEPS = [
  { key: 'prepare', name: '预处理' },
  { key: 'slices', name: '切片' },
  { key: 'polish', name: '精修' }
]

/**
 * 由项目状态推导「进度位」与各节点徽标。
 *
 * `detail`（来自 project store 缓存）缺省时用 `stage` 粗判（侧栏不为徽标打额外请求）；
 * `stage >= 4` 表示切片阶段已跑完（`clips/clips_order.txt` 存在）。
 */
export function stageInfo(project, detail) {
  const stage = Number(project?.stage || 0)
  const hasDetail = !!detail
  const clips = Number(detail?.clips?.length || 0)
  const uncut = Number(detail?.uncutEvents?.length || 0)
  const hasClips = hasDetail ? clips > 0 : !!project?.has_clips
  const polished = Number(detail?.polishedCount || 0)

  const prepareDone = stage >= STAGE_EVENTS_READY
  const slicesDone = hasDetail ? hasClips && uncut === 0 : stage >= STAGE_CLIPS_DONE
  const polishDone = hasDetail ? hasClips && clips > 0 && polished >= clips : false

  let progress = 1
  if (prepareDone) progress = 2
  if (slicesDone) progress = 3

  const badges = {
    prepare: prepareDone ? '已预处理' : (stage > 0 ? `处理中 · ${stage}/3` : '待预处理'),
    slices: hasDetail
      ? (hasClips ? (uncut > 0 ? `已切 ${clips} · 待切 ${uncut}` : `已切 ${clips}`) : (prepareDone ? '待切片' : '待预处理'))
      : (slicesDone ? '已切片' : (prepareDone ? '待切片' : '待预处理')),
    polish: hasDetail
      ? (hasClips ? `精修 ${polished}/${clips}` : '待切片')
      : (hasClips || stage >= STAGE_CLIPS_DONE ? '精修 —' : '待切片')
  }
  const done = { prepare: prepareDone, slices: slicesDone, polish: polishDone }
  return { progress, badges, done, clips, uncut, polished, hasClips, hasDetail }
}

/** 进度位 → 下一步（文案 + 路由 step）。 */
export function nextStep(project, detail) {
  const { progress } = stageInfo(project, detail)
  if (progress <= 1) return { label: '打开预处理', step: 'prepare' }
  if (progress === 2) return { label: '去切片', step: 'slices' }
  return { label: '去精修', step: 'polish' }
}

/** 项目入口路由：预处理完成 → 切片页，否则 → 预处理页。 */
export function projectEntryPath(pid, stage) {
  const view = Number(stage || 0) >= STAGE_EVENTS_READY ? 'slices' : 'prepare'
  return `/p/${encodeURIComponent(pid)}/${view}`
}

export function normalizeSlashes(p) {
  // 去掉 Windows 逐字前缀 `\\?\`（canonicalize 的产物）：带它去做 URL/拼接会变成非法路径
  return String(p || '')
    .replace(/^\\\\\?\\UNC\\/i, '\\\\')
    .replace(/^\\\\\?\\/, '')
    .split(String.fromCharCode(92))
    .join('/')
}

/**
 * 媒体 URL 的**唯一规则**：
 *
 *   `/api/media/<project_id>/<项目内相对路径>`
 *
 * - `pid` 必须是项目 id；
 * - `relPath` 必须是项目根相对路径，统一用 `/` 分隔；
 * - **不接受绝对路径**，也不做旧 `/api/media/<绝对路径>` 的兼容。
 *
 * 例：`mediaUrl(8, 'output/clips/clip-18/clip.mp4')`
 */
export function mediaUrl(pid, relPath = '') {
  const id = String(pid ?? '').trim()
  const rel = normalizeSlashes(relPath).replace(/^\/+/, '')
  if (!id || !rel) return ''
  return `/api/media/${encodeURIComponent(id)}/${rel.split('/').map(encodeURIComponent).join('/')}`
}

/**
 * 片段内文件的媒体 URL：`/api/media/<pid>/output/clips/<clipName>/<file>`。
 *
 * `clipNameOrDir` 可以传片段目录名，也可以传绝对/相对片段目录（只取最后一段目录名）。
 */
export function clipMediaUrl(pid, clipNameOrDir, file = 'clip.mp4') {
  const name = String(clipNameOrDir ?? '')
    .replace(/\\/g, '/')
    .split('/')
    .filter(Boolean)
    .pop() || ''
  if (!pid || !name || !file) return ''
  return mediaUrl(pid, `output/clips/${name}/${file}`)
}
