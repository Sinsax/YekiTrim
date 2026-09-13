// 通用前端工具：媒体路径 / 哈希贴纸 / 事件类型 / 时间格式化

export const EVENT_TYPES = [
  'gameplay', 'interaction', 'qna', 'reaction', 'storytelling',
  'music', 'food', 'rant', 'break', 'outro', 'chat', 'eating',
  'irl', 'ranking', 'discussion', 'intro', 'other'
]

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

// 项目入口路由：预处理（事件检测）完成后直接进切片页，否则进预览页。
// M2：作用域是项目 id（不再是工程路径）。
export function projectEntryPath(pid, stage) {
  const view = Number(stage || 0) >= STAGE_EVENTS_READY ? 'slices' : 'preview'
  return `/project/${encodeURIComponent(pid)}/${view}`
}

export function normalizeSlashes(p) {
  return String(p || '').split(String.fromCharCode(92)).join('/')
}

/**
 * 媒体 URL。
 *
 * - `mediaUrl(projectId, 'output/clips/clip-01/clip.mp4')` → 新契约（服务端按项目 id + 项目内相对路径放行）
 * - `mediaUrl('<绝对目录>', 'clip.mp4')` → 旧写法；服务端仍接受，但只在"该路径位于本人项目内"时放行
 */
export function mediaUrl(dirOrPid, file = 'clip.mp4') {
  const s = String(dirOrPid ?? '')
  if (/^\d+$/.test(s)) {
    return '/api/media/' + s + '/' + normalizeSlashes(file).replace(/^\/+/, '')
  }
  const norm = normalizeSlashes(s)
  return '/api/media/' + encodeURIComponent(norm + '/' + file).replace(/%2F/g, '/')
}

export function inputMediaUrl(absPath) {
  if (!absPath) return ''
  return '/api/media/' + encodeURIComponent(normalizeSlashes(absPath)).replace(/%2F/g, '/')
}
