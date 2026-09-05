<template>
  <div class="card" id="cover-design">
    <h2>🖼 封面设计器</h2>
    <div class="designer-grid">
      <div class="stage-wrap">
        <div class="stage">
          <video ref="videoRef" controls playsinline preload="metadata" class="stage-video" :src="videoSrc"></video>
          <div class="fx-layer">
            <div v-for="(fx, i) in fxList" :key="i" class="fx-item" :style="fxStyle(fx)"></div>
          </div>
          <div class="title-wrap" :style="{ left: textX, top: textY }">
            <div class="title-text">{{ title }}</div>
          </div>
        </div>
        <div class="row" style="margin-top:8px;flex-wrap:wrap">
          <span class="muted small">时间</span>
          <input v-model.number="time" type="number" step="0.1" style="width:90px">
          <button class="mini-btn" @click="takeCurrent">取当前帧</button>
          <button class="mini-btn" @click="exportFrame">导出关键帧</button>
          <button class="mini-btn" @click="fileInput.click()">导入背景图片</button>
          <input ref="fileInput" type="file" accept="image/*" style="display:none" @change="importBg">
        </div>
      </div>
      <div class="controls">
        <details open>
          <summary style="cursor:pointer;font-weight:700">文字位置</summary>
          <label>标题</label>
          <input v-model="title" placeholder="默认片段标题">
          <div class="row">
            <div style="flex:1"><label>文字 X</label><input v-model="textX"></div>
            <div style="flex:1"><label>文字 Y</label><input v-model="textY"></div>
          </div>
          <div class="row" style="margin-top:6px">
            <div style="flex:1"><label>字号</label><input v-model.number="fontSize" type="number" step="1"></div>
            <div style="flex:1"><label>描边</label><input v-model.number="stroke" type="number" step="1"></div>
          </div>
          <div class="row" style="margin-top:6px">
            <div style="flex:1"><label>高亮色</label><input v-model="hlColor" type="color"></div>
            <div style="flex:1"><label>字间距</label><input v-model="letterSpacing"></div>
          </div>
          <div class="row" style="margin-top:6px">
            <button v-for="p in presets" :key="p.key" class="mini-btn" :class="{ selected: preset === p.key }" @click="applyPreset(p.key)">{{ p.label }}</button>
          </div>
        </details>
        <details open style="margin-top:10px">
          <summary style="cursor:pointer;font-weight:700">封面特效（可叠加）</summary>
          <div v-for="(fx, i) in fxList" :key="i" class="fx-row">
            <span class="chip p">{{ fxTypeName(fx.type) }}</span>
            <input v-model="fx.amount" type="number" step="0.1" placeholder="强度" style="flex:1;min-width:60px">
            <button class="mini-btn" @click="removeFx(i)">删除</button>
          </div>
          <div class="row" style="margin-top:8px">
            <div style="flex:1"><label>渐变方向</label>
              <select v-model="gradientDir">
                <option value="left">左</option>
                <option value="right">右</option>
                <option value="radial">径向</option>
                <option value="none">无</option>
              </select>
            </div>
            <div style="flex:1"><label>暗角</label><input v-model.number="vignette" type="number" step="0.05" min="0" max="1"></div>
          </div>
          <div class="row" style="margin-top:8px">
            <select v-model="addType" style="flex:1">
              <option value="gradient">渐变</option>
              <option value="vignette">暗角</option>
              <option value="zoom">缩放</option>
              <option value="glow">光晕</option>
              <option value="blur">模糊</option>
            </select>
            <button class="mini-btn" @click="addFx">＋ 添加特效</button>
          </div>
        </details>
        <div class="row" style="margin-top:10px">
          <button class="btn btn-primary" @click="generateCover">生成封面</button>
        </div>
      </div>
    </div>
  </div>
</template>

<script setup>
import { ref, onMounted } from 'vue'
import { api } from '../api/client'
import { toast } from '../stores/toast'

const props = defineProps({ dir: { type: String, required: true } })
const videoSrc = ref('')
const videoRef = ref(null)
const fileInput = ref(null)
const title = ref('')
const time = ref(0)
const textX = ref('35%')
const textY = ref('50%')
const preset = ref('left-text')
const fontSize = ref(120)
const stroke = ref(20)
const hlColor = ref('#ffcc00')
const letterSpacing = ref('8px')
const gradientDir = ref('left')
const vignette = ref(0.08)
const fxList = ref([])
const addType = ref('gradient')
const presets = [
  { key: 'left-text', label: '左文', x: '35%', y: '50%', gradientDir: 'left', vignette: 0.08 },
  { key: 'center-text', label: '居中', x: '50%', y: '50%', gradientDir: 'radial', vignette: 0.05 },
  { key: 'right-text', label: '右文', x: '65%', y: '50%', gradientDir: 'right', vignette: 0.08 },
  { key: 'minimal', label: '极简', x: '50%', y: '50%', gradientDir: 'none', vignette: 0.15 }
]

const fxNames = { gradient: '渐变', vignette: '暗角', zoom: '缩放', glow: '光晕', blur: '模糊' }
function fxTypeName(t) { return fxNames[t] || t }
function fxStyle(fx) {
  if (fx.type === 'gradient') return { background: 'linear-gradient(90deg, rgba(20,10,40,0.55) 0%, rgba(20,10,40,0.15) 50%, transparent 100%)' }
  if (fx.type === 'vignette') return { background: 'radial-gradient(ellipse at 55% 45%, transparent 50%, rgba(0,0,0,1) 100%)', opacity: fx.amount ?? 0.2 }
  if (fx.type === 'glow') return { background: 'radial-gradient(circle at 50% 40%, rgba(255,180,220,0.35) 0%, transparent 65%)' }
  if (fx.type === 'zoom') return { transform: 'scale(' + (1 + (fx.amount ?? 0.2)) + ')' }
  if (fx.type === 'blur') return { backdropFilter: 'blur(' + (fx.amount ?? 4) + 'px)' }
  return {}
}

function applyPreset(key) {
  const p = presets.find(x => x.key === key)
  if (!p) return
  preset.value = key
  textX.value = p.x
  textY.value = p.y
  if (p.gradientDir) gradientDir.value = p.gradientDir
  if (p.vignette != null) vignette.value = p.vignette
}

function takeCurrent() {
  const v = videoRef.value
  if (v) time.value = Math.round((v.currentTime || 0) * 10) / 10
}

function addFx() {
  fxList.value.push({ type: addType.value, amount: 0.2 })
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

async function generateCover() {
  try {
    await api('/api/polish/cover-config', { method: 'POST', body: JSON.stringify({
      dir: props.dir,
      title_text: title.value,
      cover: {
        text_x: textX.value,
        text_y: textY.value,
        font_size: fontSize.value,
        stroke: stroke.value,
        hl_color: hlColor.value,
        letter_spacing: letterSpacing.value,
        gradient_dir: gradientDir.value,
        vignette: vignette.value
      }
    }) })
    await api('/api/polish/cover', { method: 'POST', body: JSON.stringify({ dir: props.dir }) })
    toast('封面已生成')
  } catch (e) { toast(e.message) }
}

onMounted(async () => {
  const sep = String.fromCharCode(92)
  const norm = String(props.dir).split(sep).join('/')
  videoSrc.value = '/api/media/' + encodeURIComponent(norm + '/clip.mp4').replace(/%2F/g, '/')
  try {
    const cfg = await api('/api/polish/project?dir=' + encodeURIComponent(props.dir))
    title.value = cfg?.cover?.title_lines?.[0] || cfg?.project?.name || ''
    textX.value = cfg?.cover?.text_x || '35%'
    textY.value = cfg?.cover?.text_y || '50%'
    fontSize.value = cfg?.cover?.font_size || 120
    stroke.value = cfg?.cover?.stroke || 20
    hlColor.value = cfg?.cover?.hl_color || '#ffcc00'
    letterSpacing.value = cfg?.cover?.letter_spacing || '8px'
    gradientDir.value = cfg?.cover?.gradient_dir || 'left'
    vignette.value = cfg?.cover?.vignette ?? 0.08
  } catch (e) {}
})
</script>

<style scoped>
.designer-grid { display:grid; grid-template-columns:minmax(0,1fr) 300px; gap:16px; align-items:start; }
.stage { position:relative; width:100%; aspect-ratio:16/9; background:#000; border-radius:14px; overflow:hidden; }
.stage-video { position:absolute; inset:0; width:100%; height:100%; object-fit:cover; }
.fx-layer { position:absolute; inset:0; pointer-events:none; z-index:1; }
.fx-item { position:absolute; inset:0; }
.title-wrap { position:absolute; z-index:2; transform:translate(-50%,-50%); text-align:center; }
.title-text { color:#f0e6d3; font-weight:900; font-size:34px; text-shadow:0 2px 24px rgba(0,0,0,.8); -webkit-text-stroke:1px rgba(0,0,0,.4); }
.fx-row { display:flex; gap:6px; align-items:center; padding:4px 0; border-bottom:1px dashed var(--line-soft); }
</style>
