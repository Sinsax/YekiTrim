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
    : s === 'waiting_resume' ? 'a'
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

export function normalizeSlashes(p) {
  return String(p || '').split(String.fromCharCode(92)).join('/')
}

export function mediaUrl(dir, file = 'clip.mp4') {
  const norm = normalizeSlashes(dir)
  return '/api/media/' + encodeURIComponent(norm + '/' + file).replace(/%2F/g, '/')
}

export function inputMediaUrl(absPath) {
  if (!absPath) return ''
  return '/api/media/' + encodeURIComponent(normalizeSlashes(absPath)).replace(/%2F/g, '/')
}

export function thumbUrl(dir) {
  const parts = String(dir || '').split(/[\\/]/)
  const name = (parts.pop() || '').replace('clip-', '')
  if (name) return '/api/data/clips/' + encodeURIComponent(name) + '/thumb'
  const m2 = String(dir).match(/clip-(\d+)/)
  return '/api/data/clips/' + (m2 ? m2[1] : '0') + '/thumb'
}
