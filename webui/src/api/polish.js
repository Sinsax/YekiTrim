// 精修 API —— 唯一的 polish 调用出口。
//
// 多用户契约（P2 起）：写请求统一带 `{ project, dir }`。
//
// ⚠️ **`dir` 是片段目录的绝对路径，不是项目内相对路径。**
// 服务端那边是 `validated_project_dir(dir)`：非空 + `Path::is_dir()`，
// 然后按它反查归属（`PolishPreview.vue` 的注释也写着"`dir` 在精修页是绝对路径"）。
// 这里以前写的是「项目内相对路径（如 `output/clips/clip-01`）」—— **那句是错的**，
// 而它错得有代价：照它拼出来的路径一定 404，且「查不到目录」与「路径写错」长得一样。
import { api } from './client'

// 区域名（与 polish.toml 的 [layout.*] 一一对应）。
// 九宫格 → 自由选框的换算（GRID_RECTS / regionRect）在 polish/engine.js。
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

export const fetchProject = (pid, dir, withPreview = false, label = '') =>
  api(`/api/polish/project?${qs(pid, dir, withPreview)}`, { label })

export const fetchSubtitles = (pid, dir, label = '') =>
  api(`/api/polish/subtitles?${qs(pid, dir)}`, { label })

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

export const fetchStickerSources = (pid, dir, label = '') =>
  api(`/api/polish/sticker-sources?${qs(pid, dir)}`, { label })

/** 贴纸库全量（只读）：`[{name, built, registered, description, tags:[{tag,files}], files}]`。
 *  编辑器里「keyword 一级 → 贴纸二级」选择的数据源（写操作仍只在 admin 侧）。 */
export const fetchStickerLibrary = (pid, dir, label = '') =>
  api(`/api/polish/sticker-library?${qs(pid, dir)}`, { label })

/** 贴纸缩略图 URL（`<img>` 直连；服务端只读 + 路径安全校验，与 admin 预览同一个实现）。 */
export const stickerFileUrl = (pid, dir, pack, name) => {
  const q = new URLSearchParams()
  if (pid != null && pid !== '') q.set('project', String(pid))
  if (dir) q.set('dir', dir)
  q.set('pack', pack)
  q.set('name', name)
  return '/api/polish/sticker-file?' + q.toString()
}

/** 音效目录（只读）：`[{id, label, cat, tags}]`，已按全局 `[sfx] blacklist` 过滤。 */
export const fetchSfxCatalog = (pid, dir, label = '') =>
  api(`/api/polish/sfx-catalog?${qs(pid, dir)}`, { label })

async function startJob(url, pid, dir, extra = {}) {
  return api(url, { method: 'POST', body: JSON.stringify({ ...scoped(pid, dir), ...extra }) })
}

/** 通用 polish POST（`{project, dir, ...extra}`），供旧组件迁移用。 */
export const polishPost = startJob

export const initPolish = (pid, dir) => startJob('/api/polish/init', pid, dir)
/** LLM 剪辑（整片）。`full` 默认 **true** = 全部重做：清掉断点续传缓存、重新分段 + 重生成全部事件。
 *  想省调用就用 `runLlmSegment`（只重做某一段）。 */
export const runLlm = (pid, dir, full = true) => startJob('/api/polish/llm', pid, dir, { full })
/** 只重做某一段的事件（`index` = polish.toml 里 `[[segments]]` 的下标）。
 *  别的段、筛选词、高亮词一律不动；失败时保留该段原有事件。 */
export const runLlmSegment = (pid, dir, index) =>
  startJob('/api/polish/llm-segment', pid, dir, { index })
export const runProcess = (pid, dir) => startJob('/api/polish/process', pid, dir)
export const runSilence = (pid, dir, extra = {}) => startJob('/api/polish/silence', pid, dir, extra)
/** 渲染导出：输出固定为项目内相对路径（默认 export.mp4）；
 *  **同名不覆盖** —— 已存在时写成 `export_1.mp4`（`render::unique_output_path`，封面同一套规则）。
 *
 *  出片通道**没有可选项**：只有底座 dhampir 一条腿（本仓那条兜底腿已整条删除）。 */
// `renderer` 参数**已删除**：出片只剩底座一条腿（本仓 headless Chrome 那条 2026-10-01 整条删除），
// 传它没有意义 —— 传 `chrome` 服务端会回 **400**（不静默当默认）。
export const runRender = (pid, dir, outputFile = 'export.mp4') =>
  startJob('/api/polish/render', pid, dir, { output_file: outputFile })
/** 生成封面。`renderer`：只有 `takumi`（静态渲染，不出浏览器、内存小）；不传 = 服务端默认。
 *  ⚠️ 老脚本里的 `chrome` 已整条删除，传了会由 `cover.rs` **明确报错**（不是"认不出"）。 */
export const runCover = (pid, dir, renderer) =>
  startJob('/api/polish/cover', pid, dir, renderer ? { renderer } : {})
/** 底座**出片**通道的就绪位（M3）：`{ready, binary, preview, translator, font, contract_version, problems[]}`。
 *  渲染弹窗打开时调一次 —— "底座不可用"要在**点渲染之前**可见，而不是任务跑到一半才失败。 */
export const fetchDhampirStatus = (pid, dir) =>
  api('/api/polish/dhampir-status', {
    method: 'POST',
    body: JSON.stringify({ ...scoped(pid, dir) }),
    label: '底座出片通道就绪位',
  })
export const coverTitle = (pid, dir) => startJob('/api/polish/cover-title', pid, dir)
export const coverFrame = (pid, dir, time) => startJob('/api/polish/cover-frame', pid, dir, { time })
export const coverConfig = (pid, dir, cover) => startJob('/api/polish/cover-config', pid, dir, { cover })
export const coverBackground = (pid, dir, imageData) =>
  startJob('/api/polish/cover-background', pid, dir, { image_data: imageData })
export const coverPreview = (pid, dir) => startJob('/api/polish/cover-preview', pid, dir)

// ⚠️ 这里曾经有 `renderFrame`（`POST /api/polish/render-frame`，wgpu 单帧真渲染预览）——
// 那条 GPU 链路已删除（GPU 那侧交给独立的 Dhampir 底座），预览只走 `polish/engine.js` 自绘。
// 别把它加回来：要真渲染预览就走底座那条链路。

export const fontUrl = (pid, dir, name) =>
  `/api/polish/font?${qs(pid, dir)}${name ? '&name=' + encodeURIComponent(name) : ''}`

/**
 * 字体库（只读）：`{ root, fonts: [{id, file, family, display_name, bytes, builtin, is_default,
 * license, scripts, emoji, installed}], builtin: [{id, display_name, family, license, license_url, installed}] }`。
 *
 * ⚠️ `family` 自 v2 起是**字体内部族名**（预览/出片/Takumi 三处唯一认的那个名字），
 * 给人看的标签是 `display_name` —— 别再拿 id 或文件名当族名用。
 * ⚠️ `installed` 只对**自带字体**有意义：`false` = 清单里有它但还没下载（界面给「下载」而非「删除」）；
 * 用户自己导入的永远是 `true`。
 *
 * 库是**程序级**的（`<程序目录>/resources/fonts/`），与项目无关 —— 所以这里不带 `{project, dir}`；
 * 读不需要 admin（写才要）。
 */
export const fetchFonts = (label = '') => api('/api/polish/fonts', { label })

/**
 * 导入一支字体（**仅 admin**）：`{path}` = **服务器上**字体文件的绝对路径。
 *
 * ⚠️ 不是上传：字体几十 MB、axum 默认 body limit 2 MB，走"服务器上已有文件"这条
 * （与 `/api/fs/list` 同一条来源）。服务端只收 `.ttf`/`.otf`，
 * `.ttc`/`.otc`（字体集合）会被**明确拒绝**并把原因回在 error 里 —— 别在前端自己判。
 *
 * ⚠️ 字段名别混：`name` 是**文件名**（服务端用它当 id），显示名走 `display`。
 */
export const importFont = ({ path = '', dir = '', fileName = '', name = '' }) =>
  api('/api/polish/fonts/import', {
    method: 'POST',
    // ⚠️ 优先 `{dir, name}`：路径**由服务端拼**（前端拼会把 `\\?\` verbatim 前缀拼坏，
    // 与素材清单同一条规矩）。`path` 只留给"我就是要给绝对路径"的调用方。
    body: dir && fileName ? { dir, name: fileName, display: name } : { path, display: name },
    label: '导入字体',
  })

/** 设为默认字体（**仅 admin**，唯一默认）。⚠️ 只影响推荐位，**不动任何工程的字体配置**。 */
export const setDefaultFont = (id) =>
  api('/api/polish/fonts/default', { method: 'POST', body: { id }, label: '设为默认字体' })

/** 删除一支字体（**仅 admin**）。程序自带的返回 **403「程序自带字体不可删除」**。 */
export const deleteFont = (id) =>
  api('/api/polish/fonts?id=' + encodeURIComponent(id), { method: 'DELETE', label: '删除字体' })

/**
 * 下载**自带字体**（**仅 admin**；幂等，已装好的跳过）。
 *
 * ⚠️ 会联网、约 38 MB（霞鹜文楷 Lite + Noto Color Emoji）⇒ 给足超时；
 * `id` 缺省 = 下全部还缺的。清单与校验和都在服务端（`vtrim_core::fonts::BUILTIN_FONTS`），
 * 前端**不自己维护这份清单**。
 */
export const downloadBuiltinFonts = (id = '') =>
  api('/api/polish/fonts/builtin', {
    method: 'POST',
    body: id ? { id } : {},
    label: '下载自带字体',
    // 几十 MB 的外网下载：别用默认那 10 分钟里的任何更紧的值
    timeout: 0,
  })

export { watchJob } from './jobs'
