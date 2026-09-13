<template>
  <div class="polish-preview">
    <div ref="stageRef" class="stage">
      <video v-if="vertical && videoSrc" ref="bgRef" class="bg-video" :src="videoSrc" muted playsinline preload="auto"></video>
      <div class="video-wrap" :class="portraitClass">
        <video ref="videoRef" class="source-video" :src="videoSrc" playsinline preload="auto"
               :style="videoStyle"
               @loadedmetadata="onMeta" @seeked="onSeeked" @play="playing = true" @pause="playing = false" @ended="playing = false"></video>
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
          <div v-for="(c, i) in capRows" :key="'c' + i" class="cap" :style="{ opacity: c.opacity, transform: 'translateY(' + c.yOff + 'px) scale(' + c.scale + ')' }">
            <span v-for="(p, j) in c.parts" :key="j" :class="{ hl: p.hl }">{{ p.text }}</span>
          </div>
        </div>
      </div>
      <div class="intro-black" :style="{ opacity: introBlack }"></div>
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
      <button class="mini-btn" :class="{ selected: muted }" @click="toggleMute" :title="muted ? '取消静音' : '静音'">
        <Icon :name="muted ? 'volume-x' : 'volume'" :size="13" />
      </button>
      <input class="vol" type="range" min="0" max="1" step="0.05" :value="volume" @input="onVolume" title="音效音量">
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
import { mediaUrl } from '../utils'
import { buildIndex, videoFxAt, getFlash, getHueShiftFlash, getNoiseOpacity, getVignette, getOverlayState, introBlackAt, subsAt, danmakuAt, stickersAt } from '../polish/engine'
import { createSfxPlayer } from '../polish/audio'
import init, { init_renderer } from '../wasm/vtrim_wasm.js'

const props = defineProps({
  dir: { type: String, required: true },
  config: { type: Object, default: null },
  subtitles: { type: Array, default: () => [] },
  danmaku: { type: Array, default: () => [] },
  metaDuration: { type: Number, default: 0 },
  // 服务端 /api/health 的 wgpu_render：false 时真渲染（单帧回退）不可用
  wgpuRender: { type: Boolean, default: false }
})
const emit = defineEmits(['time'])

const stageRef = ref(null)
const videoRef = ref(null)
const bgRef = ref(null)
const current = ref(0)
const duration = ref(0)
const playing = ref(false)
const muted = ref(false)
const volume = ref(1)
const overlayScale = ref(1)
const stickerFailed = ref({})
let rafId = 0
let ro = null

const player = createSfxPlayer({
  resolveUrl: (name) => mediaUrl(props.dir, 'sfx/' + name + '.mp3')
})

const videoSrc = computed(() => mediaUrl(props.dir, props.config?.source?.video || 'clip.mp4'))
const muteRanges = computed(() => (props.config?.mute_ranges || [])
  .map(r => ({ start: Number(r.start) || 0, end: Number(r.end) || 0 }))
  .filter(r => r.end - r.start > 0.001)
  .sort((a, b) => a.start - b.start))
function muteRangeAt(t) {
  return muteRanges.value.find(r => t >= r.start && t < r.end) || null
}
const vertical = computed(() => !!props.config?.vertical)
const portraitClass = computed(() => vertical.value ? ('portrait ' + (props.config?.vertical_position || 'left')) : '')

const stickerMap = computed(() => props.config?._assets?.sticker_map || {})
function stickerSrc(name) {
  if (stickerFailed.value[name]) return ''
  const p = stickerMap.value[name]
  return p ? mediaUrl(props.dir, p) : ''
}

const effDuration = computed(() => duration.value || props.metaDuration || 0)
const idx = computed(() => buildIndex({
  config: props.config,
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
  const f = vfx.value
  const t = []
  if (f.shake.x || f.shake.y) t.push(`translate(${f.shake.x}px, ${f.shake.y}px)`)
  if (f.stutter) t.push(`translate(${f.stutter.x}px, ${f.stutter.y}px)`)
  if (f.split && f.split.skewX) t.push(`skewX(${f.split.skewX / 100}deg)`)
  if (f.zoom > 1.001) t.push(`scale(${f.zoom})`)
  const filter = f.blur > 0 ? `blur(${f.blur}px)` : ''
  return {
    transform: t.join(' ') || 'none',
    transformOrigin: `${f.camera.ox / 19.2}% ${f.camera.oy / 10.8}%`,
    filter: [filter, f.colorShift].filter(Boolean).join(' ') || 'none'
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
  const grad = ov.shape === 'radial'
    ? 'radial-gradient(ellipse at center,transparent 0%,' + ov.colors.slice(1).join(',')
    : 'linear-gradient(' + ov.colors.join(',')
  return { opacity: ov.opacity, background: grad }
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
      if (vertical.value && bgRef.value) bgRef.value.currentTime = v.currentTime
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
  if (vertical.value && bgRef.value && bgRef.value.currentTime &&
      Math.abs(bgRef.value.currentTime - v.currentTime) > 0.05) {
    bgRef.value.currentTime = v.currentTime
  }
  if (!v.paused && !v.ended) rafId = requestAnimationFrame(tick)
}

function togglePlay() {
  const v = videoRef.value
  if (!v) return
  if (v.paused) {
    if (vertical.value && bgRef.value) bgRef.value.play().catch(() => {})
    v.play().catch(() => {})
    playing.value = true
    cancelAnimationFrame(rafId)
    rafId = requestAnimationFrame(tick)
  } else {
    v.pause()
    if (vertical.value && bgRef.value) bgRef.value.pause()
    playing.value = false
    cancelAnimationFrame(rafId)
  }
}

function seekTo(t) {
  const v = videoRef.value
  if (!v) return
  v.currentTime = Number(t) || 0
  current.value = v.currentTime
  player.seek(current.value)
  if (vertical.value && bgRef.value) bgRef.value.currentTime = v.currentTime
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
  // seek 触发首帧解码，避免打开页面时舞台全黑
  if (v.currentTime === 0) v.currentTime = 0.001
  if (vertical.value && bgRef.value) {
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
  if (vertical.value && bgRef.value) bgRef.value.currentTime = v.currentTime
}

function toggleMute() {
  muted.value = !muted.value
  player.setMuted(muted.value)
}
function onVolume(e) {
  volume.value = Number(e.target.value)
  player.setMasterVolume(volume.value)
  if (volume.value > 0 && muted.value) { muted.value = false; player.setMuted(false) }
}
function fullscreen() {
  if (stageRef.value) stageRef.value.requestFullscreen?.()
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
      const r = await fetch(mediaUrl(props.dir, p))
      if (!r.ok) continue
      names.push(name)
      datas.push(new Uint8Array(await r.arrayBuffer()))
      gifs.push(p.toLowerCase().endsWith('.gif'))
    }
    let fontBytes = null
    try {
      const fr = await fetch(`/api/polish/font?dir=${encodeURIComponent(props.dir)}&name=${encodeURIComponent(props.config?.style?.subtitle?.font || '')}`)
      if (fr.ok) fontBytes = new Uint8Array(await fr.arrayBuffer())
    } catch {}

    // canvas 挂进舞台
    let c = wasmCanvas.value
    if (!c) {
      c = document.createElement('canvas')
      c.className = 'wasm-canvas'
      stageRef.value.appendChild(c)
      wasmCanvas.value = c
    }
    c.width = 1920
    c.height = 1080

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
      1920,
      1080,
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
  ro = new ResizeObserver(() => {
    const w = stageRef.value ? stageRef.value.clientWidth : 0
    if (w) overlayScale.value = w / 1920
  })
  if (stageRef.value) ro.observe(stageRef.value)
})
onUnmounted(() => {
  cancelAnimationFrame(rafId)
  if (ro) ro.disconnect()
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

defineExpose({ seekTo, togglePlay })
</script>

<style scoped>
.polish-preview { width:100%; }
.stage { position:relative; width:min(100%, calc(66vh * 16 / 9)); aspect-ratio:16/9; margin:0 auto; background:#000; border-radius:14px; overflow:hidden; box-shadow:0 6px 18px rgba(0,0,0,.10); }
.stage:fullscreen { width:100%; border-radius:0; }
.bg-video { position:absolute; inset:0; width:100%; height:100%; object-fit:cover; filter:blur(28px) brightness(0.5) saturate(1.15); transform:scale(1.15); max-height:none; margin:0; border-radius:0; }
.video-wrap { position:absolute; inset:0; display:flex; align-items:center; overflow:hidden; }
.video-wrap.portrait.left { justify-content:flex-start; }
.video-wrap.portrait.center { justify-content:center; }
.video-wrap.portrait .source-video { width:auto; height:100%; object-fit:contain; }
.source-video { width:100%; height:100%; max-height:none; margin:0; border-radius:0; object-fit:contain; will-change:transform,filter; }
.corner-glow { position:absolute; inset:0; z-index:1; pointer-events:none; opacity:0; transition:opacity 1s; }
.corner-glow .gb { position:absolute; bottom:-60px; right:-60px; width:260px; height:260px; border-radius:50%; background:radial-gradient(circle,rgba(0,212,255,0.12),transparent 70%); filter:blur(50px); }
.stage:hover .corner-glow { opacity:1; }
.fx { position:absolute; inset:0; pointer-events:none; }
.fx-noise { background:#fff; }
.fx-color { opacity:0; }

/* 1920×1080 虚拟 overlay：top:0 left:0 后由 scale 定位中心 */
.overlay-space { position:absolute; top:50%; left:50%; width:1920px; height:1080px; transform-origin:center center; pointer-events:none; z-index:10; }
.sticker-layer { position:absolute; inset:0; overflow:hidden; }
.sticker-el { position:absolute; width:280px; height:280px; display:flex; align-items:center; justify-content:center; }
.sticker-el img { width:100%; height:100%; object-fit:contain; }
.sticker-emoji { font-size:200px; line-height:280px; text-align:center; font-family:'Noto Color Emoji','Apple Color Emoji','Segoe UI Emoji',sans-serif; }
.danmaku-layer { position:absolute; top:0; left:0; width:1920px; height:540px; overflow:hidden; }
.dm { position:absolute; white-space:nowrap; font-weight:600; -webkit-text-stroke:2px #000; paint-order:stroke fill; }
.captions { position:absolute; bottom:80px; left:120px; width:1680px; }
.cap { position:relative; text-align:center; padding:14px 44px; font-weight:700; line-height:1.5;
       -webkit-text-stroke:12px #403c3b; paint-order:stroke fill; }
.cap .hl { color:var(--hl-color, #f56b41); }
.intro-black { position:absolute; inset:0; z-index:50; background:#000; pointer-events:none; }

.preview-controls { display:flex; align-items:center; gap:8px; margin-top:10px; }
.time-label { min-width:96px; text-align:right; }
.vol { width:80px; flex:none; padding:0; border:none; background:transparent; }
input[type='range'] { accent-color:var(--main); padding:0; border:none; background:transparent; height:20px; }
.real-frame { position:absolute; inset:0; width:100%; height:100%; object-fit:fill; z-index:60; pointer-events:none; }
.wasm-canvas { position:absolute; inset:0; width:100%; height:100%; z-index:60; pointer-events:none; }
.real-loading { position:absolute; inset:0; z-index:61; display:flex; align-items:center; justify-content:center; pointer-events:none; }
.real-loading .spinner { width:22px; height:22px; border-width:3px; }
button:disabled { opacity:.45; cursor:not-allowed; }
</style>
