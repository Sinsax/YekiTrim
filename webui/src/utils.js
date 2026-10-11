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
/**
 * **多段切片**的段色（`PrepareView` 的段号 + `SubtitleTrack` 的色带共用同一份）。
 *
 * 用户口径 2026-10-06：「多段时需要将每一段都颜色标注，而不是只有最新的才标注」——
 * 所以列表里每个段号、轨道上每条色带都按**同一套顺序**取色，一眼能对上"第 3 段在哪儿"。
 * 超过 `SEG_COLORS.length` 段就循环（6 色足够分辨；真到十几段时看编号也不看颜色了）。
 */
export const SEG_COLORS = [
  { line: 'rgba(232,104,56,.92)', fill: 'rgba(232,104,56,.28)' },
  { line: 'rgba(59,130,246,.92)', fill: 'rgba(59,130,246,.26)' },
  { line: 'rgba(16,185,129,.92)', fill: 'rgba(16,185,129,.26)' },
  { line: 'rgba(168,85,247,.92)', fill: 'rgba(168,85,247,.26)' },
  { line: 'rgba(234,179,8,.92)', fill: 'rgba(234,179,8,.28)' },
  { line: 'rgba(236,72,153,.92)', fill: 'rgba(236,72,153,.26)' },
]

/** 第 i 段（0 基）的配色；`active` 时线更粗更亮。 */
export function segColor(i) {
  const c = SEG_COLORS[Math.abs(Number(i) || 0) % SEG_COLORS.length]
  return c
}

// ── 复合片段（多段切片）的「各段源区间」：**全站唯一**的格式化出口 ──────────────
//
// 真源是 `clip.json` / `events.json` 里的 `ranges`（秒）。四处卡片墙（切片页两种卡 /
// 精修片段条 / 首页总览）都调下面这几个函数 —— 别再在各自模板里拼区间文案：
// 「N 段」是谁都会写，**哪几段**只有一处能改，否则改一处漂三处，静态判据也钉不住。
//
// ⚠️ 判据只认 **> 1 段**（与写侧 `ranges.len() > 1` 同口径）：单段写不写 `ranges` 都算普通片段，
// 显示上必须和没有 `ranges` 的老片段**一个像素都不差**。
// ⚠️ 输出**按时间排序**（写侧 `create-event` 与 `Event::normalize_ranges` 也是这个序）：
// 手工标段时列表的行序是"添加顺序"，落盘后会变成时间序 —— 卡片上写的编号一律按时间，
// 所以标题里明写"按时间"。

/** 段号（0 基）：①②③… 超过 10 段退回 `11.`（圈码只到 ⑩）。 */
export function segMark(i) {
  const marks = '①②③④⑤⑥⑦⑧⑨⑩'
  const n = Math.abs(Number(i) || 0)
  return n < marks.length ? marks[n] : n + 1 + '.'
}

/** 秒 → `07:21.4` / `1:02:03.5`（**带一位小数**：段边界要精确到 0.1s 才核得动）。 */
export function fmtSpan(sec) {
  const n = Math.max(0, Number(sec) || 0)
  const h = Math.floor(n / 3600)
  const m = Math.floor((n % 3600) / 60)
  const s = n - h * 3600 - m * 60
  const mm = h ? String(m).padStart(2, '0') : String(m)
  return (h ? h + ':' : '') + mm + ':' + s.toFixed(1).padStart(4, '0')
}

/** 规范化：接受 `[{start,end}]` 或 `{ranges:[…]}` / 卡片条目（`meta` / `ev` 都行）。
 *  返回**按时间排序**、丢掉非法段之后的数组；不足 2 段返回 `[]`（调用方据此判"是不是复合片段"）。 */
export function compoundRanges(src) {
  const raw = Array.isArray(src) ? src : src?.ranges
  if (!Array.isArray(raw)) return []
  const out = raw
    .map((r) => ({ start: Number(r?.start) || 0, end: Number(r?.end) || 0 }))
    .filter((r) => r.end > r.start)
    .sort((a, b) => a.start - b.start)
  return out.length > 1 ? out : []
}

/** 段间**被丢掉**的时长（秒）= 外沿跨度 − 各段内容之和。单段/空 = 0。
 *  ⚠️ 外沿用 `max(end) - min(start)`，**不是**"排序后最后一段的 end"：段是可能**互相重叠**的
 *  （手工标段时两段可以压在一起 —— 项目 12 那条就有一条 1625.7–1653.8 包着 1632–1643），
 *  那时"最后一个起点"的那个段未必是终点最大的那个，少算 11s 且**不报错**。 */
export function rangesGapSecs(src) {
  const rs = compoundRanges(src)
  if (!rs.length) return 0
  const span = Math.max(...rs.map((r) => r.end)) - Math.min(...rs.map((r) => r.start))
  const content = rs.reduce((s, r) => s + (r.end - r.start), 0)
  return Math.max(0, span - content)
}

/** 行内串：`① 07:21.4–08:06.5 · ② 15:29.0–16:43.0`（卡片那一行与悬停 title 共用）。 */
export function rangesText(src) {
  return compoundRanges(src)
    .map((r, i) => `${segMark(i)} ${fmtSpan(r.start)}–${fmtSpan(r.end)}`)
    .join(' · ')
}

/** 悬停 title：段数 + **全部区间** + 丢掉的空隙（>1s 才提）。
 *  ⚠️ 用户可见文案里**不许**用 `**` 当强调（会原样显示成星号），这里只用中文标点。 */
export function rangesHint(src) {
  const rs = compoundRanges(src)
  if (!rs.length) return ''
  const gap = rangesGapSecs(rs)
  const parts = [
    `复合片段：按时间取 ${rs.length} 段源区间分段剪、合并输出（段间的空隙已丢掉）`,
    rangesText(rs)
  ]
  if (gap >= 1) parts.push(`丢掉的有内容空隙合计 ${Math.round(gap)}s`)
  return parts.join('\n')
}

/**
 * 弹幕正文的**可读化**：把 ASS 覆写标签与换行转义摘掉，只留人看的字。
 *
 * 为什么需要它：弹幕文件（尤其是 `ass` 那份，是从录播姬 XML 转出来的）里每一条都带
 * `{\move(1970,0,-50,0)}` / `{\pos(960,1040)}{\c&HFF3F...}` 这类**定位/上色标签** ——
 * 解析层（`danmaku::parse_ass`）**有意**原样保留它们（渲染要按标签摆位），但拿去做**关键词搜索**
 * 会两头出问题：① 卡片上显示出来是乱码一样的标签串；② 搜 `move`/`pos` 会命中**每一条**弹幕
 * （那是标签不是内容，纯属假命中）。
 *
 * ⚠️ 只摘 `{...}` 覆写块与 `\N`/`\n`/`\h` 转义；**不动**正文本身（含表情与标点）。
 * 结果为空就说明这条弹幕本来只有标签（纯摆位行），调用方负责丢掉。
 */
export function dmPlain(s) {
  return String(s ?? '')
    .replace(/\{[^}]*\}/g, '')
    .replace(/\\[Nnh]/g, ' ')
    .replace(/\s+/g, ' ')
    .trim()
}

export const TIMELINE_COLORS = {
  segments: '#525252', camera: '#2563eb', sticker: '#7c3aed', shake: '#dc2626',
  flash: '#d97706', blur: '#0891b2', vignette: '#4d7c0f',
  hue_shift: '#db2777', noise: '#64748b',
  pulse: '#0d9488', overlay: '#ca8a04', sfx: '#6d28d9'
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

/**
 * 大数 → 中文口径（B站的播放/弹幕数就是按这个展示的）。
 *
 * 只做"万/亿"两级：B站接口给的是原始整数（如 149494），界面上写全数字读起来很累。
 * `undefined`/`null` 返回空串（调用方常直接拼进文案，别渲染出 "NaN万"）。
 */
export function fmtCount(n) {
  const v = Number(n)
  if (!Number.isFinite(v) || v <= 0) return ''
  if (v >= 100000000) return (v / 100000000).toFixed(1).replace(/\.0$/, '') + '亿'
  if (v >= 10000) return (v / 10000).toFixed(1).replace(/\.0$/, '') + '万'
  return String(Math.round(v))
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

// 设置页的子页（与路由 / 页内标签条 / 面包屑共用）：key 既是路径段也是路由名后缀（settings-<key>）。
// 顺序 = 标签条顺序；`/settings` 落到第一项。
export const SETTINGS_SECTIONS = [
  { key: 'config', label: '程序配置', sub: 'ASR / 检测 / 剪切，写入 .yekitrim/config.toml' },
  // ⚠️ 副标题要带上"提示词"：那一组写的是 **config.toml 的 [detection]**（不是 llm.toml），
  //    只是摆在这一页。不写出来，用户会以为这页所有东西都落在 llm.toml 里。
  { key: 'llm', label: 'LLM 配置', sub: '接口 / 密钥 / 模型 / 并发（llm.toml）· 提示词与参考标题（config.toml）' },
  // 本地免登录模式下整页不显示（没有注册/邀请这回事）—— `localHidden` 由
  // SettingsView 的标签过滤与 router 的守卫共同执行，两处都读 `stores/health.js`。
  // ⚠️ ASR 模型管理**不在这里**：它搬去首页当「运行状况」面板了（那是全机共享的资源，
  //    跟"谁在登录"无关，摆在账号页只会让人以为它是按用户分的）。
  { key: 'account', label: '账号', sub: '注册与邀请码', localHidden: true },
  { key: 'integrations', label: '系统集成', sub: '录播姬采集 与 B 站链接下载' },
  { key: 'monitor', label: '用量与日志', sub: 'LLM 用量账本 与 运行日志' }
]

/** 路由名（settings-llm）或子页 key（llm）→ 子页定义；认不出来时回第一项（程序配置）。 */
export function settingsSection(nameOrKey) {
  const key = String(nameOrKey || '').replace(/^settings-/, '')
  return SETTINGS_SECTIONS.find((s) => s.key === key) || SETTINGS_SECTIONS[0]
}

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

/**
 * 这个键盘事件是不是**该归可编辑控件**（输入框 / 文本域 / 下拉 / 富文本 / 输入法合成中）？
 *
 * ⚠️ **只留这一份判据**：面板类快捷键（时间轴的 Space、封面设计器的 Delete/上下移、切片页的
 * 方向键、预处理页的 Space）在接管按键前都要先过它。少一处就是一类**静默事故** ——
 * 实测：封面设计器里在标题文本框按 Delete，一个键把**整个文本图层**删了
 * （`CoverDesigner::onPanelKey` 当时没有这道闸，而它绑在整块 `.controls` 上）。
 * ⚠️ `isComposing` 同样算"归控件"：中文输入法拼音候选里按 Delete/空格，语义在输入法手上。
 */
export function isEditableKeyEvent(e) {
  if (!e) return false
  if (e.isComposing === true) return true
  const t = e.target
  if (!t) return false
  if (t.isContentEditable === true) return true
  const tag = String(t.tagName || '').toUpperCase()
  return tag === 'INPUT' || tag === 'TEXTAREA' || tag === 'SELECT'
}
