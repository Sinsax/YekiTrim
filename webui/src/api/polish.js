// 精修 API —— 唯一的 polish 调用出口。
//
// 多用户契约（P2 起）：写请求统一带 `{ project, dir }`，`dir` 是**项目内相对路径**
// （如 `output/clips/clip-01`）；服务端在 scope 守卫里归一化并校验归属。
import { api } from './client'

// 九宫格格子名 — 必须与 Rust grid_rect() 的命名一致（bot-left / bot-right）
export const GRID_CELLS = [
  'top-left', 'top', 'top-right',
  'mid-left', 'center', 'mid-right',
  'bot-left', 'bottom', 'bot-right'
]

export const REGION_LABELS = {
  liver: '人物',
  danmaku_area: '弹幕区域',
  sticker_area: '贴纸区域',
  reaction_zone: '反应区',
  game_area: '游戏区'
}

/** 相对项目根的片段目录（`dir` 一律用这个形态传给服务端）。 */
export function clipRelDir(clipName) {
  return clipName ? `output/clips/${clipName}` : 'output'
}

function scoped(pid, dir) {
  return { project: pid, dir }
}

export const fetchProject = (pid, dir, withPreview = false) =>
  api(`/api/polish/project?${qs(pid, dir, withPreview)}`)

export const fetchSubtitles = (pid, dir) =>
  api(`/api/polish/subtitles?${qs(pid, dir)}`)

function qs(pid, dir, withPreview = false) {
  const q = new URLSearchParams()
  if (pid != null && pid !== '') q.set('project', String(pid))
  if (dir) q.set('dir', dir)
  if (withPreview) q.set('preview', '1')
  return q.toString()
}

export const saveSubtitles = (pid, dir, segments) =>
  api('/api/polish/subtitles', { method: 'PUT', body: JSON.stringify({ ...scoped(pid, dir), segments }) })

/** patch: { segments?, layout?, mute_ranges?, stickers?, ... } 局部更新 polish.toml */
export const saveProjectPatch = (pid, dir, patch) =>
  api('/api/polish/project', { method: 'PUT', body: JSON.stringify({ ...scoped(pid, dir), ...patch }) })

export const selectVersion = (pid, dir, version) =>
  api('/api/polish/select-version', { method: 'POST', body: JSON.stringify({ ...scoped(pid, dir), version }) })

export const fetchStickerSources = (pid, dir) =>
  api(`/api/polish/sticker-sources?${qs(pid, dir)}`)

async function startJob(url, pid, dir, extra = {}) {
  return api(url, { method: 'POST', body: JSON.stringify({ ...scoped(pid, dir), ...extra }) })
}

/** 通用 polish POST（`{project, dir, ...extra}`），供旧组件迁移用。 */
export const polishPost = startJob

export const initPolish = (pid, dir) => startJob('/api/polish/init', pid, dir)
export const runLlm = (pid, dir) => startJob('/api/polish/llm', pid, dir)
export const runProcess = (pid, dir) => startJob('/api/polish/process', pid, dir)
export const runSilence = (pid, dir) => startJob('/api/polish/silence', pid, dir)
/** 渲染导出：输出固定为项目内相对路径（默认 export.mp4） */
export const runRender = (pid, dir, outputFile = 'export.mp4') =>
  startJob('/api/polish/render', pid, dir, { output_file: outputFile })
export const runCover = (pid, dir) => startJob('/api/polish/cover', pid, dir)
export const coverTitle = (pid, dir) => startJob('/api/polish/cover-title', pid, dir)
export const coverFrame = (pid, dir, time) => startJob('/api/polish/cover-frame', pid, dir, { time })
export const coverConfig = (pid, dir, cover) => startJob('/api/polish/cover-config', pid, dir, { cover })
export const coverBackground = (pid, dir, imageData) =>
  startJob('/api/polish/cover-background', pid, dir, { image_data: imageData })
export const coverPreview = (pid, dir) => startJob('/api/polish/cover-preview', pid, dir)

/** 渲染单帧预览（wgpu feature 关闭时后端回 404/400，前端不显示入口） */
export const renderFrame = (pid, dir, t) =>
  startJob('/api/polish/render-frame', pid, dir, { t })

export const fontUrl = (pid, dir, name) =>
  `/api/polish/font?${qs(pid, dir)}${name ? '&name=' + encodeURIComponent(name) : ''}`

export { watchJob } from './jobs'
