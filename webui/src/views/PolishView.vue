<template>
  <div>
    <div class="page-head">
      <div><h1>精修编辑器</h1><div class="sub">{{ dir }}</div></div>
    </div>
    <div class="layout">
      <div class="left">
        <div class="card">
          <h2>🎬 视频播放器（原生 DOM）</h2>
          <video ref="videoRef" controls playsinline preload="metadata" class="video" :src="videoSrc"></video>
        </div>
        <div class="card">
          <h2>⏱ 进度条</h2>
          <input type="range" :min="0" :max="1000" :value="progress" @input="seek" style="width:100%">
          <div class="muted small mono">{{ fmt(current) }} / {{ fmt(duration) }}</div>
        </div>
        <div class="card">
          <h2>✎ 时间轴编辑器（原生 DOM）</h2>
          <TimelineEditor :dir="dir" @seek="seekTo" @save="saveTimeline" />
        </div>
      </div>
      <div class="right card">
        <h2>⚙️ 精修控制</h2>
        <div class="row" style="flex-direction:column;align-items:stretch">
          <button class="btn btn-primary" @click="runAction('llm')">LLM 编辑</button>
          <button class="btn" @click="runAction('process')">process</button>
          <button class="btn" @click="runAction('render')">渲染</button>
          <button class="btn" @click="scrollCover">Cover</button>
        </div>
        <div class="dash"></div>
        <details open>
          <summary style="cursor:pointer;font-weight:700">九宫格布局</summary>
          <div v-if="layout">
            <div v-for="(region, key) in layout" :key="key" style="margin:8px 0">
              <div class="small muted" style="margin-bottom:4px">{{ labelMap[key] || key }}</div>
              <div style="display:grid;grid-template-columns:repeat(3,1fr);gap:4px">
                <div
                  v-for="cell in gridNames"
                  :key="cell"
                  class="grid-cell"
                  :class="{ active: region.grid && region.grid[0] === cell }"
                  @click="setLayout(key, cell)"
                >{{ cell }}</div>
              </div>
            </div>
          </div>
          <div v-else class="muted">加载中…</div>
        </details>
        <details open style="margin-top:10px">
          <summary style="cursor:pointer;font-weight:700">字幕列表</summary>
          <div v-if="!subs.length" class="muted">暂无字幕</div>
          <div v-for="(seg, i) in subs" :key="i" class="sub-row">
            <span class="muted small mono">{{ fmt(seg.start) }}</span>
            <textarea v-model="seg.text" rows="1" class="sub-input"></textarea>
          </div>
          <div class="row" style="margin-top:8px"><button class="btn btn-primary" @click="saveSubs">保存字幕</button></div>
        </details>
      </div>
    </div>
    <CoverDesigner :dir="dir" style="margin-top:16px" />
  </div>
</template>

<script setup>
import { ref, onMounted, onUnmounted } from 'vue'
import { useRoute } from 'vue-router'
import { api } from '../api/client'
import { toast } from '../stores/toast'
import CoverDesigner from '../components/CoverDesigner.vue'
import TimelineEditor from '../components/TimelineEditor.vue'

const route = useRoute()
const dir = ref(decodeURIComponent(route.query.dir || ''))
const videoSrc = ref('')
const videoRef = ref(null)
const current = ref(0)
const duration = ref(0)
const progress = ref(0)
const layout = ref(null)
const subs = ref([])
const gridNames = ['top-left','top','top-right','mid-left','center','mid-right','bottom-left','bottom','bottom-right']
const labelMap = { liver: '人物 liver', danmaku_area: '弹幕 danmaku', sticker_area: '贴纸/emoji' }

function fmt(sec) {
  sec = Number(sec) || 0
  const m = Math.floor(sec / 60), s = Math.floor(sec % 60)
  return String(m).padStart(2, '0') + ':' + String(s).padStart(2, '0')
}

function onTime() {
  const v = videoRef.value
  if (!v) return
  current.value = v.currentTime
  duration.value = v.duration || 0
  if (duration.value) progress.value = Math.round(v.currentTime / duration.value * 1000)
}

function seek(e) {
  const v = videoRef.value
  if (!v || !duration.value) return
  v.currentTime = duration.value * Number(e.target.value) / 1000
}

function seekTo(t) {
  const v = videoRef.value
  if (!v) return
  v.currentTime = Number(t) || 0
}

async function load() {
  if (!dir.value) return
  try {
    const cfg = await api('/api/polish/project?dir=' + encodeURIComponent(dir.value))
    const file = cfg?.source?.video || 'clip.mp4'
    const sep = String.fromCharCode(92)
    const norm = String(dir.value).split(sep).join('/')
    videoSrc.value = '/api/media/' + encodeURIComponent(norm + '/' + file).replace(/%2F/g, '/')
    layout.value = cfg?.layout || null
  } catch (e) {}
  try {
    const d = await api('/api/polish/subtitles?dir=' + encodeURIComponent(dir.value))
    subs.value = d.segments || []
  } catch (e) {}
}

async function setLayout(key, cell) {
  try {
    const current = await api('/api/polish/project?dir=' + encodeURIComponent(dir.value))
    const lay = current.layout || {}
    lay[key] = { grid: [cell], origin_grid: cell, origin: null }
    await api('/api/polish/project', { method: 'PUT', body: JSON.stringify({ dir: dir.value, layout: lay }) })
    layout.value = lay
  } catch (e) { toast(e.message) }
}

async function saveSubs() {
  try {
    await api('/api/polish/subtitles', { method: 'PUT', body: JSON.stringify({ dir: dir.value, segments: subs.value }) })
    toast('字幕已保存')
  } catch (e) { toast(e.message) }
}

async function runAction(kind) {
  try {
    await api('/api/polish/' + kind, { method: 'POST', body: JSON.stringify({ dir: dir.value }) })
    toast('已启动 ' + kind)
  } catch (e) { toast(e.message) }
}

async function saveTimeline(segments) {
  try {
    await api('/api/polish/project', { method: 'PUT', body: JSON.stringify({ dir: dir.value, segments }) })
    toast('时间轴已保存')
  } catch (e) { toast(e.message) }
}

function scrollCover() {
  const el = document.getElementById('cover-design')
  if (el) el.scrollIntoView({ behavior: 'smooth' })
}

onMounted(() => {
  load()
  if (videoRef.value) videoRef.value.addEventListener('timeupdate', onTime)
})
onUnmounted(() => {
  if (videoRef.value) videoRef.value.removeEventListener('timeupdate', onTime)
})
</script>

<style scoped>
.layout { display:grid; grid-template-columns:minmax(0,1fr) 320px; gap:16px; align-items:start; }
.left { display:flex; flex-direction:column; gap:16px; min-width:0; }
.video { width:100%; max-height:70vh; object-fit:contain; background:#000; border-radius:10px; display:block; }
.grid-cell { border:1px solid var(--line); border-radius:6px; padding:6px; text-align:center; font-size:10px; cursor:pointer; }
.grid-cell.active { background:var(--main-soft); border-color:var(--main); color:var(--main-deep); }
.sub-row { display:flex; gap:6px; align-items:flex-start; padding:4px 0; border-bottom:1px dashed var(--line-soft); }
.sub-input { flex:1; font-size:12px; resize:none; }
</style>
