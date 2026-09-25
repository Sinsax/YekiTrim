// engine.js — 从 crates/vtrim-polish/templates/index.html 的 Canvas 渲染器
// （真实出片路径 renderFrame）忠实移植的纯函数特效状态机。
// buildIndex() 一次性预处理 → 各 *At(idx, t) 逐帧查询覆盖层状态。不读 DOM、不持有组件状态。
// 修改特效表现时先改 templates/index.html，再同步移植到这里，保持预览与出片一致。
// 画布尺寸/缩放一律取自 ./canvas.js（`_canvas` 契约），不再写死 1920×1080。
import { canvasOf, stickerSize, stickerMargin, stickerFloatUp } from './canvas.js'

// ---- GSAP 兼容 easing ----
export const E = {
  pow2Out: (t) => 1 - (1 - t) * (1 - t),
  pow2In: (t) => t * t,
  pow2InOut: (t) => (t < 0.5 ? 2 * t * t : 1 - Math.pow(-2 * t + 2, 2) / 2),
  sineOut: (t) => Math.sin((t * Math.PI) / 2),
  sineIn: (t) => 1 - Math.cos((t * Math.PI) / 2),
  sineInOut: (t) => -(Math.cos(Math.PI * t) - 1) / 2,
  backOut: (t, s = 1.70158) => {
    t -= 1
    return t * t * ((s + 1) * t + s) + 1
  },
  linear: (t) => t,
}

export function lerp(a, b, t) { return a + (b - a) * t }

// ---- 瞬时特效时长钳制（与 process/fx.rs transient_duration_cap 一致）----
export const TRANSIENT_CAP = {
  shake: 0.5, flash: 0.3, blur: 0.4, zoom_bounce: 0.6, stutter: 0.35,
  split: 0.35, noise: 0.4, pulse: 0.5, hue_shift: 0.5, color_shift: 0.5,
  vignette: 0.6,
}

/**
 * 事件跨度（秒）= end - time。事件统一写「开始 + 结束」；老数据/老配置的
 * `duration` 只做兜底（与 templates/index.html 的 evDur 逐行对应）。
 */
export function evDuration(ev, def) {
  if (!ev) return def
  if (ev.end != null) return Math.max(0.1, ev.end - (ev.time || 0))
  if (ev.duration != null) return Math.max(0.1, ev.duration)
  return def
}

export function transientDur(ev, def) {
  const cap = TRANSIENT_CAP[ev.type] || 0.6
  return Math.max(0.1, Math.min(evDuration(ev, def), cap))
}

// ---- 九宫格格子矩形（百分比；命名与 Rust render-core/types.rs grid_rect 一致）----
// 顺序同 Rust 的 GRID_CELLS 列表：左上→右下。
export const GRID_RECTS = {
  'top-left': [0, 0, 33.33, 33.33],
  'top': [33.33, 0, 66.67, 33.33],
  'top-right': [66.67, 0, 100, 33.33],
  'mid-left': [0, 33.33, 33.33, 66.67],
  'center': [33.33, 33.33, 66.67, 66.67],
  'mid-right': [66.67, 33.33, 100, 66.67],
  'bot-left': [0, 66.67, 33.33, 100],
  'bottom': [33.33, 66.67, 66.67, 100],
  'bot-right': [66.67, 66.67, 100, 100],
}

const GRID_CENTERS = Object.fromEntries(
  Object.entries(GRID_RECTS).map(([k, r]) => [k, [(r[0] + r[2]) / 2, (r[1] + r[3]) / 2]])
)

export function gridOrigin(name) {
  const p = GRID_CENTERS[name] || [50, 50]
  return p[0] + '% ' + p[1] + '%'
}

const clampPct = (v) => { const n = parseFloat(v); return Number.isFinite(n) ? Math.min(100, Math.max(0, n)) : 0 }

/**
 * 自由选框（`top_left` / `bottom_right`，百分比）→ {x0,y0,x1,y1}；
 * 未设置或退化（宽/高 < 0.5%）返回 null。与 Rust `types.rs::free_rect` 语义一致：
 * 顺序化 + 裁剪到 0~100，且**优先于** grid。
 */
export function freeRect(region) {
  const tl = region && region.top_left
  const br = region && region.bottom_right
  if (!tl || !br) return null
  let [x0, x1] = [clampPct(tl.x), clampPct(br.x)].sort((a, b) => a - b)
  let [y0, y1] = [clampPct(tl.y), clampPct(br.y)].sort((a, b) => a - b)
  if (x1 - x0 < 0.5 || y1 - y0 < 0.5) return null
  return { x0, y0, x1, y1 }
}

/** 区域中心（"x% y%"）：自由选框 > origin_grid > origin > 首个格子 > 65% 50%（与 Rust region_origin 一致）。 */
export function regionOrigin(region) {
  const fr = freeRect(region)
  if (fr) return ((fr.x0 + fr.x1) / 2).toFixed(2) + '% ' + ((fr.y0 + fr.y1) / 2).toFixed(2) + '%'
  if (!region) return '65% 50%'
  if (region.origin_grid) return gridOrigin(region.origin_grid)
  if (region.origin) return String(region.origin)
  if (region.grid && region.grid.length) return gridOrigin(region.grid[0])
  return '65% 50%'
}

/**
 * 区域当前占用的矩形，用于「框选布局」进入时的初始选框：
 * 自由选框优先；否则取所有九宫格格子的包围盒；都没有 → null。
 */
export function regionRect(region) {
  const fr = freeRect(region)
  if (fr) return fr
  let b = null
  for (const name of (region && region.grid) || []) {
    const r = GRID_RECTS[name]
    if (!r) continue
    b = b
      ? [Math.min(b[0], r[0]), Math.min(b[1], r[1]), Math.max(b[2], r[2]), Math.max(b[3], r[3])]
      : [r[0], r[1], r[2], r[3]]
  }
  return b ? { x0: b[0], y0: b[1], x1: b[2], y1: b[3] } : null
}

// ---- 弹幕文字宽度预测量（canvas measureText，仅用离屏画布量宽）----
let dmMeasureCtx = null
// 按数组身份缓存：拖拽时每 60ms 重建一次索引，几万条弹幕逐个 measureText 会明显卡顿
// （弹幕数组本身不变，只有 events 在改）。
const dmWidthCache = new WeakMap()
export function measureDanmakuWidths(dms, fontSize) {
  if (!dms || !dms.length) return []
  const size = fontSize || 42
  const hit = dmWidthCache.get(dms)
  if (hit && hit.size === size) return hit.widths
  if (!dmMeasureCtx) {
    const c = document.createElement('canvas')
    dmMeasureCtx = c.getContext('2d')
  }
  dmMeasureCtx.font = '600 ' + size + 'px sans-serif'
  const widths = dms.map((d) => dmMeasureCtx.measureText(d.text || '').width + 20)
  dmWidthCache.set(dms, { size, widths })
  return widths
}

function pctPx(v, total) {
  const n = parseFloat(v)
  return Number.isFinite(n) ? (n / 100) * total : NaN
}

// 贴纸安全范围：盒子 280×短边倍率、边距 32×短边倍率（与 placement.rs
// `Frame::sticker_scale` 同源；横竖画布短边都是 1080 ⇒ 恒为 280/32）。
export const STICKER_BASE = 280
export const STICKER_MARGIN_BASE = 32
function stickerGeom(idx) {
  const cv = canvasOf(idx.canvas)
  return { cv, s: stickerSize(cv), m: stickerMargin(cv) }
}
// 把**盒子左上角**钳进安全区（与 templates/index.html 的 stickerBox 逐行对应）。
// ⚠️ 只钳左上角；上浮量在本函数之外叠加 —— 出片侧是 GSAP transform（作用在已钳制
// 的位置之后），GPU 侧（timeline.rs）同样是"先钳中心再叠 y_off"。别再改回"先加 yOff 再钳"。
function clampSticker(g, left, top) {
  return {
    x: Math.max(g.m, Math.min(g.cv.W - g.s - g.m, left)),
    y: Math.max(g.m, Math.min(g.cv.H - g.s - g.m, top))
  }
}

// 贴纸落点（百分比）——与 templates/index.html 的 stickerAnchor 同构，改一处必须改两处。
// 返回值是**锚点**，`center` 说明它是盒子中心还是左上角：
//   auto   = 事件自带的 x/y（process/placement.rs 的安全区算法算好写进 segments），
//            含义是**盒子左上角**（placement.rs 模块头第 2 条）
//   manual = 全部固定到 `[sticker] position` 那一点（电台/竖屏常用），忽略事件的 x/y，
//            含义是**盒子中心**（config.rs 的 `StickerConfig.position` 注释 / 设置页口径）
// 两个键基准不同、不要"统一"。两种模式都保留入场动画与出框 clamp ——
// "固定"指落点固定，不是取消动画。
function stickerAnchor(idx, ev) {
  const st = (idx.canvas && idx.canvas.sticker) || null
  if (st && st.position_mode === 'manual') {
    const parts = String(st.position || '50% 50%').trim().split(/\s+/)
    const mx = parseFloat(parts[0])
    const my = parseFloat(parts.length > 1 ? parts[1] : parts[0])
    return { x: Number.isFinite(mx) ? mx : 50, y: Number.isFinite(my) ? my : 50, center: true }
  }
  const ex = parseFloat(ev.x); const ey = parseFloat(ev.y)
  return { x: Number.isFinite(ex) ? ex : 50, y: Number.isFinite(ey) ? ey : 50, center: false }
}

// ---- 构建事件索引（预处理，只跑一次）----
// opts: { config, subtitles, danmaku, duration, stickerSrc }
//   stickerSrc(name) → 贴纸资源 URL（来自 config._assets.sticker_map，由调用方解析为媒体 URL）
export function buildIndex({ config, subtitles, danmaku, duration, stickerSrc }) {
  const cfg = config || {}
  let dur = duration || cfg.duration || 0

  // segments 截断到视频时长（与模板一致，防 LLM 生成的超出）
  const segs = (cfg.segments || []).map((s) => ({ ...s, events: [...(s.events || [])] }))
  if (segs.length > 0 && dur > 0) {
    const kept = segs.filter((s) => s.start < dur)
    kept.forEach((s) => {
      s.end = Math.min(s.end, dur)
      s.events = s.events.filter((e) => (e.time || 0) < dur)
    })
    segs.length = 0
    segs.push(...kept)
  }

  const idx = {
    camera: [], sticker: [], shake: [], flash: [], blur: [], overlay: [],
    sfx: [], stutter: [], split: [], zoom_bounce: [], vignette: [],
    hue_shift: [], noise: [], pulse: [], color_shift: [],
  }
  const soundEvents = [] // 音效事件表：sfx 类型事件 + 任意带 ev.sfx 的事件
  const ptEvents = [] // playtest 标签表（拍平全部事件）

  segs.forEach((seg) => {
    const segLabel = seg.note || ''
    ;(seg.events || []).forEach((ev) => {
      const type = ev.type || ''
      const entry = { time: ev.time || 0, duration: evDuration(ev, 0), ev }
      if (idx[type]) idx[type].push(entry)
      else idx[type] = [entry]
      // 音效：规则引擎指定的 sfx（无引用则静音）
      if (type === 'sfx') {
        soundEvents.push({ time: entry.time, name: ev.name || 'pop-notice', volume: ev.volume || sfxVolume(cfg), duration: evDuration(ev, 1.0) })
      } else if (ev.sfx) {
        const sd = Math.max(transientDur(ev, 1.0), 1.0)
        soundEvents.push({ time: entry.time, name: ev.sfx, volume: ev.sfx_volume || sfxVolume(cfg), duration: sd })
      }
      ptEvents.push({
        time: entry.time, end: entry.time + evDuration(ev, 1.5),
        type, label: ev.keyword || ev.name || ev.mode || '', seg: segLabel,
      })
      if (type === 'sticker') {
        const name = ev.name || ev.keyword || ''
        entry.src = stickerSrc ? stickerSrc(name) : ''
        entry.isEmoji = /^[0-9a-f]+$/i.test(name) && !entry.src
        entry.emoji = entry.isEmoji ? safeEmoji(name) : ''
        // 落点不在这里改：auto 用 ev.x/ev.y（process 安全区算法），
        // manual 固定到 _canvas.sticker.position —— 统一在 stickersAt 里算，
        // 免得"落点"散在索引期与求值期两处（老代码的竖屏 xOverride 就是这种双头）。
      }
    })
  })
  ;['camera', 'shake', 'flash', 'blur', 'overlay', 'stutter', 'split', 'zoom_bounce',
    'vignette', 'hue_shift', 'noise', 'pulse', 'color_shift', 'sfx']
    .forEach((k) => idx[k] && idx[k].sort((a, b) => a.time - b.time))

  idx.subs = subtitles || []
  idx.dms = danmaku || []
  idx.dmWidths = measureDanmakuWidths(idx.dms, dmFontSize(cfg))
  idx.dur = dur
  idx.layout = cfg.layout || {}
  idx.subStyle = (cfg.style && cfg.style.subtitle) || {}
  idx.dmStyle = (cfg.style && cfg.style.danmaku) || {}
  idx.camStyle = (cfg.style && cfg.style.camera) || {}
  idx.vertical = !!cfg.vertical
  // 画面构成契约（Rust resolve_stage 唯一推导）：贴纸落点模式 / 字幕安全宽 / 画布尺寸都从这里取
  idx.canvas = cfg._canvas || null
  // 画布参数（由 `_canvas` 推导）：百分比 → 像素、运镜不动点、贴纸几何都用它
  idx.cv = canvasOf(idx.canvas)
  idx.soundEvents = soundEvents
  idx.ptEvents = ptEvents
  idx.segs = segs
  return idx
}

function sfxVolume(cfg) { return (cfg.sfx && cfg.sfx.volume) || 0.1 }
function dmFontSize(cfg) { return (cfg.style && cfg.style.danmaku && cfg.style.danmaku.font_size) || 42 }

function safeEmoji(name) {
  const code = parseInt(name, 16)
  return Number.isNaN(code) ? '✨' : String.fromCodePoint(code)
}

// ---- 运镜档位 → 缩放系数。必须与 Rust CameraStyle::factor 逐值一致 ----
// 缺省 1.3/1.5/0.55（与 render-core 的 Default 相同）；这里曾写 1.8/1.5，
// 于是 [style.camera] 缺失时预览比成片小一大截。
export function camFactor(label, camStyle) {
  const c = camStyle || {}
  const pick = (v, d) => (typeof v === 'number' && Number.isFinite(v) ? v : d)
  if (label === '近') return pick(c.zoom_close, 1.5)
  if (label === '远') return pick(c.zoom_wide, 0.55)
  return pick(c.zoom_mid, 1.3)
}

// ---- 手持摇摆（运镜的附加变种，`Event::Camera::handheld`） ----
// 与 vtrim-render-core 的 `types::handheld_sway`、模板 `handheldSway` **逐值同源**
// （Rust↔JS 由 `timeline.rs::handheld_sway_matches_js_reference` 钉住，
// 模板/engine 两个 JS 副本由 `webui/scripts/check-handheld.mjs` 对拍）。
// 位移单位 = 1920×1080 输出像素（预览里画布 px 与 CSS px 同构，直接当 px 用）；
// 倾斜是度，绕聚焦中心；波形纯 `t` 的函数（无累积状态 → 跳帧/seek 结果一致）。
const HANDHELD_TAU = Math.PI * 2
export function handheldSway(t, amp) {
  const a = amp > 0 ? amp : 0
  if (a === 0) return { dx: 0, dy: 0, rot: 0, scale: 1 }
  return {
    dx: (6.5 * Math.sin(HANDHELD_TAU * 0.47 * t) + 3.0 * Math.sin(HANDHELD_TAU * 0.83 * t + 1.7)) * a,
    dy: (5.0 * Math.sin(HANDHELD_TAU * 0.59 * t + 0.9) + 2.4 * Math.sin(HANDHELD_TAU * 1.13 * t + 2.4)) * a,
    rot: (0.32 * Math.sin(HANDHELD_TAU * 0.37 * t + 0.4) + 0.16 * Math.sin(HANDHELD_TAU * 0.91 * t + 2.9)) * a,
    scale: 1.0 + 0.007 * Math.sin(HANDHELD_TAU * 0.53 * t + 1.1) * a
  }
}

// ---- 运镜 ----
export function cameraAt(idx, t) {
  let zoom = 1.0
  let ox = 65
  let oy = 50
  // 手持摇摆的**幅度** 0~1：与 zoom 同一段窗口、同一条缓动（状态机：保持到下一条运镜）
  let handheld = 0
  // 默认聚焦中心：自由选框（框选布局）> origin_grid > origin > 首个格子
  const defOrigin = regionOrigin(idx.layout.liver)
  const dp = defOrigin.split(' ')
  ox = parseFloat(dp[0]) || 65
  oy = parseFloat(dp[1]) || 50

  // 竖屏不做运镜 —— 手持摇摆是运镜的附加变种，跟随屏蔽
  if (idx.vertical) return { zoom: 1.0, ox: (ox / 100) * idx.cv.W, oy: (oy / 100) * idx.cv.H, handheld: 0 }

  const wide = camFactor('远', idx.camStyle)
  for (const c of idx.camera) {
    if (t < c.time) break
    const tgt = camFactor(c.ev.zoom, idx.camStyle)
    const td = Math.min(c.duration * 0.5, 0.6)
    const ce = c.time + td
    // 硬切：本事件写了 cut（编辑器「方式 = 硬切」），或远景（画中画）进出整档都是硬切：
    // 缓动缩过去看着像镜头在拉远，而它要的是喜剧式的切换，所以这一段不缓动、
    // 连位移也一并瞬间到位。cut 把同一条规则放开给任意景别。
    const hardCut = c.ev.cut === true || Math.abs(tgt - wide) < 1e-6 || Math.abs(zoom - wide) < 1e-6
    const p = t >= ce || hardCut ? 1.0 : E.pow2Out((t - c.time) / td)
    if (t >= ce || hardCut) zoom = tgt
    else zoom = lerp(zoom, tgt, p)
    // 手持摇摆是状态机上的持续项（跟到下一条不带 handheld 的运镜），同一段 p 起停
    handheld = lerp(handheld, c.ev.handheld === true ? 1 : 0, p)
    // 不动点：本事件的 origin → to_origin（缺省即原地不动）。
    // 起点取事件自己的 origin，与 zoom 同一段窗口、同一条缓动。
    const oa = String(c.ev.origin || defOrigin).split(' ')
    const ob = String(c.ev.to_origin || c.ev.origin || defOrigin).split(' ')
    const ax = parseFloat(oa[0]); const ay = parseFloat(oa[1])
    const bx = parseFloat(ob[0]); const by = parseFloat(ob[1])
    ox = lerp(Number.isFinite(ax) ? ax : ox, Number.isFinite(bx) ? bx : ox, p)
    oy = lerp(Number.isFinite(ay) ? ay : oy, Number.isFinite(by) ? by : oy, p)
  }
  return { zoom, ox: (ox / 100) * idx.cv.W, oy: (oy / 100) * idx.cv.H, handheld }
}

// ---- 视频层复合状态：shake/blur/colorShift/stutter/split/最终 zoom ----
// 竖屏不再屏蔽这些视觉特效（只有运镜 camera 被屏蔽，见 cameraAt 顶部）。
// 与 templates/index.html 的 handlers 段保持一致：作用对象本来就对，
// blur/color_shift 打 #video-wrap、其余打 #source-video，背景层是兄弟节点不受波及。
export function videoFxAt(idx, t) {
  const shake = getShake(idx, t)
  const blurAmt = getBlur(idx, t)
  const colorShift = getColorShift(idx, t)
  const stutter = getStutterVideo(idx, t)
  const split = getSplitVideo(idx, t)
  const cam = cameraAt(idx, t)
  const zb = getZoomBounce(idx, t, cam.zoom)
  const pl = getPulse(idx, t, cam.zoom)
  return {
    shake, blur: blurAmt, colorShift,
    stutter, split, zoom: Math.max(zb, pl), camera: cam,
  }
}

// ---- 瞬时 FX 状态（Canvas 出片行为的窗口函数）----
function getShake(idx, t) {
  for (const sh of idx.shake) {
    if (t >= sh.time && t < sh.time + 0.2) {
      const off = sh.ev.intensity || 14
      const dt = t - sh.time
      if (dt < 0.04) return { x: off, y: -off }
      if (dt < 0.08) return { x: -off, y: off }
      if (dt < 0.12) return { x: off / 2, y: -off / 2 }
      if (dt < 0.16) return { x: -off / 2, y: off / 2 }
      return { x: 0, y: 0 }
    }
  }
  return { x: 0, y: 0 }
}

export function getFlash(idx, t) {
  for (const f of idx.flash) {
    const et = f.time, color = f.ev.color || '#ffffff', dur = f.duration || 0.35
    if (t >= et && t < et + 0.04 + dur) {
      const dt = t - et
      if (dt < 0.04) return { opacity: 0.2 * (dt / 0.04), color }
      return { opacity: 0.2 * (1 - (dt - 0.04) / dur), color }
    }
  }
  return { opacity: 0, color: '#ffffff' }
}

function getBlur(idx, t) {
  for (const b of idx.blur) {
    const et = b.time
    if (t >= et && t < et + 0.8) {
      const dt = t - et
      if (dt < 0.48) return 8
      return 8 * (1 - (dt - 0.48) / 0.32)
    }
  }
  return 0
}

/**
 * 「该色 → 透明」里的**第二档**。
 * ⚠ 不要用 CSS 的 `transparent`（= rgba(0,0,0,0)）：canvas 的渐变在**非预乘**空间插值，
 *    `#ff0000 → transparent` 会从中间插出 rgba(127,0,0,·) 那种暗红 —— 实测同一条红色覆盖层
 *    （opacity 0.3、同一个源帧、同一个时刻）平均 R 从 184.12 掉到 178.82；换成同色零透明
 *    `rgba(255,0,0,0)` 则是 193.66（干净的淡出）。而 CSS 那边两者**完全等价**（按预乘插值，
 *    取样逐点相同），所以用 `transparent` 会让浏览器预览与出片观感不一致。
 */
export function overlayFadeStop(color) {
  const s = String(color == null ? '' : color).trim()
  const hex = /^#([0-9a-f]{3}|[0-9a-f]{6}|[0-9a-f]{8})$/i.exec(s)
  if (hex) {
    let h = hex[1]
    if (h.length === 3) h = h[0] + h[0] + h[1] + h[1] + h[2] + h[2]
    h = h.slice(0, 6)
    const n = (i) => parseInt(h.slice(i, i + 2), 16)
    return `rgba(${n(0)},${n(2)},${n(4)},0)`
  }
  const fn = /^rgba?\(\s*([\d.]+)\s*,\s*([\d.]+)\s*,\s*([\d.]+)/i.exec(s)
  if (fn) return `rgba(${fn[1] | 0},${fn[2] | 0},${fn[3] | 0},0)`
  return 'transparent' // 认不出的（命名色等）偏暗，但至少有东西
}

// 覆盖层「渐变」的有效色档 —— 与 Rust `timeline.rs::overlay_stops` 逐条对齐。
// 四处实现必须同改：本文件、timeline.rs、crates/vtrim-polish/templates/index.html（出片权威路径）、
// webui/src/components/PolishPreview.vue（用本文件的 gradientCss）。
//
// **保证 ≥2 档且无空串**：canvas 按 i/(n-1) 铺档，n==1 是 0/0=NaN（抛 IndexSizeError，
// 会把整个 chunk 的渲染打断），空串色档抛 SyntaxError。
// 老工程与 LLM 一直只写单个 color（colors 从未出现过），单色按「该色 → 透明」。
// 漂移检查：`node webui/scripts/check-overlay.mjs`
export function overlayStops(ev) {
  const raw = ev.colors
  const cleaned = []
  if (Array.isArray(raw)) {
    for (const c of raw) {
      const s = String(c == null ? '' : c).trim()
      if (s) cleaned.push(s)
    }
  }
  if (cleaned.length >= 2) return cleaned
  const base = cleaned.length ? cleaned[0] : String(ev.color == null ? '' : ev.color).trim()
  if (!base) return null
  return [base, overlayFadeStop(base)]
}

/**
 * 渐变覆盖层的 CSS 字符串（仅 DOM 预览用，canvas 出片走 createLinearGradient/createRadialGradient）。
 * ⚠ 结尾那个 ')' 不能省 —— 曾经少写一个，整条 background 声明非法被浏览器丢弃，
 *    表现就是「在面板里设了色档，预览里却什么都没有」。
 * 径向：canvas 是 createRadialGradient(960,540,0,…,960)，即中心该色、边缘淡出；
 *      CSS 圆的默认半径是 farthest-corner（16:9 下比 960 大约 15%），预览近似、出片为准。
 */
export function gradientCss(colors, shape) {
  const stops = colors
    .map((c, i) => `${c} ${((i / (colors.length - 1)) * 100).toFixed(1)}%`)
    .join(',')
  return shape === 'radial'
    ? `radial-gradient(circle at center, ${stops})`
    : `linear-gradient(to bottom right, ${stops})`
}

export function getOverlayState(idx, t) {
  for (const o of idx.overlay) {
    const et = o.time, dur = o.duration || 1.5
    const ev = o.ev
    if (t >= et && t < et + dur) {
      const dt = t - et
      const fadeIn = 0.4, fadeOut = 0.4
      // 数值判定，不用 `|| 1` —— 不透明度 0 是合法值，不能被 falsy 判成 1
      let opacity = typeof ev.opacity === 'number' ? ev.opacity : 1
      const ot = ev.overlay_type || ''
      const stops = overlayStops(ev)
      // fade_in/fade_out：**整窗就是过渡本身**（fade_in 起点全遮 → 末端全透，fade_out 反过来），
      // 是线性斜坡，**不叠**下面那 0.4s 进出包络 —— 叠了起点会乘成 0，黑场压根打不开
      // （用户看到的症状就是「开头渐入没生效」）。与出片模板 / Rust `get_overlay_state` 同口径。
      if (ot === 'fade_in' || ot === 'fade_out') {
        opacity *= ot === 'fade_in' ? 1 - dt / dur : dt / dur
        if (opacity < 0.005) continue
        // 缺省黑：面板切到这两档时不写颜色也能用
        const maskCol = (ev.color != null && String(ev.color).trim()) || (stops ? stops[0] : '') || '#000000'
        return { type: 'color', color: maskCol, opacity }
      }
      if (dt < fadeIn) opacity *= dt / fadeIn
      else if (dur - dt < fadeOut) opacity *= (dur - dt) / fadeOut
      if (opacity < 0.005) continue
      if (ot === 'color') {
        const solid = (ev.color != null && String(ev.color).trim()) || (stops ? stops[0] : '')
        if (solid) return { type: 'color', color: solid, opacity }
        continue
      }
      if (ot === 'gradient' && stops) return { type: 'gradient', colors: stops, shape: ev.shape || 'linear', opacity }
      // 类型不认识 / 值不全：继续看窗口里的下一条，别把整窗吃掉
    }
  }
  return null
}

function getStutterVideo(idx, t) {
  for (const st of idx.stutter) {
    const et = st.time, off = st.ev.intensity || 12
    if (t >= et && t < et + 0.12) return (t - et) < 0.06 ? { x: -off, y: off } : { x: off, y: -off }
  }
  return null
}

function getSplitVideo(idx, t) {
  for (const sp of idx.split) {
    const et = sp.time, inten = sp.ev.intensity || 30
    if (t >= et && t < et + 0.12) {
      return (t - et) < 0.06 ? { scale: 0.95, skewX: -inten, x: -inten * 0.3 } : { scale: 1, skewX: 0, x: 0 }
    }
  }
  return null
}

function getZoomBounce(idx, t, curZoom) {
  for (const zb of idx.zoom_bounce) {
    if (t >= zb.time && t < zb.time + 0.3) return (t - zb.time) < 0.08 ? curZoom * 1.1 : curZoom
  }
  return curZoom
}

function getPulse(idx, t, curZoom) {
  for (const p of idx.pulse) {
    if (t >= p.time && t < p.time + 0.48) {
      const dt = t - p.time
      if (dt < 0.12) return curZoom * 1.06
      if (dt < 0.28) return curZoom * 0.97
      return curZoom
    }
  }
  return curZoom
}

export function getHueShiftFlash(idx, t) {
  for (const h of idx.hue_shift) {
    const et = h.time, color = h.ev.color || '#ff0040'
    if (t >= et && t < et + 0.95) {
      const dt = t - et
      if (dt < 0.15) return { opacity: 0.25 * (dt / 0.15), color }
      return { opacity: 0.25 * (1 - (dt - 0.15) / 0.8), color }
    }
  }
  return null
}

export function getNoiseOpacity(idx, t) {
  for (const n of idx.noise) {
    const et = n.time
    if (t >= et && t < et + 0.8) {
      const dt = t - et
      if (dt < 0.08) return 0.08 * (dt / 0.08)
      return 0.08 * (1 - (dt - 0.08) / 0.72)
    }
  }
  return 0
}

export function getVignette(idx, t) {
  for (const v of idx.vignette) {
    const dur = transientDur(v.ev, 0.4)
    if (t >= v.time && t < v.time + dur) {
      const dt = t - v.time
      if (dt < dur * 0.4) return 1
      return Math.max(0, 1 - (dt - dur * 0.4) / Math.min(0.3, dur * 0.6))
    }
  }
  return 0
}

function getColorShift(idx, t) {
  for (const c of idx.color_shift) {
    const et = c.time
    if (t >= et && t < et + 1.0) {
      const dt = t - et
      if (dt < 0.1) return 'sepia(0.6) saturate(1.4)'
      const p = (dt - 0.1) / 0.6
      return 'sepia(' + 0.6 * (1 - p) + ') saturate(' + (1.4 - 0.4 * p) + ')'
    }
  }
  return ''
}

// ---- 字幕：fade in/out + 高亮分段 ----
export function subsAt(idx, t) {
  const out = []
  for (const s of idx.subs) {
    if (t < s.start || t >= s.end) continue
    let opacity = 1, yOff = 0
    const fadeIn = 0.35, fadeOut = 0.18
    if (t - s.start < fadeIn) {
      opacity = E.pow2Out((t - s.start) / fadeIn)
      yOff = 20 * (1 - opacity)
    } else if (s.end - t < fadeOut) {
      const p = (s.end - t) / fadeOut
      opacity = E.pow2In(p)
      yOff = -8 * (1 - p)
    }
    // 事件联动脉冲（scale 1.25 / y −20）
    const pulse = subPulse(idx, t, s)
    // html 只在高亮富文本时优先；普通旧数据里 html 可能与 text 脱节，应跟随用户编辑的 text
    const rawHtml = s.html || ''
    const html = rawHtml.includes('<span') ? rawHtml : (s.text || rawHtml || '')
    const parts = []
    const re = /<span class="hl">([^<]+)<\/span>|[^<]+/g
    let m
    while ((m = re.exec(html)) !== null) {
      if (m[1]) parts.push({ text: m[1], hl: true })
      else if (m[0].trim()) parts.push({ text: m[0], hl: false })
    }
    if (!parts.length) parts.push({ text: s.text || '', hl: false })
    out.push({ parts, opacity, yOff: yOff + pulse.yOff, scale: pulse.scale })
  }
  return out
}

// ---- 字幕折行（A5）—— 与 templates/index.html 的 wrapCaptionParts 同一算法 ----
// parts: [{text, hl}]；measure(text) → 基准字号下的像素宽。
// 安全宽 = maxLineW（调用方传画布宽 * 0.875）；最多 3 行；装不下缩字号（下限 0.7）。
// 逐字符贪心（中文无空格），高亮分段可跨行、断行处自动拆开再按 hl 合并。
let capMeasureCtx = null
export function capMeasure(text, font) {
  if (!capMeasureCtx) capMeasureCtx = document.createElement('canvas').getContext('2d')
  capMeasureCtx.font = font
  return capMeasureCtx.measureText(text).width
}
export function wrapCaptionParts(parts, maxLineW, measure) {
  const MAX_LINES = 3, MIN_RATIO = 0.7, EPS = 0.5
  const total = () => parts.reduce((w, p) => w + measure(p.text), 0)
  function wrap(scale) {
    const lines = []
    let cur = null, curW = 0
    const pushLine = () => { if (cur && cur.length) lines.push(cur); cur = null; curW = 0 }
    const append = (ch, hl, w) => {
      // ⚠️ 溢出换行后必须**重新起一行**：`pushLine()` 会把 cur 置空，紧接着读 `cur.length`
      // 就是 `TypeError: can't access property "length", cur is null`（竖屏安全宽只有 945px，
      // 一条长字幕必折行 → 播放到它就崩）。两行顺序不能对调。
      if (cur && cur.length && curW + w > maxLineW + EPS) pushLine()
      if (!cur) cur = []
      const last = cur.length ? cur[cur.length - 1] : null
      if (last && last.hl === hl) last.text += ch
      else cur.push({ text: ch, hl })
      curW += w
    }
    for (const p of parts) {
      for (const ch of p.text) append(ch, p.hl, measure(ch) * scale)
    }
    pushLine()
    return lines
  }
  const tw = total()
  let scale = 1
  if (tw > maxLineW + EPS) scale = Math.max(MIN_RATIO, Math.min(1, maxLineW * MAX_LINES / tw))
  return { lines: wrap(scale), scale }
}

function subPulse(idx, t, sub) {  // 与模板一致：事件落在字幕区间内时，该字幕 scale 1.25 / y −20 强调后 0.35s 回落
  for (const seg of idx.segs || []) {
    for (const ev of seg.events || []) {
      const et = ev.time || 0
      if (et < sub.start || et >= sub.end) continue
      if (t < et || t >= et + 0.47) continue
      if (t < et + 0.12) {
        const p = (t - et) / 0.12
        return { scale: 1 + 0.25 * p, yOff: -20 * p }
      }
      const p = (t - et - 0.12) / 0.35
      const k = E.sineInOut(1 - p)
      return { scale: 1 + 0.25 * k, yOff: -20 * k }
    }
  }
  return { scale: 1, yOff: 0 }
}

// ---- 弹幕滚动 ----
export function danmakuAt(idx, t) {
  const out = []
  for (let i = 0; i < idx.dms.length; i++) {
    const d = idx.dms[i]
    if (t < d.start || t >= d.end) continue
    const travel = d.travel_duration || 8
    const textW = idx.dmWidths[i] || (d.text || '').length * 55
    const el = t - d.start
    const fadeIn = 0.3, fadeOut = 0.2
    const cv = idx.cv
    const y = (d.y / 1920) * (540 * cv.k)
    const progress = Math.max(0, (el - fadeIn) / travel)
    let x = cv.W - Math.min(progress, 1) * (cv.W + textW)
    let opacity = 0.9
    if (el < fadeIn) {
      opacity = 0.9 * (el / fadeIn)
      x = cv.W
    } else if (t > d.end - fadeOut) {
      opacity = 0.9 * Math.max(0, (d.end - t) / fadeOut)
    }
    if (opacity < 0.005 || x < -textW - 50) continue
    out.push({ text: d.text || '', opacity, x, y })
  }
  return out
}

// ---- 贴纸四阶段：popIn back.out(3) / floatUp / hold / fadeOut ----
export function stickersAt(idx, t) {
  const out = []
  for (const e of idx.sticker) {
    const et = e.time, dur = e.duration || 2.5
    if (t < et || t > et + dur + 0.65) continue
    const popIn = 0.35, floatUp = 0.4, fadeOut = 0.3
    let opacity = 0, scale = 1, rot = 0, yOff = 0
    if (t - et < popIn) {
      const p = (t - et) / popIn
      const bp = E.backOut(p, 3)
      scale = 0.3 + 0.7 * bp
      rot = -15 + 15 * bp
      opacity = E.pow2Out(p)
    } else if (t < et + popIn + floatUp) {
      const p = (t - et - popIn) / floatUp
      opacity = 1; scale = 1; rot = 0
      yOff = -stickerFloatUp(idx.cv) * p
    } else if (t < et + popIn + floatUp + dur) {
      opacity = 1; yOff = -stickerFloatUp(idx.cv)
    } else {
      const p = (t - et - popIn - floatUp - dur) / fadeOut
      opacity = 1 - p
      scale = 1 - 0.6 * p
      yOff = -stickerFloatUp(idx.cv)
    }
    if (opacity < 0.005) continue

    // 位置：百分比 → 画布像素。manual 的锚点是**盒子中心**、auto 的事件 x/y 是**盒子左上角**，
    // 统一换成左上角再钳进安全区；上浮量在钳制**之后**叠加（出片侧是 GSAP transform，
    // GPU 侧 timeline.rs 也是"先钳再叠"）。
    const anc = stickerAnchor(idx, e.ev)
    const g = stickerGeom(idx)
    let left = pctPx(anc.x, g.cv.W)
    let top = pctPx(anc.y, g.cv.H)
    if (anc.center) { left -= g.s / 2; top -= g.s / 2 }
    const pos = clampSticker(g, left, top)
    out.push({
      key: e.ev.name || e.ev.keyword || '',
      name: e.ev.name || e.ev.keyword || '',
      src: e.src, isEmoji: e.isEmoji, emoji: e.emoji,
      opacity, scale, rot,
      x: pos.x, y: pos.y + yOff, w: g.s, h: g.s,
    })
  }
  return out
}
