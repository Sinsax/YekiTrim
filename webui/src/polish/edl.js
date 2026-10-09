// edl.js — 「成片时间线（EDL）」的**共享只读派生** + 编辑动作。
//
// 规格：`docs/plan/edl-retiming-spec.md` §2 / §6。三条不变量：
//
//  1. **一个数组，三处消费**（§3）：Rust `Edl::resolve_clips` / 转译器 `buildKeepRanges` /
//     前端成片轨，三处必须**逐值**一致。前端那处的 out_start 推导**只有一份** ——
//     `engine.js::edlOutStart`（本文件只是它的薄封装，**不许**在这里再写一遍算式）。
//     判据 = `node scripts/check-edl-invariants.mjs`（`✓ 全部通过（17 项对拍）`）。
//
//  2. **源时间的 x 坐标永不被 EDL 影响**（§6.1）：上面那 7 条泳道是源时间，插一段重放
//     "把后面推右"就等于改写每个块的源时间语义（`types.rs:38`「事件的时间一律是绝对秒」），
//     删掉那段之后后面的块**回不到原位**（信息已丢）。EDL 是"源时间 → 成片时间"的**映射**。
//     所以成片轨是**独立的第二条时间轴**，与源轴并列显示（§6.2 双轨视图）。
//
//  3. **`clips` 为空 = 老工程 = 今天的行为**（§2.2 规则 3）：后端由 `mute_ranges` 的补集派生，
//     前端**不重算**那件事（补集是业务分流，住 `ProjectFile::resolve_edl` 一处）。
//     ⚠️ 因此前端**不能**把"空 clips"读成"什么都不播"——它读成"这条路没开"（不画成片轨，
//     老工程界面零变化）。
import { edlOutStart, edlTotalFrames } from './engine.js'

/**
 * 变速档位（§6.3 第 ⑤ 条给的档）。
 *
 * ⚠️ **`enabled` 是要如实说明的后端状态**，不是前端保守。第 3 批时后端**还没有** `speed`
 * 的实现（音轨装配遇到 `rate != 1.0` 直接 bail、转译器发 CLAMP），所以那几档是 `false`
 * —— **不许**给一个"看着能点、出片没效果"的控件。
 *
 * **第 4 批（task-7）起后端接通，全部档位 `true`**：
 * - 画面：转译器按 `speed` 缩段长（`round((ef-sf)/speed)`，与 Rust `Edl::resolve_clips` 逐值一致）；
 * - 音轨：`Ffmpeg::assemble_audio_from_edl` 逐段挂 `atempo`（**时域伸缩，基频不变**）。
 *   ⚠️ 这里**只能**用 `atempo`，用 `asetpts`/`asetrate` 缩放会变调（花栗鼠音，规格 §9 R6）。
 *
 * 区间与后端**同一个来源**：[`EDL_SPEED_MIN`] / [`EDL_SPEED_MAX`] 必须等于 Rust
 * `project::validate::SPEED_MIN/MAX`，而那一对又必须等于 ffmpeg `atempo` 的**单级**范围。
 * 超界的速率后端会串联多级 `atempo`（`0.25 ⇒ atempo=0.5,atempo=0.5`），但**UI 不给**那些档。
 */
export const EDL_SPEEDS = [
  { value: 0.5, label: '×0.5', title: '慢放 ×0.5（成片时长翻倍；音调不变）', enabled: true },
  { value: 0.75, label: '×0.75', title: '慢放 ×0.75（成片时长 ×1.33；音调不变）', enabled: true },
  { value: 1, label: '×1.0', title: '原速', enabled: true },
  { value: 1.5, label: '×1.5', title: '快放 ×1.5（成片时长 ×0.67；音调不变）', enabled: true },
  { value: 2, label: '×2.0', title: '快放 ×2.0（成片时长减半；音调不变）', enabled: true },
]

/** 段速率的合法区间（与 Rust `validate::SPEED_MIN/MAX` = ffmpeg `atempo` 单级范围一致）。 */
export const EDL_SPEED_MIN = 0.5
export const EDL_SPEED_MAX = 2.0

/** 段的语义标签 → 中文名（`kind` 是可选字段，**不参与任何推导**，只给 UI/统计）。 */
export const EDL_KINDS = {
  main: '正片',
  replay: '重放',
  recall: '回忆',
  // ⚠️ **第 5 批（task-9）新增**：冷开场由 `process` 落实（LLM 在运镜事件上写
  // `anchor = "cold_open"`），落盘时 `kind` 就是这个词。
  // 漏了这一行的症状是成片轨上那段标签**退化成原值**（显示的英文 `cold_open`），
  // 而且**不报错** —— 正是"表格少一档"这一类静默漂移。
  cold_open: '冷开场',
  intro: '片头',
  outro: '片尾',
}

/**
 * 成片轨上的一段从**哪里来**（§6.5：两种来源必须视觉可分）。
 *
 * **第 6 批（task-8）起 `EdlClip` 真的有 `source_kind` / `asset_id` 了**（规格 §11.4），
 * 所以这里从"只可能是 clip"升级成**真判据**。三处消费点必须同口径：
 *  · Rust `EdlClip::is_asset` / `asset_id_ref`（`yekitrim-render-core/src/types.rs`，出片腿 + 音轨）；
 *  · 转译器 `polish-to-dhampir.mjs::edlClipAssetId`（画面腿）；
 *  · 这里（预览腿：成片轨上色 + 角标）。
 * 分歧的症状是"**预览里那段是素材、成片里是本条**"（或反过来），而**不报错**。
 */
export const EDL_SOURCE_CLIP = 'clip'
export const EDL_SOURCE_ASSET = 'asset'

/**
 * 素材段指向的素材 id（`null` = 本条片段）。
 *
 * ⚠️ 与 Rust `EdlClip::asset_id_ref` **逐字同口径**：`source_kind` 必须是小写 `'asset'`，
 * `asset_id` 必须非空、非全空白。空串必须当成"没填" —— 它会静默退化成"这一段用回本条片段"，
 * 而用户看不出区别（Rust 侧那条注释写着同一件事）。
 */
export function edlAssetId(clip) {
  if (!clip || typeof clip !== 'object') return null
  if (clip.source_kind !== EDL_SOURCE_ASSET) return null
  const id = typeof clip.asset_id === 'string' ? clip.asset_id.trim() : ''
  return id === '' ? null : id
}

/** 一段的来源类型。字段缺席 / 显式 `clip` / 认不出的档位 = 本条片段（认不出不猜素材）。 */
export function edlSourceKind(clip) {
  return edlAssetId(clip) ? EDL_SOURCE_ASSET : EDL_SOURCE_CLIP
}

/** 一段的来源显示名（角标文案）。素材库来源带上 `asset_id`，否则界面上看不出播的是哪个素材。 */
export function edlSourceLabel(clip) {
  const id = edlAssetId(clip)
  if (!id) return '本条'
  return `素材 ${id}`
}

/**
 * 转场的两个档位（第 7 批 / task-10）。
 *
 * ⚠️ **只有这两档进 UI**：`flash` 仍然**没有消费方**（Rust `transition_kind` 认不出就返回
 * `None` = 硬切），所以界面上**不给它选项** —— 用户口径是"宁可没有，不要一个看着能点、
 * 出片没效果的选项"（规格 §11.6）。等它的消费方落地再往这里加。
 */
export const EDL_TRANSITION_CUT = 'cut'
export const EDL_TRANSITION_DISSOLVE = 'dissolve'

/**
 * 这一段的**入场转场**（`null` = 硬切）。
 *
 * ⚠️ 与 Rust `EdlClip::transition_kind` **逐字同口径**：`transition_in` 必须是**小写**
 * `'dissolve'`；`'Dissolve'` / `'DISSOLVE'` 不认（工程文件是机器写的，宽容只会让"拼错"
 * 从"看得出来"变成"看不出来"）；其余一切（字段缺席 / `'cut'` / 认不出的档位）⇒ `null`。
 *
 * 三处消费点必须同口径：Rust（出片腿 + 音轨）、转译器 `edlClipTransition`（画面腿）、
 * 这里（预览腿：面板选中态 + 成片轨角标）。分歧的症状是"**面板里选了叠化、成片里还是硬切**"
 * （或反过来），而**不报错**。
 */
export function edlTransition(clip) {
  if (!clip || typeof clip !== 'object') return null
  return clip.transition_in === EDL_TRANSITION_DISSOLVE ? EDL_TRANSITION_DISSOLVE : null
}

/** 一段的转场显示名（面板按钮 / 成片轨角标）。 */
export function edlTransitionLabel(clip) {
  return edlTransition(clip) === EDL_TRANSITION_DISSOLVE ? '叠化' : '硬切'
}

/** `edl.clips` 是否非空 —— **"这条路开没开"的唯一判据**（空 = 老工程 = 不画成片轨）。 */
export function edlVisible(config) {
  const clips = config && config.edl && config.edl.clips
  return Array.isArray(clips) && clips.length > 0
}

/**
 * 成片轨的**几何**：每段的源区间、宽、以及成片轴上的起点。
 *
 * 返回的是**纯派生值**，调用方（`TimelineEditor`）只拿它画；这里不碰 DOM、不读组件状态。
 *
 * 两个坐标系（§6.2）：
 *  · `left` / `width` —— 画在**成片轴**上：宽 = `(source_end - source_start) / speed`（秒），
 *    所以**变速看得见**（`×0.5` 的段在成片里占两倍长）。
 *  · `srcLeft` / `srcWidth` —— 同一段在**源轴**上的位置（仅用于把块的标题/提示写准，
 *    以及将来画"源 → 成片"的连线；**源轴的块一个像素都不动**）。
 *
 * ⚠️ **宽度的算式与 `edlOutStart` 的 `durFrames` 是同一口径的秒域投影**（`durFrames / fps`），
 * 不是另算一遍 `(end-start)/speed`：那样在非帧对齐的秒值上会与帧号域差 1 帧，
 * 而"块宽对不上后端算出来的时长"正是 R7（预览 ≠ 成片）的入门症状。
 * 这里故意走 `outFrames` 派生 —— **只有一处算式**。
 *
 * @param {object} config 精修工程配置（`polish.toml` 的 JSON 镜像）
 * @param {number} fps 源素材帧率（帧号域的前提）
 * @param {number} scale 每源秒多少像素（与源轴同一个 `scale`，两轴共用横向尺度）
 */
export function edlBlocks(config, fps, scale) {
  const clips = (config && config.edl && config.edl.clips) || []
  if (!Array.isArray(clips) || !clips.length) return []
  const fpsSafe = Number.isFinite(fps) && fps > 0 ? fps : 30
  const s = Number.isFinite(scale) && scale > 0 ? scale : 1
  // 逐值推导走**唯一那一份**（engine.js::edlOutStart）—— fps 非法时它返回空数组，
  // 这里跟着返回空（"算不出来"不猜默认值）。
  const resolved = edlOutStart(clips, fpsSafe)
  // `edlOutStart` 会**跳过非法段**（倒置/零长/非正速率），所以它的下标与 `clips` 不一定对齐。
  // 这里的块是"给用户看的每一段"（含非法段，标红提示），所以逐个 clip **独立**取各自的
  // 帧区间，不做跨段累加 —— 累加值只用来定 `outLeft`，取自 `resolved` 的同源游标。
  let cursorFrames = 0
  const out = []
  for (let i = 0; i < clips.length; i += 1) {
    const c = clips[i] || {}
    const ss = Number(c.source_start)
    const se = Number(c.source_end)
    const speed = Number.isFinite(Number(c.speed)) && Number(c.speed) > 0 ? Number(c.speed) : 1
    const sf = Number.isFinite(ss) ? Math.round(ss * fpsSafe) : 0
    const ef = Number.isFinite(se) ? Math.round(se * fpsSafe) : 0
    const srcFrames = ef - sf
    const durFrames = Math.round(srcFrames / speed)
    const ok = Number.isFinite(ss) && Number.isFinite(se) && srcFrames > 0 && durFrames >= 1 && speed > 0
    // 非法段**不占成片时长**（与 Rust 跳过同口径），但**照样画出来**并标红 ——
    // 静默不画 = 用户看到"我加的那段没了"，比画一个红块难懂得多。
    const outLeft = ok ? cursorFrames / fpsSafe : 0
    const outSecs = ok ? durFrames / fpsSafe : 0
    out.push({
      index: i,
      clip: c,
      invalid: !ok,
      speed: ok ? speed : Number(c.speed) || 1,
      source_start: Number.isFinite(ss) ? ss : 0,
      source_end: Number.isFinite(se) ? se : 0,
      sourceSecs: Number.isFinite(ss) && Number.isFinite(se) ? Math.max(0, se - ss) : 0,
      // 成片轴
      outStartSecs: outLeft,
      outEndSecs: outLeft + outSecs,
      outLeft: outLeft * s,
      outWidth: Math.max(outSecs * s, 8),
      // 源轴（只读展示用；**源轴上的块不因 EDL 移动**）
      srcLeft: (Number.isFinite(ss) ? ss : 0) * s,
      srcWidth: Math.max((Number.isFinite(ss) && Number.isFinite(se) ? se - ss : 0) * s, 8),
      durFrames,
      kind: c.kind || '',
      label: c.label || '',
      sourceKind: edlSourceKind(c),
      sourceLabel: edlSourceLabel(c),
      // 转场（第 7 批）：`'dissolve'` 时成片轨上要有个可见记号 ——
      // 光靠"面板里选了"不够，用户扫一眼成片轨要能看出**哪几段之间有叠化**。
      //
      // ⚠️ **`i > 0` 才算数**：叠化是"与**前一段**重叠"，成片的第一段没有前一段可叠
      // （Rust `dissolve_overlap_frames` 与转译器 `mainGeom` 都按这条退化成硬切）。
      // 界面必须同口径 —— 否则第一段会挂一个"出片时不存在"的叠化角标，那是撒谎。
      transition: i > 0 ? edlTransition(c) : null,
      transitionLabel: i > 0 ? edlTransitionLabel(c) : '硬切',
    })
    if (ok) cursorFrames += durFrames
  }
  return out
}

/**
 * 成片轨的总时长（秒）= 最后一段的 `out_end`（§2.2 规则 2 的口径）。
 * ⚠️ **不是**"源时长"、也不是"各段源长之和"（变速之后两者都不对）。
 * 后端 `sequence_frames` 读的也是这个值 —— 这里与它必须一致（R7）。
 */
export function edlOutSecs(config, fps) {
  const clips = (config && config.edl && config.edl.clips) || []
  if (!Array.isArray(clips) || !clips.length) return 0
  const fpsSafe = Number.isFinite(fps) && fps > 0 ? fps : 30
  return edlTotalFrames(clips, fpsSafe) / fpsSafe
}

// ── 编辑动作 ────────────────────────────────────────────────────
// 全部是**纯函数**（收 clips 数组 → 返回**新数组**）：调用方负责落盘与镜像。
// 就地改会让 Vue 的响应式在"同一批改动"里分辨不出前后（也因此不在这里碰 `config`）。

/** 把秒值夹进 [0, dur] 并对齐到 2 位小数（与 `TimelineEditor` 的 `round2` 同口径）。 */
function clampSec(v, dur) {
  const n = Number(v)
  if (!Number.isFinite(n)) return 0
  return Math.round(Math.max(0, Math.min(dur > 0 ? dur : n, n)) * 100) / 100
}

/** 造一段（默认原速、`kind='replay'` 由调用方按位置覆盖）。 */
export function makeClip(start, end, speed = 1, kind = '') {
  const c = { source_start: start, source_end: end, speed }
  if (kind) c.kind = kind
  return c
}

/**
 * 加一段。`where` 决定插到哪儿（§6.3 第 ② 条的三个入口）：
 *  · `head`   —— 插到片头（成片的第一段）
 *  · `at`     —— 插到指定下标（调用方按播放头折算）
 *  · `tail`（缺省）—— 追加到片尾
 */
export function addClip(clips, clip, where = 'tail', at = -1) {
  const next = [...(clips || [])]
  if (where === 'head') next.unshift(clip)
  else if (where === 'at' && at >= 0 && at <= next.length) next.splice(at, 0, clip)
  else next.push(clip)
  return next
}

/**
 * 「×2 重放」= **复制这一段并排在其后**（§6.3 第 ④ 条：这就是"连续重放 2~3 次"）。
 * 连点几次就是 3 次、4 次。返回新数组，不就地改。
 */
export function duplicateClipAt(clips, i) {
  const next = [...(clips || [])]
  const src = next[i]
  if (!src) return next
  next.splice(i + 1, 0, { ...src, kind: src.kind || 'replay' })
  return next
}

/** 删一段。 */
export function removeClipAt(clips, i) {
  const next = [...(clips || [])]
  if (i < 0 || i >= next.length) return next
  next.splice(i, 1)
  return next
}

/**
 * 改一段的**源区间**（拖左右缘 / 面板填秒数走这里）。
 * 顺序化两端 + 夹到源时长内 + 保证至少 `MIN_CLIP_SECS`、并对齐 2 位小数。
 */
export const MIN_CLIP_SECS = 0.1
export function setClipSpan(clips, i, start, end, dur) {
  const next = [...(clips || [])]
  const cur = next[i]
  if (!cur) return next
  const lim = Number.isFinite(dur) && dur > 0 ? dur : Number.MAX_SAFE_INTEGER
  let a = clampSec(start, lim)
  let b = clampSec(end, lim)
  if (b - a < MIN_CLIP_SECS) {
    // 退化：优先保证"至少一小段"，并把两端都留在 [0, dur] 内
    if (a + MIN_CLIP_SECS <= lim) b = Math.round((a + MIN_CLIP_SECS) * 100) / 100
    else { b = lim; a = Math.round(Math.max(0, lim - MIN_CLIP_SECS) * 100) / 100 }
  }
  next[i] = { ...cur, source_start: a, source_end: b }
  return next
}

/** 改一段的**速率**（面板档位）。夹进 `[SPEED_MIN, SPEED_MAX]`（与 Rust validate 同区间）。 */
export function setClipSpeed(clips, i, speed) {
  const next = [...(clips || [])]
  const cur = next[i]
  if (!cur) return next
  const n = Number(speed)
  if (!Number.isFinite(n) || n <= 0) return next
  const v = Math.min(EDL_SPEED_MAX, Math.max(EDL_SPEED_MIN, n))
  next[i] = { ...cur, speed: Math.round(v * 1000) / 1000 }
  return next
}

/** 改一段的显示名（`label`，可选；清空即删字段，保持配置干净）。 */
export function setClipLabel(clips, i, label) {
  const next = [...(clips || [])]
  const cur = next[i]
  if (!cur) return next
  const copy = { ...cur }
  const s = String(label == null ? '' : label).trim()
  if (s) copy.label = s
  else delete copy.label
  next[i] = copy
  return next
}

/**
 * 改一段的**入场转场**（第 7 批 / task-10）：`'dissolve'` 写入、其余（含 `'cut'`）**删键**。
 *
 * ⚠️ **硬切一律走"删键"**：字段缺席 / 显式 `'cut'` / 认不出的档位在三处消费点同口径
 * （全是硬切），但删键让"没动过转场"的工程**逐字节不变**（R4）——
 * 写显式 `'cut'` 会让每点一次按钮就在 `polish.toml` 里多一行无信息量的键。
 */
export function setClipTransition(clips, i, transition) {
  const next = [...(clips || [])]
  const cur = next[i]
  if (!cur) return next
  const copy = { ...cur }
  if (transition === EDL_TRANSITION_DISSOLVE) copy.transition_in = EDL_TRANSITION_DISSOLVE
  else delete copy.transition_in
  next[i] = copy
  return next
}

/**
 * 把源时间轴上框选的一段加进成片（§6.3 第 ①→② 步）。
 * 返回新数组；`where` / `at` 语义同 [`addClip`]。
 */
export function addRangeToEdl(clips, start, end, dur, where = 'tail', at = -1) {
  const lim = Number.isFinite(dur) && dur > 0 ? dur : Number.MAX_SAFE_INTEGER
  let a = clampSec(start, lim)
  let b = clampSec(end, lim)
  if (b - a < MIN_CLIP_SECS) b = Math.min(lim, Math.round((a + MIN_CLIP_SECS) * 100) / 100)
  if (b <= a) return [...(clips || [])]
  // 新加的段默认打 `replay` 标签：它是"把这段再放一次"，与正片区分得开（kind 只给 UI 用）。
  return addClip(clips, makeClip(a, b, 1, 'replay'), where, at)
}

/**
 * 播放头所在的那一段（按**成片时间**找 —— 成片轨上的播放头就是按它画的）。
 * 没有落进任何一段（理论上不会：块首尾相接，§6.4 方案 A）就返回最后一段。
 */
export function clipIndexAtOut(blocks, outSecs) {
  if (!blocks.length) return -1
  for (let i = 0; i < blocks.length; i += 1) {
    const b = blocks[i]
    if (b.invalid) continue
    if (outSecs < b.outEndSecs) return i
  }
  return blocks.length - 1
}

/**
 * 播放头所在的那一段（按**源时间**找）= 用户框选后「插到当前播放头」的目标下标。
 * 取"源区间包住播放头"的**最后**一段（同源区间可能被重放多次，插在那个重放后面最直观）；
 * 一段都没包住时退化成"源起点在播放头之前的最后一段"。
 */
export function clipIndexAtSource(blocks, srcSecs) {
  if (!blocks.length) return -1
  let inside = -1
  let before = -1
  for (let i = 0; i < blocks.length; i += 1) {
    const b = blocks[i]
    if (srcSecs >= b.source_start && srcSecs <= b.source_end) inside = i
    if (b.source_start <= srcSecs) before = i
  }
  return inside >= 0 ? inside : before
}

/**
 * 成片轴上的一秒 → 源轴上的一秒（§6.2 第 3 条：两轨共享一个播放头，但成片轨显示映射后的位置）。
 *
 * 这是 `edlOutStart` 的**逆映射**（分段线性）：段内成片秒 → 该段源秒。
 * 落在段内按**速率**线性插值（`×2` 的段里成片走 1 秒 = 源走 2 秒）。
 * 越界夹到首/末段。空 EDL 返回 `null`（这条路没开，调用方自己回落到"源=成片"）。
 */
export function edlSourceSecsAtOut(blocks, outSecs) {
  if (!blocks.length) return null
  const t = Math.max(0, Number(outSecs) || 0)
  const valid = blocks.filter((b) => !b.invalid)
  if (!valid.length) return null
  for (const b of valid) {
    if (t < b.outEndSecs) {
      const dt = t - b.outStartSecs
      return Math.min(b.source_end, b.source_start + dt * b.speed)
    }
  }
  const last = valid[valid.length - 1]
  return last.source_end
}

/**
 * 源轴上的一秒 → 成片轴上的一秒（`edlSourceSecsAtOut` 的正向）。
 * 播放头用**这个**把当前源时间投影到成片轨上。
 * 落在**没有任何成片段**的源区间里（被删掉的那段）返回 `null` —— 那一段在成片里不存在，
 * 不是"跳过去"（§6.6：预览据此跳"不在任何成片段里的源区间"）。
 */
export function edlOutSecsAtSource(blocks, srcSecs) {
  if (!blocks.length) return null
  const t = Math.max(0, Number(srcSecs) || 0)
  const valid = blocks.filter((b) => !b.invalid)
  if (!valid.length) return null
  for (const b of valid) {
    if (t >= b.source_start && t <= b.source_end) {
      return Math.min(b.outEndSecs, b.outStartSecs + (t - b.source_start) / b.speed)
    }
  }
  return null
}
