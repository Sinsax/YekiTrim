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
import { addClient } from '../stores/log'

/**
 * 把 `POST /polish/dhampir-project` 回包里的**转译报告**写进运行日志（总方案 §4「转译报告可见」）。
 *
 * **为什么要有它**：出片只剩底座一条腿（R1），而转译的失败形态是 **"静默少画"** ——
 * `DROP` 表示"参照有、底座表达不了"，那个效果在成片里**就是没有**，但以前报告只活在服务端
 * 一个局部变量里，用户看到的永远是"点了没反应"。
 *
 * ⚠️ **零 DROP 也要记一条**：静默的零与"根本没跑报告"在日志里长得一模一样，
 * 而"本次零 DROP"是**可核对的好消息**。服务端 `dhampir::report_lines` 是同一口径的镜像。
 * ⚠️ 逐条明细走 `debug`（DROP 清单是已知且逐条有理由的），别升级成 warn 淹掉面板。
 * ⚠️ 三处调用点（首启 / 热重载 / 取帧率）共用这一份 —— 别各写一遍。
 */
export function logTranslateReport(res, source = 'dhampir') {
  const rep = res && res.translate_report
  // 老服务端 / 异常回包：没有报告就**明说**，别假装"零 DROP"（那正是本条要消灭的静默）。
  if (!rep || typeof rep !== 'object') {
    addClient('debug', '转译报告：本次回包没带报告（服务端版本较旧？）', source)
    return
  }
  const drops = Number(rep.drop_count) || 0
  const clamps = Number(rep.clamp_count) || 0
  if (drops > 0) {
    addClient('warn', `转译报告：${drops} 条 DROP（参照有、底座表达不了 ⇒ 成片里这些效果是缺的）`, source)
  } else {
    addClient('info', '转译报告：0 条 DROP（本次没有静默丢失的效果）', source)
  }
  if (clamps > 0) {
    addClient('info', `转译报告：${clamps} 条 CLAMP（被截断/收敛到可表达的范围内）`, source)
  }
  for (const d of rep.drops || []) addClient('debug', `DROP ${d}`, source)
  for (const c of rep.clamps || []) addClient('debug', `CLAMP ${c}`, source)
}


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

/** §10 情绪轨道（`data/emotion.json`，`process` 算出来的观众情绪）。
 *
 *  ⚠️ **404 是正常空态，不是错误**：`emotion.json` 只在"有弹幕且判出了窗口"时才写。
 *     调用方必须把它当"这个工程没有情绪轨道"处理（画一条淡提示），
 *     **不许弹错误、也不许静默丢弃**（与 `/polish/subtitles` 的 404 同一条纪律）。
 *
 *  ⚠️ 这个函数就是 `emotion.json` 的**第一个前端读者** —— 它此前全仓零消费者。 */
export const fetchEmotion = (pid, dir, label = '') =>
  api(`/api/polish/emotion?${qs(pid, dir)}`, { label })

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

/** 用**本机**的文件管理器弹出这个目录（精修页「打开输出目录」）。
 *  ⚠️ 只有本地免登录模式（托盘双击起的那份）认得这个端点；服务端模式回 404 ——
 *  那时前端**根本不渲染那颗按钮**（`localNoAuthKnown()` 门控），所以不会有人撞到 404。 */
export const openPath = (path) =>
  api('/api/open-path', { method: 'POST', body: JSON.stringify({ path }), label: '打开输出目录' })

export const selectVersion = (pid, dir, version) =>
  api('/api/polish/select-version', { method: 'POST', body: JSON.stringify({ ...scoped(pid, dir), version }) })

export const fetchStickerSources = (pid, dir, label = '') =>
  api(`/api/polish/sticker-sources?${qs(pid, dir)}`, { label })

/** 贴纸库全量（只读）：`[{name, built, registered, description, tags:[{tag,files}], files}]`。
 *  编辑器贴纸选择器（包 → 分类 → 贴纸三段行递进）的数据源（写操作在素材库那一组接口）。 */
export const fetchStickerLibrary = (pid, dir, label = '') =>
  api(`/api/polish/sticker-library?${qs(pid, dir)}`, { label })

/** 内建 emoji 目录（只读）：`[{tag, codepoints:[hex,…]}]`，**编译期内嵌、不联网**。
 *
 *  ⚠️ 与 `fetchStickerSources` 的分工：那条只回"有哪些**来源**"（其中一项叫 `emoji`），
 *  回不了"emoji 里有哪些**具体表情**"。编辑器「贴纸轨」的 keyword 选中 `emoji` 之后，
 *  表情面板要的就是这一份（见 `StickerPicker.vue`）。
 *  ⚠️ 它**不需要作用域**：目录是编译期内嵌的、不属于任何工程，所以没有 `pid`/`dir` 参数。
 */
export const fetchEmojiCatalog = (label = '') =>
  api('/api/polish/emoji-catalog', { label })

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

/** 视频素材库（只读）：`[{name, file, duration}]`。
 *  `name` = **去扩展名**的素材键，就是 `[intro_outro]` 里存的那一串；
 *  `duration` 可能是 `null`（探不出时长）—— 界面要照列并置灰，别隐藏。 */
export const fetchVideoLibrary = (pid, dir, label = '') =>
  api(`/api/polish/video-library?${qs(pid, dir)}`, { label })

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
/** 渲染导出：`outputFile` 是**项目内相对路径**（默认空串 = 用工程 `[project] output`，缺省 `vtuber-clip.mp4`）；
 *  **同名不覆盖** —— 已存在时自动加 `_1`/`_2`（`render::unique_output_path`，封面同一套规则）。
 *
 *  出片通道**没有可选项**：只有底座 dhampir 一条腿（本仓那条兜底腿已整条删除）。 */
// `renderer` 参数**已删除**：出片只剩底座一条腿（本仓 headless Chrome 那条 2026-10-01 整条删除），
// 传它没有意义 —— 传 `chrome` 服务端会回 **400**（不静默当默认）。
//
// ⚠️ `versions`（§9 多版本成片）**已整条删除**（2026-10-18 用户口径：「为什么要一次渲染
//    多个同样的视频？没有理由吧，版本号 _n 是之前就完成的，这个没有意义」）。
//    注意：`_n` 后缀与它**无关** —— 那是 `unique_output_path`（同名不覆盖），保留。
// ⚠️ 默认参数是**空串**（= 让服务端用工程里的 `[project] output`，缺省 `vtuber-clip.mp4`）。
//    以前写死 `'export.mp4'` —— 那只是文档里的举例，与真实默认名不符。
export const runRender = (pid, dir, outputFile = '') =>
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
/** 转译后的**底座工程 JSON**（`{project, …}`）。页面用它取两个数：
 *  ① `project.timeline.timebase`（工程网格帧率）；② 那条视频素材的 `timebase`（**源素材自己的**帧率）。
 *
 *  ⚠️ 两者**不是一回事**（2026-10-06 起）：电台那种"5fps 静止帧 + 主音频"的素材，工程网格会被
 *  转译器抬到 **30fps 下限**，而素材仍是 5fps。成片轨的 `edlOutStart(clips, fps)` 要的是
 *  **源素材自己那份** —— 拿网格帧率去算就是 6 倍错位（每段的 out_start 全错），而且不报错。
 *  `PolishPreview` 走同一份判据（它那边的 `dhampirSourceFps`），这里是同一个值的第二处消费者。 */
export const fetchDhampirProject = (pid, dir) =>
  api('/api/polish/dhampir-project', {
    method: 'POST',
    body: JSON.stringify({ ...scoped(pid, dir) }),
    label: '底座工程',
  })
export const coverTitle = (pid, dir) => startJob('/api/polish/cover-title', pid, dir)
export const coverFrame = (pid, dir, time) => startJob('/api/polish/cover-frame', pid, dir, { time })
export const coverConfig = (pid, dir, cover) => startJob('/api/polish/cover-config', pid, dir, { cover })
export const coverBackground = (pid, dir, imageData) =>
  startJob('/api/polish/cover-background', pid, dir, { image_data: imageData })

// ⚠️ 这里曾经有 `renderFrame`（`POST /api/polish/render-frame`，wgpu 单帧真渲染预览）——
// 那条 GPU 链路已删除（GPU 那侧交给独立的 Dhampir 底座），预览只走 `polish/engine.js` 自绘。
// 别把它加回来：要真渲染预览就走底座那条链路。

export const fontUrl = (pid, dir, name) =>
  `/api/polish/font?${qs(pid, dir)}${name ? '&name=' + encodeURIComponent(name) : ''}`

/**
 * **素材库统一取文件** URL（素材库界面预览用）：`/api/polish/asset?kind=&file=`。
 *
 * `kind` = `'video'` | `'font'`；根目录由**服务端**按 kind 现取（带存量回落），
 * 前端只给**单个文件名**、绝不拼路径。
 *
 * ⚠️ 给 `<video>` / `<img>` / `@font-face` **直连**，别走 `api()` —— 那条只认 JSON。
 * ⚠️ 贴纸**不走这里**：贴纸是两级（包/包内文件），走 `stickersApi.fileUrl(pack, name)`。
 * 这两条分工别混 —— 硬塞进来就得让 `file` 认 `/`，正好把服务端那道
 * "只允许单个文件名" 的安全性质破掉。
 */
export const assetUrl = (kind, file) =>
  `/api/polish/asset?kind=${encodeURIComponent(kind)}&file=${encodeURIComponent(file)}`

/** 视频素材的**缩略图** URL（服务端 ffmpeg 抽第 1 秒那帧并缓存）。
 *
 * ⚠️ 抽不出来时服务端回 **404**（不是 500）—— `<img>` 会自然触发 `onerror`，
 * 前端退成图标即可。**别在 JS 里先探一次**：那等于每个缩略图多打一个请求。 */
export const videoThumbUrl = (file) => `/api/polish/video-thumb?file=${encodeURIComponent(file)}`

/**
 * 字体库（只读）：`{ root, fallback_stack, fonts: [{id, file, family, display_name, bytes, builtin,
 * is_default, license, scripts, emoji, installed, flat}], builtin: [{id, display_name, family,
 * license, license_url, installed}] }`。
 *
 * ⚠️ `family` 自 v2 起是**字体内部族名**（预览/出片/Takumi 三处唯一认的那个名字），
 * 给人看的标签是 `display_name` —— 别再拿 id 或文件名当族名用。
 * ⚠️ `installed` 只对**自带字体**有意义：`false` = 清单里有它但还没下载（界面给「下载」而非「删除」）；
 * 用户自己导入的永远是 `true`。
 * ⚠️ `fallback_stack` = **动态兜底字体栈**（首项随库里"设为默认"那支走，与出片同一份计算）：
 * 预览的 `.cap` 栈从这取，别在前端写死字体名 —— 写死的话"设为默认"对预览永远无效。
 * ⚠️ `emoji_family`（2026-10-02 字体职责分离）= **emoji 补充槽位**：库里那支负责表情的字体
 * （由服务端 `fonts::emoji_font` 自动选、至多一支），库里没有就是 `null`。
 * 它是**独立字段**、不并进 `fallback_stack` —— 预览把它追加在字体栈**末尾**（正文在前、
 * emoji 兜底），封面那条链则靠**注册顺序**而不是这个栈。
 * ⚠️ `flat` = 老平铺布局（散文件直接在库根目录）；`.ttc` 条目（`file` 以 `.ttc` 结尾）
 * 浏览器 @font-face 认不出，**只展示、不进字体下拉**。
 *
 * 库是**程序级**的（`<数据目录>/resources/fonts`），与项目无关 ——
 * 所以这里不带 `{project, dir}`；读不需要 admin（写才要）。
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
    // ⚠️ body 必须 JSON.stringify：裸对象会被 fetch 转成字面量 "[object Object]"，
    // 服务端 Json 提取器报 400 + text/plain（"接口不存在或返回格式错误"就是这么来的）。
    body: JSON.stringify(dir && fileName ? { dir, name: fileName, display: name } : { path, display: name }),
    label: '导入字体',
  })

/** 字体上传上限（字节）：与 Rust yekitrim_server::api::polish::FONT_UPLOAD_MAX_BYTES
 *  是**同一个数**（改一处要改两处，同贴纸那条规矩）。
 *  ⚠️ 前端必须先挡：axum 超限回的是 **413 + 纯文本**，而 client.js 只认 JSON，
 *  会把「文件太大」报成「接口不存在或返回格式错误」（与贴纸上传同一个坑）。 */
export const FONT_UPLOAD_MAX_BYTES = 64 * 1024 * 1024

/**
 * **上传**一支字体（**仅 admin**）—— 拖拽/选文件那条路。
 *
 * 请求体是字体的**原始字节**（application/octet-stream，不是 multipart）—— 与贴纸上传同一条做法。
 * name = **文件名**（服务端拿它当 id），显示名走 display。
 *
 * ⚠️ 与 importFont 的分工：那条收的是**服务器上已存在的绝对路径**（浏览器里的 File 对象
 * **没有磁盘路径**，所以拖拽走不了它）；这条收的是**浏览器里的字节**。
 */
export const uploadFont = (file, display = '') => {
  if (file && file.size > FONT_UPLOAD_MAX_BYTES) {
    // 前端先挡：否则拿到的是 413 纯文本，会被 client.js 报成「接口不存在或返回格式错误」
    return Promise.reject(
      new Error('文件超过 ' + Math.round(FONT_UPLOAD_MAX_BYTES / 1024 / 1024) + ' MB 上限'),
    )
  }
  const q = new URLSearchParams({ name: file.name })
  if (display) q.set('display', display)
  return api('/api/polish/fonts/upload?' + q.toString(), {
    method: 'POST',
    body: file,
    headers: { 'Content-Type': 'application/octet-stream' },
    label: '上传字体',
    // 上传几十 MB 到（可能很慢的）磁盘：别套用任何比默认更紧的超时
    timeout: 0,
  })
}

/** 设为默认字体（**仅 admin**，唯一默认）。⚠️ 只影响推荐位，**不动任何工程的字体配置**。 */
export const setDefaultFont = (id) =>
  api('/api/polish/fonts/default', { method: 'POST', body: JSON.stringify({ id }), label: '设为默认字体' })

/** 删除一支字体（**仅 admin**）。程序自带的返回 **403「程序自带字体不可删除」**。 */
export const deleteFont = (id) =>
  api('/api/polish/fonts?id=' + encodeURIComponent(id), { method: 'DELETE', label: '删除字体' })

/**
 * 装上**自带字体**（**仅 admin**；幂等，已装好的跳过）。
 *
 * ⚠️ 2026-10-02 起字体是**内嵌在二进制里**的：这条请求**不联网**，只是把内嵌字节写进字体库。
 * 正常路径下**走不到这里**（`ensure_global_config` 在 server 启动时已自动装好），
 * 它是**修复入口** —— 用户手删了库里那支之后补回来。
 * `id` 缺省 = 装全部还缺的。清单与校验和都在服务端（`yekitrim_core::fonts::BUILTIN_FONTS`），
 * 前端**不自己维护这份清单**。
 */
export const downloadBuiltinFonts = (id = '') =>
  api('/api/polish/fonts/builtin', {
    method: 'POST',
    body: JSON.stringify(id ? { id } : {}),
    label: '安装自带字体',
    // 写 ~23 MB 到（可能很慢的）磁盘：别用默认那 10 分钟里的任何更紧的值
    timeout: 0,
  })

export { watchJob } from './jobs'
