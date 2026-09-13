<template>
  <div class="card" id="cover-design">
    <div class="row spread">
      <h2 class="card-h2 m-0"><Icon name="image" :size="16" /> 封面设计器</h2>
      <button class="btn btn-primary" :disabled="generating" @click="generateCover">{{ generating ? '生成中…' : '生成封面' }}</button>
    </div>
    <div class="designer-grid">
      <div class="stage-wrap" ref="stageWrapRef">
        <div ref="stageRef" class="stage">
          <div class="stage-media">
            <video ref="videoRef" playsinline preload="metadata" class="stage-video" :src="videoSrc" :style="mediaStyle" @timeupdate="onVideoTime" @loadedmetadata="onMeta" @play="playing = true" @pause="playing = false"></video>
          </div>
          <div class="fx-layer">
            <div v-for="(fx, i) in fxList" :key="i" class="fx-item" :style="fxStyle(fx)"></div>
          </div>
          <div class="gradient-layer" :style="gradientStyle"></div>
          <div class="vignette-layer" :style="{ opacity: vignette }"></div>
          <div class="title-wrap" :style="{ left: textX + '%', top: textY + '%', width: textWidth + '%' }">
            <div class="title-text" :style="titleStyle" v-html="titleHtml"></div>
          </div>
          <div class="stage-gesture-layer" @pointerdown="onStagePointerDown" @wheel.prevent="onStageWheel"></div>
        </div>
        <div class="seek-row mt-4">
          <button class="mini-btn" @click="togglePlay">{{ playing ? '暂停' : '播放' }}</button>
          <input class="flex-1 seek-range" type="range" min="0" :max="duration || 1" step="0.1" :value="time" @input="onSeekInput">
          <span class="muted small mono seek-time">{{ fmtTime(time) }} / {{ fmtTime(duration) }}</span>
        </div>
        <div class="row mt-8">
          <span class="muted small">时间</span>
          <input v-model.number="time" type="number" step="0.1" class="w-time">
          <button class="mini-btn" @click="takeCurrent">取当前帧</button>
          <button class="mini-btn" @click="exportFrame">导出关键帧</button>
          <button class="mini-btn" @click="fileInput.click()">导入背景图片</button>
          <input ref="fileInput" type="file" accept="image/*" class="hidden-file" @change="importBg">
        </div>
      </div>
      <div class="controls" ref="controlsRef">
        <details open class="grp">
          <summary>预设</summary>
          <div class="preset-row">
            <button v-for="p in presets" :key="p.key" class="mini-btn" :class="{ selected: preset === p.key }" @click="applyPreset(p.key)">{{ p.label }}</button>
          </div>
        </details>

        <details open class="grp">
          <summary>标题与高亮</summary>
          <textarea v-model="titleLinesText" rows="2" placeholder="封面标题，每行一段"></textarea>
          <div class="mini-row mt-4">
            <button class="mini-btn" :disabled="generatingTitle" title="只用 clip.srt + polish.toml 逐个事件生成" @click="generateTitleByLlm">{{ generatingTitle ? '生成中…' : 'LLM 生成标题' }}</button>
            <span v-if="savingConfig" class="muted small">保存中…</span>
          </div>
          <div class="mini-grid mt-4">
            <div class="mini-field"><label>高亮色</label><input v-model="hlColor" type="color"></div>
            <label class="mini-field check"><input v-model="textShadow" type="checkbox"><span>文字阴影 {{ textShadow ? '开' : '关' }}</span></label>
          </div>
          <div class="mini-row mt-4">
            <input v-model="highlightInput" class="kw-input" placeholder="高亮关键词，回车添加" @keyup.enter="addHighlightKeyword">
            <button class="mini-btn" @click="addHighlightKeyword"><Icon name="plus" :size="11" /> 添加</button>
          </div>
          <div v-if="highlightKeywords.length" class="hl-chips mt-4">
            <span v-for="(kw, i) in highlightKeywords" :key="i" class="hl-chip">
              {{ kw }}
              <button class="hl-x" title="删除" @click="removeHighlightKeyword(i)">×</button>
            </span>
          </div>
        </details>

        <details open class="grp">
          <summary>排版</summary>
          <div class="mini-grid">
            <div class="mini-field"><label>对齐</label>
              <select v-model="align">
                <option value="left">左</option>
                <option value="center">中</option>
                <option value="right">右</option>
              </select>
            </div>
            <div class="mini-field"><label>宽度</label><input type="range" min="20" max="98" step="1" v-model.number="textWidth"><span class="rv">{{ textWidth }}%</span></div>
          </div>
          <div class="mini-grid mt-4">
            <div class="mini-field"><label>X</label><input type="range" min="0" max="100" step="1" v-model.number="textX"><span class="rv">{{ textX }}%</span></div>
            <div class="mini-field"><label>Y</label><input type="range" min="0" max="100" step="1" v-model.number="textY"><span class="rv">{{ textY }}%</span></div>
          </div>
          <div class="mini-grid mt-4">
            <div class="mini-field"><label>字号</label><input type="range" min="24" max="240" step="2" v-model.number="fontSize"><span class="rv">{{ fontSize }}px</span></div>
            <div class="mini-field"><label>描边</label><input type="range" min="0" max="60" step="1" v-model.number="stroke"><span class="rv">{{ stroke }}px</span></div>
          </div>
          <div class="mini-grid mt-4">
            <div class="mini-field"><label>行距</label><input type="range" min="0.8" max="2.6" step="0.05" v-model.number="lineHeight"><span class="rv">{{ Number(lineHeight).toFixed(2) }}</span></div>
            <div class="mini-field"><label>字距</label><input type="range" min="-10" max="60" step="1" v-model.number="letterSpacing"><span class="rv">{{ letterSpacing }}px</span></div>
          </div>
        </details>

        <details open class="grp">
          <summary>背景与阴影</summary>
          <div class="mini-grid">
            <div class="mini-field"><label>渐变</label>
              <select v-model="gradientDir">
                <option value="left">左</option>
                <option value="right">右</option>
                <option value="top">上</option>
                <option value="bottom">下</option>
                <option value="radial">径向</option>
                <option value="none">无</option>
              </select>
            </div>
            <div class="mini-field"><label>暗角</label><input type="range" min="0" max="1" step="0.02" v-model.number="vignette"><span class="rv">{{ Number(vignette).toFixed(2) }}</span></div>
          </div>
          <div class="mini-grid mt-4">
            <div class="mini-field"><label>缩放</label><input type="range" min="0.5" max="3" step="0.05" v-model.number="coverZoom"><span class="rv">{{ Math.round(coverZoom * 100) }}%</span></div>
            <div class="drag-hint" title="拖动预览窗口 = 移动背景 · 滚轮 = 缩放背景">拖预览移动 · 滚轮缩放</div>
          </div>
        </details>

        <details open class="grp">
          <summary>封面特效（可叠加）</summary>
          <div v-for="(fx, i) in fxList" :key="i" class="fx-card">
            <div class="fx-head">
              <span class="chip p">{{ fxTypeName(fx.type) }}</span>
              <button class="mini-btn danger" @click="removeFx(i)">删除</button>
            </div>
            <div class="mini-grid mt-4">
              <div class="mini-field"><label>强度</label><input type="range" min="0" max="1" step="0.05" v-model.number="fx.amount"><span class="rv">{{ Number(fx.amount).toFixed(2) }}</span></div>
              <div class="mini-field"><label>X</label><input type="range" min="0" max="100" step="1" v-model.number="fx.x"><span class="rv">{{ fx.x }}%</span></div>
            </div>
            <div class="mini-grid mt-4">
              <div class="mini-field"><label>Y</label><input type="range" min="0" max="100" step="1" v-model.number="fx.y"><span class="rv">{{ fx.y }}%</span></div>
              <div v-if="fx.type === 'overlay'" class="mini-field"><label>颜色</label><input type="color" v-model="fx.color"></div>
            </div>
          </div>
          <div class="mini-row mt-4">
            <select v-model="addType" class="mini-select grow">
              <option value="glow">光晕</option>
              <option value="blur">模糊</option>
              <option value="vignette">暗角点</option>
              <option value="overlay">颜色覆盖</option>
              <option value="scanline">扫描线</option>
            </select>
            <button class="mini-btn" @click="addFx"><Icon name="plus" :size="11" /> 添加</button>
          </div>
        </details>
      </div>
    </div>
  </div>
</template>

<script setup>
import { ref, computed, onMounted, onUnmounted, watch } from 'vue'
import { api } from '../api/client'
import { toast } from '../stores/toast'
import Icon from './Icon.vue'
import { mediaUrl, fmtTime } from '../utils'
import { watchJob } from '../api/jobs'

const props = defineProps({ dir: { type: String, required: true } })
const emit = defineEmits(['changed'])
const generating = ref(false)
const generatingTitle = ref(false)
const configLoaded = ref(false)
const savingConfig = ref(false)
const stageRef = ref(null)
const stageWrapRef = ref(null)
const controlsRef = ref(null)
const stageScale = ref(1)
let resizeRo = null
const videoSrc = ref('')
const videoRef = ref(null)
const fileInput = ref(null)
const titleLines = ref([])
const time = ref(0)
const textX = ref(35)
const textY = ref(50)
const textWidth = ref(62)
const preset = ref('left-text')
const fontSize = ref(120)
const stroke = ref(20)
const hlColor = ref('#ffcc00')
const highlightKeywords = ref([])
const highlightInput = ref('')
const letterSpacing = ref(8)
const lineHeight = ref(1.7)
const textShadow = ref(true)
const gradientDir = ref('left')
const vignette = ref(0.08)
const align = ref('center')
const layout = ref({})
const duration = ref(0)
const playing = ref(false)
const stageDrag = ref(null)
const coverZoom = ref(1)
const coverOffsetX = ref(0)
const coverOffsetY = ref(0)
const fxList = ref([])
const addType = ref('glow')
let saveConfigTimer = null
let saveConfigPending = false
const titleLinesText = computed({
  get: () => titleLines.value.join('\n'),
  set: (v) => { titleLines.value = String(v || '').split('\n') }
})

function escapeHtml(s) {
  return String(s ?? '').replace(/[&<>"']/g, c => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]))
}

function highlightTitleLine(line, kws) {
  let html = escapeHtml(line)
  for (const kw of kws) {
    const target = escapeHtml(kw).toLowerCase()
    if (!target) continue
    let out = ''
    let rest = html
    let idx = rest.toLowerCase().indexOf(target)
    while (idx >= 0) {
      out += rest.slice(0, idx) + '<span class="hl">' + rest.slice(idx, idx + target.length) + '</span>'
      rest = rest.slice(idx + target.length)
      idx = rest.toLowerCase().indexOf(target)
    }
    html = out + rest
  }
  return html
}

const titleHtml = computed(() => {
  const lines = titleLines.value.length ? titleLines.value : ['']
  const kws = highlightKeywords.value.filter(Boolean)
  return lines.map(line => highlightTitleLine(line, kws)).join('\n')
})

const presets = [
  { key: 'left-text', label: '左文', x: 35, y: 50, width: 62, align: 'left', textShadow: true, gradientDir: 'left', vignette: 0.08 },
  { key: 'center-text', label: '居中', x: 50, y: 50, width: 70, align: 'center', textShadow: true, gradientDir: 'radial', vignette: 0.05 },
  { key: 'right-text', label: '右文', x: 65, y: 50, width: 62, align: 'right', textShadow: true, gradientDir: 'right', vignette: 0.08 },
  { key: 'minimal', label: '极简', x: 50, y: 50, width: 72, align: 'center', textShadow: false, gradientDir: 'none', vignette: 0.15 }
]

const fxNames = { glow: '光晕', blur: '模糊', vignette: '暗角点', overlay: '颜色覆盖', scanline: '扫描线' }
function fxTypeName(t) { return fxNames[t] || t }
function fxStyle(fx) {
  const amount = Number(fx.amount) || 0.3
  const x = Number(fx.x ?? 50)
  const y = Number(fx.y ?? 50)
  if (fx.type === 'glow') return {
    background: `radial-gradient(circle at ${x}% ${y}%, rgba(255,220,180,${Math.min(0.9, amount)}) 0%, transparent 60%)`
  }
  if (fx.type === 'blur') return {
    backdropFilter: `blur(${Math.max(1, Math.round(amount * 12))}px)`,
    WebkitMaskImage: `radial-gradient(circle at ${x}% ${y}%, black 0%, transparent 60%)`,
    maskImage: `radial-gradient(circle at ${x}% ${y}%, black 0%, transparent 60%)`
  }
  if (fx.type === 'vignette') return {
    background: `radial-gradient(circle at ${x}% ${y}%, transparent 40%, rgba(0,0,0,${Math.min(1, amount)}) 100%)`
  }
  if (fx.type === 'overlay') {
    return {
      background: fx.color || '#000000',
      opacity: Math.min(0.85, amount),
      mixBlendMode: 'overlay'
    }
  }
  if (fx.type === 'scanline') {
    return {
      background: `repeating-linear-gradient(0deg, rgba(255,255,255,${Math.min(0.5, amount * 0.45)}) 0 1px, transparent 1px 4px)`
    }
  }
  return {}
}


const shadowStyle = computed(() => {
  if (!textShadow.value) return 'none'
  return '0 4px 18px rgba(0,0,0,0.78)'
})

const titleStyle = computed(() => {
  const s = stageScale.value || 1
  const ls = Number(letterSpacing.value) || 0
  return {
    fontSize: Math.max(12, Math.round((fontSize.value || 120) * s)) + 'px',
    letterSpacing: (ls * s).toFixed(1) + 'px',
    lineHeight: Number(lineHeight.value) || 1.7,
    WebkitTextStroke: ((stroke.value || 20) * s).toFixed(1) + 'px rgba(0,0,0,0.65)',
    textShadow: shadowStyle.value,
    '--hlc': hlColor.value,
    textAlign: align.value || 'center'
  }
})

const gradientStyle = computed(() => {
  const dir = gradientDir.value
  if (dir === 'none') return { display: 'none' }
  if (dir === 'radial') return { background: 'radial-gradient(ellipse at 30% 50%, rgba(20,10,40,0.55) 0%, rgba(20,10,40,0.15) 60%, transparent 100%)' }
  if (dir === 'right') return { background: 'linear-gradient(270deg, rgba(20,10,40,0.55) 0%, rgba(20,10,40,0.15) 50%, transparent 100%)' }
  if (dir === 'top') return { background: 'linear-gradient(180deg, rgba(20,10,40,0.55) 0%, rgba(20,10,40,0.15) 50%, transparent 100%)' }
  if (dir === 'bottom') return { background: 'linear-gradient(0deg, rgba(20,10,40,0.55) 0%, rgba(20,10,40,0.15) 50%, transparent 100%)' }
  return { background: 'linear-gradient(90deg, rgba(20,10,40,0.55) 0%, rgba(20,10,40,0.15) 50%, transparent 100%)' }
})


// 与后端 cover 生成保持一致的背景缩放/聚焦：基于 layout.liver 的 origin_grid/origin
const GRID_CENTERS = {
  'top-left': '16.67% 16.67%', top: '50% 16.67%', 'top-right': '83.34% 16.67%',
  'mid-left': '16.67% 50%', center: '50% 50%', 'mid-right': '83.34% 50%',
  'bot-left': '16.67% 83.33%', bottom: '50% 83.33%', 'bot-right': '83.34% 83.33%'
}

const mediaStyle = computed(() => {
  const reg = layout.value?.liver || {}
  let origin = ''
  if (reg.origin_grid && GRID_CENTERS[reg.origin_grid]) {
    origin = GRID_CENTERS[reg.origin_grid]
  } else if (reg.origin) {
    origin = reg.origin
  } else if (reg.grid && reg.grid.length && GRID_CENTERS[reg.grid[0]]) {
    origin = GRID_CENTERS[reg.grid[0]]
  }
  const parts = origin.split(/\s+/)
  const baseX = parseFloat(parts[0]) || 65
  const baseY = parseFloat(parts[1]) || 50
  const baseZoom = baseX >= 50 ? 1.4 : 1.3
  const zoom = (baseZoom || 1) * (coverZoom.value || 1)
  return {
    transformOrigin: baseX + '% ' + baseY + '%',
    transform: `translate(${coverOffsetX.value || 0}px, ${coverOffsetY.value || 0}px) scale(${zoom})`
  }
})

function applyPreset(key) {
  const p = presets.find(x => x.key === key)
  if (!p) return
  preset.value = key
  textX.value = p.x
  textY.value = p.y
  if (p.width) textWidth.value = p.width
  align.value = p.align || 'center'
  textShadow.value = p.textShadow !== false
  if (p.gradientDir) gradientDir.value = p.gradientDir
  if (p.vignette != null) vignette.value = p.vignette
}

function takeCurrent() {
  const v = videoRef.value
  if (v) time.value = Math.round((v.currentTime || 0) * 10) / 10
}

function addFx() {
  fxList.value.push({ type: addType.value, amount: 0.3, x: 50, y: 50, color: '#000000' })
}
function removeFx(i) { fxList.value.splice(i, 1) }

async function exportFrame() {
  try {
    await api('/api/polish/cover-frame', { method: 'POST', body: JSON.stringify({ dir: props.dir, time: time.value || 0 }) })
    toast('关键帧已导出')
  } catch (e) { toast(e.message) }
}

async function importBg(e) {
  const file = e.target.files[0]
  if (!file) return
  const reader = new FileReader()
  reader.onload = async () => {
    try {
      const base64 = String(reader.result).split(',')[1]
      await api('/api/polish/cover-background', { method: 'POST', body: JSON.stringify({ dir: props.dir, image_data: base64 }) })
      toast('背景已导入')
    } catch (err) { toast(err.message) }
  }
  reader.readAsDataURL(file)
}

async function generateTitleByLlm() {
  if (generatingTitle.value) return
  generatingTitle.value = true
  try {
    const r = await api('/api/polish/cover-title', { method: 'POST', body: JSON.stringify({ dir: props.dir }) })
    const lines = r?.title_lines || []
    if (lines.length) titleLines.value = lines
    else toast('LLM 未返回标题')
  } catch (e) {
    toast('LLM 生成标题失败：' + (e.message || e), 'err')
  } finally {
    generatingTitle.value = false
  }
}

function addHighlightKeyword() {
  const kw = highlightInput.value.trim()
  if (!kw) return
  if (!highlightKeywords.value.includes(kw)) {
    highlightKeywords.value = [...highlightKeywords.value, kw]
  }
  highlightInput.value = ''
}

function removeHighlightKeyword(i) {
  highlightKeywords.value.splice(i, 1)
}

function coverConfigPayload() {
  return {
    title_lines: titleLines.value,
    text_width: textWidth.value / 100,
    text_x: textX.value + '%',
    text_y: textY.value + '%',
    font_size: fontSize.value,
    stroke: stroke.value,
    hl_color: hlColor.value,
    highlight_keywords: highlightKeywords.value,
    letter_spacing: letterSpacing.value + 'px',
    line_height: lineHeight.value,
    text_shadow: textShadow.value,
    shadow_dir: textShadow.value ? 'center' : 'none',
    shadow_intensity: textShadow.value ? 1 : 0,
    gradient_dir: gradientDir.value,
    vignette: vignette.value,
    text_align: align.value,
    effects: fxList.value.map(f => ({ kind: f.type, amount: Number(f.amount) || 0.3, x: Number(f.x) || 50, y: Number(f.y) || 50, color: f.color || '#000000' })),
    bg_x: Math.round((coverOffsetX.value || 0) / (stageScale.value || 1) * 10) / 10,
    bg_y: Math.round((coverOffsetY.value || 0) / (stageScale.value || 1) * 10) / 10,
    bg_zoom: coverZoom.value || 1
  }
}

async function persistCoverConfig() {
  if (!configLoaded.value) return
  if (savingConfig.value) { saveConfigPending = true; return }
  savingConfig.value = true
  try {
    await api('/api/polish/cover-config', {
      method: 'POST',
      body: JSON.stringify({ dir: props.dir, cover: coverConfigPayload() })
    })
  } catch (e) {
    console.warn('保存封面设置失败', e)
  } finally {
    savingConfig.value = false
    if (saveConfigPending) {
      saveConfigPending = false
      persistCoverConfig()
    }
  }
}

function scheduleCoverConfigSave() {
  if (!configLoaded.value) return
  if (saveConfigTimer) clearTimeout(saveConfigTimer)
  saveConfigTimer = setTimeout(() => {
    saveConfigTimer = null
    persistCoverConfig()
  }, 600)
}

watch(
  () => [
    JSON.stringify(titleLines.value),
    textX.value, textY.value, textWidth.value,
    fontSize.value, stroke.value,
    hlColor.value, JSON.stringify(highlightKeywords.value),
    letterSpacing.value, lineHeight.value,
    textShadow.value,
    gradientDir.value, vignette.value,
    align.value,
    JSON.stringify(fxList.value),
    coverOffsetX.value, coverOffsetY.value, coverZoom.value
  ],
  scheduleCoverConfigSave
)

async function generateCover() {
  if (generating.value) return
  generating.value = true
  try {
    // 手动设定过时间时，先生成/刷新关键帧，避免生成时仍用默认 frame_pos
    if (Number(time.value) > 0) {
      await api('/api/polish/cover-frame', { method: 'POST', body: JSON.stringify({ dir: props.dir, time: time.value }) })
    }
    await api('/api/polish/cover-config', { method: 'POST', body: JSON.stringify({ dir: props.dir, cover: coverConfigPayload() }) })
    const coverRes = await api('/api/polish/cover', { method: 'POST', body: JSON.stringify({ dir: props.dir }) })
    if (coverRes && coverRes.id) {
      await new Promise((resolve, reject) => {
        const close = watchJob(coverRes.id, {
          onDone(summary) {
            close()
            if (summary.status === 'succeeded') resolve()
            else reject(new Error(summary.error || summary.message || '封面生成失败'))
          }
        })
      })
    }
    emit('changed')
    toast('封面已生成')
  } catch (e) {
    console.error('生成封面失败', e)
    toast('生成封面失败：' + (e.message || e), 'err')
  } finally {
    generating.value = false
  }
}


function onVideoTime() {
  const v = videoRef.value
  if (v) time.value = Math.round((v.currentTime || 0) * 10) / 10
}
function onMeta() {
  const v = videoRef.value
  if (v) duration.value = v.duration || 0
}
function togglePlay() {
  const v = videoRef.value
  if (!v) return
  if (v.paused) v.play().catch(() => {})
  else v.pause()
}
function onSeekInput(e) {
  const v = videoRef.value
  if (!v || !duration.value) return
  const t = Number(e.target.value) || 0
  v.currentTime = t
  time.value = Math.round(t * 10) / 10
}
function onStagePointerDown(e) {
  if (e.button !== 0) return
  e.preventDefault()
  const startX = e.clientX
  const startY = e.clientY
  const startOx = coverOffsetX.value
  const startOy = coverOffsetY.value
  const onMove = (ev) => {
    coverOffsetX.value = startOx + (ev.clientX - startX)
    coverOffsetY.value = startOy + (ev.clientY - startY)
  }
  const onUp = () => {
    window.removeEventListener('pointermove', onMove)
    window.removeEventListener('pointerup', onUp)
    window.removeEventListener('pointercancel', onUp)
    stageDrag.value = null
  }
  stageDrag.value = { onMove, onUp }
  window.addEventListener('pointermove', onMove)
  window.addEventListener('pointerup', onUp)
  window.addEventListener('pointercancel', onUp)
}
function onStageWheel(e) {
  const factor = e.deltaY > 0 ? 0.9 : 1.1
  coverZoom.value = Math.min(3, Math.max(0.5, coverZoom.value * factor))
}

function updateStageScale() {
  const w = stageRef.value ? stageRef.value.clientWidth : 0
  stageScale.value = w > 0 ? w / 1920 : 1
}

// 右侧设置面板与左侧预览（含播放条）等高：超出部分面板内部滚动，
// 不再让设置列把整张卡片撑得远高于编辑器；窄屏单列时解除限制。
function syncControlsBound() {
  const ctl = controlsRef.value
  if (!ctl) return
  if (!stageWrapRef.value || window.matchMedia('(max-width: 900px)').matches) {
    ctl.style.maxHeight = ''
    return
  }
  ctl.style.maxHeight = stageWrapRef.value.offsetHeight + 'px'
}

onMounted(async () => {
  updateStageScale()
  syncControlsBound()
  resizeRo = new ResizeObserver(() => { updateStageScale(); syncControlsBound() })
  if (stageWrapRef.value) resizeRo.observe(stageWrapRef.value)
  videoSrc.value = mediaUrl(props.dir, 'clip.mp4')
  try {
    const cfg = await api('/api/polish/project?dir=' + encodeURIComponent(props.dir))
    layout.value = cfg?.layout || {}
    videoSrc.value = mediaUrl(props.dir, cfg?.source?.video || 'clip.mp4')
    titleLines.value = (cfg?.cover?.title_lines && cfg.cover.title_lines.length)
      ? [...cfg.cover.title_lines]
      : (cfg?.project?.name ? [cfg.project.name] : [])
    const parsePct = (v, fallback) => {
      const n = parseFloat(String(v ?? '').replace('%', ''))
      return Number.isFinite(n) ? n : fallback
    }
    textX.value = parsePct(cfg?.cover?.text_x, 35)
    textY.value = parsePct(cfg?.cover?.text_y, 50)
    textWidth.value = Math.round((cfg?.cover?.text_width ?? 0.62) * 100)
    fontSize.value = cfg?.cover?.font_size || 120
    stroke.value = cfg?.cover?.stroke || 20
    hlColor.value = cfg?.cover?.hl_color || '#ffcc00'
    highlightKeywords.value = Array.isArray(cfg?.cover?.highlight_keywords) ? [...cfg.cover.highlight_keywords] : []
    letterSpacing.value = parseFloat(String(cfg?.cover?.letter_spacing ?? '8').replace('px', '')) || 8
    lineHeight.value = cfg?.cover?.line_height ?? 1.7
    textShadow.value = cfg?.cover?.text_shadow ?? !(cfg?.cover?.shadow_dir === 'none' || (cfg?.cover?.shadow_intensity ?? 1) <= 0)
    gradientDir.value = cfg?.cover?.gradient_dir || 'left'
    vignette.value = cfg?.cover?.vignette ?? 0.08
    align.value = cfg?.cover?.text_align || 'center'
    coverOffsetX.value = Math.round((cfg?.cover?.bg_x || 0) * stageScale.value * 10) / 10
    coverOffsetY.value = Math.round((cfg?.cover?.bg_y || 0) * stageScale.value * 10) / 10
    coverZoom.value = cfg?.cover?.bg_zoom || 1
    if (Array.isArray(cfg?.cover?.effects)) {
      fxList.value = cfg.cover.effects.map(e => ({ type: e.kind || e.type, amount: e.amount ?? 0.3, x: e.x ?? 50, y: e.y ?? 50, color: e.color || '#000000' }))
    }
    configLoaded.value = true
  } catch {}
})

onUnmounted(() => {
  if (saveConfigTimer) {
    clearTimeout(saveConfigTimer)
    saveConfigTimer = null
  }
  if (configLoaded.value) persistCoverConfig()
  if (resizeRo) {
    resizeRo.disconnect()
    resizeRo = null
  }
  if (stageDrag.value) {
    window.removeEventListener('pointermove', stageDrag.value.onMove)
    window.removeEventListener('pointerup', stageDrag.value.onUp)
    window.removeEventListener('pointercancel', stageDrag.value.onUp)
    stageDrag.value = null
  }
})
</script>

<style scoped>
.designer-grid { display:grid; grid-template-columns:minmax(0,1fr) 340px; gap:16px; align-items:start; }
/* 右侧设置列：紧凑分组 + 双列微网格；高度被 JS 限制为与左侧预览等高，超出内部滚动 */
.controls { display:flex; flex-direction:column; gap:8px; min-width:0; overflow-y:auto; padding-right:2px; scrollbar-width:thin; }
.controls::-webkit-scrollbar { width:6px; }
.controls::-webkit-scrollbar-thumb { background:var(--line); border-radius:3px; }
.grp { border:1px solid var(--line); border-radius:10px; background:var(--card); padding:1px 10px 8px; }
.grp summary { list-style:none; display:flex; align-items:center; gap:6px; font-size:12px; font-weight:700; color:var(--ink); padding:6px 0 3px; cursor:pointer; user-select:none; }
.grp summary::-webkit-details-marker { display:none; }
.grp summary::after { content:'▾'; margin-left:auto; font-size:10px; color:var(--ink-faint); transition:transform .15s; }
.grp:not([open]) summary::after { transform:rotate(-90deg); }
.grp textarea { font-size:12px; padding:5px 8px; line-height:1.5; }
.mini-row { display:flex; align-items:center; gap:6px; min-width:0; }
.mini-grid { display:grid; grid-template-columns:1fr 1fr; gap:4px 10px; }
.mini-field { display:flex; align-items:center; gap:5px; min-width:0; }
.mini-field > label { flex:none; margin:0; font-size:11px; color:var(--ink-dim); white-space:nowrap; }
.mini-field > input[type=range] { flex:1; min-width:0; padding:0; height:18px; background:transparent; border:none; box-shadow:none; accent-color:var(--main); cursor:pointer; }
.mini-field > select { flex:1; min-width:0; width:auto; padding:2px 6px; font-size:11.5px; border-radius:6px; }
.mini-field > input[type=color] { flex:1; min-width:0; width:auto; height:22px; padding:1px 2px; border-radius:6px; }
.mini-field.check { gap:4px; font-size:11.5px; color:var(--ink-dim); cursor:pointer; white-space:nowrap; }
.mini-field.check input[type=checkbox] { width:auto; margin:0; flex:none; }
.kw-input { flex:1; min-width:0; padding:4px 8px; font-size:12px; }
.grow { flex:1; min-width:0; }
.rv { flex:none; min-width:32px; text-align:right; font-size:10.5px; color:var(--ink-faint); font-family:monospace; }
.drag-hint { align-self:center; font-size:10.5px; color:var(--ink-faint); white-space:nowrap; overflow:hidden; text-overflow:ellipsis; min-width:0; }
.stage { position:relative; width:100%; aspect-ratio:16/9; background:#000; border-radius:14px; overflow:hidden; }
.stage-media { position:absolute; inset:0; z-index:0; background:#000; }
.stage-video { position:absolute; inset:0; width:100%; height:100%; max-height:none; margin:0; object-fit:cover; display:block; }
.fx-layer { position:absolute; inset:0; pointer-events:none; z-index:1; }
.fx-item { position:absolute; inset:0; }
.gradient-layer { position:absolute; inset:0; pointer-events:none; z-index:1; }
.vignette-layer { position:absolute; inset:0; pointer-events:none; z-index:1; background:radial-gradient(ellipse at 55% 45%, transparent 50%, rgba(0,0,0,1) 100%); }
.stage-gesture-layer { position:absolute; inset:0; z-index:5; cursor:grab; touch-action:none; }
.stage-gesture-layer:active { cursor:grabbing; }
.title-wrap { position:absolute; z-index:2; width:62%; transform:translate(-50%,-50%); text-align:center; pointer-events:none; }
.title-text { color:#f0e6d3; font-weight:900; line-height:1.7; paint-order:stroke fill; white-space:pre-line; overflow-wrap:anywhere; }
.title-text .hl { color:var(--hlc, #ffcc00); -webkit-text-stroke:inherit; paint-order:stroke fill; }
.seek-row { display:flex; align-items:center; gap:8px; }.seek-range { flex:1; min-width:0; padding:0; }
.seek-time { flex:none; min-width:120px; text-align:right; }
.w-time { width:90px; flex:none; }
.hidden-file { display:none; }
.preset-row { display:flex; gap:6px; }
.preset-row .mini-btn { flex:1; justify-content:center; padding:3px 0; }
.hl-chips { display:flex; flex-wrap:wrap; gap:6px; }
.hl-chip { display:inline-flex; align-items:center; gap:4px; background:var(--bg-soft); border:1px solid var(--line); border-radius:999px; padding:3px 8px; font-size:12px; color:var(--ink-dim); }
.hl-x { background:none; border:none; color:var(--ink-faint); cursor:pointer; font-size:14px; line-height:1; padding:0; }
.hl-x:hover { color:var(--red); }
.fx-card { border:1px solid var(--line); border-radius:8px; padding:6px 8px; background:var(--bg-soft); }
.fx-head { display:flex; align-items:center; justify-content:space-between; gap:6px; }
@media (max-width: 900px) {
  .designer-grid { grid-template-columns:1fr; }
  .controls { max-height:none !important; overflow:visible; }
}
</style>
