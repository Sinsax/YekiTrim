// engine.js — 从 crates/yekitrim-polish/templates/index.html 的 Canvas 渲染器
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

/** 运镜速度档位表（§16）：`速度 → 过渡秒数`。与转译器 `CAMERA_SPEED_SECS` **同源**。
 *
 * ⚠️ **`med` 不在表里** —— 它是**老式子** `min(dur × 0.5, 0.6)`（与"没写过 `speed`"同一档）。
 * 把它写成固定秒数就破坏了存量零回归：短段的过渡会全变。
 */
export const CAMERA_SPEED_SECS = { snap: 0.10, fast: 0.25, slow: 1.5 }

/** 运镜过渡时长（秒），**与 `tools/polish-to-dhampir.mjs::cameraTransitionFrames` 同口径**。
 *
 * ⚠️ 一边改了另一边不改，症状是「预览里推进快慢与成片不一致」—— 不看两个版本对着比根本发现不了。
 * ⚠️ 转译器那边算出**帧**（还要 `round` + `max(1,…)`），这里要**秒**；两者只在
 * "同一档位对应同一段时间"这个层面上同源，别试图把 `max(1,…)` 也搬过来。
 *
 * - `hardCut` ⇒ 0（硬切不缓动；`cut` 与速度互斥、以 `cut` 为准）
 * - `med` / 没写 / 认不出 ⇒ 老式子 `min(dur × 0.5, 0.6)`
 * - `snap` / `fast` ⇒ 固定秒数；`slow` ⇒ `min(1.5, dur × 0.8)`（夹住，否则推不完就切走）
 */
export function cameraTransitionSecs(ev, dur, hardCut) {
  if (hardCut) return 0
  const norm = typeof ev?.speed === 'string' ? ev.speed.trim().toLowerCase() : ''
  if (Object.prototype.hasOwnProperty.call(CAMERA_SPEED_SECS, norm)) {
    return norm === 'slow' ? Math.min(CAMERA_SPEED_SECS.slow, dur * 0.8) : CAMERA_SPEED_SECS[norm]
  }
  return Math.min(dur * 0.5, 0.6)
}

// ---- 瞬时特效时长钳制（与 process/fx.rs transient_duration_cap 一致）----
export const TRANSIENT_CAP = {
  shake: 0.5, flash: 0.3, blur: 0.4,
  noise: 0.4, pulse: 0.5, hue_shift: 0.5,
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

// ---- 成片时间线（EDL）的 out_start 推导 ----
/**
 * `clips[]` → 每段的成片帧区间。**契约逐字冻结**（规格 §2.4 / §14），别改名、别改签名：
 * `scripts/check-edl-invariants.mjs` 用 `export function edlOutStart(clips, fps)` 正则探测它，
 * 找到即开始与 Rust `Edl::resolve_clips` / 转译器 `buildKeepRanges` **逐值对拍**（R7 预览≠成片）。
 *
 * ```
 * sf        = round(source_start * fps)          // 源两端各自量化成帧号
 * ef        = round(source_end   * fps)
 * durFrames = round((ef - sf) / speed)           // speed 缺省 1.0
 * out_start = 整数帧号域的 running sum            // ⚠️ 累加只在帧域，绝不在秒域
 * out_end   = out_start + durFrames
 * ```
 *
 * ⚠️ **不是** `round((source_end - source_start) * fps / speed)`：两者在**非帧对齐的秒值**上
 * 差 **1 帧**，而 `out_start` 是累加的 ⇒ 那 1 帧会顺延到**后面所有段**。Rust 侧与守卫的参照
 * 推导**各自独立踩到**过这一版，所以它写进规格。改回看起来"等价"的那一版 = 整个成片轨错位。
 *
 * ⚠️ **累加必须在帧号域**（R8 累加漂移）：`out_start += durFrames`（整数）而不是
 * `out_start += (source_end - source_start) / speed`（秒）。后者每段一次 f64 舍入会推着
 * 后面**所有**段一起偏 —— 段越多、变速越碎越明显。
 *
 * ⚠️ **非法段整段跳过、不占帧号**（`source_end <= source_start` / `speed <= 0` / 非有限 /
 * 算出来 < 1 帧）—— 与 Rust `edl_clip_frames` 返回 `None` 同口径。跳过的段**不出现在返回值里**，
 * 所以调用方拿到的下标与 `clips` 的下标**不一定一一对应**（自带的 `source_start` 可用来对回去）。
 *
 * @param {Array<{source_start:number, source_end:number, speed?:number}>} clips 秒域，有序 = 成片播放顺序
 * @param {number} fps 源素材帧率（帧号域累加的分母；`<= 0` / 非有限 ⇒ 空数组）
 * @returns {Array<{out_start:number, out_end:number, durFrames:number, source_start:number, source_end:number, speed:number}>}
 */
export function edlOutStart(clips, fps) {
  const out = []
  // fps 非法 ⇒ 帧号域的前提没了：与 Rust `Edl::resolve_clips` 一样返回空，不猜一个默认值。
  if (!Number.isFinite(fps) || fps <= 0) return out
  let cursor = 0 // **整数帧号**累加（R8）
  for (const c of clips || []) {
    if (!c) continue
    const speed = Number.isFinite(c.speed) && c.speed > 0 ? c.speed : 1.0
    const ss = Number(c.source_start)
    const se = Number(c.source_end)
    if (!Number.isFinite(ss) || !Number.isFinite(se)) continue
    const sf = Math.round(ss * fps)
    const ef = Math.round(se * fps)
    if (ef <= sf) continue // 倒置 / 零长：该段作废（与 Rust `edl_clip_frames` 同口径）
    const durFrames = Math.round((ef - sf) / speed)
    if (!(durFrames >= 1)) continue
    out.push({
      out_start: cursor,
      out_end: cursor + durFrames,
      durFrames,
      source_start: ss,
      source_end: se,
      speed,
    })
    cursor += durFrames
  }
  return out
}

/**
 * 成片总帧数 = **最后一段的 `out_end`**（规格 §2.2 规则 2，与 Rust `Edl::total_frames`、
 * 底座 `sequence_frames` 读所有层 `end` 最大值同一口径）。空 clips ⇒ `0`。
 */
export function edlTotalFrames(clips, fps) {
  const r = edlOutStart(clips, fps)
  return r.length ? r[r.length - 1].out_end : 0
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
  // ⚠️ **两位小数，与 Rust `types.rs::region_origin` 的 `{:.2}` 同口径**（2026-10-05 修）：
  // 不取整时这里给的是浮点artifact（`mid-right` 中心 = `83.33500000000001%`），而 Rust 给
  // `83.34%`、模板手抄表给 `83.33%` —— 三份"同一个物理点"的字符串形式各不相同。
  // 下游目前都走 `parseFloat` 所以看不出，但**一旦有人比较/落盘这个串**就会踩
  //（`regionOrigin` 的自由选框那支本来就有 `toFixed(2)`，是网格这支漏了）。
  return p[0].toFixed(2) + '% ' + p[1].toFixed(2) + '%'
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
export function measureDanmakuWidths(dms, fontSize, fontFamily) {
  if (!dms || !dms.length) return []
  const size = fontSize || 42
  // 字体族也是缓存键的一部分：换了字体（同一批弹幕、同字号）丈量结果会变，
  // 只按 size 命中就会拿旧字体的宽度去算滚动时长 —— 弹幕没滚到边就消失。
  //
  // ⚠️ `fontFamily` 收的是**完整字体栈**（`dmFontFamily(cfg)` 的返回值），这里**原样用**、
  //    不要再包一层引号 —— 它的取值已经含回退链（自己 + 雅黑 + Noto），
  //    包了会变成一个叫 `"X","Y",sans-serif` 的假族名（浏览器认不出 ⇒ 静默退默认体）。
  const family = String(fontFamily || '').trim()
  const hit = dmWidthCache.get(dms)
  if (hit && hit.size === size && hit.family === family) return hit.widths
  if (!dmMeasureCtx) {
    const c = document.createElement('canvas')
    dmMeasureCtx = c.getContext('2d')
  }
  dmMeasureCtx.font = '600 ' + size + 'px ' + (family || 'sans-serif')
  const widths = dms.map((d) => dmMeasureCtx.measureText(d.text || '').width + 20)
  dmWidthCache.set(dms, { size, family, widths })
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
/// **索引里预建哪些桶** —— 初始化与排序共用这一份清单（见 `buildIndex` 里那段事故注释）。
///
/// ⚠️ 它必须与 `templates/index.html::buildIndex` 的集合**完全一致**（那份是出片权威路径），
/// `scripts/check-preview-engine.mjs` 会逐项对拍；**加类型时两处一起加**。
export const INDEXED_TYPES = [
  'camera', 'sticker', 'shake', 'flash', 'blur', 'overlay',
  'vignette', 'hue_shift', 'noise', 'pulse', 'sfx', 'element',
]

/// **六条轨道的显示开关**（时间轴每条轨道前那枚眼睛，2026-10-06 用户口径）：
/// 事件类型 → `[stage.lanes] <轨道>`。
/// 不在表里 = 不受门控（认不出的类型照旧进索引）。
/// ⚠️ 这张表必须与 `tools/polish-to-dhampir.mjs::LANE_OF_TYPE`、Rust `Lane::of_event`、
///    `index.html` 里那份**同集合** —— 少一处就是「预览里有这一类、成片里没有」
///    （或反过来），而且**不报错**。
export const LANE_OF_TYPE = {
  camera: 'camera',
  // 元素（§5）**沿用 `sticker` 轨**（规格 §7 的座位表：只改显示标签为「元素」）——
  // 这样老工程零迁移，而"元素的眼睛"与"贴纸的眼睛"是同一个开关（它们本来就在一条轨道上）。
  element: 'sticker',
  sticker: 'sticker',
  sfx: 'sfx',
  shake: 'effects',
  blur: 'effects',
  flash: 'effects',
  hue_shift: 'effects',
  noise: 'effects',
  vignette: 'effects',
  overlay: 'effects',
  pulse: 'effects',
}

export function buildIndex({ config, subtitles, danmaku, duration, stickerSrc, karaokeCues, fps }) {
  const cfg = config || {}
  let dur = duration || cfg.duration || 0

  // **六条轨道的显示开关**（`cfg._lanes`，**由 Rust `ProjectFile::lanes_json` 解析后注入**）。
  // 只过滤**建索引**这一步，事件本身一个字都不动 —— 眼睛点亮、重新建索引即完全恢复。
  // ⚠️ 这里**不自己算默认值**：`None`/缺字段的默认（素材竖或画布竖 ⇒ 运镜与特效默认关 ……）
  //    只住 Rust `ProjectFile::lane_default` 一处；少了注入就当全开（宁可多画，不可静默吞掉）。
  //    转译器那份必须**逐条同口径**（它有第二份实现：读磁盘 toml + 自己探源尺寸）。
  const lanes = cfg._lanes || null
  const laneOn = (lane) => {
    const v = lanes ? lanes[lane] : undefined
    return typeof v === 'boolean' ? v : true
  }
  const laneAllows = (type) => {
    const lane = LANE_OF_TYPE[type]
    return !lane || laneOn(lane)
  }

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

  // ⚠️ **桶清单只能有一份**（2026-10-01 修事故）。以前这里手写对象字面量、下面又手写一条排序数组 ——
  // 上一轮删类型时两边各改各的，结果这里丢了 `vignette` / `sfx`，而排序那条还留着它们。
  // 后果不是"少个桶"那么轻：桶是**按需创建**的（见下面的 `else idx[type] = [entry]`），
  // 所以**工程里恰好没有该类型事件**时它就是 `undefined` ⇒ `getVignette(t, idx.vignette)`
  // 里 `for (const v of undefined)` 抛错 ⇒ 预览整块被 Vue 卸载（用户报的"视频和控件一起消失"，
  // 控制台是 `can't access property Symbol.iterator, d.vignette is undefined`）。
  // 现在：一份 `INDEXED_TYPES`，初始化与排序都用它；模板那份（`templates/index.html::buildIndex`）
  // 必须有**同一个集合**（`scripts/check-preview-engine.mjs` 逐项对拍）。
  const idx = {}
  for (const type of INDEXED_TYPES) idx[type] = []
  const soundEvents = [] // 音效事件表：sfx 类型事件 + 任意带 ev.sfx 的事件
  const ptEvents = [] // playtest 标签表（拍平全部事件）

  segs.forEach((seg) => {
    const segLabel = seg.note || ''
    ;(seg.events || []).forEach((ev) => {
      const type = ev.type || ''
      // 轨道眼睛关着（见上面 `laneAllows`）：这一类整条不进索引
      if (!laneAllows(type)) return
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
      } else if (type === 'element') {
        // ⚠️⚠️ **第四处缺陷**：`element` 分支**从来不设 `entry.src`** ——
        //   只有上面的 `sticker` 分支设。而 `getElements` 是拿 `entry.src` 当图源的
        //   ⇒ **就算把 (a) 放行、(b) 改读 `e.ev.*`，`src` 仍然是空的**。
        //   （这也是为什么"只修 :889 与 :917"不足以让贴纸画出来。）
        //
        //   解析口径与 `sticker` 分支**完全一致**（`name || keyword` → `stickerSrc`），
        //   但只在 `kind === 'sticker'` 时解析：`kind === 'text'` 是**有意留空**的
        //   （`getElements` 上方的注释：文字要动态盒算落点，故意不近似），
        //   给它解析图源会误导下一个人以为 text 支也通了。
        const kind = String(ev.kind ?? '').trim().toLowerCase()
        if (kind === 'sticker') {
          const name = ev.name || ev.keyword || ev.text || ''
          entry.src = stickerSrc ? stickerSrc(name) : ''
          entry.isEmoji = /^[0-9a-f]+$/i.test(name) && !entry.src
          entry.emoji = entry.isEmoji ? safeEmoji(name) : ''
        } else if (kind === 'text') {
          // §5 E3 预览侧：文字元素**直接读 `process` 已经算好的东西**，不重算。
          //
          // ⚠️⚠️ 这条以前是**有意留空**的，理由写的是「文字要动态盒算落点，故意不近似」
          //    + 「转译器会在报告里明说这条，所以这里静默是对的」。**两条都已经不成立**：
          //    ① 排版通道**早就实现**了（`process` 的 Step 1d3 `layout_text_elements`）
          //       —— 它把 `x`/`y`（百分比）与**解析后的样式串**写回事件；
          //    ② 转译器那条 `drop()` 也**换成了真实现**（独立 ASS 资产 + 单层字幕轨），
          //       真跑出片里文字是真的画出来的（A/B 帧已验）。
          //    ⇒ 于是预览与成片**真的分叉**：成片有字、预览没有，而且不报错。
          //
          // ★ **这里不去"近似"任何东西** —— 落点与样式都是 `process` 写好的，
          //   预览只是**读**。所以原来那句"故意不近似"的顾虑在这里**根本不适用**：
          //   读一份已经算好的值，不是自己近似一份。
          //   这也是本仓对付"两份实现漂移"的既定形态（与贴纸的 `source` 同一条）。
          entry.textBody = String(ev.text ?? '')
          entry.textStyle = parseTextSource(ev.source)
          // 落点同样是百分比字符串（`"9%"`），与贴纸 `x`/`y` 同一种表示
          entry.textX = numPct(ev.x)
          entry.textY = numPct(ev.y)
          entry.textPos = String(ev.pos ?? '').trim().toLowerCase() || 'bottom'
          // §5 E4a：文字元素的**动效**（T1 弹入 / T4 抖动 / T6 底衬 / T2 打字机）。
          // ⚠️ 参数来自工程级 `[elements]` 段 —— 与出片侧同一个来源。
          //
          // ---- 两轴（2026-10-13）：样式（形态）× 动效（时间），**各自读自己的字段** ----
          //
          // ⚠️⚠️ **这里以前只读全局** ⇒ 用户在时间轴上选了「摇摆 · 大角度」，
          //    转译器按预设写进关键帧、成片里真的摆，而**预览里纹丝不动**
          //    （预览压根不知道有这个字段）。这正是"配了、预览看不见"的那一类。
          //
          // ⚠️⚠️ **2026-10-13 两级化之后，这里与转译器一样要拆两路** ——
          //    否则用户在**新界面**上选「竖排 + 抖动」（写的是两个新字段），
          //    预览仍然只看老 `anim`（空）⇒ **预览完全没效果，而成片有**。
          //    这正是上一段那句话的同一个坑，只不过换了个字段。
          //
          // 优先级与转译器**逐条同源**：
          //   ① 新字段任一有值 ⇒ **以它为准**（`anim` 被忽略）；
          //   ② 两个新字段都空、`anim` 有值 ⇒ 按**参数归属**拆（只读迁移）；
          //   ③ 都没有 ⇒ 回落全局 `[elements]`。
          const legacySplit = splitAnimTwoAxis(ev.anim)
          const styleAnimName = String(ev.style_anim ?? '').trim().toLowerCase()
            || legacySplit.style
          const effectAnimName = String(ev.effect_anim ?? '').trim().toLowerCase()
            || legacySplit.effect
          // ⚠️ 第三路（过渡）：老格式的 / 就是过渡档，
          //    不接的话它们三路全空 ⇒ 静默回落全局（与转译器同一个坑）。
          const transitionAnimName = String(ev.transition_anim ?? '').trim().toLowerCase()
            || legacySplit.transition
          // ⚠️ 时间那半边取**动效轴**。
          entry.textMotion = animMotion(cfg, effectAnimName, transitionAnimName)
          // ⚠️ 档名**必须分两个带出去**（不只是摊进 `textMotion`）：
          //    `tilt_*` 那几档改的是**静态** `rotation`（不是时间函数），
          //    而 `textTiltTransform` 才是决定 `rotate()` 的地方 —— 它读**样式轴**；
          //    `per_char`（逐字）也走样式轴。
          //    ⚠️ `slam_squash` 的压扁同理（`scale_y`）。
          entry.styleAnimName = styleAnimName
          entry.effectAnimName = effectAnimName
          entry.transitionAnimName = transitionAnimName
          // ⚠️ 兼容字段：老代码里 `animName` 意思是"那一档"（形态+时间合一）。
          //    现在两轴都可能给东西 ⇒ 取**样式轴优先**（那是决定静态长相的那个），
          //    没有才用动效轴。与 `textTiltTransform` / `animPerChar` 的取值口径一致。
          entry.animName = styleAnimName || effectAnimName
          // ---- P4 补：**逐字 spec 也要带出去**（2026-10-13）----
          //
          // ⚠️⚠️ **`charTransformAt` 一直导出了、但全仓没有任何地方 import 它**
          //    ⇒ C 组逐字变换（15 档）+ D 组逐字错峰（4 档）**共 19 档在预览里完全不动**，
          //    而成片是底座真的逐字画的。这正是用户报的"播放没有效果"。
          //    消费方在 `PolishPreview.vue`（逐字拆 `<span>`，经 `perCharSpans`）。
          //
          // ⚠️⚠️ **逐字 spec 要试两轴**（2026-10-14，与转译器 `perCharFrom` 同一个修）。
          //    三轴收敛后「逐字变化」住在**动效**轴（`perchar_big`/`perchar_small`），
          //    而它们的 `perChar` 仍是**形态字段** ⇒ 只认排版轴的话，
          //    用户选「逐字变大」在**元素上**一条都不动
          //    （本仓"两条路径只修一条"的第 N 例：字幕那条路修过，元素这条漏了）。
          //    ⚠️ 顺序 = 优先级：排版轴（`arc` 的逐字）优先，与转译器同序。
          entry.perChar = animPerChar(styleAnimName) ?? animPerChar(effectAnimName)
            ?? animPerChar(transitionAnimName)
          // ⚠ 逐字那两个数（`perCharFrame` / `perCharFps`）**不在这里给**：
          //   它们在**求值期**算（下面 `getElements` 的 push 里，
          //   `local * idx.fps` / `idx.fps`）—— `buildIndex` 是**一次性**索引，
          //   那时**没有"当前时间"这个量**（写 `t` 直接 `ReferenceError`，实测）。
          //   ⚠ 这里曾经又写了一遍 `entry.perCharFps` —— **死代码**：
          //     消费方取的是 `idx.fps`（下面 `idx.fps = …` 那一行），从来不读 entry 上这个。
          //     实测拓它改成 0 判据**一项都不红** —— 就是这个原因。
        }
      }
    })
  })
  ;INDEXED_TYPES.forEach((k) => idx[k].sort((a, b) => a.time - b.time))

  // 字幕轨的眼睛：只清**索引里的那份**（素材与 `data/subtitles.json` 一个字都没动）
  idx.subs = laneOn('subtitles') ? (subtitles || []) : []
  // ---- §5 T5 真卡拉OK：把逐字时刻表按**字幕起点**索引（预览腿）----
  //
  // ⚠️ 开着的工程才吃它：`cfg.elements.karaoke` 为假 ⇒ `idx.karaoke` 是空 Map ⇒
  //    `subsAt` 一定走整条那条路 ⇒ **存量工程逐像素不变**（R4）。
  //
  // ⚠️ 按**起点毫秒**索引而不是数组下标：结论文件里只有"对得上"的那些条
  //    （对不上的被质量闸退了），下标与 `subtitles` 对不上 —— 与转译器
  //    `polish-to-dhampir.mjs` 的 `byStart` 同一口径（那边也这么写，注释同一条理由）。
  idx.karaoke = new Map()
  if (cfg.elements && cfg.elements.karaoke === true && Array.isArray(karaokeCues)) {
    for (const c of karaokeCues) {
      if (!c || !Array.isArray(c.times) || !Array.isArray(c.chars)) continue
      // 校验"字与时间一一对应"——形不对就当没有（宁可整条，也不要画半个字）
      if (c.chars.length !== c.times.length) continue
      const key = Math.round(Number(c.start) * 1000)
      if (Number.isFinite(key)) idx.karaoke.set(key, c.times)
    }
  }
  // ---- 弹幕时间偏移（[style.danmaku] offset_secs，默认 -3 = 整体提前 3s）----
  // 只平移显示时刻（start/end）；y 泳道与 travel 与时间无关，不用动。
  // 与 templates/index.html 的 DM 注入点同口径：那里在注入时平移一次，
  // 这里在 buildIndex 平移一次 —— GSAP / canvas / getActiveDms 全吃这一份。
  // ⚠️ 必须拷贝、不能写回调用方的数组：buildIndex 会被响应式地重复求值，
  // 写回去等于每次重算再叠一次偏移（模板那份只 load 一次，没有这个问题）。
  const dmOff = (cfg.style && cfg.style.danmaku && isFinite(cfg.style.danmaku.offset_secs)) ? cfg.style.danmaku.offset_secs : -3
  // 弹幕轨的眼睛同理：关掉就是"这一帧没有弹幕"，素材与 `offset_secs` 都不动。
  idx.dms = !laneOn('danmaku')
    ? []
    : (danmaku && dmOff !== 0)
      ? danmaku.map((d) => ({ start: d.start + dmOff, end: d.end + dmOff, text: d.text, color: d.color, y: d.y, travel_duration: d.travel_duration }))
      : (danmaku || [])
  idx.dmWidths = measureDanmakuWidths(idx.dms, dmFontSize(cfg), dmFontFamily(cfg))
  idx.dur = dur
  idx.layout = cfg.layout || {}
  idx.subStyle = (cfg.style && cfg.style.subtitle) || {}
  idx.dmStyle = (cfg.style && cfg.style.danmaku) || {}
  idx.camStyle = (cfg.style && cfg.style.camera) || {}
  idx.vertical = !!cfg.vertical
  // 运镜门：**运镜轨的眼睛关着**（`[stage.lanes] camera`，唯一判据在 Rust
  // `ProjectFile::lane_enabled`，2026-10-06 从 `portrait_no_camera` 硬门改成开关）。
  // ⚠️ 与上面 buildIndex 的整类过滤是**同一枚开关**的两道消费点：事件不进索引 + 状态机不进运镜
  //（`cameraAt` 的早退与空列表结果逐值相同，留着是为了与模板 `getActiveCamera` 同形对照）。
  idx.cameraOff = !laneOn('camera')
  // 画面构成契约（Rust resolve_stage 唯一推导）：贴纸落点模式 / 字幕安全宽 / 画布尺寸都从这里取
  idx.canvas = cfg._canvas || null
  // 画布参数（由 `_canvas` 推导）：百分比 → 像素、运镜不动点、贴纸几何都用它
  idx.cv = canvasOf(idx.canvas)
  // §5 T7 摇摆（P1，2026-10-13）：工程网格帧率（由调用方从 `timeline.timebase` 传来）。
  // ⚠️ **缺省不给 30**：`swayDegAt` 靠 `fps > 0` 决定摆不摆，而"猜一个帧率"
  //    会让预览与成片**用不同的 quarter**（摆的节奏对不上，且两边都不报错）。
  //    宁可 `0` ⇒ 不摆（＝旧行为，R4），也不猜。判据见 `check-sway-parity.mjs`。
  idx.fps = Number.isFinite(fps) && fps > 0 ? fps : 0
  idx.soundEvents = soundEvents
  idx.ptEvents = ptEvents
  idx.segs = segs
  return idx
}

function sfxVolume(cfg) { return (cfg.sfx && cfg.sfx.volume) || 0.1 }
function dmFontSize(cfg) { return (cfg.style && cfg.style.danmaku && cfg.style.danmaku.font_size) || 42 }
/** 弹幕字体族（`[style.danmaku] font`，字体库 id）。
 *
 *  ⚠️ **空 = 出厂默认文字字体**（`Noto Sans SC`，与 Rust `default_font()` 同名），
 *  **不是**"跟随字幕那条字体栈"（2026-10 用户拍板改的口径，见 `DanmakuStyle::font`）。
 *  旧口径"空 = 跟随字幕"有三个问题：① 字幕那栏存的是**族名**、这一栏存的是**库 id**，格式不同；
 *  ② 于是弹幕看起来和字幕一模一样，而用户预期是"默认那支"；③ **真出片通道（底座）从不实现这一档**
 *  （转译器填 `'Sans'`）⇒ 预览与成片不一致且不报错。
 *  ⚠️ 量宽与绘制必须都走它：两处各写一份就会"量宽的字体 ≠ 画出来的字体"，
 *  弹幕没滚到边就消失（与 `templates/index.html::dmFontFamily` 同口径）。 */
function dmFontFamily(cfg) {
  const own = (cfg.style && cfg.style.danmaku && cfg.style.danmaku.font) || ''
  const fam = own || 'Noto Sans SC'
  return `"${fam}","Microsoft YaHei","Noto Sans SC",sans-serif`
}

function safeEmoji(name) {
  const code = parseInt(name, 16)
  return Number.isNaN(code) ? '✨' : String.fromCodePoint(code)
}

/** §5 E4a 文字元素动效的**工程级**参数（`[elements]` 段）。
 *
 *  ⚠️ **它与事件无关**：T1/T2/T4/T6 都是"整个工程开不开、参数多少"，
 *  与出片侧读的是**同一个段**（`render/elements.rs::ElementsConfig`）。
 *  ⇒ 预览这里也读工程级配置，**不往每条事件上加字段** —— 那会变成第二个真源。
 *
 *  ⚠️ **`None` 的落点必须与 Rust 逐值一致**（每一条都在 `elements.rs` 有出处）：
 *   * `scaleInMs` 缺省 0 ⇒ **关**。⚠️ 三个 T1 字段里**只有它决定开不开** ——
 *     `from`/`peak` 都给了但它是 0 就不缩（"窗口多长"才定义得了"缩到什么程度"）。
 *   * `shakePx` 缺省 0 ⇒ 关；`shakeHz` 缺省 **18.0**（`DEFAULT_SHAKE_HZ`）。
 *   * `plateColor` 缺省 ⇒ **不铺底衬**（一个像素都不多画）。
 *     ⚠️ 两个 T6 字段是**单向**的：没颜色时 `padPx` **读都不会被读到**。
 *   * `typewriter` 缺省 false。
 */
/** **39 档动效的预设表**（预览侧，2026-10-13，第 3 步 A+B 组）。
 *
 *  ⚠️⚠️ **权威在 `tools/polish-to-dhampir.mjs::ANIM_PRESETS`**（它才产出底座字段）。
 *  这里是**第三份实现**（转译器 / 界面下拉 / 预览），三份必须同值 ——
 *  漂了的症状是本仓最恨的那类：**预览里不动、成片里动**（或反过来），
 *  **两边各自看都正常**。判据：`scripts/check-anim-presets-parity.mjs`。
 *
 *  # 优先级（与转译器**同一条**）
 *
 *  **逐条 `anim` > 全局 `[elements]` > 内置缺省**。
 *  ⇒ 某条写了 `anim` ⇒ **它没提的字段一律按"关"**（不回落全局 —— 否则"砸入"会顺带
 *    继承全局的抖动，两种动效叠在一起，而用户以为自己只选了一档）；
 *    没写 `anim` ⇒ 整条走全局那份（老行为，R4）。
 *
 *  ⚠️ `fade_only: {}`（空对象）是**合法且有意义**的：它表示"这一档什么都不加"，
 *  用来**压掉**全局那份弹入。所以"有没有写 anim"只能看**取没取到这一档**
 *  （`undefined` 才是没写），不能看 `Object.keys(preset).length > 0`。
 */
export const ANIM_PRESETS = {
  // ── 排版轴（4 档，含"默认"= 空档名）──────────────────────────────
  //  ⚠️ **角度不在这里** —— 它走事件字段 `layout_angle_deg`（见 `layoutAngleOf`）。
  //     这里的 `tiltDeg` 是**没填角度时**的回落值。
  tilt: { tiltDeg: -6.0 },
  //  ⚠️ `arcToDefault` 是"档位自己的方向"（负 = 向上弯）。用户填角度时取绝对值
  //     当幅度、**保留这个方向** —— 否则"我填 14"会突然变成反向弯。
  arc: {
    arcToDefault: -14,
    perChar: { rotationFrom: 0, rotationTo: -14, perCharFrames: 8, staggerFrames: 3 }
  },
  //  竖排是**轨级排版方向**，不是逐字变换。
  //  ⚠️⚠️ 列容量 = 安全宽 ÷ 字号比例 —— `safeWidthRatio: 0.86` 不能省（见转译器注释）。
  //  ⚠️ 原来 `vtext_thin` / `vtext_bold` 两档曾被实测抓出"产物逐字节相同"，
  //     收敛后只剩一档，那一类事故不可能再发生。
  //  ⚠️ `fontFamily` / `fontWeight` 在预览侧只是**兜底展示**（真值走 source 串）。
  vtext: {
    vertical: true,
    safeWidthRatio: 0.86,
    fontWeight: 400
  },

  // ── 过渡轴（2 档）入场/出场效果 ─────────────────────────────────
  //  ⚠️ 与原本的 `pop_std` 同值（用户口径：「默认为弹出」）。
  pop: { scaleInFrom: 0.3, scaleInPeak: 1.0, scaleInMs: 350 },
  //  ⚠️⚠️ **空对象是合法且有意义**的：它表示"这一档什么都不加"（淡入淡出走
  //     底座自己的 opacity 包络）——用来**压掉**全局 `[elements]` 那份弹入。
  //     ⇒ "有没有写 anim"只能看**取没取到这一档**（`undefined` 才是没写），
  //       不能看 `Object.keys(preset).length > 0`。判据钉着这一条。
  fade: {},

  // ── 动效轴（4 档）在场时怎么动 ──────────────────────────────────
  shake: { shakePx: 6.0, shakeHz: 18.0 },
  //  ⚠️ 用 `scaleSpread`（**按序号的字号乘数**），**不是** `scaleTo`
  //     —— 后者是"同一个值给每个字" = **整行**缩放，字间没有差别。
  perchar_big: {
    perChar: { scaleSpread: 0.8, perCharFrames: 18, staggerFrames: 2 }
  },
  perchar_small: {
    perChar: { scaleSpread: -0.55, perCharFrames: 18, staggerFrames: 2 }
  },
  //  ⚠️⚠️ **心跳的双峰脉冲在底座侧还没实现**（现有原语只有单次/正弦）。
  //     预览这里也只能做到"一次较缓的脉冲" —— 与成片**一致地**不完整，
  //     好过预览与成片不一致。底座能力到位后两处一起补。
  heartbeat: {
    // ⚠⚠ 曾经这里是单峰占位（`scaleInFrom/Peak/Ms`），现在换成真的双峰参数。
    //    两档**不同名**：旧三个是 T1 弹入的字段，混用会让"心跳"退化成一次弹入
    //    —— 而那正是本档之前的占位行为（用户要的是"扑通扑通"）。
    // ⚠ 与转译器 `ANIM_PRESETS.heartbeat` 同名同值（驼峰 vs 蛇形那层）。
    //
    // ⚠⚠ **占空比是这一档成败的关键**（2026-10-14 用户口径：「心跳的效果不对，没有脉动感」）。
    //    旧值 `[0,180,300,480]` / `1000` 的实测占空比：
    //      0–180 第 1 下（18%）· 180–300 停（12%）· 300–480 第 2 下（18%）· **480–1000 停（52%）**
    //    ⇒ 一半以上时间是死的，读起来是「咚·咚…………」而不是「扑通·扑通·扑通」。
    //    ⚠ 那时判据是**绿的**（双峰个数、第二峰更弱、停顿恒 1.0 全都量到了）——
    //      又一个"判据绿 ≠ 用户看得见"的实例：**人眼感知的是节律，不是极值个数**。
    //    新值 `[0,210,320,530]` / `900`：停顿 520→370ms、两下各宽 30ms。
    heartbeatPeaksMs: [0, 210, 320, 530],
    //    ⚠ `amp` 1.14 → **1.22**（2026-10-14）：与成片那份 `scale_in_peak` 同值。
    //      1.14 时峰值 `1+0.14×1.25 = 1.1750` 与 `pop` 的 `0.3+0.7×1.25 = 1.1750`
    //      **逐位相同** ⇒ 像素判据量到两档同图（`check-39-tiers-pixels` ③ 红）。
    //      两者必须能分辨 —— 而且心跳**本来就该比入场弹一下更明显**。
    heartbeatAmp: 1.22,
    heartbeatCycleMs: 900
  }
}

/** 一档动效在**哪条轴**上（2026-10-14 三轴；`''` = 认不出/空）。
 *
 *  ⚠️⚠️ **精确名 + 前缀两条都要判** —— 与转译器 `animAxisOf` 同一形状。
 *     收敛后档名是族的名字本身：`tilt` / `fade` / `shake` / **`heartbeat`**
 *     这些**不带 `_`**，而 `perchar_big` / `perchar_small` 带。
 *     只判前缀 ⇒ `heartbeat` 认不出来 ⇒ 用户选了心跳**预览里不动**且无报错。
 */
const ANIM_AXIS = {
  tilt: 'style', arc: 'style', vtext: 'style',
  pop: 'transition', fade: 'transition',
  shake: 'effect', perchar: 'effect', heartbeat: 'effect'
}
const ANIM_AXIS_PREFIXES = Object.keys(ANIM_AXIS).sort((a, b) => b.length - a.length)

export function animAxisOf(name) {
  const key = String(name ?? '').trim().toLowerCase()
  if (!key) return ''
  // ① 精确名（`heartbeat` / `fade` / `tilt` / `tilt` …）
  if (Object.prototype.hasOwnProperty.call(ANIM_AXIS, key)) return ANIM_AXIS[key]
  // ② 前缀（`perchar_big` → `perchar`）。长的排前面，免得短前缀抢走长前缀的档。
  for (const p of ANIM_AXIS_PREFIXES) {
    if (key.startsWith(p + '_')) return ANIM_AXIS[p]
  }
  return ''
}

/** **排版角度**：用户填了就用它，没填回落档位自己的缺省（**度**）。
 *
 *  ⚠️⚠️ **`0` 是合法值** ⇒ 判"填没填"必须用 `!== undefined` / `!== null`，
 *     **不是** `||`。写成 `raw || def` 的症状是"我把角度调成 0，它却回到了 -6"。
 *  ⚠️ 区间夹取**不在这里**（后端是权威，转译器夹）。预览只如实显示用户的值，
 *     免得"预览显示 45、成片 30"这种两端不一致 —— 预览该跟**成片**走。
 */
export function layoutAngleOf(name, rawAngle) {
  if (rawAngle !== null && rawAngle !== undefined && Number.isFinite(Number(rawAngle))) {
    return Number(rawAngle)
  }
  const p = animPreset(name)
  if (!p) return 0
  if (p.tiltDeg !== undefined) return p.tiltDeg
  if (p.arcToDefault !== undefined) return p.arcToDefault
  return 0
}

/** **排版角度的允许区间** —— 与转译器 `LAYOUT_ANGLE_MIN/MAX` 及面板同值。
 *
 *  ⚠️ 三处（转译器 / 面板 / 这里）必须逐值一致，由
 *     `scripts/check-anim-layout-angle.mjs` 钉着。
 *     预览侧用它做滑杆/越界提示的显示基准，**但夹取权威在后端**。
 */
export const LAYOUT_ANGLE_MIN = -30
export const LAYOUT_ANGLE_MAX = 30

/** 一档动效 → 那组参数。`undefined` = **这一档不存在**（没写 / 认不出）。 */
export function animPreset(name) {
  const key = String(name ?? '').trim().toLowerCase()
  if (!key) return undefined
  return Object.prototype.hasOwnProperty.call(ANIM_PRESETS, key) ? ANIM_PRESETS[key] : undefined
}

// ============================================================================
// 三轴归属（排版 × 过渡 × 动效）—— 与转译器 `tools/polish-to-dhampir.mjs` **同源**
// ============================================================================
//
// ⚠️⚠️ 2026-10-14 从**两轴**收敛成**三轴**（用户口径）：
//    形态改名**排版**、`pop`/`fade` **独立成过渡轴**、动效只留抖动/逐字变化/心跳。
//
// ⚠️⚠️ **"老工程只有一个 `anim`、要拆到两路"那整套已删。**
//    用户明确「当前没有存量工程还在开发中不要考虑」⇒ 不做别名迁移。
//    原来那套（`animHasFormPart` / `animHasMotionPart` / `splitAnimTwoAxis`）
//    连同它的 `slam_squash` / `pop_perchar` / `hop_*` 特例一起**去掉了** ——
//    那些档名现在**一个都不存在**，留着就是永远为假的死分支。
//
// ⚠️ 三轴**字段不重叠**（这就是"可叠加"的根据）：一条档位只属于一条轴，
//    由 `ANIM_PRESETS` 的档名 + 上面的 `ANIM_AXIS` 共同表达。

/** 这一档在**排版（形态）**那路上给了东西吗。
 *
 *  ⚠️⚠️ **判据是"这一档的预设里有没有形态字段"，不是"它归哪条轴"** —— 2026-10-14 改成结构性判据。
 *
 *    原来是**按档名逐族硬编码**（`animAxisOf(k)==='style' || k==='slam_squash'
 *    || k==='pop_perchar' || /^hop_/`）。三轴收敛后那个白名单里的档名**全删了**，
 *    而 `perchar_big` / `perchar_small` **没被认成形态档** ⇒ 用户选「逐字变大」，
 *    预览里**字一动不动**（成片那边同一个坑也踩过一次）。
 *
 *    ⇒ 现在直接问预设表：有 `perChar` / `tiltDeg` / `vertical` / `squashY` ⇒ 要给形态侧。
 *      ⚠️ 以后新增任何带 `perChar` 的档都**自动被认到**，不会再出现
 *         "加了档、忘了改白名单 ⇒ 静默不生效"。
 *      ⚠️ 与转译器 `animHasFormPart` **同一形状**，别只改一边。 */
export function animHasFormPart(name) {
  const p = animPreset(name)
  if (!p) return false
  return p.perChar !== undefined
    || p.tiltDeg !== undefined
    || p.vertical === true
    || p.squashY !== undefined
}

/** 这一档在**动效**那路上给了东西吗。
 *
 *  ⚠️ 判据是"它在动效轴上"，或者它的预设给了**时间类字段**。
 *  ⚠️ 与 `animHasFormPart` 不同：`fade` 一个字段都不给，但它**有动效语义**
 *     （"这一档不加动效"要能**压掉**全局那份）⇒ 它必须回 `true`。
 *     所以这里**不能**只看"预设里有没有键"。 */
export function animHasMotionPart(name) {
  const a = animAxisOf(name)
  if (a === 'effect') return true
  // ⚠️ 排版轴里也有给时间的（`arc` 逐字转）—— 它**两边都要给**，
  //    否则老工程的"逐字动"会静默变成"静态阶梯"。
  //
  //    ⚠️ 结构性判据（问预设表），**不是档名前缀** —— 收敛后的档名是 `arc`（无下划线），
  //    而前缀正则要求下划线 ⇒ 匹配失败 ⇒ 静默丢档（转译器侧同一个坑，两处一起改）。
  //
  //    ⚠️⚠️ **必须只认 `perChar`** —— 不能顺手把 `scaleInFrom` / `shakePx` 也收进来。
  //    实测（权威 = 转译器）：
  //      · `pop` 的 `scaleInFrom` 是**过渡**语义、`fade` 一个字段都不给
  //        ⇒ 收进来会让 `pop` **同时**落进 `effect` 与 `transition` 两路
  //          （`splitLegacyAnim('pop')` 从 `{effect:''}` 变成 `{effect:'pop'}`）
  //          ⇒ 与转译器**不同源** ⇒ 老格式在预览与成片里走不同的路；
  //      · `shake`/`heartbeat` 已经在上面那条 `a === 'effect'` 里返回了，写在这里是死代码。
  //    ⇒ 只保留 `perChar`（这正是"排版轴档位也给时间"的唯一实际情形）。
  const p = animPreset(name)
  if (!p) return false
  return p.perChar !== undefined
}

/** 把老工程的**单个** `anim` 拆成**三路**档名（认不出 ⇒ 三路都空）。
 *
 *  ⚠️⚠️ **第三路（过渡）是 2026-10-14 三轴收敛时补的。**
 *    收敛把 `pop` / `fade` 的轴改成了 `'transition'`，而 `animHasFormPart('pop')`
 *    与 `animHasMotionPart('pop')` **都是 false**（它们的字段既不是形态、也不是时间）
 *    ⇒ 只吐两路的话，`pop` / `fade` 被拆成 `{style:'', effect:''}` ⇒ **三路全空** ⇒
 *    ① 轨 id 退化成 `text_<style>_<pos>`（`fade` 与 `pop` **撞同一个 id**，后面那条被吃掉）；
 *    ② `on === false` ⇒ **静默回落全局** `[elements]` ⇒ 用户选「弹出」拿到
 *       "全局弹入 + 全局抖动 + 全局摇摆"；③ 排序键跟着乱。
 *
 *    ⚠️ 与转译器 `splitLegacyAnim` **逐条同源**，改一边必须改另一边
 *      （漂了的表现是"预览与成片不一致"，两边各自看都正常）。 */
export function splitAnimTwoAxis(raw) {
  const key = String(raw ?? '').trim().toLowerCase()
  if (!key || !animPreset(key)) return { style: '', effect: '', transition: '' }
  return {
    style: animHasFormPart(key) ? key : '',
    // ⚠️ 过渡档**必须传下去**（它们自己虽然"不给形态"，但 `pop` 给 `scale_in_*`）。
    effect: animHasMotionPart(key) ? key : '',
    transition: animAxisOf(key) === 'transition' ? key : ''
  }
}

/** 把一档预设的**逐字 spec** 取出来（没有就 `undefined`）。
 *
 *  ⚠️ 与 `animMotion` 是**两条腿**，不是一回事：
 *    * `animMotion` 摊的是**整块**那条时间函数（弹入/抖动/摇摆/底衬/打字机）；
 *    * 本条摊的是**逐字**那条（`charTransformAt` 用的 `PerCharSpec`）。
 *    档位可以**只**有其中一条（`tilt_*` 只有整块、`arc_*` 只有逐字），
 *    也可以两条都有（`hop_*`：整块弹入 + 逐字错峰）。
 *
 *  ⚠️⚠️ **必须补齐缺省值**（2026-10-13 实测抓到的 bug）：
 *    `ANIM_PRESETS` 里那些 spec 是**只写它用到的几个键**的（例如 `arc_up` 只有
 *    `rotationFrom/rotationTo/spreadXEm/perCharFrames/staggerFrames`，**没有** `scaleFrom`）。
 *    而 `charTransformAt` 按"字段一定在"写的（`mix(spec.scaleFrom, spec.scaleTo)`）
 *    ⇒ 缺键时算出 **`NaN`** ⇒ CSS 得到 `scale(NaN)`，**整条 transform 被浏览器丢弃**
 *    ⇒ 症状是"这一档的字**完全不动**"，而且**不报任何错**。
 *    `readPerChar`（读 toml 那条路）本来就补缺省，所以走 toml 时看不出问题 ——
 *    只有"直接用预设表"这条路会踩。⇒ 这里统一用 `PER_CHAR_DEFAULT` 补齐，
 *    两条路的形状从此**同一份**。 */
export function animPerChar(animName) {
  const p = animPreset(animName)
  if (!p || !p.perChar) return undefined
  // ⚠️ 与 `readPerChar` 的缺省同源（那份常量就是"缺键时该是什么"的唯一声明）。
  return { ...PER_CHAR_DEFAULT, ...p.perChar }
}

/** **逐字渲染装配**：把一条文字按字拆开、每个字给出它自己的 CSS `transform`。
 *
 *  这是 `charTransformAt` 的**唯一消费点**（预览与判据都走这里，不各写一份 ——
 *  本仓"两份实现必然漂移"的老病）。
 *
 *  # 为什么会需要它（2026-10-13）
 *
 *  ⚠️⚠️ `charTransformAt` 之前**导出了、但全仓没有一处 import** ⇒
 *  C 组逐字变换（15 档）+ D 组逐字错峰（4 档）**共 19 档在预览里完全不动**，
 *  而成片是底座真的一字一字画的。用户报的「播放没有效果」就是这件事。
 *
 *  # 口径（三处必须一致）
 *
 *  * `em` → `px`：**用这一条自己的字号**（`fontPx`），与底座
 *    `text_perchar.rs` 把 `offset_*_em` 乘上字号同口径。
 *  * 旋转绕**字自己的中心**（调用方给 `transform-origin:center center`），
 *    与底座 `blit_affine` 的 `c_src = 位图中心` 同口径。
 *  * ⚠️ 没逐字档 / fps 为 0 / 只有一个字 ⇒ 返回 `null`（调用方照原样整块渲染）——
 *    **不是**返回一堆恒等 transform：那会白白把整行拆成几十个 `<span>`
 *    而画面一点变化都没有（还多一份排版风险）。
 *
 *  @returns {null | Array<{text, style}>} `null` = 不用逐字；否则每字一项，
 *    `style` 是可直接绑到 `<span>` 上的 CSS 键值对象（**空对象 = 这一字不动**）。 */
export function perCharSpans(text, { perChar, perCharFrame, perCharFps, fontPx } = {}) {
  const s = String(text ?? '')
  // 没档、没帧率、或只有一个字 —— 三种情况都没有"逐字"可言。
  if (!perCharActive(perChar)) return null
  if (!(perCharFps > 0)) return null
  if (s.length <= 1) return null
  const chars = Array.from(s)
  const n = chars.length
  const em = Number.isFinite(fontPx) && fontPx > 0 ? fontPx : 0
  return chars.map((ch, i) => {
    const t = charTransformAt(perChar, i, n, perCharFrame)
    const parts = []
    // ⚠️ 顺序与 `.text-el` 上那串**必须同形**：translate → rotate → scale。
    //    反过来（scale 在前）会沿**旋转后的轴**缩放 ——
    //    正是 `check-text-tilt.mjs` 记下的那个"斜着的字被拉长"的坑。
    const tx = t.offsetXEm * em
    const ty = t.offsetYEm * em
    // ⚠️⚠️ **三个分量各自过一道 `Number.isFinite`**（2026-10-14 加）。
    //
    //    `charTransformAt` 的数学里有一项是
    //      `scale: mix(spec.scaleFrom, spec.scaleTo) * (1 + spec.scaleSpread * rank)`
    //    —— 只要 `spec` 少了那几个键，`mix(undefined, undefined)` 就是 **`NaN`**。
    //    ⇒ 整条 `transform` 变成 `rotate(-14deg) scale(NaN)`。
    //
    //    ⚠️ **浏览器对非法 `transform` 是整条丢弃**（不是忽略那一项）：
    //      症状是"逐字档配了，预览里字**完全不动**"，而成片那边 Rust 有
    //      `#[serde(default = "one")]` 兜底、**正常在动** ⇒ 两边分叉且不报错。
    //
    //    ⚠️ 生产路径**当前**不会踩到（`animPerChar` 会先铺 `PER_CHAR_DEFAULT`）——
    //      但那是**调用约定**，不是这道函数的契约：
    //      `perCharSpans` 是 `export` 的，任何新消费者直接喂预设里的那个
    //      `perChar` 就会中招（我自己的探针就是这么踩到的）。
    //      ⇒ 把"不许产出 NaN"写成函数自己的不变式，而不是靠调用方记得。
    //      ⚠️ 判据 `check-perchar-spans-finite.mjs` 钉住它（含变异自证）。
    const okNum = (v) => Number.isFinite(v)
    if (okNum(tx) && okNum(ty) && (tx || ty)) parts.push('translate(' + tx + 'px,' + ty + 'px)')
    if (okNum(t.rotationDeg) && t.rotationDeg) parts.push('rotate(' + t.rotationDeg + 'deg)')
    if (okNum(t.scale) && t.scale !== 1) parts.push('scale(' + t.scale + ')')
    // ⚠️ 空 `style` 也要给（调用方统一 `:style="c.style"`），不要给 `undefined`。
    return { text: ch, style: parts.length ? { transform: parts.join(' '), display: 'inline-block' } : {} }
  })
}

/** 把一档预设**摊到**全局那份动效配置上（优先级：逐条 > 全局）。
 *
 *  ⚠️ **整档 `undefined` 就整份回落全局**（不是"逐字段回落"）。
 *  ⚠️ 空预设 `{}`（`fade_only`）**照样走"生效"那条路** ⇒ 全局那份被清成"什么都不加"。
 *  这是它与"没写 `anim`"的**唯一区别**，也是"只淡出"那条语义的落点。 */
export function animMotion(cfg, animName, transitionName) {
  const base = textMotionCfg(cfg)
  // ⚠️ **两轴都试**（2026-10-14 三轴收敛）：抖动住在**动效**轴、弹出/淡入住在**过渡**轴，
  //    而它们摊的是**同一组** `scale_in_*` / `shake_*` 字段。
  //    ⚠️ 只读一轴的症状：用户在**过渡**栏选「弹出」⇒ 预览里不弹（成片里弹）。
  const pEff = animPreset(animName)
  const pTrans = animPreset(transitionName)
  // 两条轴都认不出 ⇒ 回落全局那份。
  if (pEff === undefined && pTrans === undefined) return base
  // 逐条那一档生效 ⇒ 弹入/抖动/摇摆**全部重置成"关"**，再叠加预设里写了的。
  // ⚠️⚠️ **两条轴各自摊自己的字段、不许互相覆盖**（这就是"可叠加"）：
  //    先重置成全关（压掉全局），再**逐个轴**合并。
  //    直接 `{...base, reset, ...pTrans, ...pEff}` 不行 —— 后摊的那一档会把
  //    前一档的字段整个盖掉 ⇒ 用户选「抖动 + 弹出」只拿到一半，
  //    而界面上两栏看着都对（本仓"两轴互相吃掉"的老症状）。
  const off = {
    scaleInFrom: 1.0,
    scaleInPeak: 1.0,
    scaleInMs: 0,
    // ⚠ 心跳也要重置：不然"选了别的档"会**继承全局那份心跳**，
    //   两种动效叠在一起而用户以为只选了一档。
    heartbeatPeaksMs: null,
    heartbeatAmp: 1.0,
    heartbeatCycleMs: 0,
    shakePx: 0,
    shakeHz: 18.0,
    swayDeg: 0,
    swayMs: 0
  }
  // ⚠️ 先过渡、后动效 ⇒ 同名字段**动效优先**（与转译器 `pick()` 的优先级一致）。
  const merged = { ...base, ...off, ...(pTrans ?? {}), ...(pEff ?? {}) }
  // ⚠️ **逐字那两个键不参与这条腿**（`perChar` 由 `animPerChar` 单独取、
  //    `tiltDeg`/`arcToDefault` 由 `textTiltTransform` 取）⇒ 从摊出来的结果里摘掉，
  //    免得消费者以为"摊进 textMotion 就生效了"。
  delete merged.perChar
  delete merged.tiltDeg
  delete merged.arcToDefault
  return merged
}

// =====================================================================
// 逐字（P3/P4，2026-10-13）
// =====================================================================
//
// ⚠️⚠️ **本段是 Dhampir 两份 Rust 实现的移植**，不是"再设计一遍"：
//
//   `crates/dhampir-timeline/src/text_perchar.rs`      ← 几何（这份下面两段）
//   `crates/dhampir-timeline/src/text_perchar_anim.rs`  ← 时间（`charTransformAt`）
//
// **权威在 Rust 那两份**（成片是它们画的）。本段只需逐值对齐 ——
// 对齐由 `scripts/check-perchar-parity.mjs` 钉住：那条判据**不实现算法**，
// 而是跑底座真机拿 `cells`、再跑这里的函数、**逐字逐字段比**。
//
// 漂移的症状是本仓最恨的那一类：**预览里字散开了、成片里没散**（或反过来），
// 而**两边各自看都正常**。本仓已栽过四次同形状的坑。

/** 逐字档位的**缺省值**（与 Rust `PerCharSpec::default()` 逐字段同值）。
 *
 *  ⚠️ 两边缺省值必须**同一份**：`Option` 字段少写一个键时，两端算出不同的字
 *  —— 而那种"少了半档"的症状是"预览里动、成片里不动一半"。
 *  ⚠️ 缩放的缺省是 `1.0`**不是** `0.0`（Rust 那边专门有个 `one()` 函数就为这个）：
 *  写成 0 会让字**缩成不可见**，而 0 是合法值 ⇒ 不报错。 */
export const PER_CHAR_DEFAULT = {
  name: null,
  scaleFrom: 1.0,
  scaleTo: 1.0,
  rotationFrom: 0.0,
  rotationTo: 0.0,
  offsetXFrom: 0.0,
  offsetXTo: 0.0,
  offsetYFrom: 0.0,
  offsetYTo: 0.0,
  perCharFrames: 0,
  staggerFrames: 0,
  easing: 'linear',
  reverse: false,
  spreadXEm: 0.0,
  spreadYEm: 0.0,
  spreadXFrom: 0.0,
  spreadXTo: 0.0,
  spreadYFrom: 0.0,
  spreadYTo: 0.0,
  scaleSpread: 0.0
}

/** 把工程里那份 `per_char`（**snake_case**，与 toml/JSON 同形）读成上面的驼峰形状。 */
export function readPerChar(raw) {
  if (!raw || typeof raw !== 'object') return null
  const n = (v, d) => (typeof v === 'number' && Number.isFinite(v) ? v : d)
  const d = PER_CHAR_DEFAULT
  return {
    name: typeof raw.name === 'string' ? raw.name : d.name,
    scaleFrom: n(raw.scale_from, d.scaleFrom),
    scaleTo: n(raw.scale_to, d.scaleTo),
    rotationFrom: n(raw.rotation_from, d.rotationFrom),
    rotationTo: n(raw.rotation_to, d.rotationTo),
    offsetXFrom: n(raw.offset_x_from, d.offsetXFrom),
    offsetXTo: n(raw.offset_x_to, d.offsetXTo),
    offsetYFrom: n(raw.offset_y_from, d.offsetYFrom),
    offsetYTo: n(raw.offset_y_to, d.offsetYTo),
    perCharFrames: n(raw.per_char_frames, d.perCharFrames),
    staggerFrames: n(raw.stagger_frames, d.staggerFrames),
    easing: typeof raw.easing === 'string' ? raw.easing : d.easing,
    reverse: raw.reverse === true,
    spreadXEm: n(raw.spread_x_em, d.spreadXEm),
    spreadYEm: n(raw.spread_y_em, d.spreadYEm),
    spreadXFrom: n(raw.spread_x_from, d.spreadXFrom),
    spreadXTo: n(raw.spread_x_to, d.spreadXTo),
    spreadYFrom: n(raw.spread_y_from, d.spreadYFrom),
    spreadYTo: n(raw.spread_y_to, d.spreadYTo),
    // ⚠️ 按序号放大的**字号乘数**（C 组 11 档靠它）—— 见 Rust 侧同名字段的文档。
    scaleSpread: n(raw.scale_spread, d.scaleSpread)
  }
}

/** **全角 / 半角**判定 —— 逐字照抄 Rust `text_layout.rs::is_full_width`。
 *
 *  ⚠️ 覆盖范围是**按 Unicode 区块列的**，不是"看起来宽不宽"猜的：
 *  猜的话会漏掉假名、谚文、全角标点，而那些恰恰在中日韩字幕里天天出现。
 *  ⚠️ 区块表**必须与 Rust 那份逐段一致** —— 少一段的症状是"某几个字的位置
 *  从那里开始整体偏"，而**行宽看着还对**（因为偏差是局部的）。 */
function isFullWidth(ch) {
  const code = ch.codePointAt(0)
  return (
    (code >= 0x1100 && code <= 0x115f) ||
    (code >= 0x2e80 && code <= 0x303e) ||
    (code >= 0x3041 && code <= 0x33ff) ||
    (code >= 0x3400 && code <= 0x4dbf) ||
    (code >= 0x4e00 && code <= 0x9fff) ||
    (code >= 0xa000 && code <= 0xa4cf) ||
    (code >= 0xac00 && code <= 0xd7a3) ||
    (code >= 0xf900 && code <= 0xfaff) ||
    (code >= 0xfe30 && code <= 0xfe4f) ||
    (code >= 0xff00 && code <= 0xff60) ||
    (code >= 0xffe0 && code <= 0xffe6) ||
    (code >= 0x1f300 && code <= 0x1f64f) ||
    (code >= 0x20000 && code <= 0x3fffd)
  )
}

/** **零宽**判定 —— 逐字照抄 Rust `text_layout.rs::is_zero_width`。 */
function isZeroWidth(ch) {
  const code = ch.codePointAt(0)
  return (
    (code >= 0x0300 && code <= 0x036f) || // 组合用附加符号
    (code >= 0x200b && code <= 0x200f) || // 零宽空格 / 方向标记
    (code >= 0xfe00 && code <= 0xfe0f) || // 变体选择符
    (code >= 0xfe20 && code <= 0xfe2f) || // 组合用半角符号
    code === 0x200d || // 零宽连接符
    code === 0xfeff // 零宽不换行空格
  )
}

/** 一个字的**逻辑宽度**（em）—— 逐字照抄 Rust `text_layout.rs::advance_em`。
 *
 *  ⚠️ 与本模块既有的 `measureCaption` 家族共用同一套常数：
 *  全角 1.0 / 半角 0.5 / 空格 0.25。**别在这里另写一套** ——
 *  那会让"逐字定位"与"折行"用两个字宽，而症状是"折行位置对、逐字位置偏"。 */
export function advanceEm(ch) {
  if (isZeroWidth(ch)) return 0.0
  if (isFullWidth(ch)) return 1.0
  if (ch === ' ' || ch === '\t') return 0.25
  return 0.5
}

/** **逐字几何**：每个字在哪、多宽 —— 与 Rust `CharLayout::of` 同形。
 *
 *  ⚠️ 返回的是**行内局部坐标**（原点是这一行的左端），居中的事由调用方做。
 *  ⚠️ 零宽字符**也占一个格子**（位置与前一个字相同、宽为 0）：
 *  逐字错峰按**序号**取时间 ⇒ 序号必须与"用户看到的第几个字"一致，
 *  过滤掉零宽字符会让序号整体前移（"第 3 个字"指到别的字上）。 */
export function charLayoutOf(text) {
  const s = String(text ?? '')
  const out = []
  let offset = 0
  for (const ch of s) {
    const adv = advanceEm(ch)
    out.push({ ch, offsetEm: offset, advanceEm: adv })
    offset += adv
  }
  return out
}

/** 这一档**有没有逐字动效** —— 与 Rust `PerCharSpec::is_active` 同一条判据。
 *
 *  ⚠️ 判据是"时长 > 0 **且**至少一个通道真的会动"，**不是**"有没有名字"：
 *  一个只有名字、所有端点都相等的档什么都不会动，让它进逐字路径
 *  等于白白把整行拆开重画（每个字一张位图），而画面**一点变化都没有**。
 *  ⚠️ `spread_*` 那一对**也要算**：漏了它们，症状是"只配了散开的档完全不生效"。 */
export function perCharActive(spec) {
  if (!spec || !(spec.perCharFrames > 0)) return false
  const moves = (a, b) => Math.abs(a - b) > 1e-6
  return (
    moves(spec.scaleFrom, spec.scaleTo) ||
    moves(spec.rotationFrom, spec.rotationTo) ||
    moves(spec.offsetXFrom, spec.offsetXTo) ||
    moves(spec.offsetYFrom, spec.offsetYTo) ||
    moves(spec.spreadXFrom, spec.spreadXTo) ||
    moves(spec.spreadYFrom, spec.spreadYTo) ||
    // ⚠️ **单独判**，不能并进上一行：按序号放大字号那类档的两个端点
    //    **是相等的**（都是 1.0），用 moves(scaleFrom,scaleTo) 判会漏掉整个档。
    Math.abs(spec.scaleSpread) > 1e-6
  )
}

/** 第 `index` 个字**自己那一份动画的起始帧** —— 与 Rust `start_frame_for` 同式。
 *
 *  ⚠️ `count <= 1` 时恒 0（单个字没有"错峰"可言）。 */
export function perCharStartFrame(spec, index, count) {
  if (!spec || count <= 1) return 0
  const last = count - 1
  const i = Math.min(index, last)
  const rank = spec.reverse ? last - i : i
  return rank * spec.staggerFrames
}

/** **逐字变换**：第 `index` 个字在 `localFrame` 帧长什么样。
 *
 *  与 Rust `char_transform_at` 同式。⚠️ 三条边界各自会踩：
 *  ① 没动效 ⇒ 恒等；② **还没轮到 ⇒ 起始值**（不是恒等 —— 给恒等会让字
 *  "先正常显示一下再重新弹"，闪一下、极难归因）；③ **已经完了 ⇒ 终止值**
 *  （不是恒等 —— 散开的字不该在动画结束后飞回去）。 */
export function charTransformAt(spec, index, count, localFrame) {
  const IDENTITY = { scale: 1.0, rotationDeg: 0.0, offsetXEm: 0.0, offsetYEm: 0.0 }
  if (!perCharActive(spec)) return IDENTITY
  const start = perCharStartFrame(spec, index, count)
  const dur = spec.perCharFrames
  // ⚠️ 必须浮点：整数除法会算出 0 ⇒ 全部字停在起始值（"配了不动"）。
  const raw = (localFrame - start) / dur
  const t = Math.min(1, Math.max(0, raw))
  // ⚠️ 曲线走**本模块既有**的那份缓动，不另写。认不出的串退回线性
  //    （逐帧路径里不许抛：一帧抛 = 整块 chunk 失败）。
  const e = easePerChar(spec.easing, t)
  const mix = (a, b) => a + (b - a) * e
  // **按序号放大的位移**：`rank/(count-1)` 归一化 ⇒ 最后一个字挪满
  // `spread*Em`，且**与字数无关**。没有它时"散开"退化成整行平移
  //（每个字同一个位移）⇒ 字距不变，而**包围盒仍会宽一点点**（落点取整）
  // ⇒ 判据会假绿。这是 2026-10-13 真机实测抓到的。
  let rank = 0
  if (count > 1) {
    const last = count - 1
    const i = Math.min(index, last)
    rank = (spec.reverse ? last - i : i) / last
  }
  const spreadX = mix(spec.spreadXFrom, spec.spreadXTo) * spec.spreadXEm * rank
  const spreadY = mix(spec.spreadYFrom, spec.spreadYTo) * spec.spreadYEm * rank
  return {
    // ⚠️ `scaleSpread` 是**乘数**（倍率的比例才是「大一点」的直觉），
    //    且与位移**共用**下面那个 rank（组合档在 reverse 下才同向）。
    scale: mix(spec.scaleFrom, spec.scaleTo) * (1 + spec.scaleSpread * rank),
    rotationDeg: mix(spec.rotationFrom, spec.rotationTo),
    offsetXEm: mix(spec.offsetXFrom, spec.offsetXTo) + spreadX,
    offsetYEm: mix(spec.offsetYFrom, spec.offsetYTo) + spreadY
  }
}

/** 逐字用的缓动。**复用 `E` 上那几张曲线表**（与整块动效同一套）。
 *
 *  ⚠️ 认不出的串**退回线性**（与 Rust `Easing::from_text` 的兜底一致）——
 *  逐帧路径里抛一次就是整块 chunk 失败。 */
function easePerChar(name, t) {
  const key = String(name ?? '').trim()
  if (key === 'back_out') return E.backOut(t, 3)
  if (key === 'ease' || key === 'ease-in-out' || key === 'ease_in_out') {
    return t < 0.5 ? 2 * t * t : 1 - Math.pow(-2 * t + 2, 2) / 2
  }
  if (key === 'ease-in' || key === 'ease_in') return t * t
  if (key === 'ease-out' || key === 'ease_out') return 1 - (1 - t) * (1 - t)
  return t
}

function textMotionCfg(cfg) {
  const e = (cfg && cfg.elements) || {}
  const num = (v, d) => (typeof v === 'number' && Number.isFinite(v) ? v : d)
  return {
    typewriter: e.typewriter === true,
    scaleInFrom: num(e.scale_in_from, 1.0),
    scaleInPeak: num(e.scale_in_peak, 1.0),
    scaleInMs: num(e.scale_in_ms, 0),
    shakePx: num(e.shake_px, 0),
    shakeHz: num(e.shake_hz, 18.0),
    plateColor: Array.isArray(e.plate_color) && e.plate_color.length >= 4 ? e.plate_color : null,
    platePadPx: num(e.plate_pad_px, 12.0),
    // ⚠️ **摇摆的两个旋钮要一起带走**（P1，2026-10-13）：它们决定预览里
    //    「字摆不摆」。以前 `textMotionCfg` **根本不读它们** ⇒ 预览无从知道要摆。
    swayDeg: num(e.sway_deg, 0),
    swayMs: num(e.sway_ms, 0),
    // ---- 心跳（T8，2026-10-14 补）----
    //
    // ⚠️⚠️ **这三个以前不在这里**，于是「心跳」有个很别扭的不对称：
    //    档位（`effect_anim`）可以从**面板全局** `[elements]` 选，
    //    而**节奏参数只有写在事件上才读得到**。用户在面板上选了心跳时：
    //    `animMotion` 把预设的键摊进 base（那三个键在摊进来的预设里**是有的**），
    //    可用的是**全局那份 base** ⇒ 面板选的心跳**参数**无处可来。
    //
    //    ⚠ 这是本仓第 ① 类老病（"字段带过去了、没人读"）的**镜像**：
    //      **字段该带、却没带**。症状是"面板上明明选了心跳，预览里一动不动"，
    //      而成片侧读的是 toml 全局字段（**会跳**）⇒ 两边分叉、都不报错。
    //
    //    ⚠ 默认取 `1.0` / `0` 而不是预设里的 `1.14` / `1000`：
    //      「全局没配」必须等于「不跳」（与 `scaleInMs: 0` 同一纪律）。
    //      给非零默认值会让**每一个**没配心跳的工程都开始跳。
    heartbeatAmp: num(e.heartbeat_amp, 1.0),
    heartbeatCycleMs: num(e.heartbeat_cycle_ms, 0),
    heartbeatPeaksMs: Array.isArray(e.heartbeat_peaks_ms) ? e.heartbeat_peaks_ms : null
  }
}

/** **摇摆的三角波**：逐字照抄 `tools/polish-to-dhampir.mjs::swayKeyframes`。
 *
 *  ⚠️⚠️ **这是"预览 vs 成片"最容易分叉的一处**，所以两个"关"的分支必须
 *  **逐条对上转译器**（转译器那边写的是 `return []`）：
 *   * 角度非有限或为 0 ⇒ 不摆（只给角度 = 不知道摆多快）；
 *   * 周期非有限或 `<= 0` ⇒ 不摆（只给周期 = 不知道摆多远）；
 *   * `fps <= 0` 或 `span <= 0` ⇒ 不摆。
 *
 *  # 与转译器的**唯一差别**（刻意的）
 *
 *  转译器产出的是**离散的关键帧数组**（`[{frame, value}]`，喂给底座）；
 *  预览要的是**连续时间 t 上的角度**。两者必须同值，所以这里：
 *  ① 用**同一个** `PATTERN = [0, 1, 0, -1]` 与**同一个** `quarter` 公式；
 *  ② 求出"第几段 + 段内进度"，再走**同一套** `channelFrom` 的线性插值。
 *  ⇒ 于是"预览在某一帧的角度"与"底座读那批关键帧得到的角度"**逐值相等**，
 *     这件事由 `scripts/check-sway-parity.mjs` 钉住（含变异自证）。
 */
export function swayDegAt(localSec, swayDeg, swayMs, fps, spanFrames) {
  const amp = swayDeg
  const periodMs = swayMs
  if (!Number.isFinite(amp) || amp === 0) return 0
  if (!Number.isFinite(periodMs) || periodMs <= 0) return 0
  if (!(fps > 0) || !Number.isFinite(localSec)) return 0
  // 四分之一周期 = 从一个极值到中点的**帧数**（至少 1 帧，与转译器同一个 max）。
  const quarter = Math.max(1, Math.round((periodMs / 1000) * fps / 4))
  const localFrame = localSec * fps
  if (!(localFrame >= 0)) return 0
  const PATTERN = [0, 1, 0, -1]
  // ⚠️⚠️ **收尾段必须照抄转译器，这一条是"逐值对拍"抓出来的**（2026-10-13）。
  //
  //    转译器的循环是 `for (f = 0; f < span; f = i * quarter)` ——
  //    **停在最后一个严格小于 `span` 的倍数上**，然后：
  //        if (last.frame < span - 1) keys.push({ frame: span - 1, value: 0 })
  //    补一个**回到 0** 的收尾键（理由写在那边：否则 `channel_from` 会在
  //    剩余那段恒取上一个键的值 —— "字会歪着卡住"）。
  //
  //    所以真实曲线是**两段**：前面按三角波走到 `quarter * k`，
  //    最后从那个键**线性收回到 `span - 1` 处的 0**。
  //    我第一版只写了三角波 ⇒ 尾段预览还在摆、底座已经在收
  //    （实测差最大 2.79°，而**两边各自看都正常**）。
  //
  //    ⇒ 判据 `check-sway-parity.mjs` 就是为了抓这个而写的，第一次跑就抓到了。
  const span = Number.isFinite(spanFrames) && spanFrames > 0 ? spanFrames : null
  if (span !== null) {
    const tail = span - 1
    // 最后一个"严格小于 span"的倍数键的段号。
    const lastI = Math.ceil(span / quarter) - 1
    const lastKeyFrame = lastI * quarter
    // ⚠️ **顺序要紧**：先判"是否已越过收尾键"。写成
    //    `if (lastKeyFrame < tail && localFrame >= lastKeyFrame)` 会把
    //    `localFrame == tail` 那一帧也吃进插值分支 ⇒ 在 `span-1` 之后又插出一段
    //    （实测 frame 144 / span 144 时预览 0.27° 而底座 0°）。
    //    `channel_from` 的语义是"`>= 最后一键` 就取最后一键"，所以越界必须先返回。
    if (localFrame >= tail) return 0
    // 转译器只在 `lastKeyFrame < span - 1` 时才补收尾键（重合就不补）。
    if (lastKeyFrame < tail && localFrame >= lastKeyFrame) {
      const from = amp * PATTERN[lastI % PATTERN.length]
      const t = (localFrame - lastKeyFrame) / (tail - lastKeyFrame)
      return from + (0 - from) * t
    }
  }
  // 落在第 i 段：段起点 `i*quarter`，段终点 `(i+1)*quarter`，
  // 两端值 = `amp * PATTERN[i % 4]` / `amp * PATTERN[(i+1) % 4]`。
  const i = Math.floor(localFrame / quarter)
  const a = amp * PATTERN[i % PATTERN.length]
  const b = amp * PATTERN[(i + 1) % PATTERN.length]
  const seg = quarter
  const t = (localFrame - i * seg) / seg
  return a + (b - a) * t
}


/** **关键帧求值**：逐字照抄 Dhampir 的
 *  `dhampir-timeline/src/curve.rs::channel_from`（`pub fn channel_from`，2026-10-13 接通）。
 *
 *  # ⚠️⚠️ 这条以前**根本不存在**，是本计划 P1 修的那个洞
 *
 *  2026-10-13 之前，预览的 `textTiltTransform` **只读静态 `tiltDeg`**
 *  （`parseTextSource` 解出的 `p[7]`，整段恒定），**完全不认识关键帧** ——
 *  转译器 `polish-to-dhampir.mjs::swayKeyframes` 已经把「摇摆」写成
 *  `rotation` 关键帧、底座 `evaluate_overlay` 也按帧求值了，
 *  但预览这一路**一个字都没读** ⇒ 症状正是最恨的那种：
 *  **"配了摇摆、成片里在摆、预览里纹丝不动，且不报任何错"**。
 *
 *  # 语义（与底座四步**逐步对齐**，改这里要连底座一起看）
 *
 *  1. 只挑 `target` 匹配的键（不匹配 ⇒ 返回 `fallback`，即调用方给的静态值）；
 *  2. 先按 `frame` 排序（契约**不要求有序**）；
 *  3. `local <= 第一键` 取第一键、`local >= 最后一键` 取最后一键；
 *  4. 否则在相邻对里插值，用**后一个键**的缓动（`span <= 0` 时 `t = 1` 防除零）。
 *
 *  ⚠️ **`easing` 只认 `linear`**：转译器写的摇摆键全部是 `linear`。
 *  认不出的缓动**当 `linear`**（不是抛错）—— 与底座"坏缓动退化成直插"同一取舍，
 *  而且这是**预览**，抛错会让整块子树被 Vue 卸载（见本文件顶部的 2026-10-01 事故）。
 */
export function channelFrom(fallback, keyframes, target, localFrame) {
  if (!Array.isArray(keyframes) || keyframes.length === 0) return fallback
  const keys = keyframes
    .filter((k) => k && String(k.target) === String(target))
    .slice()
    .sort((a, b) => Number(a.frame) - Number(b.frame))
  if (keys.length === 0) return fallback
  const first = keys[0]
  if (localFrame <= first.frame) return Number(first.value)
  const last = keys[keys.length - 1]
  if (localFrame >= last.frame) return Number(last.value)
  for (let i = 0; i + 1 < keys.length; i++) {
    const a = keys[i]
    const b = keys[i + 1]
    if (localFrame >= a.frame && localFrame <= b.frame) {
      const span = Number(b.frame) - Number(a.frame)
      // span 为 0 = 两个键在同一帧 ⇒ 取后一个（别除零），与底座同一条。
      const t = span <= 0 ? 1 : (localFrame - Number(a.frame)) / span
      // 只认 linear；认不出当 linear（`t` 本身）。
      const eased = t
      return Number(a.value) + (Number(b.value) - Number(a.value)) * eased
    }
  }
  return fallback
}

/** T1 弹入的曲线：`back_out3`，**逐字照抄** Dhampir 的
 *  `dhampir-timeline/src/text_motion.rs::back_out3`（s = 3）。
 *
 *  ⚠️ 它**过冲到 1.25 再回落**，而 `scaleInPeak` 是**曲线端点不是峰值** ——
 *  参照值下实际最大是 `0.7 + (1.15-0.7)×1.25 = 1.2625`。
 *  两边各写一份必然漂移，所以这里连注释都按那边抄，改一处就要改另一处。 */
function backOut3(p) {
  const t = Math.min(1, Math.max(0, p))
  const u = t - 1
  return u * u * (4 * u + 3) + 1
}

/** **心跳的双峰脉冲曲线**（2026-10-14，用户：
 *  「**动效**需要增加一个心跳，扑通扑通的心跳感」）。
 *
 *  形状 = **两次快速脉冲 + 停顿**，循环（`咚·咚……咚·咚`）。
 *  返回值是**缩放倍率**（1.0 = 基线）。
 *
 *  ⚠⚠ **为什么必须单独写一个函数，不能复用 `backOut3`**：
 *     `backOut3`（s=3）是**单次过冲**曲线 —— 数学上**只有一个**极值点
 *     （导数在 `[0,1]` 内只穿零一次）。「扑通扑通」要的是**两个**峰
 *     ⇒ 单峰曲线**表达不了**，这**不是参数没调好，是函数形状不对**。
 *
 *  ⚠ 曲线**必须逐帧可复现**（纯 `ms` 的函数、无累积状态）—— 与 T4 抖动同纪律：
 *    预览要能任意 seek，带状态的曲线一 seek 就与成片分叉。
 *
 *  # 节奏（与转译器 `ANIM_PRESETS.heartbeat` 的 `heartbeatPeaksMs` **同源**）
 *
 *  一个周期 `cycleMs` 内两个峰：
 *    `0ms`     第 1 峰（收缩最紧）
 *    `180ms`   回落到基线
 *    `300ms`   第 2 峰（比第 1 峰**弱一点**）
 *    `480ms`   回到基线，**剩下 520ms 停顿**（这就是“扑通……扑通”的留白）
 *
 *  ⚠ 第二峰**必须比第一峰弱**：两个等高的峰看着像"抖动"而不是心跳。
 *
 *  @param {number} ms 相对**事件起点**的毫秒数（< 0 返回基线）
 *  @param {Array<number>} peaksMs 两个峰的起点（相对周期起点）
 *  @param {number} amp 第一峰的倍率（如 1.14）
 *  @param {number} cycleMs 一个周期的毫秒数
 */
export function heartbeatScaleAt(ms, peaksMs, amp, cycleMs) {
  const p = Array.isArray(peaksMs) ? peaksMs : []
  const p1 = Number(p[0])
  const p2 = Number(p[1])
  const a = Number(amp)
  const cyc = Number(cycleMs)
  if (!Number.isFinite(p1) || !Number.isFinite(p2) || !Number.isFinite(a) ||
      !Number.isFinite(cyc) || cyc <= 0 || a <= 1 || p2 <= p1) {
    return 1
  }
  const t = Number(ms)
  if (!Number.isFinite(t) || t < 0) return 1
  const fallMs = p2 - p1          // 两峰**同宽**（用同一个回落时长）
  const inCycle = t % cyc
  if (inCycle >= p1 && inCycle < p1 + fallMs) {
    return 1 + (a - 1) * backOut3((inCycle - p1) / fallMs)
  }
  // ⚠ 第 2 峰弱 30%（见上面"为什么必须更弱"）。
  const weak = 1 + (a - 1) * 0.7
  if (inCycle >= p2 && inCycle < p2 + fallMs) {
    return 1 + (weak - 1) * backOut3((inCycle - p2) / fallMs)
  }
  return 1
}

/** T4 抖动的**逐帧偏移**（文档像素）。**逐字照抄** `text_motion.rs::text_shake_px`。
 *
 *  ⚠️ 三条"关"的分支必须**显式挡**（那边专门写了三行解释，这里是同一套）：
 *   * `shakePx <= 0` / 非有限 ⇒ `(0,0)`；
 *   * `shakeHz <= 0` ⇒ **也要挡**。0 Hz 代进公式**不崩**：`sin(phase)` 是常数，
 *     整条字**偏移一个固定量** —— 那不是"不抖"，是**错位**，而且同屏多条各偏各的；
 *   * `t` 非有限 ⇒ `(0,0)`（NaN 会顺着 `sin` 传染到落点）。
 *  ⚠️ 两个方向的频率**刻意不同**（`2.7`/`3.1`、`1.3`/`1.0`）：
 *     都相同的话轨迹退化成一条对角直线，不是抖。 */
function textShakePx(tSec, shakePx, shakeHz, shakePhase) {
  if (!(shakePx > 0) || !Number.isFinite(shakePx)) return [0, 0]
  if (!(shakeHz > 0) || !Number.isFinite(shakeHz) || !Number.isFinite(tSec)) return [0, 0]
  const ph = Number.isFinite(shakePhase) ? shakePhase : 0
  const w = 2 * Math.PI * shakeHz
  const dx = shakePx * (0.62 * Math.sin(w * tSec + ph) + 0.38 * Math.sin(w * 2.7 * tSec + ph * 1.9))
  const dy =
    shakePx *
    (0.62 * Math.sin(w * 1.3 * tSec + ph + 1.1) + 0.38 * Math.sin(w * 3.1 * tSec + ph * 0.6 + 2.3))
  return [dx, dy]
}

/** 把 `"9%"` / `"9.00%"` / `9` 这类落点值读成数值百分比。读不出返回 `null`。
 *  ⚠️ **只读不推**：`process` 写的就是这个格式（`format!("{:.2}%", …)`），
 *  预览**不重新算落点** —— 见 `buildIndex` 的 `kind === 'text'` 分支。 */
function numPct(v) {
  if (typeof v === 'number' && Number.isFinite(v)) return v
  if (typeof v !== 'string') return null
  const m = v.trim().match(/^(-?\d+(?:\.\d+)?)%?$/)
  if (!m) return null
  const n = parseFloat(m[1])
  return Number.isFinite(n) ? n : null
}

/** 解析 `process` 写回的文字样式串：
 *
 *  ⚠️ **认不出返回 `null`，由调用方决定怎么办** —— 不在这里编一套兜底数值。
 *  理由与转译器那句注释同源：这个串是**本仓自己写的**，它坏了属于"内部不一致"；
 *  而预览如果自己编一套，就会**掩盖**那个不一致（看起来画对了、其实与成片不同）。
 *  ⇒ 读不出时 `getElements` 那条**不画**，并让调用方知道（不静默画错的东西）。 */
function parseTextSource(src) {
  if (typeof src !== 'string' || !src.includes('|')) return null
  const p = src.split('|')
  if (p.length < 7) return null
  const num = (s, d) => { const n = parseFloat(s); return Number.isFinite(n) ? n : d }
  const rgba = (s) => {
    const a = String(s || '').split(',').map((x) => parseFloat(x))
    return a.length >= 4 && a.every((x) => Number.isFinite(x)) ? a : null
  }
  const fill = rgba(p[3])
  const stroke = rgba(p[4])
  if (!fill) return null
  return {
    name: p[0] || 'comment',
    // ⚠️ 兜底 80 = Rust `placement.rs::ELEMENT_FONT_PX`（2026-10-13 四档字号统一为 80）。
    //    旧值 44 是"comment 档的字号"，统一之后**那个档不存在了** ——
    //    留着会让"串坏掉"的那条字比同类小 45%，而预览里**不报错**（最隐蔽的那种分叉）。
    //    判据：`scripts/check-text-style.mjs` ⑦（三处兜底必须与 Rust 同值）。
    fontPx: num(p[1], 80),
    strokePx: num(p[2], 0),
    fill,
    stroke: stroke || [26, 26, 26, 255],
    weight: num(p[5], 700),
    backdrop: num(p[6], 0) > 0,
    // ---- T3：倾斜 / 纵向压缩 ----
    //
    // ⚠️ 这两个字段由 `process/mod.rs` **追加在 source 串末尾**（那边注释写了理由：
    //    插在中间会让既有字段静默错位）。所以这里**只按位置取、且允许缺** ——
    //    老工程（`process` 还没重跑过）的 `source` 只有 7 段，`p[7]`/`p[8]` 是
    //    `undefined` ⇒ 落到下面这两个默认值，与老行为逐像素一致（R4）。
    //
    // ⚠️ 缺省必须是 (0, 1) 这一对**恒等值**：给错（比如 tilt 缺省非 0）会让
    //    **所有存量工程的字突然歪掉**，而那个歪从界面上看不出是从哪来的。
    tiltDeg: num(p[7], 0),
    squashY: num(p[8], 1),
    // ---- 第 10 段：颜色预设名（2026-10-13）----
    //
    // ⚠️⚠️ **只带出来给人看 / 给判据用，绝不拿它去查表覆盖 `fill`/`stroke`**：
    //    那两个 RGBA 已由 `process` 算好写在第 4/5 段里（**含 `black` 那条
    //    "描边成对翻白"**）。这里再查一次表就是第二份颜色来源 ——
    //    预览与成片必然在 `black` 上分叉（预览糊成一团、成片正常），而两边都不报错。
    //
    //    ⚠️ 老工程（7 段）与 T3 工程（9 段）没有这一段 ⇒ `undefined` ⇒ 空串，
    //       与改动前逐像素一致（R4）。
    colorName: String(p[9] ?? '').trim().toLowerCase()
  }
}

// ---- 文字元素的 CSS transform（T4）----
//
// ⚠️⚠️ **这个顺序是本模块最容易被"顺手改对"的地方，改它之前先读完这段。**
//
// 权威定义在 Dhampir 侧，两处：
//   · 契约/GPU：`dhampir-core/src/render/compose.rs::inverse_affine`
//       正向  p_out = R(rot) · ((p_src − c_src) ⊙ scale) + c_out + offset
//       逆向  p_src = R(−rot) · (p_out − c_out − offset) ⊘ scale + c_src
//   · 文字/CPU（T2）：`dhampir-worker/src/text_transform.rs::blit_affine`
//       ← 出片时真正给字做仿射的就是它。它把 `scale` 从标量推广成 `(sx, sy)`，
//         其余（绕中心、旋转方向、offset 语义）逐字不变。
//
// 出片的**正向**（上面四角循环展开）是：
//       dx = (px − c_src_x) · sx ;  dy = (py − c_src_y) · sy
//       ox = cos·dx − sin·dy + c_out_x
//       oy = sin·dx + cos·dy + c_out_y
// 写成矩阵就是 **`R(rot) · S(sx, sy)`** —— 点**先缩放、后旋转**。
//
// CSS 的 transform 是**右到左**作用的：`transform: A B` 作用在点上是 `A(B(p))`，
// 即矩阵 `A·B`。所以要得到 `R·S`，**`rotate` 必须写在 `scale` 前面**：
//
//       ✔ `rotate(T deg) scale(sx, sy)`   → 矩阵 R·S  ← 与出片逐值相同
//       ✘ `scale(sx, sy) rotate(T deg)`   → 矩阵 S·R  ← 斜着的字会被沿**旋转后的轴**拉伸
//
// ⚠️ 实测差多少（`tilt=6, sx=1.2, sy=0.92`，600×120 的字盒）：线性矩阵元素最大差 **0.183**，
//    换算到字盒角上 ≈ **4.4px**；`tilt=7,sy=0.85` 时 ≈ 5.5px。**肉眼不一定看得出**，
//    但"斜着的字被拉歪"在长句上会明显（基线不再等距）。
// ⚠️ 两个**退化情形两种顺序都对**，所以随手试很容易以为没事：
//    `sy == 1`（不压）或 `tilt == 0`（不歪）⇒ `R·S == S·R`。本档位表里
//    `chapter/comment` 恰好就是 `tilt=0, squash=1` ⇒ **默认那两档永远试不出来**。
//
// 合成顺序（自左向右 = 从最外层到最内层）：
//   `translateY(浮起) translate(抖动) rotate(倾斜+进场旋转) scale(动效缩放, 纵向压缩)`
//   · `translateY`/`translate` 是**位移**，与线性部分可交换（都在最外层，先挪位置）；
//   · **`rotate` 里是两个角相加**：样式倾斜（`tiltDeg`，绕中心、整段恒定）+ 进场动画的
//     临时旋转（`rot`，`backOut` 回弹，进场结束恒为 0）。两者都是绕**同一个中心**的
//     旋转 ⇒ 直接相加即可，**不需要**写成两次 `rotate`。
//   · `scale(scale, squashY)` 是**两参数**版本 = 非等比（CSS 的 `scale()` 两参数正好是
//     `(x, y)`）：横向 = 动画缩放，纵向 = 动画缩放 × 纵向压缩。
//     ⚠️ 写成 `scale(a) scaleY(b)` 是**等价的**（`scale(a) scaleY(b)` ≡ `scale(a, a·b)`，
//        已逐值验证），但**顺序仍要在 `rotate` 之后** —— 当年工作区里那版错就错在
//        把它放到了 `rotate` 前面，与出片差了整整一个矩阵乘法序。
//
// 导出是为了让判据（`webui/scripts/check-text-tilt.mjs`）能 import 它，
// 把"顺序"这件事钉成机器判据 —— 而不是靠下次改的人读到这段注释。
export function textTiltTransform({ yOff = 0, shakeX = 0, shakeY = 0, scale = 1, rot = 0, style = {}, swayDeg = 0, animName = '' }) {
  // ---- 39 档（2026-10-13）：`tilt_*` / `slam_squash` **压过**样式档自带的那两个值 ----
  //
  // ⚠️ **优先"逐条 `anim` 预设"，其次才是 `style`**：
  //    `style.tiltDeg` / `style.squashY` 来自 `[style.*]` 四档解析（老路径），
  //    而 `anim = "tilt_std"` 是**这一段文字自己要歪** —— 用户明确选了动效档时以它为准。
  //    反过来的症状是"选了倾斜·标准却没反应"，而**样式档那边有值**（老工程常常有），
  //    所以两边都有值时谁赢必须写死，不能靠"顺序"。
  // ⚠️ 与转译器 `transform: { rotation: preset?.tiltDeg ?? st.tiltDeg }` **逐字同形**。
  const preset = animPreset(animName)
  const squashY = Number.isFinite(preset?.squashY)
    ? preset.squashY
    : (Number.isFinite(style.squashY) ? style.squashY : 1)
  const tiltDeg = Number.isFinite(preset?.tiltDeg)
    ? preset.tiltDeg
    : (Number.isFinite(style.tiltDeg) ? style.tiltDeg : 0)
  // 旋转角 = 样式倾斜 + 进场动画旋转 + 摇摆（P1）—— 三者绕**同一个中心** ⇒ 直接相加。
  //
  // ⚠️ **摇摆必须在这里加、不能在 `getElements` 里先并进 `rot`**：
  //    `rot` 是**进场**那一下的临时旋转（`backOut` 回弹，进场结束恒为 0），
  //    而摇摆是**整段持续**的。并进 `rot` 会让"进场结束"的分支把它一起清掉。
  //    更要紧的是：那样合成顺序就变成"(进场+摇摆) vs 样式倾斜"两个阶段，
  //    而底座是**一次性**算 `静态 rotation + 曲线 rotation`。
  const deg = tiltDeg + rot + (Number.isFinite(swayDeg) ? swayDeg : 0)
  return (
    'translateY(' + yOff + 'px)' +
    ' translate(' + shakeX + 'px,' + shakeY + 'px)' +
    ' rotate(' + deg + 'deg)' +
    ' scale(' + scale + ', ' + scale * squashY + ')'
  )
}

// ---- 运镜档位 → 缩放系数。必须与 Rust CameraStyle::factor 逐值一致 ----
// 缺省 1.3/1.5/0.55（与 render-core 的 Default 相同）；这里曾写 1.8/1.5，
// 于是 [style.camera] 缺失时预览比成片小一大截。
export function camFactor(label, camStyle) {
  const c = camStyle || {}
  const pick = (v, d) => (typeof v === 'number' && Number.isFinite(v) ? v : d)
  // ⚠️ 与 Rust `CameraStyle`、模板 `camFactor` 逐值一致（门禁 check-camera-zoom-parity 钉住）。
  if (label === '特写') return pick(c.zoom_closeup, 1.8)
  if (label === '近') return pick(c.zoom_close, 1.5)
  if (label === '远') return pick(c.zoom_wide, 0.55)
  return pick(c.zoom_mid, 1.3)
}

// ---- 手持摇摆（运镜的附加变种，`Event::Camera::handheld`） ----
// 与 yekitrim-render-core 的 `types::handheld_sway`、模板 `handheldSway` **逐值同源**
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
  // 荷兰角（规格 §4）：整帧绕聚焦中心旋转的度数，**持续状态**、与 zoom 同一套状态机。
  // ⚠️ 初值必须是 0（不能取第一条事件的值）：否则**片头一开始就歪着**，而用户
  //    从任何界面都看不出那个初值是从哪来的。
  let tilt = 0
  // 默认聚焦中心：自由选框（框选布局）> origin_grid > origin > 首个格子
  const defOrigin = regionOrigin(idx.layout.liver)
  const dp = defOrigin.split(' ')
  ox = parseFloat(dp[0]) || 65
  oy = parseFloat(dp[1]) || 50

  // 运镜轨的眼睛关着 ⇒ 不做运镜 —— 手持摇摆是运镜的附加变种，跟随同一条开关。
  // ⚠️ 返回值（默认聚焦中心 + zoom=1）与"空相机列表"逐值相同，见 buildIndex 里的 `cameraOff`。
  if (idx.cameraOff) return { zoom: 1.0, ox: (ox / 100) * idx.cv.W, oy: (oy / 100) * idx.cv.H, handheld: 0, tilt: 0 }

  const wide = camFactor('远', idx.camStyle)
  for (const c of idx.camera) {
    if (t < c.time) break
    const tgt = camFactor(c.ev.zoom, idx.camStyle)
    // 硬切判据要在**算 td 之前**出来（`cameraTransitionSecs` 拿它当第一优先），
    // 所以这一段顺序不能调：先判 `hardCut`、再算 `td`。早先 `td` 只依赖 `duration`，
    // 顺序无所谓；现在 `speed` 与 `cut` 互斥、以 `cut` 为准，顺序就成了正确性的一部分。
    const hardCut = c.ev.cut === true || Math.abs(tgt - wide) < 1e-6 || Math.abs(zoom - wide) < 1e-6
    const td = cameraTransitionSecs(c.ev, c.duration, hardCut)
    const ce = c.time + td
    // 硬切：本事件写了 cut（编辑器「方式 = 硬切」），或远景（画中画）进出整档都是硬切：
    // 缓动缩过去看着像镜头在拉远，而它要的是喜剧式的切换，所以这一段不缓动、
    // 连位移也一并瞬间到位。cut 把同一条规则放开给任意景别。
    const p = t >= ce || hardCut ? 1.0 : E.pow2Out((t - c.time) / td)
    if (t >= ce || hardCut) zoom = tgt
    else zoom = lerp(zoom, tgt, p)
    // 手持摇摆是状态机上的持续项（跟到下一条不带 handheld 的运镜），同一段 p 起停
    handheld = lerp(handheld, c.ev.handheld === true ? 1 : 0, p)
    // 荷兰角走**与 zoom 逐式同形**的过渡（同 `td`、同 `hardCut`、同 `p`）。
    // ⚠️ 用 `typeof === 'number'` 而不是 `|| 0`：`tilt = 0`（不倾斜）是合法值。
    tilt = lerp(tilt, typeof c.ev.tilt === 'number' ? c.ev.tilt : 0, p)
    // 不动点：本事件的 origin → to_origin（缺省即原地不动）。
    // 起点取事件自己的 origin，与 zoom 同一段窗口、同一条缓动。
    const oa = String(c.ev.origin || defOrigin).split(' ')
    const ob = String(c.ev.to_origin || c.ev.origin || defOrigin).split(' ')
    const ax = parseFloat(oa[0]); const ay = parseFloat(oa[1])
    const bx = parseFloat(ob[0]); const by = parseFloat(ob[1])
    ox = lerp(Number.isFinite(ax) ? ax : ox, Number.isFinite(bx) ? bx : ox, p)
    oy = lerp(Number.isFinite(ay) ? ay : oy, Number.isFinite(by) ? by : oy, p)
  }
  return { zoom, ox: (ox / 100) * idx.cv.W, oy: (oy / 100) * idx.cv.H, handheld, tilt }
}

// ---- 视频层复合状态：shake/blur/最终 zoom ----
// 竖屏不再屏蔽这些视觉特效（只有运镜 camera 被屏蔽，见 cameraAt 顶部）。
// 与 templates/index.html 的 handlers 段保持一致：作用对象本来就对，
// blur/color_shift 打 #video-wrap、其余打 #source-video，背景层是兄弟节点不受波及。
export function videoFxAt(idx, t) {
  const shake = getShake(idx, t)
  const blurAmt = getBlur(idx, t)
  const cam = cameraAt(idx, t)
  const pl = getPulse(idx, t, cam.zoom)
  // ⚠️ **这个返回对象的字段就是预览的契约**（`PolishPreview.vue` 读 `f.shake` / `f.blur` /
  // `f.camera.handheld` / `f.camera.ox|oy` / `f.zoom`）。2026-10-01 删类型时它被削成了
  // `{ shake, blur }` ⇒ 预览在 `f.camera.handheld` 上抛 `can't access property "handheld",
  // i.camera is undefined`（同样会让整块预览消失）。所以：**删字段前先 grep 消费者**，
  // 并且 `scripts/check-preview-engine.mjs` 会真跑一次断言这几个字段都在。
  return {
    shake, blur: blurAmt,
    camera: cam,
    zoom: pl,
  }
}

// ---- 瞬时 FX 状态（Canvas 出片行为的窗口函数）----
function getShake(idx, t) {
  for (const sh of idx.shake) {
    if (t >= sh.time && t < sh.time + 0.2) {
      const off = typeof sh.ev.intensity === 'number' ? sh.ev.intensity : 14
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
// 四处实现必须同改：本文件、timeline.rs、crates/yekitrim-polish/templates/index.html（出片权威路径）、
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
export function gradientCss(colors, shape, angle) {
  const stops = colors
    .map((c, i) => `${c} ${((i / (colors.length - 1)) * 100).toFixed(1)}%`)
    .join(',')
  if (shape === 'radial') return `radial-gradient(circle at center, ${stops})`
  // 没写 angle ⇒ 参照缺省方向 `to bottom right`（**与改造前逐字一致**）；写了就用 CSS 角度
  // （0 = 向上、顺时针）。⚠️ 与模板 `overlayGradientCss` **必须同口径**（`check-overlay.mjs` 对拍）。
  if (typeof angle !== 'number' || !Number.isFinite(angle)) return `linear-gradient(to bottom right, ${stops})`
  const deg = ((angle % 360) + 360) % 360
  return `linear-gradient(${deg}deg, ${stops})`
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
      if (ot === 'gradient' && stops) return { type: 'gradient', colors: stops, shape: ev.shape || 'linear', angle: ev.angle, opacity }
      // 类型不认识 / 值不全：继续看窗口里的下一条，别把整窗吃掉
    }
  }
  return null
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


// ---- 字幕：fade in/out + 高亮分段 ----
/// **T5 真卡拉OK（预览腿）**：按字级时间戳算出"此刻该显示到第几个字"。
///
/// ⚠️⚠️ **这是与转译器 `polish-to-dhampir.mjs::splitKaraoke` 的逐值对拍实现**，
/// 两边**必须同口径** —— 漂了的症状是「预览里字在长、成片里是整条」（或反过来），
/// **两边各自看都正常、也都不报错**。判据 = `scripts/check-karaoke-parity.mjs`。
///
/// ⚠️ 与转译器**有意不同**的一点：那边是"先切成 N 条 cue 再交给底座"（因为底座没有
/// 逐字通道），这边是"求值期算一个截断长度"（预览本来每帧就算一次）。
/// **算法一致、产物形态不同** —— 所以对拍的是**每个字落在哪一刻**，不是 cue 列表。
///
/// ⚠️ 三道守卫与转译器一致，缺一不可（每一道都对应那边的一段注释）：
///   ① 字数对不上 / 只有一个字 ⇒ 不切
///   ② 时间戳非单调 ⇒ 数据坏了，不切（别硬算出一个来回跳的动画）
///   ③ **每字严格递增**（`prev + 1`）：ASR 把一口气的几个字给同一时刻时，
///      `Math.max(ms, prev)` 不够 —— 那样多条会挤在同一刻、屏幕上"唰"地一起冒出来。
///      ⇒ 必须严格递增；顶出末尾就整条退回。
///
/// 返回 `null` = 不切（调用方按整条字幕画）。
export function karaokeSplitCount(sub, times, subStart, subEnd) {
  const text = String(sub?.text ?? '')
  const chars = Array.from(text)
  if (chars.length <= 1) return null
  if (!Array.isArray(times) || times.length !== chars.length) return null
  const span = subEnd - subStart
  if (!(span > 0)) return null
  const at = []
  let prev = subStart
  for (let i = 0; i < chars.length; i += 1) {
    const ts = times[i]
    if (!Number.isFinite(ts)) return null
    if (i > 0 && ts < times[i - 1]) return null
    const rel = (ts - subStart) / span
    let ms = subStart + rel * span
    // ⚠️⚠️ **必须是 1 毫秒，不是 `1e-6`** —— 与转译器的 `Math.max(ms, prev + 1)` **同量纲**。
    //    预览的时间轴本来是秒，写成 `prev + 1e-6`（= 1 微秒）看着"也递增了"，但**门槛不同**：
    //    当多个字挤在同一时刻、可用区间又很窄时，出片腿会因"顶出末尾"整条退回，
    //    而预览腿塞得下 ⇒ **预览里逐字在长、成片里是整条**。
    //    这正是本判据第一次跑就抓出来的那一条（35 项里红 1 项）。
    ms = Math.max(ms, prev + 0.001)
    at.push(ms)
    prev = ms
  }
  at[0] = subStart
  // 顶出末尾 ⇒ 切不了（与转译器同一条守卫）
  if (at[at.length - 1] > subEnd) return null
  return at
}

/// 按 [karaokeSplitCount] 算出的时刻表，返回 `t` 时刻应显示的**字数**（1..n）。
/// 返回 0 表示"字幕已经出来、但第一个字还没到" —— 与转译器 `at[0] = startMs` 的
/// 口径一致：**第一个字立即显示**，所以正常不会出现 0（除非 t 恰在边界上取整）。
export function karaokeShownChars(at, t) {
  if (!Array.isArray(at) || !at.length) return 0
  let n = 0
  for (let i = 0; i < at.length; i += 1) {
    if (t >= at[i]) n = i + 1
    else break
  }
  return n
}

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
    // ---- §5 T5 真卡拉OK：按字级时刻把**显示文本**截到"此刻该出现的字数" ----
    //
    // ⚠️ **只在能对上的那一条字幕上生效**：`idx.karaoke` 按起点毫秒索引，
    //    对不上 ⇒ `times` 是 undefined ⇒ `karaokeSplitCount` 返回 null ⇒ 整条原样。
    // ⚠️ 与成片侧的形态差异（求值期截断 vs 切成 N 条 cue）是**有意**的，见
    //    `karaokeSplitCount` 的注释；两边**每个字落在哪一刻**必须逐值相等，
    //    判据 `scripts/check-karaoke-parity.mjs`。
    let shown = null
    if (idx.karaoke && idx.karaoke.size) {
      const times = idx.karaoke.get(Math.round(s.start * 1000))
      if (times) {
        const at = karaokeSplitCount(s, times, s.start, s.end)
        if (at) {
          const n = karaokeShownChars(at, t)
          // n<=0 ⇒ 第一个字还没到 —— 此时**不画**（画整条会"未唱先全出"，画空串会闪一下）
          const full = parts.map((p) => p.text).join('')
          shown = Array.from(full).slice(0, Math.max(0, n)).join('')
          if (!shown.length) continue
        }
      }
    }
    // ⚠️ `fullText` = **整条字幕的全文**（未折行、未截断）——
    //    逐字的**序号与总字数必须按整条算**（与底座同口径：底座是按整行
    //    `line.chars` 算的，而不是按折行后的某一行）。折行只影响**画在哪**，
    //    不影响"第几个字、一共几个字"。
    const fullText = parts.map((p) => p.text).join('')
    if (shown !== null) {
      // ⚠️ 截断后**丢掉 `hl` 分段**：逐字推进的每一帧都是"前缀"，而 `hl` 是按整条原文
      //    标的 —— 前缀落在 `hl` 段中间时该不该高亮没有定义。宁可退成整段同色
      //    （卡拉OK本来就该是"一个字一个字亮"，与 `hl` 是两套语义），也不要画出一个
      //    没人预期的半截高亮。
      out.push({ parts: [{ text: shown, hl: false }], fullText,
                 opacity, yOff: yOff + pulse.yOff, scale: pulse.scale,
                 ...subAnimFields(idx, s, t) })
      continue
    }
    out.push({ parts, fullText, opacity, yOff: yOff + pulse.yOff, scale: pulse.scale,
               ...subAnimFields(idx, s, t) })
  }
  return out
}

/** **字幕的动效字段**（39 档，2026-10-13 补）。
 *
 *  # 为什么会有这个函数
 *
 *  ⚠️⚠️ `subsAt` 以前只返回 `parts / opacity / yOff / scale` —— **一个动效字段都没有**。
 *  而 `PolishPreview.vue` 的字幕层也只做 `translateY + scale`。后果是：
 *  **字幕轨的 39 档在预览里一个都不动**（文字元素那条路能动，字幕这条不能），
 *  而成片是底座按轨级样式真的画的。用户报的「播放没有效果」里，字幕那一半就是它。
 *
 *  # 口径（与转译器逐字对齐）
 *
 *  转译器把 `anim` 当**轨级**属性（`${style}|${pos}|${anim}` 分组），
 *  一条轨上的所有 cue 共用一档 ⇒ 这里对**单条 cue** 取它自己的 `anim` 即可
 *  （分轨已经在转译期做完了，预览不必再分）。
 *
 *  ⚠️ 返回**展开用的对象**（`...spread`）⇒ 没有动效档时**一个键都不加**，
 *  消费方拿到的形状与改造前**逐字节相同**（R4）。
 *  ⚠️ `tilt_*` / `slam_squash` 是**静态**值（不是时间函数），必须**单独带出去**
 *  交给 `textTiltTransform` —— 与文字元素那条路同一个理由，别只摊进 `textMotion`。
 *
 *  ⚠️⚠️ **2026-10-13 两级化：这里也必须拆两轴** —— 否则用户在**新界面**上给
 *     字幕选「竖排 + 抖动」（写的是两个新字段），预览仍然只读老 `anim`（空）
 *     ⇒ **预览完全没效果、而成片有**。这正是用户报的「前端预览没有实现效果」。
 *     元素那条路（`buildIndex` 里）当天就改了，**字幕这条路漏了** ——
 *     同一个"两个消费者只改了一个"的错误，本仓犯过很多次。 */
function subAnimFields(idx, s, t) {
  // ⚠️ 优先级与转译器 / 元素那条路**逐条同源**：
  //   ① 新字段任一有值 ⇒ **以它为准**（`anim` 被忽略）；
  //   ② 两个新字段都空、`anim` 有值 ⇒ 按**参数归属**拆（只读迁移）；
  //   ③ 都没有 ⇒ 一个键都不加（老工程形状不变）。
  const legacySplit = splitAnimTwoAxis(s.anim)
  const styleAnimName = String(s.style_anim ?? '').trim().toLowerCase() || legacySplit.style
  const effectAnimName = String(s.effect_anim ?? '').trim().toLowerCase() || legacySplit.effect
  // ⚠️ 第三路（过渡）：老格式的 `anim = "pop"` / `"fade"` 就是过渡档，
  //    不接的话它们三路全空 ⇒ 静默回落全局（与转译器同一个坑，逐条同源）。
  const transitionAnimName = String(s.transition_anim ?? '').trim().toLowerCase()
    || legacySplit.transition
  // 三轴都空 ⇒ 什么都不加。
  if (!styleAnimName && !effectAnimName && !transitionAnimName) return {}
  const local = t - s.start
  return {
    // ⚠️ `animName` 是**兼容字段**（老代码理解成"那一档"）⇒ 取优先级最高的。
    animName: styleAnimName || transitionAnimName || effectAnimName,
    transitionAnimName,
    // ⚠️ 档名分两个带出去（不只是摊进 `textMotion`）：`tilt_*` / `slam_squash`
    //    改的是**静态** `rotation` / `scale_y`，由 `textTiltTransform` 读**样式轴**。
    styleAnimName,
    effectAnimName,
    // ⚠️ **第一个参传 `null` 是有意的**：`animMotion` 只在"认不出这一档"时才用
    //    那份全局配置，而这里传的是**逐条字幕**的档。字幕这条路**没有"全局动效"**
    //    这个概念（转译器那边字幕轨是轨级样式）⇒ 给 `null`（= 全关）比给一份
    //    假配置更诚实。结果是"认不出的档名 ⇒ 字幕不动"，与成片一致。
    textMotion: animMotion(null, effectAnimName, transitionAnimName),
    // ⚠️ 与文字元素同一条换算（`local * fps`），别写成秒 —— `charTransformAt`
    //    的第四参是**帧**（底座 `char_transform_at` 同口径）。
    // ⚠️ **逐字取样式轴**（`pop_perchar` / `hop_*` / `scatter_*` 的 `perChar`
    //    都归在那一档里）—— 与元素那条路同一个口径。
    // ⚠️⚠️ **逐字那半边现在要试两轴**（2026-10-14，与转译器 `motionFields` 同一个修）。
    //    原来只认 `styleAnimName` ⇒ 三轴收敛后「逐字变化」住在**动效**轴
    //    （`perchar_big`/`perchar_small` 的 `sort: 'effect'`），而它们的 `perChar`
    //    仍是**形态字段** ⇒ 只认排版轴的话，用户选「逐字变大」**字幕上一条都不动**，
    //    且报告里一行差异都没有（成片侧同一个坑，已修）。
    //    ⚠️ 顺序 = 优先级：排版轴（`arc` 的逐字）优先，与转译器的 `perCharFrom` 同序。
    perChar: animPerChar(styleAnimName) ?? animPerChar(effectAnimName)
      ?? animPerChar(transitionAnimName),
    perCharFrame: local * idx.fps,
    perCharFps: idx.fps
  }
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
// 窗口 = **事件时长** `dur`（不溢出）：淡出在窗口**内部**消化（`dur - 0.3 -> dur` 渐隐），
// 所以出点 = 最后一帧。旧判据 `dur + (0.35 + 0.4 + 0.3)` 多显示 0.4 秒 ✗（与模板/CPU 参考同形）。
/// 元素（§5）在 `t` 时刻的可见集合。
///
/// ⚠️ **`kind === "sticker"` 的元素直接复用 [`stickersAt`]**：把这一类的形状换成贴纸
/// （关键词在 `text` 里，搬到 `keyword`）再走同一条求值 —— 弹出/上浮/淡出的时序
/// 在四处实现里逐值对齐过（模板 canvas、模板 DOM、本文件、`timeline.rs`），
/// 重写一份必然漂移，而漂移的症状是"预览与成片不一样"。
///
/// ⚠️⚠️ **`kind === "text"` 现在也画了**（2026-10-12 接通，§5 E3 预览侧）。
///
/// 背景（为什么以前留空、为什么现在必须补上）：旧注释写的是「故意不近似」
/// + 「转译器会在报告里明说这条，所以这里静默是对的」。**这两条都已经不成立**：
/// ① 排版通道**早就实现**了（`process` Step 1d3 `layout_text_elements`：
///    把 `x`/`y` 百分比与**解析后的样式串**写回事件）；
/// ② 转译器那条 `drop()` 也**换成了真实现**（独立 ASS 资产 + 单层字幕轨）——
///    真跑出片里文字是**真的画出来的**（A/B 帧已验：第 4s 有那行字、第 1.5s 没有）。
/// ⇒ 于是预览与成片**真的分叉**：成片有字、预览没有，**而且不报错**。
///
/// ★ **这里不"近似"任何东西** —— 落点与样式都是 `process` 写好的，预览只是**读**
///   （`entry.textX/textY/textStyle`，见 `buildIndex` 的 `kind === 'text'` 分支）。
///   所以原来那句"故意不近似"的顾虑在这里**根本不适用**。
///   样式串读不出时**不画**（不自己编一套兜底数值去掩盖"内部不一致"）。
export function getElements(idx, t) {
  const out = []
  for (const e of idx.element || []) {
    // ⚠️ 载荷在 **`e.ev`** 上，不在 entry 自己身上 —— `buildIndex`（:371）造的是
    // `{ time, duration, ev }`。这里曾经读 `e.kind` ⇒ 恒 `undefined` ⇒ `kind !== 'sticker'`
    // 恒真 ⇒ **整条 element 通道一个都画不出来**（静默：既不报错、也不画）。
    // ⚠️ 同函数里 `e.time` / `e.duration`（下面两处）**是对的**（那两个字段确实在 entry 上，
    //    `stickersAt` 同口径）—— **所以绝不能全局把 `e.` 换成 `e.ev.`**：
    //    那会把本来能用的贴纸求值弄坏。逐行判断，这就是"两错两对"。
    const kind = String(e.ev?.kind ?? '').trim().toLowerCase()
    // §5 E3 预览侧：文字元素走**另一条**形状（它没有图源、有正文与样式）。
    // ⚠️ 排在贴纸之前判：两者都来自 `idx.element`，但载荷完全不同。
    if (kind === 'text') {
      const et = e.time, dur = e.duration || 2.5
      if (t < et || t > et + dur) continue
      // ⚠️ 样式串读不出（`process` 没跑过 / 串坏了）⇒ **不画**，
      //    不在这里编一套兜底数值 —— 那会掩盖"内部不一致"（看着画对了、其实与成片不同）。
      if (!e.textStyle || !String(e.textBody || '').trim()) continue
      if (e.textX === null || e.textX === undefined || e.textY === null || e.textY === undefined) continue
      // 与贴纸同一条进出场曲线（弹出/上浮/淡出），保证同一屏里的元素观感一致。
      const fadeOut = Math.min(0.3, dur)
      if (!(fadeOut > 0)) continue
      const k = Math.min(1, Math.max(0, dur - fadeOut) / (0.35 + 0.4))
      const popIn = 0.35 * k, floatUp = 0.4 * k, fadeStart = dur - fadeOut
      let opacity = 0, scale = 1, rot = 0, yOff = 0
      if (t - et < popIn) {
        const p = (t - et) / popIn
        const bp = E.backOut(p, 3)
        scale = 0.3 + 0.7 * bp
        rot = -15 + 15 * bp
        opacity = E.pow2Out(p)
      } else if (t < et + popIn + floatUp) {
        const p = (t - et - popIn) / floatUp
        opacity = 1; yOff = -stickerFloatUp(idx.cv) * p
      } else if (t < et + fadeStart) {
        opacity = 1; yOff = -stickerFloatUp(idx.cv)
      } else {
        const p = (t - fadeStart) / fadeOut
        opacity = Math.max(0, 1 - p)
        yOff = -stickerFloatUp(idx.cv)
      }
      // ---- §5 E4a 动效：T1 弹入 / T4 抖动 / T6 底衬 / T2 打字机 ----
      //
      // ⚠️ **全部与出片侧同一条式子**（Dhampir `text_motion.rs`，转译器逐字照抄那些名字）：
      //    预览这边是**第三份**实现 ⇒ 三份都得同值，否则症状是
      //    "预览里弹得挺好、成片里不弹"（或反过来），而且**两边各自看都正常**。
      const mo = e.textMotion
      const local = t - et
      // T1 弹入：⚠️ **窗口过了就恒为 1.0，不是停在 `peak`** ——
      //   `back_out3(1) == 1` ⇒ 曲线终点是 `peak`；要的是"弹过头再回落"，
      //   写成"窗口外给 peak"会让字**一直大 15%**，看着像"字号配错了"。
      // ⚠️ 只有 `scaleInMs > 0` 才开（另两个端点只是端点）。
      let motionScale = 1
      // ---- 心跳（双峰）：**独立于 T1 单峰那条路**（2026-10-14）----
      //
      // ⚠ 单峰曲线（`backOut3`）**数学上表达不了双峰**（见 `heartbeatScaleAt` 的文档），
      //   所以这档**不走** `scaleInMs` 那个窗口，而是走自己的周期函数。
      // ⚠ 边界：`local >= 0`（事件起点之前不跳）—— 与 T1 同一条。
      if (mo && mo.heartbeatAmp > 1 && local >= 0) {
        motionScale = heartbeatScaleAt(
          local * 1000, mo.heartbeatPeaksMs, mo.heartbeatAmp, mo.heartbeatCycleMs,
        )
      } else if (mo && mo.scaleInMs > 0 && local >= 0) {
        const win = mo.scaleInMs / 1000
        if (local < win) {
          motionScale = mo.scaleInFrom + (mo.scaleInPeak - mo.scaleInFrom) * backOut3(local / win)
        }
      }
      // T4 抖动：纯 `t` 的函数（无累积状态 ⇒ 逐帧可复现）。
      let shakeX = 0, shakeY = 0
      if (mo && mo.shakePx > 0) {
        ;[shakeX, shakeY] = textShakePx(local, mo.shakePx, mo.shakeHz, 0)
      }
      // T6 底衬：没颜色就**一个像素都不多画**（`padPx` 读都不读 —— 单向关系）。
      // ⚠️ alpha 会**乘上这一帧的不透明度**，所以这里**不在这里再乘一次淡入**（那会淡两次）。
      const plate = mo && mo.plateColor ? mo.plateColor : null
      // ---- T7 摇摆（P1，2026-10-13）：预览侧**按帧**求角度 ----
      //
      // ⚠️⚠️ **这段以前不存在**，是"配了摇摆、成片在摆、预览纹丝不动"的根因。
      //    预览读的是 `[elements]` 的两个旋钮（与转译器同一个来源），
      //    用 `swayDegAt` 在 `local` 秒处求角度，与转译器写出的关键帧**同值**。
      //    `local` 与上面弹入/抖动**共用同一个 `t - et`** ⇒ 三条动效同一时间基
      //    （与底座 `frame.saturating_sub(element.start)` 同口径）。
      let swayDeg = 0
      if (mo && mo.swayDeg) {
        // ⚠️ 必须把**这条元素自己的帧跨度**传进去（`dur` 秒 × fps）——
        //    转译器的收尾键落在 `span - 1`，不传就复现不了"最后收回到 0"那一段。
        swayDeg = swayDegAt(local, mo.swayDeg, mo.swayMs, idx.fps, dur * idx.fps)
      }
      // T2 打字机：逐字显示。⚠️ 与出片侧同口径 —— 按**总时长**均分，
      //   到点前只显示 `floor(进度 × 字数)` 个字。**不拆 cue**，直接在求值期截断
      //   （出片那边必须拆 cue 是因为底座要按帧画；预览这里一次求值就够了）。
      let shownText = String(e.textBody)
      if (mo && mo.typewriter && shownText.length > 1) {
        const p = dur > 0 ? Math.min(1, Math.max(0, local / dur)) : 1
        const n = Math.floor(p * shownText.length)
        shownText = shownText.slice(0, Math.max(0, n))
      }
      out.push({
        // `text` 支**没有 `src`** —— 消费方按这个字段分流（有 src 画图、没有则画字）
        src: '',
        isText: true,
        text: shownText,
        style: e.textStyle,
        xPct: e.textX,
        yPct: e.textY,
        pos: e.textPos,
        opacity,
        // ⚠️ 动效缩放**乘在**进出场缩放上（不是一个替代另一个）—— 与底座
        //    `line.scale × enter_scale × cue.scale` 的乘法合成同口径。
        scale: scale * motionScale,
        rot,
        yOff,
        shakeX,
        shakeY,
        plate,
        // T7 摇摆：**与 `rot` 分开带出去**，不在这里相加。
        // ⚠️ 相加要发生在 `textTiltTransform` 里（那里才知道 `tiltDeg`）——
        //    在这儿加会漏掉"样式倾斜 + 摇摆"的合成（`chapter`/`comment` 两档
        //    `tiltDeg` 是 0，相加与否看不出差别，**正好试不出来**，见 check-text-tilt 的教训）。
        swayDeg,
        // ---- 39 档（2026-10-13）：动效档名要**跟着元素走** ----
        //
        // ⚠️ `textTiltTransform` 读 `animName` 去查 `tilt_*` / `slam_squash`
        //    那两个**静态**值（它们不是时间函数，所以不能只摊进 `textMotion`）。
        //    模板那边是 `textTiltTransform(tx)` 整对象传的 ⇒ 这里带了就通了。
        animName: e.animName || '',
        // ⚠️⚠️ **排版/过渡/动效三条档名要分开带出去**（2026-10-14）。
        //
        //    上面那个 `animName` 是**兼容字段**（"那一档"，排版轴优先），
        //    `textTiltTransform` 靠它能拿到 `tiltDeg` —— 但**这不够**：
        //    `vtext` 的 `vertical` / `fontWeight` 与 `perchar_*` 的 `perChar`
        //    都要按**各自那条轴**去查表。只给一个合并字段 ⇒ 用户选「竖排」
        //    在元素上**横着**（成片里竖的），因为组件拿 `animName` 查不到
        //    "这一条排版轴是 `vtext`"这件事的**轴身份**。
        //    （`PolishPreview.vue` 的 `texts` computed 就按这三个字段消费。）
        styleAnimName: e.styleAnimName || '',
        effectAnimName: e.effectAnimName || '',
        transitionAnimName: e.transitionAnimName || '',
        // ---- P4 补：逐字（2026-10-13）----
        //
        // 消费方拿这两样去算每个字自己的变换：
        //   * `perChar` —— 这一档的 `PerCharSpec`（没逐字档时是 `undefined`）；
        //   * `perCharFrame` —— **这一条元素已经播了多少帧**（本地帧）。
        //
        // ⚠️⚠️ **必须换算成"帧"，不是秒**：`charTransformAt` 的第四参是
        //    `localFrame`，与底座 `text_perchar_anim.rs::char_transform_at` 同口径
        //    （底座那边本来就是帧）。喂秒会让 `per_char_frames=8` 的档
        //    **几百帧才走完**（8 秒）—— 看着像"慢动作/几乎不动"。
        // ⚠️ 与上面 `swayDegAt` 同一个换算（`local * fps`），两处别写成不同的式子。
        // ⚠️ `idx.fps` 可能是 0（工程没给 fps）⇒ 那时恒 0，逐字停在**起始值**
        //    （`perCharSpans` 会因 `perCharFps > 0` 不成立而直接返回 `null`）。
        perChar: e.perChar,
        perCharFrame: local * idx.fps,
        perCharFps: idx.fps,
        // ⚠️ 一并带出去：判据要用它验证"逐条预设真的覆盖了全局"，
        //    而 `motionScale` / `swayDeg` 是**已求值的标量**，看不出参数来源。
        textMotion: mo
      })
      continue
    }
    if (kind !== 'sticker') continue
    const et = e.time, dur = e.duration || 2.5
    if (t < et || t > et + dur) continue
    const fadeOut = Math.min(0.3, dur)
    if (!(fadeOut > 0)) continue
    const k = Math.min(1, Math.max(0, dur - fadeOut) / (0.35 + 0.4))
    const popIn = 0.35 * k, floatUp = 0.4 * k, fadeStart = dur - fadeOut
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
    } else if (t < et + fadeStart) {
      opacity = 1; scale = 1; rot = 0
      yOff = -stickerFloatUp(idx.cv)
    } else {
      const p = (t - fadeStart) / fadeOut
      opacity = Math.max(0, 1 - p)
      scale = 1; rot = 0
      yOff = -stickerFloatUp(idx.cv)
    }
    // ⚠️ 同上：`text` 也在 **`e.ev`** 上。这里曾经读 `e.text` ⇒ 恒 `undefined` ⇒
    // `src` 恒空串 ⇒ **画出来了但是空白的**（比"画不出来"更隐蔽：它有 opacity、有 scale，
    // 只是没图）。只修上面那个 `continue` 而不修这里，症状就从"看不见"变成"看不见但占位"。
    //
    // ⚠️⚠️ **第三处缺陷**：曾经写的是 `(idx.stickerSrc || (() => ''))(...)` ——
    //    但 **`idx` 上从来没有 `stickerSrc`**：`buildIndex` 只把解析结果落到 **`entry.src`**
    //    （见上面 `type === 'sticker'` 分支）。⇒ 那个 `||` 兜底**恒生效**、`src` **恒为空串**，
    //    于是**即使 (a)(b) 都修好，贴纸照样是空的**。
    //    这里改成与 `type === 'sticker'` 同口径：**读 `e.src`**（索引期已经解析好的 URL）。
    //    ⇒ 这就是"两处错误"背后的**第三处**：一个**恒真的兜底**，它让前两处的修复看起来无效。
    out.push({ src: e.src || '', opacity, scale, rot, yOff })
  }
  return out
}

export function stickersAt(idx, t) {
  const out = []
  for (const e of idx.sticker) {
    const et = e.time, dur = e.duration || 2.5
    if (t < et || t > et + dur) continue
    const fadeOut = Math.min(0.3, dur)
    if (!(fadeOut > 0)) continue   // 退化时长（<= 0）没有可见窗口
    // 装不下 `popIn 0.35 + floatUp 0.4 + fadeOut 0.3` 时**优先保淡出**：
    // 前两段按可用时长等比压缩，淡出起点不得早于 0。
    const k = Math.min(1, Math.max(0, dur - fadeOut) / (0.35 + 0.4))
    const popIn = 0.35 * k, floatUp = 0.4 * k, fadeStart = dur - fadeOut
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
    } else if (t < et + fadeStart) {
      opacity = 1; yOff = -stickerFloatUp(idx.cv)
    } else {
      const p = (t - et - fadeStart) / fadeOut
      // `Math.max(0, …)` 只吃掉上界那一帧的浮点尾数：出点必须**恰好 0**。
      opacity = Math.max(0, 1 - p)
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

// ── §20 `noise`（真黑白噪点）────────────────────────────────────────────
//
// ⚠️ **为什么这两条住在 `engine.js`**：`noise` 的 `dom_preview` 通道按 `spec/effects.json`
//    就定义在这里（`channels.dom_preview` = "`engine.js` 的 `get<Camel>(id)`"）。
//    出片那条腿（`templates/index.html`）跑在**无打包器的浏览器**里，**import 不了本模块**
//    ⇒ 两份物理拷贝是**必然的**。两份**逐字相同**由 `scripts/check-noise-parity.mjs` 钉住
//    （逐 token 比对，注释除外）。
//
// ⚠️ **帧号必须来自"已经画到第几帧"**，**不是** `t × fps` —— 出片模板的同一条注释写明了：
//    自己乘一个 fps 会与 canvas 路径分叉（两边用不同的噪声图 = 又一次"预览 ≠ 成片"）。
//    预览侧那个量是 `PolishPreview.vue` 的 `dhampirFrameShown`。

/// 噪声贴图缓存：`'f<frame>'` → 64×64 canvas。**只留最近 8 张**（长片逐帧缓存会吃满内存）。
const __NOISE_TILES = {}

/// 64×64 的**灰度**噪声贴图（与出片模板 `noiseTile` 逐字相同）。
///
/// 中灰 128 抖动：`128 + (v - 0.5) * 255` —— 与底座的 `(n - 0.5) * 255` 同义。
/// ⚠️ 三通道写成**同一个 `g`**（用户口径"黑白噪点"不是彩色噪点），alpha 恒 255（不透光斑）。
export function noiseTile(frame) {
  var key = 'f' + frame;
  if (__NOISE_TILES[key]) return __NOISE_TILES[key];
  var S = 64;
  var cv = document.createElement('canvas');
  cv.width = S; cv.height = S;
  var c2 = cv.getContext('2d');
  var img = c2.createImageData(S, S);
  for (var y = 0; y < S; y++) {
    for (var x = 0; x < S; x++) {
      var v = noiseHash(x, y, frame, 7);
      var g = Math.max(0, Math.min(255, Math.round(128 + (v - 0.5) * 255)));
      var i = (y * S + x) * 4;
      img.data[i] = g; img.data[i + 1] = g; img.data[i + 2] = g;
      img.data[i + 3] = 255;
    }
  }
  c2.putImageData(img, 0, 0);
  __NOISE_TILES[key] = cv;
  // 只留最近 8 张
  var keys = Object.keys(__NOISE_TILES);
  if (keys.length > 8) { delete __NOISE_TILES[keys[0]]; }
  return cv;
}

/// 整数哈希（与底座 `color_mask.wgsl::hash_u32` 同义、与出片模板逐字相同）。
/// 纯函数 ⇒ **同一帧可复现**（三腿一致的那条不变量就靠它）。
export function noiseHash(x, y, frame, seed) {
  var v = (Math.imul(x, 73856093) ^ Math.imul(y, 19349663)
         ^ Math.imul(frame, 83492791) ^ Math.imul(seed, 2654435761)) >>> 0;
  v = (v ^ (v >>> 16)) >>> 0;
  v = Math.imul(v, 0x7feb352d) >>> 0;
  v = (v ^ (v >>> 15)) >>> 0;
  v = Math.imul(v, 0x846ca68b) >>> 0;
  v = (v ^ (v >>> 16)) >>> 0;
  return (v & 0x00ffffff) / 16777216;
}
