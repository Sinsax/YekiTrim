<template>
  <div class="polish-preview">
    <div ref="stageRef" class="stage" :style="stageStyle">
      <video v-if="bgOn && videoSrc" ref="bgRef" class="bg-video" :src="videoSrc" muted playsinline preload="auto"></video>
      <div class="video-wrap" :class="stageClass">
        <div class="fit-box">
          <video ref="videoRef" class="source-video" :src="videoSrc" playsinline preload="auto"
                 :style="videoStyle"
                 @loadedmetadata="onMeta" @seeked="onSeeked" @play="setPlaying(true)" @pause="setPlaying(false)" @ended="setPlaying(false)"></video>
        </div>
      </div>
      <div class="corner-glow"><div class="gb"></div></div>
      <div class="fx fx-vignette" :style="vignetteStyle"></div>
      <div class="fx fx-noise" :style="{ opacity: noiseOp }"></div>
      <div class="fx fx-flash" :style="flashStyle"></div>
      <div class="fx fx-hue" :style="hueStyle"></div>
      <div class="fx fx-color" :style="colorOverlayStyle"></div>

      <div class="overlay-space" :style="{ transform: 'translate(-50%,-50%) scale(' + overlayScale + ')' }">
        <div class="sticker-layer">
          <div v-for="(st, i) in stickers" :key="'s' + i" class="sticker-el"
               :style="{ left: st.x + 'px', top: st.y + 'px', opacity: st.opacity, transform: 'rotate(' + st.rot + 'deg) scale(' + st.scale + ')' }">
            <img v-if="st.src" :src="st.src" alt="" @error="onStickerError(st)">
            <span v-else class="sticker-emoji">{{ st.emoji }}</span>
          </div>
        </div>
        <div class="danmaku-layer" :style="dmLayerStyle">
          <div v-for="(d, i) in dmRows" :key="'d' + i" class="dm" :style="{ left: d.x + 'px', top: d.y + 'px', opacity: d.opacity }">{{ d.text }}</div>
        </div>
        <div class="captions" :style="capLayerStyle">
          <div v-for="(c, i) in capRowsWrapped" :key="'c' + i" class="cap"
               :style="{ opacity: c.opacity, transform: 'translateY(' + c.yOff + 'px) scale(' + c.scale + ')',
                         fontSize: (subStyle.size || 72) * c.fontScale + 'px',
                         WebkitTextStrokeWidth: capStrokeW * c.fontScale + 'px' }">
            <span v-for="(line, li) in c.lines" :key="li" class="cap-line">
              <span v-for="(p, j) in line" :key="j" :class="{ hl: p.hl }">{{ p.text }}</span>
            </span>
          </div>
        </div>
      </div>
      <div class="intro-black" :style="{ opacity: introBlack }"></div>
      <!-- 框选布局：在此模式下视频运镜暂停（缩放复位），直接在整帧上拖出精确矩形 -->
      <div v-if="selectMode" class="sel-layer" @pointerdown.prevent="startSel('new', $event)">
        <div class="sel-box" :style="selBoxStyle" @pointerdown.prevent.stop="startSel('move', $event)">
          <span class="sel-tag">{{ selectLabel || '框选' }}</span>
          <span v-for="h in SEL_HANDLES" :key="h" class="sel-h" :class="'h-' + h"
                @pointerdown.prevent.stop="startSel(h, $event)"></span>
        </div>
      </div>
      <!-- 点选落点（贴纸「固定」模式）：与框选同一套交互，只是落一个点 —— 点哪儿落哪儿 -->
      <div v-else-if="pointMode" class="point-layer" @pointerdown.prevent="pickPoint($event)">
        <span class="point-tag">{{ pointLabel || '点选落点' }}</span>
        <!-- 画出**贴纸真实占位**（280×短边倍率的盒子，**以锚点为中心**，并做出片同样的出框 clamp），
             不是画一个点 —— 固定落点的含义是"贴纸中心落在这一点"（config.rs 的 position 口径） -->
        <span class="point-box" :style="pointBoxStyle"><span class="point-dot"></span></span>
      </div>
      <!-- 真渲染帧（wgpu 出片效果）叠加层 -->
      <img v-if="realFrameUrl" class="real-frame" :src="realFrameUrl" alt="">
      <div v-if="realLoading" class="real-loading"><span class="spinner"></span></div>
    </div>

    <div class="preview-controls">
      <button class="mini-btn" @click="togglePlay">
        <Icon :name="playing ? 'pause' : 'play'" :size="12" /> {{ playing ? '暂停' : '播放' }}
      </button>
      <input class="flex-1" type="range" :min="0" :max="1000" :value="progress" @input="onSeekInput">
      <span class="muted small mono time-label">{{ fmt(current) }} / {{ fmt(duration) }}</span>
      <button class="mini-btn" :class="{ selected: silenced }" @click="toggleMute" :title="silenced ? '取消静音' : '静音'">
        <Icon :name="silenced ? 'volume-x' : 'volume'" :size="13" />
      </button>
      <input class="vol" type="range" min="0" max="1" step="0.05" :value="volume" @input="onVolume" title="音量（视频 + 音效）">
      <!-- 真渲染（wgpu）：服务端未启用该 feature 时整个入口隐藏；
           该功能暂时搁置，日后开发时放开这里即可（下方 toggleReal/scheduleRealFrame 的逻辑保留） -->
      <button v-if="wgpuRender" class="mini-btn" :class="{ selected: realRender }" @click="toggleReal" title="暂停/拖动时显示 wgpu 真实渲染帧">
        <Icon name="film" :size="12" /> 真渲染
      </button>
      <button class="mini-btn" @click="fullscreen" title="全屏"><Icon name="maximize" :size="13" /></button>
    </div>
  </div>
</template>

<script setup>
// 瘦播放器：数据由父级（PolishView）一次性下发，特效状态全部来自 polish/engine.js
// （与 templates/index.html Canvas 出片路径一致），rAF 只读 video.currentTime 作为主时钟。
import { ref, computed, watch, onMounted, onUnmounted } from 'vue'
import Icon from './Icon.vue'
import { toast } from '../stores/toast'
import { csrfHeaders } from '../api/client'
import { addClient } from '../stores/log'
import { clipMediaUrl } from '../utils'
import { fontUrl } from '../api/polish'
import { buildIndex, videoFxAt, handheldSway, getFlash, getHueShiftFlash, getNoiseOpacity, getVignette, getOverlayState, gradientCss, introBlackAt, subsAt, danmakuAt, stickersAt, wrapCaptionParts, capMeasure } from '../polish/engine'
import { canvasOf, stickerScale } from '../polish/canvas.js'
import { createSfxPlayer } from '../polish/audio'
import { ensureFontFace } from '../polish/fonts'
import init, { init_renderer } from '../wasm/vtrim_wasm.js'

const props = defineProps({
  pid: { type: [String, Number], required: true },
  dir: { type: String, required: true },
  config: { type: Object, default: null },
  subtitles: { type: Array, default: () => [] },
  danmaku: { type: Array, default: () => [] },
  metaDuration: { type: Number, default: 0 },
  // 服务端 /api/health 的 wgpu_render：false 时真渲染（单帧回退）不可用
  wgpuRender: { type: Boolean, default: false },
  // 框选布局模式：叠加选框 + 暂停运镜（缩放复位），选框坐标为整帧百分比
  selectMode: { type: Boolean, default: false },
  selectRect: { type: Object, default: null },
  selectLabel: { type: String, default: '' },
  // 点选落点模式（贴纸「固定」）：点哪儿落哪儿，坐标为整帧百分比。
  // 与 selectMode 互斥（框选优先）—— 两者都是"在预览上取坐标"，没必要同时开。
  pointMode: { type: Boolean, default: false },
  point: { type: Object, default: null },
  pointLabel: { type: String, default: '' }
})
const emit = defineEmits(['time', 'update:selectRect', 'update:point', 'playing'])

const stageRef = ref(null)
const videoRef = ref(null)
const bgRef = ref(null)
const current = ref(0)
const duration = ref(0)
const playing = ref(false)
const muted = ref(false)
const volume = ref(1)
/** 非零音量的记忆值：从 0 恢复声音时用它（localStorage 里存的就是 masterVol） */
let lastVol = 1
/** 按钮/图标状态：静音或音量为 0 都算「没声音」 */
const silenced = computed(() => muted.value || volume.value <= 0)
const overlayScale = ref(1)
const stickerFailed = ref({})
let rafId = 0
let ro = null

const player = createSfxPlayer({
  resolveUrl: (name) => clipMediaUrl(props.pid, props.dir, 'sfx/' + name + '.mp3')
})

const videoSrc = computed(() => clipMediaUrl(props.pid, props.dir, props.config?.source?.video || 'clip.mp4'))
const muteRanges = computed(() => (props.config?.mute_ranges || [])
  .map(r => ({ start: Number(r.start) || 0, end: Number(r.end) || 0 }))
  .filter(r => r.end - r.start > 0.001)
  .sort((a, b) => a.start - b.start))
function muteRangeAt(t) {
  return muteRanges.value.find(r => t >= r.start && t < r.end) || null
}
const vertical = computed(() => !!props.config?.vertical)

// ---- 输出画布（`_canvas` 契约，SSOT 在 ./polish/canvas.js）----
// 横屏 1920×1080 → k=kh=1，一切几何与改造前逐像素相同；竖屏 1080×1920 时
// 舞台比例 / 虚拟 overlay / 弹幕带 / 字幕带 / 贴纸盒子全部随 k/kh 缩放。
const cv = computed(() => canvasOf(props.config))

// ---- 画面构成（位置 / 背景）----
// 取值来自 `_canvas.stage` —— Rust `ProjectFile::resolve_stage` 唯一推导，前端不重算（两边不可能不一致）。
// 拿不到 `_canvas`（老接口 / 非 preview 请求）时退回改造前的竖屏规则，行为与以前一致。
const FALLBACK_STAGE = {
  axis: 'fill', object_fit: 'cover', position: 'center', bg: 'none',
  bg_color: '#000000', blur_px: 28, blur_img_scale: 1.187, scale: 1, padding: 0
}
const stageFit = computed(() => {
  const s = props.config?._canvas?.stage
  if (s) return s
  return vertical.value
    ? { ...FALLBACK_STAGE, axis: 'x', object_fit: 'contain', bg: 'blur',
        position: props.config?.vertical_position || 'left' }
    : FALLBACK_STAGE
})
/** 背景层（当前帧模糊）只在 bg=blur 时需要；black / color 走舞台底色 */
const bgOn = computed(() => stageFit.value.bg === 'blur')
const stageClass = computed(() => {
  const s = stageFit.value
  const axis = s.axis || 'fill'
  return axis === 'fill' ? ['axis-fill'] : ['axis-' + axis, 'stage-' + (s.position || 'center')]
})
/** fit 作用域的 CSS 变量：放在 `.stage` 上，`.video-wrap` / `.fit-box` / `.bg-video` 全部继承 */
const stageStyle = computed(() => {
  const c = props.config?._canvas || {}
  const s = stageFit.value
  const src = c.src || {}
  const v = cv.value
  return {
    background: s.bg === 'color' ? (s.bg_color || '#000000') : '#000',
    '--src-ar': (src.width > 0 && src.height > 0) ? (src.width + ' / ' + src.height) : '16 / 9',
    '--stage-pad': ((Number(s.padding) || 0) / (c.width || 1920) * 100) + '%',
    '--stage-scale': String(s.scale || 1),
    '--bg-blur': (s.blur_px || 28) + 'px',
    '--bg-img-scale': String(s.blur_img_scale || 1.187),
    // 画布驱动的舞台比例 + 虚拟 overlay / 弹幕带 / 字幕带 / 贴纸盒子尺寸
    '--stage-ar': v.W + ' / ' + v.H,
    // 宽 = 高度预算(clamp) × W/H。⚠️ 必须是 **W/H**（横屏 1.7778，与 P3 前的硬编码 *16/9 一致）：
    // 曾经错写成 H/W，横屏舞台被缩到 1/3 高（用户报"横屏被改错"）。竖屏 W/H=0.5625 →
    // 宽 = clamp×0.5625，高 = 宽÷(9/16) = clamp —— **竖屏与横屏同高**（用户要求）。
    '--stage-ratio': String(v.W / v.H),
    '--ov-w': v.W + 'px',
    '--ov-h': v.H + 'px',
    '--dm-h': (540 * v.k) + 'px',
    '--sticker-s': (280 * stickerScale(v)) + 'px',
    '--sticker-emoji': (200 * stickerScale(v)) + 'px',
    '--cap-bottom': (80 * v.kh) + 'px',
    '--cap-left': (120 * v.k) + 'px',
    '--cap-w': (v.W - 240 * v.k) + 'px'
  }
})

const stickerMap = computed(() => props.config?._assets?.sticker_map || {})
function stickerSrc(name) {
  if (stickerFailed.value[name]) return ''
  const p = stickerMap.value[name]
  return p ? clipMediaUrl(props.pid, props.dir, p) : ''
}

const effDuration = computed(() => duration.value || props.metaDuration || 0)

/**
 * 喂给索引的配置：点选编辑态里把**草稿落点**临时顶上去，让贴纸本身跟着动
 * （所见即所得；不写盘，保存由父级负责）。
 *
 * 只覆盖 `_canvas.sticker.position`、不动 `position_mode` —— draft 只在「固定」模式里存在
 * （父级 `startPointEdit` 保证了这点），所以这不改变任何语义，只是把"已保存的那一版"
 * 换成"草稿那一版"。旧对象原样返回，避免无谓重建索引。
 */
const indexConfig = computed(() => {
  if (!props.pointMode || !props.point || !props.config) return props.config
  const nx = Number(props.point.x), ny = Number(props.point.y)
  if (!Number.isFinite(nx) || !Number.isFinite(ny)) return props.config
  const cl = (v) => Math.round(Math.min(100, Math.max(0, v)))
  const cv = props.config._canvas || {}
  const st = cv.sticker || {}
  const next = cl(nx) + '% ' + cl(ny) + '%'
  if (String(st.position || '') === next) return props.config
  return { ...props.config, _canvas: { ...cv, sticker: { ...st, position: next } } }
})

const idx = computed(() => buildIndex({
  config: indexConfig.value,
  subtitles: props.subtitles,
  danmaku: props.danmaku,
  duration: effDuration.value,
  stickerSrc
}))

// ---- 逐帧状态（rAF 更新 current 触发重算）----
const vfx = computed(() => videoFxAt(idx.value, current.value))
const flash = computed(() => getFlash(idx.value, current.value))
const hueFlash = computed(() => getHueShiftFlash(idx.value, current.value))
const noiseOp = computed(() => getNoiseOpacity(idx.value, current.value))
const vignette = computed(() => getVignette(idx.value, current.value))
const colorOverlay = computed(() => getOverlayState(idx.value, current.value))
const introBlack = computed(() => introBlackAt(current.value))
const capRows = computed(() => subsAt(idx.value, current.value))

// A5 字幕折行：与出片（templates/index.html renderFrame）同一套 wrapCaptionParts
// —— 安全宽 = 画布宽 * 0.875、最多 3 行、装不下缩字号（下限 0.7）。
// 行内 white-space:nowrap 禁掉浏览器自动换行，预览与出片断行位置一致。
const capFontSpec = computed(() => '700 ' + (subStyle.value.size || 72) + 'px "'
  + (subStyle.value.font || 'LXGW WenKai') + '","Microsoft YaHei","Noto Sans SC",sans-serif')
const capStrokeW = computed(() => parseFloat(subStyle.value.stroke) || 12)
const capRowsWrapped = computed(() => capRows.value.map((c) => {
  const w = wrapCaptionParts(c.parts, cv.value.W * 0.875, (tx) => capMeasure(tx, capFontSpec.value))
  return { ...c, lines: w.lines, fontScale: w.scale }
}))
const dmRows = computed(() => danmakuAt(idx.value, current.value))
const stickers = computed(() => stickersAt(idx.value, current.value))
const progress = computed(() => duration.value ? Math.round(current.value / duration.value * 1000) : 0)
const fmt = (s) => {
  const n = Number(s) || 0
  const m = Math.floor(n / 60)
  const sec = n - m * 60
  const secStr = sec.toFixed(2).padStart(5, '0')
  return String(m).padStart(2, '0') + ':' + secStr
}

const subStyle = computed(() => idx.value.subStyle || {})
const dmStyleCfg = computed(() => idx.value.dmStyle || {})

const videoStyle = computed(() => {
  // 内容层的 object-fit 与出片一致（axis=fill → cover；留边 → contain），由 `_canvas.stage` 决定
  const objectFit = stageFit.value.object_fit || 'cover'
  // 框选布局模式：运镜/缩放/抖动/滤镜全部复位，让选框直接对应整帧真实位置
  if (props.selectMode) return { transform: 'none', transformOrigin: 'center', filter: 'none', objectFit }
  const f = vfx.value
  const t = []
  if (f.shake.x || f.shake.y) t.push(`translate(${f.shake.x}px, ${f.shake.y}px)`)
  if (f.stutter) t.push(`translate(${f.stutter.x}px, ${f.stutter.y}px)`)
  if (f.split && f.split.skewX) t.push(`skewX(${f.split.skewX / 100}deg)`)
  // 手持摇摆（运镜的附加变种）：位移在屏幕空间，旋转/呼吸缩放绕同一个不动点，
  // 所以必须排在 scale 之前 —— transform 列表从右往左作用，而相机块（含运镜缩放）
  // 在出片模板里是最内层（最后 apply 到画面上的那一段）。
  const sway = handheldSway(current.value, f.camera.handheld)
  if (sway.dx || sway.dy) t.push(`translate(${sway.dx}px, ${sway.dy}px)`)
  if (sway.rot) t.push(`rotate(${sway.rot}deg)`)
  // 判据是「缩放不等于 1」而不是「大于 1」：画中画档位（zoom<1）也是合法运镜
  // 摇摆的呼吸缩放与运镜缩放都是绕不动点的缩放，合成一个 scale（amp=0 时 sway.scale 恒为 1）
  const camScale = f.zoom * sway.scale
  if (Math.abs(camScale - 1) > 0.001) t.push(`scale(${camScale})`)
  const filter = f.blur > 0 ? `blur(${f.blur}px)` : ''
  return {
    transform: t.join(' ') || 'none',
    transformOrigin: `${f.camera.ox / cv.value.W * 100}% ${f.camera.oy / cv.value.H * 100}%`,
    filter: [filter, f.colorShift].filter(Boolean).join(' ') || 'none',
    objectFit
  }
})
const flashStyle = computed(() => ({ opacity: flash.value.opacity, background: flash.value.color }))
const hueStyle = computed(() => hueFlash.value ? { opacity: hueFlash.value.opacity, background: hueFlash.value.color } : { opacity: 0 })
const vignetteStyle = computed(() => ({
  opacity: vignette.value,
  background: 'radial-gradient(ellipse at center,transparent 55%,rgba(0,0,0,0.5) 100%)'
}))
const colorOverlayStyle = computed(() => {
  const ov = colorOverlay.value
  if (!ov) return { opacity: 0 }
  if (ov.type === 'color') return { opacity: ov.opacity, background: ov.color }
  // 手拼字符串曾漏掉收尾 ')' —— 整条 background 非法会被浏览器丢弃，
  // 表现是「设了色档但预览里什么都没有」。统一走 engine 的 gradientCss。
  return { opacity: ov.opacity, background: gradientCss(ov.colors, ov.shape) }
})

const capLayerStyle = computed(() => {
  const s = subStyle.value
  return {
    fontFamily: `"${s.font || 'LXGW WenKai'}","Microsoft YaHei","Noto Sans SC",sans-serif`,
    fontSize: (s.size || 72) + 'px',
    color: s.color || '#dcbda0',
    WebkitTextStroke: s.stroke || '12px #403c3b',
    '--hl-color': s.highlight_color || '#f56b41'
  }
})
const dmLayerStyle = computed(() => {
  const s = dmStyleCfg.value
  return {
    fontSize: (s.font_size || 42) + 'px',
    color: s.color || '#ffffff',
    WebkitTextStroke: s.stroke || '2px #000000'
  }
})

function onStickerError(st) {
  stickerFailed.value = { ...stickerFailed.value, [st.name]: true }
}

// ---- 播放控制 ----
function tick(ts) {
  const v = videoRef.value
  if (!v) return
  // 预览时跳过手动屏蔽区间
  if (!v.paused && !v.ended) {
    const r = muteRangeAt(v.currentTime)
    if (r) {
      const target = Math.min(v.duration || (r.end + 0.01), r.end + 0.01)
      v.currentTime = Math.max(target, r.end)
      current.value = v.currentTime
      if (bgOn.value && bgRef.value) bgRef.value.currentTime = v.currentTime
      rafId = requestAnimationFrame(tick)
      return
    }
  }
  current.value = v.currentTime
  if (wasmActive.value && wasmRenderer) {
    // wasm 真渲染也要驱动音效播放，避免开启真渲染后 sfx 不响
    player.sync(idx.value.soundEvents, v.currentTime)
    // wasm 真渲染较重，限制在约 30fps（33ms），避免每帧都跑完整 GPU 合成
    const now = ts || performance.now()
    if (now - lastWasmFrame >= 33) {
      lastWasmFrame = now
      try {
        wasmRenderer.render_frame(v.currentTime, v)
        emit('time', v.currentTime)
      } catch (e) {
        console.warn('wasm render_frame 失败，回退自绘', e)
        disableWasm()
      }
    }
    if (!v.paused && !v.ended) rafId = requestAnimationFrame(tick)
    return
  }
  player.sync(idx.value.soundEvents, v.currentTime)
  emit('time', v.currentTime)
  if (bgOn.value && bgRef.value && bgRef.value.currentTime &&
      Math.abs(bgRef.value.currentTime - v.currentTime) > 0.05) {
    bgRef.value.currentTime = v.currentTime
  }
  if (!v.paused && !v.ended) rafId = requestAnimationFrame(tick)
}

function togglePlay() {
  const v = videoRef.value
  if (!v) return
  if (v.paused) {
    if (bgOn.value && bgRef.value) bgRef.value.play().catch(() => {})
    v.play().catch(() => {})
    setPlaying(true)
    cancelAnimationFrame(rafId)
    rafId = requestAnimationFrame(tick)
  } else {
    v.pause()
    if (bgOn.value && bgRef.value) bgRef.value.pause()
    setPlaying(false)
    cancelAnimationFrame(rafId)
  }
}

/** 播放状态往外发一份：时间轴的"跟随播放头"只在**正在播放**时生效
 *  （用户要求：拖动/跳转不该自己动视角，只有播放时才跟）。 */
function setPlaying(v) {
  const on = !!v
  if (playing.value === on) return
  playing.value = on
  emit('playing', on)
}

function seekTo(t) {
  const v = videoRef.value
  if (!v) return
  v.currentTime = Number(t) || 0
  current.value = v.currentTime
  player.seek(current.value)
  if (bgOn.value && bgRef.value) bgRef.value.currentTime = v.currentTime
  if (duration.value) emit('time', v.currentTime)
}

function onSeekInput(e) {
  const p = Number(e.target.value) || 0
  if (videoRef.value && duration.value > 0) seekTo(duration.value * p / 1000)
}

function onMeta() {
  const v = videoRef.value
  if (!v) return
  duration.value = v.duration || 0
  // 切片段/换源后把音量与静音重新落到视频元素上（元素属性虽会保留，这里保证与控件状态一致）
  applyAudio()
  // seek 触发首帧解码，避免打开页面时舞台全黑
  if (v.currentTime === 0) v.currentTime = 0.001
  if (bgOn.value && bgRef.value) {
    if (bgRef.value.currentTime === 0) bgRef.value.currentTime = 0.001
    if (Math.abs((bgRef.value.currentTime || 0) - v.currentTime) > 0.05) {
      bgRef.value.currentTime = v.currentTime
    }
  }
}

// 任何来源的 seek（进度条/代码/原生控制条）都同步覆盖层时钟
function onSeeked() {
  const v = videoRef.value
  if (!v) return
  current.value = v.currentTime
  player.seek(v.currentTime)
  if (wasmActive.value && wasmRenderer) {
    try { wasmRenderer.render_frame(v.currentTime, v) } catch (e) { console.warn(e) }
  }
  if (bgOn.value && bgRef.value) bgRef.value.currentTime = v.currentTime
}

/**
 * 音量/静音同时作用在**视频本身**和 SFX 层。
 * 之前只调 SFX 播放器，视频里的说话声/音乐丝毫不受影响 —— 用户反馈的「音量控制不好使」就是这个：
 * 点静音、拉滑块都对真正在响的那一路没作用。
 */
function applyAudio() {
  const v = videoRef.value
  if (!v) return
  const n = Number(volume.value)
  v.volume = Number.isFinite(n) ? Math.min(1, Math.max(0, n)) : 0
  v.muted = !!muted.value
}

function toggleMute() {
  if (silenced.value) {
    // 当前没声音 → 恢复（音量被拉到 0 时抬回上次的非零值，否则按钮看着是「恢复了」却依旧静音）
    muted.value = false
    player.setMuted(false)
    if (volume.value <= 0) {
      volume.value = lastVol
      player.setMasterVolume(lastVol)
    }
  } else {
    muted.value = true
    player.setMuted(true)
  }
  applyAudio()
}

function onVolume(e) {
  const v = Number(e.target.value)
  volume.value = Number.isFinite(v) ? Math.min(1, Math.max(0, v)) : 0
  if (volume.value > 0) {
    lastVol = volume.value
    if (muted.value) { muted.value = false; player.setMuted(false) }
  }
  player.setMasterVolume(volume.value)
  applyAudio()
}
function fullscreen() {
  if (stageRef.value) stageRef.value.requestFullscreen?.()
}

// ---- 框选布局：整帧百分比矩形（拖框体 = 平移，拖 8 个角/边 = 缩放）----
const SEL_HANDLES = ['nw', 'n', 'ne', 'w', 'e', 'sw', 's', 'se']
const SEL_MIN = 2 // 最小边长（百分比），防止拖成一条线
let selDrag = null

const r2 = (v) => Math.round(v * 100) / 100

/** 选框兜底：越界裁剪 + 顺序化 + 最小尺寸（父级数据异常时不至于画出负宽盒子）。 */
function selNorm(r) {
  const n = (v, d) => { const x = parseFloat(v); return Number.isFinite(x) ? Math.min(100, Math.max(0, x)) : d }
  let x0 = n(r?.x0, 0), y0 = n(r?.y0, 0), x1 = n(r?.x1, 100), y1 = n(r?.y1, 100)
  if (x1 < x0) [x0, x1] = [x1, x0]
  if (y1 < y0) [y0, y1] = [y1, y0]
  if (x1 - x0 < 0.5) x1 = Math.min(100, x0 + 0.5)
  if (y1 - y0 < 0.5) y1 = Math.min(100, y0 + 0.5)
  return { x0, y0, x1, y1 }
}

const selBoxStyle = computed(() => {
  const r = selNorm(props.selectRect)
  return {
    left: r.x0 + '%',
    top: r.y0 + '%',
    width: (r.x1 - r.x0) + '%',
    height: (r.y1 - r.y0) + '%'
  }
})

/** 指针位置 → 舞台（= 画布整帧）百分比坐标。 */
function pctAt(e) {
  const el = stageRef.value
  if (!el) return null
  const b = el.getBoundingClientRect()
  if (!b.width || !b.height) return null
  return {
    x: Math.min(100, Math.max(0, ((e.clientX - b.left) / b.width) * 100)),
    y: Math.min(100, Math.max(0, ((e.clientY - b.top) / b.height) * 100))
  }
}

function startSel(handle, e) {
  const p = pctAt(e)
  if (!p) return
  const r0 = handle === 'new' ? { x0: p.x, y0: p.y, x1: p.x, y1: p.y } : selNorm(props.selectRect)
  selDrag = { handle, sx: p.x, sy: p.y, r0 }
  if (handle === 'new') emitRect(r0)
  window.addEventListener('pointermove', onSelMove)
  window.addEventListener('pointerup', endSel)
  window.addEventListener('pointercancel', endSel)
}

function emitRect(r) {
  emit('update:selectRect', { x0: r2(r.x0), y0: r2(r.y0), x1: r2(r.x1), y1: r2(r.y1) })
}

function onSelMove(e) {
  if (!selDrag) return
  // 指针在窗口外松开（收不到 pointerup）时用 buttons=0 兜底收尾，避免选框跟着鼠标跑
  if (e.buttons === 0) { endSel(); return }
  const p = pctAt(e)
  if (!p) return
  const { handle, sx, sy, r0 } = selDrag
  const dx = p.x - sx
  const dy = p.y - sy
  let { x0, y0, x1, y1 } = r0

  if (handle === 'new') {
    x0 = Math.min(sx, p.x); x1 = Math.max(sx, p.x)
    y0 = Math.min(sy, p.y); y1 = Math.max(sy, p.y)
    if (x1 - x0 < SEL_MIN) x1 = Math.min(100, x0 + SEL_MIN)
    if (y1 - y0 < SEL_MIN) y1 = Math.min(100, y0 + SEL_MIN)
  } else if (handle === 'move') {
    const w = x1 - x0, h = y1 - y0
    x0 = Math.min(Math.max(0, x0 + dx), 100 - w)
    y0 = Math.min(Math.max(0, y0 + dy), 100 - h)
    x1 = x0 + w; y1 = y0 + h
  } else {
    if (handle.includes('w')) x0 = Math.min(Math.max(0, r0.x0 + dx), x1 - SEL_MIN)
    if (handle.includes('e')) x1 = Math.max(Math.min(100, r0.x1 + dx), x0 + SEL_MIN)
    if (handle.includes('n')) y0 = Math.min(Math.max(0, r0.y0 + dy), y1 - SEL_MIN)
    if (handle.includes('s')) y1 = Math.max(Math.min(100, r0.y1 + dy), y0 + SEL_MIN)
  }
  emitRect({ x0, y0, x1, y1 })
}

function endSel() {
  selDrag = null
  window.removeEventListener('pointermove', onSelMove)
  window.removeEventListener('pointerup', endSel)
  window.removeEventListener('pointercancel', endSel)
}

// 退出框选模式时收尾（父级可能在拖拽中被卸载/切换）
watch(() => props.selectMode, (on) => { if (!on) endSel() })

// ---- 点选落点（贴纸「固定」模式）----
// 与框选共用 `pctAt`（同一套"指针 → 整帧百分比"换算），只是取一个点而不是拖矩形。
//
// ⚠️ `[sticker] position`（manual）= **贴纸盒子的中心**（`config.rs` 的 `StickerConfig.position`
// 注释 / 设置页口径）；auto 的事件 x/y 才是盒子左上角 —— 两个键、两种基准，不要"统一"。
// 所以这里画的是**以锚点为中心的贴纸真实占位**（边长 280×短边倍率、边距 32×短边倍率），
// 并做出片同样的出框 clamp；上浮量不在这里画（那是动画，且 clamp 之后才叠加）。
const pointBoxStyle = computed(() => {
  const c = cv.value
  const size = 280 * stickerScale(c)
  const margin = 32 * stickerScale(c)
  const p = props.point
  const nx = Number(p?.x), ny = Number(p?.y)
  const px = (Number.isFinite(nx) ? Math.min(100, Math.max(0, nx)) : 50) / 100 * c.W
  const py = (Number.isFinite(ny) ? Math.min(100, Math.max(0, ny)) : 50) / 100 * c.H
  // 与出片同一个 clamp（index.html::stickerBox / engine.js::clampSticker）
  const x = Math.max(margin, Math.min(c.W - size - margin, px - size / 2))
  const y = Math.max(margin, Math.min(c.H - size - margin, py - size / 2))
  // 舞台是按 CSS 缩放的，换算成舞台内的像素；overlayScale = 舞台宽 / 画布宽
  const s = overlayScale.value || 1
  return { left: x * s + 'px', top: y * s + 'px', width: size * s + 'px', height: size * s + 'px' }
})

function pickPoint(e) {
  const p = pctAt(e)
  if (!p) return
  emit('update:point', { x: r2(p.x), y: r2(p.y) })
}

// ---- 真渲染预览：wasm 实时模式优先，回退 server 单帧 ----
const realRender = ref(false)
const realFrameUrl = ref('')
const realLoading = ref(false)
const wasmActive = ref(false)
const wasmLoading = ref(false)
let wasmRenderer = null
let wasmInited = false
let lastWasmFrame = 0
let realTimer = null
let realAbort = null

function clearRealFrame() {
  if (realFrameUrl.value) {
    URL.revokeObjectURL(realFrameUrl.value)
    realFrameUrl.value = ''
  }
  if (realAbort) {
    realAbort.abort()
    realAbort = null
  }
  realLoading.value = false
}

function scheduleRealFrame() {
  if (!props.wgpuRender || !realRender.value || playing.value || wasmActive.value) return
  if (realTimer) clearTimeout(realTimer)
  realLoading.value = true
  realTimer = setTimeout(fetchRealFrame, 250)
}

async function fetchRealFrame() {
  if (!realRender.value || playing.value || wasmActive.value) {
    realLoading.value = false
    return
  }
  if (realAbort) realAbort.abort()
  realAbort = new AbortController()
  try {
    const r = await fetch('/api/polish/render-frame', {
      method: 'POST',
      // 裸 fetch：写请求必须自己带 CSRF 头（服务端对写请求校验 X-CSRF-Token）
      headers: { 'Content-Type': 'application/json', ...csrfHeaders() },
      credentials: 'same-origin',
      body: JSON.stringify({ dir: props.dir, t: current.value }),
      signal: realAbort.signal,
    })
    if (!r.ok) {
      const j = await r.json().catch(() => ({}))
      throw new Error(j.error || `HTTP ${r.status}`)
    }
    const blob = await r.blob()
    if (realFrameUrl.value) URL.revokeObjectURL(realFrameUrl.value)
    realFrameUrl.value = URL.createObjectURL(blob)
  } catch (e) {
    if (e.name !== 'AbortError') toast(e.message, 'err')
  } finally {
    realLoading.value = false
  }
}

function toggleReal() {
  if (!props.wgpuRender) return
  realRender.value = !realRender.value
  if (!realRender.value) {
    clearRealFrame()
    if (realTimer) clearTimeout(realTimer)
    disableWasm()
  } else {
    tryEnableWasm().then((ok) => {
      if (!ok) scheduleRealFrame()
    })
  }
}

// ---- wasm 实时真渲染：WebGPU 可用时，渲染核心直接在浏览器合成 ----
const wasmCanvas = ref(null)

function disableWasm() {
  wasmActive.value = false
  if (wasmRenderer) {
    wasmRenderer.dispose()
    wasmRenderer = null
  }
}

async function tryEnableWasm() {
  if (wasmActive.value && wasmRenderer) return true
  if (!('gpu' in navigator)) return false
  wasmLoading.value = true
  try {
    if (!wasmInited) {
      await init()
      wasmInited = true
    }
    // 资源收集：贴纸字节 + 字体字节
    const map = props.config?._assets?.sticker_map || {}
    const names = []
    const datas = []
    const gifs = []
    for (const [name, p] of Object.entries(map)) {
      const r = await fetch(clipMediaUrl(props.pid, props.dir, p))
      if (!r.ok) continue
      names.push(name)
      datas.push(new Uint8Array(await r.arrayBuffer()))
      gifs.push(p.toLowerCase().endsWith('.gif'))
    }
    let fontBytes = null
    try {
      const fr = await fetch(fontUrl(props.pid, props.dir, props.config?.style?.subtitle?.font || ''))
      if (fr.ok) fontBytes = new Uint8Array(await fr.arrayBuffer())
    } catch (e) {
      // 字体缺失只是回退到内置字体，记 debug 就够（别把面板刷满）
      addClient('debug', `字体下载失败，改用内置字体：${e.message}`, 'preview')
    }

    // canvas 挂进舞台
    let c = wasmCanvas.value
    if (!c) {
      c = document.createElement('canvas')
      c.className = 'wasm-canvas'
      stageRef.value.appendChild(c)
      wasmCanvas.value = c
    }
    c.width = cv.value.W
    c.height = cv.value.H

    wasmRenderer = await init_renderer(
      c,
      JSON.stringify(props.config),
      JSON.stringify(props.subtitles),
      JSON.stringify(props.danmaku),
      props.config?.style?.subtitle?.font || '',
      fontBytes,
      names,
      datas,
      gifs.map((g) => (g ? 1 : 0)),
      cv.value.W,
      cv.value.H,
      vertical.value,
      props.config?.vertical_position || 'left'
    )
    wasmActive.value = true
    return true
  } catch (e) {
    toast('wasm 真渲染不可用，回退单帧模式：' + (e.message || e), 'err')
    console.error('wasm init 失败', e)
    try {
      const st = String(e.stack || '')
      const t2 = document.createElement('div')
      t2.className = 'toast err'
      t2.style.maxWidth = '520px'
      t2.style.whiteSpace = 'pre-wrap'
      t2.textContent = st.split('\n').slice(0, 6).join('\n')
      t2.setAttribute('data-debug', '1')
      document.querySelector('.toast-wrap')?.appendChild(t2)
      setTimeout(() => t2.remove(), 8000)
    } catch {}
    disableWasm()
    return false
  } finally {
    wasmLoading.value = false
  }
}

watch(idx, (v) => {
  player.setEvents(v.soundEvents)
  player.setLoudnessCurve(props.config?.loudness_curve || [])
}, { immediate: true })

onMounted(() => {
  volume.value = player.getMasterVolume()
  muted.value = player.isMuted()
  lastVol = volume.value > 0 ? volume.value : 1
  applyAudio()
  ro = new ResizeObserver(() => {
    const w = stageRef.value ? stageRef.value.clientWidth : 0
    if (w) overlayScale.value = w / cv.value.W
  })
  if (stageRef.value) ro.observe(stageRef.value)
})
onUnmounted(() => {
  cancelAnimationFrame(rafId)
  if (ro) ro.disconnect()
  endSel()
  clearRealFrame()
  if (realTimer) clearTimeout(realTimer)
  disableWasm()
})

// 播放中用自绘预览；暂停/seek 停稳后按需取真渲染帧
watch([realRender, playing], () => {
  if (playing.value) {
    if (realTimer) clearTimeout(realTimer)
    realLoading.value = false
    return
  }
  scheduleRealFrame()
})
watch(current, () => {
  if (!playing.value) scheduleRealFrame()
})
watch(() => props.dir, clearRealFrame)

// 字幕字体：选了程序级字体库里的字体时，浏览器得先认得它（@font-face）才会用 ——
// 出片那条路是 render/server.rs 注入的，这里是**同一套装在 DOM 预览上**，
// 少了它预览还是系统字体、出片却换了字体。字体加载完就自然重排，不用手动重挂。
// `dir` 在精修页是绝对路径（服务端按它反查归属），所以这里不用再传 project。
watch(() => [props.dir, subStyle.value.font], ([dir, font]) => {
  if (font) ensureFontFace(null, dir, font, { onDebug: (m) => addClient('debug', m, 'preview') })
}, { immediate: true })

// wasm 真渲染激活时，字幕在 Rust pipeline 里持有副本；编辑字幕要热同步，避免预览仍显示旧字幕
watch(() => props.subtitles, (subs) => {
  if (!wasmActive.value || !wasmRenderer) return
  try {
    wasmRenderer.set_subtitles(JSON.stringify(subs || []))
    // 暂停/未播放时也立刻重绘当前帧，让字幕修改所见即所得
    const v = videoRef.value
    if (v) wasmRenderer.render_frame(v.currentTime, v)
  } catch (e) {
    console.warn('wasm 字幕热更新失败，回退自绘', e)
    disableWasm()
  }
}, { deep: true })

defineExpose({ seekTo, togglePlay, pause })

/** 供父级（框选布局等）定格画面用：正在播放则暂停。 */
function pause() {
  const v = videoRef.value
  if (v && !v.paused) togglePlay()
}
</script>

<style scoped>
.polish-preview { width:100%; }
/* 预览尺寸：宽度由「可用高度」反推（16:9），并随视口收缩 —— 目标是最大化窗口下
   预览尽量大、时间轴基本同屏。540px ≈ 其余固定部分之和（面包屑 66 + 内容 padding 18 +
   紧凑页头 ~38 + 片段卡条 1 行 ~105 + 编辑区上边距 16 + 预览卡头/控制条 ~93 + 左列间距 16
   + 时间轴卡 ~259 ≈ 611），所以底部会留 ~70px 在首屏外（时间轴第 7 条泳道要小滚一下）；
   想让它完整可见就把 540 调回 610。上限 620 免得大屏上预览过分占位，下限 170。
   —— 用户要求：预览再放大一点。 */
/* 预览舞台比例跟随 `_canvas`（--stage-ar = W/H；--stage-ratio = W/H 数值，是"高度预算→宽度"的乘数：
   clamp 是高度预算，横屏 ×16/9、竖屏 ×9/16 —— 两个方向的舞台**同高**）。 */
.stage { position:relative; width:min(100%, calc(clamp(170px, 100vh - 540px, 620px) * var(--stage-ratio, 1.7778)));
  aspect-ratio:var(--stage-ar, 16/9); margin:0 auto; background:#000; border-radius:14px; overflow:hidden;
  box-shadow:0 6px 18px rgba(0,0,0,.10); }
.stage:fullscreen { width:100%; border-radius:0; }
.bg-video { position:absolute; inset:0; width:100%; height:100%; object-fit:cover;
  filter:blur(var(--bg-blur, 28px)) brightness(0.5) saturate(1.15);
  transform:scale(var(--bg-img-scale, 1.187)); max-height:none; margin:0; border-radius:0; }
/* 画面构成（`_canvas.stage` 驱动）：主画面按比例放进 .fit-box，留边由背景层/舞台底色补 */
.video-wrap { position:absolute; inset:0; display:flex; align-items:center; justify-content:center;
  overflow:hidden; padding:var(--stage-pad, 0%); }
.fit-box { position:relative; transform-origin:center center; transform:scale(var(--stage-scale, 1)); }
.video-wrap.axis-fill .fit-box { width:100%; height:100%; }
.video-wrap.axis-x .fit-box { height:100%; max-width:100%; aspect-ratio:var(--src-ar, 16 / 9); }
.video-wrap.axis-y .fit-box { width:100%; max-height:100%; aspect-ratio:var(--src-ar, 16 / 9); }
.video-wrap.stage-center { justify-content:center; align-items:center; }
.video-wrap.stage-left { justify-content:flex-start; }
.video-wrap.stage-right { justify-content:flex-end; }
.video-wrap.stage-top { align-items:flex-start; }
.video-wrap.stage-bottom { align-items:flex-end; }
.source-video { width:100%; height:100%; max-height:none; margin:0; border-radius:0; object-fit:cover; will-change:transform,filter; }
.corner-glow { position:absolute; inset:0; z-index:1; pointer-events:none; opacity:0; transition:opacity 1s; }
.corner-glow .gb { position:absolute; bottom:-60px; right:-60px; width:260px; height:260px; border-radius:50%; background:radial-gradient(circle,rgba(0,212,255,0.12),transparent 70%); filter:blur(50px); }
.stage:hover .corner-glow { opacity:1; }
.fx { position:absolute; inset:0; pointer-events:none; }
.fx-noise { background:#fff; }
.fx-color { opacity:0; }

/* 虚拟 overlay（默认 1920×1080，随 `_canvas` 由 --ov-w/--ov-h 代入）：top:0 left:0 后由 scale 定位中心 */
.overlay-space { position:absolute; top:50%; left:50%; width:var(--ov-w, 1920px); height:var(--ov-h, 1080px); transform-origin:center center; pointer-events:none; z-index:10; }
.sticker-layer { position:absolute; inset:0; overflow:hidden; }
.sticker-el { position:absolute; width:var(--sticker-s, 280px); height:var(--sticker-s, 280px); display:flex; align-items:center; justify-content:center; }
.sticker-el img { width:100%; height:100%; object-fit:contain; }
.sticker-emoji { font-size:var(--sticker-emoji, 200px); line-height:var(--sticker-s, 280px); text-align:center; font-family:'Noto Color Emoji','Apple Color Emoji','Segoe UI Emoji',sans-serif; }
.danmaku-layer { position:absolute; top:0; left:0; width:var(--ov-w, 1920px); height:var(--dm-h, 540px); overflow:hidden; }
.dm { position:absolute; white-space:nowrap; font-weight:600; -webkit-text-stroke:2px #000; paint-order:stroke fill; }
.captions { position:absolute; bottom:var(--cap-bottom, 80px); left:var(--cap-left, 120px); width:var(--cap-w, 1680px); }
.cap { position:relative; text-align:center; padding:14px 0; font-weight:700; line-height:1.5;
       -webkit-text-stroke:12px #403c3b; paint-order:stroke fill; }
/* A5 折行：每行一个块级 nowrap 行 —— 断行由 wrapCaptionParts 决定（与出片一致） */
.cap-line { display:block; white-space:nowrap; }
.cap .hl { color:var(--hl-color, #f56b41); }
.intro-black { position:absolute; inset:0; z-index:50; background:#000; pointer-events:none; }

/* ---- 框选布局覆盖层：盒子外用超大 box-shadow 压暗，突出选框 ---- */
.sel-layer { position:absolute; inset:0; z-index:70; cursor:crosshair; touch-action:none; }
.sel-box { position:absolute; border:1.5px solid var(--main-deep); cursor:move;
  box-shadow:0 0 0 9999px rgba(0,0,0,.42); }
.sel-tag { position:absolute; top:4px; left:6px; padding:2px 10px; border-radius:var(--r-full);
  background:var(--main-soft); color:var(--pink-text); font-size:11px; white-space:nowrap; }
.sel-h { position:absolute; width:12px; height:12px; border-radius:3px; background:#fff;
  border:1.5px solid var(--main-deep); }
.h-nw { left:-6px; top:-6px; cursor:nwse-resize; }
.h-n { left:50%; top:-6px; margin-left:-6px; cursor:ns-resize; }
.h-ne { right:-6px; top:-6px; cursor:nesw-resize; }
.h-w { left:-6px; top:50%; margin-top:-6px; cursor:ew-resize; }
.h-e { right:-6px; top:50%; margin-top:-6px; cursor:ew-resize; }
.h-sw { left:-6px; bottom:-6px; cursor:nesw-resize; }
.h-s { left:50%; bottom:-6px; margin-left:-6px; cursor:ns-resize; }
.h-se { right:-6px; bottom:-6px; cursor:nwse-resize; }

/* ---- 点选落点覆盖层（贴纸「固定」）：**不压暗**（要能看清画面挑落点），
   只给一个十字落点 + 中心提示；光标变十字表示可点 ---- */
.point-layer { position:absolute; inset:0; z-index:70; cursor:crosshair; touch-action:none; }
.point-tag { position:absolute; top:6px; left:8px; padding:2px 10px; border-radius:var(--r-full);
  background:var(--main-soft); color:var(--pink-text); font-size:11px; white-space:nowrap; }
/* 贴纸真实占位框（边长 280×短边倍率，**以锚点为中心**）：虚线框 + 中心一个点，便于对齐 */
.point-box { position:absolute; border:1.5px dashed var(--main-deep); background:rgba(255,255,255,.12);
  border-radius:4px; pointer-events:none; box-sizing:border-box; }
.point-dot { position:absolute; left:50%; top:50%; width:12px; height:12px; margin:-6px 0 0 -6px;
  border-radius:50%; background:#fff; border:2px solid var(--main-deep); }

.preview-controls { display:flex; align-items:center; gap:8px; margin-top:10px; }
.time-label { min-width:96px; text-align:right; }
.vol { width:80px; flex:none; padding:0; border:none; background:transparent; }
/* 音量的滑块外观统一由 app.css 的 input[type=range] 提供（这里不再覆盖，避免两套外观） */
.real-frame { position:absolute; inset:0; width:100%; height:100%; object-fit:fill; z-index:60; pointer-events:none; }
.wasm-canvas { position:absolute; inset:0; width:100%; height:100%; z-index:60; pointer-events:none; }
.real-loading { position:absolute; inset:0; z-index:61; display:flex; align-items:center; justify-content:center; pointer-events:none; }
.real-loading .spinner { width:22px; height:22px; border-width:3px; }
button:disabled { opacity:.45; cursor:not-allowed; }
</style>
